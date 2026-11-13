//! The pipeline's interactive TLS (Task 14; B5–B8, Q13, Q17, Q20, Q26): the certificate
//! question (pin, import CA, cancel), pin changes as hard stops, the public-plaintext question,
//! the insecure-auth policy and its errors. `FakeDriver` only; hosts are IP literals, so no
//! test sends a DNS query (P46).

#![allow(
    clippy::result_large_err,
    reason = "ConnectError is the pipeline's fixed interface"
)]

mod common;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use common::*;
use hacivat_driver::fake::FakeDriver;
use hacivat_driver::{InsecureAuth, RawError, RawErrorKind, fixes_for};
use hacivat_net::{
    CertPin, TlsFailure, TlsMode, TlsVerified, cert_sha256, classify_host, pin_endpoint,
};
use hacivat_proto::{DbmsKind, ErrClass, FixId};
use hacivat_session::pipeline::{
    Answer, ConnectDeps, ConnectError, LiveConnection, ProfileChange, Question, connect,
    insecure_auth_for,
};
use hacivat_session::profile::{Profile, ProfileEndpoint};

// ---------------------------------------------------------------------------------------------
// Fixtures

/// A real certificate's DER (the test CA, served as a leaf), so the review parses.
fn leaf_der() -> Vec<u8> {
    include_bytes!("fixtures/test-ca.der").to_vec()
}

fn raw(kind: RawErrorKind) -> RawError {
    RawError { kind, server: None }
}

fn tls(f: TlsFailure) -> RawError {
    raw(RawErrorKind::Tls(f))
}

fn tls_not_offered() -> RawError {
    raw(RawErrorKind::TlsNotOffered)
}

/// A PostgreSQL profile toward `host:port` with TLS `mode` (no password).
fn pg_mode(host: &str, port: u16, mode: TlsMode) -> Profile {
    let mut p = pg_profile(mode);
    p.endpoint = ProfileEndpoint::Server {
        host: host.into(),
        port: Some(port),
    };
    p
}

fn pg_auto(host: &str, port: u16) -> Profile {
    pg_mode(host, port, TlsMode::Auto)
}

fn pg_require(host: &str, port: u16) -> Profile {
    pg_mode(host, port, TlsMode::Require)
}

/// `p` with a pin of `sha256` for its endpoint.
fn pinned(mut p: Profile, sha256: [u8; 32]) -> Profile {
    let ProfileEndpoint::Server { host, port } = &p.endpoint else {
        panic!("a server profile")
    };
    let endpoint = pin_endpoint(host, port.unwrap());
    p.tls.pins.push(CertPin { endpoint, sha256 });
    p
}

/// The matrix profile: `host` (port 5432) with `mode`, and the remembered plaintext consent.
fn pg_with(host: &str, mode: TlsMode, consent: bool) -> Profile {
    let mut p = pg_mode(host, 5432, mode);
    p.tls.allow_public_plaintext = consent;
    p
}

/// A CA file the file picker returned: absolute, outside any default root.
fn picked_file(name: &str) -> PathBuf {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    let path = dir.path().join(name);
    std::fs::write(&path, include_bytes!("fixtures/test-ca.pem")).unwrap();
    path
}

fn deps_of(f: &Arc<FakeDriver>) -> ConnectDeps {
    let (mut deps, _) = deps_with(FakeDriver::new(f_kind(f)));
    let mut drivers = hacivat_driver::DriverRegistry::new();
    drivers.register(f.clone());
    deps.drivers = Arc::new(drivers);
    deps
}

fn f_kind(f: &Arc<FakeDriver>) -> DbmsKind {
    hacivat_driver::Driver::descriptor(f.as_ref()).kind
}

async fn connect_ix(
    f: &Arc<FakeDriver>,
    p: Profile,
    ui: &ScriptedUiM2b,
) -> Result<LiveConnection, ConnectError> {
    connect(&deps_of(f), req(p), ui.ix(), token(), progress()).await
}

async fn connect_headless(f: &Arc<FakeDriver>, p: Profile) -> Result<LiveConnection, ConnectError> {
    connect(&deps_of(f), req(p), ix_none(), token(), progress()).await
}

/// A driver that fails once with an unknown issuer (presenting `leaf_der`), then connects; and
/// a UI that answers the first question with `answer`.
fn untrusted_then_ok(answer: Answer) -> (Arc<FakeDriver>, ScriptedUiM2b) {
    let f = FakeDriver::new(DbmsKind::Postgres)
        .presented(vec![leaf_der()])
        .script(vec![Err(tls(TlsFailure::UnknownIssuer)), Ok(())]);
    (Arc::new(f), ScriptedUi::answers_m2b([answer]))
}

// ---------------------------------------------------------------------------------------------
// The certificate question (B6)

#[tokio::test]
async fn untrusted_auto_interactive_asks_certificate_and_pins() {
    let f = Arc::new(
        FakeDriver::new(DbmsKind::Postgres)
            .presented(vec![leaf_der()])
            .script(vec![Err(tls(TlsFailure::UnknownIssuer)), Ok(())]),
    );
    let ui = ScriptedUi::answers_m2b([Answer::Pin]);
    let live = connect_ix(&f, pg_auto("10.0.0.5", 5432), &ui)
        .await
        .unwrap();
    assert!(matches!(ui.asked()[0], Question::Certificate { .. }));
    assert_eq!(
        live.profile_changes,
        [ProfileChange::AddPin(CertPin {
            endpoint: "10.0.0.5:5432".into(),
            sha256: cert_sha256(&leaf_der())
        })]
    );
    assert_eq!(
        f.last_request().unwrap().tls.unwrap().settings.pins.len(),
        1
    );
    assert_eq!(f.connect_calls(), 2);
}

#[tokio::test]
async fn import_ca_answer_sets_ca_file_and_approves_root() {
    let path = picked_file("corp-ca.pem");
    let (f, ui) = untrusted_then_ok(Answer::ImportCa(path.clone()));
    let deps = deps_of(&f);
    let live = connect(
        &deps,
        req(pg_auto("10.0.0.5", 5432)),
        ui.ix(),
        token(),
        progress(),
    )
    .await
    .unwrap();
    assert_eq!(
        live.profile_changes,
        [ProfileChange::SetCaFile(path.clone())]
    );
    assert!(deps.approved_roots.list().contains(&path));
    let retry = f.last_request().unwrap().tls.unwrap();
    assert_eq!(retry.settings.ca_file.as_deref(), Some(path.as_path()));
}

/// Review probe: `ImportCa` comes from the file picker; a relative path or a share is never
/// opened, approved or written into the profile.
#[tokio::test]
async fn import_ca_never_takes_a_typed_or_share_path() {
    for bad in [r"\\attacker\s\ca.pem", "relative/ca.pem"] {
        let (f, ui) = untrusted_then_ok(Answer::ImportCa(PathBuf::from(bad)));
        let deps = deps_of(&f);
        let e = connect(
            &deps,
            req(pg_auto("10.0.0.5", 5432)),
            ui.ix(),
            token(),
            progress(),
        )
        .await
        .unwrap_err();
        assert_eq!(e.class, ErrClass::TlsUntrusted, "{bad}");
        assert!(e.profile_changes.is_empty(), "{bad}");
        assert!(!deps.approved_roots.list().contains(&PathBuf::from(bad)));
        assert_eq!(f.connect_calls(), 1, "{bad}");
    }
}

#[tokio::test]
async fn cancelled_certificate_question_returns_review_in_error() {
    let (f, ui) = untrusted_then_ok(Answer::Cancelled);
    let e = connect_ix(&f, pg_auto("10.0.0.5", 5432), &ui)
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::TlsUntrusted);
    assert!(e.certificate.is_some());
    assert_eq!(
        e.fixes,
        [
            FixId::ReviewCertificate,
            FixId::ImportCa,
            FixId::EditConnection
        ]
    );
}

#[tokio::test]
async fn headless_untrusted_never_asks() {
    let (f, _) = untrusted_then_ok(Answer::Pin);
    let e = connect_headless(&f, pg_auto("10.0.0.5", 5432))
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::TlsUntrusted);
    assert!(e.needs_interaction);
    assert_eq!(f.connect_calls(), 1);
}

/// Review probe: a pin that still fails (the server keeps presenting an untrusted chain) is
/// never asked again: one question, one retry.
#[tokio::test]
async fn a_pin_that_does_not_help_is_never_asked_twice() {
    let f = Arc::new(
        FakeDriver::new(DbmsKind::Postgres)
            .presented(vec![leaf_der()])
            .connect_result(Err(tls(TlsFailure::UnknownIssuer))),
    );
    let ui = ScriptedUi::answers_m2b([Answer::Pin, Answer::Pin, Answer::Pin]);
    let e = connect_ix(&f, pg_auto("10.0.0.5", 5432), &ui)
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::TlsUntrusted);
    assert_eq!(ui.asked().len(), 1);
    assert_eq!(f.connect_calls(), 2);
    // The pin the user made still reaches the profile.
    assert!(matches!(e.profile_changes[..], [ProfileChange::AddPin(_)]));
}

/// An expired certificate is untrusted too, but a pin never extends a certificate's dates (B5):
/// not asked; the review is still attached.
#[tokio::test]
async fn an_expired_certificate_is_not_offered_a_pin() {
    let f = Arc::new(
        FakeDriver::new(DbmsKind::Postgres)
            .presented(vec![leaf_der()])
            .connect_result(Err(tls(TlsFailure::Expired))),
    );
    let ui = ScriptedUi::answers_m2b([Answer::Pin]);
    let e = connect_ix(&f, pg_auto("10.0.0.5", 5432), &ui)
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::TlsUntrusted);
    assert!(ui.asked().is_empty());
    assert!(e.certificate.is_some());
}

// ---------------------------------------------------------------------------------------------
// Pins (B5, Q13, Q26)

#[tokio::test]
async fn pin_changed_is_hard_stop_with_both_fingerprints() {
    let f = Arc::new(
        FakeDriver::new(DbmsKind::Postgres)
            .presented(vec![leaf_der()])
            .connect_result(Err(tls(TlsFailure::PinMismatch))),
    );
    let ui = ScriptedUi::answers_m2b([]);
    let e = connect_ix(&f, pinned(pg_auto("10.0.0.5", 5432), [1; 32]), &ui)
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::TlsPinChanged);
    let review = e.certificate.unwrap();
    assert_eq!(review.pinned, Some([1; 32]));
    assert_eq!(review.sha256, cert_sha256(&leaf_der()));
    assert!(ui.asked().is_empty());
    assert_eq!(e.fixes, [FixId::ReviewCertificate]);
}

#[tokio::test]
async fn pinned_endpoint_tls_not_offered_is_not_asked() {
    // Q13
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).connect_result(Err(tls_not_offered())));
    let ui = ScriptedUi::answers_m2b([]);
    let e = connect_ix(&f, pinned(pg_auto("203.0.113.10", 5432), [1; 32]), &ui)
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::TlsNotSupportedByServer);
    assert_eq!(e.fixes, [FixId::EditConnection]);
    assert!(ui.asked().is_empty());
    assert!(!f.requests()[0].tls.as_ref().unwrap().plaintext_allowed);
}

/// Review probe (Q13): even a remembered consent and the overlay's consent never take a pinned
/// endpoint to plaintext.
#[tokio::test]
async fn a_pinned_endpoint_never_reaches_plaintext_through_any_consent() {
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).connect_result(Err(tls_not_offered())));
    let mut p = pinned(pg_auto("203.0.113.10", 5432), [1; 32]);
    p.tls.allow_public_plaintext = true;
    let mut r = req(p);
    r.plaintext_consent = true;
    let ui = ScriptedUi::answers_m2b([Answer::ConnectPlaintext { remember: true }]);
    let e = connect(&deps_of(&f), r, ui.ix(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::TlsNotSupportedByServer);
    assert_eq!(f.connect_calls(), 1);
    assert!(
        f.requests()
            .iter()
            .all(|r| !r.tls.as_ref().unwrap().plaintext_allowed)
    );
    assert!(ui.asked().is_empty());
}

#[tokio::test]
async fn pipeline_never_produces_replace_pin() {
    // Q26
    let f = Arc::new(
        FakeDriver::new(DbmsKind::Postgres)
            .presented(vec![leaf_der()])
            .connect_result(Err(tls(TlsFailure::PinMismatch))),
    );
    let e = connect_ix(
        &f,
        pinned(pg_auto("10.0.0.5", 5432), [1; 32]),
        &ScriptedUi::answers_m2b([Answer::Pin]),
    )
    .await
    .unwrap_err();
    assert!(
        !e.profile_changes
            .iter()
            .any(|c| matches!(c, ProfileChange::ReplacePin { .. }))
    );
}

#[test]
fn tls_untrusted_never_offers_encrypt_without_verifying() {
    assert!(!fixes_for(ErrClass::TlsUntrusted).contains(&FixId::EncryptWithoutVerifying));
    assert!(!fixes_for(ErrClass::TlsHostname).contains(&FixId::EncryptWithoutVerifying));
}

// ---------------------------------------------------------------------------------------------
// The public-plaintext question (B7, Q17)

#[tokio::test]
async fn public_plaintext_asks_and_retries_once() {
    // Renamed (Q44): T10's Require path proves "not sent".
    let f =
        Arc::new(FakeDriver::new(DbmsKind::Postgres).script(vec![Err(tls_not_offered()), Ok(())]));
    let ui = ScriptedUi::answers_m2b([Answer::ConnectPlaintext { remember: true }]);
    let live = connect_ix(&f, pg_auto("203.0.113.10", 5432), &ui)
        .await
        .unwrap();
    let reqs = f.requests();
    assert!(!reqs[0].tls.as_ref().unwrap().plaintext_allowed);
    assert_eq!(reqs[0].insecure_auth, InsecureAuth::RefuseUnlessVerified);
    assert!(reqs[1].tls.as_ref().is_none_or(|t| t.plaintext_allowed));
    assert_eq!(reqs[1].insecure_auth, InsecureAuth::Allow);
    assert_eq!(live.profile_changes, [ProfileChange::AllowPublicPlaintext]);
    assert!(live.plaintext_consent);
    assert!(matches!(ui.asked()[0], Question::PlaintextPublic { .. }));
}

#[tokio::test]
async fn plaintext_without_remember_is_a_one_gesture_consent() {
    let f =
        Arc::new(FakeDriver::new(DbmsKind::Postgres).script(vec![Err(tls_not_offered()), Ok(())]));
    let ui = ScriptedUi::answers_m2b([Answer::ConnectPlaintext { remember: false }]);
    let live = connect_ix(&f, pg_auto("203.0.113.10", 5432), &ui)
        .await
        .unwrap();
    assert!(live.profile_changes.is_empty());
    assert!(live.plaintext_consent, "for the session overlay (Q17)");
}

/// The overlay's one-gesture consent retries at once, headless too, without a question (Q17).
#[tokio::test]
async fn the_overlay_consent_retries_without_asking() {
    let f =
        Arc::new(FakeDriver::new(DbmsKind::Postgres).script(vec![Err(tls_not_offered()), Ok(())]));
    let mut r = req(pg_auto("203.0.113.10", 5432));
    r.plaintext_consent = true;
    let live = connect(&deps_of(&f), r, ix_none(), token(), progress())
        .await
        .unwrap();
    assert_eq!(f.connect_calls(), 2);
    assert!(!f.requests()[0].tls.as_ref().unwrap().plaintext_allowed);
    assert_eq!(f.requests()[1].insecure_auth, InsecureAuth::Allow);
    assert!(live.plaintext_consent);
}

#[tokio::test]
async fn headless_public_plaintext_never_asks() {
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).connect_result(Err(tls_not_offered())));
    let e = connect_headless(&f, pg_auto("203.0.113.10", 5432))
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::TlsNotSupportedByServer);
    assert!(e.needs_interaction);
    assert_eq!(f.connect_calls(), 1);
}

#[tokio::test]
async fn use_ssh_answer_returns_connect_through_ssh_fix() {
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).connect_result(Err(tls_not_offered())));
    let e = connect_ix(
        &f,
        pg_auto("203.0.113.10", 5432),
        &ScriptedUi::answers_m2b([Answer::UseSsh]),
    )
    .await
    .unwrap_err();
    assert_eq!(e.fixes, [FixId::ConnectThroughSsh, FixId::EditConnection]);
}

/// Review probe: a second refusal after the consent is the error, never a second question.
#[tokio::test]
async fn the_plaintext_question_is_asked_once() {
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).connect_result(Err(tls_not_offered())));
    let ui = ScriptedUi::answers_m2b([
        Answer::ConnectPlaintext { remember: false },
        Answer::ConnectPlaintext { remember: false },
    ]);
    let e = connect_ix(&f, pg_auto("203.0.113.10", 5432), &ui)
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::TlsNotSupportedByServer);
    assert_eq!(ui.asked().len(), 1);
    assert_eq!(f.connect_calls(), 2);
}

/// A remembered consent lets Auto fall back at once (no question) and covers the password.
#[tokio::test]
async fn a_remembered_consent_allows_plaintext_from_the_first_attempt() {
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres));
    let live = connect_headless(&f, pg_with("203.0.113.10", TlsMode::Auto, true))
        .await
        .unwrap();
    let r = f.last_request().unwrap();
    assert!(r.tls.unwrap().plaintext_allowed);
    assert_eq!(r.insecure_auth, InsecureAuth::Allow);
    assert!(!live.tls.encrypted);
}

// ---------------------------------------------------------------------------------------------
// The insecure-auth policy (B8, Q20)

#[test]
fn insecure_auth_policy_matrix() {
    use InsecureAuth::*;
    for (host, mode, consent, want) in [
        ("127.0.0.1", TlsMode::Auto, false, Allow),
        ("10.0.0.5", TlsMode::Disable, false, Allow),
        ("203.0.113.10", TlsMode::Auto, true, Allow),
        ("10.0.0.5", TlsMode::Auto, false, RefuseUnlessVerified),
        (
            "203.0.113.10",
            TlsMode::VerifyFull,
            false,
            RefuseUnlessVerified,
        ),
        // Not in the brief's table: consent only counts under Auto; a socket is local.
        ("203.0.113.10", TlsMode::Require, true, RefuseUnlessVerified),
        ("10.0.0.5", TlsMode::Require, false, RefuseUnlessVerified),
    ] {
        assert_eq!(
            insecure_auth_for(
                &pg_with(host, mode, consent),
                classify_host(host, &[]),
                mode
            ),
            want,
            "{host} {mode:?}"
        );
    }
    assert_eq!(
        insecure_auth_for(
            &pg_with("10.0.0.5", TlsMode::Auto, false),
            hacivat_net::AddressClass::UnixSocket,
            TlsMode::Auto
        ),
        Allow
    );
}

/// The connect sends the policy's answer: refused toward a private host under Auto.
#[tokio::test]
async fn the_request_carries_the_policy() {
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres));
    let _ = connect_headless(&f, pg_auto("10.0.0.5", 5432))
        .await
        .unwrap();
    assert_eq!(
        f.last_request().unwrap().insecure_auth,
        InsecureAuth::RefuseUnlessVerified
    );
    let _ = connect_headless(&f, pg_auto("127.0.0.1", 5432))
        .await
        .unwrap();
    assert_eq!(f.last_request().unwrap().insecure_auth, InsecureAuth::Allow);
}

/// Without a presented chain (no TLS) "Review certificate" is left out (mocks, NOTES ruling 10;
/// the brief's `[ReviewCertificate, EditConnection]` is the TLS case below).
#[tokio::test]
async fn insecure_method_class_and_fixes() {
    let f = Arc::new(
        FakeDriver::new(DbmsKind::Postgres)
            .connect_result(Err(raw(RawErrorKind::InsecureAuthRefused))),
    );
    let e = err_with(&f, pg_auto("10.0.0.5", 5432)).await;
    assert_eq!(e.class, ErrClass::AuthInsecureMethod);
    assert!(e.certificate.is_none());
    assert_eq!(e.fixes, [FixId::EditConnection]);
    assert_eq!(
        fixes_for(ErrClass::AuthInsecureMethod),
        [FixId::ReviewCertificate, FixId::EditConnection]
    );
}

#[tokio::test]
async fn insecure_method_keeps_certificate_when_tls_was_up() {
    // Q20
    let f = Arc::new(
        FakeDriver::new(DbmsKind::Postgres)
            .presented(vec![leaf_der()])
            .tls_outcome(TlsVerified::No)
            .connect_result(Err(raw(RawErrorKind::InsecureAuthRefused))),
    );
    let e = err_with(&f, pg_require("10.0.0.5", 5432)).await;
    assert_eq!(e.class, ErrClass::AuthInsecureMethod);
    assert!(e.certificate.is_some());
    assert_eq!(e.fixes, [FixId::ReviewCertificate, FixId::EditConnection]);
}

/// Q20: after "Review certificate" → pin, the outcome is `Pinned` and the guard allows the
/// password; the policy itself stays `RefuseUnlessVerified` (outcome-based, never mode-based).
#[tokio::test]
async fn a_pinned_require_profile_keeps_the_outcome_based_guard() {
    let f = Arc::new(
        FakeDriver::new(DbmsKind::Postgres)
            .tls_outcome(TlsVerified::Pinned)
            .tls(hacivat_net::TlsSummary {
                encrypted: true,
                version: Some("TLSv1.3".into()),
                cipher: None,
                verified: TlsVerified::Pinned,
            }),
    );
    let live = connect_headless(
        &f,
        pinned(pg_require("10.0.0.5", 5432), cert_sha256(&leaf_der())),
    )
    .await
    .unwrap();
    let r = f.last_request().unwrap();
    assert_eq!(r.insecure_auth, InsecureAuth::RefuseUnlessVerified);
    assert_eq!(r.tls.unwrap().settings.pins.len(), 1);
    drop(live);
}
