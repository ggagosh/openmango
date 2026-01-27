use std::sync::LazyLock;

use mongodb::Client;
use mongodb::IndexModel;
use mongodb::bson::doc;
use mongodb::results::{CollectionSpecification, UpdateResult};
use std::time::Duration;
use tokio::runtime::Runtime;

use crate::error::{Error, Result};
use crate::models::SavedConnection;

#[derive(Debug)]
pub enum AggregatePipelineError {
    Mongo(crate::error::Error),
    Aborted,
}

impl From<crate::error::Error> for AggregatePipelineError {
    fn from(value: crate::error::Error) -> Self {
        Self::Mongo(value)
    }
}

pub struct FindDocumentsOptions {
    pub filter: Option<mongodb::bson::Document>,
    pub sort: Option<mongodb::bson::Document>,
    pub projection: Option<mongodb::bson::Document>,
    pub skip: u64,
    pub limit: i64,
}

/// Global singleton connection manager
static CONNECTION_MANAGER: LazyLock<ConnectionManager> = LazyLock::new(ConnectionManager::new);

/// Get the global connection manager instance
pub fn get_connection_manager() -> &'static ConnectionManager {
    &CONNECTION_MANAGER
}

/// Manages MongoDB client connections with caching
pub struct ConnectionManager {
    /// Tokio runtime for MongoDB async operations
    runtime: Runtime,
}

impl ConnectionManager {
    /// Create a new connection manager
    pub fn new() -> Self {
        let runtime = Runtime::new().expect("Failed to create Tokio runtime");
        Self { runtime }
    }

    /// Connect to MongoDB using the saved connection config (runs in Tokio runtime)
    pub fn connect(&self, config: &SavedConnection) -> Result<Client> {
        let uri = config.uri.clone();
        self.runtime.block_on(async {
            let client = Client::with_uri_str(&uri).await?;

            // Ping to verify connection
            client.database("admin").run_command(doc! { "ping": 1 }).await?;

            Ok(client)
        })
    }

    /// Test connectivity with a timeout (runs in Tokio runtime)
    pub fn test_connection(&self, config: &SavedConnection, timeout: Duration) -> Result<()> {
        let uri = config.uri.clone();
        self.runtime.block_on(async {
            let fut = async {
                let client = Client::with_uri_str(&uri).await?;
                client.database("admin").run_command(doc! { "ping": 1 }).await?;
                Ok::<(), mongodb::error::Error>(())
            };

            match tokio::time::timeout(timeout, fut).await {
                Ok(result) => result.map_err(Error::from),
                Err(_) => Err(Error::Timeout("Connection timed out".to_string())),
            }
        })
    }

    /// List databases for a connected client (runs in Tokio runtime)
    pub fn list_databases(&self, client: &Client) -> Result<Vec<String>> {
        let client = client.clone();
        self.runtime.block_on(async {
            let mut databases = client.list_database_names().await?;
            databases.sort_unstable_by_key(|name| name.to_lowercase());
            Ok(databases)
        })
    }

    /// List collections in a database (runs in Tokio runtime)
    pub fn list_collections(&self, client: &Client, database: &str) -> Result<Vec<String>> {
        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let mut collections = db.list_collection_names().await?;
            collections.sort_unstable_by_key(|name| name.to_lowercase());
            Ok(collections)
        })
    }

    /// List collection specs in a database (runs in Tokio runtime)
    pub fn list_collection_specs(
        &self,
        client: &Client,
        database: &str,
    ) -> Result<Vec<CollectionSpecification>> {
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let cursor = db.list_collections().await?;
            let mut specs: Vec<CollectionSpecification> = cursor.try_collect().await?;
            specs.sort_unstable_by_key(|spec| spec.name.to_lowercase());
            Ok(specs)
        })
    }

    /// Create a collection in a database (runs in Tokio runtime)
    pub fn create_collection(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            db.create_collection(&collection).await?;
            Ok(())
        })
    }

    /// Drop a collection in a database (runs in Tokio runtime)
    pub fn drop_collection(&self, client: &Client, database: &str, collection: &str) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            coll.drop().await?;
            Ok(())
        })
    }

    /// Rename a collection in a database (runs in Tokio runtime)
    pub fn rename_collection(
        &self,
        client: &Client,
        database: &str,
        from: &str,
        to: &str,
    ) -> Result<()> {
        let client = client.clone();
        let from = format!("{database}.{from}");
        let to = format!("{database}.{to}");
        self.runtime.block_on(async {
            let admin = client.database("admin");
            admin
                .run_command(doc! { "renameCollection": from, "to": to, "dropTarget": false })
                .await?;
            Ok(())
        })
    }

    /// Drop a database (runs in Tokio runtime)
    pub fn drop_database(&self, client: &Client, database: &str) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            db.drop().await?;
            Ok(())
        })
    }

    /// Fetch collection stats (runs in Tokio runtime)
    pub fn collection_stats(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
    ) -> Result<mongodb::bson::Document> {
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
    pub fn database_stats(
        &self,
        client: &Client,
        database: &str,
    ) -> Result<mongodb::bson::Document> {
        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let stats = db.run_command(doc! { "dbStats": 1 }).await?;
            Ok(stats)
        })
    }

    /// Find documents in a collection with pagination (runs in Tokio runtime)
    pub fn find_documents(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        opts: FindDocumentsOptions,
    ) -> Result<(Vec<mongodb::bson::Document>, u64)> {
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let filter = opts.filter.unwrap_or_default();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);

            // Get total count (with filter)
            let total = coll.count_documents(filter.clone()).await?;

            // Fetch documents with pagination
            let mut options = mongodb::options::FindOptions::default();
            options.skip = Some(opts.skip);
            options.limit = Some(opts.limit);
            options.sort = opts.sort;
            options.projection = opts.projection;

            let cursor = coll.find(filter).with_options(options).await?;
            let documents: Vec<mongodb::bson::Document> = cursor.try_collect().await?;

            Ok((documents, total))
        })
    }

    /// Insert a document into a collection (runs in Tokio runtime)
    pub fn insert_document(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        document: mongodb::bson::Document,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
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
        documents: Vec<mongodb::bson::Document>,
    ) -> Result<usize> {
        let count = documents.len();
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
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
        filter: mongodb::bson::Document,
    ) -> Result<u64> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            let result = coll.delete_many(filter).await?;
            Ok(result.deleted_count)
        })
    }

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
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            let cursor = coll.list_indexes().await?;
            let indexes: Vec<IndexModel> = cursor.try_collect().await?;
            Ok(indexes)
        })
    }

    /// Sample documents from a collection (runs in Tokio runtime)
    pub fn sample_documents(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        size: i64,
    ) -> Result<Vec<mongodb::bson::Document>> {
        use futures::TryStreamExt;

        if size <= 0 {
            return Ok(Vec::new());
        }

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            let pipeline = vec![doc! { "$sample": { "size": size } }];
            let cursor = coll.aggregate(pipeline).await?;
            let docs: Vec<mongodb::bson::Document> = cursor.try_collect().await?;
            Ok(docs)
        })
    }

    /// Run an aggregation pipeline for a collection with abort support (runs in Tokio runtime)
    #[allow(clippy::too_many_arguments)]
    pub fn aggregate_pipeline_abortable(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        mut pipeline: Vec<mongodb::bson::Document>,
        limit: Option<i64>,
        append_limit: bool,
        abort_registration: futures::future::AbortRegistration,
    ) -> std::result::Result<Vec<mongodb::bson::Document>, AggregatePipelineError> {
        use futures::TryStreamExt;
        use futures::future::Abortable;

        if append_limit
            && let Some(limit) = limit
            && limit > 0
        {
            pipeline.push(doc! { "$limit": limit });
        }

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            let fut = async move {
                let cursor = coll.aggregate(pipeline).await?;
                let docs: Vec<mongodb::bson::Document> = cursor.try_collect().await?;
                Ok::<_, crate::error::Error>(docs)
            };
            match Abortable::new(fut, abort_registration).await {
                Ok(result) => result.map_err(AggregatePipelineError::from),
                Err(_aborted) => Err(AggregatePipelineError::Aborted),
            }
        })
    }

    /// Create an index for a collection (runs in Tokio runtime)
    pub fn create_index(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        index: mongodb::bson::Document,
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
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            coll.drop_index(name).await?;
            Ok(())
        })
    }

    /// Update a single document (runs in Tokio runtime)
    pub fn update_one(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        filter: mongodb::bson::Document,
        update: mongodb::bson::Document,
    ) -> Result<UpdateResult> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
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
        filter: mongodb::bson::Document,
        update: mongodb::bson::Document,
    ) -> Result<UpdateResult> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
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
        replacement: mongodb::bson::Document,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let id = id.clone();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);

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
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);

            coll.delete_one(doc! { "_id": id }).await?;
            Ok(())
        })
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

// No public server info returned yet.

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{Bson, Document};
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct DbCleanup<'a> {
        manager: &'a ConnectionManager,
        client: Client,
        database: String,
    }

    impl<'a> Drop for DbCleanup<'a> {
        fn drop(&mut self) {
            let _ = self.manager.drop_database(&self.client, &self.database);
        }
    }

    fn test_uri() -> Option<String> {
        env::var("MONGO_URI").ok().filter(|value| !value.trim().is_empty())
    }

    fn unique_db_name(prefix: &str) -> String {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let suffix = format!("{}_{}", std::process::id(), now.as_millis());
        format!("om_smoke_{prefix}_{suffix}")
    }

    #[test]
    fn smoke_core_flows() -> Result<()> {
        let uri = match test_uri() {
            Some(value) => value,
            None => {
                eprintln!("Skipping smoke_core_flows: MONGO_URI not set.");
                return Ok(());
            }
        };

        let manager = get_connection_manager();
        let connection = SavedConnection::new("Smoke Test".to_string(), uri);

        manager.test_connection(&connection, Duration::from_secs(5))?;
        let client = manager.connect(&connection)?;

        let databases = manager.list_databases(&client)?;
        if databases.is_empty() {
            return Ok(());
        }

        let db_name = env::var("MONGO_DB")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| databases[0].clone());
        let collections = manager.list_collections(&client, &db_name)?;
        if collections.is_empty() {
            return Ok(());
        }

        let collection = env::var("MONGO_COLLECTION")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| collections[0].clone());

        let _ = manager.find_documents(
            &client,
            &db_name,
            &collection,
            FindDocumentsOptions { filter: None, sort: None, projection: None, skip: 0, limit: 1 },
        )?;

        Ok(())
    }

    #[test]
    fn crud_sanity() -> Result<()> {
        let uri = match test_uri() {
            Some(value) => value,
            None => {
                eprintln!("Skipping crud_sanity: MONGO_URI not set.");
                return Ok(());
            }
        };

        let manager = get_connection_manager();
        let connection = SavedConnection::new("Smoke CRUD".to_string(), uri);
        manager.test_connection(&connection, Duration::from_secs(5))?;
        let client = manager.connect(&connection)?;

        let database = unique_db_name("crud");
        let collection = "docs";
        let _cleanup = DbCleanup { manager, client: client.clone(), database: database.clone() };

        let doc_a = doc! { "_id": "a", "name": "first", "n": 1 };
        let doc_b = doc! { "_id": "b", "name": "second", "n": 2 };
        manager.insert_document(&client, &database, collection, doc_a)?;
        manager.insert_document(&client, &database, collection, doc_b)?;

        let (docs, total) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions { filter: None, sort: None, projection: None, skip: 0, limit: 10 },
        )?;
        if total < 2 || docs.len() < 2 {
            return Err(Error::Timeout("CRUD insert failed".to_string()));
        }

        let updated = doc! { "_id": "a", "name": "updated", "n": 10 };
        manager.replace_document(
            &client,
            &database,
            collection,
            &Bson::String("a".into()),
            updated,
        )?;

        let (docs, _) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions {
                filter: Some(doc! { "_id": "a" }),
                sort: None,
                projection: None,
                skip: 0,
                limit: 1,
            },
        )?;
        let updated_name =
            docs.first().and_then(|doc| doc.get_str("name").ok()).unwrap_or_default();
        if updated_name != "updated" {
            return Err(Error::Timeout("CRUD update failed".to_string()));
        }

        manager.delete_document(&client, &database, collection, &Bson::String("b".into()))?;

        let (_, total) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions { filter: None, sort: None, projection: None, skip: 0, limit: 10 },
        )?;
        if total != 1 {
            return Err(Error::Timeout("CRUD delete failed".to_string()));
        }

        Ok(())
    }

    #[test]
    fn query_sanity() -> Result<()> {
        let uri = match test_uri() {
            Some(value) => value,
            None => {
                eprintln!("Skipping query_sanity: MONGO_URI not set.");
                return Ok(());
            }
        };

        let manager = get_connection_manager();
        let connection = SavedConnection::new("Smoke Query".to_string(), uri);
        manager.test_connection(&connection, Duration::from_secs(5))?;
        let client = manager.connect(&connection)?;

        let database = unique_db_name("query");
        let collection = "docs";
        let _cleanup = DbCleanup { manager, client: client.clone(), database: database.clone() };

        let docs = vec![
            doc! { "_id": 1, "value": "b", "n": 2 },
            doc! { "_id": 2, "value": "a", "n": 1 },
            doc! { "_id": 3, "value": "c", "n": 3 },
        ];
        for doc in docs {
            manager.insert_document(&client, &database, collection, doc)?;
        }

        let (filtered, total) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions {
                filter: Some(doc! { "value": "a" }),
                sort: None,
                projection: None,
                skip: 0,
                limit: 10,
            },
        )?;
        if total != 1 || filtered.len() != 1 {
            return Err(Error::Timeout("Query filter failed".to_string()));
        }

        let (sorted, _) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions {
                filter: None,
                sort: Some(doc! { "n": 1 }),
                projection: None,
                skip: 0,
                limit: 3,
            },
        )?;
        let first_n = sorted.first().and_then(|doc| doc.get_i32("n").ok()).unwrap_or_default();
        if first_n != 1 {
            return Err(Error::Timeout("Query sort failed".to_string()));
        }

        let (paged, _) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions {
                filter: None,
                sort: Some(doc! { "n": 1 }),
                projection: None,
                skip: 1,
                limit: 1,
            },
        )?;
        let paged_n = paged.first().and_then(|doc| doc.get_i32("n").ok()).unwrap_or_default();
        if paged_n != 2 {
            return Err(Error::Timeout("Query pagination failed".to_string()));
        }

        let (projected, _) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions {
                filter: None,
                sort: None,
                projection: Some(doc! { "value": 1, "_id": 0 }),
                skip: 0,
                limit: 1,
            },
        )?;
        let projected_doc =
            projected.first().ok_or_else(|| Error::Timeout("Projection failed".to_string()))?;
        if projected_doc.get("_id").is_some() || projected_doc.get("value").is_none() {
            return Err(Error::Timeout("Query projection failed".to_string()));
        }

        Ok(())
    }

    #[test]
    fn indexes_sanity() -> Result<()> {
        let uri = match test_uri() {
            Some(value) => value,
            None => {
                eprintln!("Skipping indexes_sanity: MONGO_URI not set.");
                return Ok(());
            }
        };

        let manager = get_connection_manager();
        let connection = SavedConnection::new("Smoke Indexes".to_string(), uri);
        manager.test_connection(&connection, Duration::from_secs(5))?;
        let client = manager.connect(&connection)?;

        let database = unique_db_name("indexes");
        let collection = "docs";
        let _cleanup = DbCleanup { manager, client: client.clone(), database: database.clone() };

        manager.insert_document(&client, &database, collection, doc! { "_id": 1, "n": 1 })?;

        let index = IndexModel::builder().keys(doc! { "n": 1 }).build();

        manager.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(collection);
            coll.create_index(index).await.map(|_| ())
        })?;

        let indexes = manager.list_indexes(&client, &database, collection)?;
        let created = indexes.iter().find(|model| model.keys == doc! { "n": 1 });
        let Some(created) = created else {
            return Err(Error::Timeout("Index list failed".to_string()));
        };
        let Some(name) = created.options.as_ref().and_then(|options| options.name.as_ref()) else {
            return Err(Error::Timeout("Index name missing".to_string()));
        };

        manager.drop_index(&client, &database, collection, name)?;

        let indexes = manager.list_indexes(&client, &database, collection)?;
        let still_has_index = indexes.iter().any(|model| model.keys == doc! { "n": 1 });
        if still_has_index {
            return Err(Error::Timeout("Index drop failed".to_string()));
        }

        Ok(())
    }

    #[test]
    fn stats_sanity() -> Result<()> {
        let uri = match test_uri() {
            Some(value) => value,
            None => {
                eprintln!("Skipping stats_sanity: MONGO_URI not set.");
                return Ok(());
            }
        };

        let manager = get_connection_manager();
        let connection = SavedConnection::new("Smoke Stats".to_string(), uri);
        manager.test_connection(&connection, Duration::from_secs(5))?;
        let client = manager.connect(&connection)?;

        let database = unique_db_name("stats");
        let collection = "docs";
        let _cleanup = DbCleanup { manager, client: client.clone(), database: database.clone() };

        manager.insert_document(&client, &database, collection, doc! { "_id": 1, "n": 1 })?;

        let db_stats = manager.database_stats(&client, &database)?;
        if db_stats.get("db").is_none() {
            return Err(Error::Timeout("Database stats missing".to_string()));
        }

        let coll_stats = manager.collection_stats(&client, &database, collection)?;
        if coll_stats.get("count").is_none() {
            return Err(Error::Timeout("Collection stats missing".to_string()));
        }

        Ok(())
    }
}
