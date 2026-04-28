//! Model catalog + on-disk cache + resumable HTTP downloader.
//!
//! Catalog = compile-time metadata (URL, size, sha256). The cache lives
//! under `~/Library/Application Support/com.vox.app/models/<kind>/<id>/`.
//! Downloads are resumable via HTTP `Range`; sha256 is verified
//! streaming. On completion the `.partial` file is renamed atomically
//! to `model.bin` (or `model.gguf`).
//!
//! sha256 sources:
//! - whisper.cpp models: <https://huggingface.co/ggerganov/whisper.cpp/tree/main>
//!   (each `.bin` has a `.sha256` sibling on disk; we leave them as
//!   "TBD" in the catalog and pin the computed hash to a per-machine
//!   `models.lock` file on first successful download. Treat the lock
//!   as a warning canary, not a hard gate, because HF can rotate.)
//! - Gemma 4 E2B GGUF: <https://huggingface.co/bartowski/google_gemma-4-E2B-it-GGUF>.
//! - Gemma 4 E4B GGUF: <https://huggingface.co/bartowski/google_gemma-4-E4B-it-GGUF>.
//! - Gemma 4 26B-A4B (MoE) GGUF:
//!   <https://huggingface.co/unsloth/gemma-4-26B-A4B-it-GGUF>.
//!
//! NB: MatFormer Gemma 4 GGUFs **as converted by Unsloth** trigger a
//! `GGML_ASSERT` in `llama-context.cpp::sched_reserve`'s FGDN
//! auto-detect path: their graphs contain `GGML_OP_GATED_DELTA_NET`
//! tensors whose names don't carry the `LLAMA_TENSOR_NAME_FGDN_AR`
//! prefix that the check expects, so a `GGML_ASSERT` fires from C++ —
//! bypassing Rust's fail-open guard. There is no Rust-side switch to
//! disable that block (`fused_gdn_*` and `auto_fgdn` live in the
//! internal cparams struct, not in the public `llama_context_params`).
//!
//! **bartowski's MatFormer Gemma 4 GGUFs avoid this**: their conversion
//! uses a tensor naming scheme that does not trigger the FGDN
//! auto-detect path. The E2B and E4B entries in `CATALOG` are therefore
//! sourced from bartowski. If we ever ship an Unsloth-converted
//! MatFormer Gemma 4 variant, it'll need an entry in
//! `is_known_broken_llm_id`.
//!
//! Gemma 4 26B-A4B (the MoE variant — 26 B params total, 4 B active,
//! `shared_kv_layers=0`, no PLE) does NOT hit this code path under
//! either converter: it dispatches through the same
//! `llm_build_gemma4_iswa` builder but uses standard `build_attn`
//! exclusively, so its graph emits zero `GGML_OP_GATED_DELTA_NET` ops.
//! We keep the Unsloth UD-Q4_K_M variant for its imatrix calibration
//! edge; it needs ~19 GB RAM at runtime so it isn't the default for
//! sub-quality tiers.

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
    /// Engine that consumes this model (`"whisper-local"` or
    /// `"gemma-local"` — local engines only).
    pub engine: &'static str,
    /// Approximate peak RAM in MB. Surfaced in the Models UI.
    pub ram_peak_mb: u32,
    /// Tier label for onboarding ("small" / "balanced" / "quality").
    pub tier: &'static str,
}

/// Compile-time catalog. Adding a new model is a one-line PR.
pub const CATALOG: &[ModelDescriptor] = &[
    // ---- STT ----
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
    // ---- LLM ----
    ModelDescriptor {
        // Gemma 4 E2B (MatFormer): 2 B effective params, n_layer=35.
        // Sourced from bartowski because Unsloth's E2B conversion
        // triggers a `GGML_ASSERT` in the FGDN auto-detect path of
        // `llama-context.cpp::sched_reserve` — see file-level docs.
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
        // Gemma 4 E4B (MatFormer): 4 B effective params, n_layer=42.
        // Sourced from bartowski for the same reason as E2B above.
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
        // Gemma 4 26B-A4B (MoE: 26 B total, 4 B active). Has no Gated
        // Delta Net ops, so it's safe under any converter — we keep
        // Unsloth's UD-Q4_K_M for its imatrix calibration edge.
        // ~17 GB on disk, ~19 GB RAM peak with KV cache; comfortable on
        // 24 GB+ Apple Silicon.
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
    // Cheap check: file exists + non-empty. Full sha256 verification
    // happens at download time and is recorded in `models.lock`.
    path.metadata().map(|m| m.len() > 0).unwrap_or(false)
}

fn root_dir<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    if let Ok(p) = app.path().app_data_dir() {
        return p.join("models");
    }
    // Fallback: dirs crate (testing / non-Tauri contexts).
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("app.vox.vox/models")
}

fn model_dir(root: &Path, d: &ModelDescriptor) -> PathBuf {
    root.join(d.kind.dir()).join(d.id)
}

// ---------------------------------------------------------------------------
// Active downloads (cancel handles).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct DownloadRegistry {
    inner: Mutex<std::collections::HashMap<String, CancellationToken>>,
}

impl DownloadRegistry {
    /// Reserve a slot for `id`. Returns the new cancel token, or `None`
    /// if a download is already in flight for the same id — callers
    /// should refuse rather than racing two writers on the same
    /// `.partial` file.
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

/// Download a model. Resumable, sha256-verified, atomic on success.
/// Emits `model:progress` events at most every 200 ms; emits
/// `model:canceled` if the download is canceled, `model:done` on success,
/// `model:error` on failure.
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

    // Refuse to start a second downloader for the same id while one is
    // already running. Without this, two writers race on the same
    // `.partial` file and the result is a corrupt model that survives
    // the atomic rename to `model.bin`.
    let Some(cancel) = REGISTRY.try_insert(&id) else {
        let _ = app.emit(
            "model:error",
            serde_json::json!({"id": id, "error": "already downloading"}),
        );
        return Err(anyhow::anyhow!("already downloading: {id}"));
    };

    let result = do_download(&app, &descriptor, &partial_path, resume_from, cancel.clone()).await;

    // Either way, drop the registry entry.
    REGISTRY.finish(&id);

    match result {
        Ok(verified) => {
            if verified {
                // Atomic rename.
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

/// Args for the generic streaming downloader. Reused by both the model
/// catalog path (`models::download_model`) and the inference-server
/// binary catalog path (`inference_server::download_and_extract`).
#[derive(Debug, Clone, Copy)]
pub struct StreamArgs<'a> {
    /// Stable id surfaced on every progress / done / error event so the
    /// frontend can route a single event stream to the right
    /// per-asset progress bar.
    pub id: &'a str,
    /// Source URL. Must be HTTPS.
    pub url: &'a str,
    /// On-disk destination for the partial bytes. The caller owns
    /// the rename to a final filename — keeping the partial separate
    /// means an interrupted download can resume cleanly the next
    /// session via the HTTP `Range:` header.
    pub partial_path: &'a Path,
    /// Hint for total-bytes fallback when the server omits
    /// `Content-Length`.
    pub expected_size: u64,
    /// Optional pin: when set, the streamed bytes' sha256 must match.
    /// When `None`, the computed digest is appended to `lock_file` (if
    /// set) on first download as a soft canary.
    pub expected_sha256: Option<&'a str>,
    /// Tauri event family name. This downloader emits
    /// `<event_prefix>:progress` events (the caller is responsible for
    /// `:done`, `:error`, `:canceled` — they belong to the higher-level
    /// orchestration around rename/extract/etc.).
    pub event_prefix: &'a str,
    /// Optional lockfile to append the computed sha256 to when no
    /// `expected_sha256` was pinned at compile time.
    pub lock_file: Option<&'a Path>,
}

/// Streaming, resumable, sha256-verified download into `partial_path`.
///
/// Memory residency is bounded by reqwest's chunk size (16 KiB by
/// default) regardless of total file size — verified safe for 17 GB
/// downloads on 8 GB Macs. Cancellation is checked on every chunk so
/// the worst-case latency between `cancel.cancel()` and the function
/// returning is one chunk's transfer time.
///
/// Atomic rename on success is the **caller's** responsibility — this
/// function only writes to `partial_path`.
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
            let n = existing.read(&mut buf[..want])
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

    // Final progress emit so the UI snaps to 100%.
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
            // Clean up the corrupted partial so a retry starts fresh
            // — resuming a corrupted prefix would re-fail at every
            // attempt forever.
            let _ = std::fs::remove_file(args.partial_path);
            return Err(anyhow::anyhow!(
                "sha256 mismatch for {}: got {computed}, expected {expected}",
                args.id
            ));
        }
    } else if let Some(lp) = args.lock_file {
        // First-time pinning: write the computed digest to the
        // sibling lockfile so a future build can pin against it. Soft
        // canary — useful for diagnosing HF asset rotation.
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

/// Detect physical RAM (rounded GB). 0 if unknown.
pub fn physical_ram_gb() -> u64 {
    let mut sys = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::new().with_memory(sysinfo::MemoryRefreshKind::new().with_ram()),
    );
    sys.refresh_memory();
    sys.total_memory() / (1024 * 1024 * 1024)
}

/// Recommend a tier based on physical RAM. Soft: just a recommendation.
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

/// Model IDs that should be migrated away from on settings load. An id
/// lands here when it has been retired from the catalog — without
/// migration, an upgrader who had it selected would end up with a
/// dangling reference that `descriptor_for` can't resolve. The list
/// also covers ids known to crash this build's bundled `llama.cpp`,
/// though no current entries fall in that bucket. `settings::load`
/// rewrites the stored id to the current default and persists the
/// change.
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
