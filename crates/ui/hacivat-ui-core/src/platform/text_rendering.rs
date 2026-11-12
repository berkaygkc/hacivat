//! Text rendering mode (L12, R11).
//!
//! GPUI's macOS text system turns CoreGraphics stroke dilation ("font smoothing") off only when
//! `CFPreferencesCopyAppValue("AppleFontSmoothing", kCFPreferencesCurrentApplication)` is `0`,
//! and it reads that value once, into a process-wide `OnceLock`, on the first glyph it
//! rasterizes (`gpui_apple/src/text_system.rs:224-260` at the pin). So the mode must be applied
//! before GPUI draws any text, and a change only takes effect after a restart.

use crate::app::TextRendering;

/// Whether a text rendering change takes effect without a restart. It does not (R11): GPUI
/// caches the preference on the first glyph and rasterized glyphs stay in the atlas. Settings
/// reads this to show "Applies after restart".
pub const TEXT_RENDERING_IS_LIVE: bool = false;

/// Applies `mode` to this process's own preferences domain.
///
/// - macOS: `Crisp` sets this app's `AppleFontSmoothing` to `0` (no stroke dilation, matching
///   the mock); `Standard` removes the app's override, so the user's global preference applies.
/// - Other OSes: no-op.
///
/// Call it before `gpui_platform::application()` ([`crate::app::run_app`] does).
pub fn apply_text_rendering(mode: TextRendering) {
    #[cfg(target_os = "macos")]
    mac::apply(mac::current_application(), mode);
    #[cfg(not(target_os = "macos"))]
    let _ = mode;
}

/// This app's own `AppleFontSmoothing` value (the app domain only, never the global domain),
/// or `None` when the app sets none. Always `None` off macOS.
pub fn app_font_smoothing_override() -> Option<i64> {
    #[cfg(target_os = "macos")]
    return mac::font_smoothing(mac::current_application());
    #[cfg(not(target_os = "macos"))]
    None
}

#[cfg(target_os = "macos")]
mod mac {
    use core_foundation::base::{CFType, TCFType as _};
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use core_foundation_sys::preferences::{
        CFPreferencesCopyValue, CFPreferencesSetAppValue, kCFPreferencesAnyHost,
        kCFPreferencesCurrentApplication, kCFPreferencesCurrentUser,
    };

    use crate::app::TextRendering;

    const KEY: &str = "AppleFontSmoothing";

    /// The preferences domain of this process (its bundle ID, or its executable name).
    pub(super) fn current_application() -> CFStringRef {
        // SAFETY: reading an immutable CoreFoundation constant.
        unsafe { kCFPreferencesCurrentApplication }
    }

    /// Writes `mode` into `app`'s domain: `Crisp` stores `0`, `Standard` removes the key.
    pub(super) fn apply(app: CFStringRef, mode: TextRendering) {
        set_font_smoothing(
            app,
            match mode {
                TextRendering::Crisp => Some(0),
                TextRendering::Standard => None,
            },
        );
    }

    /// `Some(n)` stores `n`; `None` removes `app`'s value.
    fn set_font_smoothing(app: CFStringRef, value: Option<i64>) {
        let key = CFString::from_static_string(KEY);
        let number = value.map(CFNumber::from);
        let value_ref = number
            .as_ref()
            .map_or(std::ptr::null(), |n| n.as_CFTypeRef());
        // SAFETY: `key`, `number` and `app` are live CF objects for the whole call
        // (CFPreferences retains what it stores); a null value is the documented way to remove
        // the key.
        unsafe { CFPreferencesSetAppValue(key.as_concrete_TypeRef(), value_ref, app) }
    }

    /// Reads `app`'s domain for the current user on any host, which is the domain
    /// `CFPreferencesSetAppValue` writes. Unlike `CFPreferencesCopyAppValue` it does not fall
    /// through to the global domain, so a user's own global setting never reads as ours.
    pub(super) fn font_smoothing(app: CFStringRef) -> Option<i64> {
        let key = CFString::from_static_string(KEY);
        // SAFETY: `key` and `app` are live CFStrings and the user/host arguments are
        // CoreFoundation constants. The call follows the Copy rule: it returns an owned
        // reference or null.
        let raw = unsafe {
            CFPreferencesCopyValue(
                key.as_concrete_TypeRef(),
                app,
                kCFPreferencesCurrentUser,
                kCFPreferencesAnyHost,
            )
        };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` is non-null and owned by us (Copy rule); `CFType` releases it on drop.
        let value = unsafe { CFType::wrap_under_create_rule(raw) };
        value.downcast_into::<CFNumber>()?.to_i64()
    }

    /// A throwaway preferences domain for tests. CFPreferences treats an absolute-path
    /// application ID as the plist file itself, so the domain lives in a private temp
    /// directory (`$TMPDIR/hacivat-prefs-test-<pid>/dev.hacivat.test.plist`), never in
    /// `~/Library/Preferences`. Dropping it (also during a panic) removes the key, synchronises
    /// and deletes the directory.
    #[cfg(test)]
    pub(super) struct ScratchDomain {
        dir: std::path::PathBuf,
        id: CFString,
    }

    #[cfg(test)]
    impl ScratchDomain {
        pub(super) fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("hacivat-prefs-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("create scratch preferences dir");
            let path = dir.join("dev.hacivat.test");
            let id = CFString::new(path.to_str().expect("temp dir is UTF-8"));
            Self { dir, id }
        }

        pub(super) fn id(&self) -> CFStringRef {
            self.id.as_concrete_TypeRef()
        }
    }

    #[cfg(test)]
    impl Drop for ScratchDomain {
        fn drop(&mut self) {
            set_font_smoothing(self.id(), None);
            // SAFETY: `self.id` is a live CFString naming the domain to flush.
            unsafe {
                core_foundation_sys::preferences::CFPreferencesAppSynchronize(self.id());
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::TextRendering;

    /// Same sequence as the production path, but in a throwaway domain (`dev.hacivat.test.<pid>`)
    /// that is removed on drop, so the test leaves nothing on the machine.
    #[cfg(target_os = "macos")]
    #[test]
    fn crisp_then_standard_toggles_the_app_override() {
        use super::mac;
        let domain = mac::ScratchDomain::new();
        let app = domain.id();
        mac::apply(app, TextRendering::Crisp);
        assert_eq!(mac::font_smoothing(app), Some(0));
        mac::apply(app, TextRendering::Standard);
        assert_eq!(mac::font_smoothing(app), None);
        mac::apply(app, TextRendering::Crisp);
        assert_eq!(mac::font_smoothing(app), Some(0));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn text_rendering_is_a_no_op_off_macos() {
        use crate::platform;
        platform::apply_text_rendering(TextRendering::Crisp);
        assert_eq!(platform::app_font_smoothing_override(), None);
        platform::apply_text_rendering(TextRendering::Standard);
        assert_eq!(platform::app_font_smoothing_override(), None);
    }
}
