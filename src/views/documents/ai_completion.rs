use std::cell::Cell;
use std::collections::HashSet;

use crate::state::AppState;
use gpui::*;
use gpui_component::input::{CompletionProvider, InputState, Rope, RopeExt};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    InsertReplaceEdit, InsertTextFormat, Range,
};

pub struct AiPromptCompletionProvider {
    state: Entity<AppState>,
    /// Tracks whether the last `completions()` call found an @-token.
    /// Used by `is_completion_trigger` to keep the menu alive while typing
    /// after `@` without reading InputState (which would cause a borrow panic).
    in_at_token: Cell<bool>,
}

impl AiPromptCompletionProvider {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state, in_at_token: Cell::new(false) }
    }
}

#[derive(Default)]
struct AiPromptContextData {
    collections: Vec<String>,
}

impl AiPromptCompletionProvider {
    fn prompt_context_data(&self, cx: &App) -> AiPromptContextData {
        let mut context = AiPromptContextData::default();
        let state_ref = self.state.read(cx);
        let Some(active) = state_ref.active_connection() else {
            return context;
        };
        let database = state_ref.selected_database_name().or_else(|| {
            state_ref.current_ai_session_key().map(|session_key| session_key.database.clone())
        });
        let Some(database) = database else {
            return context;
        };

        context.collections = active.collections.get(&database).cloned().unwrap_or_default();
        context.collections.sort_unstable_by_key(|name| name.to_lowercase());
        context
    }
}

impl CompletionProvider for AiPromptCompletionProvider {
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
            if text.as_bytes()[token_start - 1].is_ascii_whitespace() {
                break;
            }
            token_start -= 1;
        }
        let token = &text[token_start..offset];

        // Track whether we're in an @-token for is_completion_trigger.
        self.in_at_token.set(token.starts_with('@'));

        let start_pos = rope.offset_to_position(token_start);
        let end_pos = rope.offset_to_position(offset);
        let replace_range = Range { start: start_pos, end: end_pos };

        let mut items = Vec::new();
        let mut seen_labels = HashSet::new();
        let context = self.prompt_context_data(cx);
        let mut push_item = |item: CompletionItem| {
            if seen_labels.insert(item.label.clone()) {
                items.push(item);
            }
        };

        if token.starts_with('@') {
            let tag_prefix = token.strip_prefix('@').unwrap_or("").to_ascii_lowercase();

            for collection in context.collections {
                let collection_lower = collection.to_ascii_lowercase();
                if !tag_prefix.is_empty() && !collection_lower.starts_with(&tag_prefix) {
                    continue;
                }
                let tagged_name = format!("@{collection}");
                push_item(CompletionItem {
                    label: tagged_name.clone(),
                    kind: Some(CompletionItemKind::FIELD),
                    detail: Some("Tag collection in current database".to_string()),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                        new_text: tagged_name,
                        insert: replace_range,
                        replace: replace_range,
                    })),
                    ..Default::default()
                });
            }
        }

        items.truncate(160);
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        // Trigger immediately when '@' is typed.
        if new_text.contains('@') {
            self.in_at_token.set(true);
            return true;
        }

        // While inside an @-token (tracked by the last completions() call or the
        // '@' branch above), keep triggering for non-whitespace input so the menu
        // stays open and re-filters. Whitespace ends the token.
        if self.in_at_token.get() {
            if new_text.chars().any(char::is_whitespace) {
                self.in_at_token.set(false);
                return false;
            }
            return true;
        }

        false
    }
}
