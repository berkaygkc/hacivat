//! Application bootstrap: thin wrappers over GPUI's app and window entry points.

use gpui::{
    App, Bounds, Entity, Render, Window, WindowBounds, WindowHandle, WindowOptions, px, size,
};

/// Runs the GPUI application. Font registration and the theme global are installed here
/// by later tasks.
pub fn run_app(f: impl FnOnce(&mut App) + 'static) {
    gpui_platform::application().run(f);
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
