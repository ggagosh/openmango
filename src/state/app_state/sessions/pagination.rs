//! Pagination operations for sessions.

use crate::state::AppState;
use crate::state::app_state::types::SessionKey;

impl AppState {
    pub fn set_document_page(&mut self, session_key: &SessionKey, page: u64) {
        if let Some(session) = self.session_mut(session_key) {
            let pages = session.data.total.div_ceil(session.data.per_page.max(1) as u64).max(1);
            session.data.page = page.min(pages - 1);
        }
    }

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

    pub fn set_per_page(&mut self, session_key: &SessionKey, per_page: i64) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.per_page = per_page.max(1);
            session.data.page = 0;
        }
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
        state.session_mut(&session_key).unwrap().data.total = 101;
        state.set_per_page(&session_key, 25);
        state.set_document_page(&session_key, 200);
        assert_eq!(state.session_data(&session_key).unwrap().page, 4);
        state.set_per_page(&session_key, 0);
        assert_eq!(state.session_data(&session_key).unwrap().per_page, 1);
        assert_eq!(state.session_data(&session_key).unwrap().page, 0);
    }
}
