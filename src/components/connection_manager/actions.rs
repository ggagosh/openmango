use gpui::{
    App, AppContext as _, Context, Entity, IntoElement as _, ParentElement as _, Styled as _,
    Window, div, px,
};
use gpui_component::ActiveTheme as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputState, Position};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::components::{Button, cancel_button, open_confirm_dialog};
use crate::helpers::{
    REDACTED_PASSWORD, extract_host_from_uri, extract_uri_password, inject_uri_password,
    redact_uri_password, validate_mongodb_uri,
};
use crate::models::{ProxyConfig, ProxyKind, SavedConnection, SshAuth, SshConfig};
use crate::state::AppState;
use crate::theme::spacing;

use super::uri::{bool_to_query, parse_bool, parse_uri, value_or_none};
use super::{ConnectionManager, TestStatus};

const TEST_CONNECTION_TIMEOUT_SECS: u64 = 30;

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

    pub(super) fn open_with_selected(
        state: Entity<AppState>,
        selected_id: Option<Uuid>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view =
            cx.new(|cx| ConnectionManager::new(state.clone(), selected_id, window, cx));
        window.open_dialog(cx, move |dialog: Dialog, window: &mut Window, _cx: &mut App| {
            let vp = window.viewport_size();
            let w = (vp.width - px(200.0)).max(px(800.0)).min(px(1200.0));
            dialog
                .title("Connection Manager")
                .overlay_closable(false)
                .w(w)
                .margin_top(px(40.0))
                .h(vp.height - px(200.0))
                .child(dialog_view.clone())
        });
    }

    pub(super) fn load_connection(
        &mut self,
        connection: Option<SavedConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status = TestStatus::Idle;
        self.last_tested_uri = None;
        self.pending_test_uri = None;
        self.testing_step = None;
        self.parse_error = None;
        self.creating_new = connection.is_none();

        if let Some(connection) = connection {
            self.selected_id = Some(connection.id);
            self.draft
                .name_state
                .update(cx, |state, cx| state.set_value(connection.name.clone(), window, cx));
            self.draft.uri_state.update(cx, |state, cx| {
                state.set_value(connection.uri.clone(), window, cx);
                state.set_cursor_position(Position::new(0, 0), window, cx);
            });
            self.draft.read_only = connection.read_only;
            self.load_transport_settings(&connection, window, cx);
            self.import_from_uri(window, cx);
        } else {
            self.selected_id = None;
            self.draft.reset(window, cx);
        }
    }

    fn load_transport_settings(
        &mut self,
        connection: &SavedConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ssh) = &connection.ssh {
            self.draft.ssh_enabled = ssh.enabled;
            self.draft.ssh_use_identity_file = matches!(ssh.auth, SshAuth::IdentityFile);
            self.draft.ssh_strict_host_key_checking = ssh.strict_host_key_checking;
            self.draft
                .ssh_host_state
                .update(cx, |state, cx| state.set_value(ssh.host.clone(), window, cx));
            self.draft
                .ssh_port_state
                .update(cx, |state, cx| state.set_value(ssh.port.to_string(), window, cx));
            self.draft
                .ssh_username_state
                .update(cx, |state, cx| state.set_value(ssh.username.clone(), window, cx));
            self.draft.ssh_password_state.update(cx, |state, cx| {
                state.set_value(ssh.password.clone().unwrap_or_default(), window, cx)
            });
            self.draft.ssh_identity_file_state.update(cx, |state, cx| {
                state.set_value(ssh.identity_file.clone().unwrap_or_default(), window, cx)
            });
            self.draft.ssh_identity_passphrase_state.update(cx, |state, cx| {
                state.set_value(ssh.identity_passphrase.clone().unwrap_or_default(), window, cx)
            });
            self.draft
                .ssh_local_bind_host_state
                .update(cx, |state, cx| state.set_value(ssh.local_bind_host.clone(), window, cx));
        } else {
            self.draft.ssh_enabled = false;
            self.draft.ssh_use_identity_file = false;
            self.draft.ssh_strict_host_key_checking = true;
            self.draft
                .ssh_host_state
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
            self.draft
                .ssh_port_state
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
            self.draft
                .ssh_username_state
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
            self.draft
                .ssh_password_state
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
            self.draft
                .ssh_identity_file_state
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
            self.draft
                .ssh_identity_passphrase_state
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
            self.draft
                .ssh_local_bind_host_state
                .update(cx, |state, cx| state.set_value("127.0.0.1".to_string(), window, cx));
        }

        if let Some(proxy) = &connection.proxy {
            self.draft.proxy_enabled = proxy.enabled;
            self.draft
                .proxy_host_state
                .update(cx, |state, cx| state.set_value(proxy.host.clone(), window, cx));
            self.draft
                .proxy_port_state
                .update(cx, |state, cx| state.set_value(proxy.port.to_string(), window, cx));
            self.draft.proxy_username_state.update(cx, |state, cx| {
                state.set_value(proxy.username.clone().unwrap_or_default(), window, cx)
            });
            self.draft.proxy_password_state.update(cx, |state, cx| {
                state.set_value(proxy.password.clone().unwrap_or_default(), window, cx)
            });
        } else {
            self.draft.proxy_enabled = false;
            self.draft
                .proxy_host_state
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
            self.draft
                .proxy_port_state
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
            self.draft
                .proxy_username_state
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
            self.draft
                .proxy_password_state
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        }
    }

    fn build_transport_settings(
        &self,
        cx: &App,
    ) -> std::result::Result<(Option<SshConfig>, Option<ProxyConfig>), String> {
        let read_trim = |state: &Entity<InputState>| state.read(cx).value().trim().to_string();
        let read_opt = |state: &Entity<InputState>| {
            let value = state.read(cx).value().trim().to_string();
            if value.is_empty() { None } else { Some(value) }
        };

        let ssh = if self.draft.ssh_enabled {
            let host = read_trim(&self.draft.ssh_host_state);
            if host.is_empty() {
                return Err("SSH host is required".to_string());
            }
            let username = read_trim(&self.draft.ssh_username_state);
            if username.is_empty() {
                return Err("SSH username is required".to_string());
            }

            let port = parse_u16_or_default(read_trim(&self.draft.ssh_port_state), 22, "SSH port")?;
            let local_bind_host = {
                let value = read_trim(&self.draft.ssh_local_bind_host_state);
                if value.is_empty() { "127.0.0.1".to_string() } else { value }
            };

            let auth = if self.draft.ssh_use_identity_file {
                SshAuth::IdentityFile
            } else {
                SshAuth::Password
            };

            let password = read_opt(&self.draft.ssh_password_state);
            let identity_file = read_opt(&self.draft.ssh_identity_file_state);
            let identity_passphrase = read_opt(&self.draft.ssh_identity_passphrase_state);

            match auth {
                SshAuth::Password if password.is_none() => {
                    return Err("SSH password is required for password auth".to_string());
                }
                SshAuth::IdentityFile if identity_file.is_none() => {
                    return Err("SSH identity file is required for identity-file auth".to_string());
                }
                _ => {}
            }

            Some(SshConfig {
                enabled: true,
                host,
                port,
                username,
                auth,
                password,
                identity_file,
                identity_passphrase,
                strict_host_key_checking: self.draft.ssh_strict_host_key_checking,
                local_bind_host,
            })
        } else {
            None
        };

        let proxy = if self.draft.proxy_enabled {
            let host = read_trim(&self.draft.proxy_host_state);
            if host.is_empty() {
                return Err("SOCKS5 proxy host is required".to_string());
            }
            let port =
                parse_u16_or_default(read_trim(&self.draft.proxy_port_state), 1080, "Proxy port")?;

            Some(ProxyConfig {
                enabled: true,
                kind: ProxyKind::Socks5,
                host,
                port,
                username: read_opt(&self.draft.proxy_username_state),
                password: read_opt(&self.draft.proxy_password_state),
            })
        } else {
            None
        };

        if ssh.as_ref().is_some_and(|cfg| cfg.enabled)
            && proxy.as_ref().is_some_and(|cfg| cfg.enabled)
        {
            return Err("SSH tunnel and SOCKS5 proxy cannot be enabled together yet".to_string());
        }

        Ok((ssh, proxy))
    }

    pub(super) fn real_uri(&self, cx: &App) -> String {
        let uri = self.draft.uri_state.read(cx).value().to_string();
        if let Some(existing_pw) = extract_uri_password(&uri)
            && existing_pw == REDACTED_PASSWORD
        {
            let real_pw = self.draft.password_state.read(cx).value().to_string();
            let real_pw = real_pw.trim();
            if !real_pw.is_empty() {
                return inject_uri_password(&uri, Some(real_pw));
            }
        }
        uri
    }

    pub(super) fn import_from_uri(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let uri = self.draft.uri_state.read(cx).value().to_string();
        match parse_uri(&uri) {
            Ok(parts) => {
                let (user, password) = parts.userinfo();
                self.draft
                    .username_state
                    .update(cx, |state, cx| state.set_value(user.unwrap_or_default(), window, cx));
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
                self.draft
                    .write_concern_state
                    .update(cx, |state, cx| state.set_value(parts.get_query("w"), window, cx));
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

                let redacted = redact_uri_password(&uri);
                self.draft.uri_state.update(cx, |state, cx| {
                    state.set_value(redacted, window, cx);
                    state.set_cursor_position(Position::new(0, 0), window, cx);
                });
            }
            Err(err) => {
                self.parse_error = Some(err);
            }
        }
    }

    pub(super) fn update_uri_from_fields(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let uri = self.draft.uri_state.read(cx).value().to_string();
        let mut parts = match parse_uri(&uri) {
            Ok(parts) => parts,
            Err(err) => {
                self.parse_error = Some(err);
                return false;
            }
        };

        let username = value_or_none(&self.draft.username_state, cx);
        let password = value_or_none(&self.draft.password_state, cx);
        parts.set_userinfo(username, password);
        parts.set_query("appName", value_or_none(&self.draft.app_name_state, cx));
        parts.set_query("authSource", value_or_none(&self.draft.auth_source_state, cx));
        parts.set_query("authMechanism", value_or_none(&self.draft.auth_mechanism_state, cx));
        parts.set_query(
            "authMechanismProperties",
            value_or_none(&self.draft.auth_mechanism_props_state, cx),
        );
        parts.set_query("readPreference", value_or_none(&self.draft.read_preference_state, cx));
        parts.set_query("readConcernLevel", value_or_none(&self.draft.read_concern_state, cx));
        parts.set_query("w", value_or_none(&self.draft.write_concern_state, cx));
        parts.set_query("wTimeoutMS", value_or_none(&self.draft.w_timeout_state, cx));
        parts.set_query("connectTimeoutMS", value_or_none(&self.draft.connect_timeout_state, cx));
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
        parts.set_query("zlibCompressionLevel", value_or_none(&self.draft.zlib_level_state, cx));
        parts.set_query("tlsCAFile", value_or_none(&self.draft.tls_ca_file_state, cx));
        parts.set_query(
            "tlsCertificateKeyFile",
            value_or_none(&self.draft.tls_cert_key_file_state, cx),
        );
        parts.set_query(
            "tlsCertificateKeyFilePassword",
            value_or_none(&self.draft.tls_cert_key_password_state, cx),
        );
        parts.set_query("directConnection", bool_to_query(self.draft.direct_connection));
        parts.set_query("tls", bool_to_query(self.draft.tls));
        parts.set_query("tlsInsecure", bool_to_query(self.draft.tls_insecure));

        let updated = redact_uri_password(&parts.to_uri());
        self.draft.uri_state.update(cx, |state, cx| {
            state.set_value(updated, window, cx);
            state.set_cursor_position(Position::new(0, 0), window, cx);
        });
        self.parse_error = None;
        true
    }

    pub(super) fn start_test(view: Entity<ConnectionManager>, cx: &mut App) {
        let display_uri = view.read(cx).draft.uri_state.read(cx).value().to_string();
        let real_uri = view.read(cx).real_uri(cx);
        let (ssh, proxy) = match view.read(cx).build_transport_settings(cx) {
            Ok(settings) => settings,
            Err(err) => {
                view.update(cx, |this, cx| {
                    this.parse_error = Some(err.clone());
                    this.status = TestStatus::Error(err);
                    this.last_tested_uri = None;
                    this.pending_test_uri = None;
                    this.testing_step = None;
                    cx.notify();
                });
                return;
            }
        };
        if let Err(err) = validate_mongodb_uri(&real_uri) {
            view.update(cx, |this, cx| {
                let message = err.to_string();
                this.parse_error = Some(message.clone());
                this.status = TestStatus::Error(message);
                this.last_tested_uri = None;
                this.pending_test_uri = None;
                this.testing_step = None;
                cx.notify();
            });
            return;
        }

        let manager = view.read(cx).state.read(cx).connection_manager();

        view.update(cx, |this, cx| {
            this.status = TestStatus::Testing;
            this.pending_test_uri = Some(display_uri.clone());
            this.last_tested_uri = None;
            this.parse_error = None;
            this.testing_step = Some("Preparing transport settings".to_string());
            cx.notify();
        });

        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<String>();
        let task = cx.background_spawn({
            async move {
                let mut temp = SavedConnection::new("Test".to_string(), real_uri);
                temp.ssh = ssh;
                temp.proxy = proxy;
                manager.test_connection_with_progress(
                    &temp,
                    std::time::Duration::from_secs(TEST_CONNECTION_TIMEOUT_SECS),
                    move |step| {
                        let _ = progress_tx.send(step);
                    },
                )?;
                Ok::<(), crate::error::Error>(())
            }
        });

        cx.spawn({
            let view = view.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let mut task = std::pin::pin!(task);
                let mut progress_closed = false;
                loop {
                    tokio::select! {
                        maybe_step = progress_rx.recv(), if !progress_closed => {
                            let Some(step) = maybe_step else {
                                progress_closed = true;
                                continue;
                            };
                            let _ = cx.update(|cx| {
                                view.update(cx, |this, cx| {
                                    if matches!(this.status, TestStatus::Testing) {
                                        this.testing_step = Some(step.clone());
                                        cx.notify();
                                    }
                                });
                            });
                        }
                        result = &mut task => {
                            let result: Result<(), crate::error::Error> = result;
                            let _ = cx.update(|cx| {
                                view.update(cx, |this, cx| {
                                    let current_uri = this.draft.uri_state.read(cx).value().to_string();
                                    let pending = this.pending_test_uri.clone();
                                    if pending.as_deref() != Some(current_uri.trim()) {
                                        this.status = TestStatus::Idle;
                                        this.pending_test_uri = None;
                                        this.last_tested_uri = None;
                                        this.testing_step = None;
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
                                    this.testing_step = None;
                                    cx.notify();
                                });
                            });
                            break;
                        }
                    }
                }
            }
        })
        .detach();
    }

    pub(super) fn save_connection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Uuid> {
        if !self.update_uri_from_fields(window, cx) {
            if let Some(err) = self.parse_error.clone() {
                self.status = TestStatus::Error(err);
            }
            return None;
        }
        let uri = self.real_uri(cx);
        if validate_mongodb_uri(&uri).is_err() {
            let message = "Invalid MongoDB URI".to_string();
            self.parse_error = Some(message.clone());
            self.status = TestStatus::Error(message);
            return None;
        }

        let name_input = self.draft.name_state.read(cx).value().to_string();
        let name = if name_input.trim().is_empty() {
            extract_host_from_uri(&uri).unwrap_or_else(|| "Untitled".to_string())
        } else {
            name_input.trim().to_string()
        };

        let read_only = self.draft.read_only;
        let (ssh, proxy) = match self.build_transport_settings(cx) {
            Ok(settings) => settings,
            Err(err) => {
                self.parse_error = Some(err.clone());
                self.status = TestStatus::Error(err);
                return None;
            }
        };
        let selected_id = self.selected_id;
        let mut saved_connection: Option<SavedConnection> = None;
        self.state.update(cx, |state, cx| {
            if let Some(existing_id) = selected_id {
                if let Some(existing) = state.connections.iter().find(|conn| conn.id == existing_id)
                {
                    let connection = SavedConnection {
                        id: existing_id,
                        name: name.clone(),
                        uri: uri.clone(),
                        last_connected: existing.last_connected,
                        read_only,
                        ssh: ssh.clone(),
                        proxy: proxy.clone(),
                    };
                    state.update_connection(connection.clone(), cx);
                    saved_connection = Some(connection);
                }
            } else {
                let mut connection = SavedConnection::new(name.clone(), uri.clone());
                connection.read_only = read_only;
                connection.ssh = ssh.clone();
                connection.proxy = proxy.clone();
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

    pub(super) fn remove_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn import_uri_from_clipboard_or_dialog(
        view: Entity<ConnectionManager>,
        window: &mut Window,
        cx: &mut App,
    ) {
        ConnectionManager::open_import_uri_dialog(view, window, cx);
    }

    pub(super) fn open_import_uri_dialog(
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
                                                if let Some(text) = cx
                                                    .read_from_clipboard()
                                                    .and_then(|item| item.text())
                                                {
                                                    let value = text
                                                        .lines()
                                                        .next()
                                                        .unwrap_or("")
                                                        .trim()
                                                        .to_string();
                                                    if value.is_empty() {
                                                        return;
                                                    }
                                                    input_state.update(cx, |state, cx| {
                                                        state.set_value(value, window, cx);
                                                        state.set_cursor_position(
                                                            Position::new(0, 0),
                                                            window,
                                                            cx,
                                                        );
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
                            div().text_xs().text_color(cx.theme().muted_foreground).child(
                                "Paste a mongodb:// or mongodb+srv:// URI to import settings.",
                            ),
                        ),
                )
                .footer({
                    let view = view.clone();
                    let input_state = input_state.clone();
                    move |_ok, _cancel, _window, _cx| {
                        let view = view.clone();
                        let input_state = input_state.clone();
                        vec![
                            cancel_button("cancel-import-uri"),
                            Button::new("confirm-import-uri")
                                .primary()
                                .label("Import")
                                .on_click({
                                    let view = view.clone();
                                    let input_state = input_state.clone();
                                    move |_, window, cx| {
                                        let raw = input_state.read(cx).value().to_string();
                                        let value =
                                            raw.lines().next().unwrap_or("").trim().to_string();
                                        if value.is_empty() {
                                            window.close_dialog(cx);
                                            return;
                                        }
                                        view.update(cx, |this, cx| {
                                            this.draft.uri_state.update(cx, |state, cx| {
                                                state.set_value(value.clone(), window, cx);
                                                state.set_cursor_position(
                                                    Position::new(0, 0),
                                                    window,
                                                    cx,
                                                );
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
}

fn parse_u16_or_default(
    raw: String,
    default: u16,
    label: &str,
) -> std::result::Result<u16, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(default);
    }
    let parsed = trimmed
        .parse::<u16>()
        .map_err(|_| format!("{label} must be a number between 1 and 65535"))?;
    if parsed == 0 {
        return Err(format!("{label} must be greater than 0"));
    }
    Ok(parsed)
}
