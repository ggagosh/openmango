use std::collections::HashSet;
use std::path::Path;

use futures::TryStreamExt;
use mongodb::Client;
use mongodb::bson::Document;
use rust_xlsxwriter::{Format, Workbook};

use crate::ai::blocks::ReportSheet;
use crate::ai::tools::generate_report::parse_and_sanitize_pipeline;
use crate::connection::ConnectionManager;
use crate::connection::csv_utils::{collect_document_columns, flatten_document, order_columns};
use crate::connection::ops::export::AtomicExportFile;
use crate::error::{Error, Result};

const EXCEL_MAX_ROWS: u32 = 1_048_576;
const EXCEL_MAX_STRING_LEN: usize = 32_767;
const COLUMN_SAMPLE_SIZE: usize = 100;
const PROGRESS_INTERVAL: u64 = 500;

pub struct ReportExportResult {
    pub total_rows: u64,
    pub sheets_written: usize,
    pub errors: Vec<String>,
}

impl ConnectionManager {
    /// Stream report data from MongoDB into a multi-sheet Excel file.
    ///
    /// # Safety guarantees
    /// - Pipelines are re-sanitized before execution (defense in depth)
    /// - Streaming write via `add_worksheet_with_constant_memory` — no full dataset in memory
    /// - Excel row limit enforced per sheet
    /// - Runs inside Tokio runtime via `block_on` (matches existing export pattern)
    pub fn export_report_to_excel<F>(
        &self,
        client: &Client,
        database: &str,
        sheets: &[ReportSheet],
        path: &Path,
        on_progress: F,
    ) -> Result<ReportExportResult>
    where
        F: Fn(u64) + Send + 'static,
    {
        let client = client.clone();
        let database = database.to_string();
        let sheets = sheets.to_vec();
        let path = path.to_path_buf();

        self.runtime.block_on(async move {
            let mut workbook = Workbook::new();
            let header_format = Format::new().set_bold();
            let mut total_rows = 0u64;
            let mut sheets_written = 0usize;
            let mut errors = Vec::new();

            for sheet in &sheets {
                match write_single_sheet(
                    &client,
                    &database,
                    sheet,
                    &mut workbook,
                    &header_format,
                    total_rows,
                    &on_progress,
                )
                .await
                {
                    Ok(count) => {
                        total_rows += count;
                        sheets_written += 1;
                    }
                    Err(e) => {
                        errors.push(format!("Sheet '{}': {}", sheet.name, e));
                    }
                }
            }

            if !errors.is_empty() {
                return Err(Error::Parse(format!(
                    "Report export was not committed because sheet generation failed: {}",
                    errors.join("; ")
                )));
            }
            if sheets_written == 0 {
                return Err(Error::Parse("Report contained no exportable sheets".to_string()));
            }

            let output = AtomicExportFile::new(&path)?;
            workbook.save(output.temporary_path()).map_err(|e| Error::Parse(e.to_string()))?;
            output.commit()?;
            on_progress(total_rows);

            Ok(ReportExportResult { total_rows, sheets_written, errors })
        })
    }
}

async fn write_single_sheet(
    client: &Client,
    database: &str,
    sheet: &ReportSheet,
    workbook: &mut Workbook,
    header_format: &Format,
    rows_before: u64,
    on_progress: &impl Fn(u64),
) -> Result<u64> {
    // Defense in depth: re-sanitize pipeline before execution.
    let pipeline =
        parse_and_sanitize_pipeline(&sheet.pipeline).map_err(|e| Error::Parse(e.to_string()))?;

    let collection = client.database(database).collection::<Document>(&sheet.collection);
    let mut discovery_cursor = collection.aggregate(pipeline.clone()).await?;
    let mut seen_columns = HashSet::new();
    let mut detected = Vec::new();
    let mut width_samples: Vec<Document> = Vec::with_capacity(COLUMN_SAMPLE_SIZE);
    while let Some(doc) = discovery_cursor.try_next().await? {
        collect_document_columns(&doc, &mut seen_columns, &mut detected);
        if width_samples.len() < COLUMN_SAMPLE_SIZE {
            width_samples.push(doc);
        }
    }

    let columns = order_columns(detected, &[]);
    let worksheet = workbook.add_worksheet_with_constant_memory();
    let sheet_name = sanitize_sheet_name(&sheet.name);
    worksheet.set_name(&sheet_name).map_err(|e| Error::Parse(e.to_string()))?;
    if columns.is_empty() {
        return Ok(0);
    }
    let mut cursor = collection.aggregate(pipeline).await?;

    for (col_idx, col_name) in columns.iter().enumerate() {
        let estimated_width = estimate_column_width(col_name, &width_samples, col_idx);
        worksheet
            .set_column_width_pixels(col_idx as u16, estimated_width)
            .map_err(|e| Error::Parse(e.to_string()))?;
    }

    worksheet.set_freeze_panes(1, 0).map_err(|e| Error::Parse(e.to_string()))?;

    for (col_idx, col_name) in columns.iter().enumerate() {
        worksheet
            .write_string_with_format(0, col_idx as u16, col_name, header_format)
            .map_err(|e| Error::Parse(e.to_string()))?;
    }

    worksheet
        .autofilter(0, 0, 0, columns.len().saturating_sub(1) as u16)
        .map_err(|e| Error::Parse(e.to_string()))?;

    let mut count = 0u64;

    while let Some(doc) = cursor.try_next().await? {
        let row = count as u32 + 1;
        if row >= EXCEL_MAX_ROWS {
            return Err(Error::Parse(format!(
                "Sheet '{}' exceeds Excel's {} row limit; no rows were skipped",
                sheet.name,
                EXCEL_MAX_ROWS - 1
            )));
        }
        let flat = flatten_document(&doc);
        if let Some(field) = flat.keys().find(|field| !seen_columns.contains(*field)) {
            return Err(Error::Parse(format!(
                "Report source changed while discovering columns; new field '{field}' was not skipped"
            )));
        }
        write_excel_row(worksheet, row, &columns, &flat)?;
        count += 1;
        if count.is_multiple_of(PROGRESS_INTERVAL) {
            on_progress(rows_before + count);
        }
    }

    Ok(count)
}

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
                    .map_err(|e| Error::Parse(e.to_string()))?;
            } else if let Ok(n) = value.parse::<f64>() {
                worksheet.write_number(row, col, n).map_err(|e| Error::Parse(e.to_string()))?;
            } else if value == "true" || value == "false" {
                worksheet
                    .write_boolean(row, col, value == "true")
                    .map_err(|e| Error::Parse(e.to_string()))?;
            } else if value.chars().count() > EXCEL_MAX_STRING_LEN {
                return Err(Error::Parse(format!(
                    "Excel cell in column '{col_name}' exceeds the {EXCEL_MAX_STRING_LEN} character limit; value was not truncated"
                )));
            } else {
                worksheet.write_string(row, col, value).map_err(|e| Error::Parse(e.to_string()))?;
            }
        }
    }
    Ok(())
}

fn estimate_column_width(col_name: &str, sample_docs: &[Document], col_idx: usize) -> u32 {
    let header_chars = col_name.chars().count();
    let max_data_chars = sample_docs
        .iter()
        .take(20)
        .filter_map(|doc| {
            let flat = flatten_document(doc);
            let columns: Vec<String> = flat.keys().cloned().collect();
            columns.get(col_idx).and_then(|k| flat.get(k)).map(|v| v.chars().count())
        })
        .max()
        .unwrap_or(0);
    let chars = header_chars.max(max_data_chars).min(50);
    ((chars as f32 * 7.5) + 16.0) as u32
}

fn sanitize_sheet_name(name: &str) -> String {
    const INVALID_CHARS: &[char] = &['\\', '/', '*', '?', ':', '[', ']'];
    let sanitized: String =
        name.chars().map(|c| if INVALID_CHARS.contains(&c) { '_' } else { c }).take(31).collect();
    if sanitized.is_empty() { "Sheet".to_string() } else { sanitized }
}
