//! Per-tab comparison data. Only the setup is persisted; documents and results stay in memory.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mongodb::IndexModel;
use mongodb::bson::{Bson, DateTime, Document};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::bson::compare::IgnoreSet;
use crate::connection::CancellationToken;
use crate::connection::ops::compare::{
    CompareCounts, CompareMessage, CompareSummary, DiffKind, DiffRow, SortPlan,
};
use crate::connection::ops::compare_database::{
    CollectionKind, CollectionPair, PairKind, PairMessage, PairScan,
};

/// What a Compare tab pairs: two collections, or every collection of two databases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareScope {
    #[default]
    Collections,
    Databases,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompareEndpoint {
    pub connection_id: Option<Uuid>,
    pub database: String,
    pub collection: String,
}

impl CompareEndpoint {
    pub fn complete(&self) -> bool {
        self.connection_id.is_some() && !self.database.is_empty() && !self.collection.is_empty()
    }
    /// Everything the scope needs: a database scope ignores the collection.
    pub fn ready(&self, scope: CompareScope) -> bool {
        match scope {
            CompareScope::Collections => self.complete(),
            CompareScope::Databases => self.connection_id.is_some() && !self.database.is_empty(),
        }
    }
    pub fn namespace(&self) -> String {
        format!("{}.{}", self.database, self.collection)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompareConfig {
    pub scope: CompareScope,
    pub sides: [CompareEndpoint; 2],
    pub fields: Vec<String>,
    pub filter: String,
    pub ignore: Vec<String>,
    /// Database scope: collections left out of the content scan.
    pub skip: Vec<String>,
}

impl Default for CompareConfig {
    fn default() -> Self {
        Self {
            scope: CompareScope::Collections,
            sides: Default::default(),
            fields: vec!["_id".into()],
            filter: String::new(),
            ignore: Vec::new(),
            skip: Vec::new(),
        }
    }
}

impl CompareConfig {
    /// A custom key replaces the automatic _id default; later additions form a compound key.
    pub fn add_match_fields(&mut self, input: &str) {
        let fields: Vec<_> =
            input.split(',').map(str::trim).filter(|field| !field.is_empty()).collect();
        if self.fields == ["_id"] && fields.iter().any(|field| *field != "_id") {
            self.fields.clear();
        }
        for field in fields {
            if !self.fields.iter().any(|existing| existing == field) {
                self.fields.push(field.to_owned());
            }
        }
    }

    pub fn ignore_set(&self) -> IgnoreSet {
        let ignore = IgnoreSet::new(&self.ignore);
        if self.fields == ["_id"] { ignore } else { ignore.ignoring_id() }
    }
}

/// Where one collection of a database comparison is in the content scan.
#[derive(Clone, Debug, Default)]
pub enum PairProgress {
    /// Not scanned: on one side only, a view or time-series, or before the listing.
    #[default]
    Unscheduled,
    Waiting,
    Scanning(CompareCounts),
    Done(CompareSummary),
    Skipped,
    Cancelled,
    Failed(String),
}

/// What a collection's row says, from its presence and its progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairStatus {
    LeftOnly,
    RightOnly,
    NotComparable(CollectionKind),
    Waiting,
    Scanning,
    Different,
    Minor,
    Identical,
    Skipped,
    Cancelled,
    Failed,
}

/// Segments: All, left only, right only, different, minor, identical, not compared.
pub const PAIR_SEGMENTS: usize = 7;

impl PairStatus {
    /// All holds what differs and what is still to come; every other segment holds one outcome.
    pub fn segments(self) -> (bool, Option<usize>) {
        match self {
            Self::LeftOnly => (true, Some(1)),
            Self::RightOnly => (true, Some(2)),
            Self::Different => (true, Some(3)),
            Self::Minor => (false, Some(4)),
            Self::Identical => (false, Some(5)),
            Self::Waiting | Self::Scanning => (true, None),
            Self::NotComparable(_) | Self::Skipped | Self::Cancelled | Self::Failed => {
                (false, Some(6))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CompareTabKey {
    pub id: Uuid,
    pub connection_id: Option<Uuid>,
}

#[derive(Clone, Debug, Default)]
pub struct CompareMetadata {
    pub endpoint: CompareEndpoint,
    pub indexes: Vec<IndexModel>,
    pub count: Option<u64>,
    pub bytes: Option<u64>,
    pub timeseries: bool,
    pub non_simple_collation: bool,
    pub error: Option<String>,
    pub supports_sync: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct CompareDetail {
    pub documents: [Vec<Document>; 2],
    pub changed_since_scan: bool,
}

pub struct CompareTabState {
    pub sync: super::compare_sync::CompareSyncState,
    pub connection_identities: [Option<crate::models::ConnectionWriteIdentity>; 2],
    pub config: CompareConfig,
    pub compared: Option<CompareConfig>,
    pub run: u64,
    pub running: bool,
    pub cancellation: Option<CancellationToken>,
    pub counts: CompareCounts,
    pub rows: Vec<DiffRow>,
    /// Every row, left-only, right-only, different, minor, multiple matches.
    pub segments: [Vec<usize>; 6],
    pub segment: usize,
    pub selected: Option<usize>,
    pub summary: Option<CompareSummary>,
    pub sort: Option<SortPlan>,
    pub estimated: [Option<u64>; 2],
    pub started_sides: [bool; 2],
    pub started: Option<Instant>,
    pub compared_at: Option<DateTime>,
    pub error: Option<String>,
    /// A run has started but the previous results are still on screen (see `begin`).
    pub pending_reset: bool,
    /// The run has lasted long enough to be worth showing as busy (150 ms, see `run_compare`).
    pub slow: bool,
    pub metadata: [Option<CompareMetadata>; 2],
    pub detail: Option<Arc<CompareDetail>>,
    pub detail_row: Option<usize>,
    pub detail_loading: bool,
    pub detail_slow: bool,
    pub detail_error: Option<String>,
    pub detail_generation: u64,
    pub detail_cache: std::collections::HashMap<usize, Arc<CompareDetail>>,
    /// Database scope: every collection name on either side, alphabetical.
    pub pairs: Vec<CollectionPair>,
    pub pair_progress: Vec<PairProgress>,
    pub pair_tokens: Vec<Option<CancellationToken>>,
    /// Rebuilt when a run ends or the segment changes, never mid-run: rows must not move
    /// under the pointer while collections settle.
    pub pair_segments: [Vec<usize>; PAIR_SEGMENTS],
    pub pair_segment: usize,
    pub pair_selected: Option<usize>,
    pub pair_current: Option<usize>,
    pub pair_elapsed: Option<Duration>,
}

impl Default for CompareTabState {
    fn default() -> Self {
        Self::new(CompareConfig::default())
    }
}

impl Drop for CompareTabState {
    fn drop(&mut self) {
        self.cancel_run();
    }
}

impl CompareTabState {
    pub fn new(config: CompareConfig) -> Self {
        Self {
            sync: Default::default(),
            connection_identities: Default::default(),
            config,
            compared: None,
            run: 0,
            running: false,
            cancellation: None,
            counts: Default::default(),
            rows: Vec::new(),
            segments: Default::default(),
            segment: 0,
            selected: None,
            summary: None,
            sort: None,
            estimated: [None; 2],
            started_sides: [false; 2],
            started: None,
            compared_at: None,
            error: None,
            pending_reset: false,
            slow: false,
            metadata: Default::default(),
            detail: None,
            detail_row: None,
            detail_loading: false,
            detail_slow: false,
            detail_error: None,
            detail_generation: 0,
            detail_cache: Default::default(),
            pairs: Vec::new(),
            pair_progress: Vec::new(),
            pair_tokens: Vec::new(),
            pair_segments: Default::default(),
            pair_segment: 0,
            pair_selected: None,
            pair_current: None,
            pair_elapsed: None,
        }
    }

    /// Stops the scan and every collection in it, including the one being read.
    pub fn cancel_run(&self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
        for token in self.pair_tokens.iter().flatten() {
            token.cancel();
        }
    }

    pub fn begin(&mut self) -> CancellationToken {
        if let Some(token) = &self.cancellation {
            token.cancel();
        }
        let run = self.run.wrapping_add(1);
        if self.compared.is_none() {
            // First run: nothing on screen worth keeping.
            let metadata = self.metadata.clone();
            *self = Self::new(self.config.clone());
            self.metadata = metadata;
            self.compared = Some(self.config.clone());
        } else {
            // Keep the previous results until the new run reports. Wiping them here shows every
            // empty state for the length of the scan, which reads as the whole tab reloading.
            self.pending_reset = true;
            // Undo expires when a new comparison starts.
            self.sync = Default::default();
            self.error = None;
        }
        self.run = run;
        self.running = true;
        self.slow = false;
        self.started = Some(Instant::now());
        self.compared_at = Some(DateTime::now());
        self.sort = None;
        self.estimated = [None; 2];
        self.started_sides = [false; 2];
        let token = CancellationToken::new();
        self.cancellation = Some(token.clone());
        token
    }

    /// Drop the previous run's results; the new run is about to fill them in.
    fn reset_results(&mut self) {
        self.pending_reset = false;
        self.sync = Default::default();
        self.compared = Some(self.config.clone());
        self.counts = Default::default();
        self.rows.clear();
        self.segments = Default::default();
        self.selected = None;
        self.summary = None;
        self.detail = None;
        self.detail_row = None;
        self.detail_loading = false;
        self.detail_slow = false;
        self.detail_error = None;
        self.detail_generation = self.detail_generation.wrapping_add(1);
        self.detail_cache.clear();
        self.pairs.clear();
        self.pair_progress.clear();
        self.pair_tokens.clear();
        self.pair_segments = Default::default();
        self.pair_selected = None;
        self.pair_current = None;
        self.pair_elapsed = None;
    }

    /// The database listing arrived: it replaces whatever the previous run showed. Returns the
    /// collections to scan, smallest first; the run stays busy until they are done.
    pub fn receive_pairs(&mut self, result: Result<Vec<CollectionPair>, String>) -> Vec<PairScan> {
        if self.pending_reset {
            self.reset_results();
        }
        let pairs = match result {
            Ok(pairs) => pairs,
            Err(error) => {
                self.error = Some(error);
                self.finish_scan();
                return Vec::new();
            }
        };
        let skip = self.results_config().skip.clone();
        self.pair_progress = vec![PairProgress::Unscheduled; pairs.len()];
        self.pair_tokens = vec![None; pairs.len()];
        let mut scans = Vec::new();
        for (index, pair) in pairs.iter().enumerate() {
            if pair.kind() != PairKind::Both {
                continue;
            }
            if skip.contains(&pair.name) {
                self.pair_progress[index] = PairProgress::Skipped;
                continue;
            }
            let cancellation = CancellationToken::new();
            self.pair_progress[index] = PairProgress::Waiting;
            self.pair_tokens[index] = Some(cancellation.clone());
            scans.push(PairScan { index, name: pair.name.clone(), cancellation });
        }
        // Smallest first, so the first results arrive in seconds; unknown sizes go last.
        scans.sort_by_key(|scan| {
            let [left, right] = &pairs[scan.index].sides;
            let size = |side: &Option<_>| {
                side.as_ref()
                    .and_then(|side: &crate::connection::ops::compare_database::SideCollection| {
                        side.estimated
                    })
                    .unwrap_or(u64::MAX)
            };
            size(left).max(size(right))
        });
        self.pairs = pairs;
        self.rebuild_pair_segments();
        // Cancelled while listing: the scan passes over every collection and says so.
        if self.cancellation.as_ref().is_some_and(CancellationToken::is_cancelled) {
            scans.iter().for_each(|scan| scan.cancellation.cancel());
        }
        if scans.is_empty() {
            self.finish_scan();
        }
        scans
    }

    /// Scan one collection again, after a sync in its own tab or a failure.
    pub fn recheck_pair(&mut self, index: usize) -> Option<PairScan> {
        if self.running || self.pairs.get(index)?.kind() != PairKind::Both {
            return None;
        }
        let cancellation = CancellationToken::new();
        self.pair_progress[index] = PairProgress::Waiting;
        self.pair_tokens[index] = Some(cancellation.clone());
        self.running = true;
        self.cancellation = Some(CancellationToken::new());
        Some(PairScan { index, name: self.pairs[index].name.clone(), cancellation })
    }

    pub fn receive_pair(&mut self, message: PairMessage) {
        let cancelled = self.cancellation.as_ref().is_some_and(CancellationToken::is_cancelled);
        match message {
            PairMessage::Started(index) => {
                self.pair_progress[index] = PairProgress::Scanning(Default::default());
                self.pair_current = Some(index);
            }
            PairMessage::Progress(index, counts) => {
                if matches!(self.pair_progress[index], PairProgress::Scanning(_)) {
                    self.pair_progress[index] = PairProgress::Scanning(counts);
                }
            }
            PairMessage::Done(index, summary) => {
                self.pair_progress[index] = match summary.cancelled {
                    true if cancelled => PairProgress::Cancelled,
                    true => PairProgress::Skipped,
                    false => PairProgress::Done(summary),
                };
                self.pair_tokens[index] = None;
                self.pair_current = None;
            }
            PairMessage::Failed(index, error) => {
                self.pair_progress[index] = PairProgress::Failed(error);
                self.pair_tokens[index] = None;
                self.pair_current = None;
            }
        }
    }

    /// Skip one collection: now if it is waiting, or when its scan stops.
    pub fn skip_pair(&mut self, index: usize) {
        if let Some(token) = self.pair_tokens.get_mut(index).and_then(Option::take) {
            token.cancel();
        }
        if matches!(self.pair_progress.get(index), Some(PairProgress::Waiting)) {
            self.pair_progress[index] = PairProgress::Skipped;
        }
    }

    /// The scan ended, finished or cancelled: collections it never reached say why.
    pub fn finish_scan(&mut self) {
        let cancelled = self.cancellation.as_ref().is_some_and(CancellationToken::is_cancelled);
        for progress in &mut self.pair_progress {
            if matches!(progress, PairProgress::Waiting | PairProgress::Scanning(_)) {
                *progress = if cancelled { PairProgress::Cancelled } else { PairProgress::Skipped };
            }
        }
        self.pair_tokens.iter_mut().for_each(|token| *token = None);
        self.pair_current = None;
        self.running = false;
        self.cancellation = None;
        if self.pair_elapsed.is_none() {
            self.pair_elapsed = self.started.map(|started| started.elapsed());
        }
        self.rebuild_pair_segments();
    }

    pub fn pair_status(&self, index: usize) -> PairStatus {
        match self.pairs[index].kind() {
            PairKind::LeftOnly => PairStatus::LeftOnly,
            PairKind::RightOnly => PairStatus::RightOnly,
            PairKind::NotComparable(kind) => PairStatus::NotComparable(kind),
            PairKind::Both => match &self.pair_progress[index] {
                PairProgress::Unscheduled | PairProgress::Waiting => PairStatus::Waiting,
                PairProgress::Scanning(_) => PairStatus::Scanning,
                PairProgress::Done(summary) => {
                    let c = summary.counts;
                    if c.different + c.only_left + c.only_right > 0 {
                        PairStatus::Different
                    } else if c.minor > 0 {
                        PairStatus::Minor
                    } else {
                        PairStatus::Identical
                    }
                }
                PairProgress::Skipped => PairStatus::Skipped,
                PairProgress::Cancelled => PairStatus::Cancelled,
                PairProgress::Failed(_) => PairStatus::Failed,
            },
        }
    }

    pub fn rebuild_pair_segments(&mut self) {
        self.pair_segments = Default::default();
        for index in 0..self.pairs.len() {
            let (all, segment) = self.pair_status(index).segments();
            if all {
                self.pair_segments[0].push(index);
            }
            if let Some(segment) = segment {
                self.pair_segments[segment].push(index);
            }
        }
    }

    /// Live counts for the segment buttons; the lists themselves hold still until the run ends.
    pub fn pair_segment_counts(&self) -> [usize; PAIR_SEGMENTS] {
        let mut counts = [0; PAIR_SEGMENTS];
        for index in 0..self.pairs.len() {
            let (all, segment) = self.pair_status(index).segments();
            counts[0] += usize::from(all);
            if let Some(segment) = segment {
                counts[segment] += 1;
            }
        }
        counts
    }

    /// Documents read so far, and the estimated total of what the scan will read.
    pub fn pair_scan_reads(&self) -> (u64, Option<u64>) {
        let mut read = 0;
        let mut total = Some(0u64);
        for (pair, progress) in self.pairs.iter().zip(&self.pair_progress) {
            let counts = match progress {
                PairProgress::Scanning(counts) => Some(*counts),
                PairProgress::Done(summary) => Some(summary.counts),
                _ => None,
            };
            if let Some(counts) = counts {
                read += counts.left_read + counts.right_read;
            }
            if matches!(
                progress,
                PairProgress::Waiting | PairProgress::Scanning(_) | PairProgress::Done(_)
            ) {
                total = total
                    .zip(pair.sides[0].as_ref().and_then(|side| side.estimated))
                    .zip(pair.sides[1].as_ref().and_then(|side| side.estimated))
                    .map(|((total, left), right)| total + left + right);
            }
        }
        (read, total)
    }

    pub fn visible_pairs(&self) -> &[usize] {
        &self.pair_segments[self.pair_segment]
    }

    /// The first collection whose name contains the text, ignoring case.
    pub fn find_pair(&self, text: &str) -> Option<usize> {
        let text = text.trim().to_lowercase();
        if text.is_empty() {
            return None;
        }
        self.pairs.iter().position(|pair| pair.name.to_lowercase().contains(&text))
    }

    pub fn receive(&mut self, message: CompareMessage) {
        // `Prepared` carries no results, so the old ones stay up until rows or a verdict arrive.
        if self.pending_reset && !matches!(message, CompareMessage::Prepared { .. }) {
            self.reset_results();
        }
        match message {
            CompareMessage::Prepared { sort, estimated, .. } => {
                self.sort = Some(sort);
                self.estimated = estimated;
            }
            CompareMessage::Progress { counts, new_rows, left_started, right_started } => {
                self.counts = counts;
                self.started_sides = [left_started, right_started];
                for row in new_rows {
                    let index = self.rows.len();
                    let segment = segment_for(row.kind);
                    self.segments[segment].push(index);
                    self.segments[0].push(index);
                    self.rows.push(row);
                }
            }
            CompareMessage::Done(summary) => {
                self.counts = summary.counts;
                self.summary = Some(summary);
                self.running = false;
                self.cancellation = None;
            }
            CompareMessage::Failed(error) => {
                self.error = Some(error);
                self.running = false;
                self.cancellation = None;
            }
        }
    }

    /// Show the run as busy. A scan that finishes within 150 ms would only flicker every
    /// disabled state and progress cue across the tab, so those wait for this.
    pub fn busy(&self) -> bool {
        self.running && self.slow
    }

    pub fn visible(&self) -> &[usize] {
        &self.segments[self.segment]
    }

    pub fn results_config(&self) -> &CompareConfig {
        self.compared.as_ref().unwrap_or(&self.config)
    }

    pub fn find_key(&self, text: &str) -> Option<usize> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        fn matches(value: &Bson, text: &str) -> bool {
            match value {
                Bson::Document(document) => document.values().any(|v| matches(v, text)),
                Bson::String(value) => {
                    value == text || serde_json::from_str::<String>(text).is_ok_and(|s| s == *value)
                }
                Bson::ObjectId(value) => {
                    mongodb::bson::oid::ObjectId::parse_str(text).is_ok_and(|v| v == *value)
                }
                Bson::Int32(value) => text.parse::<i32>() == Ok(*value),
                Bson::Int64(value) => text.parse::<i64>() == Ok(*value),
                Bson::Double(value) => text.parse::<f64>() == Ok(*value),
                Bson::Binary(value)
                    if value.subtype == mongodb::bson::spec::BinarySubtype::Uuid =>
                {
                    Uuid::parse_str(text).is_ok_and(|id| id.as_bytes().as_slice() == value.bytes)
                }
                _ => crate::bson::bson_value_preview(value, usize::MAX) == text,
            }
        }
        self.rows.iter().position(|row| matches(&row.key, text))
    }
}

pub fn segment_for(kind: DiffKind) -> usize {
    match kind {
        DiffKind::OnlyLeft => 1,
        DiffKind::OnlyRight => 2,
        DiffKind::Different => 3,
        DiffKind::Minor => 4,
        DiffKind::MultipleMatches => 5,
    }
}
