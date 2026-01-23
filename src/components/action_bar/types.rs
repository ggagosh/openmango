use gpui::SharedString;

/// A single action in the palette.
#[derive(Clone)]
pub struct ActionItem {
    pub id: SharedString,
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub category: ActionCategory,
    pub shortcut: Option<SharedString>,
    pub available: bool,
    pub priority: i32,
}

/// Categories for grouping and ordering actions.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ActionCategory {
    Navigation,
    Command,
    Tab,
    View,
}

impl ActionCategory {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Navigation => "Navigation",
            Self::Command => "Commands",
            Self::Tab => "Tabs",
            Self::View => "View",
        }
    }

    pub fn sort_order(&self) -> u8 {
        match self {
            Self::Tab => 0,
            Self::Command => 1,
            Self::Navigation => 2,
            Self::View => 3,
        }
    }
}

/// Result of fuzzy matching with score for ranking.
#[derive(Clone)]
pub struct FilteredAction {
    pub item: ActionItem,
    pub score: usize,
}

/// Palette operating mode.
#[derive(Clone, Default, PartialEq)]
pub enum PaletteMode {
    #[default]
    All,
}

/// Payload returned when user executes an action.
pub struct ActionExecution {
    pub action_id: SharedString,
}
