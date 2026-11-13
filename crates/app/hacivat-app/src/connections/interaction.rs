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
use hacivat_ui_core::gpui_surface::{App, Global, Window};
use tokio::sync::{mpsc, oneshot};

use crate::connections::{
    cert_dialog, hostkey_prompt, kbd_prompt, password_prompt, plaintext_prompt, secret_prompt,
    vault_dialog,
};
use crate::shell::Workspace;

/// One question on its way to the UI thread, with the channel its answer goes back on.
pub(crate) struct PromptRequest {
    pub(crate) question: Question,
    pub(crate) reply: oneshot::Sender<Answer>,
}

/// The app's `UiPort` (R14).
pub struct AppUiPort {
    requests: mpsc::UnboundedSender<PromptRequest>,
}

impl UiPort for AppUiPort {
    fn ask(&self, question: Question) -> BoxFuture<'static, Answer> {
        let (reply, answer) = oneshot::channel();
        let sent = self.requests.send(PromptRequest { question, reply });
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
    let (requests, mut incoming) = mpsc::unbounded_channel::<PromptRequest>();
    cx.set_global(InstalledPort(Arc::new(AppUiPort { requests })));
    cx.spawn(async move |cx| {
        while let Some(request) = incoming.recv().await {
            cx.update(|cx| route(request, cx));
        }
    })
    .detach();
}

// ## m2b-t01-interfaces
/// Sends `request` to the prompt of its question: the password prompt opens itself; every
/// other prompt opens in the workspace window (the active one, else the first). Without a
/// workspace window the request is dropped, which answers `Cancelled`.
fn route(request: PromptRequest, cx: &mut App) {
    if matches!(request.question, Question::Password { .. }) {
        password_prompt::open(request, cx);
        return;
    }
    let window = cx
        .active_window()
        .into_iter()
        .chain(cx.windows())
        .find(|w| w.downcast::<Workspace>().is_some());
    let Some(window) = window else {
        return;
    };
    let PromptRequest { question, reply } = request;
    let _ = window.update(cx, move |_, window, cx| {
        open_prompt(question, reply, window, cx)
    });
}

/// The prompt module of each M2b question.
fn open_prompt(q: Question, reply: oneshot::Sender<Answer>, window: &mut Window, cx: &mut App) {
    match q {
        Question::Password { .. } => {
            let _ = reply.send(Answer::Cancelled);
        }
        Question::Certificate { .. } => cert_dialog::open(q, reply, window, cx),
        Question::PlaintextPublic { .. } => plaintext_prompt::open(q, reply, window, cx),
        Question::HostKey { .. } => hostkey_prompt::open(q, reply, window, cx),
        Question::Secret { .. } => secret_prompt::open(q, reply, window, cx),
        Question::Keyboard { .. } => kbd_prompt::open(q, reply, window, cx),
        Question::UnlockVault { .. } | Question::CreateVault => {
            vault_dialog::open(q, reply, window, cx);
        }
    }
}

/// How connects started from the UI may ask the user: `Interactive` with the app's port once
/// [`install`] ran, `Headless` before (a missing secret then fails with `needs_interaction`).
pub fn interaction(cx: &App) -> Interaction {
    match cx.try_global::<InstalledPort>() {
        Some(port) => Interaction::Interactive(port.0.clone()),
        None => Interaction::Headless,
    }
}
