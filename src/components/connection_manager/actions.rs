use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::input::{Input, InputState, Position};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Focusable as _, IntoElement as _, ParentElement as _,
    Styled as _, Window, div, px,
};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::components::{Button, cancel_button, open_confirm_dialog, request_remove_connection};
use crate::helpers::{
    UriSecrets, extract_host_from_uri, extract_uri_secrets, inject_uri_secrets, strip_uri_secrets,
    validate_mongodb_uri,
};
use crate::models::{ProxyConfig, ProxyKind, SavedConnection, SshAuth, SshConfig};
use crate::state::{AppState, TabKey};
use crate::theme::spacing;

use super::uri::{bool_to_query, parse_bool, parse_uri, value_or_none};
use super::{ConnectionManager, TestStatus};

const TEST_CONNECTION_TIMEOUT_SECS: u64 = 30;

fn non_empty_value(state: &Entity<InputState>, cx: &App) -> Option<String> {
    let value = state.read(cx).value().trim().to_string();
    (!value.is_empty()).then_some(value)
}

impl ConnectionManager {
    pub fn open(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
        Self::open_with_selected(state, None, false, window, cx);
    }

    pub fn open_new(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
        Self::open_with_selected(state, None, true, window, cx);
    }

    pub fn open_selected(
        state: Entity<AppState>,
        connection_id: Uuid,
        window: &mut Window,
        cx: &mut App,
    ) {
        Self::open_with_selected(state, Some(connection_id), false, window, cx);
    }

    pub(super) fn open_with_selected(
        state: Entity<AppState>,
        selected_id: Option<Uuid>,
        creating_new: bool,
        _window: &mut Window,
        cx: &mut App,
    ) {
        state.update(cx, |state, cx| {
            state.open_connections_tab(selected_id, creating_new, cx);
        });
    }

    pub(crate) fn apply_open_request(
        &mut self,
        selected_id: Option<Uuid>,
        creating_new: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if creating_new {
            Self::request_load_connection(cx.entity(), None, window, cx);
            return;
        }

        if let Some(connection) = selected_id.and_then(|connection_id| {
            self.state
                .read(cx)
                .connections
                .iter()
                .find(|connection| connection.id == connection_id)
                .cloned()
        }) {
            Self::request_load_connection(cx.entity(), Some(connection), window, cx);
        }
    }

    pub(crate) fn has_unsaved_changes(&self, cx: &App) -> bool {
        self.draft.fingerprint(cx) != self.baseline_fingerprint
    }

    pub(super) fn request_load_connection(
        view: Entity<Self>,
        connection: Option<SavedConnection>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let is_same_connection = connection.as_ref().is_some_and(|connection| {
            !view.read(cx).creating_new && view.read(cx).selected_id == Some(connection.id)
        });
        if is_same_connection {
            return;
        }

        if view.read(cx).has_unsaved_changes(cx) {
            open_confirm_dialog(
                window,
                cx,
                "Discard connection changes?",
                "Your unsaved connection changes will be lost.",
                "Discard",
                true,
                move |window, cx| {
                    Self::replace_draft(view, connection, window, cx);
                },
            );
        } else {
            Self::replace_draft(view, connection, window, cx);
        }
    }

    fn replace_draft(
        view: Entity<Self>,
        connection: Option<SavedConnection>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let creating_new = connection.is_none();
        view.update(cx, |this, cx| {
            if creating_new && !this.creating_new {
                this.new_connection_origin_id = this.selected_id;
            }
            this.load_connection(connection, window, cx);
            this.active_tab = super::ManagerTab::General;
            if creating_new {
                let focus = this.draft.name_state.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            cx.notify();
        });
    }

    pub(super) fn request_cancel_new(view: Entity<Self>, window: &mut Window, cx: &mut App) {
        let target = {
            let manager = view.read(cx);
            let state = manager.state.read(cx);
            manager
                .new_connection_origin_id
                .and_then(|id| state.connections.iter().find(|connection| connection.id == id))
                .or_else(|| state.connections.first())
                .cloned()
        };
        if let Some(connection) = target {
            Self::request_load_connection(view, Some(connection), window, cx);
            return;
        }

        let state = view.read(cx).state.clone();
        let close_view = view.clone();
        let close = move |window: &mut Window, cx: &mut App| {
            close_view.update(cx, |manager, cx| {
                manager.new_connection_origin_id = None;
                manager.load_connection(None, window, cx);
            });
            state.update(cx, |state, cx| {
                if let Some(index) =
                    state.open_tabs().iter().position(|tab| matches!(tab, TabKey::Connections))
                {
                    state.close_tab(index, cx);
                }
            });
        };
        if view.read(cx).has_unsaved_changes(cx) {
            open_confirm_dialog(
                window,
                cx,
                "Discard new connection?",
                "Your unsaved connection changes will be lost.",
                "Discard",
                true,
                close,
            );
        } else {
            close(window, cx);
        }
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
            self.new_connection_origin_id = None;
            self.draft
                .name_state
                .update(cx, |state, cx| state.set_value(connection.name.clone(), window, cx));
            self.draft.color = connection.color;
            self.draft.environment = connection.environment;
            self.draft.confirm_production_writes = connection.confirm_production_writes;
            self.draft.read_only = connection.read_only;
            self.draft.agent_shared = connection.agent_shared;
            self.draft.agent_writable = connection.agent_writable;
            self.draft.protected = connection.protected;
            self.draft.history_enabled = connection.history_enabled;
            self.load_transport_settings(&connection, window, cx);
            self.import_uri(connection.uri.clone(), window, cx);
        } else {
            self.selected_id = None;
            self.draft.reset(window, cx);
        }
        self.baseline_fingerprint = self.draft.fingerprint(cx);
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

    pub(super) fn capture_uri_secrets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let uri = self.draft.uri_state.read(cx).value().to_string();
        if !crate::components::should_capture_uri_change(&mut self.draft.internal_uri_value, &uri) {
            return;
        }
        let secrets = extract_uri_secrets(&uri);
        self.draft.password_state.update(cx, |state, cx| {
            state.set_value(secrets.password.clone().unwrap_or_default(), window, cx)
        });
        self.draft.tls_cert_key_password_state.update(cx, |state, cx| {
            state.set_value(
                secrets.tls_certificate_key_file_password.clone().unwrap_or_default(),
                window,
                cx,
            )
        });
        self.draft.uri_secrets = UriSecrets {
            password: None,
            tls_certificate_key_file_password: None,
            proxy_password: secrets.proxy_password,
            aws_session_token: secrets.aws_session_token,
        };
        let sanitized = strip_uri_secrets(&uri);
        if sanitized != uri {
            self.draft.internal_uri_value = Some(sanitized.clone());
            self.draft.uri_state.update(cx, |state, cx| state.set_value(sanitized, window, cx));
        }
    }

    pub(super) fn real_uri(&self, cx: &App) -> String {
        let uri = self.draft.uri_state.read(cx).value().to_string();
        let mut secrets = self.draft.uri_secrets.clone();
        secrets.password = non_empty_value(&self.draft.password_state, cx);
        secrets.tls_certificate_key_file_password =
            non_empty_value(&self.draft.tls_cert_key_password_state, cx);
        inject_uri_secrets(&strip_uri_secrets(&uri), &secrets)
    }

    pub(super) fn import_from_uri(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let uri = self.draft.uri_state.read(cx).value().to_string();
        self.import_uri(uri, window, cx);
    }

    fn import_uri(&mut self, uri: String, window: &mut Window, cx: &mut Context<Self>) {
        let uri_secrets = extract_uri_secrets(&uri);
        let sanitized_uri = strip_uri_secrets(&uri);
        match parse_uri(&sanitized_uri) {
            Ok(parts) => {
                let (user, _redacted_password) = parts.userinfo();
                self.draft
                    .username_state
                    .update(cx, |state, cx| state.set_value(user.unwrap_or_default(), window, cx));
                self.draft.password_state.update(cx, |state, cx| {
                    state.set_value(uri_secrets.password.clone().unwrap_or_default(), window, cx)
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
                    state.set_value(
                        uri_secrets.tls_certificate_key_file_password.clone().unwrap_or_default(),
                        window,
                        cx,
                    )
                });
                self.draft.direct_connection = parse_bool(parts.get_query("directConnection"));
                self.draft.tls = parse_bool(parts.get_query("tls"));
                self.draft.tls_insecure = parse_bool(parts.get_query("tlsInsecure"));
                self.parse_error = None;

                self.draft.uri_secrets = UriSecrets {
                    password: None,
                    tls_certificate_key_file_password: None,
                    proxy_password: uri_secrets.proxy_password,
                    aws_session_token: uri_secrets.aws_session_token,
                };
                self.draft.internal_uri_value = Some(sanitized_uri.clone());
                self.draft.uri_state.update(cx, |state, cx| {
                    state.set_value(sanitized_uri, window, cx);
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
        parts.set_query("tlsCertificateKeyFilePassword", None);
        parts.set_query("directConnection", bool_to_query(self.draft.direct_connection));
        parts.set_query("tls", bool_to_query(self.draft.tls));
        parts.set_query("tlsInsecure", bool_to_query(self.draft.tls_insecure));

        let rebuilt = parts.to_uri();
        let extracted = extract_uri_secrets(&rebuilt);
        if extracted.aws_session_token.is_some() {
            self.draft.uri_secrets.aws_session_token = extracted.aws_session_token;
        }
        if extracted.proxy_password.is_some() {
            self.draft.uri_secrets.proxy_password = extracted.proxy_password;
        }
        let updated = strip_uri_secrets(&rebuilt);
        self.draft.internal_uri_value = Some(updated.clone());
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
            async move |cx: &mut gpui_kit::AsyncApp| {
                let mut task = std::pin::pin!(task);
                let mut progress_closed = false;
                loop {
                    tokio::select! {
                        maybe_step = progress_rx.recv(), if !progress_closed => {
                            let Some(step) = maybe_step else {
                                progress_closed = true;
                                continue;
                            };
                            cx.update(|cx| {
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
                            cx.update(|cx| {
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

        let color = self.draft.color;
        let environment = self.draft.environment;
        let confirm_production_writes = self.draft.confirm_production_writes;
        let read_only = self.draft.read_only;
        let agent_shared = self.draft.agent_shared;
        let agent_writable = self.draft.agent_writable;
        let protected = self.draft.protected;
        let history_enabled = self.draft.history_enabled;
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
                        color,
                        environment,
                        confirm_production_writes,
                        uri: uri.clone(),
                        last_connected: existing.last_connected,
                        read_only,
                        agent_shared,
                        agent_writable,
                        protected,
                        history_enabled,
                        history_max_age_days: existing.history_max_age_days,
                        history_max_bytes: existing.history_max_bytes,
                        ssh: ssh.clone(),
                        proxy: proxy.clone(),
                        secret_id: existing.secret_id,
                    };
                    state.update_connection(connection.clone(), cx);
                    saved_connection = Some(connection);
                }
            } else {
                let mut connection = SavedConnection::new(name.clone(), uri.clone());
                connection.color = color;
                connection.environment = environment;
                connection.confirm_production_writes = confirm_production_writes;
                connection.read_only = read_only;
                connection.agent_shared = agent_shared;
                connection.agent_writable = agent_writable;
                connection.protected = protected;
                connection.history_enabled = history_enabled;
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
            move |window, cx| {
                request_remove_connection(state.clone(), connection_id, window, cx);
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
                                        .xsmall()
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
                                        .xsmall()
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

                    let view = view.clone();
                    let input_state = input_state.clone();
                    gpui_kit::component::dialog::DialogFooter::new().children(vec![
                        cancel_button("cancel-import-uri"),
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
                    ])
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
