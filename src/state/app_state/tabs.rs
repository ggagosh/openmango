//! Tab management for AppState.

use gpui::Context;
use uuid::Uuid;

use crate::state::events::AppEvent;
use crate::state::StatusLevel;

use super::AppState;
use super::types::{ActiveTab, DatabaseKey, SessionKey, TabKey, View};

#[derive(Debug, Clone, Copy)]
pub(super) enum TabOpenMode {
    Preview,
    Permanent,
}

impl AppState {
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
        if matches!(self.status_message.as_ref().map(|m| &m.level), Some(StatusLevel::Error)) {
            self.status_message = None;
        }
    }

    pub(super) fn cleanup_session(&mut self, key: &SessionKey) {
        let still_referenced = self
            .tabs
            .open
            .iter()
            .any(|tab| matches_collection(tab, key))
            || self.tabs.preview.as_ref() == Some(key);
        if !still_referenced {
            self.sessions.remove(key);
        }
    }

    pub(super) fn ensure_session_loaded(&mut self, key: SessionKey) {
        let session = self.ensure_session(key);
        if session.data.items.is_empty() {
            session.data.is_loading = true;
        }
    }

    pub(super) fn open_collection_with_mode(
        &mut self,
        database: String,
        collection: String,
        mode: TabOpenMode,
        cx: &mut Context<Self>,
    ) {
        let Some(active) = &self.conn.active else {
            return;
        };
        let new_tab = SessionKey::new(active.config.id, database.clone(), collection.clone());
        let existing_index = self
            .tabs
            .open
            .iter()
            .position(|tab| matches_collection(tab, &new_tab));
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

    pub(super) fn open_database_tab(&mut self, database: String, cx: &mut Context<Self>) {
        let Some(active) = &self.conn.active else {
            return;
        };
        let key = DatabaseKey::new(active.config.id, database.clone());
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

    pub(super) fn close_database_tabs(&mut self, cx: &mut Context<Self>) {
        let indices: Vec<usize> = self
            .tabs
            .open
            .iter()
            .enumerate()
            .filter(|(_, tab)| matches!(tab, TabKey::Database(_)))
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
                self.conn.selected_database = Some(tab.database.clone());
                self.conn.selected_collection = Some(tab.collection.clone());
                self.current_view = View::Documents;
                self.ensure_session_loaded(tab);
            }
            TabKey::Database(tab) => {
                self.conn.selected_database = Some(tab.database.clone());
                self.conn.selected_collection = None;
                self.current_view = View::Database;
                self.ensure_database_session(tab);
            }
        }
        self.update_workspace_from_state();
        self.clear_error_status();
        cx.emit(AppEvent::ViewChanged);
        cx.notify();
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
                        self.conn.selected_database = Some(tab.database.clone());
                        self.conn.selected_collection = Some(tab.collection.clone());
                        self.current_view = View::Documents;
                        self.ensure_session_loaded(tab);
                    }
                    TabKey::Database(tab) => {
                        self.conn.selected_database = Some(tab.database.clone());
                        self.conn.selected_collection = None;
                        self.current_view = View::Database;
                        self.ensure_database_session(tab);
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
                            self.conn.selected_database = Some(tab.database.clone());
                            self.conn.selected_collection = Some(tab.collection.clone());
                            self.current_view = View::Documents;
                            self.ensure_session_loaded(tab);
                        }
                        TabKey::Database(tab) => {
                            self.conn.selected_database = Some(tab.database.clone());
                            self.conn.selected_collection = None;
                            self.current_view = View::Database;
                            self.ensure_database_session(tab);
                        }
                    }
                    cx.emit(AppEvent::ViewChanged);
                }
            } else if !self.tabs.open.is_empty() {
                let tab = self.tabs.open[0].clone();
                self.set_active_index(0);
                match tab {
                    TabKey::Collection(tab) => {
                        self.conn.selected_database = Some(tab.database.clone());
                        self.conn.selected_collection = Some(tab.collection.clone());
                        self.current_view = View::Documents;
                        self.ensure_session_loaded(tab);
                    }
                    TabKey::Database(tab) => {
                        self.conn.selected_database = Some(tab.database.clone());
                        self.conn.selected_collection = None;
                        self.current_view = View::Database;
                        self.ensure_database_session(tab);
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
                matches!(tab, TabKey::Collection(tab) if tab.database == database && tab.collection == collection)
            })
            .map(|(idx, _)| idx)
            .collect();
        for index in indices.into_iter().rev() {
            self.close_tab(index, cx);
        }

        if let Some(tab) = self.tabs.preview.clone()
            && tab.database == database
            && tab.collection == collection
        {
            self.close_preview_tab(cx);
        }
    }

    pub fn close_tabs_for_database(&mut self, database: &str, cx: &mut Context<Self>) {
        let indices: Vec<usize> = self
            .tabs
            .open
            .iter()
            .enumerate()
            .filter(|(_, tab)| match tab {
                TabKey::Collection(tab) => tab.database == database,
                TabKey::Database(tab) => tab.database == database,
            })
            .map(|(idx, _)| idx)
            .collect();
        for index in indices.into_iter().rev() {
            self.close_tab(index, cx);
        }

        if let Some(tab) = self.tabs.preview.clone()
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
            if tab.connection_id == connection_id && tab.database == database && tab.collection == from
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
