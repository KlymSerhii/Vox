//! Modifier-only push-to-talk via a macOS `CGEventTap`.
//!
//! Carbon `RegisterEventHotKey` (which `tauri-plugin-global-shortcut` uses)
//! cannot bind a shortcut with no base key, so a chord like `Option+Control`
//! has to come from a `kCGEventFlagsChanged` tap.
//!
//! Design notes worth knowing:
//! - A dedicated thread owns the `CFRunLoop` the tap is attached to. Drop
//!   stops the run loop and joins the thread before freeing state.
//! - We use `kCGEventTapOptionListenOnly` so we never swallow events — the
//!   user's regular `Option+Control+...` app shortcuts keep working.
//! - We require `AXIsProcessTrusted()` at install time. Without
//!   Accessibility the tap creates successfully on some macOS versions but
//!   never receives events; callers fall back to a regular shortcut when we
//!   return `ChordError::PermissionDenied`.
//! - Raw `extern "C"` instead of `core-graphics`/`objc2-core-foundation` to
//!   avoid coupling to a specific semver of those crates.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use parking_lot::Mutex as PMutex;

type CFTypeRef = *const std::ffi::c_void;
type CFAllocatorRef = CFTypeRef;
type CFMachPortRef = CFTypeRef;
type CFRunLoopRef = CFTypeRef;
type CFRunLoopSourceRef = CFTypeRef;
type CFStringRef = CFTypeRef;
type CGEventRef = *const std::ffi::c_void;
type CGEventTapProxy = *const std::ffi::c_void;
type CGEventType = u32;
type CGEventMask = u64;
type CGEventTapLocation = u32;
type CGEventTapPlacement = u32;
type CGEventTapOptions = u32;
type CGEventField = u32;

// Constants from `CGEventTypes.h`.
const K_CG_SESSION_EVENT_TAP: CGEventTapLocation = 1;
const K_CG_HEAD_INSERT_EVENT_TAP: CGEventTapPlacement = 0;
const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: CGEventTapOptions = 1;
const K_CG_EVENT_FLAGS_CHANGED: CGEventType = 12;
const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: CGEventType = 0xFFFF_FFFE;
const K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT: CGEventType = 0xFFFF_FFFF;

pub const FLAG_SHIFT: u64 = 1 << 17;
pub const FLAG_CONTROL: u64 = 1 << 18;
pub const FLAG_ALTERNATE: u64 = 1 << 19;
pub const FLAG_COMMAND: u64 = 1 << 20;
/// Caps Lock / NumPad / Fn bits are deliberately excluded — they
/// shouldn't disarm a chord.
const MOD_MASK: u64 = FLAG_SHIFT | FLAG_CONTROL | FLAG_ALTERNATE | FLAG_COMMAND;

type CGEventTapCallback = extern "C" fn(
    proxy: CGEventTapProxy,
    ty: CGEventType,
    event: CGEventRef,
    user_info: *mut std::ffi::c_void,
) -> CGEventRef;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: CGEventTapLocation,
        place: CGEventTapPlacement,
        options: CGEventTapOptions,
        events_of_interest: CGEventMask,
        callback: CGEventTapCallback,
        user_info: *mut std::ffi::c_void,
    ) -> CFMachPortRef;

    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);

    fn CGEventGetFlags(event: CGEventRef) -> u64;

    #[allow(dead_code)]
    fn CGEventGetIntegerValueField(event: CGEventRef, field: CGEventField) -> i64;

    fn AXIsProcessTrusted() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFMachPortCreateRunLoopSource(
        allocator: CFAllocatorRef,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;

    fn CFMachPortInvalidate(port: CFMachPortRef);

    fn CFRunLoopAddSource(
        rl: CFRunLoopRef,
        source: CFRunLoopSourceRef,
        mode: CFStringRef,
    );

    fn CFRunLoopRemoveSource(
        rl: CFRunLoopRef,
        source: CFRunLoopSourceRef,
        mode: CFStringRef,
    );

    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: CFRunLoopRef);

    fn CFRelease(cf: CFTypeRef);

    static kCFRunLoopCommonModes: CFStringRef;
}

/// Which modifiers must all be held for the chord to fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChordSpec {
    pub need_option: bool,
    pub need_control: bool,
    pub need_command: bool,
    pub need_shift: bool,
    /// `true` = chord fires only on an exact match (no extra modifiers).
    /// `false` = chord fires as soon as the required subset is held.
    pub strict_all_or_none: bool,
}

impl ChordSpec {
    fn required_mask(self) -> u64 {
        [
            (self.need_option, FLAG_ALTERNATE),
            (self.need_control, FLAG_CONTROL),
            (self.need_command, FLAG_COMMAND),
            (self.need_shift, FLAG_SHIFT),
        ]
        .iter()
        .filter(|(b, _)| *b)
        .map(|(_, f)| *f)
        .fold(0, |acc, f| acc | f)
    }

    pub fn is_valid(self) -> bool {
        self.required_mask() != 0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChordError {
    #[error("Accessibility permission denied — chord cannot be installed")]
    PermissionDenied,
    #[error("CGEventTapCreate returned NULL — usually means missing TCC permission")]
    TapCreateFailed,
    #[error("Failed to start the run loop thread for the chord tap")]
    RunLoopFailed,
    #[error("ChordSpec has no required modifiers")]
    EmptySpec,
}

/// Opaque handle. Drop to remove the chord listener.
pub struct ChordHandle {
    inner: *mut Inner,
    thread: Option<JoinHandle<()>>,
}

// SAFETY: `inner` is a `Box::into_raw` pointer never exposed outside the
// impl, and the drop path joins the worker thread before freeing it.
unsafe impl Send for ChordHandle {}
unsafe impl Sync for ChordHandle {}

/// Install a chord listener. Closures fire on the run-loop thread (or a
/// short debounce thread) and must not block long — post a Tauri async
/// task instead.
///
/// `Send + 'static` (not `Sync`) so callers can capture `AppHandle` /
/// `Arc<SessionManager>`, both of which contain non-`Sync` interior
/// state (the audio handle's `mpsc::Sender`).
pub fn install(
    spec: ChordSpec,
    on_press: impl Fn() + Send + 'static,
    on_release: impl Fn() + Send + 'static,
) -> Result<ChordHandle, ChordError> {
    if !spec.is_valid() {
        return Err(ChordError::EmptySpec);
    }
    // The tap creates successfully without Accessibility on some macOS
    // versions but receives no events. Surface the error early so callers
    // can fall back to a regular shortcut.
    let trusted = unsafe { AXIsProcessTrusted() };
    log::error!(
        "mac_chord: AXIsProcessTrusted={trusted} spec={spec:?} required_mask={:#x}",
        spec.required_mask()
    );
    if !trusted {
        return Err(ChordError::PermissionDenied);
    }

    let shared = Arc::new(Shared {
        spec,
        on_press: PMutex::new(Box::new(on_press)),
        on_release: PMutex::new(Box::new(on_release)),
        armed: AtomicBool::new(false),
        last_seq: AtomicU64::new(0),
        log_counter: AtomicUsize::new(0),
        log_window_start_ms: AtomicU64::new(0),
        log_window_count: AtomicU64::new(0),
    });

    // Run-loop / mach port handles are filled in on the worker thread.
    let inner_ptr: *mut Inner = Box::into_raw(Box::new(Inner {
        shared: shared.clone(),
        run_loop: Mutex::new(std::ptr::null()),
        mach_port: Mutex::new(std::ptr::null()),
        run_loop_source: Mutex::new(std::ptr::null()),
        ready_tx: Mutex::new(None),
    }));

    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), ChordError>>();
    // SAFETY: only this function holds `inner_ptr` here.
    if let Ok(mut slot) = unsafe { (*inner_ptr).ready_tx.lock() } {
        *slot = Some(ready_tx);
    }

    // Smuggle the pointer through `usize` to satisfy `Send` without a
    // wrapper type — nobody else touches it until the worker thread takes
    // ownership.
    let inner_addr: usize = inner_ptr as usize;

    let thread = thread::Builder::new()
        .name("vox-chord-tap".into())
        .spawn(move || run_loop_thread(inner_addr as *mut Inner))
        .map_err(|_| ChordError::RunLoopFailed)?;

    let result = ready_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or(Err(ChordError::RunLoopFailed));
    match result {
        Ok(()) => Ok(ChordHandle {
            inner: inner_ptr,
            thread: Some(thread),
        }),
        Err(e) => {
            // The worker thread OWNS the `Box<Inner>` and is the only
            // thread that may free it (see `run_loop_thread`). On any
            // error path we MUST NOT call `Box::from_raw(inner_ptr)`
            // here — doing so would race with the worker still
            // referencing `inner.shared` through `&*inner_ptr` and
            // cause a use-after-free. Joining the worker is enough:
            // its tail will reclaim the box.
            let _ = thread.join();
            Err(e)
        }
    }
}

impl Drop for ChordHandle {
    fn drop(&mut self) {
        // The worker thread owns the `Box<Inner>` allocation and
        // reclaims it at thread exit — see `run_loop_thread`. We must
        // NOT call `Box::from_raw(self.inner)` here.
        //
        // To avoid racing the worker on `&*self.inner` we snapshot the
        // CF refs out of the locked slots, ask the run loop to stop,
        // join, and only THEN release the CF objects. Once the worker
        // returns from `CFRunLoopRun` it frees the box in its tail —
        // but the CF refs we snapshotted are CFRetain'd kernel mach
        // ports unaffected by that free, so we can release them after
        // the join.
        let port_snap;
        let rl_snap;
        let src_snap;
        // SAFETY: worker is still running at this point (we have not
        // joined yet) and it only frees `inner` AFTER its run loop
        // exits; we are about to ask the run loop to exit, so the
        // borrow here is bounded.
        unsafe {
            let inner = &*self.inner;
            port_snap = inner.mach_port.lock().ok().map(|g| *g).unwrap_or(std::ptr::null());
            rl_snap = inner.run_loop.lock().ok().map(|g| *g).unwrap_or(std::ptr::null());
            src_snap = inner.run_loop_source.lock().ok().map(|g| *g).unwrap_or(std::ptr::null());
        }

        unsafe {
            if !port_snap.is_null() {
                CGEventTapEnable(port_snap, false);
            }
            if !rl_snap.is_null() && !src_snap.is_null() {
                CFRunLoopRemoveSource(rl_snap, src_snap, kCFRunLoopCommonModes);
            }
            if !rl_snap.is_null() {
                CFRunLoopStop(rl_snap);
            }
        }

        if let Some(t) = self.thread.take() {
            // After this returns, the worker has freed `self.inner`.
            let _ = t.join();
        }

        // Worker is gone — safe to release CF objects we still hold
        // a reference to.
        unsafe {
            if !port_snap.is_null() {
                CFMachPortInvalidate(port_snap);
            }
            if !src_snap.is_null() {
                CFRelease(src_snap);
            }
            if !port_snap.is_null() {
                CFRelease(port_snap);
            }
        }
    }
}

type Callback = Box<dyn Fn() + Send + 'static>;

/// Held in an `Arc` so debounce threads sleeping at teardown can outlive
/// `Inner` without UAF.
struct Shared {
    spec: ChordSpec,
    on_press: PMutex<Callback>,
    on_release: PMutex<Callback>,
    armed: AtomicBool,
    /// Used to discard a stale debounce timer when modifier state changes.
    last_seq: AtomicU64,
    /// Throttles flagsChanged logs to ~10/sec (per the rolling window below).
    log_counter: AtomicUsize,
    log_window_start_ms: AtomicU64,
    log_window_count: AtomicU64,
}

struct Inner {
    shared: Arc<Shared>,
    run_loop: Mutex<CFRunLoopRef>,
    mach_port: Mutex<CFMachPortRef>,
    run_loop_source: Mutex<CFRunLoopSourceRef>,
    ready_tx: Mutex<Option<std::sync::mpsc::Sender<Result<(), ChordError>>>>,
}

fn run_loop_thread(inner_ptr: *mut Inner) {
    // The worker thread is the sole owner of the `Box<Inner>` allocated
    // in `install`. Every exit path (early failure or run-loop return)
    // funnels through the `Box::from_raw` at the bottom of this
    // function so the install-side caller never has to reclaim it —
    // which is what closes the use-after-free window the previous
    // design had: the install-side `recv_timeout(2s)` could fire while
    // the worker was still holding `&*inner_ptr` past the
    // `CGEventTapCreate` decision.
    let mask: CGEventMask = 1 << K_CG_EVENT_FLAGS_CHANGED;

    log::error!(
        "mac_chord: run-loop thread started (tid={:?}); CGEventTapCreate mask={:#x}",
        thread::current().id(),
        mask,
    );

    let port = unsafe {
        CGEventTapCreate(
            K_CG_SESSION_EVENT_TAP,
            K_CG_HEAD_INSERT_EVENT_TAP,
            K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
            mask,
            tap_callback,
            inner_ptr as *mut std::ffi::c_void,
        )
    };
    if port.is_null() {
        // Most likely Accessibility was revoked between the pre-check and now.
        log::error!("mac_chord: CGEventTapCreate returned NULL — Accessibility revoked?");
        // SAFETY: still the unique reference; install caller never
        // touches `inner_ptr` on its own.
        let inner: &Inner = unsafe { &*inner_ptr };
        notify_ready(inner, Err(ChordError::TapCreateFailed));
        // Reclaim the box at thread exit so the install caller can
        // remain "we never own this".
        unsafe { drop(Box::from_raw(inner_ptr)) };
        return;
    }
    log::error!("mac_chord: CGEventTapCreate ok (port={:p})", port);

    let source = unsafe { CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0) };
    if source.is_null() {
        log::error!("mac_chord: CFMachPortCreateRunLoopSource returned NULL");
        unsafe {
            CGEventTapEnable(port, false);
            CFRelease(port);
        }
        let inner: &Inner = unsafe { &*inner_ptr };
        notify_ready(inner, Err(ChordError::RunLoopFailed));
        unsafe { drop(Box::from_raw(inner_ptr)) };
        return;
    }

    let rl = unsafe { CFRunLoopGetCurrent() };
    unsafe {
        CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
        CGEventTapEnable(port, true);
    }
    log::error!("mac_chord: tap enabled; entering CFRunLoopRun (rl={:p})", rl);

    // Borrow `inner` only inside scopes that don't outlive the run-loop
    // body; the final `Box::from_raw` reclaim at the bottom is the
    // single freer.
    {
        let inner: &Inner = unsafe { &*inner_ptr };
        if let Ok(mut g) = inner.run_loop.lock() {
            *g = rl;
        }
        if let Ok(mut g) = inner.mach_port.lock() {
            *g = port;
        }
        if let Ok(mut g) = inner.run_loop_source.lock() {
            *g = source;
        }

        notify_ready(inner, Ok(()));
    }

    // Park on the run loop until `CFRunLoopStop` from the drop handler.
    unsafe { CFRunLoopRun() };
    log::error!("mac_chord: CFRunLoopRun exited; thread terminating");

    // Worker is the sole freer of `Box<Inner>`. The Drop path joins us
    // before observing `inner`, so reclaiming here is correct on every
    // exit path (see `ChordHandle::drop` and the `install` error arm).
    unsafe { drop(Box::from_raw(inner_ptr)) };
}

fn notify_ready(inner: &Inner, result: Result<(), ChordError>) {
    if let Ok(mut slot) = inner.ready_tx.lock() {
        if let Some(tx) = slot.take() {
            let _ = tx.send(result);
        }
    }
}

extern "C" fn tap_callback(
    _proxy: CGEventTapProxy,
    ty: CGEventType,
    event: CGEventRef,
    user_info: *mut std::ffi::c_void,
) -> CGEventRef {
    if user_info.is_null() {
        return event;
    }
    // SAFETY: `user_info` is the `Box::into_raw(Inner)` from `install`;
    // `ChordHandle::drop` joins this thread before freeing it.
    let inner: &Inner = unsafe { &*(user_info as *const Inner) };

    if ty == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT
        || ty == K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT
    {
        if let Ok(g) = inner.mach_port.lock() {
            if !g.is_null() {
                unsafe { CGEventTapEnable(*g, true) };
                log::error!("mac_chord: tap disabled by macOS ({:#x}); re-enabled", ty);
            }
        }
        return event;
    }

    if ty != K_CG_EVENT_FLAGS_CHANGED {
        return event;
    }

    let raw_flags = unsafe { CGEventGetFlags(event) };
    let flags = raw_flags & MOD_MASK;
    let shared = inner.shared.clone();
    let required = shared.spec.required_mask();
    log_throttled_flags(&shared, raw_flags, flags, required);

    let chord_held = if shared.spec.strict_all_or_none {
        flags == required
    } else {
        flags & required == required
    };
    let seq = shared.last_seq.fetch_add(1, Ordering::SeqCst) + 1;

    match (shared.armed.load(Ordering::SeqCst), chord_held) {
        (false, true) => {
            // 60ms debounce so brief Option/Control taps don't fire. Hand
            // off `Arc<Shared>` — debounce threads may outlive `Inner` but
            // never touch it, so no UAF.
            log::error!(
                "mac_chord: raise (flags={flags:#x} required={required:#x}); debouncing seq={seq}"
            );
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(60));
                if shared.last_seq.load(Ordering::SeqCst) != seq {
                    return;
                }
                if shared.armed.swap(true, Ordering::SeqCst) {
                    return;
                }
                log::error!("mac_chord: debounce fired on_press (seq={seq})");
                (shared.on_press.lock())();
            });
        }
        (true, false) => {
            log::error!(
                "mac_chord: release (flags={flags:#x} required={required:#x}); on_release"
            );
            shared.armed.store(false, Ordering::SeqCst);
            (shared.on_release.lock())();
        }
        _ => {}
    }

    event
}

/// At most ~10 raw-flag log lines per rolling second. Excess is dropped.
fn log_throttled_flags(shared: &Shared, raw: u64, masked: u64, required: u64) {
    static BASELINE: once_cell::sync::OnceCell<Instant> = once_cell::sync::OnceCell::new();
    let now_ms = BASELINE.get_or_init(Instant::now).elapsed().as_millis() as u64;

    let window_start = shared.log_window_start_ms.load(Ordering::Relaxed);
    if now_ms.saturating_sub(window_start) >= 1000 {
        shared.log_window_start_ms.store(now_ms, Ordering::Relaxed);
        shared.log_window_count.store(0, Ordering::Relaxed);
    }
    if shared.log_window_count.fetch_add(1, Ordering::Relaxed) >= 10 {
        return;
    }
    let total = shared.log_counter.fetch_add(1, Ordering::Relaxed) + 1;
    log::error!(
        "mac_chord: flagsChanged #{total} raw={raw:#x} masked={masked:#x} required={required:#x}"
    );
}

/// Interpret a hotkey-id string as a modifier-only chord. `None` if it
/// contains a non-modifier base key.
pub fn try_parse_chord(spec: &str) -> Option<ChordSpec> {
    let s = spec.trim();
    if s.is_empty() {
        return None;
    }
    let mut out = ChordSpec {
        need_option: false,
        need_control: false,
        need_command: false,
        need_shift: false,
        strict_all_or_none: true,
    };
    for tok in s.split('+') {
        match tok.trim() {
            "Alt" | "Option" => out.need_option = true,
            "Ctrl" | "Control" => out.need_control = true,
            "Cmd" | "Command" | "Meta" | "Super" | "CommandOrControl" => {
                out.need_command = true
            }
            "Shift" => out.need_shift = true,
            _ => return None,
        }
    }
    if out.is_valid() { Some(out) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pure_chord() {
        let s = try_parse_chord("Alt+Control").unwrap();
        assert!(s.need_option && s.need_control);
        assert!(!s.need_command && !s.need_shift);
    }

    #[test]
    fn parse_rejects_base_key() {
        assert!(try_parse_chord("Alt+M").is_none());
        assert!(try_parse_chord("F13").is_none());
        assert!(try_parse_chord("CommandOrControl+Shift+Space").is_none());
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(try_parse_chord("").is_none());
        assert!(try_parse_chord("   ").is_none());
    }

    #[test]
    fn parse_alt_shift() {
        let s = try_parse_chord("Alt+Shift").unwrap();
        assert!(s.need_option && s.need_shift);
    }
}
