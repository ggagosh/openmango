use std::collections::{HashMap, HashSet};

use gpui_component::table::{Column, ColumnSort};

use super::column_schema::{MIN_COL_WIDTH, TableColumnDef, build_column_defs_with_overrides};

pub struct TableColumns {
    pub columns: Vec<TableColumnDef>,
    pub column_defs: Vec<Column>,
    saved_widths: HashMap<String, f32>,
    stable_column_keys: Vec<String>,
    stable_widths: HashMap<String, f32>,
    pub active_sort: Option<(String, ColumnSort)>,
    pinned_columns: HashSet<String>,
    hidden_columns: HashSet<String>,
}

impl Default for TableColumns {
    fn default() -> Self {
        Self::new()
    }
}

impl TableColumns {
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            column_defs: Vec::new(),
            saved_widths: HashMap::new(),
            stable_column_keys: Vec::new(),
            stable_widths: HashMap::new(),
            active_sort: None,
            pinned_columns: HashSet::new(),
            hidden_columns: HashSet::new(),
        }
    }

    pub fn refresh_columns(&mut self, discovered: Vec<TableColumnDef>) {
        let all = self.merge_stable_columns(discovered);
        self.columns = all.into_iter().filter(|c| !self.hidden_columns.contains(&c.key)).collect();
        self.column_defs = build_column_defs_with_overrides(
            &self.columns,
            &self.saved_widths,
            &self.active_sort,
            &self.pinned_columns,
        );
    }

    fn merge_stable_columns(&mut self, discovered: Vec<TableColumnDef>) -> Vec<TableColumnDef> {
        let discovered_map: HashMap<String, TableColumnDef> =
            discovered.into_iter().map(|c| (c.key.clone(), c)).collect();

        if self.stable_column_keys.is_empty() {
            let mut keys: Vec<String> = discovered_map.keys().cloned().collect();
            // Deterministic order: _id first, then alphabetical.
            keys.sort();
            if let Some(pos) = keys.iter().position(|k| k == "_id") {
                keys.swap(0, pos);
            }
            self.stable_column_keys = keys;
        } else {
            for key in discovered_map.keys() {
                if !self.stable_column_keys.contains(key) {
                    self.stable_column_keys.push(key.clone());
                }
            }
        }

        // Lock in widths: first-seen width wins, never changes on re-discovery.
        for (key, col) in &discovered_map {
            self.stable_widths.entry(key.clone()).or_insert(col.width);
        }

        self.stable_column_keys
            .iter()
            .map(|key| TableColumnDef {
                key: key.clone(),
                width: self.stable_widths.get(key).copied().unwrap_or(MIN_COL_WIDTH),
            })
            .collect()
    }

    pub fn rebuild_column_defs(&mut self) {
        let col_map: HashMap<String, TableColumnDef> =
            self.columns.drain(..).map(|c| (c.key.clone(), c)).collect();
        self.columns = self
            .stable_column_keys
            .iter()
            .filter(|k| !self.hidden_columns.contains(*k))
            .filter_map(|k| col_map.get(k).cloned())
            .collect();
        self.column_defs = build_column_defs_with_overrides(
            &self.columns,
            &self.saved_widths,
            &self.active_sort,
            &self.pinned_columns,
        );
    }

    // ── Saved widths ─────────────────────────────────────────────────

    pub fn set_saved_widths(&mut self, widths: HashMap<String, f32>) {
        self.saved_widths = widths;
    }

    pub fn update_saved_widths(&mut self, widths: HashMap<String, f32>) {
        self.saved_widths.extend(widths);
    }

    // ── Column order ─────────────────────────────────────────────────

    pub fn set_column_order(&mut self, order: Vec<String>) {
        if !order.is_empty() {
            self.stable_column_keys = order;
        }
    }

    pub fn column_order(&self) -> Vec<String> {
        self.stable_column_keys.clone()
    }

    pub fn apply_column_move(&mut self, from_ix: usize, to_ix: usize) {
        let from_key = match self.columns.get(from_ix) {
            Some(c) => c.key.clone(),
            None => return,
        };
        let to_key = match self.columns.get(to_ix) {
            Some(c) => c.key.clone(),
            None => return,
        };
        let Some(src) = self.stable_column_keys.iter().position(|k| k == &from_key) else {
            return;
        };
        let Some(dst) = self.stable_column_keys.iter().position(|k| k == &to_key) else {
            return;
        };
        let key = self.stable_column_keys.remove(src);
        self.stable_column_keys.insert(dst, key);
    }

    // ── Hidden columns ───────────────────────────────────────────────

    pub fn set_hidden_columns(&mut self, hidden: HashSet<String>) {
        self.hidden_columns = hidden;
    }

    pub fn is_column_hidden(&self, key: &str) -> bool {
        self.hidden_columns.contains(key)
    }

    pub fn all_column_keys(&self) -> &[String] {
        &self.stable_column_keys
    }

    // ── Pinned columns ───────────────────────────────────────────────

    pub fn set_pinned_columns(&mut self, pinned: HashSet<String>) {
        self.pinned_columns = pinned;
    }

    pub fn pinned_columns(&self) -> &HashSet<String> {
        &self.pinned_columns
    }

    pub fn toggle_pin_column(&mut self, col_key: &str) -> bool {
        if self.pinned_columns.contains(col_key) {
            self.pinned_columns.remove(col_key);
            false
        } else {
            self.pinned_columns.insert(col_key.to_string());
            self.move_pinned_column_to_front(col_key);
            true
        }
    }

    fn move_pinned_column_to_front(&mut self, col_key: &str) {
        let Some(pos) = self.stable_column_keys.iter().position(|k| k == col_key) else {
            return;
        };
        let key = self.stable_column_keys.remove(pos);
        let insert_at = self
            .stable_column_keys
            .iter()
            .position(|k| k != "_id" && !self.pinned_columns.contains(k))
            .unwrap_or(self.stable_column_keys.len());
        self.stable_column_keys.insert(insert_at, key);
    }

    pub fn is_column_pinned(&self, col_ix: usize) -> bool {
        self.columns.get(col_ix).is_some_and(|c| self.pinned_columns.contains(&c.key))
    }

    // ── Column accessors ─────────────────────────────────────────────

    pub fn column_key(&self, col_ix: usize) -> Option<String> {
        self.columns.get(col_ix).map(|c| c.key.clone())
    }

    pub fn columns_count(&self) -> usize {
        self.column_defs.len()
    }

    pub fn column_def(&self, col_ix: usize) -> &Column {
        &self.column_defs[col_ix]
    }
}
