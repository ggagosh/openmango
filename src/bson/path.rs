//! BSON path navigation and manipulation utilities.

use mongodb::bson::{Bson, Document};

use super::DocumentKey;

/// Represents a segment in a path through a BSON document.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathSegment {
    /// A key in a document
    Key(String),
    /// An index in an array
    Index(usize),
}

/// Generate the root ID for a document at the given index.
pub fn doc_root_id(doc_key: &DocumentKey) -> String {
    format!("doc:{}", escape_key(doc_key.as_str()))
}

/// Convert a document index and path to a unique string ID.
pub fn path_to_id(doc_key: &DocumentKey, path: &[PathSegment]) -> String {
    let mut id = doc_root_id(doc_key);
    for segment in path {
        id.push('/');
        match segment {
            PathSegment::Key(key) => id.push_str(&escape_key(key)),
            PathSegment::Index(idx) => id.push_str(&format!("[{idx}]")),
        }
    }
    id
}

/// Escape a key for use in path IDs.
pub fn escape_key(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Check if a BSON value at the given path is editable inline.
pub fn is_editable_value(value: &Bson, path: &[PathSegment]) -> bool {
    // _id field is not editable
    if matches!(path.last(), Some(PathSegment::Key(key)) if key == "_id") {
        return false;
    }

    matches!(
        value,
        Bson::String(_)
            | Bson::Int32(_)
            | Bson::Int64(_)
            | Bson::Double(_)
            | Bson::Boolean(_)
            | Bson::Null
            | Bson::ObjectId(_)
            | Bson::DateTime(_)
    )
}

/// Get a reference to a BSON value at the given path within a document.
pub fn get_bson_at_path<'a>(doc: &'a Document, path: &[PathSegment]) -> Option<&'a Bson> {
    if path.is_empty() {
        return None;
    }

    match &path[0] {
        PathSegment::Key(key) => {
            doc.get(key).and_then(|value| get_bson_in_value(value, &path[1..]))
        }
        PathSegment::Index(_) => None,
    }
}

fn get_bson_in_value<'a>(value: &'a Bson, path: &[PathSegment]) -> Option<&'a Bson> {
    if path.is_empty() {
        return Some(value);
    }

    match (&path[0], value) {
        (PathSegment::Key(key), Bson::Document(doc)) => {
            doc.get(key).and_then(|inner| get_bson_in_value(inner, &path[1..]))
        }
        (PathSegment::Index(idx), Bson::Array(arr)) => {
            arr.get(*idx).and_then(|inner| get_bson_in_value(inner, &path[1..]))
        }
        _ => None,
    }
}

/// Set a BSON value at the given path within a document.
/// Returns true if the value was successfully set.
pub fn set_bson_at_path(doc: &mut Document, path: &[PathSegment], new_value: Bson) -> bool {
    if path.is_empty() {
        return false;
    }

    match &path[0] {
        PathSegment::Key(key) => {
            if path.len() == 1 {
                doc.insert(key.clone(), new_value);
                return true;
            }

            if let Some(value) = doc.get_mut(key) {
                return set_bson_in_value(value, &path[1..], new_value);
            }
        }
        PathSegment::Index(_) => return false,
    }

    false
}

fn set_bson_in_value(value: &mut Bson, path: &[PathSegment], new_value: Bson) -> bool {
    if path.is_empty() {
        *value = new_value;
        return true;
    }

    match (&path[0], value) {
        (PathSegment::Key(key), Bson::Document(doc)) => {
            if path.len() == 1 {
                doc.insert(key.clone(), new_value);
                return true;
            }

            if let Some(next) = doc.get_mut(key) {
                return set_bson_in_value(next, &path[1..], new_value);
            }
        }
        (PathSegment::Index(index), Bson::Array(arr)) => {
            if *index >= arr.len() {
                return false;
            }
            if path.len() == 1 {
                arr[*index] = new_value;
                return true;
            }
            return set_bson_in_value(&mut arr[*index], &path[1..], new_value);
        }
        _ => {}
    }

    false
}
