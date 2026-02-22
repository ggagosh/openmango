use std::collections::{HashMap, HashSet};

use crate::state::{CardinalityBand, SchemaAnalysis, SchemaCardinality, SchemaField};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SchemaFlag {
    Polymorphic,
    Sparse,
    Complete,
    Nullable,
}

impl SchemaFlag {
    pub(crate) fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "polymorphic" => Some(Self::Polymorphic),
            "sparse" => Some(Self::Sparse),
            "complete" => Some(Self::Complete),
            "nullable" => Some(Self::Nullable),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Polymorphic => "polymorphic",
            Self::Sparse => "sparse",
            Self::Complete => "complete",
            Self::Nullable => "nullable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresenceOperator {
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PresenceRule {
    pub operator: PresenceOperator,
    pub value: f64,
}

impl PresenceRule {
    fn matches(self, value: f64) -> bool {
        match self.operator {
            PresenceOperator::Gt => value > self.value,
            PresenceOperator::Gte => value >= self.value,
            PresenceOperator::Lt => value < self.value,
            PresenceOperator::Lte => value <= self.value,
            PresenceOperator::Eq => (value - self.value).abs() < f64::EPSILON,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SchemaFilterTokenKind {
    Type(String),
    Presence(PresenceRule),
    Cardinality(CardinalityBand),
    Flag(SchemaFlag),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SchemaFilterToken {
    pub raw: String,
    pub kind: SchemaFilterTokenKind,
}

impl SchemaFilterToken {
    pub(crate) fn chip_label(&self) -> String {
        self.raw.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ParsedSchemaFilter {
    pub tokens: Vec<SchemaFilterToken>,
    pub query: String,
}

impl ParsedSchemaFilter {
    pub(crate) fn has_active_filter(&self) -> bool {
        !self.tokens.is_empty() || !self.query.trim().is_empty()
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SchemaFilterPlan {
    pub parsed: ParsedSchemaFilter,
    matched_paths: HashSet<String>,
}

impl SchemaFilterPlan {
    pub(crate) fn has_active_filter(&self) -> bool {
        self.parsed.has_active_filter()
    }

    pub(crate) fn matches_path(&self, path: &str) -> bool {
        self.matched_paths.contains(path)
    }
}

pub(crate) fn build_schema_filter_input(tokens: &[SchemaFilterToken], query: &str) -> String {
    let mut parts: Vec<String> = tokens.iter().map(|token| token.raw.clone()).collect();
    let trimmed_query = query.trim();
    if !trimmed_query.is_empty() {
        parts.push(trimmed_query.to_string());
    }
    parts.join(" ")
}

pub(crate) fn parse_schema_filter(raw: &str) -> ParsedSchemaFilter {
    let mut tokens = Vec::new();
    let mut index = 0usize;
    let bytes = raw.as_bytes();

    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }

        let token_start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
            index += 1;
        }

        let candidate = &raw[token_start..index];
        if let Some(token) = parse_token(candidate) {
            tokens.push(token);
            continue;
        }

        index = token_start;
        break;
    }

    let query = raw[index..].trim().to_string();
    ParsedSchemaFilter { tokens, query }
}

pub(crate) fn compile_schema_filter(schema: &SchemaAnalysis, raw: &str) -> SchemaFilterPlan {
    let parsed = parse_schema_filter(raw);
    if !parsed.has_active_filter() {
        return SchemaFilterPlan { parsed, matched_paths: HashSet::new() };
    }

    let metadata = collect_field_metadata(schema);
    let matched_paths = metadata
        .iter()
        .filter(|meta| matches_tokens(meta, &parsed.tokens))
        .filter(|meta| plain_query_matches(meta, &parsed.query))
        .map(|meta| meta.path.clone())
        .collect();

    SchemaFilterPlan { parsed, matched_paths }
}

#[derive(Clone, Debug)]
struct FieldMetadata {
    path: String,
    path_lower: String,
    name_lower: String,
    types_lower: Vec<String>,
    presence_pct: f64,
    is_polymorphic: bool,
    is_sparse: bool,
    is_complete: bool,
    is_nullable: bool,
    cardinality: Option<SchemaCardinality>,
}

fn collect_field_metadata(schema: &SchemaAnalysis) -> Vec<FieldMetadata> {
    fn recurse(
        out: &mut Vec<FieldMetadata>,
        fields: &[SchemaField],
        sampled: u64,
        cardinality: &HashMap<String, SchemaCardinality>,
    ) {
        for field in fields {
            let presence_pct =
                if sampled == 0 { 0.0 } else { (field.presence as f64 / sampled as f64) * 100.0 };
            let types_lower: Vec<String> =
                field.types.iter().map(|ty| ty.bson_type.to_ascii_lowercase()).collect();
            out.push(FieldMetadata {
                path: field.path.clone(),
                path_lower: field.path.to_ascii_lowercase(),
                name_lower: field.name.to_ascii_lowercase(),
                types_lower,
                presence_pct,
                is_polymorphic: field.is_polymorphic,
                is_sparse: sampled > 0 && field.presence < sampled,
                is_complete: sampled > 0 && field.presence == sampled,
                is_nullable: field.null_count > 0,
                cardinality: cardinality.get(&field.path).cloned(),
            });
            recurse(out, &field.children, sampled, cardinality);
        }
    }

    let mut output = Vec::new();
    recurse(&mut output, &schema.fields, schema.sampled, &schema.cardinality);
    output
}

fn matches_tokens(field: &FieldMetadata, tokens: &[SchemaFilterToken]) -> bool {
    tokens.iter().all(|token| match &token.kind {
        SchemaFilterTokenKind::Type(needle) => {
            field.types_lower.iter().any(|value| value == needle)
        }
        SchemaFilterTokenKind::Presence(rule) => rule.matches(field.presence_pct),
        SchemaFilterTokenKind::Cardinality(band) => {
            field.cardinality.as_ref().is_some_and(|card| card.band == *band)
        }
        SchemaFilterTokenKind::Flag(flag) => match flag {
            SchemaFlag::Polymorphic => field.is_polymorphic,
            SchemaFlag::Sparse => field.is_sparse,
            SchemaFlag::Complete => field.is_complete,
            SchemaFlag::Nullable => field.is_nullable,
        },
    })
}

fn plain_query_matches(field: &FieldMetadata, query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return true;
    }

    trimmed.split_whitespace().map(str::to_ascii_lowercase).all(|term| {
        field.path_lower.contains(&term)
            || field.name_lower.contains(&term)
            || field.types_lower.iter().any(|ty| ty.contains(&term))
    })
}

fn parse_token(input: &str) -> Option<SchemaFilterToken> {
    let (name, value) = input.split_once(':')?;
    if value.trim().is_empty() {
        return None;
    }

    let kind = match name.trim().to_ascii_lowercase().as_str() {
        "type" => SchemaFilterTokenKind::Type(value.trim().to_ascii_lowercase()),
        "presence" => SchemaFilterTokenKind::Presence(parse_presence_rule(value.trim())?),
        "cardinality" | "card" => {
            SchemaFilterTokenKind::Cardinality(parse_cardinality_band(value.trim())?)
        }
        "flag" | "is" => SchemaFilterTokenKind::Flag(SchemaFlag::parse(value.trim())?),
        _ => return None,
    };

    Some(SchemaFilterToken { raw: input.to_string(), kind })
}

fn parse_presence_rule(value: &str) -> Option<PresenceRule> {
    let (operator, tail) = if let Some(rest) = value.strip_prefix(">=") {
        (PresenceOperator::Gte, rest)
    } else if let Some(rest) = value.strip_prefix("<=") {
        (PresenceOperator::Lte, rest)
    } else if let Some(rest) = value.strip_prefix('>') {
        (PresenceOperator::Gt, rest)
    } else if let Some(rest) = value.strip_prefix('<') {
        (PresenceOperator::Lt, rest)
    } else if let Some(rest) = value.strip_prefix('=') {
        (PresenceOperator::Eq, rest)
    } else {
        (PresenceOperator::Gte, value)
    };

    let number = tail.trim().trim_end_matches('%').parse::<f64>().ok()?;
    Some(PresenceRule { operator, value: number })
}

fn parse_cardinality_band(value: &str) -> Option<CardinalityBand> {
    match value.trim().to_ascii_lowercase().as_str() {
        "low" => Some(CardinalityBand::Low),
        "medium" | "med" => Some(CardinalityBand::Medium),
        "high" => Some(CardinalityBand::High),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::state::{SchemaField, SchemaFieldType};

    use super::*;

    fn schema_field(
        path: &str,
        name: &str,
        depth: usize,
        bson_type: &str,
        presence: u64,
        null_count: u64,
        is_polymorphic: bool,
    ) -> SchemaField {
        SchemaField {
            path: path.to_string(),
            name: name.to_string(),
            depth,
            types: vec![SchemaFieldType {
                bson_type: bson_type.to_string(),
                count: 10,
                percentage: 100.0,
            }],
            presence,
            null_count,
            is_polymorphic,
            children: Vec::new(),
        }
    }

    fn make_schema() -> SchemaAnalysis {
        let mut cardinality = HashMap::new();
        cardinality.insert(
            "completedBy".to_string(),
            SchemaCardinality {
                distinct_estimate: 22,
                band: CardinalityBand::Medium,
                min_value: Some("631ea587a104e342c0ffc76b".to_string()),
                max_value: Some("6980170fe8e27c1748378808".to_string()),
            },
        );

        SchemaAnalysis {
            fields: vec![
                schema_field("completedBy", "completedBy", 0, "ObjectId", 657, 0, false),
                schema_field("title", "title", 0, "String", 1000, 5, true),
            ],
            total_fields: 2,
            total_types: 2,
            max_depth: 1,
            sampled: 1000,
            total_documents: 11396,
            polymorphic_count: 1,
            sparse_count: 1,
            complete_count: 1,
            sample_values: HashMap::new(),
            cardinality,
        }
    }

    #[test]
    fn parse_schema_filter_detects_tokens_and_remaining_query() {
        let parsed = parse_schema_filter("type:ObjectId presence:>=60 flag:sparse completed by");
        assert_eq!(parsed.tokens.len(), 3);
        assert_eq!(parsed.query, "completed by");
    }

    #[test]
    fn compile_schema_filter_supports_token_and_plain_text_and_logic() {
        let schema = make_schema();
        let plan = compile_schema_filter(&schema, "type:objectid completed");
        assert!(plan.matches_path("completedBy"));
        assert!(!plan.matches_path("title"));
    }

    #[test]
    fn compile_schema_filter_supports_presence_cardinality_and_flags() {
        let schema = make_schema();

        let sparse = compile_schema_filter(&schema, "flag:sparse");
        assert!(sparse.matches_path("completedBy"));
        assert!(!sparse.matches_path("title"));

        let medium = compile_schema_filter(&schema, "cardinality:medium");
        assert!(medium.matches_path("completedBy"));
        assert!(!medium.matches_path("title"));

        let nullable = compile_schema_filter(&schema, "flag:nullable");
        assert!(nullable.matches_path("title"));
        assert!(!nullable.matches_path("completedBy"));
    }

    #[test]
    fn build_schema_filter_input_round_trips_tokens_and_query() {
        let parsed = parse_schema_filter("type:string presence:>=80 completed");
        let rebuilt = build_schema_filter_input(&parsed.tokens, &parsed.query);
        assert_eq!(rebuilt, "type:string presence:>=80 completed");
    }
}
