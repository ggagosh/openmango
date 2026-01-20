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

    pub fn clear(&mut self) {
        self.sessions.clear();
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
}

impl AppState {
    /// Build a database key for the current connection + database selection.
    pub fn current_database_key(&self) -> Option<DatabaseKey> {
        let conn = self.conn.active.as_ref()?;
        let db = self.conn.selected_database.as_ref()?;
        Some(DatabaseKey::new(conn.config.id, db))
    }

    pub fn database_session(&self, key: &DatabaseKey) -> Option<&DatabaseSessionState> {
        self.db_sessions.get(key)
    }

    pub fn ensure_database_session(&mut self, key: DatabaseKey) -> &mut DatabaseSessionState {
        self.db_sessions.ensure(key)
    }
}
