use std::sync::Arc;
use std::time::Duration;

use gpui::*;
use gpui_component::input::{CompletionProvider, InputState, Rope, RopeExt};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    InsertReplaceEdit, Range,
};

use super::ForgeRuntime;
use super::ForgeView;
use super::logic::{
    ContextKind, METHODS, OPERATORS, collection_method_template, completion_token, detect_context,
    merge_suggestions, should_skip_completion,
};
use super::runtime::active_forge_session_info;
use super::types::{Suggestion, SuggestionKind};
use crate::state::AppState;

pub struct ForgeCompletionProvider {
    state: Entity<AppState>,
    runtime: Arc<ForgeRuntime>,
}

impl ForgeCompletionProvider {
    pub fn new(state: Entity<AppState>, runtime: Arc<ForgeRuntime>) -> Self {
        Self { state, runtime }
    }
}

impl CompletionProvider for ForgeCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut Context<InputState>,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let (line_prefix, line_start) = line_prefix_for_offset(rope, offset);
        if should_skip_completion(&line_prefix) {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }

        let trimmed = line_prefix.trim_end();
        let context = detect_context(trimmed);
        let (token, token_start_in_line) = completion_token(&line_prefix, context);
        let replace_start = line_start.saturating_add(token_start_in_line);
        let replace_range = completion_range(rope, replace_start, offset);

        if context.is_none() && token.is_empty() && (trimmed.is_empty() || trimmed.ends_with('.')) {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }

        let local = match context {
            Some(ContextKind::Collections) => {
                ForgeView::build_collection_suggestions(self.state.read(cx))
            }
            Some(ContextKind::Methods) => ForgeView::build_method_suggestions(),
            Some(ContextKind::Operators) => ForgeView::build_operator_suggestions(),
            None => Vec::new(),
        };

        let completion_prefix = trimmed.to_string();
        let merged_local =
            merge_suggestions(local.clone(), Vec::new(), context, &completion_prefix, &token);
        let local_items = suggestions_to_completion_items(merged_local, &replace_range);

        let Some((session_id, uri, database)) = active_forge_session_info(self.state.read(cx))
        else {
            return Task::ready(Ok(CompletionResponse::Array(local_items)));
        };

        let runtime = self.runtime.clone();
        let runtime_handle = self.state.read(cx).connection_manager().runtime_handle();
        let token = token.clone();
        let context_for_merge = context;
        let completion_prefix = completion_prefix.clone();
        let request_text = completion_prefix.clone();
        cx.background_spawn(async move {
            let result = runtime_handle
                .spawn_blocking(move || {
                    let bridge = runtime.ensure_bridge()?;
                    bridge.ensure_session(session_id, &uri, &database)?;
                    bridge.complete(session_id, &request_text, Duration::from_millis(500))
                })
                .await;

            let completions = match result {
                Ok(Ok(list)) => list,
                Ok(Err(err)) => {
                    log::warn!("Forge completion error: {}", err);
                    Vec::new()
                }
                Err(err) => {
                    log::warn!("Forge completion join error: {}", err);
                    Vec::new()
                }
            };

            let merged = merge_suggestions(
                local,
                completions,
                context_for_merge,
                &completion_prefix,
                &token,
            );
            let items = suggestions_to_completion_items(merged, &replace_range);
            Ok(CompletionResponse::Array(items))
        })
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        if new_text.is_empty() {
            return false;
        }
        if new_text.chars().all(|c| c.is_whitespace()) {
            return false;
        }
        new_text.chars().any(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '$')
    }
}

fn line_prefix_for_offset(rope: &Rope, offset: usize) -> (String, usize) {
    let offset = offset.min(rope.len());
    let position = rope.offset_to_position(offset);
    let line_start = rope.line_start_offset(position.line as usize);
    let line = rope.slice_line(position.line as usize).to_string();
    let prefix_len = offset.saturating_sub(line_start).min(line.len());
    let prefix = line.get(..prefix_len).unwrap_or("").to_string();
    (prefix, line_start)
}

fn completion_range(rope: &Rope, start: usize, end: usize) -> Range {
    let start = start.min(rope.len());
    let end = end.min(rope.len());
    let start_pos = rope.offset_to_position(start);
    let end_pos = rope.offset_to_position(end);
    Range::new(start_pos, end_pos)
}

fn suggestions_to_completion_items(
    suggestions: Vec<Suggestion>,
    replace_range: &Range,
) -> Vec<CompletionItem> {
    suggestions
        .into_iter()
        .map(|suggestion| {
            let kind = match suggestion.kind {
                SuggestionKind::Collection => CompletionItemKind::FIELD,
                SuggestionKind::Method => CompletionItemKind::METHOD,
                SuggestionKind::Operator => CompletionItemKind::OPERATOR,
            };
            CompletionItem {
                label: suggestion.label.clone(),
                kind: Some(kind),
                detail: Some(suggestion.kind.as_str().to_string()),
                text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                    new_text: suggestion.insert_text,
                    insert: *replace_range,
                    replace: *replace_range,
                })),
                ..Default::default()
            }
        })
        .collect()
}

impl ForgeView {
    pub fn make_suggestion(label: &str, kind: SuggestionKind, insert_text: &str) -> Suggestion {
        Suggestion { label: label.to_string(), kind, insert_text: insert_text.to_string() }
    }

    pub fn build_collection_suggestions(state: &AppState) -> Vec<Suggestion> {
        let mut suggestions = Vec::new();

        for (label, template) in [
            ("stats()", "stats()"),
            ("getCollection(\"\")", "getCollection(\"\")"),
            ("getSiblingDB(\"\")", "getSiblingDB(\"\")"),
            ("runCommand({})", "runCommand({})"),
            ("listCollections({})", "listCollections({})"),
            ("createCollection(\"\")", "createCollection(\"\")"),
        ] {
            suggestions.push(Self::make_suggestion(label, SuggestionKind::Method, template));
        }

        if let Some(key) = state.active_forge_tab_key()
            && let Some(conn) = state.active_connection_by_id(key.connection_id)
            && let Some(collections) = conn.collections.get(&key.database)
        {
            for coll in collections {
                suggestions.push(Suggestion {
                    label: coll.clone(),
                    kind: SuggestionKind::Collection,
                    insert_text: coll.clone(),
                });
            }
        }

        suggestions
    }

    pub fn build_method_suggestions() -> Vec<Suggestion> {
        let mut suggestions = Vec::new();

        for method in METHODS {
            if let Some(template) = collection_method_template(method) {
                suggestions.push(Self::make_suggestion(template, SuggestionKind::Method, template));
            } else {
                let insert = format!("{}()", method);
                suggestions.push(Self::make_suggestion(&insert, SuggestionKind::Method, &insert));
            }
        }

        suggestions
    }

    pub fn build_operator_suggestions() -> Vec<Suggestion> {
        OPERATORS
            .iter()
            .map(|op| Suggestion {
                label: op.to_string(),
                kind: SuggestionKind::Operator,
                insert_text: format!("{}: ", op),
            })
            .collect()
    }
}
