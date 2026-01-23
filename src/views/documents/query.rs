use gpui::*;
use gpui_component::input::InputState;
use mongodb::bson::Document;

use crate::bson::parse_document_from_json;
use crate::state::{AppCommands, AppState, SessionKey, StatusMessage};

use super::CollectionView;

impl CollectionView {
    pub(super) fn apply_filter(
        state: Entity<AppState>,
        session_key: SessionKey,
        filter_state: Entity<InputState>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let raw = filter_state.read(cx).value().to_string();
        let trimmed = raw.trim();

        if trimmed.is_empty() || trimmed == "{}" {
            state.update(cx, |state, cx| {
                state.clear_filter(&session_key);
                state.status_message = Some(StatusMessage::info("Filter cleared"));
                cx.notify();
            });
            AppCommands::load_documents_for_session(state.clone(), session_key, cx);
            return;
        }

        match parse_document_from_json(trimmed) {
            Ok(filter) => {
                state.update(cx, |state, cx| {
                    state.set_filter(&session_key, trimmed.to_string(), Some(filter));
                    state.status_message = Some(StatusMessage::info("Filter applied"));
                    cx.notify();
                });
                AppCommands::load_documents_for_session(state.clone(), session_key, cx);
            }
            Err(err) => {
                state.update(cx, |state, cx| {
                    state.status_message =
                        Some(StatusMessage::error(format!("Invalid filter JSON: {err}")));
                    cx.notify();
                });
            }
        }
    }

    pub(super) fn apply_query_options(
        state: Entity<AppState>,
        session_key: SessionKey,
        sort_state: Entity<InputState>,
        projection_state: Entity<InputState>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let sort_raw = sort_state.read(cx).value().to_string();
        let projection_raw = projection_state.read(cx).value().to_string();

        let (sort_raw_store, sort_doc) = match parse_optional_doc(&sort_raw) {
            Ok(result) => result,
            Err(err) => {
                state.update(cx, |state, cx| {
                    state.status_message =
                        Some(StatusMessage::error(format!("Invalid sort JSON: {err}")));
                    cx.notify();
                });
                return;
            }
        };

        let (projection_raw_store, projection_doc) = match parse_optional_doc(&projection_raw) {
            Ok(result) => result,
            Err(err) => {
                state.update(cx, |state, cx| {
                    state.status_message =
                        Some(StatusMessage::error(format!("Invalid projection JSON: {err}")));
                    cx.notify();
                });
                return;
            }
        };

        let message = if sort_doc.is_none() && projection_doc.is_none() {
            "Sort/projection cleared"
        } else {
            "Sort/projection applied"
        };

        state.update(cx, |state, cx| {
            state.set_sort_projection(
                &session_key,
                sort_raw_store,
                sort_doc,
                projection_raw_store,
                projection_doc,
            );
            state.status_message = Some(StatusMessage::info(message));
            cx.notify();
        });
        AppCommands::load_documents_for_session(state.clone(), session_key, cx);
    }
}

fn parse_optional_doc(raw: &str) -> Result<(String, Option<Document>), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return Ok((String::new(), None));
    }

    match parse_document_from_json(trimmed) {
        Ok(doc) => Ok((trimmed.to_string(), Some(doc))),
        Err(err) => Err(err.to_string()),
    }
}
