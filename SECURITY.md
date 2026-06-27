# Security Policy

Vox touches a few sensitive things — your microphone, a local inference server
on loopback, synthetic keystrokes for optional auto-paste, and an auto-updater —
so if you find a security problem, we want to hear about it. Responsible
disclosure is genuinely appreciated.

## Reporting a vulnerability

**Please do not open a public issue for security vulnerabilities.**

Instead, report privately through either:

- **GitHub private vulnerability reporting** —
  [open a draft advisory](https://github.com/KlymSerhii/Vox/security/advisories/new), or
- **Email** — **vox@rizenhq.com** (use the subject line `Vox security`).

Please include:

- the Vox version and your OS (macOS / Windows) and version,
- a description of the issue and its impact,
- steps to reproduce or a proof of concept, and
- any relevant logs (`VOX_LOG=debug`) with personal data removed.

We'll acknowledge your report as soon as we can, keep you updated on progress,
and credit you in the release notes once a fix ships, unless you'd rather stay
anonymous.

## Supported versions

Vox is a continuously-updated desktop app with an auto-updater. Security fixes
land in the **latest release**; please update before reporting to confirm the
issue still reproduces.

## Threat model &amp; design notes

A few facts that bound Vox's attack surface:

- **On-device only.** All inference is local. The only network egress is
  downloading models + the `llama-server` binary and pulling update manifests.
- **No audio at rest.** Captured audio lives in RAM and is freed after
  transcription; transcripts are process-lifetime only.
- **Signed updates.** Update artifacts are verified against a minisign public
  key pinned in `tauri.conf.json` before they're applied.
- **Loopback inference.** The Gemma `llama-server` listens on loopback and is
  gated by a locally-generated API token; that token is **not** a security
  secret against a local attacker — it exists so Vox can identify and reap its
  own orphaned child processes.
- **Optional, opt-in keystroke injection.** Auto-paste is off by default. On
  macOS it requires Accessibility; on Windows it uses `SendInput`.

If you find a way these guarantees can be broken, that's exactly the kind of
report we want.
