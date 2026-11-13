#!/usr/bin/env bash
# Package target/release/bundle/CoShell.app into a drag-to-install DMG.
# Run scripts/bundle-macos.sh first (this script does not rebuild).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/target/release/bundle/CoShell.app"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
DMG="$ROOT/target/release/bundle/CoShell-${VERSION}.dmg"

[[ -d "$APP" ]] || { echo "missing $APP — run scripts/bundle-macos.sh first" >&2; exit 1; }

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/CoShell.app"          # ditto keeps the code signature intact
ln -s /Applications "$STAGE/Applications"   # drag target in the DMG window

rm -f "$DMG"
hdiutil create -volname "CoShell ${VERSION}" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG" >/dev/null
hdiutil verify "$DMG" >/dev/null
echo "$DMG"
