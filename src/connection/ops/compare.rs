//! Read-only, keyed collection comparison. Call on ConnectionManager::runtime_handle().

use std::cmp::Ordering;
use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

use futures::{TryStreamExt, channel::mpsc::UnboundedSender};
use mongodb::bson::{Bson, Document, RawBsonRef, RawDocument, RawDocumentBuf, doc};
use mongodb::options::{Collation, FindOptions};
use mongodb::results::CollectionType;
use mongodb::{Collection, IndexModel};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::bson::compare::{IgnoreSet, Verdict, cmp_key_value, compare_raw, key_value};
use crate::connection::CancellationToken;
use crate::error::{Error, Result};

const CHUNK_DOCUMENTS: usize = 1_024;
const CHUNK_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ROWS: usize = 250_000;
const MAX_ROW_BYTES: usize = 64 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const SESSION_REFRESH: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    OnlyLeft,
    OnlyRight,
    Different,
    Minor,
    MultipleMatches,
}

#[derive(Clone, Debug)]
pub struct DiffRow {
    pub key: Bson,
    /// None for the _id shortcut, an absent side, or a view that projects _id away.
    /// The side's count distinguishes an absent document from a document without _id.
    pub left_id: Option<Bson>,
    pub right_id: Option<Bson>,
    pub kind: DiffKind,
    pub changed: u16,
    pub paths: Box<str>,
    pub left_hash: u64,
    pub right_hash: u64,
    pub left_count: u64,
    pub right_count: u64,
}

impl DiffRow {
    pub fn id_on(&self, side: Side, key_is_id: bool) -> Option<&Bson> {
        let count = match side {
            Side::Left => self.left_count,
            Side::Right => self.right_count,
        };
        if self.kind == DiffKind::MultipleMatches || count == 0 {
            return None;
        }
        if key_is_id {
            return Some(&self.key);
        }
        match side {
            Side::Left => self.left_id.as_ref(),
            Side::Right => self.right_id.as_ref(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompareCounts {
    pub left_read: u64,
    pub right_read: u64,
    pub identical: u64,
    pub only_left: u64,
    pub only_right: u64,
    pub different: u64,
    pub minor: u64,
    pub multiple_matches: u64,
}

#[derive(Clone, Debug)]
pub struct CompareSummary {
    pub counts: CompareCounts,
    /// None if cancellation interrupted the skipped-document counts.
    pub skipped: Option<[u64; 2]>,
    pub truncated: bool,
    pub cancelled: bool,
    pub elapsed: Duration,
}

#[derive(Debug)]
pub enum CompareMessage {
    Prepared {
        sort: SortPlan,
        estimated: [Option<u64>; 2],
        simple_collation_forced: [bool; 2],
    },
    Progress {
        counts: CompareCounts,
        new_rows: Vec<DiffRow>,
        left_started: bool,
        right_started: bool,
    },
    Done(CompareSummary),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct CompareOptions {
    pub fields: Vec<String>,
    pub filter: Document,
    pub ignore: IgnoreSet,
    /// Can be lowered by consumers; never exceeds MAX_ROWS.
    pub row_limit: usize,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            fields: vec!["_id".into()],
            filter: Document::new(),
            ignore: IgnoreSet::default(),
            row_limit: MAX_ROWS,
        }
    }
}

impl CompareOptions {
    pub fn validate(&self) -> Result<()> {
        if self.fields.is_empty() || self.fields.len() > 32 {
            return Err(Error::Parse("Choose between 1 and 32 match fields".into()));
        }
        for (i, field) in self.fields.iter().enumerate() {
            if field
                .split('.')
                .any(|part| part.is_empty() || part.starts_with('$') || part.contains('\0'))
            {
                return Err(Error::Parse(format!("Invalid match field: {field}")));
            }
            if self.fields[..i].contains(field) {
                return Err(Error::Parse(format!("Repeated match field: {field}")));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortPlan {
    pub fields: Vec<String>,
    pub left_covered: bool,
    pub right_covered: bool,
}

fn index_order(fields: &[String], index: &IndexModel) -> Option<Vec<String>> {
    if fields.is_empty() {
        return None;
    }
    if let Some(options) = &index.options
        && (options.sparse == Some(true)
            || options.hidden == Some(true)
            || options.partial_filter_expression.is_some()
            || options.collation.as_ref().is_some_and(|c| c.locale != "simple"))
    {
        return None;
    }
    let prefix: Vec<_> = index.keys.iter().take(fields.len()).collect();
    if prefix.len() != fields.len() || prefix.iter().any(|(field, _)| !fields.contains(field)) {
        return None;
    }
    let direction = |value: &Bson| match value {
        Bson::Int32(1) | Bson::Int64(1) => Some(1),
        Bson::Int32(-1) | Bson::Int64(-1) => Some(-1),
        Bson::Double(v) if *v == 1.0 => Some(1),
        Bson::Double(v) if *v == -1.0 => Some(-1),
        _ => None,
    };
    let first = direction(prefix[0].1)?;
    if !prefix.iter().all(|(_, value)| direction(value) == Some(first)) {
        return None;
    }
    Some(prefix.into_iter().map(|(field, _)| field.clone()).collect())
}

/// Prefer a common order, then the left side's order, then the right, then as typed.
/// The scan passes the larger side first when estimates are available.
pub fn sort_plan(fields: &[String], left: &[IndexModel], right: &[IndexModel]) -> SortPlan {
    let left: Vec<_> = left.iter().filter_map(|i| index_order(fields, i)).collect();
    let right: Vec<_> = right.iter().filter_map(|i| index_order(fields, i)).collect();
    let order = left
        .iter()
        .find(|order| right.contains(order))
        .or_else(|| left.first())
        .or_else(|| right.first())
        .cloned()
        .unwrap_or_else(|| fields.to_vec());
    SortPlan {
        left_covered: left.contains(&order),
        right_covered: right.contains(&order),
        fields: order,
    }
}

/// Eligible and skipped predicates are complements, including explicit null as a valid key.
pub fn key_filters(filter: &Document, fields: &[String]) -> (Document, Option<Document>) {
    if fields == ["_id"] {
        return (filter.clone(), None);
    }
    let eligible: Vec<_> = fields
        .iter()
        .map(|field| doc! {field: {"$exists": true, "$not": {"$type": "array"}}})
        .collect();
    let skipped: Vec<_> = fields
        .iter()
        .flat_map(|field| [doc! {field: {"$exists": false}}, doc! {field: {"$type": "array"}}])
        .collect();
    (
        doc! {"$and": [filter.clone(), doc! {"$and": eligible}]},
        Some(doc! {"$and": [filter.clone(), doc! {"$or": skipped}]}),
    )
}

struct Metadata {
    indexes: Vec<IndexModel>,
    estimated: Option<u64>,
    force_simple: bool,
}

async fn metadata(collection: &Collection<RawDocumentBuf>) -> Result<Metadata> {
    let namespace = collection.namespace();
    let database = collection.client().database(&namespace.db);
    let spec = database
        .list_collections()
        .filter(doc! {"name": &namespace.coll})
        .await?
        .try_next()
        .await?
        .ok_or_else(|| Error::Parse(format!("Collection {namespace} no longer exists")))?;
    if spec.options.timeseries.is_some() || spec.collection_type == CollectionType::Timeseries {
        return Err(Error::Parse("Time-series collections cannot be compared yet".into()));
    }
    let force_simple = spec.options.collation.as_ref().is_some_and(|c| c.locale != "simple");
    if spec.collection_type == CollectionType::View {
        if force_simple {
            return Err(Error::Parse("This view has a non-simple collation, which MongoDB does not allow a comparison to override".into()));
        }
        return Ok(Metadata { indexes: Vec::new(), estimated: None, force_simple });
    }
    let indexes = collection.list_indexes().await?.try_collect().await?;
    // An estimate is advisory, and can be unavailable with restricted permissions.
    let estimated = collection.estimated_document_count().await.ok();
    Ok(Metadata { indexes, estimated, force_simple })
}

pub(crate) fn extract_key(document: &RawDocument, fields: &[String]) -> Result<RawDocumentBuf> {
    let mut key = RawDocumentBuf::new();
    for field in fields {
        let value = key_value(document, field)?.ok_or_else(|| {
            Error::Parse(format!("Match field {field} disappeared during the scan"))
        })?;
        if matches!(value, RawBsonRef::Array(_))
            || crate::bson::compare::cmp_key_value(value, value).is_none()
        {
            return Err(Error::Parse(format!(
                "Cannot safely order match field {field} ({:?}); choose another key",
                value.element_type()
            )));
        }
        key.append(field, value.to_raw_bson());
    }
    Ok(key)
}

pub(crate) fn compare_keys(a: &RawDocument, b: &RawDocument) -> Result<Ordering> {
    let mut a = a.iter();
    let mut b = b.iter();
    loop {
        match (a.next(), b.next()) {
            (None, None) => return Ok(Ordering::Equal),
            (Some(_), None) => return Ok(Ordering::Greater),
            (None, Some(_)) => return Ok(Ordering::Less),
            (Some(a), Some(b)) => {
                let (_, a) = a.map_err(|e| Error::Parse(e.to_string()))?;
                let (_, b) = b.map_err(|e| Error::Parse(e.to_string()))?;
                let order = cmp_key_value(a, b)
                    .ok_or_else(|| Error::Parse("The match key cannot be ordered safely".into()))?;
                if !order.is_eq() {
                    return Ok(order);
                }
            }
        }
    }
}

async fn check_array_paths(
    collection: &Collection<RawDocumentBuf>,
    options: &CompareOptions,
) -> Result<()> {
    // Check BEFORE the key predicates: they could otherwise hide an array traversal entirely.
    for field in &options.fields {
        for (end, _) in field.match_indices('.') {
            let prefix = &field[..end];
            if collection
                .find_one(doc! {"$and": [&options.filter, &doc! {prefix: {"$type": "array"}}]})
                .projection(doc! {"_id": 1})
                .collation(Collation::builder().locale("simple").build())
                .await?
                .is_some()
            {
                return Err(Error::Parse(format!(
                    "{field} passes through an array in some documents, so it cannot be used to match"
                )));
            }
        }
    }
    Ok(())
}

async fn read_side(
    collection: Collection<RawDocumentBuf>,
    options: CompareOptions,
    sort: Document,
    covered: bool,
    sender: mpsc::Sender<Vec<RawDocumentBuf>>,
    read: Arc<AtomicU64>,
) -> Result<u64> {
    check_array_paths(&collection, &options).await?;
    let (filter, skipped) = key_filters(&options.filter, &options.fields);
    let count = async {
        match skipped {
            Some(filter) => Ok::<_, Error>(
                collection
                    .count_documents(filter)
                    .collation(Collation::builder().locale("simple").build())
                    .await?,
            ),
            None => Ok(0),
        }
    };
    let scan = async {
        let mut session = collection.client().start_session().await?;
        let session_id = session.id().clone();
        let admin = collection.client().database("admin");
        let read = async {
            let mut find_options = FindOptions::builder()
                .sort(sort)
                .collation(Collation::builder().locale("simple").build())
                .build();
            if !covered {
                find_options.allow_disk_use = Some(true);
            }
            let mut cursor =
                collection.find(filter).with_options(find_options).session(&mut session).await?;
            let mut chunk = Vec::with_capacity(CHUNK_DOCUMENTS);
            let mut bytes = 0;
            while cursor.advance(&mut session).await? {
                let document = cursor.current().to_raw_document_buf();
                bytes += document.as_bytes().len();
                chunk.push(document);
                read.fetch_add(1, AtomicOrdering::Relaxed);
                if chunk.len() >= CHUNK_DOCUMENTS || bytes >= CHUNK_BYTES {
                    sender
                        .send(std::mem::take(&mut chunk))
                        .await
                        .map_err(|_| Error::Cancelled("Comparison closed".into()))?;
                    bytes = 0;
                }
            }
            if !chunk.is_empty() {
                sender
                    .send(chunk)
                    .await
                    .map_err(|_| Error::Cancelled("Comparison closed".into()))?;
            }
            Ok::<_, Error>(())
        };
        tokio::pin!(read);
        let mut refresh = tokio::time::interval_at(
            tokio::time::Instant::now() + SESSION_REFRESH,
            SESSION_REFRESH,
        );
        loop {
            tokio::select! {
                result = &mut read => break result,
                _ = refresh.tick() => {
                    // Explicit sessions avoid cursor idle expiry. Refresh the session even when
                    // backpressure prevents getMore; draining buffered documents is not keepalive.
                    admin.run_command(doc! {"refreshSessions": [session_id.clone()]}).await?;
                }
            }
        }
    };
    let (skipped, ()) = tokio::try_join!(count, scan)?;
    Ok(skipped)
}

#[derive(Debug)]
struct Group {
    key: RawDocumentBuf,
    document: Option<RawDocumentBuf>,
    count: u64,
}

struct Groups {
    receiver: mpsc::Receiver<Vec<RawDocumentBuf>>,
    chunk: VecDeque<RawDocumentBuf>,
    pending: Option<(RawDocumentBuf, RawDocumentBuf)>,
    previous: Option<RawDocumentBuf>,
    fields: Vec<String>,
}

impl Groups {
    fn new(receiver: mpsc::Receiver<Vec<RawDocumentBuf>>, fields: Vec<String>) -> Self {
        Self { receiver, chunk: VecDeque::new(), pending: None, previous: None, fields }
    }

    async fn next_document(&mut self) -> Result<Option<(RawDocumentBuf, RawDocumentBuf)>> {
        loop {
            if let Some(document) = self.chunk.pop_front() {
                let key = extract_key(&document, &self.fields)?;
                if let Some(previous) = &self.previous
                    && compare_keys(previous, &key)?.is_gt()
                {
                    return Err(Error::Parse("The server returned match keys out of order; comparison stopped to avoid incorrect results".into()));
                }
                self.previous = Some(key.clone());
                return Ok(Some((key, document)));
            }
            match self.receiver.recv().await {
                Some(chunk) => self.chunk = chunk.into(),
                None => return Ok(None),
            }
        }
    }

    async fn next(&mut self) -> Result<Option<Group>> {
        let first = match self.pending.take() {
            Some(value) => Some(value),
            None => self.next_document().await?,
        };
        let Some((key, document)) = first else {
            return Ok(None);
        };
        let mut group = Group { key, document: Some(document), count: 1 };
        while let Some((key, document)) = self.next_document().await? {
            if compare_keys(&group.key, &key)?.is_eq() {
                group.count += 1;
                group.document = None;
                if group.count.is_multiple_of(CHUNK_DOCUMENTS as u64) {
                    tokio::task::yield_now().await;
                }
            } else {
                self.pending = Some((key, document));
                break;
            }
        }
        Ok(Some(group))
    }
}

struct Reporter<'a> {
    sender: &'a UnboundedSender<CompareMessage>,
    counts: CompareCounts,
    read: [Arc<AtomicU64>; 2],
    rows: Vec<DiffRow>,
    stored: usize,
    stored_bytes: usize,
    limit: usize,
    truncated: bool,
    last_progress: Instant,
}

impl Reporter<'_> {
    fn progress(&mut self) -> Result<()> {
        self.counts.left_read = self.read[0].load(AtomicOrdering::Relaxed);
        self.counts.right_read = self.read[1].load(AtomicOrdering::Relaxed);
        self.sender
            .unbounded_send(CompareMessage::Progress {
                counts: self.counts,
                new_rows: std::mem::take(&mut self.rows),
                left_started: self.counts.left_read > 0,
                right_started: self.counts.right_read > 0,
            })
            .map_err(|_| Error::Cancelled("Comparison closed".into()))?;
        self.last_progress = Instant::now();
        Ok(())
    }

    async fn next(&mut self, groups: &mut Groups) -> Result<Option<Group>> {
        let next = groups.next();
        tokio::pin!(next);
        loop {
            tokio::select! {
                result = &mut next => return result,
                _ = tokio::time::sleep(PROGRESS_INTERVAL.saturating_sub(self.last_progress.elapsed())) => self.progress()?,
            }
        }
    }

    fn record(
        &mut self,
        left: Option<&Group>,
        right: Option<&Group>,
        ignore: &IgnoreSet,
        key_is_id: bool,
    ) -> Result<()> {
        let group = left.or(right).expect("at least one group");
        let (kind, changed, paths) =
            if left.is_some_and(|g| g.count > 1) || right.is_some_and(|g| g.count > 1) {
                (DiffKind::MultipleMatches, 0, Box::<str>::default())
            } else {
                match (left, right) {
                    (Some(left), Some(right)) => match compare_raw(
                        left.document.as_ref().unwrap(),
                        right.document.as_ref().unwrap(),
                        ignore,
                    )? {
                        Verdict::Same => {
                            self.counts.identical += 1;
                            return Ok(());
                        }
                        Verdict::Minor(_) => (DiffKind::Minor, 0, Box::<str>::default()),
                        Verdict::Different { changed, first_paths } => {
                            (DiffKind::Different, changed, first_paths)
                        }
                    },
                    (Some(_), None) => (DiffKind::OnlyLeft, 0, Box::<str>::default()),
                    (None, Some(_)) => (DiffKind::OnlyRight, 0, Box::<str>::default()),
                    (None, None) => unreachable!(),
                }
            };
        match kind {
            DiffKind::OnlyLeft => self.counts.only_left += 1,
            DiffKind::OnlyRight => self.counts.only_right += 1,
            DiffKind::Different => self.counts.different += 1,
            DiffKind::Minor => self.counts.minor += 1,
            DiffKind::MultipleMatches => self.counts.multiple_matches += 1,
        }
        // ponytail: cap retained rows and key/id bytes; spill to SQLite if larger result sets matter.
        let size = group.key.as_bytes().len() + paths.len() + std::mem::size_of::<DiffRow>();
        if self.truncated || self.stored >= self.limit || self.stored_bytes + size > MAX_ROW_BYTES {
            self.truncated = true;
            return Ok(());
        }
        let key_document: Document =
            (&*group.key).try_into().map_err(|e| Error::Parse(format!("Invalid key: {e}")))?;
        let key = if key_document.len() == 1 {
            key_document.into_iter().next().unwrap().1
        } else {
            Bson::Document(key_document)
        };
        let identity = |group: Option<&Group>| -> Result<(Option<Bson>, u64, usize)> {
            let Some(document) = group.and_then(|g| g.document.as_ref()) else {
                return Ok((None, 0, 0));
            };
            let id = if key_is_id {
                None
            } else {
                document
                    .get("_id")
                    .map_err(|e| Error::Parse(e.to_string()))?
                    .map(Bson::try_from)
                    .transpose()
                    .map_err(|e| Error::Parse(e.to_string()))?
            };
            let id_bytes = if let Some(id) = &id {
                mongodb::bson::to_vec(&doc! {"_id": id})
                    .map_err(|e| Error::Parse(e.to_string()))?
                    .len()
            } else {
                0
            };
            let mut hasher = DefaultHasher::new();
            document.as_bytes().hash(&mut hasher);
            Ok((id, hasher.finish(), id_bytes))
        };
        let (left_id, left_hash, left_bytes) = identity(left)?;
        let (right_id, right_hash, right_bytes) = identity(right)?;
        let size = size + left_bytes + right_bytes;
        if self.stored_bytes + size > MAX_ROW_BYTES {
            self.truncated = true;
            return Ok(());
        }
        self.rows.push(DiffRow {
            key,
            left_id,
            right_id,
            kind,
            changed,
            paths,
            left_hash,
            right_hash,
            left_count: left.map_or(0, |g| g.count),
            right_count: right.map_or(0, |g| g.count),
        });
        self.stored += 1;
        self.stored_bytes += size;
        Ok(())
    }
}

async fn merge(
    left: &mut Groups,
    right: &mut Groups,
    reporter: &mut Reporter<'_>,
    ignore: &IgnoreSet,
    key_is_id: bool,
) -> Result<()> {
    let mut a = reporter.next(left).await?;
    let mut b = reporter.next(right).await?;
    let mut groups = 0;
    while a.is_some() || b.is_some() {
        let order = match (&a, &b) {
            (Some(a), Some(b)) => compare_keys(&a.key, &b.key)?,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => unreachable!(),
        };
        match order {
            Ordering::Less => {
                reporter.record(a.as_ref(), None, ignore, key_is_id)?;
                a = reporter.next(left).await?;
            }
            Ordering::Greater => {
                reporter.record(None, b.as_ref(), ignore, key_is_id)?;
                b = reporter.next(right).await?;
            }
            Ordering::Equal => {
                reporter.record(a.as_ref(), b.as_ref(), ignore, key_is_id)?;
                a = reporter.next(left).await?;
                b = reporter.next(right).await?;
            }
        }
        groups += 1;
        if groups % CHUNK_DOCUMENTS == 0 {
            if reporter.last_progress.elapsed() >= PROGRESS_INTERVAL {
                reporter.progress()?;
            }
            tokio::task::yield_now().await;
        }
    }
    Ok(())
}

/// Streams at most MAX_ROWS differences; callers own the results and drain the receiver on GPUI.
/// All MongoDB work and reader tasks must run on the connection manager's Tokio runtime.
/// Dropping this future, cancelling, or dropping the receiver aborts both reader tasks.
pub async fn compare_collections_async(
    left: Collection<RawDocumentBuf>,
    right: Collection<RawDocumentBuf>,
    options: CompareOptions,
    cancellation: CancellationToken,
    sender: UnboundedSender<CompareMessage>,
) -> Result<CompareSummary> {
    let started = Instant::now();
    let mut reporter = Reporter {
        sender: &sender,
        counts: CompareCounts::default(),
        read: [Arc::default(), Arc::default()],
        rows: Vec::new(),
        stored: 0,
        stored_bytes: 0,
        limit: options.row_limit.min(MAX_ROWS),
        truncated: false,
        last_progress: Instant::now(),
    };
    let cancelled = async {
        loop {
            if cancellation.is_cancelled() || sender.is_closed() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let result = tokio::select! {
        biased;
        _ = cancelled => Ok(None),
        result = scan(left, right, options, &mut reporter) => result.map(Some),
    };
    // A fast scan can finish between cancellation polls. Honor a cancellation received before
    // the terminal message even if the work happened to finish in that last interval.
    let result = if result.is_ok() && cancellation.is_cancelled() { Ok(None) } else { result };
    reporter.progress()?;
    match result {
        Ok(skipped) => {
            let summary = CompareSummary {
                counts: reporter.counts,
                skipped,
                truncated: reporter.truncated,
                cancelled: skipped.is_none(),
                elapsed: started.elapsed(),
            };
            let _ = sender.unbounded_send(CompareMessage::Done(summary.clone()));
            Ok(summary)
        }
        Err(error) => {
            let _ = sender.unbounded_send(CompareMessage::Failed(error.to_string()));
            Err(error)
        }
    }
}

async fn scan(
    left: Collection<RawDocumentBuf>,
    right: Collection<RawDocumentBuf>,
    options: CompareOptions,
    reporter: &mut Reporter<'_>,
) -> Result<[u64; 2]> {
    options.validate()?;
    let (left_meta, right_meta) = tokio::try_join!(metadata(&left), metadata(&right))?;
    let mut sort = if right_meta.estimated > left_meta.estimated {
        let plan = sort_plan(&options.fields, &right_meta.indexes, &left_meta.indexes);
        SortPlan {
            fields: plan.fields,
            left_covered: plan.right_covered,
            right_covered: plan.left_covered,
        }
    } else {
        sort_plan(&options.fields, &left_meta.indexes, &right_meta.indexes)
    };
    sort.left_covered &= !left_meta.force_simple;
    sort.right_covered &= !right_meta.force_simple;
    reporter
        .sender
        .unbounded_send(CompareMessage::Prepared {
            sort: sort.clone(),
            estimated: [left_meta.estimated, right_meta.estimated],
            simple_collation_forced: [left_meta.force_simple, right_meta.force_simple],
        })
        .map_err(|_| Error::Cancelled("Comparison closed".into()))?;
    let sort_document: Document =
        sort.fields.iter().map(|field| (field.clone(), Bson::Int32(1))).collect();
    let mut tasks = JoinSet::new();
    let (left_tx, left_rx) = mpsc::channel(4);
    let (right_tx, right_rx) = mpsc::channel(4);
    for (index, collection, covered, sender) in
        [(0, left, sort.left_covered, left_tx), (1, right, sort.right_covered, right_tx)]
    {
        let options = options.clone();
        let sort = sort_document.clone();
        let read = reporter.read[index].clone();
        tasks.spawn(async move {
            read_side(collection, options, sort, covered, sender, read)
                .await
                .map(|skipped| (index, skipped))
        });
    }
    let mut left = Groups::new(left_rx, sort.fields.clone());
    let mut right = Groups::new(right_rx, sort.fields);
    let key_is_id = options.fields == ["_id"];
    let ignore = if key_is_id { options.ignore } else { options.ignore.ignoring_id() };
    let readers = async {
        let mut skipped = [0; 2];
        while let Some(result) = tasks.join_next().await {
            let (index, count) =
                result.map_err(|e| Error::Parse(format!("Comparison reader failed: {e}")))??;
            skipped[index] = count;
        }
        Ok::<_, Error>(skipped)
    };
    let ((), skipped) =
        tokio::try_join!(merge(&mut left, &mut right, reporter, &ignore, key_is_id), readers)?;
    Ok(skipped)
}

#[cfg(test)]
mod tests;
