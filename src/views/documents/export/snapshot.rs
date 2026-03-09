use std::collections::{HashMap, HashSet};

use mongodb::bson::Document;

use crate::bson::DocumentKey;
use crate::state::SessionDocument;

pub enum ExportScope {
    Selected,
    CurrentPage,
}

pub struct ColumnInfo {
    pub key: String,
    pub pinned: bool,
}

pub struct ViewExportSnapshot {
    pub documents: Vec<Document>,
    pub columns: Vec<ColumnInfo>,
    pub collection_name: String,
    pub database_name: String,
}

impl ViewExportSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn from_session_state(
        items: &[SessionDocument],
        selected_docs: &HashSet<DocumentKey>,
        column_order: &[String],
        hidden_columns: &HashSet<String>,
        pinned_columns: &HashSet<String>,
        drafts: &HashMap<DocumentKey, Document>,
        collection_name: String,
        database_name: String,
        scope: ExportScope,
    ) -> Self {
        let documents: Vec<Document> = match scope {
            ExportScope::Selected if !selected_docs.is_empty() => items
                .iter()
                .filter(|sd| selected_docs.contains(&sd.key))
                .map(|sd| drafts.get(&sd.key).cloned().unwrap_or_else(|| sd.doc.clone()))
                .collect(),
            _ => items
                .iter()
                .map(|sd| drafts.get(&sd.key).cloned().unwrap_or_else(|| sd.doc.clone()))
                .collect(),
        };

        let mut pinned_cols: Vec<ColumnInfo> = Vec::new();
        let mut regular_cols: Vec<ColumnInfo> = Vec::new();

        for key in column_order {
            if hidden_columns.contains(key) {
                continue;
            }
            let info = ColumnInfo { key: key.clone(), pinned: pinned_columns.contains(key) };
            if info.pinned {
                pinned_cols.push(info);
            } else {
                regular_cols.push(info);
            }
        }

        pinned_cols.append(&mut regular_cols);

        // If no column order is set (e.g. tree view), discover from documents.
        let columns = if pinned_cols.is_empty() {
            let mut keys = Vec::new();
            let mut seen = HashSet::new();
            for doc in &documents {
                for key in doc.keys() {
                    if seen.insert(key.clone()) {
                        keys.push(ColumnInfo { key: key.clone(), pinned: false });
                    }
                }
            }
            keys
        } else {
            pinned_cols
        };

        Self { documents, columns, collection_name, database_name }
    }
}
