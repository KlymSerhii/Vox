# Vox — Manual Test Plan

> Audience: human tester with their own macOS Mac, mic, Groq key, and
> Anthropic key.
> Time budget: ~45 minutes for the full plan, ~10 minutes for the
> "smoke" subset (marked **[SMOKE]**).
> Reference: `docs/product-requirements.md`, `docs/test-report.md`.

This file is the part of QA that the automated tester can't do for you:
real permissions, real audio, real text injection into real apps. Run
through it on a fresh Mac (or after `Delete all local data`, when that
feature ships) before any external dogfood release.

## Before you start

You will need:
- A Mac on macOS 13 (Ventura) or later, Apple Silicon.
- A working microphone (built-in is fine).
- A Groq API key (`gsk_…`) from https://console.groq.com.
- An Anthropic API key (`sk-ant-…`) from https://console.anthropic.com.
- Slack, Chrome (or Safari), VS Code or Cursor, TextEdit, Notion (web is
  fine), and at least one terminal app open. Optional but recommended:
  Discord, iTerm2, Obsidian, Linear, Figma.

If you ran the dev build before, reset state:

```sh
rm -rf "$HOME/Library/Application Support/com.vox.app"
# also delete keychain entries:
security delete-generic-password -s app.vox.vox -a groq        2>/dev/null
security delete-generic-password -s app.vox.vox -a anthropic   2>/dev/null
# revoke previous TCC grants (Microphone & Accessibility) for "Vox" or
# "vox" in System Settings → Privacy & Security → Microphone /
# Accessibility, otherwise old grants leak across builds.
```

---

## 0. Install **[SMOKE]**

1. Build the release `.app`:
   ```sh
   cd /Users/serhiiklymenko/Projects/Vox
   npm install
   npm run tauri build -- --bundles app
   open src-tauri/target/release/bundle/macos/
   ```
2. Drag `Vox.app` into `/Applications`. (You can also run from the
   build directory.)
3. Right-click → Open the first time (Gatekeeper warning — expected,
   bundle is ad-hoc signed).
4. **Expected:** A small mic-shaped icon appears in the menu bar. No
   Dock icon (LSUIElement is true). Onboarding window opens.
5. **Pass / Fail:** ___

---

## 1. Onboarding flow

1. **Welcome step** — click "Get started." **Expected:** advance to
   step 1.
2. **Microphone step [SMOKE]** — click "Allow microphone." macOS prompts;
   click "OK." Within 2 s the bullet should turn green ("Granted").
   - **Expected:** "Continue" becomes enabled.
   - **Fail case to flag:** Continue stays disabled even after the
     permission dialog says granted. Wait up to 4 s for the 2 s poll.
3. **Accessibility step** — click "Open System Settings." Toggle Vox on
   in Privacy → Accessibility. Return to the onboarding window.
   - **Expected:** within 4 s the bullet turns green ("Granted —
     restart Vox after adding it"). The wizard text is honest that you
     need to quit + re-open Vox before the global hotkey will bind.
   - **Skippable:** clicking "Skip" advances anyway (PRD §3.9).
4. **API keys step [SMOKE]** — paste your Groq key. Click "Test" — the
   green check should appear within ~1 s. Optionally paste the Anthropic
   key and Test it.
   - **Expected:** green ✓ / red ✗ badges; messages match.
   - **Fail case to flag:** Test button gives "Failed: …" with a 401 →
     bad key. With a 403 → wrong scope. With a network error → check
     internet.
5. **Try-it step** — focus the textarea inside the wizard. **DO NOT**
   try to dictate yet (you may need to restart for the global hotkey to
   bind). Click "Skip for now" or "Finish." Onboarding closes.
6. **Quit and relaunch Vox** so the global hotkey binds with the now-
   granted Accessibility permission.
7. **Pass / Fail:** ___

---

## 2. Tray icon + popover **[SMOKE]**

1. Click the menu-bar icon. **Expected:** a popover anchored beneath the
   icon, showing:
   - Status row with green "Listening ready" (if both keys set) or grey
     "Add your Groq key" (if no key).
   - Dictation toggle (default on).
   - Recent transcripts (empty initially).
   - Hotkey label (default ⌥).
   - Settings… / Quit footer.
2. Click outside the popover. **Expected:** popover hides.
3. Right-click the icon. **Expected:** native fallback menu — Open
   Settings / Quit Vox.
4. Toggle Dictation off in the popover, then close it.
5. **Pass / Fail:** ___

---

## 3. Push-to-talk **[SMOKE]** (the golden path)

This is the headline scenario. Test it carefully.

1. **Re-enable** the dictation toggle in the popover.
2. Open TextEdit → New document. Click in the document so the cursor
   is in the text area.
3. Press and hold **Right Option** (⌥). Within ~100 ms a small dark pill
   should appear at the bottom-center of your primary screen. The mic
   icon pulses; a live waveform reacts to your voice.
4. Say: *"Hello world. This is a quick test of Vox."*
5. Release ⌥. The pill changes to **Transcribing** (blue spinner), then
   **Polishing** (violet spinner — only if Anthropic key is set and
   cleanup is on), then a green check (**Pasted**), then fades.
6. **Expected:** the cleaned text appears at the cursor in TextEdit
   within ~1500 ms of release. No focus stolen; TextEdit stays focused.
7. **Pass / Fail:** ___

### 3a. Dictation states

For each release, confirm you saw all the right states:
- [ ] Pill faded in within ~100 ms of keydown
- [ ] Pulsing mic icon during listening
- [ ] Live waveform actually reacted to voice (not a static fake animation)
- [ ] State changed to Transcribing right after release
- [ ] State changed to Polishing only when cleanup is enabled
- [ ] Done check shown, then pill faded
- [ ] No Vox window stole focus

---

## 4. Test phrases

Run each phrase in TextEdit. Note the cleaned output Vox produces.

| # | Phrase | Cleanup expectation |
|---|---|---|
| 1 | "hello world" | "Hello world." |
| 2 | "hey team um lets push the launch to next thursday actually no next friday the eighth because the staging environment is still flaky" | One sentence, "Friday the 8th," no "um," "actually" gone. |
| 3 | "send the email to maria at acme dot com" | "Send the email to maria@acme.com." (Whisper may already format it; LLM should not invent a different domain.) |
| 4 | "in python you write list comprehensions like x for x in items if x is not none" | Keeps code-y phrasing; should not formalize. |
| 5 | "remember to buy milk eggs uh actually no buy oat milk and eggs" | "Remember to buy oat milk and eggs." |
| 6 | "klymenko" (with the dictionary entry "Klymenko") | Capital K, exact spelling. Add "Klymenko" in Settings → Dictionary first. |

**Pass criteria:** none of these should add invented facts (no extra
names, links, or "Best regards," signatures). LLM cleanup MUST NOT
prepend "Here is the cleaned text:" or any preamble.

---

## 5. App-coverage matrix

For each app: focus a text input, hold ⌥, say "this is a test in
[appname]," release. Confirm the text lands at the cursor and Vox does
not steal focus.

| App | Result | Notes |
|---|---|---|
| TextEdit | ___ | |
| Slack message composer | ___ | |
| Slack search field | ___ | |
| Chrome address bar | ___ | |
| Chrome web form (e.g. gmail.com Compose) | ___ | |
| Safari address bar | ___ | |
| VS Code editor | ___ | |
| VS Code terminal panel | ___ | |
| Cursor editor | ___ | |
| Notion (web) page body | ___ | |
| Messages composer | ___ | |
| Mail composer | ___ | |
| Terminal.app | ___ | |
| iTerm2 | ___ | |
| Discord composer | ___ | |
| Obsidian editor | ___ | |
| Linear issue body | ___ | |
| Figma comment | ___ | |

**Known potential failure modes (per PRD §3.5 fallback):**
- Apps that block synthetic paste (1Password, banking sites). Today
  Vox shows "Copied — press Cmd+V to paste" instead of synthesizing
  Unicode keystrokes (the PRD's fallback is **not** implemented in v1).
  If a target app fails, you'll get the "Copied — press Cmd+V" pill.
- Some apps (e.g., Figma file canvas) only have a text cursor inside
  specific modes — make sure the cursor is in a real input.

---

## 6. Edge cases

### 6.1 Misfire (hold <300 ms) **[SMOKE]**
1. Tap ⌥ very briefly (<300 ms).
2. **Expected:** pill flashes briefly then disappears. **No paste.**
   Nothing in TextEdit.
3. **Pass / Fail:** ___

### 6.2 Long hold (>60 s)
1. Open TextEdit. Hold ⌥ and start counting "one, two, three…"
2. Around 60 s you should see the pill change to Transcribing
   automatically without you releasing.
3. **Expected:** safety cap at 60 s; the audio captured up to that
   point is transcribed.
4. Release the key. **Expected:** no second session starts; the running
   transcribe finishes and pastes.
5. **Pass / Fail:** ___

### 6.3 Cancel mid-recording (Esc)
1. Hold ⌥, start saying "this should be cancelled…"
2. Without releasing ⌥, press **Esc**.
3. **Expected:** pill flashes "Canceled" then hides. **Nothing pasted.**
4. **Pass / Fail:** ___

### 6.4 Cancel mid-pipeline (Esc after release)
1. Hold ⌥, say a sentence, release ⌥.
2. While the pill shows Transcribing or Polishing, press **Esc**.
3. **Expected (post QA-fix F1):** pill switches to Canceled within
   ~50 ms; the in-flight Groq/Anthropic request is dropped; nothing is
   pasted into TextEdit. (Pre-fix this would silently complete and
   paste anyway — this is the staff review C2 case that was incomplete.)
4. **Pass / Fail:** ___

### 6.5 Re-press supersedes a previous in-flight session
1. Hold ⌥, say "first message," release.
2. **Immediately** (while pill shows Transcribing or Polishing) press
   and hold ⌥ again, say "second message," release.
3. **Expected:** Only the **second** message is pasted into TextEdit.
   The first one's pipeline is silently abandoned (no paste, no Error).
4. **Pass / Fail:** ___

### 6.6 Network disconnected mid-STT
1. Disable Wi-Fi.
2. Hold ⌥, say something, release.
3. **Expected:** within 15 s the pill shows red "Offline — Vox needs
   internet." for ~3 s, then hides. Nothing pasted. App does not crash.
4. Re-enable Wi-Fi. Try again.
5. **Pass / Fail:** ___

### 6.7 Invalid API key
1. Settings → API Keys → Groq → replace with `gsk_invalid`. Save (auto-
   saves on input, give it a beat).
2. Hold ⌥, say something, release.
3. **Expected:** pill shows red "Groq key rejected." for ~3 s.
4. Restore the real key. Try again — works.
5. **Pass / Fail:** ___

### 6.8 OS-muted mic (no audio detected guard)
1. Mute your input device at the OS level (System Settings → Sound →
   Input → drag input volume to 0).
2. Hold ⌥, "speak" (silence), release.
3. **Expected:** pill shows red "No audio detected — check your mic."
   for ~2.5 s. **No request was sent to Groq** — verify on Groq's
   dashboard (the request count should not increase).
4. Restore input volume.
5. **Pass / Fail:** ___

### 6.9 Accessibility revoked mid-session
1. Mid-recording: open System Settings → Privacy & Security →
   Accessibility → toggle Vox **off**. Release ⌥.
2. **Expected:** pill shows an error (likely "Paste error." or "Copied —
   press Cmd+V to paste."). The text should be on the clipboard. App
   does not crash.
3. Re-enable Accessibility. Quit + relaunch Vox to restore the global
   hotkey binding.
4. **Pass / Fail:** ___

### 6.10 Empty / silent recording
1. Hold ⌥ for ~2 seconds without speaking, release.
2. **Expected:** "No audio detected" or pill simply hides (Vox sends
   silence to Groq → empty text → we hide). No paste.
3. **Pass / Fail:** ___

### 6.11 Cancel hotkey when nothing is happening (idle)
1. With **no** session in flight, press Esc.
2. **Expected (post QA-fix F1):** nothing happens. No stray pill, no
   visual artifact. Esc is owned by other apps as usual.
3. **Pass / Fail:** ___

---

## 7. Settings & persistence

1. Open Settings via tray popover → "Settings…" or right-click tray →
   Open Settings. **Expected:** window appears focused.
2. Toggle Cleanup off. Hold ⌥, dictate "this is a test." **Expected:**
   pill skips Polishing state; raw Whisper text is pasted.
3. Toggle Cleanup back on.
4. Add "Klymenko" to Dictionary. Dictate "klymenko is here." **Expected:**
   the cleaned output uses capital K.
5. Privacy → Send audio to cloud → off. Hold ⌥. **Expected:** pill shows
   "Cloud dictation is off — enable it in Privacy." for ~2.5 s. Toggle
   back on.
6. Hotkeys → change push-to-talk to F13 (or another). **Expected:**
   footer warns "Hotkey changes take effect after the app restarts."
7. Quit and relaunch Vox. **Expected:** F13 is now the hotkey; ⌥ no
   longer triggers Vox.
8. Set the hotkey back to Right Option (⌥) and restart Vox again.
9. About tab → click "Replay onboarding." **Expected:** wizard opens.
10. **Pass / Fail:** ___

### 7a. Keychain test
1. After step 1 above, run:
   ```sh
   security find-generic-password -s app.vox.vox -a groq -w
   security find-generic-password -s app.vox.vox -a anthropic -w
   ```
2. **Expected:** your saved keys are returned. Keychain UI may prompt
   you to allow access — say Always Allow for Vox the first time.
3. **Pass / Fail:** ___

---

## 8. Privacy posture (ad-hoc audit)

1. While Vox is running, run:
   ```sh
   ls -la "$HOME/Library/Application Support/com.vox.app/"
   ```
   **Expected:** `vox-settings.json` exists; **no audio files**, no
   transcript history files.
2. Hold ⌥, dictate, release. Re-run the `ls`. **Expected:** still no
   audio files.
3. **Pass / Fail:** ___

---

## 9. Quitting & relaunch

1. Click tray → Quit Vox. **Expected:** menu-bar icon disappears.
2. Reopen Vox.app. **Expected:** menu-bar icon back; **no onboarding**
   (since we already finished it). Recent transcripts in tray popover
   are now empty (memory-only ring buffer).
3. **Pass / Fail:** ___

---

## 10. Voice Modes (Round-2 differentiator)

### 10.1 Built-ins seed on first run **[SMOKE]**
1. Settings → **Modes** tab.
2. **Expected:** five built-in modes are listed in this exact order:
   General, Email, Chat, Code Comment, Notes. Each row shows an icon,
   name, and short description; the active one is marked with a check.
3. Confirm `~/Library/Application Support/com.vox.app/modes/` contains
   five files (`general.json`, `email.json`, `chat.json`,
   `code-comment.json`, `notes.json`).
4. **Pass / Fail:** ___

### 10.2 Mode chip on listening pill **[SMOKE]**
1. With the Modes tab still active, click "Use" on Email.
2. Click in TextEdit. Hold ⌥. **Expected:** the listening pill shows a
   small "📧 Email" chip in addition to the waveform during recording.
3. Release. The cleanup output should be in formal-sentence form (no
   "hi"/"thanks" greeting auto-added).
4. **Pass / Fail:** ___

### 10.3 Tray popover dropdown
1. Click the menu-bar icon. **Expected:** a "Voice mode" row appears
   below the Dictation toggle, showing the active mode and a list of
   all modes.
2. Click "Chat" in the dropdown. Close the popover. Hold ⌥, dictate,
   release. **Expected:** chat-style cleanup (lowercase ok, minimal
   punctuation, capped at ~280 chars).
3. **Pass / Fail:** ___

### 10.4 Cycle hotkey ⌥M
1. Without recording, press **⌥M**. **Expected:** the pill flashes the
   next-mode chip for ~800 ms then hides. Each press advances to the
   next mode in display order; wraps around at the end.
2. **Expected (per round-2 review I8):** the cycle HUD also fires when
   `dictation_enabled = false` — this is intentional.
3. **Pass / Fail:** ___

### 10.5 Suggestion HUD (auto-suggest)
1. Settings → Modes → confirm "Show suggestion HUD" is on. Active mode
   is, say, "General."
2. Open Slack. Click into the message composer. Hold ⌥, start
   speaking.
3. **Expected:** within ~250 ms the right edge of the pill shows
   "Switch to Chat? ⌥M". Pressing ⌥M during the recording switches the
   mode for the *next* utterance (not this one — see I3 in code review).
4. **Expected (mode-swap mid-flight contract):** if you cycle modes
   *between* hotkey-release and STT-completion, the swap takes effect
   on the next dictation. The current utterance was already committed
   to the mode active at end-recording.
5. Settings → Modes → toggle "Show suggestion HUD" off. Repeat — no
   hint should appear.
6. **Pass / Fail:** ___

### 10.6 Duplicate / edit / save
1. Settings → Modes → click "Duplicate to edit" on Email.
2. **Expected:** a new "Email (Copy)" mode appears with `built_in:
   false`; the editor opens with all fields editable.
3. Change Name to "My Email", trim the system prompt, click Save.
4. **Expected:** the row updates; reload Settings → Modes; the change
   persists.
5. Try saving an edit on a built-in mode (drive Save directly via
   devtools if needed). **Expected:** rejected with `is_builtin`.
6. **Pass / Fail:** ___

### 10.7 Export / import .voxmode
1. Click "Export" on the "My Email" mode from §10.6. **Expected:** a
   `my-email.voxmode` (or similar slug) file lands in `~/Downloads`.
2. **Drag** the `.voxmode` back onto the Modes tab. **Expected:** Vox
   imports it; because the id collides with the original, the new id
   gets a `-2` suffix; `built_in` stays false.
3. Click "Import .voxmode" and pick the same file again. **Expected:**
   id becomes `-3`. Each file roundtrips with no field loss.
4. **Pass / Fail:** ___

### 10.8 Reset to defaults
1. Manually delete `email.json` from `~/Library/Application
   Support/com.vox.app/modes/` in Finder.
2. Settings → Modes → click "Reset to defaults".
3. **Expected:** Email reappears in the list; user-edited modes are
   not touched.
4. **Pass / Fail:** ___

---

## 11. Local-model setup (Round-2 local-first pivot)

### 11.1 First-run download flow **[SMOKE]**
1. After a clean install (see "Before you start"), open Onboarding.
2. Reach "Choose your stack" — by default "Run offline (recommended)"
   is selected.
3. **Expected (low-RAM Mac, < 12 GB):** the tier auto-pick shows
   "Small tier" with a soft banner explaining why; you can override.
4. **Expected (≥ 12 GB):** Balanced tier is recommended.
5. Accept the Gemma TOS click-through. **Expected:** "Continue"
   becomes enabled; clicking starts a parallel Whisper + Gemma
   download with a single progress bar.
6. **Expected:** progress events fire at most every 200 ms (no spam).
   ETA decreases monotonically; total bytes match the catalog.
7. On a flaky network, kill Wi-Fi mid-download and re-enable.
   **Expected:** a resume picks up via HTTP `Range`; sha256 stays
   correct.
8. **Pass / Fail:** ___

### 11.2 Models tab — switch tier
1. Settings → Models. **Expected:** rows show Downloaded /
   Downloading X% / Use / Delete. The default Whisper + Gemma rows
   are marked Downloaded after onboarding.
2. Click "Download" on a Quality tier model. While downloading, click
   "Download" on the *same* row again.
3. **Expected (round-2 C4 fix):** the second click is rejected with a
   visible "already downloading" error — *not* a silent overwrite
   that races on the same `.partial` file.
4. Cancel. **Expected:** the partial file is preserved on disk
   (resumable). Click Download again — resume kicks in.
5. Switch active STT to the Quality model via "Use." Dictate. Verify
   the engine label flips in the privacy chip.
6. **Pass / Fail:** ___

### 11.3 Migration dialog **[SMOKE — upgrade users only]**
Skip if you are not upgrading from a pre-round-2 install.

1. Start with a settings store that has `groq_api_key` set,
   `cleanup_enabled = true`, `send_audio_to_cloud = true`,
   `migration_seen` absent.
2. Launch Vox. **Expected:** the migration dialog appears once.
3. Click the title-bar **X** to close (do *not* click either of the
   two dialog buttons).
4. **Expected (round-2 C3 fix):** quit and relaunch — the dialog does
   *not* re-appear. `migration_seen` was flipped on title-bar close.
5. **Pass / Fail:** ___

### 11.4 Privacy chips
1. Settings → Privacy. **Expected:** read-only chips show LOCAL or
   CLOUD per data flow direction (audio + transcript) based on the
   active engines.
2. Switch the STT engine to Groq in Models. **Expected:** the audio
   chip flips to CLOUD. The chips are derived, not editable here.
3. **Pass / Fail:** ___

### 11.5 No GGUF in the bundle
1. Build with `npm run tauri build -- --bundles app`.
2. `find src-tauri/target/release/bundle/macos/Vox.app -name "*.gguf"
   -o -name "*.bin" -o -name "*.ggml"` — should output nothing.
3. **Expected:** bundle stays under ~25 MB; models live only in
   `~/Library/Application Support/com.vox.app/models/`.
4. **Pass / Fail:** ___

---

## 12. Multi-monitor pill placement

1. Connect a secondary display (or use a virtual one). Move your
   cursor onto the secondary screen and hold ⌥.
2. **Expected:** the pill renders bottom-center of the *secondary*
   screen, 96 px above the bottom edge.
3. Move the cursor back to the primary; press again. **Expected:**
   pill moves to the primary.
4. Try a screen *above* the primary (System Settings → Displays →
   drag the secondary above). **Expected:** the AppKit↔Tauri
   coordinate flip is correct (Tauri Y becomes negative for
   above-primary screens) and the pill still lands bottom-center of
   the chosen display.
5. **Pass / Fail:** ___

---

## 13. Paste-last-transcript hotkey (⌥⇧V)

1. Dictate something into TextEdit. Confirm it pasted.
2. Click somewhere else. Press **⌥⇧V**.
3. **Expected:** the *most recent* transcript pastes into the new
   focus — pulled from the in-memory ring buffer.
4. Restart Vox. Press ⌥⇧V again.
5. **Expected:** error pill "No recent transcript." (ring buffer is
   memory-only, never persisted).
6. While a real dictation is mid-pipeline (Transcribing or
   Polishing), press ⌥⇧V. **Expected:** "Vox is busy with another
   dictation." pill — the paste-last is refused mid-pipeline.
7. **Pass / Fail:** ___

---

## 14. Security smoke (Round-2)

These verify the staff-review fixes (C1, C2, C4).

### 14.1 Mode-id path traversal (C1)
1. Create a file `evil.voxmode` with this exact JSON:
   ```json
   {
     "id": "../../../tmp/vox-pwned",
     "name": "Pwned",
     "icon": "💣",
     "description": "",
     "system_prompt": "x",
     "punctuation": "Standard",
     "case": "Sentence",
     "created_at": 0,
     "updated_at": 0
   }
   ```
2. Drag it onto Settings → Modes.
3. **Expected:** import succeeds, but the id is rewritten to a safe
   slug (e.g. `pwned`). The mode lives in
   `~/Library/Application Support/com.vox.app/modes/pwned.json` —
   *not* `/tmp/vox-pwned.json`.
4. Verify: `ls /tmp/vox-pwned.json 2>&1` should report "No such file."
5. Try a `.voxmode` with `"id": "/etc/passwd-test"`. **Expected:**
   same — re-slugged to safe shape; no write outside the modes dir.
6. **Pass / Fail:** ___

### 14.2 Updater stub does NOT call the network (C2)
1. Verify the running config has `pubkey: ""` and an endpoint
   containing `updates.vox.local`. (See `tauri.conf.json` plugins
   section.)
2. Open Settings → About → click "Check for updates".
3. **Expected:** within ~50 ms (NOT 5–15 s) the UI shows
   "Auto-update is not yet configured for this build." status:
   `unconfigured`. No DNS lookup, no TCP handshake.
4. Verification options (any one is enough):
   - Run Little Snitch (or LuLu) and confirm no outgoing connection
     to `updates.vox.local`.
   - In a terminal: `sudo tcpdump -i any host updates.vox.local`
     before clicking, and confirm no packets after the click.
   - Read `commands.rs::check_for_updates` and confirm
     `updater_is_stub()` short-circuits *before* `app.updater()` is
     even invoked. (Code already verified by QA.)
5. **Pass / Fail:** ___

### 14.3 Mismatched-sha256 deletes the partial (download integrity)
1. Wait for sha256s to be pinned in the catalog (T-01 in
   `local-first-impl-notes.md`). Until then, this test is N/A —
   `descriptor.sha256` is `None` and any byte stream is accepted.
2. Once pinned: deliberately corrupt `model.bin.partial` mid-resume
   (e.g. `dd if=/dev/urandom of=…/<id>/model.bin.partial conv=notrunc
   bs=1 count=1024 seek=0`).
3. Click Download to resume. **Expected:** sha256 mismatch detected;
   the partial file is removed from disk (`std::fs::remove_file` at
   `models.rs:480`); user can retry from scratch.
4. **Pass / Fail:** ___

### 14.4 Concurrent-download dedup (C4)
1. Settings → Models → click "Download" on a not-yet-downloaded
   model.
2. *Immediately* (while the progress bar is moving) click "Download"
   on the same model again.
3. **Expected:** the second invocation surfaces an "already
   downloading" error event (model:error). The first download
   continues uninterrupted; the `.partial` file is not corrupted.
4. **Pass / Fail:** ___

---

## Summary checklist

Smoke (10 minutes):
- [ ] §0 install
- [ ] §1.2 mic permission
- [ ] §1.4 API keys validate
- [ ] §2 tray popover
- [ ] §3 golden path dictation in TextEdit
- [ ] §6.1 misfire
- [ ] §6.3 cancel mid-recording
- [ ] §10.1 built-in modes seed
- [ ] §10.2 mode chip on pill
- [ ] §11.1 first-run local model download
- [ ] §11.3 migration dialog (upgrade only)

Full (60 minutes): everything above.

If everything in the smoke pass works and you're a small audience of
willing dogfooders, you can ship. If anything in the smoke pass fails,
**HOLD** until fixed.
