use std::collections::HashMap;

use chrono::{DateTime, Utc};
use gpui_kit::{App, AppContext as _, Context, Entity, Subscription};
use uuid::Uuid;

use crate::connection::CancellationToken;
use crate::helpers::background_runner::{self, RunnerStatus};
use crate::models::{ConnectionEnvironment, SavedConnection};
use crate::state::compare::{CompareScope, CompareTaskLink};
use crate::tasks::model::{
    Approval, FailureKind, Run, RunStatus, RunTrigger, SyncChoice, Task, TaskSpec,
};
use crate::tasks::safety::SafetyLimit;
use crate::tasks::schedule::Schedule;
use crate::tasks::store::RunStore;

use super::{AppState, TabKey, TransferTabState};

/// Saved tasks, their recent runs, and the runs in progress.
#[derive(Default)]
pub struct TasksState {
    pub tasks: Vec<Task>,
    /// Why `tasks.json` could not be read. While set, tasks are not saved, so the file survives.
    pub load_error: Option<String>,
    /// Each task's runs, newest first, as last read from the store or recorded this session.
    pub runs: HashMap<Uuid, Vec<Run>>,
    pub active: HashMap<Uuid, ActiveRun>,
    pub store: Option<RunStore>,
    /// Each task's last sync run, while it can still be undone: until the app closes or the task
    /// runs again.
    pub undo: HashMap<Uuid, UndoLog>,
    /// Said once in the Tasks tab when runs can't be kept after the app closes.
    pub store_note: Option<String>,
    /// Scheduled runs that are due, waiting for the run before them to end: task, due time, and
    /// whether it is a catch-up. Runs go one at a time.
    pub queue: Vec<(Uuid, DateTime<Utc>, bool)>,
    /// The task the Tasks tab should select next, e.g. from a notification's Open.
    pub focus: Option<Uuid>,
    /// Makes this process the one that starts scheduled runs. Until it is held, another
    /// OpenMango process is running tasks and this one waits.
    pub lock: Option<crate::tasks::lock::SchedulerLock>,
    /// The lock couldn't be used at all, so this process schedules without it.
    pub unlocked: bool,
    /// This is the background runner: only tasks that may run while OpenMango is closed start.
    pub background: bool,
    /// The system entry that starts the background runner, as last seen.
    pub runner: RunnerStatus,
    /// What scheduled runs said as they ended, for a system notification when nobody is looking
    /// at OpenMango. Taken by whoever posts them.
    pub notices: Vec<TaskNotice>,
}

/// A scheduled run's news: its first failure in an outage, working again, or a success asked
/// for. It also shows in OpenMango, as a notification or in the status bar.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskNotice {
    pub task_id: Uuid,
    pub title: String,
    pub body: String,
}

impl TaskNotice {
    const TAG: &str = "task-";

    /// As a system notification, one per task. `open_button` only where a click on it can
    /// reach an OpenMango that's running; clicking the notification opens the task either way.
    pub fn system_notification(&self, open_button: bool) -> gpui_kit::SystemNotification {
        gpui_kit::SystemNotification {
            tag: format!("{}{}", Self::TAG, self.task_id).into(),
            title: self.title.clone().into(),
            body: self.body.clone().into(),
            actions: if open_button {
                vec![gpui_kit::SystemNotificationAction {
                    id: "open".into(),
                    label: "Open task".into(),
                }]
            } else {
                Vec::new()
            },
        }
    }

    /// The task a clicked notification is about.
    pub fn task_in(tag: &str) -> Option<Uuid> {
        tag.strip_prefix(Self::TAG)?.parse().ok()
    }
}

/// What the schedule editor sets on a task.
#[derive(Clone, Debug, Default)]
pub struct ScheduleSettings {
    pub schedule: Schedule,
    pub safety: SafetyLimit,
    /// A scheduled export keeps only this many of its newest files.
    pub keep_files: Option<u32>,
    /// Scheduled runs may write to a Production or protected connection.
    pub protected_writes: bool,
    /// Each scheduled run says when it starts and how it ended, not only problems.
    pub notify_every_run: bool,
    /// The background runner may start its runs while OpenMango is closed.
    pub run_when_closed: bool,
}

/// Why a task needs attention, and the button that fixes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Attention {
    pub reason: String,
    pub detail: String,
    pub fix: Fix,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fix {
    /// Record the connections as they are now.
    Approve,
    /// Open the task in its tool to choose another connection.
    Edit,
    /// Start the schedule again.
    Resume,
    /// Run it once now, after asking, despite the safety limit.
    RunAnyway,
    /// Show the failed run's details.
    ShowRun(Uuid),
    /// Open where the background runner is switched on: Login Items or Task Scheduler.
    RunnerSettings,
}

/// Runs in a row that fail for a reason that can pass before a task needs attention anyway.
const TEMPORARY_FAILURES: usize = 3;

/// A failed run's error in one line: the run's own, or its first failed collection's.
fn failure_text(run: &Run) -> String {
    let first = |text: &str| text.lines().next().unwrap_or_default().to_string();
    match &run.error {
        Some(error) => first(error),
        None => run
            .collections
            .iter()
            .find_map(|entry| Some(format!("{}: {}", entry.name, first(entry.error.as_ref()?))))
            .unwrap_or_default(),
    }
}

/// A run in progress. Its record lives in `TasksState::runs` like a finished one.
pub struct ActiveRun {
    pub run_id: Uuid,
    pub stop: RunStop,
    /// Transfers run through a Transfer tab state that isn't shown; its runtime holds progress.
    pub transfer_id: Option<Uuid>,
    /// A scheduled run stops retrying when its task's next run is due.
    pub retry_until: Option<std::time::Instant>,
    pub(crate) _events: Option<Subscription>,
}

/// What undoing a sync run needs: its undo records, one per collection and pass.
#[derive(Clone)]
pub struct UndoLog {
    pub run_id: Uuid,
    pub connection_id: Uuid,
    pub database: String,
    pub logs: Vec<(
        usize,
        String,
        std::sync::Arc<crate::connection::ops::compare_sync::restore::RestoreHandle>,
    )>,
}

pub enum RunStop {
    Transfer(Uuid),
    Token(CancellationToken),
}

/// Whether the two specs write the same way: the same kind, into the same connection, and for
/// a sync, in the same direction and mode.
fn same_writes(before: &TaskSpec, after: &TaskSpec) -> bool {
    before.kind() == after.kind()
        && before.write_connection() == after.write_connection()
        && before.sync_choice() == after.sync_choice()
}

/// The connection's identity, as task approval records it.
fn identity(connection: &SavedConnection) -> String {
    crate::actions::connection_identity_hash(connection).unwrap_or_default()
}

impl AppState {
    pub fn task(&self, id: Uuid) -> Option<&Task> {
        self.tasks.tasks.iter().find(|task| task.id == id)
    }

    pub fn task_runs(&self, id: Uuid) -> &[Run] {
        self.tasks.runs.get(&id).map(Vec::as_slice).unwrap_or_default()
    }

    pub fn task_run(&self, task_id: Uuid, run_id: Uuid) -> Option<&Run> {
        self.task_runs(task_id).iter().find(|run| run.id == run_id)
    }

    pub fn task_is_running(&self, id: Uuid) -> bool {
        self.tasks.active.contains_key(&id)
    }

    /// Adds or replaces the task and writes the list. Refused while `tasks.json` is unreadable.
    pub fn upsert_task(&mut self, mut task: Task) -> Result<(), String> {
        if let Some(error) = &self.tasks.load_error {
            return Err(error.clone());
        }
        task.updated_at = chrono::Utc::now();
        match self.tasks.tasks.iter_mut().find(|existing| existing.id == task.id) {
            Some(existing) => {
                // Scheduled writes were approved for what the task wrote before.
                if !same_writes(&existing.spec, &task.spec) {
                    task.approval = None;
                }
                *existing = task;
            }
            None => self.tasks.tasks.push(task),
        }
        self.save_tasks()
    }

    pub fn remove_task(&mut self, id: Uuid) -> Result<(), String> {
        if let Some(error) = &self.tasks.load_error {
            return Err(error.clone());
        }
        self.tasks.tasks.retain(|task| task.id != id);
        self.tasks.runs.remove(&id);
        if let Some(store) = &self.tasks.store
            && let Err(error) = store.delete_task(id)
        {
            log::warn!("Could not delete the task's runs: {error:#}");
        }
        self.save_tasks()?;
        self.sync_background_runner();
        Ok(())
    }

    /// Sets when the task runs by itself, with its safety limit. A schedule for a task that
    /// writes approves the connections it uses as they are now; `protected_writes` lets its runs
    /// write to a Production or protected connection. `notify_every_run`: each scheduled
    /// run says when it starts and how it ended.
    pub fn set_task_schedule(
        &mut self,
        id: Uuid,
        settings: ScheduleSettings,
    ) -> Result<(), String> {
        let mut task = self.task(id).cloned().ok_or("The task no longer exists.")?;
        let scheduled = !settings.schedule.is_manual();
        task.approval = (scheduled && task.spec.write_connection().is_some())
            .then(|| self.task_approval(&task, settings.protected_writes));
        task.schedule = settings.schedule;
        task.safety = settings.safety;
        task.keep_files = settings.keep_files;
        task.notify_every_run = settings.notify_every_run;
        task.run_when_closed = scheduled && settings.run_when_closed;
        task.paused = false;
        task.paused_by_sign_in = false;
        task.schedule_from = Some(Utc::now());
        self.upsert_task(task)?;
        self.sync_background_runner();
        Ok(())
    }

    /// Sets up the system entry that starts the background runner while any task may run with
    /// OpenMango closed, and removes it once none may.
    pub(crate) fn sync_background_runner(&mut self) {
        let wanted = self.wants_background_runner();
        // Nothing wants one and none is known to be there, so the system isn't asked: on Windows
        // and Linux that starts a program, at every launch.
        let known =
            matches!(self.tasks.runner, RunnerStatus::Enabled | RunnerStatus::NeedsApproval);
        if !wanted && !known {
            return;
        }
        let status = background_runner::status();
        let result = match (wanted, status) {
            (true, RunnerStatus::NotRegistered) => background_runner::register(),
            (false, RunnerStatus::Enabled | RunnerStatus::NeedsApproval) => {
                background_runner::unregister().map(|()| RunnerStatus::NotRegistered)
            }
            _ => Ok(status),
        };
        self.tasks.runner = match result {
            Ok(status) => status,
            Err(error) => {
                self.report_error(crate::error::ErrorReport::new(
                    "Couldn't set up running tasks while OpenMango is closed",
                    error,
                ));
                background_runner::status()
            }
        };
    }

    fn wants_background_runner(&self) -> bool {
        self.tasks.tasks.iter().any(|task| task.run_when_closed && !task.schedule.is_manual())
    }

    /// Reads the background runner's status again, off the main thread, since it's switched on
    /// and off outside OpenMango too.
    pub(crate) fn refresh_background_runner(&mut self, cx: &mut Context<Self>) {
        if !self.wants_background_runner() {
            return;
        }
        let status = cx.background_spawn(async { background_runner::status() });
        cx.spawn(async move |state, cx| {
            let status = status.await;
            let _ = state.update(cx, |state, cx| {
                state.tasks.runner = status;
                cx.notify();
            });
        })
        .detach();
    }

    /// Records the connections a task uses as they are now, for its scheduled runs to write.
    pub fn approve_task(&mut self, id: Uuid, protected_writes: bool) -> Result<(), String> {
        let mut task = self.task(id).cloned().ok_or("The task no longer exists.")?;
        task.approval = Some(self.task_approval(&task, protected_writes));
        self.upsert_task(task)
    }

    /// Pauses or resumes the task's schedule. A resumed schedule counts from now, so runs missed
    /// while it was paused don't catch up.
    pub fn set_task_paused(&mut self, id: Uuid, paused: bool) -> Result<(), String> {
        let mut task = self.task(id).cloned().ok_or("The task no longer exists.")?;
        task.paused = paused;
        if !paused {
            task.paused_by_sign_in = false;
            task.schedule_from = Some(Utc::now());
        }
        self.upsert_task(task)
    }

    /// A connection was edited: tasks whose schedule paused after signing in to it failed run
    /// again.
    pub(crate) fn resume_tasks_after_sign_in_fix(&mut self, connection_id: Uuid) {
        let paused: Vec<Uuid> = self
            .tasks
            .tasks
            .iter()
            .filter(|task| {
                task.paused_by_sign_in && task.spec.connections().contains(&connection_id)
            })
            .map(|task| task.id)
            .collect();
        for id in paused {
            if let Err(error) = self.set_task_paused(id, false) {
                log::warn!("Could not resume a task after its connection was edited: {error}");
            }
        }
    }

    /// Selects the task in the Tasks tab, opening the tab.
    pub fn open_task(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.tasks.focus = Some(id);
        self.open_tasks_tab(cx);
        cx.notify();
    }

    /// Why the task needs attention, if it does. Only a scheduled task can: a run someone
    /// started shows its result to them.
    pub fn task_attention(&self, task: &Task) -> Option<Attention> {
        if task.schedule.is_manual() {
            return None;
        }
        if task.spec.connections().iter().any(|id| self.connection_by_id(*id).is_none()) {
            return Some(Attention {
                reason: "A connection this task uses was deleted.".into(),
                detail: "Scheduled runs fail until you edit the task to choose another connection."
                    .into(),
                fix: Fix::Edit,
            });
        }
        if task.paused_by_sign_in {
            return Some(Attention {
                reason: "Signing in failed, so the schedule is paused.".into(),
                detail: "Edit the connection, or choose Resume, to run it on its schedule again."
                    .into(),
                fix: Fix::Resume,
            });
        }
        if let Some(problem) = self.task_approval_problem(task) {
            return Some(Attention {
                reason: problem,
                detail: "Scheduled runs fail until you approve the connections as they are now."
                    .into(),
                fix: Fix::Approve,
            });
        }
        if task.run_when_closed && self.tasks.runner != RunnerStatus::Enabled {
            let detail = match self.tasks.runner {
                RunnerStatus::Unavailable(why) => why.to_string(),
                _ => background_runner::SWITCH_ON.into(),
            };
            return Some(Attention {
                reason: "It can't run while OpenMango is closed.".into(),
                detail,
                fix: Fix::RunnerSettings,
            });
        }
        if task.paused {
            return None;
        }
        let runs: Vec<&Run> = self
            .task_runs(task.id)
            .iter()
            .filter(|run| {
                run.trigger.is_run()
                    && !matches!(run.status, RunStatus::Running | RunStatus::Skipped)
            })
            .collect();
        let last = runs.first().filter(|run| run.failed())?;
        if !last.stops.is_empty() {
            return Some(Attention {
                reason: "The safety limit stopped the last run.".into(),
                detail: "Nothing was written. Preview shows what it would change.".into(),
                fix: Fix::RunAnyway,
            });
        }
        let in_a_row = runs.iter().take_while(|run| run.failed()).count();
        if last.failure == Some(FailureKind::Temporary) && in_a_row < TEMPORARY_FAILURES {
            return None;
        }
        let reason = match last.status {
            _ if in_a_row >= TEMPORARY_FAILURES => format!("The last {in_a_row} runs failed."),
            RunStatus::PartlyDone => "Some collections failed in the last run.".into(),
            RunStatus::Interrupted => "The last run was cut short when OpenMango closed.".into(),
            _ => "The last run failed.".into(),
        };
        Some(Attention { reason, detail: failure_text(last), fix: Fix::ShowRun(last.id) })
    }

    /// How many tasks need attention, for the sidebar's badge.
    pub fn tasks_needing_attention(&self) -> usize {
        self.tasks.tasks.iter().filter(|task| self.task_attention(task).is_some()).count()
    }

    /// Posts what scheduled runs said as system notifications, and returns whether there were
    /// any. While someone is `looking` at OpenMango, its own notification or the status bar
    /// already says it, so it's dropped. `open_button` only where a click can reach this process.
    pub fn post_task_notices(
        state: &Entity<Self>,
        looking: bool,
        open_button: bool,
        cx: &mut App,
    ) -> bool {
        let notices = state.update(cx, |app, _| std::mem::take(&mut app.tasks.notices));
        if looking {
            return false;
        }
        for notice in &notices {
            cx.show_system_notification(notice.system_notification(open_button));
        }
        !notices.is_empty()
    }

    /// A click on a task's system notification opens the task. That includes a click that
    /// starts OpenMango: macOS hands it over once this is registered, while the app launches.
    pub fn open_tasks_from_notifications(state: Entity<Self>, cx: &mut App) {
        cx.on_system_notification_response(move |response, cx| {
            if let Some(id) = TaskNotice::task_in(&response.tag) {
                state.update(cx, |app, cx| app.open_task(id, cx));
                cx.activate(true);
            }
        });
    }

    /// A scheduled run of a task that notifies of every run says it started. Its end replaces
    /// the notification, which is one per task.
    fn task_run_started(&mut self, run: &Run) {
        let Some(task) = self.task(run.task_id).filter(|task| task.notify_every_run) else {
            return;
        };
        if !matches!(run.trigger, RunTrigger::Schedule | RunTrigger::CatchUp) {
            return;
        }
        let text = format!("“{}” started.", task.name);
        self.tasks.notices.push(TaskNotice {
            task_id: task.id,
            title: text.trim_end_matches('.').to_string(),
            body: format!(
                "{} · {}",
                if run.trigger == RunTrigger::CatchUp {
                    "Catching up a missed run"
                } else {
                    "On its schedule"
                },
                task.spec.subject()
            ),
        });
        self.set_status_message(Some(crate::state::StatusMessage::info(text)));
    }

    /// After a run ends: a failed sign-in pauses the task's schedule, and a scheduled run says
    /// how it went once per outage and when it works again, or every time when asked to.
    fn task_run_ended(&mut self, run: &Run) {
        let Some(mut task) = self.task(run.task_id).cloned() else {
            return;
        };
        let paused_now =
            run.failure == Some(FailureKind::SignIn) && !task.schedule.is_manual() && !task.paused;
        if paused_now {
            task.paused = true;
            task.paused_by_sign_in = true;
            if let Err(error) = self.upsert_task(task.clone()) {
                log::warn!("Could not pause a task after signing in failed: {error}");
            }
        }
        if !matches!(run.trigger, RunTrigger::Schedule | RunTrigger::CatchUp) {
            return;
        }
        let previous = self.task_runs(task.id).iter().find(|earlier| {
            earlier.id != run.id
                && earlier.trigger.is_run()
                && !matches!(earlier.status, RunStatus::Running | RunStatus::Skipped)
        });
        // The same outage: the run before failed the same way.
        let same_outage =
            previous.is_some_and(|previous| previous.failed() && previous.failure == run.failure);
        if run.failed() && (!same_outage || paused_now || task.notify_every_run) {
            let (title, message) = if paused_now {
                (
                    format!("“{}” couldn't sign in", task.name),
                    "Its schedule is paused until you edit the connection or resume it."
                        .to_string(),
                )
            } else {
                (format!("“{}” failed", task.name), failure_text(run))
            };
            self.tasks.notices.push(TaskNotice {
                task_id: task.id,
                title: title.clone(),
                body: message.clone(),
            });
            self.report_error_with_action(
                crate::error::ErrorReport::new(title, message),
                super::ErrorAction::OpenTask(task.id),
            );
        } else if run.status == RunStatus::Succeeded {
            let summary = crate::views::tasks::run_summary(task.spec.kind(), run);
            let text = if previous.is_some_and(Run::failed) {
                format!("“{}” works again.", task.name)
            } else if task.notify_every_run {
                format!("“{}” finished.", task.name)
            } else {
                return;
            };
            self.tasks.notices.push(TaskNotice {
                task_id: task.id,
                title: text.trim_end_matches('.').to_string(),
                body: summary,
            });
            self.set_status_message(Some(crate::state::StatusMessage::info(text)));
        }
    }

    /// Notes that the scheduler dealt with the task's run due at `due`, so it isn't run again.
    pub(crate) fn mark_task_due(&mut self, id: Uuid, due: DateTime<Utc>) {
        if let Some(task) = self.tasks.tasks.iter_mut().find(|task| task.id == id) {
            task.schedule_from = Some(due);
        }
        if let Err(error) = self.save_tasks() {
            log::warn!("Could not save when a task last ran: {error}");
        }
    }

    fn task_approval(&self, task: &Task, protected_writes: bool) -> Approval {
        let connections = task
            .spec
            .connections()
            .into_iter()
            .filter_map(|id| Some((id, identity(self.connection_by_id(id)?))))
            .collect();
        Approval { connections, protected_writes }
    }

    /// The name of the Production or protected connection the task writes to, if it writes to
    /// one.
    pub fn task_protected_target(&self, task: &Task) -> Option<String> {
        let connection = self.connection_by_id(task.spec.write_connection()?)?;
        (connection.protected || connection.environment == Some(ConnectionEnvironment::Production))
            .then(|| connection.name.clone())
    }

    /// Why a scheduled run of the task may not write now, if it may not: a connection it uses
    /// changed or was deleted since its schedule was approved.
    pub fn task_approval_problem(&self, task: &Task) -> Option<String> {
        task.spec.write_connection()?;
        let Some(approval) = &task.approval else {
            return Some("This task's schedule hasn't been approved.".into());
        };
        for id in task.spec.connections() {
            let Some(connection) = self.connection_by_id(id) else {
                return Some("A connection this task uses was deleted.".into());
            };
            let approved = approval.connections.iter().find(|(approved, _)| *approved == id);
            if approved.is_none_or(|(_, hash)| *hash != identity(connection)) {
                return Some("Connection settings changed since this task was approved.".into());
            }
        }
        if !approval.protected_writes
            && let Some(name) = self.task_protected_target(task)
        {
            return Some(format!(
                "Scheduled runs may not write to {name}, which is Production or protected."
            ));
        }
        None
    }

    fn save_tasks(&mut self) -> Result<(), String> {
        // While the background runner holds the lock, the app can be open too, and both write.
        let scheduler = self.tasks.lock.is_some() || self.tasks.unlocked;
        if (self.tasks.background || !scheduler)
            && let Ok(mut saved) = self.config.load_tasks()
        {
            if self.tasks.background {
                // The runner changes only when a task last ran and the sign-in pause; the rest
                // of the list is whatever the app has saved.
                for task in &mut saved {
                    let Some(mine) = self.tasks.tasks.iter().find(|mine| mine.id == task.id) else {
                        continue;
                    };
                    task.schedule_from = task.schedule_from.max(mine.schedule_from);
                    if mine.paused_by_sign_in {
                        (task.paused, task.paused_by_sign_in) = (true, true);
                    }
                }
                self.tasks.tasks = saved;
            } else {
                // The runner may have handled a due time since the list was read: keep the
                // later one, so no run happens twice.
                for task in &mut self.tasks.tasks {
                    let on_disk = saved.iter().find(|saved| saved.id == task.id);
                    if let Some(from) = on_disk.and_then(|saved| saved.schedule_from) {
                        task.schedule_from = task.schedule_from.max(Some(from));
                    }
                }
            }
        }
        self.config.save_tasks(&self.tasks.tasks).map_err(|error| format!("{error:#}"))
    }

    /// Records the run, replacing an earlier state of the same run. `persist` writes it to the
    /// store too; progress updates in between stay in memory.
    pub(crate) fn record_task_run(&mut self, run: Run, persist: bool) {
        if persist
            && let Some(store) = &self.tasks.store
            && let Err(error) = store.save(&run)
        {
            log::warn!("Could not save a task run: {error:#}");
        }
        let ended = persist && !matches!(run.status, RunStatus::Running | RunStatus::Skipped);
        let ended_run = ended.then(|| run.clone());
        let runs = self.tasks.runs.entry(run.task_id).or_default();
        let started = match runs.iter_mut().find(|existing| existing.id == run.id) {
            Some(existing) => {
                *existing = run;
                None
            }
            None => {
                let started = (run.status == RunStatus::Running).then(|| run.clone());
                runs.insert(0, run);
                started
            }
        };
        runs.truncate(crate::tasks::store::RUNS_PER_TASK);
        if let Some(run) = started {
            self.task_run_started(&run);
        }
        if let Some(run) = ended_run {
            self.task_run_ended(&run);
        }
    }

    /// Opens a Transfer tab state that no tab shows, for a task to run through.
    pub(crate) fn insert_task_transfer(&mut self, tab: TransferTabState) -> Uuid {
        let id = Uuid::new_v4();
        self.transfer_tabs.insert(id, tab);
        id
    }

    pub(crate) fn remove_task_transfer(&mut self, id: Uuid) -> Option<TransferTabState> {
        self.transfer_tabs.remove(&id)
    }

    /// Takes the opened run store and loads every task's runs.
    pub(crate) fn attach_task_runs(&mut self, store: RunStore, note: Option<String>) {
        self.tasks.store = Some(store);
        self.tasks.store_note = note;
        self.read_task_runs();
    }

    /// Reads every task's runs from the store. The process that starts scheduled runs first marks
    /// the runs no process is finishing any more as interrupted.
    fn read_task_runs(&mut self) {
        let Some(store) = self.tasks.store.clone() else {
            return;
        };
        if self.tasks.lock.is_some() || self.tasks.unlocked {
            let active: Vec<Uuid> = self.tasks.active.values().map(|run| run.run_id).collect();
            match store.mark_interrupted(&active) {
                Ok(0) => {}
                Ok(count) => log::warn!("{count} task run(s) were cut short"),
                Err(error) => log::warn!("Could not mark interrupted task runs: {error:#}"),
            }
        }
        for task in &self.tasks.tasks {
            match store.runs(task.id) {
                Ok(runs) => {
                    self.tasks.runs.insert(task.id, runs);
                }
                Err(error) => log::warn!("Could not read the runs of {}: {error:#}", task.name),
            }
        }
    }

    /// Makes this process the one that starts scheduled runs, unless another OpenMango process
    /// is. Taking over from the background runner reads again what it recorded.
    pub(crate) fn take_scheduler_lock(&mut self) -> bool {
        if self.tasks.lock.is_some() || self.tasks.unlocked {
            return true;
        }
        match self.config.take_scheduler_lock() {
            Ok(Some(lock)) => self.tasks.lock = Some(lock),
            Ok(None) => return false,
            Err(error) => {
                log::warn!("Scheduled runs start without the lock, which can't be used: {error}");
                self.tasks.unlocked = true;
            }
        }
        if self.tasks.load_error.is_none() {
            match self.config.load_tasks() {
                Ok(tasks) => self.tasks.tasks = tasks,
                Err(error) => log::warn!("Could not read tasks again: {error:#}"),
            }
        }
        self.read_task_runs();
        true
    }

    /// What saving this Transfer tab as a task would store.
    pub fn transfer_task_spec(&self, transfer_id: Uuid) -> Option<TaskSpec> {
        let tab = self.transfer_tab(transfer_id)?;
        Some(TaskSpec::Transfer { config: tab.config.clone(), options: tab.options.clone() })
    }

    /// What saving the Compare tab starts from: a sync in the sync list's direction and mode
    /// while it's showing, else in those of the Sync task the tab belongs to, else a
    /// comparison. Only a database comparison can sync.
    pub fn compare_save_choice(&self, compare_id: Uuid) -> SyncChoice {
        let tab = self.compare_tab(compare_id)?;
        if tab.config.scope != CompareScope::Databases {
            return None;
        }
        if let Some(target) = tab.sync.target {
            return Some((target, tab.sync.mode));
        }
        self.task(tab.task.as_ref()?.id)?.spec.sync_choice()
    }

    /// What saving this Compare tab as a task stores: a comparison, or with `choice` a sync.
    pub fn compare_task_spec(&self, compare_id: Uuid, choice: SyncChoice) -> Option<TaskSpec> {
        let tab = self.compare_tab(compare_id)?;
        let config = tab.config.clone();
        let Some((target, mode)) = choice.filter(|_| config.scope == CompareScope::Databases)
        else {
            return Some(TaskSpec::Compare { config });
        };
        // The sync list's unticked collections, or with it closed, the task's.
        let excluded = if tab.sync.target.is_some() {
            tab.sync_excluded_names()
        } else {
            match tab.task.as_ref().and_then(|link| self.task(link.id)).map(|task| &task.spec) {
                Some(TaskSpec::Sync { excluded, .. }) => excluded.clone(),
                _ => Vec::new(),
            }
        };
        Some(TaskSpec::Sync { config, target, mode, excluded })
    }

    /// The task a Transfer or Compare tab is linked to.
    pub fn tab_task_id(&self, tab: &TabKey) -> Option<Uuid> {
        match tab {
            TabKey::Transfer(key) => self.transfer_tab(key.id)?.task_id,
            TabKey::Compare(key) => self.compare_tab(key.id)?.task.as_ref().map(|task| task.id),
            _ => None,
        }
    }

    /// Links a tab to a task, so the next save updates that task.
    pub fn link_tab_to_task(&mut self, tab: &TabKey, task_id: Uuid) {
        match tab {
            TabKey::Transfer(key) => {
                if let Some(state) = self.transfer_tab_mut(key.id) {
                    state.task_id = Some(task_id);
                }
            }
            TabKey::Compare(key) => {
                let sync = match self.task(task_id).map(|task| &task.spec) {
                    Some(TaskSpec::Sync { target, mode, excluded, .. }) => {
                        Some((*target, *mode, excluded.clone()))
                    }
                    _ => None,
                };
                if let Some(state) = self.compare_tab_mut(key.id) {
                    state.task = Some(CompareTaskLink { id: task_id, sync });
                }
            }
            _ => {}
        }
    }

    /// Opens the task in its tool, or shows the tab already open for it.
    pub fn edit_task(&mut self, task_id: Uuid, cx: &mut Context<Self>) {
        let Some(task) = self.task(task_id).cloned() else {
            return;
        };
        if let Some(index) =
            self.tabs.open.iter().position(|tab| self.tab_task_id(tab) == Some(task_id))
        {
            self.select_tab(index, cx);
            return;
        }
        match task.spec {
            TaskSpec::Transfer { config, options } => {
                let mut tab = TransferTabState::from_settings(&self.settings);
                let connection = config.source_connection_id;
                tab.config = config;
                tab.options = options;
                tab.task_id = Some(task.id);
                self.push_transfer_tab(tab, connection, cx);
            }
            TaskSpec::Compare { config } => {
                let id = self.open_compare_tab_with(config, cx);
                if let Some(tab) = self.compare_tab_mut(id) {
                    tab.task = Some(CompareTaskLink { id: task.id, sync: None });
                }
            }
            TaskSpec::Sync { config, target, mode, excluded } => {
                let id = self.open_compare_tab_with(config, cx);
                if let Some(tab) = self.compare_tab_mut(id) {
                    tab.task =
                        Some(CompareTaskLink { id: task.id, sync: Some((target, mode, excluded)) });
                }
            }
        }
    }

    /// Opens a Transfer tab in `mode` for what the sidebar has selected, for New task.
    pub fn open_transfer_tab_for_mode(
        &mut self,
        mode: super::TransferMode,
        cx: &mut Context<Self>,
    ) {
        self.open_transfer_tab(cx);
        let Some(id) = self.active_transfer_tab_id() else {
            return;
        };
        if let Some(tab) = self.transfer_tab_mut(id) {
            tab.config.mode = mode;
            if mode == super::TransferMode::Import {
                tab.config.destination_database.clear();
                tab.config.destination_collection.clear();
            }
        }
    }
}
