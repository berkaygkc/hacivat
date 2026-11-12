//! Shared test host: renders widgets inside a real view, as the app does (stateful widgets
//! need a current view).
#![allow(dead_code)]

use gpui::{
    AnyElement, App, Context, IntoElement, Render, TestAppContext, VisualTestContext, Window,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};

type Build = Box<dyn Fn(&mut Window, &mut App) -> AnyElement>;

pub struct Host {
    build: Build,
}

impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        (self.build)(window, cx)
    }
}

/// Installs the dark theme and opens a window whose root view renders `build`.
pub fn host(
    cx: &mut TestAppContext,
    build: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
) -> &mut VisualTestContext {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (_, vcx) = cx.add_window_view(|_, _| Host {
        build: Box::new(build),
    });
    vcx
}
