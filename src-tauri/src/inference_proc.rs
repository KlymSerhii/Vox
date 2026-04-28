//! Spawn / health-poll / shutdown / orphan-reap the out-of-process
//! `llama-server` that performs Gemma 4 polishing.
//!
//! Companion to `inference_server.rs`, which owns the
//! download-and-extract half. This module owns everything that
//! happens AFTER the binary is on disk:
//!
//! 1. `spawn` the binary as a child in its own process group / Job
//!    Object so a parent SIGKILL still tears down the child.
//! 2. Health-poll `GET /health` with backoff until ready.
//! 3. Hand a `RunningServer { port, api_key }` to the engine layer
//!    (`engines::gemma_http`) so it can talk to the server.
//! 4. `ensure_running` is idempotent and self-healing — if the cached
//!    handle is dead (C++ assert in the child, OOM-kill, etc.) the
//!    next call respawns. The engine layer's `cleanup()` calls this
//!    on every request, so a one-off crash never wedges the user.
//! 5. `reap_orphans_at_startup` scans for stale `llama-server`
//!    instances bearing our installation's `--api-key` token and
//!    kills them. macOS has no equivalent of Linux's
//!    `prctl(PR_SET_PDEATHSIG)`, so this is the load-bearing safety
//!    net for "user force-killed Vox last session".
//! 6. `shutdown_managed` is invoked from `lib.rs`'s `RunEvent::Exit`
//!    handler so a clean quit leaves zero `llama-server` processes
//!    behind.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use command_group::AsyncCommandGroup;
use parking_lot::Mutex;
use rand::Rng;
use sysinfo::{ProcessesToUpdate, Signal};
use tauri::{AppHandle, Manager, Runtime};

use crate::inference_server;
use crate::models::{self, ModelKind};
use crate::settings;

/// Listening interface for `llama-server`. Bound as a literal IPv4
/// loopback (NOT `localhost`, NOT `0.0.0.0`) — empirically this is the
/// shape that doesn't trip the Windows Defender Firewall prompt on
/// first launch.
const HOST: &str = "127.0.0.1";

/// Ceiling on health-poll backoff. Cold-loading a 5 GB GGUF on M2
/// takes ~3-6 s; pad generously.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(45);

/// Granularity of the health-poll loop.
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Lightweight info the engine layer needs to make a request. Cheap
/// to clone on every cleanup() call.
#[derive(Debug, Clone)]
pub struct RunningServer {
    pub port: u16,
    pub api_key: String,
    pub model_id: String,
}

/// Owning handle to a live `llama-server` child. Stored inside the
/// app-managed `Mutex<Option<ServerHandle>>`. Dropping the inner
/// `AsyncGroupChild` SIGKILLs the child and tears down its process
/// group — that's what gives us crash-isolation: a panic in the
/// orchestrator drops the supervisor, which kills the child.
pub struct ServerHandle {
    child: command_group::AsyncGroupChild,
    pub port: u16,
    pub api_key: String,
    pub model_id: String,
}

impl ServerHandle {
    fn snapshot(&self) -> RunningServer {
        RunningServer {
            port: self.port,
            api_key: self.api_key.clone(),
            model_id: self.model_id.clone(),
        }
    }

    fn is_alive(&mut self) -> bool {
        // `try_wait` returns Ok(None) while the child is still
        // running; any other shape (Ok(Some(_)) = exited, Err = OS
        // call failed) we treat as "not alive" so the caller respawns.
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// Type alias for the value we stash in `app.manage`. Keep an
/// `Option` rather than a `OnceCell` so a dead child can be replaced
/// in place.
pub type ServerState = Arc<Mutex<Option<ServerHandle>>>;

pub fn install_state<R: Runtime>(app: &AppHandle<R>) {
    app.manage(ServerState::new(Mutex::new(None)));
}

fn state<R: Runtime>(app: &AppHandle<R>) -> tauri::State<'_, ServerState> {
    app.state::<ServerState>()
}

/// Idempotent get-or-spawn. Returns a cheap `RunningServer` snapshot
/// the engine can use to make HTTP calls. If the cached handle is
/// dead OR was started with a different model, this transparently
/// respawns.
pub async fn ensure_running<R: Runtime>(
    app: &AppHandle<R>,
    model_id: &str,
) -> anyhow::Result<RunningServer> {
    // Phase 1: under the lock, decide whether we can hand back an
    // existing handle, or whether we need to drop the existing one
    // and spawn fresh. Don't await across the lock — `parking_lot`
    // mutexes aren't async-aware and a stuck future would deadlock
    // every other invocation.
    //
    // Pattern guards bind names immutably even when matching
    // through `as_mut()` — and `is_alive` needs `&mut self` for the
    // underlying `try_wait`. So we drop into the if-let body to do
    // the mutable work, then dispatch off the result.
    let action = {
        let st = state(app);
        let mut guard = st.lock();
        let reuse_snapshot = if let Some(h) = guard.as_mut() {
            if h.is_alive() && h.model_id == model_id {
                Some(h.snapshot())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(snap) = reuse_snapshot {
            ReuseAction::Reuse(snap)
        } else {
            // Drop the existing handle so the child gets killed
            // before we spawn a replacement.
            guard.take();
            ReuseAction::Spawn
        }
    };

    match action {
        ReuseAction::Reuse(snapshot) => {
            log::debug!(
                "inference: reusing live llama-server port={} model={}",
                snapshot.port,
                snapshot.model_id
            );
            Ok(snapshot)
        }
        ReuseAction::Spawn => {
            let handle = spawn_for_model(app, model_id).await?;
            let snapshot = handle.snapshot();
            state(app).lock().replace(handle);
            Ok(snapshot)
        }
    }
}

enum ReuseAction {
    Reuse(RunningServer),
    Spawn,
}

async fn spawn_for_model<R: Runtime>(
    app: &AppHandle<R>,
    model_id: &str,
) -> anyhow::Result<ServerHandle> {
    let model_path = models::cached_model_path(app, model_id, ModelKind::Llm)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "model '{model_id}' not downloaded — open Settings → Models"
            )
        })?;
    let binary = inference_server::resolved_binary_path(app).ok_or_else(|| {
        anyhow::anyhow!(
            "inference engine not installed — re-run onboarding from Settings → About"
        )
    })?;

    spawn(app, &binary, &model_path, model_id).await
}

/// Pick an ephemeral TCP port by binding `127.0.0.1:0` and reading
/// the OS-assigned port back. Drop the listener so `llama-server`
/// can grab the same port. There is a microsecond-scale TOCTOU
/// window where another process could snipe the port; the caller
/// retries on bind failure (in practice this never happens).
fn pick_ephemeral_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind((HOST, 0))
        .map_err(|e| anyhow::anyhow!("ephemeral port bind: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| anyhow::anyhow!("local_addr: {e}"))?
        .port();
    drop(listener);
    Ok(port)
}

async fn spawn<R: Runtime>(
    app: &AppHandle<R>,
    binary: &Path,
    model_path: &Path,
    model_id: &str,
) -> anyhow::Result<ServerHandle> {
    let api_key = persistent_api_key(app);
    let port = pick_ephemeral_port()?;

    let mut cmd = tokio::process::Command::new(binary);
    cmd.arg("--host")
        .arg(HOST)
        .arg("--port")
        .arg(port.to_string())
        .arg("-m")
        .arg(model_path)
        // `-ngl 99` offloads every layer that fits to the GPU; on
        // Apple Silicon Metal that's "all of them". On Windows
        // Vulkan / CPU builds llama.cpp clamps to what's available,
        // so the high value is safe across hosts.
        .arg("-ngl")
        .arg("99")
        // Context window must fit a 30-minute dictation:
        //   ~150 wpm × 30 min × ~1.5 tok/word ≈ 6.7k input tokens
        //   + ~7k output tokens (cleanup is ≤ input + structural
        //   markup the prompt asks for) + chat-template overhead
        //   ≈ 14k tokens worst case. 16k gives a comfortable
        //   margin.
        //
        // KV cache cost on Gemma 4 E4B at fp16: ~5 GB at 16k.
        // Combined with the ~5 GB model weights this peaks around
        // 11 GB — fits easily on the 16 GB+ machines the
        // recommender targets for E4B. E2B's per-layer KV is
        // smaller, so the same flag is safe on the 12 GB tier.
        .arg("-c")
        .arg("16384")
        // `--jinja` enables the Gemma `<start_of_turn>` chat template
        // baked into the GGUF metadata, so we can speak the OpenAI
        // chat-completions shape without hand-rolling the wrapper.
        .arg("--jinja")
        // Web UI is dead weight for a desktop sidecar.
        .arg("--no-webui")
        .arg("--api-key")
        .arg(&api_key);

    // Pipe stdout/stderr so a llama.cpp diagnostic line lands in our
    // log instead of the user's tray icon getting silently broken.
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null());

    // `group_spawn()` puts the child in its own process group on
    // Unix and assigns it to a Job Object on Windows with the
    // KILL_ON_JOB_CLOSE flag — so a kernel-level guarantee prevents
    // the child from outliving us. `kill_on_drop` (Tokio's vanilla
    // mechanism) only fires on graceful Drop and would leak a child
    // if the parent crashed.
    let mut child = cmd
        .group_spawn()
        .map_err(|e| anyhow::anyhow!("spawn llama-server: {e}"))?;

    // Drain stdout/stderr in the background so they don't block on
    // the OS pipe buffer when llama-server is chatty.
    drain_pipe(child.inner().stdout.take(), "stdout");
    drain_pipe(child.inner().stderr.take(), "stderr");

    let mut handle = ServerHandle {
        child,
        port,
        api_key,
        model_id: model_id.to_string(),
    };

    // Wait for /health to come up. If the child exits before then
    // (bad model, missing dylib, etc.) the timeout fires and the
    // handle gets dropped on return, which SIGKILLs the group.
    if let Err(e) = wait_for_healthy(&mut handle).await {
        if let Ok(Some(status)) = handle.child.try_wait() {
            return Err(anyhow::anyhow!(
                "llama-server exited before becoming healthy ({status}): {e}"
            ));
        }
        return Err(e);
    }

    log::info!(
        "inference server up: pid={:?} port={} model={}",
        handle.child.id(),
        handle.port,
        handle.model_id
    );
    Ok(handle)
}

fn drain_pipe<P>(pipe: Option<P>, label: &'static str)
where
    P: tokio::io::AsyncRead + Send + Unpin + 'static,
{
    let Some(p) = pipe else { return };
    tauri::async_runtime::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut lines = BufReader::new(p).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => log::debug!("[llama-server {label}] {line}"),
                Ok(None) => break,
                Err(e) => {
                    log::debug!("[llama-server {label}] read err: {e}");
                    break;
                }
            }
        }
    });
}

async fn wait_for_healthy(handle: &mut ServerHandle) -> anyhow::Result<()> {
    let url = format!("http://{HOST}:{}/health", handle.port);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| anyhow::anyhow!("build health client: {e}"))?;

    let deadline = Instant::now() + HEALTH_TIMEOUT;
    while Instant::now() < deadline {
        // Bail early if the child died — no point hammering /health
        // for 45 s when the process exited 200 ms ago.
        if !handle.is_alive() {
            return Err(anyhow::anyhow!("llama-server exited during health probe"));
        }
        let req = client
            .get(&url)
            .bearer_auth(&handle.api_key)
            .send()
            .await;
        if let Ok(resp) = req {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
    }
    Err(anyhow::anyhow!(
        "llama-server health check timed out after {:?}",
        HEALTH_TIMEOUT
    ))
}

/// Read the persistent api_key from settings, or generate + persist a
/// fresh one on first use. The same key is used across every spawn
/// for this installation — that lets the orphan reaper kill stale
/// processes from a previous (force-killed) parent without needing
/// to rebuild the mapping.
fn persistent_api_key<R: Runtime>(app: &AppHandle<R>) -> String {
    let s = settings::load(app);
    if !s.inference_api_key.is_empty() {
        return s.inference_api_key;
    }

    let key = generate_api_key();
    let mut next = s;
    next.inference_api_key = key.clone();
    if let Err(e) = settings::save(app, &next) {
        log::warn!("persist inference_api_key: {e}");
    }
    key
}

fn generate_api_key() -> String {
    use rand::distributions::Alphanumeric;
    let mut rng = rand::thread_rng();
    (0..32).map(|_| rng.sample(Alphanumeric) as char).collect()
}

/// Best-effort: at every launch, scan running processes and kill any
/// `llama-server` whose `--api-key` argument matches our installation
/// token. This catches the macOS gap where Linux's
/// `PR_SET_PDEATHSIG` would auto-kill orphans — there is no
/// equivalent on macOS so a force-killed parent leaves the child
/// running forever.
pub fn reap_orphans_at_startup<R: Runtime>(app: &AppHandle<R>) {
    let s = settings::load(app);
    if s.inference_api_key.is_empty() {
        return; // never spawned anything yet
    }
    let needle = s.inference_api_key.as_str();

    let mut sys = sysinfo::System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut killed = 0usize;
    for (pid, proc_) in sys.processes() {
        // Cheap pre-filter on the executable name to avoid string
        // matching every cmdline on the box.
        let name = proc_.name().to_string_lossy();
        if !name.contains("llama-server") {
            continue;
        }
        let cmd_has_token = proc_
            .cmd()
            .iter()
            .any(|arg| arg.to_string_lossy().contains(needle));
        if !cmd_has_token {
            continue;
        }
        // Don't kill ourselves — sysinfo's pid type is platform-specific.
        if pid.as_u32() == std::process::id() {
            continue;
        }
        // SIGTERM first; if a stuck child needs SIGKILL, the OS will
        // hand it out on the next cycle when its parent dies.
        if proc_.kill_with(Signal::Term).unwrap_or(false) {
            log::info!(
                "reaped orphan llama-server pid={} cmd={:?}",
                pid.as_u32(),
                proc_.cmd()
            );
            killed += 1;
        }
    }
    if killed == 0 {
        log::debug!("orphan reaper: no stale llama-server instances found");
    } else {
        log::info!("orphan reaper: killed {killed} stale llama-server instance(s)");
    }
}

/// Tear down the managed server (if any). Called from `RunEvent::Exit`
/// in `lib.rs::run`. `block_on` is acceptable here — we are already on
/// the way out, and the kill is fast.
pub fn shutdown_managed<R: Runtime>(app: &AppHandle<R>) {
    let st = match app.try_state::<ServerState>() {
        Some(s) => s,
        None => return,
    };
    let handle = st.lock().take();
    let Some(mut h) = handle else { return };
    log::info!("shutting down inference server pid={:?}", h.child.id());
    // `start_kill` is the synchronous fire-and-forget signal:
    // SIGKILL on Unix (delivered to the process group) and Job
    // Object close on Windows (terminates everything in the group).
    // The async `kill` would await; we're on the way out and don't
    // want to block the run loop.
    let _ = h.child.start_kill();
    // Best-effort reap so we don't leave a zombie. A 2 s ceiling
    // keeps quit responsive even if the child is somehow stuck.
    let _ = tauri::async_runtime::block_on(async {
        tokio::time::timeout(Duration::from_secs(2), h.child.wait()).await
    });
}

/// Force a respawn on the next polish call. Used by the
/// `restart_inference_server` IPC command exposed in the Settings →
/// Models tab.
pub fn drop_managed<R: Runtime>(app: &AppHandle<R>) {
    let st = match app.try_state::<ServerState>() {
        Some(s) => s,
        None => return,
    };
    // Bind the take separately so the temporary `MutexGuard` drops
    // before `st` does — otherwise the borrow checker rightly
    // complains that the guard outlives `st`.
    let taken = st.lock().take();
    if let Some(mut h) = taken {
        let _ = h.child.start_kill();
    }
}
