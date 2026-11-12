//! Application bootstrap: thin wrappers over GPUI's app and window entry points.

use anyhow::Context as _;
use gpui::{
    App, Bounds, Entity, Render, Window, WindowBounds, WindowHandle, WindowOptions, px, size,
};

/// Process exit code when the bundled fonts cannot be registered or do not resolve.
pub const FONT_FAILURE_EXIT: i32 = 3;

/// Registers the bundled fonts and verifies GPUI resolves them (no silent system-font fallback).
pub fn bootstrap(cx: &mut App) -> anyhow::Result<()> {
    crate::fonts::register_fonts(cx).context("registering bundled fonts")?;
    crate::shell::verify_bundled_fonts(cx).context("verifying bundled fonts")
}

/// The one application bootstrap every spike uses: installs the bundled icon assets, registers
/// and verifies the bundled fonts, then runs `f`. A font failure exits the process with
/// [`FONT_FAILURE_EXIT`] before any window opens, so nothing renders with fallback fonts.
pub fn run_app(f: impl FnOnce(&mut App) + 'static) {
    gpui_platform::application()
        .with_assets(crate::shell::BundledAssets)
        .run(move |cx| {
            if let Err(err) = bootstrap(cx) {
                eprintln!("bundled fonts unavailable: {err:#}");
                std::process::exit(FONT_FAILURE_EXIT);
            }
            f(cx);
        });
}

/// Opens a centered window with the given title and logical size.
pub fn open_window<V: Render + 'static>(
    cx: &mut App,
    title: &str,
    size_px: (f32, f32),
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<WindowHandle<V>> {
    let bounds = Bounds::centered(None, size(px(size_px.0), px(size_px.1)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(title.to_string().into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    cx.open_window(options, build)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_fails_when_bundled_fonts_do_not_resolve() {
        // The test platform's text system registers nothing, which is exactly the silent
        // fallback the bootstrap must refuse.
        let cx = gpui::TestAppContext::single();
        let err = cx.update(bootstrap).unwrap_err();
        assert!(format!("{err:#}").contains("not registered"), "{err:#}");
    }

    #[test]
    fn font_failure_exit_code_is_non_zero() {
        assert_ne!(FONT_FAILURE_EXIT, 0);
    }
}
