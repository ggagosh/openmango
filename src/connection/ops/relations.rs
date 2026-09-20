//! Confirming a reference against the collections that might hold it.
//!
//! Every jump is confirmed by looking the value up, never by its name alone: a field called
//! `userId` pointing at `users` is a guess until `users` is asked whether it holds that `_id`.
//! The lookups here are covered queries against the `_id` index, so a probe reads the index and
//! never touches a document.

use std::time::Duration;

use futures::StreamExt as _;
use mongodb::Client;
use mongodb::bson::{Bson, Document, doc};

use crate::connection::ConnectionManager;
use crate::error::Result;

/// Probes in flight at once. Enough to make a search of a few dozen collections feel instant,
/// few enough to leave a production server alone.
const PROBE_CONCURRENCY: usize = 8;

/// Fetch the document a known relation points at.
///
/// One query answers both questions a jump asks: whether the target exists, and what to show in
/// the peek. A `None` is a broken reference, which is information rather than an error.
pub async fn find_by_id_async(
    client: &Client,
    database: &str,
    collection: &str,
    id: &Bson,
    max_time: Duration,
) -> Result<Option<Document>> {
    let coll = client.database(database).collection::<Document>(collection);
    Ok(coll.find_one(doc! { "_id": id.clone() }).max_time(max_time).await?)
}

/// Ask each collection whether it holds this `_id`, best-named first.
///
/// Projecting to `_id` keeps the query covered by the index. The order of `collections` is
/// preserved in the result, so the caller's ranking survives the concurrency.
pub async fn probe_id_async(
    client: &Client,
    database: &str,
    collections: &[String],
    id: &Bson,
    max_time: Duration,
) -> Vec<String> {
    let db = client.database(database);
    // Each probe owns its name and namespace, so nothing borrows the caller's slice across an
    // await point and the whole batch is one self-contained set of futures.
    let probes: Vec<_> = collections
        .iter()
        .enumerate()
        .map(|(rank, collection)| {
            let coll = db.collection::<Document>(collection);
            let id = id.clone();
            let name = collection.clone();
            let namespace = format!("{database}.{collection}");
            async move {
                let hit = coll
                    .find_one(doc! { "_id": id })
                    .projection(doc! { "_id": 1 })
                    .max_time(max_time)
                    .await;
                // A collection that errors or times out is reported as "not here". The search
                // is a convenience, and one unreadable collection must not fail the whole jump.
                match hit {
                    Ok(Some(_)) => Some((rank, name)),
                    Ok(None) => None,
                    Err(error) => {
                        log::debug!("Probe of {namespace} failed: {error}");
                        None
                    }
                }
            }
        })
        .collect();

    let mut found: Vec<(usize, String)> = futures::stream::iter(probes)
        .buffer_unordered(PROBE_CONCURRENCY)
        .collect::<Vec<Option<(usize, String)>>>()
        .await
        .into_iter()
        .flatten()
        .collect();

    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, collection)| collection).collect()
}

impl ConnectionManager {
    pub fn find_by_id(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        id: &Bson,
        max_time: Duration,
    ) -> Result<Option<Document>> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let id = id.clone();
        self.runtime.block_on(async move {
            find_by_id_async(&client, &database, &collection, &id, max_time).await
        })
    }

    pub fn probe_id(
        &self,
        client: &Client,
        database: &str,
        collections: &[String],
        id: &Bson,
        max_time: Duration,
    ) -> Vec<String> {
        let client = client.clone();
        let database = database.to_string();
        let collections = collections.to_vec();
        let id = id.clone();
        self.runtime.block_on(async move {
            probe_id_async(&client, &database, &collections, &id, max_time).await
        })
    }
}
