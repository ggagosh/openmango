//! Integration tests for MongoDB aggregation pipelines using Testcontainers.

mod common;

use common::{MongoTestContainer, fixtures};
use futures::TryStreamExt;
use mongodb::bson::doc;

/// Test basic $match stage.
#[tokio::test]
async fn test_aggregation_match_stage() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "agg_collection");

    // Insert test data
    let docs = fixtures::aggregation_test_data();
    collection.insert_many(docs).await.expect("Failed to insert");

    // Run aggregation with $match
    let pipeline = vec![doc! { "$match": { "category": "A" } }];
    let mut cursor = collection.aggregate(pipeline).await.expect("Failed to aggregate");

    let mut results = Vec::new();
    while let Some(doc) = cursor.try_next().await.expect("Cursor error") {
        results.push(doc);
    }

    assert_eq!(results.len(), 3); // 3 documents with category A
}

/// Test $group stage with aggregation operators.
#[tokio::test]
async fn test_aggregation_group_stage() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "group_collection");

    // Insert test data
    let docs = fixtures::aggregation_test_data();
    collection.insert_many(docs).await.expect("Failed to insert");

    // Group by category, sum amounts
    let pipeline = vec![
        doc! {
            "$group": {
                "_id": "$category",
                "totalAmount": { "$sum": "$amount" },
                "count": { "$sum": 1 }
            }
        },
        doc! { "$sort": { "_id": 1 } },
    ];

    let cursor = collection.aggregate(pipeline).await.expect("Failed to aggregate");
    let results: Vec<_> = cursor.try_collect().await.expect("Failed to collect");

    assert_eq!(results.len(), 3); // A, B, C

    // Find category A result
    let cat_a = results.iter().find(|d| d.get_str("_id").unwrap() == "A").unwrap();
    assert_eq!(cat_a.get_i32("totalAmount").unwrap(), 300); // 100 + 150 + 50
    assert_eq!(cat_a.get_i32("count").unwrap(), 3);
}

/// Test $project stage.
#[tokio::test]
async fn test_aggregation_project_stage() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "project_collection");

    // Insert test data
    let docs = fixtures::aggregation_test_data();
    collection.insert_many(docs).await.expect("Failed to insert");

    // Project only specific fields and compute new ones
    let pipeline = vec![
        doc! {
            "$project": {
                "_id": 0,
                "category": 1,
                "total": { "$multiply": ["$amount", "$quantity"] }
            }
        },
        doc! { "$limit": 3 },
    ];

    let cursor = collection.aggregate(pipeline).await.expect("Failed to aggregate");
    let results: Vec<_> = cursor.try_collect().await.expect("Failed to collect");

    assert_eq!(results.len(), 3);

    // Each result should only have category and total
    for result in &results {
        assert!(result.get("_id").is_none());
        assert!(result.get_str("category").is_ok());
        assert!(result.get_i32("total").is_ok());
    }
}

/// Test $sort and $limit stages.
#[tokio::test]
async fn test_aggregation_sort_and_limit() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "sort_collection");

    // Insert test data
    let docs = fixtures::aggregation_test_data();
    collection.insert_many(docs).await.expect("Failed to insert");

    // Sort by amount descending, limit to 2
    let pipeline = vec![doc! { "$sort": { "amount": -1 } }, doc! { "$limit": 2 }];

    let cursor = collection.aggregate(pipeline).await.expect("Failed to aggregate");
    let results: Vec<_> = cursor.try_collect().await.expect("Failed to collect");

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].get_i32("amount").unwrap(), 300); // Highest
    assert_eq!(results[1].get_i32("amount").unwrap(), 250); // Second highest
}

/// Test $skip stage for pagination.
#[tokio::test]
async fn test_aggregation_skip_for_pagination() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "skip_collection");

    // Insert test data
    let docs = fixtures::generate_test_documents(20);
    collection.insert_many(docs).await.expect("Failed to insert");

    // Get page 2 (skip 10, limit 10)
    let pipeline =
        vec![doc! { "$sort": { "index": 1 } }, doc! { "$skip": 10 }, doc! { "$limit": 10 }];

    let cursor = collection.aggregate(pipeline).await.expect("Failed to aggregate");
    let results: Vec<_> = cursor.try_collect().await.expect("Failed to collect");

    assert_eq!(results.len(), 10);
    assert_eq!(results[0].get_i32("index").unwrap(), 10); // First of page 2
    assert_eq!(results[9].get_i32("index").unwrap(), 19); // Last of page 2
}

/// Test $count stage.
#[tokio::test]
async fn test_aggregation_count_stage() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "count_collection");

    // Insert test data
    let docs = fixtures::aggregation_test_data();
    collection.insert_many(docs).await.expect("Failed to insert");

    // Count documents matching category A
    let pipeline = vec![doc! { "$match": { "category": "A" } }, doc! { "$count": "total" }];

    let cursor = collection.aggregate(pipeline).await.expect("Failed to aggregate");
    let results: Vec<_> = cursor.try_collect().await.expect("Failed to collect");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get_i32("total").unwrap(), 3);
}

/// Test $unwind stage for array expansion.
#[tokio::test]
async fn test_aggregation_unwind_stage() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "unwind_collection");

    // Insert documents with arrays
    let docs = vec![
        doc! { "name": "doc1", "tags": ["a", "b", "c"] },
        doc! { "name": "doc2", "tags": ["x", "y"] },
    ];
    collection.insert_many(docs).await.expect("Failed to insert");

    // Unwind the tags array
    let pipeline = vec![doc! { "$unwind": "$tags" }, doc! { "$sort": { "tags": 1 } }];

    let cursor = collection.aggregate(pipeline).await.expect("Failed to aggregate");
    let results: Vec<_> = cursor.try_collect().await.expect("Failed to collect");

    // Should have 5 documents (3 + 2 tags)
    assert_eq!(results.len(), 5);

    // Tags should be sorted
    let tags: Vec<_> = results.iter().map(|d| d.get_str("tags").unwrap()).collect();
    assert_eq!(tags, vec!["a", "b", "c", "x", "y"]);
}

/// Test multi-stage pipeline.
#[tokio::test]
async fn test_aggregation_multi_stage_pipeline() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<mongodb::bson::Document>("test_db", "multi_collection");

    // Insert test data
    let docs = fixtures::aggregation_test_data();
    collection.insert_many(docs).await.expect("Failed to insert");

    // Complex pipeline: filter, group, project, sort
    let pipeline = vec![
        doc! { "$match": { "amount": { "$gte": 100 } } },
        doc! {
            "$group": {
                "_id": "$category",
                "avgAmount": { "$avg": "$amount" },
                "totalQuantity": { "$sum": "$quantity" }
            }
        },
        doc! {
            "$project": {
                "category": "$_id",
                "avgAmount": 1,
                "totalQuantity": 1,
                "_id": 0
            }
        },
        doc! { "$sort": { "avgAmount": -1 } },
    ];

    let cursor = collection.aggregate(pipeline).await.expect("Failed to aggregate");
    let results: Vec<_> = cursor.try_collect().await.expect("Failed to collect");

    // Should have results for categories with amount >= 100
    assert!(!results.is_empty());

    // Results should be sorted by avgAmount descending
    for window in results.windows(2) {
        let avg1 = window[0].get_f64("avgAmount").unwrap();
        let avg2 = window[1].get_f64("avgAmount").unwrap();
        assert!(avg1 >= avg2);
    }
}
