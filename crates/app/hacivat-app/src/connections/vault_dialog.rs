//! The vault (NOTES new surfaces, Task 21): create it, unlock it, reset it, and the label the
//! "Remember" select shows for `Forever` when the vault holds the secrets (Q23, Q38).
//!
//! Task 1 stubs: a question is answered `Cancelled` at once (no layer opens); no vault is
//! needed before a `Forever` put; the label is the keyring's. Task 21 fills them.

use hacivat_l10n::{MsgRef, NoArgs, ids};
use hacivat_session::pipeline::{Answer, Question};
use hacivat_ui_core::gpui_surface::{App, Task, Window};
use tokio::sync::oneshot;

/// Whether a `Forever` put may go ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code, reason = "Task 21 answers Ready and Declined")]
pub(crate) enum VaultReady {
    /// The keyring holds `Forever` secrets.
    NotNeeded,
    /// The vault exists and is unlocked.
    Ready,
    /// The user declined to create or unlock the vault.
    Declined,
}

/// Asks `q` (`Question::UnlockVault`, `Question::CreateVault`); the answer goes to `reply`.
pub(crate) fn open(
    _q: Question,
    reply: oneshot::Sender<Answer>,
    _window: &mut Window,
    _cx: &mut App,
) {
    let _ = reply.send(Answer::Cancelled);
}

/// The label of the `Forever` retention: the keyring's, or the vault's when it is chosen.
#[allow(dead_code, reason = "Task 21 shows it in the dialog's Remember select")]
pub(crate) fn forever_label(_cx: &App) -> MsgRef<NoArgs> {
    ids::hv::CONN_RET_FOREVER
}

/// Makes sure a `Forever` put has somewhere to go: with the vault chosen, it creates or
/// unlocks it first. Task 1 stub: `NotNeeded`.
pub(crate) fn ensure_vault_before_forever(_cx: &mut App) -> Task<VaultReady> {
    Task::ready(VaultReady::NotNeeded)
}
