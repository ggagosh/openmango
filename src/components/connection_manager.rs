use gpui::*;
use gpui::prelude::FluentBuilder as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputEvent, InputState, Position};
use gpui_component::scroll::ScrollableElement;
use gpui_component::switch::Switch;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{Icon, IconName, Sizable as _, WindowExt as _};
use uuid::Uuid;

use crate::components::{Button, open_confirm_dialog};
use crate::connection::get_connection_manager;
use crate::helpers::{extract_host_from_uri, validate_mongodb_uri};
use crate::models::SavedConnection;
use crate::state::{AppCommands, AppState};
use crate::theme::{borders, colors, sizing, spacing};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagerTab {
    General,
    Auth,
    Options,
    Tls,
    Pool,
    Advanced,
}

impl ManagerTab {
    fn all() -> [ManagerTab; 6] {
        [
            ManagerTab::General,
            ManagerTab::Auth,
            ManagerTab::Options,
            ManagerTab::Tls,
            ManagerTab::Pool,
            ManagerTab::Advanced,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            ManagerTab::General => "General",
            ManagerTab::Auth => "Auth",
            ManagerTab::Options => "Options",
            ManagerTab::Tls => "TLS",
            ManagerTab::Pool => "Pool & Timeouts",
            ManagerTab::Advanced => "Advanced",
        }
    }

    fn index(self) -> usize {
        Self::all().iter().position(|tab| *tab == self).unwrap_or(0)
    }

    fn from_index(index: usize) -> Self {
        Self::all().get(index).copied().unwrap_or(ManagerTab::General)
    }
}

#[derive(Clone, Debug)]
enum TestStatus {
    Idle,
    Testing,
    Success,
    Error(String),
}

struct ConnectionDraft {
    name_state: Entity<InputState>,
    uri_state: Entity<InputState>,
    username_state: Entity<InputState>,
    password_state: Entity<InputState>,
    app_name_state: Entity<InputState>,
    auth_source_state: Entity<InputState>,
    auth_mechanism_state: Entity<InputState>,
    auth_mechanism_props_state: Entity<InputState>,
    read_preference_state: Entity<InputState>,
    read_concern_state: Entity<InputState>,
    write_concern_state: Entity<InputState>,
    w_timeout_state: Entity<InputState>,
    connect_timeout_state: Entity<InputState>,
    server_selection_timeout_state: Entity<InputState>,
    max_pool_state: Entity<InputState>,
    min_pool_state: Entity<InputState>,
    heartbeat_frequency_state: Entity<InputState>,
    compressors_state: Entity<InputState>,
    zlib_level_state: Entity<InputState>,
    tls_ca_file_state: Entity<InputState>,
    tls_cert_key_file_state: Entity<InputState>,
    tls_cert_key_password_state: Entity<InputState>,
    read_only: bool,
    direct_connection: bool,
    tls: bool,
    tls_insecure: bool,
}

impl ConnectionDraft {
    fn new(window: &mut Window, cx: &mut Context<ConnectionManager>) -> Self {
        Self {
            name_state: cx.new(|cx| InputState::new(window, cx).placeholder("Connection name")),
            uri_state: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("mongodb://localhost:27017")
                    .default_value("mongodb://localhost:27017")
            }),
            username_state: cx.new(|cx| InputState::new(window, cx).placeholder("username")),
            password_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("password").masked(true)),
            app_name_state: cx.new(|cx| InputState::new(window, cx).placeholder("MyApp")),
            auth_source_state: cx.new(|cx| InputState::new(window, cx).placeholder("admin")),
            auth_mechanism_state: cx.new(|cx| InputState::new(window, cx)),
            auth_mechanism_props_state: cx.new(|cx| InputState::new(window, cx)),
            read_preference_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("primary")),
            read_concern_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("local")),
            write_concern_state: cx.new(|cx| InputState::new(window, cx).placeholder("majority")),
            w_timeout_state: cx.new(|cx| InputState::new(window, cx).placeholder("5000")),
            connect_timeout_state: cx.new(|cx| InputState::new(window, cx).placeholder("10000")),
            server_selection_timeout_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("30000")),
            max_pool_state: cx.new(|cx| InputState::new(window, cx).placeholder("100")),
            min_pool_state: cx.new(|cx| InputState::new(window, cx).placeholder("0")),
            heartbeat_frequency_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("10000")),
            compressors_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("zstd,zlib")),
            zlib_level_state: cx.new(|cx| InputState::new(window, cx).placeholder("6")),
            tls_ca_file_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("/path/ca.pem")),
            tls_cert_key_file_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("/path/cert.pem")),
            tls_cert_key_password_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("password").masked(true)),
            read_only: false,
            direct_connection: false,
            tls: false,
            tls_insecure: false,
        }
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<ConnectionManager>) {
        self.name_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.uri_state.update(cx, |state, cx| {
            state.set_value("mongodb://localhost:27017".to_string(), window, cx)
        });
        self.username_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.password_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.app_name_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.auth_source_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.auth_mechanism_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.auth_mechanism_props_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.read_preference_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.read_concern_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.write_concern_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.w_timeout_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.connect_timeout_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.server_selection_timeout_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.max_pool_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.min_pool_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.heartbeat_frequency_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.compressors_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.zlib_level_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.tls_ca_file_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.tls_cert_key_file_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.tls_cert_key_password_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.read_only = false;
        self.direct_connection = false;
        self.tls = false;
        self.tls_insecure = false;
    }
}

pub struct ConnectionManager {
    state: Entity<AppState>,
    selected_id: Option<Uuid>,
    draft: ConnectionDraft,
    active_tab: ManagerTab,
    creating_new: bool,
    status: TestStatus,
    last_tested_uri: Option<String>,
    pending_test_uri: Option<String>,
    parse_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectionManager {
    pub fn open(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
        Self::open_with_selected(state, None, window, cx);
    }

    pub fn open_selected(
        state: Entity<AppState>,
        connection_id: Uuid,
        window: &mut Window,
        cx: &mut App,
    ) {
        Self::open_with_selected(state, Some(connection_id), window, cx);
    }

    fn open_with_selected(
        state: Entity<AppState>,
        selected_id: Option<Uuid>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view =
            cx.new(|cx| ConnectionManager::new(state.clone(), selected_id, window, cx));
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog
                .title("Connection Manager")
                .w(px(1040.0))
                .margin_top(px(20.0))
                .min_h(px(620.0))
                .child(dialog_view.clone())
        });
    }

    pub fn new(
        state: Entity<AppState>,
        selected_id: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = ConnectionDraft::new(window, cx);
        let mut subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];

        let uri_state = draft.uri_state.clone();
        subscriptions.push(cx.subscribe_in(
            &uri_state,
            window,
            move |view, _state, event, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.status = TestStatus::Idle;
                    view.last_tested_uri = None;
                    view.pending_test_uri = None;
                    view.parse_error = None;
                    cx.notify();
                }
            },
        ));

        let mut view = Self {
            state,
            selected_id,
            draft,
            active_tab: ManagerTab::General,
            creating_new: false,
            status: TestStatus::Idle,
            last_tested_uri: None,
            pending_test_uri: None,
            parse_error: None,
            _subscriptions: subscriptions,
        };

        if let Some(connection_id) = selected_id
            && let Some(connection) = view
                .state
                .read(cx)
                .connections
                .iter()
                .find(|conn| conn.id == connection_id)
                .cloned()
        {
            view.load_connection(Some(connection), window, cx);
        }

        view
    }

    fn load_connection(
        &mut self,
        connection: Option<SavedConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status = TestStatus::Idle;
        self.last_tested_uri = None;
        self.pending_test_uri = None;
        self.parse_error = None;
        self.creating_new = connection.is_none();

        if let Some(connection) = connection {
            self.selected_id = Some(connection.id);
            self.draft.name_state.update(cx, |state, cx| {
                state.set_value(connection.name.clone(), window, cx)
            });
            self.draft
                .uri_state
                .update(cx, |state, cx| {
                    state.set_value(connection.uri.clone(), window, cx);
                    state.set_cursor_position(Position::new(0, 0), window, cx);
                });
            self.draft.read_only = connection.read_only;
            self.import_from_uri(window, cx);
        } else {
            self.selected_id = None;
            self.draft.reset(window, cx);
        }
    }

    fn import_from_uri(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let uri = self.draft.uri_state.read(cx).value().to_string();
        match parse_uri(&uri) {
            Ok(parts) => {
                let (user, password) = parts.userinfo();
                self.draft.username_state.update(cx, |state, cx| {
                    state.set_value(user.unwrap_or_default(), window, cx)
                });
                self.draft.password_state.update(cx, |state, cx| {
                    state.set_value(password.unwrap_or_default(), window, cx)
                });
                self.draft.app_name_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("appName"), window, cx)
                });
                self.draft.auth_source_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("authSource"), window, cx)
                });
                self.draft.auth_mechanism_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("authMechanism"), window, cx)
                });
                self.draft.auth_mechanism_props_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("authMechanismProperties"), window, cx)
                });
                self.draft.read_preference_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("readPreference"), window, cx)
                });
                self.draft.read_concern_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("readConcernLevel"), window, cx)
                });
                self.draft.write_concern_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("w"), window, cx)
                });
                self.draft.w_timeout_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("wTimeoutMS"), window, cx)
                });
                self.draft.connect_timeout_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("connectTimeoutMS"), window, cx)
                });
                self.draft.server_selection_timeout_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("serverSelectionTimeoutMS"), window, cx)
                });
                self.draft.max_pool_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("maxPoolSize"), window, cx)
                });
                self.draft.min_pool_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("minPoolSize"), window, cx)
                });
                self.draft.heartbeat_frequency_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("heartbeatFrequencyMS"), window, cx)
                });
                self.draft.compressors_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("compressors"), window, cx)
                });
                self.draft.zlib_level_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("zlibCompressionLevel"), window, cx)
                });
                self.draft.tls_ca_file_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("tlsCAFile"), window, cx)
                });
                self.draft.tls_cert_key_file_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("tlsCertificateKeyFile"), window, cx)
                });
                self.draft.tls_cert_key_password_state.update(cx, |state, cx| {
                    state.set_value(parts.get_query("tlsCertificateKeyFilePassword"), window, cx)
                });

                self.draft.direct_connection = parse_bool(parts.get_query("directConnection"));
                self.draft.tls = parse_bool(parts.get_query("tls"));
                self.draft.tls_insecure = parse_bool(parts.get_query("tlsInsecure"));
                self.parse_error = None;
            }
            Err(err) => {
                self.parse_error = Some(err);
            }
        }
        cx.notify();
    }

    fn update_uri_from_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let uri = self.draft.uri_state.read(cx).value().to_string();
        let mut parts = match parse_uri(&uri) {
            Ok(parts) => parts,
            Err(err) => {
                self.parse_error = Some(err);
                cx.notify();
                return false;
            }
        };

        let username = self.draft.username_state.read(cx).value().to_string();
        let password = self.draft.password_state.read(cx).value().to_string();
        parts.set_userinfo(
            if username.trim().is_empty() { None } else { Some(username.trim().to_string()) },
            if password.trim().is_empty() { None } else { Some(password.trim().to_string()) },
        );

        parts.set_query("appName", value_or_none(&self.draft.app_name_state, cx));
        parts.set_query("authSource", value_or_none(&self.draft.auth_source_state, cx));
        parts.set_query(
            "authMechanism",
            value_or_none(&self.draft.auth_mechanism_state, cx),
        );
        parts.set_query(
            "authMechanismProperties",
            value_or_none(&self.draft.auth_mechanism_props_state, cx),
        );
        parts.set_query(
            "readPreference",
            value_or_none(&self.draft.read_preference_state, cx),
        );
        parts.set_query(
            "readConcernLevel",
            value_or_none(&self.draft.read_concern_state, cx),
        );
        parts.set_query("w", value_or_none(&self.draft.write_concern_state, cx));
        parts.set_query("wTimeoutMS", value_or_none(&self.draft.w_timeout_state, cx));
        parts.set_query(
            "connectTimeoutMS",
            value_or_none(&self.draft.connect_timeout_state, cx),
        );
        parts.set_query(
            "serverSelectionTimeoutMS",
            value_or_none(&self.draft.server_selection_timeout_state, cx),
        );
        parts.set_query("maxPoolSize", value_or_none(&self.draft.max_pool_state, cx));
        parts.set_query("minPoolSize", value_or_none(&self.draft.min_pool_state, cx));
        parts.set_query(
            "heartbeatFrequencyMS",
            value_or_none(&self.draft.heartbeat_frequency_state, cx),
        );
        parts.set_query("compressors", value_or_none(&self.draft.compressors_state, cx));
        parts.set_query(
            "zlibCompressionLevel",
            value_or_none(&self.draft.zlib_level_state, cx),
        );
        parts.set_query("tlsCAFile", value_or_none(&self.draft.tls_ca_file_state, cx));
        parts.set_query(
            "tlsCertificateKeyFile",
            value_or_none(&self.draft.tls_cert_key_file_state, cx),
        );
        parts.set_query(
            "tlsCertificateKeyFilePassword",
            value_or_none(&self.draft.tls_cert_key_password_state, cx),
        );
        parts.set_query(
            "directConnection",
            bool_to_query(self.draft.direct_connection),
        );
        parts.set_query("tls", bool_to_query(self.draft.tls));
        parts.set_query("tlsInsecure", bool_to_query(self.draft.tls_insecure));

        let updated = parts.to_uri();
        self.draft.uri_state.update(cx, |state, cx| {
            state.set_value(updated, window, cx);
            state.set_cursor_position(Position::new(0, 0), window, cx);
        });
        self.parse_error = None;
        true
    }

    fn start_test(view: Entity<ConnectionManager>, cx: &mut App) {
        let uri = view.read(cx).draft.uri_state.read(cx).value().to_string();
        if let Err(err) = validate_mongodb_uri(&uri) {
            view.update(cx, |this, cx| {
                this.status = TestStatus::Error(err.to_string());
                this.last_tested_uri = None;
                this.pending_test_uri = None;
                cx.notify();
            });
            return;
        }

        view.update(cx, |this, cx| {
            this.status = TestStatus::Testing;
            this.pending_test_uri = Some(uri.clone());
            this.last_tested_uri = None;
            cx.notify();
        });

        let task = cx.background_spawn({
            let uri = uri.clone();
            async move {
                let manager = get_connection_manager();
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
                        let current_uri = this.draft.uri_state.read(cx).value().to_string();
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

    fn save_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Uuid> {
        if !self.update_uri_from_fields(window, cx) {
            return None;
        }
        let uri = self.draft.uri_state.read(cx).value().to_string();
        if validate_mongodb_uri(&uri).is_err() {
            self.parse_error = Some("Invalid MongoDB URI".to_string());
            return None;
        }

        let name_input = self.draft.name_state.read(cx).value().to_string();
        let name = if name_input.trim().is_empty() {
            extract_host_from_uri(&uri).unwrap_or_else(|| "Untitled".to_string())
        } else {
            name_input.trim().to_string()
        };

        let read_only = self.draft.read_only;
        let selected_id = self.selected_id;
        let mut saved_connection: Option<SavedConnection> = None;
        self.state.update(cx, |state, cx| {
            if let Some(existing_id) = selected_id {
                if let Some(existing) = state.connections.iter().find(|conn| conn.id == existing_id)
                {
                    let connection = SavedConnection {
                        id: existing_id,
                        name,
                        uri,
                        last_connected: existing.last_connected,
                        read_only,
                    };
                    state.update_connection(connection.clone(), cx);
                    saved_connection = Some(connection);
                }
            } else {
                let mut connection = SavedConnection::new(name, uri);
                connection.read_only = read_only;
                state.add_connection(connection.clone(), cx);
                saved_connection = Some(connection);
            }
        });

        if let Some(saved) = saved_connection {
            let saved_id = saved.id;
            self.load_connection(Some(saved), window, cx);
            return Some(saved_id);
        }
        None
    }

    fn remove_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection_id) = self.selected_id else {
            return;
        };
        let state = self.state.clone();
        open_confirm_dialog(
            window,
            cx,
            "Remove connection",
            "Remove this connection? This cannot be undone.".to_string(),
            "Remove",
            true,
            move |_window, cx| {
                state.update(cx, |state, cx| {
                    state.remove_connection(connection_id, cx);
                });
            },
        );
    }
}

impl Render for ConnectionManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connections = self.state.read(cx).connections.clone();
        let selected_exists = self
            .selected_id
            .is_some_and(|id| connections.iter().any(|conn| conn.id == id));
        if !selected_exists && !connections.is_empty() && !self.creating_new {
            let next = connections.first().cloned();
            self.load_connection(next, window, cx);
        }

        let active_tab = self.active_tab;
        let status = self.status.clone();
        let is_testing = matches!(status, TestStatus::Testing);
        let status_text = match &status {
            TestStatus::Idle => "Test connection to verify settings".to_string(),
            TestStatus::Testing => "Testing connection...".to_string(),
            TestStatus::Success => "Connection OK".to_string(),
            TestStatus::Error(err) => format!("Connection failed: {err}"),
        };
        let status_color = match status {
            TestStatus::Success => colors::accent(),
            TestStatus::Error(_) => colors::status_error(),
            _ => colors::text_muted(),
        };

        let selected_id = self.selected_id;
        let active_id = self.state.read(cx).conn.active.as_ref().map(|conn| conn.config.id);
        let is_active_selection = selected_id.is_some_and(|id| Some(id) == active_id);

        let list = div()
            .flex()
            .flex_col()
            .w(px(320.0))
            .min_w(px(320.0))
            .h_full()
            .border_r_1()
            .border_color(colors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(spacing::md())
                    .h(sizing::header_height())
                    .border_b_1()
                    .border_color(colors::border())
                    .child(div().text_xs().text_color(colors::text_secondary()).child("Connections"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                Button::new("new-connection")
                                    .compact()
                                    .icon(Icon::new(IconName::Plus).xsmall())
                                    .on_click({
                                        let view = cx.entity();
                                        move |_, window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.load_connection(None, window, cx);
                                                this.active_tab = ManagerTab::General;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(
                                Button::new("remove-connection")
                                    .compact()
                                    .icon(Icon::new(IconName::Delete).xsmall())
                                    .disabled(selected_id.is_none())
                                    .on_click({
                                        let view = cx.entity();
                                        move |_, window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.remove_connection(window, cx);
                                            });
                                        }
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .child(if connections.is_empty() {
                        div()
                            .p(spacing::md())
                            .text_sm()
                            .text_color(colors::text_muted())
                            .child("No connections")
                            .into_any_element()
                    } else {
                        let view = cx.entity();
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .p(spacing::xs())
                            .child(
                                div().flex().flex_col().children(connections.into_iter().map(
                                    move |conn| {
                                        let is_selected = Some(conn.id) == selected_id;
                                        let host = extract_host_from_uri(&conn.uri)
                                            .unwrap_or_else(|| "Unknown host".to_string());
                                        let last_connected = conn
                                            .last_connected
                                            .map(|dt| dt.format("%Y-%m-%d").to_string())
                                            .unwrap_or_else(|| "Never".to_string());
                                        let read_only = conn.read_only;
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(4.0))
                                            .px(spacing::md())
                                            .py(spacing::sm())
                                            .cursor_pointer()
                                            .rounded(borders::radius_sm())
                                            .when(is_selected, |s| {
                                                s.bg(colors::bg_hover())
                                                    .border_1()
                                                    .border_color(colors::border())
                                            })
                                            .hover(|s| s.bg(colors::bg_hover()))
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(colors::text_primary())
                                                            .child(conn.name.clone()),
                                                    )
                                                    .when(read_only, |s| {
                                                        s.child(
                                                            div()
                                                                .px(spacing::xs())
                                                                .py(px(1.0))
                                                                .rounded(borders::radius_sm())
                                                                .bg(colors::status_warning())
                                                                .text_xs()
                                                                .text_color(colors::bg_header())
                                                                .child("RO"),
                                                        )
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(colors::text_secondary())
                                                    .child(host),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(colors::text_muted())
                                                    .child(format!("Last: {last_connected}")),
                                            )
                                            .on_mouse_down(MouseButton::Left, {
                                                let view = view.clone();
                                                let conn = conn.clone();
                                                move |_, window, cx| {
                                                    view.update(cx, |this, cx| {
                                                        this.load_connection(Some(conn.clone()), window, cx);
                                                        cx.notify();
                                                    });
                                                }
                                            })
                                    },
                                )),
                            )
                            .into_any_element()
                    }),
            );

        let tab_bar = TabBar::new("connection-manager-tabs")
            .selected_index(active_tab.index())
            .on_click({
                let view = cx.entity();
                move |index, _window, cx| {
                    let index = *index;
                    view.update(cx, |this, cx| {
                        this.active_tab = ManagerTab::from_index(index);
                        cx.notify();
                    });
                }
            })
            .children(ManagerTab::all().into_iter().map(|tab| Tab::new().label(tab.label())));

        let parse_error = self.parse_error.clone();
        let actions = div()
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
                        let view = cx.entity();
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
                        let view = cx.entity();
                        let state = self.state.clone();
                        move |_, window, cx| {
                            let mut reconnect_id = None;
                            view.update(cx, |this, cx| {
                                let active_id =
                                    this.state.read(cx).conn.active.as_ref().map(|conn| conn.config.id);
                                let saved_id = this.save_connection(window, cx);
                                if saved_id.is_some_and(|id| Some(id) == active_id) {
                                    reconnect_id = saved_id;
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
            ));
        let editor = div()
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
                                ManagerTab::General => self.render_general_tab(parse_error, window, cx),
                                ManagerTab::Auth => self.render_auth_tab(window, cx),
                                ManagerTab::Options => self.render_options_tab(window, cx),
                                ManagerTab::Tls => self.render_tls_tab(window, cx),
                                ManagerTab::Pool => self.render_pool_tab(window, cx),
                                ManagerTab::Advanced => self.render_advanced_tab(window, cx),
                            }),
                    )
                    .child(
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
                            .child(actions),
                    ),
            );

        div()
            .flex()
            .flex_row()
            .flex_1()
            .h_full()
            .w_full()
            .child(list)
            .child(editor)
    }
}

impl ConnectionManager {
    fn render_general_tab(
        &mut self,
        parse_error: Option<String>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("Name"))
                    .child(Input::new(&self.draft.name_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("URI"))
                    .child(
                        Input::new(&self.draft.uri_state)
                            .font_family(crate::theme::fonts::mono())
                            .w_full(),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(
                                Button::new("import-uri")
                                    .compact()
                                    .label("Import from URI")
                                    .on_click({
                                        let view = view.clone();
                                        move |_, window, cx| {
                                            ConnectionManager::import_uri_from_clipboard_or_dialog(
                                                view.clone(),
                                                window,
                                                cx,
                                            );
                                        }
                                    }),
                            )
                            .child(
                                Button::new("apply-uri")
                                    .compact()
                                    .label("Update URI")
                                    .on_click({
                                        let view = view.clone();
                                        move |_, window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.update_uri_from_fields(window, cx);
                                            });
                                        }
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors::text_muted())
                            .child("Keep URI as the source of truth; fields below sync on update."),
                    )
                    .child(if let Some(err) = parse_error {
                        div()
                            .text_xs()
                            .text_color(colors::text_error())
                            .child(err)
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Switch::new("connection-read-only")
                            .checked(self.draft.read_only)
                            .small()
                            .on_click({
                                let view = view.clone();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.draft.read_only = *checked;
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
                                    .child("Block inserts, updates, deletes, drops, and index changes"),
                            ),
                    ),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(spacing::md())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(div().text_sm().text_color(colors::text_primary()).child("Username"))
                            .child(Input::new(&self.draft.username_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(div().text_sm().text_color(colors::text_primary()).child("Password"))
                            .child(Input::new(&self.draft.password_state).mask_toggle()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("App name"))
                    .child(Input::new(&self.draft.app_name_state)),
            )
            .into_any_element()
    }

    fn import_uri_from_clipboard_or_dialog(
        view: Entity<ConnectionManager>,
        window: &mut Window,
        cx: &mut App,
    ) {
        ConnectionManager::open_import_uri_dialog(view, window, cx);
    }

    fn open_import_uri_dialog(
        view: Entity<ConnectionManager>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("mongodb+srv://user:pass@cluster0.example.mongodb.net")
        });

        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let value = text.lines().next().unwrap_or("").trim().to_string();
            if !value.is_empty() {
                input_state.update(cx, |state, cx| {
                    state.set_value(value, window, cx);
                    state.set_cursor_position(Position::new(0, 0), window, cx);
                });
            }
        }

        window.open_dialog(cx, move |dialog: Dialog, window: &mut Window, cx: &mut App| {
            input_state.update(cx, |state, cx| {
                state.focus(window, cx);
            });
            dialog
                .title("Import Connection URI")
                .w(px(560.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::sm())
                        .p(spacing::md())
                        .child(
                            Input::new(&input_state)
                                .font_family(crate::theme::fonts::mono())
                                .w_full(),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(spacing::sm())
                                .child(
                                    Button::new("paste-uri")
                                        .compact()
                                        .label("Paste from Clipboard")
                                        .on_click({
                                            let input_state = input_state.clone();
                                            move |_, window, cx| {
                                                if let Some(text) =
                                                    cx.read_from_clipboard().and_then(|item| item.text())
                                                {
                                                    let value = text.lines().next().unwrap_or("").trim().to_string();
                                                    if value.is_empty() {
                                                        return;
                                                    }
                                                    input_state.update(cx, |state, cx| {
                                                        state.set_value(value, window, cx);
                                                        state.set_cursor_position(Position::new(0, 0), window, cx);
                                                    });
                                                }
                                            }
                                        }),
                                )
                                .child(
                                    Button::new("clear-uri")
                                        .compact()
                                        .ghost()
                                        .label("Clear")
                                        .on_click({
                                            let input_state = input_state.clone();
                                            move |_, window, cx| {
                                                input_state.update(cx, |state, cx| {
                                                    state.set_value(String::new(), window, cx);
                                                });
                                            }
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors::text_muted())
                                .child("Paste a mongodb:// or mongodb+srv:// URI to import settings."),
                        ),
                )
                .footer({
                    let view = view.clone();
                    let input_state = input_state.clone();
                    move |_ok, _cancel, _window, _cx| {
                        let view = view.clone();
                        let input_state = input_state.clone();
                        vec![
                            Button::new("cancel-import-uri")
                                .label("Cancel")
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                })
                                .into_any_element(),
                            Button::new("confirm-import-uri")
                                .primary()
                                .label("Import")
                                .on_click({
                                    let view = view.clone();
                                    let input_state = input_state.clone();
                                    move |_, window, cx| {
                                        let raw = input_state.read(cx).value().to_string();
                                        let value = raw.lines().next().unwrap_or("").trim().to_string();
                                        if value.is_empty() {
                                            window.close_dialog(cx);
                                            return;
                                        }
                                        view.update(cx, |this, cx| {
                                            this.draft.uri_state.update(cx, |state, cx| {
                                                state.set_value(value.clone(), window, cx);
                                                state.set_cursor_position(Position::new(0, 0), window, cx);
                                            });
                                            this.import_from_uri(window, cx);
                                        });
                                        window.close_dialog(cx);
                                    }
                                })
                                .into_any_element(),
                        ]
                    }
                })
        });
    }

    fn render_auth_tab(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("Auth source"))
                    .child(Input::new(&self.draft.auth_source_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("Auth mechanism"))
                    .child(Input::new(&self.draft.auth_mechanism_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors::text_primary())
                            .child("Auth mechanism properties"),
                    )
                    .child(Input::new(&self.draft.auth_mechanism_props_state)),
            )
            .into_any_element()
    }

    fn render_options_tab(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Switch::new("direct-connection")
                            .checked(self.draft.direct_connection)
                            .small()
                            .on_click({
                                let view = _cx.entity();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.draft.direct_connection = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors::text_primary())
                            .child("Direct connection"),
                    ),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(spacing::md())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Read preference"),
                            )
                            .child(Input::new(&self.draft.read_preference_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Read concern level"),
                            )
                            .child(Input::new(&self.draft.read_concern_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(div().text_sm().text_color(colors::text_primary()).child("Write concern (w)"))
                            .child(Input::new(&self.draft.write_concern_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(div().text_sm().text_color(colors::text_primary()).child("wTimeoutMS"))
                            .child(Input::new(&self.draft.w_timeout_state)),
                    ),
            )
            .into_any_element()
    }

    fn render_tls_tab(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Switch::new("tls-enabled")
                            .checked(self.draft.tls)
                            .small()
                            .on_click({
                                let view = _cx.entity();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.draft.tls = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(div().text_sm().text_color(colors::text_primary()).child("TLS enabled")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Switch::new("tls-insecure")
                            .checked(self.draft.tls_insecure)
                            .small()
                            .on_click({
                                let view = _cx.entity();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.draft.tls_insecure = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(div().text_sm().text_color(colors::text_primary()).child("TLS insecure")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("TLS CA file"))
                    .child(Input::new(&self.draft.tls_ca_file_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors::text_primary())
                            .child("TLS certificate key file"),
                    )
                    .child(Input::new(&self.draft.tls_cert_key_file_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors::text_primary())
                            .child("TLS certificate key password"),
                    )
                    .child(Input::new(&self.draft.tls_cert_key_password_state).mask_toggle()),
            )
            .into_any_element()
    }

    fn render_pool_tab(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(spacing::md())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(div().text_sm().text_color(colors::text_primary()).child("Connect timeout (ms)"))
                            .child(Input::new(&self.draft.connect_timeout_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Server selection timeout (ms)"),
                            )
                            .child(Input::new(&self.draft.server_selection_timeout_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(div().text_sm().text_color(colors::text_primary()).child("Max pool size"))
                            .child(Input::new(&self.draft.max_pool_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(div().text_sm().text_color(colors::text_primary()).child("Min pool size"))
                            .child(Input::new(&self.draft.min_pool_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Heartbeat frequency (ms)"),
                            )
                            .child(Input::new(&self.draft.heartbeat_frequency_state)),
                    ),
            )
            .into_any_element()
    }

    fn render_advanced_tab(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("Compressors"))
                    .child(Input::new(&self.draft.compressors_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors::text_primary())
                            .child("Zlib compression level"),
                    )
                    .child(Input::new(&self.draft.zlib_level_state)),
            )
            .into_any_element()
    }
}

#[derive(Clone, Debug)]
struct UriParts {
    scheme: String,
    user: Option<String>,
    password: Option<String>,
    hosts: String,
    database: Option<String>,
    query: Vec<(String, String)>,
}

impl UriParts {
    fn get_query(&self, key: &str) -> String {
        self.query
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }

    fn set_query(&mut self, key: &str, value: Option<String>) {
        self.query.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
        if let Some(value) = value {
            self.query.push((key.to_string(), value));
        }
    }

    fn set_userinfo(&mut self, user: Option<String>, password: Option<String>) {
        self.user = user;
        self.password = password;
    }

    fn userinfo(&self) -> (Option<String>, Option<String>) {
        (self.user.clone(), self.password.clone())
    }

    fn to_uri(&self) -> String {
        let mut output = format!("{}://", self.scheme);
        if let Some(user) = &self.user {
            output.push_str(user);
            if let Some(password) = &self.password {
                output.push(':');
                output.push_str(password);
            }
            output.push('@');
        }
        output.push_str(&self.hosts);
        if let Some(database) = &self.database
            && !database.is_empty()
        {
            output.push('/');
            output.push_str(database);
        }
        if !self.query.is_empty() {
            output.push('?');
            for (idx, (key, value)) in self.query.iter().enumerate() {
                output.push_str(key);
                output.push('=');
                output.push_str(value);
                if idx + 1 < self.query.len() {
                    output.push('&');
                }
            }
        }
        output
    }
}

fn parse_uri(input: &str) -> Result<UriParts, String> {
    let raw = input.trim();
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| "URI must include a scheme (mongodb:// or mongodb+srv://)".to_string())?;
    if rest.is_empty() {
        return Err("URI is missing host".to_string());
    }
    let (base, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host_part, database) = base.split_once('/').unwrap_or((base, ""));
    if host_part.is_empty() {
        return Err("URI is missing host".to_string());
    }

    let (userinfo, hosts) = host_part
        .rsplit_once('@')
        .map(|(u, h)| (Some(u), h))
        .unwrap_or((None, host_part));

    if hosts.trim().is_empty() {
        return Err("URI is missing host".to_string());
    }

    let (user, password) = if let Some(userinfo) = userinfo {
        if let Some((user, pass)) = userinfo.split_once(':') {
            (Some(user.to_string()), Some(pass.to_string()))
        } else {
            (Some(userinfo.to_string()), None)
        }
    } else {
        (None, None)
    };

    let database = if database.trim().is_empty() {
        None
    } else {
        Some(database.to_string())
    };

    let mut query_pairs = Vec::new();
    if !query.trim().is_empty() {
        for pair in query.split('&') {
            if pair.trim().is_empty() {
                continue;
            }
            if let Some((key, value)) = pair.split_once('=') {
                query_pairs.push((key.to_string(), value.to_string()));
            } else {
                query_pairs.push((pair.to_string(), String::new()));
            }
        }
    }

    Ok(UriParts {
        scheme: scheme.to_string(),
        user,
        password,
        hosts: hosts.to_string(),
        database,
        query: query_pairs,
    })
}

fn parse_bool(value: String) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on")
}

fn bool_to_query(value: bool) -> Option<String> {
    if value {
        Some("true".to_string())
    } else {
        None
    }
}

fn value_or_none(state: &Entity<InputState>, cx: &mut Context<ConnectionManager>) -> Option<String> {
    let value = state.read(cx).value().to_string();
    if value.trim().is_empty() {
        None
    } else {
        Some(value.trim().to_string())
    }
}
