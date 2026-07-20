use gpui::{App, AppContext as _, Entity};
use mongodb::bson::Document;

use crate::state::{AppEvent, AppState, EditorSessionId, SessionKey};

use crate::state::AppCommands;

impl AppCommands {
    /// Insert a document into a collection.
    pub fn insert_document(
        state: Entity<AppState>,
        session_key: SessionKey,
        document: Document,
        cx: &mut App,
    ) {
        Self::insert_document_internal(state, session_key, document, None, cx);
    }

    pub fn insert_document_for_editor(
        state: Entity<AppState>,
        session_key: SessionKey,
        document: Document,
        editor: EditorSessionId,
        cx: &mut App,
    ) {
        Self::insert_document_internal(state, session_key, document, Some(editor), cx);
    }

    fn insert_document_internal(
        state: Entity<AppState>,
        session_key: SessionKey,
        document: Document,
        editor: Option<EditorSessionId>,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            return;
        }
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();
        let manager = state.read(cx).connection_manager();

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            let document = document.clone();
            async move { manager.insert_document(&client, &database, &collection, document) }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            let event =
                                AppEvent::DocumentInserted { session: session_key.clone(), editor };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                        AppCommands::load_documents_for_session(
                            state.clone(),
                            session_key.clone(),
                            cx,
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to insert document: {}", e);
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentInsertFailed {
                                session: session_key.clone(),
                                editor,
                                error: e.to_string(),
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }

    // Bulk insert moved to documents/bulk.rs.
}
