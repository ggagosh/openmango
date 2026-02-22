//! Schema-specific session state operations.

use crate::state::AppState;
use crate::state::app_state::types::{SchemaField, SessionKey};

impl AppState {
    pub fn set_schema_selected_field(
        &mut self,
        session_key: &SessionKey,
        field_path: Option<String>,
    ) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.schema_selected_field = field_path;
        }
    }

    pub fn toggle_schema_expanded_field(&mut self, session_key: &SessionKey, field_path: &str) {
        if let Some(session) = self.session_mut(session_key)
            && !session.view.schema_expanded_fields.remove(field_path)
        {
            session.view.schema_expanded_fields.insert(field_path.to_string());
        }
    }

    pub fn expand_all_schema_fields(&mut self, session_key: &SessionKey) {
        let paths: Vec<String> = {
            let Some(session) = self.session(session_key) else {
                return;
            };
            let Some(schema) = &session.data.schema else {
                return;
            };
            let mut paths = Vec::new();
            collect_parent_paths(&schema.fields, &mut paths);
            paths
        };
        if let Some(session) = self.session_mut(session_key) {
            session.view.schema_expanded_fields.extend(paths);
        }
    }

    pub fn collapse_all_schema_fields(&mut self, session_key: &SessionKey) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.schema_expanded_fields.clear();
        }
    }

    pub fn set_schema_filter(&mut self, session_key: &SessionKey, filter: String) {
        if let Some(session) = self.session_mut(session_key) {
            session.view.schema_filter = filter;
        }
    }
}

fn collect_parent_paths(fields: &[SchemaField], out: &mut Vec<String>) {
    for field in fields {
        if !field.children.is_empty() {
            out.push(field.path.clone());
            collect_parent_paths(&field.children, out);
        }
    }
}
