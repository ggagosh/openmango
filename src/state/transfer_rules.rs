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
}

fn validate_import(tab: &TransferTabState, validation: &mut TransferValidation) {
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
