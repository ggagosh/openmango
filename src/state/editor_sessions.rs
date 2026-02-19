//! Shared JSON editor sessions for detached windows.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui::AnyWindowHandle;
use mongodb::bson::{Bson, Document};
use uuid::Uuid;

use crate::bson::DocumentKey;
use crate::state::app_state::SessionKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditorSessionId(Uuid);

impl EditorSessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EditorSessionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum EditorSessionTarget {
    Document { doc_key: DocumentKey, original_id: Box<Bson>, baseline_document: Box<Document> },
    Insert,
}

#[derive(Debug, Clone)]
pub struct EditorSession {
    pub id: EditorSessionId,
    pub session_key: SessionKey,
    pub target: EditorSessionTarget,
    pub content: String,
}

impl EditorSession {
    pub fn title(&self) -> String {
        match &self.target {
            EditorSessionTarget::Document { doc_key, .. } => {
                format!(
                    "Edit {} ({})",
                    self.session_key.collection,
                    short_document_key_label(doc_key)
                )
            }
            EditorSessionTarget::Insert => format!("Insert {}", self.session_key.collection),
        }
    }
}

#[derive(Clone, Default)]
pub struct EditorSessionStore {
    inner: Arc<Mutex<EditorSessionStoreInner>>,
}

#[derive(Default)]
struct EditorSessionStoreInner {
    sessions: HashMap<EditorSessionId, EditorSession>,
    keys: HashMap<EditorSessionKey, EditorSessionId>,
    key_by_session: HashMap<EditorSessionId, EditorSessionKey>,
    windows: HashMap<EditorSessionId, AnyWindowHandle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum EditorSessionKey {
    Document { session_key: SessionKey, doc_key: DocumentKey },
    Insert { session_key: SessionKey },
}

impl EditorSessionStore {
    pub fn create_document_session(
        &self,
        session_key: SessionKey,
        doc_key: DocumentKey,
        original_id: Bson,
        baseline_document: Document,
        content: String,
    ) -> EditorSessionId {
        let id = EditorSessionId::new();
        let key = EditorSessionKey::Document {
            session_key: session_key.clone(),
            doc_key: doc_key.clone(),
        };
        let session = EditorSession {
            id,
            session_key,
            target: EditorSessionTarget::Document {
                doc_key,
                original_id: Box::new(original_id),
                baseline_document: Box::new(baseline_document),
            },
            content,
        };
        self.with_inner_mut(|inner| {
            inner.sessions.insert(id, session);
            inner.keys.insert(key.clone(), id);
            inner.key_by_session.insert(id, key);
        });
        id
    }

    pub fn create_insert_session(
        &self,
        session_key: SessionKey,
        content: String,
    ) -> EditorSessionId {
        let id = EditorSessionId::new();
        let key = EditorSessionKey::Insert { session_key: session_key.clone() };
        let session =
            EditorSession { id, session_key, target: EditorSessionTarget::Insert, content };
        self.with_inner_mut(|inner| {
            inner.sessions.insert(id, session);
            inner.keys.insert(key.clone(), id);
            inner.key_by_session.insert(id, key);
        });
        id
    }

    pub fn snapshot(&self, id: EditorSessionId) -> Option<EditorSession> {
        self.with_inner(|inner| inner.sessions.get(&id).cloned()).flatten()
    }

    pub fn update_content(&self, id: EditorSessionId, content: String) -> bool {
        self.with_inner_mut(|inner| {
            let Some(session) = inner.sessions.get_mut(&id) else {
                return false;
            };
            session.content = content;
            true
        })
        .unwrap_or(false)
    }

    pub fn refresh_document_baseline(
        &self,
        id: EditorSessionId,
        baseline_document: Document,
    ) -> bool {
        self.with_inner_mut(|inner| {
            let Some(session) = inner.sessions.get_mut(&id) else {
                return false;
            };
            let EditorSessionTarget::Document { baseline_document: current, .. } =
                &mut session.target
            else {
                return false;
            };
            **current = baseline_document;
            true
        })
        .unwrap_or(false)
    }

    pub fn close(&self, id: EditorSessionId) -> bool {
        self.with_inner_mut(|inner| {
            let removed = inner.sessions.remove(&id).is_some();
            if let Some(key) = inner.key_by_session.remove(&id) {
                inner.keys.remove(&key);
            }
            inner.windows.remove(&id);
            removed
        })
        .unwrap_or(false)
    }

    pub fn find_document_session(
        &self,
        session_key: &SessionKey,
        doc_key: &DocumentKey,
    ) -> Option<EditorSessionId> {
        let key = EditorSessionKey::Document {
            session_key: session_key.clone(),
            doc_key: doc_key.clone(),
        };
        self.with_inner(|inner| inner.keys.get(&key).copied()).flatten()
    }

    pub fn find_insert_session(&self, session_key: &SessionKey) -> Option<EditorSessionId> {
        let key = EditorSessionKey::Insert { session_key: session_key.clone() };
        self.with_inner(|inner| inner.keys.get(&key).copied()).flatten()
    }

    pub fn find_any_document_session(&self) -> Option<EditorSessionId> {
        self.with_inner(|inner| {
            inner.sessions.iter().find_map(|(id, session)| {
                if matches!(session.target, EditorSessionTarget::Document { .. }) {
                    Some(*id)
                } else {
                    None
                }
            })
        })
        .flatten()
    }

    pub fn find_any_insert_session(&self) -> Option<EditorSessionId> {
        self.with_inner(|inner| {
            inner.sessions.iter().find_map(|(id, session)| {
                if matches!(session.target, EditorSessionTarget::Insert) { Some(*id) } else { None }
            })
        })
        .flatten()
    }

    pub fn register_window(&self, id: EditorSessionId, window: AnyWindowHandle) -> bool {
        self.with_inner_mut(|inner| {
            if !inner.sessions.contains_key(&id) {
                return false;
            }
            inner.windows.insert(id, window);
            true
        })
        .unwrap_or(false)
    }

    pub fn window_handle(&self, id: EditorSessionId) -> Option<AnyWindowHandle> {
        self.with_inner(|inner| inner.windows.get(&id).copied()).flatten()
    }

    fn with_inner<T>(&self, f: impl FnOnce(&EditorSessionStoreInner) -> T) -> Option<T> {
        let inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Some(f(&inner))
    }

    fn with_inner_mut<T>(&self, f: impl FnOnce(&mut EditorSessionStoreInner) -> T) -> Option<T> {
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Some(f(&mut inner))
    }
}

fn short_document_key_label(doc_key: &DocumentKey) -> String {
    let raw = doc_key.as_str();

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
        if let Some(oid) = value.get("$oid").and_then(|v| v.as_str()) {
            let short = oid.chars().take(8).collect::<String>();
            return format!("{short}...");
        };
        if let Some(string_value) = value.as_str() {
            return truncate_label(string_value, 16);
        }
        if value.is_number() || value.is_boolean() || value.is_null() {
            return value.to_string();
        }
    }

    truncate_label(raw, 16)
}

fn truncate_label(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        format!("{}...", value.chars().take(max_chars).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{Bson, doc, oid::ObjectId};

    use super::*;

    #[test]
    fn update_content_updates_existing_session() {
        let store = EditorSessionStore::default();
        let session_id = store
            .create_insert_session(SessionKey::new(Uuid::new_v4(), "db", "col"), "{}".to_string());

        let updated = store.update_content(session_id, "{\"name\":\"updated\"}".to_string());
        assert!(updated);
        let snapshot = store.snapshot(session_id).expect("session should exist");
        assert_eq!(snapshot.content, "{\"name\":\"updated\"}");
    }

    #[test]
    fn refresh_baseline_only_updates_document_sessions() {
        let store = EditorSessionStore::default();
        let session_id = store.create_document_session(
            SessionKey::new(Uuid::new_v4(), "db", "col"),
            DocumentKey::from_id(&Bson::ObjectId(ObjectId::new())),
            Bson::Int32(1),
            doc! { "_id": 1, "name": "before" },
            "{ \"_id\": 1, \"name\": \"before\" }".to_string(),
        );

        let refreshed =
            store.refresh_document_baseline(session_id, doc! { "_id": 1, "name": "after" });
        assert!(refreshed);

        let snapshot = store.snapshot(session_id).expect("session should exist");
        let EditorSessionTarget::Document { baseline_document, .. } = snapshot.target else {
            panic!("expected document target");
        };
        assert_eq!(baseline_document.get_str("name").ok(), Some("after"));
    }

    #[test]
    fn find_any_session_helpers_locate_existing_sessions() {
        let store = EditorSessionStore::default();
        let document_id = store.create_document_session(
            SessionKey::new(Uuid::new_v4(), "db", "col"),
            DocumentKey::from_id(&Bson::Int32(1)),
            Bson::Int32(1),
            doc! { "_id": 1 },
            "{ \"_id\": 1 }".to_string(),
        );
        let insert_id = store.create_insert_session(
            SessionKey::new(Uuid::new_v4(), "db2", "col2"),
            "{}".to_string(),
        );

        assert_eq!(store.find_any_document_session(), Some(document_id));
        assert_eq!(store.find_any_insert_session(), Some(insert_id));
    }
}
