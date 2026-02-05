use gpui::*;
use gpui_component::{Icon, IconName, Sizable};

use crate::theme::{colors, spacing};
use crate::views::documents::tree::lazy_tree::VisibleRow;
use crate::views::results::types::ToggleNodeCallback;

pub fn render_result_row(
    ix: usize,
    row: &VisibleRow,
    meta: &crate::views::documents::tree::lazy_row::LazyRowMeta,
    on_toggle_node: ToggleNodeCallback,
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
        let on_toggle = on_toggle_node.clone();
        div()
            .id(("result-row-chevron", ix))
            .w(px(14.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                if event.click_count == 1 {
                    cx.stop_propagation();
                    on_toggle(toggle_node_id.clone(), cx);
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
        .id(("result-row", ix))
        .flex()
        .items_center()
        .w_full()
        .px(spacing::lg())
        .py(spacing::xs())
        .hover(|s| s.bg(colors::list_hover()))
        .on_mouse_down(MouseButton::Left, {
            let node_id = node_id.clone();
            let on_toggle = on_toggle_node.clone();
            move |event, _window, cx| {
                if event.click_count == 2 && is_folder {
                    on_toggle(node_id.clone(), cx);
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
