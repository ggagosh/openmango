//! Explain state helpers for collection sessions.

use crate::state::AppState;
use crate::state::app_state::types::{
    ExplainOpenMode, ExplainPanelTab, ExplainViewMode, SessionKey,
};

impl AppState {
    pub fn set_explain_open_mode(&mut self, session_key: &SessionKey, open_mode: ExplainOpenMode) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.explain.open_mode = open_mode;
        }
    }

    pub fn set_explain_mode(&mut self, session_key: &SessionKey, mode: ExplainViewMode) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.explain.view_mode = mode;
        }
    }

    pub fn set_explain_panel_tab(&mut self, session_key: &SessionKey, tab: ExplainPanelTab) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.explain.panel_tab = tab;
        }
    }

    pub fn cycle_explain_run(&mut self, session_key: &SessionKey, step: isize) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.explain.cycle_current_run(step);
        }
    }

    pub fn compare_explain_previous_run(&mut self, session_key: &SessionKey) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.explain.compare_with_previous_run();
        }
    }

    pub fn clear_explain_compare_run(&mut self, session_key: &SessionKey) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.explain.clear_compare_run();
        }
    }

    pub fn clear_explain_previous_runs(&mut self, session_key: &SessionKey) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.explain.clear_previous_runs_keep_current();
        }
    }

    pub fn set_explain_selected_node(
        &mut self,
        session_key: &SessionKey,
        selected_node_id: Option<String>,
    ) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.explain.selected_node_id = selected_node_id;
        }
    }
}
