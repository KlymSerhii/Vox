//! Model catalog + on-disk cache + resumable, sha256-verified HTTP
//! downloader. Cache lives at `<app_data_dir>/models/<kind>/<id>/`;
//! a `.partial` file is renamed atomically on success.
//!
//! Catalog sha256s start as `None` and are pinned to a sibling
//! `models.lock` file on first successful download (soft canary, not a
//! hard gate — HF can rotate assets).
//!
//! Gemma 4 MatFormer GGUFs converted by Unsloth crash the bundled
//! `llama.cpp` from C++ (`GGML_OP_GATED_DELTA_NET` tensors miss the
//! `LLAMA_TENSOR_NAME_FGDN_AR` prefix that FGDN auto-detect expects),
//! and the gating cparams are not exposed via Rust. bartowski's
//! conversion uses a tensor naming scheme that avoids this — E2B and
//! E4B are sourced from bartowski for that reason. Gemma 4 26B-A4B
//! (MoE) emits zero GDN ops so it's safe under either converter; we
//! keep Unsloth's UD-Q4_K_M for its imatrix calibration edge.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, Runtime, Wry};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Stt,
    Llm,
}

impl ModelKind {
    fn dir(self) -> &'static str {
        match self {
            ModelKind::Stt => "stt",
            ModelKind::Llm => "llm",
        }
    }
    fn filename(self) -> &'static str {
        match self {
            ModelKind::Stt => "model.bin",
            ModelKind::Llm => "model.gguf",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelDescriptor {
    pub id: &'static str,
    pub kind: ModelKind,
    pub name: &'static str,
    pub size_bytes: u64,
    pub sha256: Option<&'static str>,
    pub url: &'static str,
    /// `"whisper-local"` or `"gemma-local"`.
    pub engine: &'static str,
    pub ram_peak_mb: u32,
    /// `"small"` | `"balanced"` | `"quality"`.
    pub tier: &'static str,
}

pub const CATALOG: &[ModelDescriptor] = &[
    ModelDescriptor {
        id: "whisper-tiny-en-q5_1",
        kind: ModelKind::Stt,
        name: "Whisper Tiny English (q5_1)",
        size_bytes: 78_000_000,
        sha256: None,
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en-q5_1.bin",
        engine: "whisper-local",
        ram_peak_mb: 200,
        tier: "small",
    },
    ModelDescriptor {
        id: "whisper-small-en-q5_1",
        kind: ModelKind::Stt,
        name: "Whisper Small English (q5_1)",
        size_bytes: 264_000_000,
        sha256: None,
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en-q5_1.bin",
        engine: "whisper-local",
        ram_peak_mb: 350,
        tier: "small",
    },
    ModelDescriptor {
        id: "whisper-large-v3-turbo-q5_0",
        kind: ModelKind::Stt,
        name: "Whisper Large v3 Turbo (q5_0) — Balanced",
        size_bytes: 574_000_000,
        sha256: None,
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        engine: "whisper-local",
        ram_peak_mb: 1100,
        tier: "balanced",
    },
    ModelDescriptor {
        id: "whisper-large-v3-q5_0",
        kind: ModelKind::Stt,
        name: "Whisper Large v3 (q5_0) — Quality",
        size_bytes: 1_100_000_000,
        sha256: None,
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-q5_0.bin",
        engine: "whisper-local",
        ram_peak_mb: 1900,
        tier: "quality",
    },
    // Parakeet TDT 0.6B v3:
    //   macOS aarch64 — Core ML bundle (4 .mlmodelc dirs + vocab JSON)
    //     https://huggingface.co/FluidInference/parakeet-tdt-0.6b-v3-coreml/
    //   Windows — ONNX bundle (encoder + external data + decoder_joint + vocab)
    //     https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/
    // The single-file `stream_to_file` downloader doesn't apply to either host;
    // the multi-file manifest lives in `parakeet_v3_files_{macos,windows}()` and
    // the catalog `url` is intentionally empty so a misuse via `download_model`
    // fails loudly rather than silently fetching an empty body.
    ModelDescriptor {
        id: "parakeet-tdt-0.6b-v3",
        kind: ModelKind::Stt,
        #[cfg(target_os = "macos")]
        name: "Parakeet TDT 0.6B v3 (multilingual, 25 languages, ANE-accelerated)",
        #[cfg(not(target_os = "macos"))]
        name: "Parakeet TDT 0.6B v3 (multilingual, 25 languages, DirectML-accelerated)",
        // macOS: approximate Core ML bundle size (~480 MB; FluidAudio doesn't
        // publish exact figures). Windows: exact sum of the four ONNX manifest
        // files fetched from HF API on 2026-06-01:
        //   encoder-model.onnx       41_770_866
        //   encoder-model.onnx.data  2_435_420_160
        //   decoder_joint-model.onnx 72_520_893
        //   vocab.txt                93_939
        //   Total                    2_549_805_858
        #[cfg(target_os = "macos")]
        size_bytes: 500 * 1024 * 1024,
        #[cfg(not(target_os = "macos"))]
        size_bytes: 2_549_805_858,
        sha256: None,
        url: "",
        engine: "parakeet-local",
        // Approximate upper bound covering model weights + inference buffers.
        // Revisit after first profiled run on each platform.
        ram_peak_mb: 800,
        tier: "balanced",
    },
    // Gemma 4 MatFormer entries are sourced from bartowski; see
    // file-level docs for why Unsloth's conversion crashes.
    //
    // ram_peak_mb covers model weights + a 16k-context KV cache at fp16
    // (the llama-server default for Gemma engines). Numbers are
    // conservative round-ups.
    ModelDescriptor {
        id: "gemma-4-e2b-q5_k_m",
        kind: ModelKind::Llm,
        name: "Gemma 4 E2B (Q5_K_M) — Small",
        size_bytes: 3_658_382_592,
        sha256: None,
        url: "https://huggingface.co/bartowski/google_gemma-4-E2B-it-GGUF/resolve/main/google_gemma-4-E2B-it-Q5_K_M.gguf",
        engine: "gemma-local",
        // ~3.7 GB weights + smaller-than-E4B fp16 KV at 16k.
        ram_peak_mb: 7500,
        tier: "small",
    },
    ModelDescriptor {
        id: "gemma-4-e4b-q5_k_m",
        kind: ModelKind::Llm,
        name: "Gemma 4 E4B (Q5_K_M) — Balanced",
        size_bytes: 5_820_881_184,
        sha256: None,
        url: "https://huggingface.co/bartowski/google_gemma-4-E4B-it-GGUF/resolve/main/google_gemma-4-E4B-it-Q5_K_M.gguf",
        engine: "gemma-local",
        // ~5.8 GB weights + ~5 GB fp16 KV at 16k (see inference_proc.rs).
        ram_peak_mb: 11000,
        tier: "balanced",
    },
    ModelDescriptor {
        id: "gemma-4-26b-a4b-q4_k_m",
        kind: ModelKind::Llm,
        name: "Gemma 4 26B-A4B MoE (Q4_K_M) — Quality",
        size_bytes: 16_868_240_704,
        sha256: None,
        url: "https://huggingface.co/unsloth/gemma-4-26B-A4B-it-GGUF/resolve/main/gemma-4-26B-A4B-it-UD-Q4_K_M.gguf",
        engine: "gemma-local",
        // ~16.9 GB weights + ~5–6 GB fp16 KV at 16k.
        ram_peak_mb: 23000,
        tier: "quality",
    },
];

pub fn descriptor_for(id: &str) -> Option<&'static ModelDescriptor> {
    CATALOG.iter().find(|d| d.id == id)
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub descriptor: ModelDescriptor,
    pub installed: bool,
    pub bytes_on_disk: u64,
    pub path: Option<String>,
}

pub fn list<R: Runtime>(app: &AppHandle<R>) -> Vec<ModelStatus> {
    let root = root_dir(app);
    CATALOG
        .iter()
        .map(|d| {
            // Parakeet ships as a multi-file Core ML bundle, so the
            // single-file `model.bin` probe used by Whisper / Gemma
            // never matches. Probe the small vocab JSON instead — the
            // engine code uses the same file as a load gate (see
            // `parakeet_local::FLUIDAUDIO_VOCAB_FILE`). A non-empty
            // vocab file means the multi-file downloader finished its
            // last write.
            if d.id == "parakeet-tdt-0.6b-v3" {
                let bundle_dir = model_dir(&root, d);

                #[cfg(target_os = "macos")]
                {
                    // macOS: probe the vocab JSON written last by the Core ML
                    // downloader. Path mirrors `parakeet_local::FLUIDAUDIO_VOCAB_FILE`.
                    let vocab_path = bundle_dir
                        .join("parakeet-tdt-0.6b-v3-coreml")
                        .join("parakeet_vocab.json");
                    let vocab_bytes = vocab_path.metadata().map(|m| m.len()).unwrap_or(0);
                    let installed = vocab_bytes > 0;
                    let bytes_on_disk = if installed {
                        parakeet_v3_files()
                            .iter()
                            .map(|f| {
                                bundle_dir
                                    .join(f.relpath)
                                    .metadata()
                                    .map(|m| m.len())
                                    .unwrap_or(0)
                            })
                            .sum()
                    } else {
                        vocab_bytes
                    };
                    return ModelStatus {
                        descriptor: d.clone(),
                        installed,
                        bytes_on_disk,
                        path: if installed {
                            bundle_dir.to_str().map(|s| s.to_string())
                        } else {
                            None
                        },
                    };
                }

                #[cfg(not(target_os = "macos"))]
                {
                    // Windows (and any future non-macOS host): probe vocab.txt
                    // (written last by the ONNX downloader), then additionally
                    // confirm the large external weights file is fully present
                    // so a cancelled mid-download never shows as "installed".
                    let vocab_path = bundle_dir.join("vocab.txt");
                    let vocab_bytes = vocab_path.metadata().map(|m| m.len()).unwrap_or(0);
                    // Require the external weights file (≥2.4 GB) to guard
                    // against a partially-completed download.
                    let data_ok = bundle_dir
                        .join("encoder-model.onnx.data")
                        .metadata()
                        .map(|m| m.len() >= 2_400_000_000)
                        .unwrap_or(false);
                    let installed = vocab_bytes > 0 && data_ok;
                    let bytes_on_disk = if installed {
                        parakeet_v3_files()
                            .iter()
                            .map(|f| {
                                bundle_dir
                                    .join(f.relpath)
                                    .metadata()
                                    .map(|m| m.len())
                                    .unwrap_or(0)
                            })
                            .sum()
                    } else {
                        // Partial download: sum whatever is on disk already.
                        parakeet_v3_files()
                            .iter()
                            .map(|f| {
                                bundle_dir
                                    .join(f.relpath)
                                    .metadata()
                                    .map(|m| m.len())
                                    .unwrap_or(0)
                            })
                            .sum::<u64>()
                            .max(vocab_bytes)
                    };
                    return ModelStatus {
                        descriptor: d.clone(),
                        installed,
                        bytes_on_disk,
                        path: if installed {
                            bundle_dir.to_str().map(|s| s.to_string())
                        } else {
                            None
                        },
                    };
                }
            }

            let path = model_dir(&root, d).join(d.kind.filename());
            let bytes = path.metadata().map(|m| m.len()).unwrap_or(0);
            let installed = bytes >= d.size_bytes / 2 && bytes > 0;
            let installed = installed && verify_present(&path, d);
            ModelStatus {
                descriptor: d.clone(),
                installed,
                bytes_on_disk: bytes,
                path: if installed {
                    path.to_str().map(|s| s.to_string())
                } else {
                    None
                },
            }
        })
        .collect()
}

pub fn cached_model_path<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    expected_kind: ModelKind,
) -> Option<PathBuf> {
    let d = descriptor_for(id)?;
    if d.kind != expected_kind {
        return None;
    }
    let root = root_dir(app);
    let path = model_dir(&root, d).join(d.kind.filename());
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn verify_present(path: &Path, _d: &ModelDescriptor) -> bool {
    path.metadata().map(|m| m.len() > 0).unwrap_or(false)
}

fn root_dir<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    if let Ok(p) = app.path().app_data_dir() {
        return p.join("models");
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.vox.app/models")
}

fn model_dir(root: &Path, d: &ModelDescriptor) -> PathBuf {
    root.join(d.kind.dir()).join(d.id)
}

#[derive(Default)]
struct DownloadRegistry {
    inner: Mutex<std::collections::HashMap<String, CancellationToken>>,
}

impl DownloadRegistry {
    /// Reserve a slot for `id`; `None` if one is already in flight to
    /// avoid racing two writers on the same `.partial` file.
    fn try_insert(&self, id: &str) -> Option<CancellationToken> {
        let mut guard = self.inner.lock();
        if guard.contains_key(id) {
            return None;
        }
        let tok = CancellationToken::new();
        guard.insert(id.to_string(), tok.clone());
        Some(tok)
    }
    fn cancel(&self, id: &str) -> bool {
        if let Some(tok) = self.inner.lock().remove(id) {
            tok.cancel();
            true
        } else {
            false
        }
    }
    fn finish(&self, id: &str) {
        self.inner.lock().remove(id);
    }
}

static REGISTRY: once_cell::sync::Lazy<Arc<DownloadRegistry>> =
    once_cell::sync::Lazy::new(|| Arc::new(DownloadRegistry::default()));

/// Cancel an in-flight download. Returns true if a download was running.
pub fn cancel_download(id: &str) -> bool {
    REGISTRY.cancel(id)
}

#[derive(Debug, Clone, Serialize)]
struct ProgressPayload<'a> {
    id: &'a str,
    bytes_downloaded: u64,
    total_bytes: u64,
    eta_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct CancelPayload<'a> {
    id: &'a str,
}

/// Resumable, sha256-verified, atomic-on-success download. Emits
/// `model:progress` (≥200 ms apart), `model:done`, `model:error`, or
/// `model:canceled`.
pub async fn download_model(
    app: AppHandle<Wry>,
    id: String,
) -> anyhow::Result<()> {
    // Multi-file Core ML bundle — single `stream_to_file` doesn't apply.
    if id == "parakeet-tdt-0.6b-v3" {
        return download_parakeet_v3(&app).await;
    }

    let descriptor = descriptor_for(&id)
        .ok_or_else(|| anyhow::anyhow!("unknown model id: {id}"))?
        .clone();

    let root = root_dir(&app);
    let dir = model_dir(&root, &descriptor);
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("create cache dir: {e}"))?;
    let final_path = dir.join(descriptor.kind.filename());
    let partial_path = dir.join(format!("{}.partial", descriptor.kind.filename()));

    if final_path.exists() {
        let _ = app.emit(
            "model:done",
            serde_json::json!({"id": id, "already_installed": true}),
        );
        return Ok(());
    }

    let resume_from = partial_path.metadata().map(|m| m.len()).unwrap_or(0);

    let Some(cancel) = REGISTRY.try_insert(&id) else {
        let _ = app.emit(
            "model:error",
            serde_json::json!({"id": id, "error": "already downloading"}),
        );
        return Err(anyhow::anyhow!("already downloading: {id}"));
    };

    let result = do_download(&app, &descriptor, &partial_path, resume_from, cancel.clone()).await;
    REGISTRY.finish(&id);

    match result {
        Ok(verified) => {
            if verified {
                std::fs::rename(&partial_path, &final_path)
                    .map_err(|e| anyhow::anyhow!("rename partial: {e}"))?;
                let _ = app.emit("model:done", serde_json::json!({"id": id}));
            }
            Ok(())
        }
        Err(e) => {
            if cancel.is_cancelled() {
                let _ = app.emit("model:canceled", CancelPayload { id: &id });
                Ok(())
            } else {
                let _ =
                    app.emit("model:error", serde_json::json!({"id": id, "error": e.to_string()}));
                Err(e)
            }
        }
    }
}

async fn do_download(
    app: &AppHandle<Wry>,
    descriptor: &ModelDescriptor,
    partial_path: &Path,
    resume_from: u64,
    cancel: CancellationToken,
) -> anyhow::Result<bool> {
    let lock_path = partial_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("models.lock"));

    stream_to_file(
        app,
        StreamArgs {
            id: descriptor.id,
            url: descriptor.url,
            partial_path,
            expected_size: descriptor.size_bytes,
            expected_sha256: descriptor.sha256,
            event_prefix: "model",
            lock_file: lock_path.as_deref(),
        },
        resume_from,
        cancel,
    )
    .await?;
    Ok(true)
}

/// Args for the streaming downloader, shared with the inference-server
/// binary catalog path.
#[derive(Debug, Clone, Copy)]
pub struct StreamArgs<'a> {
    /// Surfaced on every progress / done / error event for routing.
    pub id: &'a str,
    /// HTTPS source URL.
    pub url: &'a str,
    /// Destination for the partial bytes; caller renames on success so
    /// a crash can resume via HTTP `Range:`.
    pub partial_path: &'a Path,
    /// Total-bytes fallback when the server omits `Content-Length`.
    pub expected_size: u64,
    /// When set, must match the streamed digest. When `None`, the
    /// computed digest is appended to `lock_file` as a soft canary.
    pub expected_sha256: Option<&'a str>,
    /// Tauri event family name; emits `<event_prefix>:progress` only.
    pub event_prefix: &'a str,
    /// Lockfile for soft sha256 canary when no `expected_sha256` is set.
    pub lock_file: Option<&'a Path>,
}

/// Whether the response lets us append to the existing `.partial`
/// (true) or forces a restart from byte 0 (false). ONLY a `206 Partial
/// Content` with a non-zero offset means the body is the suffix we asked
/// for. A `200` despite our `Range:` header means the server (or a
/// TLS-intercepting proxy) ignored the range and is sending the WHOLE
/// file — appending that onto the partial yields a `partial + full`
/// corrupt file, and the catalog ships no sha256 to catch it. Deciding
/// on the response status (not `resume_from`) is what makes resume safe.
fn resuming_from_response(resume_from: u64, status: u16) -> bool {
    resume_from > 0 && status == 206
}

/// Issue the GET, bounding the connect + header phase and making the wait
/// cancellable. The client's `read_timeout` only covers body reads, not
/// the TLS handshake / waiting on response headers, so a connection that
/// dies before the first byte (laptop asleep) would otherwise hang here.
/// The explicit timeout plus the cancel race close that gap.
async fn send_request(
    req: reqwest::RequestBuilder,
    cancel: &CancellationToken,
    url: &str,
) -> anyhow::Result<reqwest::Response> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(anyhow::anyhow!("canceled")),
        r = tokio::time::timeout(Duration::from_secs(60), req.send()) => match r {
            Err(_) => Err(anyhow::anyhow!("download timed out connecting to {url}")),
            Ok(Err(e)) => Err(anyhow::anyhow!("download GET: {e}")),
            Ok(Ok(resp)) => Ok(resp),
        },
    }
}

/// Streams `partial_path` from `args.url`. Memory residency is bounded
/// by reqwest's chunk size, so 17 GB downloads work on 8 GB Macs.
/// Caller owns the rename to the final path.
pub async fn stream_to_file<R: Runtime>(
    app: &AppHandle<R>,
    args: StreamArgs<'_>,
    resume_from: u64,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;

    // Root certs = webpki-roots ∪ macOS keychain trust store. The
    // keychain half (via `rustls-tls-native-roots` in Cargo.toml) is
    // what makes downloads work on work laptops behind a TLS-intercepting
    // proxy like Netskope — that proxy re-signs HTTPS with a CA IT
    // installed in the keychain, which webpki-roots alone does not know
    // about.
    let client = reqwest::Client::builder()
        .https_only(true)
        // Bound the connect + per-read waits. The motivating failure: a
        // laptop that sleeps mid-download. The TCP connection dies without
        // an RST, so a body read blocks forever — the download future never
        // returns, the REGISTRY slot for this id never frees, and every
        // retry is refused with "already downloading" until the app is
        // reinstalled. `read_timeout` turns the dead connection into an
        // error that releases the slot and leaves the `.partial` in place
        // for a clean resume. `connect_timeout` does the same for the
        // initial handshake.
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| anyhow::anyhow!("reqwest build: {e}"))?;

    // Acquire the response, with a single 416-triggered fallback. A range
    // request against an already-complete (or stale, oversized) `.partial`
    // returns `416 Range Not Satisfiable`; without this fallback the resume
    // fails identically on every attempt and the only fix is deleting the
    // partial by hand — which is what "reinstall the app" did for the user
    // who reported this. Discard the dead partial and refetch from scratch.
    let mut effective_resume = resume_from;
    let resp = {
        let mut req = client.get(args.url);
        if effective_resume > 0 {
            req = req.header("Range", format!("bytes={effective_resume}-"));
        }
        let first = send_request(req, &cancel, args.url).await?;
        if effective_resume > 0 && first.status().as_u16() == 416 {
            let _ = std::fs::remove_file(args.partial_path);
            effective_resume = 0;
            send_request(client.get(args.url), &cancel, args.url).await?
        } else {
            first
        }
    };

    let status = resp.status();
    if !status.is_success() && status.as_u16() != 206 {
        return Err(anyhow::anyhow!(
            "download HTTP {} for {}",
            status,
            args.url
        ));
    }

    // Append onto the partial only if the server honoured the resume; a 200
    // means it's sending the whole file, so we must restart at byte 0.
    let resuming = resuming_from_response(effective_resume, status.as_u16());
    let start_offset = if resuming { effective_resume } else { 0 };

    let total_bytes = resp
        .content_length()
        .map(|c| c + start_offset)
        .unwrap_or(args.expected_size);

    let mut file = OpenOptions::new()
        .create(true)
        .append(resuming)
        .write(true)
        .truncate(!resuming)
        .open(args.partial_path)
        .map_err(|e| anyhow::anyhow!("open partial: {e}"))?;

    let mut hasher = Sha256::new();
    if resuming {
        // Re-hash the existing prefix so the final digest is correct.
        use std::io::Read;
        let mut existing = std::fs::File::open(args.partial_path)
            .map_err(|e| anyhow::anyhow!("rehash open: {e}"))?;
        let mut buf = vec![0u8; 1 << 20];
        let mut left = start_offset;
        while left > 0 {
            let want = (buf.len() as u64).min(left) as usize;
            let n = existing
                .read(&mut buf[..want])
                .map_err(|e| anyhow::anyhow!("rehash read: {e}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            left -= n as u64;
        }
    }

    let mut downloaded: u64 = start_offset;
    let started = Instant::now();
    let mut last_emit = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);

    let progress_event = format!("{}:progress", args.event_prefix);

    let mut stream = resp.bytes_stream();
    // Race each body read against cancellation so a Cancel click (or
    // session teardown) interrupts a slow read immediately instead of
    // waiting up to `read_timeout` for the next chunk. The client's
    // `read_timeout` still bounds a fully idle / dead connection.
    loop {
        let next = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(anyhow::anyhow!("canceled")),
            n = stream.next() => n,
        };
        let Some(chunk) = next else { break };
        let bytes =
            chunk.map_err(|e| anyhow::anyhow!("stream chunk: {e}"))?;
        file.write_all(&bytes)
            .map_err(|e| anyhow::anyhow!("write partial: {e}"))?;
        hasher.update(&bytes);
        downloaded += bytes.len() as u64;

        if last_emit.elapsed() >= Duration::from_millis(200) {
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            let bps = (downloaded - start_offset) as f64 / elapsed;
            let remaining = total_bytes.saturating_sub(downloaded) as f64;
            let eta = if bps > 1.0 { Some((remaining / bps) as u64) } else { None };
            let _ = app.emit(
                &progress_event,
                ProgressPayload {
                    id: args.id,
                    bytes_downloaded: downloaded,
                    total_bytes,
                    eta_secs: eta,
                },
            );
            last_emit = Instant::now();
        }
    }

    // Snap UI to 100%.
    let _ = app.emit(
        &progress_event,
        ProgressPayload {
            id: args.id,
            bytes_downloaded: downloaded,
            total_bytes,
            eta_secs: Some(0),
        },
    );

    file.flush()
        .map_err(|e| anyhow::anyhow!("flush partial: {e}"))?;
    drop(file);

    let computed: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    if let Some(expected) = args.expected_sha256 {
        if !computed.eq_ignore_ascii_case(expected) {
            // Drop the corrupted partial so a retry doesn't resume into the same failure.
            let _ = std::fs::remove_file(args.partial_path);
            return Err(anyhow::anyhow!(
                "sha256 mismatch for {}: got {computed}, expected {expected}",
                args.id
            ));
        }
    } else if let Some(lp) = args.lock_file {
        let _ = append_to_lock(lp, args.id, &computed);
    }

    Ok(())
}

fn append_to_lock(lock_path: &Path, id: &str, sha: &str) -> std::io::Result<()> {
    use std::io::Write;
    let line = format!("{id} {sha}\n");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(lock_path)?;
    f.write_all(line.as_bytes())
}

/// One file in the Parakeet Core ML bundle. `relpath` is relative to
/// `<app_data>/models/stt/parakeet-tdt-0.6b-v3/`; `url` is the absolute
/// Hugging Face raw-file URL.
struct ParakeetFile {
    relpath: &'static str,
    url: &'static str,
    size_bytes: u64,
}

/// OS-dispatch: returns the macOS Core ML manifest or the Windows ONNX
/// manifest depending on the compile-target.
fn parakeet_v3_files() -> Vec<ParakeetFile> {
    #[cfg(target_os = "macos")]
    return parakeet_v3_files_macos();
    #[cfg(target_os = "windows")]
    return parakeet_v3_files_windows();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Vec::new(); // unsupported host — downloader returns no files
}

/// File manifest for the FluidAudio Parakeet TDT 0.6B v3 Core ML bundle.
/// Sourced from `huggingface.co/api/models/FluidInference/parakeet-tdt-0.6b-v3-coreml/tree/main`
/// at the time this constant was hardcoded; sizes are exact from that
/// snapshot. We pick the int8 `Encoder.mlmodelc` (the FluidAudio default
/// — `EncoderInt4.mlmodelc` is a separately-published int4 variant we
/// don't ship). Future repo updates will require bumping this list.
#[cfg(target_os = "macos")]
fn parakeet_v3_files_macos() -> Vec<ParakeetFile> {
    // The engine code reads `parakeet_vocab.json` (not the v3-suffixed
    // alias) — the HF repo publishes both with identical content; we
    // download the name the engine expects to keep load-time probes
    // simple.
    macro_rules! f {
        ($rel:expr, $size:expr) => {
            ParakeetFile {
                relpath: concat!("parakeet-tdt-0.6b-v3-coreml/", $rel),
                url: concat!(
                    "https://huggingface.co/FluidInference/parakeet-tdt-0.6b-v3-coreml/resolve/main/",
                    $rel
                ),
                size_bytes: $size,
            }
        };
    }
    vec![
        // Preprocessor.mlmodelc — small audio-feature graph.
        f!("Preprocessor.mlmodelc/coremldata.bin", 486),
        f!("Preprocessor.mlmodelc/metadata.json", 2841),
        f!("Preprocessor.mlmodelc/model.mil", 28181),
        f!("Preprocessor.mlmodelc/analytics/coremldata.bin", 243),
        f!("Preprocessor.mlmodelc/weights/weight.bin", 491_072),
        // Encoder.mlmodelc (int8) — the bulk of the download (~445 MB).
        f!("Encoder.mlmodelc/coremldata.bin", 485),
        f!("Encoder.mlmodelc/metadata.json", 2921),
        f!("Encoder.mlmodelc/model.mil", 959_769),
        f!("Encoder.mlmodelc/analytics/coremldata.bin", 243),
        f!("Encoder.mlmodelc/weights/weight.bin", 445_187_200),
        // Decoder.mlmodelc.
        f!("Decoder.mlmodelc/coremldata.bin", 554),
        f!("Decoder.mlmodelc/metadata.json", 3427),
        f!("Decoder.mlmodelc/model.mil", 13110),
        f!("Decoder.mlmodelc/analytics/coremldata.bin", 243),
        f!("Decoder.mlmodelc/weights/weight.bin", 23_604_992),
        // JointDecisionv3.mlmodelc — v3-specific joint network.
        f!("JointDecisionv3.mlmodelc/coremldata.bin", 521),
        f!("JointDecisionv3.mlmodelc/metadata.json", 3453),
        f!("JointDecisionv3.mlmodelc/model.mil", 11775),
        f!("JointDecisionv3.mlmodelc/analytics/coremldata.bin", 243),
        f!("JointDecisionv3.mlmodelc/weights/weight.bin", 12_642_764),
        // Vocab JSON (engine probes this file to gate load attempts).
        ParakeetFile {
            relpath: "parakeet-tdt-0.6b-v3-coreml/parakeet_vocab.json",
            url:
                "https://huggingface.co/FluidInference/parakeet-tdt-0.6b-v3-coreml/resolve/main/parakeet_vocab.json",
            size_bytes: 151_122,
        },
    ]
}

/// File manifest for the ONNX Parakeet TDT 0.6B v3 bundle used on Windows.
///
/// Files live directly under `<app_data>/models/stt/parakeet-tdt-0.6b-v3/`
/// (no sub-directory, unlike the macOS Core ML bundle). `parakeet-rs`'s
/// `ParakeetTDT::from_pretrained` reads `vocab.txt`, `encoder-model.onnx`,
/// and `decoder_joint-model.onnx` from the directory root; ONNX Runtime
/// resolves `encoder-model.onnx.data` from the `location` field in the
/// .onnx protobuf and it must sit next to the .onnx with this exact name.
///
/// Sizes and SHA-256 (LFS oid) sourced from HF API on 2026-06-01:
///   GET https://huggingface.co/api/models/istupakov/parakeet-tdt-0.6b-v3-onnx/tree/main
///   encoder-model.onnx        41_770_866  98a74b21b4cc0017c1e7030319a4a96f4a9506e50f0708f3a516d02a77c96bb1
///   encoder-model.onnx.data  2_435_420_160  9a22d372c51455c34f13405da2520baefb7125bd16981397561423ed32d24f36
///   decoder_joint-model.onnx  72_520_893  e978ddf6688527182c10fde2eb4b83068421648985ef23f7a86be732be8706c1
///   vocab.txt                     93_939  (no LFS)
///   Total                    2_549_805_858
///
/// We do not ship `nemo128.onnx` (unused by parakeet-rs) or the int8
/// variants (accuracy regression unmeasured against Vox's test set).
#[cfg(not(target_os = "macos"))]
fn parakeet_v3_files_windows() -> Vec<ParakeetFile> {
    macro_rules! fw {
        ($rel:expr, $size:expr) => {
            ParakeetFile {
                relpath: $rel,
                url: concat!(
                    "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/",
                    $rel
                ),
                size_bytes: $size,
            }
        };
    }
    vec![
        fw!("encoder-model.onnx", 41_770_866),
        // External weights — ONNX Runtime resolves this from the protobuf
        // `location` field; the filename must match exactly.
        fw!("encoder-model.onnx.data", 2_435_420_160),
        fw!("decoder_joint-model.onnx", 72_520_893),
        // vocab.txt is probed by the engine to gate load attempts.
        fw!("vocab.txt", 93_939),
    ]
}

/// Multi-file downloader for the Parakeet Core ML bundle. Mirrors
/// `download_model`'s event surface (`model:progress|done|error|canceled`
/// keyed on the bundle id) so the frontend doesn't need a separate
/// listener path. Files are downloaded sequentially via the existing
/// `stream_to_file` (resumable per-file via .partial), idempotent on
/// already-complete files, and progress is the byte-sum across all
/// files so the UI sees one monotonic bar instead of 21 mini ones.
pub async fn download_parakeet_v3<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<()> {
    const ID: &str = "parakeet-tdt-0.6b-v3";
    let descriptor = descriptor_for(ID)
        .ok_or_else(|| anyhow::anyhow!("unknown model id: {ID}"))?
        .clone();

    let root = root_dir(app);
    let bundle_dir = model_dir(&root, &descriptor);
    std::fs::create_dir_all(&bundle_dir)
        .map_err(|e| anyhow::anyhow!("create cache dir: {e}"))?;

    let files = parakeet_v3_files();
    let total_bytes: u64 = files.iter().map(|f| f.size_bytes).sum();

    // Skip-when-complete short-circuit: if every file is already on
    // disk at (at least) the expected size, treat as already installed
    // without re-touching the network or the registry. Manifest sizes
    // are a baseline, not an exact match — HF can re-upload an asset
    // larger than what we hardcoded, and a strict `==` would force a
    // full re-download of an otherwise-valid bundle.
    let all_done = files.iter().all(|f| {
        let path = bundle_dir.join(f.relpath);
        path.metadata().map(|m| m.len()).unwrap_or(0) >= f.size_bytes
    });
    if all_done {
        let _ = app.emit(
            "model:done",
            serde_json::json!({"id": ID, "already_installed": true}),
        );
        return Ok(());
    }

    let Some(cancel) = REGISTRY.try_insert(ID) else {
        let _ = app.emit(
            "model:error",
            serde_json::json!({"id": ID, "error": "already downloading"}),
        );
        return Err(anyhow::anyhow!("already downloading: {ID}"));
    };

    let result = do_parakeet_download(app, &bundle_dir, &files, total_bytes, cancel.clone()).await;
    REGISTRY.finish(ID);

    match result {
        Ok(()) => {
            let _ = app.emit("model:done", serde_json::json!({"id": ID}));
            Ok(())
        }
        Err(e) => {
            if cancel.is_cancelled() {
                let _ = app.emit("model:canceled", CancelPayload { id: ID });
                Ok(())
            } else {
                let _ = app.emit(
                    "model:error",
                    serde_json::json!({"id": ID, "error": e.to_string()}),
                );
                Err(e)
            }
        }
    }
}

async fn do_parakeet_download<R: Runtime>(
    app: &AppHandle<R>,
    bundle_dir: &Path,
    files: &[ParakeetFile],
    total_bytes: u64,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    const ID: &str = "parakeet-tdt-0.6b-v3";

    let started = Instant::now();
    let mut completed_bytes: u64 = 0;

    for f in files {
        if cancel.is_cancelled() {
            return Err(anyhow::anyhow!("canceled"));
        }
        let dest = bundle_dir.join(f.relpath);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("create dir {parent:?}: {e}"))?;
        }

        let on_disk = dest.metadata().map(|m| m.len()).unwrap_or(0);
        // Manifest size is a lower bound — see all_done comment above.
        if on_disk >= f.size_bytes {
            completed_bytes += f.size_bytes;
            // Snap the bar so already-installed files don't appear stuck.
            let _ = app.emit(
                "model:progress",
                ProgressPayload {
                    id: ID,
                    bytes_downloaded: completed_bytes,
                    total_bytes,
                    eta_secs: None,
                },
            );
            continue;
        }

        let partial = {
            let mut p = dest.clone();
            let mut name = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            name.push_str(".partial");
            p.set_file_name(name);
            p
        };
        let resume_from = partial.metadata().map(|m| m.len()).unwrap_or(0);

        // Aggregate progress: poll the .partial file's size on a 250 ms
        // ticker while `stream_to_file` runs. This avoids needing a
        // per-file event listener (which would require ProgressPayload
        // to be Deserialize) and keeps the UI bar smooth across the
        // ~445 MB Encoder weights file.
        let app_ticker = app.clone();
        let partial_for_ticker = partial.clone();
        let cancel_ticker = cancel.clone();
        let file_size = f.size_bytes;
        let completed_snapshot = completed_bytes;
        let total_snapshot = total_bytes;
        let stop = CancellationToken::new();
        let stop_for_ticker = stop.clone();
        let ticker = tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_millis(250));
            interval.set_missed_tick_behavior(
                tokio::time::MissedTickBehavior::Delay,
            );
            loop {
                tokio::select! {
                    _ = stop_for_ticker.cancelled() => break,
                    _ = cancel_ticker.cancelled() => break,
                    _ = interval.tick() => {
                        let in_flight = partial_for_ticker
                            .metadata()
                            .map(|m| m.len())
                            .unwrap_or(0);
                        let done = completed_snapshot
                            + in_flight.min(file_size);
                        let elapsed = started.elapsed().as_secs_f64().max(0.001);
                        let bps = done as f64 / elapsed;
                        let remaining = total_snapshot.saturating_sub(done) as f64;
                        let eta = if bps > 1.0 { Some((remaining / bps) as u64) } else { None };
                        let _ = app_ticker.emit(
                            "model:progress",
                            ProgressPayload {
                                id: ID,
                                bytes_downloaded: done,
                                total_bytes: total_snapshot,
                                eta_secs: eta,
                            },
                        );
                    }
                }
            }
        });

        // `event_prefix` is "parakeet-file" so the per-file emits don't
        // collide with the aggregated `model:*` channel the UI listens on.
        let result = stream_to_file(
            app,
            StreamArgs {
                id: ID,
                url: f.url,
                partial_path: &partial,
                expected_size: f.size_bytes,
                expected_sha256: None,
                event_prefix: "parakeet-file",
                lock_file: None,
            },
            resume_from,
            cancel.clone(),
        )
        .await;

        stop.cancel();
        let _ = ticker.await;

        // Cancel cleanup: a `stream_to_file` Err while the cancel token
        // is set leaves the in-flight `.partial` on disk. Without this,
        // a user who hits Cancel and never retries leaves the partial
        // bytes wasting disk space until the next download attempt
        // resumes (or the model dir is wiped). Best-effort removal —
        // ignore errors so we don't mask the original cancel reason.
        if let Err(e) = result {
            if cancel.is_cancelled() {
                let _ = std::fs::remove_file(&partial);
            }
            return Err(e);
        }

        std::fs::rename(&partial, &dest)
            .map_err(|e| anyhow::anyhow!("rename partial {partial:?}: {e}"))?;
        completed_bytes += f.size_bytes;
    }

    // Snap UI to 100% on the bundle channel.
    let _ = app.emit(
        "model:progress",
        ProgressPayload {
            id: ID,
            bytes_downloaded: total_bytes,
            total_bytes,
            eta_secs: Some(0),
        },
    );
    Ok(())
}

pub fn delete_model<R: Runtime>(app: &AppHandle<R>, id: &str) -> anyhow::Result<()> {
    let d = descriptor_for(id)
        .ok_or_else(|| anyhow::anyhow!("unknown model id: {id}"))?;

    // Release any process holding the file open before unlinking.
    // macOS keeps the inode (and disk bytes) alive until the last
    // handle closes, so a bare `remove_dir_all` while Whisper has the
    // GGML mmap'd or `llama-server` is serving the GGUF would silently
    // leave the user's disk space reserved.
    if let Some(registry) = app.try_state::<crate::engines::Registry>() {
        registry.drop_for_model(id);
    }
    if matches!(d.kind, ModelKind::Llm) {
        crate::inference_proc::shutdown_if_using(app, id);
    }

    let root = root_dir(app);
    let dir = model_dir(&root, d);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("rm cache dir: {e}"))?;
    }
    Ok(())
}

/// Physical RAM in rounded GB; 0 if unknown.
pub fn physical_ram_gb() -> u64 {
    let mut sys = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::new().with_memory(sysinfo::MemoryRefreshKind::new().with_ram()),
    );
    sys.refresh_memory();
    sys.total_memory() / (1024 * 1024 * 1024)
}

pub fn recommend_tier() -> &'static str {
    if physical_ram_gb() < 12 {
        "small"
    } else {
        "balanced"
    }
}

pub fn recommended_stt_model_id() -> &'static str {
    match recommend_tier() {
        "small" => "whisper-small-en-q5_1",
        "quality" => "whisper-large-v3-q5_0",
        _ => "whisper-large-v3-turbo-q5_0",
    }
}

/// ISO 639-1 codes for the 25 languages supported by Parakeet TDT 0.6B v3.
/// Source: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3 model card.
///
/// Mirrored in TypeScript (`src/lib/ipc.ts::PARAKEET_SUPPORTED_LANGUAGES`),
/// which is the only consumer today — Rust callers may appear later, so
/// keep the constant + helper here under `#[allow(dead_code)]`.
#[allow(dead_code)]
pub const PARAKEET_SUPPORTED_LANGUAGES: &[&str] = &[
    "bg", "hr", "cs", "da", "nl", "en", "et", "fi", "fr", "de", "el",
    "hu", "it", "lv", "lt", "mt", "pl", "pt", "ro", "sk", "sl", "es",
    "sv", "ru", "uk",
];

#[allow(dead_code)]
pub fn parakeet_supports_language(code: &str) -> bool {
    PARAKEET_SUPPORTED_LANGUAGES.contains(&code)
}

/// Pick the recommended STT engine id given a language preference and
/// runtime capabilities. Routes Parakeet-supported languages to
/// "parakeet-local" when Parakeet is feasible (Apple Silicon + macOS 14+),
/// otherwise falls back to "whisper-local".
///
/// `language` is the user's language setting from `Settings::language`
/// (ISO 639-1 like "en", "es", or the literal "auto").
/// `parakeet_available` is whether the runtime supports Parakeet —
/// supplied by the caller (engine availability is computed in
/// engines::parakeet_bridge::is_available, but this helper stays
/// catalog-side and platform-agnostic).
#[allow(dead_code)]
pub fn recommended_stt_engine_for(language: &str, parakeet_available: bool) -> &'static str {
    if !parakeet_available {
        return "whisper-local";
    }
    // "auto" → default to Parakeet when it can detect across the 25
    // supported langs; users on rare languages will pick a specific
    // language in onboarding which then routes to Whisper.
    if language == "auto" || parakeet_supports_language(language) {
        "parakeet-local"
    } else {
        "whisper-local"
    }
}

/// Pick the recommended STT model id for a given engine and tier.
/// Parakeet has only one model variant; Whisper has the existing
/// tier-aware logic.
#[allow(dead_code)]
pub fn recommended_stt_model_id_for_engine(engine: &str, _tier: &str) -> &'static str {
    match engine {
        "parakeet-local" => "parakeet-tdt-0.6b-v3",
        _ => recommended_stt_model_id(),
    }
}

pub fn recommended_llm_model_id() -> &'static str {
    match recommend_tier() {
        "quality" => "gemma-4-26b-a4b-q4_k_m",
        "small" => "gemma-4-e2b-q5_k_m",
        _ => "gemma-4-e4b-q5_k_m",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_registry_dedupes_by_id() {
        let reg = DownloadRegistry::default();
        let first = reg.try_insert("model-x");
        assert!(first.is_some(), "first insert should succeed");
        let second = reg.try_insert("model-x");
        assert!(second.is_none(), "second insert for same id must refuse");

        // After finish, a fresh download for the same id is allowed again.
        reg.finish("model-x");
        let third = reg.try_insert("model-x");
        assert!(third.is_some(), "post-finish reinsert should succeed");
    }

    #[test]
    fn resume_only_appends_on_206_with_offset() {
        // The only safe append: a real partial-content response.
        assert!(resuming_from_response(100, 206));
        // Server ignored our Range and is sending the whole file — must
        // restart from 0, not append onto the partial (corruption guard).
        assert!(!resuming_from_response(100, 200));
        // Fresh download: nothing on disk to resume from.
        assert!(!resuming_from_response(0, 200));
        assert!(!resuming_from_response(0, 206));
        // 416 is handled upstream (discard + refetch) and never reaches
        // here as a resume, but be explicit that it isn't an append.
        assert!(!resuming_from_response(100, 416));
    }
}
