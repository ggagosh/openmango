use std::collections::HashSet;

use mongodb::bson::{Bson, Document};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub kind: SuggestionKind,
    pub insert_text: String,
    pub is_snippet: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    Collection,
    Method,
    Operator,
    Field,
}

impl SuggestionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SuggestionKind::Collection => "Collection",
            SuggestionKind::Method => "Method",
            SuggestionKind::Operator => "Operator",
            SuggestionKind::Field => "Field",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextKind {
    /// After "db." - show collections
    Collections,
    /// After "db.collectionName." - show methods
    Methods,
    /// After "$" - show operators
    Operators,
}

pub fn statement_bounds(text: &str, cursor: usize) -> (usize, usize) {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut start = cursor.min(len);
    let mut end = cursor.min(len);

    let mut i = start;
    while i > 0 {
        let b = bytes[i - 1];
        if b == b';' {
            start = i;
            break;
        }
        if i >= 2 && bytes[i - 2] == b'\n' && b == b'\n' {
            start = i;
            break;
        }
        i -= 1;
        start = i;
    }

    let mut j = end;
    while j < len {
        let b = bytes[j];
        if b == b';' {
            end = j + 1;
            break;
        }
        if j + 1 < len && b == b'\n' && bytes[j + 1] == b'\n' {
            end = j + 1;
            break;
        }
        j += 1;
        end = j;
    }

    (start, end)
}

pub fn completion_token(line_prefix: &str, context: Option<ContextKind>) -> (String, usize) {
    match context {
        Some(ContextKind::Collections) => {
            if let Some(db_pos) = line_prefix.rfind("db.") {
                let start = db_pos + 3;
                return (line_prefix[start..].to_string(), start);
            }
        }
        Some(ContextKind::Methods) => {
            if let Some(dot_pos) = line_prefix.rfind('.') {
                let start = dot_pos + 1;
                return (line_prefix[start..].to_string(), start);
            }
        }
        Some(ContextKind::Operators) => {
            if let Some(dollar_pos) = line_prefix.rfind('$') {
                let start = dollar_pos;
                return (line_prefix[start..].to_string(), start);
            }
        }
        None => {}
    }
    (String::new(), line_prefix.len())
}

fn matches_token(candidate: &str, token: &str, context: Option<ContextKind>) -> bool {
    if token.is_empty() {
        return true;
    }
    let candidate = if matches!(context, Some(ContextKind::Methods)) {
        candidate.split_once('(').map(|(base, _)| base).unwrap_or(candidate)
    } else {
        candidate
    };
    candidate.starts_with(token)
}

pub fn merge_suggestions(
    local: Vec<Suggestion>,
    mongosh: Vec<String>,
    context: Option<ContextKind>,
    completion_prefix: &str,
    token: &str,
) -> Vec<Suggestion> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for suggestion in local {
        if !matches_token(&suggestion.label, token, context) {
            continue;
        }
        let label = suggestion.label.clone();
        if seen.insert(label.clone()) {
            out.push(suggestion);
        }
        if let Some((base, _)) = label.split_once('(')
            && !base.is_empty()
        {
            seen.insert(base.to_string());
        }
    }

    for completion in mongosh {
        let suffix = strip_completion_prefix(&completion, completion_prefix);
        let normalized = normalize_completion(&suffix, context);
        if normalized.is_empty() {
            continue;
        }
        if !matches_token(&normalized, token, context) {
            continue;
        }
        let looks_like_operator = normalized.starts_with('$');
        let suggestion = match context {
            Some(ContextKind::Collections) => db_method_template(&normalized)
                .map(|template| Suggestion {
                    label: label_from_template(template),
                    kind: SuggestionKind::Method,
                    insert_text: template.to_string(),
                    is_snippet: template.contains('$'),
                })
                .unwrap_or_else(|| Suggestion {
                    label: normalized.clone(),
                    kind: SuggestionKind::Collection,
                    insert_text: normalized,
                    is_snippet: false,
                }),
            Some(ContextKind::Methods) => collection_method_template(&normalized)
                .map(|template| Suggestion {
                    label: label_from_template(template),
                    kind: SuggestionKind::Method,
                    insert_text: template.to_string(),
                    is_snippet: template.contains('$'),
                })
                .unwrap_or_else(|| Suggestion {
                    label: normalized.clone(),
                    kind: SuggestionKind::Method,
                    insert_text: normalized,
                    is_snippet: false,
                }),
            Some(ContextKind::Operators) => Suggestion {
                label: normalized.clone(),
                kind: SuggestionKind::Operator,
                insert_text: format!("{}: ", normalized),
                is_snippet: false,
            },
            None => Suggestion {
                label: normalized.clone(),
                kind: if looks_like_operator {
                    SuggestionKind::Operator
                } else {
                    SuggestionKind::Method
                },
                insert_text: if looks_like_operator {
                    format!("{}: ", normalized)
                } else {
                    normalized
                },
                is_snippet: false,
            },
        };

        if seen.insert(suggestion.label.clone()) {
            out.push(suggestion);
        }
    }

    out
}

fn normalize_completion(completion: &str, context: Option<ContextKind>) -> String {
    let completion = completion.trim();
    if completion.is_empty() {
        return String::new();
    }

    match context {
        Some(ContextKind::Collections) => {
            completion.strip_prefix("db.").unwrap_or(completion).to_string()
        }
        Some(ContextKind::Methods) => {
            completion.rsplit('.').next().unwrap_or(completion).to_string()
        }
        Some(ContextKind::Operators) | None => completion.to_string(),
    }
}

fn strip_completion_prefix(completion: &str, prefix: &str) -> String {
    if completion.is_empty() {
        return String::new();
    }

    if let Some(stripped) = completion.strip_prefix(prefix) {
        return stripped.trim_start_matches(['.', ' ', '\t']).to_string();
    }

    let trimmed_prefix = prefix.trim_end();
    if trimmed_prefix.len() != prefix.len()
        && let Some(stripped) = completion.strip_prefix(trimmed_prefix)
    {
        return stripped.trim_start_matches(['.', ' ', '\t']).to_string();
    }

    completion.trim_start_matches(['.', ' ', '\t']).to_string()
}

pub fn detect_context(text: &str) -> Option<ContextKind> {
    if let Some(dollar_pos) = text.rfind('$') {
        let after_dollar = &text[dollar_pos + 1..];
        if !after_dollar.contains(':') && !after_dollar.contains(' ') && !after_dollar.contains('}')
        {
            return Some(ContextKind::Operators);
        }
    }

    if let Some(last_dot) = text.rfind('.') {
        let after_dot = &text[last_dot + 1..];
        if after_dot.chars().all(|c| c.is_alphanumeric() || c == '_') {
            let before_dot = &text[..last_dot];
            if looks_like_collection_access(before_dot) {
                return Some(ContextKind::Methods);
            }
        }
    }

    if let Some(db_pos) = text.rfind("db.") {
        let after_db = &text[db_pos + 3..];
        if !after_db.contains('.') && after_db.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Some(ContextKind::Collections);
        }
    }

    None
}

fn looks_like_collection_access(text: &str) -> bool {
    let trimmed = text.trim_end();
    let Some(rest) = trimmed.strip_prefix("db.") else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }

    if rest.starts_with('[') {
        return rest.ends_with(']');
    }

    if let Some((name, _args)) = rest.split_once('(') {
        let name = name.trim_end();
        if name == "getCollection" {
            return rest.ends_with(')');
        }
        return false;
    }

    rest.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

pub fn should_skip_completion(line_prefix: &str) -> bool {
    let trimmed = line_prefix.trim_start();
    if trimmed.starts_with("//") {
        return true;
    }
    if let Some(start) = trimmed.find("/*") {
        let rest = &trimmed[start + 2..];
        if !rest.contains("*/") {
            return true;
        }
    }
    false
}

pub fn db_method_template(name: &str) -> Option<&'static str> {
    match name {
        "stats" => Some("stats()"),
        "getCollection" => Some("getCollection(\"$1\")$0"),
        "getSiblingDB" => Some("getSiblingDB(\"$1\")$0"),
        "runCommand" => Some("runCommand({$1})$0"),
        "listCollections" => Some("listCollections({$1})$0"),
        "createCollection" => Some("createCollection(\"$1\")$0"),
        _ => None,
    }
}

pub fn collection_method_template(name: &str) -> Option<&'static str> {
    match name {
        "find" => Some("find({$1})$0"),
        "findOne" => Some("findOne({$1})$0"),
        "aggregate" => Some("aggregate([{$1}])$0"),
        "insertOne" => Some("insertOne({$1})$0"),
        "insertMany" => Some("insertMany([{$1}])$0"),
        "updateOne" => Some("updateOne({$1}, {$2})$0"),
        "updateMany" => Some("updateMany({$1}, {$2})$0"),
        "deleteOne" => Some("deleteOne({$1})$0"),
        "deleteMany" => Some("deleteMany({$1})$0"),
        "countDocuments" => Some("countDocuments({$1})$0"),
        "distinct" => Some("distinct(\"$1\")$0"),
        "createIndex" => Some("createIndex({$1})$0"),
        "dropIndex" => Some("dropIndex(\"$1\")$0"),
        "getIndexes" => Some("getIndexes()"),
        _ => None,
    }
}

pub fn label_from_template(template: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '$' {
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                let mut j = i + 2;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == ':' {
                    j += 1;
                    let default_start = j;
                    while j < chars.len() && chars[j] != '}' {
                        j += 1;
                    }
                    out.extend(chars[default_start..j].iter());
                    if j < chars.len() && chars[j] == '}' {
                        j += 1;
                    }
                    i = j;
                    continue;
                }
                if j < chars.len() && chars[j] == '}' {
                    i = j + 1;
                    continue;
                }
            }
            if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

pub const METHODS: &[&str] = &[
    "find",
    "findOne",
    "aggregate",
    "insertOne",
    "insertMany",
    "updateOne",
    "updateMany",
    "deleteOne",
    "deleteMany",
    "countDocuments",
    "distinct",
    "createIndex",
    "dropIndex",
    "getIndexes",
];

pub const PIPELINE_OPERATORS: &[&str] = &[
    "$match",
    "$project",
    "$group",
    "$sort",
    "$limit",
    "$skip",
    "$unwind",
    "$lookup",
    "$addFields",
    "$set",
    "$unset",
    "$replaceRoot",
    "$replaceWith",
    "$bucket",
    "$bucketAuto",
    "$count",
    "$facet",
    "$out",
    "$merge",
    "$sample",
    "$unionWith",
    "$redact",
    "$graphLookup",
];

pub const QUERY_OPERATORS: &[&str] = &[
    "$eq",
    "$ne",
    "$gt",
    "$gte",
    "$lt",
    "$lte",
    "$in",
    "$nin",
    "$exists",
    "$regex",
    "$and",
    "$or",
    "$nor",
    "$not",
    "$elemMatch",
    "$size",
    "$all",
    "$type",
];

pub const UPDATE_OPERATORS: &[&str] = &[
    "$set",
    "$unset",
    "$inc",
    "$push",
    "$addToSet",
    "$pull",
    "$pop",
    "$rename",
    "$mul",
    "$min",
    "$max",
    "$currentDate",
];

pub fn format_printable_lines(printable: &serde_json::Value) -> Vec<String> {
    if printable.is_null() {
        return vec!["null".to_string()];
    }
    if let Some(text) = printable.as_str() {
        if text.is_empty() {
            return Vec::new();
        }
        return text.split('\n').map(|line| line.to_string()).collect();
    }
    let text = serde_json::to_string_pretty(printable).unwrap_or_else(|_| printable.to_string());
    text.split('\n').map(|line| line.to_string()).collect()
}

pub fn is_trivial_printable(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            trimmed.is_empty() || trimmed.eq_ignore_ascii_case("undefined")
        }
        _ => false,
    }
}

pub fn default_result_label_for_value(value: &serde_json::Value) -> String {
    if value.is_array() {
        "Shell Output (Array)".to_string()
    } else {
        "Shell Output (Documents)".to_string()
    }
}

pub fn result_documents(printable: &serde_json::Value) -> Option<Vec<Document>> {
    if let Some(text) = printable.as_str() {
        let trimmed = text.trim();
        if (trimmed.starts_with('{') || trimmed.starts_with('['))
            && let Ok(docs) = crate::bson::parse_documents_from_json(trimmed)
        {
            return Some(docs);
        }
        return None;
    }

    if let Some(docs) = cursor_documents(printable) {
        return Some(docs);
    }

    match printable {
        serde_json::Value::Object(_) => {
            let bson =
                Bson::try_from(printable.clone()).unwrap_or_else(|_| value_to_bson(printable));
            if let Bson::Document(doc) = bson {
                return Some(vec![doc]);
            }
            None
        }
        serde_json::Value::Array(items) => {
            let mut docs = Vec::with_capacity(items.len());
            for item in items {
                let bson = value_to_bson(item);
                if let Bson::Document(doc) = bson {
                    docs.push(doc);
                } else {
                    return None;
                }
            }
            if docs.is_empty() { None } else { Some(docs) }
        }
        _ => None,
    }
}

fn cursor_documents(printable: &serde_json::Value) -> Option<Vec<Document>> {
    let obj = printable.as_object()?;
    let docs = obj.get("documents")?.as_array()?;
    if docs.is_empty() {
        return None;
    }

    let mut out = Vec::with_capacity(docs.len());
    for item in docs {
        match value_to_bson(item) {
            Bson::Document(doc) => out.push(doc),
            other => {
                let mut doc = Document::new();
                doc.insert("value", other);
                out.push(doc);
            }
        }
    }

    if out.is_empty() { None } else { Some(out) }
}

fn value_to_bson(value: &serde_json::Value) -> Bson {
    match value {
        serde_json::Value::Null => Bson::Null,
        serde_json::Value::Bool(val) => Bson::Boolean(*val),
        serde_json::Value::Number(num) => {
            if let Some(val) = num.as_i64() {
                Bson::Int64(val)
            } else if let Some(val) = num.as_u64() {
                if val <= i64::MAX as u64 {
                    Bson::Int64(val as i64)
                } else if let Some(val) = num.as_f64() {
                    Bson::Double(val)
                } else {
                    Bson::String(num.to_string())
                }
            } else if let Some(val) = num.as_f64() {
                Bson::Double(val)
            } else {
                Bson::String(num.to_string())
            }
        }
        serde_json::Value::String(val) => Bson::String(val.clone()),
        serde_json::Value::Array(items) => Bson::Array(items.iter().map(value_to_bson).collect()),
        serde_json::Value::Object(map) => {
            let mut doc = Document::new();
            for (key, val) in map {
                doc.insert(key, value_to_bson(val));
            }
            Bson::Document(doc)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statement_bounds_respects_semicolons() {
        let text = "db.stats();\n\ndb.getCollection(\"x\")";
        let (start, end) = statement_bounds(text, 5);
        assert_eq!(&text[start..end], "db.stats();");
    }

    #[test]
    fn statement_bounds_falls_back_to_paragraph() {
        let text = "db.stats()\n\n// comment\n";
        let (start, end) = statement_bounds(text, 2);
        assert_eq!(&text[start..end], "db.stats()\n");
    }

    #[test]
    fn completion_token_handles_db_context() {
        let (token, start) = completion_token("db.getC", Some(ContextKind::Collections));
        assert_eq!(token, "getC");
        assert_eq!(start, 3);
    }
}
