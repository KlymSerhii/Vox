//! Modifier-only push-to-talk via a macOS `CGEventTap`.
//!
//! `tauri-plugin-global-shortcut` (which goes through Carbon
//! `RegisterEventHotKey` on macOS) cannot bind a shortcut that has only
//! modifier keys and no base key — the registration silently fails. The
//! whispr-flow UX (hold `Option+Control` to dictate) requires exactly
//! that, so for the push-to-talk hotkey we install a session-level
//! `CGEventTap` that listens for `kCGEventFlagsChanged` and synthesises
//! press / release callbacks.
//!
//! ## Design
//!
//! 1. A dedicated thread owns the `CFRunLoop` the tap is attached to.
//!    Tearing the chord down means stopping the run loop and joining
//!    the thread.
//! 2. The C trampoline receives a `*mut Inner` user-info pointer, which
//!    points at a `Box`-allocated state record we keep alive until
//!    `ChordHandle` is dropped.
//! 3. The user closures are wrapped in `Arc<dyn Fn() + Send + Sync +
//!    'static>` so they can be cloned into the trampoline thread and
//!    called without `&mut` aliasing.
//! 4. We use `kCGEventTapOptionListenOnly` so we never swallow events —
//!    the user's normal `Option+Control+...` shortcuts in apps still
//!    work.
//! 5. We require `AXIsProcessTrusted()` at install time. Without
//!    Accessibility the tap silently receives nothing, so callers should
//!    fall back to a regular shortcut when we return
//!    `ChordError::PermissionDenied`.
//!
//! All FFI is done with raw `extern "C"` blocks rather than pulling in
//! `core-graphics` to avoid coupling to a specific semver of that crate
//! (we'd need to feature-gate `objc2-core-foundation` either way). The
//! code mirrors the FFI style already used by `mac_perm.rs`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use parking_lot::Mutex as PMutex;

// ---------- FFI bindings ---------------------------------------------------

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

const K_CG_SESSION_EVENT_TAP: CGEventTapLocation = 1;
const K_CG_HEAD_INSERT_EVENT_TAP: CGEventTapPlacement = 0;
const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: CGEventTapOptions = 1;

/// `kCGEventFlagsChanged` from `CGEventTypes.h`.
const K_CG_EVENT_FLAGS_CHANGED: CGEventType = 12;
/// `kCGEventTapDisabledByTimeout`.
const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: CGEventType = 0xFFFF_FFFE;
/// `kCGEventTapDisabledByUserInput`.
const K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT: CGEventType = 0xFFFF_FFFF;

// `CGEventFlags` bits we care about.
pub const FLAG_SHIFT: u64 = 1 << 17;
pub const FLAG_CONTROL: u64 = 1 << 18;
pub const FLAG_ALTERNATE: u64 = 1 << 19;
pub const FLAG_COMMAND: u64 = 1 << 20;
/// Mask of "real" modifier bits we evaluate. Other bits in
/// `CGEventFlags` (Caps Lock, NumPad, function keys, secondary fn) are
/// intentionally ignored — they shouldn't disarm a chord.
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

// ---------- Public API -----------------------------------------------------

/// Which modifiers must all be held for the chord to fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChordSpec {
    pub need_option: bool,
    pub need_control: bool,
    pub need_command: bool,
    pub need_shift: bool,
    /// When `true` the chord arms ONLY when exactly the requested set of
    /// modifiers is held (no extras). When `false` the chord arms as
    /// soon as the requested subset is held — a less strict mode useful
    /// when the user intentionally combines the chord with another
    /// modifier.
    pub strict_all_or_none: bool,
}

impl ChordSpec {
    fn required_mask(self) -> u64 {
        let mut m = 0;
        if self.need_option {
            m |= FLAG_ALTERNATE;
        }
        if self.need_control {
            m |= FLAG_CONTROL;
        }
        if self.need_command {
            m |= FLAG_COMMAND;
        }
        if self.need_shift {
            m |= FLAG_SHIFT;
        }
        m
    }

    /// True when at least one modifier is required. A spec with no
    /// modifiers is meaningless and should be rejected by the caller.
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

// SAFETY: `inner` is a `Box::into_raw` pointer and we never expose it
// outside the impl. The thread we spawn owns its own copy of the Arc'd
// closures. The drop path joins the thread before freeing `inner`.
unsafe impl Send for ChordHandle {}
unsafe impl Sync for ChordHandle {}

/// Install a chord listener.
///
/// The `on_press` and `on_release` closures are invoked from the dedicated
/// run-loop thread (or a short-lived debounce thread). They MUST not
/// block long — typically they should post a Tauri async task.
///
/// Bounds note: we ask for `Send + 'static` (not `Sync`) so callers can
/// pass closures that capture `tauri::AppHandle` and `SessionCtx`, both
/// of which are `Send` but not necessarily `Sync` because of internal
/// `mpsc::Sender` fields. We serialise calls behind a mutex internally.
pub fn install(
    spec: ChordSpec,
    on_press: impl Fn() + Send + 'static,
    on_release: impl Fn() + Send + 'static,
) -> Result<ChordHandle, ChordError> {
    if !spec.is_valid() {
        return Err(ChordError::EmptySpec);
    }
    // Bail early if Accessibility is off — the tap will create
    // successfully on some macOS versions even without it, but never
    // receive events. Better to surface the error so the caller can
    // fall back to a regular shortcut.
    let trusted = unsafe { AXIsProcessTrusted() };
    if !trusted {
        return Err(ChordError::PermissionDenied);
    }

    let shared = Arc::new(Shared {
        spec,
        on_press: PMutex::new(Box::new(on_press)),
        on_release: PMutex::new(Box::new(on_release)),
        armed: AtomicBool::new(false),
        last_seq: AtomicU64::new(0),
    });

    let inner = Box::new(Inner {
        shared: shared.clone(),
        // The run-loop / mach port handles are filled in on the worker
        // thread (you can't sensibly create them off-thread).
        run_loop: Mutex::new(std::ptr::null()),
        mach_port: Mutex::new(std::ptr::null()),
        run_loop_source: Mutex::new(std::ptr::null()),
        ready_tx: Mutex::new(None),
    });
    let inner_ptr: *mut Inner = Box::into_raw(inner);

    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), ChordError>>();
    {
        // SAFETY: only this function holds `inner_ptr` until we hand it
        // to the worker thread or free it on the error path below.
        let inner_ref: &Inner = unsafe { &*inner_ptr };
        if let Ok(mut slot) = inner_ref.ready_tx.lock() {
            *slot = Some(ready_tx);
        }
    }

    // Smuggle the raw pointer through a `usize` so the closure's
    // `Send` bound is satisfied without needing a custom Send wrapper.
    // Sound because nobody else touches `inner_ptr` between here and
    // the worker thread taking sole ownership.
    let inner_addr: usize = inner_ptr as usize;

    let thread = thread::Builder::new()
        .name("vox-chord-tap".into())
        .spawn(move || {
            let p = inner_addr as *mut Inner;
            run_loop_thread(p);
        })
        .map_err(|_| ChordError::RunLoopFailed)?;

    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(())) => Ok(ChordHandle {
            inner: inner_ptr,
            thread: Some(thread),
        }),
        Ok(Err(e)) => {
            // Worker reported failure and exited; just join + free.
            let _ = thread.join();
            // SAFETY: nobody else has the pointer; reclaim the box.
            unsafe {
                drop(Box::from_raw(inner_ptr));
            }
            Err(e)
        }
        Err(_) => {
            // Timed out — assume tap creation failed silently. Try to
            // wake the thread by stopping any run loop it might have
            // associated, then join and free.
            let _ = thread.join();
            unsafe {
                drop(Box::from_raw(inner_ptr));
            }
            Err(ChordError::RunLoopFailed)
        }
    }
}

impl Drop for ChordHandle {
    fn drop(&mut self) {
        // SAFETY: we own `inner` until this drop completes. We must
        // tear down the run loop FIRST so the worker thread exits,
        // then join it, then free the box.
        unsafe {
            let inner = &*self.inner;
            // Disable the tap, remove the source from the run loop,
            // stop the run loop. All of these are NULL-safe because
            // CFRelease/CFRunLoopRemoveSource/CFRunLoopStop tolerate
            // a NULL we wrap below.
            if let Ok(port_guard) = inner.mach_port.lock() {
                let port = *port_guard;
                if !port.is_null() {
                    CGEventTapEnable(port, false);
                }
            }
            if let (Ok(rl_guard), Ok(src_guard)) =
                (inner.run_loop.lock(), inner.run_loop_source.lock())
            {
                let rl = *rl_guard;
                let src = *src_guard;
                if !rl.is_null() && !src.is_null() {
                    CFRunLoopRemoveSource(rl, src, kCFRunLoopCommonModes);
                }
                if !rl.is_null() {
                    CFRunLoopStop(rl);
                }
            }
        }

        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }

        unsafe {
            let inner = &*self.inner;
            // CFRelease the source + mach port now the thread is gone.
            // Invalidate the mach port first so any in-flight callback
            // sees a no-op rather than touching `inner` after free.
            if let Ok(port_guard) = inner.mach_port.lock() {
                let port = *port_guard;
                if !port.is_null() {
                    CFMachPortInvalidate(port);
                }
            }
            if let Ok(src_guard) = inner.run_loop_source.lock() {
                let src = *src_guard;
                if !src.is_null() {
                    CFRelease(src);
                }
            }
            if let Ok(port_guard) = inner.mach_port.lock() {
                let port = *port_guard;
                if !port.is_null() {
                    CFRelease(port);
                }
            }
            drop(Box::from_raw(self.inner));
        }
    }
}

// ---------- Internals ------------------------------------------------------

type Callback = Box<dyn Fn() + Send + 'static>;

/// State shared between the run-loop thread, the tap callback, and any
/// in-flight debounce threads. Held inside an `Arc` so debounce threads
/// can outlive `Inner` if they were sleeping when teardown happened.
struct Shared {
    spec: ChordSpec,
    on_press: PMutex<Callback>,
    on_release: PMutex<Callback>,
    armed: AtomicBool,
    /// Monotonically increasing event sequence — used to discard a
    /// stale debounce timer if the modifier state changed before the
    /// debounce window expired.
    last_seq: AtomicU64,
}

struct Inner {
    /// Shared state; debounce threads keep their own `Arc<Shared>`.
    shared: Arc<Shared>,
    run_loop: Mutex<CFRunLoopRef>,
    mach_port: Mutex<CFMachPortRef>,
    run_loop_source: Mutex<CFRunLoopSourceRef>,
    ready_tx: Mutex<Option<std::sync::mpsc::Sender<Result<(), ChordError>>>>,
}

fn run_loop_thread(inner_ptr: *mut Inner) {
    // SAFETY: `inner_ptr` is valid until `ChordHandle::Drop` joins us.
    let inner: &Inner = unsafe { &*inner_ptr };

    let mask: CGEventMask = 1 << K_CG_EVENT_FLAGS_CHANGED;
    // Cast the user_info pointer through `*mut c_void`.
    let user_info = inner_ptr as *mut std::ffi::c_void;

    let port = unsafe {
        CGEventTapCreate(
            K_CG_SESSION_EVENT_TAP,
            K_CG_HEAD_INSERT_EVENT_TAP,
            K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
            mask,
            tap_callback,
            user_info,
        )
    };

    if port.is_null() {
        // Failed — likely missing Accessibility despite our pre-check
        // (e.g. revoked between AXIsProcessTrusted and TapCreate).
        notify_ready(inner, Err(ChordError::TapCreateFailed));
        return;
    }

    let source =
        unsafe { CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0) };
    if source.is_null() {
        unsafe {
            CGEventTapEnable(port, false);
            CFRelease(port);
        }
        notify_ready(inner, Err(ChordError::RunLoopFailed));
        return;
    }

    let rl = unsafe { CFRunLoopGetCurrent() };
    unsafe {
        CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
        CGEventTapEnable(port, true);
    }

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

    // Park the thread on the run loop. Returns when CFRunLoopStop is
    // called from the drop handler (or, theoretically, never — we add
    // a watchdog timer below to make sure we can always exit).
    unsafe { CFRunLoopRun() };

    // Run loop has stopped — the drop handler will CFRelease + free.
    log::debug!("mac_chord: run loop exited");
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
    // SAFETY: `user_info` is the `Box::into_raw(Inner)` we gave to
    // CGEventTapCreate, kept alive by `ChordHandle` until that handle
    // is dropped (which joins this thread before freeing).
    let inner: &Inner = unsafe { &*(user_info as *const Inner) };

    // Re-arm the tap if macOS disabled it (timeout / user input).
    if ty == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT
        || ty == K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT
    {
        if let Ok(g) = inner.mach_port.lock() {
            let port = *g;
            if !port.is_null() {
                unsafe { CGEventTapEnable(port, true) };
                log::warn!(
                    "mac_chord: tap disabled by macOS ({:#x}); re-enabled",
                    ty
                );
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

    let armed_now = shared.armed.load(Ordering::SeqCst);
    let chord_held = if shared.spec.strict_all_or_none {
        flags == required
    } else {
        flags & required == required
    };

    let seq = shared.last_seq.fetch_add(1, Ordering::SeqCst) + 1;

    match (armed_now, chord_held) {
        (false, true) => {
            // Schedule a 60ms debounce. If the chord is still held when
            // the timer fires AND we're still on the same sequence,
            // arm and fire on_press. We hand a fresh `Arc<Shared>` to
            // the debounce thread — debounce threads can outlive
            // `Inner` (the box behind `user_info`) without UAF because
            // all the data they touch lives in `Shared`.
            let shared_for_debounce = shared;
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(60));
                if shared_for_debounce.last_seq.load(Ordering::SeqCst) != seq {
                    return; // superseded by another flag change
                }
                if shared_for_debounce.armed.swap(true, Ordering::SeqCst) {
                    return; // already armed by a parallel raise
                }
                let cb = shared_for_debounce.on_press.lock();
                (cb)();
            });
        }
        (true, false) => {
            // Released: fire on_release.
            shared.armed.store(false, Ordering::SeqCst);
            let cb = shared.on_release.lock();
            (cb)();
        }
        _ => {}
    }

    event
}

// ---------- Spec parsing ---------------------------------------------------

/// Try to interpret a hotkey-id string as a modifier-only chord. Returns
/// `None` for strings that include a non-modifier base key (e.g. `Alt+M`,
/// `F13`).
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
                out.need_command = true;
            }
            "Shift" => out.need_shift = true,
            "" => return None,
            _ => return None, // unrecognised token = not a pure chord
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
