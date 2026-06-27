//! Transcript delivery primitives.
//!
//! Two modes live here:
//!
//! * **Clipboard-only** (default, requires no Accessibility permission):
//!   the polished transcript replaces the clipboard contents and the
//!   user pastes manually with ⌘V.
//! * **Auto-paste** (opt-in via `Settings::auto_paste_enabled`, requires
//!   Accessibility): the polished transcript is briefly written to the
//!   clipboard, ⌘V is synthesised at the OS event tap (see
//!   `crate::mac_paste`), and the user's prior clipboard text is
//!   restored on a delay so the paste round-trip doesn't permanently
//!   clobber whatever they had copied.
//!
//! Non-text clipboards (images, files) aren't preserved across an
//! auto-paste — `arboard`'s round-trip for those is lossy and not worth
//! the complexity for the minority case. When the user's prior content
//! isn't text, the polished transcript stays on the clipboard after the
//! paste, matching the clipboard-only fallback behaviour.
//!
//! Orchestration (snapshot → write → ⌘V → restore) lives in
//! `session::SessionManager::deliver_transcript` so the clipboard lock
//! and the pending-restore tracking stay in one place.

use arboard::Clipboard;

#[derive(Debug, thiserror::Error)]
pub enum InjectError {
    #[error("clipboard: {0}")]
    Clipboard(String),
}

pub fn copy_to_clipboard(text: &str) -> Result<(), InjectError> {
    if text.is_empty() {
        return Ok(());
    }
    let mut clip = Clipboard::new().map_err(|e| InjectError::Clipboard(e.to_string()))?;
    clip.set_text(text.to_string())
        .map_err(|e| InjectError::Clipboard(e.to_string()))?;
    Ok(())
}

/// Read the clipboard contents as text. Returns `None` when the
/// clipboard holds a non-text payload (image, file refs, …) or is
/// empty — the auto-paste flow then skips the restore step rather than
/// rendering binary data as garbled text.
///
/// Errors from `arboard` collapse to `None` because they're typically
/// "no string on the pasteboard" rather than a fatal condition; the
/// caller treats `None` the same way (no restore needed).
pub fn snapshot_clipboard_text() -> Option<String> {
    let mut clip = Clipboard::new().ok()?;
    clip.get_text().ok()
}

/// Fire-and-forget clipboard prewarm.
///
/// Forces macOS's NSPasteboard / Gatekeeper to do its first-use source-app
/// signature check on Vox at startup (while the user is in onboarding)
/// rather than on the user's very first dictation, where it otherwise
/// stalls the post-polish clipboard write by 30–60s and pops a system
/// "Verifying com.vox…" dialog on top of the in-flight pipeline.
///
/// Strategy: read whatever text is currently on the clipboard and write
/// it straight back. That round-trip is enough to trigger the OS-level
/// signature check, and on the common path leaves the user's clipboard
/// content untouched. Non-text clipboards (image, files, etc.) cause
/// `get_text` to fail; we skip the round-trip in that case — those users
/// will trip the verification on their first real dictation, but
/// preserving arbitrary pasteboard items isn't worth the complexity
/// here.
///
/// Gatekeeper caches the verification result across launches, so on
/// every subsequent app start this is a sub-millisecond no-op. Safe to
/// call unconditionally from the startup setup hook.
pub fn prewarm_in_background() {
    std::thread::spawn(|| {
        let started = std::time::Instant::now();
        match prewarm() {
            Ok(()) => log::info!(
                "clipboard: prewarm complete in {} ms",
                started.elapsed().as_millis()
            ),
            Err(e) => log::info!(
                "clipboard: prewarm skipped after {} ms: {e}",
                started.elapsed().as_millis()
            ),
        }
    });
}

fn prewarm() -> Result<(), InjectError> {
    let mut clip = Clipboard::new().map_err(|e| InjectError::Clipboard(e.to_string()))?;
    let existing = clip
        .get_text()
        .map_err(|e| InjectError::Clipboard(e.to_string()))?;
    clip.set_text(existing)
        .map_err(|e| InjectError::Clipboard(e.to_string()))?;
    Ok(())
}
