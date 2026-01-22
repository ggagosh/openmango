use gpui::{AppContext as _, Context, Entity, Window};
use gpui_component::input::{InputEvent, InputState};
use uuid::Uuid;

use crate::state::AppState;

use super::{ConnectionDraft, ConnectionManager, ManagerTab, TestStatus};

impl ManagerTab {
    pub(super) fn all() -> [ManagerTab; 6] {
        [
            ManagerTab::General,
            ManagerTab::Auth,
            ManagerTab::Options,
            ManagerTab::Tls,
            ManagerTab::Pool,
            ManagerTab::Advanced,
        ]
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            ManagerTab::General => "General",
            ManagerTab::Auth => "Auth",
            ManagerTab::Options => "Options",
            ManagerTab::Tls => "TLS",
            ManagerTab::Pool => "Pool & Timeouts",
            ManagerTab::Advanced => "Advanced",
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
            read_only: false,
            direct_connection: false,
            tls: false,
            tls_insecure: false,
        }
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
        self.read_only = false;
        self.direct_connection = false;
        self.tls = false;
        self.tls_insecure = false;
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
}
