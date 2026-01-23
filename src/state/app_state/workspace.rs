//! Workspace persistence helpers for AppState.

use gpui::Context;

use crate::state::{AppEvent, WindowState};
use uuid::Uuid;

use super::AppState;
use super::types::{ActiveTab, TabKey, View};

impl AppState {
    pub fn workspace_autoconnect_id(&self) -> Option<Uuid> {
        if self.workspace_restore_pending { self.workspace.last_connection_id } else { None }
    }

    pub fn set_workspace_expanded_nodes(&mut self, nodes: Vec<String>) {
        if self.workspace.expanded_nodes != nodes {
            self.workspace.expanded_nodes = nodes;
            self.save_workspace();
        }
    }

    pub fn set_workspace_window_bounds(&mut self, bounds: gpui::WindowBounds) {
        let window_state = WindowState::from_bounds(bounds);
        if self.workspace.window_state.as_ref() != Some(&window_state) {
            self.workspace.window_state = Some(window_state);
            self.save_workspace();
        }
    }

    pub fn update_workspace_from_state(&mut self) {
        if self.workspace_restore_pending {
            return;
        }
        let last_connection_id =
            self.conn.selected_connection.or(self.workspace.last_connection_id);
        self.workspace.last_connection_id = last_connection_id;

        self.update_workspace_tabs();

        self.save_workspace();
    }

    pub fn restore_workspace_after_connect(&mut self, cx: &mut Context<Self>) {
        if !self.workspace_restore_pending {
            return;
        }
        let Some(connection_id) = self.workspace.last_connection_id else {
            return;
        };
        let Some(active) = self.conn.active.get(&connection_id) else {
            return;
        };

        let databases = active.databases.clone();
        let active_tab = self.restore_tabs_from_workspace(connection_id, &databases);

        if let Some(active_index) = active_tab {
            self.tabs.active = ActiveTab::Index(active_index);
            if let Some(tab) = self.tabs.open.get(active_index).cloned() {
                match tab {
                    TabKey::Collection(key) => {
                        self.conn.selected_connection = Some(connection_id);
                        self.conn.selected_database = Some(key.database.clone());
                        self.conn.selected_collection = Some(key.collection.clone());
                        self.current_view = View::Documents;
                    }
                    TabKey::Database(key) => {
                        self.conn.selected_connection = Some(connection_id);
                        self.conn.selected_database = Some(key.database.clone());
                        self.conn.selected_collection = None;
                        self.current_view = View::Database;
                    }
                }
            }
        } else if let Some(selected_db) = self.workspace.selected_database.clone() {
            if databases.contains(&selected_db) {
                self.conn.selected_connection = Some(connection_id);
                self.conn.selected_database = Some(selected_db);
                self.conn.selected_collection = self.workspace.selected_collection.clone();
                self.current_view = if self.conn.selected_collection.is_some() {
                    View::Documents
                } else {
                    View::Collections
                };
            } else {
                self.conn.selected_database = None;
                self.conn.selected_collection = None;
                self.current_view = View::Databases;
            }
        } else {
            self.conn.selected_database = None;
            self.conn.selected_collection = None;
            self.current_view = View::Databases;
        }

        self.workspace_restore_pending = false;
        self.update_workspace_from_state();
        cx.emit(AppEvent::ViewChanged);
        cx.notify();
    }

    fn save_workspace(&self) {
        if let Err(err) = self.config.save_workspace(&self.workspace) {
            log::error!("Failed to save workspace: {err}");
        }
    }
}
