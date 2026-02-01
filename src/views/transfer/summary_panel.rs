//! Summary panel showing transfer configuration at a glance.

use gpui::*;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::state::{
    BsonOutputFormat, CompressionMode, TransferFormat, TransferMode, TransferScope,
    TransferTabState,
};
use crate::theme::{borders, colors, spacing};

use super::helpers::{fallback_text, summary_item};

/// Check if transfer can be executed based on current state.
pub(super) fn can_execute_transfer(state: &TransferTabState) -> bool {
    // Must have source connection and database
    if state.source_connection_id.is_none() || state.source_database.is_empty() {
        return false;
    }

    // For collection scope, must have collection
    if matches!(state.scope, TransferScope::Collection) && state.source_collection.is_empty() {
        return false;
    }

    // For export/import, must have file path
    if matches!(state.mode, TransferMode::Export | TransferMode::Import)
        && state.file_path.is_empty()
    {
        return false;
    }

    // For copy, must have destination connection
    if matches!(state.mode, TransferMode::Copy) && state.destination_connection_id.is_none() {
        return false;
    }

    true
}

/// Render the compact summary panel showing source, destination, and format.
pub(super) fn render_summary_panel(
    transfer_state: &TransferTabState,
    _source_conn_name: &str,
    dest_conn_name: &str,
) -> impl IntoElement {
    let source_db = fallback_text(&transfer_state.source_database, "...");
    let source_coll = if matches!(transfer_state.scope, TransferScope::Collection) {
        format!(".{}", fallback_text(&transfer_state.source_collection, "..."))
    } else {
        String::new()
    };

    let target_db = if transfer_state.destination_database.is_empty() {
        transfer_state.source_database.clone()
    } else {
        transfer_state.destination_database.clone()
    };

    let target_coll = if transfer_state.destination_collection.is_empty() {
        transfer_state.source_collection.clone()
    } else {
        transfer_state.destination_collection.clone()
    };

    let source_desc = format!("{source_db}{source_coll}");

    let dest_desc = match transfer_state.mode {
        TransferMode::Export => {
            let is_bson_folder = matches!(transfer_state.format, TransferFormat::Bson)
                && matches!(transfer_state.bson_output, BsonOutputFormat::Folder);

            if transfer_state.file_path.is_empty() {
                if is_bson_folder {
                    "Choose folder...".to_string()
                } else {
                    "Choose file...".to_string()
                }
            } else {
                std::path::Path::new(&transfer_state.file_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| transfer_state.file_path.clone())
            }
        }
        TransferMode::Import => {
            let mut label = fallback_text(&target_db, "...");
            if matches!(transfer_state.scope, TransferScope::Collection) {
                label.push_str(&format!(".{}", fallback_text(&target_coll, "...")));
            }
            label
        }
        TransferMode::Copy => {
            let conn = if dest_conn_name == "Select connection" { "..." } else { dest_conn_name };
            let mut label = format!("{conn}:{}", fallback_text(&target_db, "..."));
            if matches!(transfer_state.scope, TransferScope::Collection) {
                label.push_str(&format!(".{}", fallback_text(&target_coll, "...")));
            }
            label
        }
    };

    let format_label = match (transfer_state.mode, transfer_state.format) {
        (TransferMode::Copy, _) => "Live copy".to_string(),
        (_, TransferFormat::Bson) => {
            // Include BSON output type
            format!("BSON {}", transfer_state.bson_output.label())
        }
        _ => transfer_state.format.label().to_string(),
    };

    // Add compression indicator if enabled
    let format_label = match transfer_state.compression {
        CompressionMode::Gzip => format!("{format_label} (gzip)"),
        CompressionMode::None => format_label,
    };

    // Compact horizontal summary
    div()
        .flex()
        .items_center()
        .justify_between()
        .p(spacing::md())
        .bg(colors::bg_sidebar())
        .border_1()
        .border_color(colors::border())
        .rounded(borders::radius_sm())
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::lg())
                .child(summary_item("From", source_desc))
                .child(Icon::new(IconName::ArrowRight).xsmall().text_color(colors::text_muted()))
                .child(summary_item("To", dest_desc))
                .child(summary_item("Format", format_label)),
        )
}
