//! Stage rows: selection, drag reordering, preview cutoff, and the row context menu.

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::AppIcon;
use crate::components::Button;
use crate::helpers::format_number;
use crate::keyboard::{
    AGGREGATION_STAGES_CONTEXT, DeleteAggregationStage, DuplicateAggregationStage,
    MoveAggregationStageDown, MoveAggregationStageUp, ToggleAggregationStageEnabled,
};
use crate::state::app_state::{PipelineState, StageDocCounts};
use crate::state::{AppState, SessionKey};
use crate::theme::{borders, spacing};
use crate::views::CollectionView;
use crate::views::documents::views::aggregation::delete_aggregation_stage;

use super::dialogs::{OperatorPick, open_operator_picker};

#[derive(Clone)]
pub(super) struct DragStage {
    pub session_key: SessionKey,
    pub from_index: usize,
}

struct DragStagePreview(SharedString);

impl Render for DragStagePreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(spacing::sm())
            .py(spacing::xs())
            .rounded(borders::radius_sm())
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .text_sm()
            .child(self.0.clone())
    }
}

pub(super) fn render_stage_rows(
    view: &CollectionView,
    pipeline: &PipelineState,
    session_key: Option<SessionKey>,
    focused: bool,
    window: &mut Window,
    cx: &mut Context<CollectionView>,
) -> Vec<AnyElement> {
    let view_entity = cx.entity();
    if !cx.has_active_drag()
        && (view.aggregation_drag_over.is_some() || view.aggregation_drag_source.is_some())
    {
        let view_entity = view_entity.clone();
        cx.defer(move |cx| {
            view_entity.update(cx, |view, cx| {
                view.aggregation_drag_over = None;
                view.aggregation_drag_source = None;
                cx.notify();
            });
        });
    }

    let count = pipeline.stages.len();
    let divider_at = pipeline.preview_target().map_or(0, |target| target + 1);
    let stale = pipeline.is_stale();
    let mut rows = Vec::with_capacity(count + 1);
    for (idx, stage) in pipeline.stages.iter().enumerate() {
        if idx == divider_at {
            rows.push(preview_divider(cx));
        }
        let row = StageRow {
            id: stage.id,
            idx,
            count,
            operator: stage.operator.trim().to_string(),
            enabled: stage.enabled,
            counts: pipeline.stage_doc_counts.get(idx).cloned().unwrap_or_default(),
            counts_enabled: pipeline.stage_stats_mode.counts_enabled(),
            failed: pipeline.error.is_some() && pipeline.error_stage == Some(idx) && !stale,
            blocked_by: pipeline
                .error_stage
                .filter(|failed| pipeline.error.is_some() && !stale && idx > *failed)
                .filter(|_| pipeline.preview_target().is_some_and(|target| idx <= target)),
            selected: pipeline.selected_stage == Some(idx),
            focused,
            stale,
            drag_over: view.aggregation_drag_over.filter(|(target, _)| *target == idx),
            is_drag_source: view.aggregation_drag_source == Some(idx),
        };
        rows.push(render_stage_row(
            row,
            view.state.clone(),
            session_key.clone(),
            view.aggregation_focus.clone(),
            view_entity.clone(),
            window,
            cx,
        ));
    }
    rows
}

fn preview_divider(cx: &App) -> AnyElement {
    div()
        .px(spacing::sm())
        .py(px(2.0))
        .child(
            Separator::horizontal_dashed()
                .label("Preview stops here")
                .color(cx.theme().muted_foreground.opacity(0.5)),
        )
        .into_any_element()
}

struct StageRow {
    /// The stage's own id. `idx` is where it sits now, which a drag changes.
    id: u64,
    idx: usize,
    count: usize,
    operator: String,
    enabled: bool,
    counts: StageDocCounts,
    counts_enabled: bool,
    failed: bool,
    /// An earlier stage failed, so this one never ran.
    blocked_by: Option<usize>,
    selected: bool,
    focused: bool,
    stale: bool,
    drag_over: Option<(usize, bool)>,
    is_drag_source: bool,
}

fn render_stage_row(
    row: StageRow,
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    focus_handle: FocusHandle,
    view_entity: Entity<CollectionView>,
    _window: &mut Window,
    cx: &App,
) -> AnyElement {
    let StageRow { idx, count, enabled, selected, failed, .. } = row;
    let theme = cx.theme();
    let operator =
        if row.operator.is_empty() { "No operator".to_string() } else { row.operator.clone() };
    let number = idx + 1;
    let has_session = session_key.is_some();

    let status: Option<(SharedString, Hsla)> = if failed {
        Some(("Failed".into(), theme.danger))
    } else if let Some(failed_stage) = row.blocked_by.filter(|_| enabled) {
        Some((
            format!("Didn't run · stage {} failed", failed_stage + 1).into(),
            theme.muted_foreground,
        ))
    } else if !enabled {
        Some(("Skipped".into(), theme.muted_foreground))
    } else if row.counts_enabled {
        Some((counts_label(&row.counts).into(), theme.muted_foreground))
    } else {
        None
    };
    let accessible = format!(
        "Stage {number}, {operator}{}{}",
        if enabled { "" } else { ", skipped" },
        if failed { ", failed" } else { "" }
    );

    let controls = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(spacing::xs())
        .h(px(24.0))
        .child(drag_handle(idx, &operator, session_key.clone(), view_entity.clone(), cx))
        .child(
            Switch::new(("agg-stage-enabled", idx))
                .checked(enabled)
                .xsmall()
                .accessibility_label(format!("Include stage {number}"))
                .tooltip("Include this stage (Space)")
                .disabled(!has_session)
                .on_click({
                    let state = state.clone();
                    let session_key = session_key.clone();
                    move |_, _, cx| {
                        if let Some(session_key) = session_key.clone() {
                            state.update(cx, |state, cx| {
                                state.toggle_pipeline_stage_enabled(&session_key, idx);
                                cx.notify();
                            });
                        }
                    }
                }),
        );

    let title = div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .h(px(24.0))
        .min_w(px(0.0))
        .child(
            div()
                .w(px(18.0))
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(number.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_sm()
                .when(!enabled, |label| label.text_color(theme.muted_foreground).line_through())
                .child(operator.clone()),
        )
        .when(failed, |header| {
            header.child(Icon::new(IconName::TriangleAlert).xsmall().text_color(theme.danger))
        })
        .child(
            Button::new("agg-stage-remove")
                .ghost()
                .xsmall()
                .icon(Icon::new(AppIcon::Trash))
                .accessibility_label(format!("Delete stage {number}"))
                .tooltip_with_action(
                    "Delete stage",
                    &DeleteAggregationStage,
                    Some(AGGREGATION_STAGES_CONTEXT),
                )
                .disabled(!has_session)
                .on_click({
                    let state = state.clone();
                    let session_key = session_key.clone();
                    move |_, window, cx| {
                        if let Some(session_key) = session_key.clone() {
                            delete_aggregation_stage(&state, &session_key, idx, window, cx);
                        }
                    }
                }),
        );

    let mut element = div()
        // Keyed by stage, not position: the row is dragged to reorder, and a position key would
        // change under the drag the moment it lands. The handle and remove button inside are
        // scoped by this id.
        .id(("agg-stage-row", row.id))
        .role(Role::ListItem)
        .aria_label(accessible)
        .aria_selected(selected)
        .relative()
        .flex()
        .gap(spacing::xs())
        .px(spacing::xs())
        .py(px(5.0))
        .rounded(borders::radius_sm())
        .when(selected, |el| {
            el.bg(if row.focused { theme.list_active } else { theme.list_active.opacity(0.55) })
        })
        .when(!selected, |el| el.hover(|el| el.bg(theme.list_hover)))
        .when(row.is_drag_source, |el| el.opacity(0.4))
        .on_mouse_down(MouseButton::Left, {
            let state = state.clone();
            let session_key = session_key.clone();
            move |_, _, cx| {
                if let Some(session_key) = session_key.clone() {
                    state.update(cx, |state, cx| {
                        state.set_pipeline_selected_stage(&session_key, Some(idx));
                        cx.notify();
                    });
                }
            }
        })
        .child(controls)
        .child(div().flex().flex_col().flex_1().min_w(px(0.0)).child(title).when_some(
            status,
            |content, (text, color)| {
                content.child(
                    div()
                        .pl(px(22.0))
                        .text_xs()
                        .truncate()
                        .text_color(color)
                        .when(row.stale && !failed, |text| text.opacity(0.6))
                        .child(text),
                )
            },
        ))
        .when_some(row.drag_over, |el, (_, after)| {
            let line = div().absolute().left_0().right_0().h(px(2.0)).bg(theme.primary);
            el.child(if after { line.bottom(px(-1.0)) } else { line.top(px(-1.0)) })
        });

    if let Some(session_key) = session_key.clone() {
        element = element
            .can_drop({
                let session_key = session_key.clone();
                move |value, _, _| {
                    value.downcast_ref::<DragStage>().is_some_and(|drag| {
                        drag.session_key == session_key && drag.from_index != idx
                    })
                }
            })
            .on_drag_move({
                let view_entity = view_entity.clone();
                let session_key = session_key.clone();
                move |event: &DragMoveEvent<DragStage>, _, cx| {
                    let drag = event.drag(cx);
                    if drag.session_key != session_key
                        || drag.from_index == idx
                        || !event.bounds.contains(&event.event.position)
                    {
                        return;
                    }
                    let next = Some((idx, event.event.position.y > event.bounds.center().y));
                    view_entity.update(cx, |view, cx| {
                        if view.aggregation_drag_over != next {
                            view.aggregation_drag_over = next;
                            cx.notify();
                        }
                    });
                }
            })
            .on_drop({
                let state = state.clone();
                let view_entity = view_entity.clone();
                move |drag: &DragStage, _, cx| {
                    if drag.session_key != session_key {
                        return;
                    }
                    let after = view_entity
                        .read(cx)
                        .aggregation_drag_over
                        .and_then(|(target, after)| (target == idx).then_some(after))
                        .unwrap_or(false);
                    let to = compute_drop_target(drag.from_index, idx + usize::from(after), count);
                    if to != drag.from_index {
                        state.update(cx, |state, cx| {
                            state.move_pipeline_stage(&session_key, drag.from_index, to);
                            cx.notify();
                        });
                    }
                    view_entity.update(cx, |view, cx| {
                        view.aggregation_drag_over = None;
                        view.aggregation_drag_source = None;
                        cx.notify();
                    });
                }
            });
    }

    element
        .context_menu(move |menu, _, _| {
            let Some(session_key) = session_key.clone() else {
                return menu;
            };
            let insert = |label: &'static str, at: usize| {
                let state = state.clone();
                let session_key = session_key.clone();
                PopupMenuItem::new(label).on_click(move |_, window, cx| {
                    open_operator_picker(
                        window,
                        cx,
                        state.clone(),
                        session_key.clone(),
                        OperatorPick::Insert(at),
                    );
                })
            };
            let edit = |label: &'static str,
                        action: Box<dyn Action>,
                        apply: fn(&mut AppState, &SessionKey, usize)| {
                let state = state.clone();
                let session_key = session_key.clone();
                PopupMenuItem::new(label).action(action).on_click(move |_, _, cx| {
                    state.update(cx, |state, cx| {
                        apply(state, &session_key, idx);
                        cx.notify();
                    });
                })
            };
            menu.action_context(focus_handle.clone())
                .item(insert("Add stage before", idx))
                .item(insert("Add stage after", idx + 1))
                .item(PopupMenuItem::new("Change operator…").on_click({
                    let state = state.clone();
                    let session_key = session_key.clone();
                    move |_, window, cx| {
                        open_operator_picker(
                            window,
                            cx,
                            state.clone(),
                            session_key.clone(),
                            OperatorPick::Replace(idx),
                        );
                    }
                }))
                .item(PopupMenuItem::separator())
                .item(edit("Duplicate", Box::new(DuplicateAggregationStage), |s, k, i| {
                    s.duplicate_pipeline_stage(k, i);
                }))
                .item(edit(
                    if enabled { "Skip stage" } else { "Include stage" },
                    Box::new(ToggleAggregationStageEnabled),
                    |s, k, i| s.toggle_pipeline_stage_enabled(k, i),
                ))
                .item(PopupMenuItem::separator())
                .item(
                    edit("Move up", Box::new(MoveAggregationStageUp), |s, k, i| {
                        s.move_pipeline_stage(k, i, i.saturating_sub(1))
                    })
                    .disabled(idx == 0),
                )
                .item(
                    edit("Move down", Box::new(MoveAggregationStageDown), |s, k, i| {
                        s.move_pipeline_stage(k, i, i + 1)
                    })
                    .disabled(idx + 1 >= count),
                )
                .item(PopupMenuItem::separator())
                .item(
                    PopupMenuItem::new("Delete").action(Box::new(DeleteAggregationStage)).on_click(
                        {
                            let state = state.clone();
                            move |_, window, cx| {
                                delete_aggregation_stage(&state, &session_key, idx, window, cx);
                            }
                        },
                    ),
                )
        })
        .into_any_element()
}

fn counts_label(counts: &StageDocCounts) -> String {
    let (Some(input), Some(output)) = (counts.input, counts.output) else {
        return "Not counted yet".to_string();
    };
    let mut label = format!("{} → {}", format_number(input), format_number(output));
    // Each count re-runs every stage up to this one, so the time is cumulative.
    if let Some(ms) = counts.time_ms.filter(|ms| *ms > 0) {
        label.push_str(&format!(" · {ms} ms to here"));
    }
    label
}

fn drag_handle(
    idx: usize,
    operator: &str,
    session_key: Option<SessionKey>,
    view_entity: Entity<CollectionView>,
    cx: &App,
) -> AnyElement {
    let handle = div()
        .id("agg-stage-handle")
        .flex()
        .items_center()
        .justify_center()
        .w(px(16.0))
        .h(px(20.0))
        .rounded(borders::radius_sm())
        .text_color(cx.theme().muted_foreground);
    let Some(session_key) = session_key else {
        return handle.child(Icon::new(AppIcon::GripVertical).xsmall()).into_any_element();
    };
    let label = SharedString::from(format!("{} · {operator}", idx + 1));
    handle
        .cursor_grab()
        .hover(|el| el.text_color(cx.theme().foreground))
        .child(Icon::new(AppIcon::GripVertical).xsmall())
        .on_drag(DragStage { session_key, from_index: idx }, move |_, _, _, cx| {
            cx.stop_propagation();
            view_entity.update(cx, |view, cx| {
                view.aggregation_drag_source = Some(idx);
                cx.notify();
            });
            cx.new(|_| DragStagePreview(label.clone()))
        })
        .into_any_element()
}

pub(super) fn compute_drop_target(from: usize, insertion_index: usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let capped = insertion_index.min(count);
    let to = if capped > from { capped.saturating_sub(1) } else { capped };
    to.min(count.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::compute_drop_target;

    #[test]
    fn drop_target_accounts_for_the_removed_source() {
        assert_eq!(compute_drop_target(0, 3, 3), 2);
        assert_eq!(compute_drop_target(2, 0, 3), 0);
        assert_eq!(compute_drop_target(1, 2, 3), 1);
        assert_eq!(compute_drop_target(0, 0, 0), 0);
    }
}
