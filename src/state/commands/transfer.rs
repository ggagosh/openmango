//! Transfer commands for import, export, and copy operations.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Entity};
use uuid::Uuid;

use crate::bson::parse_document_from_json;
use crate::connection::csv_utils::detect_problematic_fields;
use crate::connection::get_connection_manager;

/// Maximum number of collections to process concurrently for database-scope operations.
const PARALLEL_COLLECTION_LIMIT: usize = 4;
use crate::connection::mongo::{
    BsonOutputFormat as MongoBsonOutputFormat, BsonToolProgress, CsvImportOptions,
    ExportQueryOptions, ExtendedJsonMode, FileEncoding, InsertMode, JsonExportOptions,
    JsonImportOptions, JsonTransferFormat, generate_export_preview,
};
use crate::state::app_state::CollectionTransferStatus;
use crate::state::{
    AppCommands, AppEvent, AppState, SessionKey, StatusMessage, TransferFormat, TransferMode,
    TransferScope, expand_filename_template,
};

/// Lightweight config for export operations (avoids cloning full TransferTabState).
struct ExportConfig {
    source_connection_id: Option<Uuid>,
    source_database: String,
    source_collection: String,
    file_path: String,
    format: TransferFormat,
    scope: TransferScope,
    json_mode: crate::state::ExtendedJsonMode,
    pretty_print: bool,
    bson_output: crate::state::BsonOutputFormat,
    compression: crate::state::CompressionMode,
    export_filter: String,
    export_projection: String,
    export_sort: String,
    exclude_collections: Vec<String>,
}

/// Lightweight config for import operations (avoids cloning full TransferTabState).
struct ImportConfig {
    source_connection_id: Option<Uuid>,
    source_database: String,
    source_collection: String,
    destination_database: String,
    destination_collection: String,
    file_path: String,
    format: TransferFormat,
    scope: TransferScope,
    insert_mode: crate::state::InsertMode,
    stop_on_error: bool,
    batch_size: u32,
    drop_before_import: bool,
    clear_before_import: bool,
    encoding: crate::state::Encoding,
    detect_format: bool,
}

/// Lightweight config for copy operations (avoids cloning full TransferTabState).
struct CopyConfig {
    source_connection_id: Option<Uuid>,
    destination_connection_id: Option<Uuid>,
    source_database: String,
    source_collection: String,
    destination_database: String,
    destination_collection: String,
    scope: TransferScope,
    batch_size: u32,
    drop_before_import: bool,
    clear_before_import: bool,
    copy_indexes: bool,
    exclude_collections: Vec<String>,
}

/// Variant enum for transfer config dispatch.
enum TransferConfigVariant {
    Export(ExportConfig),
    Import(ImportConfig),
    Copy(CopyConfig),
}

/// Progress messages sent from background export/copy tasks to the UI thread.
#[derive(Debug)]
enum TransferProgressMessage {
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
enum CollectionProgressMessage {
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
    /// Extracts only the needed fields to avoid cloning the entire TransferTabState.
    pub fn execute_transfer(state: Entity<AppState>, transfer_id: Uuid, cx: &mut App) {
        // Extract only the needed config fields without cloning the entire struct
        let config = {
            let state_ref = state.read(cx);
            let Some(tab) = state_ref.transfer_tab(transfer_id) else {
                return;
            };

            match tab.mode {
                TransferMode::Export => TransferConfigVariant::Export(ExportConfig {
                    source_connection_id: tab.source_connection_id,
                    source_database: tab.source_database.clone(),
                    source_collection: tab.source_collection.clone(),
                    file_path: tab.file_path.clone(),
                    format: tab.format,
                    scope: tab.scope,
                    json_mode: tab.json_mode,
                    pretty_print: tab.pretty_print,
                    bson_output: tab.bson_output,
                    compression: tab.compression,
                    export_filter: tab.export_filter.clone(),
                    export_projection: tab.export_projection.clone(),
                    export_sort: tab.export_sort.clone(),
                    exclude_collections: tab.exclude_collections.clone(),
                }),
                TransferMode::Import => TransferConfigVariant::Import(ImportConfig {
                    source_connection_id: tab.source_connection_id,
                    source_database: tab.source_database.clone(),
                    source_collection: tab.source_collection.clone(),
                    destination_database: tab.destination_database.clone(),
                    destination_collection: tab.destination_collection.clone(),
                    file_path: tab.file_path.clone(),
                    format: tab.format,
                    scope: tab.scope,
                    insert_mode: tab.insert_mode,
                    stop_on_error: tab.stop_on_error,
                    batch_size: tab.batch_size,
                    drop_before_import: tab.drop_before_import,
                    clear_before_import: tab.clear_before_import,
                    encoding: tab.encoding,
                    detect_format: tab.detect_format,
                }),
                TransferMode::Copy => TransferConfigVariant::Copy(CopyConfig {
                    source_connection_id: tab.source_connection_id,
                    destination_connection_id: tab.destination_connection_id,
                    source_database: tab.source_database.clone(),
                    source_collection: tab.source_collection.clone(),
                    destination_database: tab.destination_database.clone(),
                    destination_collection: tab.destination_collection.clone(),
                    scope: tab.scope,
                    batch_size: tab.batch_size,
                    drop_before_import: tab.drop_before_import,
                    clear_before_import: tab.clear_before_import,
                    copy_indexes: tab.copy_indexes,
                    exclude_collections: tab.exclude_collections.clone(),
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
                tab.transfer_generation.fetch_add(1, Ordering::SeqCst);

                // Abort any pending async operation
                if let Ok(mut handle) = tab.abort_handle.lock()
                    && let Some(h) = handle.take()
                {
                    h.abort();
                }

                tab.is_running = false;
                tab.error_message = Some("Transfer cancelled".to_string());
            }
            state.set_status_message(Some(StatusMessage::info("Transfer cancelled")));
            cx.emit(AppEvent::TransferCancelled { transfer_id });
            cx.notify();
        });
    }

    fn execute_export(
        state: Entity<AppState>,
        transfer_id: Uuid,
        config: ExportConfig,
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

        // For BSON export, we need the connection string instead of client
        let connection_uri = if matches!(config.format, TransferFormat::Bson) {
            state.read(cx).connection_uri(connection_id)
        } else {
            None
        };

        let client = if matches!(config.format, TransferFormat::Bson) {
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

        // Extract fields from lightweight config (already cloned during extraction)
        let database = config.source_database;
        let collection = config.source_collection;

        // Expand placeholders in file path (e.g., ${datetime}, ${database}, ${collection})
        let expanded_path = expand_filename_template(&config.file_path, &database, &collection);
        let path = PathBuf::from(&expanded_path);
        let format = config.format;
        let scope = config.scope;
        let json_mode = match config.json_mode {
            crate::state::ExtendedJsonMode::Relaxed => ExtendedJsonMode::Relaxed,
            crate::state::ExtendedJsonMode::Canonical => ExtendedJsonMode::Canonical,
        };
        let pretty_print = config.pretty_print;
        let bson_output = match config.bson_output {
            crate::state::BsonOutputFormat::Folder => MongoBsonOutputFormat::Folder,
            crate::state::BsonOutputFormat::Archive => MongoBsonOutputFormat::Archive,
        };
        let gzip = matches!(config.compression, crate::state::CompressionMode::Gzip);

        // Export query options (only for collection scope) - already cloned
        let export_filter = config.export_filter;
        let export_projection = config.export_projection;
        let export_sort = config.export_sort;

        let exclude_collections = config.exclude_collections;

        state.update(cx, |state, cx| {
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                tab.is_running = true;
                tab.progress_count = 0;
                tab.error_message = None;
                tab.database_progress = None; // Reset on new export
            }
            state.set_status_message(Some(StatusMessage::info("Exporting...")));
            cx.emit(AppEvent::TransferStarted { transfer_id });
            cx.notify();
        });

        // For database scope with JSON/CSV formats, use progress tracking
        if let Some(ref client) = client
            && matches!(scope, TransferScope::Database)
            && !matches!(format, TransferFormat::Bson)
        {
            let client = client.clone();
            Self::execute_database_export_with_progress(
                state,
                transfer_id,
                client,
                database,
                path,
                format,
                json_mode,
                pretty_print,
                gzip,
                exclude_collections,
                cx,
            );
            return;
        }

        // Collection scope with JSON/CSV - use progress tracking via channel
        if let Some(ref client) = client
            && matches!(scope, TransferScope::Collection)
            && !matches!(format, TransferFormat::Bson)
        {
            let client = client.clone();
            Self::execute_collection_export_with_progress(
                state,
                transfer_id,
                client,
                database,
                collection,
                path,
                format,
                json_mode,
                pretty_print,
                gzip,
                export_filter,
                export_projection,
                export_sort,
                cx,
            );
            return;
        }

        // BSON format (database scope only) - use progress tracking
        if matches!(format, TransferFormat::Bson) && matches!(scope, TransferScope::Database) {
            let uri = match connection_uri {
                Some(uri) => uri,
                None => {
                    state.update(cx, |state, cx| {
                        state.set_status_message(Some(StatusMessage::error(
                            "Connection URI not available",
                        )));
                        cx.notify();
                    });
                    return;
                }
            };
            Self::execute_bson_export_with_progress(
                state,
                transfer_id,
                uri,
                database,
                path,
                bson_output,
                gzip,
                exclude_collections,
                cx,
            );
            return;
        }

        // Fallback for unexpected cases
        state.update(cx, |state, cx| {
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                tab.is_running = false;
                tab.error_message = Some("Unexpected export configuration".to_string());
            }
            cx.notify();
        });
    }

    /// Execute database export with per-collection progress tracking.
    /// Uses a channel to send progress from background thread to UI thread.
    #[allow(clippy::too_many_arguments)]
    fn execute_database_export_with_progress(
        state: Entity<AppState>,
        transfer_id: Uuid,
        client: mongodb::Client,
        database: String,
        path: PathBuf,
        format: TransferFormat,
        json_mode: ExtendedJsonMode,
        pretty_print: bool,
        gzip: bool,
        exclude_collections: Vec<String>,
        cx: &mut App,
    ) {
        // Create channel for progress updates from background thread
        let (tx, rx) = mpsc::unbounded::<TransferProgressMessage>();

        // Spawn background task that does all blocking I/O
        cx.background_spawn({
            let exclude_set: HashSet<String> = exclude_collections.iter().cloned().collect();
            async move {
                let manager = get_connection_manager();

                // Get collection list
                let collections = match manager.list_collection_names(&client, &database) {
                    Ok(colls) => colls
                        .into_iter()
                        .filter(|c| !c.starts_with("system.") && !exclude_set.contains(c))
                        .collect::<Vec<_>>(),
                    Err(e) => {
                        let _ = tx.unbounded_send(TransferProgressMessage::Failed {
                            error: e.to_string(),
                        });
                        return;
                    }
                };

                // Send started message
                let _ = tx.unbounded_send(TransferProgressMessage::Started {
                    collections: collections.clone(),
                });

                // Create output directory
                if let Err(e) = std::fs::create_dir_all(&path) {
                    let _ =
                        tx.unbounded_send(TransferProgressMessage::Failed { error: e.to_string() });
                    return;
                }

                // Get runtime handle for spawning blocking tasks
                let runtime_handle = manager.runtime_handle();

                // Export collections in parallel using spawn_blocking
                let results: Vec<(String, Result<u64, crate::error::Error>)> =
                    futures::stream::iter(collections)
                        .map(|collection_name| {
                            let tx = tx.clone();
                            let client = client.clone();
                            let database = database.clone();
                            let path = path.clone();
                            let handle = runtime_handle.clone();

                            async move {
                                // Send InProgress status
                                let _ = tx.unbounded_send(
                                    TransferProgressMessage::CollectionProgress {
                                        collection_name: collection_name.clone(),
                                        status: CollectionTransferStatus::InProgress,
                                        documents_processed: 0,
                                        documents_total: None,
                                    },
                                );

                                // Spawn blocking task on Tokio runtime for actual export
                                let export_tx = tx.clone();
                                let export_collection = collection_name.clone();
                                let result = handle
                                    .spawn_blocking(move || {
                                        let manager = get_connection_manager();

                                        // Get estimated count (fast metadata query)
                                        let estimated_count = manager
                                            .estimated_document_count(
                                                &client,
                                                &database,
                                                &export_collection,
                                            )
                                            .ok();

                                        if estimated_count.is_some() {
                                            let _ = export_tx.unbounded_send(
                                                TransferProgressMessage::CollectionProgress {
                                                    collection_name: export_collection.clone(),
                                                    status: CollectionTransferStatus::InProgress,
                                                    documents_processed: 0,
                                                    documents_total: estimated_count,
                                                },
                                            );
                                        }

                                        // Determine file extension
                                        let extension = match format {
                                            TransferFormat::JsonLines => {
                                                if gzip {
                                                    "jsonl.gz"
                                                } else {
                                                    "jsonl"
                                                }
                                            }
                                            TransferFormat::JsonArray => {
                                                if gzip {
                                                    "json.gz"
                                                } else {
                                                    "json"
                                                }
                                            }
                                            TransferFormat::Csv => {
                                                if gzip {
                                                    "csv.gz"
                                                } else {
                                                    "csv"
                                                }
                                            }
                                            TransferFormat::Bson => "bson",
                                        };
                                        let file_path = path
                                            .join(format!("{}.{}", export_collection, extension));

                                        // Create progress callback
                                        let progress_tx = export_tx.clone();
                                        let progress_name = export_collection.clone();
                                        let progress_total = estimated_count;
                                        let on_progress = move |processed: u64| {
                                            let _ = progress_tx.unbounded_send(
                                                TransferProgressMessage::CollectionProgress {
                                                    collection_name: progress_name.clone(),
                                                    status: CollectionTransferStatus::InProgress,
                                                    documents_processed: processed,
                                                    documents_total: progress_total,
                                                },
                                            );
                                        };

                                        // Export collection
                                        match format {
                                            TransferFormat::JsonLines
                                            | TransferFormat::JsonArray => {
                                                let json_format = match format {
                                                    TransferFormat::JsonLines => {
                                                        JsonTransferFormat::JsonLines
                                                    }
                                                    _ => JsonTransferFormat::JsonArray,
                                                };
                                                let json_options = JsonExportOptions {
                                                    format: json_format,
                                                    json_mode,
                                                    pretty_print,
                                                    gzip,
                                                };
                                                manager.export_collection_json_with_progress(
                                                    &client,
                                                    &database,
                                                    &export_collection,
                                                    &file_path,
                                                    json_options,
                                                    on_progress,
                                                )
                                            }
                                            TransferFormat::Csv => manager
                                                .export_collection_csv_with_progress(
                                                    &client,
                                                    &database,
                                                    &export_collection,
                                                    &file_path,
                                                    gzip,
                                                    on_progress,
                                                ),
                                            TransferFormat::Bson => {
                                                Err(crate::error::Error::Parse(
                                                    "BSON export should use separate path"
                                                        .to_string(),
                                                ))
                                            }
                                        }
                                    })
                                    .await
                                    .unwrap_or_else(|e| {
                                        Err(crate::error::Error::Parse(format!(
                                            "Task join error: {}",
                                            e
                                        )))
                                    });

                                // Send completion/error status
                                match &result {
                                    Ok(count) => {
                                        let _ = tx.unbounded_send(
                                            TransferProgressMessage::CollectionProgress {
                                                collection_name: collection_name.clone(),
                                                status: CollectionTransferStatus::Completed,
                                                documents_processed: *count,
                                                documents_total: Some(*count),
                                            },
                                        );
                                    }
                                    Err(e) => {
                                        let _ = tx.unbounded_send(
                                            TransferProgressMessage::CollectionProgress {
                                                collection_name: collection_name.clone(),
                                                status: CollectionTransferStatus::Failed(
                                                    e.to_string(),
                                                ),
                                                documents_processed: 0,
                                                documents_total: None,
                                            },
                                        );
                                    }
                                }

                                (collection_name, result)
                            }
                        })
                        .buffer_unordered(PARALLEL_COLLECTION_LIMIT)
                        .collect()
                        .await;

                // Aggregate results
                let total_exported: u64 =
                    results.iter().filter_map(|(_, r)| r.as_ref().ok().copied()).sum();
                let had_error = results.iter().any(|(_, r)| r.is_err());

                // Send completion
                let _ = tx.unbounded_send(TransferProgressMessage::Completed {
                    total_count: total_exported,
                    had_error,
                });
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
                        TransferProgressMessage::Started { .. }
                        | TransferProgressMessage::Completed { .. }
                        | TransferProgressMessage::Failed { .. } => true,
                        // Progress updates batch notify every BATCH_SIZE messages
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
                                        tab.is_running = false;
                                        tab.progress_count = total_count;
                                    }
                                    if had_error {
                                        state.set_status_message(Some(StatusMessage::error(
                                            format!(
                                                "Export completed with errors: {} documents",
                                                total_count
                                            ),
                                        )));
                                    } else {
                                        state.set_status_message(Some(StatusMessage::info(
                                            format!(
                                                "Exported {} document{}",
                                                total_count,
                                                if total_count == 1 { "" } else { "s" }
                                            ),
                                        )));
                                    }
                                    cx.emit(AppEvent::TransferCompleted {
                                        transfer_id,
                                        count: total_count,
                                    });
                                }
                                TransferProgressMessage::Failed { error } => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.is_running = false;
                                        tab.error_message = Some(error.clone());
                                    }
                                    state.set_status_message(Some(StatusMessage::error(format!(
                                        "Export failed: {error}"
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

    /// Execute BSON database export with progress tracking via mongodump stderr parsing.
    #[allow(clippy::too_many_arguments)]
    fn execute_bson_export_with_progress(
        state: Entity<AppState>,
        transfer_id: Uuid,
        connection_uri: String,
        database: String,
        path: PathBuf,
        output_format: MongoBsonOutputFormat,
        gzip: bool,
        exclude_collections: Vec<String>,
        cx: &mut App,
    ) {
        let (tx, rx) = mpsc::unbounded::<TransferProgressMessage>();

        // Spawn background task that runs mongodump with progress parsing
        cx.background_spawn({
            async move {
                let manager = get_connection_manager();

                // We don't know collection list upfront for BSON, but we'll discover them
                // Send a placeholder started message
                let _ = tx.unbounded_send(TransferProgressMessage::Started {
                    collections: vec![], // Will be discovered during export
                });

                let progress_tx = tx.clone();
                let result = manager.export_database_bson_with_progress(
                    &connection_uri,
                    &database,
                    output_format,
                    &path,
                    gzip,
                    &exclude_collections,
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
                            total_count: 0, // mongodump doesn't provide total count
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
                                        tab.is_running = false;
                                        tab.progress_count = total_count;
                                    }
                                    if had_error {
                                        state.set_status_message(Some(StatusMessage::error(
                                            "BSON export completed with errors".to_string(),
                                        )));
                                    } else {
                                        state.set_status_message(Some(StatusMessage::info(
                                            "BSON export completed".to_string(),
                                        )));
                                    }
                                    cx.emit(AppEvent::TransferCompleted {
                                        transfer_id,
                                        count: total_count,
                                    });
                                }
                                TransferProgressMessage::Failed { error } => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.is_running = false;
                                        tab.error_message = Some(error.clone());
                                    }
                                    state.set_status_message(Some(StatusMessage::error(format!(
                                        "BSON export failed: {error}"
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

    /// Execute collection export with progress tracking.
    /// Uses a channel to send progress from background thread to UI thread.
    #[allow(clippy::too_many_arguments)]
    fn execute_collection_export_with_progress(
        state: Entity<AppState>,
        transfer_id: Uuid,
        client: mongodb::Client,
        database: String,
        collection: String,
        path: PathBuf,
        format: TransferFormat,
        json_mode: ExtendedJsonMode,
        pretty_print: bool,
        gzip: bool,
        export_filter: String,
        export_projection: String,
        export_sort: String,
        cx: &mut App,
    ) {
        // Create channel for progress updates from background thread
        let (tx, rx) = mpsc::unbounded::<CollectionProgressMessage>();

        // Spawn background task that does all blocking I/O
        cx.background_spawn({
            async move {
                let manager = get_connection_manager();

                // Parse query options for collection-level exports using relaxed JSON parser
                let query_options = {
                    let filter = if !export_filter.is_empty() {
                        match parse_document_from_json(&export_filter) {
                            Ok(doc) => Some(doc),
                            Err(e) => {
                                let _ = tx.unbounded_send(CollectionProgressMessage::Failed(
                                    format!("Invalid filter: {}", e),
                                ));
                                return;
                            }
                        }
                    } else {
                        None
                    };

                    let projection = if !export_projection.is_empty() {
                        match parse_document_from_json(&export_projection) {
                            Ok(doc) => Some(doc),
                            Err(e) => {
                                let _ = tx.unbounded_send(CollectionProgressMessage::Failed(
                                    format!("Invalid projection: {}", e),
                                ));
                                return;
                            }
                        }
                    } else {
                        None
                    };

                    let sort = if !export_sort.is_empty() {
                        match parse_document_from_json(&export_sort) {
                            Ok(doc) => Some(doc),
                            Err(e) => {
                                let _ = tx.unbounded_send(CollectionProgressMessage::Failed(
                                    format!("Invalid sort: {}", e),
                                ));
                                return;
                            }
                        }
                    } else {
                        None
                    };

                    ExportQueryOptions { filter, projection, sort }
                };

                // Create progress callback that sends updates via channel
                let progress_tx = tx.clone();
                let on_progress = move |processed: u64| {
                    let _ =
                        progress_tx.unbounded_send(CollectionProgressMessage::Progress(processed));
                };

                let result = match format {
                    TransferFormat::JsonLines | TransferFormat::JsonArray => {
                        let json_format = match format {
                            TransferFormat::JsonLines => JsonTransferFormat::JsonLines,
                            _ => JsonTransferFormat::JsonArray,
                        };
                        let json_options = JsonExportOptions {
                            format: json_format,
                            json_mode,
                            pretty_print,
                            gzip,
                        };

                        manager.export_collection_json_with_query_and_progress(
                            &client,
                            &database,
                            &collection,
                            &path,
                            json_options,
                            query_options,
                            on_progress,
                        )
                    }
                    TransferFormat::Csv => manager.export_collection_csv_with_query_and_progress(
                        &client,
                        &database,
                        &collection,
                        &path,
                        gzip,
                        query_options,
                        on_progress,
                    ),
                    TransferFormat::Bson => {
                        let _ = tx.unbounded_send(CollectionProgressMessage::Failed(
                            "BSON export is only supported for database scope".to_string(),
                        ));
                        return;
                    }
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
                                        tab.progress_count = processed;
                                    }
                                }
                                CollectionProgressMessage::Completed(count) => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.is_running = false;
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
                                CollectionProgressMessage::Failed(error) => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.is_running = false;
                                        tab.error_message = Some(error.clone());
                                    }
                                    state.set_status_message(Some(StatusMessage::error(format!(
                                        "Export failed: {error}"
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

    fn execute_import(
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
        let insert_mode = match config.insert_mode {
            crate::state::InsertMode::Insert => InsertMode::Insert,
            crate::state::InsertMode::Upsert => InsertMode::Upsert,
            crate::state::InsertMode::Replace => InsertMode::Replace,
        };
        let stop_on_error = config.stop_on_error;
        let batch_size = config.batch_size as usize;
        let drop_before = config.drop_before_import;
        let clear_before = config.clear_before_import;
        let encoding = match config.encoding {
            crate::state::Encoding::Utf8 => FileEncoding::Utf8,
            crate::state::Encoding::Latin1 => FileEncoding::Latin1,
        };

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

        // For collection-level JSON/CSV imports, use progress tracking via channel
        if let Some(ref client) = client
            && matches!(scope, TransferScope::Collection)
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
                cx,
            );
            return;
        }

        // BSON import (database scope only) - use progress tracking
        if matches!(format, TransferFormat::Bson) && matches!(scope, TransferScope::Database) {
            let uri = match connection_uri {
                Some(uri) => uri,
                None => {
                    state.update(cx, |state, cx| {
                        state.set_status_message(Some(StatusMessage::error(
                            "Connection URI not available",
                        )));
                        if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                            tab.is_running = false;
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
                tab.is_running = false;
                tab.error_message = Some("Unexpected import configuration".to_string());
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

        // Spawn background task that runs mongorestore with progress parsing
        cx.background_spawn({
            async move {
                let manager = get_connection_manager();

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
                                        tab.is_running = false;
                                        tab.progress_count = total_count;
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
                                        tab.is_running = false;
                                        tab.error_message = Some(error.clone());
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
        encoding: FileEncoding,
        drop_before: bool,
        clear_before: bool,
        cx: &mut App,
    ) {
        use crate::connection::mongo::ProgressCallback;

        // Create channel for progress updates from background thread
        let (tx, rx) = mpsc::unbounded::<CollectionProgressMessage>();

        // Spawn background task that does all blocking I/O
        cx.background_spawn({
            async move {
                let manager = get_connection_manager();

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
                                cancellation: None,
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
                            cancellation: None,
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
                                        tab.progress_count = processed;
                                    }
                                }
                                CollectionProgressMessage::Completed(count) => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.is_running = false;
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
                                CollectionProgressMessage::Failed(error) => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.is_running = false;
                                        tab.error_message = Some(error.clone());
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

    fn execute_copy(state: Entity<AppState>, transfer_id: Uuid, config: CopyConfig, cx: &mut App) {
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

        // Fields already cloned in lightweight config - use directly or compute fallbacks
        let src_database = config.source_database.clone(); // Clone needed for dest fallback check
        let src_collection = config.source_collection.clone(); // Clone needed for dest fallback check
        let dest_database = if config.destination_database.is_empty() {
            src_database.clone()
        } else {
            config.destination_database
        };
        let dest_collection = if config.destination_collection.is_empty() {
            src_collection.clone()
        } else {
            config.destination_collection
        };
        let scope = config.scope;
        let batch_size = config.batch_size as usize;
        let drop_before = config.drop_before_import;
        let clear_before = config.clear_before_import;
        let copy_indexes = config.copy_indexes;
        let exclude_collections = config.exclude_collections;

        state.update(cx, |state, cx| {
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                tab.is_running = true;
                tab.progress_count = 0;
                tab.error_message = None;
                tab.database_progress = None; // Reset on new copy
            }
            state.set_status_message(Some(StatusMessage::info("Copying...")));
            cx.emit(AppEvent::TransferStarted { transfer_id });
            cx.notify();
        });

        match scope {
            TransferScope::Collection => {
                // Single collection copy with progress tracking
                Self::execute_collection_copy_with_progress(
                    state,
                    transfer_id,
                    src_client,
                    dest_client,
                    src_database,
                    src_collection,
                    dest_database,
                    dest_collection,
                    batch_size,
                    copy_indexes,
                    drop_before,
                    clear_before,
                    cx,
                );
            }
            TransferScope::Database => {
                // Database copy with progress tracking
                Self::execute_database_copy_with_progress(
                    state,
                    transfer_id,
                    src_client,
                    dest_client,
                    src_database,
                    dest_database,
                    batch_size,
                    copy_indexes,
                    exclude_collections,
                    cx,
                );
            }
        }
    }

    /// Execute database copy with per-collection progress tracking.
    /// Uses a channel to send progress from background thread to UI thread.
    #[allow(clippy::too_many_arguments)]
    fn execute_database_copy_with_progress(
        state: Entity<AppState>,
        transfer_id: Uuid,
        src_client: mongodb::Client,
        dest_client: mongodb::Client,
        src_database: String,
        dest_database: String,
        batch_size: usize,
        copy_indexes: bool,
        exclude_collections: Vec<String>,
        cx: &mut App,
    ) {
        // Create channel for progress updates from background thread
        let (tx, rx) = mpsc::unbounded::<TransferProgressMessage>();

        // Spawn background task that does all blocking I/O
        cx.background_spawn({
            let exclude_set: HashSet<String> = exclude_collections.iter().cloned().collect();
            async move {
                let manager = get_connection_manager();

                // Get collection list
                let collections = match manager.list_collection_names(&src_client, &src_database) {
                    Ok(colls) => colls
                        .into_iter()
                        .filter(|c| !c.starts_with("system.") && !exclude_set.contains(c))
                        .collect::<Vec<_>>(),
                    Err(e) => {
                        let _ = tx.unbounded_send(TransferProgressMessage::Failed {
                            error: e.to_string(),
                        });
                        return;
                    }
                };

                // Send started message
                let _ = tx.unbounded_send(TransferProgressMessage::Started {
                    collections: collections.clone(),
                });

                // Get runtime handle for spawning blocking tasks
                let runtime_handle = manager.runtime_handle();

                // Copy collections in parallel using spawn_blocking
                let results: Vec<(String, Result<u64, crate::error::Error>)> =
                    futures::stream::iter(collections)
                        .map(|collection_name| {
                            let tx = tx.clone();
                            let src_client = src_client.clone();
                            let dest_client = dest_client.clone();
                            let src_database = src_database.clone();
                            let dest_database = dest_database.clone();
                            let handle = runtime_handle.clone();

                            async move {
                                // Send InProgress status
                                let _ = tx.unbounded_send(
                                    TransferProgressMessage::CollectionProgress {
                                        collection_name: collection_name.clone(),
                                        status: CollectionTransferStatus::InProgress,
                                        documents_processed: 0,
                                        documents_total: None,
                                    },
                                );

                                // Spawn blocking task on Tokio runtime for actual copy
                                let copy_tx = tx.clone();
                                let copy_collection = collection_name.clone();
                                let result = handle
                                    .spawn_blocking(move || {
                                        let manager = get_connection_manager();

                                        // Get estimated count
                                        let estimated_count = manager
                                            .estimated_document_count(
                                                &src_client,
                                                &src_database,
                                                &copy_collection,
                                            )
                                            .ok();

                                        if estimated_count.is_some() {
                                            let _ = copy_tx.unbounded_send(
                                                TransferProgressMessage::CollectionProgress {
                                                    collection_name: copy_collection.clone(),
                                                    status: CollectionTransferStatus::InProgress,
                                                    documents_processed: 0,
                                                    documents_total: estimated_count,
                                                },
                                            );
                                        }

                                        // Create progress callback
                                        let progress_tx = copy_tx.clone();
                                        let progress_name = copy_collection.clone();
                                        let progress_total = estimated_count;
                                        let progress_callback:
                                            crate::connection::mongo::ProgressCallback =
                                            std::sync::Arc::new(move |processed: u64| {
                                                let _ = progress_tx.unbounded_send(
                                                    TransferProgressMessage::CollectionProgress {
                                                        collection_name: progress_name.clone(),
                                                        status: CollectionTransferStatus::InProgress,
                                                        documents_processed: processed,
                                                        documents_total: progress_total,
                                                    },
                                                );
                                            });

                                        let copy_options = crate::connection::mongo::CopyOptions {
                                            batch_size,
                                            copy_indexes,
                                            progress: Some(progress_callback),
                                            cancellation: None,
                                        };

                                        // Copy collection
                                        manager.copy_collection_with_options(
                                            &src_client,
                                            &src_database,
                                            &copy_collection,
                                            &dest_client,
                                            &dest_database,
                                            &copy_collection,
                                            copy_options,
                                        )
                                    })
                                    .await
                                    .unwrap_or_else(|e| {
                                        Err(crate::error::Error::Parse(format!(
                                            "Task join error: {}",
                                            e
                                        )))
                                    });

                                // Send completion/error status
                                match &result {
                                    Ok(count) => {
                                        let _ = tx.unbounded_send(
                                            TransferProgressMessage::CollectionProgress {
                                                collection_name: collection_name.clone(),
                                                status: CollectionTransferStatus::Completed,
                                                documents_processed: *count,
                                                documents_total: Some(*count),
                                            },
                                        );
                                    }
                                    Err(e) => {
                                        let _ = tx.unbounded_send(
                                            TransferProgressMessage::CollectionProgress {
                                                collection_name: collection_name.clone(),
                                                status: CollectionTransferStatus::Failed(
                                                    e.to_string(),
                                                ),
                                                documents_processed: 0,
                                                documents_total: None,
                                            },
                                        );
                                    }
                                }

                                (collection_name, result)
                            }
                        })
                        .buffer_unordered(PARALLEL_COLLECTION_LIMIT)
                        .collect()
                        .await;

                // Aggregate results
                let total_copied: u64 =
                    results.iter().filter_map(|(_, r)| r.as_ref().ok().copied()).sum();
                let had_error = results.iter().any(|(_, r)| r.is_err());

                // Send completion
                let _ = tx.unbounded_send(TransferProgressMessage::Completed {
                    total_count: total_copied,
                    had_error,
                });
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
                        TransferProgressMessage::Started { .. }
                        | TransferProgressMessage::Completed { .. }
                        | TransferProgressMessage::Failed { .. } => true,
                        // Progress updates batch notify every BATCH_SIZE messages
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
                                        tab.is_running = false;
                                        tab.progress_count = total_count;
                                    }
                                    if had_error {
                                        state.set_status_message(Some(StatusMessage::error(
                                            format!(
                                                "Copy completed with errors: {} documents",
                                                total_count
                                            ),
                                        )));
                                    } else {
                                        state.set_status_message(Some(StatusMessage::info(
                                            format!(
                                                "Copied {} document{}",
                                                total_count,
                                                if total_count == 1 { "" } else { "s" }
                                            ),
                                        )));
                                    }
                                    cx.emit(AppEvent::TransferCompleted {
                                        transfer_id,
                                        count: total_count,
                                    });
                                }
                                TransferProgressMessage::Failed { error } => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.is_running = false;
                                        tab.error_message = Some(error.clone());
                                    }
                                    state.set_status_message(Some(StatusMessage::error(format!(
                                        "Copy failed: {error}"
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

    /// Execute collection copy with progress tracking.
    /// Uses a channel to send progress from background thread to UI thread.
    #[allow(clippy::too_many_arguments)]
    fn execute_collection_copy_with_progress(
        state: Entity<AppState>,
        transfer_id: Uuid,
        src_client: mongodb::Client,
        dest_client: mongodb::Client,
        src_database: String,
        src_collection: String,
        dest_database: String,
        dest_collection: String,
        batch_size: usize,
        copy_indexes: bool,
        drop_before: bool,
        clear_before: bool,
        cx: &mut App,
    ) {
        use crate::connection::mongo::{CopyOptions, ProgressCallback};

        // Create channel for progress updates from background thread
        let (tx, rx) = mpsc::unbounded::<CollectionProgressMessage>();

        // Spawn background task that does all blocking I/O
        cx.background_spawn({
            async move {
                let manager = get_connection_manager();

                // Drop or clear destination collection before copy if requested
                if drop_before {
                    let _ = manager.drop_collection(&dest_client, &dest_database, &dest_collection);
                } else if clear_before {
                    let _ = manager.delete_documents(
                        &dest_client,
                        &dest_database,
                        &dest_collection,
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

                let copy_options = CopyOptions {
                    batch_size,
                    copy_indexes,
                    progress: Some(progress_callback),
                    cancellation: None,
                };

                let result = manager.copy_collection_with_options(
                    &src_client,
                    &src_database,
                    &src_collection,
                    &dest_client,
                    &dest_database,
                    &dest_collection,
                    copy_options,
                );

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
                                        tab.progress_count = processed;
                                    }
                                }
                                CollectionProgressMessage::Completed(count) => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.is_running = false;
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
                                CollectionProgressMessage::Failed(error) => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.is_running = false;
                                        tab.error_message = Some(error.clone());
                                    }
                                    state.set_status_message(Some(StatusMessage::error(format!(
                                        "Copy failed: {error}"
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

/// Detect transfer format from file path extension.
fn detect_format_from_path(path: &str) -> Option<TransferFormat> {
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
