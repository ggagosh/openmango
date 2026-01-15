//! Stable document identity helpers.

use mongodb::bson::{Bson, Document};

/// Stable key derived from a document's `_id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocumentKey(String);

impl DocumentKey {
    /// Build a key from a BSON `_id` value.
    pub fn from_id(id: &Bson) -> Self {
        let ext = id.clone().into_relaxed_extjson();
        let repr = serde_json::to_string(&ext).unwrap_or_else(|_| format!("{id:?}"));
        Self(repr)
    }

    /// Build a key from a document, falling back to the document index.
    pub fn from_document(doc: &Document, fallback_index: usize) -> Self {
        if let Some(id) = doc.get("_id") {
            Self::from_id(id)
        } else {
            Self(format!("index:{fallback_index}"))
        }
    }

    /// Return the key as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DocumentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
