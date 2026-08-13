use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use super::model::{
    OperationDetails, OperationEvent, OperationId, OperationKind, OperationOrigin, OperationQuery,
    OperationStatus, OperationSummary, Page, PreparedOperation, StoredPayload,
};
use anyhow::{Context as _, Result, bail};
use chrono::{TimeZone as _, Utc};
use rusqlite::{Connection, OptionalExtension as _, params};

const SCHEMA_VERSION: i64 = 1;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const WORK_QUEUE_CAPACITY: usize = 64;

type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

enum WorkerMessage {
    Run(Job),
    Shutdown,
}

struct StoreInner {
    sender: mpsc::SyncSender<WorkerMessage>,
    join: Mutex<Option<thread::JoinHandle<()>>>,
}

impl Drop for StoreInner {
    fn drop(&mut self) {
        let _ = self.sender.send(WorkerMessage::Shutdown);
        if let Some(join) = self.join.lock().ok().and_then(|mut join| join.take()) {
            let _ = join.join();
        }
    }
}

#[derive(Clone)]
pub(crate) struct OperationStore {
    inner: Arc<StoreInner>,
    path: PathBuf,
}

impl OperationStore {
    pub(crate) fn open(path: PathBuf) -> Result<Self> {
        prepare_parent(&path)?;
        let (sender, receiver) = mpsc::sync_channel(WORK_QUEUE_CAPACITY);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let worker_path = path.clone();
        let join = thread::Builder::new()
            .name("operation-history-store".into())
            .spawn(move || {
                let connection = open_connection(&worker_path);
                match connection {
                    Ok(mut connection) => {
                        let _ = startup_sender.send(Ok(()));
                        while let Ok(message) = receiver.recv() {
                            match message {
                                WorkerMessage::Run(job) => job(&mut connection),
                                WorkerMessage::Shutdown => break,
                            }
                        }
                        let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
                        let _ = protect_store_files(&worker_path);
                    }
                    Err(error) => {
                        let _ = startup_sender.send(Err(error));
                    }
                }
            })
            .context("could not start operation history store worker")?;

        startup_receiver
            .recv()
            .context("operation history store worker stopped during startup")??;
        protect_store_files(&path)?;

        Ok(Self { inner: Arc::new(StoreInner { sender, join: Mutex::new(Some(join)) }), path })
    }

    fn call<T, F>(&self, work: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        let path = self.path.clone();
        self.inner
            .sender
            .send(WorkerMessage::Run(Box::new(move |connection| {
                let result = work(connection).and_then(|value| {
                    protect_store_files(&path)?;
                    Ok(value)
                });
                let _ = sender.send(result);
            })))
            .map_err(|_| anyhow::anyhow!("operation history store worker is unavailable"))?;
        receiver.recv().context("operation history store worker stopped")?
    }

    pub(crate) fn prepare(&self, operation: PreparedOperation) -> Result<()> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let summary = operation.summary;
            transaction.execute(
                "INSERT INTO operations (
                    id, kind, origin, connection_id, connection_name, database_name,
                    collection_name, status, parent_operation_id, reverts_operation_id,
                    created_at_ms, updated_at_ms, recovery_status
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL)",
                params![
                    summary.id.to_string(),
                    summary.kind.as_str(),
                    summary.origin.as_str(),
                    summary.connection_id.to_string(),
                    summary.connection_name,
                    summary.database,
                    summary.collection,
                    summary.status.as_str(),
                    summary.parent_operation_id.map(|id| id.to_string()),
                    summary.reverts_operation_id.map(|id| id.to_string()),
                    summary.created_at.timestamp_millis(),
                    summary.updated_at.timestamp_millis(),
                ],
            )?;
            let payload = operation.payload;
            transaction.execute(
                "INSERT INTO operation_items (
                    operation_id, payload_version, encrypted_payload, target_hash,
                    before_hash, after_hash, outcome
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'prepared')",
                params![
                    summary.id.to_string(),
                    payload.version,
                    payload.encrypted,
                    payload.target_hash.as_slice(),
                    payload.before_hash.as_slice(),
                    payload.after_hash.as_slice(),
                ],
            )?;
            insert_event(&transaction, summary.id, "prepared", OperationStatus::Prepared)?;
            transaction.commit()?;
            Ok(())
        })
    }

    pub(crate) fn transition(
        &self,
        id: OperationId,
        status: OperationStatus,
        event_type: &'static str,
        item_outcome: &'static str,
        recovery_status: Option<&'static str>,
    ) -> Result<()> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let changed = transaction.execute(
                "UPDATE operations
                 SET status = ?2, updated_at_ms = ?3, recovery_status = ?4
                 WHERE id = ?1",
                params![
                    id.to_string(),
                    status.as_str(),
                    Utc::now().timestamp_millis(),
                    recovery_status,
                ],
            )?;
            if changed != 1 {
                bail!("operation was not found");
            }
            transaction.execute(
                "UPDATE operation_items SET outcome = ?2 WHERE operation_id = ?1",
                params![id.to_string(), item_outcome],
            )?;
            insert_event(&transaction, id, event_type, status)?;
            transaction.commit()?;
            Ok(())
        })
    }

    pub(crate) fn record_recovery_status(
        &self,
        id: OperationId,
        event_type: &'static str,
        recovery_status: &'static str,
    ) -> Result<()> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let status: String = transaction.query_row(
                "SELECT status FROM operations WHERE id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )?;
            transaction.execute(
                "UPDATE operations SET updated_at_ms = ?2, recovery_status = ?3 WHERE id = ?1",
                params![id.to_string(), Utc::now().timestamp_millis(), recovery_status],
            )?;
            insert_event(&transaction, id, event_type, OperationStatus::parse(&status)?)?;
            transaction.commit()?;
            Ok(())
        })
    }

    pub(crate) fn get(&self, id: OperationId) -> Result<Option<OperationDetails>> {
        self.call(move |connection| {
            let Some(summary) = query_summary(connection, id)? else {
                return Ok(None);
            };
            let mut statement = connection.prepare(
                "SELECT event_type, status, created_at_ms
                 FROM operation_events WHERE operation_id = ?1 ORDER BY id ASC",
            )?;
            let rows = statement.query_map([id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
            })?;
            let events = rows
                .map(|row| {
                    let (event_type, status, created_at_ms) = row?;
                    Ok(OperationEvent {
                        event_type,
                        status: OperationStatus::parse(&status)?,
                        created_at: timestamp(created_at_ms)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Some(OperationDetails { summary, events }))
        })
    }

    pub(crate) fn list(&self, query: OperationQuery) -> Result<Page<OperationSummary>> {
        let limit = query.limit.clamp(1, 100);
        let offset = query.offset;
        let connection_id = query.connection_id.map(|id| id.to_string());
        self.call(move |connection| {
            let total = connection.query_row(
                "SELECT COUNT(*) FROM operations
                 WHERE (?1 IS NULL OR connection_id = ?1)
                   AND (?2 IS NULL OR database_name = ?2)
                   AND (?3 IS NULL OR collection_name = ?3)",
                params![
                    connection_id.as_deref(),
                    query.database.as_deref(),
                    query.collection.as_deref()
                ],
                |row| row.get::<_, u64>(0),
            )?;
            let mut statement = connection.prepare(
                "SELECT id, kind, origin, connection_id, connection_name, database_name,
                        collection_name, status, parent_operation_id, reverts_operation_id,
                        created_at_ms, updated_at_ms, recovery_status
                 FROM operations
                 WHERE (?1 IS NULL OR connection_id = ?1)
                   AND (?2 IS NULL OR database_name = ?2)
                   AND (?3 IS NULL OR collection_name = ?3)
                 ORDER BY created_at_ms DESC, rowid DESC
                 LIMIT ?4 OFFSET ?5",
            )?;
            let rows = statement.query_map(
                params![
                    connection_id.as_deref(),
                    query.database.as_deref(),
                    query.collection.as_deref(),
                    limit,
                    offset
                ],
                raw_summary,
            )?;
            let mut items = rows
                .map(|row| summary_from_raw(row?))
                .collect::<Result<Vec<OperationSummary>>>()?;
            for summary in &mut items {
                summary.has_completed_revert = completed_revert_exists(connection, summary.id)?;
            }
            let consumed = offset.saturating_add(items.len() as u32);
            Ok(Page {
                items,
                next_offset: (u64::from(consumed) < total).then_some(consumed),
                total,
            })
        })
    }

    pub(crate) fn list_incomplete(&self) -> Result<Vec<OperationSummary>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT id, kind, origin, connection_id, connection_name, database_name,
                        collection_name, status, parent_operation_id, reverts_operation_id,
                        created_at_ms, updated_at_ms, recovery_status
                 FROM operations
                 WHERE status IN ('prepared', 'running', 'uncertain')
                 ORDER BY created_at_ms ASC, rowid ASC",
            )?;
            statement
                .query_map([], raw_summary)?
                .map(|row| summary_from_raw(row?))
                .collect::<Result<Vec<_>>>()
        })
    }

    pub(crate) fn payload(&self, id: OperationId) -> Result<Option<StoredPayload>> {
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT payload_version, encrypted_payload, target_hash, before_hash, after_hash
                     FROM operation_items WHERE operation_id = ?1",
                    [id.to_string()],
                    |row| {
                        Ok((
                            row.get::<_, u32>(0)?,
                            row.get::<_, Vec<u8>>(1)?,
                            row.get::<_, Vec<u8>>(2)?,
                            row.get::<_, Vec<u8>>(3)?,
                            row.get::<_, Vec<u8>>(4)?,
                        ))
                    },
                )
                .optional()?
                .map(|(version, encrypted, target_hash, before_hash, after_hash)| {
                    Ok(StoredPayload {
                        version,
                        encrypted,
                        target_hash: fixed_hash(target_hash)?,
                        before_hash: fixed_hash(before_hash)?,
                        after_hash: fixed_hash(after_hash)?,
                    })
                })
                .transpose()
        })
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

fn open_connection(path: &Path) -> Result<Connection> {
    let mut connection = Connection::open(path).context("could not open operation history")?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    let journal_mode: String =
        connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        connection.pragma_update(None, "journal_mode", "WAL")?;
    }
    connection.pragma_update(None, "synchronous", "FULL")?;
    migrate(&mut connection)?;
    verify_integrity(&connection)?;
    Ok(connection)
}

fn migrate(connection: &mut Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at_ms INTEGER NOT NULL
        );",
    )?;
    let version = connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if version > SCHEMA_VERSION {
        bail!("operation history was created by a newer OpenMango version");
    }
    if version < 1 {
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            "CREATE TABLE operations (
                id TEXT PRIMARY KEY NOT NULL,
                kind TEXT NOT NULL,
                origin TEXT NOT NULL,
                connection_id TEXT NOT NULL,
                connection_name TEXT NOT NULL,
                database_name TEXT NOT NULL,
                collection_name TEXT NOT NULL,
                status TEXT NOT NULL,
                parent_operation_id TEXT REFERENCES operations(id),
                reverts_operation_id TEXT REFERENCES operations(id),
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                recovery_status TEXT
            );
            CREATE INDEX operations_created_at_idx
                ON operations(created_at_ms DESC);
            CREATE INDEX operations_status_idx ON operations(status);
            CREATE TABLE operation_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                operation_id TEXT NOT NULL REFERENCES operations(id),
                event_type TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );
            CREATE INDEX operation_events_operation_idx
                ON operation_events(operation_id, id);
            CREATE TABLE operation_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                operation_id TEXT NOT NULL UNIQUE REFERENCES operations(id),
                payload_version INTEGER NOT NULL,
                encrypted_payload BLOB NOT NULL,
                target_hash BLOB NOT NULL,
                before_hash BLOB NOT NULL,
                after_hash BLOB NOT NULL,
                outcome TEXT NOT NULL
            );",
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, applied_at_ms) VALUES (1, ?1)",
            [Utc::now().timestamp_millis()],
        )?;
        transaction.pragma_update(None, "user_version", 1)?;
        transaction.commit()?;
    }
    let migrated = connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let user_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if migrated != SCHEMA_VERSION || user_version != SCHEMA_VERSION {
        bail!("operation history schema version is inconsistent");
    }
    Ok(())
}

fn verify_integrity(connection: &Connection) -> Result<()> {
    let integrity: String =
        connection.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
    if integrity != "ok" {
        bail!("operation history integrity check failed");
    }
    let foreign_key_error: Option<i64> = connection
        .query_row("SELECT 1 FROM pragma_foreign_key_check LIMIT 1", [], |row| row.get(0))
        .optional()?;
    if foreign_key_error.is_some() {
        bail!("operation history foreign key check failed");
    }
    Ok(())
}

fn prepare_parent(path: &Path) -> Result<()> {
    let parent = path.parent().context("operation history path has no parent")?;
    fs::create_dir_all(parent).context("could not create operation history directory")?;
    set_permissions(parent, 0o700)?;
    Ok(())
}

fn protect_store_files(path: &Path) -> Result<()> {
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        if candidate.exists() {
            set_permissions(&candidate, 0o600)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_permissions(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_permissions(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

fn insert_event(
    transaction: &rusqlite::Transaction<'_>,
    id: OperationId,
    event_type: &str,
    status: OperationStatus,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO operation_events(operation_id, event_type, status, created_at_ms)
         VALUES (?1, ?2, ?3, ?4)",
        params![id.to_string(), event_type, status.as_str(), Utc::now().timestamp_millis(),],
    )?;
    Ok(())
}

type RawSummary = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<String>,
);

fn raw_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawSummary> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
    ))
}

fn summary_from_raw(raw: RawSummary) -> Result<OperationSummary> {
    Ok(OperationSummary {
        id: raw.0.parse()?,
        kind: OperationKind::parse(&raw.1)?,
        origin: OperationOrigin::parse(&raw.2)?,
        connection_id: raw.3.parse()?,
        connection_name: raw.4,
        database: raw.5,
        collection: raw.6,
        status: OperationStatus::parse(&raw.7)?,
        parent_operation_id: raw.8.map(|value| value.parse()).transpose()?,
        reverts_operation_id: raw.9.map(|value| value.parse()).transpose()?,
        created_at: timestamp(raw.10)?,
        updated_at: timestamp(raw.11)?,
        recovery_status: raw.12,
        preview: None,
        has_completed_revert: false,
    })
}

fn query_summary(connection: &Connection, id: OperationId) -> Result<Option<OperationSummary>> {
    let summary = connection
        .query_row(
            "SELECT id, kind, origin, connection_id, connection_name, database_name,
                    collection_name, status, parent_operation_id, reverts_operation_id,
                    created_at_ms, updated_at_ms, recovery_status
             FROM operations WHERE id = ?1",
            [id.to_string()],
            raw_summary,
        )
        .optional()?
        .map(summary_from_raw)
        .transpose()?;
    summary
        .map(|mut summary| {
            summary.has_completed_revert = completed_revert_exists(connection, summary.id)?;
            Ok(summary)
        })
        .transpose()
}

fn completed_revert_exists(connection: &Connection, id: OperationId) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM operations
            WHERE reverts_operation_id = ?1 AND status = 'completed'
        )",
        [id.to_string()],
        |row| row.get(0),
    )?)
}

fn timestamp(milliseconds: i64) -> Result<chrono::DateTime<Utc>> {
    Utc.timestamp_millis_opt(milliseconds)
        .single()
        .context("operation history contains an invalid timestamp")
}

fn fixed_hash(bytes: Vec<u8>) -> Result<[u8; 32]> {
    bytes.try_into().map_err(|_| anyhow::anyhow!("operation history contains an invalid hash"))
}
