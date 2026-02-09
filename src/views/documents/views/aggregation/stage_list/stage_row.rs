//! Stage row rendering including drag/drop support, insert chips, and context menus.

use gpui::Styled as _;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Disableable as _;
use gpui_component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_component::switch::Switch;
use gpui_component::tooltip::Tooltip;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::components::{Button, open_confirm_dialog};
use crate::helpers::format_number;
use crate::keyboard::DeleteAggregationStage;
use crate::state::app_state::PipelineState;
use crate::state::{AppState, SessionKey, StatusMessage};
use crate::theme::{borders, spacing};
use crate::views::CollectionView;

use super::dialogs::open_stage_operator_picker_dialog;

#[derive(Clone)]
pub(super) struct DragStage {
    pub session_key: SessionKey,
    pub from_index: usize,
}

#[derive(Clone)]
pub(super) struct DragStagePreview;

impl Render for DragStagePreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(spacing::sm())
            .py(spacing::xs())
            .rounded(borders::radius_sm())
            .bg(cx.theme().primary)
            .text_color(cx.theme().background)
            .text_sm()
            .child("Moving stage...")
    }
}

#[derive(Clone)]
pub(super) struct StageListView {
    pub scroll_handle: UniformListScrollHandle,
    pub focus_handle: FocusHandle,
    pub view_entity: Entity<CollectionView>,
    pub drag_over: Option<(usize, bool)>,
    pub drag_source: Option<usize>,
}

pub(super) fn render_stage_list(
    pipeline: &PipelineState,
    session_key: Option<SessionKey>,
    state: Entity<AppState>,
    selected: Option<usize>,
    view: StageListView,
    cx: &mut Context<CollectionView>,
) -> AnyElement {
    let StageListView { scroll_handle, focus_handle, view_entity, drag_over, drag_source } = view;
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
                    move |view, range: std::ops::Range<usize>, _window, cx| {
                        let drag_source = view.aggregation_drag_source;
                        range
                            .map(|idx| {
                                render_stage_row(
                                    idx,
                                    &stages[idx],
                                    &stage_doc_counts,
                                    selected,
                                    item_count,
                                    drag_source,
                                    state.clone(),
                                    session_key.clone(),
                                    focus_handle.clone(),
                                    view_entity.clone(),
                                    cx,
                                )
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

#[derive(Clone)]
struct StageMeta {
    operator: String,
    enabled: bool,
}

#[allow(clippy::too_many_arguments)]
fn render_stage_row(
    idx: usize,
    stage: &StageMeta,
    stage_doc_counts: &[crate::state::app_state::StageDocCounts],
    selected: Option<usize>,
    item_count: usize,
    drag_source: Option<usize>,
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    focus_handle: FocusHandle,
    view_entity: Entity<CollectionView>,
    cx: &App,
) -> AnyElement {
    let is_selected = selected == Some(idx);
    let is_dimmed = selected.is_some_and(|selected| idx > selected);
    let count_label = stage_doc_counts
        .get(idx)
        .map(|counts| format_doc_counts(counts.input, counts.output, counts.time_ms))
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
    let drag_value =
        drag_session.clone().map(|session_key| DragStage { session_key, from_index: idx });
    let operator_label_for_delete = operator_label.clone();
    let operator_label_for_menu = operator_label.clone();
    let stage_number = idx + 1;

    let header_left = div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .child(drag_handle(idx, drag_value.clone(), handle_view_entity, cx))
        .child(
            Switch::new(("agg-stage-enabled", idx))
                .checked(stage_enabled)
                .small()
                .tooltip("Enable/disable stage (Cmd+Shift+E)")
                .disabled(session_key.is_none())
                .on_click({
                    move |_checked, _window, cx| {
                        let Some(session_key) = stage_session.clone() else {
                            return;
                        };
                        stage_state.update(cx, |state, cx| {
                            state.toggle_pipeline_stage_enabled(&session_key, idx);
                            cx.notify();
                        });
                    }
                }),
        )
        .child(div().text_sm().text_color(cx.theme().foreground).child(format!(
            "{}. {}",
            idx + 1,
            operator_label
        )));

    let header_row =
        div().flex().items_center().justify_between().w_full().child(header_left).child(
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
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let Some(session_key) = remove_session.clone() else {
                            return;
                        };
                        let session_key_for_delete = session_key.clone();
                        let message = format!(
                            "Delete Stage {} ({}). This cannot be undone.",
                            stage_number, operator_label_for_delete
                        );
                        open_confirm_dialog(window, cx, "Delete stage", message, "Delete", true, {
                            let remove_state = remove_state.clone();
                            move |_window, cx| {
                                remove_state.update(cx, |state, cx| {
                                    state.remove_pipeline_stage(&session_key_for_delete, idx);
                                    state.set_status_message(Some(StatusMessage::info(
                                        "Stage deleted",
                                    )));
                                    cx.notify();
                                });
                            }
                        });
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
        .border_color(crate::theme::colors::transparent())
        .when(is_selected, |s| s.bg(cx.theme().list_active).border_color(cx.theme().border))
        .when(!is_selected, |s| s.hover(|s| s.bg(cx.theme().list_hover)))
        .when(is_dimmed, |s| s.opacity(0.55))
        .when(is_drag_source, |s| s.opacity(0.4).border_0())
        .on_mouse_down(MouseButton::Left, {
            move |_, _window, cx| {
                let Some(session_key) = row_session.clone() else {
                    return;
                };
                row_state.update(cx, |state, cx| {
                    state.set_pipeline_selected_stage(&session_key, Some(idx));
                    cx.notify();
                });
            }
        })
        .child(header_row)
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(count_label));

    if let Some(drag_value) = drag_value.clone() {
        let session_key = drag_value.session_key.clone();
        row = row
            .can_drop({
                let session_key = session_key.clone();
                move |value, _window, _cx| {
                    value.downcast_ref::<DragStage>().is_some_and(|drag| {
                        drag.session_key == session_key && drag.from_index != idx
                    })
                }
            })
            .drag_over::<DragStage>({
                let session_key = session_key.clone();
                let view_entity = view_entity.clone();
                move |style, drag, _window, cx| {
                    let is_same_stage = drag.from_index == idx;
                    if drag.session_key != session_key || is_same_stage {
                        return style;
                    }
                    // Read the current drag_over state to determine top vs bottom
                    let insert_after = view_entity
                        .read(cx)
                        .aggregation_drag_over
                        .and_then(|(target, after)| (target == idx).then_some(after))
                        .unwrap_or(false);
                    // Use only the insert line, no other borders
                    let accent = cx.theme().primary;
                    if insert_after {
                        style.border_0().border_b_3().border_color(accent)
                    } else {
                        style.border_0().border_t_3().border_color(accent)
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
                        .and_then(|(target, after)| (target == idx).then_some(after))
                        .unwrap_or(false);
                    let insertion_index = if insert_after { idx + 1 } else { idx };
                    let to = compute_drop_target(from, insertion_index, item_count);
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
                        state.move_pipeline_stage(&session_key, from, to);
                        state.set_status_message(Some(StatusMessage::info("Stage reordered")));
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
                cx,
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
                cx,
            ))
            .opacity(0.0)
            .hover(|s| s.opacity(1.0));
        row = row.child(insert_after);
    }

    let row = row.context_menu(move |menu, _window, _cx| {
        let mut menu = menu.action_context(menu_focus.clone());
        let has_session = menu_session.is_some();
        let operator_label_for_delete = operator_label_for_menu.clone();

        let duplicate_item = menu_item("Duplicate", Some("⌘D")).disabled(!has_session);
        let delete_item = menu_item("Delete", Some("⌫")).disabled(!has_session);
        let toggle_label = if stage_enabled { "Disable" } else { "Enable" };
        let toggle_item = menu_item(toggle_label, Some("⌘⇧E")).disabled(!has_session);

        let move_up_item = menu_item("Move Up", Some("⌘⇧↑")).disabled(!has_session || idx == 0);
        let move_down_item =
            menu_item("Move Down", Some("⌘⇧↓")).disabled(!has_session || idx + 1 >= item_count);

        menu = menu
            .item(duplicate_item.on_click({
                let menu_state = menu_state.clone();
                let menu_session = menu_session.clone();
                move |_, _window, cx| {
                    let Some(session_key) = menu_session.clone() else {
                        return;
                    };
                    menu_state.update(cx, |state, cx| {
                        state.duplicate_pipeline_stage(&session_key, idx);
                        state.set_status_message(Some(StatusMessage::info("Stage duplicated")));
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
                        state.toggle_pipeline_stage_enabled(&session_key, idx);
                        let enabled = state
                            .session(&session_key)
                            .and_then(|session| session.data.aggregation.stages.get(idx))
                            .is_some_and(|stage| stage.enabled);
                        let message = if enabled { "Stage enabled" } else { "Stage disabled" };
                        state.set_status_message(Some(StatusMessage::info(message)));
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
                        state.move_pipeline_stage(&session_key, idx, idx - 1);
                        state.set_status_message(Some(StatusMessage::info("Stage moved up")));
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
                        state.move_pipeline_stage(&session_key, idx, idx + 1);
                        state.set_status_message(Some(StatusMessage::info("Stage moved down")));
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
                    open_confirm_dialog(window, cx, "Delete stage", message, "Delete", true, {
                        let menu_state = menu_state.clone();
                        move |_window, cx| {
                            menu_state.update(cx, |state, cx| {
                                state.remove_pipeline_stage(&session_key_for_delete, idx);
                                state
                                    .set_status_message(Some(StatusMessage::info("Stage deleted")));
                                cx.notify();
                            });
                        }
                    });
                }
            }));

        menu
    });

    row.into_any_element()
}

pub(super) fn format_doc_counts(
    input: Option<u64>,
    output: Option<u64>,
    time_ms: Option<u64>,
) -> String {
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
    insert_state: Entity<AppState>,
    insert_session: Option<SessionKey>,
    cx: &App,
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
        .border_color(cx.theme().sidebar_border)
        .bg(cx.theme().sidebar)
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::Plus).xsmall());

    if insert_session.is_none() {
        chip = chip.opacity(0.5).cursor_not_allowed();
    } else {
        chip = chip
            .cursor_pointer()
            .hover(|s| s.bg(cx.theme().list_hover).text_color(cx.theme().foreground))
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
    cx: &App,
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
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::ChevronsUpDown).xsmall());

    if let Some(drag_value) = drag_value {
        handle = handle
            .cursor_move()
            .hover(|s| s.bg(cx.theme().list_hover).text_color(cx.theme().foreground))
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

pub(super) fn compute_drop_target(from: usize, insertion_index: usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let capped = insertion_index.min(count);
    let mut to = if capped > from { capped.saturating_sub(1) } else { capped };
    to = to.min(count.saturating_sub(1));
    to
}

fn menu_item(label: &'static str, shortcut: Option<&'static str>) -> PopupMenuItem {
    PopupMenuItem::element(move |_window, cx| {
        let mut row = div().flex().items_center().justify_between().w_full().gap(spacing::lg());

        row = row.child(div().text_sm().child(label));

        if let Some(shortcut) = shortcut {
            row =
                row.child(div().text_xs().text_color(cx.theme().muted_foreground).child(shortcut));
        }

        row
    })
}
