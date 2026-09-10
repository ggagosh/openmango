use gpui_kit::{App, AppContext as _, Entity};
use mongodb::bson::Document;

use crate::state::{AppCommands, AppEvent, AppState, EditorSessionId, SessionKey};

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
        let manager = state.read(cx).connection_manager();
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();
        let task = cx.background_spawn(async move {
            manager.insert_document(&client, &database, &collection, document)
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui_kit::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            let event =
                                AppEvent::DocumentInserted { session: session_key.clone(), editor };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                        AppCommands::load_documents_for_session(state, session_key, cx);
                    }
                    Err(error) => {
                        log::error!("Failed to insert document");
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentInsertFailed {
                                session: session_key.clone(),
                                editor,
                                error: error.to_string(),
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
}
