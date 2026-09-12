//! Linux AppImage identity and explicit per-user desktop integration.

use std::fs::{self, File};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};

pub const UPDATE_READY_SOCKET: &str = "OPENMANGO_UPDATE_READY_SOCKET";

pub fn appimage_path() -> Result<PathBuf> {
    let image = std::env::var_os("APPIMAGE")
        .context("Run the OpenMango AppImage to use installation and automatic updates")?;
    let directory =
        std::env::var_os("APPDIR").context("The AppImage runtime directory is missing")?;
    let executable = std::env::current_exe()?.canonicalize()?;
    let directory = PathBuf::from(directory).canonicalize()?;
    ensure!(
        executable.starts_with(&directory),
        "This executable is not running inside its AppImage"
    );
    let image = PathBuf::from(image)
        .canonicalize()
        .context("The original AppImage was moved or removed")?;
    let metadata = fs::metadata(&image)?;
    let owner = fs::metadata("/proc/self")?.uid();
    ensure!(
        owner != 0 && metadata.uid() == owner,
        "Use a user-owned AppImage; system installations must be updated by their package manager"
    );
    validate_appimage(&image, std::env::consts::ARCH)?;
    Ok(image)
}

pub fn validate_appimage(path: &Path, architecture: &str) -> Result<()> {
    ensure!(path.is_file(), "The AppImage is not a regular file");
    let mut header = [0_u8; 20];
    File::open(path)?.read_exact(&mut header).context("The AppImage header is incomplete")?;
    ensure!(
        &header[..4] == b"\x7fELF"
            && header[4] == 2
            && header[5] == 1
            && &header[8..11] == b"AI\x02",
        "The update is not a 64-bit type-2 AppImage"
    );
    let machine = u16::from_le_bytes([header[18], header[19]]);
    ensure!(
        matches!((architecture, machine), ("x86_64", 62) | ("aarch64", 183)),
        "The AppImage does not match this computer's architecture"
    );
    Ok(())
}

/// Acknowledge the first window frame before the old app quits.
pub fn notify_update_ready() {
    if let Some(path) = std::env::var_os(UPDATE_READY_SOCKET)
        && let Err(error) =
            UnixStream::connect(path).and_then(|mut stream| stream.write_all(b"ready"))
    {
        log::warn!("Could not acknowledge update startup: {error}");
    }
}

pub fn install_desktop() -> Result<PathBuf> {
    let image = appimage_path()?;
    let data = dirs::data_local_dir().context("Could not determine the user data directory")?;
    let icon = crate::assets::EmbeddedAssets::get("logo/openmango.png")
        .context("The app icon is missing")?;
    install_desktop_at(
        &image,
        &data,
        &icon.data,
        std::env::var_os("APPIMAGE_EXTRACT_AND_RUN").is_some(),
    )
}

fn install_desktop_at(
    image: &Path,
    data: &Path,
    icon: &[u8],
    extract_and_run: bool,
) -> Result<PathBuf> {
    let app_directory = data.join("openmango");
    fs::create_dir_all(&app_directory)?;
    let installed = app_directory.join("OpenMango.AppImage");
    if image.canonicalize()? != installed.canonicalize().unwrap_or_else(|_| installed.clone()) {
        ensure!(
            !installed.exists(),
            "OpenMango is already installed here. Use its updater, or remove the old AppImage before installing a different copy"
        );
        let temporary = tempfile::NamedTempFile::new_in(&app_directory)?;
        fs::copy(image, temporary.path())?;
        temporary.as_file().set_permissions(fs::Permissions::from_mode(0o755))?;
        temporary.as_file().sync_all()?;
        temporary.persist_noclobber(&installed).context("Could not install the AppImage")?;
    }
    let icon_directory = data.join("icons/hicolor/256x256/apps");
    fs::create_dir_all(&icon_directory)?;
    fs::write(icon_directory.join("com.openmango.app.png"), icon)?;
    let applications = data.join("applications");
    fs::create_dir_all(&applications)?;
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=OpenMango\nComment=MongoDB workbench\nExec={}\nIcon=com.openmango.app\nTerminal=false\nCategories=Development;Database;\nStartupNotify=true\nStartupWMClass=com.openmango.app\n",
        if extract_and_run {
            format!("/usr/bin/env APPIMAGE_EXTRACT_AND_RUN=1 {}", desktop_command(&installed)?)
        } else {
            desktop_command(&installed)?
        }
    );
    let mut temporary = tempfile::NamedTempFile::new_in(&applications)?;
    temporary.write_all(entry.as_bytes())?;
    temporary.as_file().set_permissions(fs::Permissions::from_mode(0o644))?;
    temporary.as_file().sync_all()?;
    temporary.persist(applications.join("com.openmango.app.desktop"))?;
    Ok(installed)
}

fn desktop_command(path: &Path) -> Result<String> {
    let path = path.to_str().context("Desktop launchers require a UTF-8 installation path")?;
    ensure!(!path.contains(['\n', '\r', '\0']), "Invalid installation path");
    let mut quoted = String::from("\"");
    for ch in path.chars() {
        match ch {
            '%' => quoted.push_str("%%"),
            '\\' | '"' | '`' | '$' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    // Desktop Entry string escaping is decoded before Exec argument escaping.
    Ok(quoted.replace('\\', "\\\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_install_is_relocatable_and_never_overwrites_another_copy() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("download.AppImage");
        fs::write(&source, "current").unwrap();
        let data = root.path().join("My ფაილები % data");
        let installed = install_desktop_at(&source, &data, b"icon", false).unwrap();
        assert_eq!(fs::read_to_string(&installed).unwrap(), "current");
        let entry =
            fs::read_to_string(data.join("applications/com.openmango.app.desktop")).unwrap();
        assert!(entry.contains("%% data"));
        assert!(install_desktop_at(&source, &data, b"icon", false).is_err());
        install_desktop_at(&installed, &data, b"icon", true).unwrap();
        let entry =
            fs::read_to_string(data.join("applications/com.openmango.app.desktop")).unwrap();
        assert!(entry.contains("Exec=/usr/bin/env APPIMAGE_EXTRACT_AND_RUN=1 \""));
        assert!(desktop_command(Path::new("/tmp/evil\nExec=bad")).is_err());
    }

    #[test]
    fn appimage_validation_rejects_the_wrong_cpu_and_non_appimages() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("test.AppImage");
        let mut header = [0_u8; 20];
        header[..4].copy_from_slice(b"\x7fELF");
        header[4] = 2;
        header[5] = 1;
        header[8..11].copy_from_slice(b"AI\x02");
        header[18..20].copy_from_slice(&183_u16.to_le_bytes());
        fs::write(&image, header).unwrap();
        validate_appimage(&image, "aarch64").unwrap();
        assert!(validate_appimage(&image, "x86_64").is_err());
        header[8] = 0;
        fs::write(&image, header).unwrap();
        assert!(validate_appimage(&image, "aarch64").is_err());
    }
}
