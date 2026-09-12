#!/usr/bin/env bash
# Sourced by release_linux.sh after lib.sh and openmango_platform.

linux_packaging_tools() {
    local cache="$1" arch deploy_sha tool_sha runtime_sha
    case "$OPENMANGO_ARCH_DIR" in
        linux-x86_64)
            arch=x86_64
            deploy_sha=c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d
            tool_sha=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
            runtime_sha=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d ;;
        linux-arm64)
            arch=aarch64
            deploy_sha=620095110d693282b8ebeb244a95b5e911cf8f65f76c88b4b47d16ae6346fcff
            tool_sha=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
            runtime_sha=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444 ;;
        *) echo "Linux packaging requires a Linux target." >&2; return 1 ;;
    esac
    LINUXDEPLOY="$cache/linuxdeploy-$arch.AppImage"
    APPIMAGETOOL="$cache/appimagetool-$arch.AppImage"
    APPIMAGE_RUNTIME="$cache/runtime-$arch"
    download_verified "https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-$arch.AppImage" "$deploy_sha" "$LINUXDEPLOY"
    download_verified "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-$arch.AppImage" "$tool_sha" "$APPIMAGETOOL"
    download_verified "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-$arch" "$runtime_sha" "$APPIMAGE_RUNTIME"
    chmod +x "$LINUXDEPLOY" "$APPIMAGETOOL"
}
