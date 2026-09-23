use std::collections::{HashMap, HashSet};

use gpui_kit::{Context, Subscription};
use uuid::Uuid;

use crate::connection::CancellationToken;
use crate::state::compare::{CompareScope, CompareTaskLink};
use crate::tasks::model::{Run, Task, TaskSpec};
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
    /// Tasks getting ready to run: opening connections, or listing collections to sync.
    pub starting: HashSet<Uuid>,
    pub store: Option<RunStore>,
    /// Said once in the Tasks tab when runs can't be kept after the app closes.
    pub store_note: Option<String>,
}

/// A run in progress. Its record lives in `TasksState::runs` like a finished one.
pub struct ActiveRun {
    pub run_id: Uuid,
    pub stop: RunStop,
    /// Transfers run through a Transfer tab state that isn't shown; its runtime holds progress.
    pub transfer_id: Option<Uuid>,
    pub(crate) _events: Option<Subscription>,
}

pub enum RunStop {
    Transfer(Uuid),
    Token(CancellationToken),
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
            Some(existing) => *existing = task,
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
        self.save_tasks()
    }

    fn save_tasks(&self) -> Result<(), String> {
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
        let runs = self.tasks.runs.entry(run.task_id).or_default();
        match runs.iter_mut().find(|existing| existing.id == run.id) {
            Some(existing) => *existing = run,
            None => runs.insert(0, run),
        }
        runs.truncate(crate::tasks::store::RUNS_PER_TASK);
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

    /// Takes the opened run store, marks runs cut short last time and loads every task's runs.
    pub(crate) fn attach_task_runs(&mut self, store: RunStore, note: Option<String>) {
        match store.mark_interrupted() {
            Ok(0) => {}
            Ok(count) => {
                log::warn!("{count} task run(s) were interrupted when the app last closed")
            }
            Err(error) => log::warn!("Could not mark interrupted task runs: {error:#}"),
        }
        for task in &self.tasks.tasks {
            match store.runs(task.id) {
                Ok(runs) => {
                    self.tasks.runs.insert(task.id, runs);
                }
                Err(error) => log::warn!("Could not read the runs of {}: {error:#}", task.name),
            }
        }
        self.tasks.store = Some(store);
        self.tasks.store_note = note;
    }

    /// What saving this Transfer tab as a task would store.
    pub fn transfer_task_spec(&self, transfer_id: Uuid) -> Option<TaskSpec> {
        let tab = self.transfer_tab(transfer_id)?;
        Some(TaskSpec::Transfer { config: tab.config.clone(), options: tab.options.clone() })
    }

    /// What saving this Compare tab as a task would store: a Sync task while the database
    /// sync list is showing, a comparison otherwise.
    pub fn compare_task_spec(&self, compare_id: Uuid) -> Option<TaskSpec> {
        let tab = self.compare_tab(compare_id)?;
        let config = tab.config.clone();
        match tab.sync.target {
            Some(target) if config.scope == CompareScope::Databases => Some(TaskSpec::Sync {
                config,
                target,
                mode: tab.sync.mode,
                excluded: tab.sync_excluded_names(),
            }),
            _ => Some(TaskSpec::Compare { config }),
        }
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
                let sync = match self.compare_task_spec(key.id) {
                    Some(TaskSpec::Sync { target, mode, excluded, .. }) => {
                        Some((target, mode, excluded))
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
