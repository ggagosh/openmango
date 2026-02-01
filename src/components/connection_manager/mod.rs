use gpui::{Entity, Subscription};
use gpui_component::input::InputState;
use uuid::Uuid;

use crate::state::AppState;

mod actions;
mod connection_list;
mod state;
mod tabs;
mod uri;
mod view;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagerTab {
    General,
    Auth,
    Options,
    Tls,
    Pool,
    Advanced,
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
