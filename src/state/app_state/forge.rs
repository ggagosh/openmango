//! Forge query shell state management.

use uuid::Uuid;

use super::AppState;
use super::types::{ForgeTabKey, ForgeTabState};

impl AppState {
    /// Get the active Forge tab ID if one is selected
    pub fn active_forge_tab_id(&self) -> Option<Uuid> {
        use super::types::{ActiveTab, TabKey};
        match self.tabs.active {
            ActiveTab::Index(index) => self.tabs.open.get(index).and_then(|tab| match tab {
                TabKey::Forge(key) => Some(key.id),
                _ => None,
            }),
            _ => None,
        }
    }

    /// Get the active Forge tab key if one is selected
    pub fn active_forge_tab_key(&self) -> Option<&ForgeTabKey> {
        use super::types::{ActiveTab, TabKey};
        match self.tabs.active {
            ActiveTab::Index(index) => self.tabs.open.get(index).and_then(|tab| match tab {
                TabKey::Forge(key) => Some(key),
                _ => None,
            }),
            _ => None,
        }
    }

    /// Get Forge tab label for display
    pub fn forge_tab_label(&self, id: Uuid) -> String {
        use super::types::TabKey;
        // Find the forge tab key to get the database name
        for tab in &self.tabs.open {
            if let TabKey::Forge(key) = tab
                && key.id == id
            {
                return format!("Forge: {}", key.database);
            }
        }
        "Forge".to_string()
    }

    /// Get the stored content for a Forge tab.
    pub fn forge_tab_content(&self, id: Uuid) -> Option<&str> {
        self.forge_tabs.get(&id).map(|state| state.content.as_str())
    }

    /// Update the stored content for a Forge tab.
    pub fn set_forge_tab_content(&mut self, id: Uuid, content: String) {
        if let Some(state) = self.forge_tabs.get_mut(&id) {
            state.content = content;
        }
    }
}
