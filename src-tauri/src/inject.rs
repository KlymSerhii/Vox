//! Clipboard-only transcript delivery.
//!
//! 2026-04-26: switched from synthetic Cmd+V to clipboard-only — user
//! pastes manually like any Cmd+C copy. The previous implementation
//! copied the transcript to the clipboard, then synthesized Cmd+V via
//! `enigo`. That path required macOS Accessibility permission, was
//! prone to TCC churn (especially in dev builds), and `enigo` 0.2 had
//! been observed to panic in native code when the host returned
//! surprising states (we wrapped it in `catch_unwind` defensively).
//!
//! The new behavior is: write the text to the clipboard, show the
//! "Copied — press ⌘V to paste." pill, and stop. No keystroke
//! synthesis, no Accessibility dependency, no clipboard restore — the
//! transcript staying on the clipboard is the whole point.

use arboard::Clipboard;

#[derive(Debug, thiserror::Error)]
pub enum InjectError {
    #[error("clipboard: {0}")]
    Clipboard(String),
}

/// Write `text` to the system clipboard. Empty input is a no-op so the
/// caller can pipe through transcripts without filtering. Anything
/// already on the clipboard gets overwritten — that's the user
/// expectation, since they specifically dictated something to put
/// there.
pub fn copy_to_clipboard(text: &str) -> Result<(), InjectError> {
    if text.is_empty() {
        return Ok(());
    }

    let mut clip = Clipboard::new().map_err(|e| InjectError::Clipboard(e.to_string()))?;
    clip.set_text(text.to_string())
        .map_err(|e| InjectError::Clipboard(e.to_string()))?;
    Ok(())
}
