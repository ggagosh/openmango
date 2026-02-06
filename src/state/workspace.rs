use gpui::{Bounds, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::CollectionSubview;
use crate::state::app_state::{PipelineStage, TransferTabState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WorkspaceTabKind {
    #[default]
    Collection,
    Database,
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
        };

        let encoded = serde_json::to_string(&tab).expect("workspace tab should serialize");
        let decoded: WorkspaceTab =
            serde_json::from_str(&encoded).expect("workspace tab should deserialize");
        assert_eq!(decoded.kind, WorkspaceTabKind::Forge);
        assert_eq!(decoded.forge_content, tab.forge_content);
    }
}
