//! M2b conformance over Docker (Task 23; exit criteria E1, E6, E7, E14): every TCP target of
//! the matrix connects directly, through the `sshd-a` bastion and (the `pr` targets, PostgreSQL
//! and MySQL) through the `sshd-a` → `sshd-b` jump chain; two profiles share one SSH session;
//! the unencrypted badge's facts. Run with `cargo xtask docker test --set pr|full -p
//! hacivat-session`.
//!
//! - Targets come only from `require_target`/`targets` (Q6, through Task 19's `matrix`); through
//!   SSH a test dials the target's in-network endpoint (`DockerTarget.internal`).
//! - The bastions' host keys are trusted from their published artifacts (`Strict`), the client
//!   keys are private copies of the generated ones, and no agent is asked.

mod common;

use std::sync::Arc;

use common::conformance::*;
use hacivat_driver::Support;
use hacivat_net::{AddressClass, TlsMode};
use hacivat_session::pipeline::TrailStep::*;
use hacivat_session::{LiveState, PathKind};
use hacivat_testkit::require_target;

/// The paths every TCP target runs on (`alg/01` §5); the chain only for the `pr` targets.
const PATHS: [Path; 3] = [
    Path::Direct,
    Path::Ssh("sshd-a"),
    Path::Chain("sshd-a", "sshd-b"),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn connect_matrix_direct_and_via_ssh() {
    if !docker_enabled() {
        return;
    }
    let mut ran = 0;
    for t in tcp_targets() {
        for path in PATHS {
            if matches!(path, Path::Chain(..)) && !in_pr_set(&t) {
                continue;
            }
            if !path.available() {
                continue;
            }
            let mut live = connect_main(&t, path)
                .await
                .unwrap_or_else(|e| panic!("{} {path:?}: {:?}", t.id, e.class));
            assert_eq!(live.path, path.kind(), "{} {path:?}", t.id);
            assert_eq!(live.server.kind, t.engine, "{} {path:?}", t.id);
            assert_eq!(live.support, Support::Supported, "{} {path:?}", t.id);
            match path {
                Path::Direct => {
                    assert_eq!(
                        steps(&live.trail),
                        [Resolve, Connect, Tls, SignIn, CheckServer],
                        "{}",
                        t.id
                    );
                    assert_eq!(live.via, None, "{}", t.id);
                    assert!(live.transport.is_none(), "{}", t.id);
                    assert_eq!(live.address, AddressClass::Loopback, "{}", t.id);
                }
                Path::Ssh(_) | Path::Chain(..) => {
                    // Resolve is the bastion's job (B11): the database host is never looked
                    // up here. `Disable` (a MySQL-family server without TLS, `mode_on`) has no
                    // TLS step.
                    let expected: &[_] = if mode_on(&t, path) == TlsMode::Disable {
                        &[Ssh, Connect, SignIn, CheckServer]
                    } else {
                        &[Ssh, Connect, Tls, SignIn, CheckServer]
                    };
                    assert_eq!(steps(&live.trail), expected, "{} {path:?}", t.id);
                    let last = match path {
                        Path::Chain(_, b) => require_ssh(b),
                        Path::Ssh(a) => require_ssh(a),
                        Path::Direct => None,
                    }
                    .expect("the path's bastions are in the set");
                    assert_eq!(live.via.as_deref(), Some(last.host.as_str()), "{}", t.id);
                    assert!(live.transport.is_some(), "{} {path:?}", t.id);
                    // A compose service name: the guard classes it private (B11).
                    assert_eq!(live.address, AddressClass::Private, "{} {path:?}", t.id);
                }
            }
            // MySQL and MariaDB reach the tunnel through the loopback forwarder (no stream
            // injection, P13); PostgreSQL rides the channel's stream itself (E14).
            if path != Path::Direct {
                assert_eq!(
                    live.forwarded(),
                    t.engine != hacivat_proto::DbmsKind::Postgres,
                    "{} {path:?}",
                    t.id
                );
            }
            // A round trip over the path.
            assert_eq!(
                scalar(&mut live, "SELECT 1").await,
                "1",
                "{} {path:?}",
                t.id
            );
            live.close().await;
            ran += 1;
        }
    }
    assert!(ran > 0, "no target ran");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn two_profiles_share_one_ssh_session() {
    if !docker_enabled() {
        return;
    }
    let (Some(pg), Some(my)) = (require_target("pg17"), require_target("mysql84")) else {
        return;
    };
    let path = Path::Ssh("sshd-a");
    if !path.available() {
        return;
    }
    let deps = matrix_deps();
    let tunnels = deps.tunnels.clone();
    let m = manager_over(deps);
    let profiles: Vec<Arc<_>> = [&pg, &my]
        .into_iter()
        .map(|t| Arc::new(profile_on(t, path, mode_on(t, path)).expect("sshd-a is in the set")))
        .collect();
    for p in &profiles {
        connect_on(&m, p).await;
    }
    // One chain, one hop: both profiles ride the same SSH session (E7, B11).
    assert_eq!(tunnels.sessions_open(), 1);
    for (p, t) in profiles.iter().zip([&pg, &my]) {
        let LiveState::Connected(info) = m.state(p.id) else {
            panic!("{}: connected", t.id);
        };
        assert_eq!(info.path, PathKind::Ssh, "{}", t.id);
        assert_eq!(info.hops, 1, "{}", t.id);
        let reply = run(&default_session(&m, p.id), "SELECT 1").await;
        assert!(reply.is_ok(), "{}: {:?}", t.id, reply.map_err(|e| e.kind));
    }
    // One profile leaving keeps the session for the other.
    m.disconnect(profiles[0].id);
    assert_eq!(tunnels.sessions_open(), 1);
    let reply = run(&default_session(&m, profiles[1].id), "SELECT 1").await;
    assert!(reply.is_ok(), "{}: {:?}", my.id, reply.map_err(|e| e.kind));
    m.shutdown().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn unencrypted_badge_cases_on_loopback() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    // pg17 serves no TLS: Auto connects in plaintext on loopback, where the password never
    // crosses a network (the badge says unencrypted, no question is asked).
    let live = connect_main(&t, Path::Direct)
        .await
        .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.class));
    assert_eq!(auto_or_require(&t), TlsMode::Auto);
    assert!(!live.tls.encrypted);
    assert_eq!(live.address, AddressClass::Loopback);
    assert_eq!(live.path, PathKind::Direct);
    live.close().await;
    // Through SSH the same server is a private address behind the bastion: still no TLS, and
    // the capsule names the SSH path.
    let path = Path::Ssh("sshd-a");
    if path.available() {
        let live = connect_main(&t, path)
            .await
            .unwrap_or_else(|e| panic!("{} {path:?}: {:?}", t.id, e.class));
        assert!(!live.tls.encrypted);
        assert_eq!(live.address, AddressClass::Private);
        assert_eq!(live.path, PathKind::Ssh);
        live.close().await;
    }
}
