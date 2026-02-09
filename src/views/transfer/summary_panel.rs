//! Summary panel showing transfer configuration at a glance.

use gpui::*;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};

use crate::state::{
    BsonOutputFormat, CompressionMode, TransferFormat, TransferMode, TransferScope,
    TransferTabState,
};
use crate::theme::{borders, spacing};

use super::helpers::{fallback_text, summary_item};

/// Check if transfer can be executed based on current state.
pub(super) fn can_execute_transfer(state: &TransferTabState) -> bool {
    // Must have source connection and database
    if state.config.source_connection_id.is_none() || state.config.source_database.is_empty() {
        return false;
    }

    // For collection scope, must have collection
    if matches!(state.config.scope, TransferScope::Collection)
        && state.config.source_collection.is_empty()
    {
        return false;
    }

    // For export/import, must have file path
    if matches!(state.config.mode, TransferMode::Export | TransferMode::Import)
        && state.config.file_path.is_empty()
    {
        return false;
    }

    // For copy, must have destination connection
    if matches!(state.config.mode, TransferMode::Copy)
        && state.config.destination_connection_id.is_none()
    {
        return false;
    }

    true
}

/// Render the compact summary panel showing source, destination, and format.
pub(super) fn render_summary_panel(
    transfer_state: &TransferTabState,
    _source_conn_name: &str,
    dest_conn_name: &str,
    cx: &App,
) -> AnyElement {
    let source_db = fallback_text(&transfer_state.config.source_database, "...");
    let source_coll = if matches!(transfer_state.config.scope, TransferScope::Collection) {
        format!(".{}", fallback_text(&transfer_state.config.source_collection, "..."))
    } else {
        String::new()
    };

    let target_db = if transfer_state.config.destination_database.is_empty() {
        transfer_state.config.source_database.clone()
    } else {
        transfer_state.config.destination_database.clone()
    };

    let target_coll = if transfer_state.config.destination_collection.is_empty() {
        transfer_state.config.source_collection.clone()
    } else {
        transfer_state.config.destination_collection.clone()
    };

    let source_desc = format!("{source_db}{source_coll}");

    let dest_desc = match transfer_state.config.mode {
        TransferMode::Export => {
            let is_bson_folder = matches!(transfer_state.config.format, TransferFormat::Bson)
                && matches!(transfer_state.options.bson_output, BsonOutputFormat::Folder);

            if transfer_state.config.file_path.is_empty() {
                if is_bson_folder {
                    "Choose folder...".to_string()
                } else {
                    "Choose file...".to_string()
                }
            } else {
                std::path::Path::new(&transfer_state.config.file_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| transfer_state.config.file_path.clone())
            }
        }
        TransferMode::Import => {
            let mut label = fallback_text(&target_db, "...");
            if matches!(transfer_state.config.scope, TransferScope::Collection) {
                label.push_str(&format!(".{}", fallback_text(&target_coll, "...")));
            }
            label
        }
        TransferMode::Copy => {
            let conn = if dest_conn_name == "Select connection" { "..." } else { dest_conn_name };
            let mut label = format!("{conn}:{}", fallback_text(&target_db, "..."));
            if matches!(transfer_state.config.scope, TransferScope::Collection) {
                label.push_str(&format!(".{}", fallback_text(&target_coll, "...")));
            }
            label
        }
    };

    let format_label = match (transfer_state.config.mode, transfer_state.config.format) {
        (TransferMode::Copy, _) => "Live copy".to_string(),
        (_, TransferFormat::Bson) => {
            // Include BSON output type
            format!("BSON {}", transfer_state.options.bson_output.label())
        }
        _ => transfer_state.config.format.label().to_string(),
    };

    // Add compression indicator if enabled
    let format_label = match transfer_state.options.compression {
        CompressionMode::Gzip => format!("{format_label} (gzip)"),
        CompressionMode::None => format_label,
    };

    // Compact horizontal summary
    div()
        .flex()
        .items_center()
        .justify_between()
        .p(spacing::md())
        .bg(cx.theme().sidebar)
        .border_1()
        .border_color(cx.theme().border)
        .rounded(borders::radius_sm())
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::lg())
                .child(summary_item("From", source_desc, cx))
                .child(
                    Icon::new(IconName::ArrowRight)
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(summary_item("To", dest_desc, cx))
                .child(summary_item("Format", format_label, cx)),
        )
        .into_any_element()
}
