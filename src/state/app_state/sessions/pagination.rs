//! Pagination operations for sessions.

use crate::state::AppState;
use crate::state::app_state::types::SessionKey;

impl AppState {
    pub fn prev_page(&mut self, session_key: &SessionKey) -> bool {
        if let Some(session) = self.session_mut(session_key)
            && session.data.page > 0
        {
            session.data.page -= 1;
            return true;
        }
        false
    }

    pub fn next_page(&mut self, session_key: &SessionKey, total_pages: u64) -> bool {
        if let Some(session) = self.session_mut(session_key)
            && session.data.page + 1 < total_pages
        {
            session.data.page += 1;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use crate::state::{AppState, SessionKey};

    #[test]
    fn paging_helpers_enforce_bounds() {
        let mut state = AppState::new();
        let session_key = SessionKey::new(uuid::Uuid::new_v4(), "db", "col");
        state.ensure_session(session_key.clone());

        assert!(!state.prev_page(&session_key));
        assert!(state.next_page(&session_key, 2));
        assert!(state.prev_page(&session_key));
    }
}
