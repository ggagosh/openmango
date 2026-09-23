//! Flatten only expanded branches of one document pair; unchanged siblings share a disclosure.

use crate::bson::compare::{ChangeKind, field_changes};
use crate::bson::{PathSegment, get_bson_at_path};
use crate::state::compare::{CompareConfig, CompareDetail};
use mongodb::bson::{Bson, Document};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct Expansion {
    pub unchanged: HashSet<Vec<PathSegment>>,
    pub collapsed: HashSet<Vec<PathSegment>>,
}

pub(super) enum DetailRow {
    Field {
        path: Vec<PathSegment>,
        kind: Option<ChangeKind>,
        informational: bool,
        container: bool,
        expanded: bool,
    },
    Order {
        path: Vec<PathSegment>,
    },
    Unchanged {
        path: Vec<PathSegment>,
        count: usize,
    },
}

pub(super) fn label(path: &[PathSegment]) -> String {
    if path.is_empty() { "Document".into() } else { crate::bson::dotted_path(path) }
}

pub(super) fn detail_rows(
    pair: &CompareDetail,
    config: &CompareConfig,
    expansion: &Expansion,
) -> crate::error::Result<Vec<DetailRow>> {
    let empty = Document::new();
    let documents =
        [pair.documents[0].first().unwrap_or(&empty), pair.documents[1].first().unwrap_or(&empty)];
    let changes = field_changes(documents[0], documents[1], &config.ignore_set())?;
    let mut kinds = HashMap::new();
    let mut changed = HashSet::new();
    for change in changes {
        for depth in 0..=change.path.len() {
            changed.insert(change.path[..depth].to_vec());
        }
        kinds.insert(change.path, change.kind);
    }
    let mut tree = Tree { documents, config, expansion, kinds, changed, rows: Vec::new() };
    if config.fields != ["_id"] && documents.iter().any(|d| d.contains_key("_id")) {
        tree.rows.push(DetailRow::Field {
            path: vec![PathSegment::Key("_id".into())],
            kind: None,
            informational: true,
            container: false,
            expanded: false,
        });
    }
    tree.level(&[]);
    Ok(tree.rows)
}

struct Tree<'a> {
    documents: [&'a Document; 2],
    config: &'a CompareConfig,
    expansion: &'a Expansion,
    kinds: HashMap<Vec<PathSegment>, ChangeKind>,
    changed: HashSet<Vec<PathSegment>>,
    rows: Vec<DetailRow>,
}

impl Tree<'_> {
    fn children(&self, path: &[PathSegment]) -> Vec<Vec<PathSegment>> {
        let mut children = Vec::new();
        let mut seen = HashSet::new();
        for document in self.documents {
            let parts: Vec<_> = if path.is_empty() {
                document.keys().cloned().map(PathSegment::Key).collect()
            } else {
                match get_bson_at_path(document, path) {
                    Some(Bson::Document(d)) => d.keys().cloned().map(PathSegment::Key).collect(),
                    Some(Bson::Array(a)) => (0..a.len()).map(PathSegment::Index).collect(),
                    _ => Vec::new(),
                }
            };
            for part in parts {
                if seen.insert(part.clone()) {
                    let mut child = path.to_vec();
                    child.push(part);
                    children.push(child);
                }
            }
        }
        children
    }

    fn level(&mut self, path: &[PathSegment]) {
        if self.kinds.get(path) == Some(&ChangeKind::FieldOrder) {
            self.rows.push(DetailRow::Order { path: path.to_vec() });
        }
        let children = self.children(path);
        let mut unchanged = Vec::new();
        for child in children {
            let name = label(&child);
            if self.config.ignore.contains(&name)
                || (self.config.fields != ["_id"] && name == "_id")
            {
                continue;
            }
            if self.changed.contains(&child) {
                self.field(child);
            } else {
                unchanged.push(child);
            }
        }
        if !unchanged.is_empty() {
            self.rows.push(DetailRow::Unchanged { path: path.to_vec(), count: unchanged.len() });
            if self.expansion.unchanged.contains(path) {
                for path in unchanged {
                    self.field(path);
                }
            }
        }
    }

    fn field(&mut self, path: Vec<PathSegment>) {
        let values = self.documents.map(|d| get_bson_at_path(d, &path));
        let kind = self.kinds.get(&path).copied();
        // A reordered array is one minor change; its items are equal, so there is nothing to expand.
        let container = kind != Some(ChangeKind::ArrayOrder)
            && matches!(
                values,
                [Some(Bson::Document(_)), Some(Bson::Document(_))]
                    | [Some(Bson::Array(_)), Some(Bson::Array(_))]
                    | [Some(Bson::Document(_) | Bson::Array(_)), None]
                    | [None, Some(Bson::Document(_) | Bson::Array(_))]
            );
        let expanded = container && !self.expansion.collapsed.contains(&path);
        self.rows.push(DetailRow::Field {
            kind,
            path: path.clone(),
            informational: false,
            container,
            expanded,
        });
        if expanded {
            self.level(&path);
        }
    }
}
