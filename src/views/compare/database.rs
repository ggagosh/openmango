//! Database scope: every collection of two databases, paired by name. The list is alphabetical
//! and never reorders; the detail shares the document diff's columns.

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
use crate::connection::ops::compare_database::{
    CollectionKind, CollectionPair, PairKind, SideCollection,
};
use crate::error::ErrorReport;
use crate::helpers::{format_bytes, format_number};

const COUNT_WIDTH: f32 = 88.0;
const RESULT_WIDTH: f32 = 88.0;
const MARKER_WIDTH: f32 = 12.0;

pub(super) fn pair_label(kind: PairKind) -> &'static str {
    match kind {
        PairKind::LeftOnly => "Left only",
        PairKind::RightOnly => "Right only",
        PairKind::Both => "In both",
        PairKind::NotComparable(CollectionKind::View) => "View",
        PairKind::NotComparable(_) => "Time-series",
    }
}

fn pair_color(kind: PairKind, cx: &App) -> Hsla {
    match kind {
        PairKind::LeftOnly => side_color(0, cx),
        PairKind::RightOnly => side_color(1, cx),
        PairKind::Both | PairKind::NotComparable(_) => cx.theme().muted_foreground,
    }
}

/// A filled dot for a presence, a ring for what is not compared.
fn pair_marker(kind: PairKind, cx: &App) -> Div {
    match kind {
        PairKind::NotComparable(_) => div()
            .size(px(6.0))
            .flex_shrink_0()
            .rounded_full()
            .border_1()
            .border_color(cx.theme().muted_foreground),
        kind => dot(pair_color(kind, cx)),
    }
}

/// `—` where the collection does not exist; estimates carry a `~`.
fn count_text(side: &Option<SideCollection>) -> String {
    match side {
        None => "—".into(),
        Some(side) => side.estimated.map(|n| format!("~{}", format_number(n))).unwrap_or_default(),
    }
}

/// Connection and database; the setup may still hold a collection from the other scope.
fn database_label(app: &AppState, endpoint: &CompareEndpoint) -> String {
    endpoint_label(app, &CompareEndpoint { collection: String::new(), ..endpoint.clone() })
}

fn row_label(pair: &CollectionPair) -> String {
    let counts = pair.sides.each_ref().map(count_text);
    format!(
        "{}, {}, left {}, right {}",
        pair.name,
        pair_label(pair.kind()),
        counts[0].replace('—', "none"),
        counts[1].replace('—', "none")
    )
}

/// Marker, name, two counts and the result: the heading and every row share these widths.
fn pair_columns() -> Div {
    div().size_full().min_w_0().px(spacing::sm()).flex().items_center().gap(spacing::sm())
}

fn count_cell(content: impl IntoElement) -> Div {
    div().w(px(COUNT_WIDTH)).flex_shrink_0().flex().justify_end().truncate().child(content)
}

impl CompareView {
    /// Collection segments and the run's status line.
    pub(super) fn render_database_summary(&self, id: Uuid, cx: &Context<Self>) -> AnyElement {
        let app = self.state.read(cx);
        let tab = app.compare_tab(id).unwrap();
        let appearance = app.settings.appearance.clone();
        let muted = cx.theme().muted_foreground;
        let counts = tab.pair_segments.each_ref().map(Vec::len);
        let segments = [
            (None, "All"),
            (Some(PairKind::LeftOnly), "Left only"),
            (Some(PairKind::RightOnly), "Right only"),
            (Some(PairKind::Both), "In both"),
            (Some(PairKind::NotComparable(CollectionKind::View)), "Not compared"),
        ];
        let mut group = ButtonGroup::new("compare-pair-segments").small();
        for (index, (kind, label)) in segments.into_iter().enumerate() {
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
                            .children(kind.map(|kind| pair_marker(kind, cx)))
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
                            }
                            cx.notify();
                        });
                        scroll.scroll_to_item(0, ScrollStrategy::Top);
                    }),
            );
        }
        let status = if tab.busy() {
            "Listing collections…".to_string()
        } else {
            let mut status = format!("{} collections", format_number(tab.pairs.len() as u64));
            if let Some(at) = tab.compared_at {
                status.push_str(&format!(" · {}", relative_time(at)));
            }
            status
        };
        let mut bar = div()
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
                    .child(div().id("compare-status").text_xs().text_color(muted).child(status)),
            );
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
                        .child(div().w(px(MARKER_WIDTH)).flex_shrink_0())
                        .child(div().flex_1().min_w_0().child("Collection"))
                        .child(side_heading(0))
                        .child(side_heading(1))
                        .child(div().w(px(RESULT_WIDTH)).flex_shrink_0().child("Result")),
                ),
        );
        if count == 0 {
            let (icon, title, detail): (AnyElement, &str, &str) = if tab.running {
                (
                    Spinner::new().small().into_any_element(),
                    "Listing collections…",
                    "Both databases are read at once.",
                )
            } else if tab.pairs.is_empty() && tab.error.is_none() {
                (div().into_any_element(), "No collections", "Neither database has a collection.")
            } else {
                (div().into_any_element(), "Nothing in this segment", "Pick another segment above.")
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
                        range
                            .filter_map(|position| {
                                let index = *tab.visible_pairs().get(position)?;
                                let pair = &tab.pairs[index];
                                let kind = pair.kind();
                                let name = pair.name.clone();
                                let tooltip = name.clone();
                                let state = state.clone();
                                let focus = view.focus.clone();
                                Some(
                                    div()
                                        .id(("compare-pair", index))
                                        .debug_selector(move || format!("compare-pair-{index}"))
                                        .aria_label(row_label(pair))
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
                                                .child(
                                                    div()
                                                        .w(px(MARKER_WIDTH))
                                                        .flex_shrink_0()
                                                        .flex()
                                                        .justify_center()
                                                        .child(pair_marker(kind, cx)),
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
                                                .children(pair.sides.each_ref().map(|side| {
                                                    count_cell(count_text(side))
                                                        .text_xs()
                                                        .text_color(muted)
                                                }))
                                                .child(
                                                    div()
                                                        .w(px(RESULT_WIDTH))
                                                        .flex_shrink_0()
                                                        .truncate()
                                                        .text_xs()
                                                        .text_color(pair_color(kind, cx))
                                                        .child(pair_label(kind)),
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
        let kind = pair.kind();
        let config = tab.results_config();
        let names = config.sides.each_ref().map(|side| database_label(app, side));
        let connected = config
            .sides
            .iter()
            .all(|side| side.connection_id.is_some_and(|id| app.is_connected(id)));
        let appearance = app.settings.appearance.clone();
        let color = pair_color(kind, cx);
        let title = pair.name.clone();
        let state = self.state.clone();
        let header = div()
            .id("compare-pair-toolbar")
            .w_full()
            .flex_shrink_0()
            .px(spacing::md())
            .py(spacing::sm())
            .flex()
            .items_center()
            .gap(spacing::sm())
            .min_h(px(24.0))
            .child(
                div()
                    .id("compare-pair-title")
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(title.clone())
                    .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx)),
            )
            .child(
                Tag::custom(color.opacity(0.12), color, color.opacity(0.35))
                    .xsmall()
                    .child(pair_label(kind)),
            )
            .child(
                Button::new("compare-open-pair")
                    .outline()
                    .small()
                    .label("Open comparison")
                    .disabled(kind != PairKind::Both || !connected)
                    .on_click(move |_, _, cx| open_pair(&state, id, index, cx)),
            )
            .child(gpui_kit::component::kbd::Kbd::new(Keystroke::parse("enter").unwrap()));
        let absent = pair.sides.each_ref().map(|side| side.is_none().then_some("Missing"));
        let fact = |label: &'static str, value: &dyn Fn(&SideCollection) -> String| {
            comparison_row()
                .h(px(26.0))
                .items_center()
                .text_sm()
                .child(field_column().pl(spacing::xs()).text_color(muted).child(label))
                .children(pair.sides.each_ref().map(|side| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .px(spacing::xs())
                        .truncate()
                        .child(side.as_ref().map_or_else(|| "—".to_string(), value))
                }))
        };
        let kind_name = |side: &SideCollection| {
            match side.kind {
                CollectionKind::Collection => "Collection",
                CollectionKind::View => "View",
                CollectionKind::Timeseries => "Time-series",
            }
            .to_string()
        };
        let documents = |side: &SideCollection| {
            side.estimated.map_or_else(|| "Unknown".into(), |n| format!("~{}", format_number(n)))
        };
        let size = |side: &SideCollection| side.bytes.map(format_bytes).unwrap_or_default();
        let mut notes = Vec::new();
        match kind {
            PairKind::Both => {
                notes.push("Counts are estimates from metadata.");
                notes.push("Open the comparison to see which documents differ.");
            }
            PairKind::LeftOnly => notes.push("Only the left database has this collection."),
            PairKind::RightOnly => notes.push("Only the right database has this collection."),
            PairKind::NotComparable(CollectionKind::View) => {
                notes.push("Views are not compared. They are computed from other collections.")
            }
            PairKind::NotComparable(_) => {
                notes.push("Time-series collections cannot be compared yet.")
            }
        }
        if !connected {
            notes.push("Connection closed. Reconnect to open the comparison.");
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
            .child(fact("Kind", &kind_name))
            .child(fact("Documents", &documents))
            .child(fact("Size", &size))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .px(spacing::md())
                    .py(spacing::md())
                    .children(notes.into_iter().map(|text| note(text, cx))),
            )
            .into_any_element()
    }
}

/// Opens the collection in its own Compare tab and runs it there.
pub(super) fn open_pair(state: &Entity<AppState>, id: Uuid, index: usize, cx: &mut App) {
    let opened = state.update(cx, |app, cx| {
        let tab = app.compare_tab(id)?;
        let pair = tab.pairs.get(index)?;
        let connected = tab
            .results_config()
            .sides
            .iter()
            .all(|side| side.connection_id.is_some_and(|id| app.is_connected(id)));
        if pair.kind() != PairKind::Both || !connected {
            return None;
        }
        app.open_pair_comparison(id, index, cx)
    });
    if let Some(opened) = opened {
        AppCommands::run_compare(state.clone(), opened, cx);
    }
}
