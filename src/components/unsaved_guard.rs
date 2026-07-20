use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use uuid::Uuid;

use crate::bson::parse_document_from_json;
use crate::components::Button;
use crate::error::Error;
use crate::state::{
    AppCommands, AppEvent, AppState, EditorSession, EditorSessionTarget, StatusMessage,
    UnsavedChange, UnsavedInventory, UnsavedScope,
};
use crate::theme::spacing;

#[derive(Default)]
struct UnsavedDialogState {
    focused_once: bool,
}

type Continuation = Box<dyn FnOnce(&mut Window, &mut App)>;

fn has_in_flight_editor_save(inventory: &UnsavedInventory) -> bool {
    inventory.changes.iter().any(|change| {
        matches!(change, UnsavedChange::DetachedEditor(EditorSession { save_in_flight: true, .. }))
    })
}

pub fn request_app_quit(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
    let state_for_quit = state.clone();
    request_unsaved_action(state, UnsavedScope::App, window, cx, move |window, cx| {
        state_for_quit.update(cx, |state, _| {
            state.update_workspace_from_state();
            state.flush_workspace_now();
        });
        let this_window = window.window_handle();
        for handle in cx.windows() {
            if handle != this_window {
                handle.update(cx, |_, window, _cx| window.remove_window()).ok();
            }
        }
        cx.quit();
    });
}

pub fn request_disconnect_connection(
    state: Entity<AppState>,
    connection_id: uuid::Uuid,
    window: &mut Window,
    cx: &mut App,
) {
    let state_for_disconnect = state.clone();
    request_unsaved_action(
        state,
        UnsavedScope::Connection(connection_id),
        window,
        cx,
        move |_window, cx| {
            AppCommands::disconnect(state_for_disconnect, connection_id, cx);
        },
    );
}

pub fn request_preview_collection(
    state: Entity<AppState>,
    connection_id: Uuid,
    database: String,
    collection: String,
    window: &mut Window,
    cx: &mut App,
) {
    let current_preview = state.read(cx).preview_tab().cloned();
    let is_same_preview = current_preview.as_ref().is_some_and(|preview| {
        preview.connection_id == connection_id
            && preview.database == database
            && preview.collection == collection
    });
    let state_for_preview = state.clone();
    let proceed = move |_window: &mut Window, cx: &mut App| {
        state_for_preview.update(cx, |state, cx| {
            state.select_connection(Some(connection_id), cx);
            state.preview_collection(database, collection, cx);
        });
    };
    if is_same_preview {
        proceed(window, cx);
    } else if let Some(preview) = current_preview {
        request_unsaved_action(state, UnsavedScope::Preview(preview), window, cx, proceed);
    } else {
        proceed(window, cx);
    }
}

pub fn request_remove_connection(
    state: Entity<AppState>,
    connection_id: uuid::Uuid,
    window: &mut Window,
    cx: &mut App,
) {
    let state_for_remove = state.clone();
    request_unsaved_action(
        state,
        UnsavedScope::Connection(connection_id),
        window,
        cx,
        move |_window, cx| {
            state_for_remove.update(cx, |state, cx| {
                state.remove_connection(connection_id, cx);
            });
        },
    );
}

pub fn request_unsaved_action(
    state: Entity<AppState>,
    scope: UnsavedScope,
    window: &mut Window,
    cx: &mut App,
    proceed: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    let inventory = state.read(cx).unsaved_inventory(&scope);
    if inventory.is_empty() {
        proceed(window, cx);
        return;
    }
    if has_in_flight_editor_save(&inventory) {
        state.update(cx, |state, cx| {
            state.set_status_message(Some(StatusMessage::info(
                "Wait for the current editor save to finish before continuing.",
            )));
            cx.notify();
        });
        return;
    }
    let opened = state.update(cx, |state, _| state.begin_unsaved_guard());
    if !opened {
        return;
    }
    open_unsaved_dialog(state, scope, inventory.len(), window, cx, Box::new(proceed));
}

fn open_unsaved_dialog(
    state: Entity<AppState>,
    scope: UnsavedScope,
    count: usize,
    window: &mut Window,
    cx: &mut App,
    proceed: Continuation,
) {
    let proceed = Rc::new(RefCell::new(Some(proceed)));
    let cancel_focus = cx.focus_handle().tab_index(0).tab_stop(true);
    let save_focus = cx.focus_handle().tab_index(1).tab_stop(true);
    let discard_focus = cx.focus_handle().tab_index(2).tab_stop(true);
    let message = format!(
        "{count} unsaved change{} will be lost. Save changes, discard them, or cancel.",
        if count == 1 { "" } else { "s" }
    );

    window.open_dialog(cx, move |dialog: Dialog, window, cx| {
        let dialog_state = window.use_keyed_state("unsaved-dialog-focus", cx, |_window, _cx| {
            UnsavedDialogState::default()
        });
        if !dialog_state.read(cx).focused_once {
            dialog_state.update(cx, |state, _| state.focused_once = true);
            let focus = cancel_focus.clone();
            window.defer(cx, move |window, _| window.focus(&focus));
        }

        let cancel_state = state.clone();
        let key_cancel_state = state.clone();
        let key_handler = move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            if event.keystroke.key.eq_ignore_ascii_case("escape") {
                cx.stop_propagation();
                key_cancel_state.update(cx, |state, _| state.end_unsaved_guard());
                window.close_dialog(cx);
            }
        };

        dialog.title("Unsaved changes").min_w(px(480.0)).keyboard(false).child(
            div()
                .flex()
                .flex_col()
                .gap(spacing::md())
                .p(spacing::md())
                .on_key_down(key_handler)
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().secondary_foreground)
                        .child(message.clone()),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(spacing::xs())
                        .child(
                            Button::new("unsaved-cancel")
                                .label("Cancel")
                                .track_focus(&cancel_focus)
                                .tab_index(0)
                                .on_click(move |_, window, cx| {
                                    cancel_state.update(cx, |state, _| state.end_unsaved_guard());
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("unsaved-save")
                                .primary()
                                .label("Save")
                                .track_focus(&save_focus)
                                .tab_index(1)
                                .on_click({
                                    let state = state.clone();
                                    let scope = scope.clone();
                                    let proceed = proceed.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        let Some(proceed) = proceed.borrow_mut().take() else {
                                            return;
                                        };
                                        save_unsaved_changes(
                                            state.clone(),
                                            scope.clone(),
                                            window.window_handle(),
                                            proceed,
                                            cx,
                                        );
                                    }
                                }),
                        )
                        .child(
                            Button::new("unsaved-discard")
                                .danger()
                                .label("Discard")
                                .track_focus(&discard_focus)
                                .tab_index(2)
                                .on_click({
                                    let state = state.clone();
                                    let scope = scope.clone();
                                    let proceed = proceed.clone();
                                    move |_, window, cx| {
                                        let inventory = state.read(cx).unsaved_inventory(&scope);
                                        if has_in_flight_editor_save(&inventory) {
                                            state.update(cx, |state, cx| {
                                                state.end_unsaved_guard();
                                                state.set_status_message(Some(StatusMessage::info(
                                                    "Wait for the current editor save to finish before continuing.",
                                                )));
                                                cx.notify();
                                            });
                                            window.close_dialog(cx);
                                            return;
                                        }
                                        let Some(proceed) = proceed.borrow_mut().take() else {
                                            return;
                                        };
                                        state.update(cx, |state, cx| {
                                            state.discard_unsaved(&scope, cx);
                                            state.end_unsaved_guard();
                                        });
                                        window.close_dialog(cx);
                                        proceed(window, cx);
                                    }
                                }),
                        ),
                ),
        )
    });
}

#[derive(Clone)]
enum PreparedSave {
    Inline {
        change: UnsavedChange,
        client: mongodb::Client,
    },
    DetachedDocument {
        change: UnsavedChange,
        client: mongodb::Client,
        document: mongodb::bson::Document,
    },
    DetachedInsert {
        change: UnsavedChange,
        client: mongodb::Client,
        document: mongodb::bson::Document,
    },
}

fn save_unsaved_changes(
    state: Entity<AppState>,
    scope: UnsavedScope,
    window_handle: AnyWindowHandle,
    proceed: Continuation,
    cx: &mut App,
) {
    let inventory = state.read(cx).unsaved_inventory(&scope);
    if has_in_flight_editor_save(&inventory) {
        state.update(cx, |state, cx| {
            state.end_unsaved_guard();
            state.set_status_message(Some(StatusMessage::info(
                "Wait for the current editor save to finish before continuing.",
            )));
            cx.notify();
        });
        return;
    }
    let prepared = match prepare_saves(&state, inventory, cx) {
        Ok(prepared) => prepared,
        Err(error) => {
            state.update(cx, |state, cx| {
                state.end_unsaved_guard();
                state.set_status_message(Some(StatusMessage::error(error.to_string())));
                cx.notify();
            });
            return;
        }
    };
    let manager = state.read(cx).connection_manager();
    let task = cx.background_spawn(async move {
        prepared
            .into_iter()
            .map(|save| {
                let result = execute_save(&manager, &save);
                (save, result)
            })
            .collect::<Vec<_>>()
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let results = task.await;
        let _ = cx.update_window(window_handle, |_root, window, cx| {
            let mut errors = Vec::new();
            let mut reload_sessions = HashSet::new();
            state.update(cx, |state, cx| {
                for (save, result) in results {
                    match result {
                        Ok(()) => {
                            if !apply_saved_change(state, save, cx) {
                                errors
                                    .push("A draft changed while it was being saved.".to_string());
                            }
                        }
                        Err(error) => errors.push(error.to_string()),
                    }
                }
                let remaining = state.unsaved_inventory(&scope);
                if !remaining.is_empty() {
                    errors.push(
                        "Unsaved changes remain after saving; review them and try again."
                            .to_string(),
                    );
                }
                for change in remaining.changes {
                    let session_key = match change {
                        UnsavedChange::InlineDocument { session_key, .. }
                        | UnsavedChange::InvalidInlineEdit { session_key }
                        | UnsavedChange::DetachedEditor(EditorSession { session_key, .. }) => {
                            session_key
                        }
                    };
                    reload_sessions.insert(session_key);
                }
                state.end_unsaved_guard();
                if !errors.is_empty() {
                    state.set_status_message(Some(StatusMessage::error(format!(
                        "Could not save all changes: {}",
                        errors.join("; ")
                    ))));
                }
                cx.notify();
            });

            if errors.is_empty() {
                proceed(window, cx);
            } else {
                for session_key in reload_sessions {
                    AppCommands::load_documents_for_session(state.clone(), session_key, cx);
                }
            }
        });
    })
    .detach();
}

fn prepare_saves(
    state: &Entity<AppState>,
    inventory: UnsavedInventory,
    cx: &App,
) -> Result<Vec<PreparedSave>, Error> {
    let mut prepared = Vec::new();
    let mut document_targets = HashSet::new();
    for change in inventory.changes {
        let session_key = match &change {
            UnsavedChange::InlineDocument { session_key, .. }
            | UnsavedChange::InvalidInlineEdit { session_key } => session_key,
            UnsavedChange::DetachedEditor(session) => &session.session_key,
        };
        if state.read(cx).connection_read_only(session_key.connection_id) {
            return Err(Error::Parse(format!(
                "{} is read-only; discard or cancel instead.",
                session_key.collection
            )));
        }
        let Some(client) = state.read(cx).active_connection_client(session_key.connection_id)
        else {
            return Err(Error::Parse("Connection is not active.".to_string()));
        };

        match &change {
            UnsavedChange::InvalidInlineEdit { .. } => {
                return Err(Error::Parse(
                    "An inline edit contains an invalid value. Correct it or choose Discard."
                        .to_string(),
                ));
            }
            UnsavedChange::InlineDocument { session_key, doc_key, .. } => {
                if !document_targets.insert((session_key.clone(), doc_key.clone())) {
                    return Err(Error::Parse(
                        "The same document has multiple conflicting drafts.".to_string(),
                    ));
                }
                prepared.push(PreparedSave::Inline { change, client });
            }
            UnsavedChange::DetachedEditor(session) => {
                let document = parse_document_from_json(&session.content)
                    .map_err(|error| Error::Parse(format!("Invalid JSON: {error}")))?;
                match &session.target {
                    EditorSessionTarget::Document { doc_key, original_id, .. } => {
                        if !document_targets.insert((session.session_key.clone(), doc_key.clone()))
                        {
                            return Err(Error::Parse(
                                "The same document has inline and detached drafts.".to_string(),
                            ));
                        }
                        if document.get("_id") != Some(original_id.as_ref()) {
                            return Err(Error::Parse(
                                "Edited document must keep the original _id.".to_string(),
                            ));
                        }
                        prepared.push(PreparedSave::DetachedDocument { change, client, document });
                    }
                    EditorSessionTarget::Insert => {
                        prepared.push(PreparedSave::DetachedInsert { change, client, document })
                    }
                }
            }
        }
    }
    Ok(prepared)
}

fn execute_save(
    manager: &crate::connection::ConnectionManager,
    save: &PreparedSave,
) -> Result<(), Error> {
    match save {
        PreparedSave::Inline { change, client } => {
            let UnsavedChange::InlineDocument {
                session_key,
                original_id,
                baseline_document,
                document,
                ..
            } = change
            else {
                unreachable!();
            };
            let original_id = original_id.as_ref().ok_or_else(|| {
                Error::Parse("Could not resolve the edited document's _id.".to_string())
            })?;
            let baseline_document = baseline_document.as_ref().ok_or_else(|| {
                Error::Parse("Could not resolve the edited document's baseline.".to_string())
            })?;
            manager.replace_document_if_current(
                client,
                &session_key.database,
                &session_key.collection,
                original_id,
                baseline_document,
                document.clone(),
            )
        }
        PreparedSave::DetachedDocument { change, client, document } => {
            let UnsavedChange::DetachedEditor(session) = change else {
                unreachable!();
            };
            let EditorSessionTarget::Document { original_id, baseline_document, .. } =
                &session.target
            else {
                unreachable!();
            };
            manager.replace_document_if_current(
                client,
                &session.session_key.database,
                &session.session_key.collection,
                original_id,
                baseline_document,
                document.clone(),
            )
        }
        PreparedSave::DetachedInsert { change, client, document } => {
            let UnsavedChange::DetachedEditor(session) = change else {
                unreachable!();
            };
            manager.insert_document(
                client,
                &session.session_key.database,
                &session.session_key.collection,
                document.clone(),
            )
        }
    }
}

fn apply_saved_change(
    state: &mut AppState,
    save: PreparedSave,
    cx: &mut Context<AppState>,
) -> bool {
    match save {
        PreparedSave::Inline { change, .. } => {
            let UnsavedChange::InlineDocument { session_key, doc_key, document, .. } = change
            else {
                unreachable!();
            };
            let unchanged = state.session_draft(&session_key, &doc_key).as_ref() == Some(&document);
            if let Some(session) = state.session_mut(&session_key) {
                if let Some(index) = session.data.index_by_key.get(&doc_key).copied()
                    && let Some(item) = session.data.items.get_mut(index)
                {
                    item.doc = document;
                }
                if unchanged {
                    session.view.drafts.remove(&doc_key);
                    session.view.dirty.remove(&doc_key);
                }
            }
            let dirty = state.session_view(&session_key).is_some_and(|view| !view.dirty.is_empty());
            state.set_collection_dirty(session_key.clone(), dirty, cx);
            cx.emit(AppEvent::DocumentSaved {
                session: session_key,
                document: doc_key,
                editor: None,
            });
            unchanged
        }
        PreparedSave::DetachedDocument { change, document, .. } => {
            let UnsavedChange::DetachedEditor(session) = change else {
                unreachable!();
            };
            let current = state.editor_sessions().snapshot(session.id);
            let unchanged =
                current.as_ref().is_some_and(|current| current.content == session.content);
            if unchanged {
                let handle = state.editor_sessions().window_handle(session.id);
                state.editor_sessions().close(session.id);
                if let Some(handle) = handle {
                    let _ = handle.update(cx, |_root, window, _cx| window.remove_window());
                }
            } else {
                state.editor_sessions().refresh_document_baseline(session.id, document);
            }
            unchanged
        }
        PreparedSave::DetachedInsert { change, .. } => {
            let UnsavedChange::DetachedEditor(session) = change else {
                unreachable!();
            };
            let current = state.editor_sessions().snapshot(session.id);
            let unchanged =
                current.as_ref().is_some_and(|current| current.content == session.content);
            if unchanged {
                let handle = state.editor_sessions().window_handle(session.id);
                state.editor_sessions().close(session.id);
                if let Some(handle) = handle {
                    let _ = handle.update(cx, |_root, window, _cx| window.remove_window());
                }
            }
            unchanged
        }
    }
}
