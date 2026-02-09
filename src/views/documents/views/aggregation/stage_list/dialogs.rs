//! Stage dialogs for operator picker and pipeline import.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputState};
use gpui_component::scroll::ScrollableElement;

use crate::components::{Button, cancel_button};
use crate::state::StatusMessage;
use crate::state::app_state::{PipelineStage, SessionKey};
use crate::theme::spacing;

use super::super::operators::OPERATOR_GROUPS;
use serde_json::Value as JsonValue;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Default)]
pub(super) struct OperatorPickerDialogState {
    pub focused_once: bool,
    pub last_query: String,
    pub auto_focused_operator: Option<String>,
}

#[derive(Default)]
pub(super) struct ImportPipelineDialogState {
    pub focused_once: bool,
    pub error: Option<String>,
}

pub(super) fn open_stage_operator_picker_dialog(
    window: &mut Window,
    cx: &mut App,
    state: Entity<crate::state::AppState>,
    session_key: SessionKey,
    insert_index: Option<usize>,
) {
    let title = if insert_index.is_some() { "Insert stage" } else { "Add stage" };
    let dialog_key = insert_index.unwrap_or(usize::MAX);

    window.open_dialog(cx, move |dialog: Dialog, window: &mut Window, cx: &mut App| {
        let search_state =
            window.use_keyed_state(("agg-stage-operator-search", dialog_key), cx, |window, cx| {
                InputState::new(window, cx).placeholder("Search operators...")
            });
        let dialog_state = window.use_keyed_state(
            ("agg-stage-operator-picker-state", dialog_key),
            cx,
            |_window, _cx| OperatorPickerDialogState::default(),
        );

        if !dialog_state.read(cx).focused_once {
            dialog_state.update(cx, |state, _cx| state.focused_once = true);
            search_state.update(cx, |state, cx| {
                state.set_value(String::new(), window, cx);
            });
            let focus = search_state.read(cx).focus_handle(cx);
            window.defer(cx, move |window, _cx| {
                window.focus(&focus);
            });
        }

        let query_raw = search_state.read(cx).value().to_string();
        let query = query_raw.trim().to_ascii_lowercase();
        let has_query = !query.is_empty();

        let last_query = dialog_state.read(cx).last_query.clone();
        if last_query != query_raw {
            dialog_state.update(cx, |state, _cx| {
                state.last_query = query_raw.clone();
                state.auto_focused_operator = None;
            });
        }

        let mut focus_map: Vec<(FocusHandle, String)> = Vec::new();
        let mut first_operator: Option<(String, FocusHandle)> = None;
        let mut operator_count = 0usize;
        let mut next_tab_index: isize = 1;

        let sections = OPERATOR_GROUPS
            .iter()
            .enumerate()
            .filter_map(|(group_idx, group)| {
                let mut buttons = Vec::new();
                for (op_idx, operator) in group.operators.iter().enumerate() {
                    if has_query && !operator.to_ascii_lowercase().contains(&query) {
                        continue;
                    }

                    operator_count = operator_count.saturating_add(1);
                    let operator = (*operator).to_string();
                    let focus_id = operator_focus_id(dialog_key, &operator);
                    let focus_handle = window
                        .use_keyed_state(
                            ("agg-stage-operator-focus", focus_id),
                            cx,
                            |_window, cx| cx.focus_handle(),
                        )
                        .read(cx)
                        .clone();

                    if first_operator.is_none() {
                        first_operator = Some((operator.clone(), focus_handle.clone()));
                    }

                    focus_map.push((focus_handle.clone(), operator.clone()));

                    let id_index = group_idx.saturating_mul(100).saturating_add(op_idx);
                    let state = state.clone();
                    let session_key = session_key.clone();
                    let tab_index = next_tab_index;
                    next_tab_index = next_tab_index.saturating_add(1);

                    buttons.push(
                        Button::new(("agg-stage-operator", id_index))
                            .compact()
                            .label(operator.clone())
                            .track_focus(&focus_handle)
                            .tab_index(tab_index)
                            .on_click({
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    apply_operator_selection(
                                        &state,
                                        &session_key,
                                        insert_index,
                                        &operator,
                                        window,
                                        cx,
                                    );
                                }
                            })
                            .into_any_element(),
                    );
                }

                if has_query && buttons.is_empty() {
                    return None;
                }

                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(group.label),
                        )
                        .child(div().flex().flex_wrap().gap(spacing::xs()).children(buttons))
                        .into_any_element(),
                )
            })
            .collect::<Vec<_>>();

        let empty_results = has_query && sections.is_empty();

        let single_operator = if operator_count == 1 { first_operator.clone() } else { None };

        let auto_focused = dialog_state.read(cx).auto_focused_operator.clone();
        if let Some((operator, _focus)) = single_operator.clone() {
            if auto_focused.as_deref() != Some(operator.as_str()) {
                dialog_state.update(cx, |state, _cx| {
                    state.auto_focused_operator = Some(operator.clone());
                });
            }
        } else if auto_focused.is_some() {
            dialog_state.update(cx, |state, _cx| {
                state.auto_focused_operator = None;
            });
        }

        let search_focus = search_state.read(cx).focus_handle(cx);
        let focus_map_for_keys = focus_map.clone();
        let single_operator_for_keys = single_operator.as_ref().map(|(op, _)| op.clone());
        let state_for_keys = state.clone();
        let session_key_for_keys = session_key.clone();

        let key_handler = move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            let key = event.keystroke.key.to_ascii_lowercase();
            if key == "escape" {
                cx.stop_propagation();
                window.close_dialog(cx);
                return;
            }

            if key == "enter" || key == "return" {
                if search_focus.is_focused(window)
                    && let Some(operator) = single_operator_for_keys.as_deref()
                {
                    cx.stop_propagation();
                    apply_operator_selection(
                        &state_for_keys,
                        &session_key_for_keys,
                        insert_index,
                        operator,
                        window,
                        cx,
                    );
                    return;
                }

                if let Some((_, operator)) =
                    focus_map_for_keys.iter().find(|(focus, _)| focus.is_focused(window))
                {
                    cx.stop_propagation();
                    apply_operator_selection(
                        &state_for_keys,
                        &session_key_for_keys,
                        insert_index,
                        operator,
                        window,
                        cx,
                    );
                }
            }
        };

        dialog.title(title).min_w(px(560.0)).child(
            div()
                .flex()
                .flex_col()
                .gap(spacing::md())
                .p(spacing::md())
                .on_key_down(key_handler)
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().secondary_foreground)
                        .child("Choose an operator"),
                )
                .child(Input::new(&search_state).w_full().tab_index(0))
                .child(if empty_results {
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No operators match your search")
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::sm())
                        .max_h(px(360.0))
                        .overflow_y_scrollbar()
                        .children(sections)
                        .into_any_element()
                }),
        )
    });
}

fn operator_focus_id(dialog_key: usize, operator: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    dialog_key.hash(&mut hasher);
    operator.hash(&mut hasher);
    hasher.finish()
}

fn apply_operator_selection(
    state: &Entity<crate::state::AppState>,
    session_key: &SessionKey,
    insert_index: Option<usize>,
    operator: &str,
    window: &mut Window,
    cx: &mut App,
) {
    let operator = operator.to_string();
    let status = if insert_index.is_some() { "Stage inserted" } else { "Stage added" };

    state.update(cx, |state, cx| {
        match insert_index {
            Some(index) => state.insert_pipeline_stage(session_key, index, operator.clone()),
            None => state.add_pipeline_stage(session_key, operator.clone()),
        };
        state.set_status_message(Some(StatusMessage::info(status)));
        cx.notify();
    });
    window.close_dialog(cx);
}

pub(super) fn open_import_pipeline_dialog(
    window: &mut Window,
    cx: &mut App,
    state: Entity<crate::state::AppState>,
    session_key: SessionKey,
) {
    let session_id = session_key_id(&session_key);
    window.open_dialog(cx, move |dialog: Dialog, window: &mut Window, cx: &mut App| {
        let pipeline_state =
            window.use_keyed_state(("agg-import-pipeline-input", session_id), cx, |window, cx| {
                InputState::new(window, cx)
                    .code_editor("json")
                    .line_number(true)
                    .soft_wrap(true)
                    .placeholder("Paste pipeline JSON array")
            });
        let dialog_state = window.use_keyed_state(
            ("agg-import-pipeline-state", session_id),
            cx,
            |_window, _cx| ImportPipelineDialogState::default(),
        );

        if !dialog_state.read(cx).focused_once {
            dialog_state.update(cx, |state, _cx| {
                state.focused_once = true;
                state.error = None;
            });
            pipeline_state.update(cx, |state, cx| {
                state.set_value(String::new(), window, cx);
            });
            let focus = pipeline_state.read(cx).focus_handle(cx);
            window.defer(cx, move |window, _cx| {
                window.focus(&focus);
            });
        }

        let error_text = dialog_state.read(cx).error.clone();

        dialog.title("Import pipeline").min_w(px(720.0)).child(
            div()
                .flex()
                .flex_col()
                .gap(spacing::md())
                .p(spacing::md())
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().secondary_foreground)
                        .child("Paste a MongoDB aggregation pipeline JSON array"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            Button::new("agg-import-paste")
                                .compact()
                                .label("Paste from Clipboard")
                                .tooltip("Paste pipeline JSON from clipboard")
                                .on_click({
                                    let pipeline_state = pipeline_state.clone();
                                    let dialog_state = dialog_state.clone();
                                    let state = state.clone();
                                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        let Some(text) =
                                            cx.read_from_clipboard().and_then(|item| item.text())
                                        else {
                                            let message =
                                                "Clipboard is empty or does not contain text";
                                            dialog_state.update(cx, |state, _cx| {
                                                state.error = Some(message.to_string());
                                            });
                                            state.update(cx, |state, cx| {
                                                state.set_status_message(Some(
                                                    StatusMessage::error(message),
                                                ));
                                                cx.notify();
                                            });
                                            return;
                                        };
                                        dialog_state.update(cx, |state, _cx| state.error = None);
                                        pipeline_state.update(cx, |state, cx| {
                                            state.set_value(text.to_string(), window, cx);
                                        });
                                    }
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Expected format: [{ \"$match\": { ... } }, ...]"),
                        ),
                )
                .child(
                    Input::new(&pipeline_state)
                        .font_family(crate::theme::fonts::mono())
                        .w_full()
                        .h(px(320.0)),
                )
                .when_some(error_text.clone(), |this, error| {
                    this.child(
                        div().text_sm().text_color(cx.theme().danger_foreground).child(error),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(spacing::xs())
                        .child(cancel_button("agg-import-cancel"))
                        .child(
                            Button::new("agg-import-confirm").primary().label("Import").on_click({
                                let pipeline_state = pipeline_state.clone();
                                let dialog_state = dialog_state.clone();
                                let state = state.clone();
                                let session_key = session_key.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    let raw = pipeline_state.read(cx).value().to_string();
                                    let stages = match parse_pipeline_stages(&raw) {
                                        Ok(stages) => stages,
                                        Err(err) => {
                                            dialog_state.update(cx, |state, _cx| {
                                                state.error = Some(err.clone());
                                            });
                                            state.update(cx, |state, cx| {
                                                state.set_status_message(Some(
                                                    StatusMessage::error(err),
                                                ));
                                                cx.notify();
                                            });
                                            return;
                                        }
                                    };

                                    dialog_state.update(cx, |state, _cx| state.error = None);
                                    state.update(cx, |state, cx| {
                                        state.replace_pipeline_stages(&session_key, stages);
                                        state.set_status_message(Some(StatusMessage::info(
                                            "Pipeline imported",
                                        )));
                                        cx.notify();
                                    });
                                    window.close_dialog(cx);
                                }
                            }),
                        ),
                ),
        )
    });
}

fn parse_pipeline_stages(raw: &str) -> Result<Vec<PipelineStage>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Pipeline JSON is required".to_string());
    }
    let value: JsonValue = crate::bson::parse_value_from_relaxed_json(trimmed)?;
    let stages = value.as_array().ok_or_else(|| "Pipeline must be a JSON array".to_string())?;
    if stages.is_empty() {
        return Err("Pipeline is empty".to_string());
    }

    stages.iter().enumerate().map(|(idx, stage)| parse_pipeline_stage(stage, idx)).collect()
}

fn parse_pipeline_stage(stage: &JsonValue, index: usize) -> Result<PipelineStage, String> {
    let stage_obj =
        stage.as_object().ok_or_else(|| format!("Stage {} must be a JSON document", index + 1))?;

    if stage_obj.len() != 1 {
        return Err(format!("Stage {} must contain exactly one operator", index + 1));
    }

    let (operator, body) = stage_obj.iter().next().expect("validated len == 1");
    let body_str = serde_json::to_string_pretty(body).map_err(|err| err.to_string())?;

    Ok(PipelineStage { operator: operator.to_string(), body: body_str, enabled: true })
}

pub(super) fn session_key_id(session_key: &SessionKey) -> u64 {
    let mut hasher = DefaultHasher::new();
    session_key.hash(&mut hasher);
    hasher.finish()
}
