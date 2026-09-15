use std::ops::Range;

use gpui_kit::component::RopeExt;
use gpui_kit::component::input::EditorState;
use gpui_kit::*;

use crate::views::forge::editor_behavior::wrap_closing;

pub struct AutoPairState {
    previous_text: String,
    guard: bool,
}

impl AutoPairState {
    pub fn new(initial_text: &str) -> Self {
        Self { previous_text: initial_text.to_string(), guard: false }
    }

    /// Call on InputEvent::Change. Returns true if a typed opener wrapped the
    /// replaced selection. Plain pairing is left to the editor's `auto_close`.
    pub fn try_auto_pair(
        &mut self,
        state: &Entity<EditorState>,
        in_string_or_comment: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if self.guard {
            self.guard = false;
            return false;
        }

        let current = state.read(cx).value().to_string();
        let Some((prev_range, current_range)) = diff_ranges(&self.previous_text, &current) else {
            return false;
        };
        let (Some(selected_text), Some(inserted_text)) =
            (self.previous_text.get(prev_range.clone()), current.get(current_range.clone()))
        else {
            return false;
        };
        let mut chars = inserted_text.chars();
        let (Some(open), None) = (chars.next(), chars.next()) else {
            return false;
        };
        if selected_text.is_empty() {
            return false;
        }
        let Some(close) = wrap_closing(open, in_string_or_comment) else {
            return false;
        };

        let replacement = format!("{open}{selected_text}{close}");
        let cursor_offset = current_range.start + 1 + selected_text.len();
        self.guard = true;
        state.update(cx, |input, cx| {
            let utf16_range = byte_range_to_utf16(input.text(), &current_range);
            input.replace_text_in_range(Some(utf16_range), &replacement, window, cx);
            let position = input.text().offset_to_position(cursor_offset);
            input.set_cursor_position(position, window, cx);
        });
        true
    }

    /// Sync tracked text after external changes (set_value, etc.)
    pub fn sync(&mut self, text: &str) {
        self.previous_text = text.to_string();
    }
}

/// Convert a byte-offset range to a UTF-16 offset range for EditorState methods.
fn byte_range_to_utf16(text: &impl RopeExt, range: &Range<usize>) -> Range<usize> {
    text.offset_to_offset_utf16(range.start)..text.offset_to_offset_utf16(range.end)
}

/// Find the first differing range between two strings.
pub fn diff_ranges(previous: &str, current: &str) -> Option<(Range<usize>, Range<usize>)> {
    if previous == current {
        return None;
    }

    let prev_bytes = previous.as_bytes();
    let curr_bytes = current.as_bytes();
    let mut start = 0;
    let min_len = prev_bytes.len().min(curr_bytes.len());
    while start < min_len && prev_bytes[start] == curr_bytes[start] {
        start += 1;
    }

    let mut prev_end = prev_bytes.len();
    let mut curr_end = curr_bytes.len();
    while prev_end > start
        && curr_end > start
        && prev_bytes[prev_end - 1] == curr_bytes[curr_end - 1]
    {
        prev_end -= 1;
        curr_end -= 1;
    }

    // Shared UTF-8 prefixes/suffixes can end inside a code point.
    while start > 0 && (!previous.is_char_boundary(start) || !current.is_char_boundary(start)) {
        start -= 1;
    }
    while prev_end < previous.len()
        && (!previous.is_char_boundary(prev_end) || !current.is_char_boundary(curr_end))
    {
        prev_end += 1;
        curr_end += 1;
    }

    Some((start..prev_end, start..curr_end))
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::Root;
    use gpui_kit::component::input::{Editor, EditorState, InputEvent};
    use gpui_kit::{
        AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _, Render,
        Styled as _, Subscription, TestAppContext, Window, div,
    };

    use super::{AutoPairState, diff_ranges};

    struct Host {
        state: Entity<EditorState>,
        pair: AutoPairState,
        _subscription: Subscription,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(Editor::new(&self.state))
        }
    }

    /// The editor's own pairing and our selection wrapping must not both fire.
    #[gpui_kit::test]
    fn editor_pairs_once_and_typed_openers_wrap_selections(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut state = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let host = cx.new(|cx| {
                let editor = cx.new(|cx| EditorState::new(window, cx).language("javascript"));
                let _subscription = cx.subscribe_in(
                    &editor,
                    window,
                    |host: &mut Host, editor, event, window, cx| {
                        if matches!(event, InputEvent::Change)
                            && !host.pair.try_auto_pair(editor, false, window, cx)
                        {
                            host.pair.sync(&editor.read(cx).value());
                        }
                    },
                );
                state = Some(editor.clone());
                Host { state: editor, pair: AutoPairState::new(""), _subscription }
            });
            Root::new(host, window, cx)
        });
        let state = state.unwrap();
        cx.update(|window, cx| window.focus(&state.focus_handle(cx), cx));
        cx.run_until_parked();
        let value = |cx: &mut gpui_kit::VisualTestContext| {
            state.read_with(cx, |editor, _| editor.value().to_string())
        };

        cx.simulate_input("{");
        cx.run_until_parked();
        assert_eq!(value(cx), "{}");
        cx.simulate_input("}");
        cx.run_until_parked();
        assert_eq!(value(cx), "{}");

        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" });
        cx.simulate_input("a: 1");
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" });
        cx.simulate_input("{");
        cx.run_until_parked();
        assert_eq!(value(cx), "{a: 1}");
    }

    #[test]
    fn changed_ranges_replace_complete_unicode_characters() {
        for (before, after) in [("до\n", "да\n"), ("x😀y", "x😃y"), ("旧行\n次行\n", "次行\n")]
        {
            let (old, new) = diff_ranges(before, after).unwrap();
            let rebuilt = format!("{}{}{}", &before[..old.start], &after[new], &before[old.end..]);
            assert_eq!(rebuilt, after);
        }
    }
}
