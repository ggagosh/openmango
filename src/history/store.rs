use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use chrono::{TimeZone as _, Utc};
use rusqlite::{Connection, OptionalExtension as _, params};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::crypto::{HistoryCipher, RecoveryPayload, token_hash};
use super::model::{
    BatchDetails, BatchQuery, BatchStatus, BatchSummary, GroupingKind, HistoryGap, HistoryItem,
    MAX_BATCH_ITEMS, OBSERVED_IDLE_MS, OBSERVED_MAX_MS, OperationFamily, Page, RecordedEvent,
    RestoreProgress, Usage,
};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const QUEUE_CAPACITY: usize = 128;
const PAGE_LIMIT: u32 = 100;
pub(crate) const CONNECTION_CURSOR_SCOPE: &str = "__connection__";

type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

enum Message {
    Run(Job),
    Shutdown,
}

struct Inner {
    sender: mpsc::SyncSender<Message>,
    join: Mutex<Option<thread::JoinHandle<()>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.sender.send(Message::Shutdown);
        if let Some(join) = self.join.lock().ok().and_then(|mut join| join.take()) {
            let _ = join.join();
        }
    }
}

#[derive(Clone)]
pub(crate) struct HistoryStore {
    inner: Arc<Inner>,
    cipher: Arc<HistoryCipher>,
}

#[derive(Debug, Clone)]
pub(crate) struct RestoreItem {
    pub id: Uuid,
    pub database: String,
    pub collection: String,
    pub family: OperationFamily,
    pub document_key: mongodb::bson::Document,
    pub before: Option<mongodb::bson::Document>,
    pub after: Option<mongodb::bson::Document>,
}

impl HistoryStore {
    pub(crate) fn open(path: PathBuf, key: [u8; 32]) -> Result<Self> {
        prepare_parent(&path)?;
        let cipher = Arc::new(HistoryCipher::new(key)?);
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker_path = path.clone();
        let join =
            thread::Builder::new().name("openmango-history-store".into()).spawn(move || {
                match open_connection(&worker_path) {
                    Ok(mut connection) => {
                        let _ = ready_sender.send(Ok(()));
                        while let Ok(message) = receiver.recv() {
                            match message {
                                Message::Run(job) => job(&mut connection),
                                Message::Shutdown => break,
                            }
                        }
                        let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
                    }
                    Err(error) => {
                        let _ = ready_sender.send(Err(error));
                    }
                }
            })?;
        ready_receiver.recv().context("History store stopped during startup")??;
        Ok(Self { inner: Arc::new(Inner { sender, join: Mutex::new(Some(join)) }), cipher })
    }

    fn call<T, F>(&self, work: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.inner
            .sender
            .send(Message::Run(Box::new(move |connection| {
                let _ = sender.send(work(connection));
            })))
            .map_err(|_| anyhow::anyhow!("History store is unavailable"))?;
        receiver.recv().context("History store stopped")?
    }

    pub(crate) fn record_event(&self, event: RecordedEvent) -> Result<Option<Uuid>> {
        let item_id = Uuid::new_v4();
        let token_hash = token_hash(&event.resume_token);
        let encrypted_payload = self.cipher.encrypt_item(
            item_id,
            event.connection_id,
            &event.database,
            &event.collection,
            event.family.as_str(),
            &token_hash,
            &RecoveryPayload {
                document_key: event.document_key.clone(),
                before: event.before.clone(),
                after: event.after.clone(),
            },
        )?;
        let encrypted_cursor = self.cipher.encrypt_cursor(
            event.connection_id,
            CONNECTION_CURSOR_SCOPE,
            &event.resume_token,
        )?;
        let bytes = encrypted_payload.len() as u64;
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let duplicate: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM history_items WHERE resume_token_hash = ?1)",
                [token_hash.as_slice()],
                |row| row.get(0),
            )?;
            if duplicate {
                upsert_cursor(
                    &transaction,
                    event.connection_id,
                    CONNECTION_CURSOR_SCOPE,
                    encrypted_cursor,
                    &token_hash,
                    event.cluster_time.as_deref(),
                    event.wall_time.timestamp_millis(),
                )?;
                transaction.commit()?;
                return Ok(None);
            }
            let grouping = if event.transaction_key.is_some() {
                GroupingKind::Transaction
            } else if event.trace_id.is_some() {
                GroupingKind::Attributed
            } else {
                GroupingKind::Observed
            };
            let transaction_hash =
                event.transaction_key.as_ref().map(|value| <[u8; 32]>::from(Sha256::digest(value)));
            let batch_id =
                find_open_batch(&transaction, &event, grouping, transaction_hash.as_ref())?
                    .unwrap_or_else(Uuid::new_v4);
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM history_batches WHERE id = ?1)",
                [batch_id.to_string()],
                |row| row.get(0),
            )?;
            if !exists {
                transaction.execute(
                    "INSERT INTO history_batches (
                        id, connection_id, database_name, collection_name, family, grouping_kind,
                        trace_id, transaction_key_hash, first_cluster_time, last_cluster_time,
                        first_wall_time_ms, last_wall_time_ms, item_count, revertible_count,
                        conflict_count, encrypted_bytes, status, restored_count, skipped_count,
                        failed_count, created_at_ms, updated_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, ?10,
                               0, 0, 0, 0, 'open', 0, 0, 0, ?10, ?10)",
                    params![
                        batch_id.to_string(),
                        event.connection_id.to_string(),
                        event.database,
                        event.collection,
                        event.family.as_str(),
                        grouping.as_str(),
                        event.trace_id.map(|id| id.to_string()),
                        transaction_hash.as_ref().map(|hash| hash.as_slice()),
                        event.cluster_time,
                        event.wall_time.timestamp_millis(),
                    ],
                )?;
            }
            let ordinal: u64 = transaction.query_row(
                "SELECT item_count FROM history_batches WHERE id = ?1",
                [batch_id.to_string()],
                |row| row.get(0),
            )?;
            let revertible = match event.family {
                OperationFamily::Update | OperationFamily::Replace => {
                    event.before.is_some() && event.after.is_some()
                }
                OperationFamily::Delete => event.before.is_some(),
            };
            transaction.execute(
                "INSERT INTO history_items (
                    id, batch_id, ordinal, resume_token_hash, encrypted_payload,
                    encrypted_bytes, revertible, restore_outcome, error_code
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL)",
                params![
                    item_id.to_string(),
                    batch_id.to_string(),
                    ordinal,
                    token_hash.as_slice(),
                    encrypted_payload,
                    bytes,
                    revertible,
                ],
            )?;
            transaction.execute(
                "UPDATE history_batches SET
                    last_cluster_time = ?2, last_wall_time_ms = ?3,
                    item_count = item_count + 1,
                    revertible_count = revertible_count + ?4,
                    encrypted_bytes = encrypted_bytes + ?5,
                    updated_at_ms = ?3
                 WHERE id = ?1",
                params![
                    batch_id.to_string(),
                    event.cluster_time,
                    event.wall_time.timestamp_millis(),
                    revertible,
                    bytes,
                ],
            )?;
            upsert_cursor(
                &transaction,
                event.connection_id,
                CONNECTION_CURSOR_SCOPE,
                encrypted_cursor,
                &token_hash,
                event.cluster_time.as_deref(),
                event.wall_time.timestamp_millis(),
            )?;
            transaction.commit()?;
            Ok(Some(batch_id))
        })
    }

    pub(crate) fn advance_cursor(
        &self,
        connection_id: Uuid,
        database: String,
        token: Vec<u8>,
        cluster_time: Option<String>,
        wall_time_ms: i64,
    ) -> Result<()> {
        let hash = token_hash(&token);
        let encrypted = self.cipher.encrypt_cursor(connection_id, &database, &token)?;
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            upsert_cursor(
                &transaction,
                connection_id,
                &database,
                encrypted,
                &hash,
                cluster_time.as_deref(),
                wall_time_ms,
            )?;
            transaction.commit()?;
            Ok(())
        })
    }

    pub(crate) fn load_cursor(
        &self,
        connection_id: Uuid,
        database: &str,
    ) -> Result<Option<Vec<u8>>> {
        let database = database.to_string();
        let query_database = database.clone();
        let encrypted = self.call(move |connection| {
            connection
                .query_row(
                    "SELECT encrypted_resume_token FROM history_cursors
                     WHERE connection_id = ?1 AND database_name = ?2",
                    params![connection_id.to_string(), query_database],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .optional()
                .map_err(Into::into)
        })?;
        encrypted
            .map(|value| self.cipher.decrypt_cursor(connection_id, database.as_str(), &value))
            .transpose()
    }

    #[cfg(test)]
    pub(crate) fn clear_cursor(&self, connection_id: Uuid, database: &str) -> Result<()> {
        let database = database.to_string();
        self.call(move |connection| {
            connection.execute(
                "DELETE FROM history_cursors WHERE connection_id = ?1 AND database_name = ?2",
                params![connection_id.to_string(), database],
            )?;
            Ok(())
        })
    }

    pub(crate) fn has_items_for_connection(&self, connection_id: Uuid) -> Result<bool> {
        self.call(move |connection| {
            Ok(connection.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM history_batches WHERE connection_id = ?1
                 )",
                params![connection_id.to_string()],
                |row| row.get(0),
            )?)
        })
    }

    pub(crate) fn record_gap(
        &self,
        connection_id: Uuid,
        database: Option<String>,
        collection: Option<String>,
        kind: &str,
        reason: &str,
    ) -> Result<Uuid> {
        let kind = sanitize(kind, 64);
        let reason = sanitize(reason, 500);
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let id = insert_gap(
                &transaction,
                connection_id,
                database.as_deref(),
                collection.as_deref(),
                &kind,
                &reason,
            )?;
            transaction.commit()?;
            Ok(id)
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_gap_and_advance_cursor(
        &self,
        connection_id: Uuid,
        database: Option<String>,
        collection: Option<String>,
        kind: &str,
        reason: &str,
        token: Vec<u8>,
        cluster_time: Option<String>,
        wall_time_ms: i64,
    ) -> Result<Uuid> {
        let kind = sanitize(kind, 64);
        let reason = sanitize(reason, 500);
        let token_hash = token_hash(&token);
        let encrypted =
            self.cipher.encrypt_cursor(connection_id, CONNECTION_CURSOR_SCOPE, &token)?;
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let id = insert_gap(
                &transaction,
                connection_id,
                database.as_deref(),
                collection.as_deref(),
                &kind,
                &reason,
            )?;
            upsert_cursor(
                &transaction,
                connection_id,
                CONNECTION_CURSOR_SCOPE,
                encrypted,
                &token_hash,
                cluster_time.as_deref(),
                wall_time_ms,
            )?;
            transaction.commit()?;
            Ok(id)
        })
    }

    pub(crate) fn record_gap_and_clear_cursor(
        &self,
        connection_id: Uuid,
        database: Option<String>,
        kind: &str,
        reason: &str,
    ) -> Result<Uuid> {
        let kind = sanitize(kind, 64);
        let reason = sanitize(reason, 500);
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let id =
                insert_gap(&transaction, connection_id, database.as_deref(), None, &kind, &reason)?;
            transaction.execute(
                "DELETE FROM history_cursors WHERE connection_id = ?1 AND database_name = ?2",
                params![connection_id.to_string(), CONNECTION_CURSOR_SCOPE],
            )?;
            transaction.commit()?;
            Ok(id)
        })
    }

    pub(crate) fn list_batches(&self, query: BatchQuery) -> Result<Page<BatchSummary>> {
        self.call(move |connection| list_batches(connection, query))
    }

    pub(crate) fn get_batch_summary(&self, batch_id: Uuid) -> Result<BatchSummary> {
        self.call(move |connection| {
            query_batch(connection, batch_id)?.context("History batch not found")
        })
    }

    pub(crate) fn list_gaps(
        &self,
        connection_id: Uuid,
        database: Option<&str>,
        collection: Option<&str>,
    ) -> Result<Vec<HistoryGap>> {
        let database = database.map(str::to_string);
        let collection = collection.map(str::to_string);
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT id, connection_id, database_name, collection_name, kind, reason,
                        created_at_ms, resolved
                 FROM history_gaps
                 WHERE connection_id = ?1
                   AND (?2 IS NULL OR database_name IS NULL OR database_name = ?2)
                   AND (?3 IS NULL OR collection_name IS NULL OR collection_name = ?3)
                 ORDER BY created_at_ms DESC LIMIT 100",
            )?;
            let rows = statement.query_map(
                params![connection_id.to_string(), database, collection],
                gap_from_row,
            )?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub(crate) fn get_batch(
        &self,
        batch_id: Uuid,
        offset: u32,
        limit: u32,
    ) -> Result<BatchDetails> {
        let limit = limit.clamp(1, PAGE_LIMIT);
        let cipher = self.cipher.clone();
        self.call(move |connection| {
            let summary = query_batch(connection, batch_id)?.context("History batch not found")?;
            let mut statement = connection.prepare(
                "SELECT id, resume_token_hash, encrypted_payload, restore_outcome
                 FROM history_items WHERE batch_id = ?1 ORDER BY ordinal LIMIT ?2 OFFSET ?3",
            )?;
            let rows =
                statement.query_map(params![batch_id.to_string(), limit + 1, offset], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?;
            let mut raw = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            let has_more = raw.len() > limit as usize;
            raw.truncate(limit as usize);
            let mut items = Vec::with_capacity(raw.len());
            for (id, hash, encrypted, outcome) in raw {
                let id = Uuid::parse_str(&id)?;
                let payload = cipher.decrypt_item(
                    id,
                    summary.connection_id,
                    &summary.database,
                    &summary.collection,
                    summary.family.as_str(),
                    &hash,
                    &encrypted,
                )?;
                items.push(HistoryItem {
                    id,
                    document_key: payload.document_key,
                    before: payload.before,
                    after: payload.after,
                    outcome,
                });
            }
            Ok(BatchDetails { summary, items, next_offset: has_more.then_some(offset + limit) })
        })
    }

    pub(crate) fn restore_items_page(
        &self,
        batch_id: Uuid,
        limit: usize,
    ) -> Result<Vec<RestoreItem>> {
        let cipher = self.cipher.clone();
        self.call(move |connection| {
            let summary = query_batch(connection, batch_id)?.context("History batch not found")?;
            let mut statement = connection.prepare(
                "SELECT id, resume_token_hash, encrypted_payload
                 FROM history_items
                 WHERE batch_id = ?1 AND revertible = 1
                   AND restore_outcome IN ('pending', 'applying')
                 ORDER BY ordinal DESC LIMIT ?2",
            )?;
            let rows = statement.query_map(params![batch_id.to_string(), limit as u64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?))
            })?;
            let mut items = Vec::new();
            for row in rows {
                let (id, hash, encrypted) = row?;
                let id = Uuid::parse_str(&id)?;
                let payload = cipher.decrypt_item(
                    id,
                    summary.connection_id,
                    &summary.database,
                    &summary.collection,
                    summary.family.as_str(),
                    &hash,
                    &encrypted,
                )?;
                items.push(RestoreItem {
                    id,
                    database: summary.database.clone(),
                    collection: summary.collection.clone(),
                    family: summary.family,
                    document_key: payload.document_key,
                    before: payload.before,
                    after: payload.after,
                });
            }
            Ok(items)
        })
    }

    pub(crate) fn reconcile_interrupted_restores(&self) -> Result<()> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "UPDATE history_items SET restore_outcome = 'failed', error_code = 'interrupted'
                 WHERE restore_outcome = 'applying'",
                [],
            )?;
            let mut statement =
                transaction.prepare("SELECT id FROM history_batches WHERE status = 'restoring'")?;
            let ids = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(statement);
            for id in ids {
                let batch_id = Uuid::parse_str(&id)?;
                update_restore_counts(&transaction, batch_id)?;
                transaction.execute(
                    "UPDATE history_batches SET status = 'partially_restored', updated_at_ms = ?2
                     WHERE id = ?1",
                    params![id, Utc::now().timestamp_millis()],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
    }

    pub(crate) fn begin_restore(&self, batch_id: Uuid) -> Result<()> {
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE history_batches SET status = 'restoring', updated_at_ms = ?2
                 WHERE id = ?1 AND status != 'restoring'",
                params![batch_id.to_string(), Utc::now().timestamp_millis()],
            )?;
            if changed != 1 {
                bail!("History batch is already restoring or unavailable");
            }
            Ok(())
        })
    }

    pub(crate) fn mark_item_outcomes(
        &self,
        batch_id: Uuid,
        outcomes: Vec<(Uuid, String, Option<String>)>,
    ) -> Result<()> {
        let outcomes = outcomes
            .into_iter()
            .map(|(id, outcome, error)| (id, outcome, error.map(|value| sanitize(&value, 128))))
            .collect::<Vec<_>>();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            {
                let mut statement = transaction.prepare(
                    "UPDATE history_items SET restore_outcome = ?2, error_code = ?3 WHERE id = ?1",
                )?;
                for (id, outcome, error) in outcomes {
                    statement.execute(params![id.to_string(), outcome, error])?;
                }
            }
            update_restore_counts(&transaction, batch_id)?;
            transaction.commit()?;
            Ok(())
        })
    }

    pub(crate) fn finish_restore(
        &self,
        batch_id: Uuid,
        cancelled: bool,
    ) -> Result<RestoreProgress> {
        self.call(move |connection| {
            let progress = restore_progress(connection, batch_id)?;
            let status = if cancelled || progress.failed > 0 || progress.conflicted > 0 {
                BatchStatus::PartiallyRestored
            } else if progress.processed == progress.total {
                BatchStatus::Restored
            } else {
                BatchStatus::PartiallyRestored
            };
            connection.execute(
                "UPDATE history_batches SET status = ?2, updated_at_ms = ?3 WHERE id = ?1",
                params![batch_id.to_string(), status.as_str(), Utc::now().timestamp_millis()],
            )?;
            Ok(RestoreProgress { done: true, ..progress })
        })
    }

    pub(crate) fn restore_progress(&self, batch_id: Uuid) -> Result<RestoreProgress> {
        self.call(move |connection| restore_progress(connection, batch_id))
    }

    pub(crate) fn usage(&self, connection_id: Option<Uuid>) -> Result<Usage> {
        self.call(move |connection| {
            let id = connection_id.map(|id| id.to_string());
            Ok(connection.query_row(
                "SELECT COALESCE(SUM(encrypted_bytes), 0), COUNT(*), COALESCE(SUM(item_count), 0)
                 FROM history_batches WHERE (?1 IS NULL OR connection_id = ?1)",
                [id],
                |row| {
                    Ok(Usage {
                        encrypted_bytes: row.get(0)?,
                        batches: row.get(1)?,
                        items: row.get(2)?,
                    })
                },
            )?)
        })
    }

    pub(crate) fn apply_retention(
        &self,
        connection_id: Uuid,
        max_age_days: u32,
        max_bytes: u64,
    ) -> Result<Usage> {
        self.call(move |connection| {
            let cutoff =
                Utc::now().timestamp_millis() - i64::from(max_age_days).saturating_mul(86_400_000);
            connection.execute(
                "DELETE FROM history_batches
                 WHERE connection_id = ?1 AND status != 'restoring' AND last_wall_time_ms < ?2",
                params![connection_id.to_string(), cutoff],
            )?;
            loop {
                let usage: u64 = connection.query_row(
                    "SELECT COALESCE(SUM(encrypted_bytes), 0) FROM history_batches
                     WHERE connection_id = ?1",
                    [connection_id.to_string()],
                    |row| row.get(0),
                )?;
                if usage <= max_bytes {
                    break;
                }
                let oldest: Option<String> = connection
                    .query_row(
                        "SELECT id FROM history_batches
                         WHERE connection_id = ?1 AND status != 'restoring'
                         ORDER BY last_wall_time_ms ASC LIMIT 1",
                        [connection_id.to_string()],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(oldest) = oldest else {
                    bail!("History storage limit cannot be met while restore work is active");
                };
                connection.execute("DELETE FROM history_batches WHERE id = ?1", [oldest])?;
            }
            Ok(connection.query_row(
                "SELECT COALESCE(SUM(encrypted_bytes), 0), COUNT(*), COALESCE(SUM(item_count), 0)
                 FROM history_batches WHERE connection_id = ?1",
                [connection_id.to_string()],
                |row| {
                    Ok(Usage {
                        encrypted_bytes: row.get(0)?,
                        batches: row.get(1)?,
                        items: row.get(2)?,
                    })
                },
            )?)
        })
    }

    pub(crate) fn delete_batch(&self, batch_id: Uuid) -> Result<bool> {
        self.call(move |connection| {
            Ok(connection.execute(
                "DELETE FROM history_batches WHERE id = ?1 AND status != 'restoring'",
                [batch_id.to_string()],
            )? == 1)
        })
    }

    pub(crate) fn clear_connection(&self, connection_id: Uuid) -> Result<usize> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let id = connection_id.to_string();
            let restoring: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM history_batches WHERE connection_id = ?1 AND status = 'restoring')",
                [&id],
                |row| row.get(0),
            )?;
            if restoring {
                bail!("History cannot be cleared while restore work is active");
            }
            let deleted = transaction.execute(
                "DELETE FROM history_batches WHERE connection_id = ?1",
                [&id],
            )?;
            transaction.execute("DELETE FROM history_gaps WHERE connection_id = ?1", [&id])?;
            transaction.execute("DELETE FROM history_cursors WHERE connection_id = ?1", [&id])?;
            transaction.commit()?;
            Ok(deleted)
        })
    }

    pub(crate) fn clear_collection(
        &self,
        connection_id: Uuid,
        database: &str,
        collection: &str,
    ) -> Result<usize> {
        let database = database.to_string();
        let collection = collection.to_string();
        self.call(move |connection_db| {
            let transaction = connection_db.transaction()?;
            let id = connection_id.to_string();
            let restoring: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM history_batches
                 WHERE connection_id = ?1 AND database_name = ?2 AND collection_name = ?3
                   AND status = 'restoring')",
                params![&id, &database, &collection],
                |row| row.get(0),
            )?;
            if restoring {
                bail!("Collection History cannot be cleared while restore work is active");
            }
            let deleted = transaction.execute(
                "DELETE FROM history_batches
                 WHERE connection_id = ?1 AND database_name = ?2 AND collection_name = ?3",
                params![&id, &database, &collection],
            )?;
            transaction.execute(
                "DELETE FROM history_gaps
                 WHERE connection_id = ?1 AND database_name = ?2 AND collection_name = ?3",
                params![&id, &database, &collection],
            )?;
            transaction.commit()?;
            Ok(deleted)
        })
    }

    pub(crate) fn clear_all(&self) -> Result<usize> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let restoring: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM history_batches WHERE status = 'restoring')",
                [],
                |row| row.get(0),
            )?;
            if restoring {
                bail!("History cannot be cleared while restore work is active");
            }
            let deleted = transaction.execute("DELETE FROM history_batches", [])?;
            transaction.execute("DELETE FROM history_gaps", [])?;
            transaction.execute("DELETE FROM history_cursors", [])?;
            transaction.commit()?;
            Ok(deleted)
        })
    }

    #[cfg(test)]
    pub(crate) fn delete_cursor_for_test(&self, connection_id: Uuid, database: &str) -> Result<()> {
        self.clear_cursor(connection_id, database)
    }
}

fn find_open_batch(
    transaction: &rusqlite::Transaction<'_>,
    event: &RecordedEvent,
    grouping: GroupingKind,
    transaction_hash: Option<&[u8; 32]>,
) -> Result<Option<Uuid>> {
    let id: Option<String> = match grouping {
        GroupingKind::Transaction => transaction
            .query_row(
                "SELECT id FROM history_batches
                 WHERE connection_id = ?1 AND database_name = ?2 AND collection_name = ?3
                   AND family = ?4 AND grouping_kind = 'transaction' AND status = 'open'
                   AND transaction_key_hash = ?5 AND item_count < ?6
                 ORDER BY last_wall_time_ms DESC LIMIT 1",
                params![
                    event.connection_id.to_string(),
                    event.database,
                    event.collection,
                    event.family.as_str(),
                    transaction_hash.map(|hash| hash.as_slice()),
                    MAX_BATCH_ITEMS,
                ],
                |row| row.get(0),
            )
            .optional()?,
        GroupingKind::Attributed => transaction
            .query_row(
                "SELECT id FROM history_batches
                 WHERE connection_id = ?1 AND database_name = ?2 AND collection_name = ?3
                   AND family = ?4 AND grouping_kind = 'attributed' AND status = 'open'
                   AND trace_id = ?5 AND item_count < ?6
                 ORDER BY last_wall_time_ms DESC LIMIT 1",
                params![
                    event.connection_id.to_string(),
                    event.database,
                    event.collection,
                    event.family.as_str(),
                    event.trace_id.map(|id| id.to_string()),
                    MAX_BATCH_ITEMS,
                ],
                |row| row.get(0),
            )
            .optional()?,
        GroupingKind::Observed => transaction
            .query_row(
                "SELECT id FROM history_batches
                 WHERE connection_id = ?1 AND database_name = ?2 AND collection_name = ?3
                   AND family = ?4 AND grouping_kind = 'observed' AND status = 'open'
                   AND ?5 - last_wall_time_ms <= ?6
                   AND ?5 - first_wall_time_ms <= ?7
                   AND item_count < ?8
                 ORDER BY last_wall_time_ms DESC LIMIT 1",
                params![
                    event.connection_id.to_string(),
                    event.database,
                    event.collection,
                    event.family.as_str(),
                    event.wall_time.timestamp_millis(),
                    OBSERVED_IDLE_MS,
                    OBSERVED_MAX_MS,
                    MAX_BATCH_ITEMS,
                ],
                |row| row.get(0),
            )
            .optional()?,
    };
    id.map(|value| Uuid::parse_str(&value).map_err(Into::into)).transpose()
}

fn insert_gap(
    transaction: &rusqlite::Transaction<'_>,
    connection_id: Uuid,
    database: Option<&str>,
    collection: Option<&str>,
    kind: &str,
    reason: &str,
) -> Result<Uuid> {
    if let Some(existing) = transaction
        .query_row(
            "SELECT id FROM history_gaps
             WHERE connection_id = ?1
               AND database_name IS ?2
               AND collection_name IS ?3
               AND kind = ?4 AND reason = ?5 AND resolved = 0
             ORDER BY created_at_ms DESC LIMIT 1",
            params![connection_id.to_string(), database, collection, kind, reason],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(Uuid::parse_str(&existing)?);
    }
    let id = Uuid::new_v4();
    transaction.execute(
        "INSERT INTO history_gaps (
            id, connection_id, database_name, collection_name, kind, reason,
            start_cluster_time, end_cluster_time, created_at_ms, resolved
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, ?7, 0)",
        params![
            id.to_string(),
            connection_id.to_string(),
            database,
            collection,
            kind,
            reason,
            Utc::now().timestamp_millis(),
        ],
    )?;
    Ok(id)
}

fn upsert_cursor(
    transaction: &rusqlite::Transaction<'_>,
    connection_id: Uuid,
    database: &str,
    encrypted: Vec<u8>,
    token_hash: &[u8],
    cluster_time: Option<&str>,
    wall_time_ms: i64,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO history_cursors (
            connection_id, database_name, encrypted_resume_token, token_hash,
            last_cluster_time, last_wall_time_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(connection_id, database_name) DO UPDATE SET
            encrypted_resume_token = excluded.encrypted_resume_token,
            token_hash = excluded.token_hash,
            last_cluster_time = excluded.last_cluster_time,
            last_wall_time_ms = excluded.last_wall_time_ms",
        params![
            connection_id.to_string(),
            database,
            encrypted,
            token_hash,
            cluster_time,
            wall_time_ms,
        ],
    )?;
    Ok(())
}

fn list_batches(connection: &Connection, query: BatchQuery) -> Result<Page<BatchSummary>> {
    connection.execute(
        "UPDATE history_batches SET status = 'closed', updated_at_ms = ?1
         WHERE status = 'open' AND last_wall_time_ms <= ?2",
        params![Utc::now().timestamp_millis(), Utc::now().timestamp_millis() - OBSERVED_IDLE_MS],
    )?;
    let limit = query.limit.clamp(1, PAGE_LIMIT);
    let database = query.database;
    let collection = query.collection;
    let total: u64 = connection.query_row(
        "SELECT COUNT(*) FROM history_batches
         WHERE connection_id = ?1
           AND (?2 IS NULL OR database_name = ?2)
           AND (?3 IS NULL OR collection_name = ?3)",
        params![query.connection_id.to_string(), database, collection],
        |row| row.get(0),
    )?;
    let mut statement = connection.prepare(
        "SELECT id, connection_id, database_name, collection_name, family, grouping_kind,
                trace_id, first_wall_time_ms, last_wall_time_ms, item_count, revertible_count,
                conflict_count, encrypted_bytes, status, restored_count, skipped_count, failed_count
         FROM history_batches
         WHERE connection_id = ?1
           AND (?2 IS NULL OR database_name = ?2)
           AND (?3 IS NULL OR collection_name = ?3)
         ORDER BY last_wall_time_ms DESC LIMIT ?4 OFFSET ?5",
    )?;
    let rows = statement.query_map(
        params![query.connection_id.to_string(), database, collection, limit, query.offset,],
        batch_from_row,
    )?;
    let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let next = (u64::from(query.offset) + (items.len() as u64) < total)
        .then_some(query.offset + items.len() as u32);
    Ok(Page { items, total, next_offset: next })
}

fn query_batch(connection: &Connection, id: Uuid) -> Result<Option<BatchSummary>> {
    connection
        .query_row(
            "SELECT id, connection_id, database_name, collection_name, family, grouping_kind,
                    trace_id, first_wall_time_ms, last_wall_time_ms, item_count, revertible_count,
                    conflict_count, encrypted_bytes, status, restored_count, skipped_count, failed_count
             FROM history_batches WHERE id = ?1",
            [id.to_string()],
            batch_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn batch_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BatchSummary> {
    let parse_uuid = |value: String| {
        Uuid::parse_str(&value).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                value.len(),
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    };
    let family: String = row.get(4)?;
    let grouping: String = row.get(5)?;
    let status: String = row.get(13)?;
    Ok(BatchSummary {
        id: parse_uuid(row.get(0)?)?,
        connection_id: parse_uuid(row.get(1)?)?,
        database: row.get(2)?,
        collection: row.get(3)?,
        family: OperationFamily::parse(&family).unwrap_or(OperationFamily::Update),
        grouping: GroupingKind::parse(&grouping).unwrap_or(GroupingKind::Observed),
        trace_id: row.get::<_, Option<String>>(6)?.and_then(|value| Uuid::parse_str(&value).ok()),
        first_wall_time: Utc.timestamp_millis_opt(row.get(7)?).single().unwrap_or_else(Utc::now),
        last_wall_time: Utc.timestamp_millis_opt(row.get(8)?).single().unwrap_or_else(Utc::now),
        item_count: row.get(9)?,
        revertible_count: row.get(10)?,
        conflict_count: row.get(11)?,
        encrypted_bytes: row.get(12)?,
        status: BatchStatus::parse(&status).unwrap_or(BatchStatus::Failed),
        restored_count: row.get(14)?,
        skipped_count: row.get(15)?,
        failed_count: row.get(16)?,
    })
}

fn gap_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryGap> {
    let id: String = row.get(0)?;
    let connection_id: String = row.get(1)?;
    Ok(HistoryGap {
        id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
        connection_id: Uuid::parse_str(&connection_id).unwrap_or_else(|_| Uuid::nil()),
        database: row.get(2)?,
        collection: row.get(3)?,
        kind: row.get(4)?,
        reason: row.get(5)?,
        created_at: Utc.timestamp_millis_opt(row.get(6)?).single().unwrap_or_else(Utc::now),
        resolved: row.get(7)?,
    })
}

fn update_restore_counts(transaction: &rusqlite::Transaction<'_>, batch_id: Uuid) -> Result<()> {
    transaction.execute(
        "UPDATE history_batches SET
            restored_count = (SELECT COUNT(*) FROM history_items WHERE batch_id = ?1 AND restore_outcome = 'restored'),
            skipped_count = (SELECT COUNT(*) FROM history_items WHERE batch_id = ?1 AND restore_outcome = 'skipped'),
            conflict_count = (SELECT COUNT(*) FROM history_items WHERE batch_id = ?1 AND restore_outcome = 'conflicted'),
            failed_count = (SELECT COUNT(*) FROM history_items WHERE batch_id = ?1 AND restore_outcome = 'failed'),
            updated_at_ms = ?2
         WHERE id = ?1",
        params![batch_id.to_string(), Utc::now().timestamp_millis()],
    )?;
    Ok(())
}

fn restore_progress(connection: &Connection, batch_id: Uuid) -> Result<RestoreProgress> {
    let summary = query_batch(connection, batch_id)?.context("History batch not found")?;
    let processed = summary.restored_count
        + summary.skipped_count
        + summary.conflict_count
        + summary.failed_count;
    Ok(RestoreProgress {
        total: summary.revertible_count,
        processed,
        restored: summary.restored_count,
        skipped: summary.skipped_count,
        conflicted: summary.conflict_count,
        failed: summary.failed_count,
        done: summary.status != BatchStatus::Restoring,
    })
}

fn prepare_parent(path: &Path) -> Result<()> {
    let parent = path.parent().context("History path has no parent")?;
    std::fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn open_connection(path: &Path) -> Result<Connection> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS history_batches (
            id TEXT PRIMARY KEY,
            connection_id TEXT NOT NULL,
            database_name TEXT NOT NULL,
            collection_name TEXT NOT NULL,
            family TEXT NOT NULL,
            grouping_kind TEXT NOT NULL,
            trace_id TEXT,
            transaction_key_hash BLOB,
            first_cluster_time TEXT,
            last_cluster_time TEXT,
            first_wall_time_ms INTEGER NOT NULL,
            last_wall_time_ms INTEGER NOT NULL,
            item_count INTEGER NOT NULL,
            revertible_count INTEGER NOT NULL,
            conflict_count INTEGER NOT NULL,
            encrypted_bytes INTEGER NOT NULL,
            status TEXT NOT NULL,
            restored_count INTEGER NOT NULL,
            skipped_count INTEGER NOT NULL,
            failed_count INTEGER NOT NULL,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS history_batches_scope
            ON history_batches(connection_id, database_name, collection_name, last_wall_time_ms DESC);
         CREATE TABLE IF NOT EXISTS history_items (
            id TEXT PRIMARY KEY,
            batch_id TEXT NOT NULL REFERENCES history_batches(id) ON DELETE CASCADE,
            ordinal INTEGER NOT NULL,
            resume_token_hash BLOB NOT NULL UNIQUE,
            encrypted_payload BLOB NOT NULL,
            encrypted_bytes INTEGER NOT NULL,
            revertible INTEGER NOT NULL,
            restore_outcome TEXT NOT NULL,
            error_code TEXT
         );
         CREATE INDEX IF NOT EXISTS history_items_batch ON history_items(batch_id, ordinal);
         CREATE TABLE IF NOT EXISTS history_cursors (
            connection_id TEXT NOT NULL,
            database_name TEXT NOT NULL,
            encrypted_resume_token BLOB NOT NULL,
            token_hash BLOB NOT NULL,
            last_cluster_time TEXT,
            last_wall_time_ms INTEGER NOT NULL,
            PRIMARY KEY(connection_id, database_name)
         );
         CREATE TABLE IF NOT EXISTS history_gaps (
            id TEXT PRIMARY KEY,
            connection_id TEXT NOT NULL,
            database_name TEXT,
            collection_name TEXT,
            kind TEXT NOT NULL,
            reason TEXT NOT NULL,
            start_cluster_time TEXT,
            end_cluster_time TEXT,
            created_at_ms INTEGER NOT NULL,
            resolved INTEGER NOT NULL
         );
         PRAGMA user_version = 1;",
    )?;
    Ok(connection)
}

fn sanitize(value: &str, max: usize) -> String {
    value.chars().filter(|character| !character.is_control()).take(max).collect()
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use mongodb::bson::doc;

    use super::*;

    fn event(connection_id: Uuid, index: u64, wall_time: chrono::DateTime<Utc>) -> RecordedEvent {
        RecordedEvent {
            connection_id,
            database: "app".into(),
            collection: "items".into(),
            family: OperationFamily::Update,
            document_key: doc! { "_id": index as i64 },
            before: Some(doc! { "_id": index as i64, "value": 0 }),
            after: Some(doc! { "_id": index as i64, "value": 1 }),
            resume_token: index.to_be_bytes().to_vec(),
            cluster_time: Some(index.to_string()),
            wall_time,
            transaction_key: None,
            trace_id: None,
        }
    }

    fn store() -> (tempfile::TempDir, HistoryStore) {
        let directory = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(directory.path().join("history.sqlite3"), [8; 32]).unwrap();
        (directory, store)
    }

    #[test]
    fn event_and_cursor_are_atomic_and_resume_tokens_deduplicate() {
        let (_directory, store) = store();
        let connection_id = Uuid::new_v4();
        let event = event(connection_id, 1, Utc::now());
        assert!(store.record_event(event.clone()).unwrap().is_some());
        assert!(store.record_event(event).unwrap().is_none());
        assert!(store.load_cursor(connection_id, CONNECTION_CURSOR_SCOPE).unwrap().is_some());
        let page = store
            .list_batches(BatchQuery {
                connection_id,
                database: None,
                collection: None,
                offset: 0,
                limit: 10,
            })
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].item_count, 1);
    }

    #[test]
    fn transaction_attributed_and_observed_grouping_are_honest() {
        let (_directory, store) = store();
        let connection_id = Uuid::new_v4();
        let now = Utc::now();
        let mut transaction = event(connection_id, 1, now);
        transaction.transaction_key = Some(vec![1, 2, 3]);
        let first = store.record_event(transaction).unwrap().unwrap();
        let mut same = event(connection_id, 2, now + Duration::milliseconds(10));
        same.transaction_key = Some(vec![1, 2, 3]);
        assert_eq!(store.record_event(same).unwrap(), Some(first));

        let trace_id = Uuid::new_v4();
        let mut attributed = event(connection_id, 3, now);
        attributed.trace_id = Some(trace_id);
        let attributed_batch = store.record_event(attributed).unwrap().unwrap();
        assert_ne!(attributed_batch, first);

        let observed = store.record_event(event(connection_id, 4, now)).unwrap().unwrap();
        assert_eq!(
            store.record_event(event(connection_id, 5, now + Duration::milliseconds(100))).unwrap(),
            Some(observed)
        );
        assert_ne!(
            store.record_event(event(connection_id, 6, now + Duration::seconds(2))).unwrap(),
            Some(observed)
        );
    }

    #[test]
    fn restore_pages_are_newest_first_for_repeated_document_updates() {
        let (_directory, store) = store();
        let connection_id = Uuid::new_v4();
        let now = Utc::now();
        let mut first = event(connection_id, 1, now);
        first.document_key = doc! { "_id": 1 };
        first.before = Some(doc! { "_id": 1, "value": "A" });
        first.after = Some(doc! { "_id": 1, "value": "B" });
        let batch = store.record_event(first).unwrap().unwrap();
        let mut second = event(connection_id, 2, now + Duration::milliseconds(10));
        second.document_key = doc! { "_id": 1 };
        second.before = Some(doc! { "_id": 1, "value": "B" });
        second.after = Some(doc! { "_id": 1, "value": "C" });
        assert_eq!(store.record_event(second).unwrap(), Some(batch));

        let items = store.restore_items_page(batch, 10).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].after.as_ref().unwrap().get_str("value"), Ok("C"));
        assert_eq!(items[1].after.as_ref().unwrap().get_str("value"), Ok("B"));
    }

    #[test]
    fn closed_batches_are_never_reopened() {
        let (_directory, store) = store();
        let connection_id = Uuid::new_v4();
        let now = Utc::now() - Duration::seconds(2);
        let first = store.record_event(event(connection_id, 1, now)).unwrap().unwrap();
        let _ = store
            .list_batches(BatchQuery {
                connection_id,
                database: None,
                collection: None,
                offset: 0,
                limit: 10,
            })
            .unwrap();
        let second = store.record_event(event(connection_id, 2, now)).unwrap().unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn ten_thousand_events_produce_one_observed_batch() {
        let (_directory, store) = store();
        let connection_id = Uuid::new_v4();
        let now = Utc::now();
        for index in 0..10_000 {
            store.record_event(event(connection_id, index, now)).unwrap();
        }
        let page = store
            .list_batches(BatchQuery {
                connection_id,
                database: None,
                collection: None,
                offset: 0,
                limit: 10,
            })
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].item_count, 10_000);
    }

    #[test]
    fn gap_and_cursor_advancement_are_atomic_at_the_store_boundary() {
        let (_directory, store) = store();
        let connection_id = Uuid::new_v4();
        store
            .record_gap_and_advance_cursor(
                connection_id,
                Some("app".into()),
                Some("items".into()),
                "missing_pre_post_image",
                "exact image unavailable",
                vec![9, 8, 7],
                Some("1:1".into()),
                Utc::now().timestamp_millis(),
            )
            .unwrap();
        assert_eq!(
            store.load_cursor(connection_id, CONNECTION_CURSOR_SCOPE).unwrap(),
            Some(vec![9, 8, 7])
        );
        let gaps = store.list_gaps(connection_id, Some("app"), Some("items")).unwrap();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].kind, "missing_pre_post_image");
    }

    #[test]
    fn expired_resume_token_gap_and_cursor_abandonment_are_atomic() {
        let (_directory, store) = store();
        let connection_id = Uuid::new_v4();
        store.record_event(event(connection_id, 1, Utc::now())).unwrap();
        assert!(store.load_cursor(connection_id, CONNECTION_CURSOR_SCOPE).unwrap().is_some());

        store
            .record_gap_and_clear_cursor(
                connection_id,
                Some("app".into()),
                "resume_token_expired",
                "MongoDB rejected the stored resume point",
            )
            .unwrap();

        assert!(store.load_cursor(connection_id, CONNECTION_CURSOR_SCOPE).unwrap().is_none());
        let gaps = store.list_gaps(connection_id, Some("app"), None).unwrap();
        assert!(gaps.iter().any(|gap| gap.kind == "resume_token_expired"));
    }

    #[test]
    fn missing_cursor_gap_is_durable_and_deduplicated() {
        let (_directory, store) = store();
        let connection_id = Uuid::new_v4();
        store.record_event(event(connection_id, 1, Utc::now())).unwrap();
        store.delete_cursor_for_test(connection_id, CONNECTION_CURSOR_SCOPE).unwrap();
        assert!(store.load_cursor(connection_id, CONNECTION_CURSOR_SCOPE).unwrap().is_none());
        let first = store
            .record_gap(
                connection_id,
                Some("app".into()),
                None,
                "missing_resume_token",
                "resume token is missing",
            )
            .unwrap();
        let duplicate = store
            .record_gap(
                connection_id,
                Some("app".into()),
                None,
                "missing_resume_token",
                "resume token is missing",
            )
            .unwrap();
        assert_eq!(first, duplicate);
        assert_eq!(store.list_gaps(connection_id, Some("app"), None).unwrap().len(), 1);
    }

    #[test]
    fn partial_restore_outcomes_remain_honest() {
        let (_directory, store) = store();
        let connection_id = Uuid::new_v4();
        let now = Utc::now();
        let batch = store.record_event(event(connection_id, 1, now)).unwrap().unwrap();
        store.record_event(event(connection_id, 2, now)).unwrap();
        let details = store.get_batch(batch, 0, 10).unwrap();
        store.begin_restore(batch).unwrap();
        store
            .mark_item_outcomes(
                batch,
                vec![
                    (details.items[0].id, "restored".into(), None),
                    (
                        details.items[1].id,
                        "conflicted".into(),
                        Some("current_document_mismatch".into()),
                    ),
                ],
            )
            .unwrap();
        let progress = store.finish_restore(batch, false).unwrap();
        assert_eq!(progress.restored, 1);
        assert_eq!(progress.conflicted, 1);
        assert_eq!(progress.processed, 2);
        let summary = store.get_batch(batch, 0, 10).unwrap().summary;
        assert_eq!(summary.status, BatchStatus::PartiallyRestored);
        assert!(!summary.can_restore(), "fully processed conflicts cannot be retried");
    }

    #[test]
    fn cancelled_restore_keeps_pending_items_resumable() {
        let (_directory, store) = store();
        let connection_id = Uuid::new_v4();
        let now = Utc::now();
        let batch = store.record_event(event(connection_id, 1, now)).unwrap().unwrap();
        store.record_event(event(connection_id, 2, now)).unwrap();
        let details = store.get_batch(batch, 0, 10).unwrap();
        store.begin_restore(batch).unwrap();
        store
            .mark_item_outcomes(batch, vec![(details.items[0].id, "restored".into(), None)])
            .unwrap();
        store.finish_restore(batch, true).unwrap();

        let summary = store.get_batch(batch, 0, 10).unwrap().summary;
        assert_eq!(summary.status, BatchStatus::PartiallyRestored);
        assert_eq!(summary.pending_restore_count(), 1);
        assert!(summary.can_restore());
    }

    #[test]
    fn retention_byte_limit_purges_oldest_eligible_batch_first() {
        let (_directory, store) = store();
        let connection_id = Uuid::new_v4();
        let now = Utc::now();
        let oldest = store
            .record_event(event(connection_id, 1, now - Duration::seconds(4)))
            .unwrap()
            .unwrap();
        let newest = store
            .record_event(event(connection_id, 2, now - Duration::seconds(2)))
            .unwrap()
            .unwrap();
        assert_ne!(oldest, newest);
        let page = store
            .list_batches(BatchQuery {
                connection_id,
                database: None,
                collection: None,
                offset: 0,
                limit: 10,
            })
            .unwrap();
        let newest_bytes =
            page.items.iter().find(|batch| batch.id == newest).unwrap().encrypted_bytes;

        let usage = store.apply_retention(connection_id, 365, newest_bytes).unwrap();
        assert!(usage.encrypted_bytes <= newest_bytes);
        let retained = store
            .list_batches(BatchQuery {
                connection_id,
                database: None,
                collection: None,
                offset: 0,
                limit: 10,
            })
            .unwrap();
        assert!(!retained.items.iter().any(|batch| batch.id == oldest));
        assert!(retained.items.iter().any(|batch| batch.id == newest));
    }

    #[test]
    fn retention_purges_oldest_by_age_and_bytes_but_not_active_restore() {
        let (_directory, store) = store();
        let connection_id = Uuid::new_v4();
        let old = Utc::now() - Duration::days(40);
        let old_batch = store.record_event(event(connection_id, 1, old)).unwrap().unwrap();
        let active = store.record_event(event(connection_id, 2, Utc::now())).unwrap().unwrap();
        store.begin_restore(active).unwrap();
        store.apply_retention(connection_id, 30, 0).unwrap_err();
        let page = store
            .list_batches(BatchQuery {
                connection_id,
                database: None,
                collection: None,
                offset: 0,
                limit: 10,
            })
            .unwrap();
        assert!(!page.items.iter().any(|batch| batch.id == old_batch));
        assert!(page.items.iter().any(|batch| batch.id == active));
    }
}
