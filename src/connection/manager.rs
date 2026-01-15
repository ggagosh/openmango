use std::sync::LazyLock;

use mongodb::Client;
use mongodb::bson::doc;
use mongodb::IndexModel;
use mongodb::results::CollectionSpecification;
use std::time::Duration;
use tokio::runtime::Runtime;

use crate::error::{Error, Result};
use crate::models::SavedConnection;

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
            let databases = client.list_database_names().await?;
            Ok(databases)
        })
    }

    /// List collections in a database (runs in Tokio runtime)
    pub fn list_collections(&self, client: &Client, database: &str) -> Result<Vec<String>> {
        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let collections = db.list_collection_names().await?;
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
            let specs: Vec<CollectionSpecification> = cursor.try_collect().await?;
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
