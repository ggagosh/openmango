use crate::bson::parse_document_from_json;
use crate::state::app_state::StageDocCounts;
use crate::state::{
    CollectionSubview, TransferTabKey, TransferTabState, WorkspaceTab, WorkspaceTabKind,
};
use uuid::Uuid;

use super::super::AppState;
use super::super::types::{ActiveTab, DatabaseKey, ForgeTabKey, ForgeTabState, SessionKey, TabKey};

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
                (Some(conn_id), TabKey::Transfer(key)) => key.connection_id == Some(conn_id),
                (Some(conn_id), TabKey::Forge(key)) => key.connection_id == conn_id,
                (_, TabKey::Settings | TabKey::Changelog) => false,
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
                (Some(conn_id), TabKey::Transfer(key)) => key.connection_id == Some(conn_id),
                (Some(conn_id), TabKey::Forge(key)) => key.connection_id == conn_id,
                (_, TabKey::Settings | TabKey::Changelog) => false,
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
                WorkspaceTabKind::Transfer => {
                    let mut transfer_state = tab.transfer.clone().unwrap_or_default();
                    if transfer_state.config.source_connection_id.is_none() {
                        transfer_state.config.source_connection_id = Some(connection_id);
                    }
                    if transfer_state.config.source_database.is_empty() && !tab.database.is_empty()
                    {
                        transfer_state.config.source_database = tab.database.clone();
                    }
                    if transfer_state.config.source_collection.is_empty()
                        && !tab.collection.is_empty()
                    {
                        transfer_state.config.source_collection = tab.collection.clone();
                    }
                    let id = Uuid::new_v4();
                    let key = TransferTabKey {
                        id,
                        connection_id: transfer_state.config.source_connection_id,
                    };
                    self.transfer_tabs.insert(id, transfer_state);
                    restored_tabs.push(TabKey::Transfer(key));
                }
                WorkspaceTabKind::Forge => {
                    if databases.contains(&tab.database) {
                        let id = Uuid::new_v4();
                        let key = ForgeTabKey { id, connection_id, database: tab.database.clone() };
                        let state = ForgeTabState {
                            content: tab.forge_content.clone(),
                            is_running: false,
                            error: None,
                            pending_cursor: None,
                        };
                        self.forge_tabs.insert(id, state);
                        restored_tabs.push(TabKey::Forge(key));
                    }
                }
            }
        }

        self.tabs.open = restored_tabs.clone();
        self.tabs.preview = None;
        self.tabs.dirty.clear();

        let active_tab =
            self.workspace.active_tab.and_then(|idx| workspace_tabs.get(idx)).and_then(|tab| {
                restored_tabs.iter().position(|key| match (tab.kind, key) {
                    (WorkspaceTabKind::Collection, TabKey::Collection(session)) => {
                        session.database == tab.database && session.collection == tab.collection
                    }
                    (WorkspaceTabKind::Database, TabKey::Database(database)) => {
                        database.database == tab.database
                    }
                    (WorkspaceTabKind::Transfer, TabKey::Transfer(transfer)) => {
                        let Some(state) = self.transfer_tabs.get(&transfer.id) else {
                            return false;
                        };
                        state.config.source_database == tab.database
                    }
                    (WorkspaceTabKind::Forge, TabKey::Forge(forge)) => {
                        forge.database == tab.database
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
            session.data.aggregation.stages = tab.aggregation_pipeline.clone();
            session.data.aggregation.stage_doc_counts =
                vec![StageDocCounts::default(); session.data.aggregation.stages.len()];
            session.data.aggregation.results = None;
            session.data.aggregation.results_page = 0;
            session.data.aggregation.last_run_time_ms = None;
            session.data.aggregation.error = None;
            session.data.aggregation.request_id = 0;
            session.data.aggregation.loading = false;
            if session.data.aggregation.selected_stage.is_none()
                && !session.data.aggregation.stages.is_empty()
            {
                session.data.aggregation.selected_stage = Some(0);
            }
        }

        active_tab
    }

    fn build_workspace_tab(&self, tab: &TabKey) -> WorkspaceTab {
        match tab {
            TabKey::Collection(key) => {
                let (
                    filter_raw,
                    sort_raw,
                    projection_raw,
                    aggregation_pipeline,
                    subview,
                    stats_open,
                ) = self
                    .session(key)
                    .map(|session| {
                        (
                            session.data.filter_raw.clone(),
                            session.data.sort_raw.clone(),
                            session.data.projection_raw.clone(),
                            session.data.aggregation.stages.clone(),
                            session.view.subview,
                            matches!(session.view.subview, CollectionSubview::Stats),
                        )
                    })
                    .unwrap_or_else(|| {
                        (
                            String::new(),
                            String::new(),
                            String::new(),
                            Vec::new(),
                            CollectionSubview::Documents,
                            false,
                        )
                    });
                WorkspaceTab {
                    database: key.database.clone(),
                    collection: key.collection.clone(),
                    kind: WorkspaceTabKind::Collection,
                    transfer: None,
                    filter_raw,
                    sort_raw,
                    projection_raw,
                    aggregation_pipeline,
                    stats_open,
                    subview,
                    forge_content: String::new(),
                }
            }
            TabKey::Database(key) => WorkspaceTab {
                database: key.database.clone(),
                collection: String::new(),
                kind: WorkspaceTabKind::Database,
                transfer: None,
                filter_raw: String::new(),
                sort_raw: String::new(),
                projection_raw: String::new(),
                aggregation_pipeline: Vec::new(),
                stats_open: false,
                subview: CollectionSubview::Documents,
                forge_content: String::new(),
            },
            TabKey::Transfer(key) => {
                let transfer = self.transfer_tabs.get(&key.id).cloned().unwrap_or_default();
                WorkspaceTab {
                    database: transfer.config.source_database.clone(),
                    collection: transfer.config.source_collection.clone(),
                    kind: WorkspaceTabKind::Transfer,
                    transfer: Some(transfer),
                    filter_raw: String::new(),
                    sort_raw: String::new(),
                    projection_raw: String::new(),
                    aggregation_pipeline: Vec::new(),
                    stats_open: false,
                    subview: CollectionSubview::Documents,
                    forge_content: String::new(),
                }
            }
            TabKey::Forge(key) => {
                let content = self
                    .forge_tabs
                    .get(&key.id)
                    .map(|state| state.content.clone())
                    .unwrap_or_default();
                WorkspaceTab {
                    database: key.database.clone(),
                    collection: String::new(),
                    kind: WorkspaceTabKind::Forge,
                    transfer: None,
                    filter_raw: String::new(),
                    sort_raw: String::new(),
                    projection_raw: String::new(),
                    aggregation_pipeline: Vec::new(),
                    stats_open: false,
                    subview: CollectionSubview::Documents,
                    forge_content: content,
                }
            }
            TabKey::Settings | TabKey::Changelog => {
                // Settings/Changelog tabs are not persisted in workspace
                WorkspaceTab {
                    database: String::new(),
                    collection: String::new(),
                    kind: WorkspaceTabKind::Database, // Placeholder, won't be saved
                    transfer: None,
                    filter_raw: String::new(),
                    sort_raw: String::new(),
                    projection_raw: String::new(),
                    aggregation_pipeline: Vec::new(),
                    stats_open: false,
                    subview: CollectionSubview::Documents,
                    forge_content: String::new(),
                }
            }
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
                TabKey::Transfer(key) => {
                    if let Some(transfer) = self.transfer_tabs.get(&key.id) {
                        if !transfer.config.source_database.is_empty() {
                            self.workspace.selected_database =
                                Some(transfer.config.source_database.clone());
                        }
                        if !transfer.config.source_collection.is_empty() {
                            self.workspace.selected_collection =
                                Some(transfer.config.source_collection.clone());
                        }
                    }
                }
                TabKey::Forge(key) => {
                    self.workspace.selected_database = Some(key.database.clone());
                    self.workspace.selected_collection = None;
                }
                TabKey::Settings | TabKey::Changelog => {
                    // Settings/Changelog tabs don't affect selection
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
        Err(e) => {
            log::warn!("Invalid filter JSON, resetting to empty: {e}");
            apply(String::new(), None);
        }
    }
}
