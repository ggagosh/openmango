//! Aggregation pipeline stage list component.
//!
//! This module provides the stage list panel for the aggregation view, including:
//! - Stage list rendering with drag-and-drop reordering
//! - Stage row rendering with context menus
//! - Operator picker dialog
//! - Pipeline import dialog

mod dialogs;
mod stage_row;

use gpui_kit::Styled as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::*;

use crate::components::{Button, QueryLibraryDialog, QueryLibraryTarget};
use crate::state::SessionKey;
use crate::state::app_state::PipelineState;
use crate::theme::{islands, spacing};
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
        let appearance = self.state.read(cx).settings.appearance.clone();
        let panel_bg = islands::card_bg(&appearance, cx);
        let panel_border = islands::panel_border(&appearance, cx).opacity(0.5);
        let panel_radius = islands::radius_sm(&appearance);
        let header_bg = cx.theme().transparent;
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(spacing::sm())
            .py(spacing::xs())
            .bg(header_bg)
            .child(div().text_sm().text_color(cx.theme().foreground).child("Stages"))
            .child({
                let state = self.state.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(
                        Button::new("aggregation-query-library")
                            .xsmall()
                            .icon(Icon::new(IconName::BookOpen).xsmall())
                            .label("Library")
                            .tooltip("Query Library (Cmd/Ctrl+Shift+H)")
                            .disabled(session_key.is_none())
                            .on_click({
                                let session_key = session_key.clone();
                                let state = state.clone();
                                move |_, window, cx| {
                                    let Some(session_key) = session_key.clone() else {
                                        return;
                                    };
                                    QueryLibraryDialog::open(
                                        state.clone(),
                                        QueryLibraryTarget::Aggregation(session_key),
                                        window,
                                        cx,
                                    );
                                }
                            }),
                    )
                    .child(
                        Button::new("agg-import-pipeline")
                            .xsmall()
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
                            .xsmall()
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
            .bg(panel_bg)
            .border_1()
            .border_color(panel_border)
            .rounded(panel_radius)
            .track_focus(&self.aggregation_focus)
            .on_mouse_down(MouseButton::Left, {
                let focus = self.aggregation_focus.clone();
                move |_, window, cx| {
                    window.focus(&focus, cx);
                }
            })
            .child(header)
            .child(body)
            .into_any_element()
    }
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
                .xsmall()
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
