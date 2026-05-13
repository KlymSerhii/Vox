//! Microphone capture. `cpal::Stream` is `!Send` on macOS, so capture runs
//! on a dedicated thread reached via a command channel.

use std::io::Cursor;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use hound::{SampleFormat as WavSampleFormat, WavSpec, WavWriter};
use parking_lot::Mutex;
use serde::Serialize;

const TARGET_SR: u32 = 16_000;

/// Hard RAM cap, a few seconds above the session-layer recording cap. If
/// that guard fails to fire, this drops the *oldest* samples so the latest
/// audio is still recoverable.
const MAX_BUFFER_SECS: usize = 30 * 60 + 5;

/// Hold time for `probe_device`. CoreAudio needs ~250–300 ms after
/// `stream.play()` to commit to opening a Continuity Microphone over
/// Bluetooth/Lightning and surface the iPhone wake banner; dropping
/// the stream sooner short-circuits the wake.
const CONTINUITY_WAKE_HOLD_MS: u64 = 300;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no default input device")]
    NoInputDevice,
    /// The user explicitly chose a microphone in Settings, and cpal
    /// can't open it right now — typically because the device went
    /// away (iPhone Continuity Mic disabled, USB mic unplugged,
    /// Bluetooth headset out of range). We deliberately do NOT fall
    /// back to the system default here: silently capturing from the
    /// wrong mic (and trusting Whisper to transcribe it) is the
    /// failure mode this variant exists to surface. Visible-device
    /// diagnostics go in the log, not the payload.
    #[error("selected audio input '{name}' isn't connected")]
    ConfiguredDeviceUnavailable { name: String },
    #[error("cpal: {0}")]
    Cpal(String),
    #[error("wav: {0}")]
    Wav(#[from] hound::Error),
    #[error("audio thread died")]
    ThreadDied,
    #[error("failed to spawn audio thread: {0}")]
    SpawnFailed(String),
}

/// One enumerated macOS audio input. `is_default` flags whichever
/// device `cpal::Host::default_input_device()` currently returns, so
/// the picker can render "MacBook Pro Microphone (System default)".
#[derive(Debug, Clone, Serialize)]
pub struct AudioInputDevice {
    pub name: String,
    pub is_default: bool,
}

/// Enumerate every input device cpal can see, in the order CoreAudio
/// returns them. Errors on a single device's name probe are skipped
/// (logged) rather than aborting the whole list — one misbehaving
/// device shouldn't hide the rest. Pure enumeration; does NOT call
/// `default_input_config` so it won't trip the macOS TCC prompt and
/// is safe to call from the Settings window before microphone access
/// has been granted.
pub fn list_input_devices() -> Result<Vec<AudioInputDevice>, AudioError> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok());
    let iter = host
        .input_devices()
        .map_err(|e| AudioError::Cpal(e.to_string()))?;
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dev in iter {
        match dev.name() {
            Ok(name) => {
                if !seen.insert(name.clone()) {
                    // Same OS-level device can surface under both the
                    // default CoreAudio host and an aggregate host on
                    // some macOS versions, OR the user has two
                    // physical devices with the same name (two USB
                    // mics of the same model, two iPhones with the
                    // same first name). Name-based selection can't
                    // disambiguate, so the second one becomes
                    // unselectable until we move to UID-based ids.
                    log::warn!(
                        "audio: duplicate input device name '{name}' — \
                         second instance hidden from the picker"
                    );
                    continue;
                }
                let is_default = default_name.as_deref() == Some(name.as_str());
                out.push(AudioInputDevice { name, is_default });
            }
            Err(e) => {
                log::warn!("audio device name probe failed; skipping: {e}");
            }
        }
    }
    Ok(out)
}

/// Map a cpal `BuildStreamError::DeviceNotAvailable` (and the
/// matching `DefaultStreamConfigError` / `PlayStreamError` variants)
/// onto `AudioError::ConfiguredDeviceUnavailable` when the user has
/// an explicit selection, so the pill says "iPhone Mic not connected"
/// instead of leaking a cpal-shaped string. The `None` case (no
/// explicit selection) collapses to `NoInputDevice` — there's no
/// single "configured" name to blame.
fn map_device_unavailable_error<T: std::fmt::Display>(
    device_name: Option<&str>,
    err: T,
    is_device_unavailable: bool,
) -> AudioError {
    if !is_device_unavailable {
        return AudioError::Cpal(err.to_string());
    }
    match device_name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => AudioError::ConfiguredDeviceUnavailable {
            name: name.to_string(),
        },
        None => AudioError::NoInputDevice,
    }
}

/// Briefly open the named (or default) input device and immediately
/// drop the stream. Called from `set_audio_input_device` after the
/// user picks a new mic so they get instant feedback — most
/// importantly, the macOS Continuity Microphone activation banner
/// shows up on the user's iPhone as soon as CoreAudio commits to
/// opening the device. Without this probe, the iPhone only wakes on
/// the user's next dictation, which feels broken.
///
/// `cpal::Stream` is `!Send`, so this must be called on the same
/// thread that drops it. Always invoke from a fresh
/// `std::thread::spawn` — never the audio thread, which already owns
/// a capture stream that we don't want to displace.
///
/// Errors are non-fatal: at recording time `begin_capture` does the
/// same lookup-with-fallback dance, so a failed probe doesn't break
/// future dictations.
pub fn probe_device(device_name: Option<&str>) -> Result<(), AudioError> {
    log::info!(
        "audio::probe_device (requested: {})",
        device_name.unwrap_or("<system default>")
    );
    let host = cpal::default_host();
    let device = resolve_input_device(&host, device_name)?;
    let resolved_name = device.name().ok();
    log::info!(
        "audio::probe_device opening '{}'",
        resolved_name.as_deref().unwrap_or("<unknown>")
    );
    let default_cfg = device.default_input_config().map_err(|e| {
        let unavailable = matches!(
            e,
            cpal::DefaultStreamConfigError::DeviceNotAvailable
        );
        map_device_unavailable_error(device_name, e, unavailable)
    })?;
    let sample_format = default_cfg.sample_format();
    let config: StreamConfig = default_cfg.into();
    let err_fn = |err| log::warn!("audio probe stream error: {err}");
    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |_data: &[f32], _| {},
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |_data: &[i16], _| {},
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |_data: &[u16], _| {},
            err_fn,
            None,
        ),
        other => return Err(AudioError::Cpal(format!("unsupported sample format: {other:?}"))),
    }
    .map_err(|e| {
        let unavailable = matches!(e, cpal::BuildStreamError::DeviceNotAvailable);
        map_device_unavailable_error(device_name, e, unavailable)
    })?;
    stream.play().map_err(|e| {
        let unavailable = matches!(e, cpal::PlayStreamError::DeviceNotAvailable);
        map_device_unavailable_error(device_name, e, unavailable)
    })?;
    std::thread::sleep(std::time::Duration::from_millis(CONTINUITY_WAKE_HOLD_MS));
    drop(stream);
    Ok(())
}

fn resolve_input_device(
    host: &cpal::Host,
    device_name: Option<&str>,
) -> Result<cpal::Device, AudioError> {
    let requested = device_name.map(str::trim).filter(|s| !s.is_empty());
    // No explicit selection — follow the macOS system default. This
    // is the ONLY auto-fallback path; once the user picks a specific
    // device we never silently swap them onto something else.
    let Some(name) = requested else {
        return host.default_input_device().ok_or(AudioError::NoInputDevice);
    };
    let mut iter = host.input_devices().map_err(|e| {
        log::error!("audio: input device enumeration failed: {e}");
        AudioError::Cpal(e.to_string())
    })?;
    let mut considered: Vec<String> = Vec::new();
    while let Some(dev) = iter.next() {
        let dev_name = dev.name().unwrap_or_default();
        if dev_name == name {
            log::info!("audio: resolved selected device '{name}'");
            return Ok(dev);
        }
        considered.push(dev_name);
    }
    // Explicit selection that's no longer connected. Surface the
    // structured error so `session.rs` can show "Selected mic isn't
    // connected" in the pill — rather than silently recording from
    // whatever cpal would have handed back as the default.
    log::warn!(
        "audio: selected device '{name}' is not currently connected. \
         Visible inputs: [{}]",
        considered.join(", ")
    );
    Err(AudioError::ConfiguredDeviceUnavailable {
        name: name.to_string(),
    })
}

enum Command {
    /// `device_name` carries the user's `Settings::audio_input_device`
    /// preference. `None`/empty = follow the macOS system default. A
    /// non-empty name that doesn't match anything in
    /// `cpal::Host::input_devices()` is surfaced as
    /// `AudioError::ConfiguredDeviceUnavailable` — `begin_capture`
    /// deliberately does NOT silently fall back to the system default.
    Start(Option<String>, Sender<Result<(), AudioError>>),
    StopAndEncode(Sender<Result<Vec<u8>, AudioError>>),
    /// Returns a `(wav_bytes, pcm_secs)` snapshot of the current capture
    /// buffer WITHOUT stopping the stream. The WAV is the same shape
    /// `stop_and_encode` produces (16 kHz mono PCM-16) so it can flow
    /// straight into [`crate::engines::SttEngine::transcribe`]. Used by
    /// the speculative streaming decoder
    /// (`session.rs::spawn_speculative_decoder`) so we can run partial
    /// Whisper passes while the user is still talking. Returns
    /// `Ok(None)` when no capture is currently active.
    SnapshotWav(Sender<Result<Option<SnapshotWav>, AudioError>>),
}

/// Snapshot output for [`AudioHandle::snapshot_wav`].
#[derive(Debug, Clone)]
pub struct SnapshotWav {
    /// 16 kHz mono PCM-16 WAV (header + body), ready for Whisper.
    pub wav: Vec<u8>,
    /// Captured-and-resampled audio length in seconds. The speculative
    /// decoder uses this to apply the "minimum N seconds before first
    /// decode / minimum delta before next" thresholds without having to
    /// re-parse the WAV header.
    pub duration_secs: f32,
}

#[derive(Clone)]
pub struct AudioHandle {
    tx: Sender<Command>,
    /// Live RMS level in 0..=1 as f32 bits, so reads are lock-free.
    rms_bits: Arc<AtomicU32>,
}

impl AudioHandle {
    pub fn spawn() -> Result<Self, AudioError> {
        let (tx, rx) = mpsc::channel::<Command>();
        let rms_bits = Arc::new(AtomicU32::new(0));
        let rms_for_thread = Arc::clone(&rms_bits);
        thread::Builder::new()
            .name("vox-audio".into())
            .spawn(move || audio_thread(rx, rms_for_thread))
            .map_err(|e| AudioError::SpawnFailed(e.to_string()))?;
        Ok(Self { tx, rms_bits })
    }

    pub fn start(&self, device_name: Option<String>) -> Result<(), AudioError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Command::Start(device_name, reply_tx))
            .map_err(|_| AudioError::ThreadDied)?;
        reply_rx.recv().map_err(|_| AudioError::ThreadDied)?
    }

    pub fn stop_and_encode(&self) -> Result<Vec<u8>, AudioError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Command::StopAndEncode(reply_tx))
            .map_err(|_| AudioError::ThreadDied)?;
        reply_rx.recv().map_err(|_| AudioError::ThreadDied)?
    }

    /// Take a non-destructive snapshot of the in-progress capture
    /// buffer, downmixed to mono and resampled to 16 kHz, encoded as a
    /// 16 kHz mono PCM-16 WAV (the same shape `stop_and_encode`
    /// produces). The capture stream keeps running so subsequent
    /// snapshots will see additional samples. Returns `Ok(None)` if no
    /// capture is active. Used by the speculative streaming decoder to
    /// pre-emptively run Whisper while the user is still speaking.
    pub fn snapshot_wav(&self) -> Result<Option<SnapshotWav>, AudioError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Command::SnapshotWav(reply_tx))
            .map_err(|_| AudioError::ThreadDied)?;
        reply_rx.recv().map_err(|_| AudioError::ThreadDied)?
    }

    pub fn current_rms(&self) -> f32 {
        f32::from_bits(self.rms_bits.load(Ordering::Relaxed))
    }
}

struct ActiveCapture {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    input_sr: u32,
    input_channels: u16,
}

fn audio_thread(rx: Receiver<Command>, rms_bits: Arc<AtomicU32>) {
    // Wrap the entire dispatch loop in catch_unwind so a panic in cpal /
    // hound / postprocessing kills the thread cleanly rather than silently
    // tearing down without a log line. We deliberately do NOT auto-respawn
    // from inside the thread — that risks an infinite restart loop on a
    // wedged audio stack. The next session attempt will see `tx` closed
    // and surface a clean `AudioError::ThreadDied` instead of stalling.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        audio_thread_inner(rx, rms_bits)
    }));
    if let Err(payload) = result {
        let msg = if let Some(s) = payload.downcast_ref::<&'static str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        log::error!(
            "audio thread panicked: {msg}. Audio capture is now offline; \
             the next session attempt will return AudioError::ThreadDied."
        );
    }
}

fn audio_thread_inner(rx: Receiver<Command>, rms_bits: Arc<AtomicU32>) {
    let mut active: Option<ActiveCapture> = None;

    for cmd in rx {
        match cmd {
            Command::Start(device_name, reply) => {
                if active.is_some() {
                    let _ = reply.send(Ok(()));
                    continue;
                }
                match begin_capture(Arc::clone(&rms_bits), device_name.as_deref()) {
                    Ok(cap) => {
                        active = Some(cap);
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        rms_bits.store(0, Ordering::Relaxed);
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Command::StopAndEncode(reply) => {
                let Some(cap) = active.take() else {
                    rms_bits.store(0, Ordering::Relaxed);
                    let _ = reply.send(Ok(Vec::new()));
                    continue;
                };
                drop(cap.stream);
                rms_bits.store(0, Ordering::Relaxed);
                let raw = cap.samples.lock().clone();
                let _ = reply.send(postprocess_to_wav(&raw, cap.input_sr, cap.input_channels));
            }
            Command::SnapshotWav(reply) => {
                let Some(cap) = active.as_ref() else {
                    let _ = reply.send(Ok(None));
                    continue;
                };
                let raw = cap.samples.lock().clone();
                let pcm =
                    postprocess_to_pcm_mono_16k(&raw, cap.input_sr, cap.input_channels);
                let duration_secs = pcm.len() as f32 / TARGET_SR as f32;
                match encode_wav_pcm16(&pcm, TARGET_SR) {
                    Ok(wav) => {
                        let _ = reply.send(Ok(Some(SnapshotWav { wav, duration_secs })));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
        }
    }
}

fn begin_capture(
    rms_bits: Arc<AtomicU32>,
    device_name: Option<&str>,
) -> Result<ActiveCapture, AudioError> {
    // cpal's default_input_config + build_input_stream WILL implicitly prompt
    // for mic TCC if status is NotDetermined — callers must have already
    // obtained authorization (see mac_perm).
    log::info!(
        "audio::begin_capture (requested device: {})",
        device_name.unwrap_or("<system default>")
    );
    let host = cpal::default_host();
    let device = resolve_input_device(&host, device_name)?;
    let resolved_name = device.name().ok();
    log::info!(
        "audio::begin_capture opening '{}'",
        resolved_name.as_deref().unwrap_or("<unknown>")
    );

    let default_cfg = device.default_input_config().map_err(|e| {
        let unavailable = matches!(
            e,
            cpal::DefaultStreamConfigError::DeviceNotAvailable
        );
        map_device_unavailable_error(device_name, e, unavailable)
    })?;
    let input_sr = default_cfg.sample_rate().0;
    let input_channels = default_cfg.channels();
    let sample_format = default_cfg.sample_format();
    let config: StreamConfig = default_cfg.into();

    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::with_capacity(
        (input_sr as usize) * (input_channels as usize) * 16,
    )));

    let max_samples =
        (input_sr as usize) * (input_channels as usize) * MAX_BUFFER_SECS;

    let err_fn = |err| log::error!("cpal stream error: {err}");
    let make_push = || {
        let samples_cb = Arc::clone(&samples);
        let rms_cb = Arc::clone(&rms_bits);
        move |converted: Vec<f32>| {
            update_rms(&rms_cb, converted.iter().copied());
            let mut buf = samples_cb.lock();
            buf.extend_from_slice(&converted);
            trim_front(&mut buf, max_samples);
        }
    };
    let stream = match sample_format {
        SampleFormat::F32 => {
            let push = make_push();
            device.build_input_stream(
                &config,
                move |data: &[f32], _| push(data.to_vec()),
                err_fn,
                None,
            )
        }
        SampleFormat::I16 => {
            let push = make_push();
            device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    push(data.iter().map(|&s| s as f32 / i16::MAX as f32).collect())
                },
                err_fn,
                None,
            )
        }
        SampleFormat::U16 => {
            let push = make_push();
            device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    push(
                        data.iter()
                            .map(|&s| (s as f32 - 32_768.0) / 32_768.0)
                            .collect(),
                    )
                },
                err_fn,
                None,
            )
        }
        other => return Err(AudioError::Cpal(format!("unsupported sample format: {other:?}"))),
    }
    .map_err(|e| {
        let unavailable = matches!(e, cpal::BuildStreamError::DeviceNotAvailable);
        map_device_unavailable_error(device_name, e, unavailable)
    })?;

    stream.play().map_err(|e| {
        let unavailable = matches!(e, cpal::PlayStreamError::DeviceNotAvailable);
        map_device_unavailable_error(device_name, e, unavailable)
    })?;

    log::info!(
        "audio capture started: {} Hz, {} ch, {:?}",
        input_sr,
        input_channels,
        sample_format
    );

    Ok(ActiveCapture {
        stream,
        samples,
        input_sr,
        input_channels,
    })
}

fn postprocess_to_wav(
    raw: &[f32],
    input_sr: u32,
    input_channels: u16,
) -> Result<Vec<u8>, AudioError> {
    let resampled = postprocess_to_pcm_mono_16k(raw, input_sr, input_channels);
    encode_wav_pcm16(&resampled, TARGET_SR)
}

/// Same downmix + linear-resample pipeline as [`postprocess_to_wav`] but
/// returns the raw 16 kHz mono PCM `Vec<f32>` straight to the caller —
/// used by the speculative streaming decoder so we don't pay for a WAV
/// roundtrip on every snapshot.
fn postprocess_to_pcm_mono_16k(raw: &[f32], input_sr: u32, input_channels: u16) -> Vec<f32> {
    let mono = if input_channels <= 1 {
        raw.to_vec()
    } else {
        downmix_to_mono(raw, input_channels as usize)
    };

    if input_sr == TARGET_SR {
        mono
    } else {
        linear_resample(&mono, input_sr, TARGET_SR)
    }
}

fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    let frames = interleaved.len() / channels;
    let mut out = Vec::with_capacity(frames);
    for frame in interleaved.chunks_exact(channels) {
        let sum: f32 = frame.iter().sum();
        out.push(sum / channels as f32);
    }
    out
}

fn linear_resample(input: &[f32], in_sr: u32, out_sr: u32) -> Vec<f32> {
    if input.is_empty() || in_sr == out_sr {
        return input.to_vec();
    }
    let ratio = out_sr as f64 / in_sr as f64;
    let out_len = (input.len() as f64 * ratio).round() as usize;
    let last = input.len().saturating_sub(1);
    (0..out_len)
        .map(|i| {
            let src_pos = i as f64 / ratio;
            let idx = (src_pos.floor() as usize).min(last);
            let frac = (src_pos - idx as f64) as f32;
            let a = input[idx];
            let b = input[(idx + 1).min(last)];
            a + (b - a) * frac
        })
        .collect()
}

fn trim_front(buf: &mut Vec<f32>, max: usize) {
    if max == 0 || buf.len() <= max {
        return;
    }
    buf.drain(0..buf.len() - max);
}

fn update_rms<I: IntoIterator<Item = f32>>(target: &Arc<AtomicU32>, samples: I) {
    let mut n: u32 = 0;
    let mut sum_sq: f32 = 0.0;
    for s in samples {
        sum_sq += s * s;
        n += 1;
    }
    if n == 0 {
        return;
    }
    let rms = (sum_sq / n as f32).sqrt();
    // Rough normalisation: -30 dBFS → 0.1, -6 dBFS → ~0.9.
    let scaled = (rms * 3.0).clamp(0.0, 1.0);
    let prev = f32::from_bits(target.load(Ordering::Relaxed));
    target.store(((prev + scaled) * 0.5).to_bits(), Ordering::Relaxed);
}

fn encode_wav_pcm16(samples: &[f32], sr: u32) -> Result<Vec<u8>, AudioError> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: sr,
        bits_per_sample: 16,
        sample_format: WavSampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::<u8>::new());
    {
        let mut writer = WavWriter::new(&mut cursor, spec)?;
        for &s in samples {
            writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
        }
        writer.finalize()?;
    }
    Ok(cursor.into_inner())
}
