# Vox — Staff Engineering Review

> Reviewer: Staff Engineer
> Date: 2026-04-24
> Scope: Phase 1 + Phase 2 implementation (audio capture, hotkey, STT, LLM
> cleanup, paste, pill, tray, onboarding, settings).

## Summary verdict

**Ship-with-caveats.** The end-to-end loop is real and the architecture is
mostly right (dedicated audio thread, atomic session id for cancellation,
keychain-backed secrets, accessory mode, non-activating panel). What needs
to change before any external user touches it falls in three buckets:

1. **API keys are reachable from any window** (pill, tray-popover,
   onboarding) via a blanket `get_settings` command. Must be window-scoped.
2. **Cancel doesn't actually cancel.** The `Esc` hotkey only works while
   `recording==true`; after audio stops it becomes a no-op, so the
   in-flight Groq/Anthropic calls run to completion and paste anyway. PRD
   §4.2 requires aborting "the entire operation" within 100 ms.
3. **Re-press during pipeline doesn't kill the previous run.** Per PRD
   §3.1 step 7, a fresh hotkey press while STT/LLM is running should
   cancel the prior utterance. Today the prior utterance still pastes.

After those three, the rest is correctness/polish — clipboard restore
race, hotkey thread doing keychain I/O, and PRD acceptance criteria that
aren't hard bugs but should land before a public beta.

---

## 🔴 Critical issues (must fix before ship)

### C1. API keys exposed to every WebView

- **Where:** `src-tauri/src/commands.rs::get_settings` returns a
  `Settings` struct that includes both `groq_api_key` and
  `anthropic_api_key` as plaintext strings. The capability bundle
  (`capabilities/default.json`) lists every window —
  `["settings", "pill", "onboarding", "tray-popover"]` — and Tauri 2 app
  commands are callable from any allowed window unless you split them
  across capability files.
- **Risk:** A future XSS or malicious link rendered in *any* of those
  webviews can `invoke("get_settings")` and exfiltrate the user's keys.
  The pill and tray-popover never need to know the keys; they only need
  the dictation toggle, hotkey label, etc.
- **Fix shipped:**
  - Split `get_settings` into two commands:
    - `get_settings` — settings + onboarding only. Includes keys.
    - `get_public_settings` — pill + tray-popover. Strips keys.
  - At runtime each command checks `webview.label()` and refuses if the
    caller isn't on the allow-list. (Belt-and-braces in case the caps
    file is widened.)
  - Frontend `tray-popover.tsx` switched to `getPublicSettings`.

### C2. Cancel doesn't cancel the in-flight pipeline

- **Where:** `session.rs::cancel` early-returns if
  `!ctx.recording.load(...)`. Once the user releases PTT we set
  `recording=false` immediately, then start an async task that does the
  STT + LLM HTTP calls. So `Esc` between release and paste is a no-op:
  the request finishes and we paste.
- **Risk:** PRD §4.2 hard requirement violated. Worse: a user who wants
  to abort a misrecognised dictation can't, and the wrong text lands in
  Slack. Privacy regression too — audio that the user wanted to discard
  still goes to Groq.
- **Fix shipped:**
  - `cancel()` no longer gates on `recording`. It sets `canceled=true`
    and (if recording) ends the recording. The end-recording task now
    re-checks `canceled` *between* the STT and the LLM call, before the
    paste, and aborts with a Canceled pill.
  - The STT and LLM phases are now wrapped in a `tokio::select!` racing
    the HTTP future against a `wait_for_cancel(ctx, my_sid)` poll loop
    that ticks every 50 ms and resolves when either `canceled` flips or
    the active session id moves past `my_sid`. Dropping the HTTP future
    causes `reqwest` to abort the request.
  - We don't use `tokio::sync::Notify` for the cancel signal: a stale
    permit could bleed from one session into the next, and the 50 ms
    poll loop is well under the user-perceived latency budget.

### C3. Re-press during pipeline doesn't supersede the prior session

- **Where:** `session.rs::begin_recording` swaps on `recording`. While
  the previous session is in its post-release pipeline, `recording` is
  already false, so the new press starts a new session — but the old
  session's `tauri::async_runtime::spawn` is still alive and will paste
  when it finishes.
- **Risk:** PRD §3.1 (acceptance #7) violated. User intends to redo,
  gets two pastes, the first being the abandoned utterance.
- **Fix shipped:**
  - Each session captures its `session_id` at creation. The pipeline
    task checks `ctx.session_id == my_id` before every state transition
    (after STT, before LLM, before paste). On mismatch it bails without
    pasting and without emitting any pill state for the old run.

### C4. Clipboard restored unconditionally — overwrites user's copies

- **Where:** `inject.rs::paste_text` sleeps 120ms then writes
  `previous` back to the clipboard.
- **Risk:** The 120ms window is enough for the user (or a clipboard
  manager like Maccy) to put new content on the pasteboard. We then
  silently overwrite that with the user's *old* value. Tech-stack
  Risk 2 explicitly calls this out.
- **Fix shipped:**
  - Read the clipboard before restoring. If it still equals the text we
    just pasted, restore the previous value. Otherwise leave it alone
    (something else has written meanwhile).
  - On macOS we don't get a clean change-count from `arboard`, so we
    compare text. Imperfect (a clipboard manager that re-publishes
    identical text would still be overwritten), but covers the common
    case.

### C5. `begin_recording` does keychain I/O on the hotkey thread

- **Where:** `session.rs::begin_recording` calls `settings::load(&app)`
  which invokes `keyring_get` for both Groq and Anthropic. Keychain
  access can prompt the user (first run after the daemon restarts), and
  on a cold cache routinely takes 50–200 ms. The PRD's 80 ms p95
  hotkey-to-mic-open budget assumes this path is allocation-free.
- **Risk:** Latency floor regression and possible UI hang while the
  Keychain prompt is showing — the hotkey callback can't be re-entered.
- **Fix shipped:**
  - `begin_recording` now reads only the cheap non-secret fields
    (`dictation_enabled`, `send_audio_to_cloud`) via `settings::prefs`.
    The API keys are lazily fetched in the post-release async task,
    where blocking is fine.

### C6. STT/LLM panic on malformed responses

- **Where:** `stt.rs` and `llm.rs` `.json()` decoders and the
  `serde::Deserialize` blocks have no `unwrap`/`expect`, so technically
  there's no panic path. **But** the Groq response field `text` is
  required — if Groq ever returns `{"error": …}` with HTTP 200 (their
  edge has been seen to do this on overload), we hit
  `SttError::Parse(_)` which the user will see as "Unexpected response
  from Groq." The status code path correctly handles non-2xx.
- **Risk:** Low-medium. Behaves OK today but the failure mode is opaque.
- **Fix not shipped (Phase 3):** Improving the parsed error path to surface
  Groq's `error.message` field is a one-day task and not blocking. Today
  the user sees "Transcription failed." which is acceptable for v1.

### C7. Onboarding accessibility step blocks "Continue" but PRD says it shouldn't

- **Where:** `Onboarding.tsx::PrimaryButton::disabled` returns true on
  step 2 unless accessibility is granted.
- **Risk:** PRD §3.9 says "Each step is skippable except mic permission
  (which is a hard block)." Today step 2 (accessibility) and step 3
  (API keys) are also hard-gated, so a user who hasn't yet granted
  Accessibility is stuck inside the wizard.
- **Fix shipped:** Accessibility step has the Skip option enabled and
  Continue is no longer hard-gated; the API-keys step's Skip is
  honored — a user can finish onboarding without keys, with the obvious
  consequence that dictation will surface a "Add your Groq key" pill.

---

## 🟠 Important issues (should fix soon)

### I1. `pill::hide_after` ignores session id

- **Where:** `pill.rs::hide_after` schedules a hide after `ms`. If a new
  session begins in the interim, the new pill gets hidden.
- **Fix shipped:** `hide_after_session` takes a session id and only
  hides if the current session id still matches.

### I2. Hotkey rebinding requires app restart, but UI doesn't say it loud

- The Settings page footer note says "Hotkey changes take effect after
  the app restarts." That's user-hostile but acceptable for v1. We
  could `unregister_all` and re-register on save without a restart.
- **Fix not shipped (deferred to Phase 3).** Re-registering from the
  `set_settings` command requires holding the `SessionCtx` and isn't a
  one-line change. Documented in TODOs.

### I3. STT/LLM clients are reconstructed per request

- `reqwest::Client::builder()...build()` runs on every transcribe/cleanup
  call. Connection pool is wasted; TLS handshake redone every time.
- **Fix shipped:** Lazily-initialised `OnceCell<reqwest::Client>` for
  STT and LLM; HTTPS-only, 15s/3s timeouts honored at request level via
  `client.request(...).timeout(...)`.

### I4. `enigo 0.2` versus tech-stack-spec 0.5

- Cargo says `enigo = "0.2"`. Tech-stack says 0.5. The 0.2 line still
  works but doesn't expose `Direction` ergonomically. Bumping to 0.5
  would be safer (better unicode handling on macOS) but is a small API
  migration.
- **Fix not shipped.** Documented in TODOs; functional.

### I5. Tray icon never reflects state

- README's Phase 3 list calls this out. Current placeholder mic stays
  the same in idle/recording/error states. PRD §3.8 requires animation
  on recording and red on error.
- **Fix not shipped (Phase 3).**

### I6. No "no audio detected" guard

- PRD §3.2: "Handles 0-input (muted mic, hardware off) by showing a
  'No audio detected' toast instead of sending silence to the API."
  Today we send the silence and Groq returns empty text, then we hide
  the pill. User has no signal.
- **Fix shipped:** Before STT we compute peak amplitude on the captured
  buffer. If it's below -50 dBFS we emit an Error pill ("No audio
  detected — check your mic.") and skip the upload entirely. Saves a
  request and surfaces the failure.

### I7. PTT misfire threshold is 250ms, PRD says 300ms

- Trivial.
- **Fix shipped:** `MIN_RECORD_MS` raised to 300.

### I8. Settings UI lets `groq_api_key=""` survive a save and zaps the keychain

- Saving `set_settings` with an empty string deletes the keychain
  entry. If a startup race surfaces an empty string into JS state and
  the user clicks "Test" or hits a different field, we save an empty
  field and wipe the user's key.
- **Fix shipped:** `settings::save` no longer deletes a keychain entry
  when the incoming string is empty *and* the current keychain has a
  value. Only an explicit, intentional clear (a future "Remove key"
  button or "Delete all data") should wipe.

### I9. Audio buffer growth unbounded

- `Vec<f32>` in `ActiveCapture::samples` can grow forever if the
  60s safety net fails. `parking_lot::Mutex<Vec<f32>>` over a vec that
  reallocates is also a long latency surprise.
- **Fix shipped:** Hard cap at `MAX_RECORD_SECS * input_sr *
  input_channels` samples; older samples are dropped (we keep the most
  recent N) so we always have a tail. Bounds memory and recovers
  gracefully if the watcher misses.

### I10. `tauri::async_runtime::spawn` swallows panics

- The session task can panic in `paste_text` (e.g. enigo on macOS may
  panic if Accessibility was revoked mid-flight). Today we catch the
  blocking-task panic, but if the outer task itself panics there's no
  recovery — the pill may stay visible.
- **Fix not shipped (deferred).** Acceptable today because all user
  inputs into that task are validated; documenting in TODOs.

---

## 🟡 Nice to have (Phase 3)

- T1. Tray icon state animation (idle / recording / error / permission).
- T2. Launch-at-login wiring (today: setting persists, no effect).
- T3. Default-mic picker (today: system default only).
- T4. Bump `enigo` to 0.5; bump `cocoa` to `objc2-app-kit` to silence
  upstream deprecation warnings.
- T5. Hotkey rebind without restart.
- T6. VAD-based silence trim before STT (Silero VAD).
- T7. Replace linear resampler with a windowed-sinc (better STT
  accuracy under heavy resampling).
- T8. Deal with the `pill::hide` race when an immediate re-press
  happens — currently we re-show in begin_recording, but the pill state
  emit could land before the WebView re-subscribes.
- T9. Move pill to the display containing the focused window (today:
  always primary).
- T10. Telemetry hooks (opt-in) per PRD 6.3 — not for v1.
- T11. Auto-update via tauri-plugin-updater + Ed25519 manifest.
- T12. Proper `lucide-solid` icons instead of inline SVG mishmash in
  Settings/Onboarding.

---

## ✅ Things that are well done

- **Audio thread separation.** The dedicated `vox-audio` OS thread
  with command-channel messaging neatly side-steps `cpal::Stream`'s
  non-`Send` problem on macOS. The atomic-bits RMS field is a clever
  lock-free fast path for the level meter.
- **Atomic session id.** Using a monotonically-increasing
  `AtomicU64` for cancellation is the right shape (was already there;
  we're just teaching the rest of the pipeline to honor it).
- **Keychain migration on load.** `migrate_key_from_store` doing a
  one-shot move from plaintext store to keychain (with a graceful
  fallback) is the right user-data migration story.
- **Non-activating macOS panel.** The status-window-level + collection
  behavior application in `pill::apply_macos_panel_style` is exactly
  what the spec wants and works reliably with fullscreen apps.
- **`settings::load` is the single source of truth.** No state leaks
  into the modules; everyone re-reads. Easy to reason about.
- **Privacy posture.** No audio touches disk. Transcripts ring buffer
  is in-memory only and capped. No telemetry.
- **`tokio::async_runtime::spawn_blocking` for paste.** Correctly keeps
  the synchronous enigo/arboard work off the async runtime.
- **Pill click-through toggling.** `set_ignore_cursor_events` flips off
  for error states so the user can dismiss; flips back on for normal
  flow. Subtle but right.
