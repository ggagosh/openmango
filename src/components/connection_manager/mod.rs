use gpui::{Entity, Subscription};
use gpui_component::input::InputState;
use uuid::Uuid;

use crate::state::AppState;

mod actions;
mod connection_list;
pub(crate) mod export_dialog;
pub(crate) mod import;
mod state;
mod tabs;
mod uri;
mod view;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagerTab {
    General,
    Tls,
    Network,
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
    ssh_host_state: Entity<InputState>,
    ssh_port_state: Entity<InputState>,
    ssh_username_state: Entity<InputState>,
    ssh_password_state: Entity<InputState>,
    ssh_identity_file_state: Entity<InputState>,
    ssh_identity_passphrase_state: Entity<InputState>,
    ssh_local_bind_host_state: Entity<InputState>,
    proxy_host_state: Entity<InputState>,
    proxy_port_state: Entity<InputState>,
    proxy_username_state: Entity<InputState>,
    proxy_password_state: Entity<InputState>,
    read_only: bool,
    direct_connection: bool,
    tls: bool,
    tls_insecure: bool,
    ssh_enabled: bool,
    ssh_use_identity_file: bool,
    ssh_strict_host_key_checking: bool,
    proxy_enabled: bool,
    pool_expanded: bool,
    compression_expanded: bool,
}

pub struct ConnectionManager {
    state: Entity<AppState>,
    selected_id: Option<Uuid>,
    draft: ConnectionDraft,
    testing_step: Option<String>,
    active_tab: ManagerTab,
    creating_new: bool,
    status: TestStatus,
    last_tested_uri: Option<String>,
    pending_test_uri: Option<String>,
    parse_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}
