//! Bracket and quote pairing decisions for MongoDB query editors.

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
