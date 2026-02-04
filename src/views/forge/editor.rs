use std::rc::Rc;

use gpui::*;
use gpui_component::RopeExt;
use gpui_component::input::{InputEvent, InputState, TabSize};

use super::logic::statement_bounds;

use super::ForgeView;
use super::completion::ForgeCompletionProvider;

impl ForgeView {
    pub fn ensure_editor_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_state.is_some() {
            return;
        }

        let provider =
            Rc::new(ForgeCompletionProvider::new(self.state.clone(), self.runtime.clone()));

        let editor_state = cx.new(|cx| {
            let mut editor = InputState::new(window, cx)
                .code_editor("javascript")
                .line_number(true)
                .tab_size(TabSize { tab_size: 2, hard_tabs: false })
                .placeholder("// MongoDB Shell\ndb.");

            editor.lsp.completion_provider = Some(provider.clone());
            editor
        });

        let subscription = cx.subscribe_in(
            &editor_state,
            window,
            move |this, state, event, window, cx| match event {
                InputEvent::Change => {
                    let text = state.read(cx).value().to_string();
                    this.handle_editor_change(&text, cx);
                }
                InputEvent::PressEnter { .. } => {
                    let mut adjusted = false;
                    state.update(cx, |state, cx| {
                        adjusted = auto_indent_between_braces(state, window, cx);
                    });
                    if adjusted {
                        cx.notify();
                    }
                }
                _ => {}
            },
        );

        self.editor_state = Some(editor_state);
        self.editor_subscription = Some(subscription);
        self.completion_provider = Some(provider);
    }

    pub fn save_current_content(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.active_tab_id else {
            return;
        };
        let Some(editor_state) = &self.editor_state else {
            return;
        };
        let text = editor_state.read(cx).value().to_string();
        self.current_text = text.clone();
        self.state.update(cx, |state, _cx| {
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
        let editor_state = self.editor_state.as_ref()?;
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
        let editor_state = self.editor_state.as_ref()?;
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
        let active_id = self.state.read(cx).active_forge_tab_id();
        if !force && active_id == self.active_tab_id {
            return;
        }
        self.save_current_content(cx);

        self.active_tab_id = active_id;
        let Some(active_id) = active_id else {
            return;
        };

        let content = self.state.read(cx).forge_tab_content(active_id).unwrap_or("").to_string();

        self.current_text = content.clone();
        if let Some(editor_state) = &self.editor_state {
            editor_state.update(cx, |editor, cx| {
                editor.set_value(content.clone(), window, cx);
            });
        }
    }

    pub fn handle_editor_change(&mut self, text: &str, cx: &mut Context<Self>) {
        self.current_text = text.to_string();
        if let Some(tab_id) = self.active_tab_id {
            let content = self.current_text.clone();
            self.state.update(cx, |state, _cx| {
                state.set_forge_tab_content(tab_id, content);
            });
        }
    }
}

fn auto_indent_between_braces(
    state: &mut InputState,
    window: &mut Window,
    cx: &mut Context<InputState>,
) -> bool {
    let text = state.value().to_string();
    let cursor = state.cursor();
    if cursor >= text.len() {
        return false;
    }

    let bytes = text.as_bytes();
    let close = bytes[cursor];
    let open = match close {
        b'}' => b'{',
        b']' => b'[',
        _ => return false,
    };

    let mut line_start = cursor;
    while line_start > 0 {
        if bytes[line_start - 1] == b'\n' {
            break;
        }
        line_start -= 1;
    }

    let current_indent = &text[line_start..cursor];
    if !current_indent.chars().all(|c| c.is_whitespace()) {
        return false;
    }

    let mut idx = line_start;
    let mut prev_non_ws = None;
    while idx > 0 {
        idx -= 1;
        let ch = bytes[idx];
        if !ch.is_ascii_whitespace() {
            prev_non_ws = Some(ch);
            break;
        }
    }

    if prev_non_ws != Some(open) {
        return false;
    }

    let indent = current_indent.to_string();
    let newline = format!("\n{}\n{}", indent, indent);
    let range = state.cursor()..state.cursor();
    state.replace_text_in_range(Some(range), &newline, window, cx);
    let new_cursor_offset = cursor + 1 + indent.len();
    let position = state.text().offset_to_position(new_cursor_offset);
    state.set_cursor_position(position, window, cx);
    true
}
