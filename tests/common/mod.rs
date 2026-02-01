//! Common test utilities and fixtures for integration tests using Testcontainers.

#![allow(dead_code)]

pub mod fixtures;

use mongodb::bson::{Document, doc};
use mongodb::{Client, options::ClientOptions};
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;

/// A wrapper around a MongoDB testcontainer that provides convenient access.
pub struct MongoTestContainer {
    _container: ContainerAsync<Mongo>,
    pub client: Client,
    pub connection_string: String,
}

impl MongoTestContainer {
    /// Start a new MongoDB container and connect to it.
    pub async fn start() -> Self {
        let container = Mongo::default().start().await.expect("Failed to start MongoDB container");

        let host = container.get_host().await.expect("Failed to get container host");
        let port = container.get_host_port_ipv4(27017).await.expect("Failed to get container port");

        let connection_string = format!("mongodb://{}:{}", host, port);

        let client_options = ClientOptions::parse(&connection_string)
            .await
            .expect("Failed to parse connection string");
        let client = Client::with_options(client_options).expect("Failed to create client");

        // Wait for MongoDB to be ready
        for _ in 0..30 {
            if client.list_database_names().await.is_ok() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        Self { _container: container, client, connection_string }
    }

    /// Get a database for testing.
    pub fn database(&self, name: &str) -> mongodb::Database {
        self.client.database(name)
    }

    /// Get a collection for testing.
    pub fn collection<T: Send + Sync>(&self, db: &str, collection: &str) -> mongodb::Collection<T> {
        self.database(db).collection(collection)
    }
}

/// Create a simple test document with common fields.
pub fn test_document(name: &str) -> Document {
    doc! {
        "name": name,
        "value": 42,
        "active": true,
    }
}

/// Create a test document with an explicit _id field.
pub fn test_document_with_id(id: &str, name: &str) -> Document {
    doc! {
        "_id": id,
        "name": name,
        "value": 42,
        "active": true,
    }
}
