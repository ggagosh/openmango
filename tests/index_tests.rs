//! Integration tests for MongoDB index operations using Testcontainers.

mod common;

use common::{MongoTestContainer, fixtures};
use futures::TryStreamExt;
use mongodb::IndexModel;
use mongodb::bson::{Document, doc};
use openmango::connection::ConnectionManager;

// =============================================================================
// Index CRUD Tests
// =============================================================================

#[tokio::test]
async fn conditional_index_create_and_drop_require_exact_metadata() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "conditional_index");
    collection.insert_one(doc! { "email": "ada@example.com" }).await.unwrap();
    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let (created, duplicate, current, stale_drop, dropped) =
        tokio::task::spawn_blocking(move || {
            let manager = ConnectionManager::new();
            let definition = doc! { "key": { "email": 1 }, "name": "email_idx", "unique": true };
            let created = manager.create_index_if_absent_matches(
                &client,
                &database,
                "conditional_index",
                definition.clone(),
            )?;
            let duplicate = manager.create_index_if_absent_matches(
                &client,
                &database,
                "conditional_index",
                definition.clone(),
            )?;
            let current = manager
                .find_index_document(&client, &database, "conditional_index", "email_idx")?
                .expect("created index missing");
            let stale_drop = manager.drop_index_if_current_matches(
                &client,
                &database,
                "conditional_index",
                "email_idx",
                &doc! { "key": { "other": 1 }, "name": "email_idx" },
            )?;
            let dropped = manager.drop_index_if_current_matches(
                &client,
                &database,
                "conditional_index",
                "email_idx",
                &current,
            )?;
            Ok::<_, openmango::error::Error>((created, duplicate, current, stale_drop, dropped))
        })
        .await
        .unwrap()
        .unwrap();

    assert!(created);
    assert!(!duplicate);
    assert_eq!(current.get_str("name").unwrap(), "email_idx");
    assert!(!stale_drop);
    assert!(dropped);
    assert!(!collection.list_index_names().await.unwrap().contains(&"email_idx".to_string()));
}

#[tokio::test]
async fn server_returned_text_and_collation_metadata_can_be_recreated() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "recreate_complex_index");
    collection.insert_one(doc! { "title": "Mango" }).await.unwrap();
    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let (collation_before, collation_after, text_before, text_after) =
        tokio::task::spawn_blocking(move || {
            let manager = ConnectionManager::new();
            manager.create_index(
                &client,
                &database,
                "recreate_complex_index",
                doc! {
                    "key": { "title": 1 },
                    "name": "title_collation",
                    "collation": { "locale": "en", "strength": 2 },
                },
            )?;
            let collation_before = manager
                .find_index_document(
                    &client,
                    &database,
                    "recreate_complex_index",
                    "title_collation",
                )?
                .expect("created collation index missing");
            assert!(manager.drop_index_if_current_matches(
                &client,
                &database,
                "recreate_complex_index",
                "title_collation",
                &collation_before,
            )?);
            assert!(manager.create_index_if_absent_matches(
                &client,
                &database,
                "recreate_complex_index",
                collation_before.clone(),
            )?);
            let collation_after = manager
                .find_index_document(
                    &client,
                    &database,
                    "recreate_complex_index",
                    "title_collation",
                )?
                .expect("recreated collation index missing");

            manager.create_index(
                &client,
                &database,
                "recreate_complex_index",
                doc! {
                    "key": { "title": "text" },
                    "name": "title_text",
                    "weights": { "title": 5 },
                    "default_language": "english",
                },
            )?;
            let text_before = manager
                .find_index_document(&client, &database, "recreate_complex_index", "title_text")?
                .expect("created text index missing");
            assert!(manager.drop_index_if_current_matches(
                &client,
                &database,
                "recreate_complex_index",
                "title_text",
                &text_before,
            )?);
            assert!(manager.create_index_if_absent_matches(
                &client,
                &database,
                "recreate_complex_index",
                text_before.clone(),
            )?);
            let text_after = manager
                .find_index_document(&client, &database, "recreate_complex_index", "title_text")?
                .expect("recreated text index missing");
            Ok::<_, openmango::error::Error>((
                collation_before,
                collation_after,
                text_before,
                text_after,
            ))
        })
        .await
        .unwrap()
        .unwrap();

    assert_eq!(collation_before, collation_after);
    assert_eq!(text_before, text_after);
}

#[tokio::test]
async fn test_same_name_index_replacement_keeps_final_name() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "replace_same_name");
    collection.insert_one(doc! { "old": 1, "new": 1 }).await.unwrap();
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "old": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("replace_me".to_string())
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    tokio::task::spawn_blocking(move || {
        ConnectionManager::new().replace_index(
            &client,
            &database,
            "replace_same_name",
            "replace_me",
            doc! { "key": { "new": 1 }, "name": "replace_me" },
        )
    })
    .await
    .unwrap()
    .expect("Replacement failed");

    let indexes: Vec<IndexModel> =
        collection.list_indexes().await.unwrap().try_collect().await.unwrap();
    let replacement = indexes
        .iter()
        .find(|index| {
            index.options.as_ref().and_then(|options| options.name.as_deref()) == Some("replace_me")
        })
        .expect("Replacement index missing");
    assert_eq!(replacement.keys, doc! { "new": 1 });
    assert!(!indexes.iter().any(|index| {
        index
            .options
            .as_ref()
            .and_then(|options| options.name.as_deref())
            .is_some_and(|name| name.starts_with("__openmango_validate_"))
    }));
}

#[tokio::test]
async fn test_failed_unique_replacement_preserves_original_index() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "replace_unique_failure");
    collection.insert_many(vec![doc! { "email": "same" }, doc! { "email": "same" }]).await.unwrap();
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "email": 1 })
                .options(
                    mongodb::options::IndexOptions::builder().name("email_idx".to_string()).build(),
                )
                .build(),
        )
        .await
        .unwrap();

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let result = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().replace_index(
            &client,
            &database,
            "replace_unique_failure",
            "email_idx",
            doc! { "key": { "email": 1 }, "name": "email_idx", "unique": true },
        )
    })
    .await
    .unwrap();
    assert!(result.is_err());

    let indexes: Vec<IndexModel> =
        collection.list_indexes().await.unwrap().try_collect().await.unwrap();
    let original = indexes
        .iter()
        .find(|index| {
            index.options.as_ref().and_then(|options| options.name.as_deref()) == Some("email_idx")
        })
        .expect("Original index was lost");
    assert_ne!(original.options.as_ref().and_then(|options| options.unique), Some(true));
}

#[tokio::test]
async fn test_invalid_replacement_preserves_original_index() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "replace_invalid");
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "value": 1 })
                .options(
                    mongodb::options::IndexOptions::builder().name("value_idx".to_string()).build(),
                )
                .build(),
        )
        .await
        .unwrap();

    let client = mongo.client.clone();
    let database = mongo.db_name("test_db");
    let result = tokio::task::spawn_blocking(move || {
        ConnectionManager::new().replace_index(
            &client,
            &database,
            "replace_invalid",
            "value_idx",
            doc! { "name": "value_idx" },
        )
    })
    .await
    .unwrap();
    assert!(result.is_err());

    assert!(collection.list_index_names().await.unwrap().contains(&"value_idx".to_string()));
}

/// Test creating a simple single-field index.
#[tokio::test]
async fn test_create_simple_index() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "simple_index");

    // Insert some data first
    let docs = fixtures::generate_test_documents(10);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Create index
    let index = IndexModel::builder()
        .keys(doc! { "name": 1 })
        .options(mongodb::options::IndexOptions::builder().name("name_idx".to_string()).build())
        .build();

    let result = collection.create_index(index).await.expect("Failed to create index");
    assert_eq!(result.index_name, "name_idx");

    // Verify index exists
    let indexes: Vec<IndexModel> = collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");

    let has_name_idx = indexes.iter().any(|idx| {
        idx.options.as_ref().and_then(|o| o.name.as_ref()).map(|n| n == "name_idx").unwrap_or(false)
    });
    assert!(has_name_idx);
}

/// Test creating a compound (multi-field) index.
#[tokio::test]
async fn test_create_compound_index() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "compound_index");

    // Insert data
    let docs = fixtures::generate_test_documents(10);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Create compound index
    let index = IndexModel::builder()
        .keys(doc! { "category": 1, "value": -1 })
        .options(
            mongodb::options::IndexOptions::builder()
                .name("category_value_idx".to_string())
                .build(),
        )
        .build();

    collection.create_index(index).await.expect("Failed to create index");

    // Verify index exists with correct keys
    let indexes: Vec<IndexModel> = collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");

    let compound_idx = indexes.iter().find(|idx| {
        idx.options
            .as_ref()
            .and_then(|o| o.name.as_ref())
            .map(|n| n == "category_value_idx")
            .unwrap_or(false)
    });
    assert!(compound_idx.is_some());

    let keys = &compound_idx.unwrap().keys;
    assert!(keys.get("category").is_some());
    assert!(keys.get("value").is_some());
}

/// Test creating a unique index.
#[tokio::test]
async fn test_create_unique_index() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "unique_index");

    // Create unique index first
    let index = IndexModel::builder()
        .keys(doc! { "email": 1 })
        .options(
            mongodb::options::IndexOptions::builder()
                .name("email_unique".to_string())
                .unique(true)
                .build(),
        )
        .build();

    collection.create_index(index).await.expect("Failed to create index");

    // Insert document
    collection.insert_one(doc! { "email": "test@example.com" }).await.expect("Failed to insert");

    // Try to insert duplicate - should fail
    let result = collection.insert_one(doc! { "email": "test@example.com" }).await;
    assert!(result.is_err());

    // Verify only one document exists
    let count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(count, 1);
}

/// Test creating a sparse index.
#[tokio::test]
async fn test_create_sparse_index() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "sparse_index");

    // Create sparse unique index
    let index = IndexModel::builder()
        .keys(doc! { "optional_field": 1 })
        .options(
            mongodb::options::IndexOptions::builder()
                .name("optional_sparse".to_string())
                .sparse(true)
                .unique(true)
                .build(),
        )
        .build();

    collection.create_index(index).await.expect("Failed to create index");

    // Insert documents with and without the optional field
    collection
        .insert_one(doc! { "name": "doc1", "optional_field": "value1" })
        .await
        .expect("Insert failed");
    collection.insert_one(doc! { "name": "doc2" }).await.expect("Insert failed"); // No optional_field
    collection.insert_one(doc! { "name": "doc3" }).await.expect("Insert failed"); // No optional_field

    // Multiple documents without the field should be allowed (sparse index)
    let count = collection.count_documents(doc! {}).await.expect("Failed to count");
    assert_eq!(count, 3);

    // But duplicate values in optional_field should fail
    let result = collection.insert_one(doc! { "name": "doc4", "optional_field": "value1" }).await;
    assert!(result.is_err());
}

/// Test creating a TTL (Time-To-Live) index.
#[tokio::test]
async fn test_create_ttl_index() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "ttl_index");

    // Create TTL index that expires documents after 1 second
    let index = IndexModel::builder()
        .keys(doc! { "createdAt": 1 })
        .options(
            mongodb::options::IndexOptions::builder()
                .name("ttl_idx".to_string())
                .expire_after(std::time::Duration::from_secs(1))
                .build(),
        )
        .build();

    collection.create_index(index).await.expect("Failed to create index");

    // Verify TTL index exists
    let indexes: Vec<IndexModel> = collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");

    let ttl_idx = indexes.iter().find(|idx| {
        idx.options.as_ref().and_then(|o| o.name.as_ref()).map(|n| n == "ttl_idx").unwrap_or(false)
    });
    assert!(ttl_idx.is_some());

    // Verify expire_after is set
    let expire = ttl_idx.unwrap().options.as_ref().and_then(|o| o.expire_after);
    assert!(expire.is_some());
    assert_eq!(expire.unwrap().as_secs(), 1);
}

/// Test creating a text index.
#[tokio::test]
async fn test_create_text_index() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "text_index");

    // Create text index
    let index = IndexModel::builder()
        .keys(doc! { "content": "text" })
        .options(mongodb::options::IndexOptions::builder().name("content_text".to_string()).build())
        .build();

    collection.create_index(index).await.expect("Failed to create index");

    // Insert documents
    collection
        .insert_one(doc! { "content": "The quick brown fox jumps over the lazy dog" })
        .await
        .expect("Insert failed");
    collection
        .insert_one(doc! { "content": "A fast orange fox leaps over a sleepy hound" })
        .await
        .expect("Insert failed");

    // Search using text index
    let filter = doc! { "$text": { "$search": "fox" } };
    let count = collection.count_documents(filter).await.expect("Failed to count");
    assert_eq!(count, 2);

    // Search for word that only appears in one document
    let filter = doc! { "$text": { "$search": "brown" } };
    let count = collection.count_documents(filter).await.expect("Failed to count");
    assert_eq!(count, 1);
}

/// Test listing all indexes.
#[tokio::test]
async fn test_list_indexes() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "list_indexes");

    // Create multiple indexes
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "field1": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("field1_idx".to_string())
                        .build(),
                )
                .build(),
        )
        .await
        .expect("Failed to create index");

    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "field2": -1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("field2_idx".to_string())
                        .build(),
                )
                .build(),
        )
        .await
        .expect("Failed to create index");

    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "field3": 1, "field4": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("compound_idx".to_string())
                        .build(),
                )
                .build(),
        )
        .await
        .expect("Failed to create index");

    // List indexes
    let indexes: Vec<IndexModel> = collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");

    // Should have 4 indexes: _id_ + 3 custom
    assert_eq!(indexes.len(), 4);

    // Verify all indexes exist
    let names: Vec<String> = indexes
        .iter()
        .filter_map(|idx| idx.options.as_ref().and_then(|o| o.name.clone()))
        .collect();

    assert!(names.contains(&"_id_".to_string()));
    assert!(names.contains(&"field1_idx".to_string()));
    assert!(names.contains(&"field2_idx".to_string()));
    assert!(names.contains(&"compound_idx".to_string()));
}

/// Test dropping an index by name.
#[tokio::test]
async fn test_drop_index() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "drop_index");

    // Create index
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "to_drop": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("to_drop_idx".to_string())
                        .build(),
                )
                .build(),
        )
        .await
        .expect("Failed to create index");

    // Verify it exists
    let indexes: Vec<IndexModel> = collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");
    assert_eq!(indexes.len(), 2); // _id_ + to_drop_idx

    // Drop the index
    collection.drop_index("to_drop_idx").await.expect("Failed to drop index");

    // Verify it's gone
    let indexes: Vec<IndexModel> = collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");
    assert_eq!(indexes.len(), 1); // Only _id_

    let has_to_drop = indexes.iter().any(|idx| {
        idx.options
            .as_ref()
            .and_then(|o| o.name.as_ref())
            .map(|n| n == "to_drop_idx")
            .unwrap_or(false)
    });
    assert!(!has_to_drop);
}

/// Test dropping all non-_id indexes.
#[tokio::test]
async fn test_drop_all_indexes() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "drop_all_indexes");

    // Create multiple indexes
    for i in 1..=3 {
        collection
            .create_index(
                IndexModel::builder()
                    .keys(doc! { format!("field{}", i): 1 })
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .name(format!("field{}_idx", i))
                            .build(),
                    )
                    .build(),
            )
            .await
            .expect("Failed to create index");
    }

    // Verify indexes exist
    let indexes: Vec<IndexModel> = collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");
    assert_eq!(indexes.len(), 4); // _id_ + 3 custom

    // Drop all indexes (except _id_)
    let db = mongo.database("test_db");
    db.run_command(doc! { "dropIndexes": "drop_all_indexes", "index": "*" })
        .await
        .expect("Failed to drop indexes");

    // Verify only _id_ remains
    let indexes: Vec<IndexModel> = collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");
    assert_eq!(indexes.len(), 1);

    let only_id = indexes[0]
        .options
        .as_ref()
        .and_then(|o| o.name.as_ref())
        .map(|n| n == "_id_")
        .unwrap_or(false);
    assert!(only_id);
}

// =============================================================================
// Index Options Tests
// =============================================================================

/// Test creating index with partial filter expression.
#[tokio::test]
async fn test_index_with_partial_filter() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "partial_filter_index");

    // Create partial index that only indexes documents where status == "active"
    let index = IndexModel::builder()
        .keys(doc! { "user_id": 1 })
        .options(
            mongodb::options::IndexOptions::builder()
                .name("active_users_idx".to_string())
                .partial_filter_expression(doc! { "status": "active" })
                .build(),
        )
        .build();

    collection.create_index(index).await.expect("Failed to create index");

    // Verify index exists
    let indexes: Vec<IndexModel> = collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");

    let partial_idx = indexes.iter().find(|idx| {
        idx.options
            .as_ref()
            .and_then(|o| o.name.as_ref())
            .map(|n| n == "active_users_idx")
            .unwrap_or(false)
    });
    assert!(partial_idx.is_some());

    // Verify partial filter expression is set
    let pfe =
        partial_idx.unwrap().options.as_ref().and_then(|o| o.partial_filter_expression.as_ref());
    assert!(pfe.is_some());
}

/// Test creating index in background (for older MongoDB versions) / with commit quorum.
#[tokio::test]
async fn test_index_background_option() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "background_index");

    // Insert some data first
    let docs = fixtures::generate_test_documents(100);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Note: background option is deprecated in MongoDB 4.2+, but we can still test other options
    // Using commit_quorum which is the modern replacement
    let index = IndexModel::builder()
        .keys(doc! { "value": 1 })
        .options(mongodb::options::IndexOptions::builder().name("value_idx".to_string()).build())
        .build();

    // Create index (will build in background by default in modern MongoDB)
    collection.create_index(index).await.expect("Failed to create index");

    // Verify index exists and can be used
    let indexes: Vec<IndexModel> = collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");

    let has_value_idx = indexes.iter().any(|idx| {
        idx.options
            .as_ref()
            .and_then(|o| o.name.as_ref())
            .map(|n| n == "value_idx")
            .unwrap_or(false)
    });
    assert!(has_value_idx);
}

// =============================================================================
// Index Usage Tests
// =============================================================================

/// Test that index is used for queries (using explain).
#[tokio::test]
async fn test_index_used_for_query() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "index_usage");

    // Insert data
    let docs = fixtures::generate_test_documents(100);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Create index
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "index": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("index_field_idx".to_string())
                        .build(),
                )
                .build(),
        )
        .await
        .expect("Failed to create index");

    // Run explain to verify index usage
    let db = mongo.database("test_db");
    let explain_result = db
        .run_command(doc! {
            "explain": {
                "find": "index_usage",
                "filter": { "index": 50 }
            },
            "verbosity": "queryPlanner"
        })
        .await
        .expect("Failed to run explain");

    // The explain output should reference our index
    // Note: The exact structure varies by MongoDB version
    let explain_str = format!("{:?}", explain_result);
    assert!(explain_str.contains("index_field_idx") || explain_str.contains("IXSCAN"));
}

/// Test creating many indexes at once.
#[tokio::test]
async fn test_create_many_indexes() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "many_indexes");

    // Create multiple indexes at once
    let indexes = vec![
        IndexModel::builder()
            .keys(doc! { "field1": 1 })
            .options(mongodb::options::IndexOptions::builder().name("idx1".to_string()).build())
            .build(),
        IndexModel::builder()
            .keys(doc! { "field2": 1 })
            .options(mongodb::options::IndexOptions::builder().name("idx2".to_string()).build())
            .build(),
        IndexModel::builder()
            .keys(doc! { "field3": 1 })
            .options(mongodb::options::IndexOptions::builder().name("idx3".to_string()).build())
            .build(),
    ];

    let results = collection.create_indexes(indexes).await.expect("Failed to create indexes");
    assert_eq!(results.index_names.len(), 3);

    // Verify all indexes exist
    let all_indexes: Vec<IndexModel> = collection
        .list_indexes()
        .await
        .expect("Failed to list")
        .try_collect()
        .await
        .expect("Failed to collect");
    assert_eq!(all_indexes.len(), 4); // _id_ + 3 new indexes
}
