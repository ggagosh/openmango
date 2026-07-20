//! Collection and database export operations (JSON, CSV).
//!
//! This module provides export functionality for MongoDB collections and databases:
//! - JSON/JSONL export with various options (pretty print, gzip, extended JSON modes)
//! - CSV export with automatic column detection
//! - Database-wide export (all collections)
//! - Progress callbacks for large exports

mod csv;
mod excel;
mod json;
pub mod report_excel;

use std::fs::File;
use std::path::{Path, PathBuf};

use mongodb::Client;
use mongodb::bson::Bson;

use crate::connection::ConnectionManager;
use crate::connection::types::ExtendedJsonMode;
use crate::error::Result;

pub(crate) struct AtomicExportFile {
    temporary: tempfile::NamedTempFile,
    destination: PathBuf,
}

impl AtomicExportFile {
    pub(crate) fn new(destination: &Path) -> Result<Self> {
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let temporary =
            tempfile::Builder::new().prefix(".openmango-export-").tempfile_in(parent)?;
        Ok(Self { temporary, destination: destination.to_path_buf() })
    }

    pub(crate) fn reopen(&self) -> Result<File> {
        Ok(self.temporary.reopen()?)
    }

    pub(crate) fn temporary_path(&self) -> &Path {
        self.temporary.path()
    }

    pub(crate) fn commit(mut self) -> Result<()> {
        self.temporary.as_file_mut().sync_all()?;
        self.temporary
            .persist(&self.destination)
            .map_err(|error| crate::error::Error::Io(error.error))?;
        Ok(())
    }
}

fn remove_export_path(path: &Path) -> std::io::Result<()> {
    if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }
}

pub(crate) fn promote_export_path(staged: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    if !destination.exists() {
        std::fs::rename(staged, destination)?;
        return Ok(());
    }

    let backup = parent.join(format!(".openmango-export-backup-{}", uuid::Uuid::new_v4()));
    std::fs::rename(destination, &backup)?;
    if let Err(error) = std::fs::rename(staged, destination) {
        let restore_result = std::fs::rename(&backup, destination);
        return match restore_result {
            Ok(()) => Err(error.into()),
            Err(restore_error) => Err(crate::error::Error::Parse(format!(
                "Could not promote export ({error}) or restore prior destination ({restore_error})"
            ))),
        };
    }
    if let Err(error) = remove_export_path(&backup) {
        log::warn!("Export succeeded but old destination cleanup failed: {error}");
    }
    Ok(())
}

pub(crate) fn promote_export_directory(staged: &Path, destination: &Path) -> Result<()> {
    promote_export_path(staged, destination)
}

/// Generate a preview of documents for export.
pub fn generate_export_preview(
    manager: &ConnectionManager,
    client: &Client,
    database: &str,
    collection: &str,
    json_mode: ExtendedJsonMode,
    pretty_print: bool,
    limit: usize,
) -> Result<Vec<String>> {
    let docs = manager.sample_documents(client, database, collection, limit as i64)?;

    let previews: Vec<String> = docs
        .into_iter()
        .map(|doc| {
            let json_value = match json_mode {
                ExtendedJsonMode::Relaxed => Bson::Document(doc).into_relaxed_extjson(),
                ExtendedJsonMode::Canonical => Bson::Document(doc).into_canonical_extjson(),
            };

            if pretty_print {
                serde_json::to_string_pretty(&json_value).unwrap_or_default()
            } else {
                serde_json::to_string(&json_value).unwrap_or_default()
            }
        })
        .collect();

    Ok(previews)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[test]
    fn incomplete_atomic_export_preserves_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("export.json");
        std::fs::write(&destination, "existing").unwrap();

        let staged = AtomicExportFile::new(&destination).unwrap();
        staged.reopen().unwrap().write_all(b"partial").unwrap();
        drop(staged);

        assert_eq!(std::fs::read_to_string(destination).unwrap(), "existing");
    }

    #[test]
    fn committed_atomic_export_replaces_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("export.json");
        std::fs::write(&destination, "existing").unwrap();

        let staged = AtomicExportFile::new(&destination).unwrap();
        let mut file = staged.reopen().unwrap();
        file.write_all(b"complete").unwrap();
        file.flush().unwrap();
        drop(file);
        staged.commit().unwrap();

        assert_eq!(std::fs::read_to_string(destination).unwrap(), "complete");
    }

    #[test]
    fn directory_promotion_replaces_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("destination");
        let staged = directory.path().join("staged");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("old.txt"), "old").unwrap();
        std::fs::create_dir(&staged).unwrap();
        std::fs::write(staged.join("new.txt"), "new").unwrap();

        promote_export_directory(&staged, &destination).unwrap();

        assert!(!destination.join("old.txt").exists());
        assert_eq!(std::fs::read_to_string(destination.join("new.txt")).unwrap(), "new");
    }
}
