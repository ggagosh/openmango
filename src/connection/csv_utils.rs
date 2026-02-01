//! CSV utilities for BSON document flattening and unflattening.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};

use mongodb::bson::{Bson, Document};

/// Flatten a BSON document into a map of dot-notation keys to string values.
/// Nested documents use dot notation (e.g., "address.city").
/// Arrays are serialized as JSON strings.
///
/// Optimized to avoid cloning the document - iterates by reference.
pub fn flatten_document(doc: &Document) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    flatten_document_ref(doc, "", &mut result);
    result
}

/// Internal: Flatten document by reference without wrapping in Bson::Document.
fn flatten_document_ref(doc: &Document, prefix: &str, result: &mut BTreeMap<String, String>) {
    for (key, value) in doc {
        // Avoid allocation for top-level keys (empty prefix)
        let new_key: Cow<str> = if prefix.is_empty() {
            Cow::Borrowed(key)
        } else {
            Cow::Owned(format!("{prefix}.{key}"))
        };

        flatten_value_ref(value, &new_key, result);
    }
}

/// Internal: Flatten a BSON value by reference.
fn flatten_value_ref(value: &Bson, key: &str, result: &mut BTreeMap<String, String>) {
    match value {
        Bson::Document(doc) => {
            flatten_document_ref(doc, key, result);
        }
        Bson::Array(_) => {
            // Serialize arrays as JSON strings
            let json = bson_to_json_string(value);
            result.insert(key.to_string(), json);
        }
        _ => {
            result.insert(key.to_string(), bson_to_csv_string(value));
        }
    }
}

/// Convert a BSON value to a string suitable for CSV output.
fn bson_to_csv_string(value: &Bson) -> String {
    match value {
        Bson::Null => String::new(),
        Bson::Boolean(b) => b.to_string(),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(n) => n.to_string(),
        Bson::String(s) => s.clone(),
        Bson::ObjectId(oid) => oid.to_hex(),
        Bson::DateTime(dt) => dt.try_to_rfc3339_string().unwrap_or_else(|_| format!("{dt:?}")),
        Bson::Decimal128(d) => d.to_string(),
        other => bson_to_json_string(other),
    }
}

/// Convert any BSON value to a JSON string (for complex types).
fn bson_to_json_string(value: &Bson) -> String {
    let json_value = value.clone().into_relaxed_extjson();
    serde_json::to_string(&json_value).unwrap_or_default()
}

/// Collect all unique column names from a set of documents.
/// Optimized to extract keys without computing values (skips value serialization).
pub fn collect_columns(docs: &[Document]) -> Vec<String> {
    let mut columns_set = HashSet::new();
    let mut columns_order = Vec::new();

    for doc in docs {
        collect_keys_from_doc(doc, "", &mut columns_set, &mut columns_order);
    }

    columns_order
}

/// Extract flattened keys from a document without computing values.
/// More efficient than flatten_document when only keys are needed.
fn collect_keys_from_doc(
    doc: &Document,
    prefix: &str,
    seen: &mut HashSet<String>,
    order: &mut Vec<String>,
) {
    for (key, value) in doc {
        let full_key = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };

        match value {
            Bson::Document(nested) => {
                // Recurse into nested documents
                collect_keys_from_doc(nested, &full_key, seen, order);
            }
            _ => {
                // Leaf value (including arrays which become single columns)
                if seen.insert(full_key.clone()) {
                    order.push(full_key);
                }
            }
        }
    }
}

/// Unflatten a CSV row (map of column name -> value) back into a BSON Document.
pub fn unflatten_row(row: &HashMap<String, String>) -> Document {
    let mut doc = Document::new();

    for (key, value) in row {
        set_nested_value(&mut doc, key, value);
    }

    doc
}

fn set_nested_value(doc: &mut Document, path: &str, value: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    set_nested_value_recursive(doc, &parts, value);
}

fn set_nested_value_recursive(doc: &mut Document, parts: &[&str], value: &str) {
    if parts.is_empty() {
        return;
    }

    let key = parts[0];

    if parts.len() == 1 {
        // Leaf node - try to parse the value
        doc.insert(key.to_string(), parse_csv_value(value));
    } else {
        // Intermediate node - ensure it's a document
        let nested = doc.entry(key.to_string()).or_insert_with(|| Bson::Document(Document::new()));

        if let Bson::Document(nested_doc) = nested {
            set_nested_value_recursive(nested_doc, &parts[1..], value);
        }
    }
}

/// Parse a CSV string value back into an appropriate BSON type.
fn parse_csv_value(value: &str) -> Bson {
    let trimmed = value.trim();

    // Empty string -> Null
    if trimmed.is_empty() {
        return Bson::Null;
    }

    // Try to parse as boolean
    if trimmed.eq_ignore_ascii_case("true") {
        return Bson::Boolean(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Bson::Boolean(false);
    }

    // Try to parse as integer
    if let Ok(n) = trimmed.parse::<i64>() {
        if n >= i32::MIN as i64 && n <= i32::MAX as i64 {
            return Bson::Int32(n as i32);
        }
        return Bson::Int64(n);
    }

    // Try to parse as double
    if let Ok(n) = trimmed.parse::<f64>() {
        return Bson::Double(n);
    }

    // Try to parse as ObjectId (24-char hex)
    if trimmed.len() == 24
        && trimmed.chars().all(|c| c.is_ascii_hexdigit())
        && let Ok(oid) = mongodb::bson::oid::ObjectId::parse_str(trimmed)
    {
        return Bson::ObjectId(oid);
    }

    // Try to parse as JSON (for arrays and nested objects)
    if ((trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('{') && trimmed.ends_with('}')))
        && let Ok(json_value) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Ok(bson) = mongodb::bson::Bson::try_from(json_value)
    {
        return bson;
    }

    // Default to string
    Bson::String(value.to_string())
}

/// Detect problematic fields for CSV export (fields that will lose type fidelity).
pub fn detect_problematic_fields(docs: &[Document]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut seen_types: HashMap<String, Vec<&'static str>> = HashMap::new();

    for doc in docs {
        check_document_types(doc, "", &mut seen_types);
    }

    for (path, types) in &seen_types {
        // Warn about complex types that lose fidelity
        for typ in types {
            match *typ {
                "Binary" | "Decimal128" | "Timestamp" | "RegularExpression" | "JavaScriptCode"
                | "MinKey" | "MaxKey" | "DbPointer" | "Symbol" => {
                    warnings.push(format!(
                        "Field '{}' contains {} which may lose type information",
                        path, typ
                    ));
                }
                _ => {}
            }
        }
    }

    warnings
}

fn check_document_types(
    doc: &Document,
    prefix: &str,
    seen: &mut HashMap<String, Vec<&'static str>>,
) {
    for (key, value) in doc {
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        let type_name = bson_type_name(value);

        seen.entry(path.clone()).or_default().push(type_name);

        if let Bson::Document(nested) = value {
            check_document_types(nested, &path, seen);
        }
    }
}

fn bson_type_name(value: &Bson) -> &'static str {
    match value {
        Bson::Double(_) => "Double",
        Bson::String(_) => "String",
        Bson::Document(_) => "Document",
        Bson::Array(_) => "Array",
        Bson::Binary(_) => "Binary",
        Bson::ObjectId(_) => "ObjectId",
        Bson::Boolean(_) => "Boolean",
        Bson::DateTime(_) => "DateTime",
        Bson::Null => "Null",
        Bson::RegularExpression(_) => "RegularExpression",
        Bson::JavaScriptCode(_) => "JavaScriptCode",
        Bson::JavaScriptCodeWithScope(_) => "JavaScriptCodeWithScope",
        Bson::Int32(_) => "Int32",
        Bson::Timestamp(_) => "Timestamp",
        Bson::Int64(_) => "Int64",
        Bson::Decimal128(_) => "Decimal128",
        Bson::MinKey => "MinKey",
        Bson::MaxKey => "MaxKey",
        Bson::DbPointer(_) => "DbPointer",
        Bson::Symbol(_) => "Symbol",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn test_flatten_simple_document() {
        let doc = doc! { "name": "John", "age": 30 };
        let flat = flatten_document(&doc);
        assert_eq!(flat.get("name"), Some(&"John".to_string()));
        assert_eq!(flat.get("age"), Some(&"30".to_string()));
    }

    #[test]
    fn test_flatten_nested_document() {
        let doc = doc! { "user": { "name": "John", "address": { "city": "NYC" } } };
        let flat = flatten_document(&doc);
        assert_eq!(flat.get("user.name"), Some(&"John".to_string()));
        assert_eq!(flat.get("user.address.city"), Some(&"NYC".to_string()));
    }

    #[test]
    fn test_unflatten_simple() {
        let mut row = HashMap::new();
        row.insert("name".to_string(), "John".to_string());
        row.insert("age".to_string(), "30".to_string());

        let doc = unflatten_row(&row);
        assert_eq!(doc.get_str("name"), Ok("John"));
        assert_eq!(doc.get_i32("age"), Ok(30));
    }

    #[test]
    fn test_unflatten_nested() {
        let mut row = HashMap::new();
        row.insert("user.name".to_string(), "John".to_string());
        row.insert("user.address.city".to_string(), "NYC".to_string());

        let doc = unflatten_row(&row);
        let user = doc.get_document("user").unwrap();
        assert_eq!(user.get_str("name"), Ok("John"));
        let address = user.get_document("address").unwrap();
        assert_eq!(address.get_str("city"), Ok("NYC"));
    }
}
