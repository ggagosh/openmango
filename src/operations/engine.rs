use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use thiserror::Error;
use uuid::Uuid;

use crate::bson::bson_value_preview;

use super::backend::{BackendError, MutationBackend};
use super::crypto::{HistoryCipher, document_state_hash};
use super::model::{
    Mutation, OperationChangePreview, OperationContext, OperationDetails, OperationId,
    OperationKind, OperationPreview, OperationQuery, OperationStatus, OperationSummary, Page,
    PreparedOperation, ReconciliationReport, RecoveryPayload,
};
use super::store::OperationStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OperationError {
    #[error("reversible history is unavailable; no write was made")]
    Unavailable,
    #[error("operation was not found")]
    NotFound,
    #[error("only existing single-document replacements and deletes are supported")]
    Unsupported,
    #[error("document changed after it was opened for editing")]
    PreconditionConflict,
    #[error("operation {operation_id} was blocked by a concurrent document change")]
    Conflict { operation_id: OperationId },
    #[error("operation {operation_id} has an uncertain outcome and requires reconciliation")]
    Uncertain { operation_id: OperationId },
    #[error("operation is not eligible for revert")]
    NotRevertible,
    #[error("operation history could not be updated; no further write was attempted")]
    Internal,
}

impl OperationError {
    pub fn user_message(self) -> &'static str {
        match self {
            Self::Unavailable => "Reversible history is unavailable; no write was made.",
            Self::NotFound => "The operation was not found.",
            Self::Unsupported => {
                "Reversible history currently supports existing single-document replacements and deletes only."
            }
            Self::PreconditionConflict | Self::Conflict { .. } => {
                "The document changed on the server. OpenMango did not overwrite it."
            }
            Self::Uncertain { .. } => {
                "The write outcome is uncertain. Check the collection's History tab before retrying."
            }
            Self::NotRevertible => "This operation is not eligible for revert.",
            Self::Internal => "Operation history could not be updated; no further write was made.",
        }
    }
}

pub struct OperationEngine {
    backend: Arc<dyn MutationBackend>,
    cipher: HistoryCipher,
    store: OperationStore,
}

impl OperationEngine {
    pub(crate) fn open(
        path: PathBuf,
        key: [u8; 32],
        backend: Arc<dyn MutationBackend>,
    ) -> Result<Self, OperationError> {
        Ok(Self {
            backend,
            cipher: HistoryCipher::new(key).map_err(|_| OperationError::Unavailable)?,
            store: OperationStore::open(path).map_err(|_| OperationError::Unavailable)?,
        })
    }

    pub fn execute(
        &self,
        context: OperationContext,
        mutation: Mutation,
    ) -> Result<OperationId, OperationError> {
        let (target, after, editor_precondition, kind) = match mutation {
            Mutation::ReplaceDocument { target, replacement, editor_precondition } => {
                (target, Some(replacement), editor_precondition, OperationKind::ReplaceDocument)
            }
            Mutation::DeleteDocument { target, editor_precondition } => {
                (target, None, editor_precondition, OperationKind::DeleteDocument)
            }
        };
        if after.as_ref().is_some_and(|document| document.get("_id") != Some(&target.id)) {
            return Err(OperationError::Unsupported);
        }
        let before = self
            .backend
            .current_document(&target)
            .map_err(|_| OperationError::Unavailable)?
            .ok_or(OperationError::Unsupported)?;
        if before.get("_id") != Some(&target.id) {
            return Err(OperationError::Unsupported);
        }
        if editor_precondition.as_ref().is_some_and(|expected| expected != &before) {
            return Err(OperationError::PreconditionConflict);
        }
        let operation_id = self.prepare_transition(
            context,
            kind,
            RecoveryPayload { target, before: Some(before), after },
            None,
            None,
        )?;
        self.apply_prepared(operation_id)
    }

    pub fn revert(
        &self,
        context: OperationContext,
        operation_id: OperationId,
    ) -> Result<OperationId, OperationError> {
        let details = self.get(operation_id)?.ok_or(OperationError::NotFound)?;
        if !details.summary.can_revert() {
            return Err(OperationError::NotRevertible);
        }
        let stored = self
            .store
            .payload(operation_id)
            .map_err(|_| OperationError::Internal)?
            .ok_or(OperationError::NotFound)?;
        let original = self
            .cipher
            .decrypt(operation_id, details.summary.connection_name, &stored)
            .map_err(|_| OperationError::NotRevertible)?;
        let reversed = RecoveryPayload {
            target: original.target,
            before: original.after,
            after: original.before,
        };
        let revert_id = self.prepare_transition(
            context,
            OperationKind::RevertDocument,
            reversed,
            Some(operation_id),
            Some(operation_id),
        )?;
        self.apply_prepared(revert_id)
    }

    pub fn get(
        &self,
        operation_id: OperationId,
    ) -> Result<Option<OperationDetails>, OperationError> {
        let Some(mut details) =
            self.store.get(operation_id).map_err(|_| OperationError::Internal)?
        else {
            return Ok(None);
        };
        self.hydrate_summary(&mut details.summary)?;
        Ok(Some(details))
    }

    pub fn list(&self, query: OperationQuery) -> Result<Page<OperationSummary>, OperationError> {
        let mut page = self.store.list(query).map_err(|_| OperationError::Internal)?;
        for summary in &mut page.items {
            self.hydrate_summary(summary)?;
        }
        Ok(page)
    }

    pub fn reconcile(&self) -> Result<ReconciliationReport, OperationError> {
        let mut report = ReconciliationReport::default();
        let operations = self.store.list_incomplete().map_err(|_| OperationError::Internal)?;
        for operation in operations {
            let Some(stored) =
                self.store.payload(operation.id).map_err(|_| OperationError::Internal)?
            else {
                self.store
                    .transition(
                        operation.id,
                        OperationStatus::RecoveryRequired,
                        "recovery_payload_missing",
                        "recovery_required",
                        Some("Encrypted recovery data is unavailable."),
                    )
                    .map_err(|_| OperationError::Internal)?;
                report.recovery_required += 1;
                continue;
            };
            let payload =
                match self.cipher.decrypt(operation.id, operation.connection_name.clone(), &stored)
                {
                    Ok(payload) => payload,
                    Err(_) => {
                        self.store
                            .transition(
                                operation.id,
                                OperationStatus::RecoveryRequired,
                                "recovery_payload_unreadable",
                                "recovery_required",
                                Some("Encrypted recovery data could not be authenticated."),
                            )
                            .map_err(|_| OperationError::Internal)?;
                        report.recovery_required += 1;
                        continue;
                    }
                };
            let current = match self.backend.current_document(&payload.target) {
                Ok(current) => current,
                Err(_) => {
                    self.store
                        .record_recovery_status(
                            operation.id,
                            "reconciliation_target_unavailable",
                            "Target is unavailable; recovery data was retained.",
                        )
                        .map_err(|_| OperationError::Internal)?;
                    report.unavailable += 1;
                    continue;
                }
            };
            let current_hash =
                document_state_hash(current.as_ref()).map_err(|_| OperationError::Internal)?;
            if current_hash == stored.after_hash {
                self.store
                    .transition(
                        operation.id,
                        OperationStatus::Completed,
                        "reconciled_applied",
                        "completed",
                        Some("Reconciliation confirmed that the write was applied."),
                    )
                    .map_err(|_| OperationError::Internal)?;
                report.completed += 1;
            } else if current_hash == stored.before_hash {
                self.store
                    .transition(
                        operation.id,
                        OperationStatus::Failed,
                        "reconciled_not_applied",
                        "not_applied",
                        Some("Reconciliation confirmed that the write was not applied."),
                    )
                    .map_err(|_| OperationError::Internal)?;
                report.not_applied += 1;
            } else {
                self.store
                    .transition(
                        operation.id,
                        OperationStatus::Conflict,
                        "reconciled_conflict",
                        "conflict",
                        Some("Current document matches neither recorded image."),
                    )
                    .map_err(|_| OperationError::Internal)?;
                report.conflicted += 1;
            }
        }
        Ok(report)
    }

    fn hydrate_summary(&self, summary: &mut OperationSummary) -> Result<(), OperationError> {
        let stored = self
            .store
            .payload(summary.id)
            .map_err(|_| OperationError::Internal)?
            .ok_or(OperationError::Internal)?;
        let payload = self
            .cipher
            .decrypt(summary.id, summary.connection_name.clone(), &stored)
            .map_err(|_| OperationError::Internal)?;
        if payload.target.connection_id != summary.connection_id
            || payload.target.database != summary.database
            || payload.target.collection != summary.collection
        {
            return Err(OperationError::Internal);
        }
        summary.preview = Some(operation_preview(&payload));
        Ok(())
    }

    fn prepare_transition(
        &self,
        context: OperationContext,
        kind: OperationKind,
        payload: RecoveryPayload,
        parent_operation_id: Option<OperationId>,
        reverts_operation_id: Option<OperationId>,
    ) -> Result<OperationId, OperationError> {
        let operation_id = Uuid::new_v4();
        let now = Utc::now();
        let encrypted =
            self.cipher.encrypt(operation_id, &payload).map_err(|_| OperationError::Unavailable)?;
        let summary = OperationSummary {
            id: operation_id,
            kind,
            origin: context.origin,
            connection_id: payload.target.connection_id,
            connection_name: payload.target.connection_name.clone(),
            database: payload.target.database.clone(),
            collection: payload.target.collection.clone(),
            status: OperationStatus::Prepared,
            parent_operation_id,
            reverts_operation_id,
            created_at: now,
            updated_at: now,
            recovery_status: None,
            preview: None,
            has_completed_revert: false,
        };
        self.store
            .prepare(PreparedOperation { summary, payload: encrypted })
            .map_err(|_| OperationError::Unavailable)?;
        Ok(operation_id)
    }

    fn apply_prepared(&self, operation_id: OperationId) -> Result<OperationId, OperationError> {
        let details = self
            .store
            .get(operation_id)
            .map_err(|_| OperationError::Internal)?
            .ok_or(OperationError::NotFound)?;
        let stored = self
            .store
            .payload(operation_id)
            .map_err(|_| OperationError::Internal)?
            .ok_or(OperationError::NotFound)?;
        let payload = self
            .cipher
            .decrypt(operation_id, details.summary.connection_name, &stored)
            .map_err(|_| OperationError::Internal)?;
        if payload.before.is_none() && payload.after.is_none() {
            return Err(OperationError::Internal);
        }
        self.store
            .transition(operation_id, OperationStatus::Running, "running", "running", None)
            .map_err(|_| OperationError::Internal)?;
        let result = match (payload.before.as_ref(), payload.after.as_ref()) {
            (Some(before), Some(after)) => {
                self.backend.replace_document_if_current(&payload.target, before, after)
            }
            (Some(before), None) => {
                self.backend.delete_document_if_current(&payload.target, before)
            }
            (None, Some(after)) => self.backend.insert_document_if_absent(&payload.target, after),
            (None, None) => unreachable!(),
        };
        match result {
            Ok(()) => {
                if self
                    .store
                    .transition(
                        operation_id,
                        OperationStatus::Completed,
                        "completed",
                        "completed",
                        None,
                    )
                    .is_err()
                {
                    return Err(OperationError::Uncertain { operation_id });
                }
                Ok(operation_id)
            }
            Err(BackendError::Conflict) => {
                let _ = self.store.transition(
                    operation_id,
                    OperationStatus::Conflict,
                    "conflict",
                    "conflict",
                    Some("Current document did not match the expected image."),
                );
                Err(OperationError::Conflict { operation_id })
            }
            Err(BackendError::Unavailable | BackendError::Failed) => {
                let _ = self.store.transition(
                    operation_id,
                    OperationStatus::Uncertain,
                    "outcome_uncertain",
                    "uncertain",
                    Some("The write outcome could not be confirmed."),
                );
                Err(OperationError::Uncertain { operation_id })
            }
        }
    }

    #[cfg(test)]
    pub(super) fn prepare_for_test(
        &self,
        target: super::model::DocumentTarget,
        before: mongodb::bson::Document,
        after: mongodb::bson::Document,
    ) -> OperationId {
        self.prepare_transition(
            OperationContext::user(),
            OperationKind::ReplaceDocument,
            RecoveryPayload { target, before: Some(before), after: Some(after) },
            None,
            None,
        )
        .unwrap()
    }

    #[cfg(test)]
    pub(super) fn prepare_delete_for_test(
        &self,
        target: super::model::DocumentTarget,
        before: mongodb::bson::Document,
    ) -> OperationId {
        self.prepare_transition(
            OperationContext::user(),
            OperationKind::DeleteDocument,
            RecoveryPayload { target, before: Some(before), after: None },
            None,
            None,
        )
        .unwrap()
    }

    #[cfg(test)]
    pub(super) fn store_path(&self) -> &std::path::Path {
        self.store.path()
    }
}

fn operation_preview(payload: &RecoveryPayload) -> OperationPreview {
    let mut changes = Vec::new();
    if let Some(before) = &payload.before {
        for (field, before_value) in before {
            if field == "_id" {
                continue;
            }
            let after = payload.after.as_ref().and_then(|document| document.get(field));
            if after != Some(before_value) {
                changes.push(OperationChangePreview {
                    field: field.clone(),
                    before: Some(bson_value_preview(before_value, 32)),
                    after: after.map(|value| bson_value_preview(value, 32)),
                });
            }
        }
    }
    if let Some(after) = &payload.after {
        for (field, after_value) in after {
            if field != "_id"
                && payload.before.as_ref().is_none_or(|document| !document.contains_key(field))
            {
                changes.push(OperationChangePreview {
                    field: field.clone(),
                    before: None,
                    after: Some(bson_value_preview(after_value, 32)),
                });
            }
        }
    }
    let total_changes = changes.len();
    changes.truncate(2);
    OperationPreview {
        document_id: bson_value_preview(&payload.target.id, 48),
        changes,
        total_changes,
    }
}
