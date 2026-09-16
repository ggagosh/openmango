//! Pipeline rail: header controls, stage rows, and the add-stage entry point.

mod dialogs;
mod stage_row;

use gpui_kit::component::button::{Button as MenuButton, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Selectable as _};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::*;

use crate::components::{Button, QueryLibraryDialog, QueryLibraryTarget};
use crate::keyboard::{AGGREGATION_STAGES_CONTEXT, AddAggregationStage, OpenQueryLibrary};
use crate::state::app_state::{PipelineState, StageStatsMode};
use crate::state::{AppState, SessionKey};
use crate::theme::{islands, spacing};
use crate::views::CollectionView;

pub(in crate::views::documents) use dialogs::{
    OperatorPick, open_import_pipeline_dialog, open_operator_picker,
};
use stage_row::render_stage_rows;

impl CollectionView {
    pub(in crate::views::documents) fn render_aggregation_stage_list(
        &self,
        pipeline: &PipelineState,
        session_key: Option<SessionKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let appearance = self.state.read(cx).settings.appearance.clone();
        let focused = self.aggregation_focus.contains_focused(window, cx);
        let border = if focused {
            cx.theme().ring
        } else {
            islands::panel_border(&appearance, cx).opacity(0.5)
        };

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::sm())
            .px(spacing::sm())
            .py(spacing::xs())
            .child(div().text_sm().child("Pipeline"))
            .child(pipeline_header_controls(pipeline, session_key.clone(), self.state.clone(), cx));

        let add_at = pipeline.selected_stage.map_or(pipeline.stages.len(), |index| index + 1);
        let add_stage = Button::new("agg-add-stage")
            .ghost()
            .xsmall()
            .w_full()
            .icon(Icon::new(IconName::Plus))
            .label("Add stage")
            .tooltip_with_action(
                "Add a stage after the selected one",
                &AddAggregationStage,
                Some("Documents Aggregation"),
            )
            .disabled(session_key.is_none())
            .on_click({
                let state = self.state.clone();
                let session_key = session_key.clone();
                move |_, window, cx| {
                    if let Some(session_key) = session_key.clone() {
                        open_operator_picker(
                            window,
                            cx,
                            state.clone(),
                            session_key,
                            OperatorPick::Insert(add_at),
                        );
                    }
                }
            });

        div()
            .id("agg-stage-rail")
            .key_context(AGGREGATION_STAGES_CONTEXT)
            .track_focus(&self.aggregation_focus)
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(islands::card_bg(&appearance, cx))
            .border_1()
            .border_color(border)
            .rounded(islands::radius_sm(&appearance))
            .on_mouse_down(MouseButton::Left, {
                let focus = self.aggregation_focus.clone();
                move |_, window, cx| window.focus(&focus, cx)
            })
            .child(header)
            .child(
                div()
                    .id("agg-stage-rows")
                    .role(Role::List)
                    .aria_label("Pipeline stages")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.aggregation_stage_list_scroll)
                    .px(spacing::xs())
                    .pb(spacing::sm())
                    .children(render_stage_rows(self, pipeline, session_key, focused, window, cx))
                    .child(div().pt(spacing::xs()).child(add_stage)),
            )
            .into_any_element()
    }
}

/// Stages/Text switch and the pipeline options menu, shared by both modes.
pub(super) fn pipeline_header_controls(
    pipeline: &PipelineState,
    session_key: Option<SessionKey>,
    state: Entity<AppState>,
    cx: &mut Context<CollectionView>,
) -> Div {
    let view = cx.entity();
    let text_mode = pipeline.text_mode;
    let disabled = session_key.is_none();
    let mode = ButtonGroup::new("agg-mode").xsmall().children([
        Button::new("agg-mode-stages")
            .label("Stages")
            .selected(!text_mode)
            .toggled(!text_mode)
            .disabled(disabled)
            .on_click({
                let view = view.clone();
                move |_, window, cx| {
                    view.update(cx, |view, cx| view.set_aggregation_text_mode(false, window, cx))
                }
            }),
        Button::new("agg-mode-text")
            .label("Text")
            .selected(text_mode)
            .toggled(text_mode)
            .tooltip("Edit the whole pipeline as text")
            .disabled(disabled)
            .on_click(move |_, window, cx| {
                view.update(cx, |view, cx| view.set_aggregation_text_mode(true, window, cx))
            }),
    ]);

    let counts_on = pipeline.stage_stats_mode.counts_enabled();
    let auto_run = pipeline.auto_run;
    let has_stages = !pipeline.stages.is_empty();
    let options = MenuButton::new("agg-pipeline-options")
        .ghost()
        .xsmall()
        .icon(Icon::new(IconName::Ellipsis))
        .tooltip("Pipeline options")
        .disabled(disabled)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu: PopupMenu, _, _| {
            let Some(session_key) = session_key.clone() else {
                return menu;
            };
            menu.item(PopupMenuItem::new("Count documents per stage").checked(counts_on).on_click(
                {
                    let state = state.clone();
                    let session_key = session_key.clone();
                    move |_, window, cx| {
                        let mode = if counts_on {
                            StageStatsMode::Off
                        } else {
                            StageStatsMode::CountsAndTiming
                        };
                        state.update(cx, |state, cx| {
                            state.set_pipeline_stage_stats_mode(&session_key, mode);
                            cx.notify();
                        });
                        if has_stages {
                            crate::views::documents::request_run_aggregation(
                                state.clone(),
                                session_key.clone(),
                                true,
                                window,
                                cx,
                            );
                        }
                    }
                },
            ))
            .item(PopupMenuItem::new("Run automatically after changes").checked(auto_run).on_click(
                {
                    let state = state.clone();
                    let session_key = session_key.clone();
                    move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.set_pipeline_auto_run(&session_key, !auto_run);
                            cx.notify();
                        });
                    }
                },
            ))
            .item(PopupMenuItem::separator())
            .item(PopupMenuItem::new("Import pipeline…").on_click({
                let state = state.clone();
                let session_key = session_key.clone();
                move |_, window, cx| {
                    open_import_pipeline_dialog(window, cx, state.clone(), session_key.clone());
                }
            }))
            .item(
                PopupMenuItem::new("Open query library")
                    .action(Box::new(OpenQueryLibrary))
                    .on_click({
                        let state = state.clone();
                        move |_, window, cx| {
                            QueryLibraryDialog::open(
                                state.clone(),
                                QueryLibraryTarget::Aggregation(session_key.clone()),
                                window,
                                cx,
                            );
                        }
                    }),
            )
        });

    div().flex().items_center().gap(spacing::xs()).child(mode).child(options)
}
