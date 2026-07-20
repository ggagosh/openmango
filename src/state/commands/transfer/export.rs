//! Export transfer operations.

use std::collections::HashSet;
use std::path::PathBuf;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Entity};
use uuid::Uuid;

use crate::connection::{
    BsonOutputFormat, BsonToolProgress, BsonToolRunOutcome, ExportQueryOptions, ExtendedJsonMode,
    JsonExportOptions,
};
use crate::state::app_state::CollectionTransferStatus;
use crate::state::{
    AppCommands, AppEvent, AppState, StatusMessage, TransferFormat, parse_export_query_document,
};

use super::{
    CollectionProgressMessage, ExportConfig, TransferProgressMessage,
    transfer_message_matches_generation,
};

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

        // BSON tools must reuse the active SSH/SOCKS transport rather than the saved URI.
        let connection_uri = if matches!(config.format, TransferFormat::Bson) {
            match state.read(cx).active_connection_tool_uri(connection_id) {
                Ok(uri) => Some(uri),
                Err(error) => {
                    state.update(cx, |state, cx| {
                        state.set_status_message(Some(StatusMessage::error(error.to_string())));
                        if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                            tab.runtime.is_running = false;
                            tab.runtime.error_message = Some(error.to_string());
                        }
                        cx.emit(AppEvent::TransferFailed { transfer_id, error: error.to_string() });
                        cx.notify();
                    });
                    return;
                }
            }
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

        // `execute_transfer_with_confirmation` resolves templates once and freezes the exact path.
        let path = PathBuf::from(&config.file_path);
        let format = config.format;
        let scope = config.scope;
        let json_mode = config.json_mode;
        let pretty_print = config.pretty_print;
        let bson_output = config.bson_output;
        let gzip = matches!(config.compression, crate::state::CompressionMode::Gzip);

        // Export query options (only for collection scope) - already cloned
        let export_filter = config.export_filter;
        let export_projection = config.export_projection;
        let export_sort = config.export_sort;

        let exclude_collections = config.exclude_collections;

        let cancellation_token = crate::connection::types::CancellationToken::new();

        let operation_generation = state.update(cx, |state, cx| {
            let mut operation_generation = 0;
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                operation_generation = tab
                    .runtime
                    .transfer_generation
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                tab.runtime.is_running = true;
                tab.runtime.progress_count = 0;
                tab.runtime.error_message = None;
                tab.runtime.database_progress = None; // Reset on new export
                tab.runtime.cancellation_token = Some(cancellation_token.clone());
            }
            state.set_status_message(Some(StatusMessage::info("Exporting...")));
            cx.emit(AppEvent::TransferStarted { transfer_id });
            cx.notify();
            operation_generation
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
                cancellation_token.clone(),
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
                cancellation_token.clone(),
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
                cancellation_token,
                operation_generation,
                cx,
            );
            return;
        }

        // Fallback for unexpected cases
        state.update(cx, |state, cx| {
            if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                tab.runtime.is_running = false;
                tab.runtime.error_message = Some("Unexpected export configuration".to_string());
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
        cancellation_token: crate::connection::types::CancellationToken,
        cx: &mut App,
    ) {
        // Create channel for progress updates from background thread
        let (tx, rx) = mpsc::unbounded::<TransferProgressMessage>();

        let manager = state.read(cx).connection_manager();

        // Spawn background task that does all blocking I/O
        cx.background_spawn({
            let exclude_set: HashSet<String> = exclude_collections.iter().cloned().collect();
            async move {
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

                // Stage the complete database export beside the destination. The existing
                // destination is promoted only after every collection succeeds.
                let parent = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| std::path::Path::new("."));
                let staging = match tempfile::Builder::new()
                    .prefix(".openmango-database-export-")
                    .tempdir_in(parent)
                {
                    Ok(staging) => staging,
                    Err(error) => {
                        let _ = tx.unbounded_send(TransferProgressMessage::Failed {
                            error: error.to_string(),
                        });
                        return;
                    }
                };
                let staging_path = staging.path().join("export");
                if let Err(error) = std::fs::create_dir(&staging_path) {
                    let _ = tx.unbounded_send(TransferProgressMessage::Failed {
                        error: error.to_string(),
                    });
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
                            let path = staging_path.clone();
                            let handle = runtime_handle.clone();
                            let manager = manager.clone();
                            let cancellation_token = cancellation_token.clone();

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
                                                    cancellation: Some(cancellation_token.clone()),
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
                                                manager.export_collection_csv_with_query(
                                                    &client,
                                                    &database,
                                                    &coll_name_for_task,
                                                    &file_path,
                                                    gzip,
                                                    crate::connection::ExportQueryOptions::default(),
                                                    Some(cancellation_token),
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
                for (collection, result) in &results {
                    match result {
                        Ok(count) => total_count += count,
                        Err(e) => {
                            log::error!("Export failed for collection {collection}: {e}");
                            had_error = true;
                        }
                    }
                }

                if cancellation_token.is_cancelled() {
                    had_error = true;
                }
                if !had_error
                    && let Err(error) = crate::connection::ops::export::promote_export_directory(
                        &staging_path,
                        &path,
                    )
                {
                    let _ = tx.unbounded_send(TransferProgressMessage::Failed {
                        error: format!("Could not finalize database export: {error}"),
                    });
                    return;
                }

                // Dropping `staging` removes every partial file after failure/cancellation.
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
                        | TransferProgressMessage::Cancelled { .. }
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
                                    let failure_summary = state
                                        .transfer_tab(transfer_id)
                                        .and_then(|tab| tab.runtime.database_progress.as_ref())
                                        .and_then(|progress| progress.failure_summary());
                                    let failed_count = state
                                        .transfer_tab(transfer_id)
                                        .and_then(|tab| tab.runtime.database_progress.as_ref())
                                        .map_or(0, |progress| progress.failed_count());
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.runtime.is_running = false;
                                        tab.runtime.progress_count = total_count;
                                        tab.runtime.error_message = failure_summary;
                                    }
                                    if had_error {
                                        state.set_status_message(Some(StatusMessage::error(
                                            format!(
                                                "Export completed with errors: {failed_count} collection(s) failed; {total_count} documents processed"
                                            ),
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
                                TransferProgressMessage::Cancelled { .. } => {}
                                TransferProgressMessage::Failed { error } => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.runtime.is_running = false;
                                        tab.runtime.error_message = Some(error.clone());
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
        output_format: BsonOutputFormat,
        gzip: bool,
        exclude_collections: Vec<String>,
        cancellation_token: crate::connection::types::CancellationToken,
        operation_generation: u64,
        cx: &mut App,
    ) {
        let (tx, rx) = mpsc::unbounded::<TransferProgressMessage>();

        let manager = state.read(cx).connection_manager();

        // Spawn background task that runs mongodump with progress parsing
        cx.background_spawn({
            async move {
                // We don't know collection list upfront for BSON, but we'll discover them
                // Send a placeholder started message
                let _ = tx.unbounded_send(TransferProgressMessage::Started {
                    collections: vec![], // Will be discovered during export
                });

                let final_path = match output_format {
                    BsonOutputFormat::Archive
                        if path.extension().is_none_or(|extension| extension != "archive") =>
                    {
                        path.with_extension("archive")
                    }
                    _ => path.clone(),
                };
                let parent = final_path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| std::path::Path::new("."));
                let staging = match tempfile::Builder::new()
                    .prefix(".openmango-bson-export-")
                    .tempdir_in(parent)
                {
                    Ok(staging) => staging,
                    Err(error) => {
                        let _ = tx.unbounded_send(TransferProgressMessage::Failed {
                            error: error.to_string(),
                        });
                        return;
                    }
                };
                let staged_path = staging.path().join(match output_format {
                    BsonOutputFormat::Archive => "export.archive",
                    BsonOutputFormat::Folder => "export",
                });

                let progress_tx = tx.clone();
                let cancellation_for_tool = cancellation_token.clone();
                let result = manager.export_database_bson_with_progress(
                    &connection_uri,
                    &database,
                    output_format,
                    &staged_path,
                    gzip,
                    &exclude_collections,
                    cancellation_for_tool,
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
                    Ok(BsonToolRunOutcome::Completed) => {
                        if cancellation_token.is_cancelled() {
                            let _ = tx.unbounded_send(TransferProgressMessage::Cancelled {
                                termination_succeeded: true,
                            });
                            return;
                        }
                        if let Err(error) = crate::connection::ops::export::promote_export_path(
                            &staged_path,
                            &final_path,
                        ) {
                            let _ = tx.unbounded_send(TransferProgressMessage::Failed {
                                error: format!("Could not finalize BSON export: {error}"),
                            });
                            return;
                        }
                        let _ = tx.unbounded_send(TransferProgressMessage::Completed {
                            total_count: 0, // mongodump doesn't provide total count
                            had_error: false,
                        });
                    }
                    Ok(BsonToolRunOutcome::Cancelled { termination_succeeded }) => {
                        let _ = tx.unbounded_send(TransferProgressMessage::Cancelled {
                            termination_succeeded,
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
                        | TransferProgressMessage::Cancelled { .. }
                        | TransferProgressMessage::Failed { .. } => true,
                        TransferProgressMessage::CollectionProgress { .. } => {
                            progress_count += 1;
                            progress_count.is_multiple_of(BATCH_SIZE)
                        }
                    };

                    let _ = cx.update(|cx| {
                        state.update(cx, |state, cx| {
                            let Some(tab) = state.transfer_tab(transfer_id) else {
                                return;
                            };
                            let current_generation = tab
                                .runtime
                                .transfer_generation
                                .load(std::sync::atomic::Ordering::SeqCst);
                            let cancellation_result =
                                matches!(&msg, TransferProgressMessage::Cancelled { .. });
                            if !transfer_message_matches_generation(
                                current_generation,
                                operation_generation,
                                cancellation_result,
                            ) {
                                return;
                            }
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
                                        tab.runtime.cancellation_token = None;
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
                                TransferProgressMessage::Cancelled { termination_succeeded } => {
                                    let message = if termination_succeeded {
                                        "BSON export cancelled; mongodump terminated successfully"
                                    } else {
                                        "BSON export cancellation requested, but mongodump termination could not be confirmed"
                                    };
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.runtime.is_running = false;
                                        tab.runtime.cancellation_token = None;
                                        tab.runtime.error_message = Some(message.to_string());
                                    }
                                    state.set_status_message(Some(if termination_succeeded {
                                        StatusMessage::info(message)
                                    } else {
                                        StatusMessage::error(message)
                                    }));
                                }
                                TransferProgressMessage::Failed { error } => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.runtime.is_running = false;
                                        tab.runtime.cancellation_token = None;
                                        tab.runtime.error_message = Some(error.clone());
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
        cancellation_token: crate::connection::types::CancellationToken,
        cx: &mut App,
    ) {
        // Create channel for progress updates from background thread
        let (tx, rx) = mpsc::unbounded::<CollectionProgressMessage>();

        // Parse query options without ever broadening an invalid query to `None`.
        let parsed = [
            ("Filter", export_filter.as_str()),
            ("Projection", export_projection.as_str()),
            ("Sort", export_sort.as_str()),
        ]
        .map(|(label, value)| {
            parse_export_query_document(value).map_err(|error| format!("{label}: {error}"))
        });
        let [filter, projection, sort] = parsed;
        let (filter, projection, sort) = match (filter, projection, sort) {
            (Ok(filter), Ok(projection), Ok(sort)) => (filter, projection, sort),
            (filter, projection, sort) => {
                let error = filter
                    .err()
                    .or_else(|| projection.err())
                    .or_else(|| sort.err())
                    .unwrap_or_else(|| "Invalid export query options".to_string());
                state.update(cx, |state, cx| {
                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                        tab.runtime.is_running = false;
                        tab.runtime.error_message = Some(error.clone());
                    }
                    state.set_status_message(Some(StatusMessage::error(error.clone())));
                    cx.emit(AppEvent::TransferFailed { transfer_id, error });
                    cx.notify();
                });
                return;
            }
        };

        let query_options = if filter.is_some() || projection.is_some() || sort.is_some() {
            Some(ExportQueryOptions { filter, projection, sort })
        } else {
            None
        };

        let manager = state.read(cx).connection_manager();

        // Spawn background task that does all blocking I/O
        cx.background_spawn({
            async move {
                let runtime_handle = manager.runtime_handle();

                let result = runtime_handle
                    .spawn_blocking(move || {
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
                                    cancellation: Some(cancellation_token.clone()),
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
                                        Some(cancellation_token),
                                    )
                                } else {
                                    manager.export_collection_csv_with_query(
                                        &client,
                                        &database,
                                        &collection,
                                        &path,
                                        gzip,
                                        crate::connection::ExportQueryOptions::default(),
                                        Some(cancellation_token),
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
                        let _ = tx.unbounded_send(CollectionProgressMessage::Failed {
                            error: e.to_string(),
                            processed: 0,
                        });
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
                        | CollectionProgressMessage::Failed { .. } => true,
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
                                        tab.runtime.progress_count = count;
                                    }
                                }
                                CollectionProgressMessage::Completed(count) => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.runtime.is_running = false;
                                        tab.runtime.progress_count = count;
                                    }
                                    state.set_status_message(Some(StatusMessage::info(format!(
                                        "Exported {count} documents"
                                    ))));
                                    cx.emit(AppEvent::TransferCompleted { transfer_id, count });
                                }
                                CollectionProgressMessage::Failed { error, processed } => {
                                    if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                                        tab.runtime.is_running = false;
                                        tab.runtime.progress_count =
                                            tab.runtime.progress_count.max(processed);
                                        tab.runtime.error_message = Some(error.clone());
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
