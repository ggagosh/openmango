//! `openmango --run-due-tasks`: the system scheduler starts it about every 15 minutes. It runs
//! the tasks that may run while OpenMango is closed and are due, records them, and exits.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use gpui_kit::{AppContext as _, Entity};

use crate::connection::ConnectionManager;
use crate::helpers::keystore::KeyStore;
use crate::state::{AppCommands, AppState, ConfigManager};
use crate::tasks::model::Task;
use crate::tasks::schedule::{self, Due};
use crate::tasks::store::RunStore;

/// Whether the background runner should start the task at `now`: it may run while OpenMango is
/// closed, and a run is due or a missed one has to be recorded as skipped.
pub fn due_while_closed(task: &Task, now: DateTime<Utc>) -> bool {
    if !task.run_when_closed || task.paused || task.schedule.is_manual() {
        return false;
    }
    let from = task.schedule_from.unwrap_or(task.updated_at);
    !matches!(schedule::due(&task.schedule, from, now, &chrono::Local), Due::Wait(_))
}

/// Runs the due tasks without a window, then returns. Most starts find nothing due, or the app
/// open, and return before the UI framework starts.
pub fn run_due_tasks() {
    let config = ConfigManager::default();
    // The lock comes first, so an app opening now waits for this run rather than both running.
    let lock = match config.take_scheduler_lock() {
        Ok(Some(lock)) => lock,
        Ok(None) => return log::info!("OpenMango is open, or another run is going: nothing to do"),
        Err(error) => return log::error!("The task lock can't be used: {error}"),
    };
    let tasks = match config.load_tasks() {
        Ok(tasks) => tasks,
        Err(error) => return log::error!("Tasks couldn't be read: {error:#}"),
    };
    let now = Utc::now();
    let due: Vec<&Task> = tasks.iter().filter(|task| due_while_closed(task, now)).collect();
    if due.is_empty() {
        return log::info!("Nothing set to run while OpenMango is closed is due");
    }
    log::info!("{} task(s) due while OpenMango is closed", due.len());
    // Reading the passwords would ask to unlock the keyring, with nobody to answer. The due runs
    // stay due for the next start, or for OpenMango when it opens.
    #[cfg(target_os = "linux")]
    if crate::helpers::background_runner::keyring_locked() {
        return log::info!("The keyring is locked; trying again at the next start");
    }
    let connections: Vec<uuid::Uuid> =
        due.iter().flat_map(|task| task.spec.connections()).collect();

    #[cfg(target_os = "macos")]
    crate::helpers::background_runner::stay_out_of_dock();
    gpui_platform::headless().run(move |cx| {
        // The same setup the task tests run under; no window opens.
        gpui_kit::init(cx);
        cx.set_app_identity("com.openmango.app", "OpenMango");
        let state = cx.new(|_| {
            let mut app = AppState::with_config(Arc::new(ConnectionManager::new()), config);
            app.tasks.lock = Some(lock);
            app.tasks.background = true;
            app
        });
        let passwords = AppState::read_connection_secrets(state.clone(), &connections, cx);
        let key = KeyStore::read_task_runs_key(cx);
        cx.spawn(async move |cx| {
            passwords.await;
            let store = match key.await {
                Ok(Some(key)) => <[u8; 32]>::try_from(key)
                    .map_err(|_| anyhow::anyhow!("the key is damaged"))
                    .and_then(|key| RunStore::open(state_path(&state, cx), key)),
                Ok(None) => Err(anyhow::anyhow!("OpenMango hasn't made its key yet")),
                Err(error) => Err(error),
            };
            match store {
                Ok(store) => {
                    cx.update(|cx| {
                        state.update(cx, |app, _| app.attach_task_runs(store, None));
                        AppCommands::check_schedules(&state, Utc::now(), cx);
                    });
                    // Runs go one at a time; the queue empties as each one ends.
                    while !cx.update(|cx| idle(&state, cx)) {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                    }
                    // No Open task button: this process has exited by the time someone clicks.
                    // Clicking the notification still opens OpenMango on macOS.
                    if cx.update(|cx| AppState::post_task_notices(&state, false, false, cx)) {
                        // macOS takes the notification after the call returns.
                        cx.background_executor().timer(Duration::from_secs(2)).await;
                    }
                    log::info!("Done; each run is in its task's history");
                }
                // Without the history nothing could be recorded, so nothing runs.
                Err(error) => log::error!("Task run history can't be opened: {error:#}"),
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}

fn state_path(state: &Entity<AppState>, cx: &mut gpui_kit::AsyncApp) -> std::path::PathBuf {
    cx.update(|cx| state.read(cx).config.task_runs_path())
}

fn idle(state: &Entity<AppState>, cx: &gpui_kit::App) -> bool {
    let tasks = &state.read(cx).tasks;
    tasks.active.is_empty() && tasks.queue.is_empty()
}
