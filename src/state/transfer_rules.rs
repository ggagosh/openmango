use std::path::PathBuf;

use mongodb::bson::Document;
use uuid::Uuid;

use crate::bson::parse_document_from_json;
use crate::state::app_state::{
    TargetWriteMode, TransferFormat, TransferMode, TransferScope, TransferTabState,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransferValidation {
    pub blocking_errors: Vec<String>,
    pub warnings: Vec<String>,
    pub requires_confirmation: bool,
}

impl TransferValidation {
    pub fn can_run(&self) -> bool {
        self.blocking_errors.is_empty()
    }
}

pub fn available_transfer_formats(mode: TransferMode, scope: TransferScope) -> Vec<TransferFormat> {
    match (mode, scope) {
        (TransferMode::Export, TransferScope::Collection)
        | (TransferMode::Import, TransferScope::Collection) => {
            vec![TransferFormat::JsonLines, TransferFormat::JsonArray, TransferFormat::Csv]
        }
        (TransferMode::Export, TransferScope::Database) => vec![
            TransferFormat::JsonLines,
            TransferFormat::JsonArray,
            TransferFormat::Csv,
            TransferFormat::Bson,
        ],
        (TransferMode::Import, TransferScope::Database) => vec![TransferFormat::Bson],
        (TransferMode::Copy, _) => Vec::new(),
    }
}

pub fn default_transfer_format(mode: TransferMode, scope: TransferScope) -> TransferFormat {
    available_transfer_formats(mode, scope).into_iter().next().unwrap_or(TransferFormat::JsonLines)
}

pub fn coerce_transfer_format(
    mode: TransferMode,
    scope: TransferScope,
    format: TransferFormat,
) -> TransferFormat {
    let formats = available_transfer_formats(mode, scope);
    if formats.is_empty() || formats.contains(&format) {
        format
    } else {
        default_transfer_format(mode, scope)
    }
}

pub fn transfer_write_connection(tab: &TransferTabState) -> Option<Uuid> {
    match tab.config.mode {
        TransferMode::Import => tab.config.source_connection_id,
        TransferMode::Copy => tab.config.destination_connection_id,
        TransferMode::Export => None,
    }
}

pub fn resolved_export_destination(tab: &TransferTabState) -> Option<PathBuf> {
    if tab.config.mode != TransferMode::Export || tab.config.file_path.is_empty() {
        return None;
    }
    let expanded = crate::state::expand_filename_template(
        &tab.config.file_path,
        &tab.config.source_database,
        &tab.config.source_collection,
    );
    let path = PathBuf::from(expanded);
    if tab.config.format == TransferFormat::Bson
        && matches!(tab.options.bson_output, crate::state::BsonOutputFormat::Archive)
        && path.extension().is_none_or(|extension| extension != "archive")
    {
        Some(path.with_extension("archive"))
    } else {
        Some(path)
    }
}

pub fn parse_export_query_document(raw: &str) -> Result<Option<Document>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return Ok(None);
    }
    parse_document_from_json(trimmed).map(Some).map_err(|error| error.to_string())
}

pub fn validate_transfer(tab: &TransferTabState) -> TransferValidation {
    let mut validation = TransferValidation::default();
    let mode = tab.config.mode;
    let scope = tab.config.scope;
    let format = effective_transfer_format(tab);

    if mode != TransferMode::Copy && !available_transfer_formats(mode, scope).contains(&format) {
        validation.blocking_errors.push(match (mode, scope, format) {
            (TransferMode::Import, TransferScope::Database, _) => {
                "Database import only supports BSON dumps. Use collection import for JSON or CSV."
                    .to_string()
            }
            (_, TransferScope::Collection, TransferFormat::Bson) => {
                "BSON transfer is available for database scope only.".to_string()
            }
            _ => "This format is not available for the selected mode and scope.".to_string(),
        });
    }

    match mode {
        TransferMode::Export => validate_export(tab, &mut validation),
        TransferMode::Import => validate_import(tab, &mut validation),
        TransferMode::Copy => validate_copy(tab, &mut validation),
    }

    if matches!(format, TransferFormat::Csv)
        && matches!(mode, TransferMode::Export | TransferMode::Import)
    {
        validation
            .warnings
            .push("CSV can lose BSON type fidelity such as dates and ObjectIds.".to_string());
    }

    if matches!(tab.options.target_write_mode(), TargetWriteMode::Clear | TargetWriteMode::Drop)
        && matches!(mode, TransferMode::Import | TransferMode::Copy)
    {
        validation.requires_confirmation = true;
        validation.warnings.push(format!(
            "{} is destructive. Review the exact target before running.",
            tab.options.target_write_mode().label()
        ));
    }
    if matches!(mode, TransferMode::Import | TransferMode::Copy)
        && !matches!(tab.options.insert_mode, crate::state::InsertMode::Insert)
    {
        validation.requires_confirmation = true;
        validation.warnings.push(format!(
            "{} can overwrite existing documents with matching _id values.",
            tab.options.insert_mode.label()
        ));
    }

    validation
}

fn validate_export(tab: &TransferTabState, validation: &mut TransferValidation) {
    if tab.config.source_connection_id.is_none() {
        validation.blocking_errors.push("Choose a source connection.".to_string());
    }
    if tab.config.source_database.is_empty() {
        validation.blocking_errors.push("Choose a source database.".to_string());
    }
    if matches!(tab.config.scope, TransferScope::Collection)
        && tab.config.source_collection.is_empty()
    {
        validation.blocking_errors.push("Choose a source collection.".to_string());
    }
    if tab.config.file_path.is_empty() {
        validation.blocking_errors.push("Choose an export path.".to_string());
    }
    if matches!(tab.config.scope, TransferScope::Collection) {
        for (label, value) in [
            ("Filter", tab.options.export_filter.as_str()),
            ("Projection", tab.options.export_projection.as_str()),
            ("Sort", tab.options.export_sort.as_str()),
        ] {
            if let Err(error) = parse_export_query_document(value) {
                validation.blocking_errors.push(format!("{label}: {error}"));
            }
        }
    }
}

fn validate_import(tab: &TransferTabState, validation: &mut TransferValidation) {
    if matches!(tab.config.scope, TransferScope::Database)
        && tab.options.target_write_mode() == TargetWriteMode::Clear
    {
        validation.blocking_errors.push(
            "Clear target is unavailable for BSON database restore; use Append or Drop."
                .to_string(),
        );
    }

    if tab.config.source_connection_id.is_none() {
        validation.blocking_errors.push("Choose a target connection.".to_string());
    }
    if resolved_target_database(tab).is_empty() {
        validation.blocking_errors.push("Choose a target database.".to_string());
    }
    if matches!(tab.config.scope, TransferScope::Collection)
        && resolved_target_collection(tab).is_empty()
    {
        validation.blocking_errors.push("Choose a target collection.".to_string());
    }
    if tab.config.file_path.is_empty() {
        validation.blocking_errors.push("Choose an import file.".to_string());
    }
}

fn validate_copy(tab: &TransferTabState, validation: &mut TransferValidation) {
    if matches!(tab.config.scope, TransferScope::Database)
        && tab.options.target_write_mode() != TargetWriteMode::Append
    {
        validation.blocking_errors.push(
            "Clear and Drop are unavailable for whole-database copy; copy collections individually."
                .to_string(),
        );
    }
    if matches!(tab.config.scope, TransferScope::Collection)
        && tab.options.target_write_mode() == TargetWriteMode::Clear
        && tab.options.copy_indexes
    {
        validation
            .blocking_errors
            .push("Clear preserves target indexes. Disable Copy indexes or use Drop.".to_string());
    }

    if tab.config.source_connection_id.is_none() {
        validation.blocking_errors.push("Choose a source connection.".to_string());
    }
    if tab.config.source_database.is_empty() {
        validation.blocking_errors.push("Choose a source database.".to_string());
    }
    if matches!(tab.config.scope, TransferScope::Collection)
        && tab.config.source_collection.is_empty()
    {
        validation.blocking_errors.push("Choose a source collection.".to_string());
    }
    if tab.config.destination_connection_id.is_none() {
        validation.blocking_errors.push("Choose a target connection.".to_string());
    }

    if is_same_copy_target(tab) {
        let target = match tab.config.scope {
            TransferScope::Collection => "collection",
            TransferScope::Database => "database",
        };
        validation.blocking_errors.push(format!(
            "Choose a different target {target}. Copying onto the same {target} is blocked."
        ));
    }
}

fn resolved_target_database(tab: &TransferTabState) -> &str {
    if tab.config.destination_database.is_empty() {
        &tab.config.source_database
    } else {
        &tab.config.destination_database
    }
}

fn resolved_target_collection(tab: &TransferTabState) -> &str {
    if tab.config.destination_collection.is_empty() {
        &tab.config.source_collection
    } else {
        &tab.config.destination_collection
    }
}

fn is_same_copy_target(tab: &TransferTabState) -> bool {
    if tab.config.mode != TransferMode::Copy {
        return false;
    }

    let Some(source_connection_id) = tab.config.source_connection_id else {
        return false;
    };
    let Some(destination_connection_id) = tab.config.destination_connection_id else {
        return false;
    };
    if source_connection_id != destination_connection_id {
        return false;
    }
    if tab.config.source_database != resolved_target_database(tab) {
        return false;
    }

    match tab.config.scope {
        TransferScope::Database => true,
        TransferScope::Collection => {
            tab.config.source_collection == resolved_target_collection(tab)
        }
    }
}

fn effective_transfer_format(tab: &TransferTabState) -> TransferFormat {
    if tab.config.mode == TransferMode::Import && tab.options.detect_format {
        detect_format_from_path(&tab.config.file_path).unwrap_or(tab.config.format)
    } else {
        tab.config.format
    }
}

fn detect_format_from_path(path: &str) -> Option<TransferFormat> {
    let path = std::path::Path::new(path);
    let ext = path.extension().and_then(|e| e.to_str())?.to_lowercase();

    match ext.as_str() {
        "jsonl" | "ndjson" => Some(TransferFormat::JsonLines),
        "json" => Some(TransferFormat::JsonArray),
        "csv" => Some(TransferFormat::Csv),
        "archive" | "bson" => Some(TransferFormat::Bson),
        "gz" => {
            let stem = path.file_stem()?.to_str()?;
            detect_format_from_path(stem)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn resolved_export_path_is_frozen_after_one_template_expansion() {
        let mut tab = TransferTabState::default();
        tab.config.mode = TransferMode::Export;
        tab.config.file_path = "${database}/export.jsonl".to_string();
        tab.config.source_database = "${datetime}".to_string();

        assert_eq!(
            resolved_export_destination(&tab),
            Some(std::path::PathBuf::from("${datetime}/export.jsonl"))
        );
    }

    #[test]
    fn bson_archive_destination_matches_the_actual_output_path() {
        let mut tab = TransferTabState::default();
        tab.config.mode = TransferMode::Export;
        tab.config.format = TransferFormat::Bson;
        tab.config.file_path = "/tmp/backup".to_string();
        tab.options.bson_output = crate::state::BsonOutputFormat::Archive;

        assert_eq!(
            resolved_export_destination(&tab),
            Some(std::path::PathBuf::from("/tmp/backup.archive"))
        );
    }

    #[test]
    fn invalid_export_query_fields_block_execution_without_becoming_none() {
        let mut tab = TransferTabState::default();
        tab.config.mode = TransferMode::Export;
        tab.config.scope = TransferScope::Collection;
        tab.config.source_connection_id = Some(Uuid::new_v4());
        tab.config.source_database = "db".to_string();
        tab.config.source_collection = "users".to_string();
        tab.config.file_path = "/tmp/users.jsonl".to_string();
        tab.options.export_filter = "{ broken".to_string();
        tab.options.export_projection = "[1, 2]".to_string();
        tab.options.export_sort = r#"{ "name": }"#.to_string();

        let validation = validate_transfer(&tab);

        assert!(!validation.can_run());
        assert!(validation.blocking_errors.iter().any(|error| error.starts_with("Filter: ")));
        assert!(validation.blocking_errors.iter().any(|error| error.starts_with("Projection: ")));
        assert!(validation.blocking_errors.iter().any(|error| error.starts_with("Sort: ")));
        assert!(parse_export_query_document("{}").unwrap().is_none());
        assert!(parse_export_query_document(r#"{ "active": true }"#).unwrap().is_some());
    }

    #[test]
    fn database_json_import_is_invalid() {
        let mut tab = TransferTabState::default();
        tab.config.mode = TransferMode::Import;
        tab.config.scope = TransferScope::Database;
        tab.config.format = TransferFormat::JsonLines;
        tab.config.source_connection_id = Some(Uuid::new_v4());
        tab.config.source_database = "db".to_string();
        tab.config.file_path = "/tmp/db.jsonl".to_string();

        let validation = validate_transfer(&tab);

        assert!(!validation.can_run());
        assert!(
            validation
                .blocking_errors
                .iter()
                .any(|error| error.contains("Database import only supports BSON"))
        );
    }

    #[test]
    fn same_collection_copy_is_blocked() {
        let connection_id = Uuid::new_v4();
        let mut tab = TransferTabState::default();
        tab.config.mode = TransferMode::Copy;
        tab.config.scope = TransferScope::Collection;
        tab.config.source_connection_id = Some(connection_id);
        tab.config.destination_connection_id = Some(connection_id);
        tab.config.source_database = "db".to_string();
        tab.config.destination_database = "db".to_string();
        tab.config.source_collection = "users".to_string();
        tab.config.destination_collection = "users".to_string();

        let validation = validate_transfer(&tab);

        assert!(!validation.can_run());
        assert!(
            validation
                .blocking_errors
                .iter()
                .any(|error| error.contains("Copying onto the same collection"))
        );
    }

    #[test]
    fn database_copy_destructive_modes_are_blocked() {
        let mut tab = TransferTabState::default();
        tab.config.mode = TransferMode::Copy;
        tab.config.scope = TransferScope::Database;
        tab.config.source_connection_id = Some(Uuid::new_v4());
        tab.config.destination_connection_id = Some(Uuid::new_v4());
        tab.config.source_database = "source".to_string();
        tab.config.destination_database = "target".to_string();
        tab.options.set_target_write_mode(TargetWriteMode::Drop);

        let validation = validate_transfer(&tab);

        assert!(!validation.can_run());
        assert!(
            validation.blocking_errors.iter().any(|error| error.contains("whole-database copy"))
        );
    }

    #[test]
    fn clear_copy_with_indexes_is_blocked() {
        let mut tab = TransferTabState::default();
        tab.config.mode = TransferMode::Copy;
        tab.config.scope = TransferScope::Collection;
        tab.config.source_connection_id = Some(Uuid::new_v4());
        tab.config.destination_connection_id = Some(Uuid::new_v4());
        tab.config.source_database = "source".to_string();
        tab.config.destination_database = "target".to_string();
        tab.config.source_collection = "users".to_string();
        tab.config.destination_collection = "users".to_string();
        tab.options.copy_indexes = true;
        tab.options.set_target_write_mode(TargetWriteMode::Clear);

        let validation = validate_transfer(&tab);

        assert!(!validation.can_run());
        assert!(
            validation
                .blocking_errors
                .iter()
                .any(|error| error.contains("Clear preserves target indexes"))
        );
    }

    #[test]
    fn upsert_import_requires_confirmation_without_clear_or_drop() {
        let mut tab = TransferTabState::default();
        tab.config.mode = TransferMode::Import;
        tab.config.scope = TransferScope::Collection;
        tab.config.source_connection_id = Some(Uuid::new_v4());
        tab.config.source_database = "db".to_string();
        tab.config.source_collection = "users".to_string();
        tab.config.file_path = "/tmp/users.jsonl".to_string();
        tab.options.insert_mode = crate::state::InsertMode::Upsert;

        let validation = validate_transfer(&tab);

        assert!(validation.can_run());
        assert!(validation.requires_confirmation);
    }

    #[test]
    fn transfer_write_target_uses_import_target_and_copy_destination() {
        let source = Uuid::new_v4();
        let destination = Uuid::new_v4();
        let mut tab = TransferTabState::default();
        tab.config.source_connection_id = Some(source);
        tab.config.destination_connection_id = Some(destination);

        tab.config.mode = TransferMode::Export;
        assert_eq!(transfer_write_connection(&tab), None);
        tab.config.mode = TransferMode::Import;
        assert_eq!(transfer_write_connection(&tab), Some(source));
        tab.config.mode = TransferMode::Copy;
        assert_eq!(transfer_write_connection(&tab), Some(destination));
    }

    #[test]
    fn destructive_import_requires_confirmation() {
        let mut tab = TransferTabState::default();
        tab.config.mode = TransferMode::Import;
        tab.config.scope = TransferScope::Collection;
        tab.config.source_connection_id = Some(Uuid::new_v4());
        tab.config.source_database = "db".to_string();
        tab.config.source_collection = "users".to_string();
        tab.config.file_path = "/tmp/users.jsonl".to_string();
        tab.options.set_target_write_mode(TargetWriteMode::Clear);

        let validation = validate_transfer(&tab);

        assert!(validation.can_run());
        assert!(validation.requires_confirmation);
    }
}
