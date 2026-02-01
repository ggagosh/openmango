//! Integration tests for MongoDB connection lifecycle using Testcontainers.

mod common;

use common::MongoTestContainer;
use mongodb::bson::doc;

/// Test basic connection and database listing.
#[tokio::test]
async fn test_connection_and_list_databases() {
    let mongo = MongoTestContainer::start().await;

    // Should be able to list databases
    let databases = mongo.client.list_database_names().await.expect("Failed to list databases");

    // MongoDB always has admin, local, and config databases
    assert!(databases.contains(&"admin".to_string()));
    assert!(databases.contains(&"local".to_string()));
}

/// Test creating and dropping a database.
#[tokio::test]
async fn test_create_and_drop_database() {
    let mongo = MongoTestContainer::start().await;

    let db_name = "test_create_drop_db";
    let db = mongo.database(db_name);

    // Create a collection to ensure database exists
    db.create_collection("temp_collection").await.expect("Failed to create collection");

    // Verify database appears in list
    let databases = mongo.client.list_database_names().await.expect("Failed to list");
    assert!(databases.contains(&db_name.to_string()));

    // Drop the database
    db.drop().await.expect("Failed to drop database");

    // Verify it's gone
    let databases = mongo.client.list_database_names().await.expect("Failed to list");
    assert!(!databases.contains(&db_name.to_string()));
}

/// Test listing collections in a database.
#[tokio::test]
async fn test_list_collections() {
    let mongo = MongoTestContainer::start().await;
    let db = mongo.database("collection_test_db");

    // Create multiple collections
    db.create_collection("collection_a").await.expect("Failed to create");
    db.create_collection("collection_b").await.expect("Failed to create");
    db.create_collection("collection_c").await.expect("Failed to create");

    // List collections
    let collections = db.list_collection_names().await.expect("Failed to list collections");

    assert!(collections.contains(&"collection_a".to_string()));
    assert!(collections.contains(&"collection_b".to_string()));
    assert!(collections.contains(&"collection_c".to_string()));
    assert_eq!(collections.len(), 3);
}

/// Test creating and dropping collections.
#[tokio::test]
async fn test_create_and_drop_collection() {
    let mongo = MongoTestContainer::start().await;
    let db = mongo.database("drop_collection_test_db");

    // Create a collection
    db.create_collection("to_drop").await.expect("Failed to create");

    // Verify it exists
    let collections = db.list_collection_names().await.expect("Failed to list");
    assert!(collections.contains(&"to_drop".to_string()));

    // Drop it
    db.collection::<mongodb::bson::Document>("to_drop").drop().await.expect("Failed to drop");

    // Verify it's gone
    let collections = db.list_collection_names().await.expect("Failed to list");
    assert!(!collections.contains(&"to_drop".to_string()));
}

/// Test connection string is valid.
#[tokio::test]
async fn test_connection_string_format() {
    let mongo = MongoTestContainer::start().await;

    // Connection string should start with mongodb://
    assert!(mongo.connection_string.starts_with("mongodb://"));

    // Should be able to parse it again
    let opts = mongodb::options::ClientOptions::parse(&mongo.connection_string)
        .await
        .expect("Connection string should be valid");

    assert!(!opts.hosts.is_empty());
}

/// Test running a simple command.
#[tokio::test]
async fn test_run_command() {
    let mongo = MongoTestContainer::start().await;
    let db = mongo.database("admin");

    // Run serverStatus command
    let result = db.run_command(doc! { "serverStatus": 1 }).await.expect("Failed to run command");

    // Should have version info
    assert!(result.get_str("version").is_ok());
}

/// Test collection stats.
#[tokio::test]
async fn test_collection_stats() {
    let mongo = MongoTestContainer::start().await;
    let db = mongo.database("stats_test_db");
    let collection = db.collection::<mongodb::bson::Document>("stats_collection");

    // Insert some documents
    let docs: Vec<_> = (0..100).map(|i| doc! { "index": i, "data": "x".repeat(100) }).collect();
    collection.insert_many(docs).await.expect("Failed to insert");

    // Get collection stats using collStats command
    let stats = db
        .run_command(doc! { "collStats": "stats_collection" })
        .await
        .expect("Failed to get stats");

    // Should have count
    assert_eq!(stats.get_i32("count").unwrap_or(0), 100);
}
