use std::path::Path;
use std::time::Duration;

use mongodb::Client;
use serde::Serialize;

use crate::actions::hash_serializable;
use crate::actions::model::{
    ACTION_POLICY_VERSION, ActionOrigin, ActionPolicySnapshot, ActionPrerequisites, ActionPreview,
    ActionRequest, ConnectionActionSnapshot, DatabaseStateFingerprint, ProposedActionContent,
    SyncMode,
};

const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct RuntimeActionConnection {
    pub client: Client,
    pub tool_uri: String,
    pub snapshot: ConnectionActionSnapshot,
    pub databases: Vec<String>,
}

pub struct ActionPreflight {
    pub source: Option<RuntimeActionConnection>,
    pub target: RuntimeActionConnection,
    pub backup_root: std::path::PathBuf,
    pub recovery_interlocks: Vec<(uuid::Uuid, String)>,
}

impl ActionPreflight {
    pub async fn prepare(
        self,
        request: ActionRequest,
        origin: ActionOrigin,
    ) -> Result<ProposedActionContent, String> {
        validate_request(&request)?;
        ensure_backup_storage(&self.backup_root)?;
        let tools_available = crate::connection::tools::mongodump_path().is_some()
            && crate::connection::tools::mongorestore_path().is_some();
        if !tools_available {
            return Err("MongoDB Database Tools are unavailable".to_string());
        }

        let (source_database, target_database, mode) = match &request {
            ActionRequest::DatabaseBackup { database, .. } => (None, database.clone(), None),
            ActionRequest::DatabaseSync { source_database, target_database, mode, .. } => {
                (Some(source_database.clone()), target_database.clone(), Some(*mode))
            }
            ActionRequest::OperationRevert { .. } => {
                return Err("Revert preflight must be prepared from its original operation".into());
            }
            ActionRequest::DocumentTransitions { .. } => {
                return Err("Document transitions require document preflight".into());
            }
        };
        if !matches!(request, ActionRequest::DatabaseBackup { .. })
            && self.recovery_interlocks.iter().any(|(connection_id, database)| {
                *connection_id == self.target.snapshot.connection_id && database == &target_database
            })
        {
            return Err("Target database requires recovery before another agent operation".into());
        }

        let source_fingerprint =
            if let (Some(source), Some(database)) = (&self.source, source_database.as_deref()) {
                Some(database_fingerprint(source, database).await?)
            } else {
                None
            };
        let target_fingerprint = database_fingerprint(&self.target, &target_database).await?;
        match &request {
            ActionRequest::DatabaseBackup { .. } if !target_fingerprint.exists => {
                return Err("Database does not exist on the selected connection".into());
            }
            ActionRequest::DatabaseSync { .. }
                if source_fingerprint.as_ref().is_none_or(|fingerprint| !fingerprint.exists) =>
            {
                return Err("Source database does not exist".into());
            }
            _ => {}
        }
        let mut warnings = vec!["Available backup capacity could not be verified".to_string()];
        if self.target.snapshot.protected {
            warnings.push("Target is protected or Production".to_string());
        }
        if matches!(request, ActionRequest::DatabaseSync { .. }) {
            warnings.push("The target database will be replaced after a verified backup".into());
            warnings.push(
                "The source dump reflects live data and is not a cross-collection transaction"
                    .into(),
            );
        } else {
            warnings.push(
                "The backup reflects live data and is not a cross-collection transaction".into(),
            );
        }
        let estimated_documents = source_fingerprint
            .as_ref()
            .map(|fingerprint| fingerprint.estimated_documents)
            .unwrap_or(target_fingerprint.estimated_documents);
        let estimated_bytes = source_fingerprint
            .as_ref()
            .map(|fingerprint| fingerprint.estimated_bytes)
            .unwrap_or(target_fingerprint.estimated_bytes);
        let summary = match &request {
            ActionRequest::DatabaseBackup { database, .. } => format!(
                "Create a verified app-managed backup of {} / {database}",
                self.target.snapshot.display_name
            ),
            ActionRequest::DatabaseSync { source_database, target_database, .. } => format!(
                "Replace {} / {target_database} with {} / {source_database}",
                self.target.snapshot.display_name,
                self.source
                    .as_ref()
                    .map(|source| source.snapshot.display_name.as_str())
                    .unwrap_or("source")
            ),
            ActionRequest::OperationRevert { .. } | ActionRequest::DocumentTransitions { .. } => {
                unreachable!()
            }
        };

        Ok(ProposedActionContent {
            request,
            origin,
            policy: ActionPolicySnapshot {
                version: ACTION_POLICY_VERSION,
                source_shared: self
                    .source
                    .as_ref()
                    .is_none_or(|source| source.snapshot.agent_shared),
                target_shared: self.target.snapshot.agent_shared,
                target_writable: !self.target.snapshot.read_only,
                target_protected: self.target.snapshot.protected,
            },
            preview: ActionPreview {
                summary,
                source: self.source.as_ref().map(|source| source.snapshot.clone()),
                target: self.target.snapshot,
                source_database,
                target_database,
                mode,
                estimated_documents,
                estimated_bytes,
                warnings,
                backup_behavior: match mode {
                    Some(SyncMode::Replace) => {
                        "Create and verify a full target backup before replacement".into()
                    }
                    None => "Create and verify an app-managed backup".into(),
                },
                rollback_behavior: match mode {
                    Some(SyncMode::Replace) => {
                        "Automatically restore the target backup if replacement fails".into()
                    }
                    None => "Backup creation does not mutate MongoDB".into(),
                },
            },
            prerequisites: ActionPrerequisites {
                database_tools_available: true,
                source_reachable: source_fingerprint.is_some() || self.source.is_none(),
                target_reachable: true,
                backup_storage_available: true,
                free_space_known_sufficient: None,
            },
            source_state_fingerprint: source_fingerprint,
            target_state_fingerprint: target_fingerprint,
        })
    }

    pub async fn prepare_document_action(
        self,
        request: ActionRequest,
        origin: ActionOrigin,
        estimated_documents: u64,
        estimated_bytes: u64,
    ) -> Result<ProposedActionContent, String> {
        let ActionRequest::DocumentTransitions {
            connection_id,
            database,
            collection,
            action,
            operation_ids,
        } = request.clone()
        else {
            return Err("Document action request is invalid".to_string());
        };
        validate_database_name(&database)?;
        validate_collection_name(&collection)?;
        if connection_id != self.target.snapshot.connection_id {
            return Err("Document action target does not match the approved connection".into());
        }
        if operation_ids.is_empty()
            || operation_ids.len() > crate::operations::MAX_REVERSIBLE_BULK_DOCUMENTS
        {
            return Err("Document action must contain 1-100 prepared transitions".into());
        }
        if self.recovery_interlocks.iter().any(|(target_id, target_database)| {
            *target_id == connection_id && target_database == &database
        }) {
            return Err("Target database requires recovery before another agent operation".into());
        }
        let target_fingerprint = database_fingerprint(&self.target, &database).await?;
        if !target_fingerprint.exists
            || !target_fingerprint.collections.iter().any(|candidate| candidate == &collection)
        {
            return Err("Target collection does not exist".into());
        }
        let action_label = match action {
            crate::actions::model::DocumentActionKind::Insert => "Insert into",
            crate::actions::model::DocumentActionKind::Replace => "Replace documents in",
            crate::actions::model::DocumentActionKind::Delete => "Delete documents from",
        };
        let mut warnings = vec![
            "Each document will be applied only if it still matches the encrypted proposal checkpoint."
                .to_string(),
        ];
        if self.target.snapshot.protected {
            warnings.push("Target is protected or Production".to_string());
        }
        Ok(ProposedActionContent {
            request,
            origin,
            policy: ActionPolicySnapshot {
                version: ACTION_POLICY_VERSION,
                source_shared: true,
                target_shared: self.target.snapshot.agent_shared,
                target_writable: !self.target.snapshot.read_only,
                target_protected: self.target.snapshot.protected,
            },
            preview: ActionPreview {
                summary: format!(
                    "{action_label} {} / {database}.{collection} ({estimated_documents} document{})",
                    self.target.snapshot.display_name,
                    if estimated_documents == 1 { "" } else { "s" }
                ),
                source: None,
                target: self.target.snapshot,
                source_database: None,
                target_database: database.clone(),
                mode: None,
                estimated_documents,
                estimated_bytes,
                warnings,
                backup_behavior: "Encrypted before/after checkpoints are already prepared; no database backup is required."
                    .into(),
                rollback_behavior: "Completed document transitions can be reverted individually from collection History."
                    .into(),
            },
            prerequisites: ActionPrerequisites {
                database_tools_available: false,
                source_reachable: true,
                target_reachable: true,
                backup_storage_available: false,
                free_space_known_sufficient: Some(true),
            },
            source_state_fingerprint: None,
            target_state_fingerprint: target_fingerprint,
        })
    }
}

pub async fn database_fingerprint(
    connection: &RuntimeActionConnection,
    database: &str,
) -> Result<DatabaseStateFingerprint, String> {
    let exists = connection.databases.iter().any(|candidate| candidate == database);
    let future = async {
        let mut collections = if exists {
            connection.client.database(database).list_collection_names().await?
        } else {
            Vec::new()
        };
        collections.sort();
        let stats = if exists {
            connection
                .client
                .database(database)
                .run_command(mongodb::bson::doc! { "dbStats": 1, "scale": 1 })
                .await
                .ok()
        } else {
            None
        };
        Ok::<_, mongodb::error::Error>((collections, stats))
    };
    let (collections, stats) = tokio::time::timeout(PREFLIGHT_TIMEOUT, future)
        .await
        .map_err(|_| "Database preflight timed out".to_string())?
        .map_err(|_| "Database preflight failed; check OpenMango logs".to_string())?;
    let estimated_documents = stats.as_ref().map(|stats| bson_u64(stats, "objects")).unwrap_or(0);
    let estimated_bytes = stats.as_ref().map(|stats| bson_u64(stats, "dataSize")).unwrap_or(0);
    let hash = hash_serializable(&FingerprintHashInput {
        exists,
        collections: &collections,
        estimated_documents,
        estimated_bytes,
    })
    .map_err(|_| "Could not fingerprint database state".to_string())?;
    Ok(DatabaseStateFingerprint { exists, collections, estimated_documents, estimated_bytes, hash })
}

#[derive(Serialize)]
struct FingerprintHashInput<'a> {
    exists: bool,
    collections: &'a [String],
    estimated_documents: u64,
    estimated_bytes: u64,
}

fn bson_u64(document: &mongodb::bson::Document, field: &str) -> u64 {
    match document.get(field) {
        Some(mongodb::bson::Bson::Int32(value)) => u64::try_from(*value).unwrap_or(0),
        Some(mongodb::bson::Bson::Int64(value)) => u64::try_from(*value).unwrap_or(0),
        Some(mongodb::bson::Bson::Double(value)) if value.is_finite() && *value >= 0.0 => {
            *value as u64
        }
        _ => 0,
    }
}

fn validate_request(request: &ActionRequest) -> Result<(), String> {
    match request {
        ActionRequest::DatabaseBackup { database, .. } => validate_database_name(database),
        ActionRequest::DatabaseSync {
            source_connection_id,
            source_database,
            target_connection_id,
            target_database,
            mode: SyncMode::Replace,
        } => {
            validate_database_name(source_database)?;
            validate_database_name(target_database)?;
            if source_connection_id == target_connection_id && source_database == target_database {
                return Err("Source and target database must differ".into());
            }
            Ok(())
        }
        ActionRequest::OperationRevert { .. } => Ok(()),
        ActionRequest::DocumentTransitions { database, collection, operation_ids, .. } => {
            validate_database_name(database)?;
            validate_collection_name(collection)?;
            if operation_ids.is_empty()
                || operation_ids.len() > crate::operations::MAX_REVERSIBLE_BULK_DOCUMENTS
            {
                return Err("Document action must contain 1-100 prepared transitions".into());
            }
            Ok(())
        }
    }
}

pub fn validate_database_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err("Database name must be 1-64 characters".into());
    }
    if name.chars().any(|character| {
        matches!(character, '/' | '\\' | '.' | '"' | '*' | '<' | '>' | ':' | '|' | '?' | '\0' | ' ')
    }) {
        return Err("Database name contains an unsupported character".into());
    }
    Ok(())
}

fn validate_collection_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 255 || name.contains('\0') || name.starts_with("system.") {
        return Err("Collection name is unsupported for agent document writes".into());
    }
    Ok(())
}

fn ensure_backup_storage(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path)
        .map_err(|_| "App-managed backup storage is unavailable".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Could not secure backup storage".to_string())?;
    }
    Ok(())
}
