use gpui_kit::{Action, Keystroke, SharedString, Window};

use crate::components::ConnectionIdentity;
use crate::keyboard::{
    CloseTab, CreateCollection, CreateDatabase, CreateIndex, DiscardDocumentChanges, FocusContent,
    FocusSidebar, InsertDocument, NewConnection, OpenConnectionSwitcher, OpenForge,
    OpenQueryLibrary, OpenSettings, RefreshView, RunAggregation, SaveDocument,
    ShowAggregationSubview, ShowDocumentsSubview, ShowHistorySubview, ShowIndexesSubview,
    ShowSchemaSubview, ShowStatsSubview, ToggleAiPanel, TransferCopy, TransferExport,
    TransferImport,
};
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
            TabKey::Compare(_) => ("Compare".into(), "Compare two collections".into()),
            TabKey::References(key) => {
                let label = state
                    .references_tab(key.id)
                    .map(|tab| format!("References to {} {}", key.collection, tab.label))
                    .unwrap_or_else(|| format!("References to {}", key.collection));
                let conn_name = state
                    .connection_name(key.connection_id)
                    .unwrap_or_else(|| "Connection".to_string());
                (label, format!("{} / {}", conn_name, key.database))
            }
            TabKey::Relations(key) => {
                let conn_name = state
                    .connection_name(key.connection_id)
                    .unwrap_or_else(|| "Connection".to_string());
                (format!("Relations of {}", key.database), conn_name)
            }
            TabKey::AgentActivity => {
                ("Agent Activity".to_string(), "Approvals and operations".to_string())
            }
            TabKey::Connections => {
                ("Connections".to_string(), "Manage MongoDB connections".to_string())
            }
            TabKey::Settings => ("Settings".to_string(), "Application settings".to_string()),
            TabKey::Changelog => ("What's new".to_string(), "Changelog".to_string()),
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

/// Resolved while the palette opens, so bindings follow the context that was focused before it.
fn registered_shortcut(window: &Window, action: &dyn Action) -> Option<Keystroke> {
    crate::keyboard::display_keystroke(&window.bindings_for_action(action))
}

/// Commands: create, delete, refresh, disconnect, etc.
pub fn command_actions(state: &AppState, window: &Window) -> Vec<ActionItem> {
    let has_connection = state.has_active_connections();
    let has_selected = state.selected_connection_id().is_some();
    let is_connected = has_selected
        && state.selected_connection_id().map(|id| state.is_connected(id)).unwrap_or(false);
    let current_session = state.current_session_key();
    let has_collection = current_session.is_some();
    let is_documents = matches!(state.current_view, crate::state::View::Documents);
    let current_subview = current_session.as_ref().and_then(|key| state.session_subview(key));
    let has_database = state.current_database_key().is_some();
    let can_close_tab = !state.open_tabs().is_empty() || state.preview_tab().is_some();

    vec![
        ActionItem {
            id: "cmd:compare".into(),
            label: "Compare collections…".into(),
            keywords: &["difference", "diff", "compare", "environments"],
            category: ActionCategory::Command,
            available: true,
            ..Default::default()
        },
        ActionItem {
            id: "cmd:compare-databases".into(),
            label: "Compare databases…".into(),
            keywords: &["difference", "diff", "compare", "environments", "schema"],
            category: ActionCategory::Command,
            available: true,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:new-connection"),
            keywords: &["add", "create", "uri"],
            label: SharedString::from("New connection"),
            category: ActionCategory::Command,
            available: true,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:create-database"),
            keywords: &["new", "add", "db"],
            label: SharedString::from("Create database"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &CreateDatabase),
            available: is_connected,
            priority: 10,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:create-collection"),
            keywords: &["new", "add", "table"],
            label: SharedString::from("Create collection"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &CreateCollection),
            available: is_connected && state.selected_database().is_some(),
            priority: 11,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:insert-document"),
            keywords: &["add", "new", "create", "record"],
            label: SharedString::from("Insert document"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &InsertDocument),
            available: has_collection,
            priority: 12,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:create-index"),
            keywords: &["new", "add"],
            label: SharedString::from("Create index"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &CreateIndex),
            available: has_collection,
            priority: 13,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:run-aggregation"),
            keywords: &["pipeline"],
            label: SharedString::from("Run aggregation"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &RunAggregation),
            available: has_collection,
            priority: 14,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:open-forge"),
            keywords: &["shell", "mongosh", "query", "script"],
            label: SharedString::from("Open Forge"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &OpenForge),
            available: has_database,
            priority: 15,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:transfer-export"),
            keywords: &["dump", "backup", "download", "json", "csv"],
            label: SharedString::from("Export data…"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &TransferExport),
            available: has_database,
            priority: 16,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:transfer-import"),
            keywords: &["restore", "load", "upload"],
            label: SharedString::from("Import data…"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &TransferImport),
            available: has_database,
            priority: 17,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:transfer-copy"),
            keywords: &["clone", "duplicate", "migrate"],
            label: SharedString::from("Copy data…"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &TransferCopy),
            available: has_database,
            priority: 18,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:save-document"),
            keywords: &["commit", "apply"],
            label: SharedString::from("Save document changes"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &SaveDocument),
            available: is_documents
                && current_subview == Some(crate::state::CollectionSubview::Documents),
            priority: 19,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:discard-document"),
            keywords: &["revert", "undo"],
            label: SharedString::from("Discard document changes"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &DiscardDocumentChanges),
            available: is_documents
                && current_subview == Some(crate::state::CollectionSubview::Documents),
            priority: 20,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:close-tab"),
            label: SharedString::from("Close tab"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &CloseTab),
            available: can_close_tab,
            priority: 21,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:focus-sidebar"),
            label: SharedString::from("Focus sidebar"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &FocusSidebar),
            available: true,
            priority: 22,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:focus-content"),
            label: SharedString::from("Focus content"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &FocusContent),
            available: true,
            priority: 23,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:refresh"),
            keywords: &["reload"],
            label: SharedString::from("Refresh"),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &RefreshView),
            available: has_connection,
            priority: 20,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:disconnect"),
            keywords: &["close"],
            label: SharedString::from("Disconnect…"),
            category: ActionCategory::Command,
            available: has_connection,
            priority: 30,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:query-library"),
            keywords: &["history", "saved", "snippets"],
            label: SharedString::from("Query library"),
            detail: Some(SharedString::from("Search query history and saved queries")),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &OpenQueryLibrary),
            available: state.has_query_library_target(),
            priority: 22,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:settings"),
            keywords: &["preferences", "options", "config"],
            label: SharedString::from("Settings"),
            detail: Some(SharedString::from("Application settings")),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &OpenSettings),
            available: true,
            priority: 100,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:ai"),
            keywords: &["chat", "assistant"],
            label: SharedString::from("AI assistant"),
            detail: Some(SharedString::from("Toggle assistant side panel")),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &ToggleAiPanel),
            available: state.ai_assistant_available(),
            priority: 95,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:date-display"),
            keywords: &["utc", "local", "time zone", "timezone", "date"],
            label: SharedString::from(match crate::bson::date_display() {
                crate::bson::DateDisplay::Utc => "Show dates in local time",
                crate::bson::DateDisplay::Local => "Show dates in UTC",
            }),
            detail: Some(SharedString::from("Copied and exported dates stay UTC")),
            category: ActionCategory::Command,
            available: true,
            priority: 104,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:whats-new"),
            label: SharedString::from("What's new"),
            detail: Some(SharedString::from("View changelog")),
            category: ActionCategory::Command,
            available: true,
            priority: 105,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:fps-monitor"),
            keywords: &["fps", "frame", "performance", "debug", "hud"],
            label: SharedString::from("Toggle FPS monitor"),
            detail: Some(SharedString::from("Frame time, CPU and memory overlay")),
            category: ActionCategory::Command,
            available: true,
            priority: 106,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:check-updates"),
            keywords: &["upgrade", "version"],
            label: SharedString::from("Check for updates"),
            category: ActionCategory::Command,
            available: true,
            priority: 110,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:download-update"),
            label: SharedString::from("Download update"),
            detail: match &state.update_status {
                UpdateStatus::Available(release) => Some(SharedString::from(release.label())),
                _ => None,
            },
            category: ActionCategory::Command,
            available: matches!(state.update_status, UpdateStatus::Available(_)),
            priority: -10,
            highlighted: matches!(state.update_status, UpdateStatus::Available(_)),
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:install-update"),
            label: SharedString::from("Restart to update"),
            detail: match &state.update_status {
                UpdateStatus::ReadyToInstall(download) => {
                    Some(SharedString::from(download.release.label()))
                }
                _ => None,
            },
            category: ActionCategory::Command,
            available: matches!(state.update_status, UpdateStatus::ReadyToInstall(_)),
            priority: -20,
            highlighted: matches!(state.update_status, UpdateStatus::ReadyToInstall(_)),
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:connect"),
            keywords: &["open", "connections"],
            label: SharedString::from("Switch connection…"),
            detail: Some(SharedString::from("Open or connect to a saved connection")),
            category: ActionCategory::Command,
            shortcut: registered_shortcut(window, &OpenConnectionSwitcher),
            available: true,
            priority: 4,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("cmd:change-theme"),
            keywords: &["appearance", "dark", "light", "color"],
            label: SharedString::from("Change theme…"),
            category: ActionCategory::Command,
            available: true,
            priority: 90,
            ..Default::default()
        },
    ]
}

/// Theme picker: match the system, then dark and light themes, the current choice checked.
pub fn theme_actions(state: &AppState) -> Vec<ActionItem> {
    let appearance = &state.settings.appearance;
    let dark = AppTheme::dark_themes().iter().map(|theme| (theme, ActionCategory::DarkTheme));
    let light = AppTheme::light_themes().iter().map(|theme| (theme, ActionCategory::LightTheme));
    let system = ActionItem {
        id: SharedString::from("theme:system"),
        label: SharedString::from("Match system appearance"),
        detail: Some(SharedString::from("Mango Dark or Mango Light")),
        keywords: &["auto", "os", "macos"],
        category: ActionCategory::Command,
        available: true,
        checked: appearance.follow_system,
        ..Default::default()
    };
    let themes = dark.chain(light).enumerate().map(|(i, (theme, category))| ActionItem {
        id: SharedString::from(format!("theme:{}", theme.theme_id())),
        label: SharedString::from(theme.label()),
        category,
        available: true,
        checked: !appearance.follow_system && *theme == appearance.theme,
        priority: i as i32,
        ..Default::default()
    });
    std::iter::once(system).chain(themes).collect()
}

/// Connection switcher: open connections first, then saved ones by recent use.
pub fn connection_switcher_actions(state: &AppState, window: &Window) -> Vec<ActionItem> {
    let mut connections = state.connections.iter().collect::<Vec<_>>();
    connections.sort_by(|a, b| a.cmp_recent_use(b));

    let mut actions = connections
        .into_iter()
        .enumerate()
        .map(|(ix, connection)| {
            let connected = state.is_connected(connection.id);
            ActionItem {
                id: SharedString::from(if connected {
                    format!("nav:conn:{}", connection.id)
                } else {
                    format!("connect:{}", connection.id)
                }),
                label: SharedString::from(connection.name.clone()),
                category: if connected { ActionCategory::Connected } else { ActionCategory::Saved },
                available: true,
                priority: ix as i32,
                connection: Some(ConnectionIdentity::from(connection)),
                ..Default::default()
            }
        })
        .collect::<Vec<_>>();

    actions.push(ActionItem {
        id: SharedString::from("cmd:new-connection"),
        label: SharedString::from("New connection"),
        category: ActionCategory::Command,
        shortcut: registered_shortcut(window, &NewConnection),
        available: true,
        priority: 0,
        ..Default::default()
    });
    actions.push(ActionItem {
        id: SharedString::from("cmd:manage-connections"),
        label: SharedString::from("Manage connections"),
        category: ActionCategory::Command,
        available: true,
        priority: 1,
        ..Default::default()
    });
    actions
}

/// Disconnect: connected connections available to disconnect.
pub fn disconnect_actions(state: &AppState) -> Vec<ActionItem> {
    let active = state.active_connections_snapshot();
    active
        .iter()
        .map(|(id, conn)| ActionItem {
            id: SharedString::from(format!("disconnect:{}", id)),
            label: SharedString::from(conn.config.name.clone()),
            category: ActionCategory::Command,
            available: true,
            priority: 5,
            ..Default::default()
        })
        .collect()
}

/// View: subview toggles (documents/indexes/stats).
pub fn view_actions(state: &AppState, window: &Window) -> Vec<ActionItem> {
    let has_collection = state.current_session_key().is_some();

    vec![
        ActionItem {
            id: SharedString::from("view:documents"),
            label: SharedString::from("Show documents"),
            category: ActionCategory::View,
            shortcut: registered_shortcut(window, &ShowDocumentsSubview),
            available: has_collection,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("view:indexes"),
            label: SharedString::from("Show indexes"),
            category: ActionCategory::View,
            shortcut: registered_shortcut(window, &ShowIndexesSubview),
            available: has_collection,
            priority: 1,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("view:stats"),
            label: SharedString::from("Show stats"),
            category: ActionCategory::View,
            shortcut: registered_shortcut(window, &ShowStatsSubview),
            available: has_collection,
            priority: 2,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("view:aggregation"),
            label: SharedString::from("Show aggregation"),
            category: ActionCategory::View,
            shortcut: registered_shortcut(window, &ShowAggregationSubview),
            available: has_collection,
            priority: 3,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("view:history"),
            label: SharedString::from("Show History"),
            category: ActionCategory::View,
            shortcut: registered_shortcut(window, &ShowHistorySubview),
            available: has_collection,
            priority: 4,
            ..Default::default()
        },
        ActionItem {
            id: SharedString::from("view:schema"),
            label: SharedString::from("Show schema"),
            category: ActionCategory::View,
            shortcut: registered_shortcut(window, &ShowSchemaSubview),
            available: has_collection,
            priority: 5,
            ..Default::default()
        },
    ]
}
