use mongodb::bson::{Bson, Document, oid::ObjectId};
use regex::escape as regex_escape;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
    String,
    Number,
    Boolean,
    ObjectId,
    DateTime,
    Array,
    Document,
    Null,
    Unknown,
}

impl FieldType {
    pub fn from_bson(value: &Bson) -> Self {
        match value {
            Bson::String(_) | Bson::RegularExpression(_) => Self::String,
            Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_) => Self::Number,
            Bson::Boolean(_) => Self::Boolean,
            Bson::ObjectId(_) => Self::ObjectId,
            Bson::DateTime(_) | Bson::Timestamp(_) => Self::DateTime,
            Bson::Array(_) => Self::Array,
            Bson::Document(_) => Self::Document,
            Bson::Null => Self::Null,
            _ => Self::Unknown,
        }
    }

    pub fn from_field_name(name: &str) -> Self {
        if name == "_id" {
            return Self::ObjectId;
        }
        let lower = name.to_ascii_lowercase();
        if lower.ends_with("_at")
            || lower.ends_with("date")
            || lower.ends_with("time")
            || lower.ends_with("timestamp")
        {
            return Self::DateTime;
        }
        if lower.starts_with("is_") || lower.starts_with("has_") || lower.starts_with("can_") {
            return Self::Boolean;
        }
        Self::Unknown
    }

    pub fn default_operator(self) -> FilterOperator {
        match self {
            Self::Boolean => FilterOperator::Eq,
            Self::Array => FilterOperator::In,
            Self::Null => FilterOperator::Eq,
            _ => FilterOperator::Eq,
        }
    }

    pub fn available_operators(self) -> &'static [FilterOperator] {
        use FilterOperator::*;
        match self {
            Self::ObjectId => &[Eq, Ne, In, Nin, Exists],
            Self::String => &[Eq, Ne, Regex, In, Nin, Exists],
            Self::Number => &[Eq, Ne, Gt, Gte, Lt, Lte, In, Nin, Exists],
            Self::Boolean => &[Eq, Ne, Exists],
            Self::DateTime => &[Eq, Ne, Gt, Gte, Lt, Lte, Exists],
            Self::Array => {
                &[Eq, Ne, Gt, Gte, Lt, Lte, In, Nin, All, Regex, Size, IsEmpty, NotEmpty, Exists]
            }
            Self::Document => &[Eq, Ne, Exists],
            Self::Null => &[Eq, Ne, Exists],
            Self::Unknown => &[Eq, Ne, Gt, Gte, Lt, Lte, In, Nin, Regex, Exists],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueEditorKind {
    None,
    Single,
    List,
    Toggle,
    Range,
}

impl ValueEditorKind {
    pub fn is_multiline(&self) -> bool {
        matches!(self, Self::List | Self::Range)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterOperator {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    Between,
    In,
    Nin,
    All,
    Exists,
    Contains,
    StartsWith,
    EndsWith,
    Regex,
    Size,
    IsEmpty,
    NotEmpty,
}

impl FilterOperator {
    pub fn label_for(self, _field_type: FieldType) -> &'static str {
        match self {
            Self::Eq => "$eq",
            Self::Ne => "$ne",
            Self::Gt => "$gt",
            Self::Gte => "$gte",
            Self::Lt => "$lt",
            Self::Lte => "$lte",
            Self::Between => "between",
            Self::In => "$in",
            Self::Nin => "$nin",
            Self::All => "$all",
            Self::Exists => "$exists",
            Self::Contains => "contains",
            Self::StartsWith => "starts with",
            Self::EndsWith => "ends with",
            Self::Regex => "$regex",
            Self::Size => "$size",
            Self::IsEmpty => "is empty",
            Self::NotEmpty => "is not empty",
        }
    }

    pub fn value_editor_kind(self, field_type: FieldType) -> ValueEditorKind {
        match self {
            Self::Exists => ValueEditorKind::Toggle,
            Self::Between => ValueEditorKind::Range,
            Self::In | Self::Nin | Self::All => ValueEditorKind::List,
            Self::IsEmpty | Self::NotEmpty => ValueEditorKind::None,
            Self::Eq | Self::Ne if field_type == FieldType::Boolean => ValueEditorKind::Toggle,
            Self::Eq | Self::Ne if field_type == FieldType::Null => ValueEditorKind::None,
            _ => ValueEditorKind::Single,
        }
    }

    pub fn mongo_key(self) -> Option<&'static str> {
        match self {
            Self::Ne => Some("$ne"),
            Self::Gt => Some("$gt"),
            Self::Gte => Some("$gte"),
            Self::Lt => Some("$lt"),
            Self::Lte => Some("$lte"),
            Self::In => Some("$in"),
            Self::Nin => Some("$nin"),
            Self::All => Some("$all"),
            Self::Exists => Some("$exists"),
            Self::Regex => Some("$regex"),
            Self::Size => Some("$size"),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combinator {
    And,
    Or,
}

impl Combinator {
    pub fn label(self) -> &'static str {
        match self {
            Self::And => "$and",
            Self::Or => "$or",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::And => "$and",
            Self::Or => "$or",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::And => Self::Or,
            Self::Or => Self::And,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ConditionValue {
    #[default]
    Empty,
    Scalar(String),
    List(Vec<String>),
    Bool(bool),
    Range {
        start: String,
        end: String,
    },
}

impl ConditionValue {
    pub fn has_content(&self) -> bool {
        match self {
            Self::Empty => false,
            Self::Scalar(value) => !value.trim().is_empty(),
            Self::List(values) => values.iter().any(|value| !value.trim().is_empty()),
            Self::Bool(_) => true,
            Self::Range { start, end } => !start.trim().is_empty() || !end.trim().is_empty(),
        }
    }

    pub fn scalar(&self) -> Option<&str> {
        match self {
            Self::Scalar(value) => Some(value.as_str()),
            _ => None,
        }
    }

    pub fn scalar_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::Scalar(value) => Some(value),
            _ => None,
        }
    }

    pub fn list(&self) -> Option<&[String]> {
        match self {
            Self::List(values) => Some(values),
            _ => None,
        }
    }

    pub fn list_mut(&mut self) -> Option<&mut Vec<String>> {
        match self {
            Self::List(values) => Some(values),
            _ => None,
        }
    }

    pub fn bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn range(&self) -> Option<(&str, &str)> {
        match self {
            Self::Range { start, end } => Some((start.as_str(), end.as_str())),
            _ => None,
        }
    }

    pub fn range_mut(&mut self) -> Option<(&mut String, &mut String)> {
        match self {
            Self::Range { start, end } => Some((start, end)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterCondition {
    pub id: u64,
    pub field: String,
    pub field_type: FieldType,
    pub operator: FilterOperator,
    pub value: ConditionValue,
}

impl FilterCondition {
    pub fn new(id: u64) -> Self {
        let field_type = FieldType::Unknown;
        let operator = field_type.default_operator();
        Self {
            id,
            field: String::new(),
            field_type,
            operator,
            value: default_value(field_type, operator),
        }
    }

    pub fn set_field_type(&mut self, field_type: FieldType) {
        if self.field_type == field_type {
            return;
        }
        self.field_type = field_type;
        let available = field_type.available_operators();
        if !available.contains(&self.operator) {
            self.operator = field_type.default_operator();
            self.value = default_value(field_type, self.operator);
            return;
        }
        self.value = coerce_value_for(self.value.clone(), field_type, self.operator);
    }

    pub fn set_operator(&mut self, operator: FilterOperator) {
        if self.operator == operator {
            return;
        }
        self.operator = operator;
        self.value = coerce_value_for(self.value.clone(), self.field_type, operator);
    }

    pub fn value_editor_kind(&self) -> ValueEditorKind {
        self.operator.value_editor_kind(self.field_type)
    }

    pub fn has_input(&self) -> bool {
        !self.field.trim().is_empty() || self.value.has_content()
    }

    pub fn validation_error(&self) -> Option<String> {
        if !self.has_input() {
            return None;
        }
        if self.field.trim().is_empty() {
            return Some("Choose a field".to_string());
        }

        match self.value_editor_kind() {
            ValueEditorKind::None | ValueEditorKind::Toggle => None,
            ValueEditorKind::Single => {
                let raw = self.value.scalar().unwrap_or("").trim();
                if raw.is_empty() {
                    return Some("Enter a value".to_string());
                }
                if self.parse_scalar(raw).is_none() {
                    return Some(value_error_message(self.field_type).to_string());
                }
                None
            }
            ValueEditorKind::List => {
                let values = self
                    .value
                    .list()
                    .unwrap_or(&[])
                    .iter()
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>();
                if values.is_empty() {
                    return Some("Add at least one value".to_string());
                }
                if values.iter().any(|value| self.parse_scalar(value).is_none()) {
                    return Some(value_error_message(self.field_type).to_string());
                }
                None
            }
            ValueEditorKind::Range => {
                let Some((start, end)) = self.value.range() else {
                    return Some("Enter a range".to_string());
                };
                if start.trim().is_empty() || end.trim().is_empty() {
                    return Some("Enter both values".to_string());
                }
                if self.parse_scalar(start).is_none() || self.parse_scalar(end).is_none() {
                    return Some(value_error_message(self.field_type).to_string());
                }
                None
            }
        }
    }

    pub fn to_document(&self) -> Option<Document> {
        if self.field.trim().is_empty() {
            return None;
        }
        if self.validation_error().is_some() {
            return None;
        }

        let field = self.field.clone();
        match self.operator {
            FilterOperator::Eq => {
                let value = self.scalar_bson()?;
                let mut doc = Document::new();
                doc.insert(field, value);
                Some(doc)
            }
            FilterOperator::Ne
            | FilterOperator::Gt
            | FilterOperator::Gte
            | FilterOperator::Lt
            | FilterOperator::Lte
            | FilterOperator::Exists
            | FilterOperator::Size => {
                let key = self.operator.mongo_key()?;
                let value = match self.operator {
                    FilterOperator::Exists => Bson::Boolean(self.value.bool().unwrap_or(true)),
                    FilterOperator::Size => {
                        let raw = self.value.scalar().unwrap_or("");
                        raw.trim().parse::<i64>().ok().map(Bson::Int64)?
                    }
                    _ => self.scalar_bson()?,
                };
                let mut inner = Document::new();
                inner.insert(key, value);
                let mut doc = Document::new();
                doc.insert(field, inner);
                Some(doc)
            }
            FilterOperator::Between => {
                let (start, end) = self.value.range()?;
                let mut inner = Document::new();
                inner.insert("$gte", self.parse_scalar(start.trim())?);
                inner.insert("$lte", self.parse_scalar(end.trim())?);
                let mut doc = Document::new();
                doc.insert(field, inner);
                Some(doc)
            }
            FilterOperator::In | FilterOperator::Nin | FilterOperator::All => {
                let key = self.operator.mongo_key()?;
                let values = self
                    .value
                    .list()
                    .unwrap_or(&[])
                    .iter()
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .filter_map(|value| self.parse_scalar(value))
                    .collect::<Vec<_>>();
                let mut inner = Document::new();
                inner.insert(key, Bson::Array(values));
                let mut doc = Document::new();
                doc.insert(field, inner);
                Some(doc)
            }
            FilterOperator::Contains => {
                regex_document(&field, regex_escape(self.value.scalar()?.trim()))
            }
            FilterOperator::StartsWith => {
                regex_document(&field, format!("^{}", regex_escape(self.value.scalar()?.trim())))
            }
            FilterOperator::EndsWith => {
                regex_document(&field, format!("{}$", regex_escape(self.value.scalar()?.trim())))
            }
            FilterOperator::Regex => {
                regex_document(&field, self.value.scalar()?.trim().to_string())
            }
            FilterOperator::IsEmpty => {
                let mut doc = Document::new();
                match self.field_type {
                    FieldType::Array => {
                        let mut inner = Document::new();
                        inner.insert("$size", Bson::Int64(0));
                        doc.insert(field, inner);
                    }
                    FieldType::Null => {
                        doc.insert(field, Bson::Null);
                    }
                    _ => {
                        doc.insert(field, Bson::String(String::new()));
                    }
                }
                Some(doc)
            }
            FilterOperator::NotEmpty => {
                let mut doc = Document::new();
                match self.field_type {
                    FieldType::Array => {
                        let mut not_inner = Document::new();
                        not_inner.insert("$size", Bson::Int64(0));
                        let mut inner = Document::new();
                        inner.insert("$not", Bson::Document(not_inner));
                        doc.insert(field, inner);
                    }
                    _ => {
                        let mut inner = Document::new();
                        inner.insert("$ne", Bson::String(String::new()));
                        doc.insert(field, inner);
                    }
                }
                Some(doc)
            }
        }
    }

    pub fn scalar_display_value(&self) -> String {
        match &self.value {
            ConditionValue::Scalar(value) => value.clone(),
            ConditionValue::Bool(value) => value.to_string(),
            ConditionValue::List(values) => values.join(", "),
            ConditionValue::Range { start, end } => format!("{start} -> {end}"),
            ConditionValue::Empty => String::new(),
        }
    }

    fn scalar_bson(&self) -> Option<Bson> {
        match self.value_editor_kind() {
            ValueEditorKind::Toggle => Some(Bson::Boolean(self.value.bool().unwrap_or(true))),
            ValueEditorKind::None if self.field_type == FieldType::Null => Some(Bson::Null),
            _ => self.parse_scalar(self.value.scalar()?.trim()),
        }
    }

    fn parse_scalar(&self, raw: &str) -> Option<Bson> {
        if raw.is_empty() && self.field_type != FieldType::Null {
            return None;
        }

        match self.field_type {
            FieldType::ObjectId => {
                let cleaned = raw
                    .trim_start_matches("ObjectId(\"")
                    .trim_start_matches("ObjectId('")
                    .trim_end_matches("\")")
                    .trim_end_matches("')")
                    .trim_matches('"')
                    .trim_matches('\'');
                ObjectId::parse_str(cleaned).ok().map(Bson::ObjectId)
            }
            FieldType::Number => {
                if let Ok(value) = raw.parse::<i64>() {
                    Some(Bson::Int64(value))
                } else if let Ok(value) = raw.parse::<f64>() {
                    Some(Bson::Double(value))
                } else {
                    None
                }
            }
            FieldType::Boolean => match raw.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => Some(Bson::Boolean(true)),
                "false" | "0" | "no" => Some(Bson::Boolean(false)),
                _ => None,
            },
            FieldType::DateTime => parse_datetime(raw),
            FieldType::Null => Some(Bson::Null),
            FieldType::Array => crate::bson::parse_bson_from_relaxed_json(raw)
                .ok()
                .or_else(|| Some(Bson::String(raw.to_string()))),
            FieldType::Document => crate::bson::parse_bson_from_relaxed_json(raw).ok(),
            _ => Some(Bson::String(raw.to_string())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterNode {
    Condition(FilterCondition),
    Group { id: u64, combinator: Combinator, children: Vec<FilterNode> },
}

impl FilterNode {
    pub fn node_id(&self) -> u64 {
        match self {
            Self::Condition(condition) => condition.id,
            Self::Group { id, .. } => *id,
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self, Self::Group { .. })
    }

    fn condition_mut(&mut self, id: u64) -> Option<&mut FilterCondition> {
        match self {
            Self::Condition(condition) if condition.id == id => Some(condition),
            Self::Condition(_) => None,
            Self::Group { children, .. } => {
                children.iter_mut().find_map(|child| child.condition_mut(id))
            }
        }
    }

    fn group_mut(&mut self, id: u64) -> Option<&mut FilterNode> {
        match self {
            Self::Group { id: group_id, .. } if *group_id == id => Some(self),
            Self::Group { children, .. } => {
                children.iter_mut().find_map(|child| child.group_mut(id))
            }
            Self::Condition(_) => None,
        }
    }

    fn contains_node(&self, id: u64) -> bool {
        match self {
            Self::Condition(condition) => condition.id == id,
            Self::Group { id: group_id, children, .. } => {
                *group_id == id || children.iter().any(|child| child.contains_node(id))
            }
        }
    }

    fn collect_conditions(&self, out: &mut Vec<FilterCondition>) {
        match self {
            Self::Condition(condition) => out.push(condition.clone()),
            Self::Group { children, .. } => {
                for child in children {
                    child.collect_conditions(out);
                }
            }
        }
    }

    fn clone_with_new_ids(&self, next_id: &mut u64) -> Self {
        match self {
            Self::Condition(condition) => {
                let mut next = condition.clone();
                next.id = *next_id;
                *next_id += 1;
                Self::Condition(next)
            }
            Self::Group { combinator, children, .. } => {
                let id = *next_id;
                *next_id += 1;
                Self::Group {
                    id,
                    combinator: *combinator,
                    children: children
                        .iter()
                        .map(|child| child.clone_with_new_ids(next_id))
                        .collect(),
                }
            }
        }
    }

    fn to_document(&self) -> Option<Document> {
        match self {
            Self::Condition(condition) => condition.to_document(),
            Self::Group { combinator, children, .. } => {
                let parts = children.iter().filter_map(FilterNode::to_document).collect::<Vec<_>>();
                if parts.is_empty() { None } else { Some(combine_documents(*combinator, parts)) }
            }
        }
    }

    fn has_active_conditions(&self) -> bool {
        match self {
            Self::Condition(condition) => condition.has_input(),
            Self::Group { children, .. } => children.iter().any(FilterNode::has_active_conditions),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropTarget {
    pub parent_group_id: Option<u64>,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedFilter {
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterTree {
    pub combinator: Combinator,
    pub children: Vec<FilterNode>,
    next_id: u64,
}

impl Default for FilterTree {
    fn default() -> Self {
        Self::new()
    }
}

impl FilterTree {
    pub fn new() -> Self {
        Self { combinator: Combinator::And, children: Vec::new(), next_id: 1 }
    }

    pub fn root_drop_target(&self) -> DropTarget {
        DropTarget { parent_group_id: None, index: self.children.len() }
    }

    pub fn group_drop_target(&self, group_id: u64) -> Option<DropTarget> {
        let group = self.find_group(group_id)?;
        if let FilterNode::Group { children, .. } = group {
            Some(DropTarget { parent_group_id: Some(group_id), index: children.len() })
        } else {
            None
        }
    }

    pub fn add_condition(&mut self) -> u64 {
        let target = self.root_drop_target();
        self.insert_condition_at(target)
    }

    pub fn add_condition_to_group(&mut self, group_id: u64) -> Option<u64> {
        let target = self.group_drop_target(group_id)?;
        Some(self.insert_condition_at(target))
    }

    pub fn insert_condition_at(&mut self, target: DropTarget) -> u64 {
        let id = self.next_id();
        let node = FilterNode::Condition(FilterCondition::new(id));
        self.insert_node_at(node, target);
        id
    }

    pub fn add_group(&mut self) -> u64 {
        let target = self.root_drop_target();
        self.insert_group_at(target)
    }

    pub fn add_group_to_group(&mut self, parent_id: u64) -> Option<u64> {
        let target = self.group_drop_target(parent_id)?;
        Some(self.insert_group_at(target))
    }

    pub fn insert_group_at(&mut self, target: DropTarget) -> u64 {
        let id = self.next_id();
        let node = FilterNode::Group { id, combinator: Combinator::And, children: Vec::new() };
        self.insert_node_at(node, target);
        id
    }

    pub fn remove_node(&mut self, id: u64) {
        let _ = self.take_node(id);
    }

    pub fn merge_into_group(&mut self, dragged_id: u64, target_id: u64) -> Option<u64> {
        if dragged_id == target_id {
            return None;
        }
        if self.find_node(dragged_id).is_some_and(|node| node.contains_node(target_id)) {
            return None;
        }

        let target_location = self.node_location(target_id)?;
        let dragged_node = self.take_node(dragged_id)?;
        let target_node = self.take_node(target_id)?;

        let group_id = self.next_id();
        let group = FilterNode::Group {
            id: group_id,
            combinator: Combinator::And,
            children: vec![target_node, dragged_node],
        };

        let adjusted_target = DropTarget {
            parent_group_id: target_location.parent_group_id,
            index: target_location
                .index
                .min(self.children_mut(target_location.parent_group_id).map_or(0, |c| c.len())),
        };
        self.insert_node_at(group, adjusted_target);
        Some(group_id)
    }

    pub fn add_node_to_group(&mut self, node_id: u64, group_id: u64) -> bool {
        if node_id == group_id {
            return false;
        }
        if self.find_node(node_id).is_some_and(|node| node.contains_node(group_id)) {
            return false;
        }

        let Some(node) = self.take_node(node_id) else {
            return false;
        };
        if let Some(children) = self.children_mut(Some(group_id)) {
            children.push(node);
            true
        } else {
            let target = self.root_drop_target();
            self.insert_node_at(node, target);
            false
        }
    }

    pub fn remove_condition(&mut self, id: u64) {
        self.remove_node(id);
    }

    pub fn duplicate_node(&mut self, id: u64) -> Option<u64> {
        let source = self.find_node(id)?.clone();
        let location = self.node_location(id)?;
        let duplicated = source.clone_with_new_ids(&mut self.next_id);
        let duplicate_id = duplicated.node_id();
        let target =
            DropTarget { parent_group_id: location.parent_group_id, index: location.index + 1 };
        self.insert_node_at(duplicated, target);
        Some(duplicate_id)
    }

    pub fn move_node(&mut self, node_id: u64, mut target: DropTarget) -> bool {
        if target.parent_group_id == Some(node_id) {
            return false;
        }
        if let Some(parent_group_id) = target.parent_group_id
            && self.find_node(node_id).is_some_and(|node| node.contains_node(parent_group_id))
        {
            return false;
        }

        let source = self.node_location(node_id);
        let Some(source_location) = source else {
            return false;
        };

        if source_location.parent_group_id == target.parent_group_id
            && source_location.index < target.index
        {
            target.index = target.index.saturating_sub(1);
        }

        let Some(node) = self.take_node(node_id) else {
            return false;
        };

        self.insert_node_at(node, target);
        true
    }

    pub fn condition_mut(&mut self, id: u64) -> Option<&mut FilterCondition> {
        self.children.iter_mut().find_map(|child| child.condition_mut(id))
    }

    pub fn group_combinator_mut(&mut self, id: u64) -> Option<&mut Combinator> {
        let node = self.children.iter_mut().find_map(|child| child.group_mut(id))?;
        match node {
            FilterNode::Group { combinator, .. } => Some(combinator),
            FilterNode::Condition(_) => None,
        }
    }

    pub fn find_node(&self, id: u64) -> Option<&FilterNode> {
        find_node_in_slice(&self.children, id)
    }

    pub fn find_group(&self, id: u64) -> Option<&FilterNode> {
        self.find_node(id).filter(|node| node.is_group())
    }

    pub fn conditions(&self) -> Vec<FilterCondition> {
        let mut out = Vec::new();
        for child in &self.children {
            child.collect_conditions(&mut out);
        }
        out
    }

    pub fn flat_conditions(&self) -> Vec<&FilterCondition> {
        self.children
            .iter()
            .filter_map(|node| match node {
                FilterNode::Condition(condition) => Some(condition),
                _ => None,
            })
            .collect()
    }

    pub fn validation_error(&self) -> Option<String> {
        self.conditions().into_iter().find_map(|condition| condition.validation_error())
    }

    pub fn to_document(&self) -> Document {
        let parts = self.children.iter().filter_map(FilterNode::to_document).collect::<Vec<_>>();
        if parts.is_empty() {
            return Document::new();
        }
        combine_documents(self.combinator, parts)
    }

    pub fn to_json_string(&self) -> String {
        let doc = self.to_document();
        if doc.is_empty() {
            return "{}".to_string();
        }
        let value = Bson::Document(doc).into_relaxed_extjson();
        crate::bson::format_relaxed_json_compact(&value)
    }

    pub fn has_active_conditions(&self) -> bool {
        self.children.iter().any(FilterNode::has_active_conditions)
    }

    pub fn active_condition_count(&self) -> usize {
        self.conditions().into_iter().filter(FilterCondition::has_input).count()
    }

    pub fn from_document(doc: &Document) -> Result<Self, UnsupportedFilter> {
        let mut tree = Self::new();
        if doc.is_empty() {
            return Ok(tree);
        }

        let has_and = doc.contains_key("$and");
        let has_or = doc.contains_key("$or");

        if has_or && !has_and && doc.len() == 1 {
            tree.combinator = Combinator::Or;
            let Bson::Array(arr) = doc.get("$or").cloned().unwrap_or(Bson::Array(Vec::new()))
            else {
                return Err(unsupported("`$or` must contain an array of documents"));
            };
            for item in arr {
                let Bson::Document(child_doc) = item else {
                    return Err(unsupported("`$or` children must be documents"));
                };
                tree.children.push(parse_query_node(&child_doc, &mut tree.next_id)?);
            }
            return Ok(tree);
        }

        if has_and && doc.len() == 1 {
            tree.combinator = Combinator::And;
            let Bson::Array(arr) = doc.get("$and").cloned().unwrap_or(Bson::Array(Vec::new()))
            else {
                return Err(unsupported("`$and` must contain an array of documents"));
            };
            for item in arr {
                let Bson::Document(child_doc) = item else {
                    return Err(unsupported("`$and` children must be documents"));
                };
                tree.children.push(parse_query_node(&child_doc, &mut tree.next_id)?);
            }
            return Ok(tree);
        }

        for (key, value) in doc {
            if key == "$and" || key == "$or" {
                let combinator = if key == "$and" { Combinator::And } else { Combinator::Or };
                let Bson::Array(arr) = value else {
                    return Err(unsupported("Logical groups must contain an array of documents"));
                };
                let group_id = tree.next_id();
                let mut children = Vec::new();
                for item in arr {
                    let Bson::Document(child_doc) = item else {
                        return Err(unsupported("Logical groups must contain documents"));
                    };
                    children.push(parse_query_node(child_doc, &mut tree.next_id)?);
                }
                tree.children.push(FilterNode::Group { id: group_id, combinator, children });
                continue;
            }

            if key.starts_with('$') {
                return Err(unsupported(
                    "This filter uses top-level Mongo operators that the visual builder does not support yet",
                ));
            }

            tree.children.push(parse_field_condition(key, value, &mut tree.next_id)?);
        }

        Ok(tree)
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn node_location(&self, id: u64) -> Option<DropTarget> {
        find_node_location(&self.children, id, None)
    }

    fn children_mut(&mut self, parent_group_id: Option<u64>) -> Option<&mut Vec<FilterNode>> {
        match parent_group_id {
            None => Some(&mut self.children),
            Some(group_id) => {
                match self.children.iter_mut().find_map(|child| child.group_mut(group_id))? {
                    FilterNode::Group { children, .. } => Some(children),
                    FilterNode::Condition(_) => None,
                }
            }
        }
    }

    fn insert_node_at(&mut self, node: FilterNode, target: DropTarget) {
        if let Some(children) = self.children_mut(target.parent_group_id) {
            let index = target.index.min(children.len());
            children.insert(index, node);
        }
    }

    fn take_node(&mut self, id: u64) -> Option<FilterNode> {
        take_node_from_slice(&mut self.children, id)
    }
}

fn find_node_in_slice(nodes: &[FilterNode], id: u64) -> Option<&FilterNode> {
    for node in nodes {
        if node.node_id() == id {
            return Some(node);
        }
        if let FilterNode::Group { children, .. } = node
            && let Some(found) = find_node_in_slice(children, id)
        {
            return Some(found);
        }
    }
    None
}

fn find_node_location(
    nodes: &[FilterNode],
    id: u64,
    parent_group_id: Option<u64>,
) -> Option<DropTarget> {
    for (index, node) in nodes.iter().enumerate() {
        if node.node_id() == id {
            return Some(DropTarget { parent_group_id, index });
        }
        if let FilterNode::Group { id: group_id, children, .. } = node
            && let Some(location) = find_node_location(children, id, Some(*group_id))
        {
            return Some(location);
        }
    }
    None
}

fn take_node_from_slice(nodes: &mut Vec<FilterNode>, id: u64) -> Option<FilterNode> {
    let position = nodes.iter().position(|node| node.node_id() == id);
    if let Some(position) = position {
        return Some(nodes.remove(position));
    }

    for node in nodes {
        if let FilterNode::Group { children, .. } = node
            && let Some(found) = take_node_from_slice(children, id)
        {
            return Some(found);
        }
    }

    None
}

fn parse_query_node(doc: &Document, next_id: &mut u64) -> Result<FilterNode, UnsupportedFilter> {
    if doc.len() == 1 {
        if let Some(Bson::Array(arr)) = doc.get("$and") {
            let group_id = *next_id;
            *next_id += 1;
            let mut children = Vec::new();
            for item in arr {
                let Bson::Document(child_doc) = item else {
                    return Err(unsupported("`$and` groups must contain only documents"));
                };
                children.push(parse_query_node(child_doc, next_id)?);
            }
            return Ok(FilterNode::Group { id: group_id, combinator: Combinator::And, children });
        }
        if let Some(Bson::Array(arr)) = doc.get("$or") {
            let group_id = *next_id;
            *next_id += 1;
            let mut children = Vec::new();
            for item in arr {
                let Bson::Document(child_doc) = item else {
                    return Err(unsupported("`$or` groups must contain only documents"));
                };
                children.push(parse_query_node(child_doc, next_id)?);
            }
            return Ok(FilterNode::Group { id: group_id, combinator: Combinator::Or, children });
        }
    }

    if doc.keys().any(|key| key.starts_with('$')) {
        return Err(unsupported(
            "This filter uses Mongo operators the visual builder does not support yet",
        ));
    }

    if doc.len() == 1 {
        let (field, value) = doc.iter().next().expect("document has one entry");
        return parse_field_condition(field, value, next_id);
    }

    let group_id = *next_id;
    *next_id += 1;
    let mut children = Vec::new();
    for (field, value) in doc {
        children.push(parse_field_condition(field, value, next_id)?);
    }
    Ok(FilterNode::Group { id: group_id, combinator: Combinator::And, children })
}

fn parse_field_condition(
    field: &str,
    value: &Bson,
    next_id: &mut u64,
) -> Result<FilterNode, UnsupportedFilter> {
    let id = *next_id;
    *next_id += 1;

    if let Bson::Document(inner) = value {
        if let Some(condition) = parse_special_condition(id, field, inner)? {
            return Ok(FilterNode::Condition(condition));
        }

        if inner.len() != 1 {
            return Err(unsupported(
                "A field has multiple Mongo operators. Edit it in raw JSON for now.",
            ));
        }

        let (op_key, op_value) = inner.iter().next().expect("document has one operator");
        let Some(operator) = operator_from_key(op_key) else {
            return Err(unsupported(
                "This field uses a Mongo operator the visual builder does not support yet",
            ));
        };

        let mut condition = FilterCondition::new(id);
        condition.field = field.to_string();
        condition.operator = operator;
        condition.field_type = infer_type_from_value(field, op_value, operator);
        condition.value = parse_value_for(condition.field_type, operator, op_value)?;
        return Ok(FilterNode::Condition(condition));
    }

    let mut condition = FilterCondition::new(id);
    condition.field = field.to_string();
    condition.field_type = match value {
        Bson::Document(_) => FieldType::Document,
        _ => {
            let inferred = FieldType::from_bson(value);
            if inferred == FieldType::Unknown {
                FieldType::from_field_name(field)
            } else {
                inferred
            }
        }
    };

    condition.operator = FilterOperator::Eq;
    condition.value = parse_value_for(condition.field_type, FilterOperator::Eq, value)?;
    Ok(FilterNode::Condition(condition))
}

fn parse_special_condition(
    id: u64,
    field: &str,
    inner: &Document,
) -> Result<Option<FilterCondition>, UnsupportedFilter> {
    if inner.len() == 1
        && let Some(Bson::String(pattern)) = inner.get("$regex")
    {
        let mut condition = FilterCondition::new(id);
        condition.field = field.to_string();
        condition.field_type = FieldType::String;
        condition.operator = FilterOperator::Regex;
        condition.value = ConditionValue::Scalar(pattern.clone());
        return Ok(Some(condition));
    }

    if inner.len() == 1
        && let Some(Bson::RegularExpression(regex)) = inner.get("$regex")
    {
        if !regex.options.is_empty() {
            return Err(unsupported("Regex options are only supported in raw JSON for now"));
        }
        let mut condition = FilterCondition::new(id);
        condition.field = field.to_string();
        condition.field_type = FieldType::String;
        condition.operator = FilterOperator::Regex;
        condition.value = ConditionValue::Scalar(regex.pattern.clone());
        return Ok(Some(condition));
    }

    Ok(None)
}

fn parse_value_for(
    field_type: FieldType,
    operator: FilterOperator,
    value: &Bson,
) -> Result<ConditionValue, UnsupportedFilter> {
    match operator.value_editor_kind(field_type) {
        ValueEditorKind::None => Ok(ConditionValue::Empty),
        ValueEditorKind::Toggle => match value {
            Bson::Boolean(value) => Ok(ConditionValue::Bool(*value)),
            Bson::Int32(value) if matches!(operator, FilterOperator::Exists) => {
                Ok(ConditionValue::Bool(*value != 0))
            }
            Bson::Int64(value) if matches!(operator, FilterOperator::Exists) => {
                Ok(ConditionValue::Bool(*value != 0))
            }
            other => Err(unsupported(format!("Expected a boolean value, got `{other}`"))),
        },
        ValueEditorKind::Single => Ok(ConditionValue::Scalar(bson_value_to_input(value))),
        ValueEditorKind::List => {
            let Bson::Array(values) = value else {
                return Err(unsupported("Expected an array of values"));
            };
            Ok(ConditionValue::List(values.iter().map(bson_value_to_input).collect()))
        }
        ValueEditorKind::Range => Err(unsupported("Ranges are parsed separately")),
    }
}

pub fn drag_value_for(
    field_type: FieldType,
    operator: FilterOperator,
    value: &Bson,
) -> ConditionValue {
    match operator.value_editor_kind(field_type) {
        ValueEditorKind::None => ConditionValue::Empty,
        ValueEditorKind::Toggle => match value {
            Bson::Boolean(value) => ConditionValue::Bool(*value),
            Bson::Int32(value) => ConditionValue::Bool(*value != 0),
            Bson::Int64(value) => ConditionValue::Bool(*value != 0),
            Bson::String(value) => ConditionValue::Bool(matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "true" | "1" | "yes"
            )),
            _ => ConditionValue::Bool(true),
        },
        ValueEditorKind::Single => ConditionValue::Scalar(bson_value_to_input(value)),
        ValueEditorKind::List => match value {
            Bson::Array(values) => {
                ConditionValue::List(values.iter().map(bson_value_to_input).collect())
            }
            _ => ConditionValue::List(vec![bson_value_to_input(value)]),
        },
        ValueEditorKind::Range => {
            ConditionValue::Range { start: bson_value_to_input(value), end: String::new() }
        }
    }
}

pub fn is_valid_value_for_field_type(field_type: FieldType, raw: &str) -> bool {
    let condition = FilterCondition {
        id: 0,
        field: String::new(),
        field_type,
        operator: FilterOperator::Eq,
        value: ConditionValue::Scalar(raw.to_string()),
    };
    condition.parse_scalar(raw.trim()).is_some()
}

fn operator_from_key(key: &str) -> Option<FilterOperator> {
    match key {
        "$eq" => Some(FilterOperator::Eq),
        "$ne" => Some(FilterOperator::Ne),
        "$gt" => Some(FilterOperator::Gt),
        "$gte" => Some(FilterOperator::Gte),
        "$lt" => Some(FilterOperator::Lt),
        "$lte" => Some(FilterOperator::Lte),
        "$in" => Some(FilterOperator::In),
        "$nin" => Some(FilterOperator::Nin),
        "$all" => Some(FilterOperator::All),
        "$exists" => Some(FilterOperator::Exists),
        "$regex" => Some(FilterOperator::Regex),
        "$size" => Some(FilterOperator::Size),
        _ => None,
    }
}

fn infer_type_from_value(field: &str, value: &Bson, operator: FilterOperator) -> FieldType {
    match operator {
        FilterOperator::Contains
        | FilterOperator::StartsWith
        | FilterOperator::EndsWith
        | FilterOperator::Regex => FieldType::String,
        FilterOperator::Between => FieldType::from_field_name(field),
        FilterOperator::In | FilterOperator::Nin | FilterOperator::All => {
            if let Bson::Array(arr) = value
                && let Some(first) = arr.first()
            {
                return FieldType::from_bson(first);
            }
            FieldType::from_field_name(field)
        }
        FilterOperator::Size | FilterOperator::IsEmpty | FilterOperator::NotEmpty => {
            if matches!(value, Bson::Array(_)) {
                FieldType::Array
            } else {
                let from_value = FieldType::from_bson(value);
                if from_value == FieldType::Unknown {
                    FieldType::from_field_name(field)
                } else {
                    from_value
                }
            }
        }
        _ => {
            let from_value = FieldType::from_bson(value);
            if from_value == FieldType::Unknown {
                FieldType::from_field_name(field)
            } else {
                from_value
            }
        }
    }
}

fn parse_datetime(raw: &str) -> Option<Bson> {
    let cleaned = raw
        .trim_start_matches("ISODate(\"")
        .trim_start_matches("Date(\"")
        .trim_end_matches("\")")
        .trim_matches('"');

    chrono::DateTime::parse_from_rfc3339(cleaned)
        .ok()
        .map(|datetime| {
            Bson::DateTime(mongodb::bson::DateTime::from_millis(datetime.timestamp_millis()))
        })
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(cleaned, "%Y-%m-%d").ok().map(|date| {
                let datetime = date.and_hms_opt(0, 0, 0).unwrap().and_utc();
                Bson::DateTime(mongodb::bson::DateTime::from_millis(datetime.timestamp_millis()))
            })
        })
}

fn bson_value_to_input(value: &Bson) -> String {
    match value {
        Bson::String(value) => value.clone(),
        Bson::Int32(value) => value.to_string(),
        Bson::Int64(value) => value.to_string(),
        Bson::Double(value) => value.to_string(),
        Bson::Boolean(value) => value.to_string(),
        Bson::ObjectId(value) => value.to_hex(),
        Bson::DateTime(value) => {
            let millis = value.timestamp_millis();
            chrono::DateTime::from_timestamp_millis(millis)
                .map(|datetime| datetime.to_rfc3339())
                .unwrap_or_else(|| millis.to_string())
        }
        Bson::RegularExpression(value) => value.pattern.clone(),
        Bson::Null => "null".to_string(),
        other => crate::bson::format_relaxed_json_value(&other.clone().into_relaxed_extjson()),
    }
}

fn regex_document(field: &str, pattern: String) -> Option<Document> {
    if pattern.is_empty() {
        return None;
    }
    let mut inner = Document::new();
    inner.insert("$regex", Bson::String(pattern));
    let mut doc = Document::new();
    doc.insert(field.to_string(), inner);
    Some(doc)
}

fn combine_documents(combinator: Combinator, parts: Vec<Document>) -> Document {
    if parts.len() == 1 && combinator == Combinator::And {
        return parts.into_iter().next().expect("one part");
    }

    match combinator {
        Combinator::And => {
            let mut merged = Document::new();
            let mut has_conflict = false;

            for part in &parts {
                for (key, _) in part {
                    if merged.contains_key(key) {
                        has_conflict = true;
                        break;
                    }
                }
                if has_conflict {
                    break;
                }
                for (key, value) in part {
                    merged.insert(key.clone(), value.clone());
                }
            }

            if has_conflict {
                let mut doc = Document::new();
                doc.insert("$and", Bson::Array(parts.into_iter().map(Bson::Document).collect()));
                doc
            } else {
                merged
            }
        }
        Combinator::Or => {
            let mut doc = Document::new();
            doc.insert("$or", Bson::Array(parts.into_iter().map(Bson::Document).collect()));
            doc
        }
    }
}

fn default_value(field_type: FieldType, operator: FilterOperator) -> ConditionValue {
    match operator.value_editor_kind(field_type) {
        ValueEditorKind::None => ConditionValue::Empty,
        ValueEditorKind::Single => ConditionValue::Scalar(String::new()),
        ValueEditorKind::List => ConditionValue::List(Vec::new()),
        ValueEditorKind::Toggle => {
            let default =
                matches!(operator, FilterOperator::Exists) || field_type == FieldType::Boolean;
            ConditionValue::Bool(default)
        }
        ValueEditorKind::Range => {
            ConditionValue::Range { start: String::new(), end: String::new() }
        }
    }
}

fn coerce_value_for(
    previous: ConditionValue,
    field_type: FieldType,
    operator: FilterOperator,
) -> ConditionValue {
    match operator.value_editor_kind(field_type) {
        ValueEditorKind::None => ConditionValue::Empty,
        ValueEditorKind::Single => match previous {
            ConditionValue::Scalar(value) => ConditionValue::Scalar(value),
            ConditionValue::Bool(value) => ConditionValue::Scalar(value.to_string()),
            ConditionValue::List(values) => ConditionValue::Scalar(
                values.into_iter().find(|value| !value.trim().is_empty()).unwrap_or_default(),
            ),
            ConditionValue::Range { start, .. } => ConditionValue::Scalar(start),
            ConditionValue::Empty => default_value(field_type, operator),
        },
        ValueEditorKind::List => match previous {
            ConditionValue::List(values) => ConditionValue::List(values),
            ConditionValue::Scalar(value) if !value.trim().is_empty() => {
                ConditionValue::List(vec![value])
            }
            ConditionValue::Range { start, end } => {
                let mut values = Vec::new();
                if !start.trim().is_empty() {
                    values.push(start);
                }
                if !end.trim().is_empty() {
                    values.push(end);
                }
                ConditionValue::List(values)
            }
            _ => default_value(field_type, operator),
        },
        ValueEditorKind::Toggle => match previous {
            ConditionValue::Bool(value) => ConditionValue::Bool(value),
            ConditionValue::Scalar(value) => {
                let normalized = value.trim().to_ascii_lowercase();
                ConditionValue::Bool(matches!(normalized.as_str(), "true" | "1" | "yes"))
            }
            _ => default_value(field_type, operator),
        },
        ValueEditorKind::Range => match previous {
            ConditionValue::Range { start, end } => ConditionValue::Range { start, end },
            ConditionValue::Scalar(value) if !value.trim().is_empty() => {
                ConditionValue::Range { start: value, end: String::new() }
            }
            ConditionValue::List(values) => {
                let mut values = values.into_iter();
                ConditionValue::Range {
                    start: values.next().unwrap_or_default(),
                    end: values.next().unwrap_or_default(),
                }
            }
            _ => default_value(field_type, operator),
        },
    }
}

fn value_error_message(field_type: FieldType) -> &'static str {
    match field_type {
        FieldType::ObjectId => "Enter a valid ObjectId",
        FieldType::Number => "Enter a valid number",
        FieldType::Boolean => "Choose true or false",
        FieldType::DateTime => "Enter a valid date or ISO timestamp",
        _ => "Enter a valid value",
    }
}

fn unsupported(reason: impl Into<String>) -> UnsupportedFilter {
    UnsupportedFilter { reason: reason.into() }
}

#[cfg(test)]
mod tests {
    use super::{
        Combinator, ConditionValue, DropTarget, FieldType, FilterCondition, FilterNode,
        FilterOperator, FilterTree,
    };
    use mongodb::bson::{Bson, doc};

    #[test]
    fn string_contains_serializes_to_regex() {
        let mut condition = FilterCondition::new(1);
        condition.field = "title".to_string();
        condition.field_type = FieldType::String;
        condition.set_operator(FilterOperator::Contains);
        condition.value = ConditionValue::Scalar("mango".to_string());

        let doc = condition.to_document().expect("contains doc");
        assert_eq!(doc.get_document("title").unwrap().get_str("$regex").unwrap(), "mango");
    }

    #[test]
    fn range_filter_is_unsupported_in_visual_builder() {
        let result = FilterTree::from_document(&doc! {
            "createdAt": {
                "$gte": Bson::DateTime(mongodb::bson::DateTime::from_millis(1_700_000_000_000)),
                "$lte": Bson::DateTime(mongodb::bson::DateTime::from_millis(1_700_100_000_000))
            }
        });
        assert!(result.is_err());
    }

    #[test]
    fn regex_round_trips_as_raw_regex_operator() {
        let tree = FilterTree::from_document(&doc! {
            "title": { "$regex": "^mango" }
        })
        .expect("supported");

        let condition = tree.conditions().into_iter().next().expect("condition");
        assert_eq!(condition.operator, FilterOperator::Regex);
        assert_eq!(condition.scalar_display_value(), "^mango");
    }

    #[test]
    fn unsupported_multi_operator_field_returns_error() {
        let result = FilterTree::from_document(&doc! {
            "title": { "$regex": "mango", "$options": "i" }
        });
        assert!(result.is_err());
    }

    #[test]
    fn duplicate_and_move_node_work_across_groups() {
        let mut tree = FilterTree::new();
        let first = tree.add_condition();
        let group = tree.add_group();
        let second = tree.add_condition_to_group(group).expect("group condition");
        let duplicate = tree.duplicate_node(first).expect("duplicate");

        assert_ne!(duplicate, first);
        assert!(tree.move_node(duplicate, DropTarget { parent_group_id: Some(group), index: 1 }));

        let FilterNode::Group { children, .. } = tree.find_group(group).expect("group") else {
            panic!("group node");
        };
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].node_id(), second);
        assert_eq!(children[1].node_id(), duplicate);
    }

    #[test]
    fn combine_conflicting_and_uses_and_wrapper() {
        let mut tree = FilterTree::new();
        tree.children = vec![
            FilterNode::Condition(FilterCondition {
                id: 1,
                field: "status".to_string(),
                field_type: FieldType::String,
                operator: FilterOperator::Eq,
                value: ConditionValue::Scalar("open".to_string()),
            }),
            FilterNode::Condition(FilterCondition {
                id: 2,
                field: "status".to_string(),
                field_type: FieldType::String,
                operator: FilterOperator::Ne,
                value: ConditionValue::Scalar("closed".to_string()),
            }),
        ];

        let doc = tree.to_document();
        assert!(matches!(doc.get("$and"), Some(Bson::Array(_))));
        assert_eq!(tree.combinator, Combinator::And);
    }
}
