//! Filter, sort, and projection operations for sessions.

use mongodb::bson::Document;

use crate::state::AppState;
use crate::state::app_state::types::{CollectionSubview, SessionKey};

impl AppState {
    pub fn session_filter(&self, key: &SessionKey) -> Option<Document> {
        self.session_data(key).and_then(|data| data.filter.clone())
    }

    pub fn set_filter(&mut self, session_key: &SessionKey, raw: String, filter: Option<Document>) {
        self.promote_preview_collection_tab(session_key);
        if let Some(session) = self.session_mut(session_key) {
            session.data.filter_raw = raw;
            session.data.filter = filter;
            session.data.page = 0;
        }
        self.update_workspace_session_filters(session_key);
    }

    pub fn clear_filter(&mut self, session_key: &SessionKey) {
        self.promote_preview_collection_tab(session_key);
        if let Some(session) = self.session_mut(session_key) {
            session.data.filter_raw.clear();
            session.data.filter = None;
            session.data.page = 0;
        }
        self.update_workspace_session_filters(session_key);
    }

    pub fn set_sort_projection(
        &mut self,
        session_key: &SessionKey,
        sort_raw: String,
        sort: Option<Document>,
        projection_raw: String,
        projection: Option<Document>,
    ) {
        self.promote_preview_collection_tab(session_key);
        if let Some(session) = self.session_mut(session_key) {
            session.data.sort_raw = sort_raw;
            session.data.sort = sort;
            session.data.projection_raw = projection_raw;
            session.data.projection = projection;
            session.data.page = 0;
        }
        self.update_workspace_session_filters(session_key);
    }

    /// Save raw input text without parsing or executing a query.
    /// Used when switching sessions to preserve drafts across tab switches.
    pub fn save_filter_draft(&mut self, session_key: &SessionKey, raw: String) {
        self.promote_preview_collection_tab(session_key);
        if let Some(session) = self.session_mut(session_key) {
            session.data.filter_raw = raw;
        }
    }

    pub fn save_sort_projection_draft(
        &mut self,
        session_key: &SessionKey,
        sort_raw: String,
        projection_raw: String,
    ) {
        self.promote_preview_collection_tab(session_key);
        if let Some(session) = self.session_mut(session_key) {
            session.data.sort_raw = sort_raw;
            session.data.projection_raw = projection_raw;
        }
    }

    pub fn set_collection_subview(
        &mut self,
        session_key: &SessionKey,
        subview: CollectionSubview,
    ) -> bool {
        self.promote_preview_collection_tab(session_key);
        let mut should_load = false;
        let mut changed = false;

        if let Some(session) = self.session_mut(session_key) {
            if session.view.subview == subview {
                return false;
            }
            session.view.subview = subview;
            session.view.stats_open = matches!(subview, CollectionSubview::Stats);
            should_load = subview == CollectionSubview::Stats
                && !session.data.stats_loading
                && (session.data.stats.is_none() || session.data.stats_error.is_some());
            changed = true;
        }

        if changed {
            self.update_workspace_session_view(session_key);
        }

        should_load
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ActiveTab, CollectionSubview, TabKey};

    fn preview_state() -> (AppState, SessionKey) {
        let mut state = AppState::new();
        let key = SessionKey::new(uuid::Uuid::new_v4(), "db", "col");
        state.ensure_session(key.clone());
        state.tabs.preview = Some(key.clone());
        state.tabs.active = ActiveTab::Preview;
        (state, key)
    }

    #[test]
    fn save_filter_draft_promotes_preview_tab() {
        let (mut state, key) = preview_state();

        state.save_filter_draft(&key, r#"{ "name": "alice" }"#.to_string());

        assert!(state.preview_tab().is_none());
        assert!(matches!(state.open_tabs(), [TabKey::Collection(tab)] if tab == &key));
        assert_eq!(state.active_tab(), ActiveTab::Index(0));
        assert_eq!(
            state.session_data(&key).map(|session| session.filter_raw.as_str()),
            Some(r#"{ "name": "alice" }"#)
        );
    }

    #[test]
    fn set_collection_subview_promotes_preview_tab() {
        let (mut state, key) = preview_state();

        let should_load = state.set_collection_subview(&key, CollectionSubview::Indexes);

        assert!(!should_load);
        assert!(state.preview_tab().is_none());
        assert!(matches!(state.open_tabs(), [TabKey::Collection(tab)] if tab == &key));
        assert_eq!(state.active_tab(), ActiveTab::Index(0));
        assert_eq!(state.session_subview(&key), Some(CollectionSubview::Indexes));
    }
}
