//! Step 0 of a connect: profile issues and option sanitizing (HV-CONN-01, HV-DRV-05).
//!
//! Validation runs on the profile's *effective* settings ([`effective_profile`]): libpq,
//! JDBC and MySQL-client options that a URL left in the profile's extra options (`sslmode`,
//! `sslMode`, `ssl-mode`, `useSSL`, `sslrootcert`, `sslCa`, `ssl-ca`, `connect_timeout`, ...)
//! are applied to its TLS settings and timeout first, because no driver reads them (T13, T14).
//! T7's parser maps the TLS mode, `useSSL` and `connect_timeout` itself (`MappedTo`); the file
//! options stay extras, so they are mapped here.

use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use hacivat_driver::url::FieldId;
use hacivat_driver::{EngineCatalog, SanitizeInput, ViolationReason, sanitize};
use hacivat_net::{AddressClass, ClientCertPaths, TlsMode, TlsSetupError, build_tls_params};
use hacivat_proto::DbmsKind;

use crate::profile::{Profile, ProfileEndpoint, TransportSpec};

/// Why a profile cannot connect as it is (HV-CONN-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IssueCode {
    NameMissing,
    HostMissing,
    /// The host cannot be used: whitespace, a control character, URL syntax, or a name TLS
    /// cannot verify or send as SNI.
    HostInvalid,
    PortInvalid,
    FileMissing,
    /// A SQLite or TLS path Hacivat refuses: not absolute and local (a `file:` URI, a UNC share,
    /// a device), not a regular file, or, for a profile from an untrusted origin, outside the
    /// approved roots (DRV-05).
    PathRefused,
    CreateAndReadOnly,
    TransportUnavailable,
    TlsCaUnreadable,
    TlsCertUnreadable,
    TlsKeyEncrypted,
    /// MySQL and MariaDB with `VerifyCa` in M2a (P11).
    TlsModeUnsupported {
        mode: TlsMode,
    },
    OptionRejected {
        key: String,
        reason: ViolationReason,
    },
}

/// One issue, with the field it belongs to (HV-CONN-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValidationIssue {
    pub field: Option<FieldId>,
    pub code: IssueCode,
}

/// Where a file named by a profile from an untrusted origin (an import, a deep link) may live
/// (DRV-05): the user's home directory, plus every path the user picked in the connection
/// dialog's file picker (T17 calls [`ApprovedRoots::add`]). Shared: clones see the same list.
#[derive(Clone, Default)]
pub struct ApprovedRoots(Arc<RwLock<Vec<PathBuf>>>);

impl ApprovedRoots {
    /// Exactly `roots`.
    pub fn new(roots: Vec<PathBuf>) -> Self {
        ApprovedRoots(Arc::new(RwLock::new(roots)))
    }

    /// The user's home directory (none when the OS reports none).
    pub fn for_user() -> Self {
        Self::new(std::env::home_dir().into_iter().collect())
    }

    /// Approves `path`, a file or folder the user chose themselves.
    pub fn add(&self, path: PathBuf) {
        let mut roots = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if !roots.contains(&path) {
            roots.push(path);
        }
    }

    pub fn list(&self) -> Vec<PathBuf> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl fmt::Debug for ApprovedRoots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ApprovedRoots").field(&self.list()).finish()
    }
}

/// The name SQLite reads as an in-memory database rather than a file.
const SQLITE_MEMORY: &str = ":memory:";

/// Every issue of `p` with the user's home as the only approved root; see
/// [`validate_with`].
pub fn validate(p: &Profile, catalog: &EngineCatalog) -> Vec<ValidationIssue> {
    validate_with(p, catalog, &ApprovedRoots::for_user())
}

/// Every issue of `p`, in form order: name, endpoint, transport, TLS, then options.
///
/// - The name must not be blank.
/// - A server needs a usable host (`HostMissing`, `HostInvalid`) and a port that is not 0; a
///   socket needs a path.
/// - A SQLite file must be an absolute local path (or `:memory:`): never a `file:` URI, a UNC
///   share (`\\server\share`, `\\?\UNC\…`) or a device path (`\\.\…`, `\??\…`), and when
///   something exists there it must be a regular file (`PathRefused`). A missing file is
///   `FileMissing` only when it may not be created; creating a read-only database is an issue.
/// - SSH tunnels and the Karagöz transport cannot connect in M2a (R4).
/// - TLS: `VerifyCa` is unsupported for MySQL and MariaDB (P11); elsewhere it needs a CA file
///   (libpq parity: `verify-ca` without a root file trusts nothing). The CA, certificate and
///   key files must be readable, the key unencrypted, and the host usable as a TLS name.
/// - Options go through DRV-05 [`sanitize`] with the TLS files listed as paths and `roots`
///   approved; a file outside them (untrusted origins only) is `PathRefused`.
///
/// Blocking: it reads the TLS files and checks the SQLite path. Call it off the UI thread.
pub fn validate_with(
    p: &Profile,
    catalog: &EngineCatalog,
    roots: &ApprovedRoots,
) -> Vec<ValidationIssue> {
    checked(&effective_profile(p), catalog, &roots.list()).0
}

/// The issues of an effective profile and, when there are none, the sanitized options.
pub(crate) fn checked(
    p: &Profile,
    catalog: &EngineCatalog,
    roots: &[PathBuf],
) -> (Vec<ValidationIssue>, Vec<(String, String)>) {
    let mut issues: Vec<ValidationIssue> = Vec::new();
    let mut push = |field: Option<FieldId>, code: IssueCode| {
        let issue = ValidationIssue { field, code };
        if !issues.contains(&issue) {
            issues.push(issue);
        }
    };
    if p.name.trim().is_empty() {
        push(None, IssueCode::NameMissing);
    }
    let file_based = p.engine == DbmsKind::Sqlite;
    match &p.endpoint {
        ProfileEndpoint::Server { host, port } => {
            if file_based {
                push(Some(FieldId::File), IssueCode::FileMissing);
            } else if host.trim().is_empty() {
                push(Some(FieldId::Host), IssueCode::HostMissing);
            } else if host_invalid(host.trim()) {
                push(Some(FieldId::Host), IssueCode::HostInvalid);
            }
            if *port == Some(0) {
                push(Some(FieldId::Port), IssueCode::PortInvalid);
            }
        }
        ProfileEndpoint::Socket { path, port } => {
            if file_based {
                push(Some(FieldId::File), IssueCode::FileMissing);
            } else if path.as_os_str().is_empty() {
                push(Some(FieldId::Socket), IssueCode::HostMissing);
            }
            if *port == Some(0) {
                push(Some(FieldId::Port), IssueCode::PortInvalid);
            }
        }
        ProfileEndpoint::File {
            path,
            create_if_missing,
        } => {
            if !file_based {
                push(Some(FieldId::Host), IssueCode::HostMissing);
            } else if let Some(code) = sqlite_path_issue(path, *create_if_missing) {
                push(Some(FieldId::File), code);
            }
            if file_based && *create_if_missing && p.read_only {
                push(Some(FieldId::File), IssueCode::CreateAndReadOnly);
            }
        }
    }
    if !matches!(p.transport, TransportSpec::Direct) {
        push(None, IssueCode::TransportUnavailable);
    }
    if let ProfileEndpoint::Server { host, .. } = &p.endpoint
        && !file_based
        && p.tls.mode != TlsMode::Disable
    {
        let mysql_family = catalog.chain(p.engine).contains(&DbmsKind::MySql);
        if mysql_family && p.tls.mode == TlsMode::VerifyCa {
            push(
                None,
                IssueCode::TlsModeUnsupported {
                    mode: TlsMode::VerifyCa,
                },
            );
        } else if p.tls.mode == TlsMode::VerifyCa && p.tls.ca_file.is_none() {
            push(None, IssueCode::TlsCaUnreadable);
        } else if !host.trim().is_empty() && !host_invalid(host.trim()) {
            // The address class only decides `plaintext_allowed`, which is not checked here.
            if let Err(e) = build_tls_params(&p.tls, host.trim(), AddressClass::Private) {
                let (field, code) = tls_issue(&e);
                push(field, code);
            }
        }
    }
    let paths = tls_paths(p);
    let options = match sanitize(
        &SanitizeInput {
            kind: p.engine,
            origin: &p.origin,
            options: &p.extra,
            paths: &paths,
            unsafe_toggle: false,
            approved_roots: roots,
        },
        catalog,
    ) {
        Ok(options) => options,
        Err(violations) => {
            for v in violations {
                if v.reason == ViolationReason::PathOutsideRoots {
                    push(None, IssueCode::PathRefused);
                } else {
                    push(
                        None,
                        IssueCode::OptionRejected {
                            key: v.key,
                            reason: v.reason,
                        },
                    );
                }
            }
            Vec::new()
        }
    };
    (issues, options)
}

/// A host no resolver or TLS stack should see: ASCII whitespace, a control character, or URL
/// syntax (`/ \ @ ? #`).
fn host_invalid(host: &str) -> bool {
    host.chars()
        .any(|c| c.is_ascii_whitespace() || c.is_control() || "/\\@?#".contains(c))
}

/// The issue of a TLS setup problem; an unusable host name is a host issue.
pub(crate) fn tls_issue(e: &TlsSetupError) -> (Option<FieldId>, IssueCode) {
    match e {
        TlsSetupError::CaFileUnreadable(_) => (None, IssueCode::TlsCaUnreadable),
        TlsSetupError::CertFileUnreadable(_) | TlsSetupError::KeyUnreadable(_) => {
            (None, IssueCode::TlsCertUnreadable)
        }
        TlsSetupError::KeyEncrypted(_) => (None, IssueCode::TlsKeyEncrypted),
        TlsSetupError::InvalidServerName(_) => (Some(FieldId::Host), IssueCode::HostInvalid),
    }
}

/// The profile's TLS files, under the engine's option names, for the DRV-05 path rule.
fn tls_paths(p: &Profile) -> Vec<(String, PathBuf)> {
    let (ca, cert, key) = if p.engine == DbmsKind::Postgres {
        ("sslrootcert", "sslcert", "sslkey")
    } else {
        ("sslCa", "sslCert", "sslKey")
    };
    let mut paths = Vec::new();
    if let Some(file) = &p.tls.ca_file {
        paths.push((ca.to_string(), file.clone()));
    }
    if let Some(client) = &p.tls.client_cert {
        paths.push((cert.to_string(), client.cert.clone()));
        paths.push((key.to_string(), client.key.clone()));
    }
    paths
}

/// The issue of a SQLite path, if any (T15's rules, checked before any I/O so the form can
/// show them): `PathRefused` for a path SQLite must never open, `FileMissing` for a missing
/// file that may not be created.
fn sqlite_path_issue(path: &Path, create_if_missing: bool) -> Option<IssueCode> {
    let text = path.to_string_lossy();
    if text == SQLITE_MEMORY {
        return None;
    }
    if text.is_empty() {
        return Some(IssueCode::FileMissing);
    }
    let refused_text = text
        .as_bytes()
        .get(..5)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"file:"))
        || text.starts_with("\\\\")
        || text.starts_with("//")
        || text.starts_with("\\??\\");
    if refused_text || !path.is_absolute() || refused_prefix(path) {
        return Some(IssueCode::PathRefused);
    }
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => None,
        Ok(_) => Some(IssueCode::PathRefused),
        Err(_) if create_if_missing => None,
        Err(_) => Some(IssueCode::FileMissing),
    }
}

/// A Windows prefix that never names a local file: a UNC share or a device namespace.
fn refused_prefix(path: &Path) -> bool {
    use std::path::Prefix;
    match path.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::DeviceNS(_) | Prefix::UNC(..) | Prefix::VerbatimUNC(..) => true,
            Prefix::Verbatim(name) => name.eq_ignore_ascii_case("GLOBALROOT"),
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) => false,
        },
        _ => false,
    }
}

/// An option key folded for matching: ASCII lowercase, without `-` and `_` (`sslMode`,
/// `ssl-mode` and `ssl_mode` are one key).
fn fold_key(key: &str) -> String {
    key.chars()
        .filter(|c| *c != '-' && *c != '_')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// `p` with the client-side driver options of its extras applied (the drivers read none of
/// them). An option overrides the profile's own setting, as the URL that carried it asked.
/// Keys match after [`fold_key`] (so JDBC `sslMode`, MySQL-client `ssl-mode` and libpq
/// `sslmode` are one key); values fold ASCII case and `_`/`-`.
///
/// - Mode (`sslmode`): `disable`/`disabled`; `allow`, `prefer`, `preferred` are Auto;
///   `require`/`required`; `verify-ca`; `verify-full`/`verify-identity`. MySQL and MariaDB also
///   read `useSSL` (`false` is Disable, `true` Require) when no mode is given.
/// - Files: the CA (`sslrootcert` for PostgreSQL, where `system` keeps the platform roots;
///   `sslCa`/`ssl-ca` for MySQL and MariaDB) and the client certificate with its key
///   (`sslcert` + `sslkey`, `ssl-cert` + `ssl-key`).
/// - Timeout: `connect_timeout` in seconds; MySQL's JDBC `connectTimeout` in milliseconds,
///   rounded up.
///
/// An unknown value changes nothing.
pub(crate) fn effective_profile(p: &Profile) -> Profile {
    let mut e = p.clone();
    let mysql = matches!(p.engine, DbmsKind::MySql | DbmsKind::MariaDb);
    if p.engine != DbmsKind::Postgres && !mysql {
        return e;
    }
    // The last value of a key wins.
    let get = |folded: &str| {
        p.extra
            .iter()
            .rev()
            .find(|(k, _)| fold_key(k) == folded)
            .map(|(k, v)| (k.as_str(), v.trim()))
    };
    let value = |v: &str| v.to_ascii_lowercase().replace('_', "-");
    let mode = match get("sslmode").map(|(_, v)| value(v)).as_deref() {
        Some("disable" | "disabled") => Some(TlsMode::Disable),
        Some("allow" | "prefer" | "preferred") => Some(TlsMode::Auto),
        Some("require" | "required") => Some(TlsMode::Require),
        Some("verify-ca") => Some(TlsMode::VerifyCa),
        Some("verify-full" | "verify-identity") => Some(TlsMode::VerifyFull),
        Some(_) => None,
        None if mysql => match get("usessl").map(|(_, v)| value(v)).as_deref() {
            Some("false") => Some(TlsMode::Disable),
            Some("true") => Some(TlsMode::Require),
            _ => None,
        },
        None => None,
    };
    if let Some(mode) = mode {
        e.tls.mode = mode;
    }
    let ca_key = if mysql { "sslca" } else { "sslrootcert" };
    if let Some((_, v)) = get(ca_key) {
        let system = !mysql && v.eq_ignore_ascii_case("system");
        e.tls.ca_file = (!system && !v.is_empty()).then(|| PathBuf::from(v));
    }
    if let (Some((_, cert)), Some((_, key))) = (get("sslcert"), get("sslkey"))
        && !cert.is_empty()
        && !key.is_empty()
    {
        e.tls.client_cert = Some(ClientCertPaths {
            cert: PathBuf::from(cert),
            key: PathBuf::from(key),
        });
    }
    if let Some((key, v)) = get("connecttimeout")
        && let Ok(n) = v.parse::<u64>()
        && n > 0
    {
        // JDBC's `connectTimeout` (no separator) counts milliseconds.
        let jdbc_ms = mysql && !key.contains(['_', '-']);
        let seconds = if jdbc_ms { n.div_ceil(1000) } else { n };
        e.connect_timeout_s = u32::try_from(seconds).unwrap_or(u32::MAX);
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_proto::{ImportSource, Origin};

    fn pg(extra: &[(&str, &str)]) -> Profile {
        Profile {
            name: "orders".into(),
            endpoint: ProfileEndpoint::Server {
                host: "127.0.0.1".into(),
                port: None,
            },
            extra: extra
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Profile::default()
        }
    }

    fn codes(p: &Profile, roots: &ApprovedRoots) -> Vec<IssueCode> {
        validate_with(p, EngineCatalog::embedded(), roots)
            .into_iter()
            .map(|i| i.code)
            .collect()
    }

    #[test]
    fn libpq_jdbc_and_mysql_client_options_set_the_effective_tls_settings_and_timeout() {
        let e = effective_profile(&pg(&[("SSLMODE", "verify-full"), ("connect_timeout", "7")]));
        assert_eq!((e.tls.mode, e.connect_timeout_s), (TlsMode::VerifyFull, 7));
        let e = effective_profile(&pg(&[("sslmode", "bogus")]));
        assert_eq!(e.tls.mode, TlsMode::Auto);
        let e = effective_profile(&pg(&[("sslrootcert", "system")]));
        assert_eq!(e.tls.ca_file, None);
        let e = effective_profile(&pg(&[("sslrootcert", "/certs/ca.pem")]));
        assert_eq!(e.tls.ca_file, Some(PathBuf::from("/certs/ca.pem")));

        let mut my = pg(&[("sslMode", "verify_ca"), ("connectTimeout", "2500")]);
        my.engine = DbmsKind::MySql;
        let e = effective_profile(&my);
        assert_eq!((e.tls.mode, e.connect_timeout_s), (TlsMode::VerifyCa, 3));
        assert!(
            validate(&my, EngineCatalog::embedded()).contains(&ValidationIssue {
                field: None,
                code: IssueCode::TlsModeUnsupported {
                    mode: TlsMode::VerifyCa
                }
            })
        );
        my.extra = vec![("useSSL".into(), "false".into())];
        assert_eq!(effective_profile(&my).tls.mode, TlsMode::Disable);

        // JDBC `sslCa`/`sslCert`/`sslKey` and the MySQL client's `ssl-ca`/`ssl-cert`/`ssl-key`.
        for (ca, cert, key, mode) in [
            ("sslCa", "sslCert", "sslKey", "REQUIRED"),
            ("ssl-ca", "ssl-cert", "ssl-key", "verify_identity"),
        ] {
            my.extra = [
                (ca, "/c/ca.pem"),
                (cert, "/c/me.pem"),
                (key, "/c/me.key"),
                ("ssl-mode", mode),
                ("connect_timeout", "4"),
            ]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
            let e = effective_profile(&my);
            assert_eq!(e.tls.ca_file, Some(PathBuf::from("/c/ca.pem")), "{ca}");
            assert_eq!(
                e.tls.client_cert,
                Some(ClientCertPaths {
                    cert: "/c/me.pem".into(),
                    key: "/c/me.key".into()
                })
            );
            assert_ne!(e.tls.mode, TlsMode::Auto, "{mode}");
            assert_eq!(e.connect_timeout_s, 4, "seconds, not JDBC milliseconds");
        }
    }

    #[test]
    fn sqlite_paths_must_be_absolute_local_regular_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shop.db");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(sqlite_path_issue(&file, false), None);
        assert_eq!(sqlite_path_issue(Path::new(SQLITE_MEMORY), false), None);
        // A directory exists but is not a regular file.
        assert_eq!(
            sqlite_path_issue(dir.path(), true),
            Some(IssueCode::PathRefused)
        );
        // Missing: only when it may be created.
        let new = dir.path().join("new.db");
        assert_eq!(sqlite_path_issue(&new, false), Some(IssueCode::FileMissing));
        assert_eq!(sqlite_path_issue(&new, true), None);
        assert_eq!(
            sqlite_path_issue(Path::new(""), true),
            Some(IssueCode::FileMissing)
        );
        for bad in [
            "shop.db",
            "file:shop.db?vfs=unix-none",
            "FILE:/tmp/x.db",
            "\\\\server\\share\\x.db",
            "\\\\?\\UNC\\server\\share\\x.db",
            "\\\\.\\PhysicalDrive0",
            "\\??\\C:\\x.db",
            "//server/share/x.db",
        ] {
            assert_eq!(
                sqlite_path_issue(Path::new(bad), true),
                Some(IssueCode::PathRefused),
                "{bad}"
            );
        }
    }

    #[test]
    fn unusable_hosts_are_host_invalid() {
        for bad in ["db host", "db\thost", "user@db", "db/x", "db?x", "db#x"] {
            let mut p = pg(&[]);
            p.endpoint = ProfileEndpoint::Server {
                host: bad.into(),
                port: None,
            };
            assert_eq!(
                validate(&p, EngineCatalog::embedded())[0],
                ValidationIssue {
                    field: Some(FieldId::Host),
                    code: IssueCode::HostInvalid
                },
                "{bad:?}"
            );
        }
        assert_eq!(
            tls_issue(&TlsSetupError::InvalidServerName("x".into())).1,
            IssueCode::HostInvalid
        );
        let mut p = pg(&[]);
        p.endpoint = ProfileEndpoint::Server {
            host: "[::1]".into(),
            port: None,
        };
        assert!(validate(&p, EngineCatalog::embedded()).is_empty());
    }

    #[test]
    fn an_imported_tls_file_must_lie_under_an_approved_root() {
        let home = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let ca = include_str!("../../tests/fixtures/test-ca.pem");
        let inside = home.path().join("ca.pem");
        let outside = elsewhere.path().join("ca.pem");
        std::fs::write(&inside, ca).unwrap();
        std::fs::write(&outside, ca).unwrap();
        let roots = ApprovedRoots::new(vec![home.path().to_path_buf()]);
        let imported = |file: &Path| {
            let mut p = pg(&[]);
            p.origin = Origin::Imported {
                source: ImportSource::PgService,
            };
            p.tls.mode = TlsMode::VerifyFull;
            p.tls.ca_file = Some(file.to_path_buf());
            p
        };
        assert_eq!(codes(&imported(&inside), &roots), []);
        assert_eq!(codes(&imported(&outside), &roots), [IssueCode::PathRefused]);
        // A file the user picked is approved from then on, for every clone of the roots.
        roots.clone().add(outside.clone());
        assert_eq!(codes(&imported(&outside), &roots), []);
        // A profile the user typed is never path-checked.
        let mut typed = imported(&elsewhere.path().join("ca.pem"));
        typed.origin = Origin::UserTyped;
        assert_eq!(
            codes(&typed, &ApprovedRoots::new(Vec::new())),
            Vec::<IssueCode>::new()
        );
        // A CA file an untrusted URL carried in its extras (`sslrootcert`, `sslCa`, `ssl-ca`)
        // takes the same rule.
        for (engine, key) in [
            (DbmsKind::Postgres, "sslrootcert"),
            (DbmsKind::MySql, "sslCa"),
            (DbmsKind::MySql, "ssl-ca"),
        ] {
            let mut p = imported(&inside);
            p.tls.ca_file = None;
            p.engine = engine;
            p.extra = vec![(key.into(), outside.display().to_string())];
            let roots = ApprovedRoots::new(vec![home.path().to_path_buf()]);
            let found = codes(&p, &roots);
            assert!(found.contains(&IssueCode::PathRefused), "{key}: {found:?}");
            p.extra = vec![(key.into(), inside.display().to_string())];
            assert!(
                !codes(&p, &roots).contains(&IssueCode::PathRefused),
                "{key}"
            );
        }
        // The user's default roots are the home directory.
        assert_eq!(
            ApprovedRoots::for_user().list(),
            std::env::home_dir().into_iter().collect::<Vec<_>>()
        );
    }
}
