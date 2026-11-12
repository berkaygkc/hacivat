//! The UI port of a connect: password prompts from the core runtime, answered on the UI thread
//! through `tokio::sync` channels (R14, §5.4, Task 20).
//!
//! [`AppUiPort`] is the pipeline's `UiPort`: `ask` runs on the core runtime and sends the
//! question over an `mpsc` channel to a foreground task, which opens the password prompt
//! ([`crate::connections::password_prompt`]) in the workspace window. The answer comes back on a
//! `oneshot`; a prompt that is cancelled, or closed any other way, answers
//! [`Answer::Cancelled`]. The UI thread never blocks on the core, and the core never touches
//! the UI.

use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_session::pipeline::{Answer, Interaction, Question, UiPort};
use hacivat_ui_core::gpui_surface::{App, Global};
use tokio::sync::{mpsc, oneshot};

use crate::connections::password_prompt;

/// One question on its way to the UI thread, with the channel its answer goes back on.
pub(crate) struct PasswordRequest {
    pub(crate) question: Question,
    pub(crate) reply: oneshot::Sender<Answer>,
}

/// The app's `UiPort` (R14).
pub struct AppUiPort {
    requests: mpsc::UnboundedSender<PasswordRequest>,
}

impl UiPort for AppUiPort {
    fn ask(&self, question: Question) -> BoxFuture<'static, Answer> {
        let (reply, answer) = oneshot::channel();
        let sent = self.requests.send(PasswordRequest { question, reply });
        Box::pin(async move {
            if sent.is_err() {
                // The UI is gone (quitting): nobody can answer.
                return Answer::Cancelled;
            }
            answer.await.unwrap_or(Answer::Cancelled)
        })
    }
}

/// The installed port.
struct InstalledPort(Arc<AppUiPort>);

impl Global for InstalledPort {}

/// Installs the port and the foreground task that opens a prompt for each question. Once per
/// app; no hook, no view: an M1 shell renders as before.
pub fn install(cx: &mut App) {
    if cx.has_global::<InstalledPort>() {
        return;
    }
    let (requests, mut incoming) = mpsc::unbounded_channel::<PasswordRequest>();
    cx.set_global(InstalledPort(Arc::new(AppUiPort { requests })));
    cx.spawn(async move |cx| {
        while let Some(request) = incoming.recv().await {
            cx.update(|cx| password_prompt::open(request, cx));
        }
    })
    .detach();
}

/// How connects started from the UI may ask the user: `Interactive` with the app's port once
/// [`install`] ran, `Headless` before (a missing secret then fails with `needs_interaction`).
pub fn interaction(cx: &App) -> Interaction {
    match cx.try_global::<InstalledPort>() {
        Some(port) => Interaction::Interactive(port.0.clone()),
        None => Interaction::Headless,
    }
}
