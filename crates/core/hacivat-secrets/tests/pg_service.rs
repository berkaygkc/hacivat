//! `pg_service.conf` reading (HV-SEC-03, HV-SEC-04 vectors). Every file is a temp fixture; the
//! user's own `~/.pg_service.conf` is never read. Secret comparisons never print either value.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use hacivat_secrets::{ExposeSecret, PgService, read_pg_service};

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

/// The sections of a private (0600) fixture, which must raise no warning.
fn services(contents: &str) -> Vec<PgService> {
    let f = read_pg_service(&file(0o600, contents)).unwrap();
    assert!(f.warnings.is_empty());
    f.services
}

#[test]
fn pg_service_sections() {
    let f = read_pg_service(&file(
        0o600,
        "[orders]\nhost=db\nport=5433\ndbname=orders\nuser=deploy\npassword=pw\n[x]\nhost=h\n",
    ))
    .unwrap();
    assert!(f.warnings.is_empty());
    let s = f.services;
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
    let s = services(
        "# top comment\r\nhost=ignored-before-any-section\r\n\r\n  [a]  \r\n  host =db.a \r\n# note\r\n\tport=5432\r\nnot a pair\r\n[b]\r\n[c\r\nhost=x\r\n",
    );
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
    // `[c` has no `]`: it is skipped, so the next line stays in `b`.
    assert_eq!(s[1].fields, vec![pair("host", "x")]);
}

#[test]
fn values_are_kept_as_written() {
    let s = services("[a]\noptions=-c search_path=app\nhost= spaced\n");
    assert_eq!(
        s[0].fields,
        vec![
            pair("options", "-c search_path=app"),
            pair("host", " spaced")
        ]
    );
}

#[test]
fn only_the_exact_password_key_is_a_password_and_no_casing_lands_in_fields() {
    let s = services("[a]\nPASSWORD=upper\nPassword=mixed\nhost=db\n");
    assert_eq!(s[0].fields, vec![pair("host", "db")]);
    assert!(s[0].password.is_none());
}

#[test]
fn the_first_password_wins_and_is_not_left_trimmed() {
    let s = services("[a]\npassword= x\npassword=second\n[b]\npassword=b1\n");
    assert_secret!(s[0].password.as_ref().unwrap(), " x");
    assert_secret!(s[1].password.as_ref().unwrap(), "b1");
    assert!(s[0].fields.is_empty());
}

#[test]
fn duplicate_sections_are_all_returned_in_order() {
    let s = services("[a]\nhost=first\n[a]\nhost=second\n");
    assert_eq!(
        s.iter()
            .map(|x| (x.name.as_str(), x.line))
            .collect::<Vec<_>>(),
        [("a", 1), ("a", 3)]
    );
    assert_eq!(s[0].fields, vec![pair("host", "first")]);
}

#[cfg(unix)]
#[test]
fn a_readable_file_drops_the_password_with_a_warning() {
    use hacivat_secrets::ServiceFileWarning;

    let path = file(
        0o644,
        "[a]\nhost=db\npassword=hv_test_pw\npassword=again\n[b]\nhost=h\n[c]\npassword=hv_test_pw\n",
    );
    let f = read_pg_service(&path).unwrap();
    assert_eq!(f.services[0].fields, vec![pair("host", "db")]);
    assert!(f.services.iter().all(|s| s.password.is_none()));
    assert_eq!(
        f.warnings,
        vec![
            ServiceFileWarning::PasswordIgnoredInsecureFile {
                path: path.clone(),
                mode: 0o644,
                service: "a".into(),
                line: 3
            },
            ServiceFileWarning::PasswordIgnoredInsecureFile {
                path,
                mode: 0o644,
                service: "c".into(),
                line: 8
            },
        ]
    );
    assert!(!format!("{f:?}").contains("hv_test_pw"));
}

#[test]
fn a_service_never_debug_prints_its_password() {
    let f = read_pg_service(&file(0o600, "[a]\npassword=hv_test_pw\n")).unwrap();
    assert!(f.services[0].password.is_some());
    assert!(
        !format!("{f:?}").contains("hv_test_pw"),
        "the password was printed"
    );
}

#[test]
fn a_missing_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = read_pg_service(&dir.path().join("nope")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn a_directory_or_an_oversized_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        read_pg_service(dir.path()).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    let big = "#".repeat(1 << 20) + "\n[a]\n";
    assert_eq!(
        read_pg_service(&file(0o600, &big)).unwrap_err().kind(),
        std::io::ErrorKind::FileTooLarge
    );
}

#[cfg(unix)]
#[test]
fn a_device_is_refused_without_reading() {
    assert_eq!(
        read_pg_service(Path::new("/dev/zero")).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
}
