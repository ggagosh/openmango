use gpui_kit::base::CheckboxState;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::radio::Radio;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;

use super::*;
use crate::components::tri_checkbox::tri_checkbox;
use crate::connection::ops::compare::{DiffKind, Side};
use crate::connection::ops::compare_sync::{Operation, operation_for};
use crate::helpers::format_number;
use crate::state::compare::CompareTabState;

impl CompareView {
    /// Footer: pick the collection to change, choose what to write, review. Then the run's
    /// outcome with undo.
    pub(super) fn render_sync_bar(&self, id: Uuid, cx: &Context<Self>) -> AnyElement {
        let app = self.state.read(cx);
        let tab = app.compare_tab(id).unwrap();
        // Present from the first run on, so the tab does not grow and shrink around each scan.
        if tab.compared.is_none() {
            return div().into_any_element();
        }
        let appearance = app.settings.appearance.clone();
        let sync = &tab.sync;
        let mut bar = div()
            .debug_selector(|| "compare-sync-bar".into())
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(spacing::sm())
            .px(spacing::lg())
            .py(spacing::md())
            .border_t_1()
            .border_color(islands::panel_border(&appearance, cx))
            .bg(islands::tool_bg(&appearance, cx));

        if sync.running || sync.completed {
            bar = bar.child(self.render_sync_outcome(id, app, tab, cx));
        } else {
            bar = bar.child(self.render_sync_targets(id, app, tab, cx));
            if let Some(target) = sync.target {
                bar = bar.child(self.render_sync_operations(id, app, tab, target, cx));
            }
        }
        if let Some(error) = &sync.error {
            bar = bar.child(div().text_xs().text_color(cx.theme().danger).child(error.clone()));
        }
        bar.into_any_element()
    }

    /// "Sync to" with one radio per side. Right of it: a hint, or Cancel and Review.
    fn render_sync_targets(
        &self,
        id: Uuid,
        app: &AppState,
        tab: &CompareTabState,
        cx: &Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let sync = &tab.sync;
        let pending = tab.busy() || tab.summary.is_none();
        let mut choices = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(spacing::lg())
            .gap_y(spacing::xs())
            .min_w_0()
            .child(
                div().text_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child("Sync to"),
            );
        for (index, target) in [Side::Left, Side::Right].into_iter().enumerate() {
            let endpoint = &tab.results_config().sides[index];
            let reason =
                if pending { None } else { app.compare_sync_target_disabled_reason(id, index) };
            let disabled = pending || reason.is_some();
            let state = self.state.clone();
            let choose = move |cx: &mut App| {
                state.update(cx, |app, cx| {
                    if let Some(tab) = app.compare_tab_mut(id) {
                        tab.sync.set_target(target);
                    }
                    cx.notify();
                })
            };
            let radio_choose = choose.clone();
            let path = endpoint_label(app, endpoint);
            let mut option = div()
                .id(("sync-target-option", index))
                .flex()
                .items_center()
                .gap(spacing::sm())
                .when(!disabled, |option| {
                    option.cursor_pointer().on_click(move |_, _, cx| choose(cx))
                })
                .child(
                    Radio::new(("sync-target", index))
                        .checked(sync.target == Some(target))
                        .disabled(disabled)
                        .accessibility_label(format!("{} · {path}", side_name(index)))
                        .on_click(move |_, _, cx| radio_choose(cx)),
                )
                .child(dot(side_color(index, cx)))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .when(disabled, |name| name.text_color(muted))
                        .child(side_name(index)),
                )
                .child(div().text_xs().text_color(muted).truncate().max_w(px(360.0)).child(path));
            if let Some(reason) = reason {
                option = option
                    .tooltip(move |window, cx| Tooltip::new(reason.clone()).build(window, cx));
            }
            choices = choices.child(option);
        }

        let trailing: AnyElement = match sync.target {
            None => div()
                .text_xs()
                .text_color(muted)
                .child(if pending {
                    "Available when the comparison finishes."
                } else {
                    "Pick the collection that receives the changes."
                })
                .into_any_element(),
            Some(_) => {
                let total: usize = (0..4)
                    .map(|category| {
                        sync.categories[category].count(tab.segments[category + 1].len())
                    })
                    .sum();
                let reason = app.compare_sync_disabled_reason(id, false);
                let clear_state = self.state.clone();
                let review_state = self.state.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .flex_shrink_0()
                    .child(
                        Button::new("clear-sync-target")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .tooltip("Leave sync mode (Esc)")
                            .on_click(move |_, _, cx| {
                                clear_state.update(cx, |app, cx| {
                                    if let Some(tab) = app.compare_tab_mut(id) {
                                        tab.sync.clear_target();
                                    }
                                    cx.notify();
                                })
                            }),
                    )
                    .child(
                        Button::new("review-sync")
                            .small()
                            .primary()
                            .label(format!("Review and sync {}", format_number(total as u64)))
                            .disabled(total == 0 || reason.is_some())
                            .on_click(move |_, window, cx| {
                                AppCommands::review_compare_sync(
                                    review_state.clone(),
                                    id,
                                    false,
                                    window,
                                    cx,
                                )
                            }),
                    )
                    .into_any_element()
            }
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_x(spacing::lg())
            .gap_y(spacing::xs())
            .child(choices)
            .child(trailing)
    }

    /// What the sync would do to the target, one checkbox per operation, deletes last.
    fn render_sync_operations(
        &self,
        id: Uuid,
        app: &AppState,
        tab: &CompareTabState,
        target: Side,
        cx: &Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let sync = &tab.sync;
        let kinds = [DiffKind::OnlyLeft, DiffKind::OnlyRight, DiffKind::Different, DiffKind::Minor];
        let mut entries: Vec<(usize, Operation, &str)> = kinds
            .into_iter()
            .enumerate()
            .filter_map(|(category, kind)| {
                let operation = operation_for(kind, target)?;
                let noun = match (operation, kind) {
                    (Operation::Insert, _) => "missing",
                    (Operation::Delete, _) => "extra",
                    (_, DiffKind::Minor) => "minor",
                    _ => "changed",
                };
                Some((category, operation, noun))
            })
            .collect();
        entries.sort_by_key(|(category, operation, _)| {
            (
                match operation {
                    Operation::Insert => 0,
                    Operation::Replace => 1,
                    Operation::Delete => 2,
                },
                *category,
            )
        });
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(spacing::lg())
            .gap_y(spacing::xs())
            .min_w_0();
        for (category, operation, noun) in entries {
            let count = tab.segments[category + 1].len();
            if count == 0 {
                continue;
            }
            let selected = sync.categories[category].count(count);
            let value = if selected == 0 {
                CheckboxState::Unchecked
            } else if selected == count {
                CheckboxState::Checked
            } else {
                CheckboxState::Indeterminate
            };
            let verb = match operation {
                Operation::Insert => "Insert",
                Operation::Replace => "Replace",
                Operation::Delete => "Delete",
            };
            let label = if selected == 0 || selected == count {
                format!("{verb} {} {noun}", format_number(count as u64))
            } else {
                format!(
                    "{verb} {} of {} {noun}",
                    format_number(selected as u64),
                    format_number(count as u64)
                )
            };
            let state = self.state.clone();
            row = row.child(
                tri_checkbox(("sync-category", category), value, label, false, cx).on_change(
                    move |value, _, _, cx| {
                        state.update(cx, |app, cx| {
                            if let Some(tab) = app.compare_tab_mut(id) {
                                tab.sync.set_category(category, value == CheckboxState::Checked);
                            }
                            cx.notify();
                        })
                    },
                ),
            );
        }
        let mut notes = Vec::new();
        match tab.segments[5].len() {
            0 => {}
            1 => notes.push("1 key with multiple matches is left alone.".to_string()),
            n => notes.push(format!(
                "{} keys with multiple matches are left alone.",
                format_number(n as u64)
            )),
        }
        if let Some(reason) = app.compare_sync_disabled_reason(id, false) {
            notes.push(reason);
        }
        for text in notes {
            row = row.child(div().text_xs().text_color(muted).child(text));
        }
        row
    }

    /// The run's result line: what was written, then Undo and Compare again.
    fn render_sync_outcome(
        &self,
        id: Uuid,
        app: &AppState,
        tab: &CompareTabState,
        cx: &Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let sync = &tab.sync;
        let summary = &sync.summary;
        let title = format!(
            "{} {}",
            if sync.undoing { "Undo" } else { "Sync" },
            if summary.cancelled {
                "cancelled"
            } else if sync.running {
                "running"
            } else {
                "finished"
            }
        );
        let mut parts = vec![format!(
            "{} {}",
            format_number(summary.written as u64),
            if sync.undoing { "restored" } else { "written" }
        )];
        for (count, word) in [
            (summary.skipped, "skipped"),
            (summary.failed, "failed"),
            (summary.uncertain, "uncertain"),
        ] {
            if count > 0 {
                parts.push(format!("{} {word}", format_number(count as u64)));
            }
        }
        let undo_available =
            !sync.running && sync.restore.as_ref().is_some_and(|r| r.pending() > 0);
        if undo_available {
            parts.push("Undo stays available until this tab closes or you compare again".into());
        }
        let icon: AnyElement = if sync.running {
            Spinner::new().small().into_any_element()
        } else if summary.failed + summary.uncertain > 0 || summary.cancelled {
            Icon::new(IconName::TriangleAlert)
                .small()
                .text_color(cx.theme().warning)
                .into_any_element()
        } else {
            Icon::new(IconName::CircleCheck)
                .small()
                .text_color(cx.theme().success)
                .into_any_element()
        };
        let mut buttons = div().flex().items_center().gap(spacing::sm()).flex_shrink_0();
        if sync.running {
            let state = self.state.clone();
            buttons = buttons.child(
                Button::new("cancel-sync")
                    .small()
                    .outline()
                    .label("Cancel after this batch")
                    .on_click(move |_, _, cx| AppCommands::cancel_compare_sync(&state, id, cx)),
            );
        } else {
            if undo_available {
                let state = self.state.clone();
                buttons = buttons.child(
                    Button::new("undo-sync")
                        .small()
                        .outline()
                        .icon(IconName::Undo2)
                        .label("Undo sync")
                        .disabled(app.compare_sync_disabled_reason(id, true).is_some())
                        .on_click(move |_, window, cx| {
                            AppCommands::review_compare_sync(state.clone(), id, true, window, cx)
                        }),
                );
            }
            let state = self.state.clone();
            buttons = buttons.child(
                Button::new("compare-after-sync")
                    .small()
                    .primary()
                    .label("Compare again")
                    .on_click(move |_, _, cx| AppCommands::run_compare(state.clone(), id, cx)),
            );
        }
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(spacing::md())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .min_w_0()
                    .child(icon)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .flex_shrink_0()
                            .child(title),
                    )
                    .child(div().text_xs().text_color(muted).truncate().child(parts.join(" · "))),
            )
            .child(buttons)
    }
}
