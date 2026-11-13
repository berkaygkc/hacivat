//! SSH host keys (NOTES surface 6 new-host-key prompt, Task 21): trust an unknown key, review a
//! changed one.
//!
//! Task 1 stubs: a question is answered `Cancelled` at once (no layer opens, nothing is
//! trusted); the review does nothing. Task 21 fills both.

use hacivat_proto::ProfileId;
use hacivat_session::pipeline::{Answer, HostKeyMismatch, Question};
use hacivat_ui_core::gpui_surface::{App, Window};
use tokio::sync::oneshot;

/// Asks `q` (`Question::HostKey`); the answer goes to `reply`.
pub(crate) fn open(
    _q: Question,
    reply: oneshot::Sender<Answer>,
    _window: &mut Window,
    _cx: &mut App,
) {
    let _ = reply.send(Answer::Cancelled);
}

/// The "Review host key" fix: shows the changed key `m` of `profile`'s hop (a hard stop) and
/// replaces it in Hacivat's store only on an explicit choice.
pub(crate) fn review_changed(
    _m: HostKeyMismatch,
    _profile: ProfileId,
    _window: &mut Window,
    _cx: &mut App,
) {
}
