use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::resizable::{h_resizable, resizable_panel};
use gpui_component::scroll::ScrollableElement;

use crate::components::Button;
use crate::helpers::format_number;
use crate::state::{
    AppCommands, CollectionSubview, ExplainCostBand, ExplainNode, ExplainOpenMode, ExplainPanelTab,
    ExplainSeverity, ExplainState, ExplainViewMode, SessionKey,
};
use crate::theme::spacing;
use crate::views::CollectionView;

impl CollectionView {
    pub(in crate::views::documents) fn render_explain_modal_layer(
        &mut self,
        explain: &ExplainState,
        session_key: Option<SessionKey>,
        active_subview: CollectionSubview,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !matches!(explain.open_mode, ExplainOpenMode::Modal) {
            return div().into_any_element();
        }

        let content = match explain.view_mode {
            ExplainViewMode::Tree => self.render_explain_tree_tab(explain, session_key.clone(), cx),
            ExplainViewMode::Json => render_explain_json_tab(explain, cx),
        };

        let tab_controls = div()
            .flex()
            .items_center()
            .gap(spacing::xs())
            .child(
                if explain.view_mode == ExplainViewMode::Tree {
                    Button::new("explain-mode-tree").compact().primary().label("Visual Tree")
                } else {
                    Button::new("explain-mode-tree").compact().ghost().label("Visual Tree")
                }
                .on_click({
                    let state = self.state.clone();
                    let session_key = session_key.clone();
                    move |_, _, cx| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        state.update(cx, |state, cx| {
                            state.set_explain_mode(&session_key, ExplainViewMode::Tree);
                            cx.notify();
                        });
                    }
                }),
            )
            .child(
                if explain.view_mode == ExplainViewMode::Json {
                    Button::new("explain-mode-json").compact().primary().label("Raw JSON")
                } else {
                    Button::new("explain-mode-json").compact().ghost().label("Raw JSON")
                }
                .on_click({
                    let state = self.state.clone();
                    let session_key = session_key.clone();
                    move |_, _, cx| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        state.update(cx, |state, cx| {
                            state.set_explain_mode(&session_key, ExplainViewMode::Json);
                            cx.notify();
                        });
                    }
                }),
            );

        let tab_hint = if explain.view_mode == ExplainViewMode::Tree {
            "Select a stage to inspect index usage, costs, and timings."
        } else {
            "Inspect full explain payload and copy fields directly from raw JSON."
        };

        let global_controls = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(spacing::xs())
            .child(
                Button::new("explain-rerun")
                    .compact()
                    .label("Explain")
                    .disabled(session_key.is_none() || explain.loading)
                    .on_click({
                        let state = self.state.clone();
                        let session_key = session_key.clone();
                        move |_, _, cx| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            match active_subview {
                                CollectionSubview::Aggregation => {
                                    AppCommands::run_explain_for_aggregation(
                                        state.clone(),
                                        session_key,
                                        cx,
                                    );
                                }
                                _ => {
                                    AppCommands::run_explain_for_session(
                                        state.clone(),
                                        session_key,
                                        cx,
                                    );
                                }
                            }
                        }
                    }),
            )
            .child(
                Button::new("explain-copy-json")
                    .compact()
                    .ghost()
                    .label("Copy JSON")
                    .disabled(explain.raw_json.is_none())
                    .on_click({
                        let raw_json = explain.raw_json.clone();
                        move |_, _, cx| {
                            let Some(raw_json) = raw_json.clone() else {
                                return;
                            };
                            cx.write_to_clipboard(ClipboardItem::new_string(raw_json));
                        }
                    }),
            )
            .child(Button::new("explain-close").compact().ghost().label("Close").on_click({
                let state = self.state.clone();
                let session_key = session_key.clone();
                move |_, _, cx| {
                    let Some(session_key) = session_key.clone() else {
                        return;
                    };
                    state.update(cx, |state, cx| {
                        state.set_explain_open_mode(&session_key, ExplainOpenMode::Closed);
                        cx.notify();
                    });
                }
            }));

        div()
            .absolute()
            .inset_0()
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_key_down({
                let state = self.state.clone();
                let session_key = session_key.clone();
                move |event: &KeyDownEvent, _window: &mut Window, cx: &mut App| {
                    let key = event.keystroke.key.to_ascii_lowercase();
                    if key != "escape" {
                        return;
                    }
                    let Some(session_key) = session_key.clone() else {
                        return;
                    };
                    cx.stop_propagation();
                    state.update(cx, |state, cx| {
                        state.set_explain_open_mode(&session_key, ExplainOpenMode::Closed);
                        cx.notify();
                    });
                }
            })
            .child(div().absolute().inset_0().bg(crate::theme::colors::backdrop(cx)).on_mouse_down(
                MouseButton::Left,
                |_, _, cx| {
                    cx.stop_propagation();
                },
            ))
            .child(
                div().absolute().inset_0().p(spacing::md()).child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .bg(cx.theme().background)
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(spacing::sm())
                                .px(spacing::md())
                                .py(spacing::sm())
                                .border_b_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().tab_bar.opacity(0.45))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(spacing::sm())
                                        .child(
                                            div()
                                                .text_lg()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child("Explain Plan"),
                                        )
                                        .child(
                                            explain_info_chip(
                                                explain.scope.label(),
                                                cx.theme().muted_foreground,
                                                cx,
                                            )
                                            .into_any_element(),
                                        )
                                        .child(if explain.stale {
                                            explain_info_chip("Stale", cx.theme().warning, cx)
                                                .into_any_element()
                                        } else {
                                            div().into_any_element()
                                        })
                                        .child(if explain.loading {
                                            explain_info_chip("Running...", cx.theme().primary, cx)
                                                .into_any_element()
                                        } else {
                                            div().into_any_element()
                                        }),
                                )
                                .child(global_controls),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(spacing::sm())
                                .px(spacing::md())
                                .py(spacing::xs())
                                .border_b_1()
                                .border_color(cx.theme().border.opacity(0.7))
                                .bg(cx.theme().tab_bar.opacity(0.25))
                                .child(tab_controls)
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(tab_hint),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_w(px(0.0))
                                .min_h(px(0.0))
                                .overflow_hidden()
                                .child(content),
                        ),
                ),
            )
            .into_any_element()
    }

    fn render_explain_tree_tab(
        &mut self,
        explain: &ExplainState,
        session_key: Option<SessionKey>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if explain.nodes.is_empty() {
            return explain_empty("Run Explain to generate a visual execution plan.", cx);
        }

        let visual_nodes = build_visual_nodes(&explain.nodes);
        let selected_visual = explain
            .selected_node_id
            .as_deref()
            .and_then(|selected_id| {
                visual_nodes.iter().copied().find(|node| node.id.as_str() == selected_id)
            })
            .or_else(|| visual_nodes.first().copied());

        let rows = visual_nodes.iter().enumerate().map(|(index, node)| {
            let node = *node;
            let is_selected = selected_visual.is_some_and(|selected| selected.id == node.id);
            let severity = severity_color(node.severity, cx);
            let cost = cost_color(node.cost_band, cx);
            let border =
                if is_selected { cx.theme().primary } else { cx.theme().border.opacity(0.75) };
            let bg = if is_selected {
                cx.theme().primary.opacity(0.06)
            } else {
                cx.theme().tab_bar.opacity(0.2)
            };

            let title = stage_display_label(&node.label);
            let returned = node.n_returned.map(format_number).unwrap_or_else(|| "—".to_string());
            let docs = node.docs_examined.map(format_number).unwrap_or_else(|| "—".to_string());
            let keys = node.keys_examined.map(format_number).unwrap_or_else(|| "—".to_string());
            let time =
                node.time_ms.map(|value| format!("{value}ms")).unwrap_or_else(|| "—".to_string());

            let mut card = div()
                .w(px(420.0))
                .max_w(px(520.0))
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .px(spacing::md())
                .py(spacing::sm())
                .rounded(px(9.0))
                .border_1()
                .border_color(border)
                .bg(bg)
                .on_mouse_down(MouseButton::Left, {
                    let state = self.state.clone();
                    let session_key = session_key.clone();
                    let node_id = node.id.clone();
                    move |_, _, cx| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        state.update(cx, |state, cx| {
                            state.set_explain_selected_node(&session_key, Some(node_id.clone()));
                            cx.notify();
                        });
                    }
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(spacing::sm())
                        .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(spacing::xs())
                                .child(explain_metric_chip(node.severity.label(), severity, cx))
                                .child(explain_metric_chip(node.cost_band.label(), cost, cx)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(spacing::sm())
                        .child(explain_metric_pair("Returned", returned, cx))
                        .child(explain_metric_pair("Execution Time", time, cx)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("Docs {docs}  •  Keys {keys}")),
                );

            if let Some(index_name) = node.index_name.as_ref() {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("Index: {index_name}")),
                );
            }

            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.0))
                .child(card)
                .child(if index + 1 < visual_nodes.len() {
                    div()
                        .w(px(2.0))
                        .h(px(26.0))
                        .bg(cx.theme().border.opacity(0.85))
                        .into_any_element()
                } else {
                    div().h(px(1.0)).into_any_element()
                })
                .into_any_element()
        });

        let tree_canvas = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_y_scrollbar()
            .items_center()
            .p(spacing::lg())
            .gap(px(2.0))
            .children(rows)
            .into_any_element();

        let run_count = explain.history.len();
        let current_run_index = explain.current_run_index();
        let run_label = if let Some(index) = current_run_index {
            format!("Run {}/{}", index + 1, run_count)
        } else {
            "Run —".to_string()
        };
        let can_prev_run = current_run_index.is_some_and(|index| index > 0);
        let can_next_run = current_run_index.is_some_and(|index| index + 1 < run_count);
        let can_compare_prev = current_run_index.is_some_and(|index| index > 0);
        let diff_active = explain.diff.is_some();
        let can_clear_history = explain.has_history_to_clear();

        let run_toolbar = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(spacing::xs())
            .child(
                Button::new("explain-run-prev")
                    .compact()
                    .ghost()
                    .label("Prev")
                    .disabled(session_key.is_none() || !can_prev_run)
                    .on_click({
                        let state = self.state.clone();
                        let session_key = session_key.clone();
                        move |_, _, cx| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            state.update(cx, |state, cx| {
                                state.cycle_explain_run(&session_key, -1);
                                cx.notify();
                            });
                        }
                    }),
            )
            .child(
                div()
                    .px(spacing::xs())
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(run_label),
            )
            .child(
                Button::new("explain-run-next")
                    .compact()
                    .ghost()
                    .label("Next")
                    .disabled(session_key.is_none() || !can_next_run)
                    .on_click({
                        let state = self.state.clone();
                        let session_key = session_key.clone();
                        move |_, _, cx| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            state.update(cx, |state, cx| {
                                state.cycle_explain_run(&session_key, 1);
                                cx.notify();
                            });
                        }
                    }),
            )
            .child(if diff_active {
                Button::new("explain-clear-diff")
                    .compact()
                    .ghost()
                    .label("Clear Diff")
                    .disabled(session_key.is_none())
                    .on_click({
                        let state = self.state.clone();
                        let session_key = session_key.clone();
                        move |_, _, cx| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            state.update(cx, |state, cx| {
                                state.clear_explain_compare_run(&session_key);
                                cx.notify();
                            });
                        }
                    })
            } else {
                Button::new("explain-compare-prev")
                    .compact()
                    .ghost()
                    .label("Compare Prev")
                    .disabled(session_key.is_none() || !can_compare_prev)
                    .on_click({
                        let state = self.state.clone();
                        let session_key = session_key.clone();
                        move |_, _, cx| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            state.update(cx, |state, cx| {
                                state.compare_explain_previous_run(&session_key);
                                cx.notify();
                            });
                        }
                    })
            })
            .child(
                Button::new("explain-clear-history")
                    .compact()
                    .ghost()
                    .label("Clear History")
                    .disabled(session_key.is_none() || !can_clear_history)
                    .on_click({
                        let state = self.state.clone();
                        let session_key = session_key.clone();
                        move |_, _, cx| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            state.update(cx, |state, cx| {
                                state.clear_explain_previous_runs(&session_key);
                                cx.notify();
                            });
                        }
                    }),
            );

        let panel_tabs = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(spacing::xs())
            .child(explain_panel_tab_button(
                "explain-panel-inspector",
                ExplainPanelTab::Inspector,
                explain.panel_tab,
                false,
                self.state.clone(),
                session_key.clone(),
            ))
            .child(explain_panel_tab_button(
                "explain-panel-rejected",
                ExplainPanelTab::RejectedPlans,
                explain.panel_tab,
                explain.rejected_plans.is_empty(),
                self.state.clone(),
                session_key.clone(),
            ))
            .child(explain_panel_tab_button(
                "explain-panel-diff",
                ExplainPanelTab::Diff,
                explain.panel_tab,
                explain.diff.is_none(),
                self.state.clone(),
                session_key.clone(),
            ));

        let panel_content = match explain.panel_tab {
            ExplainPanelTab::Inspector => div()
                .flex()
                .flex_col()
                .gap(spacing::sm())
                .child(render_explain_summary(explain, cx))
                .child(render_explain_bottlenecks(explain, cx))
                .child(render_explain_inspector(explain, selected_visual, cx))
                .into_any_element(),
            ExplainPanelTab::RejectedPlans => render_explain_rejected_plans(explain, cx),
            ExplainPanelTab::Diff => render_explain_diff(explain, cx),
        };

        let panel_controls = div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .pb(spacing::sm())
            .child(run_toolbar)
            .child(panel_tabs);

        let inspector = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_y_scrollbar()
            .px(spacing::md())
            .pb(spacing::md())
            .pt(spacing::xs())
            .gap(spacing::xs())
            .child(panel_controls)
            .child(panel_content);

        h_resizable("documents-explain-modal-main-split")
            .child(resizable_panel().child(tree_canvas))
            .child(
                resizable_panel().size(px(390.0)).size_range(px(300.0)..px(620.0)).child(inspector),
            )
            .into_any_element()
    }
}

fn render_explain_summary(explain: &ExplainState, cx: &App) -> AnyElement {
    let Some(summary) = &explain.summary else {
        return explain_empty("No explain summary yet.", cx);
    };

    let returned = summary.n_returned.map(format_number).unwrap_or_else(|| "—".to_string());
    let docs = summary.docs_examined.map(format_number).unwrap_or_else(|| "—".to_string());
    let keys = summary.keys_examined.map(format_number).unwrap_or_else(|| "—".to_string());
    let time = summary
        .execution_time_ms
        .map(|value| format!("{value}ms"))
        .unwrap_or_else(|| "—".to_string());

    let mut covered = div().flex().items_center().gap(spacing::xs()).flex_wrap();
    if summary.covered_indexes.is_empty() {
        covered = covered.child(
            div().text_xs().text_color(cx.theme().muted_foreground).child("No index metadata"),
        );
    } else {
        for index in &summary.covered_indexes {
            covered = covered.child(explain_info_chip(index, cx.theme().primary, cx));
        }
    }

    let body = div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Totals"))
        .child(explain_metric_line("Documents returned", &returned, cx))
        .child(explain_metric_line("Documents examined", &docs, cx))
        .child(explain_metric_line("Index keys examined", &keys, cx))
        .child(explain_metric_line("Execution time", &time, cx))
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Risk signals"))
        .child(explain_metric_line_accent(
            "Collection scan",
            if summary.has_collscan { "Detected" } else { "Not detected" },
            if summary.has_collscan { cx.theme().danger } else { cx.theme().primary },
            cx,
        ))
        .child(explain_metric_line_accent(
            "Sort stage",
            if summary.has_sort_stage {
                "Potential in-memory sort"
            } else {
                "No explicit sort stage"
            },
            if summary.has_sort_stage { cx.theme().warning } else { cx.theme().primary },
            cx,
        ))
        .child(explain_metric_line_accent(
            "Covered query",
            if summary.is_covered_query { "Yes" } else { "No" },
            if summary.is_covered_query { cx.theme().primary } else { cx.theme().warning },
            cx,
        ))
        .child(
            div().text_xs().text_color(cx.theme().muted_foreground).child("Indexes seen in plan"),
        )
        .child(covered);

    explain_section_card(
        "Query Performance Summary",
        Some("Fast health snapshot for this explain result."),
        None,
        body.into_any_element(),
        cx,
    )
}

fn render_explain_bottlenecks(explain: &ExplainState, cx: &App) -> AnyElement {
    if explain.bottlenecks.is_empty() {
        return explain_section_card(
            "Top Bottlenecks",
            Some("Ranked from highest observed impact."),
            None,
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("No bottleneck ranking available for this run.")
                .into_any_element(),
            cx,
        );
    }

    let mut items = div().flex().flex_col().gap(spacing::sm());
    for bottleneck in explain.bottlenecks.iter().take(3) {
        let evidence = format!(
            "Docs {} • Keys {} • Time {}",
            bottleneck.docs_examined.map(format_number).unwrap_or_else(|| "—".to_string()),
            bottleneck.keys_examined.map(format_number).unwrap_or_else(|| "—".to_string()),
            bottleneck
                .execution_time_ms
                .map(|value| format!("{value}ms"))
                .unwrap_or_else(|| "—".to_string())
        );
        items = items.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .px(spacing::sm())
                .py(spacing::xs())
                .rounded(px(8.0))
                .bg(cx.theme().tab_bar.opacity(0.18))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(spacing::sm())
                        .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child(format!(
                            "#{} {}",
                            bottleneck.rank,
                            stage_display_label(&bottleneck.stage)
                        )))
                        .child(
                            div().text_xs().text_color(cx.theme().muted_foreground).child(format!(
                                "Impact {}",
                                format_number(bottleneck.impact_score)
                            )),
                        ),
                )
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(evidence))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().foreground)
                        .child(bottleneck.recommendation.clone()),
                ),
        );
    }

    explain_section_card(
        "Top Bottlenecks",
        Some("Ranked from highest observed impact."),
        None,
        items.into_any_element(),
        cx,
    )
}

fn render_explain_rejected_plans(explain: &ExplainState, cx: &App) -> AnyElement {
    if explain.rejected_plans.is_empty() {
        return explain_section_card(
            "Rejected Plans",
            Some("No rejected candidate plans were reported by the server."),
            None,
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(
                    "This query currently reports only a winning plan in available explain output.",
                )
                .into_any_element(),
            cx,
        );
    }

    let winner_stage = explain
        .nodes
        .first()
        .map(|node| stage_display_label(&node.label))
        .unwrap_or_else(|| "Unknown".to_string());
    let winner_time = explain
        .summary
        .as_ref()
        .and_then(|summary| summary.execution_time_ms)
        .map(|value| format!("{value}ms"))
        .unwrap_or_else(|| "—".to_string());

    let mut plans = div().flex().flex_col().gap(spacing::sm());
    for plan in &explain.rejected_plans {
        let docs = plan.docs_examined.map(format_number).unwrap_or_else(|| "—".to_string());
        let keys = plan.keys_examined.map(format_number).unwrap_or_else(|| "—".to_string());
        let time = plan
            .execution_time_ms
            .map(|value| format!("{value}ms"))
            .unwrap_or_else(|| "—".to_string());
        let mut indexes = div().flex().items_center().flex_wrap().gap(spacing::xs());
        if plan.index_names.is_empty() {
            indexes = indexes.child(
                div().text_xs().text_color(cx.theme().muted_foreground).child("No index metadata"),
            );
        } else {
            for index_name in &plan.index_names {
                indexes =
                    indexes.child(explain_info_chip(index_name, cx.theme().muted_foreground, cx));
            }
        }

        plans = plans.child(
            div()
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .px(spacing::sm())
                .py(spacing::xs())
                .rounded(px(8.0))
                .bg(cx.theme().tab_bar.opacity(0.18))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(spacing::sm())
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(stage_display_label(&plan.root_stage)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(plan.plan_id.clone()),
                        ),
                )
                .child(explain_metric_line("Docs examined", &docs, cx))
                .child(explain_metric_line("Keys examined", &keys, cx))
                .child(explain_metric_line("Execution time", &time, cx))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(plan.reason_hint.clone()),
                )
                .child(indexes),
        );
    }

    let body = div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .child(explain_metric_line("Winning stage root", &winner_stage, cx))
        .child(explain_metric_line("Winning execution time", &winner_time, cx))
        .child(explain_metric_line(
            "Rejected candidates",
            &format_number(explain.rejected_plans.len() as u64),
            cx,
        ))
        .child(plans);

    explain_section_card(
        "Rejected Plans",
        Some("Compare planner candidates and why they lost."),
        None,
        body.into_any_element(),
        cx,
    )
}

fn render_explain_diff(explain: &ExplainState, cx: &App) -> AnyElement {
    let Some(diff) = explain.diff.as_ref() else {
        return explain_section_card(
            "Run Diff",
            Some("Choose a previous run with Compare Prev to inspect regressions."),
            None,
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("No comparison is active.")
                .into_any_element(),
            cx,
        );
    };

    let mut stage_rows = div().flex().flex_col().gap(spacing::xs());
    if diff.stage_deltas.is_empty() {
        stage_rows = stage_rows.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("No stage-level deltas available."),
        );
    } else {
        for delta in diff.stage_deltas.iter().take(10) {
            stage_rows = stage_rows.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .px(spacing::sm())
                    .py(spacing::xs())
                    .rounded(px(8.0))
                    .bg(cx.theme().tab_bar.opacity(0.18))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(spacing::sm())
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(stage_display_label(&delta.stage)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if delta.impact_score_delta > 0 {
                                        cx.theme().warning
                                    } else {
                                        cx.theme().primary
                                    })
                                    .child(format!(
                                        "Impact {}",
                                        format_signed_i64(delta.impact_score_delta)
                                    )),
                            ),
                    )
                    .child(explain_metric_line(
                        "Docs Δ",
                        &format_optional_delta(delta.docs_examined_delta),
                        cx,
                    ))
                    .child(explain_metric_line(
                        "Keys Δ",
                        &format_optional_delta(delta.keys_examined_delta),
                        cx,
                    ))
                    .child(explain_metric_line(
                        "Time Δ",
                        &format_optional_delta_suffix(delta.execution_time_delta_ms, "ms"),
                        cx,
                    )),
            );
        }
    }

    let body = div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .child(explain_metric_line("Returned Δ", &format_optional_delta(diff.n_returned_delta), cx))
        .child(explain_metric_line(
            "Docs examined Δ",
            &format_optional_delta(diff.docs_examined_delta),
            cx,
        ))
        .child(explain_metric_line(
            "Keys examined Δ",
            &format_optional_delta(diff.keys_examined_delta),
            cx,
        ))
        .child(explain_metric_line(
            "Execution time Δ",
            &format_optional_delta_suffix(diff.execution_time_delta_ms, "ms"),
            cx,
        ))
        .child(explain_metric_line(
            "Plan shape changed",
            if diff.plan_shape_changed { "Yes" } else { "No" },
            cx,
        ))
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Stage deltas"))
        .child(stage_rows);

    explain_section_card(
        "Run Diff",
        Some("Compare current run against selected baseline."),
        None,
        body.into_any_element(),
        cx,
    )
}

fn render_explain_inspector(
    explain: &ExplainState,
    selected_node: Option<&ExplainNode>,
    cx: &App,
) -> AnyElement {
    let selected = selected_node.or_else(|| explain.nodes.first());

    let Some(node) = selected else {
        return explain_empty("Select a stage to inspect details.", cx);
    };

    let severity = severity_color(node.severity, cx);
    let cost = cost_color(node.cost_band, cx);
    let stage = stage_display_label(&node.label);

    let mut metrics = div().flex().flex_col().gap(px(6.0));
    metrics = metrics
        .child(explain_metric_line("Stage", &stage, cx))
        .child(explain_metric_line("Path", &node.id, cx))
        .child(explain_metric_line(
            "Returned",
            &node.n_returned.map(format_number).unwrap_or_else(|| "—".to_string()),
            cx,
        ))
        .child(explain_metric_line(
            "Docs examined",
            &node.docs_examined.map(format_number).unwrap_or_else(|| "—".to_string()),
            cx,
        ))
        .child(explain_metric_line(
            "Keys examined",
            &node.keys_examined.map(format_number).unwrap_or_else(|| "—".to_string()),
            cx,
        ))
        .child(explain_metric_line(
            "Execution time",
            &node.time_ms.map(|value| format!("{value}ms")).unwrap_or_else(|| "—".to_string()),
            cx,
        ));

    let mut index_details = div().flex().flex_col().gap(px(6.0));
    if let Some(index_name) = &node.index_name {
        index_details = index_details.child(explain_metric_line("Index", index_name, cx));
    }
    if let Some(is_multi_key) = node.is_multi_key {
        index_details = index_details.child(explain_metric_line(
            "Multikey",
            if is_multi_key { "Yes" } else { "No" },
            cx,
        ));
    }
    if let Some(is_covered) = node.is_covered {
        index_details = index_details.child(explain_metric_line(
            "Covered",
            if is_covered { "Yes" } else { "No" },
            cx,
        ));
    }
    if node.index_name.is_none() && node.is_multi_key.is_none() && node.is_covered.is_none() {
        index_details = index_details.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("No index metadata reported for this stage."),
        );
    }

    let hints = build_stage_hints(node, cx);
    let mut hints_block = div().flex().flex_col().gap(px(6.0));
    for (text, accent) in hints {
        hints_block = hints_block.child(explain_hint_row(&text, accent));
    }

    let mut extra = div().flex().flex_col().gap(px(6.0));
    if node.extra_metrics.is_empty() {
        extra = extra.child(
            div().text_xs().text_color(cx.theme().muted_foreground).child("No extra stage metrics"),
        );
    } else {
        for (key, value) in &node.extra_metrics {
            extra = extra.child(explain_metric_line(key, value, cx));
        }
    }

    let chips = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(spacing::xs())
        .child(explain_metric_chip(node.severity.label(), severity, cx))
        .child(explain_metric_chip(node.cost_band.label(), cost, cx))
        .into_any_element();

    let body = div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .child(metrics)
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Insights"))
        .child(hints_block)
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Index details"))
        .child(index_details)
        .child(
            div().text_xs().text_color(cx.theme().muted_foreground).child("Extra engine counters"),
        )
        .child(extra);

    explain_section_card(
        "Stage Inspector",
        Some("Diagnosis-first details for the selected stage."),
        Some(chips),
        body.into_any_element(),
        cx,
    )
}

fn render_explain_json_tab(explain: &ExplainState, cx: &App) -> AnyElement {
    let Some(raw_json) = explain.raw_json.as_ref() else {
        return explain_empty("Run Explain to view raw JSON output.", cx);
    };

    let lines = raw_json.lines().map(|line| {
        div()
            .text_xs()
            .font_family(crate::theme::fonts::mono())
            .child(line.to_string())
            .into_any_element()
    });

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .overflow_y_scrollbar()
        .p(spacing::md())
        .bg(cx.theme().background)
        .children(lines)
        .into_any_element()
}

fn explain_empty(message: &str, cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(message.to_string())
        .into_any_element()
}

fn explain_metric_chip(label: &str, accent: Hsla, _cx: &App) -> Div {
    div()
        .px(spacing::xs())
        .py(px(2.0))
        .rounded(px(5.0))
        .bg(accent.opacity(0.13))
        .border_1()
        .border_color(accent.opacity(0.32))
        .text_xs()
        .text_color(accent)
        .child(label.to_string())
}

fn explain_info_chip(label: &str, accent: Hsla, _cx: &App) -> Div {
    div()
        .px(spacing::xs())
        .py(px(2.0))
        .rounded(px(5.0))
        .bg(accent.opacity(0.1))
        .border_1()
        .border_color(accent.opacity(0.28))
        .text_xs()
        .text_color(accent)
        .child(label.to_string())
}

fn explain_metric_pair(label: &str, value: String, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label.to_string()))
        .child(div().text_xs().font_weight(FontWeight::MEDIUM).child(value))
}

fn explain_metric_line(label: &str, value: &str, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::sm())
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label.to_string()))
        .child(
            div()
                .max_w(px(210.0))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_right()
                .text_ellipsis()
                .child(value.to_string()),
        )
}

fn explain_metric_line_accent(label: &str, value: &str, accent: Hsla, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::sm())
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label.to_string()))
        .child(
            div()
                .max_w(px(210.0))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_right()
                .text_ellipsis()
                .text_color(accent)
                .child(value.to_string()),
        )
}

fn explain_panel_tab_button(
    id: &'static str,
    tab: ExplainPanelTab,
    active_tab: ExplainPanelTab,
    disabled: bool,
    state: Entity<crate::state::AppState>,
    session_key: Option<SessionKey>,
) -> Button {
    let mut button = if active_tab == tab {
        Button::new(id).compact().primary().label(tab.label())
    } else {
        Button::new(id).compact().ghost().label(tab.label())
    };
    button = button.disabled(disabled || session_key.is_none());
    button.on_click(move |_, _, cx| {
        let Some(session_key) = session_key.clone() else {
            return;
        };
        state.update(cx, |state, cx| {
            state.set_explain_panel_tab(&session_key, tab);
            cx.notify();
        });
    })
}

fn format_optional_delta(delta: Option<i64>) -> String {
    delta.map(format_signed_i64).unwrap_or_else(|| "—".to_string())
}

fn format_optional_delta_suffix(delta: Option<i64>, suffix: &str) -> String {
    delta
        .map(|value| format!("{}{suffix}", format_signed_i64(value)))
        .unwrap_or_else(|| "—".to_string())
}

fn format_signed_i64(value: i64) -> String {
    if value > 0 {
        format!("+{}", format_number(value as u64))
    } else if value < 0 {
        format!("-{}", format_number(value.unsigned_abs()))
    } else {
        "0".to_string()
    }
}

fn explain_section_card(
    title: &str,
    subtitle: Option<&str>,
    trailing: Option<AnyElement>,
    body: AnyElement,
    cx: &App,
) -> AnyElement {
    let mut header_left = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title.to_string()));

    if let Some(subtitle) = subtitle {
        header_left = header_left.child(
            div().text_xs().text_color(cx.theme().muted_foreground).child(subtitle.to_string()),
        );
    }

    let mut header = div()
        .flex()
        .flex_col()
        .items_start()
        .gap(spacing::xs())
        .px(spacing::md())
        .py(spacing::sm())
        .child(header_left);

    if let Some(trailing) = trailing {
        header = header
            .child(div().flex().flex_wrap().items_center().gap(spacing::xs()).child(trailing));
    }

    div()
        .flex()
        .flex_col()
        .rounded(px(10.0))
        .border_1()
        .border_color(cx.theme().border.opacity(0.55))
        .bg(cx.theme().background.opacity(0.5))
        .overflow_hidden()
        .child(header)
        .child(div().px(spacing::md()).py(spacing::sm()).child(body))
        .into_any_element()
}

fn build_stage_hints(node: &ExplainNode, cx: &App) -> Vec<(String, Hsla)> {
    let mut hints = Vec::new();
    let stage = normalize_stage_label(&node.label);

    if stage.contains("COLLSCAN") {
        hints.push((
            "Full collection scan detected. Add a selective index for this filter path."
                .to_string(),
            cx.theme().danger,
        ));
    }

    if let (Some(docs), Some(returned)) = (node.docs_examined, node.n_returned)
        && returned > 0
    {
        let ratio = docs / returned;
        if ratio >= 500 {
            hints.push((
                format!(
                    "High scan ratio: {ratio} examined per returned document. Tighten filters or index coverage."
                ),
                cx.theme().warning,
            ));
        } else if ratio >= 100 {
            hints.push((
                format!("Scan ratio is elevated ({ratio}:1). Consider a more selective index."),
                cx.theme().muted_foreground,
            ));
        }
    }

    if let (Some(keys), Some(returned)) = (node.keys_examined, node.n_returned)
        && returned > 0
    {
        let ratio = keys / returned;
        if ratio >= 500 {
            hints.push((
                format!("Key scan overhead is high ({ratio}:1). Revisit index order and prefix."),
                cx.theme().warning,
            ));
        }
    }

    if node.index_name.is_none()
        && node.docs_examined.unwrap_or(0) > 0
        && !stage.contains("COLLSCAN")
    {
        hints.push((
            "No index metadata reported for this stage. Verify index selection in planner output."
                .to_string(),
            cx.theme().warning,
        ));
    }

    if node.is_covered == Some(true) {
        hints.push((
            "Covered access: this stage can serve results directly from index keys.".to_string(),
            cx.theme().primary,
        ));
    }

    if hints.is_empty() {
        hints.push((
            "No immediate risk indicators for this stage.".to_string(),
            cx.theme().muted_foreground,
        ));
    }

    hints
}

fn explain_hint_row(message: &str, accent: Hsla) -> Div {
    div()
        .px(spacing::xs())
        .py(px(4.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(accent.opacity(0.3))
        .bg(accent.opacity(0.11))
        .text_xs()
        .text_color(accent)
        .child(message.to_string())
}

fn build_visual_nodes(nodes: &[ExplainNode]) -> Vec<&ExplainNode> {
    let mut visual: Vec<&ExplainNode> = Vec::new();

    for node in nodes {
        if should_skip_visual_node(node) {
            continue;
        }

        if let Some(last) = visual.last_mut()
            && normalize_stage_label(&last.label) == normalize_stage_label(&node.label)
        {
            if node_signal_score(node) >= node_signal_score(last) {
                *last = node;
            }
            continue;
        }

        visual.push(node);
    }

    if visual.is_empty() {
        visual.extend(nodes.iter().take(1));
    }

    if visual.len() > 10 {
        let mut compact = Vec::with_capacity(10);
        compact.extend_from_slice(&visual[..7]);
        compact.extend_from_slice(&visual[visual.len() - 3..]);
        return compact;
    }

    visual
}

fn should_skip_visual_node(node: &ExplainNode) -> bool {
    let normalized = normalize_stage_label(&node.label);
    if matches!(normalized.as_str(), "CURSOR" | "PROJECT" | "UNWIND" | "LIMIT" | "COSCAN") {
        return true;
    }
    let noisy = matches!(
        normalized.as_str(),
        "BRANCH" | "UNIQUE" | "NLJ" | "SUBPLAN" | "OR" | "EOF" | "IXSEEK"
    );
    noisy && node_signal_score(node) < 2_000
}

fn node_signal_score(node: &ExplainNode) -> u64 {
    let metric = node.docs_examined.unwrap_or(0)
        + node.keys_examined.unwrap_or(0)
        + node.time_ms.unwrap_or(0) * 50
        + node.n_returned.unwrap_or(0);
    if node.index_name.is_some() { metric + 1_000 } else { metric }
}

fn stage_display_label(label: &str) -> String {
    normalize_stage_label(label)
}

fn normalize_stage_label(label: &str) -> String {
    label.trim_start_matches('$').to_ascii_uppercase()
}

fn severity_color(severity: ExplainSeverity, cx: &App) -> Hsla {
    match severity {
        ExplainSeverity::Low => cx.theme().primary,
        ExplainSeverity::Medium => cx.theme().muted_foreground,
        ExplainSeverity::High => cx.theme().warning,
        ExplainSeverity::Critical => cx.theme().danger,
    }
}

fn cost_color(cost: ExplainCostBand, cx: &App) -> Hsla {
    match cost {
        ExplainCostBand::Low => cx.theme().primary,
        ExplainCostBand::Medium => cx.theme().muted_foreground,
        ExplainCostBand::High => cx.theme().warning,
        ExplainCostBand::VeryHigh => cx.theme().danger,
    }
}
