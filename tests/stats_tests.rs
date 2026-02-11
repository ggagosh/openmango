//! Integration tests for MongoDB statistics operations using Testcontainers.

mod common;

use common::{MongoTestContainer, fixtures};
use mongodb::bson::{Document, doc};

// =============================================================================
// Collection Statistics Tests
// =============================================================================

/// Test collection stats for an empty collection.
#[tokio::test]
async fn test_collection_stats_empty() {
    let mongo = MongoTestContainer::start().await;
    let db = mongo.database("test_db");

    // Create empty collection
    db.create_collection("empty_stats").await.expect("Failed to create collection");

    // Get stats
    let stats =
        db.run_command(doc! { "collStats": "empty_stats" }).await.expect("Failed to get stats");

    // Verify stats
    assert_eq!(stats.get_i32("count").unwrap_or(0), 0);
    assert!(stats.get("storageSize").is_some());
    assert!(stats.get("totalIndexSize").is_some());
}

/// Test collection stats with data.
#[tokio::test]
async fn test_collection_stats_with_data() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "stats_with_data");

    // Insert documents
    let docs = fixtures::generate_test_documents(100);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Get stats
    let db = mongo.database("test_db");
    let stats =
        db.run_command(doc! { "collStats": "stats_with_data" }).await.expect("Failed to get stats");

    // Verify document count
    assert_eq!(stats.get_i32("count").unwrap_or(0), 100);

    // Verify storage stats exist
    assert!(stats.get("storageSize").is_some());
    assert!(stats.get("size").is_some());
    assert!(stats.get("avgObjSize").is_some());
    assert!(stats.get("totalIndexSize").is_some());
    assert!(stats.get("nindexes").is_some());

    // Should have at least 1 index (_id)
    assert!(stats.get_i32("nindexes").unwrap_or(0) >= 1);
}

/// Test collection stats for a capped collection.
#[tokio::test]
async fn test_collection_stats_capped() {
    let mongo = MongoTestContainer::start().await;
    let db = mongo.database("test_db");

    // Create capped collection
    db.run_command(doc! {
        "create": "capped_stats",
        "capped": true,
        "size": 1024 * 1024, // 1MB
        "max": 1000
    })
    .await
    .expect("Failed to create capped collection");

    // Insert some documents
    let collection = mongo.collection::<Document>("test_db", "capped_stats");
    let docs: Vec<Document> = (0..50).map(|i| doc! { "index": i, "data": "test data" }).collect();
    collection.insert_many(docs).await.expect("Failed to insert");

    // Get stats
    let stats =
        db.run_command(doc! { "collStats": "capped_stats" }).await.expect("Failed to get stats");

    // Verify it's capped
    assert!(stats.get_bool("capped").unwrap_or(false));

    // Verify document count
    assert_eq!(stats.get_i32("count").unwrap_or(0), 50);

    // Verify max is set
    let max = stats.get_i64("max").ok().or_else(|| stats.get_i32("max").ok().map(|v| v as i64));
    assert!(max.is_some());
}

// =============================================================================
// Database Statistics Tests
// =============================================================================

/// Test database-level statistics.
#[tokio::test]
async fn test_database_stats() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("stats_test_db", "test_collection");

    // Insert some data
    let docs = fixtures::generate_test_documents(50);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Get database stats
    let db = mongo.database("stats_test_db");
    let stats = db.run_command(doc! { "dbStats": 1 }).await.expect("Failed to get stats");

    // Verify database stats
    assert_eq!(stats.get_str("db").unwrap(), mongo.db_name("stats_test_db"));
    assert!(stats.get("collections").is_some());
    assert!(stats.get("dataSize").is_some());
    assert!(stats.get("storageSize").is_some());
    assert!(stats.get("indexes").is_some());
    assert!(stats.get("indexSize").is_some());

    // Should have at least 1 collection
    let collections = stats
        .get_i64("collections")
        .ok()
        .or_else(|| stats.get_i32("collections").ok().map(|v| v as i64))
        .unwrap_or(0);
    assert!(collections >= 1);
}

/// Test database stats with multiple collections.
#[tokio::test]
async fn test_database_stats_multiple_collections() {
    let mongo = MongoTestContainer::start().await;

    // Create multiple collections with data
    for i in 1..=5 {
        let coll_name = format!("collection_{}", i);
        let collection = mongo.collection::<Document>("multi_coll_db", &coll_name);
        let docs = fixtures::generate_test_documents(20 * i);
        collection.insert_many(docs).await.expect("Failed to insert");
    }

    // Get database stats
    let db = mongo.database("multi_coll_db");
    let stats = db.run_command(doc! { "dbStats": 1 }).await.expect("Failed to get stats");

    // Verify collection count
    let collections = stats
        .get_i64("collections")
        .ok()
        .or_else(|| stats.get_i32("collections").ok().map(|v| v as i64))
        .unwrap_or(0);
    assert_eq!(collections, 5);

    // Verify aggregate object count (20 + 40 + 60 + 80 + 100 = 300)
    let objects =
        stats.get_i64("objects").ok().or_else(|| stats.get_i32("objects").ok().map(|v| v as i64));
    assert!(objects.is_some());
    assert_eq!(objects.unwrap(), 300);
}

// =============================================================================
// Server Statistics Tests
// =============================================================================

/// Test getting server status.
#[tokio::test]
async fn test_server_status() {
    let mongo = MongoTestContainer::start().await;
    let db = mongo.database("admin");

    // Get server status
    let status = db.run_command(doc! { "serverStatus": 1 }).await.expect("Failed to get status");

    // Verify basic fields
    assert!(status.get_str("host").is_ok());
    assert!(status.get_str("version").is_ok());
    assert!(status.get("uptime").is_some());
    assert!(status.get("localTime").is_some());

    // Verify connections info
    assert!(status.get_document("connections").is_ok());

    // Verify memory info
    let mem = status.get_document("mem");
    assert!(mem.is_ok());
}

/// Test getting current operations.
#[tokio::test]
async fn test_current_op() {
    let mongo = MongoTestContainer::start().await;
    let db = mongo.client.database("admin");

    // Get current operations
    let result = db.run_command(doc! { "currentOp": 1 }).await.expect("Failed to get currentOp");

    // Should have inprog array
    assert!(result.get_array("inprog").is_ok());
}

/// Test getting build info.
#[tokio::test]
async fn test_build_info() {
    let mongo = MongoTestContainer::start().await;
    let db = mongo.database("admin");

    // Get build info
    let info = db.run_command(doc! { "buildInfo": 1 }).await.expect("Failed to get buildInfo");

    // Verify basic fields
    assert!(info.get_str("version").is_ok());
    assert!(info.get_str("gitVersion").is_ok());
    assert!(info.get("bits").is_some());
    assert!(info.get("maxBsonObjectSize").is_some());
}

// =============================================================================
// Storage Statistics Tests
// =============================================================================

/// Test getting storage stats with index details.
#[tokio::test]
async fn test_collection_stats_with_indexes() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "stats_indexes");

    // Insert documents
    let docs = fixtures::generate_test_documents(50);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Create additional indexes
    collection
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "name": 1 })
                .options(
                    mongodb::options::IndexOptions::builder().name("name_idx".to_string()).build(),
                )
                .build(),
        )
        .await
        .expect("Failed to create index");

    collection
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "value": 1 })
                .options(
                    mongodb::options::IndexOptions::builder().name("value_idx".to_string()).build(),
                )
                .build(),
        )
        .await
        .expect("Failed to create index");

    // Get stats (indexDetails option removed in MongoDB 7.0)
    let db = mongo.database("test_db");
    let stats =
        db.run_command(doc! { "collStats": "stats_indexes" }).await.expect("Failed to get stats");

    // Should have 3 indexes (_id, name, value)
    assert_eq!(stats.get_i32("nindexes").unwrap_or(0), 3);

    // Verify indexSizes exists
    let index_sizes = stats.get_document("indexSizes");
    assert!(index_sizes.is_ok());
}

/// Test estimated document count (fast count).
#[tokio::test]
async fn test_estimated_document_count() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "estimated_count");

    // Insert documents
    let docs = fixtures::generate_test_documents(200);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Get estimated count (faster than count_documents for large collections)
    let estimated =
        collection.estimated_document_count().await.expect("Failed to get estimated count");

    // Should be accurate for this small collection
    assert_eq!(estimated, 200);
}

/// Test distinct values count.
#[tokio::test]
async fn test_distinct_values() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "distinct_test");

    // Insert documents with known distinct values
    let docs = fixtures::generate_test_documents(100);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Get distinct categories
    let distinct: Vec<String> = collection
        .distinct("category", doc! {})
        .await
        .expect("Failed to get distinct")
        .into_iter()
        .filter_map(|b| b.as_str().map(String::from))
        .collect();

    // Should have exactly 2 distinct categories: "even" and "odd"
    assert_eq!(distinct.len(), 2);
    assert!(distinct.contains(&"even".to_string()));
    assert!(distinct.contains(&"odd".to_string()));
}
