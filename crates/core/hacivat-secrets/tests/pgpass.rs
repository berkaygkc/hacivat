//! `.pgpass` lookup, libpq-exact (HV-SEC-03, HV-SEC-04 vectors). Every file is a temp fixture;
//! the user's own `~/.pgpass` is never read. Secret comparisons never print either value.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use hacivat_secrets::{ExposeSecret, PgPassMatch, PgPassWarning, pgpass_lookup};

/// Compares a secret without printing either value on failure.
macro_rules! assert_secret {
    ($got:expr, $want:expr) => {
        assert!(
            $got.expose_secret() == $want,
            "secret mismatch (values not shown)"
        )
    };
}

thread_local! {
    /// Keeps each fixture's directory alive until the test thread ends.
    static DIRS: RefCell<Vec<tempfile::TempDir>> = const { RefCell::new(Vec::new()) };
    static LAST: RefCell<PathBuf> = const { RefCell::new(PathBuf::new()) };
}

/// A fresh temp file holding `contents`, with permission bits `mode` on Unix.
fn file(mode: u32, contents: &str) -> PathBuf {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("pgpass");
    std::fs::write(&path, contents).expect("the fixture is written");
    set_mode(&path, mode);
    LAST.with(|l| *l.borrow_mut() = path.clone());
    DIRS.with(|d| d.borrow_mut().push(dir));
    path
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

/// The path of the last fixture `file` made.
#[cfg(unix)]
fn last_path() -> PathBuf {
    LAST.with(|l| l.borrow().clone())
}

/// The line that matches, which must exist.
fn hit(path: &Path, host: &str, port: u16, db: &str, user: Option<&str>) -> PgPassMatch {
    pgpass_lookup(path, host, port, db, user)
        .expect("the file is readable")
        .expect("a line matches")
}

#[test]
fn pgpass_vectors() {
    let f = file(0o600, "db.example:5432:app:alice:s3cr\\:et\n*:*:*:bob:pw\n");
    assert_secret!(
        hit(&f, "db.example", 5432, "app", Some("alice")).password,
        "s3cr:et"
    );
    assert_eq!(
        hit(&f, "db.example", 5432, "app", None).user.as_deref(),
        Some("alice")
    );
    assert!(
        pgpass_lookup(&f, "db.example", 5432, "app", Some("carol"))
            .unwrap()
            .is_none()
    );
    assert!(
        pgpass_lookup(&f, "DB.EXAMPLE", 5432, "app", Some("alice"))
            .unwrap()
            .is_none()
    );
    assert_secret!(
        hit(
            &file(0o600, "h:5432:d:u:pass:with:colons\r\n"),
            "h",
            5432,
            "d",
            Some("u")
        )
        .password,
        "pass:with:colons"
    );
    assert_secret!(
        hit(&file(0o600, "h:5432:d:u:\n"), "h", 5432, "d", Some("u")).password,
        ""
    );
    #[cfg(unix)]
    assert_eq!(
        pgpass_lookup(&file(0o644, "h:5432:d:u:p\n"), "h", 5432, "d", Some("u")).unwrap_err(),
        PgPassWarning::Permissions {
            path: last_path(),
            mode: 0o644
        }
    );
}

#[test]
fn the_first_match_wins_and_reports_its_line() {
    let f = file(
        0o600,
        "# a comment\n\nh:5432:d:u:first\nh:5432:d:u:second\n*:*:*:*:any\n",
    );
    let m = hit(&f, "h", 5432, "d", Some("u"));
    assert_eq!(m.line, 3);
    assert_eq!(m.user.as_deref(), Some("u"));
    assert_secret!(m.password, "first");
    let any = hit(&f, "other", 1, "x", Some("v"));
    assert_eq!(any.line, 5);
    assert_eq!(any.user.as_deref(), Some("v"));
    assert_secret!(any.password, "any");
}

#[test]
fn wildcards_are_whole_fields_only() {
    // `*x` and `\*` are literal text, not wildcards.
    let f = file(0o600, "*x:5432:d:u:a\n\\*:5432:d:u:b\nh:*:*:*:c\n");
    assert_eq!(hit(&f, "*x", 5432, "d", Some("u")).line, 1);
    assert_eq!(hit(&f, "*", 5432, "d", Some("u")).line, 2);
    assert_eq!(hit(&f, "h", 6543, "any", Some("someone")).line, 3);
    assert!(
        pgpass_lookup(&f, "q", 5432, "d", Some("u"))
            .unwrap()
            .is_none()
    );
    // A wildcard user with no user asked for reports no user.
    assert_eq!(hit(&f, "h", 1, "d", None).user, None);
}

#[test]
fn escapes_in_every_field() {
    let f = file(0o600, "h\\:x:5432:d\\\\b:u:back\\\\slash\\:colon\\x\n");
    let m = hit(&f, "h:x", 5432, "d\\b", Some("u"));
    assert_secret!(m.password, "back\\slash:colonx");
    // A lone trailing backslash is kept.
    assert_secret!(
        hit(
            &file(0o600, "h:5432:d:u:end\\\n"),
            "h",
            5432,
            "d",
            Some("u")
        )
        .password,
        "end\\"
    );
}

#[test]
fn ports_compare_as_text_and_zero_means_5432() {
    let f = file(0o600, "h:05432:d:u:padded\nh:5432:d:u:plain\n");
    assert_eq!(hit(&f, "h", 5432, "d", Some("u")).line, 2);
    assert_eq!(hit(&f, "h", 0, "d", Some("u")).line, 2);
}

#[test]
fn localhost_matches_a_unix_socket() {
    let f = file(
        0o600,
        "localhost:5432:d:u:local\n/var/run/pg:5432:d:v:path\n",
    );
    assert_eq!(hit(&f, "/tmp", 5432, "d", Some("u")).line, 1);
    assert_eq!(hit(&f, "", 5432, "d", Some("u")).line, 1);
    assert_eq!(hit(&f, "localhost", 5432, "d", Some("u")).line, 1);
    assert_eq!(hit(&f, "/var/run/pg", 5432, "d", Some("v")).line, 2);
    assert!(
        pgpass_lookup(&f, "127.0.0.1", 5432, "d", Some("u"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn short_lines_never_match() {
    let f = file(0o600, "h:5432:d:u\nh:5432:d\n");
    assert!(
        pgpass_lookup(&f, "h", 5432, "d", Some("u"))
            .unwrap()
            .is_none()
    );
    assert!(pgpass_lookup(&f, "h", 5432, "d", None).unwrap().is_none());
}

#[test]
fn a_missing_file_has_no_match() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        pgpass_lookup(&dir.path().join("nope"), "h", 5432, "d", Some("u"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_directory_is_ignored_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        pgpass_lookup(dir.path(), "h", 5432, "d", Some("u")).unwrap_err(),
        PgPassWarning::IsDirectory(dir.path().to_path_buf())
    );
}

#[cfg(unix)]
#[test]
fn any_group_or_other_bit_is_refused() {
    for mode in [0o640, 0o604, 0o610, 0o601, 0o660] {
        let err =
            pgpass_lookup(&file(mode, "h:5432:d:u:p\n"), "h", 5432, "d", Some("u")).unwrap_err();
        assert_eq!(
            err,
            PgPassWarning::Permissions {
                path: last_path(),
                mode
            }
        );
    }
    assert!(
        pgpass_lookup(&file(0o400, "h:5432:d:u:p\n"), "h", 5432, "d", Some("u"))
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_match_never_debug_prints_its_password() {
    let m = hit(
        &file(0o600, "h:5432:d:u:hv_test_pw\n"),
        "h",
        5432,
        "d",
        Some("u"),
    );
    let shown = format!("{m:?}");
    assert!(!shown.contains("hv_test_pw"), "the password was printed");
}
