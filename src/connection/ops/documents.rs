//! Document CRUD operations for MongoDB collections.

use mongodb::Client;
use mongodb::bson::{Document, doc};
use mongodb::results::UpdateResult;

use crate::connection::ConnectionManager;
use crate::connection::types::FindDocumentsOptions;
use crate::error::Result;

impl ConnectionManager {
    /// Find documents in a collection with pagination (runs in Tokio runtime)
    pub fn find_documents(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        opts: FindDocumentsOptions,
    ) -> Result<(Vec<Document>, u64)> {
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let filter = opts.filter.unwrap_or_default();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);

            // Get total count (with filter)
            let total = coll.count_documents(filter.clone()).await?;

            // Fetch documents with pagination
            let mut options = mongodb::options::FindOptions::default();
            options.skip = Some(opts.skip);
            options.limit = Some(opts.limit);
            options.sort = opts.sort;
            options.projection = opts.projection;

            let cursor = coll.find(filter).with_options(options).await?;
            let documents: Vec<Document> = cursor.try_collect().await?;

            Ok((documents, total))
        })
    }

    /// Insert a document into a collection (runs in Tokio runtime)
    pub fn insert_document(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        document: Document,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            coll.insert_one(document).await?;
            Ok(())
        })
    }

    /// Insert multiple documents into a collection (runs in Tokio runtime)
    pub fn insert_documents(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        documents: Vec<Document>,
    ) -> Result<usize> {
        let count = documents.len();
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            coll.insert_many(documents).await?;
            Ok(count)
        })
    }

    /// Delete multiple documents by filter (runs in Tokio runtime)
    pub fn delete_documents(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        filter: Document,
    ) -> Result<u64> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            let result = coll.delete_many(filter).await?;
            Ok(result.deleted_count)
        })
    }

    /// Sample documents from a collection (runs in Tokio runtime)
    pub fn sample_documents(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        size: i64,
    ) -> Result<Vec<Document>> {
        use futures::TryStreamExt;

        if size <= 0 {
            return Ok(Vec::new());
        }

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            let pipeline = vec![doc! { "$sample": { "size": size } }];
            let cursor = coll.aggregate(pipeline).await?;
            let docs: Vec<Document> = cursor.try_collect().await?;
            Ok(docs)
        })
    }

    /// Get estimated document count for a collection (fast, uses metadata).
    /// This is much faster than count_documents() as it uses collection statistics.
    pub fn estimated_document_count(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
    ) -> Result<u64> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            let count = coll.estimated_document_count().await?;
            Ok(count)
        })
    }

    /// Update a single document (runs in Tokio runtime)
    pub fn update_one(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        filter: Document,
        update: Document,
    ) -> Result<UpdateResult> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            let result = coll.update_one(filter, update).await?;
            Ok(result)
        })
    }

    /// Update multiple documents (runs in Tokio runtime)
    pub fn update_many(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        filter: Document,
        update: Document,
    ) -> Result<UpdateResult> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            let result = coll.update_many(filter, update).await?;
            Ok(result)
        })
    }

    /// Replace a document by _id in a collection (runs in Tokio runtime)
    pub fn replace_document(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        id: &mongodb::bson::Bson,
        replacement: Document,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let id = id.clone();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            coll.replace_one(doc! { "_id": id }, replacement).await?;
            Ok(())
        })
    }

    /// Delete a document by _id in a collection (runs in Tokio runtime)
    pub fn delete_document(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        id: &mongodb::bson::Bson,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let id = id.clone();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            coll.delete_one(doc! { "_id": id }).await?;
            Ok(())
        })
    }
}
