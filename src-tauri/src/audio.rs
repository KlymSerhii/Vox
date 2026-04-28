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

const TARGET_SR: u32 = 16_000;

/// Hard RAM cap, a few seconds above the session-layer recording cap. If
/// that guard fails to fire, this drops the *oldest* samples so the latest
/// audio is still recoverable.
const MAX_BUFFER_SECS: usize = 30 * 60 + 5;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no default input device")]
    NoInputDevice,
    #[error("cpal: {0}")]
    Cpal(String),
    #[error("wav: {0}")]
    Wav(#[from] hound::Error),
    #[error("audio thread died")]
    ThreadDied,
}

enum Command {
    Start(Sender<Result<(), AudioError>>),
    StopAndEncode(Sender<Result<Vec<u8>, AudioError>>),
}

#[derive(Clone)]
pub struct AudioHandle {
    tx: Sender<Command>,
    /// Live RMS level in 0..=1 as f32 bits, so reads are lock-free.
    rms_bits: Arc<AtomicU32>,
}

impl AudioHandle {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel::<Command>();
        let rms_bits = Arc::new(AtomicU32::new(0));
        let rms_for_thread = Arc::clone(&rms_bits);
        thread::Builder::new()
            .name("vox-audio".into())
            .spawn(move || audio_thread(rx, rms_for_thread))
            .expect("spawn audio thread");
        Self { tx, rms_bits }
    }

    pub fn start(&self) -> Result<(), AudioError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Command::Start(reply_tx))
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
    let mut active: Option<ActiveCapture> = None;

    for cmd in rx {
        match cmd {
            Command::Start(reply) => {
                if active.is_some() {
                    let _ = reply.send(Ok(()));
                    continue;
                }
                match begin_capture(Arc::clone(&rms_bits)) {
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
        }
    }
}

fn begin_capture(rms_bits: Arc<AtomicU32>) -> Result<ActiveCapture, AudioError> {
    // cpal's default_input_config + build_input_stream WILL implicitly prompt
    // for mic TCC if status is NotDetermined — callers must have already
    // obtained authorization (see mac_perm).
    log::info!("audio::begin_capture");
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or(AudioError::NoInputDevice)?;

    let default_cfg = device
        .default_input_config()
        .map_err(|e| AudioError::Cpal(e.to_string()))?;
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
    .map_err(|e| AudioError::Cpal(e.to_string()))?;

    stream.play().map_err(|e| AudioError::Cpal(e.to_string()))?;

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
    let mono = if input_channels <= 1 {
        raw.to_vec()
    } else {
        downmix_to_mono(raw, input_channels as usize)
    };

    let resampled = if input_sr == TARGET_SR {
        mono
    } else {
        linear_resample(&mono, input_sr, TARGET_SR)
    };

    encode_wav_pcm16(&resampled, TARGET_SR)
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
