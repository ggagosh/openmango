use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::switch::Switch;

use crate::components::{Button, cancel_button, request_unsaved_action};
use crate::helpers::{
    UriSecrets, extract_host_from_uri, extract_uri_secrets, inject_uri_secrets, strip_uri_secrets,
    validate_mongodb_uri,
};
use crate::models::{ConnectionEnvironment, SavedConnection};
use crate::state::{AppState, UnsavedScope};
use crate::theme::spacing;

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
    password_state: Entity<InputState>,
    uri_secrets: UriSecrets,
    sanitizing_uri: bool,
    environment: Option<ConnectionEnvironment>,
    confirm_production_writes: bool,
    read_only: bool,
    agent_shared: bool,
    protected: bool,
    reversible_history: bool,
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

    fn capture_uri_secrets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sanitizing_uri {
            self.sanitizing_uri = false;
            return;
        }
        let uri = self.uri_state.read(cx).value().to_string();
        let secrets = extract_uri_secrets(&uri);
        self.password_state.update(cx, |state, cx| {
            state.set_value(secrets.password.clone().unwrap_or_default(), window, cx);
        });
        self.uri_secrets = UriSecrets { password: None, ..secrets };
        let sanitized = strip_uri_secrets(&uri);
        if sanitized != uri {
            self.sanitizing_uri = true;
            self.uri_state.update(cx, |state, cx| {
                state.set_value(sanitized, window, cx);
            });
        }
    }

    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name_state =
            cx.new(|cx| InputState::new(window, cx).placeholder("My MongoDB").default_value(""));

        let uri_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("mongodb://localhost:27017")
                .default_value("mongodb://localhost:27017")
        });

        let password_state =
            cx.new(|cx| InputState::new(window, cx).placeholder("password").masked(true));

        let mut subscriptions = vec![];
        subscriptions.push(cx.subscribe_in(
            &uri_state,
            window,
            move |view, _state, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.status = TestStatus::Idle;
                    view.last_tested_uri = None;
                    view.pending_test_uri = None;

                    view.capture_uri_secrets(window, cx);
                    cx.notify();
                }
            },
        ));

        Self {
            state,
            name_state,
            uri_state,
            password_state,
            uri_secrets: UriSecrets::default(),
            sanitizing_uri: false,
            environment: None,
            confirm_production_writes: false,
            read_only: false,
            agent_shared: false,
            protected: false,
            reversible_history: false,
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

        let extracted_secrets = extract_uri_secrets(&existing.uri);
        let redacted_default = strip_uri_secrets(&existing.uri);

        let uri_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("mongodb://localhost:27017")
                .default_value(redacted_default.clone())
        });

        let password_state = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("password").masked(true);
            if let Some(ref pw) = extracted_secrets.password {
                s = s.default_value(pw.clone());
            }
            s
        });

        let mut subscriptions = vec![];
        subscriptions.push(cx.subscribe_in(
            &uri_state,
            window,
            move |view, _state, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.status = TestStatus::Idle;
                    view.last_tested_uri = None;
                    view.pending_test_uri = None;

                    view.capture_uri_secrets(window, cx);
                    cx.notify();
                }
            },
        ));

        let uri_secrets = UriSecrets { password: None, ..extracted_secrets };
        Self {
            state,
            name_state,
            uri_state,
            password_state,
            uri_secrets,
            sanitizing_uri: false,
            environment: existing.environment,
            confirm_production_writes: existing.confirm_production_writes,
            read_only: existing.read_only,
            agent_shared: existing.agent_shared,
            protected: existing.protected,
            reversible_history: existing.reversible_history,
            status: TestStatus::Success,
            last_tested_uri: Some(redacted_default),
            pending_test_uri: None,
            existing: Some(existing),
            _subscriptions: subscriptions,
        }
    }

    fn real_uri(&self, cx: &App) -> String {
        let uri = self.uri_state.read(cx).value().to_string();
        let password = self.password_state.read(cx).value().trim().to_string();
        let mut secrets = self.uri_secrets.clone();
        secrets.password = (!password.is_empty()).then_some(password);
        inject_uri_secrets(&strip_uri_secrets(&uri), &secrets)
    }

    fn start_test(view: Entity<ConnectionDialog>, cx: &mut App) {
        let display_uri = view.read(cx).uri_state.read(cx).value().to_string();
        let real_uri = view.read(cx).real_uri(cx);
        if let Err(err) = validate_mongodb_uri(&real_uri) {
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
            this.pending_test_uri = Some(display_uri);
            this.last_tested_uri = None;
            cx.notify();
        });

        let task = cx.background_spawn({
            async move {
                let temp = SavedConnection::new("Test".to_string(), real_uri);
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
        let is_connected =
            self.existing.as_ref().is_some_and(|e| self.state.read(cx).is_connected(e.id));
        let selected_environment = self.environment;
        let view = cx.entity();
        let mut no_environment = Button::new("simple-environment-none")
            .compact()
            .label(if selected_environment.is_none() { "✓ Not set" } else { "Not set" })
            .on_click({
                let view = view.clone();
                move |_, _window, cx| {
                    view.update(cx, |this, cx| {
                        this.environment = None;
                        cx.notify();
                    });
                }
            });
        if selected_environment.is_none() {
            no_environment = no_environment.primary();
        }
        let environments = ConnectionEnvironment::ALL
            .into_iter()
            .map(|environment| {
                let view = view.clone();
                let mut button = Button::new(("simple-environment", environment as usize))
                    .compact()
                    .label(if selected_environment == Some(environment) {
                        format!("✓ {}", environment.label())
                    } else {
                        environment.label().to_string()
                    })
                    .on_click(move |_, _window, cx| {
                        view.update(cx, |this, cx| {
                            if environment == ConnectionEnvironment::Production
                                && this.environment != Some(ConnectionEnvironment::Production)
                            {
                                this.agent_shared = false;
                            }
                            this.environment = Some(environment);
                            cx.notify();
                        });
                    });
                if selected_environment == Some(environment) {
                    button = button.primary();
                }
                button.into_any_element()
            })
            .collect::<Vec<_>>();
        let save_label = if is_edit {
            if is_connected { "Update & Reconnect" } else { "Update & Connect" }
        } else {
            "Save & Connect"
        };

        let (status_text, status_color) = match &self.status {
            TestStatus::Idle => {
                ("Test connection to enable Save".to_string(), cx.theme().muted_foreground)
            }
            TestStatus::Testing => {
                ("Testing connection...".to_string(), cx.theme().muted_foreground)
            }
            TestStatus::Success => ("Connection OK".to_string(), cx.theme().primary),
            TestStatus::Error(msg) => (format!("Connection failed: {msg}"), cx.theme().danger),
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
                    .child(div().text_sm().text_color(cx.theme().foreground).child("Name"))
                    .child(Input::new(&self.name_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(cx.theme().foreground).child("URI"))
                    .child(Input::new(&self.uri_state))
                    .child(div().text_xs().text_color(status_color).child(status_text)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(cx.theme().foreground).child("Password"))
                    .child(Input::new(&self.password_state).mask_toggle()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(cx.theme().foreground).child("Environment"))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(spacing::xs())
                            .child(no_environment)
                            .children(environments),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Explicit identity; never inferred from the URI."),
                    ),
            )
            .when(self.environment == Some(ConnectionEnvironment::Production), |content| {
                content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::sm())
                        .child(
                            Switch::new("simple-confirm-production-writes")
                                .checked(self.confirm_production_writes)
                                .small()
                                .on_click({
                                    let view = view.clone();
                                    move |checked, _window, cx| {
                                        view.update(cx, |this, cx| {
                                            this.confirm_production_writes = *checked;
                                            cx.notify();
                                        });
                                    }
                                }),
                        )
                        .child("Confirm Production writes and Forge"),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Switch::new("connection-reversible-history")
                            .checked(self.reversible_history)
                            .small()
                            .on_click({
                                let view = view.clone();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.reversible_history = *checked;
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
                            .child("Reversible history")
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Encrypt recovery data for document inserts, replacements, and deletions, including bounded bulk writes."),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Switch::new("connection-agent-shared")
                            .checked(self.agent_shared)
                            .small()
                            .on_click({
                                let view = view.clone();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.agent_shared = *checked;
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
                                    .text_color(cx.theme().foreground)
                                    .child("Share with agents"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().secondary_foreground)
                                    .child("Expose this connection to authenticated MCP clients"),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Switch::new("connection-protected")
                            .checked(self.protected)
                            .small()
                            .on_click({
                                let view = view.clone();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        if *checked {
                                            this.agent_shared = false;
                                        }
                                        this.protected = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child("Protected connection"),
            )
            .when(
                self.agent_shared
                    && (self.protected
                        || self.environment == Some(ConnectionEnvironment::Production)),
                |content| {
                    content.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().warning)
                            .child("This protected connection will be visible to agents. Mutations still require native approval."),
                    )
                },
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
                                    .text_color(cx.theme().foreground)
                                    .child("Read-only (safe mode)"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().secondary_foreground)
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
                            .label(save_label)
                            .disabled(!can_save)
                            .on_click({
                                let state = self.state.clone();
                                let name_state = self.name_state.clone();
                                let uri_state = self.uri_state.clone();
                                let password_state = self.password_state.clone();
                                let uri_secrets = self.uri_secrets.clone();
                                let environment = self.environment;
                                let confirm_production_writes = self.confirm_production_writes;
                                let read_only = self.read_only;
                                let agent_shared = self.agent_shared;
                                let protected = self.protected;
                                let reversible_history = self.reversible_history;
                                let existing = self.existing.clone();
                                move |_, window, cx| {
                                    let name_input = name_state.read(cx).value().to_string();
                                    let display_uri = uri_state.read(cx).value().to_string();

                                    let password =
                                        password_state.read(cx).value().trim().to_string();
                                    let mut secrets = uri_secrets.clone();
                                    secrets.password = (!password.is_empty()).then_some(password);
                                    let uri = inject_uri_secrets(
                                        &strip_uri_secrets(&display_uri),
                                        &secrets,
                                    );

                                    if validate_mongodb_uri(&uri).is_err() {
                                        return;
                                    }

                                    let name = if name_input.trim().is_empty() {
                                        extract_host_from_uri(&uri)
                                            .unwrap_or_else(|| "Untitled".to_string())
                                    } else {
                                        name_input.trim().to_string()
                                    };

                                    let existing_id =
                                        existing.as_ref().map(|connection| connection.id);
                                    let existing_for_save = existing.clone();
                                    let save_state = state.clone();
                                    let save = move |window: &mut Window, cx: &mut App| {
                                        let mut connection_id = None;
                                        save_state.update(cx, |state, cx| {
                                            if let Some(existing) = existing_for_save {
                                                connection_id = Some(existing.id);
                                                let connection = SavedConnection {
                                                    id: existing.id,
                                                    name,
                                                    color: existing.color,
                                                    environment,
                                                    confirm_production_writes,
                                                    uri,
                                                    last_connected: existing.last_connected,
                                                    read_only,
                                                    agent_shared,
                                                    protected,
                                                    reversible_history,
                                                    ssh: existing.ssh,
                                                    proxy: existing.proxy,
                                                    secret_id: existing.secret_id,
                                                };
                                                state.update_connection(connection, cx);
                                            } else {
                                                let mut connection =
                                                    SavedConnection::new(name, uri);
                                                connection.environment = environment;
                                                connection.confirm_production_writes =
                                                    confirm_production_writes;
                                                connection.read_only = read_only;
                                                connection.agent_shared = agent_shared;
                                                connection.protected = protected;
                                                connection.reversible_history = reversible_history;
                                                connection_id = Some(connection.id);
                                                state.add_connection(connection, cx);
                                            }
                                        });
                                        if let Some(id) = connection_id {
                                            save_state.update(cx, |state, cx| {
                                                state.connect_when_secrets_ready(id, cx);
                                            });
                                        }
                                        window.close_dialog(cx);
                                    };
                                    if let Some(connection_id) = existing_id {
                                        request_unsaved_action(
                                            state.clone(),
                                            UnsavedScope::Connection(connection_id),
                                            window,
                                            cx,
                                            save,
                                        );
                                    } else {
                                        save(window, cx);
                                    }
                                }
                            }),
                    ),
            )
    }
}
