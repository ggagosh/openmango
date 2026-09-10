//! Format query whitespace without round-tripping BSON values or discarding comments.

use std::ops::Range;

use tree_sitter::{Node, Parser};

pub(super) fn format_query(
    raw: &str,
    mut selection: Range<usize>,
) -> Option<(String, Range<usize>)> {
    // Scalar IDs and legacy shorthand keep their original entry form.
    if !raw.trim_start().starts_with('{') {
        return None;
    }
    let source = format!("({raw}\n)");
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_javascript::LANGUAGE.into()).ok()?;
    let tree = parser.parse(&source, None)?;
    if tree.root_node().has_error() {
        return None;
    }
    let mut tokens = Vec::new();
    collect_tokens(tree.root_node(), &mut tokens);
    let mut edits = Vec::new();
    let mut end = 0;
    let mut previous = "";
    let mut depth: usize = 0;
    for token in
        tokens.into_iter().filter(|node| node.start_byte() >= 1 && node.end_byte() <= raw.len() + 1)
    {
        let range = end..token.start_byte() - 1;
        let gap = &raw[range.clone()];
        if !gap.chars().all(char::is_whitespace) {
            return None;
        }
        let kind = token.kind();
        if matches!(kind, "}" | "]" | ")") {
            depth = depth.saturating_sub(1);
        }
        let spacing = if previous.is_empty() {
            String::new()
        } else if gap.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
            let lines =
                gap.replace("\r\n", "\n").matches(['\n', '\r', '\u{2028}', '\u{2029}']).count();
            format!("{}{}", "\n".repeat(lines), "  ".repeat(depth))
        } else if (previous == "{" && kind == "}")
            || matches!(kind, ":" | "," | "]" | ")" | "(" | ".")
            || matches!(previous, "[" | "(" | ".")
        {
            String::new()
        } else if matches!(previous, "{" | ":" | ",")
            || matches!(kind, "}" | "comment")
            || previous == "comment"
            || !gap.is_empty()
        {
            " ".to_string()
        } else {
            String::new()
        };
        if gap != spacing {
            edits.push((range, spacing));
        }
        end = token.end_byte() - 1;
        previous = kind;
        if matches!(kind, "{" | "[" | "(") {
            depth += 1;
        }
    }
    if !raw[end..].chars().all(char::is_whitespace) {
        return None;
    }
    if end < raw.len() {
        edits.push((end..raw.len(), String::new()));
    }
    if edits.is_empty() {
        return None;
    }
    for (range, spacing) in edits.iter().rev() {
        for offset in [&mut selection.start, &mut selection.end] {
            if *offset >= range.end {
                *offset = *offset - range.len() + spacing.len();
            } else if *offset > range.start {
                *offset = range.start + (*offset - range.start).min(spacing.len());
            }
        }
    }
    let mut formatted = String::with_capacity(raw.len());
    let mut end = 0;
    for (range, spacing) in edits {
        formatted.push_str(&raw[end..range.start]);
        formatted.push_str(&spacing);
        end = range.end;
    }
    formatted.push_str(&raw[end..]);
    Some((formatted, selection))
}

fn collect_tokens<'a>(node: Node<'a>, tokens: &mut Vec<Node<'a>>) {
    if node.child_count() == 0
        || matches!(node.kind(), "string" | "comment" | "regex" | "template_string")
    {
        tokens.push(node);
    } else {
        for child in node.children(&mut node.walk()) {
            collect_tokens(child, tokens);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_query;
    use crate::views::documents::fast_filter::compile_filter_input;

    #[test]
    fn query_formatting_preserves_values_order_comments_and_line_breaks() {
        let cases = [
            (
                r#"{backend: ObjectId("6a3e5059e61da6678f2a5577")}"#,
                r#"{ backend: ObjectId("6a3e5059e61da6678f2a5577") }"#,
            ),
            (
                r#"{z:NumberLong("9007199254740993"),a:[1,{"label":'ნინო { x: 1 }'}],empty:{}}"#,
                r#"{ z: NumberLong("9007199254740993"), a: [1, { "label": 'ნინო { x: 1 }' }], empty: {} }"#,
            ),
            (
                "{\nbackend :ObjectId ( \"6a3e5059e61da6678f2a5577\" ), // keep this\n nested:{x:1, y:[2,3]}\n}",
                "{\n  backend: ObjectId(\"6a3e5059e61da6678f2a5577\"), // keep this\n  nested: { x: 1, y: [2, 3] }\n}",
            ),
            (
                r#"{x:/* keep : {} */-0.0,decimal:NumberDecimal("1.2300")}"#,
                r#"{ x: /* keep : {} */ -0.0, decimal: NumberDecimal("1.2300") }"#,
            ),
            ("{x:1,// keep this\ry:2\r\n}", "{ x: 1, // keep this\n  y: 2\n}"),
        ];
        for (source, expected) in cases {
            let (formatted, caret) = format_query(source, source.len()..source.len()).unwrap();
            assert_eq!(formatted, expected);
            assert_eq!(caret, formatted.len()..formatted.len());
            assert_eq!(
                compile_filter_input(source).unwrap().document,
                compile_filter_input(&formatted).unwrap().document,
            );
            assert!(format_query(&formatted, caret).is_none(), "formatting must be idempotent");
        }
        let source = r#"{name:'ნინო',backend:ObjectId("6a3e5059e61da6678f2a5577")}"#;
        let start = source.find("ნინო").unwrap();
        let (formatted, selection) = format_query(source, start..start + "ნინო".len()).unwrap();
        assert_eq!(&formatted[selection], "ნინო");
        for source in ["", "{}", "{ backend:", "6a3e5059e61da6678f2a5577", "age > 18"] {
            assert!(format_query(source, 0..0).is_none());
        }
    }
}
