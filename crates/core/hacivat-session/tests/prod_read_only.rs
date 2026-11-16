//! M3a Task 18: production consoles were read-only (R-M3-6 REVISED, R-M3-29, P4).
//!
//! ## m3b-t26-prod-unlock: the production policy is gone (R-M3-6 REVISED ends, R-M3-29). Every
//! session of a production-tagged profile is read-write in Manual mode (D-036), on every path
//! this file pinned read-only: the default console, other consoles, pooled and job sessions,
//! single-session borrows, reconnects and a tag applied while a console is open. What kept
//! production safe moves to the confirmations: the app's gate sends a production DML only as
//! `UnitVerdict::TxThenConfirm` (Task 15), and the session keeps nothing of it before the
//! user's answer (Task 19): the statement runs inside the user's transaction under the savepoint
//! `hv_confirm`, the run asks Keep/Undo, and Undo rolls it back. Each test below proves that on
//! the session it used to prove read-only, with the same strength: nothing is committed, and
//! nothing is kept without an answer. (The gate's own production rows are pinned in
//! `hacivat-exec`'s `gate` tests and Docker `docker_prod_unlock`; the app's in `prod_unlock`.)

mod common;

use std::sync::Arc;

use common::confirm::{answer_rollback, nth_commit, q, run_verdict, sent, st};
use common::*;
use hacivat_driver::TxStatus;
use hacivat_driver::fake::{FakeCall, FakeDriver};
use hacivat_net::TlsMode;
use hacivat_proto::{AccessLabel, DbmsKind, Department, EnvTag, ProfileId};
use hacivat_session::budget_for;
use hacivat_session::pipeline::Interaction;
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    ClientId, JobKind, SessionHandle, SessionKind, SessionManager, SessionSpec, TxCommand, TxMode,
    TxOwner, UnitVerdict,
};
use hacivat_session::{ConnectionManager, ManagerConfig, effective_read_only};

/// The app's configuration since Task 26: M2b's (no production policy).
fn app_config() -> ManagerConfig {
    ManagerConfig::m2b()
}

/// A PostgreSQL fake whose reset verify probe answers true (a pooled session is reset when
/// its borrow ends; the verify fails closed on no row), following `BEGIN`/`ROLLBACK`.
fn fake() -> FakeDriver {
    fake_pg().tracks_transactions().exec(
        (0..8)
            .map(|_| rows("SELECT current_user = session_user", &[&["t"]]))
            .collect(),
    )
}

fn tagged(env: EnvTag) -> Profile {
    Profile {
        id: ProfileId::new(),
        environment: Some(env),
        max_connections: 8,
        ..pg_profile(TlsMode::Disable)
    }
}

/// A profile's session manager (no policy to set any more).
fn sessions_of(fake: FakeDriver, profile: Profile) -> (Arc<SessionManager>, Arc<FakeDriver>) {
    let (deps, fake) = deps_with(fake);
    let budget = Arc::new(budget_for(&profile));
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile),
        Arc::new(deps),
        budget,
    );
    (sm, fake)
}

fn spec(kind: SessionKind) -> SessionSpec {
    SessionSpec {
        kind,
        read_only: false,
        department: Department::Console,
    }
}

fn default_of(m: &ConnectionManager, id: ProfileId) -> SessionHandle {
    m.sessions(id).unwrap().default_session().unwrap()
}

const UPDATE: &str = "UPDATE t SET a=1 WHERE id=3";

/// A production session is writable behind its confirmation, by every sign it gives: its
/// snapshot (read-write, Manual, D-036), its label, and what it does with a write the gate
/// sends (`TxThenConfirm`): it begins the user's transaction when none is open, runs the
/// statement under the savepoint `hv_confirm` and asks Keep/Undo **before anything is kept**;
/// Undo rolls the statement back, and nothing is ever committed. (Before Task 26 this helper
/// was `assert_read_only`: the guard refused transaction control before anything was sent.)
async fn assert_confirms(h: &SessionHandle, f: &FakeDriver, what: &str) {
    let s = snap(h);
    assert!(!s.read_only, "{what}: the snapshot");
    assert_eq!(s.access, AccessLabel::ReadWrite, "{what}: the label");
    assert_eq!(
        s.tx.mode,
        TxMode::Manual,
        "{what}: production starts Manual"
    );
    let idle = s.tx.status == TxStatus::Idle;
    let before = f.call_log().len();
    let ran = run_verdict(h, UPDATE, UnitVerdict::TxThenConfirm);
    let asked = nth_commit(&ran.o, 1).await;
    assert!(
        asked.savepoint,
        "{what}: Keep/Undo inside the user's transaction"
    );
    let mut want: Vec<FakeCall> = Vec::new();
    if idle {
        want.push(st("BEGIN"));
    }
    want.extend([st("SAVEPOINT hv_confirm"), q(UPDATE)]);
    assert_eq!(
        sent(f, before),
        want,
        "{what}: nothing kept before the answer"
    );
    assert_eq!(snap(h).tx.owner, TxOwner::User, "{what}");
    answer_rollback(h, &ran).await;
    want.extend([
        st("ROLLBACK TO SAVEPOINT hv_confirm"),
        st("RELEASE SAVEPOINT hv_confirm"),
    ]);
    assert_eq!(
        sent(f, before),
        want,
        "{what}: Undo rolls the statement back"
    );
    assert!(
        !f.executed().iter().any(|s| s.contains("COMMIT")),
        "{what}: never a COMMIT: {:?}",
        f.executed()
    );
}

// ## m3a-t22-typed-readonly (fix round 1, review M4)
// ## m3b-t26-prod-unlock: was `a_production_write_runs_inside_the_guards_read_only_transaction`.
#[tokio::test(start_paused = true)]
async fn a_production_write_is_kept_only_after_its_confirmation() {
    // A production console's write through `Run`, as the gate sends it (`TxThenConfirm`). A
    // plain `DELETE` and one the first keyword does not show (a writable CTE) both run inside
    // the user's transaction under the savepoint and wait for Keep/Undo: nothing is kept before
    // the answer, Undo rolls the statement back, and never a `COMMIT` (before Task 26 the plain
    // one was refused before anything was sent and the CTE ran inside the guard's read-only
    // transaction, then a `ROLLBACK`).
    let profile = Arc::new(tagged(EnvTag::Production));
    let (m, f) = manager_with(fake(), app_config());
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    let h = default_of(&m, profile.id);
    assert!(!snap(&h).read_only);
    for (n, sql) in [
        "DELETE FROM t WHERE id = 3",
        "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
    ]
    .into_iter()
    .enumerate()
    {
        let idle = snap(&h).tx.status == TxStatus::Idle;
        let before = f.call_log().len();
        let ran = run_verdict(&h, sql, UnitVerdict::TxThenConfirm);
        assert!(nth_commit(&ran.o, 1).await.savepoint, "{sql}");
        let mut want: Vec<FakeCall> = Vec::new();
        if idle {
            want.push(st("BEGIN"));
        }
        want.extend([st("SAVEPOINT hv_confirm"), q(sql)]);
        assert_eq!(
            sent(&f, before),
            want,
            "{sql}: nothing kept before the answer"
        );
        answer_rollback(&h, &ran).await;
        want.extend([
            st("ROLLBACK TO SAVEPOINT hv_confirm"),
            st("RELEASE SAVEPOINT hv_confirm"),
        ]);
        assert_eq!(sent(&f, before), want, "{sql}");
        assert_eq!(
            idle,
            n == 0,
            "the first write opened the user's transaction"
        );
    }
    assert!(
        !f.executed().iter().any(|s| s.contains("COMMIT")),
        "{:?}",
        f.executed()
    );
}

// ## m3b-t26-prod-unlock: was `production_session_is_read_only_after_reconnect`.
#[tokio::test(start_paused = true)]
async fn production_session_confirms_after_reconnect() {
    let profile = Arc::new(Profile {
        keepalive_s: Some(5),
        ..tagged(EnvTag::Production)
    });
    let (m, f) = manager_with(fake().pings(vec![Err(io_reset())]), app_config());
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    let h = default_of(&m, profile.id);
    assert_confirms(&h, &f, "before the loss").await;
    // The user ends the transaction the confirmation left open (Manual keeps it).
    tx_cmd(&h, TxCommand::Rollback)
        .await
        .expect("the user's Roll back");
    // The keep-alive finds the connection gone; the session reconnects in place.
    let mut events = h.events();
    tokio::time::advance(secs(6)).await;
    loop {
        let event = next(&mut events).await;
        if event == hacivat_session::session::SessionEvent::Reconnected {
            break;
        }
    }
    assert_eq!(f.connect_calls(), 2);
    // Both connects were read-write, and the session on its new connection still keeps a
    // production write only after its confirmation.
    assert!(
        f.requests().iter().all(|r| !r.read_only),
        "{:?}",
        f.requests()
    );
    assert_confirms(&h, &f, "after the reconnect").await;
}

// ## m3b-t26-prod-unlock: was `production_tag_applies_to_the_next_session` (R-M3-78).
#[tokio::test]
async fn production_tag_gives_the_next_session_manual_mode() {
    let dev = Arc::new(tagged(EnvTag::Dev));
    let (m, f) = manager_with(fake(), app_config());
    m.connect(dev.clone(), None, Interaction::Headless);
    wait_connected(&m, dev.id).await;
    let sm = m.sessions(dev.id).unwrap();
    let before = default_of(&m, dev.id);
    assert!(!snap(&before).read_only);
    assert_eq!(snap(&before).tx.mode, TxMode::Auto);
    // The profile is tagged production while its console is open.
    let prod = Arc::new(Profile {
        environment: Some(EnvTag::Production),
        ..(*dev).clone()
    });
    assert!(
        !effective_read_only(&prod),
        "the tag alone never makes it read-only"
    );
    m.update_profile(dev.id, prod);
    // Every session opened from now on starts Manual (D-036) and keeps a write only after its
    // confirmation, whatever kind it is.
    let console = sm
        .open(spec(SessionKind::Console(ClientId(2))), ix_none())
        .await
        .unwrap();
    assert_confirms(&console, &f, "a console opened after the tag").await;
    let pooled = sm.pooled(SessionKind::Service).await.unwrap();
    assert_confirms(&pooled, &f, "a pooled session after the tag").await;
    // The session that was open keeps the mode it was opened with (R-M3-78: no forced
    // reconnect, no mode change mid-session); the app's gate reads the live tag on its next
    // Run and confirms from then on (`hacivat-app` `prod_unlock`).
    assert!(!snap(&before).read_only);
    assert_eq!(snap(&before).tx.mode, TxMode::Auto);
}

// ## m3b-t26-prod-unlock: was `single_session_production_is_read_only`.
#[tokio::test]
async fn single_session_production_confirms() {
    let profile = Profile {
        single_session: true,
        ..tagged(EnvTag::Production)
    };
    let (sm, f) = sessions_of(fake(), profile);
    let console = sm
        .open(spec(SessionKind::Console(ClientId::DEFAULT)), ix_none())
        .await
        .unwrap();
    assert_confirms(&console, &f, "the single session").await;
    // Every other client shares it.
    let other = sm
        .open(spec(SessionKind::Console(ClientId(2))), ix_none())
        .await
        .unwrap();
    assert_eq!(other.id(), console.id());
    assert_eq!(f.connect_calls(), 1);
    assert!(f.requests().iter().all(|r| !r.read_only));
    assert_confirms(&other, &f, "the shared session").await;
}

// ## m3b-t26-prod-unlock: was `every_session_kind_on_a_production_profile_is_read_only`.
#[tokio::test]
async fn every_session_kind_on_a_production_profile_confirms() {
    let kinds = [
        SessionKind::Console(ClientId::DEFAULT),
        SessionKind::Console(ClientId(2)),
        SessionKind::Editor("public.t".into()),
        SessionKind::Introspection { database: None },
        SessionKind::Service,
        SessionKind::Job(1, JobKind::Export),
    ];
    let (sm, f) = sessions_of(fake(), tagged(EnvTag::Production));
    let mut open = Vec::new();
    for kind in kinds.clone() {
        let h = sm.open(spec(kind.clone()), ix_none()).await.unwrap();
        assert_confirms(&h, &f, &format!("opened {kind:?}")).await;
        open.push(h);
    }
    // And borrowed from the pool.
    let borrowed = sm.pooled(SessionKind::Service).await.unwrap();
    assert_confirms(&borrowed, &f, "a pooled borrow").await;
    // Every connection was asked for read-write (no driver read-only layer).
    assert_eq!(f.connect_calls(), kinds.len() + 1);
    assert!(f.requests().iter().all(|r| !r.read_only));

    // The same profile without the tag starts Auto: the tag is what makes it Manual.
    let (sm, f) = sessions_of(fake(), tagged(EnvTag::Staging));
    let h = sm
        .open(spec(SessionKind::Console(ClientId::DEFAULT)), ix_none())
        .await
        .unwrap();
    assert!(!snap(&h).read_only);
    assert_eq!(snap(&h).tx.mode, TxMode::Auto);
    run(&h, "COMMIT").await.unwrap();
    assert!(f.executed().iter().any(|s| s == "COMMIT"));
    // And a production profile that is read-only by its own flag stays read-only.
    let (sm, f) = sessions_of(
        fake(),
        Profile {
            read_only: true,
            ..tagged(EnvTag::Production)
        },
    );
    let h = sm
        .open(spec(SessionKind::Console(ClientId::DEFAULT)), ix_none())
        .await
        .unwrap();
    assert!(snap(&h).read_only);
    assert!(f.requests().iter().all(|r| r.read_only));
}

#[tokio::test]
async fn pooled_borrow_on_production_reuses_its_session() {
    // P4: the pool keys its idle sessions by the effective read-only flag, so a production
    // borrow finds the session it parked instead of opening one per borrow.
    // ## m3b-t26-prod-unlock: that session is read-write now.
    let (sm, f) = sessions_of(fake(), tagged(EnvTag::Production));
    let first = sm.pooled(SessionKind::Service).await.unwrap();
    let id = first.id();
    assert!(!snap(&first).read_only);
    drop(first);
    settle_until("the release resets the session", || f.reset_calls() >= 1).await;
    settle().await;
    let second = sm.pooled(SessionKind::Service).await.unwrap();
    assert_eq!(second.id(), id);
    assert!(!snap(&second).read_only);
    assert_eq!(f.connect_calls(), 1);
    assert_eq!(f.open_connections(), 1);
    // Many borrows in a row never grow the pool.
    drop(second);
    for n in 2..6 {
        settle_until("the release resets the session", || f.reset_calls() >= n).await;
        settle().await;
        drop(sm.pooled(SessionKind::Service).await.unwrap());
    }
    assert_eq!(f.connect_calls(), 1);
}

#[tokio::test]
async fn single_session_production_borrow_succeeds() {
    // P4: in single-session mode a borrow shares the default session; the borrow must find
    // it, not be refused. ## m3b-t26-prod-unlock: that session is read-write now (Manual).
    for kind in [DbmsKind::Postgres, DbmsKind::Sqlite] {
        let profile = match kind {
            DbmsKind::Sqlite => Profile {
                environment: Some(EnvTag::Production),
                ..sqlite_profile()
            },
            _ => Profile {
                single_session: true,
                ..tagged(EnvTag::Production)
            },
        };
        assert!(!profile.read_only);
        let (sm, f) = sessions_of(FakeDriver::new(kind), profile);
        let borrowed = sm.pooled(SessionKind::Service).await.unwrap();
        assert!(!snap(&borrowed).read_only, "{kind:?}");
        assert_eq!(snap(&borrowed).tx.mode, TxMode::Manual, "{kind:?}");
        let again = sm
            .pooled(SessionKind::Introspection { database: None })
            .await
            .unwrap();
        assert_eq!(again.id(), borrowed.id(), "{kind:?}");
        let console = sm
            .open(spec(SessionKind::Console(ClientId::DEFAULT)), ix_none())
            .await
            .unwrap();
        assert_eq!(console.id(), borrowed.id(), "{kind:?}");
        assert_eq!(f.connect_calls(), 1, "{kind:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// ## m3b-t26-prod-unlock (T26 carry MUST 3, the T12 ruling): a production profile's startup
// script runs only statements the classifier rates harmless (a read, a harmless `SET`); one
// that is not refuses the session with `config.invalid` and sends none of the script, and the
// warning names the statement's number, never its text.

/// A classifier that allows everything but a statement starting with `UPDATE` (a write), and
/// records what it was asked.
#[derive(Debug, Default)]
struct ScriptCheck {
    asked: std::sync::Mutex<Vec<String>>,
}

impl hacivat_session::session::StatementClassifier for ScriptCheck {
    fn check_read_only(
        &self,
        _: DbmsKind,
        _: Arc<str>,
        _: Arc<[Arc<str>]>,
    ) -> futures::future::BoxFuture<'static, hacivat_session::session::RoCheck> {
        panic!("a startup script is checked in the session's context");
    }

    fn effects(
        &self,
        _: DbmsKind,
        _: Arc<str>,
    ) -> futures::future::BoxFuture<'static, Option<hacivat_session::session::SessionEffects>> {
        Box::pin(async { None })
    }

    fn check_read_only_in(
        &self,
        _: DbmsKind,
        sql: Arc<str>,
        _: Arc<[Arc<str>]>,
        _: hacivat_session::session::ClassifierContext,
    ) -> futures::future::BoxFuture<'static, hacivat_session::session::RoAnswer> {
        self.asked.lock().unwrap().push(sql.to_string());
        let check = if sql.trim_start().to_ascii_uppercase().starts_with("UPDATE") {
            hacivat_session::session::RoCheck::Deny(hacivat_session::session::SafetyDenial {
                class_id: "safety-class-write",
                reasons: vec!["safety-reason-write-data"],
            })
        } else {
            hacivat_session::session::RoCheck::Allow
        };
        Box::pin(async move {
            hacivat_session::session::RoAnswer {
                check,
                changes_context: false,
            }
        })
    }
}

fn scripted(env: EnvTag, script: &str) -> Arc<Profile> {
    Arc::new(Profile {
        startup_script: Some(script.to_string()),
        ..tagged(env)
    })
}

/// Waits until `id` is connected, or its connect ended in `config.invalid`: which.
async fn settled_class(m: &ConnectionManager, id: ProfileId) -> Option<hacivat_proto::ErrClass> {
    use hacivat_session::LiveState;
    let mut changes = m.changes();
    loop {
        match m.state(id) {
            LiveState::Connected(_) => return None,
            LiveState::Failed(e) => return Some(e.class),
            LiveState::Lost { class } => return Some(class),
            _ => {}
        }
        tokio::time::timeout(secs(30), changes.changed())
            .await
            .expect("the connect settles")
            .expect("the manager lives");
    }
}

#[tokio::test(start_paused = true)]
async fn production_startup_script_runs_only_harmless_statements() {
    let check = Arc::new(ScriptCheck::default());
    let config = ManagerConfig {
        classifier: Some(check.clone()),
        ..app_config()
    };
    // Harmless: every statement asked, every one sent.
    let (m, f) = manager_with(fake(), config.clone());
    let ok = scripted(EnvTag::Production, "SET work_mem = '64MB'; SELECT 1");
    m.connect(ok.clone(), None, Interaction::Headless);
    assert_eq!(settled_class(&m, ok.id).await, None, "connected");
    assert_eq!(
        *check.asked.lock().unwrap(),
        ["SET work_mem = '64MB'", "SELECT 1"]
    );
    assert!(f.executed().iter().any(|s| s == "SET work_mem = '64MB'"));

    // A write in it: the session is refused (`config.invalid`), nothing of the script is sent,
    // and the warning names statement 2 without its text.
    check.asked.lock().unwrap().clear();
    let logs = capture_tracing(|| async {
        let (m, f) = manager_with(fake(), config.clone());
        let bad = scripted(
            EnvTag::Production,
            "SET work_mem = '64MB'; UPDATE hv_secret_t SET a = 1; SET x = 1",
        );
        m.connect(bad.clone(), None, Interaction::Headless);
        assert_eq!(
            settled_class(&m, bad.id).await,
            Some(hacivat_proto::ErrClass::ConfigInvalid)
        );
        assert!(
            !f.executed()
                .iter()
                .any(|s| s.contains("work_mem") || s.contains("hv_secret_t")),
            "none of the script was sent: {:?}",
            f.executed()
        );
    })
    .await;
    assert!(
        logs.contains("statement 2 of the profile's startup script"),
        "{logs}"
    );
    assert!(!logs.contains("hv_secret_t"), "the text is never logged");

    // The same script off production runs unchecked (the control: the tag decides).
    check.asked.lock().unwrap().clear();
    let (m, f) = manager_with(fake(), config);
    let staging = scripted(EnvTag::Staging, "UPDATE t SET a = 1");
    m.connect(staging.clone(), None, Interaction::Headless);
    assert_eq!(settled_class(&m, staging.id).await, None);
    assert!(check.asked.lock().unwrap().is_empty());
    assert!(f.executed().iter().any(|s| s == "UPDATE t SET a = 1"));
}

#[tokio::test(start_paused = true)]
async fn production_startup_script_without_a_classifier_never_runs() {
    // Fail closed: nothing can check it, so nothing of it is sent and the session is refused.
    let (m, f) = manager_with(fake(), app_config());
    let p = scripted(EnvTag::Production, "SET work_mem = '64MB'");
    m.connect(p.clone(), None, Interaction::Headless);
    assert_eq!(
        settled_class(&m, p.id).await,
        Some(hacivat_proto::ErrClass::ConfigInvalid)
    );
    assert!(!f.executed().iter().any(|s| s.contains("work_mem")));
}

#[tokio::test(start_paused = true)]
async fn production_startup_script_is_checked_again_on_reset() {
    // The reset replays the environment through the same check: a harmless script runs again.
    let check = Arc::new(ScriptCheck::default());
    let config = ManagerConfig {
        classifier: Some(check.clone()),
        ..app_config()
    };
    let (sm, f) = {
        let (deps, f) = deps_with(fake());
        let profile = scripted(EnvTag::Production, "SET work_mem = '64MB'");
        let budget = Arc::new(budget_for(&profile));
        let sm = SessionManager::new(
            tokio::runtime::Handle::current(),
            profile,
            Arc::new(deps),
            budget,
        );
        sm.set_classifier(config.classifier.clone());
        (sm, f)
    };
    let pooled = sm.pooled(SessionKind::Service).await.unwrap();
    // The actor sends its after-connect statements before it reads any request.
    let _ = pooled
        .submit(hacivat_session::session::Request::Ping)
        .reply
        .await;
    assert_eq!(check.asked.lock().unwrap().len(), 1, "checked at connect");
    drop(pooled);
    settle_until("the release resets the session", || f.reset_calls() >= 1).await;
    settle().await;
    let again = sm.pooled(SessionKind::Service).await.unwrap();
    assert!(!snap(&again).read_only);
    assert_eq!(
        check.asked.lock().unwrap().len(),
        2,
        "checked again at the reset"
    );
    assert_eq!(
        f.executed()
            .iter()
            .filter(|s| *s == "SET work_mem = '64MB'")
            .count(),
        2
    );
}

// ## m3b-t26-prod-unlock (fix round 1, A-I1): the live tag. A profile tagged production while
// its sessions are open: the idle pooled ones close, a borrowed one (a Count rows borrow, the
// run service's `SessionKind::Service`) checks the script its reset replays and is refused,
// and a new borrow is checked at its connect. Nothing of the write-y script is sent again.
#[tokio::test(start_paused = true)]
async fn a_retag_to_production_checks_the_scripts_of_sessions_opened_before_it() {
    const WRITE: &str = "UPDATE hv_t SET a = 1";
    let check = Arc::new(ScriptCheck::default());
    let (deps, f) = deps_with(fake());
    let dev = scripted(EnvTag::Dev, WRITE);
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        dev.clone(),
        Arc::new(deps),
        Arc::new(budget_for(&dev)),
    );
    sm.set_classifier(Some(check.clone()));
    let sent = |f: &FakeDriver| f.executed().iter().filter(|s| *s == WRITE).count();
    // Off production the script runs unchecked: two borrows (one busy, one released and
    // parked after its reset).
    let busy = sm.pooled(SessionKind::Service).await.unwrap();
    let parked = sm.pooled(SessionKind::Service).await.unwrap();
    for h in [&busy, &parked] {
        let _ = h
            .submit(hacivat_session::session::Request::Ping)
            .reply
            .await;
    }
    drop(parked);
    settle_until("the released session is parked", || sm.parked_count() == 1).await;
    assert_eq!(sent(&f), 3, "connect ×2 and the release's reset");
    assert!(
        check.asked.lock().unwrap().is_empty(),
        "unchecked off production"
    );

    // The retag: the parked session closes at once; none is handed out again.
    sm.update_profile(Arc::new(Profile {
        environment: Some(EnvTag::Production),
        ..(*dev).clone()
    }));
    assert_eq!(sm.parked_count(), 0, "the idle sessions close on the retag");

    // The busy borrow ends: its reset replays the script it opened with, now checked under the
    // live tag, and refused; nothing of it is sent, and the session is not parked.
    let calls = f.call_log().len();
    drop(busy);
    settle_until("the release's reset ran", || f.reset_calls() >= 2).await;
    settle().await;
    assert_eq!(
        *check.asked.lock().unwrap(),
        [WRITE],
        "checked at the reset"
    );
    assert_eq!(sent(&f), 3, "nothing of the script sent at the reset");
    assert!(
        !f.call_log()[calls..]
            .iter()
            .any(|c| format!("{c:?}").contains("hv_t")),
        "nothing of the script in the whole call log: {:?}",
        &f.call_log()[calls..]
    );
    assert_eq!(sm.parked_count(), 0, "the refused session is not parked");

    // A new borrow connects under the production profile: checked, refused, nothing sent.
    let again = sm.pooled(SessionKind::Service).await;
    settle().await;
    assert_eq!(sent(&f), 3, "nothing sent by a new borrow: {again:?}");
    assert_eq!(
        check.asked.lock().unwrap().len(),
        2,
        "checked at the connect"
    );
}
