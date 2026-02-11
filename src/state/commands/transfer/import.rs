//! Import transfer operations.

use std::path::PathBuf;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Entity};
use uuid::Uuid;

use crate::connection::{
    BsonToolProgress, CsvImportOptions, Encoding, InsertMode, JsonImportOptions, JsonTransferFormat,
};
use crate::state::app_state::CollectionTransferStatus;
use crate::state::{AppCommands, AppEvent, AppState, StatusMessage, TransferFormat};

use super::{
    CollectionProgressMessage, ImportConfig, TransferProgressMessage, detect_format_from_path,
};

impl AppCommands {
    pub(super) fn execute_import(
        state: Entity<AppState>,
        transfer_id: Uuid,
        config: ImportConfig,
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

        // For BSON import, we need the connection string instead of client
        let connection_uri = if matches!(config.format, TransferFormat::Bson) {
            state.read(cx).connection_uri(connection_id)
        } else {
            None
        };

        let client = if matches!(config.format, TransferFormat::Bson) {
            // For BSON, we may still need client for drop_before_import with non-BSON formats
            None
        } else {
            Self::active_client(&state, connection_id, cx)
        };

        if !matches!(config.format, TransferFormat::Bson) && client.is_none() {
            return;
        }

        if config.file_path.is_empty() {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error("No file path specified")));
                cx.notify();
            });
            return;
        }

        let path = PathBuf::from(&config.file_path);
        // Use destination if set, otherwise fall back to source (already cloned in lightweight config)
        let database = if config.destination_database.is_empty() {
            config.source_database
        } else {
            config.destination_database
        };
        let collection = if config.destination_collection.is_empty() {
            config.source_collection
        } else {
            config.destination_collection
        };

        // Auto-detect format from file extension if enabled
        let format = if config.detect_format {
            detect_format_from_path(&config.file_path).unwrap_or(config.format)
        } else {
            config.format
        };

        let scope = config.scope;
        let insert_mode = config.insert_mode;
        let stop_on_error = config.stop_on_error;
        let batch_size = config.batch_size as usize;
        let drop_before = config.drop_before_import;
        let clear_before = config.clear_before_import;
        let encoding = config.encoding;

        let cancellation_token = crate::connection::types::CancellationToken::new();

        state.update(cx, |state, cx| {
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                tab.runtime.is_running = true;
                tab.runtime.progress_count = 0;
                tab.runtime.error_message = None;
                tab.runtime.cancellation_token = Some(cancellation_token.clone());
            }
            state.set_status_message(Some(StatusMessage::info("Importing...")));
            cx.emit(AppEvent::TransferStarted { transfer_id });
            cx.notify();
        });

        // For collection-level JSON/CSV imports, use progress tracking via channel
        if let Some(ref client) = client
            && matches!(scope, crate::state::TransferScope::Collection)
            && !matches!(format, TransferFormat::Bson)
        {
            let client = client.clone();
            Self::execute_collection_import_with_progress(
                state,
                transfer_id,
                client,
                database,
                collection,
                path,
                format,
                insert_mode,
                stop_on_error,
                batch_size,
                encoding,
                drop_before,
                clear_before,
                cancellation_token,
                cx,
            );
            return;
        }

        // BSON import (database scope only) - use progress tracking
        if matches!(format, TransferFormat::Bson)
            && matches!(scope, crate::state::TransferScope::Database)
        {
            let uri = match connection_uri {
                Some(uri) => uri,
                None => {
                    state.update(cx, |state, cx| {
                        state.set_status_message(Some(StatusMessage::error(
                            "Connection URI not available",
                        )));
                        if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                            tab.runtime.is_running = false;
                        }
                        cx.notify();
                    });
                    return;
                }
            };
            Self::execute_bson_import_with_progress(
                state,
                transfer_id,
                uri,
                database,
                path,
                drop_before,
                cx,
            );
            return;
        }

        // Fallback for unexpected cases
        state.update(cx, |state, cx| {
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                tab.runtime.is_running = false;
                tab.runtime.error_message = Some("Unexpected import configuration".to_string());
            }
            cx.notify();
        });
    }

    /// Execute BSON database import with progress tracking via mongorestore stderr parsing.
    fn execute_bson_import_with_progress(
        state: Entity<AppState>,
        transfer_id: Uuid,
        connection_uri: String,
        database: String,
        path: PathBuf,
        drop_before: bool,
        cx: &mut App,
    ) {
        let (tx, rx) = mpsc::unbounded::<TransferProgressMessage>();

        let manager = state.read(cx).connection_manager();

        // Spawn background task that runs mongorestore with progress parsing
        cx.background_spawn({
            async move {
                // Send a placeholder started message
                let _ = tx.unbounded_send(TransferProgressMessage::Started {
                    collections: vec![], // Will be discovered during import
                });

                let progress_tx = tx.clone();
                let result = manager.import_database_bson_with_progress(
                    &connection_uri,
                    &database,
                    &path,
                    drop_before,
                    move |progress| {
                        let msg = match progress {
                            BsonToolProgress::Started { collection } => {
                                TransferProgressMessage::CollectionProgress {
                                    collection_name: collection,
                                    status: CollectionTransferStatus::InProgress,
                                    documents_processed: 0,
                                    documents_total: None,
                                }
                            }
                            BsonToolProgress::Progress { collection, current, total, .. } => {
                                // mongorestore reports bytes, not documents
                                TransferProgressMessage::CollectionProgress {
                                    collection_name: collection,
                                    status: CollectionTransferStatus::InProgress,
                                    documents_processed: current,
                                    documents_total: Some(total),
                                }
                            }
                            BsonToolProgress::Completed { collection, documents } => {
                                TransferProgressMessage::CollectionProgress {
                                    collection_name: collection,
                                    status: CollectionTransferStatus::Completed,
                                    documents_processed: documents,
                                    documents_total: Some(documents),
                                }
                            }
                        };
                        let _ = progress_tx.unbounded_send(msg);
                    },
                );

                match result {
                    Ok(()) => {
                        let _ = tx.unbounded_send(TransferProgressMessage::Completed {
                            total_count: 0, // mongorestore doesn't provide total count
                            had_error: false,
                        });
                    }
                    Err(e) => {
                        let _ = tx.unbounded_send(TransferProgressMessage::Failed {
                            error: e.to_string(),
                        });
                    }
                }
            }
        })
        .detach();

        // Spawn UI task to receive progress updates
        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let mut rx = rx;
                let mut progress_count = 0u32;
                const BATCH_SIZE: u32 = 50;

                while let Some(msg) = rx.next().await {
                    let should_notify = match &msg {
                        TransferProgressMessage::Started { .. }
                        | TransferProgressMessage::Completed { .. }
                        | TransferProgressMessage::Failed { .. } => true,
                        TransferProgressMessage::CollectionProgress { .. } => {
                            progress_count += 1;
                            progress_count.is_multiple_of(BATCH_SIZE)
                        }
                    };

                    let _ = cx.update(|cx| {
                        state.update(cx, |state, cx| {
                            match msg {
                                TransferProgressMessage::Started { collections } => {
                                    let event = AppEvent::DatabaseTransferStarted {
                                        transfer_id,
                                        collections,
                                    };
                                    state.update_status_from_event(&event);
                                    cx.emit(event);
                                }
                                TransferProgressMessage::CollectionProgress {
                                    collection_name,
                                    status,
                                    documents_processed,
                                    documents_total,
                                } => {
                                    let event = AppEvent::CollectionProgressUpdate {
                                        transfer_id,
                                        collection_name,
                                        status,
                                        documents_processed,
                                        documents_total,
                                    };
                                    state.update_status_from_event(&event);
                                    cx.emit(event);
                                }
                                TransferProgressMessage::Completed { total_count, had_error } => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.runtime.is_running = false;
                                        tab.runtime.progress_count = total_count;
                                    }
                                    if had_error {
                                        state.set_status_message(Some(StatusMessage::error(
                                            "BSON import completed with errors".to_string(),
                                        )));
                                    } else {
                                        state.set_status_message(Some(StatusMessage::info(
                                            "BSON import completed".to_string(),
                                        )));
                                    }
                                    cx.emit(AppEvent::TransferCompleted {
                                        transfer_id,
                                        count: total_count,
                                    });
                                }
                                TransferProgressMessage::Failed { error } => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.runtime.is_running = false;
                                        tab.runtime.error_message = Some(error.clone());
                                    }
                                    state.set_status_message(Some(StatusMessage::error(format!(
                                        "BSON import failed: {error}"
                                    ))));
                                    cx.emit(AppEvent::TransferFailed { transfer_id, error });
                                }
                            }
                            if should_notify {
                                cx.notify();
                            }
                        });
                    });
                }
            }
        })
        .detach();
    }

    /// Execute collection import with progress tracking.
    /// Uses a channel to send progress from background thread to UI thread.
    #[allow(clippy::too_many_arguments)]
    fn execute_collection_import_with_progress(
        state: Entity<AppState>,
        transfer_id: Uuid,
        client: mongodb::Client,
        database: String,
        collection: String,
        path: PathBuf,
        format: TransferFormat,
        insert_mode: InsertMode,
        stop_on_error: bool,
        batch_size: usize,
        encoding: Encoding,
        drop_before: bool,
        clear_before: bool,
        cancellation_token: crate::connection::types::CancellationToken,
        cx: &mut App,
    ) {
        use crate::connection::ProgressCallback;

        // Create channel for progress updates from background thread
        let (tx, rx) = mpsc::unbounded::<CollectionProgressMessage>();

        let manager = state.read(cx).connection_manager();

        // Spawn background task that does all blocking I/O
        cx.background_spawn({
            async move {
                // Drop or clear collection before import if requested
                if drop_before {
                    let _ = manager.drop_collection(&client, &database, &collection);
                } else if clear_before {
                    let _ = manager.delete_documents(
                        &client,
                        &database,
                        &collection,
                        mongodb::bson::doc! {},
                    );
                }

                // Create progress callback that sends updates via channel
                let progress_tx = tx.clone();
                let progress_callback: ProgressCallback =
                    std::sync::Arc::new(move |processed: u64| {
                        let _ = progress_tx
                            .unbounded_send(CollectionProgressMessage::Progress(processed));
                    });

                let result = match format {
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
                                encoding,
                                progress: Some(progress_callback),
                                cancellation: Some(cancellation_token.clone()),
                            },
                        )
                    }
                    TransferFormat::Csv => manager.import_collection_csv(
                        &client,
                        &database,
                        &collection,
                        &path,
                        CsvImportOptions {
                            insert_mode,
                            stop_on_error,
                            batch_size,
                            encoding,
                            progress: Some(progress_callback),
                            cancellation: Some(cancellation_token),
                        },
                    ),
                    TransferFormat::Bson => Err(crate::error::Error::Parse(
                        "BSON import should use separate path".to_string(),
                    )),
                };

                match result {
                    Ok(count) => {
                        let _ = tx.unbounded_send(CollectionProgressMessage::Completed(count));
                    }
                    Err(err) => {
                        let _ =
                            tx.unbounded_send(CollectionProgressMessage::Failed(err.to_string()));
                    }
                }
            }
        })
        .detach();

        // Spawn UI task that receives progress and updates state
        // Batch progress updates to reduce cx.notify() calls from 1000s to ~20
        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let mut rx = rx;
                let mut progress_count = 0u32;
                const BATCH_SIZE: u32 = 50;

                while let Some(msg) = rx.next().await {
                    let should_notify = match &msg {
                        // Terminal events always notify immediately
                        CollectionProgressMessage::Completed(_)
                        | CollectionProgressMessage::Failed(_) => true,
                        // Progress updates batch notify every BATCH_SIZE messages
                        CollectionProgressMessage::Progress(_) => {
                            progress_count += 1;
                            progress_count.is_multiple_of(BATCH_SIZE)
                        }
                    };

                    let _ = cx.update(|cx| {
                        state.update(cx, |state, cx| {
                            match msg {
                                CollectionProgressMessage::Progress(processed) => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.runtime.progress_count = processed;
                                    }
                                }
                                CollectionProgressMessage::Completed(count) => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.runtime.is_running = false;
                                        tab.runtime.progress_count = count;
                                    }
                                    let message = format!(
                                        "Imported {} document{}",
                                        count,
                                        if count == 1 { "" } else { "s" }
                                    );
                                    state.set_status_message(Some(StatusMessage::info(message)));
                                    cx.emit(AppEvent::TransferCompleted { transfer_id, count });
                                }
                                CollectionProgressMessage::Failed(error) => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.runtime.is_running = false;
                                        tab.runtime.error_message = Some(error.clone());
                                    }
                                    state.set_status_message(Some(StatusMessage::error(format!(
                                        "Import failed: {error}"
                                    ))));
                                    cx.emit(AppEvent::TransferFailed { transfer_id, error });
                                }
                            }
                            if should_notify {
                                cx.notify();
                            }
                        });
                    });
                }
            }
        })
        .detach();
    }
}
