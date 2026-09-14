//! Transfer tab state helpers.

use uuid::Uuid;

use crate::state::AppState;
use crate::state::app_state::types::{ActiveTab, TabKey, TransferTabState};

impl AppState {
    pub fn transfer_tab(&self, id: Uuid) -> Option<&TransferTabState> {
        self.transfer_tabs.get(&id)
    }

    pub fn transfer_tab_mut(&mut self, id: Uuid) -> Option<&mut TransferTabState> {
        self.transfer_tabs.get_mut(&id)
    }

    pub fn active_transfer_tab_id(&self) -> Option<Uuid> {
        let ActiveTab::Index(index) = self.tabs.active else {
            return None;
        };
        match self.tabs.open.get(index) {
            Some(TabKey::Transfer(key)) => Some(key.id),
            _ => None,
        }
    }

    pub fn transfer_tab_label(&self, id: Uuid) -> String {
        self.transfer_tabs
            .get(&id)
            .map(|tab| tab.tab_label())
            .unwrap_or_else(|| "Transfer".to_string())
    }
}
