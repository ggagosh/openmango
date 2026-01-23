use gpui::prelude::{FluentBuilder as _, InteractiveElement as _, StatefulInteractiveElement as _};
use gpui::*;
use gpui_component::input::Input;
use gpui_component::menu::ContextMenuExt;
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::components::{ConnectionManager, TreeNodeId};
use crate::keyboard::{
    CloseSidebarSearch, CopyConnectionUri, CopySelectionName, DeleteSelection,
    DisconnectConnection, EditConnection, FindInSidebar, OpenSelection, OpenSelectionPreview,
    RenameCollection,
};
use crate::state::AppCommands;
use crate::theme::{borders, colors, sizing, spacing};

use super::super::menus::{build_collection_menu, build_connection_menu, build_database_menu};
use super::Sidebar;

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_ref = self.state.read(cx);
        let active_connections = state_ref.active_connections_snapshot();
        let connecting_id = self.model.connecting_connection;

        let state = self.state.clone();
        let state_for_add = state.clone();
        let state_for_manager = state.clone();
        let state_for_tree = self.state.clone();
        let sidebar_entity = cx.entity();
        let scroll_handle = self.scroll_handle.clone();

        let search_query = self.search_state.read(cx).value().to_string();
        let search_results = if self.model.search_open {
            self.search_results(&search_query, cx)
        } else {
            Vec::new()
        };
        self.model.update_search_selection(&search_query, search_results.len());

        div()
            .key_context("Sidebar")
            .flex()
            .flex_col()
            .w(sizing::sidebar_width())
            .min_w(sizing::sidebar_width())
            .flex_shrink_0()
            .h_full()
            .bg(colors::bg_sidebar())
            .border_r_1()
            .border_color(colors::border())
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
                    .border_color(colors::border())
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(colors::text_secondary())
                            .child("CONNECTIONS"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
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
                                    .hover(|s| s.bg(colors::bg_hover()))
                                    .text_color(colors::text_primary())
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
                                    .hover(|s| s.bg(colors::bg_hover()))
                                    .text_color(colors::text_primary())
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
                        .border_color(colors::border())
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
                                    .text_color(colors::text_muted())
                                    .child("Type to search databases")
                                    .into_any_element()
                            } else if search_results.is_empty() {
                                div()
                                    .text_xs()
                                    .text_color(colors::text_muted())
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
                                            .hover(|s| s.bg(colors::list_hover()))
                                            .cursor_pointer()
                                            .id(("sidebar-search-row", result.index))
                                            .when(is_selected, |s| s.bg(colors::list_selected()))
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .min_w(px(0.0))
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(colors::text_primary())
                                                            .truncate()
                                                            .child(database.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(colors::text_muted())
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
                    .overflow_y_scrollbar()
                    .child(if self.model.entries.is_empty() {
                        div()
                            .p(spacing::md())
                            .text_sm()
                            .text_color(colors::text_muted())
                            .child("No connections yet")
                            .into_any_element()
                    } else {
                        uniform_list("sidebar-rows", self.model.entries.len(), {
                            let state_clone = state_for_tree.clone();
                            let sidebar_entity = sidebar_entity.clone();
                            cx.processor(
                                move |sidebar,
                                      visible_range: std::ops::Range<usize>,
                                      _window,
                                      _cx| {
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
                                                            event.click_count() >= 2
                                                        });

                                                    if is_double_click {
                                                        // Double-click on connection to connect (expand only)
                                                        if is_connection {
                                                            let should_expand = if is_connecting
                                                            {
                                                                true
                                                            } else if is_connected {
                                                                !is_expanded
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
                                                            let should_expand = !is_expanded;
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
                                                                .conn
                                                                .active
                                                                .get(&connection_id)
                                                                .is_some_and(|conn| {
                                                                    !conn
                                                                        .collections
                                                                        .contains_key(db)
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
                                            .hover(|s| s.bg(colors::list_hover()))
                                            .when(selected, |s| s.bg(colors::list_selected()))
                                            .cursor_pointer()
                                            // Chevron for expandable items
                                            .when(is_folder, |this| {
                                                this.child(
                                                    Icon::new(if is_expanded {
                                                        IconName::ChevronDown
                                                    } else {
                                                        IconName::ChevronRight
                                                    })
                                                    .size(sizing::icon_sm())
                                                    .text_color(colors::text_muted()),
                                                )
                                            })
                                            // Spacer for non-folders (align with chevron)
                                            .when(!is_folder, |this| {
                                                this.child(div().w(sizing::icon_sm()))
                                            })
                                            // Connection: status dot + server icon
                                            .when(is_connection, |this| {
                                                this.child(
                                                    div()
                                                        .w(sizing::status_dot())
                                                        .h(sizing::status_dot())
                                                        .rounded_full()
                                                        .bg(if is_connected {
                                                            colors::status_success()
                                                        } else if is_connecting {
                                                            colors::status_warning()
                                                        } else {
                                                            colors::text_muted()
                                                        }),
                                                )
                                                .child(
                                                    Icon::new(IconName::Globe)
                                                        .size(sizing::icon_md())
                                                        .text_color(if is_connected {
                                                            colors::text_primary()
                                                        } else {
                                                            colors::text_secondary()
                                                        }),
                                                )
                                            })
                                            // Database: folder icon
                                            .when(is_database, |this| {
                                                this.child(
                                                    Icon::new(IconName::Folder)
                                                        .size(sizing::icon_md())
                                                        .text_color(colors::text_secondary()),
                                                )
                                            })
                                            // Collection: file icon
                                            .when(is_collection, |this| {
                                                this.child(
                                                    Icon::new(IconName::File)
                                                        .size(sizing::icon_md())
                                                        .text_color(colors::text_muted()),
                                                )
                                            })
                                            // Label
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(if selected {
                                                        colors::text_primary()
                                                    } else {
                                                        colors::text_secondary()
                                                    })
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .child(label.clone()),
                                            )
                                            .when(is_connecting || is_loading_db, |this| {
                                                this.child(Spinner::new().xsmall())
                                            });

                                        let row = row.context_menu({
                                            let menu_node_id = node_id.clone();
                                            let state = state_clone.clone();
                                            let sidebar_entity = sidebar_entity.clone();
                                            move |menu, _window, cx| {
                                                let menu = menu.action_context(menu_focus.clone());
                                                match menu_node_id.clone() {
                                                    TreeNodeId::Connection(connection_id) => {
                                                        build_connection_menu(
                                                            menu,
                                                            state.clone(),
                                                            sidebar_entity.clone(),
                                                            connection_id,
                                                            connecting_id,
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
                                                        cx,
                                                    ),
                                                }
                                            }
                                        });

                                        items.push(row);
                                    }

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
    }
}
