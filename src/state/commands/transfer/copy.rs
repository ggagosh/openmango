//! Copy transfer operations.

use std::collections::HashSet;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Entity};
use uuid::Uuid;

use crate::connection::get_connection_manager;
use crate::state::app_state::CollectionTransferStatus;
use crate::state::{AppCommands, AppEvent, AppState, StatusMessage, TransferScope};

use super::{
    CollectionProgressMessage, CopyConfig, PARALLEL_COLLECTION_LIMIT, TransferProgressMessage,
};

impl AppCommands {
    pub(super) fn execute_copy(
        state: Entity<AppState>,
        transfer_id: Uuid,
        config: CopyConfig,
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
                                        let progress_callback: crate::connection::ProgressCallback =
                                            std::sync::Arc::new(move |processed: u64| {
                                                let _ = progress_tx.unbounded_send(
                                                    TransferProgressMessage::CollectionProgress {
                                                        collection_name: progress_name.clone(),
                                                        status:
                                                            CollectionTransferStatus::InProgress,
                                                        documents_processed: processed,
                                                        documents_total: progress_total,
                                                    },
                                                );
                                            });

                                        let copy_options = crate::connection::CopyOptions {
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
        use crate::connection::{CopyOptions, ProgressCallback};

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
}
