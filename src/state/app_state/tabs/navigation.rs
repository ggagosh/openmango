//! Back/forward navigation within a collection tab.
//!
//! A collection tab is a stack of views, like a browser tab. Following a reference points the
//! tab at a new view and keeps the one it left in history, so Back restores that view's
//! documents, filter, selection and scroll without re-running the query.

use gpui_kit::Context;
use mongodb::bson::Document;

use crate::state::AppState;
use crate::state::events::AppEvent;

use super::super::types::{NavHistory, SessionKey, TabKey, View};

/// Views one tab keeps behind it. Enough to walk back through a long chain of references
/// without holding every collection a session ever opened.
const MAX_TAB_HISTORY: usize = 20;

impl AppState {
    /// Point the active collection tab at `collection`, filtered by `filter`, keeping the view
    /// it leaves in history. Returns the new view's key.
    ///
    /// A tab with unsaved edits is never navigated away from: the target opens in a new tab
    /// instead, so closing that tab later cannot silently drop buried edits.
    pub fn navigate_to_collection(
        &mut self,
        database: String,
        collection: String,
        filter_raw: String,
        filter: Option<Document>,
        cx: &mut Context<Self>,
    ) -> Option<SessionKey> {
        let current = self.active_collection_session()?;
        if self.dirty_tabs().contains(&current) {
            return self.open_collection_in_new_tab(database, collection, filter_raw, filter, cx);
        }

        let target = self.build_session(&database, &collection, filter_raw, filter)?;

        let mut history = self.tabs.history.remove(&current).unwrap_or_default();
        // A new jump ends the forward chain, the same as following a link in a browser.
        for abandoned in std::mem::take(&mut history.forward) {
            self.cleanup_session(&abandoned);
        }
        history.back.push(current);
        while history.back.len() > MAX_TAB_HISTORY {
            let evicted = history.back.remove(0);
            self.cleanup_session(&evicted);
        }
        self.tabs.history.insert(target.clone(), history);

        self.replace_active_collection_session(target.clone());
        self.show_session(target.clone(), cx);
        Some(target)
    }

    /// Open `collection` in a tab of its own, leaving every existing tab as it is.
    pub fn open_collection_in_new_tab(
        &mut self,
        database: String,
        collection: String,
        filter_raw: String,
        filter: Option<Document>,
        cx: &mut Context<Self>,
    ) -> Option<SessionKey> {
        let target = self.build_session(&database, &collection, filter_raw, filter)?;
        self.tabs.open.push(TabKey::Collection(target.clone()));
        let index = self.tabs.open.len() - 1;
        self.set_active_index(index);
        self.show_session(target.clone(), cx);
        Some(target)
    }

    /// Go back one view in the active tab. Returns false when there is nowhere to go.
    pub fn navigate_back(&mut self, cx: &mut Context<Self>) -> bool {
        self.step_history(
            cx,
            |history| history.back.pop(),
            |history, left| history.forward.insert(0, left),
        )
    }

    /// Go forward one view in the active tab. Returns false when there is nowhere to go.
    pub fn navigate_forward(&mut self, cx: &mut Context<Self>) -> bool {
        self.step_history(
            cx,
            |history| {
                if history.forward.is_empty() { None } else { Some(history.forward.remove(0)) }
            },
            |history, left| history.back.push(left),
        )
    }

    pub fn can_navigate_back(&self) -> bool {
        self.active_tab_history().is_some_and(|history| !history.back.is_empty())
    }

    pub fn can_navigate_forward(&self) -> bool {
        self.active_tab_history().is_some_and(|history| !history.forward.is_empty())
    }

    /// The active tab's trail, oldest first, ending with the view on screen. One entry means
    /// the tab has not navigated anywhere and callers should show no breadcrumb.
    pub fn navigation_trail(&self) -> Vec<SessionKey> {
        let Some(current) = self.active_collection_session() else {
            return Vec::new();
        };
        let mut trail = match self.tabs.history.get(&current) {
            Some(history) => history.back.clone(),
            None => Vec::new(),
        };
        trail.push(current);
        trail
    }

    /// Jump straight to a point in the active tab's trail, as clicking a breadcrumb does.
    ///
    /// Walking back one step at a time keeps one definition of what Back means; a breadcrumb
    /// click is rare enough that the repeated redraws do not matter.
    pub fn navigate_to_trail_index(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
        let depth = self.navigation_trail().len();
        if index + 1 >= depth {
            return false;
        }
        for _ in 0..depth - 1 - index {
            if !self.navigate_back(cx) {
                return false;
            }
        }
        true
    }

    fn active_tab_history(&self) -> Option<&NavHistory> {
        self.tabs.history.get(&self.active_collection_session()?)
    }

    /// Move the active tab one step through its history. `take` pulls the destination off one
    /// stack; `park` puts the view being left onto the other.
    fn step_history(
        &mut self,
        cx: &mut Context<Self>,
        take: impl Fn(&mut NavHistory) -> Option<SessionKey>,
        park: impl Fn(&mut NavHistory, SessionKey),
    ) -> bool {
        let Some(current) = self.active_collection_session() else {
            return false;
        };
        let Some(mut history) = self.tabs.history.remove(&current) else {
            return false;
        };
        let Some(target) = take(&mut history) else {
            self.tabs.history.insert(current, history);
            return false;
        };
        // The view being left always lands on the opposite stack, so the history never empties.
        park(&mut history, current);
        self.tabs.history.insert(target.clone(), history);

        self.replace_active_collection_session(target.clone());
        self.show_session(target, cx);
        true
    }

    /// Create the view a navigation lands on, with its filter already applied.
    ///
    /// The filter is written straight onto the session rather than through `set_filter`, which
    /// would promote a preview tab; navigating inside a preview tab must leave it a preview.
    fn build_session(
        &mut self,
        database: &str,
        collection: &str,
        filter_raw: String,
        filter: Option<Document>,
    ) -> Option<SessionKey> {
        let conn_id = self.conn.selected_connection?;
        if !self.conn.active.contains_key(&conn_id) {
            return None;
        }
        let instance = self.allocate_session_instance();
        let key = SessionKey::with_instance(conn_id, database, collection, instance);
        let session = self.ensure_session(key.clone());
        session.data.filter_raw = filter_raw;
        session.data.set_filter(filter);
        self.update_workspace_session_filters(&key);
        Some(key)
    }

    /// Bring `key` on screen: select its namespace, switch to the documents view and tell the
    /// document view to catch up. The view loads the session if it has never been loaded, and
    /// otherwise redraws what is already there — which is what makes Back instant.
    fn show_session(&mut self, key: SessionKey, cx: &mut Context<Self>) {
        self.set_selected_connection_internal(key.connection_id);
        self.conn.selected_database = Some(key.database.clone());
        self.conn.selected_collection = Some(key.collection.clone());
        self.current_view = View::Documents;
        self.ensure_session(key);
        self.update_workspace_from_state_debounced();
        cx.emit(AppEvent::ViewChanged);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use gpui_kit::{AppContext as _, Entity, TestAppContext};
    use mongodb::bson::doc;
    use tempfile::TempDir;

    use super::MAX_TAB_HISTORY;
    use crate::connection::ConnectionManager;
    use crate::models::{ActiveConnection, SavedConnection};
    use crate::state::{AppState, ConfigManager, SessionKey};

    /// One connected server holding `shop`, with `shop.users` open in a permanent tab.
    fn setup(cx: &mut TestAppContext) -> (Entity<AppState>, SessionKey, TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let saved = SavedConnection::new("Local".into(), "mongodb://localhost:27017".into());
        let conn_id = saved.id;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let client = runtime.block_on(async {
            mongodb::Client::with_options(mongodb::options::ClientOptions::default()).unwrap()
        });
        let state = cx.new(|cx| {
            let mut state = AppState::with_config(
                Arc::new(ConnectionManager::new()),
                ConfigManager::with_config_dir(dir.path().into()),
            );
            state.connections = vec![saved.clone()];
            state.insert_active_connection(
                conn_id,
                ActiveConnection {
                    config: saved,
                    client,
                    databases: vec!["shop".into()],
                    collections: HashMap::from([(
                        "shop".to_string(),
                        vec!["users".into(), "orders".into(), "products".into()],
                    )]),
                    runtime_meta: Default::default(),
                },
            );
            state.conn.selected_connection = Some(conn_id);
            state.select_collection("shop".into(), "users".into(), cx);
            state
        });
        let users = state.read_with(cx, |state, _| state.current_session_key().unwrap());
        (state, users, dir)
    }

    fn go(
        state: &Entity<AppState>,
        cx: &mut TestAppContext,
        collection: &str,
        filter_raw: &str,
    ) -> SessionKey {
        let filter = (!filter_raw.is_empty()).then(|| doc! { "userId": 1 });
        state
            .update(cx, |state, cx| {
                state.navigate_to_collection(
                    "shop".into(),
                    collection.into(),
                    filter_raw.into(),
                    filter,
                    cx,
                )
            })
            .expect("navigation should land")
    }

    #[gpui_kit::test]
    fn navigating_stays_in_the_tab_and_keeps_the_view_it_left(cx: &mut TestAppContext) {
        let (state, users, _dir) = setup(cx);
        assert_eq!(users.collection, "users");

        let orders = go(&state, cx, "orders", "{ userId: 1 }");

        state.read_with(cx, |state, _| {
            assert_eq!(state.open_tabs().len(), 1, "navigation reuses the tab");
            assert_eq!(state.current_session_key().as_ref(), Some(&orders));
            assert_eq!(state.session_data(&orders).unwrap().filter_raw, "{ userId: 1 }");
            assert!(state.can_navigate_back());
            assert!(!state.can_navigate_forward());
            assert!(state.session(&users).is_some(), "the view we left is still live");
        });
    }

    #[gpui_kit::test]
    fn back_and_forward_walk_the_same_views(cx: &mut TestAppContext) {
        let (state, users, _dir) = setup(cx);
        let orders = go(&state, cx, "orders", "{ userId: 1 }");

        assert!(state.update(cx, |state, cx| state.navigate_back(cx)));
        state.read_with(cx, |state, _| {
            assert_eq!(state.current_session_key().as_ref(), Some(&users));
            assert!(state.can_navigate_forward());
            assert!(!state.can_navigate_back());
        });

        assert!(state.update(cx, |state, cx| state.navigate_forward(cx)));
        state.read_with(cx, |state, _| {
            // The same view, not a fresh one: its filter is still there without a reload.
            assert_eq!(state.current_session_key().as_ref(), Some(&orders));
            assert_eq!(state.session_data(&orders).unwrap().filter_raw, "{ userId: 1 }");
            assert!(state.can_navigate_back());
            assert!(!state.can_navigate_forward());
        });
    }

    #[gpui_kit::test]
    fn back_at_the_start_reports_nowhere_to_go(cx: &mut TestAppContext) {
        let (state, users, _dir) = setup(cx);
        assert!(!state.update(cx, |state, cx| state.navigate_back(cx)));
        assert!(!state.update(cx, |state, cx| state.navigate_forward(cx)));
        state.read_with(cx, |state, _| {
            assert_eq!(state.current_session_key().as_ref(), Some(&users));
        });
    }

    #[gpui_kit::test]
    fn a_new_jump_ends_the_forward_chain(cx: &mut TestAppContext) {
        let (state, _users, _dir) = setup(cx);
        let orders = go(&state, cx, "orders", "");
        assert!(state.update(cx, |state, cx| state.navigate_back(cx)));

        let products = go(&state, cx, "products", "");

        state.read_with(cx, |state, _| {
            assert!(!state.can_navigate_forward());
            assert_eq!(state.current_session_key().as_ref(), Some(&products));
            assert!(state.session(&orders).is_none(), "the abandoned view is dropped");
        });
    }

    #[gpui_kit::test]
    fn history_is_capped_and_drops_the_oldest_views(cx: &mut TestAppContext) {
        let (state, users, _dir) = setup(cx);
        let mut visited = Vec::new();
        for step in 0..MAX_TAB_HISTORY + 3 {
            let collection = if step % 2 == 0 { "orders" } else { "products" };
            visited.push(go(&state, cx, collection, ""));
        }

        state.read_with(cx, |state, _| {
            assert_eq!(state.navigation_trail().len(), MAX_TAB_HISTORY + 1);
            assert!(state.session(&users).is_none(), "the oldest view is evicted");
            assert!(state.session(visited.last().unwrap()).is_some());
        });
    }

    #[gpui_kit::test]
    fn a_breadcrumb_click_jumps_straight_back_through_the_trail(cx: &mut TestAppContext) {
        let (state, users, _dir) = setup(cx);
        go(&state, cx, "orders", "");
        let products = go(&state, cx, "products", "");

        state.read_with(cx, |state, _| {
            let trail = state.navigation_trail();
            assert_eq!(trail.len(), 3);
            assert_eq!(trail[0], users);
            assert_eq!(trail[2], products, "the trail ends at what is on screen");
        });

        // Clicking the first crumb goes back two steps at once.
        assert!(state.update(cx, |state, cx| state.navigate_to_trail_index(0, cx)));
        state.read_with(cx, |state, _| {
            assert_eq!(state.current_session_key().as_ref(), Some(&users));
            assert_eq!(state.navigation_trail().len(), 1);
            assert!(state.can_navigate_forward(), "the way back forward is kept");
        });

        // The crumb for the view already on screen does nothing.
        assert!(!state.update(cx, |state, cx| state.navigate_to_trail_index(0, cx)));
    }

    #[gpui_kit::test]
    fn a_tab_with_unsaved_edits_is_never_navigated_away_from(cx: &mut TestAppContext) {
        let (state, users, _dir) = setup(cx);
        state.update(cx, |state, cx| state.set_collection_dirty(users.clone(), true, cx));

        let orders = go(&state, cx, "orders", "");

        state.read_with(cx, |state, _| {
            assert_eq!(state.open_tabs().len(), 2, "the target opens in its own tab");
            assert_eq!(state.current_session_key().as_ref(), Some(&orders));
            assert!(state.session(&users).is_some(), "the edited view stays on its own tab");
            assert!(!state.can_navigate_back(), "a fresh tab has nowhere to go back to");
        });
    }

    #[gpui_kit::test]
    fn closing_a_tab_drops_every_view_it_held(cx: &mut TestAppContext) {
        let (state, users, _dir) = setup(cx);
        let orders = go(&state, cx, "orders", "");
        let products = go(&state, cx, "products", "");

        state.update(cx, |state, cx| state.close_tab(0, cx));

        state.read_with(cx, |state, _| {
            assert!(state.open_tabs().is_empty());
            for key in [&users, &orders, &products] {
                assert!(state.session(key).is_none(), "{} outlived its tab", key.namespace());
            }
        });
    }

    #[gpui_kit::test]
    fn the_same_collection_can_be_open_in_two_tabs(cx: &mut TestAppContext) {
        let (state, users, _dir) = setup(cx);

        let second = state
            .update(cx, |state, cx| {
                state.open_collection_in_new_tab(
                    "shop".into(),
                    "users".into(),
                    "{ active: true }".into(),
                    Some(doc! { "active": true }),
                    cx,
                )
            })
            .expect("a second view");

        state.read_with(cx, |state, _| {
            assert_eq!(state.open_tabs().len(), 2);
            assert_ne!(second, users, "each tab gets its own view");
            assert!(second.same_collection(&users));
            assert_eq!(state.session_data(&second).unwrap().filter_raw, "{ active: true }");
            assert_eq!(state.session_data(&users).unwrap().filter_raw, "");
        });
    }

    #[gpui_kit::test]
    fn opening_from_the_sidebar_reuses_whichever_tab_shows_the_collection(cx: &mut TestAppContext) {
        let (state, _users, _dir) = setup(cx);
        let orders = go(&state, cx, "orders", "");

        state.update(cx, |state, cx| {
            state.select_collection("shop".into(), "orders".into(), cx);
        });

        state.read_with(cx, |state, _| {
            assert_eq!(state.open_tabs().len(), 1, "no second tab for a collection on screen");
            assert_eq!(state.current_session_key().as_ref(), Some(&orders));
            assert!(state.can_navigate_back(), "history survives a sidebar re-open");
        });
    }
}
