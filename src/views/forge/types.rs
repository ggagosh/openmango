use chrono::{DateTime, Utc};
use gpui_kit::UniformListScrollHandle;
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

use crate::state::SessionDocument;

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
    pub evaluation_lines: Vec<String>,
    pub error: Option<String>,
    pub last_print_line: Option<String>,
}

pub struct ResultPage {
    pub id: Uuid,
    pub label: String,
    pub documents: Arc<Vec<SessionDocument>>,
    pub pinned: bool,
    pub print_run: Option<u64>,
    pub expanded_nodes: HashSet<String>,
    pub scroll: UniformListScrollHandle,
}
