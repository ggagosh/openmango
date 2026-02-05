use chrono::{DateTime, Utc};
use mongodb::bson::Document;

pub use super::logic::{Suggestion, SuggestionKind};

pub const MAX_OUTPUT_RUNS: usize = 50;
pub const MAX_OUTPUT_LINES: usize = 5000;
pub const SYSTEM_RUN_ID: u64 = 0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ForgeOutputTab {
    Results,
    Raw,
}

pub struct ForgeRunOutput {
    pub id: u64,
    pub started_at: DateTime<Utc>,
    pub code_preview: String,
    pub raw_lines: Vec<String>,
    pub error: Option<String>,
    pub last_print_line: Option<String>,
}

pub struct ResultPage {
    pub label: String,
    pub docs: Vec<Document>,
    pub pinned: bool,
}
