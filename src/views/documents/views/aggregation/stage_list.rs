use gpui::Styled as _;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::Disableable as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputState};
use gpui_component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_component::scroll::ScrollableElement;
use gpui_component::switch::Switch;
use gpui_component::tooltip::Tooltip;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::components::{Button, open_confirm_dialog};
use crate::helpers::format_number;
use crate::keyboard::DeleteAggregationStage;
use crate::state::app_state::{PipelineStage, PipelineState};
use crate::state::{SessionKey, StatusMessage};
use crate::theme::{borders, colors, spacing};

use crate::views::CollectionView;

use super::operators::{OPERATOR_GROUPS, QUICK_START_OPERATORS};
use serde_json::Value as JsonValue;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Clone)]
struct DragStage {
    session_key: SessionKey,
    from_index: usize,
}

#[derive(Clone)]
struct DragStagePreview;

#[derive(Clone)]
struct StageListView {
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    view_entity: Entity<CollectionView>,
    drag_over: Option<(usize, bool)>,
    drag_source: Option<usize>,
}

#[derive(Default)]
struct OperatorPickerDialogState {
    focused_once: bool,
    last_query: String,
    auto_focused_operator: Option<String>,
}

#[derive(Default)]
struct ImportPipelineDialogState {
    focused_once: bool,
    error: Option<String>,
}

impl Render for DragStagePreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(spacing::sm())
            .py(spacing::xs())
            .rounded(borders::radius_sm())
            .bg(colors::accent())
            .text_color(colors::bg_app())
            .text_sm()
            .child("Moving stage...")
    }
}

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
                            .label("+ Add Stage")
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
            render_empty_state(session_key.clone(), self.state.clone())
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
        .child(div().text_xs().text_color(colors::text_muted()).child("Common starting points"))
        .child(div().flex().items_center().gap(spacing::xs()).children(quick_buttons))
        .into_any_element()
}

fn render_stage_list(
    pipeline: &PipelineState,
    session_key: Option<SessionKey>,
    state: Entity<crate::state::AppState>,
    selected: Option<usize>,
    view: StageListView,
    cx: &mut Context<CollectionView>,
) -> AnyElement {
    let StageListView { scroll_handle, focus_handle, view_entity, drag_over, drag_source } = view;
    #[derive(Clone)]
    struct StageMeta {
        operator: String,
        enabled: bool,
    }
    let stages: Vec<StageMeta> = pipeline
        .stages
        .iter()
        .map(|stage| StageMeta { operator: stage.operator.clone(), enabled: stage.enabled })
        .collect();
    let stage_doc_counts = pipeline.stage_doc_counts.clone();
    let item_count = stages.len();
    if !cx.has_active_drag() && (drag_over.is_some() || drag_source.is_some()) {
        let view_entity = view_entity.clone();
        cx.defer(move |cx| {
            let entity_id = view_entity.entity_id();
            let mut cleared = false;
            view_entity.update(cx, |this, _cx| {
                if this.aggregation_drag_over.is_some() {
                    this.aggregation_drag_over = None;
                    cleared = true;
                }
                if this.aggregation_drag_source.is_some() {
                    this.aggregation_drag_source = None;
                    cleared = true;
                }
            });
            if cleared {
                cx.notify(entity_id);
            }
        });
    }

    div()
        .id("agg-stage-list-container")
        .relative()
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
                    let focus_handle = focus_handle.clone();
                    let view_entity = view_entity.clone();
                    move |view, range: std::ops::Range<usize>, _window, _cx| {
                        let drag_source = view.aggregation_drag_source;
                        range
                            .map(|idx| {
                                let stage = &stages[idx];
                                let is_selected = selected == Some(idx);
                                let is_dimmed = selected.is_some_and(|selected| idx > selected);
                                let count_label = stage_doc_counts
                                    .get(idx)
                                    .map(|counts| {
                                        format_doc_counts(
                                            counts.input,
                                            counts.output,
                                            counts.time_ms,
                                        )
                                    })
                                    .unwrap_or_else(|| "—".to_string());
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
                                let insert_state = state.clone();
                                let insert_session = session_key.clone();
                                let menu_state = state.clone();
                                let menu_session = session_key.clone();
                                let menu_focus = focus_handle.clone();
                                let drag_state = state.clone();
                                let drag_session = session_key.clone();
                                let handle_view_entity = view_entity.clone();
                                let is_drag_source = drag_source == Some(idx);
                                let drag_value = drag_session
                                    .clone()
                                    .map(|session_key| DragStage { session_key, from_index: idx });
                                let operator_label_for_delete = operator_label.clone();
                                let operator_label_for_menu = operator_label.clone();
                                let stage_number = idx + 1;

                                let header_left = div()
                                    .flex()
                                    .items_center()
                                    .gap(spacing::xs())
                                    .child(drag_handle(idx, drag_value.clone(), handle_view_entity))
                                    .child(
                                        Switch::new(("agg-stage-enabled", idx))
                                            .checked(stage_enabled)
                                            .small()
                                            .tooltip("Enable/disable stage (Cmd+Shift+E)")
                                            .disabled(session_key.is_none())
                                            .on_click({
                                                move |_checked, _window, cx| {
                                                    let Some(session_key) = stage_session.clone()
                                                    else {
                                                        return;
                                                    };
                                                    stage_state.update(cx, |state, cx| {
                                                        state.toggle_pipeline_stage_enabled(
                                                            &session_key,
                                                            idx,
                                                        );
                                                        cx.notify();
                                                    });
                                                }
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(colors::text_primary())
                                            .child(format!("{}. {}", idx + 1, operator_label)),
                                    );

                                let header_row = div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .w_full()
                                    .child(header_left)
                                    .child(
                                        Button::new(("agg-stage-remove", idx))
                                            .ghost()
                                            .compact()
                                            .icon(Icon::new(IconName::Delete).xsmall())
                                            .tooltip_with_action(
                                                "Delete stage",
                                                &DeleteAggregationStage,
                                                Some("Documents Aggregation"),
                                            )
                                            .disabled(session_key.is_none())
                                            .on_click({
                                                move |_: &ClickEvent,
                                                      window: &mut Window,
                                                      cx: &mut App| {
                                                    let Some(session_key) = remove_session.clone()
                                                    else {
                                                        return;
                                                    };
                                                    let session_key_for_delete = session_key.clone();
                                                    let message = format!(
                                                        "Delete Stage {} ({}). This cannot be undone.",
                                                        stage_number, operator_label_for_delete
                                                    );
                                                    open_confirm_dialog(
                                                        window,
                                                        cx,
                                                        "Delete stage",
                                                        message,
                                                        "Delete",
                                                        true,
                                                        {
                                                            let remove_state = remove_state.clone();
                                                            move |_window, cx| {
                                                                remove_state.update(cx, |state, cx| {
                                                                    state.remove_pipeline_stage(
                                                                        &session_key_for_delete,
                                                                        idx,
                                                                    );
                                                                    state.set_status_message(Some(
                                                                        StatusMessage::info("Stage deleted"),
                                                                    ));
                                                                    cx.notify();
                                                                });
                                                            }
                                                        },
                                                    );
                                                }
                                            }),
                                    );

                                let mut row = div()
                                    .id(("agg-stage-row", idx))
                                    .relative()
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
                                    .when(is_dimmed, |s| s.opacity(0.55))
                                    .when(is_drag_source, |s| s.opacity(0.4).border_0())
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
                                    .child(header_row)
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(colors::text_muted())
                                            .child(count_label),
                                    );

                                if let Some(drag_value) = drag_value.clone() {
                                    let session_key = drag_value.session_key.clone();
                                    row = row
                                        .can_drop({
                                            let session_key = session_key.clone();
                                            move |value, _window, _cx| {
                                                value.downcast_ref::<DragStage>().is_some_and(
                                                    |drag| {
                                                        drag.session_key == session_key
                                                            && drag.from_index != idx
                                                    },
                                                )
                                            }
                                        })
                                        .drag_over::<DragStage>({
                                            let session_key = session_key.clone();
                                            let view_entity = view_entity.clone();
                                            move |style, drag, _window, cx| {
                                                let is_same_stage = drag.from_index == idx;
                                                if drag.session_key != session_key || is_same_stage
                                                {
                                                    return style;
                                                }
                                                // Read the current drag_over state to determine top vs bottom
                                                let insert_after = view_entity
                                                    .read(cx)
                                                    .aggregation_drag_over
                                                    .and_then(|(target, after)| {
                                                        (target == idx).then_some(after)
                                                    })
                                                    .unwrap_or(false);
                                                // Use only the insert line, no other borders
                                                let accent = colors::accent();
                                                if insert_after {
                                                    style
                                                        .border_0()
                                                        .border_b_3()
                                                        .border_color(accent)
                                                } else {
                                                    style
                                                        .border_0()
                                                        .border_t_3()
                                                        .border_color(accent)
                                                }
                                            }
                                        })
                                        .on_drag_move({
                                            let view_entity = view_entity.clone();
                                            let session_key = session_key.clone();
                                            move |event: &DragMoveEvent<DragStage>, _window, cx| {
                                                let drag = event.drag(cx);
                                                if drag.session_key != session_key {
                                                    return;
                                                }
                                                // Skip updating drag_over when hovering over source
                                                if drag.from_index == idx {
                                                    return;
                                                }
                                                let mid = event.bounds.center().y;
                                                let insert_after = event.event.position.y > mid;
                                                let entity_id = view_entity.entity_id();
                                                let mut changed = false;
                                                view_entity.update(cx, |this, _cx| {
                                                    let next = Some((idx, insert_after));
                                                    if this.aggregation_drag_over != next {
                                                        this.aggregation_drag_over = next;
                                                        changed = true;
                                                    }
                                                });
                                                if changed {
                                                    cx.notify(entity_id);
                                                }
                                            }
                                        })
                                        .on_drop({
                                            let drag_state = drag_state.clone();
                                            let session_key = session_key.clone();
                                            let view_entity = view_entity.clone();
                                            move |drag: &DragStage, _window, cx| {
                                                if drag.session_key != session_key {
                                                    return;
                                                }
                                                let from = drag.from_index;
                                                let insert_after = view_entity
                                                    .read(cx)
                                                    .aggregation_drag_over
                                                    .and_then(|(target, after)| {
                                                        (target == idx).then_some(after)
                                                    })
                                                    .unwrap_or(false);
                                                let insertion_index =
                                                    if insert_after { idx + 1 } else { idx };
                                                let to = compute_drop_target(
                                                    from,
                                                    insertion_index,
                                                    item_count,
                                                );
                                                if from == to {
                                                    let entity_id = view_entity.entity_id();
                                                    let mut cleared = false;
                                                    view_entity.update(cx, |this, _cx| {
                                                        if this.aggregation_drag_over.is_some() {
                                                            this.aggregation_drag_over = None;
                                                            cleared = true;
                                                        }
                                                    });
                                                    if cleared {
                                                        cx.notify(entity_id);
                                                    }
                                                    return;
                                                }
                                                drag_state.update(cx, |state, cx| {
                                                    state.move_pipeline_stage(
                                                        &session_key,
                                                        from,
                                                        to,
                                                    );
                                                    state.set_status_message(Some(
                                                        StatusMessage::info("Stage reordered"),
                                                    ));
                                                    cx.notify();
                                                });
                                                let entity_id = view_entity.entity_id();
                                                let mut cleared = false;
                                                view_entity.update(cx, |this, _cx| {
                                                    if this.aggregation_drag_over.is_some() {
                                                        this.aggregation_drag_over = None;
                                                        cleared = true;
                                                    }
                                                });
                                                if cleared {
                                                    cx.notify(entity_id);
                                                }
                                            }
                                        });
                                }

                                if idx > 0 {
                                    let insert_before = div()
                                        .absolute()
                                        .left(px(0.0))
                                        .right(px(0.0))
                                        .top(px(-9.0))
                                        .h(px(18.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(insert_chip(
                                            "agg-stage-insert-before",
                                            idx,
                                            idx,
                                            insert_state.clone(),
                                            insert_session.clone(),
                                        ))
                                        .opacity(0.0)
                                        .hover(|s| s.opacity(1.0));
                                    row = row.child(insert_before);
                                }

                                if idx + 1 == item_count {
                                    let insert_after = div()
                                        .absolute()
                                        .left(px(0.0))
                                        .right(px(0.0))
                                        .bottom(px(-9.0))
                                        .h(px(18.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(insert_chip(
                                            "agg-stage-insert-after",
                                            idx,
                                            idx + 1,
                                            insert_state,
                                            insert_session,
                                        ))
                                        .opacity(0.0)
                                        .hover(|s| s.opacity(1.0));
                                    row = row.child(insert_after);
                                }

                                let row = row.context_menu(move |menu, _window, _cx| {
                                    let mut menu = menu.action_context(menu_focus.clone());
                                    let has_session = menu_session.is_some();
                                    let operator_label_for_delete = operator_label_for_menu.clone();

                                    let duplicate_item =
                                        menu_item("Duplicate", Some("⌘D")).disabled(!has_session);
                                    let delete_item =
                                        menu_item("Delete", Some("⌫")).disabled(!has_session);
                                    let toggle_label =
                                        if stage_enabled { "Disable" } else { "Enable" };
                                    let toggle_item =
                                        menu_item(toggle_label, Some("⌘⇧E")).disabled(!has_session);

                                    let move_up_item = menu_item("Move Up", Some("⌘⇧↑"))
                                        .disabled(!has_session || idx == 0);
                                    let move_down_item = menu_item("Move Down", Some("⌘⇧↓"))
                                        .disabled(!has_session || idx + 1 >= item_count);

                                    menu = menu
                                        .item(duplicate_item.on_click({
                                            let menu_state = menu_state.clone();
                                            let menu_session = menu_session.clone();
                                            move |_, _window, cx| {
                                                let Some(session_key) = menu_session.clone() else {
                                                    return;
                                                };
                                                menu_state.update(cx, |state, cx| {
                                                    state.duplicate_pipeline_stage(
                                                        &session_key,
                                                        idx,
                                                    );
                                                    state.set_status_message(Some(
                                                        StatusMessage::info("Stage duplicated"),
                                                    ));
                                                    cx.notify();
                                                });
                                            }
                                        }))
                                        .item(toggle_item.on_click({
                                            let menu_state = menu_state.clone();
                                            let menu_session = menu_session.clone();
                                            move |_, _window, cx| {
                                                let Some(session_key) = menu_session.clone() else {
                                                    return;
                                                };
                                                menu_state.update(cx, |state, cx| {
                                                    state.toggle_pipeline_stage_enabled(
                                                        &session_key,
                                                        idx,
                                                    );
                                                    let enabled = state
                                                        .session(&session_key)
                                                        .and_then(|session| {
                                                            session.data.aggregation.stages.get(idx)
                                                        })
                                                        .is_some_and(|stage| stage.enabled);
                                                    let message =
                                                        if enabled { "Stage enabled" } else { "Stage disabled" };
                                                    state.set_status_message(Some(
                                                        StatusMessage::info(message),
                                                    ));
                                                    cx.notify();
                                                });
                                            }
                                        }))
                                        .item(PopupMenuItem::separator())
                                        .item(move_up_item.on_click({
                                            let menu_state = menu_state.clone();
                                            let menu_session = menu_session.clone();
                                            move |_, _window, cx| {
                                                let Some(session_key) = menu_session.clone() else {
                                                    return;
                                                };
                                                if idx == 0 {
                                                    return;
                                                }
                                                menu_state.update(cx, |state, cx| {
                                                    state.move_pipeline_stage(
                                                        &session_key,
                                                        idx,
                                                        idx - 1,
                                                    );
                                                    state.set_status_message(Some(
                                                        StatusMessage::info("Stage moved up"),
                                                    ));
                                                    cx.notify();
                                                });
                                            }
                                        }))
                                        .item(move_down_item.on_click({
                                            let menu_state = menu_state.clone();
                                            let menu_session = menu_session.clone();
                                            move |_, _window, cx| {
                                                let Some(session_key) = menu_session.clone() else {
                                                    return;
                                                };
                                                if idx + 1 >= item_count {
                                                    return;
                                                }
                                                menu_state.update(cx, |state, cx| {
                                                    state.move_pipeline_stage(
                                                        &session_key,
                                                        idx,
                                                        idx + 1,
                                                    );
                                                    state.set_status_message(Some(
                                                        StatusMessage::info("Stage moved down"),
                                                    ));
                                                    cx.notify();
                                                });
                                            }
                                        }))
                                        .item(PopupMenuItem::separator())
                                        .item(delete_item.on_click({
                                            let menu_state = menu_state.clone();
                                            let menu_session = menu_session.clone();
                                            move |_, window, cx| {
                                                let Some(session_key) = menu_session.clone() else {
                                                    return;
                                                };
                                                let session_key_for_delete = session_key.clone();
                                                let message = format!(
                                                    "Delete Stage {} ({}). This cannot be undone.",
                                                    stage_number, operator_label_for_delete
                                                );
                                                open_confirm_dialog(
                                                    window,
                                                    cx,
                                                    "Delete stage",
                                                    message,
                                                    "Delete",
                                                    true,
                                                    {
                                                        let menu_state = menu_state.clone();
                                                        move |_window, cx| {
                                                            menu_state.update(cx, |state, cx| {
                                                                state.remove_pipeline_stage(
                                                                    &session_key_for_delete,
                                                                    idx,
                                                                );
                                                                state.set_status_message(Some(
                                                                    StatusMessage::info("Stage deleted"),
                                                                ));
                                                                cx.notify();
                                                            });
                                                        }
                                                    },
                                                );
                                            }
                                        }));

                                    menu
                                });

                                row.into_any_element()
                            })
                            .collect()
                    }
                }),
            )
            .flex_1()
            .px(spacing::sm())
            .pt(spacing::sm())
            .pb(px(24.0))
            .track_scroll(scroll_handle),
        )
        .into_any_element()
}

fn open_stage_operator_picker_dialog(
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
                        .child(div().text_xs().text_color(colors::text_muted()).child(group.label))
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
                        .text_color(colors::text_secondary())
                        .child("Choose an operator"),
                )
                .child(Input::new(&search_state).w_full().tab_index(0))
                .child(if empty_results {
                    div()
                        .text_sm()
                        .text_color(colors::text_muted())
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

fn open_import_pipeline_dialog(
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
                        .text_color(colors::text_secondary())
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
                                .text_color(colors::text_muted())
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
                    this.child(div().text_sm().text_color(colors::text_error()).child(error))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(spacing::xs())
                        .child(Button::new("agg-import-cancel").label("Cancel").on_click(
                            |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                window.close_dialog(cx);
                            },
                        ))
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
    let value: JsonValue = serde_json::from_str(trimmed).map_err(|err| err.to_string())?;
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

fn session_key_id(session_key: &SessionKey) -> u64 {
    let mut hasher = DefaultHasher::new();
    session_key.hash(&mut hasher);
    hasher.finish()
}

fn format_doc_counts(input: Option<u64>, output: Option<u64>, time_ms: Option<u64>) -> String {
    if input.is_none() && output.is_none() {
        return "—".to_string();
    }

    let input_label = input.map(format_number).unwrap_or_else(|| "—".to_string());
    let output_label = output.map(format_number).unwrap_or_else(|| "—".to_string());
    let mut label = format!("{input_label} → {output_label} docs");
    if let Some(time_ms) = time_ms
        && time_ms > 0
    {
        label.push_str(&format!(" · {time_ms}ms"));
    }
    label
}

fn insert_chip(
    id_prefix: &'static str,
    idx: usize,
    insert_index: usize,
    insert_state: Entity<crate::state::AppState>,
    insert_session: Option<SessionKey>,
) -> AnyElement {
    let insert_enabled = insert_session.is_some();
    let mut chip = div()
        .id((id_prefix, idx))
        .flex()
        .items_center()
        .justify_center()
        .w(px(16.0))
        .h(px(16.0))
        .rounded(borders::radius_sm())
        .border_1()
        .border_color(colors::border_subtle())
        .bg(colors::bg_sidebar())
        .text_color(colors::text_muted())
        .child(Icon::new(IconName::Plus).xsmall());

    if insert_session.is_none() {
        chip = chip.opacity(0.5).cursor_not_allowed();
    } else {
        chip = chip
            .cursor_pointer()
            .hover(|s| s.bg(colors::list_hover()).text_color(colors::text_primary()))
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let Some(session_key) = insert_session.clone() else {
                    return;
                };
                open_stage_operator_picker_dialog(
                    window,
                    cx,
                    insert_state.clone(),
                    session_key,
                    Some(insert_index),
                );
            });
    }

    let tooltip_text = if insert_enabled { "Insert stage here" } else { "No active session" };
    chip = chip.tooltip(move |window, cx| Tooltip::new(tooltip_text).build(window, cx));

    chip.into_any_element()
}

fn drag_handle(
    idx: usize,
    drag_value: Option<DragStage>,
    view_entity: Entity<CollectionView>,
) -> AnyElement {
    let can_drag = drag_value.is_some();
    let mut handle = div()
        .id(("agg-stage-handle", idx))
        .flex()
        .items_center()
        .justify_center()
        .w(px(18.0))
        .h(px(18.0))
        .rounded(borders::radius_sm())
        .text_color(colors::text_muted())
        .child(Icon::new(IconName::ChevronsUpDown).xsmall());

    if let Some(drag_value) = drag_value {
        handle = handle
            .cursor_move()
            .hover(|s| s.bg(colors::list_hover()).text_color(colors::text_primary()))
            .on_drag(drag_value, move |_drag: &DragStage, _position, _window, cx| {
                cx.stop_propagation();
                let entity_id = view_entity.entity_id();
                view_entity.update(cx, |this, _cx| {
                    this.aggregation_drag_source = Some(idx);
                });
                cx.notify(entity_id);
                cx.new(|_| DragStagePreview)
            });
    } else {
        handle = handle.opacity(0.5).cursor_not_allowed();
    }

    let tooltip_text = if can_drag { "Drag to reorder" } else { "Drag unavailable" };
    handle = handle.tooltip(move |window, cx| Tooltip::new(tooltip_text).build(window, cx));

    handle.into_any_element()
}

fn compute_drop_target(from: usize, insertion_index: usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let capped = insertion_index.min(count);
    let mut to = if capped > from { capped.saturating_sub(1) } else { capped };
    to = to.min(count.saturating_sub(1));
    to
}

fn menu_item(label: &'static str, shortcut: Option<&'static str>) -> PopupMenuItem {
    PopupMenuItem::element(move |_window, _cx| {
        let mut row = div().flex().items_center().justify_between().w_full().gap(spacing::lg());

        row = row.child(div().text_sm().child(label));

        if let Some(shortcut) = shortcut {
            row = row.child(div().text_xs().text_color(colors::text_muted()).child(shortcut));
        }

        row
    })
}
