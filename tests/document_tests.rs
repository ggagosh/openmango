//! Integration tests for document CRUD operations using Testcontainers.

mod common;

use common::{MongoTestContainer, fixtures, test_document};
use mongodb::bson::doc;

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
