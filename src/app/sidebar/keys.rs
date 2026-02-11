use gpui::*;

use crate::models::TreeNodeId;
use crate::state::AppCommands;

use super::Sidebar;

impl Sidebar {
    pub(super) fn handle_sidebar_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.to_lowercase();
        if !self.model.search_open {
            match key.as_str() {
                "up" | "arrowup" => {
                    self.move_sidebar_selection(-1, cx);
                    return true;
                }
                "down" | "arrowdown" => {
                    self.move_sidebar_selection(1, cx);
                    return true;
                }
                "left" | "arrowleft" => {
                    if let Some(node_id) = self.model.selected_tree_id.clone() {
                        if self.model.expanded_nodes.contains(&node_id) {
                            self.model.expanded_nodes.remove(&node_id);
                            self.persist_expanded_nodes(cx);
                            self.refresh_tree(cx);
                        } else if let TreeNodeId::Database { connection, database: _ } = node_id {
                            let parent = TreeNodeId::connection(connection);
                            self.model.selected_tree_id = Some(parent.clone());
                            self.scroll_handle.scroll_to_item(
                                self.model
                                    .entries
                                    .iter()
                                    .position(|entry| entry.id == parent)
                                    .unwrap_or(0),
                                gpui::ScrollStrategy::Center,
                            );
                            cx.notify();
                        } else if let TreeNodeId::Collection { connection, database, .. } = node_id
                        {
                            let parent = TreeNodeId::database(connection, database.clone());
                            self.model.selected_tree_id = Some(parent.clone());
                            self.scroll_handle.scroll_to_item(
                                self.model
                                    .entries
                                    .iter()
                                    .position(|entry| entry.id == parent)
                                    .unwrap_or(0),
                                gpui::ScrollStrategy::Center,
                            );
                            cx.notify();
                        }
                    }
                    return true;
                }
                "right" | "arrowright" => {
                    if let Some(node_id) = self.model.selected_tree_id.clone() {
                        if node_id.is_connection() {
                            if !self.model.expanded_nodes.contains(&node_id) {
                                self.model.expanded_nodes.insert(node_id.clone());
                                self.persist_expanded_nodes(cx);
                                self.refresh_tree(cx);
                            }
                        } else if let TreeNodeId::Database { connection, database } =
                            node_id.clone()
                        {
                            if !self.model.expanded_nodes.contains(&node_id) {
                                self.model.expanded_nodes.insert(node_id.clone());
                                self.persist_expanded_nodes(cx);
                                self.refresh_tree(cx);
                            }
                            let should_load = self
                                .state
                                .read(cx)
                                .active_connection_by_id(connection)
                                .is_some_and(|conn| !conn.collections.contains_key(&database));
                            if should_load && !self.model.loading_databases.contains(&node_id) {
                                self.model.loading_databases.insert(node_id.clone());
                                cx.notify();
                                AppCommands::load_collections(
                                    self.state.clone(),
                                    connection,
                                    database,
                                    cx,
                                );
                            }
                        }
                    }
                    return true;
                }
                _ => {}
            }
        }
        self.handle_typeahead_key(event, cx);
        false
    }
}
