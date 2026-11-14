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

// ## m3a-t01b-seams

/// The drawn runs that spell `label` (P15): the one run with exactly that text, else the
/// consecutive runs whose texts join into it. On Linux and Windows a `PopupMenu` draws an item
/// as runs around its access-key letter ("b" + "astion"); a test that finds a menu or button
/// label by its drawn text uses this. `None` when nothing spells it.
pub fn find_label_runs(
    frame: &[hacivat_ui::text::RenderedText],
    label: &str,
) -> Option<Vec<hacivat_ui::text::RenderedText>> {
    if let Some(r) = frame.iter().find(|r| r.text.as_ref() == label) {
        return Some(vec![r.clone()]);
    }
    for i in 0..frame.len() {
        let mut spelled = String::new();
        for j in i..frame.len() {
            spelled.push_str(frame[j].text.as_ref());
            if spelled == label {
                return Some(frame[i..=j].to_vec());
            }
            if !label.starts_with(spelled.as_str()) {
                break;
            }
        }
    }
    None
}
