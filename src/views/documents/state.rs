use gpui::*;
use gpui_component::input::InputState;

use mongodb::bson::Document;

use crate::bson::DocumentKey;
use crate::state::{AppCommands, AppEvent, AppState, SessionKey};

use super::node_meta::NodeMeta;
use super::view_model::DocumentViewModel;

/// View for browsing documents in a collection
pub struct CollectionView {
    pub(crate) state: Entity<AppState>,
    pub(crate) view_model: DocumentViewModel,
    pub(crate) documents_focus: FocusHandle,
    pub(crate) filter_state: Option<Entity<InputState>>,
    pub(crate) sort_state: Option<Entity<InputState>>,
    pub(crate) projection_state: Option<Entity<InputState>>,
    pub(crate) search_state: Option<Entity<InputState>>,
    pub(crate) search_visible: bool,
    pub(crate) search_matches: Vec<String>,
    pub(crate) search_index: Option<usize>,
    pub(crate) input_session: Option<SessionKey>,
    pub(crate) filter_subscription: Option<Subscription>,
    pub(crate) sort_subscription: Option<Subscription>,
    pub(crate) projection_subscription: Option<Subscription>,
    pub(crate) search_subscription: Option<Subscription>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

impl CollectionView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];

        let weak_view = cx.entity().downgrade();
        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            let Some(view) = weak_view.upgrade() else {
                return;
            };
            let key = event.keystroke.key.to_ascii_lowercase();
            let is_escape = key == "escape";
            let is_enter = key == "enter" || key == "return";
            let is_arrow = key == "up" || key == "down" || key == "left" || key == "right";

            if !is_escape && !is_enter && !is_arrow {
                return;
            }

            // Handle arrow keys for tree navigation
            if is_arrow {
                view.update(cx, |this, cx| {
                    // Only navigate when documents_focus is focused and no inline editing
                    if !this.documents_focus.is_focused(window) {
                        return;
                    }
                    if this.view_model.inline_state().is_some() {
                        return;
                    }
                    let Some(session_key) = this.view_model.current_session() else {
                        return;
                    };

                    let count = this.view_model.tree_order().len();
                    if count == 0 {
                        return;
                    }

                    let current_ix =
                        this.view_model.tree_state().read(cx).selected_index().unwrap_or(0);
                    let current_node_id = this.view_model.tree_order().get(current_ix).cloned();

                    match key.as_str() {
                        "up" => {
                            let new_ix = if current_ix == 0 { count - 1 } else { current_ix - 1 };
                            Self::select_tree_index(this, new_ix, &session_key, cx);
                            cx.stop_propagation();
                        }
                        "down" => {
                            let new_ix = if current_ix >= count - 1 { 0 } else { current_ix + 1 };
                            Self::select_tree_index(this, new_ix, &session_key, cx);
                            cx.stop_propagation();
                        }
                        "left" => {
                            // 1. Collapse if expanded folder
                            // 2. Otherwise move to parent
                            if let Some(node_id) = current_node_id {
                                let node_meta = this.view_model.node_meta();
                                let is_folder =
                                    node_meta.get(&node_id).is_some_and(|m| m.is_folder);
                                let is_expanded = this
                                    .state
                                    .read(cx)
                                    .session_view(&session_key)
                                    .is_some_and(|v| v.expanded_nodes.contains(&node_id));
                                if is_folder && is_expanded {
                                    this.state.update(cx, |state, cx| {
                                        state.toggle_expanded_node(&session_key, &node_id);
                                        cx.notify();
                                    });
                                    this.view_model.rebuild_tree(&this.state, cx);
                                    cx.notify();
                                } else {
                                    // Move to parent: strip last path segment from node_id
                                    let parent_id = node_id.rfind('/').map(|i| &node_id[..i]);
                                    if let Some(parent_id) = parent_id {
                                        let parent_ix = this
                                            .view_model
                                            .tree_order()
                                            .iter()
                                            .position(|id| id == parent_id);
                                        if let Some(parent_ix) = parent_ix {
                                            Self::select_tree_index(
                                                this,
                                                parent_ix,
                                                &session_key,
                                                cx,
                                            );
                                        }
                                    }
                                }
                            }
                            cx.stop_propagation();
                        }
                        "right" => {
                            // 1. Expand if collapsed folder
                            // 2. Move to first child if expanded folder
                            if let Some(node_id) = current_node_id {
                                let node_meta = this.view_model.node_meta();
                                let is_folder =
                                    node_meta.get(&node_id).is_some_and(|m| m.is_folder);
                                let is_expanded = this
                                    .state
                                    .read(cx)
                                    .session_view(&session_key)
                                    .is_some_and(|v| v.expanded_nodes.contains(&node_id));
                                if is_folder && !is_expanded {
                                    this.state.update(cx, |state, cx| {
                                        state.toggle_expanded_node(&session_key, &node_id);
                                        cx.notify();
                                    });
                                    this.view_model.rebuild_tree(&this.state, cx);
                                    cx.notify();
                                } else if is_folder && is_expanded && current_ix + 1 < count {
                                    // Move to first child (next item in DFS order)
                                    Self::select_tree_index(this, current_ix + 1, &session_key, cx);
                                }
                            }
                            cx.stop_propagation();
                        }
                        _ => {}
                    }
                });
                return;
            }
            view.update(cx, |this, cx| {
                let mut handled = false;
                let save_selected_document = |this: &mut CollectionView, cx: &mut Context<Self>| {
                    let Some(session_key) = this.view_model.current_session() else {
                        return false;
                    };
                    let (doc_key, doc) = {
                        let state_ref = this.state.read(cx);
                        let doc_key = state_ref.session_selected_doc(&session_key);
                        let doc = doc_key
                            .as_ref()
                            .and_then(|doc_key| state_ref.session_draft(&session_key, doc_key));
                        (doc_key, doc)
                    };
                    let (Some(doc_key), Some(doc)) = (doc_key, doc) else {
                        return false;
                    };
                    AppCommands::save_document(this.state.clone(), session_key, doc_key, doc, cx);
                    true
                };
                if is_escape {
                    if this.search_visible {
                        this.close_search(window, cx);
                        handled = true;
                    }
                    if this.view_model.inline_state().is_some()
                        || this.view_model.editing_node_id().is_some()
                    {
                        this.view_model.clear_inline_edit();
                        window.focus(&this.documents_focus);
                        handled = true;
                    }
                } else if is_enter {
                    let modifiers = event.keystroke.modifiers;
                    let cmd_or_ctrl = modifiers.secondary() || modifiers.control;
                    if this.view_model.inline_state().is_some() {
                        this.view_model.commit_inline_edit(&this.state, cx);
                        let committed = this.view_model.inline_state().is_none();
                        if committed {
                            window.focus(&this.documents_focus);
                            if cmd_or_ctrl {
                                save_selected_document(this, cx);
                            }
                        }
                        handled = true;
                    } else if cmd_or_ctrl {
                        handled = save_selected_document(this, cx);
                    } else if this.documents_focus.is_focused(window) {
                        let Some(session_key) = this.view_model.current_session() else {
                            return;
                        };
                        let selected_node =
                            this.state.read(cx).session_selected_node_id(&session_key);
                        if let Some(node_id) = selected_node {
                            let node_meta = this.view_model.node_meta();
                            if let Some(meta) = node_meta.get(&node_id)
                                && meta.is_editable
                            {
                                this.view_model.begin_inline_edit(
                                    node_id.clone(),
                                    meta,
                                    window,
                                    &this.state,
                                    cx,
                                );
                                handled = true;
                            }
                        }
                    }
                }
                if handled {
                    cx.notify();
                    cx.stop_propagation();
                }
            });
        }));

        let current_session = {
            let state_ref = state.read(cx);
            let session = state_ref.current_session_key();
            if let Some(session_key) = session.clone() {
                let should_load = state_ref
                    .session_data(&session_key)
                    .map(|data| data.items.is_empty())
                    .unwrap_or(true);
                if should_load {
                    AppCommands::load_documents_for_session(state.clone(), session_key, cx);
                }
            }
            session
        };

        let mut view_model = DocumentViewModel::new(cx);
        view_model.set_current_session(current_session.clone(), &state, cx);

        subscriptions.push(cx.subscribe(&state, |this, state, event, cx| match event {
            AppEvent::ViewChanged | AppEvent::Connected(_) => {
                let state_ref = state.read(cx);
                let next_session = state_ref.current_session_key();
                let should_load = next_session
                    .as_ref()
                    .map(|session| {
                        state_ref
                            .session_data(session)
                            .map(|data| data.items.is_empty())
                            .unwrap_or(true)
                    })
                    .unwrap_or(false);
                if this.view_model.set_current_session(next_session.clone(), &state, cx) {
                    if let Some(session) = next_session {
                        if should_load {
                            AppCommands::load_documents_for_session(state.clone(), session, cx);
                        } else {
                            this.view_model.rebuild_tree(&state, cx);
                        }
                    }
                    this.update_search_results(cx);
                    cx.notify();
                }
            }
            AppEvent::DocumentsLoaded { session, .. } => {
                if !this.view_model.is_current_session(session) {
                    return;
                }
                this.view_model.clear_inline_edit();
                this.view_model.rebuild_tree(&state, cx);
                this.view_model.sync_dirty_state(&state, cx);
                this.update_search_results(cx);
                cx.notify();
            }
            AppEvent::DocumentSaved { session, document } => {
                if !this.view_model.is_current_session(session) {
                    return;
                }
                if this.view_model.is_editing_doc(document) {
                    this.view_model.clear_inline_edit();
                }
                this.view_model.rebuild_tree(&state, cx);
                this.view_model.sync_dirty_state(&state, cx);
                this.update_search_results(cx);
                cx.notify();
            }
            AppEvent::DocumentDeleted { session, document } => {
                if !this.view_model.is_current_session(session) {
                    return;
                }
                if this.view_model.is_editing_doc(document) {
                    this.view_model.clear_inline_edit();
                }
                this.view_model.rebuild_tree(&state, cx);
                this.view_model.sync_dirty_state(&state, cx);
                this.update_search_results(cx);
                cx.notify();
            }
            AppEvent::DocumentSaveFailed { session, .. } => {
                if this.view_model.is_current_session(session) {
                    cx.notify();
                }
            }
            AppEvent::DocumentDeleteFailed { session, .. } => {
                if this.view_model.is_current_session(session) {
                    cx.notify();
                }
            }
            _ => {}
        }));

        Self {
            state,
            view_model,
            documents_focus: cx.focus_handle(),
            filter_state: None,
            sort_state: None,
            projection_state: None,
            search_state: None,
            search_visible: false,
            search_matches: Vec::new(),
            search_index: None,
            input_session: None,
            filter_subscription: None,
            sort_subscription: None,
            projection_subscription: None,
            search_subscription: None,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn selected_doc_key(
        &self,
        session_key: &SessionKey,
        cx: &App,
    ) -> Option<DocumentKey> {
        self.state.read(cx).session_selected_doc(session_key)
    }

    pub(crate) fn selected_doc_key_for_current_session(
        &self,
        cx: &App,
    ) -> Option<(SessionKey, DocumentKey)> {
        let session_key = self.view_model.current_session()?;
        let doc_key = self.selected_doc_key(&session_key, cx)?;
        Some((session_key, doc_key))
    }

    pub(crate) fn selected_draft(
        &self,
        session_key: &SessionKey,
        doc_key: &DocumentKey,
        cx: &App,
    ) -> Option<Document> {
        self.state.read(cx).session_draft(session_key, doc_key)
    }

    pub(crate) fn resolve_document(
        &self,
        session_key: &SessionKey,
        doc_key: &DocumentKey,
        cx: &App,
    ) -> Option<Document> {
        self.state.read(cx).session_draft_or_document(session_key, doc_key)
    }

    pub(crate) fn selected_document_for_current_session(
        &self,
        cx: &App,
    ) -> Option<(SessionKey, DocumentKey, Document)> {
        let (session_key, doc_key) = self.selected_doc_key_for_current_session(cx)?;
        let doc = self.resolve_document(&session_key, &doc_key, cx)?;
        Some((session_key, doc_key, doc))
    }

    pub(crate) fn selected_draft_for_current_session(
        &self,
        cx: &App,
    ) -> Option<(SessionKey, DocumentKey, Document)> {
        let (session_key, doc_key) = self.selected_doc_key_for_current_session(cx)?;
        let doc = self.selected_draft(&session_key, &doc_key, cx)?;
        Some((session_key, doc_key, doc))
    }

    pub(crate) fn current_search_query(&self, cx: &mut Context<Self>) -> Option<String> {
        let raw = self
            .search_state
            .as_ref()
            .map(|state| state.read(cx).value().to_string())
            .unwrap_or_default();
        let trimmed = raw.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_lowercase()) }
    }

    pub(crate) fn show_search_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_visible = true;
        if let Some(search_state) = self.search_state.clone() {
            search_state.update(cx, |state, cx| {
                state.focus(window, cx);
            });
        }
        self.update_search_results(cx);
        cx.notify();
    }

    pub(crate) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_visible = false;
        window.blur();
        if let Some(search_state) = self.search_state.clone() {
            search_state.update(cx, |state, cx| {
                state.set_value(String::new(), window, cx);
            });
        }
        self.search_matches.clear();
        self.search_index = None;
    }

    pub(crate) fn update_search_results(&mut self, cx: &mut Context<Self>) {
        let Some(query) = self.current_search_query(cx) else {
            self.search_matches.clear();
            self.search_index = None;
            return;
        };

        let node_meta = self.view_model.node_meta();
        let mut matches = Vec::new();
        for node_id in self.view_model.tree_order_all() {
            let Some(meta) = node_meta.get(node_id) else {
                continue;
            };
            if meta.path.is_empty() {
                continue;
            }
            if meta.value_label.to_lowercase().contains(&query) {
                matches.push(node_id.clone());
            }
        }

        self.search_matches = matches;
        if self.search_matches.is_empty() {
            self.search_index = None;
            return;
        }

        self.search_index = Some(0);
        let view = cx.entity();
        cx.defer(move |cx| {
            view.update(cx, |this, cx| {
                this.go_to_match(0, cx);
                cx.notify();
            });
        });
    }

    pub(crate) fn next_match(&mut self, cx: &mut Context<Self>) {
        let total = self.search_matches.len();
        if total == 0 {
            return;
        }
        let next = match self.search_index {
            Some(index) => (index + 1) % total,
            None => 0,
        };
        self.search_index = Some(next);
        self.go_to_match(next, cx);
    }

    pub(crate) fn prev_match(&mut self, cx: &mut Context<Self>) {
        let total = self.search_matches.len();
        if total == 0 {
            return;
        }
        let prev = match self.search_index {
            Some(0) | None => total.saturating_sub(1),
            Some(index) => index - 1,
        };
        self.search_index = Some(prev);
        self.go_to_match(prev, cx);
    }

    pub(crate) fn selected_property_context(&self, cx: &App) -> Option<(SessionKey, NodeMeta)> {
        let session_key = self.view_model.current_session()?;
        let node_id = self.state.read(cx).session_selected_node_id(&session_key)?;
        let node_meta = self.view_model.node_meta();
        let meta = node_meta.get(&node_id).cloned()?;
        Some((session_key, meta))
    }

    fn select_tree_index(
        this: &mut CollectionView,
        new_ix: usize,
        session_key: &SessionKey,
        cx: &mut Context<Self>,
    ) {
        let tree_state = this.view_model.tree_state();
        let tree_order = this.view_model.tree_order();

        tree_state.update(cx, |tree, cx| {
            tree.set_selected_index(Some(new_ix), cx);
            tree.scroll_to_item(new_ix, ScrollStrategy::Top);
        });

        // Update app state selected node
        if let Some(node_id) = tree_order.get(new_ix) {
            let node_meta = this.view_model.node_meta();
            if let Some(meta) = node_meta.get(node_id) {
                this.state.update(cx, |state, cx| {
                    state.set_selected_node(session_key, meta.doc_key.clone(), node_id.clone());
                    cx.notify();
                });
            }
        }
        cx.notify();
    }

    pub(crate) fn go_to_match(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(match_id) = self.search_matches.get(index).cloned() else {
            return;
        };
        let node_meta = self.view_model.node_meta();
        let Some(meta) = node_meta.get(&match_id).cloned() else {
            return;
        };
        let Some(session_key) = self.view_model.current_session() else {
            return;
        };

        self.state.update(cx, |state, cx| {
            state.expand_path(&session_key, &meta.doc_key, &meta.path);
            state.set_selected_node(&session_key, meta.doc_key.clone(), match_id.clone());
            cx.notify();
        });

        self.view_model.rebuild_tree(&self.state, cx);

        let tree_state = self.view_model.tree_state();
        let order = self.view_model.tree_order();
        if let Some(ix) = order.iter().position(|entry| entry == &match_id) {
            tree_state.update(cx, |tree, cx| {
                tree.set_selected_index(Some(ix), cx);
                tree.scroll_to_item(ix, ScrollStrategy::Center);
            });
        }
    }
}
