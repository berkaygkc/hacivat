//! Spike-only helpers (feature `spike-support`): display selection and a unified (full-size
//! content) window for visual-fidelity captures. Product code never enables this feature.

use anyhow::Result;
use gpui::{
    App, Bounds, Entity, Render, Window, WindowBounds, WindowHandle, WindowOptions, point, px, size,
};

/// The native window number for `screencapture -l`; lives in [`crate::platform`] because the
/// app's `--print-window-id` uses it too.
pub use crate::platform::native_window_id;

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
