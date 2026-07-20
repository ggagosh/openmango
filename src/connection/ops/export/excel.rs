use std::collections::{HashMap, HashSet};
use std::path::Path;

use mongodb::Client;
use mongodb::bson::Document;

use crate::connection::ConnectionManager;
use crate::connection::ops::export::AtomicExportFile;
use crate::connection::types::{CancellationToken, ExportQueryOptions};
use crate::error::Result;

impl ConnectionManager {
    #[allow(clippy::too_many_arguments)]
    pub fn export_collection_excel_with_query<F>(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        path: &Path,
        query: ExportQueryOptions,
        column_widths: HashMap<String, f32>,
        column_order: Vec<String>,
        cancellation: Option<CancellationToken>,
        on_progress: F,
    ) -> Result<u64>
    where
        F: Fn(u64) + Send + 'static,
    {
        use crate::connection::csv_utils::{
            collect_document_columns, flatten_document, order_columns,
        };
        use futures::TryStreamExt;
        use rust_xlsxwriter::{Format, Workbook};

        let client = client.clone();
        let database = database.to_string();
        let collection_name = collection.to_string();
        let path = path.to_path_buf();

        self.runtime.block_on(async move {
            let coll = client.database(&database).collection::<Document>(&collection_name);

            let filter = query.filter.unwrap_or_default();
            let mut find_options = mongodb::options::FindOptions::default();
            find_options.projection = query.projection;
            find_options.sort = query.sort;

            let mut discovery_cursor =
                coll.find(filter.clone()).with_options(find_options.clone()).await?;
            let mut seen_columns = HashSet::new();
            let mut detected_columns = Vec::new();
            while let Some(doc) = discovery_cursor.try_next().await? {
                if cancellation.as_ref().is_some_and(|token| token.is_cancelled()) {
                    return Err(crate::error::Error::Parse("Export cancelled".to_string()));
                }
                collect_document_columns(&doc, &mut seen_columns, &mut detected_columns);
            }

            let columns = order_columns(detected_columns, &column_order);
            let mut cursor = coll.find(filter).with_options(find_options).await?;

            let mut workbook = Workbook::new();
            let header_format = Format::new().set_bold();
            let worksheet = workbook.add_worksheet_with_constant_memory();
            worksheet
                .set_name(&collection_name)
                .map_err(|e| crate::error::Error::Parse(e.to_string()))?;

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

            let mut count = 0u64;
            const PROGRESS_INTERVAL: u64 = 1000;

            while let Some(doc) = cursor.try_next().await? {
                if cancellation.as_ref().is_some_and(|c| c.is_cancelled()) {
                    return Err(crate::error::Error::Parse("Export cancelled".to_string()));
                }

                let row = count as u32 + 1;
                if row >= EXCEL_MAX_ROWS {
                    return Err(crate::error::Error::Parse(format!(
                        "Excel export exceeds the {} row limit; no rows were skipped",
                        EXCEL_MAX_ROWS - 1
                    )));
                }
                let flat = flatten_document(&doc);
                if let Some(field) = flat.keys().find(|field| !seen_columns.contains(*field)) {
                    return Err(crate::error::Error::Parse(format!(
                        "Export source changed while discovering columns; new field '{field}' was not skipped"
                    )));
                }
                write_excel_row(worksheet, row, &columns, &flat)?;
                count += 1;

                if count.is_multiple_of(PROGRESS_INTERVAL) {
                    on_progress(count);
                }
            }

            if cancellation.as_ref().is_some_and(|token| token.is_cancelled()) {
                return Err(crate::error::Error::Parse("Export cancelled".to_string()));
            }
            let output = AtomicExportFile::new(&path)?;
            workbook
                .save(output.temporary_path())
                .map_err(|e| crate::error::Error::Parse(e.to_string()))?;
            if cancellation.as_ref().is_some_and(|token| token.is_cancelled()) {
                return Err(crate::error::Error::Parse("Export cancelled".to_string()));
            }
            output.commit()?;
            on_progress(count);
            Ok(count)
        })
    }
}

const EXCEL_MAX_ROWS: u32 = 1_048_576;
const EXCEL_MAX_STRING_LEN: usize = 32_767;

fn write_excel_row(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    row: u32,
    columns: &[String],
    flat: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
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
            } else if value.chars().count() > EXCEL_MAX_STRING_LEN {
                return Err(crate::error::Error::Parse(format!(
                    "Excel cell in column '{col_name}' exceeds the {EXCEL_MAX_STRING_LEN} character limit; value was not truncated"
                )));
            } else {
                worksheet
                    .write_string(row, col, value)
                    .map_err(|e| crate::error::Error::Parse(e.to_string()))?;
            }
        }
    }
    Ok(())
}
