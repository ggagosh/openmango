//! Integration tests for Import/Export/Copy operations using Testcontainers.

mod common;

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};

use common::{MongoTestContainer, fixtures};
use futures::TryStreamExt;
use mongodb::bson::{Bson, Document, doc};
use openmango::connection::ConnectionManager;
use openmango::connection::types::{
    CancellationToken, CopyOptions, InsertMode, JsonExportOptions, JsonImportOptions,
    JsonTransferFormat, TargetWriteMode,
};
use tempfile::TempDir;

// =============================================================================
// JSON Export Tests
// =============================================================================

/// Test exporting a collection to JSON Lines format.
#[tokio::test]
async fn test_export_collection_jsonl() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "export_jsonl");

    // Insert test documents
    let docs = fixtures::generate_test_documents(10);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let export_path = temp_dir.path().join("export.jsonl");

    // Export using cursor iteration (JSONL format)
    let mut cursor = collection.find(doc! {}).await.expect("Failed to find");
    let mut file = File::create(&export_path).expect("Failed to create file");
    let mut count = 0u64;

    while let Some(doc) = cursor.try_next().await.expect("Cursor error") {
        let json = Bson::Document(doc).into_relaxed_extjson();
        let line = serde_json::to_string(&json).expect("Failed to serialize");
        writeln!(file, "{}", line).expect("Failed to write");
        count += 1;
    }

    assert_eq!(count, 10);

    // Verify file content
    let content = fs::read_to_string(&export_path).expect("Failed to read export file");
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 10);

    // Each line should be valid JSON
    for line in lines {
        let _: serde_json::Value = serde_json::from_str(line).expect("Invalid JSON line");
    }
}

/// Test exporting a collection to JSON array format.
#[tokio::test]
async fn test_export_collection_json_array() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "export_json_array");

    // Insert test documents
    let docs = fixtures::generate_test_documents(5);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let export_path = temp_dir.path().join("export.json");

    // Export as JSON array
    let cursor = collection.find(doc! {}).await.expect("Failed to find");
    let documents: Vec<Document> = cursor.try_collect().await.expect("Failed to collect");

    let json_values: Vec<serde_json::Value> =
        documents.into_iter().map(|doc| Bson::Document(doc).into_relaxed_extjson()).collect();

    let json_content = serde_json::to_string_pretty(&json_values).expect("Failed to serialize");
    fs::write(&export_path, json_content).expect("Failed to write");

    // Verify file content is a JSON array
    let content = fs::read_to_string(&export_path).expect("Failed to read export file");
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("Invalid JSON");
    assert!(parsed.is_array());
    assert_eq!(parsed.as_array().unwrap().len(), 5);
}

/// Test export with filter.
#[tokio::test]
async fn test_export_with_filter() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "export_filtered");

    // Insert test documents
    let docs = fixtures::generate_test_documents(20);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let export_path = temp_dir.path().join("export.jsonl");

    // Export with filter (only "even" category)
    let filter = doc! { "category": "even" };
    let mut cursor = collection.find(filter).await.expect("Failed to find");
    let mut file = File::create(&export_path).expect("Failed to create file");
    let mut count = 0u64;

    while let Some(doc) = cursor.try_next().await.expect("Cursor error") {
        let json = Bson::Document(doc).into_relaxed_extjson();
        let line = serde_json::to_string(&json).expect("Failed to serialize");
        writeln!(file, "{}", line).expect("Failed to write");
        count += 1;
    }

    // Should only export even-indexed documents (0, 2, 4, 6, 8, 10, 12, 14, 16, 18)
    assert_eq!(count, 10);
}

/// Test export with projection.
#[tokio::test]
async fn test_export_with_projection() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "export_projected");

    // Insert test documents
    let docs = fixtures::generate_test_documents(5);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let export_path = temp_dir.path().join("export.jsonl");

    // Export with projection (only name and index fields)
    let options = mongodb::options::FindOptions::builder()
        .projection(doc! { "name": 1, "index": 1, "_id": 0 })
        .build();
    let mut cursor = collection.find(doc! {}).with_options(options).await.expect("Failed to find");
    let mut file = File::create(&export_path).expect("Failed to create file");
    let mut count = 0u64;

    while let Some(doc) = cursor.try_next().await.expect("Cursor error") {
        let json = Bson::Document(doc).into_relaxed_extjson();
        let line = serde_json::to_string(&json).expect("Failed to serialize");
        writeln!(file, "{}", line).expect("Failed to write");
        count += 1;
    }

    assert_eq!(count, 5);

    // Verify exported documents only have projected fields
    let content = fs::read_to_string(&export_path).expect("Failed to read");
    for line in content.lines() {
        let doc: serde_json::Value = serde_json::from_str(line).expect("Invalid JSON");
        assert!(doc.get("name").is_some());
        assert!(doc.get("index").is_some());
        assert!(doc.get("_id").is_none());
        assert!(doc.get("category").is_none());
        assert!(doc.get("value").is_none());
    }
}

/// Test export with sort.
#[tokio::test]
async fn test_export_with_sort() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "export_sorted");

    // Insert test documents
    let docs = fixtures::generate_test_documents(10);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let export_path = temp_dir.path().join("export.jsonl");

    // Export with sort (descending by index)
    let options = mongodb::options::FindOptions::builder().sort(doc! { "index": -1 }).build();
    let mut cursor = collection.find(doc! {}).with_options(options).await.expect("Failed to find");
    let mut file = File::create(&export_path).expect("Failed to create file");
    let mut count = 0u64;

    while let Some(doc) = cursor.try_next().await.expect("Cursor error") {
        let json = Bson::Document(doc).into_relaxed_extjson();
        let line = serde_json::to_string(&json).expect("Failed to serialize");
        writeln!(file, "{}", line).expect("Failed to write");
        count += 1;
    }

    assert_eq!(count, 10);

    // Verify sorted order
    let content = fs::read_to_string(&export_path).expect("Failed to read");
    let indices: Vec<i64> = content
        .lines()
        .map(|line| {
            let doc: serde_json::Value = serde_json::from_str(line).expect("Invalid JSON");
            doc["index"].as_i64().unwrap()
        })
        .collect();

    assert_eq!(indices, vec![9, 8, 7, 6, 5, 4, 3, 2, 1, 0]);
}

/// Test export of empty collection.
#[tokio::test]
async fn test_export_empty_collection() {
    let mongo = MongoTestContainer::start().await;
    let db = mongo.database("test_db");
    db.create_collection("empty_collection").await.expect("Failed to create collection");

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let export_path = temp_dir.path().join("export.jsonl");

    // Export empty collection
    let collection = mongo.collection::<Document>("test_db", "empty_collection");
    let mut cursor = collection.find(doc! {}).await.expect("Failed to find");
    let mut file = File::create(&export_path).expect("Failed to create file");
    let mut count = 0u64;

    while let Some(doc) = cursor.try_next().await.expect("Cursor error") {
        let json = Bson::Document(doc).into_relaxed_extjson();
        let line = serde_json::to_string(&json).expect("Failed to serialize");
        writeln!(file, "{}", line).expect("Failed to write");
        count += 1;
    }

    assert_eq!(count, 0);

    // File should exist but be empty
    let content = fs::read_to_string(&export_path).expect("Failed to read");
    assert!(content.is_empty());
}

/// Test export of large collection (streaming/batching).
#[tokio::test]
async fn test_export_large_collection() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "large_collection");

    // Insert many documents
    let docs = fixtures::generate_test_documents(2500);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let export_path = temp_dir.path().join("export.jsonl");

    // Export using streaming
    let mut cursor = collection.find(doc! {}).await.expect("Failed to find");
    let mut file = File::create(&export_path).expect("Failed to create file");
    let mut count = 0u64;

    while let Some(doc) = cursor.try_next().await.expect("Cursor error") {
        let json = Bson::Document(doc).into_relaxed_extjson();
        let line = serde_json::to_string(&json).expect("Failed to serialize");
        writeln!(file, "{}", line).expect("Failed to write");
        count += 1;
    }

    assert_eq!(count, 2500);

    // Verify line count
    let file = fs::File::open(&export_path).expect("Failed to open");
    let line_count = BufReader::new(file).lines().count();
    assert_eq!(line_count, 2500);
}

// =============================================================================
// JSON Import Tests
// =============================================================================

/// Parse relaxed JSON to BSON Document.
fn parse_json_to_document(json: &str) -> Result<Document, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    mongodb::bson::Bson::try_from(value).map_err(|e| e.to_string()).and_then(|b| match b {
        Bson::Document(doc) => Ok(doc),
        _ => Err("Expected document".to_string()),
    })
}

/// Test importing from JSON Lines format.
#[tokio::test]
async fn test_import_jsonl() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "import_jsonl");

    // Create JSONL file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let import_path = temp_dir.path().join("import.jsonl");
    let jsonl_content = r#"{"name": "doc1", "value": 100}
{"name": "doc2", "value": 200}
{"name": "doc3", "value": 300}"#;
    fs::write(&import_path, jsonl_content).expect("Failed to write file");

    // Import line by line
    let file = File::open(&import_path).expect("Failed to open file");
    let reader = BufReader::new(file);
    let mut docs: Vec<Document> = Vec::new();

    for line in reader.lines() {
        let line = line.expect("Failed to read line");
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let doc = parse_json_to_document(trimmed).expect("Failed to parse JSON");
        docs.push(doc);
    }

    collection.insert_many(docs).await.expect("Failed to insert");

    // Verify documents in collection
    let doc_count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(doc_count, 3);
}

/// Test importing from JSON array format.
#[tokio::test]
async fn test_import_json_array() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "import_json_array");

    // Create JSON array file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let import_path = temp_dir.path().join("import.json");
    let json_content = r#"[
        {"name": "doc1", "value": 100},
        {"name": "doc2", "value": 200},
        {"name": "doc3", "value": 300}
    ]"#;
    fs::write(&import_path, json_content).expect("Failed to write file");

    // Import JSON array
    let content = fs::read_to_string(&import_path).expect("Failed to read file");
    let value: serde_json::Value = serde_json::from_str(&content).expect("Failed to parse JSON");
    let array = value.as_array().expect("Expected array");

    let docs: Vec<Document> = array
        .iter()
        .map(|v| {
            let bson = Bson::try_from(v.clone()).expect("Failed to convert to BSON");
            match bson {
                Bson::Document(doc) => doc,
                _ => panic!("Expected document"),
            }
        })
        .collect();

    collection.insert_many(docs).await.expect("Failed to insert");

    // Verify documents in collection
    let doc_count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(doc_count, 3);
}

/// Test import in Insert mode (fail on duplicates).
#[tokio::test]
async fn test_import_insert_mode() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "import_insert_mode");

    // Insert a document with known _id
    collection
        .insert_one(doc! { "_id": "existing", "name": "original" })
        .await
        .expect("Failed to insert");

    // Try to insert document with same _id
    let duplicate_doc = doc! { "_id": "existing", "name": "new" };

    // Should fail due to duplicate key
    let result = collection.insert_one(duplicate_doc).await;
    assert!(result.is_err());

    // Original document should be unchanged
    let found = collection.find_one(doc! { "_id": "existing" }).await.expect("Failed to find");
    assert_eq!(found.unwrap().get_str("name").unwrap(), "original");
}

/// Test import in Upsert mode (update or insert).
#[tokio::test]
async fn test_import_upsert_mode() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "import_upsert_mode");

    // Insert a document with known _id
    collection
        .insert_one(doc! { "_id": "existing", "name": "original", "extra": "keep" })
        .await
        .expect("Failed to insert");

    // Upsert documents
    let docs =
        vec![doc! { "_id": "existing", "name": "updated" }, doc! { "_id": "new1", "name": "doc1" }];

    let options = mongodb::options::UpdateOptions::builder().upsert(true).build();

    for doc in docs {
        let id = doc.get("_id").unwrap().clone();
        let filter = doc! { "_id": id };
        let mut update_doc = doc.clone();
        update_doc.remove("_id");
        collection
            .update_one(filter, doc! { "$set": update_doc })
            .with_options(options.clone())
            .await
            .expect("Failed to upsert");
    }

    // Check that existing document was updated (using $set, so "extra" should still exist)
    let existing =
        collection.find_one(doc! { "_id": "existing" }).await.expect("Failed to find").unwrap();
    assert_eq!(existing.get_str("name").unwrap(), "updated");
    assert_eq!(existing.get_str("extra").unwrap(), "keep");

    // Check that new document was inserted
    let new_doc = collection.find_one(doc! { "_id": "new1" }).await.expect("Failed to find");
    assert!(new_doc.is_some());
}

/// Test import in Replace mode (full replacement).
#[tokio::test]
async fn test_import_replace_mode() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "import_replace_mode");

    // Insert a document with known _id
    collection
        .insert_one(doc! { "_id": "existing", "name": "original", "extra": "remove_me" })
        .await
        .expect("Failed to insert");

    // Replace document
    let new_doc = doc! { "_id": "existing", "name": "replaced" };
    let options = mongodb::options::ReplaceOptions::builder().upsert(true).build();

    collection
        .replace_one(doc! { "_id": "existing" }, new_doc)
        .with_options(options)
        .await
        .expect("Failed to replace");

    // Check that document was fully replaced (extra field should be gone)
    let existing =
        collection.find_one(doc! { "_id": "existing" }).await.expect("Failed to find").unwrap();
    assert_eq!(existing.get_str("name").unwrap(), "replaced");
    assert!(existing.get("extra").is_none());
}

/// Test import with existing _ids (preserving _id).
#[tokio::test]
async fn test_import_with_existing_ids() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "import_with_ids");

    // Import documents with custom _ids
    let docs = vec![
        doc! { "_id": "custom_id_1", "name": "doc1" },
        doc! { "_id": "custom_id_2", "name": "doc2" },
    ];

    collection.insert_many(docs).await.expect("Failed to insert");

    // Verify _ids are preserved
    let doc1 = collection.find_one(doc! { "_id": "custom_id_1" }).await.expect("Failed to find");
    assert!(doc1.is_some());
    let doc2 = collection.find_one(doc! { "_id": "custom_id_2" }).await.expect("Failed to find");
    assert!(doc2.is_some());
}

/// Test import batch processing.
#[tokio::test]
async fn test_import_batch_processing() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "import_batch");

    // Create documents in batches
    let batch_size = 50;
    let total_docs = 250;

    for batch_start in (0..total_docs).step_by(batch_size) {
        let batch_end = (batch_start + batch_size).min(total_docs);
        let batch: Vec<Document> = (batch_start..batch_end)
            .map(|i| doc! { "index": i as i32, "name": format!("doc{}", i) })
            .collect();

        collection.insert_many(batch).await.expect("Failed to insert batch");
    }

    // Verify all documents imported
    let doc_count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(doc_count, 250);
}

/// Test import ordered vs unordered behavior.
#[tokio::test]
async fn test_import_stop_on_error() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "import_stop_error");

    // Create unique index
    collection
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "unique_field": 1 })
                .options(mongodb::options::IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .expect("Failed to create index");

    // Documents with duplicate unique values
    let docs = vec![
        doc! { "unique_field": "a", "name": "doc1" },
        doc! { "unique_field": "a", "name": "doc2" }, // Duplicate
        doc! { "unique_field": "b", "name": "doc3" },
    ];

    // Import with ordered=false (continue on error)
    let options = mongodb::options::InsertManyOptions::builder().ordered(false).build();
    let result = collection.insert_many(docs).with_options(options).await;

    // With ordered=false, it continues after errors
    // The result may be an error but some docs are inserted
    // Either it succeeds with 2 inserted, or fails with an error but still inserts 2
    match result {
        Ok(r) => {
            // All non-duplicate documents were inserted
            assert_eq!(r.inserted_ids.len(), 2);
        }
        Err(_) => {
            // Expected - duplicate key error, but unordered insert continues
        }
    }

    // Verify 2 documents were inserted (first and third)
    let doc_count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(doc_count, 2);
}

// =============================================================================
// Recoverable Import/Copy Tests
// =============================================================================

#[tokio::test]
async fn recoverable_import_invalid_jsonl_preserves_clear_target() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "recoverable_import_clear");
    collection
        .insert_one(doc! { "_id": "original", "name": "keep" })
        .await
        .expect("Failed to seed target");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let import_path = temp_dir.path().join("malformed.jsonl");
    fs::write(&import_path, "{\"_id\":\"new\"}\n{not valid json}\n")
        .expect("Failed to write fixture");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let result = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().import_collection_json_with_options(
            &client,
            &database,
            "recoverable_import_clear",
            &import_path,
            JsonImportOptions {
                format: JsonTransferFormat::JsonLines,
                batch_size: 1,
                target_write_mode: TargetWriteMode::Clear,
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked");

    assert!(result.is_err(), "malformed input must fail");
    let original = collection
        .find_one(doc! { "_id": "original" })
        .await
        .expect("Failed to read target")
        .expect("Original target document was removed");
    assert_eq!(original.get_str("name").unwrap(), "keep");
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 1);
    let collection_names = mongo.database("test_db").list_collection_names().await.unwrap();
    assert!(
        collection_names.iter().all(|name| !name.starts_with("__openmango_stage_")),
        "failed imports must remove staging collections"
    );
}

#[tokio::test]
async fn recoverable_import_clear_replaces_target_and_preserves_indexes() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "recoverable_import_clear_success");
    collection
        .insert_one(doc! { "_id": "old", "email": "old@example.com" })
        .await
        .expect("Failed to seed target");
    collection
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "email": 1 })
                .options(mongodb::options::IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .expect("Failed to create target index");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let import_path = temp_dir.path().join("valid.jsonl");
    fs::write(&import_path, "{\"_id\":\"new\",\"email\":\"new@example.com\"}\n")
        .expect("Failed to write fixture");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let count = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().import_collection_json_with_options(
            &client,
            &database,
            "recoverable_import_clear_success",
            &import_path,
            JsonImportOptions {
                format: JsonTransferFormat::JsonLines,
                batch_size: 10,
                target_write_mode: TargetWriteMode::Clear,
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked")
    .expect("Import failed");

    assert_eq!(count, 1);
    assert!(collection.find_one(doc! { "_id": "old" }).await.unwrap().is_none());
    assert!(collection.find_one(doc! { "_id": "new" }).await.unwrap().is_some());
    assert!(
        collection
            .insert_one(doc! { "_id": "duplicate", "email": "new@example.com" })
            .await
            .is_err(),
        "Clear replacement must preserve target indexes"
    );
    let names = mongo.database("test_db").list_collection_names().await.unwrap();
    assert!(names.iter().all(|name| !name.starts_with("__openmango_stage_")));
}

#[tokio::test]
async fn recoverable_import_drop_replaces_target_collection() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "recoverable_import_drop");
    collection
        .insert_one(doc! { "_id": "old", "name": "old" })
        .await
        .expect("Failed to seed target");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let import_path = temp_dir.path().join("valid.jsonl");
    fs::write(&import_path, "{\"_id\":\"new\",\"name\":\"new\"}\n")
        .expect("Failed to write fixture");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    tokio::task::spawn_blocking(move || {
        ConnectionManager::new().import_collection_json_with_options(
            &client,
            &database,
            "recoverable_import_drop",
            &import_path,
            JsonImportOptions {
                format: JsonTransferFormat::JsonLines,
                batch_size: 10,
                target_write_mode: TargetWriteMode::Drop,
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked")
    .expect("Import failed");

    assert!(collection.find_one(doc! { "_id": "old" }).await.unwrap().is_none());
    assert!(collection.find_one(doc! { "_id": "new" }).await.unwrap().is_some());
    let names = mongo.database("test_db").list_collection_names().await.unwrap();
    assert!(names.iter().all(|name| !name.starts_with("__openmango_stage_")));
}

#[tokio::test]
async fn recoverable_import_empty_drop_replaces_target_with_empty_collection() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "recoverable_import_empty_drop");
    collection
        .insert_one(doc! { "_id": "old", "name": "old" })
        .await
        .expect("Failed to seed target");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let import_path = temp_dir.path().join("empty.jsonl");
    fs::write(&import_path, "").expect("Failed to write fixture");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let count = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().import_collection_json_with_options(
            &client,
            &database,
            "recoverable_import_empty_drop",
            &import_path,
            JsonImportOptions {
                format: JsonTransferFormat::JsonLines,
                batch_size: 10,
                target_write_mode: TargetWriteMode::Drop,
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked")
    .expect("Empty import failed");

    assert_eq!(count, 0);
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 0);
}

#[tokio::test]
async fn recoverable_import_cancelled_after_staging_preserves_target() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "recoverable_import_cancelled");
    collection
        .insert_one(doc! { "_id": "old", "name": "keep" })
        .await
        .expect("Failed to seed target");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let import_path = temp_dir.path().join("valid.jsonl");
    fs::write(&import_path, "{\"_id\":\"new\",\"name\":\"new\"}\n")
        .expect("Failed to write fixture");

    let token = CancellationToken::new();
    let callback_token = token.clone();
    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let error = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().import_collection_json_with_options(
            &client,
            &database,
            "recoverable_import_cancelled",
            &import_path,
            JsonImportOptions {
                format: JsonTransferFormat::JsonLines,
                batch_size: 1,
                target_write_mode: TargetWriteMode::Clear,
                progress: Some(std::sync::Arc::new(move |_| callback_token.cancel())),
                cancellation: Some(token),
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked")
    .expect_err("Cancelled staged import must fail");

    assert!(error.to_string().contains("cancelled"));
    let original = collection
        .find_one(doc! { "_id": "old" })
        .await
        .unwrap()
        .expect("Original target was replaced after cancellation");
    assert_eq!(original.get_str("name").unwrap(), "keep");
    let names = mongo.database("test_db").list_collection_names().await.unwrap();
    assert!(names.iter().all(|name| !name.starts_with("__openmango_stage_")));
}

#[tokio::test]
async fn recoverable_import_unordered_insert_reports_partial_success() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "recoverable_import_insert_partial");
    collection
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "email": 1 })
                .options(mongodb::options::IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .expect("Failed to create target index");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let import_path = temp_dir.path().join("partial.jsonl");
    fs::write(
        &import_path,
        concat!(
            "{\"_id\":\"one\",\"email\":\"duplicate@example.com\"}\n",
            "{\"_id\":\"two\",\"email\":\"duplicate@example.com\"}\n",
            "{\"_id\":\"three\",\"email\":\"other@example.com\"}\n"
        ),
    )
    .expect("Failed to write fixture");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let error = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().import_collection_json_with_options(
            &client,
            &database,
            "recoverable_import_insert_partial",
            &import_path,
            JsonImportOptions {
                format: JsonTransferFormat::JsonLines,
                insert_mode: InsertMode::Insert,
                stop_on_error: false,
                batch_size: 10,
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked")
    .expect_err("Unordered duplicate insert must report partial failure");

    assert_eq!(error.processed_count(), 2);
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 2);
}

#[tokio::test]
async fn recoverable_import_replace_failure_preserves_failed_original_and_reports_partial() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "recoverable_import_replace");
    collection
        .insert_many(vec![
            doc! { "_id": "one", "email": "original@example.com", "name": "original" },
            doc! { "_id": "two", "email": "occupied@example.com" },
        ])
        .await
        .expect("Failed to seed target");
    collection
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "email": 1 })
                .options(mongodb::options::IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .expect("Failed to create unique index");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let import_path = temp_dir.path().join("replacement.jsonl");
    fs::write(
        &import_path,
        concat!(
            "{\"_id\":\"one\",\"email\":\"updated@example.com\",\"name\":\"replacement\"}\n",
            "{\"_id\":\"two\",\"email\":\"updated@example.com\",\"name\":\"must-fail\"}\n"
        ),
    )
    .expect("Failed to write fixture");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let result = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().import_collection_json_with_options(
            &client,
            &database,
            "recoverable_import_replace",
            &import_path,
            JsonImportOptions {
                format: JsonTransferFormat::JsonLines,
                insert_mode: InsertMode::Replace,
                stop_on_error: true,
                batch_size: 10,
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked");

    let error = result.expect_err("conflicting replacement must fail");
    assert_eq!(error.processed_count(), 1);

    let replaced = collection
        .find_one(doc! { "_id": "one" })
        .await
        .expect("Failed to read first target")
        .expect("Successful replacement is missing");
    assert_eq!(replaced.get_str("name").unwrap(), "replacement");
    assert_eq!(replaced.get_str("email").unwrap(), "updated@example.com");

    let preserved = collection
        .find_one(doc! { "_id": "two" })
        .await
        .expect("Failed to read conflicting target")
        .expect("Failed replacement deleted the original document");
    assert_eq!(preserved.get_str("email").unwrap(), "occupied@example.com");
}

#[tokio::test]
async fn recoverable_copy_clear_commit_failure_preserves_target() {
    let mongo = MongoTestContainer::start().await;
    let source = mongo.collection::<Document>("test_db", "recoverable_copy_source");
    source
        .insert_many(vec![
            doc! { "_id": "source-one", "email": "duplicate@example.com" },
            doc! { "_id": "source-two", "email": "duplicate@example.com" },
        ])
        .await
        .expect("Failed to seed source");

    let target = mongo.collection::<Document>("test_db", "recoverable_copy_target");
    target
        .insert_one(doc! { "_id": "target", "email": "keep@example.com" })
        .await
        .expect("Failed to seed target");
    target
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "email": 1 })
                .options(mongodb::options::IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .expect("Failed to create target index");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let result = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().copy_collection_with_options(
            &client,
            &database,
            "recoverable_copy_source",
            &client,
            &database,
            "recoverable_copy_target",
            CopyOptions {
                batch_size: 10,
                target_write_mode: TargetWriteMode::Clear,
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked");

    assert!(result.is_err(), "commit must fail on the target unique index");
    let original = target
        .find_one(doc! { "_id": "target" })
        .await
        .expect("Failed to read target")
        .expect("Original target was replaced");
    assert_eq!(original.get_str("email").unwrap(), "keep@example.com");
    assert_eq!(target.count_documents(doc! {}).await.unwrap(), 1);
    let names = mongo.database("test_db").list_collection_names().await.unwrap();
    assert!(names.iter().all(|name| !name.starts_with("__openmango_stage_")));
}

#[tokio::test]
async fn recoverable_copy_replace_failure_reports_partial_and_preserves_failed_document() {
    let mongo = MongoTestContainer::start().await;
    let source = mongo.collection::<Document>("test_db", "recoverable_copy_replace_source");
    source
        .insert_many(vec![
            doc! { "_id": "one", "email": "updated@example.com" },
            doc! { "_id": "two", "email": "updated@example.com" },
        ])
        .await
        .expect("Failed to seed source");

    let target = mongo.collection::<Document>("test_db", "recoverable_copy_replace_target");
    target
        .insert_many(vec![
            doc! { "_id": "one", "email": "one@example.com" },
            doc! { "_id": "two", "email": "two@example.com" },
        ])
        .await
        .expect("Failed to seed target");
    target
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "email": 1 })
                .options(mongodb::options::IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .expect("Failed to create target index");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let error = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().copy_collection_with_options(
            &client,
            &database,
            "recoverable_copy_replace_source",
            &client,
            &database,
            "recoverable_copy_replace_target",
            CopyOptions {
                batch_size: 1,
                insert_mode: InsertMode::Replace,
                ordered: true,
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked")
    .expect_err("Conflicting replacements must fail");

    assert_eq!(error.processed_count(), 1);
    assert_eq!(target.count_documents(doc! {}).await.unwrap(), 2);
    let documents: Vec<Document> = target.find(doc! {}).await.unwrap().try_collect().await.unwrap();
    assert_eq!(
        documents
            .iter()
            .filter(|document| document.get_str("email").unwrap() == "updated@example.com")
            .count(),
        1
    );
}

#[tokio::test]
async fn recoverable_copy_unordered_upsert_reports_partial_success() {
    let mongo = MongoTestContainer::start().await;
    let source = mongo.collection::<Document>("test_db", "recoverable_copy_upsert_source");
    source
        .insert_many(vec![
            doc! { "_id": "one", "email": "new@example.com" },
            doc! { "_id": "two", "email": "occupied@example.com" },
        ])
        .await
        .expect("Failed to seed source");

    let target = mongo.collection::<Document>("test_db", "recoverable_copy_upsert_target");
    target
        .insert_one(doc! { "_id": "occupied", "email": "occupied@example.com" })
        .await
        .expect("Failed to seed target");
    target
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "email": 1 })
                .options(mongodb::options::IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .expect("Failed to create target index");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let error = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().copy_collection_with_options(
            &client,
            &database,
            "recoverable_copy_upsert_source",
            &client,
            &database,
            "recoverable_copy_upsert_target",
            CopyOptions {
                batch_size: 10,
                insert_mode: InsertMode::Upsert,
                ordered: false,
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked")
    .expect_err("Conflicting unordered upsert must report partial failure");

    assert_eq!(error.processed_count(), 1);
    assert_eq!(target.count_documents(doc! {}).await.unwrap(), 2);
    assert!(target.find_one(doc! { "_id": "one" }).await.unwrap().is_some());
    assert!(target.find_one(doc! { "_id": "two" }).await.unwrap().is_none());
}

#[tokio::test]
async fn recoverable_copy_empty_drop_replaces_target_with_empty_collection() {
    let mongo = MongoTestContainer::start().await;
    mongo
        .database("test_db")
        .create_collection("recoverable_copy_empty_source")
        .await
        .expect("Failed to create empty source");
    let target = mongo.collection::<Document>("test_db", "recoverable_copy_empty_target");
    target.insert_one(doc! { "_id": "old", "name": "old" }).await.expect("Failed to seed target");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let count = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().copy_collection_with_options(
            &client,
            &database,
            "recoverable_copy_empty_source",
            &client,
            &database,
            "recoverable_copy_empty_target",
            CopyOptions {
                batch_size: 10,
                target_write_mode: TargetWriteMode::Drop,
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked")
    .expect("Empty copy failed");

    assert_eq!(count, 0);
    assert_eq!(target.count_documents(doc! {}).await.unwrap(), 0);
    let names = mongo.database("test_db").list_collection_names().await.unwrap();
    assert!(names.iter().all(|name| !name.starts_with("__openmango_stage_")));
}

#[tokio::test]
async fn recoverable_copy_missing_source_preserves_drop_target() {
    let mongo = MongoTestContainer::start().await;
    let target = mongo.collection::<Document>("test_db", "recoverable_copy_missing_target");
    target
        .insert_one(doc! { "_id": "target", "name": "keep" })
        .await
        .expect("Failed to seed target");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let result = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().copy_collection_with_options(
            &client,
            &database,
            "recoverable_copy_missing_source",
            &client,
            &database,
            "recoverable_copy_missing_target",
            CopyOptions {
                batch_size: 10,
                target_write_mode: TargetWriteMode::Drop,
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked");

    assert!(result.is_err(), "missing source collection must fail");
    let original = target
        .find_one(doc! { "_id": "target" })
        .await
        .expect("Failed to read target")
        .expect("Original target was dropped");
    assert_eq!(original.get_str("name").unwrap(), "keep");
}

// =============================================================================
// CSV Export/Import Tests
// =============================================================================

/// Flatten a document for CSV export.
fn flatten_document(doc: &Document) -> HashMap<String, String> {
    let mut flat = HashMap::new();
    flatten_helper(doc, "", &mut flat);
    flat
}

fn flatten_helper(doc: &Document, prefix: &str, flat: &mut HashMap<String, String>) {
    for (key, value) in doc {
        let full_key = if prefix.is_empty() { key.clone() } else { format!("{}.{}", prefix, key) };

        match value {
            Bson::Document(nested) => flatten_helper(nested, &full_key, flat),
            Bson::Array(arr) => {
                flat.insert(full_key, format!("{:?}", arr));
            }
            _ => {
                let str_val = match value {
                    Bson::String(s) => s.clone(),
                    Bson::Int32(i) => i.to_string(),
                    Bson::Int64(i) => i.to_string(),
                    Bson::Double(d) => d.to_string(),
                    Bson::Boolean(b) => b.to_string(),
                    Bson::ObjectId(oid) => oid.to_hex(),
                    Bson::Null => "".to_string(),
                    _ => format!("{:?}", value),
                };
                flat.insert(full_key, str_val);
            }
        }
    }
}

/// Unflatten a row (dot notation keys) into nested documents.
fn unflatten_row(row: &HashMap<String, String>) -> Document {
    let mut doc = Document::new();
    for (key, value) in row {
        insert_nested(&mut doc, key, value);
    }
    doc
}

fn insert_nested(doc: &mut Document, key: &str, value: &str) {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.len() == 1 {
        doc.insert(key.to_string(), Bson::String(value.to_string()));
    } else {
        let first = parts[0];
        let rest = parts[1..].join(".");
        let nested =
            doc.entry(first.to_string()).or_insert_with(|| Bson::Document(Document::new()));
        if let Bson::Document(nested_doc) = nested {
            insert_nested(nested_doc, &rest, value);
        }
    }
}

/// Test exporting flat documents to CSV.
#[tokio::test]
async fn test_export_csv_simple() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "export_csv_simple");

    // Insert flat documents
    let docs = vec![
        doc! { "name": "Alice", "age": 30, "city": "NYC" },
        doc! { "name": "Bob", "age": 25, "city": "LA" },
        doc! { "name": "Charlie", "age": 35, "city": "Chicago" },
    ];
    collection.insert_many(docs).await.expect("Failed to insert");

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let export_path = temp_dir.path().join("export.csv");

    // Export to CSV
    let cursor = collection.find(doc! {}).await.expect("Failed to find");
    let documents: Vec<Document> = cursor.try_collect().await.expect("Failed to collect");

    // Collect all columns
    let mut columns: Vec<String> = Vec::new();
    for doc in &documents {
        let flat = flatten_document(doc);
        for key in flat.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }
    columns.sort();

    // Write CSV
    let mut wtr = csv::Writer::from_path(&export_path).expect("Failed to create writer");
    wtr.write_record(&columns).expect("Failed to write header");

    for doc in &documents {
        let flat = flatten_document(doc);
        let row: Vec<String> =
            columns.iter().map(|c| flat.get(c).cloned().unwrap_or_default()).collect();
        wtr.write_record(&row).expect("Failed to write row");
    }
    wtr.flush().expect("Failed to flush");

    // Verify CSV content
    let content = fs::read_to_string(&export_path).expect("Failed to read");
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 4); // Header + 3 data rows

    // Header should contain field names
    let header = lines[0];
    assert!(header.contains("name"));
    assert!(header.contains("age"));
    assert!(header.contains("city"));
}

/// Test exporting nested documents to CSV (flattening).
#[tokio::test]
async fn test_export_csv_nested() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "export_csv_nested");

    // Insert nested documents
    let docs = vec![
        doc! { "name": "Alice", "address": { "city": "NYC", "zip": "10001" } },
        doc! { "name": "Bob", "address": { "city": "LA", "zip": "90001" } },
    ];
    collection.insert_many(docs).await.expect("Failed to insert");

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let export_path = temp_dir.path().join("export.csv");

    // Export to CSV with flattening
    let cursor = collection.find(doc! {}).await.expect("Failed to find");
    let documents: Vec<Document> = cursor.try_collect().await.expect("Failed to collect");

    let mut columns: Vec<String> = Vec::new();
    for doc in &documents {
        let flat = flatten_document(doc);
        for key in flat.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }
    columns.sort();

    let mut wtr = csv::Writer::from_path(&export_path).expect("Failed to create writer");
    wtr.write_record(&columns).expect("Failed to write header");

    for doc in &documents {
        let flat = flatten_document(doc);
        let row: Vec<String> =
            columns.iter().map(|c| flat.get(c).cloned().unwrap_or_default()).collect();
        wtr.write_record(&row).expect("Failed to write row");
    }
    wtr.flush().expect("Failed to flush");

    // Verify CSV content has flattened field names
    let content = fs::read_to_string(&export_path).expect("Failed to read");
    let header = content.lines().next().expect("No header");
    assert!(header.contains("address.city"));
    assert!(header.contains("address.zip"));
}

/// Test importing from simple CSV.
#[tokio::test]
async fn test_import_csv_simple() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "import_csv_simple");

    // Create CSV file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let import_path = temp_dir.path().join("import.csv");
    let csv_content = "name,age,city\nAlice,30,NYC\nBob,25,LA";
    fs::write(&import_path, csv_content).expect("Failed to write file");

    // Import CSV
    let mut rdr = csv::Reader::from_path(&import_path).expect("Failed to open CSV");
    let headers: Vec<String> =
        rdr.headers().expect("No headers").iter().map(|s| s.to_string()).collect();

    let mut docs: Vec<Document> = Vec::new();
    for result in rdr.records() {
        let record = result.expect("Failed to read record");
        let mut row: HashMap<String, String> = HashMap::new();
        for (i, value) in record.iter().enumerate() {
            if let Some(header) = headers.get(i) {
                row.insert(header.clone(), value.to_string());
            }
        }
        docs.push(unflatten_row(&row));
    }

    collection.insert_many(docs).await.expect("Failed to insert");

    // Verify documents
    let alice =
        collection.find_one(doc! { "name": "Alice" }).await.expect("Failed to find").unwrap();
    assert_eq!(alice.get_str("city").unwrap(), "NYC");
}

/// Test importing from CSV with dot notation columns (nested documents).
#[tokio::test]
async fn test_import_csv_nested() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "import_csv_nested");

    // Create CSV file with dot notation for nested fields
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let import_path = temp_dir.path().join("import.csv");
    let csv_content = "name,address.city,address.zip\nAlice,NYC,10001\nBob,LA,90001";
    fs::write(&import_path, csv_content).expect("Failed to write file");

    // Import CSV with nested structure
    let mut rdr = csv::Reader::from_path(&import_path).expect("Failed to open CSV");
    let headers: Vec<String> =
        rdr.headers().expect("No headers").iter().map(|s| s.to_string()).collect();

    let mut docs: Vec<Document> = Vec::new();
    for result in rdr.records() {
        let record = result.expect("Failed to read record");
        let mut row: HashMap<String, String> = HashMap::new();
        for (i, value) in record.iter().enumerate() {
            if let Some(header) = headers.get(i) {
                row.insert(header.clone(), value.to_string());
            }
        }
        docs.push(unflatten_row(&row));
    }

    collection.insert_many(docs).await.expect("Failed to insert");

    // Verify nested structure
    let alice =
        collection.find_one(doc! { "name": "Alice" }).await.expect("Failed to find").unwrap();
    let address = alice.get_document("address").expect("No address");
    assert_eq!(address.get_str("city").unwrap(), "NYC");
    assert_eq!(address.get_str("zip").unwrap(), "10001");
}

// =============================================================================
// Cancellation Tests
// =============================================================================

/// Test that JSON export respects a pre-cancelled token.
#[tokio::test]
async fn test_export_json_cancellation() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "cancel_export_json");

    let docs = fixtures::generate_test_documents(100);
    collection.insert_many(docs).await.expect("Failed to insert");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let export_path = temp_dir.path().join("cancelled.jsonl");

    let token = CancellationToken::new();
    token.cancel();

    let client = mongo.client.clone();
    let db = mongo.db_name("test_db");

    // ConnectionManager::block_on can't nest inside tokio, so run off the async thread
    let result = tokio::task::spawn_blocking(move || {
        let mgr = ConnectionManager::new();
        mgr.export_collection_json_with_options(
            &client,
            &db,
            "cancel_export_json",
            &export_path,
            JsonExportOptions { cancellation: Some(token), ..Default::default() },
        )
    })
    .await
    .expect("Task panicked");

    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("cancelled"), "Expected 'cancelled' in error: {err_msg}");
}

/// Test that CSV export respects a pre-cancelled token.
#[tokio::test]
async fn test_export_csv_cancellation() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "cancel_export_csv");

    // Insert >1000 docs so the cursor moves past the buffered sample phase
    let docs = fixtures::generate_test_documents(1500);
    collection.insert_many(docs).await.expect("Failed to insert");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let export_path = temp_dir.path().join("cancelled.csv");

    let token = CancellationToken::new();
    token.cancel();

    let client = mongo.client.clone();
    let db = mongo.db_name("test_db");

    let result = tokio::task::spawn_blocking(move || {
        let mgr = ConnectionManager::new();
        mgr.export_collection_csv_with_query(
            &client,
            &db,
            "cancel_export_csv",
            &export_path,
            false,
            Default::default(),
            Some(token),
        )
    })
    .await
    .expect("Task panicked");

    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("cancelled"), "Expected 'cancelled' in error: {err_msg}");
}

/// Test that collection copy respects a pre-cancelled token.
#[tokio::test]
async fn test_copy_collection_cancellation() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "cancel_copy_src");

    let docs = fixtures::generate_test_documents(100);
    collection.insert_many(docs).await.expect("Failed to insert");

    let token = CancellationToken::new();
    token.cancel();

    let client = mongo.client.clone();
    let db = mongo.db_name("test_db");

    let result = tokio::task::spawn_blocking(move || {
        let mgr = ConnectionManager::new();
        mgr.copy_collection_with_options(
            &client,
            &db,
            "cancel_copy_src",
            &client,
            &db,
            "cancel_copy_dest",
            CopyOptions { batch_size: 50, cancellation: Some(token), ..Default::default() },
        )
    })
    .await
    .expect("Task panicked");

    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("cancelled"), "Expected 'cancelled' in error: {err_msg}");

    // Destination should have fewer docs than source (or none at all)
    let dest = mongo.collection::<Document>("test_db", "cancel_copy_dest");
    let dest_count = dest.count_documents(doc! {}).await.unwrap_or(0);
    assert!(dest_count < 100, "Expected fewer than 100 docs in dest, got {dest_count}");
}

/// Test that JSON import respects a pre-cancelled token.
#[tokio::test]
async fn test_import_json_cancellation() {
    let mongo = MongoTestContainer::start().await;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let import_path = temp_dir.path().join("import.jsonl");

    // Write a JSONL file with several lines
    {
        let mut f = File::create(&import_path).expect("Failed to create file");
        for i in 0..50 {
            writeln!(f, r#"{{"index": {i}, "name": "doc{i}"}}"#).expect("Failed to write");
        }
    }

    let token = CancellationToken::new();
    token.cancel();

    let client = mongo.client.clone();

    let result = tokio::task::spawn_blocking(move || {
        let mgr = ConnectionManager::new();
        mgr.import_collection_json_with_options(
            &client,
            "test_db",
            "cancel_import_json",
            &import_path,
            JsonImportOptions {
                format: JsonTransferFormat::JsonLines,
                batch_size: 10,
                cancellation: Some(token),
                ..Default::default()
            },
        )
    })
    .await
    .expect("Task panicked");

    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("cancelled"), "Expected 'cancelled' in error: {err_msg}");
}

// =============================================================================
// Copy Tests
// =============================================================================

/// Test copying a collection within the same database.
#[tokio::test]
async fn test_copy_collection_same_db() {
    let mongo = MongoTestContainer::start().await;
    let source_collection = mongo.collection::<Document>("test_db", "copy_source");

    // Insert documents in source
    let docs = fixtures::generate_test_documents(20);
    source_collection.insert_many(docs).await.expect("Failed to insert");

    // Copy collection using aggregation $out
    let pipeline = vec![doc! { "$out": { "db": mongo.db_name("test_db"), "coll": "copy_dest" } }];
    let _: Vec<Document> = source_collection
        .aggregate(pipeline)
        .await
        .expect("Failed to aggregate")
        .try_collect()
        .await
        .expect("Failed to collect");

    // Verify destination collection
    let dest_collection = mongo.collection::<Document>("test_db", "copy_dest");
    let dest_count = dest_collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(dest_count, 20);
}

/// Test copying a collection between different databases.
#[tokio::test]
async fn test_copy_collection_different_db() {
    let mongo = MongoTestContainer::start().await;
    let source_collection = mongo.collection::<Document>("source_db", "copy_source");

    // Insert documents in source
    let docs = fixtures::generate_test_documents(15);
    source_collection.insert_many(docs).await.expect("Failed to insert");

    // Copy collection to different database using $out
    let pipeline = vec![doc! { "$out": { "db": mongo.db_name("dest_db"), "coll": "copy_dest" } }];
    let _: Vec<Document> = source_collection
        .aggregate(pipeline)
        .await
        .expect("Failed to aggregate")
        .try_collect()
        .await
        .expect("Failed to collect");

    // Verify destination collection
    let dest_collection = mongo.collection::<Document>("dest_db", "copy_dest");
    let dest_count = dest_collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(dest_count, 15);
}

#[tokio::test]
async fn test_copy_index_failure_reports_incomplete_transfer() {
    let mongo = MongoTestContainer::start().await;
    let source = mongo.collection::<Document>("test_db", "copy_index_failure_source");
    source.insert_many(vec![doc! { "source": 1 }, doc! { "source": 2 }]).await.unwrap();
    source
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "source": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("conflicting_name".to_string())
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let destination = mongo.collection::<Document>("test_db", "copy_index_failure_destination");
    destination
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "destination": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("conflicting_name".to_string())
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let result = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().copy_collection(
            &client,
            &database,
            "copy_index_failure_source",
            &client,
            &database,
            "copy_index_failure_destination",
            100,
            true,
        )
    })
    .await
    .unwrap();

    let error = result.expect_err("Index conflict must make the copy incomplete");
    assert_eq!(error.processed_count(), 2);
    assert!(error.to_string().contains("Transfer failed after 2 document(s)"));
}

#[tokio::test]
async fn test_copy_collection_preserves_index_metadata() {
    let mongo = MongoTestContainer::start().await;
    let source = mongo.collection::<Document>("test_db", "copy_index_metadata_source");
    source
        .insert_many(vec![
            doc! { "status": "active", "title": "One", "details": { "public": true } },
            doc! { "status": "inactive", "title": "Two", "details": { "public": false } },
        ])
        .await
        .unwrap();
    mongo
        .client
        .database(&mongo.db_name("test_db"))
        .run_command(doc! {
            "createIndexes": "copy_index_metadata_source",
            "indexes": [
                {
                    "key": { "status": 1 },
                    "name": "active_status",
                    "partialFilterExpression": { "status": "active" },
                    "collation": { "locale": "en", "strength": 2 },
                    "hidden": true,
                },
                {
                    "key": { "$**": 1 },
                    "name": "public_details",
                    "wildcardProjection": { "details.public": 1 },
                },
                {
                    "key": { "title": "text" },
                    "name": "title_text",
                    "weights": { "title": 5 },
                    "default_language": "english",
                    "language_override": "language",
                },
            ],
        })
        .await
        .unwrap();

    let client = mongo.client.clone();
    let source_database = mongo.db_name("test_db");
    let destination_database = source_database.clone();
    tokio::task::spawn_blocking(move || {
        ConnectionManager::new().copy_collection(
            &client,
            &source_database,
            "copy_index_metadata_source",
            &client,
            &destination_database,
            "copy_index_metadata_destination",
            100,
            true,
        )
    })
    .await
    .unwrap()
    .expect("Copy failed");

    let indexes: Vec<mongodb::IndexModel> = mongo
        .collection::<Document>("test_db", "copy_index_metadata_destination")
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let active = indexes
        .iter()
        .find(|index| {
            index.options.as_ref().and_then(|options| options.name.as_deref())
                == Some("active_status")
        })
        .unwrap()
        .options
        .as_ref()
        .unwrap();
    assert_eq!(active.partial_filter_expression, Some(doc! { "status": "active" }));
    assert_eq!(active.hidden, Some(true));
    assert_eq!(active.collation.as_ref().map(|collation| collation.locale.as_str()), Some("en"));

    let wildcard = indexes
        .iter()
        .find(|index| {
            index.options.as_ref().and_then(|options| options.name.as_deref())
                == Some("public_details")
        })
        .unwrap()
        .options
        .as_ref()
        .unwrap();
    assert_eq!(wildcard.wildcard_projection, Some(doc! { "details.public": 1 }));

    let text = indexes
        .iter()
        .find(|index| {
            index.options.as_ref().and_then(|options| options.name.as_deref()) == Some("title_text")
        })
        .unwrap()
        .options
        .as_ref()
        .unwrap();
    assert_eq!(text.weights, Some(doc! { "title": 5 }));
    assert_eq!(text.default_language.as_deref(), Some("english"));
    assert_eq!(text.language_override.as_deref(), Some("language"));
}

/// Test copying a collection with indexes.
#[tokio::test]
async fn test_copy_collection_with_indexes() {
    let mongo = MongoTestContainer::start().await;
    let source_collection = mongo.collection::<Document>("test_db", "copy_with_indexes");

    // Insert documents and create indexes
    let docs = fixtures::generate_test_documents(10);
    source_collection.insert_many(docs).await.expect("Failed to insert");

    // Create a custom index
    source_collection
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "name": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("name_index".to_string())
                        .build(),
                )
                .build(),
        )
        .await
        .expect("Failed to create index");

    // Copy collection data
    let pipeline =
        vec![doc! { "$out": { "db": mongo.db_name("test_db"), "coll": "copy_with_indexes_dest" } }];
    let _: Vec<Document> = source_collection
        .aggregate(pipeline)
        .await
        .expect("Failed to aggregate")
        .try_collect()
        .await
        .expect("Failed to collect");

    // Copy indexes manually
    let source_indexes: Vec<mongodb::IndexModel> = source_collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");
    let dest_collection = mongo.collection::<Document>("test_db", "copy_with_indexes_dest");

    for index in source_indexes {
        let name =
            index.options.as_ref().and_then(|o| o.name.as_ref()).map(|n| n.as_str()).unwrap_or("");
        if name == "_id_" {
            continue; // Skip _id index
        }
        dest_collection.create_index(index).await.expect("Failed to create index");
    }

    // Verify indexes were copied
    let dest_indexes: Vec<mongodb::IndexModel> = dest_collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");

    assert!(dest_indexes.len() >= 2);
    let has_name_index = dest_indexes.iter().any(|idx| {
        idx.options
            .as_ref()
            .and_then(|o| o.name.as_ref())
            .map(|n| n == "name_index")
            .unwrap_or(false)
    });
    assert!(has_name_index, "name_index should be copied");
}

/// Test copying an entire database.
#[tokio::test]
async fn test_copy_database() {
    let mongo = MongoTestContainer::start().await;

    // Create multiple collections in source database
    let coll_a = mongo.collection::<Document>("copy_db_source", "collection_a");
    let coll_b = mongo.collection::<Document>("copy_db_source", "collection_b");

    coll_a.insert_many(fixtures::generate_test_documents(5)).await.expect("Failed to insert");
    coll_b.insert_many(fixtures::generate_test_documents(8)).await.expect("Failed to insert");

    // Copy each collection
    let source_db = mongo.database("copy_db_source");
    let collections = source_db.list_collection_names().await.expect("Failed to list");

    for coll_name in collections {
        if coll_name.starts_with("system.") {
            continue;
        }
        let src_coll = mongo.collection::<Document>("copy_db_source", &coll_name);
        let pipeline =
            vec![doc! { "$out": { "db": mongo.db_name("copy_db_dest"), "coll": &coll_name } }];
        let _: Vec<Document> = src_coll
            .aggregate(pipeline)
            .await
            .expect("Failed to aggregate")
            .try_collect()
            .await
            .expect("Failed to collect");
    }

    // Verify destination database
    let dest_a = mongo.collection::<Document>("copy_db_dest", "collection_a");
    let dest_b = mongo.collection::<Document>("copy_db_dest", "collection_b");

    let count_a = dest_a.count_documents(doc! {}).await.expect("Failed to count");
    let count_b = dest_b.count_documents(doc! {}).await.expect("Failed to count");

    assert_eq!(count_a, 5);
    assert_eq!(count_b, 8);
}
