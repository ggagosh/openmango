//! Read-only compare tools: the Compare tab's engine, bounded in time and output. A client that
//! declares the MCP tasks extension gets a task it can poll and cancel instead of a blocked call.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use mongodb::Client;
use mongodb::bson::RawDocumentBuf;
use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, DetailedTask, Task};
use rmcp::schemars::JsonSchema;
use rmcp::task_manager::{TaskExit, TaskManager, TaskOptions};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::server::{
    MAX_OUTPUT_BYTES, parse_connection_id, parse_read_document, validate_namespace,
};
use crate::bson::compare::IgnoreSet;
use crate::connection::CancellationToken;
use crate::connection::ops::compare::{
    CompareCounts, CompareMessage, CompareOptions, DiffKind, compare_collections_async,
};
use crate::connection::ops::compare_database::{
    CollectionKind, PairKind, PairMessage, PairScan, compare_pairs_async, list_side,
    pair_collections,
};
use crate::state::compare::{PairStatus, summary_status};

/// Without tasks a compare stops here and returns what it counted, inside the 35 s wall limit.
pub(super) const INLINE_LIMIT: Duration = Duration::from_secs(30);
/// As a task it may run this long, then stops the same way.
pub(super) const TASK_LIMIT: Duration = Duration::from_secs(600);
/// Counted from creation: a task past its limit, plus five minutes to fetch the result.
const TASK_TTL: Duration = Duration::from_secs(900);
const MAX_TASKS_PER_GRANT: usize = 2;
const MAX_TASKS: usize = 4;
const DEFAULT_DIFFERENCES: i64 = 50;
const MAX_DIFFERENCES: i64 = 200;
const MAX_COLLECTIONS: usize = 500;
const LISTING_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CompareCollectionsRequest {
    left_connection_id: String,
    left_database: String,
    left_collection: String,
    right_connection_id: String,
    right_database: String,
    right_collection: String,
    /// Fields that pair documents across the collections, compound if several. Defaults to _id.
    #[serde(default)]
    match_fields: Option<Vec<String>>,
    /// Extended JSON filter applied to both collections.
    #[serde(default)]
    filter: Option<serde_json::Value>,
    /// Dotted paths left out of value comparisons, for example updatedAt.
    #[serde(default)]
    ignore_fields: Vec<String>,
    /// Arrays holding the same items in another order count as minor.
    #[serde(default)]
    ignore_array_order: bool,
    /// Differences to list, 50 by default and at most 200. Counts always cover every document.
    #[serde(default)]
    max_differences: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CompareDatabasesRequest {
    left_connection_id: String,
    left_database: String,
    right_connection_id: String,
    right_database: String,
    /// Dotted paths left out of value comparisons in every collection.
    #[serde(default)]
    ignore_fields: Vec<String>,
    /// Arrays holding the same items in another order count as minor.
    #[serde(default)]
    ignore_array_order: bool,
    /// Collections listed but not compared, for example a large log collection.
    #[serde(default)]
    skip_collections: Vec<String>,
    /// List identical collections too. They are always counted.
    #[serde(default)]
    include_identical: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct Endpoint {
    connection_id: String,
    database: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    collection: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct Counts {
    left_read: i64,
    right_read: i64,
    identical: i64,
    only_left: i64,
    only_right: i64,
    different: i64,
    minor: i64,
    multiple_matches: i64,
}

/// Responses use signed integers: some MCP clients reject unsigned schema formats.
fn signed(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn elapsed_ms(started: Instant) -> i64 {
    signed(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
}

impl From<CompareCounts> for Counts {
    fn from(c: CompareCounts) -> Self {
        Self {
            left_read: signed(c.left_read),
            right_read: signed(c.right_read),
            identical: signed(c.identical),
            only_left: signed(c.only_left),
            only_right: signed(c.only_right),
            different: signed(c.different),
            minor: signed(c.minor),
            multiple_matches: signed(c.multiple_matches),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct Difference {
    /// The match key, canonical Extended JSON.
    key: serde_json::Value,
    /// only_left, only_right, different, minor or multiple_matches.
    kind: &'static str,
    /// Changed leaf fields, for different documents.
    changed_fields: i64,
    /// Up to three changed paths.
    changed_paths: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct CompareCollectionsResponse {
    data_classification: &'static str,
    extended_json: &'static str,
    left: Endpoint,
    right: Endpoint,
    match_fields: Vec<String>,
    /// False when the time limit or a cancel stopped the scan; counts cover what was read.
    complete: bool,
    counts: Counts,
    /// Documents left out for lacking a match field, left then right. Null if not counted.
    skipped_without_key: Option<[i64; 2]>,
    differences: Vec<Difference>,
    /// More differences exist than are listed.
    differences_truncated: bool,
    elapsed_ms: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct IndexDifference {
    left_only: Vec<String>,
    right_only: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct CollectionResult {
    name: String,
    /// identical, different, minor, left_only, right_only, view, timeseries, skipped,
    /// incomplete (stopped mid-scan), not_reached (stopped before its turn) or failed.
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    counts: Option<Counts>,
    /// Metadata estimates, left then right; approximate.
    estimated_documents: [Option<i64>; 2],
    /// Indexes on one side only, described by keys and options, not names.
    #[serde(skip_serializing_if = "Option::is_none")]
    index_differences: Option<IndexDifference>,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
pub(super) struct Totals {
    collections: i64,
    identical: i64,
    different: i64,
    minor: i64,
    left_only: i64,
    right_only: i64,
    not_compared: i64,
    skipped: i64,
    incomplete: i64,
    not_reached: i64,
    failed: i64,
    with_index_differences: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct CompareDatabasesResponse {
    data_classification: &'static str,
    left: Endpoint,
    right: Endpoint,
    /// Documents are matched by _id in every collection.
    match_fields: Vec<String>,
    /// False when the time limit or a cancel stopped the run.
    complete: bool,
    totals: Totals,
    /// Every collection except identical ones, unless include_identical was set.
    collections: Vec<CollectionResult>,
    collections_truncated: bool,
    elapsed_ms: i64,
}

pub(super) struct CollectionsPlan {
    pub connections: [Uuid; 2],
    endpoints: [(String, String); 2],
    options: CompareOptions,
    max_differences: usize,
}

pub(super) struct DatabasesPlan {
    pub connections: [Uuid; 2],
    databases: [String; 2],
    ignore: IgnoreSet,
    skip: Vec<String>,
    include_identical: bool,
}

fn ignore_set(fields: &[String], array_order: bool) -> Result<IgnoreSet, String> {
    if fields.len() > 100 || fields.iter().any(|field| field.is_empty() || field.len() > 255) {
        return Err("ignore_fields takes at most 100 non-empty paths".into());
    }
    Ok(IgnoreSet::new(fields).ignoring_array_order(array_order))
}

impl CompareCollectionsRequest {
    pub(super) fn validate(self) -> Result<CollectionsPlan, String> {
        let connections = [
            parse_connection_id(&self.left_connection_id)?,
            parse_connection_id(&self.right_connection_id)?,
        ];
        for (database, collection) in [
            (&self.left_database, &self.left_collection),
            (&self.right_database, &self.right_collection),
        ] {
            validate_namespace(database, "database")?;
            validate_namespace(collection, "collection")?;
        }
        let fields = self.match_fields.unwrap_or_else(|| vec!["_id".into()]);
        let ignore = ignore_set(&self.ignore_fields, self.ignore_array_order)?;
        let ignore = if fields == ["_id"] { ignore } else { ignore.ignoring_id() };
        let filter = self.filter.map(|f| parse_read_document(f, "filter")).transpose()?;
        let max_differences = self.max_differences.unwrap_or(DEFAULT_DIFFERENCES);
        let max_differences = max_differences.clamp(0, MAX_DIFFERENCES) as usize;
        let options = CompareOptions {
            fields,
            filter: filter.unwrap_or_default(),
            ignore,
            row_limit: max_differences,
            row_kinds: None,
        };
        options.validate().map_err(|error| error.to_string())?;
        Ok(CollectionsPlan {
            connections,
            endpoints: [
                (self.left_database, self.left_collection),
                (self.right_database, self.right_collection),
            ],
            options,
            max_differences,
        })
    }
}

impl CompareDatabasesRequest {
    pub(super) fn validate(self) -> Result<DatabasesPlan, String> {
        let connections = [
            parse_connection_id(&self.left_connection_id)?,
            parse_connection_id(&self.right_connection_id)?,
        ];
        validate_namespace(&self.left_database, "database")?;
        validate_namespace(&self.right_database, "database")?;
        if self.skip_collections.len() > MAX_COLLECTIONS {
            return Err(format!("skip_collections takes at most {MAX_COLLECTIONS} names"));
        }
        Ok(DatabasesPlan {
            connections,
            databases: [self.left_database, self.right_database],
            ignore: ignore_set(&self.ignore_fields, self.ignore_array_order)?,
            skip: self.skip_collections,
            include_identical: self.include_identical,
        })
    }
}

/// Cancels `token` after `limit`, or sooner when `stop` resolves. The scan then returns what
/// it counted instead of an error.
fn cancel_after(
    token: CancellationToken,
    limit: Duration,
    stop: impl Future<Output = ()> + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::select! {
            _ = tokio::time::sleep(limit) => {}
            _ = stop => {}
        }
        token.cancel();
    })
}

fn kind_name(kind: DiffKind) -> &'static str {
    match kind {
        DiffKind::OnlyLeft => "only_left",
        DiffKind::OnlyRight => "only_right",
        DiffKind::Different => "different",
        DiffKind::Minor => "minor",
        DiffKind::MultipleMatches => "multiple_matches",
    }
}

pub(super) async fn compare_collections(
    clients: [Client; 2],
    plan: CollectionsPlan,
    limit: Duration,
    stop: impl Future<Output = ()> + Send + 'static,
) -> Result<CompareCollectionsResponse, String> {
    let started = Instant::now();
    let [left, right] = [0, 1].map(|side| {
        let (database, collection) = &plan.endpoints[side];
        clients[side].database(database).collection::<RawDocumentBuf>(collection)
    });
    let token = CancellationToken::new();
    let timer = cancel_after(token.clone(), limit, stop);
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    let rows = receiver
        .filter_map(|message| async move {
            match message {
                CompareMessage::Progress { new_rows, .. } => Some(futures::stream::iter(new_rows)),
                _ => None,
            }
        })
        .flatten()
        .collect::<Vec<_>>();
    let fields = plan.options.fields.clone();
    let (result, rows) =
        tokio::join!(compare_collections_async(left, right, plan.options, token, sender), rows);
    timer.abort();
    let summary = result.map_err(super::server::safe_database_error)?;
    // ponytail: the byte budget is half the response limit, leaving room for everything else.
    let mut bytes = 0;
    let mut differences = Vec::new();
    for row in rows.into_iter().take(plan.max_differences) {
        let key = row.key.into_canonical_extjson();
        bytes += key.to_string().len() + row.paths.len();
        if bytes > MAX_OUTPUT_BYTES / 2 {
            break;
        }
        differences.push(Difference {
            key,
            kind: kind_name(row.kind),
            changed_fields: i64::from(row.changed),
            changed_paths: row
                .paths
                .split(", ")
                .filter(|path| !path.is_empty())
                .map(str::to_owned)
                .collect(),
        });
    }
    let c = summary.counts;
    let total = c.only_left + c.only_right + c.different + c.minor + c.multiple_matches;
    let [left, right] = [0, 1].map(|side| Endpoint {
        connection_id: plan.connections[side].to_string(),
        database: plan.endpoints[side].0.clone(),
        collection: Some(plan.endpoints[side].1.clone()),
    });
    Ok(CompareCollectionsResponse {
        data_classification: "untrusted_database_content",
        extended_json: "canonical",
        left,
        right,
        match_fields: fields,
        complete: !summary.cancelled,
        counts: c.into(),
        skipped_without_key: summary.skipped.map(|skipped| skipped.map(signed)),
        differences_truncated: (differences.len() as u64) < total,
        differences,
        elapsed_ms: elapsed_ms(started),
    })
}

pub(super) async fn compare_databases(
    clients: [Client; 2],
    plan: DatabasesPlan,
    limit: Duration,
    stop: impl Future<Output = ()> + Send + 'static,
) -> Result<CompareDatabasesResponse, String> {
    let started = Instant::now();
    let token = CancellationToken::new();
    let timer = cancel_after(token.clone(), limit, stop);
    let (left, right) = tokio::try_join!(
        list_side(&clients[0], &plan.databases[0], LISTING_TIMEOUT),
        list_side(&clients[1], &plan.databases[1], LISTING_TIMEOUT)
    )
    .map_err(super::server::safe_database_error)?;
    let pairs = pair_collections(left, right);
    // Smallest first, as in the database view, so a time limit still answers for most.
    let mut scans: Vec<_> = pairs
        .iter()
        .enumerate()
        .filter(|(_, pair)| pair.kind() == PairKind::Both && !plan.skip.contains(&pair.name))
        .map(|(index, pair)| {
            let size = pair.sides.iter().flatten().filter_map(|side| side.estimated).max();
            (size.unwrap_or(u64::MAX), index)
        })
        .collect();
    scans.sort();
    let scans = scans
        .into_iter()
        .map(|(_, index)| PairScan {
            index,
            name: pairs[index].name.clone(),
            cancellation: token.clone(),
        })
        .collect();
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    let (_, messages) = tokio::join!(
        compare_pairs_async(clients, plan.databases.clone(), scans, plan.ignore, sender),
        receiver.collect::<Vec<_>>()
    );
    timer.abort();
    let mut results: HashMap<usize, Result<crate::connection::ops::compare::CompareSummary, ()>> =
        HashMap::new();
    for message in messages {
        match message {
            PairMessage::Done(index, summary) => {
                results.insert(index, Ok(summary));
            }
            PairMessage::Failed(index, _) => {
                results.insert(index, Err(()));
            }
            PairMessage::Started(_) | PairMessage::Progress(..) => {}
        }
    }
    let mut totals = Totals { collections: pairs.len() as i64, ..Default::default() };
    let mut collections = Vec::new();
    for (index, pair) in pairs.iter().enumerate() {
        let mut counts = None;
        let status = match pair.kind() {
            PairKind::LeftOnly => "left_only",
            PairKind::RightOnly => "right_only",
            PairKind::NotComparable(CollectionKind::View) => "view",
            PairKind::NotComparable(_) => "timeseries",
            PairKind::Both if plan.skip.contains(&pair.name) => "skipped",
            PairKind::Both => match results.remove(&index) {
                None => "not_reached",
                Some(Err(())) => "failed",
                Some(Ok(summary)) => {
                    counts = Some(summary.counts.into());
                    match summary_status(&summary) {
                        _ if summary.cancelled => "incomplete",
                        PairStatus::Different => "different",
                        PairStatus::Minor => "minor",
                        _ => "identical",
                    }
                }
            },
        };
        let slot = match status {
            "identical" => &mut totals.identical,
            "different" => &mut totals.different,
            "minor" => &mut totals.minor,
            "left_only" => &mut totals.left_only,
            "right_only" => &mut totals.right_only,
            "view" | "timeseries" => &mut totals.not_compared,
            "skipped" => &mut totals.skipped,
            "incomplete" => &mut totals.incomplete,
            "not_reached" => &mut totals.not_reached,
            _ => &mut totals.failed,
        };
        *slot += 1;
        let index_differences = pair.index_difference().map(|[left_only, right_only]| {
            totals.with_index_differences += 1;
            IndexDifference { left_only, right_only }
        });
        if status == "identical" && !plan.include_identical && index_differences.is_none() {
            continue;
        }
        collections.push(CollectionResult {
            name: pair.name.clone(),
            status,
            counts,
            estimated_documents: pair
                .sides
                .each_ref()
                .map(|side| side.as_ref()?.estimated.map(signed)),
            index_differences,
        });
    }
    let collections_truncated = collections.len() > MAX_COLLECTIONS;
    collections.truncate(MAX_COLLECTIONS);
    let [left, right] = [0, 1].map(|side| Endpoint {
        connection_id: plan.connections[side].to_string(),
        database: plan.databases[side].clone(),
        collection: None,
    });
    Ok(CompareDatabasesResponse {
        data_classification: "untrusted_database_content",
        left,
        right,
        match_fields: vec!["_id".into()],
        complete: totals.incomplete + totals.not_reached == 0,
        totals,
        collections,
        collections_truncated,
        elapsed_ms: elapsed_ms(started),
    })
}

/// Tasks per grant, so one client can neither read nor cancel another's. Shared by every
/// session, since the server is cloned per session.
#[derive(Default)]
pub(super) struct McpTasks {
    managers: Mutex<HashMap<Uuid, TaskManager>>,
}

impl McpTasks {
    /// Runs `work` as a task. It receives a future that resolves when the client cancels.
    pub(super) fn spawn<F, W, T>(&self, grant: Uuid, work: W) -> Result<Task, String>
    where
        W: FnOnce(std::pin::Pin<Box<dyn Future<Output = ()> + Send>>) -> F,
        F: Future<Output = Result<T, String>> + Send + 'static,
        T: Serialize,
    {
        let mut managers = self.managers.lock().map_err(|_| "Task list is unavailable")?;
        let running: usize = managers.values().map(TaskManager::running_task_count).sum();
        let manager = managers.entry(grant).or_default();
        if running >= MAX_TASKS || manager.running_task_count() >= MAX_TASKS_PER_GRANT {
            return Err("Too many compare tasks are running; wait for one to finish".into());
        }
        let options = TaskOptions::new().with_ttl_ms(TASK_TTL.as_millis() as u64);
        Ok(manager.spawn(options, move |context| {
            let stop = context.clone();
            let future = work(Box::pin(async move { stop.cancelled().await }));
            Box::pin(async move {
                let result = future.await;
                if context.is_cancel_requested() {
                    return Err(TaskExit::Cancelled);
                }
                Ok(
                    match result.and_then(|value| {
                        serde_json::to_value(value).map_err(|error| error.to_string())
                    }) {
                        Ok(value) => CallToolResult::structured(value),
                        Err(message) => {
                            CallToolResult::error(vec![rmcp::model::ContentBlock::text(message)])
                        }
                    },
                )
            })
        }))
    }

    fn manager<R>(
        &self,
        grant: Uuid,
        task_id: &str,
        use_it: impl FnOnce(&TaskManager) -> Result<R, McpError>,
    ) -> Result<R, McpError> {
        let managers = self
            .managers
            .lock()
            .map_err(|_| McpError::internal_error("Task list is unavailable", None))?;
        match managers.get(&grant) {
            Some(manager) => use_it(manager),
            None => Err(McpError::invalid_params(format!("Unknown task: {task_id}"), None)),
        }
    }

    pub(super) fn get(&self, grant: Uuid, task_id: &str) -> Result<DetailedTask, McpError> {
        self.manager(grant, task_id, |manager| manager.get_task(task_id))
    }

    pub(super) fn cancel(&self, grant: Uuid, task_id: &str) -> Result<(), McpError> {
        self.manager(grant, task_id, |manager| manager.cancel_task(task_id))
    }

    pub(super) fn update(
        &self,
        grant: Uuid,
        task_id: &str,
        responses: impl IntoIterator<Item = (String, serde_json::Value)>,
    ) -> Result<(), McpError> {
        self.manager(grant, task_id, |manager| manager.update_task(task_id, responses))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::TaskStatus;

    #[tokio::test]
    async fn tasks_stay_with_their_grant_cancel_stops_work_and_are_capped() {
        let tasks = McpTasks::default();
        let (grant, other) = (Uuid::new_v4(), Uuid::new_v4());
        let waiting = |stop: std::pin::Pin<Box<dyn Future<Output = ()> + Send>>| async move {
            stop.await;
            Ok::<_, String>("stopped")
        };
        let first = tasks.spawn(grant, waiting).unwrap();
        assert!(tasks.get(other, &first.task_id).is_err(), "another grant sees nothing");
        assert!(tasks.cancel(other, &first.task_id).is_err());
        let _second = tasks.spawn(grant, waiting).unwrap();
        assert!(tasks.spawn(grant, waiting).is_err(), "two per grant");
        tasks.cancel(grant, &first.task_id).unwrap();
        let status = loop {
            let task = tasks.get(grant, &first.task_id).unwrap();
            if task.status().is_terminal() {
                break task.status();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        assert_eq!(status, TaskStatus::Cancelled);
        assert!(tasks.spawn(grant, waiting).is_ok(), "a finished task frees its slot");
    }
}
