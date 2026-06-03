//! Catalog + downloader + extractor for the out-of-process
//! `llama-server` inference engine.
//!
//! Why this exists: `llama-cpp-2` was linked into the Tauri binary so a
//! C++ assert (notably the FGDN auto-detect crash on Gemma 4 MatFormer
//! GGUFs) brought the entire menu-bar app down. With Vox used during
//! live sales calls that's unacceptable. This module owns the
//! "fetch a pre-built `llama-server` from the official llama.cpp
//! GitHub release into AppData and verify it" half of the new
//! out-of-process design; `inference_proc.rs` owns the spawn /
//! supervise / kill half.
//!
//! Lifecycle on disk:
//!
//! ```text
//! <AppData>/inference-server/<version>/
//!   ├── archive.{tar.gz|zip}           ← downloaded once, kept for diag
//!   ├── archive.{tar.gz|zip}.partial   ← removed on success
//!   ├── extracted/...                  ← `tar -xzf` / `zip` output
//!   └── .complete                      ← sentinel: extraction verified
//! ```
//!
//! `is_installed()` checks the sentinel rather than re-walking the
//! extracted tree, so the warm path is two stat()s.
//!
//! Bumping `llama.cpp`: change `LLAMA_CPP_TAG` and the `expected_sha256`
//! pin (when one ships). Users on the old tag get a fresh download into
//! a different version directory; the old tree is left for one launch
//! cycle and reaped by `cleanup_stale_versions` afterward.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio_util::sync::CancellationToken;

use crate::models::{self, StreamArgs};

/// Pinned llama.cpp release tag. Bump in lockstep with `expected_sha256`
/// in the per-host descriptors below. Reproducibility wins over
/// chasing latest — Gemma cleanup must behave identically across
/// Vox installs of the same version.
pub const LLAMA_CPP_TAG: &str = "b8953";

#[derive(Debug, Clone, Copy, Serialize)]
#[allow(dead_code)] // Zip variant is win-only; TarGz is mac-only.
pub enum ArchiveFormat {
    TarGz,
    Zip,
}

impl ArchiveFormat {
    fn extension(self) -> &'static str {
        match self {
            ArchiveFormat::TarGz => "tar.gz",
            ArchiveFormat::Zip => "zip",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct BinaryDescriptor {
    pub id: &'static str,
    pub version: &'static str,
    pub url: &'static str,
    /// Expected sha256 of the *archive* (not the extracted binary).
    /// `None` while we're in the bootstrapping phase — the lockfile
    /// canary catches asset rotation. Pin to a real digest before
    /// shipping a release.
    pub expected_sha256: Option<&'static str>,
    pub archive: ArchiveFormat,
    /// Approximate compressed size in bytes; used as a fallback when
    /// the server omits `Content-Length`. Off by a few percent is
    /// fine — the progress bar falls back to "% known so far".
    pub size_bytes: u64,
    /// Filename of the executable inside the extracted tree. We
    /// `walk_dir` for it rather than hardcoding the full path because
    /// llama.cpp shuffles the layout occasionally between releases.
    pub binary_filename: &'static str,
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const HOST_BINARY: &BinaryDescriptor = &BinaryDescriptor {
    id: "llama-server-b8953-macos-arm64",
    version: LLAMA_CPP_TAG,
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b8953/llama-b8953-bin-macos-arm64.tar.gz",
    // Computed locally from the b8953 release archive on 2026-05-04.
    // Pin: bumping LLAMA_CPP_TAG REQUIRES recomputing this digest.
    expected_sha256: Some("343b6ac2a5bd77b6d9cc6a6af2b298de7228aa3ea9d5056ac0fbbf6b77186ada"),
    archive: ArchiveFormat::TarGz,
    size_bytes: 9_000_000,
    binary_filename: "llama-server",
};

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const HOST_BINARY: &BinaryDescriptor = &BinaryDescriptor {
    id: "llama-server-b8953-win-vulkan-x64",
    version: LLAMA_CPP_TAG,
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b8953/llama-b8953-bin-win-vulkan-x64.zip",
    // Computed locally from the b8953 release archive on 2026-05-04.
    // Pin: bumping LLAMA_CPP_TAG REQUIRES recomputing this digest.
    expected_sha256: Some("af6f9b7a98fd549609db979015bb727fa0048df20d40954678bc2571d514e2e3"),
    archive: ArchiveFormat::Zip,
    size_bytes: 36_000_000,
    binary_filename: "llama-server.exe",
};

// Unsupported host: hide the symbol so callers degrade gracefully.
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
const HOST_BINARY: &BinaryDescriptor = &BinaryDescriptor {
    id: "llama-server-unsupported",
    version: LLAMA_CPP_TAG,
    url: "",
    expected_sha256: None,
    archive: ArchiveFormat::TarGz,
    size_bytes: 0,
    binary_filename: "llama-server",
};

pub fn descriptor_for_host() -> Option<&'static BinaryDescriptor> {
    if HOST_BINARY.url.is_empty() {
        None
    } else {
        Some(HOST_BINARY)
    }
}

/// Public status surface for the Models tab + onboarding. Mirrors
/// `models::ModelStatus` so the frontend can reuse its rendering.
#[derive(Debug, Clone, Serialize)]
pub struct InferenceServerStatus {
    pub id: String,
    pub version: String,
    pub installed: bool,
    pub supported_host: bool,
    pub binary_path: Option<String>,
    pub size_bytes: u64,
}

pub fn status<R: Runtime>(app: &AppHandle<R>) -> InferenceServerStatus {
    let descriptor = match descriptor_for_host() {
        Some(d) => d,
        None => {
            return InferenceServerStatus {
                id: "llama-server-unsupported".into(),
                version: LLAMA_CPP_TAG.into(),
                installed: false,
                supported_host: false,
                binary_path: None,
                size_bytes: 0,
            };
        }
    };
    let installed = is_installed(app);
    let path = if installed { resolved_binary_path(app).map(|p| p.display().to_string()) } else { None };
    InferenceServerStatus {
        id: descriptor.id.to_string(),
        version: descriptor.version.to_string(),
        installed,
        supported_host: true,
        binary_path: path,
        size_bytes: descriptor.size_bytes,
    }
}

/// Root directory under AppData that holds every installed version.
fn root_dir<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    if let Ok(p) = app.path().app_data_dir() {
        return p.join("inference-server");
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.vox.app/inference-server")
}

fn version_dir<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    root_dir(app).join(LLAMA_CPP_TAG)
}

fn extracted_dir<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    version_dir(app).join("extracted")
}

fn sentinel_path<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    version_dir(app).join(".complete")
}

fn binary_path_record<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    version_dir(app).join("binary.path")
}

pub fn is_installed<R: Runtime>(app: &AppHandle<R>) -> bool {
    sentinel_path(app).exists() && resolved_binary_path(app).is_some()
}

/// The absolute path to `llama-server` inside the extracted archive.
/// Reads the `binary.path` sidecar file written at extraction time.
pub fn resolved_binary_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let record = binary_path_record(app);
    let raw = std::fs::read_to_string(&record).ok()?;
    let candidate = PathBuf::from(raw.trim());
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

#[derive(Default)]
struct DownloadGuard {
    in_flight: parking_lot::Mutex<Option<CancellationToken>>,
}

static GUARD: once_cell::sync::Lazy<DownloadGuard> =
    once_cell::sync::Lazy::new(DownloadGuard::default);

/// Cancel an in-flight inference-server download if there is one.
pub fn cancel_download() -> bool {
    if let Some(tok) = GUARD.in_flight.lock().take() {
        tok.cancel();
        true
    } else {
        false
    }
}

/// Download + verify + extract the per-host `llama-server` archive.
///
/// Idempotent: if the sentinel already exists and the binary resolves,
/// emits `binary:done {already_installed:true}` and returns immediately.
/// Concurrent invocations refuse the second call (mirrors
/// `models::download_model` behavior).
pub async fn download_and_extract<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<()> {
    let descriptor = descriptor_for_host()
        .ok_or_else(|| anyhow::anyhow!("no llama-server binary built for this OS/arch"))?;

    if is_installed(app) {
        let _ = app.emit(
            "binary:done",
            serde_json::json!({"id": descriptor.id, "already_installed": true}),
        );
        return Ok(());
    }

    let cancel = {
        let mut guard = GUARD.in_flight.lock();
        if guard.is_some() {
            let _ = app.emit(
                "binary:error",
                serde_json::json!({"id": descriptor.id, "error": "already downloading"}),
            );
            return Err(anyhow::anyhow!("already downloading inference server"));
        }
        let tok = CancellationToken::new();
        *guard = Some(tok.clone());
        tok
    };

    let result = run_download(app, descriptor, cancel.clone()).await;

    // Always release the guard.
    GUARD.in_flight.lock().take();

    match result {
        Ok(()) => {
            let _ = app.emit("binary:done", serde_json::json!({"id": descriptor.id}));
            Ok(())
        }
        Err(e) => {
            if cancel.is_cancelled() {
                let _ = app.emit("binary:canceled", serde_json::json!({"id": descriptor.id}));
                Ok(())
            } else {
                let _ = app.emit(
                    "binary:error",
                    serde_json::json!({"id": descriptor.id, "error": e.to_string()}),
                );
                Err(e)
            }
        }
    }
}

async fn run_download<R: Runtime>(
    app: &AppHandle<R>,
    descriptor: &BinaryDescriptor,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let dir = version_dir(app);
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("create version dir: {e}"))?;
    let archive_path = dir.join(format!("archive.{}", descriptor.archive.extension()));
    let partial_path = dir.join(format!("archive.{}.partial", descriptor.archive.extension()));

    let resume_from = partial_path.metadata().map(|m| m.len()).unwrap_or(0);

    let lock_path = root_dir(app).join("inference.lock");
    models::stream_to_file(
        app,
        StreamArgs {
            id: descriptor.id,
            url: descriptor.url,
            partial_path: &partial_path,
            expected_size: descriptor.size_bytes,
            expected_sha256: descriptor.expected_sha256,
            event_prefix: "binary",
            lock_file: Some(&lock_path),
        },
        resume_from,
        cancel.clone(),
    )
    .await?;

    // Atomic rename: keep the archive on disk for diagnostic re-extraction
    // — it's small enough not to bother cleaning up.
    std::fs::rename(&partial_path, &archive_path)
        .map_err(|e| anyhow::anyhow!("rename archive: {e}"))?;

    // Extract on a blocking thread — tar / zip are sync and the work is
    // CPU-bound on decompression.
    let archive_path_owned = archive_path.clone();
    let extract_root = extracted_dir(app);
    let archive_format = descriptor.archive;
    let extract_handle = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
        if extract_root.exists() {
            // Wipe any stale partial extraction so the walker below
            // doesn't pick up a binary from a half-extracted previous
            // attempt.
            std::fs::remove_dir_all(&extract_root)
                .map_err(|e| anyhow::anyhow!("wipe stale extracted dir: {e}"))?;
        }
        std::fs::create_dir_all(&extract_root)
            .map_err(|e| anyhow::anyhow!("create extracted dir: {e}"))?;
        match archive_format {
            ArchiveFormat::TarGz => extract_tar_gz(&archive_path_owned, &extract_root)?,
            ArchiveFormat::Zip => extract_zip(&archive_path_owned, &extract_root)?,
        }
        Ok(())
    });
    extract_handle
        .await
        .map_err(|e| anyhow::anyhow!("extract task panicked: {e}"))??;

    // Locate the binary inside the extracted tree.
    let binary = walk_for_binary(&extracted_dir(app), descriptor.binary_filename)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "extracted archive does not contain '{}' — archive layout changed?",
                descriptor.binary_filename
            )
        })?;

    // Defensive: clear `com.apple.quarantine` on the binary AND the
    // sibling dylibs. Apple has tightened Gatekeeper enough times
    // that we don't trust `tar -xzf` to keep skipping the bit
    // forever. No-op on Windows.
    #[cfg(unix)]
    {
        strip_quarantine_recursive(&extracted_dir(app));
        ensure_executable(&binary)?;
    }

    // Persist the resolved path so subsequent launches can find it
    // without rewalking the tree.
    std::fs::write(binary_path_record(app), binary.display().to_string())
        .map_err(|e| anyhow::anyhow!("write binary.path: {e}"))?;

    // Sentinel last — its existence implies "everything above
    // succeeded", which is what `is_installed` checks.
    std::fs::write(sentinel_path(app), b"ok\n")
        .map_err(|e| anyhow::anyhow!("write sentinel: {e}"))?;

    Ok(())
}

/// Validates an in-archive relative path against `dest`. Returns the
/// safe joined path on success — `CurDir` (".") components are
/// normalised away so a bare `./` entry returns `dest` itself, which
/// the caller treats as "skip, this is just the archive root". Rejects:
///   - absolute paths (`/etc/passwd`, `C:\Windows\System32\...`)
///   - any `..` or other parent-dir component
///   - empty paths
///
/// `dest` does not need to exist yet, but it must be canonicalised by
/// the caller (we canonicalise inside as a defence-in-depth check).
/// The historical `tar::Archive::unpack` and `zip::ZipArchive::extract`
/// helpers each follow symlinks and normalise lazily — a malicious
/// archive can use either to escape `dest`. We hand-roll iteration
/// here so neither codepath is reachable.
fn safe_join(dest: &Path, relative: &Path) -> anyhow::Result<PathBuf> {
    if relative.as_os_str().is_empty() {
        anyhow::bail!("archive entry has empty path");
    }
    if relative.is_absolute() {
        anyhow::bail!("archive contains absolute path: {}", relative.display());
    }
    let mut joined = dest.to_path_buf();
    for c in relative.components() {
        match c {
            std::path::Component::Normal(part) => joined.push(part),
            // CurDir (".") components are benign — they don't traverse
            // anywhere. The macOS llama.cpp release tarball ships a
            // root "./" entry plus paths like "./bin/llama-server", so
            // we strip CurDir rather than reject the whole archive.
            std::path::Component::CurDir => continue,
            // ParentDir / RootDir / Prefix are all unsafe.
            _ => {
                anyhow::bail!(
                    "archive contains unsafe path component: {}",
                    relative.display()
                );
            }
        }
    }
    // Defence-in-depth: even if components() somehow misses a tricky
    // construction, verify the joined path stays within dest. We don't
    // canonicalise (the file may not exist yet) but we use a literal
    // prefix comparison.
    if !joined.starts_with(dest) {
        anyhow::bail!(
            "archive entry escapes destination directory: {}",
            relative.display()
        );
    }
    Ok(joined)
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let f = std::fs::File::open(archive)
        .map_err(|e| anyhow::anyhow!("open archive: {e}"))?;
    let gz = flate2::read::GzDecoder::new(f);
    let mut tar = tar::Archive::new(gz);
    tar.set_preserve_permissions(true);
    // `tar::Archive::unpack` follows symlinks during extraction and
    // does not validate that entry paths stay under `dest`; a hostile
    // archive can write `../../etc/passwd` or symlink-then-write to
    // escape. Iterate manually so each entry is validated before any
    // file is touched.
    for entry in tar.entries()
        .map_err(|e| anyhow::anyhow!("read tar entries: {e}"))?
    {
        let mut entry = entry.map_err(|e| anyhow::anyhow!("tar entry: {e}"))?;
        let rel: PathBuf = entry
            .path()
            .map_err(|e| anyhow::anyhow!("tar entry path: {e}"))?
            .into_owned();
        let safe = safe_join(dest, &rel)?;
        // `./` (the archive root) normalises to `dest` itself, which
        // the caller already created. Skip it so we don't try to
        // unpack a directory entry on top of the destination.
        if safe == dest {
            continue;
        }
        // Defence-in-depth for symlink entries: reject any link target
        // that resolves outside `dest`.
        if let Ok(Some(link)) = entry.link_name() {
            // `link_name` is the literal target (relative or absolute).
            // We must validate it the same way as the entry path —
            // otherwise an extracted file could pass safe_join while
            // pointing at /etc/passwd.
            let link_path: PathBuf = link.into_owned();
            // Allow only relative, no-parent-dir link targets.
            if let Err(e) = safe_join(dest, &link_path) {
                anyhow::bail!(
                    "tar entry {} has unsafe symlink target {}: {e}",
                    rel.display(),
                    link_path.display()
                );
            }
        }
        // `Entry::unpack` (unlike the higher-level `Archive::unpack`)
        // does NOT create parent directories. The llama.cpp release
        // tarball ships entries like `llama-b8953/llama-mtmd-debug`
        // with no intermediate `llama-b8953/` directory entry, so we
        // must create the parent ourselves. The zip extractor below
        // does the same.
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            std::fs::create_dir_all(&safe)
                .map_err(|e| anyhow::anyhow!("create dir {}: {e}", safe.display()))?;
            continue;
        }
        if let Some(parent) = safe.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("create parent {}: {e}", parent.display()))?;
        }
        entry
            .unpack(&safe)
            .map_err(|e| anyhow::anyhow!("untar entry {}: {e:#}", rel.display()))?;
    }
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let f = std::fs::File::open(archive)
        .map_err(|e| anyhow::anyhow!("open archive: {e}"))?;
    let mut zip =
        zip::ZipArchive::new(f).map_err(|e| anyhow::anyhow!("read zip: {e}"))?;
    // `ZipArchive::extract` does its own zip-slip check in recent
    // versions, but we prefer to be explicit so a future dependency
    // bump can't silently regress. Iterate, validate each path, and
    // write into `dest` ourselves.
    for i in 0..zip.len() {
        let mut zfile = zip
            .by_index(i)
            .map_err(|e| anyhow::anyhow!("read zip entry {i}: {e}"))?;
        // `enclosed_name` returns None for any path containing absolute
        // or parent-dir components — exactly the validation we want.
        let rel = zfile
            .enclosed_name()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "zip entry has unsafe path: {}",
                    zfile.name()
                )
            })?
            .to_path_buf();
        let safe = safe_join(dest, &rel)?;
        // `./` (the archive root) normalises to `dest` itself, which
        // already exists. Skip rather than create-it-again or attempt
        // to write a file at the destination path.
        if safe == dest {
            continue;
        }
        if zfile.is_dir() {
            std::fs::create_dir_all(&safe)
                .map_err(|e| anyhow::anyhow!("create dir {}: {e}", safe.display()))?;
            continue;
        }
        if let Some(parent) = safe.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("create parent {}: {e}", parent.display()))?;
        }
        let mut out = std::fs::File::create(&safe)
            .map_err(|e| anyhow::anyhow!("create file {}: {e}", safe.display()))?;
        std::io::copy(&mut zfile, &mut out)
            .map_err(|e| anyhow::anyhow!("write file {}: {e}", safe.display()))?;
        // Preserve unix permissions if the entry carries them.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if let Some(mode) = zfile.unix_mode() {
                let _ = std::fs::set_permissions(
                    &safe,
                    std::fs::Permissions::from_mode(mode),
                );
            }
        }
    }
    Ok(())
}

/// Recursively search `root` for the first regular file whose final
/// path component matches `name`. Returns `None` if nothing matches.
fn walk_for_binary(root: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ft = match entry.file_type() {
                Ok(f) => f,
                Err(_) => continue,
            };
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file()
                && path.file_name().and_then(|s| s.to_str()) == Some(name)
            {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(unix)]
fn ensure_executable(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut perms = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("stat binary: {e}"))?
        .permissions();
    let mode = perms.mode();
    if mode & 0o111 == 0 {
        perms.set_mode(mode | 0o755);
        std::fs::set_permissions(path, perms)
            .map_err(|e| anyhow::anyhow!("chmod binary: {e}"))?;
    }
    Ok(())
}

#[cfg(unix)]
fn strip_quarantine_recursive(root: &Path) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ft = match entry.file_type() {
                Ok(f) => f,
                Err(_) => continue,
            };
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                // `xattr::remove` returns NoSuchAttribute (ENODATA) if
                // the bit isn't set — which is the common case after
                // tar -xzf. Swallow that.
                let _ = xattr::remove(&path, "com.apple.quarantine");
            }
        }
    }
}

/// Best-effort cleanup of older version trees. Called once per launch
/// from `setup`. Keeps the *current* `LLAMA_CPP_TAG` directory and
/// removes everything else under `inference-server/`.
pub fn cleanup_stale_versions<R: Runtime>(app: &AppHandle<R>) {
    let root = root_dir(app);
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|s| s.to_str()) == Some(LLAMA_CPP_TAG) {
            continue;
        }
        if path.is_dir() {
            if let Err(e) = std::fs::remove_dir_all(&path) {
                log::warn!(
                    "stale inference-server cleanup failed for {}: {e}",
                    path.display()
                );
            } else {
                log::info!("removed stale inference-server: {}", path.display());
            }
        }
    }
}

/// One-shot delete of every installed binary. Used by Settings
/// "Reinstall" and "Uninstall" actions.
pub fn delete<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<()> {
    let dir = root_dir(app);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("delete inference-server dir: {e}"))?;
    }
    Ok(())
}

/// Tiny health probe used by lib.rs setup() to decide whether to
/// pre-warm. Returns immediately with no I/O if the binary isn't
/// installed.
#[allow(dead_code)]
pub async fn probe_ready<R: Runtime>(app: &AppHandle<R>) -> bool {
    if !is_installed(app) {
        return false;
    }
    // 50 ms ceiling — this is just an existence check, not a deep probe.
    tokio::time::timeout(Duration::from_millis(50), async {
        resolved_binary_path(app).is_some()
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_for_host_supported_oses() {
        // The host running the tests is one of the two supported
        // platforms today (macOS arm64 dev or Windows CI). Either way
        // the descriptor must resolve and have a non-empty URL.
        if cfg!(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "windows", target_arch = "x86_64")
        )) {
            let d = descriptor_for_host().expect("supported host has a descriptor");
            assert!(!d.url.is_empty(), "host descriptor must have a URL");
            assert_eq!(d.version, LLAMA_CPP_TAG);
        }
    }

    #[test]
    fn walk_for_binary_finds_nested() {
        // Use a per-test unique directory under the system temp root
        // rather than pulling in `tempfile` as a dev-dependency.
        let unique = format!(
            "vox-walk-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let tmp = std::env::temp_dir().join(unique);
        let nested = tmp.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let target = nested.join("llama-server");
        std::fs::write(&target, b"stub").unwrap();
        let found = walk_for_binary(&tmp, "llama-server");
        assert_eq!(found.as_deref(), Some(target.as_path()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn safe_join_accepts_curdir_components() {
        // The macOS llama.cpp release tarball contains a bare "./" root
        // entry plus paths like "./bin/llama-server". Both must extract
        // cleanly — see the onboarding regression where this surfaced
        // as "archive contains unsafe path component: ./" in red.
        let dest = Path::new("/tmp/vox-dest");
        assert_eq!(safe_join(dest, Path::new("./")).unwrap(), dest);
        assert_eq!(
            safe_join(dest, Path::new("./bin/llama-server")).unwrap(),
            dest.join("bin").join("llama-server")
        );
        assert_eq!(
            safe_join(dest, Path::new("bin/llama-server")).unwrap(),
            dest.join("bin").join("llama-server")
        );
    }

    #[test]
    fn safe_join_rejects_parent_dir_and_absolute() {
        let dest = Path::new("/tmp/vox-dest");
        assert!(safe_join(dest, Path::new("../etc/passwd")).is_err());
        assert!(safe_join(dest, Path::new("bin/../../etc/passwd")).is_err());
        assert!(safe_join(dest, Path::new("/etc/passwd")).is_err());
        assert!(safe_join(dest, Path::new("")).is_err());
    }

    #[test]
    fn extract_tar_gz_creates_missing_parent_dirs() {
        // The llama.cpp release tarball ships file entries like
        // `llama-b8953/llama-mtmd-debug` with NO separate directory
        // entry for `llama-b8953/`. `tar::Entry::unpack` doesn't
        // auto-create parent dirs, so the extractor must.
        let unique = format!(
            "vox-extract-tar-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let work = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&work).unwrap();
        let archive_path = work.join("test.tar.gz");
        let dest = work.join("out");
        std::fs::create_dir_all(&dest).unwrap();

        // Build a tar.gz with a "./" root entry and a deeply-nested
        // file with no intermediate directory entries — exactly the
        // shape that broke onboarding.
        let f = std::fs::File::create(&archive_path).unwrap();
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);

        let mut root_header = tar::Header::new_gnu();
        root_header.set_path("./").unwrap();
        root_header.set_size(0);
        root_header.set_entry_type(tar::EntryType::Directory);
        root_header.set_mode(0o755);
        root_header.set_cksum();
        builder
            .append(&root_header, std::io::empty())
            .unwrap();

        let payload = b"binary contents";
        let mut file_header = tar::Header::new_gnu();
        file_header.set_path("llama-b8953/llama-mtmd-debug").unwrap();
        file_header.set_size(payload.len() as u64);
        file_header.set_mode(0o755);
        file_header.set_entry_type(tar::EntryType::Regular);
        file_header.set_cksum();
        builder
            .append(&file_header, &payload[..])
            .unwrap();

        builder.into_inner().unwrap().finish().unwrap();

        extract_tar_gz(&archive_path, &dest).expect("extraction must succeed");
        let extracted = dest.join("llama-b8953").join("llama-mtmd-debug");
        assert!(extracted.is_file(), "expected {} to exist", extracted.display());
        assert_eq!(std::fs::read(&extracted).unwrap(), payload);

        let _ = std::fs::remove_dir_all(&work);
    }
}
