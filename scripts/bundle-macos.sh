#!/usr/bin/env bash
# Build CoShell.app (release) with the brand icon from crates/coshell-app/assets/.
# Usage: scripts/bundle-macos.sh            → target/release/bundle/CoShell.app
#        COSHELL_BUNDLE_ID=com.example.coshell scripts/bundle-macos.sh
#        COSHELL_SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" scripts/bundle-macos.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE_ID="${COSHELL_BUNDLE_ID:-app.coshell.CoShell}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
ICNS="$ROOT/crates/coshell-app/assets/macos/CoShell.icns"
APP="$ROOT/target/release/bundle/CoShell.app"

[[ "$(uname)" == "Darwin" ]] || { echo "macOS only" >&2; exit 1; }
[[ -f "$ICNS" ]] || { echo "missing $ICNS" >&2; exit 1; }

cargo build --release -p coshell-app --manifest-path "$ROOT/Cargo.toml"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/target/release/coshell" "$APP/Contents/MacOS/CoShell"
cp "$ICNS" "$APP/Contents/Resources/CoShell.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>CoShell</string>
  <key>CFBundleDisplayName</key><string>CoShell</string>
  <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
  <key>CFBundleExecutable</key><string>CoShell</string>
  <key>CFBundleIconFile</key><string>CoShell</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>CoShell ${VERSION} · Apache-2.0</string>
</dict>
</plist>
PLIST

plutil -lint "$APP/Contents/Info.plist" >/dev/null
if [[ -n "${COSHELL_SIGN_IDENTITY:-}" ]]; then
  # Distribution build: Developer ID + hardened runtime (required for notarization).
  codesign --force --options runtime --timestamp --sign "$COSHELL_SIGN_IDENTITY" "$APP"
  codesign --verify --strict "$APP"
else
  # Ad-hoc signature so Gatekeeper accepts the local build (not for distribution).
  codesign --force --sign - "$APP" >/dev/null 2>&1 || echo "warning: ad-hoc codesign failed" >&2
fi
echo "$APP"
