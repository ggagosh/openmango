//! Export transfer operations.

use std::collections::HashSet;
use std::path::PathBuf;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Entity};
use uuid::Uuid;

use crate::bson::parse_document_from_json;
use crate::connection::{
    BsonOutputFormat as MongoBsonOutputFormat, BsonToolProgress, ExportQueryOptions,
    ExtendedJsonMode, JsonExportOptions, get_connection_manager,
};
use crate::state::app_state::CollectionTransferStatus;
use crate::state::{AppCommands, AppEvent, AppState, StatusMessage, TransferFormat};

use super::{CollectionProgressMessage, ExportConfig, TransferProgressMessage};

/// Maximum number of collections to process concurrently for database-scope operations.
const PARALLEL_COLLECTION_LIMIT: usize = 4;

impl AppCommands {
    pub(super) fn execute_export(
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
        let expanded_path =
            crate::state::expand_filename_template(&config.file_path, &database, &collection);
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
            && matches!(scope, crate::state::TransferScope::Database)
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
            && matches!(scope, crate::state::TransferScope::Collection)
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

                                // Build file path for this collection
                                let ext = match format {
                                    TransferFormat::JsonLines => {
                                        if gzip { "jsonl.gz" } else { "jsonl" }
                                    }
                                    TransferFormat::JsonArray => {
                                        if gzip { "json.gz" } else { "json" }
                                    }
                                    TransferFormat::Csv => {
                                        if gzip { "csv.gz" } else { "csv" }
                                    }
                                    TransferFormat::Bson => "bson",
                                };
                                let file_path = path.join(format!("{collection_name}.{ext}"));

                                // Execute blocking export
                                let coll_name_for_task = collection_name.clone();
                                let result = handle
                                    .spawn_blocking(move || {
                                        let manager = get_connection_manager();
                                        match format {
                                            TransferFormat::JsonLines
                                            | TransferFormat::JsonArray => {
                                                let json_options = JsonExportOptions {
                                                    format: if matches!(
                                                        format,
                                                        TransferFormat::JsonLines
                                                    ) {
                                                        crate::connection::JsonTransferFormat::JsonLines
                                                    } else {
                                                        crate::connection::JsonTransferFormat::JsonArray
                                                    },
                                                    json_mode,
                                                    pretty_print,
                                                    gzip,
                                                };
                                                manager.export_collection_json_with_options(
                                                    &client,
                                                    &database,
                                                    &coll_name_for_task,
                                                    &file_path,
                                                    json_options,
                                                )
                                            }
                                            TransferFormat::Csv => {
                                                manager.export_collection_csv(
                                                    &client,
                                                    &database,
                                                    &coll_name_for_task,
                                                    &file_path,
                                                    gzip,
                                                )
                                            }
                                            TransferFormat::Bson => {
                                                // BSON handled separately
                                                Ok(0)
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

                                // Send completion status
                                let (status, count) = match &result {
                                    Ok(count) => (CollectionTransferStatus::Completed, *count),
                                    Err(e) => (CollectionTransferStatus::Failed(e.to_string()), 0),
                                };
                                let _ = tx.unbounded_send(
                                    TransferProgressMessage::CollectionProgress {
                                        collection_name: collection_name.clone(),
                                        status,
                                        documents_processed: count,
                                        documents_total: Some(count),
                                    },
                                );

                                (collection_name, result)
                            }
                        })
                        .buffer_unordered(PARALLEL_COLLECTION_LIMIT)
                        .collect()
                        .await;

                // Calculate totals
                let mut total_count = 0u64;
                let mut had_error = false;
                for (_, result) in &results {
                    match result {
                        Ok(count) => total_count += count,
                        Err(_) => had_error = true,
                    }
                }

                // Send completed message
                let _ = tx.unbounded_send(TransferProgressMessage::Completed {
                    total_count,
                    had_error,
                });
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
                                            "Export completed with errors".to_string(),
                                        )));
                                    } else {
                                        state.set_status_message(Some(StatusMessage::info(
                                            format!("Exported {total_count} documents"),
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

        // Parse query options
        let filter = if export_filter.trim().is_empty() || export_filter.trim() == "{}" {
            None
        } else {
            parse_document_from_json(&export_filter).ok()
        };
        let projection = if export_projection.trim().is_empty() || export_projection.trim() == "{}"
        {
            None
        } else {
            parse_document_from_json(&export_projection).ok()
        };
        let sort = if export_sort.trim().is_empty() || export_sort.trim() == "{}" {
            None
        } else {
            parse_document_from_json(&export_sort).ok()
        };

        let query_options = if filter.is_some() || projection.is_some() || sort.is_some() {
            Some(ExportQueryOptions { filter, projection, sort })
        } else {
            None
        };

        // Spawn background task that does all blocking I/O
        cx.background_spawn({
            async move {
                let manager = get_connection_manager();
                let runtime_handle = manager.runtime_handle();

                let result = runtime_handle
                    .spawn_blocking(move || {
                        let manager = get_connection_manager();

                        match format {
                            TransferFormat::JsonLines | TransferFormat::JsonArray => {
                                let json_options = JsonExportOptions {
                                    format: if matches!(format, TransferFormat::JsonLines) {
                                        crate::connection::JsonTransferFormat::JsonLines
                                    } else {
                                        crate::connection::JsonTransferFormat::JsonArray
                                    },
                                    json_mode,
                                    pretty_print,
                                    gzip,
                                };
                                if let Some(query) = query_options {
                                    manager.export_collection_json_with_query(
                                        &client,
                                        &database,
                                        &collection,
                                        &path,
                                        json_options,
                                        query,
                                    )
                                } else {
                                    manager.export_collection_json_with_options(
                                        &client,
                                        &database,
                                        &collection,
                                        &path,
                                        json_options,
                                    )
                                }
                            }
                            TransferFormat::Csv => {
                                if let Some(query) = query_options {
                                    manager.export_collection_csv_with_query(
                                        &client,
                                        &database,
                                        &collection,
                                        &path,
                                        gzip,
                                        query,
                                    )
                                } else {
                                    manager.export_collection_csv(
                                        &client,
                                        &database,
                                        &collection,
                                        &path,
                                        gzip,
                                    )
                                }
                            }
                            TransferFormat::Bson => {
                                // BSON handled separately at database scope
                                Ok(0)
                            }
                        }
                    })
                    .await
                    .map_err(|e| crate::error::Error::Parse(e.to_string()))?;

                match result {
                    Ok(count) => {
                        let _ = tx.unbounded_send(CollectionProgressMessage::Completed(count));
                    }
                    Err(e) => {
                        let _ = tx.unbounded_send(CollectionProgressMessage::Failed(e.to_string()));
                    }
                }

                Ok::<(), crate::error::Error>(())
            }
        })
        .detach();

        // Spawn UI task to receive progress updates
        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let mut rx = rx;
                let mut progress_count = 0u32;
                const BATCH_SIZE: u32 = 100;

                while let Some(msg) = rx.next().await {
                    let should_notify = match &msg {
                        CollectionProgressMessage::Completed(_)
                        | CollectionProgressMessage::Failed(_) => true,
                        CollectionProgressMessage::Progress(_) => {
                            progress_count += 1;
                            progress_count.is_multiple_of(BATCH_SIZE)
                        }
                    };

                    let _ = cx.update(|cx| {
                        state.update(cx, |state, cx| {
                            match msg {
                                CollectionProgressMessage::Progress(count) => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.progress_count = count;
                                    }
                                }
                                CollectionProgressMessage::Completed(count) => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.is_running = false;
                                        tab.progress_count = count;
                                    }
                                    state.set_status_message(Some(StatusMessage::info(format!(
                                        "Exported {count} documents"
                                    ))));
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
}
