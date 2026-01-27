//! Session management for per-tab collection state.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::atomic::Ordering;

use mongodb::bson::{Bson, Document};
use uuid::Uuid;

use crate::bson::{DocumentKey, PathSegment, path_to_id, set_bson_at_path};
use crate::state::AppState;
use crate::state::app_state::types::{
    CollectionSubview, SessionData, SessionKey, SessionSnapshot, SessionState, SessionViewState,
};
use crate::state::app_state::{PipelineStage, PipelineState, StageDocCounts};

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
            aggregation: session.data.aggregation.clone(),
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
        self.update_workspace_session_filters(session_key);
    }

    pub fn clear_filter(&mut self, session_key: &SessionKey) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.filter_raw.clear();
            session.data.filter = None;
            session.data.page = 0;
        }
        self.update_workspace_session_filters(session_key);
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
        self.update_workspace_session_filters(session_key);
    }

    fn invalidate_aggregation_run(aggregation: &mut PipelineState) {
        aggregation.request_id += 1;
        aggregation.run_generation.fetch_add(1, Ordering::SeqCst);
        aggregation.loading = false;
    }

    fn reset_aggregation_results(aggregation: &mut PipelineState) {
        aggregation.results = None;
        aggregation.results_page = 0;
        aggregation.last_run_time_ms = None;
        aggregation.error = None;
        Self::invalidate_aggregation_run(aggregation);
    }

    fn reset_aggregation_stage_stats_state(aggregation: &mut PipelineState) {
        aggregation.analysis = None;
        aggregation.stage_doc_counts = vec![StageDocCounts::default(); aggregation.stages.len()];
    }

    fn reset_aggregation_stage_stats_only(aggregation: &mut PipelineState) {
        Self::reset_aggregation_stage_stats_state(aggregation);
        aggregation.last_run_time_ms = None;
        aggregation.error = None;
        Self::invalidate_aggregation_run(aggregation);
    }

    fn reset_aggregation_after_edit(aggregation: &mut PipelineState) {
        Self::reset_aggregation_stage_stats_state(aggregation);
        Self::reset_aggregation_results(aggregation);
    }

    pub fn add_pipeline_stage(
        &mut self,
        session_key: &SessionKey,
        operator: impl Into<String>,
    ) -> Option<usize> {
        let mut new_index = None;
        if let Some(session) = self.session_mut(session_key) {
            session.data.aggregation.stages.push(PipelineStage::new(operator));
            let len = session.data.aggregation.stages.len();
            session.data.aggregation.selected_stage = len.checked_sub(1);
            Self::reset_aggregation_after_edit(&mut session.data.aggregation);
            new_index = session.data.aggregation.selected_stage;
        }
        self.update_workspace_session_view(session_key);
        new_index
    }

    pub fn replace_pipeline_stages(
        &mut self,
        session_key: &SessionKey,
        stages: Vec<PipelineStage>,
    ) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.aggregation.stages = stages;
            let len = session.data.aggregation.stages.len();
            session.data.aggregation.selected_stage = len.checked_sub(1);
            session.data.aggregation.loading = false;
            Self::reset_aggregation_after_edit(&mut session.data.aggregation);
        }
        self.update_workspace_session_view(session_key);
    }

    pub fn insert_pipeline_stage(
        &mut self,
        session_key: &SessionKey,
        index: usize,
        operator: impl Into<String>,
    ) -> Option<usize> {
        let mut new_index = None;
        if let Some(session) = self.session_mut(session_key) {
            let len = session.data.aggregation.stages.len();
            let insert_index = index.min(len);
            session.data.aggregation.stages.insert(insert_index, PipelineStage::new(operator));
            session.data.aggregation.selected_stage = Some(insert_index);
            Self::reset_aggregation_after_edit(&mut session.data.aggregation);
            new_index = session.data.aggregation.selected_stage;
        }
        self.update_workspace_session_view(session_key);
        new_index
    }

    pub fn remove_pipeline_stage(&mut self, session_key: &SessionKey, index: usize) {
        if let Some(session) = self.session_mut(session_key) {
            if index >= session.data.aggregation.stages.len() {
                return;
            }
            session.data.aggregation.stages.remove(index);
            let len = session.data.aggregation.stages.len();
            let selected = session.data.aggregation.selected_stage;
            session.data.aggregation.selected_stage = match selected {
                Some(sel) if sel == index => {
                    if len == 0 {
                        None
                    } else {
                        Some(sel.min(len.saturating_sub(1)))
                    }
                }
                Some(sel) if sel > index => Some(sel - 1),
                Some(sel) if sel < len => Some(sel),
                _ => None,
            };
            Self::reset_aggregation_after_edit(&mut session.data.aggregation);
        }
        self.update_workspace_session_view(session_key);
    }

    pub fn set_pipeline_selected_stage(
        &mut self,
        session_key: &SessionKey,
        selected: Option<usize>,
    ) {
        if let Some(session) = self.session_mut(session_key) {
            let max_index = session.data.aggregation.stages.len().saturating_sub(1);
            session.data.aggregation.selected_stage = selected.filter(|idx| *idx <= max_index);
            session.data.aggregation.results_page = 0;
            Self::invalidate_aggregation_run(&mut session.data.aggregation);
        }
        self.update_workspace_session_view(session_key);
    }

    pub fn set_pipeline_stage_body(
        &mut self,
        session_key: &SessionKey,
        index: usize,
        body: String,
    ) {
        if let Some(session) = self.session_mut(session_key)
            && let Some(stage) = session.data.aggregation.stages.get_mut(index)
        {
            stage.body = body;
            Self::reset_aggregation_after_edit(&mut session.data.aggregation);
        }
        self.update_workspace_session_view(session_key);
    }

    pub fn set_pipeline_stage_operator(
        &mut self,
        session_key: &SessionKey,
        index: usize,
        operator: String,
    ) {
        if let Some(session) = self.session_mut(session_key)
            && let Some(stage) = session.data.aggregation.stages.get_mut(index)
        {
            stage.operator = operator;
            Self::reset_aggregation_after_edit(&mut session.data.aggregation);
        }
        self.update_workspace_session_view(session_key);
    }

    pub fn toggle_pipeline_stage_enabled(&mut self, session_key: &SessionKey, index: usize) {
        if let Some(session) = self.session_mut(session_key)
            && let Some(stage) = session.data.aggregation.stages.get_mut(index)
        {
            stage.enabled = !stage.enabled;
            Self::reset_aggregation_after_edit(&mut session.data.aggregation);
        }
        self.update_workspace_session_view(session_key);
    }

    pub fn set_pipeline_result_limit(&mut self, session_key: &SessionKey, limit: i64) {
        if let Some(session) = self.session_mut(session_key) {
            let normalized_limit = limit.max(1);
            session.data.aggregation.result_limit = normalized_limit;
            Self::reset_aggregation_results(&mut session.data.aggregation);
        }
        self.update_workspace_session_view(session_key);
    }

    pub fn set_pipeline_stage_stats_enabled(&mut self, session_key: &SessionKey, enabled: bool) {
        let mut changed = false;
        if let Some(session) = self.session_mut(session_key)
            && session.data.aggregation.stage_stats_enabled != enabled
        {
            session.data.aggregation.stage_stats_enabled = enabled;
            Self::reset_aggregation_stage_stats_only(&mut session.data.aggregation);
            changed = true;
        }
        if changed {
            self.update_workspace_session_view(session_key);
        }
    }

    pub fn duplicate_pipeline_stage(
        &mut self,
        session_key: &SessionKey,
        index: usize,
    ) -> Option<usize> {
        let mut new_index = None;
        if let Some(session) = self.session_mut(session_key) {
            let stage = session.data.aggregation.stages.get(index).cloned()?;
            let insert_index = index + 1;
            session.data.aggregation.stages.insert(insert_index, stage);
            session.data.aggregation.selected_stage = Some(insert_index);
            Self::reset_aggregation_after_edit(&mut session.data.aggregation);
            new_index = session.data.aggregation.selected_stage;
        }
        self.update_workspace_session_view(session_key);
        new_index
    }

    pub fn move_pipeline_stage(&mut self, session_key: &SessionKey, from: usize, to: usize) {
        if let Some(session) = self.session_mut(session_key) {
            let len = session.data.aggregation.stages.len();
            if from >= len || to >= len || from == to {
                return;
            }
            let stage = session.data.aggregation.stages.remove(from);
            session.data.aggregation.stages.insert(to, stage);

            let selected = session.data.aggregation.selected_stage;
            session.data.aggregation.selected_stage = match selected {
                Some(sel) if sel == from => Some(to),
                Some(sel) if from < to && sel > from && sel <= to => Some(sel - 1),
                Some(sel) if from > to && sel >= to && sel < from => Some(sel + 1),
                other => other,
            };

            Self::reset_aggregation_after_edit(&mut session.data.aggregation);
        }
        self.update_workspace_session_view(session_key);
    }

    pub fn prev_pipeline_page(&mut self, session_key: &SessionKey) -> bool {
        if let Some(session) = self.session_mut(session_key)
            && session.data.aggregation.results_page > 0
        {
            session.data.aggregation.results_page -= 1;
            Self::invalidate_aggregation_run(&mut session.data.aggregation);
            self.update_workspace_session_view(session_key);
            return true;
        }
        false
    }

    pub fn next_pipeline_page(&mut self, session_key: &SessionKey, total_pages: u64) -> bool {
        if total_pages == 0 {
            return false;
        }
        if let Some(session) = self.session_mut(session_key) {
            let next = session.data.aggregation.results_page + 1;
            if next < total_pages {
                session.data.aggregation.results_page = next;
                Self::invalidate_aggregation_run(&mut session.data.aggregation);
                self.update_workspace_session_view(session_key);
                return true;
            }
        }
        false
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
            self.update_workspace_session_view(session_key);
        }

        should_load
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

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

    #[test]
    fn stage_edit_resets_counts_results_and_cancels_run() {
        let mut state = AppState::new();
        let session_key = SessionKey::new(uuid::Uuid::new_v4(), "db", "col");
        state.ensure_session(session_key.clone());
        state.add_pipeline_stage(&session_key, "$match");

        let (prev_request_id, prev_generation) = {
            let session = state.session_mut(&session_key).expect("session exists");
            session.data.aggregation.stage_doc_counts[0].input = Some(10);
            session.data.aggregation.stage_doc_counts[0].output = Some(5);
            session.data.aggregation.stage_doc_counts[0].time_ms = Some(2);
            session.data.aggregation.results = Some(vec![doc! { "_id": 1 }]);
            session.data.aggregation.results_page = 3;
            session.data.aggregation.last_run_time_ms = Some(42);
            session.data.aggregation.error = Some("boom".to_string());
            (
                session.data.aggregation.request_id,
                session.data.aggregation.run_generation.load(Ordering::SeqCst),
            )
        };

        state.set_pipeline_stage_body(&session_key, 0, r#"{ "a": 1 }"#.to_string());

        let session = state.session(&session_key).expect("session exists");
        let counts = &session.data.aggregation.stage_doc_counts[0];
        assert_eq!(counts.input, None);
        assert_eq!(counts.output, None);
        assert_eq!(counts.time_ms, None);
        assert!(session.data.aggregation.results.is_none());
        assert_eq!(session.data.aggregation.results_page, 0);
        assert_eq!(session.data.aggregation.last_run_time_ms, None);
        assert_eq!(session.data.aggregation.error, None);
        assert!(!session.data.aggregation.loading);
        assert_eq!(session.data.aggregation.request_id, prev_request_id + 1);
        assert_eq!(
            session.data.aggregation.run_generation.load(Ordering::SeqCst),
            prev_generation + 1
        );
    }

    #[test]
    fn limit_change_resets_results_but_not_counts() {
        let mut state = AppState::new();
        let session_key = SessionKey::new(uuid::Uuid::new_v4(), "db", "col");
        state.ensure_session(session_key.clone());
        state.add_pipeline_stage(&session_key, "$match");

        let (prev_request_id, prev_generation) = {
            let session = state.session_mut(&session_key).expect("session exists");
            session.data.aggregation.stage_doc_counts[0].output = Some(7);
            session.data.aggregation.results = Some(vec![doc! { "_id": 1 }]);
            session.data.aggregation.results_page = 2;
            session.data.aggregation.last_run_time_ms = Some(11);
            session.data.aggregation.error = Some("err".to_string());
            (
                session.data.aggregation.request_id,
                session.data.aggregation.run_generation.load(Ordering::SeqCst),
            )
        };

        state.set_pipeline_result_limit(&session_key, 25);

        let session = state.session(&session_key).expect("session exists");
        assert_eq!(session.data.aggregation.result_limit, 25);
        assert_eq!(session.data.aggregation.stage_doc_counts[0].output, Some(7));
        assert!(session.data.aggregation.results.is_none());
        assert_eq!(session.data.aggregation.results_page, 0);
        assert_eq!(session.data.aggregation.last_run_time_ms, None);
        assert_eq!(session.data.aggregation.error, None);
        assert_eq!(session.data.aggregation.request_id, prev_request_id + 1);
        assert_eq!(
            session.data.aggregation.run_generation.load(Ordering::SeqCst),
            prev_generation + 1
        );
    }

    #[test]
    fn stage_stats_toggle_resets_counts_but_keeps_results() {
        let mut state = AppState::new();
        let session_key = SessionKey::new(uuid::Uuid::new_v4(), "db", "col");
        state.ensure_session(session_key.clone());
        state.add_pipeline_stage(&session_key, "$match");

        let (prev_request_id, prev_generation) = {
            let session = state.session_mut(&session_key).expect("session exists");
            session.data.aggregation.stage_doc_counts[0].output = Some(9);
            session.data.aggregation.results = Some(vec![doc! { "_id": 1 }]);
            session.data.aggregation.last_run_time_ms = Some(33);
            session.data.aggregation.error = Some("old".to_string());
            (
                session.data.aggregation.request_id,
                session.data.aggregation.run_generation.load(Ordering::SeqCst),
            )
        };

        state.set_pipeline_stage_stats_enabled(&session_key, false);

        let session = state.session(&session_key).expect("session exists");
        let counts = &session.data.aggregation.stage_doc_counts[0];
        assert_eq!(counts.input, None);
        assert_eq!(counts.output, None);
        assert_eq!(counts.time_ms, None);
        assert!(session.data.aggregation.results.is_some());
        assert_eq!(session.data.aggregation.last_run_time_ms, None);
        assert_eq!(session.data.aggregation.error, None);
        assert!(!session.data.aggregation.stage_stats_enabled);
        assert_eq!(session.data.aggregation.request_id, prev_request_id + 1);
        assert_eq!(
            session.data.aggregation.run_generation.load(Ordering::SeqCst),
            prev_generation + 1
        );
    }
}
