//! UI helper functions for transfer view components.

use gpui::*;
use gpui_component::checkbox::Checkbox;
use gpui_component::{ActiveTheme as _, IconName};

use crate::components::Button;
use crate::theme::{borders, spacing};

use super::QueryEditField;

/// Panel wrapper with title and content.
pub(super) fn panel(title: &str, content: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .p(spacing::md())
        .bg(cx.theme().tab_bar)
        .border_1()
        .border_color(cx.theme().sidebar_border)
        .rounded(borders::radius_sm())
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().secondary_foreground)
                .child(title.to_string()),
        )
        .child(content)
}

/// Form row with horizontal label + control for cleaner alignment.
pub(super) fn form_row(label: &str, control: impl IntoElement, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(spacing::md())
        .child(
            div()
                .w(px(100.0)) // Fixed label width for alignment
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_string()),
        )
        .child(div().flex_1().max_w(px(400.0)).child(control))
}

/// Static form row with horizontal label + value.
pub(super) fn form_row_static(label: &str, value: impl Into<String>, cx: &App) -> impl IntoElement {
    form_row(label, value_box(value, false, cx), cx)
}

/// Value display box.
pub(super) fn value_box(value: impl Into<String>, muted: bool, cx: &App) -> Div {
    div()
        .px(spacing::sm())
        .py(px(6.0))
        .bg(cx.theme().sidebar)
        .border_1()
        .border_color(cx.theme().sidebar_border)
        .rounded(borders::radius_sm())
        .text_sm()
        .text_color(if muted { cx.theme().muted_foreground } else { cx.theme().foreground })
        .child(value.into())
}

/// Option value pill display.
pub(super) fn option_value_pill(value: impl Into<String>, cx: &App) -> AnyElement {
    div()
        .px(spacing::sm())
        .py(px(4.0))
        .bg(cx.theme().sidebar)
        .border_1()
        .border_color(cx.theme().sidebar_border)
        .rounded(borders::radius_sm())
        .text_xs()
        .text_color(cx.theme().secondary_foreground)
        .child(value.into())
        .into_any_element()
}

/// Option section with title and rows.
pub(super) fn option_section(title: &str, rows: Vec<AnyElement>, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .p(spacing::sm())
        .bg(cx.theme().tab_bar)
        .border_1()
        .border_color(cx.theme().sidebar_border)
        .rounded(borders::radius_sm())
        .min_w(px(220.0))
        .flex_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().muted_foreground)
                .child(title.to_string()),
        )
        .child(div().flex().flex_wrap().gap(spacing::md()).children(rows))
}

/// Option field with label and control.
pub(super) fn option_field(label: &str, control: AnyElement, cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .min_w(px(160.0))
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label.to_string()))
        .child(control)
        .into_any_element()
}

/// Static option field with label and value pill.
pub(super) fn option_field_static(label: &str, value: impl Into<String>, cx: &App) -> AnyElement {
    option_field(label, option_value_pill(value, cx), cx)
}

/// Creates a checkbox field with "Enabled" label.
pub(super) fn checkbox_field<F>(
    id: impl Into<ElementId>,
    checked: bool,
    on_click: F,
    cx: &App,
) -> Div
where
    F: Fn(&mut App) + 'static,
{
    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .child(Checkbox::new(id).checked(checked).on_click(move |_, _, cx| on_click(cx)))
        .child(div().text_sm().text_color(cx.theme().secondary_foreground).child("Enabled"))
}

/// Compact summary item for horizontal summary bar.
pub(super) fn summary_item(label: &str, value: impl Into<String>, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label.to_string()))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().secondary_foreground)
                .overflow_x_hidden()
                .text_ellipsis()
                .child(value.into()),
        )
}

/// Returns the value if non-empty, otherwise returns the fallback.
pub(super) fn fallback_text(value: &str, fallback: &str) -> String {
    if value.is_empty() { fallback.to_string() } else { value.to_string() }
}

/// Render a read-only query field row with Edit and Clear buttons.
pub(super) fn render_query_field_row(
    label: &str,
    field: QueryEditField,
    value: &str,
    view: Entity<super::TransferView>,
    state: Entity<crate::state::AppState>,
    cx: &App,
) -> impl IntoElement {
    // Display text: truncated JSON or "(none)"
    let display_text = if value.is_empty() {
        "(none)".to_string()
    } else {
        // Truncate if longer than ~40 chars
        if value.len() > 40 { format!("{}...", &value[..37]) } else { value.to_string() }
    };

    let is_empty = value.is_empty();

    let value_box = div()
        .flex_1()
        .px(spacing::sm())
        .py_1()
        .rounded(borders::radius_sm())
        .bg(cx.theme().background)
        .border_1()
        .border_color(cx.theme().border)
        .text_sm()
        .text_color(if is_empty { cx.theme().muted_foreground } else { cx.theme().foreground })
        .overflow_hidden()
        .text_ellipsis()
        .child(display_text);

    let edit_button = Button::new(("edit-query", field as usize)).compact().label("Edit").on_click(
        move |_, window, cx| {
            view.update(cx, |view, cx| {
                view.open_query_modal(field, window, cx);
            });
        },
    );

    // Clear button - only shown when field has a value
    let clear_button = if !is_empty {
        Some(
            Button::new(("clear-query", field as usize))
                .ghost()
                .compact()
                .icon(IconName::Close)
                .on_click(move |_, _, cx| {
                    state.update(cx, |state, cx| {
                        if let Some(id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(id)
                        {
                            match field {
                                QueryEditField::Filter => tab.options.export_filter.clear(),
                                QueryEditField::Projection => tab.options.export_projection.clear(),
                                QueryEditField::Sort => tab.options.export_sort.clear(),
                            }
                            cx.notify();
                        }
                    });
                }),
        )
    } else {
        None
    };

    form_row(
        label,
        div()
            .flex()
            .items_center()
            .gap(spacing::sm())
            .child(value_box)
            .child(edit_button)
            .children(clear_button),
        cx,
    )
}
