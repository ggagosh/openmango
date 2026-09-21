//! Lazy row rendering for virtualized document tree.
//!
//! This module provides on-demand row rendering that computes metadata
//! only for visible rows, avoiding the overhead of pre-computing
//! metadata for all 20,000+ nodes.

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::*;
use mongodb::bson::Bson;

use crate::bson::{bson_type_label, bson_value_preview, get_bson_at_path, has_value_details};
use crate::state::relations::path_from_segments;
use crate::state::relations::resolve::{Reference, reference_at};
use crate::state::{AppState, SessionDocument, SessionKey};
use crate::theme::{colors, spacing};
use crate::views::documents::CollectionView;
use crate::views::documents::reference::{ReferenceLink, on_reference_mouse_down};
use crate::views::documents::table::cell_renderer::value_details_tooltip;
use gpui_kit::prelude::FluentBuilder as _;

use super::lazy_tree::VisibleRow;

/// Metadata computed on-demand for a single row.
pub struct LazyRowMeta {
    pub key_label: String,
    pub value_label: String,
    pub value_color: Hsla,
    pub type_label: String,
    /// Set when the value is an id that can be followed to the document it names.
    pub reference: Option<RowReference>,
    /// A date or binary value, kept for the hover card that shows its other readings. Cloned
    /// per visible row, the same ceiling as `cell_renderer::render_cell`.
    pub details: Option<Bson>,
}

/// An id in a pipeline's output, and where in the result it sits.
pub struct RowReference {
    pub document: crate::bson::DocumentKey,
    pub path: String,
    pub reference: Reference,
}

/// Compute metadata for a row on-demand.
pub fn compute_row_meta(row: &VisibleRow, documents: &[SessionDocument], cx: &App) -> LazyRowMeta {
    let doc = &documents[row.doc_index].doc;

    if row.is_document_root {
        // Document root node
        let value_label = format!("{{{} fields}}", doc.len());
        LazyRowMeta {
            key_label: row.key_label.clone(),
            value_label,
            value_color: cx.theme().muted_foreground,
            type_label: "Document".to_string(),
            reference: None,
            details: None,
        }
    } else {
        // Get the value at this path
        let value = get_bson_at_path(doc, &row.path);

        match value {
            Some(value) => {
                let value_label = bson_value_preview(value, 120);
                let type_label = bson_type_label(value).to_string();
                let value_color = bson_value_color(value, cx);

                let path = path_from_segments(&row.path);
                let reference = reference_at(&path, value).map(|reference| RowReference {
                    document: documents[row.doc_index].key.clone(),
                    path,
                    reference,
                });
                LazyRowMeta {
                    key_label: row.key_label.clone(),
                    value_label,
                    value_color,
                    type_label,
                    reference,
                    details: has_value_details(value).then(|| value.clone()),
                }
            }
            None => {
                // Fallback for missing value (shouldn't happen in normal use)
                LazyRowMeta {
                    key_label: row.key_label.clone(),
                    value_label: "—".to_string(),
                    value_color: cx.theme().muted_foreground,
                    type_label: "Unknown".to_string(),
                    reference: None,
                    details: None,
                }
            }
        }
    }
}

/// Get the display color for a BSON value.
fn bson_value_color(value: &Bson, cx: &App) -> Hsla {
    match value {
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
    }
}

/// Render a single readonly row for aggregation results.
///
/// Works with VisibleRow and computes metadata on demand.
pub fn render_lazy_readonly_row(
    row: &VisibleRow,
    meta: &LazyRowMeta,
    _selected: bool,
    view_entity: Entity<CollectionView>,
    // Where a followed id starts from. Passed in, not read off `view_entity`: rows are built
    // while the view is being updated, and reading it then panics.
    link_base: Option<&(Entity<AppState>, SessionKey)>,
    cx: &App,
) -> AnyElement {
    let node_id = row.node_id.clone();
    let depth = row.depth;
    let is_folder = row.is_folder;
    let is_expanded = row.is_expanded;

    let key_label = meta.key_label.clone();
    let value_label = meta.value_label.clone();
    let value_color = meta.value_color;
    let type_label = meta.type_label.clone();

    let lazy_chevron_hover = cx.theme().foreground.opacity(0.1);
    let leading = if is_folder {
        let toggle_node_id = node_id.clone();
        let toggle_view = view_entity.clone();
        div()
            .id("agg-row-chevron")
            .size(px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(crate::theme::borders::radius_sm())
            .cursor_pointer()
            .hover(|s| s.bg(lazy_chevron_hover))
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
                    .text_color(cx.theme().muted_foreground),
            )
            .into_any_element()
    } else {
        div().w(px(18.0)).into_any_element()
    };

    div()
        // Keyed by node, not position; the chevron inside is scoped by this id.
        .id((ElementId::from("agg-result-row"), node_id.clone()))
        .flex()
        .items_center()
        .w_full()
        .px(spacing::lg())
        .py(spacing::xs())
        .hover(|s| s.bg(cx.theme().list_hover))
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
        .child(render_key_column(depth, leading, &key_label, cx))
        .child(match &meta.reference {
            Some(found) => {
                // Followed, never learned from: a pipeline's output path is not a field of the
                // collection, so it has nothing to teach the graph.
                let link = link_base.map(|(state, session)| ReferenceLink {
                    state: state.clone(),
                    session: session.clone(),
                    document: found.document.clone(),
                    path: found.path.clone(),
                    reference: found.reference.clone(),
                    derived: true,
                });
                render_value_column(&value_label, value_color, link, None)
            }
            None => render_value_column(&value_label, value_color, None, meta.details.clone()),
        })
        .child(
            div()
                .w(px(120.0))
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .overflow_hidden()
                .text_ellipsis()
                .child(type_label),
        )
        .into_any_element()
}

fn render_key_column(
    depth: usize,
    leading: AnyElement,
    key_label: &str,
    cx: &App,
) -> impl IntoElement {
    let key_color = colors::syntax_key(cx);
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
                .text_color(key_color)
                .overflow_hidden()
                .text_ellipsis()
                .child(key_label),
        )
}

fn render_value_column(
    value_label: &str,
    value_color: Hsla,
    link: Option<ReferenceLink>,
    details: Option<Bson>,
) -> impl IntoElement {
    div().flex_1().min_w(px(0.0)).overflow_hidden().child(
        div()
            .id("agg-row-value")
            .text_sm()
            .text_color(value_color)
            .overflow_hidden()
            .text_ellipsis()
            .when_some(link, |value, link| {
                value
                    .cursor_pointer()
                    .hover(|style| style.underline())
                    .on_mouse_down(MouseButton::Left, on_reference_mouse_down(link))
            })
            .when_some(details, |value, details| {
                value.tooltip(move |window, cx| value_details_tooltip(&details, window, cx))
            })
            .child(value_label.to_string()),
    )
}
