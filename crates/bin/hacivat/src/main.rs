//! The Hacivat executable. Task 13 adds settings, logging, localization and the workspace
//! shell; until then it opens an empty window so CI and packaging can target the crate.

use hacivat_ui_core::app::{Boot, open_window, run_app};
use hacivat_ui_core::gpui_surface::{AppContext, Context, IntoElement, Render, Window, div};

struct Empty;

impl Render for Empty {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn main() {
    run_app(Boot::default(), |cx| {
        hacivat_app::init(cx);
        // The brand is never translated; the window title is not chrome text.
        if let Err(err) = open_window(cx, "Hacivat", (1440.0, 900.0), |_, cx| cx.new(|_| Empty)) {
            eprintln!("opening the window failed: {err:#}");
            std::process::exit(1);
        }
    });
}
