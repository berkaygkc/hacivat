//! The workspace window and its per-OS chrome. `ChromeKind` and `TRAFFIC_LIGHTS_INSET_PX` are
//! complete; Task 8 fills in the per-OS window options and `chrome_kind`.

use gpui::{
    App, Bounds, Entity, Render, SharedString, Window, WindowBounds, WindowHandle, WindowOptions,
    px, size,
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

/// Opens the workspace window. Stub (Task 8 adds the per-OS chrome): a centered, titled window
/// of `spec.size` with `spec.min_size` as its minimum.
pub fn open_workspace_window<V: Render + 'static>(
    cx: &mut App,
    spec: WorkspaceWindowSpec,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<WindowHandle<V>> {
    let bounds = Bounds::centered(None, size(px(spec.size.0), px(spec.size.1)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(spec.min_size.0), px(spec.min_size.1))),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(spec.title),
            ..Default::default()
        }),
        ..Default::default()
    };
    cx.open_window(options, build)
}

/// The chrome `window` was opened with. Stub (Task 8 reads the window's decorations): the
/// platform default, `LinuxClient` on Linux.
pub fn chrome_kind(window: &Window) -> ChromeKind {
    let _ = window;
    if cfg!(target_os = "macos") {
        ChromeKind::MacTrafficLights
    } else if cfg!(target_os = "windows") {
        ChromeKind::WindowsCaption
    } else {
        ChromeKind::LinuxClient
    }
}
