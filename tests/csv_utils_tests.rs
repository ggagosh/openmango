//! Integration tests for CSV flatten/unflatten utilities (`openmango::connection::csv_utils`).
//!
//! No MongoDB container needed — pure function tests on BSON documents.

use std::collections::HashMap;

use mongodb::bson::{Bson, doc, oid::ObjectId};
use openmango::connection::csv_utils::{collect_columns, flatten_document, unflatten_row};

// =============================================================================
// flatten_document — simple flat documents
// =============================================================================

#[test]
fn test_flatten_simple() {
    let doc = doc! { "name": "Alice", "age": 30, "active": true };
    let flat = flatten_document(&doc);

    assert_eq!(flat.get("name"), Some(&"Alice".to_string()));
    assert_eq!(flat.get("age"), Some(&"30".to_string()));
    assert_eq!(flat.get("active"), Some(&"true".to_string()));
    assert_eq!(flat.len(), 3);
}

// =============================================================================
// flatten_document — nested documents use dot-notation
// =============================================================================

#[test]
fn test_flatten_nested() {
    let doc = doc! {
        "user": {
            "name": "Bob",
            "address": {
                "city": "NYC",
                "zip": "10001"
            }
        }
    };
    let flat = flatten_document(&doc);

    assert_eq!(flat.get("user.name"), Some(&"Bob".to_string()));
    assert_eq!(flat.get("user.address.city"), Some(&"NYC".to_string()));
    assert_eq!(flat.get("user.address.zip"), Some(&"10001".to_string()));
    // Top-level "user" should not appear as a standalone key
    assert!(!flat.contains_key("user"));
}

// =============================================================================
// flatten_document — arrays serialized as JSON strings
// =============================================================================

#[test]
fn test_flatten_with_arrays() {
    let doc = doc! { "tags": ["rust", "mongodb"], "name": "test" };
    let flat = flatten_document(&doc);

    let tags_value = flat.get("tags").unwrap();
    // Should be a JSON string representation of the array
    assert!(tags_value.contains("rust"));
    assert!(tags_value.contains("mongodb"));
    assert_eq!(flat.get("name"), Some(&"test".to_string()));
}

// =============================================================================
// flatten_document — BSON types serialize correctly
// =============================================================================

#[test]
fn test_flatten_with_bson_types() {
    let oid = ObjectId::parse_str("6283a37e34d71078c4996c72").unwrap();
    let dt = mongodb::bson::DateTime::parse_rfc3339_str("2023-06-15T12:00:00Z").unwrap();

    let doc = doc! {
        "_id": oid,
        "created": dt,
        "active": true,
        "count": 42_i32,
        "big": 9_999_999_999_i64,
        "score": 2.78,
        "empty": Bson::Null,
    };
    let flat = flatten_document(&doc);

    // ObjectId → hex string
    assert_eq!(flat.get("_id"), Some(&"6283a37e34d71078c4996c72".to_string()));
    // Boolean
    assert_eq!(flat.get("active"), Some(&"true".to_string()));
    // Int32
    assert_eq!(flat.get("count"), Some(&"42".to_string()));
    // Int64
    assert_eq!(flat.get("big"), Some(&"9999999999".to_string()));
    // Double
    assert_eq!(flat.get("score"), Some(&"2.78".to_string()));
    // Null → empty string
    assert_eq!(flat.get("empty"), Some(&String::new()));
    // DateTime → RFC3339 string
    let created = flat.get("created").unwrap();
    assert!(created.contains("2023"));
}

// =============================================================================
// unflatten_row — simple values with type inference
// =============================================================================

#[test]
fn test_unflatten_simple() {
    let mut row = HashMap::new();
    row.insert("name".to_string(), "Alice".to_string());
    row.insert("age".to_string(), "30".to_string());
    row.insert("score".to_string(), "2.78".to_string());
    row.insert("active".to_string(), "true".to_string());

    let doc = unflatten_row(&row);

    assert_eq!(doc.get_str("name"), Ok("Alice"));
    // Integer inference
    assert_eq!(doc.get_i32("age"), Ok(30));
    // Float inference
    assert!(matches!(doc.get("score"), Some(Bson::Double(v)) if (*v - 2.78).abs() < 1e-9));
    // Boolean inference
    assert_eq!(doc.get_bool("active"), Ok(true));
}

// =============================================================================
// unflatten_row — dot-notation keys produce nested documents
// =============================================================================

#[test]
fn test_unflatten_nested() {
    let mut row = HashMap::new();
    row.insert("user.name".to_string(), "Bob".to_string());
    row.insert("user.address.city".to_string(), "NYC".to_string());
    row.insert("user.address.zip".to_string(), "10001".to_string());

    let doc = unflatten_row(&row);
    let user = doc.get_document("user").unwrap();
    assert_eq!(user.get_str("name"), Ok("Bob"));

    let address = user.get_document("address").unwrap();
    assert_eq!(address.get_str("city"), Ok("NYC"));
    // "10001" looks like an int → should parse as Int32
    assert_eq!(address.get_i32("zip"), Ok(10001));
}

// =============================================================================
// collect_columns — ordering and deduplication
// =============================================================================

#[test]
fn test_collect_columns_ordering() {
    let docs = vec![
        doc! { "_id": 1, "name": "Alice", "age": 30 },
        doc! { "_id": 2, "name": "Bob", "email": "bob@test.com" },
        doc! { "_id": 3, "name": "Charlie", "age": 25, "city": "LA" },
    ];

    let columns = collect_columns(&docs);

    // All unique leaf keys should be present
    assert!(columns.contains(&"_id".to_string()));
    assert!(columns.contains(&"name".to_string()));
    assert!(columns.contains(&"age".to_string()));
    assert!(columns.contains(&"email".to_string()));
    assert!(columns.contains(&"city".to_string()));

    // No duplicates
    let unique: std::collections::HashSet<&String> = columns.iter().collect();
    assert_eq!(unique.len(), columns.len());

    // _id should appear first (it's in the first doc)
    assert_eq!(columns[0], "_id");
}

// =============================================================================
// collect_columns — nested doc keys use dot-notation
// =============================================================================

#[test]
fn test_collect_columns_nested() {
    let docs = vec![doc! {
        "_id": 1,
        "user": { "name": "Alice", "address": { "city": "NYC" } },
        "active": true
    }];

    let columns = collect_columns(&docs);

    assert!(columns.contains(&"_id".to_string()));
    assert!(columns.contains(&"user.name".to_string()));
    assert!(columns.contains(&"user.address.city".to_string()));
    assert!(columns.contains(&"active".to_string()));
    // "user" and "user.address" should NOT be columns (they're intermediate docs)
    assert!(!columns.iter().any(|c| c == "user"));
    assert!(!columns.iter().any(|c| c == "user.address"));
}

// =============================================================================
// flatten → unflatten roundtrip (simple types)
// =============================================================================

#[test]
fn test_flatten_unflatten_roundtrip() {
    let original = doc! {
        "name": "Alice",
        "age": 30_i32,
        "active": true,
        "score": 2.78,
    };

    let flat = flatten_document(&original);
    let row: HashMap<String, String> = flat.into_iter().collect();
    let restored = unflatten_row(&row);

    assert_eq!(restored.get_str("name"), Ok("Alice"));
    assert_eq!(restored.get_i32("age"), Ok(30));
    assert_eq!(restored.get_bool("active"), Ok(true));
    assert!(matches!(restored.get("score"), Some(Bson::Double(v)) if (*v - 2.78).abs() < 1e-9));
}
