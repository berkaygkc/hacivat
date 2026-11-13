//! The connect's prompts beyond the password (Task 14): the bridge from the tunnel's
//! `SshPrompter` to the `UiPort` (`Question::{HostKey, Secret, Keyboard}`), the certificate
//! and public-plaintext questions with their one retry ([`ask_and_retry`]), the TLS key
//! passphrase, and the vault questions (unlock before a read, create before a `Forever` secret;
//! Q38).
//!
//! **Headless never asks.** Every question here goes through `Interaction::Interactive`; a
//! headless connect that would have asked fails with `needs_interaction` (or, for the vault,
//! goes on as if nothing were stored).
//!
//! **No loops.** [`ask_and_retry`] retries at most once per question kind per connect
//! ([`AskKind`]): a pin that still fails, or a second plaintext refusal, returns the error.

// ## m2b-t14-pipeline

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use futures::future::BoxFuture;
use hacivat_driver::{RawError, RawErrorKind};
use hacivat_net::ssh::{HostKey, KbdRequest, SshPrompter, SshSecretKind};
use hacivat_net::{CertPin, CertReview, TlsFailure, TlsMode, TlsParams, cert_review, pin_endpoint};
use hacivat_proto::{ErrClass, FixId, ProfileId};
use hacivat_secrets::{SecretError, SecretKey, SecretStore};
use secrecy::SecretString;

use super::steps::{StepState, build_tls, tls_setup_error};
use super::{Answer, ConnectError, Interaction, ProfileChange, Question, SecretPrompt, UiPort};
use crate::profile::Profile;

/// How many times one gesture may enter the vault's master password (B13).
const UNLOCK_ATTEMPTS: u8 = 3;

/// How many times a TLS key passphrase is asked for in one connect.
pub(crate) const PASSPHRASE_ATTEMPTS: u8 = 3;

/// A question that may lead to one retry of the driver connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AskKind {
    /// `Question::Certificate` (pin, import CA).
    Certificate,
    /// `Question::PlaintextPublic`, or the session overlay's consent to it.
    Plaintext,
}

/// The retries a connect has used, one per [`AskKind`].
#[derive(Debug, Default)]
pub(crate) struct Retries(Vec<AskKind>);

impl Retries {
    /// Takes `kind`'s one retry; `false` when it was already taken.
    fn take(&mut self, kind: AskKind) -> bool {
        if self.0.contains(&kind) {
            return false;
        }
        self.0.push(kind);
        true
    }
}

/// The review of the chain the last attempt's handshake recorded, for `endpoint`.
fn review_of(
    attempt: Option<&TlsParams>,
    endpoint: &str,
    pinned: Option<[u8; 32]>,
) -> Option<CertReview> {
    let chain = attempt?
        .presented
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()?;
    cert_review(&chain, endpoint, pinned)
}

/// After a driver error of a class that may be answered, asks (once per kind) and prepares the
/// retry: `Ok(())` means "connect again", with the state already changed (a pin, a CA file, the
/// plaintext consent). Otherwise the error, completed with what the error dialog needs (the
/// certificate, the fixes).
///
/// - `tls.untrusted` (an unknown issuer only: a pin never extends a certificate's dates):
///   `Question::Certificate`. `Pin` adds `ProfileChange::AddPin` and rebuilds the TLS
///   parameters with the pin; `ImportCa(path)` adds `SetCaFile`, approves the path and rebuilds
///   with it as the CA file. Anything else returns the error with the review.
/// - `tls.pin_changed`: never asked; the review carries the pinned and the presented
///   fingerprints. The pipeline never builds `ProfileChange::ReplacePin` (Q26).
/// - `tls.not_supported_by_server`: a pinned endpoint is never asked and offers only Edit
///   connection (Q13). Under `Auto` the session overlay's consent retries at once (Q17);
///   otherwise `Question::PlaintextPublic`: `ConnectPlaintext` retries in plaintext with
///   `InsecureAuth::Allow` (plus `AllowPublicPlaintext` with `remember`), `UseSsh` returns
///   the error with `[ConnectThroughSsh, EditConnection]`.
/// - `auth.insecure_method`: never asked; with a presented chain (TLS was up) the review is
///   attached so "Review certificate" can pin (Q20); without one the fixes narrow to Edit
///   connection (NOTES ruling 10).
pub(crate) async fn ask_and_retry(
    p: &Profile,
    st: &mut StepState,
    raw: &RawError,
    mut e: ConnectError,
) -> Result<(), ConnectError> {
    let attempt = st.attempt_tls.take();
    let port = st.port.unwrap_or_default();
    let endpoint = pin_endpoint(&st.tls_host, port);
    let pinned = st
        .tls_settings
        .pin_for(&st.tls_host, port)
        .map(|pin| pin.sha256);
    match e.class {
        ErrClass::TlsPinChanged => {
            e.certificate = review_of(attempt.as_ref(), &endpoint, pinned);
            Err(e)
        }
        ErrClass::AuthInsecureMethod => {
            e.certificate = review_of(attempt.as_ref(), &endpoint, None);
            e.fixes = if e.certificate.is_some() {
                vec![FixId::ReviewCertificate, FixId::EditConnection]
            } else {
                vec![FixId::EditConnection]
            };
            Err(e)
        }
        ErrClass::TlsUntrusted => {
            let pinnable = matches!(raw.kind, RawErrorKind::Tls(TlsFailure::UnknownIssuer));
            if !pinnable {
                // Fix round 1 (review M1): expired or not yet valid. Neither a pin (B5: it
                // never extends the dates) nor a CA can help, so neither is offered.
                e.fixes = vec![FixId::EditConnection];
                return Err(e);
            }
            let review = review_of(attempt.as_ref(), &endpoint, None);
            e.certificate = review.clone();
            let Some(review) = review else {
                // No chain to review: only the CA can still help.
                e.fixes = vec![FixId::ImportCa, FixId::EditConnection];
                return Err(e);
            };
            let Interaction::Interactive(ui) = st.ix.clone() else {
                e.needs_interaction = true;
                return Err(e);
            };
            if !st.retries.take(AskKind::Certificate) {
                return Err(e);
            }
            let sha256 = review.sha256;
            let q = Question::Certificate {
                profile: p.id,
                name: p.name.clone(),
                review,
            };
            match ui.ask(q).await {
                Answer::Pin => {
                    let pin = CertPin { endpoint, sha256 };
                    st.changes.push(ProfileChange::AddPin(pin.clone()));
                    st.tls_settings.pins.push(pin);
                    tracing::info!("certificate pinned by the user; connecting again");
                    rebuild(st).await
                }
                Answer::ImportCa(path) if picked_path(&path) => {
                    st.changes.push(ProfileChange::SetCaFile(path.clone()));
                    st.approved_roots.add(path.clone());
                    st.tls_settings.ca_file = Some(path);
                    tracing::info!("CA file imported by the user; connecting again");
                    rebuild(st).await
                }
                _ => Err(e),
            }
        }
        ErrClass::TlsNotSupportedByServer => {
            if pinned.is_some() {
                // A pin means "expect TLS with this certificate": never plaintext (Q13).
                e.fixes = vec![FixId::EditConnection];
                return Err(e);
            }
            if st.tls_settings.mode != TlsMode::Auto || st.tls.is_none() {
                return Err(e);
            }
            if st.consent {
                // The overlay's one-gesture consent (Q17), or an answer earlier in this connect.
                if !st.retries.take(AskKind::Plaintext) {
                    return Err(e);
                }
                allow_plaintext(st);
                return Ok(());
            }
            let Interaction::Interactive(ui) = st.ix.clone() else {
                e.needs_interaction = true;
                return Err(e);
            };
            if !st.retries.take(AskKind::Plaintext) {
                return Err(e);
            }
            let q = Question::PlaintextPublic {
                profile: p.id,
                name: p.name.clone(),
                host: st.tls_host.clone(),
                ip: st.resolved.first().map(std::net::SocketAddr::ip),
            };
            match ui.ask(q).await {
                Answer::ConnectPlaintext { remember } => {
                    st.consent = true;
                    if remember {
                        st.changes.push(ProfileChange::AllowPublicPlaintext);
                    }
                    allow_plaintext(st);
                    tracing::info!(
                        remember,
                        "plaintext toward a public host accepted by the user"
                    );
                    Ok(())
                }
                Answer::UseSsh => {
                    e.fixes = vec![FixId::ConnectThroughSsh, FixId::EditConnection];
                    Err(e)
                }
                _ => Err(e),
            }
        }
        _ => Err(e),
    }
}

/// A CA file from the file picker: absolute and local (a share or a device is never opened,
/// N2). `Answer::ImportCa` is produced only by the picker (Task 19), which approves what it
/// returns; a typed path never reaches it.
fn picked_path(path: &Path) -> bool {
    path.is_absolute() && hacivat_net::paths::local_path(path).is_some()
}

/// The next attempt may fall back to plaintext (the consent covers the password too: the
/// request then says `InsecureAuth::Allow`).
fn allow_plaintext(st: &mut StepState) {
    if let Some(t) = st.tls.as_mut() {
        t.plaintext_allowed = true;
        st.plaintext_retry = true;
    }
}

/// Rebuilds the TLS parameters from the changed settings, for the retry.
async fn rebuild(st: &mut StepState) -> Result<(), ConnectError> {
    let passphrase = st.tls_passphrase.clone();
    st.tls = build_tls(st, passphrase)
        .await?
        .map_err(|e| tls_setup_error(&e))?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The TLS key passphrase

/// Asks for the TLS client key's passphrase (`attempt` from 1). `None`: headless, or the user
/// cancelled.
pub(crate) async fn ask_tls_passphrase(
    p: &Profile,
    ix: &Interaction,
    key: &Path,
    attempt: u8,
) -> Option<SecretString> {
    let Interaction::Interactive(ui) = ix else {
        return None;
    };
    let q = Question::Secret {
        profile: p.id,
        name: p.name.clone(),
        prompt: SecretPrompt::TlsKeyPassphrase {
            key: key.to_path_buf(),
        },
        attempt,
        // ## m2b-t21-prompts (fix round 2): the DB retention keeps it; no choice here.
        remember: None,
    };
    match ui.ask(q).await {
        Answer::Secret(s) | Answer::Password(s) => Some(s),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// The vault (B13, Q38)

/// Unlocks the store's locked vault, asking `Question::UnlockVault { attempt }` up to 3 times:
/// `Master` → `unlock` on the blocking pool; a wrong master password → the next attempt.
/// `false` (go on as if nothing were stored) when headless, cancelled, after 3 failures, on any
/// other vault error, or when this connect already tried. Once per connect.
pub(crate) async fn unlock_vault(st: &mut StepState) -> bool {
    if st.vault_tried {
        return false;
    }
    st.vault_tried = true;
    let Interaction::Interactive(ui) = st.ix.clone() else {
        return false;
    };
    // A prompt after the transport opened its first stream may let it idle out.
    st.interacted |= st.first_stream.is_some();
    for attempt in 1..=UNLOCK_ATTEMPTS {
        let master = match ui.ask(Question::UnlockVault { attempt }).await {
            Answer::Master(master) => master,
            _ => return false,
        };
        let store = st.secrets.clone();
        let unlocked = tokio::task::spawn_blocking(move || match store.vault() {
            Some(vault) => vault.unlock(master),
            None => Err(SecretError::Unavailable),
        })
        .await;
        match unlocked {
            Ok(Ok(())) => {
                tracing::debug!(attempt, "vault unlocked");
                return true;
            }
            Ok(Err(SecretError::WrongMasterPassword)) => {
                tracing::debug!(attempt, "wrong vault master password");
            }
            // ## m2b-t22-live: a damaged file says so (the T21 notice), then nothing is stored.
            Ok(Err(SecretError::VaultCorrupt)) => {
                tracing::warn!("the vault failed its integrity check");
                let _ = ui.ask(Question::VaultDamaged).await;
                return false;
            }
            _ => {
                tracing::warn!("the vault could not be unlocked");
                return false;
            }
        }
    }
    false
}

// ## m2b-t22-live
/// A store read met a vault that failed its integrity check (T21 carry): an interactive
/// connect shows the damaged-file notice (`Question::VaultDamaged`) once, and this connect
/// tries the vault no further (a damaged file cannot be unlocked either). The caller then goes
/// on as if nothing were stored.
pub(crate) async fn vault_damaged(st: &mut StepState) {
    if st.vault_tried {
        return;
    }
    st.vault_tried = true;
    let Interaction::Interactive(ui) = st.ix.clone() else {
        return;
    };
    // A prompt after the transport opened its first stream may let it idle out.
    st.interacted |= st.first_stream.is_some();
    tracing::warn!("the vault failed its integrity check");
    let _ = ui.ask(Question::VaultDamaged).await;
}

/// Reads `keys` from the store on the blocking pool (outside the deadline, P23). A locked vault
/// is unlocked first when the connect may ask ([`unlock_vault`]); a read that still fails
/// counts as nothing stored.
pub(crate) async fn read_store(
    st: &mut StepState,
    keys: Vec<(SecretKey, hacivat_secrets::Retention)>,
) -> Vec<Option<SecretString>> {
    if keys.is_empty() {
        return Vec::new();
    }
    let read = |store: Arc<SecretStore>, keys: Vec<(SecretKey, hacivat_secrets::Retention)>| async move {
        tokio::task::spawn_blocking(move || {
            keys.iter()
                .map(|(key, retention)| store.get(key, *retention))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default()
    };
    let mut got = read(st.secrets.clone(), keys.clone()).await;
    let locked = got
        .iter()
        .any(|r| matches!(r, Err(SecretError::VaultLocked)));
    if locked && unlock_vault(st).await {
        got = read(st.secrets.clone(), keys).await;
    }
    // ## m2b-t22-live
    if got
        .iter()
        .any(|r| matches!(r, Err(SecretError::VaultCorrupt)))
    {
        vault_damaged(st).await;
    }
    got.into_iter()
        .map(|r| match r {
            Ok(secret) => secret,
            Err(_) => {
                tracing::debug!("a stored secret could not be read; asking instead");
                None
            }
        })
        .collect()
}

/// Before the first `Forever` secret of a connect is stored (Q38): when the store's backend is
/// a vault that does not exist yet, an interactive connect asks `Question::CreateVault`;
/// `Master` creates it on the blocking pool. Cancelled, headless or a failed create: the put
/// that follows falls back to `UntilRestart` (the M2a toast).
pub(crate) async fn ensure_vault(
    ix: &Interaction,
    secrets: &Arc<SecretStore>,
    cancel: &tokio_util::sync::CancellationToken,
) {
    let store = secrets.clone();
    let absent = tokio::task::spawn_blocking(move || store.vault().is_some_and(|v| !v.exists()))
        .await
        .unwrap_or(false);
    if !absent {
        return;
    }
    let Interaction::Interactive(ui) = ix else {
        return;
    };
    // Fix round 1 (review M2): a cancel while the question is open closes it unanswered, and
    // an answer that comes after a cancel creates nothing.
    let answer = tokio::select! {
        biased;
        () = cancel.cancelled() => return,
        answer = ui.ask(Question::CreateVault) => answer,
    };
    let Answer::Master(master) = answer else {
        tracing::debug!("vault not created; the secret is kept until restart");
        return;
    };
    if cancel.is_cancelled() {
        return;
    }
    let store = secrets.clone();
    let created = tokio::task::spawn_blocking(move || match store.vault() {
        Some(vault) => vault.create(master),
        None => Err(SecretError::Unavailable),
    })
    .await;
    if !matches!(created, Ok(Ok(()))) {
        tracing::warn!("the vault could not be created; the secret is kept until restart");
    }
}

// ---------------------------------------------------------------------------------------------
// SSH prompts

/// Which SSH secret a hop's prompt was answered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SshSlot {
    Password,
    Passphrase,
}

/// The tunnel's prompts, bridged to the connect's `UiPort` (B11): an unknown host key asks
/// `Question::HostKey`, a missing password or passphrase `Question::Secret`, a
/// keyboard-interactive round `Question::Keyboard`. It records which secrets the user typed, per
/// hop, so only those are stored after the whole connect succeeded (R8). Interactive only:
/// a headless connect gives the tunnel no prompter.
pub(crate) struct PipelinePrompter {
    ui: Arc<dyn UiPort>,
    profile: ProfileId,
    name: String,
    typed: Arc<Mutex<Vec<(u8, SshSlot)>>>,
    // ## m2b-t21-prompts (fix round 1): the Remember of the last SSH secret prompt answered.
    remember: Arc<Mutex<Option<hacivat_secrets::Retention>>>,
    /// Fix round 2 (R2): where a Main connect keeps typed SSH secrets unless a prompt chose
    /// otherwise, the saved profile's `ssh_retention`; `None` for a Test or a probe, whose
    /// prompts offer no Remember (they store nothing).
    remember_default: Option<hacivat_secrets::Retention>,
}

impl PipelinePrompter {
    pub(crate) fn new(ui: Arc<dyn UiPort>, p: &Profile, stores: bool) -> Arc<PipelinePrompter> {
        Arc::new(PipelinePrompter {
            ui,
            profile: p.id,
            name: p.name.clone(),
            typed: Arc::default(),
            remember: Arc::default(),
            remember_default: stores.then_some(p.ssh_retention),
        })
    }

    /// The secrets the user typed, by hop.
    pub(crate) fn typed(&self) -> Vec<(u8, SshSlot)> {
        self.typed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // ## m2b-t21-prompts (fix round 1)
    /// The retention the user chose at an SSH secret prompt (the last answer wins), if any.
    pub(crate) fn remember(&self) -> Option<hacivat_secrets::Retention> {
        *self.remember.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Records that the user typed `slot`'s secret for `hop`.
fn note(typed: &Mutex<Vec<(u8, SshSlot)>>, hop: u8, slot: SshSlot) {
    let mut typed = typed.lock().unwrap_or_else(PoisonError::into_inner);
    if !typed.contains(&(hop, slot)) {
        typed.push((hop, slot));
    }
}

impl SshPrompter for PipelinePrompter {
    fn host_key(&self, hop: u8, host: &str, port: u16, key: &HostKey) -> BoxFuture<'static, bool> {
        let asked = self.ui.ask(Question::HostKey {
            profile: self.profile,
            name: self.name.clone(),
            hop,
            host: host.to_string(),
            port,
            key: key.clone(),
        });
        Box::pin(async move { matches!(asked.await, Answer::TrustHostKey) })
    }

    fn secret(
        &self,
        hop: u8,
        kind: SshSecretKind,
        attempt: u8,
    ) -> BoxFuture<'static, Option<SecretString>> {
        let (prompt, slot) = match kind {
            SshSecretKind::Password { host, user } => (
                SecretPrompt::SshPassword { hop, host, user },
                SshSlot::Password,
            ),
            SshSecretKind::Passphrase { key } => (
                SecretPrompt::SshPassphrase { hop, key },
                SshSlot::Passphrase,
            ),
        };
        // ## m2b-t21-prompts (fix round 2, R2): the prompt starts from the saved retention, or
        // from the choice an earlier prompt of this same connect made; never from anything
        // older, and never for a connect that stores nothing.
        let remember = self
            .remember_default
            .map(|saved| self.remember().unwrap_or(saved));
        let asked = self.ui.ask(Question::Secret {
            profile: self.profile,
            name: self.name.clone(),
            prompt,
            attempt,
            remember,
        });
        let typed = self.typed.clone();
        // Fix round 2 (R2): a connect that offered no choice records none, whatever the answer.
        let chosen = remember.is_some().then(|| self.remember.clone());
        Box::pin(async move {
            match asked.await {
                Answer::Secret(s) | Answer::Password(s) => {
                    note(&typed, hop, slot);
                    Some(s)
                }
                // ## m2b-t21-prompts (fix round 1): the prompt's Remember travels with it.
                Answer::SshSecret { secret, remember } => {
                    note(&typed, hop, slot);
                    if let Some(chosen) = chosen {
                        *chosen.lock().unwrap_or_else(PoisonError::into_inner) = Some(remember);
                    }
                    Some(secret)
                }
                _ => None,
            }
        })
    }

    fn keyboard(
        &self,
        hop: u8,
        req: KbdRequest,
        attempt: u8,
    ) -> BoxFuture<'static, Option<Vec<SecretString>>> {
        let asked = self.ui.ask(Question::Keyboard {
            profile: self.profile,
            name: self.name.clone(),
            hop,
            request: req,
            attempt,
        });
        Box::pin(async move {
            match asked.await {
                Answer::Keyboard(answers) => Some(answers),
                _ => None,
            }
        })
    }
}
