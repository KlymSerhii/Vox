#!/usr/bin/env bash
# Build and publish a Vox release entirely on this Mac — a local port of
# .github/workflows/release.yml for when GitHub Actions quota is exhausted.
#
# Why this machine is fine (better, even): the CI runner had to hunt for an
# Xcode containing the macOS 26 SDK so the apple-fm bridge could weak-link
# FoundationModels. This Mac is Apple Silicon (native aarch64 build) and
# already has that SDK, and it already holds every signing credential:
#   * Developer ID Application cert in the login keychain
#   * the Tauri updater key at ~/.tauri/vox-updater.key — whose public half
#     is byte-identical to the pubkey embedded in tauri.conf.json, so updates
#     signed here are accepted by every existing user's auto-updater.
#
# The only inputs that lived solely in CI secrets and must be supplied at
# runtime:
#   APPLE_ID                            Apple ID email (for notarization)
#   APPLE_PASSWORD                      app-specific password (appleid.apple.com)
#   TAURI_SIGNING_PRIVATE_KEY_PASSWORD  password for ~/.tauri/vox-updater.key
#
# Optional overrides (sensible defaults baked in):
#   APPLE_TEAM_ID              default 4NUJDWKWX5
#   APPLE_SIGNING_IDENTITY     default "Developer ID Application: Serhii Klymenko (4NUJDWKWX5)"
#   TAURI_SIGNING_PRIVATE_KEY  default: contents of ~/.tauri/vox-updater.key
#
# Usage:
#   scripts/release.sh             build + sign + notarize + stage locally; STOP before publishing
#   scripts/release.sh --publish   ...then create the GitHub release (auto-updates ALL users)
#
# The default (no flag) is a full dry run: it produces the exact artifacts and
# latest.json under a staging dir and prints them, but does NOT touch the
# public vox-releases repo. Inspect, then re-run with --publish.

set -euo pipefail
cd "$(dirname "$0")/.."

PUBLISH=0
if [ "${1:-}" = "--publish" ]; then
  PUBLISH=1
elif [ -n "${1:-}" ]; then
  echo "Unknown argument: $1 (expected nothing, or --publish)" >&2
  exit 2
fi

RELEASES_REPO="KlymSerhii/vox-releases"

# ---------------------------------------------------------------------------
# Non-secret defaults. The signing identity also lives in tauri.conf.json, but
# exporting it makes the bundler's choice explicit and matches the CI env.
# ---------------------------------------------------------------------------
export APPLE_TEAM_ID="${APPLE_TEAM_ID:-4NUJDWKWX5}"
export APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-Developer ID Application: Serhii Klymenko (4NUJDWKWX5)}"
# Pin the deployment target across every sub-build (cargo cc, swiftc, link,
# tauri bundle); mismatches produce ___isPlatformVersionAtLeast link errors.
export MACOSX_DEPLOYMENT_TARGET="14.0"

UPDATER_KEY="$HOME/.tauri/vox-updater.key"
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  if [ ! -f "$UPDATER_KEY" ]; then
    echo "✗ $UPDATER_KEY not found and TAURI_SIGNING_PRIVATE_KEY is unset." >&2
    exit 1
  fi
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$UPDATER_KEY")"
  export TAURI_SIGNING_PRIVATE_KEY
fi

# ---------------------------------------------------------------------------
# Required secrets — fail early with the full list rather than mid-build.
# ---------------------------------------------------------------------------
missing=0
for v in APPLE_ID APPLE_PASSWORD TAURI_SIGNING_PRIVATE_KEY_PASSWORD; do
  if [ -z "${!v:-}" ]; then
    echo "✗ missing required env var: $v" >&2
    missing=1
  fi
done
if [ "$missing" = 1 ]; then
  echo >&2
  echo "Export the variables above, then re-run. Example:" >&2
  echo "  export APPLE_ID='you@example.com'" >&2
  echo "  export APPLE_PASSWORD='abcd-efgh-ijkl-mnop'   # app-specific password" >&2
  echo "  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD='…'" >&2
  exit 1
fi

if [ "$PUBLISH" = 1 ] && ! gh auth status >/dev/null 2>&1; then
  echo "✗ --publish requested but 'gh' is not authenticated. Run: gh auth login" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Resolve version from Cargo.toml (same source the CI uses on manual dispatch).
# ---------------------------------------------------------------------------
VERSION=$(grep -m1 '^version' src-tauri/Cargo.toml | sed -E 's/version *= *"(.*)"/\1/')
echo "==> Vox release v${VERSION}  (publish=${PUBLISH})"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

# ---------------------------------------------------------------------------
# Release notes from the commit log since the previous v* tag. The previous
# release is the latest v* tag that ISN'T the one we're cutting — we exclude
# v${VERSION} because that tag often already exists locally (a prior CI attempt
# pushed it; the workflow triggers on a v* tag push). Without --exclude,
# `git describe` returns v${VERSION} itself when HEAD sits on it, and the
# changelog comes back empty ("No user-visible changes since v${VERSION}").
# This is also correct in the not-yet-tagged case: --exclude is then a no-op
# and we still get the latest prior tag.
# ---------------------------------------------------------------------------
echo "==> Generating release notes..."
PREV_TAG=$(git describe --tags --abbrev=0 --match='v*' --exclude="v${VERSION}" 2>/dev/null || true)
NOTES_PLAIN="$STAGE/release-notes.txt"
NOTES_BODY="$STAGE/release-body.md"

if [ -n "$PREV_TAG" ]; then
  COMMITS=$(git log "$PREV_TAG"..HEAD --no-merges --reverse --pretty=format:'- %s')
  if [ -z "$COMMITS" ]; then
    COMMITS="- No user-visible changes since ${PREV_TAG}."
  fi
  HEADING="## What's new since ${PREV_TAG}"
else
  COMMITS="- Initial release."
  HEADING="## What's new"
fi

printf '%s\n' "$COMMITS" > "$NOTES_PLAIN"
{
  echo "${HEADING}"
  echo
  echo "$COMMITS"
  echo
  echo "## Install"
  echo "1. Download \`Vox.dmg\`."
  echo "2. Open the DMG and drag Vox into Applications."
  echo "3. Launch from Applications. macOS will verify the signature & notarization on first launch — no \"unknown developer\" warning."
  echo
  echo "## Auto-update"
  echo "The auto-updater fetches \`latest.json\` from this release. Existing v${VERSION}+ users will receive future updates automatically."
  echo
  echo "## License"
  echo "Free for personal use. Commercial use requires a paid license — contact \`vox@rizenhq.com\`. See [LICENSE](https://github.com/KlymSerhii/vox-releases/blob/main/LICENSE)."
} > "$NOTES_BODY"

# ---------------------------------------------------------------------------
# Build, sign, and notarize. Tauri's bundler auto-submits to Apple's notary
# service when APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID are present, then staples.
# The updater .tar.gz + .sig come for free (createUpdaterArtifacts: true).
# ---------------------------------------------------------------------------
echo "==> Building, signing, and notarizing (this takes a while + uploads to Apple notary)..."
npm run tauri build -- \
  --target aarch64-apple-darwin \
  --bundles app dmg \
  --verbose

# ---------------------------------------------------------------------------
# Stage artifacts under version-agnostic names so the marketing site can link
# to /releases/latest/download/Vox.dmg, then build latest.json.
# ---------------------------------------------------------------------------
echo "==> Staging artifacts..."
BUNDLE_DIR="src-tauri/target/aarch64-apple-darwin/release/bundle"
APP_TARGZ="$BUNDLE_DIR/macos/Vox.app.tar.gz"
APP_SIG="$BUNDLE_DIR/macos/Vox.app.tar.gz.sig"
DMG="$BUNDLE_DIR/dmg/Vox_${VERSION}_aarch64.dmg"

for f in "$APP_TARGZ" "$APP_SIG" "$DMG"; do
  if [ ! -f "$f" ]; then
    echo "✗ missing expected artifact: $f" >&2
    ls -la "$BUNDLE_DIR/macos" "$BUNDLE_DIR/dmg" 2>/dev/null || true
    exit 1
  fi
done

cp "$APP_TARGZ" "$STAGE/Vox.app.tar.gz"
cp "$APP_SIG"   "$STAGE/Vox.app.tar.gz.sig"
cp "$DMG"       "$STAGE/Vox.dmg"

# jq -Rs / --rawfile JSON-escapes the notes so a commit subject containing a
# quote or backslash can't break the manifest.
#
# This script is macOS-only: it publishes the darwin-aarch64 platform entry.
# Windows is published separately and additively by scripts/release-windows.ps1,
# which merges a `windows-x86_64` entry into this same latest.json AFTER this
# runs (it never deletes the release). Run order is macOS first, Windows second.
SIGNATURE=$(cat "$APP_SIG")
PUB_DATE=$(date -u +%Y-%m-%dT%H:%M:%SZ)
URL="https://github.com/${RELEASES_REPO}/releases/download/v${VERSION}/Vox.app.tar.gz"

jq -n \
  --arg version "$VERSION" \
  --arg pub_date "$PUB_DATE" \
  --arg sig "$SIGNATURE" \
  --arg url "$URL" \
  --rawfile notes "$NOTES_PLAIN" \
  '{
    version: $version,
    notes: $notes,
    pub_date: $pub_date,
    platforms: {
      "darwin-aarch64": { signature: $sig, url: $url }
    }
  }' > "$STAGE/latest.json"

# ---------------------------------------------------------------------------
# Gatekeeper sanity check on the signed, notarized, stapled .app.
# ---------------------------------------------------------------------------
echo "==> Verifying Gatekeeper acceptance..."
APP_PATH="$BUNDLE_DIR/macos/Vox.app"
codesign --verify --deep --strict --verbose=2 "$APP_PATH"
spctl --assess --type exec --verbose=4 "$APP_PATH"

echo
echo "==> latest.json:"
cat "$STAGE/latest.json"
echo
echo "==> Staged artifacts in $STAGE:"
ls -la "$STAGE"

if [ "$PUBLISH" != 1 ]; then
  echo
  echo "============================================================="
  echo "Dry run complete — nothing was published."
  echo "Artifacts above are signed, notarized, and Gatekeeper-clean."
  echo "To publish v${VERSION} to ${RELEASES_REPO} (auto-updates all users):"
  echo
  echo "    scripts/release.sh --publish"
  echo "============================================================="
  # Copy artifacts somewhere durable so the staging tempdir cleanup doesn't
  # discard a good build the user wanted to inspect or hand-upload.
  OUT="src-tauri/target/aarch64-apple-darwin/release/vox-release-v${VERSION}"
  rm -rf "$OUT"; mkdir -p "$OUT"
  cp "$STAGE"/Vox.app.tar.gz "$STAGE"/Vox.app.tar.gz.sig "$STAGE"/Vox.dmg "$STAGE"/latest.json "$OUT/"
  echo "(also copied to $OUT for inspection)"
  exit 0
fi

# ---------------------------------------------------------------------------
# Publish. Deleting any existing release first makes manual retries idempotent
# (no-op if it doesn't exist). This creates tag v${VERSION} on the releases
# repo and uploads all four artifacts.
# ---------------------------------------------------------------------------
echo "==> Publishing v${VERSION} to ${RELEASES_REPO}..."
gh release delete "v${VERSION}" --repo "$RELEASES_REPO" --yes || true
gh release create "v${VERSION}" \
  --repo "$RELEASES_REPO" \
  --title "Vox ${VERSION}" \
  --notes-file "$NOTES_BODY" \
  "$STAGE/Vox.app.tar.gz" \
  "$STAGE/Vox.app.tar.gz.sig" \
  "$STAGE/Vox.dmg" \
  "$STAGE/latest.json"

echo
echo "Released v${VERSION}: https://github.com/${RELEASES_REPO}/releases/tag/v${VERSION}"
