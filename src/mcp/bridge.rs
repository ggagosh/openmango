use gpui::{AsyncApp, Context, Entity, WeakEntity};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use mongodb::Client;

use crate::actions::model::{
    BackupManifest, OperationRecord, ProposedAction, ProposedActionContent,
};
use crate::state::{AppEvent, AppState};
use crate::sync::plan::ActionPreflight;

use super::{McpConnection, policy::PolicyEvaluator};

#[derive(Clone)]
pub struct McpBridge {
    requests: mpsc::Sender<BridgeRequest>,
}

enum BridgeRequest {
    ListConnections(oneshot::Sender<Vec<McpConnection>>),
    ListDatabases {
        connection_id: Uuid,
        response: oneshot::Sender<Result<Vec<String>, String>>,
    },
    ResolveRead {
        connection_id: Uuid,
        response: oneshot::Sender<Result<Client, String>>,
    },
    ResolveAction {
        source_connection_id: Option<Uuid>,
        target_connection_id: Uuid,
        target_writable: bool,
        response: oneshot::Sender<Result<ActionPreflight, String>>,
    },
    ClientLabel {
        grant_id: Uuid,
        response: oneshot::Sender<Option<String>>,
    },
    ProposeAction {
        content: Box<ProposedActionContent>,
        response: oneshot::Sender<Result<ProposedAction, String>>,
    },
    GetAction {
        action_id: Uuid,
        grant_id: Uuid,
        response: oneshot::Sender<Result<ProposedAction, String>>,
    },
    ListActions {
        grant_id: Uuid,
        offset: usize,
        limit: usize,
        response: oneshot::Sender<Result<Vec<ProposedAction>, String>>,
    },
    GetOperation {
        operation_id: Uuid,
        grant_id: Uuid,
        response: oneshot::Sender<Result<OperationRecord, String>>,
    },
    GetBackupManifest {
        backup_id: Uuid,
        response: oneshot::Sender<Result<BackupManifest, String>>,
    },
    CancelOperation {
        operation_id: Uuid,
        grant_id: Uuid,
        response: oneshot::Sender<Result<OperationRecord, String>>,
    },
}

impl McpBridge {
    pub fn attach<V: 'static>(state: Entity<AppState>, cx: &mut Context<V>) -> Self {
        let (requests, mut receiver) = mpsc::channel(16);
        cx.spawn(async move |_view: WeakEntity<V>, cx: &mut AsyncApp| {
            while let Some(request) = receiver.recv().await {
                let result = cx.update(|cx| match request {
                    BridgeRequest::ListConnections(response) => {
                        let _ = response.send(shared_connections(state.read(cx)));
                    }
                    BridgeRequest::ListDatabases { connection_id, response } => {
                        let result = shared_databases(state.read(cx), connection_id);
                        let _ = response.send(result);
                    }
                    BridgeRequest::ResolveRead { connection_id, response } => {
                        let result = shared_client(state.read(cx), connection_id);
                        let _ = response.send(result);
                    }
                    BridgeRequest::ResolveAction {
                        source_connection_id,
                        target_connection_id,
                        target_writable,
                        response,
                    } => {
                        let state = state.read(cx);
                        let policy = PolicyEvaluator::new(state);
                        let result = (|| {
                            let source = source_connection_id
                                .map(|id| policy.authorize_action_connection(id, false))
                                .transpose()?;
                            let target = policy.authorize_action_connection(
                                target_connection_id,
                                target_writable,
                            )?;
                            let broker = state.action_broker();
                            let recovery_interlocks = broker
                                .store()
                                .list_operations()
                                .map_err(|error| error.to_string())?
                                .into_iter()
                                .filter(|operation| operation.recovery_interlock)
                                .map(|operation| {
                                    (operation.target_connection_id, operation.target_database)
                                })
                                .collect();
                            Ok(ActionPreflight {
                                source,
                                target,
                                backup_root: broker.store().backups_root(),
                                recovery_interlocks,
                            })
                        })();
                        let _ = response.send(result);
                    }
                    BridgeRequest::ClientLabel { grant_id, response } => {
                        let label = state
                            .read(cx)
                            .settings
                            .mcp
                            .grants
                            .iter()
                            .find(|grant| grant.id == grant_id && grant.active())
                            .map(|grant| grant.label.clone());
                        let _ = response.send(label);
                    }
                    BridgeRequest::ProposeAction { content, response } => {
                        let result = state
                            .read(cx)
                            .action_broker()
                            .propose(*content)
                            .map_err(|error| error.to_string());
                        let _ = response.send(result);
                        state.update(cx, |_state, cx| {
                            cx.emit(AppEvent::AgentActivityChanged);
                            cx.notify();
                        });
                    }
                    BridgeRequest::GetAction { action_id, grant_id, response } => {
                        let result = state
                            .read(cx)
                            .action_broker()
                            .get_for_grant(action_id, grant_id)
                            .map_err(|error| error.to_string());
                        let _ = response.send(result);
                    }
                    BridgeRequest::ListActions { grant_id, offset, limit, response } => {
                        let result = state
                            .read(cx)
                            .action_broker()
                            .list_for_grant(grant_id, offset, limit)
                            .map_err(|error| error.to_string());
                        let _ = response.send(result);
                    }
                    BridgeRequest::GetOperation { operation_id, grant_id, response } => {
                        let result = state
                            .read(cx)
                            .action_broker()
                            .get_operation_for_grant(operation_id, grant_id)
                            .map_err(|error| error.to_string());
                        let _ = response.send(result);
                    }
                    BridgeRequest::GetBackupManifest { backup_id, response } => {
                        let result = state
                            .read(cx)
                            .action_broker()
                            .store()
                            .load_backup_manifest(backup_id)
                            .map_err(|error| error.to_string());
                        let _ = response.send(result);
                    }
                    BridgeRequest::CancelOperation { operation_id, grant_id, response } => {
                        let result = state
                            .read(cx)
                            .action_broker()
                            .cancel_operation_for_grant(operation_id, grant_id)
                            .map_err(|error| error.to_string());
                        let _ = response.send(result);
                        state.update(cx, |_state, cx| {
                            cx.emit(AppEvent::AgentActivityChanged);
                            cx.notify();
                        });
                    }
                });
                if result.is_err() {
                    break;
                }
            }
        })
        .detach();
        Self { requests }
    }

    pub async fn list_connections(&self) -> Result<Vec<McpConnection>, String> {
        let (response, receiver) = oneshot::channel();
        self.requests
            .send(BridgeRequest::ListConnections(response))
            .await
            .map_err(|_| "OpenMango is shutting down".to_string())?;
        receiver.await.map_err(|_| "OpenMango is shutting down".to_string())
    }

    pub async fn list_databases(&self, connection_id: Uuid) -> Result<Vec<String>, String> {
        let (response, receiver) = oneshot::channel();
        self.requests
            .send(BridgeRequest::ListDatabases { connection_id, response })
            .await
            .map_err(|_| "OpenMango is shutting down".to_string())?;
        receiver.await.map_err(|_| "OpenMango is shutting down".to_string())?
    }

    pub async fn resolve_read(&self, connection_id: Uuid) -> Result<Client, String> {
        let (response, receiver) = oneshot::channel();
        self.requests
            .send(BridgeRequest::ResolveRead { connection_id, response })
            .await
            .map_err(|_| "OpenMango is shutting down".to_string())?;
        receiver.await.map_err(|_| "OpenMango is shutting down".to_string())?
    }

    pub async fn resolve_action(
        &self,
        source_connection_id: Option<Uuid>,
        target_connection_id: Uuid,
        target_writable: bool,
    ) -> Result<ActionPreflight, String> {
        let (response, receiver) = oneshot::channel();
        self.requests
            .send(BridgeRequest::ResolveAction {
                source_connection_id,
                target_connection_id,
                target_writable,
                response,
            })
            .await
            .map_err(|_| "OpenMango is shutting down".to_string())?;
        receiver.await.map_err(|_| "OpenMango is shutting down".to_string())?
    }

    pub async fn client_label(&self, grant_id: Uuid) -> Result<Option<String>, String> {
        let (response, receiver) = oneshot::channel();
        self.requests
            .send(BridgeRequest::ClientLabel { grant_id, response })
            .await
            .map_err(|_| "OpenMango is shutting down".to_string())?;
        receiver.await.map_err(|_| "OpenMango is shutting down".to_string())
    }

    pub async fn propose_action(
        &self,
        content: ProposedActionContent,
    ) -> Result<ProposedAction, String> {
        let (response, receiver) = oneshot::channel();
        self.requests
            .send(BridgeRequest::ProposeAction { content: Box::new(content), response })
            .await
            .map_err(|_| "OpenMango is shutting down".to_string())?;
        receiver.await.map_err(|_| "OpenMango is shutting down".to_string())?
    }

    pub async fn get_action(
        &self,
        action_id: Uuid,
        grant_id: Uuid,
    ) -> Result<ProposedAction, String> {
        let (response, receiver) = oneshot::channel();
        self.requests
            .send(BridgeRequest::GetAction { action_id, grant_id, response })
            .await
            .map_err(|_| "OpenMango is shutting down".to_string())?;
        receiver.await.map_err(|_| "OpenMango is shutting down".to_string())?
    }

    pub async fn list_actions(
        &self,
        grant_id: Uuid,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ProposedAction>, String> {
        let (response, receiver) = oneshot::channel();
        self.requests
            .send(BridgeRequest::ListActions { grant_id, offset, limit, response })
            .await
            .map_err(|_| "OpenMango is shutting down".to_string())?;
        receiver.await.map_err(|_| "OpenMango is shutting down".to_string())?
    }

    pub async fn get_operation(
        &self,
        operation_id: Uuid,
        grant_id: Uuid,
    ) -> Result<OperationRecord, String> {
        let (response, receiver) = oneshot::channel();
        self.requests
            .send(BridgeRequest::GetOperation { operation_id, grant_id, response })
            .await
            .map_err(|_| "OpenMango is shutting down".to_string())?;
        receiver.await.map_err(|_| "OpenMango is shutting down".to_string())?
    }

    pub async fn get_backup_manifest(&self, backup_id: Uuid) -> Result<BackupManifest, String> {
        let (response, receiver) = oneshot::channel();
        self.requests
            .send(BridgeRequest::GetBackupManifest { backup_id, response })
            .await
            .map_err(|_| "OpenMango is shutting down".to_string())?;
        receiver.await.map_err(|_| "OpenMango is shutting down".to_string())?
    }

    pub async fn cancel_operation(
        &self,
        operation_id: Uuid,
        grant_id: Uuid,
    ) -> Result<OperationRecord, String> {
        let (response, receiver) = oneshot::channel();
        self.requests
            .send(BridgeRequest::CancelOperation { operation_id, grant_id, response })
            .await
            .map_err(|_| "OpenMango is shutting down".to_string())?;
        receiver.await.map_err(|_| "OpenMango is shutting down".to_string())?
    }

    #[cfg(test)]
    pub fn fixed(connections: Vec<McpConnection>) -> Self {
        let (requests, mut receiver) = mpsc::channel(16);
        tokio::spawn(async move {
            while let Some(request) = receiver.recv().await {
                match request {
                    BridgeRequest::ListConnections(response) => {
                        let _ = response.send(connections.clone());
                    }
                    BridgeRequest::ListDatabases { connection_id, response } => {
                        let result = connections
                            .iter()
                            .find(|connection| connection.id == connection_id)
                            .ok_or_else(|| "Connection is not shared with agents".to_string())
                            .and_then(|connection| {
                                connection
                                    .connected
                                    .then(|| connection.databases.clone())
                                    .ok_or_else(|| "Connection is not connected".to_string())
                            });
                        let _ = response.send(result);
                    }
                    BridgeRequest::ResolveRead { response, .. } => {
                        drop(response);
                    }
                    BridgeRequest::ResolveAction { response, .. } => {
                        let _ = response
                            .send(Err("Action preflight is unavailable in this test".into()));
                    }
                    BridgeRequest::ClientLabel { response, .. } => {
                        let _ = response.send(None);
                    }
                    BridgeRequest::ProposeAction { response, .. }
                    | BridgeRequest::GetAction { response, .. } => {
                        let _ =
                            response.send(Err("Action storage is unavailable in this test".into()));
                    }
                    BridgeRequest::ListActions { response, .. } => {
                        let _ = response.send(Ok(Vec::new()));
                    }
                    BridgeRequest::GetOperation { response, .. }
                    | BridgeRequest::CancelOperation { response, .. } => {
                        let _ = response
                            .send(Err("Operation storage is unavailable in this test".into()));
                    }
                    BridgeRequest::GetBackupManifest { response, .. } => {
                        let _ =
                            response.send(Err("Backup storage is unavailable in this test".into()));
                    }
                }
            }
        });
        Self { requests }
    }
}

fn shared_connections(state: &AppState) -> Vec<McpConnection> {
    PolicyEvaluator::new(state).visible_connections()
}

fn shared_client(state: &AppState, connection_id: Uuid) -> Result<Client, String> {
    PolicyEvaluator::new(state).authorize_read(connection_id)
}

fn shared_databases(state: &AppState, connection_id: Uuid) -> Result<Vec<String>, String> {
    PolicyEvaluator::new(state).cached_databases(connection_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConnectionEnvironment, SavedConnection};

    #[test]
    fn only_explicitly_shared_connections_are_visible() {
        let mut state = AppState::new();
        state.connections.clear();
        let hidden = SavedConnection::new("Hidden".into(), "mongodb://hidden".into());
        let mut production = SavedConnection::new("Production".into(), "mongodb://prod".into());
        production.agent_shared = true;
        production.environment = Some(ConnectionEnvironment::Production);
        state.connections.extend([hidden, production]);

        let visible = shared_connections(&state);

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "Production");
        assert!(visible[0].protected);
        assert!(!visible[0].connected);
        assert!(visible[0].databases.is_empty());
    }
}
