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
    mac::set_font_smoothing(match mode {
        TextRendering::Crisp => Some(0),
        TextRendering::Standard => None,
    });
    #[cfg(not(target_os = "macos"))]
    let _ = mode;
}

/// This app's own `AppleFontSmoothing` value (the app domain only, never the global domain),
/// or `None` when the app sets none. Always `None` off macOS.
pub fn app_font_smoothing_override() -> Option<i64> {
    #[cfg(target_os = "macos")]
    return mac::app_font_smoothing();
    #[cfg(not(target_os = "macos"))]
    None
}

#[cfg(target_os = "macos")]
mod mac {
    use core_foundation::base::{CFType, TCFType as _};
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_foundation_sys::preferences::{
        CFPreferencesCopyValue, CFPreferencesSetAppValue, kCFPreferencesAnyHost,
        kCFPreferencesCurrentApplication, kCFPreferencesCurrentUser,
    };

    const KEY: &str = "AppleFontSmoothing";

    /// `Some(n)` stores `n`; `None` removes the app's value.
    pub(super) fn set_font_smoothing(value: Option<i64>) {
        let key = CFString::from_static_string(KEY);
        let number = value.map(CFNumber::from);
        let value_ref = number
            .as_ref()
            .map_or(std::ptr::null(), |n| n.as_CFTypeRef());
        // SAFETY: `key` and `number` are live CF objects that outlive the call (CFPreferences
        // retains what it stores); a null value is the documented way to remove the key; the
        // `kCFPreferences*` statics are CoreFoundation constants.
        unsafe {
            CFPreferencesSetAppValue(
                key.as_concrete_TypeRef(),
                value_ref,
                kCFPreferencesCurrentApplication,
            );
        }
    }

    /// Reads the current application's domain for the current user on any host, which is the
    /// domain `CFPreferencesSetAppValue` writes. Unlike `CFPreferencesCopyAppValue` it does not
    /// fall through to the global domain, so a user's own global setting never reads as ours.
    pub(super) fn app_font_smoothing() -> Option<i64> {
        let key = CFString::from_static_string(KEY);
        // SAFETY: `key` is a live CFString and the domain arguments are CoreFoundation
        // constants. The call follows the Copy rule: it returns an owned reference or null.
        let raw = unsafe {
            CFPreferencesCopyValue(
                key.as_concrete_TypeRef(),
                kCFPreferencesCurrentApplication,
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
}

#[cfg(test)]
mod tests {
    use crate::app::TextRendering;
    use crate::platform;

    #[cfg(target_os = "macos")]
    #[test]
    fn crisp_then_standard_toggles_the_app_override() {
        platform::apply_text_rendering(TextRendering::Crisp);
        assert_eq!(platform::app_font_smoothing_override(), Some(0));
        platform::apply_text_rendering(TextRendering::Standard);
        assert_eq!(platform::app_font_smoothing_override(), None);
        platform::apply_text_rendering(TextRendering::Crisp);
        assert_eq!(platform::app_font_smoothing_override(), Some(0));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn text_rendering_is_a_no_op_off_macos() {
        platform::apply_text_rendering(TextRendering::Crisp);
        assert_eq!(platform::app_font_smoothing_override(), None);
        platform::apply_text_rendering(TextRendering::Standard);
        assert_eq!(platform::app_font_smoothing_override(), None);
    }
}
