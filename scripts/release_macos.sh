#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="OpenMango"
BUNDLE_ID="com.openmango.app"
VERSION="$(grep '^version = ' "$ROOT_DIR/Cargo.toml" | head -1 | cut -d '"' -f2)"

# Accept target architecture as argument (e.g., aarch64-apple-darwin or x86_64-apple-darwin)
TARGET="${1:-}"

DIST_DIR="$ROOT_DIR/dist"
APP_DIR="$DIST_DIR/${APP_NAME}.app"
ICON_ICNS="$ROOT_DIR/assets/logo/openmango.icns"

mkdir -p "$DIST_DIR"

if [[ -n "$TARGET" ]]; then
    rustup target add "$TARGET"
    cargo build --release --target "$TARGET" --features mimalloc
    BIN_PATH="$ROOT_DIR/target/$TARGET/release/openmango"
    # Determine suffix from target
    case "$TARGET" in
        aarch64-apple-darwin) ARCH_SUFFIX="macos-arm64" ;;
        x86_64-apple-darwin)  ARCH_SUFFIX="macos-x86_64" ;;
        *)                    ARCH_SUFFIX="macos" ;;
    esac
else
    cargo build --release --features mimalloc
    BIN_PATH="$ROOT_DIR/target/release/openmango"
    ARCH_SUFFIX="macos"
fi

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources/bin"

cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/$APP_NAME"
chmod +x "$APP_DIR/Contents/MacOS/$APP_NAME"

# Bundle external tools/runtime if available
BIN_DIR="$ROOT_DIR/resources/bin/$ARCH_SUFFIX"
REQUIRE_BUNDLED_TOOLS="${REQUIRE_BUNDLED_TOOLS:-0}"
missing_tools=()

for tool in mongodump mongorestore node; do
    if [[ -f "$BIN_DIR/$tool" ]]; then
        cp "$BIN_DIR/$tool" "$APP_DIR/Contents/Resources/bin/$tool"
        chmod +x "$APP_DIR/Contents/Resources/bin/$tool"
    else
        missing_tools+=("$tool")
    fi
done

if [[ ${#missing_tools[@]} -eq 0 ]]; then
    echo "Bundled tools from $BIN_DIR"
else
    if [[ "$REQUIRE_BUNDLED_TOOLS" == "1" ]]; then
        echo "Error: missing bundled binaries for $ARCH_SUFFIX: ${missing_tools[*]}" >&2
        echo "Expected location: $BIN_DIR" >&2
        exit 1
    fi
    echo "Warning: missing bundled binaries for $ARCH_SUFFIX: ${missing_tools[*]}"
    echo "Expected location: $BIN_DIR"
    echo "BSON export/import may require system-installed tools"
    echo "Forge shell may require a system Node runtime"
fi

# Copy third-party notices (required for bundled tools attribution)
if [[ -f "$ROOT_DIR/THIRD_PARTY_NOTICES" ]]; then
    cp "$ROOT_DIR/THIRD_PARTY_NOTICES" "$APP_DIR/Contents/Resources/"
fi

HAS_ICON=false
if [[ -f "$ICON_ICNS" ]]; then
    cp "$ICON_ICNS" "$APP_DIR/Contents/Resources/openmango.icns"
    HAS_ICON=true
else
    echo "Warning: $ICON_ICNS not found. App will use the default icon."
fi

cat > "$APP_DIR/Contents/Info.plist" <<EOF2
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleExecutable</key>
    <string>${APP_NAME}</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
EOF2

if [[ "$HAS_ICON" == true ]]; then
cat >> "$APP_DIR/Contents/Info.plist" <<EOF2
    <key>CFBundleIconFile</key>
    <string>openmango</string>
EOF2
fi

cat >> "$APP_DIR/Contents/Info.plist" <<EOF2
</dict>
</plist>
EOF2

SIGNING_IDENTITY="${MACOS_SIGNING_IDENTITY:-}"
if [[ -n "$SIGNING_IDENTITY" ]]; then
    echo "Codesigning app with identity: $SIGNING_IDENTITY"
    # Sign bundled tools first (required before signing the app bundle)
    if [[ -f "$APP_DIR/Contents/Resources/bin/mongodump" ]]; then
        codesign --force --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$APP_DIR/Contents/Resources/bin/mongodump"
    fi
    if [[ -f "$APP_DIR/Contents/Resources/bin/mongorestore" ]]; then
        codesign --force --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$APP_DIR/Contents/Resources/bin/mongorestore"
    fi
    if [[ -f "$APP_DIR/Contents/Resources/bin/node" ]]; then
        codesign --force --options runtime --timestamp \
            --entitlements "$ROOT_DIR/scripts/node.entitlements" \
            --sign "$SIGNING_IDENTITY" "$APP_DIR/Contents/Resources/bin/node"
    fi
    # Sign the main app bundle
    codesign --force --options runtime --timestamp --deep --sign "$SIGNING_IDENTITY" "$APP_DIR"
    codesign --verify --deep --strict "$APP_DIR"
fi

ZIP_PATH="$DIST_DIR/${APP_NAME}-${VERSION}-${ARCH_SUFFIX}.zip"
rm -f "$ZIP_PATH"

ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ZIP_PATH"

if [[ -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER_ID:-}" ]]; then
    NOTARY_KEY_PATH="${APPLE_API_KEY_PATH:-"$DIST_DIR/AuthKey.p8"}"
    if [[ -n "${APPLE_API_KEY:-}" ]]; then
        if base64 --help 2>&1 | grep -q -- "-d"; then
            echo "$APPLE_API_KEY" | base64 -d > "$NOTARY_KEY_PATH"
        else
            echo "$APPLE_API_KEY" | base64 -D > "$NOTARY_KEY_PATH"
        fi
    fi
    if [[ ! -f "$NOTARY_KEY_PATH" ]]; then
        echo "Notarization key not found at $NOTARY_KEY_PATH"
        exit 1
    fi
    echo "Submitting for notarization..."
    xcrun notarytool submit "$ZIP_PATH" \
        --key "$NOTARY_KEY_PATH" \
        --key-id "$APPLE_API_KEY_ID" \
        --issuer "$APPLE_API_ISSUER_ID" \
        --wait
    echo "Stapling notarization ticket..."
    xcrun stapler staple "$APP_DIR"
    # Re-create zip with stapled app
    rm -f "$ZIP_PATH"
    ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ZIP_PATH"
fi

echo "Built: $APP_DIR"
echo "Packaged: $ZIP_PATH"
