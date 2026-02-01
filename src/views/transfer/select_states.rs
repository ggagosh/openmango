//! Select state initialization and management for transfer view.

use gpui::*;
use gpui_component::input::{InputEvent, InputState};
use gpui_component::select::{SearchableVec, SelectEvent, SelectItem, SelectState};
use uuid::Uuid;

use crate::state::AppCommands;

use super::TransferView;

/// Custom SelectItem for connections (stores UUID + display name).
#[derive(Clone, Debug)]
pub(super) struct ConnectionItem {
    pub id: Uuid,
    pub name: SharedString,
}

impl SelectItem for ConnectionItem {
    type Value = Uuid;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(&query.to_lowercase())
    }
}

impl TransferView {
    /// Initialize select states on first render (when window is available).
    pub(super) fn ensure_select_states(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.source_conn_state.is_some() {
            return; // Already initialized
        }

        let state = self.state.clone();

        // Create select states
        let source_conn_state = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None, window, cx).searchable(true)
        });
        let source_db_state = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None, window, cx).searchable(true)
        });
        let source_coll_state = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None, window, cx).searchable(true)
        });
        let dest_conn_state = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None, window, cx).searchable(true)
        });

        // Create exclude collections select state (searchable multi-select behavior)
        let exclude_coll_state = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None, window, cx).searchable(true)
        });

        // Subscribe to select events
        let state_clone = state.clone();
        let sub1 = cx.subscribe_in(
            &source_conn_state,
            window,
            move |view, _select_state, event, window, cx| {
                if let SelectEvent::Confirm(Some(conn_id)) = event {
                    let tab_id = state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            tab.source_connection_id = Some(*conn_id);
                            tab.source_database.clear();
                            tab.source_collection.clear();
                            cx.notify();
                            return Some(tab_id);
                        }
                        None
                    });
                    // Clear dependent selects
                    if let Some(ref db_state) = view.source_db_state {
                        db_state.update(cx, |s, cx| {
                            s.set_selected_index(None, window, cx);
                        });
                    }
                    if let Some(ref coll_state) = view.source_coll_state {
                        coll_state.update(cx, |s, cx| {
                            s.set_selected_index(None, window, cx);
                        });
                    }
                    if tab_id.is_some() {
                        cx.notify();
                    }
                }
            },
        );

        let state_clone = state.clone();
        let sub2 = cx.subscribe_in(
            &source_db_state,
            window,
            move |view,
                  _select_state,
                  event: &SelectEvent<SearchableVec<SharedString>>,
                  window,
                  cx| {
                if let SelectEvent::Confirm(Some(db_name)) = event {
                    let db_str = db_name.to_string();
                    let conn_id = state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            tab.source_database = db_str.clone();
                            tab.source_collection.clear();
                            cx.notify();
                            return tab.source_connection_id;
                        }
                        None
                    });
                    // Clear collection select
                    if let Some(ref coll_state) = view.source_coll_state {
                        coll_state.update(cx, |s, cx| {
                            s.set_selected_index(None, window, cx);
                        });
                    }
                    // Load collections for the selected database
                    if let Some(conn_id) = conn_id {
                        AppCommands::load_collections(state_clone.clone(), conn_id, db_str, cx);
                    }
                    cx.notify();
                }
            },
        );

        let state_clone = state.clone();
        let sub3 = cx.subscribe_in(
            &source_coll_state,
            window,
            move |_view,
                  _select_state,
                  event: &SelectEvent<SearchableVec<SharedString>>,
                  _window,
                  cx| {
                if let SelectEvent::Confirm(Some(coll_name)) = event {
                    let coll_str = coll_name.to_string();
                    let tab_id = state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            tab.source_collection = coll_str.clone();
                            cx.notify();
                            return Some(tab_id);
                        }
                        None
                    });
                    if let Some(tab_id) = tab_id {
                        AppCommands::load_transfer_preview(state_clone.clone(), tab_id, cx);
                    }
                }
            },
        );

        let state_clone = state.clone();
        let sub4 = cx.subscribe_in(
            &dest_conn_state,
            window,
            move |_view, _select_state, event, _window, cx| {
                if let SelectEvent::Confirm(Some(conn_id)) = event {
                    state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            tab.destination_connection_id = Some(*conn_id);
                            cx.notify();
                        }
                    });
                }
            },
        );

        // Subscribe to exclude collections select - append to exclude list (multi-select behavior)
        let state_clone = state.clone();
        let exclude_coll_state_clone = exclude_coll_state.clone();
        let sub6 = cx.subscribe_in(
            &exclude_coll_state,
            window,
            move |_view,
                  _select_state,
                  event: &SelectEvent<SearchableVec<SharedString>>,
                  window,
                  cx| {
                if let SelectEvent::Confirm(Some(coll_name)) = event {
                    let coll_str = coll_name.to_string();
                    state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            // Only add if not already excluded
                            if !tab.exclude_collections.contains(&coll_str) {
                                tab.exclude_collections.push(coll_str);
                            }
                            cx.notify();
                        }
                    });
                    // Clear selection after pick (multi-select behavior)
                    exclude_coll_state_clone.update(cx, |s, cx| {
                        s.set_selected_index(None, window, cx);
                    });
                }
            },
        );

        // Create export path input state
        let current_file_path = {
            let state_ref = state.read(cx);
            state_ref
                .active_transfer_tab_id()
                .and_then(|id| state_ref.transfer_tab(id).map(|tab| tab.file_path.clone()))
                .unwrap_or_default()
        };

        let export_path_input_state = cx.new(|cx| {
            let mut input_state =
                InputState::new(window, cx).placeholder("Select folder or enter path...");
            input_state.set_value(current_file_path, window, cx);
            input_state
        });

        // Subscribe to export path input changes
        let state_clone = state.clone();
        let sub5 = cx.subscribe_in(
            &export_path_input_state,
            window,
            move |_view, input_state, event, _window, cx| {
                if let InputEvent::Change = event {
                    let new_path = input_state.read(cx).value().to_string();
                    state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            tab.file_path = new_path;
                            cx.notify();
                        }
                    });
                }
            },
        );

        self._select_subscriptions = vec![sub1, sub2, sub3, sub4, sub5, sub6];
        self.source_conn_state = Some(source_conn_state);
        self.source_db_state = Some(source_db_state);
        self.source_coll_state = Some(source_coll_state);
        self.dest_conn_state = Some(dest_conn_state);
        self.exclude_coll_state = Some(exclude_coll_state);
        self.export_path_input_state = Some(export_path_input_state);
    }
}
