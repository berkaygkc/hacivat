//! `pg_service.conf` reading (HV-SEC-03, HV-SEC-04 vectors). Every file is a temp fixture; the
//! user's own `~/.pg_service.conf` is never read. Secret comparisons never print either value.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use hacivat_secrets::{ExposeSecret, read_pg_service};

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
}

/// A fresh temp file holding `contents`, with permission bits `mode` on Unix.
fn file(mode: u32, contents: &str) -> PathBuf {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("pg_service.conf");
    std::fs::write(&path, contents).expect("the fixture is written");
    set_mode(&path, mode);
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

fn pair(k: &str, v: &str) -> (String, String) {
    (k.to_string(), v.to_string())
}

#[test]
fn pg_service_sections() {
    let s = read_pg_service(&file(
        0o600,
        "[orders]\nhost=db\nport=5433\ndbname=orders\nuser=deploy\npassword=pw\n[x]\nhost=h\n",
    ))
    .unwrap();
    assert_eq!(
        s.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
        ["orders", "x"]
    );
    assert_eq!(s[0].line, 1);
    assert_eq!(
        s[0].fields,
        vec![
            ("host".into(), "db".into()),
            ("port".into(), "5433".into()),
            ("dbname".into(), "orders".into()),
            ("user".into(), "deploy".into())
        ]
    );
    assert_secret!(s[0].password.as_ref().unwrap(), "pw");
}

#[test]
fn comments_blank_lines_whitespace_and_crlf() {
    let s = read_pg_service(&file(
        0o600,
        "# top comment\r\nhost=ignored-before-any-section\r\n\r\n  [a]  \r\n  host = db.a \r\n# note\r\n\tport=5432\r\nnot a pair\r\n[b]\r\n",
    ))
    .unwrap();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].name, "a");
    assert_eq!(s[0].line, 4);
    assert_eq!(
        s[0].fields,
        vec![pair("host", "db.a"), pair("port", "5432")]
    );
    assert!(s[0].password.is_none());
    assert_eq!(s[1].name, "b");
    assert_eq!(s[1].line, 9);
    assert!(s[1].fields.is_empty());
}

#[test]
fn values_keep_equals_signs_and_the_password_key_is_exact() {
    let s = read_pg_service(&file(
        0o600,
        "[a]\noptions=-c search_path=app\nPASSWORD=upper\n",
    ))
    .unwrap();
    assert_eq!(
        s[0].fields,
        vec![
            pair("options", "-c search_path=app"),
            pair("PASSWORD", "upper")
        ]
    );
    assert!(s[0].password.is_none());
}

#[cfg(unix)]
#[test]
fn a_readable_file_drops_the_password() {
    let s = read_pg_service(&file(0o644, "[a]\nhost=db\npassword=hv_test_pw\n")).unwrap();
    assert_eq!(s[0].fields, vec![pair("host", "db")]);
    assert!(s[0].password.is_none());
    assert!(!format!("{s:?}").contains("hv_test_pw"));
}

#[test]
fn a_service_never_debug_prints_its_password() {
    let s = read_pg_service(&file(0o600, "[a]\npassword=hv_test_pw\n")).unwrap();
    assert!(s[0].password.is_some());
    assert!(
        !format!("{s:?}").contains("hv_test_pw"),
        "the password was printed"
    );
}

#[test]
fn a_missing_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = read_pg_service(&dir.path().join("nope")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}
