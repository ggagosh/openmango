//! Aggregation pipeline management for sessions.
//!
//! Edits never clear results: they bump `edit_revision`, which marks the
//! previous output as outdated until the next run.

use std::sync::{Arc, atomic::Ordering};

use crate::state::AppState;
use crate::state::app_state::types::SessionKey;
use crate::state::app_state::{
    PIPELINE_UNDO_LIMIT, PipelineSnapshot, PipelineStage, PipelineState, StageDocCounts,
    StageStatsMode, UndoGroup, default_stage_body,
};

#[derive(Clone, Copy)]
enum Undo {
    Skip,
    Step,
    /// Merge with the previous edit when it had the same group, like typing in one field.
    Group(UndoGroup),
}

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

    /// Apply a pipeline edit. `edit` returns `None` when nothing changed.
    fn edit_pipeline<R>(
        &mut self,
        session_key: &SessionKey,
        undo: Undo,
        edit: impl FnOnce(&mut PipelineState) -> Option<R>,
    ) -> Option<R> {
        let session = self.session_mut(session_key)?;
        let aggregation = &mut session.data.aggregation;
        let push = match undo {
            Undo::Skip => false,
            Undo::Step => true,
            Undo::Group(group) => aggregation.undo_group != Some(group),
        };
        let before = push.then(|| snapshot(aggregation));
        let result = edit(aggregation)?;
        if let Some(before) = before {
            push_undo(aggregation, before);
            aggregation.redo_stack = Arc::default();
        }
        aggregation.undo_group = match undo {
            Undo::Group(group) => Some(group),
            Undo::Skip | Undo::Step => None,
        };
        aggregation.edit_revision += 1;
        aggregation.results_page = 0;
        Self::invalidate_aggregation_run(aggregation);
        session.data.explain.mark_stale();
        self.update_workspace_session_view(session_key);
        Some(result)
    }

    pub fn add_pipeline_stage(
        &mut self,
        session_key: &SessionKey,
        operator: impl Into<String>,
    ) -> Option<usize> {
        let index = self.session(session_key)?.data.aggregation.stages.len();
        self.insert_pipeline_stage(session_key, index, operator)
    }

    pub fn insert_pipeline_stage(
        &mut self,
        session_key: &SessionKey,
        index: usize,
        operator: impl Into<String>,
    ) -> Option<usize> {
        let operator = operator.into();
        self.edit_pipeline(session_key, Undo::Step, |aggregation| {
            let index = index.min(aggregation.stages.len());
            aggregation.stages.insert(index, PipelineStage::new(operator));
            insert_counts(aggregation, index);
            aggregation.selected_stage = Some(index);
            aggregation.error_stage = None;
            Some(index)
        })
    }

    /// Insert several stages at once, as one step to undo: a generated join is a `$lookup` and
    /// its `$unwind`, and undoing half of it leaves a pipeline nobody asked for.
    pub fn insert_pipeline_stages(
        &mut self,
        session_key: &SessionKey,
        index: usize,
        stages: Vec<PipelineStage>,
    ) -> Option<usize> {
        if stages.is_empty() {
            return None;
        }
        self.edit_pipeline(session_key, Undo::Step, |aggregation| {
            let index = index.min(aggregation.stages.len());
            for (offset, stage) in stages.into_iter().enumerate() {
                aggregation.stages.insert(index + offset, stage);
                insert_counts(aggregation, index + offset);
            }
            // The `$lookup` is what gets edited next, not the `$unwind` after it.
            aggregation.selected_stage = Some(index);
            aggregation.error_stage = None;
            Some(index)
        })
    }

    /// Replace the pipeline from the library or an import. Undoable.
    pub fn replace_pipeline_stages(
        &mut self,
        session_key: &SessionKey,
        stages: Vec<PipelineStage>,
    ) {
        self.edit_pipeline(session_key, Undo::Step, |aggregation| {
            aggregation.selected_stage = stages.len().checked_sub(1);
            aggregation.stage_doc_counts = vec![StageDocCounts::default(); stages.len()];
            aggregation.stages = stages;
            aggregation.error_stage = None;
            Some(())
        });
    }

    /// Replace the pipeline while typing in Text mode; one undo step per typing session.
    pub fn replace_pipeline_stages_from_text(
        &mut self,
        session_key: &SessionKey,
        stages: Vec<PipelineStage>,
    ) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.aggregation.text_draft = None;
        }
        self.edit_pipeline(session_key, Undo::Group(UndoGroup::Text), |aggregation| {
            if aggregation.stages == stages {
                return None;
            }
            let same_shape =
                aggregation.stages.len() == stages.len()
                    && aggregation.stages.iter().zip(&stages).all(|(old, new)| {
                        old.operator == new.operator && old.enabled == new.enabled
                    });
            if !same_shape {
                aggregation.stage_doc_counts = vec![StageDocCounts::default(); stages.len()];
                aggregation.error_stage = None;
            }
            aggregation.stages = stages;
            Some(())
        });
    }

    /// Keep Text mode input that doesn't parse, so switching tabs doesn't lose it.
    pub fn set_pipeline_text_draft(&mut self, session_key: &SessionKey, draft: Option<String>) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.aggregation.text_draft = draft;
        }
    }

    pub fn remove_pipeline_stage(&mut self, session_key: &SessionKey, index: usize) {
        self.edit_pipeline(session_key, Undo::Step, |aggregation| {
            if index >= aggregation.stages.len() {
                return None;
            }
            aggregation.stages.remove(index);
            if index < aggregation.stage_doc_counts.len() {
                aggregation.stage_doc_counts.remove(index);
            }
            let len = aggregation.stages.len();
            aggregation.selected_stage = match aggregation.selected_stage {
                Some(_) if len == 0 => None,
                Some(sel) if sel > index => Some(sel - 1),
                Some(sel) => Some(sel.min(len - 1)),
                None => None,
            };
            aggregation.error_stage = None;
            Some(())
        });
    }

    pub fn set_pipeline_selected_stage(
        &mut self,
        session_key: &SessionKey,
        selected: Option<usize>,
    ) {
        if let Some(session) = self.session_mut(session_key) {
            let aggregation = &mut session.data.aggregation;
            let max_index = aggregation.stages.len().saturating_sub(1);
            let selected = selected.filter(|idx| *idx <= max_index);
            if aggregation.selected_stage == selected {
                return;
            }
            aggregation.selected_stage = selected;
            aggregation.results_page = 0;
            Self::invalidate_aggregation_run(aggregation);
            session.data.explain.mark_stale();
        }
        self.update_workspace_session_view(session_key);
    }

    pub fn set_pipeline_stage_body(
        &mut self,
        session_key: &SessionKey,
        index: usize,
        body: String,
    ) {
        self.edit_pipeline(session_key, Undo::Group(UndoGroup::StageBody(index)), |aggregation| {
            let stage = aggregation.stages.get_mut(index)?;
            if stage.body == body {
                return None;
            }
            stage.body = body;
            Some(())
        });
    }

    pub fn set_pipeline_stage_operator(
        &mut self,
        session_key: &SessionKey,
        index: usize,
        operator: String,
    ) {
        self.edit_pipeline(session_key, Undo::Step, |aggregation| {
            let stage = aggregation.stages.get_mut(index)?;
            if stage.operator == operator {
                return None;
            }
            if let Some(template) = default_stage_body(operator.trim()) {
                stage.body = template.to_string();
            }
            stage.operator = operator;
            Some(())
        });
    }

    pub fn toggle_pipeline_stage_enabled(&mut self, session_key: &SessionKey, index: usize) {
        self.edit_pipeline(session_key, Undo::Step, |aggregation| {
            let stage = aggregation.stages.get_mut(index)?;
            stage.enabled = !stage.enabled;
            Some(())
        });
    }

    pub fn set_pipeline_result_limit(&mut self, session_key: &SessionKey, limit: i64) {
        if let Some(session) = self.session_mut(session_key) {
            let aggregation = &mut session.data.aggregation;
            aggregation.result_limit = limit.max(1);
            aggregation.results_page = 0;
            Self::invalidate_aggregation_run(aggregation);
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
            let aggregation = &mut session.data.aggregation;
            aggregation.stage_stats_mode = mode;
            aggregation.stage_doc_counts =
                vec![StageDocCounts::default(); aggregation.stages.len()];
            Self::invalidate_aggregation_run(aggregation);
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
        self.edit_pipeline(session_key, Undo::Step, |aggregation| {
            let stage = aggregation.stages.get(index)?.duplicate();
            aggregation.stages.insert(index + 1, stage);
            insert_counts(aggregation, index + 1);
            aggregation.selected_stage = Some(index + 1);
            aggregation.error_stage = None;
            Some(index + 1)
        })
    }

    pub fn move_pipeline_stage(&mut self, session_key: &SessionKey, from: usize, to: usize) {
        self.edit_pipeline(session_key, Undo::Step, |aggregation| {
            let len = aggregation.stages.len();
            if from >= len || to >= len || from == to {
                return None;
            }
            let stage = aggregation.stages.remove(from);
            aggregation.stages.insert(to, stage);
            if aggregation.stage_doc_counts.len() == len {
                let counts = aggregation.stage_doc_counts.remove(from);
                aggregation.stage_doc_counts.insert(to, counts);
            }
            aggregation.selected_stage = match aggregation.selected_stage {
                Some(sel) if sel == from => Some(to),
                Some(sel) if from < to && sel > from && sel <= to => Some(sel - 1),
                Some(sel) if from > to && sel >= to && sel < from => Some(sel + 1),
                other => other,
            };
            aggregation.error_stage = None;
            Some(())
        });
    }

    pub fn set_pipeline_preview_input(&mut self, session_key: &SessionKey, input: bool) {
        if let Some(session) = self.session_mut(session_key)
            && session.data.aggregation.preview_input != input
        {
            let aggregation = &mut session.data.aggregation;
            aggregation.preview_input = input;
            aggregation.results_page = 0;
            Self::invalidate_aggregation_run(aggregation);
        }
    }

    pub fn set_pipeline_auto_run(&mut self, session_key: &SessionKey, auto_run: bool) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.aggregation.auto_run = auto_run;
        }
    }

    /// Text mode previews the whole pipeline, so it clears the stage selection.
    pub fn set_pipeline_text_mode(&mut self, session_key: &SessionKey, text_mode: bool) {
        if let Some(session) = self.session_mut(session_key)
            && session.data.aggregation.text_mode != text_mode
        {
            let aggregation = &mut session.data.aggregation;
            aggregation.text_mode = text_mode;
            aggregation.selected_stage =
                if text_mode { None } else { aggregation.stages.len().checked_sub(1) };
            aggregation.results_page = 0;
            Self::invalidate_aggregation_run(aggregation);
        }
        self.update_workspace_session_view(session_key);
    }

    /// Serial of the edit the next undo reverts, used to check an Undo button still applies.
    pub fn pipeline_undo_top(&self, session_key: &SessionKey) -> Option<u64> {
        let session = self.session(session_key)?;
        session.data.aggregation.undo_stack.last().map(|snapshot| snapshot.serial)
    }

    pub fn undo_pipeline_edit(&mut self, session_key: &SessionKey) -> bool {
        self.step_pipeline_history(session_key, true)
    }

    pub fn redo_pipeline_edit(&mut self, session_key: &SessionKey) -> bool {
        self.step_pipeline_history(session_key, false)
    }

    fn step_pipeline_history(&mut self, session_key: &SessionKey, undo: bool) -> bool {
        self.edit_pipeline(session_key, Undo::Skip, |aggregation| {
            let current = snapshot(aggregation);
            let restored = if undo {
                let restored = Arc::make_mut(&mut aggregation.undo_stack).pop()?;
                Arc::make_mut(&mut aggregation.redo_stack).push(current);
                restored
            } else {
                let restored = Arc::make_mut(&mut aggregation.redo_stack).pop()?;
                push_undo(aggregation, current);
                restored
            };
            // Counts can't be matched to restored stages reliably; the next run refills them.
            aggregation.stage_doc_counts = vec![StageDocCounts::default(); restored.stages.len()];
            aggregation.selected_stage = restored.selected_stage;
            aggregation.stages = restored.stages;
            aggregation.error_stage = None;
            Some(())
        })
        .is_some()
    }

    pub fn set_pipeline_page(&mut self, session_key: &SessionKey, page: u64) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.aggregation.results_page = page;
            Self::invalidate_aggregation_run(&mut session.data.aggregation);
        }
        self.update_workspace_session_view(session_key);
    }
}

fn snapshot(aggregation: &PipelineState) -> PipelineSnapshot {
    PipelineSnapshot {
        stages: aggregation.stages.clone(),
        selected_stage: aggregation.selected_stage,
        serial: 0,
    }
}

fn push_undo(aggregation: &mut PipelineState, mut snapshot: PipelineSnapshot) {
    aggregation.undo_serial += 1;
    snapshot.serial = aggregation.undo_serial;
    let undo = Arc::make_mut(&mut aggregation.undo_stack);
    undo.push(snapshot);
    if undo.len() > PIPELINE_UNDO_LIMIT {
        undo.remove(0);
    }
}

fn insert_counts(aggregation: &mut PipelineState, index: usize) {
    let counts = &mut aggregation.stage_doc_counts;
    counts.resize(aggregation.stages.len() - 1, StageDocCounts::default());
    counts.insert(index.min(counts.len()), StageDocCounts::default());
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::Ordering};

    use mongodb::bson::doc;

    use crate::state::app_state::{PipelineRun, StageStatsMode};
    use crate::state::{AppState, SessionKey};

    fn state_with(operators: &[&str]) -> (AppState, SessionKey) {
        let mut state = AppState::new();
        let key = SessionKey::new(uuid::Uuid::new_v4(), "db", "col");
        state.ensure_session(key.clone());
        for operator in operators {
            state.add_pipeline_stage(&key, *operator);
        }
        (state, key)
    }

    #[test]
    fn stage_edit_keeps_results_marks_them_outdated_and_cancels_run() {
        let (mut state, key) = state_with(&["$match"]);
        let (prev_request_id, prev_generation) = {
            let aggregation = &mut state.session_mut(&key).unwrap().data.aggregation;
            aggregation.stage_doc_counts[0].output = Some(5);
            aggregation.results = Some(Arc::new(vec![doc! { "_id": 1 }]));
            aggregation.results_page = 3;
            aggregation.last_run =
                Some(PipelineRun { target: Some(0), revision: aggregation.edit_revision, page: 0 });
            assert!(!aggregation.is_stale());
            (aggregation.request_id, aggregation.run_generation.load(Ordering::SeqCst))
        };

        state.set_pipeline_stage_body(&key, 0, r#"{ "a": 1 }"#.to_string());

        let aggregation = &state.session(&key).unwrap().data.aggregation;
        assert!(aggregation.results.is_some());
        assert_eq!(aggregation.stage_doc_counts[0].output, Some(5));
        assert!(aggregation.is_stale());
        assert_eq!(aggregation.results_page, 0);
        assert!(!aggregation.loading);
        assert_eq!(aggregation.request_id, prev_request_id + 1);
        assert_eq!(aggregation.run_generation.load(Ordering::SeqCst), prev_generation + 1);
    }

    #[test]
    fn selecting_or_switching_to_input_marks_results_outdated() {
        let (mut state, key) = state_with(&["$match", "$group"]);
        {
            let aggregation = &mut state.session_mut(&key).unwrap().data.aggregation;
            aggregation.last_run =
                Some(PipelineRun { target: Some(1), revision: aggregation.edit_revision, page: 0 });
            assert!(!aggregation.is_stale());
        }

        state.set_pipeline_selected_stage(&key, Some(0));
        assert!(state.session(&key).unwrap().data.aggregation.is_stale());

        state.set_pipeline_selected_stage(&key, Some(1));
        state.set_pipeline_preview_input(&key, true);
        let aggregation = &state.session(&key).unwrap().data.aggregation;
        assert_eq!(aggregation.preview_target(), Some(0));
        assert!(aggregation.is_stale());
    }

    #[test]
    fn structural_edits_keep_counts_aligned_with_stages() {
        let (mut state, key) = state_with(&["$match", "$group", "$sort"]);
        {
            let aggregation = &mut state.session_mut(&key).unwrap().data.aggregation;
            for (ix, counts) in aggregation.stage_doc_counts.iter_mut().enumerate() {
                counts.output = Some(ix as u64);
            }
        }

        state.move_pipeline_stage(&key, 2, 0);
        state.remove_pipeline_stage(&key, 1);
        state.insert_pipeline_stage(&key, 1, "$limit");

        let aggregation = &state.session(&key).unwrap().data.aggregation;
        let operators: Vec<_> = aggregation.stages.iter().map(|s| s.operator.as_str()).collect();
        let outputs: Vec<_> = aggregation.stage_doc_counts.iter().map(|c| c.output).collect();
        assert_eq!(operators, ["$sort", "$limit", "$group"]);
        assert_eq!(outputs, [Some(2), None, Some(1)]);
    }

    #[test]
    fn a_generated_join_goes_in_together_and_comes_out_together() {
        use crate::state::app_state::PipelineStage;

        let (mut state, key) = state_with(&["$match", "$limit"]);
        let join = vec![
            PipelineStage::with("$lookup", "{ from: \"users\" }", true),
            PipelineStage::with("$unwind", "{ path: \"$user\" }", true),
        ];

        assert_eq!(state.insert_pipeline_stages(&key, 1, join), Some(1));

        let operators = |state: &AppState| -> Vec<String> {
            let stages = &state.session(&key).unwrap().data.aggregation.stages;
            stages.iter().map(|stage| stage.operator.clone()).collect()
        };
        assert_eq!(operators(&state), ["$match", "$lookup", "$unwind", "$limit"]);
        let aggregation = &state.session(&key).unwrap().data.aggregation;
        // The `$lookup` is what gets edited next, and every stage still has its count slot.
        assert_eq!(aggregation.selected_stage, Some(1));
        assert_eq!(aggregation.stage_doc_counts.len(), 4);

        // One undo, not two: half a join is a pipeline nobody asked for.
        assert!(state.undo_pipeline_edit(&key));
        assert_eq!(operators(&state), ["$match", "$limit"]);
        assert_eq!(state.insert_pipeline_stages(&key, 0, Vec::new()), None);
    }

    #[test]
    fn undo_and_redo_restore_stages_and_selection() {
        let (mut state, key) = state_with(&["$match", "$group"]);
        state.set_pipeline_selected_stage(&key, Some(0));
        state.remove_pipeline_stage(&key, 0);
        assert_eq!(state.session(&key).unwrap().data.aggregation.stages.len(), 1);

        assert!(state.undo_pipeline_edit(&key));
        let aggregation = &state.session(&key).unwrap().data.aggregation;
        assert_eq!(aggregation.stages[0].operator, "$match");
        assert_eq!(aggregation.selected_stage, Some(0));

        assert!(state.redo_pipeline_edit(&key));
        assert_eq!(state.session(&key).unwrap().data.aggregation.stages.len(), 1);
        assert!(!state.redo_pipeline_edit(&key));
    }

    #[test]
    fn typing_in_one_stage_is_one_undo_step_that_survives_structural_undo() {
        let (mut state, key) = state_with(&["$match"]);
        let template = state.session(&key).unwrap().data.aggregation.stages[0].body.clone();
        state.set_pipeline_stage_body(&key, 0, "{ a".to_string());
        state.set_pipeline_stage_body(&key, 0, "{ a: 1 }".to_string());
        let typed = state.pipeline_undo_top(&key);
        state.add_pipeline_stage(&key, "$sort");

        assert!(state.undo_pipeline_edit(&key));
        let aggregation = &state.session(&key).unwrap().data.aggregation;
        assert_eq!(aggregation.stages.len(), 1);
        assert_eq!(aggregation.stages[0].body, "{ a: 1 }");
        assert_eq!(state.pipeline_undo_top(&key), typed);

        assert!(state.undo_pipeline_edit(&key));
        assert_eq!(state.session(&key).unwrap().data.aggregation.stages[0].body, template);
    }

    #[test]
    fn undo_serials_stay_unique_past_the_history_limit() {
        let (mut state, key) = state_with(&["$match"]);
        for _ in 0..crate::state::app_state::PIPELINE_UNDO_LIMIT + 5 {
            state.toggle_pipeline_stage_enabled(&key, 0);
        }
        let top = state.pipeline_undo_top(&key);
        state.undo_pipeline_edit(&key);
        state.toggle_pipeline_stage_enabled(&key, 0);
        assert_ne!(state.pipeline_undo_top(&key), top);
    }

    #[test]
    fn limit_change_keeps_results_and_counts() {
        let (mut state, key) = state_with(&["$match"]);
        {
            let aggregation = &mut state.session_mut(&key).unwrap().data.aggregation;
            aggregation.stage_doc_counts[0].output = Some(7);
            aggregation.results = Some(Arc::new(vec![doc! { "_id": 1 }]));
            aggregation.results_page = 2;
        }

        state.set_pipeline_result_limit(&key, 25);

        let aggregation = &state.session(&key).unwrap().data.aggregation;
        assert_eq!(aggregation.result_limit, 25);
        assert_eq!(aggregation.stage_doc_counts[0].output, Some(7));
        assert!(aggregation.results.is_some());
        assert_eq!(aggregation.results_page, 0);
    }

    #[test]
    fn stage_stats_toggle_resets_counts_but_keeps_results() {
        let (mut state, key) = state_with(&["$match"]);
        {
            let aggregation = &mut state.session_mut(&key).unwrap().data.aggregation;
            aggregation.stage_doc_counts[0].output = Some(9);
            aggregation.results = Some(Arc::new(vec![doc! { "_id": 1 }]));
        }

        state.set_pipeline_stage_stats_mode(&key, StageStatsMode::Off);

        let aggregation = &state.session(&key).unwrap().data.aggregation;
        assert_eq!(aggregation.stage_doc_counts[0].output, None);
        assert!(aggregation.results.is_some());
        assert_eq!(aggregation.stage_stats_mode, StageStatsMode::Off);
    }

    #[test]
    fn text_mode_previews_the_whole_pipeline() {
        let (mut state, key) = state_with(&["$match", "$group"]);
        state.set_pipeline_selected_stage(&key, Some(0));
        state.set_pipeline_preview_input(&key, true);

        state.set_pipeline_text_mode(&key, true);
        let aggregation = &state.session(&key).unwrap().data.aggregation;
        assert_eq!(aggregation.selected_stage, None);
        assert_eq!(aggregation.preview_target(), Some(1));

        state.set_pipeline_text_mode(&key, false);
        assert_eq!(state.session(&key).unwrap().data.aggregation.selected_stage, Some(1));
    }
}
