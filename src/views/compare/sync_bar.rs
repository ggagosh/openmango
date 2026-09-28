use gpui_kit::base::CheckboxState;
use gpui_kit::component::button::ButtonGroup;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Selectable as _, Size};

use super::database::database_label;
use super::*;
use crate::components::tri_checkbox::tri_checkbox;
use crate::connection::ops::compare::{DiffKind, Side};
use crate::connection::ops::compare_database::SyncMode;
use crate::connection::ops::compare_sync::{Operation, operation_for};
use crate::helpers::format_number;
use crate::state::compare::{CompareScope, CompareTabState};

fn databases(tab: &CompareTabState) -> bool {
    tab.results_config().scope == CompareScope::Databases
}

/// "insert 2, replace 3, delete 5", naming only what `mode` writes; `~` marks an estimate.
pub(super) fn writes_text(writes: [u64; 3], mode: SyncMode, estimated: bool) -> String {
    let mut parts =
        vec![format!("insert {}{}", if estimated { "~" } else { "" }, format_number(writes[0]))];
    if mode != SyncMode::AddMissing {
        parts.push(format!("replace {}", format_number(writes[1])));
    }
    if mode == SyncMode::Mirror {
        parts.push(format!("delete {}", format_number(writes[2])));
    }
    parts.join(", ")
}

fn collections(count: usize) -> String {
    format!("{} collection{}", format_number(count as u64), if count == 1 { "" } else { "s" })
}

/// "Read from ● Left … → Write to ● Right …": which side the documents come from and which
/// one is written, and for databases what the mode does. The side written stands out.
fn render_sync_direction(
    app: &AppState,
    tab: &CompareTabState,
    target: Side,
    cx: &Context<CompareView>,
) -> Div {
    let theme = cx.theme();
    let (muted, foreground) = (theme.muted_foreground, theme.foreground);
    let config = tab.results_config();
    let databases = databases(tab);
    let into = if target == Side::Left { 0 } else { 1 };
    let side = |index: usize, verb: &'static str| {
        let written = index == into;
        let place = if databases {
            database_label(app, &config.sides[index])
        } else {
            endpoint_label(app, &config.sides[index])
        };
        div()
            .flex()
            .items_center()
            .gap(spacing::xs())
            .min_w_0()
            .child(div().flex_none().child(verb))
            .child(dot(side_color(index, cx)))
            .child(
                div()
                    .flex_none()
                    .when(written, |name| {
                        name.text_color(foreground).font_weight(FontWeight::MEDIUM)
                    })
                    .child(side_name(index)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .when(written, |path| path.text_color(foreground))
                    .child(format!("· {place}")),
            )
    };
    let mut line = div()
        .debug_selector(|| "compare-sync-direction".into())
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(spacing::sm())
        .gap_y(spacing::xs())
        .min_w_0()
        .text_xs()
        .text_color(muted)
        .child(side(1 - into, "Read from"))
        .child(Icon::new(IconName::ArrowRight).xsmall().text_color(muted))
        .child(side(into, "Write to"));
    if databases {
        let mode = tab.sync.mode;
        // Mirror deletes: say so where the mode is chosen, before Review.
        line = line.child(
            div()
                .ml_auto()
                .when(mode == SyncMode::Mirror, |note| note.text_color(theme.warning))
                .child(mode.note()),
        );
    }
    line
}

impl CompareView {
    /// Footer: pick the collection to change, choose what to write, review. Then the run's
    /// outcome with undo.
    pub(super) fn render_sync_bar(&self, id: Uuid, cx: &Context<Self>) -> AnyElement {
        let app = self.state.read(cx);
        let tab = app.compare_tab(id).unwrap();
        // Present from the first run on, so the tab does not grow and shrink around each scan.
        if tab.compared.is_none() || (databases(tab) && tab.pairs.is_empty()) {
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
                bar = bar.child(render_sync_direction(app, tab, target, cx));
                bar = bar.child(if databases(tab) {
                    self.render_sync_modes(id, app, tab, target, cx)
                } else {
                    self.render_sync_operations(id, app, tab, target, cx)
                });
            }
        }
        if let Some(error) = &sync.error {
            bar = bar.child(div().text_xs().text_color(cx.theme().danger).child(error.clone()));
        }
        bar.into_any_element()
    }

    /// The primary row: "Sync to" Left or Right, for databases "Write" and a mode, then Cancel
    /// and Review. Before a side is picked, a hint instead of the actions.
    fn render_sync_targets(
        &self,
        id: Uuid,
        app: &AppState,
        tab: &CompareTabState,
        cx: &Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let sync = &tab.sync;
        let databases = databases(tab);
        let finished = if databases { tab.pair_elapsed.is_some() } else { tab.summary.is_some() };
        let pending = tab.busy() || !finished;
        let config = tab.results_config();
        let field = |label: &'static str, control: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .flex_none()
                .child(
                    div().text_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child(label),
                )
                .child(control)
        };

        let mut sides = ButtonGroup::new("sync-targets").compact();
        for (index, target) in [Side::Left, Side::Right].into_iter().enumerate() {
            let reason =
                if pending { None } else { app.compare_sync_target_disabled_reason(id, index) };
            let path = if databases {
                database_label(app, &config.sides[index])
            } else {
                endpoint_label(app, &config.sides[index])
            };
            let mut side = Button::new(("sync-target", index))
                .with_size(Size::Small)
                .selected(sync.target == Some(target))
                .disabled(pending || reason.is_some())
                .accessibility_label(format!("Sync to {} · {path}", side_name(index)))
                .child(dot(side_color(index, cx)))
                .child(side_name(index));
            if let Some(reason) = reason {
                side = side.tooltip(reason);
            }
            sides = sides.child(side);
        }
        let state = self.state.clone();
        let sides = sides.on_click(move |selection: &Vec<usize>, _, cx| {
            // A greyed-out side reports the current choice, or none.
            let Some(&index) = selection.first() else {
                return;
            };
            state.update(cx, |app, cx| {
                if let Some(tab) = app.compare_tab_mut(id) {
                    tab.sync.set_target(if index == 0 { Side::Left } else { Side::Right });
                }
                cx.notify();
            })
        });
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(spacing::lg())
            .gap_y(spacing::sm())
            .min_w_0()
            .child(field("Sync to", sides.into_any_element()));

        let Some(_) = sync.target else {
            let hint = if pending {
                "Available when the comparison finishes."
            } else {
                "Pick the side that receives the changes."
            };
            return row.child(div().ml_auto().text_xs().text_color(muted).child(hint));
        };

        if databases {
            let state = self.state.clone();
            let modes = ButtonGroup::new("sync-modes")
                .compact()
                .children(SyncMode::ALL.into_iter().enumerate().map(|(index, mode)| {
                    Button::new(("sync-mode", index))
                        .with_size(Size::Small)
                        .label(mode.label())
                        .selected(sync.mode == mode)
                }))
                .on_click(move |selection: &Vec<usize>, _, cx| {
                    let Some(&index) = selection.first() else {
                        return;
                    };
                    state.update(cx, |app, cx| {
                        if let Some(tab) = app.compare_tab_mut(id) {
                            tab.sync.set_mode(SyncMode::ALL[index]);
                        }
                        cx.notify();
                    })
                });
            row = row.child(field("Write", modes.into_any_element()));
        }

        let total: usize = if databases {
            tab.sync_selected().len()
        } else {
            (0..4)
                .map(|category| sync.categories[category].count(tab.segments[category + 1].len()))
                .sum()
        };
        let reason = app.compare_sync_disabled_reason(id, false);
        let clear_state = self.state.clone();
        let review_state = self.state.clone();
        // No count on the button when there's nothing to write; the summary says why.
        let review = match (total, databases) {
            (0, _) => "Review and sync".to_string(),
            (total, true) => format!("Review and sync {}", collections(total)),
            (total, false) => format!(
                "Review and sync {} document{}",
                format_number(total as u64),
                if total == 1 { "" } else { "s" }
            ),
        };
        row.child(
            div()
                .ml_auto()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .flex_none()
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
                        .label(review)
                        .disabled(total == 0 || reason.is_some())
                        .on_click(move |_, window, cx| {
                            if databases {
                                AppCommands::review_database_sync(
                                    review_state.clone(),
                                    id,
                                    false,
                                    window,
                                    cx,
                                )
                            } else {
                                AppCommands::review_compare_sync(
                                    review_state.clone(),
                                    id,
                                    false,
                                    window,
                                    cx,
                                )
                            }
                        }),
                ),
        )
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
        // No checkbox to show: say why there's nothing to write.
        if entries.iter().all(|(category, ..)| tab.segments[category + 1].is_empty()) {
            row = row.child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div().text_sm().font_weight(FontWeight::MEDIUM).child("Nothing to write"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("No documents differ between the two collections."),
                    ),
            );
        }
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

    /// Database scope: what the sync would write, or why nothing, and what it leaves alone.
    fn render_sync_modes(
        &self,
        id: Uuid,
        app: &AppState,
        tab: &CompareTabState,
        target: Side,
        cx: &Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let mode = tab.sync.mode;
        let index = if target == Side::Left { 0 } else { 1 };
        let receiver = side_name(index);
        let selected = tab.sync_selected();
        let mut writes = [0u64; 3];
        for candidate in &selected {
            for (total, count) in writes.iter_mut().zip(candidate.writes) {
                *total += count;
            }
        }
        let estimated = selected.iter().any(|c| c.create);
        let (headline, detail) = if tab.sync_candidates().is_empty() {
            let lacks = match mode {
                SyncMode::AddMissing => "missing",
                SyncMode::AddAndUpdate => "missing or changed",
                SyncMode::Mirror => "missing, changed or extra",
            };
            (
                "Nothing to write".to_string(),
                format!("{receiver} has no {lacks} documents in the compared collections."),
            )
        } else if selected.is_empty() {
            (
                "No collections ticked".to_string(),
                "Tick the collections to sync in the list above.".to_string(),
            )
        } else {
            (
                writes_text(writes, mode, estimated).replacen("insert", "Insert", 1),
                format!(
                    "{} in {}",
                    collections(selected.len()),
                    tab.results_config().sides[index].database
                ),
            )
        };
        let unchanged = "Collections only on the target, views, time-series and minor differences are left alone.";
        div()
            .flex()
            .flex_wrap()
            .items_start()
            .justify_between()
            .gap_x(spacing::lg())
            .gap_y(spacing::xs())
            .child(
                // Takes the row's free space: sized to its text, it can squeeze to a letter.
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .debug_selector(|| "compare-sync-totals".into())
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(headline),
                    )
                    .child(div().text_xs().text_color(muted).child(detail))
                    .children(
                        app.compare_sync_disabled_reason(id, false)
                            .map(|reason| div().text_xs().text_color(muted).child(reason)),
                    ),
            )
            .child(
                Button::new("sync-unchanged")
                    .flex_none()
                    .ghost()
                    .xsmall()
                    .label("What stays unchanged?")
                    .tooltip(unchanged)
                    .accessibility_label(unchanged),
            )
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
        let databases = databases(tab);
        let totals = tab.pair_sync_totals();
        let summary = if databases { &totals } else { &sync.summary };
        let title = format!(
            "{} {}",
            if sync.undoing {
                "Undo"
            } else if sync.field_copies {
                "Copy"
            } else {
                "Sync"
            },
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
        if databases {
            let failed = sync
                .pairs
                .values()
                .filter(|result| {
                    matches!(result, crate::state::compare_sync::PairSyncResult::Failed(_))
                })
                .count();
            if let Some(name) = sync.pair_current.and_then(|index| tab.pairs.get(index)) {
                parts.insert(0, name.name.clone());
            }
            parts.push(collections(sync.pairs.len()));
            if failed > 0 {
                parts.push(format!("{} failed", collections(failed)));
            }
        }
        for (count, word) in [
            (summary.skipped, "skipped"),
            (summary.failed, "failed"),
            (summary.uncertain, "uncertain"),
        ] {
            if count > 0 {
                parts.push(format!("{} {word}", format_number(count as u64)));
            }
        }
        let undo_available = !sync.running
            && if databases {
                sync.logs.iter().any(|(_, _, log)| log.pending() > 0)
            } else {
                sync.restore.as_ref().is_some_and(|r| r.pending() > 0)
            };
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
                    .icon(app_icon("circle-stop"))
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
                        .label(if sync.field_copies { "Undo copies" } else { "Undo sync" })
                        .disabled(app.compare_sync_disabled_reason(id, true).is_some())
                        .on_click(move |_, window, cx| {
                            if databases {
                                AppCommands::review_database_sync(
                                    state.clone(),
                                    id,
                                    true,
                                    window,
                                    cx,
                                )
                            } else {
                                AppCommands::review_compare_sync(
                                    state.clone(),
                                    id,
                                    true,
                                    window,
                                    cx,
                                )
                            }
                        }),
                );
            }
            let state = self.state.clone();
            buttons = buttons.child(
                Button::new("compare-after-sync")
                    .small()
                    .primary()
                    .icon(app_icon("rotate-cw"))
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
