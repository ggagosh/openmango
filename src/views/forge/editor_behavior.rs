//! Bracket and quote pairing decisions for MongoDB query editors.

use std::ops::Range;
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

pub struct NewlineEdit {
    pub range: Range<usize>,
    pub text: String,
    pub cursor: usize,
}

/// Add one indentation level after an opener, splitting a matching closer onto
/// its own line. The caller checks the syntax context to exclude strings/comments.
pub fn newline_after_opening(source: &str, selection: Range<usize>) -> Option<NewlineEdit> {
    let prefix = source.get(..selection.start)?;
    let before = prefix.trim_end_matches([' ', '\t']);
    let closing = match before.chars().next_back()? {
        '{' => '}',
        '[' => ']',
        '(' => ')',
        _ => return None,
    };
    let suffix = source.get(selection.end..)?;
    let after = suffix.trim_start_matches([' ', '\t']);
    let line = prefix.rsplit('\n').next()?;
    let base_indent: String = line.chars().take_while(|ch| matches!(ch, ' ' | '\t')).collect();
    let indent = format!("{}{}", base_indent, " ".repeat(INDENT_WIDTH));
    let text = if after.starts_with(closing) {
        format!("\n{indent}\n{base_indent}")
    } else {
        format!("\n{indent}")
    };
    Some(NewlineEdit {
        range: before.len()..selection.end + suffix.len() - after.len(),
        cursor: before.len() + 1 + indent.len(),
        text,
    })
}

/// What to do when user types an opening bracket or quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairAction {
    /// Insert the closing char after cursor.
    InsertClosing(char),
    /// Wrap the current selection in open/close chars.
    WrapSelection(char, char),
    /// Skip past the existing closing char (overtype).
    Overtype,
    /// Do nothing (inside string/comment, or closing bracket already follows).
    Skip,
}

/// Pure decision function for auto-pairing brackets and quotes.
pub fn pair_action(
    inserted_char: char,
    char_after_cursor: Option<char>,
    in_string_or_comment: bool,
    has_selection: bool,
) -> PairAction {
    // Overtype: typing closing char when same char follows cursor
    if !has_selection {
        if inserted_char == '"' && char_after_cursor == Some('"') {
            return PairAction::Overtype;
        }
        if matches!(inserted_char, '}' | ']' | ')') && char_after_cursor == Some(inserted_char) {
            return PairAction::Overtype;
        }
    }

    let closing = match inserted_char {
        '{' => '}',
        '[' => ']',
        '(' => ')',
        '"' => '"',
        _ => return PairAction::Skip,
    };

    if in_string_or_comment {
        return PairAction::Skip;
    }

    if has_selection {
        return PairAction::WrapSelection(inserted_char, closing);
    }

    PairAction::InsertClosing(closing)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PairAction tests ────────────────────────────────────────────

    #[test]
    fn pair_opening_brace() {
        assert_eq!(pair_action('{', Some(' '), false, false), PairAction::InsertClosing('}'));
    }

    #[test]
    fn pair_opening_bracket() {
        assert_eq!(pair_action('[', None, false, false), PairAction::InsertClosing(']'));
    }

    #[test]
    fn pair_opening_paren() {
        assert_eq!(pair_action('(', None, false, false), PairAction::InsertClosing(')'));
    }

    #[test]
    fn pair_in_string() {
        assert_eq!(pair_action('{', None, true, false), PairAction::Skip);
    }

    #[test]
    fn pair_in_comment() {
        assert_eq!(pair_action('{', None, true, false), PairAction::Skip);
    }

    #[test]
    fn pair_closing_after_cursor() {
        // Typing `{` when `}` follows should still insert closing `}` to support nesting.
        assert_eq!(pair_action('{', Some('}'), false, false), PairAction::InsertClosing('}'));
    }

    #[test]
    fn pair_with_selection() {
        assert_eq!(pair_action('{', None, false, true), PairAction::WrapSelection('{', '}'));
    }

    #[test]
    fn pair_non_bracket_char() {
        assert_eq!(pair_action('a', None, false, false), PairAction::Skip);
    }

    #[test]
    fn pair_overtype_closing_brace() {
        assert_eq!(pair_action('}', Some('}'), false, false), PairAction::Overtype);
    }

    #[test]
    fn pair_overtype_closing_bracket() {
        assert_eq!(pair_action(']', Some(']'), false, false), PairAction::Overtype);
    }

    #[test]
    fn pair_overtype_closing_paren() {
        assert_eq!(pair_action(')', Some(')'), false, false), PairAction::Overtype);
    }

    #[test]
    fn pair_closing_brace_no_overtype_when_different() {
        assert_eq!(pair_action('}', Some(' '), false, false), PairAction::Skip);
    }
}
