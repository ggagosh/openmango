use std::sync::Arc;

use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button as MenuButton, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::pagination::Pagination;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::bson::DocumentKey;
use crate::components::{Button, ErrorCallout};
use crate::helpers::format_number;
use crate::keyboard::RunAggregation;
use crate::state::app_state::PipelineState;
use crate::state::{AppCommands, AppState, DocumentViewMode, SessionDocument, SessionKey};
use crate::theme::spacing;
use crate::views::CollectionView;
use crate::views::documents::tree::lazy_row::{compute_row_meta, render_lazy_readonly_row};
use crate::views::documents::tree::lazy_tree::{build_visible_rows, collect_all_expandable_nodes};
use crate::views::documents::{aggregation_write_impact, request_run_aggregation};

use super::stage_editor::panel;

const PAGE_SIZES: &[i64] = &[10, 25, 50, 100, 500];

impl CollectionView {
    pub(in crate::views::documents) fn render_aggregation_results(
        &mut self,
        pipeline: &PipelineState,
        session_key: Option<SessionKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let appearance = self.state.read(cx).settings.appearance.clone();
        let muted = cx.theme().muted_foreground;
        let stale = pipeline.is_stale();
        // Describe what the results are, not what is selected now.
        let shown_target =
            pipeline.last_run.map(|run| run.target).unwrap_or_else(|| pipeline.preview_target());
        let results_len = pipeline.results.as_ref().map_or(0, |docs| docs.len());
        let total = match shown_target {
            Some(index) => pipeline.stage_doc_counts.get(index).and_then(|counts| counts.output),
            None => pipeline.stage_doc_counts.first().and_then(|counts| counts.input),
        };
        let has_results = results_len > 0 && pipeline.error.is_none();

        let title = match shown_target {
            _ if pipeline.text_mode => "Pipeline output".to_string(),
            Some(index) => match pipeline.stages.get(index).map(|stage| stage.operator.trim()) {
                Some(operator) if !operator.is_empty() => {
                    format!("Output of stage {} · {operator}", index + 1)
                }
                _ => format!("Output of stage {}", index + 1),
            },
            None => "Collection documents · input to stage 1".to_string(),
        };
        let meta = (pipeline.results.is_some() && pipeline.error.is_none()).then(|| {
            // Without stage counts only this page's size is known.
            let mut meta = match total {
                Some(1) => "1 document".to_string(),
                Some(count) => format!("{} documents", format_number(count)),
                None => format!("{} shown", format_number(results_len as u64)),
            };
            if let Some(ms) = pipeline.last_run_time_ms {
                meta.push_str(&format!(" · {ms} ms"));
            }
            meta
        });

        let io_toggle = (!pipeline.text_mode && pipeline.selected_stage.is_some()).then(|| {
            let preview_input = pipeline.preview_input;
            let toggle =
                |id: &'static str, label: &'static str, input: bool, tooltip: &'static str| {
                    let state = self.state.clone();
                    let session_key = session_key.clone();
                    Button::new(id)
                        .label(label)
                        .selected(preview_input == input)
                        .toggled(preview_input == input)
                        .tooltip(tooltip)
                        .disabled(session_key.is_none())
                        .on_click(move |_, _, cx| {
                            if let Some(session_key) = session_key.clone() {
                                state.update(cx, |state, cx| {
                                    state.set_pipeline_preview_input(&session_key, input);
                                    cx.notify();
                                });
                            }
                        })
                };
            ButtonGroup::new("agg-preview-side").xsmall().children([
                toggle("agg-preview-input", "Input", true, "Documents entering the selected stage"),
                toggle(
                    "agg-preview-output",
                    "Output",
                    false,
                    "Documents leaving the selected stage",
                ),
            ])
        });

        let view_mode = pipeline.results_view_mode;
        let view_toggle = {
            let toggle =
                |id: &'static str, icon: IconName, label: &'static str, mode: DocumentViewMode| {
                    let state = self.state.clone();
                    let session_key = session_key.clone();
                    let view = cx.entity();
                    Button::new(id)
                        .icon(Icon::new(icon))
                        .accessibility_label(label)
                        .tooltip(label)
                        .selected(view_mode == mode)
                        .toggled(view_mode == mode)
                        .on_click(move |_, _, cx| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            state.update(cx, |state, cx| {
                                state.set_aggregation_view_mode(&session_key, mode);
                                cx.notify();
                            });
                            view.update(cx, |view, cx| {
                                view.view_model.invalidate_agg_table();
                                cx.notify();
                            });
                        })
                };
            ButtonGroup::new("agg-view-mode").xsmall().children([
                toggle("agg-view-tree", IconName::Menu, "Tree view", DocumentViewMode::Tree),
                toggle(
                    "agg-view-table",
                    IconName::LayoutDashboard,
                    "Table view",
                    DocumentViewMode::Table,
                ),
            ])
        };

        let header = div()
            .flex()
            .flex_wrap()
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
                    .min_w(px(0.0))
                    .child(div().text_sm().truncate().child(title))
                    .when_some(meta, |row, meta| {
                        row.child(div().text_xs().text_color(muted).child(meta))
                    })
                    .when(stale && !pipeline.loading, |row| {
                        row.child(
                            div()
                                .id("agg-outdated")
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "The pipeline changed since this run. Run it to update.",
                                    )
                                    .action(&RunAggregation, Some("Documents Aggregation"))
                                    .build(window, cx)
                                })
                                .child(Tag::warning().xsmall().child("Outdated")),
                        )
                    })
                    .when(pipeline.loading, |row| row.child(Spinner::new().xsmall())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .children(io_toggle)
                    .child(view_toggle)
                    .child(Separator::vertical().h(px(16.0)))
                    .child(render_copy_menu(cx.entity(), has_results, view_mode))
                    .child(render_export_menu(
                        self.state.clone(),
                        session_key.clone(),
                        has_results,
                    )),
            );

        let body = self.render_aggregation_results_body(pipeline, session_key.clone(), window, cx);
        // Keep paging reachable on an empty page past the end.
        let show_footer = pipeline.error.is_none()
            && pipeline.results.is_some()
            && (results_len > 0 || pipeline.results_page > 0);
        let footer = show_footer.then(|| {
            render_results_footer(pipeline, total, results_len, self.state.clone(), session_key, cx)
        });

        panel(&appearance, cx).child(header).child(body).children(footer).into_any_element()
    }

    fn render_aggregation_results_body(
        &mut self,
        pipeline: &PipelineState,
        session_key: Option<SessionKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let stale = pipeline.is_stale();
        let database = session_key.as_ref().map(|key| key.database.clone()).unwrap_or_default();
        let write_target =
            aggregation_write_impact(&pipeline.stages, pipeline.preview_target(), &database);
        let centered = || {
            div()
                .flex()
                .flex_1()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(spacing::sm())
                .p(spacing::lg())
                .text_center()
        };

        if let Some(error) = pipeline.error.clone() {
            let retry = error.is_retryable().then(|| {
                let state = self.state.clone();
                let session_key = session_key.clone();
                Button::new("agg-error-retry").xsmall().label("Run again").on_click(
                    move |_, window, cx| {
                        if let Some(session_key) = session_key.clone() {
                            request_run_aggregation(state.clone(), session_key, false, window, cx);
                        }
                    },
                )
            });
            let go_to = pipeline
                .error_stage
                .filter(|index| !pipeline.text_mode && pipeline.selected_stage != Some(*index))
                .map(|index| {
                    let state = self.state.clone();
                    let session_key = session_key.clone();
                    Button::new("agg-error-go-to")
                        .xsmall()
                        .label(format!("Go to stage {}", index + 1))
                        .on_click(move |_, _, cx| {
                            if let Some(session_key) = session_key.clone() {
                                state.update(cx, |state, cx| {
                                    state.set_pipeline_selected_stage(&session_key, Some(index));
                                    cx.notify();
                                });
                            }
                        })
                });
            let mut callout =
                ErrorCallout::new("agg-results-error", error).state(self.state.clone());
            if let Some(go_to) = go_to {
                callout = callout.action(go_to);
            }
            if let Some(retry) = retry {
                callout = callout.action(retry);
            }
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .p(spacing::sm())
                .when(stale, |body| body.opacity(0.6))
                .child(callout)
                .into_any_element();
        }

        if pipeline.results.is_none() {
            if pipeline.loading {
                return centered()
                    .child(Spinner::new().small())
                    .child(div().text_sm().text_color(muted).child("Running the pipeline…"))
                    .into_any_element();
            }
            let message = match &write_target {
                Some((operator, namespace)) => {
                    format!("Running this pipeline writes to {namespace} with {operator}.")
                }
                None => "Run the pipeline to preview its output.".to_string(),
            };
            return centered()
                .child(div().text_sm().text_color(muted).child(message))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::xs())
                        .text_xs()
                        .text_color(muted)
                        .child("Run")
                        .children(Kbd::binding_for_action(
                            &RunAggregation,
                            Some("Documents Aggregation"),
                            window,
                        )),
                )
                .into_any_element();
        }

        let write_notice = write_target.map(|(operator, namespace)| {
            Alert::warning(
                "agg-write-notice",
                format!("Running this pipeline writes to {namespace} with {operator}."),
            )
            .banner()
        });

        let results = pipeline.results.clone().unwrap_or_default();
        let content = if results.is_empty() {
            centered()
                .child(div().text_sm().text_color(muted).child("No documents"))
                .child(div().text_xs().text_color(muted).child(
                    "The previewed stage returned nothing. Check the filters in earlier stages.",
                ))
                .into_any_element()
        } else if pipeline.results_view_mode == DocumentViewMode::Table {
            self.view_model.rebuild_agg_table(&self.state, window, cx);
            match self.view_model.agg_table_state().cloned() {
                Some(table) => {
                    gpui_kit::component::table::DataTable::new(&table).into_any_element()
                }
                None => div().into_any_element(),
            }
        } else {
            render_results_tree(self, &results, cx)
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .children(write_notice)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .when(stale, |content| content.opacity(0.6))
                    .child(content),
            )
            .into_any_element()
    }
}

fn render_results_footer(
    pipeline: &PipelineState,
    total: Option<u64>,
    results_len: usize,
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let per_page = pipeline.result_limit.max(1);
    // Edits reset the requested page, but the documents on screen are from the last run.
    let page = pipeline.last_run.map_or(pipeline.results_page, |run| run.page);
    let start = page * per_page as u64 + 1;
    let end = (start + results_len as u64).saturating_sub(1);
    let disabled = pipeline.loading || session_key.is_none();
    let range = match total {
        _ if results_len == 0 => "No documents on this page".to_string(),
        Some(total) => {
            format!("{}–{} of {}", format_number(start), format_number(end), format_number(total))
        }
        None => format!("{}–{}", format_number(start), format_number(end)),
    };

    let go_to_page = {
        let state = state.clone();
        let session_key = session_key.clone();
        move |page: u64, window: &mut Window, cx: &mut App| {
            let Some(session_key) = session_key.clone() else {
                return;
            };
            state.update(cx, |state, cx| {
                state.set_pipeline_page(&session_key, page);
                cx.notify();
            });
            request_run_aggregation(state.clone(), session_key, true, window, cx);
        }
    };

    let page_size = MenuButton::new("agg-page-size")
        .ghost()
        .xsmall()
        .label(format!("{per_page} / page"))
        .dropdown_caret(true)
        .disabled(disabled)
        .dropdown_menu_with_anchor(Anchor::BottomLeft, {
            let state = state.clone();
            let session_key = session_key.clone();
            move |mut menu: PopupMenu, _, _| {
                for &size in PAGE_SIZES {
                    let state = state.clone();
                    let session_key = session_key.clone();
                    menu = menu.item(
                        PopupMenuItem::new(size.to_string()).checked(size == per_page).on_click(
                            move |_, window, cx| {
                                let Some(session_key) = session_key.clone() else {
                                    return;
                                };
                                state.update(cx, |state, cx| {
                                    state.set_pipeline_result_limit(&session_key, size);
                                    cx.notify();
                                });
                                request_run_aggregation(
                                    state.clone(),
                                    session_key,
                                    true,
                                    window,
                                    cx,
                                );
                            },
                        ),
                    );
                }
                menu
            }
        });

    let navigation = match total {
        Some(total) => {
            let total_pages = total.div_ceil(per_page as u64).max(1);
            let go_to_page = go_to_page.clone();
            Pagination::new("agg-pages")
                .xsmall()
                .visible_pages(5)
                .when(total_pages > 100, |pagination| pagination.compact())
                .current_page(page as usize + 1)
                .total_pages(total_pages as usize)
                .disabled(disabled)
                .on_click(move |page, window, cx| {
                    go_to_page((*page as u64).saturating_sub(1), window, cx)
                })
                .into_any_element()
        }
        // Without counts the total is unknown, so only step one page at a time.
        None => {
            let previous = go_to_page.clone();
            let next = go_to_page;
            div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .child(
                    Button::new("agg-prev-page")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::ChevronLeft))
                        .accessibility_label("Previous page")
                        .disabled(disabled || page == 0)
                        .on_click(move |_, window, cx| {
                            previous(page.saturating_sub(1), window, cx)
                        }),
                )
                .child(
                    Button::new("agg-next-page")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::ChevronRight))
                        .accessibility_label("Next page")
                        .disabled(disabled || (results_len as i64) < per_page)
                        .on_click(move |_, window, cx| next(page + 1, window, cx)),
                )
                .into_any_element()
        }
    };

    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(spacing::sm())
        .px(spacing::sm())
        .py(px(4.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .child(div().text_xs().text_color(muted).child(range))
                .child(page_size),
        )
        .child(navigation)
        .into_any_element()
}

fn render_results_tree(
    view: &mut CollectionView,
    results: &Arc<Vec<mongodb::bson::Document>>,
    cx: &mut Context<CollectionView>,
) -> AnyElement {
    let view_entity = cx.entity();

    // Rebuild the SessionDocument list only when a run delivers new results.
    let signature = Arc::as_ptr(results) as usize;
    if view.aggregation_results_signature != Some(signature) {
        view.aggregation_results_signature = Some(signature);
        view.aggregation_results_expanded_nodes.clear();
        view.aggregation_results_documents = Some(Arc::new(
            results
                .iter()
                .enumerate()
                .map(|(idx, doc)| SessionDocument {
                    key: DocumentKey::from_document(doc, idx),
                    doc: doc.clone(),
                })
                .collect(),
        ));
    }
    let documents =
        view.aggregation_results_documents.clone().unwrap_or_else(|| Arc::new(Vec::new()));
    let visible_rows =
        Arc::new(build_visible_rows(&documents, &view.aggregation_results_expanded_nodes));
    let row_count = visible_rows.len();
    let muted = cx.theme().muted_foreground;
    let column = |label: &'static str| {
        div().flex_1().min_w(px(0.0)).text_xs().text_color(muted).child(label)
    };

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
                .child(column("Key"))
                .child(column("Value"))
                .child(
                    div()
                        .w(px(120.0))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_xs().text_color(muted).child("Type"))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .child(
                                    Button::new("agg-expand-all")
                                        .ghost()
                                        .xsmall()
                                        .icon(Icon::new(IconName::ChevronDown))
                                        .accessibility_label("Expand all")
                                        .tooltip("Expand all")
                                        .on_click({
                                            let view_entity = view_entity.clone();
                                            let documents = documents.clone();
                                            move |_, _, cx| {
                                                let nodes =
                                                    collect_all_expandable_nodes(&documents);
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
                                        .xsmall()
                                        .icon(Icon::new(IconName::ChevronUp))
                                        .accessibility_label("Collapse all")
                                        .tooltip("Collapse all")
                                        .on_click({
                                            let view_entity = view_entity.clone();
                                            move |_, _, cx| {
                                                view_entity.update(cx, |view, cx| {
                                                    view.aggregation_results_expanded_nodes.clear();
                                                    cx.notify();
                                                });
                                            }
                                        }),
                                ),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .overflow_hidden()
                .child(
                    uniform_list(
                        "agg-results-tree",
                        row_count,
                        cx.processor({
                            let view_entity = view_entity.clone();
                            move |view, range: std::ops::Range<usize>, _window, cx| {
                                let link_base = view
                                    .view_model
                                    .current_session()
                                    .map(|session| (view.state.clone(), session));
                                range
                                    .map(|ix| {
                                        let row = &visible_rows[ix];
                                        let meta = compute_row_meta(row, &documents, cx);
                                        render_lazy_readonly_row(
                                            row,
                                            &meta,
                                            false,
                                            view_entity.clone(),
                                            link_base.as_ref(),
                                            cx,
                                        )
                                    })
                                    .collect()
                            }
                        }),
                    )
                    .flex_1()
                    .track_scroll(&view.aggregation_results_scroll),
                )
                // The list scrolls itself; the bar reads the list's own handle.
                .vertical_scrollbar(&view.aggregation_results_scroll),
        )
        .into_any_element()
}

fn render_copy_menu(
    view: Entity<CollectionView>,
    has_results: bool,
    view_mode: DocumentViewMode,
) -> impl IntoElement {
    use crate::views::documents::actions::copy_aggregation_as;
    use crate::views::documents::export::CopyFormat;

    let formats = match view_mode {
        DocumentViewMode::Table => CopyFormat::table_formats().to_vec(),
        _ => CopyFormat::tree_formats().to_vec(),
    };
    MenuButton::new("agg-copy-as")
        .ghost()
        .xsmall()
        .icon(Icon::new(IconName::Copy))
        .label("Copy")
        .tooltip("Copy results")
        .disabled(!has_results)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu: PopupMenu, _, _| {
            for &format in &formats {
                let view = view.clone();
                menu = menu.item(PopupMenuItem::new(format.label()).icon(format.icon()).on_click(
                    move |_, _, cx| {
                        view.update(cx, |view, cx| copy_aggregation_as(view, format, cx))
                    },
                ));
            }
            menu
        })
}

fn render_export_menu(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    has_results: bool,
) -> impl IntoElement {
    use crate::views::documents::export::FileExportFormat;

    MenuButton::new("agg-export")
        .ghost()
        .xsmall()
        .icon(Icon::new(crate::assets::AppIcon::Download))
        .label("Export")
        .tooltip("Export results to a file")
        .disabled(!has_results || session_key.is_none())
        .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu: PopupMenu, _, _| {
            for &format in FileExportFormat::all() {
                let state = state.clone();
                let session_key = session_key.clone();
                menu = menu.item(
                    PopupMenuItem::new(format.label()).icon(Icon::new(IconName::File)).on_click(
                        move |_, _, cx| {
                            if let Some(session_key) = session_key.clone() {
                                AppCommands::save_aggregation_as(
                                    state.clone(),
                                    session_key,
                                    format,
                                    cx,
                                );
                            }
                        },
                    ),
                );
            }
            menu
        })
}
