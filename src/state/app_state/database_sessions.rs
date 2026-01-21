//! Database session management for per-tab database state.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use super::{AppState, DatabaseKey, DatabaseSessionState};
use uuid::Uuid;

#[derive(Default)]
pub struct DatabaseSessionStore {
    sessions: HashMap<DatabaseKey, DatabaseSessionState>,
}

impl DatabaseSessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &DatabaseKey) -> Option<&DatabaseSessionState> {
        self.sessions.get(key)
    }

    pub fn ensure(&mut self, key: DatabaseKey) -> &mut DatabaseSessionState {
        match self.sessions.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(DatabaseSessionState::default()),
        }
    }

    pub fn remove(&mut self, key: &DatabaseKey) -> Option<DatabaseSessionState> {
        self.sessions.remove(key)
    }

    pub fn remove_connection(&mut self, connection_id: Uuid) {
        self.sessions.retain(|key, _| key.connection_id != connection_id);
    }
}

impl AppState {
    /// Build a database key for the current connection + database selection.
    pub fn current_database_key(&self) -> Option<DatabaseKey> {
        let conn_id = self.conn.selected_connection?;
        if !self.conn.active.contains_key(&conn_id) {
            return None;
        }
        let db = self.conn.selected_database.as_ref()?;
        Some(DatabaseKey::new(conn_id, db))
    }

    pub fn database_session(&self, key: &DatabaseKey) -> Option<&DatabaseSessionState> {
        self.db_sessions.get(key)
    }

    pub fn ensure_database_session(&mut self, key: DatabaseKey) -> &mut DatabaseSessionState {
        self.db_sessions.ensure(key)
    }
}
