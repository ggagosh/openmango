//! Test fixtures for integration tests.

#![allow(dead_code)]

use mongodb::bson::{Document, doc, oid::ObjectId};

/// Generate a batch of test documents for bulk operations.
pub fn generate_test_documents(count: usize) -> Vec<Document> {
    (0..count)
        .map(|i| {
            doc! {
                "index": i as i32,
                "name": format!("Document {}", i),
                "category": if i % 2 == 0 { "even" } else { "odd" },
                "value": (i * 10) as i32,
                "nested": {
                    "field": format!("nested_{}", i),
                    "number": i as i32,
                },
            }
        })
        .collect()
}

/// Generate a document with various BSON types for type handling tests.
pub fn document_with_all_types() -> Document {
    doc! {
        "_id": ObjectId::new(),
        "string": "hello world",
        "int32": 42_i32,
        "int64": 9_000_000_000_000_i64,
        "double": std::f64::consts::PI,
        "boolean": true,
        "null": null,
        "array": ["a", "b", "c"],
        "nested": {
            "key": "value",
            "deep": {
                "deeper": "bottom"
            }
        },
        "date": mongodb::bson::DateTime::now(),
    }
}

/// Generate test documents for aggregation pipeline tests.
pub fn aggregation_test_data() -> Vec<Document> {
    vec![
        doc! { "category": "A", "amount": 100, "quantity": 5 },
        doc! { "category": "B", "amount": 200, "quantity": 3 },
        doc! { "category": "A", "amount": 150, "quantity": 2 },
        doc! { "category": "C", "amount": 300, "quantity": 1 },
        doc! { "category": "B", "amount": 250, "quantity": 4 },
        doc! { "category": "A", "amount": 50, "quantity": 10 },
    ]
}
