use std::sync::Arc;

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::dialog::{Dialog, DialogFooter};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{WindowExt as _, h_flex};
use mongodb::bson::{Bson, Document};

pub(super) use super::detail_tree::{DetailRow, detail_rows};
use super::detail_tree::{Expansion, label};
use super::*;
use crate::bson::compare::ChangeKind;
use crate::bson::{PathSegment, bson_value_preview, get_bson_at_path};
use crate::connection::ops::compare::DiffKind;
use crate::connection::ops::compare_sync::RowOutcome;
use crate::state::AppearanceSettings;
use crate::state::compare::{CompareConfig, CompareDetail};
use crate::views::documents::table::cell_renderer::{value_color, value_details_tooltip};

/// The field-by-field table for one document pair. The compare tab shows the selected
/// difference in it; the two-document dialog shows a pair picked in a collection.
pub(crate) struct DiffTable {
    pair: Arc<CompareDetail>,
    config: CompareConfig,
    rows: Vec<DetailRow>,
    expansion: Expansion,
    pub(super) error: Option<String>,
    scroll: UniformListScrollHandle,
}

impl DiffTable {
    pub(super) fn new(pair: Arc<CompareDetail>, config: CompareConfig) -> Self {
        let mut table = Self {
            pair,
            config,
            rows: Vec::new(),
            expansion: Default::default(),
            error: None,
            scroll: UniformListScrollHandle::new(),
        };
        table.rebuild();
        table
    }

    fn rebuild(&mut self) {
        match detail_rows(&self.pair, &self.config, &self.expansion) {
            Ok(rows) => {
                self.rows = rows;
                self.error = None;
            }
            Err(error) => {
                self.rows.clear();
                self.error = Some(error.to_string());
            }
        }
    }
}

impl Render for DiffTable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "compare-detail-body".into())
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .min_w_0()
            .w_full()
            .overflow_hidden()
            .child(
                uniform_list(
                    "compare-document-diff",
                    self.rows.len(),
                    cx.processor(|table, range: std::ops::Range<usize>, _, cx| {
                        range
                            .filter_map(|index| {
                                let row = table.rows.get(index)?;
                                Some(render_row(index, row, &table.pair, &table.expansion, cx))
                            })
                            .collect()
                    }),
                )
                .flex_1()
                .w_full()
                .track_scroll(&self.scroll),
            )
            .vertical_scrollbar(&self.scroll)
    }
}

/// Two documents picked in a collection view, side by side. Nothing is matched or written.
pub(crate) fn open_document_compare(
    state: Entity<AppState>,
    namespace: String,
    documents: [Document; 2],
    window: &mut Window,
    cx: &mut App,
) {
    // Picked documents are not paired by _id, so it shows as information, as with a custom key.
    let config = CompareConfig { fields: Vec::new(), ..Default::default() };
    let summary = match crate::bson::compare::field_changes(
        &documents[0],
        &documents[1],
        &config.ignore_set(),
    ) {
        Ok(changes) if changes.is_empty() => "No differences".to_string(),
        Ok(changes) if changes.len() == 1 => "1 difference".to_string(),
        Ok(changes) => format!("{} differences", changes.len()),
        Err(error) => error.to_string(),
    };
    let names = documents.each_ref().map(|document| {
        document.get("_id").map_or_else(|| "No _id".into(), |id| bson_value_preview(id, 80))
    });
    let pair = Arc::new(CompareDetail {
        documents: documents.map(|document| vec![document]),
        changed_since_scan: false,
    });
    let table = cx.new(|_| DiffTable::new(pair.clone(), config));
    window.open_dialog(cx, move |dialog: Dialog, window, cx| {
        let appearance = state.read(cx).settings.appearance.clone();
        dialog
            .title("Compare documents")
            .w(px(960.0))
            .child(
                div()
                    .debug_selector(|| "document-compare".into())
                    .h((window.viewport_size().height * 0.6).clamp(px(240.0), px(640.0)))
                    .flex()
                    .flex_col()
                    .gap(spacing::sm())
                    .child(note(format!("{namespace} · {summary}"), cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .border_1()
                            .border_color(islands::panel_border(&appearance, cx))
                            .rounded(borders::radius_sm())
                            .overflow_hidden()
                            .child(diff_heading(names.clone(), [None, None], &appearance, cx))
                            .child(table.clone()),
                    ),
            )
            .footer(
                DialogFooter::new().child(
                    Button::new("document-compare-close")
                        .icon(IconName::Close)
                        .label("Close")
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}

/// Column titles for a pair. They share the rows' columns, even when a side is absent; `absent`
/// names what that side lacks.
pub(super) fn diff_heading(
    names: [String; 2],
    absent: [Option<&'static str>; 2],
    appearance: &AppearanceSettings,
    cx: &App,
) -> Div {
    let muted = cx.theme().muted_foreground;
    let mut heading = comparison_row()
        .flex_shrink_0()
        .h(px(28.0))
        .items_center()
        .border_b_1()
        .border_color(islands::panel_border(appearance, cx))
        .bg(islands::tool_bg(appearance, cx))
        .child(
            field_column()
                .debug_selector(|| "compare-heading-field".into())
                .flex()
                .items_center()
                .child(div().text_xs().text_color(muted).child("Field")),
        );
    for (side, name) in names.into_iter().enumerate() {
        let missing = absent[side];
        heading = heading.child(
            h_flex()
                .debug_selector(move || format!("compare-heading-{side}"))
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .px(spacing::xs())
                .gap(spacing::sm())
                .child(
                    h_flex()
                        .gap(spacing::xs())
                        .flex_shrink_0()
                        .child(dot(side_color(side, cx)))
                        .child(
                            div().text_xs().font_weight(FontWeight::MEDIUM).child(side_name(side)),
                        ),
                )
                .child(
                    div()
                        .id(("compare-column-name", side))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(muted)
                        .child(name.clone())
                        .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx)),
                )
                .when_some(missing, |column, missing| {
                    column.child(Tag::secondary().xsmall().child(missing))
                }),
        );
    }
    heading
}

impl CompareView {
    pub(super) fn render_detail(&mut self, id: Uuid, cx: &mut Context<Self>) -> AnyElement {
        let app = self.state.read(cx);
        let tab = app.compare_tab(id).unwrap();
        let appearance = app.settings.appearance.clone();
        let muted = cx.theme().muted_foreground;
        let config = tab.results_config().clone();
        let Some(selected) = tab.selected else {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(spacing::xs())
                .p(spacing::lg())
                .text_center()
                .child(div().text_sm().text_color(muted).child("Select a difference"))
                .child(note("Both documents appear here, field by field.", cx))
                .into_any_element();
        };
        let row = tab.rows[selected].clone();
        let outcome = tab.sync.outcomes.get(&selected).cloned();
        let detail = tab.detail.clone();
        let loaded_row = tab.detail_row;
        let error = tab.detail_error.clone();
        let slow = tab.detail_loading && tab.detail_slow;
        let enabled = config
            .sides
            .iter()
            .all(|side| side.connection_id.is_some_and(|id| app.is_connected(id)));
        let names = config.sides.each_ref().map(|side| endpoint_label(app, side));
        let signature = detail
            .as_ref()
            .and_then(|pair| loaded_row.map(|row| (id, tab.run, row, Arc::as_ptr(pair) as usize)));
        if self.detail_signature != signature {
            self.detail_signature = signature;
            self.diff = detail.clone().map(|pair| cx.new(|_| DiffTable::new(pair, config.clone())));
        }
        let mut panel = div()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .track_focus(&self.detail_focus)
            .overflow_hidden();

        let selected_key = super::results::key_label(&row);
        // After a sync the row's kind is history; the tag says what happened to it.
        let (color, tag_label) = match &outcome {
            Some(RowOutcome::Written) => (cx.theme().success, "Synced"),
            Some(RowOutcome::Restored) => (cx.theme().success, "Restored"),
            Some(RowOutcome::Skipped(_)) => (muted, "Skipped"),
            Some(RowOutcome::Failed(_) | RowOutcome::Uncertain(_)) => (cx.theme().danger, "Failed"),
            None => (kind_color(row.kind, cx), kind_label(row.kind)),
        };
        let copy_pair = detail.clone();
        let open_state = self.state.clone();
        let open_config = config.clone();
        let open_row = row.clone();
        let mut header = div()
            .id("compare-detail-toolbar")
            .debug_selector(|| "compare-detail-toolbar".into())
            .w_full()
            .flex_shrink_0()
            .px(spacing::md())
            .py(spacing::sm())
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .min_h(px(24.0))
                    .child(
                        div()
                            .id("compare-detail-key")
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(selected_key.clone())
                            .tooltip(move |window, cx| {
                                Tooltip::new(selected_key.clone()).build(window, cx)
                            }),
                    )
                    .child(
                        Tag::custom(color.opacity(0.12), color, color.opacity(0.35))
                            .xsmall()
                            .child(tag_label),
                    )
                    .child(
                        Button::new("compare-copy")
                            .ghost()
                            .small()
                            .icon(IconName::Copy)
                            .tooltip("Copy both documents as Extended JSON")
                            .disabled(detail.is_none() || loaded_row != Some(selected))
                            .on_click(move |_, _, cx| {
                                if let Some(pair) = &copy_pair {
                                    let value = Bson::Document(mongodb::bson::doc! {
                                        "left": pair.documents[0].clone(),
                                        "right": pair.documents[1].clone(),
                                    });
                                    if let Ok(text) = serde_json::to_string_pretty(
                                        &value.into_canonical_extjson(),
                                    ) {
                                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                                    }
                                }
                            }),
                    )
                    .child(
                        Button::new("compare-open")
                            .ghost()
                            .small()
                            .icon(app_icon("square-arrow-out-up-right"))
                            .label("Open in…")
                            .dropdown_caret(true)
                            .disabled(!enabled)
                            .dropdown_menu(move |mut menu, _, _| {
                                for (index, label) in
                                    ["Open in Left", "Open in Right"].into_iter().enumerate()
                                {
                                    let state = open_state.clone();
                                    let endpoint = open_config.sides[index].clone();
                                    let filter = crate::state::commands::compare::row_filter(
                                        &open_config,
                                        &open_row,
                                        index,
                                    )
                                    .ok();
                                    menu = menu.item(
                                        PopupMenuItem::new(label)
                                            .icon(app_icon("square-arrow-out-up-right"))
                                            .disabled(filter.is_none())
                                            .on_click(move |_, _, cx| {
                                                if let Some(filter) = &filter {
                                                    open_side(&state, &endpoint, filter, cx);
                                                }
                                            }),
                                    );
                                }
                                menu
                            }),
                    ),
            );
        let tree_error = self.diff.as_ref().and_then(|table| table.read(cx).error.clone());
        if let Some(error) = error.or(tree_error) {
            header = header.child(div().text_xs().text_color(cx.theme().danger).child(error));
        }
        if !enabled {
            header = header.child(note(
                "Connection closed. Previously fetched documents remain readable.",
                cx,
            ));
        }
        if slow {
            header = header.child(note("Loading both documents…", cx));
        }
        if detail.is_some() && loaded_row != Some(selected) {
            header = header.child(note("Showing the previous document while this one loads.", cx));
        }
        if detail.as_ref().is_some_and(|pair| pair.changed_since_scan) {
            header = header.child(note("Changed since the comparison ran.", cx));
        }
        panel = panel.child(header);

        let Some(detail) = detail else {
            if slow {
                panel = panel.children((0..5).map(|index| {
                    div()
                        .mx(spacing::md())
                        .my(px(6.0))
                        .h(px(12.0))
                        .w(relative(0.35 + 0.1 * (index % 3) as f32))
                        .rounded(borders::radius_sm())
                        .bg(cx.theme().secondary)
                }));
            }
            return panel.into_any_element();
        };

        if row.kind == DiffKind::MultipleMatches && loaded_row == Some(selected) {
            let mut matches = div()
                .flex_1()
                .min_h_0()
                .w_full()
                .flex()
                .flex_col()
                .gap(spacing::md())
                .px(spacing::md())
                .py(spacing::sm());
            for (side, documents) in detail.documents.iter().enumerate() {
                let count = if side == 0 { row.left_count } else { row.right_count };
                let mut group = div().flex().flex_col().gap(px(2.0)).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::xs())
                        .child(dot(side_color(side, cx)))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .child(format!("{} · {} matches", names[side], count)),
                        )
                        .when(count > 20, |group| group.child(note("showing 20", cx))),
                );
                for document in documents {
                    group = group.child(
                        div()
                            .w_full()
                            .pl(px(10.0))
                            .text_xs()
                            .truncate()
                            .child(bson_value_preview(&Bson::Document(document.clone()), 150)),
                    );
                }
                matches = matches.child(group);
            }
            return panel.child(matches.overflow_y_scrollbar()).into_any_element();
        }

        panel
            .child(diff_heading(names, absent_documents(&detail), &appearance, cx))
            .children(self.diff.clone())
            .into_any_element()
    }
}

fn absent_documents(pair: &CompareDetail) -> [Option<&'static str>; 2] {
    pair.documents.each_ref().map(|documents| documents.is_empty().then_some("No document"))
}

/// The header and every virtual row share these columns, even when a side is absent.
pub(super) fn comparison_row() -> Div {
    div().w_full().min_w_0().flex().gap(spacing::sm()).px(spacing::sm())
}

pub(super) fn field_column() -> Div {
    div().w(relative(0.24)).max_w(px(240.0)).flex_shrink_0().min_w_0().overflow_hidden()
}

/// Disclosures and names share one leading edge per depth, like the document tree.
fn indent(depth: usize) -> Div {
    div().flex().items_center().w_full().min_w_0().pl(relative(indent_fraction(depth)))
}

fn indent_fraction(depth: usize) -> f32 {
    // ponytail: cap indentation at four levels so narrow panes keep the disclosure reachable;
    // the full path remains available in the field tooltip.
    depth.min(4) as f32 * 0.06
}

fn render_row(
    index: usize,
    row: &DetailRow,
    pair: &Arc<CompareDetail>,
    expansion: &Expansion,
    cx: &Context<DiffTable>,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let base = comparison_row()
        .debug_selector(move || format!("compare-detail-row-{index}"))
        .h(px(26.0))
        .items_center()
        .text_sm();
    match row {
        DetailRow::Unchanged { path, count } => {
            let expanded = expansion.unchanged.contains(path);
            let path = path.clone();
            base.child(
                field_column().flex().items_center().child(
                    indent(path.len()).child(
                        Button::new(("compare-unchanged", index))
                            .ghost()
                            .xsmall()
                            .max_w_full()
                            .min_w_0()
                            .text_color(muted)
                            .icon(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .label(format!(
                                "{count} unchanged field{}",
                                if *count == 1 { "" } else { "s" }
                            ))
                            .tooltip("Show or hide unchanged fields")
                            .on_click(cx.listener(move |table, _, _, cx| {
                                if !table.expansion.unchanged.remove(&path) {
                                    table.expansion.unchanged.insert(path.clone());
                                }
                                table.rebuild();
                                cx.notify();
                            })),
                    ),
                ),
            )
            .child(div().flex_1().min_w_0())
            .child(div().flex_1().min_w_0())
            .into_any_element()
        }
        DetailRow::Order { path } => {
            let order = |side: usize| {
                pair.documents[side]
                    .first()
                    .map(|document| {
                        if path.is_empty() {
                            document.keys().cloned().collect::<Vec<_>>().join(", ")
                        } else {
                            get_bson_at_path(document, path)
                                .and_then(Bson::as_document)
                                .map(|d| d.keys().cloned().collect::<Vec<_>>().join(", "))
                                .unwrap_or_default()
                        }
                    })
                    .unwrap_or_default()
            };
            base.child(
                field_column().flex().items_center().child(
                    indent(path.len()).child(
                        div()
                            .pl(px(18.0))
                            .truncate()
                            .text_color(muted)
                            .child(format!("{} · field order", label(path))),
                    ),
                ),
            )
            .children((0..2).map(|side| {
                div()
                    .debug_selector(move || format!("compare-value-{}", index * 2 + side))
                    .flex_1()
                    .min_w_0()
                    .h(px(24.0))
                    .px(spacing::xs())
                    .flex()
                    .items_center()
                    .rounded(borders::radius_xs())
                    .bg(crate::theme::colors::bg_changed(cx))
                    .text_color(muted)
                    .child(div().w_full().truncate().child(order(side)))
            }))
            .into_any_element()
        }
        DetailRow::Field { path, kind, informational, container, expanded } => {
            let name = match path.last() {
                Some(PathSegment::Key(key)) => key.clone(),
                Some(PathSegment::Index(i)) => format!("[{i}]"),
                None => "Document".into(),
            };
            let field = indent(path.len().saturating_sub(1));
            let field = if *container {
                let path = path.clone();
                field.child(
                    Button::new(("compare-branch", index))
                        .ghost()
                        .xsmall()
                        .max_w_full()
                        .min_w_0()
                        .icon(if *expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .label(name)
                        .tooltip(label(&path))
                        .on_click(cx.listener(move |table, _, _, cx| {
                            if !table.expansion.collapsed.remove(&path) {
                                table.expansion.collapsed.insert(path.clone());
                            }
                            table.rebuild();
                            cx.notify();
                        })),
                )
            } else {
                let full_path = label(path);
                field.child(
                    div()
                        .id(("compare-field-name", index))
                        .pl(px(18.0))
                        .min_w_0()
                        .max_w_full()
                        .truncate()
                        .text_color(if *informational { muted } else { cx.theme().foreground })
                        .child(name)
                        .tooltip(move |window, cx| {
                            Tooltip::new(full_path.clone()).build(window, cx)
                        }),
                )
            };
            base.child(
                field_column()
                    .debug_selector(move || format!("compare-field-{index}"))
                    .flex()
                    .items_center()
                    .child(field),
            )
            .children((0..2).map(|side| {
                value_cell(index * 2 + side, pair.clone(), path.clone(), *kind, *informational, cx)
            }))
            .into_any_element()
        }
    }
}

fn value_cell(
    id: usize,
    pair: Arc<CompareDetail>,
    path: Vec<PathSegment>,
    kind: Option<ChangeKind>,
    informational: bool,
    cx: &App,
) -> AnyElement {
    let side = id % 2;
    let muted = cx.theme().muted_foreground;
    let value = pair.documents[side].first().and_then(|document| get_bson_at_path(document, &path));
    let one_sided = pair.documents.iter().any(Vec::is_empty);
    let other =
        pair.documents[1 - side].first().and_then(|document| get_bson_at_path(document, &path));
    // A whole missing document is said once in the heading; tints mark differences inside a pair.
    let background = if informational || value.is_none() || one_sided {
        crate::theme::colors::transparent()
    } else if other.is_none() {
        side_color(side, cx).opacity(0.12)
    } else if kind.is_some() {
        crate::theme::colors::bg_changed(cx)
    } else {
        crate::theme::colors::transparent()
    };
    let color =
        if informational { muted } else { value.map(|v| value_color(v, cx)).unwrap_or(muted) };
    let mut cell = div()
        .id(("compare-value", id))
        .debug_selector(move || format!("compare-value-{id}"))
        .flex_1()
        .min_w_0()
        .h(px(24.0))
        .px(spacing::xs())
        .flex()
        .items_center()
        .gap(spacing::xs())
        .overflow_hidden()
        .rounded(borders::radius_xs())
        .bg(background)
        .text_color(color);
    if let Some(value) = value {
        if kind == Some(ChangeKind::NumberType) {
            cell = cell.child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("{:?}", value.element_type()).to_lowercase()),
            );
        }
        cell = cell.child(div().min_w_0().truncate().child(bson_value_preview(value, 120)));
    } else if !one_sided {
        // The other document has this field; say so instead of leaving an ambiguous blank.
        cell = cell.child(div().text_color(muted).child("—"));
    }
    cell.when(value.is_some(), |cell| {
        cell.tooltip(move |window, cx| {
            let value = pair.documents[side]
                .first()
                .and_then(|document| get_bson_at_path(document, &path))
                .unwrap();
            if crate::bson::has_value_details(value) {
                value_details_tooltip(value, window, cx)
            } else {
                Tooltip::new(bson_value_preview(value, 8_192)).build(window, cx)
            }
        })
    })
    .into_any_element()
}
