//! Filter bar and query options rendering for collection header.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::RopeExt as _;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Icon, IconName, Sizable as _, Size};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{Button, QueryLibraryDialog, QueryLibraryTarget};
use crate::state::{AppCommands, AppState, SessionKey};
use crate::theme::spacing;
use crate::views::documents::CollectionView;

const QUERY_FONT_REM: f32 = 0.875;
const QUERY_LINE_HEIGHT: f32 = 1.5;

fn query_editor_height(rows: usize, window: &Window) -> Pixels {
    // Editor uses Medium input padding internally, even with appearance(false).
    window.rem_size() * QUERY_FONT_REM * QUERY_LINE_HEIGHT * rows as f32
        + Size::Medium.input_py() * 2.0
        + px(2.0)
}

fn query_editor(
    input: &Entity<EditorState>,
    rows: usize,
    label: &'static str,
    disabled: bool,
    invalid: bool,
    window: &Window,
    cx: &App,
) -> impl IntoElement {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    let editor = Editor::new(input)
        .font_family(crate::theme::fonts::mono())
        .text_size(rems(QUERY_FONT_REM))
        .line_height(relative(QUERY_LINE_HEIGHT))
        .h(query_editor_height(rows, window))
        .w_full()
        .bordered(true)
        .border_color(if invalid {
            cx.theme().danger
        } else if focused {
            cx.theme().ring
        } else {
            cx.theme().input
        })
        .aria_label(label)
        .disabled(disabled);
    div()
        .id(("query-pointer", input.entity_id()))
        .min_w(px(0.0))
        .capture_any_mouse_down({
            let input = input.clone();
            move |event, window, cx| {
                if !disabled {
                    super::super::query_editor::correct_query_pointer(&input, event, window, cx);
                }
            }
        })
        .child(editor)
}

fn query_find_button(window: &Window) -> Button {
    Button::new("apply-filter")
        .primary()
        .with_size(Size::Medium)
        .h(query_editor_height(1, window))
        .label("Find")
        .icon(Icon::new(IconName::Search).small())
        .tooltip("Find matching documents · Enter")
}

fn set_query_object_default(
    input: &mut EditorState,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) {
    input.set_value("{}".to_string(), window, cx);
    let position = input.text().offset_to_position(1);
    input.set_cursor_position(position, window, cx);
}

impl CollectionView {
    pub(in crate::views::documents) fn render_filter_row(
        &self,
        is_loading: bool,
        explain_loading: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let session_key = self.view_model.current_session();
        let filter_state = self.filter_state.clone();
        let text = filter_state
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        let valid = super::super::fast_filter::compile_filter_input(&text).is_ok();
        let id = super::super::fast_filter::document_id_input(&text);
        let focused = filter_state
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window));
        let (filter_active, options_open, builder_open, option_count, failed, has_results) =
            session_key
                .as_ref()
                .and_then(|key| self.state.read(cx).session(key))
                .map(|session| {
                    (
                        session.data.filter.is_some(),
                        session.view.query_options_open,
                        session.view.filter_builder_open,
                        usize::from(session.data.sort.is_some())
                            + usize::from(session.data.projection.is_some()),
                        session.data.query_error.is_some(),
                        !session.data.items.is_empty(),
                    )
                })
                .unwrap_or_default();
        let disabled = session_key.is_none();
        let line_count = filter_state.as_ref().map_or(1, |input| input.read(cx).text().lines_len());
        let rows = if self.filter_expanded { 10 } else { line_count.clamp(1, 4) };
        let control_height = query_editor_height(1, window);
        let view = cx.entity();
        let state = self.state.clone();

        let mut input_row =
            div().flex().items_start().gap(spacing::sm()).flex_1().min_w(px(0.0)).child(
                div()
                    .h(control_height)
                    .flex()
                    .items_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Filter")
                    .on_mouse_down(MouseButton::Left, {
                        let input = filter_state.clone();
                        move |_, window, cx| {
                            if disabled {
                                return;
                            }
                            if let Some(input) = &input {
                                let focus = input.read(cx).focus_handle(cx);
                                window.focus(&focus, cx);
                            }
                        }
                    }),
            );
        if let Some(input) = filter_state.clone() {
            input_row = input_row.child(
                div().flex_1().min_w(px(0.0)).debug_selector(|| "query-editor-frame".into()).child(
                    query_editor(
                        &input,
                        rows,
                        "MongoDB filter",
                        disabled,
                        self.filter_error_message.is_some(),
                        window,
                        cx,
                    ),
                ),
            );
        }
        input_row = input_row.child(
            Button::new("expand-filter-editor")
                .ghost()
                .with_size(Size::Medium)
                .h(control_height)
                .icon(
                    Icon::new(if self.filter_expanded {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
                    .small(),
                )
                .tooltip(if self.filter_expanded {
                    "Collapse query editor"
                } else {
                    "Expand query editor · Shift+Enter adds a line"
                })
                .disabled(disabled)
                .on_click({
                    let view = view.clone();
                    let input = filter_state.clone();
                    move |_, window, cx| {
                        view.update(cx, |view, cx| {
                            view.filter_expanded = !view.filter_expanded;
                            cx.notify();
                        });
                        if let Some(input) = &input {
                            let focus = input.read(cx).focus_handle(cx);
                            window.focus(&focus, cx);
                        }
                    }
                }),
        );

        let find = query_find_button(window).disabled(disabled || !valid).on_click({
            let state = state.clone();
            let session = session_key.clone();
            let input = filter_state.clone();
            move |_, window, cx| {
                if let (Some(session), Some(input)) = (session.clone(), input.clone()) {
                    CollectionView::apply_filter(state.clone(), session, input, window, cx);
                }
            }
        });
        let mut primary =
            div().flex().items_start().gap(spacing::sm()).child(input_row).child(find);
        if is_loading {
            primary = primary.child(
                div()
                    .h(control_height)
                    .flex()
                    .items_center()
                    .child(gpui_kit::component::spinner::Spinner::new().small()),
            );
        }

        let mut tools = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(spacing::xs())
            .child(
                Button::new("toggle-filter-builder")
                    .ghost()
                    .small()
                    .label(if builder_open { "Hide conditions" } else { "Add condition" })
                    .icon(Icon::new(IconName::Plus).small())
                    .selected(builder_open)
                    .disabled(disabled)
                    .on_click({
                        let state = state.clone();
                        let session = session_key.clone();
                        move |_, _, cx| {
                            if let Some(session) = &session {
                                state.update(cx, |state, cx| {
                                    state.toggle_filter_builder_open(session);
                                    cx.notify();
                                });
                            }
                        }
                    }),
            )
            .child(
                Button::new("toggle-options")
                    .ghost()
                    .small()
                    .label(if option_count == 0 {
                        "Options".to_string()
                    } else {
                        format!("Options ({option_count})")
                    })
                    .tooltip("Sort and projection")
                    .selected(options_open)
                    .disabled(disabled)
                    .on_click({
                        let state = state.clone();
                        let session = session_key.clone();
                        move |_, _, cx| {
                            if let Some(session) = &session {
                                state.update(cx, |state, cx| {
                                    state.toggle_query_options_open(session);
                                    cx.notify();
                                });
                            }
                        }
                    }),
            )
            .child(
                Button::new("document-query-library")
                    .ghost()
                    .small()
                    .label("History")
                    .tooltip("Query history and saved queries · Cmd/Ctrl+Shift+H")
                    .disabled(disabled)
                    .on_click({
                        let state = state.clone();
                        let session = session_key.clone();
                        move |_, window, cx| {
                            if let Some(session) = session.clone() {
                                QueryLibraryDialog::open(
                                    state.clone(),
                                    QueryLibraryTarget::Documents(session),
                                    window,
                                    cx,
                                );
                            }
                        }
                    }),
            )
            .child(
                Button::new("run-explain")
                    .ghost()
                    .small()
                    .label("Explain")
                    .tooltip("Explain the applied query")
                    .disabled(disabled || explain_loading || self.filter_dirty)
                    .on_click({
                        let state = state.clone();
                        let session = session_key.clone();
                        move |_, _, cx| {
                            if let Some(session) = session.clone() {
                                AppCommands::run_explain_for_session(state.clone(), session, cx);
                            }
                        }
                    }),
            );
        if filter_active || !text.trim().is_empty() {
            tools = tools.child(
                Button::new("clear-filter")
                    .ghost()
                    .small()
                    .label("Reset")
                    .tooltip("Clear the filter and show all documents")
                    .disabled(disabled)
                    .on_click({
                        let state = state.clone();
                        let session = session_key.clone();
                        let input = filter_state.clone();
                        move |_, window, cx| {
                            if let (Some(session), Some(input)) = (session.clone(), input.clone()) {
                                input.update(cx, |input, cx| {
                                    input.replace_all(String::new(), window, cx);
                                });
                                CollectionView::apply_filter(
                                    state.clone(),
                                    session,
                                    input,
                                    window,
                                    cx,
                                );
                            }
                        }
                    }),
            );
        }

        let mut bar = div()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .gap(px(4.0))
            .font_family(crate::theme::fonts::ui())
            .child(primary)
            .child(tools);
        let feedback = if let Some(error) = &self.filter_error_message {
            Some((error.clone(), cx.theme().danger_foreground))
        } else if !valid && !text.trim().is_empty() {
            Some(("Complete the filter · Enter shows details".into(), cx.theme().muted_foreground))
        } else if is_loading {
            Some((
                format!(
                    "Searching collection…{}{}",
                    if self.filter_dirty { " Changes not applied." } else { "" },
                    if has_results { " Showing previous results." } else { "" }
                ),
                cx.theme().muted_foreground,
            ))
        } else if failed && !self.filter_dirty {
            Some((
                if has_results {
                    "Query failed. Showing previous results."
                } else {
                    "Query failed. See details below."
                }
                .into(),
                cx.theme().danger_foreground,
            ))
        } else if let Some(id) = id {
            Some((
                format!(
                    "{} ID · exact _id match{}",
                    crate::bson::bson_type_label(&id),
                    if self.filter_dirty { " · Changes not applied" } else { "" }
                ),
                cx.theme().muted_foreground,
            ))
        } else if self.filter_dirty {
            Some(("Changes not applied · Enter to find".into(), cx.theme().muted_foreground))
        } else if focused && text.trim().is_empty() {
            Some((
                "Type a field or paste a document ID · Tab completes · Shift+Enter adds a line"
                    .into(),
                cx.theme().muted_foreground,
            ))
        } else {
            None
        };
        if let Some((text, color)) = feedback {
            bar = bar.child(div().text_xs().text_color(color).child(text));
        }
        bar
    }
}

fn render_query_segment(
    label: &'static str,
    empty_label: &'static str,
    state: Option<Entity<EditorState>>,
    valid: bool,
    disabled: bool,
    _window: &Window,
    cx: &App,
) -> impl IntoElement {
    let mut row = div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .flex_shrink_0()
        .child(div().text_sm().text_color(cx.theme().muted_foreground).child(label));
    if let Some(state) = state {
        let raw = state.read(cx).value().to_string();
        let fields = crate::bson::parse_document_from_json(&raw).map(|doc| doc.len()).unwrap_or(0);
        let summary = if !valid {
            "Invalid JSON".to_string()
        } else {
            match fields {
                0 => empty_label.to_string(),
                1 => "1 field".to_string(),
                count => format!("{count} fields"),
            }
        };
        let focus = state.read(cx).focus_handle(cx);
        let id = state.entity_id();
        row = row.child(
            div().debug_selector(move || format!("query-option-{label}").into()).child(
                Popover::new(("query-option", id))
                    .trigger(
                        Button::new(("edit-query-option", id))
                            .small()
                            .label(summary)
                            .icon(Icon::new(IconName::ChevronDown).small())
                            .tooltip(format!("Edit {label}"))
                            .when(!valid, |button| button.text_color(cx.theme().danger))
                            .disabled(disabled),
                    )
                    .track_focus(&focus)
                    .content(move |_, window, cx| {
                        let input = state.read(cx);
                        let rows = input.text().lines_len().clamp(4, 12);
                        let invalid = !super::super::query::is_valid_query(&input.value());
                        let popover = cx.entity();
                        div()
                            .flex()
                            .flex_col()
                            .w(rems(34.0))
                            .max_w(window.viewport_size().width - px(48.0))
                            .gap(spacing::sm())
                            .font_family(crate::theme::fonts::ui())
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label))
                            .child(query_editor(&state, rows, label, disabled, invalid, window, cx))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap(spacing::sm())
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Enter applies · Esc closes"),
                                    )
                                    .child(
                                        Button::new("close-query-option")
                                            .small()
                                            .label("Done")
                                            .on_click(move |_, window, cx| {
                                                popover.update(cx, |popover, cx| {
                                                    popover.dismiss(window, cx)
                                                });
                                            }),
                                    ),
                            )
                    }),
            ),
        );
    } else {
        row = row.child(Button::new(label).small().label(empty_label).disabled(true));
    }
    row
}

/// Render the query options panel (sort/projection).
#[allow(clippy::too_many_arguments)]
pub fn render_query_options(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    sort_state: Option<Entity<EditorState>>,
    projection_state: Option<Entity<EditorState>>,
    sort_valid: bool,
    projection_valid: bool,
    sort_active: bool,
    projection_active: bool,
    window: &Window,
    cx: &App,
) -> Div {
    let state_for_query = state.clone();
    let state_for_clear = state.clone();

    let apply_disabled = session_key.is_none() || !sort_valid || !projection_valid;
    let disabled = session_key.is_none();
    let apply_button = filter_action_button(
        Button::new("apply-query").primary().small(),
        IconName::Check,
        "Apply",
    )
    .disabled(apply_disabled)
    .on_click({
        let session_key = session_key.clone();
        let sort_state = sort_state.clone();
        let projection_state = projection_state.clone();
        let state_for_query = state_for_query.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            let Some(session_key) = session_key.clone() else {
                return;
            };
            let Some(sort_state) = sort_state.clone() else {
                return;
            };
            let Some(projection_state) = projection_state.clone() else {
                return;
            };
            CollectionView::apply_query_options(
                state_for_query.clone(),
                session_key,
                sort_state,
                projection_state,
                window,
                cx,
            );
        }
    });
    let clear_button =
        filter_action_button(Button::new("clear-query").ghost().small(), IconName::Close, "Clear")
            .disabled(session_key.is_none() || (!sort_active && !projection_active))
            .on_click({
                let session_key = session_key.clone();
                let sort_state = sort_state.clone();
                let projection_state = projection_state.clone();
                let state_for_clear = state_for_clear.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    let Some(session_key) = session_key.clone() else {
                        return;
                    };
                    let Some(sort_state) = sort_state.clone() else {
                        return;
                    };
                    let Some(projection_state) = projection_state.clone() else {
                        return;
                    };
                    sort_state.update(cx, |state, cx| {
                        set_query_object_default(state, window, cx);
                    });
                    projection_state.update(cx, |state, cx| {
                        set_query_object_default(state, window, cx);
                    });
                    CollectionView::apply_query_options(
                        state_for_clear.clone(),
                        session_key,
                        sort_state,
                        projection_state,
                        window,
                        cx,
                    );
                }
            });

    div()
        .flex()
        .flex_wrap()
        .items_center()
        .w_full()
        .min_w(px(0.0))
        .gap(spacing::sm())
        .font_family(crate::theme::fonts::ui())
        .child(render_query_segment(
            "Sort", "Unsorted", sort_state, sort_valid, disabled, window, cx,
        ))
        .child(render_query_segment(
            "Projection",
            "All fields",
            projection_state,
            projection_valid,
            disabled,
            window,
            cx,
        ))
        .child(clear_button)
        .child(apply_button)
}

fn filter_action_button(button: Button, icon: IconName, label: &'static str) -> Button {
    button.icon(Icon::new(icon).small()).label(label).tooltip(label)
}

#[cfg(test)]
#[path = "filter_bar_tests.rs"]
mod tests;
