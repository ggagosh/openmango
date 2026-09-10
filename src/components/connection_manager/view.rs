use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{ConnectionManager, ManagerTab, TestStatus};
use crate::theme::{islands, spacing};

impl Render for ConnectionManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self
            .last_tested_fingerprint
            .as_ref()
            .is_some_and(|tested| tested != &self.draft.fingerprint(cx))
        {
            self.status = TestStatus::Idle;
            self.last_tested_fingerprint = None;
        }
        let is_active = self.selected_id.is_some_and(|id| self.state.read(cx).is_connected(id));
        let list = self.render_connection_list(cx);
        let editor = self.render_editor_panel(is_active, window, cx);
        div()
            .flex()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .child(list)
            .child(editor)
    }
}

impl ConnectionManager {
    fn render_editor_panel(
        &mut self,
        is_active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let appearance = self.state.read(cx).settings.appearance.clone();
        let title = if self.creating_new {
            "New connection".to_string()
        } else {
            let name = self.draft.name_state.read(cx).value().trim().to_string();
            if name.is_empty() { "Connection".to_string() } else { name }
        };
        let dirty = self.has_unsaved_changes(cx);
        let view = cx.entity();
        let tabs = islands::tab_bar(TabBar::new("connection-manager-tabs"), &appearance)
            .small()
            .min_w(px(0.))
            .selected_index(self.active_tab.index())
            .on_click({
                let view = view.clone();
                move |index, window, cx| {
                    view.update(cx, |this, cx| {
                        // Publish structured edits before returning to the URI.
                        if this.active_tab != ManagerTab::General && this.has_unsaved_changes(cx) {
                            this.update_uri_from_fields(window, cx);
                        }
                        this.active_tab = ManagerTab::from_index(*index);
                        cx.notify();
                    });
                }
            })
            .children(ManagerTab::all().into_iter().map(|tab| Tab::new().label(tab.label())));
        let content = match self.active_tab {
            ManagerTab::General => self.render_general_tab(None, window, cx),
            ManagerTab::Authentication => self.render_authentication_tab(),
            ManagerTab::Tls => self.render_tls_tab(window, cx),
            ManagerTab::Network => self.render_network_tab(window, cx),
            ManagerTab::Advanced => self.render_advanced_tab(window, cx),
            ManagerTab::Access => self.render_access_tab(cx),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .h_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .px(spacing::md())
                    .py(spacing::sm())
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(title),
                    )
                    .when(dirty, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Unsaved changes"),
                        )
                    })
                    .when(self.selected_id.is_some(), |this| {
                        this.child(
                            Button::new("remove-connection")
                                .small()
                                .ghost()
                                .label("Remove…")
                                .disabled(self.pending_save.is_some())
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |this, cx| this.remove_connection(window, cx))
                                }),
                        )
                    }),
            )
            .child(div().px(spacing::md()).child(tabs))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scrollbar()
                    .p(spacing::md())
                    .child(div().w_full().max_w(px(720.)).child(content)),
            )
            .child(self.render_status_bar(is_active, cx))
            .into_any_element()
    }

    fn render_status_bar(&self, is_active: bool, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let saving = self.pending_save.is_some();
        let testing = matches!(self.status, TestStatus::Testing);
        let connecting = self.connecting_id.is_some() && self.connecting_id == self.selected_id;
        let busy =
            saving || testing || connecting || !self.state.read(cx).connection_secrets_ready();
        let dirty = self.has_unsaved_changes(cx);
        let needs_reconnect =
            self.selected_id.is_some_and(|id| self.state.read(cx).connection_needs_reconnect(id));
        let message = if saving {
            "Saving connection…".to_string()
        } else if connecting {
            "Connecting…".to_string()
        } else if let Some(error) = &self.parse_error {
            error.clone()
        } else {
            match &self.status {
                TestStatus::Idle => if needs_reconnect {
                    "Connected using previous settings. Reconnect to apply saved changes."
                } else if is_active {
                    "Connected"
                } else if self.creating_new {
                    "Test is optional. Save keeps this connection for later."
                } else {
                    "Disconnected"
                }
                .to_string(),
                TestStatus::Testing => {
                    self.testing_step.clone().unwrap_or_else(|| "Testing connection…".into())
                }
                TestStatus::Success => "Test succeeded".into(),
                TestStatus::Error(error) => error.clone(),
                TestStatus::Saved => if needs_reconnect {
                    "Connection saved. Reconnect to apply changes."
                } else {
                    "Connection saved"
                }
                .into(),
            }
        };
        let error = matches!(self.status, TestStatus::Error(_)) || self.parse_error.is_some();
        let mut actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(spacing::sm())
            .when(self.creating_new, |row| {
                row.child(
                    Button::new("cancel-new-connection")
                        .small()
                        .ghost()
                        .label("Cancel")
                        .disabled(saving)
                        .on_click({
                            let view = view.clone();
                            move |_, window, cx| Self::request_cancel_new(view.clone(), window, cx)
                        }),
                )
            })
            .child(
                Button::new("test-connection")
                    .small()
                    .label("Test")
                    .loading(testing)
                    .disabled(busy)
                    .on_click({
                        let view = view.clone();
                        move |_, window, cx| Self::start_test(view.clone(), window, cx)
                    }),
            )
            .map(|row| {
                row.child(
                    Button::new("save-connection")
                        .small()
                        .label("Save")
                        .disabled(busy || (!dirty && !self.creating_new))
                        .on_click({
                            let view = view.clone();
                            move |_, window, cx| Self::request_save(view.clone(), false, window, cx)
                        }),
                )
            })
            .child(
                Button::new("save-connect")
                    .small()
                    .primary()
                    .loading(connecting)
                    .disabled(busy)
                    .label(if is_active {
                        if dirty { "Save & Reconnect" } else { "Reconnect" }
                    } else if self.creating_new || dirty {
                        "Save & Connect"
                    } else {
                        "Connect"
                    })
                    .on_click({
                        let view = view.clone();
                        move |_, window, cx| Self::request_save(view.clone(), true, window, cx)
                    }),
            );
        if let TestStatus::Error(details) = &self.status {
            let details = details.clone();
            actions = actions.child(
                Button::new("connection-error-details").small().ghost().label("Details").on_click(
                    move |_, window, cx| {
                        let details = details.clone();
                        window.open_dialog(cx, move |dialog: Dialog, _, _| {
                            dialog.title("Connection details").child(
                                div()
                                    .max_h(px(360.))
                                    .overflow_y_scrollbar()
                                    .text_sm()
                                    .child(details.clone()),
                            )
                        });
                    },
                ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .px(spacing::md())
            .py(spacing::sm())
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_xs()
                    .text_color(if error { cx.theme().danger } else { cx.theme().muted_foreground })
                    .child(message),
            )
            .child(actions)
            .into_any_element()
    }
}
