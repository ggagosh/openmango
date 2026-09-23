//! Command helpers for async operations + event emission.

use gpui_kit::{App, Entity};
use mongodb::Client;
use uuid::Uuid;

use crate::state::{AppState, SessionKey, StatusMessage};

pub struct AppCommands;

impl AppCommands {
    pub(super) fn ensure_writable(
        state: &Entity<AppState>,
        connection_id: Option<Uuid>,
        cx: &mut App,
    ) -> bool {
        let Some(connection_id) = connection_id else {
            return true;
        };
        if state.read(cx).connection_read_only(connection_id) {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "Read-only connection: writes are disabled.",
                )));
                cx.notify();
            });
            return false;
        }
        if !state.read(cx).connection_requires_production_write_confirmation(connection_id) {
            return true;
        }
        let authorized = state
            .update(cx, |state, _cx| state.consume_production_write_authorization(connection_id));
        if !authorized {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "Production write blocked: review and confirm this operation first.",
                )));
                cx.notify();
            });
        }
        authorized
    }

    /// `ensure_writable` for a write aimed at one namespace. The view check runs first so a
    /// refused write never spends a production-write authorization.
    pub(super) fn ensure_collection_writable(
        state: &Entity<AppState>,
        session_key: &SessionKey,
        cx: &mut App,
    ) -> bool {
        if let Some(reason) = state.read(cx).view_read_only_reason(session_key) {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(reason)));
                cx.notify();
            });
            return false;
        }
        Self::ensure_writable(state, Some(session_key.connection_id), cx)
    }

    pub(super) fn active_client(
        state: &Entity<AppState>,
        connection_id: Uuid,
        cx: &mut App,
    ) -> Option<Client> {
        state.read(cx).active_connection_client(connection_id)
    }

    pub(super) fn client_for_session(
        state: &Entity<AppState>,
        session_key: &SessionKey,
        cx: &mut App,
    ) -> Option<Client> {
        Self::active_client(state, session_key.connection_id, cx)
    }
}

mod actions;
mod aggregation;
mod collection_meta;
mod collections;
pub(crate) mod compare;
mod compare_sync;
mod connections;
mod databases;
mod documents;
mod explain;
mod indexes;
mod operations;
mod relations;
mod schema;
pub(crate) use aggregation::view_pipeline;
pub use collections::{ViewSave, ViewSource};
pub use documents::save_as::ExportProgress;
pub(crate) use schema::{SCHEMA_SAMPLE_SIZE, build_schema_analysis};
pub use schema::{schema_to_compass, schema_to_json_schema, schema_to_summary};
mod stats;
mod task_run;
mod tasks;
mod transfer;
mod updater;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConnectionEnvironment, SavedConnection};

    #[test]
    fn production_write_authorization_is_explicit_and_one_shot() {
        let mut state = AppState::new();
        state.connections.clear();
        let mut connection =
            SavedConnection::new("prod-looking-host".into(), "mongodb://localhost".into());
        connection.environment = Some(ConnectionEnvironment::Production);
        connection.confirm_production_writes = true;
        let connection_id = connection.id;
        state.connections.push(connection);

        assert!(state.connection_requires_production_write_confirmation(connection_id));
        assert!(!state.consume_production_write_authorization(connection_id));
        state.authorize_next_production_write(connection_id);
        assert!(state.consume_production_write_authorization(connection_id));
        assert!(!state.consume_production_write_authorization(connection_id));

        state.authorize_next_production_write(connection_id);
        state.revoke_production_write_authorizations(connection_id, 1);
        assert!(!state.consume_production_write_authorization(connection_id));
    }
}
