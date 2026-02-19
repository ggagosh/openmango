//! JSON editor tab state helpers.

use uuid::Uuid;

use crate::bson::DocumentKey;
use crate::state::AppState;
use crate::state::app_state::types::{ActiveTab, JsonEditorTabState, JsonEditorTarget, TabKey};

impl AppState {
    pub fn json_editor_tab(&self, id: Uuid) -> Option<&JsonEditorTabState> {
        self.json_editor_tabs.get(&id)
    }

    pub fn json_editor_tab_mut(&mut self, id: Uuid) -> Option<&mut JsonEditorTabState> {
        self.json_editor_tabs.get_mut(&id)
    }

    pub fn active_json_editor_tab_id(&self) -> Option<Uuid> {
        let ActiveTab::Index(index) = self.tabs.active else {
            return None;
        };
        match self.tabs.open.get(index) {
            Some(TabKey::JsonEditor(key)) => Some(key.id),
            _ => None,
        }
    }

    pub fn active_json_editor_tab(&self) -> Option<&JsonEditorTabState> {
        self.active_json_editor_tab_id().and_then(|id| self.json_editor_tabs.get(&id))
    }

    pub fn json_editor_tab_label(&self, id: Uuid) -> String {
        self.json_editor_tabs
            .get(&id)
            .map(JsonEditorTabState::tab_label)
            .unwrap_or_else(|| "JSON Editor".to_string())
    }

    pub fn set_json_editor_tab_content(&mut self, id: Uuid, content: String) {
        if let Some(tab) = self.json_editor_tabs.get_mut(&id) {
            tab.content = content;
        }
    }

    pub fn refresh_json_editor_baseline(
        &mut self,
        session_key: &crate::state::SessionKey,
        doc_key: &DocumentKey,
    ) {
        let Some(current_document) = self.document_for_key(session_key, doc_key) else {
            return;
        };

        for tab in self.json_editor_tabs.values_mut() {
            if tab.session_key != *session_key {
                continue;
            }

            let JsonEditorTarget::Document { doc_key: tab_doc_key, baseline_document } =
                &mut tab.target
            else {
                continue;
            };
            if tab_doc_key == doc_key {
                *baseline_document = current_document.clone();
            }
        }
    }
}
