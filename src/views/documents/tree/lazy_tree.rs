//! Lazy tree building for virtualized document rendering.
//!
//! This module provides a lightweight representation of visible tree rows
//! for use with `uniform_list`, avoiding the overhead of building full
//! TreeItem structures for all nodes upfront.

use std::collections::HashSet;

use mongodb::bson::{Bson, Document};

use crate::bson::{DocumentKey, PathSegment, doc_root_id, path_to_id};
use crate::state::SessionDocument;

/// A lightweight representation of a visible row in the tree.
///
/// Unlike TreeItem which builds the full tree structure upfront,
/// VisibleRow only stores the minimal data needed to render a single row,
/// with document data accessed on-demand during rendering.
#[derive(Clone, Debug)]
pub struct VisibleRow {
    /// Unique identifier for this node (used for expand/collapse tracking)
    pub node_id: String,
    /// Index into the documents array
    pub doc_index: usize,
    /// Nesting depth (0 = document root, 1 = top-level field, etc.)
    pub depth: usize,
    /// Path from document root to this node
    pub path: Vec<PathSegment>,
    /// Whether this node can be expanded (is a document/array)
    pub is_folder: bool,
    /// Whether this node is currently expanded
    pub is_expanded: bool,
    /// Whether this is a document root node
    pub is_document_root: bool,
    /// The key label for this row (field name or array index)
    pub key_label: String,
}

/// Collect all expandable node IDs (documents, sub-documents, arrays) from a set of documents.
/// Used by "expand all" to populate the expanded nodes set.
pub fn collect_all_expandable_nodes(documents: &[SessionDocument]) -> HashSet<String> {
    let mut nodes = HashSet::new();
    for item in documents {
        let root_id = doc_root_id(&item.key);
        if !item.doc.is_empty() {
            nodes.insert(root_id);
        }
        collect_expandable_in_doc(&mut nodes, &item.key, &item.doc, &[]);
    }
    nodes
}

fn collect_expandable_in_doc(
    nodes: &mut HashSet<String>,
    doc_key: &DocumentKey,
    doc: &Document,
    parent_path: &[PathSegment],
) {
    for (key, value) in doc.iter() {
        let mut path = parent_path.to_vec();
        path.push(PathSegment::Key(key.clone()));
        collect_expandable_bson(nodes, doc_key, &path, value);
    }
}

fn collect_expandable_bson(
    nodes: &mut HashSet<String>,
    doc_key: &DocumentKey,
    path: &[PathSegment],
    value: &Bson,
) {
    match value {
        Bson::Document(inner) => {
            nodes.insert(path_to_id(doc_key, path));
            for (key, child) in inner.iter() {
                let mut child_path = path.to_vec();
                child_path.push(PathSegment::Key(key.clone()));
                collect_expandable_bson(nodes, doc_key, &child_path, child);
            }
        }
        Bson::Array(arr) => {
            nodes.insert(path_to_id(doc_key, path));
            for (idx, child) in arr.iter().enumerate() {
                let mut child_path = path.to_vec();
                child_path.push(PathSegment::Index(idx));
                collect_expandable_bson(nodes, doc_key, &child_path, child);
            }
        }
        _ => {}
    }
}

/// Build the list of visible rows based on expanded state.
///
/// This walks through documents and only descends into expanded nodes,
/// creating a flat list of VisibleRow that maps directly to what
/// uniform_list will render.
pub fn build_visible_rows(
    documents: &[SessionDocument],
    expanded_nodes: &HashSet<String>,
) -> Vec<VisibleRow> {
    let mut rows = Vec::new();

    for (doc_index, item) in documents.iter().enumerate() {
        let doc_key = &item.key;
        let doc = &item.doc;

        // Add document root
        let root_id = doc_root_id(doc_key);
        let is_expanded = expanded_nodes.contains(&root_id);

        let id_preview = doc
            .get("_id")
            .map(|value| crate::bson::bson_value_preview(value, 64))
            .unwrap_or_else(|| "doc".to_string());
        let key_label = format!("_id: {}", id_preview);

        rows.push(VisibleRow {
            node_id: root_id.clone(),
            doc_index,
            depth: 0,
            path: Vec::new(),
            is_folder: !doc.is_empty(),
            is_expanded,
            is_document_root: true,
            key_label,
        });

        // If expanded, add children
        if is_expanded {
            build_document_children(&mut rows, doc_key, doc, doc_index, 1, expanded_nodes);
        }
    }

    rows
}

/// Recursively build visible rows for an expanded document's fields.
fn build_document_children(
    rows: &mut Vec<VisibleRow>,
    doc_key: &DocumentKey,
    doc: &Document,
    doc_index: usize,
    depth: usize,
    expanded_nodes: &HashSet<String>,
) {
    for (key, value) in doc.iter() {
        let path = vec![PathSegment::Key(key.clone())];
        build_bson_row(rows, doc_key, key.clone(), path, value, doc_index, depth, expanded_nodes);
    }
}

/// Build a visible row for a BSON value and its children if expanded.
#[allow(clippy::too_many_arguments)]
fn build_bson_row(
    rows: &mut Vec<VisibleRow>,
    doc_key: &DocumentKey,
    key_label: String,
    path: Vec<PathSegment>,
    value: &Bson,
    doc_index: usize,
    depth: usize,
    expanded_nodes: &HashSet<String>,
) {
    let node_id = path_to_id(doc_key, &path);
    let is_folder = matches!(value, Bson::Document(_) | Bson::Array(_));
    let is_expanded = is_folder && expanded_nodes.contains(&node_id);

    rows.push(VisibleRow {
        node_id: node_id.clone(),
        doc_index,
        depth,
        path: path.clone(),
        is_folder,
        is_expanded,
        is_document_root: false,
        key_label,
    });

    // If expanded, add children
    if is_expanded {
        match value {
            Bson::Document(doc) => {
                for (key, child_value) in doc.iter() {
                    let mut child_path = path.clone();
                    child_path.push(PathSegment::Key(key.clone()));
                    build_bson_row(
                        rows,
                        doc_key,
                        key.clone(),
                        child_path,
                        child_value,
                        doc_index,
                        depth + 1,
                        expanded_nodes,
                    );
                }
            }
            Bson::Array(arr) => {
                for (idx, child_value) in arr.iter().enumerate() {
                    let mut child_path = path.clone();
                    child_path.push(PathSegment::Index(idx));
                    build_bson_row(
                        rows,
                        doc_key,
                        format!("[{}]", idx),
                        child_path,
                        child_value,
                        doc_index,
                        depth + 1,
                        expanded_nodes,
                    );
                }
            }
            _ => {}
        }
    }
}
