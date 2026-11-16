//! M3b Task 1c: the session's M3b seams. Out-of-band answers reach the actor as commands that
//! never queue behind the running request (P-M3b-3); units that need Task 19 fail closed with
//! nothing sent; the classifier plumbing changes no M3a behaviour (P-M3b-11); one ownership
//! rule for Hacivat's transactions (P-M3b-12); the FakeDriver's binds, notices and warnings;
//! the profile's new fields (P-M3b-10).

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::runs::{Recorder, generated, select, spec_of};
use common::*;
use futures::future::BoxFuture;
use hacivat_driver::fake::{FakeCall, FakeDriver, FakeQuery, FakeTypedCall};
use hacivat_driver::{
    Driver, DriverConnect, Endpoint, InsecureAuth, Notice, NoticeSeverity, Protocol, QueryEvent,
    QuerySpec, RawConn,
};
use hacivat_proto::DbmsKind;
use hacivat_session::profile::{
    Profile, ProfileDiag, ProfileStore, valid_fn_allowlist_entry, valid_startup_script,
};
use hacivat_session::session::{
    CommitAnswer, ErrorPolicy, ExecErrorKind, RoCheck, RunChoice, RunEvent, RunId, RunSpec,
    RunUnit, SafetyDenial, SessionEffects, SessionState, StatementClassifier, TxCommand, TxMode,
    TxOwner, UnitEnd, UnitId, UnitVerdict,
};
use hacivat_session::{ManagerConfig, profile::ProfileEndpoint};
use hacivat_values::{BindValue, LogicalType};

// ---------------------------------------------------------------------------------------------
// Fixtures

/// A run of `sql` whose one unit carries `verdict`.
fn spec_with_verdict(o: &Arc<Recorder>, sql: &str, verdict: UnitVerdict) -> RunSpec {
    let mut spec = spec_of(o, &[sql]);
    let mut units: Vec<RunUnit> = spec.units.to_vec();
    units[0].verdict = verdict;
    spec.units = units.into();
    spec
}

/// The fake's calls from `mark` on.
fn calls_since(f: &FakeDriver, mark: usize) -> Vec<FakeCall> {
    f.call_log()[mark..].to_vec()
}

/// A connection straight from the fake driver (no session, no pipeline).
async fn connect_fake(f: &FakeDriver) -> Box<dyn RawConn> {
    let req = DriverConnect {
        endpoint: Endpoint::File {
            path: PathBuf::from("hv-test.db"),
            create_if_missing: false,
            read_only: false,
        },
        tls: None,
        user: Some("hv_test".into()),
        password: None,
        database: None,
        application_name: "Hacivat".into(),
        read_only: false,
        options: Vec::new(),
        connect_timeout: Duration::from_secs(15),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::default(),
    };
    f.connect(req).await.expect("the fake connects")
}

fn query_spec<'a>(sql: &'a str, binds: &'a [BindValue]) -> QuerySpec<'a> {
    QuerySpec {
        sql,
        binds,
        protocol: Protocol::Extended,
        suspendable: false,
        first_rows: 501,
        chunk_rows: 4_096,
        chunk_bytes: 4 << 20,
        cell_cap: 1 << 20,
        max_result_sets: 1,
    }
}

/// A classifier that denies everything and counts its calls.
#[derive(Debug, Default)]
struct DenyAll {
    calls: Mutex<u32>,
}

impl StatementClassifier for DenyAll {
    fn check_read_only(
        &self,
        _kind: DbmsKind,
        _sql: Arc<str>,
        _allow: Arc<[Arc<str>]>,
    ) -> BoxFuture<'static, RoCheck> {
        *self.calls.lock().unwrap() += 1;
        Box::pin(async {
            RoCheck::Deny(SafetyDenial {
                class_id: "safety-class-unknown",
                reasons: vec!["safety-reason-not-implemented"],
            })
        })
    }

    fn effects(
        &self,
        _kind: DbmsKind,
        _sql: Arc<str>,
    ) -> BoxFuture<'static, Option<SessionEffects>> {
        Box::pin(async { Some(SessionEffects::all()) })
    }
}

// ---------------------------------------------------------------------------------------------
// Out-of-band answers (P-M3b-3)

#[tokio::test]
async fn choose_without_a_waiting_run_answers_false() {
    let (h, _) = session(fake_pg()).await;
    assert!(!h.choose(7, 1, RunChoice::Ignore).await.unwrap());
    let unit = UnitId {
        run: RunId(1),
        ordinal: 0,
    };
    assert!(
        !h.commit_decision(7, unit, 1, CommitAnswer::Commit)
            .await
            .unwrap()
    );
}

#[tokio::test(start_paused = true)]
async fn choose_is_served_while_a_request_runs() {
    // The query's first event waits 30 s of (paused) time: the run holds the session.
    let held = FakeQuery {
        first_delay: secs(30),
        ..select("SELECT", 10)
    };
    let (h, _f) = session(fake_pg().tracks_transactions().queries(vec![held])).await;
    let o = Recorder::new();
    let mut spec = spec_of(&o, &["SELECT * FROM t"]);
    spec.policy = ErrorPolicy::Ask;
    let t = h.run(spec);
    settle_until(
        "the run holds the session",
        || matches!(snap(&h).state, SessionState::Busy { req, .. } if req == t.seq),
    )
    .await;
    let start = tokio::time::Instant::now();
    // Answered by `Ladder::command` while the run is in flight, not after it (a queued
    // request would wait the 30 s): no run waits for a choice yet (Task 11), so `false`.
    assert!(!h.choose(t.seq + 1, 1, RunChoice::Ignore).await.unwrap());
    assert!(!h.choose(t.seq, 1, RunChoice::Stop).await.unwrap());
    let unit = UnitId {
        run: RunId(1),
        ordinal: 0,
    };
    assert!(
        !h.commit_decision(t.seq, unit, 1, CommitAnswer::RollBack)
            .await
            .unwrap()
    );
    assert_eq!(tokio::time::Instant::now(), start, "no time passed");
    assert!(
        matches!(snap(&h).state, SessionState::Busy { req, .. } if req == t.seq),
        "the run still holds the session"
    );
    t.reply.await.unwrap().unwrap();
}

#[tokio::test]
async fn choose_on_a_closed_session_answers_false() {
    let (h, _) = session(fake_pg()).await;
    h.close(hacivat_session::session::ClosePolicy::RollBack)
        .await
        .unwrap();
    settle_until("the actor ends", || {
        snap(&h).state == SessionState::Disconnected
    })
    .await;
    assert!(!h.choose(1, 1, RunChoice::Stop).await.unwrap_or(false));
}

// ---------------------------------------------------------------------------------------------
// Fail-closed units (R-M3-52)

#[tokio::test]
async fn tx_then_confirm_unit_fails_closed_until_task_19() {
    for verdict in [
        UnitVerdict::TxThenConfirm,
        UnitVerdict::ExplainAnalyzeRollback,
    ] {
        let (h, f) = session(fake_pg()).await;
        let before = f.call_log().len();
        let o = Recorder::new();
        let t = h.run(spec_with_verdict(&o, "UPDATE t SET a=1", verdict));
        let e = t.reply.await.unwrap().unwrap_err();
        assert_eq!(e.kind, ExecErrorKind::Internal, "{verdict:?}");
        assert!(
            matches!(&o.unit_ends()[..], [UnitEnd::Failed(e)] if e.kind == ExecErrorKind::Internal),
            "{verdict:?}: {:?}",
            o.unit_ends()
        );
        assert!(
            calls_since(&f, before).is_empty(),
            "{verdict:?}: nothing sent"
        );
    }
}

#[tokio::test]
async fn refused_units_send_nothing_in_manual_mode_either() {
    // Manual mode would send its `BEGIN` first: the refusal comes before it.
    let (h, f) = session(fake_pg().tracks_transactions()).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    let before = f.call_log().len();
    let o = Recorder::new();
    let t = h.run(spec_with_verdict(
        &o,
        "UPDATE t SET a=1",
        UnitVerdict::TxThenConfirm,
    ));
    assert!(t.reply.await.unwrap().is_err());
    // ## m3b-t11-session-runs (named exception): a unit with binds runs since Task 11
    // (`tests/run_m3b.rs`); this test keeps the Task 19 refusal.
    assert!(calls_since(&f, before).is_empty());
    assert_eq!(snap(&h).tx.status, hacivat_driver::TxStatus::Idle);
}

#[tokio::test]
async fn read_and_write_verdicts_run_as_in_m3a() {
    // Two result sets scripted. ## m3b-t11-session-runs (named exception): since Task 11 a unit
    // keeps `max_result_sets` of them (R-M3-66), each its own result.
    let two_sets = FakeQuery {
        prefix: "SELECT 1",
        sets: vec![generated(1), generated(1)],
        ..FakeQuery::default()
    };
    let (h, f) = session(fake_pg().queries(vec![two_sets])).await;
    let o = Recorder::new();
    let mut spec = spec_with_verdict(&o, "SELECT 1", UnitVerdict::Read);
    let mut units: Vec<RunUnit> = spec.units.to_vec();
    units[0].suspendable = false;
    units[0].max_result_sets = 100;
    units[0].effects = SessionEffects::SESSION_STATE;
    spec.units = units.into();
    h.run(spec).reply.await.unwrap().unwrap();
    assert_eq!(f.typed_log(), vec![FakeTypedCall::Query("SELECT 1".into())]);
    assert_eq!(o.stores(), 2);
    assert!(
        !o.events()
            .iter()
            .any(|e| matches!(e, RunEvent::MoreResultSets { .. }))
    );
}

// ---------------------------------------------------------------------------------------------
// Classifier plumbing (R-M3-83, P-M3b-11)

#[tokio::test]
async fn manager_default_has_no_classifier() {
    assert!(ManagerConfig::default().classifier.is_none());
    assert!(ManagerConfig::m2b().classifier.is_none());
    assert_eq!(ManagerConfig::default(), ManagerConfig::default());
    let c: Arc<dyn StatementClassifier> = Arc::new(DenyAll::default());
    let with = ManagerConfig {
        classifier: Some(c.clone()),
        ..ManagerConfig::default()
    };
    assert_ne!(with, ManagerConfig::default());
    assert_eq!(
        with,
        ManagerConfig {
            classifier: Some(c),
            ..ManagerConfig::default()
        },
        "the same classifier compares equal"
    );
}

#[tokio::test]
async fn an_installed_classifier_changes_nothing_until_task_18() {
    // `ManagerConfig::classifier` reaches the sessions the manager starts; step 0b is a stub,
    // so a read-only session with a denying classifier still runs its reads through the guard
    // (typed and M2b paths) and never asks the classifier.
    let deny = Arc::new(DenyAll::default());
    let config = ManagerConfig {
        classifier: Some(deny.clone()),
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
    let o = Recorder::new();
    let reply = h.run(spec_of(&o, &["SELECT 1"])).reply.await.unwrap();
    assert!(reply.is_ok(), "{reply:?}");
    assert!(
        f.typed_log()
            .contains(&FakeTypedCall::Query("SELECT 1".into()))
    );
    run(&h, "SELECT 2").await.unwrap();
    assert!(f.executed().iter().any(|s| s == "SELECT 2"));
    assert_eq!(*deny.calls.lock().unwrap(), 0);
}

// ---------------------------------------------------------------------------------------------
// Hacivat's transactions (P-M3b-12)

#[test]
fn hacivats_owners() {
    assert!(
        TxOwner::Confirm.is_hacivats()
            && TxOwner::Cursor.is_hacivats()
            && TxOwner::Guard.is_hacivats()
    );
    assert!(!TxOwner::User.is_hacivats() && !TxOwner::None.is_hacivats());
}

// ---------------------------------------------------------------------------------------------
// FakeDriver: binds, notices, warnings

#[tokio::test]
async fn fake_driver_logs_binds_after_the_query() {
    let f = FakeDriver::new(DbmsKind::Postgres);
    let mut c = connect_fake(&f).await;
    let binds = [
        BindValue::Null,
        BindValue::Typed {
            hint: LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            text: "müşteri".into(),
        },
    ];
    c.query(query_spec("SELECT $1, $2", &binds)).await.unwrap();
    while !matches!(c.next_event().await.unwrap(), QueryEvent::Done(_)) {}
    c.query(query_spec("SELECT 1", &[])).await.unwrap();
    assert_eq!(
        f.typed_log(),
        vec![
            FakeTypedCall::Query("SELECT $1, $2".into()),
            FakeTypedCall::Binds(binds.to_vec()),
            FakeTypedCall::Query("SELECT 1".into()),
        ],
        "a query without binds logs as in M3a"
    );
    assert_eq!(f.binds_seen(), vec![binds.to_vec(), Vec::new()]);
}

#[tokio::test]
async fn fake_driver_emits_scripted_notices() {
    let notice = Notice::new(
        NoticeSeverity::Notice,
        Some("00000"),
        "tablo yok, atlanıyor",
    );
    let warning = Notice::new(NoticeSeverity::Warning, Some("1265"), "Data truncated");
    let f = FakeDriver::new(DbmsKind::Postgres).queries(vec![FakeQuery {
        prefix: "DO",
        notices: vec![notice.clone()],
        warnings: vec![warning.clone()],
        ..FakeQuery::default()
    }]);
    let mut c = connect_fake(&f).await;
    assert_eq!(
        c.warnings().await.unwrap(),
        Vec::new(),
        "none before a query"
    );
    c.query(query_spec("DO $$ BEGIN RAISE NOTICE 'x'; END $$", &[]))
        .await
        .unwrap();
    assert_eq!(
        c.next_event().await.unwrap(),
        QueryEvent::Notice(notice.clone())
    );
    assert!(matches!(
        c.next_event().await.unwrap(),
        QueryEvent::ResultStart(_)
    ));
    let done = loop {
        if let QueryEvent::Done(d) = c.next_event().await.unwrap() {
            break d;
        }
    };
    assert_eq!(done.warnings, 1);
    assert_eq!(c.warnings().await.unwrap(), vec![warning]);
    assert_eq!(c.warnings().await.unwrap(), Vec::new(), "taken once");

    // Through a session: the notice reaches the run's observer.
    let (h, _f) = session(fake_pg().tracks_transactions().queries(vec![FakeQuery {
        prefix: "DO",
        notices: vec![notice.clone()],
        ..FakeQuery::default()
    }]))
    .await;
    let o = Recorder::new();
    h.run(spec_of(&o, &["DO $$ BEGIN NULL; END $$"]))
        .reply
        .await
        .unwrap()
        .unwrap();
    assert!(
        o.events()
            .iter()
            .any(|e| matches!(e, RunEvent::Notice { notice: n, .. } if *n == notice))
    );
}

#[test]
fn notice_text_is_capped_at_a_char_boundary() {
    let long = "ş".repeat(10_000); // 20,000 bytes
    let n = Notice::new(NoticeSeverity::Warning, None, &long);
    assert!(n.text.len() <= Notice::MAX_TEXT_BYTES);
    assert_eq!(
        n.text.len(),
        Notice::MAX_TEXT_BYTES,
        "16 KiB is an even number of 2-byte chars"
    );
    let odd = format!("a{long}");
    let n = Notice::new(NoticeSeverity::Info, None, &odd);
    assert_eq!(
        n.text.len(),
        Notice::MAX_TEXT_BYTES - 1,
        "never inside a character"
    );
}

// ---------------------------------------------------------------------------------------------
// Profile fields (P-M3b-10)

/// An M2b `connections.toml`, as an M2b build writes it.
const M2B_FILE: &str = "version = 1\n\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"orders\"\nengine = \"postgres\"\nread_only = false\nhost = \"127.0.0.1\"\nretention = \"forever\"\ntls.mode = \"verify_full\"\ntransport = \"direct\"\nconnect_timeout_s = 15\napplication_name = \"Hacivat\"\nmax_connections = 4\norigin = \"user_typed\"\nallow_unsupported_server = false\ntx_mode = \"manual\"\n";

fn tmp_file(text: &str) -> PathBuf {
    let path = tempfile::tempdir().unwrap().keep().join("connections.toml");
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn m2b_profile_file_loads_with_new_defaults() {
    let path = tmp_file(M2B_FILE);
    let (store, loaded) = ProfileStore::open(path.clone());
    assert!(loaded.diags.is_empty(), "{:?}", loaded.diags);
    let p = &loaded.profiles[0];
    assert!(p.record_history);
    assert_eq!(p.startup_script, None);
    assert!(p.read_only_fn_allowlist.is_empty());
    // It round-trips byte for byte: no M3b key is written at its default.
    let before = std::fs::read_to_string(&path).unwrap();
    store.save(&loaded.profiles).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

    let mut q: Profile = p.clone();
    q.startup_script = Some("SET search_path = app, public".into());
    q.read_only_fn_allowlist = vec!["audit.mask_email".into(), "müşteri_adı".into()];
    q.record_history = false;
    store.save(std::slice::from_ref(&q)).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("record_history = false"), "{text}");
    let (_, back) = ProfileStore::open(path);
    assert!(back.diags.is_empty(), "{:?}", back.diags);
    assert_eq!(back.profiles, vec![q]);
}

#[test]
fn invalid_profile_fields_make_the_entry_invalid() {
    for (line, key) in [
        ("startup_script = 3\n", "startup_script"),
        ("read_only_fn_allowlist = \"f\"\n", "read_only_fn_allowlist"),
        (
            "read_only_fn_allowlist = [\"f; DROP\"]\n",
            "read_only_fn_allowlist",
        ),
        ("record_history = \"no\"\n", "record_history"),
    ] {
        let (_, loaded) = ProfileStore::open(tmp_file(&format!("{M2B_FILE}{line}")));
        assert!(loaded.profiles.is_empty(), "{line}");
        assert!(
            matches!(&loaded.diags[..], [ProfileDiag::InvalidEntry { reason, .. }] if reason.contains(key)),
            "{line}: {:?}",
            loaded.diags
        );
    }
}

#[test]
fn allowlist_and_script_rules() {
    assert!(valid_fn_allowlist_entry("audit.mask_email"));
    assert!(valid_fn_allowlist_entry("İŞLEM_özet"));
    for bad in ["", "f()", "a b", "x;y", "a,b", "o'neil", "a\u{0}b"] {
        assert!(!valid_fn_allowlist_entry(bad), "{bad:?}");
    }
    assert!(!valid_fn_allowlist_entry(&"f".repeat(129)));
    assert!(valid_startup_script("SET search_path = app"));
    assert!(!valid_startup_script("SET a = 1;\0"));
    assert!(!valid_startup_script(&"x".repeat(64 * 1024 + 1)));
    let p = Profile {
        endpoint: ProfileEndpoint::Server {
            host: "127.0.0.1".into(),
            port: None,
        },
        ..Profile::default()
    };
    assert!(p.record_history);
}

#[test]
fn run_spec_deadline_is_five_minutes() {
    assert_eq!(RunSpec::CONFIRM_DEADLINE, Duration::from_secs(300));
}
