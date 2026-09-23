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
    SyncMode, compare_pairs_async, list_side, pair_collections, sync_pairs_async,
};
use crate::state::app_state::{ActiveRun, CollectionTransferStatus, RunStop, TransferTabState};
use crate::state::compare::{CompareConfig, CompareScope};
use crate::state::{
    AppEvent, AppState, SessionKey, StatusMessage, TransferMode, TransferScope,
    resolved_export_destination, validate_transfer,
};
use crate::tasks::model::{LogLevel, Run, RunTrigger, Task as SavedTask, TaskSpec, side_index};

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

    /// Runs the task now. Closed connections are opened first; a task that writes asks before
    /// it starts, in the same dialog that confirms writes to Production.
    pub fn run_task(state: Entity<AppState>, task_id: Uuid, window: &mut Window, cx: &mut App) {
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
                "A connection this task uses no longer exists. Edit the task to choose another.",
                cx,
            );
            return;
        }
        let closed: Vec<Uuid> =
            connections.into_iter().filter(|id| !app.is_connected(*id)).collect();
        if closed.is_empty() {
            Self::confirm_task_run(state, task, window, cx);
            return;
        }
        state.update(cx, |app, cx| {
            app.tasks.starting.insert(task_id);
            cx.notify();
        });
        let waits: Vec<_> =
            closed.into_iter().map(|id| Self::connect_and_wait(state.clone(), id, cx)).collect();
        let window = window.window_handle();
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
                if let Some(error) = failure {
                    Self::fail_before_start(
                        &state,
                        &task,
                        &format!("Could not connect: {error}"),
                        cx,
                    );
                    return;
                }
                let _ = cx.update_window(window, |_, window, cx| {
                    Self::confirm_task_run(state, task, window, cx)
                });
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

    fn confirm_task_run(
        state: Entity<AppState>,
        task: SavedTask,
        window: &mut Window,
        cx: &mut App,
    ) {
        match task.spec.clone() {
            TaskSpec::Transfer { config, options } => {
                let mut tab = TransferTabState::from_settings(&state.read(cx).settings);
                tab.config = config;
                tab.options = options;
                let validation = validate_transfer(&tab);
                if !validation.can_run() {
                    let reason = validation
                        .blocking_errors
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "The transfer isn't ready to run.".into());
                    Self::fail_before_start(&state, &task, &reason, cx);
                    return;
                }
                let destination = resolved_export_destination(&tab);
                if let Some(connection) = task.spec.write_connection() {
                    let confirmation = WriteConfirmation {
                        title: format!("Run “{}”?", task.name),
                        message: format!(
                            "{} writes into {}.",
                            task.spec.kind().label(),
                            transfer_target(&tab)
                        ),
                        confirm_label: "Run".into(),
                        destructive: tab.options.drop_before_import
                            || tab.options.clear_before_import,
                    };
                    let request = WriteRequest::new(
                        connection,
                        transfer_target(&tab),
                        format!("Run “{}”", task.name),
                        Some(confirmation),
                    );
                    request_connection_write(state.clone(), request, window, cx, move |_, cx| {
                        Self::start_transfer_run(state, task, tab, None, cx)
                    });
                } else if let Some(path) = destination.filter(|path| path.exists()) {
                    let file = path.display().to_string();
                    open_confirm_dialog(
                        window,
                        cx,
                        format!("Replace {file}?"),
                        format!("“{}” exports to a file that already exists.", task.name),
                        "Replace",
                        true,
                        move |_, cx| Self::start_transfer_run(state, task, tab, Some(path), cx),
                    );
                } else {
                    Self::start_transfer_run(state, task, tab, None, cx);
                }
            }
            TaskSpec::Compare { config } => Self::start_compare_run(state, task, config, cx),
            TaskSpec::Sync { config, target, mode, excluded } => {
                let window = window.window_handle();
                Self::plan_sync_run(state, task, config, target, mode, excluded, window, cx);
            }
        }
    }

    /// Records a run that failed before any work started, so the reason shows in its history.
    fn fail_before_start(state: &Entity<AppState>, task: &SavedTask, reason: &str, cx: &mut App) {
        let mut run = Run::start(task.id, RunTrigger::Manual);
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

    /// Starts the run's record and marks the task running.
    fn begin_task_run(
        state: &Entity<AppState>,
        task: &SavedTask,
        stop: RunStop,
        transfer_id: Option<Uuid>,
        events: Option<gpui_kit::Subscription>,
        cx: &mut App,
    ) -> Uuid {
        let mut run = Run::start(task.id, RunTrigger::Manual);
        run.log(LogLevel::Info, format!("Started: {}", task.spec.subject()));
        let run_id = run.id;
        state.update(cx, |app, cx| {
            app.record_task_run(run, true);
            app.tasks
                .active
                .insert(task.id, ActiveRun { run_id, stop, transfer_id, _events: events });
            cx.notify();
        });
        run_id
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

    fn start_transfer_run(
        state: Entity<AppState>,
        task: SavedTask,
        tab: TransferTabState,
        confirmed_overwrite: Option<PathBuf>,
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
        let task_id = task.id;
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
        Self::begin_task_run(
            &state,
            &task,
            RunStop::Transfer(transfer_id),
            Some(transfer_id),
            Some(events),
            cx,
        );
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
            Self::fail_before_start(&state, &task, "Both connections must be open.", cx);
            return;
        };
        let runtime = app.connection_manager().runtime_handle();
        let timeout = Duration::from_millis(app.settings.interactive_query_timeout_ms.max(100));
        let cancellation = CancellationToken::new();
        let task_id = task.id;
        Self::begin_task_run(&state, &task, RunStop::Token(cancellation.clone()), None, None, cx);

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

    /// Lists both databases, works out which collections the sync writes, then asks before
    /// writing. The count is known before the question, so the dialog can say it.
    #[allow(clippy::too_many_arguments)]
    fn plan_sync_run(
        state: Entity<AppState>,
        task: SavedTask,
        config: CompareConfig,
        target: Side,
        mode: SyncMode,
        excluded: Vec<String>,
        window: AnyWindowHandle,
        cx: &mut App,
    ) {
        let app = state.read(cx);
        let clients = config
            .sides
            .each_ref()
            .map(|side| side.connection_id.and_then(|id| app.active_connection_client(id)));
        let [Some(left), Some(right)] = clients else {
            Self::fail_before_start(&state, &task, "Both connections must be open.", cx);
            return;
        };
        let runtime = app.connection_manager().runtime_handle();
        let timeout = Duration::from_millis(app.settings.interactive_query_timeout_ms.max(100));
        let databases = config.sides.each_ref().map(|side| side.database.clone());
        state.update(cx, |app, cx| {
            app.tasks.starting.insert(task.id);
            cx.notify();
        });
        cx.spawn(async move |cx| {
            let listed = runtime
                .spawn(async move {
                    let (l, r) = tokio::join!(
                        list_side(&left, &databases[0], timeout),
                        list_side(&right, &databases[1], timeout)
                    );
                    Ok::<_, crate::error::Error>(pair_collections(l?, r?))
                })
                .await;
            cx.update(|cx| {
                state.update(cx, |app, cx| {
                    app.tasks.starting.remove(&task.id);
                    cx.notify();
                });
                let pairs = match listed {
                    Ok(Ok(pairs)) => pairs,
                    Ok(Err(error)) => {
                        return Self::fail_before_start(&state, &task, &error.to_string(), cx);
                    }
                    Err(error) => {
                        return Self::fail_before_start(&state, &task, &error.to_string(), cx);
                    }
                };
                let (plan, notes) = sync_plan(&pairs, target, &excluded, &config.skip);
                if plan.is_empty() {
                    let mut run = Run::start(task.id, RunTrigger::Manual);
                    for (name, note) in notes {
                        run.collection_mut(&name).note = Some(note.into());
                    }
                    run.log(
                        LogLevel::Info,
                        "Nothing to sync: the source has no collections this task writes.",
                    );
                    run.finish(false);
                    state.update(cx, |app, cx| {
                        app.record_task_run(run, true);
                        cx.notify();
                    });
                    return;
                }
                let _ = cx.update_window(window, |_, window, cx| {
                    Self::confirm_sync_run(
                        state, task, config, target, mode, plan, notes, window, cx,
                    )
                });
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn confirm_sync_run(
        state: Entity<AppState>,
        task: SavedTask,
        config: CompareConfig,
        target: Side,
        mode: SyncMode,
        plan: Vec<PairSync>,
        notes: Vec<(String, &'static str)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let index = side_index(target);
        let Some(connection) = config.sides[index].connection_id else {
            return;
        };
        let database = config.sides[index].database.clone();
        let count = plan.len();
        let deletes = if mode == SyncMode::Mirror {
            " Mirror also deletes documents that exist only in the target."
        } else {
            ""
        };
        let confirmation = WriteConfirmation {
            title: format!("Run “{}”?", task.name),
            message: format!(
                "{} writes into {count} collection{} of {database}.{deletes}",
                mode.label(),
                if count == 1 { "" } else { "s" }
            ),
            confirm_label: "Run".into(),
            destructive: mode == SyncMode::Mirror,
        };
        let request = WriteRequest::new(
            connection,
            database.clone(),
            format!("Run “{}”", task.name),
            Some(confirmation),
        )
        .for_writes(count);
        request_connection_write(state.clone(), request, window, cx, move |_, cx| {
            // One production grant per collection, spent before anything is written.
            for pair in &plan {
                let key = SessionKey::new(connection, &database, &pair.name);
                if !Self::ensure_collection_writable(&state, &key, cx) {
                    return;
                }
            }
            Self::start_sync_run(state, task, config, target, mode, plan, notes, cx)
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn start_sync_run(
        state: Entity<AppState>,
        task: SavedTask,
        config: CompareConfig,
        target: Side,
        mode: SyncMode,
        plan: Vec<PairSync>,
        notes: Vec<(String, &'static str)>,
        cx: &mut App,
    ) {
        let app = state.read(cx);
        let clients = config
            .sides
            .each_ref()
            .map(|side| side.connection_id.and_then(|id| app.active_connection_client(id)));
        let [Some(left), Some(right)] = clients else {
            Self::fail_before_start(&state, &task, "Both connections must be open.", cx);
            return;
        };
        let runtime = app.connection_manager().runtime_handle();
        let restore_dir = app.compare_restore_dir();
        let ignore = config.ignore_set();
        let databases = config.sides.each_ref().map(|side| side.database.clone());
        let cancellation = CancellationToken::new();
        let task_id = task.id;
        Self::begin_task_run(&state, &task, RunStop::Token(cancellation.clone()), None, None, cx);
        let names: Vec<String> = plan.iter().map(|pair| pair.name.clone()).collect();
        Self::update_task_run(&state, task_id, None, cx, |run| {
            for name in &names {
                run.collection_mut(name);
            }
            for (name, note) in notes {
                run.collection_mut(&name).note = Some(note.into());
            }
        });
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        let work = runtime.spawn(sync_pairs_async(
            DatabaseSync {
                clients: [left, right],
                databases,
                target,
                mode,
                pairs: plan,
                ignore,
                restore_dir,
                pass_rows: MAX_ROWS,
            },
            cancellation.clone(),
            sender,
        ));
        cx.spawn(async move |cx| {
            while let Some(message) = receiver.next().await {
                cx.update(|cx| {
                    Self::update_task_run(&state, task_id, None, cx, |run| match message {
                        PairSyncMessage::Progress(i, summary)
                        | PairSyncMessage::Done(i, summary) => {
                            run.collection_mut(&names[i]).writes = Some(summary)
                        }
                        PairSyncMessage::Failed(i, error) => {
                            run.collection_mut(&names[i]).error = Some(error)
                        }
                        PairSyncMessage::Started(..) => {}
                    })
                });
            }
            let result = work.await;
            cx.update(|cx| {
                Self::update_task_run(
                    &state,
                    task_id,
                    Some(cancellation.is_cancelled()),
                    cx,
                    |run| match result {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => run.error = Some(error.to_string()),
                        Err(error) => run.error = Some(format!("The sync stopped: {error}")),
                    },
                );
            });
        })
        .detach();
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

    /// Runs real tasks against a disposable MongoDB 8 server, through the same code as Run now.
    #[gpui_kit::test]
    #[ignore = "starts a MongoDB 8 container: cargo test --lib tasks_run_against_mongodb -- --ignored"]
    fn tasks_run_against_mongodb(cx: &mut gpui_kit::TestAppContext) {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        use mongodb::bson::doc;
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
            client
                .database("shop_copy")
                .collection("orders")
                .insert_many([doc! {"_id": 1, "n": 1}, doc! {"_id": 2, "n": 20}])
                .await
                .unwrap();
            client
        });

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
        let save = |cx: &mut gpui_kit::TestAppContext, task: &SavedTask| {
            state.update(cx, |app, _| app.upsert_task(task.clone()).unwrap());
        };
        let finished = |cx: &mut gpui_kit::TestAppContext, task: &SavedTask| -> Run {
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                cx.run_until_parked();
                let run = state.read_with(cx, |app, _| app.task_runs(task.id).first().cloned());
                if let Some(run) = run.filter(|run| run.status != RunStatus::Running) {
                    return run;
                }
                assert!(Instant::now() < deadline, "{} didn't finish", task.name);
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        struct Blank;
        impl gpui_kit::Render for Blank {
            fn render(
                &mut self,
                _: &mut Window,
                _: &mut gpui_kit::Context<Self>,
            ) -> impl gpui_kit::IntoElement {
                gpui_kit::div()
            }
        }
        let window = cx.add_window(|_, _| Blank);

        // Export, through Run now and a Transfer tab state no tab shows.
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
        window
            .update(cx, |_, window, cx| AppCommands::run_task(state.clone(), export.id, window, cx))
            .unwrap();
        let run = finished(cx, &export);
        assert_eq!(run.status, RunStatus::Succeeded, "{run:?}");
        assert_eq!(run.documents(), 3);
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 3);
        assert!(
            state.read_with(cx, |app, _| app.tasks.active.is_empty()),
            "the run no longer counts as running"
        );

        // A database comparison, through Run now.
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
        window
            .update(cx, |_, window, cx| {
                AppCommands::run_task(state.clone(), compare.id, window, cx)
            })
            .unwrap();
        let run = finished(cx, &compare);
        assert_eq!(run.status, RunStatus::Succeeded, "{run:?}");
        let orders = run.collections.iter().find(|c| c.name == "orders").unwrap();
        let counts = orders.differences.unwrap();
        assert_eq!((counts.different, counts.only_left, counts.identical), (1, 1, 1));
        let customers = run.collections.iter().find(|c| c.name == "customers").unwrap();
        assert_eq!(customers.note.as_deref(), Some("Exists only on the left"));

        // A Mirror sync into shop_copy. Run now asks first; this starts where the answer leads.
        let sync = SavedTask::new(
            "Mirror shop".into(),
            TaskSpec::Sync {
                config: config.clone(),
                target: Side::Right,
                mode: SyncMode::Mirror,
                excluded: vec![],
            },
        );
        save(cx, &sync);
        let pairs = manager.runtime_handle().block_on(async {
            let timeout = Duration::from_secs(10);
            pair_collections(
                list_side(&client, "shop", timeout).await.unwrap(),
                list_side(&client, "shop_copy", timeout).await.unwrap(),
            )
        });
        let (plan, notes) = sync_plan(&pairs, Side::Right, &[], &[]);
        cx.update(|cx| {
            AppCommands::start_sync_run(
                state.clone(),
                sync.clone(),
                config,
                Side::Right,
                SyncMode::Mirror,
                plan,
                notes,
                cx,
            )
        });
        let run = finished(cx, &sync);
        assert_eq!(run.status, RunStatus::Succeeded, "{run:?}");
        let writes = run.writes();
        assert_eq!((writes.inserted, writes.replaced, writes.deleted), (3, 1, 0));
        let copied = manager.runtime_handle().block_on(async {
            let target = client.database("shop_copy");
            let changed = target
                .collection::<mongodb::bson::Document>("orders")
                .find_one(doc! {"_id": 2})
                .await
                .unwrap();
            (
                target
                    .collection::<mongodb::bson::Document>("orders")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                target
                    .collection::<mongodb::bson::Document>("customers")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                changed.unwrap().get_i32("n").unwrap(),
            )
        });
        assert_eq!(copied, (3, 2, 2), "orders mirrored, customers created");

        // A copy, through the Transfer tab's own copy path.
        let copy_config = TransferConfig {
            mode: TransferMode::Copy,
            scope: TransferScope::Collection,
            source_connection_id: Some(connection),
            source_database: "shop".into(),
            source_collection: "orders".into(),
            destination_connection_id: Some(connection),
            destination_database: "shop_backup".into(),
            destination_collection: "orders".into(),
            ..Default::default()
        };
        let copy = SavedTask::new(
            "Copy orders".into(),
            TaskSpec::Transfer { config: copy_config.clone(), options: TransferOptions::default() },
        );
        save(cx, &copy);
        cx.update(|cx| {
            let mut tab = TransferTabState::from_settings(&state.read(cx).settings);
            tab.config = copy_config;
            AppCommands::start_transfer_run(state.clone(), copy.clone(), tab, None, cx)
        });
        let run = finished(cx, &copy);
        assert_eq!(run.status, RunStatus::Succeeded, "{run:?}");
        assert_eq!(run.documents(), 3);
        let backup = manager.runtime_handle().block_on(async {
            client
                .database("shop_backup")
                .collection::<mongodb::bson::Document>("orders")
                .count_documents(doc! {})
                .await
                .unwrap()
        });
        assert_eq!(backup, 3);
        assert!(
            state.read_with(cx, |app, _| app.tasks.active.is_empty()),
            "no hidden Transfer tab is left running"
        );
        // The container stops through Tokio, so it is dropped inside a runtime.
        docker.block_on(async move { drop(container) });
    }
}
