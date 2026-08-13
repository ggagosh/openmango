use gpui::{App, AsyncApp, Entity};
use uuid::Uuid;

use crate::actions::ApprovalValidation;
use crate::actions::model::{
    ACTION_POLICY_VERSION, ActionPolicySnapshot, ActionRequest, ProposedAction,
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
                let recovery_clear = matches!(
                    action.content.request,
                    ActionRequest::DatabaseBackup { .. }
                ) || broker
                    .store()
                    .list_operations()
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .all(|operation| {
                        !operation.recovery_interlock
                            || operation.target_connection_id != target_id
                            || operation.target_database != target_database
                    });
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
                    prerequisites_satisfied: recovery_clear
                        && crate::connection::tools::mongodump_path().is_some()
                        && crate::connection::tools::mongorestore_path().is_some(),
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
            ))) = prepared
            else {
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
            let result = runtime
                .spawn_blocking(move || {
                    executor.execute(operation_id, connections, cancellation, lease)
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
                AppCommands::refresh_databases(state.clone(), target_id, cx);
            });
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
        let result = state.read(cx).action_broker().reject(action_id, "local_user", None);
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

fn fingerprint_on_runtime(
    runtime: &tokio::runtime::Handle,
    connection: RuntimeActionConnection,
    database: String,
) -> tokio::task::JoinHandle<Result<crate::actions::model::DatabaseStateFingerprint, String>> {
    runtime.spawn(async move { database_fingerprint(&connection, &database).await })
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
    use crate::actions::model::ConnectionActionSnapshot;
    use crate::connection::ConnectionManager;

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
}
