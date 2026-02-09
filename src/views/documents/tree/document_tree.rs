//! Document tree building utilities.

use std::collections::{HashMap, HashSet};

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::tree::TreeItem;
use mongodb::bson::{Bson, Document};

use crate::bson::{
    DocumentKey, PathSegment, bson_type_label, bson_value_preview, doc_root_id, get_bson_at_path,
    is_editable_value, path_to_id,
};
use crate::state::SessionDocument;
use crate::theme::colors;

use super::super::node_meta::NodeMeta;

/// Build the document tree from a list of documents.
///
/// Returns:
/// - Vec<TreeItem>: The tree items for rendering
/// - HashMap<String, NodeMeta>: Metadata for each node
/// - Vec<String>: The flattened order of visible nodes
pub fn build_documents_tree(
    documents: &[SessionDocument],
    drafts: &HashMap<DocumentKey, Document>,
    expanded_nodes: &HashSet<String>,
    cx: &App,
) -> (Vec<TreeItem>, HashMap<String, NodeMeta>, Vec<String>) {
    let mut items = Vec::new();
    let mut meta = HashMap::new();

    for item in documents {
        let doc_key = item.key.clone();
        let original = &item.doc;
        let doc = drafts.get(&doc_key).unwrap_or(original);
        let root_id = doc_root_id(&doc_key);
        let id_preview = doc
            .get("_id")
            .map(|value| bson_value_preview(value, 64))
            .unwrap_or_else(|| "doc".to_string());
        let key_label = format!("_id: {}", id_preview);
        let value_label = format!("{{{} fields}}", doc.len());
        let is_doc_dirty = drafts.get(&doc_key).is_some_and(|draft| draft != original);

        let root_meta = NodeMeta {
            key_label: key_label.clone(),
            value_label: value_label.clone(),
            value_color: cx.theme().muted_foreground,
            type_label: "Document".to_string(),
            is_folder: !doc.is_empty(),
            is_editable: false,
            is_dirty: is_doc_dirty,
            doc_key: doc_key.clone(),
            path: Vec::new(),
            value: None,
        };
        meta.insert(root_id.clone(), root_meta);

        let mut root = TreeItem::new(root_id.clone(), key_label)
            .expanded(expanded_nodes.contains(&root_id))
            .disabled(true);
        let children: Vec<TreeItem> = doc
            .iter()
            .map(|(key, value)| {
                build_bson_tree_item(
                    &doc_key,
                    key.clone(),
                    vec![PathSegment::Key(key.clone())],
                    value,
                    original,
                    expanded_nodes,
                    &mut meta,
                    cx,
                )
            })
            .collect();
        root = root.children(children);
        items.push(root);
    }

    let mut order = Vec::new();
    for item in &items {
        flatten_tree_order(item, &mut order);
    }

    (items, meta, order)
}

/// Build a tree item for a BSON value.
#[allow(clippy::too_many_arguments)]
pub fn build_bson_tree_item(
    doc_key: &DocumentKey,
    key_label: String,
    path: Vec<PathSegment>,
    value: &Bson,
    original: &Document,
    expanded_nodes: &HashSet<String>,
    meta: &mut HashMap<String, NodeMeta>,
    cx: &App,
) -> TreeItem {
    let node_id = path_to_id(doc_key, &path);
    let is_folder = matches!(value, Bson::Document(_) | Bson::Array(_));
    let is_editable = is_editable_value(value, &path);
    let original_value = get_bson_at_path(original, &path);
    let is_dirty = original_value.map(|orig| orig != value).unwrap_or(true);

    let value_label = bson_value_preview(value, 120);
    let type_label = bson_type_label(value).to_string();

    let value_color = match value {
        Bson::String(_) | Bson::Symbol(_) => colors::syntax_string(cx),
        Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_) => {
            colors::syntax_number(cx)
        }
        Bson::Boolean(_) => colors::syntax_boolean(cx),
        Bson::Null | Bson::Undefined => colors::syntax_null(cx),
        Bson::ObjectId(_) => colors::syntax_object_id(cx),
        Bson::DateTime(_) | Bson::Timestamp(_) => colors::syntax_date(cx),
        Bson::RegularExpression(_) | Bson::JavaScriptCode(_) | Bson::JavaScriptCodeWithScope(_) => {
            colors::syntax_comment(cx)
        }
        Bson::Document(_) | Bson::Array(_) | Bson::Binary(_) => cx.theme().muted_foreground,
        _ => cx.theme().foreground,
    };

    meta.insert(
        node_id.clone(),
        NodeMeta {
            key_label: key_label.clone(),
            value_label,
            value_color,
            type_label,
            is_folder,
            is_editable,
            is_dirty,
            doc_key: doc_key.clone(),
            path: path.clone(),
            value: if is_editable { Some(value.clone()) } else { None },
        },
    );

    let mut item = TreeItem::new(node_id.clone(), key_label)
        .expanded(expanded_nodes.contains(&node_id))
        .disabled(true);

    match value {
        Bson::Document(doc) => {
            let children: Vec<TreeItem> = doc
                .iter()
                .map(|(key, value)| {
                    let mut child_path = path.clone();
                    child_path.push(PathSegment::Key(key.clone()));
                    build_bson_tree_item(
                        doc_key,
                        key.clone(),
                        child_path,
                        value,
                        original,
                        expanded_nodes,
                        meta,
                        cx,
                    )
                })
                .collect();
            item = item.children(children);
        }
        Bson::Array(arr) => {
            let children: Vec<TreeItem> = arr
                .iter()
                .enumerate()
                .map(|(idx, value)| {
                    let mut child_path = path.clone();
                    child_path.push(PathSegment::Index(idx));
                    build_bson_tree_item(
                        doc_key,
                        format!("[{}]", idx),
                        child_path,
                        value,
                        original,
                        expanded_nodes,
                        meta,
                        cx,
                    )
                })
                .collect();
            item = item.children(children);
        }
        _ => {}
    }

    item
}

/// Flatten the tree order for visible nodes.
pub fn flatten_tree_order(item: &TreeItem, order: &mut Vec<String>) {
    order.push(item.id.to_string());
    if item.is_expanded() {
        for child in &item.children {
            flatten_tree_order(child, order);
        }
    }
}

/// Flatten the tree order for all nodes, regardless of expanded state.
pub fn flatten_tree_order_all(item: &TreeItem, order: &mut Vec<String>) {
    order.push(item.id.to_string());
    for child in &item.children {
        flatten_tree_order_all(child, order);
    }
}
