//! The confirmation dialog (R-M3-55, R-M3-84; M3b Task 21, `design/m3b-safe` surface 1) and the
//! app's `ConfirmPort`.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: [`open`] draws nothing and answers
//! `Cancel` (the safe value: nothing runs), and [`AppConfirmPort`] answers `Cancel` and
//! `RollBack` without asking. Task 21 fills both (the port then goes to the foreground through
//! an mpsc, the `AppUiPort` pattern).

use tokio::sync::oneshot;

use hacivat_ui_core::gpui_surface::App;

use super::answered;
use hacivat_exec::{CommitAnswer, CommitQuestion, ConfirmAnswer, ConfirmPort, ConfirmQuestion};

/// Asks `question`; any close (Escape, window close, ⌘W, quit) answers `Cancel`.
/// STUB: answers `Cancel` at once.
pub fn open(question: ConfirmQuestion, cx: &mut App) -> oneshot::Receiver<ConfirmAnswer> {
    let _ = (question, cx);
    answered(ConfirmAnswer::Cancel)
}

/// The app's `ConfirmPort` (the run service asks it from the core runtime).
/// STUB: answers `Cancel` and `RollBack` without asking.
#[derive(Debug, Default)]
pub struct AppConfirmPort;

impl ConfirmPort for AppConfirmPort {
    fn confirm(&self, q: ConfirmQuestion) -> oneshot::Receiver<ConfirmAnswer> {
        let _ = q;
        answered(ConfirmAnswer::Cancel)
    }

    fn commit_decision(&self, q: CommitQuestion) -> oneshot::Receiver<CommitAnswer> {
        let _ = q;
        answered(CommitAnswer::RollBack)
    }
}
