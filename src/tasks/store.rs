use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use chrono::{Duration, Utc};
use rusqlite::{Connection, params};
use uuid::Uuid;

use super::model::{Run, RunStatus};

/// Runs kept per task.
pub const RUNS_PER_TASK: usize = 100;
/// Runs older than this are removed whatever their number.
pub const RUN_MAX_AGE_DAYS: i64 = 90;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS runs (
         id         TEXT PRIMARY KEY,
         task_id    TEXT NOT NULL,
         started_at TEXT NOT NULL,
         payload    TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS runs_by_task ON runs (task_id, started_at);";

/// Every task run, in a SQLCipher database whose key lives in the OS keychain.
#[derive(Clone)]
pub struct RunStore {
    connection: Arc<Mutex<Connection>>,
}

impl RunStore {
    pub fn open(path: PathBuf, key: [u8; 32]) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Could not create {}", parent.display()))?;
        }
        match Self::open_encrypted(&path, key) {
            Ok(store) => Ok(store),
            Err(error) => {
                // Unreadable with this key, so the key was lost. The runs are a log, not data:
                // starting over loses the record, never a task.
                log::warn!("Starting task run history over: {error:#}");
                for suffix in ["", "-wal", "-shm"] {
                    let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
                }
                Self::open_encrypted(&path, key)
            }
        }
    }

    fn open_encrypted(path: &Path, key: [u8; 32]) -> Result<Self> {
        let connection =
            Connection::open(path).with_context(|| format!("Could not open {}", path.display()))?;
        // The key comes first: SQLCipher reads nothing until it is set.
        let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
        connection.pragma_update(None, "key", format!("x'{hex}'"))?;
        // The app and the background runner can both write; one waits for the other.
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;",
        )?;
        // A wrong key fails here rather than at open.
        connection
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get::<_, i64>(0))
            .context("Task run history is not readable with this key")?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self { connection: Arc::new(Mutex::new(connection)) })
    }

    /// The store used without a keychain entry: runs are forgotten when the app closes.
    pub fn in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self { connection: Arc::new(Mutex::new(connection)) })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| anyhow::anyhow!("Task run history is unavailable"))
    }

    /// Writes the run, replacing an earlier save of the same run, then trims the task's history.
    pub fn save(&self, run: &Run) -> Result<()> {
        let payload = serde_json::to_string(run)?;
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO runs (id, task_id, started_at, payload) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (id) DO UPDATE SET payload = excluded.payload",
            params![
                run.id.to_string(),
                run.task_id.to_string(),
                run.started_at.to_rfc3339(),
                payload
            ],
        )?;
        let cutoff = (Utc::now() - Duration::days(RUN_MAX_AGE_DAYS)).to_rfc3339();
        connection.execute(
            "DELETE FROM runs WHERE task_id = ?1 AND (started_at < ?2 OR id NOT IN (
                 SELECT id FROM runs WHERE task_id = ?1 ORDER BY started_at DESC LIMIT ?3))",
            params![run.task_id.to_string(), cutoff, RUNS_PER_TASK as i64],
        )?;
        Ok(())
    }

    /// The task's runs, newest first.
    pub fn runs(&self, task_id: Uuid) -> Result<Vec<Run>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT payload FROM runs WHERE task_id = ?1 ORDER BY started_at DESC")?;
        let rows =
            statement.query_map(params![task_id.to_string()], |row| row.get::<_, String>(0))?;
        let mut runs = Vec::new();
        for row in rows {
            // A run this version can no longer read is skipped rather than hiding the others.
            match serde_json::from_str(&row?) {
                Ok(run) => runs.push(run),
                Err(error) => log::warn!("Skipping an unreadable task run: {error}"),
            }
        }
        Ok(runs)
    }

    pub fn delete_task(&self, task_id: Uuid) -> Result<()> {
        self.lock()?
            .execute("DELETE FROM runs WHERE task_id = ?1", params![task_id.to_string()])?;
        Ok(())
    }

    /// Marks runs still recorded as running as interrupted, except `keep`: the process that ran
    /// them stopped during them.
    pub fn mark_interrupted(&self, keep: &[Uuid]) -> Result<usize> {
        let running: Vec<Run> = {
            let connection = self.lock()?;
            let mut statement = connection.prepare("SELECT payload FROM runs")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.filter_map(|row| row.ok())
                .filter_map(|payload| serde_json::from_str::<Run>(&payload).ok())
                .filter(|run| run.status == RunStatus::Running && !keep.contains(&run.id))
                .collect()
        };
        for mut run in running.iter().cloned() {
            run.status = RunStatus::Interrupted;
            run.finished_at.get_or_insert_with(Utc::now);
            self.save(&run)?;
        }
        Ok(running.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::model::RunTrigger;

    fn run_at(task_id: Uuid, days_ago: i64) -> Run {
        let mut run = Run::start(task_id, RunTrigger::Manual);
        run.started_at = Utc::now() - Duration::days(days_ago);
        run.finish(false);
        run
    }

    #[test]
    fn history_keeps_the_newest_runs_within_ninety_days() {
        let store = RunStore::in_memory().unwrap();
        let task = Uuid::new_v4();
        let other = Uuid::new_v4();
        store.save(&run_at(other, 1)).unwrap();
        store.save(&run_at(task, RUN_MAX_AGE_DAYS + 1)).unwrap();
        for day in 0..RUNS_PER_TASK as i64 + 3 {
            store.save(&run_at(task, day % 80)).unwrap();
        }
        let runs = store.runs(task).unwrap();
        assert_eq!(runs.len(), RUNS_PER_TASK);
        assert!(runs.windows(2).all(|pair| pair[0].started_at >= pair[1].started_at));
        assert!(
            runs.iter().all(|run| run.started_at > Utc::now() - Duration::days(RUN_MAX_AGE_DAYS))
        );
        assert_eq!(store.runs(other).unwrap().len(), 1, "other tasks keep their runs");
    }

    #[test]
    fn a_run_saved_twice_is_one_run_and_running_ones_become_interrupted() {
        let store = RunStore::in_memory().unwrap();
        let task = Uuid::new_v4();
        let mut run = Run::start(task, RunTrigger::Manual);
        store.save(&run).unwrap();
        run.log(crate::tasks::model::LogLevel::Info, "halfway");
        store.save(&run).unwrap();
        assert_eq!(store.runs(task).unwrap().len(), 1);

        assert_eq!(store.mark_interrupted(&[]).unwrap(), 1);
        let saved = &store.runs(task).unwrap()[0];
        assert_eq!(saved.status, RunStatus::Interrupted);
        assert_eq!(saved.log.len(), 1);
        assert!(saved.finished_at.is_some());
    }

    #[test]
    fn the_file_is_encrypted_and_reopens_with_its_key() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("task-runs.sqlite3");
        let key = [7u8; 32];
        let task = Uuid::new_v4();
        {
            let store = RunStore::open(path.clone(), key).unwrap();
            let mut run = run_at(task, 0);
            run.error = Some("E11000 duplicate key: { email: \"ana@example.com\" }".into());
            store.save(&run).unwrap();
        }
        for suffix in ["", "-wal"] {
            let Ok(bytes) = std::fs::read(format!("{}{suffix}", path.display())) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains("ana@example.com") && !text.contains("SQLite format"));
        }
        assert_eq!(RunStore::open(path.clone(), key).unwrap().runs(task).unwrap().len(), 1);
        // A lost key starts the history over instead of failing.
        assert!(RunStore::open(path, [8u8; 32]).unwrap().runs(task).unwrap().is_empty());
    }
}
