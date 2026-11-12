//! The workspace window opens at its spec size with the per-OS chrome (Task 8).

use gpui::{AppContext as _, Empty, TestAppContext, WindowHandle, px, size};
use hacivat_ui_core::window::{
    ChromeKind, WorkspaceWindowSpec, chrome_kind, open_workspace_window,
};

fn open_spec(
    cx: &mut TestAppContext,
    spec: WorkspaceWindowSpec,
) -> anyhow::Result<WindowHandle<Empty>> {
    cx.update(|cx| open_workspace_window(cx, spec, |_, cx| cx.new(|_| Empty)))
}

#[gpui::test]
fn workspace_window_opens_with_min_size(cx: &mut TestAppContext) {
    let w = open_spec(cx, WorkspaceWindowSpec::default()).unwrap();
    w.update(cx, |_, window, _| {
        assert_eq!(window.bounds().size, size(px(1440.), px(900.)))
    })
    .unwrap();
}

#[gpui::test]
fn chrome_kind_per_os(cx: &mut TestAppContext) {
    let w = open_spec(cx, WorkspaceWindowSpec::default()).unwrap();
    w.update(cx, |_, window, _| {
        let k = chrome_kind(window);
        #[cfg(target_os = "macos")]
        assert_eq!(k, ChromeKind::MacTrafficLights);
        #[cfg(target_os = "windows")]
        assert_eq!(k, ChromeKind::WindowsCaption);
        #[cfg(target_os = "linux")]
        assert!(
            matches!(k, ChromeKind::LinuxClient | ChromeKind::LinuxServer),
            "{k:?}"
        );
    })
    .unwrap();
}
