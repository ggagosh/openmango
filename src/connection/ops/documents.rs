//! Document CRUD operations for MongoDB collections.

use futures::TryStreamExt;
use mongodb::Client;
use mongodb::bson::{Document, doc};
use mongodb::results::UpdateResult;

use crate::connection::ConnectionManager;
use crate::connection::types::FindDocumentsOptions;
use crate::error::Result;

pub struct AsyncFindOptions {
    pub filter: Document,
    pub sort: Option<Document>,
    pub projection: Option<Document>,
    pub skip: u64,
    pub limit: i64,
    pub max_time: std::time::Duration,
}

pub async fn find_documents_async(
    client: &Client,
    database: &str,
    collection: &str,
    options: AsyncFindOptions,
) -> Result<Vec<Document>> {
    let coll = client.database(database).collection::<Document>(collection);
    let find_options = mongodb::options::FindOptions::builder()
        .skip(options.skip)
        .limit(options.limit)
        .sort(options.sort)
        .projection(options.projection)
        .max_time(options.max_time)
        .build();
    Ok(coll.find(options.filter).with_options(find_options).await?.try_collect().await?)
}

pub async fn count_documents_async(
    client: &Client,
    database: &str,
    collection: &str,
    filter: Document,
    max_time: std::time::Duration,
) -> Result<u64> {
    let coll = client.database(database).collection::<Document>(collection);
    Ok(coll.count_documents(filter).max_time(max_time).await?)
}

pub async fn find_documents_page_async(
    client: &Client,
    database: &str,
    collection: &str,
    opts: FindDocumentsOptions,
) -> Result<(Vec<Document>, u64)> {
    let FindDocumentsOptions { filter, sort, projection, skip, limit, max_time, cancellation } =
        opts;
    let filter = filter.unwrap_or_default();
    let coll = client.database(database).collection::<Document>(collection);
    let cancelled = || async {
        while !cancellation.is_cancelled() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    };

    let total = tokio::select! {
        _ = cancelled() => return Err(crate::error::Error::Parse("Query cancelled".to_string())),
        result = coll.count_documents(filter.clone()).max_time(max_time) => result?,
    };
    let options = mongodb::options::FindOptions::builder()
        .skip(skip)
        .limit(limit)
        .sort(sort)
        .projection(projection)
        .max_time(max_time)
        .build();
    let cursor = tokio::select! {
        _ = cancelled() => return Err(crate::error::Error::Parse("Query cancelled".to_string())),
        result = coll.find(filter).with_options(options) => result?,
    };
    let documents = tokio::select! {
        _ = cancelled() => return Err(crate::error::Error::Parse("Query cancelled".to_string())),
        result = cursor.try_collect() => result?,
    };
    Ok((documents, total))
}

pub async fn replace_document_async(
    client: &Client,
    database: &str,
    collection: &str,
    id: mongodb::bson::Bson,
    replacement: Document,
) -> Result<()> {
    client
        .database(database)
        .collection::<Document>(collection)
        .replace_one(doc! { "_id": id }, replacement)
        .await?;
    Ok(())
}

pub async fn replace_document_if_current_async(
    client: &Client,
    database: &str,
    collection: &str,
    id: mongodb::bson::Bson,
    expected: Document,
    replacement: Document,
) -> Result<()> {
    let result = client
        .database(database)
        .collection::<Document>(collection)
        .replace_one(
            doc! {
                "_id": id,
                "$expr": { "$eq": ["$$ROOT", { "$literal": expected }] },
            },
            replacement,
        )
        .await?;
    if result.matched_count == 1 {
        Ok(())
    } else {
        Err(crate::error::Error::Parse(
            "Document changed on the server; reload before saving.".to_string(),
        ))
    }
}

impl ConnectionManager {
    /// Find documents in a collection with pagination (runs in Tokio runtime)
    pub fn find_documents(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        opts: FindDocumentsOptions,
    ) -> Result<(Vec<Document>, u64)> {
        self.runtime.block_on(find_documents_page_async(client, database, collection, opts))
    }

    /// Count documents matching an exact filter (runs in Tokio runtime).
    pub fn count_documents(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        filter: Document,
    ) -> Result<u64> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async move {
            let coll = client.database(&database).collection::<Document>(&collection);
            Ok(coll.count_documents(filter).await?)
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

        self.runtime.block_on(replace_document_async(
            &client,
            &database,
            &collection,
            id,
            replacement,
        ))
    }

    /// Replace every document matching a frozen filter while preserving each `_id`.
    /// Replacements are ordered so duplicate-key failures report an exact partial count.
    pub fn replace_documents_by_filter(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        filter: Document,
        replacement: Document,
        cancellation: crate::connection::types::CancellationToken,
    ) -> Result<crate::connection::types::BulkReplaceResult> {
        use futures::TryStreamExt as _;

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async move {
            let coll = client.database(&database).collection::<Document>(&collection);
            let mut cursor = coll
                .find(filter)
                .projection(doc! { "_id": 1 })
                .sort(doc! { "_id": 1 })
                .await?;
            let mut ids = Vec::new();
            while let Some(document) = cursor.try_next().await? {
                if cancellation.is_cancelled() {
                    return Err(crate::error::Error::Parse(
                        "Bulk replacement cancelled before any documents were replaced".to_string(),
                    ));
                }
                let Some(id) = document.get("_id").cloned() else {
                    return Err(crate::error::Error::Parse(
                        "Matched document is missing _id; no replacements were started".to_string(),
                    ));
                };
                ids.push(id);
            }

            let matched_count = ids.len() as u64;
            let mut modified_count = 0u64;
            for (replaced_count, id) in ids.into_iter().enumerate() {
                let replaced_count = replaced_count as u64;
                if cancellation.is_cancelled() {
                    return Err(crate::error::Error::Parse(format!(
                        "Bulk replacement cancelled after replacing {replaced_count} of {matched_count} matched documents"
                    )));
                }
                let mut document = replacement.clone();
                document.insert("_id", id.clone());
                let result = coll.replace_one(doc! { "_id": id }, document).await.map_err(|error| {
                    crate::error::Error::Parse(format!(
                        "Bulk replacement failed after replacing {replaced_count} of {matched_count} matched documents: {error}"
                    ))
                })?;
                if result.matched_count != 1 {
                    return Err(crate::error::Error::Parse(format!(
                        "Bulk replacement stopped after replacing {replaced_count} of {matched_count} matched documents because a document disappeared"
                    )));
                }
                modified_count += result.modified_count;
            }

            Ok(crate::connection::types::BulkReplaceResult {
                matched_count,
                modified_count,
            })
        })
    }

    /// Replace only when the server document still matches the expected baseline.
    pub fn replace_document_if_current(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        id: &mongodb::bson::Bson,
        expected: &Document,
        replacement: Document,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let id = id.clone();
        let expected = expected.clone();

        self.runtime.block_on(replace_document_if_current_async(
            &client,
            &database,
            &collection,
            id,
            expected,
            replacement,
        ))
    }

    /// Return whether an exact-current-state replacement matched its document.
    pub fn replace_document_if_current_matches(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        id: &mongodb::bson::Bson,
        expected: &Document,
        replacement: Document,
    ) -> Result<bool> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let id = id.clone();
        let expected = expected.clone();
        let filter = doc! {
            "_id": id,
            "$expr": { "$eq": ["$$ROOT", { "$literal": expected }] },
        };

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            let result = coll.replace_one(filter, replacement).await?;
            Ok(result.matched_count == 1)
        })
    }

    /// Find a single document by _id (runs in Tokio runtime)
    pub fn find_document_by_id(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        id: &mongodb::bson::Bson,
    ) -> Result<Option<Document>> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let id = id.clone();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            let result = coll.find_one(doc! { "_id": id }).await?;
            Ok(result)
        })
    }

    /// Delete only when the server document still matches the expected image.
    pub fn delete_document_if_current_matches(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        id: &mongodb::bson::Bson,
        expected: &Document,
    ) -> Result<bool> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let id = id.clone();
        let expected = expected.clone();
        let filter = doc! {
            "_id": id,
            "$expr": { "$eq": ["$$ROOT", { "$literal": expected }] },
        };

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            let result = coll.delete_one(filter).await?;
            Ok(result.deleted_count == 1)
        })
    }

    /// Insert a recovery image only while its `_id` remains absent.
    pub fn insert_document_if_absent_matches(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        document: Document,
    ) -> Result<bool> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            match coll.insert_one(document).await {
                Ok(_) => Ok(true),
                Err(error) if is_duplicate_key(&error) => Ok(false),
                Err(error) => Err(error.into()),
            }
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

fn is_duplicate_key(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}
