use crate::bson::parse_document_from_json;
use crate::state::{CollectionSubview, WorkspaceTab, WorkspaceTabKind};
use uuid::Uuid;

use super::super::AppState;
use super::super::types::{ActiveTab, DatabaseKey, SessionKey, TabKey};

impl AppState {
    pub(in crate::state::app_state) fn update_workspace_tabs(&mut self) {
        let selected_connection = self.conn.selected_connection;
        let active_index = match self.tabs.active {
            ActiveTab::Index(index) if index < self.tabs.open.len() => Some(index),
            _ => None,
        };

        self.workspace.active_tab = active_index
            .and_then(|index| self.tabs.open.get(index))
            .filter(|tab| match (selected_connection, tab) {
                (Some(conn_id), TabKey::Collection(key)) => key.connection_id == conn_id,
                (Some(conn_id), TabKey::Database(key)) => key.connection_id == conn_id,
                _ => false,
            })
            .and_then(|tab| {
                self.tabs.open.iter().position(|candidate| std::ptr::eq(candidate, tab))
            });
        self.workspace.open_tabs = self
            .tabs
            .open
            .iter()
            .filter(|tab| match (selected_connection, tab) {
                (Some(conn_id), TabKey::Collection(key)) => key.connection_id == conn_id,
                (Some(conn_id), TabKey::Database(key)) => key.connection_id == conn_id,
                _ => false,
            })
            .map(|tab| self.build_workspace_tab(tab))
            .collect();

        self.update_workspace_selection();
    }

    pub(in crate::state::app_state) fn restore_tabs_from_workspace(
        &mut self,
        connection_id: Uuid,
        databases: &[String],
    ) -> Option<usize> {
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
                            connection_id,
                            tab.database.clone(),
                            tab.collection.clone(),
                        );
                        restored_tabs.push(TabKey::Collection(key.clone()));
                        restored_meta.push((key, tab.clone()));
                    }
                }
                WorkspaceTabKind::Database => {
                    if databases.contains(&tab.database) {
                        let key = DatabaseKey::new(connection_id, tab.database.clone());
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

        active_tab
    }

    fn build_workspace_tab(&self, tab: &TabKey) -> WorkspaceTab {
        match tab {
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
        }
    }

    pub(in crate::state::app_state) fn update_workspace_selection(&mut self) {
        if let Some(index) = self.workspace.active_tab
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
