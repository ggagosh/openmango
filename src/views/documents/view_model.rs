//! View-model for document tree rendering and editing behavior.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::*;
use gpui_component::input::{InputState, NumberInputEvent, StepAction};
use gpui_component::tree::{TreeItem, TreeState};
use mongodb::bson::Bson;

use crate::bson::{DocumentKey, PathSegment, bson_value_for_edit, parse_edited_value};
use crate::state::{AppState, SessionKey};
use crate::views::documents::document_tree::build_documents_tree;
use crate::views::documents::node_meta::NodeMeta;
use crate::views::documents::types::InlineEditor;

use super::CollectionView;

pub struct DocumentViewModel {
    tree_state: Entity<TreeState>,
    current_session: Option<SessionKey>,
    node_meta: Arc<HashMap<String, NodeMeta>>,
    tree_order: Vec<String>,
    inline_editor_state: Option<InlineEditor>,
    inline_editor_subscription: Option<Subscription>,
    editing_node_id: Option<String>,
    editing_doc_key: Option<DocumentKey>,
    editing_path: Vec<PathSegment>,
    editing_original: Option<Bson>,
}

impl DocumentViewModel {
    pub fn new(cx: &mut Context<CollectionView>) -> Self {
        Self {
            tree_state: cx.new(|cx| TreeState::new(cx)),
            current_session: None,
            node_meta: Arc::new(HashMap::new()),
            tree_order: Vec::new(),
            inline_editor_state: None,
            inline_editor_subscription: None,
            editing_node_id: None,
            editing_doc_key: None,
            editing_path: Vec::new(),
            editing_original: None,
        }
    }

    pub fn tree_state(&self) -> Entity<TreeState> {
        self.tree_state.clone()
    }

    pub fn node_meta(&self) -> Arc<HashMap<String, NodeMeta>> {
        self.node_meta.clone()
    }

    pub fn editing_node_id(&self) -> Option<String> {
        self.editing_node_id.clone()
    }

    pub fn inline_state(&self) -> Option<InlineEditor> {
        self.inline_editor_state.clone()
    }

    pub fn set_inline_bool(&mut self, value: bool) {
        self.inline_editor_state = Some(InlineEditor::Bool(value));
    }

    pub fn current_session(&self) -> Option<SessionKey> {
        self.current_session.clone()
    }

    pub fn is_current_session(&self, session: &SessionKey) -> bool {
        self.current_session.as_ref() == Some(session)
    }

    pub fn is_editing_doc(&self, doc_key: &DocumentKey) -> bool {
        self.editing_doc_key.as_ref() == Some(doc_key)
    }

    pub fn set_current_session(
        &mut self,
        next: Option<SessionKey>,
        state: &Entity<AppState>,
        cx: &mut Context<CollectionView>,
    ) -> bool {
        if self.current_session == next {
            return false;
        }
        self.current_session = next;
        self.reset_view_state(cx);
        self.sync_dirty_state(state, cx);
        true
    }

    pub fn reset_view_state(&mut self, cx: &mut Context<CollectionView>) {
        self.node_meta = Arc::new(HashMap::new());
        self.tree_order.clear();
        self.inline_editor_state = None;
        self.inline_editor_subscription = None;
        self.clear_inline_edit();

        self.tree_state.update(cx, |tree, cx| {
            tree.set_items(Vec::<TreeItem>::new(), cx);
            tree.set_selected_index(None, cx);
        });
    }

    pub fn rebuild_tree(&mut self, state: &Entity<AppState>, cx: &mut Context<CollectionView>) {
        let Some(session_key) = self.current_session.clone() else {
            return;
        };
        let state_ref = state.read(cx);
        let Some(session) = state_ref.session(&session_key) else {
            return;
        };

        let (items, meta, order) = build_documents_tree(
            &session.data.items,
            &session.view.drafts,
            &session.view.expanded_nodes,
        );

        let selected_index = session
            .view
            .selected_node_id
            .as_ref()
            .and_then(|id| order.iter().position(|entry| entry == id));

        self.node_meta = Arc::new(meta);
        self.tree_order = order;

        self.tree_state.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            tree.set_selected_index(selected_index, cx);
        });
    }

    pub fn sync_dirty_state(&self, state: &Entity<AppState>, cx: &mut Context<CollectionView>) {
        let Some(session) = self.current_session.clone() else {
            return;
        };
        let dirty = state
            .read(cx)
            .session(&session)
            .is_some_and(|session_state| !session_state.view.dirty.is_empty());
        state.update(cx, |state, cx| {
            state.set_collection_dirty(session.clone(), dirty, cx);
        });
    }

    pub fn clear_inline_edit(&mut self) {
        self.editing_node_id = None;
        self.editing_doc_key = None;
        self.editing_path.clear();
        self.editing_original = None;
        self.inline_editor_state = None;
        self.inline_editor_subscription = None;
    }

    pub fn begin_inline_edit(
        &mut self,
        node_id: String,
        meta: &NodeMeta,
        window: &mut Window,
        state: &Entity<AppState>,
        cx: &mut Context<CollectionView>,
    ) {
        if !meta.is_editable {
            return;
        }
        self.inline_editor_subscription = None;
        let value = meta.value.as_ref();
        let editor = match value {
            Some(Bson::Boolean(current)) => InlineEditor::Bool(*current),
            Some(Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_)) => {
                let state = cx.new(|cx| InputState::new(window, cx));
                if let Some(value) = value.map(bson_value_for_edit) {
                    state.update(cx, |state, cx| {
                        state.set_value(value, window, cx);
                    });
                }
                let is_float = matches!(value, Some(Bson::Double(_)));
                let subscription =
                    cx.subscribe_in(&state, window, move |_view, state, event, window, cx| {
                        let NumberInputEvent::Step(step) = event;
                        let step = *step;
                        let current = state.read(cx).value().to_string();
                        let next = if is_float {
                            let value = current.parse::<f64>().unwrap_or(0.0);
                            let delta = match step {
                                StepAction::Increment => 1.0,
                                StepAction::Decrement => -1.0,
                            };
                            (value + delta).to_string()
                        } else {
                            let value = current.parse::<i64>().unwrap_or(0);
                            let delta = match step {
                                StepAction::Increment => 1,
                                StepAction::Decrement => -1,
                            };
                            (value + delta).to_string()
                        };
                        state.update(cx, |input, cx| {
                            input.set_value(next, window, cx);
                        });
                    });
                self.inline_editor_subscription = Some(subscription);
                InlineEditor::Number(state)
            }
            _ => {
                let placeholder = match value {
                    Some(Bson::DateTime(_)) => Some("2024-01-01T00:00:00Z"),
                    Some(Bson::ObjectId(_)) => Some("507f1f77bcf86cd799439011"),
                    Some(Bson::Null) => Some("null"),
                    _ => None,
                };
                let state = cx.new(|cx| {
                    let mut state = InputState::new(window, cx);
                    if let Some(text) = placeholder {
                        state = state.placeholder(text);
                    }
                    state
                });
                if let Some(value) = value.map(bson_value_for_edit) {
                    state.update(cx, |state, cx| {
                        state.set_value(value, window, cx);
                    });
                }
                InlineEditor::Text(state)
            }
        };

        self.inline_editor_state = Some(editor);

        self.editing_node_id = Some(node_id);
        self.editing_doc_key = Some(meta.doc_key.clone());
        let path = meta.path.clone();
        if let Some(session_key) = self.current_session.clone() {
            state.update(cx, |state, cx| {
                state.expand_path(&session_key, &meta.doc_key, &path);
                cx.notify();
            });
        }
        self.editing_path = path;
        self.editing_original = meta.value.clone();
    }

    pub fn commit_inline_edit(
        &mut self,
        state: &Entity<AppState>,
        cx: &mut Context<CollectionView>,
    ) {
        let Some(doc_key) = self.editing_doc_key.clone() else {
            return;
        };
        let Some(original) = self.editing_original.clone() else {
            return;
        };
        let Some(editor_state) = self.inline_editor_state.as_ref() else {
            return;
        };
        let path = self.editing_path.clone();
        let result = match (editor_state, &original) {
            (InlineEditor::Bool(value), Bson::Boolean(_)) => Ok(Bson::Boolean(*value)),
            (InlineEditor::Text(state), _) => {
                let input = state.read(cx).value().to_string();
                parse_edited_value(&original, &input)
            }
            (InlineEditor::Number(state), _) => {
                let input = state.read(cx).value().to_string();
                parse_edited_value(&original, &input)
            }
            _ => Err("Unsupported inline editor state".to_string()),
        };

        match result {
            Ok(new_value) => {
                if new_value == original {
                    self.clear_inline_edit();
                    return;
                }

                if self.update_draft_value(state, &doc_key, &path, new_value, cx) {
                    self.clear_inline_edit();
                    self.rebuild_tree(state, cx);
                }
            }
            Err(err) => {
                log::warn!("Inline edit failed: {err}");
            }
        }
    }

    pub fn update_draft_value(
        &mut self,
        state: &Entity<AppState>,
        doc_key: &DocumentKey,
        path: &[PathSegment],
        new_value: Bson,
        cx: &mut Context<CollectionView>,
    ) -> bool {
        let Some(session_key) = self.current_session.clone() else {
            return false;
        };
        let original = {
            let state_ref = state.read(cx);
            state_ref.document_for_key(&session_key, doc_key)
        };
        let Some(original) = original else {
            return false;
        };

        let mut updated = false;
        state.update(cx, |state, cx| {
            updated = state.update_draft_value(&session_key, doc_key, &original, path, new_value);
            cx.notify();
        });

        if updated {
            self.sync_dirty_state(state, cx);
        }
        updated
    }
}
