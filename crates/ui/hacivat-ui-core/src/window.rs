//! The workspace window and its per-OS chrome: a transparent macOS title bar with the native
//! traffic lights inset, a custom Windows caption, client-side decorations on Linux.

use gpui::{
    App, Bounds, Decorations, Entity, Pixels, Render, SharedString, TitlebarOptions, Window,
    WindowBounds, WindowDecorations, WindowHandle, WindowOptions, point, px, size,
};

/// Who draws the window chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChromeKind {
    /// macOS: transparent title bar with the native traffic lights inset at (14, 13).
    MacTrafficLights,
    /// Windows: custom caption drawn by the title bar (`WindowControlArea`).
    WindowsCaption,
    /// Linux with client-side decorations: the title bar draws the window controls.
    LinuxClient,
    /// Linux with server-side decorations: the compositor draws them.
    LinuxServer,
}

/// Width reserved at the left of the macOS title bar for the native traffic lights: the mock's
/// 3 × 12 px lights, 2 × 8 px gaps and its 18 px margin (from `spike-harman`'s title bar).
pub const TRAFFIC_LIGHTS_INSET_PX: f32 = 52.0 + 18.0;

/// Where the native traffic lights sit in the 40 px title bar: the mock's 14 px left padding,
/// vertically centred (the position `spike-harman` passed visual fidelity with).
const TRAFFIC_LIGHTS_POSITION_PX: (f32, f32) = (14.0, 13.0);

const MACOS: bool = cfg!(target_os = "macos");
const WINDOWS: bool = cfg!(target_os = "windows");

/// What the workspace window opens with. Default: 1440 × 900, minimum 960 × 600.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceWindowSpec {
    pub title: SharedString,
    pub size: (f32, f32),
    pub min_size: (f32, f32),
}

impl Default for WorkspaceWindowSpec {
    fn default() -> Self {
        Self {
            title: SharedString::default(),
            size: (1440.0, 900.0),
            min_size: (960.0, 600.0),
        }
    }
}

/// Opens the workspace window, centred on the primary display at `spec.size` with
/// `spec.min_size` as its minimum, with the chrome [`chrome_kind`] reports:
///
/// - macOS: transparent title bar, native traffic lights at (14, 13); the app's title bar
///   reserves [`TRAFFIC_LIGHTS_INSET_PX`] for them;
/// - Windows: transparent title bar; the app's title bar draws the caption and marks its
///   buttons with `WindowControlArea`;
/// - Linux: client-side decorations requested; the compositor may refuse
///   ([`ChromeKind::LinuxServer`]).
pub fn open_workspace_window<V: Render + 'static>(
    cx: &mut App,
    spec: WorkspaceWindowSpec,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<WindowHandle<V>> {
    let bounds = Bounds::centered(None, size(px(spec.size.0), px(spec.size.1)), cx);
    cx.open_window(workspace_window_options(spec, bounds), build)
}

/// The per-OS window options for `spec` at `bounds` (all branches compile on every OS).
fn workspace_window_options(spec: WorkspaceWindowSpec, bounds: Bounds<Pixels>) -> WindowOptions {
    let (x, y) = TRAFFIC_LIGHTS_POSITION_PX;
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(spec.min_size.0), px(spec.min_size.1))),
        titlebar: Some(TitlebarOptions {
            title: Some(spec.title),
            appears_transparent: MACOS || WINDOWS,
            traffic_light_position: MACOS.then(|| point(px(x), px(y))),
        }),
        window_decorations: (!MACOS && !WINDOWS).then_some(WindowDecorations::Client),
        ..Default::default()
    }
}

/// Who draws `window`'s chrome. Fixed on macOS and Windows; on Linux it follows the decorations
/// the compositor granted (client-side was requested, server-side is the fallback).
pub fn chrome_kind(window: &Window) -> ChromeKind {
    if MACOS {
        ChromeKind::MacTrafficLights
    } else if WINDOWS {
        ChromeKind::WindowsCaption
    } else {
        match window.window_decorations() {
            Decorations::Client { .. } => ChromeKind::LinuxClient,
            Decorations::Server => ChromeKind::LinuxServer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> WindowOptions {
        let spec = WorkspaceWindowSpec {
            title: "t".into(),
            ..Default::default()
        };
        let bounds = Bounds {
            origin: gpui::point(px(0.), px(0.)),
            size: size(px(1440.), px(900.)),
        };
        workspace_window_options(spec, bounds)
    }

    #[test]
    fn options_carry_spec_size_minimum_and_title() {
        let o = options();
        assert!(
            matches!(o.window_bounds, Some(WindowBounds::Windowed(b)) if b.size == size(px(1440.), px(900.)))
        );
        assert_eq!(o.window_min_size, Some(size(px(960.), px(600.))));
        assert_eq!(o.titlebar.and_then(|t| t.title), Some("t".into()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_has_a_transparent_title_bar_with_inset_traffic_lights() {
        let o = options();
        let t = o.titlebar.unwrap();
        assert!(t.appears_transparent);
        assert_eq!(
            t.traffic_light_position,
            Some(gpui::point(px(14.), px(13.)))
        );
        assert_eq!(o.window_decorations, None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_has_a_transparent_title_bar_for_the_custom_caption() {
        let o = options();
        assert_eq!(o.window_decorations, None);
        assert!(o.titlebar.unwrap().appears_transparent);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn linux_requests_client_side_decorations() {
        assert_eq!(
            options().window_decorations,
            Some(gpui::WindowDecorations::Client)
        );
    }
}
