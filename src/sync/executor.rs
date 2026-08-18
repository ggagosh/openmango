use std::collections::HashSet;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use uuid::Uuid;

use crate::actions::ActionStore;
use crate::actions::model::{
    ActionRequest, BackupManifest, OperationPhase, OperationRecord, OperationStatus,
};
use crate::connection::ConnectionManager;
use crate::connection::types::{BsonToolProgress, BsonToolRunOutcome, CancellationToken};
use crate::sync::backup::{create_verified_backup, create_verified_source_dump};
use crate::sync::plan::{RuntimeActionConnection, database_fingerprint};

pub struct ExecutionConnections {
    pub source: Option<RuntimeActionConnection>,
    pub target: RuntimeActionConnection,
}

pub struct SyncExecutor {
    manager: Arc<ConnectionManager>,
    store: Arc<ActionStore>,
    target_leases: Arc<Mutex<HashSet<(Uuid, String)>>>,
}

pub struct TargetMutationLease {
    leases: Arc<Mutex<HashSet<(Uuid, String)>>>,
    key: Option<(Uuid, String)>,
}

impl Drop for TargetMutationLease {
    fn drop(&mut self) {
        if let Some(key) = self.key.take()
            && let Ok(mut leases) = self.leases.lock()
        {
            leases.remove(&key);
        }
    }
}

impl SyncExecutor {
    pub fn new(manager: Arc<ConnectionManager>, store: Arc<ActionStore>) -> Self {
        Self { manager, store, target_leases: Arc::new(Mutex::new(HashSet::new())) }
    }

    pub fn reserve_target(
        &self,
        connection_id: Uuid,
        database: &str,
    ) -> Result<TargetMutationLease, String> {
        let key = (connection_id, database.to_string());
        let mut leases = self
            .target_leases
            .lock()
            .map_err(|_| "Target mutation leases are unavailable".to_string())?;
        if !leases.insert(key.clone()) {
            return Err("Another operation is already mutating this target database".into());
        }
        drop(leases);
        Ok(TargetMutationLease { leases: self.target_leases.clone(), key: Some(key) })
    }

    pub fn execute(
        &self,
        operation_id: Uuid,
        connections: ExecutionConnections,
        cancellation: CancellationToken,
        _lease: TargetMutationLease,
    ) -> Result<OperationRecord, String> {
        let mut operation = self.store.load_operation(operation_id).map_err(safe_error)?;
        let action = self.store.load_action(operation.action_id).map_err(safe_error)?;
        let expected_hash = crate::actions::content_hash(&action.content).map_err(safe_error)?;
        if operation.action_hash != action.content_hash
            || action.content_hash != expected_hash
            || action.status != crate::actions::model::ActionStatus::Accepted
            || action.operation_id != Some(operation.id)
        {
            return self.finish_failed(operation, "action_hash_mismatch", false);
        }
        self.execute_leased(&mut operation, connections, cancellation)
    }

    fn execute_leased(
        &self,
        operation: &mut OperationRecord,
        connections: ExecutionConnections,
        cancellation: CancellationToken,
    ) -> Result<OperationRecord, String> {
        self.set_phase(operation, OperationPhase::Preparing)?;
        match operation.request.clone() {
            ActionRequest::DatabaseBackup { database, .. } => {
                self.execute_backup(operation, &connections.target, &database, cancellation)
            }
            ActionRequest::DatabaseSync { source_database, target_database, .. } => {
                let source = connections
                    .source
                    .as_ref()
                    .ok_or_else(|| "Source connection is unavailable".to_string())?;
                self.execute_sync(
                    operation,
                    source,
                    &connections.target,
                    &source_database,
                    &target_database,
                    cancellation,
                )
            }
            ActionRequest::OperationRevert { operation_id } => {
                self.execute_revert(operation, operation_id, &connections.target, cancellation)
            }
            ActionRequest::DocumentTransitions { .. } => {
                Err("Document transitions require the document operation executor".to_string())
            }
        }
    }

    fn execute_backup(
        &self,
        operation: &mut OperationRecord,
        target: &RuntimeActionConnection,
        database: &str,
        cancellation: CancellationToken,
    ) -> Result<OperationRecord, String> {
        if let Err(error) = self.verify_target_precondition(operation, target, database) {
            return self.finish_failed(operation.clone(), &error, false);
        }
        self.set_phase(operation, OperationPhase::DumpingDatabase)?;
        let progress = self.progress_sink(operation.id, OperationPhase::DumpingDatabase);
        match create_verified_backup(
            &self.manager,
            &self.store,
            target,
            database,
            operation.id,
            cancellation,
            progress,
        ) {
            Ok(manifest) => {
                operation.backup_id = Some(manifest.backup_id);
                self.finish_completed(operation.clone())
            }
            Err(code) if code == "backup_cancelled" => self.finish_cancelled(operation.clone()),
            Err(code) => self.finish_failed(
                operation.clone(),
                &code,
                code == "backup_cancellation_unconfirmed",
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_sync(
        &self,
        operation: &mut OperationRecord,
        source: &RuntimeActionConnection,
        target: &RuntimeActionConnection,
        source_database: &str,
        target_database: &str,
        cancellation: CancellationToken,
    ) -> Result<OperationRecord, String> {
        self.set_phase(operation, OperationPhase::DumpingSource)?;
        let source_dump = match create_verified_source_dump(
            &self.manager,
            &self.store,
            source,
            source_database,
            operation.id,
            cancellation.clone(),
            self.progress_sink(operation.id, OperationPhase::DumpingSource),
        ) {
            Ok(dump) => dump,
            Err(code) if code == "source_dump_cancelled" => {
                return self.finish_cancelled(operation.clone());
            }
            Err(code) => {
                return self.finish_failed(
                    operation.clone(),
                    &code,
                    code == "source_dump_cancellation_unconfirmed",
                );
            }
        };
        self.set_phase(operation, OperationPhase::ValidatingSourceDump)?;
        let action = self.store.load_action(operation.action_id).map_err(safe_error)?;
        if action
            .content
            .source_state_fingerprint
            .as_ref()
            .is_none_or(|fingerprint| fingerprint.collections != source_dump.collections)
        {
            let _ = fs::remove_dir_all(self.store.source_dump_dir(operation.id));
            return self.finish_failed(operation.clone(), "source_state_changed", false);
        }
        if cancellation.is_cancelled() {
            return self.finish_cancelled(operation.clone());
        }
        self.set_phase(operation, OperationPhase::CheckingTargetPrecondition)?;
        if let Err(error) = self.verify_target_precondition(operation, target, target_database) {
            let _ = fs::remove_dir_all(self.store.source_dump_dir(operation.id));
            return self.finish_failed(operation.clone(), &error, false);
        }

        self.set_phase(operation, OperationPhase::BackingUpTarget)?;
        let target_backup = match create_verified_backup(
            &self.manager,
            &self.store,
            target,
            target_database,
            operation.id,
            cancellation.clone(),
            self.progress_sink(operation.id, OperationPhase::BackingUpTarget),
        ) {
            Ok(manifest) => manifest,
            Err(code) if code == "backup_cancelled" => {
                return self.finish_cancelled(operation.clone());
            }
            Err(code) => {
                return self.finish_failed(
                    operation.clone(),
                    &code,
                    code == "backup_cancellation_unconfirmed",
                );
            }
        };
        operation.backup_id = Some(target_backup.backup_id);
        self.store.save_operation(operation).map_err(safe_error)?;
        self.set_phase(operation, OperationPhase::VerifyingBackup)?;
        if !target_backup.verified {
            return self.finish_failed(operation.clone(), "target_backup_unverified", false);
        }
        if let Err(error) = self.verify_target_precondition(operation, target, target_database) {
            return self.finish_failed(operation.clone(), &error, false);
        }
        if cancellation.is_cancelled() {
            return self.finish_cancelled(operation.clone());
        }

        self.set_phase(operation, OperationPhase::ReplacingTarget)?;
        operation.target_mutation_started = true;
        self.store.save_operation(operation).map_err(safe_error)?;
        let replacement = self.replace_database(
            target,
            source_database,
            target_database,
            &source_dump.path,
            cancellation.clone(),
            self.progress_sink(operation.id, OperationPhase::ReplacingTarget),
        );
        let replacement = replacement.and_then(|()| {
            self.set_phase(operation, OperationPhase::VerifyingTarget)?;
            self.verify_target_namespaces(target, target_database, &source_dump.collections)
        });
        if replacement.as_ref().err().map(String::as_str) == Some("process_termination_unconfirmed")
        {
            let _ = fs::remove_dir_all(self.store.source_dump_dir(operation.id));
            return self.finish_failed(operation.clone(), "recovery_required", true);
        }
        match replacement {
            Ok(()) if !cancellation.is_cancelled() => {
                operation.warnings.push(
                    "Namespace inventory verified; document counts and index contents are not transactional guarantees"
                        .into(),
                );
                let _ = fs::remove_dir_all(self.store.source_dump_dir(operation.id));
                self.finish_completed(operation.clone())
            }
            result => {
                let cancelled = cancellation.is_cancelled();
                let recovery =
                    self.restore_manifest(operation, target, target_database, &target_backup);
                let _ = fs::remove_dir_all(self.store.source_dump_dir(operation.id));
                match recovery {
                    Ok(()) if cancelled => self.finish_cancelled(operation.clone()),
                    Ok(()) => self.finish_failed(
                        operation.clone(),
                        result.err().as_deref().unwrap_or("replacement_failed"),
                        false,
                    ),
                    Err(_) => self.finish_failed(operation.clone(), "recovery_required", true),
                }
            }
        }
    }

    fn execute_revert(
        &self,
        operation: &mut OperationRecord,
        original_operation_id: Uuid,
        target: &RuntimeActionConnection,
        cancellation: CancellationToken,
    ) -> Result<OperationRecord, String> {
        let target_database = operation.target_database.clone();
        if let Err(error) = self.verify_target_precondition(operation, target, &target_database) {
            return self.finish_failed(operation.clone(), &error, false);
        }
        let original = self.store.load_operation(original_operation_id).map_err(safe_error)?;
        let recovery_backup_id = match original.request {
            ActionRequest::DatabaseSync { .. } => original.backup_id,
            ActionRequest::OperationRevert { .. } => original.safety_backup_id,
            ActionRequest::DatabaseBackup { .. } | ActionRequest::DocumentTransitions { .. } => {
                None
            }
        }
        .ok_or_else(|| "Original operation has no recovery backup".to_string())?;
        let recovery = self.store.load_backup_manifest(recovery_backup_id).map_err(safe_error)?;
        if !recovery.verified
            || recovery.database != operation.target_database
            || recovery.connection_identity_hash != target.snapshot.identity_hash
        {
            return self.finish_failed(operation.clone(), "recovery_backup_unverified", false);
        }

        self.set_phase(operation, OperationPhase::BackingUpTarget)?;
        let safety_backup = match create_verified_backup(
            &self.manager,
            &self.store,
            target,
            &operation.target_database,
            operation.id,
            cancellation.clone(),
            self.progress_sink(operation.id, OperationPhase::BackingUpTarget),
        ) {
            Ok(manifest) => manifest,
            Err(code) if code == "backup_cancelled" => {
                return self.finish_cancelled(operation.clone());
            }
            Err(code) => {
                return self.finish_failed(
                    operation.clone(),
                    &code,
                    code == "backup_cancellation_unconfirmed",
                );
            }
        };
        operation.safety_backup_id = Some(safety_backup.backup_id);
        self.store.save_operation(operation).map_err(safe_error)?;
        if let Err(error) = self.verify_target_precondition(operation, target, &target_database) {
            return self.finish_failed(operation.clone(), &error, false);
        }
        if cancellation.is_cancelled() {
            return self.finish_cancelled(operation.clone());
        }

        self.set_phase(operation, OperationPhase::ReplacingTarget)?;
        operation.target_mutation_started = true;
        self.store.save_operation(operation).map_err(safe_error)?;
        match self.restore_manifest(operation, target, &target_database, &recovery) {
            Ok(()) => self.finish_completed(operation.clone()),
            Err(_) => {
                match self.restore_manifest(operation, target, &target_database, &safety_backup) {
                    Ok(()) => self.finish_failed(operation.clone(), "revert_failed", false),
                    Err(_) => self.finish_failed(operation.clone(), "recovery_required", true),
                }
            }
        }
    }

    fn replace_database(
        &self,
        target: &RuntimeActionConnection,
        source_database: &str,
        database: &str,
        dump_path: &std::path::Path,
        cancellation: CancellationToken,
        progress: Arc<dyn Fn(BsonToolProgress) + Send + Sync>,
    ) -> Result<(), String> {
        self.drop_database(target, database)?;
        let callback = move |event| progress(event);
        match self
            .manager
            .import_database_bson_with_progress(
                &target.tool_uri,
                source_database,
                database,
                dump_path,
                false,
                cancellation,
                callback,
            )
            .map_err(safe_error)?
        {
            BsonToolRunOutcome::Completed => Ok(()),
            BsonToolRunOutcome::Cancelled { termination_succeeded: true } => {
                Err("replacement_cancelled".into())
            }
            BsonToolRunOutcome::Cancelled { termination_succeeded: false } => {
                Err("process_termination_unconfirmed".into())
            }
        }
    }

    fn restore_manifest(
        &self,
        operation: &mut OperationRecord,
        target: &RuntimeActionConnection,
        database: &str,
        manifest: &BackupManifest,
    ) -> Result<(), String> {
        self.set_phase(operation, OperationPhase::RestoringTargetBackup)?;
        if manifest.absence_marker {
            self.drop_database(target, database)?;
        } else {
            let dump = crate::sync::backup::backup_payload_path(&self.store, manifest);
            self.replace_database(
                target,
                &manifest.database,
                database,
                &dump,
                CancellationToken::new(),
                self.progress_sink(operation.id, OperationPhase::RestoringTargetBackup),
            )?;
        }
        self.set_phase(operation, OperationPhase::VerifyingRecovery)?;
        let expected = if manifest.absence_marker {
            Vec::new()
        } else {
            manifest.preflight_collections.clone()
        };
        self.verify_target_namespaces(target, database, &expected)
    }

    fn verify_target_precondition(
        &self,
        operation: &OperationRecord,
        target: &RuntimeActionConnection,
        database: &str,
    ) -> Result<(), String> {
        let action = self.store.load_action(operation.action_id).map_err(safe_error)?;
        let current =
            self.manager.runtime_handle().block_on(database_fingerprint(target, database))?;
        if current.hash != action.content.target_state_fingerprint.hash {
            return Err("target_state_changed".into());
        }
        Ok(())
    }

    fn verify_target_namespaces(
        &self,
        target: &RuntimeActionConnection,
        database: &str,
        expected: &[String],
    ) -> Result<(), String> {
        let mut actual = self.list_collection_names(target, database)?;
        actual.sort();
        let mut expected = expected.to_vec();
        expected.sort();
        if actual != expected {
            return Err("target_namespace_verification_failed".into());
        }
        Ok(())
    }

    fn drop_database(
        &self,
        target: &RuntimeActionConnection,
        database: &str,
    ) -> Result<(), String> {
        self.manager
            .runtime_handle()
            .block_on(async { target.client.database(database).drop().await })
            .map_err(safe_error)
    }

    fn list_collection_names(
        &self,
        target: &RuntimeActionConnection,
        database: &str,
    ) -> Result<Vec<String>, String> {
        self.manager
            .runtime_handle()
            .block_on(async { target.client.database(database).list_collection_names().await })
            .map_err(safe_error)
    }

    fn set_phase(
        &self,
        operation: &mut OperationRecord,
        phase: OperationPhase,
    ) -> Result<(), String> {
        operation.status = OperationStatus::Running;
        operation.progress.phase = phase;
        operation.updated_at = Utc::now();
        self.store.save_operation(operation).map_err(safe_error)
    }

    fn progress_sink(
        &self,
        operation_id: Uuid,
        phase: OperationPhase,
    ) -> Arc<dyn Fn(BsonToolProgress) + Send + Sync> {
        let store = self.store.clone();
        let last_write = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1)));
        Arc::new(move |event| {
            let Ok(mut last_write) = last_write.lock() else {
                return;
            };
            let complete = matches!(event, BsonToolProgress::Completed { .. });
            if !complete && last_write.elapsed() < Duration::from_millis(250) {
                return;
            }
            *last_write = Instant::now();
            let Ok(mut operation) = store.load_operation(operation_id) else {
                return;
            };
            operation.progress.phase = phase;
            match event {
                BsonToolProgress::Started { collection } => {
                    operation.progress.collection = Some(collection);
                }
                BsonToolProgress::Progress { collection, current, total, .. } => {
                    operation.progress.collection = Some(collection);
                    operation.progress.documents_processed = current;
                    operation.progress.documents_total = total;
                }
                BsonToolProgress::Completed { collection, documents } => {
                    operation.progress.collection = Some(collection);
                    operation.progress.documents_processed = documents;
                    operation.progress.documents_total = documents;
                }
            }
            operation.updated_at = Utc::now();
            let _ = store.save_operation(&operation);
        })
    }

    fn finish_completed(&self, mut operation: OperationRecord) -> Result<OperationRecord, String> {
        operation.status = OperationStatus::Completed;
        operation.progress.phase = OperationPhase::Completed;
        operation.completed_at = Some(Utc::now());
        operation.updated_at = Utc::now();
        operation.recovery_interlock = false;
        self.store.save_operation(&operation).map_err(safe_error)?;
        Ok(operation)
    }

    fn finish_cancelled(&self, mut operation: OperationRecord) -> Result<OperationRecord, String> {
        operation.status = OperationStatus::Cancelled;
        operation.completed_at = Some(Utc::now());
        operation.updated_at = Utc::now();
        operation.recovery_interlock = false;
        self.store.save_operation(&operation).map_err(safe_error)?;
        Ok(operation)
    }

    fn finish_failed(
        &self,
        mut operation: OperationRecord,
        code: &str,
        recovery_required: bool,
    ) -> Result<OperationRecord, String> {
        operation.status = if recovery_required {
            OperationStatus::RecoveryRequired
        } else {
            OperationStatus::Failed
        };
        operation.public_error_code = Some(code.to_string());
        operation.completed_at = Some(Utc::now());
        operation.updated_at = Utc::now();
        operation.recovery_interlock = recovery_required;
        self.store.save_operation(&operation).map_err(safe_error)?;
        Ok(operation)
    }
}

fn safe_error(_error: impl std::fmt::Display) -> String {
    "Sync operation failed; check OpenMango logs".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_mutation_lease_is_one_at_a_time_and_releases_on_drop() {
        let temp = tempfile::TempDir::new().unwrap();
        let executor = SyncExecutor::new(
            Arc::new(ConnectionManager::new()),
            Arc::new(ActionStore::new(temp.path().join("agent"))),
        );
        let connection_id = Uuid::new_v4();
        let lease = executor.reserve_target(connection_id, "app").unwrap();
        assert!(executor.reserve_target(connection_id, "app").is_err());
        assert!(executor.reserve_target(connection_id, "other").is_ok());
        drop(lease);
        assert!(executor.reserve_target(connection_id, "app").is_ok());
    }
}
