//! `ConnectionsModel`: the profiles, their live states and the one ordered writer of
//! `connections.toml` (Task 12). Profile writes go only through it.
//!
//! **Writes.** `save`, `delete` and `duplicate` change the profile list on the UI thread at
//! once, then queue a job on the core runtime: the job's secret calls (`SecretStore` blocks)
//! run on the runtime's blocking pool, then the latest submitted list is written (the
//! `LatestWriter` pattern: one slot, written under its lock, so an older list can never land
//! after a newer one). Jobs run one at a time, in submit order, so two saves of one profile
//! reach the keyring in order too. The UI side only awaits a `oneshot` (§5.4, P48).
//!
//! **Secrets (R8).** A draft's password and TLS key passphrase are stored under the profile's
//! retention (`Forever` by default); a `Forever` secret that lands in the keyring is labelled
//! with the profile name (P44). Changing the retention moves the stored secrets. When the
//! keyring refuses, the secret is kept until quit and the `hv-conn-keyring-unavailable` toast
//! shows, once per session. A secret error never loses the profile's fields: they are written
//! and the error is returned. `test` never stores a secret; a password re-entered at connect is
//! stored only after the connect succeeds (Task 16).
//!
//! **M2b (Task 21).** Where the vault holds `Forever` secrets, a save's job reads the vault's
//! state on the blocking pool and, when the vault is missing or locked, waits for the Create or
//! Unlock dialog before any `Forever` put (`Declined` falls back to Until restart, Q38). An edit
//! that adds, removes or reorders jump hosts moves each hop's stored SSH secrets with the hop and
//! drops those of a hop that is gone or changed. A delete or duplicate that meets a locked vault
//! asks to unlock it, then runs its secret calls again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_net::ssh::{HopSecrets, HopSpec, KnownHosts, SshAuth, TunnelManager};
use hacivat_proto::{ProfileId, TagSource};
use hacivat_secrets::{
    BackendChoice, MAX_SSH_HOP, Retention, SecretError, SecretKey, SecretSlot, SecretStore, Stored,
};
use hacivat_session::pipeline::ProfileChange;
use hacivat_session::profile::{
    Profile, ProfileDiag, ProfileDraft, ProfileStore, StoreError, TransportSpec, duplicate_name,
    fold_typed_options, strip_secret_options,
};
use hacivat_session::{ConnectionManager, CoreRuntime, LiveState, TestHandle};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Global, SharedString, Task};
use secrecy::SecretString;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use crate::connections::interaction::interaction;
use crate::connections::vault_dialog;
use crate::l10n_global::tr;

/// Why a profile write failed (M2a).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SaveError {
    StoreReadOnly,
    Io(String),
    Secret(SecretError),
    /// The profile is not in the list (deleted meanwhile): an edit never brings it back.
    NotFound,
    // ## m2b-t21-prompts (fix round 1, I1, m2)
    /// Nothing was changed: the profile's stored secrets could not be moved (a jump-host
    /// change) or deleted (a delete) — a locked vault whose unlock was declined, a write the
    /// store refused. The profile stays as it was, so no secret is ever left on the wrong hop
    /// or orphaned under a deleted profile.
    Unchanged(SecretError),
}

impl From<StoreError> for SaveError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::ReadOnly => SaveError::StoreReadOnly,
            StoreError::Io(e) => SaveError::Io(e),
        }
    }
}

/// The secret slots a profile's dialog fills (M2a; SSH and proxy slots arrive in M2b).
const DRAFT_SLOTS: [SecretSlot; 2] = [SecretSlot::DbPassword, SecretSlot::TlsKeyPassphrase];

/// The app's connections (a Global, M2a).
pub struct ConnectionsModel {
    profiles: Vec<Arc<Profile>>,
    live: HashMap<ProfileId, LiveState>,
    selected: Option<ProfileId>,
    current: Option<ProfileId>,
    manager: Arc<ConnectionManager>,
    secrets: Arc<SecretStore>,
    runtime: Handle,
    core: Arc<CoreRuntime>,
    read_only: bool,
    writer: ProfileWriter,
    /// The keyring fallback toast was shown this session.
    keyring_notice_shown: bool,
    // ## m2b-t01-interfaces
    known_hosts: Arc<KnownHosts>,
    tunnels: Arc<TunnelManager>,
    // ## m2b-t21-prompts (fix round 1, I1): profiles whose SSH slots are being moved; a save
    // of one waits until the move is done (its hop layout is then the saved one).
    remapping: HashMap<ProfileId, Vec<oneshot::Sender<()>>>,
}

impl Global for ConnectionsModel {}

impl ConnectionsModel {
    /// The model over `profiles` loaded from `store` (Task 12's boot).
    pub(crate) fn new(
        core: Arc<CoreRuntime>,
        store: ProfileStore,
        profiles: Vec<Profile>,
        secrets: Arc<SecretStore>,
        manager: Arc<ConnectionManager>,
        known_hosts: Arc<KnownHosts>,
        tunnels: Arc<TunnelManager>,
    ) -> Self {
        let runtime = core.handle();
        ConnectionsModel {
            profiles: profiles.into_iter().map(Arc::new).collect(),
            live: HashMap::new(),
            selected: None,
            current: None,
            manager,
            secrets,
            read_only: store.read_only(),
            writer: ProfileWriter::new(store, &runtime),
            runtime,
            core,
            keyring_notice_shown: false,
            known_hosts,
            tunnels,
            remapping: HashMap::new(),
        }
    }

    // ## m2b-t01-interfaces

    /// The known-host stores (Q23).
    pub fn known_hosts(&self) -> Arc<KnownHosts> {
        self.known_hosts.clone()
    }

    /// The SSH chains (Q23).
    pub fn tunnels(&self) -> Arc<TunnelManager> {
        self.tunnels.clone()
    }

    /// Applies `changes` (a pin, a CA file, the plaintext consent) to the saved profile `id`
    /// (Task 19). Task 1 stub: refuses, so no change is reported as saved before Task 19.
    pub fn apply_changes(
        cx: &mut App,
        id: ProfileId,
        changes: Vec<ProfileChange>,
    ) -> Task<Result<(), SaveError>> {
        // ## m2b-t21-prompts (fix round 1, Remember seam): a connect that reports only the
        // Remember of an SSH secret prompt re-saves the profile with that `ssh_retention`.
        if let [ProfileChange::SshRetention(retention)] = changes[..] {
            return apply_ssh_retention(cx, id, retention);
        }
        Task::ready(Err(SaveError::Io("apply_changes: Task 19".to_string())))
    }

    pub fn profiles(&self) -> &[Arc<Profile>] {
        &self.profiles
    }

    pub fn get(&self, id: ProfileId) -> Option<Arc<Profile>> {
        self.profiles.iter().find(|p| p.id == id).cloned()
    }

    /// The mirrored live state of `id`.
    pub fn live(&self, id: ProfileId) -> LiveState {
        self.live.get(&id).cloned().unwrap_or_default()
    }

    pub fn set_live(&mut self, id: ProfileId, state: LiveState) {
        self.live.insert(id, state);
    }

    /// The profile selected in the explorer.
    pub fn selected(&self) -> Option<ProfileId> {
        self.selected
    }

    /// Selects `id` in the explorer (an unknown ID selects nothing).
    pub fn select(cx: &mut App, id: Option<ProfileId>) {
        let m = cx.global_mut::<ConnectionsModel>();
        m.selected = id.filter(|id| m.profiles.iter().any(|p| p.id == *id));
    }

    /// The profile the capsule shows: the last one connected or attempted.
    pub fn current(&self) -> Option<ProfileId> {
        self.current
    }

    /// Makes `id` current (a connect was attempted).
    pub fn note_attempt(&mut self, id: ProfileId) {
        self.current = Some(id);
    }

    /// Saves `draft` (a new profile, or the saved one with its ID), and its password and TLS
    /// key passphrase by its retention (see the module docs). A read-only store refuses at
    /// once and changes nothing; so does an edit of a profile that was deleted meanwhile
    /// (`NotFound`: it is never brought back).
    ///
    /// Environment tags (D-004): a profile whose tag the organization set keeps its stored
    /// `environment` and `env_source`, whatever the draft says; any other profile's tag is the
    /// user's (`env_source` is always `User`).
    pub fn save(cx: &mut App, draft: ProfileDraft) -> Task<Result<ProfileId, SaveError>> {
        Self::save_checked(cx, draft, false)
    }

    // ## m2b-t21-prompts (fix round 1, I1)
    /// [`Self::save`]; `remapped` once the SSH slots already follow the draft's hops. A draft
    /// that adds, removes or reorders jump hosts first moves the stored SSH secrets in a
    /// secrets-only job (the vault unlocked first when needed); only when every read and write
    /// succeeded is the new hop order saved. Otherwise nothing changes and the save fails with
    /// `Unchanged`, so a secret never stays on a hop it was not typed for.
    fn save_checked(
        cx: &mut App,
        draft: ProfileDraft,
        remapped: bool,
    ) -> Task<Result<ProfileId, SaveError>> {
        let m = cx.global::<ConnectionsModel>();
        if m.read_only {
            return Task::ready(Err(SaveError::StoreReadOnly));
        }
        let id = draft.profile.id;
        if !remapped && m.remapping.contains_key(&id) {
            let (tx, rx) = oneshot::channel();
            if let Some(waiting) = cx.global_mut::<ConnectionsModel>().remapping.get_mut(&id) {
                waiting.push(tx);
            }
            return cx.spawn(async move |cx| {
                let _ = rx.await;
                cx.update(|cx| Self::save(cx, draft)).await
            });
        }
        let stored = m.get(id);
        if !remapped
            && let Some(old) = stored.as_deref()
            && let Some(remap) = hop_remap(old, &draft.profile)
        {
            let retention = old.ssh_retention;
            return Self::remap_then_save(cx, draft, retention, remap);
        }
        let ProfileDraft {
            mut profile,
            password,
            tls_key_passphrase,
            is_new,
            ssh_secrets,
            ..
        } = draft;
        let id = profile.id;
        let stored = m.get(id);
        if !is_new && stored.is_none() {
            return Task::ready(Err(SaveError::NotFound));
        }
        // ## m2a-finalfix: one value per setting, and never a password in `extra` (I1, I2):
        // options with a typed setting go to it, password-like ones are dropped, in memory as
        // in the file.
        strip_secret_options(&mut profile);
        fold_typed_options(&mut profile);
        match stored.as_deref() {
            Some(old) if old.env_source == TagSource::Org => {
                profile.environment = old.environment;
                profile.env_source = TagSource::Org;
            }
            _ => profile.env_source = TagSource::User,
        }
        // RF5 (P39, Task 18): an edit of a profile that is connecting cancels the attempt
        // first, so it never lands with the old values; the next connect uses the new ones.
        if !is_new && matches!(m.manager.state(id), LiveState::Connecting(_)) {
            m.manager.cancel(id);
        }
        let old_retention = stored.as_ref().map(|p| p.retention);
        let retention = profile.retention;
        // ## m2b-t01-interfaces: the typed SSH secrets, per hop, with `ssh_retention` (Q23).
        let old_ssh_retention = stored.map(|p| p.ssh_retention);
        let ssh_retention = profile.ssh_retention;
        let ssh_values = ssh_slot_values(&profile, ssh_secrets);
        let name = profile.name.clone();
        let secrets = m.secrets.clone();
        // ## m2b-t21-prompts: a `Forever` put waits for the vault (T1 review M7, Q38). Unless
        // the keyring or the memory backend holds Forever secrets, the job below reads the
        // vault's state on the blocking pool; only when the vault is missing or locked does it
        // ask the UI (Create or Unlock) and wait for the answer. Declined: the put falls back to
        // Until restart, with the M2a toast. A save that needs no dialog never waits for the UI
        // thread, so a quit right after it still drains the queue (P15).
        let touches = touches_forever(
            Change {
                old: old_retention,
                new: retention,
                typed: password.is_some() || tls_key_passphrase.is_some(),
            },
            Change {
                old: old_ssh_retention,
                new: ssh_retention,
                typed: ssh_values.iter().any(|(_, v)| v.is_some()),
            },
        );
        let (job_gate, ui_gate) = vault_gate(touches && maybe_vault(&secrets));
        let env = profile.environment;

        let m = cx.global_mut::<ConnectionsModel>();
        let profile = Arc::new(profile);
        // Q17: a connected profile's sessions reconnect with the saved values.
        m.manager.update_profile(id, profile.clone());
        match m.profiles.iter_mut().find(|p| p.id == id) {
            Some(slot) => *slot = profile,
            None => m.profiles.push(profile),
        }
        let asker = name.clone();
        let done = m.writer.run(m.snapshot(), move || {
            // ## m2b-t21-prompts: on the blocking pool. A dropped channel (the app quit, the
            // dialog could not open) goes on, and the put falls back (Q38).
            pass_vault_gate(job_gate, &secrets);
            let values = DRAFT_SLOTS
                .into_iter()
                .zip([password, tls_key_passphrase])
                .collect();
            let (db_fell_back, db_error) =
                store_secrets(&secrets, id, &name, old_retention, retention, values);
            let (ssh_fell_back, ssh_error) = store_secrets(
                &secrets,
                id,
                &name,
                old_ssh_retention,
                ssh_retention,
                ssh_values,
            );
            (db_fell_back || ssh_fell_back, db_error.or(ssh_error))
        });
        cx.spawn(async move |cx| {
            // ## m2b-t21-prompts: the job asks only when the vault must be created or unlocked.
            cx.update(|cx| answer_vault_gate(ui_gate, asker, env, cx))
                .await;
            let ((fell_back, secret), written) = done.await.map_err(|_| cancelled())?;
            if fell_back {
                cx.update(notice_keyring_unavailable);
            }
            cx.update(|cx| written.surface(cx))?;
            secret.map_or(Ok(id), |e| Err(SaveError::Secret(e)))
        })
    }

    // ## m2b-t21-prompts (fix round 1, I1)
    /// Moves the stored SSH slots of `draft`'s profile to its new hop layout (`remap`, kept
    /// under `retention`) in a secrets-only job, unlocking the vault first when it holds them;
    /// then, only if the move fully succeeded, saves `draft` as usual. Any failure (a declined
    /// unlock, a refused write) leaves the stored profile and its slots as they were and fails
    /// with `Unchanged`.
    fn remap_then_save(
        cx: &mut App,
        draft: ProfileDraft,
        retention: Retention,
        remap: HopRemap,
    ) -> Task<Result<ProfileId, SaveError>> {
        let id = draft.profile.id;
        let (name, env) = (draft.profile.name.clone(), draft.profile.environment);
        let m = cx.global_mut::<ConnectionsModel>();
        m.remapping.insert(id, Vec::new());
        let secrets = m.secrets.clone();
        let needs_vault = retention == Retention::Forever && maybe_vault(&secrets);
        let (job_gate, ui_gate) = vault_gate(needs_vault);
        let label = name.clone();
        let moved = m.writer.run_only(move || {
            pass_vault_gate(job_gate, &secrets);
            remap_ssh_slots(&secrets, id, retention, &remap, &label)
        });
        cx.spawn(async move |cx| {
            cx.update(|cx| answer_vault_gate(ui_gate, name, env, cx))
                .await;
            let moved = moved.await.map_err(|_| cancelled());
            match moved {
                Ok(Ok(fell_back)) => {
                    if fell_back {
                        cx.update(notice_keyring_unavailable);
                    }
                    let saving = cx.update(|cx| {
                        let saving = Self::save_checked(cx, draft, true);
                        finish_remap(cx, id);
                        saving
                    });
                    saving.await
                }
                Ok(Err(e)) => {
                    tracing::debug!(error = ?e, "connections: SSH secrets not moved; not saved");
                    cx.update(|cx| finish_remap(cx, id));
                    Err(SaveError::Unchanged(e))
                }
                Err(e) => {
                    cx.update(|cx| finish_remap(cx, id));
                    Err(e)
                }
            }
        })
    }

    /// Deletes `id`: the manager forgets it (cancels, disconnects, drops its session secrets),
    /// the profile leaves the list and the file, and every secret slot is deleted. A secret
    /// that cannot be deleted is reported after the profile is gone.
    ///
    /// ## m2b-t21-prompts (fix round 1, m2): where the vault may hold the profile's `Forever`
    /// secrets, they are deleted first, in a secrets-only job: a locked vault asks to be
    /// unlocked. When the user declines (or misses three times), nothing is deleted and the
    /// delete fails with `Unchanged`, so no secret outlives its profile in the vault. A vault
    /// that cannot be used at all (a damaged or non-regular file) cannot be read either, so the
    /// profile then goes as in M2a and the error is reported.
    pub fn delete(cx: &mut App, id: ProfileId) -> Task<Result<(), SaveError>> {
        if cx.global::<ConnectionsModel>().read_only {
            return Task::ready(Err(SaveError::StoreReadOnly));
        }
        let m = cx.global::<ConnectionsModel>();
        if !maybe_vault(&m.secrets) {
            return Self::delete_now(cx, id, None);
        }
        let asker = m.get(id).map(|p| (p.name.clone(), p.environment));
        let secrets = m.secrets.clone();
        let again = secrets.clone();
        let first = m.writer.run_only(move || secrets.delete_all(id));
        cx.spawn(async move |cx| {
            let mut deleted = first.await.map_err(|_| cancelled())?;
            if deleted == Err(SecretError::VaultLocked)
                && let Some((name, env)) = asker
            {
                let unlocked = cx.update(|cx| vault_dialog::unlock_vault(cx, name, env));
                match unlocked.await {
                    vault_dialog::FlowEnd::Ready => {
                        let retry = cx.update(|cx| {
                            let m = cx.global::<ConnectionsModel>();
                            m.writer.run_only(move || again.delete_all(id))
                        });
                        deleted = retry.await.map_err(|_| cancelled())?;
                    }
                    vault_dialog::FlowEnd::Declined => {
                        return Err(SaveError::Unchanged(SecretError::VaultLocked));
                    }
                    // The vault can't be used: its entries can't be read by anyone either, so
                    // the profile goes (M2a) and the secret error is reported.
                    vault_dialog::FlowEnd::Unusable => {
                        let gone = Some(Err(SecretError::VaultLocked));
                        return cx.update(|cx| Self::delete_now(cx, id, gone)).await;
                    }
                }
            }
            match deleted {
                Err(SecretError::VaultLocked) => {
                    Err(SaveError::Unchanged(SecretError::VaultLocked))
                }
                deleted => {
                    cx.update(|cx| Self::delete_now(cx, id, Some(deleted)))
                        .await
                }
            }
        })
    }

    /// The M2a delete: the profile leaves the list and the file at once; its secrets are
    /// deleted in the same job unless `secrets_done` already says how that went.
    fn delete_now(
        cx: &mut App,
        id: ProfileId,
        secrets_done: Option<Result<(), SecretError>>,
    ) -> Task<Result<(), SaveError>> {
        let m = cx.global_mut::<ConnectionsModel>();
        m.manager.forget(id);
        m.profiles.retain(|p| p.id != id);
        m.live.remove(&id);
        if m.selected == Some(id) {
            m.selected = None;
        }
        if m.current == Some(id) {
            m.current = None;
        }
        let secrets = m.secrets.clone();
        let done = m.writer.run(m.snapshot(), move || match secrets_done {
            Some(result) => result,
            None => secrets.delete_all(id),
        });
        cx.spawn(async move |cx| {
            let (deleted, written) = done.await.map_err(|_| cancelled())?;
            cx.update(|cx| written.surface(cx))?;
            deleted.map_err(SaveError::Secret)
        })
    }

    /// Saves a copy of `id` right after it, named `<name>-2` (or the next free number), with
    /// copies of its secrets, selects it and shows the `hv-conn-toast-dup` toast. The copy's
    /// tag is the user's (`env_source` `User`, D-004), even when the organization set the
    /// original's. It keeps the original's origin (M2a final review M1): a copy of an imported
    /// or pasted profile is still untrusted, so a duplicate never launders its options past
    /// DRV-05.
    pub fn duplicate(cx: &mut App, id: ProfileId) -> Task<Result<ProfileId, SaveError>> {
        let m = cx.global::<ConnectionsModel>();
        if m.read_only {
            return Task::ready(Err(SaveError::StoreReadOnly));
        }
        let Some(at) = m.profiles.iter().position(|p| p.id == id) else {
            return Task::ready(Err(SaveError::NotFound));
        };
        let names: Vec<&str> = m.profiles.iter().map(|p| p.name.as_str()).collect();
        let mut copy = (*m.profiles[at]).clone();
        copy.id = ProfileId::new();
        copy.name = duplicate_name(&m.profiles[at].name, &names);
        copy.env_source = TagSource::User;
        let (new_id, name, retention) = (copy.id, copy.name.clone(), copy.retention);
        // ## m2b-t01-interfaces (fix round 1, I2): the SSH slots keep their own retention.
        let ssh_retention = copy.ssh_retention;
        let ssh_labels: Vec<SecretSlot> = ssh_slot_values(&copy, Vec::new())
            .into_iter()
            .map(|(slot, _)| slot)
            .collect();
        let secrets = m.secrets.clone();

        // ## m2b-t21-prompts: named in the unlock dialog should the vault be locked.
        let env = copy.environment;
        let original = m.profiles[at].name.clone();

        let m = cx.global_mut::<ConnectionsModel>();
        m.profiles.insert(at + 1, Arc::new(copy));
        m.selected = Some(new_id);
        let copier = Arc::new(CopySecrets {
            secrets,
            from: id,
            to: new_id,
            retention,
            ssh_retention,
            ssh_labels,
            label: name.clone(),
        });
        let first = copier.clone();
        let done = m.writer.run(m.snapshot(), move || first.run());
        cx.spawn(async move |cx| {
            let (mut copied, written) = done.await.map_err(|_| cancelled())?;
            cx.update(|cx| written.surface(cx))?;
            // ## m2b-t21-prompts (T6 carry): a locked vault holds the original's Forever
            // secrets: unlock it, then copy them.
            if copied == Err(SecretError::VaultLocked) {
                let unlocked = cx.update(|cx| vault_dialog::unlock_vault(cx, original, env));
                if unlocked.await == vault_dialog::FlowEnd::Ready {
                    let again = cx.update(|cx| {
                        let m = cx.global::<ConnectionsModel>();
                        m.writer.run(m.snapshot(), move || copier.run())
                    });
                    let (result, written) = again.await.map_err(|_| cancelled())?;
                    cx.update(|cx| written.surface(cx))?;
                    copied = result;
                }
            }
            cx.update(|cx| {
                let text = Rc::new(move |cx: &App| {
                    tr(
                        cx,
                        ids::hv::CONN_TOAST_DUP,
                        ids::hv::HvConnToastDup {
                            name: Text(name.clone()),
                        },
                    )
                });
                ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
            });
            copied.map(|()| new_id).map_err(SaveError::Secret)
        })
    }

    /// Connects `id` through the manager and makes it current. `draft_password` is a password
    /// the user typed for this attempt only (the manager stores it after a successful connect).
    pub fn connect(cx: &mut App, id: ProfileId, draft_password: Option<SecretString>) {
        let ix = interaction(cx);
        let m = cx.global_mut::<ConnectionsModel>();
        let Some(profile) = m.get(id) else {
            return;
        };
        m.note_attempt(id);
        m.manager.connect(profile, draft_password, ix);
    }

    pub fn disconnect(cx: &mut App, id: ProfileId) {
        cx.global::<ConnectionsModel>().manager.disconnect(id);
    }

    pub fn cancel_connect(cx: &mut App, id: ProfileId) {
        cx.global::<ConnectionsModel>().manager.cancel(id);
    }

    /// Tests `draft` through the manager. Needs the model (call it only after `boot`).
    pub fn test(cx: &mut App, draft: ProfileDraft) -> TestHandle {
        Self::test_continuing(cx, draft, 0)
    }

    /// [`Self::test`] continuing the connection dialog's count of server rejections (P8).
    pub fn test_continuing(cx: &mut App, draft: ProfileDraft, prior_rejections: u8) -> TestHandle {
        let ix = interaction(cx);
        cx.global::<ConnectionsModel>()
            .manager
            .test_continuing(draft, prior_rejections, ix)
    }

    /// [`Self::connect`] continuing the connection dialog's count of server rejections (P8).
    pub fn connect_continuing(
        cx: &mut App,
        id: ProfileId,
        draft_password: Option<SecretString>,
        prior_rejections: u8,
    ) {
        let ix = interaction(cx);
        let m = cx.global_mut::<ConnectionsModel>();
        let Some(profile) = m.get(id) else {
            return;
        };
        m.note_attempt(id);
        m.manager
            .connect_continuing(profile, draft_password, prior_rejections, ix);
    }

    pub fn manager(&self) -> &Arc<ConnectionManager> {
        &self.manager
    }

    pub fn secrets(&self) -> &Arc<SecretStore> {
        &self.secrets
    }

    /// The core runtime's handle.
    pub fn runtime(&self) -> Handle {
        self.runtime.clone()
    }

    /// The core runtime itself, for the bounded shutdown at quit (P15, Task 18).
    pub fn core_runtime(&self) -> &Arc<CoreRuntime> {
        &self.core
    }

    /// Whether `connections.toml` cannot be written (P29).
    pub fn store_read_only(&self) -> bool {
        self.read_only
    }

    /// Writes a pending profile list now, on this thread (app quit).
    pub fn flush_writes(&self) {
        self.writer.flush();
    }

    /// Waits, at most `budget`, until every queued job (secret calls and profile writes) has
    /// run; `true` if the queue drained in time. Blocks this thread: only the bounded quit
    /// path calls it (P15, Task 18).
    pub fn drain_writes(&self, budget: Duration) -> bool {
        self.writer.drain(budget)
    }

    /// The profile list as the file should hold it.
    fn snapshot(&self) -> Vec<Profile> {
        self.profiles.iter().map(|p| (**p).clone()).collect()
    }
}

/// The error of a job the core runtime dropped (it shut down).
fn cancelled() -> SaveError {
    SaveError::Io("cancelled".to_string())
}

/// Shows the keyring fallback toast, once per session.
fn notice_keyring_unavailable(cx: &mut App) {
    let m = cx.global_mut::<ConnectionsModel>();
    if std::mem::replace(&mut m.keyring_notice_shown, true) {
        return;
    }
    let text = Rc::new(|cx: &App| tr(cx, ids::hv::CONN_KEYRING_UNAVAILABLE, NoArgs));
    ToastQueue::push(cx, Toast::new(ToastKind::Error, text));
}

/// Labels a keyring entry with the profile name (P44). A label is cosmetic: a failure is
/// logged (without the secret) and ignored.
fn label_entry(secrets: &SecretStore, key: SecretKey, name: &str) {
    if let Err(e) = secrets.set_label(&key, name) {
        tracing::debug!(slot = %key.slot.id(), error = ?e, "connections: labelling a secret failed");
    }
}

// ## m2b-t01-interfaces
/// Whether `slot` holds an SSH secret (kept under `Profile::ssh_retention`).
fn is_ssh_slot(slot: SecretSlot) -> bool {
    matches!(
        slot,
        SecretSlot::SshPassword { .. } | SecretSlot::SshPassphrase { .. }
    )
}

/// The SSH slots of `profile`'s hops with the draft's typed `secrets` (Q23): a password and a
/// passphrase slot per hop of the tunnel (and per typed entry), `None` where nothing was typed,
/// so an edit that changes `ssh_retention` still moves every hop's stored secrets. At most
/// `MAX_SSH_HOP + 1` hops.
fn ssh_slot_values(
    profile: &Profile,
    secrets: Vec<HopSecrets>,
) -> Vec<(SecretSlot, Option<SecretString>)> {
    let hops = match &profile.transport {
        TransportSpec::SshTunnel(spec) => spec.hops.len(),
        _ => 0,
    };
    let mut typed = secrets.into_iter();
    // Fix round 1 (M3): never beyond `MAX_SSH_HOP`, the last hop `delete_all` reaches.
    (0..hops.max(typed.len()))
        .map_while(|i| u8::try_from(i).ok())
        .take_while(|hop| *hop <= MAX_SSH_HOP)
        .flat_map(|hop| {
            let (password, passphrase) = typed
                .next()
                .map_or((None, None), |s| (s.password, s.passphrase));
            [
                (SecretSlot::SshPassword { hop }, password),
                (SecretSlot::SshPassphrase { hop }, passphrase),
            ]
        })
        .collect()
}

/// Moves the profile's stored secrets in `values`' slots from `old` to `retention` (an edit
/// that changed it), then stores the draft's new values (the password, the TLS key passphrase;
/// M2b: the SSH secrets) under `retention`. Returns whether the keyring refused (a fallback)
/// and the first hard error. Blocking: runs on the core runtime's blocking pool.
fn store_secrets(
    secrets: &SecretStore,
    profile: ProfileId,
    name: &str,
    old: Option<Retention>,
    retention: Retention,
    values: Vec<(SecretSlot, Option<SecretString>)>,
) -> (bool, Option<SecretError>) {
    let mut fell_back = false;
    let mut first_error = None;
    for (slot, value) in values {
        let key = SecretKey { profile, slot };
        if let Some(old) = old.filter(|old| *old != retention) {
            match secrets.change_retention(&key, old, retention) {
                Ok(Stored::FellBackTo { .. }) => fell_back = true,
                Ok(Stored::AsRequested) => {}
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        let Some(value) = value else {
            continue;
        };
        match secrets.put(&key, value, retention) {
            Ok(Stored::AsRequested) if retention == Retention::Forever => {
                label_entry(secrets, key, name);
            }
            Ok(Stored::AsRequested) => {}
            Ok(Stored::FellBackTo { .. }) => fell_back = true,
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    (fell_back, first_error)
}

// ## m2b-t21-prompts

/// A duplicate's secret copy, repeatable after the vault was unlocked: the DB slots under the
/// profile's retention and the SSH slots under `ssh_retention` (never `copy_all`, which would
/// put the SSH slots under the DB retention; T1 re-review N1), then the `Forever` copies are
/// labelled with the copy's name (P44).
struct CopySecrets {
    secrets: Arc<SecretStore>,
    from: ProfileId,
    to: ProfileId,
    retention: Retention,
    ssh_retention: Retention,
    ssh_labels: Vec<SecretSlot>,
    label: String,
}

impl CopySecrets {
    /// Blocking: runs on the core runtime's blocking pool.
    fn run(&self) -> Result<(), SecretError> {
        let not_ssh = |slot: SecretSlot| !is_ssh_slot(slot);
        let (secrets, from, to) = (&self.secrets, self.from, self.to);
        secrets
            .copy_slots(from, to, self.retention, &not_ssh)
            .and_then(|()| secrets.copy_slots(from, to, self.ssh_retention, &is_ssh_slot))?;
        let forever = |r: Retention| r == Retention::Forever;
        let slots = DRAFT_SLOTS
            .into_iter()
            .filter(|_| forever(self.retention))
            .chain(
                self.ssh_labels
                    .iter()
                    .copied()
                    .filter(|_| forever(self.ssh_retention)),
            );
        for slot in slots {
            label_entry(secrets, SecretKey { profile: to, slot }, &self.label);
        }
        Ok(())
    }
}

/// One retention of a save: the stored one (`None` for a new profile), the new one, and
/// whether the draft typed a secret kept under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Change {
    old: Option<Retention>,
    new: Retention,
    typed: bool,
}

/// Whether a save reaches the `Forever` backend: it puts a typed secret under `Forever`, moves
/// stored secrets into or out of `Forever`, or moves SSH slots that live under `Forever`.
fn touches_forever(db: Change, ssh: Change) -> bool {
    let forever = Retention::Forever;
    let reaches = |c: Change| {
        (c.typed && c.new == forever)
            || c.old
                .is_some_and(|old| old != c.new && (old == forever || c.new == forever))
    };
    reaches(db) || reaches(ssh)
}

/// A hop's identity for its stored secrets: host (ASCII-folded), port, user and sign-in method
/// (a key file's path included). A hop whose identity changed is a different hop.
fn hop_identity(hop: &HopSpec) -> (String, u16, &str, &SshAuth) {
    (
        hop.host.to_ascii_lowercase(),
        hop.port,
        &hop.user,
        &hop.auth,
    )
}

/// The SSH hops of `p` (none without a tunnel).
fn hops_of(p: &Profile) -> &[HopSpec] {
    match &p.transport {
        TransportSpec::SshTunnel(spec) => &spec.hops,
        _ => &[],
    }
}

/// How the stored SSH slots of `old` map onto `new`'s hops (T20 carry): for every hop of `new`,
/// the index of the same hop in `old` (`None`: a new or changed hop), plus how many hops `old`
/// had. `None` when every hop kept its place (nothing to move).
fn hop_remap(old: &Profile, new: &Profile) -> Option<HopRemap> {
    let (old, new) = (hops_of(old), hops_of(new));
    let mut used = vec![false; old.len()];
    let mut map = Vec::with_capacity(new.len());
    for (j, hop) in new.iter().enumerate() {
        let id = hop_identity(hop);
        // The same place first, then the first unused match.
        let at = (old
            .get(j)
            .is_some_and(|o| !used[j] && hop_identity(o) == id))
        .then_some(j)
        .or_else(|| {
            old.iter()
                .enumerate()
                .position(|(i, o)| !used[i] && hop_identity(o) == id)
        });
        if let Some(i) = at {
            used[i] = true;
        }
        map.push(at);
    }
    let unchanged = old.len() == new.len() && map.iter().enumerate().all(|(j, at)| *at == Some(j));
    (!unchanged).then_some(HopRemap {
        map,
        old_len: old.len(),
    })
}

/// The result of [`hop_remap`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HopRemap {
    /// For each new hop, the old hop it was.
    map: Vec<Option<usize>>,
    old_len: usize,
}

/// Moves the stored SSH slots (password, passphrase) of `profile` by `remap`, under `retention`
/// (where they are kept now), all or nothing (fix round 1, I1): every old hop's value is read
/// first (any read error other than "nothing stored" stops here, before any write); then the
/// moved values are written at their new index and every slot that is not kept in place and
/// got no value is removed. On the first failed write, everything written so far is put back as
/// it was (a slot that cannot be put back is removed, never left on the wrong hop) and the error
/// is returned. A moved `Forever` secret is labelled with `name` (P44). Returns whether a put
/// fell back to Until restart. Blocking: on the core runtime's blocking pool.
fn remap_ssh_slots(
    secrets: &SecretStore,
    profile: ProfileId,
    retention: Retention,
    remap: &HopRemap,
    name: &str,
) -> Result<bool, SecretError> {
    let key = |hop: usize, password: bool| {
        let hop = u8::try_from(hop).ok().filter(|h| *h <= MAX_SSH_HOP)?;
        let slot = if password {
            SecretSlot::SshPassword { hop }
        } else {
            SecretSlot::SshPassphrase { hop }
        };
        Some(SecretKey { profile, slot })
    };
    // 1. Read every slot of every old hop.
    let mut old: HashMap<SecretKey, SecretString> = HashMap::new();
    for hop in 0..remap.old_len {
        for password in [true, false] {
            let Some(k) = key(hop, password) else {
                continue;
            };
            match secrets.get(&k, retention) {
                Ok(Some(value)) => {
                    old.insert(k, value);
                }
                Ok(None) | Err(SecretError::Unavailable | SecretError::Denied) => {}
                Err(e) => return Err(e),
            }
        }
    }
    // 2. The writes: moved values first, then removals of what nothing replaces.
    let mut puts = Vec::new();
    let mut removals = Vec::new();
    for at in 0..remap.old_len.max(remap.map.len()) {
        let from = remap.map.get(at).copied().flatten();
        if from == Some(at) {
            continue;
        }
        for password in [true, false] {
            let Some(target) = key(at, password) else {
                continue;
            };
            let value = from
                .and_then(|from| key(from, password))
                .and_then(|source| old.get(&source).cloned());
            match value {
                Some(value) => puts.push((target, value)),
                None if old.contains_key(&target) => removals.push(target),
                None => {}
            }
        }
    }
    // 3. Apply; roll back on the first failure.
    let mut fell_back = false;
    let mut done: Vec<SecretKey> = Vec::new();
    let mut failed = None;
    for (target, value) in puts {
        match secrets.put(&target, value, retention) {
            Ok(Stored::AsRequested) => {
                if retention == Retention::Forever {
                    label_entry(secrets, target, name);
                }
                done.push(target);
            }
            Ok(Stored::FellBackTo { .. }) => {
                fell_back = true;
                done.push(target);
            }
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    if failed.is_none() {
        for target in removals {
            match secrets.remove(&target, retention) {
                Ok(()) => done.push(target),
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
    }
    let Some(e) = failed else {
        return Ok(fell_back);
    };
    for target in done {
        let restored = match old.get(&target) {
            Some(value) => secrets.put(&target, value.clone(), retention).map(|_| ()),
            None => secrets.remove(&target, retention),
        };
        if restored.is_err() {
            // Never a secret on the wrong hop: lose it rather than misplace it.
            let _ = secrets.remove(&target, retention);
        }
    }
    Err(e)
}

/// The two ends of a save's vault gate (M7): the job's (it says when a dialog is due and waits
/// for the answer) and the UI's. Both `None` unless `needed`.
type JobGate = (
    oneshot::Sender<vault_dialog::VaultState>,
    std::sync::mpsc::Receiver<vault_dialog::VaultReady>,
);
type UiGate = (
    oneshot::Receiver<vault_dialog::VaultState>,
    std::sync::mpsc::Sender<vault_dialog::VaultReady>,
);

fn vault_gate(needed: bool) -> (Option<JobGate>, Option<UiGate>) {
    needed
        .then(|| {
            let (need_tx, need_rx) = oneshot::channel::<vault_dialog::VaultState>();
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<vault_dialog::VaultReady>();
            ((need_tx, ready_rx), (need_rx, ready_tx))
        })
        .unzip()
}

/// The job's side, on the blocking pool: reads the vault's state and, when it is missing or
/// locked, waits for the UI's answer. A dropped channel (the app quit, the dialog could not
/// open) goes on; the secret calls then fall back or fail as without a vault (Q38).
fn pass_vault_gate(gate: Option<JobGate>, secrets: &SecretStore) {
    if let Some((need_tx, ready_rx)) = gate {
        let state = vault_dialog::vault_state(secrets);
        if state.needs_dialog() && need_tx.send(state).is_ok() {
            let _ = ready_rx.recv();
        }
    }
}

/// The UI's side: runs the Create or Unlock dialog when the job asks, and answers it.
fn answer_vault_gate(
    gate: Option<UiGate>,
    name: String,
    env: Option<hacivat_proto::EnvTag>,
    cx: &mut App,
) -> Task<()> {
    let Some((need_rx, ready_tx)) = gate else {
        return Task::ready(());
    };
    cx.spawn(async move |cx| {
        if let Ok(state) = need_rx.await {
            let ready =
                cx.update(|cx| vault_dialog::ensure_vault_before_forever(cx, state, name, env));
            let _ = ready_tx.send(ready.await);
        }
    })
}

/// Whether the vault may hold `Forever` secrets: any backend but the keyring or the memory one
/// (a backend not probed yet may still choose the vault).
fn maybe_vault(secrets: &SecretStore) -> bool {
    !matches!(
        secrets.backend_choice(),
        Some(BackendChoice::Keyring | BackendChoice::Memory)
    )
}

/// The SSH slot move of `id` is over: saves that waited for it go on.
fn finish_remap(cx: &mut App, id: ProfileId) {
    let waiting = cx
        .global_mut::<ConnectionsModel>()
        .remapping
        .remove(&id)
        .unwrap_or_default();
    for waiter in waiting {
        let _ = waiter.send(());
    }
}

/// Saves the stored profile `id` with `ssh_retention = retention` (the Remember of an SSH secret
/// prompt, reported by a successful connect; fix round 1). The save moves every hop's stored
/// SSH secrets to the new retention, behind the vault gate when Forever is involved.
fn apply_ssh_retention(
    cx: &mut App,
    id: ProfileId,
    retention: Retention,
) -> Task<Result<(), SaveError>> {
    let Some(stored) = cx.global::<ConnectionsModel>().get(id) else {
        return Task::ready(Err(SaveError::NotFound));
    };
    if stored.ssh_retention == retention {
        return Task::ready(Ok(()));
    }
    let mut profile = (*stored).clone();
    profile.ssh_retention = retention;
    let saved = ConnectionsModel::save(cx, ProfileDraft::edit(profile));
    cx.spawn(async move |_| saved.await.map(|_| ()))
}

/// One queued write: secret calls, then the profile list.
type Job = Box<dyn FnOnce() + Send>;

/// The latest profile list to write and the result of the last write.
struct Pending {
    profiles: Option<Vec<Profile>>,
    last: Result<(), StoreError>,
}

/// What a job's write did: its result, and a file it set aside first (P29).
struct Written {
    result: Result<(), StoreError>,
    quarantined: Option<PathBuf>,
}

impl Written {
    /// Shows the `hv-conn-file-quarantined` toast when the write set the file aside, and
    /// returns the write's result.
    fn surface(self, cx: &mut App) -> Result<(), SaveError> {
        if let Some(path) = self.quarantined {
            push_quarantined(cx, file_name(&path));
        }
        self.result.map_err(SaveError::from)
    }
}

/// The ordered writer of `connections.toml` (P48): a serial job queue on the core runtime and
/// a latest-wins slot.
struct ProfileWriter {
    store: Arc<ProfileStore>,
    pending: Arc<Mutex<Pending>>,
    jobs: mpsc::UnboundedSender<Job>,
}

impl ProfileWriter {
    fn new(store: ProfileStore, runtime: &Handle) -> Self {
        let (jobs, mut queue) = mpsc::unbounded_channel::<Job>();
        let blocking = runtime.clone();
        runtime.spawn(async move {
            while let Some(job) = queue.recv().await {
                // One job at a time; secret calls and file writes block.
                let _ = blocking.spawn_blocking(job).await;
            }
        });
        ProfileWriter {
            store: Arc::new(store),
            pending: Arc::new(Mutex::new(Pending {
                profiles: None,
                last: Ok(()),
            })),
            jobs,
        }
    }

    /// Makes `profiles` the list to write and queues `work` (secret calls) followed by the
    /// write. The receiver gets `work`'s result and the result of the write that carried
    /// `profiles` (a later job's list may already have replaced it: last wins).
    fn run<T: Send + 'static>(
        &self,
        profiles: Vec<Profile>,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> oneshot::Receiver<(T, Written)> {
        lock(&self.pending).profiles = Some(profiles);
        let (done, rx) = oneshot::channel();
        let pending = self.pending.clone();
        let store = self.store.clone();
        let job: Job = Box::new(move || {
            let out = work();
            let written = write_pending(&pending, &store);
            let _ = done.send((out, written));
        });
        // A closed queue (the runtime shut down) drops the job, which closes `rx`.
        let _ = self.jobs.send(job);
        rx
    }

    // ## m2b-t21-prompts (fix round 1, I1)
    /// Queues `work` (secret calls only) in order with the other jobs; no profile list is set
    /// or written. The receiver gets `work`'s result.
    fn run_only<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> oneshot::Receiver<T> {
        let (done, rx) = oneshot::channel();
        let job: Job = Box::new(move || {
            let _ = done.send(work());
        });
        // A closed queue (the runtime shut down) drops the job, which closes `rx`.
        let _ = self.jobs.send(job);
        rx
    }

    /// Writes the pending list now; a failure is already logged by `write_pending`.
    fn flush(&self) {
        let _ = write_pending(&self.pending, &self.store);
    }

    /// Queues a marker job and waits for it at most `budget`: jobs run in order, so when it
    /// runs every earlier job is done. A closed queue has nothing left to run.
    fn drain(&self, budget: Duration) -> bool {
        let (done, wait) = std::sync::mpsc::channel();
        let marker: Job = Box::new(move || {
            let _ = done.send(());
        });
        if self.jobs.send(marker).is_err() {
            return true;
        }
        wait.recv_timeout(budget).is_ok()
    }
}

/// Writes the pending list, if any, under the slot's lock (so writes never overlap or reorder);
/// with nothing pending, the last write's result stands. Only the write that set the file
/// aside reports it.
fn write_pending(pending: &Mutex<Pending>, store: &ProfileStore) -> Written {
    let mut slot = lock(pending);
    let mut quarantined = None;
    if let Some(profiles) = slot.profiles.take() {
        slot.last = store.save_reporting(&profiles).map(|report| {
            quarantined = report.quarantined;
        });
        if let Err(e) = &slot.last {
            tracing::warn!(error = ?e, "connections: saving connections.toml failed");
        }
    }
    Written {
        result: slot.last.clone(),
        quarantined,
    }
}

fn lock(m: &Mutex<Pending>) -> std::sync::MutexGuard<'_, Pending> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// Toast helpers for the store diagnostics (P29) live here so every connections toast is built
// the same way.

/// The file name a quarantined file was saved under.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn push_quarantined(cx: &mut App, file: String) {
    let text = Rc::new(move |cx: &App| {
        tr(
            cx,
            ids::hv::CONN_FILE_QUARANTINED,
            ids::hv::HvConnFileQuarantined {
                file: Text(file.clone()),
            },
        )
    });
    ToastQueue::push(cx, Toast::new(ToastKind::Error, text));
}

/// A toast for a count of `id`-style problems (entries skipped, values ignored).
fn push_count(
    cx: &mut App,
    kind: ToastKind,
    count: usize,
    render: fn(&App, Count) -> SharedString,
) {
    if count == 0 {
        return;
    }
    let count = u64::try_from(count).unwrap_or(u64::MAX);
    let text = Rc::new(move |cx: &App| render(cx, Count(count)));
    ToastQueue::push(cx, Toast::new(kind, text));
}

/// One toast per kind of store diagnostic (P29), in a fixed order: the file set aside, the
/// file that cannot be read (or set aside), the skipped entries, the ignored values, an
/// ignored password (never echoed), a newer file.
pub(crate) fn push_store_toasts(cx: &mut App, diags: &[ProfileDiag], quarantined: Option<&Path>) {
    if let Some(path) = quarantined {
        push_quarantined(cx, file_name(path));
    }
    let any = |f: fn(&ProfileDiag) -> bool| diags.iter().any(f);
    let count = |f: fn(&ProfileDiag) -> bool| diags.iter().filter(|d| f(d)).count();
    if any(|d| {
        matches!(
            d,
            ProfileDiag::Unreadable { .. } | ProfileDiag::QuarantineFailed { .. }
        )
    }) {
        let text = Rc::new(|cx: &App| tr(cx, ids::hv::CONN_FILE_UNREADABLE, NoArgs));
        ToastQueue::push(cx, Toast::new(ToastKind::Error, text));
    }
    push_count(
        cx,
        ToastKind::Error,
        count(|d| {
            matches!(
                d,
                ProfileDiag::InvalidEntry { .. }
                    | ProfileDiag::DuplicateId { .. }
                    | ProfileDiag::UnknownEngine { .. }
            )
        }),
        |cx, count| {
            tr(
                cx,
                ids::hv::CONN_FILE_ENTRIES_SKIPPED,
                ids::hv::HvConnFileEntriesSkipped { count },
            )
        },
    );
    push_count(
        cx,
        ToastKind::Info,
        count(|d| matches!(d, ProfileDiag::ValueIgnored { .. })),
        |cx, count| {
            tr(
                cx,
                ids::hv::CONN_FILE_VALUES_IGNORED,
                ids::hv::HvConnFileValuesIgnored { count },
            )
        },
    );
    if any(|d| {
        matches!(
            d,
            ProfileDiag::SecretKeyIgnored { .. } | ProfileDiag::SecretKeyOutsideEntries
        )
    }) {
        let text = Rc::new(|cx: &App| tr(cx, ids::hv::CONN_FILE_PASSWORD_IGNORED, NoArgs));
        ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
    }
    if any(|d| matches!(d, ProfileDiag::NewerVersion { .. })) {
        let text = Rc::new(|cx: &App| tr(cx, ids::hv::CONN_STORE_READ_ONLY, NoArgs));
        ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
    }
}
