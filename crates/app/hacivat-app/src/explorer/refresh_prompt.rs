//! Refresh after DDL (Task 27; mock 9): the Ask toast, the "waits for COMMIT" stamp on a
//! namespace, and the broad-refresh notice for dynamic SQL.
//!
//! ## m4-t27-refresh-ui
//!
//! **One coordinator per open catalog** (T21's `RefreshCoordinator`, next to the profile's
//! `ProfileCatalog`): made when the live host opens a profile's catalog
//! ([`catalog_opened`]) and dropped with its engine ([`catalog_closed`]). The refreshes it owed
//! for want of a connection are kept and run by the next one once the profile is connected
//! again ([`reconnected`], T21 m6). What a session whose transaction is still open waits to
//! commit is carried to the next coordinator too (fix round I-1: Clear cache or a profile Save
//! mid-transaction). Its preference is `metadata.refresh_after_ddl`, set again whenever the
//! setting changes ([`preference_changed`]).
//!
//! **Runs** (the orchestrator's named lines): every run on a profile with a coordinator carries
//! the coordinator's inbox and the classifier's reports ([`catalog_run`]); the exec service
//! installs `CatalogRefreshSink` in the run's auditor chain, so a `CREATE TABLE` run in any
//! console reaches the explorer. The sink never waits (T21). When the run ended
//! ([`run_ended`]) and when the toolbar ended a transaction ([`tx_ended`], through the inbox,
//! after the run's own messages), the targets waiting for each session's `COMMIT` are read
//! again on the core runtime and drawn as the namespace's amber stamp; the console's output
//! says what waits. A session that no console holds any more is told to the coordinator
//! ([`session_closed`]).
//!
//! **Counts** (ruling R-W5-10, [`summarize`]): the Ask, the stamp and the output count changed
//! objects (one `CREATE TABLE` is one); the planner's name-list and scope targets count
//! nothing; a schema-wide change (a namespace or database target: dynamic SQL, a widened plan,
//! an opaque run) names its schema with no count.
//!
//! **Events.** `Ask` shows a wide sticky toast that never takes the focus and never blocks a
//! run (NOTES ruling 9): Refresh, Always refresh (sets the preference) and Not now; Escape
//! or its ✕ answers Not now (NOTES §9, review m-1). `Stale`
//! needs nothing here for namespaces (the engine publishes them stale: the "out of date"
//! badge); `scope: true` badges the stale schema list until it is read again.
//!
//! **Dynamic SQL** (`EXECUTE`, `DO`, `CALL` with an opaque catalog effect): with the preference
//! Always and no transaction open, the run's end shows "Refreshing all of {schema}" and why.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use hacivat_exec::{CatalogRun, StmtKind, StmtReport};
use hacivat_introspect::IntroHandle;
use hacivat_introspect::ddl_effects::{RefreshPlan, Target, lower, ns_of_key};
use hacivat_introspect::refresh_coord::{CoordEvent, RefreshCoordinator, RefreshMsg};
use hacivat_l10n::ArgValue;
use hacivat_meta::{CatalogSnapshot, NsKey, ObjectKey};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_session::session::{SessionId, TxOutcome};
use hacivat_settings::schema::RefreshAfterDdl;
use hacivat_ui::icon::IconName;
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastAction, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Global, Task};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::live;
use crate::console::{ConsoleId, Consoles};
use crate::shell::{AppState, update_settings};

/// One profile's coordinator and what forwards its events to the UI.
struct Coord {
    coord: Arc<RefreshCoordinator>,
    handle: IntroHandle,
    stop: CancellationToken,
    _ui: Task<()>,
}

impl Drop for Coord {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// The coordinators and what their runs left behind (a global).
#[derive(Default)]
pub struct Coordinators {
    by_profile: HashMap<ProfileId, Coord>,
    /// Refreshes a closed coordinator owed, for the profile's next one.
    owed: HashMap<ProfileId, Vec<Target>>,
    /// The sessions each profile's runs went through.
    sessions: HashMap<ProfileId, HashSet<SessionId>>,
    /// Per profile and session: what waits for its COMMIT, per namespace.
    pending: HashMap<ProfileId, HashMap<SessionId, Changes>>,
    /// Per profile: what open transactions wait to commit, carried from a closed coordinator to
    /// the next one (fix round I-1: Clear cache or a profile Save mid-transaction).
    carried: HashMap<ProfileId, Vec<(SessionId, Vec<Target>)>>,
    /// Per console: the dynamic SQL its last run held (`EXECUTE`, `DO`, `CALL`) and its schema.
    broad: HashMap<ConsoleId, (String, String)>,
    /// The console name of the profile's last run (the Ask toast's body).
    last_console: HashMap<ProfileId, String>,
    /// The Ask toast on screen per profile: (question, toast).
    ask_toasts: HashMap<ProfileId, (u64, u64)>,
    /// Asks shown so far (tests).
    asks: u64,
    observing: bool,
}

impl Global for Coordinators {}

fn all_mut(cx: &mut App) -> Option<&mut Coordinators> {
    if cx.has_global::<Coordinators>() {
        Some(cx.global_mut::<Coordinators>())
    } else {
        None
    }
}

fn preference(cx: &App) -> RefreshAfterDdl {
    cx.try_global::<AppState>()
        .map_or_else(RefreshAfterDdl::default, |s| {
            s.settings.metadata.refresh_after_ddl
        })
}

fn runtime(cx: &App) -> Option<tokio::runtime::Handle> {
    cx.try_global::<crate::connections::ConnectionsModel>()
        .map(|m| m.runtime())
}

fn coord(profile: ProfileId, cx: &App) -> Option<Arc<RefreshCoordinator>> {
    cx.try_global::<Coordinators>()?
        .by_profile
        .get(&profile)
        .map(|c| c.coord.clone())
}

/// The coordinator of `profile`'s open catalog (tests, M7's Modify Object).
pub fn coordinator(profile: ProfileId, cx: &App) -> Option<Arc<RefreshCoordinator>> {
    coord(profile, cx)
}

/// How many Ask toasts were shown (tests).
pub fn asks(cx: &App) -> u64 {
    cx.try_global::<Coordinators>().map_or(0, |c| c.asks)
}

/// The live host opened `profile`'s catalog (engine `handle`, engine kind `dbms`): its
/// coordinator starts, with the preference in force.
pub fn catalog_opened(profile: ProfileId, handle: IntroHandle, dbms: DbmsKind, cx: &mut App) {
    let Some(rt) = runtime(cx) else {
        return;
    };
    let pref = preference(cx);
    let coord = Arc::new(RefreshCoordinator::new(
        handle.clone(),
        dbms,
        pref,
        rt.clone(),
    ));
    let (tx, mut rx) = mpsc::unbounded_channel::<CoordEvent>();
    let mut events = coord.events();
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    rt.spawn(async move {
        loop {
            tokio::select! {
                () = stopped.cancelled() => break,
                e = events.recv() => match e {
                    Ok(e) => {
                        if tx.send(e).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                },
            }
        }
    });
    let ui = cx.spawn(async move |cx| {
        while let Some(e) = rx.recv().await {
            cx.update(|cx| on_event(profile, e, cx));
        }
    });
    // Fix round I-1: what open transactions wait to commit survives the reopen; the new
    // coordinator keeps it pending for the same sessions.
    let carried = cx
        .default_global::<Coordinators>()
        .carried
        .remove(&profile)
        .unwrap_or_default();
    for (session, pending) in &carried {
        drop(coord.submit(
            *session,
            RefreshPlan {
                now: Vec::new(),
                pending: pending.clone(),
            },
        ));
    }
    let all = cx.default_global::<Coordinators>();
    all.by_profile.insert(
        profile,
        Coord {
            coord,
            handle,
            stop,
            _ui: ui,
        },
    );
    if !carried.is_empty() {
        all.sessions
            .entry(profile)
            .or_default()
            .extend(carried.iter().map(|(s, _)| *s));
    }
    observe(cx);
    if !carried.is_empty() {
        update_pending(profile, None, cx);
    }
}

/// The live host closes `profile`'s catalog: its coordinator stops; what it owed waits for the
/// next one. What a session whose user transaction is still open waits to commit is carried to
/// the next coordinator too (fix round I-1: a Clear cache or a profile Save mid-transaction);
/// any other session's transaction ended with the connection.
pub fn catalog_closed(profile: ProfileId, cx: &mut App) {
    let open_tx: HashSet<SessionId> = Consoles::ids(cx)
        .into_iter()
        .filter_map(|c| Consoles::get(cx, c))
        .filter(|s| s.profile == Some(profile))
        .filter_map(|s| s.session.as_ref())
        .filter(|s| crate::exec_ui::tx::user_tx_of(s).is_some())
        .map(|s| s.id())
        .collect();
    let Some(all) = all_mut(cx) else {
        return;
    };
    if let Some(c) = all.by_profile.remove(&profile) {
        let owed = c.coord.owed();
        if !owed.is_empty() {
            all.owed.entry(profile).or_default().extend(owed);
        }
        // What the inbox still holds is planned first, so nothing of a finished run is lost.
        c.coord.drain();
        let mut carried: Vec<(SessionId, Vec<Target>)> = all
            .sessions
            .get(&profile)
            .into_iter()
            .flatten()
            .filter(|s| open_tx.contains(s))
            .map(|s| (*s, c.coord.pending(*s)))
            .filter(|(_, p)| !p.is_empty())
            .collect();
        carried.sort_by_key(|(s, _)| *s);
        if !carried.is_empty() {
            all.carried.insert(profile, carried);
        }
    }
    all.pending.remove(&profile);
    all.sessions.remove(&profile);
    if let Some((_, toast)) = all.ask_toasts.remove(&profile) {
        ToastQueue::dismiss(cx, toast);
    }
}

/// `profile` is connected again (T21 m6): the refreshes owed since a lost connection run now,
/// also those a coordinator closed meanwhile owed.
pub fn reconnected(profile: ProfileId, cx: &mut App) {
    let Some(c) = coord(profile, cx) else {
        return;
    };
    drop(c.reconnected());
    let owed = all_mut(cx)
        .and_then(|a| a.owed.remove(&profile))
        .unwrap_or_default();
    if !owed.is_empty() {
        drop(c.refresh_now(owed));
    }
}

/// `metadata.refresh_after_ddl` changed: every coordinator follows it.
pub fn preference_changed(cx: &mut App) {
    let pref = preference(cx);
    if let Some(all) = cx.try_global::<Coordinators>() {
        for c in all.by_profile.values() {
            c.coord.set_preference(pref);
        }
    }
}

/// The dynamic SQL keyword of `reports`, if a statement there runs code whose catalog effects
/// the classifier cannot see (`EXECUTE`, `DO`, `CALL`).
fn dynamic_sql(reports: &[Option<Vec<StmtReport>>]) -> Option<&'static str> {
    reports.iter().flatten().flatten().find_map(|r| {
        let opaque = r.ddl.iter().any(|e| e.opaque);
        match r.kind {
            StmtKind::Execute if opaque => Some("EXECUTE"),
            StmtKind::Do if opaque => Some("DO"),
            StmtKind::Call if opaque => Some("CALL"),
            _ => None,
        }
    })
}

/// What a run of console `console` on `profile` tells the profile's coordinator (`None`
/// without one: the catalog is not open, nothing to refresh). `reports` are the classifier's,
/// one per unit.
pub fn catalog_run(
    console: ConsoleId,
    profile: ProfileId,
    dbms: DbmsKind,
    reports: Vec<Option<Vec<StmtReport>>>,
    cx: &mut App,
) -> Option<CatalogRun> {
    let c = coord(profile, cx)?;
    let session = Consoles::get(cx, console)?.session.clone()?;
    let name = crate::exec_ui::orchestrator::console_name(cx, console);
    let broad = dynamic_sql(&reports).map(|what| {
        let snap = session.state();
        let schema = snap
            .borrow()
            .schema
            .as_ref()
            .and_then(|s| s.schema.clone().or_else(|| s.database.clone()))
            .unwrap_or_default();
        (what.to_string(), schema)
    });
    let all = cx.default_global::<Coordinators>();
    all.sessions
        .entry(profile)
        .or_default()
        .insert(session.id());
    all.last_console.insert(profile, name);
    match broad {
        Some(b) => {
            all.broad.insert(console, b);
        }
        None => {
            all.broad.remove(&console);
        }
    }
    Some(CatalogRun {
        inbox: c.inbox(),
        dialect: live::dialect_of(dbms),
        reports,
    })
}

/// A run of console `console` ended: the broad-refresh notice, then what waits for COMMIT.
pub fn run_ended(console: ConsoleId, cx: &mut App) {
    let Some(state) = Consoles::get(cx, console).cloned() else {
        return;
    };
    let Some(profile) = state.profile else {
        return;
    };
    let broad = all_mut(cx).and_then(|a| a.broad.remove(&console));
    if let Some((what, schema)) = broad
        && coord(profile, cx).is_some()
        && preference(cx) == RefreshAfterDdl::Always
        && crate::exec_ui::tx::user_tx(console, cx).is_none()
    {
        broad_notice(what, schema, cx);
    }
    update_pending(profile, Some(console), cx);
}

/// The toolbar (or a prompt) ended `console`'s transaction with `outcome`: the targets waiting
/// for its COMMIT are refreshed (or dropped on a rollback).
pub fn tx_ended(console: ConsoleId, outcome: TxOutcome, cx: &mut App) {
    let Some(state) = Consoles::get(cx, console).cloned() else {
        return;
    };
    let (Some(profile), Some(session)) = (state.profile, state.session) else {
        return;
    };
    let Some(c) = coord(profile, cx) else {
        return;
    };
    // Review m-2: through the inbox, after the run's own messages for the session, in order.
    c.inbox().push(RefreshMsg::TxEnded {
        session: session.id(),
        outcome,
    });
    update_pending(profile, None, cx);
}

/// Sessions that no console holds any more are closed for the coordinators: their open
/// transactions were rolled back, their waiting targets go.
fn session_closed(cx: &mut App) {
    let held: HashSet<SessionId> = Consoles::ids(cx)
        .into_iter()
        .filter_map(|c| Consoles::get(cx, c).and_then(|s| s.session.as_ref().map(|s| s.id())))
        .collect();
    let Some(all) = all_mut(cx) else {
        return;
    };
    let mut changed = Vec::new();
    for (profile, sessions) in all.sessions.iter_mut() {
        let gone: Vec<SessionId> = sessions.difference(&held).copied().collect();
        for s in gone {
            sessions.remove(&s);
            if let Some(c) = all.by_profile.get(profile) {
                c.coord.session_closed(s);
            }
            if let Some(p) = all.pending.get_mut(profile) {
                p.remove(&s);
            }
            changed.push(*profile);
        }
    }
    changed.dedup();
    for profile in changed {
        apply_stamps(profile, cx);
    }
}

fn observe(cx: &mut App) {
    let all = cx.default_global::<Coordinators>();
    if all.observing {
        return;
    }
    all.observing = true;
    cx.observe_global::<Consoles>(session_closed).detach();
}

/// Reads again, on the core runtime, what waits for each of `profile`'s sessions' COMMIT (the
/// coordinator's inbox is drained first, so the run that just ended is counted), then draws it.
/// `console`: the console whose run ended (its output says what waits).
fn update_pending(profile: ProfileId, console: Option<ConsoleId>, cx: &mut App) {
    let Some(rt) = runtime(cx) else {
        return;
    };
    let Some((c, handle)) = cx
        .try_global::<Coordinators>()
        .and_then(|a| a.by_profile.get(&profile))
        .map(|c| (c.coord.clone(), c.handle.clone()))
    else {
        return;
    };
    let sessions: Vec<SessionId> = cx
        .try_global::<Coordinators>()
        .and_then(|a| a.sessions.get(&profile))
        .map(|s| s.iter().copied().collect())
        .unwrap_or_default();
    let (tx, rx) = oneshot::channel();
    rt.spawn(async move {
        c.drain();
        let snap = handle.catalog().current();
        let out: HashMap<SessionId, Changes> = sessions
            .into_iter()
            .map(|s| (s, summarize(&c.pending(s), &snap)))
            .collect();
        let _ = tx.send(out);
    });
    cx.spawn(async move |cx| {
        let Ok(out) = rx.await else {
            return;
        };
        cx.update(|cx| {
            let session = console
                .and_then(|c| Consoles::get(cx, c))
                .and_then(|s| s.session.as_ref().map(|s| s.id()));
            let before = session.and_then(|s| {
                cx.try_global::<Coordinators>()
                    .and_then(|a| a.pending.get(&profile))
                    .and_then(|p| p.get(&s))
                    .cloned()
            });
            let now = session.and_then(|s| out.get(&s)).cloned();
            cx.default_global::<Coordinators>()
                .pending
                .insert(profile, out);
            apply_stamps(profile, cx);
            if let (Some(console), Some(now)) = (console, now)
                && !now.is_empty()
                && before.as_ref() != Some(&now)
            {
                log_pending(console, &now, cx);
            }
        });
    })
    .detach();
}

// ## fix round (ruling R-W5-10): changed objects are counted; a schema-wide change has no count.

/// What changes in one namespace (the Ask and the COMMIT stamp say the same).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NsChange {
    /// These many objects (created, dropped, altered or renamed).
    Objects(u32),
    /// The whole namespace (dynamic SQL, a widened plan, a database-wide change): no count.
    Whole,
}

/// What changes, per namespace.
pub type Changes = BTreeMap<NsKey, NsChange>;

/// Counts the objects `targets` change, per namespace: each object once; a namespace or a
/// database target makes its namespaces whole; the planner's name-list and scope targets are
/// bookkeeping and count nothing (ruling R-W5-10).
pub fn summarize(targets: &[Target], snap: &CatalogSnapshot) -> Changes {
    let mut objects: BTreeMap<NsKey, BTreeSet<ObjectKey>> = BTreeMap::new();
    let mut whole: BTreeSet<NsKey> = BTreeSet::new();
    for t in targets {
        match t {
            Target::Object(k) => {
                objects.entry(ns_of_key(k)).or_default().insert(k.clone());
            }
            Target::Objects { ns, keys } => {
                objects
                    .entry(ns.clone())
                    .or_default()
                    .extend(keys.iter().cloned());
            }
            Target::Namespace(ns) => {
                whole.insert(ns.clone());
            }
            Target::Database(_) => {
                for l in lower(std::slice::from_ref(t), snap) {
                    if let Target::Namespace(ns) = l {
                        whole.insert(ns);
                    }
                }
            }
            Target::NamespaceNames(_) | Target::ScopeRefresh => {}
        }
    }
    let mut out: Changes = whole.into_iter().map(|ns| (ns, NsChange::Whole)).collect();
    for (ns, keys) in objects {
        out.entry(ns).or_insert(NsChange::Objects(
            u32::try_from(keys.len()).unwrap_or(u32::MAX),
        ));
    }
    out
}

/// Adds `b` to `a` (a whole namespace stays whole).
fn merge_into(a: &mut Changes, b: &Changes) {
    for (ns, c) in b {
        let next = match (a.get(ns), c) {
            (Some(NsChange::Whole), _) | (_, NsChange::Whole) => NsChange::Whole,
            (Some(NsChange::Objects(x)), NsChange::Objects(y)) => {
                NsChange::Objects(x.saturating_add(*y))
            }
            (None, c) => *c,
        };
        a.insert(ns.clone(), next);
    }
}

/// The objects counted in `c`, unless a namespace changes whole (`None`).
fn object_count(c: &Changes) -> Option<u64> {
    c.values().try_fold(0u64, |n, ch| match ch {
        NsChange::Objects(k) => Some(n.saturating_add(u64::from(*k))),
        NsChange::Whole => None,
    })
}

/// The name a schema-wide change is told by: its namespace, or its database when several
/// namespaces change whole.
fn whole_name(c: &Changes) -> Option<String> {
    let mut whole = c
        .iter()
        .filter(|(_, ch)| **ch == NsChange::Whole)
        .map(|(ns, _)| ns);
    let first = whole.next()?;
    Some(if whole.next().is_some() {
        first
            .path
            .first()
            .map_or_else(|| first.root_name().to_string(), ToString::to_string)
    } else {
        first.root_name().to_string()
    })
}

/// Draws `profile`'s stamps: every session's waiting changes per namespace.
fn apply_stamps(profile: ProfileId, cx: &mut App) {
    let mut all: Changes = BTreeMap::new();
    if let Some(p) = cx
        .try_global::<Coordinators>()
        .and_then(|a| a.pending.get(&profile))
    {
        for m in p.values() {
            merge_into(&mut all, m);
        }
    }
    live::set_refresh_marks(profile, Some(all), None, cx);
}

/// "Explorer refresh waits for COMMIT ({count} objects)", or "(changes in {schema})", in
/// `console`'s output.
fn log_pending(console: ConsoleId, changes: &Changes, cx: &mut App) {
    let Some(state) = Consoles::get(cx, console) else {
        return;
    };
    let msg = match (object_count(changes), whole_name(changes)) {
        (Some(n), _) => hacivat_exec::LogMsg::RunUi {
            id: "hv-metadata-log-pending",
            args: vec![("count", hacivat_exec::LogArg::U64(n))],
            warn: false,
        },
        (None, schema) => hacivat_exec::LogMsg::RunUi {
            id: "hv-metadata-log-pending-schema",
            args: vec![(
                "schema",
                hacivat_exec::LogArg::Str(Arc::from(schema.unwrap_or_default())),
            )],
            warn: false,
        },
    };
    state.log.append(msg, None, None);
    if let Some(results) = state.results.clone() {
        results.update(cx, |r, cx| r.note_output(cx));
    }
}

fn text(id: &'static str, args: Vec<(&'static str, ArgValue)>) -> TextFn {
    Rc::new(move |cx: &App| live::text(cx, id, args.clone()))
}

/// "Refreshing all of {schema}": the script ran dynamic SQL (mock 9).
fn broad_notice(what: String, schema: String, cx: &mut App) {
    let mut toast = Toast::new(
        ToastKind::Info,
        text(
            "hv-metadata-broad-body",
            vec![("what", ArgValue::Text(what))],
        ),
    );
    toast.title = Some(text(
        "hv-metadata-broad",
        vec![("schema", ArgValue::Text(schema))],
    ));
    toast.icon = Some(IconName::RefreshCw);
    ToastQueue::push(cx, toast);
}

/// What the coordinator of `profile` announced.
fn on_event(profile: ProfileId, e: CoordEvent, cx: &mut App) {
    match e {
        CoordEvent::Ask { id, .. } => ask(profile, id, cx),
        CoordEvent::Stale { scope, .. } => {
            // Stale namespaces are published by the engine (their badge); a stale schema list
            // is badged from here on.
            if scope {
                live::set_refresh_marks(profile, None, Some(true), cx);
            }
        }
        CoordEvent::Dropped { total } => {
            tracing::debug!(
                total,
                "explorer: refresh messages dropped; refreshed widely"
            );
        }
        // ## m4-t33b-carries (the T27 carry, mock 9): one Output line per namespace.
        CoordEvent::Refreshed { objects } => refreshed(profile, &objects, cx),
    }
}

// ## m4-t33b-carries (the T27 carry, mock 9 "Refreshed")
/// "Explorer refreshed 2 objects in public", on the output of the profile's console that has
/// (or last had) the focus, else its newest one; no console of the profile: nothing.
fn refreshed(profile: ProfileId, objects: &[(NsKey, usize)], cx: &mut App) {
    let on_profile =
        |c: &ConsoleId| Consoles::get(cx, *c).is_some_and(|s| s.profile == Some(profile));
    let console = crate::exec_ui::cancel::focused_console(cx)
        .filter(on_profile)
        .or_else(|| Consoles::ids(cx).into_iter().filter(on_profile).max());
    let Some(console) = console else {
        return;
    };
    for (ns, count) in objects {
        crate::exec_ui::orchestrator::log_note(
            console,
            "hv-metadata-refreshed",
            vec![
                ("count", hacivat_exec::LogArg::U64(*count as u64)),
                (
                    "schema",
                    hacivat_exec::LogArg::Str(Arc::from(ns.root_name())),
                ),
            ],
            cx,
        );
    }
}

/// The Ask toast (mock 9): a newer question replaces an older one (the coordinator folded its
/// targets into the newer). It counts changed objects; a schema-wide change names the schema
/// (ruling R-W5-10). Escape or its ✕ answers Not now (NOTES §9; review m-1).
fn ask(profile: ProfileId, id: u64, cx: &mut App) {
    let Some((c, handle)) = cx
        .try_global::<Coordinators>()
        .and_then(|a| a.by_profile.get(&profile))
        .map(|c| (c.coord.clone(), c.handle.clone()))
    else {
        return;
    };
    let targets = c.asked(id);
    if targets.is_empty() {
        // Answered already, or folded into a newer question (whose event follows).
        return;
    }
    let changes = summarize(&targets, &handle.catalog().current());
    let console = cx
        .try_global::<Coordinators>()
        .and_then(|a| a.last_console.get(&profile).cloned())
        .unwrap_or_default();
    let title = match (object_count(&changes), whole_name(&changes)) {
        (Some(n), _) if n > 0 => text(
            "hv-metadata-refresh-ask",
            vec![("count", ArgValue::Count(n))],
        ),
        (_, schema) => text(
            "hv-metadata-refresh-ask-schema",
            vec![(
                "schema",
                ArgValue::Text(schema.unwrap_or_else(|| console.clone())),
            )],
        ),
    };
    let answer = move |refresh: bool, always: bool| -> hacivat_ui::actions::ActionFn {
        Rc::new(move |_, cx: &mut App| {
            if always {
                update_settings(cx, |s| {
                    s.metadata.refresh_after_ddl = RefreshAfterDdl::Always;
                });
                preference_changed(cx);
            }
            if let Some(c) = coord(profile, cx) {
                drop(c.answer(id, refresh));
            }
            if let Some(all) = all_mut(cx)
                && all.ask_toasts.get(&profile).is_some_and(|(q, _)| *q == id)
            {
                all.ask_toasts.remove(&profile);
            }
        })
    };
    let mut toast = Toast::new(
        ToastKind::Info,
        text(
            "hv-metadata-refresh-ask-body",
            vec![("console", ArgValue::Text(console))],
        ),
    );
    toast.title = Some(title);
    toast.on_dismiss = Some(answer(false, false));
    toast.sticky = true;
    toast.wide = true;
    toast.icon = Some(IconName::RefreshCw);
    toast.actions = vec![
        ToastAction {
            label: text("hv-metadata-refresh", Vec::new()),
            run: answer(true, false),
        },
        ToastAction {
            label: text("hv-metadata-refresh-always", Vec::new()),
            run: answer(true, true),
        },
        ToastAction {
            label: text("hv-metadata-refresh-not-now", Vec::new()),
            run: answer(false, false),
        },
    ];
    let shown = ToastQueue::push_id(cx, toast);
    let all = cx.default_global::<Coordinators>();
    all.asks += 1;
    if let Some((_, old)) = all.ask_toasts.insert(profile, (id, shown)) {
        ToastQueue::dismiss(cx, old);
    }
}
