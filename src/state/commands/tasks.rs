//! Running saved tasks.
//!
//! A task runs through the same code as its tab. Transfers go through a Transfer tab state that no
//! tab shows, so validation, production confirmation and every export, import and copy path are
//! the Transfer tab's own. Comparisons and syncs call the engines the Compare tab calls.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt as _;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, AsyncApp, Entity, Task, Window};
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
use crate::tasks::model::{
    FailureKind, LogLevel, Run, RunTrigger, Task as SavedTask, TaskSpec, side_index,
};
use crate::tasks::safety::{self, Planned, StopReason};
use crate::tasks::schedule::{prune_exports, stamped_path};

use super::AppCommands;
use super::task_run::{
    self, Reconnect, RunConnections, Watch, open_connections, retry_once, run_steps,
};
use crate::error::Failure;

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

    /// Runs the task now. A task that writes works out what it would change, then asks before
    /// writing.
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
        if app.task_is_running(task_id) {
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
        // Every run opens connections of its own, so nothing needs to be open in the sidebar.
        let window = window.window_handle();
        Self::start_task(state, task, launch, Some(window), cx);
    }

    /// Starts a run. Without a window nobody is there to ask, as for a scheduled run: where Run
    /// now would ask, the safety limit stops the run instead, and the schedule's approval stands
    /// in for the Production write confirmation.
    pub(super) fn start_task(
        state: Entity<AppState>,
        task: SavedTask,
        launch: Launch,
        window: Option<AnyWindowHandle>,
        cx: &mut App,
    ) {
        match task.spec.clone() {
            TaskSpec::Transfer { config, options } => {
                let mut tab = TransferTabState::from_settings(&state.read(cx).settings);
                tab.config = config;
                tab.options = options;
                // Each scheduled export gets a file of its own, named by the time it was due.
                if let Launch::Schedule { due, .. } = launch
                    && tab.config.mode == TransferMode::Export
                {
                    let at = due.with_timezone(&chrono::Local).naive_local();
                    tab.config.file_path = stamped_path(&tab.config.file_path, at);
                }
                Self::start_transfer_task(state, task, tab, launch, window, cx);
            }
            // A comparison writes nothing, so its preview is the run itself.
            TaskSpec::Compare { config } => {
                Self::start_compare_run(state, task, config, launch.trigger(), cx)
            }
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
        window: Option<AnyWindowHandle>,
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
        if launch != Launch::Preview && !(writes && whole_target) {
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
        window: Option<AnyWindowHandle>,
        cx: &mut App,
    ) {
        let task_id = task.id;
        let Some(window) = window else {
            if !stops.is_empty() {
                return Self::fail_run(&state, task_id, SAFETY_STOPPED.into(), cx);
            }
            // A scheduled export's file is named by its run, so one that exists is replaced.
            let replace = resolved_export_destination(&tab).filter(|path| path.exists());
            return Self::start_transfer_run(state, task_id, tab, replace, false, false, cx);
        };
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
                Self::start_transfer_run(state, task_id, tab, None, anyway, true, cx)
            });
        } else if let Some(path) = resolved_export_destination(&tab).filter(|path| path.exists()) {
            let question = Question::Replace {
                title: format!("Replace {}?", path.display()),
                message: format!("“{}” exports to a file that already exists.", task.name),
            };
            Self::ask(state.clone(), task_id, window, question, cx, move |cx| {
                Self::start_transfer_run(state, task_id, tab, Some(path), false, true, cx)
            });
        } else {
            Self::start_transfer_run(state, task_id, tab, None, false, true, cx);
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
    pub(super) fn fail_before_start(
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
        if state.read(cx).tasks.background {
            run.log(LogLevel::Info, "Started while OpenMango was closed.");
        }
        let run_id = run.id;
        let cancellation = CancellationToken::new();
        // A scheduled run stops retrying when the task's next run is due.
        let retry_until = matches!(trigger, RunTrigger::Schedule | RunTrigger::CatchUp)
            .then(|| task.schedule.next_after(&chrono::Local::now()))
            .flatten()
            .and_then(|next| (next.with_timezone(&chrono::Utc) - chrono::Utc::now()).to_std().ok())
            .map(|wait| std::time::Instant::now() + wait);
        state.update(cx, |app, cx| {
            app.record_task_run(run, true);
            app.tasks.active.insert(
                task.id,
                ActiveRun {
                    run_id,
                    stop: RunStop::Token(cancellation.clone()),
                    transfer_id: None,
                    retry_until,
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
        if done.is_some() {
            let state = state.clone();
            cx.defer(move |cx| Self::start_queued(&state, cx));
        }
    }

    /// When a scheduled run stops retrying: when its task's next run is due.
    fn retry_until(
        state: &Entity<AppState>,
        task_id: Uuid,
        cx: &App,
    ) -> Option<std::time::Instant> {
        state.read(cx).tasks.active.get(&task_id).and_then(|active| active.retry_until)
    }

    /// Runs the transfer inside the task's run, on connections of its own, through a Transfer
    /// tab state no tab shows. A transfer that starts over cleanly (an export, or an import or
    /// copy that clears or drops its target first) is run again after a failure that can pass.
    /// `attended`: someone chose Run now and answered its question, which granted the first
    /// try's Production write. A scheduled run grants its own, as its approval allows.
    #[allow(clippy::too_many_arguments)]
    fn start_transfer_run(
        state: Entity<AppState>,
        task_id: Uuid,
        tab: TransferTabState,
        confirmed_overwrite: Option<PathBuf>,
        anyway: bool,
        attended: bool,
        cx: &mut App,
    ) {
        let Some(task) = state.read(cx).task(task_id).cloned() else {
            return;
        };
        let Some(reconnect) = Self::reconnect_for(&state, &task, cx) else {
            return Self::fail_run(
                &state,
                task_id,
                "A connection this task uses no longer exists.".into(),
                cx,
            );
        };
        let Some(RunStop::Token(cancellation)) =
            state.read(cx).tasks.active.get(&task_id).map(|active| match &active.stop {
                RunStop::Token(token) => RunStop::Token(token.clone()),
                RunStop::Transfer(id) => RunStop::Transfer(*id),
            })
        else {
            return;
        };
        if anyway {
            Self::update_task_run(&state, task_id, None, cx, |run| {
                run.log(LogLevel::Warning, "Run anyway: confirmed despite the safety limit.");
            });
        }
        let restarts = tab.config.mode == TransferMode::Export
            || tab.options.drop_before_import
            || tab.options.clear_before_import;
        let write_connection = task.spec.write_connection();
        // A scheduled export may keep only its newest files, named from the task's own path.
        let prune = match (&task.spec, task.keep_files) {
            (TaskSpec::Transfer { config, .. }, Some(keep)) if !attended => {
                let mut original = tab.clone();
                original.config.file_path = config.file_path.clone();
                resolved_export_destination(&original).map(|path| (path, keep as usize))
            }
            _ => None,
        };
        let until = Self::retry_until(&state, task_id, cx);
        let scope = tab.config.scope;
        let collection = match tab.config.mode {
            TransferMode::Import if tab.config.source_collection.is_empty() => {
                tab.config.destination_collection.clone()
            }
            _ => tab.config.source_collection.clone(),
        };

        cx.spawn(async move |cx| {
            let mut watch = Watch::new(cancellation.clone(), until);
            let log = Self::run_log(&state, task_id);
            let connections = match open_connections(cx, &mut watch, &reconnect, log).await {
                Ok(connections) => connections,
                Err(failure) => return Self::end_run(cx, &state, task_id, &watch, Some(failure)),
            };
            let mut connections = connections;
            let mut tries = 1;
            loop {
                let (outcome, after) = Self::transfer_attempt(
                    cx,
                    &state,
                    task_id,
                    tab.clone(),
                    confirmed_overwrite.clone(),
                    &connections,
                    (tries > 1 || !attended).then_some(write_connection).flatten(),
                )
                .await;
                // Between attempts, Cancel stops the wait.
                cx.update(|cx| {
                    state.update(cx, |app, _| {
                        if let Some(active) = app.tasks.active.get_mut(&task_id) {
                            active.stop = RunStop::Token(cancellation.clone());
                            active.transfer_id = None;
                            active._events = None;
                        }
                    })
                });
                let transient = after.as_ref().is_some_and(|tab| tab.runtime.failure_transient);
                let retry = matches!(outcome, TransferOutcome::Failed(_))
                    && transient
                    && restarts
                    && tries < task_run::ATTEMPTS
                    && !watch.over()
                    && watch.may_retry();
                if retry {
                    let TransferOutcome::Failed(error) = &outcome else { unreachable!() };
                    let wait = task_run::backoff(tries);
                    Self::run_log(&state, task_id)(
                        cx,
                        format!(
                            "{error}. Starting over (attempt {} of {}) after {} s.",
                            tries + 1,
                            task_run::ATTEMPTS,
                            wait.as_secs()
                        ),
                    );
                    task_run::pause(cx, wait, &watch.run).await;
                    if !watch.over() {
                        if let Ok(fresh) = reconnect.open(cx).await {
                            connections = fresh;
                        }
                        tries += 1;
                        continue;
                    }
                }
                let not_restartable = matches!(outcome, TransferOutcome::Failed(_)) && transient && !restarts;
                let cancelled = matches!(outcome, TransferOutcome::Cancelled) || watch.run.is_cancelled();
                let pruned = match (&prune, &outcome) {
                    (Some((path, keep)), TransferOutcome::Completed(_)) => {
                        Some(prune_exports(path, *keep).map_err(|error| error.to_string()))
                    }
                    _ => None,
                };
                cx.update(|cx| {
                    Self::update_task_run(&state, task_id, None, cx, |run| {
                        record_transfer(run, scope, &collection, &outcome, after.as_ref());
                        if matches!(outcome, TransferOutcome::Failed(_)) && transient && restarts {
                            run.failure = Some(FailureKind::Temporary);
                        }
                        if not_restartable {
                            run.log(
                                LogLevel::Warning,
                                "Not run again: without Clear or Drop target first, running it again could write documents twice.",
                            );
                        }
                        match pruned {
                            Some(Ok(deleted)) if !deleted.is_empty() => run.log(
                                LogLevel::Info,
                                format!(
                                    "Deleted {} older export file{}, keeping the newest {}.",
                                    deleted.len(),
                                    if deleted.len() == 1 { "" } else { "s" },
                                    prune.as_ref().map_or(0, |(_, keep)| *keep)
                                ),
                            ),
                            Some(Err(error)) => run.log(
                                LogLevel::Warning,
                                format!("Couldn't delete older export files: {error}"),
                            ),
                            _ => {}
                        }
                    })
                });
                drop(connections);
                return cx.update(|cx| {
                    Self::update_task_run(&state, task_id, Some(cancelled), cx, |_| {})
                });
            }
        })
        .detach();
    }

    /// One try at the transfer: a fresh hidden Transfer tab on the run's own connections, run
    /// through the Transfer tab's code until it reports how it ended. `regrant` is the
    /// connection a retry writes to: the run was confirmed once, so a retry grants itself the
    /// Production write the confirmation gave the first try.
    async fn transfer_attempt(
        cx: &mut AsyncApp,
        state: &Entity<AppState>,
        task_id: Uuid,
        mut tab: TransferTabState,
        confirmed_overwrite: Option<PathBuf>,
        connections: &RunConnections,
        regrant: Option<Uuid>,
    ) -> (TransferOutcome, Option<TransferTabState>) {
        tab.runtime.clients = connections.task_clients();
        let (sender, receiver) = futures::channel::oneshot::channel::<TransferOutcome>();
        let sender = std::rc::Rc::new(RefCell::new(Some(sender)));
        let transfer_id = cx.update(|cx| {
            let transfer_id = state.update(cx, |app, _| app.insert_task_transfer(tab));
            let notify = sender.clone();
            let events = cx.subscribe(state, move |_, event: &AppEvent, _| {
                let outcome = match event {
                    AppEvent::TransferCompleted { transfer_id: id, count }
                        if *id == transfer_id =>
                    {
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
                if let Some(sender) = notify.borrow_mut().take() {
                    let _ = sender.send(outcome);
                }
            });
            state.update(cx, |app, cx| {
                if let Some(active) = app.tasks.active.get_mut(&task_id) {
                    active.stop = RunStop::Transfer(transfer_id);
                    active.transfer_id = Some(transfer_id);
                    active._events = Some(events);
                }
                if let Some(connection) = regrant
                    && app.connection_requires_production_write_confirmation(connection)
                {
                    app.authorize_production_writes(connection, 1);
                }
                cx.notify();
            });
            match confirmed_overwrite {
                Some(path) => {
                    Self::execute_confirmed_transfer(state.clone(), transfer_id, Some(path), cx)
                }
                None => Self::execute_transfer(state.clone(), transfer_id, cx),
            }
            if let Some(connection) = regrant {
                state
                    .update(cx, |app, _| app.revoke_production_write_authorizations(connection, 1));
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
            if let Some(error) = refused
                && let Some(sender) = sender.borrow_mut().take()
            {
                let _ = sender.send(TransferOutcome::Failed(error));
            }
            transfer_id
        });
        let outcome = receiver.await.unwrap_or(TransferOutcome::Cancelled);
        let after =
            cx.update(|cx| state.update(cx, |app, _| app.remove_task_transfer(transfer_id)));
        (outcome, after)
    }

    fn start_compare_run(
        state: Entity<AppState>,
        task: SavedTask,
        config: CompareConfig,
        trigger: RunTrigger,
        cx: &mut App,
    ) {
        let Some(reconnect) = Self::reconnect_for(&state, &task, cx) else {
            let reason = "A connection this task uses no longer exists.";
            Self::fail_before_start(&state, &task, Launch::Run, reason, cx);
            return;
        };
        let timeout = Self::listing_timeout(&state, cx);
        let task_id = task.id;
        let (_, cancellation) = Self::begin_task_run(&state, &task, trigger, cx);
        let until = Self::retry_until(&state, task_id, cx);
        let filter = match config.scope {
            CompareScope::Databases => Document::new(),
            CompareScope::Collections if config.filter.trim().is_empty() => Document::new(),
            CompareScope::Collections => {
                match crate::bson::parse_document_from_json(&config.filter) {
                    Ok(filter) => filter,
                    Err(error) => {
                        let error = format!("The filter isn't valid JSON: {error}");
                        return Self::fail_run(&state, task_id, error, cx);
                    }
                }
            }
        };
        cx.spawn(async move |cx| {
            let mut watch = Watch::new(cancellation, until);
            let log = Self::run_log(&state, task_id);
            let mut connections = match open_connections(cx, &mut watch, &reconnect, log).await {
                Ok(connections) => connections,
                Err(failure) => return Self::end_run(cx, &state, task_id, &watch, Some(failure)),
            };
            let error = match config.scope {
                CompareScope::Collections => {
                    Self::compare_collection(
                        cx,
                        &state,
                        task_id,
                        &config,
                        filter,
                        &reconnect,
                        &mut connections,
                        &mut watch,
                    )
                    .await
                }
                CompareScope::Databases => {
                    Self::compare_database(
                        cx,
                        &state,
                        task_id,
                        &config,
                        timeout,
                        &reconnect,
                        &mut connections,
                        &mut watch,
                    )
                    .await
                }
            };
            Self::end_run(cx, &state, task_id, &watch, error);
        })
        .detach();
    }

    /// The saved connections the task uses, to open the run's own connections to.
    fn reconnect_for(state: &Entity<AppState>, task: &SavedTask, cx: &App) -> Option<Reconnect> {
        let app = state.read(cx);
        let saved = task
            .spec
            .connections()
            .iter()
            .map(|id| app.connection_by_id(*id).cloned())
            .collect::<Option<Vec<_>>>()?;
        Some(Reconnect { manager: app.connection_manager(), saved })
    }

    fn listing_timeout(state: &Entity<AppState>, cx: &App) -> Duration {
        Duration::from_millis(state.read(cx).settings.interactive_query_timeout_ms.max(100))
    }

    /// Writes a line into the run's log, from inside a running task.
    fn run_log(
        state: &Entity<AppState>,
        task_id: Uuid,
    ) -> impl FnMut(&mut AsyncApp, String) + use<> {
        let state = state.clone();
        move |cx, message| {
            cx.update(|cx| {
                Self::update_task_run(&state, task_id, None, cx, |run| {
                    run.log(LogLevel::Warning, message)
                })
            })
        }
    }

    /// Ends the run: cancelled, past its time limit, failed with `error`, or done.
    fn end_run(
        cx: &mut AsyncApp,
        state: &Entity<AppState>,
        task_id: Uuid,
        watch: &Watch,
        error: Option<Failure>,
    ) {
        let cancelled = watch.run.is_cancelled();
        let expired = watch.expired;
        cx.update(|cx| {
            Self::update_task_run(state, task_id, Some(cancelled), cx, |run| {
                if expired {
                    run.error = Some(format!(
                        "Stopped after {} hours, the longest a run may take.",
                        task_run::RUN_LIMIT.as_secs() / 3600
                    ));
                } else if let Some(error) = error {
                    run.log(LogLevel::Error, error.message.clone());
                    // ponytail: only a whole-run failure is sorted; a collection that ran out of
                    // retries counts as lasting, so it needs attention at once rather than never.
                    run.failure = if error.sign_in {
                        Some(FailureKind::SignIn)
                    } else if error.transient
                        && run.collections.iter().all(|entry| entry.error.is_none())
                    {
                        Some(FailureKind::Temporary)
                    } else {
                        None
                    };
                    run.error = Some(error.message);
                }
            })
        });
    }

    #[allow(clippy::too_many_arguments)]
    async fn compare_collection(
        cx: &mut AsyncApp,
        state: &Entity<AppState>,
        task_id: Uuid,
        config: &CompareConfig,
        filter: Document,
        reconnect: &Reconnect,
        connections: &mut RunConnections,
        watch: &mut Watch,
    ) -> Option<Failure> {
        let runtime = reconnect.manager.runtime_handle();
        let name = config.sides[0].collection.clone();
        let options = CompareOptions {
            fields: config.fields.clone(),
            filter,
            ignore: config.ignore_set(),
            row_limit: 0,
            row_kinds: None,
        };
        let mut tries = 1;
        loop {
            let clients = config
                .sides
                .each_ref()
                .map(|side| side.connection_id.and_then(|id| connections.client(id)));
            let [Some(left), Some(right)] = clients else {
                return Some(Failure::lasting("The run's connections closed."));
            };
            let sides = [(&left, 0), (&right, 1)].map(|(client, i)| {
                client.database(&config.sides[i].database).collection(&config.sides[i].collection)
            });
            let [left, right] = sides;
            let token = CancellationToken::new();
            let (sender, mut receiver) = futures::channel::mpsc::unbounded();
            let work = runtime.spawn(compare_collections_async(
                left,
                right,
                options.clone(),
                token.clone(),
                sender,
            ));
            watch
                .drain(cx, &token, &mut receiver, task_run::STALL, |cx, message| {
                    if let CompareMessage::Progress { counts, .. } = message {
                        cx.update(|cx| {
                            Self::update_task_run(state, task_id, None, cx, |run| {
                                run.collection_mut(&name).differences = Some(counts);
                            })
                        });
                    }
                })
                .await;
            let failure = match work.await {
                Ok(Ok(summary)) if !summary.cancelled => {
                    cx.update(|cx| {
                        Self::update_task_run(state, task_id, None, cx, |run| {
                            run.collection_mut(&name).differences = Some(summary.counts);
                        })
                    });
                    return None;
                }
                Ok(Ok(_)) if watch.over() => return None,
                Ok(Ok(_)) => Failure::temporary(format!(
                    "No progress for {} minutes",
                    task_run::STALL.as_secs() / 60
                )),
                Ok(Err(error)) => Failure::from(error),
                Err(error) => Failure::lasting(format!("The comparison stopped: {error}")),
            };
            if !failure.transient
                || tries == task_run::ATTEMPTS
                || watch.over()
                || !watch.may_retry()
            {
                return Some(failure);
            }
            let wait = task_run::backoff(tries);
            Self::run_log(state, task_id)(
                cx,
                format!(
                    "{failure}. Trying again (attempt {} of {}) after {} s.",
                    tries + 1,
                    task_run::ATTEMPTS,
                    wait.as_secs()
                ),
            );
            task_run::pause(cx, wait, &watch.run).await;
            if let Ok(fresh) = reconnect.open(cx).await {
                *connections = fresh;
            }
            tries += 1;
        }
    }

    /// Lists both databases, each side on its own connection.
    #[allow(clippy::too_many_arguments)]
    async fn list_pairs(
        cx: &mut AsyncApp,
        state: &Entity<AppState>,
        task_id: Uuid,
        config: &CompareConfig,
        timeout: Duration,
        reconnect: &Reconnect,
        connections: &mut RunConnections,
        watch: &mut Watch,
    ) -> Result<Vec<CollectionPair>, Failure> {
        let runtime = reconnect.manager.runtime_handle();
        let ids = config.sides.each_ref().map(|side| side.connection_id);
        let databases = config.sides.each_ref().map(|side| side.database.clone());
        retry_once(
            cx,
            watch,
            reconnect,
            connections,
            Self::run_log(state, task_id),
            |connections| {
                let clients = ids.map(|id| id.and_then(|id| connections.client(id)));
                let databases = databases.clone();
                let work = runtime.spawn(async move {
                    let [Some(left), Some(right)] = clients else {
                        return Err(Failure::lasting("The run's connections closed."));
                    };
                    let (l, r) = tokio::join!(
                        list_side(&left, &databases[0], timeout),
                        list_side(&right, &databases[1], timeout)
                    );
                    Ok(pair_collections(l.map_err(Failure::from)?, r.map_err(Failure::from)?))
                });
                async move {
                    work.await.unwrap_or_else(|error| {
                        Err(Failure::lasting(format!("Listing stopped: {error}")))
                    })
                }
            },
        )
        .await
    }

    /// Compares `names` pair by pair on the run's connections, retrying what fails for a reason
    /// that can pass. `on` sees every message.
    #[allow(clippy::too_many_arguments)]
    async fn compare_pairs(
        cx: &mut AsyncApp,
        state: &Entity<AppState>,
        task_id: Uuid,
        config: &CompareConfig,
        names: &[(usize, String)],
        reconnect: &Reconnect,
        connections: &mut RunConnections,
        watch: &mut Watch,
        on: impl FnMut(&mut AsyncApp, PairMessage),
    ) -> task_run::Finished<crate::connection::ops::compare::CompareSummary> {
        let runtime = reconnect.manager.runtime_handle();
        let ids = config.sides.each_ref().map(|side| side.connection_id);
        let databases = config.sides.each_ref().map(|side| side.database.clone());
        let ignore = config.ignore_set();
        let pending = names.iter().map(|(index, _)| *index).collect();
        run_steps(
            cx,
            watch,
            reconnect,
            connections,
            pending,
            Self::run_log(state, task_id),
            |connections, pending, token| {
                let clients = ids.map(|id| id.and_then(|id| connections.client(id)));
                let scans: Vec<PairScan> = names
                    .iter()
                    .filter(|(index, _)| pending.contains(index))
                    .map(|(index, name)| PairScan {
                        index: *index,
                        name: name.clone(),
                        cancellation: token.clone(),
                    })
                    .collect();
                let (sender, receiver) = futures::channel::mpsc::unbounded();
                let (databases, ignore) = (databases.clone(), ignore.clone());
                let work = runtime.spawn(async move {
                    let [Some(left), Some(right)] = clients else {
                        return Err(Failure::lasting("The run's connections closed."));
                    };
                    compare_pairs_async([left, right], databases, scans, ignore, sender).await;
                    Ok(())
                });
                (receiver, async move {
                    work.await.unwrap_or_else(|error| {
                        Err(Failure::lasting(format!("The comparison stopped: {error}")))
                    })
                })
            },
            on,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn compare_database(
        cx: &mut AsyncApp,
        state: &Entity<AppState>,
        task_id: Uuid,
        config: &CompareConfig,
        timeout: Duration,
        reconnect: &Reconnect,
        connections: &mut RunConnections,
        watch: &mut Watch,
    ) -> Option<Failure> {
        let pairs = match Self::list_pairs(
            cx,
            state,
            task_id,
            config,
            timeout,
            reconnect,
            connections,
            watch,
        )
        .await
        {
            Ok(pairs) => pairs,
            Err(failure) => return Some(failure),
        };
        let mut names = Vec::new();
        cx.update(|cx| {
            Self::update_task_run(state, task_id, None, cx, |run| {
                for pair in &pairs {
                    let note = if config.skip.contains(&pair.name) {
                        Some("Listed under Skip collections")
                    } else {
                        pair_note(pair)
                    };
                    let entry = run.collection_mut(&pair.name);
                    match note {
                        Some(note) => entry.note = Some(note.into()),
                        None => names.push((names.len(), pair.name.clone())),
                    }
                }
            })
        });
        let lookup = names.clone();
        let finished = Self::compare_pairs(
            cx,
            state,
            task_id,
            config,
            &names,
            reconnect,
            connections,
            watch,
            |cx, message| {
                if let PairMessage::Progress(index, counts) = message {
                    cx.update(|cx| {
                        Self::update_task_run(state, task_id, None, cx, |run| {
                            run.collection_mut(&lookup[index].1).differences = Some(counts)
                        })
                    });
                }
            },
        )
        .await;
        cx.update(|cx| {
            Self::update_task_run(state, task_id, None, cx, |run| {
                for (index, summary) in &finished.done {
                    run.collection_mut(&names[*index].1).differences = Some(summary.counts);
                }
                for (index, failure) in &finished.failed {
                    run.collection_mut(&names[*index].1).error = Some(failure.message.clone());
                }
            })
        });
        finished.error
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
        window: Option<AnyWindowHandle>,
        cx: &mut App,
    ) {
        let Some(reconnect) = Self::reconnect_for(&state, &task, cx) else {
            let reason = "A connection this task uses no longer exists.";
            Self::fail_before_start(&state, &task, launch, reason, cx);
            return;
        };
        let timeout = Self::listing_timeout(&state, cx);
        let task_id = task.id;
        let (_, cancellation) = Self::begin_task_run(&state, &task, launch.trigger(), cx);
        let until = Self::retry_until(&state, task_id, cx);
        let (target, source) = (side_index(sync.target), 1 - side_index(sync.target));

        cx.spawn(async move |cx| {
            let mut watch = Watch::new(cancellation.clone(), until);
            let log = Self::run_log(&state, task_id);
            let mut connections = match open_connections(cx, &mut watch, &reconnect, log).await {
                Ok(connections) => connections,
                Err(failure) => return Self::end_run(cx, &state, task_id, &watch, Some(failure)),
            };
            let pairs = match Self::list_pairs(
                cx,
                &state,
                task_id,
                &sync.config,
                timeout,
                &reconnect,
                &mut connections,
                &mut watch,
            )
            .await
            {
                Ok(pairs) => pairs,
                Err(failure) => return Self::end_run(cx, &state, task_id, &watch, Some(failure)),
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
            let compared: Vec<(usize, String)> = plan
                .iter()
                .filter(|pair| !pair.create)
                .map(|pair| (pair.index, pair.name.clone()))
                .collect();
            let finished = Self::compare_pairs(
                cx,
                &state,
                task_id,
                &sync.config,
                &compared,
                &reconnect,
                &mut connections,
                &mut watch,
                |_, _| {},
            )
            .await;
            drop(connections);
            if watch.over() || finished.error.is_some() || !finished.failed.is_empty() {
                let failed: Vec<(String, String)> = finished
                    .failed
                    .iter()
                    .map(|(index, failure)| (plan[*index].name.clone(), failure.message.clone()))
                    .collect();
                cx.update(|cx| {
                    Self::update_task_run(&state, task_id, None, cx, |run| {
                        for (name, error) in failed {
                            run.collection_mut(&name).error = Some(error);
                        }
                    })
                });
                return Self::end_run(cx, &state, task_id, &watch, finished.error);
            }
            cx.update(|cx| {
                let planned: Vec<Planned> = plan
                    .iter()
                    .map(|pair| {
                        let Some(summary) = finished.done.get(&pair.index) else {
                            let inserts = pairs
                                .iter()
                                .find(|p| p.name == pair.name)
                                .and_then(|p| p.sides[source].as_ref())
                                .and_then(|side| side.estimated)
                                .unwrap_or(0);
                            return Planned {
                                name: pair.name.clone(),
                                inserts,
                                ..Default::default()
                            };
                        };
                        let counts = &summary.counts;
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
        window: Option<AnyWindowHandle>,
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
        let Some(window) = window else {
            if anyway {
                return Self::fail_run(&state, task_id, SAFETY_STOPPED.into(), cx);
            }
            // The schedule's approval stands in for the question and its grants.
            state.update(cx, |app, _| {
                if app.connection_requires_production_write_confirmation(connection) {
                    app.authorize_production_writes(connection, written.len());
                }
            });
            let sync_writes = (connection, database);
            return Self::begin_sync_writes(
                state,
                task_id,
                sync,
                written,
                sync_writes,
                false,
                cancellation,
                cx,
            );
        };
        Self::ask(state.clone(), task_id, window, Question::Write(request), cx, move |cx| {
            let sync_writes = (connection, database);
            Self::begin_sync_writes(
                state,
                task_id,
                sync,
                written,
                sync_writes,
                anyway,
                cancellation,
                cx,
            )
        });
    }

    /// Spends one Production grant per collection before anything is written, then writes.
    #[allow(clippy::too_many_arguments)]
    fn begin_sync_writes(
        state: Entity<AppState>,
        task_id: Uuid,
        sync: SyncRequest,
        written: Vec<PairSync>,
        (connection, database): (Uuid, String),
        anyway: bool,
        cancellation: CancellationToken,
        cx: &mut App,
    ) {
        for (spent, pair) in written.iter().enumerate() {
            let key = SessionKey::new(connection, &database, &pair.name);
            if !Self::ensure_collection_writable(&state, &key, cx) {
                // Grants for the collections not reached aren't left for another write to use.
                state.update(cx, |app, _| {
                    app.revoke_production_write_authorizations(connection, written.len() - spent)
                });
                Self::update_task_run(&state, task_id, Some(true), cx, |run| {
                    run.log(LogLevel::Warning, format!("Writing to {} was refused.", pair.name));
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
    }

    /// Writes the sync on the run's own connections. A Mirror writes inserts and replacements
    /// first and deletes last, and skips the deletes when anything before them failed. A
    /// collection that fails for a reason that can pass is tried again: the sync compares it
    /// again and writes only what still differs.
    fn write_sync(
        state: Entity<AppState>,
        task_id: Uuid,
        sync: SyncRequest,
        plan: Vec<PairSync>,
        cancellation: CancellationToken,
        cx: &mut App,
    ) {
        let Some(task) = state.read(cx).task(task_id).cloned() else {
            return;
        };
        let Some(reconnect) = Self::reconnect_for(&state, &task, cx) else {
            return Self::fail_run(
                &state,
                task_id,
                "A connection this task uses no longer exists.".into(),
                cx,
            );
        };
        let app = state.read(cx);
        let restore_dir = app.compare_restore_dir();
        let target = side_index(sync.target);
        let ids = sync.config.sides.each_ref().map(|side| side.connection_id);
        let databases = sync.config.sides.each_ref().map(|side| side.database.clone());
        let (Some(connection_id), database) = (ids[target], databases[target].clone()) else {
            return;
        };
        let run_id = app.tasks.active.get(&task_id).map(|active| active.run_id);
        let until = Self::retry_until(&state, task_id, cx);
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
            let runtime = reconnect.manager.runtime_handle();
            let mut watch = Watch::new(cancellation, until);
            let log = Self::run_log(&state, task_id);
            let mut connections = match open_connections(cx, &mut watch, &reconnect, log).await {
                Ok(connections) => connections,
                Err(failure) => return Self::end_run(cx, &state, task_id, &watch, Some(failure)),
            };
            let count = plan.len();
            let name_of = |index: usize| plan[index % count].name.clone();
            let mut written: std::collections::HashMap<usize, SyncSummary> = Default::default();
            let mut logs = Vec::new();
            let mut error = None;
            for (pass, (mode, deletes_only)) in passes.into_iter().enumerate() {
                let pending: Vec<usize> = (0..count).map(|i| pass * count + i).collect();
                let finished = run_steps(
                    cx,
                    &mut watch,
                    &reconnect,
                    &mut connections,
                    pending,
                    Self::run_log(&state, task_id),
                    |connections, pending, token| {
                        let clients = ids.map(|id| id.and_then(|id| connections.client(id)));
                        let pairs = pending
                            .iter()
                            .map(|index| {
                                let pair = &plan[index % count];
                                PairSync {
                                    index: *index,
                                    name: pair.name.clone(),
                                    create: pair.create && pass == 0,
                                }
                            })
                            .collect();
                        let (sender, receiver) = futures::channel::mpsc::unbounded();
                        let request =
                            (databases.clone(), sync.config.ignore_set(), restore_dir.clone());
                        let work = runtime.spawn(async move {
                            let [Some(left), Some(right)] = clients else {
                                return Err(Failure::lasting("The run's connections closed."));
                            };
                            let (databases, ignore, restore_dir) = request;
                            sync_pairs_async(
                                DatabaseSync {
                                    clients: [left, right],
                                    databases,
                                    target: sync.target,
                                    mode,
                                    pairs,
                                    ignore,
                                    restore_dir,
                                    pass_rows: MAX_ROWS,
                                    deletes_only,
                                },
                                token,
                                sender,
                            )
                            .await
                            .map_err(Failure::from)
                        });
                        (receiver, async move {
                            work.await.unwrap_or_else(|error| {
                                Err(Failure::lasting(format!("The sync stopped: {error}")))
                            })
                        })
                    },
                    |cx, message| {
                        match message {
                            PairSyncMessage::Started(index, restore) => {
                                logs.push((index, name_of(index), restore))
                            }
                            PairSyncMessage::Progress(index, summary)
                            | PairSyncMessage::Done(index, summary) => {
                                written.insert(index, summary);
                            }
                            PairSyncMessage::Failed(..) => {}
                        }
                        let totals = totals_by_name(&written, &name_of);
                        cx.update(|cx| {
                            Self::update_task_run(&state, task_id, None, cx, |run| {
                                for (name, summary) in totals {
                                    run.collection_mut(&name).writes = Some(summary);
                                }
                            })
                        });
                    },
                )
                .await;
                let failed: Vec<(String, String)> = finished
                    .failed
                    .iter()
                    .map(|(index, failure)| (name_of(*index), failure.message.clone()))
                    .collect();
                let broken = !failed.is_empty() || finished.error.is_some() || watch.over();
                cx.update(|cx| {
                    Self::update_task_run(&state, task_id, None, cx, |run| {
                        for (name, message) in failed {
                            run.collection_mut(&name).error = Some(message);
                        }
                    })
                });
                error = finished.error;
                if broken {
                    if pass == 0 && deletes_only_follows(sync.mode) && !watch.over() {
                        Self::run_log(&state, task_id)(
                            cx,
                            "Deletes were skipped because something before them failed.".into(),
                        );
                    }
                    break;
                }
            }
            drop(connections);
            if let Some(run_id) = run_id
                && !logs.is_empty()
            {
                cx.update(|cx| {
                    state.update(cx, |app, _| {
                        app.tasks
                            .undo
                            .insert(task_id, UndoLog { run_id, connection_id, database, logs });
                    })
                });
            }
            Self::end_run(cx, &state, task_id, &watch, error);
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
        let Some(reconnect) = Self::reconnect_for(&state, &task, cx) else {
            let reason = "A connection this task uses no longer exists.";
            Self::fail_before_start(&state, &task, Launch::Run, reason, cx);
            return;
        };
        let task_id = task.id;
        state.update(cx, |app, _| {
            app.tasks.undo.remove(&task_id);
        });
        let (_, cancellation) = Self::begin_task_run(&state, &task, RunTrigger::Undo, cx);
        let names: std::collections::HashMap<usize, String> =
            undo.logs.iter().map(|(index, name, _)| (*index, name.clone())).collect();
        cx.spawn(async move |cx| {
            let runtime = reconnect.manager.runtime_handle();
            let mut watch = Watch::new(cancellation, None);
            let log = Self::run_log(&state, task_id);
            let mut connections = match open_connections(cx, &mut watch, &reconnect, log).await {
                Ok(connections) => connections,
                Err(failure) => return Self::end_run(cx, &state, task_id, &watch, Some(failure)),
            };
            let name_of = |index: usize| names.get(&index).cloned().unwrap_or_default();
            let mut restored: std::collections::HashMap<usize, SyncSummary> = Default::default();
            // The last pass first: a Mirror's deletes are undone before its inserts and replacements.
            let mut logs = undo.logs.clone();
            logs.reverse();
            let pending = logs.iter().map(|(index, ..)| *index).collect();
            let finished = run_steps(
                cx,
                &mut watch,
                &reconnect,
                &mut connections,
                pending,
                Self::run_log(&state, task_id),
                |connections, pending, token| {
                    let client = connections.client(undo.connection_id);
                    let logs: Vec<_> = logs
                        .iter()
                        .filter(|(index, ..)| pending.contains(index))
                        .cloned()
                        .collect();
                    let database = undo.database.clone();
                    let (sender, receiver) = futures::channel::mpsc::unbounded();
                    let work = runtime.spawn(async move {
                        let Some(client) = client else {
                            return Err(Failure::lasting("The run's connection closed."));
                        };
                        undo_pairs_async(client, database, logs, token, sender).await;
                        Ok(())
                    });
                    (receiver, async move {
                        work.await.unwrap_or_else(|error| {
                            Err(Failure::lasting(format!("The undo stopped: {error}")))
                        })
                    })
                },
                |cx, message| {
                    if let PairSyncMessage::Progress(index, summary)
                    | PairSyncMessage::Done(index, summary) = message
                    {
                        restored.insert(index, summary);
                    }
                    let totals = totals_by_name(&restored, &name_of);
                    cx.update(|cx| {
                        Self::update_task_run(&state, task_id, None, cx, |run| {
                            for (name, summary) in totals {
                                run.collection_mut(&name).writes = Some(summary);
                            }
                        })
                    });
                },
            )
            .await;
            let failed: Vec<(String, String)> = finished
                .failed
                .iter()
                .map(|(index, failure)| (name_of(*index), failure.message.clone()))
                .collect();
            cx.update(|cx| {
                Self::update_task_run(&state, task_id, None, cx, |run| {
                    for (name, message) in failed {
                        run.collection_mut(&name).error = Some(message);
                    }
                })
            });
            Self::end_run(cx, &state, task_id, &watch, finished.error);
        })
        .detach();
    }
}

#[cfg(test)]
pub(crate) struct Self_;

#[cfg(test)]
impl Self_ {
    /// Run now, for a task that asks nothing before it runs.
    pub(crate) fn start_task_for_test(
        state: &Entity<AppState>,
        task: &SavedTask,
        window: AnyWindowHandle,
        cx: &mut App,
    ) {
        AppCommands::start_task(state.clone(), task.clone(), Launch::Run, Some(window), cx);
    }

    /// Starts a transfer where confirming Run now leads, without the question.
    pub(crate) fn start_transfer_for_test(
        state: &Entity<AppState>,
        task: &SavedTask,
        cx: &mut App,
    ) {
        let TaskSpec::Transfer { config, options } = task.spec.clone() else {
            panic!("a transfer task");
        };
        let mut tab = TransferTabState::from_settings(&state.read(cx).settings);
        tab.config = config;
        tab.options = options;
        AppCommands::begin_task_run(state, task, RunTrigger::Manual, cx);
        AppCommands::start_transfer_run(state.clone(), task.id, tab, None, false, true, cx);
    }

    pub(crate) fn start_compare_run_for_test(
        state: &Entity<AppState>,
        task: &SavedTask,
        cx: &mut App,
    ) {
        let TaskSpec::Compare { config } = task.spec.clone() else {
            panic!("a compare task");
        };
        AppCommands::start_compare_run(state.clone(), task.clone(), config, RunTrigger::Manual, cx);
    }
}

/// Run now, Preview, or a run the schedule started for the time it was due.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Launch {
    Run,
    Preview,
    Schedule { due: chrono::DateTime<chrono::Utc>, catch_up: bool },
}

impl Launch {
    fn trigger(self) -> RunTrigger {
        match self {
            Self::Run => RunTrigger::Manual,
            Self::Preview => RunTrigger::Preview,
            Self::Schedule { catch_up: false, .. } => RunTrigger::Schedule,
            Self::Schedule { catch_up: true, .. } => RunTrigger::CatchUp,
        }
    }
}

/// Why a run nobody was there to ask ended before writing.
const SAFETY_STOPPED: &str = "The safety limit stopped this run before it wrote anything.";

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

/// Writes how a transfer ended into the run, per collection where it went collection by
/// collection.
fn record_transfer(
    run: &mut Run,
    scope: TransferScope,
    collection: &str,
    outcome: &TransferOutcome,
    tab: Option<&TransferTabState>,
) {
    let progress = tab.and_then(|tab| tab.runtime.database_progress.as_ref());
    match (scope, progress) {
        (TransferScope::Database, Some(progress)) => {
            for item in &progress.collections {
                let entry = run.collection_mut(&item.name);
                entry.documents = item.documents_processed;
                match &item.status {
                    CollectionTransferStatus::Failed(error) => entry.error = Some(error.clone()),
                    CollectionTransferStatus::Cancelled => entry.note = Some("Cancelled".into()),
                    _ => {}
                }
            }
            if let TransferOutcome::Failed(error) = outcome
                && run.collections.iter().all(|entry| entry.error.is_none())
            {
                run.error = Some(error.clone());
            }
        }
        (TransferScope::Database, None) => {
            if let TransferOutcome::Failed(error) = outcome {
                run.error = Some(error.clone());
            }
        }
        _ if collection.is_empty() => {
            if let TransferOutcome::Failed(error) = outcome {
                run.error = Some(error.clone());
            }
        }
        _ => {
            let processed = tab.map_or(0, |tab| tab.runtime.progress_count);
            let entry = run.collection_mut(collection);
            match outcome {
                TransferOutcome::Completed(count) => entry.documents = *count,
                TransferOutcome::Failed(error) => {
                    entry.documents = processed;
                    entry.error = Some(error.clone());
                }
                TransferOutcome::Cancelled => entry.documents = processed,
            }
        }
    }
    if let TransferOutcome::Failed(error) = outcome {
        run.log(LogLevel::Error, error.clone());
    }
}

/// Whether a sync in `mode` has a deletes pass after its first.
fn deletes_only_follows(mode: SyncMode) -> bool {
    mode == SyncMode::Mirror
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

/// The document counts a transfer's safety check needs, read without a tab, on connections of
/// its own.
struct TransferCounts {
    reconnect: Reconnect,
    mode: TransferMode,
    scope: TransferScope,
    source: Option<(Uuid, String, String)>,
    target: Option<(Uuid, String, String)>,
    file: PathBuf,
    whole_target: bool,
}

impl TransferCounts {
    fn new(app: &AppState, tab: &TransferTabState) -> Option<Self> {
        let config = &tab.config;
        let source_id = config.source_connection_id?;
        let (source, target) = match config.mode {
            TransferMode::Export => (
                Some((source_id, config.source_database.clone(), config.source_collection.clone())),
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
                (None, Some((source_id, database, collection)))
            }
            TransferMode::Copy => {
                let destination = config.destination_connection_id?;
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
                        source_id,
                        config.source_database.clone(),
                        config.source_collection.clone(),
                    )),
                    Some((destination, database, collection)),
                )
            }
        };
        let mut ids = vec![source_id];
        if let Some((destination, ..)) = &target
            && *destination != source_id
        {
            ids.push(*destination);
        }
        let saved =
            ids.iter().map(|id| app.connection_by_id(*id).cloned()).collect::<Option<Vec<_>>>()?;
        Some(Self {
            reconnect: Reconnect { manager: app.connection_manager(), saved },
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
        let manager = self.reconnect.manager.clone();
        let connections = RunConnections::open(manager.clone(), &self.reconnect.saved)
            .map_err(|failure| format!("Could not connect: {failure}"))?;
        let documents = |side: &Option<(Uuid, String, String)>| -> Result<Option<u64>, String> {
            let Some((id, database, collection)) = side else {
                return Ok(None);
            };
            let client = connections.client(*id).ok_or("The connection closed.")?;
            let count = if self.scope == TransferScope::Database || collection.is_empty() {
                let stats = manager.database_stats(&client, database).map_err(|e| e.to_string())?;
                stats.get("objects").and_then(mongodb::bson::Bson::as_i64).unwrap_or(0) as u64
            } else {
                manager
                    .estimated_document_count(&client, database, collection)
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

        // Scheduled runs: nobody is asked. The connection is Production from here on.
        use chrono::TimeZone as _;
        let at = |day: u32| {
            chrono::Local
                .with_ymd_and_hms(2027, 3, day, 2, 0, 0)
                .earliest()
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let daily = crate::tasks::schedule::Schedule::Daily {
            at: chrono::NaiveTime::from_hms_opt(2, 0, 0).unwrap(),
            weekdays_only: false,
        };
        let schedule = |cx: &mut VisualTestContext, task: &SavedTask, safety, keep, from| {
            state.update(cx, |app, _| {
                app.connections[0].environment =
                    Some(crate::models::ConnectionEnvironment::Production);
                app.connections[0].confirm_production_writes = true;
                let settings = crate::state::app_state::ScheduleSettings {
                    schedule: daily.clone(),
                    safety,
                    keep_files: keep,
                    protected_writes: true,
                    ..Default::default()
                };
                app.set_task_schedule(task.id, settings).unwrap();
                app.mark_task_due(task.id, from);
            });
        };
        let due = |cx: &mut VisualTestContext, now| {
            cx.update(|_, cx| AppCommands::check_schedules(&state, now, cx));
        };

        // The limit stops a scheduled Mirror before it writes, instead of asking.
        schedule(cx, &mirror, Default::default(), None, at(1));
        due(cx, at(2));
        let stopped = finished(cx, &mirror);
        assert_eq!(stopped.trigger, RunTrigger::Schedule);
        assert_eq!(stopped.status, RunStatus::Failed, "{stopped:?}");
        assert_eq!(stopped.error.as_deref(), Some(SAFETY_STOPPED));
        assert_eq!(stopped.stops.len(), 1);
        assert_eq!(read(doc! {}, "shop_copy", "orders"), 152);

        // With the limit raised, it writes into Production without a question, spending exactly
        // the grants its approval gave.
        let loose = crate::tasks::safety::SafetyLimit { percent: 100, jump: 1_000, floor: 100 };
        schedule(cx, &mirror, loose, None, at(2));
        due(cx, at(3));
        let done = finished(cx, &mirror);
        assert_eq!(done.status, RunStatus::Succeeded, "{done:?}");
        assert_eq!(read(doc! {}, "shop_copy", "orders"), 3);
        assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
        assert!(
            state.read_with(cx, |app, _| !app.has_production_write_authorizations(connection, 1)),
            "no grant is left over"
        );

        // Scheduled exports name each file by its run and keep only the newest two. The Mirror is
        // paused, or it would take its turn in the queue each day too.
        state.update(cx, |app, _| app.set_task_paused(mirror.id, true).unwrap());
        schedule(cx, &export, Default::default(), Some(2), at(3));
        for day in 4..7 {
            due(cx, at(day));
            let done = finished(cx, &export);
            assert_eq!(done.status, RunStatus::Succeeded, "{done:?}");
        }
        let mut files: Vec<String> = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok()?.file_name().to_str().map(str::to_string))
            .filter(|name| name.starts_with("orders"))
            .collect();
        files.sort();
        assert_eq!(
            files,
            ["orders-2027-03-05T0200.jsonl", "orders-2027-03-06T0200.jsonl", "orders.jsonl"]
        );
        assert!(state.read_with(cx, |app, _| app.tasks.active.is_empty()));

        // A wrong password: the server refuses the sign-in, and the schedule pauses instead of
        // trying it again each day.
        state.update(cx, |app, _| {
            app.connections[0].uri = uri.replacen("mongodb://", "mongodb://nobody:wrong@", 1);
        });
        due(cx, at(7));
        let refused = finished(cx, &export);
        assert_eq!(refused.failure, Some(crate::tasks::model::FailureKind::SignIn), "{refused:?}");
        state.read_with(cx, |app, _| {
            let task = app.task(export.id).unwrap();
            assert!(task.paused && task.paused_by_sign_in);
            let fix = app.task_attention(task).map(|attention| attention.fix);
            assert_eq!(fix, Some(crate::state::app_state::Fix::Resume));
        });
        due(cx, at(8));
        assert_eq!(finished(cx, &export).id, refused.id, "paused, so nothing else ran");
        // The container stops through Tokio, so it is dropped inside a runtime.
        docker.block_on(async move { drop(container) });
    }

    /// A server error that can pass is retried and the run succeeds; one that can't fails at
    /// once. The server's own `failCommand` test hook makes the errors.
    #[gpui_kit::test]
    #[ignore = "starts a MongoDB 8 container: cargo test --lib task_runs_retry -- --ignored"]
    fn task_runs_retry_transient_failures_only(cx: &mut gpui_kit::TestAppContext) {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        use mongodb::bson::doc;
        use testcontainers::{ImageExt as _, runners::AsyncRunner as _};

        use crate::connection::ConnectionManager;
        use crate::models::SavedConnection;
        use crate::state::ConfigManager;
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
            .block_on(
                testcontainers_modules::mongo::Mongo::default()
                    .with_tag("8.0")
                    .with_cmd(["--setParameter", "enableTestCommands=1"])
                    .start(),
            )
            .unwrap();
        let uri = docker.block_on(async {
            format!(
                "mongodb://{}:{}",
                container.get_host().await.unwrap(),
                container.get_host_port_ipv4(27017).await.unwrap()
            )
        });
        let manager = Arc::new(ConnectionManager::new());
        let admin = manager.runtime_handle().block_on(async {
            let client = mongodb::Client::with_uri_str(&uri).await.unwrap();
            client
                .database("shop")
                .collection("orders")
                .insert_many([doc! {"_id": 1}, doc! {"_id": 2}])
                .await
                .unwrap();
            client
                .database("shop_copy")
                .collection("orders")
                .insert_one(doc! {"_id": 1})
                .await
                .unwrap();
            client
        });
        let fail = |code: i32, times: i32| {
            manager.runtime_handle().block_on(async {
                admin
                    .database("admin")
                    .run_command(doc! {
                        "configureFailPoint": "failCommand",
                        "mode": {"times": times},
                        "data": {"failCommands": ["find"], "errorCode": code},
                    })
                    .await
                    .unwrap();
            })
        };

        let directory = tempfile::tempdir().unwrap();
        let saved = SavedConnection::new("Test".into(), uri.clone());
        let state = cx.new(|_| {
            let mut state = AppState::with_config(
                manager.clone(),
                ConfigManager::with_config_dir(directory.path().into()),
            );
            state.connections = vec![saved.clone()];
            state.attach_task_runs(RunStore::in_memory().unwrap(), None);
            state
        });
        let mut config = CompareConfig { scope: CompareScope::Databases, ..Default::default() };
        config.sides = ["shop", "shop_copy"].map(|database| CompareEndpoint {
            connection_id: Some(saved.id),
            database: database.into(),
            collection: String::new(),
        });
        let task = SavedTask::new("Compare shop".into(), TaskSpec::Compare { config });
        state.update(cx, |app, _| app.upsert_task(task.clone()).unwrap());
        let run = |cx: &mut gpui_kit::TestAppContext| -> Run {
            cx.update(|cx| Self_::start_compare_run_for_test(&state, &task, cx));
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                cx.run_until_parked();
                let run = state.read_with(cx, |app, _| app.task_runs(task.id).first().cloned());
                if let Some(run) = run.clone().filter(|run| run.status != RunStatus::Running) {
                    return run;
                }
                assert!(Instant::now() < deadline, "the run didn't finish: {run:?}");
                // Waits between retries run on the executor's clock.
                cx.executor().advance_clock(Duration::from_secs(1));
                std::thread::sleep(Duration::from_millis(50));
            }
        };

        // "Network timeout" three times on the scan: the driver retries once, then the run
        // tries again.
        fail(89, 3);
        let done = run(cx);
        assert_eq!(done.status, RunStatus::Succeeded, "{done:?}");
        let orders = done.collections.iter().find(|c| c.name == "orders").unwrap();
        assert_eq!(orders.differences.unwrap().only_left, 1);
        assert!(
            done.log.iter().any(|line| line.message.contains("Trying")),
            "the retry is logged: {:?}",
            done.log
        );

        // "Not authorized" won't pass, so the run fails at once without retrying.
        fail(13, 1);
        let failed = run(cx);
        assert_eq!(failed.status, RunStatus::Failed, "{failed:?}");
        assert!(!failed.log.iter().any(|line| line.message.contains("Trying")));

        let finished = |cx: &mut gpui_kit::TestAppContext, task: &SavedTask| -> Run {
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                cx.run_until_parked();
                let run = state.read_with(cx, |app, _| app.task_runs(task.id).first().cloned());
                if let Some(run) = run.clone().filter(|run| run.status != RunStatus::Running) {
                    return run;
                }
                assert!(Instant::now() < deadline, "the run didn't finish: {run:?}");
                cx.executor().advance_clock(Duration::from_secs(1));
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
        let window: AnyWindowHandle = cx.add_window(|_, _| Blank).into();
        use crate::state::app_state::{TransferConfig, TransferOptions};
        let transfer = |name: &str, config: TransferConfig| {
            let task = SavedTask::new(
                name.into(),
                TaskSpec::Transfer { config, options: TransferOptions::default() },
            );
            (task.clone(), task)
        };

        // An export starts over after a failure that can pass, on its own connections.
        let file = directory.path().join("orders.jsonl");
        let (export, saved_export) = transfer(
            "Export orders",
            TransferConfig {
                mode: TransferMode::Export,
                scope: TransferScope::Collection,
                source_connection_id: Some(saved.id),
                source_database: "shop".into(),
                source_collection: "orders".into(),
                file_path: file.display().to_string(),
                ..Default::default()
            },
        );
        state.update(cx, |app, _| app.upsert_task(saved_export).unwrap());
        fail(89, 3);
        cx.update(|cx| Self_::start_task_for_test(&state, &export, window, cx));
        let done = finished(cx, &export);
        assert_eq!(done.status, RunStatus::Succeeded, "{done:?}");
        assert!(
            done.log.iter().any(|line| line.message.contains("Starting over")),
            "{:?}",
            done.log
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 2);
        assert!(
            state.read_with(cx, |app, _| !app.is_connected(saved.id)),
            "a run never opens the sidebar's connection"
        );

        // A copy that appends isn't run again: repeating it could write documents twice.
        let (copy, saved_copy) = transfer(
            "Copy orders",
            TransferConfig {
                mode: TransferMode::Copy,
                scope: TransferScope::Collection,
                source_connection_id: Some(saved.id),
                source_database: "shop".into(),
                source_collection: "orders".into(),
                destination_connection_id: Some(saved.id),
                destination_database: "shop_backup".into(),
                destination_collection: "orders".into(),
                ..Default::default()
            },
        );
        state.update(cx, |app, _| app.upsert_task(saved_copy).unwrap());
        fail(89, 3);
        cx.update(|cx| Self_::start_transfer_for_test(&state, &copy, cx));
        let done = finished(cx, &copy);
        assert_ne!(done.status, RunStatus::Succeeded, "{done:?}");
        assert!(
            done.log.iter().any(|line| line.message.contains("Not run again")),
            "{:?}",
            done.log
        );
        docker.block_on(async move { drop(container) });
    }
}
