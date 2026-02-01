//! Query edit modal for filter, projection, and sort fields.

use gpui::*;
use gpui_component::IconName;
use gpui_component::input::{Input, InputState};

use crate::bson::{format_relaxed_json_compact, parse_value_from_relaxed_json};
use crate::components::Button;
use crate::theme::{borders, colors, spacing};

use super::TransferView;

/// Which query field is being edited in the modal.
#[derive(Clone, Copy, PartialEq)]
pub enum QueryEditField {
    Filter,
    Projection,
    Sort,
}

impl TransferView {
    /// Open the query edit modal for the specified field.
    pub(super) fn open_query_modal(
        &mut self,
        field: QueryEditField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Get current value for this field
        let current_value = {
            let state_ref = self.state.read(cx);
            if let Some(id) = state_ref.active_transfer_tab_id()
                && let Some(tab) = state_ref.transfer_tab(id)
            {
                match field {
                    QueryEditField::Filter => tab.options.export_filter.clone(),
                    QueryEditField::Projection => tab.options.export_projection.clone(),
                    QueryEditField::Sort => tab.options.export_sort.clone(),
                }
            } else {
                String::new()
            }
        };

        // Create input state for modal textarea
        let input_state = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(current_value, window, cx);
            state
        });

        self.query_edit_modal = Some(field);
        self.query_edit_input = Some(input_state);
        cx.notify();
    }

    /// Save the query modal content and close.
    pub(super) fn save_query_modal(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.query_edit_modal else {
            return;
        };
        let Some(ref input_state) = self.query_edit_input else {
            return;
        };

        let new_value = input_state.read(cx).value().to_string();

        self.state.update(cx, |state, cx| {
            if let Some(id) = state.active_transfer_tab_id()
                && let Some(tab) = state.transfer_tab_mut(id)
            {
                match field {
                    QueryEditField::Filter => tab.options.export_filter = new_value,
                    QueryEditField::Projection => tab.options.export_projection = new_value,
                    QueryEditField::Sort => tab.options.export_sort = new_value,
                }
                cx.notify();
            }
        });

        // Close modal
        self.query_edit_modal = None;
        self.query_edit_input = None;
        cx.notify();
    }

    /// Close the query modal without saving.
    pub(super) fn close_query_modal(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.query_edit_modal = None;
        self.query_edit_input = None;
        cx.notify();
    }

    /// Format the JSON in the modal textarea (compact, single-line since Input doesn't support newlines).
    pub(super) fn format_query_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ref input_state) = self.query_edit_input else {
            return;
        };

        let current_text = input_state.read(cx).value().to_string();
        if current_text.is_empty() {
            return;
        }

        // Try to parse using relaxed JSON parser, then output as compact relaxed JSON
        // (Input component doesn't support newlines, so we use single-line format)
        if let Ok(value) = parse_value_from_relaxed_json(&current_text) {
            let formatted = format_relaxed_json_compact(&value);
            input_state.update(cx, |state, cx| {
                state.set_value(formatted, window, cx);
            });
        }
    }

    /// Clear the JSON in the modal textarea.
    pub(super) fn clear_query_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ref input_state) = self.query_edit_input {
            input_state.update(cx, |state, cx| {
                state.set_value(String::new(), window, cx);
            });
        }
    }

    /// Render the query edit modal (returns empty if not open).
    pub(super) fn render_query_edit_modal(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(field) = self.query_edit_modal else {
            return div().into_any_element();
        };

        let Some(ref input_state) = self.query_edit_input else {
            return div().into_any_element();
        };

        let title = match field {
            QueryEditField::Filter => "Edit Filter",
            QueryEditField::Projection => "Edit Projection",
            QueryEditField::Sort => "Edit Sort",
        };

        let current_text = input_state.read(cx).value().to_string();
        let is_valid =
            current_text.is_empty() || parse_value_from_relaxed_json(&current_text).is_ok();

        let view = cx.entity();
        let view_save = view.clone();
        let view_cancel = view.clone();
        let view_format = view.clone();
        let view_clear = view.clone();

        // Modal overlay
        div()
            .absolute()
            .inset_0()
            .bg(hsla(0.0, 0.0, 0.0, 0.5)) // Semi-transparent backdrop
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(500.0))
                    .max_h(px(400.0))
                    .bg(colors::bg_sidebar())
                    .rounded(borders::radius_sm())
                    .border_1()
                    .border_color(colors::border())
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    // Header
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(spacing::md())
                            .py(spacing::sm())
                            .border_b_1()
                            .border_color(colors::border_subtle())
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(colors::text_primary())
                                    .child(title),
                            )
                            .child(
                                Button::new("modal-close")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Close)
                                    .on_click(move |_, window, cx| {
                                        view_cancel.update(cx, |view, cx| {
                                            view.close_query_modal(window, cx);
                                        });
                                    }),
                            ),
                    )
                    // Body - textarea
                    .child(
                        div()
                            .flex_1()
                            .p(spacing::md())
                            .min_h(px(200.0))
                            .child(Input::new(input_state).h_full().w_full()),
                    )
                    // Validation status
                    .child(
                        div()
                            .px(spacing::md())
                            .pb(spacing::sm())
                            .text_sm()
                            .text_color(if is_valid {
                                hsla(0.33, 0.7, 0.5, 1.0) // Green
                            } else {
                                hsla(0.0, 0.7, 0.5, 1.0) // Red
                            })
                            .child(if is_valid { "✓ Valid JSON" } else { "✗ Invalid JSON" }),
                    )
                    // Footer buttons
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(spacing::sm())
                            .px(spacing::md())
                            .py(spacing::sm())
                            .border_t_1()
                            .border_color(colors::border_subtle())
                            .child(
                                Button::new("modal-format")
                                    .ghost()
                                    .compact()
                                    .label("Format")
                                    .on_click(move |_, window, cx| {
                                        view_format.update(cx, |view, cx| {
                                            view.format_query_modal(window, cx);
                                        });
                                    }),
                            )
                            .child(
                                Button::new("modal-clear")
                                    .ghost()
                                    .compact()
                                    .label("Clear")
                                    .on_click(move |_, window, cx| {
                                        view_clear.update(cx, |view, cx| {
                                            view.clear_query_modal(window, cx);
                                        });
                                    }),
                            )
                            .child(
                                Button::new("modal-cancel")
                                    .ghost()
                                    .compact()
                                    .label("Cancel")
                                    .on_click(move |_, window, cx| {
                                        view.update(cx, |view, cx| {
                                            view.close_query_modal(window, cx);
                                        });
                                    }),
                            )
                            .child(
                                Button::new("modal-save")
                                    .primary()
                                    .compact()
                                    .label("Save")
                                    .disabled(!is_valid)
                                    .on_click(move |_, window, cx| {
                                        view_save.update(cx, |view, cx| {
                                            view.save_query_modal(window, cx);
                                        });
                                    }),
                            ),
                    ),
            )
            .into_any_element()
    }
}
