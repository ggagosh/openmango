//! Integration tests for bulk document operations using Testcontainers.

mod common;

use common::{MongoTestContainer, fixtures};
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use openmango::connection::ConnectionManager;

// =============================================================================
// Bulk Insert Tests
// =============================================================================

/// Test bulk inserting many documents.
#[tokio::test]
async fn test_bulk_insert_many() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "bulk_insert_many");

    // Generate and insert many documents
    let docs = fixtures::generate_test_documents(100);
    let result = collection.insert_many(docs).await.expect("Failed to insert");

    assert_eq!(result.inserted_ids.len(), 100);

    // Verify count
    let count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(count, 100);
}

/// Test bulk insert with custom _ids.
#[tokio::test]
async fn test_bulk_insert_with_ids() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "bulk_insert_ids");

    // Create documents with custom _ids
    let docs: Vec<Document> = (0..20)
        .map(|i| doc! { "_id": format!("custom_id_{}", i), "name": format!("doc_{}", i) })
        .collect();

    collection.insert_many(docs).await.expect("Failed to insert");

    // Verify all documents with custom _ids exist
    for i in 0..20 {
        let filter = doc! { "_id": format!("custom_id_{}", i) };
        let found = collection.find_one(filter).await.expect("Failed to find");
        assert!(found.is_some());
    }
}

/// Test ordered bulk insert (stop on error).
#[tokio::test]
async fn test_bulk_insert_ordered() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "bulk_insert_ordered");

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

    // Documents with duplicate unique values at position 1
    let docs = vec![
        doc! { "unique_field": "a", "order": 0 },
        doc! { "unique_field": "a", "order": 1 }, // Duplicate - will fail
        doc! { "unique_field": "b", "order": 2 }, // Won't be inserted due to ordered=true
    ];

    // Ordered insert (default) - stops at first error
    let options = mongodb::options::InsertManyOptions::builder().ordered(true).build();
    let result = collection.insert_many(docs).with_options(options).await;

    // Should fail
    assert!(result.is_err());

    // Only the first document should be inserted (before the error)
    let count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(count, 1);

    // Verify only doc with order=0 exists
    let found = collection.find_one(doc! { "order": 0 }).await.expect("Find failed");
    assert!(found.is_some());
    let not_found = collection.find_one(doc! { "order": 2 }).await.expect("Find failed");
    assert!(not_found.is_none());
}

/// Test unordered bulk insert (continue on error).
#[tokio::test]
async fn test_bulk_insert_unordered() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "bulk_insert_unordered");

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
        doc! { "unique_field": "a", "order": 0 },
        doc! { "unique_field": "a", "order": 1 }, // Duplicate
        doc! { "unique_field": "b", "order": 2 }, // Should be inserted
    ];

    // Unordered insert - continues despite errors
    let options = mongodb::options::InsertManyOptions::builder().ordered(false).build();
    let result = collection.insert_many(docs).with_options(options).await;

    // May return error but still inserts valid documents
    if result.is_err() {
        // Expected for duplicate key
    }

    // Both non-duplicate documents should be inserted
    let count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(count, 2);

    // Verify both valid docs exist
    let found_0 = collection.find_one(doc! { "order": 0 }).await.expect("Find failed");
    let found_2 = collection.find_one(doc! { "order": 2 }).await.expect("Find failed");
    assert!(found_0.is_some());
    assert!(found_2.is_some());
}

// =============================================================================
// Bulk Update Tests
// =============================================================================

/// Test updating many documents by filter.
#[tokio::test]
async fn test_update_many_by_filter() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "update_many_filter");

    // Insert test documents
    let docs = fixtures::generate_test_documents(20);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Update all "even" category documents
    let filter = doc! { "category": "even" };
    let update = doc! { "$set": { "updated": true } };
    let result = collection.update_many(filter, update).await.expect("Failed to update");

    // Should update 10 documents (indices 0, 2, 4, 6, 8, 10, 12, 14, 16, 18)
    assert_eq!(result.modified_count, 10);

    // Verify updates
    let updated_count =
        collection.count_documents(doc! { "updated": true }).await.expect("Failed to count");
    assert_eq!(updated_count, 10);
}

/// Count the exact frozen filter through OpenMango's public operation seam.
#[tokio::test]
async fn test_count_documents_for_bulk_confirmation() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "bulk_confirmation_count");
    collection
        .insert_many(fixtures::generate_test_documents(20))
        .await
        .expect("Failed to seed collection");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let count = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().count_documents(
            &client,
            &database,
            "bulk_confirmation_count",
            doc! { "category": "even" },
        )
    })
    .await
    .expect("Count task panicked")
    .expect("Count failed");

    assert_eq!(count, 10);
}

/// Conditional saves must reject a stale server baseline instead of overwriting it.
#[tokio::test]
async fn test_replace_document_if_current_rejects_conflicts() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "conditional_replace");
    let baseline = doc! { "_id": 1, "value": "baseline", "nested": { "ok": true } };
    collection.insert_one(baseline.clone()).await.expect("Failed to seed collection");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let expected = baseline.clone();
    tokio::task::spawn_blocking(move || {
        ConnectionManager::new().replace_document_if_current(
            &client,
            &database,
            "conditional_replace",
            &mongodb::bson::Bson::Int32(1),
            &expected,
            doc! { "_id": 1, "value": "first save", "nested": { "ok": true } },
        )
    })
    .await
    .expect("Replace task panicked")
    .expect("Matching baseline should save");

    collection
        .update_one(doc! { "_id": 1 }, doc! { "$set": { "value": "server edit" } })
        .await
        .expect("Failed to simulate concurrent server edit");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let stale = doc! { "_id": 1, "value": "first save", "nested": { "ok": true } };
    let result = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().replace_document_if_current(
            &client,
            &database,
            "conditional_replace",
            &mongodb::bson::Bson::Int32(1),
            &stale,
            doc! { "_id": 1, "value": "stale overwrite", "nested": { "ok": true } },
        )
    })
    .await
    .expect("Replace task panicked");

    assert!(result.is_err(), "stale baseline must fail closed");
    let current = collection
        .find_one(doc! { "_id": 1 })
        .await
        .expect("Failed to read final document")
        .expect("Document should still exist");
    assert_eq!(current.get_str("value").ok(), Some("server edit"));
}

#[tokio::test]
async fn test_csv_export_discovers_columns_after_first_thousand_documents() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "csv_all_columns");
    let mut documents: Vec<Document> =
        (0..1000).map(|index| doc! { "_id": index, "common": true }).collect();
    documents.push(doc! { "_id": 1000, "common": true, "late_field": "preserved" });
    collection.insert_many(documents).await.expect("Failed to seed collection");

    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("all-columns.csv");
    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let path = destination.clone();
    tokio::task::spawn_blocking(move || {
        ConnectionManager::new().export_collection_csv_with_query(
            &client,
            &database,
            "csv_all_columns",
            &path,
            false,
            openmango::connection::ExportQueryOptions::default(),
            None,
        )
    })
    .await
    .expect("Export task panicked")
    .expect("CSV export failed");

    let mut reader = csv::Reader::from_path(destination).unwrap();
    assert!(reader.headers().unwrap().iter().any(|header| header == "late_field"));
}

#[tokio::test]
async fn test_cancelled_json_export_preserves_existing_destination() {
    let mongo = MongoTestContainer::start().await;
    mongo
        .collection::<Document>("test_db", "atomic_cancel")
        .insert_one(doc! { "_id": 1, "value": "new" })
        .await
        .expect("Failed to seed collection");

    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("atomic.jsonl");
    std::fs::write(&destination, "existing\n").unwrap();
    let cancellation = openmango::connection::types::CancellationToken::new();
    cancellation.cancel();
    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let path = destination.clone();
    let result = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().export_collection_json_with_options(
            &client,
            &database,
            "atomic_cancel",
            &path,
            openmango::connection::JsonExportOptions {
                cancellation: Some(cancellation),
                ..Default::default()
            },
        )
    })
    .await
    .expect("Export task panicked");

    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(destination).unwrap(), "existing\n");
}

#[tokio::test]
async fn test_bulk_replace_preserves_ids_and_replaces_entire_documents() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "bulk_replace");
    collection
        .insert_many(vec![
            doc! { "_id": 1, "old": "one", "group": "target" },
            doc! { "_id": 2, "old": "two", "group": "target" },
        ])
        .await
        .expect("Failed to seed collection");

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let result = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().replace_documents_by_filter(
            &client,
            &database,
            "bulk_replace",
            doc! { "group": "target" },
            doc! { "replacement": true },
            openmango::connection::types::CancellationToken::new(),
        )
    })
    .await
    .expect("Replace task panicked")
    .expect("Bulk replacement failed");

    assert_eq!(result.matched_count, 2);
    let documents: Vec<Document> = collection
        .find(doc! {})
        .sort(doc! { "_id": 1 })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        documents,
        vec![doc! { "_id": 1, "replacement": true }, doc! { "_id": 2, "replacement": true },]
    );
}

#[tokio::test]
async fn test_bulk_replace_reports_duplicate_key_partial_execution() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "bulk_replace_duplicate");
    collection
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "slug": 1 })
                .options(mongodb::options::IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .unwrap();
    collection
        .insert_many(vec![doc! { "_id": 1, "slug": "a" }, doc! { "_id": 2, "slug": "b" }])
        .await
        .unwrap();

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let error = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().replace_documents_by_filter(
            &client,
            &database,
            "bulk_replace_duplicate",
            doc! {},
            doc! { "slug": "same" },
            openmango::connection::types::CancellationToken::new(),
        )
    })
    .await
    .expect("Replace task panicked")
    .expect_err("Unique conflict should fail");

    assert!(error.to_string().contains("after replacing 1 of 2"));
    assert_eq!(collection.count_documents(doc! { "slug": "same" }).await.unwrap(), 1);
}

#[tokio::test]
async fn test_bulk_replace_honors_preexisting_cancellation() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "bulk_replace_cancel");
    collection.insert_one(doc! { "_id": 1, "value": "original" }).await.unwrap();
    let cancellation = openmango::connection::types::CancellationToken::new();
    cancellation.cancel();

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let result = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().replace_documents_by_filter(
            &client,
            &database,
            "bulk_replace_cancel",
            doc! {},
            doc! { "value": "replacement" },
            cancellation,
        )
    })
    .await
    .expect("Replace task panicked");

    assert!(result.is_err());
    let current = collection.find_one(doc! { "_id": 1 }).await.unwrap().unwrap();
    assert_eq!(current.get_str("value").unwrap(), "original");
}

/// Test update with various operators.
#[tokio::test]
async fn test_update_with_operators() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "update_operators");

    // Insert a document
    collection
        .insert_one(doc! {
            "name": "test",
            "count": 10,
            "tags": ["a", "b"],
            "nested": { "value": 5 }
        })
        .await
        .expect("Failed to insert");

    // Test $set
    collection
        .update_one(doc! { "name": "test" }, doc! { "$set": { "new_field": "added" } })
        .await
        .expect("Failed to update");

    // Test $inc
    collection
        .update_one(doc! { "name": "test" }, doc! { "$inc": { "count": 5 } })
        .await
        .expect("Failed to update");

    // Test $push
    collection
        .update_one(doc! { "name": "test" }, doc! { "$push": { "tags": "c" } })
        .await
        .expect("Failed to update");

    // Test $pull
    collection
        .update_one(doc! { "name": "test" }, doc! { "$pull": { "tags": "a" } })
        .await
        .expect("Failed to update");

    // Verify all updates
    let doc = collection.find_one(doc! { "name": "test" }).await.expect("Failed to find").unwrap();
    assert_eq!(doc.get_str("new_field").unwrap(), "added");
    assert_eq!(doc.get_i32("count").unwrap(), 15); // 10 + 5
    let tags = doc.get_array("tags").unwrap();
    assert_eq!(tags.len(), 2); // ["b", "c"] after push "c" and pull "a"
}

/// Test update with upsert.
#[tokio::test]
async fn test_update_upsert() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "update_upsert");

    let options = mongodb::options::UpdateOptions::builder().upsert(true).build();

    // Upsert into empty collection (should insert)
    let result = collection
        .update_one(
            doc! { "name": "new_doc" },
            doc! { "$set": { "name": "new_doc", "value": 100 } },
        )
        .with_options(options.clone())
        .await
        .expect("Failed to upsert");

    assert!(result.upserted_id.is_some());
    assert_eq!(result.matched_count, 0);

    // Upsert existing document (should update)
    let result = collection
        .update_one(doc! { "name": "new_doc" }, doc! { "$set": { "value": 200 } })
        .with_options(options)
        .await
        .expect("Failed to upsert");

    assert!(result.upserted_id.is_none());
    assert_eq!(result.matched_count, 1);
    assert_eq!(result.modified_count, 1);

    // Verify final state
    let doc =
        collection.find_one(doc! { "name": "new_doc" }).await.expect("Failed to find").unwrap();
    assert_eq!(doc.get_i32("value").unwrap(), 200);
}

/// Test update with array filters.
#[tokio::test]
async fn test_update_array_filters() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "update_array_filters");

    // Insert document with array of objects
    collection
        .insert_one(doc! {
            "name": "test",
            "items": [
                { "type": "a", "value": 10 },
                { "type": "b", "value": 20 },
                { "type": "a", "value": 30 }
            ]
        })
        .await
        .expect("Failed to insert");

    // Update only items where type == "a" using arrayFilters
    let options = mongodb::options::UpdateOptions::builder()
        .array_filters(vec![doc! { "elem.type": "a" }])
        .build();

    collection
        .update_one(doc! { "name": "test" }, doc! { "$set": { "items.$[elem].updated": true } })
        .with_options(options)
        .await
        .expect("Failed to update");

    // Verify
    let doc = collection.find_one(doc! { "name": "test" }).await.expect("Failed to find").unwrap();
    let items = doc.get_array("items").unwrap();

    // First and third items should be updated (type "a")
    let item0 = items[0].as_document().unwrap();
    let item1 = items[1].as_document().unwrap();
    let item2 = items[2].as_document().unwrap();

    assert!(item0.get_bool("updated").unwrap_or(false));
    assert!(!item1.contains_key("updated")); // type "b" not updated
    assert!(item2.get_bool("updated").unwrap_or(false));
}

// =============================================================================
// Bulk Delete Tests
// =============================================================================

/// Test deleting many documents by filter.
#[tokio::test]
async fn test_delete_many_by_filter() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "delete_many_filter");

    // Insert test documents
    let docs = fixtures::generate_test_documents(30);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Delete all "odd" category documents
    let filter = doc! { "category": "odd" };
    let result = collection.delete_many(filter).await.expect("Failed to delete");

    // Should delete 15 documents (indices 1, 3, 5, ... 29)
    assert_eq!(result.deleted_count, 15);

    // Verify remaining documents
    let remaining = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(remaining, 15);

    // All remaining should be "even"
    let even_count =
        collection.count_documents(doc! { "category": "even" }).await.expect("Failed to count");
    assert_eq!(even_count, 15);
}

/// Test deleting all documents in a collection.
#[tokio::test]
async fn test_delete_all() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "delete_all");

    // Insert test documents
    let docs = fixtures::generate_test_documents(50);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Verify documents exist
    let before_count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(before_count, 50);

    // Delete all documents
    let result = collection.delete_many(doc! {}).await.expect("Failed to delete");
    assert_eq!(result.deleted_count, 50);

    // Verify collection is empty
    let after_count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(after_count, 0);
}

// =============================================================================
// Replace Tests
// =============================================================================

/// Test replacing a document.
#[tokio::test]
async fn test_replace_document() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "replace_doc");

    // Insert original document
    collection
        .insert_one(doc! {
            "_id": "test_id",
            "name": "original",
            "field1": "value1",
            "field2": "value2"
        })
        .await
        .expect("Failed to insert");

    // Replace with new document
    let replacement = doc! {
        "_id": "test_id",
        "name": "replaced",
        "new_field": "new_value"
    };

    let result = collection
        .replace_one(doc! { "_id": "test_id" }, replacement)
        .await
        .expect("Failed to replace");

    assert_eq!(result.modified_count, 1);

    // Verify replacement
    let doc =
        collection.find_one(doc! { "_id": "test_id" }).await.expect("Failed to find").unwrap();

    assert_eq!(doc.get_str("name").unwrap(), "replaced");
    assert_eq!(doc.get_str("new_field").unwrap(), "new_value");
    // Old fields should be gone
    assert!(doc.get("field1").is_none());
    assert!(doc.get("field2").is_none());
}

/// Test that replace preserves _id.
#[tokio::test]
async fn test_replace_preserves_id() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "replace_preserves_id");

    // Insert document with ObjectId
    let result = collection
        .insert_one(doc! { "name": "original", "value": 42 })
        .await
        .expect("Failed to insert");

    let original_id = result.inserted_id.as_object_id().unwrap();

    // Replace without specifying _id in replacement
    let replacement = doc! { "name": "replaced", "value": 100 };
    collection
        .replace_one(doc! { "_id": original_id }, replacement)
        .await
        .expect("Failed to replace");

    // Verify _id is preserved
    let doc =
        collection.find_one(doc! { "_id": original_id }).await.expect("Failed to find").unwrap();
    assert_eq!(doc.get_object_id("_id").unwrap(), original_id);
    assert_eq!(doc.get_str("name").unwrap(), "replaced");
    assert_eq!(doc.get_i32("value").unwrap(), 100);
}

// =============================================================================
// Additional Bulk Operation Tests
// =============================================================================

/// Test find and modify (findOneAndUpdate).
#[tokio::test]
async fn test_find_and_update() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "find_and_update");

    // Insert document
    collection.insert_one(doc! { "counter": 0, "name": "test" }).await.expect("Failed to insert");

    // Find and update with return_document: After
    let options = mongodb::options::FindOneAndUpdateOptions::builder()
        .return_document(mongodb::options::ReturnDocument::After)
        .build();

    let result = collection
        .find_one_and_update(doc! { "name": "test" }, doc! { "$inc": { "counter": 1 } })
        .with_options(options)
        .await
        .expect("Failed to find_one_and_update");

    let updated_doc = result.expect("Should return document");
    assert_eq!(updated_doc.get_i32("counter").unwrap(), 1);
}

/// Test find and delete (findOneAndDelete).
#[tokio::test]
async fn test_find_and_delete() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "find_and_delete");

    // Insert documents
    let docs = vec![doc! { "priority": 1, "name": "low" }, doc! { "priority": 10, "name": "high" }];
    collection.insert_many(docs).await.expect("Failed to insert");

    // Find and delete highest priority
    let options =
        mongodb::options::FindOneAndDeleteOptions::builder().sort(doc! { "priority": -1 }).build();

    let result = collection
        .find_one_and_delete(doc! {})
        .with_options(options)
        .await
        .expect("Failed to find_one_and_delete");

    let deleted_doc = result.expect("Should return deleted document");
    assert_eq!(deleted_doc.get_str("name").unwrap(), "high");

    // Verify only one document remains
    let count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(count, 1);
}

/// Test find and replace (findOneAndReplace).
#[tokio::test]
async fn test_find_and_replace() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "find_and_replace");

    // Insert document
    collection
        .insert_one(doc! { "status": "pending", "data": "old" })
        .await
        .expect("Failed to insert");

    // Find and replace
    let options = mongodb::options::FindOneAndReplaceOptions::builder()
        .return_document(mongodb::options::ReturnDocument::After)
        .build();

    let result = collection
        .find_one_and_replace(
            doc! { "status": "pending" },
            doc! { "status": "completed", "data": "new" },
        )
        .with_options(options)
        .await
        .expect("Failed to find_one_and_replace");

    let replaced_doc = result.expect("Should return document");
    assert_eq!(replaced_doc.get_str("status").unwrap(), "completed");
    assert_eq!(replaced_doc.get_str("data").unwrap(), "new");
}

/// Test bulk write operations.
#[tokio::test]
async fn test_bulk_write_operations() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "bulk_write");

    // Insert initial documents
    collection
        .insert_many(vec![
            doc! { "_id": 1, "name": "doc1", "value": 10 },
            doc! { "_id": 2, "name": "doc2", "value": 20 },
            doc! { "_id": 3, "name": "doc3", "value": 30 },
        ])
        .await
        .expect("Failed to insert");

    // Perform multiple operations using individual calls (simulating bulk write)
    // Insert new
    collection
        .insert_one(doc! { "_id": 4, "name": "doc4", "value": 40 })
        .await
        .expect("Insert failed");

    // Update one
    collection
        .update_one(doc! { "_id": 1 }, doc! { "$set": { "value": 100 } })
        .await
        .expect("Update failed");

    // Delete one
    collection.delete_one(doc! { "_id": 2 }).await.expect("Delete failed");

    // Replace one
    collection
        .replace_one(doc! { "_id": 3 }, doc! { "_id": 3, "name": "replaced", "value": 300 })
        .await
        .expect("Replace failed");

    // Verify all operations
    let all_docs: Vec<Document> = collection
        .find(doc! {})
        .await
        .expect("Find failed")
        .try_collect()
        .await
        .expect("Collect failed");

    assert_eq!(all_docs.len(), 3); // 4 original - 1 deleted + 1 new - wait, 3 + 1 - 1 = 3

    // Verify doc1 updated
    let doc1 = collection.find_one(doc! { "_id": 1 }).await.expect("Find failed").unwrap();
    assert_eq!(doc1.get_i32("value").unwrap(), 100);

    // Verify doc2 deleted
    let doc2 = collection.find_one(doc! { "_id": 2 }).await.expect("Find failed");
    assert!(doc2.is_none());

    // Verify doc3 replaced
    let doc3 = collection.find_one(doc! { "_id": 3 }).await.expect("Find failed").unwrap();
    assert_eq!(doc3.get_str("name").unwrap(), "replaced");
    assert_eq!(doc3.get_i32("value").unwrap(), 300);

    // Verify doc4 inserted
    let doc4 = collection.find_one(doc! { "_id": 4 }).await.expect("Find failed");
    assert!(doc4.is_some());
}
