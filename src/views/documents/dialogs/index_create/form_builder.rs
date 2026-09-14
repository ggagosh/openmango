//! Form building logic for the index create dialog.

use gpui_kit::*;
use mongodb::bson::Document;

use crate::bson::parse_document_from_json;

use super::IndexCreateDialog;
use super::support::{IndexKeyKind, IndexKeySummary};

impl IndexCreateDialog {
    /// Compute a summary of the current key configuration.
    pub(super) fn key_summary(&self, cx: &mut Context<Self>) -> IndexKeySummary {
        let mut summary = IndexKeySummary::default();

        for row in &self.rows {
            let value = row.field_state.read(cx).value().to_string();
            if value.trim().is_empty() {
                continue;
            }
            summary.key_count += 1;
            match row.kind {
                IndexKeyKind::Text => {
                    summary.has_text = true;
                    summary.has_special = true;
                }
                IndexKeyKind::Hashed => {
                    summary.has_hashed = true;
                    summary.has_special = true;
                }
                IndexKeyKind::TwoDSphere => {
                    summary.has_special = true;
                }
                IndexKeyKind::Wildcard => {
                    summary.has_wildcard = true;
                }
                _ => {}
            }
        }

        summary
    }

    /// Enforce guardrails based on the current key configuration.
    pub(super) fn enforce_guardrails(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let summary = self.key_summary(cx);
        if summary.has_hashed || summary.has_text || summary.has_wildcard {
            self.unique = false;
        }

        let ttl_disabled = summary.key_count != 1 || summary.has_special || summary.has_wildcard;
        if ttl_disabled {
            self.ttl_state.update(cx, |state, cx| {
                state.set_value(String::new(), window, cx);
            });
        }

        if summary.has_wildcard {
            for row in &self.rows {
                if row.kind == IndexKeyKind::Wildcard {
                    row.field_state.update(cx, |state, cx| {
                        state.set_value("$**".to_string(), window, cx);
                    });
                }
            }
        }
    }

    /// Build an index document from the form fields.
    pub(super) fn build_index_from_form(&mut self, cx: &mut Context<Self>) -> Option<Document> {
        self.error_message = None;

        let mut keys = Document::new();
        let mut has_hashed = false;
        let mut has_text = false;
        let mut has_wildcard = false;
        let mut has_special = false;
        let mut wildcard_rows = 0;
        let mut key_count = 0;

        for row in &self.rows {
            let field = row.field_state.read(cx).value().to_string();
            let field = field.trim();
            if field.is_empty() {
                continue;
            }
            let key = if row.kind == IndexKeyKind::Wildcard { "$**" } else { field };
            if row.kind == IndexKeyKind::Wildcard {
                has_wildcard = true;
                wildcard_rows += 1;
            }
            if row.kind == IndexKeyKind::Hashed {
                has_hashed = true;
            }
            if row.kind == IndexKeyKind::Text {
                has_text = true;
            }
            if matches!(
                row.kind,
                IndexKeyKind::Text | IndexKeyKind::Hashed | IndexKeyKind::TwoDSphere
            ) {
                has_special = true;
            }
            keys.insert(key, row.kind.as_bson());
            key_count += 1;
        }

        if keys.is_empty() {
            self.error_message = Some("Add at least one key field.".to_string());
            return None;
        }

        if has_wildcard && keys.len() > 1 {
            self.error_message = Some(
                "A wildcard index can only have the $** key. Remove the other fields.".to_string(),
            );
            return None;
        }

        if has_wildcard && wildcard_rows == 0 {
            self.error_message = Some("Use $** as the field for a wildcard index.".to_string());
            return None;
        }

        if (has_hashed || has_text || has_wildcard) && self.unique {
            self.error_message =
                Some("Turn off Unique, or use ascending or descending keys.".to_string());
            return None;
        }

        let mut index_doc = self.preserved_options.clone();
        index_doc.insert("key", keys);

        let name = self.name_state.read(cx).value().to_string();
        if !name.trim().is_empty() {
            index_doc.insert("name", name.trim());
        } else if self.edit_target.is_some() {
            self.error_message = Some("Enter a name to replace this index.".to_string());
            return None;
        }

        if self.unique {
            index_doc.insert("unique", true);
        }
        if self.sparse {
            index_doc.insert("sparse", true);
        }
        if self.hidden {
            index_doc.insert("hidden", true);
        }

        let ttl_raw = self.ttl_state.read(cx).value().to_string();
        if !ttl_raw.trim().is_empty() {
            if key_count != 1 || has_special || has_wildcard {
                self.error_message = Some(
                    "Clear the expiry, or use exactly one ascending or descending key.".to_string(),
                );
                return None;
            }
            match ttl_raw.trim().parse::<i64>() {
                Ok(value) if value > 0 => {
                    index_doc.insert("expireAfterSeconds", value);
                }
                _ => {
                    self.error_message = Some(
                        "Enter the expiry as a whole number of seconds greater than 0.".to_string(),
                    );
                    return None;
                }
            }
        }

        let partial_raw = self.partial_state.read(cx).value().to_string();
        if !partial_raw.trim().is_empty() && partial_raw.trim() != "{}" {
            match parse_document_from_json(partial_raw.trim()) {
                Ok(doc) => {
                    index_doc.insert("partialFilterExpression", doc);
                }
                Err(err) => {
                    self.error_message = Some(format!("Fix the partial filter expression: {err}"));
                    return None;
                }
            }
        }

        let collation_raw = self.collation_state.read(cx).value().to_string();
        if !collation_raw.trim().is_empty() && collation_raw.trim() != "{}" {
            match parse_document_from_json(collation_raw.trim()) {
                Ok(doc) => {
                    index_doc.insert("collation", doc);
                }
                Err(err) => {
                    self.error_message = Some(format!("Fix the collation: {err}"));
                    return None;
                }
            }
        }

        Some(index_doc)
    }

    /// Build an index document from the JSON input.
    pub(super) fn build_index_from_json(&mut self, cx: &mut Context<Self>) -> Option<Document> {
        self.error_message = None;
        let raw = self.json_state.read(cx).value().to_string();
        match parse_document_from_json(raw.trim()) {
            Ok(doc) => match doc.get_document("key") {
                Ok(keys) if !keys.is_empty() => {
                    if self.edit_target.is_some() && doc.get("name").is_none() {
                        self.error_message = Some(
                            "Add a name to the index definition to replace this index.".to_string(),
                        );
                        return None;
                    }
                    Some(doc)
                }
                _ => {
                    self.error_message = Some(
                        "Add a key document to the definition, such as key: { status: 1 }."
                            .to_string(),
                    );
                    None
                }
            },
            Err(err) => {
                self.error_message = Some(format!("Fix the index definition: {err}"));
                None
            }
        }
    }
}
