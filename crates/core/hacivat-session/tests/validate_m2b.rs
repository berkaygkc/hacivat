//! Validate in M2b (Task 12; B10, B26): SSH hop issues, VerifyCa for MySQL and MariaDB, the
//! legacy encrypted TLS key, and the approved-roots check on canonical paths (Q22).

// Unix-only tests (file modes, Unix sockets, ssh-agent) leave their helpers unused on other
// systems; the lints still run in full on Unix.
#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

mod common;

use std::path::{Path, PathBuf};

use common::{mariadb_profile, mysql_profile, pg_profile};
use hacivat_driver::EngineCatalog;
use hacivat_net::ssh::{HopSpec, SshAuth, SshSpec};
use hacivat_net::{ClientCertPaths, TlsMode};
use hacivat_proto::{DbmsKind, ImportSource, Origin};
use hacivat_session::pipeline::{ApprovedRoots, IssueCode, validate, validate_with};
use hacivat_session::profile::{Profile, ProfileEndpoint, TransportSpec};

// ---- helpers ---------------------------------------------------------------------------------

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

fn hop(host: &str, user: &str, auth: SshAuth) -> HopSpec {
    HopSpec {
        host: host.into(),
        port: 22,
        user: user.into(),
        auth,
        identities_only: false,
        identity_files: Vec::new(),
    }
}

fn key_file(path: &str) -> SshAuth {
    SshAuth::KeyFile { path: path.into() }
}

fn ssh_profile(hops: Vec<HopSpec>) -> Profile {
    Profile {
        transport: TransportSpec::SshTunnel(SshSpec {
            hops,
            ..Default::default()
        }),
        ..pg_profile(TlsMode::Disable)
    }
}

fn karagoz_profile() -> Profile {
    Profile {
        transport: TransportSpec::Karagoz {
            org: "acme".into(),
            agent_id: "a1".into(),
            resource_id: "r1".into(),
        },
        ..pg_profile(TlsMode::Disable)
    }
}

fn codes(p: &Profile) -> Vec<IssueCode> {
    validate(p, EngineCatalog::embedded())
        .into_iter()
        .map(|i| i.code)
        .collect()
}

fn codes_with_roots(p: &Profile, roots: &[&Path]) -> Vec<IssueCode> {
    let roots = ApprovedRoots::new(roots.iter().map(|r| r.to_path_buf()).collect());
    validate_with(p, EngineCatalog::embedded(), &roots)
        .into_iter()
        .map(|i| i.code)
        .collect()
}

fn imported(mut p: Profile) -> Profile {
    p.origin = Origin::Imported {
        source: ImportSource::Url,
    };
    p
}

fn pg_with_ca(ca: &Path) -> Profile {
    let mut p = pg_profile(TlsMode::VerifyFull);
    p.tls.ca_file = Some(ca.to_path_buf());
    p
}

fn sqlite_at(path: &Path) -> Profile {
    Profile {
        engine: DbmsKind::Sqlite,
        endpoint: ProfileEndpoint::File {
            path: path.to_path_buf(),
            create_if_missing: true,
        },
        max_connections: 1,
        ..pg_profile(TlsMode::Auto)
    }
}

const CA: &str = include_str!("fixtures/test-ca.pem");

// ---- SSH -------------------------------------------------------------------------------------

#[test]
fn ssh_issues() {
    let mut p = ssh_profile(vec![hop("", "", SshAuth::Password)]);
    let found = codes(&p);
    assert!(
        found.contains(&IssueCode::SshHostMissing) && found.contains(&IssueCode::SshUserMissing),
        "{found:?}"
    );
    // A blank (whitespace) host or user is missing too.
    p.transport = TransportSpec::SshTunnel(SshSpec {
        hops: vec![hop("  ", "\t", SshAuth::Agent)],
        ..Default::default()
    });
    let found = codes(&p);
    assert!(
        found.contains(&IssueCode::SshHostMissing) && found.contains(&IssueCode::SshUserMissing),
        "{found:?}"
    );
    // No hops at all: there is no bastion.
    p.transport = TransportSpec::SshTunnel(SshSpec::default());
    assert!(codes(&p).contains(&IssueCode::SshHostMissing));

    p.transport = TransportSpec::SshTunnel(SshSpec {
        hops: vec![hop("h", "u", SshAuth::Agent); 10],
        ..Default::default()
    });
    assert!(codes(&p).contains(&IssueCode::SshTooManyHops));
    // Nine hops is the most (`MAX_SSH_HOP` is the last hop index with a secret slot).
    p.transport = TransportSpec::SshTunnel(SshSpec {
        hops: vec![hop("h", "u", SshAuth::Agent); 9],
        ..Default::default()
    });
    let found = codes(&p);
    assert!(!found.contains(&IssueCode::SshTooManyHops), "{found:?}");
    for code in [
        IssueCode::SshHostMissing,
        IssueCode::SshUserMissing,
        IssueCode::SshKeyFileMissing,
    ] {
        assert!(!found.contains(&code), "{code:?}");
    }
}

/// Fix round 1, I1: a hop's host and port take the database host's rules. (m2b-t14-pipeline,
/// named exception: an SSH tunnel is no longer `TransportUnavailable`; the SSH step exists.)
#[test]
fn hop_hosts_and_ports_take_the_database_host_rules() {
    for bad in [
        "u:hv_pw@bastion",
        "b c",
        "b\u{7}x",
        "b/x",
        "b?x",
        "b#x",
        r"b\x",
    ] {
        let found = codes(&ssh_profile(vec![hop(bad, "u", SshAuth::Agent)]));
        assert!(
            found.contains(&IssueCode::HostInvalid),
            "{bad:?}: {found:?}"
        );
        assert!(!found.contains(&IssueCode::TransportUnavailable), "{bad:?}");
        assert!(!found.contains(&IssueCode::SshHostMissing), "{bad:?}");
    }
    let port_zero = HopSpec {
        port: 0,
        ..hop("bastion", "u", SshAuth::Agent)
    };
    let found = codes(&ssh_profile(vec![port_zero]));
    assert!(found.contains(&IssueCode::PortInvalid), "{found:?}");
    assert!(!found.contains(&IssueCode::TransportUnavailable));
    // One bad hop among good ones is still found.
    let found = codes(&ssh_profile(vec![
        hop("jump.acme.io", "u", SshAuth::Agent),
        hop("u:hv_pw@bastion", "u", SshAuth::Agent),
    ]));
    assert!(found.contains(&IssueCode::HostInvalid), "{found:?}");
    // Valid hops: no false positives.
    for good in ["bastion.acme.io", "10.0.0.1", "[::1]", "::1", " bastion "] {
        let p = ssh_profile(vec![
            HopSpec {
                port: 2222,
                ..hop(good, "u", SshAuth::Agent)
            },
            hop("b2", "u", SshAuth::Password),
        ]);
        assert!(codes(&p).is_empty(), "{good:?}");
    }
}

/// Fix round 1, M1: a `~` path is read where Validate checked it (`local_path`), never as a
/// folder named `~` under the working directory.
#[test]
fn tilde_paths_are_checked_and_read_as_the_home_folder() {
    use hacivat_net::paths::fs_probe;
    let Some(home) = std::env::home_dir() else {
        return;
    };
    fs_probe::enable();
    let ca = "hv-t12-tilde-probe-missing-ca.pem";
    let key = "hv-t12-tilde-probe-missing-key";
    let mut p = pg_profile(TlsMode::VerifyFull);
    p.tls.ca_file = Some(Path::new("~").join(ca));
    assert!(codes(&p).contains(&IssueCode::TlsCaUnreadable));
    let p = ssh_profile(vec![hop(
        "b",
        "u",
        key_file(Path::new("~").join(key).to_str().unwrap()),
    )]);
    assert!(codes(&p).contains(&IssueCode::SshKeyFileMissing));
    let touched = fs_probe::touched();
    for name in [ca, key] {
        assert!(touched.contains(&home.join(name)), "{name}: {touched:?}");
        assert!(
            !touched.iter().any(|t| t.starts_with("~")),
            "a literal ~ path was touched: {touched:?}"
        );
    }
}

#[test]
fn ssh_key_file_must_name_an_existing_file() {
    let dir = tempdir();
    let key = dir.path().join("id_ed25519");
    std::fs::write(&key, "k").unwrap();
    let with_key = |path: &str| ssh_profile(vec![hop("b", "u", key_file(path))]);
    assert!(!codes(&with_key(key.to_str().unwrap())).contains(&IssueCode::SshKeyFileMissing));
    for missing in [
        String::new(),
        dir.path().join("nope").display().to_string(),
        // A folder is not a key file.
        dir.path().display().to_string(),
        // A relative path names no file the tunnel can find.
        "id_ed25519".to_string(),
    ] {
        assert!(
            codes(&with_key(&missing)).contains(&IssueCode::SshKeyFileMissing),
            "{missing:?}"
        );
    }
    // Only the key file auth needs one.
    assert!(
        !codes(&ssh_profile(vec![hop("b", "u", SshAuth::Agent)]))
            .contains(&IssueCode::SshKeyFileMissing)
    );
}

/// A key file (or an imported `IdentityFile`) on a share or a device is refused before any file
/// system call, for every origin.
#[test]
fn ssh_key_on_a_share_is_refused_without_touching_it() {
    use hacivat_net::paths::fs_probe;
    fs_probe::enable();
    for bad in [
        r"\\attacker-t12probe\s\id",
        "//t12probe/s/id",
        r"\\.\pipe\t12probe",
    ] {
        let mut by_key = ssh_profile(vec![hop("b", "u", key_file(bad))]);
        assert!(codes(&by_key).contains(&IssueCode::PathRefused), "{bad}");
        by_key = imported(by_key);
        assert!(codes(&by_key).contains(&IssueCode::PathRefused), "{bad}");
        let mut listed = hop("b", "u", SshAuth::Agent);
        listed.identity_files = vec![PathBuf::from(bad)];
        assert!(
            codes(&ssh_profile(vec![listed])).contains(&IssueCode::PathRefused),
            "{bad}"
        );
    }
    let touched = fs_probe::touched();
    assert!(
        !touched
            .iter()
            .any(|t| t.to_string_lossy().contains("t12probe")),
        "{touched:?}"
    );
}

#[test]
fn karagoz_still_transport_unavailable() {
    assert!(codes(&karagoz_profile()).contains(&IssueCode::TransportUnavailable));
}

// ---- TLS -------------------------------------------------------------------------------------

#[test]
fn mysql_verify_ca_is_accepted_now() {
    assert!(validate(&mysql_profile(TlsMode::VerifyCa), EngineCatalog::embedded()).is_empty());
    assert!(
        validate(
            &mariadb_profile(TlsMode::VerifyCa),
            EngineCatalog::embedded()
        )
        .is_empty()
    );
    // As for PostgreSQL, VerifyCa without a CA file trusts nothing.
    let mut p = mysql_profile(TlsMode::VerifyCa);
    p.tls.ca_file = None;
    assert_eq!(codes(&p), [IssueCode::TlsCaUnreadable]);
}

#[test]
fn a_legacy_encrypted_client_key_is_its_own_issue() {
    let dir = tempdir();
    let cert = dir.path().join("client.pem");
    let key = dir.path().join("client.key");
    std::fs::write(&cert, CA).unwrap();
    std::fs::write(
        &key,
        "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\n\
         DEK-Info: AES-128-CBC,00000000000000000000000000000000\n\nAAAA\n\
         -----END RSA PRIVATE KEY-----\n",
    )
    .unwrap();
    let mut p = pg_profile(TlsMode::Require);
    p.tls.client_cert = Some(ClientCertPaths { cert, key });
    assert_eq!(codes(&p), [IssueCode::TlsKeyLegacyEncrypted]);
}

// ---- approved roots on canonical paths (B26, Q22) -------------------------------------------

#[cfg(unix)]
#[test]
fn canonicalize_before_roots_rejects_symlink_escape() {
    let home = tempdir();
    let link = home.path().join("ca.pem");
    std::os::unix::fs::symlink("/etc/hosts", &link).unwrap(); // Q5
    let p = imported(pg_with_ca(&link));
    assert!(codes_with_roots(&p, &[home.path()]).contains(&IssueCode::PathRefused));
    // The same for an SSH key and a SQLite file.
    let key = imported(ssh_profile(vec![hop(
        "b",
        "u",
        key_file(link.to_str().unwrap()),
    )]));
    assert!(codes_with_roots(&key, &[home.path()]).contains(&IssueCode::PathRefused));
    let db = home.path().join("app.db");
    std::os::unix::fs::symlink("/etc/hosts", &db).unwrap();
    assert!(
        codes_with_roots(&imported(sqlite_at(&db)), &[home.path()])
            .contains(&IssueCode::PathRefused)
    );
}

/// A dangling symlink inside a root is refused: SQLite would create the file where it points.
#[cfg(unix)]
#[test]
fn a_dangling_symlink_inside_a_root_is_refused() {
    let home = tempdir();
    let outside = tempdir();
    let db = home.path().join("new.sqlite");
    std::os::unix::fs::symlink(outside.path().join("created-here.sqlite"), &db).unwrap();
    assert!(
        codes_with_roots(&imported(sqlite_at(&db)), &[home.path()])
            .contains(&IssueCode::PathRefused)
    );
}

/// A symlinked root is canonicalised as well, so a file under its real path is inside.
#[cfg(unix)]
#[test]
fn a_symlinked_root_is_canonicalised() {
    let real = tempdir();
    let links = tempdir();
    let root = links.path().join("home");
    std::os::unix::fs::symlink(real.path(), &root).unwrap();
    let ca = real.path().join("ca.pem");
    std::fs::write(&ca, CA).unwrap();
    assert_eq!(codes_with_roots(&imported(pg_with_ca(&ca)), &[&root]), []);
    // And through the link.
    assert_eq!(
        codes_with_roots(&imported(pg_with_ca(&root.join("ca.pem"))), &[real.path()]),
        []
    );
}

#[test]
fn imported_ssh_key_outside_roots_refused() {
    let outside = tempdir();
    let key = outside.path().join("id_ed25519");
    std::fs::write(&key, "k").unwrap(); // portable (Q5)
    let home = tempdir();
    let p = imported(ssh_profile(vec![hop(
        "b",
        "u",
        key_file(key.to_str().unwrap()),
    )]));
    assert!(codes_with_roots(&p, &[home.path()]).contains(&IssueCode::PathRefused));
    // Imported `IdentityFile`s take the same rule.
    let mut listed = hop("b", "u", SshAuth::Agent);
    listed.identity_files = vec![key.clone()];
    let p = imported(ssh_profile(vec![listed]));
    assert!(codes_with_roots(&p, &[home.path()]).contains(&IssueCode::PathRefused));
    // Inside a root it is fine; a profile the user typed is never root-checked.
    let p = imported(ssh_profile(vec![hop(
        "b",
        "u",
        key_file(key.to_str().unwrap()),
    )]));
    assert!(!codes_with_roots(&p, &[outside.path()]).contains(&IssueCode::PathRefused));
    let mut typed = p.clone();
    typed.origin = Origin::UserTyped;
    assert!(!codes_with_roots(&typed, &[home.path()]).contains(&IssueCode::PathRefused));
}

/// A path that leaves the root through `..` is outside it, though its text starts with the root.
#[test]
fn dot_dot_out_of_a_root_is_refused() {
    let home = tempdir();
    let outside = tempdir();
    let ca = outside.path().join("ca.pem");
    std::fs::write(&ca, CA).unwrap();
    let sneaky = home
        .path()
        .join("..")
        .join(outside.path().file_name().unwrap())
        .join("ca.pem");
    assert!(
        codes_with_roots(&imported(pg_with_ca(&sneaky)), &[home.path()])
            .contains(&IssueCode::PathRefused)
    );
}

/// A TLS file outside the roots is refused without being read (it could be a FIFO).
#[test]
fn a_tls_file_outside_the_roots_is_never_read() {
    let home = tempdir();
    let outside = tempdir();
    let ca = outside.path().join("not-a-ca.pem");
    std::fs::write(&ca, "garbage").unwrap();
    assert_eq!(
        codes_with_roots(&imported(pg_with_ca(&ca)), &[home.path()]),
        [IssueCode::PathRefused]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn canonical_root_on_macos_temp_is_accepted() {
    let root = tempdir(); // /var/folders → /private/var (Q22)
    assert!(root.path().starts_with("/var"), "{}", root.path().display());
    let f = root.path().join("ca.pem");
    std::fs::write(&f, CA).unwrap();
    assert!(
        !codes_with_roots(&imported(pg_with_ca(&f)), &[root.path()])
            .contains(&IssueCode::PathRefused)
    );
    // Either spelling of the root accepts either spelling of the file.
    let private = Path::new("/private").join(root.path().strip_prefix("/").unwrap());
    assert!(
        !codes_with_roots(&imported(pg_with_ca(&f)), &[&private]).contains(&IssueCode::PathRefused)
    );
    assert!(
        !codes_with_roots(
            &imported(pg_with_ca(&private.join("ca.pem"))),
            &[root.path()]
        )
        .contains(&IssueCode::PathRefused)
    );
}

#[cfg(windows)]
#[test]
fn windows_verbatim_disk_is_normalised() {
    let root = tempdir();
    let f = root.path().join("ca.pem");
    std::fs::write(&f, CA).unwrap(); // Q22
    assert!(
        !codes_with_roots(&imported(pg_with_ca(&f)), &[root.path()])
            .contains(&IssueCode::PathRefused)
    );
}

#[test]
fn missing_sqlite_file_is_checked_through_its_parent() {
    let root = tempdir();
    let f = root.path().join("new.sqlite"); // Q22
    assert!(
        !codes_with_roots(&imported(sqlite_at(&f)), &[root.path()])
            .contains(&IssueCode::PathRefused)
    );
    // Outside the roots, a new file is refused too; so is one whose folder does not exist.
    let other = tempdir();
    assert!(
        codes_with_roots(&imported(sqlite_at(&f)), &[other.path()])
            .contains(&IssueCode::PathRefused)
    );
    let deep = root.path().join("no-such-dir").join("new.sqlite");
    assert!(
        codes_with_roots(&imported(sqlite_at(&deep)), &[root.path()])
            .contains(&IssueCode::PathRefused)
    );
}

#[test]
fn an_existing_sqlite_file_inside_a_root_is_accepted() {
    let root = tempdir();
    let f = root.path().join("shop.sqlite");
    std::fs::write(&f, b"").unwrap();
    let mut p = imported(sqlite_at(&f));
    if let ProfileEndpoint::File {
        create_if_missing, ..
    } = &mut p.endpoint
    {
        *create_if_missing = false;
    }
    assert_eq!(codes_with_roots(&p, &[root.path()]), []);
    assert!(codes_with_roots(&p, &[]).contains(&IssueCode::PathRefused));
}
