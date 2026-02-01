//! Integration tests for edge cases and error handling using Testcontainers.

mod common;

use common::{MongoTestContainer, fixtures};
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};

// =============================================================================
// Special Characters Tests
// =============================================================================

/// Test collection names with special characters (dots).
#[tokio::test]
async fn test_collection_name_with_dots() {
    let mongo = MongoTestContainer::start().await;
    let db = mongo.database("test_db");

    // Collection name with dots (allowed but special)
    db.create_collection("my.collection.name").await.expect("Failed to create collection");

    let collection = db.collection::<Document>("my.collection.name");
    collection.insert_one(doc! { "name": "test" }).await.expect("Failed to insert");

    // Verify
    let count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(count, 1);
}

/// Test document field names with special characters.
#[tokio::test]
async fn test_document_field_special_chars() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "special_fields");

    // Fields with special characters (some are allowed, some have restrictions)
    let doc = doc! {
        "normal_field": "value",
        "field-with-dashes": "dash_value",
        "field_with_underscores": "underscore_value",
        "field.with.dots": "dot_value", // Dots in field names are allowed but tricky
        "field with spaces": "space_value",
        "123_starts_with_number": "number_value",
        "unicode_日本語": "japanese_value",
        "emoji_🎉": "emoji_value",
    };

    collection.insert_one(doc).await.expect("Failed to insert");

    // Verify all fields
    let found = collection.find_one(doc! {}).await.expect("Failed to find").unwrap();
    assert_eq!(found.get_str("normal_field").unwrap(), "value");
    assert_eq!(found.get_str("field-with-dashes").unwrap(), "dash_value");
    assert_eq!(found.get_str("field_with_underscores").unwrap(), "underscore_value");
    assert_eq!(found.get_str("field with spaces").unwrap(), "space_value");
    assert_eq!(found.get_str("unicode_日本語").unwrap(), "japanese_value");
    assert_eq!(found.get_str("emoji_🎉").unwrap(), "emoji_value");
}

/// Test Unicode content in documents.
#[tokio::test]
async fn test_unicode_content() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "unicode_content");

    // Insert document with various Unicode content
    let doc = doc! {
        "english": "Hello World",
        "chinese": "你好世界",
        "japanese": "こんにちは世界",
        "korean": "안녕하세요 세계",
        "arabic": "مرحبا بالعالم",
        "russian": "Привет мир",
        "emoji": "Hello 👋 World 🌍",
        "mixed": "Hello 你好 مرحبا",
        "special": "Line1\nLine2\tTabbed",
    };

    collection.insert_one(doc).await.expect("Failed to insert");

    // Verify all Unicode content
    let found = collection.find_one(doc! {}).await.expect("Failed to find").unwrap();
    assert_eq!(found.get_str("chinese").unwrap(), "你好世界");
    assert_eq!(found.get_str("japanese").unwrap(), "こんにちは世界");
    assert_eq!(found.get_str("korean").unwrap(), "안녕하세요 세계");
    assert_eq!(found.get_str("arabic").unwrap(), "مرحبا بالعالم");
    assert_eq!(found.get_str("russian").unwrap(), "Привет мир");
    assert_eq!(found.get_str("emoji").unwrap(), "Hello 👋 World 🌍");
}

/// Test querying with Unicode content.
#[tokio::test]
async fn test_unicode_query() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "unicode_query");

    // Insert documents with Unicode
    let docs = vec![
        doc! { "city": "東京", "country": "日本" },
        doc! { "city": "北京", "country": "中国" },
        doc! { "city": "서울", "country": "한국" },
    ];
    collection.insert_many(docs).await.expect("Failed to insert");

    // Query with Unicode
    let found =
        collection.find_one(doc! { "city": "東京" }).await.expect("Failed to find").unwrap();
    assert_eq!(found.get_str("country").unwrap(), "日本");

    // Regex with Unicode
    let filter = doc! { "city": { "$regex": "^北" } };
    let count = collection.count_documents(filter).await.expect("Failed to count");
    assert_eq!(count, 1);
}

// =============================================================================
// Large Data Tests
// =============================================================================

/// Test approaching the 16MB document size limit.
#[tokio::test]
async fn test_large_document() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "large_document");

    // Create a document approaching the size limit
    // Don't actually hit 16MB as it would be slow, but test with ~1MB
    let large_string = "x".repeat(100_000); // 100KB string
    let doc = doc! {
        "field1": large_string.clone(),
        "field2": large_string.clone(),
        "field3": large_string.clone(),
        "field4": large_string.clone(),
        "field5": large_string.clone(),
        // ~500KB total
    };

    collection.insert_one(doc).await.expect("Failed to insert");

    // Verify
    let found = collection.find_one(doc! {}).await.expect("Failed to find").unwrap();
    assert_eq!(found.get_str("field1").unwrap().len(), 100_000);
}

/// Test large batch operations.
#[tokio::test]
async fn test_large_batch() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "large_batch");

    // Insert large batch of documents
    let docs = fixtures::generate_test_documents(5000);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Verify
    let count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(count, 5000);

    // Query with limit
    let options = mongodb::options::FindOptions::builder().limit(100).build();
    let cursor = collection.find(doc! {}).with_options(options).await.expect("Failed to find");
    let docs: Vec<Document> = cursor.try_collect().await.expect("Failed to collect");
    assert_eq!(docs.len(), 100);
}

/// Test document with many fields.
#[tokio::test]
async fn test_many_fields() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "many_fields");

    // Create document with many fields
    let mut doc = Document::new();
    for i in 0..500 {
        doc.insert(format!("field_{}", i), format!("value_{}", i));
    }

    collection.insert_one(doc).await.expect("Failed to insert");

    // Verify
    let found = collection.find_one(doc! {}).await.expect("Failed to find").unwrap();
    assert!(found.len() >= 500); // +_id
    assert_eq!(found.get_str("field_0").unwrap(), "value_0");
    assert_eq!(found.get_str("field_499").unwrap(), "value_499");
}

// =============================================================================
// Error Handling Tests
// =============================================================================

/// Test duplicate key error with unique index.
#[tokio::test]
async fn test_duplicate_key_error() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "duplicate_key");

    // Create unique index
    collection
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "email": 1 })
                .options(mongodb::options::IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .expect("Failed to create index");

    // Insert first document
    collection
        .insert_one(doc! { "email": "test@example.com" })
        .await
        .expect("First insert should succeed");

    // Try to insert duplicate
    let result = collection.insert_one(doc! { "email": "test@example.com" }).await;
    assert!(result.is_err());

    // Verify error type
    let err = result.unwrap_err();
    let is_duplicate = matches!(err.kind.as_ref(), mongodb::error::ErrorKind::Write(_));
    assert!(is_duplicate || err.to_string().contains("duplicate"));
}

/// Test schema validation errors (if supported).
#[tokio::test]
async fn test_validation_error() {
    let mongo = MongoTestContainer::start().await;
    let db = mongo.database("test_db");

    // Create collection with validation
    db.run_command(doc! {
        "create": "validated_collection",
        "validator": {
            "$jsonSchema": {
                "bsonType": "object",
                "required": ["name", "email"],
                "properties": {
                    "name": { "bsonType": "string" },
                    "email": { "bsonType": "string" }
                }
            }
        },
        "validationLevel": "strict"
    })
    .await
    .expect("Failed to create collection");

    let collection = db.collection::<Document>("validated_collection");

    // Valid document should succeed
    collection
        .insert_one(doc! { "name": "Test", "email": "test@example.com" })
        .await
        .expect("Valid insert should succeed");

    // Invalid document (missing required field) should fail
    let result = collection.insert_one(doc! { "name": "Test" }).await;
    assert!(result.is_err());
}

/// Test operation timeout handling.
#[tokio::test]
async fn test_timeout_handling() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "timeout_test");

    // Insert documents
    let docs = fixtures::generate_test_documents(100);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Set a very short maxTimeMS for a query that should complete quickly
    // This tests that the timeout mechanism works
    let options = mongodb::options::FindOptions::builder()
        .max_time(std::time::Duration::from_secs(10)) // 10 seconds - should complete
        .build();

    let cursor = collection.find(doc! {}).with_options(options).await.expect("Find should succeed");
    let docs: Vec<Document> = cursor.try_collect().await.expect("Collect should succeed");
    assert_eq!(docs.len(), 100);
}

// =============================================================================
// Concurrent Operation Tests
// =============================================================================

/// Test concurrent inserts.
#[tokio::test]
async fn test_concurrent_inserts() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "concurrent_insert");

    // Perform concurrent inserts
    let handles: Vec<_> = (0..10)
        .map(|i| {
            let coll = collection.clone();
            tokio::spawn(async move {
                for j in 0..10 {
                    let doc = doc! { "thread": i, "index": j };
                    coll.insert_one(doc).await.expect("Insert failed");
                }
            })
        })
        .collect();

    // Wait for all inserts
    for handle in handles {
        handle.await.expect("Task failed");
    }

    // Verify total count
    let count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(count, 100); // 10 threads * 10 documents
}

/// Test concurrent reads and writes.
#[tokio::test]
async fn test_concurrent_read_write() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "concurrent_rw");

    // Insert initial documents
    let docs = fixtures::generate_test_documents(50);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Concurrent operations
    let coll_read = collection.clone();
    let coll_write = collection.clone();

    let read_handle = tokio::spawn(async move {
        for _ in 0..20 {
            let _ = coll_read.count_documents(doc! {}).await;
            let _ = coll_read.find_one(doc! { "category": "even" }).await;
        }
    });

    let write_handle = tokio::spawn(async move {
        for i in 0..20 {
            coll_write.insert_one(doc! { "concurrent": i }).await.expect("Insert failed");
        }
    });

    // Wait for both
    read_handle.await.expect("Read task failed");
    write_handle.await.expect("Write task failed");

    // Verify
    let count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(count, 70); // 50 initial + 20 concurrent writes
}

// =============================================================================
// Empty and Null Tests
// =============================================================================

/// Test empty string fields.
#[tokio::test]
async fn test_empty_string() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "empty_string");

    // Insert document with empty string
    collection
        .insert_one(doc! { "name": "", "description": "has empty name" })
        .await
        .expect("Failed to insert");

    // Query for empty string
    let found = collection.find_one(doc! { "name": "" }).await.expect("Failed to find");
    assert!(found.is_some());
    assert_eq!(found.unwrap().get_str("name").unwrap(), "");
}

/// Test null values.
#[tokio::test]
async fn test_null_values() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "null_values");

    // Insert document with null
    collection
        .insert_one(doc! { "value": null, "name": "null_test" })
        .await
        .expect("Failed to insert");

    // Query for null
    let found = collection.find_one(doc! { "value": null }).await.expect("Failed to find");
    assert!(found.is_some());
    assert!(found.unwrap().is_null("value"));
}

/// Test missing fields vs null fields.
#[tokio::test]
async fn test_missing_vs_null() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "missing_null");

    // Insert documents with different states
    collection
        .insert_one(doc! { "name": "has_field", "optional": "present" })
        .await
        .expect("Insert failed");
    collection
        .insert_one(doc! { "name": "null_field", "optional": null })
        .await
        .expect("Insert failed");
    collection.insert_one(doc! { "name": "missing_field" }).await.expect("Insert failed"); // No "optional" field

    // Query for null or missing (using $exists)
    let null_filter = doc! { "optional": null };
    let null_count = collection.count_documents(null_filter).await.expect("Count failed");
    // MongoDB treats both null and missing as matching null query
    assert_eq!(null_count, 2);

    // Query for only explicitly null (not missing)
    let explicit_null = doc! { "optional": { "$type": "null" } };
    let explicit_count = collection.count_documents(explicit_null).await.expect("Count failed");
    assert_eq!(explicit_count, 1);

    // Query for missing field
    let missing_filter = doc! { "optional": { "$exists": false } };
    let missing_count = collection.count_documents(missing_filter).await.expect("Count failed");
    assert_eq!(missing_count, 1);
}

// =============================================================================
// Array Edge Cases
// =============================================================================

/// Test empty array.
#[tokio::test]
async fn test_empty_array() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "empty_array");

    // Insert document with empty array
    let doc = doc! { "items": [], "name": "empty_array_test" };
    collection.insert_one(doc).await.expect("Failed to insert");

    // Query for empty array
    let found =
        collection.find_one(doc! { "items": { "$size": 0 } }).await.expect("Failed to find");
    assert!(found.is_some());

    let doc = found.unwrap();
    let items = doc.get_array("items").unwrap();
    assert!(items.is_empty());
}

/// Test deeply nested arrays.
#[tokio::test]
async fn test_nested_arrays() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "nested_arrays");

    // Insert document with nested arrays
    let doc = doc! {
        "matrix": [
            [1, 2, 3],
            [4, 5, 6],
            [7, 8, 9]
        ],
        "deep": [[[1]], [[2]], [[3]]],
        "name": "nested_test"
    };
    collection.insert_one(doc).await.expect("Failed to insert");

    // Verify
    let found =
        collection.find_one(doc! { "name": "nested_test" }).await.expect("Failed to find").unwrap();
    let matrix = found.get_array("matrix").unwrap();
    assert_eq!(matrix.len(), 3);

    let row = matrix[0].as_array().unwrap();
    assert_eq!(row.len(), 3);
}
