#!/usr/bin/env bash
# Build a debug `.app` bundle and launch it.
#
# Why: `npm run tauri dev` produces an unbundled, ad-hoc-signed raw binary
# (`src-tauri/target/debug/vox`) whose macOS bundle identity is something
# like `vox-<hash>` rather than `com.vox.app`. macOS TCC keys permission
# entries by bundle identity + signing, so:
#
#   * The dev binary doesn't appear as "Vox" in System Settings →
#     Privacy & Security → Microphone / Accessibility.
#   * Permissions granted to a previous build of the dev binary may be
#     invalidated when its hashed identifier changes.
#
# Building a debug `.app` bundle gives you a real `Vox.app` with the
# correct `Info.plist` and bundle id `com.vox.app`, which TCC recognises.
# Use this for any work that touches macOS permissions.
#
# See `docs/dev-permissions.md` for the full rationale and reset workflow.

set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Building debug .app bundle (this is slower than 'tauri dev')..."
npm run tauri build -- --debug --bundles app

# Tauri 2.x writes `--debug` bundles to `target/debug/bundle/macos/`.
# Older toolchains and some target-triple combos still drop them under
# `target/release/bundle/macos/`; we accept either to stay forward-compat.
APP="src-tauri/target/debug/bundle/macos/Vox.app"
if [ ! -d "$APP" ]; then
  APP="src-tauri/target/release/bundle/macos/Vox.app"
fi

if [ ! -d "$APP" ]; then
  echo "ERROR: could not find Vox.app under src-tauri/target/{debug,release}/bundle/macos/" >&2
  echo "       Look at the tauri build output above for the actual path." >&2
  exit 1
fi

echo
echo "==> Bundle identifier:"
/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$APP/Contents/Info.plist" || true
echo "==> Code signature:"
codesign -dv "$APP" 2>&1 | sed 's/^/    /' || true
echo
echo "==> Launching $APP"
open "$APP"
