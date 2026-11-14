// ## m3a-t01b-seams
//! Sessions over a [`FakeDriver`] for other crates' tests (feature `test-support`, P6): the
//! run service's (`hacivat-exec`) and the app's (`register_test_console`). Lifted from the
//! session tests' `session`/`ro_session` helpers (`tests/common/mod.rs`): the real pipeline over
//! the fake, with a transport over `tokio::io::duplex` (no socket, no DNS query), an in-memory
//! secret store and side connections (cancel, kill) to a second fake of the same engine.

use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_driver::fake::FakeDriver;
use hacivat_driver::{Driver, DriverRegistry, EngineCatalog};
use hacivat_net::BoxedStream;
use hacivat_net::StreamFactory;
use hacivat_net::ssh::{KnownHosts, TunnelManager};
use hacivat_proto::Department;
use hacivat_secrets::{CredentialFiles, MemoryBackend, SecretBackend, SecretStore};
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::pipeline::{
    ApprovedRoots, ConnectDeps, ConnectError, ConnectRequest, Interaction, LiveConnection, Purpose,
    TransportWrap, connect, fresh_progress,
};
use crate::profile::Profile;
use crate::session::{
    ClientId, SessionHandle, SessionHooks, SessionKind, SessionSpec, SideConnector, spawn_session,
};

/// A transport over `tokio::io::duplex`: no socket is ever opened.
fn duplex_wrap() -> TransportWrap {
    Arc::new(|_direct: StreamFactory| {
        StreamFactory::new(|| {
            Box::pin(async {
                let (near, _far) = tokio::io::duplex(64);
                Ok(Box::new(near) as BoxedStream)
            })
        })
    })
}

/// Connect dependencies with `fake` as the only driver.
fn deps_over(fake: Arc<FakeDriver>) -> ConnectDeps {
    let mut drivers = DriverRegistry::new();
    drivers.register(fake as Arc<dyn Driver>);
    // A known-hosts store whose files do not exist: loading does no I/O (M2b Q42).
    let known_hosts = KnownHosts::load(Vec::new(), PathBuf::from("hacivat-test-known-hosts"));
    let tunnels = TunnelManager::new(known_hosts.clone(), Default::default());
    let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
    ConnectDeps {
        drivers: Arc::new(drivers),
        catalog: EngineCatalog::embedded(),
        secrets: Arc::new(SecretStore::new(Some(backend))),
        files: CredentialFiles {
            pgpass: None,
            pg_service: None,
        },
        transport_wrap: Some(duplex_wrap()),
        extra_steps: Vec::new(),
        approved_roots: ApprovedRoots::for_user(),
        timeout_override: None,
        tunnels,
        known_hosts,
    }
}

fn request(profile: Profile, department: Department) -> ConnectRequest {
    ConnectRequest {
        read_only: profile.read_only,
        profile: Arc::new(profile),
        draft_password: None,
        purpose: Purpose::Main,
        department,
        prior_rejections: 0,
        draft_ssh: Vec::new(),
        overlay_secrets: None,
        plaintext_consent: false,
    }
}

/// Side connections (cancel, kill) through the real pipeline, to a second fake.
struct FakeSide {
    fake: Arc<FakeDriver>,
    profile: Profile,
}

impl SideConnector for FakeSide {
    fn open_side(&self) -> BoxFuture<'static, Result<LiveConnection, ConnectError>> {
        let deps = deps_over(self.fake.clone());
        let req = request(self.profile.clone(), Department::Cancel);
        Box::pin(async move {
            connect(
                &deps,
                req,
                Interaction::Headless,
                CancellationToken::new(),
                tokio::sync::watch::Sender::new(fresh_progress()),
            )
            .await
        })
    }
}

/// A console session (the default client) of `profile` over `fake`, connected and spawned on
/// `rt`; read-only exactly when `profile.read_only`. Returns the handle and the fake, whose
/// counters and logs the test reads. Awaitable from any executor (the connect runs on `rt`).
///
/// # Panics
/// When the connect fails (the profile does not suit the fake: its engine, a password, a file
/// that is missing).
pub async fn fake_session(
    rt: Handle,
    fake: FakeDriver,
    profile: Profile,
) -> (SessionHandle, Arc<FakeDriver>) {
    let kind = fake.descriptor().kind;
    let fake = Arc::new(fake);
    let read_only = profile.read_only;
    let deps = deps_over(fake.clone());
    let req = request(profile.clone(), Department::Console);
    let (tx, rx) = oneshot::channel();
    rt.spawn(async move {
        let live = connect(
            &deps,
            req,
            Interaction::Headless,
            CancellationToken::new(),
            tokio::sync::watch::Sender::new(fresh_progress()),
        )
        .await;
        let _ = tx.send(live);
    });
    let live = rx
        .await
        .expect("the core runtime ran the connect")
        .unwrap_or_else(|e| {
            panic!(
                "the fake session's connect failed: {:?} in {:?}",
                e.class, e.phase
            )
        });
    let hooks = SessionHooks {
        side: Arc::new(FakeSide {
            fake: Arc::new(FakeDriver::new(kind)),
            profile: profile.clone(),
        }),
        reconnect: None,
    };
    let spec = SessionSpec {
        kind: SessionKind::Console(ClientId::DEFAULT),
        read_only,
        department: Department::Console,
    };
    (
        spawn_session(&rt, spec, Arc::new(profile), live, hooks),
        fake,
    )
}
