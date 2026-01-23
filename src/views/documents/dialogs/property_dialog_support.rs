use mongodb::bson::{Bson, DateTime};

use crate::bson::{PathSegment, bson_value_for_edit, document_to_relaxed_extjson_string};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PropertyActionKind {
    EditValue,
    AddField,
    RenameField,
    RemoveField,
    AddElement,
    RemoveMatchingValues,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdateScope {
    CurrentDocument,
    MatchQuery,
    AllDocuments,
}

impl UpdateScope {
    pub(super) fn label(self) -> &'static str {
        match self {
            UpdateScope::CurrentDocument => "Current document only",
            UpdateScope::MatchQuery => "Match current query",
            UpdateScope::AllDocuments => "All documents",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ValueType {
    Document,
    Array,
    ObjectId,
    String,
    Bool,
    Int32,
    Int64,
    Double,
    Date,
    Null,
}

impl ValueType {
    pub(super) fn label(self) -> &'static str {
        match self {
            ValueType::Document => "Document",
            ValueType::Array => "Array",
            ValueType::ObjectId => "ObjectId",
            ValueType::String => "String",
            ValueType::Bool => "Bool",
            ValueType::Int32 => "Int32",
            ValueType::Int64 => "Int64",
            ValueType::Double => "Double",
            ValueType::Date => "Date",
            ValueType::Null => "Null",
        }
    }

    pub(super) fn placeholder(self) -> &'static str {
        match self {
            ValueType::Document => "{ }",
            ValueType::Array => "[ ]",
            ValueType::ObjectId => "ObjectId hex",
            ValueType::String => "Value",
            ValueType::Bool => "true / false",
            ValueType::Int32 => "0",
            ValueType::Int64 => "0",
            ValueType::Double => "0.0",
            ValueType::Date => "RFC3339 timestamp",
            ValueType::Null => "",
        }
    }

    pub(super) fn from_bson(value: &Bson) -> Self {
        match value {
            Bson::Document(_) => ValueType::Document,
            Bson::Array(_) => ValueType::Array,
            Bson::ObjectId(_) => ValueType::ObjectId,
            Bson::String(_) => ValueType::String,
            Bson::Boolean(_) => ValueType::Bool,
            Bson::Int32(_) => ValueType::Int32,
            Bson::Int64(_) => ValueType::Int64,
            Bson::Double(_) => ValueType::Double,
            Bson::DateTime(_) => ValueType::Date,
            Bson::Null => ValueType::Null,
            _ => ValueType::String,
        }
    }
}

pub(super) fn parent_path(path: &[PathSegment]) -> Vec<PathSegment> {
    if path.is_empty() {
        return Vec::new();
    }
    path[..path.len() - 1].to_vec()
}

pub(super) fn display_segment(segment: Option<&PathSegment>) -> String {
    match segment {
        Some(PathSegment::Key(key)) => key.to_string(),
        Some(PathSegment::Index(index)) => format!("[{index}]"),
        None => "".to_string(),
    }
}

pub(super) fn display_path(path: &[PathSegment]) -> String {
    let mut out = String::new();
    for segment in path {
        match segment {
            PathSegment::Key(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            PathSegment::Index(index) => {
                out.push('[');
                out.push_str(&index.to_string());
                out.push(']');
            }
        }
    }
    out
}

pub(super) fn dot_path(path: &[PathSegment]) -> String {
    let mut out = String::new();
    for segment in path {
        match segment {
            PathSegment::Key(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            PathSegment::Index(index) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(&index.to_string());
            }
        }
    }
    out
}

pub(super) fn format_bson_for_input(value: &Bson) -> String {
    match value {
        Bson::Document(doc) => document_to_relaxed_extjson_string(doc),
        Bson::Array(arr) => {
            let value = Bson::Array(arr.clone()).into_relaxed_extjson();
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| format!("{value:?}"))
        }
        _ => bson_value_for_edit(value),
    }
}

pub(super) fn parse_bool(trimmed: &str) -> Result<Bson, String> {
    match trimmed.to_ascii_lowercase().as_str() {
        "true" => Ok(Bson::Boolean(true)),
        "false" => Ok(Bson::Boolean(false)),
        _ => Err("Expected true/false".to_string()),
    }
}

pub(super) fn parse_i32(trimmed: &str) -> Result<Bson, String> {
    trimmed.parse::<i32>().map(Bson::Int32).map_err(|_| "Expected int32".to_string())
}

pub(super) fn parse_i64(trimmed: &str) -> Result<Bson, String> {
    trimmed.parse::<i64>().map(Bson::Int64).map_err(|_| "Expected int64".to_string())
}

pub(super) fn parse_f64(trimmed: &str) -> Result<Bson, String> {
    trimmed.parse::<f64>().map(Bson::Double).map_err(|_| "Expected number".to_string())
}

pub(super) fn parse_date(trimmed: &str) -> Result<Bson, String> {
    DateTime::parse_rfc3339_str(trimmed)
        .map(Bson::DateTime)
        .map_err(|_| "Expected RFC3339 date".to_string())
}
