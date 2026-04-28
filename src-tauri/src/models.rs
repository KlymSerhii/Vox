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
    // Gemma 4 MatFormer entries are sourced from bartowski; see
    // file-level docs for why Unsloth's conversion crashes.
    ModelDescriptor {
        id: "gemma-4-e2b-q4_k_m",
        kind: ModelKind::Llm,
        name: "Gemma 4 E2B (Q4_K_M) — Small",
        size_bytes: 3_462_677_760,
        sha256: None,
        url: "https://huggingface.co/bartowski/google_gemma-4-E2B-it-GGUF/resolve/main/google_gemma-4-E2B-it-Q4_K_M.gguf",
        engine: "gemma-local",
        ram_peak_mb: 4000,
        tier: "small",
    },
    ModelDescriptor {
        id: "gemma-4-e4b-q4_k_m",
        kind: ModelKind::Llm,
        name: "Gemma 4 E4B (Q4_K_M) — Balanced",
        size_bytes: 5_405_167_904,
        sha256: None,
        url: "https://huggingface.co/bartowski/google_gemma-4-E4B-it-GGUF/resolve/main/google_gemma-4-E4B-it-Q4_K_M.gguf",
        engine: "gemma-local",
        ram_peak_mb: 6000,
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
        ram_peak_mb: 19000,
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
        .join("app.vox.vox/models")
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

    let client = reqwest::Client::builder()
        .https_only(true)
        .build()
        .map_err(|e| anyhow::anyhow!("reqwest build: {e}"))?;

    let mut req = client.get(args.url);
    if resume_from > 0 {
        req = req.header("Range", format!("bytes={resume_from}-"));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("download GET: {e}"))?;

    let status = resp.status();
    if !status.is_success() && status.as_u16() != 206 {
        return Err(anyhow::anyhow!(
            "download HTTP {} for {}",
            status,
            args.url
        ));
    }

    let total_bytes = resp
        .content_length()
        .map(|c| c + resume_from)
        .unwrap_or(args.expected_size);

    let mut file = OpenOptions::new()
        .create(true)
        .append(resume_from > 0)
        .write(true)
        .truncate(resume_from == 0)
        .open(args.partial_path)
        .map_err(|e| anyhow::anyhow!("open partial: {e}"))?;

    let mut hasher = Sha256::new();
    if resume_from > 0 {
        // Re-hash the existing prefix so the final digest is correct.
        use std::io::Read;
        let mut existing = std::fs::File::open(args.partial_path)
            .map_err(|e| anyhow::anyhow!("rehash open: {e}"))?;
        let mut buf = vec![0u8; 1 << 20];
        let mut left = resume_from;
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

    let mut downloaded: u64 = resume_from;
    let started = Instant::now();
    let mut last_emit = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);

    let progress_event = format!("{}:progress", args.event_prefix);

    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel.is_cancelled() {
            return Err(anyhow::anyhow!("canceled"));
        }
        let bytes =
            chunk.map_err(|e| anyhow::anyhow!("stream chunk: {e}"))?;
        file.write_all(&bytes)
            .map_err(|e| anyhow::anyhow!("write partial: {e}"))?;
        hasher.update(&bytes);
        downloaded += bytes.len() as u64;

        if last_emit.elapsed() >= Duration::from_millis(200) {
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            let bps = (downloaded - resume_from) as f64 / elapsed;
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

pub fn delete_model<R: Runtime>(app: &AppHandle<R>, id: &str) -> anyhow::Result<()> {
    let d = descriptor_for(id)
        .ok_or_else(|| anyhow::anyhow!("unknown model id: {id}"))?;
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

pub fn recommended_llm_model_id() -> &'static str {
    match recommend_tier() {
        "quality" => "gemma-4-26b-a4b-q4_k_m",
        "small" => "gemma-4-e2b-q4_k_m",
        _ => "gemma-4-e4b-q4_k_m",
    }
}

/// IDs `settings::load` migrates to the current default — either
/// retired from the catalog (a dangling reference `descriptor_for`
/// can't resolve) or known to crash the bundled `llama.cpp`.
pub fn is_known_broken_llm_id(id: &str) -> bool {
    matches!(id, "phi-4-mini-q4_k_m")
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
}
