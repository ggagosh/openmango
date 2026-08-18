use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use tempfile::{NamedTempFile, TempDir};

use super::crypto::HistoryCipher;
use super::model::OperationId;

pub(crate) struct TemporaryArchive {
    _directory: TempDir,
    path: PathBuf,
}

impl TemporaryArchive {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

pub(crate) struct SnapshotArtifacts {
    root: PathBuf,
}

impl SnapshotArtifacts {
    pub(crate) fn open(history_path: &Path) -> Result<Self> {
        let parent = history_path.parent().context("operation history path has no parent")?;
        let root = parent.join("operation-artifacts");
        fs::create_dir_all(&root).context("could not create history artifact directory")?;
        set_permissions(&root, 0o700)?;
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with(".openmango-history-") {
                if entry.path().is_dir() {
                    fs::remove_dir_all(entry.path())?;
                } else {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        Ok(Self { root })
    }

    pub(crate) fn temporary_archive(&self) -> Result<TemporaryArchive> {
        let directory = tempfile::Builder::new()
            .prefix(".openmango-history-")
            .tempdir_in(&self.root)
            .context("could not create history artifact staging directory")?;
        set_permissions(directory.path(), 0o700)?;
        let path = directory.path().join("snapshot.archive");
        Ok(TemporaryArchive { _directory: directory, path })
    }

    pub(crate) fn store(
        &self,
        cipher: &HistoryCipher,
        operation_id: OperationId,
        source: &Path,
    ) -> Result<()> {
        let mut staged = NamedTempFile::new_in(&self.root)
            .context("could not stage encrypted history artifact")?;
        cipher.encrypt_artifact(operation_id, source, staged.as_file_mut())?;
        staged.as_file_mut().sync_all()?;
        let path = self.path(operation_id);
        staged
            .persist(&path)
            .map_err(|error| error.error)
            .context("could not persist encrypted history artifact")?;
        set_permissions(&path, 0o600)
    }

    pub(crate) fn decrypt(
        &self,
        cipher: &HistoryCipher,
        operation_id: OperationId,
        destination: &Path,
    ) -> Result<()> {
        cipher.decrypt_artifact(operation_id, &self.path(operation_id), destination)
    }

    pub(crate) fn exists(&self, operation_id: OperationId) -> bool {
        self.path(operation_id).is_file()
    }

    pub(crate) fn remove(&self, operation_id: OperationId) {
        let _ = fs::remove_file(self.path(operation_id));
    }

    fn path(&self, operation_id: OperationId) -> PathBuf {
        self.root.join(format!("{operation_id}.archive.enc"))
    }
}

#[cfg(unix)]
fn set_permissions(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_permissions(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}
