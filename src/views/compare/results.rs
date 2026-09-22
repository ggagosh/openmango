use gpui_kit::component::Selectable as _;
use gpui_kit::component::button::{ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;

use super::*;
use crate::components::ErrorCallout;
use crate::connection::ops::compare::{DiffKind, DiffRow};
use crate::connection::ops::compare_sync::RowOutcome;
use crate::error::ErrorReport;
use crate::helpers::format_number;

pub(super) fn key_label(row: &DiffRow) -> String {
    match &row.key {
        mongodb::bson::Bson::Document(key) => key
            .values()
            .map(|v| crate::bson::bson_value_preview(v, 60))
            .collect::<Vec<_>>()
            .join(" · "),
        value => crate::bson::bson_value_preview(value, 120),
    }
}

impl CompareView {
    /// Segment filter, scan progress and the run's status line.
    pub(super) fn render_summary(&self, id: Uuid, cx: &Context<Self>) -> AnyElement {
        let app = self.state.read(cx);
        let tab = app.compare_tab(id).unwrap();
        let appearance = app.settings.appearance.clone();
        let muted = cx.theme().muted_foreground;
        let c = tab.counts;
        let mut bar = div()
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(spacing::sm())
            .px(spacing::lg())
            .py(spacing::sm())
            .border_b_1()
            .border_color(islands::panel_border(&appearance, cx));

        // While scanning, the status text is replaced in place and a 2 px line runs along the
        // bottom edge: nothing in the bar changes height, so the list below never jumps.
        let mut scanning: Option<(String, Option<f32>)> = None;
        if tab.busy() {
            let read = c.left_read + c.right_read;
            let elapsed = tab.started.map_or(0.0, |s| s.elapsed().as_secs_f64());
            let total = tab.estimated[0].zip(tab.estimated[1]).map(|(a, b)| a + b);
            let rate = if elapsed > 0.0 { (read as f64 / elapsed) as u64 } else { 0 };
            let progress = if tab.results_config().filter.trim().is_empty() {
                total
                    .filter(|t| *t > 0)
                    .map(|total| (read as f32 / total as f32 * 100.0).min(100.0))
            } else {
                None
            };
            let mut text =
                format!("{} documents read · {}/s", format_number(read), format_number(rate));
            if let Some(sort) = &tab.sort {
                for (side, covered, started) in [
                    ("Left", sort.left_covered, tab.started_sides[0]),
                    ("Right", sort.right_covered, tab.started_sides[1]),
                ] {
                    if !covered && !started {
                        text.push_str(&format!(" · {side} is sorting on the server…"));
                    }
                }
            }
            scanning = Some((text, progress));
        }

        let segments = [
            (None, "All", c.only_left + c.only_right + c.different),
            (Some(DiffKind::OnlyLeft), "Left only", c.only_left),
            (Some(DiffKind::OnlyRight), "Right only", c.only_right),
            (Some(DiffKind::Different), "Different", c.different),
            (Some(DiffKind::Minor), "Minor", c.minor),
            (Some(DiffKind::MultipleMatches), "Multiple matches", c.multiple_matches),
        ];
        // Plain buttons, no sliding indicator: segment switching is a high-frequency action.
        let mut tabs = ButtonGroup::new("compare-segments").small();
        for (index, (kind, label, count)) in segments.into_iter().enumerate() {
            // The ambiguity bucket only exists for custom keys; keep it out of the _id case.
            if index == 5 && tab.results_config().fields == ["_id"] && count == 0 {
                continue;
            }
            let state = self.state.clone();
            let scroll = self.scroll.clone();
            tabs = tabs.child(
                Button::new(("compare-segment", index))
                    .ghost()
                    .small()
                    .selected(tab.segment == index)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .children(kind.map(|kind| dot(kind_color(kind, cx))))
                            .child(label)
                            .child(div().text_xs().text_color(muted).child(format_number(count))),
                    )
                    .on_click(move |_, _, cx| {
                        state.update(cx, |app, cx| {
                            if let Some(tab) = app.compare_tab_mut(id) {
                                tab.segment = index;
                            }
                            cx.notify();
                        });
                        scroll.scroll_to_item(0, ScrollStrategy::Top);
                    }),
            );
        }

        let mut status = vec![format!("{} identical", format_number(c.identical))];
        if let Some(summary) = &tab.summary {
            if let Some([left, right]) = summary.skipped
                && left + right > 0
            {
                status.push(format!(
                    "{} without key skipped (left {}, right {})",
                    format_number(left + right),
                    format_number(left),
                    format_number(right)
                ));
            }
            status.push(format_elapsed(summary.elapsed));
            if summary.cancelled {
                status.push(format!(
                    "cancelled after {} documents",
                    format_number(c.left_read + c.right_read)
                ));
            }
        }
        let compared_at = tab.compared_at;
        if let Some(at) = compared_at {
            status.push(relative_time(at));
        }
        bar = bar.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_x(spacing::md())
                .gap_y(spacing::xs())
                .child(div().flex().min_w_0().max_w_full().child(tabs))
                .child(match &scanning {
                    Some((text, _)) => {
                        let state = self.state.clone();
                        div()
                            .id("compare-status")
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(div().text_xs().text_color(muted).child(text.clone()))
                            .child(
                                Button::new("compare-cancel")
                                    .ghost()
                                    .small()
                                    .label("Cancel")
                                    .on_click(move |_, _, cx| {
                                        AppCommands::cancel_compare(&state, id, cx)
                                    }),
                            )
                            .into_any_element()
                    }
                    None => div()
                        .id("compare-status")
                        .text_xs()
                        .text_color(muted)
                        .child(status.join(" · "))
                        .when_some(compared_at, |status, at| {
                            status.tooltip(move |window, cx| {
                                Tooltip::new(format!(
                                    "Compared {}",
                                    crate::bson::format_datetime_displayed(at)
                                ))
                                .build(window, cx)
                            })
                        })
                        .into_any_element(),
                }),
        );
        if let Some((_, progress)) = scanning {
            let primary = cx.theme().primary;
            bar = bar.child(
                div()
                    .debug_selector(|| "compare-progress".into())
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .h(px(2.0))
                    .w(relative(progress.map_or(1.0, |p| p / 100.0)))
                    .bg(primary)
                    .when(progress.is_none(), |line| line.opacity(0.35)),
            );
        }

        if let Some(config) = &tab.compared
            && config != &tab.config
        {
            let names = config.sides.each_ref().map(|side| endpoint_label(app, side));
            bar = bar.child(note(
                format!(
                    "These results compared {} with {} · Match by {} · {}{}",
                    names[0],
                    names[1],
                    config.fields.join(", "),
                    if config.filter.trim().is_empty() {
                        "No filter".to_string()
                    } else {
                        format!("Filter {}", config.filter)
                    },
                    if config.ignore.is_empty() {
                        String::new()
                    } else {
                        format!(" · Ignoring {}", config.ignore.join(", "))
                    }
                ),
                cx,
            ));
        }
        if let Some(reason) = app.compare_disabled_reason(tab.results_config())
            && tab.compared.is_some()
        {
            bar = bar.child(note(reason, cx));
        }
        if tab.summary.as_ref().is_some_and(|s| s.truncated) {
            bar = bar.child(note(
                format!(
                    "Showing the first {} differences; counts are exact. Use a filter to compare a smaller part.",
                    format_number(tab.rows.len() as u64)
                ),
                cx,
            ));
        }
        if c.multiple_matches >= 1_000 {
            bar = bar.child(note(
                "Many keys match several documents. Choose a field that identifies each document uniquely.",
                cx,
            ));
        }
        if let Some(error) = &tab.error {
            let retry_state = self.state.clone();
            bar = bar.child(
                ErrorCallout::new(
                    format!("compare-error-{id}"),
                    ErrorReport::new("Comparison failed", error.clone()),
                )
                .compact()
                .state(self.state.clone())
                .action(
                    Button::new("compare-retry")
                        .ghost()
                        .xsmall()
                        .label("Retry")
                        .disabled(app.compare_disabled_reason(&tab.config).is_some())
                        .on_click(move |_, _, cx| {
                            AppCommands::run_compare(retry_state.clone(), id, cx)
                        }),
                ),
            );
        }
        bar.into_any_element()
    }

    pub(super) fn render_results(&self, id: Uuid, cx: &Context<Self>) -> AnyElement {
        let tab = self.state.read(cx).compare_tab(id).unwrap();
        let count = tab.visible().len();
        let muted = cx.theme().muted_foreground;
        let mut panel =
            div().flex().flex_col().size_full().min_w_0().min_h_0().overflow_hidden().child(
                div().px(spacing::sm()).pt(spacing::sm()).pb(spacing::xs()).child(
                    Input::new(&self.controls.as_ref().unwrap().find)
                        .small()
                        .w_full()
                        .prefix(Icon::new(IconName::Search).xsmall().text_color(muted))
                        .cleanable(true),
                ),
            );
        if let Some(message) = &self.find_error {
            panel = panel
                .child(div().px(spacing::md()).pb(spacing::xs()).child(note(message.clone(), cx)));
        }
        if count == 0 {
            let (icon, title, detail): (AnyElement, &str, String) = if tab.running {
                (
                    Spinner::new().small().into_any_element(),
                    "Scanning…",
                    "Differences appear here as they are found.".into(),
                )
            } else if tab.rows.is_empty()
                && tab.error.is_none()
                && !tab.summary.as_ref().is_some_and(|s| s.cancelled)
            {
                (
                    Icon::new(IconName::CircleCheck)
                        .small()
                        .text_color(cx.theme().success)
                        .into_any_element(),
                    "No differences",
                    format!("{} documents are identical.", format_number(tab.counts.identical)),
                )
            } else {
                (
                    div().into_any_element(),
                    "Nothing in this segment",
                    "Pick another segment above.".into(),
                )
            };
            return panel
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(spacing::xs())
                        .p(spacing::lg())
                        .text_center()
                        .child(icon)
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                        .child(note(detail, cx)),
                )
                .into_any_element();
        }
        let state = self.state.clone();
        let selected_bg = cx.theme().list_active;
        let hover = cx.theme().list_hover;
        let foreground = cx.theme().foreground;
        let success = cx.theme().success;
        let danger = cx.theme().danger;
        panel
            .child(
                uniform_list(
                    "compare-differences",
                    count,
                    cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                        let app = state.read(cx);
                        let Some(tab) = app.compare_tab(id) else {
                            return Vec::new();
                        };
                        let all_segment = tab.segment == 0;
                        let selectable = tab.sync.target.is_some();
                        let frozen = tab.sync.running || tab.sync.completed;
                        range
                            .filter_map(|position| {
                                let index = *tab.visible().get(position)?;
                                let row = &tab.rows[index];
                                let kind = row.kind;
                                let trailing = match kind {
                                    DiffKind::Different => format!(
                                        "{} field{}",
                                        row.changed,
                                        if row.changed == 1 { "" } else { "s" }
                                    ),
                                    DiffKind::Minor => "minor".into(),
                                    DiffKind::MultipleMatches => {
                                        format!("{} · {}", row.left_count, row.right_count)
                                    }
                                    DiffKind::OnlyLeft | DiffKind::OnlyRight if all_segment => {
                                        kind_label(kind).into()
                                    }
                                    _ => String::new(),
                                };
                                let state = state.clone();
                                let selection_state = state.clone();
                                let checked = tab.sync.selected(index, kind);
                                let outcome = tab.sync.outcomes.get(&index).cloned();
                                let focus = view.focus.clone();
                                let marker: AnyElement = match &outcome {
                                    Some(RowOutcome::Written | RowOutcome::Restored) => Icon::new(
                                        IconName::Check,
                                    )
                                    .xsmall()
                                    .text_color(success)
                                    .into_any_element(),
                                    Some(RowOutcome::Skipped(_)) => {
                                        div().text_xs().text_color(muted).child("↷").into_any_element()
                                    }
                                    Some(_) => Icon::new(IconName::TriangleAlert)
                                        .xsmall()
                                        .text_color(danger)
                                        .into_any_element(),
                                    None => dot(kind_color(kind, cx)).into_any_element(),
                                };
                                Some(
                                    div()
                                        .id(("compare-row", index))
                                        .h(px(28.0))
                                        .w_full()
                                        .min_w_0()
                                        .px(spacing::xs())
                                        .py(px(1.0))
                                        .child(
                                            div()
                                                .size_full()
                                                .min_w_0()
                                                .px(spacing::sm())
                                                .rounded(borders::radius_sm())
                                                .flex()
                                                .items_center()
                                                .gap(spacing::sm())
                                                .text_sm()
                                                .text_color(foreground)
                                                .when(tab.selected == Some(index), |row| {
                                                    row.bg(selected_bg)
                                                })
                                                .hover(|row| row.bg(hover))
                                                .cursor_pointer()
                                                .when(selectable && kind != DiffKind::MultipleMatches, |line| {
                                                    line.child(
                                                        crate::components::tri_checkbox::tri_checkbox(
                                                            ("sync-row", index),
                                                            if checked {
                                                                gpui_kit::base::CheckboxState::Checked
                                                            } else {
                                                                gpui_kit::base::CheckboxState::Unchecked
                                                            },
                                                            "",
                                                            frozen,
                                                            cx,
                                                        )
                                                        .accessibility_label(format!(
                                                            "Select {} for sync",
                                                            key_label(row)
                                                        ))
                                                        .on_change(move |_, event, _, cx| {
                                                            cx.stop_propagation();
                                                            selection_state.update(cx, |app, cx| {
                                                                if let Some(tab) = app.compare_tab_mut(id) {
                                                                    tab.select_sync_row(
                                                                        index,
                                                                        event.modifiers().shift,
                                                                        true,
                                                                    );
                                                                }
                                                                cx.notify();
                                                            });
                                                        }),
                                                    )
                                                })
                                                .child(
                                                    div()
                                                        .w(px(12.0))
                                                        .flex_shrink_0()
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .child(marker),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .truncate()
                                                        .child(key_label(row)),
                                                )
                                                .when(!trailing.is_empty(), |line| {
                                                    line.child(
                                                        div()
                                                            .flex_shrink_0()
                                                            .max_w(px(120.0))
                                                            .truncate()
                                                            .text_xs()
                                                            .text_color(muted)
                                                            .child(trailing),
                                                    )
                                                }),
                                        )
                                        .when_some(outcome, |line, outcome| {
                                            line.tooltip(move |window, cx| {
                                                Tooltip::new(outcome.message().to_owned())
                                                    .build(window, cx)
                                            })
                                        })
                                        .on_click(move |event, window, cx| {
                                            state.update(cx, |app, cx| {
                                                if let Some(tab) = app.compare_tab_mut(id) {
                                                    let modifiers = event.modifiers();
                                                    tab.select_sync_row(
                                                        index,
                                                        modifiers.shift,
                                                        modifiers.secondary() || modifiers.control,
                                                    );
                                                }
                                                cx.notify();
                                            });
                                            AppCommands::select_compare_row(
                                                state.clone(),
                                                id,
                                                index,
                                                cx,
                                            );
                                            window.focus(&focus, cx);
                                        }),
                                )
                            })
                            .collect()
                    }),
                )
                .flex_1()
                .track_scroll(&self.scroll),
            )
            .vertical_scrollbar(&self.scroll)
            .into_any_element()
    }
}
