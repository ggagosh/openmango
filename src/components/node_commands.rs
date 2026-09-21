//! Commands on a tree node: a connection, a database or a collection.
//!
//! Each command has one body, here. The sidebar acts on the selected row, the window-level
//! actions on what the app is showing, and the context menus on the row under the pointer; all
//! they do is work out *which* node and call in. That is what keeps their wording, their guards
//! and their results from drifting apart.

use gpui_kit::{App, ClipboardItem, Entity, Window};

use super::{
    WriteConfirmation, WriteRequest, open_confirm_dialog, request_connection_write,
    request_remove_connection,
};
use crate::models::TreeNodeId;
use crate::state::{AppCommands, AppState};

/// The name a node goes by: the connection's name, or the database or collection name itself.
pub fn node_name(state: &AppState, node: &TreeNodeId) -> Option<String> {
    match node {
        TreeNodeId::Connection(connection_id) => state.connection_name(*connection_id),
        TreeNodeId::Database { database, .. } => Some(database.clone()),
        TreeNodeId::Collection { collection, .. } => Some(collection.clone()),
    }
}

/// Copy Name. Copying `database/collection` is a different command (Copy, on the tree).
pub fn copy_node_name(state: &Entity<AppState>, node: &TreeNodeId, cx: &mut App) {
    if let Some(name) = node_name(state.read(cx), node) {
        cx.write_to_clipboard(ClipboardItem::new_string(name));
    }
}

/// Asks, then removes the connection or drops the database or collection. The title is the
/// decision and names its object; the body says only what the title cannot: that it is final.
pub fn confirm_delete_node(
    state: Entity<AppState>,
    node: TreeNodeId,
    window: &mut Window,
    cx: &mut App,
) {
    const FINAL: &str = "This cannot be undone.";
    match node {
        TreeNodeId::Connection(connection_id) => {
            let name = state
                .read(cx)
                .connection_name(connection_id)
                .unwrap_or_else(|| "this connection".to_string());
            let title = format!("Remove connection \"{name}\"?");
            open_confirm_dialog(window, cx, title, FINAL.to_string(), "Remove", true, {
                move |window, cx| {
                    request_remove_connection(state.clone(), connection_id, window, cx);
                }
            });
        }
        TreeNodeId::Database { connection, database } => {
            let confirmation = WriteConfirmation {
                title: format!("Drop database \"{database}\"?"),
                message: FINAL.to_string(),
                confirm_label: "Drop".into(),
                destructive: true,
            };
            let request = WriteRequest::new(
                connection,
                database.clone(),
                "Drop a database",
                Some(confirmation),
            );
            request_connection_write(state.clone(), request, window, cx, move |_window, cx| {
                AppCommands::drop_database(state, connection, database, cx);
            });
        }
        TreeNodeId::Collection { connection, database, collection } => {
            let namespace = format!("{database}.{collection}");
            // Dropping a view is far less than it sounds, so the dialog says what survives.
            let view_source = state
                .read(cx)
                .active_connection_by_id(connection)
                .and_then(|conn| conn.collection_detail(&database, &collection))
                .and_then(|detail| match detail {
                    crate::models::CollectionDetail::View { view_on, .. } => Some(view_on.clone()),
                    crate::models::CollectionDetail::Timeseries => None,
                });
            let (title, message, operation) = match view_source {
                Some(source) => (
                    format!("Drop view \"{namespace}\"?"),
                    format!(
                        "Only the view is removed. The documents in {source} are untouched. {FINAL}"
                    ),
                    "Drop a view",
                ),
                None => (
                    format!("Drop collection \"{namespace}\"?"),
                    FINAL.to_string(),
                    "Drop a collection",
                ),
            };
            let confirmation = WriteConfirmation {
                title,
                message,
                confirm_label: "Drop".into(),
                destructive: true,
            };
            let request = WriteRequest::new(connection, namespace, operation, Some(confirmation));
            request_connection_write(state.clone(), request, window, cx, move |_window, cx| {
                AppCommands::drop_collection(state, connection, database, collection, cx);
            });
        }
    }
}
