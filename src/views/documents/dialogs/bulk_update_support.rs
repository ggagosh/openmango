use mongodb::bson::Document;

use crate::bson::parse_document_from_json;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BulkUpdateScope {
    SelectedDocument,
    FilteredQuery,
    AllDocuments,
    CustomFilter,
}

impl BulkUpdateScope {
    pub(super) fn label(self) -> &'static str {
        match self {
            BulkUpdateScope::SelectedDocument => "Selected document",
            BulkUpdateScope::FilteredQuery => "Filtered documents",
            BulkUpdateScope::AllDocuments => "All documents",
            BulkUpdateScope::CustomFilter => "Custom filter",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BulkUpdateMode {
    Update,
    Replace,
}

impl BulkUpdateMode {
    pub(super) fn label(self) -> &'static str {
        match self {
            BulkUpdateMode::Update => "Update",
            BulkUpdateMode::Replace => "Replace",
        }
    }

    pub(super) fn template(self) -> &'static str {
        match self {
            BulkUpdateMode::Update => "{\n  \"$set\": {\n    \"field\": \"value\"\n  }\n}",
            BulkUpdateMode::Replace => "{\n  \"field\": \"value\"\n}",
        }
    }
}

pub(super) fn parse_update_doc(raw: &str) -> Result<Document, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Update JSON is required.".to_string());
    }
    parse_document_from_json(trimmed).map_err(|err| format!("Invalid update JSON: {err}"))
}

pub(super) fn validate_update_doc(
    mode: BulkUpdateMode,
    scope: BulkUpdateScope,
    doc: &Document,
    selected_id: Option<&mongodb::bson::Bson>,
) -> Result<(), String> {
    if doc.is_empty() {
        return Err("Update document cannot be empty.".to_string());
    }
    match mode {
        BulkUpdateMode::Update => {
            if !doc.keys().all(|key| key.starts_with('$')) {
                return Err("Update document must use update operators (keys starting with '$')."
                    .to_string());
            }
        }
        BulkUpdateMode::Replace => {
            if doc.keys().any(|key| key.starts_with('$')) {
                return Err("Replacement document cannot include update operators.".to_string());
            }
            if let Some(id) = doc.get("_id") {
                match scope {
                    BulkUpdateScope::SelectedDocument => {
                        let Some(selected_id) = selected_id else {
                            return Err("Selected document is missing _id.".to_string());
                        };
                        if id != selected_id {
                            return Err(
                                "Replacement document cannot change the _id value.".to_string()
                            );
                        }
                    }
                    _ => {
                        return Err(
                            "Replacement document cannot include _id when updating multiple documents."
                                .to_string(),
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
