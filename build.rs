use std::process::Command;

fn main() {
    // Embed git SHA so the updater can compare nightly builds
    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    println!("cargo:rustc-env=OPENMANGO_GIT_SHA={}", sha.trim());

    // Re-run only when HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");

    // Explorer, the taskbar, and installers read the icon and version from the executable.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=resources/windows/openmango.ico");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("resources/windows/openmango.ico")
            .set("ProductName", "OpenMango")
            .set("FileDescription", "OpenMango")
            .set("LegalCopyright", "GPL-3.0");
        resource.compile().expect("Could not embed the Windows icon and version resources");
    }
}
