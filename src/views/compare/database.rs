//! Database scope: every collection of two databases, paired by name. The list is alphabetical
//! and never reorders while a scan runs; the detail shares the document diff's columns.

use gpui_kit::component::Selectable as _;
use gpui_kit::component::button::{ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;

use super::detail::{comparison_row, diff_heading, field_column};
use super::*;
use crate::components::ErrorCallout;
use crate::components::tri_checkbox::tri_checkbox;
use crate::connection::ops::compare::CompareSummary;
use crate::connection::ops::compare_database::{CollectionKind, PairKind, SideCollection};
use crate::error::ErrorReport;
use crate::helpers::{format_bytes, format_number};
use crate::state::compare::{CompareTabState, PAIR_SEGMENTS, PairProgress, PairStatus};
use crate::state::compare_sync::PairSyncResult;
use gpui_kit::base::CheckboxState;

const COUNT_WIDTH: f32 = 80.0;
const RESULT_WIDTH: f32 = 160.0;
const MARKER_WIDTH: f32 = 12.0;
/// The sync tick box; the heading keeps the same space so the columns stay aligned.
const SYNC_BOX_WIDTH: f32 = 24.0;

fn status_label(status: PairStatus) -> &'static str {
    match status {
        PairStatus::LeftOnly => "Left only",
        PairStatus::RightOnly => "Right only",
        PairStatus::NotComparable(CollectionKind::View) => "View",
        PairStatus::NotComparable(_) => "Time-series",
        PairStatus::Waiting => "Waiting",
        PairStatus::Scanning => "Comparing",
        PairStatus::Different => "Different",
        PairStatus::Minor => "Minor",
        PairStatus::Identical => "Identical",
        PairStatus::Skipped => "Skipped",
        PairStatus::Cancelled => "Cancelled",
        PairStatus::Failed => "Failed",
    }
}

fn status_color(status: PairStatus, cx: &App) -> Hsla {
    match status {
        PairStatus::LeftOnly => side_color(0, cx),
        PairStatus::RightOnly => side_color(1, cx),
        PairStatus::Different => kind_color(DiffKind::Different, cx),
        PairStatus::Identical => cx.theme().success,
        PairStatus::Failed => cx.theme().danger,
        _ => cx.theme().muted_foreground,
    }
}

/// A filled dot for an outcome, a ring for what is pending or was not compared.
fn status_marker(status: PairStatus, cx: &App) -> Div {
    match status {
        PairStatus::Waiting
        | PairStatus::Scanning
        | PairStatus::NotComparable(_)
        | PairStatus::Skipped
        | PairStatus::Cancelled => div()
            .size(px(6.0))
            .flex_shrink_0()
            .rounded_full()
            .border_1()
            .border_color(cx.theme().muted_foreground),
        status => dot(status_color(status, cx)),
    }
}

/// The parts of a result worth a row: what differs, in the order of the segments.
fn summary_parts(summary: &CompareSummary) -> Vec<String> {
    let c = summary.counts;
    [(c.different, "different"), (c.only_left, "left only"), (c.only_right, "right only")]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, label)| format!("{} {label}", format_number(count)))
        .collect()
}

fn percent(read: u64, pair: &crate::connection::ops::compare_database::CollectionPair) -> String {
    let total = pair.sides.iter().flatten().map(|side| side.estimated).sum::<Option<u64>>();
    match total.filter(|total| *total > 0) {
        Some(total) => format!("{}%", (read * 100 / total).min(99)),
        None => format!("{} read", format_number(read)),
    }
}

/// What a row says after its name.
fn row_result(tab: &CompareTabState, index: usize) -> String {
    let undoing = tab.sync.undoing;
    match tab.sync.pairs.get(&index) {
        Some(PairSyncResult::Running(summary)) => {
            return format!(
                "{} · {} {}",
                if undoing { "Undoing" } else { "Syncing" },
                format_number(summary.written as u64),
                if undoing { "restored" } else { "written" }
            );
        }
        Some(PairSyncResult::Failed(_)) => {
            return if undoing { "Undo failed" } else { "Sync failed" }.into();
        }
        _ => {}
    }
    let result = documents_result(tab, index);
    if tab.pairs[index].index_difference().is_some() {
        format!("{result} · indexes differ")
    } else {
        result
    }
}

fn documents_result(tab: &CompareTabState, index: usize) -> String {
    let status = tab.pair_status(index);
    match &tab.pair_progress[index] {
        PairProgress::Scanning(counts) => {
            percent(counts.left_read + counts.right_read, &tab.pairs[index])
        }
        PairProgress::Done(summary) => match status {
            PairStatus::Different => summary_parts(summary).join(" · "),
            PairStatus::Minor => format!("{} minor", format_number(summary.counts.minor)),
            status => status_label(status).into(),
        },
        _ => status_label(status).into(),
    }
}

/// Exact once compared; `~` for estimates from metadata; `—` where the collection is missing.
fn count_text(tab: &CompareTabState, index: usize, side: usize) -> String {
    let Some(collection) = &tab.pairs[index].sides[side] else {
        return "—".into();
    };
    if let PairProgress::Done(summary) = &tab.pair_progress[index] {
        let c = summary.counts;
        return format_number(if side == 0 { c.left_read } else { c.right_read });
    }
    collection.estimated.map(|n| format!("~{}", format_number(n))).unwrap_or_default()
}

/// Connection and database; the setup may still hold a collection from the other scope.
pub(super) fn database_label(app: &AppState, endpoint: &CompareEndpoint) -> String {
    endpoint_label(app, &CompareEndpoint { collection: String::new(), ..endpoint.clone() })
}

/// Marker, name, two counts and the result: the heading and every row share these widths.
fn pair_columns() -> Div {
    div().size_full().min_w_0().px(spacing::sm()).flex().items_center().gap(spacing::sm())
}

/// The sync's own marker for a collection it wrote, in place of the status dot.
fn sync_marker(result: &PairSyncResult, cx: &App) -> AnyElement {
    match result {
        PairSyncResult::Running(_) => Spinner::new().xsmall().into_any_element(),
        PairSyncResult::Done(summary) if summary.failed + summary.uncertain == 0 => {
            Icon::new(IconName::Check).xsmall().text_color(cx.theme().success).into_any_element()
        }
        PairSyncResult::Done(_) => Icon::new(IconName::TriangleAlert)
            .xsmall()
            .text_color(cx.theme().warning)
            .into_any_element(),
        PairSyncResult::Failed(_) => Icon::new(IconName::TriangleAlert)
            .xsmall()
            .text_color(cx.theme().danger)
            .into_any_element(),
    }
}

/// The detail's line about the sync: what it wrote to this collection, or what it would.
fn sync_line(tab: &CompareTabState, index: usize, cx: &App) -> Option<AnyElement> {
    let undoing = tab.sync.undoing;
    if let Some(result) = tab.sync.pairs.get(&index) {
        let (text, color) = match result {
            PairSyncResult::Failed(error) => (
                format!("{}: {error}", if undoing { "Undo failed" } else { "Sync failed" }),
                cx.theme().danger,
            ),
            PairSyncResult::Running(summary) | PairSyncResult::Done(summary) => {
                let running = matches!(result, PairSyncResult::Running(_));
                let mut parts = Vec::new();
                for (count, word) in if undoing {
                    vec![(summary.written, "restored")]
                } else {
                    vec![
                        (summary.inserted, "inserted"),
                        (summary.replaced, "replaced"),
                        (summary.deleted, "deleted"),
                    ]
                }
                .into_iter()
                .chain([
                    (summary.skipped, "skipped"),
                    (summary.failed, "failed"),
                    (summary.uncertain, "uncertain"),
                ]) {
                    if count > 0 {
                        parts.push(format!("{} {word}", format_number(count as u64)));
                    }
                }
                let verb = match (undoing, running) {
                    (false, true) => "Syncing",
                    (false, false) => "Synced",
                    (true, true) => "Undoing",
                    (true, false) => "Undone",
                };
                let parts =
                    if parts.is_empty() { "nothing written".into() } else { parts.join(" · ") };
                (format!("{verb}: {parts}"), cx.theme().foreground)
            }
        };
        return Some(div().text_sm().text_color(color).child(text).into_any_element());
    }
    if tab.sync.target.is_none() || tab.sync.running || tab.sync.completed {
        return None;
    }
    let candidate = tab.sync_candidates().into_iter().find(|c| c.index == index)?;
    let included = !tab.sync.excluded.contains(&index);
    let writes = super::sync_bar::writes_text(candidate.writes, tab.sync.mode, candidate.create);
    let text = match (included, candidate.create) {
        (true, true) => format!("In this sync: created, then {writes}"),
        (true, false) => format!("In this sync: {writes}"),
        (false, _) => format!("Left out of this sync, which would {writes}"),
    };
    Some(note(text, cx).into_any_element())
}

fn count_cell(content: impl IntoElement) -> Div {
    div().w(px(COUNT_WIDTH)).flex_shrink_0().flex().justify_end().truncate().child(content)
}

fn connected(app: &AppState, tab: &CompareTabState) -> bool {
    tab.results_config()
        .sides
        .iter()
        .all(|side| side.connection_id.is_some_and(|id| app.is_connected(id)))
}

impl CompareView {
    /// Collection segments, then the scan's progress or the run's outcome.
    pub(super) fn render_database_summary(&self, id: Uuid, cx: &Context<Self>) -> AnyElement {
        let app = self.state.read(cx);
        let tab = app.compare_tab(id).unwrap();
        let appearance = app.settings.appearance.clone();
        let muted = cx.theme().muted_foreground;
        let counts = tab.pair_segment_counts();
        let segments: [(Option<PairStatus>, &str); PAIR_SEGMENTS] = [
            (None, "All"),
            (Some(PairStatus::LeftOnly), "Left only"),
            (Some(PairStatus::RightOnly), "Right only"),
            (Some(PairStatus::Different), "Different"),
            (Some(PairStatus::Minor), "Minor"),
            (Some(PairStatus::Identical), "Identical"),
            (Some(PairStatus::Skipped), "Not compared"),
        ];
        // Present at zero too, so the strip never shifts while counts arrive.
        let mut group = ButtonGroup::new("compare-pair-segments").small();
        for (index, (status, label)) in segments.into_iter().enumerate() {
            let state = self.state.clone();
            let scroll = self.scroll.clone();
            group = group.child(
                Button::new(("compare-pair-segment", index))
                    .ghost()
                    .small()
                    .selected(tab.pair_segment == index)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .children(status.map(|status| status_marker(status, cx)))
                            .child(label)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(format_number(counts[index] as u64)),
                            ),
                    )
                    .on_click(move |_, _, cx| {
                        state.update(cx, |app, cx| {
                            if let Some(tab) = app.compare_tab_mut(id) {
                                tab.pair_segment = index;
                                tab.rebuild_pair_segments();
                            }
                            cx.notify();
                        });
                        scroll.scroll_to_item(0, ScrollStrategy::Top);
                    }),
            );
        }

        let both: Vec<usize> =
            (0..tab.pairs.len()).filter(|i| tab.pairs[*i].kind() == PairKind::Both).collect();
        let settled = both
            .iter()
            .filter(|i| {
                !matches!(tab.pair_progress[**i], PairProgress::Waiting | PairProgress::Scanning(_))
            })
            .count();
        let (read, estimate) = tab.pair_scan_reads();
        let scanning = tab.busy() && !tab.pairs.is_empty();
        let status: AnyElement = if tab.busy() && tab.pairs.is_empty() {
            note("Listing collections…", cx).into_any_element()
        } else if scanning {
            let elapsed = tab.started.map_or(0.0, |started| started.elapsed().as_secs_f64());
            let rate = if elapsed > 0.0 { (read as f64 / elapsed) as u64 } else { 0 };
            let current = tab.pair_current.map(|index| (index, tab.pairs[index].name.clone()));
            let mut text = format!(
                "{} of {} collections · {} read · {}/s",
                format_number(settled as u64),
                format_number(both.len() as u64),
                format_number(read),
                format_number(rate)
            );
            if let Some((_, name)) = &current {
                text = format!("Comparing {name} · {text}");
            }
            let skip_state = self.state.clone();
            let cancel_state = self.state.clone();
            div()
                .id("compare-status")
                .flex()
                .items_center()
                .gap(spacing::sm())
                .min_w_0()
                .child(div().min_w_0().truncate().text_xs().text_color(muted).child(text))
                // Skip sits beside the name it skips, so its effect is plain.
                .children(current.map(|(index, name)| {
                    Button::new("compare-skip-current")
                        .ghost()
                        .small()
                        .icon(app_icon("skip-forward"))
                        .label(format!("Skip {name}"))
                        .on_click(move |_, _, cx| {
                            AppCommands::skip_database_pair(&skip_state, id, index, cx)
                        })
                }))
                .child(
                    Button::new("compare-cancel")
                        .ghost()
                        .small()
                        .icon(app_icon("circle-stop"))
                        .label("Cancel")
                        .on_click(move |_, _, cx| {
                            AppCommands::cancel_compare(&cancel_state, id, cx)
                        }),
                )
                .into_any_element()
        } else {
            let mut parts = Vec::new();
            if tab.pair_elapsed.is_some() {
                parts.push(format!("{} identical", format_number(counts[5] as u64)));
            }
            if counts[6] > 0 {
                parts.push(format!("{} not compared", format_number(counts[6] as u64)));
            }
            let indexes = tab.pairs.iter().filter(|pair| pair.index_difference().is_some()).count();
            if indexes > 0 {
                parts.push(format!("{} with different indexes", format_number(indexes as u64)));
            }
            if let Some(elapsed) = tab.pair_elapsed {
                parts.push(format_elapsed(elapsed));
            }
            if let Some(at) = tab.compared_at {
                parts.push(relative_time(at));
            }
            div()
                .id("compare-status")
                .text_xs()
                .text_color(muted)
                .child(parts.join(" · "))
                .into_any_element()
        };
        let mut bar = div()
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(spacing::sm())
            .px(spacing::lg())
            .py(spacing::sm())
            .border_b_1()
            .border_color(islands::panel_border(&appearance, cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_x(spacing::md())
                    .gap_y(spacing::xs())
                    .child(div().flex().min_w_0().max_w_full().child(group))
                    .child(status),
            );
        if scanning {
            // The collection scope's 2 px line; indeterminate without estimates.
            let fraction = estimate
                .filter(|total| *total > 0)
                .map(|total| (read as f32 / total as f32).min(1.0));
            bar = bar.child(
                div()
                    .debug_selector(|| "compare-progress".into())
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .h(px(2.0))
                    .w(relative(fraction.unwrap_or(1.0)))
                    .bg(cx.theme().primary)
                    .when(fraction.is_none(), |line| line.opacity(0.35)),
            );
        }
        if let Some(config) = &tab.compared
            && config != &tab.config
        {
            let names = config.sides.each_ref().map(|side| database_label(app, side));
            bar = bar
                .child(note(format!("These results compared {} with {}", names[0], names[1]), cx));
        }
        if let Some(error) = &tab.error {
            let retry = self.state.clone();
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
                        .icon(app_icon("rotate-ccw"))
                        .label("Retry")
                        .disabled(app.compare_disabled_reason(&tab.config).is_some())
                        .on_click(move |_, _, cx| AppCommands::run_compare(retry.clone(), id, cx)),
                ),
            );
        }
        bar.into_any_element()
    }

    pub(super) fn render_database_list(&self, id: Uuid, cx: &Context<Self>) -> AnyElement {
        let tab = self.state.read(cx).compare_tab(id).unwrap();
        let count = tab.visible_pairs().len();
        let muted = cx.theme().muted_foreground;
        // Tick boxes while choosing what to sync; once it runs, rows show its outcome instead.
        let selectable = tab.sync.target.is_some() && !tab.sync.running && !tab.sync.completed;
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
        let side_heading = |side: usize| {
            count_cell(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(dot(side_color(side, cx)))
                    .child(side_name(side)),
            )
        };
        panel = panel.child(
            div()
                .debug_selector(|| "compare-pair-heading".into())
                .h(px(24.0))
                .flex_shrink_0()
                .px(spacing::xs())
                .text_xs()
                .text_color(muted)
                .child(
                    pair_columns()
                        .when(selectable, |row| {
                            row.child(div().w(px(SYNC_BOX_WIDTH)).flex_shrink_0())
                        })
                        .child(div().w(px(MARKER_WIDTH)).flex_shrink_0())
                        .child(div().flex_1().min_w_0().child("Collection"))
                        .child(side_heading(0))
                        .child(side_heading(1))
                        .child(div().w(px(RESULT_WIDTH)).flex_shrink_0().child("Result")),
                ),
        );
        if count == 0 {
            let (icon, title, detail): (AnyElement, &str, String) =
                if tab.running && tab.pairs.is_empty() {
                    (
                        Spinner::new().small().into_any_element(),
                        "Listing collections…",
                        "Both databases are read at once.".into(),
                    )
                } else if tab.pairs.is_empty() && tab.error.is_none() {
                    (div().into_any_element(), "No collections", "Neither database has one.".into())
                } else if tab.pair_segment == 0 && !tab.running {
                    (
                        Icon::new(IconName::CircleCheck)
                            .small()
                            .text_color(cx.theme().success)
                            .into_any_element(),
                        "No differences",
                        format!(
                            "{} collections are identical. Identical lists them.",
                            format_number(tab.pair_segment_counts()[5] as u64)
                        ),
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
        panel
            .child(
                uniform_list(
                    "compare-pairs",
                    count,
                    cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                        let app = state.read(cx);
                        let Some(tab) = app.compare_tab(id) else {
                            return Vec::new();
                        };
                        let candidates: Vec<usize> =
                            tab.sync_candidates().iter().map(|c| c.index).collect();
                        range
                            .filter_map(|position| {
                                let index = *tab.visible_pairs().get(position)?;
                                let status = tab.pair_status(index);
                                let name = tab.pairs[index].name.clone();
                                let tooltip = name.clone();
                                let result = row_result(tab, index);
                                let counts = [0, 1].map(|side| count_text(tab, index, side));
                                let label = format!(
                                    "{name}, {result}, left {}, right {}",
                                    counts[0].replace('—', "none"),
                                    counts[1].replace('—', "none")
                                );
                                let state = state.clone();
                                let toggle_state = state.clone();
                                let focus = view.focus.clone();
                                let include = format!("Include {name} in the sync");
                                let candidate = candidates.contains(&index);
                                let checked = !tab.sync.excluded.contains(&index);
                                let marker = match tab.sync.pairs.get(&index) {
                                    Some(result) => sync_marker(result, cx),
                                    None => status_marker(status, cx).into_any_element(),
                                };
                                Some(
                                    div()
                                        .id(("compare-pair", index))
                                        .debug_selector(move || format!("compare-pair-{index}"))
                                        .aria_label(label)
                                        .h(px(28.0))
                                        .w_full()
                                        .min_w_0()
                                        .px(spacing::xs())
                                        .py(px(1.0))
                                        .child(
                                            pair_columns()
                                                .rounded(borders::radius_sm())
                                                .text_sm()
                                                .text_color(foreground)
                                                .when(tab.pair_selected == Some(index), |row| {
                                                    row.bg(selected_bg)
                                                })
                                                .hover(|row| row.bg(hover))
                                                .cursor_pointer()
                                                .when(selectable, |row| {
                                                    row.child(
                                                        div()
                                                            .w(px(SYNC_BOX_WIDTH))
                                                            .flex_shrink_0()
                                                            .flex()
                                                            .items_center()
                                                            .when(candidate, |cell| {
                                                                cell.child(
                                                                    tri_checkbox(
                                                                        ("sync-pair", index),
                                                                        if checked {
                                                                            CheckboxState::Checked
                                                                        } else {
                                                                            CheckboxState::Unchecked
                                                                        },
                                                                        "",
                                                                        false,
                                                                        cx,
                                                                    )
                                                                    .accessibility_label(include)
                                                                    .on_change(move |_, _, _, cx| {
                                                                        cx.stop_propagation();
                                                                        toggle_state.update(cx, |app, cx| {
                                                                            if let Some(tab) = app.compare_tab_mut(id) {
                                                                                tab.sync.toggle_pair(index);
                                                                            }
                                                                            cx.notify();
                                                                        });
                                                                    }),
                                                                )
                                                            }),
                                                    )
                                                })
                                                .child(
                                                    div()
                                                        .w(px(MARKER_WIDTH))
                                                        .flex_shrink_0()
                                                        .flex()
                                                        .justify_center()
                                                        .child(marker),
                                                )
                                                .child(
                                                    div()
                                                        .id(("compare-pair-name", index))
                                                        .flex_1()
                                                        .min_w_0()
                                                        .truncate()
                                                        .child(name)
                                                        .tooltip(move |window, cx| {
                                                            Tooltip::new(tooltip.clone())
                                                                .build(window, cx)
                                                        }),
                                                )
                                                .children(counts.map(|text| {
                                                    count_cell(text).text_xs().text_color(muted)
                                                }))
                                                .child(
                                                    div()
                                                        .w(px(RESULT_WIDTH))
                                                        .flex_shrink_0()
                                                        .truncate()
                                                        .text_xs()
                                                        .text_color(status_color(status, cx))
                                                        .child(result),
                                                ),
                                        )
                                        .on_click(move |_, window, cx| {
                                            state.update(cx, |app, cx| {
                                                if let Some(tab) = app.compare_tab_mut(id) {
                                                    tab.pair_selected = Some(index);
                                                }
                                                cx.notify();
                                            });
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

    pub(super) fn render_database_detail(&self, id: Uuid, cx: &Context<Self>) -> AnyElement {
        let app = self.state.read(cx);
        let tab = app.compare_tab(id).unwrap();
        let muted = cx.theme().muted_foreground;
        let Some(index) = tab.pair_selected.filter(|index| *index < tab.pairs.len()) else {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(spacing::xs())
                .p(spacing::lg())
                .text_center()
                .child(div().text_sm().text_color(muted).child("Select a collection"))
                .child(note("Its counts and actions appear here.", cx))
                .into_any_element();
        };
        let pair = &tab.pairs[index];
        let status = tab.pair_status(index);
        let progress = tab.pair_progress[index].clone();
        let config = tab.results_config();
        let names = config.sides.each_ref().map(|side| database_label(app, side));
        let connected = connected(app, tab);
        let both = pair.kind() == PairKind::Both;
        let pending = matches!(status, PairStatus::Waiting | PairStatus::Scanning);
        let appearance = app.settings.appearance.clone();
        let color = status_color(status, cx);
        let title = pair.name.clone();
        let open_state = self.state.clone();
        let recheck_state = self.state.clone();
        let skip_state = self.state.clone();
        let header = div()
            .id("compare-pair-toolbar")
            .w_full()
            .flex_shrink_0()
            .px(spacing::md())
            .py(spacing::sm())
            .flex()
            .flex_wrap()
            .items_center()
            .gap(spacing::sm())
            .min_h(px(24.0))
            .child(
                div()
                    .id("compare-pair-title")
                    .flex_1()
                    .min_w(px(80.0))
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(title.clone())
                    .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx)),
            )
            .child(
                Tag::custom(color.opacity(0.12), color, color.opacity(0.35))
                    .xsmall()
                    .child(status_label(status)),
            )
            .when(pending && tab.running, |header| {
                header.child(
                    Button::new("compare-skip-pair")
                        .ghost()
                        .small()
                        .icon(app_icon("skip-forward"))
                        .label("Skip")
                        .on_click(move |_, _, cx| {
                            AppCommands::skip_database_pair(&skip_state, id, index, cx)
                        }),
                )
            })
            .when(both && !pending, |header| {
                header.child(
                    Button::new("compare-recheck-pair")
                        .ghost()
                        .small()
                        .icon(app_icon("rotate-cw"))
                        .label("Recheck")
                        .disabled(tab.running || tab.sync.running || !connected)
                        .on_click(move |_, _, cx| {
                            AppCommands::recheck_database_pair(recheck_state.clone(), id, index, cx)
                        }),
                )
            })
            .child(match pair.kind() {
                PairKind::LeftOnly | PairKind::RightOnly => Button::new("compare-copy-pair")
                    .outline()
                    .small()
                    .icon(app_icon("copy-plus"))
                    .label(if pair.kind() == PairKind::LeftOnly {
                        "Copy to Right…"
                    } else {
                        "Copy to Left…"
                    })
                    .tooltip("Opens Transfer to review the copy. Nothing is written until it runs.")
                    .disabled(!connected)
                    .on_click(move |_, _, cx| open_pair(&open_state, id, index, cx)),
                _ => Button::new("compare-open-pair")
                    .outline()
                    .small()
                    .icon(app_icon("git-compare-arrows"))
                    .label("Open comparison")
                    .disabled(!both || !connected)
                    .on_click(move |_, _, cx| open_pair(&open_state, id, index, cx)),
            })
            .child(gpui_kit::component::kbd::Kbd::new(Keystroke::parse("enter").unwrap()));
        let absent = pair.sides.each_ref().map(|side| side.is_none().then_some("Missing"));
        let fact = |label: &'static str, values: [String; 2]| {
            comparison_row()
                .h(px(26.0))
                .items_center()
                .text_sm()
                .child(field_column().pl(spacing::xs()).text_color(muted).child(label))
                .children(values.map(|value| {
                    div().flex_1().min_w_0().px(spacing::xs()).truncate().child(value)
                }))
        };
        let per_side = |value: &dyn Fn(&SideCollection) -> String| {
            pair.sides.each_ref().map(|side| side.as_ref().map_or_else(|| "—".to_string(), value))
        };
        let kinds = per_side(&|side| {
            match side.kind {
                CollectionKind::Collection => "Collection",
                CollectionKind::View => "View",
                CollectionKind::Timeseries => "Time-series",
            }
            .into()
        });
        let documents = [0, 1].map(|side| match count_text(tab, index, side) {
            text if text.is_empty() => "Unknown".into(),
            text => text,
        });
        let sizes = per_side(&|side| side.bytes.map(format_bytes).unwrap_or_default());
        let index_counts = per_side(&|side| match (&side.indexes, side.kind) {
            (Some(indexes), _) => format_number(indexes.len() as u64),
            (None, CollectionKind::Collection) => "Unknown".into(),
            (None, _) => String::new(),
        });

        let mut body =
            div().flex().flex_col().gap(spacing::xs()).px(spacing::md()).py(spacing::md());
        if let Some(line) = sync_line(tab, index, cx) {
            body = body.child(line);
        }
        if let PairProgress::Done(summary) = &progress {
            let c = summary.counts;
            for (count, label, color) in [
                (c.different, "different", kind_color(DiffKind::Different, cx)),
                (c.only_left, "left only", side_color(0, cx)),
                (c.only_right, "right only", side_color(1, cx)),
                (c.minor, "minor", kind_color(DiffKind::Minor, cx)),
                (c.identical, "identical", cx.theme().success),
            ] {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::xs())
                        .text_sm()
                        .child(dot(color))
                        .child(format!("{} {label}", format_number(count))),
                );
            }
            body = body.child(note(format!("Compared in {}", format_elapsed(summary.elapsed)), cx));
            if c.identical + c.different + c.minor == 0 && c.only_left > 0 && c.only_right > 0 {
                body = body.child(note(
                    "No document matched by _id. They were probably inserted separately. Open the comparison and match by another field.",
                    cx,
                ));
            } else if status == PairStatus::Different {
                body = body.child(note(
                    "Open the comparison to see which documents differ and sync them.",
                    cx,
                ));
            }
        }
        let message: Option<String> = match (&progress, status) {
            (PairProgress::Scanning(counts), _) => Some(format!(
                "Comparing: {} documents read.",
                format_number(counts.left_read + counts.right_read)
            )),
            (PairProgress::Waiting, _) => {
                Some("Waiting for its turn. Skip leaves it out of this run.".into())
            }
            (PairProgress::Skipped, _) if config.skip.contains(&pair.name) => {
                Some("Listed under Skip collections in Settings.".into())
            }
            (PairProgress::Skipped, _) => Some("Skipped. Recheck compares it on its own.".into()),
            (PairProgress::Cancelled, _) => {
                Some("The run was cancelled before this collection finished.".into())
            }
            (_, PairStatus::LeftOnly) => Some("Only the left database has this collection.".into()),
            (_, PairStatus::RightOnly) => {
                Some("Only the right database has this collection.".into())
            }
            (_, PairStatus::NotComparable(CollectionKind::View)) => {
                Some("Views are not compared. They are computed from other collections.".into())
            }
            (_, PairStatus::NotComparable(_)) => {
                Some("Time-series collections cannot be compared yet.".into())
            }
            _ => None,
        };
        if let Some(difference) = pair.index_difference() {
            for (side, indexes) in difference.into_iter().enumerate() {
                if indexes.is_empty() {
                    continue;
                }
                body = body
                    .child(
                        div()
                            .pt(spacing::xs())
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child(dot(side_color(side, cx)))
                            .child(format!(
                                "Indexes on the {} only",
                                side_name(side).to_lowercase()
                            )),
                    )
                    .children(
                        indexes
                            .into_iter()
                            .map(|index| div().pl(px(10.0)).text_xs().truncate().child(index)),
                    );
            }
        }
        body = body.children(message.map(|message| note(message, cx)));
        if both && !matches!(progress, PairProgress::Done(_)) {
            body = body.child(note("Counts are estimates from metadata.", cx));
        }
        if let PairProgress::Failed(error) = &progress {
            let retry = self.state.clone();
            body = body.child(
                ErrorCallout::new(
                    format!("compare-pair-error-{index}"),
                    ErrorReport::new("Couldn't compare this collection", error.clone()),
                )
                .compact()
                .state(self.state.clone())
                .action(
                    Button::new("compare-pair-retry")
                        .ghost()
                        .xsmall()
                        .icon(app_icon("rotate-ccw"))
                        .label("Retry")
                        .disabled(tab.running || !connected)
                        .on_click(move |_, _, cx| {
                            AppCommands::recheck_database_pair(retry.clone(), id, index, cx)
                        }),
                ),
            );
        }
        if !connected {
            body = body.child(note("Connection closed. Reconnect to compare.", cx));
        }
        div()
            .debug_selector(|| "compare-pair-detail".into())
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .track_focus(&self.detail_focus)
            .overflow_hidden()
            .child(header)
            .child(diff_heading(names, absent, &appearance, cx))
            .child(fact("Kind", kinds))
            .child(fact("Documents", documents))
            .child(fact("Size", sizes))
            .child(fact("Indexes", index_counts))
            .child(div().flex_1().min_h_0().child(body.overflow_y_scrollbar()))
            .into_any_element()
    }
}

/// The row's main action, also on `enter`: open the collection in its own Compare tab and run
/// it there, or, for a collection on one side only, open Transfer to copy it across.
pub(super) fn open_pair(state: &Entity<AppState>, id: Uuid, index: usize, cx: &mut App) {
    let opened = state.update(cx, |app, cx| {
        let tab = app.compare_tab(id)?;
        let kind = tab.pairs.get(index)?.kind();
        if !connected(app, tab) {
            return None;
        }
        match kind {
            PairKind::Both => app.open_pair_comparison(id, index, cx),
            PairKind::LeftOnly | PairKind::RightOnly => {
                app.open_pair_copy(id, index, cx);
                None
            }
            PairKind::NotComparable(_) => None,
        }
    });
    if let Some(opened) = opened {
        AppCommands::run_compare(state.clone(), opened, cx);
    }
}
