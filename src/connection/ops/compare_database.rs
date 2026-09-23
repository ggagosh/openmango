//! Pairs the collections of two databases by name. Read-only; call on the connection runtime.

use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use futures::{StreamExt, TryStreamExt};
use mongodb::Client;
use mongodb::bson::{Document, RawDocumentBuf};
use mongodb::results::CollectionType;

use crate::bson::compare::IgnoreSet;
use crate::connection::CancellationToken;
use crate::connection::ops::compare::{
    CompareCounts, CompareMessage, CompareOptions, CompareSummary, compare_collections_async,
};
use crate::connection::ops::stats::{collection_stats_async, storage_count_and_size};
use crate::error::Result;
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
            let (estimated, bytes) = if kind == CollectionKind::Collection {
                collection_stats_async(client, database, &name, timeout)
                    .await
                    .map_or((None, None), |stats| storage_count_and_size(&stats))
            } else {
                (None, None)
            };
            (name, SideCollection { kind, estimated, bytes })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn side(kind: CollectionKind) -> SideCollection {
        SideCollection { kind, estimated: Some(1), bytes: Some(1) }
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
