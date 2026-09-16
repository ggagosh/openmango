use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::Editor;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{Button, ErrorCallout};
use crate::error::{ErrorKind, ErrorReport, sentence};
use crate::keyboard::{ClearAggregationStage, FormatAggregationStage};
use crate::state::SessionKey;
use crate::state::app_state::PipelineState;
use crate::theme::{islands, spacing};
use crate::views::CollectionView;

use super::{OperatorPick, format_stage_body, open_operator_picker};

impl CollectionView {
    pub(in crate::views::documents) fn render_aggregation_stage_editor(
        &mut self,
        pipeline: &PipelineState,
        session_key: Option<SessionKey>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let appearance = self.state.read(cx).settings.appearance.clone();
        let muted = cx.theme().muted_foreground;
        let selected = pipeline
            .selected_stage
            .and_then(|index| pipeline.stages.get(index).map(|stage| (index, stage.clone())));

        let Some((index, stage)) = selected else {
            return panel(&appearance, cx)
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(muted)
                .child("Select a stage to edit it")
                .into_any_element();
        };

        let operator = stage.operator.trim().to_string();
        let operator_button = Button::new("agg-operator")
            .outline()
            .xsmall()
            .label(if operator.is_empty() { "Choose operator".to_string() } else { operator })
            .icon(Icon::new(IconName::ChevronsUpDown))
            .tooltip("Change operator")
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
                            OperatorPick::Replace(index),
                        );
                    }
                }
            });

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::sm())
            .px(spacing::sm())
            .py(spacing::xs())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(div().text_sm().child(format!("Stage {}", index + 1)))
                    .child(operator_button)
                    .when(!stage.enabled, |row| {
                        row.child(Tag::secondary().xsmall().child("Skipped"))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(
                        Button::new("agg-format-stage")
                            .ghost()
                            .xsmall()
                            .label("Format")
                            .tooltip_with_action(
                                "Format stage",
                                &FormatAggregationStage,
                                Some("Documents Aggregation"),
                            )
                            .on_click(cx.listener(|view, _, window, cx| {
                                let Some(body_state) = view.aggregation_stage_body_state.clone()
                                else {
                                    return;
                                };
                                let raw = body_state.read(cx).value().to_string();
                                match format_stage_body(&raw) {
                                    Ok(formatted) => body_state.update(cx, |state, cx| {
                                        state.replace_all(formatted, window, cx);
                                    }),
                                    Err(error) => {
                                        view.aggregation_format_error = Some(error);
                                        cx.notify();
                                    }
                                }
                            })),
                    )
                    .child(
                        Button::new("agg-clear-stage")
                            .ghost()
                            .xsmall()
                            .label("Clear")
                            .tooltip_with_action("Clear stage", &ClearAggregationStage, None)
                            .on_click({
                                let body_state = self.aggregation_stage_body_state.clone();
                                move |_, window, cx| {
                                    if let Some(body_state) = body_state.clone() {
                                        body_state.update(cx, |state, cx| {
                                            state.replace_all("{}", window, cx);
                                        });
                                    }
                                }
                            }),
                    ),
            );

        // A Format problem is about the text in front of you, so it wins over the last run's error.
        let error = match self.aggregation_format_error.clone() {
            Some(message) => Some(
                ErrorReport::new("Couldn't format this stage", sentence(&message))
                    .kind(ErrorKind::Validation),
            ),
            None => pipeline
                .error
                .clone()
                .filter(|_| pipeline.error_stage == Some(index) && !pipeline.is_stale()),
        };

        panel(&appearance, cx)
            .child(header)
            // The editor keeps room even when an error is expanded below it.
            .child(div().flex().flex_1().min_h(px(96.0)).when_some(
                self.aggregation_stage_body_state.clone(),
                |slot, body_state| {
                    slot.child(
                        Editor::new(&body_state)
                            .font_family(crate::theme::fonts::mono())
                            .aria_label(format!("Stage {} body", index + 1))
                            .w_full()
                            .h_full()
                            .disabled(session_key.is_none()),
                    )
                },
            ))
            .when_some(error, |panel, report| {
                panel.child(
                    div().p(spacing::xs()).flex_shrink_0().child(
                        ErrorCallout::new("agg-stage-error", report)
                            .compact()
                            .state(self.state.clone()),
                    ),
                )
            })
            .into_any_element()
    }
}

pub(super) fn panel(appearance: &crate::state::settings::AppearanceSettings, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .bg(islands::card_bg(appearance, cx))
        .border_1()
        .border_color(islands::panel_border(appearance, cx).opacity(0.5))
        .rounded(islands::radius_sm(appearance))
}
