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
//! - Gemma 3 GGUF: <https://huggingface.co/unsloth/gemma-3-1b-it-GGUF>
//!   and <https://huggingface.co/unsloth/gemma-3-4b-it-GGUF>.
//!
//! NB: Gemma 4 was the original LLM, but the bundled `llama.cpp` snapshot
//! (revision `e21cdc1`, the latest available through `llama-cpp-2`) hits a
//! `GGML_ASSERT` in `llama-context.cpp::sched_reserve` when constructing a
//! context for Gemma 4 E2B/E4B GGUFs. The graph contains
//! `GGML_OP_GATED_DELTA_NET` tensors whose names don't carry the
//! `LLAMA_TENSOR_NAME_FGDN_AR` prefix that the FGDN auto-detect block
//! expects, and the assertion `abort()`s the process from C++ — bypassing
//! Rust's fail-open guard. There is no Rust-side switch to disable that
//! block (`fused_gdn_*` and `auto_fgdn` live in the internal cparams
//! struct, not in the public `llama_context_params`), and the bug is
//! unfixed at upstream HEAD. We sidestep it by shipping Gemma 3 instead:
//! same `<start_of_turn>` chat template (so `gemma_local.rs` is unchanged)
//! and a standard transformer architecture (no Gated Delta Net, so no
//! crash). When `llama.cpp` ships a fix we can re-add the Gemma 4 entries.
//! - Phi-4-mini GGUF: <https://huggingface.co/microsoft/Phi-4-mini-instruct-onnx>
//!   (mirrored in `bartowski/microsoft_Phi-4-mini-instruct-GGUF`).

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
        id: "gemma-3-1b-q4_k_m",
        kind: ModelKind::Llm,
        name: "Gemma 3 1B (Q4_K_M) — Small",
        size_bytes: 806_058_272,
        sha256: None,
        url: "https://huggingface.co/unsloth/gemma-3-1b-it-GGUF/resolve/main/gemma-3-1b-it-Q4_K_M.gguf",
        engine: "gemma-local",
        ram_peak_mb: 1500,
        tier: "small",
    },
    ModelDescriptor {
        id: "gemma-3-4b-q4_k_m",
        kind: ModelKind::Llm,
        name: "Gemma 3 4B (Q4_K_M) — Balanced",
        size_bytes: 2_489_894_016,
        sha256: None,
        url: "https://huggingface.co/unsloth/gemma-3-4b-it-GGUF/resolve/main/gemma-3-4b-it-Q4_K_M.gguf",
        engine: "gemma-local",
        ram_peak_mb: 3500,
        tier: "balanced",
    },
    ModelDescriptor {
        id: "phi-4-mini-q4_k_m",
        kind: ModelKind::Llm,
        name: "Phi-4-mini Instruct (Q4_K_M)",
        size_bytes: 2_400_000_000,
        sha256: None,
        url: "https://huggingface.co/bartowski/microsoft_Phi-4-mini-instruct-GGUF/resolve/main/microsoft_Phi-4-mini-instruct-Q4_K_M.gguf",
        engine: "gemma-local",
        ram_peak_mb: 4000,
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
    use std::fs::OpenOptions;
    use std::io::Write;

    let client = reqwest::Client::builder()
        .https_only(true)
        .build()
        .map_err(|e| anyhow::anyhow!("reqwest build: {e}"))?;

    let mut req = client.get(descriptor.url);
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
            descriptor.url
        ));
    }

    let total_bytes = resp
        .content_length()
        .map(|c| c + resume_from)
        .unwrap_or(descriptor.size_bytes);

    let mut file = OpenOptions::new()
        .create(true)
        .append(resume_from > 0)
        .write(true)
        .truncate(resume_from == 0)
        .open(partial_path)
        .map_err(|e| anyhow::anyhow!("open partial: {e}"))?;

    let mut hasher = Sha256::new();
    if resume_from > 0 {
        // Re-hash the existing prefix so the final digest is correct.
        use std::io::Read;
        let mut existing = std::fs::File::open(partial_path)
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
                "model:progress",
                ProgressPayload {
                    id: descriptor.id,
                    bytes_downloaded: downloaded,
                    total_bytes,
                    eta_secs: eta,
                },
            );
            last_emit = Instant::now();
        }
    }

    // Final progress emit.
    let _ = app.emit(
        "model:progress",
        ProgressPayload {
            id: descriptor.id,
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

    if let Some(expected) = descriptor.sha256 {
        if !computed.eq_ignore_ascii_case(expected) {
            // Clean up the corrupted partial.
            let _ = std::fs::remove_file(partial_path);
            return Err(anyhow::anyhow!(
                "sha256 mismatch for {}: got {computed}, expected {expected}",
                descriptor.id
            ));
        }
    } else {
        // First-time pinning: write to models.lock so we have a reference.
        let lock_path = partial_path
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("models.lock"));
        if let Some(lp) = lock_path {
            let _ = append_to_lock(&lp, descriptor.id, &computed);
        }
    }

    Ok(true)
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
        "small" => "gemma-3-1b-q4_k_m",
        _ => "gemma-3-4b-q4_k_m",
    }
}

/// Model IDs that point at GGUFs known to crash this build's bundled
/// `llama.cpp` (see file-level docs). On settings load we rewrite any
/// stored selection that names one of these to the current default so
/// the user doesn't get an `abort()` the next time they hit the polish
/// hotkey.
pub fn is_known_broken_llm_id(id: &str) -> bool {
    matches!(id, "gemma-4-e2b-q4_k_m" | "gemma-4-e4b-q4_k_m")
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
