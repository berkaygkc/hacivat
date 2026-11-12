use gpui::{AppContext as _, Empty};

#[gpui::test]
fn opens_a_window(cx: &mut gpui::TestAppContext) {
    let handle = cx.update(|cx| {
        hacivat_ui_core::app::open_window(cx, "smoke", (800.0, 600.0), |_, cx| cx.new(|_| Empty))
    });
    assert!(handle.is_ok());
}
