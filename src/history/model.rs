use chrono::{DateTime, Utc};
use mongodb::{Client, bson::Document};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const OBSERVED_IDLE_MS: i64 = 1_000;
pub const OBSERVED_MAX_MS: i64 = 30_000;
pub const MAX_BATCH_ITEMS: u64 = 100_000;
pub const RESTORE_CHUNK_ITEMS: usize = 500;

#[derive(Clone)]
pub struct HistoryConnection {
    pub id: Uuid,
    pub name: String,
    pub client: Client,
    pub databases: Vec<String>,
    pub max_age_days: u32,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationFamily {
    Update,
    Replace,
    Delete,
}

impl OperationFamily {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Update => "update",
            Self::Replace => "replace",
            Self::Delete => "delete",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "update" => Some(Self::Update),
            "replace" => Some(Self::Replace),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Update => "Updated documents",
            Self::Replace => "Replaced documents",
            Self::Delete => "Deleted documents",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupingKind {
    Transaction,
    Attributed,
    Observed,
}

impl GroupingKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Transaction => "transaction",
            Self::Attributed => "attributed",
            Self::Observed => "observed",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "transaction" => Some(Self::Transaction),
            "attributed" => Some(Self::Attributed),
            "observed" => Some(Self::Observed),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Transaction => "Transaction (exact)",
            Self::Attributed => "Attributed (best effort)",
            Self::Observed => "Observed change set",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Open,
    Closed,
    Restoring,
    PartiallyRestored,
    Restored,
    Failed,
}

impl BatchStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Restoring => "restoring",
            Self::PartiallyRestored => "partially_restored",
            Self::Restored => "restored",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "closed" => Some(Self::Closed),
            "restoring" => Some(Self::Restoring),
            "partially_restored" => Some(Self::PartiallyRestored),
            "restored" => Some(Self::Restored),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchSummary {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub database: String,
    pub collection: String,
    pub family: OperationFamily,
    pub grouping: GroupingKind,
    pub trace_id: Option<Uuid>,
    pub first_wall_time: DateTime<Utc>,
    pub last_wall_time: DateTime<Utc>,
    pub item_count: u64,
    pub revertible_count: u64,
    pub conflict_count: u64,
    pub encrypted_bytes: u64,
    pub status: BatchStatus,
    pub restored_count: u64,
    pub skipped_count: u64,
    pub failed_count: u64,
}

impl BatchSummary {
    pub fn can_restore(&self) -> bool {
        self.revertible_count > 0
            && !matches!(self.status, BatchStatus::Restoring | BatchStatus::Restored)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryItem {
    pub id: Uuid,
    pub document_key: Document,
    pub before: Option<Document>,
    pub after: Option<Document>,
    pub outcome: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BatchDetails {
    pub summary: BatchSummary,
    pub items: Vec<HistoryItem>,
    pub next_offset: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryGap {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub database: Option<String>,
    pub collection: Option<String>,
    pub kind: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub next_offset: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct BatchQuery {
    pub connection_id: Uuid,
    pub database: Option<String>,
    pub collection: Option<String>,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub encrypted_bytes: u64,
    pub batches: u64,
    pub items: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EligibilityStatus {
    Unavailable,
    NeedsSetup,
    Eligible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionCoverage {
    pub database: String,
    pub collection: String,
    pub regular: bool,
    pub pre_post_images: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligibilityReport {
    pub status: EligibilityStatus,
    pub version: Option<String>,
    pub topology: Option<String>,
    pub storage_engine: Option<String>,
    pub failures: Vec<String>,
    pub collections: Vec<CollectionCoverage>,
}

impl EligibilityReport {
    pub fn available(&self) -> bool {
        self.status != EligibilityStatus::Unavailable
    }

    pub fn exact_reason(&self) -> Option<&str> {
        self.failures
            .first()
            .map(String::as_str)
            .or_else(|| self.collections.iter().find_map(|coverage| coverage.reason.as_deref()))
    }

    pub fn collection_available(&self, database: &str, collection: &str) -> bool {
        self.status == EligibilityStatus::Eligible
            && self.collections.iter().any(|coverage| {
                coverage.database == database
                    && coverage.collection == collection
                    && coverage.regular
                    && coverage.pre_post_images
                    && coverage.reason.is_none()
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetupReport {
    pub enabled: Vec<String>,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RestoreProgress {
    pub total: u64,
    pub processed: u64,
    pub restored: u64,
    pub skipped: u64,
    pub conflicted: u64,
    pub failed: u64,
    pub done: bool,
}

#[derive(Debug, Clone)]
pub struct TraceDescriptor {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub database: String,
    pub collection: String,
    pub family: OperationFamily,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub affected_count: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedEvent {
    pub connection_id: Uuid,
    pub database: String,
    pub collection: String,
    pub family: OperationFamily,
    pub document_key: Document,
    pub before: Option<Document>,
    pub after: Option<Document>,
    pub resume_token: Vec<u8>,
    pub cluster_time: Option<String>,
    pub wall_time: DateTime<Utc>,
    pub transaction_key: Option<Vec<u8>>,
    pub trace_id: Option<Uuid>,
}
