use std::fmt;

use mongodb::bson::{Bson, Document, oid::ObjectId};

use crate::bson::{
    format_relaxed_json_compact, parse_bson_from_relaxed_json, parse_document_from_json,
};

#[derive(Clone, Debug)]
pub(super) struct CompiledFilter {
    pub raw_store: String,
    pub document: Option<Document>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FastFilterErrorKind {
    Incomplete,
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FastFilterError {
    kind: FastFilterErrorKind,
    message: String,
}

impl FastFilterError {
    fn incomplete(message: impl Into<String>) -> Self {
        Self { kind: FastFilterErrorKind::Incomplete, message: message.into() }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self { kind: FastFilterErrorKind::Invalid, message: message.into() }
    }

    pub(super) fn is_incomplete(&self) -> bool {
        self.kind == FastFilterErrorKind::Incomplete
    }
}

impl fmt::Display for FastFilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FastOperator {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    Regex,
}

struct ParsedInlineCondition<'a> {
    field: &'a str,
    op: FastOperator,
    value: &'a str,
    consumed: usize,
}

pub(super) fn compile_filter_input(raw: &str) -> Result<CompiledFilter, FastFilterError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return Ok(CompiledFilter { raw_store: String::new(), document: None });
    }

    let doc = if trimmed.starts_with('{') {
        parse_document_from_json(trimmed).map_err(FastFilterError::invalid)?
    } else if looks_like_document_body(trimmed) {
        match parse_document_from_json(&format!("{{{trimmed}}}")) {
            Ok(doc) => doc,
            Err(err) => {
                let fast_input = top_level_commas_to_spaces(trimmed);
                parse_fast_filter(&fast_input).map_err(|_| FastFilterError::invalid(err))?
            }
        }
    } else {
        parse_fast_filter(trimmed)?
    };

    Ok(compiled_from_document(doc))
}

pub(super) fn format_compiled_filter(compiled: &CompiledFilter) -> String {
    if compiled.document.is_none() { "{}".to_string() } else { compiled.raw_store.clone() }
}

fn compiled_from_document(doc: Document) -> CompiledFilter {
    if doc.is_empty() {
        return CompiledFilter { raw_store: String::new(), document: None };
    }

    let value = Bson::Document(doc.clone()).into_relaxed_extjson();
    CompiledFilter { raw_store: format_relaxed_json_compact(&value), document: Some(doc) }
}

fn looks_like_document_body(trimmed: &str) -> bool {
    trimmed.contains(':')
        && (trimmed.contains(',')
            || trimmed.contains('{')
            || trimmed.contains('}')
            || trimmed.contains('$')
            || trimmed.starts_with('"')
            || trimmed.starts_with('\''))
}

fn parse_fast_filter(input: &str) -> Result<Document, FastFilterError> {
    let tokens = tokenize_fast_filter(input);
    if tokens.is_empty() {
        return Ok(Document::new());
    }

    let mut conditions = Vec::new();
    let mut i = 0usize;

    while i < tokens.len() {
        let token = tokens[i].trim();
        if token.is_empty() {
            i += 1;
            continue;
        }

        if let Some(field) = token.strip_prefix('!')
            && !field.is_empty()
            && !contains_inline_operator(field)
        {
            conditions.push(field_condition(field, operator_doc("$ne", Bson::Boolean(true))));
            i += 1;
            continue;
        }

        if i + 1 < tokens.len() && tokens[i + 1].eq_ignore_ascii_case("exists") {
            conditions.push(field_condition(token, operator_doc("$exists", Bson::Boolean(true))));
            i += 2;
            continue;
        }

        if i + 1 < tokens.len() && tokens[i + 1].eq_ignore_ascii_case("missing") {
            conditions.push(field_condition(token, operator_doc("$exists", Bson::Boolean(false))));
            i += 2;
            continue;
        }

        if i + 2 < tokens.len()
            && tokens[i + 1].eq_ignore_ascii_case("not")
            && tokens[i + 2].eq_ignore_ascii_case("in")
        {
            let values = tokens.get(i + 3).ok_or_else(|| {
                FastFilterError::incomplete(format!("Expected values after `{token} not in`"))
            })?;
            conditions.push(field_condition(
                token,
                operator_doc("$nin", Bson::Array(parse_list_values(token, values)?)),
            ));
            i += 4;
            continue;
        }

        if i + 1 < tokens.len() && tokens[i + 1].eq_ignore_ascii_case("in") {
            let values = tokens.get(i + 2).ok_or_else(|| {
                FastFilterError::incomplete(format!("Expected values after `{token} in`"))
            })?;
            conditions.push(field_condition(
                token,
                operator_doc("$in", Bson::Array(parse_list_values(token, values)?)),
            ));
            i += 3;
            continue;
        }

        let parsed = parse_inline_condition(&tokens, i)?;
        let value = if parsed.op == FastOperator::Regex {
            regex_condition(parsed.value)?
        } else {
            let mut parsed_value = parse_filter_value(parsed.field, parsed.value)?;
            if parsed.op == FastOperator::Ne
                && let Some(stripped) = parsed.value.trim().strip_prefix('!')
            {
                parsed_value = parse_filter_value(parsed.field, stripped)?;
            }
            match parsed.op {
                FastOperator::Eq => parsed_value,
                FastOperator::Ne => operator_doc("$ne", parsed_value),
                FastOperator::Gt => operator_doc("$gt", parsed_value),
                FastOperator::Gte => operator_doc("$gte", parsed_value),
                FastOperator::Lt => operator_doc("$lt", parsed_value),
                FastOperator::Lte => operator_doc("$lte", parsed_value),
                FastOperator::Regex => unreachable!(),
            }
        };

        conditions.push(field_condition(parsed.field, value));
        i += parsed.consumed;
    }

    combine_conditions(conditions)
}

fn parse_inline_condition<'a>(
    tokens: &'a [String],
    index: usize,
) -> Result<ParsedInlineCondition<'a>, FastFilterError> {
    let token = tokens[index].trim();

    if index + 2 < tokens.len()
        && let Some(op) = standalone_operator(tokens[index + 1].trim())
    {
        return Ok(ParsedInlineCondition {
            field: token,
            op,
            value: tokens[index + 2].trim(),
            consumed: 3,
        });
    }

    let Some((field, op, value)) = split_inline_operator(token) else {
        return Err(FastFilterError::incomplete(format!(
            "Expected a filter operator after `{token}`. Try `{token}:value`, `{token}>10`, or `{token} in a,b`."
        )));
    };

    if field.trim().is_empty() {
        return Err(FastFilterError::invalid("Expected a field before the filter operator"));
    }

    let value = value.trim();
    if value.is_empty() {
        let next = tokens.get(index + 1).ok_or_else(|| {
            FastFilterError::incomplete(format!("Expected a value after `{field}`"))
        })?;
        return Ok(ParsedInlineCondition { field, op, value: next.trim(), consumed: 2 });
    }

    Ok(ParsedInlineCondition { field, op, value, consumed: 1 })
}

fn split_inline_operator(token: &str) -> Option<(&str, FastOperator, &str)> {
    for (needle, op) in [
        (">=", FastOperator::Gte),
        ("<=", FastOperator::Lte),
        ("!=", FastOperator::Ne),
        (":!", FastOperator::Ne),
        (":", FastOperator::Eq),
        ("=", FastOperator::Eq),
        ("~", FastOperator::Regex),
        (">", FastOperator::Gt),
        ("<", FastOperator::Lt),
    ] {
        if let Some(idx) = token.find(needle) {
            let value_start = idx + needle.len();
            return Some((&token[..idx], op, &token[value_start..]));
        }
    }
    None
}

fn standalone_operator(token: &str) -> Option<FastOperator> {
    match token {
        ":" | "=" => Some(FastOperator::Eq),
        "!=" | ":!" => Some(FastOperator::Ne),
        ">" => Some(FastOperator::Gt),
        ">=" => Some(FastOperator::Gte),
        "<" => Some(FastOperator::Lt),
        "<=" => Some(FastOperator::Lte),
        "~" => Some(FastOperator::Regex),
        _ => None,
    }
}

fn contains_inline_operator(token: &str) -> bool {
    [">=", "<=", "!=", ":!", ":", "=", "~", ">", "<"].iter().any(|op| token.contains(op))
}

fn field_condition(field: &str, value: Bson) -> Document {
    let mut doc = Document::new();
    doc.insert(field.trim(), value);
    doc
}

fn operator_doc(operator: &str, value: Bson) -> Bson {
    let mut doc = Document::new();
    doc.insert(operator, value);
    Bson::Document(doc)
}

fn regex_condition(raw: &str) -> Result<Bson, FastFilterError> {
    let pattern = regex_pattern(raw)?;
    let mut doc = Document::new();
    doc.insert("$regex", Bson::String(pattern));
    doc.insert("$options", Bson::String("i".to_string()));
    Ok(Bson::Document(doc))
}

fn regex_pattern(raw: &str) -> Result<String, FastFilterError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FastFilterError::incomplete("Expected a regex value"));
    }
    if trimmed.len() >= 2 && trimmed.starts_with('/') && trimmed.ends_with('/') {
        return Ok(trimmed[1..trimmed.len() - 1].to_string());
    }
    match parse_filter_value("", trimmed)? {
        Bson::String(value) => Ok(value),
        other => Ok(other.to_string()),
    }
}

fn combine_conditions(conditions: Vec<Document>) -> Result<Document, FastFilterError> {
    let mut root = Document::new();
    for condition in conditions {
        for (field, value) in condition {
            if let Some(existing) = root.get_mut(&field) {
                match merge_operator_condition(existing, value) {
                    Ok(()) => continue,
                    Err(value) => return Ok(and_document(root, field, value)),
                }
            }
            root.insert(field, value);
        }
    }
    Ok(root)
}

fn merge_operator_condition(existing: &mut Bson, value: Bson) -> Result<(), Bson> {
    let Bson::Document(existing_doc) = existing else {
        return Err(value);
    };
    if !existing_doc.keys().all(|key| key.starts_with('$')) {
        return Err(value);
    }

    let Bson::Document(next_doc) = value else {
        return Err(value);
    };
    if !next_doc.keys().all(|key| key.starts_with('$'))
        || next_doc.keys().any(|key| existing_doc.contains_key(key))
    {
        return Err(Bson::Document(next_doc));
    }

    for (key, value) in next_doc {
        existing_doc.insert(key, value);
    }
    Ok(())
}

fn and_document(existing: Document, field: String, value: Bson) -> Document {
    let mut clauses: Vec<Bson> = existing
        .into_iter()
        .map(|(field, value)| Bson::Document(field_condition(&field, value)))
        .collect();
    clauses.push(Bson::Document(field_condition(&field, value)));

    let mut root = Document::new();
    root.insert("$and", Bson::Array(clauses));
    root
}

fn parse_filter_value(field: &str, raw: &str) -> Result<Bson, FastFilterError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FastFilterError::incomplete("Expected a filter value"));
    }

    if should_parse_object_id(field, trimmed) {
        return ObjectId::parse_str(trimmed)
            .map(Bson::ObjectId)
            .map_err(|err| FastFilterError::invalid(err.to_string()));
    }

    if let Ok(value) = parse_bson_from_relaxed_json(trimmed) {
        return Ok(value);
    }

    Ok(Bson::String(unquote(trimmed).to_string()))
}

fn parse_list_values(field: &str, raw: &str) -> Result<Vec<Bson>, FastFilterError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FastFilterError::incomplete("Expected values after `in`"));
    }

    if let Ok(Bson::Array(values)) = parse_bson_from_relaxed_json(trimmed) {
        return Ok(values);
    }

    let inner =
        trimmed.strip_prefix('[').and_then(|value| value.strip_suffix(']')).unwrap_or(trimmed);
    let values = split_list_values(inner)
        .into_iter()
        .map(|value| parse_filter_value(field, value.trim()))
        .collect::<Result<Vec<_>, _>>()?;

    if values.is_empty() {
        return Err(FastFilterError::incomplete("Expected at least one value after `in`"));
    }

    Ok(values)
}

fn should_parse_object_id(field: &str, value: &str) -> bool {
    !is_quoted(value)
        && is_object_id_field(field)
        && value.len() == 24
        && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_object_id_field(field: &str) -> bool {
    let segment = field.rsplit('.').next().unwrap_or(field).trim();
    let lower = segment.to_ascii_lowercase();
    lower == "_id"
        || lower == "id"
        || lower.ends_with("_id")
        || segment.ends_with("Id")
        || segment.ends_with("ID")
}

fn split_list_values(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut string_delim = '\0';
    let mut escape = false;

    for (idx, ch) in input.char_indices() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == string_delim {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' | '\'' => {
                in_string = true;
                string_delim = ch;
            }
            '[' | '{' | '(' => depth += 1,
            ']' | '}' | ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let part = input[start..idx].trim();
                if !part.is_empty() {
                    parts.push(part.to_string());
                }
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }

    let tail = input[start..].trim();
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }
    parts
}

fn top_level_commas_to_spaces(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut depth = 0usize;
    let mut in_string = false;
    let mut string_delim = '\0';
    let mut escape = false;

    for ch in input.chars() {
        if in_string {
            out.push(ch);
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == string_delim {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' | '\'' => {
                in_string = true;
                string_delim = ch;
                out.push(ch);
            }
            '[' | '{' | '(' => {
                depth += 1;
                out.push(ch);
            }
            ']' | '}' | ')' => {
                depth = depth.saturating_sub(1);
                out.push(ch);
            }
            ',' if depth == 0 => out.push(' '),
            _ => out.push(ch),
        }
    }

    out
}

fn tokenize_fast_filter(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut string_delim = '\0';
    let mut escape = false;

    for (idx, ch) in input.char_indices() {
        if start.is_none() && !ch.is_whitespace() {
            start = Some(idx);
        }

        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == string_delim {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' | '\'' => {
                in_string = true;
                string_delim = ch;
            }
            '[' | '{' | '(' => depth += 1,
            ']' | '}' | ')' => depth = depth.saturating_sub(1),
            _ if ch.is_whitespace() && depth == 0 => {
                if let Some(token_start) = start.take()
                    && token_start < idx
                {
                    tokens.push(input[token_start..idx].to_string());
                }
            }
            _ => {}
        }
    }

    if let Some(token_start) = start
        && token_start < input.len()
    {
        tokens.push(input[token_start..].to_string());
    }

    tokens
}

fn unquote(value: &str) -> &str {
    if !is_quoted(value) {
        return value;
    }
    &value[1..value.len() - 1]
}

fn is_quoted(value: &str) -> bool {
    if value.len() < 2 {
        return false;
    }
    let bytes = value.as_bytes();
    (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
        || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
}

#[cfg(test)]
mod tests {
    use super::{compile_filter_input, format_compiled_filter};

    fn formatted(raw: &str) -> String {
        let compiled = compile_filter_input(raw).expect("compile filter");
        format_compiled_filter(&compiled)
    }

    #[test]
    fn compiles_fast_equality_filter() {
        assert_eq!(formatted("status:active"), "{status: \"active\"}");
        assert_eq!(formatted("status:active,type:user"), "{status: \"active\", type: \"user\"}");
        assert_eq!(formatted("name: \"alice\""), "{name: \"alice\"}");
    }

    #[test]
    fn compiles_fast_comparison_filters() {
        assert_eq!(formatted("age>30"), "{age: {$gt: 30}}");
        assert_eq!(formatted("age>30 age<50"), "{age: {$gt: 30, $lt: 50}}");
        assert_eq!(
            formatted("createdAt>=ISODate(\"2024-01-01T00:00:00Z\")"),
            "{createdAt: {$gte: ISODate(\"2024-01-01T00:00:00Z\")}}"
        );
    }

    #[test]
    fn compiles_fast_contains_and_negation_filters() {
        assert_eq!(
            formatted("email~gmail !deleted"),
            "{email: {$regex: \"gmail\", $options: \"i\"}, deleted: {$ne: true}}"
        );
    }

    #[test]
    fn compiles_fast_in_filters() {
        assert_eq!(formatted("plan in pro,team"), "{plan: {$in: [\"pro\", \"team\"]}}");
        assert_eq!(formatted("plan not in [free,team]"), "{plan: {$nin: [\"free\", \"team\"]}}");
    }

    #[test]
    fn smart_converts_bare_object_ids_for_id_fields() {
        assert_eq!(
            formatted("_id:6392478cbdd1f183c69543c3"),
            "{_id: ObjectId(\"6392478cbdd1f183c69543c3\")}"
        );
        assert_eq!(
            formatted("ownerId:6392478cbdd1f183c69543c3"),
            "{ownerId: ObjectId(\"6392478cbdd1f183c69543c3\")}"
        );
        assert_eq!(
            formatted("_id in 6392478cbdd1f183c69543c3,6392478cbdd1f183c69543c4"),
            "{_id: {$in: [ObjectId(\"6392478cbdd1f183c69543c3\"), ObjectId(\"6392478cbdd1f183c69543c4\")]}}"
        );
    }

    #[test]
    fn smart_object_id_conversion_is_field_aware_and_quote_safe() {
        assert_eq!(
            formatted("token:6392478cbdd1f183c69543c3"),
            "{token: \"6392478cbdd1f183c69543c3\"}"
        );
        assert_eq!(
            formatted("_id:\"6392478cbdd1f183c69543c3\""),
            "{_id: \"6392478cbdd1f183c69543c3\"}"
        );
    }

    #[test]
    fn preserves_raw_document_filters() {
        assert_eq!(formatted("name:\"alice\",age:1"), "{name: \"alice\", age: 1}");
        assert_eq!(formatted("{ status: { $ne: \"archived\" } }"), "{status: {$ne: \"archived\"}}");
    }

    #[test]
    fn reports_incomplete_fast_filters_without_accepting_them() {
        let err = compile_filter_input("age>").expect_err("missing value should be incomplete");
        assert!(err.is_incomplete());
    }
}
