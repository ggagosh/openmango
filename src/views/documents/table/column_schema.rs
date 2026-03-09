use std::collections::{HashMap, HashSet};

use gpui::px;
use gpui_component::table::{Column, ColumnSort};
use mongodb::bson::{Bson, Document};

use crate::state::SessionDocument;

const MAX_COLUMNS: usize = 50;
const CHAR_WIDTH: f32 = 10.0;
/// Header overhead: left pad (8) + right pad (8) + sort icon (12) + pin icon (12) + gaps (8) = 48.
const HEADER_PAD: f32 = 48.0;
/// Data cell overhead: left pad (8) + right pad (8) = 16.
const CELL_PAD: f32 = 16.0;
pub const MIN_COL_WIDTH: f32 = 80.0;
const MAX_COL_WIDTH: f32 = 400.0;
/// Default width for text/string-heavy columns (no per-value sampling).
const DEFAULT_TEXT_WIDTH: f32 = 200.0;

/// Full ObjectId hex = 24 chars.
const OBJECTID_CHARS: f32 = 24.0;
/// RFC 3339 with offset, e.g. "2024-01-15T14:30:00.000+00:00" ≈ 30 chars.
const DATETIME_CHARS: f32 = 30.0;

#[derive(Clone)]
pub struct TableColumnDef {
    pub key: String,
    pub width: f32,
}

/// Minimum character count to guarantee a BSON type's value is fully visible.
fn type_min_chars(value: &Bson) -> Option<f32> {
    match value {
        Bson::ObjectId(_) => Some(OBJECTID_CHARS),
        Bson::DateTime(_) | Bson::Timestamp(_) => Some(DATETIME_CHARS),
        _ => None,
    }
}

/// Whether a value is a variable-length text type that should use a flat default width
/// instead of being measured per-value.
fn is_text_type(value: &Bson) -> bool {
    matches!(value, Bson::String(_) | Bson::Document(_) | Bson::Array(_) | Bson::Binary(_))
}

/// Discover all unique top-level keys across documents, preserving insertion order.
/// `_id` is always first. Returns at most MAX_COLUMNS columns.
///
/// Width priority (largest wins):
/// 1. Column key (header) — always fully visible, including sort icon
/// 2. Type-aware minimum — ObjectId and DateTime columns show full values
/// 3. Flat default for text-heavy columns (strings, nested docs/arrays)
/// 4. Sampled content width for short scalar types (numbers, booleans, etc.)
pub fn discover_columns(documents: &[SessionDocument]) -> Vec<TableColumnDef> {
    discover_columns_inner(documents.iter().map(|item| &item.doc))
}

pub fn discover_columns_raw(documents: &[Document]) -> Vec<TableColumnDef> {
    discover_columns_inner(documents.iter())
}

fn discover_columns_inner<'a>(
    docs: impl Iterator<Item = &'a Document> + Clone,
) -> Vec<TableColumnDef> {
    let mut keys: Vec<String> = Vec::new();
    keys.push("_id".to_string());

    for doc in docs.clone() {
        for key in doc.keys() {
            if key == "_id" {
                continue;
            }
            if !keys.contains(key) {
                keys.push(key.clone());
            }
            if keys.len() >= MAX_COLUMNS {
                break;
            }
        }
        if keys.len() >= MAX_COLUMNS {
            break;
        }
    }

    keys.into_iter()
        .map(|key| {
            let sampled: Vec<&Bson> =
                docs.clone().take(20).filter_map(|doc| doc.get(&key)).collect();
            compute_column_width(key, &sampled)
        })
        .collect()
}

fn compute_column_width(key: String, sampled: &[&Bson]) -> TableColumnDef {
    let header_width = key.chars().count() as f32 * CHAR_WIDTH + HEADER_PAD;

    let type_width = sampled
        .iter()
        .filter_map(|v| type_min_chars(v))
        .reduce(f32::max)
        .map(|chars| chars * CHAR_WIDTH + CELL_PAD)
        .unwrap_or(0.0);

    let has_text = sampled.iter().any(|v| is_text_type(v));
    let content_width = if has_text {
        DEFAULT_TEXT_WIDTH
    } else {
        sampled
            .iter()
            .map(|v| {
                crate::bson::bson_value_preview(v, 40).chars().count() as f32 * CHAR_WIDTH
                    + CELL_PAD
            })
            .reduce(f32::max)
            .unwrap_or(0.0)
    };

    let width = header_width.max(type_width).max(content_width).clamp(MIN_COL_WIDTH, MAX_COL_WIDTH);
    TableColumnDef { key, width }
}

pub fn build_column_defs(columns: &[TableColumnDef]) -> Vec<Column> {
    build_column_defs_with_overrides(columns, &HashMap::new(), &None, &HashSet::new())
}

/// Build Column definitions, using saved widths, active sort, and pinned columns.
pub fn build_column_defs_with_overrides(
    columns: &[TableColumnDef],
    saved_widths: &HashMap<String, f32>,
    active_sort: &Option<(String, ColumnSort)>,
    pinned_columns: &HashSet<String>,
) -> Vec<Column> {
    columns
        .iter()
        .map(|col| {
            let width = saved_widths.get(&col.key).copied().unwrap_or(col.width);
            let mut column = Column::new(col.key.clone(), col.key.clone()).width(px(width));
            // Restore sort state for the active column, default for others.
            column = match active_sort {
                Some((key, sort)) if key == &col.key => column.sort(*sort),
                _ => column.sortable(),
            };
            if col.key == "_id" || pinned_columns.contains(&col.key) {
                column = column.fixed_left().movable(false);
            }
            column
        })
        .collect()
}
