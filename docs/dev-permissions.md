# macOS Permissions in Development

Vox needs two macOS permissions to function:

* **Microphone** (TCC `Microphone`) — to capture audio while the hotkey is held.
* **Accessibility** (TCC `Accessibility`) — to read the focused window title and to inject the transcript via simulated `Cmd+V`.

Both are gated by macOS's TCC subsystem, which does *not* play nicely with the unbundled binary that `npm run tauri dev` produces. This document explains why, and the recommended workflow.

## Why TCC doesn't show "Vox" in dev mode

`npm run tauri dev` compiles `src-tauri/target/debug/vox`. That binary:

* is **ad-hoc, linker-signed**: `codesign -dv` reports `Signature=adhoc` and `flags=0x20002(adhoc,linker-signed)`;
* has **no bound `Info.plist`**: `codesign -dv` reports `Info.plist=not bound`. (An `__info_plist` is embedded as a Mach-O section, but it is not the loaded bundle plist that TCC keys against.)
* reports its bundle identity as something like `vox-<8-byte-hash>` rather than `com.vox.app`. Verify on your machine with:

    ```sh
    codesign -dv src-tauri/target/debug/vox 2>&1 | grep Identifier
    mdls -name kMDItemCFBundleIdentifier src-tauri/target/debug/vox
    ```

  Expected: `Identifier=vox-<hash>` and `kMDItemCFBundleIdentifier = (null)`.

TCC entries are keyed by **bundle identity + code signature**. Because `tauri dev` produces a binary whose bundle identity is a per-build hash and whose code signature changes every time you rebuild, TCC:

1. cannot list it under "Vox" in *System Settings → Privacy & Security → Microphone / Accessibility*;
2. invalidates any prior grant the moment a new compile changes the binary's signature.

This is **not** a Vox bug — it's a fundamental limitation of running an unbundled, ad-hoc-signed binary on modern macOS. Production builds (`tauri build`) produce a real `Vox.app` with `Info.plist`, bundle id `com.vox.app`, and a stable signature; TCC handles those correctly.

## Recommended dev workflow

### Best: build a debug `.app` bundle once and run from there

Use the helper script:

```sh
./scripts/dev-bundle.sh
```

It runs `npm run tauri build -- --debug --bundles app` and `open`s the result. The output path under Tauri 2.x is `src-tauri/target/debug/bundle/macos/Vox.app` (older toolchains may put it under `target/release/bundle/macos/Vox.app`; the script handles both).

The resulting `Vox.app`:

* has bundle id `com.vox.app` (matches `tauri.conf.json:identifier`);
* has the bundled `Info.plist` (`NSMicrophoneUsageDescription`, `NSAppleEventsUsageDescription`, `LSUIElement=true`);
* has the entitlements from `src-tauri/entitlements.plist` (`com.apple.security.device.audio-input`, etc.).

After granting permissions to this bundle, **you can keep relaunching it** as long as you don't rebuild with a different code signature. (Re-running the script will rebuild and may invalidate the grant — only do it when you've changed Rust code.)

> **Hot-reload caveat.** This is a *bundled debug* build, not `tauri dev`. You do not get HMR or fast-refresh for Rust changes. For pure frontend work, `tauri dev` is still the fastest loop; switch to `dev-bundle.sh` only for permission-sensitive testing.

### Alternative: manually add the dev binary to Accessibility

For Accessibility specifically (which doesn't show its prompt at all — it just silently refuses calls until the user grants), you can drag the dev binary into the list:

1. Open *System Settings → Privacy & Security → Accessibility*.
2. Click `+`.
3. Press `Cmd+Shift+G` and paste:

    ```
    /Users/serhiiklymenko/Projects/Vox/src-tauri/target/debug/vox
    ```

4. Add it. The list will show it as `vox` (lowercase) — that's the correct dev binary.
5. Toggle it on.

This **does not work** reliably for Microphone, because the mic prompt is what populates the entry and the dev binary's hashed identifier may not stick. Use `dev-bundle.sh` for mic.

### Reset workflow when permissions get into a weird state

If grants and revocations get tangled (e.g. you swapped between `dev-bundle.sh` and `tauri dev`, or pasted in a stale binary path), nuke the entries:

```sh
tccutil reset Microphone com.vox.app
tccutil reset Accessibility com.vox.app
```

You may also need to reset by the dev-binary identifier if `tauri dev` left a stale entry behind:

```sh
# Replace <hash> with the value reported by `codesign -dv`.
tccutil reset Microphone vox-<hash>
tccutil reset Accessibility vox-<hash>
```

If unsure, the nuclear option resets *all* TCC for both subsystems (will affect every other app too):

```sh
tccutil reset Microphone
tccutil reset Accessibility
```

Then relaunch via `dev-bundle.sh` and re-grant.

## What to verify after granting

A correctly-permitted Vox build, on first launch, should:

* **Tray icon** turns from the **padlock** glyph (`Permission` state) to the **mic outline** (`Idle` state) within ~2 s of granting both permissions.
* **Onboarding step 2 (mic)** flips its status from `○ Waiting…` to `✓ Granted` within 2 s of the user clicking *Allow microphone* (the onboarding window polls `check_permissions` every 2 s).
* **Onboarding step 3 (accessibility)** flips to `✓ Granted` within 2 s of toggling Vox on in System Settings.

If any of those don't happen, see the **common pitfalls** section below.

## Common pitfalls

* **Stale TCC entries from a prior cargo binary path.** `cargo clean` (or just letting incremental compilation churn) can change the dev binary's hashed identifier; the old TCC entry then dangles forever pointing at a binary that no longer matches. Reset with `tccutil` as above.

* **Bundle-id mismatch between dev and prod.** The dev binary is `vox-<hash>`; the prod bundle is `com.vox.app`. Granting one does not grant the other. If you alternate between `dev-bundle.sh` and `tauri dev`, expect to grant twice.

* **Code-signature changes invalidate TCC entries.** Even within `com.vox.app`, if the signing certificate changes (e.g. you switch from ad-hoc to a real Developer ID cert), TCC re-prompts. This is by design.

* **The mic prompt fires at app launch, before the user reaches the onboarding mic step.** This used to happen because the tray-icon code probed mic state via cpal at startup, which on macOS 14+ implicitly trips the TCC microphone gate. Fixed in `src-tauri/src/mac_perm.rs` — we now use `AVCaptureDevice.authorizationStatusForMediaType:`, which is non-prompting. If you ever see a mic prompt before the user clicks *Allow microphone*, search the logs for `cpal default_input_config` / `requestAccessForMediaType` — only the latter should appear, and only on user click.

* **`tauri dev`'s window title says "Vox" but TCC says "vox" (lowercase).** That's expected; the TCC list shows the binary file name, not the bundle display name. Trust the path you pasted in step 3 of the manual workflow.

## Reference: `tauri.conf.json` permission-relevant fields

* `identifier`: `com.vox.app` — must match `tccutil reset` calls.
* `bundle.macOS.entitlements`: `entitlements.plist` — declares `com.apple.security.device.audio-input` (mic), `com.apple.security.automation.apple-events` (paste), and `com.apple.security.cs.allow-jit` (Metal shader compilation under hardened runtime).
* `Info.plist` (at `src-tauri/Info.plist`): contains `NSMicrophoneUsageDescription` and `NSAppleEventsUsageDescription`. macOS displays these strings *inside* the TCC prompt when the user is asked to consent. If they're missing or generic, the App Store and notarisation will reject the build.

These are wired up correctly today; nothing here needs changing. If you're modifying any of them, re-run `dev-bundle.sh` and re-grant — the changes won't take effect in the running app.
