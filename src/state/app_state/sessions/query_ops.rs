//! Filter, sort, and projection operations for sessions.

use mongodb::bson::Document;

use crate::state::AppState;
use crate::state::app_state::types::{CollectionSubview, SessionKey};

impl AppState {
    pub fn session_filter(&self, key: &SessionKey) -> Option<Document> {
        self.session_data(key).and_then(|data| data.filter.clone())
    }

    pub fn set_filter(&mut self, session_key: &SessionKey, raw: String, filter: Option<Document>) {
        if let Some(session) = self.session_mut(session_key) {
            session.data.filter_raw = raw;
            session.data.filter = filter;
            session.data.page = 0;
        }
        self.update_workspace_session_filters(session_key);
    }

    pub fn clear_filter(&mut self, session_key: &SessionKey) {
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
        if let Some(session) = self.session_mut(session_key) {
            session.data.sort_raw = sort_raw;
            session.data.sort = sort;
            session.data.projection_raw = projection_raw;
            session.data.projection = projection;
            session.data.page = 0;
        }
        self.update_workspace_session_filters(session_key);
    }

    pub fn set_collection_subview(
        &mut self,
        session_key: &SessionKey,
        subview: CollectionSubview,
    ) -> bool {
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
