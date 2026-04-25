//! Orchestrates the dictation pipeline: start mic → stop mic → STT → paste.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Runtime};

use crate::audio::AudioHandle;
use crate::{inject, settings, stt};

#[derive(Clone)]
pub struct SessionCtx {
    pub audio: AudioHandle,
    pub recording: Arc<AtomicBool>,
}

impl SessionCtx {
    pub fn new(audio: AudioHandle) -> Self {
        Self {
            audio,
            recording: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub fn begin_recording(ctx: &SessionCtx) {
    if ctx.recording.swap(true, Ordering::SeqCst) {
        log::debug!("begin_recording called while already recording; ignored");
        return;
    }
    log::info!("▶ begin recording");
    if let Err(e) = ctx.audio.start() {
        log::error!("failed to start audio: {e}");
        ctx.recording.store(false, Ordering::SeqCst);
    }
}

pub fn end_recording<R: Runtime>(app: AppHandle<R>, ctx: SessionCtx) {
    if !ctx.recording.swap(false, Ordering::SeqCst) {
        log::debug!("end_recording called without an active recording; ignored");
        return;
    }
    log::info!("■ end recording");

    // Move stop + upload + paste off the hotkey callback thread.
    tauri::async_runtime::spawn(async move {
        match ctx.audio.stop_and_encode() {
            Ok(wav) => {
                if wav.len() < 44 + 16_000 * 2 / 4 {
                    // ~< 250 ms of audio after header; probably a misfire.
                    log::info!("recording too short ({} bytes), skipping STT", wav.len());
                    return;
                }
                let s = settings::load(&app);
                match stt::transcribe_wav(wav, &s.groq_api_key).await {
                    Ok(text) if !text.trim().is_empty() => {
                        log::info!("transcript: {:?}", text);
                        // Paste on a blocking thread — enigo does sync work.
                        let t = text.clone();
                        if let Err(e) = tauri::async_runtime::spawn_blocking(move || {
                            inject::paste_text(&t)
                        })
                        .await
                        {
                            log::error!("paste task panicked: {e}");
                        }
                    }
                    Ok(_) => log::info!("empty transcript, nothing to paste"),
                    Err(e) => log::error!("STT failed: {e}"),
                }
            }
            Err(e) => log::error!("audio stop/encode failed: {e}"),
        }
    });
}
