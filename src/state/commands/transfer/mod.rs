//! Transfer commands for import, export, and copy operations.

mod copy;
mod export;
mod import;

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use gpui::{App, AppContext as _, Entity};
use uuid::Uuid;

use crate::connection::csv_utils::detect_problematic_fields;
use crate::connection::{JsonTransferFormat, generate_export_preview};
use crate::state::app_state::CollectionTransferStatus;
use crate::state::{
    AppCommands, AppEvent, AppState, SessionKey, StatusMessage, TransferFormat, TransferMode,
};

/// Maximum number of collections to process concurrently for database-scope operations.
pub(super) const PARALLEL_COLLECTION_LIMIT: usize = 4;

/// Lightweight config for export operations (avoids cloning full TransferTabState).
pub(super) struct ExportConfig {
    pub source_connection_id: Option<Uuid>,
    pub source_database: String,
    pub source_collection: String,
    pub file_path: String,
    pub format: TransferFormat,
    pub scope: crate::state::TransferScope,
    pub json_mode: crate::state::ExtendedJsonMode,
    pub pretty_print: bool,
    pub bson_output: crate::state::BsonOutputFormat,
    pub compression: crate::state::CompressionMode,
    pub export_filter: String,
    pub export_projection: String,
    pub export_sort: String,
    pub exclude_collections: Vec<String>,
}

/// Lightweight config for import operations (avoids cloning full TransferTabState).
pub(super) struct ImportConfig {
    pub source_connection_id: Option<Uuid>,
    pub source_database: String,
    pub source_collection: String,
    pub destination_database: String,
    pub destination_collection: String,
    pub file_path: String,
    pub format: TransferFormat,
    pub scope: crate::state::TransferScope,
    pub insert_mode: crate::state::InsertMode,
    pub stop_on_error: bool,
    pub batch_size: u32,
    pub drop_before_import: bool,
    pub clear_before_import: bool,
    pub encoding: crate::state::Encoding,
    pub detect_format: bool,
}

/// Lightweight config for copy operations (avoids cloning full TransferTabState).
pub(super) struct CopyConfig {
    pub source_connection_id: Option<Uuid>,
    pub destination_connection_id: Option<Uuid>,
    pub source_database: String,
    pub source_collection: String,
    pub destination_database: String,
    pub destination_collection: String,
    pub scope: crate::state::TransferScope,
    pub batch_size: u32,
    pub insert_mode: crate::state::InsertMode,
    pub stop_on_error: bool,
    pub drop_before_import: bool,
    pub clear_before_import: bool,
    pub copy_indexes: bool,
    pub exclude_collections: Vec<String>,
}

/// Variant enum for transfer config dispatch.
enum TransferConfigVariant {
    Export(ExportConfig),
    Import(ImportConfig),
    Copy(CopyConfig),
}

/// Progress messages sent from background export/copy tasks to the UI thread.
#[derive(Debug)]
pub(super) enum TransferProgressMessage {
    /// Transfer started with list of collections
    Started { collections: Vec<String> },
    /// Collection progress update
    CollectionProgress {
        collection_name: String,
        status: CollectionTransferStatus,
        documents_processed: u64,
        documents_total: Option<u64>,
    },
    /// Transfer completed
    Completed { total_count: u64, had_error: bool },
    /// Transfer failed with error
    Failed { error: String },
}

/// Simple progress messages for collection-level operations (not database-scope).
#[derive(Debug)]
pub(super) enum CollectionProgressMessage {
    /// Progress update (document count so far)
    Progress(u64),
    /// Operation completed with final count
    Completed(u64),
    /// Operation failed with error
    Failed(String),
}

impl AppCommands {
    /// Load preview documents for a transfer tab.
    pub fn load_transfer_preview(state: Entity<AppState>, transfer_id: Uuid, cx: &mut App) {
        let (connection_id, database, collection, json_mode, pretty_print) = {
            let state_ref = state.read(cx);
            let Some(tab) = state_ref.transfer_tab(transfer_id) else {
                return;
            };

            // Only load preview for export mode with a valid source
            if !matches!(tab.config.mode, TransferMode::Export) {
                return;
            }

            let Some(conn_id) = tab.config.source_connection_id else {
                return;
            };

            if tab.config.source_database.is_empty() || tab.config.source_collection.is_empty() {
                return;
            }

            let json_mode = tab.options.json_mode;

            (
                conn_id,
                tab.config.source_database.clone(),
                tab.config.source_collection.clone(),
                json_mode,
                tab.options.pretty_print,
            )
        };

        let Some(client) = Self::active_client(&state, connection_id, cx) else {
            return;
        };

        let manager = state.read(cx).connection_manager();

        state.update(cx, |state, cx| {
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                tab.preview.loading = true;
                tab.preview.docs.clear();
                tab.preview.warnings.clear();
            }
            cx.notify();
        });

        let task = cx.background_spawn(async move {
            // Generate preview docs
            let preview = generate_export_preview(
                &manager,
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
                            tab.preview.loading = false;
                            match result {
                                Ok((preview, warnings)) => {
                                    tab.preview.docs = preview;
                                    tab.preview.warnings = warnings;
                                }
                                Err(e) => {
                                    tab.runtime.error_message = Some(e.to_string());
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
    /// Extracts only the needed fields to avoid cloning the entire TransferTabState.
    pub fn execute_transfer(state: Entity<AppState>, transfer_id: Uuid, cx: &mut App) {
        // Extract only the needed config fields without cloning the entire struct
        let config = {
            let state_ref = state.read(cx);
            let Some(tab) = state_ref.transfer_tab(transfer_id) else {
                return;
            };

            match tab.config.mode {
                TransferMode::Export => TransferConfigVariant::Export(ExportConfig {
                    source_connection_id: tab.config.source_connection_id,
                    source_database: tab.config.source_database.clone(),
                    source_collection: tab.config.source_collection.clone(),
                    file_path: tab.config.file_path.clone(),
                    format: tab.config.format,
                    scope: tab.config.scope,
                    json_mode: tab.options.json_mode,
                    pretty_print: tab.options.pretty_print,
                    bson_output: tab.options.bson_output,
                    compression: tab.options.compression,
                    export_filter: tab.options.export_filter.clone(),
                    export_projection: tab.options.export_projection.clone(),
                    export_sort: tab.options.export_sort.clone(),
                    exclude_collections: tab.options.exclude_collections.clone(),
                }),
                TransferMode::Import => TransferConfigVariant::Import(ImportConfig {
                    source_connection_id: tab.config.source_connection_id,
                    source_database: tab.config.source_database.clone(),
                    source_collection: tab.config.source_collection.clone(),
                    destination_database: tab.config.destination_database.clone(),
                    destination_collection: tab.config.destination_collection.clone(),
                    file_path: tab.config.file_path.clone(),
                    format: tab.config.format,
                    scope: tab.config.scope,
                    insert_mode: tab.options.insert_mode,
                    stop_on_error: tab.options.stop_on_error,
                    batch_size: tab.options.batch_size,
                    drop_before_import: tab.options.drop_before_import,
                    clear_before_import: tab.options.clear_before_import,
                    encoding: tab.options.encoding,
                    detect_format: tab.options.detect_format,
                }),
                TransferMode::Copy => TransferConfigVariant::Copy(CopyConfig {
                    source_connection_id: tab.config.source_connection_id,
                    destination_connection_id: tab.config.destination_connection_id,
                    source_database: tab.config.source_database.clone(),
                    source_collection: tab.config.source_collection.clone(),
                    destination_database: tab.config.destination_database.clone(),
                    destination_collection: tab.config.destination_collection.clone(),
                    scope: tab.config.scope,
                    batch_size: tab.options.batch_size,
                    insert_mode: tab.options.insert_mode,
                    stop_on_error: tab.options.stop_on_error,
                    drop_before_import: tab.options.drop_before_import,
                    clear_before_import: tab.options.clear_before_import,
                    copy_indexes: tab.options.copy_indexes,
                    exclude_collections: tab.options.exclude_collections.clone(),
                }),
            }
        };

        match config {
            TransferConfigVariant::Export(c) => Self::execute_export(state, transfer_id, c, cx),
            TransferConfigVariant::Import(c) => Self::execute_import(state, transfer_id, c, cx),
            TransferConfigVariant::Copy(c) => Self::execute_copy(state, transfer_id, c, cx),
        }
    }

    /// Cancel a running transfer operation.
    pub fn cancel_transfer(state: Entity<AppState>, transfer_id: Uuid, cx: &mut App) {
        state.update(cx, |state, cx| {
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                // Increment generation to invalidate any running operation
                tab.runtime.transfer_generation.fetch_add(1, Ordering::SeqCst);

                // Signal cancellation token (cooperative cancellation in loops)
                if let Some(ref token) = tab.runtime.cancellation_token {
                    token.cancel();
                }

                // Abort any pending async operation
                if let Ok(mut handle) = tab.runtime.abort_handle.lock()
                    && let Some(h) = handle.take()
                {
                    h.abort();
                }

                tab.runtime.is_running = false;
                tab.runtime.error_message = Some("Transfer cancelled".to_string());
            }
            state.set_status_message(Some(StatusMessage::info("Transfer cancelled")));
            cx.emit(AppEvent::TransferCancelled { transfer_id });
            cx.notify();
        });
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

        let manager = state.read(cx).connection_manager();

        state.update(cx, |state, cx| {
            state.set_status_message(Some(StatusMessage::info("Exporting collection...")));
            cx.notify();
        });

        let task =
            cx.background_spawn({
                let path = path.clone();
                async move {
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

        let manager = state.read(cx).connection_manager();

        state.update(cx, |state, cx| {
            state.set_status_message(Some(StatusMessage::info("Importing collection...")));
            cx.notify();
        });

        let task = cx.background_spawn({
            let path = path.clone();
            async move {
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

/// Detect transfer format from file path extension.
pub(super) fn detect_format_from_path(path: &str) -> Option<TransferFormat> {
    let path = std::path::Path::new(path);
    let ext = path.extension().and_then(|e| e.to_str())?.to_lowercase();

    match ext.as_str() {
        "jsonl" | "ndjson" => Some(TransferFormat::JsonLines),
        "json" => Some(TransferFormat::JsonArray),
        "csv" => Some(TransferFormat::Csv),
        "archive" | "bson" => Some(TransferFormat::Bson),
        "gz" => {
            // Check double extension: file.jsonl.gz
            let stem = path.file_stem()?.to_str()?;
            detect_format_from_path(stem)
        }
        _ => None,
    }
}
