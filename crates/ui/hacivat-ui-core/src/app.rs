//! Application bootstrap: thin wrappers over GPUI's app and window entry points.

use anyhow::Context as _;
use gpui::{
    App, Bounds, Entity, Render, Window, WindowBounds, WindowHandle, WindowOptions, px, size,
};

use crate::assets::BundledAssets;
use crate::platform;
use crate::theme::{Theme, ThemeMode};

/// Process exit code when the bundled fonts cannot be registered or do not resolve.
pub const FONT_FAILURE_EXIT: i32 = 3;

/// How text is rasterized (L12). `Crisp` turns macOS font smoothing off for this app;
/// `Standard` leaves the user's system preference in place. Applies on restart only (R11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextRendering {
    #[default]
    Crisp,
    Standard,
}

/// Process-level choices that must be made before GPUI starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Boot {
    pub text_rendering: TextRendering,
}

/// Registers the bundled fonts and verifies GPUI resolves them (no silent system-font fallback).
pub fn bootstrap(cx: &mut App) -> anyhow::Result<()> {
    crate::fonts::register_fonts(cx).context("registering bundled fonts")?;
    crate::fonts::verify_bundled_fonts(cx).context("verifying bundled fonts")
}

/// The one application bootstrap, used by the app and every spike:
///
/// 1. applies `boot.text_rendering` ([`platform::apply_text_rendering`]) before GPUI starts,
///    because GPUI reads the font-smoothing preference once, on the first glyph (R11);
/// 2. installs the bundled icon assets;
/// 3. registers and verifies the bundled fonts; a failure exits the process with
///    [`FONT_FAILURE_EXIT`] before any window opens, so nothing renders with fallback fonts;
/// 4. installs the `Dark` theme (callers may replace it);
/// 5. runs `f`.
pub fn run_app(boot: Boot, f: impl FnOnce(&mut App) + 'static) {
    platform::apply_text_rendering(boot.text_rendering);
    gpui_platform::application()
        .with_assets(BundledAssets)
        .run(move |cx| {
            if let Err(err) = bootstrap(cx) {
                eprintln!("bundled fonts unavailable: {err:#}");
                std::process::exit(FONT_FAILURE_EXIT);
            }
            Theme::install(cx, ThemeMode::Dark);
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

    #[test]
    fn boot_default_is_crisp() {
        assert_eq!(Boot::default().text_rendering, TextRendering::Crisp);
    }
}
