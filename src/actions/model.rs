use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const ACTION_FORMAT_VERSION: u32 = 1;
pub const OPERATION_FORMAT_VERSION: u32 = 1;
pub const ACTION_POLICY_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    PendingApproval,
    Rejected,
    Expired,
    Stale,
    Accepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Queued,
    Running,
    CancelRequested,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationPhase {
    Queued,
    Preparing,
    DumpingDatabase,
    DumpingSource,
    ValidatingSourceDump,
    CheckingTargetPrecondition,
    BackingUpTarget,
    VerifyingBackup,
    ReplacingTarget,
    ApplyingDocuments,
    VerifyingTarget,
    RestoringTargetBackup,
    VerifyingRecovery,
    Completed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    #[default]
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentActionKind {
    Insert,
    Replace,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionRequest {
    DatabaseBackup {
        connection_id: Uuid,
        database: String,
    },
    DatabaseSync {
        source_connection_id: Uuid,
        source_database: String,
        target_connection_id: Uuid,
        target_database: String,
        mode: SyncMode,
    },
    OperationRevert {
        operation_id: Uuid,
    },
    DocumentTransitions {
        connection_id: Uuid,
        database: String,
        collection: String,
        action: DocumentActionKind,
        operation_ids: Vec<Uuid>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionOrigin {
    pub kind: ActionOriginKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_grant_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionOriginKind {
    Mcp,
    BuiltInAi,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionActionSnapshot {
    pub connection_id: Uuid,
    pub display_name: String,
    pub environment: Option<String>,
    pub protected: bool,
    pub read_only: bool,
    pub agent_shared: bool,
    pub connected: bool,
    pub identity_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseStateFingerprint {
    pub exists: bool,
    pub collections: Vec<String>,
    pub estimated_documents: u64,
    pub estimated_bytes: u64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPreview {
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ConnectionActionSnapshot>,
    pub target: ConnectionActionSnapshot,
    pub source_database: Option<String>,
    pub target_database: String,
    pub mode: Option<SyncMode>,
    pub estimated_documents: u64,
    pub estimated_bytes: u64,
    pub warnings: Vec<String>,
    pub backup_behavior: String,
    pub rollback_behavior: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPrerequisites {
    pub database_tools_available: bool,
    pub source_reachable: bool,
    pub target_reachable: bool,
    pub backup_storage_available: bool,
    pub free_space_known_sufficient: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPolicySnapshot {
    pub version: u32,
    pub source_shared: bool,
    pub target_shared: bool,
    pub target_writable: bool,
    pub target_protected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedActionContent {
    pub request: ActionRequest,
    pub origin: ActionOrigin,
    pub policy: ActionPolicySnapshot,
    pub preview: ActionPreview,
    pub prerequisites: ActionPrerequisites,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_state_fingerprint: Option<DatabaseStateFingerprint>,
    pub target_state_fingerprint: DatabaseStateFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDecision {
    pub actor: String,
    pub decided_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedAction {
    pub version: u32,
    pub id: Uuid,
    pub content: ProposedActionContent,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: ActionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<ActionDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<Uuid>,
}

impl ProposedAction {
    pub fn is_pending(&self, now: DateTime<Utc>) -> bool {
        self.status == ActionStatus::PendingApproval && self.expires_at > now
    }

    pub fn hash_suffix(&self) -> &str {
        let start = self.content_hash.len().saturating_sub(12);
        &self.content_hash[start..]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationProgress {
    pub phase: OperationPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    pub documents_processed: u64,
    pub documents_total: u64,
}

impl Default for OperationProgress {
    fn default() -> Self {
        Self {
            phase: OperationPhase::Queued,
            collection: None,
            documents_processed: 0,
            documents_total: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRecord {
    pub version: u32,
    pub id: Uuid,
    pub action_id: Uuid,
    pub action_hash: String,
    pub request: ActionRequest,
    pub origin: ActionOrigin,
    pub target_connection_id: Uuid,
    pub target_database: String,
    pub status: OperationStatus,
    pub progress: OperationProgress,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_backup_id: Option<Uuid>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_error_code: Option<String>,
    pub target_mutation_started: bool,
    pub recovery_interlock: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    pub version: u32,
    pub backup_id: Uuid,
    pub operation_id: Uuid,
    pub connection_identity_hash: String,
    pub database: String,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    pub tools_version: Option<String>,
    pub process_succeeded: bool,
    pub preflight_collections: Vec<String>,
    pub files: Vec<BackupFile>,
    pub file_count: u64,
    pub byte_count: u64,
    pub absence_marker: bool,
    pub verified: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupFile {
    pub relative_path: String,
    pub bytes: u64,
}
