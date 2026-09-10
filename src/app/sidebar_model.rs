use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::models::TreeNodeId;
use crate::models::{ActiveConnection, SavedConnection};

use super::search::{SidebarEntry, ranked_match_score};

pub(crate) const TYPEAHEAD_RESET_DELAY: Duration = Duration::from_millis(1100);

pub(crate) struct SidebarModel {
    pub(crate) connecting_connection: Option<Uuid>,
    pub(crate) loading_databases: HashSet<TreeNodeId>,
    pub(crate) expanded_nodes: HashSet<TreeNodeId>,
    pub(crate) selected_tree_id: Option<TreeNodeId>,
    selected_index: Option<usize>,
    entry_index_by_id: HashMap<TreeNodeId, usize>,
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
        let entry_index_by_id = Self::build_index(&entries);
        Self {
            connecting_connection: None,
            loading_databases: HashSet::new(),
            expanded_nodes: HashSet::new(),
            selected_tree_id: None,
            selected_index: None,
            entry_index_by_id,
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
        self.rebuild_index();
        self.sync_selected_index();
        self.selected_index
    }

    pub(crate) fn index_of(&self, node_id: &TreeNodeId) -> Option<usize> {
        self.entry_index_by_id.get(node_id).copied()
    }

    pub(crate) fn select_node(&mut self, node_id: TreeNodeId) -> Option<usize> {
        let index = self.index_of(&node_id)?;
        self.selected_tree_id = Some(node_id);
        self.selected_index = Some(index);
        Some(index)
    }

    pub(crate) fn clear_selection(&mut self) {
        self.selected_tree_id = None;
        self.selected_index = None;
    }

    pub(crate) fn select_index(&mut self, index: usize) -> Option<(usize, TreeNodeId)> {
        let entry = self.entries.get(index)?;
        self.selected_tree_id = Some(entry.id.clone());
        self.selected_index = Some(index);
        Some((index, entry.id.clone()))
    }

    pub(crate) fn select_first(&mut self) -> Option<(usize, TreeNodeId)> {
        self.select_index(0)
    }

    pub(crate) fn select_last(&mut self) -> Option<(usize, TreeNodeId)> {
        self.entries.len().checked_sub(1).and_then(|index| self.select_index(index))
    }

    pub(crate) fn move_sidebar_page(
        &mut self,
        delta: isize,
        page_size: usize,
    ) -> Option<(usize, TreeNodeId)> {
        if self.entries.is_empty() {
            return None;
        }
        let current_index = self.selected_index.unwrap_or(0).min(self.entries.len() - 1);
        let page_size = page_size.max(1) as isize;
        let next = (current_index as isize + delta * page_size)
            .clamp(0, self.entries.len().saturating_sub(1) as isize) as usize;
        self.select_index(next)
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

        self.sync_selected_index();
        self.selected_index
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
        let current_index = self.selected_index.unwrap_or(0).min(self.entries.len() - 1);
        let len = self.entries.len() as isize;
        let next = (current_index as isize + delta).rem_euclid(len) as usize;
        self.select_index(next)
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
        if key == "backspace" || key == "delete" {
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
        if self.typeahead_last.is_none_or(|last| now.duration_since(last) > TYPEAHEAD_RESET_DELAY) {
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

        let best = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| {
                ranked_match_score(&query, &entry.search_label).map(|score| (idx, score))
            })
            .min_by(|(left_idx, left_score), (right_idx, right_score)| {
                left_score
                    .cmp(right_score)
                    .then_with(|| {
                        let left_selected = self.selected_index == Some(*left_idx);
                        let right_selected = self.selected_index == Some(*right_idx);
                        right_selected.cmp(&left_selected)
                    })
                    .then_with(|| {
                        self.entries[*left_idx]
                            .label
                            .len()
                            .cmp(&self.entries[*right_idx].label.len())
                    })
                    .then_with(|| {
                        self.entries[*left_idx].label.cmp(&self.entries[*right_idx].label)
                    })
            })?;

        let entry = &self.entries[best.0];
        self.selected_tree_id = Some(entry.id.clone());
        self.selected_index = Some(best.0);
        Some((best.0, entry.id.clone()))
    }

    pub(crate) fn find_parent_connection_index(
        entries: &[SidebarEntry],
        from: usize,
    ) -> Option<usize> {
        (0..=from).rev().find(|&i| entries[i].depth == 0)
    }

    pub(crate) fn build_entries(
        connections: &[SavedConnection],
        active: &std::collections::HashMap<Uuid, ActiveConnection>,
        expanded: &HashSet<TreeNodeId>,
    ) -> Vec<SidebarEntry> {
        let mut items = Vec::new();
        for conn in connections {
            let active_conn = active.get(&conn.id);
            let conn_node_id = TreeNodeId::connection(conn.id);
            let conn_expanded = active_conn.is_some() && expanded.contains(&conn_node_id);
            items.push(SidebarEntry::new(
                conn_node_id,
                conn.name.clone(),
                0,
                active_conn.is_some(),
                conn_expanded,
            ));

            if let Some(active_conn) = active_conn
                && conn_expanded
            {
                for db_name in &active_conn.databases {
                    let db_node_id = TreeNodeId::database(conn.id, db_name);
                    let db_expanded = expanded.contains(&db_node_id);
                    items.push(SidebarEntry::new(
                        db_node_id.clone(),
                        db_name.clone(),
                        1,
                        true,
                        db_expanded,
                    ));

                    if db_expanded && let Some(collections) = active_conn.collections.get(db_name) {
                        for col_name in collections {
                            let col_node_id = TreeNodeId::collection(conn.id, db_name, col_name);
                            items.push(SidebarEntry::new(
                                col_node_id,
                                col_name.clone(),
                                2,
                                false,
                                false,
                            ));
                        }
                    }
                }
            }
        }

        items
    }

    fn rebuild_index(&mut self) {
        self.entry_index_by_id = Self::build_index(&self.entries);
    }

    fn build_index(entries: &[SidebarEntry]) -> HashMap<TreeNodeId, usize> {
        entries.iter().enumerate().map(|(ix, entry)| (entry.id.clone(), ix)).collect()
    }

    fn sync_selected_index(&mut self) {
        self.selected_index =
            self.selected_tree_id.as_ref().and_then(|id| self.entry_index_by_id.get(id).copied());
        if self.selected_index.is_none() {
            self.selected_tree_id = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_saved_connections_remain_visible() {
        let connection = SavedConnection::new("Saved".into(), "mongodb://localhost".into());
        let id = TreeNodeId::connection(connection.id);
        let expanded = HashSet::from([id.clone()]);
        let entries = SidebarModel::build_entries(
            &[connection],
            &std::collections::HashMap::new(),
            &expanded,
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert!(!entries[0].is_folder);
        assert!(!entries[0].is_expanded);
    }

    fn model_with_entries(entries: Vec<SidebarEntry>) -> SidebarModel {
        let entry_index_by_id = SidebarModel::build_index(&entries);
        SidebarModel {
            connecting_connection: None,
            loading_databases: HashSet::new(),
            expanded_nodes: HashSet::new(),
            selected_tree_id: None,
            selected_index: None,
            entry_index_by_id,
            entries,
            search_open: false,
            search_selected: None,
            typeahead_query: String::new(),
            typeahead_last: None,
        }
    }

    #[test]
    fn typeahead_uses_typo_tolerant_matching() {
        let connection_id = Uuid::new_v4();
        let mut model = model_with_entries(vec![
            SidebarEntry::new(TreeNodeId::connection(connection_id), "Production", 0, true, true),
            SidebarEntry::new(
                TreeNodeId::database(connection_id, "analytics"),
                "analytics",
                1,
                true,
                false,
            ),
        ]);

        model.typeahead_query = "prodction".to_string();
        let (_, selected) = model.select_typeahead_match().expect("expected typo match");

        assert_eq!(selected, TreeNodeId::connection(connection_id));
    }

    #[test]
    fn typeahead_delete_keeps_the_session_active_when_empty() {
        let connection_id = Uuid::new_v4();
        let mut model = model_with_entries(vec![SidebarEntry::new(
            TreeNodeId::connection(connection_id),
            "Production",
            0,
            true,
            true,
        )]);
        model.typeahead_query = "p".to_string();

        assert!(model.handle_typeahead_key("backspace", None));
        assert!(model.typeahead_query.is_empty());
        assert!(model.typeahead_last.is_some());
    }
}
