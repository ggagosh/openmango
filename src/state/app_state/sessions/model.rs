//! Session management for per-tab collection state.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use mongodb::bson::Document;
use uuid::Uuid;

use crate::bson::DocumentKey;
use crate::state::AppState;
use crate::state::app_state::types::{
    CollectionSubview, DocumentViewMode, SessionData, SessionKey, SessionSnapshot, SessionState,
    SessionViewState,
};

#[derive(Default)]
pub struct SessionStore {
    sessions: HashMap<SessionKey, SessionState>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &SessionKey) -> Option<&SessionState> {
        self.sessions.get(key)
    }

    pub fn get_mut(&mut self, key: &SessionKey) -> Option<&mut SessionState> {
        self.sessions.get_mut(key)
    }

    pub fn ensure(&mut self, key: SessionKey) -> &mut SessionState {
        match self.sessions.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(SessionState::default()),
        }
    }

    pub fn remove(&mut self, key: &SessionKey) -> Option<SessionState> {
        self.sessions.remove(key)
    }

    pub fn remove_connection(&mut self, connection_id: Uuid) {
        self.sessions.retain(|key, _| key.connection_id != connection_id);
    }

    pub fn rename_collection(&mut self, connection_id: Uuid, database: &str, from: &str, to: &str) {
        let keys: Vec<SessionKey> = self
            .sessions
            .keys()
            .filter(|key| {
                key.connection_id == connection_id
                    && key.database == database
                    && key.collection == from
            })
            .cloned()
            .collect();

        for key in keys {
            if let Some(state) = self.sessions.remove(&key) {
                let mut new_key = key.clone();
                new_key.collection = to.to_string();
                self.sessions.insert(new_key, state);
            }
        }
    }
}

impl AppState {
    /// Build a session key for the current connection + collection selection.
    pub fn current_session_key(&self) -> Option<SessionKey> {
        let conn_id = self.conn.selected_connection?;
        if !self.conn.active.contains_key(&conn_id) {
            return None;
        }
        let db = self.conn.selected_database.as_ref()?;
        let col = self.conn.selected_collection.as_ref()?;
        Some(SessionKey::new(conn_id, db, col))
    }

    /// Build a runtime AI session key from current selection.
    /// Database/collection can be empty to support metadata-only AI mode.
    pub fn current_ai_session_key(&self) -> Option<SessionKey> {
        let conn_id = self.conn.selected_connection?;
        if !self.conn.active.contains_key(&conn_id) {
            return None;
        }
        let db = self.conn.selected_database.clone().unwrap_or_default();
        let col = self.conn.selected_collection.clone().unwrap_or_default();
        Some(SessionKey::new(conn_id, db, col))
    }

    /// Get an immutable reference to a session.
    pub fn session(&self, key: &SessionKey) -> Option<&SessionState> {
        self.sessions.get(key)
    }

    pub fn session_view(&self, key: &SessionKey) -> Option<&SessionViewState> {
        self.session(key).map(|session| &session.view)
    }

    pub fn session_data(&self, key: &SessionKey) -> Option<&SessionData> {
        self.session(key).map(|session| &session.data)
    }

    pub fn session_snapshot(&self, key: &SessionKey) -> Option<SessionSnapshot> {
        let session = self.session(key)?;
        let selected_doc = session.view.selected_doc.clone();
        let selected_docs = session.view.selected_docs.clone();
        let selected_count = selected_docs.len();
        let any_selected_dirty = selected_docs.iter().any(|k| session.view.dirty.contains(k));
        Some(SessionSnapshot {
            items: session.data.items.clone(),
            total: session.data.total,
            page: session.data.page,
            per_page: session.data.per_page,
            is_loading: session.data.is_loading,
            selected_doc,
            selected_docs,
            selected_count,
            any_selected_dirty,
            filter_raw: session.data.filter_raw.clone(),
            sort_raw: session.data.sort_raw.clone(),
            projection_raw: session.data.projection_raw.clone(),
            query_options_open: session.view.query_options_open,
            filter_builder_open: session.view.filter_builder_open,
            subview: session.view.subview,
            stats: session.data.stats.clone(),
            stats_loading: session.data.stats_loading,
            stats_error: session.data.stats_error.clone(),
            indexes: session.data.indexes.clone(),
            indexes_loading: session.data.indexes_loading,
            indexes_error: session.data.indexes_error.clone(),
            aggregation: session.data.aggregation.clone(),
            explain: session.data.explain.clone(),
            ai_chat: session.data.ai_chat.clone(),
            schema: session.data.schema.clone(),
            schema_loading: session.data.schema_loading,
            schema_error: session.data.schema_error.clone(),
            schema_selected_field: session.view.schema_selected_field.clone(),
            schema_expanded_fields: session.view.schema_expanded_fields.clone(),
            schema_filter: session.view.schema_filter.clone(),
        })
    }

    pub fn session_selected_doc(&self, key: &SessionKey) -> Option<DocumentKey> {
        self.session_view(key).and_then(|view| view.selected_doc.clone())
    }

    pub fn session_selected_node_id(&self, key: &SessionKey) -> Option<String> {
        self.session_view(key).and_then(|view| view.selected_node_id.clone())
    }

    pub fn session_subview(&self, key: &SessionKey) -> Option<CollectionSubview> {
        self.session_view(key).map(|view| view.subview)
    }

    pub fn session_view_mode(&self, key: &SessionKey) -> DocumentViewMode {
        self.session_view(key).map(|view| view.view_mode).unwrap_or_default()
    }

    pub fn set_view_mode(&mut self, key: &SessionKey, mode: DocumentViewMode) {
        if let Some(session) = self.session_mut(key) {
            session.view.view_mode = mode;
        }
    }

    pub fn table_column_widths(&self, key: &SessionKey) -> HashMap<String, f32> {
        self.session_view(key).map(|v| v.table_column_widths.clone()).unwrap_or_default()
    }

    pub fn set_table_column_widths(&mut self, key: &SessionKey, widths: HashMap<String, f32>) {
        if let Some(session) = self.session_mut(key) {
            session.view.table_column_widths = widths;
        }
    }

    pub fn table_column_order(&self, key: &SessionKey) -> Vec<String> {
        self.session_view(key).map(|v| v.table_column_order.clone()).unwrap_or_default()
    }

    pub fn set_table_column_order(&mut self, key: &SessionKey, order: Vec<String>) {
        if let Some(session) = self.session_mut(key) {
            session.view.table_column_order = order;
        }
    }

    pub fn table_pinned_columns(&self, key: &SessionKey) -> HashSet<String> {
        self.session_view(key).map(|v| v.table_pinned_columns.clone()).unwrap_or_default()
    }

    pub fn set_table_pinned_columns(&mut self, key: &SessionKey, pinned: HashSet<String>) {
        if let Some(session) = self.session_mut(key) {
            session.view.table_pinned_columns = pinned;
        }
    }

    pub fn toggle_table_pinned_column(&mut self, key: &SessionKey, column: String) -> bool {
        if let Some(session) = self.session_mut(key) {
            let is_pinned = if session.view.table_pinned_columns.contains(&column) {
                session.view.table_pinned_columns.remove(&column);
                false
            } else {
                session.view.table_pinned_columns.insert(column);
                true
            };
            return is_pinned;
        }
        false
    }

    pub fn table_hidden_columns(&self, key: &SessionKey) -> HashSet<String> {
        self.session_view(key).map(|v| v.table_hidden_columns.clone()).unwrap_or_default()
    }

    pub fn set_table_hidden_columns(&mut self, key: &SessionKey, hidden: HashSet<String>) {
        if let Some(session) = self.session_mut(key) {
            session.view.table_hidden_columns = hidden;
        }
    }

    pub fn toggle_table_hidden_column(&mut self, key: &SessionKey, column: String) {
        if let Some(session) = self.session_mut(key) {
            if session.view.table_hidden_columns.contains(&column) {
                session.view.table_hidden_columns.remove(&column);
            } else {
                session.view.table_hidden_columns.insert(column);
            }
        }
    }

    // ── Aggregation table column state ───────────────────────────────

    pub fn set_agg_table_column_widths(&mut self, key: &SessionKey, widths: HashMap<String, f32>) {
        if let Some(session) = self.session_mut(key) {
            session.view.agg_table_column_widths = widths;
        }
    }

    pub fn set_agg_table_column_order(&mut self, key: &SessionKey, order: Vec<String>) {
        if let Some(session) = self.session_mut(key) {
            session.view.agg_table_column_order = order;
        }
    }

    pub fn toggle_agg_table_pinned_column(&mut self, key: &SessionKey, column: String) -> bool {
        if let Some(session) = self.session_mut(key) {
            let is_pinned = if session.view.agg_table_pinned_columns.contains(&column) {
                session.view.agg_table_pinned_columns.remove(&column);
                false
            } else {
                session.view.agg_table_pinned_columns.insert(column);
                true
            };
            return is_pinned;
        }
        false
    }

    pub fn toggle_agg_table_hidden_column(&mut self, key: &SessionKey, column: String) {
        if let Some(session) = self.session_mut(key) {
            if session.view.agg_table_hidden_columns.contains(&column) {
                session.view.agg_table_hidden_columns.remove(&column);
            } else {
                session.view.agg_table_hidden_columns.insert(column);
            }
        }
    }

    pub fn set_aggregation_view_mode(
        &mut self,
        key: &SessionKey,
        mode: super::super::types::DocumentViewMode,
    ) {
        if let Some(session) = self.session_mut(key) {
            session.data.aggregation.results_view_mode = mode;
        }
    }

    pub fn aggregation_view_mode(&self, key: &SessionKey) -> super::super::types::DocumentViewMode {
        self.session(key).map(|s| s.data.aggregation.results_view_mode).unwrap_or_default()
    }

    pub fn session_mut(&mut self, key: &SessionKey) -> Option<&mut SessionState> {
        self.sessions.get_mut(key)
    }

    pub fn ensure_session(&mut self, key: SessionKey) -> &mut SessionState {
        self.sessions.ensure(key)
    }
}
