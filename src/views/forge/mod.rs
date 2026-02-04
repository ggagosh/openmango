//! Forge - MongoDB Query Shell
//!
//! A database-scoped query shell with a Forge editor for syntax highlighting,
//! autocomplete, and IDE-like experience.

mod completion;
mod editor;
mod logic;
mod mongosh;
mod output;
mod runtime;
mod types;

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::*;
use gpui_component::input::{Input, InputState};
use gpui_component::resizable::{resizable_panel, v_resizable};
use gpui_component::scroll::ScrollableElement;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{Icon, IconName, Sizable};

use crate::components::Button;
use crate::keyboard::{
    CancelForgeRun, ClearForgeOutput, FindInForgeOutput, FocusForgeEditor, FocusForgeOutput,
    RunForgeAll, RunForgeSelectionOrStatement,
};
use crate::state::SessionDocument;
use crate::state::{AppEvent, AppState, View};
use crate::theme::{borders, colors, fonts, spacing};
use crate::views::documents::tree::lazy_row::compute_row_meta;
use crate::views::documents::tree::lazy_tree::{VisibleRow, build_visible_rows};
use completion::ForgeCompletionProvider;
use output::filter_visible_rows;
use runtime::ForgeRuntime;
use types::{ForgeOutputTab, ForgeRunOutput, ResultPage, format_result_tab_label};

// ============================================================================
// ForgeView
// ============================================================================

pub struct ForgeView {
    state: Entity<AppState>,
    editor_state: Option<Entity<InputState>>,
    editor_subscription: Option<Subscription>,
    completion_provider: Option<Rc<ForgeCompletionProvider>>,
    raw_output_state: Option<Entity<InputState>>,
    raw_output_subscription: Option<Subscription>,
    raw_output_text: String,
    raw_output_programmatic: bool,
    results_search_state: Option<Entity<InputState>>,
    results_search_subscription: Option<Subscription>,
    results_search_query: String,
    current_text: String,
    focus_handle: FocusHandle,
    editor_focus_requested: bool,
    runtime: Arc<ForgeRuntime>,
    mongosh_error: Option<String>,
    run_seq: u64,
    is_running: bool,
    output_runs: Vec<ForgeRunOutput>,
    output_tab: ForgeOutputTab,
    active_run_id: Option<u64>,
    output_events_started: bool,
    last_result: Option<String>,
    last_error: Option<String>,
    result_documents: Option<Arc<Vec<SessionDocument>>>,
    result_pages: Vec<ResultPage>,
    result_page_index: usize,
    result_signature: Option<u64>,
    result_expanded_nodes: HashSet<String>,
    result_scroll: UniformListScrollHandle,
    active_tab_id: Option<uuid::Uuid>,
    output_visible: bool,
    _subscriptions: Vec<Subscription>,
}

impl ForgeView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let runtime = Arc::new(ForgeRuntime::new());

        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&state, |this, state, event, cx| {
                if matches!(event, AppEvent::ViewChanged) {
                    let visible = matches!(state.read(cx).current_view, View::Forge);
                    this.editor_focus_requested = visible;
                    cx.notify();
                }
            }),
        ];

        Self {
            state,
            editor_state: None,
            editor_subscription: None,
            completion_provider: None,
            raw_output_state: None,
            raw_output_subscription: None,
            raw_output_text: String::new(),
            raw_output_programmatic: false,
            results_search_state: None,
            results_search_subscription: None,
            results_search_query: String::new(),
            current_text: String::new(),
            focus_handle,
            editor_focus_requested: false,
            runtime,
            mongosh_error: None,
            run_seq: 0,
            is_running: false,
            output_runs: Vec::new(),
            output_tab: ForgeOutputTab::Raw,
            active_run_id: None,
            output_events_started: false,
            last_result: None,
            last_error: None,
            result_documents: None,
            result_pages: Vec::new(),
            result_page_index: 0,
            result_signature: None,
            result_expanded_nodes: HashSet::new(),
            result_scroll: UniformListScrollHandle::new(),
            active_tab_id: None,
            output_visible: true,
            _subscriptions: subscriptions,
        }
    }

    fn render_header(&self, cx: &App) -> impl IntoElement {
        let (database, _connection_name) = {
            let state_ref = self.state.read(cx);
            let db = state_ref
                .active_forge_tab_key()
                .map(|k| k.database.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            let conn_name = state_ref
                .active_forge_tab_key()
                .and_then(|k| state_ref.active_connection_by_id(k.connection_id))
                .map(|c| c.config.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            (db, conn_name)
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .px(spacing::md())
            .py(spacing::sm())
            .border_b_1()
            .border_color(colors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(colors::text_primary())
                            .child("Forge"),
                    )
                    .child(
                        div()
                            .px(spacing::sm())
                            .py(px(2.0))
                            .rounded(borders::radius_sm())
                            .bg(colors::bg_sidebar())
                            .text_xs()
                            .text_color(colors::text_secondary())
                            .child(database),
                    ),
            )
    }

    fn render_output(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let forge_view = cx.entity();

        let clear_button =
            Button::new("forge-output-clear").compact().ghost().label("Clear").on_click({
                let forge_view = forge_view.clone();
                move |_, _window, cx| {
                    forge_view.update(cx, |this, _cx| {
                        this.clear_output_runs();
                        if let Some(raw_state) = &this.raw_output_state {
                            this.raw_output_programmatic = true;
                            raw_state.update(_cx, |state, cx| {
                                state.set_value(String::new(), _window, cx);
                            });
                            this.raw_output_programmatic = false;
                        }
                        _cx.notify();
                    });
                }
            });

        let selected_index = match self.output_tab {
            ForgeOutputTab::Raw => 0,
            ForgeOutputTab::Results => {
                if self.result_pages.is_empty() {
                    1
                } else {
                    self.result_page_index.min(self.result_pages.len().saturating_sub(1)) + 1
                }
            }
        };

        let has_inline_result =
            self.last_result.is_some() || self.last_error.is_some() || self.mongosh_error.is_some();

        let tab_bar = TabBar::new("forge-output-tabs")
            .underline()
            .small()
            .selected_index(selected_index)
            .on_click({
                let forge_view = forge_view.clone();
                move |index, _window, cx| {
                    let index = *index;
                    forge_view.update(cx, |this, _cx| {
                        if index == 0 {
                            this.output_tab = ForgeOutputTab::Raw;
                        } else {
                            this.output_tab = ForgeOutputTab::Results;
                            if !this.result_pages.is_empty() {
                                this.select_result_page(index - 1);
                            }
                        }
                    });
                }
            })
            .children(
                std::iter::once(Tab::new().label("Raw output"))
                    .chain(if self.result_pages.is_empty() && has_inline_result {
                        vec![Tab::new().label("Shell Output")].into_iter()
                    } else {
                        Vec::new().into_iter()
                    })
                    .chain(self.result_pages.iter().enumerate().map(|(index, page)| {
                        let label = format_result_tab_label(&page.label, index);
                        let view_entity = forge_view.clone();
                        let pin_icon = if page.pinned {
                            Icon::new(IconName::Star).xsmall().text_color(colors::accent())
                        } else {
                            Icon::new(IconName::StarOff).xsmall().text_color(colors::text_muted())
                        };
                        let pin_button = div()
                            .id(("forge-result-pin", index))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded(borders::radius_sm())
                            .cursor_pointer()
                            .hover(|s| s.bg(colors::bg_hover()))
                            .child(pin_icon)
                            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                cx.stop_propagation();
                                view_entity.update(cx, |this, _cx| {
                                    this.toggle_result_pinned(index);
                                });
                            });

                        let view_entity = forge_view.clone();
                        let close_button = div()
                            .id(("forge-result-close", index))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded(borders::radius_sm())
                            .cursor_pointer()
                            .hover(|s| s.bg(colors::bg_hover()))
                            .child(
                                Icon::new(IconName::Close)
                                    .xsmall()
                                    .text_color(colors::text_muted()),
                            )
                            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                cx.stop_propagation();
                                view_entity.update(cx, |this, _cx| {
                                    this.close_result_page(index);
                                });
                            });

                        Tab::new().label(label).prefix(pin_button).suffix(close_button)
                    })),
            );

        let body: AnyElement = match self.output_tab {
            ForgeOutputTab::Results => self.render_results_body(window, cx).into_any_element(),
            ForgeOutputTab::Raw => self.render_raw_output_body(window, cx).into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .size_full()
            .px(spacing::md())
            .py(spacing::sm())
            .border_t_1()
            .border_color(colors::border())
            .bg(colors::bg_sidebar())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(div().text_xs().text_color(colors::text_muted()).child("Output"))
                            .child(tab_bar),
                    )
                    .child(clear_button),
            )
            .child(body)
    }

    fn bind_root_actions(
        &mut self,
        root: Div,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        root.on_action(cx.listener(|this, _: &RunForgeAll, _window, cx| {
            if let Some(editor_state) = &this.editor_state {
                let text = editor_state.read(cx).value().to_string();
                this.handle_execute_query(&text, cx);
                cx.stop_propagation();
            }
        }))
        .on_action(cx.listener(|this, _: &RunForgeSelectionOrStatement, window, cx| {
            this.handle_execute_selection_or_statement(window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &CancelForgeRun, _window, cx| {
            this.cancel_running(cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &ClearForgeOutput, _window, cx| {
            this.clear_output_runs();
            if let Some(raw_state) = &this.raw_output_state {
                this.raw_output_programmatic = true;
                raw_state.update(cx, |state, cx| {
                    state.set_value(String::new(), _window, cx);
                });
                this.raw_output_programmatic = false;
            }
            cx.notify();
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &FocusForgeEditor, window, cx| {
            if let Some(editor_state) = &this.editor_state {
                editor_state.update(cx, |state, cx| {
                    state.focus(window, cx);
                });
            }
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &FocusForgeOutput, window, cx| {
            match this.output_tab {
                ForgeOutputTab::Raw => {
                    let state = this.ensure_raw_output_state(window, cx);
                    state.update(cx, |state, cx| {
                        state.focus(window, cx);
                    });
                }
                ForgeOutputTab::Results => {
                    let state = this.ensure_results_search_state(window, cx);
                    state.update(cx, |state, cx| {
                        state.focus(window, cx);
                    });
                }
            }
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &FindInForgeOutput, window, cx| {
            match this.output_tab {
                ForgeOutputTab::Raw => {
                    let state = this.ensure_raw_output_state(window, cx);
                    state.update(cx, |state, cx| {
                        state.focus(window, cx);
                    });
                    cx.dispatch_action(&gpui_component::input::Search);
                }
                ForgeOutputTab::Results => {
                    let state = this.ensure_results_search_state(window, cx);
                    state.update(cx, |state, cx| {
                        state.focus(window, cx);
                    });
                }
            }
            cx.stop_propagation();
        }))
    }

    fn render_results_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut body =
            div().flex().flex_col().flex_1().min_w(px(0.0)).min_h(px(0.0)).overflow_hidden();

        if let Some(err) = &self.mongosh_error {
            body = body.child(
                div()
                    .px(spacing::sm())
                    .py(spacing::xs())
                    .text_sm()
                    .text_color(colors::text_error())
                    .child(format!("Forge runtime error: {err}")),
            );
        } else if let Some(err) = &self.last_error {
            body = body.child(
                div()
                    .px(spacing::sm())
                    .py(spacing::xs())
                    .text_sm()
                    .text_color(colors::text_error())
                    .child(err.clone()),
            );
        }

        let search_state = self.ensure_results_search_state(window, cx);
        let current_search = search_state.read(cx).value().to_string();
        if current_search != self.results_search_query {
            search_state.update(cx, |state, cx| {
                state.set_value(self.results_search_query.clone(), window, cx);
            });
        }
        let search_input = Input::new(&search_state)
            .appearance(true)
            .bordered(true)
            .focus_bordered(true)
            .w(px(220.0));
        body = body.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .px(spacing::sm())
                .py(spacing::xs())
                .child(search_input),
        );

        if let Some(documents) = self.result_documents.clone() {
            let expanded_nodes = &self.result_expanded_nodes;
            let mut visible_rows = build_visible_rows(&documents, expanded_nodes);
            if !self.results_search_query.trim().is_empty() {
                visible_rows =
                    filter_visible_rows(&documents, visible_rows, &self.results_search_query);
            }
            let visible_rows = Arc::new(visible_rows);
            let row_count = visible_rows.len();
            let scroll_handle = self.result_scroll.clone();
            let view_entity = cx.entity();

            let header = div()
                .flex()
                .items_center()
                .px(spacing::lg())
                .py(spacing::xs())
                .bg(colors::bg_header())
                .border_b_1()
                .border_color(colors::border())
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .text_color(colors::text_muted())
                        .child("Key"),
                )
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .text_color(colors::text_muted())
                        .child("Value"),
                )
                .child(div().w(px(120.0)).text_xs().text_color(colors::text_muted()).child("Type"));

            if documents.is_empty() {
                body = body.child(div().flex().flex_1().items_center().justify_center().child(
                    div().text_sm().text_color(colors::text_muted()).child("No documents returned"),
                ));
            } else if row_count == 0 {
                body = body.child(div().flex().flex_1().items_center().justify_center().child(
                    div().text_sm().text_color(colors::text_muted()).child("No matching results"),
                ));
            } else {
                let list = div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(
                        uniform_list(
                            "forge-results-tree",
                            row_count,
                            cx.processor({
                                let documents = documents.clone();
                                let visible_rows = visible_rows.clone();
                                let view_entity = view_entity.clone();
                                move |_view, range: std::ops::Range<usize>, _window, _cx| {
                                    range
                                        .map(|ix| {
                                            let row = &visible_rows[ix];
                                            let meta = compute_row_meta(row, &documents);
                                            render_forge_row(ix, row, &meta, view_entity.clone())
                                        })
                                        .collect()
                                }
                            }),
                        )
                        .flex_1()
                        .track_scroll(scroll_handle),
                    );

                body = body.child(header).child(list);
            }
        } else {
            let (text, color) = if let Some(result) = &self.last_result {
                (result.clone(), colors::text_secondary())
            } else if self.is_running {
                ("Running...".to_string(), colors::text_secondary())
            } else {
                ("No output yet.".to_string(), colors::text_muted())
            };

            body = body.child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .text_xs()
                    .font_family(fonts::mono())
                    .text_color(color)
                    .child(text),
            );
        }

        body
    }

    fn render_raw_output_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.ensure_raw_output_state(window, cx);
        let text = self.build_raw_output_text();
        if text != self.raw_output_text {
            self.raw_output_text = text.clone();
        }
        let current = state.read(cx).value().to_string();
        if current != text {
            self.raw_output_programmatic = true;
            state.update(cx, |state, cx| {
                state.set_value(text, window, cx);
            });
            self.raw_output_programmatic = false;
        }

        Input::new(&state)
            .h_full()
            .appearance(false)
            .bordered(false)
            .focus_bordered(false)
            .font_family(fonts::mono())
            .text_xs()
            .text_color(colors::text_secondary())
    }
}

impl Render for ForgeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.clone();

        // Check if we have an active Forge tab
        let has_forge_tab = state.read(cx).active_forge_tab_id().is_some();
        log::debug!("ForgeView::render - has_forge_tab: {}", has_forge_tab);

        // If no Forge tab, return empty placeholder
        if !has_forge_tab {
            return div().size_full().into_any_element();
        }

        self.ensure_editor_state(window, cx);
        self.sync_active_tab_content(window, cx, false);
        let Some(editor_state) = &self.editor_state else {
            return div().size_full().into_any_element();
        };
        if self.editor_focus_requested {
            self.editor_focus_requested = false;
            let focus = editor_state.read(cx).focus_handle(cx);
            window.focus(&focus);
        };
        let forge_view = cx.entity();
        let editor_child: AnyElement = Input::new(editor_state)
            .appearance(false)
            .font_family(fonts::mono())
            .text_sm()
            .text_color(colors::text_primary())
            .h_full()
            .w_full()
            .into_any_element();
        let editor_for_focus = editor_state.downgrade();
        let forge_focus_handle = self.focus_handle.clone();
        let status_text = if self.mongosh_error.is_some() {
            "Shell error"
        } else if self.is_running {
            "Running..."
        } else {
            "Ready"
        };

        let editor_panel = {
            let mut panel = div()
                .id("forge-editor-container")
                .relative()
                .flex()
                .min_h(px(0.0))
                .p(spacing::md())
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .h_full()
                        .border_1()
                        .border_color(colors::border())
                        .rounded(borders::radius_sm())
                        .bg(colors::bg_app())
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            window.focus(&forge_focus_handle);
                            if let Some(editor) = editor_for_focus.upgrade() {
                                let focus = editor.read(cx).focus_handle(cx);
                                window.focus(&focus);
                            }
                        })
                        .child(editor_child),
                );

            if self.output_visible {
                panel = panel.flex_1().min_h(px(0.0));
            } else {
                panel = panel.flex_1();
            }
            panel
        };

        let output_panel = if self.output_visible {
            Some(self.render_output(window, cx).into_any_element())
        } else {
            None
        };

        let show_output_button = if self.output_visible {
            None
        } else {
            Some(
                Button::new("forge-output-show")
                    .ghost()
                    .compact()
                    .icon(Icon::new(IconName::ChevronDown).xsmall())
                    .label("Show output")
                    .on_click({
                        let forge_view = forge_view.clone();
                        move |_, _window, cx| {
                            forge_view.update(cx, |this, _cx| {
                                this.output_visible = true;
                            });
                        }
                    })
                    .into_any_element(),
            )
        };

        let split_panel = if self.output_visible {
            v_resizable("forge-main-split")
                .child(
                    resizable_panel()
                        .size(px(320.0))
                        .size_range(px(200.0)..px(1200.0))
                        .child(editor_panel),
                )
                .child(
                    resizable_panel()
                        .size(px(320.0))
                        .size_range(px(200.0)..px(1600.0))
                        .child(output_panel.unwrap_or_else(|| div().into_any_element())),
                )
                .into_any_element()
        } else {
            div().flex().flex_col().flex_1().min_h(px(0.0)).child(editor_panel).into_any_element()
        };

        let root = div()
            .key_context("ForgeView")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(colors::bg_app())
            .child(self.render_header(cx))
            .child(div().flex_1().flex().flex_col().min_h(px(0.0)).child(split_panel))
            .child(
                // Status bar / help text
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(spacing::md())
                    .py(spacing::xs())
                    .border_t_1()
                    .border_color(colors::border())
                    .bg(colors::bg_sidebar())
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors::text_muted())
                            .font_family(fonts::ui())
                            .child("⌘↩ Run all | ⌘⇧↩ Run selection/statement | Esc Cancel"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .children(show_output_button)
                            .child(
                                div().text_xs().text_color(colors::text_muted()).child(status_text),
                            )
                            .child(
                                Button::new("forge-restart")
                                    .ghost()
                                    .compact()
                                    .icon(Icon::new(IconName::Redo).xsmall())
                                    .label("Restart")
                                    .on_click({
                                        let forge_view = forge_view.clone();
                                        move |_, _window, cx| {
                                            forge_view.update(cx, |this, cx| {
                                                this.restart_session(cx);
                                            });
                                        }
                                    }),
                            ),
                    ),
            );

        self.bind_root_actions(root, window, cx).into_any_element()
    }
}

// ============================================================================
// Forge Results Tree Rendering (Aggregation-style)
// ============================================================================

fn render_forge_row(
    ix: usize,
    row: &VisibleRow,
    meta: &crate::views::documents::tree::lazy_row::LazyRowMeta,
    view_entity: Entity<ForgeView>,
) -> AnyElement {
    let node_id = row.node_id.clone();
    let depth = row.depth;
    let is_folder = row.is_folder;
    let is_expanded = row.is_expanded;

    let key_label = meta.key_label.clone();
    let value_label = meta.value_label.clone();
    let value_color = meta.value_color;
    let type_label = meta.type_label.clone();

    let leading = if is_folder {
        let toggle_node_id = node_id.clone();
        let toggle_view = view_entity.clone();
        div()
            .id(("forge-row-chevron", ix))
            .w(px(14.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                if event.click_count == 1 {
                    cx.stop_propagation();
                    toggle_view.update(cx, |this, cx| {
                        if this.result_expanded_nodes.contains(&toggle_node_id) {
                            this.result_expanded_nodes.remove(&toggle_node_id);
                        } else {
                            this.result_expanded_nodes.insert(toggle_node_id.clone());
                        }
                        cx.notify();
                    });
                }
            })
            .child(
                Icon::new(if is_expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                    .xsmall()
                    .text_color(colors::text_muted()),
            )
            .into_any_element()
    } else {
        div().w(px(14.0)).into_any_element()
    };

    div()
        .id(("forge-result-row", ix))
        .flex()
        .items_center()
        .w_full()
        .px(spacing::lg())
        .py(spacing::xs())
        .hover(|s| s.bg(colors::list_hover()))
        .on_mouse_down(MouseButton::Left, {
            let node_id = node_id.clone();
            let row_view = view_entity.clone();
            move |event, _window, cx| {
                if event.click_count == 2 && is_folder {
                    row_view.update(cx, |this, cx| {
                        if this.result_expanded_nodes.contains(&node_id) {
                            this.result_expanded_nodes.remove(&node_id);
                        } else {
                            this.result_expanded_nodes.insert(node_id.clone());
                        }
                        cx.notify();
                    });
                }
            }
        })
        .child(render_forge_key_column(depth, leading, &key_label))
        .child(render_forge_value_column(&value_label, value_color))
        .child(
            div()
                .w(px(120.0))
                .text_sm()
                .text_color(colors::text_muted())
                .overflow_hidden()
                .text_ellipsis()
                .child(type_label),
        )
        .into_any_element()
}

fn render_forge_key_column(depth: usize, leading: AnyElement, key_label: &str) -> impl IntoElement {
    let key_label = key_label.to_string();

    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .flex_1()
        .min_w(px(0.0))
        .overflow_hidden()
        .pl(px(14.0 * depth as f32))
        .child(leading)
        .child(
            div()
                .text_sm()
                .text_color(colors::syntax_key())
                .overflow_hidden()
                .text_ellipsis()
                .child(key_label),
        )
}

fn render_forge_value_column(value_label: &str, value_color: Rgba) -> impl IntoElement {
    div().flex_1().min_w(px(0.0)).overflow_hidden().child(
        div()
            .text_sm()
            .text_color(value_color)
            .overflow_hidden()
            .text_ellipsis()
            .child(value_label.to_string()),
    )
}
