//! The error-policy bar (HV-EXEC-08, R-M3-61; M3b Task 23, `design/m3b-safe` surface 7): a
//! failed unit under the Ask policy offers Retry, Ignore, Ignore all, Stop and (in a
//! transaction) Roll back and continue.
//!
//! ## m3b-t01b-seams: the seams (Task 1b; P-M3b-17). STUB: [`show`] draws nothing and answers
//! `Stop` (the safe value: the run ends). [`on_choice`] is what Task 24's orchestrator calls on
//! every run event; Task 23 fills it in this file (the bar instead of the automatic Stop).
//!
//! **One answer per question.** A `RunEvent::NeedsChoice` carries its question id; the
//! session takes exactly one answer to it (`SessionHandle::choose(run_seq, question, c)`, a
//! second is refused). [`on_choice`] answers each question at most once: the question is
//! noted before the answer goes out, and a question already noted is never answered again
//! (Task 23's bar disables itself after the first click on the same rule).

use std::collections::HashSet;

use tokio::sync::oneshot;

use hacivat_exec::RunChoice;
use hacivat_session::session::{ExecError, RunEvent, UnitId};
use hacivat_ui_core::gpui_surface::{App, Global};

use super::{answered, console_exec};
use crate::console::ConsoleId;

/// What the bar asks about: the failed unit, the question's id (the answer names it), its
/// error, and whether Roll back and continue is offered (a transaction is open; Retry then
/// hides, P-M3b-19).
#[derive(Debug, Clone)]
pub struct ErrorAsk {
    pub unit: UnitId,
    pub question: u64,
    pub error: ExecError,
    pub rollback_and_continue: bool,
}

/// Shows the bar for `ask` on console `console`. STUB: answers `Stop` at once (one answer).
pub fn show(console: ConsoleId, ask: ErrorAsk, cx: &mut App) -> oneshot::Receiver<RunChoice> {
    let _ = (console, ask, cx);
    answered(RunChoice::Stop)
}

/// The questions answered, per console, of its current run (`(console, run_seq, question)`).
#[derive(Default)]
struct Answered(HashSet<(ConsoleId, u64, u64)>);

impl Global for Answered {}

/// Notes question `question` of run `run_seq` on `console` as answered; `false` when it already
/// was. Questions of the console's earlier runs are forgotten.
pub(crate) fn first_answer(cx: &mut App, console: ConsoleId, run_seq: u64, question: u64) -> bool {
    let answered = &mut cx.default_global::<Answered>().0;
    answered.retain(|(c, seq, _)| *c != console || *seq == run_seq);
    answered.insert((console, run_seq, question))
}

/// A run event of console `console`'s run (Task 24's orchestrator calls it for every event).
/// A `NeedsChoice` of the console's current run is answered `Stop`, once per question
/// (STUB until Task 23's bar asks the user). Any other event, or a question of another run,
/// changes nothing.
pub fn on_choice(console: ConsoleId, ev: &RunEvent, cx: &mut App) {
    let RunEvent::NeedsChoice { unit, question, .. } = ev else {
        return;
    };
    let Some(run) = console_exec(cx, console).and_then(|e| e.run.as_ref()) else {
        return;
    };
    if run.run != unit.run {
        return;
    }
    let (session, run_seq) = (run.session.clone(), run.seq);
    if !first_answer(cx, console, run_seq, *question) {
        return;
    }
    // The session says whether a run waited on this question; a refusal changes nothing, and
    // nothing else is to be done with it here.
    drop(session.choose(run_seq, *question, RunChoice::Stop));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn a_question_is_answered_once(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let c = ConsoleId(1);
            assert!(first_answer(cx, c, 7, 1));
            assert!(!first_answer(cx, c, 7, 1), "a second answer to question 1");
            assert!(first_answer(cx, c, 7, 2));
            // Another console's question of the same ids is its own.
            assert!(first_answer(cx, ConsoleId(2), 7, 1));
            // A new run forgets the questions of the console's earlier runs.
            assert!(first_answer(cx, c, 8, 1));
            assert!(!first_answer(cx, c, 8, 1));
        });
    }
}
