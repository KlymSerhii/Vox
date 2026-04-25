# Vox — Code Signing & Notarization Runbook

> Status: **operational draft for Phase 3 → public beta.** Owners must
> have an active Apple Developer account before any of this is real.
> Last updated: 2026-04-24.

This document is the single source of truth for taking a `cargo tauri
build`-produced `Vox.app` and turning it into something a regular user
can double-click without hitting Gatekeeper. Auto-updates will not work
without it — the updater plugin (`tauri-plugin-updater`, configured in
`tauri.conf.json`) verifies an Ed25519 signature *and* expects the
downloaded `.app` to be Apple-signed, so the chain has to be intact end
to end.

---

## 1. Prerequisites

- **Apple Developer Program** membership ($99/yr).
- **A "Developer ID Application" certificate** issued from your team
  account in `developer.apple.com → Account → Certificates, IDs &
  Profiles → Certificates`. Download the `.cer`, double-click to install
  it into the **login** Keychain. The private key must live next to it
  in the same Keychain (Xcode normally generates the key locally during
  the request flow — keep that machine's Keychain backed up).
- **Apple ID with an app-specific password** for `notarytool` uploads.
  Generate one at `appleid.apple.com → Sign-In and Security → App-
  Specific Passwords`. Label it `notarytool`.
- **Team ID** — the 10-character string in the upper right of your
  account page (looks like `ABCD123XYZ`).
- **Xcode 16+** installed (`xcode-select --install` is not enough on its
  own — full Xcode is what ships `notarytool` and the new stapling
  toolchain).
- **Tauri 2.3+** with the bundler enabled in `tauri.conf.json` (already
  set: `"bundle.targets": ["app", "dmg"]`).

One-time keychain check:

```bash
security find-identity -v -p codesigning
# Expect at least one line like:
#   1) AB12CD34EF...  "Developer ID Application: Your Name (ABCD123XYZ)"
```

If nothing prints, the certificate isn't installed in `login.keychain`.

---

## 2. Configure `tauri.conf.json`

Add the signing identity to `bundle.macOS.signingIdentity`. The exact
string is the **CN** (Common Name) of the certificate as printed by
`security find-identity` — keep the surrounding quotes.

```jsonc
{
  "bundle": {
    "macOS": {
      "minimumSystemVersion": "13.0",
      "entitlements": "entitlements.plist",
      "signingIdentity": "Developer ID Application: Your Name (ABCD123XYZ)",
      "providerShortName": "ABCD123XYZ",
      "exceptionDomain": "",
      "frameworks": []
    }
  }
}
```

Notes:

- `providerShortName` is your Team ID. Required when you have multiple
  teams in your Keychain; harmless when you don't.
- `signingIdentity` can also be supplied via the env var
  `APPLE_SIGNING_IDENTITY=...` so the conf stays personal-data-free in
  the repo. Recommended for OSS projects.
- We **do not enable `com.apple.security.app-sandbox`** in
  `entitlements.plist`. Vox needs to post `CGEvent`s into other apps for
  paste, which the sandbox blocks. Distribution outside the Mac App
  Store is the trade-off.

---

## 3. Hardened-runtime entitlements

The `entitlements.plist` we already ship enables:

```xml
<key>com.apple.security.device.audio-input</key><true/>
<key>com.apple.security.automation.apple-events</key><true/>
```

Both are required for hardened-runtime + notarization to succeed. If
you add more entitlements later, the same file is the place — Tauri
hands it to `codesign` automatically when bundling.

Verifying after a sign:

```bash
codesign -d --entitlements - --xml \
  src-tauri/target/release/bundle/macos/Vox.app | plutil -p -
```

You should see both keys present and `true`.

---

## 4. Build, sign, notarize, staple

Run from the repo root. **Each step depends on the previous one
succeeding.**

```bash
# 1. Build a Universal `.app` (arm64 + x86_64). For arm64-only:
#    `npm run tauri build -- --bundles app --target aarch64-apple-darwin`
APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (ABCD123XYZ)" \
  npm run tauri build -- --bundles app --target universal-apple-darwin

APP=src-tauri/target/universal-apple-darwin/release/bundle/macos/Vox.app

# 2. Sanity check: the bundle should already be hardened-runtime-signed
#    by the Tauri bundler. Confirm:
codesign --verify --strict --verbose=2 "$APP"
spctl --assess --verbose=2 "$APP" || true   # will fail until notarized

# 3. Submit to Apple's notary service. notarytool needs a zip of the
#    bundle, not the .app directory itself.
ditto -c -k --sequesterRsrc --keepParent "$APP" Vox.zip

xcrun notarytool submit Vox.zip \
  --apple-id "you@example.com" \
  --team-id "ABCD123XYZ" \
  --password "abcd-efgh-ijkl-mnop"   `# the app-specific password` \
  --wait

# Expected final line:
#   status: Accepted

# If something failed, fetch the log:
#   xcrun notarytool log <submission-id> --apple-id ... --team-id ... --password ...

# 4. Staple the notarization ticket so Gatekeeper doesn't need a network
#    call to verify on first launch.
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"
spctl --assess --verbose=2 "$APP"
# Expected:
#   Vox.app: accepted
#   source=Notarized Developer ID
```

For DMG distribution, sign+notarize+staple the `.dmg` after the same
pattern (replace `$APP` with the dmg path). The inner `.app` only has
to be processed once if you bundle the staple-validated app into the
dmg afterwards.

---

## 5. Storing the app-specific password (recommended)

Avoid passing the password on the command line on every release.
`notarytool` can read it from the Keychain:

```bash
xcrun notarytool store-credentials "vox-notary" \
  --apple-id "you@example.com" \
  --team-id "ABCD123XYZ" \
  --password "abcd-efgh-ijkl-mnop"

# Then on every release:
xcrun notarytool submit Vox.zip --keychain-profile "vox-notary" --wait
```

CI environments use `--apple-id`, `--team-id`, `--password` straight
from secrets — see §7.

---

## 6. Auto-update — Ed25519 manifest

`tauri-plugin-updater` uses an **Ed25519** key pair separate from
Apple's signing chain. The Apple chain proves "this app came from this
developer"; the Ed25519 key proves "this *update* was published by the
person who shipped the original app." Both are required.

```bash
# One-time, on a trusted machine. Save the private key somewhere
# durable (1Password / shared secret store) — losing it means every
# user has to re-install Vox to receive future updates.
npm run tauri signer generate -- -w ~/.tauri/vox-updater.key
# Output:
#   Private key: ~/.tauri/vox-updater.key
#   Public  key: <base64>

# Add the public key to tauri.conf.json:
#   "plugins.updater.pubkey": "<base64>"

# On every release:
TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/vox-updater.key) \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="if you set one" \
  npm run tauri build -- --bundles app
# Tauri now emits a `latest.json` next to the bundle with a signature.
# Upload `Vox.app.tar.gz` + `latest.json` to your update host.
```

Update server URL is templated in `tauri.conf.json` →
`plugins.updater.endpoints[0]`. Phase 3 currently ships a placeholder
(`https://updates.vox.local/...`) — replace it with the real CDN before
flipping auto-update on.

`commands::check_for_updates` (`src-tauri/src/commands.rs`) is wired
to the Settings → About → "Check for updates" button. We do **not**
auto-check at startup; that flag flips on once we have a signed
release in production.

---

## 7. GitHub Actions sketch

This is a *sketch*, not a working pipeline — it shows the moving parts
so a release engineer can fill in the secrets. Drop into
`.github/workflows/release.yml` and adapt.

```yaml
name: release
on:
  push:
    tags: ["v*.*.*"]

jobs:
  macos-release:
    runs-on: macos-14   # arm64 by default
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-node@v4
        with: { node-version: 22 }

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-apple-darwin,x86_64-apple-darwin

      - name: Install Apple Developer cert
        env:
          MACOS_CERTIFICATE: ${{ secrets.MACOS_CERTIFICATE }}    # base64 of .p12
          MACOS_CERTIFICATE_PWD: ${{ secrets.MACOS_CERTIFICATE_PWD }}
          KEYCHAIN_PWD: ${{ secrets.KEYCHAIN_PWD }}
        run: |
          echo "$MACOS_CERTIFICATE" | base64 --decode > cert.p12
          security create-keychain -p "$KEYCHAIN_PWD" build.keychain
          security default-keychain -s build.keychain
          security unlock-keychain -p "$KEYCHAIN_PWD" build.keychain
          security import cert.p12 -k build.keychain \
            -P "$MACOS_CERTIFICATE_PWD" -T /usr/bin/codesign
          security set-key-partition-list -S apple-tool:,apple: \
            -s -k "$KEYCHAIN_PWD" build.keychain

      - run: npm ci

      - name: Build, sign, notarize
        env:
          APPLE_SIGNING_IDENTITY: ${{ secrets.APPLE_SIGNING_IDENTITY }}
          APPLE_ID: ${{ secrets.APPLE_ID }}
          APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
          APPLE_APP_SPECIFIC_PASSWORD: ${{ secrets.APPLE_APP_SPECIFIC_PASSWORD }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        run: |
          npm run tauri build -- --bundles app,dmg \
            --target universal-apple-darwin
          # The Tauri bundler honors APPLE_* env vars and submits to notarytool
          # automatically when they are present. If you need the manual
          # variant, see Section 4.

      - name: Publish artifacts
        uses: softprops/action-gh-release@v2
        with:
          files: |
            src-tauri/target/universal-apple-darwin/release/bundle/macos/Vox.app.tar.gz
            src-tauri/target/universal-apple-darwin/release/bundle/macos/latest.json
            src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg
```

Required GitHub secrets:

- `MACOS_CERTIFICATE` — base64 of an exported `.p12` containing the
  Developer ID Application cert + private key.
- `MACOS_CERTIFICATE_PWD` — password used when exporting that `.p12`.
- `KEYCHAIN_PWD` — anything; only used to lock the temporary keychain
  on the runner.
- `APPLE_SIGNING_IDENTITY` — the CN string from §2.
- `APPLE_ID` — your Apple ID email.
- `APPLE_TEAM_ID` — `ABCD123XYZ`.
- `APPLE_APP_SPECIFIC_PASSWORD` — the password from §1.
- `TAURI_SIGNING_PRIVATE_KEY` — contents of the Ed25519 file from §6.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — only set if you encrypted the key.

---

## 8. First-run "open anyway" workaround

If a user is on a build that **isn't** notarized yet (Phase 2 dogfood
or a local dev build), Gatekeeper will block first launch with:

> "Vox.app" cannot be opened because the developer cannot be verified.

The workaround for users to keep moving until we ship a notarized
build:

1. Right-click (or Control-click) the `Vox.app`.
2. Select **Open**. The dialog now offers an **Open** button instead
   of just **OK**.
3. Click **Open**. The app launches and macOS records an exception so
   future double-clicks work.

If macOS Sequoia or later refuses even the right-click → Open path
("the app is damaged"), the user can clear the quarantine attribute
that triggered Gatekeeper:

```bash
xattr -dr com.apple.quarantine /Applications/Vox.app
```

This is a stop-gap, not a substitute for notarization. Document it in
the GitHub Releases description for any pre-notarized build.

---

## 9. Verification checklist before publishing

- [ ] `codesign --verify --strict --verbose=2 Vox.app` exits 0.
- [ ] `spctl --assess --verbose=2 Vox.app` says `source=Notarized Developer ID`.
- [ ] `xcrun stapler validate Vox.app` says `The validate action worked`.
- [ ] Launching from `/Applications` on a clean macOS install (or fresh
      VM) does **not** prompt with a Gatekeeper warning.
- [ ] `latest.json` contains a non-empty `signature` field and the
      version matches `tauri.conf.json::version`.
- [ ] Public key in `tauri.conf.json::plugins.updater.pubkey` matches
      the private key used to sign `latest.json` (a wrong public key
      causes silent "no update available" on every check).
- [ ] In Settings → About, "Check for updates" returns the new version
      number, not "unconfigured" / "error".

When all six are green, ship.
