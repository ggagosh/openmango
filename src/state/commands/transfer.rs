//! Transfer commands for import, export, and copy operations.

use std::path::PathBuf;

use gpui::{App, AppContext as _, Entity};
use uuid::Uuid;

use crate::connection::csv_utils::detect_problematic_fields;
use crate::connection::get_connection_manager;
use crate::connection::mongo::{
    CsvImportOptions, ExtendedJsonMode, InsertMode, JsonExportOptions, JsonImportOptions,
    JsonTransferFormat, generate_export_preview,
};
use crate::state::{
    AppCommands, AppEvent, AppState, SessionKey, StatusMessage, TransferFormat, TransferMode,
    TransferScope,
};

impl AppCommands {
    /// Load preview documents for a transfer tab.
    pub fn load_transfer_preview(state: Entity<AppState>, transfer_id: Uuid, cx: &mut App) {
        let (connection_id, database, collection, json_mode, pretty_print) = {
            let state_ref = state.read(cx);
            let Some(tab) = state_ref.transfer_tab(transfer_id) else {
                return;
            };

            // Only load preview for export mode with a valid source
            if !matches!(tab.mode, TransferMode::Export) {
                return;
            }

            let Some(conn_id) = tab.source_connection_id else {
                return;
            };

            if tab.source_database.is_empty() || tab.source_collection.is_empty() {
                return;
            }

            let json_mode = match tab.json_mode {
                crate::state::ExtendedJsonMode::Relaxed => ExtendedJsonMode::Relaxed,
                crate::state::ExtendedJsonMode::Canonical => ExtendedJsonMode::Canonical,
            };

            (
                conn_id,
                tab.source_database.clone(),
                tab.source_collection.clone(),
                json_mode,
                tab.pretty_print,
            )
        };

        let Some(client) = Self::active_client(&state, connection_id, cx) else {
            return;
        };

        state.update(cx, |state, cx| {
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                tab.preview_loading = true;
                tab.preview_docs.clear();
                tab.warnings.clear();
            }
            cx.notify();
        });

        let task = cx.background_spawn(async move {
            let manager = get_connection_manager();

            // Generate preview docs
            let preview = generate_export_preview(
                manager,
                &client,
                &database,
                &collection,
                json_mode,
                pretty_print,
                5,
            )?;

            // Sample docs to detect problematic fields
            let sample_docs = manager.sample_documents(&client, &database, &collection, 100)?;
            let warnings = detect_problematic_fields(&sample_docs);

            Ok::<_, crate::error::Error>((preview, warnings))
        });

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result = task.await;
                let _ = cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                            tab.preview_loading = false;
                            match result {
                                Ok((preview, warnings)) => {
                                    tab.preview_docs = preview;
                                    tab.warnings = warnings;
                                }
                                Err(e) => {
                                    tab.error_message = Some(e.to_string());
                                }
                            }
                        }
                        cx.emit(AppEvent::TransferPreviewLoaded { transfer_id });
                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }

    /// Execute the transfer operation for a transfer tab.
    pub fn execute_transfer(state: Entity<AppState>, transfer_id: Uuid, cx: &mut App) {
        let transfer_config = {
            let state_ref = state.read(cx);
            state_ref.transfer_tab(transfer_id).cloned()
        };

        let Some(config) = transfer_config else {
            return;
        };

        match config.mode {
            TransferMode::Export => Self::execute_export(state, transfer_id, config, cx),
            TransferMode::Import => Self::execute_import(state, transfer_id, config, cx),
            TransferMode::Copy => Self::execute_copy(state, transfer_id, config, cx),
        }
    }

    fn execute_export(
        state: Entity<AppState>,
        transfer_id: Uuid,
        config: crate::state::TransferTabState,
        cx: &mut App,
    ) {
        let Some(connection_id) = config.source_connection_id else {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "No source connection selected",
                )));
                cx.notify();
            });
            return;
        };

        let Some(client) = Self::active_client(&state, connection_id, cx) else {
            return;
        };

        if config.file_path.is_empty() {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error("No file path specified")));
                cx.notify();
            });
            return;
        }

        let path = PathBuf::from(&config.file_path);
        let database = config.source_database.clone();
        let collection = config.source_collection.clone();
        let format = config.format;
        let json_mode = match config.json_mode {
            crate::state::ExtendedJsonMode::Relaxed => ExtendedJsonMode::Relaxed,
            crate::state::ExtendedJsonMode::Canonical => ExtendedJsonMode::Canonical,
        };
        let pretty_print = config.pretty_print;

        state.update(cx, |state, cx| {
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                tab.is_running = true;
                tab.progress_count = 0;
                tab.error_message = None;
            }
            state.set_status_message(Some(StatusMessage::info("Exporting...")));
            cx.emit(AppEvent::TransferStarted { transfer_id });
            cx.notify();
        });

        let task = cx.background_spawn(async move {
            let manager = get_connection_manager();

            match format {
                TransferFormat::JsonLines | TransferFormat::JsonArray => {
                    let json_format = match format {
                        TransferFormat::JsonLines => JsonTransferFormat::JsonLines,
                        _ => JsonTransferFormat::JsonArray,
                    };
                    manager.export_collection_json_with_options(
                        &client,
                        &database,
                        &collection,
                        &path,
                        JsonExportOptions { format: json_format, json_mode, pretty_print },
                    )
                }
                TransferFormat::Csv => {
                    manager.export_collection_csv(&client, &database, &collection, &path)
                }
                TransferFormat::Bson => {
                    // BSON export requires mongodump - not implemented for collection scope
                    Err(crate::error::Error::Parse(
                        "BSON export is only supported for database scope".to_string(),
                    ))
                }
            }
        });

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result = task.await;
                let _ = cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                            tab.is_running = false;
                        }

                        match result {
                            Ok(count) => {
                                if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                    tab.progress_count = count;
                                }
                                let message = format!(
                                    "Exported {} document{}",
                                    count,
                                    if count == 1 { "" } else { "s" }
                                );
                                state.set_status_message(Some(StatusMessage::info(message)));
                                cx.emit(AppEvent::TransferCompleted { transfer_id, count });
                            }
                            Err(err) => {
                                if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                    tab.error_message = Some(err.to_string());
                                }
                                state.set_status_message(Some(StatusMessage::error(format!(
                                    "Export failed: {err}"
                                ))));
                                cx.emit(AppEvent::TransferFailed {
                                    transfer_id,
                                    error: err.to_string(),
                                });
                            }
                        }
                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }

    fn execute_import(
        state: Entity<AppState>,
        transfer_id: Uuid,
        config: crate::state::TransferTabState,
        cx: &mut App,
    ) {
        let Some(connection_id) = config.source_connection_id else {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error("No connection selected")));
                cx.notify();
            });
            return;
        };

        if !Self::ensure_writable(&state, Some(connection_id), cx) {
            return;
        }

        let Some(client) = Self::active_client(&state, connection_id, cx) else {
            return;
        };

        if config.file_path.is_empty() {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error("No file path specified")));
                cx.notify();
            });
            return;
        }

        let path = PathBuf::from(&config.file_path);
        let database = if config.destination_database.is_empty() {
            config.source_database.clone()
        } else {
            config.destination_database.clone()
        };
        let collection = if config.destination_collection.is_empty() {
            config.source_collection.clone()
        } else {
            config.destination_collection.clone()
        };
        let format = config.format;
        let insert_mode = match config.insert_mode {
            crate::state::InsertMode::Insert => InsertMode::Insert,
            crate::state::InsertMode::Upsert => InsertMode::Upsert,
            crate::state::InsertMode::Replace => InsertMode::Replace,
        };
        let stop_on_error = config.stop_on_error;
        let batch_size = config.batch_size as usize;

        state.update(cx, |state, cx| {
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                tab.is_running = true;
                tab.progress_count = 0;
                tab.error_message = None;
            }
            state.set_status_message(Some(StatusMessage::info("Importing...")));
            cx.emit(AppEvent::TransferStarted { transfer_id });
            cx.notify();
        });

        let task = cx.background_spawn(async move {
            let manager = get_connection_manager();

            match format {
                TransferFormat::JsonLines | TransferFormat::JsonArray => {
                    let json_format = match format {
                        TransferFormat::JsonLines => JsonTransferFormat::JsonLines,
                        _ => JsonTransferFormat::JsonArray,
                    };
                    manager.import_collection_json_with_options(
                        &client,
                        &database,
                        &collection,
                        &path,
                        JsonImportOptions {
                            format: json_format,
                            insert_mode,
                            stop_on_error,
                            batch_size,
                        },
                    )
                }
                TransferFormat::Csv => manager.import_collection_csv(
                    &client,
                    &database,
                    &collection,
                    &path,
                    CsvImportOptions { insert_mode, stop_on_error, batch_size },
                ),
                TransferFormat::Bson => Err(crate::error::Error::Parse(
                    "BSON import is only supported for database scope".to_string(),
                )),
            }
        });

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result = task.await;
                let _ = cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                            tab.is_running = false;
                        }

                        match result {
                            Ok(count) => {
                                if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                    tab.progress_count = count;
                                }
                                let message = format!(
                                    "Imported {} document{}",
                                    count,
                                    if count == 1 { "" } else { "s" }
                                );
                                state.set_status_message(Some(StatusMessage::info(message)));
                                cx.emit(AppEvent::TransferCompleted { transfer_id, count });
                            }
                            Err(err) => {
                                if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                    tab.error_message = Some(err.to_string());
                                }
                                state.set_status_message(Some(StatusMessage::error(format!(
                                    "Import failed: {err}"
                                ))));
                                cx.emit(AppEvent::TransferFailed {
                                    transfer_id,
                                    error: err.to_string(),
                                });
                            }
                        }
                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }

    fn execute_copy(
        state: Entity<AppState>,
        transfer_id: Uuid,
        config: crate::state::TransferTabState,
        cx: &mut App,
    ) {
        let Some(src_conn_id) = config.source_connection_id else {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "No source connection selected",
                )));
                cx.notify();
            });
            return;
        };

        let Some(dest_conn_id) = config.destination_connection_id else {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "No destination connection selected",
                )));
                cx.notify();
            });
            return;
        };

        if !Self::ensure_writable(&state, Some(dest_conn_id), cx) {
            return;
        }

        let Some(src_client) = Self::active_client(&state, src_conn_id, cx) else {
            return;
        };

        let Some(dest_client) = Self::active_client(&state, dest_conn_id, cx) else {
            return;
        };

        let src_database = config.source_database.clone();
        let src_collection = config.source_collection.clone();
        let dest_database = if config.destination_database.is_empty() {
            config.source_database.clone()
        } else {
            config.destination_database.clone()
        };
        let dest_collection = if config.destination_collection.is_empty() {
            config.source_collection.clone()
        } else {
            config.destination_collection.clone()
        };
        let scope = config.scope;
        let batch_size = config.batch_size as usize;

        state.update(cx, |state, cx| {
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                tab.is_running = true;
                tab.progress_count = 0;
                tab.error_message = None;
            }
            state.set_status_message(Some(StatusMessage::info("Copying...")));
            cx.emit(AppEvent::TransferStarted { transfer_id });
            cx.notify();
        });

        let task = cx.background_spawn(async move {
            let manager = get_connection_manager();

            match scope {
                TransferScope::Collection => manager.copy_collection(
                    &src_client,
                    &src_database,
                    &src_collection,
                    &dest_client,
                    &dest_database,
                    &dest_collection,
                    batch_size,
                ),
                TransferScope::Database => manager.copy_database(
                    &src_client,
                    &src_database,
                    &dest_client,
                    &dest_database,
                    batch_size,
                ),
            }
        });

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result = task.await;
                let _ = cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                            tab.is_running = false;
                        }

                        match result {
                            Ok(count) => {
                                if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                    tab.progress_count = count;
                                }
                                let message = format!(
                                    "Copied {} document{}",
                                    count,
                                    if count == 1 { "" } else { "s" }
                                );
                                state.set_status_message(Some(StatusMessage::info(message)));
                                cx.emit(AppEvent::TransferCompleted { transfer_id, count });
                            }
                            Err(err) => {
                                if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                    tab.error_message = Some(err.to_string());
                                }
                                state.set_status_message(Some(StatusMessage::error(format!(
                                    "Copy failed: {err}"
                                ))));
                                cx.emit(AppEvent::TransferFailed {
                                    transfer_id,
                                    error: err.to_string(),
                                });
                            }
                        }
                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }

    // Legacy functions for backward compatibility

    #[allow(dead_code)]
    pub fn export_collection_json(
        state: Entity<AppState>,
        session_key: SessionKey,
        format: JsonTransferFormat,
        path: PathBuf,
        cx: &mut App,
    ) {
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };

        let (database, collection) = (session_key.database.clone(), session_key.collection.clone());

        state.update(cx, |state, cx| {
            state.set_status_message(Some(StatusMessage::info("Exporting collection...")));
            cx.notify();
        });

        let task = cx.background_spawn({
            let path = path.clone();
            async move {
                let manager = get_connection_manager();
                manager.export_collection_json(&client, &database, &collection, format, &path)
            }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<u64, crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(count) => {
                        state.update(cx, |state, cx| {
                            let message = format!(
                                "Exported {} document{}",
                                count,
                                if count == 1 { "" } else { "s" }
                            );
                            state.set_status_message(Some(StatusMessage::info(message)));
                            cx.emit(AppEvent::DocumentsLoaded {
                                session: session_key.clone(),
                                total: count,
                            });
                            cx.notify();
                        });
                    }
                    Err(err) => {
                        state.update(cx, |state, cx| {
                            state.set_status_message(Some(StatusMessage::error(format!(
                                "Export failed: {err}",
                            ))));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }

    #[allow(dead_code)]
    pub fn import_collection_json(
        state: Entity<AppState>,
        session_key: SessionKey,
        format: JsonTransferFormat,
        path: PathBuf,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            return;
        }
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };

        let (database, collection) = (session_key.database.clone(), session_key.collection.clone());

        state.update(cx, |state, cx| {
            state.set_status_message(Some(StatusMessage::info("Importing collection...")));
            cx.notify();
        });

        let task = cx.background_spawn({
            let path = path.clone();
            async move {
                let manager = get_connection_manager();
                manager.import_collection_json(&client, &database, &collection, format, &path, 1000)
            }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<u64, crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(count) => {
                        state.update(cx, |state, cx| {
                            let message = format!(
                                "Imported {} document{}",
                                count,
                                if count == 1 { "" } else { "s" }
                            );
                            state.set_status_message(Some(StatusMessage::info(message)));
                            cx.notify();
                        });
                        AppCommands::load_documents_for_session(
                            state.clone(),
                            session_key.clone(),
                            cx,
                        );
                    }
                    Err(err) => {
                        state.update(cx, |state, cx| {
                            state.set_status_message(Some(StatusMessage::error(format!(
                                "Import failed: {err}",
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
