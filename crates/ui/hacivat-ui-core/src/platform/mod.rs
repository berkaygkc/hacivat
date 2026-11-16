//! Direct OS calls: text rendering (L12), resident set size (F18) and the native window number.
//! This is the only module allowed to use `unsafe` (workspace lint `unsafe_code = "deny"`);
//! every unsafe block carries a `// SAFETY:` comment.
#![allow(
    unsafe_code,
    reason = "CoreFoundation preferences, Mach task info, Win32 process memory and the AppKit \
              window number need FFI; confined to `platform/**`"
)]

mod memory;
mod text_rendering;

pub use memory::resident_set_mib;
// ## m3a-t01b-seams
pub use memory::physical_memory_bytes;
pub use text_rendering::{
    TEXT_RENDERING_IS_LIVE, app_font_smoothing_override, apply_text_rendering,
};

use gpui::Window;

/// The native window number for `screencapture -l` (macOS only; `None` elsewhere). The app's
/// `--print-window-id` and the spikes use it.
#[cfg(target_os = "macos")]
pub fn native_window_id(window: &Window) -> Option<u32> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(h) = HasWindowHandle::window_handle(window).ok()?.as_raw() else {
        return None;
    };
    let view: *mut AnyObject = h.ns_view.as_ptr().cast();
    // SAFETY: `ns_view` is the live NSView of this window; `window` and `windowNumber` are
    // plain getters returning an object pointer and an NSInteger.
    unsafe {
        let win: *mut AnyObject = msg_send![view, window];
        if win.is_null() {
            return None;
        }
        let n: isize = msg_send![win, windowNumber];
        u32::try_from(n).ok()
    }
}

/// The native window number for `screencapture -l` (macOS only; `None` elsewhere).
#[cfg(not(target_os = "macos"))]
pub fn native_window_id(_window: &Window) -> Option<u32> {
    None
}
