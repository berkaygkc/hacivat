//! M4 Task 1c seams of the session crate: the profile's `catalog_sensitive` flag (R-M4-15) and
//! the fail-closed metadata connection stubs (R-M4-7, R-M4-29; bodies Task 5).
//!
//! Profiles live in `connections.toml` (`profile/store.rs`): the M2a fixture file and the
//! M2b/M3 files below load and save byte for byte (P-M4-23; the precedent is
//! `m2a_file_round_trips_verbatim`, `tests/profiles_m2b.rs`).

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::{deps_with, ok_pg, pg_profile, session};
use hacivat_net::TlsMode;
use hacivat_session::budget_for;
use hacivat_session::meta::{MetaConn, MetaConnSpec, MetaError};
use hacivat_session::profile::{Profile, ProfileDiag, ProfileStore};
use hacivat_session::session::SessionManager;
use tokio_util::sync::CancellationToken;

/// An M2b `connections.toml`: an SSH profile with every M2b key away from its default.
const M2B_FILE: &str = "version = 1\n\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"orders\"\nengine = \"postgres\"\nread_only = false\nhost = \"10.0.0.5\"\nport = 5433\nuser = \"deploy\"\nretention = \"forever\"\ntls.mode = \"verify_full\"\ntls.pins = [{ endpoint = \"10.0.0.5:5433\", sha256 = \"0707070707070707070707070707070707070707070707070707070707070707\" }]\ntransport = \"ssh_tunnel\"\nconnect_timeout_s = 15\napplication_name = \"Hacivat\"\nmax_connections = 4\norigin = \"user_typed\"\nallow_unsupported_server = false\nsingle_session = true\ntx_mode = \"manual\"\nisolation = \"serializable\"\nidle_disconnect_s = 900\nssh_retention = \"until_restart\"\n\n[connection.ssh]\nhost_keys = \"strict\"\nkeepalive_s = 30\n\n[[connection.ssh.hops]]\nhost = \"bastion.internal\"\nport = 2222\nuser = \"deploy\"\nauth = \"agent\"\n";

/// An M3b `connections.toml`: the three M3b keys away from their defaults.
const M3B_FILE: &str = "version = 1\n\n[[connection]]\nid = \"7f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"shop\"\nengine = \"mysql\"\nread_only = false\nhost = \"127.0.0.1\"\nretention = \"forever\"\ntls.mode = \"auto\"\ntransport = \"direct\"\nconnect_timeout_s = 15\napplication_name = \"Hacivat\"\nmax_connections = 4\norigin = \"user_typed\"\nallow_unsupported_server = false\nstartup_script = \"SET time_zone = '+00:00'\"\nread_only_fn_allowlist = [\"audit.mask_email\"]\nrecord_history = false\n";

fn tmp_file(text: &str) -> (PathBuf, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("connections.toml");
    std::fs::write(&path, text).expect("write the file");
    (path, dir)
}

fn read(path: &PathBuf) -> String {
    std::fs::read_to_string(path).expect("read the file")
}

/// Loads `text`, saves what loaded back and returns the profiles and the saved file.
fn load_and_resave(text: &str) -> (Vec<Profile>, String) {
    let (path, _dir) = tmp_file(text);
    let (store, loaded) = ProfileStore::open(path.clone());
    assert!(loaded.diags.is_empty(), "{:?}", loaded.diags);
    store.save(&loaded.profiles).expect("save");
    (loaded.profiles, read(&path))
}

#[test]
fn profile_without_catalog_sensitive_round_trips_byte_for_byte() {
    for (name, text) in [
        ("m2a", include_str!("fixtures/m2a_connections.toml")),
        ("m2b", M2B_FILE),
        ("m3b", M3B_FILE),
    ] {
        let (profiles, saved) = load_and_resave(text);
        assert_eq!(saved, text, "{name}");
        assert!(!profiles.is_empty(), "{name}");
        assert!(profiles.iter().all(|p| !p.catalog_sensitive), "{name}");
        assert!(!saved.contains("catalog_sensitive"), "{name}");
    }
    assert!(!Profile::default().catalog_sensitive);
}

#[test]
fn catalog_sensitive_true_is_written_and_read() {
    let (path, _dir) = tmp_file(M3B_FILE);
    let (store, loaded) = ProfileStore::open(path.clone());
    let mut p = loaded.profiles[0].clone();
    p.catalog_sensitive = true;
    store.save(std::slice::from_ref(&p)).expect("save");
    let text = read(&path);
    assert_eq!(text, format!("{M3B_FILE}catalog_sensitive = true\n"));
    let (_, back) = ProfileStore::open(path.clone());
    assert!(back.diags.is_empty(), "{:?}", back.diags);
    assert_eq!(back.profiles, vec![p.clone()]);

    // Back to the default: the key goes, and the file is the M3b file again.
    p.catalog_sensitive = false;
    store.save(std::slice::from_ref(&p)).expect("save");
    assert_eq!(read(&path), M3B_FILE);

    // A value that is not a boolean never loads half-read: the entry is invalid.
    let (_, bad) =
        ProfileStore::open(tmp_file(&format!("{M3B_FILE}catalog_sensitive = \"yes\"\n")).0);
    assert!(bad.profiles.is_empty());
    assert!(
        matches!(&bad.diags[..], [ProfileDiag::InvalidEntry { reason, .. }] if reason.contains("catalog_sensitive")),
        "{:?}",
        bad.diags
    );
}

fn spec(p: &Profile) -> MetaConnSpec {
    MetaConnSpec {
        profile: p.id,
        database: Some("hv_test".into()),
        statement_timeout: Duration::from_secs(30),
    }
}

#[tokio::test(start_paused = true)]
async fn meta_conn_stub_is_unavailable() {
    let (deps, fake) = deps_with(ok_pg());
    let profile = Arc::new(pg_profile(TlsMode::Disable));
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        profile.clone(),
        Arc::new(deps),
        Arc::new(budget_for(&profile)),
    );
    let cancel = CancellationToken::new();
    let got = sm.meta_conn(spec(&profile), &cancel).await;
    assert!(matches!(got, Err(MetaError::Unavailable)), "{got:?}");
    // Fail closed without touching the server: no connection was opened.
    assert_eq!(fake.connect_calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn meta_join_stub_is_unavailable() {
    let (handle, fake) = session(ok_pg()).await;
    let connects = fake.connect_calls();
    let got = MetaConn::join(&handle);
    assert!(matches!(got, Err(MetaError::Unavailable)), "{got:?}");
    // Joining sends nothing to the session's connection and opens no other.
    assert_eq!(fake.connect_calls(), connects);
}
