//! The connect pipeline and test connection over `FakeDriver` and a scripted `UiPort`
//! (HV-CONN-01, -07, P8, P9, P11, P23, P43, R3, R8, R10, R11, R25). No Docker, no socket.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use hacivat_driver::EngineCatalog;
use hacivat_driver::Support;
use hacivat_driver::fake::FakeDriver;
use hacivat_net::TlsMode;
use hacivat_proto::{AccessLabel, DbmsKind, ErrClass, FixId};
use hacivat_session::TestWarning;
use hacivat_session::pipeline::{
    Interaction, IssueCode, Phase, StepOutcome, TrailStep::*, connect, validate,
};

#[tokio::test]
async fn trail_orders() {
    assert_eq!(
        steps(connect_ok(pg_profile(TlsMode::Auto)).await),
        [Resolve, Connect, Tls, SignIn, CheckServer]
    );
    // The mock omits TLS when disabled (P43).
    assert_eq!(
        steps(connect_ok(pg_profile(TlsMode::Disable)).await),
        [Resolve, Connect, SignIn, CheckServer]
    );
    assert_eq!(steps(connect_ok(pgpass_profile()).await)[0], PgPass);
    assert_eq!(
        steps(connect_ok(sqlite_profile()).await),
        [OpenFile, CheckServer]
    );
}

#[tokio::test]
async fn a_successful_trail_is_done_and_timed_where_the_pipeline_saw_it() {
    let live = connect_ok(pgpass_profile()).await;
    for t in &live.trail {
        // Auto toward a loopback server that answered in plaintext: TLS was skipped.
        let want = if t.step == Tls {
            StepOutcome::Skipped
        } else {
            StepOutcome::Done
        };
        assert_eq!(t.outcome, want, "{:?}", t.step);
    }
    let timed = |s| live.trail.iter().find(|t| t.step == s).unwrap().elapsed;
    // The pipeline ran these itself (the factory's first stream is the Connect step, P23).
    for s in [PgPass, Resolve, Connect, SignIn, CheckServer] {
        assert!(timed(s).is_some(), "{s:?}");
    }
    assert!(
        !live.forwarded(),
        "a stream-injecting driver needs no forwarder"
    );
}

#[tokio::test]
async fn a_driver_that_dials_itself_gets_the_direct_target_and_an_untimed_connect() {
    // MySQL Direct (P23): no stream is opened here, the Connect step has no duration.
    let (deps, fake) = deps_dialing_itself(FakeDriver::new(DbmsKind::MySql), false);
    let live = connect(
        &deps,
        req(mysql_profile(TlsMode::Disable)),
        ix_none(),
        token(),
        progress(),
    )
    .await
    .unwrap();
    let connect_step = live.trail.iter().find(|t| t.step == Connect).unwrap();
    assert_eq!(
        (connect_step.outcome, connect_step.elapsed),
        (StepOutcome::Done, None)
    );
    assert!(!live.forwarded());
    let Some(hacivat_driver::fake::FakeEndpoint::Network {
        direct, resolved, ..
    }) = fake.last_request().unwrap().endpoint
    else {
        panic!("a network endpoint");
    };
    assert_eq!(
        direct,
        Some(hacivat_net::Target::Tcp {
            host: "127.0.0.1".into(),
            port: 3306
        })
    );
    assert_eq!(resolved, ["127.0.0.1:3306".parse().unwrap()]);

    // With a transport wrap there is no direct target (P13): the driver goes through its
    // forwarder, and the pipeline opens the first stream (timed).
    let (deps, fake) = deps_dialing_itself(FakeDriver::new(DbmsKind::MySql), true);
    let live = connect(
        &deps,
        req(mysql_profile(TlsMode::Disable)),
        ix_none(),
        token(),
        progress(),
    )
    .await
    .unwrap();
    assert!(live.forwarded());
    assert!(
        live.trail
            .iter()
            .find(|t| t.step == Connect)
            .unwrap()
            .elapsed
            .is_some()
    );
    assert!(matches!(
        fake.last_request().unwrap().endpoint,
        Some(hacivat_driver::fake::FakeEndpoint::Network { direct: None, .. })
    ));
}

#[tokio::test]
async fn validation_fails_before_any_io() {
    let (deps, fake) = deps_with(FakeDriver::new(DbmsKind::Postgres));
    let e = connect(&deps, req(no_host()), ix_none(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!(
        (e.class, e.issues[0].code.clone(), fake.connect_calls()),
        (ErrClass::ConfigInvalid, IssueCode::HostMissing, 0)
    );
    assert!(e.trail.is_empty());
}

#[tokio::test]
async fn ssh_transport_rejected_in_m2a() {
    assert_eq!(
        err(ssh_profile()).await.issues[0].code,
        IssueCode::TransportUnavailable
    );
}

#[tokio::test]
async fn mysql_verify_ca_is_rejected_at_validate() {
    // P11
    assert_eq!(
        err(mysql_profile(TlsMode::VerifyCa)).await.issues[0].code,
        IssueCode::TlsModeUnsupported {
            mode: TlsMode::VerifyCa
        }
    );
    assert!(
        validate(
            &mariadb_profile(TlsMode::VerifyCa),
            EngineCatalog::embedded()
        )
        .iter()
        .any(|i| i.code
            == IssueCode::TlsModeUnsupported {
                mode: TlsMode::VerifyCa
            })
    );
    assert!(validate(&pg_profile(TlsMode::VerifyCa), EngineCatalog::embedded()).is_empty());
}

#[tokio::test]
async fn verify_ca_without_a_ca_file_and_bad_sqlite_paths_are_validate_issues() {
    let mut p = pg_profile(TlsMode::VerifyCa);
    p.tls.ca_file = None;
    let codes: Vec<IssueCode> = validate(&p, EngineCatalog::embedded())
        .into_iter()
        .map(|i| i.code)
        .collect();
    assert_eq!(codes, [IssueCode::TlsCaUnreadable]);

    let mut s = sqlite_profile();
    s.endpoint = hacivat_session::profile::ProfileEndpoint::File {
        path: "file:shop.db?vfs=unix-none".into(),
        create_if_missing: true,
    };
    let e = err(s).await;
    assert_eq!(e.issues[0].code, IssueCode::PathRefused);
    assert!(e.trail.is_empty());
}

#[tokio::test]
async fn cancel_mid_connect_runs_cleanups_lifo() {
    let order = Arc::new(Mutex::new(vec![]));
    // Two ConnectDeps.extra_steps at order 41 and 42 whose guards push "guard-a" and
    // "guard-b" into `order` (P23).
    let deps = deps_with_guards(
        order.clone(),
        FakeDriver::new(DbmsKind::Postgres).delay(Duration::from_millis(500)),
    );
    let t = token();
    let h = tokio::spawn(connect_owned(
        deps,
        req(pg_profile(TlsMode::Disable)),
        ix_none(),
        t.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;
    t.cancel();
    let e = h.await.unwrap().unwrap_err();
    assert_eq!(e.class, ErrClass::Cancelled);
    assert_eq!(*order.lock().unwrap(), ["guard-b", "guard-a"]);
}

#[tokio::test(start_paused = true)]
async fn late_connection_after_timeout_is_closed() {
    let fake = Arc::new(FakeDriver::new(DbmsKind::Postgres).delay(Duration::from_secs(20)));
    let e = err_with(&fake, pg_profile_timeout(15)).await;
    assert_eq!(e.class, ErrClass::NetworkTimeout);
    tokio::time::advance(Duration::from_secs(10)).await;
    // `advance` yields once, and `block_on` polls the test body before the task the timer
    // woke; let that task run (it holds the late connection).
    settle().await;
    assert_eq!(fake.close_calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn slow_prompt_does_not_count_toward_timeout() {
    // P23: paused clock, the prompt answers after 20 s of Tokio time.
    let (deps, _) = deps_with(ok_pg());
    let ui = ScriptedUi::delayed(Duration::from_secs(20), ["pw"]);
    assert!(
        connect(
            &deps,
            req(never_profile_timeout(15)),
            ui.ix(),
            token(),
            progress()
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn slow_keyring_does_not_count_toward_timeout() {
    // P23: real time; the backend's blocking get sleeps 300 ms.
    let mut deps = deps_with_slow_backend(ms(300), "pw");
    deps.timeout_override = Some(ms(200));
    assert!(
        connect(
            &deps,
            req(forever_profile()),
            ix_none(),
            token(),
            progress()
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn password_prompt_when_nothing_is_stored() {
    let (deps, fake) = deps_with(ok_pg());
    let ui = ScriptedUi::answers(["pw"]);
    connect(&deps, req(never_profile()), ui.ix(), token(), progress())
        .await
        .unwrap();
    assert_eq!(fake.last_request().unwrap().password.as_deref(), Some("pw"));
    assert_eq!(backend(&deps).calls(), 0);
}

#[tokio::test]
async fn auth_invalid_reprompts_three_times_then_stops() {
    // P8
    let (deps, _) = deps_with(FakeDriver::new(DbmsKind::Postgres).connect_result(Err(db("28P01"))));
    let ui = ScriptedUi::answers(["a", "b", "c", "d"]);
    let e = connect(&deps, req(never_profile()), ui.ix(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!((e.class, e.attempts), (ErrClass::AuthInvalid, 3));
    assert_eq!(ui.asked_attempts(), [1, 2, 3]);
    assert_eq!(backend(&deps).calls(), 0);
    assert_eq!(e.fixes, [FixId::ReenterPassword, FixId::EditConnection]);
}

#[tokio::test]
async fn stored_password_rejected_returns_without_prompt() {
    // P8
    let (deps, _) = deps_with(FakeDriver::new(DbmsKind::Postgres).connect_result(Err(db("28P01"))));
    let ui = ScriptedUi::answers(["a"]);
    let e = connect(
        &deps,
        req(forever_profile_with("old")),
        ui.ix(),
        token(),
        progress(),
    )
    .await
    .unwrap_err();
    assert_eq!((e.class, e.attempts), (ErrClass::AuthInvalid, 1));
    assert!(ui.asked_attempts().is_empty());
    assert_eq!(stored(&deps), "old");
}

#[tokio::test]
async fn reprompted_password_stored_only_after_success() {
    let fake = FakeDriver::new(DbmsKind::Postgres).script(vec![Err(db("28P01")), Ok(())]);
    let deps = deps_with_fake(fake);
    // Forever profile with nothing stored: prompt, rejection, re-prompt, success.
    let ui = ScriptedUi::answers(["bad", "new"]);
    connect(&deps, req(forever_profile()), ui.ix(), token(), progress())
        .await
        .unwrap();
    assert_eq!(ui.asked_attempts(), [1, 2]);
    assert_eq!(stored(&deps), "new");
}

#[tokio::test]
async fn a_failed_reprompt_stores_nothing() {
    let fake = FakeDriver::new(DbmsKind::Postgres).connect_result(Err(db("28P01")));
    let deps = deps_with_fake(fake);
    let ui = ScriptedUi::answers(["bad", "worse", "worst"]);
    let e = connect(&deps, req(forever_profile()), ui.ix(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!(e.attempts, 3);
    assert_eq!(stored(&deps), "");
}

#[tokio::test]
async fn headless_without_secret_fails_without_prompt() {
    let e = err_ix(never_profile(), Interaction::Headless).await;
    assert!(e.needs_interaction);
    assert_eq!((e.class, e.attempts), (ErrClass::AuthInvalid, 0));
}

#[tokio::test]
async fn unknown_database_suggestions_ranked() {
    let fake = Arc::new(
        FakeDriver::new(DbmsKind::Postgres)
            .script(vec![Err(db("3D000")), Ok(())])
            .databases(vec![
                "postgres".into(),
                "orders_archive".into(),
                "orders".into(),
                "analytics".into(),
            ]),
    );
    let e = err_with(&fake, pg_db("orders_v2")).await;
    assert_eq!(e.class, ErrClass::ConfigUnknownDatabase);
    assert_eq!(e.suggestions, ["orders", "orders_archive", "postgres"]);
    // The probe went to `postgres` and was closed.
    assert_eq!(
        fake.last_request().unwrap().database.as_deref(),
        Some("postgres")
    );
    assert_eq!(fake.close_calls(), 1);
}

#[tokio::test]
async fn sqlite_cantopen_offers_no_database() {
    let fake = Arc::new(FakeDriver::new(DbmsKind::Sqlite).connect_result(Err(
        hacivat_driver::RawError {
            kind: hacivat_driver::RawErrorKind::Db,
            server: Some(hacivat_driver::ServerMessage {
                sqlstate: None,
                vendor_code: Some(14),
                severity: None,
                text: "unable to open database file".into(),
                detail: None,
                hint: None,
                lang_hint: None,
            }),
        },
    )));
    let e = err_with(&fake, sqlite_profile()).await;
    assert_eq!(e.class, ErrClass::ConfigUnknownDatabase);
    assert!(e.suggestions.is_empty());
    assert_eq!((fake.connect_calls(), failed_step(e)), (1, OpenFile));
}

#[tokio::test]
async fn unsupported_version_gate() {
    let fake = Arc::new(FakeDriver::new(DbmsKind::Postgres).server(pg_info("9.6.24")));
    let e = err_with(&fake, pg_profile(TlsMode::Disable)).await;
    assert_eq!(
        (e.class, e.phase),
        (ErrClass::ServerUnsupportedVersion, Phase::PostConnect)
    );
    assert_eq!(
        e.fixes,
        [
            FixId::SupportedVersions,
            FixId::EditConnection,
            FixId::ConnectAnyway
        ]
    );
    assert_eq!(fake.close_calls(), 1);
    assert_eq!(
        (e.args.engine.as_str(), e.args.version.as_deref()),
        ("PostgreSQL", Some("9.6.24"))
    );
    assert_eq!(failed_step(e), CheckServer);
    assert_eq!(
        ok_with(&fake, allow_unsupported(pg_profile(TlsMode::Disable)))
            .await
            .support,
        Support::Unsupported { floor: "14".into() }
    );
}

#[tokio::test]
async fn auto_tls_public_plaintext_refused() {
    // public_auto_profile() is 203.0.113.10
    let fake = Arc::new(FakeDriver::new(DbmsKind::Postgres).tls(plaintext()));
    let e = err_with(&fake, public_auto_profile()).await;
    assert_eq!(e.class, ErrClass::TlsNotSupportedByServer);
    // P9: the driver was told plaintext is not allowed.
    assert!(!fake.last_request().unwrap().tls.unwrap().plaintext_allowed);
    assert_eq!(fake.close_calls(), 1);
}

#[tokio::test]
async fn read_only_is_labelled_guarded() {
    assert_eq!(
        connect_ok(ro(pg_profile(TlsMode::Disable))).await.access,
        AccessLabel::ReadOnlyGuarded
    );
    assert_eq!(
        connect_ok(rw(pg_profile(TlsMode::Disable))).await.access,
        AccessLabel::ReadWrite
    );
}

#[tokio::test]
async fn trail_failure_position_follows_class_family() {
    for (raw, step) in [
        (dns(), Resolve),
        (refused(), Connect),
        (tls_unknown(), Tls),
        (db("28P01"), SignIn),
        (db("3D000"), SignIn),
    ] {
        let fake = Arc::new(FakeDriver::new(DbmsKind::Postgres).connect_result(Err(raw.clone())));
        assert_eq!(
            failed_step(err_with(&fake, pg_profile(TlsMode::Auto)).await),
            step,
            "{raw:?}"
        );
    }
}

#[tokio::test]
async fn connect_logs_never_contain_the_password() {
    let logs = capture_tracing(|| connect_ok(forever_profile_with("s3cret-pw")));
    let logs = logs.await;
    assert!(!logs.contains("s3cret-pw"));
    assert!(logs.contains("connected"), "the connect was traced: {logs}");
}

#[tokio::test]
async fn test_connection_report() {
    let fake = Arc::new(
        ok_pg()
            .pings(vec![Ok(ms(30)), Ok(ms(10)), Ok(ms(20))])
            .tls(plaintext()),
    );
    let r = test_with(&fake, draft(pg_profile(TlsMode::Auto), Some("pw"))).await;
    assert!(r.ok);
    assert_eq!(r.ping_median, Some(ms(20)));
    assert!(r.warnings.contains(&TestWarning::NoTls));
    assert_eq!(backend_calls(), 0);
    assert_eq!(fake.close_calls(), 1);
}

#[tokio::test]
async fn a_test_never_stores_the_draft_but_a_main_connect_does() {
    let fake = Arc::new(ok_pg());
    let r = test_with(&fake, draft(forever_profile(), Some("typed"))).await;
    assert!(r.ok);
    assert_eq!(
        fake.last_request().unwrap().password.as_deref(),
        Some("typed")
    );
    assert_eq!(backend_calls(), 0);

    let deps = deps_with_fake(ok_pg());
    let mut request = req(forever_profile());
    request.draft_password = Some(hacivat_secrets::SecretString::from("typed"));
    connect(&deps, request, ix_none(), token(), progress())
        .await
        .unwrap();
    assert_eq!(stored(&deps), "typed");
}

// Fix round 1.

#[tokio::test]
async fn the_pre_opened_stream_is_used_when_nothing_waited() {
    let (deps, seen) = deps_reading_streams(ok_pg());
    connect(
        &deps,
        req(pg_profile(TlsMode::Disable)),
        ix_none(),
        token(),
        progress(),
    )
    .await
    .unwrap();
    assert_eq!(*seen.lock().unwrap(), [1]);
}

#[tokio::test]
async fn a_prompt_or_a_keyring_read_gets_a_fresh_stream() {
    // The pre-opened stream (1) idled through the prompt: the driver gets stream 2.
    let (deps, seen) = deps_reading_streams(ok_pg());
    let ui = ScriptedUi::answers(["pw"]);
    connect(&deps, req(never_profile()), ui.ix(), token(), progress())
        .await
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), [2]);

    // A keyring read may show a system dialog: the same.
    let (deps, seen) = deps_reading_streams(ok_pg());
    connect(
        &deps,
        req(forever_profile_with("kept")),
        ix_none(),
        token(),
        progress(),
    )
    .await
    .unwrap();
    assert_eq!(*seen.lock().unwrap(), [2]);
}

#[tokio::test(start_paused = true)]
async fn a_stream_older_than_five_seconds_is_replaced() {
    let (mut deps, seen) = deps_reading_streams(ok_pg());
    deps.extra_steps = vec![wait_step(Duration::from_secs(6))];
    let live = connect(
        &deps,
        req(pg_profile(TlsMode::Disable)),
        ix_none(),
        token(),
        progress(),
    )
    .await
    .unwrap();
    assert_eq!(*seen.lock().unwrap(), [2]);
    // The Connect step keeps the time the pipeline measured for the first stream.
    let connect_step = live.trail.iter().find(|t| t.step == Connect).unwrap();
    assert_eq!(connect_step.outcome, StepOutcome::Done);
    assert!(connect_step.elapsed.is_some());
}

fn password_required() -> hacivat_driver::RawError {
    hacivat_driver::RawError {
        kind: hacivat_driver::RawErrorKind::PasswordRequired,
        server: None,
    }
}

#[tokio::test]
async fn a_password_required_answer_leads_to_the_prompt() {
    // PostgreSQL's "password missing": the server wanted one and none was sent.
    let fake = FakeDriver::new(DbmsKind::Postgres).script(vec![Err(password_required())]);
    let (deps, fake) = deps_with(fake);
    let ui = ScriptedUi::answers(["pw"]);
    connect(
        &deps,
        req(pg_profile(TlsMode::Disable)),
        ui.ix(),
        token(),
        progress(),
    )
    .await
    .unwrap();
    assert_eq!(
        ui.asked_attempts(),
        [1],
        "a missing password is not a rejection"
    );
    assert_eq!(fake.last_request().unwrap().password.as_deref(), Some("pw"));
    // Headless, it stops where it would ask.
    let fake =
        Arc::new(FakeDriver::new(DbmsKind::Postgres).connect_result(Err(password_required())));
    let e = err_with(&fake, pg_profile(TlsMode::Disable)).await;
    assert_eq!(
        (e.class, e.attempts, e.needs_interaction),
        (ErrClass::AuthInvalid, 0, true)
    );
}

#[tokio::test]
async fn a_reentry_continues_the_gestures_count_of_three() {
    // R8, P8: the error dialog's re-entry after 2 rejections gets one more try.
    let (deps, fake) =
        deps_with(FakeDriver::new(DbmsKind::Postgres).connect_result(Err(db("28P01"))));
    let mut r = req(forever_profile());
    r.draft_password = Some(hacivat_secrets::SecretString::from("again"));
    r.prior_rejections = 2;
    let ui = ScriptedUi::answers(["never-asked"]);
    let e = connect(&deps, r, ui.ix(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!((e.class, e.attempts), (ErrClass::AuthInvalid, 3));
    assert!(ui.asked_attempts().is_empty());
    assert_eq!(fake.connect_calls(), 1);

    // A spent gesture does not connect at all.
    let mut r = req(forever_profile());
    r.prior_rejections = 3;
    let e = connect(&deps, r, ui.ix(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!((e.class, e.attempts), (ErrClass::AuthInvalid, 3));
    assert_eq!(fake.connect_calls(), 1);

    // Prompts continue the numbering: after 1 rejection, attempts 2 and 3 are left.
    let mut r = req(never_profile());
    r.prior_rejections = 1;
    let ui = ScriptedUi::answers(["a", "b", "c"]);
    let e = connect(&deps, r, ui.ix(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!(ui.asked_attempts(), [2, 3]);
    assert_eq!(e.attempts, 3);
}

#[tokio::test]
async fn a_reentry_that_works_is_stored() {
    let (deps, _) = deps_with(ok_pg());
    let mut r = req(forever_profile());
    r.draft_password = Some(hacivat_secrets::SecretString::from("fixed"));
    r.prior_rejections = 1;
    connect(&deps, r, ix_none(), token(), progress())
        .await
        .unwrap();
    assert_eq!(stored(&deps), "fixed");
}

/// A UI that cancels the connect while its prompt is open, then answers.
struct CancellingUi(tokio_util::sync::CancellationToken);

impl hacivat_session::pipeline::UiPort for CancellingUi {
    fn ask(
        &self,
        _q: hacivat_session::pipeline::Question,
    ) -> futures::future::BoxFuture<'static, hacivat_session::pipeline::Answer> {
        self.0.cancel();
        Box::pin(async {
            hacivat_session::pipeline::Answer::Password(hacivat_secrets::SecretString::from("late"))
        })
    }
}

#[tokio::test]
async fn a_cancelled_connect_stores_nothing() {
    let (deps, fake) = deps_with(ok_pg());
    let t = token();
    let ix = Interaction::Interactive(Arc::new(CancellingUi(t.clone())));
    let e = connect(&deps, req(forever_profile()), ix, t, progress())
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::Cancelled);
    assert_eq!(stored(&deps), "");
    assert_eq!(fake.connect_calls(), 0);
}

#[tokio::test]
async fn validation_uses_the_deps_approved_roots() {
    let home = tempfile::tempdir().unwrap();
    let ca = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/test-ca.pem"
    ))
    .unwrap();
    let file = home.path().join("ca.pem");
    std::fs::write(&file, ca).unwrap();
    let mut p = pg_profile(TlsMode::VerifyFull);
    p.tls.ca_file = Some(file.clone());
    p.origin = hacivat_proto::Origin::Imported {
        source: hacivat_proto::ImportSource::PgService,
    };
    // A server that negotiates verified TLS (VerifyFull refuses plaintext, R3).
    let (mut deps, fake) = deps_with(ok_pg().tls(hacivat_net::TlsSummary {
        encrypted: true,
        version: Some("TLSv1.3".into()),
        cipher: None,
        verified: hacivat_net::TlsVerified::Full,
    }));
    deps.approved_roots = hacivat_session::pipeline::ApprovedRoots::new(Vec::new());
    let e = connect(&deps, req(p.clone()), ix_none(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!(
        (e.class, e.issues[0].code.clone(), fake.connect_calls()),
        (ErrClass::ConfigInvalid, IssueCode::PathRefused, 0)
    );
    // The user picks the file in the dialog: it is approved for every later connect.
    deps.approved_roots.add(file);
    assert!(
        connect(&deps, req(p), ix_none(), token(), progress())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn rejected_and_prompted_passwords_never_reach_logs_or_debug() {
    // A stored password the server rejects.
    let fake = Arc::new(FakeDriver::new(DbmsKind::Postgres).connect_result(Err(db("28P01"))));
    let profile = forever_profile_with("s3cret-stored");
    let logs = capture_tracing(|| err_with(&fake, profile.clone())).await;
    assert!(!logs.contains("s3cret-stored"));
    let e = err_with(&fake, profile).await;
    assert!(!format!("{e:?}").contains("s3cret-stored"));

    // Prompted passwords, each rejected.
    let (deps, _) = deps_with(FakeDriver::new(DbmsKind::Postgres).connect_result(Err(db("28P01"))));
    let answers = ["s3cret-a", "s3cret-b", "s3cret-c"];
    let ui = ScriptedUi::answers(answers);
    let logs = capture_tracing(|| async {
        let e = connect(&deps, req(never_profile()), ui.ix(), token(), progress())
            .await
            .unwrap_err();
        format!("{e:?}")
    });
    let logs = logs.await;
    let ui = ScriptedUi::answers(answers);
    let e = connect(&deps, req(never_profile()), ui.ix(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!(e.attempts, 3);
    for pw in answers {
        assert!(!logs.contains(pw), "{pw} in logs");
        assert!(!format!("{e:?}").contains(pw), "{pw} in ConnectError");
    }

    // A request with a draft, and the live connection made from it.
    let mut r = req(forever_profile());
    r.draft_password = Some(hacivat_secrets::SecretString::from("s3cret-draft"));
    assert!(!format!("{r:?}").contains("s3cret-draft"));
    let (deps, _) = deps_with(ok_pg());
    let live = connect(&deps, r, ix_none(), token(), progress())
        .await
        .unwrap();
    assert!(!format!("{live:?}").contains("s3cret-draft"));
}
