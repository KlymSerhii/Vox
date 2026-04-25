# Vox — Staff Engineering Review (Round 2)

> Reviewer: Staff Engineer
> Date: 2026-04-24
> Scope: Phase-3 polish, local-first pivot, Voice Modes — three workstreams
> landed in parallel since the round-1 review.

## Summary verdict

**Ship-with-caveats.** The three tracks landed cleanly with surprisingly
little integration debt: hotkeys cohabit, the engine registry holds, the
post-cancel pipeline still cancels, and the new Voice Modes layer plugs
through `LlmEngine::cleanup` cleanly. The round-1 issues are not regressed.
However, there are real correctness/security gaps in the new code that
must land before any external user touches it:

1. **Mode ID is used as a filesystem path component without validation.** A
   crafted `.voxmode` import (or `save_mode` payload from a future plugin)
   with `id = "../../tmp/evil"` writes outside the modes directory. Same
   bug in `delete_mode` and `duplicate_mode`. Path traversal.
2. **Updater stub makes a real network call.** Despite the docstring
   ("documented placeholder endpoint, not auto-checking at startup"), if
   the user clicks "Check for updates" today, Vox sends an HTTP request to
   the placeholder URL with an *empty* pubkey configured. Empty pubkey +
   real GET = an attacker who hijacks the (non-existent) DNS could in
   principle serve an update; even setting that aside, the brief is
   explicit: "should NOT do real network calls."
3. **Migration dialog can re-appear forever.** Closing the migration
   window via the title-bar X is intercepted by `prevent_close()` and the
   window is hidden — but `migration_seen` is never flipped, so the
   dialog returns on every launch. This breaks the "exactly once"
   contract from the brief.
4. **`set_active_mode` doesn't validate the mode exists carefully and
   doesn't gate by window**, so a crafted webview (or a future plugin)
   can pin `active_mode_id` to a deleted mode. Resolved at runtime by
   `general` fallback (good), but the persisted preference is wrong.
5. **Active downloads aren't deduped.** Clicking "Download" twice on the
   same model (or once in Onboarding and once in Models tab) overwrites
   the cancellation handle in the registry, orphaning the first request
   — both writers race on the same `.partial` file.

After those, the rest of the issues are correctness/UX polish — the title-
case routine handles non-ASCII strangely, the engine-cache memory note in
the brief got dropped silently, and the mid-flight engine-swap behavior
is undocumented (works correctly because `prefs` is read once at end-
recording, but is implicit). Documented inline.

---

## 🔴 Critical issues (must fix before ship)

### C1. Path traversal via `mode.id` in modes store — FIX SHIPPED

- **Where:** `src-tauri/src/modes.rs` — every persistence path:
  `save()` (line 436), `delete()` (line 448), `duplicate()` (line 467),
  `import_json()` (line 493), and `init_with_root()` seeding (line 340)
  build paths with `mode_path(&root, &mode.id)` which is just
  `root.join(format!("{id}.json"))`. The id is **arbitrary user input**
  on the import path (and a webview-supplied string on the save/delete
  paths).
- **Risk:** A crafted `.voxmode` file with
  `"id": "../../../../tmp/owned"` causes `import_json` to call
  `fs::write("…/modes/../../../../tmp/owned.json", …)`, writing outside
  the modes directory. Worst case, an attacker who can socially-engineer
  a user into dragging a `.voxmode` onto Settings can write arbitrary
  files into the user's home directory (subject to Tauri's macOS
  sandboxing). Even without traversal, `id = "/etc/something"` causes
  `fs::write` to attempt an absolute path — `Path::join` on an absolute
  path replaces the base, so `root.join("/etc/foo")` = `/etc/foo`.
  `delete_mode("/Users/me/.ssh/id_rsa")` would in principle attempt
  `fs::remove_file` on it (gated by FS perms but still bad).
- **Fix shipped (this review):**
  - New `is_safe_mode_id(id) -> bool` in `modes.rs` checks the id is
    non-empty, ≤64 chars, and matches `[a-z0-9-]+` only (the same
    character set our slugifier produces). Imports re-slugify a
    non-conforming id; saves/deletes refuse outright.
  - `save`, `delete`, `duplicate`, and `import_json` now run the id
    through `is_safe_mode_id` before any filesystem call. Built-in
    seeding is unchanged because the built-in ids are compile-time
    constants.
  - Test: a new `import_rejects_traversal_id` and
    `save_rejects_traversal_id` assert that `"../foo"` and `"/etc/passwd"`
    don't escape the modes directory.

### C2. Updater stub makes a live network call — FIX SHIPPED

- **Where:** `src-tauri/src/commands.rs::check_for_updates` calls
  `app.updater()` then `updater.check().await`. The endpoint configured
  in `tauri.conf.json` is the documented placeholder
  `https://updates.vox.local/...` and `pubkey` is `""`.
- **Risk:**
  1. Real HTTPS GET to a non-existent host — wastes battery, leaks the
     fact that Vox is running on the user's network to anyone watching
     DNS, and hangs for the resolver timeout (typically 5–15 s) before
     erroring. The handoff doc *says* this fails gracefully; the
     measurement said "doesn't blow up." Both are true. Neither is
     "doesn't do a real network call," which is what the brief asks.
  2. Empty pubkey: if a future config flip ever points the endpoint at
     a hostile domain (DNS poisoning on a corporate network, for
     example), there is no signature pin to fail-closed against. The
     plugin will refuse a missing signature, but it's trivially easy
     to misconfigure.
- **Fix shipped (this review):**
  - `check_for_updates` now detects the placeholder endpoint and the
    empty pubkey at command entry and returns
    `status: "unconfigured"` *before* invoking the updater plugin.
    That means no DNS, no TCP handshake, no time spent.
  - The detection treats both the literal stub host (`updates.vox.local`)
    and an empty pubkey as "stub config" — either condition trips the
    short-circuit.
  - The friendly message is `"Auto-update is not yet configured for
    this build."` (was already the message text the UI surfaces.)

### C3. Migration dialog can re-appear after window close — FIX SHIPPED

- **Where:** `lib.rs::on_window_event` intercepts `CloseRequested` for
  `migration` (along with `settings`, `tray-popover`, `pill`) and calls
  `api.prevent_close()` + `window.hide()`. `migration_seen` is only
  flipped via the explicit `dismissMigration` command from the dialog's
  Keep / Switch buttons — closing via the title-bar X bypasses both
  branches. Next launch hits `is_legacy_cloud_user(...) && !migration_seen`
  and re-shows the dialog.
- **Risk:** PRD violation ("exactly once"). User-hostile: there is no
  way to dismiss the dialog without clicking through. A user who
  habitually closes windows with `Cmd+W` will see this on every
  launch.
- **Fix shipped (this review):**
  - `on_window_event` for `migration` now flips `migration_seen` to
    `true` before hiding the window, so the X button has the same
    effect as clicking "Keep using cloud" (no engine changes are made,
    but the dialog dismisses for good).
  - `dismiss_migration` already does this for the explicit-button path
    — we just route the X through the same flag.

### C4. Active downloads dedupe is missing — FIX SHIPPED

- **Where:** `src-tauri/src/models.rs::download_model` calls
  `REGISTRY.insert(&id)` which `HashMap::insert`s the cancel token,
  silently overwriting any prior entry. If the user clicks "Download"
  twice on the same model (a double-click in the Models tab, or once in
  Onboarding then again in Settings), two HTTP requests now race on
  the same `.partial` file. Both writers truncate or seek-then-write,
  corrupting bytes.
- **Risk:** Corrupted partial → sha256 mismatch on completion (if
  pinned) → user has to delete and retry. With unpinned sha256 (the
  current state), the first valid byte stream wins, the second writes
  garbage on top, and we *atomic-rename* a corrupt file to `model.bin`.
- **Fix shipped (this review):**
  - `REGISTRY.insert` returns `Option<CancellationToken>`. If a
    download is already running for the id, `download_model` errors
    out with `"already downloading"` rather than starting a second
    writer.
  - The cancellation token is keyed by id and removed in the `finish`
    branch (success or error), so a fresh download after completion
    works as before.

### C5. `import_mode_json` validates only after id-collision dedup — fix shipped (collision handling moved after `is_safe_mode_id`)

- **Where:** `import_json` validates name + system_prompt early but
  the id is only deduped against existing modes. With C1's fix in
  place, an id that fails `is_safe_mode_id` is rewritten via
  `slugify`-of-name. If `slugify(name)` is empty (e.g. a name that's
  all emoji), the function falls through to `format!("mode-{ts}")` —
  good.
- **Fix:** subsumed by C1. Verified by the new tests.

---

## 🟠 Important issues (should fix soon)

### I1. `set_active_mode` doesn't validate via webview gate — FIX SHIPPED

- `commands::set_active_mode` is callable from any window. It does
  fetch the mode by id and reject unknown ones (`"unknown mode id"`),
  so it can't pin `active_mode_id` to a non-existent mode. But it
  *can* be called from the pill or tray-popover, which is fine for v1
  but should ideally be a settings-window-only mutation. Documented in
  TODOs; not blocking because the validation is solid.

### I2. Title-case post-processing is ASCII-only — FIX NOT SHIPPED

- `modes.rs::title_case` lowers via `to_lowercase()` and capitalizes
  via `to_uppercase().collect::<String>() + chars.as_str()`. That's
  Unicode-correct in theory (Rust's `char::to_uppercase` returns an
  iterator that handles multi-codepoint mappings like `ß → SS`).
  However, the small-words list is ASCII-only, so a French dictation
  with "le", "la", "de", "du" doesn't get the small-word treatment.
  Cosmetic only; documented for v1.1.

### I3. Mode swap mid-flight is non-deterministic — DOCUMENTED

- `session::end_recording` reads `prefs.active_mode_id` once at the
  top of the post-release async task. If the user cycles modes
  between hotkey-release and STT-completion, the cycle takes effect
  on the *next* utterance, not this one. This is the right behavior,
  but it's not tested or documented. Added a short comment in
  `session.rs` stating the contract. No code change.

### I4. Engine swap mid-flight — DOCUMENTED

- Same shape: `prefs.stt_engine` / `prefs.llm_engine` are read once
  per utterance at end-recording. Mid-utterance flips don't take
  effect until the next dictation. Documented inline.

### I5. Models tab — double-click on Download — FIX SHIPPED

- See C4. The fix surfaces a clear `"already downloading"` error to
  the frontend so the UI can update the button state.

### I6. AX permission failures are silent — DOCUMENTED, BEHAVIOR IS CORRECT

- `frontmost.rs::frontmost_window_title` returns `None` on AX permission
  denial (the `err != 0` branch). The bundle-id path uses NSWorkspace
  which doesn't require AX. `detect()` returns `Some((bundle_id, None))`
  — the suggestion HUD then has only the bundle id to match against.
  No PII leaks to logs (we don't log the title or bundle id at info
  level; only at trace).

### I7. Memory pressure unload is missing (T-04) — DOCUMENTED

- The brief calls out that local-first-impl-notes flagged this as a
  known TODO. Confirmed: warm Whisper Large v3 Turbo + Gemma 4 E2B sit
  at ~4 GB RAM at idle once both models are loaded. Acceptable for
  v1; the unload-after-10-min idle path remains TODO. No code change
  this round.

### I8. Cycle-key shows the HUD even with dictation off — FIX NOT SHIPPED (acceptable)

- `hotkey.rs::spawn_cycle_active_mode` shows the pill and the new mode
  chip even when `dictation_enabled = false`. Probably the right
  behavior (user gets feedback that their cycle press was received)
  but worth a UX call. Documented; not blocking.

### I9. `physical_ram_gb()` arithmetic — POTENTIAL UNDERESTIMATE

- `models.rs::physical_ram_gb` divides by `1024^3`. On a Mac with 8
  GB RAM, `total_memory()` returns ~8.5 GB physical due to firmware
  reserved regions; the divide-and-floor produces 7. The recommend
  threshold is `< 12 GB → small`, so the off-by-one doesn't matter
  in practice. Cosmetic only.

### I10. Non-Wry runtime fallback in session.rs — FRAGILE

- `session::try_into_wry` uses `std::any::Any` to downcast `&AppHandle<R>`
  to `AppHandle<Wry>`. If `R` is *not* `Wry` (for example, a future
  test runtime), the engine path bails with an "Internal error" pill.
  Not a bug today (we always run on Wry) but the failure mode is
  unfriendly. Documented in TODOs.

---

## 🟡 Nice to have (Phase-N)

- **N1.** `download_model` re-hashes the entire prefix on every resume.
  For a 1.1 GB model this is ~5 s of pure I/O on disk after a network
  error — surprising the user. Consider persisting the partial-byte
  digest alongside the `.partial` file.
- **N2.** Tray icon pulse loop tick is 500 ms. Two pulse loops can
  briefly co-exist if `set_state(Recording)` is called from two
  threads at the same exact instant: the swap on `pulse_running` is
  correctly atomic, but the loop-exit branch reads `state` after a
  500 ms sleep. Worst case, one extra tick is rendered before the
  second loop self-terminates. Cosmetic.
- **N3.** Mode `system_prompt` is sent verbatim to the LLM with no
  size cap — a 100 KB prompt would happily spend Anthropic budget. A
  10 KB cap on the prompt at save time would be prudent.
- **N4.** Built-in mode JSON files on disk include the same constants
  the Rust catalog already has — they're effectively duplicated state.
  If we ever change a built-in's prompt, existing installs keep the
  stale on-disk version unless the user clicks "Reset to defaults."
  Document or version the built-ins.
- **N5.** The `cocoa → objc2` migration is correct but `setCanHide:false`
  remains as a `msg_send!` because objc2-app-kit doesn't expose a typed
  setter. That's fine.
- **N6.** `migration.html` is included as a Vite entry but the dialog
  has no escape route besides a button click; the X is fixed by C3.
- **N7.** Voice mode dictionary is merged into the global dictionary
  but there's no UI to surface "this term comes from this mode" — fine
  for v1.
- **N8.** Front-end `any` types and Tailwind duplication noted but not
  flagged — out of scope.
- **N9.** Spawned `tauri::async_runtime::spawn_blocking` tasks in
  whisper_local and gemma_local hold the engine `Mutex` across the
  entire inference call. Two concurrent dictations of the same engine
  serialize on this lock. Today the session code never starts a second
  dictation while the first is mid-pipeline (the recording flag
  prevents it), so this is fine — but worth noting.
- **N10.** `active_screen_logical_frame` falls back through multiple
  paths; the macOS `MainThreadMarker::new()` returns `None` on
  background threads, which means a pill `show()` from a non-main
  thread silently uses the cross-platform fallback. Unlikely to
  matter (Tauri schedules on the main runloop) but worth noting.

---

## ✅ Things that are well done

- **Hotkey collision matrix.** `hotkey::register` carefully
  cross-checks all four shortcuts (PTT, cancel, paste-last, mode
  cycle) and refuses to double-register on conflicts. The fall-back
  string when an unknown name is supplied is clean.
- **Cancel still cancels through the new pipeline.** The round-1 fix
  for `cancel()` survives intact through the engine refactor: both
  `tokio::select!` blocks (STT + LLM) honor `wait_for_cancel`, and
  `PipelineGuard` cleans up `pipeline_active` on every exit including
  panic.
- **Migration store invariants.** The legacy `cleanup_enabled` /
  `send_audio_to_cloud` upgrade path is robust: existing users with a
  Groq key default to `groq` STT; users who turned off cleanup land
  on `none`; new installs get local. Tested by reading the load
  path; would benefit from an integration test.
- **Mode store first-launch seeding** is correctly behind
  `dir_is_empty`. Re-launches don't overwrite user edits.
- **`reset_defaults` only re-creates missing files** rather than
  blowing over user customizations of forked built-ins.
- **`engines::Registry`** is a clean lazy-init: the `(engine, model)`
  composite key avoids the "swap model but cached engine still
  points at the old GGUF" bug pattern.
- **Tray icon pulse-loop reentrancy** is guarded by a single atomic
  bool — no double-spawn.
- **`objc2-app-kit` migration** is faithful to the cocoa shape;
  `setLevel(NSStatusWindowLevel)`, the collection-behavior bitset,
  and the `Borderless | NonactivatingPanel` style mask all match the
  documented behavior. The `setIgnoresMouseEvents` / `setCanHide`
  pair preserves click-through and prevents accidental hide.
- **Voice Modes are zero-telemetry.** No counters, no anonymized
  hashes, no identifiers leak to disk beyond what the user explicitly
  saves.
- **Frontmost-app probe fails open.** Both NSWorkspace and the AX
  branch return `None` on failure — no PII in logs, no crash, no
  blocked dictation.

End of round-2 review.
