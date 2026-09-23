//! Running saved tasks.
//!
//! A task runs through the same code as its tab. Transfers go through a Transfer tab state that no
//! tab shows, so validation, production confirmation and every export, import and copy path are
//! the Transfer tab's own. Comparisons and syncs call the engines the Compare tab calls.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt as _;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, Task, Window};
use mongodb::bson::Document;
use uuid::Uuid;

use crate::components::{
    WriteConfirmation, WriteRequest, open_confirm_dialog, request_connection_write,
};
use crate::connection::CancellationToken;
use crate::connection::ops::compare::{
    CompareMessage, CompareOptions, MAX_ROWS, Side, compare_collections_async,
};
use crate::connection::ops::compare_database::{
    CollectionKind, CollectionPair, DatabaseSync, PairMessage, PairScan, PairSync, PairSyncMessage,
    SyncMode, compare_pairs_async, list_side, pair_collections, sync_pairs_async, undo_pairs_async,
};
use crate::connection::ops::compare_sync::SyncSummary;
use crate::helpers::format_number;
use crate::state::app_state::{
    ActiveRun, CollectionTransferStatus, RunStop, TransferTabState, UndoLog,
};
use crate::state::compare::{CompareConfig, CompareScope};
use crate::state::{
    AppEvent, AppState, SessionKey, StatusMessage, TransferMode, TransferScope,
    resolved_export_destination, validate_transfer,
};
use crate::tasks::model::{LogLevel, Run, RunTrigger, Task as SavedTask, TaskSpec, side_index};
use crate::tasks::safety::{self, Planned, StopReason};

use super::AppCommands;

/// How a transfer run through a hidden Transfer tab ended.
enum TransferOutcome {
    Completed(u64),
    Failed(String),
    Cancelled,
}

impl AppCommands {
    /// Adds or updates a task and writes the task list.
    pub fn save_task(
        state: &Entity<AppState>,
        task: SavedTask,
        cx: &mut App,
    ) -> Result<(), String> {
        let name = task.name.clone();
        let result = state.update(cx, |app, cx| {
            let result = app.upsert_task(task);
            cx.notify();
            result
        });
        match &result {
            Ok(()) => {
                Self::task_status(state, StatusMessage::info(format!("Saved task “{name}”")), cx)
            }
            Err(error) => Self::task_status(state, StatusMessage::error(error.clone()), cx),
        }
        result
    }

    /// Writes the tab's current settings into the task it is linked to.
    pub fn save_linked_task(state: &Entity<AppState>, tab: &crate::state::TabKey, cx: &mut App) {
        let app = state.read(cx);
        let Some(mut task) = app.tab_task_id(tab).and_then(|id| app.task(id)).cloned() else {
            return;
        };
        let spec = match tab {
            crate::state::TabKey::Transfer(key) => app.transfer_task_spec(key.id),
            crate::state::TabKey::Compare(key) => app.compare_task_spec(key.id),
            _ => None,
        };
        let Some(spec) = spec else {
            return;
        };
        task.spec = spec;
        let id = task.id;
        if Self::save_task(state, task, cx).is_ok() {
            state.update(cx, |app, cx| {
                app.link_tab_to_task(tab, id);
                cx.notify();
            });
        }
    }

    pub fn delete_task(state: &Entity<AppState>, id: Uuid, cx: &mut App) {
        if state.read(cx).task_is_running(id) {
            return;
        }
        let result = state.update(cx, |app, cx| {
            let result = app.remove_task(id);
            cx.notify();
            result
        });
        if let Err(error) = result {
            Self::task_status(state, StatusMessage::error(error), cx);
        }
    }

    pub fn cancel_task_run(state: &Entity<AppState>, id: Uuid, cx: &mut App) {
        let stop = state.read(cx).tasks.active.get(&id).map(|active| match &active.stop {
            RunStop::Transfer(transfer) => Err(*transfer),
            RunStop::Token(token) => Ok(token.clone()),
        });
        match stop {
            Some(Ok(token)) => token.cancel(),
            Some(Err(transfer)) => Self::cancel_transfer(state.clone(), transfer, cx),
            None => {}
        }
    }

    /// Runs the task now. Closed connections are opened first. A task that writes works out
    /// what it would change, then asks before writing.
    pub fn run_task(state: Entity<AppState>, task_id: Uuid, window: &mut Window, cx: &mut App) {
        Self::launch(state, task_id, Launch::Run, window, cx);
    }

    /// Works out what the task would change and records it as a run, writing nothing.
    pub fn preview_task(state: Entity<AppState>, task_id: Uuid, window: &mut Window, cx: &mut App) {
        Self::launch(state, task_id, Launch::Preview, window, cx);
    }

    fn launch(
        state: Entity<AppState>,
        task_id: Uuid,
        launch: Launch,
        window: &mut Window,
        cx: &mut App,
    ) {
        let app = state.read(cx);
        let Some(task) = app.task(task_id).cloned() else {
            return;
        };
        if app.task_is_running(task_id) || app.tasks.starting.contains(&task_id) {
            return;
        }
        let connections = task.spec.connections();
        if connections.iter().any(|id| app.connection_by_id(*id).is_none()) {
            Self::fail_before_start(
                &state,
                &task,
                launch,
                "A connection this task uses no longer exists. Edit the task to choose another.",
                cx,
            );
            return;
        }
        let closed: Vec<Uuid> =
            connections.into_iter().filter(|id| !app.is_connected(*id)).collect();
        let window = window.window_handle();
        if closed.is_empty() {
            Self::start_task(state, task, launch, window, cx);
            return;
        }
        state.update(cx, |app, cx| {
            app.tasks.starting.insert(task_id);
            cx.notify();
        });
        let waits: Vec<_> =
            closed.into_iter().map(|id| Self::connect_and_wait(state.clone(), id, cx)).collect();
        cx.spawn(async move |cx| {
            let mut failure = None;
            for wait in waits {
                if let Err(error) = wait.await {
                    failure = Some(error);
                    break;
                }
            }
            cx.update(|cx| {
                state.update(cx, |app, cx| {
                    app.tasks.starting.remove(&task_id);
                    cx.notify();
                });
                match failure {
                    Some(error) => Self::fail_before_start(
                        &state,
                        &task,
                        launch,
                        &format!("Could not connect: {error}"),
                        cx,
                    ),
                    None => Self::start_task(state, task, launch, window, cx),
                }
            });
        })
        .detach();
    }

    /// Connects in the background and resolves when the connection is open or has failed.
    fn connect_and_wait(
        state: Entity<AppState>,
        id: Uuid,
        cx: &mut App,
    ) -> Task<Result<(), String>> {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let sender = RefCell::new(Some(sender));
        let subscription = cx.subscribe(&state, move |_, event: &AppEvent, _| {
            let result = match event {
                AppEvent::Connected(connected) if *connected == id => Ok(()),
                AppEvent::ConnectionFailed { connection_id, error } if *connection_id == id => {
                    Err(error.clone())
                }
                _ => return,
            };
            if let Some(sender) = sender.borrow_mut().take() {
                let _ = sender.send(result);
            }
        });
        Self::connect_in_background(state, id, cx);
        cx.spawn(async move |_| {
            let result = receiver.await.unwrap_or_else(|_| Err("the attempt stopped".into()));
            drop(subscription);
            result
        })
    }

    fn start_task(
        state: Entity<AppState>,
        task: SavedTask,
        launch: Launch,
        window: AnyWindowHandle,
        cx: &mut App,
    ) {
        match task.spec.clone() {
            TaskSpec::Transfer { config, options } => {
                let mut tab = TransferTabState::from_settings(&state.read(cx).settings);
                tab.config = config;
                tab.options = options;
                Self::start_transfer_task(state, task, tab, launch, window, cx);
            }
            // A comparison writes nothing, so its preview is the run itself.
            TaskSpec::Compare { config } => Self::start_compare_run(state, task, config, cx),
            TaskSpec::Sync { config, target, mode, excluded } => {
                let sync = SyncRequest { config, target, mode, excluded };
                Self::start_sync_task(state, task, sync, launch, window, cx);
            }
        }
    }

    /// Asks in `window` before a run writes, then runs `go`. Closing the dialog without
    /// answering ends the run as not confirmed.
    fn ask(
        state: Entity<AppState>,
        task_id: Uuid,
        window: AnyWindowHandle,
        question: Question,
        cx: &mut App,
        go: impl FnOnce(&mut App) + 'static,
    ) {
        let (answered, unanswered) = futures::channel::oneshot::channel::<()>();
        // The dialog holds this until it closes. Dropped unused, the channel closes, and that is
        // how a dismissed dialog is noticed.
        let on_confirm = move |_: &mut Window, cx: &mut App| {
            let _ = answered.send(());
            go(cx);
        };
        // Deferred: this is often reached while `window` itself is being updated, and a window
        // can't be updated again from inside its own update.
        let dialog_state = state.clone();
        cx.defer(move |cx| {
            let _ = cx.update_window(window, |_, window, cx| match question {
                Question::Write(request) => {
                    request_connection_write(dialog_state, request, window, cx, on_confirm)
                }
                Question::Replace { title, message } => {
                    open_confirm_dialog(window, cx, title, message, "Replace", true, on_confirm)
                }
            });
        });
        cx.spawn(async move |cx| {
            if unanswered.await.is_err() {
                cx.update(|cx| {
                    Self::update_task_run(&state, task_id, Some(true), cx, |run| {
                        run.log(LogLevel::Info, "Not confirmed, so nothing was written.");
                    })
                });
            }
        })
        .detach();
    }

    fn start_transfer_task(
        state: Entity<AppState>,
        task: SavedTask,
        tab: TransferTabState,
        launch: Launch,
        window: AnyWindowHandle,
        cx: &mut App,
    ) {
        let validation = validate_transfer(&tab);
        if !validation.can_run() {
            let reason = validation
                .blocking_errors
                .first()
                .cloned()
                .unwrap_or_else(|| "The transfer isn't ready to run.".into());
            Self::fail_before_start(&state, &task, launch, &reason, cx);
            return;
        }
        let whole_target = tab.options.drop_before_import || tab.options.clear_before_import;
        let writes = task.spec.write_connection().is_some();
        let (_, cancellation) = Self::begin_task_run(&state, &task, launch.trigger(), cx);
        if launch == Launch::Run && !(writes && whole_target) {
            Self::confirm_transfer(state, task, tab, Vec::new(), window, cx);
            return;
        }
        let Some(request) = TransferCounts::new(state.read(cx), &tab) else {
            Self::fail_run(&state, task.id, "The connections must be open.".into(), cx);
            return;
        };
        cx.spawn(async move |cx| {
            let planned = cx.background_spawn(async move { request.count() }).await;
            cx.update(|cx| {
                let planned = match planned {
                    Ok(planned) => planned,
                    Err(error) => return Self::fail_run(&state, task.id, error, cx),
                };
                let stops = Self::record_plan(&state, &task, std::slice::from_ref(&planned), cx);
                if launch == Launch::Preview || cancellation.is_cancelled() {
                    Self::update_task_run(
                        &state,
                        task.id,
                        Some(cancellation.is_cancelled()),
                        cx,
                        |_| {},
                    );
                    return;
                }
                Self::confirm_transfer(state, task, tab, stops, window, cx);
            });
        })
        .detach();
    }

    fn confirm_transfer(
        state: Entity<AppState>,
        task: SavedTask,
        tab: TransferTabState,
        stops: Vec<String>,
        window: AnyWindowHandle,
        cx: &mut App,
    ) {
        let task_id = task.id;
        if let Some(connection) = task.spec.write_connection() {
            let target = transfer_target(&tab);
            let mut message = format!("{} writes into {target}.", task.spec.kind().label());
            if tab.options.drop_before_import {
                message.push_str(" The target is dropped first.");
            } else if tab.options.clear_before_import {
                message.push_str(" Every document in the target is deleted first.");
            }
            let anyway = !stops.is_empty();
            let request = WriteRequest::new(
                connection,
                target,
                format!("Run “{}”", task.name),
                Some(confirmation(
                    &task,
                    message,
                    &stops,
                    tab.options.drop_before_import || tab.options.clear_before_import,
                )),
            );
            Self::ask(state.clone(), task_id, window, Question::Write(request), cx, move |cx| {
                Self::start_transfer_run(state, task_id, tab, None, anyway, cx)
            });
        } else if let Some(path) = resolved_export_destination(&tab).filter(|path| path.exists()) {
            let question = Question::Replace {
                title: format!("Replace {}?", path.display()),
                message: format!("“{}” exports to a file that already exists.", task.name),
            };
            Self::ask(state.clone(), task_id, window, question, cx, move |cx| {
                Self::start_transfer_run(state, task_id, tab, Some(path), false, cx)
            });
        } else {
            Self::start_transfer_run(state, task_id, tab, None, false, cx);
        }
    }

    /// Writes the plan into the run and returns why the safety limit would stop it.
    fn record_plan(
        state: &Entity<AppState>,
        task: &SavedTask,
        planned: &[Planned],
        cx: &mut App,
    ) -> Vec<String> {
        let history = state.read(cx).task_runs(task.id).to_vec();
        let stops: Vec<String> = safety::check(&task.safety, planned, &history)
            .iter()
            .map(StopReason::describe)
            .collect();
        Self::update_task_run(state, task.id, None, cx, |run| {
            for plan in planned {
                run.collection_mut(&plan.name).planned =
                    Some([plan.inserts, plan.replaces, plan.deletes]);
            }
            let [inserts, replaces, deletes] = planned.iter().fold([0; 3], |total, plan| {
                [total[0] + plan.inserts, total[1] + plan.replaces, total[2] + plan.deletes]
            });
            run.log(
                LogLevel::Info,
                format!(
                    "Would insert {}, replace {} and delete {}.",
                    format_number(inserts),
                    format_number(replaces),
                    format_number(deletes)
                ),
            );
            for stop in &stops {
                run.log(LogLevel::Warning, format!("Safety limit: {stop}"));
            }
            run.stops = stops.clone();
        });
        stops
    }

    /// Records a run that failed before any work started, so the reason shows in its history.
    fn fail_before_start(
        state: &Entity<AppState>,
        task: &SavedTask,
        launch: Launch,
        reason: &str,
        cx: &mut App,
    ) {
        let mut run = Run::start(task.id, launch.trigger());
        run.error = Some(reason.to_string());
        run.log(LogLevel::Error, reason);
        run.finish(false);
        state.update(cx, |app, cx| {
            app.record_task_run(run, true);
            cx.notify();
        });
    }

    fn task_status(state: &Entity<AppState>, message: StatusMessage, cx: &mut App) {
        state.update(cx, |app, cx| {
            app.set_status_message(Some(message));
            cx.notify();
        });
    }

    /// Starts the run's record and marks the task running. Cancel stops it through the
    /// returned token until a transfer takes over.
    fn begin_task_run(
        state: &Entity<AppState>,
        task: &SavedTask,
        trigger: RunTrigger,
        cx: &mut App,
    ) -> (Uuid, CancellationToken) {
        let mut run = Run::start(task.id, trigger);
        run.log(LogLevel::Info, format!("{}: {}", trigger.label(), task.spec.subject()));
        let run_id = run.id;
        let cancellation = CancellationToken::new();
        state.update(cx, |app, cx| {
            app.record_task_run(run, true);
            app.tasks.active.insert(
                task.id,
                ActiveRun {
                    run_id,
                    stop: RunStop::Token(cancellation.clone()),
                    transfer_id: None,
                    _events: None,
                },
            );
            cx.notify();
        });
        (run_id, cancellation)
    }

    /// Applies `change` to the run in progress. `done` ends it and saves it.
    fn update_task_run(
        state: &Entity<AppState>,
        task_id: Uuid,
        done: Option<bool>,
        cx: &mut App,
        change: impl FnOnce(&mut Run),
    ) {
        state.update(cx, |app, cx| {
            let Some(run_id) = app.tasks.active.get(&task_id).map(|active| active.run_id) else {
                return;
            };
            let Some(mut run) = app.task_run(task_id, run_id).cloned() else {
                return;
            };
            change(&mut run);
            if let Some(cancelled) = done {
                run.finish(cancelled);
                run.log(LogLevel::Info, format!("Finished: {}", run.status.label().to_lowercase()));
                app.tasks.active.remove(&task_id);
            }
            app.record_task_run(run, done.is_some());
            cx.notify();
        });
    }

    /// Runs the transfer inside the task's run, through a Transfer tab state no tab shows.
    fn start_transfer_run(
        state: Entity<AppState>,
        task_id: Uuid,
        tab: TransferTabState,
        confirmed_overwrite: Option<PathBuf>,
        anyway: bool,
        cx: &mut App,
    ) {
        let scope = tab.config.scope;
        let collection = match tab.config.mode {
            TransferMode::Import if tab.config.source_collection.is_empty() => {
                tab.config.destination_collection.clone()
            }
            _ => tab.config.source_collection.clone(),
        };
        let transfer_id = state.update(cx, |app, _| app.insert_task_transfer(tab));
        let events = cx.subscribe(&state, move |state, event: &AppEvent, cx| {
            let outcome = match event {
                AppEvent::TransferCompleted { transfer_id: id, count } if *id == transfer_id => {
                    TransferOutcome::Completed(*count)
                }
                AppEvent::TransferFailed { transfer_id: id, error } if *id == transfer_id => {
                    TransferOutcome::Failed(error.clone())
                }
                AppEvent::TransferCancelled { transfer_id: id } if *id == transfer_id => {
                    TransferOutcome::Cancelled
                }
                _ => return,
            };
            let collection = collection.clone();
            // Finishing drops this subscription, so it happens after the event is delivered.
            cx.defer(move |cx| {
                Self::finish_transfer_run(
                    &state,
                    task_id,
                    transfer_id,
                    scope,
                    &collection,
                    outcome,
                    cx,
                )
            });
        });
        state.update(cx, |app, cx| {
            if let Some(active) = app.tasks.active.get_mut(&task_id) {
                active.stop = RunStop::Transfer(transfer_id);
                active.transfer_id = Some(transfer_id);
                active._events = Some(events);
            }
            cx.notify();
        });
        if anyway {
            Self::update_task_run(&state, task_id, None, cx, |run| {
                run.log(LogLevel::Warning, "Run anyway: confirmed despite the safety limit.");
            });
        }
        match confirmed_overwrite {
            Some(path) => {
                Self::execute_confirmed_transfer(state.clone(), transfer_id, Some(path), cx)
            }
            None => Self::execute_transfer(state.clone(), transfer_id, cx),
        }
        // A transfer that couldn't start leaves its reason on the tab state and sends no event.
        let refused = state.read(cx).transfer_tab(transfer_id).and_then(|tab| {
            (!tab.runtime.is_running).then(|| {
                tab.runtime
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "The transfer didn't start.".into())
            })
        });
        if let Some(error) = refused {
            let collection = String::new();
            Self::finish_transfer_run(
                &state,
                task_id,
                transfer_id,
                scope,
                &collection,
                TransferOutcome::Failed(error),
                cx,
            );
        }
    }

    fn finish_transfer_run(
        state: &Entity<AppState>,
        task_id: Uuid,
        transfer_id: Uuid,
        scope: TransferScope,
        collection: &str,
        outcome: TransferOutcome,
        cx: &mut App,
    ) {
        let tab = state.update(cx, |app, _| app.remove_task_transfer(transfer_id));
        let cancelled = matches!(outcome, TransferOutcome::Cancelled);
        Self::update_task_run(state, task_id, Some(cancelled), cx, |run| {
            let progress = tab.as_ref().and_then(|tab| tab.runtime.database_progress.as_ref());
            match (scope, progress) {
                (TransferScope::Database, Some(progress)) => {
                    for item in &progress.collections {
                        let entry = run.collection_mut(&item.name);
                        entry.documents = item.documents_processed;
                        match &item.status {
                            CollectionTransferStatus::Failed(error) => {
                                entry.error = Some(error.clone())
                            }
                            CollectionTransferStatus::Cancelled => {
                                entry.note = Some("Cancelled".into())
                            }
                            _ => {}
                        }
                    }
                    if let TransferOutcome::Failed(error) = &outcome
                        && run.collections.iter().all(|entry| entry.error.is_none())
                    {
                        run.error = Some(error.clone());
                    }
                }
                (TransferScope::Database, None) => {
                    if let TransferOutcome::Failed(error) = &outcome {
                        run.error = Some(error.clone());
                    }
                }
                _ if collection.is_empty() => {
                    if let TransferOutcome::Failed(error) = &outcome {
                        run.error = Some(error.clone());
                    }
                }
                _ => {
                    let processed = tab.as_ref().map_or(0, |tab| tab.runtime.progress_count);
                    let entry = run.collection_mut(collection);
                    match &outcome {
                        TransferOutcome::Completed(count) => entry.documents = *count,
                        TransferOutcome::Failed(error) => {
                            entry.documents = processed;
                            entry.error = Some(error.clone());
                        }
                        TransferOutcome::Cancelled => entry.documents = processed,
                    }
                }
            }
            if let TransferOutcome::Failed(error) = &outcome {
                run.log(LogLevel::Error, error.clone());
            }
        });
    }

    fn start_compare_run(
        state: Entity<AppState>,
        task: SavedTask,
        config: CompareConfig,
        cx: &mut App,
    ) {
        let app = state.read(cx);
        let clients = config
            .sides
            .each_ref()
            .map(|side| side.connection_id.and_then(|id| app.active_connection_client(id)));
        let [Some(left), Some(right)] = clients else {
            Self::fail_before_start(
                &state,
                &task,
                Launch::Run,
                "Both connections must be open.",
                cx,
            );
            return;
        };
        let runtime = app.connection_manager().runtime_handle();
        let timeout = Duration::from_millis(app.settings.interactive_query_timeout_ms.max(100));
        let task_id = task.id;
        let (_, cancellation) = Self::begin_task_run(&state, &task, RunTrigger::Manual, cx);

        match config.scope {
            CompareScope::Collections => {
                let filter = if config.filter.trim().is_empty() {
                    Document::new()
                } else {
                    match crate::bson::parse_document_from_json(&config.filter) {
                        Ok(filter) => filter,
                        Err(error) => {
                            Self::update_task_run(&state, task_id, Some(false), cx, |run| {
                                run.error = Some(format!("The filter isn't valid JSON: {error}"));
                            });
                            return;
                        }
                    }
                };
                let name = config.sides[0].collection.clone();
                let (sender, mut receiver) = futures::channel::mpsc::unbounded();
                let collections = [0, 1].map(|i| {
                    [left.clone(), right.clone()][i]
                        .database(&config.sides[i].database)
                        .collection(&config.sides[i].collection)
                });
                let options = CompareOptions {
                    fields: config.fields.clone(),
                    filter,
                    ignore: config.ignore_set(),
                    row_limit: 0,
                    row_kinds: None,
                };
                let [left, right] = collections;
                let work = runtime.spawn(compare_collections_async(
                    left,
                    right,
                    options,
                    cancellation,
                    sender,
                ));
                cx.spawn(async move |cx| {
                    while let Some(message) = receiver.next().await {
                        if let CompareMessage::Progress { counts, .. } = message {
                            cx.update(|cx| {
                                Self::update_task_run(&state, task_id, None, cx, |run| {
                                    run.collection_mut(&name).differences = Some(counts);
                                })
                            });
                        }
                    }
                    let result = work.await;
                    cx.update(|cx| {
                        let cancelled = matches!(&result, Ok(Ok(summary)) if summary.cancelled);
                        Self::update_task_run(&state, task_id, Some(cancelled), cx, |run| {
                            let entry = run.collection_mut(&name);
                            match result {
                                Ok(Ok(summary)) => entry.differences = Some(summary.counts),
                                Ok(Err(error)) => entry.error = Some(error.to_string()),
                                Err(error) => {
                                    entry.error = Some(format!("The comparison stopped: {error}"))
                                }
                            }
                        });
                    });
                })
                .detach();
            }
            CompareScope::Databases => {
                let databases = config.sides.each_ref().map(|side| side.database.clone());
                let skip = config.skip.clone();
                let ignore = config.ignore_set();
                cx.spawn(async move |cx| {
                    let listed = runtime
                        .spawn({
                            let (left, right, databases) = (left.clone(), right.clone(), databases.clone());
                            async move {
                                let (l, r) = tokio::join!(
                                    list_side(&left, &databases[0], timeout),
                                    list_side(&right, &databases[1], timeout)
                                );
                                Ok::<_, crate::error::Error>(pair_collections(l?, r?))
                            }
                        })
                        .await;
                    let pairs = match listed {
                        Ok(Ok(pairs)) => pairs,
                        Ok(Err(error)) => return cx.update(|cx| Self::fail_run(&state, task_id, error.to_string(), cx)),
                        Err(error) => return cx.update(|cx| Self::fail_run(&state, task_id, error.to_string(), cx)),
                    };
                    let mut scans = Vec::new();
                    cx.update(|cx| {
                        Self::update_task_run(&state, task_id, None, cx, |run| {
                            for pair in &pairs {
                                let note = if skip.contains(&pair.name) {
                                    Some("Listed under Skip collections")
                                } else {
                                    pair_note(pair)
                                };
                                let entry = run.collection_mut(&pair.name);
                                match note {
                                    Some(note) => entry.note = Some(note.into()),
                                    None => scans.push(PairScan {
                                        index: scans.len(),
                                        name: pair.name.clone(),
                                        cancellation: cancellation.clone(),
                                    }),
                                }
                            }
                        })
                    });
                    let names: Vec<String> = scans.iter().map(|scan| scan.name.clone()).collect();
                    let (sender, mut receiver) = futures::channel::mpsc::unbounded();
                    let work = runtime.spawn(compare_pairs_async([left, right], databases, scans, ignore, sender));
                    while let Some(message) = receiver.next().await {
                        cx.update(|cx| {
                            Self::update_task_run(&state, task_id, None, cx, |run| match message {
                                PairMessage::Progress(i, counts) => {
                                    run.collection_mut(&names[i]).differences = Some(counts)
                                }
                                PairMessage::Done(i, summary) => {
                                    let entry = run.collection_mut(&names[i]);
                                    entry.differences = Some(summary.counts);
                                    if summary.cancelled {
                                        entry.note = Some("Cancelled".into());
                                    }
                                }
                                PairMessage::Failed(i, error) => {
                                    run.collection_mut(&names[i]).error = Some(error)
                                }
                                PairMessage::Started(_) => {}
                            })
                        });
                    }
                    let _ = work.await;
                    cx.update(|cx| {
                        let cancelled = state
                            .read(cx)
                            .tasks
                            .active
                            .get(&task_id)
                            .is_some_and(|active| matches!(&active.stop, RunStop::Token(token) if token.is_cancelled()));
                        Self::update_task_run(&state, task_id, Some(cancelled), cx, |_| {});
                    });
                })
                .detach();
            }
        }
    }

    fn fail_run(state: &Entity<AppState>, task_id: Uuid, error: String, cx: &mut App) {
        Self::update_task_run(state, task_id, Some(false), cx, |run| {
            run.log(LogLevel::Error, error.clone());
            run.error = Some(error);
        });
    }

    /// Lists both databases, compares the collections the sync would write, records what it
    /// would change, then asks before writing. A preview stops after recording.
    fn start_sync_task(
        state: Entity<AppState>,
        task: SavedTask,
        sync: SyncRequest,
        launch: Launch,
        window: AnyWindowHandle,
        cx: &mut App,
    ) {
        let app = state.read(cx);
        let clients = sync
            .config
            .sides
            .each_ref()
            .map(|side| side.connection_id.and_then(|id| app.active_connection_client(id)));
        let [Some(left), Some(right)] = clients else {
            Self::fail_before_start(&state, &task, launch, "Both connections must be open.", cx);
            return;
        };
        let runtime = app.connection_manager().runtime_handle();
        let timeout = Duration::from_millis(app.settings.interactive_query_timeout_ms.max(100));
        let databases = sync.config.sides.each_ref().map(|side| side.database.clone());
        let ignore = sync.config.ignore_set();
        let task_id = task.id;
        let (_, cancellation) = Self::begin_task_run(&state, &task, launch.trigger(), cx);
        let (target, source) = (side_index(sync.target), 1 - side_index(sync.target));

        cx.spawn(async move |cx| {
            let listed = runtime
                .spawn({
                    let (left, right, databases) = (left.clone(), right.clone(), databases.clone());
                    async move {
                        let (l, r) = tokio::join!(
                            list_side(&left, &databases[0], timeout),
                            list_side(&right, &databases[1], timeout)
                        );
                        Ok::<_, crate::error::Error>(pair_collections(l?, r?))
                    }
                })
                .await;
            let pairs = match listed {
                Ok(Ok(pairs)) => pairs,
                Ok(Err(error)) => {
                    return cx.update(|cx| Self::fail_run(&state, task_id, error.to_string(), cx));
                }
                Err(error) => {
                    return cx.update(|cx| Self::fail_run(&state, task_id, error.to_string(), cx));
                }
            };
            let (plan, notes) = sync_plan(&pairs, sync.target, &sync.excluded, &sync.config.skip);
            cx.update(|cx| {
                Self::update_task_run(&state, task_id, None, cx, |run| {
                    for pair in &plan {
                        run.collection_mut(&pair.name);
                    }
                    for (name, note) in &notes {
                        run.collection_mut(name).note = Some((*note).into());
                    }
                })
            });

            // Compare what exists on both sides; a collection the target lacks is all inserts.
            let names: Vec<String> = plan.iter().map(|pair| pair.name.clone()).collect();
            let scans: Vec<PairScan> = plan
                .iter()
                .filter(|pair| !pair.create)
                .map(|pair| PairScan {
                    index: pair.index,
                    name: pair.name.clone(),
                    cancellation: cancellation.clone(),
                })
                .collect();
            let (sender, mut receiver) = futures::channel::mpsc::unbounded();
            let work = runtime.spawn(compare_pairs_async(
                [left.clone(), right.clone()],
                databases.clone(),
                scans,
                ignore,
                sender,
            ));
            let mut counts = std::collections::HashMap::new();
            let mut failures = Vec::new();
            while let Some(message) = receiver.next().await {
                match message {
                    PairMessage::Done(i, summary) => {
                        counts.insert(i, summary.counts);
                    }
                    PairMessage::Failed(i, error) => failures.push((i, error)),
                    PairMessage::Started(_) | PairMessage::Progress(..) => {}
                }
            }
            let _ = work.await;
            cx.update(|cx| {
                let cancelled = cancellation.is_cancelled();
                if cancelled || !failures.is_empty() {
                    Self::update_task_run(&state, task_id, Some(cancelled), cx, |run| {
                        for (i, error) in failures {
                            run.collection_mut(&names[i]).error = Some(error);
                        }
                    });
                    return;
                }
                let planned: Vec<Planned> = plan
                    .iter()
                    .map(|pair| {
                        let Some(counts) = counts.get(&pair.index) else {
                            let sides = pairs.iter().find(|p| p.name == pair.name);
                            let inserts = sides
                                .and_then(|p| p.sides[source].as_ref())
                                .and_then(|side| side.estimated)
                                .unwrap_or(0);
                            return Planned {
                                name: pair.name.clone(),
                                inserts,
                                ..Default::default()
                            };
                        };
                        let [inserts, replaces, deletes] = sync.mode.writes(counts, sync.target);
                        let read = [counts.left_read, counts.right_read];
                        Planned {
                            name: pair.name.clone(),
                            inserts,
                            replaces,
                            deletes,
                            target_documents: read[target],
                            source_documents: Some(read[source]),
                            replaces_whole_target: false,
                        }
                    })
                    .collect();
                let stops = Self::record_plan(&state, &task, &planned, cx);
                if launch == Launch::Preview {
                    Self::update_task_run(&state, task_id, Some(false), cx, |_| {});
                    return;
                }
                let writes: u64 = planned.iter().map(|p| p.inserts + p.replaces + p.deletes).sum();
                if writes == 0 {
                    Self::update_task_run(&state, task_id, Some(false), cx, |run| {
                        run.log(LogLevel::Info, "Nothing to write: the target already matches.");
                    });
                    return;
                }
                Self::confirm_sync(
                    state,
                    task,
                    sync,
                    plan,
                    planned,
                    stops,
                    cancellation,
                    window,
                    cx,
                );
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn confirm_sync(
        state: Entity<AppState>,
        task: SavedTask,
        sync: SyncRequest,
        plan: Vec<PairSync>,
        planned: Vec<Planned>,
        stops: Vec<String>,
        cancellation: CancellationToken,
        window: AnyWindowHandle,
        cx: &mut App,
    ) {
        let index = side_index(sync.target);
        let Some(connection) = sync.config.sides[index].connection_id else {
            return;
        };
        let database = sync.config.sides[index].database.clone();
        // Only collections with something to write are written, and each takes one grant.
        let written: Vec<PairSync> = plan
            .into_iter()
            .zip(&planned)
            .filter(|(_, planned)| planned.inserts + planned.replaces + planned.deletes > 0)
            .map(|(pair, _)| pair)
            .collect();
        let [inserts, replaces, deletes] = planned.iter().fold([0; 3], |total, plan| {
            [total[0] + plan.inserts, total[1] + plan.replaces, total[2] + plan.deletes]
        });
        let count = written.len();
        let message = format!(
            "{} writes into {count} collection{} of {database}: {} inserted, {} replaced, {} deleted.",
            sync.mode.label(),
            if count == 1 { "" } else { "s" },
            format_number(inserts),
            format_number(replaces),
            format_number(deletes),
        );
        let request = WriteRequest::new(
            connection,
            database.clone(),
            format!("Run “{}”", task.name),
            Some(confirmation(&task, message, &stops, deletes > 0)),
        )
        .for_writes(count);
        let task_id = task.id;
        let anyway = !stops.is_empty();
        Self::ask(state.clone(), task_id, window, Question::Write(request), cx, move |cx| {
            // One production grant per collection, spent before anything is written.
            for pair in &written {
                let key = SessionKey::new(connection, &database, &pair.name);
                if !Self::ensure_collection_writable(&state, &key, cx) {
                    Self::update_task_run(&state, task_id, Some(true), cx, |run| {
                        run.log(
                            LogLevel::Warning,
                            format!("Writing to {} was refused.", pair.name),
                        );
                    });
                    return;
                }
            }
            if anyway {
                Self::update_task_run(&state, task_id, None, cx, |run| {
                    run.log(LogLevel::Warning, "Run anyway: confirmed despite the safety limit.");
                });
            }
            Self::write_sync(state, task_id, sync, written, cancellation, cx)
        });
    }

    /// Writes the sync. A Mirror writes inserts and replacements first and deletes last, and
    /// skips the deletes when anything before them failed.
    fn write_sync(
        state: Entity<AppState>,
        task_id: Uuid,
        sync: SyncRequest,
        plan: Vec<PairSync>,
        cancellation: CancellationToken,
        cx: &mut App,
    ) {
        let app = state.read(cx);
        let clients = sync
            .config
            .sides
            .each_ref()
            .map(|side| side.connection_id.and_then(|id| app.active_connection_client(id)));
        let [Some(left), Some(right)] = clients else {
            Self::fail_run(&state, task_id, "Both connections must be open.".into(), cx);
            return;
        };
        let runtime = app.connection_manager().runtime_handle();
        let restore_dir = app.compare_restore_dir();
        let databases = sync.config.sides.each_ref().map(|side| side.database.clone());
        let target = side_index(sync.target);
        let (Some(connection_id), database) =
            (sync.config.sides[target].connection_id, databases[target].clone())
        else {
            return;
        };
        let run_id = app.tasks.active.get(&task_id).map(|active| active.run_id);
        let passes = if sync.mode == SyncMode::Mirror {
            vec![(SyncMode::AddAndUpdate, false), (SyncMode::Mirror, true)]
        } else {
            vec![(sync.mode, false)]
        };
        // A new sync replaces the task's undo: only its last sync can be undone.
        state.update(cx, |app, _| {
            app.tasks.undo.remove(&task_id);
        });

        // Numbered from 0 again: each pass offsets the numbers by the count, so every undo record
        // and message names its collection and pass.
        let plan: Vec<PairSync> =
            plan.into_iter().enumerate().map(|(index, pair)| PairSync { index, ..pair }).collect();
        cx.spawn(async move |cx| {
            let count = plan.len();
            let name_of = |index: usize| plan[index % count].name.clone();
            let mut written: std::collections::HashMap<usize, SyncSummary> = Default::default();
            let mut logs = Vec::new();
            let mut stopped = None;
            for (pass, (mode, deletes_only)) in passes.into_iter().enumerate() {
                if pass > 0 {
                    let failed = stopped.is_some()
                        || cancellation.is_cancelled()
                        || written
                            .values()
                            .any(|summary| summary.failed > 0 || summary.uncertain > 0);
                    if failed {
                        cx.update(|cx| {
                            Self::update_task_run(&state, task_id, None, cx, |run| {
                                run.log(
                                    LogLevel::Warning,
                                    "Deletes were skipped because something before them failed.",
                                );
                            })
                        });
                        break;
                    }
                }
                let pairs = plan
                    .iter()
                    .map(|pair| PairSync {
                        index: pass * count + pair.index,
                        name: pair.name.clone(),
                        create: pair.create && pass == 0,
                    })
                    .collect();
                let (sender, mut receiver) = futures::channel::mpsc::unbounded();
                let work = runtime.spawn(sync_pairs_async(
                    DatabaseSync {
                        clients: [left.clone(), right.clone()],
                        databases: databases.clone(),
                        target: sync.target,
                        mode,
                        pairs,
                        ignore: sync.config.ignore_set(),
                        restore_dir: restore_dir.clone(),
                        pass_rows: MAX_ROWS,
                        deletes_only,
                    },
                    cancellation.clone(),
                    sender,
                ));
                while let Some(message) = receiver.next().await {
                    let failure = match message {
                        PairSyncMessage::Started(index, restore) => {
                            logs.push((index, name_of(index), restore));
                            None
                        }
                        PairSyncMessage::Progress(index, summary)
                        | PairSyncMessage::Done(index, summary) => {
                            written.insert(index, summary);
                            None
                        }
                        PairSyncMessage::Failed(index, error) => Some((index, error)),
                    };
                    let totals = totals_by_name(&written, &name_of);
                    cx.update(|cx| {
                        Self::update_task_run(&state, task_id, None, cx, |run| {
                            for (name, summary) in totals {
                                run.collection_mut(&name).writes = Some(summary);
                            }
                            if let Some((index, error)) = failure {
                                run.collection_mut(&name_of(index)).error = Some(error);
                            }
                        })
                    });
                }
                match work.await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => stopped = Some(error.to_string()),
                    Err(error) => stopped = Some(format!("The sync stopped: {error}")),
                }
                if stopped.is_some() {
                    break;
                }
            }
            cx.update(|cx| {
                if let Some(run_id) = run_id
                    && !logs.is_empty()
                {
                    state.update(cx, |app, _| {
                        app.tasks
                            .undo
                            .insert(task_id, UndoLog { run_id, connection_id, database, logs });
                    });
                }
                Self::update_task_run(
                    &state,
                    task_id,
                    Some(cancellation.is_cancelled()),
                    cx,
                    |run| {
                        run.error = stopped;
                    },
                );
            });
        })
        .detach();
    }

    /// Reverts the task's last sync run: every document it changed goes back as it was, unless
    /// it changed again since.
    pub fn undo_task_run(
        state: Entity<AppState>,
        task_id: Uuid,
        window: &mut Window,
        cx: &mut App,
    ) {
        let app = state.read(cx);
        if app.task_is_running(task_id) {
            return;
        }
        let (Some(task), Some(undo)) =
            (app.task(task_id).cloned(), app.tasks.undo.get(&task_id).cloned())
        else {
            return;
        };
        let mut names: Vec<String> = undo.logs.iter().map(|(_, name, _)| name.clone()).collect();
        names.sort();
        names.dedup();
        let when = app
            .task_run(task_id, undo.run_id)
            .map(|run| {
                run.started_at.with_timezone(&chrono::Local).format("%b %-d, %H:%M").to_string()
            })
            .unwrap_or_default();
        let request = WriteRequest::new(
            undo.connection_id,
            undo.database.clone(),
            format!("Undo “{}”", task.name),
            Some(WriteConfirmation {
                title: format!("Undo the run of {when}?"),
                message: format!(
                    "Puts back what it changed in {} collection{} of {}. A document changed again since is left as it is.",
                    names.len(),
                    if names.len() == 1 { "" } else { "s" },
                    undo.database
                ),
                confirm_label: "Undo run".into(),
                destructive: true,
            }),
        )
        .for_writes(names.len());
        request_connection_write(state.clone(), request, window, cx, move |_, cx| {
            for name in &names {
                let key = SessionKey::new(undo.connection_id, &undo.database, name);
                if !Self::ensure_collection_writable(&state, &key, cx) {
                    return;
                }
            }
            Self::write_undo(state, task, undo, cx);
        });
    }

    fn write_undo(state: Entity<AppState>, task: SavedTask, undo: UndoLog, cx: &mut App) {
        let app = state.read(cx);
        let Some(client) = app.active_connection_client(undo.connection_id) else {
            Self::fail_before_start(&state, &task, Launch::Run, "The connection must be open.", cx);
            return;
        };
        let runtime = app.connection_manager().runtime_handle();
        let task_id = task.id;
        state.update(cx, |app, _| {
            app.tasks.undo.remove(&task_id);
        });
        let (_, cancellation) = Self::begin_task_run(&state, &task, RunTrigger::Undo, cx);
        let names: std::collections::HashMap<usize, String> =
            undo.logs.iter().map(|(index, name, _)| (*index, name.clone())).collect();
        // The last pass first: a Mirror's deletes are undone before its inserts and replacements.
        let mut logs = undo.logs;
        logs.reverse();
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        let work = runtime.spawn(undo_pairs_async(
            client,
            undo.database,
            logs,
            cancellation.clone(),
            sender,
        ));
        cx.spawn(async move |cx| {
            let name_of = |index: usize| names.get(&index).cloned().unwrap_or_default();
            let mut restored: std::collections::HashMap<usize, SyncSummary> = Default::default();
            while let Some(message) = receiver.next().await {
                let failure = match message {
                    PairSyncMessage::Progress(index, summary)
                    | PairSyncMessage::Done(index, summary) => {
                        restored.insert(index, summary);
                        None
                    }
                    PairSyncMessage::Failed(index, error) => Some((index, error)),
                    PairSyncMessage::Started(..) => None,
                };
                let totals = totals_by_name(&restored, &name_of);
                cx.update(|cx| {
                    Self::update_task_run(&state, task_id, None, cx, |run| {
                        for (name, summary) in totals {
                            run.collection_mut(&name).writes = Some(summary);
                        }
                        if let Some((index, error)) = failure {
                            run.collection_mut(&name_of(index)).error = Some(error);
                        }
                    })
                });
            }
            let _ = work.await;
            cx.update(|cx| {
                Self::update_task_run(
                    &state,
                    task_id,
                    Some(cancellation.is_cancelled()),
                    cx,
                    |_| {},
                )
            });
        })
        .detach();
    }
}

/// Run now or Preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Launch {
    Run,
    Preview,
}

impl Launch {
    fn trigger(self) -> RunTrigger {
        match self {
            Self::Run => RunTrigger::Manual,
            Self::Preview => RunTrigger::Preview,
        }
    }
}

/// What a run asks before it writes.
enum Question {
    Write(WriteRequest),
    Replace { title: String, message: String },
}

/// A Sync task's settings.
#[derive(Clone)]
struct SyncRequest {
    config: CompareConfig,
    target: Side,
    mode: SyncMode,
    excluded: Vec<String>,
}

/// The question before a run writes. When the safety limit would stop the run, it says why and
/// the answer is Run anyway.
fn confirmation(
    task: &SavedTask,
    message: String,
    stops: &[String],
    destructive: bool,
) -> WriteConfirmation {
    if stops.is_empty() {
        return WriteConfirmation {
            title: format!("Run “{}”?", task.name),
            message,
            confirm_label: "Run".into(),
            destructive,
        };
    }
    let reasons: Vec<String> = stops.iter().map(|stop| format!("• {stop}")).collect();
    WriteConfirmation {
        title: format!("Run “{}” anyway?", task.name),
        message: format!(
            "{message}\n\nThe safety limit would stop this run:\n{}",
            reasons.join("\n")
        ),
        confirm_label: "Run anyway".into(),
        destructive: true,
    }
}

/// Adds up what each pass wrote, per collection.
fn totals_by_name(
    written: &std::collections::HashMap<usize, SyncSummary>,
    name_of: &impl Fn(usize) -> String,
) -> Vec<(String, SyncSummary)> {
    let mut totals: Vec<(String, SyncSummary)> = Vec::new();
    for (index, summary) in written {
        let name = name_of(*index);
        match totals.iter_mut().find(|(existing, _)| *existing == name) {
            Some((_, total)) => total.absorb(summary),
            None => totals.push((name, summary.clone())),
        }
    }
    totals
}

/// The document counts a transfer's safety check needs, read without a tab.
struct TransferCounts {
    manager: std::sync::Arc<crate::connection::ConnectionManager>,
    mode: TransferMode,
    scope: TransferScope,
    source: Option<(mongodb::Client, String, String)>,
    target: Option<(mongodb::Client, String, String)>,
    file: PathBuf,
    whole_target: bool,
}

impl TransferCounts {
    fn new(app: &AppState, tab: &TransferTabState) -> Option<Self> {
        let config = &tab.config;
        let client = |id: Option<Uuid>| id.and_then(|id| app.active_connection_client(id));
        let source = client(config.source_connection_id)?;
        let (source, target) = match config.mode {
            TransferMode::Export => (
                Some((source, config.source_database.clone(), config.source_collection.clone())),
                None,
            ),
            TransferMode::Import => {
                let collection = if config.destination_collection.is_empty() {
                    config.source_collection.clone()
                } else {
                    config.destination_collection.clone()
                };
                let database = if config.destination_database.is_empty() {
                    config.source_database.clone()
                } else {
                    config.destination_database.clone()
                };
                (None, Some((source, database, collection)))
            }
            TransferMode::Copy => {
                let destination = client(config.destination_connection_id)?;
                let database = if config.destination_database.is_empty() {
                    config.source_database.clone()
                } else {
                    config.destination_database.clone()
                };
                let collection = if config.destination_collection.is_empty() {
                    config.source_collection.clone()
                } else {
                    config.destination_collection.clone()
                };
                (
                    Some((
                        source,
                        config.source_database.clone(),
                        config.source_collection.clone(),
                    )),
                    Some((destination, database, collection)),
                )
            }
        };
        Some(Self {
            manager: app.connection_manager(),
            mode: config.mode,
            scope: config.scope,
            source,
            target,
            file: PathBuf::from(&config.file_path),
            whole_target: tab.options.drop_before_import || tab.options.clear_before_import,
        })
    }

    /// Blocking: estimated counts, or dbStats for a whole database.
    fn count(self) -> Result<Planned, String> {
        let documents =
            |side: &Option<(mongodb::Client, String, String)>| -> Result<Option<u64>, String> {
                let Some((client, database, collection)) = side else {
                    return Ok(None);
                };
                let count = if self.scope == TransferScope::Database || collection.is_empty() {
                    let stats =
                        self.manager.database_stats(client, database).map_err(|e| e.to_string())?;
                    stats.get("objects").and_then(mongodb::bson::Bson::as_i64).unwrap_or(0) as u64
                } else {
                    self.manager
                        .estimated_document_count(client, database, collection)
                        .map_err(|e| e.to_string())?
                };
                Ok(Some(count))
            };
        let source = match self.mode {
            // An empty file imports nothing; its size says so without reading it.
            TransferMode::Import => {
                std::fs::metadata(&self.file).ok().map(|file| file.len().min(1))
            }
            _ => documents(&self.source)?,
        };
        let target = documents(&self.target)?.unwrap_or(0);
        let name = self
            .target
            .as_ref()
            .or(self.source.as_ref())
            .map(|(_, database, collection)| {
                if collection.is_empty() || self.scope == TransferScope::Database {
                    database.clone()
                } else {
                    collection.clone()
                }
            })
            .unwrap_or_default();
        Ok(Planned {
            name,
            inserts: if self.mode == TransferMode::Import { 0 } else { source.unwrap_or(0) },
            replaces: 0,
            deletes: if self.whole_target { target } else { 0 },
            target_documents: target,
            source_documents: source,
            replaces_whole_target: self.whole_target,
        })
    }
}

/// Where a transfer writes, for its confirmation.
fn transfer_target(tab: &TransferTabState) -> String {
    let (database, collection) = match tab.config.mode {
        TransferMode::Copy => {
            (&tab.config.destination_database, &tab.config.destination_collection)
        }
        _ => (&tab.config.source_database, &tab.config.source_collection),
    };
    match tab.config.scope {
        TransferScope::Collection if !collection.is_empty() => format!("{database}.{collection}"),
        _ => database.clone(),
    }
}

/// Which collections a Sync task writes, in listing order, and why the others are left alone.
fn sync_plan(
    pairs: &[CollectionPair],
    target: Side,
    excluded: &[String],
    skip: &[String],
) -> (Vec<PairSync>, Vec<(String, &'static str)>) {
    let target = side_index(target);
    let mut plan = Vec::new();
    let mut notes = Vec::new();
    for pair in pairs {
        let kinds = pair.sides.each_ref().map(|side| side.as_ref().map(|side| side.kind));
        let note = if excluded.contains(&pair.name) || skip.contains(&pair.name) {
            Some("Left out of this task")
        } else if kinds[1 - target].is_none() {
            Some("Exists only in the target, so it is left alone")
        } else if kinds.iter().flatten().any(|kind| *kind != CollectionKind::Collection) {
            Some("A view or time-series collection isn't synced")
        } else {
            None
        };
        match note {
            Some(note) => notes.push((pair.name.clone(), note)),
            None => plan.push(PairSync {
                index: plan.len(),
                name: pair.name.clone(),
                create: kinds[target].is_none(),
            }),
        }
    }
    (plan, notes)
}

/// Why a database comparison leaves a collection pair out, if it does.
fn pair_note(pair: &CollectionPair) -> Option<&'static str> {
    use crate::connection::ops::compare_database::PairKind;
    match pair.kind() {
        PairKind::Both => None,
        PairKind::LeftOnly => Some("Exists only on the left"),
        PairKind::RightOnly => Some("Exists only on the right"),
        PairKind::NotComparable(CollectionKind::View) => Some("View, not compared"),
        PairKind::NotComparable(_) => Some("Time-series collection, not compared"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sync_writes_what_the_source_has_and_creates_what_the_target_lacks() {
        use crate::connection::ops::compare_database::SideCollection;
        let side =
            |kind| Some(SideCollection { kind, estimated: None, bytes: None, indexes: None });
        let pair = |name: &str, sides| CollectionPair { name: name.into(), sides };
        let pairs = vec![
            pair("both", [side(CollectionKind::Collection), side(CollectionKind::Collection)]),
            pair("new", [side(CollectionKind::Collection), None]),
            pair("target_only", [None, side(CollectionKind::Collection)]),
            pair("view", [side(CollectionKind::View), side(CollectionKind::View)]),
            pair("audit", [side(CollectionKind::Collection), side(CollectionKind::Collection)]),
            pair("skipped", [side(CollectionKind::Collection), None]),
        ];
        let (plan, notes) = sync_plan(&pairs, Side::Right, &["audit".into()], &["skipped".into()]);
        let planned: Vec<_> = plan.iter().map(|pair| (pair.name.as_str(), pair.create)).collect();
        assert_eq!(planned, [("both", false), ("new", true)]);
        assert_eq!(plan.iter().map(|pair| pair.index).collect::<Vec<_>>(), [0, 1]);
        let left_alone: Vec<_> = notes.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(left_alone, ["target_only", "view", "audit", "skipped"]);

        // Syncing the other way round: the right-only collection is now the new one.
        let (plan, _) = sync_plan(&pairs, Side::Left, &[], &[]);
        let planned: Vec<_> = plan.iter().map(|pair| (pair.name.as_str(), pair.create)).collect();
        assert_eq!(planned, [("both", false), ("target_only", true), ("audit", false)]);
    }

    /// Runs real tasks against a disposable MongoDB 8 server, through Run now, its questions,
    /// Preview and Undo.
    #[gpui_kit::test]
    #[ignore = "starts a MongoDB 8 container: cargo test --lib tasks_run_against_mongodb -- --ignored"]
    fn tasks_run_against_mongodb(cx: &mut gpui_kit::TestAppContext) {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        use gpui_kit::component::{Root, WindowExt as _};
        use gpui_kit::{VisualTestContext, px, size};
        use mongodb::bson::{Document, doc};
        use testcontainers::{ImageExt as _, runners::AsyncRunner as _};

        use crate::connection::ConnectionManager;
        use crate::models::{ActiveConnection, SavedConnection};
        use crate::state::ConfigManager;
        use crate::state::app_state::{TransferConfig, TransferOptions};
        use crate::state::compare::CompareEndpoint;
        use crate::tasks::model::{RunStatus, Task as SavedTask};
        use crate::tasks::store::RunStore;

        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply_design_tokens(cx);
        });
        cx.executor().allow_parking();
        let docker = tokio::runtime::Runtime::new().unwrap();
        let container = docker
            .block_on(testcontainers_modules::mongo::Mongo::default().with_tag("8.0").start())
            .unwrap();
        let uri = docker.block_on(async {
            format!(
                "mongodb://{}:{}",
                container.get_host().await.unwrap(),
                container.get_host_port_ipv4(27017).await.unwrap()
            )
        });
        let manager = Arc::new(ConnectionManager::new());
        // The client lives on the manager's runtime, like one the app opens.
        let client = manager.runtime_handle().block_on(async {
            let client = mongodb::Client::with_uri_str(&uri).await.unwrap();
            let shop = client.database("shop");
            shop.collection("orders")
                .insert_many([
                    doc! {"_id": 1, "n": 1},
                    doc! {"_id": 2, "n": 2},
                    doc! {"_id": 3, "n": 3},
                ])
                .await
                .unwrap();
            shop.collection("customers")
                .insert_many([doc! {"_id": 1}, doc! {"_id": 2}])
                .await
                .unwrap();
            // 150 documents only the target has: a Mirror would delete them, past the limit.
            let mut copy = vec![doc! {"_id": 1, "n": 1}, doc! {"_id": 2, "n": 20}];
            copy.extend((1000..1150).map(|id| doc! {"_id": id, "n": 0}));
            client.database("shop_copy").collection("orders").insert_many(copy).await.unwrap();
            client.database("shop").create_collection("empty").await.unwrap();
            client
        });
        let read = |filter: Document, database: &str, collection: &str| {
            manager.runtime_handle().block_on(async {
                client
                    .database(database)
                    .collection::<Document>(collection)
                    .count_documents(filter)
                    .await
                    .unwrap()
            })
        };

        let directory = tempfile::tempdir().unwrap();
        let saved = SavedConnection::new("Test".into(), uri.clone());
        let connection = saved.id;
        let state = cx.new(|_| {
            let mut state = AppState::with_config(
                manager.clone(),
                ConfigManager::with_config_dir(directory.path().into()),
            );
            state.connections = vec![saved.clone()];
            state.insert_active_connection(
                connection,
                ActiveConnection {
                    config: saved.clone(),
                    client: client.clone(),
                    databases: vec!["shop".into(), "shop_copy".into()],
                    collections: Default::default(),
                    collection_details: Default::default(),
                    runtime_meta: Default::default(),
                },
            );
            state.attach_task_runs(RunStore::in_memory().unwrap(), None);
            state
        });
        // The app's root view draws the dialog layer; this host stands in for it.
        struct DialogHost;
        impl gpui_kit::Render for DialogHost {
            fn render(
                &mut self,
                window: &mut Window,
                cx: &mut gpui_kit::Context<Self>,
            ) -> impl gpui_kit::IntoElement {
                use gpui_kit::{ParentElement as _, Styled as _};
                gpui_kit::div().size_full().children(Root::render_dialog_layer(window, cx))
            }
        }
        let (_, cx) = cx.add_window_view(|window, cx| {
            let host = cx.new(|_| DialogHost);
            Root::new(host, window, cx).bordered(false)
        });
        cx.simulate_resize(size(px(1200.0), px(900.0)));

        let save = |cx: &mut VisualTestContext, task: &SavedTask| {
            state.update(cx, |app, _| app.upsert_task(task.clone()).unwrap());
        };
        let run = |cx: &mut VisualTestContext, task: &SavedTask, preview: bool| {
            let (state, id) = (state.clone(), task.id);
            cx.update(|window, cx| {
                if preview {
                    AppCommands::preview_task(state, id, window, cx)
                } else {
                    AppCommands::run_task(state, id, window, cx)
                }
            });
        };
        let settle = |cx: &mut VisualTestContext| {
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.run_until_parked();
        };
        // Waits for the question and returns its title and message.
        let question = |cx: &mut VisualTestContext| -> String {
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                settle(cx);
                if cx.update(|window, cx| window.has_active_dialog(cx)) {
                    return cx.update(|window, _| {
                        gpui_kit::base::test_support::snapshots(window)
                            .into_iter()
                            .filter_map(|node| node.label().map(str::to_string))
                            .collect::<Vec<_>>()
                            .join(" | ")
                    });
                }
                assert!(Instant::now() < deadline, "no question was asked");
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let answer = |cx: &mut VisualTestContext| {
            let button = cx
                .update(|window, _| gpui_kit::base::test_support::snapshots(window))
                .into_iter()
                .find(|node| {
                    node.path().last() == Some(&gpui_kit::ElementId::from("confirm-action"))
                })
                .expect("the question's answer button");
            cx.simulate_click(button.bounds().center(), Default::default());
        };
        let finished = |cx: &mut VisualTestContext, task: &SavedTask| -> Run {
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                settle(cx);
                let run = state.read_with(cx, |app, _| app.task_runs(task.id).first().cloned());
                if let Some(run) = run.filter(|run| run.status != RunStatus::Running) {
                    return run;
                }
                assert!(Instant::now() < deadline, "{} didn't finish", task.name);
                std::thread::sleep(Duration::from_millis(50));
            }
        };

        // Export, through Run now and a Transfer tab state no tab shows. Nothing to ask.
        let file = directory.path().join("orders.jsonl");
        let export = SavedTask::new(
            "Export orders".into(),
            TaskSpec::Transfer {
                config: TransferConfig {
                    mode: TransferMode::Export,
                    scope: TransferScope::Collection,
                    source_connection_id: Some(connection),
                    source_database: "shop".into(),
                    source_collection: "orders".into(),
                    file_path: file.display().to_string(),
                    ..Default::default()
                },
                options: TransferOptions::default(),
            },
        );
        save(cx, &export);
        run(cx, &export, false);
        let done = finished(cx, &export);
        assert_eq!(done.status, RunStatus::Succeeded, "{done:?}");
        assert_eq!(done.documents(), 3);
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 3);
        // The file exists now, so the next run asks before replacing it.
        run(cx, &export, false);
        assert!(question(cx).contains("Replace"));
        answer(cx);
        assert_eq!(finished(cx, &export).status, RunStatus::Succeeded);

        // A database comparison.
        let mut config = CompareConfig { scope: CompareScope::Databases, ..Default::default() };
        config.sides = [
            CompareEndpoint {
                connection_id: Some(connection),
                database: "shop".into(),
                collection: String::new(),
            },
            CompareEndpoint {
                connection_id: Some(connection),
                database: "shop_copy".into(),
                collection: String::new(),
            },
        ];
        let compare =
            SavedTask::new("Compare shop".into(), TaskSpec::Compare { config: config.clone() });
        save(cx, &compare);
        run(cx, &compare, false);
        let done = finished(cx, &compare);
        assert_eq!(done.status, RunStatus::Succeeded, "{done:?}");
        let orders = done.collections.iter().find(|c| c.name == "orders").unwrap();
        let counts = orders.differences.unwrap();
        assert_eq!((counts.different, counts.only_left, counts.only_right), (1, 1, 150));

        // A Mirror into shop_copy. Preview works out what it would do and writes nothing.
        let mirror = SavedTask::new(
            "Mirror shop".into(),
            TaskSpec::Sync {
                config: config.clone(),
                target: Side::Right,
                mode: SyncMode::Mirror,
                excluded: vec!["empty".into()],
            },
        );
        save(cx, &mirror);
        run(cx, &mirror, true);
        let preview = finished(cx, &mirror);
        assert_eq!(preview.trigger, RunTrigger::Preview);
        let orders = preview.collections.iter().find(|c| c.name == "orders").unwrap();
        assert_eq!(orders.planned, Some([1, 1, 150]));
        assert_eq!(preview.stops.len(), 1, "150 of 152 is past the limit: {:?}", preview.stops);
        assert_eq!(read(doc! {}, "shop_copy", "orders"), 152, "a preview writes nothing");

        // Run now asks with the reason; closing the question writes nothing.
        run(cx, &mirror, false);
        assert!(question(cx).contains("anyway"));
        cx.update(|window, cx| window.close_dialog(cx));
        let declined = finished(cx, &mirror);
        assert_eq!(declined.status, RunStatus::Cancelled, "{declined:?}");
        assert!(declined.log.iter().any(|line| line.message.contains("Not confirmed")));
        assert_eq!(read(doc! {}, "shop_copy", "orders"), 152);

        // Run anyway: inserts and replacements first, then the deletes.
        run(cx, &mirror, false);
        question(cx);
        answer(cx);
        let done = finished(cx, &mirror);
        assert_eq!(done.status, RunStatus::Succeeded, "{done:?}");
        let writes = done.writes();
        assert_eq!((writes.inserted, writes.replaced, writes.deleted), (3, 1, 150));
        assert!(done.log.iter().any(|line| line.message.starts_with("Run anyway")));
        assert_eq!(read(doc! {}, "shop_copy", "orders"), 3);
        assert_eq!(read(doc! {"_id": 2, "n": 2}, "shop_copy", "orders"), 1);
        assert_eq!(read(doc! {}, "shop_copy", "customers"), 2, "created and filled");

        // Undo puts the target back as it was, deletes first.
        cx.update(|window, cx| AppCommands::undo_task_run(state.clone(), mirror.id, window, cx));
        question(cx);
        answer(cx);
        let undone = finished(cx, &mirror);
        assert_eq!(undone.trigger, RunTrigger::Undo);
        assert_eq!(undone.status, RunStatus::Succeeded, "{undone:?}");
        assert_eq!(read(doc! {}, "shop_copy", "orders"), 152);
        assert_eq!(read(doc! {"_id": 2, "n": 20}, "shop_copy", "orders"), 1);
        assert_eq!(read(doc! {}, "shop_copy", "customers"), 0);
        assert!(state.read_with(cx, |app, _| !app.tasks.undo.contains_key(&mirror.id)));

        // A copy from an empty collection that clears its target stops, whatever the floor.
        let refresh_config = TransferConfig {
            mode: TransferMode::Copy,
            scope: TransferScope::Collection,
            source_connection_id: Some(connection),
            source_database: "shop".into(),
            source_collection: "empty".into(),
            destination_connection_id: Some(connection),
            destination_database: "shop".into(),
            destination_collection: "orders".into(),
            ..Default::default()
        };
        let refresh = SavedTask::new(
            "Refresh orders".into(),
            TaskSpec::Transfer {
                config: refresh_config,
                options: TransferOptions {
                    clear_before_import: true,
                    copy_indexes: false,
                    ..Default::default()
                },
            },
        );
        save(cx, &refresh);
        run(cx, &refresh, true);
        let preview = finished(cx, &refresh);
        assert!(preview.stops.first().is_some_and(|stop| stop.contains("empty")), "{preview:?}");
        assert_eq!(read(doc! {}, "shop", "orders"), 3);

        // A plain copy, through the Transfer tab's own copy path.
        let copy = SavedTask::new(
            "Copy orders".into(),
            TaskSpec::Transfer {
                config: TransferConfig {
                    mode: TransferMode::Copy,
                    scope: TransferScope::Collection,
                    source_connection_id: Some(connection),
                    source_database: "shop".into(),
                    source_collection: "orders".into(),
                    destination_connection_id: Some(connection),
                    destination_database: "shop_backup".into(),
                    destination_collection: "orders".into(),
                    ..Default::default()
                },
                options: TransferOptions::default(),
            },
        );
        save(cx, &copy);
        run(cx, &copy, false);
        question(cx);
        answer(cx);
        let done = finished(cx, &copy);
        assert_eq!(done.status, RunStatus::Succeeded, "{done:?}");
        assert_eq!(read(doc! {}, "shop_backup", "orders"), 3);
        assert!(state.read_with(cx, |app, _| app.tasks.active.is_empty()));
        // The container stops through Tokio, so it is dropped inside a runtime.
        docker.block_on(async move { drop(container) });
    }
}
