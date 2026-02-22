use gpui::*;
use gpui_component::input::InputState;
use gpui_component::tree::TreeState;

use mongodb::bson::Document;
use regex::RegexBuilder;

use std::collections::HashSet;
use std::time::Instant;

use crate::bson::DocumentKey;
use crate::helpers::auto_pair::AutoPairState;
use crate::perf::log_tabs_duration;
use crate::state::{AppCommands, AppEvent, AppState, CollectionSubview, SessionKey, StatusMessage};

use super::node_meta::NodeMeta;
use super::view_model::DocumentViewModel;

/// View for browsing documents in a collection
pub struct CollectionView {
    pub(crate) state: Entity<AppState>,
    pub(crate) view_model: DocumentViewModel,
    pub(crate) documents_focus: FocusHandle,
    pub(crate) aggregation_focus: FocusHandle,
    pub(crate) aggregation_stage_list_scroll: UniformListScrollHandle,
    pub(crate) filter_state: Option<Entity<InputState>>,
    pub(crate) sort_state: Option<Entity<InputState>>,
    pub(crate) projection_state: Option<Entity<InputState>>,
    pub(crate) schema_filter_state: Option<Entity<InputState>>,
    pub(crate) filter_auto_pair: AutoPairState,
    pub(crate) sort_auto_pair: AutoPairState,
    pub(crate) projection_auto_pair: AutoPairState,
    pub(crate) filter_error: bool,
    pub(crate) sort_error: bool,
    pub(crate) projection_error: bool,
    pub(crate) search_state: Option<Entity<InputState>>,
    pub(crate) search_visible: bool,
    pub(crate) search_matches: Vec<String>,
    pub(crate) search_index: Option<usize>,
    pub(crate) search_case_sensitive: bool,
    pub(crate) search_whole_word: bool,
    pub(crate) search_regex: bool,
    pub(crate) search_values_only: bool,
    pub(crate) input_session: Option<SessionKey>,
    pub(crate) schema_filter_session: Option<SessionKey>,
    pub(crate) aggregation_input_session: Option<SessionKey>,
    pub(crate) aggregation_selected_stage: Option<usize>,
    pub(crate) aggregation_stage_count: usize,
    pub(crate) aggregation_drag_over: Option<(usize, bool)>,
    pub(crate) aggregation_drag_source: Option<usize>,
    pub(crate) filter_subscription: Option<Subscription>,
    pub(crate) sort_subscription: Option<Subscription>,
    pub(crate) projection_subscription: Option<Subscription>,
    pub(crate) schema_filter_subscription: Option<Subscription>,
    pub(crate) search_subscription: Option<Subscription>,
    pub(crate) aggregation_stage_body_state: Option<Entity<InputState>>,
    pub(crate) aggregation_results_tree_state: Option<Entity<TreeState>>,
    pub(crate) aggregation_results_scroll: UniformListScrollHandle,
    pub(crate) aggregation_limit_state: Option<Entity<InputState>>,
    pub(crate) aggregation_results_expanded_nodes: HashSet<String>,
    pub(crate) aggregation_results_signature: Option<u64>,
    pub(crate) aggregation_ignore_body_change: bool,
    pub(crate) aggregation_stage_body_subscription: Option<Subscription>,
    pub(crate) aggregation_limit_subscription: Option<Subscription>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

fn handle_aggregation_shortcut(
    view: &mut CollectionView,
    session_key: &SessionKey,
    key: &str,
    modifiers: Modifiers,
    body_focused: bool,
    window: &mut Window,
    cx: &mut Context<CollectionView>,
) -> bool {
    let cmd_or_ctrl = modifiers.secondary() || modifiers.control;
    if !cmd_or_ctrl {
        return false;
    }

    let (selected_stage, stage_count) = {
        let state_ref = view.state.read(cx);
        let Some(session) = state_ref.session(session_key) else {
            return false;
        };
        (session.data.aggregation.selected_stage, session.data.aggregation.stages.len())
    };

    match key {
        "enter" | "return" => {
            AppCommands::run_aggregation(view.state.clone(), session_key.clone(), false, cx);
            true
        }
        "f" if modifiers.shift => {
            let Some(body_state) = view.aggregation_stage_body_state.clone() else {
                return false;
            };
            if !body_focused {
                return false;
            }
            let raw = body_state.read(cx).value().to_string();
            match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(value) => {
                    if let Ok(formatted) = serde_json::to_string_pretty(&value) {
                        body_state.update(cx, |state, cx| {
                            state.set_value(formatted, window, cx);
                        });
                        true
                    } else {
                        false
                    }
                }
                Err(err) => {
                    view.state.update(cx, |state, cx| {
                        state.set_status_message(Some(StatusMessage::error(format!(
                            "Invalid JSON: {err}",
                        ))));
                        cx.notify();
                    });
                    true
                }
            }
        }
        "k" if modifiers.shift => {
            if !body_focused {
                return false;
            }
            let Some(index) = selected_stage else {
                return false;
            };
            if let Some(body_state) = view.aggregation_stage_body_state.clone() {
                body_state.update(cx, |state, cx| {
                    state.set_value("{}".to_string(), window, cx);
                });
            }
            view.state.update(cx, |state, cx| {
                state.set_pipeline_stage_body(session_key, index, "{}".to_string());
                cx.notify();
            });
            true
        }
        "d" if !modifiers.shift => {
            let Some(index) = selected_stage else {
                return false;
            };
            view.state.update(cx, |state, cx| {
                state.duplicate_pipeline_stage(session_key, index);
                state.set_status_message(Some(StatusMessage::info("Stage duplicated")));
                cx.notify();
            });
            true
        }
        "up" if modifiers.shift => {
            let Some(index) = selected_stage else {
                return false;
            };
            if index == 0 {
                return false;
            }
            view.state.update(cx, |state, cx| {
                state.move_pipeline_stage(session_key, index, index - 1);
                state.set_status_message(Some(StatusMessage::info("Stage moved up")));
                cx.notify();
            });
            true
        }
        "down" if modifiers.shift => {
            let Some(index) = selected_stage else {
                return false;
            };
            if index + 1 >= stage_count {
                return false;
            }
            view.state.update(cx, |state, cx| {
                state.move_pipeline_stage(session_key, index, index + 1);
                state.set_status_message(Some(StatusMessage::info("Stage moved down")));
                cx.notify();
            });
            true
        }
        _ => false,
    }
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
            let modifiers = event.keystroke.modifiers;
            let cmd_or_ctrl = modifiers.secondary() || modifiers.control;
            let is_escape = key == "escape";
            let is_enter = key == "enter" || key == "return";

            if !is_escape && !is_enter && !cmd_or_ctrl {
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
                let aggregation_context =
                    this.view_model.current_session().and_then(|session_key| {
                        let subview = this.state.read(cx).session_subview(&session_key);
                        (subview == Some(CollectionSubview::Aggregation)).then_some(session_key)
                    });
                let body_focused =
                    this.aggregation_stage_body_state.as_ref().is_some_and(|body_state| {
                        body_state.read(cx).focus_handle(cx).is_focused(window)
                    });

                if let Some(session_key) = aggregation_context.clone()
                    && cmd_or_ctrl
                    && handle_aggregation_shortcut(
                        this,
                        &session_key,
                        key.as_str(),
                        modifiers,
                        body_focused,
                        window,
                        cx,
                    )
                {
                    cx.stop_propagation();
                    return;
                }
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
                    if !handled {
                        let is_aggregation = this
                            .view_model
                            .current_session()
                            .and_then(|session_key| {
                                this.state.read(cx).session_subview(&session_key)
                            })
                            .is_some_and(|subview| subview == CollectionSubview::Aggregation);
                        if is_aggregation
                            && let Some(body_state) = this.aggregation_stage_body_state.clone()
                        {
                            let focused = body_state.read(cx).focus_handle(cx).is_focused(window);
                            if focused {
                                window.focus(&this.aggregation_focus);
                                handled = true;
                            }
                        }
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
                let should_load =
                    state_ref.session_data(&session_key).map(|data| !data.loaded).unwrap_or(true);
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
                let start = Instant::now();
                let next_session = state.read(cx).current_session_key();
                if this.input_session != next_session {
                    this.persist_query_input_drafts(cx);
                }

                let state_ref = state.read(cx);
                let next_session = state_ref.current_session_key();
                let should_load = next_session
                    .as_ref()
                    .map(|session| {
                        state_ref.session_data(session).map(|data| !data.loaded).unwrap_or(true)
                    })
                    .unwrap_or(false);
                let session_changed =
                    this.view_model.set_current_session(next_session.clone(), &state, cx);
                if session_changed || should_load {
                    if let Some(session) = next_session.clone() {
                        if should_load {
                            AppCommands::load_documents_for_session(state.clone(), session, cx);
                        } else {
                            this.view_model.rebuild_tree(&state, cx);
                        }
                    }
                    if session_changed {
                        this.update_search_results(cx);
                    }
                    cx.notify();
                }
                log_tabs_duration("documents.view_changed", start, || {
                    let target = next_session
                        .as_ref()
                        .map(|s| format!("{}/{}", s.database, s.collection))
                        .unwrap_or_else(|| "-".to_string());
                    format!(
                        "session_changed={session_changed} should_load={should_load} target={target}"
                    )
                });
                // Ensure subview-specific data is loaded (indexes/stats)
                if let Some(session_key) = next_session {
                    this.ensure_subview_data_loaded(&session_key, &state, cx);
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
            aggregation_focus: cx.focus_handle(),
            aggregation_stage_list_scroll: UniformListScrollHandle::default(),
            filter_state: None,
            sort_state: None,
            projection_state: None,
            schema_filter_state: None,
            filter_auto_pair: AutoPairState::new("{}"),
            sort_auto_pair: AutoPairState::new("{}"),
            projection_auto_pair: AutoPairState::new("{}"),
            filter_error: false,
            sort_error: false,
            projection_error: false,
            search_state: None,
            search_visible: false,
            search_matches: Vec::new(),
            search_index: None,
            search_case_sensitive: false,
            search_whole_word: false,
            search_regex: false,
            search_values_only: false,
            input_session: None,
            schema_filter_session: None,
            aggregation_input_session: None,
            aggregation_selected_stage: None,
            aggregation_stage_count: 0,
            aggregation_drag_over: None,
            aggregation_drag_source: None,
            filter_subscription: None,
            sort_subscription: None,
            projection_subscription: None,
            schema_filter_subscription: None,
            search_subscription: None,
            aggregation_stage_body_state: None,
            aggregation_results_tree_state: None,
            aggregation_results_scroll: UniformListScrollHandle::new(),
            aggregation_limit_state: None,
            aggregation_results_expanded_nodes: HashSet::new(),
            aggregation_results_signature: None,
            aggregation_ignore_body_change: false,
            aggregation_stage_body_subscription: None,
            aggregation_limit_subscription: None,
            _subscriptions: subscriptions,
        }
    }

    /// Ensure subview-specific data is loaded (indexes/stats) based on current subview.
    /// This replaces the logic that was previously in render().
    fn ensure_subview_data_loaded(
        &self,
        session_key: &SessionKey,
        state: &Entity<AppState>,
        cx: &mut App,
    ) {
        let state_ref = state.read(cx);
        let Some(snapshot) = state_ref.session_snapshot(session_key) else {
            return;
        };

        let subview = snapshot.subview;
        let indexes = snapshot.indexes;
        let indexes_loading = snapshot.indexes_loading;
        let indexes_error = snapshot.indexes_error;
        let stats = snapshot.stats;
        let stats_loading = snapshot.stats_loading;
        let stats_error = snapshot.stats_error;

        if subview == CollectionSubview::Indexes
            && indexes.is_none()
            && !indexes_loading
            && indexes_error.is_none()
        {
            AppCommands::load_collection_indexes(state.clone(), session_key.clone(), false, cx);
        }

        if subview == CollectionSubview::Stats
            && stats.is_none()
            && !stats_loading
            && stats_error.is_none()
        {
            AppCommands::load_collection_stats(state.clone(), session_key.clone(), cx);
        }

        let schema = snapshot.schema;
        let schema_loading = snapshot.schema_loading;
        let schema_error = snapshot.schema_error;
        if subview == CollectionSubview::Schema
            && schema.is_none()
            && !schema_loading
            && schema_error.is_none()
        {
            AppCommands::analyze_collection_schema(state.clone(), session_key.clone(), cx);
        }
    }

    fn persist_query_input_drafts(&mut self, cx: &mut Context<Self>) {
        let Some(session_key) = self.input_session.clone() else {
            return;
        };

        let filter_raw = self.filter_state.as_ref().map(|input| input.read(cx).value().to_string());
        let sort_raw = self.sort_state.as_ref().map(|input| input.read(cx).value().to_string());
        let projection_raw =
            self.projection_state.as_ref().map(|input| input.read(cx).value().to_string());

        let (stored_filter, stored_sort, stored_projection) = {
            let state_ref = self.state.read(cx);
            let Some(session_data) = state_ref.session_data(&session_key) else {
                return;
            };
            (
                session_data.filter_raw.clone(),
                session_data.sort_raw.clone(),
                session_data.projection_raw.clone(),
            )
        };

        let next_filter = filter_raw.unwrap_or_else(|| stored_filter.clone());
        let next_sort = sort_raw.unwrap_or_else(|| stored_sort.clone());
        let next_projection = projection_raw.unwrap_or_else(|| stored_projection.clone());

        let filter_changed = next_filter != stored_filter;
        let sort_projection_changed =
            next_sort != stored_sort || next_projection != stored_projection;
        if !filter_changed && !sort_projection_changed {
            return;
        }

        self.state.update(cx, |state, _| {
            if filter_changed {
                state.save_filter_draft(&session_key, next_filter.clone());
            }
            if sort_projection_changed {
                state.save_sort_projection_draft(
                    &session_key,
                    next_sort.clone(),
                    next_projection.clone(),
                );
            }
        });
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

    pub(crate) fn current_search_query(&self, cx: &mut Context<Self>) -> Option<String> {
        let raw = self
            .search_state
            .as_ref()
            .map(|state| state.read(cx).value().to_string())
            .unwrap_or_default();
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() { None } else { Some(trimmed) }
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
        window.focus(&self.documents_focus);
        if let Some(search_state) = self.search_state.clone() {
            search_state.update(cx, |state, cx| {
                state.set_value(String::new(), window, cx);
            });
        }
        self.search_matches.clear();
        self.search_index = None;
        self.search_case_sensitive = false;
        self.search_whole_word = false;
        self.search_regex = false;
        self.search_values_only = false;
    }

    pub(crate) fn toggle_search_case_sensitive(&mut self, cx: &mut Context<Self>) {
        self.search_case_sensitive = !self.search_case_sensitive;
        self.update_search_results(cx);
    }

    pub(crate) fn toggle_search_whole_word(&mut self, cx: &mut Context<Self>) {
        self.search_whole_word = !self.search_whole_word;
        self.update_search_results(cx);
    }

    pub(crate) fn toggle_search_regex(&mut self, cx: &mut Context<Self>) {
        self.search_regex = !self.search_regex;
        self.update_search_results(cx);
    }

    pub(crate) fn toggle_search_values_only(&mut self, cx: &mut Context<Self>) {
        self.search_values_only = !self.search_values_only;
        self.update_search_results(cx);
    }

    pub(crate) fn update_search_results(&mut self, cx: &mut Context<Self>) {
        let Some(query) = self.current_search_query(cx) else {
            self.search_matches.clear();
            self.search_index = None;
            return;
        };

        let case_sensitive = self.search_case_sensitive;
        let whole_word = self.search_whole_word;
        let use_regex = self.search_regex;
        let values_only = self.search_values_only;

        let node_meta = self.view_model.node_meta();
        let mut matches = Vec::new();
        for node_id in self.view_model.tree_order_all() {
            let Some(meta) = node_meta.get(node_id) else {
                continue;
            };
            if meta.path.is_empty() {
                continue;
            }
            let key_matches = !values_only
                && matches_query(&query, &meta.key_label, case_sensitive, whole_word, use_regex);
            let value_matches =
                matches_query(&query, &meta.value_label, case_sensitive, whole_word, use_regex);
            if key_matches || value_matches {
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

        // Update app state selected node — arrow keys clear multi-selection
        if let Some(node_id) = tree_order.get(new_ix) {
            let node_meta = this.view_model.node_meta();
            if let Some(meta) = node_meta.get(node_id) {
                this.state.update(cx, |state, cx| {
                    state.select_single_doc(session_key, meta.doc_key.clone(), node_id.clone());
                    cx.notify();
                });
            }
        }
        cx.notify();
    }

    pub(crate) fn handle_tree_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if self.view_model.inline_state().is_some() {
            return false;
        }
        let Some(session_key) = self.view_model.current_session() else {
            return false;
        };

        let count = self.view_model.tree_order().len();
        if count == 0 {
            return false;
        }

        let current_ix = self.view_model.tree_state().read(cx).selected_index().unwrap_or(0);
        let current_node_id = self.view_model.tree_order().get(current_ix).cloned();
        let key = event.keystroke.key.to_ascii_lowercase();

        match key.as_str() {
            "up" => {
                let new_ix = if current_ix == 0 { count - 1 } else { current_ix - 1 };
                Self::select_tree_index(self, new_ix, &session_key, cx);
                true
            }
            "down" => {
                let new_ix = if current_ix >= count - 1 { 0 } else { current_ix + 1 };
                Self::select_tree_index(self, new_ix, &session_key, cx);
                true
            }
            "left" => {
                if let Some(node_id) = current_node_id {
                    let node_meta = self.view_model.node_meta();
                    let is_folder = node_meta.get(&node_id).is_some_and(|m| m.is_folder);
                    let is_expanded = self
                        .state
                        .read(cx)
                        .session_view(&session_key)
                        .is_some_and(|v| v.expanded_nodes.contains(&node_id));
                    if is_folder && is_expanded {
                        self.state.update(cx, |state, cx| {
                            state.toggle_expanded_node(&session_key, &node_id);
                            cx.notify();
                        });
                        self.view_model.rebuild_tree(&self.state, cx);
                        cx.notify();
                    } else {
                        let parent_id = node_id.rfind('/').map(|i| &node_id[..i]);
                        if let Some(parent_id) = parent_id {
                            let parent_ix =
                                self.view_model.tree_order().iter().position(|id| id == parent_id);
                            if let Some(parent_ix) = parent_ix {
                                Self::select_tree_index(self, parent_ix, &session_key, cx);
                            }
                        }
                    }
                }
                true
            }
            "right" => {
                if let Some(node_id) = current_node_id {
                    let node_meta = self.view_model.node_meta();
                    let is_folder = node_meta.get(&node_id).is_some_and(|m| m.is_folder);
                    let is_expanded = self
                        .state
                        .read(cx)
                        .session_view(&session_key)
                        .is_some_and(|v| v.expanded_nodes.contains(&node_id));
                    if is_folder && !is_expanded {
                        self.state.update(cx, |state, cx| {
                            state.toggle_expanded_node(&session_key, &node_id);
                            cx.notify();
                        });
                        self.view_model.rebuild_tree(&self.state, cx);
                        cx.notify();
                    } else if is_folder && is_expanded && current_ix + 1 < count {
                        Self::select_tree_index(self, current_ix + 1, &session_key, cx);
                    }
                }
                true
            }
            _ => false,
        }
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

pub(crate) fn matches_query(
    query: &str,
    text: &str,
    case_sensitive: bool,
    whole_word: bool,
    use_regex: bool,
) -> bool {
    if query.is_empty() {
        return false;
    }

    if use_regex {
        let Ok(re) = RegexBuilder::new(query).case_insensitive(!case_sensitive).build() else {
            return false;
        };
        if whole_word {
            // Check that any regex match is a whole word
            for m in re.find_iter(text) {
                let start_ok =
                    m.start() == 0 || !text[..m.start()].ends_with(char::is_alphanumeric);
                let end_ok =
                    m.end() == text.len() || !text[m.end()..].starts_with(char::is_alphanumeric);
                if start_ok && end_ok {
                    return true;
                }
            }
            false
        } else {
            re.is_match(text)
        }
    } else if whole_word {
        let words: Vec<&str> = text.split(|c: char| !c.is_alphanumeric() && c != '_').collect();
        if case_sensitive {
            words.contains(&query)
        } else {
            let q = query.to_lowercase();
            words.iter().any(|w| w.to_lowercase() == q)
        }
    } else if case_sensitive {
        text.contains(query)
    } else {
        text.to_lowercase().contains(&query.to_lowercase())
    }
}
