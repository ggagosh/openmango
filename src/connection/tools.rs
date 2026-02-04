//! External tool/runtime path detection and execution.

use std::path::PathBuf;

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

/// Find the path to a bundled tool or runtime.
pub fn node_path() -> Option<PathBuf> {
    find_bundled_tool("node")
}

fn find_bundled_tool(name: &str) -> Option<PathBuf> {
    // 1. Check app bundle (macOS)
    #[cfg(target_os = "macos")]
    {
        if let Ok(exe_path) = std::env::current_exe() {
            // In app bundle: ../Resources/bin/mongodump
            if let Some(parent) = exe_path.parent() {
                let bundle_path = parent.join("../Resources/bin").join(name);
                if bundle_path.exists() && is_executable(&bundle_path) {
                    return Some(bundle_path);
                }
            }
        }
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
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64")
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
        std::fs::metadata(path).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}
