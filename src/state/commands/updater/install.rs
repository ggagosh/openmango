use crate::state::app_state::updater::DownloadedUpdate;
#[cfg(not(any(target_os = "linux", windows)))]
use anyhow::bail;
use anyhow::{Context as _, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
use windows as platform;

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
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

/// Copy the verified download into place, rehashing it so a changed file is rejected.
#[cfg(any(target_os = "linux", windows))]
pub(super) fn copy_verified(source: &Path, target: &Path, size: u64, digest: &str) -> Result<()> {
    use sha2::{Digest as _, Sha256};
    use std::io::{Read as _, Write as _};

    let mut input =
        fs::File::open(source).context("The downloaded update is no longer available")?;
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o700);
    let mut output = options.open(target)?;
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        copied = copied.saturating_add(count as u64);
        anyhow::ensure!(copied <= size, "The staged update is larger than its authenticated size");
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
    }
    anyhow::ensure!(
        copied == size && format!("{:x}", hasher.finalize()) == digest,
        "The downloaded update changed after verification. Download it again"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        output.set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    output.sync_all()?;
    Ok(())
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
