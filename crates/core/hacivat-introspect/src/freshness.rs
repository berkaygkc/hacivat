// ## m4-t01c-core
//! Freshness (HV-INTRO-15; Task 22): `check_state` compares `Introspector::stamps` with the
//! snapshot's marks by equality; `check_namespace` validates a cached namespace cheaply (count,
//! highest mark, existence digest) on connect and periodically (R-M4-20).
//!
//! ## m4-t22-freshness (M4 Task 22; ADR 0008 `## m4-t22 — freshness`)
//!
//! - **State checks** ([`check_state`]). Each asked key is resolved in the snapshot to its
//!   **stamp owner**: its top-level ancestor (or itself), the unit a stamp describes and a
//!   re-read replaces (PostgreSQL's `StateOfRelation` covers a relation's columns, indexes,
//!   constraints, triggers and comments; MySQL answers a child key with its table's mark;
//!   SQLite's marks are per schema). The owners' stamps are read per kind, at most
//!   [`MAX_KEYS_PER_ROUND_TRIP`] keys per call (R-M4-3), and compared **by equality** with the
//!   owner's stored mark ([`compare`]; never an ordering, so a 32-bit PostgreSQL xmin that
//!   wrapped compares as well as any other). Outcomes: `Current`, `Changed`, `Dropped`
//!   (only where absence is a fact: PostgreSQL and SQLite; MySQL's `INFORMATION_SCHEMA` hides
//!   what the login lost the privilege for), `Unknown` (a permission error, no stamp, no
//!   server id where the stamp needs one, a namespace not loaded).
//! - **Marks recorded on re-read.** PostgreSQL passes store no per-object mark
//!   (`StateMark::None`), so a first check of a PostgreSQL object reads `Changed`; the engine
//!   re-reads it and stores the stamp read **before** the re-read as its mark
//!   ([`CheckResult::marks`]). A change between the stamp and the re-read moves the next
//!   stamp again: the order can only cost a second re-read, never hide a change.
//! - **By name** (the negative cache's probe, HV-MODEL-02 step 6): a key the loaded namespace
//!   does not hold compares as absent: still absent is `Current` (the `Missing` answer stands),
//!   present is `Changed` (the engine reads it).
//! - **Namespace validation** ([`check_namespace`]): the introspector's server-side
//!   [`NsCheck`] against its own cached-side one ([`Introspector::cached_check`]); equal is
//!   [`NsVerdict::Current`] (the engine clears the stale mark, nothing else is read), anything
//!   else [`NsVerdict::Changed`] (an incremental pass; a full one when the server side says it
//!   cannot vouch for an incremental read, `max_mark == StateMark::None`).
//! - **Periodic validation** ([`validate_interval`], R-M4-20) runs on a [`ValidationClock`]:
//!   tokio's clock in the product, a [`ManualClock`] in tests (no fixed sleeps, no wall clock).

#![allow(
    clippy::result_large_err,
    reason = "IntroError carries the server's message by value (MetaError::Server), as the seam does"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hacivat_meta::snapshot::{ContainerKey, LoadState};
use hacivat_meta::{
    CatalogSnapshot, Level, NamespaceSnapshot, NsKey, NsKind, ObjectKey, StateMark,
};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::meta::MetaConn;
use hacivat_settings::{MetadataSettings, ValidateInterval};
use tokio::sync::watch;

use crate::introspector::{IntroError, Introspector, Staleness};

/// The cheap validation result of one namespace.
///
/// ## m4-t22-freshness
/// A server-side check whose `max_mark` is `StateMark::None` (where the cached side has a
/// mark) asks for a **full** re-read: the introspector saw a change it cannot vouch an
/// incremental pass would catch (PostgreSQL: a full plan, or a renamed schema the namespace's
/// objects depend on, two hops away).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NsCheck {
    /// How many objects the namespace has.
    pub objects: u64,
    /// The highest state mark of the namespace.
    pub max_mark: StateMark,
    /// XXH3-64 of the sorted existence keys.
    pub existence_digest: u64,
}

// ## m4-t22-freshness

/// At most this many keys go to the server in one stamp read (R-M4-3's list-bind chunk).
pub const MAX_KEYS_PER_ROUND_TRIP: usize = 1_000;

/// What the snapshot holds for a stamp owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stored {
    /// The loaded namespace does not hold it (a name the negative cache answered `Missing`).
    Absent,
    /// It holds it, with this mark.
    Mark(StateMark),
}

/// Whether a missing stamp proves the object is gone on `dbms`: PostgreSQL's catalogs and
/// SQLite's schema show every object to every login; MySQL's `INFORMATION_SCHEMA` hides what
/// the login has no privilege on, so a missing stamp there may be a lost privilege (never
/// `Dropped`, HV-INTRO-11).
pub fn absence_is_fact(dbms: DbmsKind) -> bool {
    matches!(dbms, DbmsKind::Postgres | DbmsKind::Sqlite)
}

/// One comparison **by equality** (never an ordering: a PostgreSQL xmin wraps). A stamp of
/// `StateMark::None` carries no information (an object without an oid, a routine without a
/// `LAST_ALTERED`): `Unknown`. On MySQL and MariaDB an equal mark is `Current` only with the
/// stored arena's watermark at hand ([`compare_in`]); without it, `Changed` (fix round 4).
pub fn compare(dbms: DbmsKind, stored: Stored, stamp: Option<StateMark>) -> Staleness {
    compare_in(dbms, stored, stamp, None)
}

// ## m4-t22-state (fix round 4, RR-2)
/// [`compare`] with the stored arena's MySQL watermark time (`Watermark::MyTime.at_utc_micros`).
/// R-M4-39: MySQL markers have one-second granularity, so a second DDL in the same second as
/// the stored read leaves the same marker. On MySQL and MariaDB an equal mark is `Current` only
/// when the arena has a `MyTime` watermark and the stamp is outside the one-second window of
/// it; otherwise it is `Changed` (the incremental pass would re-read it too).
pub fn compare_in(
    dbms: DbmsKind,
    stored: Stored,
    stamp: Option<StateMark>,
    mytime: Option<i64>,
) -> Staleness {
    let st = compare_marks(dbms, stored, stamp);
    if st == Staleness::Current
        && matches!(dbms, DbmsKind::MySql | DbmsKind::MariaDb)
        && matches!(stored, Stored::Mark(_))
    {
        return match (mytime, stamp) {
            (Some(w), Some(s)) if !crate::mysql::pass::in_window(&s, w) => Staleness::Current,
            _ => Staleness::Changed,
        };
    }
    st
}

fn compare_marks(dbms: DbmsKind, stored: Stored, stamp: Option<StateMark>) -> Staleness {
    match (stored, stamp) {
        (_, Some(StateMark::None)) => Staleness::Unknown,
        (Stored::Absent, None) => Staleness::Current,
        (Stored::Absent, Some(_)) => Staleness::Changed,
        (Stored::Mark(_), None) if absence_is_fact(dbms) => Staleness::Dropped,
        (Stored::Mark(_), None) => Staleness::Unknown,
        (Stored::Mark(m), Some(s)) if m == s => Staleness::Current,
        (Stored::Mark(_), Some(_)) => Staleness::Changed,
    }
}

/// How one asked key is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Slot {
    /// Decided without the server.
    Decided(Staleness),
    /// By the stamp of probe `n`.
    Probe(usize),
}

/// What a state check reads: per asked key, the probe that answers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckPlan {
    dbms: DbmsKind,
    slots: Vec<Slot>,
    /// The distinct stamp owners to read, and what the snapshot holds for each.
    pub probes: Vec<(ObjectKey, Stored)>,
    /// Per probe, the `MyTime` watermark time of the arena holding it (R-M4-39's window).
    windows: Vec<Option<i64>>,
}

/// What a state check found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckResult {
    /// One per asked key, in order.
    pub staleness: Vec<Staleness>,
    /// The stamp owners that read `Changed`, with the stamp read (the engine stores it as the
    /// owner's mark when it re-reads the owner and the read brings no mark of its own).
    pub marks: Vec<(ObjectKey, StateMark)>,
}

/// The namespace arena that holds `key` (the longest namespace prefix of its path).
pub fn arena_of<'s>(
    snapshot: &'s CatalogSnapshot,
    key: &ObjectKey,
) -> Option<&'s Arc<NamespaceSnapshot>> {
    for k in (0..key.path.len()).rev() {
        for kind in [NsKind::Schema, NsKind::Database, NsKind::Server] {
            let ns = NsKey {
                kind,
                path: key.path[..k].iter().cloned().collect(),
            };
            if let Some(a) = snapshot.namespaces.get(&ns) {
                return Some(a);
            }
        }
    }
    None
}

/// Whether `ns` is loaded completely enough that an object it does not hold is absent: loaded
/// (stale or not) at `Structure` or above. A `Partial` namespace (a kind the login could not
/// read) or one loaded at names only vouches for nothing missing.
fn absence_known(snapshot: &CatalogSnapshot, ns: &NsKey) -> bool {
    matches!(
        snapshot.state(&ContainerKey::Namespace(ns.clone())),
        LoadState::Loaded { level, .. } if *level >= Level::Structure
    )
}

/// SQLite's `temp` schema is per connection and always re-read whole (T18 I-1, I-2): its keys
/// are never decided by a stamp (no connection but the user's can see them), they read
/// `Changed`, so the engine re-reads them on the user's connection.
fn is_sqlite_temp(dbms: DbmsKind, key: &ObjectKey) -> bool {
    dbms == DbmsKind::Sqlite
        && key
            .path
            .first()
            .is_some_and(|s| s.eq_ignore_ascii_case("temp"))
}

/// Resolves `keys` against `snapshot` (module docs, "State checks"). Pure: no I/O.
pub fn plan(dbms: DbmsKind, snapshot: &CatalogSnapshot, keys: &[ObjectKey]) -> CheckPlan {
    let mut probes: Vec<(ObjectKey, Stored)> = Vec::new();
    let mut windows: Vec<Option<i64>> = Vec::new();
    let mut index: BTreeMap<ObjectKey, usize> = BTreeMap::new();
    let mut slots = Vec::with_capacity(keys.len());
    for key in keys {
        let slot = 'slot: {
            if is_sqlite_temp(dbms, key) {
                break 'slot Slot::Decided(Staleness::Changed);
            }
            let Some(arena) = arena_of(snapshot, key) else {
                break 'slot Slot::Decided(Staleness::Unknown);
            };
            let (owner, stored) = match arena.find(key) {
                Some(mut idx) => {
                    // The top-level ancestor (or the key itself): the stamp's unit.
                    while let Some(p) = arena.obj(idx).and_then(|o| o.parent()) {
                        if p == 0 {
                            break;
                        }
                        idx = p;
                    }
                    if idx == 0 {
                        // The namespace itself: validated by `check_namespace`, not stamped.
                        break 'slot Slot::Decided(Staleness::Unknown);
                    }
                    let (Some(owner), Some(obj)) = (arena.object_key(idx), arena.obj(idx)) else {
                        break 'slot Slot::Decided(Staleness::Unknown);
                    };
                    (owner, Stored::Mark(obj.state()))
                }
                // ## m4-t22-state (fix round 3, CI 44667350956): a key that names an object by
                // its server id is an object that existed, whatever the cache holds now (a
                // re-read may already hold a recreated object of that name under a new oid):
                // it compares as "was there, mark unknown", so a missing stamp is `Dropped`
                // and a present one `Changed`, never `Current`.
                None if key.path.len() == arena.key().path.len() + 1 && key.server_id.is_some() => {
                    (key.clone(), Stored::Mark(StateMark::None))
                }
                None => {
                    // A name the loaded namespace does not hold: only a top-level one can be
                    // stamped by itself, and only where the namespace vouches for absence.
                    if key.path.len() != arena.key().path.len() + 1
                        || !absence_known(snapshot, arena.key())
                    {
                        break 'slot Slot::Decided(Staleness::Unknown);
                    }
                    (key.clone(), Stored::Absent)
                }
            };
            // PostgreSQL stamps by oid: an owner without one has no stamp.
            if dbms == DbmsKind::Postgres && owner.server_id.is_none() {
                break 'slot Slot::Decided(Staleness::Unknown);
            }
            let n = *index.entry(owner.clone()).or_insert_with(|| {
                probes.push((owner, stored));
                windows.push(match &arena.meta().watermark {
                    hacivat_meta::Watermark::MyTime { at_utc_micros, .. } => Some(*at_utc_micros),
                    _ => None,
                });
                probes.len() - 1
            });
            Slot::Probe(n)
        };
        slots.push(slot);
    }
    CheckPlan {
        dbms,
        slots,
        probes,
        windows,
    }
}

/// Reads the stamps `plan` needs on `c` and compares them (module docs). Probes are grouped
/// by kind, at most [`MAX_KEYS_PER_ROUND_TRIP`] per call; a permission error leaves its chunk
/// `Unknown` (permission loss is never a drop); any other error fails the check.
pub async fn run(
    intro: &dyn Introspector,
    c: &mut MetaConn,
    plan: &CheckPlan,
) -> Result<CheckResult, IntroError> {
    let mut by_kind: BTreeMap<(u16, Option<hacivat_meta::Name>), Vec<usize>> = BTreeMap::new();
    for (i, (k, _)) in plan.probes.iter().enumerate() {
        by_kind
            .entry((k.kind.code(), k.raw_kind.clone()))
            .or_default()
            .push(i);
    }
    let mut answers: Vec<Staleness> = vec![Staleness::Unknown; plan.probes.len()];
    let mut marks = Vec::new();
    for probes in by_kind.values() {
        for chunk in probes.chunks(MAX_KEYS_PER_ROUND_TRIP) {
            let keys: Vec<ObjectKey> = chunk.iter().map(|&i| plan.probes[i].0.clone()).collect();
            let stamps = match intro.stamps(c, &keys).await {
                Ok(s) if s.len() == keys.len() => s,
                Ok(_) => return Err(IntroError::Protocol(hacivat_meta::Name::from("stamps"))),
                Err(IntroError::Permission { .. }) => continue,
                Err(e) => return Err(e),
            };
            for (&i, stamp) in chunk.iter().zip(stamps) {
                let (owner, stored) = &plan.probes[i];
                let st = compare_in(plan.dbms, *stored, stamp, plan.windows[i]);
                if st == Staleness::Changed
                    && let Some(m) = stamp
                {
                    marks.push((owner.clone(), m));
                }
                answers[i] = st;
            }
        }
    }
    let staleness = plan
        .slots
        .iter()
        .map(|s| match s {
            Slot::Decided(st) => *st,
            Slot::Probe(n) => answers[*n],
        })
        .collect();
    Ok(CheckResult { staleness, marks })
}

/// The freshness of `keys` against the server behind `c`, as `snapshot` holds them: [`plan`]
/// then [`run`].
pub async fn check_state(
    intro: &dyn Introspector,
    c: &mut MetaConn,
    snapshot: &CatalogSnapshot,
    keys: &[ObjectKey],
) -> Result<CheckResult, IntroError> {
    let plan = plan(intro.capabilities().dbms, snapshot, keys);
    run(intro, c, &plan).await
}

/// What the validation of one cached namespace decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NsVerdict {
    /// The server matches the cache: clear the stale mark, read nothing.
    Current,
    /// Read again: incrementally, or in full when `full`.
    Changed { full: bool },
}

/// The verdict of a cached-side and a server-side check (module docs).
pub fn verdict(cached: &NsCheck, server: &NsCheck) -> NsVerdict {
    if cached == server {
        NsVerdict::Current
    } else {
        NsVerdict::Changed {
            full: server.max_mark == StateMark::None && cached.max_mark != StateMark::None,
        }
    }
}

/// Validates the cached namespace `snap` of `ns` on `c` (module docs). An introspector with no
/// cached side reads nothing and answers `Changed` (the pass decides); a permission error is
/// `Changed` too (the pass reports what the login lost).
pub async fn check_namespace(
    intro: &dyn Introspector,
    c: &mut MetaConn,
    ns: &NsKey,
    snap: &NamespaceSnapshot,
) -> Result<NsVerdict, IntroError> {
    let Some(cached) = intro.cached_check(snap) else {
        return Ok(NsVerdict::Changed { full: false });
    };
    match intro.check_namespace(c, ns, snap).await {
        Ok(server) => Ok(verdict(&cached, &server)),
        Err(IntroError::Permission { .. }) => Ok(NsVerdict::Changed { full: false }),
        Err(e) => Err(e),
    }
}

/// How often loaded namespaces are validated (R-M4-20, PO-Q6): `auto` is every 10 minutes for
/// profiles tagged `local` or `dev`, off for every other tag and for untagged profiles.
pub fn validate_interval(m: &MetadataSettings, environment: Option<EnvTag>) -> Option<Duration> {
    match m.validate_interval_min {
        ValidateInterval::Off => None,
        ValidateInterval::Minutes(n) => Some(Duration::from_secs(u64::from(n.get()) * 60)),
        ValidateInterval::Auto => matches!(environment, Some(EnvTag::Local | EnvTag::Dev))
            .then(|| Duration::from_secs(10 * 60)),
    }
}

/// The clock periodic validation runs on: tokio's (the product) or a [`ManualClock`] (tests).
/// Times are durations since the clock's start.
#[derive(Clone, Debug)]
pub struct ValidationClock(Clock);

#[derive(Clone, Debug)]
enum Clock {
    Tokio(tokio::time::Instant),
    Manual(Arc<watch::Sender<Duration>>),
}

impl Default for ValidationClock {
    fn default() -> Self {
        ValidationClock::tokio()
    }
}

impl ValidationClock {
    /// Tokio's clock, from now.
    pub fn tokio() -> Self {
        ValidationClock(Clock::Tokio(tokio::time::Instant::now()))
    }

    /// A clock that moves only when its [`ManualClock`] advances it.
    pub fn manual() -> (Self, ManualClock) {
        let tx = Arc::new(watch::Sender::new(Duration::ZERO));
        (ValidationClock(Clock::Manual(tx.clone())), ManualClock(tx))
    }

    /// The time since the clock's start.
    pub fn now(&self) -> Duration {
        match &self.0 {
            Clock::Tokio(base) => base.elapsed(),
            Clock::Manual(tx) => *tx.borrow(),
        }
    }

    /// Resolves once the clock reads `at` or later.
    pub async fn sleep_until(&self, at: Duration) {
        match &self.0 {
            Clock::Tokio(base) => tokio::time::sleep_until(*base + at).await,
            Clock::Manual(tx) => {
                let mut rx = tx.subscribe();
                if rx.wait_for(|now| *now >= at).await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        }
    }
}

/// The hand of a manual [`ValidationClock`].
#[derive(Clone, Debug)]
pub struct ManualClock(Arc<watch::Sender<Duration>>);

impl ManualClock {
    /// Moves the clock forward by `by`.
    pub fn advance(&self, by: Duration) {
        self.0.send_modify(|now| *now += by);
    }

    /// The clock's time.
    pub fn now(&self) -> Duration {
        *self.0.borrow()
    }
}
// ## m4-t22-freshness

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equality_never_ordering() {
        let pg = DbmsKind::Postgres;
        let m = |x| Stored::Mark(StateMark::Xmin(x));
        // A wrapped xmin is just another value: equal is current, different is changed,
        // whichever is "greater".
        assert_eq!(
            compare(pg, m(5), Some(StateMark::Xmin(5))),
            Staleness::Current
        );
        assert_eq!(
            compare(pg, m(u32::MAX), Some(StateMark::Xmin(3))),
            Staleness::Changed
        );
        assert_eq!(
            compare(pg, m(3), Some(StateMark::Xmin(u32::MAX))),
            Staleness::Changed
        );
    }

    #[test]
    fn absence_is_dropped_only_where_it_is_a_fact() {
        let stored = Stored::Mark(StateMark::Hash(1));
        assert_eq!(
            compare(DbmsKind::Postgres, stored, None),
            Staleness::Dropped
        );
        assert_eq!(compare(DbmsKind::Sqlite, stored, None), Staleness::Dropped);
        assert_eq!(compare(DbmsKind::MySql, stored, None), Staleness::Unknown);
        assert_eq!(compare(DbmsKind::MariaDb, stored, None), Staleness::Unknown);
        // No mark is no information, whatever is stored.
        assert_eq!(
            compare(DbmsKind::Postgres, stored, Some(StateMark::None)),
            Staleness::Unknown
        );
        assert_eq!(
            compare(DbmsKind::MySql, Stored::Absent, Some(StateMark::None)),
            Staleness::Unknown
        );
        // By name: still absent is current, present is changed.
        assert_eq!(
            compare(DbmsKind::MySql, Stored::Absent, None),
            Staleness::Current
        );
        assert_eq!(
            compare(DbmsKind::MySql, Stored::Absent, Some(StateMark::Created(1))),
            Staleness::Changed
        );
    }

    #[test]
    fn a_full_read_is_asked_only_by_a_markless_server_side() {
        let c = |m| NsCheck {
            objects: 3,
            max_mark: m,
            existence_digest: 7,
        };
        assert_eq!(
            verdict(&c(StateMark::Xmin(9)), &c(StateMark::Xmin(9))),
            NsVerdict::Current
        );
        assert_eq!(
            verdict(&c(StateMark::Xmin(9)), &c(StateMark::Xmin(10))),
            NsVerdict::Changed { full: false }
        );
        assert_eq!(
            verdict(&c(StateMark::Xmin(9)), &c(StateMark::None)),
            NsVerdict::Changed { full: true }
        );
        assert_eq!(
            verdict(&c(StateMark::None), &c(StateMark::None)),
            NsVerdict::Current
        );
    }

    #[tokio::test]
    async fn the_manual_clock_moves_only_when_advanced() {
        let (clock, hand) = ValidationClock::manual();
        assert_eq!(clock.now(), Duration::ZERO);
        let c2 = clock.clone();
        let t = tokio::spawn(async move { c2.sleep_until(Duration::from_secs(600)).await });
        hand.advance(Duration::from_secs(599));
        tokio::task::yield_now().await;
        assert!(!t.is_finished());
        hand.advance(Duration::from_secs(1));
        t.await.unwrap();
        assert_eq!(hand.now(), Duration::from_secs(600));
    }
}
