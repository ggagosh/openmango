//! BSON parsing utilities for converting between formats.

use mongodb::bson::{self, Bson, DateTime, Document, oid::ObjectId};
use serde_json::Value;

/// Parse an edited string value back into BSON, matching the original type.
pub fn parse_edited_value(original: &Bson, input: &str) -> Result<Bson, String> {
    let trimmed = input.trim();
    match original {
        Bson::String(_) => Ok(Bson::String(trimmed.to_string())),
        Bson::Int32(_) => {
            trimmed.parse::<i32>().map(Bson::Int32).map_err(|_| "Expected int32".to_string())
        }
        Bson::Int64(_) => {
            trimmed.parse::<i64>().map(Bson::Int64).map_err(|_| "Expected int64".to_string())
        }
        Bson::Double(_) => {
            trimmed.parse::<f64>().map(Bson::Double).map_err(|_| "Expected number".to_string())
        }
        Bson::Boolean(_) => match trimmed.to_ascii_lowercase().as_str() {
            "true" => Ok(Bson::Boolean(true)),
            "false" => Ok(Bson::Boolean(false)),
            _ => Err("Expected true/false".to_string()),
        },
        Bson::Null => {
            if trimmed.eq_ignore_ascii_case("null") {
                Ok(Bson::Null)
            } else {
                Err("Expected null".to_string())
            }
        }
        Bson::ObjectId(_) => ObjectId::parse_str(trimmed)
            .map(Bson::ObjectId)
            .map_err(|_| "Expected ObjectId hex".to_string()),
        Bson::DateTime(_) => DateTime::parse_rfc3339_str(trimmed)
            .map(Bson::DateTime)
            .map_err(|_| "Expected RFC3339 date".to_string()),
        _ => Err("Unsupported type".to_string()),
    }
}

/// Convert a BSON document to a pretty-printed relaxed Extended JSON string.
pub fn document_to_relaxed_extjson_string(doc: &Document) -> String {
    let value = bson::Bson::Document(doc.clone()).into_relaxed_extjson();
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| format!("{doc:?}"))
}

/// Parse a JSON string into a BSON document.
pub fn parse_document_from_json(input: &str) -> Result<Document, String> {
    let value: Value = serde_json::from_str(input).map_err(|e| e.to_string())?;
    let bson = bson::Bson::try_from(value).map_err(|e| e.to_string())?;
    match bson {
        bson::Bson::Document(doc) => Ok(doc),
        _ => Err("Root JSON must be a document".to_string()),
    }
}
