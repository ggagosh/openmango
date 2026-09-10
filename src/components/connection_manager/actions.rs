use gpui_kit::component::input::InputState;
use gpui_kit::{App, AppContext as _, Context, Entity, Focusable as _, Window};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::components::{open_confirm_dialog, request_remove_connection};
use crate::helpers::validate::{percent_decode, percent_encode};
use crate::helpers::{
    UriSecrets, extract_host_from_uri, extract_uri_secrets, inject_uri_secrets, strip_uri_secrets,
    validate_mongodb_uri,
};
use crate::models::{ProxyConfig, ProxyKind, SavedConnection, SshAuth, SshConfig};
use crate::state::{AppCommands, AppState, TabKey, UnsavedScope};

use super::uri::{bool_to_query, parse_bool, parse_uri, value_or_none};
use super::{ConnectionManager, TestStatus};

const TEST_CONNECTION_TIMEOUT_SECS: u64 = 30;

fn non_empty_value(state: &Entity<InputState>, cx: &App) -> Option<String> {
    let value = state.read(cx).value().to_string();
    (!value.is_empty()).then_some(value)
}

impl ConnectionManager {
    pub(super) fn request_save(
        view: Entity<Self>,
        connect: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if view.read(cx).pending_save.is_some()
            || matches!(view.read(cx).status, TestStatus::Testing)
            || view.read(cx).connecting_id == view.read(cx).selected_id
                && view.read(cx).connecting_id.is_some()
        {
            return;
        }
        let state = view.read(cx).state.clone();
        let selected_id = view.read(cx).selected_id;
        let save = {
            let state = state.clone();
            move |window: &mut Window, cx: &mut App| {
                if connect
                    && !view.read(cx).has_unsaved_changes(cx)
                    && let Some(connection_id) = view.read(cx).selected_id
                {
                    AppCommands::connect(state.clone(), connection_id, cx);
                } else {
                    view.update(cx, |this, cx| {
                        this.save_connection(connect, window, cx);
                    });
                }
            }
        };
        if connect
            && let Some(connection_id) = selected_id
            && state.read(cx).is_connected(connection_id)
        {
            crate::components::request_unsaved_action(
                state,
                UnsavedScope::Connection(connection_id),
                window,
                cx,
                save,
            );
        } else {
            save(window, cx);
        }
    }

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
        view: Entity<Self>,
        selected_id: Option<Uuid>,
        creating_new: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if creating_new {
            Self::request_load_connection(view, None, window, cx);
            return;
        }

        if let Some(connection) = selected_id.and_then(|connection_id| {
            view.read(cx)
                .state
                .read(cx)
                .connections
                .iter()
                .find(|connection| connection.id == connection_id)
                .cloned()
        }) {
            Self::request_load_connection(view, Some(connection), window, cx);
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
        if view.read(cx).pending_save.is_some() {
            return;
        }
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
                let focus = this.draft.uri_state.read(cx).focus_handle(cx);
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
        self.test_generation += 1;
        self.connecting_id = None;
        self.status = TestStatus::Idle;
        self.last_tested_fingerprint = None;
        self.pending_test_fingerprint = None;
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
        self.sync_connection_list(window, cx);
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
        let mut secrets = extract_uri_secrets(&uri);
        let sanitized = if secrets.is_empty() { uri.clone() } else { strip_uri_secrets(&uri) };
        if let Ok(parts) = parse_uri(&uri) {
            let (username, password) = parts.userinfo();
            let same_user = username.as_deref().unwrap_or_default()
                == self.draft.username_state.read(cx).value().as_ref();
            if same_user
                && parts.get_query("authMechanism").eq_ignore_ascii_case("MONGODB-AWS")
                && secrets.aws_session_token.is_none()
            {
                secrets.aws_session_token = self.draft.uri_secrets.aws_session_token.clone();
            }
            if same_user
                && !parts.get_query("proxyHost").is_empty()
                && secrets.proxy_password.is_none()
            {
                secrets.proxy_password = self.draft.uri_secrets.proxy_password.clone();
            }
            if username.as_deref() == Some(self.draft.username_state.read(cx).value().as_ref())
                && password.is_none()
            {
                secrets.password = non_empty_value(&self.draft.password_state, cx)
                    .map(|value| percent_encode(&value));
            }
            if secrets.tls_certificate_key_file_password.is_none() {
                secrets.tls_certificate_key_file_password =
                    non_empty_value(&self.draft.tls_cert_key_password_state, cx)
                        .map(|value| percent_encode(&value));
            }
        }
        // Preserve the authority while a credential-bearing URI has no host yet.
        if secrets.password.is_some()
            && let Some(userinfo) = uri
                .split_once("://")
                .and_then(|(_, rest)| rest.split_once('@').map(|(userinfo, _)| userinfo))
        {
            let username = userinfo.split_once(':').map(|(user, _)| user).unwrap_or(userinfo);
            self.draft
                .username_state
                .update(cx, |input, cx| input.set_value(percent_decode(username), window, cx));
        }
        self.import_uri_with_display(
            inject_uri_secrets(&sanitized, &secrets),
            sanitized,
            window,
            cx,
        );
    }

    pub(super) fn real_uri(&self, cx: &App) -> String {
        let uri = self.draft.uri_state.read(cx).value().to_string();
        let mut secrets = self.draft.uri_secrets.clone();
        secrets.password =
            non_empty_value(&self.draft.password_state, cx).map(|value| percent_encode(&value));
        secrets.tls_certificate_key_file_password =
            non_empty_value(&self.draft.tls_cert_key_password_state, cx)
                .map(|value| percent_encode(&value));
        inject_uri_secrets(&strip_uri_secrets(&uri), &secrets)
    }

    fn import_uri(&mut self, uri: String, window: &mut Window, cx: &mut Context<Self>) {
        let sanitized_uri = strip_uri_secrets(&uri);
        self.import_uri_with_display(uri, sanitized_uri, window, cx);
    }

    fn import_uri_with_display(
        &mut self,
        uri: String,
        sanitized_uri: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let uri_secrets = extract_uri_secrets(&uri);
        if self.draft.uri_state.read(cx).value().as_ref() != sanitized_uri {
            self.draft.internal_uri_value = Some(sanitized_uri.clone());
            self.draft
                .uri_state
                .update(cx, |state, cx| state.set_value(sanitized_uri.clone(), window, cx));
        }
        self.draft.password_state.update(cx, |state, cx| {
            state.set_value(
                uri_secrets.password.as_deref().map(percent_decode).unwrap_or_default(),
                window,
                cx,
            )
        });
        self.draft.tls_cert_key_password_state.update(cx, |state, cx| {
            state.set_value(
                uri_secrets
                    .tls_certificate_key_file_password
                    .as_deref()
                    .map(percent_decode)
                    .unwrap_or_default(),
                window,
                cx,
            )
        });
        self.draft.uri_secrets = UriSecrets {
            password: None,
            tls_certificate_key_file_password: None,
            proxy_password: uri_secrets.proxy_password,
            aws_session_token: uri_secrets.aws_session_token,
        };
        match parse_uri(&sanitized_uri) {
            Ok(parts) => {
                let (user, _redacted_password) = parts.userinfo();
                self.draft
                    .username_state
                    .update(cx, |state, cx| state.set_value(user.unwrap_or_default(), window, cx));
                self.draft.password_state.update(cx, |state, cx| {
                    state.set_value(
                        uri_secrets.password.as_deref().map(percent_decode).unwrap_or_default(),
                        window,
                        cx,
                    )
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
                        uri_secrets
                            .tls_certificate_key_file_password
                            .as_deref()
                            .map(percent_decode)
                            .unwrap_or_default(),
                        window,
                        cx,
                    )
                });
                self.draft.direct_connection = parse_bool(parts.get_query("directConnection"));
                let tls = if parts.get_query("tls").is_empty() {
                    parts.get_query("ssl")
                } else {
                    parts.get_query("tls")
                };
                self.draft.tls = if tls.is_empty() {
                    sanitized_uri.starts_with("mongodb+srv://")
                } else {
                    parse_bool(tls)
                };
                self.draft.tls_insecure = parse_bool(parts.get_query("tlsInsecure"));
                self.parse_error = None;

                if self.draft.uri_state.read(cx).value().as_ref() != sanitized_uri {
                    self.draft.internal_uri_value = Some(sanitized_uri.clone());
                    self.draft
                        .uri_state
                        .update(cx, |state, cx| state.set_value(sanitized_uri, window, cx));
                }
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

        let username = non_empty_value(&self.draft.username_state, cx);
        let password = non_empty_value(&self.draft.password_state, cx);
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
        let tls_key = if parts.get_query("tls").is_empty() && !parts.get_query("ssl").is_empty() {
            "ssl"
        } else {
            "tls"
        };
        let tls = if parts.get_query(tls_key).is_empty()
            && self.draft.tls == uri.starts_with("mongodb+srv://")
        {
            None
        } else {
            Some(self.draft.tls.to_string())
        };
        parts.set_query(tls_key, tls);
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
        if updated != uri {
            self.draft.internal_uri_value = Some(updated.clone());
            self.draft.uri_state.update(cx, |state, cx| state.set_value(updated, window, cx));
        }
        self.parse_error = None;
        true
    }

    pub(super) fn start_test(view: Entity<ConnectionManager>, window: &mut Window, cx: &mut App) {
        if matches!(view.read(cx).status, TestStatus::Testing)
            || view.read(cx).pending_save.is_some()
        {
            return;
        }
        if !view.update(cx, |this, cx| this.update_uri_from_fields(window, cx)) {
            return;
        }
        let fingerprint = view.read(cx).draft.fingerprint(cx);
        let real_uri = view.read(cx).real_uri(cx);
        let (ssh, proxy) = match view.read(cx).build_transport_settings(cx) {
            Ok(settings) => settings,
            Err(err) => {
                view.update(cx, |this, cx| {
                    this.parse_error = Some(err.clone());
                    this.status = TestStatus::Error(err);
                    this.last_tested_fingerprint = None;
                    this.pending_test_fingerprint = None;
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
                this.last_tested_fingerprint = None;
                this.pending_test_fingerprint = None;
                this.testing_step = None;
                cx.notify();
            });
            return;
        }

        let manager = view.read(cx).state.read(cx).connection_manager();

        let generation = view.update(cx, |this, cx| {
            this.status = TestStatus::Testing;
            this.test_generation += 1;
            this.pending_test_fingerprint = Some(fingerprint.clone());
            this.last_tested_fingerprint = None;
            this.parse_error = None;
            this.testing_step = Some("Preparing transport settings".to_string());
            cx.notify();
            this.test_generation
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
                                    if this.test_generation == generation && matches!(this.status, TestStatus::Testing) {
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
                                    this.finish_test(generation, result.map_err(|error| error.to_string()), cx);
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

    pub(super) fn finish_test(
        &mut self,
        generation: u64,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if self.test_generation != generation {
            return;
        }
        let fingerprint = self.draft.fingerprint(cx);
        if self.pending_test_fingerprint.as_ref() != Some(&fingerprint) {
            self.status = TestStatus::Idle;
            self.last_tested_fingerprint = None;
        } else {
            self.status = match result {
                Ok(()) => TestStatus::Success,
                Err(error) => TestStatus::Error(error),
            };
            self.last_tested_fingerprint = Some(fingerprint);
        }
        self.pending_test_fingerprint = None;
        self.testing_step = None;
        cx.notify();
    }

    pub(super) fn save_connection(
        &mut self,
        connect: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Uuid> {
        if self.pending_save.is_some() || !self.state.read(cx).connection_secrets_ready() {
            return None;
        }
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
            self.pending_save = Some(super::PendingSave {
                connection_id: saved_id,
                fingerprint: self.draft.fingerprint(cx),
                connect,
            });
            cx.notify();
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
