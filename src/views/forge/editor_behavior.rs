//! Word movement and selection wrapping for MongoDB query editors.

use std::sync::OnceLock;

use regex::Regex;

pub const INDENT_WIDTH: usize = 2;

#[derive(Clone, Copy)]
pub enum WordAction {
    MoveBackward,
    MoveForward,
    SelectBackward,
    SelectForward,
    DeleteBackward,
    DeleteForward,
}

impl WordAction {
    pub fn forward(self) -> bool {
        matches!(self, Self::MoveForward | Self::SelectForward | Self::DeleteForward)
    }
}

/// JavaScript identifiers are words; member-access dots and other punctuation
/// separate them. Unicode word characters keep combining marks with their name.
pub fn code_word_boundary(source: &str, cursor: usize, forward: bool) -> usize {
    static WORDS: OnceLock<Regex> = OnceLock::new();
    let words = WORDS.get_or_init(|| Regex::new(r"[\w$]+|[^\w\s$]+|\s+").unwrap());
    if forward {
        let Some(suffix) = source.get(cursor..) else { return cursor };
        words
            .find_iter(suffix)
            .find(|word| !word.as_str().trim().is_empty())
            .map(|word| cursor + word.end())
            .unwrap_or(source.len())
    } else {
        let Some(prefix) = source.get(..cursor) else { return cursor };
        words
            .find_iter(prefix)
            .filter(|word| !word.as_str().trim().is_empty())
            .last()
            .map(|word| word.start())
            .unwrap_or(0)
    }
}

/// Closer used to wrap a selection when an opener is typed over it.
/// gpui-kit's editor pairs, skips over and indents on its own, but replaces selections.
pub fn wrap_closing(inserted: char, in_string_or_comment: bool) -> Option<char> {
    if in_string_or_comment {
        return None;
    }
    match inserted {
        '{' => Some('}'),
        '[' => Some(']'),
        '(' => Some(')'),
        '"' => Some('"'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_brackets_and_quotes_outside_strings() {
        assert_eq!(wrap_closing('{', false), Some('}'));
        assert_eq!(wrap_closing('[', false), Some(']'));
        assert_eq!(wrap_closing('(', false), Some(')'));
        assert_eq!(wrap_closing('"', false), Some('"'));
        assert_eq!(wrap_closing('{', true), None);
        assert_eq!(wrap_closing('a', false), None);
    }
}
