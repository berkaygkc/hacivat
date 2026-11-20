// ## m3b-t01c-core-seams
//! The auditor chain (R-M3-63; Task 16): around M3a's observer, the synchronous O(1) auditors
//! (timing, the transaction marker, cancel progress, gutter state) see every `RunEvent` in
//! order on the actor's task; the asynchronous sinks (the output log, the history) get
//! `try_send` with room for [`SINK_CAPACITY`] events, and what does not fit is counted
//! (`LogMsg::Dropped`), never waited for.
//!
//! ## m3b-t16-run-service (ADR 0006 `m3b-t16`)
//! - **Order.** [`AuditorChain::on_event`] hands an event to every synchronous auditor in the
//!   order they were added, then offers it to every sink. A sink maps the event to its own
//!   item (or nothing) and `try_send`s it: a full channel counts one drop
//!   ([`AuditorChain::dropped`]), a closed one is ignored. Nothing in the chain awaits, so a
//!   slow consumer never slows the run.
//! - **Coalescing.** `RowsFetched` reaches the sinks at most once per 50 ms per result (the
//!   session already sends it at most that often; the chain does not rely on it).
//! - **The output log** is the console's `SessionLog` ring: a bounded, non-blocking sink of
//!   its own (an append is O(1) under a short lock, and a full ring drops its oldest line and
//!   counts it). The run service's observer writes it directly, so the log keeps M3a's order
//!   and timing; the chain's drops are added to it as one `Dropped { n }` line when the run
//!   ends.
//! - **The history** sink ([`HistorySink`]) turns each `UnitEnded` into one `HistoryRecord`:
//!   the masked statement only (never the text sent, never a bind value), its error message
//!   with every bind value scrubbed out, and no `text_hash` or `fingerprint` (the store
//!   computes both from its own redacted text, Task 17). A record is never formatted with
//!   `{:?}` anywhere (it may hold a statement the store has yet to redact).
//! - **Not an audit trail.** The chain is an event pipeline: what a sink drops is gone. The
//!   history may lose records (full, busy, broken) and is never evidence of what ran
//!   (R-M3-62; Karagöz's audit log is M8).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hacivat_results::ResultId;
use hacivat_session::session::{
    ExecError, ExecErrorKind, RowsOutcome, RunEvent, RunId, SessionHandle, UnitEnd, UnitId,
};
use hacivat_values::BindValue;
// ## m4-t21-refresh
use hacivat_introspect::refresh_coord::{RefreshInbox, RunTracker, TrackedUnit, TxPhase};
use tokio::sync::mpsc;

use crate::history::{HistoryMeta, HistoryRecord, Outcome};

/// The room of each asynchronous sink, in events.
pub const SINK_CAPACITY: usize = 10_000;

/// A synchronous auditor: O(1) per event, never blocks (it runs on the session's actor task).
pub trait SyncAuditor: Send {
    fn on_event(&mut self, ev: &RunEvent);
}

// ## m3b-t16-run-service
/// How often `RowsFetched` reaches the sinks per result.
const ROWS_EVERY: Duration = Duration::from_millis(50);

/// What a sink did with an event.
enum Offered {
    Sent,
    /// The sink takes nothing of this event.
    Skipped,
    /// The channel was full: one drop.
    Full,
}

/// An asynchronous sink: a channel and how an event becomes its item.
trait Sink: Send {
    fn offer(&mut self, ev: &RunEvent) -> Offered;
}

struct ChannelSink<T, F> {
    tx: mpsc::Sender<T>,
    map: F,
}

impl<T: Send, F: FnMut(&RunEvent) -> Option<T> + Send> Sink for ChannelSink<T, F> {
    fn offer(&mut self, ev: &RunEvent) -> Offered {
        let Some(item) = (self.map)(ev) else {
            return Offered::Skipped;
        };
        match self.tx.try_send(item) {
            Ok(()) => Offered::Sent,
            Err(mpsc::error::TrySendError::Full(_)) => Offered::Full,
            // Nobody reads it any more (the store shut down): nothing to count.
            Err(mpsc::error::TrySendError::Closed(_)) => Offered::Skipped,
        }
    }
}

/// The chain of a run (the module docs).
#[derive(Default)]
pub struct AuditorChain {
    sync: Vec<Box<dyn SyncAuditor>>,
    dropped: AtomicU64,
    // ## m3b-t16-run-service
    sinks: Vec<Box<dyn Sink>>,
    /// When each result's `RowsFetched` last reached the sinks.
    rows_sent: HashMap<ResultId, tokio::time::Instant>,
}

impl std::fmt::Debug for AuditorChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditorChain")
            .field("sync", &self.sync.len())
            .field("sinks", &self.sinks.len())
            .field("dropped", &self.dropped())
            .finish()
    }
}

impl AuditorChain {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a synchronous auditor; they see each event in the order they were added.
    pub fn push_sync(&mut self, auditor: Box<dyn SyncAuditor>) {
        self.sync.push(auditor);
    }

    // ## m3b-t16-run-service
    /// Adds an asynchronous sink: `map` turns an event into the channel's item (or `None`),
    /// which is `try_send`; a full channel counts one drop.
    pub fn push_sink<T, F>(&mut self, tx: mpsc::Sender<T>, map: F)
    where
        T: Send + 'static,
        F: FnMut(&RunEvent) -> Option<T> + Send + 'static,
    {
        self.sinks.push(Box::new(ChannelSink { tx, map }));
    }

    /// Hands `ev` to every auditor, then offers it to every sink (the module docs).
    pub fn on_event(&mut self, ev: &RunEvent) {
        for auditor in &mut self.sync {
            auditor.on_event(ev);
        }
        if self.sinks.is_empty() {
            return;
        }
        if let RunEvent::RowsFetched { result, .. } = ev {
            let now = tokio::time::Instant::now();
            match self.rows_sent.get(result) {
                Some(at) if now.saturating_duration_since(*at) < ROWS_EVERY => return,
                _ => {
                    self.rows_sent.insert(*result, now);
                }
            }
        }
        if let RunEvent::ResultEnded { result, .. } = ev {
            self.rows_sent.remove(result);
        }
        let mut full = 0u64;
        for sink in &mut self.sinks {
            if let Offered::Full = sink.offer(ev) {
                full += 1;
            }
        }
        if full > 0 {
            self.dropped.fetch_add(full, Ordering::Relaxed);
        }
    }

    /// Events the asynchronous sinks had no room for.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

// ## m3b-t16-run-service

/// What the history sink knows of one unit of a run: never the text sent, only its masked
/// form, and the bind values only to scrub them out of the server's error message.
#[derive(Clone)]
pub(crate) struct HistoryUnit {
    pub id: UnitId,
    pub masked: Arc<str>,
    pub binds: Arc<[BindValue]>,
    // ## m3b-t27-history (named exception, T16 carry): the masked text differs from the text
    // sent (`mask_secrets` masked a literal or comment), so the history shows its marker.
    pub redacted: bool,
}

/// The history's sink of one run: one [`HistoryRecord`] per `UnitEnded` of a unit that ran
/// (the module docs). Skipped units (a stop, the user's Skip) record nothing.
pub(crate) struct HistorySink {
    pub meta: HistoryMeta,
    pub run: RunId,
    pub units: Arc<[HistoryUnit]>,
    pub session: SessionHandle,
    /// When each unit's last attempt started.
    pub started: HashMap<u32, SystemTime>,
    // Fix round 1 (review m5)
    /// When the run was sent: a unit that ends without starting (a queued run cancelled, a
    /// full queue) is recorded at it.
    pub run_started: SystemTime,
}

/// The most bytes of an error message a record keeps.
const ERROR_MESSAGE_CAP: usize = 4 << 10;

impl HistorySink {
    /// The record of `ev`, if it is one.
    pub(crate) fn map(&mut self, ev: &RunEvent) -> Option<Arc<HistoryRecord>> {
        match ev {
            RunEvent::UnitStarted { unit, .. } => {
                self.started.insert(unit.ordinal, SystemTime::now());
                None
            }
            RunEvent::UnitEnded {
                unit,
                outcome,
                exec_ms,
                fetch_ms,
            } => self.record(*unit, outcome, exec_ms.saturating_add(*fetch_ms)),
            _ => None,
        }
    }

    /// The record of `unit`'s end. A unit that ended without starting (review m5) is recorded
    /// as not run: at the run's start, with no duration and no rows (`HistoryRecord` has no
    /// flag of its own for it), its masked text as always.
    fn record(&mut self, unit: UnitId, end: &UnitEnd, ms: u64) -> Option<Arc<HistoryRecord>> {
        let (started, ms) = match self.started.remove(&unit.ordinal) {
            Some(at) => (at, ms),
            None => (self.run_started, 0),
        };
        let u = self.units.iter().find(|u| u.id == unit)?;
        let (outcome, rows_affected, rows_fetched, error) = match end {
            UnitEnd::Succeeded { rows } => match rows {
                Some(RowsOutcome::Fetched { rows, .. }) => (Outcome::Ok, None, Some(*rows), None),
                Some(RowsOutcome::Affected(n)) => (Outcome::Ok, Some(*n), None, None),
                None => (Outcome::Ok, None, None, None),
            },
            UnitEnd::Failed(e) => {
                let outcome = match e.kind {
                    ExecErrorKind::ReadOnlyDenied(_) | ExecErrorKind::SafetyDenied(_) => {
                        Outcome::Denied
                    }
                    ExecErrorKind::Cancelled => Outcome::Cancelled,
                    _ => Outcome::Failed,
                };
                (outcome, None, None, Some(e))
            }
            UnitEnd::Cancelled => (Outcome::Cancelled, None, None, None),
            UnitEnd::Skipped => return None,
        };
        let (error_code, error_message) = match error {
            Some(e) => (code_of(e), message_of(e, &u.binds)),
            None => (None, None),
        };
        let snapshot = self.session.state().borrow().clone();
        let (database, schema) = match &snapshot.schema {
            Some(ctx) => (ctx.database.clone(), ctx.schema.clone()),
            None => (None, None),
        };
        Some(Arc::new(HistoryRecord {
            ts_start_ms: started
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| i64::try_from(d.as_millis()).ok())
                .unwrap_or(0),
            duration_ms: u32::try_from(ms).unwrap_or(u32::MAX),
            profile: self.meta.profile,
            database,
            schema,
            console: self.meta.console,
            origin: self.meta.origin,
            statement_masked: u.masked.clone(),
            // The store computes both from its own redacted text (Task 17); never from the
            // statement here, which it has yet to redact.
            text_hash: [0; 32],
            fingerprint: 0,
            params_masked: None,
            outcome,
            rows_affected,
            rows_fetched,
            error_code,
            error_message,
            tx_mode: snapshot.tx.mode,
            run: Some(self.run),
            // ## m3b-t27-history (named exception, T16 carry)
            redacted: u.redacted,
        }))
    }
}

/// The server's code of `e`: its SQLSTATE, else its vendor code.
fn code_of(e: &ExecError) -> Option<String> {
    let m = e.server.as_ref()?;
    m.sqlstate
        .clone()
        .or_else(|| m.vendor_code.map(|c| c.to_string()))
}

/// The server's message of `e`, with every bind value of the unit scrubbed out, at most
/// [`ERROR_MESSAGE_CAP`] bytes.
fn message_of(e: &ExecError, binds: &[BindValue]) -> Option<String> {
    let text = e.server.as_ref().map(|m| m.text.as_str())?;
    let mut out = scrub_binds(text, binds);
    if out.len() > ERROR_MESSAGE_CAP {
        let mut cut = ERROR_MESSAGE_CAP;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    Some(out)
}

/// What a bind value is replaced with in a server message.
const BIND_MASK: &str = "?";

/// `text` with every bind value's text replaced by [`BIND_MASK`]: longest values first (one
/// value never leaves part of a longer one behind); a value of one or two bytes only where it
/// stands alone (not inside a word or a number), so `1` does not eat `line 12`.
pub(crate) fn scrub_binds(text: &str, binds: &[BindValue]) -> String {
    let mut values: Vec<&str> = binds
        .iter()
        .filter_map(|b| match b {
            BindValue::Typed { text, .. } if !text.is_empty() => Some(&**text),
            _ => None,
        })
        .collect();
    values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    values.dedup();
    let mut out = text.to_string();
    for v in values {
        out = if v.len() > 2 {
            out.replace(v, BIND_MASK)
        } else {
            replace_standalone(&out, v)
        };
    }
    out
}

/// `text` with `v` replaced where no letter, digit or `_` touches it.
fn replace_standalone(text: &str, v: &str) -> String {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut before: Option<char> = None;
    while let Some(at) = rest.find(v) {
        let after = rest[at + v.len()..].chars().next();
        let prev = rest[..at].chars().next_back().or(before);
        out.push_str(&rest[..at]);
        if word(prev) || word(after) {
            out.push_str(v);
        } else {
            out.push_str(BIND_MASK);
        }
        before = v.chars().next_back();
        rest = &rest[at + v.len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_values::LogicalType;

    fn bind(s: &str) -> BindValue {
        BindValue::Typed {
            hint: LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            text: s.into(),
        }
    }

    #[test]
    fn bind_values_are_scrubbed_from_server_text() {
        let binds = [bind("s3cret"), bind("7"), bind("hv_test_pw")];
        let got = scrub_binds(
            "invalid input syntax for type integer: \"s3cret\" at line 17, key (7)=(hv_test_pw)",
            &binds,
        );
        assert!(!got.contains("s3cret"), "{got}");
        assert!(!got.contains("hv_test_pw"), "{got}");
        assert!(
            got.contains("line 17"),
            "a short value stays inside a number: {got}"
        );
        assert!(got.contains("key (?)"), "{got}");
    }

    /// Fix round 1 (review m5): a unit that ends without starting (a queued run cancelled) is
    /// recorded as not run, with its masked text.
    #[tokio::test]
    async fn a_unit_that_never_started_is_recorded_as_not_run() {
        use hacivat_driver::fake::FakeDriver;
        use hacivat_proto::{DbmsKind, ProfileId};
        use hacivat_secrets::{AuthProviderId, Retention};
        use hacivat_session::profile::{Profile, ProfileEndpoint};
        use hacivat_session::testing::fake_session;

        use crate::history::HistoryOrigin;

        let profile = Profile {
            name: "orders".into(),
            user: Some("deploy".into()),
            engine: DbmsKind::Postgres,
            endpoint: ProfileEndpoint::Server {
                host: "127.0.0.1".into(),
                port: None,
            },
            auth: AuthProviderId::None,
            retention: Retention::Never,
            ..Profile::default()
        };
        let (session, _f) = fake_session(
            tokio::runtime::Handle::current(),
            FakeDriver::new(DbmsKind::Postgres),
            profile,
        )
        .await;
        let unit = UnitId {
            run: RunId(3),
            ordinal: 0,
        };
        let run_started = UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        let mut sink = HistorySink {
            meta: HistoryMeta {
                profile: ProfileId::new(),
                console: 1,
                private: false,
                origin: HistoryOrigin::Console,
            },
            run: RunId(3),
            units: Arc::from(vec![HistoryUnit {
                id: unit,
                masked: "UPDATE t SET pw = ?".into(),
                binds: Arc::from(vec![bind("s3cret")]),
                // ## m3b-t27-history (named exception)
                redacted: false,
            }]),
            session,
            started: HashMap::new(),
            run_started,
        };
        let record = sink
            .map(&RunEvent::UnitEnded {
                unit,
                outcome: UnitEnd::Cancelled,
                exec_ms: 0,
                fetch_ms: 0,
            })
            .expect("a record");
        assert_eq!(record.ts_start_ms, 1_000_000);
        assert_eq!(record.duration_ms, 0);
        assert_eq!(record.outcome, Outcome::Cancelled);
        assert_eq!(&*record.statement_masked, "UPDATE t SET pw = ?");
        assert_eq!((record.rows_affected, record.rows_fetched), (None, None));
        // ## m3b-t27-history (named exception, T16 carry): the unit's mask flag reaches the
        // record (the history dialog's marker and its Insert and run guard).
        assert!(!record.redacted);
        sink.units = Arc::from(vec![HistoryUnit {
            id: unit,
            masked: "ALTER ROLE app PASSWORD '***'".into(),
            binds: Arc::from(Vec::new()),
            redacted: true,
        }]);
        let masked = sink
            .map(&RunEvent::UnitEnded {
                unit,
                outcome: UnitEnd::Cancelled,
                exec_ms: 0,
                fetch_ms: 0,
            })
            .expect("a record");
        assert!(masked.redacted);
        // A skipped unit records nothing.
        assert!(
            sink.map(&RunEvent::UnitEnded {
                unit,
                outcome: UnitEnd::Skipped,
                exec_ms: 0,
                fetch_ms: 0,
            })
            .is_none()
        );
    }
}

// ## m4-t01c-core (seam) / ## m4-t21-refresh (body; ADR 0006 `## m4-t21 — catalog refresh sink`)
/// The refresh-after-DDL sink (HV-INTRO-12; body Task 21, installed per run by Task 27): a
/// synchronous auditor that hands each finished unit's catalog effects (`StmtReport.ddl`) and
/// the session's transaction outcome to the profile's refresh coordinator, without waiting.
///
/// It feeds the run's [`RunTracker`] (`hacivat_introspect::refresh_coord`): every
/// `UnitStarted`, `TxChanged`, `UpdateCount` and `UnitEnded`, in order, on the session's actor
/// task. The tracker reads the session's state when a unit ends (execution time) and pushes
/// what the unit meant into the profile's `RefreshInbox`: a short lock, never an await, and a
/// full inbox drops its oldest message and counts it (the HV-EXEC-06 pattern of
/// [`AuditorChain`]'s sinks), so a slow coordinator never slows the run. Planning and the
/// catalog reads happen on the coordinator's own task.
#[derive(Debug)]
pub struct CatalogRefreshSink {
    tracker: RunTracker,
}

// ## m4-t21-refresh
impl CatalogRefreshSink {
    /// The sink of a run that `tracker` follows.
    pub fn new(tracker: RunTracker) -> Self {
        CatalogRefreshSink { tracker }
    }

    /// The sink of a run on `session` (of engine `dbms`) whose units are `units`, feeding
    /// `inbox`: the session's transaction status now is the run's starting point.
    pub fn for_run(
        session: &SessionHandle,
        dbms: hacivat_proto::DbmsKind,
        units: Vec<TrackedUnit>,
        inbox: RefreshInbox,
    ) -> Self {
        let state = session.state();
        let status = phase(state.borrow().tx.status);
        CatalogRefreshSink::new(RunTracker::new(
            session.id(),
            dbms,
            units,
            state,
            status,
            inbox,
        ))
    }
}

// ## m4-t21-refresh
/// The tracker's view of a driver transaction status.
fn phase(s: hacivat_driver::TxStatus) -> TxPhase {
    match s {
        hacivat_driver::TxStatus::Idle => TxPhase::Idle,
        hacivat_driver::TxStatus::InTx => TxPhase::InTx,
        hacivat_driver::TxStatus::Failed => TxPhase::Failed,
    }
}

impl SyncAuditor for CatalogRefreshSink {
    // ## m4-t21-refresh
    fn on_event(&mut self, ev: &RunEvent) {
        match ev {
            RunEvent::UnitStarted { unit, .. } => self.tracker.unit_started(*unit),
            RunEvent::TxChanged { from, to } => self.tracker.tx_changed(phase(*from), phase(*to)),
            RunEvent::UpdateCount {
                unit, command_tag, ..
            } => self.tracker.update_count(*unit, command_tag.as_deref()),
            RunEvent::UnitEnded { unit, outcome, .. } => self.tracker.unit_ended(*unit, outcome),
            _ => {}
        }
    }
}

// ## m4-t21-refresh
#[cfg(test)]
mod refresh_sink_tests {
    use hacivat_driver::TxStatus;
    use hacivat_driver::fake::FakeDriver;
    use hacivat_introspect::ddl_effects::TxnState;
    use hacivat_introspect::refresh_coord::RefreshMsg;
    use hacivat_proto::DbmsKind;
    use hacivat_secrets::{AuthProviderId, Retention};
    use hacivat_session::profile::{Profile, ProfileEndpoint};
    use hacivat_session::session::{RunId, TxOutcome};
    use hacivat_session::testing::fake_session;
    use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
    use sql_safety::{ClassifyCtx, Expect, classify};

    use super::*;

    fn unit(n: u32) -> UnitId {
        UnitId {
            run: RunId(9),
            ordinal: n,
        }
    }

    fn tracked(n: u32, sql: &str) -> TrackedUnit {
        let ctx = ClassifyCtx::new(
            DialectCtx::new(DialectId::Postgres, None),
            ClientFlavor::None,
            Expect::Script,
        );
        let r = classify(sql, &ctx).items;
        TrackedUnit::new(unit(n), Some(&r), DialectId::Postgres, sql)
    }

    async fn session() -> SessionHandle {
        let profile = Profile {
            name: "orders".into(),
            user: Some("deploy".into()),
            engine: DbmsKind::Postgres,
            endpoint: ProfileEndpoint::Server {
                host: "127.0.0.1".into(),
                port: None,
            },
            auth: AuthProviderId::None,
            retention: Retention::Never,
            ..Profile::default()
        };
        fake_session(
            tokio::runtime::Handle::current(),
            FakeDriver::new(DbmsKind::Postgres),
            profile,
        )
        .await
        .0
    }

    fn started(n: u32) -> RunEvent {
        RunEvent::UnitStarted {
            unit: unit(n),
            at: std::time::Instant::now(),
        }
    }

    fn ended(n: u32, outcome: UnitEnd) -> RunEvent {
        RunEvent::UnitEnded {
            unit: unit(n),
            outcome,
            exec_ms: 0,
            fetch_ms: 0,
        }
    }

    /// The sink maps a run's events to the coordinator's messages: the DDL waits inside the
    /// transaction, the COMMIT ends it; a failed unit plans nothing.
    #[tokio::test]
    async fn the_sink_maps_run_events_to_refresh_messages() {
        let s = session().await;
        let inbox = RefreshInbox::with_capacity(16);
        let mut sink = CatalogRefreshSink::for_run(
            &s,
            DbmsKind::Postgres,
            vec![
                tracked(0, "begin"),
                tracked(1, "create table public.a(x int)"),
                tracked(2, "create table public.b(x int)"),
                tracked(3, "commit"),
            ],
            inbox.clone(),
        );
        let ok = UnitEnd::Succeeded { rows: None };
        for ev in [
            started(0),
            RunEvent::TxChanged {
                from: TxStatus::Idle,
                to: TxStatus::InTx,
            },
            ended(0, ok.clone()),
            started(1),
            ended(1, ok.clone()),
            started(2),
            ended(2, UnitEnd::Failed(ExecError::of(ExecErrorKind::Db))),
            started(3),
            RunEvent::TxChanged {
                from: TxStatus::InTx,
                to: TxStatus::Idle,
            },
            ended(3, ok),
        ] {
            sink.on_event(&ev);
        }
        let q = inbox.queued();
        assert_eq!(q.len(), 2, "{q:?}");
        assert!(matches!(
            &q[0],
            RefreshMsg::Executed { txn: TxnState::InTransaction, stmts, .. } if stmts.len() == 1
        ));
        assert!(matches!(
            &q[1],
            RefreshMsg::TxEnded {
                outcome: TxOutcome::Committed,
                ..
            }
        ));
    }

    /// Review probe "a run slowed by the sink": with nobody draining, the sink keeps going; the
    /// inbox drops its oldest messages and counts them, and never waits.
    #[tokio::test]
    async fn the_sink_never_waits_for_a_stuck_coordinator() {
        let s = session().await;
        let inbox = RefreshInbox::with_capacity(8);
        let mut sink = CatalogRefreshSink::for_run(
            &s,
            DbmsKind::Postgres,
            vec![tracked(0, "create table public.a(x int)")],
            inbox.clone(),
        );
        let n = 20_000u64;
        for _ in 0..n {
            sink.on_event(&started(0));
            sink.on_event(&ended(0, UnitEnd::Succeeded { rows: None }));
        }
        assert_eq!(inbox.len(), 8);
        assert_eq!(inbox.dropped(), n - 8);
    }
}
