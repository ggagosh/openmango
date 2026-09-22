//! Compare work runs on the connection runtime; only messages and small detail pairs reach GPUI.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use futures::{StreamExt, TryStreamExt};
use gpui_kit::{App, AppContext as _, Entity};
use mongodb::bson::{Document, RawDocumentBuf, doc};
use mongodb::options::Collation;
use uuid::Uuid;

use super::AppCommands;
use crate::connection::ops::compare::{
    CompareMessage, CompareOptions, DiffKind, DiffRow, Side, compare_collections_async,
};
use crate::error::{Error, ErrorReport};
use crate::state::compare::{CompareConfig, CompareDetail, CompareEndpoint, CompareMetadata};
use crate::state::{AppEvent, AppState};

impl AppCommands {
    pub fn run_compare(state: Entity<AppState>, id: Uuid, cx: &mut App) {
        let (config, runtime, clients) = {
            let app = state.read(cx);
            let Some(tab) = app.compare_tab(id) else {
                return;
            };
            if tab.running || tab.sync.running {
                return;
            }
            let config = tab.config.clone();
            if let Some(reason) = app.compare_disabled_reason(&config) {
                state.update(cx, |app, cx| {
                    if let Some(tab) = app.compare_tab_mut(id) {
                        tab.error = Some(reason);
                    }
                    cx.notify();
                });
                return;
            }
            let clients = config
                .sides
                .each_ref()
                .map(|side| app.active_connection_client(side.connection_id.unwrap()).unwrap());
            (config, app.connection_manager().runtime_handle(), clients)
        };
        let filter = if config.filter.trim().is_empty() {
            Document::new()
        } else {
            match crate::bson::parse_document_from_json(&config.filter) {
                Ok(filter) => filter,
                Err(_) => return,
            }
        };
        let (run, cancellation) = state.update(cx, |app, cx| {
            let identities = config.sides.each_ref().map(|side| {
                side.connection_id
                    .and_then(|id| app.connection_by_id(id))
                    .map(crate::models::ConnectionWriteIdentity::from)
            });
            let tab = app.compare_tab_mut(id).unwrap();
            let token = tab.begin();
            tab.connection_identities = identities;
            let run = tab.run;
            cx.emit(AppEvent::CompareChanged { compare_id: id });
            cx.notify();
            (run, token)
        });
        let slow_state = state.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(150)).await;
            cx.update(|cx| {
                slow_state.update(cx, |app, cx| {
                    if let Some(tab) =
                        app.compare_tab_mut(id).filter(|tab| tab.run == run && tab.running)
                    {
                        tab.slow = true;
                        cx.notify();
                    }
                })
            });
        })
        .detach();
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        let task = runtime.spawn(async move {
            let collections = [0, 1].map(|i| {
                clients[i]
                    .database(&config.sides[i].database)
                    .collection(&config.sides[i].collection)
            });
            compare_collections_async(
                collections[0].clone(),
                collections[1].clone(),
                CompareOptions {
                    fields: config.fields.clone(),
                    filter,
                    ignore: config.ignore_set(),
                    ..Default::default()
                },
                cancellation,
                sender,
            )
            .await
        });
        cx.spawn(async move |cx| {
            while let Some(message) = receiver.next().await {
                cx.update(|cx| {
                    state.update(cx, |app, cx| {
                        let Some(tab) = app.compare_tab_mut(id).filter(|tab| tab.run == run) else {
                            return;
                        };
                        let error = match &message {
                            CompareMessage::Failed(error) => Some(error.clone()),
                            _ => None,
                        };
                        tab.receive(message);
                        if let Some(error) = error {
                            app.report_compare_error(
                                id,
                                ErrorReport::from_message("Couldn't compare collections", &error),
                            );
                        }
                        cx.emit(AppEvent::CompareChanged { compare_id: id });
                        cx.notify();
                    })
                });
            }
            if let Err(error) = task.await {
                cx.update(|cx| {
                    state.update(cx, |app, cx| {
                        if let Some(tab) = app.compare_tab_mut(id).filter(|tab| tab.run == run) {
                            let error = format!("Comparison stopped: {error}");
                            tab.receive(CompareMessage::Failed(error.clone()));
                            app.report_compare_error(
                                id,
                                ErrorReport::from_message("Couldn't compare collections", &error),
                            );
                            cx.emit(AppEvent::CompareChanged { compare_id: id });
                            cx.notify();
                        }
                    })
                });
            }
        })
        .detach();
    }

    pub fn cancel_compare(state: &Entity<AppState>, id: Uuid, cx: &App) {
        if let Some(token) =
            state.read(cx).compare_tab(id).and_then(|tab| tab.cancellation.as_ref())
        {
            token.cancel();
        }
    }

    pub fn load_compare_metadata(state: Entity<AppState>, id: Uuid, side: usize, cx: &mut App) {
        let (endpoint, client, runtime, timeout) = {
            let app = state.read(cx);
            let Some(endpoint) = app
                .compare_tab(id)
                .map(|tab| tab.config.sides[side].clone())
                .filter(CompareEndpoint::complete)
            else {
                return;
            };
            let Some(client) = app.active_connection_client(endpoint.connection_id.unwrap()) else {
                return;
            };
            (
                endpoint,
                client,
                app.connection_manager().runtime_handle(),
                Duration::from_millis(app.settings.interactive_query_timeout_ms.max(100)),
            )
        };
        let requested = endpoint.clone();
        let task = runtime.spawn(async move {
            let database = client.database(&endpoint.database);
            let spec = database
                .list_collections()
                .filter(doc! {"name": &endpoint.collection})
                .await?
                .try_next()
                .await?
                .ok_or_else(|| Error::Parse("Collection no longer exists".into()))?;
            let mut metadata = CompareMetadata {
                endpoint: endpoint.clone(),
                supports_sync: crate::connection::ops::compare_sync::supports_sync(&client)
                    .await
                    .ok(),
                timeseries: spec.options.timeseries.is_some(),
                non_simple_collation: spec
                    .options
                    .collation
                    .as_ref()
                    .is_some_and(|c| c.locale != "simple"),
                ..Default::default()
            };
            if spec.collection_type == mongodb::results::CollectionType::Collection {
                metadata.indexes = crate::connection::ops::indexes::list_indexes_async(
                    &client,
                    &endpoint.database,
                    &endpoint.collection,
                    timeout,
                )
                .await?;
                // Stats are optional on restricted accounts; the comparison itself still works.
                if let Ok(stats) = crate::connection::ops::stats::collection_stats_async(
                    &client,
                    &endpoint.database,
                    &endpoint.collection,
                    timeout,
                )
                .await
                    && let Ok(storage) = stats.get_document("storageStats")
                {
                    let number = |name| {
                        storage
                            .get_i64(name)
                            .ok()
                            .or_else(|| storage.get_i32(name).ok().map(i64::from))
                            .and_then(|n| u64::try_from(n).ok())
                    };
                    metadata.count = number("count");
                    metadata.bytes = number("size");
                }
            }
            Ok::<_, Error>(metadata)
        });
        cx.spawn(async move |cx| {
            let result =
                task.await.map_err(|e| e.to_string()).and_then(|r| r.map_err(|e| e.to_string()));
            cx.update(|cx| {
                state.update(cx, |app, cx| {
                    let Some(tab) =
                        app.compare_tab_mut(id).filter(|tab| tab.config.sides[side] == requested)
                    else {
                        return;
                    };
                    tab.metadata[side] = Some(result.unwrap_or_else(|error| CompareMetadata {
                        endpoint: requested,
                        error: Some(error),
                        ..Default::default()
                    }));
                    cx.notify();
                })
            });
        })
        .detach();
    }

    pub fn select_compare_row(state: Entity<AppState>, id: Uuid, row_index: usize, cx: &mut App) {
        let Some((run, generation, config, row)) = state.update(cx, |app, cx| {
            let tab = app.compare_tab_mut(id)?;
            let row = tab.rows.get(row_index)?.clone();
            tab.selected = Some(row_index);
            tab.detail_generation = tab.detail_generation.wrapping_add(1);
            tab.detail_error = None;
            if let Some(detail) = tab.detail_cache.get(&row_index).cloned() {
                tab.detail = Some(detail);
                tab.detail_row = Some(row_index);
                tab.detail_loading = false;
                cx.notify();
                return None;
            }
            tab.detail_loading = true;
            tab.detail_slow = false;
            let result = (tab.run, tab.detail_generation, tab.results_config().clone(), row);
            cx.notify();
            Some(result)
        }) else {
            return;
        };
        let slow_state = state.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(150)).await;
            cx.update(|cx| {
                slow_state.update(cx, |app, cx| {
                    if let Some(tab) = app.compare_tab_mut(id).filter(|tab| {
                        tab.run == run && tab.detail_generation == generation && tab.detail_loading
                    }) {
                        tab.detail_slow = true;
                        cx.notify();
                    }
                })
            });
        })
        .detach();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(80)).await;
            let request = cx.update(|cx| {
                let app = state.read(cx);
                let tab = app.compare_tab(id)?;
                if tab.run != run || tab.detail_generation != generation {
                    return None;
                }
                let clients = config
                    .sides
                    .each_ref()
                    .map(|side| side.connection_id.and_then(|id| app.active_connection_client(id)));
                let [Some(left), Some(right)] = clients else {
                    return None;
                };
                Some((
                    app.connection_manager().runtime_handle(),
                    [left, right],
                    Duration::from_millis(app.settings.interactive_query_timeout_ms.max(100)),
                ))
            });
            let Some((runtime, clients, timeout)) = request else {
                cx.update(|cx| {
                    state.update(cx, |app, cx| {
                        if let Some(tab) = app
                            .compare_tab_mut(id)
                            .filter(|t| t.run == run && t.detail_generation == generation)
                        {
                            tab.detail_loading = false;
                            tab.detail_error =
                                Some("Reconnect both connections to fetch documents".into());
                            cx.notify();
                        }
                    })
                });
                return;
            };
            let task = runtime.spawn(async move {
                let fetch = |index: usize| {
                    let collection = clients[index]
                        .database(&config.sides[index].database)
                        .collection::<RawDocumentBuf>(&config.sides[index].collection);
                    let filter = row_filter(&config, &row, index);
                    async move {
                        let documents: Vec<RawDocumentBuf> = collection
                            .find(filter?)
                            .collation(Collation::builder().locale("simple").build())
                            .limit(if row.kind == DiffKind::MultipleMatches { 20 } else { 2 })
                            .max_time(timeout)
                            .await?
                            .try_collect()
                            .await?;
                        let expected_hash = if index == 0 { row.left_hash } else { row.right_hash };
                        let expected_count =
                            if index == 0 { row.left_count } else { row.right_count };
                        let hash = documents.first().map_or(0, |doc| {
                            let mut h = DefaultHasher::new();
                            doc.as_bytes().hash(&mut h);
                            h.finish()
                        });
                        let stale = row.kind != DiffKind::MultipleMatches
                            && (hash != expected_hash || documents.len() as u64 != expected_count);
                        let documents = documents
                            .into_iter()
                            .map(|d| {
                                Document::try_from(&*d).map_err(|e| Error::Parse(e.to_string()))
                            })
                            .collect::<crate::error::Result<Vec<_>>>()?;
                        Ok::<_, Error>((documents, stale))
                    }
                };
                let (left, right) = tokio::try_join!(fetch(0), fetch(1))?;
                Ok::<_, Error>(CompareDetail {
                    documents: [left.0, right.0],
                    changed_since_scan: left.1 || right.1,
                })
            });
            let result =
                task.await.map_err(|e| e.to_string()).and_then(|r| r.map_err(|e| e.to_string()));
            cx.update(|cx| {
                state.update(cx, |app, cx| {
                    let Some(tab) = app
                        .compare_tab_mut(id)
                        .filter(|t| t.run == run && t.detail_generation == generation)
                    else {
                        return;
                    };
                    tab.detail_loading = false;
                    match result {
                        Ok(detail) => {
                            let detail = Arc::new(detail);
                            if tab.detail_cache.len() >= 64 {
                                tab.detail_cache.clear();
                            }
                            // Large pairs remain inspectable, but are not retained in the browse cache.
                            let bytes: usize = detail
                                .documents
                                .iter()
                                .flatten()
                                .filter_map(|d| mongodb::bson::to_vec(d).ok())
                                .map(|d| d.len())
                                .sum();
                            if bytes <= 512 * 1024 {
                                tab.detail_cache.insert(row_index, detail.clone());
                            }
                            tab.detail = Some(detail);
                            tab.detail_row = Some(row_index);
                        }
                        Err(error) => tab.detail_error = Some(error),
                    }
                    cx.notify();
                })
            });
        })
        .detach();
    }
}

pub(crate) fn row_filter(
    config: &CompareConfig,
    row: &DiffRow,
    index: usize,
) -> crate::error::Result<Document> {
    if row.kind != DiffKind::MultipleMatches
        && let Some(id) =
            row.id_on(if index == 0 { Side::Left } else { Side::Right }, config.fields == ["_id"])
    {
        return Ok(doc! {"_id": {"$eq": id}});
    }
    let clauses: Vec<_> = config
        .fields
        .iter()
        .map(|field| {
            let value = if config.fields.len() == 1 {
                &row.key
            } else {
                row.key
                    .as_document()
                    .and_then(|d| d.get(field))
                    .unwrap_or(&mongodb::bson::Bson::Null)
            };
            doc! {field: {"$eq": value, "$exists": true}}
        })
        .collect();
    let filter = doc! {"$and": [scan_filter(config)?, doc! {"$and": clauses}]};
    Ok(crate::connection::ops::compare::key_filters(&filter, &config.fields).0)
}

/// The documents a run skipped for lacking a usable key. None when matching by _id.
pub(crate) fn skipped_filter(config: &CompareConfig) -> crate::error::Result<Option<Document>> {
    Ok(crate::connection::ops::compare::key_filters(&scan_filter(config)?, &config.fields).1)
}

fn scan_filter(config: &CompareConfig) -> crate::error::Result<Document> {
    if config.filter.trim().is_empty() {
        return Ok(Document::new());
    }
    crate::bson::parse_document_from_json(&config.filter).map_err(Error::Parse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::Bson;

    #[test]
    fn key_details_keep_scan_filter_and_skip_arrays_but_point_reads_show_current_document() {
        let config = CompareConfig {
            fields: vec!["sku".into()],
            filter: "{active:true}".into(),
            ..Default::default()
        };
        let mut row = DiffRow {
            key: Bson::String("A".into()),
            left_id: Some(Bson::Int32(1)),
            right_id: None,
            kind: DiffKind::OnlyLeft,
            changed: 0,
            paths: "".into(),
            left_hash: 1,
            right_hash: 0,
            left_count: 1,
            right_count: 0,
        };
        assert_eq!(row_filter(&config, &row, 0).unwrap(), doc! {"_id": {"$eq": 1}});
        let fallback = row_filter(&config, &row, 1).unwrap();
        let filter = doc! {"$and": [doc! {"active": true}, doc! {"$and": [doc! {"sku": {"$eq": "A", "$exists": true}}]}]};
        assert_eq!(
            fallback,
            crate::connection::ops::compare::key_filters(&filter, &config.fields).0
        );
        row.kind = DiffKind::MultipleMatches;
        assert_eq!(row_filter(&config, &row, 0).unwrap(), fallback);
    }

    #[test]
    fn skipped_links_open_exactly_the_documents_the_scan_skipped() {
        let mut config = CompareConfig {
            fields: vec!["sku".into()],
            filter: "{active:true}".into(),
            ..Default::default()
        };
        assert_eq!(
            skipped_filter(&config).unwrap(),
            crate::connection::ops::compare::key_filters(&doc! {"active": true}, &config.fields).1
        );
        config.fields = vec!["_id".into()];
        assert_eq!(skipped_filter(&config).unwrap(), None);
    }
}
