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

/// Shared scaffolding owned jointly by the install caller (via
/// `ChordHandle`) and the worker thread. Carries the CF refs the worker
/// fills in once the tap is live; the drop handler snapshots them out
/// without touching the typed `Inner` so single-tap chord and
/// double-tap handles can share one `ChordHandle` shape.
struct TapHandles {
    run_loop: Mutex<CFRunLoopRef>,
    mach_port: Mutex<CFMachPortRef>,
    run_loop_source: Mutex<CFRunLoopSourceRef>,
}

impl TapHandles {
    fn new() -> Self {
        Self {
            run_loop: Mutex::new(std::ptr::null()),
            mach_port: Mutex::new(std::ptr::null()),
            run_loop_source: Mutex::new(std::ptr::null()),
        }
    }
}

// SAFETY: the contained CF refs are kernel mach port pointers; the
// Mutex guards make the cross-thread fills race-free.
unsafe impl Send for TapHandles {}
unsafe impl Sync for TapHandles {}

/// Opaque handle. Drop to remove the chord listener.
pub struct ChordHandle {
    handles: Arc<TapHandles>,
    thread: Option<JoinHandle<()>>,
}

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
    log::debug!(
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

    let handles = Arc::new(TapHandles::new());
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), ChordError>>();

    // Run-loop / mach port handles are filled in on the worker thread.
    let inner_ptr: *mut Inner = Box::into_raw(Box::new(Inner {
        shared: shared.clone(),
        handles: handles.clone(),
        ready_tx: Mutex::new(Some(ready_tx)),
    }));

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
            handles,
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
        // Snapshot the CF refs from the Arc — the worker writes them
        // once it has the tap up, and they're stable for the lifetime
        // of the run loop. Ask the run loop to stop, join the worker,
        // then release the CF objects we still hold a reference to.
        let port_snap = self
            .handles
            .mach_port
            .lock()
            .ok()
            .map(|g| *g)
            .unwrap_or(std::ptr::null());
        let rl_snap = self
            .handles
            .run_loop
            .lock()
            .ok()
            .map(|g| *g)
            .unwrap_or(std::ptr::null());
        let src_snap = self
            .handles
            .run_loop_source
            .lock()
            .ok()
            .map(|g| *g)
            .unwrap_or(std::ptr::null());

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
            let _ = t.join();
        }

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
    handles: Arc<TapHandles>,
    ready_tx: Mutex<Option<std::sync::mpsc::Sender<Result<(), ChordError>>>>,
}

fn run_loop_thread(inner_ptr: *mut Inner) {
    let mask: CGEventMask = 1 << K_CG_EVENT_FLAGS_CHANGED;

    log::debug!(
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
        log::error!("mac_chord: CGEventTapCreate returned NULL — Accessibility revoked?");
        let inner: &Inner = unsafe { &*inner_ptr };
        notify_ready(inner, Err(ChordError::TapCreateFailed));
        unsafe { drop(Box::from_raw(inner_ptr)) };
        return;
    }
    log::debug!("mac_chord: CGEventTapCreate ok (port={:p})", port);

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
    log::debug!("mac_chord: tap enabled; entering CFRunLoopRun (rl={:p})", rl);

    {
        let inner: &Inner = unsafe { &*inner_ptr };
        if let Ok(mut g) = inner.handles.run_loop.lock() {
            *g = rl;
        }
        if let Ok(mut g) = inner.handles.mach_port.lock() {
            *g = port;
        }
        if let Ok(mut g) = inner.handles.run_loop_source.lock() {
            *g = source;
        }

        notify_ready(inner, Ok(()));
    }

    unsafe { CFRunLoopRun() };
    log::debug!("mac_chord: CFRunLoopRun exited; thread terminating");

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
    let inner: &Inner = unsafe { &*(user_info as *const Inner) };

    if ty == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT
        || ty == K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT
    {
        if let Ok(g) = inner.handles.mach_port.lock() {
            if !g.is_null() {
                unsafe { CGEventTapEnable(*g, true) };
                log::warn!("mac_chord: tap disabled by macOS ({:#x}); re-enabled", ty);
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
            log::trace!(
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
                log::trace!("mac_chord: debounce fired on_press (seq={seq})");
                (shared.on_press.lock())();
            });
        }
        (true, false) => {
            log::trace!(
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
    log::trace!(
        "mac_chord: flagsChanged #{total} raw={raw:#x} masked={masked:#x} required={required:#x}"
    );
}

/// Interpret a hotkey-id string as a modifier-only chord. `None` if it
/// contains a non-modifier base key.
///
/// Accepts the explicit `Mods:Cmd+Alt` prefix (v2 wire format) and the
/// legacy bare `Cmd+Alt` form. The legacy form is kept so any on-disk
/// settings doc that hand-rolled a mod-only spec keeps working.
pub fn try_parse_chord(spec: &str) -> Option<ChordSpec> {
    let raw = spec.trim();
    if raw.is_empty() {
        return None;
    }
    // Reject the `Tap2:` prefix here — double-tap goes through a
    // different installer (see `try_parse_double_tap` /
    // `install_double_tap`).
    if raw.starts_with("Tap2:") {
        return None;
    }
    let body = raw.strip_prefix("Mods:").unwrap_or(raw).trim();
    if body.is_empty() {
        return None;
    }
    let mut out = ChordSpec {
        need_option: false,
        need_control: false,
        need_command: false,
        need_shift: false,
        strict_all_or_none: true,
    };
    for tok in body.split('+') {
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

/// Which single modifier a `Tap2:` spec targets. v2 supports
/// single-modifier double-tap only (`Tap2:Alt`); `Tap2:Cmd+Alt` is
/// out of scope per PM spec §9.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubleTapModifier {
    Command,
    Option,
    Control,
    Shift,
}

impl DoubleTapModifier {
    pub fn flag(self) -> u64 {
        match self {
            DoubleTapModifier::Command => FLAG_COMMAND,
            DoubleTapModifier::Option => FLAG_ALTERNATE,
            DoubleTapModifier::Control => FLAG_CONTROL,
            DoubleTapModifier::Shift => FLAG_SHIFT,
        }
    }
}

/// Interpret a hotkey-id string as a `Tap2:<Modifier>` spec. Returns
/// `None` for any other shape — multi-modifier double-tap, missing
/// prefix, base keys, etc.
pub fn try_parse_double_tap(spec: &str) -> Option<DoubleTapModifier> {
    let raw = spec.trim();
    let body = raw.strip_prefix("Tap2:")?.trim();
    if body.is_empty() {
        return None;
    }
    // Multi-modifier `Tap2:Cmd+Alt` is intentionally rejected — v2
    // ships single-modifier only.
    if body.contains('+') {
        return None;
    }
    match body {
        "Alt" | "Option" => Some(DoubleTapModifier::Option),
        "Ctrl" | "Control" => Some(DoubleTapModifier::Control),
        "Cmd" | "Command" | "Meta" | "Super" | "CommandOrControl" => {
            Some(DoubleTapModifier::Command)
        }
        "Shift" => Some(DoubleTapModifier::Shift),
        _ => None,
    }
}

/// Window inside which a second tap of the same modifier triggers the
/// double-tap. PM spec §4 / design spec §8 settle on 300 ms.
pub const DOUBLE_TAP_TRIGGER_MS: u64 = 300;

/// Pure-function state machine for the double-tap trigger. Lives next
/// to the CGEventTap callback so the timing rules are testable without
/// mocking the tap. The callback feeds it (`flags`, `now_ms`) on every
/// `kCGEventFlagsChanged` event and acts on the returned `Action`.
#[derive(Debug, Clone, Copy)]
pub struct DoubleTapState {
    pub target: DoubleTapModifier,
    /// `Some(t)` after a clean "down, up" of the target with the rest
    /// of the modifier set empty. Cleared when any other modifier
    /// joins or when the trigger fires.
    pub first_release_at: Option<u64>,
    /// `true` while the target modifier is held above the baseline.
    pub target_down: bool,
    /// Set when another modifier joined the set while the target was
    /// held. The subsequent target-release is the end of a chord, not
    /// a fresh first-tap release — don't arm `first_release_at`. Cleared
    /// once both target + other are baseline.
    pub tainted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubleTapAction {
    /// Promote to the held state — fire `on_press`.
    Press,
    /// Cancel the held state — fire `on_release`.
    Release,
    /// Nothing visible to the caller.
    Idle,
}

impl DoubleTapState {
    pub fn new(target: DoubleTapModifier) -> Self {
        Self {
            target,
            first_release_at: None,
            target_down: false,
            tainted: false,
        }
    }

    /// Step the state machine on a `kCGEventFlagsChanged` event.
    ///
    /// `flags` is masked to `MOD_MASK` already; `armed` reports whether
    /// `on_press` has been called and not yet released. The state
    /// machine itself owns the trigger timing — `armed` is the caller's
    /// side of the contract so we don't double-fire `Press`.
    pub fn step(&mut self, flags: u64, armed: bool, now_ms: u64) -> DoubleTapAction {
        let target_bit = self.target.flag();
        let target_held = flags & target_bit != 0;
        let other_held = flags & !target_bit & MOD_MASK != 0;

        // When the user joins any other modifier into the set, the
        // armed double-tap path is dead until the modifiers all drop.
        if other_held {
            self.first_release_at = None;
            self.target_down = target_held;
            self.tainted = true;
            if armed {
                return DoubleTapAction::Release;
            }
            return DoubleTapAction::Idle;
        }

        if armed {
            if !target_held {
                self.target_down = false;
                self.first_release_at = None;
                self.tainted = false;
                return DoubleTapAction::Release;
            }
            self.target_down = true;
            return DoubleTapAction::Idle;
        }

        // From the unarmed state, with nothing else held.
        let prev_down = self.target_down;
        let was_tainted = self.tainted;
        self.target_down = target_held;

        if !prev_down && target_held {
            // Press transition. If we already saw a clean release of
            // the target inside the window, that's a double-tap.
            self.tainted = false;
            if let Some(first) = self.first_release_at.take() {
                if now_ms.saturating_sub(first) <= DOUBLE_TAP_TRIGGER_MS {
                    return DoubleTapAction::Press;
                }
            }
            return DoubleTapAction::Idle;
        }
        if prev_down && !target_held {
            // Lone modifier released. If the press path was tainted by
            // another modifier joining in, this isn't a fresh first-tap
            // release — don't arm.
            if !was_tainted {
                self.first_release_at = Some(now_ms);
            }
            self.tainted = false;
            return DoubleTapAction::Idle;
        }
        DoubleTapAction::Idle
    }
}

/// Install a double-tap listener for a single modifier. Returns the
/// same `ChordHandle` shape as `install` so `hotkey::register` can hold
/// it on `ActiveHotkeyState::chord` and drop it on re-bind.
pub fn install_double_tap(
    target: DoubleTapModifier,
    on_press: impl Fn() + Send + 'static,
    on_release: impl Fn() + Send + 'static,
) -> Result<ChordHandle, ChordError> {
    let trusted = unsafe { AXIsProcessTrusted() };
    log::debug!(
        "mac_chord: install_double_tap target={target:?} AXIsProcessTrusted={trusted}"
    );
    if !trusted {
        return Err(ChordError::PermissionDenied);
    }

    let shared = Arc::new(SharedTap2 {
        target,
        on_press: PMutex::new(Box::new(on_press)),
        on_release: PMutex::new(Box::new(on_release)),
        armed: AtomicBool::new(false),
        state: PMutex::new(DoubleTapState::new(target)),
    });

    let handles = Arc::new(TapHandles::new());
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), ChordError>>();

    let inner_ptr: *mut Tap2Inner = Box::into_raw(Box::new(Tap2Inner {
        shared: shared.clone(),
        handles: handles.clone(),
        ready_tx: Mutex::new(Some(ready_tx)),
    }));
    let inner_addr: usize = inner_ptr as usize;

    let thread = thread::Builder::new()
        .name("vox-tap2-tap".into())
        .spawn(move || tap2_run_loop_thread(inner_addr as *mut Tap2Inner))
        .map_err(|_| ChordError::RunLoopFailed)?;
    let result = ready_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or(Err(ChordError::RunLoopFailed));
    match result {
        Ok(()) => Ok(ChordHandle {
            handles,
            thread: Some(thread),
        }),
        Err(e) => {
            let _ = thread.join();
            Err(e)
        }
    }
}

struct SharedTap2 {
    target: DoubleTapModifier,
    on_press: PMutex<Callback>,
    on_release: PMutex<Callback>,
    armed: AtomicBool,
    state: PMutex<DoubleTapState>,
}

struct Tap2Inner {
    shared: Arc<SharedTap2>,
    handles: Arc<TapHandles>,
    ready_tx: Mutex<Option<std::sync::mpsc::Sender<Result<(), ChordError>>>>,
}

fn tap2_run_loop_thread(inner_ptr: *mut Tap2Inner) {
    let mask: CGEventMask = 1 << K_CG_EVENT_FLAGS_CHANGED;
    let port = unsafe {
        CGEventTapCreate(
            K_CG_SESSION_EVENT_TAP,
            K_CG_HEAD_INSERT_EVENT_TAP,
            K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
            mask,
            tap2_callback,
            inner_ptr as *mut std::ffi::c_void,
        )
    };
    if port.is_null() {
        let inner: &Tap2Inner = unsafe { &*inner_ptr };
        tap2_notify_ready(inner, Err(ChordError::TapCreateFailed));
        unsafe { drop(Box::from_raw(inner_ptr)) };
        return;
    }
    let source = unsafe { CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0) };
    if source.is_null() {
        unsafe {
            CGEventTapEnable(port, false);
            CFRelease(port);
        }
        let inner: &Tap2Inner = unsafe { &*inner_ptr };
        tap2_notify_ready(inner, Err(ChordError::RunLoopFailed));
        unsafe { drop(Box::from_raw(inner_ptr)) };
        return;
    }
    let rl = unsafe { CFRunLoopGetCurrent() };
    unsafe {
        CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
        CGEventTapEnable(port, true);
    }
    {
        let inner: &Tap2Inner = unsafe { &*inner_ptr };
        if let Ok(mut g) = inner.handles.run_loop.lock() {
            *g = rl;
        }
        if let Ok(mut g) = inner.handles.mach_port.lock() {
            *g = port;
        }
        if let Ok(mut g) = inner.handles.run_loop_source.lock() {
            *g = source;
        }
        tap2_notify_ready(inner, Ok(()));
    }
    unsafe { CFRunLoopRun() };
    unsafe { drop(Box::from_raw(inner_ptr)) };
}

fn tap2_notify_ready(inner: &Tap2Inner, result: Result<(), ChordError>) {
    if let Ok(mut slot) = inner.ready_tx.lock() {
        if let Some(tx) = slot.take() {
            let _ = tx.send(result);
        }
    }
}

extern "C" fn tap2_callback(
    _proxy: CGEventTapProxy,
    ty: CGEventType,
    event: CGEventRef,
    user_info: *mut std::ffi::c_void,
) -> CGEventRef {
    if user_info.is_null() {
        return event;
    }
    let inner: &Tap2Inner = unsafe { &*(user_info as *const Tap2Inner) };
    if ty == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT
        || ty == K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT
    {
        if let Ok(g) = inner.handles.mach_port.lock() {
            if !g.is_null() {
                unsafe { CGEventTapEnable(*g, true) };
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
    let now_ms = monotonic_ms();
    let armed = shared.armed.load(Ordering::SeqCst);
    let action = {
        let mut state = shared.state.lock();
        state.step(flags, armed, now_ms)
    };
    match action {
        DoubleTapAction::Press => {
            if !shared.armed.swap(true, Ordering::SeqCst) {
                log::trace!(
                    "mac_chord: double-tap fired on_press target={:?}",
                    shared.target
                );
                (shared.on_press.lock())();
            }
        }
        DoubleTapAction::Release => {
            if shared.armed.swap(false, Ordering::SeqCst) {
                log::trace!(
                    "mac_chord: double-tap fired on_release target={:?}",
                    shared.target
                );
                (shared.on_release.lock())();
            }
        }
        DoubleTapAction::Idle => {}
    }
    event
}

fn monotonic_ms() -> u64 {
    static BASELINE: once_cell::sync::OnceCell<Instant> = once_cell::sync::OnceCell::new();
    BASELINE.get_or_init(Instant::now).elapsed().as_millis() as u64
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

    #[test]
    fn parse_chord_accepts_mods_prefix() {
        let s = try_parse_chord("Mods:Cmd+Alt").unwrap();
        assert!(s.need_command && s.need_option);
        assert!(!s.need_shift && !s.need_control);
    }

    #[test]
    fn parse_chord_rejects_tap2_prefix() {
        assert!(try_parse_chord("Tap2:Alt").is_none());
        assert!(try_parse_chord("Tap2:Cmd+Alt").is_none());
    }

    #[test]
    fn parse_chord_rejects_mods_with_base_key() {
        assert!(try_parse_chord("Mods:Cmd+Alt+M").is_none());
        assert!(try_parse_chord("Mods:").is_none());
    }

    #[test]
    fn parse_double_tap_each_modifier() {
        assert_eq!(try_parse_double_tap("Tap2:Alt"), Some(DoubleTapModifier::Option));
        assert_eq!(try_parse_double_tap("Tap2:Cmd"), Some(DoubleTapModifier::Command));
        assert_eq!(try_parse_double_tap("Tap2:Control"), Some(DoubleTapModifier::Control));
        assert_eq!(try_parse_double_tap("Tap2:Shift"), Some(DoubleTapModifier::Shift));
        assert_eq!(
            try_parse_double_tap("Tap2:CommandOrControl"),
            Some(DoubleTapModifier::Command),
        );
    }

    #[test]
    fn parse_double_tap_rejects_multi_mod() {
        // Multi-modifier double-tap is out of scope per PM spec §9.
        assert!(try_parse_double_tap("Tap2:Cmd+Alt").is_none());
    }

    #[test]
    fn parse_double_tap_rejects_unprefixed_and_legacy() {
        assert!(try_parse_double_tap("Alt").is_none());
        assert!(try_parse_double_tap("Mods:Alt").is_none());
        assert!(try_parse_double_tap("Cmd+Alt+Period").is_none());
        assert!(try_parse_double_tap("").is_none());
    }

    #[test]
    fn double_tap_state_fires_within_window() {
        let mut s = DoubleTapState::new(DoubleTapModifier::Option);
        // First press of Option only.
        assert_eq!(s.step(FLAG_ALTERNATE, false, 0), DoubleTapAction::Idle);
        // First release of Option only — sets first_release_at.
        assert_eq!(s.step(0, false, 50), DoubleTapAction::Idle);
        // Second press inside the 300 ms window — fires Press.
        assert_eq!(
            s.step(FLAG_ALTERNATE, false, 200),
            DoubleTapAction::Press,
        );
        // The caller flips armed=true after Press fires; subsequent
        // events without target_held release.
        assert_eq!(s.step(0, true, 240), DoubleTapAction::Release);
    }

    #[test]
    fn double_tap_state_misses_outside_window() {
        let mut s = DoubleTapState::new(DoubleTapModifier::Option);
        assert_eq!(s.step(FLAG_ALTERNATE, false, 0), DoubleTapAction::Idle);
        assert_eq!(s.step(0, false, 50), DoubleTapAction::Idle);
        // 400 ms later is well past the 300 ms window — no Press.
        assert_eq!(s.step(FLAG_ALTERNATE, false, 450), DoubleTapAction::Idle);
        assert_eq!(s.step(0, false, 500), DoubleTapAction::Idle);
    }

    #[test]
    fn double_tap_state_cancels_on_other_modifier() {
        let mut s = DoubleTapState::new(DoubleTapModifier::Option);
        assert_eq!(s.step(FLAG_ALTERNATE, false, 0), DoubleTapAction::Idle);
        assert_eq!(s.step(0, false, 50), DoubleTapAction::Idle);
        // User folds Cmd into the set inside the window — no double-tap.
        assert_eq!(
            s.step(FLAG_ALTERNATE | FLAG_COMMAND, false, 100),
            DoubleTapAction::Idle,
        );
        // Drop everything and tap Option again within 300 ms of the
        // ORIGINAL release — still no Press because the tracker was
        // cleared when Cmd joined.
        assert_eq!(s.step(0, false, 150), DoubleTapAction::Idle);
        assert_eq!(s.step(FLAG_ALTERNATE, false, 180), DoubleTapAction::Idle);
    }

    #[test]
    fn double_tap_state_releases_when_armed_modifier_lifts() {
        let mut s = DoubleTapState::new(DoubleTapModifier::Option);
        // Skip past the arm transitions — the caller's armed=true is
        // what gates the release path.
        assert_eq!(s.step(0, true, 0), DoubleTapAction::Release);
    }

    #[test]
    fn double_tap_state_fires_after_long_hold_release() {
        let mut s = DoubleTapState::new(DoubleTapModifier::Option);
        assert_eq!(s.step(FLAG_ALTERNATE, false, 0), DoubleTapAction::Idle);
        // Held for 2 s, then released.
        assert_eq!(s.step(0, false, 2000), DoubleTapAction::Idle);
        // Tap within 300 ms of the release fires Press.
        assert_eq!(
            s.step(FLAG_ALTERNATE, false, 2100),
            DoubleTapAction::Press,
        );
    }
}
