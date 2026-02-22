//! Schema sampling operations.

use futures::TryStreamExt;
use mongodb::Client;
use mongodb::bson::{Document, doc};

use crate::connection::ConnectionManager;
use crate::error::Result;

impl ConnectionManager {
    /// Sample documents from a collection for schema analysis.
    /// Returns (sampled_docs, estimated_total_count).
    pub fn sample_for_schema(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        sample_size: u64,
    ) -> Result<(Vec<Document>, u64)> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let coll = db.collection::<Document>(&collection);

            let total = coll.estimated_document_count().await.unwrap_or(0);
            if total == 0 {
                return Ok((Vec::new(), 0));
            }

            let pipeline = vec![doc! { "$sample": { "size": sample_size as i64 } }];
            let cursor = coll.aggregate(pipeline).await?;
            let docs: Vec<Document> = cursor.try_collect().await?;
            Ok((docs, total))
        })
    }
}
