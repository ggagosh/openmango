//! Index operations for MongoDB collections.

use mongodb::Client;
use mongodb::IndexModel;
use mongodb::bson::{Document, doc};

use crate::connection::ConnectionManager;
use crate::error::Result;

impl ConnectionManager {
    /// List indexes for a collection (runs in Tokio runtime)
    pub fn list_indexes(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
    ) -> Result<Vec<IndexModel>> {
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            let cursor = coll.list_indexes().await?;
            let indexes: Vec<IndexModel> = cursor.try_collect().await?;
            Ok(indexes)
        })
    }

    /// Create an index for a collection (runs in Tokio runtime)
    pub fn create_index(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        index: Document,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let db = client.database(&database);
            db.run_command(doc! { "createIndexes": collection, "indexes": [index] }).await?;
            Ok(())
        })
    }

    /// Drop an index by name in a collection (runs in Tokio runtime)
    pub fn drop_index(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        name: &str,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let name = name.to_string();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            coll.drop_index(name).await?;
            Ok(())
        })
    }
}
