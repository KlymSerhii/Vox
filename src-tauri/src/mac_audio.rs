//! macOS CoreAudio leaf for the audio-attenuation facade
//! ([`crate::audio_attenuation`]).
//!
//! Exposes the five contract functions the facade dispatches to —
//! `output_volume` / `set_output_volume` / `output_is_active` /
//! `supports_play_pause` / `send_play_pause` — implemented against
//! CoreAudio's `AudioObject*` C ABI plus a synthetic `NX_KEYTYPE_PLAY`
//! media key for the pause path. The Windows engineer's `win_audio.rs`
//! and the non-OS stub in `audio_attenuation` expose the same five
//! signatures.
//!
//! Structure mirrors [`crate::mac_power`] exactly: a file-internal
//! `#[cfg(target_os = "macos")] mod imp` holding the raw `extern "C"`
//! FFI, a `#[cfg(not(target_os = "macos"))] mod imp` stub, and thin
//! `pub fn`s delegating to `imp::`. Raw FFI (no `coreaudio-sys`) keeps
//! the link surface identical to `mac_paste` / `mac_power`.
//!
//! Every function is panic-safe and returns `None` / `false` on any
//! failure — the facade treats those as "couldn't attenuate" and
//! fails open so dictation is never blocked.

#[cfg(target_os = "macos")]
mod imp {
    use std::os::raw::c_void;

    // --- CoreAudio C ABI handle / value aliases ---------------------
    type AudioObjectID = u32;
    type OSStatus = i32;
    /// FourCharCode-typed selector / scope / element fields.
    type AudioObjectPropertySelector = u32;
    type AudioObjectPropertyScope = u32;
    type AudioObjectPropertyElement = u32;

    const NO_ERR: OSStatus = 0;

    /// `kAudioObjectSystemObject` — the well-known id of the global
    /// audio system object you query for the default device.
    const K_AUDIO_OBJECT_SYSTEM_OBJECT: AudioObjectID = 1;

    // FourCharCodes. Each is the big-endian packing of its 4-char
    // literal, exactly as the CoreAudio headers define them.
    /// `kAudioHardwarePropertyDefaultOutputDevice` = 'dOut'.
    const K_DEFAULT_OUTPUT_DEVICE: AudioObjectPropertySelector =
        u32::from_be_bytes(*b"dOut");
    /// `kAudioObjectPropertyScopeGlobal` = 'glob'.
    const K_SCOPE_GLOBAL: AudioObjectPropertyScope = u32::from_be_bytes(*b"glob");
    /// `kAudioObjectPropertyScopeOutput` = 'outp'.
    const K_SCOPE_OUTPUT: AudioObjectPropertyScope = u32::from_be_bytes(*b"outp");
    /// `kAudioObjectPropertyElementMain` (formerly `…Master`) = 0.
    const K_ELEMENT_MAIN: AudioObjectPropertyElement = 0;
    /// `kAudioHardwareServiceDeviceProperty_VirtualMainVolume` = 'vmvc'.
    /// The 0.0..=1.0 scalar main-output volume.
    const K_VIRTUAL_MAIN_VOLUME: AudioObjectPropertySelector =
        u32::from_be_bytes(*b"vmvc");
    /// `kAudioDevicePropertyDeviceIsRunningSomewhere` = 'gone'.
    /// Non-zero when *any* process is actively doing IO on the device.
    const K_DEVICE_IS_RUNNING_SOMEWHERE: AudioObjectPropertySelector =
        u32::from_be_bytes(*b"gone");

    /// `AudioObjectPropertyAddress` — the C struct the CoreAudio
    /// property API keys every get/set on. `#[repr(C)]` so the field
    /// order matches the header.
    #[repr(C)]
    struct AudioObjectPropertyAddress {
        m_selector: AudioObjectPropertySelector,
        m_scope: AudioObjectPropertyScope,
        m_element: AudioObjectPropertyElement,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectHasProperty(
            in_object_id: AudioObjectID,
            in_address: *const AudioObjectPropertyAddress,
        ) -> bool;
        fn AudioObjectIsPropertySettable(
            in_object_id: AudioObjectID,
            in_address: *const AudioObjectPropertyAddress,
            out_is_settable: *mut bool,
        ) -> OSStatus;
        fn AudioObjectGetPropertyData(
            in_object_id: AudioObjectID,
            in_address: *const AudioObjectPropertyAddress,
            in_qualifier_data_size: u32,
            in_qualifier_data: *const c_void,
            io_data_size: *mut u32,
            out_data: *mut c_void,
        ) -> OSStatus;
        fn AudioObjectSetPropertyData(
            in_object_id: AudioObjectID,
            in_address: *const AudioObjectPropertyAddress,
            in_qualifier_data_size: u32,
            in_qualifier_data: *const c_void,
            in_data_size: u32,
            in_data: *const c_void,
        ) -> OSStatus;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        /// Non-prompting Accessibility-trust probe. Same symbol
        /// `mac_paste` links — this is the gate for posting the
        /// synthetic media key at the HID event tap.
        fn AXIsProcessTrusted() -> bool;
    }

    // --- CoreGraphics: posting the synthetic media key -------------
    // The media Play/Pause key is an `NSSystemDefined` event. We build
    // it through AppKit's NSEvent (its `otherEventWithType:…`
    // constructor is the only public way to mint a system-defined
    // event), pull the backing `CGEventRef`, and post that at the HID
    // tap — the same tap `mac_paste` posts ⌘V through.
    type CGEventRef = *const c_void;

    /// `kCGHIDEventTap` — bottom of the tap stack; the event flows up
    /// to whichever app owns media playback.
    const KCG_HID_EVENT_TAP: u32 = 0;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventPost(tap: u32, event: CGEventRef);
    }

    /// Read the system default OUTPUT device id, or `None` on failure.
    fn default_output_device() -> Option<AudioObjectID> {
        let addr = AudioObjectPropertyAddress {
            m_selector: K_DEFAULT_OUTPUT_DEVICE,
            m_scope: K_SCOPE_GLOBAL,
            m_element: K_ELEMENT_MAIN,
        };
        let mut dev: AudioObjectID = 0;
        let mut size = std::mem::size_of::<AudioObjectID>() as u32;
        // SAFETY: well-known system object id + a stack-allocated
        // address/out-param; sizes match the C types exactly.
        let status = unsafe {
            AudioObjectGetPropertyData(
                K_AUDIO_OBJECT_SYSTEM_OBJECT,
                &addr,
                0,
                std::ptr::null(),
                &mut size,
                &mut dev as *mut AudioObjectID as *mut c_void,
            )
        };
        if status == NO_ERR && dev != 0 {
            Some(dev)
        } else {
            None
        }
    }

    pub fn output_volume() -> Option<f32> {
        let dev = default_output_device()?;
        let addr = AudioObjectPropertyAddress {
            m_selector: K_VIRTUAL_MAIN_VOLUME,
            m_scope: K_SCOPE_OUTPUT,
            m_element: K_ELEMENT_MAIN,
        };
        // Not every device exposes a settable virtual main volume (e.g.
        // some aggregate / digital-out devices). Probe first so we
        // return None — and the facade skips ducking — rather than
        // reading garbage.
        // SAFETY: stack address, valid device id.
        if !unsafe { AudioObjectHasProperty(dev, &addr) } {
            return None;
        }
        let mut vol: f32 = 0.0;
        let mut size = std::mem::size_of::<f32>() as u32;
        // SAFETY: out buffer is a live f32; size matches.
        let status = unsafe {
            AudioObjectGetPropertyData(
                dev,
                &addr,
                0,
                std::ptr::null(),
                &mut size,
                &mut vol as *mut f32 as *mut c_void,
            )
        };
        if status == NO_ERR {
            Some(vol.clamp(0.0, 1.0))
        } else {
            None
        }
    }

    pub fn set_output_volume(v: f32) -> bool {
        let Some(dev) = default_output_device() else {
            return false;
        };
        let addr = AudioObjectPropertyAddress {
            m_selector: K_VIRTUAL_MAIN_VOLUME,
            m_scope: K_SCOPE_OUTPUT,
            m_element: K_ELEMENT_MAIN,
        };
        // SAFETY: stack address, valid device id.
        if !unsafe { AudioObjectHasProperty(dev, &addr) } {
            return false;
        }
        let mut settable = false;
        // SAFETY: out bool is live; address valid.
        let s = unsafe { AudioObjectIsPropertySettable(dev, &addr, &mut settable) };
        if s != NO_ERR || !settable {
            return false;
        }
        let value: f32 = v.clamp(0.0, 1.0);
        let size = std::mem::size_of::<f32>() as u32;
        // SAFETY: in buffer is a live f32; size matches the C type.
        let status = unsafe {
            AudioObjectSetPropertyData(
                dev,
                &addr,
                0,
                std::ptr::null(),
                size,
                &value as *const f32 as *const c_void,
            )
        };
        status == NO_ERR
    }

    pub fn output_is_active() -> bool {
        let Some(dev) = default_output_device() else {
            return false;
        };
        let addr = AudioObjectPropertyAddress {
            m_selector: K_DEVICE_IS_RUNNING_SOMEWHERE,
            m_scope: K_SCOPE_GLOBAL,
            m_element: K_ELEMENT_MAIN,
        };
        // SAFETY: stack address, valid device id.
        if !unsafe { AudioObjectHasProperty(dev, &addr) } {
            return false;
        }
        let mut running: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: out buffer is a live u32; size matches.
        let status = unsafe {
            AudioObjectGetPropertyData(
                dev,
                &addr,
                0,
                std::ptr::null(),
                &mut size,
                &mut running as *mut u32 as *mut c_void,
            )
        };
        status == NO_ERR && running != 0
    }

    pub fn supports_play_pause() -> bool {
        // SAFETY: non-prompting, side-effect-free.
        unsafe { AXIsProcessTrusted() }
    }

    /// `NSEventTypeSystemDefined` — the event type that carries media /
    /// special keys.
    const NS_EVENT_TYPE_SYSTEM_DEFINED: u64 = 14;
    /// Subtype `8` (`NX_SUBTYPE_AUX_CONTROL_BUTTONS`) marks the
    /// system-defined event as an aux/media key.
    const NX_SUBTYPE_AUX_CONTROL_BUTTONS: i16 = 8;
    /// `NX_KEYTYPE_PLAY` — the Play/Pause media key code.
    const NX_KEYTYPE_PLAY: i32 = 16;
    /// data1 low-word flags: 0xA00 = key-down, 0xB00 = key-up. The
    /// high word carries the key code.
    const FLAG_KEY_DOWN: i32 = 0xA00;
    const FLAG_KEY_UP: i32 = 0xB00;

    /// Build one media-key NSEvent (down or up) and post its backing
    /// CGEvent at the HID tap. Returns false on any failure.
    fn post_media_key(down: bool) -> bool {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        use objc2::ClassType;
        use objc2_app_kit::NSEvent;
        use objc2_foundation::NSPoint;

        let flags = if down { FLAG_KEY_DOWN } else { FLAG_KEY_UP };
        let data1: i64 = ((NX_KEYTYPE_PLAY << 16) | flags) as i64;

        // SAFETY: `otherEventWithType:…` is a documented AppKit class
        // constructor returning an autoreleased NSEvent (or nil). We
        // ask it for its CGEvent (a borrow owned by the NSEvent, valid
        // for this scope) and post that. All pointers are checked for
        // null before use; nothing is released here (autorelease owns
        // the NSEvent, the CGEvent is a borrow).
        unsafe {
            let class = NSEvent::class();
            let event: *mut NSEvent = msg_send![
                class,
                otherEventWithType: NS_EVENT_TYPE_SYSTEM_DEFINED,
                location: NSPoint { x: 0.0, y: 0.0 },
                modifierFlags: 0u64,
                timestamp: 0.0f64,
                windowNumber: 0i64,
                context: std::ptr::null_mut::<AnyObject>(),
                subtype: NX_SUBTYPE_AUX_CONTROL_BUTTONS,
                data1: data1,
                data2: -1i64,
            ];
            if event.is_null() {
                return false;
            }
            let cg: CGEventRef = msg_send![event, CGEvent];
            if cg.is_null() {
                return false;
            }
            CGEventPost(KCG_HID_EVENT_TAP, cg);
            true
        }
    }

    pub fn send_play_pause() -> bool {
        // A media key is a down + up pair, exactly like a physical
        // press. Sending only the down leaves some apps latched.
        let down = post_media_key(true);
        let up = post_media_key(false);
        down && up
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    // Stubs so callers in the OS-agnostic facade compile on every host.
    // The facade only dispatches here via cfg when the target is macOS,
    // but the symbols must exist regardless of how the file is compiled.
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
        true
    }
    pub fn send_play_pause() -> bool {
        false
    }
}

/// Current default-output-device volume, `0.0..=1.0`. `None` if
/// unavailable / not settable.
pub fn output_volume() -> Option<f32> {
    imp::output_volume()
}

/// Set the default-output-device volume (clamped to `0..=1`). Returns
/// `true` if applied.
pub fn set_output_volume(v: f32) -> bool {
    imp::set_output_volume(v)
}

/// Is audio currently playing on the default output device?
pub fn output_is_active() -> bool {
    imp::output_is_active()
}

/// Can we post a media Play/Pause key right now? (macOS:
/// `AXIsProcessTrusted()`.)
pub fn supports_play_pause() -> bool {
    imp::supports_play_pause()
}

/// Post a media Play/Pause toggle to the system. Returns `true` if sent.
pub fn send_play_pause() -> bool {
    imp::send_play_pause()
}
