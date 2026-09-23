//! Selection is all-or-none plus exceptions, so selecting a large category is constant time.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::compare::{CompareConfig, CompareTabState, PairProgress, segment_for};
use crate::connection::CancellationToken;
use crate::connection::ops::compare::{DiffKind, Side};
use crate::connection::ops::compare_database::{
    CollectionKind, PairKind, PairSync, PairSyncMessage, SyncMode,
};
use crate::connection::ops::compare_sync::{
    Operation, RowOutcome, SyncItem, SyncSummary, operation_for, restore::RestoreHandle,
};

#[derive(Default)]
pub struct CategorySelection {
    pub all: bool,
    exceptions: HashSet<usize>,
}

impl CategorySelection {
    pub fn contains(&self, row: usize) -> bool {
        self.all != self.exceptions.contains(&row)
    }
    pub fn count(&self, total: usize) -> usize {
        if self.all { total.saturating_sub(self.exceptions.len()) } else { self.exceptions.len() }
    }
    pub fn set_all(&mut self, selected: bool) {
        self.all = selected;
        self.exceptions.clear();
    }
    fn toggle(&mut self, row: usize) {
        if !self.exceptions.remove(&row) {
            self.exceptions.insert(row);
        }
    }
}

#[derive(Default)]
pub struct CompareSyncState {
    pub target: Option<Side>,
    pub categories: [CategorySelection; 4],
    pub revision: u64,
    pub anchor: Option<usize>,
    pub running: bool,
    pub undoing: bool,
    pub completed: bool,
    /// The finished writes are single-field copies, not a sync.
    pub field_copies: bool,
    pub cancellation: Option<CancellationToken>,
    pub restore: Option<Arc<RestoreHandle>>,
    pub summary: SyncSummary,
    pub outcomes: HashMap<usize, RowOutcome>,
    pub error: Option<String>,
    // Database scope.
    pub mode: SyncMode,
    /// Collections left out of a database sync, by index in the listing.
    pub excluded: HashSet<usize>,
    /// Each synced collection's undo log.
    pub logs: Vec<(usize, String, Arc<RestoreHandle>)>,
    /// Per collection: what the current sync or undo wrote.
    pub pairs: HashMap<usize, PairSyncResult>,
    pub pair_current: Option<usize>,
}

#[derive(Clone, Debug)]
pub enum PairSyncResult {
    Running(SyncSummary),
    Done(SyncSummary),
    Failed(String),
}

impl Drop for CompareSyncState {
    fn drop(&mut self) {
        if let Some(token) = &self.cancellation {
            token.cancel();
        }
    }
}

impl CompareSyncState {
    pub fn set_target(&mut self, target: Side) {
        if self.running || self.completed || self.target == Some(target) {
            return;
        }
        self.target = Some(target);
        self.anchor = None;
        self.excluded.clear();
        for (selection, kind) in self.categories.iter_mut().zip([
            DiffKind::OnlyLeft,
            DiffKind::OnlyRight,
            DiffKind::Different,
            DiffKind::Minor,
        ]) {
            selection.set_all(
                operation_for(kind, target) != Some(Operation::Delete) && kind != DiffKind::Minor,
            );
        }
        self.revision = self.revision.wrapping_add(1);
    }
    /// Back out of sync mode: no target, nothing selected.
    pub fn clear_target(&mut self) {
        if self.running || self.completed || self.target.is_none() {
            return;
        }
        self.target = None;
        self.anchor = None;
        self.excluded.clear();
        for selection in &mut self.categories {
            selection.set_all(false);
        }
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn selected(&self, row: usize, kind: DiffKind) -> bool {
        self.target.is_some()
            && self.categories.get(segment_for(kind) - 1).is_some_and(|c| c.contains(row))
    }
    pub fn toggle(&mut self, row: usize, kind: DiffKind) {
        if self.target.is_none() || self.running || self.completed {
            return;
        }
        if let Some(category) = self.categories.get_mut(segment_for(kind) - 1) {
            category.toggle(row);
            self.revision = self.revision.wrapping_add(1);
        }
    }
    pub fn set_mode(&mut self, mode: SyncMode) {
        if self.running || self.completed || self.mode == mode {
            return;
        }
        self.mode = mode;
        self.excluded.clear();
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn toggle_pair(&mut self, index: usize) {
        if self.target.is_none() || self.running || self.completed {
            return;
        }
        if !self.excluded.remove(&index) {
            self.excluded.insert(index);
        }
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn set_category(&mut self, category: usize, selected: bool) {
        if self.target.is_none() || self.running || self.completed {
            return;
        }
        if let Some(category) = self.categories.get_mut(category) {
            category.set_all(selected);
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

impl CompareTabState {
    pub fn select_sync_row(&mut self, row: usize, range: bool, toggle: bool) {
        if self.sync.running || self.sync.completed {
            return;
        }
        if range {
            let visible = &self.segments[self.segment];
            let anchor = self.sync.anchor.and_then(|row| visible.iter().position(|i| *i == row));
            if let Some(end) = visible.iter().position(|i| *i == row) {
                let start = anchor.unwrap_or(end);
                for index in &visible[start.min(end)..=start.max(end)] {
                    let kind = self.rows[*index].kind;
                    if !self.sync.selected(*index, kind) {
                        self.sync.toggle(*index, kind);
                    }
                }
            }
        } else {
            if toggle && let Some(item) = self.rows.get(row) {
                self.sync.toggle(row, item.kind);
            }
            self.sync.anchor = Some(row);
        }
    }
}

/// A collection a database sync can write: its copy on the target differs in a way the mode
/// writes, or the target lacks it. `writes` are inserts, replacements and deletes, exact from the
/// scan; for a collection to create, the inserts are the source's estimated size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncCandidate {
    pub index: usize,
    pub create: bool,
    pub writes: [u64; 3],
}

impl CompareTabState {
    pub fn sync_candidates(&self) -> Vec<SyncCandidate> {
        let Some(target) = self.sync.target else {
            return Vec::new();
        };
        let (source, destination) = if target == Side::Left { (1, 0) } else { (0, 1) };
        let skip = &self.results_config().skip;
        self.pairs
            .iter()
            .enumerate()
            .filter_map(|(index, pair)| match (pair.kind(), &self.pair_progress[index]) {
                (PairKind::Both, PairProgress::Done(summary)) => {
                    let writes = self.sync.mode.writes(&summary.counts, target);
                    (writes.iter().sum::<u64>() > 0).then_some(SyncCandidate {
                        index,
                        create: false,
                        writes,
                    })
                }
                (PairKind::LeftOnly | PairKind::RightOnly, _) => {
                    let side = pair.sides[source].as_ref()?;
                    (pair.sides[destination].is_none()
                        && side.kind == CollectionKind::Collection
                        && !skip.contains(&pair.name))
                    .then(|| SyncCandidate {
                        index,
                        create: true,
                        writes: [side.estimated.unwrap_or(0), 0, 0],
                    })
                }
                _ => None,
            })
            .collect()
    }

    pub fn sync_selected(&self) -> Vec<SyncCandidate> {
        let mut candidates = self.sync_candidates();
        candidates.retain(|candidate| !self.sync.excluded.contains(&candidate.index));
        candidates
    }

    /// Totals across the collections of the current database sync or undo.
    pub fn pair_sync_totals(&self) -> SyncSummary {
        let mut total = SyncSummary::default();
        for result in self.sync.pairs.values() {
            if let PairSyncResult::Running(summary) | PairSyncResult::Done(summary) = result {
                total.absorb(summary);
            }
        }
        total
    }

    pub fn receive_pair_sync(&mut self, message: PairSyncMessage) {
        match message {
            PairSyncMessage::Started(index, restore) => {
                self.sync.pair_current = Some(index);
                self.sync.pairs.insert(index, PairSyncResult::Running(SyncSummary::default()));
                if !self.sync.undoing
                    && let Some(pair) = self.pairs.get(index)
                {
                    self.sync.logs.push((index, pair.name.clone(), restore));
                }
            }
            PairSyncMessage::Progress(index, summary) => {
                self.sync.pairs.insert(index, PairSyncResult::Running(summary));
            }
            PairSyncMessage::Done(index, summary) => {
                // A collection the sync created now exists on both sides, so it can be rechecked.
                if !self.sync.undoing
                    && let Some(target) = self.sync.target
                    && let Some(pair) = self.pairs.get_mut(index)
                {
                    let (source, destination) = if target == Side::Left { (1, 0) } else { (0, 1) };
                    if pair.sides[destination].is_none() {
                        pair.sides[destination] = pair.sides[source].clone();
                    }
                }
                self.sync.pairs.insert(index, PairSyncResult::Done(summary));
                self.sync.pair_current = None;
            }
            PairSyncMessage::Failed(index, error) => {
                self.sync.pairs.insert(index, PairSyncResult::Failed(error));
                self.sync.pair_current = None;
            }
        }
    }
}

pub struct DatabaseSyncPlan {
    pub run: u64,
    pub revision: u64,
    pub config: CompareConfig,
    pub target: Side,
    pub mode: SyncMode,
    pub pairs: Vec<PairSync>,
    pub writes: [u64; 3],
    /// Inserts include estimates for collections to create.
    pub estimated: bool,
}

impl DatabaseSyncPlan {
    pub fn from_tab(tab: &CompareTabState) -> Option<Self> {
        if tab.running
            || tab.sync.running
            || tab.sync.completed
            || tab.error.is_some()
            || tab.pair_elapsed.is_none()
            || tab.compared.as_ref() != Some(&tab.config)
        {
            return None;
        }
        let target = tab.sync.target?;
        let selected = tab.sync_selected();
        if selected.is_empty() {
            return None;
        }
        let mut writes = [0; 3];
        for candidate in &selected {
            for (total, count) in writes.iter_mut().zip(candidate.writes) {
                *total += count;
            }
        }
        Some(Self {
            run: tab.run,
            revision: tab.sync.revision,
            config: tab.config.clone(),
            target,
            mode: tab.sync.mode,
            estimated: selected.iter().any(|candidate| candidate.create),
            pairs: selected
                .iter()
                .map(|candidate| PairSync {
                    index: candidate.index,
                    name: tab.pairs[candidate.index].name.clone(),
                    create: candidate.create,
                })
                .collect(),
            writes,
        })
    }
    pub fn matches(&self, tab: &CompareTabState) -> bool {
        self.run == tab.run
            && self.revision == tab.sync.revision
            && self.config == tab.config
            && tab.compared.as_ref() == Some(&self.config)
            && tab.sync.target == Some(self.target)
            && tab.sync.mode == self.mode
            && !tab.running
            && !tab.sync.running
            && !tab.sync.completed
    }
}

pub struct SyncPlan {
    pub run: u64,
    pub revision: u64,
    pub config: CompareConfig,
    pub target: Side,
    pub items: Vec<SyncItem>,
}

impl SyncPlan {
    pub fn from_tab(tab: &CompareTabState) -> Option<Self> {
        if tab.running
            || tab.sync.running
            || tab.sync.completed
            || tab.error.is_some()
            || tab.summary.is_none()
            || tab.compared.as_ref() != Some(&tab.config)
        {
            return None;
        }
        let target = tab.sync.target?;
        let items = tab
            .rows
            .iter()
            .enumerate()
            .filter(|(i, row)| tab.sync.selected(*i, row.kind))
            .filter_map(|(row_index, row)| {
                operation_for(row.kind, target).map(|operation| SyncItem {
                    row_index,
                    row: row.clone(),
                    operation,
                    field: None,
                })
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            return None;
        }
        Some(Self {
            run: tab.run,
            revision: tab.sync.revision,
            config: tab.config.clone(),
            target,
            items,
        })
    }
    pub fn matches(&self, tab: &CompareTabState) -> bool {
        self.run == tab.run
            && self.revision == tab.sync.revision
            && self.config == tab.config
            && tab.compared.as_ref() == Some(&self.config)
            && tab.sync.target == Some(self.target)
            && !tab.running
            && !tab.sync.running
            && !tab.sync.completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_switch_resets_defaults_and_exceptions_and_excludes_ambiguous_rows() {
        let mut selection = CompareSyncState::default();
        assert!(!selection.selected(0, DiffKind::OnlyLeft));
        selection.set_target(Side::Right);
        assert!(selection.selected(0, DiffKind::OnlyLeft));
        assert!(!selection.selected(1, DiffKind::OnlyRight));
        assert!(!selection.selected(2, DiffKind::Minor));
        assert!(!selection.selected(3, DiffKind::MultipleMatches));
        selection.toggle(0, DiffKind::OnlyLeft);
        assert_eq!(selection.categories[0].count(250_000), 249_999);
        selection.set_target(Side::Left);
        assert!(!selection.selected(0, DiffKind::OnlyLeft));
        assert!(selection.selected(1, DiffKind::OnlyRight));
        selection.completed = true;
        selection.set_target(Side::Right);
        assert_eq!(selection.target, Some(Side::Left));
    }

    #[test]
    fn approval_snapshot_is_invalidated_by_selection_config_or_new_run() {
        let mut tab = CompareTabState::default();
        tab.compared = Some(tab.config.clone());
        tab.sync.set_target(Side::Right);
        let plan = SyncPlan {
            run: tab.run,
            revision: tab.sync.revision,
            config: tab.config.clone(),
            target: Side::Right,
            items: vec![],
        };
        assert!(plan.matches(&tab));
        tab.sync.toggle(0, DiffKind::OnlyLeft);
        assert!(!plan.matches(&tab));
        tab.sync.revision = plan.revision;
        tab.config.filter = "{active:true}".into();
        assert!(!plan.matches(&tab));
        tab.config = plan.config.clone();
        tab.run += 1;
        assert!(!plan.matches(&tab));
    }
}
