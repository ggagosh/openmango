//! Aggregation pipeline management for sessions.

use std::sync::atomic::Ordering;

use crate::state::AppState;
use crate::state::app_state::types::SessionKey;
use crate::state::app_state::{
    PipelineStage, PipelineState, StageDocCounts, StageStatsMode, default_stage_body,
};

impl AppState {
    fn invalidate_aggregation_run(aggregation: &mut PipelineState) {
        if let Ok(mut handle) = aggregation.abort_handle.lock()
            && let Some(handle) = handle.take()
        {
            handle.abort();
        }
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
            if let Some(template) = default_stage_body(stage.operator.trim()) {
                stage.body = template.to_string();
            }
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

    pub fn set_pipeline_stage_stats_mode(
        &mut self,
        session_key: &SessionKey,
        mode: StageStatsMode,
    ) {
        let mut changed = false;
        if let Some(session) = self.session_mut(session_key)
            && session.data.aggregation.stage_stats_mode != mode
        {
            session.data.aggregation.stage_stats_mode = mode;
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
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use mongodb::bson::doc;

    use crate::state::app_state::StageStatsMode;
    use crate::state::{AppState, SessionKey};

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

        state.set_pipeline_stage_stats_mode(&session_key, StageStatsMode::Off);

        let session = state.session(&session_key).expect("session exists");
        let counts = &session.data.aggregation.stage_doc_counts[0];
        assert_eq!(counts.input, None);
        assert_eq!(counts.output, None);
        assert_eq!(counts.time_ms, None);
        assert!(session.data.aggregation.results.is_some());
        assert_eq!(session.data.aggregation.last_run_time_ms, None);
        assert_eq!(session.data.aggregation.error, None);
        assert_eq!(session.data.aggregation.stage_stats_mode, StageStatsMode::Off);
        assert_eq!(session.data.aggregation.request_id, prev_request_id + 1);
        assert_eq!(
            session.data.aggregation.run_generation.load(Ordering::SeqCst),
            prev_generation + 1
        );
    }
}
