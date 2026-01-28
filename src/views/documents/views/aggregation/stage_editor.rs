use gpui::*;
use gpui_component::button::{Button as MenuButton, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::input::Input;
use gpui_component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Disableable as _, Sizable as _, Size};

use crate::bson::{format_relaxed_json_value, parse_value_from_relaxed_json};
use crate::components::Button;
use crate::keyboard::{ClearAggregationStage, FormatAggregationStage};
use crate::state::app_state::{PipelineState, default_stage_body};
use crate::state::{SessionKey, StatusMessage};
use crate::theme::{borders, colors, spacing};

use crate::views::CollectionView;

use super::operators::OPERATOR_GROUPS;

impl CollectionView {
    pub(in crate::views::documents) fn render_aggregation_stage_editor(
        &mut self,
        pipeline: &PipelineState,
        session_key: Option<SessionKey>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected_index = pipeline.selected_stage;
        let stage = selected_index.and_then(|idx| pipeline.stages.get(idx));
        let selected_operator = stage.map(|stage| stage.operator.clone()).unwrap_or_default();

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(spacing::sm())
            .py(spacing::xs())
            .bg(colors::bg_header())
            .border_b_1()
            .border_color(colors::border())
            .child(div().flex().items_center().gap(spacing::xs()).child(
                if let Some(stage) = stage {
                    let operator_label = if stage.operator.trim().is_empty() {
                        "Select operator".to_string()
                    } else {
                        stage.operator.clone()
                    };
                    let session_key_for_menu = session_key.clone();
                    let state_for_menu = self.state.clone();
                    let body_state_for_menu = self.aggregation_stage_body_state.clone();
                    let operator_variant = ButtonCustomVariant::new(cx)
                        .color(colors::bg_button_secondary().into())
                        .foreground(colors::text_primary().into())
                        .border(colors::border_subtle().into())
                        .hover(colors::bg_button_secondary_hover().into())
                        .active(colors::bg_button_secondary_hover().into())
                        .shadow(false);
                    MenuButton::new("agg-operator")
                        .compact()
                        .label(operator_label)
                        .dropdown_caret(true)
                        .custom(operator_variant)
                        .rounded(borders::radius_sm())
                        .with_size(Size::XSmall)
                        .disabled(session_key.is_none())
                        .dropdown_menu_with_anchor(Corner::BottomLeft, {
                            let selected_operator = selected_operator.clone();
                            move |menu: PopupMenu, _window, _cx| {
                                let mut menu = menu;
                                for (group_idx, group) in OPERATOR_GROUPS.iter().enumerate() {
                                    menu = menu.item(PopupMenuItem::label(group.label.to_string()));
                                    for operator in group.operators {
                                        let operator = operator.to_string();
                                        let state = state_for_menu.clone();
                                        let session_key = session_key_for_menu.clone();
                                        let body_state = body_state_for_menu.clone();
                                        let checked = selected_operator == operator;
                                        menu = menu.item(
                                            PopupMenuItem::new(operator.clone())
                                                .checked(checked)
                                                .on_click({
                                                    move |_, window, cx| {
                                                        let Some(session_key) = session_key.clone()
                                                        else {
                                                            return;
                                                        };
                                                        let Some(index) = selected_index else {
                                                            return;
                                                        };
                                                        let template =
                                                            default_stage_body(&operator)
                                                                .map(|value| value.to_string());
                                                        state.update(cx, |state, cx| {
                                                            state.set_pipeline_stage_operator(
                                                                &session_key,
                                                                index,
                                                                operator.clone(),
                                                            );
                                                            cx.notify();
                                                        });
                                                        if let (Some(body_state), Some(template)) =
                                                            (body_state.clone(), template)
                                                        {
                                                            body_state.update(cx, |state, cx| {
                                                                state.set_value(
                                                                    template, window, cx,
                                                                );
                                                            });
                                                        }
                                                    }
                                                }),
                                        );
                                    }
                                    if group_idx + 1 < OPERATOR_GROUPS.len() {
                                        menu = menu.item(PopupMenuItem::separator());
                                    }
                                }
                                menu
                            }
                        })
                        .into_any_element()
                } else {
                    div()
                        .text_sm()
                        .text_color(colors::text_muted())
                        .child("Select a stage")
                        .into_any_element()
                },
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(
                        Button::new("agg-format-stage")
                            .compact()
                            .label("Format")
                            .tooltip_with_action(
                                "Format JSON",
                                &FormatAggregationStage,
                                Some("Documents Aggregation"),
                            )
                            .disabled(session_key.is_none() || stage.is_none())
                            .on_click({
                                let body_state = self.aggregation_stage_body_state.clone();
                                let state = self.state.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    let Some(body_state) = body_state.clone() else {
                                        return;
                                    };
                                    let raw = body_state.read(cx).value().to_string();
                                    match parse_value_from_relaxed_json(&raw) {
                                        Ok(value) => {
                                            let formatted = format_relaxed_json_value(&value);
                                            body_state.update(cx, |state, cx| {
                                                state.set_value(formatted, window, cx);
                                            });
                                        }
                                        Err(err) => {
                                            state.update(cx, |state, cx| {
                                                state.set_status_message(Some(
                                                    StatusMessage::error(format!(
                                                        "Invalid JSON: {err}"
                                                    )),
                                                ));
                                                cx.notify();
                                            });
                                        }
                                    }
                                }
                            }),
                    )
                    .child(
                        Button::new("agg-clear-stage")
                            .compact()
                            .label("Clear")
                            .tooltip_with_action(
                                "Clear stage",
                                &ClearAggregationStage,
                                Some("Documents Aggregation Input"),
                            )
                            .disabled(session_key.is_none() || stage.is_none())
                            .on_click({
                                let body_state = self.aggregation_stage_body_state.clone();
                                let state = self.state.clone();
                                let session_key = session_key.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    let Some(body_state) = body_state.clone() else {
                                        return;
                                    };
                                    body_state.update(cx, |state, cx| {
                                        state.set_value("{}".to_string(), window, cx);
                                    });
                                    let Some(session_key) = session_key.clone() else {
                                        return;
                                    };
                                    let Some(index) = selected_index else {
                                        return;
                                    };
                                    state.update(cx, |state, cx| {
                                        state.set_pipeline_stage_body(
                                            &session_key,
                                            index,
                                            "{}".to_string(),
                                        );
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            );

        let body = if stage.is_some() {
            if let Some(body_state) = self.aggregation_stage_body_state.clone() {
                Input::new(&body_state)
                    .font_family(crate::theme::fonts::mono())
                    .w_full()
                    .h_full()
                    .disabled(session_key.is_none())
                    .into_any_element()
            } else {
                div().into_any_element()
            }
        } else {
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(colors::text_muted())
                .child("Select a stage to edit")
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(colors::bg_app())
            .border_1()
            .border_color(colors::border_subtle())
            .rounded(borders::radius_sm())
            .child(header)
            .child(div().flex().flex_1().min_w(px(0.0)).overflow_y_scrollbar().child(body))
            .into_any_element()
    }
}
