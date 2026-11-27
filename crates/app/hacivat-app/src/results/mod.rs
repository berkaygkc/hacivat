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
// ## m3b-t01b-seams: the M3b/M3c results modules (stubs that draw nothing): Count rows and
// pinned tabs (Task 25), local sort (Task 36), find and go to row (Task 37), the views and the
// value viewer (Task 38), the grid context menu (Task 39).
pub mod context_menu;
pub mod count;
pub mod find;
pub mod goto;
pub mod pin;
pub mod sort;
pub mod value_viewer;
pub mod views;

use std::rc::Rc;

use hacivat_ui::actions::{ActionDef, ActionRegistry};
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
    // ## m3b-t01b-seams
    register_tab_steps(cx);
    sort::register(cx);
    views::register(cx);
}

// ## m3b-t01b-seams

/// The key context of the results tile while its root has the focus (P-M3b-21): the
/// `results.next_tab`/`prev_tab` rows (`ctrl-pagedown`/`ctrl-pageup`) live there.
pub const RESULTS_CONTEXT: &str = "Results";

/// Registers `results.next_tab` and `results.prev_tab` (context `Results`). Their bodies and
/// `enabled` are `tabs::{step_tab, can_step_tab}` (Task 25; until then disabled, so the keys
/// pass through as in M3a).
fn register_tab_steps(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    for (id, title_id, forward) in [
        ("results.next_tab", "hv-cmd-results-next-tab", true),
        ("results.prev_tab", "hv-cmd-results-prev-tab", false),
    ] {
        let _ = reg.register(ActionDef {
            id,
            title_id,
            context: RESULTS_CONTEXT,
            run: Rc::new(move |window, cx| tabs::step_tab(forward, window, cx)),
            enabled: tabs::can_step_tab,
        });
    }
}
