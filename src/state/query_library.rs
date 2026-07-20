use chrono::{DateTime, Utc};
use mongodb::bson::Document;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::app_state::PipelineStage;

pub const QUERY_HISTORY_LIMIT: usize = 200;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct QueryLibraryPersistenceError(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryKind {
    Documents,
    Aggregation,
    Forge,
}

impl QueryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Documents => "Documents",
            Self::Aggregation => "Aggregation",
            Self::Forge => "Forge",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentQuery {
    pub filter_raw: String,
    pub filter: Option<Document>,
    pub sort_raw: String,
    pub sort: Option<Document>,
    pub projection_raw: String,
    pub projection: Option<Document>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "query", rename_all = "snake_case")]
pub enum QueryContent {
    Documents(Box<DocumentQuery>),
    Aggregation {
        stages: Vec<PipelineStage>,
        #[serde(default)]
        selected_stage: Option<usize>,
    },
    Forge {
        statement: String,
    },
}

impl QueryContent {
    pub fn kind(&self) -> QueryKind {
        match self {
            Self::Documents(_) => QueryKind::Documents,
            Self::Aggregation { .. } => QueryKind::Aggregation,
            Self::Forge { .. } => QueryKind::Forge,
        }
    }

    pub fn preview(&self) -> String {
        let raw = match self {
            Self::Documents(query) => {
                let mut parts = Vec::new();
                if !is_empty_document(&query.filter_raw) {
                    parts.push(format!("find {}", query.filter_raw));
                }
                if !query.sort_raw.trim().is_empty() {
                    parts.push(format!("sort {}", query.sort_raw));
                }
                if !query.projection_raw.trim().is_empty() {
                    parts.push(format!("project {}", query.projection_raw));
                }
                parts.join(" · ")
            }
            Self::Aggregation { stages, .. } => stages
                .iter()
                .filter(|stage| stage.enabled)
                .map(|stage| format!("{} {}", stage.operator, stage.body.trim()))
                .collect::<Vec<_>>()
                .join(" · "),
            Self::Forge { statement } => statement.trim().to_string(),
        };
        compact_preview(&raw, 220)
    }

    pub fn copy_text(&self) -> String {
        match self {
            Self::Documents(query) => {
                let filter =
                    if query.filter_raw.trim().is_empty() { "{}" } else { query.filter_raw.trim() };
                let mut text = format!("Filter: {filter}");
                if !query.sort_raw.trim().is_empty() {
                    text.push_str(&format!("\nSort: {}", query.sort_raw.trim()));
                }
                if !query.projection_raw.trim().is_empty() {
                    text.push_str(&format!("\nProjection: {}", query.projection_raw.trim()));
                }
                text
            }
            Self::Aggregation { stages, selected_stage } => {
                let end = selected_stage
                    .unwrap_or_else(|| stages.len().saturating_sub(1))
                    .min(stages.len().saturating_sub(1));
                let rendered = stages
                    .iter()
                    .take(end.saturating_add(1))
                    .filter(|stage| stage.enabled)
                    .map(|stage| format!("  {{ {}: {} }}", stage.operator, stage.body.trim()))
                    .collect::<Vec<_>>()
                    .join(",\n");
                format!("[\n{rendered}\n]")
            }
            Self::Forge { statement } => statement.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Documents(query) => {
                is_empty_document(&query.filter_raw)
                    && query.sort_raw.trim().is_empty()
                    && query.projection_raw.trim().is_empty()
            }
            Self::Aggregation { stages, .. } => stages.is_empty(),
            Self::Forge { statement } => statement.trim().is_empty(),
        }
    }

    fn may_contain_credentials(&self) -> bool {
        match self {
            Self::Documents(query) => {
                [&query.filter_raw, &query.sort_raw, &query.projection_raw]
                    .into_iter()
                    .any(|raw| may_contain_credentials(raw))
                    || [&query.filter, &query.sort, &query.projection]
                        .into_iter()
                        .flatten()
                        .filter_map(|document| serde_json::to_string(document).ok())
                        .any(|raw| may_contain_credentials(&raw))
            }
            Self::Aggregation { stages, .. } => {
                stages.iter().any(|stage| may_contain_credentials(&stage.body))
            }
            Self::Forge { statement } => may_contain_credentials(statement),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryDefinition {
    pub connection_id: Uuid,
    pub database: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    pub content: QueryContent,
}

impl QueryDefinition {
    pub fn kind(&self) -> QueryKind {
        self.content.kind()
    }

    pub fn namespace(&self) -> String {
        match &self.collection {
            Some(collection) => format!("{}.{}", self.database, collection),
            None => self.database.clone(),
        }
    }

    pub fn matches_scope(
        &self,
        kind: QueryKind,
        connection_id: Uuid,
        database: &str,
        collection: Option<&str>,
    ) -> bool {
        self.kind() == kind
            && self.connection_id == connection_id
            && self.database == database
            && self.collection.as_deref() == collection
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryHistoryEntry {
    pub id: Uuid,
    pub executed_at: DateTime<Utc>,
    pub definition: QueryDefinition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedQuery {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub definition: QueryDefinition,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryLibrary {
    #[serde(default)]
    history: Vec<QueryHistoryEntry>,
    #[serde(default)]
    saved: Vec<SavedQuery>,
}

impl QueryLibrary {
    pub fn history(&self) -> &[QueryHistoryEntry] {
        &self.history
    }

    pub fn saved(&self) -> &[SavedQuery] {
        &self.saved
    }

    pub fn record(&mut self, definition: QueryDefinition) -> bool {
        if definition.content.is_empty() || definition.content.may_contain_credentials() {
            return false;
        }
        let now = Utc::now();
        if let Some(latest) = self.history.first_mut()
            && latest.definition == definition
        {
            latest.executed_at = now;
            return true;
        }
        self.history
            .insert(0, QueryHistoryEntry { id: Uuid::new_v4(), executed_at: now, definition });
        self.history.truncate(QUERY_HISTORY_LIMIT);
        true
    }

    pub fn delete_history(&mut self, id: Uuid) -> bool {
        let len = self.history.len();
        self.history.retain(|entry| entry.id != id);
        self.history.len() != len
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    pub fn save_history(&mut self, history_id: Uuid, name: &str) -> Result<Uuid, String> {
        let definition = self
            .history
            .iter()
            .find(|entry| entry.id == history_id)
            .map(|entry| entry.definition.clone())
            .ok_or_else(|| "That history entry no longer exists.".to_string())?;
        self.save(definition, name)
    }

    pub fn save(&mut self, definition: QueryDefinition, name: &str) -> Result<Uuid, String> {
        validate_definition(&definition)?;
        let name = validate_name(name)?;
        if self.saved.iter().any(|query| query.name.eq_ignore_ascii_case(name)) {
            return Err(format!("A saved query named \"{name}\" already exists."));
        }
        let id = Uuid::new_v4();
        let now = Utc::now();
        self.saved.push(SavedQuery {
            id,
            name: name.to_string(),
            created_at: now,
            updated_at: now,
            definition,
        });
        self.sort_saved();
        Ok(id)
    }

    pub fn update_saved(&mut self, id: Uuid, definition: QueryDefinition) -> Result<(), String> {
        validate_definition(&definition)?;
        let query = self
            .saved
            .iter_mut()
            .find(|query| query.id == id)
            .ok_or_else(|| "That saved query no longer exists.".to_string())?;
        query.definition = definition;
        query.updated_at = Utc::now();
        Ok(())
    }

    pub fn rename_saved(&mut self, id: Uuid, name: &str) -> Result<(), String> {
        let name = validate_name(name)?;
        if self.saved.iter().any(|query| query.id != id && query.name.eq_ignore_ascii_case(name)) {
            return Err(format!("A saved query named \"{name}\" already exists."));
        }
        let query = self
            .saved
            .iter_mut()
            .find(|query| query.id == id)
            .ok_or_else(|| "That saved query no longer exists.".to_string())?;
        query.name = name.to_string();
        query.updated_at = Utc::now();
        self.sort_saved();
        Ok(())
    }

    pub fn duplicate_saved(&mut self, id: Uuid) -> Result<Uuid, String> {
        let source = self
            .saved
            .iter()
            .find(|query| query.id == id)
            .cloned()
            .ok_or_else(|| "That saved query no longer exists.".to_string())?;
        let mut suffix = " Copy".to_string();
        let mut name = copy_name(&source.name, &suffix);
        let mut number = 2;
        while self.saved.iter().any(|query| query.name.eq_ignore_ascii_case(&name)) {
            suffix = format!(" Copy {number}");
            name = copy_name(&source.name, &suffix);
            number += 1;
        }
        let id = Uuid::new_v4();
        let now = Utc::now();
        self.saved.push(SavedQuery {
            id,
            name,
            created_at: now,
            updated_at: now,
            definition: source.definition,
        });
        self.sort_saved();
        Ok(id)
    }

    pub fn delete_saved(&mut self, id: Uuid) -> bool {
        let len = self.saved.len();
        self.saved.retain(|query| query.id != id);
        self.saved.len() != len
    }

    pub fn history_entry(&self, id: Uuid) -> Option<&QueryHistoryEntry> {
        self.history.iter().find(|entry| entry.id == id)
    }

    pub fn saved_query(&self, id: Uuid) -> Option<&SavedQuery> {
        self.saved.iter().find(|query| query.id == id)
    }

    fn sort_saved(&mut self) {
        self.saved.sort_by_key(|query| query.name.to_lowercase());
    }
}

fn validate_definition(definition: &QueryDefinition) -> Result<(), String> {
    if definition.content.is_empty() {
        Err("Enter a query before saving it.".to_string())
    } else if definition.content.may_contain_credentials() {
        Err("Queries that may contain credentials cannot be saved.".to_string())
    } else {
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        Err("Enter a name for this query.".to_string())
    } else if name.chars().count() > 80 {
        Err("Query names must be 80 characters or fewer.".to_string())
    } else {
        Ok(name)
    }
}

fn is_empty_document(raw: &str) -> bool {
    matches!(raw.trim(), "" | "{}" | "{ }")
}

fn compact_preview(raw: &str, max_chars: usize) -> String {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut preview = compact.chars().take(max_chars.saturating_sub(1)).collect::<String>();
    preview.push('…');
    preview
}

fn copy_name(source: &str, suffix: &str) -> String {
    let max_source_chars = 80usize.saturating_sub(suffix.chars().count());
    let source = source.chars().take(max_source_chars).collect::<String>();
    format!("{source}{suffix}")
}

fn may_contain_credentials(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    [
        "mongodb://",
        "mongodb+srv://",
        "db.auth",
        "createuser",
        "updateuser",
        "changeuserpassword",
        "password",
        "passwd",
        "passphrase",
        "pwd",
        "secret",
        "credential",
        "api_key",
        "apikey",
        "access_key",
        "accesskey",
        "access_token",
        "auth_token",
        "token",
        "authorization",
        "bearer ",
        "client_secret",
        "private_key",
        "privatekey",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forge_definition(statement: &str) -> QueryDefinition {
        QueryDefinition {
            connection_id: Uuid::nil(),
            database: "app".into(),
            collection: None,
            content: QueryContent::Forge { statement: statement.into() },
        }
    }

    #[test]
    fn history_deduplicates_consecutive_queries_and_is_bounded() {
        let mut library = QueryLibrary::default();
        assert!(library.record(forge_definition("db.users.find({ active: true })")));
        let first_id = library.history()[0].id;
        assert!(library.record(forge_definition("db.users.find({ active: true })")));
        assert_eq!(library.history().len(), 1);
        assert_eq!(library.history()[0].id, first_id);

        for index in 0..QUERY_HISTORY_LIMIT + 10 {
            assert!(library.record(forge_definition(&format!("db.c.find({{ index: {index} }})"))));
        }
        assert_eq!(library.history().len(), QUERY_HISTORY_LIMIT);
    }

    #[test]
    fn sensitive_or_empty_queries_are_not_recorded() {
        let mut library = QueryLibrary::default();
        assert!(!library.record(forge_definition("")));
        for statement in [
            "connect('mongodb://user:password@example.com')",
            "db.auth('user', 'secret-value')",
            "db.createUser({ user: 'admin', pwd : 'secret-value' })",
            "db.tokens.find({ bearer: 'secret-value' })",
            "const privateKey = 'secret-value'",
        ] {
            assert!(!library.record(forge_definition(statement)), "recorded {statement}");
        }
        assert!(library.history().is_empty());
        assert!(
            library
                .save(forge_definition("db.auth('user', 'secret-value')"), "Credentials")
                .is_err()
        );

        let stale_parsed_secret = QueryDefinition {
            connection_id: Uuid::nil(),
            database: "app".into(),
            collection: Some("users".into()),
            content: QueryContent::Documents(Box::new(DocumentQuery {
                filter_raw: "{ active: true }".into(),
                filter: Some(mongodb::bson::doc! { "password": "secret-value" }),
                sort_raw: String::new(),
                sort: None,
                projection_raw: String::new(),
                projection: None,
            })),
        };
        assert!(!library.record(stale_parsed_secret.clone()));
        assert!(library.save(stale_parsed_secret, "Credentials").is_err());
    }

    #[test]
    fn history_can_be_saved_renamed_and_duplicated() {
        let mut library = QueryLibrary::default();
        library.record(forge_definition("db.users.find({})"));
        let history_id = library.history()[0].id;
        let saved_id = library.save_history(history_id, "Active users").unwrap();
        library.rename_saved(saved_id, "Users").unwrap();
        let copy_id = library.duplicate_saved(saved_id).unwrap();
        let updated = forge_definition("db.accounts.find({})");
        library.update_saved(saved_id, updated.clone()).unwrap();

        assert_eq!(library.saved_query(saved_id).unwrap().name, "Users");
        assert_eq!(library.saved_query(saved_id).unwrap().definition, updated);
        assert_eq!(library.saved_query(copy_id).unwrap().name, "Users Copy");
        assert!(library.save_history(history_id, "users").is_err());
    }

    #[test]
    fn duplicated_names_stay_within_the_name_limit() {
        let mut library = QueryLibrary::default();
        library.record(forge_definition("db.users.find({})"));
        let history_id = library.history()[0].id;
        let saved_id = library.save_history(history_id, &"a".repeat(80)).unwrap();

        let copy_id = library.duplicate_saved(saved_id).unwrap();

        assert_eq!(library.saved_query(copy_id).unwrap().name.chars().count(), 80);
        assert!(library.saved_query(copy_id).unwrap().name.ends_with(" Copy"));
    }
}
