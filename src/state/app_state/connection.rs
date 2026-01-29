//! Connection management for AppState.

use std::collections::HashMap;

use gpui::Context;

use super::AppState;
use crate::components::TreeNodeId;
use crate::models::{ActiveConnection, SavedConnection};
use crate::state::ActiveTab;
use crate::state::View;
use crate::state::events::AppEvent;
use uuid::Uuid;

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
        self.conn.active.get(&connection_id).map(|conn| conn.config.read_only).unwrap_or(false)
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
        self.db_sessions.remove_connection(connection_id);

        if self.conn.selected_connection == Some(connection_id) {
            self.conn.selected_connection = None;
            self.conn.selected_database = None;
            self.conn.selected_collection = None;
        }
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

        if let Some(active) = self.conn.active.get_mut(&connection.id) {
            active.config = connection.clone();
            if uri_changed {
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

        self.save_connections();
        let event = AppEvent::ConnectionUpdated;
        self.update_status_from_event(&event);
        cx.emit(event);
        self.update_workspace_from_state();
        cx.notify();
    }

    pub fn remove_connection(&mut self, connection_id: Uuid, cx: &mut Context<Self>) {
        let was_active = self.conn.active.contains_key(&connection_id);

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

    /// Save connections to disk
    pub(super) fn save_connections(&self) {
        if let Err(e) = self.config.save_connections(&self.connections) {
            log::error!("Failed to save connections: {}", e);
        }
    }

    // Disconnect functionality is not wired yet.
}
