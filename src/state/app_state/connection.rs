//! Connection management for AppState.

use gpui::Context;

use super::AppState;
use crate::components::TreeNodeId;
use crate::models::connection::SavedConnection;
use crate::state::ActiveTab;
use crate::state::View;
use crate::state::events::AppEvent;
use uuid::Uuid;

impl AppState {
    /// Reset all runtime state (used on connect/disconnect)
    pub(crate) fn reset_runtime_state(&mut self) {
        self.conn.selected_database = None;
        self.conn.selected_collection = None;
        self.tabs.open.clear();
        self.tabs.active = ActiveTab::None;
        self.tabs.preview = None;
        self.tabs.dirty.clear();
        self.sessions.clear();
        self.db_sessions.clear();
    }

    /// Add a new connection and persist to disk
    pub fn add_connection(&mut self, connection: SavedConnection, cx: &mut Context<Self>) {
        self.connections.push(connection);
        self.save_connections();
        cx.emit(AppEvent::ConnectionAdded);
    }

    pub fn update_connection(&mut self, connection: SavedConnection, cx: &mut Context<Self>) {
        let mut updated = false;
        let mut uri_changed = false;
        for existing in &mut self.connections {
            if existing.id == connection.id {
                uri_changed = existing.uri != connection.uri;
                *existing = connection.clone();
                updated = true;
                break;
            }
        }

        if !updated {
            self.add_connection(connection, cx);
            return;
        }

        if let Some(active) = self.conn.active.as_mut()
            && active.config.id == connection.id
        {
            active.config = connection.clone();
            if uri_changed {
                self.conn.active = None;
                self.reset_runtime_state();
                self.current_view = View::Welcome;
                let event = AppEvent::Disconnected(connection.id);
                self.update_status_from_event(&event);
                cx.emit(event);
                cx.emit(AppEvent::ViewChanged);
            }
        }

        self.save_connections();
        let event = AppEvent::ConnectionUpdated;
        self.update_status_from_event(&event);
        cx.emit(event);
        self.update_workspace_from_state();
        cx.notify();
    }

    pub fn remove_connection(&mut self, connection_id: Uuid, cx: &mut Context<Self>) {
        let was_active =
            self.conn.active.as_ref().is_some_and(|conn| conn.config.id == connection_id);

        self.connections.retain(|conn| conn.id != connection_id);
        self.save_connections();

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
            self.conn.active = None;
            self.reset_runtime_state();
            self.current_view = View::Welcome;
            let event = AppEvent::Disconnected(connection_id);
            self.update_status_from_event(&event);
            cx.emit(event);
            cx.emit(AppEvent::ViewChanged);
        }

        let event = AppEvent::ConnectionRemoved;
        self.update_status_from_event(&event);
        cx.emit(event);
        self.update_workspace_from_state();
        cx.notify();
    }

    /// Save connections to disk
    pub(super) fn save_connections(&self) {
        if let Err(e) = self.config.save_connections(&self.connections) {
            log::error!("Failed to save connections: {}", e);
        }
    }

    // Disconnect functionality is not wired yet.
}
