//! Aggregation pipeline stage list component.
//!
//! This module provides the stage list panel for the aggregation view, including:
//! - Stage list rendering with drag-and-drop reordering
//! - Stage row rendering with context menus
//! - Operator picker dialog
//! - Pipeline import dialog

mod dialogs;
mod stage_row;

use gpui::Styled as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};

use crate::components::Button;
use crate::state::app_state::PipelineState;
use crate::state::{SessionKey, StatusMessage};
use crate::theme::{borders, spacing};
use crate::views::CollectionView;

use super::operators::QUICK_START_OPERATORS;
use dialogs::{open_import_pipeline_dialog, open_stage_operator_picker_dialog};
use stage_row::{StageListView, render_stage_list};

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
            .bg(cx.theme().tab_bar)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().text_color(cx.theme().foreground).child("Stages"))
            .child({
                let state = self.state.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(
                        Button::new("agg-import-pipeline")
                            .compact()
                            .label("Import")
                            .tooltip("Import pipeline JSON")
                            .disabled(session_key.is_none())
                            .on_click({
                                let session_key = session_key.clone();
                                let state = state.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    let Some(session_key) = session_key.clone() else {
                                        return;
                                    };
                                    open_import_pipeline_dialog(
                                        window,
                                        cx,
                                        state.clone(),
                                        session_key,
                                    );
                                }
                            }),
                    )
                    .child(
                        Button::new("agg-add-stage")
                            .compact()
                            .icon(Icon::new(IconName::Plus).xsmall())
                            .label("Add Stage")
                            .tooltip("Add a pipeline stage")
                            .disabled(session_key.is_none())
                            .on_click({
                                let session_key = session_key.clone();
                                let state = state.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    let Some(session_key) = session_key.clone() else {
                                        return;
                                    };
                                    open_stage_operator_picker_dialog(
                                        window,
                                        cx,
                                        state.clone(),
                                        session_key,
                                        None,
                                    );
                                }
                            }),
                    )
            });

        let body = if pipeline.stages.is_empty() {
            render_empty_state(session_key.clone(), self.state.clone(), cx)
        } else {
            let view_ctx = StageListView {
                scroll_handle: self.aggregation_stage_list_scroll.clone(),
                focus_handle: self.aggregation_focus.clone(),
                view_entity: view.clone(),
                drag_over: self.aggregation_drag_over,
                drag_source: self.aggregation_drag_source,
            };
            render_stage_list(
                pipeline,
                session_key.clone(),
                self.state.clone(),
                pipeline.selected_stage,
                view_ctx,
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
            .bg(cx.theme().sidebar)
            .border_1()
            .border_color(cx.theme().sidebar_border)
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
                    let mut handled = false;
                    view.update(cx, |this, cx| {
                        handled = handle_stage_list_key(this, event, &session_key, cx);
                        if handled {
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

    let mut handled = false;
    match key.as_str() {
        "up" if cmd_or_ctrl && modifiers.shift => {
            let Some(selected) = pipeline.selected_stage else {
                return handled;
            };
            if selected == 0 {
                return handled;
            }
            view.state.update(cx, |state, cx| {
                state.move_pipeline_stage(session_key, selected, selected - 1);
                state.set_status_message(Some(StatusMessage::info("Stage moved up")));
                cx.notify();
            });
            handled = true;
        }
        "down" if cmd_or_ctrl && modifiers.shift => {
            let Some(selected) = pipeline.selected_stage else {
                return handled;
            };
            if selected + 1 >= count {
                return handled;
            }
            view.state.update(cx, |state, cx| {
                state.move_pipeline_stage(session_key, selected, selected + 1);
                state.set_status_message(Some(StatusMessage::info("Stage moved down")));
                cx.notify();
            });
            handled = true;
        }
        "up" if !modifiers.modified() => {
            let current = pipeline.selected_stage.unwrap_or(0);
            let next = current.saturating_sub(1);
            view.state.update(cx, |state, cx| {
                state.set_pipeline_selected_stage(session_key, Some(next));
                cx.notify();
            });
            handled = true;
        }
        "down" if !modifiers.modified() => {
            let current = pipeline.selected_stage.unwrap_or(0);
            let next = (current + 1).min(count.saturating_sub(1));
            view.state.update(cx, |state, cx| {
                state.set_pipeline_selected_stage(session_key, Some(next));
                cx.notify();
            });
            handled = true;
        }
        "d" if cmd_or_ctrl => {
            let Some(selected) = pipeline.selected_stage else {
                return handled;
            };
            view.state.update(cx, |state, cx| {
                state.duplicate_pipeline_stage(session_key, selected);
                state.set_status_message(Some(StatusMessage::info("Stage duplicated")));
                cx.notify();
            });
            handled = true;
        }
        _ => {}
    }
    handled
}

fn render_empty_state(
    session_key: Option<SessionKey>,
    state: Entity<crate::state::AppState>,
    cx: &App,
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
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No pipeline stages yet."),
        )
        .child(
            Button::new("agg-add-first-stage")
                .icon(Icon::new(IconName::Plus).xsmall())
                .label("Add your first stage")
                .disabled(session_key.is_none())
                .on_click({
                    let session_key = session_key.clone();
                    let state = state.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        open_stage_operator_picker_dialog(
                            window,
                            cx,
                            state.clone(),
                            session_key,
                            None,
                        );
                    }
                }),
        )
        .child(
            div().text_xs().text_color(cx.theme().muted_foreground).child("Common starting points"),
        )
        .child(div().flex().items_center().gap(spacing::xs()).children(quick_buttons))
        .into_any_element()
}
