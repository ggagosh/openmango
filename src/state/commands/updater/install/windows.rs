//! A running executable cannot replace itself on Windows. The verified installer
//! runs from a hidden PowerShell step once OpenMango has exited, then reopens it.

use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context as _, Result, ensure};
use base64::Engine as _;

use super::copy_verified;
use crate::state::app_state::updater::DownloadedUpdate;

pub(super) struct PreparedInstall {
    staging: tempfile::TempDir,
    installer: PathBuf,
    executable: PathBuf,
}

pub(super) fn running_installation() -> Result<PathBuf> {
    super::super::release::signed::public_key()?;
    let executable = std::env::current_exe().context("Could not locate the running application")?;
    installation_for_executable(&executable)
        .context("Install OpenMango with the Windows installer to use automatic updates")
}

/// Installer-managed copies have Inno Setup's uninstaller beside the executable.
fn installation_for_executable(executable: &Path) -> Option<PathBuf> {
    let directory = executable.parent()?;
    directory.join("unins000.exe").is_file().then(|| directory.to_path_buf())
}

pub(super) fn prepare(download: &DownloadedUpdate) -> Result<PreparedInstall> {
    let manifest = download
        .release
        .signed_manifest
        .as_ref()
        .context("The update has no authenticated Windows metadata")?;
    running_installation()?;
    let executable = std::env::current_exe()?;
    let staging = tempfile::Builder::new().prefix("openmango-update-").tempdir()?;
    // The signed manifest fixes this name to OpenMango-<version>-windows-<arch>-setup.exe.
    let installer = staging.path().join(&manifest.filename);
    copy_verified(&download.archive, &installer, manifest.size, &manifest.sha256)?;
    validate_installer(&installer)?;
    Ok(PreparedInstall { staging, installer, executable })
}

fn validate_installer(path: &Path) -> Result<()> {
    let mut file = File::open(path)?;
    let mut header = [0_u8; 64];
    file.read_exact(&mut header).context("The installer is incomplete")?;
    ensure!(&header[..2] == b"MZ", "The update is not a Windows installer");
    let offset = u32::from_le_bytes([header[60], header[61], header[62], header[63]]);
    let mut signature = [0_u8; 4];
    file.seek(SeekFrom::Start(offset.into()))?;
    file.read_exact(&mut signature).context("The installer is incomplete")?;
    ensure!(signature == *b"PE\0\0", "The update is not a Windows installer");
    Ok(())
}

pub(super) fn activate_and_restart(prepared: PreparedInstall) -> Result<()> {
    let script = update_script(
        std::process::id(),
        &prepared.installer,
        &prepared.executable,
        prepared.staging.path(),
    );
    let system_root = std::env::var_os("SystemRoot").context("SystemRoot is not set")?;
    let powershell =
        Path::new(&system_root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    crate::connection::tools::tool_command(powershell)
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand"])
        .arg(encode_command(&script))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Could not start the update installer")?;
    // The script owns the staged installer from here and removes it after a successful install.
    let _ = prepared.staging.keep();
    Ok(())
}

// ponytail: a failed install reopens the previous version and keeps install.log in the
// staging folder; surface that in the app if silent failures turn out to matter.
fn update_script(pid: u32, installer: &Path, executable: &Path, staging: &Path) -> String {
    let quote = |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "''"));
    let (installer, executable, staging) = (quote(installer), quote(executable), quote(staging));
    format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         Wait-Process -Id {pid} -Timeout 60\n\
         $log = Join-Path {staging} 'install.log'\n\
         $setup = Start-Process -FilePath {installer} -PassThru -Wait -ArgumentList \
         '/SILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/NOCANCEL', ('/LOG=\"' + $log + '\"')\n\
         Start-Process -FilePath {executable}\n\
         if ($setup.ExitCode -eq 0) {{ Remove-Item -LiteralPath {staging} -Recurse -Force }}\n"
    )
}

/// PowerShell's -EncodedCommand takes base64 UTF-16LE, which avoids command-line quoting.
fn encode_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_installer_managed_copies_update_themselves() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("OpenMango.exe");
        std::fs::write(&executable, b"app").unwrap();
        assert!(installation_for_executable(&executable).is_none());
        std::fs::write(root.path().join("unins000.exe"), b"uninstaller").unwrap();
        assert_eq!(installation_for_executable(&executable).unwrap(), root.path());
    }

    #[test]
    fn installer_must_be_a_portable_executable() {
        let root = tempfile::tempdir().unwrap();
        let installer = root.path().join("setup.exe");
        let mut image = vec![0_u8; 128];
        image[..2].copy_from_slice(b"MZ");
        image[60..64].copy_from_slice(&64_u32.to_le_bytes());
        image[64..68].copy_from_slice(b"PE\0\0");
        std::fs::write(&installer, &image).unwrap();
        validate_installer(&installer).unwrap();
        image[64] = b'X';
        std::fs::write(&installer, &image).unwrap();
        assert!(validate_installer(&installer).is_err());
        std::fs::write(&installer, b"MZ").unwrap();
        assert!(validate_installer(&installer).is_err());
    }

    #[test]
    fn script_quotes_paths_and_waits_for_this_process() {
        let script = update_script(
            4242,
            Path::new(
                r"C:\Users\O'Neil\Temp\openmango-update-1\OpenMango-0.2.2-windows-x86_64-setup.exe",
            ),
            Path::new(r"C:\Users\O'Neil\AppData\Local\Programs\OpenMango\OpenMango.exe"),
            Path::new(r"C:\Users\O'Neil\Temp\openmango-update-1"),
        );
        assert!(script.contains("Wait-Process -Id 4242"));
        assert!(
            script.contains(r"'C:\Users\O''Neil\AppData\Local\Programs\OpenMango\OpenMango.exe'")
        );
        assert!(!script.contains(r"O'Neil"));
        let decoded =
            base64::engine::general_purpose::STANDARD.decode(encode_command(&script)).unwrap();
        let units: Vec<u16> =
            decoded.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        assert_eq!(String::from_utf16(&units).unwrap(), script);
    }
}
