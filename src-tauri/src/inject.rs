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

/// Save the current clipboard, write `text`, simulate Cmd+V, restore the
/// original clipboard after ~100 ms.
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

    // Synthesize Cmd+V.
    enigo
        .key(Key::Meta, Direction::Press)
        .map_err(|e| InjectError::Enigo(e.to_string()))?;
    enigo
        .key(Key::Unicode('v'), Direction::Click)
        .map_err(|e| InjectError::Enigo(e.to_string()))?;
    enigo
        .key(Key::Meta, Direction::Release)
        .map_err(|e| InjectError::Enigo(e.to_string()))?;

    // Give the target app time to consume the paste before we restore.
    thread::sleep(Duration::from_millis(120));

    if let Some(prev) = previous {
        if let Err(e) = clip.set_text(prev) {
            log::warn!("clipboard restore failed: {e}");
        }
    }

    Ok(())
}
