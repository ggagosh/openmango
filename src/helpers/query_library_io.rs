//! Portable saved-query import and export.

use std::io::{Read as _, Write as _};
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use chrono::{DateTime, Utc};
use mongodb::bson::{Bson, Document};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::bson::format_relaxed_json_compact;
use crate::state::{QueryContent, QueryDefinition, SavedQuery, SavedQueryInput, SavedQueryScope};

pub const CURRENT_VERSION: u32 = 1;
pub const MAX_IMPORT_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_IMPORT_QUERIES: usize = 1_000;
const MAX_NAMESPACE_CHARS: usize = 255;
const MAX_CONTENT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryLibraryExportFile {
    pub version: u32,
    pub app: String,
    pub exported_at: DateTime<Utc>,
    pub queries: Vec<PortableSavedQuery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableSavedQuery {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub scope: SavedQueryScope,
    pub database: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    pub content: PortableQueryContent,
}

/// The portable Documents shape intentionally contains only executable BSON.
/// Display text is regenerated from these documents during import, so a file
/// cannot present one query while executing another.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableDocumentQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    filter: Option<Document>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sort: Option<Document>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    projection: Option<Document>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "query", rename_all = "snake_case", deny_unknown_fields)]
pub enum PortableQueryContent {
    Documents(Box<PortableDocumentQuery>),
    Aggregation {
        stages: Vec<crate::state::app_state::PipelineStage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected_stage: Option<usize>,
    },
    Forge {
        statement: String,
    },
}

impl PortableQueryContent {
    fn from_saved(content: &QueryContent) -> Self {
        match content {
            QueryContent::Documents(query) => Self::Documents(Box::new(PortableDocumentQuery {
                filter: query.filter.clone(),
                sort: query.sort.clone(),
                projection: query.projection.clone(),
            })),
            QueryContent::Aggregation { stages, selected_stage } => {
                Self::Aggregation { stages: stages.clone(), selected_stage: *selected_stage }
            }
            QueryContent::Forge { statement } => Self::Forge { statement: statement.clone() },
        }
    }

    fn into_query_content(self) -> QueryContent {
        match self {
            Self::Documents(query) => {
                let filter_raw =
                    query.filter.as_ref().map(canonical_document).unwrap_or_else(|| "{}".into());
                let sort_raw = query.sort.as_ref().map(canonical_document).unwrap_or_default();
                let projection_raw =
                    query.projection.as_ref().map(canonical_document).unwrap_or_default();
                QueryContent::Documents(Box::new(crate::state::DocumentQuery {
                    filter_raw,
                    filter: query.filter,
                    sort_raw,
                    sort: query.sort,
                    projection_raw,
                    projection: query.projection,
                }))
            }
            Self::Aggregation { stages, selected_stage } => {
                QueryContent::Aggregation { stages, selected_stage }
            }
            Self::Forge { statement } => QueryContent::Forge { statement },
        }
    }
}

impl PortableSavedQuery {
    pub fn into_input(self, connection_id: Uuid) -> Result<SavedQueryInput> {
        SavedQueryInput {
            name: self.name,
            description: self.description,
            tags: self.tags,
            scope: self.scope,
            definition: QueryDefinition {
                connection_id,
                database: self.database.trim().to_string(),
                collection: self.collection.map(|value| value.trim().to_string()),
                content: self.content.into_query_content(),
            },
        }
        .normalized()
        .map_err(anyhow::Error::msg)
    }

    fn validate(&self) -> Result<()> {
        validate_namespace("database", &self.database)?;
        if let Some(collection) = self.collection.as_deref() {
            validate_namespace("collection", collection)?;
        }
        let content_bytes = serde_json::to_vec(&self.content)?.len();
        if content_bytes > MAX_CONTENT_BYTES {
            bail!("saved query content exceeds the 1 MiB limit");
        }
        self.clone().into_input(Uuid::nil())?;
        Ok(())
    }
}

pub fn build_export(saved: &[SavedQuery]) -> Result<QueryLibraryExportFile> {
    if saved.len() > MAX_IMPORT_QUERIES {
        bail!("query library has too many saved queries to export");
    }
    let queries = saved
        .iter()
        .map(|query| PortableSavedQuery {
            name: query.name.clone(),
            description: query.description.clone(),
            tags: query.tags.clone(),
            scope: query.scope,
            database: query.definition.database.clone(),
            collection: query.definition.collection.clone(),
            content: PortableQueryContent::from_saved(&query.definition.content),
        })
        .map(|query| {
            query.validate()?;
            Ok(query)
        })
        .collect::<Result<Vec<_>>>()?;

    let file = QueryLibraryExportFile {
        version: CURRENT_VERSION,
        app: "openmango".to_string(),
        exported_at: Utc::now(),
        queries,
    };
    encoded_export(&file)?;
    Ok(file)
}

pub fn parse_import(json: &str) -> Result<QueryLibraryExportFile> {
    if json.len() as u64 > MAX_IMPORT_BYTES {
        bail!("query library file exceeds the 5 MiB limit");
    }
    let file: QueryLibraryExportFile =
        serde_json::from_str(json).context("invalid query library JSON")?;
    if file.app != "openmango" {
        bail!("this is not an OpenMango query library file");
    }
    if file.version != CURRENT_VERSION {
        bail!(
            "unsupported query library version {} (supported: {})",
            file.version,
            CURRENT_VERSION
        );
    }
    if file.queries.is_empty() {
        bail!("query library file contains no saved queries");
    }
    if file.queries.len() > MAX_IMPORT_QUERIES {
        bail!("query library file contains more than {MAX_IMPORT_QUERIES} saved queries");
    }
    for query in &file.queries {
        query.validate()?;
    }
    Ok(file)
}

pub fn read_import(path: &Path) -> Result<QueryLibraryExportFile> {
    let file =
        std::fs::File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(MAX_IMPORT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_IMPORT_BYTES {
        bail!("query library file exceeds the 5 MiB limit");
    }
    let json = String::from_utf8(bytes).context("query library file must be UTF-8")?;
    parse_import(&json)
}

pub fn write_export(path: &Path, file: &QueryLibraryExportFile) -> Result<()> {
    let json = encoded_export(file)?;
    let dir = path.parent().unwrap_or(path);
    let mut temp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("failed to create export beside {}", path.display()))?;
    temp.write_all(&json)?;
    temp.flush()?;
    temp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn encoded_export(file: &QueryLibraryExportFile) -> Result<Vec<u8>> {
    let json = serde_json::to_vec_pretty(file)?;
    if json.len() as u64 > MAX_IMPORT_BYTES {
        bail!("query library export exceeds the 5 MiB limit");
    }
    Ok(json)
}

fn canonical_document(document: &Document) -> String {
    format_relaxed_json_compact(&Bson::Document(document.clone()).into_relaxed_extjson())
}

fn validate_namespace(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("saved query {label} cannot be empty");
    }
    if value.chars().count() > MAX_NAMESPACE_CHARS {
        bail!("saved query {label} exceeds {MAX_NAMESPACE_CHARS} characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use mongodb::bson::doc;
    use tempfile::TempDir;

    use super::*;
    use crate::state::{DocumentQuery, QueryLibrary};

    fn saved_query(scope: SavedQueryScope, content: QueryContent) -> SavedQuery {
        let mut library = QueryLibrary::default();
        let id = library
            .save_input(SavedQueryInput {
                name: "Active users".into(),
                description: "Reusable account query".into(),
                tags: vec!["accounts".into(), "active".into()],
                scope,
                definition: QueryDefinition {
                    connection_id: Uuid::new_v4(),
                    database: "app".into(),
                    collection: Some("users".into()),
                    content,
                },
            })
            .unwrap();
        library.saved_query(id).unwrap().clone()
    }

    fn document_query() -> SavedQuery {
        saved_query(
            SavedQueryScope::Global,
            QueryContent::Documents(Box::new(DocumentQuery {
                filter_raw: "status = active".into(),
                filter: Some(doc! { "status": "active" }),
                sort_raw: "{ createdAt: -1 }".into(),
                sort: Some(doc! { "createdAt": -1 }),
                projection_raw: String::new(),
                projection: None,
            })),
        )
    }

    #[test]
    fn portable_round_trip_omits_local_identity_and_canonicalizes_documents() {
        let connection_id = Uuid::new_v4();
        let mut query = document_query();
        query.definition.connection_id = connection_id;

        let file = build_export(&[query]).unwrap();
        let json = serde_json::to_string_pretty(&file).unwrap();
        assert!(!json.contains(&connection_id.to_string()));
        assert!(!json.contains("filter_raw"));
        assert!(!json.contains("created_at"));
        assert!(!json.contains("updated_at"));
        assert!(!json.contains("history"));

        let parsed = parse_import(&json).unwrap();
        let input = parsed.queries.into_iter().next().unwrap().into_input(Uuid::nil()).unwrap();
        let QueryContent::Documents(query) = input.definition.content else {
            panic!("expected Documents query");
        };
        assert_eq!(query.filter, Some(doc! { "status": "active" }));
        assert!(query.filter_raw.contains("active"));
        assert!(!query.filter_raw.contains("status = active"));
    }

    #[test]
    fn rejects_document_display_execution_mismatch_fields() {
        let file = build_export(&[document_query()]).unwrap();
        let mut value = serde_json::to_value(file).unwrap();
        value["queries"][0]["content"]["query"]["filter_raw"] =
            serde_json::Value::String("{ role: 'admin' }".into());
        assert!(parse_import(&serde_json::to_string(&value).unwrap()).is_err());
    }

    #[test]
    fn all_query_kinds_round_trip() {
        let queries = vec![
            document_query(),
            saved_query(
                SavedQueryScope::Global,
                QueryContent::Aggregation {
                    stages: vec![crate::state::app_state::PipelineStage {
                        operator: "$match".into(),
                        body: "{ active: true }".into(),
                        enabled: true,
                    }],
                    selected_stage: Some(0),
                },
            ),
            saved_query(
                SavedQueryScope::Connection,
                QueryContent::Forge { statement: "db.users.find({})".into() },
            ),
        ];
        let json = serde_json::to_string(&build_export(&queries).unwrap()).unwrap();
        let parsed = parse_import(&json).unwrap();
        assert_eq!(parsed.queries.len(), 3);
        assert!(matches!(parsed.queries[0].content, PortableQueryContent::Documents(_)));
        assert!(matches!(parsed.queries[1].content, PortableQueryContent::Aggregation { .. }));
        assert!(matches!(parsed.queries[2].content, PortableQueryContent::Forge { .. }));
    }

    #[test]
    fn rejects_wrong_envelope_credentials_and_invalid_stage() {
        let query = saved_query(
            SavedQueryScope::Connection,
            QueryContent::Forge { statement: "db.users.find({})".into() },
        );
        let mut file = build_export(&[query]).unwrap();
        file.version = CURRENT_VERSION + 1;
        assert!(parse_import(&serde_json::to_string(&file).unwrap()).is_err());
        file.version = CURRENT_VERSION;
        file.app = "other".into();
        assert!(parse_import(&serde_json::to_string(&file).unwrap()).is_err());
        file.app = "openmango".into();
        file.queries[0].content =
            PortableQueryContent::Forge { statement: "db.auth('admin', 'secret')".into() };
        assert!(parse_import(&serde_json::to_string(&file).unwrap()).is_err());

        let mut legacy = saved_query(
            SavedQueryScope::Connection,
            QueryContent::Forge { statement: "db.users.find({})".into() },
        );
        legacy.description = "AWS_ACCESS_KEY_ID : secret-value".into();
        assert!(build_export(&[legacy]).is_err());

        let mut aggregation = saved_query(
            SavedQueryScope::Global,
            QueryContent::Aggregation {
                stages: vec![crate::state::app_state::PipelineStage {
                    operator: "$match".into(),
                    body: "{}".into(),
                    enabled: true,
                }],
                selected_stage: Some(0),
            },
        );
        if let QueryContent::Aggregation { selected_stage, .. } =
            &mut aggregation.definition.content
        {
            *selected_stage = Some(2);
        }
        assert!(build_export(&[aggregation]).is_err());
    }

    #[test]
    fn rejects_malformed_empty_oversized_and_non_utf8_files() {
        assert!(parse_import("not json").is_err());
        let empty = QueryLibraryExportFile {
            version: CURRENT_VERSION,
            app: "openmango".into(),
            exported_at: Utc::now(),
            queries: Vec::new(),
        };
        assert!(parse_import(&serde_json::to_string(&empty).unwrap()).is_err());
        assert!(parse_import(&" ".repeat(MAX_IMPORT_BYTES as usize + 1)).is_err());

        let dir = TempDir::new().unwrap();
        let oversized = dir.path().join("oversized.json");
        std::fs::write(&oversized, vec![b' '; MAX_IMPORT_BYTES as usize + 2]).unwrap();
        assert!(read_import(&oversized).is_err());
        let invalid = dir.path().join("invalid.json");
        std::fs::write(&invalid, [0xff, 0xfe]).unwrap();
        assert!(read_import(&invalid).is_err());
    }

    #[test]
    fn export_size_limit_is_enforced_before_writing() {
        let base = saved_query(
            SavedQueryScope::Global,
            QueryContent::Forge { statement: "x".repeat(900_000) },
        );
        let queries = (0..6)
            .map(|index| {
                let mut query = base.clone();
                query.id = Uuid::new_v4();
                query.name = format!("Large {index}");
                query
            })
            .collect::<Vec<_>>();
        assert!(build_export(&queries).is_err());
    }

    #[test]
    fn atomic_export_replaces_complete_destination() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("queries.json");
        std::fs::write(&path, b"old").unwrap();
        let query = saved_query(
            SavedQueryScope::Connection,
            QueryContent::Forge { statement: "db.users.find({})".into() },
        );
        write_export(&path, &build_export(&[query]).unwrap()).unwrap();
        assert!(parse_import(&std::fs::read_to_string(path).unwrap()).is_ok());
    }
}
