use std::collections::HashSet;

use gpui::*;
use gpui_component::input::{CompletionProvider, InputState, Rope, RopeExt};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    InsertReplaceEdit, InsertTextFormat, Range,
};

use crate::state::AppState;

struct Operator {
    label: &'static str,
    snippet: &'static str,
    detail: &'static str,
}

const OPERATORS: &[Operator] = &[
    Operator { label: "$gt", snippet: "$gt: $0", detail: "Greater than" },
    Operator { label: "$gte", snippet: "$gte: $0", detail: "Greater than or equal" },
    Operator { label: "$lt", snippet: "$lt: $0", detail: "Less than" },
    Operator { label: "$lte", snippet: "$lte: $0", detail: "Less than or equal" },
    Operator { label: "$eq", snippet: "$eq: $0", detail: "Equals" },
    Operator { label: "$ne", snippet: "$ne: $0", detail: "Not equal" },
    Operator { label: "$in", snippet: "$in: [$0]", detail: "In array" },
    Operator { label: "$nin", snippet: "$nin: [$0]", detail: "Not in array" },
    Operator { label: "$exists", snippet: "$exists: true", detail: "Field exists" },
    Operator { label: "$type", snippet: "$type: \"$0\"", detail: "BSON type" },
    Operator { label: "$regex", snippet: "$regex: \"$0\"", detail: "Regular expression" },
    Operator { label: "$not", snippet: "$not: {$0}", detail: "Logical NOT" },
    Operator { label: "$and", snippet: "$and: [{$0}]", detail: "Logical AND" },
    Operator { label: "$or", snippet: "$or: [{$0}]", detail: "Logical OR" },
    Operator { label: "$nor", snippet: "$nor: [{$0}]", detail: "Logical NOR" },
    Operator { label: "$elemMatch", snippet: "$elemMatch: {$0}", detail: "Array element match" },
    Operator { label: "$all", snippet: "$all: [$0]", detail: "All elements match" },
    Operator { label: "$size", snippet: "$size: $0", detail: "Array size" },
];

pub struct QueryCompletionProvider {
    state: Entity<AppState>,
}

impl QueryCompletionProvider {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state }
    }

    fn field_names(&self, cx: &App) -> Vec<String> {
        let state_ref = self.state.read(cx);
        let Some(session_key) = state_ref.current_session_key() else {
            return Vec::new();
        };
        let Some(session_data) = state_ref.session_data(&session_key) else {
            return Vec::new();
        };

        let mut set = HashSet::new();
        for item in &session_data.items {
            for key in item.doc.keys() {
                set.insert(key.to_string());
            }
        }
        let mut fields: Vec<String> = set.into_iter().collect();
        fields.sort();
        fields
    }
}

impl CompletionProvider for QueryCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut Context<InputState>,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let text = rope.to_string();

        // Scan backward from offset to find the token being typed
        let mut token_start = offset;
        while token_start > 0 {
            let ch = text.as_bytes()[token_start - 1];
            if ch == b'$' || ch == b'_' || ch.is_ascii_alphanumeric() {
                token_start -= 1;
            } else {
                break;
            }
        }

        let token = &text[token_start..offset];
        let start_pos = rope.offset_to_position(token_start);
        let end_pos = rope.offset_to_position(offset);
        let replace_range = Range { start: start_pos, end: end_pos };

        let mut items: Vec<CompletionItem> = Vec::new();

        if token.starts_with('$') {
            // Operator completions
            items.extend(OPERATORS.iter().filter(|op| op.label.starts_with(token)).map(|op| {
                CompletionItem {
                    label: op.label.to_string(),
                    kind: Some(CompletionItemKind::OPERATOR),
                    detail: Some(op.detail.to_string()),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                        new_text: op.snippet.to_string(),
                        insert: replace_range,
                        replace: replace_range,
                    })),
                    ..Default::default()
                }
            }));
        } else {
            // Field name completions from loaded documents
            let fields = self.field_names(cx);
            items.extend(fields.into_iter().filter(|f| f.starts_with(token)).map(|field| {
                let insert = format!("{}: $0", field);
                CompletionItem {
                    label: field,
                    kind: Some(CompletionItemKind::FIELD),
                    detail: Some("field".to_string()),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                        new_text: insert,
                        insert: replace_range,
                        replace: replace_range,
                    })),
                    ..Default::default()
                }
            }));
        }

        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        new_text.contains('$') || new_text.chars().any(|c| c.is_ascii_alphanumeric() || c == '_')
    }
}
