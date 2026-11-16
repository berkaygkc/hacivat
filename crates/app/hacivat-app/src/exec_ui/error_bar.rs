//! The error-policy bar (HV-EXEC-08, R-M3-61; M3b Task 23, `design/m3b-safe` surface 7): a
//! failed unit under the Ask policy offers Retry, Ignore, Ignore all, Stop and (in a
//! transaction) Roll back and continue.
//!
//! ## m3b-t01b-seams: the seams (Task 1b; P-M3b-17). STUB: [`show`] draws nothing and answers
//! `Stop` (the safe value: the run ends). [`on_choice`] is what Task 24's orchestrator calls on
//! every run event; Task 23 fills it in this file. Until then it answers nothing on its own:
//! no M3a run waits for a choice (`RunEvent::NeedsChoice` is Task 1c's, and the session's stub
//! never waits before Task 11). **Merge note (Task 1c):** once `RunEvent::NeedsChoice` and
//! `SessionHandle::choose` exist, this stub answers `RunChoice::Stop` through
//! `choose(run_seq, RunChoice::Stop)` for a `NeedsChoice` of the console's run.

use tokio::sync::oneshot;

use hacivat_session::session::{ExecError, RunEvent, UnitId};
use hacivat_ui_core::gpui_surface::App;

use super::answered;
use super::pending_core::RunChoice;
use crate::console::ConsoleId;

/// What the bar asks about: the failed unit, its error, and whether Roll back and continue is
/// offered (a transaction is open; Retry then hides, P-M3b-19).
#[derive(Debug, Clone)]
pub struct ErrorAsk {
    pub unit: UnitId,
    pub error: ExecError,
    pub rollback_and_continue: bool,
}

/// Shows the bar for `ask` on console `console`. STUB: answers `Stop` at once.
pub fn show(console: ConsoleId, ask: ErrorAsk, cx: &mut App) -> oneshot::Receiver<RunChoice> {
    let _ = (console, ask, cx);
    answered(RunChoice::Stop)
}

/// A run event of console `console`'s run (Task 24's orchestrator calls it for every event).
/// STUB: nothing (module docs, merge note).
pub fn on_choice(console: ConsoleId, ev: &RunEvent, cx: &mut App) {
    let _ = (console, ev, cx);
}
