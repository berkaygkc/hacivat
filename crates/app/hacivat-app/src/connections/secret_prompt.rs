//! SSH passwords, SSH key passphrases and TLS key passphrases asked during a connect (NOTES new
//! surfaces, Task 21).
//!
//! Task 1 stub: answered `Cancelled` at once (no layer opens). Task 21 fills it.

use hacivat_session::pipeline::{Answer, Question};
use hacivat_ui_core::gpui_surface::{App, Window};
use tokio::sync::oneshot;

/// Asks `q` (`Question::Secret`); the answer goes to `reply`.
pub(crate) fn open(
    _q: Question,
    reply: oneshot::Sender<Answer>,
    _window: &mut Window,
    _cx: &mut App,
) {
    let _ = reply.send(Answer::Cancelled);
}
