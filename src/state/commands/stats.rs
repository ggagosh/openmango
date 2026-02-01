use gpui::{App, AppContext as _, Entity};
use mongodb::bson::Document;

use crate::state::{AppState, CollectionStats, SessionKey, StatusMessage};

use super::AppCommands;

impl AppCommands {
    /// Load collection stats for a session.
    pub fn load_collection_stats(state: Entity<AppState>, session_key: SessionKey, cx: &mut App) {
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();
        let manager = state.read(cx).connection_manager();

        state.update(cx, |state, cx| {
            let session = state.ensure_session(session_key.clone());
            session.data.stats_loading = true;
            session.data.stats_error = None;
            cx.notify();
        });

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move { manager.collection_stats(&client, &database, &collection) }
        });

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<Document, crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(stats_doc) => {
                        let stats = CollectionStats::from_document(&stats_doc);
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                session.data.stats = Some(stats);
                                session.data.stats_loading = false;
                            }
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to load collection stats: {}", e);
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                session.data.stats_loading = false;
                                session.data.stats_error = Some(e.to_string());
                            }
                            state.set_status_message(Some(StatusMessage::error(format!(
                                "Stats failed: {e}"
                            ))));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }
}
