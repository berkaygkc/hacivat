//! The results tile (Task 30): the result tab strip, the grid of the active tab, the cursor
//! strip, the output log view, and the events the console's run UI handles.
//!
//! Task 1b stub: the tile is exactly today's M1 tile (`tiles::results`), so the M1 and M2
//! shots do not move.

use std::ops::Range;

use hacivat_ui_core::gpui_surface::{
    AnyView, App, AppContext as _, Context, IntoElement, Render, Window,
};
use hacivat_ui_core::theme::Theme;

use crate::console::ConsoleId;

/// What the results tile asks of the console's run UI (Task 33 handles them).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResultsEvent {
    /// Run the unit behind a closed result again.
    Rerun {
        console: ConsoleId,
        unit_ordinal: u32,
    },
    /// Select the statement an output-log error line points at.
    GoToSource {
        console: ConsoleId,
        range: Range<usize>,
    },
}

/// The results tile's view.
struct ResultsHost;

impl Render for ResultsHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        crate::shell::tiles::results(&theme, cx)
    }
}

/// The `results_view` hook (a `ViewBuilder`): the results tile, built once per window.
pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let _ = window;
    cx.new(|_| ResultsHost).into()
}
