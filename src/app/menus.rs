use gpui::*;
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use uuid::Uuid;

use crate::components::{ConnectionManager, TreeNodeId, open_confirm_dialog};
use crate::keyboard::{
    CopyConnectionUri, CopySelectionName, CreateCollection, DeleteSelection, DisconnectConnection,
    EditConnection, OpenSelection, RefreshView, RenameCollection,
};
use crate::state::{AppCommands, AppState};

use super::dialogs::{open_create_collection_dialog, open_rename_collection_dialog};
use super::sidebar::Sidebar;

pub(crate) fn build_connection_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    connection_id: Uuid,
    connecting_id: Option<Uuid>,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let is_connected = state.read(cx).is_connected(connection_id);
    let is_connecting = connecting_id == Some(connection_id);

    menu = menu
        .item(
            PopupMenuItem::new("Connect")
                .action(Box::new(OpenSelection))
                .disabled(is_connected || is_connecting)
                .on_click({
                    let state = state.clone();
                    let sidebar = sidebar.clone();
                    move |_, _window, cx| {
                        if state.read(cx).is_connected(connection_id) {
                            return;
                        }

                        sidebar.update(cx, |sidebar, cx| {
                            sidebar.expand_connection_and_refresh(connection_id, cx);
                        });

                        AppCommands::connect(state.clone(), connection_id, cx);
                    }
                }),
        )
        .item(PopupMenuItem::new("Edit Connection...").action(Box::new(EditConnection)).on_click({
            let state = state.clone();
            move |_, window, cx| {
                ConnectionManager::open_selected(state.clone(), connection_id, window, cx);
            }
        }))
        .item(
            PopupMenuItem::new("Remove Connection...").action(Box::new(DeleteSelection)).on_click(
                {
                    let state = state.clone();
                    move |_, window, cx| {
                        let name = state
                            .read(cx)
                            .connection_name(connection_id)
                            .unwrap_or_else(|| "connection".to_string());
                        let message = format!("Remove connection \"{name}\"?");
                        open_confirm_dialog(
                            window,
                            cx,
                            "Remove connection",
                            message,
                            "Remove",
                            true,
                            {
                                let state = state.clone();
                                move |_window, cx| {
                                    state.update(cx, |state, cx| {
                                        state.remove_connection(connection_id, cx);
                                    });
                                }
                            },
                        );
                    }
                },
            ),
        )
        .item(PopupMenuItem::new("Disconnect").action(Box::new(DisconnectConnection)).on_click({
            let state = state.clone();
            move |_, _window, cx| {
                AppCommands::disconnect(state.clone(), connection_id, cx);
            }
        }))
        .separator()
        .item(PopupMenuItem::new("Copy URI").action(Box::new(CopyConnectionUri)).on_click({
            let state = state.clone();
            move |_, _window, cx| {
                if let Some(uri) = state.read(cx).connection_uri(connection_id) {
                    cx.write_to_clipboard(ClipboardItem::new_string(uri));
                }
            }
        }))
        .item(PopupMenuItem::new("Copy Name").action(Box::new(CopySelectionName)).on_click({
            let state = state.clone();
            move |_, _window, cx| {
                if let Some(name) = state.read(cx).connection_name(connection_id) {
                    cx.write_to_clipboard(ClipboardItem::new_string(name));
                }
            }
        }));

    menu
}

pub(crate) fn build_database_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    node_id: TreeNodeId,
    database: String,
    is_loading: bool,
    _cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let database_for_select = database.clone();
    let database_for_create = database.clone();
    let database_for_refresh = database.clone();
    let database_for_drop = database.clone();
    let database_for_copy = database;

    menu = menu
        .item(PopupMenuItem::new("Select Database").action(Box::new(OpenSelection)).on_click({
            let state = state.clone();
            let connection_id = node_id.connection_id();
            move |_, _window, cx| {
                state.update(cx, |state, cx| {
                    state.select_connection(Some(connection_id), cx);
                    state.select_database(database_for_select.clone(), cx);
                });
            }
        }))
        .item(
            PopupMenuItem::new("Create Collection...").action(Box::new(CreateCollection)).on_click(
                {
                    let state = state.clone();
                    let connection_id = node_id.connection_id();
                    let database = database_for_create.clone();
                    move |_, window, cx| {
                        state.update(cx, |state, cx| {
                            state.select_connection(Some(connection_id), cx);
                        });
                        open_create_collection_dialog(state.clone(), database.clone(), window, cx);
                    }
                },
            ),
        )
        .item(
            PopupMenuItem::new("Refresh Collections")
                .action(Box::new(RefreshView))
                .disabled(is_loading)
                .on_click({
                    let state = state.clone();
                    let sidebar = sidebar.clone();
                    let node_id = node_id.clone();
                    let connection_id = node_id.connection_id();
                    move |_, _window, cx| {
                        sidebar.update(cx, |sidebar, cx| {
                            sidebar.mark_database_loading(node_id.clone(), cx);
                        });
                        AppCommands::load_collections(
                            state.clone(),
                            connection_id,
                            database_for_refresh.clone(),
                            cx,
                        );
                    }
                }),
        )
        .item(PopupMenuItem::new("Drop Database...").action(Box::new(DeleteSelection)).on_click({
            let state = state.clone();
            let connection_id = node_id.connection_id();
            let database = database_for_drop.clone();
            move |_, window, cx| {
                let message = format!("Drop database \"{database}\"? This cannot be undone.");
                open_confirm_dialog(window, cx, "Drop database", message, "Drop", true, {
                    let state = state.clone();
                    let database = database.clone();
                    move |_window, cx| {
                        state.update(cx, |state, cx| {
                            state.select_connection(Some(connection_id), cx);
                        });
                        AppCommands::drop_database(state.clone(), database.clone(), cx);
                    }
                });
            }
        }))
        .separator()
        .item(PopupMenuItem::new("Copy Name").action(Box::new(CopySelectionName)).on_click({
            move |_, _window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(database_for_copy.clone()));
            }
        }));

    menu
}

pub(crate) fn build_collection_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    connection_id: Uuid,
    database: String,
    collection: String,
    label: String,
    _cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    menu = menu
        .item(PopupMenuItem::new("Open Collection").action(Box::new(OpenSelection)).on_click({
            let state = state.clone();
            let database = database.clone();
            let collection = collection.clone();
            move |_, _window, cx| {
                state.update(cx, |state, cx| {
                    state.select_connection(Some(connection_id), cx);
                    state.select_database(database.clone(), cx);
                    state.select_collection(database.clone(), collection.clone(), cx);
                });
            }
        }))
        .item(
            PopupMenuItem::new("Rename Collection...").action(Box::new(RenameCollection)).on_click(
                {
                    let state = state.clone();
                    let database = database.clone();
                    let collection = collection.clone();
                    move |_, window, cx| {
                        state.update(cx, |state, cx| {
                            state.select_connection(Some(connection_id), cx);
                        });
                        open_rename_collection_dialog(
                            state.clone(),
                            database.clone(),
                            collection.clone(),
                            window,
                            cx,
                        );
                    }
                },
            ),
        )
        .item(PopupMenuItem::new("Drop Collection...").action(Box::new(DeleteSelection)).on_click(
            {
                let state = state.clone();
                let database = database.clone();
                let collection = collection.clone();
                move |_, window, cx| {
                    let message = format!(
                        "Drop collection \"{database}.{collection}\"? This cannot be undone."
                    );
                    open_confirm_dialog(window, cx, "Drop collection", message, "Drop", true, {
                        let state = state.clone();
                        let database = database.clone();
                        let collection = collection.clone();
                        move |_window, cx| {
                            state.update(cx, |state, cx| {
                                state.select_connection(Some(connection_id), cx);
                            });
                            AppCommands::drop_collection(
                                state.clone(),
                                database.clone(),
                                collection.clone(),
                                cx,
                            );
                        }
                    });
                }
            },
        ))
        .separator()
        .item(PopupMenuItem::new("Copy Name").action(Box::new(CopySelectionName)).on_click({
            move |_, _window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(label.clone()));
            }
        }));

    menu
}
