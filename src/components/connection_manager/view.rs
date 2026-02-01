//! Main render implementation for ConnectionManager.
//!
//! This file contains the core `Render` impl that composes the connection list
//! and editor panels. Tab-specific rendering is in `tabs.rs` and
//! connection list rendering is in `connection_list.rs`.

use gpui::*;
use gpui_component::WindowExt as _;
use gpui_component::scroll::ScrollableElement;
use gpui_component::tab::{Tab, TabBar};

use crate::components::Button;
use crate::state::AppCommands;
use crate::theme::{colors, sizing, spacing};

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

        div().flex().flex_row().flex_1().h_full().w_full().child(list).child(editor)
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

        let tab_bar = self.render_tab_bar(cx);
        let status_bar = self.render_status_bar(is_active_selection, cx);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w(px(0.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(spacing::md())
                    .h(sizing::header_height())
                    .border_b_1()
                    .border_color(colors::border())
                    .child(tab_bar),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .overflow_y_scrollbar()
                            .p(spacing::md())
                            .child(match active_tab {
                                ManagerTab::General => {
                                    self.render_general_tab(parse_error, window, cx)
                                }
                                ManagerTab::Auth => self.render_auth_tab(window, cx),
                                ManagerTab::Options => self.render_options_tab(window, cx),
                                ManagerTab::Tls => self.render_tls_tab(window, cx),
                                ManagerTab::Pool => self.render_pool_tab(window, cx),
                                ManagerTab::Advanced => self.render_advanced_tab(window, cx),
                            }),
                    )
                    .child(status_bar),
            )
            .into_any_element()
    }

    /// Renders the tab bar for switching between configuration sections.
    fn render_tab_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let active_tab = self.active_tab;

        TabBar::new("connection-manager-tabs")
            .selected_index(active_tab.index())
            .on_click({
                move |index, _window, cx| {
                    let index = *index;
                    view.update(cx, |this, cx| {
                        this.active_tab = ManagerTab::from_index(index);
                        cx.notify();
                    });
                }
            })
            .children(ManagerTab::all().into_iter().map(|tab| Tab::new().label(tab.label())))
            .into_any_element()
    }

    /// Renders the status bar with test status and action buttons.
    fn render_status_bar(&self, is_active_selection: bool, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let state = self.state.clone();
        let status = self.status.clone();
        let is_testing = matches!(status, TestStatus::Testing);

        let (status_text, status_color) = Self::format_status(&status);
        let actions = Self::render_action_buttons(view, state, is_testing, is_active_selection);

        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::sm())
            .px(spacing::md())
            .h(sizing::header_height())
            .border_t_1()
            .border_color(colors::border())
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
    fn format_status(status: &TestStatus) -> (String, Hsla) {
        let text = match status {
            TestStatus::Idle => "Test connection to verify settings".to_string(),
            TestStatus::Testing => "Testing connection...".to_string(),
            TestStatus::Success => "Connection OK".to_string(),
            TestStatus::Error(err) => format!("Connection failed: {err}"),
        };
        let color = match status {
            TestStatus::Success => colors::accent(),
            TestStatus::Error(_) => colors::status_error(),
            _ => colors::text_muted(),
        };
        (text, color.into())
    }

    /// Renders the action buttons (Test, Save, Close).
    fn render_action_buttons(
        view: Entity<Self>,
        state: Entity<crate::state::AppState>,
        is_testing: bool,
        is_active_selection: bool,
    ) -> AnyElement {
        div()
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
                    .label(if is_active_selection { "Save & Reconnect" } else { "Save" })
                    .on_click({
                        let view = view.clone();
                        let state = state.clone();
                        move |_, window, cx| {
                            let mut reconnect_id = None;
                            view.update(cx, |this, cx| {
                                let saved_id = this.save_connection(window, cx);
                                if let Some(saved_id) = saved_id
                                    && this.state.read(cx).is_connected(saved_id)
                                {
                                    reconnect_id = Some(saved_id);
                                }
                            });
                            if let Some(connection_id) = reconnect_id {
                                AppCommands::connect(state.clone(), connection_id, cx);
                            }
                        }
                    }),
            )
            .child(Button::new("close-manager").compact().label("Close").on_click(
                |_, window, cx| {
                    window.close_dialog(cx);
                },
            ))
            .into_any_element()
    }
}
