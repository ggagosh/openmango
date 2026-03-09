use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{App, AppContext as _, Entity};

use crate::components::file_picker::{FilePickerMode, open_file_dialog_async};
use crate::connection::types::{
    CancellationToken, ExportQueryOptions, ExtendedJsonMode, JsonExportOptions, JsonTransferFormat,
};
use crate::state::{AppCommands, AppState, SessionKey, StatusMessage};
use crate::views::documents::export::FileExportFormat;

impl AppCommands {
    pub fn save_as_file(
        state: Entity<AppState>,
        session_key: SessionKey,
        format: FileExportFormat,
        cx: &mut App,
    ) {
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };

        let (filter, sort, projection, column_widths, column_order, manager) = {
            let st = state.read(cx);
            let (filter, sort, projection) = match st.session(&session_key) {
                Some(session) => (
                    session.data.filter.clone(),
                    session.data.sort.clone(),
                    session.data.projection.clone(),
                ),
                None => (None, None, None),
            };
            let widths = st.table_column_widths(&session_key);
            let order = st.table_column_order(&session_key);
            let mgr = st.connection_manager();
            (filter, sort, projection, widths, order, mgr)
        };

        let collection = session_key.collection.clone();
        let database = session_key.database.clone();
        let filters = format.file_filters();
        let now = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let default_name = format!("{}_{}.{}", collection, now, format.extension());

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let path =
                    open_file_dialog_async(FilePickerMode::Save, filters, Some(default_name)).await;

                let Some(path) = path else {
                    return;
                };

                Self::run_export(
                    state,
                    client,
                    manager,
                    database,
                    collection,
                    path,
                    format,
                    filter,
                    sort,
                    projection,
                    column_widths,
                    column_order,
                    cx,
                );
            }
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn run_export(
        state: Entity<AppState>,
        client: mongodb::Client,
        manager: std::sync::Arc<crate::connection::ConnectionManager>,
        database: String,
        collection: String,
        path: PathBuf,
        format: FileExportFormat,
        filter: Option<mongodb::bson::Document>,
        sort: Option<mongodb::bson::Document>,
        projection: Option<mongodb::bson::Document>,
        column_widths: HashMap<String, f32>,
        column_order: Vec<String>,
        cx: &mut gpui::AsyncApp,
    ) {
        let cancellation = CancellationToken::new();
        let cancellation_for_task = cancellation.clone();

        let _ = cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.set_export_progress(Some(ExportProgress {
                    count: 0,
                    format,
                    cancellation: cancellation.clone(),
                }));
                state.set_status_message(Some(StatusMessage::info("Exporting...")));
                cx.notify();
            });
        });

        let query = ExportQueryOptions { filter, projection, sort };
        let (tx, rx) = futures::channel::mpsc::unbounded::<u64>();

        let state_for_task = state.clone();
        let _ = cx.update(|cx| {
            let task = cx.background_spawn({
                let database = database.clone();
                let collection = collection.clone();
                let path = path.clone();
                async move {
                    match format {
                        FileExportFormat::JsonArray => {
                            let options = JsonExportOptions {
                                format: JsonTransferFormat::JsonArray,
                                json_mode: ExtendedJsonMode::Relaxed,
                                pretty_print: true,
                                gzip: false,
                                cancellation: Some(cancellation_for_task),
                            };
                            manager.export_collection_json_with_query_and_progress(
                                &client,
                                &database,
                                &collection,
                                &path,
                                options,
                                query,
                                move |count| {
                                    let _ = tx.unbounded_send(count);
                                },
                            )
                        }
                        FileExportFormat::JsonLines => {
                            let options = JsonExportOptions {
                                format: JsonTransferFormat::JsonLines,
                                json_mode: ExtendedJsonMode::Relaxed,
                                pretty_print: false,
                                gzip: false,
                                cancellation: Some(cancellation_for_task),
                            };
                            manager.export_collection_json_with_query_and_progress(
                                &client,
                                &database,
                                &collection,
                                &path,
                                options,
                                query,
                                move |count| {
                                    let _ = tx.unbounded_send(count);
                                },
                            )
                        }
                        FileExportFormat::Csv => manager
                            .export_collection_csv_with_query_and_progress(
                                &client,
                                &database,
                                &collection,
                                &path,
                                false,
                                query,
                                move |count| {
                                    let _ = tx.unbounded_send(count);
                                },
                            ),
                        FileExportFormat::Excel => manager.export_collection_excel_with_query(
                            &client,
                            &database,
                            &collection,
                            &path,
                            query,
                            column_widths,
                            column_order,
                            Some(cancellation_for_task),
                            move |count| {
                                let _ = tx.unbounded_send(count);
                            },
                        ),
                    }
                }
            });

            cx.spawn({
                let state = state_for_task.clone();
                async move |cx: &mut gpui::AsyncApp| {
                    use futures::StreamExt;
                    let mut rx = rx;
                    let progress_task = cx.spawn({
                        let state = state.clone();
                        async move |cx: &mut gpui::AsyncApp| {
                            while let Some(count) = rx.next().await {
                                let _ = cx.update(|cx| {
                                    state.update(cx, |state, cx| {
                                        if let Some(progress) = state.export_progress_mut() {
                                            progress.count = count;
                                        }
                                        cx.notify();
                                    });
                                });
                            }
                        }
                    });

                    let result = task.await;
                    progress_task.detach();

                    let _ = cx.update(|cx| {
                        state.update(cx, |state, cx| {
                            state.set_export_progress(None);
                            match result {
                                Ok(count) => {
                                    state.set_status_message(Some(StatusMessage::info(format!(
                                        "Exported {} documents",
                                        count
                                    ))));
                                }
                                Err(e) => {
                                    let msg = e.to_string();
                                    if !msg.contains("cancelled") {
                                        state.set_status_message(Some(StatusMessage::error(
                                            format!("Export failed: {}", msg),
                                        )));
                                    } else {
                                        state.set_status_message(Some(StatusMessage::info(
                                            "Export cancelled",
                                        )));
                                    }
                                }
                            }
                            cx.notify();
                        });
                    });
                }
            })
            .detach();
        });
    }

    pub fn save_aggregation_as(
        state: Entity<AppState>,
        session_key: SessionKey,
        format: FileExportFormat,
        cx: &mut App,
    ) {
        let (documents, column_widths, column_order) = {
            let st = state.read(cx);
            let Some(session) = st.session(&session_key) else {
                return;
            };
            let Some(results) = session.data.aggregation.results.as_ref() else {
                return;
            };
            if results.is_empty() {
                return;
            }
            let widths = session.view.agg_table_column_widths.clone();
            let order = session.view.agg_table_column_order.clone();
            (results.clone(), widths, order)
        };

        let collection = session_key.collection.clone();
        let filters = format.file_filters();
        let now = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let default_name = format!("{}_{}.{}", collection, now, format.extension());

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let path =
                    open_file_dialog_async(FilePickerMode::Save, filters, Some(default_name)).await;

                let Some(path) = path else {
                    return;
                };

                let _ = cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        state.set_status_message(Some(StatusMessage::info("Exporting...")));
                        cx.notify();
                    });

                    let task = cx.background_spawn({
                        let path = path.clone();
                        let documents = documents.clone();
                        let column_widths = column_widths.clone();
                        let column_order = column_order.clone();
                        async move {
                            write_documents_to_file(
                                &documents,
                                &path,
                                format,
                                column_widths,
                                column_order,
                            )
                        }
                    });

                    cx.spawn({
                        let state = state.clone();
                        async move |cx: &mut gpui::AsyncApp| {
                            let result = task.await;
                            let _ = cx.update(|cx| {
                                state.update(cx, |state, cx| {
                                    match result {
                                        Ok(count) => {
                                            state.set_status_message(Some(StatusMessage::info(
                                                format!("Exported {} documents", count),
                                            )));
                                        }
                                        Err(e) => {
                                            state.set_status_message(Some(StatusMessage::error(
                                                format!("Export failed: {}", e),
                                            )));
                                        }
                                    }
                                    cx.notify();
                                });
                            });
                        }
                    })
                    .detach();
                });
            }
        })
        .detach();
    }
}

fn write_documents_to_file(
    documents: &[mongodb::bson::Document],
    path: &std::path::Path,
    format: FileExportFormat,
    column_widths: HashMap<String, f32>,
    column_order: Vec<String>,
) -> crate::error::Result<u64> {
    use mongodb::bson::Bson;
    use std::io::{BufWriter, Write};

    let count = documents.len() as u64;

    match format {
        FileExportFormat::JsonArray => {
            let file = std::fs::File::create(path)?;
            let mut writer = BufWriter::new(file);
            writer.write_all(b"[\n")?;
            for (i, doc) in documents.iter().enumerate() {
                let json = Bson::Document(doc.clone()).into_relaxed_extjson();
                let text = serde_json::to_string_pretty(&json)?;
                writer.write_all(text.as_bytes())?;
                if i + 1 < documents.len() {
                    writer.write_all(b",")?;
                }
                writer.write_all(b"\n")?;
            }
            writer.write_all(b"]")?;
            writer.flush()?;
        }
        FileExportFormat::JsonLines => {
            let file = std::fs::File::create(path)?;
            let mut writer = BufWriter::new(file);
            for doc in documents {
                let json = Bson::Document(doc.clone()).into_relaxed_extjson();
                let text = serde_json::to_string(&json)?;
                writer.write_all(text.as_bytes())?;
                writer.write_all(b"\n")?;
            }
            writer.flush()?;
        }
        FileExportFormat::Csv => {
            use crate::connection::csv_utils::{collect_columns, flatten_document, order_columns};

            let detected = collect_columns(documents);
            let columns = order_columns(detected, &column_order);
            if columns.is_empty() {
                return Ok(0);
            }
            let file = std::fs::File::create(path)?;
            let mut csv_writer = csv::Writer::from_writer(file);
            csv_writer.write_record(&columns)?;
            for doc in documents {
                let flat = flatten_document(doc);
                let row: Vec<String> =
                    columns.iter().map(|c| flat.get(c).cloned().unwrap_or_default()).collect();
                csv_writer.write_record(&row)?;
            }
            csv_writer.flush()?;
        }
        FileExportFormat::Excel => {
            use crate::connection::csv_utils::{collect_columns, flatten_document, order_columns};
            use rust_xlsxwriter::{Format, Workbook};

            let detected = collect_columns(documents);
            let columns = order_columns(detected, &column_order);
            if columns.is_empty() {
                return Ok(0);
            }
            let mut workbook = Workbook::new();
            let header_format = Format::new().set_bold();
            let worksheet = workbook.add_worksheet_with_constant_memory();

            for (col_idx, col_name) in columns.iter().enumerate() {
                if let Some(&width_px) = column_widths.get(col_name) {
                    worksheet
                        .set_column_width_pixels(col_idx as u16, width_px as u32)
                        .map_err(|e| crate::error::Error::Parse(e.to_string()))?;
                }
            }

            for (col_idx, col_name) in columns.iter().enumerate() {
                worksheet
                    .write_string_with_format(0, col_idx as u16, col_name, &header_format)
                    .map_err(|e| crate::error::Error::Parse(e.to_string()))?;
            }

            for (row_idx, doc) in documents.iter().enumerate() {
                let row = (row_idx as u32) + 1;
                let flat = flatten_document(doc);
                for (col_idx, col_name) in columns.iter().enumerate() {
                    let col = col_idx as u16;
                    if let Some(value) = flat.get(col_name) {
                        if value.is_empty() {
                            continue;
                        }
                        if let Ok(n) = value.parse::<i64>() {
                            worksheet
                                .write_number(row, col, n as f64)
                                .map_err(|e| crate::error::Error::Parse(e.to_string()))?;
                        } else if let Ok(n) = value.parse::<f64>() {
                            worksheet
                                .write_number(row, col, n)
                                .map_err(|e| crate::error::Error::Parse(e.to_string()))?;
                        } else if value == "true" || value == "false" {
                            worksheet
                                .write_boolean(row, col, value == "true")
                                .map_err(|e| crate::error::Error::Parse(e.to_string()))?;
                        } else if value.len() > EXCEL_MAX_STRING_LEN {
                            worksheet
                                .write_string(row, col, &value[..EXCEL_MAX_STRING_LEN])
                                .map_err(|e| crate::error::Error::Parse(e.to_string()))?;
                        } else {
                            worksheet
                                .write_string(row, col, value)
                                .map_err(|e| crate::error::Error::Parse(e.to_string()))?;
                        }
                    }
                }
            }

            workbook.save(path).map_err(|e| crate::error::Error::Parse(e.to_string()))?;
        }
    }

    Ok(count)
}

const EXCEL_MAX_STRING_LEN: usize = 32_767;

#[derive(Clone)]
pub struct ExportProgress {
    pub count: u64,
    pub format: FileExportFormat,
    pub cancellation: CancellationToken,
}
