//! BSON value formatting utilities for display and editing.

use mongodb::bson::Bson;

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
        Bson::String(s) => truncate_for_preview(s, max_len),
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
