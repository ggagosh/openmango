//! Integration tests for Import/Export/Copy operations using Testcontainers.
//!
//! These tests exercise the export/import/copy patterns that would be used
//! by the application. Since the main crate doesn't expose a library, we test
//! the MongoDB operations directly using the same patterns as the application.

mod common;

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};

use common::{MongoTestContainer, fixtures};
use futures::TryStreamExt;
use mongodb::bson::{Bson, Document, doc};
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
    let pipeline = vec![doc! { "$out": { "db": "test_db", "coll": "copy_dest" } }];
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
    let pipeline = vec![doc! { "$out": { "db": "dest_db", "coll": "copy_dest" } }];
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
    let pipeline = vec![doc! { "$out": { "db": "test_db", "coll": "copy_with_indexes_dest" } }];
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
        let pipeline = vec![doc! { "$out": { "db": "copy_db_dest", "coll": &coll_name } }];
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
