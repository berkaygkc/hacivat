// ## m3b-t01c-core-seams
//! Where the classifier runs (R-M3-3, R-M3-53; Task 14): `sql_safety::classify` is pure and
//! synchronous, so the pool owns std threads with 16 MiB stacks and answers through a
//! `tokio::sync::oneshot`; the UI thread never classifies. It is also the read-only guard's
//! [`StatementClassifier`] (R-M3-83), installed by the app's boot from Task 18.
//!
//! ## m3b-t14-orchestrator
//!
//! - **Threads.** [`ClassifyPool::new`] starts `threads` std threads (R-M3-53: 2), each with a
//!   16 MiB stack, that take jobs from one queue. Every libpg_query call a job makes (parse
//!   tree, scanner tokens, fingerprint) runs on `sql-oracle-pg`'s own serial parser thread,
//!   never on a pool thread (T9 ruling), so two pool threads never parse at once. Each pool
//!   thread is started by `sql_safety::spawn_classifier_thread`, so `classify` runs on it
//!   directly; called anywhere else, `classify` makes its own 16 MiB stack (fix round 1, I2).
//! - **Fail closed.** Every entry resolves to a report, never an error the caller must map
//!   (`Result<_, Infallible>`, fix round 1). A job that panics answers an `Unknown` report
//!   (`EffectsUnknown`), and the thread lives on. After [`ClassifyPool::shutdown`] (or when no
//!   thread could start), and while [`QUEUE_CAPACITY`] jobs already wait, every request
//!   answers an `Unknown` report at once. A report that does not arrive within the deadline
//!   is `Unknown` too ([`ClassifyPool::classify_within`]); a queued job whose caller stopped
//!   waiting is skipped, and one whose deadline has passed answers `Unknown` without
//!   classifying (a running C call cannot be cancelled). An `Unknown` report never runs on a
//!   read-only session and always asks on production (R-M3-55): nothing is sent unconfirmed
//!   on a classifier failure.
//! - **Lexing modes.** Without the session's modes `classify` reads a text the modes would lex
//!   differently under each of them and allows it only if every reading allows (fix round 1,
//!   C2; `sql_safety::classify`): `check_read_only` (Count rows) still does. ## m3b-t18-safe06:
//!   the read-only guard's step 0b calls `check_read_only_in` with the session's context
//!   ([`guard_ctx_in`]: the modes, search path and version read at connect), so a known mode
//!   is read once, exactly so; unknown modes stay the fail-closed path.
//! - **Units one at a time.** [`ClassifyPool::classify_units`] classifies a run's units in
//!   order on one thread (`sql_safety::classify_units`): once a unit may change the context
//!   (search path, lexing modes, role, code the classifier cannot see, a cursor or prepared
//!   statement it runs), every later unit is `Unknown`.
//! - **The read-only guard.** [`StatementClassifier::check_read_only`] classifies one statement
//!   with the profile's function allowlist (P-M3b-10) and allows it only when
//!   `decide(.., Profile::ReadOnlyPlusExplain, Actor::Human)` does (R-M3-56); every other answer
//!   is `Deny` with Fluent IDs only. [`StatementClassifier::effects`] maps the report's effects
//!   to [`SessionEffects`] ([`session_effects`]), `None` when they are unknown.
//!
//! Nothing here is wired into Run (R-M3-52): Tasks 15, 18, 24 and 26 do that.

use std::convert::Infallible;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use hacivat_proto::DbmsKind;
use hacivat_session::session::{RoCheck, SafetyDenial, SessionEffects, StatementClassifier};
// ## m3b-t18-safe06
use hacivat_session::session::{ClassifierContext, RoAnswer};
use sql_dialect::ClientFlavor;
use sql_dialect::DialectId;
use sql_safety::{
    Actor, ClassifyCtx, CoarseClass, EffectSet, Expect, Profile, ReasonCode, SafetyReport, Verdict,
    decide,
};
use tokio::sync::oneshot;

use crate::sqlctx::dialect_ctx;

/// A classifier thread's stack (R-M3-3): `sql-safety`'s own.
pub const CLASSIFY_STACK_BYTES: usize = sql_safety::CLASSIFIER_STACK_BYTES;

/// How long the read-only guard waits for a report before it denies (the module docs).
pub const READ_ONLY_CHECK_DEADLINE: Duration = Duration::from_secs(10);

/// How many jobs may wait in the queue; a request past it answers `Unknown` at once (fix round
/// 1, M1).
pub const QUEUE_CAPACITY: usize = 256;

type Job = Box<dyn FnOnce() + Send + 'static>;

// ## m3b-t24-exec-ui (named exception, T1c M7: the "recording fake pool" of Task 24's tests)
/// What [`ClassifyPool::observed`] tells its hook: every text the pool classifies, with its
/// context, on the pool thread, just before it is classified. It cannot change the answer.
pub type ClassifyHook = Arc<dyn Fn(&str, &ClassifyCtx) + Send + Sync>;

/// The classifier's threads (the module docs).
pub struct ClassifyPool {
    threads: usize,
    queue: Mutex<Option<mpsc::SyncSender<Job>>>,
    // ## m3b-t24-exec-ui (named exception)
    hook: Option<ClassifyHook>,
}

// ## m3b-t24-exec-ui (named exception): the hook is a closure, so `Debug` is written out.
impl std::fmt::Debug for ClassifyPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClassifyPool")
            .field("threads", &self.threads)
            .field("hook", &self.hook.is_some())
            .finish_non_exhaustive()
    }
}

impl ClassifyPool {
    /// A pool of `threads` classifier threads (R-M3-53: 2; at least one), each with a 16 MiB
    /// stack. A thread that cannot start is not counted; with none, every request answers
    /// `Unknown`.
    pub fn new(threads: usize) -> Arc<Self> {
        Self::with_hook(threads, None)
    }

    // ## m3b-t24-exec-ui (named exception, T1c M7)
    /// A pool whose `hook` sees every text it classifies, with its context, on the pool thread
    /// just before the real classifier reads it (tests record what was classified, and on
    /// which thread). The answers are the real classifier's; the hook changes nothing.
    pub fn observed(threads: usize, hook: ClassifyHook) -> Arc<Self> {
        Self::with_hook(threads, Some(hook))
    }

    fn with_hook(threads: usize, hook: Option<ClassifyHook>) -> Arc<Self> {
        let (tx, rx) = mpsc::sync_channel::<Job>(QUEUE_CAPACITY);
        let rx = Arc::new(Mutex::new(rx));
        let mut started = 0;
        for i in 0..threads.max(1) {
            let rx = Arc::clone(&rx);
            // A classifier thread (16 MiB): `classify` runs on it directly.
            let spawned =
                sql_safety::spawn_classifier_thread(format!("hacivat-classify-{i}"), move || {
                    worker(&rx);
                });
            if spawned.is_ok() {
                started += 1;
            }
        }
        Arc::new(ClassifyPool {
            threads: started,
            queue: Mutex::new((started > 0).then_some(tx)),
            hook,
        })
    }

    /// How many threads the pool runs.
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Stops taking requests: the jobs already queued still run, every later request answers
    /// `Unknown` at once, and the threads end when the queue is empty.
    pub fn shutdown(&self) {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    /// Classifies `sql` in `ctx` on a pool thread. The future always resolves to a report (the
    /// `Result` cannot fail): a panic in the classifier, a full or shut-down pool, and a lost
    /// job each answer an `Unknown` report of `sql` (fix round 1, M1).
    pub fn classify(
        &self,
        sql: Arc<str>,
        ctx: ClassifyCtx,
    ) -> impl Future<Output = Result<SafetyReport, Infallible>> + Send + 'static {
        self.classify_by(sql, ctx, sql_safety::classify)
    }

    /// [`ClassifyPool::classify`] with the classifier given (the unit tests inject a panic).
    fn classify_by(
        &self,
        sql: Arc<str>,
        ctx: ClassifyCtx,
        classifier: fn(&str, &ClassifyCtx) -> SafetyReport,
    ) -> impl Future<Output = Result<SafetyReport, Infallible>> + Send + 'static {
        let rx = self.start(Arc::clone(&sql), ctx.clone(), None, classifier);
        async move { Ok(rx.await.unwrap_or_else(|_| failed(&sql, &ctx))) }
    }

    /// Queues one classification and gives its receiver. The job is skipped when nobody waits
    /// for it any more (a caller past its deadline dropped the receiver), and answers
    /// `Unknown` without classifying once `deadline` has passed (fix round 1, M1).
    fn start(
        &self,
        sql: Arc<str>,
        ctx: ClassifyCtx,
        deadline: Option<Instant>,
        classifier: fn(&str, &ClassifyCtx) -> SafetyReport,
    ) -> oneshot::Receiver<SafetyReport> {
        let (tx, rx) = oneshot::channel();
        let job_sql = Arc::clone(&sql);
        let job_ctx = ctx.clone();
        let hook = self.hook.clone();
        let job: Job = Box::new(move || {
            if tx.is_closed() {
                return;
            }
            if let Some(hook) = &hook {
                hook(&job_sql, &job_ctx);
            }
            let report = if deadline.is_some_and(|d| Instant::now() >= d) {
                failed(&job_sql, &job_ctx)
            } else {
                catch_unwind(AssertUnwindSafe(|| classifier(&job_sql, &job_ctx)))
                    .unwrap_or_else(|_| failed(&job_sql, &job_ctx))
            };
            let _ = tx.send(report);
        });
        if self.submit(job) {
            return rx;
        }
        answered(failed(&sql, &ctx))
    }

    /// [`ClassifyPool::classify`], awaited for at most `deadline`: a report that does not
    /// arrive in time, or a lost job, is an `Unknown` report of `sql` (never an error); the job
    /// does not run once its deadline has passed.
    pub async fn classify_within(
        &self,
        sql: Arc<str>,
        ctx: ClassifyCtx,
        deadline: Duration,
    ) -> SafetyReport {
        let until = Instant::now() + deadline;
        let rx = self.start(
            Arc::clone(&sql),
            ctx.clone(),
            Some(until),
            sql_safety::classify,
        );
        match tokio::time::timeout(deadline, rx).await {
            Ok(Ok(report)) => report,
            Ok(Err(_)) | Err(_) => failed(&sql, &ctx),
        }
    }

    /// Classifies a run's units in order on one pool thread, each with `ctx`
    /// (`sql_safety::classify_units`: after a unit that may change the context every later
    /// unit is `Unknown`). Always one report per unit: a panic, a full or shut-down pool, or a
    /// lost job answers `Unknown` for every unit.
    pub fn classify_units(
        &self,
        units: Vec<Arc<str>>,
        ctx: ClassifyCtx,
    ) -> impl Future<Output = Result<Vec<SafetyReport>, Infallible>> + Send + 'static {
        let all_failed = |units: &[Arc<str>], ctx: &ClassifyCtx| -> Vec<SafetyReport> {
            units.iter().map(|u| failed(u, ctx)).collect()
        };
        let (tx, rx) = oneshot::channel();
        let job_units = units.clone();
        let job_ctx = ctx.clone();
        let hook = self.hook.clone();
        let job: Job = Box::new(move || {
            if tx.is_closed() {
                return;
            }
            if let Some(hook) = &hook {
                for u in &job_units {
                    hook(u, &job_ctx);
                }
            }
            let reports = catch_unwind(AssertUnwindSafe(|| {
                sql_safety::classify_units(&job_units, &job_ctx)
            }))
            .unwrap_or_else(|_| all_failed(&job_units, &job_ctx));
            let _ = tx.send(reports);
        });
        let rx = if self.submit(job) {
            rx
        } else {
            answered(all_failed(&units, &ctx))
        };
        async move { Ok(rx.await.unwrap_or_else(|_| all_failed(&units, &ctx))) }
    }

    /// Queues `job`; false when the pool takes no more jobs (shut down, no thread started,
    /// every thread gone, or [`QUEUE_CAPACITY`] jobs already waiting).
    fn submit(&self, job: Job) -> bool {
        let queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        queue.as_ref().is_some_and(|q| q.try_send(job).is_ok())
    }
}

impl Drop for ClassifyPool {
    fn drop(&mut self) {
        // The threads end once the queue is empty; a job stuck in a C call is not waited for.
        self.shutdown();
    }
}

/// A pool thread: runs jobs until the queue closes. Each job catches its own panic.
fn worker(rx: &Mutex<mpsc::Receiver<Job>>) {
    loop {
        let job = {
            let rx = rx.lock().unwrap_or_else(PoisonError::into_inner);
            rx.recv()
        };
        match job {
            Ok(job) => {
                let _ = catch_unwind(AssertUnwindSafe(job));
            }
            Err(_) => return,
        }
    }
}

/// A receiver that already holds `value`.
fn answered<T>(value: T) -> oneshot::Receiver<T> {
    let (tx, rx) = oneshot::channel();
    let _ = tx.send(value);
    rx
}

/// The report of a classification that did not happen (a panic, a deadline, a shut-down pool):
/// `Unknown`, pre-decided with `EffectsUnknown`, for exactly `sql`'s bytes and `ctx`'s dialect.
fn failed(sql: &str, ctx: &ClassifyCtx) -> SafetyReport {
    sql_safety::unknown_report(sql, ctx, ReasonCode::EffectsUnknown)
}

/// The session-level effects of a report (C8, Smart, the D-062 wrap), `None` when they are
/// unknown: a pre-decided report, an `Unknown` request, or `EffectSet::UNKNOWN` anywhere.
/// Otherwise the one mapping of [`crate::gate::session_effects_of`] (merge carry, T15 M8): `text`
/// is the classified text and `kind` the session's engine.
pub fn session_effects(r: &SafetyReport, text: &str, kind: DbmsKind) -> Option<SessionEffects> {
    if r.pre_decision.is_some()
        || r.items.is_empty()
        || r.request_class == CoarseClass::Unknown
        || r.request_effects.contains(EffectSet::UNKNOWN)
    {
        return None;
    }
    Some(crate::gate::session_effects_of(r, text, kind))
}

/// The classification context of one statement on a session of `kind` (the read-only guard's
/// path: no server version, the dialect's default lexing modes, no client flavor).
fn guard_ctx(kind: DbmsKind, allow: Arc<[Arc<str>]>) -> ClassifyCtx {
    let mut ctx = ClassifyCtx::new(
        dialect_ctx(kind, None, None),
        ClientFlavor::None,
        Expect::SingleStatement,
    );
    ctx.fn_allowlist = allow;
    ctx
}

// ## m3b-t18-safe06 (plan amendment C2(b)/I1)
/// The classification context of one statement on a read-only session of `kind` in the
/// session's context `c` (the read-only guard's step 0b):
/// - the dialect at the server's version, its modes read from `c.lex_modes` as the console's
///   SQL context reads them ([`dialect_ctx`]); **known** (`with_known_modes`, one reading)
///   when there is such a setting: PostgreSQL `on`/`off`, MySQL/MariaDB any `sql_mode`, and on
///   SQLite always (it has no session mode). Unknown modes stay T14's fallback: the text is
///   read under every mode its bytes can tell apart (fail-closed). A MariaDB `sql_mode` with
///   `ORACLE` is known too, and the interim backend refuses it as an unsupported dialect, so
///   every statement is `Unknown` and denied (T18 carry).
/// - PostgreSQL's search path from `c.search_path`; unknown, it is one unreadable element, so
///   an unqualified builtin is `Unknown` (the PG backend's `catalog_first` contract), never
///   assumed to reach `pg_catalog` first. MySQL/MariaDB: the current database as
///   `search_path[0]` (the T20 carry); unknown, empty (as before).
pub fn guard_ctx_in(kind: DbmsKind, allow: Arc<[Arc<str>]>, c: &ClassifierContext) -> ClassifyCtx {
    // ## m3b-t24-exec-ui (named exception): the dialect and whether its modes are known come
    // from one function, which Run's preparation reads too.
    let (dialect, known) = modes_of(kind, c);
    let modes = dialect.modes;
    let mut ctx = ClassifyCtx::new(dialect, ClientFlavor::None, Expect::SingleStatement);
    if known {
        ctx = ctx.with_known_modes(modes);
    }
    ctx.fn_allowlist = allow;
    match ctx.dialect.id {
        DialectId::Postgres | DialectId::PgWire(_) => {
            ctx.search_path = match &c.search_path {
                Some(path) => Arc::clone(path),
                None => Arc::from(vec![Arc::<str>::from("")]),
            };
        }
        // The T20 carry: `search_path[0]` is the current database; unread, empty (as before).
        DialectId::MySql | DialectId::MariaDb => {
            if let Some(path) = &c.search_path {
                ctx.search_path = Arc::clone(path);
            }
        }
        _ => {}
    }
    ctx
}

// ## m3b-t24-exec-ui (named exception)
/// The dialect of a session of `kind` in its context `c` (its version and lexing modes, read
/// as the console's SQL context reads them), and whether those modes are known: PostgreSQL
/// `on`/`off`, any MySQL/MariaDB `sql_mode`, always on SQLite ([`guard_ctx_in`]'s rule; Run's
/// preparation uses it too, so it scans in the context it classifies in).
pub fn modes_of(kind: DbmsKind, c: &ClassifierContext) -> (sql_dialect::DialectCtx, bool) {
    let dialect = dialect_ctx(kind, c.server_version.as_ref(), c.lex_modes.as_deref());
    let known = match dialect.id {
        DialectId::Postgres | DialectId::PgWire(_) => c
            .lex_modes
            .as_deref()
            .map(str::trim)
            .is_some_and(|v| v.eq_ignore_ascii_case("on") || v.eq_ignore_ascii_case("off")),
        DialectId::MySql | DialectId::MariaDb => c.lex_modes.is_some(),
        DialectId::Sqlite => true,
        _ => false,
    };
    (dialect, known)
}

// ## m3b-t24-exec-ui (named exception: the plan amendment C2(b)/I1 on the Run path, the T24
// I1/I2 rulings)
/// The classification context of a run's units on a session of `kind` in that session's own
/// context `c` (`SessionHandle::classifier_context`): exactly [`guard_ctx_in`]'s rules (one
/// source, one parser: the modes known only when the session read them, the search path as
/// T18 splits it, PostgreSQL unknown as one unreadable element, MySQL/MariaDB the current
/// database), with the profile's allowlist `allow` and the unit's `expect`
/// (`units::expect_for`: `Script` for a unit run as one).
pub fn run_ctx_in(
    kind: DbmsKind,
    allow: Arc<[Arc<str>]>,
    c: &ClassifierContext,
    expect: Expect,
) -> ClassifyCtx {
    let mut ctx = guard_ctx_in(kind, allow, c);
    ctx.expect = expect;
    ctx
}

/// The read-only check's answer for `r` (the module docs).
pub fn read_only_check(r: &SafetyReport) -> RoCheck {
    let d = decide(r, Profile::ReadOnlyPlusExplain, Actor::Human);
    match d.verdict {
        Verdict::Allow => RoCheck::Allow,
        Verdict::Confirm(reasons) | Verdict::Deny(reasons) => {
            let mut ids: Vec<&'static str> = reasons.iter().map(|x| x.code.fluent_id()).collect();
            ids.sort_unstable();
            ids.dedup();
            if ids.is_empty() {
                ids.push(ReasonCode::EffectsUnknown.fluent_id());
            }
            RoCheck::Deny(SafetyDenial {
                class_id: d.class.fluent_id(),
                reasons: ids,
            })
        }
    }
}

impl StatementClassifier for ClassifyPool {
    /// Classifies `sql` as one statement with the profile's allowlist and allows it only when
    /// `ReadOnlyPlusExplain` does for a human; a report that does not arrive within
    /// [`READ_ONLY_CHECK_DEADLINE`] denies.
    fn check_read_only(
        &self,
        kind: DbmsKind,
        sql: Arc<str>,
        allow: Arc<[Arc<str>]>,
    ) -> BoxFuture<'static, RoCheck> {
        let ctx = guard_ctx(kind, allow);
        let until = Instant::now() + READ_ONLY_CHECK_DEADLINE;
        let rx = self.start(
            Arc::clone(&sql),
            ctx.clone(),
            Some(until),
            sql_safety::classify,
        );
        Box::pin(async move {
            let report = match tokio::time::timeout(READ_ONLY_CHECK_DEADLINE, rx).await {
                Ok(Ok(r)) => r,
                Ok(Err(_)) | Err(_) => failed(&sql, &ctx),
            };
            read_only_check(&report)
        })
    }

    // ## m3b-t18-safe06 (plan amendment C2(b)/I1)
    /// [`StatementClassifier::check_read_only`] in the session's context ([`guard_ctx_in`]):
    /// with the lexing modes known the text is read once, exactly so; whether the statement may
    /// change the context is `sql_safety::changes_context` of its report.
    fn check_read_only_in(
        &self,
        kind: DbmsKind,
        sql: Arc<str>,
        allow: Arc<[Arc<str>]>,
        ctx: ClassifierContext,
    ) -> BoxFuture<'static, RoAnswer> {
        let ctx = guard_ctx_in(kind, allow, &ctx);
        let until = Instant::now() + READ_ONLY_CHECK_DEADLINE;
        let rx = self.start(
            Arc::clone(&sql),
            ctx.clone(),
            Some(until),
            sql_safety::classify,
        );
        Box::pin(async move {
            let report = match tokio::time::timeout(READ_ONLY_CHECK_DEADLINE, rx).await {
                Ok(Ok(r)) => r,
                Ok(Err(_)) | Err(_) => failed(&sql, &ctx),
            };
            RoAnswer {
                check: read_only_check(&report),
                changes_context: sql_safety::changes_context(&sql, &report),
            }
        })
    }

    /// The session-level effects of `sql` as one statement ([`session_effects`]); `None` when
    /// they are unknown or the report does not arrive within [`READ_ONLY_CHECK_DEADLINE`].
    fn effects(&self, kind: DbmsKind, sql: Arc<str>) -> BoxFuture<'static, Option<SessionEffects>> {
        let ctx = guard_ctx(kind, Arc::from(Vec::new()));
        let until = Instant::now() + READ_ONLY_CHECK_DEADLINE;
        let rx = self.start(Arc::clone(&sql), ctx, Some(until), sql_safety::classify);
        Box::pin(async move {
            match tokio::time::timeout(READ_ONLY_CHECK_DEADLINE, rx).await {
                Ok(Ok(r)) => session_effects(&r, &sql, kind),
                Ok(Err(_)) | Err(_) => None,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use sql_dialect::{DialectCtx, DialectId};

    use super::*;

    fn pg() -> ClassifyCtx {
        ClassifyCtx::new(
            DialectCtx::new(DialectId::Postgres, None),
            ClientFlavor::None,
            Expect::SingleStatement,
        )
    }

    fn panics(_: &str, _: &ClassifyCtx) -> SafetyReport {
        panic!("an injected classifier panic")
    }

    /// A job that holds a pool thread until the returned sender is dropped or sends; it
    /// returns once the thread has taken the job (so the queue holds only later jobs).
    fn hold(p: &ClassifyPool) -> mpsc::Sender<()> {
        let (tx, rx) = mpsc::channel::<()>();
        let (started_tx, started) = mpsc::channel::<()>();
        assert!(p.submit(Box::new(move || {
            let _ = started_tx.send(());
            let _ = rx.recv();
        })));
        started.recv().expect("the pool thread takes the job");
        tx
    }

    #[tokio::test]
    async fn a_panic_is_unknown_and_the_thread_lives_on() {
        let p = ClassifyPool::new(1);
        let r = p
            .classify_by("SELECT 1".into(), pg(), panics)
            .await
            .unwrap();
        assert_eq!(r.request_class, CoarseClass::Unknown);
        assert!(r.matches_text("SELECT 1"));
        assert_eq!(r.dialect, Some(DialectId::Postgres));
        assert_eq!(
            r.pre_decision.as_deref().map(|v| v[0].code),
            Some(ReasonCode::EffectsUnknown)
        );
        assert_eq!(
            read_only_check(&r),
            RoCheck::Deny(SafetyDenial {
                class_id: "safety-class-unknown",
                reasons: vec![ReasonCode::EffectsUnknown.fluent_id()],
            })
        );
        assert_eq!(session_effects(&r, "SELECT 1", DbmsKind::Postgres), None);
        // The one thread still classifies.
        let ok = p.classify("SELECT 1".into(), pg()).await.unwrap();
        assert_eq!(ok.request_class, CoarseClass::Read);
    }

    #[tokio::test(start_paused = true)]
    async fn a_report_past_its_deadline_is_unknown() {
        let p = ClassifyPool::new(1);
        let release = hold(&p);
        // The only thread is held: the request cannot be classified before its deadline.
        let r = p
            .classify_within("SELECT 1".into(), pg(), Duration::from_millis(5))
            .await;
        assert_eq!(r.request_class, CoarseClass::Unknown);
        assert!(r.matches_text("SELECT 1"));
        // The read-only check denies on its own deadline the same way.
        let check = p.check_read_only(DbmsKind::Postgres, "SELECT 1".into(), Arc::from(Vec::new()));
        assert!(matches!(check.await, RoCheck::Deny(_)));
        assert_eq!(p.effects(DbmsKind::Postgres, "SELECT 1".into()).await, None);
        drop(release);
        // Released, the thread answers again (awaited without a deadline: the paused clock
        // would jump past any deadline while the std thread works).
        let r = p.classify("SELECT 1".into(), pg()).await.unwrap();
        assert_eq!(r.request_class, CoarseClass::Read);
    }

    #[tokio::test]
    async fn queued_jobs_still_run_after_a_shutdown() {
        let p = ClassifyPool::new(1);
        let release = hold(&p);
        let queued = p.classify("DELETE FROM t".into(), pg());
        p.shutdown();
        let refused = p.classify("SELECT 1".into(), pg()).await.unwrap();
        assert_eq!(refused.request_class, CoarseClass::Unknown);
        drop(release);
        assert_eq!(queued.await.unwrap().request_class, CoarseClass::Write);
    }

    // ## fix round 1, M1

    #[tokio::test]
    async fn a_full_queue_answers_unknown_at_once() {
        let p = ClassifyPool::new(1);
        let release = hold(&p);
        // The held job may still sit in the queue or already run: fill every slot left.
        let mut waiting = Vec::new();
        while p.submit(Box::new(|| {})) {
            waiting.push(());
            assert!(waiting.len() <= QUEUE_CAPACITY + 1);
        }
        let r = p.classify("SELECT 1".into(), pg()).await.unwrap();
        assert_eq!(r.request_class, CoarseClass::Unknown);
        assert!(r.matches_text("SELECT 1"));
        let units = p
            .classify_units(vec!["SELECT 1".into()], pg())
            .await
            .unwrap();
        assert_eq!(units[0].request_class, CoarseClass::Unknown);
        drop(release);
        // Drained, the pool classifies again.
        let r = loop {
            let r = p.classify("SELECT 1".into(), pg()).await.unwrap();
            if r.pre_decision.is_none() {
                break r;
            }
            tokio::task::yield_now().await;
        };
        assert_eq!(r.request_class, CoarseClass::Read);
    }

    fn counting(sql: &str, ctx: &ClassifyCtx) -> SafetyReport {
        RAN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        sql_safety::classify(sql, ctx)
    }

    static RAN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    #[tokio::test]
    async fn a_job_past_its_deadline_or_without_a_waiter_does_not_classify() {
        let p = ClassifyPool::new(1);
        let release = hold(&p);
        // Past its deadline when the thread reaches it: Unknown, not classified.
        let late = p.start("SELECT 1".into(), pg(), Some(Instant::now()), counting);
        // Nobody waits any more: skipped.
        drop(p.start("SELECT 2".into(), pg(), None, counting));
        let before = RAN.load(std::sync::atomic::Ordering::SeqCst);
        drop(release);
        let r = late.await.unwrap();
        assert_eq!(r.request_class, CoarseClass::Unknown);
        assert!(r.matches_text("SELECT 1"));
        // A later job runs after both: neither of them classified.
        let after = p
            .start("SELECT 3".into(), pg(), None, counting)
            .await
            .unwrap();
        assert_eq!(after.request_class, CoarseClass::Read);
        assert_eq!(RAN.load(std::sync::atomic::Ordering::SeqCst), before + 1);
    }

    #[test]
    fn denial_reasons_are_unique() {
        let r = sql_safety::classify("SELECT f(1), g(2), f(3)", &pg());
        match read_only_check(&r) {
            RoCheck::Deny(d) => {
                let mut sorted = d.reasons.clone();
                sorted.sort_unstable();
                sorted.dedup();
                assert_eq!(sorted.len(), d.reasons.len(), "{:?}", d.reasons);
            }
            RoCheck::Allow => panic!("unknown functions allowed"),
        }
    }
}
