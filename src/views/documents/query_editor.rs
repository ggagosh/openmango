use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::component::RopeExt as _;
use gpui_kit::component::input::{CompletionProvider, EditorState, Rope, TabSize};
use gpui_kit::*;
use lsp_types::{CompletionContext, CompletionResponse};

use crate::views::editor_completion::EditorCompletionMenu;
use crate::views::forge::editor_behavior::newline_after_opening;

use super::CollectionView;
use super::query_completion::{QueryCompletionProvider, is_query_input_in_string_or_comment};

pub(super) fn new_query_editor(
    window: &mut Window,
    cx: &mut Context<EditorState>,
    placeholder: &'static str,
) -> EditorState {
    EditorState::new(window, cx)
        .language("javascript")
        .line_number(false)
        .folding(false)
        .soft_wrap(false)
        // Inline query controls must not scroll their only line into blank editor space.
        .scroll_beyond_last_line(Some(0))
        .cursor_surrounding_lines(Some(0))
        .tab_size(TabSize { tab_size: 2, hard_tabs: false })
        .submit_on_enter(true)
        .placeholder(placeholder)
}

pub(super) fn format_query_editor(
    editor: &Entity<EditorState>,
    window: &mut Window,
    cx: &mut App,
) -> String {
    editor.update(cx, |input, cx| {
        let raw = input.value().to_string();
        let selection = input.selected_range();
        let reversed = input.cursor() == selection.start;
        let Some((formatted, selection)) = super::query_format::format_query(&raw, selection)
        else {
            return raw;
        };
        let scroll = input.scroll_offset();
        input.replace_all(formatted.clone(), window, cx);
        input.set_selected_range(
            if reversed { selection.end..selection.start } else { selection },
            cx,
        );
        input.set_scroll_offset(scroll, cx);
        formatted
    })
}

pub(super) fn correct_query_pointer(
    editor: &Entity<EditorState>,
    event: &MouseDownEvent,
    window: &mut Window,
    cx: &mut App,
) {
    if event.button != MouseButton::Left
        || event.click_count != 1
        || event.modifiers.control
        || event.modifiers.secondary()
        || event.modifiers.alt
    {
        return;
    }
    // gpui-pre 0.3.4 rounds every click after the final glyph's leading edge to
    // the line end. Use native IME bounds for that glyph, not guessed font widths.
    // This is only for our unwrapped, non-folding query inputs; remove when fixed upstream.
    let correction = editor.update(cx, |input, cx| {
        let line_height = input.line_height()?;
        let bounds = input.input_bounds();
        if !bounds.contains(&event.position) {
            return None;
        }
        let scroll = input.scroll_offset();
        let row = ((event.position.y - bounds.top() - scroll.y) / line_height).floor();
        if row < 0.0 || row as usize >= input.text().lines_len() {
            return None;
        }
        let row = row as usize;
        let start = input.text().line_start_offset(row);
        let line_end = input.text().line_end_offset(row);
        let line = input.text().slice(start..line_end).to_string();
        let line = line.trim_end_matches('\r');
        let last = line.chars().next_back()?;
        let end = start + line.len();
        let last_start = end - last.len_utf8();
        let range = input.text().offset_to_offset_utf16(last_start)
            ..input.text().offset_to_offset_utf16(end);
        let glyph = input.bounds_for_range(
            range,
            Bounds::new(bounds.origin + scroll, bounds.size),
            window,
            cx,
        )?;
        if glyph.size.width <= px(0.)
            || !glyph.contains(&event.position)
            || event.position.x >= glyph.center().x
        {
            return None;
        }
        let selected = input.selected_range();
        let anchor = if input.cursor() == selected.start { selected.end } else { selected.start };
        Some((input.text().clone(), line_end, last_start, anchor))
    });
    let Some((source, native_end, target, anchor)) = correction else { return };
    let editor = editor.downgrade();
    let shift = event.modifiers.shift;
    let handle = window.window_handle();
    // Preserve native focus, click-count handling, and drag-selection setup.
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            if let Some(editor) = editor.upgrade() {
                editor.update(cx, |input, cx| {
                    if input.focus_handle(cx).is_focused(window)
                        && input.text() == &source
                        && input.cursor() == native_end
                    {
                        input.set_selected_range(
                            if shift { anchor..target } else { target..target },
                            cx,
                        );
                    }
                });
            }
        });
    });
}

#[derive(Clone)]
pub(crate) struct QueryEditorCompletions {
    provider: QueryCompletionProvider,
    editor: WeakEntity<EditorState>,
    menu: WeakEntity<EditorCompletionMenu>,
    generation: Arc<AtomicU64>,
    window: AnyWindowHandle,
    typed_change: Rc<Cell<bool>>,
    suppress_next_change: Rc<Cell<bool>>,
}

impl QueryEditorCompletions {
    pub fn new(
        provider: QueryCompletionProvider,
        editor: &Entity<EditorState>,
        menu: &Entity<EditorCompletionMenu>,
        generation: Arc<AtomicU64>,
        window: &Window,
    ) -> Self {
        Self {
            provider,
            editor: editor.downgrade(),
            menu: menu.downgrade(),
            generation,
            window: window.window_handle(),
            typed_change: Rc::new(Cell::new(false)),
            suppress_next_change: Rc::new(Cell::new(false)),
        }
    }

    pub fn take_typed_change(&self) -> bool {
        self.typed_change.replace(false)
    }

    pub fn take_suppressed_change(&self) -> bool {
        self.suppress_next_change.replace(false)
    }

    pub fn dismiss(&self, cx: &mut App) -> bool {
        self.menu.upgrade().is_some_and(|menu| menu.update(cx, |menu, cx| menu.dismiss(cx)))
    }

    pub fn trigger(&self, cx: &mut App) {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let this = self.clone();
        // Input hooks run under an editor update lease. Read the final text afterwards.
        cx.defer(move |cx| {
            let _ = this.window.update(cx, |_, window, cx| {
                if this.generation.load(Ordering::Acquire) != generation {
                    return;
                }
                let (Some(editor), Some(menu)) = (this.editor.upgrade(), this.menu.upgrade())
                else {
                    return;
                };
                if !editor.read(cx).focus_handle(cx).is_focused(window)
                    || !menu.read(cx).is_active(cx)
                {
                    return;
                }
                let source = editor.read(cx).text().clone();
                let cursor = editor.read(cx).cursor();
                let items = this.provider.items(&source, cursor, cx);
                menu.update(cx, |menu, cx| {
                    menu.present(generation, source, cursor, items, window, cx)
                });
            });
        });
    }
}

impl CompletionProvider for QueryEditorCompletions {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        Task::ready(Ok(CompletionResponse::Array(self.provider.items(rope, offset, cx))))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, cx: &mut App) -> bool {
        self.typed_change.set(true);
        if new_text.is_empty()
            || new_text.chars().any(|ch| {
                ch.is_alphanumeric()
                    || matches!(ch, '_' | '$' | '.' | ':' | '"' | '\'' | '{' | '[' | ' ')
            })
        {
            self.trigger(cx);
        } else {
            self.dismiss(cx);
        }
        // Use the shared atomic completion menu instead of the toolkit's deferred one.
        false
    }
}

impl CollectionView {
    pub(crate) fn handle_query_editor_key(
        &mut self,
        key: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(editor) = self.filter_state.clone() else { return false };
        let command = key.modifiers.secondary() || key.modifiers.control;
        let key_name = key.key.to_ascii_lowercase();
        let newline = key.modifiers.shift && matches!(key_name.as_str(), "enter" | "return");
        if ((command && matches!(key_name.as_str(), "z" | "y" | "x" | "v")) || newline)
            && let Some(provider) = &self.filter_completions
        {
            provider.dismiss(cx);
            provider.suppress_next_change.set(newline || matches!(key_name.as_str(), "z" | "y"));
        }
        if key_name == "escape" {
            if let Some(provider) = &self.filter_completions {
                provider.dismiss(cx);
            }
            self.calendar_open = false;
            cx.notify();
            return true;
        }
        if command && key_name == "space" {
            if let Some(provider) = &self.filter_completions {
                provider.trigger(cx);
            }
            return true;
        }
        if !command && !key.modifiers.alt {
            if let Some(menu) = &self.filter_completion_menu {
                let handled = menu.update(cx, |menu, cx| match key_name.as_str() {
                    "tab" | "enter" | "return" if !key.modifiers.shift => {
                        menu.accept_selected(window, cx)
                    }
                    "up" if !key.modifiers.shift => menu.navigate(-1, window, cx),
                    "down" if !key.modifiers.shift => menu.navigate(1, window, cx),
                    _ => false,
                });
                if handled {
                    return true;
                }
            }
            if matches!(key_name.as_str(), "enter" | "return") && key.modifiers.shift {
                let text = editor.read(cx).value().to_string();
                let selection = editor.read(cx).selected_range();
                if !is_query_input_in_string_or_comment(&text, selection.start)
                    && let Some(edit) = newline_after_opening(&text, selection)
                {
                    editor.update(cx, |editor, cx| {
                        editor.set_selected_range(edit.range, cx);
                        editor.replace(edit.text, window, cx);
                        editor.set_selected_range(edit.cursor..edit.cursor, cx);
                    });
                    return true;
                }
            }
        }
        false
    }
}
