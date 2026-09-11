use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use anyhow::{Context as _, Result, bail, ensure};

pub(super) struct PreparedInstall {
    staging: tempfile::TempDir,
    target: PathBuf,
    extracted: PathBuf,
    original_modified: SystemTime,
    _lock: File,
}

pub(super) fn running_bundle() -> Result<PathBuf> {
    ensure!(
        cfg!(target_os = "macos"),
        "Automatic installation is currently available on macOS only"
    );
    let executable = std::env::current_exe().context("Could not locate the running application")?;
    let bundle = bundle_for_executable(&executable)
        .context("This is a development executable. Install OpenMango.app to use in-app updates")?;
    ensure!(
        !bundle.components().any(|part| part.as_os_str() == "AppTranslocation"),
        "Move OpenMango to Applications and reopen it before updating."
    );
    Ok(bundle)
}

fn bundle_for_executable(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension().is_some_and(|extension| extension == "app"))
    .then(|| bundle.to_path_buf())
}

fn designated_requirement(bundle: &Path) -> Result<String> {
    let output = Command::new("/usr/bin/codesign")
        .args(["-d", "-r-"])
        .arg(bundle)
        .output()
        .context("Could not read the installed app's signing identity")?;
    ensure!(output.status.success(), "Could not verify the installed app's signing identity");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let requirement = text
        .lines()
        .find_map(|line| line.strip_prefix("designated => "))
        .context("The installed app has no signing requirement")?
        .to_string();
    ensure!(
        requirement.contains("identifier")
            && requirement.contains("anchor apple generic")
            && requirement.contains("subject.OU"),
        "The installed app has no trusted Developer ID identity. Install a signed release first."
    );
    Ok(requirement)
}

fn verify_signature(bundle: &Path, requirement: &str) -> Result<()> {
    let output = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "--verbose=2"])
        .arg(bundle)
        .output()
        .context("Could not start macOS signature verification")?;
    ensure!(
        output.status.success(),
        "The app signature is invalid: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let identity = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "-R", requirement])
        .arg(bundle)
        .output()
        .context("Could not verify the app's signing identity")?;
    ensure!(
        identity.status.success(),
        "The update's signing identity does not match this application: {}",
        String::from_utf8_lossy(&identity.stderr).trim()
    );
    Ok(())
}

/// Extraction and signature checks do not modify the installed application.
pub(super) fn prepare(archive: &Path) -> Result<PreparedInstall> {
    ensure!(archive.is_file(), "The downloaded update is no longer available. Download it again.");
    let target = running_bundle()?;
    let parent = target.parent().context("The application has no installation folder")?;
    let lock = OpenOptions::new().create(true).truncate(false).write(true)
        .open(parent.join(".openmango-update.lock"))
        .context("The application folder is not writable. Move OpenMango to a writable Applications folder.")?;
    lock.try_lock().context(
        "Another OpenMango instance is preparing an update. Try again after it finishes.",
    )?;
    let original_modified = fs::metadata(&target)
        .context("Could not find the installed app. Reopen OpenMango if it was moved")?
        .modified()
        .context("Could not inspect the installed app's modification time")?;
    let requirement = designated_requirement(&target)?;
    verify_signature(&target, &requirement)?;
    let staging = tempfile::Builder::new()
        .prefix(".openmango-update-")
        .tempdir_in(parent)
        .context("Could not create the installation staging folder")?;
    let payload = staging.path().join("payload");
    fs::create_dir(&payload).context("Could not create the update extraction folder")?;
    let extraction = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(archive)
        .arg(&payload)
        .output()
        .context("Could not start the macOS update extractor")?;
    ensure!(
        extraction.status.success(),
        "Could not extract the update: {}",
        String::from_utf8_lossy(&extraction.stderr).trim()
    );
    let extracted = payload.join("OpenMango.app");
    ensure!(extracted.is_dir(), "The downloaded update does not contain OpenMango.app");
    verify_signature(&extracted, &requirement)?;
    Ok(PreparedInstall { staging, target, extracted, original_modified, _lock: lock })
}

fn replace_bundle(target: &Path, extracted: &Path, backup: &Path) -> Result<()> {
    fs::rename(target, backup).context("Could not back up the current application")?;
    if let Err(error) = fs::rename(extracted, target) {
        restore_bundle(target, extracted, backup)
            .with_context(|| format!("Could not install the update ({error}) and could not restore the previous application"))?;
        bail!("Could not install the update; the previous application was restored: {error}");
    }
    Ok(())
}

fn restore_bundle(target: &Path, extracted: &Path, backup: &Path) -> Result<()> {
    if target.exists() {
        fs::rename(target, extracted)
            .context("Could not move the failed update out of the installation folder")?;
    }
    fs::rename(backup, target).context("Could not restore the previous application")
}

/// Called only after the unsaved-work guard approves restart. The final swap and
/// launch stay in one callback so no new edits can appear between approval and quit.
pub(super) fn activate_and_restart(prepared: PreparedInstall) -> Result<()> {
    let current_modified = fs::metadata(&prepared.target)
        .context("The installed app is no longer available. Reopen OpenMango and try again")?
        .modified()
        .context("Could not recheck the installed application")?;
    ensure!(
        current_modified == prepared.original_modified,
        "The installed application changed while preparing this update. Reopen OpenMango and check again."
    );
    let backup = prepared.staging.path().join("previous.app");
    if let Err(error) = replace_bundle(&prepared.target, &prepared.extracted, &backup) {
        if backup.exists() {
            let recovery = prepared.staging.keep();
            bail!("{error:#}. The previous application is preserved in {}", recovery.display());
        }
        return Err(error);
    }
    let launch = Command::new("/usr/bin/open").arg("-n").arg(&prepared.target).output();
    let error = match launch {
        Ok(output) if output.status.success() => None,
        Ok(output) => Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        Err(error) => Some(error.to_string()),
    };
    if let Some(error) = error {
        if let Err(restore) = restore_bundle(&prepared.target, &prepared.extracted, &backup) {
            let recovery = prepared.staging.keep();
            bail!(
                "The update could not be opened ({error}) or restored ({restore:#}). The previous application is preserved in {}",
                recovery.display()
            );
        }
        bail!("The new app could not be opened. The previous app was restored: {error}");
    }
    // Only this operation's staging area and backup are removed.
    if let Err(error) = prepared.staging.close() {
        log::warn!("Update installed, but its backup could not be cleaned up: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{bundle_for_executable, replace_bundle};
    use std::path::Path;

    #[test]
    fn development_executables_never_target_applications() {
        assert!(bundle_for_executable(Path::new("/work/target/debug/openmango")).is_none());
        assert_eq!(
            bundle_for_executable(Path::new(
                "/Applications/OpenMango.app/Contents/MacOS/OpenMango"
            ))
            .unwrap(),
            Path::new("/Applications/OpenMango.app")
        );
    }

    #[test]
    fn failed_swap_preserves_the_installed_application() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("OpenMango.app");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("original"), "keep").unwrap();
        assert!(
            replace_bundle(
                &target,
                &root.path().join("missing.app"),
                &root.path().join("backup.app")
            )
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(target.join("original")).unwrap(), "keep");
    }
}
