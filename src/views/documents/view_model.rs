//! View-model for document tree rendering and editing behavior.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui::*;
use gpui_component::input::{InputEvent, InputState, NumberInputEvent, StepAction};
use gpui_component::table::TableState;
use gpui_component::tree::{TreeItem, TreeState};
use mongodb::bson::Bson;

use crate::bson::{DocumentKey, PathSegment, bson_value_for_edit, parse_edited_value};
use crate::perf::log_tabs_duration;
use crate::state::{AppState, SessionKey};
use crate::views::documents::dialogs::property_dialog::PropertyActionDialog;
use crate::views::documents::node_meta::NodeMeta;
use crate::views::documents::table::document_table_delegate::DocumentTableDelegate;
use crate::views::documents::tree::document_tree::{build_documents_tree, flatten_tree_order_all};
use crate::views::documents::types::InlineEditor;

use super::CollectionView;

struct CachedTreeState {
    generation: u64,
    items: Vec<TreeItem>,
    meta: Arc<HashMap<String, NodeMeta>>,
    order: Vec<String>,
    order_all: Vec<String>,
    selected_index: Option<usize>,
}

pub struct DocumentViewModel {
    tree_state: Entity<TreeState>,
    current_session: Option<SessionKey>,
    node_meta: Arc<HashMap<String, NodeMeta>>,
    tree_items: Vec<TreeItem>,
    tree_order: Vec<String>,
    tree_order_all: Vec<String>,
    tree_cache: HashMap<SessionKey, CachedTreeState>,
    inline_editor_state: Option<InlineEditor>,
    inline_editor_subscription: Option<Subscription>,
    inline_blur_subscription: Option<Subscription>,
    editing_node_id: Option<String>,
    editing_doc_key: Option<DocumentKey>,
    editing_path: Vec<PathSegment>,
    editing_original: Option<Bson>,
    table_state: Option<Entity<TableState<DocumentTableDelegate>>>,
    table_generation: Option<u64>,
    col_visibility_search: Option<Entity<InputState>>,
}

impl DocumentViewModel {
    pub fn new(cx: &mut Context<CollectionView>) -> Self {
        Self {
            tree_state: cx.new(|cx| TreeState::new(cx)),
            current_session: None,
            node_meta: Arc::new(HashMap::new()),
            tree_items: Vec::new(),
            tree_order: Vec::new(),
            tree_order_all: Vec::new(),
            tree_cache: HashMap::new(),
            inline_editor_state: None,
            inline_editor_subscription: None,
            inline_blur_subscription: None,
            editing_node_id: None,
            editing_doc_key: None,
            editing_path: Vec::new(),
            editing_original: None,
            table_state: None,
            table_generation: None,
            col_visibility_search: None,
        }
    }

    pub fn tree_state(&self) -> Entity<TreeState> {
        self.tree_state.clone()
    }

    pub fn table_state(&self) -> Option<&Entity<TableState<DocumentTableDelegate>>> {
        self.table_state.as_ref()
    }

    pub fn node_meta(&self) -> Arc<HashMap<String, NodeMeta>> {
        self.node_meta.clone()
    }

    pub fn tree_order_all(&self) -> &[String] {
        &self.tree_order_all
    }

    pub fn tree_order(&self) -> &[String] {
        &self.tree_order
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
        let start = Instant::now();
        let prev = self.current_session.clone();
        if self.current_session == next {
            log_tabs_duration("documents.set_current_session.noop", start, || {
                "unchanged=true".to_string()
            });
            return false;
        }

        let next_loaded = next
            .as_ref()
            .and_then(|key| state.read(cx).session_data(key).map(|data| data.loaded))
            .unwrap_or(false);

        // Save current tree state to cache before switching away.
        if let Some(prev_key) = self.current_session.clone() {
            let generation = state.read(cx).session(&prev_key).map(|s| s.generation).unwrap_or(0);
            let selected_index = self.tree_state.read(cx).selected_index();
            self.tree_cache.insert(
                prev_key,
                CachedTreeState {
                    generation,
                    items: std::mem::take(&mut self.tree_items),
                    meta: self.node_meta.clone(),
                    order: std::mem::take(&mut self.tree_order),
                    order_all: std::mem::take(&mut self.tree_order_all),
                    selected_index,
                },
            );
        }
        self.current_session = next;

        // For loaded sessions, avoid rendering an intermediate empty tree during tab switches.
        if next_loaded {
            self.node_meta = Arc::new(HashMap::new());
            self.tree_items.clear();
            self.tree_order.clear();
            self.tree_order_all.clear();
            self.inline_editor_state = None;
            self.inline_editor_subscription = None;
            self.clear_inline_edit();
        } else {
            self.reset_view_state(cx);
        }
        self.sync_dirty_state(state, cx);
        log_tabs_duration("documents.set_current_session", start, || {
            let from = prev
                .as_ref()
                .map(|s| format!("{}/{}", s.database, s.collection))
                .unwrap_or_else(|| "-".to_string());
            let to = self
                .current_session
                .as_ref()
                .map(|s| format!("{}/{}", s.database, s.collection))
                .unwrap_or_else(|| "-".to_string());
            format!("from={from} to={to} next_loaded={next_loaded}")
        });
        true
    }

    pub fn reset_view_state(&mut self, cx: &mut Context<CollectionView>) {
        self.node_meta = Arc::new(HashMap::new());
        self.tree_items.clear();
        self.tree_order.clear();
        self.tree_order_all.clear();
        self.inline_editor_state = None;
        self.inline_editor_subscription = None;
        self.clear_inline_edit();

        self.tree_state.update(cx, |tree, cx| {
            tree.set_items(Vec::<TreeItem>::new(), cx);
            tree.set_selected_index(None, cx);
        });
    }

    pub fn rebuild_tree(&mut self, state: &Entity<AppState>, cx: &mut Context<CollectionView>) {
        let start = Instant::now();
        let Some(session_key) = self.current_session.clone() else {
            return;
        };
        let state_ref = state.read(cx);
        let Some(session) = state_ref.session(&session_key) else {
            return;
        };

        // Try to restore from cache if the underlying data hasn't changed.
        let generation = session.generation;
        if let Some(cached) = self.tree_cache.remove(&session_key)
            && cached.generation == generation
        {
            self.node_meta = cached.meta;
            self.tree_items = cached.items.clone();
            self.tree_order = cached.order;
            self.tree_order_all = cached.order_all;

            self.tree_state.update(cx, |tree, cx| {
                tree.set_items(cached.items, cx);
                tree.set_selected_index(cached.selected_index, cx);
            });
            let items = self.tree_order_all.len();
            log_tabs_duration("documents.rebuild_tree", start, || {
                format!(
                    "cache=hit generation={generation} items={items} session={}/{}",
                    session_key.database, session_key.collection
                )
            });
            return;
        }

        let data = &session.data;
        let view = &session.view;

        let (items, meta, order) =
            build_documents_tree(&data.items, &view.drafts, &view.expanded_nodes, cx);
        let mut full_order = Vec::new();
        for item in &items {
            flatten_tree_order_all(item, &mut full_order);
        }

        let selected_index = view
            .selected_node_id
            .as_ref()
            .and_then(|id| order.iter().position(|entry| entry == id));

        self.node_meta = Arc::new(meta);
        self.tree_items = items.clone();
        self.tree_order = order;
        self.tree_order_all = full_order;

        self.tree_state.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            tree.set_selected_index(selected_index, cx);
        });
        let items = self.tree_order_all.len();
        log_tabs_duration("documents.rebuild_tree", start, || {
            format!(
                "cache=miss generation={generation} items={items} session={}/{}",
                session_key.database, session_key.collection
            )
        });
    }

    pub fn sync_dirty_state(&self, state: &Entity<AppState>, cx: &mut Context<CollectionView>) {
        let Some(session) = self.current_session.clone() else {
            return;
        };
        let dirty =
            state.read(cx).session_view(&session).is_some_and(|view| !view.dirty.is_empty());
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
        self.inline_blur_subscription = None;
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
        if let Some(Bson::String(text)) = value
            && (text.contains('\n') || text.contains('\r'))
        {
            let Some(session_key) = self.current_session.clone() else {
                return;
            };
            let allow_bulk =
                !meta.path.iter().any(|segment| matches!(segment, PathSegment::Index(_)));
            PropertyActionDialog::open_edit_value(
                state.clone(),
                session_key,
                meta.clone(),
                allow_bulk,
                window,
                cx,
            );
            return;
        }
        let mut focus_state: Option<Entity<InputState>> = None;
        let editor = match value {
            Some(Bson::Boolean(current)) => InlineEditor::Bool(*current),
            Some(Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_)) => {
                let state = cx.new(|cx| InputState::new(window, cx));
                if let Some(value) = value.map(bson_value_for_edit) {
                    state.update(cx, |state, cx| {
                        state.set_value(value, window, cx);
                    });
                }
                focus_state = Some(state.clone());
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
                focus_state = Some(state.clone());
                InlineEditor::Text(state)
            }
        };

        self.inline_editor_state = Some(editor);
        if let Some(state) = &focus_state {
            let blur_sub = cx.subscribe_in(state, window, |view, _state, event, _window, cx| {
                if matches!(event, InputEvent::Blur) {
                    view.view_model.clear_inline_edit();
                    cx.notify();
                }
            });
            self.inline_blur_subscription = Some(blur_sub);
        }
        if let Some(state) = focus_state {
            let focus = state.read(cx).focus_handle(cx);
            window.defer(cx, move |window, _cx| {
                window.focus(&focus);
            });
        }

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

    pub fn ensure_table_state(
        &mut self,
        state: &Entity<AppState>,
        view: &Entity<CollectionView>,
        window: &mut Window,
        cx: &mut Context<CollectionView>,
    ) -> Entity<TableState<DocumentTableDelegate>> {
        if let Some(table_state) = &self.table_state {
            return table_state.clone();
        }
        let delegate =
            DocumentTableDelegate::new(state.clone(), view.clone(), self.current_session.clone());
        let table_state = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .col_selectable(false)
                .col_movable(true)
                .row_selectable(false)
        });

        // Subscribe to table events for row selection and double-click.
        let state_clone = state.clone();
        let view_clone = view.clone();
        cx.subscribe_in(&table_state, window, move |cv, ts, event, window, cx| {
            use gpui_component::table::TableEvent;
            match event {
                TableEvent::SelectRow(_) => {}
                TableEvent::DoubleClickedRow(row_ix) => {
                    let row_ix = *row_ix;
                    let session_key = cv.view_model.current_session();
                    let doc_key = ts.read(cx).delegate().document_key(row_ix);
                    if let (Some(sk), Some(dk)) = (session_key, doc_key) {
                        CollectionView::open_document_json_editor(
                            view_clone.clone(),
                            state_clone.clone(),
                            sk,
                            dk,
                            window,
                            cx,
                        );
                    }
                }
                TableEvent::ColumnWidthsChanged(widths) => {
                    let col_widths: HashMap<String, f32> = {
                        let delegate = ts.read(cx).delegate();
                        widths
                            .iter()
                            .enumerate()
                            .filter_map(|(i, w)| delegate.column_key(i).map(|k| (k, f32::from(*w))))
                            .collect()
                    };
                    ts.update(cx, |ts, _cx| {
                        ts.delegate_mut().update_saved_widths(col_widths.clone());
                    });
                    if let Some(sk) = cv.view_model.current_session() {
                        cv.state.update(cx, |state, cx| {
                            state.set_table_column_widths(&sk, col_widths);
                            cx.notify();
                        });
                    }
                }
                TableEvent::MoveColumn(from_ix, to_ix) => {
                    let from_ix = *from_ix;
                    let to_ix = *to_ix;
                    let order = {
                        ts.update(cx, |ts, _cx| {
                            ts.delegate_mut().apply_column_move(from_ix, to_ix);
                            ts.delegate_mut().column_order()
                        })
                    };
                    if let Some(sk) = cv.view_model.current_session() {
                        cv.state.update(cx, |state, cx| {
                            state.set_table_column_order(&sk, order);
                            cx.notify();
                        });
                    }
                    cv.view_model.invalidate_table();
                    cx.notify();
                }
                _ => {}
            }
        })
        .detach();

        self.table_state = Some(table_state.clone());
        table_state
    }

    pub fn rebuild_table(
        &mut self,
        state: &Entity<AppState>,
        view: &Entity<CollectionView>,
        window: &mut Window,
        cx: &mut Context<CollectionView>,
    ) {
        let Some(session_key) = self.current_session.clone() else {
            return;
        };
        let state_ref = state.read(cx);
        let Some(session) = state_ref.session(&session_key) else {
            return;
        };
        let generation = session.generation;
        let generation_changed =
            self.table_generation != Some(generation) || self.table_state.is_none();
        let selected_docs = session.view.selected_docs.clone();

        if generation_changed {
            let documents = session.data.items.clone();
            let drafts = session.view.drafts.clone();
            let is_loading = session.data.is_loading;
            let saved_widths = session.view.table_column_widths.clone();
            let saved_order = session.view.table_column_order.clone();
            let pinned = session.view.table_pinned_columns.clone();
            let hidden = session.view.table_hidden_columns.clone();

            let table_state = self.ensure_table_state(state, view, window, cx);
            table_state.update(cx, |ts, cx| {
                ts.delegate_mut().set_saved_widths(saved_widths);
                ts.delegate_mut().set_column_order(saved_order);
                ts.delegate_mut().set_pinned_columns(pinned);
                ts.delegate_mut().set_hidden_columns(hidden);
                ts.delegate_mut().set_selected_doc_keys(selected_docs);
                ts.delegate_mut().refresh_data(documents, drafts, Some(session_key), is_loading);
                ts.refresh(cx);
            });
            self.table_generation = Some(generation);
        } else {
            let table_state = self.ensure_table_state(state, view, window, cx);
            table_state.update(cx, |ts, cx| {
                ts.delegate_mut().set_selected_doc_keys(selected_docs);
                cx.notify();
            });
        }
    }

    pub fn invalidate_table(&mut self) {
        self.table_generation = None;
    }

    pub fn ensure_col_visibility_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<CollectionView>,
    ) -> Entity<InputState> {
        if let Some(ref state) = self.col_visibility_search {
            return state.clone();
        }
        let state = cx.new(|cx| InputState::new(window, cx).placeholder("Search columns..."));
        self.col_visibility_search = Some(state.clone());
        state
    }
}
