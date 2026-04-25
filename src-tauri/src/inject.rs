use std::thread;
use std::time::Duration;

use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

#[derive(Debug, thiserror::Error)]
pub enum InjectError {
    #[error("clipboard: {0}")]
    Clipboard(String),
    #[error("enigo: {0}")]
    Enigo(String),
}

/// Save the current clipboard, write `text`, simulate Cmd+V, then restore
/// the original clipboard — but only if our text is still on the clipboard
/// at restore time. If the user (or a clipboard manager) has written
/// something else in the interim we leave it alone, per the tech-stack
/// risk mitigation.
pub fn paste_text(text: &str) -> Result<(), InjectError> {
    if text.is_empty() {
        return Ok(());
    }

    let mut clip = Clipboard::new().map_err(|e| InjectError::Clipboard(e.to_string()))?;
    let previous = clip.get_text().ok();

    clip.set_text(text.to_string())
        .map_err(|e| InjectError::Clipboard(e.to_string()))?;

    // Short pause for some apps to notice the new clipboard content.
    thread::sleep(Duration::from_millis(30));

    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| InjectError::Enigo(e.to_string()))?;

    // Synthesize Cmd+V. Press the modifier first, click 'v', release the
    // modifier — order matters so the target app sees a real ⌘V.
    enigo
        .key(Key::Meta, Direction::Press)
        .map_err(|e| InjectError::Enigo(e.to_string()))?;
    let v_result = enigo.key(Key::Unicode('v'), Direction::Click);
    // Always release the modifier even if the v keystroke errored, so we
    // don't leave Cmd "stuck" from the user's perspective.
    let release_result = enigo.key(Key::Meta, Direction::Release);
    v_result.map_err(|e| InjectError::Enigo(e.to_string()))?;
    release_result.map_err(|e| InjectError::Enigo(e.to_string()))?;

    // Give the target app time to consume the paste before we restore.
    // 150 ms matches the tech-stack restore window.
    thread::sleep(Duration::from_millis(150));

    if let Some(prev) = previous {
        // Verify the clipboard still holds *our* text before overwriting.
        // If a clipboard manager or the user has written something else in
        // the meantime, don't clobber it.
        let still_ours = clip
            .get_text()
            .map(|cur| cur == text)
            .unwrap_or(false);
        if still_ours {
            if let Err(e) = clip.set_text(prev) {
                log::warn!("clipboard restore failed: {e}");
            }
        } else {
            log::debug!("clipboard changed during paste; leaving user content alone");
        }
    }

    Ok(())
}
