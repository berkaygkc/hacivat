//! M2b conformance over Docker (Task 23; exit criteria E6, E8): TLS `Auto` pins a self-signed
//! certificate (PostgreSQL and, through patch §5, MySQL), directly and through SSH; MySQL
//! `VerifyCa` with the container's CA; the insecure-auth guard refuses PostgreSQL md5 and
//! cleartext passwords through SSH unless TLS is off by the user's choice. Run with `cargo xtask
//! docker test --set pr|full -p hacivat-session`.
//!
//! Targets come only from `require_target` (Q6); through SSH a test dials the target's
//! in-network endpoint, and the TLS host (SNI, the pin's endpoint) is the database host.

#![allow(
    clippy::result_large_err,
    reason = "ConnectError is the pipeline's fixed interface"
)]

mod common;

use common::ScriptedUi;
use common::conformance::*;
use hacivat_net::{ClientCertPaths, TlsMode, TlsVerified};
use hacivat_proto::ErrClass;
use hacivat_secrets::SecretString;
use hacivat_session::pipeline::{
    Answer, ConnectError, Interaction, LiveConnection, ProfileChange, Question, SessionSecrets,
    connect,
};
use hacivat_session::profile::Profile;
use hacivat_testkit::{DockerTarget, MYSQL_X509_KEY_PASSPHRASE, require_target};

/// Connects `p` with `ix`; the chain's secrets are not needed (one bastion, plain key).
async fn connect_ix(p: Profile, ix: Interaction) -> Result<LiveConnection, ConnectError> {
    connect(&matrix_deps(), req(p), ix, token(), progress()).await
}

/// `t` in `Auto` on `path`, answered `Pin`: the connect asks once, pins, and is `Pinned` with
/// one `AddPin`; the pinned profile then connects headless, pinned, with no change.
async fn pins_in_auto(t: &DockerTarget, path: Path) {
    let p = profile_on(t, path, TlsMode::Auto).expect("the path's bastions are in the set");
    let ui = ScriptedUi::answers_m2b([Answer::Pin]);
    let live = connect_ix(p.clone(), ui.ix())
        .await
        .unwrap_or_else(|e| panic!("{} {path:?}: {:?}", t.id, e.class));
    assert!(live.tls.encrypted, "{} {path:?}", t.id);
    assert_eq!(live.tls.verified, TlsVerified::Pinned, "{} {path:?}", t.id);
    assert_eq!(live.profile_changes.len(), 1, "{} {path:?}", t.id);
    let ProfileChange::AddPin(pin) = live.profile_changes[0].clone() else {
        panic!("{} {path:?}: {:?}", t.id, live.profile_changes);
    };
    // One question, the certificate review.
    let asked = ui.asked();
    assert_eq!(asked.len(), 1, "{} {path:?}", t.id);
    assert!(
        matches!(asked[0], Question::Certificate { .. }),
        "{} {path:?}",
        t.id
    );
    // The pin names the TLS host: the database host (its in-network name through SSH).
    let host = match path {
        Path::Direct => t.host.clone(),
        _ => t.internal.0.clone(),
    };
    assert!(
        pin.endpoint.starts_with(&host),
        "{} {path:?}: the pin is for {}",
        t.id,
        pin.endpoint
    );
    live.close().await;

    let mut pinned = p;
    pinned.tls.pins.push(pin);
    let again = connect_ix(pinned, Interaction::Headless)
        .await
        .unwrap_or_else(|e| panic!("{} {path:?} pinned: {:?}", t.id, e.class));
    assert_eq!(again.tls.verified, TlsVerified::Pinned, "{} {path:?}", t.id);
    assert!(again.profile_changes.is_empty(), "{} {path:?}", t.id);
    again.close().await;
}

/// MySQL `VerifyCa` with the CA the container generated (`xtask docker up` copies it).
async fn connect_mysql_verify_ca(id: &str) -> Option<Result<LiveConnection, ConnectError>> {
    let t = require_target(id)?;
    let mut p = profile_for(&t, TlsMode::VerifyCa);
    p.tls.ca_file = Some(hacivat_testkit::artifact("mysql84-ca.pem"));
    Some(connect_ix(p, Interaction::Headless).await)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn tls_auto_pin_and_verify_ca() {
    if !docker_enabled() {
        return;
    }
    for t in ["pg17-tls", "mysql84"]
        .into_iter()
        .filter_map(require_target)
    {
        for path in [Path::Direct, Path::Ssh("sshd-a")] {
            if path.available() {
                pins_in_auto(&t, path).await;
            }
        }
    }
    if let Some(verified) = connect_mysql_verify_ca("mysql84").await {
        let live = verified.unwrap_or_else(|e| panic!("mysql84 VerifyCa: {:?}", e.class));
        assert_eq!(live.tls.verified, TlsVerified::CaOnly);
        live.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn insecure_auth_refused_through_ssh_on_md5() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17-auth") else {
        return;
    };
    let path = Path::Ssh("sshd-a");
    if !path.available() {
        return;
    }
    // pg17-auth signs `hv_md5` in with md5 and `hv_clear` with a cleartext password, and serves
    // no TLS. Through SSH the database host is a private address (B11), so under Auto the
    // password would cross the bastion's network unverified: refused before it is sent (B8).
    for user in ["hv_md5", "hv_clear"] {
        let p = profile_on(&t, path, TlsMode::Auto)
            .expect("sshd-a is in the set")
            .user(user);
        let e = connect_ix(p, Interaction::Headless)
            .await
            .err()
            .unwrap_or_else(|| panic!("{user}: the connect succeeded"));
        assert_eq!(e.class, ErrClass::AuthInsecureMethod, "{user}");
        // `Disable` is the user's explicit choice of plaintext: allowed.
        let p = profile_on(&t, path, TlsMode::Disable)
            .expect("sshd-a is in the set")
            .user(user);
        let live = connect_ix(p, Interaction::Headless)
            .await
            .unwrap_or_else(|e| panic!("{user} with Disable: {:?}", e.class));
        assert_eq!(live.user.as_deref(), Some(user));
        assert!(!live.tls.encrypted, "{user}");
        live.close().await;
        // Directly the server is on loopback, where the password never crosses a network.
        let p = profile_for(&t, TlsMode::Auto).user(user);
        let live = connect_ix(p, Interaction::Headless)
            .await
            .unwrap_or_else(|e| panic!("{user} on loopback: {:?}", e.class));
        live.close().await;
    }
    // SCRAM is not an insecure method: the test account connects through SSH under Auto.
    let p = profile_on(&t, path, TlsMode::Auto).expect("sshd-a is in the set");
    let live = connect_ix(p, Interaction::Headless)
        .await
        .unwrap_or_else(|e| panic!("hv_test through SSH: {:?}", e.class));
    live.close().await;
}

// ## m2b-fix-f2-x509 (T13 carry)

/// A file `mysql-x509-certs` generated for `mysql84-x509` at `docker up`
/// (`mysql-x509-<name>`, a Docker artifact).
fn x509_file(name: &str) -> std::path::PathBuf {
    hacivat_testkit::artifact(&format!("mysql-x509-{name}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn mysql_client_certificate_sign_in() {
    // T13 carry: `mysql84-x509`'s `ssl_ca` is a test CA generated at `docker up` (nothing
    // committed), and `hv_x509` is `REQUIRE X509`. `VerifyCa` with that CA checks the server's
    // chain (`CaOnly`, Task 3's `ChainOnly` through patch §5). Without a client certificate the
    // server refuses the account (`1045`); with the one Hacivat reads (PKCS #8, and its
    // encrypted twin opened with the passphrase from the overlay, B25) it signs in, directly.
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("mysql84-x509") else {
        return;
    };
    let profile = |key: Option<&str>| {
        let mut p = profile_for(&t, TlsMode::VerifyCa).user("hv_x509");
        p.tls.ca_file = Some(x509_file("ca.pem"));
        p.tls.client_cert = key.map(|key| ClientCertPaths {
            cert: x509_file("client.pem"),
            key: x509_file(key),
        });
        p
    };

    let refused = connect_ix(profile(None), Interaction::Headless)
        .await
        .err()
        .unwrap_or_else(|| panic!("REQUIRE X509 let hv_x509 in without a client certificate"));
    assert_eq!(
        code_of(refused.server.as_ref()).as_deref(),
        Some("1045"),
        "{:?}",
        refused.class
    );

    let mut plain = connect_ix(profile(Some("client-key.pem")), Interaction::Headless)
        .await
        .unwrap_or_else(|e| panic!("hv_x509 with its certificate: {:?}", e.class));
    assert!(plain.tls.encrypted);
    assert_eq!(plain.tls.verified, TlsVerified::CaOnly);
    assert_eq!(
        scalar(&mut plain, "SELECT CURRENT_USER()").await,
        "hv_x509@%"
    );
    plain.close().await;

    let mut r = req(profile(Some("client-key-enc.pem")));
    r.overlay_secrets = Some(SessionSecrets {
        tls_key_passphrase: Some(SecretString::from(MYSQL_X509_KEY_PASSPHRASE.to_string())),
        ..SessionSecrets::default()
    });
    let mut encrypted = connect(
        &matrix_deps(),
        r,
        Interaction::Headless,
        token(),
        progress(),
    )
    .await
    .unwrap_or_else(|e| panic!("hv_x509 with its encrypted key: {:?}", e.class));
    assert_eq!(encrypted.tls.verified, TlsVerified::CaOnly);
    assert_eq!(
        scalar(&mut encrypted, "SELECT CURRENT_USER()").await,
        "hv_x509@%"
    );
    encrypted.close().await;
}
