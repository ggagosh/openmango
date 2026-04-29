use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Disableable as _;
use gpui_component::Sizable as _;
use gpui_component::input::Input;
use gpui_component::spinner::Spinner;
use gpui_component::switch::Switch;
use gpui_component::{Icon, IconName};

use crate::bson::DocumentKey;
use crate::components::Button;
use crate::helpers::format_number;
use crate::state::app_state::{PipelineState, StageStatsMode};
use crate::state::{AppCommands, DocumentViewMode, SessionDocument, SessionKey};
use crate::theme::{islands, spacing};
use crate::views::documents::header::clean_toolbar_icon_button;
use crate::views::documents::tree::lazy_row::{compute_row_meta, render_lazy_readonly_row};
use crate::views::documents::tree::lazy_tree::{build_visible_rows, collect_all_expandable_nodes};

use crate::views::CollectionView;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

impl CollectionView {
    pub(in crate::views::documents) fn render_aggregation_results(
        &mut self,
        pipeline: &PipelineState,
        session_key: Option<SessionKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let appearance = self.state.read(cx).settings.appearance.clone();
        let panel_bg = islands::card_bg(&appearance, cx);
        let panel_border = islands::panel_border(&appearance, cx).opacity(0.5);
        let panel_radius = islands::radius_sm(&appearance);
        let section_bg = cx.theme().transparent;
        let results_count = pipeline.results.as_ref().map(|docs| docs.len()).unwrap_or(0);
        let target_index = pipeline.selected_stage.or_else(|| pipeline.stages.len().checked_sub(1));
        let total_count = target_index
            .and_then(|idx| pipeline.stage_doc_counts.get(idx))
            .and_then(|counts| counts.output);
        let per_page = if pipeline.result_limit > 0 { pipeline.result_limit as u64 } else { 50 };
        let current_page = pipeline.results_page;
        let total_pages =
            total_count.map(|total| if total == 0 { 1 } else { ((total - 1) / per_page) + 1 });
        let (shown_start, shown_end) = if results_count == 0 {
            (0, 0)
        } else {
            let start = current_page.saturating_mul(per_page) + 1;
            let end = start + results_count as u64 - 1;
            (start, end)
        };
        let count_label =
            total_count.map(format_number).unwrap_or_else(|| format_number(results_count as u64));
        let mut meta_label = format!("{count_label} document(s)");
        if let Some(ms) = pipeline.last_run_time_ms {
            meta_label.push_str(&format!(" · {ms}ms"));
        }
        let stage_label = pipeline
            .selected_stage
            .and_then(|idx| pipeline.stages.get(idx).map(|stage| (idx, stage)))
            .map(|(idx, stage)| format!("Results (after Stage {}: {})", idx + 1, stage.operator))
            .unwrap_or_else(|| "Results (full pipeline)".to_string());
        let stage_stats_mode = pipeline.stage_stats_mode;
        let stage_stats_enabled = stage_stats_mode.counts_enabled();
        let stage_timings_enabled = stage_stats_mode.timings_enabled();
        let can_toggle_stage_stats = session_key.is_some() && !pipeline.stages.is_empty();
        let can_toggle_timing = can_toggle_stage_stats && stage_stats_enabled;

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(spacing::sm())
            .py(spacing::xs())
            .bg(section_bg)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(cx.theme().foreground).child(stage_label))
                    .child(
                        div().text_xs().text_color(cx.theme().muted_foreground).child(meta_label),
                    )
                    .child(if pipeline.loading {
                        Spinner::new().xsmall().into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                Switch::new("agg-stage-stats-enabled")
                                    .checked(stage_stats_enabled)
                                    .small()
                                    .tooltip("Per-stage counts/timing (slower)")
                                    .disabled(!can_toggle_stage_stats)
                                    .on_click({
                                        let state = self.state.clone();
                                        let session_key = session_key.clone();
                                        move |checked, _window, cx| {
                                            let checked = *checked;
                                            let Some(session_key) = session_key.clone() else {
                                                return;
                                            };
                                            let next_mode = if checked {
                                                StageStatsMode::CountsAndTiming
                                            } else {
                                                StageStatsMode::Off
                                            };
                                            if next_mode == stage_stats_mode {
                                                return;
                                            }
                                            state.update(cx, |state, cx| {
                                                state.set_pipeline_stage_stats_mode(
                                                    &session_key,
                                                    next_mode,
                                                );
                                                cx.notify();
                                            });
                                            let should_run = state
                                                .read(cx)
                                                .session(&session_key)
                                                .is_some_and(|session| {
                                                    !session.data.aggregation.stages.is_empty()
                                                });
                                            if should_run {
                                                AppCommands::run_aggregation(
                                                    state.clone(),
                                                    session_key,
                                                    false,
                                                    cx,
                                                );
                                            }
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Stats"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                Switch::new("agg-stage-stats-timing")
                                    .checked(stage_timings_enabled)
                                    .small()
                                    .tooltip("Per-stage timing (slower)")
                                    .disabled(!can_toggle_timing)
                                    .on_click({
                                        let state = self.state.clone();
                                        let session_key = session_key.clone();
                                        move |checked, _window, cx| {
                                            let checked = *checked;
                                            let Some(session_key) = session_key.clone() else {
                                                return;
                                            };
                                            if !stage_stats_enabled {
                                                return;
                                            }
                                            let next_mode = if checked {
                                                StageStatsMode::CountsAndTiming
                                            } else {
                                                StageStatsMode::Counts
                                            };
                                            if next_mode == stage_stats_mode {
                                                return;
                                            }
                                            state.update(cx, |state, cx| {
                                                state.set_pipeline_stage_stats_mode(
                                                    &session_key,
                                                    next_mode,
                                                );
                                                cx.notify();
                                            });
                                            let should_run = state
                                                .read(cx)
                                                .session(&session_key)
                                                .is_some_and(|session| {
                                                    !session.data.aggregation.stages.is_empty()
                                                });
                                            if should_run {
                                                AppCommands::run_aggregation(
                                                    state.clone(),
                                                    session_key,
                                                    false,
                                                    cx,
                                                );
                                            }
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Timing"),
                            ),
                    )
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Limit"))
                    .child(if let Some(limit_state) = self.aggregation_limit_state.clone() {
                        Input::new(&limit_state)
                            .w(px(72.0))
                            .disabled(session_key.is_none())
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    })
                    .child(agg_separator(cx))
                    .child(self.render_agg_view_toggle(
                        &pipeline.results_view_mode,
                        session_key.clone(),
                        cx,
                    ))
                    .child(agg_separator(cx))
                    .child(render_agg_copy_as(
                        cx.entity().clone(),
                        results_count > 0,
                        &pipeline.results_view_mode,
                        cx,
                    ))
                    .child(render_agg_export(
                        self.state.clone(),
                        session_key.clone(),
                        results_count > 0,
                        cx,
                    )),
            );

        let mut body =
            div().flex().flex_col().flex_1().min_w(px(0.0)).min_h(px(0.0)).overflow_hidden();
        if let Some(error) = pipeline.error.clone() {
            body = body.child(
                div()
                    .px(spacing::sm())
                    .py(spacing::xs())
                    .text_sm()
                    .text_color(cx.theme().danger_foreground)
                    .child(error),
            );
        }
        if pipeline.results_view_mode == DocumentViewMode::Table {
            self.view_model.rebuild_agg_table(&self.state, window, cx);
            if let Some(agg_table) = self.view_model.agg_table_state().cloned() {
                body = body.child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .overflow_hidden()
                        .child(gpui_component::table::Table::new(&agg_table)),
                );
            }
        } else {
            body = body.child(render_results_tree(self, pipeline, session_key.clone(), window, cx));
        }
        let footer_data = ResultsFooterData {
            total_count,
            per_page,
            current_page,
            total_pages,
            shown_start,
            shown_end,
        };
        body = body.child(render_results_footer(self.state.clone(), session_key, footer_data, cx));

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(panel_bg)
            .border_1()
            .border_color(panel_border)
            .rounded(panel_radius)
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_agg_view_toggle(
        &self,
        mode: &DocumentViewMode,
        session_key: Option<SessionKey>,
        cx: &mut Context<Self>,
    ) -> Div {
        let is_tree = *mode == DocumentViewMode::Tree;
        let is_table = *mode == DocumentViewMode::Table;
        let active_bg = cx.theme().secondary.opacity(0.55);

        let tree_btn = {
            let mut btn = clean_toolbar_icon_button(
                Button::new("agg-view-tree").compact().on_click({
                    let state = self.state.clone();
                    let session_key = session_key.clone();
                    let view = cx.entity().clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(sk) = session_key.clone() else {
                            return;
                        };
                        state.update(cx, |state, cx| {
                            state.set_aggregation_view_mode(&sk, DocumentViewMode::Tree);
                            cx.notify();
                        });
                        view.update(cx, |this, cx| {
                            this.view_model.invalidate_agg_table();
                            cx.notify();
                        });
                    }
                }),
                IconName::Menu,
                "Tree view",
            );
            if is_tree {
                btn = btn.active_style(active_bg);
            }
            btn
        };

        let table_btn = {
            let mut btn = clean_toolbar_icon_button(
                Button::new("agg-view-table").compact().on_click({
                    let state = self.state.clone();
                    let session_key = session_key.clone();
                    let view = cx.entity().clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(sk) = session_key.clone() else {
                            return;
                        };
                        state.update(cx, |state, cx| {
                            state.set_aggregation_view_mode(&sk, DocumentViewMode::Table);
                            cx.notify();
                        });
                        view.update(cx, |_this, cx| {
                            cx.notify();
                        });
                    }
                }),
                IconName::LayoutDashboard,
                "Table view",
            );
            if is_table {
                btn = btn.active_style(active_bg);
            }
            btn
        };

        div().flex().items_center().gap_0p5().child(tree_btn).child(table_btn)
    }
}

struct ResultsFooterData {
    total_count: Option<u64>,
    per_page: u64,
    current_page: u64,
    total_pages: Option<u64>,
    shown_start: u64,
    shown_end: u64,
}

fn render_results_footer(
    state: Entity<crate::state::AppState>,
    session_key: Option<SessionKey>,
    data: ResultsFooterData,
    cx: &App,
) -> AnyElement {
    let ResultsFooterData {
        total_count,
        per_page,
        current_page,
        total_pages,
        shown_start,
        shown_end,
    } = data;
    let has_session = session_key.is_some();
    let has_total = total_count.is_some();
    let total_pages_value = total_pages.unwrap_or(0);
    let prev_disabled = !has_session || current_page == 0 || !has_total;
    let next_disabled = !has_session || !has_total || current_page + 1 >= total_pages_value;

    let range_label = if let Some(total) = total_count {
        if shown_start == 0 {
            format!("Showing 0 of {}", format_number(total))
        } else {
            format!(
                "Showing {}-{} of {}",
                format_number(shown_start),
                format_number(shown_end.min(total)),
                format_number(total),
            )
        }
    } else {
        format!("Showing {} per page", format_number(per_page))
    };
    let page_label = if let Some(total_pages) = total_pages {
        format!("Page {} of {}", current_page + 1, total_pages.max(1))
    } else {
        "Page —".to_string()
    };

    div()
        .flex()
        .items_center()
        .justify_between()
        .px(spacing::sm())
        .py(spacing::xs())
        .bg(cx.theme().transparent)
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(range_label))
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(page_label))
                .child(
                    Button::new("agg-prev-page")
                        .compact()
                        .label("Prev")
                        .disabled(prev_disabled)
                        .on_click({
                            let state = state.clone();
                            let session_key = session_key.clone();
                            move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                let Some(session_key) = session_key.clone() else {
                                    return;
                                };
                                let moved = state.update(cx, |state, cx| {
                                    let changed = state.prev_pipeline_page(&session_key);
                                    if changed {
                                        cx.notify();
                                    }
                                    changed
                                });
                                if moved {
                                    AppCommands::run_aggregation(
                                        state.clone(),
                                        session_key,
                                        false,
                                        cx,
                                    );
                                }
                            }
                        }),
                )
                .child(
                    Button::new("agg-next-page")
                        .compact()
                        .label("Next")
                        .disabled(next_disabled)
                        .on_click({
                            let state = state.clone();
                            let session_key = session_key.clone();
                            move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                let Some(session_key) = session_key.clone() else {
                                    return;
                                };
                                let total_pages = total_pages_value.max(1);
                                let moved = state.update(cx, |state, cx| {
                                    let changed =
                                        state.next_pipeline_page(&session_key, total_pages);
                                    if changed {
                                        cx.notify();
                                    }
                                    changed
                                });
                                if moved {
                                    AppCommands::run_aggregation(
                                        state.clone(),
                                        session_key,
                                        false,
                                        cx,
                                    );
                                }
                            }
                        }),
                ),
        )
        .into_any_element()
}

fn render_results_tree(
    view: &mut CollectionView,
    pipeline: &PipelineState,
    _session_key: Option<SessionKey>,
    _window: &mut Window,
    cx: &mut Context<CollectionView>,
) -> AnyElement {
    let view_entity = cx.entity();

    if pipeline.loading {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(spacing::sm())
            .child(Spinner::new().small())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Running pipeline..."),
            )
            .into_any_element();
    }

    let Some(results) = pipeline.results.as_ref() else {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Run the pipeline to see results"),
            )
            .into_any_element();
    };

    if results.is_empty() {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No documents returned"),
            )
            .into_any_element();
    }

    // Build documents from results
    let documents: Arc<Vec<SessionDocument>> = Arc::new(
        results
            .iter()
            .enumerate()
            .map(|(idx, doc)| SessionDocument {
                key: DocumentKey::from_document(doc, idx),
                doc: doc.clone(),
            })
            .collect(),
    );

    // Check if results changed - if so, clear expanded state
    let signature = results_signature(&documents);
    if view.aggregation_results_signature != Some(signature) {
        view.aggregation_results_signature = Some(signature);
        view.aggregation_results_expanded_nodes.clear();
    }

    // Build visible rows lazily based on expanded state
    let expanded_nodes = &view.aggregation_results_expanded_nodes;
    let visible_rows = Arc::new(build_visible_rows(&documents, expanded_nodes));
    let row_count = visible_rows.len();
    let scroll_handle = view.aggregation_results_scroll.clone();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .child(
            div()
                .flex()
                .items_center()
                .px(spacing::lg())
                .py(spacing::xs())
                .bg(cx.theme().transparent)
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Key"),
                )
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Value"),
                )
                .child({
                    let view_entity = view_entity.clone();
                    let documents_for_expand = documents.clone();
                    div()
                        .w(px(120.0))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div().text_xs().text_color(cx.theme().muted_foreground).child("Type"),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .child(
                                    Button::new("agg-expand-all")
                                        .ghost()
                                        .compact()
                                        .icon(Icon::new(IconName::ChevronDown).xsmall())
                                        .tooltip("Expand all")
                                        .on_click({
                                            let view_entity = view_entity.clone();
                                            let documents = documents_for_expand.clone();
                                            move |_: &ClickEvent,
                                                  _window: &mut Window,
                                                  cx: &mut App| {
                                                let nodes =
                                                    collect_all_expandable_nodes(
                                                        &documents,
                                                    );
                                                view_entity.update(cx, |view, cx| {
                                                    view.aggregation_results_expanded_nodes = nodes;
                                                    cx.notify();
                                                });
                                            }
                                        }),
                                )
                                .child(
                                    Button::new("agg-collapse-all")
                                        .ghost()
                                        .compact()
                                        .icon(Icon::new(IconName::ChevronUp).xsmall())
                                        .tooltip("Collapse all")
                                        .on_click({
                                            let view_entity = view_entity.clone();
                                            move |_: &ClickEvent,
                                                  _window: &mut Window,
                                                  cx: &mut App| {
                                                view_entity.update(cx, |view, cx| {
                                                    view.aggregation_results_expanded_nodes.clear();
                                                    cx.notify();
                                                });
                                            }
                                        }),
                                ),
                        )
                }),
        )
        .child(
            div().flex().flex_col().flex_1().min_w(px(0.0)).min_h(px(0.0)).overflow_hidden().child(
                uniform_list(
                    "agg-results-tree",
                    row_count,
                    cx.processor({
                        let documents = documents.clone();
                        let visible_rows = visible_rows.clone();
                        let view_entity = view_entity.clone();
                        move |_view, range: std::ops::Range<usize>, _window, cx| {
                            range
                                .map(|ix| {
                                    let row = &visible_rows[ix];
                                    let meta = compute_row_meta(row, &documents, cx);
                                    render_lazy_readonly_row(
                                        ix,
                                        row,
                                        &meta,
                                        false,
                                        view_entity.clone(),
                                        cx,
                                    )
                                })
                                .collect()
                        }
                    }),
                )
                .flex_1()
                .track_scroll(scroll_handle),
            ),
        )
        .into_any_element()
}

fn agg_separator(cx: &App) -> Div {
    div().w(px(1.0)).h(px(16.0)).bg(cx.theme().border.opacity(0.5))
}

fn render_agg_copy_as(
    view: Entity<CollectionView>,
    has_results: bool,
    view_mode: &DocumentViewMode,
    cx: &App,
) -> impl IntoElement {
    use gpui_component::Sizable as _;
    use gpui_component::button::{Button as MenuButton, ButtonCustomVariant, ButtonVariants as _};
    use gpui_component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};

    use crate::theme::borders;
    use crate::views::documents::actions::copy_aggregation_as;
    use crate::views::documents::export::CopyFormat;

    let clean_variant = ButtonCustomVariant::new(cx)
        .color(cx.theme().transparent)
        .foreground(cx.theme().muted_foreground)
        .border(cx.theme().transparent)
        .hover(cx.theme().secondary.opacity(0.5))
        .active(cx.theme().secondary.opacity(0.62))
        .shadow(false);

    let formats = match view_mode {
        DocumentViewMode::Table => CopyFormat::table_formats().to_vec(),
        _ => CopyFormat::tree_formats().to_vec(),
    };

    MenuButton::new("agg-copy-as")
        .compact()
        .rounded(borders::radius_sm())
        .with_size(gpui_component::Size::Small)
        .custom(clean_variant)
        .label("Copy Results As")
        .icon(Icon::new(IconName::Copy).xsmall())
        .tooltip("Copy results to clipboard")
        .disabled(!has_results)
        .dropdown_menu_with_anchor(Corner::TopRight, move |menu: PopupMenu, _window, _cx| {
            let mut menu = menu;
            for &fmt in &formats {
                let view_click = view.clone();
                let item = PopupMenuItem::new(fmt.label()).icon(fmt.icon()).on_click(
                    move |_, _window, cx| {
                        view_click.update(cx, |this, cx| {
                            copy_aggregation_as(this, fmt, cx);
                        });
                    },
                );
                menu = menu.item(item);
            }
            menu
        })
}

fn render_agg_export(
    state: Entity<crate::state::AppState>,
    session_key: Option<SessionKey>,
    has_results: bool,
    cx: &App,
) -> impl IntoElement {
    use gpui_component::Sizable as _;
    use gpui_component::button::{Button as MenuButton, ButtonCustomVariant, ButtonVariants as _};
    use gpui_component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};

    use crate::state::AppCommands;
    use crate::theme::borders;
    use crate::views::documents::export::FileExportFormat;

    let clean_variant = ButtonCustomVariant::new(cx)
        .color(cx.theme().transparent)
        .foreground(cx.theme().muted_foreground)
        .border(cx.theme().transparent)
        .hover(cx.theme().secondary.opacity(0.5))
        .active(cx.theme().secondary.opacity(0.62))
        .shadow(false);

    MenuButton::new("agg-export")
        .compact()
        .rounded(borders::radius_sm())
        .with_size(gpui_component::Size::Small)
        .custom(clean_variant)
        .label("Export Results")
        .icon(Icon::new(IconName::Download).xsmall())
        .tooltip("Export results to file")
        .disabled(!has_results || session_key.is_none())
        .dropdown_menu_with_anchor(Corner::TopRight, move |menu: PopupMenu, _window, _cx| {
            let mut menu = menu;
            for &fmt in FileExportFormat::all() {
                let state_click = state.clone();
                let sk = session_key.clone();
                let item = PopupMenuItem::new(fmt.label())
                    .icon(Icon::new(IconName::File))
                    .on_click(move |_, _window, cx| {
                        if let Some(sk) = sk.clone() {
                            AppCommands::save_aggregation_as(state_click.clone(), sk, fmt, cx);
                        }
                    });
                menu = menu.item(item);
            }
            menu
        })
}

fn results_signature(documents: &[SessionDocument]) -> u64 {
    let mut hasher = DefaultHasher::new();
    documents.len().hash(&mut hasher);
    for doc in documents {
        doc.key.hash(&mut hasher);
    }
    hasher.finish()
}
