//! Aggregation pipeline state for a collection session.

use std::sync::{Arc, Mutex, atomic::AtomicU64};

use futures::future::AbortHandle;
use mongodb::bson::Document;
use serde::{Deserialize, Serialize};

use super::types::DocumentViewMode;

#[derive(Debug, Clone, Eq, Serialize, Deserialize)]
pub struct PipelineStage {
    /// Which stage this is, for the UI: stages are dragged to reorder, so a row keyed by position
    /// would change identity in the middle of the drag. Runtime only. It is never saved, and it
    /// is not part of equality, so a reloaded pipeline still equals the one that was stored.
    #[serde(skip, default = "next_stage_id")]
    pub id: u64,
    pub operator: String,
    pub body: String,
    #[serde(default = "stage_enabled_default")]
    pub enabled: bool,
}

impl PartialEq for PipelineStage {
    fn eq(&self, other: &Self) -> bool {
        self.operator == other.operator && self.body == other.body && self.enabled == other.enabled
    }
}

fn stage_enabled_default() -> bool {
    true
}

fn next_stage_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl PipelineStage {
    pub fn new(operator: impl Into<String>) -> Self {
        let operator = operator.into();
        let body = default_stage_body(&operator).unwrap_or("{}").to_string();
        Self::with(operator, body, true)
    }

    pub fn with(operator: impl Into<String>, body: impl Into<String>, enabled: bool) -> Self {
        Self { id: next_stage_id(), operator: operator.into(), body: body.into(), enabled }
    }

    /// A copy that is its own stage, for Duplicate: a plain clone keeps the original's id.
    pub fn duplicate(&self) -> Self {
        Self { id: next_stage_id(), ..self.clone() }
    }
}

pub(crate) fn default_stage_body(operator: &str) -> Option<&'static str> {
    match operator {
        "$match" => Some("{\n  field: value\n}"),
        "$project" => Some("{\n  field: 1\n}"),
        "$group" => Some("{\n  _id: \"$field\",\n  value: { $sum: 1 }\n}"),
        "$sort" => Some("{\n  field: 1\n}"),
        "$limit" => Some("10"),
        "$skip" => Some("0"),
        "$lookup" => Some(
            "{\n  from: \"collection\",\n  localField: \"field\",\n  foreignField: \"field\",\n  as: \"results\"\n}",
        ),
        "$unwind" => Some("\"$field\""),
        "$addFields" | "$set" => Some("{\n  newField: value\n}"),
        "$unset" => Some("\"field\""),
        "$replaceRoot" => Some("{\n  newRoot: \"$field\"\n}"),
        "$replaceWith" => Some("\"$field\""),
        "$count" => Some("\"count\""),
        "$sample" => Some("{\n  size: 10\n}"),
        "$bucket" => Some(
            "{\n  groupBy: \"$field\",\n  boundaries: [0, 10],\n  default: \"other\",\n  output: {\n    count: { $sum: 1 }\n  }\n}",
        ),
        "$bucketAuto" => Some(
            "{\n  groupBy: \"$field\",\n  buckets: 5,\n  output: {\n    count: { $sum: 1 }\n  }\n}",
        ),
        "$facet" => Some("{\n  facet: [\n    { $match: { field: value } }\n  ]\n}"),
        "$unionWith" => Some(
            "{\n  coll: \"collection\",\n  pipeline: [\n    { $match: { field: value } }\n  ]\n}",
        ),
        "$redact" => Some(
            "{\n  $cond: {\n    if: { $gt: [\"$field\", value] },\n    then: \"$$KEEP\",\n    else: \"$$PRUNE\"\n  }\n}",
        ),
        "$graphLookup" => Some(
            "{\n  from: \"collection\",\n  startWith: \"$field\",\n  connectFromField: \"field\",\n  connectToField: \"field\",\n  as: \"results\"\n}",
        ),
        "$out" => Some("\"collection\""),
        "$merge" => Some("{\n  into: \"collection\"\n}"),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StageStatsMode {
    Off,
    Counts,
    #[default]
    CountsAndTiming,
}

impl StageStatsMode {
    pub fn counts_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn timings_enabled(self) -> bool {
        matches!(self, Self::CountsAndTiming)
    }
}

/// What produced the current results or error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineRun {
    /// Last stage included in the run; `None` means the collection itself.
    pub target: Option<usize>,
    /// `PipelineState::edit_revision` when the run started.
    pub revision: u64,
    /// Results page the run fetched.
    pub page: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineSnapshot {
    pub stages: Vec<PipelineStage>,
    pub selected_stage: Option<usize>,
    /// Unique per push, so an Undo button can tell its edit is still on top.
    pub serial: u64,
}

/// Consecutive edits of one kind share a single undo step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoGroup {
    StageBody(usize),
    Text,
}

pub(crate) const PIPELINE_UNDO_LIMIT: usize = 50;

#[derive(Debug, Clone)]
pub struct PipelineState {
    pub stages: Vec<PipelineStage>,
    pub selected_stage: Option<usize>,
    pub results: Option<Arc<Vec<Document>>>,
    pub stage_doc_counts: Vec<StageDocCounts>,
    pub loading: bool,
    pub error: Option<crate::error::ErrorReport>,
    /// Stage the error belongs to, when the run could tell.
    pub error_stage: Option<usize>,
    pub request_id: u64,
    pub stage_stats_mode: StageStatsMode,
    pub run_generation: Arc<AtomicU64>,
    pub abort_handle: Arc<Mutex<Option<AbortHandle>>>,
    pub result_limit: i64,
    pub results_page: u64,
    pub last_run_time_ms: Option<u64>,
    pub results_view_mode: DocumentViewMode,
    pub last_run: Option<PipelineRun>,
    /// Bumped by every change to stage content, order, or enablement.
    pub edit_revision: u64,
    /// Preview the documents entering the selected stage instead of leaving it.
    pub preview_input: bool,
    pub auto_run: bool,
    pub text_mode: bool,
    pub undo_stack: Arc<Vec<PipelineSnapshot>>,
    pub redo_stack: Arc<Vec<PipelineSnapshot>>,
    pub undo_serial: u64,
    pub undo_group: Option<UndoGroup>,
    /// Text mode input that doesn't parse yet, kept per session.
    pub text_draft: Option<String>,
    /// The view whose definition this pipeline is an edit of. Saving then updates that view
    /// instead of asking for a new name.
    pub editing_view: Option<EditingView>,
}

/// A view's definition open in the builder, and what the server holds for it.
#[derive(Debug, Clone)]
pub struct EditingView {
    pub name: String,
    /// The definition as last read or written, in the form the builder produces. Held in that
    /// form, not the server's, so an untouched pipeline compares equal: the server may store
    /// `5` as a double where the builder writes an integer.
    pub saved: Vec<Document>,
    pub updating: bool,
}

/// Where an edit of a view stands against the server's copy of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewEditStatus {
    /// The pipeline is the view's definition. There is nothing to save.
    UpToDate,
    Changed,
    Updating,
}

impl Default for PipelineState {
    fn default() -> Self {
        Self {
            stages: Vec::new(),
            selected_stage: None,
            results: None,
            stage_doc_counts: Vec::new(),
            loading: false,
            error: None,
            error_stage: None,
            request_id: 0,
            stage_stats_mode: StageStatsMode::default(),
            run_generation: Arc::new(AtomicU64::new(0)),
            abort_handle: Arc::new(Mutex::new(None)),
            result_limit: 50,
            results_page: 0,
            last_run_time_ms: None,
            results_view_mode: DocumentViewMode::default(),
            last_run: None,
            edit_revision: 0,
            preview_input: false,
            auto_run: true,
            text_mode: false,
            undo_stack: Arc::default(),
            redo_stack: Arc::default(),
            undo_serial: 0,
            undo_group: None,
            text_draft: None,
            editing_view: None,
        }
    }
}

impl PipelineState {
    /// Last stage the preview runs through; `None` means the collection itself.
    pub fn preview_target(&self) -> Option<usize> {
        let last = self.stages.len().checked_sub(1)?;
        if self.text_mode {
            return Some(last);
        }
        let stage = self.selected_stage.unwrap_or(last);
        if self.preview_input { stage.checked_sub(1) } else { Some(stage) }
    }

    /// Results or error exist but no longer describe the current pipeline and preview point.
    pub fn is_stale(&self) -> bool {
        self.last_run.is_some_and(|run| {
            run.revision != self.edit_revision || run.target != self.preview_target()
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct StageDocCounts {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub time_ms: Option<u64>,
}

#[cfg(test)]
mod stage_id_tests {
    use super::PipelineStage;

    #[test]
    fn a_stage_id_is_runtime_only() {
        let stage = PipelineStage::with("$match", "{ a: 1 }", true);
        let other = PipelineStage::with("$match", "{ a: 1 }", true);
        assert_ne!(stage.id, other.id, "every stage is its own");
        assert_eq!(stage, other, "identity is not part of equality");

        let saved = serde_json::to_string(&stage).unwrap();
        assert!(!saved.contains("\"id\""), "the id was persisted: {saved}");
        let loaded: PipelineStage = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded, stage);
        assert_ne!(loaded.id, stage.id, "a loaded stage gets a fresh id");

        assert_eq!(stage.clone().id, stage.id, "a snapshot keeps the id");
        let copy = stage.duplicate();
        assert_ne!(copy.id, stage.id, "a duplicate is a second stage");
        assert_eq!(copy, stage);
    }
}
