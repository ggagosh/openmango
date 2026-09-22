//! Per-tab comparison data. Only the setup is persisted; documents and results stay in memory.

use std::sync::Arc;
use std::time::Instant;

use mongodb::IndexModel;
use mongodb::bson::{Bson, DateTime, Document};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::bson::compare::IgnoreSet;
use crate::connection::CancellationToken;
use crate::connection::ops::compare::{
    CompareCounts, CompareMessage, CompareSummary, DiffKind, DiffRow, SortPlan,
};

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
    pub fn namespace(&self) -> String {
        format!("{}.{}", self.database, self.collection)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompareConfig {
    pub sides: [CompareEndpoint; 2],
    pub fields: Vec<String>,
    pub filter: String,
    pub ignore: Vec<String>,
}

impl Default for CompareConfig {
    fn default() -> Self {
        Self {
            sides: Default::default(),
            fields: vec!["_id".into()],
            filter: String::new(),
            ignore: Vec::new(),
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
}

impl Default for CompareTabState {
    fn default() -> Self {
        Self::new(CompareConfig::default())
    }
}

impl Drop for CompareTabState {
    fn drop(&mut self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
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
