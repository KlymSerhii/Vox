#!/usr/bin/env bash
# Wipe ALL Vox state for a full first-launch experience.
#
# After running this and starting `npm run tauri dev`, the app behaves
# as if it had never run on this Mac:
#
#   * onboarding wizard fires (settings file is gone, so `onboarded`
#     is false)
#   * mic prompt again (TCC entry reset). 2026-04-26: Accessibility
#     is no longer required — Vox copies transcripts to the clipboard
#     and the user pastes manually with ⌘V — but we still reset the
#     entry so a clean slate looks fully clean to the user.
#   * downloaded Whisper / Gemma weights are gone
#   * Voice Modes are reseeded from the built-in catalog
#   * legacy cloud Keychain entries (groq, anthropic) are removed
#
# This script is intentionally conservative — it touches ONLY paths
# scoped to `com.vox.app` and the `app.vox.vox` Keychain service. It
# does not delete the build cache (`src-tauri/target`) or anything
# under `~/Library` outside Vox's own directory.
#
# Pair with:
#   ./scripts/dev-clean-state.sh
#   npm run tauri dev    (or ./scripts/dev-bundle.sh)

set -euo pipefail

BUNDLE_ID="com.vox.app"
APP_SUPPORT="$HOME/Library/Application Support/$BUNDLE_ID"
KEYRING_SERVICE="app.vox.vox"

echo "==> Removing app state at:"
echo "    $APP_SUPPORT"
if [ -d "$APP_SUPPORT" ]; then
  # Show what's about to go.
  /bin/ls -la "$APP_SUPPORT" 2>/dev/null | sed 's/^/    /' || true
  rm -rf "$APP_SUPPORT"
  echo "    (deleted)"
else
  echo "    (no app-support directory — nothing to remove)"
fi

echo
echo "==> Resetting TCC entries for $BUNDLE_ID..."
tccutil reset Microphone "$BUNDLE_ID" || true
tccutil reset Accessibility "$BUNDLE_ID" || true

echo
echo "==> Removing legacy Keychain entries for $KEYRING_SERVICE..."
# `security delete-generic-password` exits non-zero when an entry isn't
# found; we treat that as success because the goal is "be gone".
for ACCOUNT in groq anthropic; do
  if security delete-generic-password \
      -s "$KEYRING_SERVICE" -a "$ACCOUNT" >/dev/null 2>&1; then
    echo "    deleted $ACCOUNT"
  else
    echo "    (no $ACCOUNT entry)"
  fi
done

echo
echo "============================================================="
echo "Clean. Now run:"
echo
echo "    npm run tauri dev"
echo
echo "Onboarding will fire on first launch. macOS will prompt for"
echo "Microphone — grant it. (Accessibility is no longer required:"
echo "Vox copies the transcript to the clipboard and you paste with ⌘V.)"
echo "============================================================="
