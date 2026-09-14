//! External tool/runtime path detection and execution.

use std::path::{Path, PathBuf};

/// Check if mongodump/mongorestore tools are available.
pub fn tools_available() -> bool {
    mongodump_path().is_some() && mongorestore_path().is_some()
}

/// Find the path to mongodump executable.
pub fn mongodump_path() -> Option<PathBuf> {
    find_bundled_tool("mongodump")
}

/// Find the path to mongorestore executable.
pub fn mongorestore_path() -> Option<PathBuf> {
    find_bundled_tool("mongorestore")
}

/// Find the path to the compiled mongosh sidecar binary.
pub fn mongosh_sidecar_path() -> Option<PathBuf> {
    find_bundled_tool("mongosh-sidecar")
}

fn find_bundled_tool(name: &str) -> Option<PathBuf> {
    // Packaged tools are relative to the executable, never the launch directory.
    if let Ok(executable) = std::env::current_exe()
        && let Some(path) = packaged_tool_path(&executable, name, std::env::consts::OS)
        && is_executable(&path)
    {
        return Some(path);
    }

    // 2. Check resources/bin (dev mode) with architecture-specific paths
    let arch_dir = dev_tools_arch();
    let dev_path = PathBuf::from("resources/bin")
        .join(arch_dir)
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    if dev_path.exists() && is_executable(&dev_path) {
        return Some(dev_path);
    }

    // 3. Check PATH
    which::which(name).ok()
}

fn packaged_tool_path(executable: &Path, name: &str, os: &str) -> Option<PathBuf> {
    let (directory, file) = match os {
        "macos" => ("../Resources/bin", name.to_string()),
        "linux" => ("../lib/openmango/bin", name.to_string()),
        "windows" => ("bin", format!("{name}.exe")),
        _ => return None,
    };
    Some(executable.parent()?.join(directory).join(file))
}

/// A command for a bundled helper. On Windows it runs without opening a console window.
pub fn tool_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Get the architecture-specific directory name for dev mode tools
fn dev_tools_arch() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x86_64",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-arm64",
        ("windows", "x86_64") => "windows-x86_64",
        ("windows", "aarch64") => "windows-arm64",
        _ => "unknown",
    }
}

/// Check if a path is executable
fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_tools_are_relocatable_and_architecture_independent() {
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("Mango ფაილები.AppDir");
        let executable = bundle.join("usr/bin/openmango");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::create_dir_all(bundle.join("usr/lib/openmango/bin")).unwrap();
        let tool = bundle.join("usr/lib/openmango/bin/mongosh-sidecar");
        std::fs::write(&tool, "test").unwrap();
        let resolved = packaged_tool_path(&executable, "mongosh-sidecar", "linux").unwrap();
        assert_eq!(resolved.canonicalize().unwrap(), tool.canonicalize().unwrap());
        assert!(resolved.is_absolute());
        assert!(!is_executable(&bundle));
        assert_eq!(
            packaged_tool_path(
                Path::new("/Applications/OpenMango.app/Contents/MacOS/OpenMango"),
                "mongodump",
                "macos"
            )
            .unwrap(),
            Path::new("/Applications/OpenMango.app/Contents/MacOS/../Resources/bin/mongodump")
        );
        assert_eq!(
            packaged_tool_path(
                Path::new("C:/Apps/OpenMango/OpenMango.exe"),
                "mongodump",
                "windows"
            )
            .unwrap(),
            Path::new("C:/Apps/OpenMango").join("bin").join("mongodump.exe")
        );
    }
}
