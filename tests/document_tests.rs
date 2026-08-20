//! Integration tests for document CRUD operations using Testcontainers.

mod common;

use std::time::Duration;

use common::{MongoTestContainer, fixtures, test_document};
use mongodb::bson::{Document, doc, oid::ObjectId};
use openmango::connection::{CancellationToken, ConnectionManager, FindDocumentsOptions};

#[tokio::test]
async fn test_find_documents_honors_preexisting_cancellation() {
    let mongo = MongoTestContainer::start().await;
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");

    let error = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().find_documents(
            &client,
            &database,
            "cancelled_query",
            FindDocumentsOptions {
                filter: None,
                sort: None,
                projection: None,
                skip: 0,
                limit: 50,
                max_time: Duration::from_secs(30),
                cancellation,
            },
        )
    })
    .await
    .expect("Query task panicked")
    .expect_err("Cancelled query should fail");

    assert!(error.to_string().contains("Query cancelled"));
}

#[tokio::test]
async fn test_find_documents_returns_server_query_errors() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "failed_query");
    collection.insert_one(doc! { "value": 1 }).await.unwrap();
    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");

    let result = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().find_documents(
            &client,
            &database,
            "failed_query",
            FindDocumentsOptions {
                filter: Some(doc! { "$expr": { "$divide": [1, 0] } }),
                sort: None,
                projection: None,
                skip: 0,
                limit: 50,
                max_time: Duration::from_secs(30),
                cancellation: CancellationToken::new(),
            },
        )
    })
    .await
    .expect("Query task panicked");

    assert!(result.is_err(), "server query error must not become an empty result");
}

#[tokio::test]
async fn test_find_documents_sends_configured_max_time_to_server() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.db_name("test_db");
    let db = mongo.client.database(&database);
    db.run_command(doc! { "profile": 2 }).await.expect("Failed to enable profiler");
    db.collection::<Document>("profiled_query").insert_one(doc! { "value": 1 }).await.unwrap();
    let client = mongo.client.clone();
    let query_database = database.clone();

    tokio::task::spawn_blocking(move || {
        ConnectionManager::new().find_documents(
            &client,
            &query_database,
            "profiled_query",
            FindDocumentsOptions {
                filter: None,
                sort: None,
                projection: None,
                skip: 0,
                limit: 50,
                max_time: Duration::from_millis(12_345),
                cancellation: CancellationToken::new(),
            },
        )
    })
    .await
    .expect("Query task panicked")
    .expect("Query failed");

    let profile = db.collection::<Document>("system.profile");
    let profiled_find = profile
        .find_one(doc! {
            "command.find": "profiled_query",
            "command.maxTimeMS": 12_345_i64,
        })
        .await
        .expect("Find profile lookup failed");
    assert!(profiled_find.is_some(), "find command did not receive configured maxTimeMS");

    // The driver implements count_documents with an aggregate command on modern MongoDB.
    let profiled_count = profile
        .find_one(doc! {
            "command.aggregate": "profiled_query",
            "command.maxTimeMS": 12_345_i64,
        })
        .await
        .expect("Count profile lookup failed");
    assert!(profiled_count.is_some(), "count command did not receive configured maxTimeMS");
}

/// Test inserting and retrieving a single document.
#[tokio::test]
async fn test_insert_and_find_document() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "test_collection");

    // Insert a document
    let doc = test_document("test_item");
    collection.insert_one(doc.clone()).await.expect("Failed to insert document");

    // Find the document
    let filter = doc! { "name": "test_item" };
    let found = collection.find_one(filter).await.expect("Failed to find document");

    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.get_str("name").unwrap(), "test_item");
    assert_eq!(found.get_i32("value").unwrap(), 42);
}

/// Test inserting multiple documents.
#[tokio::test]
async fn test_insert_many_documents() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "bulk_collection");

    let docs = fixtures::generate_test_documents(50);
    let result = collection.insert_many(docs).await.expect("Failed to insert documents");

    assert_eq!(result.inserted_ids.len(), 50);

    let count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(count, 50);
}

/// Test updating a document.
#[tokio::test]
async fn test_update_document() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "update_collection");

    // Insert initial document
    let doc = test_document("update_me");
    collection.insert_one(doc).await.expect("Failed to insert");

    // Update the document
    let filter = doc! { "name": "update_me" };
    let update = doc! { "$set": { "value": 100, "updated": true } };
    collection.update_one(filter.clone(), update).await.expect("Failed to update");

    // Verify the update
    let found = collection.find_one(filter).await.expect("Failed to find").unwrap();
    assert_eq!(found.get_i32("value").unwrap(), 100);
    assert!(found.get_bool("updated").unwrap());
}

/// Test deleting a document.
#[tokio::test]
async fn test_delete_document() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "delete_collection");

    // Insert documents
    let docs = vec![test_document("keep_me"), test_document("delete_me")];
    collection.insert_many(docs).await.expect("Failed to insert");

    // Delete one document
    let filter = doc! { "name": "delete_me" };
    let result = collection.delete_one(filter).await.expect("Failed to delete");
    assert_eq!(result.deleted_count, 1);

    // Verify only one remains
    let count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(count, 1);

    // Verify the correct one remains
    let found = collection.find_one(doc! { "name": "keep_me" }).await.expect("Failed to find");
    assert!(found.is_some());
}

#[tokio::test]
async fn test_conditional_delete_and_restore_require_exact_document_state() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.db_name("test_db");
    let collection_name = "conditional_delete";
    let collection = mongo.client.database(&database).collection::<Document>(collection_name);
    let mut original = test_document("recover_me");
    original.insert("_id", ObjectId::new());
    let id = original.get("_id").unwrap().clone();
    collection.insert_one(original).await.expect("Failed to insert");
    let original = collection
        .find_one(doc! { "_id": id.clone() })
        .await
        .expect("Failed to read inserted document")
        .expect("Inserted document is missing");

    let client = mongo.client.clone();
    let database_for_task = database.clone();
    let original_for_task = original.clone();
    let id_for_task = id.clone();
    let (stale_delete, deleted, restored, duplicate_restore) =
        tokio::task::spawn_blocking(move || {
            let manager = ConnectionManager::new();
            let mut stale = original_for_task.clone();
            stale.insert("value", 99);
            let stale_delete = manager.delete_document_if_current_matches(
                &client,
                &database_for_task,
                collection_name,
                &id_for_task,
                &stale,
            )?;
            let deleted = manager.delete_document_if_current_matches(
                &client,
                &database_for_task,
                collection_name,
                &id_for_task,
                &original_for_task,
            )?;
            let restored = manager.insert_document_if_absent_matches(
                &client,
                &database_for_task,
                collection_name,
                original_for_task.clone(),
            )?;
            let duplicate_restore = manager.insert_document_if_absent_matches(
                &client,
                &database_for_task,
                collection_name,
                original_for_task,
            )?;
            Ok::<_, openmango::error::Error>((stale_delete, deleted, restored, duplicate_restore))
        })
        .await
        .expect("Conditional delete task panicked")
        .expect("Conditional delete failed");

    assert!(!stale_delete);
    assert!(deleted);
    assert!(restored);
    assert!(!duplicate_restore);
    assert_eq!(collection.find_one(doc! { "_id": id }).await.unwrap(), Some(original));
}

/// Test document with various BSON types.
#[tokio::test]
async fn test_document_with_various_types() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "types_collection");

    // Insert document with all types
    let doc = fixtures::document_with_all_types();
    let id = doc.get_object_id("_id").unwrap();
    collection.insert_one(doc).await.expect("Failed to insert");

    // Retrieve and verify
    let found = collection.find_one(doc! { "_id": id }).await.expect("Failed to find").unwrap();

    assert_eq!(found.get_str("string").unwrap(), "hello world");
    assert_eq!(found.get_i32("int32").unwrap(), 42);
    assert_eq!(found.get_i64("int64").unwrap(), 9_000_000_000_000_i64);
    assert!(found.get_bool("boolean").unwrap());
    assert!(found.is_null("null"));
    assert!(found.get_array("array").is_ok());
    assert!(found.get_document("nested").is_ok());
}

/// Test filtering with complex queries.
#[tokio::test]
async fn test_complex_filter_queries() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "filter_collection");

    // Insert test data
    let docs = fixtures::generate_test_documents(20);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Test range query
    let filter = doc! { "value": { "$gte": 50, "$lt": 100 } };
    let count = collection.count_documents(filter).await.expect("Failed to count");
    assert_eq!(count, 5); // indices 5,6,7,8,9 have values 50,60,70,80,90

    // Test regex query
    let filter = doc! { "name": { "$regex": "Document 1" } };
    let count = collection.count_documents(filter).await.expect("Failed to count");
    assert_eq!(count, 11); // Document 1, 10-19

    // Test nested field query
    let filter = doc! { "nested.number": { "$lt": 5 } };
    let count = collection.count_documents(filter).await.expect("Failed to count");
    assert_eq!(count, 5); // indices 0,1,2,3,4
}
