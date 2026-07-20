use std::fmt;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Utc};
use mongodb::bson::{Bson, DateTime as BsonDateTime, Document, oid::ObjectId};

use crate::bson::{
    format_relaxed_json_compact, parse_bson_from_relaxed_json, parse_document_from_json,
};

#[derive(Clone, Debug)]
pub(crate) struct CompiledFilter {
    pub raw_store: String,
    pub document: Option<Document>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FastFilterErrorKind {
    Incomplete,
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FastFilterError {
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

    pub(crate) fn is_incomplete(&self) -> bool {
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

#[derive(Clone, Copy, Debug)]
enum ParsedDateValue {
    Instant(BsonDateTime),
    Range { start: Option<BsonDateTime>, end: Option<BsonDateTime> },
}

struct ParsedInlineCondition<'a> {
    field: &'a str,
    op: FastOperator,
    value: &'a str,
    consumed: usize,
}

pub(crate) fn compile_filter_input(raw: &str) -> Result<CompiledFilter, FastFilterError> {
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

pub(crate) fn format_compiled_filter(compiled: &CompiledFilter) -> String {
    if compiled.document.is_none() { "{}".to_string() } else { compiled.raw_store.clone() }
}

pub(crate) fn filter_chips_for_input(raw: &str) -> Vec<String> {
    let Ok(compiled) = compile_filter_input(raw) else {
        return Vec::new();
    };
    let Some(doc) = compiled.document else {
        return Vec::new();
    };
    let mut chips = Vec::new();
    collect_document_chips(&doc, &mut chips);
    chips
}

fn compiled_from_document(doc: Document) -> CompiledFilter {
    if doc.is_empty() {
        return CompiledFilter { raw_store: String::new(), document: None };
    }

    let value = Bson::Document(doc.clone()).into_relaxed_extjson();
    CompiledFilter { raw_store: format_relaxed_json_compact(&value), document: Some(doc) }
}

fn collect_document_chips(doc: &Document, out: &mut Vec<String>) {
    if let Some(Bson::Array(items)) = doc.get("$and") {
        for item in items {
            if let Bson::Document(child) = item {
                collect_document_chips(child, out);
            }
        }
    }
    if let Some(Bson::Array(items)) = doc.get("$or") {
        let mut clauses = Vec::new();
        for item in items {
            if let Bson::Document(child) = item {
                let mut child_chips = Vec::new();
                collect_document_chips(child, &mut child_chips);
                if !child_chips.is_empty() {
                    clauses.push(child_chips.join(" AND "));
                }
            }
        }
        if !clauses.is_empty() {
            out.push(format!("({})", clauses.join(" OR ")));
        }
    }

    for (field, value) in doc {
        if field == "$and" || field == "$or" {
            continue;
        }
        collect_field_chips(field, value, out);
    }
}

fn collect_field_chips(field: &str, value: &Bson, out: &mut Vec<String>) {
    let Bson::Document(operators) = value else {
        out.push(format!("{field} = {}", chip_value(value)));
        return;
    };

    if !operators.keys().all(|key| key.starts_with('$')) {
        out.push(format!("{field} = {}", chip_value(value)));
        return;
    }

    if let Some(value) = operators.get("$exists") {
        match value {
            Bson::Boolean(true) => out.push(format!("{field} exists")),
            Bson::Boolean(false) => out.push(format!("{field} missing")),
            _ => out.push(format!("{field} exists {}", chip_value(value))),
        }
    }
    if let Some(value) = operators.get("$eq") {
        out.push(format!("{field} = {}", chip_value(value)));
    }
    if let Some(value) = operators.get("$ne") {
        out.push(format!("{field} != {}", chip_value(value)));
    }
    for (operator, label) in
        [("$gt", ">"), ("$gte", ">="), ("$lt", "<"), ("$lte", "<="), ("$type", "type")]
    {
        if let Some(value) = operators.get(operator) {
            out.push(format!("{field} {label} {}", chip_value(value)));
        }
    }
    if let Some(value) = operators.get("$in") {
        out.push(format!("{field} in {}", chip_value(value)));
    }
    if let Some(value) = operators.get("$nin") {
        out.push(format!("{field} not in {}", chip_value(value)));
    }
    if let Some(value) = operators.get("$regex") {
        out.push(format!("{field} ~ {}", chip_value(value)));
    }
}

fn chip_value(value: &Bson) -> String {
    let label = match value {
        Bson::String(value) => value.clone(),
        Bson::Array(values) => values.iter().map(chip_value).collect::<Vec<_>>().join(", "),
        _ => {
            let value = value.clone().into_relaxed_extjson();
            format_relaxed_json_compact(&value)
        }
    };
    truncate_chip_label(&label, 54)
}

fn truncate_chip_label(label: &str, max_chars: usize) -> String {
    if label.chars().count() <= max_chars {
        return label.to_string();
    }
    let mut out = label.chars().take(max_chars.saturating_sub(3)).collect::<String>();
    out.push_str("...");
    out
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
            build_condition_value(parsed.field, parsed.op, parsed.value)?
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

fn build_condition_value(
    field: &str,
    op: FastOperator,
    raw: &str,
) -> Result<Bson, FastFilterError> {
    let mut value_raw = raw.trim();
    if op == FastOperator::Ne
        && let Some(stripped) = value_raw.strip_prefix('!')
    {
        value_raw = stripped.trim();
    }

    if let Some(date_value) = parse_date_filter_value(field, value_raw)? {
        return Ok(date_condition_value(op, date_value));
    }

    let parsed_value = parse_filter_value(field, value_raw)?;
    Ok(match op {
        FastOperator::Eq => parsed_value,
        FastOperator::Ne => operator_doc("$ne", parsed_value),
        FastOperator::Gt => operator_doc("$gt", parsed_value),
        FastOperator::Gte => operator_doc("$gte", parsed_value),
        FastOperator::Lt => operator_doc("$lt", parsed_value),
        FastOperator::Lte => operator_doc("$lte", parsed_value),
        FastOperator::Regex => unreachable!(),
    })
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

fn parse_date_filter_value(
    field: &str,
    raw: &str,
) -> Result<Option<ParsedDateValue>, FastFilterError> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || is_quoted(trimmed)
        || trimmed.starts_with("ISODate(")
        || trimmed.starts_with("Date(")
    {
        return Ok(None);
    }

    if !is_date_field(field) || !looks_like_date_shortcut(trimmed) {
        return Ok(None);
    }

    parse_date_shortcut(trimmed)
        .map(Some)
        .ok_or_else(|| FastFilterError::invalid(format!("Invalid date shortcut `{trimmed}`")))
}

fn parse_date_shortcut(raw: &str) -> Option<ParsedDateValue> {
    let trimmed = raw.trim();
    if let Some((start_raw, end_raw)) = trimmed.split_once("..") {
        let start = if start_raw.trim().is_empty() {
            None
        } else {
            Some(date_range_start(start_raw.trim())?)
        };
        let end =
            if end_raw.trim().is_empty() { None } else { Some(date_range_end(end_raw.trim())?) };
        if start.is_none() && end.is_none() {
            return None;
        }
        return Some(ParsedDateValue::Range { start, end });
    }

    if let Some(instant) = parse_datetime_literal(trimmed) {
        return Some(ParsedDateValue::Instant(instant));
    }

    if let Some(period) = parse_relative_period(trimmed) {
        return Some(period);
    }

    if let Some(period) = parse_named_period(trimmed) {
        return Some(period);
    }

    parse_calendar_period(trimmed)
}

fn parse_datetime_literal(raw: &str) -> Option<BsonDateTime> {
    if !(raw.contains('T') || raw.contains(':')) {
        return None;
    }

    let candidates = if raw.ends_with('Z') || raw.contains('+') {
        vec![raw.to_string()]
    } else {
        vec![raw.to_string(), format!("{raw}Z")]
    };

    candidates.into_iter().find_map(|candidate| BsonDateTime::parse_rfc3339_str(candidate).ok())
}

fn parse_relative_period(raw: &str) -> Option<ParsedDateValue> {
    let normalized = normalize_date_shortcut(raw);
    let now = Utc::now();

    let (direction, rest) = if let Some(rest) = normalized.strip_prefix("last") {
        (-1, rest)
    } else if let Some(rest) = normalized.strip_prefix("past") {
        (-1, rest)
    } else {
        let rest = normalized.strip_prefix("next")?;
        (1, rest)
    };

    let digit_count = rest.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_count == 0 {
        return None;
    }
    let amount = rest[..digit_count].parse::<i64>().ok()?;
    let unit = &rest[digit_count..];
    let duration = match unit {
        "m" | "min" | "mins" | "minute" | "minutes" => Duration::minutes(amount),
        "h" | "hr" | "hrs" | "hour" | "hours" => Duration::hours(amount),
        "d" | "day" | "days" => Duration::days(amount),
        "w" | "week" | "weeks" => Duration::weeks(amount),
        _ => return None,
    };

    let shifted_dt = now + duration * direction;
    let now = bson_datetime_from_millis(now.timestamp_millis());
    let shifted = bson_datetime_from_millis(shifted_dt.timestamp_millis());
    if direction < 0 {
        Some(ParsedDateValue::Range { start: Some(shifted), end: Some(now) })
    } else {
        Some(ParsedDateValue::Range { start: Some(now), end: Some(shifted) })
    }
}

fn parse_named_period(raw: &str) -> Option<ParsedDateValue> {
    let normalized = normalize_date_shortcut(raw);
    let today = Utc::now().date_naive();
    let now = bson_datetime_from_millis(Utc::now().timestamp_millis());

    match normalized.as_str() {
        "now" => Some(ParsedDateValue::Instant(now)),
        "today" => Some(day_period(today)),
        "yesterday" => Some(day_period(today - Duration::days(1))),
        "tomorrow" => Some(day_period(today + Duration::days(1))),
        "thisweek" | "week" => Some(week_period(today)),
        "lastweek" | "previousweek" | "prevweek" => Some(week_period(today - Duration::weeks(1))),
        "nextweek" => Some(week_period(today + Duration::weeks(1))),
        "thismonth" | "month" => Some(month_period(today.year(), today.month())),
        "lastmonth" | "previousmonth" | "prevmonth" => {
            let (year, month) = add_months(today.year(), today.month(), -1);
            Some(month_period(year, month))
        }
        "nextmonth" => {
            let (year, month) = add_months(today.year(), today.month(), 1);
            Some(month_period(year, month))
        }
        "thisyear" | "year" => Some(year_period(today.year())),
        "lastyear" | "previousyear" | "prevyear" => Some(year_period(today.year() - 1)),
        "nextyear" => Some(year_period(today.year() + 1)),
        "wtd" => {
            let start = start_of_week(today);
            Some(ParsedDateValue::Range {
                start: Some(bson_datetime_from_date(start)),
                end: Some(now),
            })
        }
        "mtd" => Some(ParsedDateValue::Range {
            start: Some(bson_datetime_from_date(ymd(today.year(), today.month(), 1)?)),
            end: Some(now),
        }),
        "ytd" => Some(ParsedDateValue::Range {
            start: Some(bson_datetime_from_date(ymd(today.year(), 1, 1)?)),
            end: Some(now),
        }),
        _ => parse_month_name_period(&normalized, today)
            .or_else(|| parse_named_quarter_period(&normalized, today)),
    }
}

fn parse_calendar_period(raw: &str) -> Option<ParsedDateValue> {
    let normalized = raw.trim();

    if let Some((year, quarter)) = parse_year_quarter(normalized) {
        return Some(quarter_period(year, quarter));
    }

    if let Ok(date) = NaiveDate::parse_from_str(normalized, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(normalized, "%Y/%m/%d"))
    {
        return Some(day_period(date));
    }

    if let Some((year, month)) = parse_year_month(normalized) {
        return Some(month_period(year, month));
    }

    if normalized.len() == 4
        && normalized.chars().all(|ch| ch.is_ascii_digit())
        && let Ok(year) = normalized.parse::<i32>()
    {
        return Some(year_period(year));
    }

    None
}

fn parse_year_month(raw: &str) -> Option<(i32, u32)> {
    let separator = if raw.contains('-') { '-' } else { '/' };
    let (year_raw, month_raw) = raw.split_once(separator)?;
    if year_raw.len() != 4 || month_raw.len() != 2 {
        return None;
    }
    let year = year_raw.parse::<i32>().ok()?;
    let month = month_raw.parse::<u32>().ok()?;
    (1..=12).contains(&month).then_some((year, month))
}

fn parse_year_quarter(raw: &str) -> Option<(i32, u32)> {
    let lower = raw.to_ascii_lowercase().replace('-', "");
    if let Some((year_raw, quarter_raw)) = lower.split_once('q')
        && year_raw.len() == 4
    {
        let year = year_raw.parse::<i32>().ok()?;
        let quarter = quarter_raw.parse::<u32>().ok()?;
        return (1..=4).contains(&quarter).then_some((year, quarter));
    }
    if let Some(quarter_raw) = lower.strip_prefix('q') {
        let quarter = quarter_raw.parse::<u32>().ok()?;
        let today = Utc::now().date_naive();
        let mut year = today.year();
        let current_quarter = ((today.month() - 1) / 3) + 1;
        if quarter > current_quarter {
            year -= 1;
        }
        return (1..=4).contains(&quarter).then_some((year, quarter));
    }
    None
}

fn parse_named_quarter_period(normalized: &str, today: NaiveDate) -> Option<ParsedDateValue> {
    let quarter = normalized.strip_prefix('q')?.parse::<u32>().ok()?;
    if !(1..=4).contains(&quarter) {
        return None;
    }
    let current_quarter = ((today.month() - 1) / 3) + 1;
    let mut year = today.year();
    if quarter > current_quarter {
        year -= 1;
    }
    Some(quarter_period(year, quarter))
}

fn parse_month_name_period(normalized: &str, today: NaiveDate) -> Option<ParsedDateValue> {
    let (month, suffix) = month_name_prefix(normalized)?;
    let year = if suffix.is_empty() {
        let mut year = today.year();
        if month > today.month() {
            year -= 1;
        }
        year
    } else {
        suffix.parse::<i32>().ok()?
    };
    Some(month_period(year, month))
}

fn month_name_prefix(value: &str) -> Option<(u32, &str)> {
    for (index, aliases) in MONTH_ALIASES.iter().enumerate() {
        for alias in *aliases {
            if let Some(suffix) = value.strip_prefix(alias)
                && (suffix.is_empty()
                    || (suffix.len() == 4 && suffix.chars().all(|ch| ch.is_ascii_digit())))
            {
                return Some((index as u32 + 1, suffix));
            }
        }
    }
    None
}

fn date_condition_value(op: FastOperator, value: ParsedDateValue) -> Bson {
    match value {
        ParsedDateValue::Instant(date) => match op {
            FastOperator::Eq => Bson::DateTime(date),
            FastOperator::Ne => operator_doc("$ne", Bson::DateTime(date)),
            FastOperator::Gt => operator_doc("$gt", Bson::DateTime(date)),
            FastOperator::Gte => operator_doc("$gte", Bson::DateTime(date)),
            FastOperator::Lt => operator_doc("$lt", Bson::DateTime(date)),
            FastOperator::Lte => operator_doc("$lte", Bson::DateTime(date)),
            FastOperator::Regex => unreachable!(),
        },
        ParsedDateValue::Range { start, end } => match op {
            FastOperator::Eq => date_range_doc(start, end),
            FastOperator::Ne => operator_doc("$not", date_range_doc(start, end)),
            FastOperator::Gt => {
                let bound = end.or(start).expect("date range should have at least one bound");
                operator_doc("$gte", Bson::DateTime(bound))
            }
            FastOperator::Gte => {
                let bound = start.or(end).expect("date range should have at least one bound");
                operator_doc("$gte", Bson::DateTime(bound))
            }
            FastOperator::Lt => {
                let bound = start.or(end).expect("date range should have at least one bound");
                operator_doc("$lt", Bson::DateTime(bound))
            }
            FastOperator::Lte => {
                let bound = end.or(start).expect("date range should have at least one bound");
                operator_doc("$lt", Bson::DateTime(bound))
            }
            FastOperator::Regex => unreachable!(),
        },
    }
}

fn date_range_doc(start: Option<BsonDateTime>, end: Option<BsonDateTime>) -> Bson {
    let mut doc = Document::new();
    if let Some(start) = start {
        doc.insert("$gte", Bson::DateTime(start));
    }
    if let Some(end) = end {
        doc.insert("$lt", Bson::DateTime(end));
    }
    Bson::Document(doc)
}

fn date_range_start(raw: &str) -> Option<BsonDateTime> {
    match parse_date_shortcut(raw)? {
        ParsedDateValue::Instant(date) => Some(date),
        ParsedDateValue::Range { start, end } => start.or(end),
    }
}

fn date_range_end(raw: &str) -> Option<BsonDateTime> {
    match parse_date_shortcut(raw)? {
        ParsedDateValue::Instant(date) => Some(date),
        ParsedDateValue::Range { start, end } => end.or(start),
    }
}

fn day_period(date: NaiveDate) -> ParsedDateValue {
    ParsedDateValue::Range {
        start: Some(bson_datetime_from_date(date)),
        end: Some(bson_datetime_from_date(date + Duration::days(1))),
    }
}

fn week_period(date: NaiveDate) -> ParsedDateValue {
    let start = start_of_week(date);
    ParsedDateValue::Range {
        start: Some(bson_datetime_from_date(start)),
        end: Some(bson_datetime_from_date(start + Duration::weeks(1))),
    }
}

fn month_period(year: i32, month: u32) -> ParsedDateValue {
    let start = ymd(year, month, 1).expect("valid month period");
    let (next_year, next_month) = add_months(year, month, 1);
    ParsedDateValue::Range {
        start: Some(bson_datetime_from_date(start)),
        end: Some(bson_datetime_from_date(
            ymd(next_year, next_month, 1).expect("valid next month"),
        )),
    }
}

fn year_period(year: i32) -> ParsedDateValue {
    ParsedDateValue::Range {
        start: Some(bson_datetime_from_date(ymd(year, 1, 1).expect("valid year"))),
        end: Some(bson_datetime_from_date(ymd(year + 1, 1, 1).expect("valid next year"))),
    }
}

fn quarter_period(year: i32, quarter: u32) -> ParsedDateValue {
    let start_month = ((quarter - 1) * 3) + 1;
    let (end_year, end_month) = add_months(year, start_month, 3);
    ParsedDateValue::Range {
        start: Some(bson_datetime_from_date(ymd(year, start_month, 1).expect("valid quarter"))),
        end: Some(bson_datetime_from_date(ymd(end_year, end_month, 1).expect("valid quarter end"))),
    }
}

fn start_of_week(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

fn add_months(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero_based = year * 12 + month as i32 - 1 + delta;
    (zero_based.div_euclid(12), zero_based.rem_euclid(12) as u32 + 1)
}

fn ymd(year: i32, month: u32, day: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, day)
}

fn bson_datetime_from_date(date: NaiveDate) -> BsonDateTime {
    bson_datetime_from_naive(date.and_hms_opt(0, 0, 0).expect("valid midnight"))
}

fn bson_datetime_from_naive(date: NaiveDateTime) -> BsonDateTime {
    bson_datetime_from_millis(date.and_utc().timestamp_millis())
}

fn bson_datetime_from_millis(ms: i64) -> BsonDateTime {
    BsonDateTime::from_millis(ms)
}

fn looks_like_date_shortcut(raw: &str) -> bool {
    let normalized = normalize_date_shortcut(raw);
    raw.contains("..")
        || raw.contains('T')
        || raw.contains('/')
        || raw.contains('-')
        || raw.contains(':')
        || normalized.chars().all(|ch| ch.is_ascii_digit()) && normalized.len() == 4
        || matches!(
            normalized.as_str(),
            "now"
                | "today"
                | "yesterday"
                | "tomorrow"
                | "thisweek"
                | "week"
                | "lastweek"
                | "previousweek"
                | "prevweek"
                | "nextweek"
                | "thismonth"
                | "month"
                | "lastmonth"
                | "previousmonth"
                | "prevmonth"
                | "nextmonth"
                | "thisyear"
                | "year"
                | "lastyear"
                | "previousyear"
                | "prevyear"
                | "nextyear"
                | "wtd"
                | "mtd"
                | "ytd"
        )
        || ["last", "past", "next"].iter().any(|prefix| normalized.starts_with(prefix))
        || parse_year_quarter(&normalized).is_some()
        || month_name_prefix(&normalized).is_some()
}

pub(crate) fn is_date_field(field: &str) -> bool {
    let leaf = field.rsplit('.').next().unwrap_or(field);
    let normalized = leaf.to_ascii_lowercase();
    normalized == "date"
        || normalized == "time"
        || normalized == "timestamp"
        || normalized == "ts"
        || normalized.ends_with("date")
        || normalized.ends_with("time")
        || normalized.ends_with("timestamp")
        || normalized.ends_with("at")
        || normalized.contains("created")
        || normalized.contains("updated")
        || normalized.contains("deleted")
        || normalized.contains("expired")
        || normalized.contains("expires")
        || normalized.contains("expiry")
        || normalized.contains("published")
        || normalized.contains("scheduled")
        || normalized.contains("started")
        || normalized.contains("finished")
        || normalized.contains("completed")
        || normalized == "due"
        || normalized == "since"
        || normalized == "until"
}

fn normalize_date_shortcut(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().chars().filter(|ch| !matches!(ch, '_' | '-' | ' ')).collect()
}

const MONTH_ALIASES: &[&[&str]] = &[
    &["january", "jan"],
    &["february", "feb"],
    &["march", "mar"],
    &["april", "apr"],
    &["may"],
    &["june", "jun"],
    &["july", "jul"],
    &["august", "aug"],
    &["september", "sept", "sep"],
    &["october", "oct"],
    &["november", "nov"],
    &["december", "dec"],
];

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
    use super::{compile_filter_input, filter_chips_for_input, format_compiled_filter};

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
    fn smart_converts_date_day_shorthand_for_date_fields() {
        assert_eq!(
            formatted("createdAt:2026-05-23"),
            "{createdAt: {$gte: ISODate(\"2026-05-23T00:00:00Z\"), $lt: ISODate(\"2026-05-24T00:00:00Z\")}}"
        );
        assert_eq!(
            formatted("createdAt<=2026-05-23"),
            "{createdAt: {$lt: ISODate(\"2026-05-24T00:00:00Z\")}}"
        );
    }

    #[test]
    fn smart_converts_month_year_quarter_and_explicit_ranges() {
        assert_eq!(
            formatted("createdAt:2026-05"),
            "{createdAt: {$gte: ISODate(\"2026-05-01T00:00:00Z\"), $lt: ISODate(\"2026-06-01T00:00:00Z\")}}"
        );
        assert_eq!(
            formatted("createdAt:2026"),
            "{createdAt: {$gte: ISODate(\"2026-01-01T00:00:00Z\"), $lt: ISODate(\"2027-01-01T00:00:00Z\")}}"
        );
        assert_eq!(
            formatted("createdAt:2026Q2"),
            "{createdAt: {$gte: ISODate(\"2026-04-01T00:00:00Z\"), $lt: ISODate(\"2026-07-01T00:00:00Z\")}}"
        );
        assert_eq!(
            formatted("createdAt:2026-05-01..2026-05-31"),
            "{createdAt: {$gte: ISODate(\"2026-05-01T00:00:00Z\"), $lt: ISODate(\"2026-06-01T00:00:00Z\")}}"
        );
        assert_eq!(
            formatted("createdAt:..2026-05-31"),
            "{createdAt: {$lt: ISODate(\"2026-06-01T00:00:00Z\")}}"
        );
    }

    #[test]
    fn date_conversion_is_field_aware() {
        assert_eq!(formatted("status:2026-05-23"), "{status: \"2026-05-23\"}");
    }

    #[test]
    fn relative_date_shortcuts_compile_to_date_ranges() {
        let compiled = compile_filter_input("updatedAt:last7d").expect("compile filter");
        let doc = compiled.document.expect("document");
        let updated_at = doc.get_document("updatedAt").expect("updatedAt doc");
        assert!(updated_at.get_datetime("$gte").is_ok());
        assert!(updated_at.get_datetime("$lt").is_ok());
    }

    #[test]
    fn builds_readable_filter_chips() {
        assert_eq!(
            filter_chips_for_input("status:active age>30 email~gmail !deleted"),
            vec!["status = active", "age > 30", "email ~ gmail", "deleted != true",]
        );
    }

    #[test]
    fn builds_readable_or_filter_chip() {
        assert_eq!(
            filter_chips_for_input(r#"{"$or":[{"status":"active"},{"age":{"$gt":30}}]}"#),
            vec!["(status = active OR age > 30)",]
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
