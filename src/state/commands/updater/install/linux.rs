use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use sha2::{Digest as _, Sha256};

use crate::helpers::linux::{UPDATE_READY_SOCKET, appimage_path, validate_appimage};
use crate::state::app_state::updater::DownloadedUpdate;

pub(super) struct PreparedInstall {
    staging: tempfile::TempDir,
    target: PathBuf,
    replacement: PathBuf,
    original: Metadata,
    _lock: File,
}

pub(super) fn running_installation() -> Result<PathBuf> {
    super::super::release::linux::public_key()?;
    appimage_path()
}

pub(super) fn prepare(download: &DownloadedUpdate) -> Result<PreparedInstall> {
    let manifest = download
        .release
        .linux_manifest
        .as_ref()
        .context("The update has no authenticated Linux metadata")?;
    let target = running_installation()?;
    let parent = target.parent().context("The AppImage has no installation directory")?;
    let name = target.file_name().context("The AppImage has no filename")?.to_string_lossy();
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
        .open(parent.join(format!(".{name}.update.lock")))
        .context("The AppImage directory is not writable. Move it to a user-owned folder before updating")?;
    lock.try_lock().context("Another OpenMango instance is installing an update")?;
    let original = fs::metadata(&target)?;
    let staging = tempfile::Builder::new().prefix(".openmango-update-").tempdir_in(parent)?;
    let replacement = staging.path().join("replacement.AppImage");
    copy_verified(&download.archive, &replacement, manifest.size, &manifest.sha256)?;
    validate_appimage(&replacement, &manifest.arch)?;
    Ok(PreparedInstall { staging, target, replacement, original, _lock: lock })
}

fn copy_verified(source: &Path, target: &Path, size: u64, digest: &str) -> Result<()> {
    let mut input = File::open(source).context("The downloaded update is no longer available")?;
    let mut output = OpenOptions::new().create_new(true).write(true).mode(0o700).open(target)?;
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        copied = copied.saturating_add(count as u64);
        ensure!(copied <= size, "The staged update is larger than its authenticated size");
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
    }
    ensure!(
        copied == size && format!("{:x}", hasher.finalize()) == digest,
        "The downloaded update changed after verification. Download it again"
    );
    output.set_permissions(fs::Permissions::from_mode(0o755))?;
    output.sync_all()?;
    Ok(())
}

pub(super) fn activate_and_restart(prepared: PreparedInstall) -> Result<()> {
    let current = fs::metadata(&prepared.target).context("The installed AppImage was removed")?;
    ensure!(
        current.dev() == prepared.original.dev()
            && current.ino() == prepared.original.ino()
            && current.len() == prepared.original.len()
            && current.modified()? == prepared.original.modified()?,
        "The installed AppImage changed while preparing the update. Reopen OpenMango and check again"
    );
    // Keep the Unix socket path short even when the installation path is long.
    let ready_directory = tempfile::Builder::new().prefix("openmango-ready-").tempdir_in("/tmp")?;
    let ready_path = ready_directory.path().join("ready.sock");
    let listener = UnixListener::bind(&ready_path)?;
    listener.set_nonblocking(true)?;
    let backup = prepared.staging.path().join("previous.AppImage");
    if let Err(error) = super::replace_path(&prepared.target, &prepared.replacement, &backup) {
        if backup.exists() {
            let recovery = prepared.staging.keep();
            bail!("{error:#}. The previous AppImage is preserved in {}", recovery.display());
        }
        return Err(error);
    }
    let mut termination_error = None;
    let launch = (|| {
        if let Some(parent) = prepared.target.parent() {
            File::open(parent)?.sync_all()?;
        }
        Command::new(&prepared.target)
            .process_group(0)
            .env_remove("APPDIR")
            .env_remove("APPIMAGE")
            .env_remove("ARGV0")
            .env(UPDATE_READY_SOCKET, &ready_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Could not launch the updated AppImage")
            .and_then(|mut child| {
                let result = wait_for_window(&mut child, &listener, Duration::from_secs(20));
                if result.is_err() {
                    termination_error = terminate_failed_startup(&mut child).err();
                }
                result
            })
    })();
    if let Some(error) = termination_error {
        let recovery = prepared.staging.keep();
        bail!(
            "Could not stop the failed update ({error:#}); close it before retrying. The previous AppImage is preserved in {}",
            recovery.display()
        );
    }
    if let Err(error) = launch {
        if let Err(restore) = super::restore_path(&prepared.target, &prepared.replacement, &backup)
        {
            if !backup.exists() {
                bail!(
                    "The previous AppImage was restored at {}, but its directory could not be synchronized: {restore:#}",
                    prepared.target.display()
                );
            }
            let recovery = prepared.staging.keep();
            bail!(
                "Update startup failed ({error:#}) and rollback failed ({restore:#}). The previous AppImage is preserved in {}",
                recovery.display()
            );
        }
        bail!("The new app did not open; the previous AppImage was restored: {error:#}");
    }
    if let Err(error) = prepared.staging.close() {
        log::warn!("Linux update installed, but its backup could not be cleaned up: {error}");
    }
    Ok(())
}

fn terminate_failed_startup(child: &mut Child) -> Result<()> {
    let group: i32 = child.id().try_into().context("Invalid update process ID")?;
    // The AppImage runtime forks AppRun in extraction mode. Signal the private
    // group created above so rollback cannot leave that replacement app running.
    // SAFETY: kill takes integer arguments; the positive child ID names our group.
    if unsafe { libc::kill(-group, libc::SIGKILL) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error).context("Could not terminate the update process group");
        }
    }
    child.wait().context("Could not reap the failed update launcher")?;
    Ok(())
}

fn wait_for_window(child: &mut Child, listener: &UnixListener, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            bail!("The updated app exited before opening a window ({status})");
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_millis(250)))?;
                let mut response = [0_u8; 5];
                if stream.read_exact(&mut response).is_ok() && response == *b"ready" {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    bail!("The updated app did not acknowledge its first window within 20 seconds")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead as _;

    #[test]
    fn failed_startup_stops_the_launcher_and_its_child() {
        let mut launcher = Command::new("/bin/sh")
            .args(["-c", "sleep 60 & echo $!; wait"])
            .process_group(0)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut pid = String::new();
        std::io::BufReader::new(launcher.stdout.take().unwrap()).read_line(&mut pid).unwrap();
        let pid: i32 = pid.trim().parse().unwrap();
        let result = terminate_failed_startup(&mut launcher);
        let deadline = Instant::now() + Duration::from_secs(2);
        let stopped = loop {
            let status = fs::read_to_string(format!("/proc/{pid}/stat"));
            if status.as_ref().is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                || status.as_ref().is_ok_and(|stat| {
                    stat.rsplit_once(") ").is_some_and(|(_, rest)| rest.starts_with('Z'))
                })
            {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        // Also clean up when the regression fails, so tests never leave a sleeper.
        // SAFETY: this PID came from our wrapper's child, not an external process.
        if !stopped {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        let _ = launcher.kill();
        let _ = launcher.wait();
        result.unwrap();
        assert!(stopped, "The AppImage child survived termination of its launcher");
    }

    #[test]
    fn staged_bytes_are_reverified_and_a_failed_swap_restores_the_original() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("download");
        fs::write(&source, b"verified bytes").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"verified bytes"));
        let installed = root.path().join("OpenMango.AppImage");
        copy_verified(&source, &installed, 14, &digest).unwrap();
        assert!(copy_verified(&source, &root.path().join("bad"), 14, &"a".repeat(64)).is_err());
        let backup = root.path().join("backup");
        assert!(
            super::super::replace_path(&installed, &root.path().join("missing"), &backup).is_err()
        );
        assert_eq!(fs::read(&installed).unwrap(), b"verified bytes");
        assert_eq!(fs::read(backup).unwrap(), b"verified bytes");
    }

    #[test]
    fn early_child_exit_is_not_successful_update_startup() {
        let root = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(root.path().join("ready")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut child = Command::new("/bin/false").spawn().unwrap();
        assert!(wait_for_window(&mut child, &listener, Duration::from_secs(2)).is_err());
        let _ = child.wait();
    }

    #[test]
    fn readiness_acknowledgement_completes_startup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("ready");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let sender = std::thread::spawn(move || {
            std::os::unix::net::UnixStream::connect(path).unwrap().write_all(b"ready").unwrap();
        });
        let mut child = Command::new("/bin/sleep").arg("5").spawn().unwrap();
        let result = wait_for_window(&mut child, &listener, Duration::from_secs(2));
        let _ = child.kill();
        let _ = child.wait();
        sender.join().unwrap();
        result.unwrap();
    }

    #[test]
    fn launch_failure_rolls_back_the_installed_appimage() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("OpenMango.AppImage");
        fs::write(&target, b"old app").unwrap();
        let staging = tempfile::tempdir_in(root.path()).unwrap();
        let replacement = staging.path().join("new");
        fs::write(&replacement, b"not an executable").unwrap();
        let original = fs::metadata(&target).unwrap();
        let lock = File::create(root.path().join("lock")).unwrap();
        let prepared =
            PreparedInstall { staging, target: target.clone(), replacement, original, _lock: lock };
        assert!(activate_and_restart(prepared).unwrap_err().to_string().contains("restored"));
        assert_eq!(fs::read(target).unwrap(), b"old app");
    }
}
