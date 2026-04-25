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
echo "and grant Accessibility (and Microphone, when the prompt fires)"
echo "when System Settings asks. After granting, hold Option+Control"
echo "in any text field — the listening pill should appear."
echo "============================================================="
