//! Session management for per-tab collection state.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use mongodb::bson::Document;
use uuid::Uuid;

use crate::bson::DocumentKey;
use crate::state::AppState;
use crate::state::app_state::types::{
    CollectionSubview, SessionData, SessionKey, SessionSnapshot, SessionState, SessionViewState,
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
            subview: session.view.subview,
            stats: session.data.stats.clone(),
            stats_loading: session.data.stats_loading,
            stats_error: session.data.stats_error.clone(),
            indexes: session.data.indexes.clone(),
            indexes_loading: session.data.indexes_loading,
            indexes_error: session.data.indexes_error.clone(),
            aggregation: session.data.aggregation.clone(),
            explain: session.data.explain.clone(),
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

    /// Get a mutable reference to a session.
    pub fn session_mut(&mut self, key: &SessionKey) -> Option<&mut SessionState> {
        self.sessions.get_mut(key)
    }

    /// Ensure a session exists and return a mutable reference to it.
    pub fn ensure_session(&mut self, key: SessionKey) -> &mut SessionState {
        self.sessions.ensure(key)
    }
}
