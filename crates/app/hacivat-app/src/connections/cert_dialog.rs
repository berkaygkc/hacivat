//! The certificate review (NOTES new surface 5, Task 19): trust and pin a certificate the
//! verifier refused, review a changed pin, import a CA.
//!
//! Task 1 stubs: a question is answered `Cancelled` at once (no layer opens); the review fix
//! does nothing. Task 19 fills both.

use std::sync::Arc;

use hacivat_session::pipeline::{Answer, ConnectError, Question};
use hacivat_ui_core::gpui_surface::{App, Window};
use tokio::sync::oneshot;

/// Asks `q` (`Question::Certificate`); the answer goes to `reply`.
pub(crate) fn open(
    _q: Question,
    reply: oneshot::Sender<Answer>,
    _window: &mut Window,
    _cx: &mut App,
) {
    let _ = reply.send(Answer::Cancelled);
}

/// The "Review certificate" fix of a failed connect: on "Replace pin" it builds
/// `ProfileChange::ReplacePin` itself, applies it and reconnects (Q26).
pub(crate) fn review(_err: Arc<ConnectError>, _window: &mut Window, _cx: &mut App) {}
