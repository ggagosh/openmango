use gpui::{Bounds, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::CollectionSubview;
use crate::state::app_state::PipelineStage;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum WorkspaceTabKind {
    #[default]
    Collection,
    Database,
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
