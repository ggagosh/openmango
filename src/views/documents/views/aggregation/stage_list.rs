use gpui::Styled as _;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::Disableable as _;
use gpui_component::switch::Switch;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::components::Button;
use crate::state::app_state::PipelineState;
use crate::state::{SessionKey, StatusMessage};
use crate::theme::{borders, colors, spacing};

use crate::views::CollectionView;

const QUICK_START_OPERATORS: &[&str] = &["$match", "$group", "$project"];

impl CollectionView {
    pub(in crate::views::documents) fn render_aggregation_stage_list(
        &self,
        pipeline: &PipelineState,
        session_key: Option<SessionKey>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(spacing::sm())
            .py(spacing::xs())
            .bg(colors::bg_header())
            .border_b_1()
            .border_color(colors::border())
            .child(div().text_sm().text_color(colors::text_primary()).child("Stages"))
            .child(
                Button::new("agg-add-stage")
                    .compact()
                    .label("+ Add Stage")
                    .disabled(session_key.is_none())
                    .on_click({
                        let session_key = session_key.clone();
                        let state = self.state.clone();
                        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            state.update(cx, |state, cx| {
                                state.add_pipeline_stage(&session_key, "$match");
                                state.set_status_message(Some(StatusMessage::info("Stage added")));
                                cx.notify();
                            });
                        }
                    }),
            );

        let body = if pipeline.stages.is_empty() {
            render_empty_state(session_key.clone(), self.state.clone())
        } else {
            render_stage_list(
                pipeline,
                session_key.clone(),
                self.state.clone(),
                pipeline.selected_stage,
                self.aggregation_stage_list_scroll.clone(),
                cx,
            )
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(colors::bg_sidebar())
            .border_1()
            .border_color(colors::border_subtle())
            .rounded(borders::radius_sm())
            .track_focus(&self.aggregation_focus)
            .on_mouse_down(MouseButton::Left, {
                let focus = self.aggregation_focus.clone();
                move |_, window, _cx| {
                    window.focus(&focus);
                }
            })
            .on_key_down({
                let view = view.clone();
                let session_key = session_key.clone();
                move |event, _window, cx| {
                    let Some(session_key) = session_key.clone() else {
                        return;
                    };
                    view.update(cx, |this, cx| {
                        if handle_stage_list_key(this, event, &session_key, cx) {
                            cx.stop_propagation();
                        }
                    });
                }
            })
            .child(header)
            .child(body)
            .into_any_element()
    }
}

fn handle_stage_list_key(
    view: &mut CollectionView,
    event: &KeyDownEvent,
    session_key: &SessionKey,
    cx: &mut Context<CollectionView>,
) -> bool {
    let pipeline =
        view.state.read(cx).session_data(session_key).map(|data| data.aggregation.clone());
    let Some(pipeline) = pipeline else {
        return false;
    };
    let count = pipeline.stages.len();
    if count == 0 {
        return false;
    }

    let key = event.keystroke.key.to_ascii_lowercase();
    let modifiers = event.keystroke.modifiers;
    let cmd_or_ctrl = modifiers.secondary() || modifiers.control;

    match key.as_str() {
        "up" if cmd_or_ctrl && modifiers.shift => {
            let Some(selected) = pipeline.selected_stage else {
                return false;
            };
            if selected == 0 {
                return false;
            }
            view.state.update(cx, |state, cx| {
                state.move_pipeline_stage(session_key, selected, selected - 1);
                cx.notify();
            });
            true
        }
        "down" if cmd_or_ctrl && modifiers.shift => {
            let Some(selected) = pipeline.selected_stage else {
                return false;
            };
            if selected + 1 >= count {
                return false;
            }
            view.state.update(cx, |state, cx| {
                state.move_pipeline_stage(session_key, selected, selected + 1);
                cx.notify();
            });
            true
        }
        "up" if !modifiers.modified() => {
            let current = pipeline.selected_stage.unwrap_or(0);
            let next = current.saturating_sub(1);
            view.state.update(cx, |state, cx| {
                state.set_pipeline_selected_stage(session_key, Some(next));
                cx.notify();
            });
            true
        }
        "down" if !modifiers.modified() => {
            let current = pipeline.selected_stage.unwrap_or(0);
            let next = (current + 1).min(count.saturating_sub(1));
            view.state.update(cx, |state, cx| {
                state.set_pipeline_selected_stage(session_key, Some(next));
                cx.notify();
            });
            true
        }
        "d" if cmd_or_ctrl => {
            let Some(selected) = pipeline.selected_stage else {
                return false;
            };
            view.state.update(cx, |state, cx| {
                state.duplicate_pipeline_stage(session_key, selected);
                cx.notify();
            });
            true
        }
        "backspace" | "delete" if !modifiers.modified() => {
            let Some(selected) = pipeline.selected_stage else {
                return false;
            };
            view.state.update(cx, |state, cx| {
                state.remove_pipeline_stage(session_key, selected);
                cx.notify();
            });
            true
        }
        _ => false,
    }
}

fn render_empty_state(
    session_key: Option<SessionKey>,
    state: Entity<crate::state::AppState>,
) -> AnyElement {
    let quick_buttons = QUICK_START_OPERATORS
        .iter()
        .enumerate()
        .map(|(idx, operator)| {
            Button::new(("agg-quick-stage", idx))
                .compact()
                .label(*operator)
                .disabled(session_key.is_none())
                .on_click({
                    let session_key = session_key.clone();
                    let state = state.clone();
                    let operator = operator.to_string();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        state.update(cx, |state, cx| {
                            state.add_pipeline_stage(&session_key, operator.clone());
                            cx.notify();
                        });
                    }
                })
                .into_any_element()
        })
        .collect::<Vec<_>>();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(spacing::sm())
        .px(spacing::sm())
        .py(spacing::lg())
        .child(div().text_sm().text_color(colors::text_muted()).child("No pipeline stages yet."))
        .child(
            Button::new("agg-add-first-stage")
                .label("+ Add your first stage")
                .disabled(session_key.is_none())
                .on_click({
                    let session_key = session_key.clone();
                    let state = state.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        state.update(cx, |state, cx| {
                            state.add_pipeline_stage(&session_key, "$match");
                            cx.notify();
                        });
                    }
                }),
        )
        .child(div().text_xs().text_color(colors::text_muted()).child("Common starting points"))
        .child(div().flex().items_center().gap(spacing::xs()).children(quick_buttons))
        .into_any_element()
}

fn render_stage_list(
    pipeline: &PipelineState,
    session_key: Option<SessionKey>,
    state: Entity<crate::state::AppState>,
    selected: Option<usize>,
    scroll_handle: UniformListScrollHandle,
    cx: &mut Context<CollectionView>,
) -> AnyElement {
    let stages = pipeline.stages.clone();
    let stage_doc_counts = pipeline.stage_doc_counts.clone();
    let item_count = stages.len();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .overflow_hidden()
        .child(
            uniform_list(
                "agg-stage-list",
                item_count,
                cx.processor({
                    let state = state.clone();
                    let session_key = session_key.clone();
                    move |_view, range: std::ops::Range<usize>, _window, _cx| {
                        range
                            .map(|idx| {
                                let stage = &stages[idx];
                                let is_selected = selected == Some(idx);
                                let count_label = stage_doc_counts
                                    .get(idx)
                                    .and_then(|value| *value)
                                    .map(|count| format!("{count} docs"));
                                let operator_label = if stage.operator.trim().is_empty() {
                                    "Operator".to_string()
                                } else {
                                    stage.operator.clone()
                                };

                                let stage_state = state.clone();
                                let stage_session = session_key.clone();
                                let row_state = state.clone();
                                let row_session = session_key.clone();
                                let remove_state = state.clone();
                                let remove_session = session_key.clone();
                                let stage_enabled = stage.enabled;

                                div()
                                    .id(("agg-stage-row", idx))
                                    .flex()
                                    .flex_col()
                                    .w_full()
                                    .flex_shrink_0()
                                    .gap(px(2.0))
                                    .px(spacing::sm())
                                    .py(spacing::xs())
                                    .rounded(borders::radius_sm())
                                    .border_1()
                                    .border_color(rgba(0x00000000))
                                    .when(is_selected, |s| {
                                        s.bg(colors::list_selected()).border_color(colors::border())
                                    })
                                    .when(!is_selected, |s| s.hover(|s| s.bg(colors::list_hover())))
                                    .on_mouse_down(MouseButton::Left, {
                                        move |_, _window, cx| {
                                            let Some(session_key) = row_session.clone() else {
                                                return;
                                            };
                                            row_state.update(cx, |state, cx| {
                                                state.set_pipeline_selected_stage(
                                                    &session_key,
                                                    Some(idx),
                                                );
                                                cx.notify();
                                            });
                                        }
                                    })
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .w_full()
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap(spacing::xs())
                                                    .child(
                                                        Switch::new(("agg-stage-enabled", idx))
                                                            .checked(stage_enabled)
                                                            .small()
                                                            .disabled(session_key.is_none())
                                                            .on_click({
                                                                move |_checked, _window, cx| {
                                                                    let Some(session_key) =
                                                                        stage_session.clone()
                                                                    else {
                                                                        return;
                                                                    };
                                                                    stage_state.update(
                                                                        cx,
                                                                        |state, cx| {
                                                                            state.toggle_pipeline_stage_enabled(
                                                                            &session_key,
                                                                            idx,
                                                                        );
                                                                            cx.notify();
                                                                        },
                                                                    );
                                                                }
                                                            }),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(colors::text_primary())
                                                            .child(format!(
                                                                "{}. {}",
                                                                idx + 1,
                                                                operator_label
                                                            )),
                                                    ),
                                            )
                                            .child(
                                                Button::new(("agg-stage-remove", idx))
                                                    .ghost()
                                                    .compact()
                                                    .icon(Icon::new(IconName::Delete).xsmall())
                                                    .disabled(session_key.is_none())
                                                    .on_click({
                                                        move |_: &ClickEvent,
                                                              _window: &mut Window,
                                                              cx: &mut App| {
                                                            let Some(session_key) =
                                                                remove_session.clone()
                                                            else {
                                                                return;
                                                            };
                                                            remove_state.update(cx, |state, cx| {
                                                                state.remove_pipeline_stage(
                                                                    &session_key,
                                                                    idx,
                                                                );
                                                                cx.notify();
                                                            });
                                                        }
                                                    }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(colors::text_muted())
                                            .child(count_label.unwrap_or_else(|| "—".to_string())),
                                    )
                                    .into_any_element()
                            })
                            .collect()
                    }
                }),
            )
            .flex_1()
            .p(spacing::sm())
            .track_scroll(scroll_handle),
        )
        .into_any_element()
}
