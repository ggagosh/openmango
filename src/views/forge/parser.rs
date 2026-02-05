use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionKind {
    /// `{ name| }` — typing a field name
    Key,
    /// `{ name: val| }` — typing a value
    Value,
    /// `{ $ma| }` — typing an operator key (starts with `$`)
    OperatorKey,
    /// `[ val| ]` — inside array literal
    ArrayElement,
    /// tree-sitter can't determine position
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeKind {
    /// find/findOne/deleteOne/deleteMany/countDocuments/distinct arg 0
    FindFilter,
    /// updateOne/updateMany arg 1
    UpdateDoc,
    /// aggregate([{ ... }]) — top-level stage
    AggregateStage,
    /// `{ $match: { ... } }`
    MatchFilter,
    /// `{ $group: { ... } }`
    GroupSpec,
    /// `{ $project: { ... } }`
    ProjectSpec,
    /// `{ $set: { ... } }`, `{ $addFields: { ... } }`
    SetDoc,
    /// insertOne/insertMany arg 0
    InsertDoc,
    /// `{ $gt: val }` — inside query operator value
    OperatorValue,
    /// Not inside any method call — line-level heuristics
    #[default]
    TopLevel,
    /// Inside call but can't determine scope
    Unknown,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedContext {
    pub collection: Option<String>,
    pub method: Option<String>,
    pub position_kind: PositionKind,
    pub scope_kind: ScopeKind,
    pub in_comment: bool,
}

// ── Main entry point ───────────────────────────────────────────────────────

pub fn parse_context(text: &str, cursor: usize) -> ParsedContext {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_javascript::LANGUAGE.into()).is_err() {
        return ParsedContext::default();
    }

    let tree = match parser.parse(text, None) {
        Some(tree) => tree,
        None => return ParsedContext::default(),
    };

    let cursor = cursor.min(text.len());
    let Some(node) = tree.root_node().named_descendant_for_byte_range(cursor, cursor) else {
        return ParsedContext::default();
    };

    if is_in_comment_or_string(node) {
        return ParsedContext { in_comment: true, ..Default::default() };
    }

    let Some(call) = find_enclosing_call(node) else {
        // No call found — top-level context
        return ParsedContext {
            scope_kind: ScopeKind::TopLevel,
            position_kind: PositionKind::Unknown,
            ..Default::default()
        };
    };

    let mut ctx = ParsedContext::default();

    let Some((method, collection)) = parse_call_target(text, &call) else {
        return ctx;
    };

    ctx.method = Some(method.clone());
    ctx.collection = collection;

    // Determine argument index
    let arg_index = infer_arg_index(&call, cursor);

    // Determine scope from method + arg index + nesting
    ctx.scope_kind = infer_scope_kind(text, cursor, &call, &method, arg_index);

    // Determine position (key/value/operator/array)
    ctx.position_kind = infer_position_kind(text, cursor, &call);

    ctx
}

// ── Position inference (B2) ────────────────────────────────────────────────

fn infer_position_kind(text: &str, cursor: usize, call: &Node) -> PositionKind {
    let node = match call
        .named_descendant_for_byte_range(cursor, cursor)
        .or_else(|| call.named_descendant_for_byte_range(cursor.saturating_sub(1), cursor))
    {
        Some(n) => n,
        None => return PositionKind::Unknown,
    };

    // Walk upward from cursor node to find the nearest pair or container
    let mut current = node;
    loop {
        match current.kind() {
            "pair" => {
                return position_in_pair(text, cursor, &current);
            }
            "object" => {
                // Cursor might be just past the end of a pair (e.g. `{ name: val| }`).
                // tree-sitter resolves to the object node in this case.
                // Check if any pair child contains the cursor or cursor-1.
                if let Some(pair) = find_pair_containing_cursor(&current, cursor) {
                    return position_in_pair(text, cursor, &pair);
                }
                // Inside object but not in a pair — typing a new key after `{` or `,`
                return if is_cursor_token_operator(text, cursor) {
                    PositionKind::OperatorKey
                } else {
                    PositionKind::Key
                };
            }
            "array" => {
                return PositionKind::ArrayElement;
            }
            _ => {}
        }
        current = match current.parent() {
            Some(p) => p,
            None => return PositionKind::Unknown,
        };
    }
}

/// Find a pair child of an object node that contains the cursor position.
/// Handles the edge case where cursor is at pair.end_byte() (just past the pair).
fn find_pair_containing_cursor<'a>(object: &Node<'a>, cursor: usize) -> Option<Node<'a>> {
    for i in 0..object.named_child_count() {
        if let Some(child) = object.named_child(i)
            && child.kind() == "pair"
            && cursor >= child.start_byte()
            && cursor <= child.end_byte()
        {
            return Some(child);
        }
    }
    // Also check cursor-1 for boundary case
    if cursor > 0 {
        for i in 0..object.named_child_count() {
            if let Some(child) = object.named_child(i)
                && child.kind() == "pair"
                && (cursor - 1) >= child.start_byte()
                && (cursor - 1) < child.end_byte()
            {
                return Some(child);
            }
        }
    }
    None
}

fn position_in_pair(text: &str, cursor: usize, pair: &Node) -> PositionKind {
    if let Some(key_node) = pair.child_by_field_name("key")
        && cursor >= key_node.start_byte()
        && cursor <= key_node.end_byte()
    {
        // Inside the key portion
        return if is_cursor_token_operator(text, cursor) {
            PositionKind::OperatorKey
        } else {
            PositionKind::Key
        };
    }
    if let Some(value_node) = pair.child_by_field_name("value")
        && cursor >= value_node.start_byte()
        && cursor <= value_node.end_byte()
    {
        return PositionKind::Value;
    }
    // Between key and value (after colon) — treat as value
    // Or if pair exists but cursor doesn't fall in key/value range
    if let Some(key_node) = pair.child_by_field_name("key")
        && cursor > key_node.end_byte()
    {
        return PositionKind::Value;
    }
    PositionKind::Unknown
}

/// Check if the token being typed at cursor starts with `$`
fn is_cursor_token_operator(text: &str, cursor: usize) -> bool {
    // Scan backwards from cursor to find start of current token
    let bytes = text.as_bytes();
    let mut start = cursor;
    while start > 0 {
        let ch = bytes[start - 1];
        if ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'$' {
            start -= 1;
        } else {
            break;
        }
    }
    text.get(start..cursor).map(|token| token.starts_with('$')).unwrap_or(false)
}

// ── Scope inference (B3) — deep walk ───────────────────────────────────────

fn infer_scope_kind(
    text: &str,
    cursor: usize,
    call: &Node,
    method: &str,
    arg_index: Option<usize>,
) -> ScopeKind {
    let Some(arg_index) = arg_index else {
        return ScopeKind::Unknown;
    };

    // First determine the base scope from method + arg_index
    let base_scope = method_arg_scope(method, arg_index);

    // For aggregate, we need deeper analysis
    if base_scope == ScopeKind::AggregateStage {
        // Walk up through pairs to find pipeline operator context
        if let Some(deeper) = deep_scope_walk(text, cursor, call) {
            return deeper;
        }
        return ScopeKind::AggregateStage;
    }

    // For filter/update scopes, check for nested operator values
    if matches!(base_scope, ScopeKind::FindFilter | ScopeKind::UpdateDoc) {
        if let Some(deeper) = deep_scope_walk(text, cursor, call) {
            // Only return deeper scope if it's more specific
            if deeper != ScopeKind::AggregateStage {
                return deeper;
            }
        }
        return base_scope;
    }

    base_scope
}

fn method_arg_scope(method: &str, arg_index: usize) -> ScopeKind {
    match method {
        "find" | "findOne" | "deleteOne" | "deleteMany" | "countDocuments" | "distinct" => {
            if arg_index == 0 {
                ScopeKind::FindFilter
            } else {
                ScopeKind::Unknown
            }
        }
        "updateOne" | "updateMany" => match arg_index {
            0 => ScopeKind::FindFilter,
            1 => ScopeKind::UpdateDoc,
            _ => ScopeKind::Unknown,
        },
        "insertOne" | "insertMany" => {
            if arg_index == 0 {
                ScopeKind::InsertDoc
            } else {
                ScopeKind::Unknown
            }
        }
        "aggregate" => {
            if arg_index == 0 {
                ScopeKind::AggregateStage
            } else {
                ScopeKind::Unknown
            }
        }
        _ => ScopeKind::Unknown,
    }
}

/// Walk up through pair nodes to find the innermost pipeline/query operator scope.
fn deep_scope_walk(text: &str, cursor: usize, call: &Node) -> Option<ScopeKind> {
    let node = call
        .named_descendant_for_byte_range(cursor, cursor)
        .or_else(|| call.named_descendant_for_byte_range(cursor.saturating_sub(1), cursor))?;

    let mut current = node;

    loop {
        if current.kind() == "pair"
            && let Some(key_node) = current.child_by_field_name("key")
        {
            // Check if cursor is inside the value portion of this pair
            let is_in_value = if let Some(value_node) = current.child_by_field_name("value") {
                cursor >= value_node.start_byte() && cursor <= value_node.end_byte()
            } else {
                // Pair without value node: cursor is after the key
                cursor > key_node.end_byte()
            };

            if is_in_value {
                let key_text = node_text(text, &key_node);
                if let Some(scope) = operator_key_to_scope(&key_text) {
                    // Return the innermost matching scope immediately
                    return Some(scope);
                }
            }
        }

        // When we hit an object, also check its pair children for boundary cases
        if current.kind() == "object"
            && let Some(pair) = find_pair_containing_cursor(&current, cursor)
            && let Some(key_node) = pair.child_by_field_name("key")
        {
            let is_in_value = if let Some(value_node) = pair.child_by_field_name("value") {
                cursor >= value_node.start_byte() && cursor <= value_node.end_byte()
            } else {
                cursor > key_node.end_byte()
            };
            if is_in_value {
                let key_text = node_text(text, &key_node);
                if let Some(scope) = operator_key_to_scope(&key_text) {
                    return Some(scope);
                }
            }
        }

        if current.kind() == "call_expression" {
            break;
        }
        current = match current.parent() {
            Some(p) => p,
            None => break,
        };
    }

    None
}

/// Map operator key text to scope kind.
fn operator_key_to_scope(key: &str) -> Option<ScopeKind> {
    match key {
        "$match" => Some(ScopeKind::MatchFilter),
        "$group" => Some(ScopeKind::GroupSpec),
        "$project" => Some(ScopeKind::ProjectSpec),
        "$set" | "$addFields" | "$replaceRoot" | "$replaceWith" => Some(ScopeKind::SetDoc),
        // Query operators → OperatorValue
        "$eq" | "$ne" | "$gt" | "$gte" | "$lt" | "$lte" | "$in" | "$nin" | "$exists" | "$regex"
        | "$not" | "$elemMatch" | "$size" | "$all" | "$type" => Some(ScopeKind::OperatorValue),
        // Logical operators that contain filters — treat inner scope as parent scope
        // $and, $or, $nor contain arrays of filter documents, skip them
        _ => None,
    }
}

// ── Arg index helper ───────────────────────────────────────────────────────

fn infer_arg_index(call: &Node, cursor: usize) -> Option<usize> {
    let args = call.child_by_field_name("arguments")?;
    for i in 0..args.named_child_count() {
        let arg = args.named_child(i)?;
        if cursor >= arg.start_byte() && cursor <= arg.end_byte() {
            return Some(i);
        }
    }
    None
}

// ── Shared helpers ─────────────────────────────────────────────────────────

fn is_in_comment_or_string(mut node: Node) -> bool {
    loop {
        let kind = node.kind();
        if matches!(kind, "comment" | "string" | "template_string" | "template_substitution") {
            return true;
        }
        if let Some(parent) = node.parent() {
            node = parent;
        } else {
            return false;
        }
    }
}

fn find_enclosing_call<'a>(mut node: Node<'a>) -> Option<Node<'a>> {
    loop {
        if node.kind() == "call_expression" {
            return Some(node);
        }
        node = node.parent()?;
    }
}

fn parse_call_target(text: &str, call: &Node) -> Option<(String, Option<String>)> {
    let callee = call.child_by_field_name("function")?;
    if callee.kind() != "member_expression" {
        return None;
    }
    let method_node = callee.child_by_field_name("property")?;
    let method = node_text(text, &method_node);
    let object = callee.child_by_field_name("object")?;
    let collection = extract_collection(text, &object);
    Some((method, collection))
}

fn extract_collection(text: &str, node: &Node) -> Option<String> {
    match node.kind() {
        "member_expression" => {
            let base = node.child_by_field_name("object")?;
            let prop = node.child_by_field_name("property")?;
            if node_text(text, &base) == "db" {
                return Some(node_text(text, &prop));
            }
            None
        }
        "call_expression" => {
            let callee = node.child_by_field_name("function")?;
            if callee.kind() != "member_expression" {
                return None;
            }
            let base = callee.child_by_field_name("object")?;
            let prop = callee.child_by_field_name("property")?;
            if node_text(text, &base) != "db" || node_text(text, &prop) != "getCollection" {
                return None;
            }
            let args = node.child_by_field_name("arguments")?;
            let first = args.named_child(0)?;
            if first.kind() != "string" {
                return None;
            }
            let raw = node_text(text, &first);
            Some(raw.trim_matches(&['"', '\''][..]).to_string())
        }
        _ => None,
    }
}

fn node_text(text: &str, node: &Node) -> String {
    let range = node.byte_range();
    text.get(range).unwrap_or("").to_string()
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_at(text: &str) -> ParsedContext {
        let cursor = text.find('|').expect("test input must contain | for cursor");
        let clean = format!("{}{}", &text[..cursor], &text[cursor + 1..]);
        parse_context(&clean, cursor)
    }

    // ── PositionKind tests ──────────────────────────────────────────

    #[test]
    fn position_key_in_find_filter() {
        let ctx = ctx_at("db.users.find({ name| })");
        assert_eq!(ctx.position_kind, PositionKind::Key);
        assert_eq!(ctx.scope_kind, ScopeKind::FindFilter);
    }

    #[test]
    fn position_value_in_find_filter() {
        let ctx = ctx_at("db.users.find({ name: val| })");
        assert_eq!(ctx.position_kind, PositionKind::Value);
        assert_eq!(ctx.scope_kind, ScopeKind::FindFilter);
    }

    #[test]
    fn position_operator_key_in_find_filter() {
        let ctx = ctx_at("db.users.find({ $gt| })");
        assert_eq!(ctx.position_kind, PositionKind::OperatorKey);
        assert_eq!(ctx.scope_kind, ScopeKind::FindFilter);
    }

    #[test]
    fn position_operator_key_nested_in_value() {
        // { name: { $gt| } } — typing an operator key inside the value of field `name`
        // `name` is not an operator, so scope remains FindFilter (not OperatorValue).
        // OperatorValue would be the scope inside `{ $gt: 5| }`.
        let ctx = ctx_at("db.users.find({ name: { $gt| } })");
        assert_eq!(ctx.position_kind, PositionKind::OperatorKey);
        assert_eq!(ctx.scope_kind, ScopeKind::FindFilter);
    }

    #[test]
    fn position_value_inside_operator() {
        let ctx = ctx_at("db.users.find({ name: { $gt: 5| } })");
        assert_eq!(ctx.position_kind, PositionKind::Value);
        assert_eq!(ctx.scope_kind, ScopeKind::OperatorValue);
    }

    #[test]
    fn position_key_in_and_array() {
        let ctx = ctx_at("db.users.find({ $and: [{ n| }] })");
        assert_eq!(ctx.position_kind, PositionKind::Key);
        // $and doesn't change scope, should still be FindFilter context
        assert_eq!(ctx.scope_kind, ScopeKind::FindFilter);
    }

    #[test]
    fn position_operator_key_in_update_doc() {
        let ctx = ctx_at("db.users.updateOne({}, { $set| })");
        assert_eq!(ctx.position_kind, PositionKind::OperatorKey);
        assert_eq!(ctx.scope_kind, ScopeKind::UpdateDoc);
    }

    #[test]
    fn position_key_in_set_doc() {
        let ctx = ctx_at("db.users.updateOne({}, { $set: { n| } })");
        assert_eq!(ctx.position_kind, PositionKind::Key);
        assert_eq!(ctx.scope_kind, ScopeKind::SetDoc);
    }

    #[test]
    fn position_operator_key_in_aggregate_stage() {
        let ctx = ctx_at("db.c.aggregate([{ $ma| }])");
        assert_eq!(ctx.position_kind, PositionKind::OperatorKey);
        assert_eq!(ctx.scope_kind, ScopeKind::AggregateStage);
    }

    #[test]
    fn position_key_in_match_filter() {
        let ctx = ctx_at("db.c.aggregate([{ $match: { n| } }])");
        assert_eq!(ctx.position_kind, PositionKind::Key);
        assert_eq!(ctx.scope_kind, ScopeKind::MatchFilter);
    }

    #[test]
    fn position_key_in_nested_and_in_match() {
        let ctx = ctx_at("db.c.aggregate([{ $match: { $and: [{ n| }] } }])");
        assert_eq!(ctx.position_kind, PositionKind::Key);
        assert_eq!(ctx.scope_kind, ScopeKind::MatchFilter);
    }

    #[test]
    fn position_operator_in_group_spec() {
        let ctx = ctx_at("db.c.aggregate([{ $group: { count: { $su| } } }])");
        assert_eq!(ctx.position_kind, PositionKind::OperatorKey);
        assert_eq!(ctx.scope_kind, ScopeKind::GroupSpec);
    }

    #[test]
    fn position_key_in_insert_doc() {
        let ctx = ctx_at("db.c.insertOne({ field| })");
        assert_eq!(ctx.position_kind, PositionKind::Key);
        assert_eq!(ctx.scope_kind, ScopeKind::InsertDoc);
    }

    #[test]
    fn position_value_in_insert_doc() {
        let ctx = ctx_at("db.c.insertOne({ field: val| })");
        assert_eq!(ctx.position_kind, PositionKind::Value);
        assert_eq!(ctx.scope_kind, ScopeKind::InsertDoc);
    }

    #[test]
    fn top_level_context() {
        let ctx = ctx_at("db.| ");
        assert_eq!(ctx.position_kind, PositionKind::Unknown);
        assert_eq!(ctx.scope_kind, ScopeKind::TopLevel);
    }

    // ── Comment/string detection ────────────────────────────────────

    #[test]
    fn in_comment() {
        let ctx = ctx_at("// db.users.find({ name| })");
        assert!(ctx.in_comment);
    }

    #[test]
    fn in_string() {
        let ctx = ctx_at("var x = \"some text|\"");
        assert!(ctx.in_comment); // in_comment covers strings too
    }

    // ── Collection extraction ───────────────────────────────────────

    #[test]
    fn collection_from_member() {
        let ctx = ctx_at("db.users.find({ name| })");
        assert_eq!(ctx.collection.as_deref(), Some("users"));
    }

    #[test]
    fn method_detected() {
        let ctx = ctx_at("db.users.find({ name| })");
        assert_eq!(ctx.method.as_deref(), Some("find"));
    }

    // ── Project scope ───────────────────────────────────────────────

    #[test]
    fn position_key_in_project_spec() {
        let ctx = ctx_at("db.c.aggregate([{ $project: { n| } }])");
        assert_eq!(ctx.position_kind, PositionKind::Key);
        assert_eq!(ctx.scope_kind, ScopeKind::ProjectSpec);
    }
}
