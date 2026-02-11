//! Integration tests for BSON shell constructor parsing (`openmango::bson::parse_*`).
//!
//! These tests cover the shell syntax preprocessor (ObjectId(), ISODate(), etc.)
//! which runs entirely in-process — no MongoDB container needed.

use mongodb::bson::oid::ObjectId;
use mongodb::bson::spec::BinarySubtype;
use mongodb::bson::{Bson, DateTime};
use openmango::bson::{parse_document_from_json, parse_documents_from_json};

// =============================================================================
// Shell constructor: ObjectId()
// =============================================================================

#[test]
fn test_object_id_shell_syntax() {
    let doc = parse_document_from_json(r#"{ _id: ObjectId("6283a37e34d71078c4996c72") }"#).unwrap();
    let oid = doc.get_object_id("_id").unwrap();
    assert_eq!(oid, ObjectId::parse_str("6283a37e34d71078c4996c72").unwrap());

    // ObjectID (alternate casing)
    let doc2 =
        parse_document_from_json(r#"{ _id: ObjectID("6283a37e34d71078c4996c72") }"#).unwrap();
    assert_eq!(doc2.get_object_id("_id").unwrap(), oid);
}

// =============================================================================
// Shell constructor: ISODate() / Date()
// =============================================================================

#[test]
fn test_iso_date_shell_syntax() {
    let doc =
        parse_document_from_json(r#"{ createdAt: ISODate("2020-01-01T00:00:00Z") }"#).unwrap();
    let dt = *doc.get_datetime("createdAt").unwrap();
    let expected = DateTime::parse_rfc3339_str("2020-01-01T00:00:00Z").unwrap();
    assert_eq!(dt, expected);

    // Date() alias
    let doc2 = parse_document_from_json(r#"{ createdAt: Date("2020-06-15T12:30:00Z") }"#).unwrap();
    let dt2 = *doc2.get_datetime("createdAt").unwrap();
    assert_eq!(dt2, DateTime::parse_rfc3339_str("2020-06-15T12:30:00Z").unwrap());
}

// =============================================================================
// Shell constructors: NumberLong, NumberInt, NumberDouble
// =============================================================================

#[test]
fn test_numeric_shell_syntax() {
    let doc = parse_document_from_json(
        r#"{ long: NumberLong("42"), int: NumberInt(7), dbl: NumberDouble(3.5) }"#,
    )
    .unwrap();

    assert!(matches!(doc.get("long"), Some(Bson::Int64(42))));
    assert!(matches!(doc.get("int"), Some(Bson::Int32(7))));
    assert!(matches!(doc.get("dbl"), Some(Bson::Double(v)) if (*v - 3.5).abs() < 1e-9));

    // NumberLong with raw number (no quotes)
    let doc2 = parse_document_from_json(r#"{ n: NumberLong(9999999999) }"#).unwrap();
    assert!(matches!(doc2.get("n"), Some(Bson::Int64(9999999999))));
}

// =============================================================================
// Shell constructors: NumberDecimal, UUID
// =============================================================================

#[test]
fn test_decimal_and_uuid_shell_syntax() {
    let doc = parse_document_from_json(
        r#"{ dec: NumberDecimal("1.25"), id: UUID("00112233-4455-6677-8899-aabbccddeeff") }"#,
    )
    .unwrap();

    assert!(matches!(doc.get("dec"), Some(Bson::Decimal128(_))));

    if let Some(Bson::Binary(bin)) = doc.get("id") {
        assert_eq!(bin.subtype, BinarySubtype::Uuid);
        assert_eq!(bin.bytes.len(), 16);
    } else {
        panic!("expected UUID binary");
    }
}

// =============================================================================
// Shell constructor: Timestamp(t, i)
// =============================================================================

#[test]
fn test_timestamp_shell_syntax() {
    let doc = parse_document_from_json(r#"{ ts: Timestamp(5, 7) }"#).unwrap();
    if let Some(Bson::Timestamp(ts)) = doc.get("ts") {
        assert_eq!(ts.time, 5);
        assert_eq!(ts.increment, 7);
    } else {
        panic!("expected Timestamp");
    }

    // Larger values
    let doc2 = parse_document_from_json(r#"{ ts: Timestamp(1700000000, 1) }"#).unwrap();
    if let Some(Bson::Timestamp(ts)) = doc2.get("ts") {
        assert_eq!(ts.time, 1_700_000_000);
        assert_eq!(ts.increment, 1);
    } else {
        panic!("expected Timestamp");
    }
}

// =============================================================================
// Shell constructors inside strings should NOT be replaced
// =============================================================================

#[test]
fn test_shell_syntax_inside_strings_not_replaced() {
    let doc = parse_document_from_json(r#"{ note: "ObjectId(\"abc\")" }"#).unwrap();
    let note = doc.get_str("note").unwrap();
    assert_eq!(note, r#"ObjectId("abc")"#);

    // ISODate inside a string
    let doc2 =
        parse_document_from_json(r#"{ msg: "Use ISODate(\"2020-01-01T00:00:00Z\")" }"#).unwrap();
    let msg = doc2.get_str("msg").unwrap();
    assert!(msg.contains("ISODate("));
}

// =============================================================================
// JSON5: unquoted keys
// =============================================================================

#[test]
fn test_parse_json5_unquoted_keys() {
    let doc = parse_document_from_json(r#"{ name: "test", age: 25 }"#).unwrap();
    assert_eq!(doc.get_str("name"), Ok("test"));
    assert!(matches!(doc.get("age"), Some(Bson::Int32(25))));
}

// =============================================================================
// JSON5: trailing commas
// =============================================================================

#[test]
fn test_parse_json5_trailing_comma() {
    let doc = parse_document_from_json(r#"{ a: 1, b: 2, }"#).unwrap();
    assert!(matches!(doc.get("a"), Some(Bson::Int32(1))));
    assert!(matches!(doc.get("b"), Some(Bson::Int32(2))));
}

// =============================================================================
// parse_documents_from_json — array of documents
// =============================================================================

#[test]
fn test_parse_documents_array() {
    let docs = parse_documents_from_json(r#"[{ "a": 1 }, { "b": 2 }]"#).unwrap();
    assert_eq!(docs.len(), 2);
    assert!(matches!(docs[0].get("a"), Some(Bson::Int32(1))));
    assert!(matches!(docs[1].get("b"), Some(Bson::Int32(2))));

    // Single document (not wrapped in array) → vec of one
    let docs2 = parse_documents_from_json(r#"{ "x": 42 }"#).unwrap();
    assert_eq!(docs2.len(), 1);
    assert!(matches!(docs2[0].get("x"), Some(Bson::Int32(42))));
}

// =============================================================================
// Malformed input → Err
// =============================================================================

#[test]
fn test_parse_invalid_json_error() {
    assert!(parse_document_from_json("").is_err());
    assert!(parse_document_from_json("{{{").is_err());
    assert!(parse_document_from_json("not json at all").is_err());
    assert!(parse_documents_from_json("").is_err());
    assert!(parse_documents_from_json("   ").is_err());
}

// =============================================================================
// Nested shell constructors (e.g. inside query operators)
// =============================================================================

#[test]
fn test_nested_shell_constructors() {
    let doc =
        parse_document_from_json(r#"{ createdAt: { $gte: ISODate("2023-01-01T00:00:00Z") } }"#)
            .unwrap();

    let inner = doc.get_document("createdAt").unwrap();
    let gte = inner.get_datetime("$gte").unwrap();
    assert_eq!(*gte, DateTime::parse_rfc3339_str("2023-01-01T00:00:00Z").unwrap());

    // ObjectId in $in array
    let doc2 = parse_document_from_json(
        r#"{ _id: { $in: [ObjectId("6283a37e34d71078c4996c72"), ObjectId("6283a37e34d71078c4996c73")] } }"#,
    )
    .unwrap();
    let inner2 = doc2.get_document("_id").unwrap();
    if let Some(Bson::Array(arr)) = inner2.get("$in") {
        assert_eq!(arr.len(), 2);
        assert!(matches!(&arr[0], Bson::ObjectId(_)));
        assert!(matches!(&arr[1], Bson::ObjectId(_)));
    } else {
        panic!("expected $in array");
    }
}
