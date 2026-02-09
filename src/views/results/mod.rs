mod search;
mod tree;
mod types;

use std::sync::Arc;

use gpui::*;
use gpui_component::ActiveTheme as _;

use crate::theme::spacing;
use crate::views::documents::tree::lazy_row::compute_row_meta;
use crate::views::documents::tree::lazy_tree::build_visible_rows;
pub use types::{ResultEmptyState, ResultViewMode, ResultViewProps};

pub fn render_results_view<T: 'static>(
    props: ResultViewProps,
    on_toggle_node: types::ToggleNodeCallback,
    cx: &mut Context<T>,
) -> AnyElement {
    let cx_ref: &App = cx;
    let documents = props.documents.clone();
    let expanded_nodes = props.expanded_nodes.clone();
    let mut visible_rows = build_visible_rows(&documents, &expanded_nodes);
    if !props.search_query.trim().is_empty() {
        visible_rows =
            search::filter_visible_rows(&documents, visible_rows, &props.search_query, cx_ref);
    }
    let visible_rows = Arc::new(visible_rows);
    let row_count = visible_rows.len();

    let header = div()
        .flex()
        .items_center()
        .px(spacing::lg())
        .py(spacing::xs())
        .bg(cx_ref.theme().tab_bar)
        .border_b_1()
        .border_color(cx_ref.theme().border)
        .child(
            div()
                .flex()
                .flex_1()
                .min_w(px(0.0))
                .text_xs()
                .text_color(cx_ref.theme().muted_foreground)
                .child("Key"),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_w(px(0.0))
                .text_xs()
                .text_color(cx_ref.theme().muted_foreground)
                .child("Value"),
        )
        .child(
            div().w(px(120.0)).text_xs().text_color(cx_ref.theme().muted_foreground).child("Type"),
        );

    if documents.is_empty() {
        return empty_state_view(ResultEmptyState::NoDocuments, cx_ref).into_any_element();
    }
    if row_count == 0 {
        return empty_state_view(ResultEmptyState::NoMatches, cx_ref).into_any_element();
    }

    let list =
        div().flex().flex_col().flex_1().min_w(px(0.0)).min_h(px(0.0)).overflow_hidden().child(
            uniform_list(
                "results-tree",
                row_count,
                cx.processor({
                    let documents = documents.clone();
                    let visible_rows = visible_rows.clone();
                    move |_view, range: std::ops::Range<usize>, _window, cx| {
                        range
                            .map(|ix| {
                                let row = &visible_rows[ix];
                                let meta = compute_row_meta(row, &documents, cx);
                                tree::render_result_row(ix, row, &meta, on_toggle_node.clone(), cx)
                            })
                            .collect()
                    }
                }),
            )
            .flex_1()
            .track_scroll(props.scroll_handle),
        );

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .child(header)
        .child(list)
        .into_any_element()
}

fn empty_state_view(state: ResultEmptyState, cx: &App) -> impl IntoElement {
    let text = match state {
        ResultEmptyState::NoDocuments => "No documents returned".to_string(),
        ResultEmptyState::NoMatches => "No matching results".to_string(),
        ResultEmptyState::Custom(text) => text,
    };
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .child(div().text_sm().text_color(cx.theme().muted_foreground).child(text))
}
