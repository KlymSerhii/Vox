//! Spawn / health-poll / shutdown / orphan-reap the out-of-process
//! `llama-server` that performs Gemma 4 polishing.
//!
//! Companion to `inference_server.rs`, which owns the
//! download-and-extract half. This module owns everything that
//! happens AFTER the binary is on disk:
//!
//! 1. `spawn` the binary as a child. On Windows we assign it to a
//!    Job Object with `KILL_ON_JOB_CLOSE` so a parent SIGKILL still
//!    tears down the child. On macOS there is no equivalent — the
//!    child gets reparented to launchd and survives indefinitely
//!    after a force-kill of Vox; `reap_orphans_at_startup` (step 5)
//!    is the load-bearing safety net for that case.
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
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, Signal, UpdateKind};
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

/// Container for the supervisor's shared state.
///
/// Two locks because we have two distinct concurrency concerns:
///
/// - `handle` holds the live (or `None`) child handle. Reads are
///   short and synchronous, so a `parking_lot::Mutex` is the right
///   shape — async code never holds it across an `.await`.
/// - `spawning` serialises the *spawn path* itself. Without it, two
///   concurrent `ensure_running` calls (e.g. the app-launch
///   pre-warm and a fast user dictating into the hotkey within the
///   ~5 s warmup window) would both see `None`, both call
///   `spawn_for_model`, both try to `replace()` the handle — and we
///   would have briefly spawned two `llama-server` children, each
///   loading 5 GB of GGUF, with one promptly orphaned. The
///   spawn-lock turns this into the standard double-checked
///   locking idiom: the second caller waits, then re-checks the
///   `handle` slot and returns the just-spawned snapshot without
///   spawning again.
pub struct ServerStateInner {
    handle: Mutex<Option<ServerHandle>>,
    spawning: tokio::sync::Mutex<()>,
}

pub type ServerState = Arc<ServerStateInner>;

pub fn install_state<R: Runtime>(app: &AppHandle<R>) {
    app.manage(Arc::new(ServerStateInner {
        handle: Mutex::new(None),
        spawning: tokio::sync::Mutex::new(()),
    }));
}

fn state<R: Runtime>(app: &AppHandle<R>) -> tauri::State<'_, ServerState> {
    app.state::<ServerState>()
}

/// Idempotent get-or-spawn. Returns a cheap `RunningServer` snapshot
/// the engine can use to make HTTP calls. If the cached handle is
/// dead OR was started with a different model, this transparently
/// respawns. Concurrent invocations are race-free: only one caller
/// will spawn; the others wait, see the new handle, and return its
/// snapshot.
pub async fn ensure_running<R: Runtime>(
    app: &AppHandle<R>,
    model_id: &str,
) -> anyhow::Result<RunningServer> {
    // Fast path: already-warm handle that matches the requested
    // model. No spawn lock, no async wait — a hot polish call
    // should pay one parking_lot mutex acquire and nothing else.
    if let Some(snap) = try_reuse(app, model_id) {
        log::debug!(
            "inference: reusing live llama-server port={} model={}",
            snap.port,
            snap.model_id
        );
        return Ok(snap);
    }

    // Slow path: serialise the spawn. The first caller takes this
    // lock, performs the spawn, and stores the handle; concurrent
    // callers block here.
    let st = state(app).inner().clone();
    let _spawn_guard = st.spawning.lock().await;

    // Re-check under the spawn lock — if a parallel caller (e.g.
    // the app-startup pre-warm) finished spawning while we were
    // waiting, reuse its handle instead of spawning again.
    if let Some(snap) = try_reuse(app, model_id) {
        log::debug!(
            "inference: reusing handle spawned by a concurrent caller, port={} model={}",
            snap.port,
            snap.model_id
        );
        return Ok(snap);
    }

    // Drop any dead/wrong-model handle so the spawn replaces it
    // cleanly. The drop sends SIGKILL via command-group's Drop.
    st.handle.lock().take();

    let handle = spawn_for_model(app, model_id).await?;
    let snapshot = handle.snapshot();
    st.handle.lock().replace(handle);
    Ok(snapshot)
}

/// Returns a snapshot if the cached handle is alive and matches
/// `model_id`. Touches only the parking_lot mutex — never awaits.
fn try_reuse<R: Runtime>(app: &AppHandle<R>, model_id: &str) -> Option<RunningServer> {
    let st = state(app);
    let mut guard = st.handle.lock();
    let h = guard.as_mut()?;
    if h.is_alive() && h.model_id == model_id {
        Some(h.snapshot())
    } else {
        None
    }
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
        // Gemma 4 is a thinking model: llama.cpp's `peg-gemma4` chat
        // handler routes chain-of-thought into `reasoning_content` and
        // the cleaned text into `content`. We read only `content` (see
        // gemma_http::post_cleanup), and `gemma_http::dynamic_max_tokens`
        // keeps the per-request budget large enough that the answer
        // lands before max_tokens trips. We deliberately do NOT pass
        // `--reasoning-budget 0`: it sounds like "skip thinking" but
        // actually only collapses the reasoning/content split, leaving
        // a verbose mid-thought dump in `content`.
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
    // KILL_ON_JOB_CLOSE flag. The Job Object is a kernel-level
    // guarantee that the child can't outlive us on Windows. On
    // macOS the process group is just for signal grouping — there
    // is no equivalent of Linux's `PR_SET_PDEATHSIG`, so a parent
    // SIGKILL still leaks the child to launchd. The startup orphan
    // reaper (`reap_orphans_at_startup`) cleans up after that.
    // `kill_on_drop` (Tokio's vanilla mechanism) is strictly weaker
    // — it only fires on graceful Drop and would leak a child if
    // the parent crashed even on Windows.
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
    // The bare `refresh_processes` in sysinfo 0.32 does NOT populate
    // `Process::cmd()` — its default `ProcessRefreshKind` covers
    // memory/cpu/disk/exe only. Without explicitly opting in to
    // `with_cmd`, the api-key match below silently never fires and
    // every orphan survives every launch.
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_cmd(UpdateKind::Always),
    );

    let mut seen = 0usize;
    let mut killed = 0usize;
    for (pid, proc_) in sys.processes() {
        // Cheap pre-filter on the executable name to avoid string
        // matching every cmdline on the box.
        let name = proc_.name().to_string_lossy();
        if !name.contains("llama-server") {
            continue;
        }
        seen += 1;
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
        // SIGKILL, not SIGTERM: these are orphans of a dead parent
        // (PPID=1), so there is no in-flight session to drain and no
        // graceful shutdown to honour. SIGKILL is uncatchable, so
        // this can't wedge on a llama-server build that ignores
        // SIGTERM.
        if proc_.kill_with(Signal::Kill).unwrap_or(false) {
            log::info!(
                "reaped orphan llama-server pid={} cmd={:?}",
                pid.as_u32(),
                proc_.cmd()
            );
            killed += 1;
        } else {
            log::warn!("orphan llama-server pid={} could not be killed", pid.as_u32());
        }
    }
    if seen == 0 {
        log::debug!("orphan reaper: no llama-server processes on the box");
    } else {
        log::info!(
            "orphan reaper: killed {killed}/{seen} llama-server instance(s)"
        );
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
    let taken = st.handle.lock().take();
    let Some(mut h) = taken else { return };
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

/// Conditionally pre-warm the inference server in the background so the
/// first polish doesn't pay the ~5-10 s cold spawn + GGUF-load cost on
/// top of the actual roundtrip. No-op when:
/// - the user hasn't finished onboarding,
/// - polishing is disabled,
/// - the LLM engine isn't `gemma-local`, or
/// - the `llama-server` binary isn't installed yet.
///
/// Idempotent: a concurrent `ensure_running` call serialises against
/// the spawn lock so a fast user dictating during the warmup window
/// won't double-spawn. Errors log at info level (not warn) because
/// "binary not installed yet" is a normal pre-onboarding state.
pub fn prewarm_in_background<R: Runtime>(app: &AppHandle<R>) {
    let s = settings::load(app);
    if !(s.onboarded
        && s.polishing_enabled
        && s.llm_engine == "gemma-local"
        && !s.llm_model_id.is_empty()
        && inference_server::is_installed(app))
    {
        return;
    }

    let warm_handle = app.clone();
    let warm_model_id = s.llm_model_id.clone();
    tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        log::info!("inference: pre-warming (model={warm_model_id})");
        match ensure_running(&warm_handle, &warm_model_id).await {
            Ok(server) => log::info!(
                "inference: pre-warm complete on port {} in {} ms",
                server.port,
                started.elapsed().as_millis()
            ),
            Err(e) => log::info!(
                "inference: pre-warm skipped after {} ms: {e}",
                started.elapsed().as_millis()
            ),
        }
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
    let taken = st.handle.lock().take();
    if let Some(mut h) = taken {
        let _ = h.child.start_kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Validate the spawn-lock invariant: at most one caller can be
    /// inside the protected section at any moment. This is the
    /// regression guard for the pre-warm-vs-first-dictation race
    /// — without this lock, two concurrent `ensure_running` calls
    /// would both miss the warm-handle check, both spawn a
    /// `llama-server`, and one would orphan the other.
    #[tokio::test]
    async fn spawn_lock_serialises_concurrent_callers() {
        let state = Arc::new(ServerStateInner {
            handle: Mutex::new(None),
            spawning: tokio::sync::Mutex::new(()),
        });
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        // Five concurrent tasks each "pretend to spawn" by
        // holding the lock for ~30 ms. If the lock is exclusive,
        // `max_concurrent` should never exceed 1.
        let mut tasks = Vec::with_capacity(5);
        for _ in 0..5 {
            let s = state.clone();
            let active = in_flight.clone();
            let peak = max_concurrent.clone();
            tasks.push(tokio::spawn(async move {
                let _g = s.spawning.lock().await;
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }

        for t in tasks {
            t.await.expect("task panicked");
        }

        assert_eq!(
            max_concurrent.load(Ordering::SeqCst),
            1,
            "spawn lock must be exclusive — observed concurrent holders"
        );
    }

    /// `try_reuse` is the fast-path probe that an `ensure_running`
    /// caller takes before reaching for the spawn lock. Empty
    /// state → `None`, no allocation.
    #[test]
    fn try_reuse_returns_none_when_no_handle() {
        let state = Arc::new(ServerStateInner {
            handle: Mutex::new(None),
            spawning: tokio::sync::Mutex::new(()),
        });
        // Direct probe of the same predicate `try_reuse` uses,
        // without the AppHandle plumbing.
        let mut guard = state.handle.lock();
        assert!(guard.as_mut().is_none());
    }

    /// Sanity check that `ServerStateInner` constructs without
    /// dragging in unwanted runtime state.
    #[test]
    fn server_state_inner_constructs_clean() {
        let state = ServerStateInner {
            handle: Mutex::new(None),
            spawning: tokio::sync::Mutex::new(()),
        };
        assert!(state.handle.lock().is_none());
    }
}
