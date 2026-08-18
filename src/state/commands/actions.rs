use gpui::{App, AppContext as _, AsyncApp, Entity};
use uuid::Uuid;

use crate::actions::ApprovalValidation;
use crate::actions::model::{
    ACTION_POLICY_VERSION, ActionPolicySnapshot, ActionRequest, ActionStatus, OperationPhase,
    OperationRecord, OperationStatus, ProposedAction,
};
use crate::connection::types::CancellationToken;
use crate::mcp::policy::PolicyEvaluator;
use crate::state::{AppEvent, AppState, StatusMessage};
use crate::sync::{
    ExecutionConnections,
    plan::{RuntimeActionConnection, database_fingerprint},
};

use super::AppCommands;

impl AppCommands {
    pub fn approve_agent_action(state: Entity<AppState>, action_id: Uuid, cx: &mut App) {
        let initial = state.read(cx).action_broker().store().load_action(action_id);
        let Ok(action) = initial else {
            report(&state, "The action no longer exists", cx);
            return;
        };
        let (source_id, target_id, target_writable) = action_connection_ids(&action);
        let (document_session, document_operation_ids) = match &action.content.request {
            ActionRequest::DocumentTransitions {
                connection_id,
                database,
                collection,
                operation_ids,
                ..
            } => (
                Some(crate::state::SessionKey::new(
                    *connection_id,
                    database.clone(),
                    collection.clone(),
                )),
                operation_ids.clone(),
            ),
            _ => (None, Vec::new()),
        };
        let document_action = document_session.is_some();
        let initial_connections = {
            let state_ref = state.read(cx);
            let policy = PolicyEvaluator::new(state_ref);
            let source =
                source_id.map(|id| policy.authorize_action_connection(id, false)).transpose();
            let target = policy.authorize_action_connection(target_id, target_writable);
            source.and_then(|source| target.map(|target| (source, target)))
        };
        let Ok((_, initial_target)) = initial_connections else {
            report(&state, "Action preconditions are no longer available", cx);
            return;
        };
        let target_database = action.content.preview.target_database.clone();
        let runtime = state.read(cx).connection_manager().runtime_handle();
        let target_state =
            fingerprint_on_runtime(&runtime, initial_target, target_database.clone());

        cx.spawn(async move |cx: &mut AsyncApp| {
            let target_state = target_state
                .await
                .map_err(|_| "Database preflight task stopped unexpectedly".to_string())
                .and_then(|result| result);
            let prepared = cx.update(|cx| -> Result<_, String> {
                let target_state = target_state?;
                let state_ref = state.read(cx);
                let policy = PolicyEvaluator::new(state_ref);
                let source = source_id
                    .map(|id| policy.authorize_action_connection(id, false))
                    .transpose()?;
                let target = policy.authorize_action_connection(target_id, target_writable)?;
                let broker = state_ref.action_broker();
                let recovery_clear =
                    matches!(action.content.request, ActionRequest::DatabaseBackup { .. })
                        || broker
                            .store()
                            .list_operations()
                            .map_err(|error| error.to_string())?
                            .into_iter()
                            .all(|operation| {
                                !operation.recovery_interlock
                                    || operation.target_connection_id != target_id
                                    || operation.target_database != target_database
                            });
                let document_engine = if document_action {
                    let engine = state_ref.operation_engine();
                    if engine.is_some() {
                        state_ref
                            .operation_backend()
                            .register_client(target_id, target.client.clone());
                    }
                    engine
                } else {
                    None
                };
                let prerequisites_satisfied = if document_action {
                    recovery_clear
                        && document_engine.is_some()
                        && state_ref.connection_reversible_history(target_id)
                } else {
                    recovery_clear
                        && crate::connection::tools::mongodump_path().is_some()
                        && crate::connection::tools::mongorestore_path().is_some()
                };
                let validation = ApprovalValidation {
                    source_identity_hash: source
                        .as_ref()
                        .map(|source| source.snapshot.identity_hash.clone()),
                    target_identity_hash: target.snapshot.identity_hash.clone(),
                    target_state_hash: target_state.hash,
                    policy: ActionPolicySnapshot {
                        version: ACTION_POLICY_VERSION,
                        source_shared: source
                            .as_ref()
                            .is_none_or(|source| source.snapshot.agent_shared),
                        target_shared: target.snapshot.agent_shared,
                        target_writable: !target.snapshot.read_only,
                        target_protected: target.snapshot.protected,
                    },
                    prerequisites_satisfied,
                };
                let executor = state_ref.sync_executor();
                let lease = executor.reserve_target(target_id, &target_database)?;
                let (_, operation) = broker
                    .approve_and_create_operation(action_id, "local_user", validation)
                    .map_err(|error| error.to_string())?;
                let cancellation = CancellationToken::new();
                broker
                    .register_cancellation(operation.id, cancellation.clone())
                    .map_err(|error| error.to_string())?;
                Ok((
                    operation.id,
                    executor,
                    broker,
                    state_ref.connection_manager().runtime_handle(),
                    ExecutionConnections { source, target },
                    cancellation,
                    lease,
                    document_engine,
                ))
            });
            let Ok(Ok((
                operation_id,
                executor,
                broker,
                runtime,
                connections,
                cancellation,
                lease,
                document_engine,
            ))) = prepared
            else {
                if !document_operation_ids.is_empty()
                    && let Ok(Some(engine)) = cx.update(|cx| {
                        let state = state.read(cx);
                        let broker = state.action_broker();
                        let action = broker.store().load_action(action_id).ok()?;
                        if !action_requires_checkpoint_cleanup(action.status) {
                            return None;
                        }
                        if let Some(operation_id) = action.operation_id
                            && let Ok(mut operation) = broker.store().load_operation(operation_id)
                            && operation.status == OperationStatus::Queued
                        {
                            operation.status = OperationStatus::Failed;
                            operation.public_error_code = Some("approval_setup_failed".to_string());
                            operation.updated_at = chrono::Utc::now();
                            operation.completed_at = Some(operation.updated_at);
                            let _ = broker.store().save_operation(&operation);
                        }
                        state.operation_engine()
                    })
                {
                    let operation_ids = document_operation_ids.clone();
                    cx.background_spawn(async move {
                        for operation_id in operation_ids {
                            let _ = engine.cancel_pending(operation_id);
                        }
                    })
                    .detach();
                }
                let message = prepared
                    .ok()
                    .and_then(Result::err)
                    .unwrap_or_else(|| "Action approval failed".to_string());
                let _ = cx.update(|cx| report(&state, &message, cx));
                return;
            };
            let _ = cx.update(|cx| {
                state.update(cx, |_state, cx| {
                    cx.emit(AppEvent::AgentActivityChanged);
                    cx.notify();
                });
            });
            let broker_for_execution = broker.clone();
            let result = runtime
                .spawn_blocking(move || {
                    if let Some(engine) = document_engine {
                        execute_document_operation(
                            &broker_for_execution,
                            &engine,
                            operation_id,
                            cancellation,
                            lease,
                        )
                    } else {
                        executor.execute(operation_id, connections, cancellation, lease)
                    }
                })
                .await
                .map_err(|_| "Operation task stopped unexpectedly".to_string())
                .and_then(|result| result);
            broker.unregister_cancellation(operation_id);
            let _ = cx.update(|cx| {
                state.update(cx, |state, cx| {
                    state.set_status_message(Some(match result {
                        Ok(operation) => StatusMessage::info(format!(
                            "Agent operation {}",
                            operation_status_label(operation.status)
                        )),
                        Err(error) => StatusMessage::error(error),
                    }));
                    cx.emit(AppEvent::AgentActivityChanged);
                    cx.notify();
                });
                if let Some(session_key) = document_session {
                    AppCommands::load_documents_for_session(state.clone(), session_key.clone(), cx);
                    AppCommands::collection_history_changed(state.clone(), session_key, cx);
                } else {
                    AppCommands::refresh_databases(state.clone(), target_id, cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn schedule_document_action_expiry(
        state: Entity<AppState>,
        action: ProposedAction,
        cx: &mut App,
    ) {
        let ActionRequest::DocumentTransitions {
            connection_id,
            database,
            collection,
            operation_ids,
            ..
        } = action.content.request
        else {
            return;
        };
        if action.status != ActionStatus::PendingApproval {
            return;
        }
        let delay = (action.expires_at - chrono::Utc::now()).to_std().unwrap_or_default();
        let action_id = action.id;
        cx.spawn(async move |cx: &mut AsyncApp| {
            gpui::Timer::after(delay).await;
            let cleanup = cx.update(|cx| {
                let broker = state.read(cx).action_broker();
                let Ok(action) = broker.expire_action_if_needed(action_id) else {
                    return None;
                };
                if action.status != ActionStatus::Expired {
                    return None;
                }
                state.read(cx).operation_engine().map(|engine| {
                    (
                        engine,
                        operation_ids,
                        crate::state::SessionKey::new(connection_id, database, collection),
                    )
                })
            });
            if let Ok(Some((engine, operation_ids, session_key))) = cleanup {
                let task = cx.background_spawn(async move {
                    for operation_id in operation_ids {
                        let _ = engine.cancel_pending(operation_id);
                    }
                });
                task.await;
                let _ = cx.update(|cx| {
                    state.update(cx, |_state, cx| {
                        cx.emit(AppEvent::AgentActivityChanged);
                        cx.notify();
                    });
                    AppCommands::collection_history_changed(state, session_key, cx);
                });
            }
        })
        .detach();
    }

    pub fn cancel_agent_operation(state: Entity<AppState>, operation_id: Uuid, cx: &mut App) {
        let result = state.read(cx).action_broker().cancel_operation_from_ui(operation_id);
        state.update(cx, |state, cx| {
            state.set_status_message(Some(match result {
                Ok(_) => StatusMessage::info("Cancellation requested"),
                Err(error) => StatusMessage::error(error.to_string()),
            }));
            cx.emit(AppEvent::AgentActivityChanged);
            cx.notify();
        });
    }

    pub fn reject_agent_action(state: Entity<AppState>, action_id: Uuid, cx: &mut App) {
        let (pending, engine) = {
            let state = state.read(cx);
            let pending =
                state.action_broker().store().load_action(action_id).ok().and_then(|action| {
                    match action.content.request {
                        ActionRequest::DocumentTransitions {
                            connection_id,
                            database,
                            collection,
                            operation_ids,
                            ..
                        } => Some((
                            operation_ids,
                            crate::state::SessionKey::new(connection_id, database, collection),
                        )),
                        _ => None,
                    }
                });
            (pending, state.operation_engine())
        };
        let result = state.read(cx).action_broker().reject(action_id, "local_user", None);
        if result.is_ok()
            && let (Some((operation_ids, session_key)), Some(engine)) = (pending, engine)
        {
            let task = cx.background_spawn(async move {
                for operation_id in operation_ids {
                    let _ = engine.cancel_pending(operation_id);
                }
            });
            let state = state.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                task.await;
                let _ = cx.update(|cx| {
                    AppCommands::collection_history_changed(state, session_key, cx);
                });
            })
            .detach();
        }
        state.update(cx, |state, cx| {
            state.set_status_message(Some(match result {
                Ok(_) => StatusMessage::info("Agent action rejected"),
                Err(error) => StatusMessage::error(error.to_string()),
            }));
            cx.emit(AppEvent::AgentActivityChanged);
            cx.notify();
        });
    }
}

impl AppCommands {
    pub(crate) fn reconcile_document_action_operations(
        engine: &crate::operations::OperationEngine,
        store: &crate::actions::ActionStore,
    ) -> Result<(), String> {
        for mut operation in store.list_operations().map_err(|error| error.to_string())? {
            if !matches!(
                operation.status,
                OperationStatus::Interrupted | OperationStatus::RecoveryRequired
            ) {
                continue;
            }
            let ActionRequest::DocumentTransitions { ref operation_ids, .. } = operation.request
            else {
                continue;
            };
            let mut statuses = Vec::with_capacity(operation_ids.len());
            for operation_id in operation_ids {
                let Some(details) = engine.get(*operation_id).map_err(|_| {
                    "Document history is unavailable during action reconciliation".to_string()
                })?
                else {
                    statuses.clear();
                    break;
                };
                statuses.push(details.summary.status);
            }
            if statuses.len() != operation_ids.len()
                || statuses.iter().any(|status| {
                    matches!(
                        status,
                        crate::operations::OperationStatus::PendingApproval
                            | crate::operations::OperationStatus::Prepared
                            | crate::operations::OperationStatus::Running
                            | crate::operations::OperationStatus::Uncertain
                            | crate::operations::OperationStatus::RecoveryRequired
                    )
                })
            {
                continue;
            }
            let completed = statuses
                .iter()
                .filter(|status| **status == crate::operations::OperationStatus::Completed)
                .count() as u64;
            let all_completed = completed == statuses.len() as u64;
            operation.status =
                if all_completed { OperationStatus::Completed } else { OperationStatus::Failed };
            operation.progress.documents_total = statuses.len() as u64;
            operation.progress.documents_processed = completed;
            if all_completed {
                operation.progress.phase = OperationPhase::Completed;
            }
            operation.target_mutation_started = completed > 0;
            operation.recovery_interlock = false;
            operation.public_error_code =
                (!all_completed).then(|| "document_write_reconciled_partial".to_string());
            operation.updated_at = chrono::Utc::now();
            operation.completed_at = Some(operation.updated_at);
            store.save_operation(&operation).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

fn action_requires_checkpoint_cleanup(status: ActionStatus) -> bool {
    status != ActionStatus::PendingApproval
}

fn fingerprint_on_runtime(
    runtime: &tokio::runtime::Handle,
    connection: RuntimeActionConnection,
    database: String,
) -> tokio::task::JoinHandle<Result<crate::actions::model::DatabaseStateFingerprint, String>> {
    runtime.spawn(async move { database_fingerprint(&connection, &database).await })
}

fn execute_document_operation(
    broker: &std::sync::Arc<crate::actions::ActionBroker>,
    engine: &std::sync::Arc<crate::operations::OperationEngine>,
    operation_id: Uuid,
    cancellation: CancellationToken,
    _lease: crate::sync::executor::TargetMutationLease,
) -> Result<OperationRecord, String> {
    let store = broker.store();
    let mut operation = store
        .load_operation(operation_id)
        .map_err(|_| "Agent operation store is unavailable".to_string())?;
    let action = store
        .load_action(operation.action_id)
        .map_err(|_| "Agent action store is unavailable".to_string())?;
    let expected_hash = crate::actions::content_hash(&action.content)
        .map_err(|_| "Agent action could not be verified".to_string())?;
    if operation.action_hash != action.content_hash
        || action.content_hash != expected_hash
        || action.status != ActionStatus::Accepted
        || action.operation_id != Some(operation.id)
    {
        operation.status = OperationStatus::Failed;
        operation.public_error_code = Some("action_hash_mismatch".to_string());
        operation.updated_at = chrono::Utc::now();
        operation.completed_at = Some(operation.updated_at);
        store
            .save_operation(&operation)
            .map_err(|_| "Agent operation could not be updated".to_string())?;
        return Ok(operation);
    }
    let ActionRequest::DocumentTransitions { operation_ids, .. } = operation.request.clone() else {
        return Err("Document operation request is invalid".to_string());
    };
    operation.status = OperationStatus::Running;
    operation.progress.phase = OperationPhase::ApplyingDocuments;
    operation.progress.documents_total = operation_ids.len() as u64;
    operation.updated_at = chrono::Utc::now();
    store
        .save_operation(&operation)
        .map_err(|_| "Agent operation could not be updated".to_string())?;

    for (index, transition_id) in operation_ids.iter().enumerate() {
        if cancellation.is_cancelled() {
            for pending in &operation_ids[index..] {
                let _ = engine.cancel_pending(*pending);
            }
            operation.status = OperationStatus::Cancelled;
            operation.updated_at = chrono::Utc::now();
            operation.completed_at = Some(operation.updated_at);
            store
                .save_operation(&operation)
                .map_err(|_| "Agent operation could not be updated".to_string())?;
            return Ok(operation);
        }
        operation.target_mutation_started = true;
        operation.updated_at = chrono::Utc::now();
        store
            .save_operation(&operation)
            .map_err(|_| "Agent operation could not be updated".to_string())?;
        if let Err(error) = engine.apply_approved(*transition_id) {
            for pending in &operation_ids[index + 1..] {
                let _ = engine.cancel_pending(*pending);
            }
            let uncertain = matches!(error, crate::operations::OperationError::Uncertain { .. });
            operation.status =
                if uncertain { OperationStatus::RecoveryRequired } else { OperationStatus::Failed };
            operation.recovery_interlock = uncertain;
            operation.public_error_code = Some(
                if uncertain { "document_write_uncertain" } else { "document_write_conflict" }
                    .to_string(),
            );
            operation.warnings.push(error.user_message().to_string());
            operation.updated_at = chrono::Utc::now();
            operation.completed_at = Some(operation.updated_at);
            store
                .save_operation(&operation)
                .map_err(|_| "Agent operation could not be updated".to_string())?;
            return Ok(operation);
        }
        operation.progress.documents_processed = (index + 1) as u64;
        operation.updated_at = chrono::Utc::now();
        store
            .save_operation(&operation)
            .map_err(|_| "Agent operation could not be updated".to_string())?;
    }

    operation.status = OperationStatus::Completed;
    operation.progress.phase = OperationPhase::Completed;
    operation.updated_at = chrono::Utc::now();
    operation.completed_at = Some(operation.updated_at);
    store
        .save_operation(&operation)
        .map_err(|_| "Agent operation could not be updated".to_string())?;
    Ok(operation)
}

fn action_connection_ids(action: &ProposedAction) -> (Option<Uuid>, Uuid, bool) {
    match action.content.request {
        ActionRequest::DatabaseBackup { connection_id, .. } => (None, connection_id, false),
        ActionRequest::DatabaseSync { source_connection_id, target_connection_id, .. } => {
            (Some(source_connection_id), target_connection_id, true)
        }
        ActionRequest::OperationRevert { .. } => {
            (None, action.content.preview.target.connection_id, true)
        }
        ActionRequest::DocumentTransitions { connection_id, .. } => (None, connection_id, true),
    }
}

fn operation_status_label(status: crate::actions::model::OperationStatus) -> &'static str {
    match status {
        crate::actions::model::OperationStatus::Completed => "completed",
        crate::actions::model::OperationStatus::Cancelled => "cancelled",
        crate::actions::model::OperationStatus::RecoveryRequired => "requires recovery",
        crate::actions::model::OperationStatus::Failed => "failed",
        _ => "updated",
    }
}

fn report(state: &Entity<AppState>, message: &str, cx: &mut App) {
    state.update(cx, |state, cx| {
        state.set_status_message(Some(StatusMessage::error(message)));
        cx.notify();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::model::{
        ActionOrigin, ActionOriginKind, ActionPolicySnapshot, ActionPrerequisites, ActionPreview,
        ConnectionActionSnapshot, DatabaseStateFingerprint, DocumentActionKind,
        ProposedActionContent,
    };
    use crate::connection::ConnectionManager;

    #[test]
    fn transient_approval_failures_keep_pending_checkpoints_for_retry() {
        assert!(!action_requires_checkpoint_cleanup(ActionStatus::PendingApproval));
        assert!(action_requires_checkpoint_cleanup(ActionStatus::Stale));
        assert!(action_requires_checkpoint_cleanup(ActionStatus::Accepted));
    }

    #[test]
    fn approval_fingerprint_runs_on_the_mongodb_runtime() {
        let manager = ConnectionManager::new();
        let runtime = manager.runtime_handle();
        let client =
            runtime.block_on(mongodb::Client::with_uri_str("mongodb://127.0.0.1:1")).unwrap();
        let connection = RuntimeActionConnection {
            client,
            tool_uri: String::new(),
            snapshot: ConnectionActionSnapshot {
                connection_id: Uuid::new_v4(),
                display_name: "Test".into(),
                environment: None,
                protected: false,
                read_only: false,
                agent_shared: true,
                connected: true,
                identity_hash: "test".into(),
            },
            databases: Vec::new(),
        };

        let fingerprint = runtime
            .block_on(fingerprint_on_runtime(&runtime, connection, "missing".into()))
            .unwrap()
            .unwrap();

        assert!(!fingerprint.exists);
    }

    #[test]
    fn approved_document_action_applies_prepared_mcp_transition() {
        let directory = tempfile::TempDir::new().unwrap();
        let backend = std::sync::Arc::new(crate::operations::InMemoryMutationBackend::default());
        let engine = std::sync::Arc::new(
            crate::operations::OperationEngine::open(
                directory.path().join("history.sqlite3"),
                [41; 32],
                backend.clone(),
            )
            .unwrap(),
        );
        let connection_id = Uuid::new_v4();
        let target = crate::operations::DocumentTarget {
            connection_id,
            connection_name: "Local".into(),
            database: "app".into(),
            collection: "users".into(),
            id: 1.into(),
        };
        let transition = engine
            .prepare_for_approval(
                crate::operations::OperationContext::mcp(),
                crate::operations::Mutation::InsertDocument {
                    target: target.clone(),
                    document: mongodb::bson::doc! { "_id": 1, "name": "Ada" },
                },
            )
            .unwrap();
        let store =
            std::sync::Arc::new(crate::actions::ActionStore::new(directory.path().join("actions")));
        let broker = std::sync::Arc::new(crate::actions::ActionBroker::new(store.clone()));
        let snapshot = ConnectionActionSnapshot {
            connection_id,
            display_name: "Local".into(),
            environment: None,
            protected: false,
            read_only: false,
            agent_shared: true,
            connected: true,
            identity_hash: "identity".into(),
        };
        let policy = ActionPolicySnapshot {
            version: ACTION_POLICY_VERSION,
            source_shared: true,
            target_shared: true,
            target_writable: true,
            target_protected: false,
        };
        let fingerprint = DatabaseStateFingerprint {
            exists: true,
            collections: vec!["users".into()],
            estimated_documents: 0,
            estimated_bytes: 0,
            hash: "state".into(),
        };
        let action = broker
            .propose(ProposedActionContent {
                request: ActionRequest::DocumentTransitions {
                    connection_id,
                    database: "app".into(),
                    collection: "users".into(),
                    action: DocumentActionKind::Insert,
                    operation_ids: vec![transition],
                },
                origin: ActionOrigin {
                    kind: ActionOriginKind::Mcp,
                    client_grant_id: Some(Uuid::new_v4()),
                    client_label: Some("Test".into()),
                    session_id: None,
                },
                policy: policy.clone(),
                preview: ActionPreview {
                    summary: "Insert one document".into(),
                    source: None,
                    target: snapshot,
                    source_database: None,
                    target_database: "app".into(),
                    mode: None,
                    estimated_documents: 1,
                    estimated_bytes: 32,
                    warnings: Vec::new(),
                    backup_behavior: "Encrypted checkpoint".into(),
                    rollback_behavior: "Revert from History".into(),
                },
                prerequisites: ActionPrerequisites {
                    database_tools_available: false,
                    source_reachable: true,
                    target_reachable: true,
                    backup_storage_available: false,
                    free_space_known_sufficient: Some(true),
                },
                source_state_fingerprint: None,
                target_state_fingerprint: fingerprint,
            })
            .unwrap();
        assert!(!serde_json::to_string(&action).unwrap().contains("Ada"));
        let (_, operation) = broker
            .approve_and_create_operation(
                action.id,
                "local_user",
                ApprovalValidation {
                    source_identity_hash: None,
                    target_identity_hash: "identity".into(),
                    target_state_hash: "state".into(),
                    policy,
                    prerequisites_satisfied: true,
                },
            )
            .unwrap();
        let executor = crate::sync::SyncExecutor::new(
            std::sync::Arc::new(ConnectionManager::new()),
            store.clone(),
        );
        let lease = executor.reserve_target(connection_id, "app").unwrap();

        let result = execute_document_operation(
            &broker,
            &engine,
            operation.id,
            CancellationToken::new(),
            lease,
        )
        .unwrap();

        assert_eq!(result.status, OperationStatus::Completed);
        assert!(result.target_mutation_started);
        assert_eq!(
            backend.document(&target),
            Some(mongodb::bson::doc! { "_id": 1, "name": "Ada" })
        );
        assert_eq!(
            engine.get(transition).unwrap().unwrap().summary.origin,
            crate::operations::OperationOrigin::Mcp
        );

        let mut interrupted = result;
        interrupted.status = OperationStatus::RecoveryRequired;
        interrupted.recovery_interlock = true;
        store.save_operation(&interrupted).unwrap();
        AppCommands::reconcile_document_action_operations(&engine, &store).unwrap();
        let reconciled = store.load_operation(interrupted.id).unwrap();
        assert_eq!(reconciled.status, OperationStatus::Completed);
        assert!(!reconciled.recovery_interlock);
    }
}
