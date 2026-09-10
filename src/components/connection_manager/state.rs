use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{App, AppContext as _, Context, Entity, Window};
use uuid::Uuid;

use crate::state::{AppCommands, AppEvent, AppState};

use super::{ConnectionDraft, ConnectionManager, ManagerTab, TestStatus};

impl ManagerTab {
    pub(super) fn all() -> [ManagerTab; 6] {
        [
            ManagerTab::General,
            ManagerTab::Authentication,
            ManagerTab::Tls,
            ManagerTab::Network,
            ManagerTab::Advanced,
            ManagerTab::Access,
        ]
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            ManagerTab::General => "Connection",
            ManagerTab::Authentication => "Auth",
            ManagerTab::Tls => "TLS",
            ManagerTab::Network => "Network",
            ManagerTab::Advanced => "Advanced",
            ManagerTab::Access => "Access",
        }
    }

    pub(super) fn index(self) -> usize {
        Self::all().iter().position(|tab| *tab == self).unwrap_or(0)
    }

    pub(super) fn from_index(index: usize) -> Self {
        Self::all().get(index).copied().unwrap_or(ManagerTab::General)
    }
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
            read_preference_state: cx.new(|cx| InputState::new(window, cx).placeholder("primary")),
            read_concern_state: cx.new(|cx| InputState::new(window, cx).placeholder("local")),
            write_concern_state: cx.new(|cx| InputState::new(window, cx).placeholder("majority")),
            w_timeout_state: cx.new(|cx| InputState::new(window, cx).placeholder("5000")),
            connect_timeout_state: cx.new(|cx| InputState::new(window, cx).placeholder("10000")),
            server_selection_timeout_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("30000")),
            max_pool_state: cx.new(|cx| InputState::new(window, cx).placeholder("100")),
            min_pool_state: cx.new(|cx| InputState::new(window, cx).placeholder("0")),
            heartbeat_frequency_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("10000")),
            compressors_state: cx.new(|cx| InputState::new(window, cx).placeholder("zstd,zlib")),
            zlib_level_state: cx.new(|cx| InputState::new(window, cx).placeholder("6")),
            tls_ca_file_state: cx.new(|cx| InputState::new(window, cx).placeholder("/path/ca.pem")),
            tls_cert_key_file_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("/path/cert.pem")),
            tls_cert_key_password_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("password").masked(true)),
            ssh_host_state: cx.new(|cx| InputState::new(window, cx).placeholder("ssh.example.com")),
            ssh_port_state: cx.new(|cx| InputState::new(window, cx).placeholder("22")),
            ssh_username_state: cx.new(|cx| InputState::new(window, cx).placeholder("ubuntu")),
            ssh_password_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("password").masked(true)),
            ssh_identity_file_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("~/.ssh/id_ed25519")),
            ssh_identity_passphrase_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("passphrase").masked(true)),
            ssh_local_bind_host_state: cx
                .new(|cx| InputState::new(window, cx).default_value("127.0.0.1")),
            proxy_host_state: cx.new(|cx| InputState::new(window, cx).placeholder("127.0.0.1")),
            proxy_port_state: cx.new(|cx| InputState::new(window, cx).placeholder("1080")),
            proxy_username_state: cx.new(|cx| InputState::new(window, cx).placeholder("username")),
            proxy_password_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("password").masked(true)),
            uri_secrets: crate::helpers::UriSecrets::default(),
            internal_uri_value: None,
            color: None,
            environment: None,
            confirm_production_writes: false,
            read_only: false,
            agent_shared: false,
            agent_writable: false,
            protected: false,
            history_enabled: false,
            direct_connection: false,
            tls: false,
            tls_insecure: false,
            ssh_enabled: false,
            ssh_use_identity_file: false,
            ssh_strict_host_key_checking: true,
            proxy_enabled: false,
            pool_expanded: false,
            compression_expanded: false,
        }
    }

    fn input_states(&self) -> [&Entity<InputState>; 33] {
        [
            &self.name_state,
            &self.uri_state,
            &self.username_state,
            &self.password_state,
            &self.app_name_state,
            &self.auth_source_state,
            &self.auth_mechanism_state,
            &self.auth_mechanism_props_state,
            &self.read_preference_state,
            &self.read_concern_state,
            &self.write_concern_state,
            &self.w_timeout_state,
            &self.connect_timeout_state,
            &self.server_selection_timeout_state,
            &self.max_pool_state,
            &self.min_pool_state,
            &self.heartbeat_frequency_state,
            &self.compressors_state,
            &self.zlib_level_state,
            &self.tls_ca_file_state,
            &self.tls_cert_key_file_state,
            &self.tls_cert_key_password_state,
            &self.ssh_host_state,
            &self.ssh_port_state,
            &self.ssh_username_state,
            &self.ssh_password_state,
            &self.ssh_identity_file_state,
            &self.ssh_identity_passphrase_state,
            &self.ssh_local_bind_host_state,
            &self.proxy_host_state,
            &self.proxy_port_state,
            &self.proxy_username_state,
            &self.proxy_password_state,
        ]
    }

    pub(super) fn fingerprint(&self, cx: &App) -> String {
        let mut values = self
            .input_states()
            .into_iter()
            .map(|state| state.read(cx).value().to_string())
            .collect::<Vec<_>>();
        values.push(format!(
            "{:?}",
            (
                self.color,
                self.environment,
                self.confirm_production_writes,
                self.read_only,
                self.agent_shared,
                self.agent_writable,
                self.protected,
                self.history_enabled,
                self.direct_connection,
                self.tls,
                self.tls_insecure,
            )
        ));
        values.push(format!(
            "{:?}",
            (
                self.ssh_enabled,
                self.ssh_use_identity_file,
                self.ssh_strict_host_key_checking,
                self.proxy_enabled,
            )
        ));
        values.push(serde_json::to_string(&self.uri_secrets).unwrap());
        serde_json::to_string(&values).unwrap()
    }

    pub(super) fn reset(&mut self, window: &mut Window, cx: &mut Context<ConnectionManager>) {
        self.name_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.uri_state.update(cx, |state, cx| {
            state.set_value("mongodb://localhost:27017".to_string(), window, cx)
        });
        self.username_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.password_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.app_name_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.auth_source_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.auth_mechanism_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.auth_mechanism_props_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.read_preference_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.read_concern_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.write_concern_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.w_timeout_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.connect_timeout_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.server_selection_timeout_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.max_pool_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.min_pool_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.heartbeat_frequency_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.compressors_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.zlib_level_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.tls_ca_file_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.tls_cert_key_file_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.tls_cert_key_password_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.ssh_host_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.ssh_port_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.ssh_username_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.ssh_password_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.ssh_identity_file_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.ssh_identity_passphrase_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.ssh_local_bind_host_state
            .update(cx, |state, cx| state.set_value("127.0.0.1".to_string(), window, cx));
        self.proxy_host_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.proxy_port_state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.proxy_username_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.proxy_password_state
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.uri_secrets = crate::helpers::UriSecrets::default();
        self.internal_uri_value = None;
        self.color = None;
        self.environment = None;
        self.confirm_production_writes = false;
        self.read_only = false;
        self.agent_shared = false;
        self.agent_writable = false;
        self.protected = false;
        self.history_enabled = false;
        self.direct_connection = false;
        self.tls = false;
        self.tls_insecure = false;
        self.ssh_enabled = false;
        self.ssh_use_identity_file = false;
        self.ssh_strict_host_key_checking = true;
        self.proxy_enabled = false;
        self.pool_expanded = false;
        self.compression_expanded = false;
    }
}

impl ConnectionManager {
    pub fn new(
        state: Entity<AppState>,
        selected_id: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = ConnectionDraft::new(window, cx);
        let manager = cx.entity().downgrade();
        let connection_list = cx.new(|cx| {
            gpui_kit::component::list::ListState::new(
                super::connection_list::ConnectionList {
                    manager,
                    state: state.clone(),
                    query: String::new(),
                    connections: state.read(cx).connections_snapshot(),
                    selected: None,
                },
                window,
                cx,
            )
            .searchable(true)
        });
        let mut subscriptions = vec![cx.observe_in(&state, window, |view, _, window, cx| {
            view.sync_connection_list(window, cx);
            cx.notify();
        })];
        subscriptions.push(cx.subscribe_in(&state, window, |view, state, event, window, cx| {
            match event {
                AppEvent::Connecting(id) if Some(*id) == view.selected_id => {
                    view.connecting_id = Some(*id);
                }
                AppEvent::Connected(id) if view.connecting_id == Some(*id) => {
                    view.connecting_id = None;
                    view.status = TestStatus::Idle;
                }
                AppEvent::ConnectionFailed { connection_id, error }
                    if view.connecting_id == Some(*connection_id)
                        && view.selected_id == Some(*connection_id) =>
                {
                    view.connecting_id = None;
                    view.status = TestStatus::Error(error.clone());
                }
                AppEvent::ConnectionRemoved
                    if view
                        .selected_id
                        .is_some_and(|id| state.read(cx).connection_by_id(id).is_none()) =>
                {
                    view.selected_id = None;
                    view.creating_new = true;
                }
                _ => {}
            }
            cx.notify();
            let AppEvent::ConnectionSaveFinished { connection_id, result } = event else {
                return;
            };
            if view
                .pending_save
                .as_ref()
                .is_none_or(|pending| pending.connection_id != *connection_id)
            {
                return;
            }
            let pending = view.pending_save.take().unwrap();
            match result {
                Ok(()) => {
                    let unchanged = view.draft.fingerprint(cx) == pending.fingerprint;
                    view.selected_id = Some(*connection_id);
                    view.creating_new = false;
                    view.new_connection_origin_id = None;
                    view.baseline_fingerprint = pending.fingerprint;
                    if unchanged {
                        let saved = state.read(cx).connection_by_id(*connection_id).cloned();
                        view.load_connection(saved, window, cx);
                    }
                    view.status = TestStatus::Saved;
                    if pending.connect {
                        let state = state.clone();
                        let connection_id = *connection_id;
                        window.defer(cx, move |window, cx| {
                            if state.read(cx).is_connected(connection_id) {
                                let connect_state = state.clone();
                                crate::components::request_unsaved_action(
                                    state,
                                    crate::state::UnsavedScope::Connection(connection_id),
                                    window,
                                    cx,
                                    move |_, cx| {
                                        AppCommands::connect(connect_state, connection_id, cx)
                                    },
                                );
                            } else {
                                AppCommands::connect(state, connection_id, cx);
                            }
                        });
                    }
                }
                Err(error) => {
                    view.status = TestStatus::Error(format!("Could not save connection: {error}"));
                }
            }
            view.sync_connection_list(window, cx);
            cx.notify();
        }));

        for input in draft.input_states() {
            subscriptions.push(cx.subscribe(input, |view, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    if view.has_unsaved_changes(cx) && !matches!(view.status, TestStatus::Testing) {
                        view.status = TestStatus::Idle;
                        view.last_tested_fingerprint = None;
                    }
                    cx.notify();
                }
            }));
        }

        let uri_state = draft.uri_state.clone();
        subscriptions.push(cx.subscribe_in(
            &uri_state,
            window,
            move |view, _state, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.capture_uri_secrets(window, cx);
                    cx.notify();
                }
            },
        ));

        let mut view = Self {
            state,
            connection_list,
            selected_id,
            connecting_id: None,
            draft,
            testing_step: None,
            active_tab: ManagerTab::General,
            creating_new: false,
            new_connection_origin_id: None,
            baseline_fingerprint: String::new(),
            status: TestStatus::Idle,
            last_tested_fingerprint: None,
            pending_test_fingerprint: None,
            test_generation: 0,
            pending_save: None,
            parse_error: None,
            _subscriptions: subscriptions,
        };

        if let Some(connection_id) = selected_id
            .or_else(|| view.state.read(cx).connections.first().map(|connection| connection.id))
            && let Some(connection) = view
                .state
                .read(cx)
                .connections
                .iter()
                .find(|conn| conn.id == connection_id)
                .cloned()
        {
            view.load_connection(Some(connection), window, cx);
        } else {
            view.creating_new = view.state.read(cx).connections.is_empty();
            view.baseline_fingerprint = view.draft.fingerprint(cx);
        }

        view
    }
}
