use gpui::*;
use gpui_component::Sizable as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::switch::Switch;

use crate::components::{Button, cancel_button};
use crate::helpers::{extract_host_from_uri, validate_mongodb_uri};
use crate::models::SavedConnection;
use crate::state::AppState;
use crate::theme::{colors, spacing};

#[derive(Clone, Debug)]
enum TestStatus {
    Idle,
    Testing,
    Success,
    Error(String),
}

pub struct ConnectionDialog {
    state: Entity<AppState>,
    name_state: Entity<InputState>,
    uri_state: Entity<InputState>,
    read_only: bool,
    status: TestStatus,
    last_tested_uri: Option<String>,
    pending_test_uri: Option<String>,
    existing: Option<SavedConnection>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectionDialog {
    pub fn open(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
        let dialog_view = cx.new(|cx| ConnectionDialog::new(state.clone(), window, cx));
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("New Connection").min_w(px(420.0)).child(dialog_view.clone())
        });
    }

    #[allow(dead_code)]
    pub fn open_edit(
        state: Entity<AppState>,
        connection: SavedConnection,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view =
            cx.new(|cx| ConnectionDialog::new_with_existing(state.clone(), connection, window, cx));
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("Edit Connection").min_w(px(420.0)).child(dialog_view.clone())
        });
    }

    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name_state =
            cx.new(|cx| InputState::new(window, cx).placeholder("My MongoDB").default_value(""));

        let uri_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("mongodb://localhost:27017")
                .default_value("mongodb://localhost:27017")
        });

        let mut subscriptions = vec![];
        subscriptions.push(cx.subscribe_in(
            &uri_state,
            window,
            move |view, _state, event, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.status = TestStatus::Idle;
                    view.last_tested_uri = None;
                    view.pending_test_uri = None;
                    cx.notify();
                }
            },
        ));

        Self {
            state,
            name_state,
            uri_state,
            read_only: false,
            status: TestStatus::Idle,
            last_tested_uri: None,
            pending_test_uri: None,
            existing: None,
            _subscriptions: subscriptions,
        }
    }

    #[allow(dead_code)]
    pub fn new_with_existing(
        state: Entity<AppState>,
        existing: SavedConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("My MongoDB")
                .default_value(existing.name.clone())
        });

        let uri_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("mongodb://localhost:27017")
                .default_value(existing.uri.clone())
        });

        let mut subscriptions = vec![];
        subscriptions.push(cx.subscribe_in(
            &uri_state,
            window,
            move |view, _state, event, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.status = TestStatus::Idle;
                    view.last_tested_uri = None;
                    view.pending_test_uri = None;
                    cx.notify();
                }
            },
        ));

        Self {
            state,
            name_state,
            uri_state,
            read_only: existing.read_only,
            status: TestStatus::Success,
            last_tested_uri: Some(existing.uri.clone()),
            pending_test_uri: None,
            existing: Some(existing),
            _subscriptions: subscriptions,
        }
    }

    fn start_test(view: Entity<ConnectionDialog>, cx: &mut App) {
        let uri = view.read(cx).uri_state.read(cx).value().to_string();
        if let Err(err) = validate_mongodb_uri(&uri) {
            view.update(cx, |this, cx| {
                this.status = TestStatus::Error(err.to_string());
                this.last_tested_uri = None;
                this.pending_test_uri = None;
                cx.notify();
            });
            return;
        }

        let manager = view.read(cx).state.read(cx).connection_manager();

        view.update(cx, |this, cx| {
            this.status = TestStatus::Testing;
            this.pending_test_uri = Some(uri.clone());
            this.last_tested_uri = None;
            cx.notify();
        });

        let task = cx.background_spawn({
            let uri = uri.clone();
            async move {
                let temp = SavedConnection::new("Test".to_string(), uri);
                manager.test_connection(&temp, std::time::Duration::from_secs(5))?;
                Ok::<(), crate::error::Error>(())
            }
        });

        cx.spawn({
            let view = view.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                let _ = cx.update(|cx| {
                    view.update(cx, |this, cx| {
                        let current_uri = this.uri_state.read(cx).value().to_string();
                        let pending = this.pending_test_uri.clone();
                        if pending.as_deref() != Some(current_uri.trim()) {
                            this.status = TestStatus::Idle;
                            this.pending_test_uri = None;
                            this.last_tested_uri = None;
                            cx.notify();
                            return;
                        }

                        match result {
                            Ok(()) => {
                                this.status = TestStatus::Success;
                                this.last_tested_uri = Some(current_uri);
                            }
                            Err(err) => {
                                this.status = TestStatus::Error(err.to_string());
                                this.last_tested_uri = None;
                            }
                        }
                        this.pending_test_uri = None;
                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }
}

impl Render for ConnectionDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let uri_value = self.uri_state.read(cx).value().to_string();
        let uri_trimmed = uri_value.trim();
        let is_testing = matches!(self.status, TestStatus::Testing);
        let can_save = matches!(self.status, TestStatus::Success)
            && self.last_tested_uri.as_deref() == Some(uri_trimmed);
        let is_edit = self.existing.is_some();

        let (status_text, status_color) = match &self.status {
            TestStatus::Idle => {
                ("Test connection to enable Save".to_string(), colors::text_muted())
            }
            TestStatus::Testing => ("Testing connection...".to_string(), colors::text_muted()),
            TestStatus::Success => ("Connection OK".to_string(), colors::accent()),
            TestStatus::Error(msg) => (format!("Connection failed: {msg}"), colors::status_error()),
        };

        div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .p(spacing::md())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("Name"))
                    .child(Input::new(&self.name_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("URI"))
                    .child(Input::new(&self.uri_state))
                    .child(div().text_xs().text_color(status_color).child(status_text)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Switch::new("connection-read-only")
                            .checked(self.read_only)
                            .small()
                            .on_click({
                                let view = cx.entity();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.read_only = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Read-only (safe mode)"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors::text_secondary())
                                    .child("Disable all writes, deletes, drops, and index changes"),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(spacing::sm())
                    .child(
                        Button::new("test-connection")
                            .label(if is_testing { "Testing..." } else { "Test" })
                            .disabled(is_testing || uri_trimmed.is_empty())
                            .on_click({
                                let view = cx.entity();
                                move |_, _window, cx| {
                                    ConnectionDialog::start_test(view.clone(), cx);
                                }
                            }),
                    )
                    .child(cancel_button("cancel"))
                    .child(
                        Button::new("save-connection")
                            .primary()
                            .label(if is_edit { "Update" } else { "Save" })
                            .disabled(!can_save)
                            .on_click({
                                let state = self.state.clone();
                                let name_state = self.name_state.clone();
                                let uri_state = self.uri_state.clone();
                                let read_only = self.read_only;
                                let existing = self.existing.clone();
                                move |_, window, cx| {
                                    let name_input = name_state.read(cx).value().to_string();
                                    let uri = uri_state.read(cx).value().to_string();

                                    if validate_mongodb_uri(&uri).is_err() {
                                        return;
                                    }

                                    let name = if name_input.trim().is_empty() {
                                        extract_host_from_uri(&uri)
                                            .unwrap_or_else(|| "Untitled".to_string())
                                    } else {
                                        name_input.trim().to_string()
                                    };

                                    state.update(cx, |state, cx| {
                                        if let Some(existing) = existing.clone() {
                                            let connection = SavedConnection {
                                                id: existing.id,
                                                name,
                                                uri,
                                                last_connected: existing.last_connected,
                                                read_only,
                                            };
                                            state.update_connection(connection, cx);
                                        } else {
                                            let mut connection = SavedConnection::new(name, uri);
                                            connection.read_only = read_only;
                                            state.add_connection(connection, cx);
                                        }
                                    });
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
    }
}
