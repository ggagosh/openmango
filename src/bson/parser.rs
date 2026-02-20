//! BSON parsing utilities for converting between formats.

use mongodb::bson::{self, Bson, DateTime, Document, oid::ObjectId};
use serde_json::Value;

/// Parse JSON or JSON5 into a serde_json Value.
pub fn parse_value_from_relaxed_json(input: &str) -> Result<Value, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Input is empty".to_string());
    }

    let preprocessed = preprocess_shell_syntax(trimmed);
    serde_json::from_str(&preprocessed)
        .or_else(|_| json5::from_str(&preprocessed).map_err(|e| e.to_string()))
}

/// Parse JSON or JSON5 into BSON.
pub fn parse_bson_from_relaxed_json(input: &str) -> Result<Bson, String> {
    let value = parse_value_from_relaxed_json(input)?;
    bson::Bson::try_from(value).map_err(|e| e.to_string())
}

fn preprocess_shell_syntax(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    let mut in_string = false;
    let mut string_delim = b'"';
    let mut escape = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < bytes.len() {
        let b = bytes[i];

        // Non-ASCII bytes are always content (never delimiters or comment markers).
        // Decode the full UTF-8 char to avoid corrupting multi-byte characters.
        if !b.is_ascii() {
            if let Some(ch) = input[i..].chars().next() {
                out.push(ch);
                i += ch.len_utf8();
            } else {
                i += 1;
            }
            continue;
        }

        if in_line_comment {
            out.push(b as char);
            if b == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }

        if in_block_comment {
            out.push(b as char);
            if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
                out.push('/');
                i += 2;
                in_block_comment = false;
                continue;
            }
            i += 1;
            continue;
        }

        if in_string {
            out.push(b as char);
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == string_delim {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            out.push('/');
            out.push('/');
            i += 2;
            in_line_comment = true;
            continue;
        }

        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            out.push('/');
            out.push('*');
            i += 2;
            in_block_comment = true;
            continue;
        }

        if b == b'"' || b == b'\'' {
            in_string = true;
            string_delim = b;
            out.push(b as char);
            i += 1;
            continue;
        }

        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let name = &input[start..i];
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len()
                && bytes[j] == b'('
                && is_shell_constructor(name)
                && let Some((end, args)) = parse_call_args(input, j)
                && let Some(replacement) = convert_shell_constructor(name, &args)
            {
                out.push_str(&replacement);
                i = end;
                continue;
            }
            out.push_str(name);
            continue;
        }

        out.push(b as char);
        i += 1;
    }

    out
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn is_shell_constructor(name: &str) -> bool {
    matches!(
        name,
        "ObjectId"
            | "ObjectID"
            | "ISODate"
            | "Date"
            | "NumberLong"
            | "NumberInt"
            | "NumberDecimal"
            | "NumberDouble"
            | "Timestamp"
            | "UUID"
    )
}

fn parse_call_args(input: &str, open_paren: usize) -> Option<(usize, Vec<String>)> {
    let bytes = input.as_bytes();
    let mut i = open_paren + 1;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut string_delim = b'"';
    let mut escape = false;
    let args_start = open_paren + 1;

    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == string_delim {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if b == b'"' || b == b'\'' {
            in_string = true;
            string_delim = b;
            i += 1;
            continue;
        }

        if b == b'(' {
            depth += 1;
        } else if b == b')' {
            if depth == 0 {
                let args_str = &input[args_start..i];
                let args = split_args(args_str);
                return Some((i + 1, args));
            }
            depth = depth.saturating_sub(1);
        }
        i += 1;
    }
    None
}

fn split_args(args: &str) -> Vec<String> {
    let bytes = args.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth_paren = 0usize;
    let mut depth_brace = 0usize;
    let mut depth_bracket = 0usize;
    let mut in_string = false;
    let mut string_delim = b'"';
    let mut escape = false;
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == string_delim {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if b == b'"' || b == b'\'' {
            in_string = true;
            string_delim = b;
            i += 1;
            continue;
        }

        match b {
            b'(' => depth_paren += 1,
            b')' => depth_paren = depth_paren.saturating_sub(1),
            b'{' => depth_brace += 1,
            b'}' => depth_brace = depth_brace.saturating_sub(1),
            b'[' => depth_bracket += 1,
            b']' => depth_bracket = depth_bracket.saturating_sub(1),
            b',' if depth_paren == 0 && depth_brace == 0 && depth_bracket == 0 => {
                let part = args[start..i].trim();
                if !part.is_empty() {
                    parts.push(part.to_string());
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }

    let tail = args[start..].trim();
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }
    parts
}

fn convert_shell_constructor(name: &str, args: &[String]) -> Option<String> {
    match name {
        "ObjectId" | "ObjectID" => {
            let value = arg_as_string(args.first()?);
            Some(format!("{{\"$oid\":{}}}", serde_json::to_string(&value).ok()?))
        }
        "ISODate" | "Date" => {
            let value = arg_as_string(args.first()?);
            Some(format!("{{\"$date\":{}}}", serde_json::to_string(&value).ok()?))
        }
        "NumberLong" => {
            let value = arg_as_number_string(args.first()?);
            Some(format!("{{\"$numberLong\":{}}}", serde_json::to_string(&value).ok()?))
        }
        "NumberInt" => {
            let value = arg_as_number_string(args.first()?);
            Some(format!("{{\"$numberInt\":{}}}", serde_json::to_string(&value).ok()?))
        }
        "NumberDecimal" => {
            let value = arg_as_number_string(args.first()?);
            Some(format!("{{\"$numberDecimal\":{}}}", serde_json::to_string(&value).ok()?))
        }
        "NumberDouble" => {
            let value = arg_as_number_string(args.first()?);
            Some(format!("{{\"$numberDouble\":{}}}", serde_json::to_string(&value).ok()?))
        }
        "UUID" => {
            let value = arg_as_string(args.first()?);
            Some(format!("{{\"$uuid\":{}}}", serde_json::to_string(&value).ok()?))
        }
        "Timestamp" => {
            if args.len() < 2 {
                return None;
            }
            let t = arg_as_i64(&args[0])?;
            let i = arg_as_i64(&args[1])?;
            Some(format!("{{\"$timestamp\":{{\"t\":{t},\"i\":{i}}}}}"))
        }
        _ => None,
    }
}

fn arg_as_string(arg: &str) -> String {
    if let Ok(Value::String(text)) = serde_json::from_str::<Value>(arg) {
        return text;
    }
    if let Ok(Value::String(text)) = json5::from_str::<Value>(arg) {
        return text;
    }
    arg.trim().trim_matches(['"', '\'']).to_string()
}

fn arg_as_number_string(arg: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(arg) {
        match value {
            Value::Number(num) => return num.to_string(),
            Value::String(text) => return text,
            _ => {}
        }
    }
    if let Ok(value) = json5::from_str::<Value>(arg) {
        match value {
            Value::Number(num) => return num.to_string(),
            Value::String(text) => return text,
            _ => {}
        }
    }
    arg.trim().trim_matches(['"', '\'']).to_string()
}

fn arg_as_i64(arg: &str) -> Option<i64> {
    if let Ok(Value::Number(num)) = serde_json::from_str::<Value>(arg) {
        return num.as_i64();
    }
    if let Ok(Value::Number(num)) = json5::from_str::<Value>(arg) {
        return num.as_i64();
    }
    arg.trim().trim_matches(['"', '\'']).parse::<i64>().ok()
}

/// Format a JSON value using relaxed MongoDB-style keys (no quotes for simple identifiers).
pub fn format_relaxed_json_value(value: &Value) -> String {
    format_relaxed_value(value, 0)
}

fn format_relaxed_value(value: &Value, indent: usize) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(val) => val.to_string(),
        Value::Number(num) => num.to_string(),
        Value::String(text) => serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(items) => format_relaxed_array(items, indent),
        Value::Object(map) => format_relaxed_object(map, indent),
    }
}

fn format_relaxed_array(items: &[Value], indent: usize) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }

    let next_indent = indent + 2;
    let mut out = String::new();
    out.push('[');
    out.push('\n');
    for (idx, item) in items.iter().enumerate() {
        out.push_str(&" ".repeat(next_indent));
        out.push_str(&format_relaxed_value(item, next_indent));
        if idx + 1 < items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str(&" ".repeat(indent));
    out.push(']');
    out
}

fn try_format_shell_constructor(map: &serde_json::Map<String, Value>) -> Option<String> {
    if map.len() == 1 {
        if let Some(Value::String(v)) = map.get("$oid") {
            return Some(format!("ObjectId(\"{}\")", v));
        }
        if let Some(Value::String(v)) = map.get("$date") {
            return Some(format!("ISODate(\"{}\")", v));
        }
        if let Some(Value::String(v)) = map.get("$numberLong") {
            return Some(format!("NumberLong(\"{}\")", v));
        }
        if let Some(Value::String(v)) = map.get("$numberInt") {
            return Some(format!("NumberInt({})", v));
        }
        if let Some(Value::String(v)) = map.get("$numberDecimal") {
            return Some(format!("NumberDecimal(\"{}\")", v));
        }
        if let Some(Value::String(v)) = map.get("$numberDouble") {
            return Some(format!("NumberDouble(\"{}\")", v));
        }
        if let Some(Value::String(v)) = map.get("$uuid") {
            return Some(format!("UUID(\"{}\")", v));
        }
    }
    if map.len() == 2 {
        if let Some(Value::Object(ts)) = map.get("$timestamp")
            && let (Some(Value::Number(t)), Some(Value::Number(i))) = (ts.get("t"), ts.get("i"))
        {
            return Some(format!("Timestamp({}, {})", t, i));
        }
        if let Some(Value::Object(re)) = map.get("$regularExpression")
            && let (Some(Value::String(pattern)), Some(Value::String(options))) =
                (re.get("pattern"), re.get("options"))
        {
            return Some(format!("/{}/{}", pattern, options));
        }
    }
    None
}

fn format_relaxed_object(map: &serde_json::Map<String, Value>, indent: usize) -> String {
    if map.is_empty() {
        return "{}".to_string();
    }
    if let Some(shell) = try_format_shell_constructor(map) {
        return shell;
    }

    let next_indent = indent + 2;
    let mut out = String::new();
    out.push('{');
    out.push('\n');
    let len = map.len();
    for (idx, (key, value)) in map.iter().enumerate() {
        out.push_str(&" ".repeat(next_indent));
        if is_relaxed_key(key) {
            out.push_str(key);
        } else {
            out.push_str(&serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()));
        }
        out.push_str(": ");
        out.push_str(&format_relaxed_value(value, next_indent));
        if idx + 1 < len {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str(&" ".repeat(indent));
    out.push('}');
    out
}

fn is_relaxed_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first == '$' || first.is_ascii_alphabetic()) {
        return false;
    }
    for ch in chars {
        if !(ch == '_' || ch == '$' || ch.is_ascii_alphanumeric()) {
            return false;
        }
    }
    true
}

/// Format a JSON value using relaxed MongoDB-style keys, compact single-line format.
pub fn format_relaxed_json_compact(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(val) => val.to_string(),
        Value::Number(num) => num.to_string(),
        Value::String(text) => serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(items) => format_relaxed_array_compact(items),
        Value::Object(map) => format_relaxed_object_compact(map),
    }
}

fn format_relaxed_array_compact(items: &[Value]) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }
    let mut out = String::new();
    out.push('[');
    for (idx, item) in items.iter().enumerate() {
        out.push_str(&format_relaxed_json_compact(item));
        if idx + 1 < items.len() {
            out.push_str(", ");
        }
    }
    out.push(']');
    out
}

fn format_relaxed_object_compact(map: &serde_json::Map<String, Value>) -> String {
    if map.is_empty() {
        return "{}".to_string();
    }
    if let Some(shell) = try_format_shell_constructor(map) {
        return shell;
    }
    let mut out = String::new();
    out.push('{');
    let len = map.len();
    for (idx, (key, value)) in map.iter().enumerate() {
        if is_relaxed_key(key) {
            out.push_str(key);
        } else {
            out.push_str(&serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()));
        }
        out.push_str(": ");
        out.push_str(&format_relaxed_json_compact(value));
        if idx + 1 < len {
            out.push_str(", ");
        }
    }
    out.push('}');
    out
}

/// Parse an edited string value back into BSON, matching the original type.
pub fn parse_edited_value(original: &Bson, input: &str) -> Result<Bson, String> {
    let trimmed = input.trim();
    match original {
        Bson::String(_) => Ok(Bson::String(trimmed.to_string())),
        Bson::Int32(_) => {
            trimmed.parse::<i32>().map(Bson::Int32).map_err(|_| "Expected int32".to_string())
        }
        Bson::Int64(_) => {
            trimmed.parse::<i64>().map(Bson::Int64).map_err(|_| "Expected int64".to_string())
        }
        Bson::Double(_) => {
            trimmed.parse::<f64>().map(Bson::Double).map_err(|_| "Expected number".to_string())
        }
        Bson::Boolean(_) => match trimmed.to_ascii_lowercase().as_str() {
            "true" => Ok(Bson::Boolean(true)),
            "false" => Ok(Bson::Boolean(false)),
            _ => Err("Expected true/false".to_string()),
        },
        Bson::Null => {
            if trimmed.eq_ignore_ascii_case("null") {
                Ok(Bson::Null)
            } else {
                Err("Expected null".to_string())
            }
        }
        Bson::ObjectId(_) => ObjectId::parse_str(trimmed)
            .map(Bson::ObjectId)
            .map_err(|_| "Expected ObjectId hex".to_string()),
        Bson::DateTime(_) => DateTime::parse_rfc3339_str(trimmed)
            .map(Bson::DateTime)
            .map_err(|_| "Expected RFC3339 date".to_string()),
        _ => Err("Unsupported type".to_string()),
    }
}

/// Convert a BSON document to a pretty-printed MongoDB shell-style string.
///
/// Uses shell constructors like `ObjectId("...")`, `ISODate("...")` instead of
/// Extended JSON wrappers like `{"$oid": "..."}`.
pub fn document_to_shell_string(doc: &Document) -> String {
    let value = bson::Bson::Document(doc.clone()).into_relaxed_extjson();
    format_relaxed_json_value(&value)
}

/// Parse a JSON string into a BSON document.
pub fn parse_document_from_json(input: &str) -> Result<Document, String> {
    let value: Value = parse_value_from_relaxed_json(input)?;
    let bson = bson::Bson::try_from(value).map_err(|e| e.to_string())?;
    match bson {
        bson::Bson::Document(doc) => Ok(doc),
        _ => Err("Root JSON must be a document".to_string()),
    }
}

/// Parse JSON as either a single document or array of documents.
pub fn parse_documents_from_json(input: &str) -> Result<Vec<Document>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Clipboard is empty".to_string());
    }

    match parse_value_from_relaxed_json(trimmed) {
        Ok(value) => match value {
            Value::Array(arr) => {
                let mut docs = Vec::with_capacity(arr.len());
                for (i, item) in arr.into_iter().enumerate() {
                    let bson = bson::Bson::try_from(item).map_err(|e| e.to_string())?;
                    match bson {
                        bson::Bson::Document(doc) => docs.push(doc),
                        _ => return Err(format!("Array item {} is not a document", i)),
                    }
                }
                Ok(docs)
            }
            Value::Object(_) => {
                let bson = bson::Bson::try_from(value).map_err(|e| e.to_string())?;
                match bson {
                    bson::Bson::Document(doc) => Ok(vec![doc]),
                    _ => Err("Root JSON must be a document or array".to_string()),
                }
            }
            _ => Err("Root JSON must be a document or array of documents".to_string()),
        },
        Err(_) => {
            let mut docs = Vec::new();
            for (line_no, line) in trimmed.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let value: Value = parse_value_from_relaxed_json(line)
                    .map_err(|e| format!("Line {}: {}", line_no + 1, e))?;
                let bson = bson::Bson::try_from(value).map_err(|e| e.to_string())?;
                match bson {
                    bson::Bson::Document(doc) => docs.push(doc),
                    _ => return Err(format!("Line {} is not a document", line_no + 1)),
                }
            }

            if docs.is_empty() { Err("No documents found".to_string()) } else { Ok(docs) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::oid::ObjectId;
    use mongodb::bson::spec::BinarySubtype;
    use mongodb::bson::{Bson, DateTime};

    #[test]
    fn parses_object_id_shell_syntax() {
        let doc = parse_document_from_json("{ _id: ObjectId(\"6283a37e34d71078c4996c72\") }")
            .expect("parse");
        let oid = doc.get_object_id("_id").expect("oid");
        let expected = ObjectId::parse_str("6283a37e34d71078c4996c72").expect("oid");
        assert_eq!(oid, expected);
    }

    #[test]
    fn parses_iso_date_shell_syntax() {
        let doc = parse_document_from_json("{ createdAt: ISODate(\"2020-01-01T00:00:00Z\") }")
            .expect("parse");
        let dt = *doc.get_datetime("createdAt").expect("datetime");
        let expected = DateTime::parse_rfc3339_str("2020-01-01T00:00:00Z").expect("dt");
        assert_eq!(dt, expected);
    }

    #[test]
    fn parses_numeric_shell_syntax() {
        let doc = parse_document_from_json(
            "{ long: NumberLong(\"42\"), int: NumberInt(7), dbl: NumberDouble(3.5) }",
        )
        .expect("parse");
        assert!(matches!(doc.get("long"), Some(Bson::Int64(42))));
        assert!(matches!(doc.get("int"), Some(Bson::Int32(7))));
        assert!(matches!(doc.get("dbl"), Some(Bson::Double(v)) if (*v - 3.5).abs() < 1e-9));
    }

    #[test]
    fn parses_decimal_and_uuid_shell_syntax() {
        let doc = parse_document_from_json(
            "{ dec: NumberDecimal(\"1.25\"), id: UUID(\"00112233-4455-6677-8899-aabbccddeeff\") }",
        )
        .expect("parse");
        assert!(matches!(doc.get("dec"), Some(Bson::Decimal128(_))));
        if let Some(Bson::Binary(bin)) = doc.get("id") {
            assert_eq!(bin.subtype, BinarySubtype::Uuid);
            assert_eq!(bin.bytes.len(), 16);
        } else {
            panic!("expected uuid binary");
        }
    }

    #[test]
    fn parses_timestamp_shell_syntax() {
        let doc = parse_document_from_json("{ ts: Timestamp(5, 7) }").expect("parse");
        if let Some(Bson::Timestamp(ts)) = doc.get("ts") {
            assert_eq!(ts.time, 5);
            assert_eq!(ts.increment, 7);
        } else {
            panic!("expected timestamp");
        }
    }

    #[test]
    fn does_not_replace_inside_strings() {
        let doc = parse_document_from_json("{ note: \"ObjectId(\\\"abc\\\")\" }").expect("parse");
        let note = doc.get_str("note").expect("note");
        assert_eq!(note, "ObjectId(\"abc\")");
    }

    #[test]
    fn preserves_non_ascii_text() {
        let input = r#"{ "name": "ატარებს" }"#;
        let doc = parse_document_from_json(input).expect("parse");
        assert_eq!(doc.get_str("name").expect("name"), "ატარებს");
    }

    #[test]
    fn format_roundtrips_non_ascii_text() {
        let input = r#"{ "name": "ატარებს", "city": "東京" }"#;
        let value = parse_value_from_relaxed_json(input).expect("parse");
        let formatted = format_relaxed_json_value(&value);
        let reparsed = parse_value_from_relaxed_json(&formatted).expect("reparse");
        assert_eq!(value, reparsed);
    }
}
