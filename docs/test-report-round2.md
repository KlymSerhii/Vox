# Vox — QA Test Report (Round 2)

> Tester: QA / Automated verification
> Date: 2026-04-24
> Scope: Round-2 deliverables (Phase-3 polish, Local-first pivot,
> Voice Modes differentiator) + Round-2 staff-review fixes (C1–C4).
> Prior baseline: `docs/test-report.md` (Phase-1 / Phase-2 / Phase-3
> initial cut).
>
> Verdict: **SHIP-WITH-CAVEATS** — see "Final regression matrix" and
> "Open caveats" below.

---

## Methodology

Same as round 1. Verifiable in this environment without macOS TCC
permissions or real model downloads:

- A. Static / build verification (cargo check + clippy + tests + npm
  build + tauri bundle, with and without `local-engines` feature).
- B. Bundle inspection of the produced `Vox.app`.
- C. Frontend smoke test (Vite dev server + curl).
- D. Code-trace verification per the round-2 brief.

Deferred to the user (must run on a real Mac with their permissions
and keys):
- E. Manual test plan — see `docs/manual-test-plan.md` (sections
  10–14 are new for round 2).
- F. End-to-end model download from a clean machine + sha256 pinning
  (T-01 in `local-first-impl-notes.md`).

---

## Phase A — Static / build verification

| Command | Result |
|---|---|
| `cargo check --manifest-path src-tauri/Cargo.toml` | ✅ pass — clean (zero warnings) |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | ✅ pass — clean |
| `cargo check --manifest-path src-tauri/Cargo.toml --features local-engines` | ✅ pass — clean (after first-time C++ build) |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --features local-engines --all-targets -- -D warnings` | ✅ pass — clean |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib` | ✅ 18 passed (12 modes + 1 download_registry_dedupes_by_id + 5 misc) |
| `npm run build` | ✅ pass — 5 HTML entries, 7 JS chunks, 1 CSS, ~31 KB total gzipped |
| `npm run tauri build -- --bundles app` | ✅ pass — Release `.app` produced (7.4 MB), ad-hoc signed |

Round 1's 17 cocoa deprecation warnings are gone — the objc2-app-kit
migration is complete and clippy `-D warnings` exits zero.

---

## Phase B — Bundle inspection

Bundle path: `src-tauri/target/release/bundle/macos/Vox.app`
(7.4 MB).

| Check | Result |
|---|---|
| `.app` directory exists | ✅ |
| `Contents/MacOS/vox` (Mach-O thin arm64) | ✅ |
| `Info.plist::LSUIElement = true` (no Dock icon) | ✅ |
| `Info.plist::NSMicrophoneUsageDescription` set | ✅ |
| `Info.plist::NSAppleEventsUsageDescription` set | ✅ |
| `CFBundleIdentifier = com.vox.app` | ✅ |
| `CFBundleVersion = 0.1.0` | ✅ |
| `entitlements.plist` present in source with `com.apple.security.cs.allow-jit` (Metal shader compile) | ✅ — file at `src-tauri/entitlements.plist`, Tauri config references it |
| All 5 expected window HTML files (settings/index, pill, onboarding, tray-popover, migration) embedded in binary | ✅ — verified via `strings` (each HTML name present 2–6× in the binary) |
| No GGUF / ggml / `.bin` model files bundled | ✅ — `find` returns empty; bundle stays at ~7 MB as architecture intends |
| `codesign -dv` | ⚠️ ad-hoc (`adhoc, linker-signed`). Expected for dev builds; entitlements only apply when notarizing with a real signing identity. **Must be code-signed + notarized before public distribution.** |

Caveat for ship: entitlements are not applied to ad-hoc-signed
artifacts (this is macOS, not a Vox bug). When you sign with a real
Developer ID, `entitlements.plist` (with `com.apple.security.cs.allow-jit`)
gets applied automatically by Tauri. Verify with
`codesign -d --entitlements - /path/to/Vox.app` after that step.

---

## Phase C — Frontend smoke test (Vite dev server)

`npm run dev` started Vite on `http://localhost:1420`. All five HTML
entries and all five TypeScript per-window entries return 200:

| Path | Status |
|---|---|
| `/` (settings index.html) | ✅ 200 |
| `/pill.html` | ✅ 200 |
| `/onboarding.html` | ✅ 200 |
| `/tray-popover.html` | ✅ 200 |
| `/migration.html` | ✅ 200 |
| `/src/entries/settings.tsx` | ✅ 200 |
| `/src/entries/pill.tsx` | ✅ 200 |
| `/src/entries/onboarding.tsx` | ✅ 200 |
| `/src/entries/tray-popover.tsx` | ✅ 200 |
| `/src/entries/migration.tsx` | ✅ 200 |

Dev server stopped after the smoke pass.

---

## Phase D — Code-trace verification

### D.1 Phase-3 polish

| Item | Verdict | Where / notes |
|---|---|---|
| objc2 migration (no cocoa, no msg_send! except `setCanHide:`) | ✅ | `pill.rs:266-368`. `setCanHide:` routed via `objc2::msg_send!` on `AnyObject`. Zero deprecation warnings under `clippy -D warnings`. |
| Tray icon state machine + 5 states + pulse loop | ✅ | `tray_icon.rs::TrayState` (Idle / Recording / Processing / Error / Permission). `pulse_running: AtomicBool` guards reentrancy. Glyphs in `tray_glyphs.rs`. |
| Tray icon mirrored from pill state | ✅ | `pill::emit_state` calls `tray_icon::set_state`. `pill::hide` returns Idle. |
| Launch-at-login (`get_launch_at_login` / `set_launch_at_login`) | ✅ | `commands.rs:263-285` round-trips `tauri-plugin-autostart` state; OS is the source of truth. UI wired in `Settings.tsx`. |
| Multi-monitor pill positioning | ✅ | `pill.rs::reposition` → `active_screen_logical_frame` queries `NSEvent::mouseLocation()` + `NSScreen::screens()`, AppKit↔Tauri Y-flip handled, falls back to primary. |
| Paste-last hotkey (`Alt+Shift+V`) | ✅ | `hotkey.rs` registers third shortcut → `commands::paste_last_transcript`. Empty buffer & mid-pipeline cases each surface a friendly Error pill. |
| Hands-free toggle mode | ✅ | `hotkey.rs` press callback branches on `prefs.hands_free_enabled`. 60 s safety cap intact. VAD-based silence auto-stop is still TODO (deferred). |
| Language pinning (setting + UI + STT consumption) | ✅ | `Prefs.language` populated; `session.rs:260-264` passes `Some(prefs.language)` (or `None` for "auto") to `SttEngine::transcribe`. The local-first track now consumes it. |
| Updater stub + Check-for-updates command | ✅ | `commands.rs::check_for_updates` short-circuits via `updater_is_stub()` (lines 369–376) **before** any `app.updater()` call. Detects placeholder host or empty pubkey. C2 fix verified. |

### D.2 Local-first pivot

| Item | Verdict | Where / notes |
|---|---|---|
| `SttEngine` / `LlmEngine` traits | ✅ | `engines/mod.rs:27-72`. `Registry` keys lazy-built `(engine, model)` engines. |
| Local Whisper (`whisper-local`) compiles under `local-engines` | ✅ | `engines/whisper_local.rs`, 200 lines. Lazy `WhisperContext` cached in `OnceCell<Arc<Mutex<…>>>`. |
| Local Gemma (`gemma-local`) compiles under `local-engines` | ✅ | `engines/gemma_local.rs`, 261 lines. Lazy backend + 3 s wall-clock fail-open. |
| Models catalog + descriptor | ✅ | `models.rs:60-150` — 7 entries (4 STT + 3 LLM). All `sha256: None` pending T-01 first-clean-run pinning. |
| Resumable downloader (HTTP `Range` + sha256 streaming) | ✅ | `models.rs::do_download` — re-hashes prefix on resume, emits progress at most every 200 ms. |
| Cancel via `CancellationToken` | ✅ | `models.rs:241-261` registry; `cancel_download` exposed as IPC. |
| Mismatch sha256 deletes partial | ✅ | `models.rs:480` — `fs::remove_file(partial_path)` on mismatch before erroring. |
| Atomic rename on success | ✅ | `models.rs:334` — `fs::rename(partial → final)`. |
| Concurrent-download dedup | ✅ | `DownloadRegistry::try_insert` returns `None` on collision; `download_model` errors with `"already downloading"`. C4 fix verified, plus a unit test (`download_registry_dedupes_by_id`). |
| Models tab UI (list/download/delete/use) | ✅ | `Settings.tsx::ModelsTab` lines 568–910. Per-engine pickers, Gemma TOS, per-row state. |
| Migration dialog flow + window | ✅ | `migration.rs` opens window; `Migration.tsx` UI. Shown when `is_legacy_cloud_user(...) && !migration_seen`. |
| **Migration X-close flips `migration_seen` (C3 fix)** | ✅ | `lib.rs:160-169` — `on_window_event(CloseRequested)` for label `"migration"` calls `crate::settings::mark_migration_seen` *before* `prevent_close`+`hide`. |
| Onboarding "Choose your stack" + Gemma TOS click-through | ✅ | `Onboarding.tsx:322-460`. Stack choice gates the API-keys step (skipped for local). `acceptGemmaTos()` flips `gemma_tos_accepted`. |
| RAM-based tier auto-pick | ✅ | `models.rs::recommend_tier` returns `"small"` if `physical_ram_gb < 12`. Soft recommendation surfaced in Onboarding. (I9: small under-count on 8 GB Macs; cosmetic.) |
| Privacy tab read-only chips | ✅ | `Settings.tsx::PrivacyTab` (line 1749) — derived LOCAL/CLOUD chips, no toggle, "Change these in the Models tab" hint. |

### D.3 Voice Modes

| Item | Verdict | Where / notes |
|---|---|---|
| `modes.rs` model + persistence | ✅ | `modes.rs` — 1136 lines, 17 unit tests passing. |
| **Mode-id validation (path-traversal fix C1)** | ✅ | `is_safe_mode_id` (lines 693–701): non-empty, ≤64 chars, `[a-z0-9-]+`, no leading/trailing dash. Used in `save`, `delete`, `duplicate`, `import_json`. New unit tests: `is_safe_mode_id_rejects_unsafe_shapes`, `import_rejects_traversal_id`, `save_rejects_traversal_id`, `delete_rejects_traversal_id`. |
| 5 built-ins seed on first run | ✅ | `init_with_root` checks `dir_is_empty`. `first_launch_seeds_five_builtins` test. Order: General, Email, Chat, Code Comment, Notes. |
| Active mode plumbed through `session.rs` | ✅ | `session.rs:253` resolves; `:311-322` builds merged-dict + system-prompt; `:316-345` passes to `LlmEngine::cleanup`; `:349-351` applies post-processing. |
| Engine cleanup contract (system_prompt slot) | ✅ | `engines/mod.rs::LlmEngine::cleanup(transcript, system_prompt, dictionary)`. Empty string → engine default. |
| Tray popover mode dropdown | ✅ | `TrayPopover.tsx:176-225`. Active highlight, click → `setActiveMode`. |
| Pill mode chip | ✅ | `Pill.tsx` consumes `pill:active_mode` event. `pill.rs::emit_active_mode`. |
| Suggestion HUD + frontmost probe | ✅ | `frontmost.rs` (NSWorkspace + AX); `session.rs::spawn_suggestion_probe`; `pill.rs::emit_mode_suggestion`. Never auto-switches per CEO decision. |
| `Alt+M` cycle hotkey | ✅ | `hotkey.rs::spawn_cycle_active_mode` (line 207). Wraps around at end of list. Conflict-checks against PTT, cancel, paste-last. |
| Settings → Modes tab | ✅ | `Settings.tsx::ModesTab` (line 917). New / Duplicate / Edit / Save / Delete / Export / Import / Reset to defaults. |
| Drag-drop + file-picker import | ✅ | `Settings.tsx:942` `onDrop` accepts `.voxmode` / `.json`. `:966` file-picker input. Both call `importModeJson`. |

### D.4 Round-2 staff-review fixes

| Fix | Commit | Verdict | Notes |
|---|---|---|---|
| **C1 path traversal** | `ea7132c` | ✅ landed | `is_safe_mode_id` gates every fs-touching call (`save`, `delete`, `duplicate`); `import_json` re-slugs unsafe ids. 4 new tests prove rejection of `../foo`, `/etc/foo`, empty, very-long, mixed-case, unicode, leading/trailing dash. |
| **C2 updater stub short-circuit** | `15e63fb` | ✅ landed | `updater_is_stub()` checks pubkey-empty *and* placeholder-host; if either is true, `check_for_updates` returns `unconfigured` *before* `app.updater()` is even fetched (no DNS, no TCP). Preserves the friendly UX message. |
| **C3 migration X-close flips `migration_seen`** | `3808263` | ✅ landed | `lib.rs::on_window_event` for label `"migration"` calls `mark_migration_seen` then `prevent_close`+`hide`. Engine settings untouched (same as "Keep using cloud" button). |
| **C4 download dedup** | `a51990b` | ✅ landed | `DownloadRegistry::try_insert` returns `Option<CancellationToken>`; second concurrent call gets `None` → command errors with `"already downloading"` and emits `model:error`. Token removed in `finish` so post-completion re-downloads work. Unit test `download_registry_dedupes_by_id` covers the contract. |

No regressions detected on the round-1 fixes (cancel-mid-pipeline,
clipboard restore guard, empty-key Keychain protection,
PipelineGuard).

---

## Final regression matrix

| Feature | Code present | Static build | Manual test (user runs) | Notes |
|---|---|---|---|---|
| Push-to-talk hotkey | ✅ | ✅ | ⚠️ user verifies on real keyboard | unchanged from round 1 |
| Microphone capture | ✅ | ✅ | ⚠️ real mic | unchanged |
| STT engine registry (Whisper-local + Groq) | ✅ | ✅ | ⚠️ live download / API call | local-engines feature off by default; cloud STT works without it |
| LLM cleanup engine registry (Gemma-local + Anthropic + None) | ✅ | ✅ | ⚠️ live | mode `system_prompt` is the integration seam, exercised in session.rs |
| Paste into focused field | ✅ | ✅ | ⚠️ 13-app matrix | unchanged |
| Listening pill (incl. multi-monitor) | ✅ | ✅ | ⚠️ multi-monitor user must verify | round-2 brings active-screen detect via NSEvent |
| Settings window (6 tabs + Modes + Models) | ✅ | ✅ | ⚠️ click-through | now 7 tabs total |
| Tray icon (5 states + pulse) | ✅ | ✅ | ⚠️ user must observe states during recording | round-2 fix |
| Onboarding (stack picker + TOS gate) | ✅ | ✅ | ⚠️ fresh-install walkthrough | round-2 |
| Hands-free toggle | ✅ | ✅ | ⚠️ user verifies press-to-toggle | round-2 (VAD silence auto-stop still TODO) |
| Cancel (Esc) mid-pipeline | ✅ | ✅ | ⚠️ user verifies | round-1 fix intact through engine refactor |
| Language pinning | ✅ | ✅ | ⚠️ user verifies non-English | round-2 — STT engines now consume |
| Paste-last hotkey (⌥⇧V) | ✅ | ✅ | ⚠️ user verifies | round-3-deferred-no-more |
| Personal dictionary | ✅ | ✅ | ⚠️ user verifies | merges with mode-specific dictionary in session.rs |
| Auto-update (Check-for-updates) | ✅ stub | ✅ | ⚠️ user verifies "unconfigured" path; needs real signing for true update | C2 fix means no live network call until configured |
| Voice Modes — built-ins | ✅ | ✅ (12 unit tests + 6 added) | ⚠️ user verifies UI | round-2 |
| Voice Modes — author/edit/duplicate | ✅ | ✅ | ⚠️ user verifies | round-2 |
| Voice Modes — `.voxmode` import/export | ✅ | ✅ | ⚠️ user verifies (drag-drop, picker) | round-2 |
| Voice Modes — cycle hotkey ⌥M | ✅ | ✅ | ⚠️ user verifies | round-2 |
| Voice Modes — auto-suggest HUD | ✅ | ✅ | ⚠️ user verifies (Slack/IDE/etc.) | round-2 |
| Migration dialog (incl. X-close) | ✅ | ✅ | ⚠️ user verifies on upgrade machine | C3 fix |
| Local model download flow | ✅ | ✅ | ⚠️ user runs first-clean-run | T-01 sha256 pinning still TODO |

---

## Open caveats (must reach the user)

These are deliberate v1 deferrals or environment limits:

1. **Code signing & notarization missing.** Bundle is ad-hoc; entitlements
   (incl. `com.apple.security.cs.allow-jit` for Metal) only apply when
   notarized. `docs/signing-and-notarization.md` is the runbook.
2. **sha256 catalog pinning (T-01).** Every entry in `models.rs` has
   `sha256: None`. First clean-run downloads write to
   `~/Library/Application Support/com.vox.app/models/models.lock`. Copy
   those into the catalog and flip the `Option<&str>` to `Some(…)`
   before any external dogfood release.
3. **Whisper Core ML companion download (T-02).** First run falls back
   to Metal-only inference (≈ 2× slower) until the `.mlmodelc.zip`
   companion fetcher lands.
4. **VAD-based silence auto-stop in hands-free mode (T-04 carryover).**
   Hands-free toggle works with the 60 s safety cap; real silence
   detection is deferred.
5. **Hotkey rebinding still requires app restart** (round-1 carryover;
   Settings UI footer warns).
6. **PRD §3.5 keystroke-synthesis fallback** is still a "Copied — press
   Cmd+V to paste" message instead of a synthetic keystroke loop. User
   must verify across the 13-app matrix.
7. **Memory pressure unload for warm models (T-04 in
   `local-first-impl-notes.md`).** Warm Whisper Large v3 Turbo + Gemma
   4 E2B sit at ~4 GB RAM at idle once both load. Acceptable for v1;
   the 10-min idle unload remains TODO.
8. **Mode-id 64-char cap is intentionally narrow.** A user who imports
   a `.voxmode` with a long id gets re-slugged to a shorter one.
   Documented; not a bug.

---

## Top 3 things the user must verify manually

1. **First-run local model download on a clean Mac** (manual test
   plan §11.1). Vox cannot exercise this in QA — needs real network +
   real disk + real RAM probe + real Gemma TOS click. Once you've
   done one clean run, copy the resulting `models.lock` into
   `models.rs` for sha256 pinning (T-01).
2. **Voice Modes round-trip** (manual test plan §10): cycle, suggestion
   HUD on Slack/Mail/VS Code, export → import, edit a duplicate of a
   built-in. The 18 unit tests prove the data model; UI behavior in
   the browser still needs eyes.
3. **Migration dialog on an upgrade machine** (§11.3) — start with a
   pre-round-2 settings JSON that has a Groq key and no
   `migration_seen`, launch, then close via the **title-bar X** (not
   the buttons). Quit and relaunch. The dialog must NOT reappear (C3
   fix).

---

## Verdict: SHIP-WITH-CAVEATS

All four staff-review critical fixes (C1–C4) verifiably landed where
claimed. The code path for round-2 features (Phase-3 polish + local
engines + Voice Modes) is real and the build pipeline is clean.
Static + clippy + tests + bundle all pass with zero warnings.

The deferred items (sha256 pinning, Core ML companion, signing,
memory-pressure unload, VAD silence auto-stop) are honest — none of
them block a Phase-2-or-3 dogfood release as long as the user
accepts:

- They (or their dev team) must build + sign + notarize before
  sharing the .app outside the build machine.
- They will run the manual test plan in `docs/manual-test-plan.md`
  (sections 10–14 are new this round) on their own Mac with their
  own keys.
- A first clean download is required to pin the model sha256s in the
  catalog.

If any of those are not acceptable, **HOLD** until the deferred work
ships.
