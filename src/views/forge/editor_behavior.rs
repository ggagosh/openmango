//! Pure editor behavior spec — no GPUI dependencies, fully testable.

/// Source of truth for indent width. Read from editor config.
#[derive(Debug, Clone, Copy)]
pub struct IndentConfig {
    pub width: usize,
    pub use_tabs: bool,
}

impl IndentConfig {
    pub fn indent_str(&self) -> String {
        if self.use_tabs { "\t".to_string() } else { " ".repeat(self.width) }
    }
}

impl Default for IndentConfig {
    fn default() -> Self {
        Self { width: 2, use_tabs: false }
    }
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

/// What indent to insert after Enter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndentResult {
    /// No custom indent needed.
    None,
    /// Simple indent continuation.
    Simple(String),
    /// Cursor between braces: inner indent for cursor, outer for closing brace.
    BetweenBraces { inner: String, outer: String },
}

/// Pure decision function for indentation after Enter.
///
/// `prev_non_ws_char` — the last non-whitespace char before the newline.
/// `next_non_ws_char` — the first non-whitespace char after the cursor.
/// `base_indent` — the leading whitespace of the previous non-empty line.
/// `config` — indent width configuration.
pub fn indent_after_enter(
    prev_non_ws_char: Option<char>,
    next_non_ws_char: Option<char>,
    base_indent: &str,
    config: &IndentConfig,
) -> IndentResult {
    let step = config.indent_str();

    let opens_block = matches!(prev_non_ws_char, Some('{') | Some('[') | Some('('));

    if opens_block {
        let expected_close = match prev_non_ws_char {
            Some('{') => Some('}'),
            Some('[') => Some(']'),
            Some('(') => Some(')'),
            _ => Option::None,
        };
        let inner = format!("{}{}", base_indent, step);
        if expected_close == next_non_ws_char {
            // Between braces: cursor on inner indent, closing brace on base indent
            return IndentResult::BetweenBraces { inner, outer: base_indent.to_string() };
        }
        return IndentResult::Simple(inner);
    }

    if base_indent.is_empty() {
        return IndentResult::None;
    }

    IndentResult::Simple(base_indent.to_string())
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

    // ── IndentResult tests ──────────────────────────────────────────

    #[test]
    fn indent_after_open_brace() {
        let config = IndentConfig { width: 2, use_tabs: false };
        assert_eq!(
            indent_after_enter(Some('{'), Some(' '), "  ", &config),
            IndentResult::Simple("    ".to_string())
        );
    }

    #[test]
    fn indent_between_braces() {
        let config = IndentConfig { width: 2, use_tabs: false };
        assert_eq!(
            indent_after_enter(Some('{'), Some('}'), "  ", &config),
            IndentResult::BetweenBraces { inner: "    ".to_string(), outer: "  ".to_string() }
        );
    }

    #[test]
    fn indent_between_brackets() {
        let config = IndentConfig { width: 2, use_tabs: false };
        assert_eq!(
            indent_after_enter(Some('['), Some(']'), "", &config),
            IndentResult::BetweenBraces { inner: "  ".to_string(), outer: "".to_string() }
        );
    }

    #[test]
    fn indent_plain_line() {
        let config = IndentConfig { width: 2, use_tabs: false };
        assert_eq!(
            indent_after_enter(Some('x'), Some('y'), "  ", &config),
            IndentResult::Simple("  ".to_string())
        );
    }

    #[test]
    fn indent_empty_file() {
        let config = IndentConfig { width: 2, use_tabs: false };
        assert_eq!(indent_after_enter(None, None, "", &config), IndentResult::None);
    }

    #[test]
    fn indent_width_from_config() {
        let config = IndentConfig { width: 2, use_tabs: false };
        assert_eq!(config.indent_str(), "  ");
    }

    #[test]
    fn indent_width_4() {
        let config = IndentConfig { width: 4, use_tabs: false };
        assert_eq!(
            indent_after_enter(Some('{'), Some('}'), "", &config),
            IndentResult::BetweenBraces { inner: "    ".to_string(), outer: "".to_string() }
        );
    }

    #[test]
    fn indent_with_tabs() {
        let config = IndentConfig { width: 4, use_tabs: true };
        assert_eq!(config.indent_str(), "\t");
        assert_eq!(
            indent_after_enter(Some('{'), Some(' '), "", &config),
            IndentResult::Simple("\t".to_string())
        );
    }

    #[test]
    fn indent_after_open_bracket() {
        let config = IndentConfig { width: 2, use_tabs: false };
        assert_eq!(
            indent_after_enter(Some('['), Some('1'), "", &config),
            IndentResult::Simple("  ".to_string())
        );
    }

    #[test]
    fn indent_after_open_paren() {
        let config = IndentConfig { width: 2, use_tabs: false };
        assert_eq!(
            indent_after_enter(Some('('), Some(')'), "  ", &config),
            IndentResult::BetweenBraces { inner: "    ".to_string(), outer: "  ".to_string() }
        );
    }
}
