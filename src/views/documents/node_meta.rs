//! Node metadata for the document tree view.

use gpui::Hsla;
use mongodb::bson::Bson;

use crate::bson::{DocumentKey, PathSegment};

/// Metadata for a node in the document tree.
#[derive(Clone)]
pub struct NodeMeta {
    pub key_label: String,
    pub value_label: String,
    pub value_color: Hsla,
    pub type_label: String,
    pub is_folder: bool,
    pub is_editable: bool,
    pub is_dirty: bool,
    pub doc_key: DocumentKey,
    pub path: Vec<PathSegment>,
    pub value: Option<Bson>,
}
