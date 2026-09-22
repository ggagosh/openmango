use gpui_kit::Context;
use uuid::Uuid;

use crate::state::compare::{CompareConfig, CompareEndpoint, CompareTabKey, CompareTabState};
use crate::state::{ActiveTab, AppEvent, AppState, TabKey, View};

impl AppState {
    pub(crate) fn compare_restore_dir(&self) -> std::path::PathBuf {
        self.config.compare_restore_dir()
    }

    pub fn compare_sync_disabled_reason(&self, id: Uuid, undo: bool) -> Option<String> {
        let tab = self.compare_tab(id)?;
        if tab.running || tab.sync.running {
            return Some("Wait for the current operation to finish".into());
        }
        let Some(target) = tab.sync.target else {
            return Some("Choose which collection to change".into());
        };
        if !undo
            && (tab.sync.completed
                || tab.compared.as_ref() != Some(&tab.config)
                || tab.error.is_some()
                || tab.summary.is_none())
        {
            return Some("Compare again before syncing".into());
        }
        let index = if target == crate::connection::ops::compare::Side::Left { 0 } else { 1 };
        let config = tab.results_config();
        for (i, side) in config.sides.iter().enumerate() {
            if undo && i != index {
                continue;
            }
            let Some(connection_id) = side.connection_id else {
                return Some("Connection is missing".into());
            };
            if !self.is_connected(connection_id) || self.connection_needs_reconnect(connection_id) {
                return Some("Reconnect the collection before writing".into());
            }
            if tab.connection_identities[i].as_ref().is_none_or(|snapshot| {
                self.connection_by_id(connection_id)
                    .is_none_or(|connection| !snapshot.matches(connection))
            }) {
                return Some("Connection settings changed since this comparison. Restore those settings to undo, or compare again before syncing.".into());
            }
        }
        self.compare_sync_target_disabled_reason(id, index)
    }

    pub fn compare_sync_target_disabled_reason(&self, id: Uuid, index: usize) -> Option<String> {
        let tab = self.compare_tab(id)?;
        let config = tab.results_config();
        let endpoint = &config.sides[index];
        let key = crate::state::SessionKey::new(
            endpoint.connection_id?,
            &endpoint.database,
            &endpoint.collection,
        );
        if let Some(reason) = self.session_read_only_reason(&key) {
            return Some(reason);
        }
        if let Some(metadata) = &tab.metadata[index]
            && metadata.endpoint == *endpoint
        {
            if metadata.timeseries {
                return Some("Time-series collections cannot be sync targets".into());
            }
            if metadata.supports_sync == Some(false) {
                return Some("Sync and undo require MongoDB 8.0+ on the target. Older servers support comparison only.".into());
            }
        }
        None
    }
    pub(super) fn restore_compare_configs(&mut self) {
        for (index, saved) in self.workspace.open_tabs.iter().enumerate() {
            if saved.kind != crate::state::WorkspaceTabKind::Compare {
                continue;
            }
            let config = saved.compare.clone().unwrap_or_default();
            let id = Uuid::new_v4();
            self.compare_restored.insert(index, id);
            self.tabs.open.push(TabKey::Compare(CompareTabKey {
                id,
                connection_id: config.sides[0].connection_id,
            }));
            self.compare_tabs.insert(id, CompareTabState::new(config));
            if self.workspace.active_tab == Some(index)
                || matches!(self.tabs.active, ActiveTab::None)
            {
                self.tabs.active = ActiveTab::Index(self.tabs.open.len() - 1);
                self.current_view = View::Compare;
            }
        }
    }

    pub fn compare_tab(&self, id: Uuid) -> Option<&CompareTabState> {
        self.compare_tabs.get(&id)
    }
    pub fn compare_tab_mut(&mut self, id: Uuid) -> Option<&mut CompareTabState> {
        self.compare_tabs.get_mut(&id)
    }

    pub fn active_compare_tab_id(&self) -> Option<Uuid> {
        let ActiveTab::Index(index) = self.active_tab() else {
            return None;
        };
        match self.open_tabs().get(index) {
            Some(TabKey::Compare(key)) => Some(key.id),
            _ => None,
        }
    }

    pub fn open_compare_tab(&mut self, prefill: Option<CompareEndpoint>, cx: &mut Context<Self>) {
        let left = prefill.unwrap_or_else(|| CompareEndpoint {
            connection_id: self.selected_connection_id(),
            database: self.selected_database_name().unwrap_or_default(),
            collection: self.selected_collection().map(str::to_owned).unwrap_or_default(),
        });
        let id = Uuid::new_v4();
        self.tabs
            .open
            .push(TabKey::Compare(CompareTabKey { id, connection_id: left.connection_id }));
        self.compare_tabs.insert(
            id,
            CompareTabState::new(CompareConfig {
                sides: [left, CompareEndpoint::default()],
                ..Default::default()
            }),
        );
        self.tabs.active = ActiveTab::Index(self.tabs.open.len() - 1);
        self.current_view = View::Compare;
        self.update_workspace_from_state_debounced();
        cx.emit(AppEvent::ViewChanged);
        cx.notify();
    }

    pub fn update_compare_config(
        &mut self,
        id: Uuid,
        edit: impl FnOnce(&mut CompareConfig),
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.compare_tabs.get_mut(&id) else {
            return;
        };
        if tab.sync.running {
            return;
        }
        edit(&mut tab.config);
        if tab.config.fields.is_empty() {
            tab.config.fields.push("_id".into());
        }
        let connection_id = tab.config.sides[0].connection_id;
        if let Some(TabKey::Compare(key)) = self
            .tabs
            .open
            .iter_mut()
            .find(|key| matches!(key, TabKey::Compare(key) if key.id == id))
        {
            key.connection_id = connection_id;
        }
        self.update_workspace_from_state_debounced();
        cx.notify();
    }

    pub fn compare_disabled_reason(&self, config: &CompareConfig) -> Option<String> {
        if let Err(error) = (crate::connection::ops::compare::CompareOptions {
            fields: config.fields.clone(),
            ..Default::default()
        })
        .validate()
        {
            return Some(error.to_string());
        }
        for (side, endpoint) in ["Left", "Right"].into_iter().zip(&config.sides) {
            if !endpoint.complete() {
                return Some(format!(
                    "Choose a connection, database, and collection on the {}",
                    side.to_lowercase()
                ));
            }
            let id = endpoint.connection_id?;
            if !self.is_connected(id) {
                return Some(format!(
                    "{side} connection is closed. Reconnect to compare or fetch documents."
                ));
            }
            let session =
                crate::state::SessionKey::new(id, &endpoint.database, &endpoint.collection);
            if matches!(
                self.collection_detail(&session),
                Some(crate::models::CollectionDetail::Timeseries)
            ) {
                return Some(format!(
                    "{side} is a time-series collection, which cannot be compared yet"
                ));
            }
        }
        if config.sides[0] == config.sides[1] {
            return Some("Choose two different collections".into());
        }
        if !config.filter.trim().is_empty()
            && let Err(error) = crate::bson::parse_document_from_json(&config.filter)
        {
            return Some(format!("Filter: {error}"));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ConfigManager, WorkspaceState, WorkspaceTab, WorkspaceTabKind};
    use std::sync::Arc;

    fn saved_state(connection: Option<Uuid>) -> (tempfile::TempDir, AppState) {
        let directory = tempfile::tempdir().unwrap();
        let config = ConfigManager::with_config_dir(directory.path().into());
        config
            .save_workspace(&WorkspaceState {
                last_connection_id: connection,
                active_tab: Some(0),
                open_tabs: vec![WorkspaceTab {
                    kind: WorkspaceTabKind::Compare,
                    compare: Some(CompareConfig {
                        fields: vec!["sku".into()],
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
        (
            directory,
            AppState::with_config(Arc::new(crate::connection::ConnectionManager::new()), config),
        )
    }

    #[test]
    fn compare_restores_offline_and_connected_restore_reuses_live_state() {
        let (_directory, state) = saved_state(None);
        let id = state.active_compare_tab_id().unwrap();
        assert_eq!(state.current_view, View::Compare);
        assert_eq!(state.compare_tab(id).unwrap().config.fields, ["sku"]);
        assert!(state.compare_tab(id).unwrap().rows.is_empty());
        let connection = Uuid::new_v4();
        let (_directory, mut state) = saved_state(Some(connection));
        let id = state.active_compare_tab_id().unwrap();
        state.compare_tab_mut(id).unwrap().config.filter = "{active:true}".into();
        state.restore_tabs_from_workspace(connection, &[]);
        assert_eq!(state.open_tabs().len(), 1);
        assert!(matches!(&state.open_tabs()[0], TabKey::Compare(key) if key.id == id));
        assert_eq!(state.compare_tab(id).unwrap().config.filter, "{active:true}");
    }

    #[test]
    fn closing_an_eager_restored_compare_does_not_resurrect_it_after_connect() {
        let connection = Uuid::new_v4();
        let (_directory, mut state) = saved_state(Some(connection));
        let id = state.active_compare_tab_id().unwrap();
        state.compare_tabs.remove(&id);
        state.tabs.open.clear();
        state.restore_tabs_from_workspace(connection, &[]);
        assert!(state.open_tabs().is_empty());
    }
}
