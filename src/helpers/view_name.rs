//! A name to offer for a new view, read off its pipeline.
//!
//! A view is named after what it shows of its source: `orders` filtered to open ones and
//! grouped by country is `orders_open_by_country`. The source comes first on purpose, so the
//! view sorts next to it in the sidebar. It is only an offer; the person can type over it.

use mongodb::bson::{Bson, Document};

const MAX_PART: usize = 24;

/// `source` plus what the pipeline filters on and what it shapes the result by, in the
/// source's own naming style, and not a name `taken` already holds.
pub fn suggest_view_name(source: &str, pipeline: &[Document], taken: &[String]) -> String {
    let mut parts: Vec<Vec<String>> =
        [filter_part(pipeline), shape_part(pipeline)].into_iter().flatten().collect();
    if parts.is_empty() {
        parts.push(vec!["view".to_string()]);
    }
    unique(join(source, &parts), taken)
}

/// `{view}_copy`, or the first numbered copy that is free.
pub fn suggest_copy_name(view: &str, taken: &[String]) -> String {
    unique(join(view, &[vec!["copy".to_string()]]), taken)
}

/// What the first `$match` keeps: `status: "open"` reads as `open`, `active: true` as
/// `active`, `deleted: false` as `not_deleted`. Ranges and operators say nothing short.
fn filter_part(pipeline: &[Document]) -> Option<Vec<String>> {
    let filter = pipeline.iter().find_map(|stage| stage.get_document("$match").ok())?;
    filter.iter().filter(|(field, _)| !field.starts_with('$')).find_map(
        |(field, value)| match value {
            Bson::String(text) => words(text),
            Bson::Boolean(true) => words(last_segment(field)),
            Bson::Boolean(false) => {
                words(last_segment(field)).map(|field| [vec!["not".to_string()], field].concat())
            }
            _ => None,
        },
    )
}

/// What the result is shaped by: the last `$group`'s key, or failing that a join, an unwound
/// array or a count.
fn shape_part(pipeline: &[Document]) -> Option<Vec<String>> {
    let by = |key: Vec<String>| Some([vec!["by".to_string()], key].concat());
    let stage = |operator: &str| pipeline.iter().rev().find_map(|stage| stage.get(operator));

    if let Some(group) = stage("$group").and_then(Bson::as_document) {
        return match group.get("_id") {
            // One group over everything is a totals row, not a breakdown "by" anything.
            Some(Bson::Null) => words("totals"),
            Some(key) => group_key(key).and_then(by).or_else(|| words("grouped")),
            None => words("grouped"),
        };
    }
    for operator in ["$sortByCount", "$bucket", "$bucketAuto"] {
        let key = stage(operator).and_then(|body| match body {
            Bson::Document(body) => body.get("groupBy").and_then(group_key),
            body => group_key(body),
        });
        if let Some(key) = key {
            return by(key);
        }
    }
    if let Some(from) =
        stage("$lookup").and_then(Bson::as_document).and_then(|l| l.get_str("from").ok())
    {
        return words(from).map(|from| [vec!["with".to_string()], from].concat());
    }
    if let Some(unwind) = stage("$unwind") {
        let path = match unwind {
            Bson::Document(body) => body.get_str("path").ok(),
            body => body.as_str(),
        };
        return path.and_then(|path| words(last_segment(path)));
    }
    stage("$count").and_then(|_| words("count"))
}

/// A group key as words: `"$customer.country"` is `country`, `{ year, month }` is
/// `year month`, and `{ $dateTrunc: { unit: "month" } }` is `month`.
fn group_key(key: &Bson) -> Option<Vec<String>> {
    match key {
        Bson::String(path) if path.starts_with('$') => words(last_segment(path)),
        Bson::Document(fields) => match fields.iter().next() {
            Some((operator, body)) if operator.starts_with('$') => body
                .as_document()
                .and_then(|body| body.get_str("unit").ok())
                .and_then(words)
                .or_else(|| words(operator)),
            _ => Some(fields.keys().take(2).filter_map(|key| words(key)).flatten().collect())
                .filter(|words: &Vec<String>| !words.is_empty()),
        },
        _ => None,
    }
}

fn last_segment(path: &str) -> &str {
    path.trim_start_matches('$').rsplit('.').next().unwrap_or(path)
}

/// Lowercase words of `text`, split at anything that isn't a letter or digit and at camelCase
/// humps. `None` when nothing is left or it is too long to be a name part.
fn words(text: &str) -> Option<Vec<String>> {
    let mut words: Vec<String> = Vec::new();
    let mut previous_lower = false;
    for ch in text.chars() {
        if !ch.is_alphanumeric() {
            previous_lower = false;
            words.push(String::new());
            continue;
        }
        if words.is_empty() || (ch.is_uppercase() && previous_lower) {
            words.push(String::new());
        }
        previous_lower = ch.is_lowercase() || ch.is_numeric();
        if let Some(word) = words.last_mut() {
            word.extend(ch.to_lowercase());
        }
    }
    words.retain(|word| !word.is_empty());
    let length: usize = words.iter().map(String::len).sum();
    (length > 0 && length <= MAX_PART).then_some(words)
}

/// Joins in the style the source is written in: `auditLogs` gets `auditLogsByMonth`,
/// `audit-logs` gets `audit-logs-by-month`, anything else `audit_logs_by_month`.
fn join(source: &str, parts: &[Vec<String>]) -> String {
    let words = parts.iter().flatten();
    let camel = source.chars().any(char::is_uppercase) && !source.contains(['_', '-']);
    if camel {
        let capitalized = words.map(|word| {
            let mut chars = word.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>())
        });
        return std::iter::once(source.to_string()).chain(capitalized.flatten()).collect();
    }
    let separator = if source.contains('-') && !source.contains('_') { "-" } else { "_" };
    std::iter::once(source).chain(words.map(String::as_str)).collect::<Vec<_>>().join(separator)
}

/// `candidate`, or the first numbered form of it that is free, numbered in its own style.
fn unique(candidate: String, taken: &[String]) -> String {
    if !taken.contains(&candidate) {
        return candidate;
    }
    (2usize..)
        .map(|n| join(&candidate, &[vec![n.to_string()]]))
        .find(|name| !taken.contains(name))
        .unwrap_or(candidate)
}

#[cfg(test)]
mod tests {
    use mongodb::bson::doc;

    use super::{suggest_copy_name, suggest_view_name};

    #[test]
    fn names_say_what_the_pipeline_keeps_and_shapes() {
        let name = |source: &str, pipeline: &[_]| suggest_view_name(source, pipeline, &[]);
        assert_eq!(name("orders", &[doc! { "$match": { "status": "open" } }]), "orders_open");
        assert_eq!(
            name(
                "orders",
                &[
                    doc! { "$match": { "status": "open" } },
                    doc! { "$group": { "_id": "$customer.country", "n": { "$sum": 1 } } },
                ]
            ),
            "orders_open_by_country"
        );
        assert_eq!(
            name(
                "orders",
                &[doc! { "$group": { "_id": { "$dateTrunc": {
                    "date": "$createdAt", "unit": "month"
                } } } }]
            ),
            "orders_by_month"
        );
        assert_eq!(name("orders", &[doc! { "$group": { "_id": null } }]), "orders_totals");
        assert_eq!(name("users", &[doc! { "$match": { "active": true } }]), "users_active");
        assert_eq!(name("users", &[doc! { "$match": { "deleted": false } }]), "users_not_deleted");
        assert_eq!(
            name("orders", &[doc! { "$lookup": { "from": "customers", "as": "c" } }]),
            "orders_with_customers"
        );
        assert_eq!(name("orders", &[doc! { "$unwind": "$line.items" }]), "orders_items");
        assert_eq!(name("orders", &[doc! { "$sortByCount": "$status" }]), "orders_by_status");
    }

    #[test]
    fn names_follow_the_sources_style() {
        let pipeline = [doc! { "$group": { "_id": { "year": 1, "month": 1 } } }];
        assert_eq!(suggest_view_name("auditLogs", &pipeline, &[]), "auditLogsByYearMonth");
        assert_eq!(suggest_view_name("audit-logs", &pipeline, &[]), "audit-logs-by-year-month");
        assert_eq!(suggest_view_name("audit_logs", &pipeline, &[]), "audit_logs_by_year_month");
    }

    #[test]
    fn a_pipeline_that_says_nothing_short_still_gets_a_free_name() {
        // A range, an operator and a sentence-long value are not names.
        let vague = [doc! { "$match": {
            "at": { "$gte": 5 },
            "$or": [],
            "note": "a very long free text value that nobody wants in a name",
        } }];
        assert_eq!(suggest_view_name("orders", &vague, &[]), "orders_view");
        assert_eq!(suggest_view_name("orders", &[], &["orders_view".into()]), "orders_view_2");
        let taken = ["orders_view".to_string(), "orders_view_2".to_string()];
        assert_eq!(suggest_view_name("orders", &[], &taken), "orders_view_3");
        assert_eq!(suggest_copy_name("orders_open", &[]), "orders_open_copy");
        assert_eq!(suggest_copy_name("ordersOpen", &["ordersOpenCopy".into()]), "ordersOpenCopy2");
    }
}
