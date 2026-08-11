//! UI helper functions for transfer view components.

use gpui::*;
use gpui_component::checkbox::Checkbox;
use gpui_component::{ActiveTheme as _, IconName};

use crate::components::Button;
use crate::state::parse_export_query_document;
use crate::theme::{borders, spacing};

use super::QueryEditField;

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
        .px(spacing::md())
        .py(spacing::sm())
        .border_t_1()
        .border_color(cx.theme().sidebar_border)
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

/// Creates a compact checkbox control for use with an explicit option label.
pub(super) fn checkbox_field<F>(
    id: impl Into<ElementId>,
    checked: bool,
    on_click: F,
    _cx: &App,
) -> Div
where
    F: Fn(&mut App) + 'static,
{
    div()
        .flex()
        .items_center()
        .child(Checkbox::new(id).checked(checked).on_click(move |_, _, cx| on_click(cx)))
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
        if value.len() > 40 {
            let end = value.floor_char_boundary(37);
            format!("{}...", &value[..end])
        } else {
            value.to_string()
        }
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

    let validation_error = parse_export_query_document(value).err();
    form_row(
        label,
        div()
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(value_box)
                    .child(edit_button)
                    .children(clear_button),
            )
            .children(
                validation_error
                    .map(|error| div().text_xs().text_color(cx.theme().danger).child(error)),
            ),
        cx,
    )
}
