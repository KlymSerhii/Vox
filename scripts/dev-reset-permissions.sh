#!/usr/bin/env bash
# Reset Vox's macOS TCC permissions for a clean dev cycle.
#
# Why: macOS TCC keys grants by bundle identity + code signature. When a
# dev build's signature changes (which happens any time you rebuild with
# `tauri dev` or `dev-bundle.sh`) the prior grant may be invalidated, but
# the entry sometimes sticks around in a half-revoked state where Vox
# appears toggled on yet `AXIsProcessTrusted()` returns false. Resetting
# clears this so the next launch can re-prompt cleanly.
#
# This script ONLY resets entries keyed by `com.vox.app` (the bundle id
# from `tauri.conf.json:identifier`). It does not touch other apps.

set -euo pipefail

BUNDLE_ID="com.vox.app"

echo "==> Resetting Microphone for $BUNDLE_ID..."
tccutil reset Microphone "$BUNDLE_ID" || true

echo "==> Resetting Accessibility for $BUNDLE_ID..."
tccutil reset Accessibility "$BUNDLE_ID" || true

echo
echo "============================================================="
echo "Permissions reset. Re-launch Vox via:"
echo
echo "    ./scripts/dev-bundle.sh"
echo
echo "and grant Accessibility (only needed for the optional chord-hotkey"
echo "fallback) and Microphone when System Settings asks. After granting,"
echo "press Cmd+Ctrl+Space in any text field — the listening pill"
echo "should appear."
echo "============================================================="
