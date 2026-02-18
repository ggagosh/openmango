//! Document operations for sessions (drafts, node selection, expansion).

use mongodb::bson::{Bson, Document};

use std::collections::HashSet;

use crate::bson::{DocumentKey, PathSegment, path_to_id, set_bson_at_path};
use crate::state::AppState;
use crate::state::app_state::types::SessionKey;

impl AppState {
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

    pub fn clear_expanded_nodes(&mut self, session_key: &SessionKey) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.expanded_nodes.clear();
        }
    }

    pub fn set_expanded_nodes(&mut self, session_key: &SessionKey, nodes: HashSet<String>) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.expanded_nodes = nodes;
        }
    }

    /// Toggle a document in the multi-selection set.
    pub fn toggle_doc_selection(&mut self, session_key: &SessionKey, doc_key: &DocumentKey) {
        if let Some(session) = self.session_mut(session_key) {
            if session.view.selected_docs.contains(doc_key) {
                session.view.selected_docs.remove(doc_key);
            } else {
                session.view.selected_docs.insert(doc_key.clone());
            }
        }
    }

    /// Clear multi-selection, insert a single doc, and set it as the primary selection.
    pub fn select_single_doc(
        &mut self,
        session_key: &SessionKey,
        doc_key: DocumentKey,
        node_id: String,
    ) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.selected_docs.clear();
            session.view.selected_docs.insert(doc_key.clone());
            session.view.selected_doc = Some(doc_key);
            session.view.selected_node_id = Some(node_id);
        }
    }

    /// Select all documents currently loaded in the session.
    pub fn select_all_docs(&mut self, session_key: &SessionKey) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.selected_docs =
                session.data.items.iter().map(|item| item.key.clone()).collect();
        }
    }

    /// Replace multi-selection with a range of doc keys and set the primary selection.
    pub fn select_doc_range(
        &mut self,
        session_key: &SessionKey,
        doc_keys: HashSet<DocumentKey>,
        primary_doc_key: DocumentKey,
        primary_node_id: String,
    ) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.selected_docs = doc_keys;
            session.view.selected_doc = Some(primary_doc_key);
            session.view.selected_node_id = Some(primary_node_id);
        }
    }

    /// Clear the multi-selection set.
    pub fn clear_doc_selection(&mut self, session_key: &SessionKey) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.selected_docs.clear();
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
}
