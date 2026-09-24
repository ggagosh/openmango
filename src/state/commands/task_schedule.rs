//! Starting tasks on their schedules while OpenMango is open: a timer for the earliest next run,
//! a look at the wall clock that also catches runs missed while the computer slept, and a queue
//! that runs one task at a time.

use std::time::Duration;

use chrono::{DateTime, Utc};
use gpui_kit::{App, AppContext as _, Entity, WeakEntity};

use super::AppCommands;
use super::tasks::Launch;
use crate::state::AppState;
use crate::tasks::model::Run;
use crate::tasks::schedule::{self, Due};

/// The longest the scheduler waits before looking at the clock again. Timers don't count time the
/// computer spends asleep, so a run that came due during sleep starts within this long of waking.
const LOOK_AT_LEAST_EVERY: Duration = Duration::from_secs(60);

impl AppCommands {
    /// Runs the scheduler for as long as the app state exists.
    pub fn start_scheduler(state: WeakEntity<AppState>, cx: &mut App) {
        cx.spawn(async move |cx| {
            loop {
                let next = cx.update(|cx| {
                    let state = state.upgrade()?;
                    Some(Self::check_schedules(&state, Utc::now(), cx))
                });
                let Some(next) = next else {
                    return;
                };
                let wait = next
                    .and_then(|at| (at - Utc::now()).to_std().ok())
                    .unwrap_or(LOOK_AT_LEAST_EVERY)
                    .clamp(Duration::from_millis(250), LOOK_AT_LEAST_EVERY);
                cx.background_executor().timer(wait).await;
            }
        })
        .detach();
    }

    /// Queues every task whose run is due at `now`, starts the queue if nothing is running, and
    /// says when the next run is due.
    pub(crate) fn check_schedules(
        state: &Entity<AppState>,
        now: DateTime<Utc>,
        cx: &mut App,
    ) -> Option<DateTime<Utc>> {
        // Until the lock is free, the background runner is starting runs; look again later.
        if !state.update(cx, |app, cx| {
            cx.notify();
            app.take_scheduler_lock()
        }) {
            return None;
        }
        let (tasks, background) = {
            let app = state.read(cx);
            (app.tasks.tasks.clone(), app.tasks.background)
        };
        let mut next: Option<DateTime<Utc>> = None;
        for task in tasks {
            let queued = state.read(cx).tasks.queue.iter().any(|(id, ..)| *id == task.id);
            let skipped = background && !task.run_when_closed;
            if task.paused || task.schedule.is_manual() || queued || skipped {
                continue;
            }
            let from = task.schedule_from.unwrap_or(task.updated_at);
            let (due, reason) = match schedule::due(&task.schedule, from, now, &chrono::Local) {
                Due::Wait(at) => {
                    next = earliest(next, at);
                    continue;
                }
                Due::Skip { due, next: at } => {
                    next = earliest(next, Some(at));
                    let reason = format!(
                        "The run due {} was missed while OpenMango was closed or the computer \
                         slept. The next one, {}, is soon, so this one didn't run.",
                        local(due),
                        local(at)
                    );
                    (due, reason)
                }
                Due::Run { due, .. } if state.read(cx).task_is_running(task.id) => (
                    due,
                    format!(
                        "The run due {} didn't start: the one before it was still going.",
                        local(due)
                    ),
                ),
                Due::Run { due, catch_up } => {
                    state.update(cx, |app, _| app.tasks.queue.push((task.id, due, catch_up)));
                    continue;
                }
            };
            state.update(cx, |app, cx| {
                app.mark_task_due(task.id, due);
                app.record_task_run(Run::skipped(task.id, reason), true);
                cx.notify();
            });
        }
        Self::start_queued(state, cx);
        next
    }

    /// Starts queued runs, the earliest due first, until one is running.
    pub(super) fn start_queued(state: &Entity<AppState>, cx: &mut App) {
        while state.read(cx).tasks.active.is_empty() {
            let queued = state.update(cx, |app, _| {
                let queue = &mut app.tasks.queue;
                let first = (0..queue.len()).min_by_key(|index| queue[*index].1)?;
                Some(queue.remove(first))
            });
            let Some((task_id, due, catch_up)) = queued else {
                return;
            };
            let app = state.read(cx);
            let Some(task) = app.task(task_id).cloned() else {
                continue;
            };
            if task.paused || task.schedule.is_manual() {
                continue;
            }
            let missing =
                task.spec.connections().iter().any(|id| app.connection_by_id(*id).is_none());
            let problem = if missing {
                Some("A connection this task uses no longer exists.".to_string())
            } else {
                app.task_approval_problem(&task)
            };
            state.update(cx, |app, cx| {
                app.mark_task_due(task_id, due);
                cx.notify();
            });
            let launch = Launch::Schedule { due, catch_up };
            match problem {
                Some(problem) => Self::fail_before_start(state, &task, launch, &problem, cx),
                None => Self::start_task(state.clone(), task, launch, None, cx),
            }
        }
    }
}

fn earliest(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// "Sep 24, 02:00", in local time.
fn local(at: DateTime<Utc>) -> String {
    at.with_timezone(&chrono::Local).format("%b %-d, %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{Local, NaiveTime, TimeZone as _};
    use gpui_kit::{AppContext as _, TestAppContext};

    use super::*;
    use crate::connection::CancellationToken;
    use crate::connection::ops::compare::Side;
    use crate::connection::ops::compare_database::SyncMode;
    use crate::models::{ConnectionEnvironment, SavedConnection};
    use crate::state::ConfigManager;
    use crate::state::app_state::{ActiveRun, RunStop};
    use crate::state::compare::CompareConfig;
    use crate::tasks::model::{RunStatus, RunTrigger, Task, TaskSpec};
    use crate::tasks::schedule::Schedule;
    use crate::tasks::store::RunStore;

    fn local(day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Local
            .with_ymd_and_hms(2026, 9, day, hour, minute, 0)
            .earliest()
            .unwrap()
            .with_timezone(&Utc)
    }

    fn daily(name: &str, spec: TaskSpec) -> Task {
        let mut task = Task::new(name.into(), spec);
        task.schedule =
            Schedule::Daily { at: NaiveTime::from_hms_opt(2, 0, 0).unwrap(), weekdays_only: false };
        task.schedule_from = Some(local(20, 12, 0));
        task
    }

    fn triggers(
        state: &Entity<AppState>,
        task: &Task,
        cx: &mut TestAppContext,
    ) -> Vec<(RunTrigger, RunStatus)> {
        state.read_with(cx, |app, _| {
            app.task_runs(task.id).iter().map(|run| (run.trigger, run.status)).collect()
        })
    }

    #[gpui_kit::test]
    fn due_tasks_run_one_at_a_time_and_a_missed_run_catches_up_once(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let state = cx.new(|_| {
            let mut app = AppState::with_config(
                Arc::new(crate::connection::ConnectionManager::new()),
                ConfigManager::with_config_dir(directory.path().into()),
            );
            app.attach_task_runs(RunStore::in_memory().unwrap(), None);
            app
        });
        // Comparisons without connections: each run fails at once, which is all this needs.
        let compare = || TaskSpec::Compare { config: CompareConfig::default() };
        let (first, second) = (daily("First", compare()), daily("Second", compare()));
        state.update(cx, |app, _| {
            app.upsert_task(first.clone()).unwrap();
            app.upsert_task(second.clone()).unwrap();
        });

        // 02:00 on the 21st: both are due; one starts and the other waits for it.
        let next = cx.update(|cx| AppCommands::check_schedules(&state, local(21, 2, 0), cx));
        state.read_with(cx, |app, _| {
            assert_eq!(app.tasks.active.len(), 1);
            assert_eq!(app.tasks.queue.len(), 1);
        });
        assert_eq!(next, None, "nothing else is waiting");
        cx.run_until_parked();
        for task in [&first, &second] {
            assert_eq!(triggers(&state, task, cx), [(RunTrigger::Schedule, RunStatus::Failed)]);
        }
        // Handled, so the same time doesn't run again.
        let next = cx.update(|cx| AppCommands::check_schedules(&state, local(21, 2, 1), cx));
        assert_eq!(next, Some(local(22, 2, 0)));
        assert_eq!(triggers(&state, &first, cx).len(), 1);

        // Closed until the 24th at 09:00: one catch-up each, not one per missed night.
        cx.update(|cx| AppCommands::check_schedules(&state, local(24, 9, 0), cx));
        cx.run_until_parked();
        assert_eq!(triggers(&state, &first, cx)[0], (RunTrigger::CatchUp, RunStatus::Failed));
        assert_eq!(triggers(&state, &first, cx).len(), 2);

        // A task still running when it comes due again is skipped, not queued.
        state.update(cx, |app, _| {
            app.tasks.active.insert(
                first.id,
                ActiveRun {
                    run_id: uuid::Uuid::new_v4(),
                    stop: RunStop::Token(CancellationToken::new()),
                    transfer_id: None,
                    retry_until: None,
                    _events: None,
                },
            );
        });
        cx.update(|cx| AppCommands::check_schedules(&state, local(25, 2, 0), cx));
        assert_eq!(triggers(&state, &first, cx)[0], (RunTrigger::Schedule, RunStatus::Skipped));
        state.read_with(cx, |app, _| assert_eq!(app.tasks.queue.len(), 1, "the second waits"));

        // Paused: nothing is due.
        state.update(cx, |app, _| {
            app.tasks.active.clear();
            app.tasks.queue.clear();
            app.set_task_paused(second.id, true).unwrap();
        });
        let runs = triggers(&state, &second, cx).len();
        cx.update(|cx| AppCommands::check_schedules(&state, local(26, 2, 0), cx));
        cx.run_until_parked();
        assert_eq!(triggers(&state, &second, cx).len(), runs);
    }

    /// While another OpenMango process holds the lock, this one starts nothing. Once the lock is
    /// free it takes over with what the other process recorded, so no run happens twice.
    #[gpui_kit::test]
    fn the_app_waits_for_the_background_runner_and_takes_over_its_record(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let config = ConfigManager::with_config_dir(directory.path().into());
        let state = cx.new(|_| {
            let mut app = AppState::with_config(
                Arc::new(crate::connection::ConnectionManager::new()),
                config.clone(),
            );
            app.attach_task_runs(RunStore::in_memory().unwrap(), None);
            app
        });
        let task = daily("Nightly", TaskSpec::Compare { config: CompareConfig::default() });
        state.update(cx, |app, _| app.upsert_task(task.clone()).unwrap());

        // The runner holds the lock and handles the run due on the 21st.
        let runner = config.take_scheduler_lock().unwrap().expect("nobody holds it yet");
        let mut handled = task.clone();
        handled.schedule_from = Some(local(21, 2, 0));
        config.save_tasks(&[handled]).unwrap();
        let next = cx.update(|cx| AppCommands::check_schedules(&state, local(21, 2, 5), cx));
        assert_eq!(next, None, "it looks again later");
        assert!(triggers(&state, &task, cx).is_empty(), "the app starts nothing");

        // An edit in the app meanwhile keeps the runner's record of what it handled.
        state.update(cx, |app, _| {
            let mut renamed = app.task(task.id).unwrap().clone();
            renamed.name = "Nightly sync".into();
            app.upsert_task(renamed).unwrap();
        });
        assert_eq!(config.load_tasks().unwrap()[0].schedule_from, Some(local(21, 2, 0)));

        // The runner ends. The app takes over, and the 21st doesn't run again.
        drop(runner);
        let next = cx.update(|cx| AppCommands::check_schedules(&state, local(21, 2, 10), cx));
        assert_eq!(next, Some(local(22, 2, 0)));
        assert!(triggers(&state, &task, cx).is_empty());
    }

    /// The background runner starts only the tasks that may run while OpenMango is closed, and
    /// what it saves never drops what the app saved meanwhile.
    #[gpui_kit::test]
    fn the_background_runner_runs_only_its_tasks_and_keeps_the_apps_edits(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let config = ConfigManager::with_config_dir(directory.path().into());
        let compare = || TaskSpec::Compare { config: CompareConfig::default() };
        let mut closed = daily("Closed", compare());
        closed.run_when_closed = true;
        let open_only = daily("Open only", compare());
        config.save_tasks(&[closed.clone(), open_only.clone()]).unwrap();
        assert!(crate::app::background::due_while_closed(&closed, local(21, 2, 0)));
        assert!(!crate::app::background::due_while_closed(&open_only, local(21, 2, 0)));

        let state = cx.new(|_| {
            let mut app = AppState::with_config(
                Arc::new(crate::connection::ConnectionManager::new()),
                config.clone(),
            );
            app.tasks.background = true;
            app.attach_task_runs(RunStore::in_memory().unwrap(), None);
            app
        });
        cx.update(|cx| AppCommands::check_schedules(&state, local(21, 2, 0), cx));
        cx.run_until_parked();
        assert_eq!(triggers(&state, &closed, cx), [(RunTrigger::Schedule, RunStatus::Failed)]);
        assert!(triggers(&state, &open_only, cx).is_empty());
        let log = state.read_with(cx, |app, _| app.task_runs(closed.id)[0].log.clone());
        assert!(log.iter().any(|line| line.message == "Started while OpenMango was closed."));

        // The app adds a task while the runner works; the runner's next save keeps it.
        let added = daily("Added in the app", compare());
        let mut saved = config.load_tasks().unwrap();
        saved.push(added.clone());
        config.save_tasks(&saved).unwrap();
        state.update(cx, |app, _| app.mark_task_due(closed.id, local(22, 2, 0)));
        let saved = config.load_tasks().unwrap();
        assert!(saved.iter().any(|task| task.id == added.id), "the app's task stays");
        let record = saved.iter().find(|task| task.id == closed.id).unwrap();
        assert_eq!(record.schedule_from, Some(local(22, 2, 0)));
    }

    #[gpui_kit::test]
    fn an_outage_notifies_once_and_needs_attention_after_three_failed_runs(
        cx: &mut TestAppContext,
    ) {
        use crate::state::ErrorAction;
        use crate::state::app_state::Fix;
        use crate::tasks::model::FailureKind;

        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let source = SavedConnection::new("Source".into(), "mongodb://localhost:1".into());
        let state = cx.new(|_| {
            let mut app = AppState::with_config(
                Arc::new(crate::connection::ConnectionManager::new()),
                ConfigManager::with_config_dir(directory.path().into()),
            );
            app.connections = vec![source.clone()];
            app.attach_task_runs(RunStore::in_memory().unwrap(), None);
            app
        });
        let mut config = CompareConfig::default();
        config.sides[0].connection_id = Some(source.id);
        config.sides[1].connection_id = Some(source.id);
        let task = daily("Nightly", TaskSpec::Compare { config });
        let run = |status: RunStatus, failure: Option<FailureKind>| {
            let mut run = Run::start(task.id, RunTrigger::Schedule);
            run.status = status;
            run.failure = failure;
            run.error = (status == RunStatus::Failed).then(|| "Server unreachable".to_string());
            run.finished_at = Some(Utc::now());
            run
        };
        let notified = |app: &AppState| {
            app.error_entries()
                .filter(|entry| matches!(entry.action, Some(ErrorAction::OpenTask(id)) if id == task.id))
                .count()
        };
        state.update(cx, |app, _| {
            app.upsert_task(task.clone()).unwrap();
            let attention = |app: &AppState| app.task_attention(app.task(task.id).unwrap());

            // A server that can't be reached: one notification for the outage, and the task
            // needs attention only at the third failure in a row.
            let temporary = Some(FailureKind::Temporary);
            app.record_task_run(run(RunStatus::Failed, temporary), true);
            assert_eq!(notified(app), 1);
            assert_eq!(attention(app), None);
            app.record_task_run(run(RunStatus::Failed, temporary), true);
            assert_eq!(notified(app), 1, "the same outage");
            app.record_task_run(run(RunStatus::Failed, temporary), true);
            let reason = attention(app).unwrap().reason;
            assert_eq!(reason, "The last 3 runs failed.");
            assert_eq!(app.tasks_needing_attention(), 1);

            // It works again, and says so.
            app.record_task_run(run(RunStatus::Succeeded, None), true);
            assert_eq!(attention(app), None);
            let message = app.status_message().unwrap().text;
            assert_eq!(message, "“Nightly” works again.");

            // A failure that won't pass needs attention at once, with the run to look at.
            let lasting = run(RunStatus::Failed, None);
            app.record_task_run(lasting.clone(), true);
            assert_eq!(notified(app), 2);
            assert_eq!(attention(app).unwrap().fix, Fix::ShowRun(lasting.id));

            // Signing in failed: the schedule pauses, which needs attention and notifies even
            // though the run before failed too.
            app.record_task_run(run(RunStatus::Failed, Some(FailureKind::SignIn)), true);
            let paused = app.task(task.id).unwrap();
            assert!(paused.paused && paused.paused_by_sign_in);
            assert_eq!(attention(app).unwrap().fix, Fix::Resume);
            assert_eq!(notified(app), 3);

            // Editing the connection resumes it.
            app.resume_tasks_after_sign_in_fix(source.id);
            let resumed = app.task(task.id).unwrap();
            assert!(!resumed.paused && !resumed.paused_by_sign_in);

            // A task without a schedule never needs attention: whoever ran it saw the result.
            let mut manual = app.task(task.id).unwrap().clone();
            manual.schedule = Schedule::Manual;
            app.upsert_task(manual).unwrap();
            assert_eq!(attention(app), None);
        });
    }

    #[gpui_kit::test]
    fn a_scheduled_write_stops_when_its_connection_changed_since_approval(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let (source, target) = (
            SavedConnection::new("Source".into(), "mongodb://localhost:1".into()),
            SavedConnection::new("Target".into(), "mongodb://localhost:2".into()),
        );
        let state = cx.new(|_| {
            let mut app = AppState::with_config(
                Arc::new(crate::connection::ConnectionManager::new()),
                ConfigManager::with_config_dir(directory.path().into()),
            );
            app.connections = vec![source.clone(), target.clone()];
            app.attach_task_runs(RunStore::in_memory().unwrap(), None);
            // The app's own schedule, as at startup: it may move a task's record back in time.
            assert!(app.take_scheduler_lock());
            app
        });
        let mut config = CompareConfig::default();
        config.sides[0].connection_id = Some(source.id);
        config.sides[1].connection_id = Some(target.id);
        let sync = daily(
            "Mirror",
            TaskSpec::Sync {
                config,
                target: Side::Right,
                mode: SyncMode::Mirror,
                excluded: vec![],
            },
        );
        state.update(cx, |app, _| {
            app.upsert_task(sync.clone()).unwrap();
            let schedule = sync.schedule.clone();
            let settings =
                crate::state::app_state::ScheduleSettings { schedule, ..Default::default() };
            app.set_task_schedule(sync.id, settings).unwrap();
            assert_eq!(app.task_approval_problem(app.task(sync.id).unwrap()), None);

            // The target becomes Production after approval.
            app.connections[1].environment = Some(ConnectionEnvironment::Production);
            let task = app.task(sync.id).unwrap();
            assert_eq!(
                app.task_approval_problem(task).as_deref(),
                Some("Connection settings changed since this task was approved.")
            );
            assert_eq!(app.task_protected_target(task).as_deref(), Some("Target"));

            // Approving again isn't enough without allowing Production writes.
            app.approve_task(sync.id, false).unwrap();
            let task = app.task(sync.id).unwrap();
            assert!(app.task_approval_problem(task).unwrap().contains("may not write to Target"));
            app.approve_task(sync.id, true).unwrap();
            assert_eq!(app.task_approval_problem(app.task(sync.id).unwrap()), None);
            app.approve_task(sync.id, false).unwrap();
            app.mark_task_due(sync.id, local(20, 12, 0));
        });

        // The schedule records the problem as a failed run instead of writing.
        cx.update(|cx| AppCommands::check_schedules(&state, local(21, 2, 0), cx));
        let run = state.read_with(cx, |app, _| app.task_runs(sync.id)[0].clone());
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.error.unwrap().contains("may not write to Target"));
    }
}
