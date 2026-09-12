use crate::state::app_state::updater::DownloadedUpdate;
#[cfg(not(target_os = "linux"))]
use anyhow::bail;
use anyhow::{Context as _, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    use super::*;
    pub(super) struct PreparedInstall;
    pub(super) fn running_installation() -> Result<PathBuf> {
        bail!("Automatic installation is unavailable on this platform")
    }
    pub(super) fn prepare(_: &DownloadedUpdate) -> Result<PreparedInstall> {
        bail!("Automatic installation is unavailable on this platform")
    }
    pub(super) fn activate_and_restart(_: PreparedInstall) -> Result<()> {
        bail!("Automatic installation is unavailable on this platform")
    }
}

pub(super) struct PreparedInstall(platform::PreparedInstall);
pub(super) fn running_installation() -> Result<PathBuf> {
    platform::running_installation()
}
pub(super) fn prepare(download: &DownloadedUpdate) -> Result<PreparedInstall> {
    platform::prepare(download).map(PreparedInstall)
}
pub(super) fn activate_and_restart(prepared: PreparedInstall) -> Result<()> {
    platform::activate_and_restart(prepared.0)
}

#[cfg(target_os = "linux")]
pub(super) fn replace_path(target: &Path, extracted: &Path, backup: &Path) -> Result<()> {
    // Preserve the old inode before atomically replacing the directory entry.
    // A failed backup leaves the installed image untouched.
    fs::hard_link(target, backup)
        .context("Could not back up the AppImage; use a filesystem supporting hard links")?;
    fs::File::open(backup)?.sync_all()?;
    fs::File::open(backup.parent().context("The backup has no directory")?)?.sync_all()?;
    fs::rename(extracted, target)
        .context("Could not replace the AppImage; the previous image is still installed")
}

#[cfg(target_os = "macos")]
pub(super) fn replace_path(target: &Path, extracted: &Path, backup: &Path) -> Result<()> {
    fs::rename(target, backup).context("Could not back up the current application")?;
    if let Err(error) = fs::rename(extracted, target) {
        restore_path(target, extracted, backup)
            .with_context(|| format!("Could not install the update ({error}) and could not restore the previous application"))?;
        bail!("Could not install the update; the previous application was restored: {error}");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn restore_path(target: &Path, _: &Path, backup: &Path) -> Result<()> {
    fs::rename(backup, target).context("Could not restore the previous AppImage")?;
    fs::File::open(target.parent().context("The AppImage has no directory")?)?.sync_all()?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn restore_path(target: &Path, extracted: &Path, backup: &Path) -> Result<()> {
    if target.exists() {
        fs::rename(target, extracted)
            .context("Could not move the failed update out of the installation folder")?;
    }
    fs::rename(backup, target).context("Could not restore the previous application")
}
