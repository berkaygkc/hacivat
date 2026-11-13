//! M2b Task 22's core seams for the live-state UI:
//! - a pooled read-only session's enforcement gap reaches `ConnectionManager::events()` (T17
//!   carry: the manager's monitor watches only the default session), once;
//! - a vault that fails its integrity check during a connect asks
//!   `Question::VaultDamaged` once (the T21 Damaged notice), then the password as if nothing
//!   were stored.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use hacivat_driver::TxStatus;
use hacivat_net::TlsMode;
use hacivat_secrets::{FileVault, SecretBackend, SecretStore, VaultBackend, VaultParams};
use hacivat_session::pipeline::{Answer, Question, connect};
use hacivat_session::session::{ExecErrorKind, SessionKind};
use hacivat_session::{ConnectionEvent, GapReason, LiveState, ManagerConfig};
use tokio::sync::broadcast;

/// The next `EnforcementGap` event, within 5 s.
async fn next_gap(
    ev: &mut broadcast::Receiver<ConnectionEvent>,
) -> (hacivat_proto::ProfileId, GapReason) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ev.recv().await {
                Ok(ConnectionEvent::EnforcementGap { profile, reason }) => {
                    return (profile, reason);
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => panic!("the manager is gone"),
            }
        }
    })
    .await
    .expect("an enforcement gap event")
}

/// How many `EnforcementGap` events are queued now.
fn queued_gaps(ev: &mut broadcast::Receiver<ConnectionEvent>) -> usize {
    let mut n = 0;
    loop {
        match ev.try_recv() {
            Ok(ConnectionEvent::EnforcementGap { .. }) => n += 1,
            Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(_) => return n,
        }
    }
}

/// A PostgreSQL fake whose guard opens its transaction and whose `SELECT 1; COMMIT` ends it:
/// the guard's gap (Q35).
fn gap_fake() -> hacivat_driver::fake::FakeDriver {
    ok_pg().exec(vec![
        tx("BEGIN ISOLATION LEVEL", TxStatus::InTx),
        tx("SELECT 1; COMMIT", TxStatus::Idle),
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pooled_read_only_sessions_gap_is_relayed_once() {
    let (m, _) = manager_with(gap_fake(), ManagerConfig::m2b());
    let profile = Arc::new(ro(pg_profile(TlsMode::Disable)));
    let mut ev = m.events();
    m.connect(
        profile.clone(),
        None,
        hacivat_session::pipeline::Interaction::Headless,
    );
    wait_connected(&m, profile.id).await;
    let sessions = m.sessions(profile.id).expect("connected");
    let pooled = sessions
        .pooled(SessionKind::Service)
        .await
        .expect("a pooled session");
    let refused = pooled
        .submit(exec("SELECT 1; COMMIT"))
        .reply
        .await
        .expect("a reply")
        .expect_err("the guard stops it");
    assert_eq!(
        refused.kind,
        ExecErrorKind::EnforcementGap(GapReason::TransactionEndedByStatement)
    );
    let (id, reason) = next_gap(&mut ev).await;
    assert_eq!(id, profile.id);
    assert_eq!(reason, GapReason::TransactionEndedByStatement);
    // The pooled session broke; the profile stays connected on its default session.
    settle().await;
    assert!(matches!(m.state(profile.id), LiveState::Connected(_)));
    assert_eq!(queued_gaps(&mut ev), 0, "relayed once");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_default_sessions_gap_is_still_relayed_once() {
    let (m, _) = manager_with(gap_fake(), ManagerConfig::m2b());
    let profile = Arc::new(ro(pg_profile(TlsMode::Disable)));
    let mut ev = m.events();
    m.connect(
        profile.clone(),
        None,
        hacivat_session::pipeline::Interaction::Headless,
    );
    wait_connected(&m, profile.id).await;
    let default = m
        .sessions(profile.id)
        .and_then(|s| s.default_session())
        .expect("the default session");
    let _ = default.submit(exec("SELECT 1; COMMIT")).reply.await;
    let (id, _) = next_gap(&mut ev).await;
    assert_eq!(id, profile.id);
    let mut changes = m.changes();
    while !matches!(m.state(profile.id), LiveState::Lost { .. }) {
        tokio::time::timeout(Duration::from_secs(5), changes.changed())
            .await
            .expect("the profile ends Lost")
            .expect("the manager runs");
    }
    settle().await;
    assert_eq!(queued_gaps(&mut ev), 0, "the monitor's relay only");
}

// ---------------------------------------------------------------------------------------------
// The damaged vault (T21 carry)

fn fast() -> VaultParams {
    VaultParams {
        m_kib: 8,
        t: 1,
        p: 1,
    }
}

/// The kind of every question asked, in order.
fn kinds(ui: &ScriptedUiM2b) -> Vec<&'static str> {
    ui.asked()
        .iter()
        .map(|q| match q {
            Question::UnlockVault { .. } => "unlock",
            Question::VaultDamaged => "damaged",
            Question::Password { .. } => "password",
            _ => "other",
        })
        .collect()
}

#[tokio::test]
async fn a_damaged_locked_vault_shows_the_notice_once_then_asks_the_password() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vault.bin");
    std::fs::write(&path, b"this is not a vault, it only looks like a file").expect("write");
    let vault = Arc::new(FileVault::with_params(path, fast()));
    let store = Arc::new(SecretStore::new(Some(vault as Arc<dyn SecretBackend>)));
    let deps = deps_with_store(ok_pg(), store);
    let ui = ScriptedUi::answers_m2b([
        Answer::Master(sec("correct horse")),
        Answer::Cancelled,
        Answer::Password(sec("hv_test_pw")),
    ]);
    connect(&deps, req(forever_profile()), ui.ix(), token(), progress())
        .await
        .expect("the typed password connects");
    assert_eq!(kinds(&ui), ["unlock", "damaged", "password"]);
}

#[tokio::test]
async fn a_vault_damaged_while_unlocked_shows_the_notice_then_asks_the_password() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vault.bin");
    let vault = Arc::new(FileVault::with_params(path.clone(), fast()));
    vault.create(sec("correct horse")).expect("create");
    vault.put(&db_key(pid()), &sec("hv_test_pw")).expect("put");
    // The file is damaged under the unlocked vault (a disk error, a bad restore).
    let mut bytes = std::fs::read(&path).expect("read");
    // A changed nonce makes the held key reopen the body, which then fails its tag.
    bytes[40] ^= 0xFF;
    std::fs::write(&path, &bytes).expect("write");
    let store = Arc::new(SecretStore::new(Some(
        vault.clone() as Arc<dyn SecretBackend>
    )));
    let deps = deps_with_store(ok_pg(), store);
    let ui = ScriptedUi::answers_m2b([Answer::Cancelled, Answer::Password(sec("hv_test_pw"))]);
    connect(&deps, req(forever_profile()), ui.ix(), token(), progress())
        .await
        .expect("the typed password connects");
    assert_eq!(kinds(&ui), ["damaged", "password"]);
    assert!(!vault.is_unlocked(), "a damaged file locks the vault");
}

#[tokio::test]
async fn a_headless_connect_never_asks_about_a_damaged_vault() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vault.bin");
    std::fs::write(&path, b"garbage").expect("write");
    let vault = Arc::new(FileVault::with_params(path, fast()));
    let store = Arc::new(SecretStore::new(Some(vault as Arc<dyn SecretBackend>)));
    let deps = deps_with_store(ok_pg(), store);
    let e = connect(
        &deps,
        req(forever_profile()),
        hacivat_session::pipeline::Interaction::Headless,
        token(),
        progress(),
    )
    .await
    .expect_err("nothing to sign in with");
    assert!(e.needs_interaction);
}
