//! Session management for per-tab collection state.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use mongodb::bson::{Bson, Document};
use uuid::Uuid;

use super::{
    AppState, CollectionSubview, SessionData, SessionKey, SessionSnapshot, SessionState,
    SessionViewState,
};
use crate::bson::{DocumentKey, PathSegment, path_to_id, set_bson_at_path};

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
        let dirty_selected =
            selected_doc.as_ref().is_some_and(|doc| session.view.dirty.contains(doc));
        Some(SessionSnapshot {
            items: session.data.items.clone(),
            total: session.data.total,
            page: session.data.page,
            per_page: session.data.per_page,
            is_loading: session.data.is_loading,
            selected_doc,
            dirty_selected,
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

    pub fn session_filter(&self, key: &SessionKey) -> Option<Document> {
        self.session_data(key).and_then(|data| data.filter.clone())
    }

    pub fn session_draft(&self, key: &SessionKey, doc_key: &DocumentKey) -> Option<Document> {
        self.session_view(key).and_then(|view| view.drafts.get(doc_key).cloned())
    }

    pub fn session_draft_or_document(
        &self,
        key: &SessionKey,
        doc_key: &DocumentKey,
    ) -> Option<Document> {
        self.session_draft(key, doc_key).or_else(|| self.document_for_key(key, doc_key))
    }

    /// Get a mutable reference to a session.
    pub fn session_mut(&mut self, key: &SessionKey) -> Option<&mut SessionState> {
        self.sessions.get_mut(key)
    }

    pub fn document_index(&self, session_key: &SessionKey, doc_key: &DocumentKey) -> Option<usize> {
        let session = self.session(session_key)?;
        session.data.index_by_key.get(doc_key).copied()
    }

    pub fn document_for_key(
        &self,
        session_key: &SessionKey,
        doc_key: &DocumentKey,
    ) -> Option<Document> {
        let session = self.session(session_key)?;
        let index = session.data.index_by_key.get(doc_key)?;
        session.data.items.get(*index).map(|item| item.doc.clone())
    }

    /// Ensure a session exists and return a mutable reference to it.
    pub fn ensure_session(&mut self, key: SessionKey) -> &mut SessionState {
        self.sessions.ensure(key)
    }

    pub fn set_selected_node(
        &mut self,
        session_key: &SessionKey,
        doc_key: DocumentKey,
        node_id: String,
    ) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.selected_doc = Some(doc_key);
            session.view.selected_node_id = Some(node_id);
        }
    }

    pub fn toggle_expanded_node(&mut self, session_key: &SessionKey, node_id: &str) {
        if let Some(session) = self.session_mut(session_key) {
            if session.view.expanded_nodes.contains(node_id) {
                session.view.expanded_nodes.remove(node_id);
            } else {
                session.view.expanded_nodes.insert(node_id.to_string());
            }
        }
    }

    pub fn expand_path(
        &mut self,
        session_key: &SessionKey,
        doc_key: &DocumentKey,
        path: &[PathSegment],
    ) {
        if let Some(session) = self.session_mut(session_key) {
            for depth in 0..=path.len() {
                session.view.expanded_nodes.insert(path_to_id(doc_key, &path[..depth]));
            }
        }
    }

    pub fn set_draft(&mut self, session_key: &SessionKey, doc_key: DocumentKey, doc: Document) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.drafts.insert(doc_key.clone(), doc);
            session.view.dirty.insert(doc_key);
        }
    }

    pub fn clear_draft(&mut self, session_key: &SessionKey, doc_key: &DocumentKey) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.drafts.remove(doc_key);
            session.view.dirty.remove(doc_key);
        }
    }

    pub fn clear_all_drafts(&mut self, session_key: &SessionKey) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.drafts.clear();
            session.view.dirty.clear();
        }
    }

    pub fn update_draft_value(
        &mut self,
        session_key: &SessionKey,
        doc_key: &DocumentKey,
        original: &Document,
        path: &[PathSegment],
        new_value: Bson,
    ) -> bool {
        let Some(session) = self.session_mut(session_key) else {
            return false;
        };

        let draft = session.view.drafts.entry(doc_key.clone()).or_insert_with(|| original.clone());

        if set_bson_at_path(draft, path, new_value) {
            if draft == original {
                session.view.drafts.remove(doc_key);
                session.view.dirty.remove(doc_key);
            } else {
                session.view.dirty.insert(doc_key.clone());
            }
            return true;
        }
        false
    }

    pub fn prev_page(&mut self, session_key: &SessionKey) -> bool {
        if let Some(session) = self.session_mut(session_key)
            && session.data.page > 0
        {
            session.data.page -= 1;
            return true;
        }
        false
    }

    pub fn next_page(&mut self, session_key: &SessionKey, total_pages: u64) -> bool {
        if let Some(session) = self.session_mut(session_key)
            && session.data.page + 1 < total_pages
        {
            session.data.page += 1;
            return true;
        }
        false
    }

    pub fn set_filter(&mut self, session_key: &SessionKey, raw: String, filter: Option<Document>) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.filter_raw = raw;
            session.data.filter = filter;
            session.data.page = 0;
        }
        self.update_workspace_from_state();
    }

    pub fn clear_filter(&mut self, session_key: &SessionKey) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.filter_raw.clear();
            session.data.filter = None;
            session.data.page = 0;
        }
        self.update_workspace_from_state();
    }

    pub fn set_sort_projection(
        &mut self,
        session_key: &SessionKey,
        sort_raw: String,
        sort: Option<Document>,
        projection_raw: String,
        projection: Option<Document>,
    ) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.sort_raw = sort_raw;
            session.data.sort = sort;
            session.data.projection_raw = projection_raw;
            session.data.projection = projection;
            session.data.page = 0;
        }
        self.update_workspace_from_state();
    }

    pub fn set_collection_subview(
        &mut self,
        session_key: &SessionKey,
        subview: CollectionSubview,
    ) -> bool {
        let mut should_load = false;
        let mut changed = false;

        if let Some(session) = self.session_mut(session_key) {
            if session.view.subview == subview {
                return false;
            }
            session.view.subview = subview;
            session.view.stats_open = matches!(subview, CollectionSubview::Stats);
            should_load = subview == CollectionSubview::Stats
                && !session.data.stats_loading
                && (session.data.stats.is_none() || session.data.stats_error.is_some());
            changed = true;
        }

        if changed {
            self.update_workspace_from_state();
        }

        should_load
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{Bson, doc};

    use crate::bson::{DocumentKey, PathSegment};
    use crate::state::{AppState, SessionKey};

    #[test]
    fn update_draft_value_tracks_dirty_and_clears() {
        let mut state = AppState::new();
        let session_key = SessionKey::new(uuid::Uuid::new_v4(), "db", "col");
        state.ensure_session(session_key.clone());

        let original = doc! { "_id": "doc1", "name": "alpha" };
        let doc_key = DocumentKey::from_document(&original, 0);
        let path = vec![PathSegment::Key("name".to_string())];

        let updated = state.update_draft_value(
            &session_key,
            &doc_key,
            &original,
            &path,
            Bson::String("beta".to_string()),
        );
        assert!(updated);
        let session = state.session(&session_key).unwrap();
        assert!(session.view.dirty.contains(&doc_key));

        let cleared = state.update_draft_value(
            &session_key,
            &doc_key,
            &original,
            &path,
            Bson::String("alpha".to_string()),
        );
        assert!(cleared);
        let session = state.session(&session_key).unwrap();
        assert!(!session.view.dirty.contains(&doc_key));
    }

    #[test]
    fn paging_helpers_enforce_bounds() {
        let mut state = AppState::new();
        let session_key = SessionKey::new(uuid::Uuid::new_v4(), "db", "col");
        state.ensure_session(session_key.clone());

        assert!(!state.prev_page(&session_key));
        assert!(state.next_page(&session_key, 2));
        assert!(state.prev_page(&session_key));
    }
}
