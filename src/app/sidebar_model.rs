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
        let entries = Self::build_entries(&connections, &active, None, &HashSet::new(), false);
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

    // ponytail: a full rebuild per change, ~0.3us a row (27ms at 100k visible rows, under 2ms
    // at 5k). Splice the toggled folder's child range in and out if trees ever get that big.
    pub(crate) fn refresh_entries(
        &mut self,
        connections: &[SavedConnection],
        active: &std::collections::HashMap<Uuid, ActiveConnection>,
        show_system: bool,
    ) -> Option<usize> {
        self.entries = Self::build_entries(
            connections,
            active,
            self.connecting_connection,
            &self.expanded_nodes,
            show_system,
        );
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

    /// Selects `node_id`, or its nearest visible ancestor while it sits under a collapsed row.
    pub(crate) fn select_nearest(&mut self, node_id: TreeNodeId) -> Option<usize> {
        self.selected_tree_id = Some(node_id);
        self.sync_selected_index();
        self.selected_index
    }

    /// The row that stands for what the app is showing.
    pub(crate) fn node_for_view(
        connection_id: Option<Uuid>,
        selected_db: Option<String>,
        selected_col: Option<String>,
    ) -> Option<TreeNodeId> {
        let connection_id = connection_id?;
        Some(match (selected_db, selected_col) {
            (Some(db), Some(col)) => TreeNodeId::collection(connection_id, db, col),
            (Some(db), None) => TreeNodeId::database(connection_id, db),
            _ => TreeNodeId::connection(connection_id),
        })
    }

    /// Opens every row above `node_id`. Returns whether anything changed, in which case the
    /// entries are stale until the caller rebuilds them.
    pub(crate) fn expand_ancestors(&mut self, node_id: &TreeNodeId) -> bool {
        let mut changed = false;
        for ancestor in std::iter::successors(node_id.parent(), TreeNodeId::parent) {
            changed |= self.expanded_nodes.insert(ancestor);
        }
        changed
    }

    /// Opens or closes a row. Closing with `recursive` also forgets what was open below it, so
    /// the subtree comes back collapsed. Returns whether anything changed.
    pub(crate) fn set_expanded(
        &mut self,
        node_id: &TreeNodeId,
        expanded: bool,
        recursive: bool,
    ) -> bool {
        if expanded {
            return self.expanded_nodes.insert(node_id.clone());
        }
        let before = self.expanded_nodes.len();
        self.expanded_nodes.remove(node_id);
        if recursive {
            self.expanded_nodes.retain(|node| !node.is_descendant_of(node_id));
        }
        before != self.expanded_nodes.len()
    }

    /// The first row under an open folder.
    pub(crate) fn first_child_index(&self, index: usize) -> Option<usize> {
        let depth = self.entries.get(index)?.depth;
        self.entries.get(index + 1).filter(|child| child.depth > depth).map(|_| index + 1)
    }

    /// The row at `depth` that `from` sits under, or `from` itself when it is at that depth.
    pub(crate) fn ancestor_index(
        entries: &[SidebarEntry],
        from: usize,
        depth: usize,
    ) -> Option<usize> {
        if entries.get(from)?.depth < depth {
            return None;
        }
        (0..=from).rev().find(|&ix| entries[ix].depth == depth)
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

    /// Moves the selection by `delta` rows and stops at the ends. It does not wrap: a held
    /// arrow key should come to rest, not loop past the top.
    pub(crate) fn move_sidebar_selection(&mut self, delta: isize) -> Option<(usize, TreeNodeId)> {
        let last = self.entries.len().checked_sub(1)?;
        let next = match self.selected_index {
            Some(index) => index.saturating_add_signed(delta).min(last),
            None if delta < 0 => last,
            None => 0,
        };
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

    /// The tree lists open connections only. A connection being opened shows too, so the
    /// row that will hold its databases appears the moment the user asks for it.
    pub(crate) fn build_entries(
        connections: &[SavedConnection],
        active: &std::collections::HashMap<Uuid, ActiveConnection>,
        connecting: Option<Uuid>,
        expanded: &HashSet<TreeNodeId>,
        show_system: bool,
    ) -> Vec<SidebarEntry> {
        let mut items = Vec::new();
        for conn in connections {
            let active_conn = active.get(&conn.id);
            if active_conn.is_none() && connecting != Some(conn.id) {
                continue;
            }
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
                        // Hidden here, not filtered from the data. When shown they go last,
                        // so `system.views` never sits between two real collections.
                        let (system, regular): (Vec<_>, Vec<_>) = collections
                            .iter()
                            .partition(|name| crate::models::is_system_collection(name));
                        let system = if show_system { system } else { Vec::new() };
                        for col_name in regular.into_iter().chain(system) {
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

    /// A selection hidden by a collapse moves to its nearest visible ancestor, as IDE trees do,
    /// so the keyboard never loses its place.
    fn sync_selected_index(&mut self) {
        let visible = std::iter::successors(self.selected_tree_id.take(), TreeNodeId::parent)
            .find_map(|node| self.index_of(&node).map(|index| (node, index)));
        (self.selected_tree_id, self.selected_index) = visible.unzip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_lists_only_open_and_connecting_connections() {
        let saved = SavedConnection::new("Saved".into(), "mongodb://localhost".into());
        let connecting = SavedConnection::new("Connecting".into(), "mongodb://localhost".into());
        let id = TreeNodeId::connection(connecting.id);
        let expanded = HashSet::from([id.clone(), TreeNodeId::connection(saved.id)]);
        let entries = SidebarModel::build_entries(
            &[saved, connecting.clone()],
            &std::collections::HashMap::new(),
            Some(connecting.id),
            &expanded,
            false,
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert!(!entries[0].is_folder);
        assert!(!entries[0].is_expanded);
    }

    #[test]
    fn selected_connection_without_database_selects_its_row() {
        let connection_id = Uuid::new_v4();
        let id = TreeNodeId::connection(connection_id);
        let mut model =
            model_with_entries(vec![SidebarEntry::new(id.clone(), "Production", 0, true, false)]);

        let node = SidebarModel::node_for_view(Some(connection_id), None, None).unwrap();
        assert_eq!(model.select_nearest(node), Some(0));
        assert_eq!(model.selected_tree_id, Some(id));
    }

    /// One open connection holding databases `a` and `b`, each with collection `c`.
    fn open_model() -> (SidebarModel, Vec<SavedConnection>, HashMap<Uuid, ActiveConnection>) {
        let saved = SavedConnection::new("Local".into(), "mongodb://localhost".into());
        // The client is lazy: building it opens no socket, but it wants a runtime around.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let client = runtime.block_on(async {
            mongodb::Client::with_options(mongodb::options::ClientOptions::default()).unwrap()
        });
        let conn = ActiveConnection {
            config: saved.clone(),
            client,
            databases: vec!["a".into(), "b".into()],
            collections: HashMap::from([
                ("a".to_string(), vec!["c".to_string()]),
                ("b".to_string(), vec!["c".to_string()]),
            ]),
            collection_details: Default::default(),
            runtime_meta: Default::default(),
        };
        let active = HashMap::from([(saved.id, conn)]);
        let model = SidebarModel::new(vec![saved.clone()], active.clone());
        (model, vec![saved], active)
    }

    #[test]
    fn system_collections_are_hidden_or_listed_last() {
        let (_, saved, mut active) = open_model();
        let id = saved[0].id;
        // A user collection that merely mentions "system" must never be treated as one.
        let names = ["system.views", "system_audit", "zebra"].map(String::from).to_vec();
        active.get_mut(&id).unwrap().collections.insert("a".into(), names);
        let expanded = HashSet::from([TreeNodeId::connection(id), TreeNodeId::database(id, "a")]);
        let labels = |show_system| {
            SidebarModel::build_entries(&saved, &active, None, &expanded, show_system)
                .into_iter()
                .filter(|entry| entry.depth == 2)
                .map(|entry| entry.label.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(labels(false), ["system_audit", "zebra"]);
        assert_eq!(labels(true), ["system_audit", "zebra", "system.views"]);
    }

    #[test]
    fn collapsing_moves_the_selection_up_and_nothing_reopens() {
        let (mut model, saved, active) = open_model();
        let id = saved[0].id;
        let col = TreeNodeId::collection(id, "a", "c");
        assert!(model.expand_ancestors(&col));
        model.refresh_entries(&saved, &active, false);
        assert!(model.select_node(col).is_some());

        // Collapse the database holding the selection: the selection lands on the database.
        let db_a = TreeNodeId::database(id, "a");
        assert!(model.set_expanded(&db_a, false, false));
        model.refresh_entries(&saved, &active, false);
        assert_eq!(model.selected_tree_id, Some(db_a.clone()));

        // Opening a sibling leaves the collapsed one closed.
        assert!(model.set_expanded(&TreeNodeId::database(id, "b"), true, false));
        model.refresh_entries(&saved, &active, false);
        assert!(!model.expanded_nodes.contains(&db_a));
        assert_eq!(model.entries.len(), 4);
    }

    #[test]
    fn recursive_collapse_forgets_the_subtree() {
        let (mut model, saved, _) = open_model();
        let id = saved[0].id;
        model.expand_ancestors(&TreeNodeId::collection(id, "a", "c"));
        assert!(model.set_expanded(&TreeNodeId::connection(id), false, true));
        assert!(model.expanded_nodes.is_empty());
    }

    #[test]
    fn arrows_stop_at_the_ends() {
        let (mut model, saved, active) = open_model();
        model.expand_ancestors(&TreeNodeId::database(saved[0].id, "a"));
        model.refresh_entries(&saved, &active, false);

        assert_eq!(model.move_sidebar_selection(1).map(|(ix, _)| ix), Some(0));
        assert_eq!(model.move_sidebar_selection(-1).map(|(ix, _)| ix), Some(0));
        assert_eq!(model.move_sidebar_selection(99).map(|(ix, _)| ix), Some(2));
        assert_eq!(model.move_sidebar_selection(1).map(|(ix, _)| ix), Some(2));
        assert_eq!(model.first_child_index(0), Some(1));
        assert_eq!(model.first_child_index(2), None);
        assert_eq!(SidebarModel::ancestor_index(&model.entries, 2, 0), Some(0));
        assert_eq!(SidebarModel::ancestor_index(&model.entries, 0, 1), None);
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
