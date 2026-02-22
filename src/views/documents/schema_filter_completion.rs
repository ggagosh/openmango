use std::collections::HashSet;

use gpui::*;
use gpui_component::input::{CompletionProvider, InputState, Rope, RopeExt};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    InsertReplaceEdit, InsertTextFormat, Range,
};

use crate::state::{AppState, SchemaField};

use super::schema_filter::SchemaFlag;

const CHIP_PREFIXES: &[(&str, &str)] = &[
    ("type:", "Filter by BSON type"),
    ("presence:", "Filter by presence percentage"),
    ("cardinality:", "Filter by cardinality band"),
    ("flag:", "Filter by structural flags"),
];

const PRESENCE_TOKENS: &[&str] =
    &["presence:>=90", "presence:>=75", "presence:<25", "presence:=100"];

const CARDINALITY_TOKENS: &[&str] = &["cardinality:low", "cardinality:medium", "cardinality:high"];

pub struct SchemaFilterCompletionProvider {
    state: Entity<AppState>,
}

impl SchemaFilterCompletionProvider {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state }
    }

    fn schema_field_paths(&self, cx: &App) -> Vec<String> {
        fn recurse(fields: &[SchemaField], out: &mut Vec<String>) {
            for field in fields {
                out.push(field.path.clone());
                recurse(&field.children, out);
            }
        }

        let state_ref = self.state.read(cx);
        let Some(session_key) = state_ref.current_session_key() else {
            return Vec::new();
        };
        let Some(session) = state_ref.session(&session_key) else {
            return Vec::new();
        };
        let Some(schema) = &session.data.schema else {
            return Vec::new();
        };

        let mut paths = Vec::new();
        recurse(&schema.fields, &mut paths);
        paths.sort();
        paths.dedup();
        paths
    }

    fn schema_types(&self, cx: &App) -> Vec<String> {
        fn recurse(fields: &[SchemaField], out: &mut HashSet<String>) {
            for field in fields {
                for ty in &field.types {
                    out.insert(ty.bson_type.clone());
                }
                recurse(&field.children, out);
            }
        }

        let state_ref = self.state.read(cx);
        let Some(session_key) = state_ref.current_session_key() else {
            return Vec::new();
        };
        let Some(session) = state_ref.session(&session_key) else {
            return Vec::new();
        };
        let Some(schema) = &session.data.schema else {
            return Vec::new();
        };

        let mut types = HashSet::new();
        recurse(&schema.fields, &mut types);
        let mut ordered: Vec<String> = types.into_iter().collect();
        ordered.sort();
        ordered
    }
}

impl CompletionProvider for SchemaFilterCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut Context<InputState>,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let text = rope.to_string();

        let mut token_start = offset;
        while token_start > 0 {
            let ch = text.as_bytes()[token_start - 1];
            if ch.is_ascii_whitespace() {
                break;
            }
            token_start -= 1;
        }
        let token = &text[token_start..offset];
        let token_lower = token.to_ascii_lowercase();

        let start_pos = rope.offset_to_position(token_start);
        let end_pos = rope.offset_to_position(offset);
        let replace_range = Range { start: start_pos, end: end_pos };

        let field_paths = self.schema_field_paths(cx);
        let field_paths_lower: Vec<(String, String)> = field_paths
            .into_iter()
            .map(|path| {
                let lower = path.to_ascii_lowercase();
                (path, lower)
            })
            .collect();

        let mut items = Vec::new();

        if let Some((prefix, typed_value)) = token_lower.split_once(':') {
            match prefix {
                "type" => {
                    for ty in self.schema_types(cx) {
                        if ty.to_ascii_lowercase().starts_with(typed_value) {
                            let new_text = format!("type:{ty}");
                            items.push(CompletionItem {
                                label: new_text.clone(),
                                kind: Some(CompletionItemKind::FIELD),
                                detail: Some("BSON type token".to_string()),
                                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                                text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                    InsertReplaceEdit {
                                        new_text,
                                        insert: replace_range,
                                        replace: replace_range,
                                    },
                                )),
                                ..Default::default()
                            });
                        }
                    }
                }
                "presence" => {
                    for candidate in PRESENCE_TOKENS {
                        if candidate.starts_with(token) {
                            items.push(CompletionItem {
                                label: (*candidate).to_string(),
                                kind: Some(CompletionItemKind::VALUE),
                                detail: Some("Presence token".to_string()),
                                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                                text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                    InsertReplaceEdit {
                                        new_text: (*candidate).to_string(),
                                        insert: replace_range,
                                        replace: replace_range,
                                    },
                                )),
                                ..Default::default()
                            });
                        }
                    }
                }
                "cardinality" | "card" => {
                    for candidate in CARDINALITY_TOKENS {
                        if candidate.starts_with(token) {
                            items.push(CompletionItem {
                                label: (*candidate).to_string(),
                                kind: Some(CompletionItemKind::VALUE),
                                detail: Some("Cardinality token".to_string()),
                                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                                text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                    InsertReplaceEdit {
                                        new_text: (*candidate).to_string(),
                                        insert: replace_range,
                                        replace: replace_range,
                                    },
                                )),
                                ..Default::default()
                            });
                        }
                    }
                }
                "flag" | "is" => {
                    for flag in [
                        SchemaFlag::Polymorphic,
                        SchemaFlag::Sparse,
                        SchemaFlag::Complete,
                        SchemaFlag::Nullable,
                    ] {
                        let candidate = format!("flag:{}", flag.as_str());
                        if candidate.starts_with(token) {
                            items.push(CompletionItem {
                                label: candidate.clone(),
                                kind: Some(CompletionItemKind::ENUM_MEMBER),
                                detail: Some("Flag token".to_string()),
                                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                                text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                    InsertReplaceEdit {
                                        new_text: candidate,
                                        insert: replace_range,
                                        replace: replace_range,
                                    },
                                )),
                                ..Default::default()
                            });
                        }
                    }
                }
                _ => {}
            }
        } else {
            for (prefix, detail) in CHIP_PREFIXES {
                if token.is_empty() || prefix.starts_with(token) {
                    items.push(CompletionItem {
                        label: (*prefix).to_string(),
                        kind: Some(CompletionItemKind::KEYWORD),
                        detail: Some((*detail).to_string()),
                        insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                        text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                            new_text: (*prefix).to_string(),
                            insert: replace_range,
                            replace: replace_range,
                        })),
                        ..Default::default()
                    });
                }
            }

            for (path, path_lower) in field_paths_lower {
                if token.is_empty() || path_lower.starts_with(&token_lower) {
                    items.push(CompletionItem {
                        label: path.clone(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some("Field path".to_string()),
                        insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                        text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                            new_text: path,
                            insert: replace_range,
                            replace: replace_range,
                        })),
                        ..Default::default()
                    });
                }
            }
        }

        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        new_text.chars().any(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '.' | '_'))
    }
}
