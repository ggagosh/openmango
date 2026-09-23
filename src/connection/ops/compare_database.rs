//! Pairs the collections of two databases by name and compares them, then syncs them on
//! request. Call on the connection runtime.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::{Bson, Document, RawDocumentBuf, doc};
use mongodb::error::ErrorKind;
use mongodb::results::CollectionType;
use mongodb::{Client, Collection, IndexModel};

use crate::bson::compare::IgnoreSet;
use crate::connection::CancellationToken;
use crate::connection::ops::compare::{
    CompareCounts, CompareMessage, CompareOptions, CompareSummary, DiffKind, DiffRow, Side,
    compare_collections_async,
};
use crate::connection::ops::compare_sync::restore::RestoreHandle;
use crate::connection::ops::compare_sync::{
    SyncItem, SyncProgress, SyncSummary, operation_for, supports_sync, sync_collections_async,
    undo_sync_async,
};
use crate::connection::ops::stats::{collection_stats_async, storage_count_and_size};
use crate::error::{Error, Result};
use crate::models::is_system_collection;

/// Sizes are read a few collections at a time, so a large database does not flood the server.
const STATS_CONCURRENCY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionKind {
    Collection,
    View,
    Timeseries,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideCollection {
    pub kind: CollectionKind,
    /// From metadata: approximate after an unclean shutdown, and counts orphans when sharded.
    pub estimated: Option<u64>,
    pub bytes: Option<u64>,
    /// Each index described by what it does, not its name; sorted. None when unreadable.
    pub indexes: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairKind {
    LeftOnly,
    RightOnly,
    Both,
    /// Present on both sides, but a view or time-series on at least one.
    NotComparable(CollectionKind),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionPair {
    pub name: String,
    pub sides: [Option<SideCollection>; 2],
}

impl CollectionPair {
    pub fn kind(&self) -> PairKind {
        match &self.sides {
            [Some(left), Some(right)] => match (left.kind, right.kind) {
                (CollectionKind::Collection, CollectionKind::Collection) => PairKind::Both,
                (CollectionKind::Collection, other) | (other, _) => PairKind::NotComparable(other),
            },
            [None, Some(_)] => PairKind::RightOnly,
            _ => PairKind::LeftOnly,
        }
    }

    /// Indexes found on one side only, left then right. None when both match or one is unknown.
    pub fn index_difference(&self) -> Option<[Vec<String>; 2]> {
        let [Some(left), Some(right)] = &self.sides else {
            return None;
        };
        let (left, right) = (left.indexes.as_ref()?, right.indexes.as_ref()?);
        let only = |a: &[String], b: &[String]| {
            a.iter().filter(|index| !b.contains(index)).cloned().collect::<Vec<_>>()
        };
        let difference = [only(left, right), only(right, left)];
        difference.iter().any(|side| !side.is_empty()).then_some(difference)
    }
}

/// An index as its keys and the options that change what it does, e.g. `{ sku: 1 } unique`.
/// Names are left out: two sides may name the same index differently.
pub fn describe_index(index: &IndexModel) -> String {
    let value = |value: &Bson| value.clone().into_relaxed_extjson().to_string();
    let keys: Vec<_> =
        index.keys.iter().map(|(key, order)| format!("{key}: {}", value(order))).collect();
    let mut text = format!("{{ {} }}", keys.join(", "));
    let Some(options) = &index.options else {
        return text;
    };
    if options.unique == Some(true) {
        text.push_str(" unique");
    }
    if options.sparse == Some(true) {
        text.push_str(" sparse");
    }
    if options.hidden == Some(true) {
        text.push_str(" hidden");
    }
    if let Some(ttl) = options.expire_after {
        text.push_str(&format!(" expires after {} s", ttl.as_secs()));
    }
    if let Some(filter) = &options.partial_filter_expression {
        text.push_str(&format!(" where {}", value(&Bson::Document(filter.clone()))));
    }
    if let Some(collation) = &options.collation {
        text.push_str(&format!(" collation {}", collation.locale));
    }
    text
}

/// One database's collections without `system.*`, with metadata sizes where readable.
pub async fn list_side(
    client: &Client,
    database: &str,
    timeout: Duration,
) -> Result<Vec<(String, SideCollection)>> {
    let specs: Vec<_> = client.database(database).list_collections().await?.try_collect().await?;
    let named: Vec<_> = specs
        .into_iter()
        .filter(|spec| !is_system_collection(&spec.name))
        .map(|spec| {
            let kind = if spec.options.timeseries.is_some()
                || spec.collection_type == CollectionType::Timeseries
            {
                CollectionKind::Timeseries
            } else if spec.collection_type == CollectionType::View {
                CollectionKind::View
            } else {
                CollectionKind::Collection
            };
            (spec.name, kind)
        })
        .collect();
    // Sizes are optional: an account without collStats still gets the pairing.
    Ok(futures::stream::iter(named)
        .map(|(name, kind)| async move {
            if kind != CollectionKind::Collection {
                return (
                    name,
                    SideCollection { kind, estimated: None, bytes: None, indexes: None },
                );
            }
            let collection = client.database(database).collection::<Document>(&name);
            let (stats, indexes) =
                tokio::join!(collection_stats_async(client, database, &name, timeout), async {
                    collection.list_indexes().max_time(timeout).await?.try_collect::<Vec<_>>().await
                },);
            let (estimated, bytes) =
                stats.map_or((None, None), |stats| storage_count_and_size(&stats));
            let indexes = indexes.ok().map(|indexes| {
                let mut indexes: Vec<_> = indexes.iter().map(describe_index).collect();
                indexes.sort();
                indexes
            });
            (name, SideCollection { kind, estimated, bytes, indexes })
        })
        .buffered(STATS_CONCURRENCY)
        .collect()
        .await)
}

#[derive(Debug)]
pub enum PairMessage {
    Started(usize),
    Progress(usize, CompareCounts),
    /// `cancelled` in the summary means the collection was skipped or the run cancelled.
    Done(usize, CompareSummary),
    Failed(usize, String),
}

/// One collection to scan: its index in the listing, its name, and the token that skips it.
pub struct PairScan {
    pub index: usize,
    pub name: String,
    pub cancellation: CancellationToken,
}

/// Pass two: the collection comparison, once per collection, one at a time, in the order given.
/// Differences are counted and never stored. A collection whose token is cancelled before its
/// turn is passed over without a message; cancelling every token stops the run.
pub async fn compare_pairs_async(
    clients: [Client; 2],
    databases: [String; 2],
    scans: Vec<PairScan>,
    ignore: IgnoreSet,
    sender: UnboundedSender<PairMessage>,
) {
    for PairScan { index, name, cancellation } in scans {
        if sender.is_closed() {
            return;
        }
        if cancellation.is_cancelled() {
            continue;
        }
        let _ = sender.unbounded_send(PairMessage::Started(index));
        let [left, right] = [0, 1].map(|side| {
            clients[side].database(&databases[side]).collection::<RawDocumentBuf>(&name)
        });
        let options = CompareOptions {
            fields: vec!["_id".into()],
            filter: Document::new(),
            ignore: ignore.clone(),
            row_limit: 0,
            row_kinds: None,
        };
        let (progress, mut messages) = futures::channel::mpsc::unbounded();
        let forward = async {
            while let Some(message) = messages.next().await {
                if let CompareMessage::Progress { counts, .. } = message {
                    let _ = sender.unbounded_send(PairMessage::Progress(index, counts));
                }
            }
        };
        let (result, ()) = tokio::join!(
            compare_collections_async(left, right, options, cancellation, progress),
            forward
        );
        let _ = sender.unbounded_send(match result {
            Ok(summary) => PairMessage::Done(index, summary),
            Err(error) => PairMessage::Failed(index, error.to_string()),
        });
    }
}

/// Every name on either side, in the sidebar's order: case-insensitive, then exact.
pub fn pair_collections(
    left: Vec<(String, SideCollection)>,
    right: Vec<(String, SideCollection)>,
) -> Vec<CollectionPair> {
    let mut pairs: std::collections::BTreeMap<String, [Option<SideCollection>; 2]> =
        Default::default();
    for (side, collections) in [left, right].into_iter().enumerate() {
        for (name, collection) in collections {
            pairs.entry(name).or_default()[side] = Some(collection);
        }
    }
    let mut pairs: Vec<_> =
        pairs.into_iter().map(|(name, sides)| CollectionPair { name, sides }).collect();
    pairs.sort_by(|a, b| {
        a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.name.cmp(&b.name))
    });
    pairs
}

/// What a database sync writes into the target. It never drops collections, never writes to
/// views or time-series collections, and leaves minor differences as they are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncMode {
    /// Insert the documents the target lacks. Existing documents are left alone.
    #[default]
    AddMissing,
    /// Also replace the documents that differ.
    AddAndUpdate,
    /// Also delete the documents only the target has.
    Mirror,
}

impl SyncMode {
    pub const ALL: [Self; 3] = [Self::AddMissing, Self::AddAndUpdate, Self::Mirror];

    pub fn label(self) -> &'static str {
        match self {
            Self::AddMissing => "Add missing",
            Self::AddAndUpdate => "Add and update",
            Self::Mirror => "Mirror",
        }
    }

    /// The difference kinds this mode writes when `target` receives the changes.
    pub fn kinds(self, target: Side) -> Vec<DiffKind> {
        let (missing, extra) = match target {
            Side::Right => (DiffKind::OnlyLeft, DiffKind::OnlyRight),
            Side::Left => (DiffKind::OnlyRight, DiffKind::OnlyLeft),
        };
        match self {
            Self::AddMissing => vec![missing],
            Self::AddAndUpdate => vec![missing, DiffKind::Different],
            Self::Mirror => vec![missing, DiffKind::Different, extra],
        }
    }

    /// Inserts, replacements and deletes this mode makes, from a finished scan's counts.
    pub fn writes(self, counts: &CompareCounts, target: Side) -> [u64; 3] {
        let (missing, extra) = match target {
            Side::Right => (counts.only_left, counts.only_right),
            Side::Left => (counts.only_right, counts.only_left),
        };
        match self {
            Self::AddMissing => [missing, 0, 0],
            Self::AddAndUpdate => [missing, counts.different, 0],
            Self::Mirror => [missing, counts.different, extra],
        }
    }
}

/// One collection to sync: its index in the listing, and whether the target lacks it.
#[derive(Clone, Debug)]
pub struct PairSync {
    pub index: usize,
    pub name: String,
    pub create: bool,
}

pub enum PairSyncMessage {
    /// The collection's undo log, sent before its first write.
    Started(usize, Arc<RestoreHandle>),
    Progress(usize, SyncSummary),
    Done(usize, SyncSummary),
    Failed(usize, String),
}

/// A database sync: `pairs` from the other side into `target`, matched by `_id`.
pub struct DatabaseSync {
    pub clients: [Client; 2],
    pub databases: [String; 2],
    pub target: Side,
    pub mode: SyncMode,
    pub pairs: Vec<PairSync>,
    pub ignore: IgnoreSet,
    pub restore_dir: PathBuf,
    /// Differences written per read of a collection: `MAX_ROWS`, lower in tests.
    pub pass_rows: usize,
}

/// Runs a database sync one collection at a time. Each collection is scanned for the kinds the
/// mode writes, then written through the guarded sync with its own undo log. A collection the
/// target lacks is first created like the source's.
pub async fn sync_pairs_async(
    sync: DatabaseSync,
    cancellation: CancellationToken,
    sender: UnboundedSender<PairSyncMessage>,
) -> Result<()> {
    let DatabaseSync { clients, databases, target, mode, pairs, ignore, restore_dir, pass_rows } =
        sync;
    let destination = if target == Side::Left { 0 } else { 1 };
    if !supports_sync(&clients[destination]).await? {
        return Err(Error::Parse(
            "Sync and undo require MongoDB 8.0 or newer on the target. Older servers support comparison only.".into(),
        ));
    }
    for pair in pairs {
        if cancellation.is_cancelled() || sender.is_closed() {
            break;
        }
        let [left, right] = [0, 1].map(|side| {
            clients[side].database(&databases[side]).collection::<RawDocumentBuf>(&pair.name)
        });
        let run =
            SyncPass { target, mode, ignore: &ignore, pass_rows, cancellation: &cancellation };
        let result = run.pair(&pair, [left, right], &restore_dir, &sender).await;
        let _ = sender.unbounded_send(match result {
            Ok(summary) => PairSyncMessage::Done(pair.index, summary),
            Err(error) => PairSyncMessage::Failed(pair.index, error.to_string()),
        });
    }
    Ok(())
}

struct SyncPass<'a> {
    target: Side,
    mode: SyncMode,
    ignore: &'a IgnoreSet,
    pass_rows: usize,
    cancellation: &'a CancellationToken,
}

impl SyncPass<'_> {
    async fn pair(
        &self,
        pair: &PairSync,
        sides: [Collection<RawDocumentBuf>; 2],
        restore_dir: &Path,
        sender: &UnboundedSender<PairSyncMessage>,
    ) -> Result<SyncSummary> {
        let (source, destination) = if self.target == Side::Left { (1, 0) } else { (0, 1) };
        if pair.create {
            create_like(&sides[source], &sides[destination]).await?;
        }
        let directory = restore_dir.to_path_buf();
        let restore = Arc::new(
            tokio::task::spawn_blocking(move || RestoreHandle::create(&directory))
                .await
                .map_err(|e| Error::Parse(e.to_string()))??,
        );
        let _ = sender.unbounded_send(PairSyncMessage::Started(pair.index, restore.clone()));
        let mut total = SyncSummary::default();
        // ponytail: a pass holds `pass_rows` differences, then the collection is read again for
        // the rest; millions of differences mean several reads. Stream rows into the sync if slow.
        loop {
            let options = CompareOptions {
                fields: vec!["_id".into()],
                filter: Document::new(),
                ignore: self.ignore.clone(),
                row_limit: self.pass_rows,
                row_kinds: Some(self.mode.kinds(self.target)),
            };
            let (rows_sender, messages) = futures::channel::mpsc::unbounded();
            let (scan, rows) = tokio::join!(
                compare_collections_async(
                    sides[0].clone(),
                    sides[1].clone(),
                    options,
                    self.cancellation.clone(),
                    rows_sender
                ),
                collect_rows(messages)
            );
            let scan = scan?;
            if scan.cancelled || rows.is_empty() {
                total.cancelled |= scan.cancelled;
                break;
            }
            let items = rows
                .into_iter()
                .enumerate()
                .filter_map(|(row_index, row)| {
                    Some(SyncItem {
                        row_index,
                        operation: operation_for(row.kind, self.target)?,
                        field: None,
                        row,
                    })
                })
                .collect();
            let (progress, mut updates) = futures::channel::mpsc::unbounded::<SyncProgress>();
            let base = total.clone();
            let forward = async {
                while let Some(update) = updates.next().await {
                    let mut running = base.clone();
                    running.absorb(&update.summary);
                    let _ = sender.unbounded_send(PairSyncMessage::Progress(pair.index, running));
                }
            };
            let (pass, ()) = tokio::join!(
                sync_collections_async(
                    sides.clone(),
                    self.target,
                    vec!["_id".into()],
                    items,
                    restore.clone(),
                    self.cancellation.clone(),
                    progress
                ),
                forward
            );
            let pass = pass?;
            total.absorb(&pass);
            // A pass that wrote nothing would find the same rows again.
            if !scan.truncated || pass.cancelled || pass.uncertain > 0 || pass.written == 0 {
                break;
            }
        }
        Ok(total)
    }
}

async fn collect_rows(mut messages: UnboundedReceiver<CompareMessage>) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    while let Some(message) = messages.next().await {
        if let CompareMessage::Progress { new_rows, .. } = message {
            rows.extend(new_rows);
        }
    }
    rows
}

/// Creates `target` with the options and indexes of `source`. A collection created in the
/// meantime is used as it is.
async fn create_like(
    source: &Collection<RawDocumentBuf>,
    target: &Collection<RawDocumentBuf>,
) -> Result<()> {
    let spec = source
        .client()
        .database(&source.namespace().db)
        .list_collections()
        .filter(doc! {"name": source.name()})
        .await?
        .try_next()
        .await?
        .ok_or_else(|| Error::Parse(format!("{} no longer exists", source.namespace())))?;
    let database = target.client().database(&target.namespace().db);
    match database.create_collection(target.name()).with_options(spec.options).await {
        Ok(()) => {}
        // NamespaceExists
        Err(error) if matches!(*error.kind, ErrorKind::Command(ref e) if e.code == 48) => {}
        Err(error) => return Err(error.into()),
    }
    let indexes: Vec<IndexModel> = source
        .list_indexes()
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .filter(|index| index.options.as_ref().and_then(|o| o.name.as_deref()) != Some("_id_"))
        .collect();
    if !indexes.is_empty() {
        target.create_indexes(indexes).await?;
    }
    Ok(())
}

/// Undoes a database sync one collection at a time, each from its own undo log.
pub async fn undo_pairs_async(
    client: Client,
    database: String,
    logs: Vec<(usize, String, Arc<RestoreHandle>)>,
    cancellation: CancellationToken,
    sender: UnboundedSender<PairSyncMessage>,
) {
    for (index, name, restore) in logs {
        if cancellation.is_cancelled() || sender.is_closed() {
            break;
        }
        let _ = sender.unbounded_send(PairSyncMessage::Started(index, restore.clone()));
        let (progress, mut updates) = futures::channel::mpsc::unbounded::<SyncProgress>();
        let forward = async {
            while let Some(update) = updates.next().await {
                let _ = sender.unbounded_send(PairSyncMessage::Progress(index, update.summary));
            }
        };
        let collection = client.database(&database).collection::<RawDocumentBuf>(&name);
        let (result, ()) = tokio::join!(
            undo_sync_async(collection, restore, cancellation.clone(), progress),
            forward
        );
        let _ = sender.unbounded_send(match result {
            Ok(summary) => PairSyncMessage::Done(index, summary),
            Err(error) => PairSyncMessage::Failed(index, error.to_string()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(kind: CollectionKind) -> SideCollection {
        SideCollection { kind, estimated: Some(1), bytes: Some(1), indexes: None }
    }

    #[test]
    fn indexes_are_compared_by_what_they_do_not_by_name() {
        use mongodb::bson::doc;
        use mongodb::options::IndexOptions;
        let index = |keys, options| IndexModel::builder().keys(keys).options(options).build();
        let unique = index(
            doc! {"sku": 1},
            Some(IndexOptions::builder().unique(true).name("sku_unique".to_string()).build()),
        );
        assert_eq!(describe_index(&unique), "{ sku: 1 } unique");
        let ttl = index(
            doc! {"at": 1},
            Some(IndexOptions::builder().expire_after(Duration::from_secs(60)).build()),
        );
        assert_eq!(describe_index(&ttl), "{ at: 1 } expires after 60 s");
        assert_eq!(describe_index(&index(doc! {"body": "text"}, None)), r#"{ body: "text" }"#);

        let with = |indexes: &[&str]| SideCollection {
            indexes: Some(indexes.iter().map(|index| index.to_string()).collect()),
            ..side(CollectionKind::Collection)
        };
        let pair = |left, right| CollectionPair {
            name: "orders".into(),
            sides: [Some(left), Some(right)],
        };
        let id = "{ _id: 1 }";
        assert_eq!(pair(with(&[id]), with(&[id])).index_difference(), None);
        assert_eq!(
            pair(with(&[id, "{ sku: 1 } unique"]), with(&[id, "{ sku: 1 }"])).index_difference(),
            Some([vec!["{ sku: 1 } unique".to_string()], vec!["{ sku: 1 }".to_string()]])
        );
        assert_eq!(pair(side(CollectionKind::Collection), with(&[id])).index_difference(), None);
    }

    #[test]
    fn pairs_every_name_once_in_sidebar_order_and_names_why_some_cannot_be_compared() {
        use CollectionKind::*;
        let pairs = pair_collections(
            vec![
                ("orders".into(), side(Collection)),
                ("Audit".into(), side(Collection)),
                ("active_users".into(), side(View)),
                ("metrics".into(), side(Timeseries)),
            ],
            vec![
                ("orders".into(), side(Collection)),
                ("active_users".into(), side(Collection)),
                ("metrics".into(), side(Timeseries)),
                ("zones".into(), side(Collection)),
            ],
        );
        let summary: Vec<_> = pairs.iter().map(|pair| (pair.name.as_str(), pair.kind())).collect();
        assert_eq!(
            summary,
            [
                ("active_users", PairKind::NotComparable(View)),
                ("Audit", PairKind::LeftOnly),
                ("metrics", PairKind::NotComparable(Timeseries)),
                ("orders", PairKind::Both),
                ("zones", PairKind::RightOnly),
            ]
        );
    }
}
