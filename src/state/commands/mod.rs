//! Command helpers for async operations + event emission.

use gpui::{App, Entity};
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
        let read_only =
            connection_id.map(|id| state.read(cx).connection_read_only(id)).unwrap_or(false);
        if read_only {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "Read-only connection: writes are disabled.",
                )));
                cx.notify();
            });
        }
        !read_only
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

mod aggregation;
mod collections;
mod connections;
mod databases;
mod documents;
mod indexes;
mod stats;
mod transfer;
mod updater;
