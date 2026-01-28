use crate::state::CollectionSubview;
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
        session_key: &SessionKey,
    ) {
        let is_aggregation = self
            .session(session_key)
            .is_some_and(|session| session.view.subview == CollectionSubview::Aggregation);
        if is_aggregation {
            self.update_workspace_from_state_debounced();
        } else {
            self.update_workspace_from_state();
        }
    }
}
