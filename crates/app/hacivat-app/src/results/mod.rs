//! A console's results (Tasks 26 and 30): the Output tab and one tab per unit result, the grid
//! adapter over a result store, the status-bar context of the active result, copy, and the
//! results tile.
//!
//! Module lines, re-exports and [`install`] are Task 1b's (P12); Tasks 26 and 30 fill the
//! submodules and never edit this file. Task 1b's stubs: the `results_view` hook keeps today's
//! M1 tile; the adapter has no rows; no result context.

pub mod adapter;
pub mod cache;
pub mod context;
pub mod copy;
pub mod output_view;
pub mod strip;
pub mod tabs;
pub mod toolbar;
pub mod view;

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::App;

pub use adapter::GridAdapter;
pub use context::{CompletenessView, ResultContext, active_result_context};
pub use tabs::{ActiveResult, ConsoleResults};
pub use view::ResultsEvent;

use crate::shell::set_results_view;

/// Registers the results tile (`results::view::build`) as the `results_view` hook, and the
/// results and grid commands (`view::register_commands`, Task 30's body).
pub fn install(cx: &mut App) {
    set_results_view(cx, Rc::new(view::build));
    // ## m3a-t01b-seams (fix round 1)
    view::register_commands(cx);
}
