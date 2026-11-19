//! Per-profile explorer data (Task 20): levels, view filters and options kept in
//! `profiles/<uuid>.toml` through `hacivat_settings::profile_data` (this module is the app's only
//! caller of it).
//!
//! ## m4-t20-scope
//!
//! **The section.** Each profile's [`ProfileUserData`] is the `[metadata]` table of its data
//! file ([`SECTION`]); other owners' tables in the file are never touched
//! (`ProfileDataDoc::save_section`).
//!
//! **Threading (§5.4).** Every read and write runs on the core runtime: one writer task takes
//! the requests in order and does each read-modify-write on the blocking pool, so two changes
//! made in a row land in that order. The UI holds the last value read or written
//! ([`get`]); until a profile's file has been read, [`get`] answers `None` and callers claim
//! nothing about its data (the explorer then draws no level pill, never a wrong one).
//!
//! **Fail-closed.** A file that is not TOML, mixes line endings, or holds a `[metadata]` table
//! of the wrong shape is never rewritten (another owner's data, or ours, would be lost): the
//! profile's data stays unknown and every change is refused with the reason.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use hacivat_meta::userdata::ProfileUserData;
use hacivat_meta::{Level, ObjectKey};
use hacivat_proto::ProfileId;
use hacivat_settings::LocalPaths;
use hacivat_settings::profile_data::{ProfileDataDoc, ProfileDataError};
use hacivat_ui_core::gpui_surface::{App, Global, Task};
use tokio::sync::{mpsc, oneshot};

use crate::connections::ConnectionsModel;
use crate::shell::AppState;

/// The profile data file's table that holds the catalog user data.
pub const SECTION: &str = "metadata";

/// Why a profile's user data could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserDataError {
    /// No core runtime or local paths (a shell without connections).
    Unavailable,
    /// The file (or its `[metadata]` table) cannot be read safely: it is never rewritten.
    Unreadable,
    /// The file could not be written.
    Io(String),
}

impl fmt::Display for UserDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UserDataError::Unavailable => f.write_str("profile data is unavailable"),
            UserDataError::Unreadable => f.write_str("profile data file is unreadable"),
            UserDataError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for UserDataError {}

/// A change to one profile's data, applied to the value read from disk.
type Change = Arc<dyn Fn(&mut ProfileUserData) + Send + Sync>;

/// One request to the writer.
struct Request {
    /// `None`: a barrier.
    profile: Option<ProfileId>,
    /// `None`: read only.
    change: Option<Change>,
    reply: oneshot::Sender<Result<Arc<ProfileUserData>, UserDataError>>,
}

#[derive(Clone)]
enum Entry {
    Loading,
    Loaded(Arc<ProfileUserData>),
    Unreadable,
}

/// The user data of the profiles the UI asked for, and the writer's queue.
#[derive(Default)]
pub struct UserData {
    entries: HashMap<ProfileId, Entry>,
    /// Moves whenever a value changes (the explorer rebuilds its rows).
    generation: u64,
    /// The latest request per profile whose answer the UI applies (an older answer that lands
    /// after a newer local change is ignored).
    seq: HashMap<ProfileId, u64>,
    writer: Option<mpsc::UnboundedSender<Request>>,
    /// Reads and writes sent ([`requests`]).
    requests: u64,
}

impl Global for UserData {}

/// `profile`'s data, once its file has been read (`None` before, or when it cannot be read).
pub fn get(profile: ProfileId, cx: &App) -> Option<Arc<ProfileUserData>> {
    match cx.try_global::<UserData>()?.entries.get(&profile)? {
        Entry::Loaded(d) => Some(d.clone()),
        Entry::Loading | Entry::Unreadable => None,
    }
}

/// Whether `profile`'s data file is known to be unreadable (changes are refused).
pub fn is_unreadable(profile: ProfileId, cx: &App) -> bool {
    cx.try_global::<UserData>()
        .and_then(|g| g.entries.get(&profile))
        .is_some_and(|e| matches!(e, Entry::Unreadable))
}

/// Moves whenever a profile's data changes.
pub fn generation(cx: &App) -> u64 {
    cx.try_global::<UserData>().map_or(0, |g| g.generation)
}

/// The explicit introspection level of `key` in `profile`'s data (the pills, mock 6).
pub fn level_override(profile: ProfileId, key: &ObjectKey, cx: &App) -> Option<Level> {
    get(profile, cx)?.level_overrides.get(key).copied()
}

/// Reads `profile`'s file on the core runtime unless it was read (or is being read). Only a
/// profile not asked for before takes the global mutably: a mutable access notifies its
/// observers, and the explorer calls this from its own observer.
pub fn ensure(profile: ProfileId, cx: &mut App) {
    if cx
        .try_global::<UserData>()
        .is_some_and(|g| g.entries.contains_key(&profile))
    {
        return;
    }
    let Some(rx) = send(profile, None, cx) else {
        return;
    };
    cx.default_global::<UserData>()
        .entries
        .insert(profile, Entry::Loading);
    let seq = bump(profile, cx);
    land(profile, seq, false, rx, cx).detach();
}

/// How many reads and writes were sent to the writer (tests: no loop).
pub fn requests(cx: &App) -> u64 {
    cx.try_global::<UserData>().map_or(0, |g| g.requests)
}

/// Resolves once every request sent before it has been answered (the writer runs them in
/// order): tests wait on it instead of a fixed time.
pub fn barrier(cx: &mut App) -> Task<()> {
    let Some(rx) = send_request(None, None, cx) else {
        return Task::ready(());
    };
    cx.spawn(async move |_| {
        let _ = rx.await;
    })
}

/// Forgets every value read, so the next [`ensure`] reads the files again (tests: a restart).
pub fn forget_all(cx: &mut App) {
    if let Some(g) = cx.try_global::<UserData>()
        && g.entries.is_empty()
    {
        return;
    }
    let g = cx.default_global::<UserData>();
    g.entries.clear();
    g.generation += 1;
}

/// Applies `change` to `profile`'s data: at once to the value the UI holds (when read), then
/// to the file, on the core runtime, in request order. The task answers with the saved value.
pub fn update(
    profile: ProfileId,
    change: impl Fn(&mut ProfileUserData) + Send + Sync + 'static,
    cx: &mut App,
) -> Task<Result<Arc<ProfileUserData>, UserDataError>> {
    if is_unreadable(profile, cx) {
        return Task::ready(Err(UserDataError::Unreadable));
    }
    let change: Change = Arc::new(change);
    let Some(rx) = send(profile, Some(change.clone()), cx) else {
        return Task::ready(Err(UserDataError::Unavailable));
    };
    let g = cx.default_global::<UserData>();
    if let Some(Entry::Loaded(d)) = g.entries.get(&profile) {
        let mut next = (**d).clone();
        change(&mut next);
        g.entries.insert(profile, Entry::Loaded(Arc::new(next)));
        g.generation += 1;
    }
    let seq = bump(profile, cx);
    land(profile, seq, true, rx, cx)
}

/// Sets (`Some`) or clears (`None`, "Automatic") the explicit level of `key` in `profile`'s
/// data.
pub fn set_level_override(
    profile: ProfileId,
    key: ObjectKey,
    level: Option<Level>,
    cx: &mut App,
) -> Task<Result<Arc<ProfileUserData>, UserDataError>> {
    update(
        profile,
        move |d| match level {
            Some(l) => {
                d.level_overrides.insert(key.clone(), l);
            }
            None => {
                d.level_overrides.remove(&key);
            }
        },
        cx,
    )
}

/// Sets `profile`'s scope pattern (`None`: the engine's default, R-M4-14).
pub fn set_scope(
    profile: ProfileId,
    scope: Option<String>,
    cx: &mut App,
) -> Task<Result<Arc<ProfileUserData>, UserDataError>> {
    update(profile, move |d| d.scope = scope.clone(), cx)
}

fn bump(profile: ProfileId, cx: &mut App) -> u64 {
    let seq = cx
        .default_global::<UserData>()
        .seq
        .entry(profile)
        .or_default();
    *seq += 1;
    *seq
}

/// Applies the writer's answer to request `seq` of `profile` (a `write`, or a read) when no
/// newer request was made.
fn land(
    profile: ProfileId,
    seq: u64,
    write: bool,
    rx: oneshot::Receiver<Result<Arc<ProfileUserData>, UserDataError>>,
    cx: &mut App,
) -> Task<Result<Arc<ProfileUserData>, UserDataError>> {
    cx.spawn(async move |cx| {
        let answer = rx.await.unwrap_or(Err(UserDataError::Unavailable));
        let applied = answer.clone();
        cx.update(|cx| {
            let g = cx.default_global::<UserData>();
            if g.seq.get(&profile).copied() != Some(seq) {
                return;
            }
            // Fix round 1 (I-1): a failed read is final (the file is unreadable: never read
            // again in a loop); a failed write leaves the file as it was, so it is read again
            // once, and that read is final in turn.
            let read_again = match applied {
                Ok(d) => {
                    g.entries.insert(profile, Entry::Loaded(d));
                    false
                }
                Err(UserDataError::Io(_) | UserDataError::Unavailable) if write => {
                    g.entries.remove(&profile);
                    true
                }
                Err(_) => {
                    g.entries.insert(profile, Entry::Unreadable);
                    false
                }
            };
            g.generation += 1;
            if read_again {
                ensure(profile, cx);
            }
        });
        answer
    })
}

/// Queues a request with the writer (started on first use); `None` without a core runtime.
fn send(
    profile: ProfileId,
    change: Option<Change>,
    cx: &mut App,
) -> Option<oneshot::Receiver<Result<Arc<ProfileUserData>, UserDataError>>> {
    send_request(Some(profile), change, cx)
}

/// [`send`]; `profile` `None` is a barrier (nothing is read or written).
fn send_request(
    profile: Option<ProfileId>,
    change: Option<Change>,
    cx: &mut App,
) -> Option<oneshot::Receiver<Result<Arc<ProfileUserData>, UserDataError>>> {
    let paths = cx.try_global::<AppState>()?.paths.clone();
    let runtime = cx.try_global::<ConnectionsModel>()?.runtime();
    let g = cx.default_global::<UserData>();
    if profile.is_some() {
        g.requests += 1;
    }
    let writer = match &g.writer {
        Some(w) if !w.is_closed() => w.clone(),
        _ => {
            let (tx, rx) = mpsc::unbounded_channel();
            runtime.spawn(write_loop(paths, runtime.clone(), rx));
            g.writer = Some(tx.clone());
            tx
        }
    };
    let (reply, answer) = oneshot::channel();
    writer
        .send(Request {
            profile,
            change,
            reply,
        })
        .ok()?;
    Some(answer)
}

/// The writer: one request at a time, each on the blocking pool.
async fn write_loop(
    paths: LocalPaths,
    runtime: tokio::runtime::Handle,
    mut rx: mpsc::UnboundedReceiver<Request>,
) {
    while let Some(req) = rx.recv().await {
        let paths = paths.clone();
        let Request {
            profile,
            change,
            reply,
        } = req;
        let Some(profile) = profile else {
            // A barrier: every request before it has been answered.
            let _ = reply.send(Ok(Arc::default()));
            continue;
        };
        let done = runtime
            .spawn_blocking(move || read_modify_write(&paths, profile, change.as_deref()))
            .await
            .unwrap_or(Err(UserDataError::Unavailable));
        let _ = reply.send(done);
    }
}

/// Reads `profile`'s `[metadata]` table (the defaults when the file or the table is missing)
/// and, with a `change`, writes the changed table back.
fn read_modify_write(
    paths: &LocalPaths,
    profile: ProfileId,
    change: Option<&(dyn Fn(&mut ProfileUserData) + Send + Sync)>,
) -> Result<Arc<ProfileUserData>, UserDataError> {
    let uuid = profile.0.to_string();
    let mut doc = ProfileDataDoc::load(paths, &uuid).map_err(classify)?;
    let mut data: ProfileUserData = doc.section(SECTION).map_err(classify)?.unwrap_or_default();
    if let Some(change) = change {
        change(&mut data);
        doc.save_section(SECTION, &data).map_err(classify)?;
    }
    Ok(Arc::new(data))
}

fn classify(e: ProfileDataError) -> UserDataError {
    match e {
        ProfileDataError::Corrupt
        | ProfileDataError::InvalidSection(_)
        | ProfileDataError::MixedLineEndings
        | ProfileDataError::InvalidProfileId => UserDataError::Unreadable,
        ProfileDataError::Encode(e) => UserDataError::Io(e),
        ProfileDataError::Io(e) => UserDataError::Io(e.to_string()),
    }
}
