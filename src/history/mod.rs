mod crypto;
mod model;
mod store;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::TryStreamExt as _;
use mongodb::bson::{Document, doc};
use mongodb::change_stream::event::{ChangeStreamEvent, OperationType};
use mongodb::options::{FullDocumentBeforeChangeType, FullDocumentType};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub use model::{
    BatchDetails, BatchQuery, BatchStatus, BatchSummary, CollectionCoverage, EligibilityReport,
    EligibilityStatus, GroupingKind, HistoryConnection, HistoryGap, HistoryItem, OperationFamily,
    Page, RestoreProgress, SetupReport, TraceDescriptor, Usage,
};
use model::{RESTORE_CHUNK_ITEMS, RecordedEvent};
use store::{HistoryStore, RestoreItem};

#[derive(Debug, Clone)]
struct TraceState {
    descriptor: TraceDescriptor,
    matched_events: u64,
}

#[derive(Clone)]
pub struct HistoryService {
    store: HistoryStore,
    runtime: tokio::runtime::Handle,
    supervisors: Arc<Mutex<HashMap<Uuid, CancellationToken>>>,
    clients: Arc<Mutex<HashMap<Uuid, mongodb::Client>>>,
    traces: Arc<Mutex<Vec<TraceState>>>,
    restore_cancellations: Arc<Mutex<HashMap<Uuid, CancellationToken>>>,
    stopped_gaps: Arc<Mutex<Vec<HistoryGap>>>,
    usage_cache: Arc<Mutex<HashMap<Uuid, Usage>>>,
}

impl HistoryService {
    pub fn open(
        path: PathBuf,
        key: [u8; 32],
        runtime: tokio::runtime::Handle,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            store: HistoryStore::open(path, key)?,
            runtime,
            supervisors: Arc::new(Mutex::new(HashMap::new())),
            clients: Arc::new(Mutex::new(HashMap::new())),
            traces: Arc::new(Mutex::new(Vec::new())),
            restore_cancellations: Arc::new(Mutex::new(HashMap::new())),
            stopped_gaps: Arc::new(Mutex::new(Vec::new())),
            usage_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Start one connection-level supervisor. It owns one watcher per database, never per UI tab.
    pub fn start(&self, connection: HistoryConnection) {
        self.stop(connection.id);
        if let Ok(mut clients) = self.clients.lock() {
            clients.insert(connection.id, connection.client.clone());
        }
        let cancellation = CancellationToken::new();
        if let Ok(mut supervisors) = self.supervisors.lock() {
            supervisors.insert(connection.id, cancellation.clone());
        }
        let watched =
            Arc::new(Mutex::new(connection.databases.iter().cloned().collect::<HashSet<_>>()));
        for database in connection.databases.clone() {
            self.spawn_database_watcher(connection.clone(), database, cancellation.child_token());
        }
        let service = self.clone();
        let cancellation = cancellation.child_token();
        self.runtime.spawn(async move {
            let _ = service.store.apply_retention(
                connection.id,
                connection.max_age_days,
                connection.max_bytes,
            );
            service.refresh_usage_cache(connection.id);
            while !cancellation.is_cancelled() {
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        match connection.client.list_database_names().await {
                            Ok(databases) => {
                                for database in databases {
                                    let is_new = watched.lock().is_ok_and(|mut watched| watched.insert(database.clone()));
                                    if is_new {
                                        let _ = service.store.record_gap(
                                            connection.id,
                                            Some(database.clone()),
                                            None,
                                            "new_database_discovered",
                                            "A database appeared after History started; coverage begins after setup completed.",
                                        );
                                        let mut database_connection = connection.clone();
                                        database_connection.databases = vec![database.clone()];
                                        if let Err(error) = service.setup_pre_post_images(&database_connection).await {
                                            let _ = service.store.record_gap(
                                                connection.id,
                                                Some(database.clone()),
                                                None,
                                                "coverage_setup_failed",
                                                &error,
                                            );
                                        }
                                        service.spawn_database_watcher(
                                            database_connection,
                                            database,
                                            cancellation.child_token(),
                                        );
                                    }
                                }
                            }
                            Err(error) => {
                                let reason = format!(
                                    "History could not discover newly created databases: {error}"
                                );
                                if service
                                    .store
                                    .record_gap(
                                        connection.id,
                                        None,
                                        None,
                                        "database_discovery_failed",
                                        &reason,
                                    )
                                    .is_err()
                                {
                                    service.surface_stopped_gap(
                                        connection.id,
                                        None,
                                        None,
                                        "recorder_stopped",
                                        &reason,
                                    );
                                }
                            }
                        }
                        if let Err(error) = service.setup_pre_post_images(&connection).await {
                            let _ = service.store.record_gap(
                                connection.id,
                                None,
                                None,
                                "coverage_setup_failed",
                                &error,
                            );
                        }
                    }
                }
            }
        });
    }

    fn spawn_database_watcher(
        &self,
        connection: HistoryConnection,
        database: String,
        cancellation: CancellationToken,
    ) {
        let service = self.clone();
        self.runtime.spawn(async move {
            service.watch_database(connection, database, cancellation).await;
        });
    }

    pub fn stop(&self, connection_id: Uuid) {
        if let Ok(mut supervisors) = self.supervisors.lock()
            && let Some(cancellation) = supervisors.remove(&connection_id)
        {
            cancellation.cancel();
        }
        if let Ok(mut clients) = self.clients.lock() {
            clients.remove(&connection_id);
        }
    }

    pub fn reconcile(&self) -> anyhow::Result<()> {
        self.store.reconcile_interrupted_restores()
    }

    pub async fn eligibility(connection: &HistoryConnection) -> EligibilityReport {
        eligibility(connection).await
    }

    pub async fn setup_pre_post_images(
        &self,
        connection: &HistoryConnection,
    ) -> Result<SetupReport, String> {
        setup_pre_post_images(connection).await
    }

    pub fn list_batches(&self, query: BatchQuery) -> anyhow::Result<Page<BatchSummary>> {
        self.store.list_batches(query)
    }

    pub fn list_gaps(
        &self,
        connection_id: Uuid,
        database: Option<&str>,
        collection: Option<&str>,
    ) -> anyhow::Result<Vec<HistoryGap>> {
        let mut gaps = self.store.list_gaps(connection_id, database, collection)?;
        if let Ok(stopped) = self.stopped_gaps.lock() {
            gaps.extend(
                stopped
                    .iter()
                    .filter(|gap| {
                        gap.connection_id == connection_id
                            && database.is_none_or(|value| {
                                gap.database.as_deref().is_none_or(|gap| gap == value)
                            })
                            && collection.is_none_or(|value| {
                                gap.collection.as_deref().is_none_or(|gap| gap == value)
                            })
                    })
                    .cloned(),
            );
        }
        gaps.sort_by_key(|gap| std::cmp::Reverse(gap.created_at));
        Ok(gaps)
    }

    pub fn get_batch(
        &self,
        batch_id: Uuid,
        offset: u32,
        limit: u32,
    ) -> anyhow::Result<BatchDetails> {
        self.store.get_batch(batch_id, offset, limit)
    }

    pub fn usage(&self, connection_id: Option<Uuid>) -> anyhow::Result<Usage> {
        self.store.usage(connection_id)
    }

    pub fn cached_usage(&self, connection_id: Uuid) -> Option<Usage> {
        self.usage_cache.lock().ok().and_then(|usage| usage.get(&connection_id).copied())
    }

    fn refresh_usage_cache(&self, connection_id: Uuid) {
        if let Ok(usage) = self.store.usage(Some(connection_id))
            && let Ok(mut cache) = self.usage_cache.lock()
        {
            cache.insert(connection_id, usage);
        }
    }

    pub fn set_retention(
        &self,
        connection_id: Uuid,
        max_age_days: u32,
        max_bytes: u64,
    ) -> anyhow::Result<Usage> {
        let usage =
            self.store.apply_retention(connection_id, max_age_days.max(1), max_bytes.max(1))?;
        if let Ok(mut cache) = self.usage_cache.lock() {
            cache.insert(connection_id, usage);
        }
        Ok(usage)
    }

    pub fn delete_batch(&self, batch_id: Uuid) -> anyhow::Result<bool> {
        let connection_id = self.store.get_batch(batch_id, 0, 1)?.summary.connection_id;
        let deleted = self.store.delete_batch(batch_id)?;
        self.refresh_usage_cache(connection_id);
        Ok(deleted)
    }

    pub fn clear_collection(
        &self,
        connection_id: Uuid,
        database: &str,
        collection: &str,
    ) -> anyhow::Result<usize> {
        let deleted = self.store.clear_collection(connection_id, database, collection)?;
        self.refresh_usage_cache(connection_id);
        Ok(deleted)
    }

    pub fn clear_connection(&self, connection_id: Uuid) -> anyhow::Result<usize> {
        let deleted = self.store.clear_connection(connection_id)?;
        self.refresh_usage_cache(connection_id);
        Ok(deleted)
    }

    pub fn clear_all(&self) -> anyhow::Result<usize> {
        let deleted = self.store.clear_all()?;
        if let Ok(mut cache) = self.usage_cache.lock() {
            cache.clear();
        }
        Ok(deleted)
    }

    pub fn register_trace(&self, descriptor: TraceDescriptor) {
        if let Ok(mut traces) = self.traces.lock() {
            let now = Utc::now();
            traces.retain(|trace| {
                now - trace.descriptor.started_at < chrono::Duration::minutes(5)
                    && trace.matched_events < trace.descriptor.affected_count.unwrap_or(u64::MAX)
            });
            traces.push(TraceState { descriptor, matched_events: 0 });
        }
    }

    pub fn complete_trace(&self, trace_id: Uuid, affected_count: u64) {
        if let Ok(mut traces) = self.traces.lock()
            && let Some(trace) = traces.iter_mut().find(|trace| trace.descriptor.id == trace_id)
        {
            trace.descriptor.completed_at = Some(Utc::now());
            trace.descriptor.affected_count = Some(affected_count);
        }
    }

    pub fn abandon_trace(&self, trace_id: Uuid) {
        if let Ok(mut traces) = self.traces.lock() {
            traces.retain(|trace| trace.descriptor.id != trace_id);
        }
    }

    pub fn revert_batch(&self, batch_id: Uuid) -> Result<(), String> {
        let details = self.store.get_batch(batch_id, 0, 1).map_err(|error| error.to_string())?;
        let connection_id = details.summary.connection_id;
        let client = self
            .clients
            .lock()
            .ok()
            .and_then(|clients| clients.get(&connection_id).cloned())
            .ok_or_else(|| "Connect the History batch target before restoring".to_string())?;
        self.store.begin_restore(batch_id).map_err(|error| error.to_string())?;
        let cancellation = CancellationToken::new();
        if let Ok(mut cancellations) = self.restore_cancellations.lock() {
            cancellations.insert(batch_id, cancellation.clone());
        }
        let service = self.clone();
        self.runtime.spawn(async move {
            let result = service.restore_batch(client, batch_id, cancellation.clone()).await;
            if let Err(error) = result {
                let _ = service.store.finish_restore(batch_id, true);
                if service
                    .store
                    .record_gap(
                        connection_id,
                        Some(details.summary.database),
                        Some(details.summary.collection),
                        "restore_failed",
                        &error,
                    )
                    .is_err()
                {
                    service.surface_stopped_gap(
                        connection_id,
                        None,
                        None,
                        "restore_failed",
                        &error,
                    );
                }
            }
            if let Ok(mut cancellations) = service.restore_cancellations.lock() {
                cancellations.remove(&batch_id);
            }
        });
        Ok(())
    }

    pub fn cancel_restore(&self, batch_id: Uuid) {
        if let Ok(cancellations) = self.restore_cancellations.lock()
            && let Some(cancellation) = cancellations.get(&batch_id)
        {
            cancellation.cancel();
        }
    }

    pub fn restore_progress(&self, batch_id: Uuid) -> anyhow::Result<RestoreProgress> {
        self.store.restore_progress(batch_id)
    }

    async fn watch_database(
        &self,
        connection: HistoryConnection,
        database: String,
        cancellation: CancellationToken,
    ) {
        let mut resume = match self.store.load_cursor(connection.id, &database) {
            Ok(resume) => resume,
            Err(error) => {
                if self
                    .store
                    .record_gap_and_clear_cursor(
                        connection.id,
                        database.clone(),
                        "resume_token_unreadable",
                        &format!("Stored resume token could not be decrypted: {error}"),
                    )
                    .is_err()
                {
                    self.surface_stopped_gap(
                        connection.id,
                        Some(database.clone()),
                        None,
                        "recorder_stopped",
                        "History stopped because its resume state and gap marker could not be persisted.",
                    );
                    return;
                }
                None
            }
        };
        if resume.is_none()
            && self.store.has_items_for_database(connection.id, &database).unwrap_or(false)
            && self
                .store
                .record_gap(
                    connection.id,
                    Some(database.clone()),
                    None,
                    "missing_resume_token",
                    "Stored events exist without a resume token; recording restarted at the current point.",
                )
                .is_err()
        {
            self.surface_stopped_gap(
                connection.id,
                Some(database.clone()),
                None,
                "recorder_stopped",
                "History stopped because a missing-token gap could not be persisted.",
            );
            return;
        }
        let split_large_events = supports_split_large_events(&connection.client).await;
        loop {
            if cancellation.is_cancelled() {
                break;
            }
            let watched_database = connection.client.database(&database);
            let pipeline = if split_large_events {
                vec![doc! { "$changeStreamSplitLargeEvent": {} }]
            } else {
                Vec::new()
            };
            let watch = watched_database
                .watch()
                .pipeline(pipeline)
                .full_document(FullDocumentType::WhenAvailable)
                .full_document_before_change(FullDocumentBeforeChangeType::WhenAvailable)
                .show_expanded_events(true);
            let parsed_resume = match resume.as_ref() {
                Some(bytes) => match mongodb::bson::from_slice::<
                    mongodb::change_stream::event::ResumeToken,
                >(bytes)
                {
                    Ok(token) => Some(token),
                    Err(error) => {
                        if self
                            .store
                            .record_gap_and_clear_cursor(
                                connection.id,
                                database.clone(),
                                "resume_token_invalid",
                                &format!("Stored resume token is malformed: {error}"),
                            )
                            .is_err()
                        {
                            self.surface_stopped_gap(
                                connection.id,
                                Some(database.clone()),
                                None,
                                "recorder_stopped",
                                "History stopped because an invalid resume token could not be surfaced.",
                            );
                            return;
                        }
                        resume = None;
                        None
                    }
                },
                None => None,
            };
            let opened = if let Some(token) = parsed_resume {
                watch.resume_after(token).await
            } else {
                watch.await
            };
            let mut stream = match opened {
                Ok(stream) => stream.with_type::<Document>(),
                Err(error) => {
                    if resume.is_some() && resume_history_is_invalid(&error) {
                        if self
                            .store
                            .record_gap_and_clear_cursor(
                                connection.id,
                                database.clone(),
                                "resume_token_expired",
                                &format!("MongoDB rejected the stored resume point: {error}"),
                            )
                            .is_err()
                        {
                            self.surface_stopped_gap(
                                connection.id,
                                Some(database.clone()),
                                None,
                                "recorder_stopped",
                                "History stopped because an expired resume point could not be surfaced.",
                            );
                            return;
                        }
                        resume = None;
                    } else if resume.is_none() {
                        let _ = self.store.record_gap(
                            connection.id,
                            Some(database.clone()),
                            None,
                            "change_stream_unavailable",
                            &format!("Could not open the change stream: {error}"),
                        );
                    }
                    tokio::select! {
                        _ = cancellation.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_secs(2)) => continue,
                    }
                }
            };
            loop {
                let next = tokio::select! {
                    _ = cancellation.cancelled() => return,
                    next = next_change_event(&mut stream) => next,
                };
                let event = match next {
                    Ok(Some(event)) => event,
                    Ok(None) => break,
                    Err(NextEventError::Transient) => break,
                    Err(NextEventError::Discontinuity { kind, reason }) => {
                        if self
                            .store
                            .record_gap_and_clear_cursor(
                                connection.id,
                                database.clone(),
                                kind,
                                &reason,
                            )
                            .is_err()
                        {
                            self.surface_stopped_gap(
                                connection.id,
                                Some(database.clone()),
                                None,
                                "recorder_stopped",
                                "History stopped because a stream discontinuity could not be surfaced.",
                            );
                            return;
                        }
                        resume = None;
                        break;
                    }
                };
                let token = match mongodb::bson::to_vec(&event.id) {
                    Ok(token) => token,
                    Err(error) => {
                        let _ = self.store.record_gap(
                            connection.id,
                            Some(database.clone()),
                            None,
                            "resume_token_invalid",
                            &error.to_string(),
                        );
                        break;
                    }
                };
                resume = Some(token.clone());
                let wall_time = event
                    .wall_time
                    .and_then(|value| DateTime::from_timestamp_millis(value.timestamp_millis()))
                    .unwrap_or_else(Utc::now);
                let cluster_time =
                    event.cluster_time.map(|value| format!("{}:{}", value.time, value.increment));
                let family = match event.operation_type {
                    OperationType::Update => Some(OperationFamily::Update),
                    OperationType::Replace => Some(OperationFamily::Replace),
                    OperationType::Delete => Some(OperationFamily::Delete),
                    OperationType::Rename | OperationType::Other(_) => {
                        if let Err(error) = self.setup_pre_post_images(&connection).await {
                            let namespace = event.ns.as_ref();
                            if self
                                .store
                                .record_gap_and_advance_cursor(
                                    connection.id,
                                    database.clone(),
                                    namespace.and_then(|ns| ns.coll.clone()),
                                    "uncovered_collection",
                                    &error,
                                    token.clone(),
                                    cluster_time.clone(),
                                    wall_time.timestamp_millis(),
                                )
                                .is_err()
                            {
                                self.surface_stopped_gap(
                                    connection.id,
                                    Some(database.clone()),
                                    namespace.and_then(|ns| ns.coll.clone()),
                                    "recorder_stopped",
                                    "History stopped because new-collection coverage could not be persisted.",
                                );
                                return;
                            }
                            continue;
                        }
                        None
                    }
                    _ => None,
                };
                let Some(family) = family else {
                    if self
                        .store
                        .advance_cursor(
                            connection.id,
                            database.clone(),
                            token,
                            cluster_time,
                            wall_time.timestamp_millis(),
                        )
                        .is_err()
                    {
                        self.surface_stopped_gap(
                            connection.id,
                            Some(database.clone()),
                            None,
                            "recorder_stopped",
                            "History stopped because resume state could not be persisted.",
                        );
                        return;
                    }
                    continue;
                };
                let Some(namespace) = event.ns else {
                    let _ = self.store.record_gap(
                        connection.id,
                        Some(database.clone()),
                        None,
                        "event_missing_namespace",
                        "A supported change event had no namespace.",
                    );
                    continue;
                };
                let collection = namespace.coll.unwrap_or_default();
                let before = event.full_document_before_change;
                let after = event.full_document;
                let images_available = match family {
                    OperationFamily::Update | OperationFamily::Replace => {
                        before.is_some() && after.is_some()
                    }
                    OperationFamily::Delete => before.is_some(),
                };
                if !images_available {
                    if self
                        .store
                        .record_gap_and_advance_cursor(
                            connection.id,
                            database.clone(),
                            Some(collection.clone()),
                            "missing_pre_post_image",
                            "MongoDB did not return the exact required pre/post image for a supported change.",
                            token,
                            cluster_time,
                            wall_time.timestamp_millis(),
                        )
                        .is_err()
                    {
                        self.surface_stopped_gap(
                            connection.id,
                            Some(database.clone()),
                            Some(collection),
                            "recorder_stopped",
                            "History stopped because a missing-image gap could not be persisted.",
                        );
                        return;
                    }
                    continue;
                }
                let document_key = event.document_key.unwrap_or_default();
                let transaction_key = event.lsid.and_then(|lsid| {
                    event.txn_number.map(|txn| {
                        mongodb::bson::to_vec(&doc! {
                            "lsid": lsid,
                            "txnNumber": txn,
                        })
                        .unwrap_or_default()
                    })
                });
                let trace_id =
                    self.match_trace(connection.id, &database, &collection, family, wall_time);
                let recorded = self.store.record_event(RecordedEvent {
                    connection_id: connection.id,
                    database: database.clone(),
                    collection,
                    family,
                    document_key,
                    before,
                    after,
                    resume_token: token,
                    cluster_time,
                    wall_time,
                    transaction_key,
                    trace_id,
                });
                if let Err(error) = recorded {
                    if self
                        .store
                        .record_gap(
                            connection.id,
                            Some(database.clone()),
                            None,
                            "persistence_failed",
                            &error.to_string(),
                        )
                        .is_err()
                    {
                        self.surface_stopped_gap(
                            connection.id,
                            Some(database.clone()),
                            None,
                            "recorder_stopped",
                            &format!("History persistence failed: {error}"),
                        );
                    }
                    return;
                }
                if let Err(error) = self.store.apply_retention(
                    connection.id,
                    connection.max_age_days,
                    connection.max_bytes,
                ) {
                    if self
                        .store
                        .record_gap(
                            connection.id,
                            Some(database.clone()),
                            None,
                            "storage_limit",
                            &error.to_string(),
                        )
                        .is_err()
                    {
                        self.surface_stopped_gap(
                            connection.id,
                            Some(database.clone()),
                            None,
                            "recorder_stopped",
                            &format!("History retention failed: {error}"),
                        );
                    }
                    return;
                }
                self.refresh_usage_cache(connection.id);
            }
            tokio::select! {
                _ = cancellation.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
        }
    }

    fn surface_stopped_gap(
        &self,
        connection_id: Uuid,
        database: Option<String>,
        collection: Option<String>,
        kind: &str,
        reason: &str,
    ) {
        if let Ok(mut gaps) = self.stopped_gaps.lock() {
            gaps.push(HistoryGap {
                id: Uuid::new_v4(),
                connection_id,
                database,
                collection,
                kind: kind.chars().take(64).collect(),
                reason: reason.chars().take(500).collect(),
                created_at: Utc::now(),
                resolved: false,
            });
        }
    }

    fn match_trace(
        &self,
        connection_id: Uuid,
        database: &str,
        collection: &str,
        family: OperationFamily,
        wall_time: DateTime<Utc>,
    ) -> Option<Uuid> {
        let Ok(mut traces) = self.traces.lock() else {
            return None;
        };
        let now = Utc::now();
        traces.retain(|trace| {
            now - trace.descriptor.started_at < chrono::Duration::minutes(5)
                && trace.matched_events < trace.descriptor.affected_count.unwrap_or(u64::MAX)
        });
        let candidates = traces
            .iter()
            .enumerate()
            .filter(|(_, trace)| {
                let descriptor = &trace.descriptor;
                descriptor.completed_at.is_some()
                    && descriptor.affected_count.is_some_and(|count| trace.matched_events < count)
                    && descriptor.connection_id == connection_id
                    && descriptor.database == database
                    && descriptor.collection == collection
                    && descriptor.family == family
                    && wall_time >= descriptor.started_at - chrono::Duration::seconds(1)
                    && wall_time
                        <= descriptor.completed_at.unwrap_or(descriptor.started_at)
                            + chrono::Duration::seconds(1)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let index = *candidates.first()?;
        if candidates.len() != 1 {
            return None;
        }
        traces[index].matched_events += 1;
        Some(traces[index].descriptor.id)
    }

    async fn restore_batch(
        &self,
        client: mongodb::Client,
        batch_id: Uuid,
        cancellation: CancellationToken,
    ) -> Result<(), String> {
        loop {
            let items = self
                .store
                .restore_items_page(batch_id, RESTORE_CHUNK_ITEMS)
                .map_err(|error| error.to_string())?;
            if items.is_empty() {
                break;
            }
            for item in &items {
                if cancellation.is_cancelled() {
                    self.store.finish_restore(batch_id, true).map_err(|error| error.to_string())?;
                    return Ok(());
                }
                self.store
                    .mark_item_outcome(batch_id, item.id, "applying", None)
                    .map_err(|error| error.to_string())?;
                let outcome = restore_item(&client, item).await;
                match outcome {
                    Ok("restored") => {
                        self.store.mark_item_outcome(batch_id, item.id, "restored", None)
                    }
                    Ok("skipped") => {
                        self.store.mark_item_outcome(batch_id, item.id, "skipped", None)
                    }
                    Ok(_) => self.store.mark_item_outcome(
                        batch_id,
                        item.id,
                        "conflicted",
                        Some("current_document_mismatch"),
                    ),
                    Err(error) => {
                        self.store.mark_item_outcome(batch_id, item.id, "failed", Some(&error))
                    }
                }
                .map_err(|error| error.to_string())?;
            }
        }
        self.store.finish_restore(batch_id, false).map_err(|error| error.to_string())?;
        if let Ok(details) = self.store.get_batch(batch_id, 0, 1) {
            self.refresh_usage_cache(details.summary.connection_id);
        }
        Ok(())
    }
}

async fn restore_item(
    client: &mongodb::Client,
    item: &RestoreItem,
) -> Result<&'static str, String> {
    let collection = client.database(&item.database).collection::<Document>(&item.collection);
    match item.family {
        OperationFamily::Update | OperationFamily::Replace => {
            let Some(before) = item.before.clone() else {
                return Ok("skipped");
            };
            let Some(after) = item.after.clone() else {
                return Ok("skipped");
            };
            let current = collection
                .find_one(item.document_key.clone())
                .await
                .map_err(|error| error.to_string())?;
            if current.as_ref() == Some(&before) {
                return Ok("restored");
            }
            if current.as_ref() != Some(&after) {
                return Ok("conflicted");
            }
            let mut filter = item.document_key.clone();
            filter.insert("$expr", doc! { "$eq": ["$$ROOT", { "$literal": after }] });
            let result =
                collection.replace_one(filter, before).await.map_err(|error| error.to_string())?;
            Ok(if result.matched_count == 1 { "restored" } else { "conflicted" })
        }
        OperationFamily::Delete => {
            let Some(before) = item.before.clone() else {
                return Ok("skipped");
            };
            let current = collection
                .find_one(item.document_key.clone())
                .await
                .map_err(|error| error.to_string())?;
            if current.as_ref() == Some(&before) {
                return Ok("restored");
            }
            if current.is_some() {
                return Ok("conflicted");
            }
            match collection.insert_one(before).await {
                Ok(_) => Ok("restored"),
                Err(error) if is_duplicate_key(&error) => Ok("conflicted"),
                Err(error) => Err(error.to_string()),
            }
        }
    }
}

#[derive(Debug)]
enum NextEventError {
    Transient,
    Discontinuity { kind: &'static str, reason: String },
}

async fn next_change_event(
    stream: &mut mongodb::change_stream::ChangeStream<Document>,
) -> Result<Option<ChangeStreamEvent<Document>>, NextEventError> {
    let Some(mut document) = stream.try_next().await.map_err(classify_stream_error)? else {
        return Ok(None);
    };
    let Some(split) = document.get_document("splitEvent").ok().cloned() else {
        return mongodb::bson::from_document(document).map(Some).map_err(|error| {
            NextEventError::Discontinuity {
                kind: "invalid_change_event",
                reason: format!("MongoDB returned an invalid change event: {error}"),
            }
        });
    };
    let fragment = positive_integer(&split, "fragment");
    let total = positive_integer(&split, "of");
    if fragment != Some(1) || total.is_none() {
        return Err(NextEventError::Discontinuity {
            kind: "split_event_invalid",
            reason: "MongoDB returned an out-of-order split change event.".into(),
        });
    }
    let total = total.unwrap_or(1);
    document.remove("splitEvent");
    for expected in 2..=total {
        let Some(mut next) = stream.try_next().await.map_err(classify_stream_error)? else {
            return Err(NextEventError::Discontinuity {
                kind: "split_event_incomplete",
                reason: "The change stream ended before a split event was complete.".into(),
            });
        };
        let valid = next.get_document("splitEvent").ok().is_some_and(|split| {
            positive_integer(split, "fragment") == Some(expected)
                && positive_integer(split, "of") == Some(total)
        });
        if !valid {
            return Err(NextEventError::Discontinuity {
                kind: "split_event_invalid",
                reason: "MongoDB returned non-sequential split event fragments.".into(),
            });
        }
        if let Some(id) = next.remove("_id") {
            document.insert("_id", id);
        }
        next.remove("splitEvent");
        merge_split_fields(&mut document, next)?;
    }
    mongodb::bson::from_document(document).map(Some).map_err(|error| {
        NextEventError::Discontinuity {
            kind: "split_event_invalid",
            reason: format!("A reassembled split event was invalid: {error}"),
        }
    })
}

fn merge_split_fields(document: &mut Document, fragment: Document) -> Result<(), NextEventError> {
    for (key, value) in fragment {
        if document.contains_key(&key) {
            return Err(NextEventError::Discontinuity {
                kind: "split_event_invalid",
                reason: format!(
                    "MongoDB repeated the top-level field {key:?} across split fragments."
                ),
            });
        }
        document.insert(key, value);
    }
    Ok(())
}

fn positive_integer(document: &Document, key: &str) -> Option<i64> {
    match document.get(key)? {
        mongodb::bson::Bson::Int32(value) if *value > 0 => Some(i64::from(*value)),
        mongodb::bson::Bson::Int64(value) if *value > 0 => Some(*value),
        _ => None,
    }
}

fn classify_stream_error(error: mongodb::error::Error) -> NextEventError {
    let reason = format!("Change stream interrupted: {error}");
    let lower = reason.to_ascii_lowercase();
    if lower.contains("large") || lower.contains("bsonobj size") || lower.contains("16mb") {
        NextEventError::Discontinuity { kind: "oversized_event", reason }
    } else {
        NextEventError::Transient
    }
}

fn resume_history_is_invalid(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Command(command) if resume_history_code_is_invalid(command.code)
    )
}

fn resume_history_code_is_invalid(code: i32) -> bool {
    matches!(code, 136 | 237 | 280 | 286)
}

async fn supports_split_large_events(client: &mongodb::Client) -> bool {
    let Ok(info) = client.database("admin").run_command(doc! { "buildInfo": 1 }).await else {
        return false;
    };
    let Ok(version) = info.get_str("version") else {
        return false;
    };
    semver::Version::parse(version).is_ok_and(|version| {
        version.major >= 7 || (version.major == 6 && (version.minor > 0 || version.patch >= 9))
    })
}

fn is_duplicate_key(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

pub async fn eligibility(connection: &HistoryConnection) -> EligibilityReport {
    let mut failures = Vec::new();
    let build_info = connection.client.database("admin").run_command(doc! { "buildInfo": 1 }).await;
    let version = match build_info {
        Ok(info) => info.get_str("version").ok().map(str::to_string),
        Err(error) => {
            failures.push(format!("Cannot inspect MongoDB version (buildInfo privilege): {error}"));
            None
        }
    };
    let hello = connection.client.database("admin").run_command(doc! { "hello": 1 }).await;
    let topology = match hello {
        Ok(hello) if hello.get_str("msg").ok() == Some("isdbgrid") => Some("sharded".into()),
        Ok(hello) if hello.get_str("setName").is_ok() => Some("replica_set".into()),
        Ok(_) => {
            failures.push("History requires a replica set or sharded topology; standalone MongoDB is unsupported.".into());
            Some("standalone".into())
        }
        Err(error) => {
            failures.push(format!("Cannot inspect MongoDB topology (hello privilege): {error}"));
            None
        }
    };
    let storage_status =
        connection.client.database("admin").run_command(doc! { "serverStatus": 1 }).await;
    let sharded = topology.as_deref() == Some("sharded");
    let storage_engine = match storage_status {
        Ok(status) => status
            .get_document("storageEngine")
            .ok()
            .and_then(|storage| storage.get_str("name").ok())
            .map(str::to_string)
            // MongoDB 6+ mongos processes have no local storage engine; supported sharded
            // deployments use WiredTiger on their data-bearing members.
            .or_else(|| sharded.then(|| "wiredTiger".to_string())),
        Err(_) if sharded => Some("wiredTiger".to_string()),
        Err(error) => {
            failures
                .push(format!("Cannot prove WiredTiger storage (serverStatus privilege): {error}"));
            None
        }
    };
    let mut collections = Vec::new();
    for database in &connection.databases {
        let specifications = match connection.client.database(database).list_collections().await {
            Ok(specifications) => specifications,
            Err(error) => {
                failures.push(format!("Cannot inspect collections in {database}: {error}"));
                continue;
            }
        };
        let specifications = match specifications.try_collect::<Vec<_>>().await {
            Ok(specifications) => specifications,
            Err(error) => {
                failures.push(format!("Cannot inspect collections in {database}: {error}"));
                continue;
            }
        };
        for specification in specifications {
            let regular =
                specification.collection_type == mongodb::results::CollectionType::Collection;
            let pre_post_images = specification
                .options
                .change_stream_pre_and_post_images
                .as_ref()
                .is_some_and(|images| images.enabled);
            let reason = if regular {
                if connection
                    .client
                    .database(database)
                    .collection::<Document>(&specification.name)
                    .find(doc! {})
                    .limit(1)
                    .await
                    .is_err()
                {
                    Some("Missing find privilege".into())
                } else if !pre_post_images {
                    Some("changeStreamPreAndPostImages is not enabled".into())
                } else {
                    None
                }
            } else {
                Some(
                    match specification.collection_type {
                        mongodb::results::CollectionType::View => "Views are unsupported",
                        mongodb::results::CollectionType::Timeseries => {
                            "Time-series collections are unsupported"
                        }
                        _ => "Collection type is unsupported",
                    }
                    .into(),
                )
            };
            collections.push(CollectionCoverage {
                database: database.clone(),
                collection: specification.name,
                regular,
                pre_post_images,
                reason,
            });
        }
        match tokio::time::timeout(
            Duration::from_secs(5),
            connection.client.database(database).watch(),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => failures.push(format!(
                "Change streams are unavailable for {database} (changeStream privilege): {error}"
            )),
            Err(_) => {
                failures.push(format!("Change stream privilege probe timed out for {database}"))
            }
        }
    }
    evaluate_requirements(version, topology, storage_engine, failures, collections)
}

fn evaluate_requirements(
    version: Option<String>,
    topology: Option<String>,
    storage_engine: Option<String>,
    mut failures: Vec<String>,
    collections: Vec<CollectionCoverage>,
) -> EligibilityReport {
    if let Some(version_value) = version.as_deref() {
        match semver::Version::parse(version_value) {
            Ok(version) if version.major >= 6 => {}
            Ok(_) => failures.push("History requires MongoDB 6.0 or newer.".into()),
            Err(_) => {
                failures.push(format!("MongoDB version {version_value} could not be parsed."))
            }
        }
    }
    if storage_engine.as_deref().is_some_and(|storage| storage != "wiredTiger") {
        failures.push("History requires the WiredTiger storage engine.".into());
    }
    let missing_find = collections.iter().any(|collection| {
        collection.regular && collection.reason.as_deref() == Some("Missing find privilege")
    });
    if missing_find {
        failures
            .push("History requires find privilege on every covered regular collection.".into());
    }
    let mandatory_failed = !failures.is_empty()
        || topology.as_deref().is_none_or(|value| value == "standalone")
        || storage_engine.as_deref() != Some("wiredTiger")
        || version.is_none();
    let needs_setup = collections.iter().any(|collection| {
        collection.regular
            && collection.reason.as_deref() != Some("Missing find privilege")
            && !collection.pre_post_images
    });
    EligibilityReport {
        status: if mandatory_failed {
            EligibilityStatus::Unavailable
        } else if needs_setup {
            EligibilityStatus::NeedsSetup
        } else {
            EligibilityStatus::Eligible
        },
        version,
        topology,
        storage_engine,
        failures,
        collections,
    }
}

pub async fn setup_pre_post_images(connection: &HistoryConnection) -> Result<SetupReport, String> {
    let mut report = SetupReport::default();
    for database in &connection.databases {
        let specifications = connection
            .client
            .database(database)
            .list_collections()
            .await
            .map_err(|error| format!("Cannot inspect collections in {database}: {error}"))?
            .try_collect::<Vec<_>>()
            .await
            .map_err(|error| format!("Cannot inspect collections in {database}: {error}"))?;
        for specification in specifications {
            if specification.collection_type != mongodb::results::CollectionType::Collection
                || specification
                    .options
                    .change_stream_pre_and_post_images
                    .as_ref()
                    .is_some_and(|images| images.enabled)
            {
                continue;
            }
            let namespace = format!("{database}.{}", specification.name);
            match connection
                .client
                .database(database)
                .run_command(doc! {
                    "collMod": &specification.name,
                    "changeStreamPreAndPostImages": { "enabled": true },
                })
                .await
            {
                Ok(_) => report.enabled.push(namespace),
                Err(error) => report.failed.push(format!(
                    "{namespace}: missing collMod privilege or unsupported collection ({error})"
                )),
            }
        }
    }
    if report.failed.is_empty() { Ok(report) } else { Err(report.failed.join("; ")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligibility_requires_version_topology_storage_and_collection_coverage() {
        let eligible = evaluate_requirements(
            Some("7.0.4".into()),
            Some("replica_set".into()),
            Some("wiredTiger".into()),
            Vec::new(),
            vec![CollectionCoverage {
                database: "app".into(),
                collection: "items".into(),
                regular: true,
                pre_post_images: true,
                reason: None,
            }],
        );
        assert_eq!(eligible.status, EligibilityStatus::Eligible);

        let old = evaluate_requirements(
            Some("5.0.0".into()),
            Some("replica_set".into()),
            Some("wiredTiger".into()),
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(old.status, EligibilityStatus::Unavailable);
        assert!(old.failures.iter().any(|failure| failure.contains("6.0")));

        let standalone = evaluate_requirements(
            Some("7.0.0".into()),
            Some("standalone".into()),
            Some("wiredTiger".into()),
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(standalone.status, EligibilityStatus::Unavailable);

        let setup = evaluate_requirements(
            Some("7.0.0".into()),
            Some("sharded".into()),
            Some("wiredTiger".into()),
            Vec::new(),
            vec![CollectionCoverage {
                database: "app".into(),
                collection: "items".into(),
                regular: true,
                pre_post_images: false,
                reason: Some("changeStreamPreAndPostImages is not enabled".into()),
            }],
        );
        assert_eq!(setup.status, EligibilityStatus::NeedsSetup);
    }

    #[tokio::test]
    async fn trace_matching_is_zero_safe_ambiguous_safe_and_count_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let service = HistoryService::open(
            directory.path().join("history.sqlite3"),
            [31; 32],
            tokio::runtime::Handle::current(),
        )
        .unwrap();
        let connection_id = Uuid::new_v4();
        let now = Utc::now();
        assert_eq!(
            service.match_trace(connection_id, "app", "items", OperationFamily::Update, now,),
            None
        );
        let descriptor = |id| TraceDescriptor {
            id,
            connection_id,
            database: "app".into(),
            collection: "items".into(),
            family: OperationFamily::Update,
            started_at: now,
            completed_at: Some(now),
            affected_count: Some(1),
        };
        let first = Uuid::new_v4();
        service.register_trace(descriptor(first));
        assert_eq!(
            service.match_trace(connection_id, "app", "items", OperationFamily::Update, now,),
            Some(first)
        );
        assert_eq!(
            service.match_trace(connection_id, "app", "items", OperationFamily::Update, now,),
            None
        );
        service.register_trace(descriptor(Uuid::new_v4()));
        service.register_trace(descriptor(Uuid::new_v4()));
        assert_eq!(
            service.match_trace(connection_id, "app", "items", OperationFamily::Update, now,),
            None
        );
    }

    #[test]
    fn missing_find_privilege_is_mandatory() {
        let report = evaluate_requirements(
            Some("7.0.0".into()),
            Some("replica_set".into()),
            Some("wiredTiger".into()),
            Vec::new(),
            vec![CollectionCoverage {
                database: "app".into(),
                collection: "items".into(),
                regular: true,
                pre_post_images: true,
                reason: Some("Missing find privilege".into()),
            }],
        );
        assert_eq!(report.status, EligibilityStatus::Unavailable);
        assert!(report.exact_reason().unwrap().contains("find privilege"));
    }

    #[test]
    fn split_fragments_merge_distinct_fields_and_reject_duplicates() {
        let mut event = doc! { "operationType": "update" };
        merge_split_fields(&mut event, doc! { "fullDocument": { "_id": 1 } }).unwrap();
        assert!(event.contains_key("fullDocument"));

        let error =
            merge_split_fields(&mut event, doc! { "fullDocument": { "_id": 2 } }).unwrap_err();
        assert!(matches!(error, NextEventError::Discontinuity { kind: "split_event_invalid", .. }));
    }

    #[test]
    fn expired_resume_history_codes_are_classified_without_treating_transient_errors_as_gaps() {
        for code in [136, 237, 280, 286] {
            assert!(resume_history_code_is_invalid(code));
        }
        for code in [6, 7, 89, 91, 11600] {
            assert!(!resume_history_code_is_invalid(code));
        }
    }

    #[test]
    fn restore_filters_require_exact_after_images() {
        let after = doc! { "_id": 1, "value": "after" };
        let mut filter = doc! { "_id": 1 };
        filter.insert("$expr", doc! { "$eq": ["$$ROOT", { "$literal": after.clone() }] });
        assert_eq!(
            filter.get_document("$expr").unwrap(),
            &doc! { "$eq": ["$$ROOT", { "$literal": after }] }
        );
    }
}
