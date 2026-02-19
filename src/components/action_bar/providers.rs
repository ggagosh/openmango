use gpui::SharedString;

use crate::state::AppState;
use crate::state::TabKey;
use crate::state::app_state::updater::UpdateStatus;
use crate::state::settings::AppTheme;

use super::types::{ActionCategory, ActionItem};

/// Navigation: connections, databases, collections from active connections.
pub fn navigation_actions(state: &AppState) -> Vec<ActionItem> {
    let mut actions = Vec::new();
    let active = state.active_connections_snapshot();

    for (conn_id, conn) in &active {
        let conn_name = &conn.config.name;

        // Connection-level entry
        actions.push(ActionItem {
            id: SharedString::from(format!("nav:conn:{}", conn_id)),
            label: SharedString::from(conn_name.clone()),
            detail: Some(SharedString::from("Connection")),
            category: ActionCategory::Navigation,
            available: true,
            ..Default::default()
        });

        // Databases
        for db in &conn.databases {
            actions.push(ActionItem {
                id: SharedString::from(format!("nav:db:{}:{}", conn_id, db)),
                label: SharedString::from(db.clone()),
                detail: Some(SharedString::from(conn_name.clone())),
                category: ActionCategory::Navigation,
                available: true,
                priority: 10,
                ..Default::default()
            });

            // Collections within this database
            if let Some(collections) = conn.collections.get(db) {
                for col in collections {
                    actions.push(ActionItem {
                        id: SharedString::from(format!("nav:col:{}:{}:{}", conn_id, db, col)),
                        label: SharedString::from(col.clone()),
                        detail: Some(SharedString::from(format!("{} / {}", conn_name, db))),
                        category: ActionCategory::Navigation,
                        available: true,
                        priority: 20,
                        ..Default::default()
                    });
                }
            }
        }
    }

    actions
}

/// Tabs: currently open tabs.
pub fn tab_actions(state: &AppState) -> Vec<ActionItem> {
    let mut actions = Vec::new();

    for (index, tab) in state.open_tabs().iter().enumerate() {
        let (label, detail) = match tab {
            TabKey::Collection(key) => {
                let conn_name = state
                    .connection_name(key.connection_id)
                    .unwrap_or_else(|| "Connection".to_string());
                (key.collection.clone(), format!("{} / {}", conn_name, key.database))
            }
            TabKey::Database(key) => {
                let conn_name = state
                    .connection_name(key.connection_id)
                    .unwrap_or_else(|| "Connection".to_string());
                (key.database.clone(), conn_name)
            }
            TabKey::JsonEditor(key) => {
                let detail = state
                    .json_editor_tab(key.id)
                    .map(|tab| {
                        format!("{}/{}", tab.session_key.database, tab.session_key.collection)
                    })
                    .unwrap_or_else(|| "JSON editor".to_string());
                (state.json_editor_tab_label(key.id), detail)
            }
            TabKey::Transfer(key) => {
                let conn_name = key
                    .connection_id
                    .and_then(|id| state.connection_name(id))
                    .unwrap_or_else(|| "Connection".to_string());
                (state.transfer_tab_label(key.id), conn_name)
            }
            TabKey::Forge(key) => {
                let conn_name = state
                    .connection_name(key.connection_id)
                    .unwrap_or_else(|| "Connection".to_string());
                (state.forge_tab_label(key.id), format!("{} / {}", conn_name, key.database))
            }
            TabKey::Settings => ("Settings".to_string(), "Application settings".to_string()),
            TabKey::Changelog => ("What's New".to_string(), "Changelog".to_string()),
        };

        actions.push(ActionItem {
            id: SharedString::from(format!("tab:{}", index)),
            label: SharedString::from(label),
            detail: Some(SharedString::from(detail)),
            category: ActionCategory::Tab,
            available: true,
            priority: index as i32,
            ..Default::default()
        });
    }

    // Preview tab
    if let Some(preview) = state.preview_tab() {
        let conn_name = state
            .connection_name(preview.connection_id)
            .unwrap_or_else(|| "Connection".to_string());
        actions.push(ActionItem {
            id: SharedString::from("tab:preview"),
            label: SharedString::from(format!("{} (preview)", preview.collection)),
            detail: Some(SharedString::from(format!("{} / {}", conn_name, preview.database))),
            category: ActionCategory::Tab,
            available: true,
            priority: actions.len() as i32,
            ..Default::default()
        });
    }

    actions
}

/// Commands: create, delete, refresh, disconnect, etc.
pub fn command_actions(state: &AppState) -> Vec<ActionItem> {
    let has_connection = state.has_active_connections();
    let has_selected = state.selected_connection_id().is_some();
    let is_connected = has_selected
        && state.selected_connection_id().map(|id| state.is_connected(id)).unwrap_or(false);

    vec![
        ActionItem {
            id: SharedString::from("cmd:new-connection"),
            label: SharedString::from("New Connection"),
            category: ActionCategory::Command,
            available: true,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:create-database"),
            label: SharedString::from("Create Database"),
            category: ActionCategory::Command,
            shortcut: Some(SharedString::from("Cmd+Shift+N")),
            available: is_connected,
            priority: 10,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:create-collection"),
            label: SharedString::from("Create Collection"),
            category: ActionCategory::Command,
            shortcut: Some(SharedString::from("Cmd+N")),
            available: is_connected && state.selected_database().is_some(),
            priority: 11,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:refresh"),
            label: SharedString::from("Refresh"),
            category: ActionCategory::Command,
            shortcut: Some(SharedString::from("Cmd+R")),
            available: has_connection,
            priority: 20,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:disconnect"),
            label: SharedString::from("Disconnect"),
            category: ActionCategory::Command,
            available: has_connection,
            priority: 30,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:settings"),
            label: SharedString::from("Settings"),
            detail: Some(SharedString::from("Application settings")),
            category: ActionCategory::Command,
            shortcut: Some(SharedString::from("Cmd+,")),
            available: true,
            priority: 100,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:whats-new"),
            label: SharedString::from("What's New"),
            detail: Some(SharedString::from("View changelog")),
            category: ActionCategory::Command,
            available: true,
            priority: 105,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:check-updates"),
            label: SharedString::from("Check for Updates"),
            category: ActionCategory::Command,
            available: true,
            priority: 110,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:download-update"),
            label: SharedString::from("Download Update"),
            detail: match &state.update_status {
                UpdateStatus::Available { version, .. } => {
                    Some(SharedString::from(format!("v{version}")))
                }
                _ => None,
            },
            category: ActionCategory::Command,
            available: matches!(state.update_status, UpdateStatus::Available { .. }),
            priority: -10,
            highlighted: matches!(state.update_status, UpdateStatus::Available { .. }),
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:install-update"),
            label: SharedString::from("Restart to Update"),
            detail: match &state.update_status {
                UpdateStatus::ReadyToInstall { version, .. } => {
                    Some(SharedString::from(format!("v{version}")))
                }
                _ => None,
            },
            category: ActionCategory::Command,
            available: matches!(state.update_status, UpdateStatus::ReadyToInstall { .. }),
            priority: -20,
            highlighted: matches!(state.update_status, UpdateStatus::ReadyToInstall { .. }),
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:connect"),
            label: SharedString::from("Connect"),
            detail: Some(SharedString::from("Connect to a saved connection")),
            category: ActionCategory::Command,
            available: !state.connections.is_empty(),
            priority: 4,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:change-theme"),
            label: SharedString::from("Theme Selector: Toggle"),
            category: ActionCategory::Command,
            available: true,
            priority: 90,
            ..Default::default()
        },
    ]
}

/// Theme picker: flat list of all themes, current theme highlighted.
pub fn theme_actions(state: &AppState) -> Vec<ActionItem> {
    let current = state.settings.appearance.theme;
    let mut actions = Vec::new();

    for (i, theme) in AppTheme::dark_themes().iter().chain(AppTheme::light_themes()).enumerate() {
        actions.push(ActionItem {
            id: SharedString::from(format!("theme:{}", theme.theme_id())),
            label: SharedString::from(theme.label()),
            category: ActionCategory::Command,
            available: true,
            highlighted: *theme == current,
            priority: i as i32,
            ..Default::default()
        });
    }

    actions
}

/// Connect: disconnected saved connections available to connect.
pub fn connection_actions(state: &AppState) -> Vec<ActionItem> {
    let active = state.active_connections_snapshot();
    state
        .connections
        .iter()
        .filter(|c| !active.contains_key(&c.id))
        .map(|c| ActionItem {
            id: SharedString::from(format!("connect:{}", c.id)),
            label: SharedString::from(c.name.clone()),
            detail: Some(SharedString::from("Connect")),
            category: ActionCategory::Command,
            available: true,
            priority: 5,
            ..Default::default()
        })
        .collect()
}

/// Disconnect: connected connections available to disconnect.
pub fn disconnect_actions(state: &AppState) -> Vec<ActionItem> {
    let active = state.active_connections_snapshot();
    active
        .iter()
        .map(|(id, conn)| ActionItem {
            id: SharedString::from(format!("disconnect:{}", id)),
            label: SharedString::from(conn.config.name.clone()),
            detail: Some(SharedString::from("Disconnect")),
            category: ActionCategory::Command,
            available: true,
            priority: 5,
            ..Default::default()
        })
        .collect()
}

/// View: subview toggles (documents/indexes/stats).
pub fn view_actions(state: &AppState) -> Vec<ActionItem> {
    let has_collection = state.selected_collection().is_some();

    vec![
        ActionItem {
            id: SharedString::from("view:documents"),
            label: SharedString::from("Show Documents"),
            category: ActionCategory::View,
            shortcut: Some(SharedString::from("Cmd+Alt+1")),
            available: has_collection,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("view:indexes"),
            label: SharedString::from("Show Indexes"),
            category: ActionCategory::View,
            shortcut: Some(SharedString::from("Cmd+Alt+2")),
            available: has_collection,
            priority: 1,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("view:stats"),
            label: SharedString::from("Show Stats"),
            category: ActionCategory::View,
            shortcut: Some(SharedString::from("Cmd+Alt+3")),
            available: has_collection,
            priority: 2,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("view:aggregation"),
            label: SharedString::from("Show Aggregation"),
            category: ActionCategory::View,
            shortcut: Some(SharedString::from("Cmd+Alt+4")),
            available: has_collection,
            priority: 3,
            ..Default::default()
        },
    ]
}
