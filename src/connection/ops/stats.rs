//! Database and collection statistics operations.

use std::time::Duration;

use futures::TryStreamExt;
use mongodb::Client;
use mongodb::bson::{Document, doc};

use crate::connection::ConnectionManager;
use crate::error::Result;

pub async fn collection_stats_async(
    client: &Client,
    database: &str,
    collection: &str,
    max_time: Duration,
) -> Result<Document> {
    let coll = client.database(database).collection::<Document>(collection);
    let pipeline = vec![doc! { "$collStats": { "storageStats": { "scale": 1 } } }];
    coll.aggregate(pipeline)
        .max_time(max_time)
        .await?
        .try_next()
        .await?
        .ok_or_else(|| crate::error::Error::Parse("No collection stats returned".to_string()))
}

impl ConnectionManager {
    /// Fetch collection stats (runs in Tokio runtime)
    pub fn collection_stats(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
    ) -> Result<Document> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let stats = db.run_command(doc! { "collStats": collection }).await?;
            Ok(stats)
        })
    }

    /// Fetch database stats (runs in Tokio runtime)
    pub fn database_stats(&self, client: &Client, database: &str) -> Result<Document> {
        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let stats = db.run_command(doc! { "dbStats": 1 }).await?;
            Ok(stats)
        })
    }
}
