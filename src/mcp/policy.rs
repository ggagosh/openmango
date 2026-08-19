use mongodb::Client;
use uuid::Uuid;

use crate::actions::hash_serializable;
use crate::actions::model::ConnectionActionSnapshot;
use crate::models::ConnectionEnvironment;
use crate::state::AppState;
use crate::sync::plan::RuntimeActionConnection;

use super::McpConnection;

pub struct PolicyEvaluator<'a> {
    state: &'a AppState,
}

impl<'a> PolicyEvaluator<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    pub fn visible_connections(&self) -> Vec<McpConnection> {
        self.state
            .connections
            .iter()
            .filter(|connection| connection.agent_shared)
            .map(|connection| {
                let active = self.state.active_connection_by_id(connection.id);
                McpConnection {
                    id: connection.id,
                    name: connection.name.clone(),
                    environment: connection
                        .environment
                        .map(|environment| environment.label().into()),
                    protected: connection.protected
                        || connection.environment == Some(ConnectionEnvironment::Production),
                    read_only: connection.read_only,
                    writable: connection.agent_writable && !connection.read_only,
                    connected: active.is_some(),
                    databases: active.map(|active| active.databases.clone()).unwrap_or_default(),
                }
            })
            .collect()
    }

    pub fn authorize_read(&self, connection_id: Uuid) -> Result<Client, String> {
        self.shared_connection(connection_id)?;
        self.state
            .active_connection_client(connection_id)
            .ok_or_else(|| "Connection is not connected".to_string())
    }

    pub fn authorize_direct_write(&self, connection_id: Uuid) -> Result<Client, String> {
        let connection = self.shared_connection(connection_id)?;
        if !connection.agent_writable {
            return Err("Agent writes are not enabled for this connection".to_string());
        }
        if connection.read_only {
            return Err("Target connection is read-only".to_string());
        }
        self.state
            .active_connection_client(connection_id)
            .ok_or_else(|| "Connection is not connected".to_string())
    }

    pub fn cached_databases(&self, connection_id: Uuid) -> Result<Vec<String>, String> {
        let connection = self.shared_connection(connection_id)?;
        self.state
            .active_connection_by_id(connection.id)
            .map(|active| active.databases.clone())
            .ok_or_else(|| "Connection is not connected".to_string())
    }

    pub fn authorize_action_connection(
        &self,
        connection_id: Uuid,
        require_writable: bool,
    ) -> Result<RuntimeActionConnection, String> {
        let connection = self.shared_connection(connection_id)?;
        if require_writable && connection.read_only {
            return Err("Target connection is read-only".to_string());
        }
        let active = self
            .state
            .active_connection_by_id(connection_id)
            .ok_or_else(|| "Connection is not connected".to_string())?;
        let stripped = connection.with_secrets_stripped();
        let identity_hash = hash_serializable(&serde_json::json!({
            "id": stripped.id,
            "name": stripped.name,
            "uri": stripped.uri,
            "environment": stripped.environment,
            "protected": stripped.protected,
            "read_only": stripped.read_only,
            "ssh": stripped.ssh,
            "proxy": stripped.proxy,
            "secret_id": stripped.secret_id,
        }))
        .map_err(|_| "Could not fingerprint connection identity".to_string())?;
        let protected = connection.protected
            || connection.environment == Some(ConnectionEnvironment::Production);
        Ok(RuntimeActionConnection {
            client: active.client.clone(),
            tool_uri: self
                .state
                .active_connection_tool_uri(connection_id)
                .map_err(|_| "Could not resolve active connection transport".to_string())?,
            snapshot: ConnectionActionSnapshot {
                connection_id,
                display_name: connection.name.clone(),
                environment: connection.environment.map(|environment| environment.label().into()),
                protected,
                read_only: connection.read_only,
                agent_shared: connection.agent_shared,
                connected: true,
                identity_hash,
            },
            databases: active.databases.clone(),
        })
    }

    fn shared_connection(
        &self,
        connection_id: Uuid,
    ) -> Result<&crate::models::SavedConnection, String> {
        self.state
            .connection_by_id(connection_id)
            .filter(|connection| connection.agent_shared)
            .ok_or_else(|| "Connection is not shared with agents".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SavedConnection;

    #[test]
    fn unshared_connections_have_no_read_authority() {
        let mut state = AppState::new();
        state.connections.clear();
        let connection = SavedConnection::new("Hidden".into(), "mongodb://localhost".into());
        let id = connection.id;
        state.connections.push(connection);

        let policy = PolicyEvaluator::new(&state);

        assert!(policy.visible_connections().is_empty());
        assert_eq!(policy.authorize_read(id).unwrap_err(), "Connection is not shared with agents");
    }

    #[test]
    fn direct_write_requires_explicit_authority_and_read_only_overrides_it() {
        let mut state = AppState::new();
        state.connections.clear();
        let mut connection = SavedConnection::new("Shared".into(), "mongodb://localhost".into());
        connection.agent_shared = true;
        let id = connection.id;
        state.connections.push(connection);
        assert_eq!(
            PolicyEvaluator::new(&state).authorize_direct_write(id).unwrap_err(),
            "Agent writes are not enabled for this connection"
        );

        state.connections[0].agent_writable = true;
        state.connections[0].read_only = true;
        assert_eq!(
            PolicyEvaluator::new(&state).authorize_direct_write(id).unwrap_err(),
            "Target connection is read-only"
        );
    }
}
