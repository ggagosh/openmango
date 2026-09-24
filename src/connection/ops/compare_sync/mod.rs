//! Selective sync and guarded undo. These futures must run on the connection's Tokio runtime.

pub mod restore;

use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use futures::{TryStreamExt, channel::mpsc::UnboundedSender};
use mongodb::bson::{Bson, Document, RawDocument, RawDocumentBuf, doc};
use mongodb::options::{
    Acknowledgment, Collation, CollectionOptions, DeleteOneModel, InsertOneModel, ReadPreference,
    ReplaceOneModel, SelectionCriteria, WriteModel,
};
use mongodb::{
    Collection,
    error::{ErrorKind, PartialBulkWriteResult},
    results::VerboseBulkWriteResult,
};
use sha2::{Digest, Sha256};

use super::compare::{DiffKind, DiffRow, Side, compare_keys, extract_key};
use crate::bson::{PathSegment, get_bson_at_path, remove_bson_at_path, set_bson_at_path};
use crate::connection::CancellationToken;
use crate::error::{Error, Result};
use restore::{RestoreHandle, UndoRecord};

pub const BATCH_ROWS: usize = 1_000;
const READ_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Insert,
    Replace,
    Delete,
}

impl Operation {
    fn code(self) -> i32 {
        match self {
            Self::Insert => 0,
            Self::Replace => 1,
            Self::Delete => 2,
        }
    }
    fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Insert),
            1 => Some(Self::Replace),
            2 => Some(Self::Delete),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SyncItem {
    pub row_index: usize,
    pub row: DiffRow,
    pub operation: Operation,
    /// A field copy: a Replace that writes the target with only this path taken from the source.
    pub field: Option<Vec<PathSegment>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowOutcome {
    Written,
    Restored,
    Skipped(String),
    Failed(String),
    Uncertain(String),
}

impl RowOutcome {
    pub fn message(&self) -> &str {
        match self {
            Self::Written => "Written",
            Self::Restored => "Restored",
            Self::Skipped(message) | Self::Failed(message) | Self::Uncertain(message) => message,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SyncSummary {
    pub processed: usize,
    pub written: usize,
    pub skipped: usize,
    pub failed: usize,
    pub uncertain: usize,
    pub inserted: usize,
    pub replaced: usize,
    pub deleted: usize,
    pub cancelled: bool,
}

impl SyncSummary {
    /// Adds another run's totals, e.g. the next pass over the same collection.
    pub fn absorb(&mut self, other: &SyncSummary) {
        self.processed += other.processed;
        self.written += other.written;
        self.skipped += other.skipped;
        self.failed += other.failed;
        self.uncertain += other.uncertain;
        self.inserted += other.inserted;
        self.replaced += other.replaced;
        self.deleted += other.deleted;
        self.cancelled |= other.cancelled;
    }

    fn record(&mut self, operation: Operation, outcome: &RowOutcome) {
        self.processed += 1;
        match outcome {
            RowOutcome::Written | RowOutcome::Restored => {
                self.written += 1;
                match operation {
                    Operation::Insert => self.inserted += 1,
                    Operation::Replace => self.replaced += 1,
                    Operation::Delete => self.deleted += 1,
                }
            }
            RowOutcome::Skipped(_) => self.skipped += 1,
            RowOutcome::Failed(_) => self.failed += 1,
            RowOutcome::Uncertain(_) => self.uncertain += 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SyncProgress {
    pub summary: SyncSummary,
    pub outcomes: Vec<(usize, RowOutcome)>,
}

pub fn operation_for(kind: DiffKind, target: Side) -> Option<Operation> {
    match (kind, target) {
        (DiffKind::OnlyLeft, Side::Right) | (DiffKind::OnlyRight, Side::Left) => {
            Some(Operation::Insert)
        }
        (DiffKind::OnlyLeft, Side::Left) | (DiffKind::OnlyRight, Side::Right) => {
            Some(Operation::Delete)
        }
        (DiffKind::Different | DiffKind::Minor, _) => Some(Operation::Replace),
        (DiffKind::MultipleMatches, _) => None,
    }
}

pub fn raw_hash(document: &RawDocument) -> u64 {
    let mut hasher = DefaultHasher::new();
    document.as_bytes().hash(&mut hasher);
    hasher.finish()
}

fn fingerprint(document: &RawDocument) -> [u8; 32] {
    Sha256::digest(document.as_bytes()).into()
}

fn primary(collection: &Collection<RawDocumentBuf>) -> Collection<RawDocumentBuf> {
    let mut options = CollectionOptions::builder()
        .selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::Primary))
        .build();
    options.write_concern = collection.write_concern().cloned();
    options.read_concern = collection.read_concern().cloned();
    let ns = collection.namespace();
    collection.client().database(&ns.db).collection_with_options(&ns.coll, options)
}

async fn validate_target(target: &Collection<RawDocumentBuf>) -> Result<()> {
    if !supports_sync(target.client()).await? {
        return Err(Error::Parse("Sync and undo require MongoDB 8.0 or newer on the target. Older servers support comparison only.".into()));
    }
    if target.write_concern().is_some_and(|w| w.w == Some(Acknowledgment::Nodes(0))) {
        return Err(Error::Parse(
            "Sync and undo require acknowledged writes (w must not be 0)".into(),
        ));
    }
    let ns = target.namespace();
    let options = mongodb::options::DatabaseOptions::builder()
        .selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::Primary))
        .build();
    let spec = target
        .client()
        .database_with_options(&ns.db, options)
        .list_collections()
        .filter(doc! {"name": &ns.coll})
        .await?
        .try_next()
        .await?
        .ok_or_else(|| Error::Parse(format!("Target collection {ns} no longer exists")))?;
    if spec.collection_type != mongodb::results::CollectionType::Collection
        || spec.options.timeseries.is_some()
    {
        return Err(Error::Parse(
            "Sync targets must be regular collections, not views or time-series collections".into(),
        ));
    }
    Ok(())
}

pub async fn supports_sync(client: &mongodb::Client) -> Result<bool> {
    let hello = client
        .database("admin")
        .run_command(doc! {"hello": 1})
        .selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::Primary))
        .await?;
    Ok(hello.get_i32("maxWireVersion").unwrap_or(0) >= 25)
}

fn key_document(row: &DiffRow, fields: &[String]) -> Result<RawDocumentBuf> {
    let document: Document = if fields.len() == 1 {
        doc! {&fields[0]: row.key.clone()}
    } else {
        fields
            .iter()
            .map(|field| {
                row.key
                    .as_document()
                    .and_then(|d| d.get(field))
                    .cloned()
                    .map(|value| (field.clone(), value))
                    .ok_or_else(|| Error::Parse("Incomplete compound match key".into()))
            })
            .collect::<Result<_>>()?
    };
    let raw = RawDocumentBuf::from_document(&document).map_err(bson_error)?;
    compare_keys(&raw, &raw)?;
    Ok(raw)
}

fn key_query(row: &DiffRow, fields: &[String]) -> Result<Document> {
    let key: Document = (&*key_document(row, fields)?).try_into().map_err(bson_error)?;
    // $eq keeps document-valued keys literal. Explicit null must not match missing fields.
    Ok(key
        .into_iter()
        .map(|(field, value)| (field, Bson::Document(doc! {"$eq": value, "$exists": true})))
        .collect())
}

fn bson_error(error: impl std::fmt::Display) -> Error {
    Error::Parse(format!("Invalid BSON in sync: {error}"))
}

#[derive(Default)]
struct Match {
    document: Option<RawDocumentBuf>,
    count: usize,
}

enum ReadFailure {
    TooLarge,
    Error(Error),
}
impl From<Error> for ReadFailure {
    fn from(error: Error) -> Self {
        Self::Error(error)
    }
}
impl From<mongodb::error::Error> for ReadFailure {
    fn from(error: mongodb::error::Error) -> Self {
        Self::Error(error.into())
    }
}

async fn read_batch(
    collection: &Collection<RawDocumentBuf>,
    items: &[SyncItem],
    fields: &[String],
) -> std::result::Result<Vec<Match>, ReadFailure> {
    let keys = items.iter().map(|item| key_document(&item.row, fields)).collect::<Result<_>>()?;
    let queries = items.iter().map(|item| key_query(&item.row, fields)).collect::<Result<_>>()?;
    read_documents(collection, keys, queries, fields).await
}

async fn read_documents(
    collection: &Collection<RawDocumentBuf>,
    keys: Vec<RawDocumentBuf>,
    queries: Vec<Document>,
    fields: &[String],
) -> std::result::Result<Vec<Match>, ReadFailure> {
    let count = keys.len();
    let mut keys: Vec<_> = keys.into_iter().enumerate().collect();
    keys.sort_by(|a, b| compare_keys(&a.1, &b.1).expect("keys validated before sorting"));
    if keys
        .windows(2)
        .any(|pair| compare_keys(&pair[0].1, &pair[1].1).is_ok_and(|order| order.is_eq()))
    {
        return Err(
            Error::Parse("A sync plan cannot contain the same match key twice".into()).into()
        );
    }
    let filter = if fields.len() == 1 {
        let values = queries
            .iter()
            .map(|q| q.get_document(&fields[0]).unwrap().get("$eq").unwrap().clone())
            .collect::<Vec<_>>();
        doc! {&fields[0]: {"$in": values, "$exists": true}}
    } else {
        doc! {"$or": queries}
    };
    if mongodb::bson::to_vec(&filter).map_err(bson_error)?.len() > 12 * 1024 * 1024 {
        return Err(ReadFailure::TooLarge);
    }
    // Read by key on BOTH sides, without the compare filter. This also detects new duplicates
    // and key/identity changes since the scan. Never treat a filtered-out document as absent.
    let mut cursor =
        collection.find(filter).collation(Collation::builder().locale("simple").build()).await?;
    let mut found: Vec<Match> = (0..count).map(|_| Match::default()).collect();
    let mut bytes = 0;
    while cursor.advance().await? {
        let document = cursor.current();
        let key = extract_key(document, fields)?;
        let position = keys
            .binary_search_by(|(_, expected)| {
                compare_keys(expected, &key).expect("extracted keys are orderable")
            })
            .map_err(|_| {
                Error::Parse("The server returned a key outside this sync batch".into())
            })?;
        let found = &mut found[keys[position].0];
        found.count += 1;
        if let Some(previous) = found.document.take() {
            bytes -= previous.as_bytes().len();
        }
        if found.count == 1 {
            bytes += document.as_bytes().len();
            if bytes > READ_BYTES {
                return Err(ReadFailure::TooLarge);
            }
            found.document = Some(document.to_raw_document_buf());
        }
    }
    Ok(found)
}

fn matches_scan(found: &Match, expected_count: u64, expected_hash: u64) -> bool {
    found.count as u64 == expected_count
        && match &found.document {
            Some(document) => raw_hash(document) == expected_hash,
            None => expected_count == 0,
        }
}

fn id(document: &RawDocument) -> Result<Bson> {
    document
        .get("_id")
        .map_err(bson_error)?
        .ok_or_else(|| {
            Error::Parse("A document has no _id; it cannot be inserted or restored safely".into())
        })?
        .try_into()
        .map_err(bson_error)
}

fn replacement(source: &RawDocument, target_id: &Bson) -> Result<RawDocumentBuf> {
    // MongoDB stores _id first. Hash the exact representation that will be written.
    let mut result = RawDocumentBuf::from_document(&doc! {"_id": target_id}).map_err(bson_error)?;
    for element in source {
        let (key, value) = element.map_err(bson_error)?;
        if key != "_id" {
            result.append(key, value.to_raw_bson());
        }
    }
    Ok(result)
}

/// The target as it is, with one path set to the source's value, or removed if the source lacks it.
pub fn field_copy(
    source: &RawDocument,
    target: &RawDocument,
    path: &[PathSegment],
) -> Result<RawDocumentBuf> {
    let source: Document = source.try_into().map_err(bson_error)?;
    let mut target = faithful_document(target)?;
    let copied = match get_bson_at_path(&source, path) {
        Some(value) => set_bson_at_path(&mut target, path, value.clone()),
        None => remove_bson_at_path(&mut target, path),
    };
    if !copied {
        return Err(Error::Parse(
            "The target lacks this field's parent or array item; copy the parent instead".into(),
        ));
    }
    RawDocumentBuf::from_document(&target).map_err(bson_error)
}

/// Why this path of two loaded documents cannot be copied into `target`, without cloning either.
pub fn field_copy_check(
    source: &Document,
    target: &Document,
    path: &[PathSegment],
    fields: &[String],
) -> Option<&'static str> {
    if let Some(reason) = field_copy_refusal(path, fields) {
        return Some(reason);
    }
    let value = get_bson_at_path(source, path);
    if value == get_bson_at_path(target, path) {
        return Some("Already the same on both sides");
    }
    let (last, parent) = path.split_last()?;
    // None is the document itself, which always exists.
    let parent = (!parent.is_empty()).then(|| get_bson_at_path(target, parent));
    match (last, parent) {
        (PathSegment::Key(_), None | Some(Some(Bson::Document(_)))) => None,
        (PathSegment::Key(_), _) => {
            Some("The target lacks this field's parent; copy the parent instead")
        }
        (PathSegment::Index(index), Some(Some(Bson::Array(items))))
            if *index < items.len() && value.is_some() =>
        {
            None
        }
        (PathSegment::Index(_), _) => Some("Copy the whole array instead"),
    }
}

/// Why a path cannot be copied on its own: it would change how documents are matched.
pub fn field_copy_refusal(path: &[PathSegment], fields: &[String]) -> Option<&'static str> {
    let label = crate::bson::dotted_path(path);
    let touches = |field: &str| {
        label == field
            || label.starts_with(&format!("{field}."))
            || field.starts_with(&format!("{label}."))
    };
    if path.is_empty() || touches("_id") {
        Some("_id is never copied")
    } else if fields.iter().any(|field| touches(field)) {
        Some("Match fields are not copied; they decide which documents pair up")
    } else {
        None
    }
}

struct Prepared {
    row: usize,
    operation: Operation,
    id: Bson,
    before: Option<RawDocumentBuf>,
    after: Option<RawDocumentBuf>,
}

impl Prepared {
    fn undo_record(&self) -> UndoRecord {
        UndoRecord {
            row: self.row,
            operation: self.operation,
            id: self.id.clone(),
            before: self.before.clone(),
            after_hash: self.after.as_ref().map(|document| fingerprint(document)),
        }
    }
}

fn prepare_item(
    item: &SyncItem,
    left: &Match,
    right: &Match,
    target: Side,
) -> std::result::Result<Prepared, RowOutcome> {
    if !matches_scan(left, item.row.left_count, item.row.left_hash)
        || !matches_scan(right, item.row.right_count, item.row.right_hash)
    {
        return Err(RowOutcome::Skipped(
            "Changed since the comparison, or the key now has multiple matches".into(),
        ));
    }
    let (source, destination) = match target {
        Side::Left => (right, left),
        Side::Right => (left, right),
    };
    let result = (|| -> Result<Prepared> {
        let before = destination.document.clone();
        let target_id = match item.operation {
            Operation::Insert => id(source
                .document
                .as_ref()
                .ok_or_else(|| Error::Parse("Source document disappeared".into()))?)?,
            _ => id(before
                .as_ref()
                .ok_or_else(|| Error::Parse("Target document disappeared".into()))?)?,
        };
        let after = if item.operation == Operation::Delete {
            None
        } else if let Some(path) = &item.field {
            Some(field_copy(
                source
                    .document
                    .as_ref()
                    .ok_or_else(|| Error::Parse("Source document disappeared".into()))?,
                before
                    .as_ref()
                    .ok_or_else(|| Error::Parse("Target document disappeared".into()))?,
                path,
            )?)
        } else {
            Some(replacement(
                source
                    .document
                    .as_ref()
                    .ok_or_else(|| Error::Parse("Source document disappeared".into()))?,
                &target_id,
            )?)
        };
        Ok(Prepared {
            row: item.row_index,
            operation: item.operation,
            id: target_id,
            before,
            after,
        })
    })();
    let prepared = result.map_err(|error| RowOutcome::Failed(error.to_string()))?;
    if prepared.before.as_ref().map(|d| d.as_bytes())
        == prepared.after.as_ref().map(|d| d.as_bytes())
    {
        return Err(RowOutcome::Skipped("Target already matches while preserving its _id".into()));
    }
    Ok(prepared)
}

fn current_filter(id: &Bson, expected: &RawDocument) -> Result<Document> {
    let expected = faithful_document(expected)?;
    Ok(doc! {"_id": {"$eq": id}, "$expr": {"$eq": ["$$ROOT", {"$literal": expected}]}})
}

fn faithful_document(raw: &RawDocument) -> Result<Document> {
    let document: Document = raw.try_into().map_err(bson_error)?;
    if RawDocumentBuf::from_document(&document).map_err(bson_error)?.as_bytes() != raw.as_bytes() {
        return Err(Error::Parse("This BSON document cannot be written without changing its representation (for example, duplicate field names)".into()));
    }
    Ok(document)
}

fn write_model(target: &Collection<RawDocumentBuf>, write: &Prepared) -> Result<WriteModel> {
    let namespace = target.namespace();
    let document = |raw: Option<&RawDocumentBuf>| -> Result<Document> {
        faithful_document(raw.ok_or_else(|| Error::Parse("Incomplete sync operation".into()))?)
    };
    Ok(match write.operation {
        Operation::Insert => InsertOneModel::builder()
            .namespace(namespace)
            .document(document(write.after.as_ref())?)
            .build()
            .into(),
        Operation::Replace => ReplaceOneModel::builder()
            .namespace(namespace)
            .filter(current_filter(
                &write.id,
                write.before.as_ref().ok_or_else(|| Error::Parse("Missing target image".into()))?,
            )?)
            .replacement(document(write.after.as_ref())?)
            .upsert(false)
            .collation(doc! {"locale": "simple"})
            .build()
            .into(),
        Operation::Delete => DeleteOneModel::builder()
            .namespace(namespace)
            .filter(current_filter(
                &write.id,
                write.before.as_ref().ok_or_else(|| Error::Parse("Missing target image".into()))?,
            )?)
            .collation(doc! {"locale": "simple"})
            .build()
            .into(),
    })
}

fn bulk_outcome(
    result: &mongodb::error::Result<VerboseBulkWriteResult>,
    index: usize,
    operation: Operation,
    undo: bool,
) -> RowOutcome {
    let verbose = match result {
        Ok(result) => Some(result),
        Err(error) => match error.kind.as_ref() {
            ErrorKind::BulkWrite(error) => {
                if let Some(write_error) = error.write_errors.get(&index) {
                    return RowOutcome::Failed(write_error.message.clone());
                }
                if !error.write_concern_errors.is_empty() {
                    None
                } else if let Some(PartialBulkWriteResult::Verbose(result)) = &error.partial_result
                {
                    Some(result)
                } else {
                    None
                }
            }
            // A top-level error may follow an earlier driver-split batch. Keep its log.
            _ => None,
        },
    };
    let matched = verbose.and_then(|result| match operation {
        Operation::Insert => result.insert_results.get(&index).map(|_| 1),
        Operation::Replace => result.update_results.get(&index).map(|r| r.matched_count),
        Operation::Delete => result.delete_results.get(&index).map(|r| r.deleted_count),
    });
    match matched {
        Some(1) => {
            if undo {
                RowOutcome::Restored
            } else {
                RowOutcome::Written
            }
        }
        Some(_) => RowOutcome::Skipped("Target changed while this batch was being written".into()),
        None => RowOutcome::Uncertain(format!(
            "Write acknowledgement unavailable; guarded undo retained. {}",
            result.as_ref().err().map(ToString::to_string).unwrap_or_default()
        )),
    }
}

async fn execute_batch(
    target: &Collection<RawDocumentBuf>,
    prepared: Vec<(usize, Prepared)>,
    restore: &Arc<RestoreHandle>,
    undo: bool,
) -> Result<Vec<(usize, Operation, RowOutcome)>> {
    if prepared.is_empty() {
        return Ok(Vec::new());
    }
    let models: Vec<_> =
        prepared.iter().map(|(_, write)| write_model(target, write)).collect::<Result<_>>()?;
    for (entry, _) in &prepared {
        if !undo {
            restore.started(*entry)?;
        }
    }
    let mut request = target.client().bulk_write(models).ordered(false).verbose_results();
    if let Some(concern) = target.write_concern() {
        request = request.write_concern(concern.clone());
    }
    let result = request.await;
    prepared
        .into_iter()
        .enumerate()
        .map(|(index, (entry, write))| {
            let outcome = bulk_outcome(&result, index, write.operation, undo);
            if undo {
                if matches!(outcome, RowOutcome::Restored) {
                    restore.inactive(entry)?;
                }
            } else {
                match outcome {
                    RowOutcome::Written => restore.confirmed(entry)?,
                    RowOutcome::Skipped(_) | RowOutcome::Failed(_) => restore.inactive(entry)?,
                    _ => {}
                }
            }
            Ok::<_, Error>((write.row, write.operation, outcome))
        })
        .collect()
}

fn publish(
    summary: &mut SyncSummary,
    results: Vec<(usize, Operation, RowOutcome)>,
    sender: &UnboundedSender<SyncProgress>,
) {
    let mut outcomes = Vec::with_capacity(results.len());
    for (row, operation, outcome) in results {
        summary.record(operation, &outcome);
        outcomes.push((row, outcome));
    }
    let _ = sender.unbounded_send(SyncProgress { summary: summary.clone(), outcomes });
}

/// Reads and logs at most 1,000 rows per batch, splitting large BSON batches by a byte budget.
pub async fn sync_collections_async(
    sides: [Collection<RawDocumentBuf>; 2],
    target: Side,
    fields: Vec<String>,
    items: Vec<SyncItem>,
    restore: Arc<RestoreHandle>,
    cancellation: CancellationToken,
    sender: UnboundedSender<SyncProgress>,
) -> Result<SyncSummary> {
    super::compare::CompareOptions { fields: fields.clone(), ..Default::default() }.validate()?;
    for item in &items {
        if operation_for(item.row.kind, target) != Some(item.operation) {
            return Err(Error::Parse("Sync plan does not match its comparison rows".into()));
        }
        if let Some(path) = &item.field {
            if item.operation != Operation::Replace {
                return Err(Error::Parse("Only a document on both sides can take a field".into()));
            }
            if let Some(reason) = field_copy_refusal(path, &fields) {
                return Err(Error::Parse(reason.into()));
            }
        }
    }
    let sides = sides.each_ref().map(primary);
    let destination = &sides[if target == Side::Left { 0 } else { 1 }];
    validate_target(destination).await?;
    let mut batches: VecDeque<_> = (0..items.len())
        .step_by(BATCH_ROWS)
        .map(|start| start..(start + BATCH_ROWS).min(items.len()))
        .collect();
    let mut summary = SyncSummary::default();
    while let Some(range) = batches.pop_front() {
        if cancellation.is_cancelled() {
            summary.cancelled = true;
            break;
        }
        let batch = &items[range.clone()];
        let reads = tokio::try_join!(
            read_batch(&sides[0], batch, &fields),
            read_batch(&sides[1], batch, &fields)
        );
        let (left, right) = match reads {
            Ok(reads) => reads,
            Err(ReadFailure::TooLarge) if range.len() > 1 => {
                let middle = range.start + range.len() / 2;
                batches.push_front(middle..range.end);
                batches.push_front(range.start..middle);
                continue;
            }
            Err(ReadFailure::TooLarge) => {
                return Err(Error::Parse("A match key exceeds the safe sync read budget".into()));
            }
            Err(ReadFailure::Error(error)) => return Err(error),
        };
        let mut prepared = Vec::new();
        let mut outcomes = Vec::new();
        for ((item, left), right) in batch.iter().zip(&left).zip(&right) {
            match prepare_item(item, left, right, target) {
                Ok(write) => prepared.push(write),
                Err(outcome) => outcomes.push((item.row_index, item.operation, outcome)),
            }
        }
        if cancellation.is_cancelled() {
            summary.cancelled = true;
            break;
        }
        let records: Vec<_> = prepared.iter().map(Prepared::undo_record).collect();
        let log = restore.clone();
        let entries = tokio::task::spawn_blocking(move || log.prepare(&records))
            .await
            .map_err(|e| Error::Parse(format!("Undo logging failed: {e}")))??;
        if cancellation.is_cancelled() {
            summary.cancelled = true;
            break;
        }
        outcomes.extend(
            execute_batch(
                destination,
                entries.into_iter().zip(prepared).collect(),
                &restore,
                false,
            )
            .await?,
        );
        publish(&mut summary, outcomes, &sender);
        if summary.uncertain > 0 {
            break;
        }
    }
    summary.cancelled |= cancellation.is_cancelled();
    Ok(summary)
}

async fn read_undo_batch(
    target: &Collection<RawDocumentBuf>,
    records: &[(usize, UndoRecord)],
) -> std::result::Result<Vec<Match>, ReadFailure> {
    // Undo uses exact BSON identities; _id may be a type not supported as a comparison key.
    let keys: HashMap<Vec<u8>, usize> = records
        .iter()
        .enumerate()
        .map(|(index, (_, record))| {
            mongodb::bson::to_vec(&doc! {"_id": &record.id})
                .map(|key| (key, index))
                .map_err(bson_error)
        })
        .collect::<Result<_>>()?;
    let filter =
        doc! {"_id": {"$in": records.iter().map(|(_, r)| r.id.clone()).collect::<Vec<_>>()}};
    if mongodb::bson::to_vec(&filter).map_err(bson_error)?.len() > 12 * 1024 * 1024 {
        return Err(ReadFailure::TooLarge);
    }
    let mut cursor =
        target.find(filter).collation(Collation::builder().locale("simple").build()).await?;
    let mut found: Vec<Match> = (0..records.len()).map(|_| Match::default()).collect();
    let mut bytes = 0;
    while cursor.advance().await? {
        let document = cursor.current();
        let raw_id = document
            .get("_id")
            .map_err(bson_error)?
            .ok_or_else(|| Error::Parse("Missing target _id".into()))?;
        let mut key = RawDocumentBuf::new();
        key.append("_id", raw_id.to_raw_bson());
        // A different BSON identity must not pass the after-image guard. An insert cannot
        // overwrite a numerically equivalent _id: MongoDB's unique constraint rejects it.
        let Some(index) = keys.get(key.as_bytes()) else {
            continue;
        };
        bytes += document.as_bytes().len();
        if bytes > READ_BYTES {
            return Err(ReadFailure::TooLarge);
        }
        found[*index] = Match { count: 1, document: Some(document.to_raw_document_buf()) };
    }
    Ok(found)
}

pub async fn undo_sync_async(
    target: Collection<RawDocumentBuf>,
    restore: Arc<RestoreHandle>,
    cancellation: CancellationToken,
    sender: UnboundedSender<SyncProgress>,
) -> Result<SyncSummary> {
    let target = primary(&target);
    validate_target(&target).await?;
    let mut cursor = 0;
    let mut summary = SyncSummary::default();
    loop {
        if cancellation.is_cancelled() {
            summary.cancelled = true;
            break;
        }
        let log = restore.clone();
        let (next, records) = tokio::task::spawn_blocking(move || log.read_pending(cursor))
            .await
            .map_err(|e| Error::Parse(format!("Cannot read undo log: {e}")))??;
        cursor = next;
        if records.is_empty() {
            break;
        }
        let mut batches = VecDeque::from([records]);
        while let Some(mut records) = batches.pop_front() {
            if cancellation.is_cancelled() {
                summary.cancelled = true;
                break;
            }
            let current = match read_undo_batch(&target, &records).await {
                Ok(current) => current,
                Err(ReadFailure::TooLarge) if records.len() > 1 => {
                    let second = records.split_off(records.len() / 2);
                    batches.push_front(second);
                    batches.push_front(records);
                    continue;
                }
                Err(ReadFailure::TooLarge) => {
                    return Err(Error::Parse(
                        "Target document exceeds the undo read budget".into(),
                    ));
                }
                Err(ReadFailure::Error(error)) => return Err(error),
            };
            let mut ready = Vec::new();
            let mut results = Vec::new();
            for ((entry, record), current) in records.into_iter().zip(current) {
                let current = current.document;
                let operation = match record.operation {
                    Operation::Insert => Operation::Delete,
                    Operation::Replace => Operation::Replace,
                    Operation::Delete => Operation::Insert,
                };
                if current.as_ref().map(|d| fingerprint(d)) != record.after_hash {
                    // A previously interrupted undo may already have restored the before-image.
                    if current.as_ref().map(|d| d.as_bytes())
                        == record.before.as_ref().map(|d| d.as_bytes())
                    {
                        restore.inactive(entry)?;
                    }
                    results.push((
                        record.row,
                        operation,
                        RowOutcome::Skipped(
                            "Target changed since sync, or is already restored".into(),
                        ),
                    ));
                    continue;
                }
                ready.push((
                    entry,
                    Prepared {
                        row: record.row,
                        operation,
                        id: record.id,
                        before: current,
                        after: record.before,
                    },
                ));
            }
            if cancellation.is_cancelled() {
                summary.cancelled = true;
                break;
            }
            results.extend(execute_batch(&target, ready, &restore, true).await?);
            publish(&mut summary, results, &sender);
            if summary.uncertain > 0 {
                break;
            }
        }
        if summary.cancelled || summary.uncertain > 0 {
            break;
        }
    }
    summary.cancelled |= cancellation.is_cancelled();
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_bulk_acknowledgements_are_uncertain_and_zero_matches_are_skipped() {
        let mut result = VerboseBulkWriteResult::default();
        result.update_results.insert(0, mongodb::results::UpdateResult::default());
        let result = Ok(result);
        assert!(matches!(
            bulk_outcome(&result, 0, Operation::Replace, false),
            RowOutcome::Skipped(_)
        ));
        assert!(matches!(
            bulk_outcome(&result, 1, Operation::Replace, false),
            RowOutcome::Uncertain(_)
        ));
    }

    #[test]
    fn bson_roundtrip_cannot_silently_drop_duplicate_fields() {
        let mut raw = RawDocumentBuf::new();
        raw.append("_id", 1);
        raw.append("value", 1);
        raw.append("value", 2);
        assert!(faithful_document(&raw).is_err());
    }
}
