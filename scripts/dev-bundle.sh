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
# See `CLAUDE.md` § Development for the full rationale and reset workflow.

set -euo pipefail

cd "$(dirname "$0")/.."

# Tauri 2.x writes `--debug` bundles under `target/debug/bundle/macos/`.
# Older toolchains and some target-triple combos still drop them under
# `target/release/bundle/macos/`; we accept either path.
BUNDLE_DEBUG="src-tauri/target/debug/bundle/macos/Vox.app"
BUNDLE_RELEASE="src-tauri/target/release/bundle/macos/Vox.app"

# If a previous bundle exists, remove it before rebuilding so TCC sees a
# fresh signature when the new bundle launches. Without this step, TCC
# may (sometimes — it's not deterministic) carry a stale grant forward
# from the previous compile, which is convenient until it isn't and you
# end up with a half-permitted state that's hard to diagnose.
echo "==> Removing any previous Vox.app bundle so TCC sees a fresh signature..."
rm -rf "$BUNDLE_DEBUG" "$BUNDLE_RELEASE" 2>/dev/null || true

echo "==> Building debug .app bundle (this is slower than 'tauri dev')..."
npm run tauri build -- --debug --bundles app

APP="$BUNDLE_DEBUG"
if [ ! -d "$APP" ]; then
  APP="$BUNDLE_RELEASE"
fi

if [ ! -d "$APP" ]; then
  echo "ERROR: could not find Vox.app under src-tauri/target/{debug,release}/bundle/macos/" >&2
  echo "       Look at the tauri build output above for the actual path." >&2
  exit 1
fi

# Resolve to an absolute path so the user can paste it into TCC if they
# need to add Vox manually via the "+" button.
APP_ABS="$(cd "$(dirname "$APP")" && pwd)/$(basename "$APP")"

echo
echo "==> Bundle identifier:"
/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$APP/Contents/Info.plist" || true
echo "==> Code signature:"
codesign -dv "$APP" 2>&1 | sed 's/^/    /' || true
echo
echo "==> Launching $APP_ABS"
open "$APP"

echo
echo "============================================================="
echo "Now: open System Settings → Privacy & Security → Accessibility"
echo "and toggle 'Vox' on. If Vox doesn't appear in the list, click"
echo "'+' and paste this absolute path (Cmd+Shift+G works too):"
echo
echo "    $APP_ABS"
echo
echo "Then hold Option+Control in any text field — the listening pill"
echo "should appear. Release to transcribe and paste."
echo "============================================================="
