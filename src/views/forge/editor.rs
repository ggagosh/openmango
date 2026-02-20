use std::rc::Rc;

use gpui::*;
use gpui_component::RopeExt;
use gpui_component::input::{InputEvent, InputState, TabSize};

use super::logic::statement_bounds;

use super::ForgeView;
use super::completion::ForgeCompletionProvider;
use super::editor_behavior::{IndentConfig, IndentResult, indent_after_enter};
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
            let mut editor = InputState::new(window, cx)
                .code_editor("javascript")
                .auto_indent(false)
                .line_number(true)
                .tab_size(TabSize { tab_size: 2, hard_tabs: false })
                .placeholder("// MongoDB Shell (db.)");

            editor.lsp.completion_provider = Some(provider.clone());
            editor
        });

        let subscription = cx.subscribe_in(
            &editor_state,
            window,
            move |this, state, event, window, cx| match event {
                InputEvent::Change => {
                    if this.try_auto_pair(state, window, cx) {
                        return;
                    }
                    let text = state.read(cx).value().to_string();
                    this.handle_editor_change(&text, cx);
                }
                InputEvent::PressEnter { secondary: false } => {
                    let mut adjusted = false;
                    state.update(cx, |state, cx| {
                        adjusted = apply_custom_indent(state, window, cx);
                    });
                    if adjusted {
                        cx.notify();
                    }
                }
                _ => {}
            },
        );

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
        if !force && active_id == self.state.editor.active_tab_id {
            return;
        }
        self.save_current_content(cx);

        self.state.editor.active_tab_id = active_id;
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
        state: &gpui::Entity<InputState>,
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

fn apply_custom_indent(
    state: &mut InputState,
    window: &mut Window,
    cx: &mut Context<InputState>,
) -> bool {
    let text = state.value().to_string();
    let cursor = state.cursor();
    if cursor == 0 || cursor > text.len() {
        return false;
    }

    let bytes = text.as_bytes();
    if bytes[cursor - 1] != b'\n' {
        return false;
    }

    // Read indent config from editor state
    let config = IndentConfig {
        width: state.current_tab_size().tab_size,
        use_tabs: state.current_tab_size().hard_tabs,
    };

    let mut prev_non_ws = None;
    let mut idx = cursor - 1;
    while idx > 0 {
        idx -= 1;
        let ch = bytes[idx];
        if !ch.is_ascii_whitespace() {
            prev_non_ws = Some((idx, ch));
            break;
        }
    }

    let mut next_non_ws = None;
    let mut j = cursor;
    while j < bytes.len() {
        let ch = bytes[j];
        if !ch.is_ascii_whitespace() {
            next_non_ws = Some((j, ch));
            break;
        }
        j += 1;
    }

    let mut base_line_end = cursor - 1;
    while base_line_end > 0 && bytes[base_line_end - 1] == b'\n' {
        base_line_end -= 1;
    }

    let mut base_line_start = base_line_end;
    while base_line_start > 0 && bytes[base_line_start - 1] != b'\n' {
        base_line_start -= 1;
    }

    // If the previous line is empty, walk back to find a non-empty line.
    while base_line_start < base_line_end
        && text[base_line_start..base_line_end].trim().is_empty()
        && base_line_start > 0
    {
        let mut scan = base_line_start - 1;
        while scan > 0 && bytes[scan - 1] != b'\n' {
            scan -= 1;
        }
        base_line_end = base_line_start - 1;
        base_line_start = scan;
    }

    let mut indent_end = base_line_start;
    while indent_end < bytes.len() {
        let ch = bytes[indent_end];
        if ch == b'\n' || !ch.is_ascii_whitespace() {
            break;
        }
        indent_end += 1;
    }
    let base_indent = text.get(base_line_start..indent_end).unwrap_or("");

    let prev_char = prev_non_ws.map(|(_, ch)| ch as char);
    let next_char = next_non_ws.map(|(_, ch)| ch as char);

    let result = indent_after_enter(prev_char, next_char, base_indent, &config);

    match result {
        IndentResult::None => false,
        IndentResult::Simple(indent) => {
            // Replace any auto-inserted horizontal whitespace after the newline.
            let mut ws_end = cursor;
            while ws_end < bytes.len() && matches!(bytes[ws_end], b' ' | b'\t') {
                ws_end += 1;
            }
            let range = state.text().offset_to_offset_utf16(cursor)
                ..state.text().offset_to_offset_utf16(ws_end);
            state.replace_text_in_range(Some(range), &indent, window, cx);
            let position = state.text().offset_to_position(cursor + indent.len());
            state.set_cursor_position(position, window, cx);
            true
        }
        IndentResult::BetweenBraces { inner, outer } => {
            let next_idx = next_non_ws.map(|(idx, _)| idx).unwrap_or(cursor);
            // Normalize the between-braces region to exactly one inner line.
            debug_assert!(cursor >= 1, "between-braces indent requires newline at cursor - 1");
            let start = cursor - 1; // include the newline inserted by Enter
            let insertion = format!("\n{inner}\n{outer}");
            let range = state.text().offset_to_offset_utf16(start)
                ..state.text().offset_to_offset_utf16(next_idx);
            state.replace_text_in_range(Some(range), &insertion, window, cx);
            let position = state.text().offset_to_position(start + 1 + inner.len());
            state.set_cursor_position(position, window, cx);
            true
        }
    }
}
