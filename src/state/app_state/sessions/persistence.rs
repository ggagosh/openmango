use crate::state::app_state::types::SessionKey;

use super::super::AppState;

impl AppState {
    pub(in crate::state::app_state) fn update_workspace_session_filters(
        &mut self,
        _session_key: &SessionKey,
    ) {
        self.update_workspace_from_state();
    }

    pub(in crate::state::app_state) fn update_workspace_session_view(
        &mut self,
        _session_key: &SessionKey,
    ) {
        self.update_workspace_from_state();
    }
}
