use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::input::{InputEvent, InputState, Position, RopeExt, TabSize};
use gpui_component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_component::tree::TreeState;

use crate::state::app_state::PipelineState;
use crate::state::{AppCommands, SessionKey, StatusMessage};
use crate::theme::spacing;

use crate::views::CollectionView;

mod operators;
mod results_view;
mod stage_editor;
mod stage_list;

impl CollectionView {
    pub(in crate::views::documents) fn render_aggregation_view(
        &mut self,
        pipeline: PipelineState,
        session_key: Option<SessionKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.ensure_aggregation_states(window, cx);
        self.sync_aggregation_inputs(&pipeline, session_key.clone(), window, cx);

        let stage_list = self.render_aggregation_stage_list(&pipeline, session_key.clone(), cx);
        let stage_editor =
            self.render_aggregation_stage_editor(&pipeline, session_key.clone(), window, cx);
        let results = self.render_aggregation_results(&pipeline, session_key, window, cx);

        let top_split = h_resizable("agg-top-split")
            .child(
                resizable_panel().size(px(280.0)).size_range(px(220.0)..px(480.0)).child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .overflow_hidden()
                        .child(stage_list),
                ),
            )
            .child(
                resizable_panel().child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .overflow_hidden()
                        .child(stage_editor),
                ),
            )
            .into_any_element();

        let top_panel = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .p(spacing::lg())
            .child(div().flex().flex_1().min_h(px(0.0)).child(top_split));

        let main_split = v_resizable("agg-main-split")
            .child(
                resizable_panel().size(px(240.0)).size_range(px(180.0)..px(900.0)).child(top_panel),
            )
            .child(
                resizable_panel().size(px(360.0)).size_range(px(220.0)..px(1400.0)).child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .overflow_hidden()
                        .child(results),
                ),
            )
            .into_any_element();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(cx.theme().background)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(main_split),
            )
            .into_any_element()
    }

    fn ensure_aggregation_states(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.aggregation_stage_body_state.is_none() {
            let body_state = cx.new(|cx| {
                InputState::new(window, cx)
                    .code_editor("javascript")
                    .line_number(true)
                    .searchable(true)
                    .soft_wrap(true)
                    .placeholder("Stage body (JSON)")
            });
            let subscription =
                cx.subscribe_in(&body_state, window, move |view, state, event, window, cx| {
                    match event {
                        InputEvent::Change => {
                            if view.aggregation_ignore_body_change {
                                view.aggregation_ignore_body_change = false;
                                return;
                            }
                            let Some(session_key) = view.view_model.current_session() else {
                                return;
                            };
                            let selected = view
                                .state
                                .read(cx)
                                .session(&session_key)
                                .and_then(|session| session.data.aggregation.selected_stage);
                            let Some(index) = selected else {
                                return;
                            };
                            let raw = state.read(cx).value().to_string();
                            view.state.update(cx, |state, cx| {
                                state.set_pipeline_stage_body(&session_key, index, raw);
                                cx.notify();
                            });
                        }
                        InputEvent::PressEnter { secondary } => {
                            if *secondary {
                                let Some(session_key) = view.view_model.current_session() else {
                                    return;
                                };
                                AppCommands::run_aggregation(
                                    view.state.clone(),
                                    session_key,
                                    false,
                                    cx,
                                );
                                return;
                            }
                            let mut adjusted = false;
                            state.update(cx, |state, cx| {
                                adjusted = auto_indent_between_braces(state, window, cx);
                            });
                            if adjusted {
                                cx.notify();
                            }
                        }
                        _ => {}
                    }
                });
            self.aggregation_stage_body_state = Some(body_state);
            self.aggregation_stage_body_subscription = Some(subscription);
        }

        if self.aggregation_results_tree_state.is_none() {
            self.aggregation_results_tree_state = Some(cx.new(|cx| TreeState::new(cx)));
        }

        if self.aggregation_limit_state.is_none() {
            let limit_state =
                cx.new(|cx| InputState::new(window, cx).placeholder("Limit").clean_on_escape());
            let subscription =
                cx.subscribe_in(&limit_state, window, move |view, state, event, window, cx| {
                    match event {
                        InputEvent::PressEnter { .. } | InputEvent::Blur => {
                            let Some(session_key) = view.view_model.current_session() else {
                                return;
                            };
                            let raw = state.read(cx).value().to_string();
                            let trimmed = raw.trim();
                            let parsed = if trimmed.is_empty() {
                                Ok(50)
                            } else {
                                trimmed.parse::<i64>().map_err(|_| "Limit must be a number")
                            };
                            let previous_limit = view
                                .state
                                .read(cx)
                                .session(&session_key)
                                .map(|session| session.data.aggregation.result_limit)
                                .unwrap_or(50);
                            match parsed {
                                Ok(limit) if limit > 0 => {
                                    let normalized = limit.to_string();
                                    state.update(cx, |state, cx| {
                                        state.set_value(normalized.clone(), window, cx);
                                    });
                                    view.state.update(cx, |state, cx| {
                                        state.set_pipeline_result_limit(&session_key, limit);
                                        state.set_status_message(Some(StatusMessage::info(
                                            format!("Aggregation limit set to {limit}"),
                                        )));
                                        cx.notify();
                                    });
                                    let should_run =
                                        view.state.read(cx).session(&session_key).is_some_and(
                                            |session| !session.data.aggregation.stages.is_empty(),
                                        );
                                    if should_run {
                                        AppCommands::run_aggregation(
                                            view.state.clone(),
                                            session_key.clone(),
                                            false,
                                            cx,
                                        );
                                    }
                                }
                                Ok(_) => {
                                    let previous_value = previous_limit.to_string();
                                    state.update(cx, |state, cx| {
                                        state.set_value(previous_value, window, cx);
                                    });
                                    view.state.update(cx, |state, cx| {
                                        state.set_status_message(Some(StatusMessage::error(
                                            "Limit must be a positive number.",
                                        )));
                                        cx.notify();
                                    });
                                }
                                Err(message) => {
                                    view.state.update(cx, |state, cx| {
                                        state.set_status_message(Some(StatusMessage::error(
                                            message.to_string(),
                                        )));
                                        cx.notify();
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                });
            self.aggregation_limit_state = Some(limit_state);
            self.aggregation_limit_subscription = Some(subscription);
        }
    }

    fn sync_aggregation_inputs(
        &mut self,
        pipeline: &PipelineState,
        session_key: Option<SessionKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.aggregation_input_session != session_key
            || self.aggregation_selected_stage != pipeline.selected_stage
        {
            let session_changed = self.aggregation_input_session != session_key;
            self.aggregation_input_session = session_key.clone();
            self.aggregation_selected_stage = pipeline.selected_stage;
            if session_changed {
                self.aggregation_drag_over = None;
            }

            let new_count = pipeline.stages.len();
            let prev_count = self.aggregation_stage_count;
            self.aggregation_stage_count = new_count;

            if !session_changed
                && new_count > prev_count
                && pipeline.selected_stage == new_count.checked_sub(1)
                && let Some(selected) = pipeline.selected_stage
            {
                // Scroll the newly added stage into view without a deferred jump.
                self.aggregation_stage_list_scroll
                    .scroll_to_item_strict(selected, ScrollStrategy::Bottom);
            }

            if let Some(body_state) = self.aggregation_stage_body_state.clone() {
                let body_value = pipeline
                    .selected_stage
                    .and_then(|idx| pipeline.stages.get(idx))
                    .map(|stage| stage.body.clone())
                    .unwrap_or_else(|| "{}".to_string());
                let current_value = body_state.read(cx).value().to_string();
                if current_value != body_value {
                    self.aggregation_ignore_body_change = true;
                    body_state.update(cx, |state, cx| {
                        state.set_value(body_value, window, cx);
                    });
                }
            }

            if let Some(limit_state) = self.aggregation_limit_state.clone() {
                let limit_value = pipeline.result_limit.to_string();
                limit_state.update(cx, |state, cx| {
                    state.set_value(limit_value, window, cx);
                });
            }
        }
    }
}

fn auto_indent_between_braces(
    state: &mut InputState,
    window: &mut Window,
    cx: &mut Context<InputState>,
) -> bool {
    let text = state.value().to_string();
    let cursor = state.cursor();
    if cursor >= text.len() {
        return false;
    }

    let bytes = text.as_bytes();
    let close = bytes[cursor];
    let open = match close {
        b'}' => b'{',
        b']' => b'[',
        _ => return false,
    };

    let mut line_start = cursor;
    while line_start > 0 {
        if bytes[line_start - 1] == b'\n' {
            break;
        }
        line_start -= 1;
    }

    let current_indent = &text[line_start..cursor];
    if !current_indent.chars().all(|c| c.is_whitespace()) {
        return false;
    }

    let mut idx = line_start;
    let mut prev_non_ws = None;
    while idx > 0 {
        let b = bytes[idx - 1];
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            idx -= 1;
            continue;
        }
        prev_non_ws = Some(b);
        break;
    }

    if prev_non_ws != Some(open) {
        return false;
    }

    let extra_indent = " ".repeat(TabSize::default().tab_size);
    if extra_indent.is_empty() {
        return false;
    }

    let insert = format!("{extra_indent}\n{current_indent}");
    let mut new_text = String::with_capacity(text.len() + insert.len());
    new_text.push_str(&text[..cursor]);
    new_text.push_str(&insert);
    new_text.push_str(&text[cursor..]);

    state.set_value(new_text, window, cx);
    let new_cursor_offset = cursor + extra_indent.len();
    let position = state.text().offset_to_position(new_cursor_offset);
    state.set_cursor_position(Position::new(position.line, position.character), window, cx);
    true
}
