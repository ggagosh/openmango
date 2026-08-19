use std::sync::Arc;

use gpui::{App, AppContext as _, Entity};
use uuid::Uuid;

use crate::history::{BatchQuery, EligibilityReport, HistoryConnection, HistoryService, Usage};
use crate::state::{AppCommands, AppState, SessionKey, StatusMessage};

fn spawn_history_inspection(
    runtime: tokio::runtime::Handle,
    service: Arc<HistoryService>,
    connection: HistoryConnection,
    setup: bool,
) -> tokio::task::JoinHandle<(EligibilityReport, Option<Usage>, Option<String>)> {
    runtime.spawn(async move {
        let setup_error =
            if setup { service.setup_pre_post_images(&connection).await.err() } else { None };
        let report = HistoryService::eligibility(&connection).await;
        let usage = service.usage(Some(connection.id)).ok();
        (report, usage, setup_error)
    })
}

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
        if !state.read(cx).collection_history_available(
            session_key.connection_id,
            &session_key.database,
            &session_key.collection,
        ) {
            state.update(cx, |state, cx| {
                state.set_collection_subview(
                    &session_key,
                    crate::state::CollectionSubview::Documents,
                );
                if let Some(session) = state.session_mut(&session_key) {
                    session.data.history_loaded = false;
                    session.data.history_loading = false;
                }
                cx.notify();
            });
            return;
        }
        let Some(service) = state.read(cx).history_service() else {
            return;
        };
        let runtime = state.read(cx).connection_manager().runtime_handle();
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
        let query = BatchQuery {
            connection_id: session_key.connection_id,
            database: Some(session_key.database.clone()),
            collection: Some(session_key.collection.clone()),
            offset,
            limit: 50,
        };
        let service_for_task = service.clone();
        let session_for_task = session_key.clone();
        let task = runtime.spawn_blocking(move || {
            let page = service_for_task.list_batches(query);
            let gaps = service_for_task.list_collection_gaps(
                session_for_task.connection_id,
                &session_for_task.database,
                &session_for_task.collection,
            );
            page.and_then(|page| gaps.map(|gaps| (page, gaps)))
        });
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let result = match task.await {
                Ok(result) => result,
                Err(error) => Err(anyhow::anyhow!("History query task failed: {error}")),
            };
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
                        Ok((page, gaps)) => {
                            if append {
                                session.data.history.extend(page.items);
                            } else {
                                session.data.history = page.items;
                                session.data.history_gaps = gaps;
                            }
                            session.data.history_loaded = true;
                            session.data.history_total = page.total;
                            session.data.history_next_offset = page.next_offset;
                            session.data.history_error = None;
                        }
                        Err(error) => {
                            session.data.history_error =
                                Some(format!("Collection History could not be loaded: {error}"));
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    pub fn toggle_history_batch_details(
        state: Entity<AppState>,
        session_key: SessionKey,
        batch_id: Uuid,
        cx: &mut App,
    ) {
        let hidden = state.update(cx, |state, cx| {
            let Some(session) = state.session_mut(&session_key) else {
                return false;
            };
            let hidden = session.data.history_details.remove(&batch_id).is_some();
            if hidden {
                cx.notify();
            }
            hidden
        });
        if hidden {
            return;
        }
        let Some(service) = state.read(cx).history_service() else {
            return;
        };
        let runtime = state.read(cx).connection_manager().runtime_handle();
        let should_load = state.update(cx, |state, cx| {
            let Some(session) = state.session_mut(&session_key) else {
                return false;
            };
            let inserted = session.data.history_detail_loading.insert(batch_id);
            if inserted {
                cx.notify();
            }
            inserted
        });
        if !should_load {
            return;
        }
        let task = runtime.spawn_blocking(move || service.get_batch(batch_id, 0, 3));
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let result = match task.await {
                Ok(result) => result,
                Err(error) => Err(anyhow::anyhow!("History details task failed: {error}")),
            };
            let _ = cx.update(|cx| {
                state.update(cx, |state, cx| {
                    let Some(session) = state.session_mut(&session_key) else {
                        return;
                    };
                    session.data.history_detail_loading.remove(&batch_id);
                    match result {
                        Ok(details) => {
                            session.data.history_details.insert(batch_id, details);
                        }
                        Err(error) => {
                            session.data.history_error =
                                Some(format!("History details could not be loaded: {error}"));
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    pub fn inspect_history_eligibility(
        state: Entity<AppState>,
        connection_id: Uuid,
        setup: bool,
        enable_after_setup: bool,
        cx: &mut App,
    ) {
        let Some((service, connection, runtime)) = state.update(cx, |state, cx| {
            let service = state.history_service()?;
            let configuration = state.connection_by_id(connection_id)?.clone();
            let active = state.active_connection_by_id(connection_id)?.clone();
            if !state.begin_history_inspection(connection_id) {
                return None;
            }
            cx.notify();
            Some((
                service,
                crate::history::HistoryConnection {
                    id: connection_id,
                    name: configuration.name,
                    client: active.client,
                    databases: active.databases,
                    max_age_days: configuration.history_max_age_days,
                    max_bytes: configuration.history_max_bytes,
                },
                state.connection_manager().runtime_handle(),
            ))
        }) else {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "Connect this connection before inspecting History eligibility.",
                )));
                cx.notify();
            });
            return;
        };
        let task = spawn_history_inspection(runtime, service.clone(), connection, setup);
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let (report, usage, setup_error) = match task.await {
                Ok(result) => result,
                Err(error) => (
                    EligibilityReport {
                        status: crate::history::EligibilityStatus::Unavailable,
                        version: None,
                        topology: None,
                        storage_engine: None,
                        failures: vec![format!("History inspection task failed: {error}")],
                        collections: Vec::new(),
                    },
                    None,
                    None,
                ),
            };
            let eligible = report.status == crate::history::EligibilityStatus::Eligible;
            let _ = cx.update(|cx| {
                state.update(cx, |state, cx| {
                    let message = if let Some(error) = setup_error {
                        StatusMessage::error(format!("History setup failed: {error}"))
                    } else if eligible {
                        StatusMessage::info("History is eligible on this connection.")
                    } else {
                        StatusMessage::error(
                            report
                                .exact_reason()
                                .unwrap_or("Enable pre/post images for covered collections."),
                        )
                    };
                    state.finish_history_inspection(connection_id, report, usage);
                    state.set_status_message(Some(message));
                    if eligible && enable_after_setup {
                        state.set_connection_history_enabled(connection_id, true, cx);
                    } else if eligible
                        && state.connection_history_enabled(connection_id)
                        && let (Some(service), Some(active), Some(configuration)) = (
                            state.history_service(),
                            state.active_connection_by_id(connection_id),
                            state.connection_by_id(connection_id),
                        )
                    {
                        service.start(crate::history::HistoryConnection {
                            id: connection_id,
                            name: configuration.name.clone(),
                            client: active.client.clone(),
                            databases: active.databases.clone(),
                            max_age_days: configuration.history_max_age_days,
                            max_bytes: configuration.history_max_bytes,
                        });
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    pub fn delete_history_batch(
        state: Entity<AppState>,
        session_key: SessionKey,
        batch_id: Uuid,
        cx: &mut App,
    ) {
        let result = state
            .read(cx)
            .history_service()
            .ok_or_else(|| "History is unavailable".to_string())
            .and_then(|service| service.delete_batch(batch_id).map_err(|error| error.to_string()));
        state.update(cx, |state, cx| {
            state.refresh_history_usage(session_key.connection_id);
            state.set_status_message(Some(match result {
                Ok(true) => StatusMessage::info("History batch deleted."),
                Ok(false) => StatusMessage::error("Active restore work cannot be deleted."),
                Err(error) => StatusMessage::error(error),
            }));
            cx.notify();
        });
        Self::load_collection_history(state, session_key, cx);
    }

    pub fn clear_collection_history(
        state: Entity<AppState>,
        session_key: SessionKey,
        cx: &mut App,
    ) {
        let result = state
            .read(cx)
            .history_service()
            .ok_or_else(|| "History is unavailable".to_string())
            .and_then(|service| {
                service
                    .clear_collection(
                        session_key.connection_id,
                        &session_key.database,
                        &session_key.collection,
                    )
                    .map_err(|error| error.to_string())
            });
        state.update(cx, |state, cx| {
            state.refresh_history_usage(session_key.connection_id);
            state.set_status_message(Some(match result {
                Ok(count) => StatusMessage::info(format!("Cleared {count} History batches.")),
                Err(error) => StatusMessage::error(error),
            }));
            cx.notify();
        });
        Self::load_collection_history(state, session_key, cx);
    }

    pub fn clear_connection_history(state: Entity<AppState>, connection_id: Uuid, cx: &mut App) {
        let result = state
            .read(cx)
            .history_service()
            .ok_or_else(|| "History is unavailable".to_string())
            .and_then(|service| {
                service.clear_connection(connection_id).map_err(|error| error.to_string())
            });
        state.update(cx, |state, cx| {
            state.refresh_history_usage(connection_id);
            state.set_status_message(Some(match result {
                Ok(count) => StatusMessage::info(format!("Cleared {count} History batches.")),
                Err(error) => StatusMessage::error(error),
            }));
            cx.notify();
        });
    }

    pub fn clear_all_history(state: Entity<AppState>, cx: &mut App) {
        let result = state
            .read(cx)
            .history_service()
            .ok_or_else(|| "History is unavailable".to_string())
            .and_then(|service| service.clear_all().map_err(|error| error.to_string()));
        state.update(cx, |state, cx| {
            let ids = state.connections.iter().map(|connection| connection.id).collect::<Vec<_>>();
            for id in ids {
                state.refresh_history_usage(id);
            }
            state.set_status_message(Some(match result {
                Ok(count) => StatusMessage::info(format!("Cleared {count} History batches.")),
                Err(error) => StatusMessage::error(error),
            }));
            cx.notify();
        });
    }

    pub fn cancel_history_restore(state: Entity<AppState>, batch_id: Uuid, cx: &mut App) {
        if let Some(service) = state.read(cx).history_service() {
            service.cancel_restore(batch_id);
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::info(
                    "History restore cancellation requested.",
                )));
                cx.notify();
            });
        }
    }

    pub fn revert_operation(
        state: Entity<AppState>,
        batch_id: Uuid,
        connection_id: Uuid,
        database: String,
        collection: String,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(connection_id), cx) {
            return;
        }
        let Some(service) = state.read(cx).history_service() else {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error("History is unavailable.")));
                cx.notify();
            });
            return;
        };
        let result = service.revert_batch(batch_id);
        state.update(cx, |state, cx| {
            state.set_status_message(Some(match result {
                Ok(()) => StatusMessage::info("History restore started."),
                Err(error) => StatusMessage::error(error),
            }));
            cx.notify();
        });
        let state_for_poll = state.clone();
        let session_key = SessionKey::new(connection_id, database, collection);
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            loop {
                gpui::Timer::after(std::time::Duration::from_millis(250)).await;
                let progress = service.restore_progress(batch_id);
                let done = progress.as_ref().is_ok_and(|progress| progress.done);
                let _ = cx.update(|cx| {
                    state_for_poll.update(cx, |state, cx| {
                        if let Ok(progress) = &progress {
                            state.set_status_message(Some(StatusMessage::info(format!(
                                "History restore: {} of {} processed ({} restored, {} skipped, {} conflicts, {} failed)",
                                progress.processed,
                                progress.total,
                                progress.restored,
                                progress.skipped,
                                progress.conflicted,
                                progress.failed
                            ))));
                        }
                        cx.notify();
                    });
                    AppCommands::load_collection_history(
                        state_for_poll.clone(),
                        session_key.clone(),
                        cx,
                    );
                    if done {
                        state_for_poll.update(cx, |state, _| {
                            state.refresh_history_usage(connection_id);
                        });
                        AppCommands::load_documents_for_session(
                            state_for_poll.clone(),
                            session_key.clone(),
                            cx,
                        );
                    }
                });
                if done || progress.is_err() {
                    break;
                }
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_inspection_runs_on_the_mongodb_runtime() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let client = runtime.block_on(async {
            mongodb::Client::with_uri_str(
                "mongodb://127.0.0.1:1/?directConnection=true&serverSelectionTimeoutMS=10",
            )
            .await
            .unwrap()
        });
        let directory = tempfile::tempdir().unwrap();
        let service = Arc::new(
            HistoryService::open(
                directory.path().join("history.sqlite3"),
                [31; 32],
                runtime.handle().clone(),
            )
            .unwrap(),
        );
        let task = spawn_history_inspection(
            runtime.handle().clone(),
            service,
            HistoryConnection {
                id: Uuid::new_v4(),
                name: "Unavailable test server".into(),
                client,
                databases: Vec::new(),
                max_age_days: 30,
                max_bytes: 1024,
            },
            false,
        );

        let (report, _, _) = runtime.block_on(task).unwrap();
        assert_eq!(report.status, crate::history::EligibilityStatus::Unavailable);
    }
}
