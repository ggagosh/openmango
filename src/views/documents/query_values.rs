//! Value completion follows MongoDB field paths and uses only already-loaded documents.

use std::ops::Range;

use mongodb::bson::{Bson, Document};
use tree_sitter::{Node, Parser};

#[derive(Clone, Debug)]
pub(super) struct ValueContext {
    pub field: String,
    pub range: Range<usize>,
    pub prefix: String,
    pub direct: bool,
}

#[derive(Clone, Debug)]
pub(super) struct FieldContext {
    pub range: Range<usize>,
    pub prefix: String,
    pub has_colon: bool,
    pub quoted: bool,
    pub parent_path: String,
}

pub(super) fn field_context(text: &str, cursor: usize) -> Option<FieldContext> {
    text.get(..cursor)?;
    let source = format!("({text})");
    let offset = cursor + 1;
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_javascript::LANGUAGE.into()).ok()?;
    let tree = parser.parse(&source, None)?;
    let mut node = tree
        .root_node()
        .named_descendant_for_byte_range(offset.saturating_sub(1), offset.saturating_sub(1))?;
    while !matches!(
        node.kind(),
        "string" | "property_identifier" | "shorthand_property_identifier" | "identifier"
    ) {
        if node.kind() == "comment" {
            return None;
        }
        node = node.parent()?;
    }
    let parent = node.parent()?;
    let has_colon = parent.kind() == "pair" && parent.child_by_field_name("key") == Some(node);
    let new_key = matches!(parent.kind(), "object" | "ERROR")
        && source.get(..node.start_byte())?.trim_end().ends_with(['{', ',']);
    if !has_colon && !new_key {
        return None;
    }
    if offset < node.start_byte() || offset > node.end_byte() {
        return None;
    }
    Some(FieldContext {
        range: node.start_byte().saturating_sub(1)..node.end_byte().saturating_sub(1),
        prefix: source.get(node.start_byte()..offset)?.trim_start_matches(['"', '\'']).to_string(),
        has_colon,
        quoted: node.kind() == "string",
        parent_path: field_path(if has_colon { parent.parent() } else { Some(parent) }, &source)
            .join("."),
    })
}

pub(super) fn value_context(text: &str, cursor: usize) -> Option<ValueContext> {
    text.get(..cursor)?;
    let source = format!("({text})");
    let offset = cursor + 1;
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_javascript::LANGUAGE.into()).ok()?;
    let tree = parser.parse(&source, None)?;
    let mut pending = vec![tree.root_node()];
    while let Some(node) = pending.pop() {
        if node.kind() == "pair"
            && let (Some(key), Some(value)) =
                (node.child_by_field_name("key"), node.child_by_field_name("value"))
        {
            let inside = value.start_byte() <= offset && offset <= value.end_byte();
            let missing = value.is_missing()
                && value.end_byte() <= offset
                && source.get(value.end_byte()..offset).is_some_and(|s| s.trim().is_empty());
            let before_value = key.end_byte() <= offset
                && offset < value.start_byte()
                && source.get(key.end_byte()..offset).is_some_and(|s| s.trim() == ":");
            if (inside || missing || before_value)
                && matches!(
                    value.kind(),
                    "string" | "identifier" | "number" | "null" | "true" | "false"
                )
            {
                let path = field_path(Some(node), &source);
                if path.is_empty() {
                    return None;
                }
                let start = if missing { offset } else { value.start_byte() };
                let end = if missing { offset } else { value.end_byte() };
                let raw = source.get(start..offset.max(start))?;
                // An accepted, closed string is complete. Do not immediately reopen its menu.
                if value.kind() == "string"
                    && offset == end
                    && raw.len() > 1
                    && (raw.ends_with('"') || raw.ends_with('\''))
                {
                    return None;
                }
                return Some(ValueContext {
                    field: path.join("."),
                    range: start.saturating_sub(1)..end.saturating_sub(1),
                    prefix: raw.trim_start_matches(['"', '\'']).to_string(),
                    direct: !key_text(key, &source)?.starts_with('$'),
                });
            }
        }
        let mut walk = node.walk();
        pending.extend(node.named_children(&mut walk));
    }
    None
}

fn key_text(key: Node<'_>, source: &str) -> Option<String> {
    let raw = key.utf8_text(source.as_bytes()).ok()?;
    if raw.starts_with('"') {
        serde_json::from_str(raw).ok()
    } else {
        Some(raw.trim_matches('\'').to_string())
    }
}

fn field_path(mut node: Option<Node<'_>>, source: &str) -> Vec<String> {
    let mut path = Vec::new();
    while let Some(ancestor) = node {
        if ancestor.kind() == "pair"
            && let Some(key) = ancestor.child_by_field_name("key")
            && let Some(key) = key_text(key, source)
            && !key.starts_with('$')
        {
            path.push(key);
        }
        node = ancestor.parent();
    }
    path.reverse();
    path
}

pub(super) fn sampled_values<'a>(
    documents: impl Iterator<Item = &'a Document>,
    field: &str,
) -> Vec<Bson> {
    let path: Vec<_> = field.split('.').collect();
    let mut values = Vec::new();
    let mut remaining = 1000;
    // ponytail: a bounded local sample; schema-wide distinct queries need explicit user intent.
    for document in documents.take(100) {
        if let Some((key, rest)) = path.split_first()
            && let Some(value) = document.get(*key)
        {
            collect_values(value, rest, &mut values, &mut remaining);
        }
        if values.len() >= 20 || remaining == 0 {
            break;
        }
    }
    values
}

fn collect_values(value: &Bson, path: &[&str], out: &mut Vec<Bson>, remaining: &mut usize) {
    if out.len() >= 20 || *remaining == 0 {
        return;
    }
    *remaining -= 1;
    if let Bson::Array(values) = value {
        for value in values.iter().take(8) {
            collect_values(value, path, out, remaining);
        }
    } else if let Some((key, rest)) = path.split_first() {
        if let Bson::Document(doc) = value
            && let Some(value) = doc.get(*key)
        {
            collect_values(value, rest, out, remaining);
        }
    } else if !matches!(
        value,
        Bson::Document(_)
            | Bson::Binary(_)
            | Bson::JavaScriptCode(_)
            | Bson::JavaScriptCodeWithScope(_)
    ) && !matches!(value, Bson::String(text) if text.len() > 160)
        && !out.contains(value)
    {
        out.push(value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    fn context(marked: &str) -> ValueContext {
        let cursor = marked.find('|').unwrap();
        value_context(&marked.replacen('|', "", 1), cursor).expect(marked)
    }

    #[test]
    fn value_completion_tracks_fields_quotes_and_operators() {
        let ctx = context("{ status: | }");
        assert_eq!(ctx.field, "status");
        assert!(ctx.prefix.is_empty());
        let ctx = context("{ status: \"ac|tive\" }");
        assert_eq!(ctx.field, "status");
        assert_eq!(ctx.prefix, "ac");
        assert_eq!(&"{ status: \"active\" }"[ctx.range], "\"active\"");
        assert_eq!(context("{ profile: { name: 'Al|' } }").field, "profile.name");
        let ctx = context("{ age: { $gt: 3| } }");
        assert_eq!(ctx.field, "age");
        assert!(!ctx.direct);
        assert!(value_context("{ /* status: abc */ }", 12).is_none());
        assert!(value_context("{ status: \"active\" }", 18).is_none());
    }

    #[test]
    fn sampled_values_follow_nested_array_fields_and_deduplicate() {
        let docs =
            [doc! {"items": [{"status": "active"}, {"status": "active"}, {"status": "pending"}]}];
        assert_eq!(
            sampled_values(docs.iter(), "items.status"),
            vec![Bson::String("active".into()), Bson::String("pending".into())]
        );
    }

    #[test]
    fn field_completion_replaces_the_whole_key_and_preserves_existing_values() {
        let source = "{ \"status\": \"active\" }";
        let field = field_context(source, source.find("status").unwrap() + 2).unwrap();
        assert_eq!(field.prefix, "st");
        assert_eq!(&source[field.range], "\"status\"");
        assert!(field.has_colon && field.quoted);
        let field = field_context("{ sta }", 5).unwrap();
        assert_eq!(field.prefix, "sta");
        assert!(!field.has_colon);
        let field = field_context("{ \"sta\" }", 6).unwrap();
        assert_eq!(field.prefix, "sta");
        assert!(!field.has_colon);
        assert!(field_context("{ status: \"active\" }", 14).is_none());
        assert!(field_context("{ /* \"sta\" */ }", 9).is_none());
    }
}
