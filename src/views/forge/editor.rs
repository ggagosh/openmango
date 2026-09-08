use std::rc::Rc;

use gpui_kit::component::RopeExt;
use gpui_kit::component::input::{EditorState, InputEvent, TabSize};
use gpui_kit::*;

use super::logic::statement_bounds;

use super::ForgeView;
use super::completion::ForgeCompletionProvider;
use super::parser::parse_context;

impl ForgeView {
    pub fn ensure_editor_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.editor.editor_state.is_some() {
            return;
        }

        let provider = Rc::new(ForgeCompletionProvider::new(
            self.app_state.clone(),
            self.controller.runtime.clone(),
            self.state.editor.completion_request_id.clone(),
        ));

        let editor_state = cx.new(|cx| {
            let mut editor = EditorState::new(window, cx)
                .language("javascript")
                .line_number(true)
                .tab_size(TabSize { tab_size: 2, hard_tabs: false })
                .placeholder("// MongoDB Shell (db.)");

            editor.lsp_mut().completion_provider = Some(provider.clone());
            editor
        });

        let subscription =
            cx.subscribe_in(&editor_state, window, move |this, state, event, window, cx| {
                if let InputEvent::Change = event {
                    if this.try_auto_pair(state, window, cx) {
                        return;
                    }
                    let text = state.read(cx).value().to_string();
                    this.handle_editor_change(&text, cx);
                }
            });

        self.state.editor.editor_state = Some(editor_state);
        self.state.editor.editor_subscription = Some(subscription);
        self.state.editor.completion_provider = Some(provider);
    }

    pub fn save_current_content(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.state.editor.active_tab_id else {
            return;
        };
        let Some(editor_state) = &self.state.editor.editor_state else {
            return;
        };
        let text = editor_state.read(cx).value().to_string();
        self.state.editor.current_text = text.clone();
        self.state.editor.auto_pair.sync(&text);
        self.app_state.update(cx, |state, _cx| {
            state.set_forge_tab_content(tab_id, text);
        });
    }

    pub fn handle_execute_selection_or_statement(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = self.editor_selection_text(window, cx) {
            self.handle_execute_query(&selection, cx);
            return;
        }

        if let Some(statement) = self.editor_statement_at_cursor(cx) {
            self.handle_execute_query(&statement, cx);
        }
    }

    fn editor_selection_text(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<String> {
        let editor_state = self.state.editor.editor_state.as_ref()?;
        editor_state.update(cx, |state, cx| {
            let selection = state.selected_text_range(true, window, cx)?;
            if selection.range.start == selection.range.end {
                return None;
            }
            let mut adjusted = None;
            let text = state.text_for_range(selection.range.clone(), &mut adjusted, window, cx)?;
            let trimmed = text.trim();
            if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
        })
    }

    fn editor_statement_at_cursor(&self, cx: &mut Context<Self>) -> Option<String> {
        let editor_state = self.state.editor.editor_state.as_ref()?;
        let text = editor_state.read(cx).text().to_string();
        let cursor = editor_state.read(cx).cursor().min(text.len());
        let (start, end) = statement_bounds(&text, cursor);
        let snippet = text.get(start..end)?.trim();
        if snippet.is_empty() { None } else { Some(snippet.to_string()) }
    }

    pub fn sync_active_tab_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        force: bool,
    ) {
        let active_id = self.app_state.read(cx).active_forge_tab_id();
        let same_tab = active_id == self.state.editor.active_tab_id;
        if !force && same_tab {
            let stored = active_id
                .and_then(|id| self.app_state.read(cx).forge_tab_content(id))
                .unwrap_or("");
            if stored == self.state.editor.current_text {
                return;
            }
        } else {
            self.save_current_content(cx);
            self.state.editor.active_tab_id = active_id;
        }
        let Some(active_id) = active_id else {
            return;
        };

        let content =
            self.app_state.read(cx).forge_tab_content(active_id).unwrap_or("").to_string();

        self.state.editor.current_text = content.clone();
        self.state.editor.auto_pair.sync(&content);
        if let Some(editor_state) = &self.state.editor.editor_state {
            editor_state.update(cx, |editor, cx| {
                editor.set_value(content.clone(), window, cx);
            });
            let pending_cursor = self
                .app_state
                .update(cx, |state, _cx| state.take_forge_tab_pending_cursor(active_id));
            if let Some(offset) = pending_cursor {
                editor_state.update(cx, |editor, cx| {
                    let safe_offset = offset.min(editor.text().len());
                    let position = editor.text().offset_to_position(safe_offset);
                    editor.set_cursor_position(position, window, cx);
                });
            }
        }

        // Schema warm-up: parse content to find collection, pre-fetch schema fields
        self.warm_up_schema(&content, cx);
    }

    fn warm_up_schema(&self, content: &str, cx: &mut Context<Self>) {
        if content.is_empty() {
            return;
        }

        let ctx = parse_context(content, content.len());
        let Some(collection) = ctx.collection else {
            return;
        };

        let needs_fetch = {
            let state_ref = self.app_state.read(cx);
            let Some(tab_key) = state_ref.active_forge_tab_key() else {
                return;
            };
            let session_key = crate::state::SessionKey::new(
                tab_key.connection_id,
                tab_key.database.clone(),
                collection.clone(),
            );
            state_ref.forge_schema_stale(&session_key)
        };

        if !needs_fetch {
            return;
        }

        let provider = self.state.editor.completion_provider.clone();
        let editor_state = self.state.editor.editor_state.clone();
        if let (Some(provider), Some(editor_state)) = (provider, editor_state) {
            editor_state.update(cx, |_editor, cx| {
                provider.schedule_schema_sample(&collection, cx);
            });
        }
    }

    pub fn handle_editor_change(&mut self, text: &str, cx: &mut Context<Self>) {
        self.state.editor.current_text = text.to_string();
        self.state.editor.auto_pair.sync(text);
        if let Some(tab_id) = self.state.editor.active_tab_id {
            let content = self.state.editor.current_text.clone();
            self.app_state.update(cx, |state, _cx| {
                state.set_forge_tab_content(tab_id, content);
            });
        }
    }

    fn try_auto_pair(
        &mut self,
        state: &gpui_kit::Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = state.read(cx).value().to_string();
        let cursor = state.read(cx).cursor();
        let in_comment = if cursor > 0 && cursor <= current.len() {
            parse_context(&current, cursor.saturating_sub(1)).in_comment
        } else {
            false
        };
        self.state.editor.auto_pair.try_auto_pair(state, in_comment, window, cx)
    }
}
