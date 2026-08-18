use gpui::{App, AppContext as _, Entity};
use uuid::Uuid;

use crate::operations::{OperationContext, OperationId, OperationQuery};
use crate::state::{AppCommands, AppState, SessionKey, StatusMessage};

impl AppCommands {
    pub(crate) fn collection_history_changed(
        state: Entity<AppState>,
        session_key: SessionKey,
        cx: &mut App,
    ) {
        if state.read(cx).session_subview(&session_key)
            == Some(crate::state::CollectionSubview::History)
        {
            Self::load_collection_history(state, session_key, cx);
            return;
        }
        state.update(cx, |state, cx| {
            if let Some(session) = state.session_mut(&session_key) {
                session.data.history_request_id = session.data.history_request_id.wrapping_add(1);
                session.data.history_loading = false;
                session.data.history_loaded = false;
                session.data.history_error = None;
                cx.notify();
            }
        });
    }

    pub fn load_collection_history(state: Entity<AppState>, session_key: SessionKey, cx: &mut App) {
        Self::load_collection_history_page(state, session_key, false, cx);
    }

    pub fn load_more_collection_history(
        state: Entity<AppState>,
        session_key: SessionKey,
        cx: &mut App,
    ) {
        Self::load_collection_history_page(state, session_key, true, cx);
    }

    fn load_collection_history_page(
        state: Entity<AppState>,
        session_key: SessionKey,
        append: bool,
        cx: &mut App,
    ) {
        let Some(engine) = state.read(cx).operation_engine() else {
            return;
        };
        let Some((request_id, offset)) = state.update(cx, |state, cx| {
            let session = state.session_mut(&session_key)?;
            let offset = if append { session.data.history_next_offset? } else { 0 };
            session.data.history_request_id = session.data.history_request_id.wrapping_add(1);
            session.data.history_loading = true;
            session.data.history_error = None;
            cx.notify();
            Some((session.data.history_request_id, offset))
        }) else {
            return;
        };
        let mut query = OperationQuery::for_collection(
            session_key.connection_id,
            &session_key.database,
            &session_key.collection,
        );
        query.offset = offset;
        let task = cx.background_spawn(async move { engine.list(query) });
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let result = task.await;
            let _ = cx.update(|cx| {
                state.update(cx, |state, cx| {
                    let Some(session) = state.session_mut(&session_key) else {
                        return;
                    };
                    if session.data.history_request_id != request_id {
                        return;
                    }
                    session.data.history_loading = false;
                    match result {
                        Ok(page) => {
                            if append {
                                session.data.history.extend(page.items);
                            } else {
                                session.data.history = page.items;
                            }
                            session.data.history_loaded = true;
                            session.data.history_total = page.total;
                            session.data.history_next_offset = page.next_offset;
                            session.data.history_error = None;
                        }
                        Err(_) => {
                            session.data.history_error =
                                Some("Collection history could not be loaded.".into());
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    pub fn revert_operation(
        state: Entity<AppState>,
        operation_id: OperationId,
        connection_id: Uuid,
        database: String,
        collection: String,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(connection_id), cx) {
            return;
        }
        let Some(client) = Self::active_client(&state, connection_id, cx) else {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "Connect the operation's target before reverting.",
                )));
                cx.notify();
            });
            return;
        };
        let (engine, backend) = {
            let state = state.read(cx);
            (state.operation_engine(), state.operation_backend())
        };
        let Some(engine) = engine else {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "Reversible history is unavailable; no write was made.",
                )));
                cx.notify();
            });
            return;
        };
        backend.register_client(connection_id, client);
        let task = cx.background_spawn(async move {
            let index_change = engine
                .get(operation_id)?
                .ok_or(crate::operations::OperationError::NotFound)?
                .summary
                .kind
                .is_index();
            engine.revert(OperationContext::user(), operation_id)?;
            Ok::<_, crate::operations::OperationError>(index_change)
        });
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let result = task.await;
            let succeeded = result.is_ok();
            let index_change = matches!(&result, Ok(true));
            let _ = cx.update(|cx| {
                state.update(cx, |state, cx| {
                    let message = match result {
                        Ok(_) => StatusMessage::info("Tracked change revert completed."),
                        Err(error) => StatusMessage::error(error.user_message()),
                    };
                    state.set_status_message(Some(message));
                    cx.notify();
                });
                let session_key = SessionKey::new(connection_id, database, collection);
                if succeeded {
                    if index_change {
                        AppCommands::load_collection_indexes(
                            state.clone(),
                            session_key.clone(),
                            true,
                            cx,
                        );
                    } else {
                        AppCommands::load_documents_for_session(
                            state.clone(),
                            session_key.clone(),
                            cx,
                        );
                    }
                }
                AppCommands::load_collection_history(state.clone(), session_key, cx);
            });
        })
        .detach();
    }
}
