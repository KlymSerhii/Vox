//! Subscribe to `NSWorkspaceDidWakeNotification` so the background
//! update scheduler can re-evaluate immediately after the Mac wakes
//! from sleep. Wall-clock-based timers already handle a >24 h gap
//! correctly across a sleep cycle; this notification just shortens the
//! worst-case latency from "next tokio tick" to "right now (with the
//! configured debounce in `updater::should_check_now`)".
//!
//! Mirrors the ObjC FFI shape in `mac_perm.rs`: hand-rolled `msg_send!`
//! calls against the AppKit/Foundation classes. The objc2-app-kit
//! `NSWorkspace` binding is enabled in `Cargo.toml`, but we still
//! reach for `addObserverForName:object:queue:usingBlock:` directly
//! because objc2-foundation's `NSNotificationCenter` feature isn't
//! turned on and pulling it in just for this one call would widen the
//! AppKit link surface for no benefit. Non-macOS builds compile to a
//! no-op stub so shared callers don't need their own `cfg` gates.

#[cfg(target_os = "macos")]
mod imp {
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;
    use once_cell::sync::OnceCell;

    /// Idempotency guard. Stashed AFTER the observer token + block are
    /// leaked so a double-subscribe doesn't accidentally short-circuit
    /// before the strong references are taken.
    static REGISTERED: OnceCell<()> = OnceCell::new();

    /// Subscribe to `NSWorkspaceDidWakeNotification`. Idempotent —
    /// repeat calls are no-ops. With a `nil` queue argument, the
    /// callback fires synchronously on the notification's posting
    /// thread (typically the main thread, but not guaranteed). The
    /// callback only does `tauri::async_runtime::spawn`, which is
    /// `Send`-safe — keep it that way.
    pub fn subscribe<F>(callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        if REGISTERED.get().is_some() {
            return;
        }

        // SAFETY: `+sharedWorkspace` returns the process-singleton
        // workspace; `-notificationCenter` returns its per-process
        // notification centre. Both are unretained and live for the
        // lifetime of the app.
        unsafe {
            let workspace = NSWorkspace::sharedWorkspace();
            let ws_obj: &AnyObject = &workspace;
            let centre: *mut AnyObject = msg_send![ws_obj, notificationCenter];
            if centre.is_null() {
                log::warn!("mac_wake: -[NSWorkspace notificationCenter] returned NULL");
                return;
            }

            // NSWorkspaceDidWakeNotification name. Constructed via
            // NSString rather than linking the global symbol because
            // the Foundation feature flag we'd need for the binding
            // isn't enabled.
            let name = NSString::from_str("NSWorkspaceDidWakeNotification");
            let name_obj: &AnyObject = &name;

            let block = RcBlock::new(move |_n: *mut AnyObject| {
                callback();
            });

            // `addObserverForName:object:queue:usingBlock:` returns an
            // opaque observer token. We leak both the block and the
            // token: there is no unsubscribe path in this codebase, and
            // AppKit's retain on the block keeps it alive for as long
            // as the observer is registered. The leak is one block +
            // one token for the lifetime of the process.
            let nil: *mut AnyObject = std::ptr::null_mut();
            let observer: *mut AnyObject = msg_send![
                centre,
                addObserverForName: name_obj,
                object: nil,
                queue: nil,
                usingBlock: &*block,
            ];
            if observer.is_null() {
                log::warn!("mac_wake: addObserverForName: returned NULL token");
                return;
            }
            std::mem::forget(block);
            let _ = REGISTERED.set(());
            log::info!("mac_wake: subscribed to NSWorkspaceDidWakeNotification");
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn subscribe<F>(_callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        // No-op on non-macOS. The background updater scheduler in
        // `updater.rs` is robust to wake-from-sleep via wall-clock
        // comparison alone.
    }
}

/// Register a callback for the macOS wake-from-sleep notification.
/// Idempotent — repeat calls are no-ops. With a `nil` observer queue
/// the callback fires synchronously on the notification's posting
/// thread (typically main, but not guaranteed); spawn into tokio
/// for any non-trivial work.
pub fn subscribe<F>(callback: F)
where
    F: Fn() + Send + Sync + 'static,
{
    imp::subscribe(callback)
}
