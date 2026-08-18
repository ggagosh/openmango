use chrono::{DateTime, Utc};
use mongodb::bson::{Bson, Document};
use uuid::Uuid;

pub type OperationId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    InsertDocument,
    ReplaceDocument,
    DeleteDocument,
    RevertDocument,
    CreateIndex,
    DropIndex,
    RevertIndex,
}

impl OperationKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::InsertDocument => "insert_document",
            Self::ReplaceDocument => "replace_document",
            Self::DeleteDocument => "delete_document",
            Self::RevertDocument => "revert_document",
            Self::CreateIndex => "create_index",
            Self::DropIndex => "drop_index",
            Self::RevertIndex => "revert_index",
        }
    }

    pub(crate) fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "insert_document" => Ok(Self::InsertDocument),
            "replace_document" => Ok(Self::ReplaceDocument),
            "delete_document" => Ok(Self::DeleteDocument),
            "revert_document" => Ok(Self::RevertDocument),
            "create_index" => Ok(Self::CreateIndex),
            "drop_index" => Ok(Self::DropIndex),
            "revert_index" => Ok(Self::RevertIndex),
            _ => anyhow::bail!("unsupported operation kind"),
        }
    }

    pub(crate) fn is_index(self) -> bool {
        matches!(self, Self::CreateIndex | Self::DropIndex | Self::RevertIndex)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::InsertDocument => "Document insertion",
            Self::ReplaceDocument => "Document replacement",
            Self::DeleteDocument => "Document deletion",
            Self::RevertDocument => "Document restore",
            Self::CreateIndex => "Index creation",
            Self::DropIndex => "Index deletion",
            Self::RevertIndex => "Index restore",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationOrigin {
    User,
    BuiltInAi,
    Mcp,
}

impl OperationOrigin {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::BuiltInAi => "built_in_ai",
            Self::Mcp => "mcp",
        }
    }

    pub(crate) fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "user" => Ok(Self::User),
            "built_in_ai" => Ok(Self::BuiltInAi),
            "mcp" => Ok(Self::Mcp),
            _ => anyhow::bail!("unsupported operation origin"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::BuiltInAi => "Built-in AI",
            Self::Mcp => "MCP",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationStatus {
    PendingApproval,
    Prepared,
    Running,
    Completed,
    Failed,
    Conflict,
    Uncertain,
    RecoveryRequired,
}

impl OperationStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PendingApproval => "pending_approval",
            Self::Prepared => "prepared",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Conflict => "conflict",
            Self::Uncertain => "uncertain",
            Self::RecoveryRequired => "recovery_required",
        }
    }

    pub(crate) fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "pending_approval" => Ok(Self::PendingApproval),
            "prepared" => Ok(Self::Prepared),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "conflict" => Ok(Self::Conflict),
            "uncertain" => Ok(Self::Uncertain),
            "recovery_required" => Ok(Self::RecoveryRequired),
            _ => anyhow::bail!("unsupported operation status"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PendingApproval => "Awaiting approval",
            Self::Prepared => "Prepared",
            Self::Running => "Running",
            Self::Completed => "Completed",
            Self::Failed => "Not applied",
            Self::Conflict => "Conflict",
            Self::Uncertain => "Uncertain",
            Self::RecoveryRequired => "Recovery required",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocumentTarget {
    pub connection_id: Uuid,
    pub connection_name: String,
    pub database: String,
    pub collection: String,
    pub id: Bson,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mutation {
    InsertDocument {
        target: DocumentTarget,
        document: Document,
    },
    ReplaceDocument {
        target: DocumentTarget,
        replacement: Document,
        /// Existing editor concurrency precondition. The engine still captures and owns the
        /// durable server before-image.
        editor_precondition: Option<Document>,
    },
    DeleteDocument {
        target: DocumentTarget,
        editor_precondition: Option<Document>,
    },
    CreateIndex {
        target: DocumentTarget,
        definition: Document,
    },
    DropIndex {
        target: DocumentTarget,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationContext {
    pub origin: OperationOrigin,
}

impl OperationContext {
    pub fn user() -> Self {
        Self { origin: OperationOrigin::User }
    }

    pub fn built_in_ai() -> Self {
        Self { origin: OperationOrigin::BuiltInAi }
    }

    pub fn mcp() -> Self {
        Self { origin: OperationOrigin::Mcp }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationChangePreview {
    pub field: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationPreview {
    pub document_id: String,
    pub changes: Vec<OperationChangePreview>,
    pub total_changes: usize,
}

#[derive(Debug, Clone)]
pub struct OperationSummary {
    pub id: OperationId,
    pub kind: OperationKind,
    pub origin: OperationOrigin,
    pub connection_id: Uuid,
    pub connection_name: String,
    pub database: String,
    pub collection: String,
    pub status: OperationStatus,
    pub parent_operation_id: Option<OperationId>,
    pub reverts_operation_id: Option<OperationId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub recovery_status: Option<String>,
    pub preview: Option<OperationPreview>,
    pub(crate) has_completed_revert: bool,
}

impl OperationSummary {
    pub fn can_revert(&self) -> bool {
        self.status == OperationStatus::Completed && !self.has_completed_revert
    }
}

#[derive(Debug, Clone)]
pub struct OperationEvent {
    pub event_type: String,
    pub status: OperationStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct OperationDetails {
    pub summary: OperationSummary,
    pub events: Vec<OperationEvent>,
}

#[derive(Debug, Clone)]
pub struct OperationQuery {
    pub offset: u32,
    pub limit: u32,
    pub connection_id: Option<Uuid>,
    pub database: Option<String>,
    pub collection: Option<String>,
}

impl OperationQuery {
    pub fn for_collection(connection_id: Uuid, database: &str, collection: &str) -> Self {
        Self {
            connection_id: Some(connection_id),
            database: Some(database.to_string()),
            collection: Some(collection.to_string()),
            ..Self::default()
        }
    }
}

impl Default for OperationQuery {
    fn default() -> Self {
        Self { offset: 0, limit: 50, connection_id: None, database: None, collection: None }
    }
}

#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_offset: Option<u32>,
    pub total: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconciliationReport {
    pub completed: u64,
    pub not_applied: u64,
    pub conflicted: u64,
    pub unavailable: u64,
    pub recovery_required: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct RecoveryPayload {
    pub target: DocumentTarget,
    pub before: Option<Document>,
    pub after: Option<Document>,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredPayload {
    pub version: u32,
    pub encrypted: Vec<u8>,
    pub target_hash: [u8; 32],
    pub before_hash: [u8; 32],
    pub after_hash: [u8; 32],
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedOperation {
    pub summary: OperationSummary,
    pub payload: StoredPayload,
}
