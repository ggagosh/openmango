use std::rc::Rc;
use std::sync::atomic::Ordering;

use gpui_kit::component::RopeExt;
use gpui_kit::component::input::{self, EditorState, InputEvent, TabSize};
use gpui_kit::*;
use uuid::Uuid;

use super::logic::statement_bounds;
use crate::helpers::auto_pair::AutoPairState;

use super::ForgeView;
use super::completion::ForgeCompletionProvider;
use super::editor_behavior::{INDENT_WIDTH, WordAction, code_word_boundary, newline_after_opening};
use super::parser::parse_context;
use super::state::ForgeEditorBuffer;
use crate::views::editor_completion::{CompletionScope, EditorCompletionMenu};

impl ForgeView {
    fn create_editor_buffer(
        &mut self,
        tab_id: Uuid,
        content: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor_state = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("javascript")
                .line_number(true)
                .folding(true)
                .tab_size(TabSize { tab_size: INDENT_WIDTH, hard_tabs: false })
                .placeholder("// MongoDB Shell (db.)")
                .default_value(content.clone())
        });
        let completion_menu = cx.new(|cx| {
            EditorCompletionMenu::new(
                &editor_state,
                self.app_state.clone(),
                CompletionScope::Forge(tab_id),
                self.state.editor.completion_request_id.clone(),
                window,
                cx,
            )
        });
        let provider = Rc::new(ForgeCompletionProvider::new(
            self.app_state.clone(),
            self.controller.runtime.clone(),
            self.state.editor.completion_request_id.clone(),
            tab_id,
            editor_state.downgrade(),
            completion_menu.downgrade(),
            window.window_handle(),
        ));
        editor_state.update(cx, |editor, _| {
            editor.lsp_mut().completion_provider = Some(provider.clone());
        });

        let subscription =
            cx.subscribe_in(&editor_state, window, move |this, state, event, window, cx| {
                if let InputEvent::Blur = event {
                    if let Some(buffer) = this.state.editor.buffers.get(&tab_id) {
                        buffer.completion_provider.dismiss(cx);
                    }
                } else if let InputEvent::Change = event {
                    let Some(buffer) = this.state.editor.buffers.get_mut(&tab_id) else {
                        return;
                    };
                    let typed = buffer.completion_provider.take_typed_change();
                    let text = state.read(cx).value().to_string();
                    let cursor = state.read(cx).cursor();
                    // Ordinary typing needs no second JavaScript parse for pairing.
                    let previous_char = text.get(..cursor).and_then(|s| s.chars().next_back());
                    let in_comment = matches!(previous_char, Some('{' | '[' | '(' | '"'))
                        && parse_context(&text, cursor.saturating_sub(1)).in_comment;
                    if typed && buffer.auto_pair.try_auto_pair(state, in_comment, window, cx) {
                        return;
                    }
                    buffer.auto_pair.sync(&text);
                    buffer.content.clone_from(&text);
                    // Save to the editor's own tab, even if focus has already moved.
                    this.app_state.update(cx, |state, _cx| {
                        state.set_forge_tab_content(tab_id, text);
                    });
                }
            });

        self.state.editor.buffers.insert(
            tab_id,
            ForgeEditorBuffer {
                editor_state,
                completion_provider: provider,
                completion_menu,
                _subscription: subscription,
                auto_pair: AutoPairState::new(&content),
                content,
            },
        );
    }

    pub fn accept_completion_or_indent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.accept_completion(window, cx) {
            return;
        }
        window.dispatch_action(Box::new(input::IndentInline), cx);
    }

    fn accept_completion(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(editor) = &self.state.editor.editor_state
            && editor.read(cx).focus_handle(cx).is_focused(window)
            && let Some(buffer) =
                self.state.editor.active_tab_id.and_then(|id| self.state.editor.buffers.get(&id))
        {
            return buffer.completion_menu.update(cx, |menu, cx| menu.accept_selected(window, cx));
        }
        false
    }

    pub fn dismiss_completions(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(buffer) =
            self.state.editor.active_tab_id.and_then(|id| self.state.editor.buffers.get_mut(&id))
        else {
            return false;
        };
        if !buffer.editor_state.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        let dismissed = buffer.completion_provider.dismiss(cx);
        buffer.editor_state.update(cx, |editor, cx| editor.dismiss_lsp_overlays(cx));
        dismissed
    }

    pub fn insert_newline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.accept_completion(window, cx) {
            return;
        }
        let enter = input::Enter { secondary: false, shift: false };
        if let Some(editor) = &self.state.editor.editor_state
            && editor.read(cx).focus_handle(cx).is_focused(window)
        {
            if editor.update(cx, |editor, cx| {
                editor.route_overlay_action(Box::new(enter.clone()), window, cx)
            }) {
                return;
            }
            let source = editor.read(cx).value().to_string();
            let selection = editor.read(cx).selected_range();
            if let Some(edit) = newline_after_opening(&source, selection)
                && !parse_context(&source, edit.range.start.saturating_sub(1)).in_comment
            {
                editor.update(cx, |editor, cx| {
                    let range = editor.text().offset_to_offset_utf16(edit.range.start)
                        ..editor.text().offset_to_offset_utf16(edit.range.end);
                    editor.replace_text_in_range(Some(range), &edit.text, window, cx);
                    editor.set_selected_range(edit.cursor..edit.cursor, cx);
                });
                return;
            }
        }
        window.dispatch_action(Box::new(enter), cx);
    }

    pub fn edit_word(&mut self, action: WordAction, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = &self.state.editor.editor_state
            && editor.read(cx).focus_handle(cx).is_focused(window)
        {
            if let Some(buffer) =
                self.state.editor.active_tab_id.and_then(|id| self.state.editor.buffers.get(&id))
            {
                buffer.completion_provider.dismiss(cx);
            }
            editor.update(cx, |editor, cx| {
                let selection = editor.selected_range();
                let cursor = editor.cursor();
                let target = code_word_boundary(&editor.value(), cursor, action.forward());
                match action {
                    WordAction::DeleteBackward | WordAction::DeleteForward => {
                        editor.dismiss_lsp_overlays(cx);
                        let range = if selection.is_empty() {
                            cursor.min(target)..cursor.max(target)
                        } else {
                            selection
                        };
                        let range = editor.text().offset_to_offset_utf16(range.start)
                            ..editor.text().offset_to_offset_utf16(range.end);
                        editor.replace_text_in_range(Some(range), "", window, cx);
                    }
                    WordAction::SelectBackward | WordAction::SelectForward => {
                        let anchor =
                            if cursor == selection.start { selection.end } else { selection.start };
                        // The native setter preserves reversed selections when end < start.
                        editor.set_selected_range(anchor..target, cx);
                    }
                    WordAction::MoveBackward | WordAction::MoveForward => {
                        editor.set_selected_range(target..target, cx);
                    }
                }
            });
            return;
        }
        let native_action: Box<dyn Action> = match action {
            WordAction::DeleteBackward => Box::new(input::DeleteToPreviousWordStart),
            WordAction::DeleteForward => Box::new(input::DeleteToNextWordEnd),
            WordAction::MoveBackward => Box::new(input::MoveToPreviousWord),
            WordAction::MoveForward => Box::new(input::MoveToNextWord),
            WordAction::SelectBackward => Box::new(input::SelectToPreviousWordStart),
            WordAction::SelectForward => Box::new(input::SelectToNextWordEnd),
        };
        window.dispatch_action(native_action, cx);
    }

    pub fn trigger_completion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self.state.editor.active_tab_id else { return };
        let Some(buffer) = self.state.editor.buffers.get(&tab_id) else { return };
        buffer.completion_provider.trigger(window, cx);
    }

    pub fn navigate_completion(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(buffer) =
            self.state.editor.active_tab_id.and_then(|id| self.state.editor.buffers.get(&id))
            && buffer.editor_state.read(cx).focus_handle(cx).is_focused(window)
            && buffer.completion_menu.update(cx, |menu, cx| menu.navigate(delta, window, cx))
        {
            return;
        }
        let action: Box<dyn Action> =
            if delta < 0 { Box::new(input::MoveUp) } else { Box::new(input::MoveDown) };
        window.dispatch_action(action, cx);
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

    pub fn sync_active_tab_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active_id = self.app_state.read(cx).active_forge_tab_id();
        let switched = active_id != self.state.editor.active_tab_id;
        if switched {
            self.state.editor.completion_request_id.fetch_add(1, Ordering::AcqRel);
            if let Some(buffer) =
                self.state.editor.active_tab_id.and_then(|id| self.state.editor.buffers.get(&id))
            {
                buffer.completion_provider.dismiss(cx);
            }
            if let Some(editor) = &self.state.editor.editor_state {
                editor.update(cx, |editor, cx| editor.dismiss_lsp_overlays(cx));
            }
            self.state.editor.active_tab_id = active_id;
            self.state.editor.editor_focus_requested = true;
        }
        let Some(active_id) = active_id else {
            self.state.editor.editor_state = None;
            return;
        };

        let stored = self.app_state.read(cx).forge_tab_content(active_id).unwrap_or("");
        let created = !self.state.editor.buffers.contains_key(&active_id);
        let externally_changed = self
            .state
            .editor
            .buffers
            .get(&active_id)
            .is_some_and(|buffer| buffer.content != stored);
        let content = (created || externally_changed).then(|| stored.to_string());
        if created {
            self.create_editor_buffer(active_id, content.clone().unwrap(), window, cx);
        }
        let buffer = self.state.editor.buffers.get_mut(&active_id).unwrap();
        let editor_state = buffer.editor_state.clone();
        if externally_changed && let Some(content) = &content {
            buffer.completion_provider.dismiss(cx);
            buffer.content.clone_from(content);
            buffer.auto_pair.sync(content);
            // Loading a saved query is an edit; changing tabs is not.
            editor_state.update(cx, |editor, cx| editor.replace_all(content.clone(), window, cx));
        }
        self.state.editor.editor_state = Some(editor_state.clone());
        if created || externally_changed || switched {
            let pending_cursor = self
                .app_state
                .update(cx, |state, _cx| state.take_forge_tab_pending_cursor(active_id));
            if let Some(offset) = pending_cursor {
                editor_state.update(cx, |editor, cx| {
                    editor.set_selected_range(offset..offset, cx);
                });
            }
        }
        if let Some(content) = content {
            self.warm_up_schema(&content, cx);
        }
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

        if let Some(buffer) =
            self.state.editor.active_tab_id.and_then(|id| self.state.editor.buffers.get(&id))
        {
            buffer.completion_provider.schedule_schema_sample(&collection, cx);
        }
    }
}
