//! Aggregation pipeline state for a collection session.

use std::sync::{Arc, Mutex, atomic::AtomicU64};

use futures::future::AbortHandle;
use mongodb::bson::Document;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStage {
    pub operator: String,
    pub body: String,
    #[serde(default = "stage_enabled_default")]
    pub enabled: bool,
}

fn stage_enabled_default() -> bool {
    true
}

impl PipelineStage {
    pub fn new(operator: impl Into<String>) -> Self {
        let operator = operator.into();
        let body = default_stage_body(&operator).unwrap_or("{}").to_string();
        Self { operator, body, enabled: true }
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

#[derive(Debug, Clone)]
pub struct PipelineState {
    pub stages: Vec<PipelineStage>,
    pub selected_stage: Option<usize>,
    pub results: Option<Vec<Document>>,
    pub stage_doc_counts: Vec<StageDocCounts>,
    pub analysis: Option<PipelineAnalysis>,
    #[allow(dead_code)]
    pub auto_preview: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub request_id: u64,
    pub stage_stats_mode: StageStatsMode,
    pub run_generation: Arc<AtomicU64>,
    pub abort_handle: Arc<Mutex<Option<AbortHandle>>>,
    pub result_limit: i64,
    pub results_page: u64,
    pub last_run_time_ms: Option<u64>,
}

impl Default for PipelineState {
    fn default() -> Self {
        Self {
            stages: Vec::new(),
            selected_stage: None,
            results: None,
            stage_doc_counts: Vec::new(),
            analysis: None,
            auto_preview: false,
            loading: false,
            error: None,
            request_id: 0,
            stage_stats_mode: StageStatsMode::default(),
            run_generation: Arc::new(AtomicU64::new(0)),
            abort_handle: Arc::new(Mutex::new(None)),
            result_limit: 50,
            results_page: 0,
            last_run_time_ms: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct StageDocCounts {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub time_ms: Option<u64>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PipelineAnalysis {
    pub stages: Vec<StageAnalysis>,
    pub warnings: Vec<AnalysisWarning>,
    pub total_time_ms: u64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StageAnalysis {
    pub docs_in: u64,
    pub docs_out: u64,
    pub strategy: String,
    pub index_name: Option<String>,
    pub time_ms: u64,
    pub memory_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AnalysisWarning {
    pub stage_index: Option<usize>,
    pub message: String,
}
