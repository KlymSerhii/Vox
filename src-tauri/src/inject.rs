//! Clipboard-only transcript delivery. The user pastes manually — no
//! keystroke synthesis, no Accessibility permission required.

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
