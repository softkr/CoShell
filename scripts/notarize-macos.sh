#!/usr/bin/env bash
# Sign, notarize and staple the DMG made by scripts/package-dmg.sh.
# Needs a Developer ID build (COSHELL_SIGN_IDENTITY set for bundle-macos.sh too) and
# notarization credentials stored once in the keychain:
#   xcrun notarytool store-credentials coshell-notary --apple-id … --team-id … --password …
# Usage: COSHELL_SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" \
#        COSHELL_NOTARY_PROFILE=coshell-notary scripts/notarize-macos.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
DMG="$ROOT/target/release/bundle/CoShell-${VERSION}.dmg"

[[ "$(uname)" == "Darwin" ]] || { echo "macOS only" >&2; exit 1; }
[[ -n "${COSHELL_SIGN_IDENTITY:-}" ]] || { echo "set COSHELL_SIGN_IDENTITY (Developer ID Application: …)" >&2; exit 1; }
[[ -n "${COSHELL_NOTARY_PROFILE:-}" ]] || { echo "set COSHELL_NOTARY_PROFILE (notarytool keychain profile)" >&2; exit 1; }
[[ -f "$DMG" ]] || { echo "missing $DMG — run scripts/bundle-macos.sh and scripts/package-dmg.sh first" >&2; exit 1; }

codesign --force --timestamp --sign "$COSHELL_SIGN_IDENTITY" "$DMG"
xcrun notarytool submit "$DMG" --keychain-profile "$COSHELL_NOTARY_PROFILE" --wait
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"
spctl --assess --type open --context context:primary-signature "$DMG"
echo "$DMG"
