use gpui::{AppContext as _, Context};
use mongodb::bson::{Bson, Document};
use uuid::Uuid;

use crate::bson::{DocumentKey, parse_bson_from_relaxed_json};
use crate::state::{EditorSession, EditorSessionId, EditorSessionTarget, SessionKey, TabKey};

use super::AppState;

#[derive(Clone)]
pub enum UnsavedScope {
    Tab(TabKey),
    Preview(SessionKey),
    Connection(Uuid),
    Editor(EditorSessionId),
    Workspace,
    App,
}

#[derive(Clone)]
pub enum UnsavedChange {
    InlineDocument {
        session_key: SessionKey,
        doc_key: DocumentKey,
        original_id: Option<Bson>,
        baseline_document: Option<Document>,
        document: Document,
    },
    InvalidInlineEdit {
        session_key: SessionKey,
    },
    DetachedEditor(EditorSession),
}

#[derive(Clone, Default)]
pub struct UnsavedInventory {
    pub changes: Vec<UnsavedChange>,
}

impl UnsavedInventory {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }
}

impl AppState {
    pub fn set_invalid_inline_edit(&mut self, session_key: SessionKey, invalid: bool) {
        if invalid {
            self.invalid_inline_edits.insert(session_key);
        } else {
            self.invalid_inline_edits.remove(&session_key);
        }
    }

    pub fn unsaved_guard_is_active(&self) -> bool {
        self.unsaved_guard_active
    }

    pub fn begin_unsaved_guard(&mut self) -> bool {
        if self.unsaved_guard_active {
            return false;
        }
        self.unsaved_guard_active = true;
        true
    }

    pub fn end_unsaved_guard(&mut self) {
        self.unsaved_guard_active = false;
    }

    pub fn unsaved_inventory(&self, scope: &UnsavedScope) -> UnsavedInventory {
        let mut changes = Vec::new();
        if !matches!(scope, UnsavedScope::Editor(_)) {
            changes.extend(
                self.invalid_inline_edits
                    .iter()
                    .filter(|session_key| scope_matches_session(scope, session_key))
                    .cloned()
                    .map(|session_key| UnsavedChange::InvalidInlineEdit { session_key }),
            );
            for (session_key, session) in self.sessions.iter() {
                if !scope_matches_session(scope, session_key) {
                    continue;
                }
                for doc_key in &session.view.dirty {
                    let Some(document) = session.view.drafts.get(doc_key).cloned() else {
                        continue;
                    };
                    let baseline_document = self.document_for_key(session_key, doc_key);
                    let original_id = baseline_document
                        .as_ref()
                        .and_then(|document| document.get("_id").cloned())
                        .or_else(|| parse_bson_from_relaxed_json(doc_key.as_str()).ok());
                    changes.push(UnsavedChange::InlineDocument {
                        session_key: session_key.clone(),
                        doc_key: doc_key.clone(),
                        original_id,
                        baseline_document,
                        document,
                    });
                }
            }
        }

        changes.extend(
            self.editor_sessions
                .snapshots()
                .into_iter()
                .filter(|session| session.is_dirty() || session.save_in_flight)
                .filter(|session| scope_matches_editor(scope, session))
                .map(UnsavedChange::DetachedEditor),
        );

        UnsavedInventory { changes }
    }

    pub fn discard_unsaved(&mut self, scope: &UnsavedScope, cx: &mut Context<Self>) {
        let inventory = self.unsaved_inventory(scope);
        let mut affected_sessions = Vec::new();
        for change in inventory.changes {
            match change {
                UnsavedChange::InlineDocument { session_key, doc_key, .. } => {
                    self.clear_draft(&session_key, &doc_key);
                    affected_sessions.push(session_key);
                }
                UnsavedChange::InvalidInlineEdit { session_key } => {
                    self.invalid_inline_edits.remove(&session_key);
                }
                UnsavedChange::DetachedEditor(session) => {
                    let handle = self.editor_sessions.window_handle(session.id);
                    self.editor_sessions.close(session.id);
                    if let Some(handle) = handle {
                        let _ = handle.update(cx, |_root, window, _cx| window.remove_window());
                    }
                }
            }
        }
        affected_sessions.sort_by(|left, right| {
            (&left.connection_id, &left.database, &left.collection).cmp(&(
                &right.connection_id,
                &right.database,
                &right.collection,
            ))
        });
        affected_sessions.dedup();
        for session_key in affected_sessions {
            let dirty = self.session_view(&session_key).is_some_and(|view| !view.dirty.is_empty());
            self.set_collection_dirty(session_key, dirty, cx);
        }
        cx.notify();
    }
}

fn scope_matches_session(scope: &UnsavedScope, session_key: &SessionKey) -> bool {
    match scope {
        UnsavedScope::Tab(TabKey::Collection(key)) | UnsavedScope::Preview(key) => {
            key == session_key
        }
        UnsavedScope::Tab(_) | UnsavedScope::Editor(_) => false,
        UnsavedScope::Connection(connection_id) => session_key.connection_id == *connection_id,
        UnsavedScope::Workspace | UnsavedScope::App => true,
    }
}

fn scope_matches_editor(scope: &UnsavedScope, session: &EditorSession) -> bool {
    match scope {
        UnsavedScope::Editor(id) => session.id == *id,
        UnsavedScope::Tab(TabKey::Collection(key)) | UnsavedScope::Preview(key) => {
            session.session_key == *key
        }
        UnsavedScope::Tab(_) => false,
        UnsavedScope::Connection(connection_id) => {
            session.session_key.connection_id == *connection_id
        }
        UnsavedScope::Workspace | UnsavedScope::App => true,
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{Bson, doc};

    use super::*;

    #[test]
    fn inventory_is_scoped_to_the_requested_collection() {
        let mut state = AppState::new();
        let connection_id = Uuid::new_v4();
        let first = SessionKey::new(connection_id, "db", "first");
        let second = SessionKey::new(connection_id, "db", "second");
        state.ensure_session(first.clone());
        state.ensure_session(second.clone());
        let first_doc = doc! { "_id": 1, "value": "one" };
        let second_doc = doc! { "_id": 2, "value": "two" };
        let first_key = DocumentKey::from_id(&Bson::Int32(1));
        let second_key = DocumentKey::from_id(&Bson::Int32(2));
        state.set_draft(&first, first_key, first_doc);
        state.set_draft(&second, second_key, second_doc);

        let inventory = state.unsaved_inventory(&UnsavedScope::Tab(TabKey::Collection(first)));

        assert_eq!(inventory.len(), 1);
    }

    #[test]
    fn editor_inventory_ignores_unchanged_sessions() {
        let state = AppState::new();
        let session_key = SessionKey::new(Uuid::new_v4(), "db", "col");
        let editor_id = state.editor_sessions.create_insert_session(session_key, "{}".into());
        assert!(state.unsaved_inventory(&UnsavedScope::Editor(editor_id)).is_empty());

        state.editor_sessions.update_content(editor_id, "{\"value\":1}".into());
        assert_eq!(state.unsaved_inventory(&UnsavedScope::Editor(editor_id)).len(), 1);
    }

    #[test]
    fn invalid_inline_edits_are_scoped_and_never_treated_as_clean() {
        let mut state = AppState::new();
        let connection_id = Uuid::new_v4();
        let first = SessionKey::new(connection_id, "db", "first");
        let second = SessionKey::new(connection_id, "db", "second");
        state.set_invalid_inline_edit(first.clone(), true);

        assert_eq!(state.unsaved_inventory(&UnsavedScope::Tab(TabKey::Collection(first))).len(), 1);
        assert!(state.unsaved_inventory(&UnsavedScope::Tab(TabKey::Collection(second))).is_empty());
    }

    #[test]
    fn drafts_without_resolvable_ids_remain_in_the_inventory() {
        let mut state = AppState::new();
        let session_key = SessionKey::new(Uuid::new_v4(), "db", "col");
        state.ensure_session(session_key.clone());
        let key = DocumentKey::from_document(&doc! { "value": 1 }, 7);
        state.set_draft(&session_key, key, doc! { "value": 2 });

        let inventory =
            state.unsaved_inventory(&UnsavedScope::Tab(TabKey::Collection(session_key)));

        assert_eq!(inventory.len(), 1);
        assert!(matches!(
            &inventory.changes[0],
            UnsavedChange::InlineDocument { original_id: None, .. }
        ));
    }
}
