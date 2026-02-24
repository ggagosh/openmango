use gpui::prelude::{FluentBuilder as _, InteractiveElement as _, StatefulInteractiveElement as _};
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::Input;
use gpui_component::menu::{ContextMenuExt, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::components::ConnectionManager;
use crate::keyboard::{
    CloseSidebarSearch, CopyConnectionUri, CopySelectionName, CopyTreeItem, DeleteSelection,
    DisconnectConnection, EditConnection, FindInSidebar, OpenSelection, OpenSelectionPreview,
    PasteTreeItem, RenameCollection, TransferCopy, TransferExport, TransferImport,
};
use crate::models::TreeNodeId;
use crate::state::{AppCommands, TransferMode};
use crate::theme::{borders, sizing, spacing};

use super::super::menus::{build_collection_menu, build_connection_menu, build_database_menu};
use super::super::sidebar_model::SidebarModel;
use super::Sidebar;

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_ref = self.state.read(cx);
        let active_connections = state_ref.active_connections_snapshot();
        let connecting_id = self.model.connecting_connection;

        // Collect disconnected connections for the connect dropdown
        let disconnected_connections: Vec<_> = state_ref
            .connections_snapshot()
            .into_iter()
            .filter(|c| !active_connections.contains_key(&c.id))
            .collect();

        let state = self.state.clone();
        let state_for_add = state.clone();
        let state_for_manager = state.clone();
        let state_for_connect = state.clone();
        let state_for_tree = self.state.clone();
        let sidebar_entity = cx.entity();
        let scroll_handle = self.scroll_handle.clone();

        // Sticky connection header (one-frame-delayed: uses index computed by previous processor run)
        let sticky_info = self.sticky_connection_index.and_then(|idx| {
            let entry = self.model.entries.get(idx)?;
            let connection_id = entry.id.connection_id();
            let is_connected = active_connections.contains_key(&connection_id);
            let is_connecting = connecting_id == Some(connection_id);
            Some((idx, entry.label.clone(), connection_id, is_connected, is_connecting))
        });

        let search_query = self.search_state.read(cx).value().to_string();
        let search_results = if self.model.search_open {
            self.search_results(&search_query, cx)
        } else {
            Vec::new()
        };
        self.model.update_search_selection(&search_query, search_results.len());

        let sidebar_w = self.width();

        div()
            .key_context("Sidebar")
            .flex()
            .flex_col()
            .w(sidebar_w)
            .min_w(sidebar_w)
            .flex_shrink_0()
            .h_full()
            .overflow_hidden()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .track_focus(&self.focus_handle)
            .on_mouse_down(MouseButton::Left, {
                let focus_handle = self.focus_handle.clone();
                move |_, window, _cx| {
                    window.focus(&focus_handle);
                }
            })
            .on_key_down({
                let sidebar_entity = sidebar_entity.clone();
                move |event: &KeyDownEvent, _window: &mut Window, cx: &mut App| {
                    sidebar_entity.update(cx, |sidebar, cx| {
                        if sidebar.handle_sidebar_key(event, cx) {
                            cx.stop_propagation();
                        }
                    });
                }
            })
            .on_action(cx.listener(|this, _: &OpenSelection, window, cx| {
                this.handle_open_selection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSelectionPreview, _window, cx| {
                this.handle_open_preview(cx);
            }))
            .on_action(cx.listener(|this, _: &EditConnection, window, cx| {
                this.handle_edit_connection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DisconnectConnection, _window, cx| {
                this.handle_disconnect_connection(cx);
            }))
            .on_action(cx.listener(|this, _: &CopySelectionName, _window, cx| {
                this.handle_copy_selection_name(cx);
            }))
            .on_action(cx.listener(|this, _: &CopyConnectionUri, _window, cx| {
                this.handle_copy_connection_uri(cx);
            }))
            .on_action(cx.listener(|this, _: &RenameCollection, window, cx| {
                this.handle_rename_collection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteSelection, window, cx| {
                this.handle_delete_selection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &TransferExport, _window, cx| {
                this.handle_transfer_action(TransferMode::Export, cx);
            }))
            .on_action(cx.listener(|this, _: &TransferImport, _window, cx| {
                this.handle_transfer_action(TransferMode::Import, cx);
            }))
            .on_action(cx.listener(|this, _: &TransferCopy, _window, cx| {
                this.handle_transfer_action(TransferMode::Copy, cx);
            }))
            .on_action(cx.listener(|this, _: &CopyTreeItem, _window, cx| {
                this.handle_copy_tree_item(cx);
            }))
            .on_action(cx.listener(|this, _: &PasteTreeItem, _window, cx| {
                this.handle_paste_tree_item(cx);
            }))
            .on_action(cx.listener(|this, _: &FindInSidebar, window, cx| {
                this.open_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseSidebarSearch, window, cx| {
                this.close_search(window, cx);
            }))
            .child(
                // Header with "+" button
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(spacing::md())
                    .h(sizing::header_height())
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(cx.theme().secondary_foreground)
                            .child("CONNECTIONS"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child({
                                let sidebar_entity = sidebar_entity.clone();
                                // Connect dropdown button
                                Button::new("connect-dropdown-btn")
                                    .icon(Icon::new(IconName::Globe).xsmall())
                                    .ghost()
                                    .xsmall()
                                    .dropdown_menu(move |mut menu: PopupMenu, _window, _cx| {
                                        if disconnected_connections.is_empty() {
                                            menu = menu.item(
                                                PopupMenuItem::new("All connected").disabled(true),
                                            );
                                        } else {
                                            for conn in &disconnected_connections {
                                                let conn_id = conn.id;
                                                let state = state_for_connect.clone();
                                                let sidebar_entity = sidebar_entity.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(conn.name.clone())
                                                        .on_click(move |_, _window, cx| {
                                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                                sidebar.expand_connection_and_refresh(conn_id, cx);
                                                            });
                                                            AppCommands::connect(
                                                                state.clone(),
                                                                conn_id,
                                                                cx,
                                                            );
                                                        }),
                                                );
                                            }
                                        }
                                        menu
                                    })
                            })
                            .child(
                                // Add button
                                div()
                                    .id("add-connection-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(sizing::icon_lg())
                                    .h(sizing::icon_lg())
                                    .rounded(crate::theme::borders::radius_sm())
                                    .cursor_pointer()
                                    .hover(|s| s.bg(cx.theme().list_hover))
                                    .text_color(cx.theme().foreground)
                                    .child(Icon::new(IconName::Plus).xsmall())
                                    .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        Sidebar::open_add_dialog(state_for_add.clone(), window, cx);
                                    }),
                            )
                            .child(
                                // Manager button
                                div()
                                    .id("manage-connections-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(sizing::icon_lg())
                                    .h(sizing::icon_lg())
                                    .rounded(crate::theme::borders::radius_sm())
                                    .cursor_pointer()
                                    .hover(|s| s.bg(cx.theme().list_hover))
                                    .text_color(cx.theme().foreground)
                                    .child(Icon::new(IconName::Settings).xsmall())
                                    .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        ConnectionManager::open(state_for_manager.clone(), window, cx);
                                    }),
                            )
                    ),
            )
            .child(
                if self.model.search_open {
                    let sidebar_entity = sidebar_entity.clone();
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .px(spacing::md())
                        .py(spacing::xs())
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .capture_key_down({
                                    let sidebar_entity = sidebar_entity.clone();
                                    move |event: &KeyDownEvent,
                                          window: &mut Window,
                                          cx: &mut App| {
                                        let key = event.keystroke.key.to_lowercase();
                                        if key == "escape" {
                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                sidebar.close_search(window, cx);
                                            });
                                            cx.stop_propagation();
                                            return;
                                        }
                                        if key == "down" || key == "arrowdown" {
                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                sidebar.move_search_selection(1, cx);
                                            });
                                            cx.stop_propagation();
                                            return;
                                        }
                                        if key == "up" || key == "arrowup" {
                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                sidebar.move_search_selection(-1, cx);
                                            });
                                            cx.stop_propagation();
                                            return;
                                        }
                                        if key == "enter" || key == "return" {
                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                let query =
                                                    sidebar.search_state.read(cx).value().to_string();
                                                let results = sidebar.search_results(&query, cx);
                                                let selection = sidebar.model.search_selected;
                                                let result = selection
                                                    .and_then(|ix| results.get(ix))
                                                    .or_else(|| results.first());
                                                if let Some(result) = result {
                                                    sidebar.select_search_result(result, window, cx);
                                                }
                                            });
                                            cx.stop_propagation();
                                        }
                                    }
                                })
                                .child(Input::new(&self.search_state).w_full()),
                        )
                        .child({
                            if search_query.trim().is_empty() {
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Type to search databases")
                                    .into_any_element()
                            } else if search_results.is_empty() {
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("No matches")
                                    .into_any_element()
                            } else {
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .children(search_results.iter().enumerate().map(|(ix, result)| {
                                        let result = result.clone();
                                        let database = result.database.clone();
                                        let connection_name = result.connection_name.clone();
                                        let sidebar_entity = sidebar_entity.clone();
                                        let is_selected = self.model.search_selected == Some(ix);
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .px(spacing::sm())
                                            .py(px(4.0))
                                            .rounded(borders::radius_sm())
                                            .hover(|s| s.bg(cx.theme().list_hover))
                                            .cursor_pointer()
                                            .id(("sidebar-search-row", result.index))
                                            .when(is_selected, |s| s.bg(cx.theme().list_active))
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .min_w(px(0.0))
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(cx.theme().foreground)
                                                            .truncate()
                                                            .child(database.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(cx.theme().muted_foreground)
                                                            .truncate()
                                                            .child(connection_name),
                                                    ),
                                            )
                                            .on_click(move |_: &ClickEvent,
                                                           window: &mut Window,
                                                           cx: &mut App| {
                                                let result = result.clone();
                                                sidebar_entity.update(cx, |sidebar, cx| {
                                                    sidebar.select_search_result(&result, window, cx);
                                                });
                                            })
                                    }))
                                    .into_any_element()
                            }
                        })
                        .into_any_element()
                } else {
                    div().into_any_element()
                },
            )
            .child(
                // Connection tree
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .relative()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .overflow_y_scrollbar()
                            .child(if self.model.entries.is_empty() {
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .flex_1()
                            .gap(spacing::sm())
                            .p(spacing::lg())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("No active connections"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .text_center()
                                    .child("Use the connect button or Cmd+K to connect"),
                            )
                            .into_any_element()
                    } else {
                        // Extract theme colors before the processor closure to avoid
                        // capturing `cx` inside the move closure.
                        let theme_list_hover = cx.theme().list_hover;
                        let theme_list_active = cx.theme().list_active;
                        let theme_muted_foreground = cx.theme().muted_foreground;
                        let theme_foreground = cx.theme().foreground;
                        let theme_secondary_foreground = cx.theme().secondary_foreground;
                        let theme_primary = cx.theme().primary;
                        let theme_info = cx.theme().info;
                        let theme_warning = cx.theme().warning;
                        uniform_list("sidebar-rows", self.model.entries.len(), {
                            let state_clone = state_for_tree.clone();
                            let sidebar_entity = sidebar_entity.clone();
                            cx.processor(
                                move |sidebar,
                                      visible_range: std::ops::Range<usize>,
                                      _window,
                                      _cx| {
                                    let visible_start = visible_range.start;
                                    let mut items = Vec::with_capacity(visible_range.len());
                                    let active_connections = active_connections.clone();
                                    let connecting_id = connecting_id;

                                    for ix in visible_range {
                                        let Some(entry) = sidebar.model.entries.get(ix) else {
                                            continue;
                                        };
                                        let node_id = entry.id.clone();
                                        let depth = entry.depth;
                                        let is_folder = entry.is_folder;
                                        let is_expanded = entry.is_expanded;
                                        let label = entry.label.clone();
                                        let label_for_menu = label.clone();

                                        let is_connection = node_id.is_connection();
                                        let is_database = node_id.is_database();
                                        let is_collection = node_id.is_collection();

                                        let connection_id = node_id.connection_id();
                                        let is_connected = is_connection
                                            && active_connections.contains_key(&connection_id);
                                        let is_connecting =
                                            is_connection && connecting_id == Some(connection_id);
                                        let is_loading_db =
                                            is_database && sidebar.model.loading_databases.contains(&node_id);

                                        let db_name =
                                            node_id.database_name().map(|db| db.to_string());
                                        let selected_db = db_name.clone();
                                        let selected_col =
                                            node_id.collection_name().map(|col| col.to_string());

                                        let selected =
                                            sidebar.model.selected_tree_id.as_ref() == Some(&node_id);
                                        let menu_focus = sidebar.focus_handle.clone();
                                        let row_focus = menu_focus.clone();

                                        let row = div()
                                            .id(("sidebar-row", ix))
                                            .flex()
                                            .items_center()
                                            .w_full()
                                            .overflow_hidden()
                                            .gap(px(4.0))
                                            .pl(px(8.0 + 12.0 * depth as f32))
                                            .py(px(2.0))
                                            .on_click({
                                                let node_id = node_id.clone();
                                                let state_clone = state_clone.clone();
                                                let db_name = db_name.clone();
                                                let selected_db = selected_db.clone();
                                                let selected_col = selected_col.clone();
                                                let sidebar_entity = sidebar_entity.clone();
                                                move |event: &ClickEvent,
                                                      _window: &mut Window,
                                                      cx: &mut App| {
                                                    _window.focus(&row_focus);
                                                    cx.stop_propagation();

                                                    state_clone.update(cx, |state, cx| {
                                                        state.select_connection(
                                                            Some(connection_id),
                                                            cx,
                                                        );
                                                    });

                                                    let is_double_click =
                                                        sidebar_entity.update(cx, |sidebar, cx| {
                                                            sidebar.model.selected_tree_id =
                                                                Some(node_id.clone());
                                                            cx.notify();
                                                            event.click_count() == 2
                                                        });

                                                    if is_double_click {
                                                        // Double-click on connection to connect (expand only)
                                                        if is_connection {
                                                            // Read live state instead of stale closure capture
                                                            let currently_expanded = sidebar_entity
                                                                .update(cx, |sidebar, _cx| {
                                                                    sidebar.model.expanded_nodes.contains(&node_id)
                                                                });
                                                            let should_expand = if is_connecting
                                                            {
                                                                true
                                                            } else if is_connected {
                                                                !currently_expanded
                                                            } else {
                                                                true
                                                            };
                                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                                if should_expand {
                                                                    sidebar
                                                                        .model
                                                                        .expanded_nodes
                                                                        .insert(node_id.clone());
                                                                } else {
                                                                    sidebar
                                                                        .model
                                                                        .expanded_nodes
                                                                        .remove(&node_id);
                                                                }
                                                                sidebar.persist_expanded_nodes(cx);
                                                                sidebar.refresh_tree(cx);
                                                            });

                                                            if is_connected || is_connecting {
                                                                return;
                                                            }
                                                            AppCommands::connect(
                                                                state_clone.clone(),
                                                                node_id.connection_id(),
                                                                cx,
                                                            );
                                                        }
                                                        // Double-click on database to load collections (expand only)
                                                        else if is_database
                                                            && let Some(ref db) = db_name
                                                        {
                                                            // Read live state instead of stale closure capture
                                                            let currently_expanded = sidebar_entity
                                                                .update(cx, |sidebar, _cx| {
                                                                    sidebar.model.expanded_nodes.contains(&node_id)
                                                                });
                                                            let should_expand = !currently_expanded;
                                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                                if should_expand {
                                                                    sidebar
                                                                        .model
                                                                        .expanded_nodes
                                                                        .insert(node_id.clone());
                                                                } else {
                                                                    sidebar
                                                                        .model
                                                                        .expanded_nodes
                                                                        .remove(&node_id);
                                                                }
                                                                sidebar.persist_expanded_nodes(cx);
                                                                sidebar.refresh_tree(cx);
                                                            });

                                                            if !should_expand || is_loading_db {
                                                                return;
                                                            }
                                                            let should_load = state_clone
                                                                .read(cx)
                                                                .active_connection_by_id(connection_id)
                                                                .is_some_and(|conn| {
                                                                    !conn.collections.contains_key(db)
                                                                });
                                                            if should_load {
                                                                sidebar_entity.update(
                                                                    cx,
                                                                    |sidebar, cx| {
                                                                        sidebar
                                                                            .model
                                                                            .loading_databases
                                                                            .insert(node_id.clone());
                                                                        cx.notify();
                                                                    },
                                                                );
                                                                AppCommands::load_collections(
                                                                    state_clone.clone(),
                                                                    connection_id,
                                                                    db.clone(),
                                                                    cx,
                                                                );
                                                            }
                                                        }
                                                        // Double-click on collection to open
                                                        else if is_collection
                                                            && let (Some(db), Some(col)) =
                                                                (&selected_db, &selected_col)
                                                        {
                                                            state_clone.update(cx, |state, cx| {
                                                                state.select_collection(
                                                                    db.clone(),
                                                                    col.clone(),
                                                                    cx,
                                                                );
                                                            });
                                                        }
                                                    }
                                                    // Single click: select database or preview collection
                                                    else if is_database
                                                        && let Some(db) = &db_name
                                                    {
                                                        state_clone.update(cx, |state, cx| {
                                                            state.select_database(db.clone(), cx);
                                                        });
                                                    } else if is_collection
                                                        && let (Some(db), Some(col)) =
                                                            (&selected_db, &selected_col)
                                                    {
                                                        state_clone.update(cx, |state, cx| {
                                                            state.preview_collection(
                                                                db.clone(),
                                                                col.clone(),
                                                                cx,
                                                            );
                                                        });
                                                    }
                                                }
                                            })
                                            .hover(|s| s.bg(theme_list_hover))
                                            .when(selected, |s| s.bg(theme_list_active))
                                            .cursor_pointer()
                                            // Chevron for expandable items — single-click to toggle
                                            .when(is_folder, |this| {
                                                let chevron_node_id = node_id.clone();
                                                let chevron_sidebar = sidebar_entity.clone();
                                                let chevron_state = state_clone.clone();
                                                let chevron_db = db_name.clone();
                                                let chevron_connection_id = connection_id;
                                                let chevron_is_connection = is_connection;
                                                let chevron_is_database = is_database;
                                                let chevron_is_loading = is_loading_db;
                                                this.child(
                                                    div()
                                                        .id(("chevron", ix))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .size(px(18.0))
                                                        .rounded(px(4.0))
                                                        .cursor_pointer()
                                                        .hover(|s| s.bg(theme_foreground.opacity(0.1)))
                                                        .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                                                            cx.stop_propagation();

                                                            let currently_expanded = chevron_sidebar
                                                                .update(cx, |sidebar, _cx| {
                                                                    sidebar.model.expanded_nodes.contains(&chevron_node_id)
                                                                });
                                                            let should_expand = !currently_expanded;
                                                            chevron_sidebar.update(cx, |sidebar, cx| {
                                                                if should_expand {
                                                                    sidebar.model.expanded_nodes.insert(chevron_node_id.clone());
                                                                } else {
                                                                    sidebar.model.expanded_nodes.remove(&chevron_node_id);
                                                                }
                                                                sidebar.persist_expanded_nodes(cx);
                                                                sidebar.refresh_tree(cx);
                                                            });

                                                            // For connections: connect if not connected
                                                            if chevron_is_connection && should_expand {
                                                                let is_connected = chevron_state.read(cx)
                                                                    .active_connection_by_id(chevron_connection_id)
                                                                    .is_some();
                                                                if !is_connected {
                                                                    AppCommands::connect(
                                                                        chevron_state.clone(),
                                                                        chevron_connection_id,
                                                                        cx,
                                                                    );
                                                                }
                                                            }

                                                            // For databases: load collections if needed
                                                            if chevron_is_database && should_expand && !chevron_is_loading
                                                                && let Some(ref db) = chevron_db
                                                            {
                                                                let should_load = chevron_state
                                                                    .read(cx)
                                                                    .active_connection_by_id(chevron_connection_id)
                                                                    .is_some_and(|conn| {
                                                                        !conn.collections.contains_key(db)
                                                                    });
                                                                if should_load {
                                                                    chevron_sidebar.update(cx, |sidebar, cx| {
                                                                        sidebar.model.loading_databases.insert(chevron_node_id.clone());
                                                                        cx.notify();
                                                                    });
                                                                    AppCommands::load_collections(
                                                                        chevron_state.clone(),
                                                                        chevron_connection_id,
                                                                        db.clone(),
                                                                        cx,
                                                                    );
                                                                }
                                                            }
                                                        })
                                                        .child(
                                                            Icon::new(if is_expanded {
                                                                IconName::ChevronDown
                                                            } else {
                                                                IconName::ChevronRight
                                                            })
                                                            .size(sizing::icon_sm())
                                                            .text_color(theme_muted_foreground),
                                                        ),
                                                )
                                            })
                                            // Spacer for non-folders (align with chevron)
                                            .when(!is_folder, |this| {
                                                this.child(div().w(sizing::icon_sm()))
                                            })
                                            // Connection: server icon (green)
                                            .when(is_connection, |this| {
                                                this.child(
                                                    Icon::new(IconName::Globe)
                                                        .size(sizing::icon_md())
                                                        .text_color(theme_primary),
                                                )
                                            })
                                            // Database: dashboard icon (blue)
                                            .when(is_database, |this| {
                                                this.child(
                                                    Icon::new(IconName::LayoutDashboard)
                                                        .size(sizing::icon_md())
                                                        .text_color(theme_info),
                                                )
                                            })
                                            // Collection: braces icon (amber)
                                            .when(is_collection, |this| {
                                                this.child(
                                                    Icon::new(IconName::Braces)
                                                        .size(sizing::icon_md())
                                                        .text_color(theme_warning),
                                                )
                                            })
                                            // Label
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .text_sm()
                                                    .text_color(if selected {
                                                        theme_foreground
                                                    } else {
                                                        theme_secondary_foreground
                                                    })
                                                    .truncate()
                                                    .child(label.clone()),
                                            )
                                            .when(is_connecting || is_loading_db, |this| {
                                                this.child(Spinner::new().xsmall())
                                            });

                                        let row = row;

                                        let row = row.context_menu({
                                            let menu_node_id = node_id.clone();
                                            let state = state_clone.clone();
                                            let sidebar_entity = sidebar_entity.clone();
                                            move |menu, window, cx| {
                                                let menu = menu.action_context(menu_focus.clone());
                                                match menu_node_id.clone() {
                                                    TreeNodeId::Connection(connection_id) => {
                                                        build_connection_menu(
                                                            menu,
                                                            state.clone(),
                                                            sidebar_entity.clone(),
                                                            connection_id,
                                                            connecting_id,
                                                            window,
                                                            cx,
                                                        )
                                                    }
                                                    TreeNodeId::Database {
                                                        connection,
                                                        database,
                                                    } => {
                                                        let node_id = TreeNodeId::database(
                                                            connection,
                                                            database.clone(),
                                                        );
                                                        build_database_menu(
                                                            menu,
                                                            state.clone(),
                                                            sidebar_entity.clone(),
                                                            node_id,
                                                            database.clone(),
                                                            is_loading_db,
                                                            window,
                                                            cx,
                                                        )
                                                    }
                                                    TreeNodeId::Collection {
                                                        connection,
                                                        database,
                                                        collection,
                                                    } => build_collection_menu(
                                                        menu,
                                                        state.clone(),
                                                        connection,
                                                        database.clone(),
                                                        collection.clone(),
                                                        label_for_menu.clone(),
                                                        window,
                                                        cx,
                                                    ),
                                                }
                                            }
                                        });

                                        items.push(row);
                                    }

                                    // Compute sticky connection header
                                    sidebar.sticky_connection_index =
                                        if !sidebar.model.entries.is_empty()
                                            && visible_start > 0
                                        {
                                            SidebarModel::find_parent_connection_index(
                                                &sidebar.model.entries,
                                                visible_start,
                                            )
                                            .filter(|&idx| idx < visible_start)
                                        } else {
                                            None
                                        };

                                    items
                                },
                            )
                        })
                        .flex_grow()
                        .size_full()
                        .track_scroll(scroll_handle)
                        .with_sizing_behavior(ListSizingBehavior::Auto)
                        .into_any_element()
                    }),
                    )
                    // Sticky connection header overlay
                    .when_some(sticky_info, |this, (idx, label, _connection_id, _is_connected, _is_connecting)| {
                        let scroll_handle = self.scroll_handle.clone();
                        let sidebar_entity = sidebar_entity.clone();
                        this.child(
                            div()
                                .id("sticky-connection-header")
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .pl(px(8.0))
                                .py(px(2.0))
                                .bg(cx.theme().sidebar)
                                .border_b_1()
                                .border_color(cx.theme().border)
                                .cursor_pointer()
                                .hover(|s| s.bg(cx.theme().list_hover))
                                .on_click(move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                    scroll_handle.scroll_to_item(idx, gpui::ScrollStrategy::Top);
                                    sidebar_entity.update(cx, |_sidebar, cx| {
                                        cx.notify();
                                    });
                                })
                                // Globe icon
                                .child(
                                    Icon::new(IconName::Globe)
                                        .size(sizing::icon_md())
                                        .text_color(cx.theme().foreground),
                                )
                                // Label
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_sm()
                                        .text_color(cx.theme().foreground)
                                        .truncate()
                                        .child(label),
                                ),
                        )
                    }),
            )
    }
}
