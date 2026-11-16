//! M3b Task 18: step 0b, the read-only guard's classifier (R-M3-83, R-M3-56; SQL-SAFE-06 run
//! A′ on a fake server). A user statement of a read-only session (an M2b `Exec { origin: User }`
//! or a typed `Run` unit) is asked of the injected classifier first, in the session's context
//! (plan amendment C2(b)/I1: the lexing modes, the search path and the server version the
//! connect read); a denial is `SafetyDenied` and nothing is sent. Internal statements are
//! exempt; a session without a classifier keeps M2b/M3a's layers alone (run B). MySQL and
//! MariaDB read-only sessions pin the observed `sql_mode` after every reset (R-M3-57).
//!
//! The classifier here is a fake (`hacivat-session` has no `sql-safety` edge, R-M3-83); the real
//! pool over a fake server is `hacivat-exec/tests/docker_m3b_bypass.rs` (P-M3b-13).

#![allow(
    clippy::result_large_err,
    reason = "Reply is the session's fixed interface"
)]

mod common;

use std::sync::{Arc, Mutex};

use common::runs::{Recorder, spec_of};
use common::*;
use futures::future::BoxFuture;
use hacivat_driver::fake::{FakeCall, FakeDriver, FakeExec, FakeTypedCall};
use hacivat_driver::{ExecSummary, SemVerLoose, ServerInfo, SessionSettings};
use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    ClassifierContext, ExecErrorKind, ExecOrigin, Reply, Request, RoAnswer, RoCheck, SafetyDenial,
    SessionEffects, SessionHandle, StatementClassifier, UnitEnd, context_probe_sql,
};
use hacivat_session::{ConnectionManager, ManagerConfig};

// ---------------------------------------------------------------------------------------------
// The fake classifier

/// A classifier that denies statements starting with one word (ASCII case ignored), records
/// what it was asked in which context, and says whether each statement changes the context.
#[derive(Debug, Default)]
struct Fake {
    deny_word: Option<&'static str>,
    changes_context: bool,
    asked: Mutex<Vec<(String, ClassifierContext)>>,
}

impl Fake {
    fn asked(&self) -> Vec<(String, ClassifierContext)> {
        self.asked.lock().unwrap().clone()
    }

    fn denies(&self, sql: &str) -> bool {
        self.deny_word.is_some_and(|w| {
            sql.trim_start()
                .get(..w.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(w))
        })
    }
}

fn denial() -> SafetyDenial {
    SafetyDenial {
        class_id: "safety-class-write",
        reasons: vec!["safety-reason-write-data"],
    }
}

impl StatementClassifier for Fake {
    fn check_read_only(
        &self,
        _kind: DbmsKind,
        _sql: Arc<str>,
        _allow: Arc<[Arc<str>]>,
    ) -> BoxFuture<'static, RoCheck> {
        panic!("step 0b asks in context (check_read_only_in), never without one");
    }

    fn effects(
        &self,
        _kind: DbmsKind,
        _sql: Arc<str>,
    ) -> BoxFuture<'static, Option<SessionEffects>> {
        Box::pin(async { None })
    }

    fn check_read_only_in(
        &self,
        _kind: DbmsKind,
        sql: Arc<str>,
        _allow: Arc<[Arc<str>]>,
        ctx: ClassifierContext,
    ) -> BoxFuture<'static, RoAnswer> {
        self.asked.lock().unwrap().push((sql.to_string(), ctx));
        let check = if self.denies(&sql) {
            RoCheck::Deny(denial())
        } else {
            RoCheck::Allow
        };
        let changes_context = self.changes_context;
        Box::pin(async move {
            RoAnswer {
                check,
                changes_context,
            }
        })
    }
}

fn fake_classifier_denying(word: &'static str) -> Arc<Fake> {
    Arc::new(Fake {
        deny_word: Some(word),
        ..Fake::default()
    })
}

fn fake_classifier_allowing() -> Arc<Fake> {
    Arc::new(Fake::default())
}

// ---------------------------------------------------------------------------------------------
// Sessions

const CANARY: &str = "SELECT pg_catalog.txid_current_if_assigned()";

/// The guard's canary answering SQL NULL (no transaction ID).
fn clean_canary() -> FakeExec {
    FakeExec {
        prefix: CANARY,
        result: Ok(ExecSummary {
            rows: vec![vec![None]],
            row_count: 1,
            ..ExecSummary::default()
        }),
        delay: std::time::Duration::ZERO,
        tx_after: None,
        panics: false,
    }
}

/// A PostgreSQL fake whose guard passes `rounds` times.
fn pg_ro_clean(rounds: usize) -> FakeDriver {
    fake_pg()
        .tracks_transactions()
        .exec((0..rounds).map(|_| clean_canary()).collect())
}

/// A read-only PostgreSQL session over a fake whose guard passes, asking `c`.
async fn ro_session_with(c: Arc<Fake>) -> (SessionHandle, Arc<FakeDriver>, Arc<ConnectionManager>) {
    ro_over(pg_ro_clean(4), ro_profile(), c).await
}

/// The default session of `profile` (made read-only) on a manager over `fake` whose
/// `ManagerConfig::classifier` is `c`: the product path (the manager hands its classifier to
/// every session it starts). The manager is returned so it outlives the test's requests.
async fn ro_over(
    fake: FakeDriver,
    profile: Profile,
    c: Arc<Fake>,
) -> (SessionHandle, Arc<FakeDriver>, Arc<ConnectionManager>) {
    let (h, f, m) = classified(fake, ro(profile), c).await;
    assert!(snap(&h).read_only);
    (h, f, m)
}

/// [`ro_over`] without making the profile read-only.
async fn classified(
    fake: FakeDriver,
    profile: Profile,
    c: Arc<Fake>,
) -> (SessionHandle, Arc<FakeDriver>, Arc<ConnectionManager>) {
    let config = ManagerConfig {
        classifier: Some(c as Arc<dyn StatementClassifier>),
        ..ManagerConfig::m2b()
    };
    let (m, f) = manager_with(fake, config);
    let profile = Arc::new(profile);
    m.connect(
        profile.clone(),
        None,
        hacivat_session::pipeline::Interaction::Headless,
    );
    wait_connected(&m, profile.id).await;
    let h = m
        .sessions(profile.id)
        .expect("connected")
        .default_session()
        .expect("a default session");
    (h, f, m)
}

/// An M2b user statement.
async fn exec_user(h: &SessionHandle, sql: &str) -> Reply {
    run(h, sql).await
}

fn safety_denied(reply: &Reply) -> bool {
    matches!(reply, Err(e) if matches!(e.kind, ExecErrorKind::SafetyDenied(_)))
}

/// What the fake ran from `mark` on.
fn since(f: &FakeDriver, mark: usize) -> Vec<FakeCall> {
    f.call_log()[mark..].to_vec()
}

fn sent(f: &FakeDriver, mark: usize, sql: &str) -> bool {
    since(f, mark).iter().any(|c| match c {
        FakeCall::Statement(s) => s == sql,
        FakeCall::Typed(FakeTypedCall::Query(q)) => q == sql,
        _ => false,
    })
}

// ---------------------------------------------------------------------------------------------
// Step 0b

#[tokio::test]
async fn denied_statement_sends_nothing() {
    let (h, f, _m) = ro_session_with(fake_classifier_denying("INSERT")).await;
    // The env plan ran at connect (P-M3b-18): the mark is taken after it.
    let before = f.call_log().len();
    let e = exec_user(&h, "INSERT INTO t VALUES (1)").await.unwrap_err();
    assert!(matches!(e.kind, ExecErrorKind::SafetyDenied(_)), "{e:?}");
    assert!(f.call_log()[before..].is_empty());
}

#[tokio::test]
async fn the_denial_carries_the_classifiers_fluent_ids_only() {
    let (h, _f, _m) = ro_session_with(fake_classifier_denying("SELECT")).await;
    let e = exec_user(&h, "SELECT 'a secret literal'")
        .await
        .unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::SafetyDenied(denial()));
    assert!(e.server.is_none());
    assert!(!format!("{e:?}").contains("secret"), "{e:?}");
}

#[tokio::test]
async fn a_denied_typed_unit_sends_nothing() {
    // `txid_current` passes M3a's first-keyword write list (DEMO-7): only the classifier stops
    // it before the guard.
    let c = fake_classifier_denying("SELECT TXID");
    let (h, f, _m) = ro_session_with(c.clone()).await;
    let before = f.call_log().len();
    let o = Recorder::new();
    let reply = h
        .run(spec_of(&o, &["SELECT txid_current()"]))
        .reply
        .await
        .unwrap();
    assert!(safety_denied(&reply), "{reply:?}");
    assert!(
        o.events().iter().any(|e| matches!(
            e,
            hacivat_session::session::RunEvent::UnitEnded {
                outcome: UnitEnd::Failed(err),
                ..
            } if matches!(err.kind, ExecErrorKind::SafetyDenied(_))
        )),
        "{:?}",
        o.events()
    );
    assert!(since(&f, before).is_empty(), "{:?}", since(&f, before));
    assert_eq!(c.asked().len(), 1);
}

#[tokio::test]
async fn an_allowed_statement_still_meets_every_guard_layer() {
    // The classifier only ever denies early (`alg/13` §0.1): an allowed statement runs inside
    // the guard's read-only transaction, with its canary, rollback, reset and replay.
    let c = fake_classifier_allowing();
    let (h, f, _m) = ro_session_with(c.clone()).await;
    let before = f.call_log().len();
    exec_user(&h, "SELECT 1").await.unwrap();
    let calls = since(&f, before);
    let at = |s: &str| {
        calls
            .iter()
            .position(|c| *c == FakeCall::Statement(s.into()))
            .unwrap_or_else(|| panic!("{s} not sent: {calls:?}"))
    };
    let begin = at("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY");
    let stmt = at("SELECT 1");
    let canary = at(CANARY);
    let rollback = at("ROLLBACK");
    assert!(
        begin < stmt && stmt < canary && canary < rollback,
        "{calls:?}"
    );
    assert!(calls.contains(&FakeCall::Reset), "{calls:?}");
    assert_eq!(c.asked().len(), 1);
}

#[tokio::test]
async fn internal_probe_is_not_classified() {
    let c = fake_classifier_denying("SELECT");
    let (h, f, _m) = ro_session_with(c.clone()).await;
    let before = f.call_log().len();
    let probe = context_probe_sql(DbmsKind::Postgres).unwrap();
    let reply = h
        .submit(Request::Exec {
            sql: probe.to_string(),
            opts: hacivat_driver::ExecOpts::values(1),
            origin: ExecOrigin::Internal { idempotent: true },
        })
        .reply
        .await
        .unwrap();
    assert!(reply.is_ok(), "{reply:?}");
    assert!(sent(&f, before, probe));
    // The schema probe after a user statement is internal too: only the statement was asked.
    assert!(c.asked().is_empty(), "{:?}", c.asked());
}

#[tokio::test]
async fn no_classifier_keeps_m2b_behaviour() {
    // Run B: without a classifier the statement reaches the guard (and the server decides).
    let (h, f) = ro_session(pg_ro_clean(2), ro_profile()).await;
    let before = f.call_log().len();
    let reply = exec_user(&h, "SELECT txid_current()").await;
    assert!(!safety_denied(&reply), "{reply:?}");
    assert!(sent(&f, before, "SELECT txid_current()"));
    // The M3a first-keyword layer still refuses a typed write before anything is sent.
    let o = Recorder::new();
    let mark = f.call_log().len();
    let reply = h
        .run(spec_of(&o, &["INSERT INTO t VALUES (1)"]))
        .reply
        .await
        .unwrap();
    assert!(reply.is_err() && !safety_denied(&reply), "{reply:?}");
    assert!(since(&f, mark).is_empty());
}

#[tokio::test]
async fn call_is_denied_on_read_only() {
    // ER15: `CALL` never runs on a read-only session. The M2b path has no first-keyword
    // layer, so step 0b is what refuses it there (the real classifier's verdict is pinned in
    // `hacivat-exec/tests/docker_m3b_bypass.rs`); nothing reaches the server.
    let (h, f, _m) = ro_session_with(fake_classifier_denying("CALL")).await;
    let before = f.call_log().len();
    let reply = exec_user(&h, "CALL hv_proc()").await;
    assert!(safety_denied(&reply), "{reply:?}");
    assert!(since(&f, before).is_empty());
}

#[tokio::test]
async fn a_read_write_session_never_asks_the_classifier() {
    let c = fake_classifier_denying("SELECT");
    let (h, f, _m) = classified(fake_pg(), pg_profile(TlsMode::Disable), c.clone()).await;
    assert!(!snap(&h).read_only);
    exec_user(&h, "SELECT 1").await.unwrap();
    assert!(f.executed().iter().any(|s| s == "SELECT 1"));
    assert!(c.asked().is_empty());
}

// ---------------------------------------------------------------------------------------------
// The context (plan amendment C2(b)/I1)

fn version(v: &str) -> ServerInfo {
    ServerInfo {
        version: SemVerLoose::parse(v),
        raw_version: v.into(),
        ..pg_info(v)
    }
}

fn settings(lex_modes: Option<&str>, search_path: Option<&str>) -> SessionSettings {
    SessionSettings {
        lex_modes: lex_modes.map(Arc::from),
        search_path: search_path.map(Arc::from),
    }
}

#[tokio::test]
async fn the_classifier_gets_the_connects_context() {
    let c = fake_classifier_allowing();
    let fake = pg_ro_clean(2)
        .server(version("17.4"))
        .session_settings(settings(Some("on"), Some("\"$user\", Public, \"My S\"")));
    let (h, _f, _m) = ro_over(fake, ro_profile(), c.clone()).await;
    exec_user(&h, "SELECT 1").await.unwrap();
    let (_, ctx) = c.asked().remove(0);
    assert_eq!(ctx.lex_modes.as_deref(), Some("on"));
    let path: Vec<&str> = ctx
        .search_path
        .as_deref()
        .expect("read at connect")
        .iter()
        .map(|s| &**s)
        .collect();
    // Split and folded as the server reads the setting; `$user` stays the token.
    assert_eq!(path, ["$user", "public", "My S"]);
    assert_eq!(
        ctx.server_version.map(|v| (v.major, v.minor)),
        Some((17, 4))
    );
}

#[tokio::test]
async fn an_unread_or_off_context_is_unknown() {
    // PostgreSQL: `standard_conforming_strings` off at connect (the replay pins it on after the
    // first reset, so the session's statements do not all lex alike) or not read → unknown
    // modes; a search path that was not read → unknown (`None`: fail-closed downstream).
    let c = fake_classifier_allowing();
    let fake = pg_ro_clean(2).session_settings(settings(Some("off"), None));
    let (h, _f, _m) = ro_over(fake, ro_profile(), c.clone()).await;
    exec_user(&h, "SELECT 1").await.unwrap();
    let (_, ctx) = c.asked().remove(0);
    assert_eq!(ctx.lex_modes, None);
    assert_eq!(ctx.search_path, None);
}

#[tokio::test]
async fn mysql_gets_the_observed_sql_mode() {
    let c = fake_classifier_allowing();
    let fake = fake_mysql()
        .tracks_transactions()
        .session_settings(settings(Some("ANSI_QUOTES,STRICT_TRANS_TABLES"), None));
    let (h, _f, _m) = ro_over(fake, mysql_profile(TlsMode::Disable), c.clone()).await;
    exec_user(&h, "SELECT 1").await.unwrap();
    exec_user(&h, "SELECT 2").await.unwrap();
    for (_, ctx) in c.asked() {
        assert_eq!(
            ctx.lex_modes.as_deref(),
            Some("ANSI_QUOTES,STRICT_TRANS_TABLES")
        );
        assert_eq!(ctx.search_path, None);
    }
    // The T20 carry: the current database read at connect is `search_path[0]`.
    let c = fake_classifier_allowing();
    let fake = fake_mysql()
        .tracks_transactions()
        .session_settings(settings(Some("STRICT_TRANS_TABLES"), Some("app")));
    let (h, _f, _m) = ro_over(fake, mysql_profile(TlsMode::Disable), c.clone()).await;
    exec_user(&h, "SELECT 1").await.unwrap();
    let path: Vec<String> = c.asked()[0]
        .1
        .search_path
        .as_deref()
        .expect("the database")
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(path, ["app"]);
    // A mode that cannot be pinned is never trusted as the session's.
    let c = fake_classifier_allowing();
    let fake = fake_mysql()
        .tracks_transactions()
        .session_settings(settings(Some("ANSI'; DROP TABLE t; --"), None));
    let (h, f, _m) = ro_over(fake, mysql_profile(TlsMode::Disable), c.clone()).await;
    exec_user(&h, "SELECT 1").await.unwrap();
    assert_eq!(c.asked()[0].1.lex_modes, None);
    assert!(
        !f.executed().iter().any(|s| s.contains("sql_mode")),
        "{:?}",
        f.executed()
    );
}

#[tokio::test]
async fn the_context_is_read_again_after_a_statement_that_may_change_it() {
    // T9 R1-M5 / the T18 carry: after the unit that may change the context (a FETCH, an
    // EXECUTE, a SET the classifier allows), the next check reads it again, with Hacivat's own
    // constant query, outside the guard.
    let reread = "SELECT pg_catalog.current_setting('standard_conforming_strings')";
    let c = Arc::new(Fake {
        changes_context: true,
        ..Fake::default()
    });
    let mut script: Vec<FakeExec> = (0..3).map(|_| clean_canary()).collect();
    script.push(rows(reread, &[&["on", "app, pg_catalog"]]));
    let fake = fake_pg()
        .tracks_transactions()
        .exec(script)
        .session_settings(settings(Some("on"), Some("public")));
    let (h, f, _m) = ro_over(fake, ro_profile(), c.clone()).await;
    let mark = f.call_log().len();
    exec_user(&h, "FETCH ALL FROM c").await.unwrap();
    assert!(
        !since(&f, mark)
            .iter()
            .any(|c| matches!(c, FakeCall::Statement(s) if s.starts_with(reread))),
        "the first check uses the connect's read"
    );
    exec_user(&h, "SELECT 42").await.unwrap();
    let read_at = since(&f, mark)
        .iter()
        .position(|c| matches!(c, FakeCall::Statement(s) if s.starts_with(reread)))
        .expect("read again");
    let select_at = since(&f, mark)
        .iter()
        .position(|c| *c == FakeCall::Statement("SELECT 42".into()))
        .unwrap();
    assert!(read_at < select_at, "read before the next statement");
    let asked = c.asked();
    let path = |i: usize| -> Vec<String> {
        asked[i]
            .1
            .search_path
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.to_string())
            .collect()
    };
    assert_eq!(path(0), ["public"]);
    assert_eq!(path(1), ["app", "pg_catalog"]);
}

#[tokio::test]
async fn an_unanswered_reread_leaves_the_context_unknown() {
    let c = Arc::new(Fake {
        changes_context: true,
        ..Fake::default()
    });
    let fake = pg_ro_clean(3).session_settings(settings(Some("on"), Some("public")));
    let (h, _f, _m) = ro_over(fake, ro_profile(), c.clone()).await;
    exec_user(&h, "SET search_path = public").await.unwrap();
    exec_user(&h, "SELECT 1").await.unwrap();
    let (_, ctx) = c.asked().remove(1);
    assert_eq!(ctx.lex_modes, None);
    assert_eq!(ctx.search_path, None);
}

// ---------------------------------------------------------------------------------------------
// R-M3-57: the observed `sql_mode` pinned after every reset

#[tokio::test]
async fn mysql_sql_mode_is_pinned_after_every_reset() {
    let fake = fake_mysql()
        .tracks_transactions()
        .session_settings(settings(Some("NO_BACKSLASH_ESCAPES"), None));
    let (h, f) = ro_session(fake, mysql_profile(TlsMode::Disable)).await;
    let mark = f.call_log().len();
    exec_user(&h, "SELECT 1").await.unwrap();
    exec_user(&h, "SELECT 2").await.unwrap();
    let calls = since(&f, mark);
    let pin = FakeCall::Statement("SET SESSION sql_mode = 'NO_BACKSLASH_ESCAPES'".into());
    let resets: Vec<usize> = calls
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == FakeCall::Reset)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(resets.len(), 2, "{calls:?}");
    for r in resets {
        let after = &calls[r..];
        let names = after
            .iter()
            .position(|c| *c == FakeCall::Statement("SET NAMES utf8mb4".into()))
            .expect("the setup replayed");
        assert_eq!(after.get(names + 1), Some(&pin), "{calls:?}");
    }
    // A read-write session pins nothing.
    let fake = fake_mysql().session_settings(settings(Some("NO_BACKSLASH_ESCAPES"), None));
    let (h, f) = session(fake).await;
    exec_user(&h, "SELECT 1").await.unwrap();
    assert!(!f.executed().iter().any(|s| s.contains("sql_mode")));
}

// ---------------------------------------------------------------------------------------------
// T1c review M7: the classifier in `ManagerConfig` is the one `Core` uses

#[tokio::test]
async fn the_managers_classifier_reaches_every_read_only_session() {
    let c = fake_classifier_denying("SELECT");
    let config = ManagerConfig {
        classifier: Some(c.clone() as Arc<dyn StatementClassifier>),
        ..ManagerConfig::m2b()
    };
    let (m, f) = manager_with(FakeDriver::new(DbmsKind::Sqlite), config);
    let profile = Arc::new(ro(sqlite_profile()));
    m.connect(
        profile.clone(),
        None,
        hacivat_session::pipeline::Interaction::Headless,
    );
    wait_connected(&m, profile.id).await;
    let h = m.sessions(profile.id).unwrap().default_session().unwrap();
    assert!(snap(&h).read_only);
    let mark = f.call_log().len();
    assert!(safety_denied(&run(&h, "SELECT 1").await));
    let o = Recorder::new();
    let reply = h.run(spec_of(&o, &["SELECT 2"])).reply.await.unwrap();
    assert!(safety_denied(&reply), "{reply:?}");
    assert!(since(&f, mark).is_empty(), "{:?}", since(&f, mark));
    assert_eq!(
        c.asked()
            .iter()
            .map(|(s, _)| s.as_str())
            .collect::<Vec<_>>(),
        ["SELECT 1", "SELECT 2"]
    );
}

// ---------------------------------------------------------------------------------------------
// ## m3b-t18-safe06 fix round 1

/// What the re-read query answers: no row (a failed read).
fn reread_without_row(prefix: &'static str) -> FakeExec {
    FakeExec {
        prefix,
        result: Ok(ExecSummary::default()),
        delay: std::time::Duration::ZERO,
        tx_after: None,
        panics: false,
    }
}

#[tokio::test]
async fn a_failed_reread_is_tried_again_at_the_next_check() {
    // m2: a failed read is not sticky. The check after it is told nothing (fail-closed), the
    // next one reads again, once per statement.
    let reread = "SELECT pg_catalog.current_setting('standard_conforming_strings')";
    let c = Arc::new(Fake {
        changes_context: true,
        ..Fake::default()
    });
    let mut script: Vec<FakeExec> = (0..4).map(|_| clean_canary()).collect();
    script.push(reread_without_row(reread));
    script.push(rows(reread, &[&["on", "app, pg_catalog"]]));
    let fake = fake_pg()
        .tracks_transactions()
        .exec(script)
        .session_settings(settings(Some("on"), Some("public")));
    let (h, f, _m) = ro_over(fake, ro_profile(), c.clone()).await;
    let reads = |f: &FakeDriver| {
        f.call_log()
            .iter()
            .filter(|c| matches!(c, FakeCall::Statement(s) if s.starts_with(reread)))
            .count()
    };
    exec_user(&h, "FETCH ALL FROM c").await.unwrap();
    assert_eq!(reads(&f), 0);
    exec_user(&h, "SELECT 41").await.unwrap();
    assert_eq!(reads(&f), 1, "one read for the second statement");
    exec_user(&h, "SELECT 42").await.unwrap();
    assert_eq!(reads(&f), 2, "read again, once, for the third");
    let asked = c.asked();
    assert_eq!(asked[1].1.search_path, None, "the failed read: unknown");
    assert_eq!(asked[1].1.lex_modes, None);
    let path: Vec<String> = asked[2]
        .1
        .search_path
        .as_deref()
        .expect("the second read answered")
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(path, ["app", "pg_catalog"]);
}

#[tokio::test]
async fn a_schema_switch_whose_probe_fails_leaves_the_schema_unknown() {
    // m3: after `SwitchSchema`, a re-probe that fails never lets the classifier use the old
    // schema (the replay's) or the connect's: the search path is unknown.
    let probe = "SELECT current_database()";
    let fails = FakeExec {
        prefix: probe,
        result: Err(db("XX000")),
        delay: std::time::Duration::ZERO,
        tx_after: None,
        panics: false,
    };
    let mut script: Vec<FakeExec> = (0..2).map(|_| clean_canary()).collect();
    script.push(fails);
    let c = fake_classifier_allowing();
    let fake = fake_pg()
        .tracks_transactions()
        .exec(script)
        .session_settings(settings(Some("on"), Some("public")));
    let (h, _f, _m) = ro_over(fake, ro_profile(), c.clone()).await;
    h.submit(Request::SwitchSchema("app".into()))
        .reply
        .await
        .unwrap()
        .unwrap();
    exec_user(&h, "SELECT 1").await.unwrap();
    assert_eq!(c.asked()[0].1.search_path, None);
    // A switch whose probe answers gives the classifier the new path.
    let mut script: Vec<FakeExec> = (0..2).map(|_| clean_canary()).collect();
    script.push(rows(probe, &[&["db", "app", "pg_catalog,app", "u", "app"]]));
    let c = fake_classifier_allowing();
    let fake = fake_pg()
        .tracks_transactions()
        .exec(script)
        .session_settings(settings(Some("on"), Some("public")));
    let (h, _f, _m) = ro_over(fake, ro_profile(), c.clone()).await;
    h.submit(Request::SwitchSchema("app".into()))
        .reply
        .await
        .unwrap()
        .unwrap();
    exec_user(&h, "SELECT 1").await.unwrap();
    let path: Vec<String> = c.asked()[0]
        .1
        .search_path
        .as_deref()
        .expect("the probed path")
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(path, ["app"]);
}

// ---------------------------------------------------------------------------------------------
// ## m3b-t24-exec-ui (fix round 1, I1/I2/I6): Run's classification context, the session's own

/// T18's constant context read (`readonly::PG_CONTEXT_SQL`), which Run's context reads too.
const RUN_CTX_READ: &str = "SELECT pg_catalog.current_setting('standard_conforming_strings')";

fn path_of(ctx: &ClassifierContext) -> Vec<String> {
    ctx.search_path
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn ctx_reads(f: &FakeDriver) -> usize {
    f.executed()
        .iter()
        .filter(|s| s.starts_with(RUN_CTX_READ))
        .count()
}

#[tokio::test]
async fn run_context_is_the_sessions_own_read_and_never_stale() {
    let fake = fake_pg().tracks_transactions().exec(vec![
        rows(RUN_CTX_READ, &[&["off", "\"$user\", Public, \"My S\""]]),
        rows(RUN_CTX_READ, &[&["on", "app"]]),
    ]);
    let (h, f) = session(fake).await;
    let ctx = h.classifier_context().await;
    assert_eq!(ctx.lex_modes.as_deref(), Some("off"));
    assert_eq!(path_of(&ctx), ["$user", "public", "My S"], "T18's parser");
    // Every console that shares the session gets the same context, read once.
    let other = h.clone();
    assert_eq!(other.classifier_context().await, ctx);
    assert_eq!(ctx_reads(&f), 1);
    // A user statement that may change it: the session reads it again before it answers,
    // whichever console asks.
    run(&h, "SET standard_conforming_strings = on")
        .await
        .unwrap();
    let again = other.classifier_context().await;
    assert_eq!(again.lex_modes.as_deref(), Some("on"));
    assert_eq!(path_of(&again), ["app"]);
    assert_eq!(ctx_reads(&f), 2);
}

#[tokio::test]
async fn run_context_read_never_counts_or_runs_in_a_failed_transaction() {
    let fails = FakeExec {
        prefix: "SELECT 1/0",
        result: Err(db("22012")),
        delay: std::time::Duration::ZERO,
        tx_after: Some(hacivat_driver::TxStatus::Failed),
        panics: false,
    };
    let fake = fake_pg().tracks_transactions().exec(vec![
        rows(RUN_CTX_READ, &[&["on", "public"]]),
        fails,
        rows(RUN_CTX_READ, &[&["on", "public"]]),
    ]);
    let (h, f) = session(fake).await;
    tx_cmd(&h, hacivat_session::session::TxCommand::Begin)
        .await
        .unwrap();
    run(&h, "SET search_path = app").await.unwrap();
    let before = snap(&h).tx.statements;
    // M7: Hacivat's own read is not a statement of the user's transaction.
    let ctx = h.classifier_context().await;
    assert_eq!(ctx.lex_modes.as_deref(), Some("on"));
    assert_eq!(
        snap(&h).tx.statements,
        before,
        "the badge's count is unchanged"
    );
    // A failed transaction: nothing is read (it would fail there), the context is unknown.
    let _ = run(&h, "SELECT 1/0").await;
    assert_eq!(snap(&h).tx.status, hacivat_driver::TxStatus::Failed);
    let mark = f.call_log().len();
    let ctx = h.classifier_context().await;
    assert_eq!(ctx.lex_modes, None);
    assert_eq!(ctx.search_path, None);
    assert!(since(&f, mark).is_empty(), "{:?}", since(&f, mark));
}

#[tokio::test]
async fn run_context_read_is_hacivats_own_and_the_same_text_from_the_user_is_guarded() {
    let c = Arc::new(Fake {
        deny_word: Some("SELECT"),
        changes_context: true,
        ..Fake::default()
    });
    let mut script: Vec<FakeExec> = (0..3).map(|_| clean_canary()).collect();
    script.push(rows(RUN_CTX_READ, &[&["on", "app"]]));
    let fake = fake_pg()
        .tracks_transactions()
        .exec(script)
        .session_settings(settings(Some("on"), Some("public")));
    let (h, f, _m) = ro_over(fake, ro_profile(), c.clone()).await;
    exec_user(&h, "SET search_path = app").await.unwrap();
    let asked = c.asked().len();
    let mark = f.call_log().len();
    // Hacivat's own read runs outside the guard: sent, and the classifier is not asked.
    let ctx = h.classifier_context().await;
    assert_eq!(path_of(&ctx), ["app"]);
    assert!(
        since(&f, mark)
            .iter()
            .any(|c| matches!(c, FakeCall::Statement(s) if s.starts_with(RUN_CTX_READ))),
        "{:?}",
        since(&f, mark)
    );
    assert_eq!(c.asked().len(), asked, "{:?}", c.asked());
    // The same text from the user goes through step 0b: asked, refused, never sent.
    let text = format!("{RUN_CTX_READ}, pg_catalog.current_setting('search_path')");
    let mark = f.call_log().len();
    assert!(safety_denied(&exec_user(&h, &text).await));
    assert_eq!(c.asked().last().map(|(s, _)| s.clone()), Some(text.clone()));
    assert!(!sent(&f, mark, &text), "{:?}", since(&f, mark));
}
