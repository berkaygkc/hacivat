//! Known host keys (HV-SEC-01, B11, Task 4): OpenSSH `known_hosts` parsing (plain, bracket,
//! hashed, lists, markers, CRLF, long lines), lazy loading (Q42), re-reading on an `(mtime, len)`
//! change (Q24) and Hacivat's own `0600` store with one ordered writer. Every file lives in a
//! temp directory: no test reads the real `~/.ssh`.

use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hacivat_net::ssh::known_hosts::host_pattern;
use hacivat_net::ssh::{HostKey, HostKeyCheck, KnownHosts, KnownSource};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

/// A syntactically valid Ed25519 public key blob (`string "ssh-ed25519"`, `string key`) with
/// every key byte `seed`. Not a real key: host-key checks compare blobs, never use them.
fn ed25519(seed: u8) -> HostKey {
    let mut blob = Vec::new();
    for part in [&b"ssh-ed25519"[..], &[seed; 32][..]] {
        blob.extend_from_slice(&u32::try_from(part.len()).unwrap().to_be_bytes());
        blob.extend_from_slice(part);
    }
    HostKey {
        algorithm: "ssh-ed25519".into(),
        sha256: Sha256::digest(&blob).into(),
        openssh: format!("ssh-ed25519 {}", STANDARD.encode(&blob)),
    }
}

/// An ECDSA-typed blob (only the type string matters to the checks).
fn ecdsa(seed: u8) -> HostKey {
    let mut blob = Vec::new();
    for part in [&b"ecdsa-sha2-nistp256"[..], b"nistp256", &[seed; 65][..]] {
        blob.extend_from_slice(&u32::try_from(part.len()).unwrap().to_be_bytes());
        blob.extend_from_slice(part);
    }
    HostKey {
        algorithm: "ecdsa-sha2-nistp256".into(),
        sha256: Sha256::digest(&blob).into(),
        openssh: format!("ecdsa-sha2-nistp256 {}", STANDARD.encode(&blob)),
    }
}

static K1: LazyLock<HostKey> = LazyLock::new(|| ed25519(1));
static K2: LazyLock<HostKey> = LazyLock::new(|| ed25519(2));
static K3: LazyLock<HostKey> = LazyLock::new(|| ed25519(3));
static CA: LazyLock<HostKey> = LazyLock::new(|| ed25519(9));

/// A store and the temp directory that holds its files.
struct Kh {
    kh: Arc<KnownHosts>,
    _dir: TempDir,
}

impl Deref for Kh {
    type Target = KnownHosts;
    fn deref(&self) -> &KnownHosts {
        &self.kh
    }
}

/// An OpenSSH file holding `lines`, and an empty Hacivat store.
fn known(lines: &[&str]) -> Kh {
    let mut body = lines.join("\n");
    body.push('\n');
    known_openssh_tmp(&body).0
}

/// `(store, OpenSSH file, Hacivat file)`; the Hacivat file and its directory do not exist yet.
fn known_tmp() -> (Kh, PathBuf, PathBuf) {
    let (kh, ssh_file) = known_openssh_tmp(&format!("other.acme.io {}\n", K3.openssh));
    let store = kh.hacivat_path().to_path_buf();
    (kh, ssh_file, store)
}

fn known_openssh_tmp(body: &str) -> (Kh, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let ssh_file = dir.path().join("dot-ssh/known_hosts");
    fs::create_dir_all(ssh_file.parent().unwrap()).unwrap();
    fs::write(&ssh_file, body).unwrap();
    let store = dir.path().join("config/hacivat/known_hosts");
    let kh = KnownHosts::load(vec![ssh_file.clone()], store);
    (Kh { kh, _dir: dir }, ssh_file)
}

/// A store path in a directory that no longer exists.
fn tmp_store() -> PathBuf {
    tempfile::tempdir()
        .unwrap()
        .path()
        .join("hacivat/known_hosts")
}

/// `|1|salt|hash key`: HMAC-SHA1 over `host`, or `[host]:port` off port 22.
fn hashed_line(host: &str, port: u16, key: &HostKey) -> String {
    let salt = [7u8; 20];
    let name = if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    };
    let mut mac = Hmac::<Sha1>::new_from_slice(&salt).unwrap();
    mac.update(name.as_bytes());
    let hash = mac.finalize().into_bytes();
    format!(
        "|1|{}|{} {}",
        STANDARD.encode(salt),
        STANDARD.encode(hash),
        key.openssh
    )
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Rewrites `path` and moves its mtime, as `ssh-keygen -R` (a rewrite and a rename) would.
fn rewrite_with_new_mtime(path: &Path, body: &str) {
    let before = fs::metadata(path).unwrap().modified().unwrap();
    fs::write(path, body).unwrap();
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(before + Duration::from_secs(60)).unwrap();
}

fn openssh(path: &Path, line: usize) -> KnownSource {
    KnownSource::OpenSsh {
        path: path.to_path_buf(),
        line,
    }
}

// The brief's tests (Task 4 Step 1).

#[test]
fn hashed_entries_match() {
    let kh = known(&[&hashed_line("db.acme.io", 22, &K1)]);
    assert!(matches!(
        kh.check("db.acme.io", 22, &K1),
        HostKeyCheck::Known(KnownSource::OpenSsh { .. })
    ));
}

#[test]
fn non_default_port_uses_bracket_form() {
    let kh = known(&[&format!("[db.acme.io]:2222 {}", K1.openssh)]);
    assert!(matches!(
        kh.check("db.acme.io", 2222, &K1),
        HostKeyCheck::Known(_)
    ));
    assert_eq!(kh.check("db.acme.io", 22, &K1), HostKeyCheck::Unknown);
}

#[test]
fn changed_key_reports_old_key_and_source() {
    let kh = known(&[&format!("db {}", K1.openssh)]);
    assert!(matches!(
        kh.check("db", 22, &K2),
        HostKeyCheck::Changed { old, source: KnownSource::OpenSsh { line: 1, .. } } if old == *K1
    ));
}

#[test]
fn revoked_marker_is_a_hard_stop() {
    assert!(matches!(
        known(&[&format!("@revoked * {}", K1.openssh)]).check("db", 22, &K1),
        HostKeyCheck::Revoked { .. }
    ));
}

#[test]
fn trust_writes_hacivat_store_only() {
    let (kh, ssh_file, store) = known_tmp();
    let before = std::fs::read(&ssh_file).unwrap();
    kh.trust("db", 22, &K1).unwrap();
    assert_eq!(std::fs::read(&ssh_file).unwrap(), before);
    #[cfg(unix)]
    assert_eq!(mode_of(&store), 0o600); // Q5
    assert!(matches!(
        kh.check("db", 22, &K1),
        HostKeyCheck::Known(KnownSource::Hacivat)
    ));
}

#[test]
fn replace_only_touches_the_hacivat_store() {
    let (kh, _, store) = known_tmp();
    kh.trust("db", 22, &K1).unwrap();
    kh.replace("db", 22, &K1, &K2).unwrap();
    assert!(!read(&store).contains(&K1.openssh));
    assert!(matches!(
        kh.check("db", 22, &K2),
        HostKeyCheck::Known(KnownSource::Hacivat)
    ));
}

#[test]
fn concurrent_trusts_are_ordered() {
    let (kh, _, store) = known_tmp();
    std::thread::scope(|s| {
        for i in 0..8 {
            let kh = &kh;
            s.spawn(move || kh.trust(&format!("h{i}"), 22, &K1).unwrap());
        }
    });
    assert_eq!(read(&store).lines().count(), 8);
}

#[test]
fn cert_authority_line_is_recognised() {
    let r =
        known(&[&format!("@cert-authority *.acme.io {}", CA.openssh)]).check("db.acme.io", 22, &K1);
    // Q32: host certificates are not checked in v1 (ADR 0003, m2b-t04), so a CA line never
    // makes a plain key known: the key is Unknown.
    assert_eq!(r, HostKeyCheck::Unknown);
}

#[test]
fn openssh_file_edit_is_seen_without_restart() {
    // Q24
    let (kh, ssh_file) = known_openssh_tmp(&format!("db {}", K1.openssh));
    assert!(matches!(
        kh.check("db", 22, &K2),
        HostKeyCheck::Changed { .. }
    ));
    rewrite_with_new_mtime(&ssh_file, ""); // as `ssh-keygen -R db` would
    assert_eq!(kh.check("db", 22, &K2), HostKeyCheck::Unknown);
}

#[test]
fn load_does_no_io() {
    // Q42
    let kh = KnownHosts::load(vec![PathBuf::from("/nonexistent/known_hosts")], tmp_store());
    assert_eq!(kh.check("db", 22, &K1), HostKeyCheck::Unknown);
}

#[test]
fn fingerprint_is_sha256_base64_unpadded() {
    assert!(K1.fingerprint().starts_with("SHA256:"));
    assert!(!K1.fingerprint().ends_with('='));
}

// Beyond the brief.

#[test]
fn files_are_read_at_the_first_check_not_at_load() {
    let dir = tempfile::tempdir().unwrap();
    let ssh_file = dir.path().join("known_hosts");
    let kh = KnownHosts::load(vec![ssh_file.clone()], dir.path().join("hv/known_hosts"));
    // The file appears after `load`: a load that read it would have cached "no file".
    fs::write(&ssh_file, format!("db {}\n", K1.openssh)).unwrap();
    assert_eq!(
        kh.check("db", 22, &K1),
        HostKeyCheck::Known(openssh(&ssh_file, 1))
    );
    assert!(
        !dir.path().join("hv").exists(),
        "a check never creates the store"
    );
}

#[test]
fn the_fixture_parses_with_lf_and_crlf_endings() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ssh/known_hosts");
    let lf = fs::read_to_string(src).unwrap();
    for body in [lf.clone(), lf.replace('\n', "\r\n")] {
        let (kh, f) = known_openssh_tmp(&body);
        assert_eq!(
            kh.check("bastion.acme.io", 22, &K1),
            HostKeyCheck::Known(openssh(&f, 2))
        );
        assert_eq!(
            kh.check("10.0.0.5", 22, &K1),
            HostKeyCheck::Known(openssh(&f, 2))
        );
        assert_eq!(
            kh.check("bastion.acme.io", 2222, &K2),
            HostKeyCheck::Known(openssh(&f, 3))
        );
        assert_eq!(
            kh.check("bastion.acme.io", 2222, &K1),
            HostKeyCheck::Changed {
                old: K2.clone(),
                source: openssh(&f, 3)
            }
        );
        assert_eq!(
            kh.check("db.acme.io", 22, &K1),
            HostKeyCheck::Known(openssh(&f, 4))
        );
        assert_eq!(
            kh.check("anything.example", 22, &K3),
            HostKeyCheck::Revoked {
                source: openssh(&f, 5)
            }
        );
        // The CA key presented as a plain host key is not known through its CA line.
        assert_eq!(kh.check("x.acme.io", 22, &CA), HostKeyCheck::Unknown);
        assert_eq!(kh.skipped_lines(), 0);
    }
}

#[test]
fn comma_lists_and_ascii_case_folding_match() {
    let kh = known(&[&format!("Foo,DB.Acme.IO,bar {}", K1.openssh)]);
    assert!(matches!(
        kh.check("db.acme.io", 22, &K1),
        HostKeyCheck::Known(_)
    ));
    assert!(matches!(
        kh.check("DB.ACME.IO", 22, &K1),
        HostKeyCheck::Known(_)
    ));
    assert!(matches!(kh.check("foo", 22, &K1), HostKeyCheck::Known(_)));
    assert_eq!(kh.check("baz", 22, &K1), HostKeyCheck::Unknown);
}

#[test]
fn a_hashed_entry_uses_the_bracket_form_off_port_22() {
    let kh = known(&[&hashed_line("db.acme.io", 2222, &K1)]);
    assert!(matches!(
        kh.check("db.acme.io", 2222, &K1),
        HostKeyCheck::Known(_)
    ));
    assert_eq!(kh.check("db.acme.io", 22, &K1), HostKeyCheck::Unknown);
    assert_eq!(kh.check("db.acme.io", 2223, &K1), HostKeyCheck::Unknown);
}

#[test]
fn a_negated_pattern_wins_over_a_wildcard() {
    let kh = known(&[&format!("*.acme.io,!db.acme.io {}", K1.openssh)]);
    assert!(matches!(
        kh.check("web.acme.io", 22, &K1),
        HostKeyCheck::Known(_)
    ));
    assert_eq!(kh.check("db.acme.io", 22, &K1), HostKeyCheck::Unknown);
    // A negated pattern alone matches nothing.
    let kh = known(&[&format!("!db.acme.io {}", K1.openssh)]);
    assert_eq!(kh.check("web.acme.io", 22, &K1), HostKeyCheck::Unknown);
}

#[test]
fn a_wildcard_does_not_match_the_bracket_form_of_another_port() {
    let kh = known(&[&format!("db.acme.io? {}", K1.openssh)]);
    assert_eq!(kh.check("db.acme.io", 22, &K1), HostKeyCheck::Unknown);
    assert!(matches!(
        kh.check("db.acme.io1", 22, &K1),
        HostKeyCheck::Known(_)
    ));
    let kh = known(&[&format!("[db.acme.io]:* {}", K1.openssh)]);
    assert!(matches!(
        kh.check("db.acme.io", 2222, &K1),
        HostKeyCheck::Known(_)
    ));
    assert_eq!(kh.check("db.acme.io", 22, &K1), HostKeyCheck::Unknown);
}

#[test]
fn a_known_key_anywhere_beats_a_changed_one() {
    let (kh, f) = known_openssh_tmp(&format!("db {}\ndb {}\n", K2.openssh, K1.openssh));
    assert_eq!(kh.check("db", 22, &K1), HostKeyCheck::Known(openssh(&f, 2)));
}

#[test]
fn a_different_key_type_is_unknown_not_changed() {
    // OpenSSH's rule: only a key of the same type can be "changed"; another type is new.
    let kh = known(&[&format!("db {}", K1.openssh)]);
    assert_eq!(kh.check("db", 22, &ecdsa(4)), HostKeyCheck::Unknown);
}

#[test]
fn a_revoked_key_is_revoked_even_when_also_listed_as_known() {
    let (kh, f) = known_openssh_tmp(&format!("db {}\n@revoked db {}\n", K1.openssh, K1.openssh));
    assert_eq!(
        kh.check("db", 22, &K1),
        HostKeyCheck::Revoked {
            source: openssh(&f, 2)
        }
    );
    // `@revoked` only names its own hosts.
    let kh = known(&[&format!("@revoked other {}", K1.openssh)]);
    assert_eq!(kh.check("db", 22, &K1), HostKeyCheck::Unknown);
}

#[test]
fn unknown_markers_comments_and_garbage_are_skipped() {
    let (kh, f) = known_openssh_tmp(&format!(
        "# db {k1}\n@marker db {k1}\ndb ssh-ed25519 not-base64!\ndb\n   \n\tdb {k2}\n",
        k1 = K1.openssh,
        k2 = K2.openssh
    ));
    assert_eq!(
        kh.check("db", 22, &K1),
        HostKeyCheck::Changed {
            old: K2.clone(),
            source: openssh(&f, 6)
        }
    );
}

#[test]
fn a_line_over_16_kib_is_skipped_and_counted_and_a_10_mib_line_does_not_panic() {
    let mut body = format!("db {} {}\n", K2.openssh, "c".repeat(16 * 1024));
    body.push_str(&"x".repeat(10 << 20));
    body.push('\n');
    body.push_str(&format!("db {}\n", K1.openssh));
    let (kh, f) = known_openssh_tmp(&body);
    // Line 1 (just over 16 KiB) and line 2 (10 MiB) are skipped; line 3 still counts.
    assert_eq!(kh.check("db", 22, &K1), HostKeyCheck::Known(openssh(&f, 3)));
    assert_eq!(
        kh.check("db", 22, &K2),
        HostKeyCheck::Changed {
            old: K1.clone(),
            source: openssh(&f, 3)
        }
    );
    assert_eq!(kh.skipped_lines(), 2);
}

#[test]
fn a_line_of_exactly_16_kib_is_read() {
    let line = format!("db {} ", K1.openssh);
    let pad = 16 * 1024 - line.len();
    let (kh, f) = known_openssh_tmp(&format!("{line}{}\n", "c".repeat(pad)));
    assert_eq!(kh.check("db", 22, &K1), HostKeyCheck::Known(openssh(&f, 1)));
}

#[test]
fn non_utf8_bytes_do_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("known_hosts");
    let mut body = b"db\xff\xfe,\xc3 ".to_vec();
    body.extend_from_slice(K2.openssh.as_bytes());
    body.extend_from_slice(b" \xff comment\n");
    body.extend_from_slice(format!("db {}\n", K1.openssh).as_bytes());
    fs::write(&f, body).unwrap();
    let kh = KnownHosts::load(vec![f.clone()], dir.path().join("hv/known_hosts"));
    assert_eq!(kh.check("db", 22, &K1), HostKeyCheck::Known(openssh(&f, 2)));
}

#[test]
fn an_edit_with_the_same_length_is_seen_through_the_mtime() {
    let (kh, ssh_file) = known_openssh_tmp(&format!("db {}\n", K1.openssh));
    assert!(matches!(kh.check("db", 22, &K1), HostKeyCheck::Known(_)));
    rewrite_with_new_mtime(&ssh_file, &format!("db {}\n", K2.openssh));
    assert!(matches!(kh.check("db", 22, &K2), HostKeyCheck::Known(_)));
    assert!(matches!(
        kh.check("db", 22, &K1),
        HostKeyCheck::Changed { .. }
    ));
}

#[test]
fn a_deleted_or_late_openssh_file_is_followed() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("known_hosts");
    let b = dir.path().join("known_hosts2");
    fs::write(&a, format!("db {}\n", K1.openssh)).unwrap();
    let kh = KnownHosts::load(
        vec![a.clone(), b.clone()],
        dir.path().join("hv/known_hosts"),
    );
    assert_eq!(kh.check("db", 22, &K1), HostKeyCheck::Known(openssh(&a, 1)));
    fs::remove_file(&a).unwrap();
    assert_eq!(kh.check("db", 22, &K1), HostKeyCheck::Unknown);
    fs::write(&b, format!("# second file\ndb {}\n", K1.openssh)).unwrap();
    assert_eq!(kh.check("db", 22, &K1), HostKeyCheck::Known(openssh(&b, 2)));
}

#[cfg(unix)]
#[test]
fn the_store_directory_is_created_0700() {
    let (kh, _, store) = known_tmp();
    kh.trust("db", 22, &K1).unwrap();
    assert_eq!(mode_of(store.parent().unwrap()), 0o700);
    assert_eq!(read(&store), format!("db {}\n", K1.openssh));
}

#[test]
fn trust_uses_the_bracket_form_off_port_22_and_is_idempotent() {
    let (kh, _, store) = known_tmp();
    kh.trust("DB.acme.io", 2222, &K1).unwrap();
    kh.trust("db.acme.io", 2222, &K1).unwrap();
    assert_eq!(read(&store), format!("[db.acme.io]:2222 {}\n", K1.openssh));
    assert_eq!(
        kh.check("db.acme.io", 2222, &K1),
        HostKeyCheck::Known(KnownSource::Hacivat)
    );
    assert_eq!(kh.check("db.acme.io", 22, &K1), HostKeyCheck::Unknown);
}

#[test]
fn trust_refuses_names_that_could_inject_lines_or_patterns() {
    let (kh, _, store) = known_tmp();
    for host in [
        "",
        "db\nevil",
        "db\r",
        "db evil",
        "db\tx",
        "*",
        "*.acme.io",
        "db?",
        "!db",
        "a,b",
        "[db]",
        "|1|x|y",
        "#db",
        "@revoked",
        "db\u{0}",
        "dé.acme.io",
    ] {
        let err = kh.trust(host, 22, &K1).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{host:?}");
    }
    assert!(!store.exists());
    // IPv6 literals and scope ids are fine.
    kh.trust("2001:db8::1", 22, &K1).unwrap();
    kh.trust("fe80::1%en0", 2222, &K1).unwrap();
    assert_eq!(read(&store).lines().count(), 2);
    assert!(matches!(
        kh.check("fe80::1%en0", 2222, &K1),
        HostKeyCheck::Known(KnownSource::Hacivat)
    ));
}

#[test]
fn trust_refuses_a_key_that_is_not_one_openssh_line() {
    let (kh, _, store) = known_tmp();
    let mut bad = K1.clone();
    bad.openssh = format!("{}\nevil.acme.io {}", K1.openssh, K2.openssh);
    let mut inconsistent = K1.clone();
    inconsistent.sha256 = K2.sha256;
    let mut garbage = K1.clone();
    garbage.openssh = "ssh-ed25519 !!!".into();
    for key in [&bad, &inconsistent, &garbage] {
        assert_eq!(
            kh.trust("db", 22, key).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
    assert!(!store.exists());
}

#[test]
fn trust_refuses_a_revoked_key() {
    let (kh, _f) = known_openssh_tmp(&format!("@revoked * {}\n", K1.openssh));
    assert!(kh.trust("db", 22, &K1).is_err());
    assert!(!kh.hacivat_path().exists());
}

#[test]
fn trust_refuses_a_changed_key_whichever_store_lists_the_old_one() {
    // B11: a changed key is a hard stop; no policy (not even AcceptNew) may trust it.
    let (kh, f) = known_openssh_tmp(&format!("db {}\n", K1.openssh));
    let err = kh.trust("db", 22, &K2).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(!kh.hacivat_path().exists());
    kh.trust("web", 22, &K2).unwrap();
    assert_eq!(
        kh.trust("web", 22, &K1).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        kh.check("db", 22, &K2),
        HostKeyCheck::Changed {
            old: K1.clone(),
            source: openssh(&f, 1)
        }
    );
    // A key of another type is new, as in OpenSSH.
    kh.trust("db", 22, &ecdsa(5)).unwrap();
}

#[test]
fn a_hacivat_entry_wins_the_changed_source_so_replace_is_offered() {
    let (kh, _) = known_openssh_tmp(&format!("db {}\n", K1.openssh));
    kh.trust("db", 22, &K1).unwrap();
    assert_eq!(
        kh.check("db", 22, &K2),
        HostKeyCheck::Changed {
            old: K1.clone(),
            source: KnownSource::Hacivat
        }
    );
}

#[test]
fn replace_keeps_other_entries_and_refuses_a_missing_old_key() {
    let (kh, ssh_file, store) = known_tmp();
    kh.trust("db", 22, &K1).unwrap();
    kh.trust("web", 22, &K1).unwrap();
    let before = read(&store);
    // The old key is not in Hacivat's store for this host: nothing changes.
    assert_eq!(
        kh.replace("db", 22, &K3, &K2).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(
        kh.replace("other.acme.io", 22, &K3, &K2)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(read(&store), before);
    kh.replace("db", 22, &K1, &K2).unwrap();
    assert_eq!(
        read(&store),
        format!("web {}\ndb {}\n", K1.openssh, K2.openssh)
    );
    assert!(matches!(
        kh.check("web", 22, &K1),
        HostKeyCheck::Known(KnownSource::Hacivat)
    ));
    assert!(read(&ssh_file).contains("other.acme.io"));
    #[cfg(unix)]
    assert_eq!(mode_of(&store), 0o600);
}

#[test]
fn a_hand_edited_store_is_reread() {
    let (kh, _, store) = known_tmp();
    kh.trust("db", 22, &K1).unwrap();
    assert!(matches!(kh.check("db", 22, &K1), HostKeyCheck::Known(_)));
    rewrite_with_new_mtime(&store, "");
    assert_eq!(kh.check("db", 22, &K1), HostKeyCheck::Unknown);
}

#[cfg(unix)]
#[test]
fn a_fifo_at_any_path_never_blocks_a_check_or_a_write() {
    // M3: opening a FIFO with no writer blocks; neither the store nor an OpenSSH file may be one.
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("known_hosts");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .is_ok_and(|s| s.success());
    if !made {
        eprintln!("skip: mkfifo unavailable");
        return;
    }
    let kh = KnownHosts::load(vec![fifo.clone()], fifo.clone());
    assert_eq!(kh.check("db", 22, &K1), HostKeyCheck::Unknown);
    assert_eq!(
        kh.trust("db", 22, &K1).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(
        kh.replace("db", 22, &K1, &K2).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
}

#[test]
fn host_pattern_follows_openssh() {
    assert_eq!(host_pattern("Bastion.acme.io", 22), "bastion.acme.io");
    assert_eq!(
        host_pattern("bastion.acme.io", 2222),
        "[bastion.acme.io]:2222"
    );
    assert_eq!(host_pattern("2001:db8::1", 2222), "[2001:db8::1]:2222");
}

#[test]
fn host_keys_parse_from_their_openssh_form() {
    assert_eq!(HostKey::from_openssh(&K1.openssh), Some(K1.clone()));
    assert_eq!(
        HostKey::from_openssh(&format!("{} a comment", K1.openssh)),
        Some(K1.clone())
    );
    assert_eq!(HostKey::from_openssh("ssh-ed25519"), None);
    assert_eq!(HostKey::from_openssh("ssh-ed25519 AAAA"), None);
    // The type in the blob must match the named type.
    let blob = K1.openssh.split(' ').nth(1).unwrap();
    assert_eq!(HostKey::from_openssh(&format!("ssh-rsa {blob}")), None);
}
