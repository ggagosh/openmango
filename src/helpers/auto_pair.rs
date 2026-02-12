use std::ops::Range;

use gpui::*;
use gpui_component::RopeExt;
use gpui_component::input::InputState;

use crate::views::forge::editor_behavior::{PairAction, pair_action};

pub struct AutoPairState {
    previous_text: String,
    guard: bool,
}

impl AutoPairState {
    pub fn new(initial_text: &str) -> Self {
        Self { previous_text: initial_text.to_string(), guard: false }
    }

    /// Call on InputEvent::Change. Returns true if auto-pair was applied.
    pub fn try_auto_pair(
        &mut self,
        state: &Entity<InputState>,
        in_string_or_comment: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if self.guard {
            self.guard = false;
            return false;
        }

        let current = state.read(cx).value().to_string();
        let cursor = state.read(cx).cursor();
        if cursor == 0 || cursor > current.len() {
            return false;
        }

        let Some((prev_range, current_range)) = diff_ranges(&self.previous_text, &current) else {
            return false;
        };
        let Some(inserted_text) = current.get(current_range.clone()) else {
            return false;
        };
        let mut chars = inserted_text.chars();
        let inserted_char = match (chars.next(), chars.next()) {
            (Some(ch), None) => ch,
            _ => return false,
        };

        let char_after_cursor = current.get(cursor..).and_then(|s| s.chars().next());
        let has_selection = !prev_range.is_empty() && current_range.len() == 1;

        let action =
            pair_action(inserted_char, char_after_cursor, in_string_or_comment, has_selection);

        match action {
            PairAction::Skip => false,
            PairAction::Overtype => {
                // Undo the just-inserted char and move cursor past the existing closing char.
                self.guard = true;
                state.update(cx, |input, cx| {
                    input.replace_text_in_range(Some(current_range.clone()), "", window, cx);
                    let new_cursor = current_range.start + 1;
                    let position = input.text().offset_to_position(new_cursor);
                    input.set_cursor_position(position, window, cx);
                });
                true
            }
            PairAction::WrapSelection(open, close) => {
                let Some(selected_text) = self.previous_text.get(prev_range.clone()) else {
                    return false;
                };
                let replacement = format!("{}{}{}", open, selected_text, close);
                let cursor_offset = current_range.start + 1 + selected_text.len();
                self.guard = true;
                state.update(cx, |input, cx| {
                    input.replace_text_in_range(Some(current_range), &replacement, window, cx);
                    let position = input.text().offset_to_position(cursor_offset);
                    input.set_cursor_position(position, window, cx);
                });
                true
            }
            PairAction::InsertClosing(close) => {
                if current.len() != self.previous_text.len() + 1 {
                    return false;
                }
                self.guard = true;
                state.update(cx, |input, cx| {
                    let range = cursor..cursor;
                    let text = close.to_string();
                    input.replace_text_in_range(Some(range), &text, window, cx);
                    let position = input.text().offset_to_position(cursor);
                    input.set_cursor_position(position, window, cx);
                });
                true
            }
        }
    }

    /// Sync tracked text after external changes (set_value, etc.)
    pub fn sync(&mut self, text: &str) {
        self.previous_text = text.to_string();
    }
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

    Some((start..prev_end, start..curr_end))
}
