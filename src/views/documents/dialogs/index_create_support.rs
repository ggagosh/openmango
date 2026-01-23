use std::collections::HashMap;

use mongodb::bson::{Bson, Document};

pub(super) const SAMPLE_SIZE: i64 = 500;
pub(super) const MAX_SUGGESTIONS: usize = 12;
pub(super) const MAX_ARRAY_SCAN: usize = 20;
pub(super) const MAX_DEPTH: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum IndexMode {
    Form,
    Json,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum IndexKeyKind {
    Asc,
    Desc,
    Text,
    Hashed,
    TwoDSphere,
    Wildcard,
}

impl IndexKeyKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            IndexKeyKind::Asc => "1",
            IndexKeyKind::Desc => "-1",
            IndexKeyKind::Text => "text",
            IndexKeyKind::Hashed => "hashed",
            IndexKeyKind::TwoDSphere => "2dsphere",
            IndexKeyKind::Wildcard => "wildcard ($**)",
        }
    }

    pub(super) fn as_bson(self) -> Bson {
        match self {
            IndexKeyKind::Asc => Bson::Int32(1),
            IndexKeyKind::Desc => Bson::Int32(-1),
            IndexKeyKind::Text => Bson::String("text".to_string()),
            IndexKeyKind::Hashed => Bson::String("hashed".to_string()),
            IndexKeyKind::TwoDSphere => Bson::String("2dsphere".to_string()),
            IndexKeyKind::Wildcard => Bson::Int32(1),
        }
    }
}

#[derive(Default, Clone, Copy)]
pub(super) struct IndexKeySummary {
    pub(super) key_count: usize,
    pub(super) has_text: bool,
    pub(super) has_hashed: bool,
    pub(super) has_wildcard: bool,
    pub(super) has_special: bool,
}

#[derive(Clone)]
pub(super) struct FieldSuggestion {
    pub(super) path: String,
    pub(super) count: usize,
}

#[derive(Clone)]
pub(super) struct IndexEditTarget {
    pub(super) original_name: String,
}

#[derive(Clone)]
pub(super) enum SampleStatus {
    Idle,
    Loading,
    Ready,
    Error(String),
}

pub(super) fn build_field_suggestions(docs: &[Document]) -> Vec<FieldSuggestion> {
    let mut counts: HashMap<String, usize> = HashMap::new();

    for doc in docs {
        for (key, value) in doc {
            let path = key.to_string();
            *counts.entry(path.clone()).or_insert(0) += 1;
            collect_paths(value, &path, &mut counts, 1);
        }
    }

    let mut suggestions =
        counts.into_iter().map(|(path, count)| FieldSuggestion { path, count }).collect::<Vec<_>>();

    suggestions.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.path.cmp(&b.path)));
    suggestions.truncate(200);
    suggestions
}

pub(super) fn index_kind_from_bson(key: &str, value: &Bson) -> IndexKeyKind {
    if key == "$**" {
        return IndexKeyKind::Wildcard;
    }

    match value {
        Bson::String(text) => match text.as_str() {
            "text" => IndexKeyKind::Text,
            "hashed" => IndexKeyKind::Hashed,
            "2dsphere" => IndexKeyKind::TwoDSphere,
            "-1" => IndexKeyKind::Desc,
            _ => IndexKeyKind::Asc,
        },
        Bson::Int32(val) => {
            if *val < 0 {
                IndexKeyKind::Desc
            } else {
                IndexKeyKind::Asc
            }
        }
        Bson::Int64(val) => {
            if *val < 0 {
                IndexKeyKind::Desc
            } else {
                IndexKeyKind::Asc
            }
        }
        Bson::Double(val) => {
            if *val < 0.0 {
                IndexKeyKind::Desc
            } else {
                IndexKeyKind::Asc
            }
        }
        _ => IndexKeyKind::Asc,
    }
}

fn collect_paths(value: &Bson, prefix: &str, counts: &mut HashMap<String, usize>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }

    match value {
        Bson::Document(doc) => {
            for (key, value) in doc {
                let path = format!("{prefix}.{key}");
                *counts.entry(path.clone()).or_insert(0) += 1;
                collect_paths(value, &path, counts, depth + 1);
            }
        }
        Bson::Array(values) => {
            if prefix.is_empty() {
                return;
            }
            let array_path = format!("{prefix}[]");
            *counts.entry(array_path.clone()).or_insert(0) += 1;

            for value in values.iter().take(MAX_ARRAY_SCAN) {
                match value {
                    Bson::Document(doc) => {
                        for (key, value) in doc {
                            let path = format!("{array_path}.{key}");
                            *counts.entry(path.clone()).or_insert(0) += 1;
                            collect_paths(value, &path, counts, depth + 1);
                        }
                    }
                    Bson::Array(_) => {
                        collect_paths(value, &array_path, counts, depth + 1);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}
