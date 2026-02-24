//! Main render implementation for ConnectionManager.
//!
//! This file contains the core `Render` impl that composes the connection list
//! and editor panels. Tab-specific rendering is in `tabs.rs` and
//! connection list rendering is in `connection_list.rs`.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::scroll::ScrollableElement;
use gpui_component::tab::{Tab, TabBar};

use crate::components::Button;
use crate::state::AppCommands;
use crate::theme::{sizing, spacing};

use super::{ConnectionManager, ManagerTab, TestStatus};

impl Render for ConnectionManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connections = self.state.read(cx).connections_snapshot();
        let selected_exists =
            self.selected_id.is_some_and(|id| connections.iter().any(|conn| conn.id == id));
        if !selected_exists && !connections.is_empty() && !self.creating_new {
            let next = connections.first().cloned();
            self.load_connection(next, window, cx);
        }

        let selected_id = self.selected_id;
        let is_active_selection =
            selected_id.is_some_and(|id| self.state.read(cx).is_connected(id));

        let list = self.render_connection_list(connections, selected_id, window, cx);
        let editor = self.render_editor_panel(is_active_selection, window, cx);

        // Dialog chrome (padding, title, gap, border) eats ~86px from the
        // dialog height (vp.height - 200).  Pin to an explicit pixel height
        // so the Dialog's own overflow_y_scrollbar never activates.
        let content_h = window.viewport_size().height - px(290.0);
        div().flex().flex_row().w_full().h(content_h).overflow_hidden().child(list).child(editor)
    }
}

impl ConnectionManager {
    /// Renders the right-side editor panel with tabs and form fields.
    fn render_editor_panel(
        &mut self,
        is_active_selection: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active_tab = self.active_tab;
        let parse_error = self.parse_error.clone();
        let global_parse_error =
            if active_tab == ManagerTab::General { None } else { parse_error.clone() };
        let parse_error_border = cx.theme().danger.opacity(0.5);
        let parse_error_bg = cx.theme().danger.opacity(0.08);
        let parse_error_fg = cx.theme().danger_foreground;

        let tab_bar = self.render_tab_bar(cx);
        let status_bar = self.render_status_bar(is_active_selection, cx);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .min_w(px(0.0))
            // Tab bar — sticky top
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .px(spacing::md())
                    .h(sizing::header_height())
                    .child(div().flex_1().min_w(px(0.0)).child(tab_bar)),
            )
            // Tab content — scrollable
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .p(spacing::md())
                    .when_some(global_parse_error, |this, err| {
                        this.child(
                            div()
                                .mb(spacing::md())
                                .rounded_md()
                                .border_1()
                                .border_color(parse_error_border)
                                .bg(parse_error_bg)
                                .px(spacing::sm())
                                .py(spacing::xs())
                                .text_xs()
                                .text_color(parse_error_fg)
                                .child(err),
                        )
                    })
                    .child(match active_tab {
                        ManagerTab::General => self.render_general_tab(parse_error, window, cx),
                        ManagerTab::Tls => self.render_tls_tab(window, cx),
                        ManagerTab::Network => self.render_network_tab(window, cx),
                        ManagerTab::Advanced => self.render_advanced_tab(window, cx),
                    }),
            )
            // Status bar — sticky bottom
            .child(div().flex_shrink_0().child(status_bar))
            .into_any_element()
    }

    /// Renders the tab bar for switching between configuration sections.
    fn render_tab_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let active_tab = self.active_tab;

        TabBar::new("connection-manager-tabs")
            .underline()
            .small()
            .min_w(px(0.0))
            .menu(false)
            .selected_index(active_tab.index())
            .on_click(move |index, _window, cx| {
                let index = *index;
                view.update(cx, |this, cx| {
                    this.active_tab = ManagerTab::from_index(index);
                    cx.notify();
                });
            })
            .children(ManagerTab::all().into_iter().map(|tab| Tab::new().label(tab.label())))
            .into_any_element()
    }

    /// Renders the status bar with test status and action buttons.
    fn render_status_bar(&self, is_active_selection: bool, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let state = self.state.clone();
        let status = self.status.clone();
        let testing_step = self.testing_step.clone();
        let is_testing = matches!(status, TestStatus::Testing);
        let error_details = match &status {
            TestStatus::Error(err) => Some(err.clone()),
            _ => None,
        };

        let (status_text, status_color) = Self::format_status(&status, testing_step.as_deref(), cx);
        let actions = Self::render_action_buttons(
            view,
            state,
            is_testing,
            is_active_selection,
            error_details,
        );

        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::sm())
            .px(spacing::md())
            .h(sizing::header_height())
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_xs()
                    .text_color(status_color)
                    .truncate()
                    .child(status_text),
            )
            .child(actions)
            .into_any_element()
    }

    /// Formats the test status into a display string and color.
    fn format_status(status: &TestStatus, testing_step: Option<&str>, cx: &App) -> (String, Hsla) {
        let text = match status {
            TestStatus::Idle => "Test connection to verify settings".to_string(),
            TestStatus::Testing => testing_step
                .map(|step| format!("Testing: {step}"))
                .unwrap_or_else(|| "Testing connection...".to_string()),
            TestStatus::Success => "Connection OK".to_string(),
            TestStatus::Error(_) => "Connection failed. Open Details for diagnostics.".to_string(),
        };
        let color = match status {
            TestStatus::Success => cx.theme().primary,
            TestStatus::Error(_) => cx.theme().danger,
            _ => cx.theme().muted_foreground,
        };
        (text, color)
    }

    /// Renders the action buttons (Test, Save, Close).
    fn render_action_buttons(
        view: Entity<Self>,
        state: Entity<crate::state::AppState>,
        is_testing: bool,
        is_active_selection: bool,
        error_details: Option<String>,
    ) -> AnyElement {
        let row = div()
            .flex()
            .items_center()
            .gap(spacing::sm())
            .flex_shrink_0()
            .child(
                Button::new("test-connection")
                    .compact()
                    .label(if is_testing { "Testing..." } else { "Test" })
                    .disabled(is_testing)
                    .on_click({
                        let view = view.clone();
                        move |_, _window, cx| {
                            ConnectionManager::start_test(view.clone(), cx);
                        }
                    }),
            )
            .child(
                Button::new("save-connection")
                    .compact()
                    .primary()
                    .label(if is_active_selection { "Save & Reconnect" } else { "Save & Connect" })
                    .on_click({
                        let view = view.clone();
                        let state = state.clone();
                        move |_, window, cx| {
                            let mut saved_id = None;
                            view.update(cx, |this, cx| {
                                saved_id = this.save_connection(window, cx);
                            });
                            if let Some(connection_id) = saved_id {
                                AppCommands::connect(state.clone(), connection_id, cx);
                            }
                        }
                    }),
            )
            .child(Button::new("close-manager").compact().label("Close").on_click(
                |_, window, cx| {
                    window.close_dialog(cx);
                },
            ));

        let row = if let Some(error_details) = error_details {
            row.child(Button::new("test-details").compact().label("Details").on_click(
                move |_, window, cx| {
                    let details = error_details.clone();
                    window.open_dialog(cx, move |dialog: Dialog, _window, _cx| {
                        dialog
                            .title("Connection Test Details")
                            .w(px(780.0))
                            .margin_top(px(20.0))
                            .min_h(px(420.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .h_full()
                                    .overflow_y_scrollbar()
                                    .p(spacing::md())
                                    .text_xs()
                                    .font_family(crate::theme::fonts::mono())
                                    .child(details.clone()),
                            )
                    });
                },
            ))
        } else {
            row
        };

        row.into_any_element()
    }
}
