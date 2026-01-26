use gpui::SharedString;

use crate::state::{AppState, TabKey};

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
            shortcut: None,
            available: true,
            priority: 0,
        });

        // Databases
        for db in &conn.databases {
            actions.push(ActionItem {
                id: SharedString::from(format!("nav:db:{}:{}", conn_id, db)),
                label: SharedString::from(db.clone()),
                detail: Some(SharedString::from(conn_name.clone())),
                category: ActionCategory::Navigation,
                shortcut: None,
                available: true,
                priority: 10,
            });

            // Collections within this database
            if let Some(collections) = conn.collections.get(db) {
                for col in collections {
                    actions.push(ActionItem {
                        id: SharedString::from(format!("nav:col:{}:{}:{}", conn_id, db, col)),
                        label: SharedString::from(col.clone()),
                        detail: Some(SharedString::from(format!("{} / {}", conn_name, db))),
                        category: ActionCategory::Navigation,
                        shortcut: None,
                        available: true,
                        priority: 20,
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
        };

        actions.push(ActionItem {
            id: SharedString::from(format!("tab:{}", index)),
            label: SharedString::from(label),
            detail: Some(SharedString::from(detail)),
            category: ActionCategory::Tab,
            shortcut: None,
            available: true,
            priority: index as i32,
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
            shortcut: None,
            available: true,
            priority: actions.len() as i32,
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
            detail: None,
            category: ActionCategory::Command,
            shortcut: None,
            available: true,
            priority: 0,
        },
        ActionItem {
            id: SharedString::from("cmd:create-database"),
            label: SharedString::from("Create Database"),
            detail: None,
            category: ActionCategory::Command,
            shortcut: Some(SharedString::from("Cmd+Shift+N")),
            available: is_connected,
            priority: 10,
        },
        ActionItem {
            id: SharedString::from("cmd:create-collection"),
            label: SharedString::from("Create Collection"),
            detail: None,
            category: ActionCategory::Command,
            shortcut: Some(SharedString::from("Cmd+N")),
            available: is_connected && state.selected_database().is_some(),
            priority: 11,
        },
        ActionItem {
            id: SharedString::from("cmd:refresh"),
            label: SharedString::from("Refresh"),
            detail: None,
            category: ActionCategory::Command,
            shortcut: Some(SharedString::from("Cmd+R")),
            available: has_connection,
            priority: 20,
        },
        ActionItem {
            id: SharedString::from("cmd:disconnect"),
            label: SharedString::from("Disconnect"),
            detail: None,
            category: ActionCategory::Command,
            shortcut: Some(SharedString::from("Cmd+Shift+D")),
            available: is_connected,
            priority: 30,
        },
    ]
}

/// View: subview toggles (documents/indexes/stats).
pub fn view_actions(state: &AppState) -> Vec<ActionItem> {
    let has_collection = state.selected_collection().is_some();

    vec![
        ActionItem {
            id: SharedString::from("view:documents"),
            label: SharedString::from("Show Documents"),
            detail: None,
            category: ActionCategory::View,
            shortcut: Some(SharedString::from("Cmd+Alt+1")),
            available: has_collection,
            priority: 0,
        },
        ActionItem {
            id: SharedString::from("view:indexes"),
            label: SharedString::from("Show Indexes"),
            detail: None,
            category: ActionCategory::View,
            shortcut: Some(SharedString::from("Cmd+Alt+2")),
            available: has_collection,
            priority: 1,
        },
        ActionItem {
            id: SharedString::from("view:stats"),
            label: SharedString::from("Show Stats"),
            detail: None,
            category: ActionCategory::View,
            shortcut: Some(SharedString::from("Cmd+Alt+3")),
            available: has_collection,
            priority: 2,
        },
        ActionItem {
            id: SharedString::from("view:aggregation"),
            label: SharedString::from("Show Aggregation"),
            detail: None,
            category: ActionCategory::View,
            shortcut: Some(SharedString::from("Cmd+Alt+4")),
            available: has_collection,
            priority: 3,
        },
    ]
}
