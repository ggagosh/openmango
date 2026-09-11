//! BSON value formatting utilities for display and editing.

use mongodb::bson::Bson;

/// Strict, type-preserving Extended JSON for document editing and clipboard round trips.
pub fn document_to_json_string(document: &mongodb::bson::Document) -> String {
    serde_json::to_string_pretty(&Bson::Document(document.clone()).into_canonical_extjson())
        .expect("Extended JSON is serializable")
}

/// Copy scalar text naturally; use Extended JSON for structured and specialized BSON values.
pub fn format_bson_for_clipboard(value: &Bson) -> String {
    match value {
        Bson::String(_)
        | Bson::ObjectId(_)
        | Bson::DateTime(_)
        | Bson::Int32(_)
        | Bson::Int64(_)
        | Bson::Double(_)
        | Bson::Boolean(_)
        | Bson::Null => bson_value_for_edit(value),
        _ => serde_json::to_string_pretty(&value.clone().into_canonical_extjson())
            .expect("Extended JSON is serializable"),
    }
}

/// Get a human-readable type label for a BSON value.
pub fn bson_type_label(value: &Bson) -> &'static str {
    match value {
        Bson::Document(_) => "Document",
        Bson::Array(_) => "Array",
        Bson::String(_) => "String",
        Bson::Int32(_) => "Int32",
        Bson::Int64(_) => "Int64",
        Bson::Double(_) => "Double",
        Bson::Boolean(_) => "Bool",
        Bson::Null => "Null",
        Bson::ObjectId(_) => "ObjectId",
        Bson::DateTime(_) => "Date",
        Bson::Binary(_) => "Binary",
        Bson::Decimal128(_) => "Decimal128",
        _ => "Value",
    }
}

/// Get a preview string for a BSON value, truncated to max_len.
pub fn bson_value_preview(value: &Bson, max_len: usize) -> String {
    match value {
        Bson::String(s) => {
            let sanitized = sanitize_for_preview(s);
            truncate_for_preview(&sanitized, max_len)
        }
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(n) => n.to_string(),
        Bson::Boolean(b) => b.to_string(),
        Bson::Null => "null".to_string(),
        Bson::ObjectId(oid) => oid.to_hex(),
        Bson::DateTime(dt) => (*dt).try_to_rfc3339_string().unwrap_or_else(|_| format!("{dt:?}")),
        Bson::Document(doc) => format!("{{{} fields}}", doc.len()),
        Bson::Array(arr) => format!("[{} items]", arr.len()),
        other => truncate_for_preview(&format!("{other:?}"), max_len),
    }
}

/// Get a BSON value formatted for editing in an input field.
pub fn bson_value_for_edit(value: &Bson) -> String {
    match value {
        Bson::String(s) => s.clone(),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(n) => n.to_string(),
        Bson::Boolean(b) => b.to_string(),
        Bson::Null => "null".to_string(),
        Bson::ObjectId(oid) => oid.to_hex(),
        Bson::DateTime(dt) => (*dt).try_to_rfc3339_string().unwrap_or_else(|_| format!("{dt:?}")),
        other => format!("{other:?}"),
    }
}

/// Truncate a string for preview display, adding ellipsis if needed.
pub fn truncate_for_preview(input: &str, max_len: usize) -> String {
    if input.chars().count() <= max_len {
        return input.to_string();
    }

    let mut output = String::new();
    for (idx, ch) in input.chars().enumerate() {
        if idx >= max_len.saturating_sub(3) {
            break;
        }
        output.push(ch);
    }
    output.push_str("...");
    output
}

fn sanitize_for_preview(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch.is_control() => output.push('?'),
            _ => output.push(ch),
        }
    }
    output
}

#[cfg(test)]
mod json_tests {
    use super::*;
    use mongodb::bson::{Decimal128, doc};
    #[test]
    fn document_json_preserves_numeric_types_and_string_whitespace() {
        let document = doc! { "small_long": Bson::Int64(1), "large_long": Bson::Int64(i64::MAX), "decimal": Bson::Decimal128("12.30".parse::<Decimal128>().unwrap()), "text": "  value  " };
        let text = document_to_json_string(&document);
        assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
        assert_eq!(crate::bson::parse_document_from_json(&text).unwrap(), document);
    }
}
