//! Workspace persistence helpers for AppState.

use gpui::Context;

use crate::bson::parse_document_from_json;
use crate::state::{AppEvent, WindowState, WorkspaceTab, WorkspaceTabKind};
use uuid::Uuid;

use super::AppState;
use super::types::{ActiveTab, CollectionSubview, DatabaseKey, SessionKey, TabKey, View};

impl AppState {
    pub fn workspace_autoconnect_id(&self) -> Option<Uuid> {
        if self.workspace_restore_pending { self.workspace.last_connection_id } else { None }
    }

    pub fn set_workspace_expanded_nodes(&mut self, nodes: Vec<String>) {
        if self.workspace.expanded_nodes != nodes {
            self.workspace.expanded_nodes = nodes;
            self.save_workspace();
        }
    }

    pub fn set_workspace_window_bounds(&mut self, bounds: gpui::WindowBounds) {
        let window_state = WindowState::from_bounds(bounds);
        if self.workspace.window_state.as_ref() != Some(&window_state) {
            self.workspace.window_state = Some(window_state);
            self.save_workspace();
        }
    }

    pub fn update_workspace_from_state(&mut self) {
        if self.workspace_restore_pending {
            return;
        }
        let last_connection_id = self
            .conn
            .active
            .as_ref()
            .map(|conn| conn.config.id)
            .or(self.workspace.last_connection_id);
        self.workspace.last_connection_id = last_connection_id;

        let active_index = match self.tabs.active {
            ActiveTab::Index(index) if index < self.tabs.open.len() => Some(index),
            _ => None,
        };

        self.workspace.active_tab = active_index;
        self.workspace.open_tabs = self
            .tabs
            .open
            .iter()
            .map(|tab| match tab {
                TabKey::Collection(key) => {
                    let (filter_raw, sort_raw, projection_raw, subview, stats_open) = self
                        .session(key)
                        .map(|session| {
                            (
                                session.data.filter_raw.clone(),
                                session.data.sort_raw.clone(),
                                session.data.projection_raw.clone(),
                                session.view.subview,
                                matches!(session.view.subview, CollectionSubview::Stats),
                            )
                        })
                        .unwrap_or_else(|| {
                            (
                                String::new(),
                                String::new(),
                                String::new(),
                                CollectionSubview::Documents,
                                false,
                            )
                        });
                    WorkspaceTab {
                        database: key.database.clone(),
                        collection: key.collection.clone(),
                        kind: WorkspaceTabKind::Collection,
                        filter_raw,
                        sort_raw,
                        projection_raw,
                        stats_open,
                        subview,
                    }
                }
                TabKey::Database(key) => WorkspaceTab {
                    database: key.database.clone(),
                    collection: String::new(),
                    kind: WorkspaceTabKind::Database,
                    filter_raw: String::new(),
                    sort_raw: String::new(),
                    projection_raw: String::new(),
                    stats_open: false,
                    subview: CollectionSubview::Documents,
                },
            })
            .collect();

        if let Some(index) = active_index
            && let Some(tab) = self.tabs.open.get(index)
        {
            match tab {
                TabKey::Collection(key) => {
                    self.workspace.selected_database = Some(key.database.clone());
                    self.workspace.selected_collection = Some(key.collection.clone());
                }
                TabKey::Database(key) => {
                    self.workspace.selected_database = Some(key.database.clone());
                    self.workspace.selected_collection = None;
                }
            }
        } else {
            self.workspace.selected_database = self.conn.selected_database.clone();
            self.workspace.selected_collection = self.conn.selected_collection.clone();
        }

        self.save_workspace();
    }

    pub fn restore_workspace_after_connect(&mut self, cx: &mut Context<Self>) {
        if !self.workspace_restore_pending {
            return;
        }
        let Some(active) = self.conn.active.as_ref() else {
            return;
        };
        if self.workspace.last_connection_id != Some(active.config.id) {
            self.workspace_restore_pending = false;
            return;
        }

        let databases = active.databases.clone();
        let workspace_tabs = self.workspace.open_tabs.clone();
        let mut restored_tabs: Vec<TabKey> = Vec::new();
        let mut restored_meta: Vec<(SessionKey, WorkspaceTab)> = Vec::new();
        for tab in &workspace_tabs {
            match tab.kind {
                WorkspaceTabKind::Collection => {
                    if tab.collection.is_empty() {
                        continue;
                    }
                    if databases.contains(&tab.database) {
                        let key = SessionKey::new(
                            active.config.id,
                            tab.database.clone(),
                            tab.collection.clone(),
                        );
                        restored_tabs.push(TabKey::Collection(key.clone()));
                        restored_meta.push((key, tab.clone()));
                    }
                }
                WorkspaceTabKind::Database => {
                    if databases.contains(&tab.database) {
                        let key = DatabaseKey::new(active.config.id, tab.database.clone());
                        restored_tabs.push(TabKey::Database(key));
                    }
                }
            }
        }

        self.tabs.open = restored_tabs.clone();
        self.tabs.preview = None;
        self.tabs.dirty.clear();

        let active_tab =
            self.workspace.active_tab.and_then(|idx| workspace_tabs.get(idx)).and_then(|tab| {
                restored_tabs.iter().position(|key| match (tab.kind.clone(), key) {
                    (WorkspaceTabKind::Collection, TabKey::Collection(session)) => {
                        session.database == tab.database && session.collection == tab.collection
                    }
                    (WorkspaceTabKind::Database, TabKey::Database(database)) => {
                        database.database == tab.database
                    }
                    _ => false,
                })
            });

        for (key, tab) in restored_meta.iter() {
            let session = self.ensure_session(key.clone());
            let restored_subview = if tab.subview == CollectionSubview::Documents && tab.stats_open
            {
                CollectionSubview::Stats
            } else {
                tab.subview
            };
            session.view.subview = restored_subview;
            session.view.stats_open = matches!(restored_subview, CollectionSubview::Stats);
            restore_doc_option(&tab.filter_raw, |raw, doc| {
                session.data.filter_raw = raw;
                session.data.filter = doc;
            });
            restore_doc_option(&tab.sort_raw, |raw, doc| {
                session.data.sort_raw = raw;
                session.data.sort = doc;
            });
            restore_doc_option(&tab.projection_raw, |raw, doc| {
                session.data.projection_raw = raw;
                session.data.projection = doc;
            });
        }

        if let Some(active_index) = active_tab {
            self.tabs.active = ActiveTab::Index(active_index);
            if let Some(tab) = self.tabs.open.get(active_index).cloned() {
                match tab {
                    TabKey::Collection(key) => {
                        self.conn.selected_database = Some(key.database.clone());
                        self.conn.selected_collection = Some(key.collection.clone());
                        self.current_view = View::Documents;
                    }
                    TabKey::Database(key) => {
                        self.conn.selected_database = Some(key.database.clone());
                        self.conn.selected_collection = None;
                        self.current_view = View::Database;
                    }
                }
            }
        } else if let Some(selected_db) = self.workspace.selected_database.clone() {
            if databases.contains(&selected_db) {
                self.conn.selected_database = Some(selected_db);
                self.conn.selected_collection = self.workspace.selected_collection.clone();
                self.current_view = if self.conn.selected_collection.is_some() {
                    View::Documents
                } else {
                    View::Collections
                };
            } else {
                self.conn.selected_database = None;
                self.conn.selected_collection = None;
                self.current_view = View::Databases;
            }
        } else {
            self.conn.selected_database = None;
            self.conn.selected_collection = None;
            self.current_view = View::Databases;
        }

        self.workspace_restore_pending = false;
        self.update_workspace_from_state();
        cx.emit(AppEvent::ViewChanged);
        cx.notify();
    }

    fn save_workspace(&self) {
        if let Err(err) = self.config.save_workspace(&self.workspace) {
            log::error!("Failed to save workspace: {err}");
        }
    }
}

fn restore_doc_option(raw: &str, mut apply: impl FnMut(String, Option<mongodb::bson::Document>)) {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        apply(String::new(), None);
        return;
    }

    match parse_document_from_json(trimmed) {
        Ok(doc) => apply(raw.to_string(), Some(doc)),
        Err(_) => apply(String::new(), None),
    }
}
