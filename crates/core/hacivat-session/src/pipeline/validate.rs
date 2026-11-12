//! Step 0 of a connect: profile issues and option sanitizing (HV-CONN-01, HV-DRV-05).
//!
//! Validation runs on the profile's *effective* settings ([`effective_profile`]): libpq and
//! JDBC client-side options that a URL carried into the profile's extra options (`sslmode`,
//! `sslMode`, `useSSL`, `sslrootcert`, `connect_timeout`, ...) are applied to its TLS settings
//! and timeout first, because no driver reads them (T13, T14).

use std::path::{Component, Path, PathBuf};

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
    PortInvalid,
    FileMissing,
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

/// The name SQLite reads as an in-memory database rather than a file.
const SQLITE_MEMORY: &str = ":memory:";

/// Every issue of `p`, in form order: name, endpoint, transport, TLS, then options.
///
/// - The name must not be blank.
/// - A server needs a host and a port that is not 0; a socket needs a path.
/// - A SQLite file must be an absolute local path (or `:memory:`): never a `file:` URI, a UNC
///   share (`\\server\share`, `\\?\UNC\…`) or a device path (`\\.\…`, `\??\…`), and when
///   something exists there it must be a regular file. A missing file is an issue only when it
///   may not be created; creating a read-only database is an issue.
/// - SSH tunnels and the Karagöz transport cannot connect in M2a (R4).
/// - TLS: `VerifyCa` is unsupported for MySQL and MariaDB (P11); elsewhere it needs a CA file
///   (libpq parity: `verify-ca` without a root file trusts nothing). The CA, certificate and
///   key files must be readable, the key unencrypted, and the host usable as a TLS name.
/// - Options go through DRV-05 [`sanitize`], with the TLS files listed as paths.
///
/// Blocking: it reads the TLS files and checks the SQLite path. Call it off the UI thread.
pub fn validate(p: &Profile, catalog: &EngineCatalog) -> Vec<ValidationIssue> {
    checked(&effective_profile(p), catalog).0
}

/// The issues of an effective profile and, when there are none, the sanitized options.
pub(crate) fn checked(
    p: &Profile,
    catalog: &EngineCatalog,
) -> (Vec<ValidationIssue>, Vec<(String, String)>) {
    let mut issues = Vec::new();
    let mut push = |field: Option<FieldId>, code: IssueCode| {
        issues.push(ValidationIssue { field, code });
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
            } else if !sqlite_path_ok(path, *create_if_missing) {
                push(Some(FieldId::File), IssueCode::FileMissing);
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
        } else if !host.trim().is_empty() {
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
            approved_roots: &[],
        },
        catalog,
    ) {
        Ok(options) => options,
        Err(violations) => {
            for v in violations {
                push(
                    None,
                    IssueCode::OptionRejected {
                        key: v.key,
                        reason: v.reason,
                    },
                );
            }
            Vec::new()
        }
    };
    (issues, options)
}

/// The issue of a TLS setup problem; an unusable host name is a host issue.
pub(crate) fn tls_issue(e: &TlsSetupError) -> (Option<FieldId>, IssueCode) {
    match e {
        TlsSetupError::CaFileUnreadable(_) => (None, IssueCode::TlsCaUnreadable),
        TlsSetupError::CertFileUnreadable(_) | TlsSetupError::KeyUnreadable(_) => {
            (None, IssueCode::TlsCertUnreadable)
        }
        TlsSetupError::KeyEncrypted(_) => (None, IssueCode::TlsKeyEncrypted),
        TlsSetupError::InvalidServerName(_) => (Some(FieldId::Host), IssueCode::HostMissing),
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

/// Whether `path` may be opened as a SQLite database (T15's rules, checked before any I/O so
/// the form can show them).
fn sqlite_path_ok(path: &Path, create_if_missing: bool) -> bool {
    let text = path.to_string_lossy();
    if text == SQLITE_MEMORY {
        return true;
    }
    let refused_text = text.is_empty()
        || text
            .as_bytes()
            .get(..5)
            .is_some_and(|head| head.eq_ignore_ascii_case(b"file:"))
        || text.starts_with("\\\\")
        || text.starts_with("//")
        || text.starts_with("\\??\\");
    if refused_text || !path.is_absolute() || refused_prefix(path) {
        return false;
    }
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file(),
        Err(_) => create_if_missing,
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

/// `p` with the client-side driver options of its extras applied (the drivers read none of
/// them). An option overrides the profile's own setting, as the URL that carried it asked:
///
/// - PostgreSQL (libpq): `sslmode` (`disable`; `allow` and `prefer` are Auto; `require`,
///   `verify-ca`, `verify-full`), `sslrootcert` (a file; `system` keeps the platform roots),
///   `sslcert` with `sslkey`, `connect_timeout` (seconds).
/// - MySQL and MariaDB (JDBC): `sslMode` (`DISABLED`, `PREFERRED`, `REQUIRED`, `VERIFY_CA`,
///   `VERIFY_IDENTITY`), else `useSSL` (`false` is Disable, `true` Require), `connectTimeout`
///   (milliseconds, rounded up to seconds).
///
/// Keys and values compare with ASCII case folding only. An unknown value changes nothing.
pub(crate) fn effective_profile(p: &Profile) -> Profile {
    let mut e = p.clone();
    let get = |key: &str| {
        p.extra
            .iter()
            .rev()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.trim())
    };
    let is = |v: &str, word: &str| v.eq_ignore_ascii_case(word);
    match p.engine {
        DbmsKind::Postgres => {
            if let Some(v) = get("sslmode") {
                let mode = match v {
                    _ if is(v, "disable") => Some(TlsMode::Disable),
                    _ if is(v, "allow") || is(v, "prefer") => Some(TlsMode::Auto),
                    _ if is(v, "require") => Some(TlsMode::Require),
                    _ if is(v, "verify-ca") => Some(TlsMode::VerifyCa),
                    _ if is(v, "verify-full") => Some(TlsMode::VerifyFull),
                    _ => None,
                };
                if let Some(mode) = mode {
                    e.tls.mode = mode;
                }
            }
            if let Some(v) = get("sslrootcert") {
                e.tls.ca_file = (!is(v, "system") && !v.is_empty()).then(|| PathBuf::from(v));
            }
            if let (Some(cert), Some(key)) = (get("sslcert"), get("sslkey"))
                && !cert.is_empty()
                && !key.is_empty()
            {
                e.tls.client_cert = Some(ClientCertPaths {
                    cert: PathBuf::from(cert),
                    key: PathBuf::from(key),
                });
            }
            if let Some(seconds) = get("connect_timeout").and_then(|v| v.parse::<u32>().ok())
                && seconds > 0
            {
                e.connect_timeout_s = seconds;
            }
        }
        DbmsKind::MySql | DbmsKind::MariaDb => {
            let mode = match get("sslMode") {
                Some(v) if is(v, "DISABLED") => Some(TlsMode::Disable),
                Some(v) if is(v, "PREFERRED") => Some(TlsMode::Auto),
                Some(v) if is(v, "REQUIRED") => Some(TlsMode::Require),
                Some(v) if is(v, "VERIFY_CA") => Some(TlsMode::VerifyCa),
                Some(v) if is(v, "VERIFY_IDENTITY") => Some(TlsMode::VerifyFull),
                Some(_) => None,
                None => match get("useSSL") {
                    Some(v) if is(v, "false") => Some(TlsMode::Disable),
                    Some(v) if is(v, "true") => Some(TlsMode::Require),
                    _ => None,
                },
            };
            if let Some(mode) = mode {
                e.tls.mode = mode;
            }
            if let Some(ms) = get("connectTimeout").and_then(|v| v.parse::<u64>().ok())
                && ms > 0
            {
                e.connect_timeout_s = u32::try_from(ms.div_ceil(1000)).unwrap_or(u32::MAX);
            }
        }
        _ => {}
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn libpq_and_jdbc_options_set_the_effective_tls_mode_and_timeout() {
        let e = effective_profile(&pg(&[("SSLMODE", "verify-full"), ("connect_timeout", "7")]));
        assert_eq!((e.tls.mode, e.connect_timeout_s), (TlsMode::VerifyFull, 7));
        let e = effective_profile(&pg(&[("sslmode", "bogus")]));
        assert_eq!(e.tls.mode, TlsMode::Auto);
        let e = effective_profile(&pg(&[("sslrootcert", "system")]));
        assert_eq!(e.tls.ca_file, None);

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
    }

    #[test]
    fn sqlite_paths_must_be_absolute_local_regular_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shop.db");
        std::fs::write(&file, b"").unwrap();
        assert!(sqlite_path_ok(&file, false));
        assert!(sqlite_path_ok(Path::new(SQLITE_MEMORY), false));
        // A directory exists but is not a regular file.
        assert!(!sqlite_path_ok(dir.path(), true));
        // Missing: only when it may be created.
        assert!(!sqlite_path_ok(&dir.path().join("new.db"), false));
        assert!(sqlite_path_ok(&dir.path().join("new.db"), true));
        for bad in [
            "shop.db",
            "file:shop.db?vfs=unix-none",
            "FILE:/tmp/x.db",
            "\\\\server\\share\\x.db",
            "\\\\?\\UNC\\server\\share\\x.db",
            "\\\\.\\PhysicalDrive0",
            "\\??\\C:\\x.db",
            "//server/share/x.db",
            "",
        ] {
            assert!(!sqlite_path_ok(Path::new(bad), true), "{bad}");
        }
    }
}
