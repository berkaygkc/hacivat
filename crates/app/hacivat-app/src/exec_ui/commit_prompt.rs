//! The confirm-in-transaction prompt (R-M3-55, D-014; M3b Task 21, `design/m3b-safe` surface
//! 2): Commit or Roll back the confirmed statement before the deadline.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: [`open`] draws nothing and answers `RollBack`
//! (the safe value: the confirmed statement is undone). Task 21 fills it.

use tokio::sync::oneshot;

use hacivat_ui_core::gpui_surface::App;

use super::answered;
use hacivat_exec::{CommitAnswer, CommitQuestion};

/// Asks `question`; any close answers `RollBack`. STUB: answers `RollBack` at once.
pub fn open(question: CommitQuestion, cx: &mut App) -> oneshot::Receiver<CommitAnswer> {
    let _ = (question, cx);
    answered(CommitAnswer::RollBack)
}
