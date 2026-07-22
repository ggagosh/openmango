//! Summary panel showing transfer configuration at a glance.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};

use crate::components::{ConnectionIdentity, connection_identity_badge};
use crate::state::{
    BsonOutputFormat, CompressionMode, TransferFormat, TransferMode, TransferScope,
    TransferTabState,
};
use crate::theme::{borders, spacing};

use super::helpers::{fallback_text, summary_item};

/// Render the compact summary panel showing source, destination, and format.
pub(super) fn render_summary_panel(
    transfer_state: &TransferTabState,
    source_identity: Option<&ConnectionIdentity>,
    destination_identity: Option<&ConnectionIdentity>,
    cx: &App,
) -> AnyElement {
    let source_conn_name = source_identity
        .map(ConnectionIdentity::display_name)
        .unwrap_or_else(|| "Select connection".into());
    let dest_conn_name = destination_identity
        .map(ConnectionIdentity::display_name)
        .unwrap_or_else(|| "Select connection".into());
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

    let source_namespace = format!("{source_db}{source_coll}");
    let source_connection =
        if source_conn_name == "Select connection" { "..." } else { &source_conn_name };
    let source_desc = match transfer_state.config.mode {
        TransferMode::Import => {
            if transfer_state.config.file_path.is_empty() {
                "Choose file...".to_string()
            } else {
                std::path::Path::new(&transfer_state.config.file_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&transfer_state.config.file_path)
                    .to_string()
            }
        }
        TransferMode::Export | TransferMode::Copy => {
            format!("{source_connection}:{source_namespace}")
        }
    };

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
            let mut label = format!("{source_connection}:{}", fallback_text(&target_db, "..."));
            if matches!(transfer_state.config.scope, TransferScope::Collection) {
                label.push_str(&format!(".{}", fallback_text(&target_coll, "...")));
            }
            label
        }
        TransferMode::Copy => {
            let conn = if dest_conn_name == "Select connection" { "..." } else { &dest_conn_name };
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
    let format_label = match (transfer_state.config.mode, transfer_state.options.compression) {
        (TransferMode::Export, CompressionMode::Gzip) => format!("{format_label} (gzip)"),
        _ => format_label,
    };

    let identity_row = div()
        .flex()
        .items_center()
        .gap(spacing::md())
        .when_some(source_identity, |row, identity| {
            row.child(connection_identity_badge(identity, true, cx))
        })
        .when_some(
            (transfer_state.config.mode == TransferMode::Copy)
                .then_some(destination_identity)
                .flatten(),
            |row, identity| {
                row.child(
                    Icon::new(IconName::ArrowRight)
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(connection_identity_badge(identity, true, cx))
            },
        );

    div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .justify_between()
        .p(spacing::md())
        .bg(cx.theme().sidebar)
        .border_1()
        .border_color(cx.theme().border)
        .rounded(borders::radius_sm())
        .child(identity_row)
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
