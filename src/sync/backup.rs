use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::actions::ActionStore;
use crate::actions::model::{BackupFile, BackupManifest};
use crate::connection::ConnectionManager;
use crate::connection::types::{
    BsonOutputFormat, BsonToolProgress, BsonToolRunOutcome, CancellationToken,
};
use crate::sync::plan::RuntimeActionConnection;

pub fn create_verified_backup(
    manager: &ConnectionManager,
    store: &ActionStore,
    connection: &RuntimeActionConnection,
    database: &str,
    operation_id: Uuid,
    cancellation: CancellationToken,
    progress: Arc<dyn Fn(BsonToolProgress) + Send + Sync>,
) -> Result<BackupManifest, String> {
    let backup_id = Uuid::new_v4();
    let backup_dir = store.backup_dir(backup_id);
    let archive_path = backup_dir.join("dump.archive");
    let mut collections = if connection.databases.iter().any(|candidate| candidate == database) {
        list_collection_names(manager, connection, database)?
    } else {
        Vec::new()
    };
    collections.sort();
    let absent = !connection.databases.iter().any(|candidate| candidate == database);
    let started_at = Utc::now();
    let mut manifest = BackupManifest {
        version: 1,
        backup_id,
        operation_id,
        connection_identity_hash: connection.snapshot.identity_hash.clone(),
        database: database.to_string(),
        started_at,
        completed_at: None,
        tools_version: database_tools_version(),
        process_succeeded: false,
        preflight_collections: collections.clone(),
        files: Vec::new(),
        file_count: 0,
        byte_count: 0,
        absence_marker: absent,
        verified: false,
        warnings: Vec::new(),
    };
    store.save_backup_manifest(&manifest).map_err(safe_error)?;
    if absent {
        manifest.process_succeeded = true;
        manifest.verified = true;
        manifest.completed_at = Some(Utc::now());
        store.save_backup_manifest(&manifest).map_err(safe_error)?;
        return Ok(manifest);
    }

    fs::create_dir_all(&backup_dir).map_err(safe_error)?;
    let callback = move |event| progress(event);
    match manager
        .export_database_bson_with_progress(
            &connection.tool_uri,
            database,
            BsonOutputFormat::Archive,
            &archive_path,
            false,
            &[],
            cancellation,
            callback,
        )
        .map_err(safe_error)?
    {
        BsonToolRunOutcome::Completed => manifest.process_succeeded = true,
        BsonToolRunOutcome::Cancelled { termination_succeeded: true } => {
            return Err("backup_cancelled".into());
        }
        BsonToolRunOutcome::Cancelled { termination_succeeded: false } => {
            return Err("backup_cancellation_unconfirmed".into());
        }
    }
    verify_archive(&mut manifest, &archive_path, database)?;
    manifest.completed_at = Some(Utc::now());
    store.save_backup_manifest(&manifest).map_err(safe_error)?;
    Ok(manifest)
}

pub struct SourceDump {
    pub path: PathBuf,
    pub collections: Vec<String>,
}

pub fn create_verified_source_dump(
    manager: &ConnectionManager,
    store: &ActionStore,
    connection: &RuntimeActionConnection,
    database: &str,
    operation_id: Uuid,
    cancellation: CancellationToken,
    progress: Arc<dyn Fn(BsonToolProgress) + Send + Sync>,
) -> Result<SourceDump, String> {
    let root = store.source_dump_dir(operation_id);
    fs::create_dir_all(&root).map_err(safe_error)?;
    let archive_path = root.join("dump.archive");
    let mut collections = list_collection_names(manager, connection, database)?;
    collections.sort();
    let callback = move |event| progress(event);
    match manager
        .export_database_bson_with_progress(
            &connection.tool_uri,
            database,
            BsonOutputFormat::Archive,
            &archive_path,
            false,
            &[],
            cancellation,
            callback,
        )
        .map_err(safe_error)?
    {
        BsonToolRunOutcome::Completed => {}
        BsonToolRunOutcome::Cancelled { termination_succeeded: true } => {
            return Err("source_dump_cancelled".into());
        }
        BsonToolRunOutcome::Cancelled { termination_succeeded: false } => {
            return Err("source_dump_cancellation_unconfirmed".into());
        }
    }
    verify_archive_namespaces(&archive_path, database, &collections)?;
    Ok(SourceDump { path: archive_path, collections })
}

pub fn backup_payload_path(store: &ActionStore, manifest: &BackupManifest) -> PathBuf {
    let backup_dir = store.backup_dir(manifest.backup_id);
    let archive = backup_dir.join("dump.archive");
    if archive.exists() { archive } else { backup_dir.join("dump") }
}

fn list_collection_names(
    manager: &ConnectionManager,
    connection: &RuntimeActionConnection,
    database: &str,
) -> Result<Vec<String>, String> {
    manager
        .runtime_handle()
        .block_on(async { connection.client.database(database).list_collection_names().await })
        .map_err(safe_error)
}

fn verify_archive(
    manifest: &mut BackupManifest,
    archive_path: &Path,
    database: &str,
) -> Result<(), String> {
    verify_archive_namespaces(archive_path, database, &manifest.preflight_collections)?;
    let bytes = fs::metadata(archive_path).map_err(safe_error)?.len();
    manifest.file_count = 1;
    manifest.byte_count = bytes;
    manifest.files = vec![BackupFile { relative_path: "dump.archive".into(), bytes }];
    manifest.verified = manifest.process_succeeded;
    Ok(())
}

fn verify_archive_namespaces(
    archive_path: &Path,
    database: &str,
    collections: &[String],
) -> Result<(), String> {
    if !archive_path.is_file() || fs::metadata(archive_path).map_err(safe_error)?.len() == 0 {
        return Err("Backup archive is missing or empty".into());
    }
    let mongorestore = crate::connection::tools::mongorestore_path()
        .ok_or_else(|| "MongoDB restore tool is unavailable".to_string())?;
    let output = std::process::Command::new(mongorestore)
        .arg("-v")
        .arg("--dryRun")
        .arg(format!("--archive={}", archive_path.display()))
        .output()
        .map_err(safe_error)?;
    if !output.status.success() {
        return Err("Backup archive validation failed".into());
    }
    let dry_run = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    verify_dry_run_namespaces(&dry_run, database, collections)
}

fn verify_dry_run_namespaces(
    dry_run: &str,
    database: &str,
    collections: &[String],
) -> Result<(), String> {
    for collection in collections {
        let namespace = format!("{database}.{collection}");
        let quoted = format!("found collection `{namespace}` bson");
        let unquoted = format!("found collection {namespace} bson");
        if !dry_run.contains(&quoted) && !dry_run.contains(&unquoted) {
            return Err(format!("Backup is missing namespace {collection}"));
        }
    }
    Ok(())
}

fn database_tools_version() -> Option<String> {
    let path = crate::connection::tools::mongodump_path()?;
    let output = std::process::Command::new(path).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().next().map(|line| line.chars().take(120).collect())
}

fn safe_error(_error: impl std::fmt::Display) -> String {
    "Database backup operation failed; check OpenMango logs".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_verification_preserves_case_distinct_namespaces() {
        let quoted = "found collection `test.dataTypes` bson to restore to `test.dataTypes`\n\
                      found collection `test.datatypes` bson to restore to `test.datatypes`";
        let unquoted = "found collection test.dataTypes bson to restore to test.dataTypes\n\
                        found collection test.datatypes bson to restore to test.datatypes";
        let collections = vec!["dataTypes".into(), "datatypes".into()];

        assert!(verify_dry_run_namespaces(quoted, "test", &collections).is_ok());
        assert!(verify_dry_run_namespaces(unquoted, "test", &collections).is_ok());
        assert!(
            verify_dry_run_namespaces(quoted.lines().next().unwrap(), "test", &collections)
                .is_err()
        );
    }
}
