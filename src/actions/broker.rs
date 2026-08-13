use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use chrono::{Duration, Utc};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::model::{
    ACTION_FORMAT_VERSION, ActionDecision, ActionOriginKind, ActionPolicySnapshot, ActionStatus,
    OPERATION_FORMAT_VERSION, OperationProgress, OperationRecord, OperationStatus, ProposedAction,
    ProposedActionContent,
};
use super::store::ActionStore;

const ACTION_TTL: Duration = Duration::hours(1);

#[derive(Debug, Clone)]
pub struct ApprovalValidation {
    pub source_identity_hash: Option<String>,
    pub target_identity_hash: String,
    pub target_state_hash: String,
    pub policy: ActionPolicySnapshot,
    pub prerequisites_satisfied: bool,
}

pub struct ActionBroker {
    store: Arc<ActionStore>,
    mutation_lock: Mutex<()>,
    cancellations: Mutex<HashMap<Uuid, crate::connection::types::CancellationToken>>,
}

impl ActionBroker {
    pub fn new(store: Arc<ActionStore>) -> Self {
        Self { store, mutation_lock: Mutex::new(()), cancellations: Mutex::new(HashMap::new()) }
    }

    pub fn store(&self) -> Arc<ActionStore> {
        self.store.clone()
    }

    pub fn propose(&self, content: ProposedActionContent) -> Result<ProposedAction> {
        validate_proposal(&content)?;
        let content_hash = content_hash(&content)?;
        let now = Utc::now();
        let _guard = self
            .mutation_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Action broker is unavailable"))?;
        if let Some(existing) = self.store.list_actions()?.into_iter().find(|action| {
            action.content_hash == content_hash
                && action.content.origin.client_grant_id == content.origin.client_grant_id
                && action.is_pending(now)
        }) {
            return Ok(existing);
        }
        let action = ProposedAction {
            version: ACTION_FORMAT_VERSION,
            id: Uuid::new_v4(),
            content,
            content_hash,
            created_at: now,
            expires_at: now + ACTION_TTL,
            status: ActionStatus::PendingApproval,
            decision: None,
            operation_id: None,
        };
        self.store.save_action(&action)?;
        Ok(action)
    }

    pub fn get_for_grant(&self, id: Uuid, grant_id: Uuid) -> Result<ProposedAction> {
        let action = self.load_verified_action(id)?;
        ensure_grant_visibility(&action, grant_id)?;
        self.expire_if_needed(action)
    }

    pub fn list_for_grant(
        &self,
        grant_id: Uuid,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ProposedAction>> {
        self.store
            .list_actions()?
            .into_iter()
            .filter(|action| action.content.origin.client_grant_id == Some(grant_id))
            .skip(offset)
            .take(limit)
            .map(|action| self.expire_if_needed(action))
            .collect()
    }

    pub fn list_all(&self) -> Result<Vec<ProposedAction>> {
        self.store.list_actions()?.into_iter().map(|action| self.expire_if_needed(action)).collect()
    }

    pub fn approve_and_create_operation(
        &self,
        action_id: Uuid,
        actor: &str,
        validation: ApprovalValidation,
    ) -> Result<(ProposedAction, OperationRecord)> {
        let _guard = self
            .mutation_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Action broker is unavailable"))?;
        let mut action = self.expire_if_needed(self.load_verified_action(action_id)?)?;
        anyhow::ensure!(
            action.status == ActionStatus::PendingApproval,
            "Action is not pending approval"
        );
        if let Err(reason) = validate_approval(&action, &validation) {
            action.status = ActionStatus::Stale;
            action.decision = Some(ActionDecision {
                actor: actor.to_string(),
                decided_at: Utc::now(),
                reason: Some(reason.to_string()),
            });
            self.store.save_action(&action)?;
            anyhow::bail!("Action is stale: {reason}");
        }

        let operation_id = Uuid::new_v4();
        let now = Utc::now();
        let operation = OperationRecord {
            version: OPERATION_FORMAT_VERSION,
            id: operation_id,
            action_id: action.id,
            action_hash: action.content_hash.clone(),
            request: action.content.request.clone(),
            origin: action.content.origin.clone(),
            target_connection_id: action.content.preview.target.connection_id,
            target_database: action.content.preview.target_database.clone(),
            status: OperationStatus::Queued,
            progress: OperationProgress::default(),
            created_at: now,
            updated_at: now,
            completed_at: None,
            backup_id: None,
            safety_backup_id: None,
            warnings: Vec::new(),
            public_error_code: None,
            target_mutation_started: false,
            recovery_interlock: false,
        };
        action.status = ActionStatus::Accepted;
        action.operation_id = Some(operation_id);
        action.decision =
            Some(ActionDecision { actor: actor.to_string(), decided_at: now, reason: None });
        self.store.accept_action(&action, &operation)?;
        Ok((action, operation))
    }

    pub fn reject(
        &self,
        action_id: Uuid,
        actor: &str,
        reason: Option<String>,
    ) -> Result<ProposedAction> {
        let _guard = self
            .mutation_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Action broker is unavailable"))?;
        let mut action = self.expire_if_needed(self.load_verified_action(action_id)?)?;
        anyhow::ensure!(
            action.status == ActionStatus::PendingApproval,
            "Action is not pending approval"
        );
        action.status = ActionStatus::Rejected;
        action.decision =
            Some(ActionDecision { actor: actor.to_string(), decided_at: Utc::now(), reason });
        self.store.save_action(&action)?;
        Ok(action)
    }

    pub fn get_operation_for_grant(&self, id: Uuid, grant_id: Uuid) -> Result<OperationRecord> {
        let operation = self.store.load_operation(id)?;
        anyhow::ensure!(
            operation.origin.client_grant_id == Some(grant_id),
            "Operation is not visible to this client grant"
        );
        Ok(operation)
    }

    pub fn cancel_operation_for_grant(&self, id: Uuid, grant_id: Uuid) -> Result<OperationRecord> {
        let operation = self.get_operation_for_grant(id, grant_id)?;
        self.cancel_operation(operation)
    }

    pub fn cancel_operation_from_ui(&self, id: Uuid) -> Result<OperationRecord> {
        let operation = self.store.load_operation(id)?;
        self.cancel_operation(operation)
    }

    fn cancel_operation(&self, mut operation: OperationRecord) -> Result<OperationRecord> {
        let _guard = self
            .mutation_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Action broker is unavailable"))?;
        anyhow::ensure!(
            matches!(operation.status, OperationStatus::Queued | OperationStatus::Running),
            "Operation cannot be cancelled in its current state"
        );
        operation.status = OperationStatus::CancelRequested;
        operation.updated_at = Utc::now();
        self.store.save_operation(&operation)?;
        if let Ok(cancellations) = self.cancellations.lock()
            && let Some(cancellation) = cancellations.get(&operation.id)
        {
            cancellation.cancel();
        }
        Ok(operation)
    }

    pub fn register_cancellation(
        &self,
        operation_id: Uuid,
        cancellation: crate::connection::types::CancellationToken,
    ) -> Result<()> {
        self.cancellations
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation cancellation registry is unavailable"))?
            .insert(operation_id, cancellation);
        Ok(())
    }

    pub fn unregister_cancellation(&self, operation_id: Uuid) {
        if let Ok(mut cancellations) = self.cancellations.lock() {
            cancellations.remove(&operation_id);
        }
    }

    fn load_verified_action(&self, id: Uuid) -> Result<ProposedAction> {
        let action = self.store.load_action(id)?;
        anyhow::ensure!(
            action.content_hash == content_hash(&action.content)?,
            "Action content hash mismatch"
        );
        Ok(action)
    }

    fn expire_if_needed(&self, mut action: ProposedAction) -> Result<ProposedAction> {
        if action.status == ActionStatus::PendingApproval && action.expires_at <= Utc::now() {
            action.status = ActionStatus::Expired;
            self.store.save_action(&action)?;
        }
        Ok(action)
    }
}

fn validate_proposal(content: &ProposedActionContent) -> Result<()> {
    anyhow::ensure!(content.policy.version > 0, "Proposal policy version is missing");
    anyhow::ensure!(content.policy.target_shared, "Target is not shared with agents");
    anyhow::ensure!(
        content.prerequisites.database_tools_available,
        "MongoDB Database Tools are unavailable"
    );
    anyhow::ensure!(content.prerequisites.target_reachable, "Target is unreachable");
    anyhow::ensure!(
        content.prerequisites.backup_storage_available,
        "Backup storage is unavailable"
    );
    if matches!(
        content.request,
        super::model::ActionRequest::DatabaseSync { .. }
            | super::model::ActionRequest::OperationRevert { .. }
    ) {
        anyhow::ensure!(content.policy.source_shared, "Source is not shared with agents");
        anyhow::ensure!(content.policy.target_writable, "Target is read-only");
        anyhow::ensure!(content.prerequisites.source_reachable, "Source is unreachable");
    }
    Ok(())
}

fn validate_approval(action: &ProposedAction, validation: &ApprovalValidation) -> Result<()> {
    anyhow::ensure!(action.expires_at > Utc::now(), "approval expired");
    anyhow::ensure!(validation.prerequisites_satisfied, "prerequisites changed");
    anyhow::ensure!(action.content.policy == validation.policy, "policy changed");
    anyhow::ensure!(
        action.content.preview.target.identity_hash == validation.target_identity_hash,
        "target identity changed"
    );
    anyhow::ensure!(
        action.content.target_state_fingerprint.hash == validation.target_state_hash,
        "target database changed"
    );
    match (&action.content.preview.source, &validation.source_identity_hash) {
        (Some(source), Some(identity_hash)) => {
            anyhow::ensure!(source.identity_hash == *identity_hash, "source identity changed");
        }
        (None, None) => {}
        _ => anyhow::bail!("source identity changed"),
    }
    Ok(())
}

fn ensure_grant_visibility(action: &ProposedAction, grant_id: Uuid) -> Result<()> {
    anyhow::ensure!(
        action.content.origin.kind == ActionOriginKind::Mcp
            && action.content.origin.client_grant_id == Some(grant_id),
        "Action is not visible to this client grant"
    );
    Ok(())
}

pub fn content_hash(content: &ProposedActionContent) -> Result<String> {
    let mut normalized = content.clone();
    normalized.origin.client_label = None;
    normalized.origin.session_id = None;
    let bytes = serde_json::to_vec(&normalized).context("Failed to normalize action")?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(to_hex(&hasher.finalize()))
}

pub fn hash_serializable(value: &impl serde::Serialize) -> Result<String> {
    let bytes = serde_json::to_vec(value).context("Failed to normalize identity")?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(to_hex(&hasher.finalize()))
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::model::*;

    fn content(grant_id: Uuid) -> ProposedActionContent {
        let target = ConnectionActionSnapshot {
            connection_id: Uuid::new_v4(),
            display_name: "Target".into(),
            environment: Some("Staging".into()),
            protected: false,
            read_only: false,
            agent_shared: true,
            connected: true,
            identity_hash: "target-identity".into(),
        };
        ProposedActionContent {
            request: ActionRequest::DatabaseBackup {
                connection_id: target.connection_id,
                database: "app".into(),
            },
            origin: ActionOrigin {
                kind: ActionOriginKind::Mcp,
                client_grant_id: Some(grant_id),
                client_label: Some("Pi".into()),
                session_id: None,
            },
            policy: ActionPolicySnapshot {
                version: ACTION_POLICY_VERSION,
                source_shared: true,
                target_shared: true,
                target_writable: true,
                target_protected: false,
            },
            preview: ActionPreview {
                summary: "Back up app".into(),
                source: None,
                target,
                source_database: None,
                target_database: "app".into(),
                mode: None,
                estimated_documents: 1,
                estimated_bytes: 1,
                warnings: vec![],
                backup_behavior: "App-managed backup".into(),
                rollback_behavior: "No target mutation".into(),
            },
            prerequisites: ActionPrerequisites {
                database_tools_available: true,
                source_reachable: true,
                target_reachable: true,
                backup_storage_available: true,
                free_space_known_sufficient: None,
            },
            source_state_fingerprint: None,
            target_state_fingerprint: DatabaseStateFingerprint {
                exists: true,
                collections: vec!["items".into()],
                estimated_documents: 1,
                estimated_bytes: 1,
                hash: "target-state".into(),
            },
        }
    }

    #[test]
    fn identical_pending_proposals_are_deduplicated_per_grant() {
        let temp = tempfile::TempDir::new().unwrap();
        let broker = ActionBroker::new(Arc::new(ActionStore::new(temp.path().join("agent"))));
        let grant = Uuid::new_v4();

        let content = content(grant);
        let first = broker.propose(content.clone()).unwrap();
        let second = broker.propose(content).unwrap();

        assert_eq!(first.id, second.id);
    }

    #[test]
    fn content_hash_ignores_observed_label_and_session_but_binds_grant() {
        let grant = Uuid::new_v4();
        let first = content(grant);
        let mut second = first.clone();
        second.origin.client_label = Some("Renamed Pi".into());
        second.origin.session_id = Some("new-session".into());
        assert_eq!(content_hash(&first).unwrap(), content_hash(&second).unwrap());

        second.origin.client_grant_id = Some(Uuid::new_v4());
        assert_ne!(content_hash(&first).unwrap(), content_hash(&second).unwrap());
    }

    #[test]
    fn edited_action_is_rejected_by_hash_verification() {
        let temp = tempfile::TempDir::new().unwrap();
        let broker = ActionBroker::new(Arc::new(ActionStore::new(temp.path().join("agent"))));
        let grant = Uuid::new_v4();
        let action = broker.propose(content(grant)).unwrap();
        let mut edited = broker.store.load_action(action.id).unwrap();
        edited.content.preview.summary = "Edited on disk".into();
        broker.store.save_action(&edited).unwrap();

        assert!(broker.get_for_grant(action.id, grant).is_err());
    }

    #[test]
    fn approval_fails_stale_when_target_state_changed() {
        let temp = tempfile::TempDir::new().unwrap();
        let broker = ActionBroker::new(Arc::new(ActionStore::new(temp.path().join("agent"))));
        let action = broker.propose(content(Uuid::new_v4())).unwrap();
        let result = broker.approve_and_create_operation(
            action.id,
            "local_user",
            ApprovalValidation {
                source_identity_hash: None,
                target_identity_hash: "target-identity".into(),
                target_state_hash: "changed".into(),
                policy: action.content.policy.clone(),
                prerequisites_satisfied: true,
            },
        );

        assert!(result.is_err());
        assert_eq!(broker.store.load_action(action.id).unwrap().status, ActionStatus::Stale);
    }
}
