use gpui_kit::{Keystroke, SharedString};

use crate::components::ConnectionIdentity;

/// A single action in the palette.
#[derive(Clone, Default)]
pub struct ActionItem {
    pub id: SharedString,
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub category: ActionCategory,
    pub shortcut: Option<Keystroke>,
    /// Extra search terms, such as "dump" for Export Data.
    pub keywords: &'static [&'static str],
    pub available: bool,
    pub priority: i32,
    /// Highlighted items render with accent color and sort to the top.
    pub highlighted: bool,
    /// Marks the current choice, such as the active theme.
    pub checked: bool,
    /// Connection rows render the connection icon, color and identity tags.
    pub connection: Option<ConnectionIdentity>,
}

impl ActionItem {
    pub fn search_text(&self) -> String {
        let mut text = self.label.to_string();
        for part in
            self.detail.iter().map(|detail| detail.as_ref()).chain(self.keywords.iter().copied())
        {
            text.push(' ');
            text.push_str(part);
        }
        text
    }
}

/// Categories for grouping and ordering actions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ActionCategory {
    Navigation,
    #[default]
    Command,
    Tab,
    View,
    Connected,
    Saved,
    DarkTheme,
    LightTheme,
}

impl ActionCategory {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Navigation => "Navigation",
            Self::Command => "Commands",
            Self::Tab => "Tabs",
            Self::View => "View",
            Self::Connected => "Connected",
            Self::Saved => "Saved",
            Self::DarkTheme => "Dark",
            Self::LightTheme => "Light",
        }
    }

    /// Group order. Navigation can hold thousands of collections, so it goes last.
    pub fn sort_order(&self) -> u8 {
        match self {
            Self::Tab => 0,
            Self::Command => 1,
            Self::View => 2,
            Self::Navigation => 3,
            Self::Connected => 4,
            Self::Saved => 5,
            Self::DarkTheme => 6,
            Self::LightTheme => 7,
        }
    }
}

/// Palette operating mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaletteMode {
    #[default]
    All,
    Theme,
    Connect,
    Disconnect,
    Navigate,
}

impl PaletteMode {
    pub fn title(&self) -> &'static str {
        match self {
            Self::All => "Commands",
            Self::Theme => "Themes",
            Self::Connect => "Connections",
            Self::Disconnect => "Disconnect",
            Self::Navigate => "Go to",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            Self::All => "Search commands…",
            Self::Theme => "Search themes…",
            Self::Connect => "Search connections…",
            Self::Disconnect => "Search connections to disconnect…",
            Self::Navigate => "Search databases and collections…",
        }
    }

    /// The scope a leading `#` or `@` jumps to, with the rest of the query.
    pub fn from_prefix(query: &str) -> Option<(Self, &str)> {
        let mut chars = query.chars();
        let mode = match chars.next()? {
            '#' => Self::Navigate,
            '@' => Self::Connect,
            _ => return None,
        };
        Some((mode, chars.as_str()))
    }
}

/// Payload returned when user executes an action.
pub struct ActionExecution {
    pub action_id: SharedString,
}
