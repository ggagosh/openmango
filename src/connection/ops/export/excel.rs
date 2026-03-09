use std::collections::HashMap;
use std::path::Path;

use mongodb::Client;
use mongodb::bson::Document;

use crate::connection::ConnectionManager;
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
        use crate::connection::csv_utils::{collect_columns, flatten_document, order_columns};
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

            let mut cursor = coll.find(filter).with_options(find_options).await?;

            const SAMPLE_SIZE: usize = 1000;
            let mut buffered_docs: Vec<Document> = Vec::with_capacity(SAMPLE_SIZE);

            while buffered_docs.len() < SAMPLE_SIZE {
                match cursor.try_next().await? {
                    Some(doc) => buffered_docs.push(doc),
                    None => break,
                }
            }

            let detected_columns = collect_columns(&buffered_docs);

            if detected_columns.is_empty() {
                on_progress(0);
                return Ok(0);
            }

            let columns = order_columns(detected_columns, &column_order);

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

            for doc in buffered_docs {
                if cancellation.as_ref().is_some_and(|c| c.is_cancelled()) {
                    return Err(crate::error::Error::Parse("Export cancelled".to_string()));
                }

                let row = count as u32 + 1;
                let flat = flatten_document(&doc);
                write_excel_row(worksheet, row, &columns, &flat)?;
                count += 1;

                if count.is_multiple_of(PROGRESS_INTERVAL) {
                    on_progress(count);
                }
            }

            while let Some(doc) = cursor.try_next().await? {
                if cancellation.as_ref().is_some_and(|c| c.is_cancelled()) {
                    return Err(crate::error::Error::Parse("Export cancelled".to_string()));
                }

                let row = count as u32 + 1;
                let flat = flatten_document(&doc);
                write_excel_row(worksheet, row, &columns, &flat)?;
                count += 1;

                if count.is_multiple_of(PROGRESS_INTERVAL) {
                    on_progress(count);
                }
            }

            workbook.save(&path).map_err(|e| crate::error::Error::Parse(e.to_string()))?;
            on_progress(count);
            Ok(count)
        })
    }
}

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
    Ok(())
}
