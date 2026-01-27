//! Aggregation pipeline state for a collection session.

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
        Self { operator: operator.into(), body: "{}".to_string(), enabled: true }
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
