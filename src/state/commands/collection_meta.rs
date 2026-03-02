//! Background prefetch of sibling collection schemas.

use gpui::{App, AppContext as _, Entity};

use crate::state::{AppState, SessionKey};

use super::{AppCommands, SCHEMA_SAMPLE_SIZE, build_schema_analysis};

const MAX_CONCURRENT_PREFETCH: usize = 3;

impl AppCommands {
    /// Prefetch schema metadata for sibling collections in the current database.
    ///
    /// Runs after `load_database_overview` completes so that the collection list
    /// is available. At most `MAX_CONCURRENT_PREFETCH` fetches run concurrently;
    /// remaining collections are picked up on the next trigger.
    pub fn prefetch_sibling_schemas(state: Entity<AppState>, cx: &mut App) {
        let (conn_id, database, siblings, client, manager) = {
            let s = state.read(cx);
            let conn_id = match s.selected_connection_id() {
                Some(id) => id,
                None => return,
            };
            let database = match s.selected_database_name() {
                Some(db) => db,
                None => return,
            };
            let active = match s.active_connection_by_id(conn_id) {
                Some(c) => c,
                None => return,
            };
            let client = active.client.clone();
            let manager = s.connection_manager();

            let col_names: Vec<String> =
                active.collections.get(&database).cloned().unwrap_or_default();

            let current_col = s.selected_collection_name().unwrap_or_default();

            let siblings: Vec<String> = col_names
                .into_iter()
                .filter(|name| name != &current_col && !name.starts_with("system."))
                .filter(|name| {
                    let key = SessionKey::new(conn_id, &database, name);
                    s.collection_meta_stale(&key) && !s.is_collection_meta_inflight(&key)
                })
                .take(MAX_CONCURRENT_PREFETCH)
                .collect();

            (conn_id, database, siblings, client, manager)
        };

        if siblings.is_empty() {
            return;
        }

        for collection in siblings {
            let key = SessionKey::new(conn_id, &database, &collection);

            state.update(cx, |s, _| {
                s.mark_collection_meta_inflight(&key);
            });

            let task = cx.background_spawn({
                let manager = manager.clone();
                let client = client.clone();
                let database = database.clone();
                let collection = collection.clone();
                async move {
                    let (docs, total) = manager.sample_for_schema(
                        &client,
                        &database,
                        &collection,
                        SCHEMA_SAMPLE_SIZE,
                    )?;
                    Ok::<_, crate::error::Error>(build_schema_analysis(&docs, total))
                }
            });

            cx.spawn({
                let state = state.clone();
                let key = key.clone();
                async move |cx: &mut gpui::AsyncApp| {
                    let result = task.await;
                    let _ = cx.update(|cx| {
                        state.update(cx, |s, cx| {
                            s.clear_collection_meta_inflight(&key);
                            if let Ok(schema) = result {
                                s.set_collection_meta(key, schema);
                                cx.notify();
                            }
                        });
                    });
                }
            })
            .detach();
        }
    }
}
