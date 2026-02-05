use std::sync::Arc;

use gpui::UniformListScrollHandle;

use crate::state::SessionDocument;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultViewMode {
    Tree,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ResultEmptyState {
    NoDocuments,
    NoMatches,
    Custom(String),
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct ResultViewProps {
    pub documents: Arc<Vec<SessionDocument>>,
    pub expanded_nodes: Arc<std::collections::HashSet<String>>,
    pub search_query: String,
    pub scroll_handle: UniformListScrollHandle,
    pub empty_state: ResultEmptyState,
    pub view_mode: ResultViewMode,
}

pub type ToggleNodeCallback = Arc<dyn Fn(String, &mut gpui::App) + Send + Sync>;
