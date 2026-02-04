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

pub fn format_result_tab_label(label: &str, idx: usize) -> String {
    let trimmed = label.trim();
    let base = if trimmed.is_empty() { format!("Result {}", idx + 1) } else { trimmed.to_string() };
    const MAX_LEN: usize = 32;
    if base.chars().count() <= MAX_LEN {
        return base;
    }
    let shortened: String = base.chars().take(MAX_LEN.saturating_sub(3)).collect();
    format!("{shortened}...")
}
