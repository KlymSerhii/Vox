//! Audio attenuation — quiet other audio (music, video) while a
//! dictation session is recording, then put it back.
//!
//! Driven by the `Settings::audio_attenuation` string:
//!   - `"off"`   — do nothing.
//!   - `"duck"`  — lower the system output volume to [`DUCK_FACTOR`] of
//!                 its current level (default).
//!   - `"pause"` — post a media Play/Pause key to pause whatever's
//!                 playing (opt-in). Falls back to ducking when the
//!                 media key can't be posted (macOS Accessibility
//!                 missing).
//!
//! This module is the OS-agnostic facade + state machine. The actual
//! device pokes live in a per-OS *leaf* aliased as `leaf` below:
//! [`crate::mac_audio`] on macOS, `crate::win_audio` on Windows (written
//! by another engineer), and a local no-op stub everywhere else. All
//! three expose the identical five functions (`output_volume`,
//! `set_output_volume`, `output_is_active`, `supports_play_pause`,
//! `send_play_pause`).
//!
//! **Fail-open.** Every operation here is best-effort: a failure to
//! duck, pause, or restore logs and moves on. [`restore`] in particular
//! runs from a `Drop` impl in `session.rs`, so it must never panic.
//!
//! **Restore is anchored to mic-release.** The session's primary
//! `restore` fires the moment recording stops (in `end_recording`),
//! symmetric with the serialized recording lifecycle, so an overlapping
//! pipeline's later `begin` can't race a second [`Snapshot`] over the one
//! global slot while the prior one is still in flight; the cancel and
//! `PipelineGuard::drop` restores are idempotent backstops. The module
//! keeps a single global [`Snapshot`]; [`begin`] still defends against a
//! stale one by restoring it first, and [`restore`] is idempotent (a
//! second call take()s nothing and no-ops).
//!
//! **Playback is probed before mic-open.** [`begin`] takes a
//! `was_playing` bool the caller captures via [`probe_playback`] *before*
//! starting capture — a combined Bluetooth headset (one device for input
//! + output) can otherwise read its output as "active" purely because our
//! mic is open, which would let pause mode start music from silence.

use std::sync::Mutex;
use std::sync::OnceLock;

#[cfg(target_os = "macos")]
use crate::mac_audio as leaf;
#[cfg(target_os = "windows")]
use crate::win_audio as leaf;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod leaf {
    // No-op leaf for hosts without a CoreAudio / WASAPI backend. The
    // facade compiles and runs everywhere; on these targets attenuation
    // is simply a no-op (no playback signal, no volume control).
    pub fn output_volume() -> Option<f32> {
        None
    }
    pub fn set_output_volume(_v: f32) -> bool {
        false
    }
    pub fn output_is_active() -> bool {
        false
    }
    pub fn supports_play_pause() -> bool {
        // The stub can't post a media key (`send_play_pause` is a no-op
        // returning `false`), so report it unsupported — pause mode then
        // routes through the friendly duck-fallback branch instead of
        // logging a spurious "failed to post pause key".
        false
    }
    pub fn send_play_pause() -> bool {
        false
    }
}

/// Fraction of the prior volume we duck down to. 0.25 = 25%, quiet
/// enough to hear yourself over background music without fully muting.
const DUCK_FACTOR: f32 = 0.25;

/// Tolerance when deciding whether the user moved the volume slider
/// themselves mid-session. If the live volume is within this of where
/// we left it, we assume it's still ours and restore; otherwise we back
/// off and leave the user's choice alone.
const RESTORE_EPSILON: f32 = 0.02;

/// What we did at session start, so [`restore`] can undo exactly that.
struct Snapshot {
    /// The volume before we ducked. `Some` only on the duck path.
    prior_volume: Option<f32>,
    /// The volume we set. `Some` only on the duck path — used to detect
    /// a user override before restoring.
    ducked_to: Option<f32>,
    /// True if we posted a pause (so restore posts a play to resume).
    we_paused: bool,
}

/// The single active attenuation, or `None` when nothing is attenuated.
/// `std::sync::Mutex` in a `OnceLock` — matches the lazy-static-free
/// style elsewhere in the crate and keeps poison handling explicit.
fn state() -> &'static Mutex<Option<Snapshot>> {
    static STATE: OnceLock<Mutex<Option<Snapshot>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

/// Apply the duck: read the current volume, drop it to [`DUCK_FACTOR`],
/// and return the [`Snapshot`] to store on success. `None` (store
/// nothing) if the volume is unreadable / unsettable. Pure of the
/// global lock — the caller holds it.
fn duck_now() -> Option<Snapshot> {
    let Some(prior) = leaf::output_volume() else {
        log::warn!("audio attenuation: output volume unreadable; not ducking");
        return None;
    };
    let target = (prior * DUCK_FACTOR).clamp(0.0, 1.0);
    if leaf::set_output_volume(target) {
        log::debug!("audio attenuation: ducked {prior:.2} -> {target:.2}");
        Some(Snapshot {
            prior_volume: Some(prior),
            ducked_to: Some(target),
            we_paused: false,
        })
    } else {
        log::warn!("audio attenuation: output volume not settable; not ducking");
        None
    }
}

/// Undo one snapshot. Best-effort, never panics — shared by [`restore`]
/// and [`begin`]'s defensive stale-snapshot cleanup.
fn restore_snapshot(snap: Snapshot) {
    if snap.we_paused {
        // We paused playback; post Play/Pause again to resume.
        if leaf::send_play_pause() {
            log::debug!("audio attenuation: resumed paused media");
        } else {
            log::warn!("audio attenuation: failed to post resume key");
        }
        return;
    }

    if let (Some(prior), Some(ducked)) = (snap.prior_volume, snap.ducked_to) {
        // Only restore if the user hasn't taken the slider over since we
        // ducked. If the live volume drifted away from where we left it,
        // they changed it — respect that and leave it alone.
        match leaf::output_volume() {
            Some(current) if (current - ducked).abs() < RESTORE_EPSILON => {
                if leaf::set_output_volume(prior) {
                    log::debug!("audio attenuation: restored volume to {prior:.2}");
                } else {
                    log::warn!("audio attenuation: failed to restore volume");
                }
            }
            Some(current) => {
                log::debug!(
                    "audio attenuation: volume moved to {current:.2} (left at {ducked:.2}); \
                     user took over, leaving as-is"
                );
            }
            None => {
                log::debug!("audio attenuation: volume unreadable on restore; leaving as-is");
            }
        }
    }
}

/// Probe whether audio is currently playing on the default output.
/// Panic-safe — the leaf already returns `false` on any failure. The
/// caller is expected to run this *before* opening the mic: on a combined
/// Bluetooth headset (one device carrying input + output) an open capture
/// stream can make the output read "active" even in silence, so a reading
/// taken after mic-open would be a false positive (see [`begin`]).
pub fn probe_playback() -> bool {
    leaf::output_is_active()
}

/// Begin attenuating for a new recording, per the user's
/// `audio_attenuation` setting string. No-op for `"off"` / unknown.
///
/// Gated on playback via the caller-supplied `was_playing` (captured by
/// [`probe_playback`] *before* the mic opened): if nothing was playing on
/// the default output we do nothing and store no snapshot — ducking
/// silence is pointless, and *pausing* silence could toggle audio on.
/// Stores at most one snapshot; a leftover one (shouldn't happen,
/// sessions are serialised) is restored first.
pub fn begin(mode_setting: &str, was_playing: bool) {
    let mode = Mode::from_setting(mode_setting);
    if matches!(mode, Mode::Off) {
        return;
    }

    let mut guard = match state().lock() {
        Ok(g) => g,
        Err(poisoned) => {
            // A prior panic poisoned the lock. Recover the guard and
            // carry on — attenuation must never wedge dictation.
            log::warn!("audio attenuation: state lock poisoned; recovering");
            poisoned.into_inner()
        }
    };

    // Defensive: a snapshot already exists. Sessions are serialised so
    // this shouldn't happen, but if it does, undo it before starting
    // fresh rather than leaking the prior state.
    if let Some(stale) = guard.take() {
        log::warn!("audio attenuation: stale snapshot present on begin; restoring it first");
        restore_snapshot(stale);
    }

    // Only attenuate when something was actually playing (probed before
    // mic-open by the caller — see `was_playing`).
    if !was_playing {
        log::debug!("audio attenuation: nothing playing; skipping");
        return;
    }

    let snapshot = match mode {
        Mode::Off => return, // unreachable (handled above), keeps match total
        Mode::Duck => duck_now(),
        Mode::Pause => {
            if leaf::supports_play_pause() {
                if leaf::send_play_pause() {
                    log::debug!("audio attenuation: paused media");
                    Some(Snapshot {
                        prior_volume: None,
                        ducked_to: None,
                        we_paused: true,
                    })
                } else {
                    log::warn!("audio attenuation: failed to post pause key; not attenuating");
                    None
                }
            } else {
                // Media key unavailable (macOS: Accessibility not
                // granted). Fall back to ducking so the user still gets
                // attenuation.
                log::info!(
                    "audio attenuation: play/pause unsupported (Accessibility?); \
                     falling back to duck"
                );
                duck_now()
            }
        }
    };

    *guard = snapshot;
}

/// Restore whatever [`begin`] changed. Idempotent — a second call is a
/// no-op (the snapshot is taken on the first). Safe to call from a
/// `Drop` impl: never panics, recovers a poisoned lock.
pub fn restore() {
    let snap = {
        let mut guard = match state().lock() {
            Ok(g) => g,
            Err(poisoned) => {
                log::warn!("audio attenuation: state lock poisoned on restore; recovering");
                poisoned.into_inner()
            }
        };
        guard.take()
    };

    if let Some(snap) = snap {
        restore_snapshot(snap);
    }
}

/// The attenuation behaviour for a session, parsed from the
/// `audio_attenuation` setting string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Do nothing.
    Off,
    /// Lower system output volume to [`DUCK_FACTOR`].
    Duck,
    /// Pause playing media via a media key.
    Pause,
}

impl Mode {
    /// Parse the stored setting string. `"duck"` / `"pause"` map to the
    /// matching variant; everything else (`"off"`, unknown, empty) is
    /// [`Mode::Off`] so a malformed doc fails safe.
    pub fn from_setting(s: &str) -> Mode {
        match s {
            "duck" => Mode::Duck,
            "pause" => Mode::Pause,
            _ => Mode::Off,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_parsing_maps_known_strings_and_falls_back_to_off() {
        assert_eq!(Mode::from_setting("duck"), Mode::Duck);
        assert_eq!(Mode::from_setting("pause"), Mode::Pause);
        assert_eq!(Mode::from_setting("off"), Mode::Off);
        // Unknown / malformed strings fail safe to Off.
        assert_eq!(Mode::from_setting(""), Mode::Off);
        assert_eq!(Mode::from_setting("DUCK"), Mode::Off);
        assert_eq!(Mode::from_setting("mute"), Mode::Off);
    }

    #[test]
    fn off_mode_is_a_pure_no_op() {
        // `begin("off", _)` returns before touching the lock or the leaf,
        // and a subsequent `restore()` with no snapshot is a no-op. The
        // `was_playing` argument is irrelevant on the Off path; pass both
        // values to make that explicit.
        begin("off", true);
        begin("off", false);
        restore();
        assert!(state().lock().unwrap().is_none());
    }

    #[test]
    fn restore_without_begin_is_idempotent_noop() {
        // Calling restore twice with no active attenuation must not
        // panic and must leave the state empty.
        restore();
        restore();
        assert!(state().lock().unwrap().is_none());
    }
}
