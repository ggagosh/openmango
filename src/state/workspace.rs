use gpui::{Bounds, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ai::{AiChatEntry, ChatMessage};
use crate::state::CollectionSubview;
use crate::state::app_state::{PipelineStage, TransferTabState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WorkspaceTabKind {
    #[default]
    Collection,
    Database,
    Ai,
    Transfer,
    Forge,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceState {
    pub last_connection_id: Option<Uuid>,
    pub selected_database: Option<String>,
    pub selected_collection: Option<String>,
    pub open_tabs: Vec<WorkspaceTab>,
    pub active_tab: Option<usize>,
    pub expanded_nodes: Vec<String>,
    pub window_state: Option<WindowState>,
    /// Whether the AI side panel was open.
    #[serde(default)]
    pub ai_panel_open: bool,
    /// Draft input text in the AI panel.
    #[serde(default)]
    pub ai_draft_input: String,
    /// Persisted AI chat entries.
    #[serde(default)]
    pub ai_entries: Vec<AiChatEntry>,
    /// Persisted width of AI side panel (px), restored on reopen/restart.
    #[serde(default)]
    pub ai_panel_width: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTab {
    pub database: String,
    pub collection: String,
    #[serde(default)]
    pub kind: WorkspaceTabKind,
    #[serde(default)]
    pub transfer: Option<TransferTabState>,
    #[serde(default)]
    pub filter_raw: String,
    #[serde(default)]
    pub sort_raw: String,
    #[serde(default)]
    pub projection_raw: String,
    #[serde(default)]
    pub aggregation_pipeline: Vec<PipelineStage>,
    #[serde(default)]
    pub stats_open: bool,
    #[serde(default)]
    pub subview: CollectionSubview,
    #[serde(default)]
    pub forge_content: String,
    #[serde(default)]
    pub ai_panel_open: bool,
    #[serde(default)]
    pub ai_draft_input: String,
    /// Unified timeline entries.
    #[serde(default)]
    pub ai_entries: Vec<AiChatEntry>,
    /// Legacy: kept for backwards-compatible deserialization of old workspaces.
    #[serde(default)]
    pub ai_messages: Vec<ChatMessage>,
}

impl WorkspaceTab {
    /// Returns the unified timeline entries, migrating from legacy fields if needed.
    pub fn resolved_ai_entries(&self) -> Vec<AiChatEntry> {
        if !self.ai_entries.is_empty() {
            return self.ai_entries.clone();
        }
        // Legacy: convert old separate messages.
        let mut result = Vec::new();
        for msg in &self.ai_messages {
            result.push(AiChatEntry::LegacyMessage(msg.clone()));
        }
        result
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WindowMode {
    Windowed,
    Maximized,
    Fullscreen,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowState {
    pub mode: WindowMode,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl WindowState {
    pub fn from_bounds(bounds: WindowBounds) -> Self {
        let (mode, bounds) = match bounds {
            WindowBounds::Windowed(bounds) => (WindowMode::Windowed, bounds),
            WindowBounds::Maximized(bounds) => (WindowMode::Maximized, bounds),
            WindowBounds::Fullscreen(bounds) => (WindowMode::Fullscreen, bounds),
        };
        Self {
            mode,
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
        }
    }

    pub fn to_bounds(&self) -> WindowBounds {
        let bounds =
            Bounds::new(point(px(self.x), px(self.y)), size(px(self.width), px(self.height)));
        match self.mode {
            WindowMode::Windowed => WindowBounds::Windowed(bounds),
            WindowMode::Maximized => WindowBounds::Maximized(bounds),
            WindowMode::Fullscreen => WindowBounds::Fullscreen(bounds),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_state_ai_panel_width_defaults_to_none() {
        let raw = r#"{
            "last_connection_id": null,
            "selected_database": null,
            "selected_collection": null,
            "open_tabs": [],
            "active_tab": null,
            "expanded_nodes": [],
            "window_state": null,
            "ai_panel_open": false,
            "ai_draft_input": "",
            "ai_entries": []
        }"#;

        let workspace: WorkspaceState =
            serde_json::from_str(raw).expect("workspace should deserialize");
        assert_eq!(workspace.ai_panel_width, None);
    }

    #[test]
    fn workspace_state_roundtrips_ai_panel_width() {
        let workspace =
            WorkspaceState { ai_panel_width: Some(1234.5), ..WorkspaceState::default() };
        let encoded = serde_json::to_string(&workspace).expect("workspace should serialize");
        let decoded: WorkspaceState =
            serde_json::from_str(&encoded).expect("workspace should deserialize");
        assert_eq!(decoded.ai_panel_width, Some(1234.5));
    }

    #[test]
    fn workspace_tab_defaults_forge_content_when_missing() {
        let raw = r#"{
            "database":"admin",
            "collection":"",
            "kind":"Database",
            "transfer":null,
            "filter_raw":"",
            "sort_raw":"",
            "projection_raw":"",
            "aggregation_pipeline":[],
            "stats_open":false,
            "subview":"Documents"
        }"#;

        let tab: WorkspaceTab =
            serde_json::from_str(raw).expect("workspace tab should deserialize");
        assert!(tab.forge_content.is_empty());
        assert!(!tab.ai_panel_open);
        assert!(tab.ai_draft_input.is_empty());
        assert!(tab.ai_entries.is_empty());
        assert!(tab.ai_messages.is_empty());
    }

    #[test]
    fn workspace_tab_roundtrip_with_forge_kind() {
        let tab = WorkspaceTab {
            database: "admin".to_string(),
            collection: String::new(),
            kind: WorkspaceTabKind::Forge,
            transfer: None,
            filter_raw: String::new(),
            sort_raw: String::new(),
            projection_raw: String::new(),
            aggregation_pipeline: Vec::new(),
            stats_open: false,
            subview: CollectionSubview::Documents,
            forge_content: "db.getCollection(\"users\").find({})".to_string(),
            ai_panel_open: false,
            ai_draft_input: String::new(),
            ai_entries: Vec::new(),
            ai_messages: Vec::new(),
        };

        let encoded = serde_json::to_string(&tab).expect("workspace tab should serialize");
        let decoded: WorkspaceTab =
            serde_json::from_str(&encoded).expect("workspace tab should deserialize");
        assert_eq!(decoded.kind, WorkspaceTabKind::Forge);
        assert_eq!(decoded.forge_content, tab.forge_content);
    }
}
