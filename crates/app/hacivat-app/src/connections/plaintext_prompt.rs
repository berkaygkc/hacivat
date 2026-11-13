//! The public-plaintext question (NOTES new surface 5, Task 19): credentials would cross a
//! public network unencrypted.
//!
//! Task 1 stub: answered `Cancelled` at once (no layer opens). Task 19 fills it.

use hacivat_session::pipeline::{Answer, Question};
use hacivat_ui_core::gpui_surface::{App, Window};
use tokio::sync::oneshot;

/// Asks `q` (`Question::PlaintextPublic`); the answer goes to `reply`.
pub(crate) fn open(
    _q: Question,
    reply: oneshot::Sender<Answer>,
    _window: &mut Window,
    _cx: &mut App,
) {
    let _ = reply.send(Answer::Cancelled);
}
