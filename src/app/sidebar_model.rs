use std::collections::HashSet;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::components::TreeNodeId;
use crate::models::{ActiveConnection, SavedConnection};

use super::search::SidebarEntry;

pub(crate) struct SidebarModel {
    pub(crate) connecting_connection: Option<Uuid>,
    pub(crate) loading_databases: HashSet<TreeNodeId>,
    pub(crate) expanded_nodes: HashSet<TreeNodeId>,
    pub(crate) selected_tree_id: Option<TreeNodeId>,
    pub(crate) entries: Vec<SidebarEntry>,
    pub(crate) search_open: bool,
    pub(crate) search_selected: Option<usize>,
    pub(crate) typeahead_query: String,
    pub(crate) typeahead_last: Option<Instant>,
}

impl SidebarModel {
    pub(crate) fn new(
        connections: Vec<SavedConnection>,
        active: std::collections::HashMap<Uuid, ActiveConnection>,
    ) -> Self {
        let entries = Self::build_entries(&connections, &active, &HashSet::new());
        Self {
            connecting_connection: None,
            loading_databases: HashSet::new(),
            expanded_nodes: HashSet::new(),
            selected_tree_id: None,
            entries,
            search_open: false,
            search_selected: None,
            typeahead_query: String::new(),
            typeahead_last: None,
        }
    }

    pub(crate) fn refresh_entries(
        &mut self,
        connections: &[SavedConnection],
        active: &std::collections::HashMap<Uuid, ActiveConnection>,
    ) -> Option<usize> {
        self.entries = Self::build_entries(connections, active, &self.expanded_nodes);
        self.selected_index()
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        let node_id = self.selected_tree_id.as_ref()?;
        self.entries.iter().position(|entry| &entry.id == node_id)
    }

    pub(crate) fn ensure_selection_from_state(
        &mut self,
        connection_id: Option<Uuid>,
        selected_db: Option<String>,
        selected_col: Option<String>,
    ) -> Option<usize> {
        let connection_id = connection_id?;
        if let Some(db) = selected_db.as_ref() {
            self.expanded_nodes.insert(TreeNodeId::connection(connection_id));
            if selected_col.is_some() {
                self.expanded_nodes.insert(TreeNodeId::database(connection_id, db));
            }
        }

        self.selected_tree_id = match (selected_db.as_ref(), selected_col.as_ref()) {
            (Some(db), Some(col)) => {
                Some(TreeNodeId::collection(connection_id, db.to_string(), col.to_string()))
            }
            (Some(db), None) => Some(TreeNodeId::database(connection_id, db.to_string())),
            _ => None,
        };

        self.selected_index()
    }

    pub(crate) fn open_search(&mut self) {
        self.search_open = true;
        self.typeahead_query.clear();
        self.typeahead_last = None;
        self.search_selected = Some(0);
    }

    pub(crate) fn close_search(&mut self) {
        self.search_open = false;
        self.search_selected = None;
    }

    pub(crate) fn update_search_selection(&mut self, query: &str, results_len: usize) {
        if !self.search_open {
            return;
        }
        if query.trim().is_empty() || results_len == 0 {
            self.search_selected = None;
        } else if self.search_selected.is_none_or(|ix| ix >= results_len) {
            self.search_selected = Some(0);
        }
    }

    pub(crate) fn move_search_selection(
        &mut self,
        delta: isize,
        results_len: usize,
    ) -> Option<usize> {
        if results_len == 0 {
            self.search_selected = None;
            return None;
        }
        let len = results_len as isize;
        let current = self.search_selected.unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.search_selected = Some(next);
        Some(next)
    }

    pub(crate) fn move_sidebar_selection(&mut self, delta: isize) -> Option<(usize, TreeNodeId)> {
        if self.entries.is_empty() {
            return None;
        }
        let current_index = self
            .selected_tree_id
            .as_ref()
            .and_then(|id| self.entries.iter().position(|entry| &entry.id == id))
            .unwrap_or(0);
        let len = self.entries.len() as isize;
        let next = (current_index as isize + delta).rem_euclid(len) as usize;
        let entry = &self.entries[next];
        self.selected_tree_id = Some(entry.id.clone());
        Some((next, entry.id.clone()))
    }

    pub(crate) fn handle_typeahead_key(&mut self, key: &str, key_char: Option<&str>) -> bool {
        if self.search_open {
            return false;
        }
        if key == "escape" {
            if !self.typeahead_query.is_empty() {
                self.typeahead_query.clear();
                return true;
            }
            return false;
        }
        if key == "backspace" {
            if !self.typeahead_query.is_empty() {
                self.typeahead_query.pop();
                self.typeahead_last = Some(Instant::now());
                return true;
            }
            return false;
        }
        let Some(key_char) = key_char else {
            return false;
        };
        if key_char.chars().count() != 1 {
            return false;
        }
        let now = Instant::now();
        if self
            .typeahead_last
            .is_none_or(|last| now.duration_since(last) > Duration::from_millis(1000))
        {
            self.typeahead_query.clear();
        }
        self.typeahead_last = Some(now);
        self.typeahead_query.push_str(&key_char.to_lowercase());
        true
    }

    pub(crate) fn select_typeahead_match(&mut self) -> Option<(usize, TreeNodeId)> {
        let query = self.typeahead_query.trim();
        if query.is_empty() {
            return None;
        }
        let query = query.to_lowercase();
        if self.entries.is_empty() {
            return None;
        }
        let start = self
            .selected_tree_id
            .as_ref()
            .and_then(|id| self.entries.iter().position(|entry| &entry.id == id))
            .map(|ix| ix + 1)
            .unwrap_or(0);

        for offset in 0..self.entries.len() {
            let idx = (start + offset) % self.entries.len();
            let entry = &self.entries[idx];
            if entry.label.to_lowercase().starts_with(&query) {
                self.selected_tree_id = Some(entry.id.clone());
                return Some((idx, entry.id.clone()));
            }
        }
        None
    }

    pub(crate) fn build_entries(
        connections: &[SavedConnection],
        active: &std::collections::HashMap<Uuid, ActiveConnection>,
        expanded: &HashSet<TreeNodeId>,
    ) -> Vec<SidebarEntry> {
        let mut items = Vec::new();
        for conn in connections {
            let conn_node_id = TreeNodeId::connection(conn.id);
            let conn_expanded = expanded.contains(&conn_node_id);
            let active_conn = active.get(&conn.id);
            let conn_is_folder = active_conn.is_some();
            items.push(SidebarEntry {
                id: conn_node_id,
                label: conn.name.clone(),
                depth: 0,
                is_folder: conn_is_folder,
                is_expanded: conn_expanded,
            });

            if let Some(active_conn) = active_conn
                && conn_expanded
            {
                for db_name in &active_conn.databases {
                    let db_node_id = TreeNodeId::database(conn.id, db_name);
                    let db_expanded = expanded.contains(&db_node_id);
                    items.push(SidebarEntry {
                        id: db_node_id.clone(),
                        label: db_name.clone(),
                        depth: 1,
                        is_folder: true,
                        is_expanded: db_expanded,
                    });

                    if db_expanded && let Some(collections) = active_conn.collections.get(db_name) {
                        for col_name in collections {
                            let col_node_id = TreeNodeId::collection(conn.id, db_name, col_name);
                            items.push(SidebarEntry {
                                id: col_node_id,
                                label: col_name.clone(),
                                depth: 2,
                                is_folder: false,
                                is_expanded: false,
                            });
                        }
                    }
                }
            }
        }

        items
    }
}
