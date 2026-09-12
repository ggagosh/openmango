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
    let dev_path = PathBuf::from("resources/bin").join(arch_dir).join(name);
    if dev_path.exists() && is_executable(&dev_path) {
        return Some(dev_path);
    }

    // 3. Check PATH
    which::which(name).ok()
}

fn packaged_tool_path(executable: &Path, name: &str, os: &str) -> Option<PathBuf> {
    let directory = match os {
        "macos" => "../Resources/bin",
        "linux" => "../lib/openmango/bin",
        _ => return None,
    };
    Some(executable.parent()?.join(directory).join(name))
}

/// Get the architecture-specific directory name for dev mode tools
fn dev_tools_arch() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-arm64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "linux-arm64"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64")
    )))]
    {
        "unknown"
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
    }
}
