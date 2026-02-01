//! Lazy row rendering for virtualized document tree.
//!
//! This module provides on-demand row rendering that computes metadata
//! only for visible rows, avoiding the overhead of pre-computing
//! metadata for all 20,000+ nodes.

use gpui::*;
use gpui_component::{Icon, IconName, Sizable as _};
use mongodb::bson::Bson;

use crate::bson::{bson_type_label, bson_value_preview, get_bson_at_path};
use crate::state::SessionDocument;
use crate::theme::{colors, spacing};
use crate::views::documents::CollectionView;

use super::lazy_tree::VisibleRow;

/// Metadata computed on-demand for a single row.
pub struct LazyRowMeta {
    pub key_label: String,
    pub value_label: String,
    pub value_color: Rgba,
    pub type_label: String,
}

/// Compute metadata for a row on-demand.
pub fn compute_row_meta(row: &VisibleRow, documents: &[SessionDocument]) -> LazyRowMeta {
    let doc = &documents[row.doc_index].doc;

    if row.is_document_root {
        // Document root node
        let value_label = format!("{{{} fields}}", doc.len());
        LazyRowMeta {
            key_label: row.key_label.clone(),
            value_label,
            value_color: colors::text_muted(),
            type_label: "Document".to_string(),
        }
    } else {
        // Get the value at this path
        let value = get_bson_at_path(doc, &row.path);

        match value {
            Some(value) => {
                let value_label = bson_value_preview(value, 120);
                let type_label = bson_type_label(value).to_string();
                let value_color = bson_value_color(value);

                LazyRowMeta {
                    key_label: row.key_label.clone(),
                    value_label,
                    value_color,
                    type_label,
                }
            }
            None => {
                // Fallback for missing value (shouldn't happen in normal use)
                LazyRowMeta {
                    key_label: row.key_label.clone(),
                    value_label: "—".to_string(),
                    value_color: colors::text_muted(),
                    type_label: "Unknown".to_string(),
                }
            }
        }
    }
}

/// Get the display color for a BSON value.
fn bson_value_color(value: &Bson) -> Rgba {
    match value {
        Bson::String(_) | Bson::Symbol(_) => colors::syntax_string(),
        Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_) => {
            colors::syntax_number()
        }
        Bson::Boolean(_) => colors::syntax_boolean(),
        Bson::Null | Bson::Undefined => colors::syntax_null(),
        Bson::ObjectId(_) => colors::syntax_object_id(),
        Bson::DateTime(_) | Bson::Timestamp(_) => colors::syntax_date(),
        Bson::RegularExpression(_) | Bson::JavaScriptCode(_) | Bson::JavaScriptCodeWithScope(_) => {
            colors::syntax_comment()
        }
        Bson::Document(_) | Bson::Array(_) | Bson::Binary(_) => colors::text_muted(),
        _ => colors::text_primary(),
    }
}

/// Render a single readonly row for aggregation results.
///
/// This is a lightweight version of render_readonly_tree_row that works
/// with VisibleRow and computes metadata on-demand.
pub fn render_lazy_readonly_row(
    ix: usize,
    row: &VisibleRow,
    meta: &LazyRowMeta,
    _selected: bool,
    view_entity: Entity<CollectionView>,
) -> AnyElement {
    let node_id = row.node_id.clone();
    let depth = row.depth;
    let is_folder = row.is_folder;
    let is_expanded = row.is_expanded;

    let key_label = meta.key_label.clone();
    let value_label = meta.value_label.clone();
    let value_color = meta.value_color;
    let type_label = meta.type_label.clone();

    let leading = if is_folder {
        let toggle_node_id = node_id.clone();
        let toggle_view = view_entity.clone();
        div()
            .id(("agg-row-chevron", ix))
            .w(px(14.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                if event.click_count == 1 {
                    cx.stop_propagation();
                    toggle_view.update(cx, |this, cx| {
                        if this.aggregation_results_expanded_nodes.contains(&toggle_node_id) {
                            this.aggregation_results_expanded_nodes.remove(&toggle_node_id);
                        } else {
                            this.aggregation_results_expanded_nodes.insert(toggle_node_id.clone());
                        }
                        cx.notify();
                    });
                }
            })
            .child(
                Icon::new(if is_expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                    .xsmall()
                    .text_color(colors::text_muted()),
            )
            .into_any_element()
    } else {
        div().w(px(14.0)).into_any_element()
    };

    div()
        .id(("agg-result-row", ix))
        .flex()
        .items_center()
        .w_full()
        .px(spacing::lg())
        .py(spacing::xs())
        .hover(|s| s.bg(colors::list_hover()))
        .on_mouse_down(MouseButton::Left, {
            let node_id = node_id.clone();
            let row_view = view_entity.clone();
            move |event, _window, cx| {
                if event.click_count == 2 && is_folder {
                    row_view.update(cx, |this, cx| {
                        if this.aggregation_results_expanded_nodes.contains(&node_id) {
                            this.aggregation_results_expanded_nodes.remove(&node_id);
                        } else {
                            this.aggregation_results_expanded_nodes.insert(node_id.clone());
                        }
                        cx.notify();
                    });
                }
            }
        })
        .child(render_key_column(depth, leading, &key_label))
        .child(render_value_column(&value_label, value_color))
        .child(
            div()
                .w(px(120.0))
                .text_sm()
                .text_color(colors::text_muted())
                .overflow_hidden()
                .text_ellipsis()
                .child(type_label),
        )
        .into_any_element()
}

fn render_key_column(depth: usize, leading: AnyElement, key_label: &str) -> impl IntoElement {
    let key_label = key_label.to_string();

    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .flex_1()
        .min_w(px(0.0))
        .overflow_hidden()
        .pl(px(14.0 * depth as f32))
        .child(leading)
        .child(
            div()
                .text_sm()
                .text_color(colors::syntax_key())
                .overflow_hidden()
                .text_ellipsis()
                .child(key_label),
        )
}

fn render_value_column(value_label: &str, value_color: Rgba) -> impl IntoElement {
    div().flex_1().min_w(px(0.0)).overflow_hidden().child(
        div()
            .text_sm()
            .text_color(value_color)
            .overflow_hidden()
            .text_ellipsis()
            .child(value_label.to_string()),
    )
}
