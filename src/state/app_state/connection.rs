//! Connection management for AppState.

use std::collections::HashMap;

use anyhow::Result;
use gpui::{App, AppContext as _, Context, Task};
use uuid::Uuid;

use super::AppState;
use crate::helpers::keystore::KeyStore;
use crate::helpers::validate::{
    UriSecrets, extract_uri_secrets, inject_uri_secrets, strip_uri_secrets,
};
use crate::models::TreeNodeId;
use crate::models::{ActiveConnection, SavedConnection};
use crate::state::ActiveTab;
use crate::state::AppCommands;
use crate::state::View;
use crate::state::events::AppEvent;

pub(crate) const LEGACY_CONNECTION_SECRET_KEYS: &[&str] =
    &["uri", "uri-query", "ssh", "ssh-passphrase", "proxy"];
const SECRET_BUNDLE_PREFIX: &str = "bundle-v1-";

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct ConnectionSecrets {
    #[serde(default)]
    pub uri: UriSecrets,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_identity_passphrase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_password: Option<String>,
}

impl ConnectionSecrets {
    pub(crate) fn from_connection(connection: &SavedConnection) -> Self {
        Self {
            uri: extract_uri_secrets(&connection.uri),
            ssh_password: connection.ssh.as_ref().and_then(|ssh| ssh.password.clone()),
            ssh_identity_passphrase: connection
                .ssh
                .as_ref()
                .and_then(|ssh| ssh.identity_passphrase.clone()),
            proxy_password: connection.proxy.as_ref().and_then(|proxy| proxy.password.clone()),
        }
    }

    pub(crate) fn apply_to(&self, connection: &mut SavedConnection) {
        connection.uri = inject_uri_secrets(&strip_uri_secrets(&connection.uri), &self.uri);
        if let Some(ssh) = &mut connection.ssh {
            ssh.password.clone_from(&self.ssh_password);
            ssh.identity_passphrase.clone_from(&self.ssh_identity_passphrase);
        }
        if let Some(proxy) = &mut connection.proxy {
            proxy.password.clone_from(&self.proxy_password);
        }
    }
}

pub(crate) fn connection_secret_bundle_key(secret_id: Uuid) -> String {
    format!("{SECRET_BUNDLE_PREFIX}{secret_id}")
}

fn write_conn_secret_bundle(cx: &App, connection: &SavedConnection) -> Task<Result<()>> {
    let Some(secret_id) = connection.secret_id else {
        return cx.spawn(async move |_cx| Err(anyhow::anyhow!("missing connection secret id")));
    };
    let payload = match serde_json::to_string(&ConnectionSecrets::from_connection(connection)) {
        Ok(payload) => payload,
        Err(error) => return cx.spawn(async move |_cx| Err(error.into())),
    };
    KeyStore::write_conn(cx, connection.id, &connection_secret_bundle_key(secret_id), &payload)
}

fn delete_conn_secret_bundle(cx: &App, connection_id: Uuid, secret_id: Uuid) -> Task<Result<()>> {
    KeyStore::delete_conn(cx, connection_id, &connection_secret_bundle_key(secret_id))
}

fn delete_legacy_conn_secrets(cx: &App, id: Uuid) -> Vec<Task<Result<()>>> {
    LEGACY_CONNECTION_SECRET_KEYS.iter().map(|key| KeyStore::delete_conn(cx, id, key)).collect()
}

impl AppState {
    pub fn connections_snapshot(&self) -> Vec<SavedConnection> {
        self.connections.clone()
    }

    pub fn connection_by_id(&self, connection_id: Uuid) -> Option<&SavedConnection> {
        self.connections.iter().find(|conn| conn.id == connection_id)
    }

    pub fn connection_name(&self, connection_id: Uuid) -> Option<String> {
        self.connection_by_id(connection_id).map(|conn| conn.name.clone())
    }

    pub fn connection_uri(&self, connection_id: Uuid) -> Option<String> {
        self.connection_by_id(connection_id).map(|conn| conn.uri.clone())
    }

    pub fn active_connection_tool_uri(&self, connection_id: Uuid) -> crate::error::Result<String> {
        let active = self.active_connection_by_id(connection_id).ok_or_else(|| {
            crate::error::Error::Parse("The connection is not active".to_string())
        })?;
        self.connection_manager
            .effective_uri_for_active_connection(&active.config, &active.runtime_meta)
    }

    pub fn active_connections_snapshot(&self) -> HashMap<Uuid, ActiveConnection> {
        self.conn.active.clone()
    }

    pub fn active_connection_by_id(&self, connection_id: Uuid) -> Option<&ActiveConnection> {
        self.conn.active.get(&connection_id)
    }

    pub(crate) fn active_connection_mut(
        &mut self,
        connection_id: Uuid,
    ) -> Option<&mut ActiveConnection> {
        self.conn.active.get_mut(&connection_id)
    }

    pub(crate) fn insert_active_connection(
        &mut self,
        connection_id: Uuid,
        connection: ActiveConnection,
    ) -> Option<ActiveConnection> {
        self.conn.active.insert(connection_id, connection)
    }

    pub(crate) fn remove_active_connection(
        &mut self,
        connection_id: Uuid,
    ) -> Option<ActiveConnection> {
        self.conn.active.remove(&connection_id)
    }

    pub fn active_connection_client(&self, connection_id: Uuid) -> Option<mongodb::Client> {
        self.conn.active.get(&connection_id).map(|conn| conn.client.clone())
    }

    pub fn connection_read_only(&self, connection_id: Uuid) -> bool {
        self.conn.active.get(&connection_id).map(|conn| conn.config.read_only).unwrap_or_else(
            || self.connection_by_id(connection_id).is_some_and(|connection| connection.read_only),
        )
    }

    pub fn connection_history_enabled(&self, connection_id: Uuid) -> bool {
        self.connection_by_id(connection_id).is_some_and(|connection| connection.history_enabled)
    }

    pub fn connection_requires_production_write_confirmation(&self, connection_id: Uuid) -> bool {
        self.connection_by_id(connection_id)
            .is_some_and(SavedConnection::requires_production_write_confirmation)
    }

    pub fn authorize_production_writes(&mut self, connection_id: Uuid, uses: usize) {
        if uses > 0 {
            *self.production_write_authorizations.entry(connection_id).or_default() += uses;
        }
    }

    pub fn authorize_next_production_write(&mut self, connection_id: Uuid) {
        self.authorize_production_writes(connection_id, 1);
    }

    pub(crate) fn has_production_write_authorizations(
        &self,
        connection_id: Uuid,
        uses: usize,
    ) -> bool {
        self.production_write_authorizations.get(&connection_id).copied().unwrap_or(0) >= uses
    }

    pub(crate) fn revoke_production_write_authorizations(
        &mut self,
        connection_id: Uuid,
        uses: usize,
    ) {
        let Some(remaining) = self.production_write_authorizations.get_mut(&connection_id) else {
            return;
        };
        *remaining = remaining.saturating_sub(uses);
        if *remaining == 0 {
            self.production_write_authorizations.remove(&connection_id);
        }
    }

    pub(crate) fn consume_production_write_authorizations(
        &mut self,
        connection_id: Uuid,
        uses: usize,
    ) -> bool {
        if uses == 0 {
            return true;
        }
        let Some(remaining) = self.production_write_authorizations.get_mut(&connection_id) else {
            return false;
        };
        if *remaining < uses {
            return false;
        }
        *remaining -= uses;
        if *remaining == 0 {
            self.production_write_authorizations.remove(&connection_id);
        }
        true
    }

    pub(crate) fn consume_production_write_authorization(&mut self, connection_id: Uuid) -> bool {
        self.consume_production_write_authorizations(connection_id, 1)
    }

    pub fn is_connected(&self, connection_id: Uuid) -> bool {
        self.conn.active.contains_key(&connection_id)
    }

    pub fn has_active_connections(&self) -> bool {
        !self.conn.active.is_empty()
    }

    pub fn selected_connection_id(&self) -> Option<Uuid> {
        self.conn.selected_connection
    }

    pub(crate) fn selected_connection_is(&self, connection_id: Uuid) -> bool {
        self.conn.selected_connection == Some(connection_id)
    }

    pub(crate) fn set_selected_database_name(&mut self, database: Option<String>) {
        self.conn.selected_database = database;
    }

    pub(crate) fn set_selected_collection_name(&mut self, collection: Option<String>) {
        self.conn.selected_collection = collection;
    }

    pub fn selected_database(&self) -> Option<&str> {
        self.conn.selected_database.as_deref()
    }

    pub fn selected_collection(&self) -> Option<&str> {
        self.conn.selected_collection.as_deref()
    }

    pub fn selected_database_name(&self) -> Option<String> {
        self.conn.selected_database.clone()
    }

    pub fn selected_collection_name(&self) -> Option<String> {
        self.conn.selected_collection.clone()
    }

    pub(crate) fn set_selected_connection_internal(&mut self, connection_id: Uuid) {
        if self.conn.selected_connection == Some(connection_id) {
            return;
        }
        if let Some(current) = self.conn.selected_connection {
            self.conn.selection_cache.insert(
                current,
                (self.conn.selected_database.clone(), self.conn.selected_collection.clone()),
            );
        }
        self.conn.selected_connection = Some(connection_id);
    }
    pub fn active_connection(&self) -> Option<&crate::models::ActiveConnection> {
        let selected = self.conn.selected_connection?;
        self.conn.active.get(&selected)
    }

    pub fn select_connection(&mut self, connection_id: Option<Uuid>, cx: &mut Context<Self>) {
        if self.conn.selected_connection == connection_id {
            return;
        }

        if let Some(current) = self.conn.selected_connection {
            self.conn.selection_cache.insert(
                current,
                (self.conn.selected_database.clone(), self.conn.selected_collection.clone()),
            );
        }

        self.conn.selected_connection = connection_id;
        if let Some(next) = connection_id {
            if let Some((db, col)) = self.conn.selection_cache.get(&next).cloned() {
                self.conn.selected_database = db;
                self.conn.selected_collection = col;
            } else {
                self.conn.selected_database = None;
                self.conn.selected_collection = None;
            }
        } else {
            self.conn.selected_database = None;
            self.conn.selected_collection = None;
        }

        self.current_view = if let Some(conn_id) = connection_id {
            if self.conn.active.contains_key(&conn_id) {
                if self.conn.selected_collection.is_some() {
                    View::Documents
                } else if self.conn.selected_database.is_some() {
                    View::Collections
                } else {
                    View::Databases
                }
            } else {
                View::Welcome
            }
        } else {
            View::Welcome
        };

        cx.emit(AppEvent::ViewChanged);
        cx.notify();
    }

    pub(crate) fn reset_connection_runtime_state(
        &mut self,
        connection_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        let indices: Vec<usize> = self
            .tabs
            .open
            .iter()
            .enumerate()
            .filter(|(_, tab)| match tab {
                super::types::TabKey::Collection(tab) => tab.connection_id == connection_id,
                super::types::TabKey::Database(tab) => tab.connection_id == connection_id,
                super::types::TabKey::Transfer(tab) => tab.connection_id == Some(connection_id),
                super::types::TabKey::Forge(tab) => tab.connection_id == connection_id,
                super::types::TabKey::AgentActivity
                | super::types::TabKey::Connections
                | super::types::TabKey::Settings
                | super::types::TabKey::Changelog => false,
            })
            .map(|(idx, _)| idx)
            .collect();

        for index in indices.into_iter().rev() {
            self.close_tab(index, cx);
        }

        if let Some(tab) = self.tabs.preview.clone()
            && tab.connection_id == connection_id
        {
            self.close_preview_tab(cx);
        }

        self.tabs.dirty.retain(|key| key.connection_id != connection_id);
        self.sessions.remove_connection(connection_id);
        self.invalid_inline_edits.retain(|session_key| session_key.connection_id != connection_id);
        self.db_sessions.remove_connection(connection_id);
        self.forge_schema.retain(|k, _| k.connection_id != connection_id);
        self.forge_schema_inflight.retain(|k| k.connection_id != connection_id);
        self.evict_collection_meta_for_connection(connection_id);

        if self.conn.selected_connection == Some(connection_id) {
            self.conn.selected_connection = None;
            self.conn.selected_database = None;
            self.conn.selected_collection = None;
        }
    }

    fn should_queue_connection(&mut self, connection_id: Uuid) -> bool {
        if self.connection_secrets_ready() {
            return false;
        }
        self.connections_waiting_for_secret_sync.insert(connection_id);
        true
    }

    pub(crate) fn take_connections_waiting_for_secrets(&mut self) -> Vec<Uuid> {
        if !self.connection_secrets_ready() {
            return Vec::new();
        }
        std::mem::take(&mut self.connections_waiting_for_secret_sync)
            .into_iter()
            .filter(|connection_id| {
                self.connections.iter().any(|connection| connection.id == *connection_id)
            })
            .collect()
    }

    pub fn connect_when_secrets_ready(&mut self, connection_id: Uuid, cx: &mut Context<Self>) {
        if self.should_queue_connection(connection_id) {
            return;
        }
        AppCommands::connect(cx.entity(), connection_id, cx);
    }

    /// Add connections in memory immediately, but persist only after keychain success.
    pub fn add_connection(&mut self, connection: SavedConnection, cx: &mut Context<Self>) {
        self.add_connections(vec![connection], cx);
    }

    pub fn add_connections(
        &mut self,
        mut connections: Vec<SavedConnection>,
        cx: &mut Context<Self>,
    ) {
        if !self.connection_changes_allowed(cx) || connections.is_empty() {
            return;
        }
        let rollback = self.connections.clone();
        for connection in &mut connections {
            connection.secret_id = Some(Uuid::new_v4());
        }
        let count = connections.len();
        self.connections.extend(connections);
        for _ in 0..count {
            cx.emit(AppEvent::ConnectionAdded);
        }
        cx.notify();
        self.sync_connection_secrets(rollback, cx);
    }

    pub fn update_connection(&mut self, mut connection: SavedConnection, cx: &mut Context<Self>) {
        if !self.connection_changes_allowed(cx) {
            return;
        }
        let rollback = self.connections.clone();
        if let Some(existing) = self.connection_by_id(connection.id) {
            apply_agent_sharing_safety(existing, &mut connection);
        }
        connection.secret_id = Some(Uuid::new_v4());
        self.finish_update_connection(connection, cx);
        self.sync_connection_secrets(rollback, cx);
    }

    pub fn set_connection_agent_shared(
        &mut self,
        connection_id: Uuid,
        shared: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.connection_changes_allowed(cx) {
            return;
        }
        let Some(index) = self.connections.iter().position(|item| item.id == connection_id) else {
            return;
        };
        if self.connections[index].agent_shared == shared {
            return;
        }
        let previous_writable = self.connections[index].agent_writable;
        self.connections[index].agent_shared = shared;
        if !shared {
            self.connections[index].agent_writable = false;
        }
        if let Err(error) = self.config.save_connections(&self.connections) {
            self.connections[index].agent_shared = !shared;
            self.connections[index].agent_writable = previous_writable;
            self.set_status_message(Some(crate::state::StatusMessage::error(format!(
                "Could not update agent sharing: {error}"
            ))));
            cx.notify();
            return;
        }
        cx.emit(AppEvent::ConnectionUpdated);
        cx.notify();
    }

    pub fn set_connection_agent_writable(
        &mut self,
        connection_id: Uuid,
        writable: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.connection_changes_allowed(cx) {
            return;
        }
        let Some(index) = self.connections.iter().position(|item| item.id == connection_id) else {
            return;
        };
        if writable && !self.connections[index].agent_shared {
            self.set_status_message(Some(crate::state::StatusMessage::error(
                "Share the connection before allowing agent writes.",
            )));
            cx.notify();
            return;
        }
        if self.connections[index].agent_writable == writable {
            return;
        }
        self.connections[index].agent_writable = writable;
        if let Err(error) = self.config.save_connections(&self.connections) {
            self.connections[index].agent_writable = !writable;
            self.set_status_message(Some(crate::state::StatusMessage::error(format!(
                "Could not update agent write access: {error}"
            ))));
            cx.notify();
            return;
        }
        cx.emit(AppEvent::ConnectionUpdated);
        cx.notify();
    }

    pub fn set_connection_history_enabled(
        &mut self,
        connection_id: Uuid,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.connection_changes_allowed(cx) {
            return;
        }
        let Some(index) = self.connections.iter().position(|item| item.id == connection_id) else {
            return;
        };
        if enabled
            && !self
                .history_eligibility(connection_id)
                .is_some_and(|report| report.status == crate::history::EligibilityStatus::Eligible)
        {
            self.set_status_message(Some(crate::state::StatusMessage::error(
                "Inspect History eligibility and enable pre/post images before recording.",
            )));
            cx.notify();
            return;
        }
        if self.connections[index].history_enabled == enabled {
            return;
        }
        self.connections[index].history_enabled = enabled;
        if let Err(error) = self.config.save_connections(&self.connections) {
            self.connections[index].history_enabled = !enabled;
            self.set_status_message(Some(crate::state::StatusMessage::error(format!(
                "Could not update History: {error}"
            ))));
            cx.notify();
            return;
        }
        if let Some(history) = self.history_service() {
            if enabled {
                if let Some(active) = self.active_connection_by_id(connection_id) {
                    let configuration = &self.connections[index];
                    history.start(crate::history::HistoryConnection {
                        id: connection_id,
                        name: configuration.name.clone(),
                        client: active.client.clone(),
                        databases: active.databases.clone(),
                        max_age_days: configuration.history_max_age_days,
                        max_bytes: configuration.history_max_bytes,
                    });
                }
            } else {
                history.stop(connection_id);
            }
        }
        cx.emit(AppEvent::ConnectionUpdated);
        cx.notify();
    }

    pub fn set_connection_history_retention(
        &mut self,
        connection_id: Uuid,
        max_age_days: u32,
        max_bytes: u64,
        cx: &mut Context<Self>,
    ) {
        if !self.connection_changes_allowed(cx) {
            return;
        }
        let Some(index) = self.connections.iter().position(|item| item.id == connection_id) else {
            return;
        };
        let previous = (
            self.connections[index].history_max_age_days,
            self.connections[index].history_max_bytes,
        );
        self.connections[index].history_max_age_days = max_age_days.max(1);
        self.connections[index].history_max_bytes = max_bytes.max(1);
        if let Err(error) = self.config.save_connections(&self.connections) {
            self.connections[index].history_max_age_days = previous.0;
            self.connections[index].history_max_bytes = previous.1;
            self.set_status_message(Some(crate::state::StatusMessage::error(format!(
                "Could not update History retention: {error}"
            ))));
            cx.notify();
            return;
        }
        if self.connections[index].history_enabled
            && let Some(history) = self.history_service()
            && let Some(active) = self.active_connection_by_id(connection_id)
        {
            let configuration = &self.connections[index];
            history.start(crate::history::HistoryConnection {
                id: connection_id,
                name: configuration.name.clone(),
                client: active.client.clone(),
                databases: active.databases.clone(),
                max_age_days: configuration.history_max_age_days,
                max_bytes: configuration.history_max_bytes,
            });
        }
        cx.emit(AppEvent::ConnectionUpdated);
        cx.notify();
    }

    fn finish_update_connection(&mut self, connection: SavedConnection, cx: &mut Context<Self>) {
        let mut updated = false;
        let mut transport_changed = false;
        for existing in &mut self.connections {
            if existing.id == connection.id {
                transport_changed = connection_transport_changed(existing, &connection);
                *existing = connection.clone();
                updated = true;
                break;
            }
        }

        if !updated {
            self.connections.push(connection);
            cx.emit(AppEvent::ConnectionAdded);
            cx.notify();
            return;
        }

        if let Some(active) = self.conn.active.get_mut(&connection.id) {
            active.config = connection.clone();
            if transport_changed {
                self.connection_manager().disconnect(connection.id);
                self.conn.active.remove(&connection.id);
                self.reset_connection_runtime_state(connection.id, cx);
                if self.conn.selected_connection == Some(connection.id) {
                    self.current_view = View::Welcome;
                    cx.emit(AppEvent::ViewChanged);
                }
                let event = AppEvent::Disconnected(connection.id);
                self.update_status_from_event(&event);
                cx.emit(event);
            }
        }

        let event = AppEvent::ConnectionUpdated;
        self.update_status_from_event(&event);
        cx.emit(event);
        self.update_workspace_from_state();
        cx.notify();
    }

    fn sync_connection_secrets(&mut self, rollback: Vec<SavedConnection>, cx: &mut Context<Self>) {
        self.connection_secret_sync_pending = true;
        let tasks: Vec<_> = self
            .connections
            .iter()
            .map(|connection| write_conn_secret_bundle(cx, connection))
            .collect();
        let stale_bundles: Vec<_> = rollback
            .iter()
            .filter_map(|old| {
                let old_secret = old.secret_id?;
                let current_secret = self
                    .connections
                    .iter()
                    .find(|connection| connection.id == old.id)
                    .and_then(|connection| connection.secret_id);
                (current_secret != Some(old_secret)).then_some((old.id, old_secret))
            })
            .collect();
        let candidate_bundles: Vec<_> = self
            .connections
            .iter()
            .filter_map(|connection| {
                let secret_id = connection.secret_id?;
                let old_secret = rollback
                    .iter()
                    .find(|old| old.id == connection.id)
                    .and_then(|old| old.secret_id);
                (old_secret != Some(secret_id)).then_some((connection.id, secret_id))
            })
            .collect();
        let state = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let mut result = Ok(());
            for task in tasks {
                if let Err(error) = task.await
                    && result.is_ok()
                {
                    result = Err(error);
                }
            }
            let _ = cx.update(|cx| {
                let connection_ids = state.update(cx, |state, cx| match result {
                    Ok(()) => match state.config.save_connections(&state.connections) {
                        Ok(()) => {
                            state.connection_secret_sync_pending = false;
                            state.cleanup_secret_bundles(stale_bundles, cx);
                            state.take_connections_waiting_for_secrets()
                        }
                        Err(error) => {
                            state.connection_secret_sync_pending = false;
                            state.connections_waiting_for_secret_sync.clear();
                            state.restore_connections_after_secret_failure(
                                rollback,
                                &candidate_bundles,
                                cx,
                            );
                            state.cleanup_secret_bundles(candidate_bundles, cx);
                            state.report_secret_store_error(error, cx);
                            Vec::new()
                        }
                    },
                    Err(error) => {
                        state.connection_secret_sync_pending = false;
                        state.connections_waiting_for_secret_sync.clear();
                        state.restore_connections_after_secret_failure(
                            rollback,
                            &candidate_bundles,
                            cx,
                        );
                        state.cleanup_secret_bundles(candidate_bundles, cx);
                        state.report_secret_store_error(error, cx);
                        Vec::new()
                    }
                });
                for connection_id in connection_ids {
                    AppCommands::connect(state.clone(), connection_id, cx);
                }
            });
        })
        .detach();
    }

    fn restore_connections_after_secret_failure(
        &mut self,
        rollback: Vec<SavedConnection>,
        candidate_bundles: &[(Uuid, Uuid)],
        cx: &mut Context<Self>,
    ) {
        for (connection_id, _) in candidate_bundles {
            if self.conn.active.remove(connection_id).is_some() {
                self.connection_manager().disconnect(*connection_id);
                self.reset_connection_runtime_state(*connection_id, cx);
                cx.emit(AppEvent::Disconnected(*connection_id));
            }
        }
        self.connections = rollback;
        if self
            .conn
            .selected_connection
            .is_some_and(|selected| !self.connections.iter().any(|conn| conn.id == selected))
        {
            self.conn.selected_connection = None;
            self.current_view = View::Welcome;
            cx.emit(AppEvent::ViewChanged);
        }
        cx.notify();
    }

    fn cleanup_secret_bundles(&mut self, bundles: Vec<(Uuid, Uuid)>, cx: &mut Context<Self>) {
        if bundles.is_empty() {
            return;
        }
        let tasks: Vec<_> = bundles
            .into_iter()
            .map(|(connection_id, secret_id)| {
                delete_conn_secret_bundle(cx, connection_id, secret_id)
            })
            .collect();
        let state = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let mut first_error = None;
            for task in tasks {
                if let Err(error) = task.await
                    && first_error.is_none()
                {
                    first_error = Some(error);
                }
            }
            if let Some(error) = first_error {
                let _ = cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        state.report_secret_store_error(error, cx);
                    });
                });
            }
        })
        .detach();
    }

    fn report_secret_store_error(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        let message = format!("Could not store connection credentials: {error}");
        log::error!("{message}");
        self.set_status_message(Some(crate::state::StatusMessage::error(message)));
        cx.notify();
    }

    pub(crate) fn report_connection_secret_error(
        &mut self,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        self.report_secret_store_error(error, cx);
    }

    pub fn remove_connection(&mut self, connection_id: Uuid, cx: &mut Context<Self>) {
        if !self.connection_changes_allowed(cx) {
            return;
        }
        let Some(connection) =
            self.connections.iter().find(|conn| conn.id == connection_id).cloned()
        else {
            return;
        };
        let remaining: Vec<_> =
            self.connections.iter().filter(|conn| conn.id != connection_id).cloned().collect();
        if let Err(error) = self.config.save_connections(&remaining) {
            self.report_secret_store_error(error, cx);
            return;
        }
        self.finish_remove_connection(connection_id, cx);

        let mut tasks = delete_legacy_conn_secrets(cx, connection_id);
        if let Some(secret_id) = connection.secret_id {
            tasks.push(delete_conn_secret_bundle(cx, connection_id, secret_id));
        }
        let state = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let mut first_error = None;
            for task in tasks {
                if let Err(error) = task.await
                    && first_error.is_none()
                {
                    first_error = Some(error);
                }
            }
            if let Some(error) = first_error {
                let _ = cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        state.report_secret_store_error(error, cx);
                    });
                });
            }
        })
        .detach();
    }

    fn finish_remove_connection(&mut self, connection_id: Uuid, cx: &mut Context<Self>) {
        let was_active = self.conn.active.contains_key(&connection_id);

        self.connections.retain(|conn| conn.id != connection_id);

        if self.workspace.last_connection_id == Some(connection_id) {
            self.workspace.last_connection_id = None;
        }

        let mut expanded = self.workspace.expanded_nodes.clone();
        expanded.retain(|node_id| {
            TreeNodeId::from_tree_id(node_id)
                .map(|node| node.connection_id() != connection_id)
                .unwrap_or(true)
        });
        self.set_workspace_expanded_nodes(expanded);

        if was_active {
            self.connection_manager().disconnect(connection_id);
            self.conn.active.remove(&connection_id);
            self.reset_connection_runtime_state(connection_id, cx);
            if self.conn.selected_connection == Some(connection_id) {
                self.current_view = View::Welcome;
                cx.emit(AppEvent::ViewChanged);
            }
            let event = AppEvent::Disconnected(connection_id);
            self.update_status_from_event(&event);
            cx.emit(event);
        }

        let event = AppEvent::ConnectionRemoved;
        self.update_status_from_event(&event);
        cx.emit(event);
        self.update_workspace_from_state();
        cx.notify();
    }

    fn connection_changes_allowed(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.connections_persistence_blocked && !self.connection_secret_sync_pending {
            return true;
        }
        self.set_status_message(Some(crate::state::StatusMessage::error(
            "Connection changes are blocked until credential storage or config recovery finishes.",
        )));
        cx.notify();
        false
    }

    pub(crate) fn begin_connection_secret_migration(&mut self) {
        self.connections_persistence_blocked = true;
    }

    pub(crate) fn complete_connection_secret_startup(
        &mut self,
        connections: Vec<SavedConnection>,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.config.save_connections(&connections) {
            Ok(()) => {
                self.connections = connections;
                self.connections_persistence_blocked = false;
                cx.notify();
                true
            }
            Err(error) => {
                self.connections_persistence_blocked = true;
                self.report_secret_store_error(error, cx);
                false
            }
        }
    }

    pub(crate) fn finish_connection_secret_hydration(
        &mut self,
        result: Result<()>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(()) => self.connections_persistence_blocked = false,
            Err(error) => {
                self.connections_persistence_blocked = true;
                self.report_secret_store_error(error, cx);
            }
        }
    }

    pub(crate) fn connection_secrets_ready(&self) -> bool {
        !self.connections_persistence_blocked && !self.connection_secret_sync_pending
    }

    // Disconnect functionality is not wired yet.
}

fn connection_transport_changed(existing: &SavedConnection, updated: &SavedConnection) -> bool {
    existing.uri != updated.uri || existing.ssh != updated.ssh || existing.proxy != updated.proxy
}

fn apply_agent_sharing_safety(existing: &SavedConnection, updated: &mut SavedConnection) {
    let became_protected = !existing.protected && updated.protected;
    let became_production = existing.environment
        != Some(crate::models::ConnectionEnvironment::Production)
        && updated.environment == Some(crate::models::ConnectionEnvironment::Production);
    let identity_changed = existing.uri != updated.uri
        || existing.ssh != updated.ssh
        || existing.proxy != updated.proxy;
    if became_protected || became_production || identity_changed {
        updated.agent_shared = false;
        updated.agent_writable = false;
    } else if !updated.agent_shared {
        updated.agent_writable = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_autoconnect_waits_for_startup_credential_hydration() {
        let connection_id = Uuid::new_v4();
        let mut state = AppState::new();
        state.connections_persistence_blocked = true;
        state.connection_secret_sync_pending = false;

        assert!(state.should_queue_connection(connection_id));
        assert!(state.connections_waiting_for_secret_sync.contains(&connection_id));

        let mut connection =
            SavedConnection::new("workspace".into(), "mongodb://user@localhost".into());
        connection.id = connection_id;
        state.connections = vec![connection];
        state.connections_persistence_blocked = false;

        assert_eq!(state.take_connections_waiting_for_secrets(), vec![connection_id]);
        assert!(state.connections_waiting_for_secret_sync.is_empty());
    }

    #[test]
    fn history_is_scoped_per_connection_and_defaults_off() {
        let mut state = AppState::new();
        state.connections.clear();
        let disabled = SavedConnection::new("Disabled".into(), "mongodb://disabled".into());
        let disabled_id = disabled.id;
        let mut enabled = SavedConnection::new("Enabled".into(), "mongodb://enabled".into());
        enabled.history_enabled = true;
        let enabled_id = enabled.id;
        state.connections = vec![disabled, enabled];

        assert!(!state.connection_history_enabled(disabled_id));
        assert!(state.connection_history_enabled(enabled_id));
    }

    #[test]
    fn sensitive_connection_changes_disable_agent_sharing() {
        let mut existing = SavedConnection::new("Local".into(), "mongodb://localhost".into());
        existing.agent_shared = true;
        existing.agent_writable = true;

        let mut updated = existing.clone();
        updated.protected = true;
        apply_agent_sharing_safety(&existing, &mut updated);
        assert!(!updated.agent_shared);
        assert!(!updated.agent_writable);

        let mut updated = existing.clone();
        updated.environment = Some(crate::models::ConnectionEnvironment::Production);
        apply_agent_sharing_safety(&existing, &mut updated);
        assert!(!updated.agent_shared);
        assert!(!updated.agent_writable);

        let mut updated = existing.clone();
        updated.uri = "mongodb://remote".into();
        apply_agent_sharing_safety(&existing, &mut updated);
        assert!(!updated.agent_shared);
        assert!(!updated.agent_writable);

        let mut updated = existing.clone();
        updated.agent_shared = false;
        apply_agent_sharing_safety(&existing, &mut updated);
        assert!(!updated.agent_writable);
    }

    #[test]
    fn connection_transport_changes_require_a_new_client() {
        let connection = SavedConnection::new("Local".into(), "mongodb://localhost".into());
        let mut updated = connection.clone();
        updated.name = "Renamed".into();
        updated.color = Some(crate::models::ConnectionColor::Blue);
        assert!(!connection_transport_changed(&connection, &updated));

        updated.ssh = Some(crate::models::SshConfig {
            enabled: true,
            host: "jump.example".into(),
            ..Default::default()
        });
        assert!(connection_transport_changed(&connection, &updated));

        updated = connection.clone();
        updated.proxy = Some(crate::models::ProxyConfig {
            enabled: true,
            host: "proxy.example".into(),
            ..Default::default()
        });
        assert!(connection_transport_changed(&connection, &updated));

        updated = connection.clone();
        updated.uri = "mongodb://remote".into();
        assert!(connection_transport_changed(&connection, &updated));
    }

    #[test]
    fn secret_bundle_round_trips_all_connection_credentials() {
        let mut connection = SavedConnection::new(
            "bundle".into(),
            "mongodb://user:*****@host/db?tlsCertificateKeyFilePassword=tls&proxyPassword=uri-proxy&authMechanismProperties=AWS_SESSION_TOKEN%3Aaws".into(),
        );
        connection.secret_id = Some(Uuid::new_v4());
        connection.ssh = Some(crate::models::SshConfig {
            password: Some("ssh".into()),
            identity_passphrase: Some("identity".into()),
            ..crate::models::SshConfig::default()
        });
        connection.proxy = Some(crate::models::ProxyConfig {
            password: Some("modeled-proxy".into()),
            ..crate::models::ProxyConfig::default()
        });

        let bundle = ConnectionSecrets::from_connection(&connection);
        let mut persisted = connection.with_secrets_stripped();
        assert_eq!(persisted.uri, "mongodb://user@host/db");
        assert!(persisted.ssh.as_ref().unwrap().password.is_none());
        assert!(persisted.proxy.as_ref().unwrap().password.is_none());

        bundle.apply_to(&mut persisted);
        assert_eq!(persisted.uri, connection.uri);
        assert_eq!(persisted.ssh, connection.ssh);
        assert_eq!(persisted.proxy, connection.proxy);
        assert_eq!(persisted.secret_id, connection.secret_id);
    }
}
