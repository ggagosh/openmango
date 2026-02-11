//! Tab management for AppState.

use std::collections::HashSet;

use gpui::Context;
use uuid::Uuid;

use crate::state::events::AppEvent;
use crate::state::{AppState, StatusLevel};

use crate::state::app_state::types::{
    ActiveTab, DatabaseKey, ForgeTabKey, ForgeTabState, SessionKey, TabKey, TransferMode,
    TransferScope, TransferTabKey, TransferTabState, View,
};

#[derive(Debug, Clone, Copy)]
pub(in crate::state::app_state) enum TabOpenMode {
    Preview,
    Permanent,
}

impl AppState {
    pub fn open_tabs(&self) -> &[TabKey] {
        &self.tabs.open
    }

    pub fn preview_tab(&self) -> Option<&SessionKey> {
        self.tabs.preview.as_ref()
    }

    pub fn active_tab(&self) -> ActiveTab {
        self.tabs.active
    }

    pub fn dirty_tabs(&self) -> &HashSet<SessionKey> {
        &self.tabs.dirty
    }

    fn active_index(&self) -> Option<usize> {
        match self.tabs.active {
            ActiveTab::Index(index) => Some(index),
            _ => None,
        }
    }

    fn set_active_index(&mut self, index: usize) {
        self.tabs.active = ActiveTab::Index(index);
    }

    fn set_active_preview(&mut self) {
        self.tabs.active = ActiveTab::Preview;
    }

    fn clear_active(&mut self) {
        self.tabs.active = ActiveTab::None;
    }

    fn is_preview_active(&self) -> bool {
        matches!(self.tabs.active, ActiveTab::Preview)
    }

    fn clear_error_status(&mut self) {
        if matches!(self.status_message().as_ref().map(|m| &m.level), Some(StatusLevel::Error)) {
            self.clear_status_message();
        }
    }

    pub(super) fn cleanup_session(&mut self, key: &SessionKey) {
        let still_referenced = self.tabs.open.iter().any(|tab| matches_collection(tab, key))
            || self.tabs.preview.as_ref() == Some(key);
        if !still_referenced {
            self.sessions.remove(key);
        }
    }

    pub(super) fn ensure_session_loaded(&mut self, key: SessionKey) {
        self.ensure_session(key);
    }

    pub(in crate::state::app_state) fn open_collection_with_mode(
        &mut self,
        database: String,
        collection: String,
        mode: TabOpenMode,
        cx: &mut Context<Self>,
    ) {
        let Some(conn_id) = self.conn.selected_connection else {
            return;
        };
        if !self.conn.active.contains_key(&conn_id) {
            return;
        }
        let new_tab = SessionKey::new(conn_id, database.clone(), collection.clone());
        let existing_index =
            self.tabs.open.iter().position(|tab| matches_collection(tab, &new_tab));
        let selection_changed = self.conn.selected_database.as_ref() != Some(&database)
            || self.conn.selected_collection.as_ref() != Some(&collection);
        let mut tab_changed = false;

        match mode {
            TabOpenMode::Permanent => {
                let active_index = if let Some(index) = existing_index {
                    index
                } else {
                    self.tabs.open.push(TabKey::Collection(new_tab.clone()));
                    self.tabs.open.len() - 1
                };

                if self.active_index() != Some(active_index) {
                    self.set_active_index(active_index);
                    tab_changed = true;
                }

                if self.tabs.preview.as_ref() == Some(&new_tab) {
                    self.tabs.preview = None;
                    tab_changed = true;
                } else if self.is_preview_active() {
                    tab_changed = true;
                }

                self.set_selected_connection_internal(conn_id);
                self.conn.selected_database = Some(database);
                self.conn.selected_collection = Some(collection);
                self.current_view = View::Documents;
                self.ensure_session_loaded(new_tab);
                self.update_workspace_from_state();
            }
            TabOpenMode::Preview => {
                if let Some(index) = existing_index {
                    if self.active_index() != Some(index) {
                        self.set_active_index(index);
                        tab_changed = true;
                    }
                    if self.is_preview_active() {
                        tab_changed = true;
                    }

                    self.set_selected_connection_internal(conn_id);
                    self.conn.selected_database = Some(database);
                    self.conn.selected_collection = Some(collection);
                    self.current_view = View::Documents;
                    self.ensure_session_loaded(new_tab);
                } else {
                    if let Some(old_preview) = self.tabs.preview.clone()
                        && old_preview != new_tab
                    {
                        self.tabs.preview = None;
                        self.cleanup_session(&old_preview);
                        tab_changed = true;
                    }

                    if self.tabs.preview.as_ref() != Some(&new_tab) {
                        self.tabs.preview = Some(new_tab.clone());
                        tab_changed = true;
                    }

                    let was_preview_active = self.is_preview_active();
                    self.set_active_preview();
                    if !was_preview_active {
                        tab_changed = true;
                    }

                    self.set_selected_connection_internal(conn_id);
                    self.conn.selected_database = Some(database);
                    self.conn.selected_collection = Some(collection);
                    self.current_view = View::Documents;
                    self.ensure_session_loaded(new_tab);
                }
            }
        }

        if selection_changed || tab_changed {
            self.clear_error_status();
            cx.emit(AppEvent::ViewChanged);
        }
        cx.notify();
    }

    pub(in crate::state::app_state) fn open_database_tab(
        &mut self,
        database: String,
        cx: &mut Context<Self>,
    ) {
        let Some(conn_id) = self.conn.selected_connection else {
            return;
        };
        if !self.conn.active.contains_key(&conn_id) {
            return;
        }
        let key = DatabaseKey::new(conn_id, database.clone());
        let database_indices: Vec<usize> = self
            .tabs
            .open
            .iter()
            .enumerate()
            .filter_map(|(idx, tab)| matches!(tab, TabKey::Database(_)).then_some(idx))
            .collect();
        let existing_index = database_indices.first().copied();
        let selection_changed = self.conn.selected_database.as_ref() != Some(&database)
            || self.conn.selected_collection.is_some();
        let mut tab_changed = false;

        let active_index = if let Some(index) = existing_index {
            let previous_key = match &self.tabs.open[index] {
                TabKey::Database(key) => key.clone(),
                _ => key.clone(),
            };
            if previous_key != key {
                self.db_sessions.remove(&previous_key);
            }
            self.tabs.open[index] = TabKey::Database(key.clone());
            index
        } else {
            self.tabs.open.push(TabKey::Database(key.clone()));
            self.tabs.open.len() - 1
        };

        if database_indices.len() > 1 {
            let keep_index = database_indices[0];
            for index in database_indices.into_iter().rev() {
                if index == keep_index {
                    continue;
                }
                if let TabKey::Database(old_key) = self.tabs.open.remove(index) {
                    self.db_sessions.remove(&old_key);
                }
            }
        }

        if self.active_index() != Some(active_index) {
            self.set_active_index(active_index);
            tab_changed = true;
        }

        self.set_selected_connection_internal(conn_id);
        self.conn.selected_database = Some(database);
        self.conn.selected_collection = None;
        self.current_view = View::Database;
        self.ensure_database_session(key);
        self.update_workspace_from_state();

        if selection_changed || tab_changed {
            self.clear_error_status();
            cx.emit(AppEvent::ViewChanged);
        }
        cx.notify();
    }

    pub(crate) fn open_transfer_tab(&mut self, cx: &mut Context<Self>) {
        let connection_id = self.conn.selected_connection;
        let mut transfer_state = TransferTabState::from_settings(&self.settings);
        transfer_state.config.source_connection_id = connection_id;
        transfer_state.config.source_database =
            self.conn.selected_database.clone().unwrap_or_default();
        transfer_state.config.source_collection =
            self.conn.selected_collection.clone().unwrap_or_default();
        transfer_state.config.destination_connection_id = connection_id;
        transfer_state.config.destination_database = transfer_state.config.source_database.clone();
        transfer_state.config.destination_collection =
            transfer_state.config.source_collection.clone();
        if transfer_state.config.source_database.is_empty() {
            transfer_state.config.scope = TransferScope::Collection;
        } else if transfer_state.config.source_collection.is_empty() {
            transfer_state.config.scope = TransferScope::Database;
        } else {
            transfer_state.config.scope = TransferScope::Collection;
        }
        self.push_transfer_tab(transfer_state, connection_id, cx);
    }

    pub(crate) fn open_transfer_tab_with_prefill(
        &mut self,
        connection_id: Uuid,
        database: String,
        collection: Option<String>,
        scope: TransferScope,
        mode: TransferMode,
        cx: &mut Context<Self>,
    ) {
        let mut transfer_state = TransferTabState::from_settings(&self.settings);
        transfer_state.config.mode = mode;
        transfer_state.config.scope = scope;
        transfer_state.config.source_connection_id = Some(connection_id);
        transfer_state.config.source_database = database;
        transfer_state.config.source_collection = collection.unwrap_or_default();
        transfer_state.config.destination_connection_id = Some(connection_id);
        transfer_state.config.destination_database = transfer_state.config.source_database.clone();
        transfer_state.config.destination_collection =
            transfer_state.config.source_collection.clone();
        if scope == TransferScope::Database {
            transfer_state.config.source_collection.clear();
            transfer_state.config.destination_collection.clear();
        }

        self.push_transfer_tab(transfer_state, Some(connection_id), cx);
    }

    /// Open a Transfer tab for Copy mode with separate source and destination prefills.
    /// Used when pasting a copied tree item to a different location.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open_transfer_tab_for_paste(
        &mut self,
        source_connection_id: Uuid,
        source_database: String,
        source_collection: Option<String>,
        dest_connection_id: Option<Uuid>,
        dest_database: Option<String>,
        scope: TransferScope,
        cx: &mut Context<Self>,
    ) {
        let mut transfer_state = TransferTabState::from_settings(&self.settings);
        transfer_state.config.mode = TransferMode::Copy;
        transfer_state.config.scope = scope;
        transfer_state.config.source_connection_id = Some(source_connection_id);
        transfer_state.config.source_database = source_database.clone();
        transfer_state.config.source_collection = source_collection.clone().unwrap_or_default();

        // Set destination based on provided values, falling back to source
        transfer_state.config.destination_connection_id =
            dest_connection_id.or(Some(source_connection_id));
        transfer_state.config.destination_database =
            dest_database.unwrap_or_else(|| source_database.clone());
        // For collection scope, keep destination collection empty so user can fill it
        // For database scope, collection is not relevant
        transfer_state.config.destination_collection = if scope == TransferScope::Collection {
            // If copying a collection, suggest the same collection name at destination
            source_collection.unwrap_or_default()
        } else {
            String::new()
        };

        if scope == TransferScope::Database {
            transfer_state.config.source_collection.clear();
            transfer_state.config.destination_collection.clear();
        }

        self.push_transfer_tab(transfer_state, Some(source_connection_id), cx);
    }

    fn push_transfer_tab(
        &mut self,
        transfer_state: TransferTabState,
        connection_id: Option<Uuid>,
        cx: &mut Context<Self>,
    ) {
        let selected_database = if transfer_state.config.source_database.is_empty() {
            None
        } else {
            Some(transfer_state.config.source_database.clone())
        };
        let selected_collection = if transfer_state.config.source_collection.is_empty() {
            None
        } else {
            Some(transfer_state.config.source_collection.clone())
        };

        let id = Uuid::new_v4();
        let key = TransferTabKey { id, connection_id };
        self.transfer_tabs.insert(id, transfer_state);
        self.tabs.open.push(TabKey::Transfer(key.clone()));
        self.set_active_index(self.tabs.open.len() - 1);
        if let Some(conn_id) = connection_id {
            self.set_selected_connection_internal(conn_id);
        }
        self.conn.selected_database = selected_database;
        self.conn.selected_collection = selected_collection;
        self.current_view = View::Transfer;
        self.update_workspace_from_state();
        self.clear_error_status();
        cx.emit(AppEvent::ViewChanged);
        cx.notify();
    }

    /// Open settings tab (singleton - only one settings tab allowed)
    pub fn open_settings_tab(&mut self, cx: &mut Context<Self>) {
        // Check if settings tab already exists
        if let Some(index) = self.tabs.open.iter().position(|tab| matches!(tab, TabKey::Settings)) {
            // Settings tab already open, just select it
            if self.active_index() != Some(index) {
                self.set_active_index(index);
                self.current_view = View::Settings;
                self.clear_error_status();
                cx.emit(AppEvent::ViewChanged);
                cx.notify();
            }
            return;
        }

        // Add new settings tab
        self.tabs.open.push(TabKey::Settings);
        self.set_active_index(self.tabs.open.len() - 1);
        self.current_view = View::Settings;
        self.clear_error_status();
        cx.emit(AppEvent::ViewChanged);
        cx.notify();
    }

    /// Open a Forge query shell tab for a database (optionally prefilled for a collection).
    pub fn open_forge_tab(
        &mut self,
        connection_id: Uuid,
        database: String,
        collection: Option<String>,
        cx: &mut Context<Self>,
    ) {
        // Check if a Forge tab for this database already exists
        let existing_index = self.tabs.open.iter().position(|tab| {
            matches!(
                tab,
                TabKey::Forge(key)
                    if key.connection_id == connection_id && key.database == database
            )
        });

        if let Some(index) = existing_index {
            // Forge tab for this database already exists, select it
            if self.active_index() != Some(index) {
                self.set_active_index(index);
                self.set_selected_connection_internal(connection_id);
                self.conn.selected_database = Some(database);
                self.current_view = View::Forge;
                self.clear_error_status();
                cx.emit(AppEvent::ViewChanged);
                cx.notify();
            }
            return;
        }

        // Create new Forge tab
        let id = Uuid::new_v4();
        let key = ForgeTabKey { id, connection_id, database: database.clone() };
        let mut state = ForgeTabState::default();
        if let Some(collection) = collection {
            let escaped = collection.replace('"', "\\\"");
            let content = format!("db.getCollection(\"{}\").find({{}})", escaped);
            state.pending_cursor = content.rfind('{').map(|idx| idx + 1);
            state.content = content;
        }

        self.forge_tabs.insert(id, state);
        self.tabs.open.push(TabKey::Forge(key));
        self.set_active_index(self.tabs.open.len() - 1);
        self.set_selected_connection_internal(connection_id);
        self.conn.selected_database = Some(database);
        self.conn.selected_collection = None;
        self.current_view = View::Forge;
        self.update_workspace_from_state();
        self.clear_error_status();
        cx.emit(AppEvent::ViewChanged);
        cx.notify();
    }

    pub(in crate::state::app_state) fn close_database_tabs(&mut self, cx: &mut Context<Self>) {
        let Some(conn_id) = self.conn.selected_connection else {
            return;
        };
        let indices: Vec<usize> = self
            .tabs
            .open
            .iter()
            .enumerate()
            .filter(|(_, tab)| match tab {
                TabKey::Database(tab) => tab.connection_id == conn_id,
                _ => false,
            })
            .map(|(idx, _)| idx)
            .collect();
        for index in indices.into_iter().rev() {
            self.close_tab(index, cx);
        }
    }

    pub fn select_preview_tab(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.preview.clone() else {
            return;
        };
        self.set_active_preview();
        self.set_selected_connection_internal(tab.connection_id);
        self.conn.selected_database = Some(tab.database.clone());
        self.conn.selected_collection = Some(tab.collection.clone());
        self.current_view = View::Documents;
        self.ensure_session_loaded(tab);
        self.clear_error_status();
        cx.emit(AppEvent::ViewChanged);
        cx.notify();
    }

    pub fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.open.get(index).cloned() else {
            return;
        };
        self.set_active_index(index);
        match tab {
            TabKey::Collection(tab) => {
                self.set_selected_connection_internal(tab.connection_id);
                self.conn.selected_database = Some(tab.database.clone());
                self.conn.selected_collection = Some(tab.collection.clone());
                self.current_view = View::Documents;
                self.ensure_session_loaded(tab);
            }
            TabKey::Database(tab) => {
                self.set_selected_connection_internal(tab.connection_id);
                self.conn.selected_database = Some(tab.database.clone());
                self.conn.selected_collection = None;
                self.current_view = View::Database;
                self.ensure_database_session(tab);
            }
            TabKey::Transfer(tab) => {
                if let Some(conn_id) = tab.connection_id {
                    self.set_selected_connection_internal(conn_id);
                }
                if let Some(state) = self.transfer_tabs.get(&tab.id) {
                    if !state.config.source_database.is_empty() {
                        self.conn.selected_database = Some(state.config.source_database.clone());
                    }
                    if !state.config.source_collection.is_empty() {
                        self.conn.selected_collection =
                            Some(state.config.source_collection.clone());
                    }
                }
                self.current_view = View::Transfer;
            }
            TabKey::Settings => {
                self.current_view = View::Settings;
            }
            TabKey::Forge(tab) => {
                self.set_selected_connection_internal(tab.connection_id);
                self.conn.selected_database = Some(tab.database.clone());
                self.conn.selected_collection = None;
                self.current_view = View::Forge;
            }
        }
        self.update_workspace_from_state();
        self.clear_error_status();
        cx.emit(AppEvent::ViewChanged);
        cx.notify();
    }

    pub fn select_next_tab(&mut self, cx: &mut Context<Self>) {
        let preview_count = if self.tabs.preview.is_some() { 1 } else { 0 };
        let tab_count = self.tabs.open.len() + preview_count;
        if tab_count == 0 {
            return;
        }

        let current_index = match self.tabs.active {
            ActiveTab::Index(index) => index.min(self.tabs.open.len().saturating_sub(1)),
            ActiveTab::Preview => self.tabs.open.len(),
            ActiveTab::None => 0,
        };
        let next_index = (current_index + 1) % tab_count;
        if next_index == self.tabs.open.len() {
            self.select_preview_tab(cx);
        } else {
            self.select_tab(next_index, cx);
        }
    }

    pub fn select_prev_tab(&mut self, cx: &mut Context<Self>) {
        let preview_count = if self.tabs.preview.is_some() { 1 } else { 0 };
        let tab_count = self.tabs.open.len() + preview_count;
        if tab_count == 0 {
            return;
        }

        let current_index = match self.tabs.active {
            ActiveTab::Index(index) => index.min(self.tabs.open.len().saturating_sub(1)),
            ActiveTab::Preview => self.tabs.open.len(),
            ActiveTab::None => 0,
        };
        let prev_index = if current_index == 0 { tab_count - 1 } else { current_index - 1 };
        if prev_index == self.tabs.open.len() {
            self.select_preview_tab(cx);
        } else {
            self.select_tab(prev_index, cx);
        }
    }

    pub fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.open.len() {
            return;
        }
        let was_active = matches!(self.tabs.active, ActiveTab::Index(active) if active == index);
        let removed = self.tabs.open.remove(index);
        match &removed {
            TabKey::Collection(key) => {
                self.tabs.dirty.remove(key);
                self.cleanup_session(key);
            }
            TabKey::Database(key) => {
                self.db_sessions.remove(key);
            }
            TabKey::Transfer(key) => {
                self.transfer_tabs.remove(&key.id);
            }
            TabKey::Forge(key) => {
                self.forge_tabs.remove(&key.id);
            }
            TabKey::Settings => {
                // No cleanup needed for settings tab
            }
        }

        if let Some(active) = self.active_index()
            && active > index
        {
            self.set_active_index(active - 1);
        }

        if self.tabs.open.is_empty() && self.tabs.preview.is_none() {
            self.clear_active();
            self.conn.selected_collection = None;
            self.current_view = if self.conn.selected_database.is_some() {
                View::Collections
            } else {
                View::Databases
            };
            self.update_workspace_from_state();
            cx.emit(AppEvent::ViewChanged);
            cx.notify();
            return;
        }

        if was_active {
            if self.tabs.open.is_empty() && self.tabs.preview.is_some() {
                self.set_active_preview();
                let tab = self.tabs.preview.clone().unwrap();
                self.set_selected_connection_internal(tab.connection_id);
                self.conn.selected_database = Some(tab.database.clone());
                self.conn.selected_collection = Some(tab.collection.clone());
                self.current_view = View::Documents;
                self.ensure_session_loaded(tab.clone());
                cx.emit(AppEvent::ViewChanged);
            } else if !self.tabs.open.is_empty() {
                let next_index = index.min(self.tabs.open.len() - 1);
                let tab = self.tabs.open[next_index].clone();
                self.set_active_index(next_index);
                match tab {
                    TabKey::Collection(tab) => {
                        self.set_selected_connection_internal(tab.connection_id);
                        self.conn.selected_database = Some(tab.database.clone());
                        self.conn.selected_collection = Some(tab.collection.clone());
                        self.current_view = View::Documents;
                        self.ensure_session_loaded(tab);
                    }
                    TabKey::Database(tab) => {
                        self.set_selected_connection_internal(tab.connection_id);
                        self.conn.selected_database = Some(tab.database.clone());
                        self.conn.selected_collection = None;
                        self.current_view = View::Database;
                        self.ensure_database_session(tab);
                    }
                    TabKey::Transfer(tab) => {
                        if let Some(conn_id) = tab.connection_id {
                            self.set_selected_connection_internal(conn_id);
                        }
                        if let Some(state) = self.transfer_tabs.get(&tab.id) {
                            if !state.config.source_database.is_empty() {
                                self.conn.selected_database =
                                    Some(state.config.source_database.clone());
                            }
                            if !state.config.source_collection.is_empty() {
                                self.conn.selected_collection =
                                    Some(state.config.source_collection.clone());
                            }
                        }
                        self.current_view = View::Transfer;
                    }
                    TabKey::Forge(tab) => {
                        self.set_selected_connection_internal(tab.connection_id);
                        self.conn.selected_database = Some(tab.database.clone());
                        self.conn.selected_collection = None;
                        self.current_view = View::Forge;
                    }
                    TabKey::Settings => {
                        self.current_view = View::Settings;
                    }
                }
                cx.emit(AppEvent::ViewChanged);
            }
        }

        self.update_workspace_from_state();
        cx.notify();
    }

    pub fn close_preview_tab(&mut self, cx: &mut Context<Self>) {
        if self.tabs.preview.is_none() {
            return;
        }
        let was_active = self.is_preview_active();
        if let Some(tab) = self.tabs.preview.clone() {
            self.tabs.dirty.remove(&tab);
            self.cleanup_session(&tab);
        }
        self.tabs.preview = None;

        if was_active {
            if let Some(index) = self.active_index() {
                if let Some(tab) = self.tabs.open.get(index).cloned() {
                    match tab {
                        TabKey::Collection(tab) => {
                            self.set_selected_connection_internal(tab.connection_id);
                            self.conn.selected_database = Some(tab.database.clone());
                            self.conn.selected_collection = Some(tab.collection.clone());
                            self.current_view = View::Documents;
                            self.ensure_session_loaded(tab);
                        }
                        TabKey::Database(tab) => {
                            self.set_selected_connection_internal(tab.connection_id);
                            self.conn.selected_database = Some(tab.database.clone());
                            self.conn.selected_collection = None;
                            self.current_view = View::Database;
                            self.ensure_database_session(tab);
                        }
                        TabKey::Transfer(tab) => {
                            if let Some(conn_id) = tab.connection_id {
                                self.set_selected_connection_internal(conn_id);
                            }
                            if let Some(state) = self.transfer_tabs.get(&tab.id) {
                                if !state.config.source_database.is_empty() {
                                    self.conn.selected_database =
                                        Some(state.config.source_database.clone());
                                }
                                if !state.config.source_collection.is_empty() {
                                    self.conn.selected_collection =
                                        Some(state.config.source_collection.clone());
                                }
                            }
                            self.current_view = View::Transfer;
                        }
                        TabKey::Forge(tab) => {
                            self.set_selected_connection_internal(tab.connection_id);
                            self.conn.selected_database = Some(tab.database.clone());
                            self.conn.selected_collection = None;
                            self.current_view = View::Forge;
                        }
                        TabKey::Settings => {
                            self.current_view = View::Settings;
                        }
                    }
                    cx.emit(AppEvent::ViewChanged);
                }
            } else if !self.tabs.open.is_empty() {
                let tab = self.tabs.open[0].clone();
                self.set_active_index(0);
                match tab {
                    TabKey::Collection(tab) => {
                        self.set_selected_connection_internal(tab.connection_id);
                        self.conn.selected_database = Some(tab.database.clone());
                        self.conn.selected_collection = Some(tab.collection.clone());
                        self.current_view = View::Documents;
                        self.ensure_session_loaded(tab);
                    }
                    TabKey::Database(tab) => {
                        self.set_selected_connection_internal(tab.connection_id);
                        self.conn.selected_database = Some(tab.database.clone());
                        self.conn.selected_collection = None;
                        self.current_view = View::Database;
                        self.ensure_database_session(tab);
                    }
                    TabKey::Transfer(tab) => {
                        if let Some(conn_id) = tab.connection_id {
                            self.set_selected_connection_internal(conn_id);
                        }
                        if let Some(state) = self.transfer_tabs.get(&tab.id) {
                            if !state.config.source_database.is_empty() {
                                self.conn.selected_database =
                                    Some(state.config.source_database.clone());
                            }
                            if !state.config.source_collection.is_empty() {
                                self.conn.selected_collection =
                                    Some(state.config.source_collection.clone());
                            }
                        }
                        self.current_view = View::Transfer;
                    }
                    TabKey::Forge(tab) => {
                        self.set_selected_connection_internal(tab.connection_id);
                        self.conn.selected_database = Some(tab.database.clone());
                        self.conn.selected_collection = None;
                        self.current_view = View::Forge;
                    }
                    TabKey::Settings => {
                        self.current_view = View::Settings;
                    }
                }
                cx.emit(AppEvent::ViewChanged);
            } else {
                self.clear_active();
                self.conn.selected_collection = None;
                self.current_view = if self.conn.selected_database.is_some() {
                    View::Collections
                } else {
                    View::Databases
                };
                cx.emit(AppEvent::ViewChanged);
            }
        }

        self.update_workspace_from_state();
        cx.notify();
    }

    pub fn close_tabs_for_collection(
        &mut self,
        connection_id: Uuid,
        database: &str,
        collection: &str,
        cx: &mut Context<Self>,
    ) {
        let indices: Vec<usize> = self
            .tabs
            .open
            .iter()
            .enumerate()
            .filter(|(_, tab)| {
                matches!(
                    tab,
                    TabKey::Collection(tab)
                        if tab.connection_id == connection_id
                            && tab.database == database
                            && tab.collection == collection
                )
            })
            .map(|(idx, _)| idx)
            .collect();
        for index in indices.into_iter().rev() {
            self.close_tab(index, cx);
        }

        if let Some(tab) = self.tabs.preview.clone()
            && tab.connection_id == connection_id
            && tab.database == database
            && tab.collection == collection
        {
            self.close_preview_tab(cx);
        }
    }

    pub fn close_tabs_for_database(
        &mut self,
        connection_id: Uuid,
        database: &str,
        cx: &mut Context<Self>,
    ) {
        let indices: Vec<usize> = self
            .tabs
            .open
            .iter()
            .enumerate()
            .filter(|(_, tab)| match tab {
                TabKey::Collection(tab) => {
                    tab.connection_id == connection_id && tab.database == database
                }
                TabKey::Database(tab) => {
                    tab.connection_id == connection_id && tab.database == database
                }
                TabKey::Forge(tab) => {
                    tab.connection_id == connection_id && tab.database == database
                }
                TabKey::Transfer(_) => false,
                TabKey::Settings => false,
            })
            .map(|(idx, _)| idx)
            .collect();
        for index in indices.into_iter().rev() {
            self.close_tab(index, cx);
        }

        if let Some(tab) = self.tabs.preview.clone()
            && tab.connection_id == connection_id
            && tab.database == database
        {
            self.close_preview_tab(cx);
        }
    }

    pub fn rename_collection_keys(
        &mut self,
        connection_id: Uuid,
        database: &str,
        from: &str,
        to: &str,
    ) {
        for tab in &mut self.tabs.open {
            let TabKey::Collection(tab) = tab else {
                continue;
            };
            if tab.connection_id == connection_id
                && tab.database == database
                && tab.collection == from
            {
                tab.collection = to.to_string();
            }
        }

        if let Some(tab) = self.tabs.preview.as_mut()
            && tab.connection_id == connection_id
            && tab.database == database
            && tab.collection == from
        {
            tab.collection = to.to_string();
        }

        if !self.tabs.dirty.is_empty() {
            let mut updated = std::collections::HashSet::with_capacity(self.tabs.dirty.len());
            for key in self.tabs.dirty.iter() {
                let mut next = key.clone();
                if next.connection_id == connection_id
                    && next.database == database
                    && next.collection == from
                {
                    next.collection = to.to_string();
                }
                updated.insert(next);
            }
            self.tabs.dirty = updated;
        }

        self.sessions.rename_collection(connection_id, database, from, to);
        self.update_workspace_from_state();
    }

    pub fn set_collection_dirty(
        &mut self,
        session: SessionKey,
        dirty: bool,
        cx: &mut Context<Self>,
    ) {
        let tab = session;

        if dirty {
            if !self.tabs.dirty.contains(&tab) {
                self.tabs.dirty.insert(tab.clone());
            }

            let mut index =
                self.tabs.open.iter().position(|existing| matches_collection(existing, &tab));
            if index.is_none() {
                self.tabs.open.push(TabKey::Collection(tab.clone()));
                index = Some(self.tabs.open.len() - 1);
            }

            if self.tabs.preview.as_ref() == Some(&tab) {
                self.tabs.preview = None;
            }

            if let Some(index) = index
                && ({
                    match self.tabs.active {
                        ActiveTab::None => true,
                        ActiveTab::Index(active) => active == index,
                        ActiveTab::Preview => {
                            self.conn.selected_database.as_ref() == Some(&tab.database)
                                && self.conn.selected_collection.as_ref() == Some(&tab.collection)
                        }
                    }
                })
            {
                self.set_active_index(index);
            }
        } else {
            self.tabs.dirty.remove(&tab);
        }

        cx.notify();
    }
}

fn matches_collection(tab: &TabKey, key: &SessionKey) -> bool {
    matches!(tab, TabKey::Collection(tab) if tab == key)
}

#[cfg(test)]
mod tests {
    use crate::state::{AppState, SessionKey};

    #[test]
    fn cleanup_session_removes_or_retains() {
        let mut state = AppState::new();
        let key = SessionKey::new(uuid::Uuid::new_v4(), "db", "col");
        state.ensure_session(key.clone());
        state.cleanup_session(&key);
        assert!(state.session(&key).is_none());

        let key = SessionKey::new(uuid::Uuid::new_v4(), "db", "col2");
        state.ensure_session(key.clone());
        state.tabs.preview = Some(key.clone());
        state.cleanup_session(&key);
        assert!(state.session(&key).is_some());
    }
}
