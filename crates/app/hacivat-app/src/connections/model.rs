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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_proto::{Origin, ProfileId, TagSource};
use hacivat_secrets::{Retention, SecretError, SecretKey, SecretSlot, SecretStore, Stored};
use hacivat_session::profile::{
    Profile, ProfileDiag, ProfileDraft, ProfileStore, StoreError, duplicate_name,
};
use hacivat_session::{ConnectionManager, CoreRuntime, LiveState, TestHandle};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Global, SharedString, Task};
use secrecy::SecretString;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use crate::connections::interaction::interaction;
use crate::l10n_global::tr;

/// Why a profile write failed (M2a).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SaveError {
    StoreReadOnly,
    Io(String),
    Secret(SecretError),
    /// The profile is not in the list (deleted meanwhile): an edit never brings it back.
    NotFound,
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
        }
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
        let m = cx.global::<ConnectionsModel>();
        if m.read_only {
            return Task::ready(Err(SaveError::StoreReadOnly));
        }
        let ProfileDraft {
            mut profile,
            password,
            tls_key_passphrase,
            is_new,
            ..
        } = draft;
        let id = profile.id;
        let stored = m.get(id);
        if !is_new && stored.is_none() {
            return Task::ready(Err(SaveError::NotFound));
        }
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
        let old_retention = stored.map(|p| p.retention);
        let retention = profile.retention;
        let name = profile.name.clone();
        let secrets = m.secrets.clone();

        let m = cx.global_mut::<ConnectionsModel>();
        let profile = Arc::new(profile);
        match m.profiles.iter_mut().find(|p| p.id == id) {
            Some(slot) => *slot = profile,
            None => m.profiles.push(profile),
        }
        let done = m.writer.run(m.snapshot(), move || {
            let values = [password, tls_key_passphrase];
            store_secrets(&secrets, id, &name, old_retention, retention, values)
        });
        cx.spawn(async move |cx| {
            let ((fell_back, secret), written) = done.await.map_err(|_| cancelled())?;
            if fell_back {
                cx.update(notice_keyring_unavailable);
            }
            cx.update(|cx| written.surface(cx))?;
            secret.map_or(Ok(id), |e| Err(SaveError::Secret(e)))
        })
    }

    /// Deletes `id`: the manager forgets it (cancels, disconnects, drops its session secrets),
    /// the profile leaves the list and the file, and every secret slot is deleted. A secret
    /// that cannot be deleted is reported after the profile is gone.
    pub fn delete(cx: &mut App, id: ProfileId) -> Task<Result<(), SaveError>> {
        if cx.global::<ConnectionsModel>().read_only {
            return Task::ready(Err(SaveError::StoreReadOnly));
        }
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
        let done = m.writer.run(m.snapshot(), move || secrets.delete_all(id));
        cx.spawn(async move |cx| {
            let (deleted, written) = done.await.map_err(|_| cancelled())?;
            cx.update(|cx| written.surface(cx))?;
            deleted.map_err(SaveError::Secret)
        })
    }

    /// Saves a copy of `id` right after it, named `<name>-2` (or the next free number), with
    /// copies of its secrets, selects it and shows the `hv-conn-toast-dup` toast. The copy is
    /// the user's own: its tag is the user's and its origin `UserTyped` (D-004), even when the
    /// original is managed by the organization.
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
        copy.origin = Origin::UserTyped;
        let (new_id, name, retention) = (copy.id, copy.name.clone(), copy.retention);
        let secrets = m.secrets.clone();

        let m = cx.global_mut::<ConnectionsModel>();
        m.profiles.insert(at + 1, Arc::new(copy));
        m.selected = Some(new_id);
        let label = name.clone();
        let done = m.writer.run(m.snapshot(), move || {
            let copied = secrets.copy_all(id, new_id, retention);
            if copied.is_ok() && retention == Retention::Forever {
                for slot in DRAFT_SLOTS {
                    label_entry(
                        &secrets,
                        SecretKey {
                            profile: new_id,
                            slot,
                        },
                        &label,
                    );
                }
            }
            copied
        });
        cx.spawn(async move |cx| {
            let (copied, written) = done.await.map_err(|_| cancelled())?;
            cx.update(|cx| written.surface(cx))?;
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
        let ix = interaction(cx);
        cx.global::<ConnectionsModel>().manager.test(draft, ix)
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

/// Moves the profile's stored secrets from `old` to `retention` (an edit that changed it),
/// then stores the draft's new `values` (the password, the TLS key passphrase) under
/// `retention`. Returns whether the keyring refused (a fallback) and the first hard error.
/// Blocking: runs on the core runtime's blocking pool.
fn store_secrets(
    secrets: &SecretStore,
    profile: ProfileId,
    name: &str,
    old: Option<Retention>,
    retention: Retention,
    values: [Option<SecretString>; 2],
) -> (bool, Option<SecretError>) {
    let mut fell_back = false;
    let mut first_error = None;
    for (slot, value) in DRAFT_SLOTS.into_iter().zip(values) {
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
