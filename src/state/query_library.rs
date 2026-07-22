use chrono::{DateTime, Utc};
use mongodb::bson::Document;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::app_state::PipelineStage;

pub const QUERY_HISTORY_LIMIT: usize = 200;
pub const QUERY_NAME_LIMIT: usize = 80;
pub const QUERY_DESCRIPTION_LIMIT: usize = 500;
pub const QUERY_TAG_LIMIT: usize = 20;
pub const QUERY_TAG_LENGTH_LIMIT: usize = 30;

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedQueryScope {
    Global,
    #[default]
    Connection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavedQueryInput {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub scope: SavedQueryScope,
    pub definition: QueryDefinition,
}

impl SavedQueryInput {
    pub fn normalized(mut self) -> Result<Self, String> {
        self.name = validate_name(&self.name)?.to_string();
        self.description = self.description.trim().to_string();
        if self.description.chars().count() > QUERY_DESCRIPTION_LIMIT {
            return Err(format!(
                "Query descriptions must be {QUERY_DESCRIPTION_LIMIT} characters or fewer."
            ));
        }
        self.tags = normalize_tags(self.tags)?;
        validate_metadata(&self.name, &self.description, &self.tags)?;
        validate_definition(&self.definition)?;
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedQuery {
    pub id: Uuid,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub scope: SavedQueryScope,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub definition: QueryDefinition,
}

impl SavedQuery {
    pub fn matches_scope(
        &self,
        kind: QueryKind,
        connection_id: Uuid,
        database: &str,
        collection: Option<&str>,
    ) -> bool {
        if self.definition.kind() != kind {
            return false;
        }
        self.scope == SavedQueryScope::Global
            || self.definition.matches_scope(kind, connection_id, database, collection)
    }

    pub fn matches_search(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        query.is_empty()
            || self.name.to_ascii_lowercase().contains(&query)
            || self.description.to_ascii_lowercase().contains(&query)
            || self.tags.iter().any(|tag| tag.to_ascii_lowercase().contains(&query))
            || self.definition.kind().label().to_ascii_lowercase().contains(&query)
            || self.definition.namespace().to_ascii_lowercase().contains(&query)
            || self.definition.content.copy_text().to_ascii_lowercase().contains(&query)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueryImportReport {
    pub imported: usize,
    pub renamed: usize,
    pub global: usize,
    pub connection: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
        self.save_input(SavedQueryInput {
            name: name.to_string(),
            description: String::new(),
            tags: Vec::new(),
            scope: SavedQueryScope::Connection,
            definition,
        })
    }

    pub fn save_input(&mut self, input: SavedQueryInput) -> Result<Uuid, String> {
        let input = input.normalized()?;
        if self.saved.iter().any(|query| query.name.eq_ignore_ascii_case(&input.name)) {
            return Err(format!("A saved query named \"{}\" already exists.", input.name));
        }
        let id = Uuid::new_v4();
        let now = Utc::now();
        self.saved.push(SavedQuery {
            id,
            name: input.name,
            description: input.description,
            tags: input.tags,
            scope: input.scope,
            created_at: now,
            updated_at: now,
            definition: input.definition,
        });
        self.sort_saved();
        Ok(id)
    }

    pub fn update_saved(&mut self, id: Uuid, definition: QueryDefinition) -> Result<(), String> {
        let source = self
            .saved_query(id)
            .cloned()
            .ok_or_else(|| "That saved query no longer exists.".to_string())?;
        self.edit_saved(
            id,
            SavedQueryInput {
                name: source.name,
                description: source.description,
                tags: source.tags,
                scope: source.scope,
                definition,
            },
        )
    }

    pub fn edit_saved(&mut self, id: Uuid, input: SavedQueryInput) -> Result<(), String> {
        let input = input.normalized()?;
        if self
            .saved
            .iter()
            .any(|query| query.id != id && query.name.eq_ignore_ascii_case(&input.name))
        {
            return Err(format!("A saved query named \"{}\" already exists.", input.name));
        }
        let query = self
            .saved
            .iter_mut()
            .find(|query| query.id == id)
            .ok_or_else(|| "That saved query no longer exists.".to_string())?;
        query.name = input.name;
        query.description = input.description;
        query.tags = input.tags;
        query.scope = input.scope;
        query.definition = input.definition;
        query.updated_at = Utc::now();
        self.sort_saved();
        Ok(())
    }

    pub fn rename_saved(&mut self, id: Uuid, name: &str) -> Result<(), String> {
        let name = validate_name(name)?;
        validate_metadata(name, "", &[])?;
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
            description: source.description,
            tags: source.tags,
            scope: source.scope,
            created_at: now,
            updated_at: now,
            definition: source.definition,
        });
        self.sort_saved();
        Ok(id)
    }

    pub fn preview_import(&self, inputs: &[SavedQueryInput]) -> Result<QueryImportReport, String> {
        let mut preview = self.clone();
        preview.import_saved(inputs.to_vec())
    }

    pub fn import_saved(
        &mut self,
        inputs: Vec<SavedQueryInput>,
    ) -> Result<QueryImportReport, String> {
        let mut normalized =
            inputs.into_iter().map(SavedQueryInput::normalized).collect::<Result<Vec<_>, _>>()?;
        let mut names = self
            .saved
            .iter()
            .map(|query| query.name.to_ascii_lowercase())
            .collect::<std::collections::HashSet<_>>();
        let mut renamed = 0;
        for input in &mut normalized {
            if names.contains(&input.name.to_ascii_lowercase()) {
                input.name = available_import_name(&input.name, &names);
                renamed += 1;
            }
            names.insert(input.name.to_ascii_lowercase());
        }

        let mut report =
            QueryImportReport { imported: normalized.len(), renamed, ..Default::default() };
        for input in normalized {
            match input.scope {
                SavedQueryScope::Global => report.global += 1,
                SavedQueryScope::Connection => report.connection += 1,
            }
            let id = Uuid::new_v4();
            let now = Utc::now();
            self.saved.push(SavedQuery {
                id,
                name: input.name,
                description: input.description,
                tags: input.tags,
                scope: input.scope,
                created_at: now,
                updated_at: now,
                definition: input.definition,
            });
        }
        self.sort_saved();
        Ok(report)
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
    if let QueryContent::Aggregation { stages, selected_stage: Some(selected_stage) } =
        &definition.content
        && *selected_stage >= stages.len()
    {
        return Err("The selected aggregation stage is outside the pipeline.".to_string());
    }
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
    } else if name.chars().count() > QUERY_NAME_LIMIT {
        Err(format!("Query names must be {QUERY_NAME_LIMIT} characters or fewer."))
    } else {
        Ok(name)
    }
}

fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if tag.is_empty() || normalized.iter().any(|saved: &String| saved.eq_ignore_ascii_case(tag))
        {
            continue;
        }
        if tag.chars().count() > QUERY_TAG_LENGTH_LIMIT {
            return Err(format!(
                "Query tags must be {QUERY_TAG_LENGTH_LIMIT} characters or fewer."
            ));
        }
        normalized.push(tag.to_string());
        if normalized.len() > QUERY_TAG_LIMIT {
            return Err(format!("Queries can have at most {QUERY_TAG_LIMIT} tags."));
        }
    }
    Ok(normalized)
}

fn validate_metadata(name: &str, description: &str, tags: &[String]) -> Result<(), String> {
    if std::iter::once(name)
        .chain(std::iter::once(description))
        .chain(tags.iter().map(String::as_str))
        .any(may_contain_credentials)
    {
        Err("Query metadata that may contain credentials cannot be saved.".to_string())
    } else {
        Ok(())
    }
}

fn available_import_name(source: &str, names: &std::collections::HashSet<String>) -> String {
    let mut number = 1;
    loop {
        let suffix =
            if number == 1 { " Imported".to_string() } else { format!(" Imported {number}") };
        let candidate = copy_name(source, &suffix);
        if !names.contains(&candidate.to_ascii_lowercase()) {
            return candidate;
        }
        number += 1;
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
    if [
        "mongodb://",
        "mongodb+srv://",
        "db.auth(",
        "db.auth (",
        "db.createuser(",
        "db.updateuser(",
        "db.changeuserpassword(",
        "bearer ",
        "-----begin private key-----",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return true;
    }

    [
        "authenticate",
        "saslstart",
        "saslcontinue",
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
        "api key",
        "apikey",
        "access_key",
        "access key",
        "accesskey",
        "access_token",
        "auth_token",
        "token",
        "authorization",
        "bearer",
        "client_secret",
        "client secret",
        "private_key",
        "private key",
        "privatekey",
        "aws_access_key_id",
        "aws_secret_access_key",
        "aws_session_token",
        "nonce",
    ]
    .iter()
    .any(|key| contains_sensitive_assignment(&lower, key))
}

fn contains_sensitive_assignment(raw: &str, key: &str) -> bool {
    raw.match_indices(key).any(|(start, _)| {
        let end = start + key.len();
        let before_is_word = raw[..start]
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        let after_is_word =
            raw[end..].chars().next().is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        if before_is_word || after_is_word {
            return false;
        }

        let tail = &raw[end..];
        let trimmed = tail
            .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | '`'));
        let separated_by_whitespace = tail.len() != tail.trim_start().len();
        trimmed.starts_with(':')
            || trimmed.starts_with('=')
            || trimmed.starts_with("->")
            || trimmed.starts_with("=>")
            || (separated_by_whitespace
                && trimmed
                    .chars()
                    .next()
                    .is_some_and(|ch| !matches!(ch, '.' | ',' | ')' | ']' | '}' | ';')))
    })
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
            "db.users.find({ token: 'secret-value' })",
            "db.runCommand({ authenticate: 1, user: 'admin', nonce: 'n', key: 'k' })",
            "const privateKey = 'secret-value'",
        ] {
            assert!(!library.record(forge_definition(statement)), "recorded {statement}");
        }
        assert!(library.history().is_empty());
        assert!(library.record(forge_definition("db.tokens.find({ active: true })")));
        assert!(library.record(forge_definition("db.secretsArchive.find({ active: true })")));
        assert!(library.record(forge_definition("db.authTokens.find({ active: true })")));
        assert!(library.record(forge_definition("db.createUsersAudit.find({ active: true })")));
        assert_eq!(library.history().len(), 4);
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

    #[test]
    fn legacy_saved_query_defaults_metadata_and_connection_scope() {
        let now = Utc::now();
        let json = serde_json::json!({
            "id": Uuid::new_v4(),
            "name": "Legacy",
            "created_at": now,
            "updated_at": now,
            "definition": forge_definition("db.users.find({})")
        });
        let saved: SavedQuery = serde_json::from_value(json).unwrap();
        assert_eq!(saved.description, "");
        assert!(saved.tags.is_empty());
        assert_eq!(saved.scope, SavedQueryScope::Connection);
    }

    #[test]
    fn metadata_is_normalized_searchable_and_global_across_namespaces() {
        let mut library = QueryLibrary::default();
        let id = library
            .save_input(SavedQueryInput {
                name: "  Active users  ".into(),
                description: "  Accounts needing review  ".into(),
                tags: vec![" Team ".into(), "team".into(), "Review".into()],
                scope: SavedQueryScope::Global,
                definition: forge_definition("db.users.find({ active: true })"),
            })
            .unwrap();
        let saved = library.saved_query(id).unwrap();
        assert_eq!(saved.name, "Active users");
        assert_eq!(saved.description, "Accounts needing review");
        assert_eq!(saved.tags, ["Team", "Review"]);
        assert!(saved.matches_search("review"));
        assert!(saved.matches_scope(QueryKind::Forge, Uuid::new_v4(), "other", None));
        assert!(!saved.matches_scope(QueryKind::Documents, Uuid::new_v4(), "other", None));

        let mut duplicate = saved.clone();
        duplicate.scope = SavedQueryScope::Connection;
        assert!(!duplicate.matches_scope(QueryKind::Forge, Uuid::new_v4(), "other", None));
    }

    #[test]
    fn import_resolves_all_collisions_and_is_all_or_nothing() {
        let mut library = QueryLibrary::default();
        library.save(forge_definition("db.a.find({})"), "Users").unwrap();
        let input = |name: &str, statement: &str| SavedQueryInput {
            name: name.into(),
            description: String::new(),
            tags: Vec::new(),
            scope: SavedQueryScope::Connection,
            definition: forge_definition(statement),
        };

        let report = library
            .import_saved(vec![input("Users", "db.b.find({})"), input("users", "db.c.find({})")])
            .unwrap();
        assert_eq!(report.imported, 2);
        assert_eq!(report.renamed, 2);
        assert!(library.saved().iter().any(|query| query.name == "Users Imported"));
        assert!(library.saved().iter().any(|query| query.name == "users Imported 2"));

        let before = library.clone();
        assert!(
            library
                .import_saved(vec![
                    input("Valid", "db.valid.find({})"),
                    input("Unsafe", "db.auth('admin', 'secret-value')"),
                ])
                .is_err()
        );
        assert_eq!(library, before);
    }

    #[test]
    fn metadata_limits_and_explicit_secret_assignments_are_rejected() {
        let input = |description: String, tags: Vec<String>| SavedQueryInput {
            name: "Users".into(),
            description,
            tags,
            scope: SavedQueryScope::Connection,
            definition: forge_definition("db.users.find({})"),
        };
        for secret in [
            "password = hunter2",
            "credential: hunter2",
            "private_key -> hunter2",
            "AWS_ACCESS_KEY_ID : AKIA123",
            "aws_secret_access_key=>secret-value",
            "token abc123",
            "password hunter2",
            "-----BEGIN PRIVATE KEY-----",
        ] {
            assert!(input(secret.into(), vec![]).normalized().is_err(), "accepted {secret}");
        }
        assert!(input("x".repeat(QUERY_DESCRIPTION_LIMIT + 1), vec![]).normalized().is_err());
        assert!(
            input(
                String::new(),
                (0..=QUERY_TAG_LIMIT).map(|index| format!("tag-{index}")).collect(),
            )
            .normalized()
            .is_err()
        );
    }

    #[test]
    fn invalid_aggregation_selection_is_rejected() {
        let input = SavedQueryInput {
            name: "Invalid pipeline".into(),
            description: String::new(),
            tags: Vec::new(),
            scope: SavedQueryScope::Global,
            definition: QueryDefinition {
                connection_id: Uuid::nil(),
                database: "app".into(),
                collection: Some("users".into()),
                content: QueryContent::Aggregation {
                    stages: vec![PipelineStage {
                        operator: "$match".into(),
                        body: "{}".into(),
                        enabled: true,
                    }],
                    selected_stage: Some(1),
                },
            },
        };
        assert!(input.normalized().is_err());
    }

    #[test]
    fn import_preview_uses_the_same_collision_resolver_without_mutation() {
        let mut library = QueryLibrary::default();
        library.save(forge_definition("db.a.find({})"), "Users").unwrap();
        let input = SavedQueryInput {
            name: "Users".into(),
            description: String::new(),
            tags: Vec::new(),
            scope: SavedQueryScope::Connection,
            definition: forge_definition("db.b.find({})"),
        };
        let before = library.clone();
        let preview = library.preview_import(&[input]).unwrap();
        assert_eq!(preview.imported, 1);
        assert_eq!(preview.renamed, 1);
        assert_eq!(library, before);
    }
}
