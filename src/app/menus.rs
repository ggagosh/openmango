use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use uuid::Uuid;

use crate::components::{ConnectionManager, TreeNodeId, open_confirm_dialog};
use crate::keyboard::{
    CopyConnectionUri, CopySelectionName, CopyTreeItem, CreateCollection, DeleteSelection,
    DisconnectConnection, EditConnection, OpenForge, OpenSelection, PasteTreeItem, RefreshView,
    RenameCollection, TransferCopy, TransferExport, TransferImport,
};
use crate::state::{
    AppCommands, AppState, CopiedTreeItem, StatusMessage, TransferMode, TransferScope, View,
};
use crate::theme::{colors, spacing};

use super::dialogs::{open_create_collection_dialog, open_rename_collection_dialog};
use super::sidebar::Sidebar;

fn maybe_occlude_webview(
    state: &Entity<AppState>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) {
    if !matches!(state.read(cx).current_view, View::Forge) {
        return;
    }

    let menu_entity = cx.entity();
    let state_for_dismiss = state.clone();
    let subscription =
        window.subscribe(&menu_entity, cx, move |_, _: &DismissEvent, _window, cx| {
            state_for_dismiss.update(cx, |state, cx| {
                state.set_webview_occluded(false, None, cx);
            });
        });

    state.update(cx, |state, cx| {
        state.set_webview_occluded(true, Some(subscription), cx);
    });
}

pub(crate) fn build_connection_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    connection_id: Uuid,
    connecting_id: Option<Uuid>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    maybe_occlude_webview(&state, window, cx);
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

fn menu_item_with_shortcut(label: &'static str, shortcut: &'static str) -> PopupMenuItem {
    PopupMenuItem::element(move |_window, _cx| {
        div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .gap(spacing::lg())
            .child(div().text_sm().child(label))
            .child(div().text_xs().text_color(colors::text_muted()).child(shortcut))
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_database_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    node_id: TreeNodeId,
    database: String,
    is_loading: bool,
    window: &mut Window,
    _cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    maybe_occlude_webview(&state, window, _cx);
    let database_for_select = database.clone();
    let database_for_create = database.clone();
    let database_for_refresh = database.clone();
    let database_for_drop = database.clone();
    let database_for_export = database.clone();
    let database_for_import = database.clone();
    let database_for_transfer_copy = database.clone();
    let database_for_forge = database.clone();
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
            menu_item_with_shortcut("Open Forge", "Cmd+Alt+F")
                .on_click({
                    let state = state.clone();
                    let connection_id = node_id.connection_id();
                    let database = database_for_forge.clone();
                    move |_, _window, cx| {
                        state.update(cx, |state, cx| {
                            state.open_forge_tab(connection_id, database.clone(), cx);
                        });
                    }
                })
                .action(Box::new(OpenForge)),
        )
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
        .item(
            menu_item_with_shortcut("Export...", "Cmd+Alt+E")
                .action(Box::new(TransferExport))
                .on_click({
                    let state = state.clone();
                    let database = database_for_export.clone();
                    let connection_id = node_id.connection_id();
                    move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.open_transfer_tab_with_prefill(
                                connection_id,
                                database.clone(),
                                None,
                                TransferScope::Database,
                                TransferMode::Export,
                                cx,
                            );
                        });
                    }
                }),
        )
        .item(
            menu_item_with_shortcut("Import...", "Cmd+Alt+I")
                .action(Box::new(TransferImport))
                .on_click({
                    let state = state.clone();
                    let database = database_for_import.clone();
                    let connection_id = node_id.connection_id();
                    move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.open_transfer_tab_with_prefill(
                                connection_id,
                                database.clone(),
                                None,
                                TransferScope::Database,
                                TransferMode::Import,
                                cx,
                            );
                        });
                    }
                }),
        )
        .item(
            menu_item_with_shortcut("Copy to...", "Cmd+Alt+C")
                .action(Box::new(TransferCopy))
                .on_click({
                    let state = state.clone();
                    let database = database_for_transfer_copy.clone();
                    let connection_id = node_id.connection_id();
                    move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.open_transfer_tab_with_prefill(
                                connection_id,
                                database.clone(),
                                None,
                                TransferScope::Database,
                                TransferMode::Copy,
                                cx,
                            );
                        });
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
        .item(menu_item_with_shortcut("Copy", "⌘C").action(Box::new(CopyTreeItem)).on_click({
            let state = state.clone();
            let connection_id = node_id.connection_id();
            let database = database_for_copy.clone();
            move |_, _window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(database.clone()));
                state.update(cx, |state, cx| {
                    state.copied_tree_item = Some(CopiedTreeItem::Database {
                        connection_id,
                        database: database.clone(),
                    });
                    state.set_status_message(Some(StatusMessage::info(format!(
                        "Copied database: {}",
                        database
                    ))));
                    cx.notify();
                });
            }
        }))
        .when(state.read(_cx).copied_tree_item.is_some(), |menu: PopupMenu| {
            let dest_connection_id = node_id.connection_id();
            let dest_database = database_for_copy.clone();
            menu.item(
                menu_item_with_shortcut("Paste", "⌘V").action(Box::new(PasteTreeItem)).on_click({
                    let state = state.clone();
                    move |_, _window, cx| {
                        let copied = state.read(cx).copied_tree_item.clone();
                        let Some(item) = copied else {
                            return;
                        };

                        let source_connection_id = match &item {
                            CopiedTreeItem::Database { connection_id, .. } => *connection_id,
                            CopiedTreeItem::Collection { connection_id, .. } => *connection_id,
                        };

                        if state.read(cx).connection_by_id(source_connection_id).is_none() {
                            state.update(cx, |state, cx| {
                                state.set_status_message(Some(StatusMessage::error(
                                    "Source connection no longer exists",
                                )));
                                state.copied_tree_item = None;
                                cx.notify();
                            });
                            return;
                        }

                        state.update(cx, |state, cx| match item {
                            CopiedTreeItem::Database { connection_id, database } => {
                                state.open_transfer_tab_for_paste(
                                    connection_id,
                                    database,
                                    None,
                                    Some(dest_connection_id),
                                    Some(dest_database.clone()),
                                    TransferScope::Database,
                                    cx,
                                );
                            }
                            CopiedTreeItem::Collection { connection_id, database, collection } => {
                                state.open_transfer_tab_for_paste(
                                    connection_id,
                                    database,
                                    Some(collection),
                                    Some(dest_connection_id),
                                    Some(dest_database.clone()),
                                    TransferScope::Collection,
                                    cx,
                                );
                            }
                        });
                    }
                }),
            )
        });

    menu
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_collection_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    connection_id: Uuid,
    database: String,
    collection: String,
    label: String,
    window: &mut Window,
    _cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    maybe_occlude_webview(&state, window, _cx);
    let label_for_copy = label.clone();
    let database_for_copy = database.clone();
    let collection_for_copy = collection.clone();

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
            menu_item_with_shortcut("Open Forge", "Cmd+Alt+F")
                .on_click({
                    let state = state.clone();
                    let database = database.clone();
                    move |_, _window, cx| {
                        state.update(cx, |state, cx| {
                            state.open_forge_tab(connection_id, database.clone(), cx);
                        });
                    }
                })
                .action(Box::new(OpenForge)),
        )
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
        .item(
            menu_item_with_shortcut("Export...", "Cmd+Alt+E")
                .action(Box::new(TransferExport))
                .on_click({
                    let state = state.clone();
                    let database = database.clone();
                    let collection = collection.clone();
                    move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.open_transfer_tab_with_prefill(
                                connection_id,
                                database.clone(),
                                Some(collection.clone()),
                                TransferScope::Collection,
                                TransferMode::Export,
                                cx,
                            );
                        });
                    }
                }),
        )
        .item(
            menu_item_with_shortcut("Import...", "Cmd+Alt+I")
                .action(Box::new(TransferImport))
                .on_click({
                    let state = state.clone();
                    let database = database.clone();
                    let collection = collection.clone();
                    move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.open_transfer_tab_with_prefill(
                                connection_id,
                                database.clone(),
                                Some(collection.clone()),
                                TransferScope::Collection,
                                TransferMode::Import,
                                cx,
                            );
                        });
                    }
                }),
        )
        .item(
            menu_item_with_shortcut("Copy to...", "Cmd+Alt+C")
                .action(Box::new(TransferCopy))
                .on_click({
                    let state = state.clone();
                    let database = database.clone();
                    let collection = collection.clone();
                    move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.open_transfer_tab_with_prefill(
                                connection_id,
                                database.clone(),
                                Some(collection.clone()),
                                TransferScope::Collection,
                                TransferMode::Copy,
                                cx,
                            );
                        });
                    }
                }),
        )
        .separator()
        .item(menu_item_with_shortcut("Copy", "⌘C").action(Box::new(CopyTreeItem)).on_click({
            let state = state.clone();
            let database = database_for_copy.clone();
            let collection = collection_for_copy.clone();
            move |_, _window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(format!(
                    "{}/{}",
                    database, collection
                )));
                state.update(cx, |state, cx| {
                    state.copied_tree_item = Some(CopiedTreeItem::Collection {
                        connection_id,
                        database: database.clone(),
                        collection: collection.clone(),
                    });
                    state.set_status_message(Some(StatusMessage::info(format!(
                        "Copied collection: {}.{}",
                        database, collection
                    ))));
                    cx.notify();
                });
            }
        }))
        .when(state.read(_cx).copied_tree_item.is_some(), |menu: PopupMenu| {
            let dest_database = database_for_copy.clone();
            menu.item(
                menu_item_with_shortcut("Paste", "⌘V").action(Box::new(PasteTreeItem)).on_click({
                    let state = state.clone();
                    move |_, _window, cx| {
                        let copied = state.read(cx).copied_tree_item.clone();
                        let Some(item) = copied else {
                            return;
                        };

                        let source_connection_id = match &item {
                            CopiedTreeItem::Database { connection_id, .. } => *connection_id,
                            CopiedTreeItem::Collection { connection_id, .. } => *connection_id,
                        };

                        if state.read(cx).connection_by_id(source_connection_id).is_none() {
                            state.update(cx, |state, cx| {
                                state.set_status_message(Some(StatusMessage::error(
                                    "Source connection no longer exists",
                                )));
                                state.copied_tree_item = None;
                                cx.notify();
                            });
                            return;
                        }

                        state.update(cx, |state, cx| match item {
                            CopiedTreeItem::Database {
                                connection_id: src_conn,
                                database: src_db,
                            } => {
                                state.open_transfer_tab_for_paste(
                                    src_conn,
                                    src_db,
                                    None,
                                    Some(connection_id),
                                    Some(dest_database.clone()),
                                    TransferScope::Database,
                                    cx,
                                );
                            }
                            CopiedTreeItem::Collection {
                                connection_id: src_conn,
                                database: src_db,
                                collection: src_col,
                            } => {
                                state.open_transfer_tab_for_paste(
                                    src_conn,
                                    src_db,
                                    Some(src_col),
                                    Some(connection_id),
                                    Some(dest_database.clone()),
                                    TransferScope::Collection,
                                    cx,
                                );
                            }
                        });
                    }
                }),
            )
        })
        .item(PopupMenuItem::new("Copy Name").action(Box::new(CopySelectionName)).on_click({
            move |_, _window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(label_for_copy.clone()));
            }
        }));

    menu
}
