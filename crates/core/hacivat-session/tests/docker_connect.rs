//! Engine conformance over Docker (Task 19, exit criteria 1–3): the whole connect pipeline,
//! `test_connection` and the `ConnectionManager` with the real drivers against every server of
//! the M2a matrix. Run with `cargo xtask docker test --set full -p hacivat-session`.
//!
//! `connects_and_detects_every_matrix_target` also writes each target's resolved server
//! version to the Docker artifact `resolved-versions.txt` (`hacivat_testkit::artifact`, so
//! `<main checkout>/target/docker/resolved-versions.txt` under `cargo xtask docker test`), for
//! the gate (Task 22). One line per target, `<id>\t<ServerInfo.raw_version>`, after a
//! `# set: <set>` header; under `--set full` it lists all 13 targets (pg13 is connected with
//! `allow_unsupported_server` only to read its version).

mod docker_support;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use docker_support::*;
use hacivat_driver::Support;
use hacivat_net::{AddressClass, TlsMode};
use hacivat_proto::{DbmsKind, ErrClass};
use hacivat_session::pipeline::{Interaction, TrailStep::*, connect};
use hacivat_session::profile::ProfileDraft;
use hacivat_session::{ConnectionManager, LiveInfo, LiveState, ManagerConfig};

#[test]
fn only_the_full_set_requires_every_target() {
    assert!(must_exist(Some("full")));
    assert!(must_exist(None));
    assert!(!must_exist(Some("pr")));
}

#[tokio::test]
#[ignore = "docker"]
async fn connects_and_detects_every_matrix_target() {
    if !docker_enabled() {
        return;
    }
    let mut resolved = Vec::new();
    for t in matrix() {
        let live = connect(
            &deps(),
            req(profile_for(&t, auto_or_require(&t))),
            Interaction::Headless,
            token(),
            progress(),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.class));
        assert_eq!(live.server.kind, t.engine, "{}", t.id);
        assert_eq!(live.support, Support::Supported, "{}", t.id);
        assert_eq!(
            steps(&live.trail),
            [Resolve, Connect, Tls, SignIn, CheckServer],
            "{}",
            t.id
        );
        // The server line `targets.toml` names: `17` for PostgreSQL, `8.4` or `10.11` for the
        // MySQL family.
        let v = live
            .server
            .version
            .clone()
            .unwrap_or_else(|| panic!("{}: no parsed version", t.id));
        assert!(
            t.major == v.major.to_string() || t.major == format!("{}.{}", v.major, v.minor),
            "{}: {}",
            t.id,
            live.server.raw_version
        );
        resolved.push((t.id.clone(), live.server.raw_version.clone()));
        live.close().await;
    }
    // Outside the matrix: pg13 is below the floor (read with `allow_unsupported_server`) and
    // pg17-tr is pg17 with Turkish server messages.
    for id in ["pg13", "pg17-tr"] {
        let Some(t) = target_required(id) else {
            continue;
        };
        let mut p = profile_for(&t, auto_or_require(&t));
        p.allow_unsupported_server = true;
        // ## m2b-t14-pipeline: pg13 signs in with md5 and no TLS. Under the insecure-auth
        // policy (B8) it connects on the allowed path, deliberately: its published port is on
        // loopback, where the password never crosses a network. The same server on a private
        // address under Auto would be refused (`auth.insecure_method`, pipeline_tls).
        let mode = p.tls.mode;
        assert_eq!(
            hacivat_session::pipeline::insecure_auth_for(
                &p,
                hacivat_net::classify_host(&t.host, &[]),
                mode
            ),
            hacivat_driver::InsecureAuth::Allow,
            "{id}: the matrix reaches its servers on loopback"
        );
        let live = connect(&deps(), req(p), Interaction::Headless, token(), progress())
            .await
            .unwrap_or_else(|e| panic!("{id}: {:?}", e.class));
        assert!(live.server.raw_version.contains(&t.major), "{id}");
        resolved.push((t.id.clone(), live.server.raw_version.clone()));
        live.close().await;
    }
    write_resolved_versions(&resolved);
}

/// Writes the resolved versions to the Docker artifact `resolved-versions.txt` (Step 4; read
/// by the gate).
fn write_resolved_versions(resolved: &[(String, String)]) {
    let path = hacivat_testkit::artifact("resolved-versions.txt");
    let set = std::env::var("HACIVAT_DOCKER_SET").unwrap_or_else(|_| "full".to_string());
    let mut text = format!("# set: {set}\n");
    for (id, version) in resolved {
        text.push_str(&format!("{id}\t{version}\n"));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("the artifact directory");
    }
    std::fs::write(&path, text).expect("resolved-versions.txt is written");
    eprintln!("resolved versions: {}", path.display());
}

#[tokio::test]
#[ignore = "docker"]
async fn sqlite_file_connects() {
    if !docker_enabled() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conformance.db");
    let live = connect(
        &deps(),
        req(sqlite_profile(&path)),
        Interaction::Headless,
        token(),
        progress(),
    )
    .await
    .unwrap_or_else(|e| panic!("sqlite: {:?}", e.class));
    assert_eq!(live.server.kind, DbmsKind::Sqlite);
    assert_eq!(live.support, Support::Supported);
    assert_eq!(steps(&live.trail), [OpenFile, CheckServer]);
    assert!(path.is_file(), "the database file was created");
    live.close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn stream_injection_is_used_for_the_main_connection() {
    if !docker_enabled() {
        return;
    }
    for id in ["pg17", "mysql84"] {
        let Some(t) = target_required(id) else {
            continue;
        };
        // A transport wrap leaves the endpoint without a direct target (P13).
        let (deps, count) = deps_counting();
        // The target's own mode: pg17 serves no TLS, so Require would be refused there
        // (`tls.not_supported_by_server`) before the driver ever ran.
        let live = connect(
            &deps,
            req(profile_for(&t, auto_or_require(&t))),
            Interaction::Headless,
            token(),
            progress(),
        )
        .await
        .unwrap_or_else(|e| panic!("{id}: {:?}", e.class));
        assert!(count.load(Ordering::SeqCst) > 0, "{id}");
        assert!(live.trail.iter().any(|s| s.step == Connect), "{id}");
        // MySQL goes through the TCP loopback forwarder.
        assert_eq!(live.forwarded(), id == "mysql84", "{id}");
        live.close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn plaintext_loopback_gets_the_unencrypted_state() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = target_required("pg17") else {
        return;
    };
    let live = connect_target(&t, TlsMode::Auto).await;
    assert!(!live.tls.encrypted);
    assert_eq!(live.address, AddressClass::Loopback);
    live.close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn test_report_is_complete_for_every_target() {
    if !docker_enabled() {
        return;
    }
    for t in matrix() {
        let r = test_target(&t).await;
        assert!(r.ok, "{}: {:?}", t.id, r.error.as_ref().map(|e| e.class));
        let ping = r.ping_median.unwrap_or_else(|| panic!("{}: no ping", t.id));
        assert!(ping < ms(200), "{}: {ping:?}", t.id);
        assert_eq!(r.user.as_deref(), Some("hv_test"), "{}", t.id);
    }
}

/// Waits until the manager reports `id` connected (or failed), for at most 30 s.
async fn wait_live(
    m: &ConnectionManager,
    id: hacivat_proto::ProfileId,
) -> Result<Arc<LiveInfo>, ErrClass> {
    let mut changes = m.changes();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match m.state(id) {
                LiveState::Connected(info) => return Ok(info),
                LiveState::Failed(e) => return Err(e.class),
                _ => {}
            }
            changes.changed().await.expect("the manager is alive");
        }
    })
    .await
    .expect("the manager settles within 30 s")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn the_manager_connects_tests_and_disconnects_real_servers() {
    if !docker_enabled() {
        return;
    }
    for id in ["pg17", "mysql84"] {
        let Some(t) = target_required(id) else {
            continue;
        };
        let m = ConnectionManager::new(
            tokio::runtime::Handle::current(),
            Arc::new(deps()),
            ManagerConfig::default(),
        );
        let profile = Arc::new(profile_for(&t, auto_or_require(&t)));
        m.connect(
            profile.clone(),
            Some(docker_password()),
            Interaction::Headless,
        );
        let info = wait_live(&m, profile.id)
            .await
            .unwrap_or_else(|class| panic!("{id}: {class:?}"));
        assert_eq!(info.server.kind, t.engine, "{id}");
        assert_eq!(info.support, Support::Supported, "{id}");
        assert_eq!(info.user.as_deref(), Some("hv_test"), "{id}");

        let draft = ProfileDraft {
            password: Some(docker_password()),
            ..ProfileDraft::edit((*profile).clone())
        };
        let report = m
            .test(draft, Interaction::Headless)
            .result
            .await
            .expect("the test reports");
        assert!(report.ok, "{id}: {:?}", report.error.map(|e| e.class));

        m.disconnect(profile.id);
        assert_eq!(m.state(profile.id), LiveState::Disconnected, "{id}");
        m.shutdown().await;
    }
}
