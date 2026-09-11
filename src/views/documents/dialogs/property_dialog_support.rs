use mongodb::bson::{Bson, DateTime};

use crate::bson::{PathSegment, bson_value_for_edit, document_to_json_string};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PropertyActionKind {
    EditValue,
    AddField,
    RenameField,
    RemoveField,
    AddElement,
    RemoveMatchingValues,
}

/// Apply one field edit using typed path segments, including literal dotted keys.
pub(super) fn apply_property_edit(
    document: &mut mongodb::bson::Document,
    path: &[PathSegment],
    action: PropertyActionKind,
    field: &str,
    value: Bson,
) -> Result<(), String> {
    use crate::bson::{get_bson_at_path, set_bson_at_path};
    if matches!(path.first(), Some(PathSegment::Key(key)) if key == "_id") {
        return Err("The document _id cannot be changed.".into());
    }
    let mut target = path.to_vec();
    let replacement = match action {
        PropertyActionKind::EditValue => value,
        PropertyActionKind::AddElement | PropertyActionKind::RemoveMatchingValues => {
            if matches!(target.last(), Some(PathSegment::Index(_))) {
                target.pop();
            }
            let Some(Bson::Array(mut values)) = get_bson_at_path(document, &target).cloned() else {
                return Err("Array is no longer available.".into());
            };
            if action == PropertyActionKind::AddElement {
                values.push(value);
            } else {
                values.retain(|item| item != &value);
            }
            Bson::Array(values)
        }
        PropertyActionKind::AddField
        | PropertyActionKind::RenameField
        | PropertyActionKind::RemoveField => {
            let old_key = match target.last() {
                Some(PathSegment::Key(key)) => Some(key.clone()),
                _ => None,
            };
            if action != PropertyActionKind::AddField
                || !matches!(get_bson_at_path(document, &target), Some(Bson::Document(_)))
            {
                target.pop();
            }
            let mut parent = if target.is_empty() {
                document.clone()
            } else if let Some(Bson::Document(parent)) = get_bson_at_path(document, &target) {
                parent.clone()
            } else {
                return Err("Parent document is no longer available.".into());
            };
            if action != PropertyActionKind::RemoveField {
                if field.is_empty() {
                    return Err("Field name is required.".into());
                }
                if target.is_empty() && field == "_id" {
                    return Err("The document _id cannot be changed.".into());
                }
                if parent.contains_key(field)
                    && (action == PropertyActionKind::AddField || old_key.as_deref() != Some(field))
                {
                    return Err("A field with this name already exists.".into());
                }
            }
            match action {
                PropertyActionKind::AddField => {
                    parent.insert(field, value);
                }
                PropertyActionKind::RenameField => {
                    let value = parent
                        .remove(old_key.as_deref().ok_or("Select a field to rename.")?)
                        .ok_or("Field is no longer available.")?;
                    parent.insert(field, value);
                }
                PropertyActionKind::RemoveField => {
                    parent.remove(old_key.as_deref().ok_or("Select a field to remove.")?);
                }
                _ => unreachable!(),
            }
            Bson::Document(parent)
        }
    };
    if target.is_empty() {
        if let Bson::Document(updated) = replacement {
            *document = updated;
            return Ok(());
        }
    } else if set_bson_at_path(document, &target, replacement) {
        return Ok(());
    }
    Err("Field is no longer available.".into())
}

#[cfg(test)]
mod staged_edit_tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn edits_preserve_literal_keys_types_and_existing_fields() {
        let mut document = doc! { "_id": 1, "a.b": { "count": Bson::Int64(i64::MAX) }, "a": { "b": 8 }, "values": [1, 2, 1] };
        let path = [PathSegment::Key("a.b".into()), PathSegment::Key("count".into())];
        apply_property_edit(
            &mut document,
            &path,
            PropertyActionKind::RenameField,
            "total",
            Bson::Null,
        )
        .unwrap();
        assert_eq!(
            document.get_document("a.b").unwrap().get("total"),
            Some(&Bson::Int64(i64::MAX))
        );
        assert_eq!(document.get_document("a").unwrap().get_i32("b").unwrap(), 8);
        assert!(
            apply_property_edit(&mut document, &[], PropertyActionKind::AddField, "a", Bson::Null)
                .is_err()
        );
        assert!(
            apply_property_edit(
                &mut document,
                &[PathSegment::Key("_id".into())],
                PropertyActionKind::EditValue,
                "",
                Bson::Int32(2)
            )
            .is_err()
        );
        apply_property_edit(
            &mut document,
            &[PathSegment::Key("values".into())],
            PropertyActionKind::RemoveMatchingValues,
            "",
            Bson::Int32(1),
        )
        .unwrap();
        assert_eq!(document.get_array("values").unwrap(), &vec![Bson::Int32(2)]);
    }
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
    ExtendedJson,
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
            ValueType::ExtendedJson => "Extended JSON",
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
            ValueType::ExtendedJson => "BSON value in Extended JSON",
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
            _ => ValueType::ExtendedJson,
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
        Bson::Document(doc) => document_to_json_string(doc),
        Bson::Array(arr) => {
            let value = Bson::Array(arr.clone()).into_canonical_extjson();
            serde_json::to_string_pretty(&value).expect("Extended JSON is serializable")
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
