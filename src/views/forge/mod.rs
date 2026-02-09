//! Forge - MongoDB Query Shell
//!
//! A database-scoped query shell with a Forge editor for syntax highlighting,
//! autocomplete, and IDE-like experience.

mod actions;
mod completion;
mod controller;
mod editor;
mod editor_behavior;
mod logic;
mod mongosh;
mod output;
mod parser;
mod runtime;
mod state;
mod types;

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::input::Input;
use gpui_component::resizable::{resizable_panel, v_resizable};
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{Icon, IconName, Sizable};

use crate::components::Button;
use crate::state::{AppEvent, AppState, View};
use crate::theme::{borders, fonts, spacing};
use controller::ForgeController;
use output::format_result_tab_label;
use state::ForgeState;
use types::ForgeOutputTab;

// ============================================================================
// ForgeView
// ============================================================================

pub struct ForgeView {
    app_state: Entity<AppState>,
    state: ForgeState,
    controller: ForgeController,
    _subscriptions: Vec<Subscription>,
}

impl ForgeView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let controller = ForgeController::new();

        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&state, |this, state, event, cx| {
                if matches!(event, AppEvent::ViewChanged) {
                    let visible = matches!(state.read(cx).current_view, View::Forge);
                    this.state.editor.editor_focus_requested = visible;
                    cx.notify();
                }
            }),
        ];

        Self {
            app_state: state,
            state: ForgeState::new(focus_handle),
            controller,
            _subscriptions: subscriptions,
        }
    }

    fn render_header(&self, cx: &App) -> impl IntoElement {
        let (database, _connection_name) = {
            let state_ref = self.app_state.read(cx);
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
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child("Forge"),
                    )
                    .child(
                        div()
                            .px(spacing::sm())
                            .py(px(2.0))
                            .rounded(borders::radius_sm())
                            .bg(cx.theme().sidebar)
                            .text_xs()
                            .text_color(cx.theme().secondary_foreground)
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
                        if let Some(raw_state) = &this.state.output.raw_output_state {
                            this.state.output.raw_output_programmatic = true;
                            raw_state.update(_cx, |state, cx| {
                                state.set_value(String::new(), _window, cx);
                            });
                            this.state.output.raw_output_programmatic = false;
                        }
                        _cx.notify();
                    });
                }
            });

        let selected_index = match self.state.output.output_tab {
            ForgeOutputTab::Raw => 0,
            ForgeOutputTab::Results => {
                if self.state.output.result_pages.is_empty() {
                    1
                } else {
                    self.state
                        .output
                        .result_page_index
                        .min(self.state.output.result_pages.len().saturating_sub(1))
                        + 1
                }
            }
        };

        let has_inline_result = self.state.output.last_result.is_some()
            || self.state.output.last_error.is_some()
            || self.state.runtime.mongosh_error.is_some();

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
                            this.state.output.output_tab = ForgeOutputTab::Raw;
                        } else {
                            this.state.output.output_tab = ForgeOutputTab::Results;
                            if !this.state.output.result_pages.is_empty() {
                                ForgeController::select_result_page(this, index - 1);
                            }
                        }
                    });
                }
            })
            .children(
                std::iter::once(Tab::new().label("Raw output"))
                    .chain(if self.state.output.result_pages.is_empty() && has_inline_result {
                        vec![Tab::new().label("Shell Output")].into_iter()
                    } else {
                        Vec::new().into_iter()
                    })
                    .chain(self.state.output.result_pages.iter().enumerate().map(
                        |(index, page)| {
                            let label = format_result_tab_label(&page.label, index);
                            let view_entity = forge_view.clone();
                            let pin_icon = if page.pinned {
                                Icon::new(IconName::Star).xsmall().text_color(cx.theme().primary)
                            } else {
                                Icon::new(IconName::StarOff)
                                    .xsmall()
                                    .text_color(cx.theme().muted_foreground)
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
                                .hover(|s| s.bg(cx.theme().list_hover))
                                .child(pin_icon)
                                .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                    cx.stop_propagation();
                                    view_entity.update(cx, |this, _cx| {
                                        ForgeController::toggle_result_pinned(this, index);
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
                                .hover(|s| s.bg(cx.theme().list_hover))
                                .child(
                                    Icon::new(IconName::Close)
                                        .xsmall()
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                    cx.stop_propagation();
                                    view_entity.update(cx, |this, _cx| {
                                        ForgeController::close_result_page(this, index);
                                    });
                                });

                            Tab::new().label(label).prefix(pin_button).suffix(close_button)
                        },
                    )),
            );

        let body: AnyElement = match self.state.output.output_tab {
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
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
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
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Output"),
                            )
                            .child(tab_bar),
                    )
                    .child(clear_button),
            )
            .child(body)
    }

    // render_results_body/render_raw_output_body moved to output module
}

impl Render for ForgeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.app_state.clone();

        // Check if we have an active Forge tab
        let has_forge_tab = state.read(cx).active_forge_tab_id().is_some();
        log::debug!("ForgeView::render - has_forge_tab: {}", has_forge_tab);

        // If no Forge tab, return empty placeholder
        if !has_forge_tab {
            return div().size_full().into_any_element();
        }

        self.ensure_editor_state(window, cx);
        self.sync_active_tab_content(window, cx, false);
        let Some(editor_state) = &self.state.editor.editor_state else {
            return div().size_full().into_any_element();
        };
        if self.state.editor.editor_focus_requested {
            self.state.editor.editor_focus_requested = false;
            let focus = editor_state.read(cx).focus_handle(cx);
            window.focus(&focus);
        };
        let forge_view = cx.entity();
        let editor_child: AnyElement = Input::new(editor_state)
            .appearance(false)
            .font_family(fonts::mono())
            .text_sm()
            .text_color(cx.theme().foreground)
            .h_full()
            .w_full()
            .into_any_element();
        let editor_for_focus = editor_state.downgrade();
        let forge_focus_handle = self.state.focus_handle.clone();
        let status_text = if self.state.runtime.mongosh_error.is_some() {
            "Shell error"
        } else if self.state.runtime.is_running {
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
                        .border_color(cx.theme().border)
                        .rounded(borders::radius_sm())
                        .bg(cx.theme().background)
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

            if self.state.output.output_visible {
                panel = panel.flex_1().min_h(px(0.0));
            } else {
                panel = panel.flex_1();
            }
            panel
        };

        let output_panel = if self.state.output.output_visible {
            Some(self.render_output(window, cx).into_any_element())
        } else {
            None
        };

        let show_output_button = if self.state.output.output_visible {
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
                                this.state.output.output_visible = true;
                            });
                        }
                    })
                    .into_any_element(),
            )
        };

        let split_panel = if self.state.output.output_visible {
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
            .track_focus(&self.state.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().background)
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
                    .border_color(cx.theme().border)
                    .bg(cx.theme().sidebar)
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
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
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(status_text),
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

        actions::bind_root_actions(root, window, cx).into_any_element()
    }
}

// ============================================================================
// Forge Results Tree Rendering (Aggregation-style)
// ============================================================================
