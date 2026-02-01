//! Core ConnectionManager struct and basic connection methods.

use std::time::Duration;

use mongodb::Client;
use mongodb::bson::doc;
use mongodb::results::CollectionSpecification;
use tokio::runtime::Runtime;

use crate::error::{Error, Result};
use crate::models::SavedConnection;

/// Manages MongoDB client connections with caching
pub struct ConnectionManager {
    /// Tokio runtime for MongoDB async operations
    pub(crate) runtime: Runtime,
}

impl ConnectionManager {
    /// Create a new connection manager
    pub fn new() -> Self {
        let runtime = Runtime::new().expect("Failed to create Tokio runtime");
        Self { runtime }
    }

    /// Get a handle to the Tokio runtime for spawning parallel tasks
    pub fn runtime_handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
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

    /// List all collection names in a database (runs in Tokio runtime).
    pub fn list_collection_names(&self, client: &Client, database: &str) -> Result<Vec<String>> {
        let client = client.clone();
        let database = database.to_string();

        self.runtime.block_on(async {
            let db = client.database(&database);
            let names = db.list_collection_names().await?;
            Ok(names)
        })
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}
