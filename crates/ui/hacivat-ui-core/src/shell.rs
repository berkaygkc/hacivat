//! Spike shell: bundled SVG icons (installed by `app::run_app`), a unified (full-size content)
//! window for visual-fidelity captures, a font-resolution guard and the native window id for
//! `screencapture`.

use std::borrow::Cow;

use anyhow::{Result, anyhow, bail};
use gpui::{
    App, AssetSource, Bounds, Entity, FontWeight, Render, SharedString, Window,
    WindowBounds, WindowHandle, WindowOptions, point, px, size,
};

use crate::fonts::{MONO_FAMILY, UI_FAMILY};

macro_rules! lucide {
    ($($n:literal),* $(,)?) => {
        &[$((
            concat!("icons/lucide/", $n, ".svg"),
            include_bytes!(concat!("../assets/icons/lucide/", $n, ".svg")) as &[u8],
        )),*]
    };
}

/// Lucide v0.460.0 (ISC) icons the harman mock uses, plus the Hacivat placeholder mark.
const ICONS: &[(&str, &[u8])] = lucide![
    "lock", "arrow-left-right", "chevrons-up-down", "search", "layout-panel-left", "moon", "sun",
    "database", "cylinder", "layers", "table-properties", "table-2", "eye", "square-function",
    "list-ordered", "radio", "chevron-right", "chevron-down", "chevron-left", "file-code-2", "x",
    "play", "list-tree", "align-left", "columns-2", "calendar", "brackets", "key-round", "check",
    "filter", "arrow-down", "sigma", "copy", "download", "trash-2", "file-diff", "undo-2",
    "panel-right", "shield-check", "languages", "git-commit-horizontal",
];
const HV_MARK: &[u8] = include_bytes!("../assets/icons/hacivat/hv-mark.svg");

/// Serves the bundled icons under `icons/lucide/<name>.svg` and `icons/hacivat/hv-mark.svg`.
pub struct BundledAssets;

impl AssetSource for BundledAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == "icons/hacivat/hv-mark.svg" {
            return Ok(Some(Cow::Borrowed(HV_MARK)));
        }
        Ok(ICONS.iter().find(|(p, _)| *p == path).map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}

/// Fails unless the bundled faces are what GPUI resolves: both families are registered, the
/// Semibold weight resolves to a different face than Regular, and the mono family really is
/// monospaced while the UI family is not (a silent system-font fallback would break one of these).
pub fn verify_bundled_fonts(cx: &App) -> Result<()> {
    let ts = cx.text_system();
    let names = ts.all_font_names();
    for fam in [UI_FAMILY, MONO_FAMILY] {
        if !names.iter().any(|n| n == fam) {
            bail!("bundled font family {fam:?} is not registered with GPUI");
        }
    }
    let face = |family: &str, w: f32| {
        let mut f = gpui::font(family.to_string());
        f.weight = FontWeight(w);
        ts.resolve_font(&f)
    };
    if face(UI_FAMILY, 400.0) == face(UI_FAMILY, 600.0) {
        bail!("{UI_FAMILY}: weight 400 and 600 resolve to the same face");
    }
    let adv = |id, c| {
        ts.advance(id, px(100.0), c).map(|s| f32::from(s.width)).map_err(|e| anyhow!("{e}"))
    };
    let mono = face(MONO_FAMILY, 400.0);
    if (adv(mono, 'i')? - adv(mono, 'M')?).abs() > 0.5 {
        bail!("{MONO_FAMILY} did not resolve to a monospaced face");
    }
    let ui = face(UI_FAMILY, 400.0);
    if (adv(ui, 'i')? - adv(ui, 'M')?).abs() < 5.0 {
        bail!("{UI_FAMILY} did not resolve to a proportional face");
    }
    Ok(())
}

/// Logical sizes of the attached displays, in `cx.displays()` order (for `--display` selection).
pub fn display_sizes(cx: &App) -> Vec<(f32, f32)> {
    cx.displays()
        .iter()
        .map(|d| {
            let b = d.bounds();
            (f32::from(b.size.width), f32::from(b.size.height))
        })
        .collect()
}

/// Opens a window whose content fills the whole frame (transparent titlebar, native traffic
/// lights inset at `lights`), at an exact logical size.
pub fn open_unified_window<V: Render + 'static>(
    cx: &mut App,
    title: &str,
    size_px: (f32, f32),
    lights: (f32, f32),
    display: Option<usize>,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Result<WindowHandle<V>> {
    // GPUI exposes no per-display scale factor, so the caller picks the display by index into
    // `cx.displays()`; `None` uses the primary display.
    let display_id = display.and_then(|i| cx.displays().get(i).map(|d| d.id()));
    let bounds = Bounds::centered(display_id, size(px(size_px.0), px(size_px.1)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id,
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(title.to_string().into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(lights.0), px(lights.1))),
        }),
        ..Default::default()
    };
    cx.open_window(options, build)
}

/// The native window number for `screencapture -l` (macOS only).
#[cfg(target_os = "macos")]
#[allow(
    unsafe_code,
    reason = "Objective-C getters on the live NSView; Task 4 moves this into `platform/**`"
)]
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

#[cfg(not(target_os = "macos"))]
pub fn native_window_id(_window: &Window) -> Option<u32> {
    None
}
