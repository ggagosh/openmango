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
    /// `db.us|` — typing a property name in a member expression
    MemberAccess,
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
    /// After `db.` — show collections + db methods
    DbMember,
    /// After `db.collection.` — show collection methods
    CollectionMember,
    /// Not inside any method call — nothing useful to complete
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
    /// Partial text being typed after the dot in member access (e.g. `"us"` in `db.us|`)
    pub member_token: Option<String>,
}

// ── Main entry point ───────────────────────────────────────────────────────

pub fn parse_context(text: &str, cursor: usize) -> ParsedContext {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_javascript::LANGUAGE.into()).is_err() {
        return fallback_member_access(text, cursor).unwrap_or_default();
    }

    let tree = match parser.parse(text, None) {
        Some(tree) => tree,
        None => return fallback_member_access(text, cursor).unwrap_or_default(),
    };

    let cursor = cursor.min(text.len());
    let Some(node) = tree.root_node().named_descendant_for_byte_range(cursor, cursor) else {
        return fallback_member_access(text, cursor).unwrap_or_default();
    };

    if is_in_comment_or_string(node) {
        return ParsedContext { in_comment: true, ..Default::default() };
    }

    // Check for member access (db. / db.collection.) before looking for enclosing call
    if let Some(ctx) = detect_member_access(text, cursor, &tree.root_node()) {
        return ctx;
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
                if let Some(scope) = operator_key_to_scope(normalize_key_text(&key_text)) {
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
                if let Some(scope) = operator_key_to_scope(normalize_key_text(&key_text)) {
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
        "$set" | "$addFields" | "$replaceRoot" | "$replaceWith" | "$unset" | "$inc" | "$mul"
        | "$min" | "$max" | "$rename" | "$push" | "$pull" | "$addToSet" | "$currentDate"
        | "$setOnInsert" => Some(ScopeKind::SetDoc),
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
    let named_count = args.named_child_count();

    // Empty call site (e.g. `find(|)`) should resolve to first argument.
    if named_count == 0 {
        if cursor >= args.start_byte() && cursor <= args.end_byte() {
            return Some(0);
        }
        return None;
    }

    let mut ranges: Vec<(usize, usize)> = Vec::with_capacity(named_count);
    for i in 0..named_count {
        let arg = args.named_child(i)?;
        ranges.push((arg.start_byte(), arg.end_byte()));
    }

    // Cursor inside an existing argument.
    for (index, (start, end)) in ranges.iter().enumerate() {
        if cursor >= *start && cursor <= *end {
            return Some(index);
        }
    }

    // Cursor before first arg but inside argument list punctuation.
    if let Some((first_start, _)) = ranges.first().copied()
        && cursor <= first_start
        && cursor >= args.start_byte()
    {
        return Some(0);
    }

    // Cursor between args resolves to the next arg slot.
    for index in 0..ranges.len().saturating_sub(1) {
        let (_, left_end) = ranges[index];
        let (right_start, _) = ranges[index + 1];
        if cursor > left_end && cursor < right_start {
            return Some(index + 1);
        }
    }

    // Cursor after the last existing arg but before `)` -> next slot.
    if let Some((_, last_end)) = ranges.last().copied()
        && cursor > last_end
        && cursor <= args.end_byte()
    {
        return Some(named_count);
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

fn normalize_key_text(key: &str) -> &str {
    key.trim().trim_matches(&['"', '\''][..])
}

// ── Member-access detection ────────────────────────────────────────────────

/// Detect `db.`, `db.collection.`, `db.getCollection("x").`, `db["x"].`,
/// and chained calls like `db.users.find().sort().`.
fn detect_member_access(text: &str, cursor: usize, root: &Node) -> Option<ParsedContext> {
    // Strategy: use cursor-1 to find the property_identifier or other node,
    // then walk up to find the member_expression context.
    // When cursor is at end of text (e.g. `db.us|`), named_descendant at cursor
    // returns `program`, but cursor-1 returns the `property_identifier`.

    // First try: find node at cursor-1 (covers partial typing like `db.us|`)
    let node = if cursor > 0 {
        root.named_descendant_for_byte_range(cursor.saturating_sub(1), cursor)
    } else {
        None
    };

    if let Some(n) = node {
        // If we found a property_identifier, walk up to its parent member_expression
        if n.kind() == "property_identifier"
            && let Some(parent) = n.parent()
            && parent.kind() == "member_expression"
        {
            let member_token = text.get(n.start_byte()..cursor).unwrap_or("").to_string();
            if let Some(obj) = parent.child_by_field_name("object") {
                return analyze_member_chain(text, &obj, member_token);
            }
        }

        // Walk up from the node looking for member_expression or ERROR
        let mut current = n;
        loop {
            match current.kind() {
                "ERROR" => {
                    return analyze_error_node(text, cursor, &current);
                }
                "call_expression" | "program" => break,
                _ => {}
            }
            current = match current.parent() {
                Some(p) => p,
                None => break,
            };
        }
    }

    // Second try: check if cursor is right after a dot
    if cursor > 0 && text.as_bytes().get(cursor - 1) == Some(&b'.') {
        let dot_pos = cursor - 1;
        // Find the expression before the dot
        if let Some(obj_node) =
            root.named_descendant_for_byte_range(dot_pos.saturating_sub(1), dot_pos)
        {
            // Walk up to find the complete expression ending at the dot
            let mut expr = obj_node;
            while let Some(parent) = expr.parent() {
                if parent.end_byte() > dot_pos || parent.kind() == "program" {
                    break;
                }
                expr = parent;
            }
            if expr.end_byte() == dot_pos {
                return analyze_member_chain(text, &expr, String::new());
            }
        }

        // Also check ERROR nodes at the root level for `db.` patterns
        for i in 0..root.named_child_count() {
            if let Some(child) = root.named_child(i)
                && child.kind() == "ERROR"
                && cursor >= child.start_byte()
                && cursor <= child.end_byte()
            {
                return analyze_error_node(text, cursor, &child);
            }
        }
    }

    None
}

/// Analyze the object part of a member expression to classify the chain.
fn analyze_member_chain(text: &str, object: &Node, member_token: String) -> Option<ParsedContext> {
    match object.kind() {
        "identifier" => {
            let name = node_text(text, object);
            if name == "db" {
                return Some(ParsedContext {
                    scope_kind: ScopeKind::DbMember,
                    position_kind: PositionKind::MemberAccess,
                    member_token: Some(member_token),
                    ..Default::default()
                });
            }
            None
        }
        "member_expression" => {
            let base = object.child_by_field_name("object")?;
            let prop = object.child_by_field_name("property")?;
            let base_name = node_text(text, &base);
            if base_name == "db" {
                let collection = node_text(text, &prop);
                return Some(ParsedContext {
                    scope_kind: ScopeKind::CollectionMember,
                    position_kind: PositionKind::MemberAccess,
                    collection: Some(collection),
                    member_token: Some(member_token),
                    ..Default::default()
                });
            }
            // Could be deeper: check if base itself resolves to db.xxx
            if base.kind() == "call_expression" {
                return analyze_call_chain(text, &base, member_token);
            }
            None
        }
        "call_expression" => analyze_call_chain(text, object, member_token),
        "subscript_expression" => {
            let base = object.child_by_field_name("object")?;
            let index = object.child_by_field_name("index")?;
            let base_name = node_text(text, &base);
            if base_name == "db" && index.kind() == "string" {
                let raw = node_text(text, &index);
                let collection = raw.trim_matches(&['"', '\''][..]).to_string();
                return Some(ParsedContext {
                    scope_kind: ScopeKind::CollectionMember,
                    position_kind: PositionKind::MemberAccess,
                    collection: Some(collection),
                    member_token: Some(member_token),
                    ..Default::default()
                });
            }
            None
        }
        _ => None,
    }
}

/// Walk through chained call expressions to find the db.xxx root.
fn analyze_call_chain(text: &str, call: &Node, member_token: String) -> Option<ParsedContext> {
    let callee = call.child_by_field_name("function")?;

    if callee.kind() == "member_expression" {
        let callee_obj = callee.child_by_field_name("object")?;
        let callee_prop = callee.child_by_field_name("property")?;
        let callee_obj_text = node_text(text, &callee_obj);
        let callee_prop_text = node_text(text, &callee_prop);

        // `db.getCollection("x")` — direct db method
        if callee_obj_text == "db" && callee_prop_text == "getCollection" {
            let args = call.child_by_field_name("arguments")?;
            let first = args.named_child(0)?;
            if first.kind() == "string" {
                let raw = node_text(text, &first);
                let collection = raw.trim_matches(&['"', '\''][..]).to_string();
                return Some(ParsedContext {
                    scope_kind: ScopeKind::CollectionMember,
                    position_kind: PositionKind::MemberAccess,
                    collection: Some(collection),
                    member_token: Some(member_token),
                    ..Default::default()
                });
            }
            return None;
        }

        // `db.xxx.method()` — callee_obj is `db.xxx` member_expression
        if callee_obj.kind() == "member_expression" {
            let base = callee_obj.child_by_field_name("object")?;
            let prop = callee_obj.child_by_field_name("property")?;
            if node_text(text, &base) == "db" {
                let collection = node_text(text, &prop);
                return Some(ParsedContext {
                    scope_kind: ScopeKind::CollectionMember,
                    position_kind: PositionKind::MemberAccess,
                    collection: Some(collection),
                    member_token: Some(member_token),
                    ..Default::default()
                });
            }
        }

        // Recurse through call chains: `db.users.find().sort().`
        if callee_obj.kind() == "call_expression" {
            return analyze_call_chain(text, &callee_obj, member_token);
        }
    }

    None
}

/// Handle ERROR nodes produced by tree-sitter for incomplete expressions like `db.`
fn analyze_error_node(text: &str, cursor: usize, error_node: &Node) -> Option<ParsedContext> {
    let child_count = error_node.child_count();
    for i in 0..child_count {
        let child = error_node.child(i)?;
        if child.kind() == "." && child.end_byte() <= cursor && i > 0 {
            let before = error_node.child(i - 1)?;
            let member_token = if child.end_byte() < cursor {
                text.get(child.end_byte()..cursor).unwrap_or("").to_string()
            } else {
                String::new()
            };

            if before.kind() == "identifier" && node_text(text, &before) == "db" {
                return Some(ParsedContext {
                    scope_kind: ScopeKind::DbMember,
                    position_kind: PositionKind::MemberAccess,
                    member_token: Some(member_token),
                    ..Default::default()
                });
            }

            if matches!(
                before.kind(),
                "member_expression" | "call_expression" | "subscript_expression"
            ) {
                return analyze_member_chain(text, &before, member_token);
            }
        }
    }

    // Check named children for member/call expressions inside the ERROR
    let named_count = error_node.named_child_count();
    for i in 0..named_count {
        if let Some(child) = error_node.named_child(i) {
            if child.kind() == "member_expression" && cursor > child.end_byte() {
                let between = text.get(child.end_byte()..cursor).unwrap_or("");
                if let Some(dot_pos) = between.find('.') {
                    let after_dot = &between[dot_pos + 1..];
                    return analyze_member_chain(text, &child, after_dot.to_string());
                }
            }
            if child.kind() == "call_expression" && cursor > child.end_byte() {
                let between = text.get(child.end_byte()..cursor).unwrap_or("");
                if let Some(dot_pos) = between.find('.') {
                    let after_dot = &between[dot_pos + 1..];
                    return analyze_member_chain(text, &child, after_dot.to_string());
                }
            }
        }
    }

    None
}

/// Minimal parser-failure fallback for explicit `db.` and `db.<collection>.` chains.
fn fallback_member_access(text: &str, cursor: usize) -> Option<ParsedContext> {
    let cursor = cursor.min(text.len());
    let prefix = text.get(..cursor)?;

    // Use the trailing expression chunk only.
    let expr_start = prefix
        .rfind(|c: char| c.is_whitespace() || matches!(c, ';' | ',' | '(' | ')' | '{' | '}'))
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let expr = prefix.get(expr_start..)?.trim();
    if !expr.starts_with("db.") {
        return None;
    }

    let rest = &expr[3..];
    if rest.is_empty() {
        return Some(ParsedContext {
            scope_kind: ScopeKind::DbMember,
            position_kind: PositionKind::MemberAccess,
            member_token: Some(String::new()),
            ..Default::default()
        });
    }

    // db.<partial>
    if !rest.contains('.') {
        if is_member_token(rest) {
            return Some(ParsedContext {
                scope_kind: ScopeKind::DbMember,
                position_kind: PositionKind::MemberAccess,
                member_token: Some(rest.to_string()),
                ..Default::default()
            });
        }
        return None;
    }

    // db.<collection>.<partial>
    let (collection, token) = rest.split_once('.')?;
    if collection.is_empty() || !is_member_token(collection) || !is_member_token(token) {
        return None;
    }

    Some(ParsedContext {
        scope_kind: ScopeKind::CollectionMember,
        position_kind: PositionKind::MemberAccess,
        collection: Some(collection.to_string()),
        member_token: Some(token.to_string()),
        ..Default::default()
    })
}

fn is_member_token(input: &str) -> bool {
    input.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
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
        // bare text with no db. prefix → TopLevel
        let ctx = ctx_at("x = 5|");
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

    // ── Member-access detection ────────────────────────────────────

    #[test]
    fn db_dot_cursor() {
        let ctx = ctx_at("db.|");
        assert_eq!(ctx.scope_kind, ScopeKind::DbMember);
        assert_eq!(ctx.position_kind, PositionKind::MemberAccess);
        assert_eq!(ctx.collection, None);
        assert_eq!(ctx.member_token.as_deref(), Some(""));
    }

    #[test]
    fn db_dot_partial() {
        let ctx = ctx_at("db.us|");
        assert_eq!(ctx.scope_kind, ScopeKind::DbMember);
        assert_eq!(ctx.position_kind, PositionKind::MemberAccess);
        assert_eq!(ctx.collection, None);
        assert_eq!(ctx.member_token.as_deref(), Some("us"));
    }

    #[test]
    fn collection_dot_cursor() {
        let ctx = ctx_at("db.users.|");
        assert_eq!(ctx.scope_kind, ScopeKind::CollectionMember);
        assert_eq!(ctx.position_kind, PositionKind::MemberAccess);
        assert_eq!(ctx.collection.as_deref(), Some("users"));
        assert_eq!(ctx.member_token.as_deref(), Some(""));
    }

    #[test]
    fn collection_dot_partial() {
        let ctx = ctx_at("db.users.fi|");
        assert_eq!(ctx.scope_kind, ScopeKind::CollectionMember);
        assert_eq!(ctx.position_kind, PositionKind::MemberAccess);
        assert_eq!(ctx.collection.as_deref(), Some("users"));
        assert_eq!(ctx.member_token.as_deref(), Some("fi"));
    }

    #[test]
    fn get_collection_dot_cursor() {
        let ctx = ctx_at("db.getCollection(\"users\").|");
        assert_eq!(ctx.scope_kind, ScopeKind::CollectionMember);
        assert_eq!(ctx.position_kind, PositionKind::MemberAccess);
        assert_eq!(ctx.collection.as_deref(), Some("users"));
        assert_eq!(ctx.member_token.as_deref(), Some(""));
    }

    #[test]
    fn chained_call_partial() {
        let ctx = ctx_at("db.users.find().so|");
        assert_eq!(ctx.scope_kind, ScopeKind::CollectionMember);
        assert_eq!(ctx.position_kind, PositionKind::MemberAccess);
        assert_eq!(ctx.collection.as_deref(), Some("users"));
        assert_eq!(ctx.member_token.as_deref(), Some("so"));
    }

    #[test]
    fn bracket_access_dot_cursor() {
        let ctx = ctx_at("db[\"users\"].|");
        assert_eq!(ctx.scope_kind, ScopeKind::CollectionMember);
        assert_eq!(ctx.position_kind, PositionKind::MemberAccess);
        assert_eq!(ctx.collection.as_deref(), Some("users"));
        assert_eq!(ctx.member_token.as_deref(), Some(""));
    }

    #[test]
    fn bracket_access_partial() {
        let ctx = ctx_at("db[\"users\"].fi|");
        assert_eq!(ctx.scope_kind, ScopeKind::CollectionMember);
        assert_eq!(ctx.position_kind, PositionKind::MemberAccess);
        assert_eq!(ctx.collection.as_deref(), Some("users"));
        assert_eq!(ctx.member_token.as_deref(), Some("fi"));
    }

    #[test]
    fn bare_text_is_top_level() {
        let ctx = ctx_at("x = 5|");
        assert_eq!(ctx.scope_kind, ScopeKind::TopLevel);
        assert_eq!(ctx.position_kind, PositionKind::Unknown);
        assert_eq!(ctx.collection, None);
        assert!(ctx.member_token.is_none());
    }

    #[test]
    fn find_empty_first_argument_slot() {
        let ctx = ctx_at("db.c.find(|)");
        assert_eq!(ctx.scope_kind, ScopeKind::FindFilter);
    }

    #[test]
    fn update_second_argument_slot() {
        let ctx = ctx_at("db.c.updateOne({}, |)");
        assert_eq!(ctx.scope_kind, ScopeKind::UpdateDoc);
    }

    #[test]
    fn quoted_match_operator_scope() {
        let ctx = ctx_at("db.c.aggregate([{ \"$match\": { n| } }])");
        assert_eq!(ctx.scope_kind, ScopeKind::MatchFilter);
        assert_eq!(ctx.position_kind, PositionKind::Key);
    }

    #[test]
    fn quoted_query_operator_value_scope() {
        let ctx = ctx_at("db.c.find({ name: { \"$gt\": 5| } })");
        assert_eq!(ctx.scope_kind, ScopeKind::OperatorValue);
        assert_eq!(ctx.position_kind, PositionKind::Value);
    }

    #[test]
    fn update_operator_maps_to_set_doc_scope() {
        let ctx = ctx_at("db.c.updateOne({}, { $inc: { co|: 1 } })");
        assert_eq!(ctx.scope_kind, ScopeKind::SetDoc);
        assert_eq!(ctx.position_kind, PositionKind::Key);
    }
}
