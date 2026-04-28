//! Frontmost-app detection (bundle id + focused window title).
//!
//! Bundle id comes from NSWorkspace; window title comes from the AX API
//! (`AXFocusedWindow` → `AXTitle`), which requires Accessibility permission.
//! Any probe failure returns `None` — fail-open, no suggestion shown.

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::{c_char, c_void, CStr, CString};

    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type AXUIElementRef = *const c_void;
    type AXError = i32;

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[allow(non_snake_case)]
    extern "C" {
        fn CFRelease(cf: *const c_void);
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            cstr: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringGetCStringPtr(s: CFStringRef, encoding: u32) -> *const c_char;
        fn CFStringGetLength(s: CFStringRef) -> isize;
        fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
        fn CFStringGetCString(
            s: CFStringRef,
            buffer: *mut c_char,
            buffer_size: isize,
            encoding: u32,
        ) -> bool;

        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> AXError;
    }

    pub fn detect() -> Option<(String, Option<String>)> {
        let (bundle_id, pid) = frontmost_id_and_pid()?;
        let title = if pid > 0 { frontmost_window_title(pid) } else { None };
        Some((bundle_id, title))
    }

    fn frontmost_id_and_pid() -> Option<(String, i32)> {
        unsafe {
            let workspace = NSWorkspace::sharedWorkspace();
            let app = workspace.frontmostApplication()?;
            let app_obj: &AnyObject = &app;
            let id_ptr: *mut AnyObject = msg_send![app_obj, bundleIdentifier];
            if id_ptr.is_null() {
                return None;
            }
            let ns: &NSString = &*(id_ptr as *const NSString);
            let bundle_id = ns.to_string();
            let pid: i32 = msg_send![app_obj, processIdentifier];
            Some((bundle_id, pid))
        }
    }

    fn frontmost_window_title(pid: i32) -> Option<String> {
        unsafe {
            let app_elem = AXUIElementCreateApplication(pid);
            if app_elem.is_null() {
                return None;
            }
            let focused_attr = make_cfstr("AXFocusedWindow")?;
            let title_attr = make_cfstr("AXTitle");

            let mut focused_window: CFTypeRef = std::ptr::null();
            let err = AXUIElementCopyAttributeValue(app_elem, focused_attr, &mut focused_window);
            CFRelease(focused_attr);
            CFRelease(app_elem);

            let title_attr = title_attr?;
            if err != 0 || focused_window.is_null() {
                CFRelease(title_attr);
                return None;
            }

            let mut title_cf: CFTypeRef = std::ptr::null();
            let err2 = AXUIElementCopyAttributeValue(focused_window, title_attr, &mut title_cf);
            CFRelease(title_attr);
            CFRelease(focused_window);

            if err2 != 0 || title_cf.is_null() {
                return None;
            }
            let result = cfstring_to_rust(title_cf);
            CFRelease(title_cf);
            result.filter(|s| !s.trim().is_empty())
        }
    }

    unsafe fn make_cfstr(s: &str) -> Option<CFStringRef> {
        let bytes = CString::new(s).ok()?;
        let cf =
            CFStringCreateWithCString(std::ptr::null(), bytes.as_ptr(), K_CF_STRING_ENCODING_UTF8);
        (!cf.is_null()).then_some(cf)
    }

    unsafe fn cfstring_to_rust(s: CFTypeRef) -> Option<String> {
        let s_ref: CFStringRef = s;
        // Zero-copy fast path.
        let ptr = CFStringGetCStringPtr(s_ref, K_CF_STRING_ENCODING_UTF8);
        if !ptr.is_null() {
            return CStr::from_ptr(ptr).to_str().ok().map(str::to_string);
        }
        let len = CFStringGetLength(s_ref);
        let max = CFStringGetMaximumSizeForEncoding(len, K_CF_STRING_ENCODING_UTF8);
        if max <= 0 {
            return None;
        }
        let buf_len = (max as usize) + 1;
        let mut buf: Vec<c_char> = vec![0; buf_len];
        if !CFStringGetCString(
            s_ref,
            buf.as_mut_ptr(),
            buf_len as isize,
            K_CF_STRING_ENCODING_UTF8,
        ) {
            return None;
        }
        CStr::from_ptr(buf.as_ptr()).to_str().ok().map(str::to_string)
    }
}

#[cfg(target_os = "macos")]
pub fn detect() -> Option<(String, Option<String>)> {
    imp::detect()
}

#[cfg(not(target_os = "macos"))]
pub fn detect() -> Option<(String, Option<String>)> {
    None
}
