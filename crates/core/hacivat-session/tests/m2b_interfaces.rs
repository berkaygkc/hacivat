//! M2b Task 1: the interface commit's types, defaults, redaction and working stubs.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::fake::{FakeDriver, FakeExec};
use hacivat_driver::{
    Driver as _, DriverConnect, Endpoint, ExecOpts, ExecSummary, InsecureAuth, RawConn, RawError,
    RawErrorKind, TxStatus,
};
use hacivat_net::ssh::{HopSecrets, HostKeyPolicy, SshSpec, TunnelRequest};
use hacivat_net::{AddressClass, ssh_address_class};
use hacivat_proto::{DbmsKind, ErrClass};
use hacivat_secrets::{FileVault, Retention, SecretString};
use hacivat_session::pipeline::{Answer, SessionSecrets};
use hacivat_session::profile::{Profile, ProfileDraft};
use hacivat_session::session::readonly::run_guarded;
use hacivat_session::session::{ExecErrorKind, IsolationLevel, TxMode};

fn raw(kind: RawErrorKind) -> RawError {
    RawError { kind, server: None }
}

async fn connect_fake(f: &FakeDriver) -> Box<dyn RawConn> {
    f.connect(DriverConnect {
        endpoint: Endpoint::File {
            path: PathBuf::from("m2b.db"),
            create_if_missing: true,
            read_only: false,
        },
        tls: None,
        user: None,
        password: None,
        database: None,
        application_name: "Hacivat".into(),
        read_only: false,
        options: Vec::new(),
        connect_timeout: Duration::from_secs(5),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::default(),
    })
    .await
    .expect("the fake connects")
}

#[test]
fn three_new_error_classes() {
    assert_eq!(ErrClass::ALL.len(), 51);
    for (c, id) in [
        (ErrClass::TlsPinChanged, "tls.pin_changed"),
        (ErrClass::AuthInsecureMethod, "auth.insecure_method"),
        (ErrClass::SshHostKeyRevoked, "ssh.host_key_revoked"),
    ] {
        assert_eq!(c.id(), id);
        assert_eq!(ErrClass::from_id(id), Some(c));
    }
}

/// Answer, HopSecrets, TunnelRequest, ProfileDraft.ssh_secrets, SessionSecrets, FileVault (Q32).
#[test]
fn debug_redaction_m2b() {
    let hop = || HopSecrets {
        password: Some(SecretString::from("s3cret")),
        passphrase: Some(SecretString::from("s3cret")),
    };
    for s in [
        format!("{:?}", Answer::Secret(SecretString::from("s3cret"))),
        format!("{:?}", Answer::Master(SecretString::from("s3cret"))),
        format!("{:?}", Answer::Keyboard(vec![SecretString::from("s3cret")])),
        format!("{:?}", hop()),
        format!(
            "{:?}",
            TunnelRequest {
                spec: SshSpec::default(),
                secrets: vec![hop()],
                share: true,
                connect_timeout: Duration::ZERO,
            }
        ),
        format!(
            "{:?}",
            ProfileDraft {
                ssh_secrets: vec![hop()],
                ..Default::default()
            }
        ),
        format!(
            "{:?}",
            SessionSecrets {
                db: Some(SecretString::from("s3cret")),
                hops: vec![hop()],
                tls_key_passphrase: None,
            }
        ),
        format!(
            "{:?}",
            FileVault::open(PathBuf::from("/nonexistent/vault.bin"))
        ),
    ] {
        assert!(!s.contains("s3cret"), "{s}");
    }
}

#[test]
fn profile_m2b_defaults() {
    let p = Profile::default();
    assert!(!p.single_session);
    assert_eq!(p.tx_mode, None);
    assert_eq!(p.ssh_retention, Retention::Forever);
    assert!(p.tls.pins.is_empty() && !p.tls.allow_public_plaintext);
    assert_eq!(HostKeyPolicy::default(), HostKeyPolicy::Ask);
    assert_eq!(InsecureAuth::default(), InsecureAuth::RefuseUnlessVerified);
    assert_eq!(TxMode::default(), TxMode::Auto);
    assert_eq!(IsolationLevel::default(), IsolationLevel::Default);
}

#[tokio::test]
async fn fake_driver_scripts_exec_and_tx() {
    let f = FakeDriver::new(DbmsKind::Postgres).exec(vec![FakeExec {
        prefix: "BEGIN",
        result: Ok(ExecSummary::default()),
        delay: Duration::ZERO,
        tx_after: Some(TxStatus::InTx),
        panics: false,
    }]);
    let mut c = connect_fake(&f).await;
    c.execute("BEGIN", ExecOpts::default()).await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::InTx);
    c.execute("select 1", ExecOpts::default()).await.unwrap();
    assert_eq!(f.executed(), ["BEGIN", "select 1"]);
}

#[tokio::test]
async fn fake_exec_entries_are_consumed_once() {
    let f = FakeDriver::new(DbmsKind::Postgres).exec(vec![FakeExec {
        prefix: "SELECT x",
        result: Err(raw(RawErrorKind::Io(std::io::ErrorKind::ConnectionReset))),
        delay: Duration::ZERO,
        tx_after: None,
        panics: false,
    }]);
    let mut c = connect_fake(&f).await;
    assert!(c.execute("SELECT x", ExecOpts::default()).await.is_err());
    // Unmatched → Ok, status unchanged.
    assert!(c.execute("SELECT x", ExecOpts::default()).await.is_ok());
    assert_eq!(c.tx_status(), TxStatus::Idle);
}

#[test]
fn ssh_address_class_rule() {
    assert_eq!(ssh_address_class("db.internal"), AddressClass::Private);
    assert_eq!(ssh_address_class("localhost"), AddressClass::Loopback);
    assert_eq!(ssh_address_class("8.8.8.8"), AddressClass::Public);
}

#[tokio::test]
async fn read_only_stub_fails_closed() {
    let f = FakeDriver::new(DbmsKind::Postgres);
    let mut c = connect_fake(&f).await;
    assert_eq!(
        run_guarded(
            &mut *c,
            DbmsKind::Postgres,
            "select 1",
            ExecOpts::default(),
            &[]
        )
        .await
        .unwrap_err()
        .kind,
        ExecErrorKind::Internal
    );
    assert!(f.executed().is_empty(), "nothing ran unguarded");
}

/// Fix round 1 (I1): `ConnectRequest.read_only` (profile OR session) is what the driver gets,
/// on the request and on a SQLite file endpoint, even for a read-write profile.
#[tokio::test]
async fn a_read_only_session_reaches_the_driver_read_only() {
    use hacivat_driver::fake::FakeEndpoint;
    use hacivat_driver::{DriverRegistry, EngineCatalog};
    use hacivat_proto::Department;
    use hacivat_secrets::{CredentialFiles, MemoryBackend, SecretStore};
    use hacivat_session::pipeline::{
        ApprovedRoots, ConnectDeps, ConnectRequest, Interaction, Progress, Purpose, connect,
    };
    use hacivat_session::profile::ProfileEndpoint;
    use tokio_util::sync::CancellationToken;

    let fake = Arc::new(FakeDriver::new(DbmsKind::Sqlite));
    let mut drivers = DriverRegistry::new();
    drivers.register(fake.clone());
    let known_hosts = hacivat_net::ssh::KnownHosts::load(
        Vec::new(),
        PathBuf::from("/nonexistent/hacivat/known_hosts"),
    );
    let deps = ConnectDeps {
        drivers: Arc::new(drivers),
        catalog: EngineCatalog::embedded(),
        secrets: Arc::new(SecretStore::new(Some(Arc::new(MemoryBackend::new())))),
        files: CredentialFiles {
            pgpass: None,
            pg_service: None,
        },
        transport_wrap: None,
        extra_steps: Vec::new(),
        approved_roots: ApprovedRoots::for_user(),
        timeout_override: None,
        tunnels: hacivat_net::ssh::TunnelManager::new(known_hosts.clone(), Default::default()),
        known_hosts,
    };
    for (session_read_only, want) in [(true, true), (false, false)] {
        let profile = Profile {
            name: "ro-session".into(),
            engine: DbmsKind::Sqlite,
            endpoint: ProfileEndpoint::File {
                path: PathBuf::from(":memory:"),
                create_if_missing: false,
            },
            read_only: false,
            ..Profile::default()
        };
        let req = ConnectRequest {
            profile: Arc::new(profile),
            draft_password: None,
            purpose: Purpose::Main,
            department: Department::Console,
            prior_rejections: 0,
            read_only: session_read_only,
            draft_ssh: Vec::new(),
            overlay_secrets: None,
            plaintext_consent: false,
        };
        let progress = tokio::sync::watch::Sender::new(Progress {
            trail: Vec::new(),
            current: None,
            started: std::time::Instant::now(),
        });
        let live = connect(
            &deps,
            req,
            Interaction::Headless,
            CancellationToken::new(),
            progress,
        )
        .await
        .expect("the fake connects");
        let sent = fake.last_request().expect("a request");
        assert_eq!(sent.read_only, want, "request, session {session_read_only}");
        assert!(
            matches!(sent.endpoint, Some(FakeEndpoint::File { read_only, .. }) if read_only == want),
            "endpoint, session {session_read_only}: {:?}",
            sent.endpoint
        );
        live.close().await;
    }
}

/// `cargo metadata` for the workspace, as JSON text. (The brief's pseudo-code uses the
/// `cargo_metadata` crate; the manifest table adds no such dev-dependency, so this reads the
/// JSON text.)
fn cargo_metadata() -> String {
    let out = std::process::Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--offline"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata runs");
    assert!(out.status.success(), "cargo metadata failed");
    String::from_utf8(out.stdout).expect("UTF-8 metadata")
}

#[test]
fn mysql_async_is_the_vendored_copy() {
    let md = cargo_metadata().replace("\\\\", "/");
    // The package entry (a dependency entry has a `req` after its name, not a `version`); its
    // dependencies come before its own `manifest_path`, and they have none.
    let start = md
        .find("{\"name\":\"mysql_async\",\"version\"")
        .expect("mysql_async is in the dependency graph");
    let manifest = md[start..]
        .split("\"manifest_path\":\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a manifest path");
    assert!(manifest.contains("third_party/mysql_async"), "{manifest}");
    let members = md
        .split("\"workspace_members\":[")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("the workspace members");
    assert!(!members.contains("mysql_async"), "{members}");
}
