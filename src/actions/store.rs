use std::cmp::Reverse;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context as _, Result};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use super::model::{BackupManifest, OperationRecord, ProposedAction};

pub struct ActionStore {
    root: PathBuf,
    write_lock: Mutex<()>,
}

#[derive(Serialize, serde::Deserialize)]
struct AcceptTransaction {
    action: ProposedAction,
    operation: OperationRecord,
}

impl ActionStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root, write_lock: Mutex::new(()) }
    }

    pub fn backups_root(&self) -> PathBuf {
        self.root.join("backups")
    }

    pub fn backup_dir(&self, backup_id: Uuid) -> PathBuf {
        self.backups_root().join(backup_id.to_string())
    }

    pub fn source_dump_dir(&self, operation_id: Uuid) -> PathBuf {
        self.root.join("operations").join(operation_id.to_string()).join("source-dump")
    }

    pub fn save_action(&self, action: &ProposedAction) -> Result<()> {
        self.save_json(&self.action_path(action.id), action)
    }

    pub fn accept_action(
        &self,
        action: &ProposedAction,
        operation: &OperationRecord,
    ) -> Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Action storage lock is unavailable"))?;
        let transaction_path = self.root.join("transactions").join(format!("{}.json", action.id));
        self.save_json_locked(
            &transaction_path,
            &AcceptTransaction { action: action.clone(), operation: operation.clone() },
        )?;
        self.save_json_locked(&self.operation_path(operation.id), operation)?;
        self.save_json_locked(&self.action_path(action.id), action)?;
        fs::remove_file(transaction_path)?;
        Ok(())
    }

    pub fn load_action(&self, id: Uuid) -> Result<ProposedAction> {
        self.load_json(&self.action_path(id))
    }

    pub fn list_actions(&self) -> Result<Vec<ProposedAction>> {
        let mut actions =
            self.load_directory::<ProposedAction>(&self.root.join("actions"), "json")?;
        actions.sort_by_key(|action| Reverse(action.created_at));
        Ok(actions)
    }

    pub fn save_operation(&self, operation: &OperationRecord) -> Result<()> {
        self.save_json(&self.operation_path(operation.id), operation)
    }

    pub fn load_operation(&self, id: Uuid) -> Result<OperationRecord> {
        self.load_json(&self.operation_path(id))
    }

    pub fn list_operations(&self) -> Result<Vec<OperationRecord>> {
        let root = self.root.join("operations");
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut operations: Vec<OperationRecord> = Vec::new();
        for entry in
            fs::read_dir(&root).with_context(|| format!("Failed to read {}", root.display()))?
        {
            let path = entry?.path().join("operation.json");
            if path.exists() {
                operations.push(self.load_json(&path)?);
            }
        }
        operations.sort_by_key(|operation| Reverse(operation.created_at));
        Ok(operations)
    }

    pub fn save_backup_manifest(&self, manifest: &BackupManifest) -> Result<()> {
        self.save_json(&self.backup_dir(manifest.backup_id).join("manifest.json"), manifest)
    }

    pub fn load_backup_manifest(&self, id: Uuid) -> Result<BackupManifest> {
        self.load_json(&self.backup_dir(id).join("manifest.json"))
    }

    pub fn reconcile_interrupted(&self) -> Result<Vec<OperationRecord>> {
        self.recover_accept_transactions()?;
        let mut changed = Vec::new();
        for mut operation in self.list_operations()? {
            if matches!(
                operation.status,
                super::model::OperationStatus::Queued
                    | super::model::OperationStatus::Running
                    | super::model::OperationStatus::CancelRequested
            ) {
                operation.status = if operation.target_mutation_started {
                    operation.recovery_interlock = true;
                    super::model::OperationStatus::RecoveryRequired
                } else {
                    super::model::OperationStatus::Interrupted
                };
                operation.updated_at = chrono::Utc::now();
                self.save_operation(&operation)?;
                changed.push(operation);
            }
        }
        Ok(changed)
    }

    fn recover_accept_transactions(&self) -> Result<()> {
        let root = self.root.join("transactions");
        if !root.exists() {
            return Ok(());
        }
        for entry in
            fs::read_dir(&root).with_context(|| format!("Failed to read {}", root.display()))?
        {
            let path = entry?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let transaction: AcceptTransaction = self.load_json(&path)?;
            self.save_operation(&transaction.operation)?;
            self.save_action(&transaction.action)?;
            fs::remove_file(path)?;
        }
        Ok(())
    }

    fn action_path(&self, id: Uuid) -> PathBuf {
        self.root.join("actions").join(format!("{id}.json"))
    }

    fn operation_path(&self, id: Uuid) -> PathBuf {
        self.root.join("operations").join(id.to_string()).join("operation.json")
    }

    fn save_json<T: Serialize + ?Sized>(&self, path: &Path, value: &T) -> Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Action storage lock is unavailable"))?;
        self.save_json_locked(path, value)
    }

    fn save_json_locked<T: Serialize + ?Sized>(&self, path: &Path, value: &T) -> Result<()> {
        let parent = path.parent().context("Action storage path has no parent")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
        set_owner_only_directory(parent)?;
        let data = serde_json::to_vec_pretty(value).context("Failed to serialize action data")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("Failed to stage {}", path.display()))?;
        temp.as_file_mut().write_all(&data)?;
        temp.as_file_mut().sync_all()?;
        temp.persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("Failed to persist {}", path.display()))?;
        set_owner_only_file(path)?;
        Ok(())
    }

    fn load_json<T: DeserializeOwned>(&self, path: &Path) -> Result<T> {
        let raw = fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;
        serde_json::from_slice(&raw).with_context(|| format!("Failed to parse {}", path.display()))
    }

    fn load_directory<T: DeserializeOwned>(&self, root: &Path, extension: &str) -> Result<Vec<T>> {
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut values = Vec::new();
        for entry in
            fs::read_dir(root).with_context(|| format!("Failed to read {}", root.display()))?
        {
            let path = entry?.path();
            if path.extension().is_some_and(|value| value == extension) {
                values.push(self.load_json(&path)?);
            }
        }
        Ok(values)
    }
}

#[cfg(unix)]
fn set_owner_only_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_owner_only_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_owner_only_file(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::actions::model::*;

    fn action() -> ProposedAction {
        let target = ConnectionActionSnapshot {
            connection_id: Uuid::new_v4(),
            display_name: "Staging".into(),
            environment: Some("Staging".into()),
            protected: false,
            read_only: false,
            agent_shared: true,
            connected: true,
            identity_hash: "identity".into(),
        };
        ProposedAction {
            version: ACTION_FORMAT_VERSION,
            id: Uuid::new_v4(),
            content: ProposedActionContent {
                request: ActionRequest::DatabaseBackup {
                    connection_id: target.connection_id,
                    database: "app".into(),
                },
                origin: ActionOrigin {
                    kind: ActionOriginKind::Mcp,
                    client_grant_id: Some(Uuid::new_v4()),
                    client_label: Some("Pi".into()),
                    session_id: None,
                },
                policy: ActionPolicySnapshot {
                    version: ACTION_POLICY_VERSION,
                    source_shared: true,
                    target_shared: true,
                    target_writable: true,
                    target_protected: false,
                },
                preview: ActionPreview {
                    summary: "Back up app".into(),
                    source: None,
                    target,
                    source_database: None,
                    target_database: "app".into(),
                    mode: None,
                    estimated_documents: 0,
                    estimated_bytes: 0,
                    warnings: vec![],
                    backup_behavior: "App-managed backup".into(),
                    rollback_behavior: "No database mutation".into(),
                },
                prerequisites: ActionPrerequisites {
                    database_tools_available: true,
                    source_reachable: true,
                    target_reachable: true,
                    backup_storage_available: true,
                    free_space_known_sufficient: None,
                },
                source_state_fingerprint: None,
                target_state_fingerprint: DatabaseStateFingerprint {
                    exists: true,
                    collections: vec![],
                    estimated_documents: 0,
                    estimated_bytes: 0,
                    hash: "state".into(),
                },
            },
            content_hash: "hash".into(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            status: ActionStatus::PendingApproval,
            decision: None,
            operation_id: None,
        }
    }

    #[test]
    fn action_round_trip_is_atomic_and_secret_free() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = ActionStore::new(temp.path().join("agent"));
        let action = action();

        store.save_action(&action).unwrap();
        let restored = store.load_action(action.id).unwrap();

        assert_eq!(restored, action);
        let raw = fs::read_to_string(store.action_path(action.id)).unwrap();
        assert!(!raw.contains("mongodb://"));
    }
}
