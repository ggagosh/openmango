use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::ResizeHandleRenderer;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{EditorState, InputEvent};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_kit::component::{Disableable as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::AppIcon;
use crate::bson::{format_relaxed_json_value, parse_value_from_relaxed_json};
use crate::components::{Button, QueryLibraryDialog, QueryLibraryTarget};
use crate::keyboard::{AGGREGATION_STAGES_CONTEXT, AddAggregationStage};
use crate::state::app_state::{PipelineState, parse_pipeline_text, pipeline_to_text};
use crate::state::{AppCommands, AppState, SessionKey};
use crate::theme::{islands, spacing};

use crate::views::CollectionView;

mod operators;
mod results_view;
mod stage_editor;
mod stage_list;
mod text_mode;

pub(in crate::views::documents) use stage_list::{OperatorPick, open_operator_picker};

use operators::QUICK_START_OPERATORS;
use stage_list::open_import_pipeline_dialog;

/// Wait after typing stops before an automatic run.
const AUTO_RUN_AFTER_EDIT: Duration = Duration::from_millis(700);
/// Short wait after moving the preview point, so arrowing through stages stays cheap.
const AUTO_RUN_AFTER_SELECT: Duration = Duration::from_millis(250);

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
        self.schedule_aggregation_auto_run(&pipeline, session_key.clone(), cx);

        let content = if pipeline.stages.is_empty() && !pipeline.text_mode {
            render_empty_pipeline(self.state.clone(), session_key, window, cx)
        } else if pipeline.text_mode {
            let editor = self.render_aggregation_text_mode(&pipeline, session_key.clone(), cx);
            let results = self.render_aggregation_results(&pipeline, session_key, window, cx);
            v_resizable("agg-text-split")
                .with_handle_appearance(split_handle(&self.state, cx))
                .child(
                    resizable_panel()
                        .size(px(320.0))
                        .size_range(px(160.0)..px(1200.0))
                        .child(panel_slot().pb(spacing::xs()).child(editor)),
                )
                .child(resizable_panel().child(panel_slot().pt(spacing::xs()).child(results)))
                .into_any_element()
        } else {
            let rail =
                self.render_aggregation_stage_list(&pipeline, session_key.clone(), window, cx);
            let editor =
                self.render_aggregation_stage_editor(&pipeline, session_key.clone(), window, cx);
            let results = self.render_aggregation_results(&pipeline, session_key, window, cx);
            let right = v_resizable("agg-stage-split")
                .with_handle_appearance(split_handle(&self.state, cx))
                .child(
                    resizable_panel()
                        .size(px(260.0))
                        .size_range(px(120.0)..px(1000.0))
                        .child(panel_slot().pb(spacing::xs()).child(editor)),
                )
                .child(resizable_panel().child(panel_slot().pt(spacing::xs()).child(results)));
            h_resizable("agg-rail-split")
                .with_handle_appearance(split_handle(&self.state, cx))
                .child(
                    resizable_panel()
                        .size(px(300.0))
                        .size_range(px(240.0)..px(520.0))
                        .child(panel_slot().pr(spacing::xs()).child(rail)),
                )
                .child(resizable_panel().child(panel_slot().pl(spacing::xs()).child(right)))
                .into_any_element()
        };

        let empty = pipeline.stages.is_empty() && !pipeline.text_mode;
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(cx.theme().background)
            .p(spacing::lg())
            // The rail owns this focus when it is shown; otherwise keep keys in this view.
            .when(pipeline.text_mode || empty, |view| view.track_focus(&self.aggregation_focus))
            .when(empty, |view| view.key_context(AGGREGATION_STAGES_CONTEXT))
            .child(content)
            .into_any_element()
    }

    fn ensure_aggregation_states(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.aggregation_stage_body_state.is_none() {
            let body_state = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("javascript")
                    .line_number(true)
                    .searchable(true)
                    .soft_wrap(true)
                    .placeholder("{ field: \"value\" }")
            });
            let subscription =
                cx.subscribe_in(&body_state, window, move |view, state, event, window, cx| {
                    match event {
                        // `set_value` from syncing emits no Change, so this is always the user.
                        InputEvent::Change => {
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
                            view.aggregation_body_revision = view.state.update(cx, |state, cx| {
                                state.set_pipeline_stage_body(&session_key, index, raw);
                                cx.notify();
                                pipeline_revision(state, &session_key)
                            });
                        }
                        InputEvent::PressEnter { secondary: true, .. } => {
                            if let Some(session_key) = view.view_model.current_session() {
                                crate::views::documents::request_run_aggregation(
                                    view.state.clone(),
                                    session_key,
                                    false,
                                    window,
                                    cx,
                                );
                            }
                        }
                        _ => {}
                    }
                });
            self.aggregation_stage_body_state = Some(body_state);
            self.aggregation_stage_body_subscription = Some(subscription);
        }

        if self.aggregation_text_state.is_none() {
            let text_state = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("javascript")
                    .line_number(true)
                    .searchable(true)
                    .soft_wrap(true)
                    .placeholder("[\n  { $match: { status: \"active\" } }\n]")
            });
            let subscription =
                cx.subscribe_in(&text_state, window, move |view, state, event, window, cx| {
                    match event {
                        InputEvent::Change => {
                            let Some(session_key) = view.view_model.current_session() else {
                                return;
                            };
                            let text = state.read(cx).value().to_string();
                            match parse_pipeline_text(&text) {
                                Ok(stages) => {
                                    view.aggregation_text_error = None;
                                    view.aggregation_text_revision =
                                        view.state.update(cx, |state, cx| {
                                            state.replace_pipeline_stages_from_text(
                                                &session_key,
                                                stages,
                                            );
                                            cx.notify();
                                            pipeline_revision(state, &session_key)
                                        });
                                }
                                Err(error) => {
                                    view.aggregation_text_error = Some(error);
                                    view.state.update(cx, |state, _| {
                                        state.set_pipeline_text_draft(&session_key, Some(text));
                                    });
                                }
                            }
                            cx.notify();
                        }
                        InputEvent::PressEnter { secondary: true, .. } => {
                            if let Some(session_key) = view.view_model.current_session() {
                                crate::views::documents::request_run_aggregation(
                                    view.state.clone(),
                                    session_key,
                                    false,
                                    window,
                                    cx,
                                );
                            }
                        }
                        _ => {}
                    }
                });
            self.aggregation_text_state = Some(text_state);
            self.aggregation_text_subscription = Some(subscription);
        }
    }

    fn sync_aggregation_inputs(
        &mut self,
        pipeline: &PipelineState,
        session_key: Option<SessionKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session_changed = self.aggregation_input_session != session_key;
        if session_changed {
            self.aggregation_input_session = session_key;
            self.aggregation_drag_over = None;
            self.aggregation_auto_run = None;
            self.aggregation_body_revision = None;
            self.aggregation_text_revision = None;
        }
        let stage_changed = session_changed
            || self.aggregation_selected_stage != pipeline.selected_stage
            || self.aggregation_stage_count != pipeline.stages.len();
        if stage_changed {
            self.aggregation_selected_stage = pipeline.selected_stage;
            if !session_changed && let Some(selected) = pipeline.selected_stage {
                // The preview divider sits before `selected` when it previews the stage input.
                let divider_before = pipeline.preview_target().map_or(0, |target| target + 1);
                let child = selected + usize::from(divider_before <= selected);
                self.aggregation_stage_list_scroll.scroll_to_item(child);
            }
        }
        self.aggregation_stage_count = pipeline.stages.len();

        // An editor follows the pipeline unless its own typing produced the current revision,
        // so undo, operator templates, and library restores show up even while it has focus.
        let revision = Some(pipeline.edit_revision);
        if let Some(body_state) = self.aggregation_stage_body_state.clone()
            && (stage_changed || self.aggregation_body_revision != revision)
        {
            let body = pipeline
                .selected_stage
                .and_then(|index| pipeline.stages.get(index))
                .map(|stage| stage.body.clone())
                .unwrap_or_default();
            if body_state.read(cx).value().as_ref() != body {
                body_state.update(cx, |state, cx| state.set_value(body, window, cx));
            }
            self.aggregation_body_revision = revision;
        }

        if pipeline.text_mode
            && self.aggregation_text_revision != revision
            && let Some(text_state) = self.aggregation_text_state.clone()
        {
            let (text, error) = match &pipeline.text_draft {
                Some(draft) => (draft.clone(), parse_pipeline_text(draft).err()),
                None => (pipeline_to_text(&pipeline.stages), None),
            };
            if text_state.read(cx).value().as_ref() != text {
                text_state.update(cx, |state, cx| state.set_value(text, window, cx));
            }
            self.aggregation_text_error = error;
            self.aggregation_text_revision = revision;
        }
    }

    /// Re-run a pipeline that already ran once, after it goes stale.
    fn schedule_aggregation_auto_run(
        &mut self,
        pipeline: &PipelineState,
        session_key: Option<SessionKey>,
        cx: &mut Context<Self>,
    ) {
        let Some(session_key) = session_key else {
            return;
        };
        let target = pipeline.preview_target();
        let key = (pipeline.edit_revision, target);
        let wanted = pipeline.auto_run
            && pipeline.is_stale()
            && !pipeline.loading
            && !pipeline.stages.is_empty()
            && !(pipeline.text_mode && self.aggregation_text_error.is_some())
            && crate::views::documents::aggregation_write_impact(
                &pipeline.stages,
                target,
                &session_key.database,
            )
            .is_none();
        if !wanted {
            self.aggregation_auto_run = None;
            return;
        }
        if self.aggregation_auto_run.as_ref().is_some_and(|(scheduled, _)| *scheduled == key) {
            return;
        }
        let edited = pipeline.last_run.is_some_and(|run| run.revision != pipeline.edit_revision);
        let delay = if edited { AUTO_RUN_AFTER_EDIT } else { AUTO_RUN_AFTER_SELECT };
        let state = self.state.clone();
        let task = cx.spawn(async move |view, cx| {
            cx.background_executor().timer(delay).await;
            let text_ok = view.update(cx, |view, _| view.aggregation_text_error.is_none());
            cx.update(|cx| {
                let still_wanted = state.read(cx).session(&session_key).is_some_and(|session| {
                    let pipeline = &session.data.aggregation;
                    pipeline.auto_run
                        && pipeline.is_stale()
                        && !pipeline.loading
                        && (pipeline.edit_revision, pipeline.preview_target()) == key
                });
                if still_wanted && text_ok.unwrap_or(false) {
                    AppCommands::run_aggregation(state, session_key, true, cx);
                }
            });
        });
        self.aggregation_auto_run = Some((key, task));
    }

    pub(in crate::views::documents) fn set_aggregation_text_mode(
        &mut self,
        text_mode: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session_key) = self.view_model.current_session() else {
            return;
        };
        let current =
            self.state.read(cx).session(&session_key).map(|s| s.data.aggregation.text_mode);
        if current == Some(text_mode) {
            return;
        }
        if !text_mode && let Some(error) = self.aggregation_text_error.clone() {
            window.push_notification(
                Notification::error(format!("Fix the pipeline text first. {error}")),
                cx,
            );
            return;
        }
        if text_mode {
            // The next render fills the editor from the stages.
            self.aggregation_text_revision = None;
            self.aggregation_text_error = None;
            if let Some(text_state) = self.aggregation_text_state.clone() {
                text_state.update(cx, |state, cx| state.focus(window, cx));
            }
        } else {
            window.focus(&self.aggregation_focus, cx);
        }
        self.state.update(cx, |state, cx| {
            state.set_pipeline_text_mode(&session_key, text_mode);
            cx.notify();
        });
    }
}

/// The gap between islands already separates them, so the split handle stays invisible
/// until hovered or dragged, like the sidebar handle.
fn split_handle(state: &Entity<AppState>, cx: &App) -> ResizeHandleRenderer {
    let border = islands::panel_border(&state.read(cx).settings.appearance, cx);
    Rc::new(move |handle, _, _| {
        let line = div()
            .flex_none()
            .rounded_full()
            .when(handle.is_active(), |line| line.bg(border.opacity(0.9)))
            .group_hover("handle", |line| line.bg(border.opacity(0.7)));
        Some(match handle.axis() {
            Axis::Horizontal => line.h_full().w(px(3.0)).ml(px(-1.0)).into_any_element(),
            Axis::Vertical => line.w_full().h(px(3.0)).mt(px(-1.0)).into_any_element(),
        })
    })
}

fn pipeline_revision(state: &AppState, session_key: &SessionKey) -> Option<u64> {
    state.session(session_key).map(|session| session.data.aggregation.edit_revision)
}

fn panel_slot() -> Div {
    div().flex().flex_1().min_w(px(0.0)).min_h(px(0.0)).overflow_hidden()
}

/// Canonical relaxed-JSON formatting for a stage body.
pub(in crate::views::documents) fn format_stage_body(raw: &str) -> Result<String, String> {
    parse_value_from_relaxed_json(raw).map(|value| format_relaxed_json_value(&value))
}

struct AggregationStageDeleted;

/// Delete a stage now and offer Undo, instead of asking first.
pub(in crate::views::documents) fn delete_aggregation_stage(
    state: &Entity<AppState>,
    session_key: &SessionKey,
    index: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(operator) = state
        .read(cx)
        .session(session_key)
        .and_then(|session| session.data.aggregation.stages.get(index))
        .map(|stage| stage.operator.clone())
    else {
        return;
    };
    let deleted = state.update(cx, |state, cx| {
        state.remove_pipeline_stage(session_key, index);
        cx.notify();
        state.pipeline_undo_top(session_key)
    });
    let state = state.clone();
    let session_key = session_key.clone();
    window.push_notification(
        Notification::new()
            .id::<AggregationStageDeleted>()
            .message(format!("Deleted stage {} · {operator}", index + 1))
            .action(move |_, _, cx| {
                let state = state.clone();
                let session_key = session_key.clone();
                Button::new("agg-undo-delete").xsmall().label("Undo").on_click(cx.listener(
                    move |notification, _, window, cx| {
                        state.update(cx, |state, cx| {
                            // Only undo the delete itself, not a later edit.
                            if deleted.is_some() && state.pipeline_undo_top(&session_key) == deleted
                            {
                                state.undo_pipeline_edit(&session_key);
                                cx.notify();
                            }
                        });
                        notification.dismiss(window, cx);
                    },
                ))
            })
            .autohide(true),
        cx,
    );
}

const EMPTY_PIPELINE_HINT: &str =
    "Stages run in order. Each one reshapes the documents from the stage before it.";

fn render_empty_pipeline(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    window: &mut Window,
    cx: &mut Context<CollectionView>,
) -> AnyElement {
    let view = cx.entity();
    let disabled = session_key.is_none();
    let muted = cx.theme().muted_foreground;
    let add_shortcut =
        Kbd::binding_for_action(&AddAggregationStage, Some("Documents Aggregation"), window);
    let quick_starts = QUICK_START_OPERATORS.iter().map(|operator| {
        let state = state.clone();
        let session_key = session_key.clone();
        let view = view.clone();
        Button::new(SharedString::from(format!("agg-quick-{operator}")))
            .xsmall()
            .outline()
            .label(*operator)
            .disabled(disabled)
            .on_click(move |_, window, cx| {
                let Some(session_key) = session_key.clone() else {
                    return;
                };
                state.update(cx, |state, cx| {
                    state.add_pipeline_stage(&session_key, *operator);
                    cx.notify();
                });
                // The template is ready to edit, so start typing in it.
                if let Some(body) = view.read(cx).aggregation_stage_body_state.clone() {
                    body.update(cx, |body, cx| body.focus(window, cx));
                }
            })
    });

    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(spacing::md())
                .max_w(px(420.0))
                .text_center()
                .child(Icon::new(AppIcon::Workflow).size(px(28.0)).text_color(muted))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .child(div().text_base().child("Build an aggregation pipeline"))
                        .child(div().text_sm().text_color(muted).child(EMPTY_PIPELINE_HINT)),
                )
                .child(
                    Button::new("agg-add-first-stage")
                        .primary()
                        .small()
                        .icon(Icon::new(IconName::Plus))
                        .label("Add stage")
                        .when_some(add_shortcut, |button, kbd| button.child(kbd.appearance(false)))
                        .disabled(disabled)
                        .on_click({
                            let state = state.clone();
                            let session_key = session_key.clone();
                            move |_, window, cx| {
                                if let Some(session_key) = session_key.clone() {
                                    open_operator_picker(
                                        window,
                                        cx,
                                        state.clone(),
                                        session_key,
                                        OperatorPick::Insert(0),
                                    );
                                }
                            }
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::xs())
                        .child(div().text_xs().text_color(muted).child("Start with"))
                        .children(quick_starts),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::xs())
                        .child(
                            Button::new("agg-empty-import")
                                .ghost()
                                .xsmall()
                                .label("Import pipeline")
                                .disabled(disabled)
                                .on_click({
                                    let state = state.clone();
                                    let session_key = session_key.clone();
                                    move |_, window, cx| {
                                        if let Some(session_key) = session_key.clone() {
                                            open_import_pipeline_dialog(
                                                window,
                                                cx,
                                                state.clone(),
                                                session_key,
                                            );
                                        }
                                    }
                                }),
                        )
                        .child(
                            Button::new("agg-empty-text")
                                .ghost()
                                .xsmall()
                                .label("Write as text")
                                .disabled(disabled)
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |view, cx| {
                                        view.set_aggregation_text_mode(true, window, cx)
                                    });
                                }),
                        )
                        .child(
                            Button::new("agg-empty-library")
                                .ghost()
                                .xsmall()
                                .label("Open library")
                                .disabled(disabled)
                                .on_click(move |_, window, cx| {
                                    if let Some(session_key) = session_key.clone() {
                                        QueryLibraryDialog::open(
                                            state.clone(),
                                            QueryLibraryTarget::Aggregation(session_key),
                                            window,
                                            cx,
                                        );
                                    }
                                }),
                        ),
                ),
        )
        .into_any_element()
}
