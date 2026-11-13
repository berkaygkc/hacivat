//! Step 0 of a connect: profile issues and option sanitizing (HV-CONN-01, HV-DRV-05).
//!
//! Validation and the connect read the profile's typed settings only. Options with a typed
//! setting (`sslmode`, `sslrootcert`, `connect_timeout`, ...) never override it here: they are
//! folded into it where they enter a profile (`profile::fold_typed_options`: a paste, the
//! import, the dialog, a load and every save), so the form, the quick fixes and the connect
//! agree (M2a final review I2). One that is still in `extra` changes nothing (no driver reads
//! it) and goes through DRV-05 like any other option.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use hacivat_driver::url::FieldId;
use hacivat_driver::{EngineCatalog, SanitizeInput, ViolationReason, sanitize};
use hacivat_net::{AddressClass, TlsMode, TlsSetupError, build_tls_params};
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
    // ## m2a-finalfix
    /// An extra option whose key carries a password (`hacivat_proto::is_secret_param`): a
    /// password belongs in the password field, never in `extra` (M2a final review I1).
    OptionSecret {
        key: String,
    },
    // ## m2b-t01-interfaces: Task 12 checks them; Task 20 renders the SSH ones (Q9).
    /// An SSH hop has no host.
    SshHostMissing,
    /// An SSH hop has no user.
    SshUserMissing,
    /// More hops than a tunnel allows.
    SshTooManyHops,
    /// An SSH key file is missing or unreadable.
    SshKeyFileMissing,
    /// A legacy OpenSSL-encrypted TLS key (`Proc-Type: 4,ENCRYPTED`), which cannot be
    /// decrypted (Task 3, Task 19).
    TlsKeyLegacyEncrypted,
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
///   key files must be readable, the key unencrypted, and the host usable as a TLS name. For
///   every origin, a TLS file is never a UNC share or a device path (`\\server\share`, `//`,
///   `\\?\UNC\`, `\\.\`, `\??\`): `PathRefused`, as for SQLite.
/// - An extra option whose key carries a password is `OptionSecret`.
/// - Options go through DRV-05 [`sanitize`] with the TLS files listed as paths and `roots`
///   approved; a file outside them (untrusted origins only) is `PathRefused`.
///
/// Blocking: it reads the TLS files and checks the SQLite path. Call it off the UI thread.
pub fn validate_with(
    p: &Profile,
    catalog: &EngineCatalog,
    roots: &ApprovedRoots,
) -> Vec<ValidationIssue> {
    checked(p, catalog, &roots.list()).0
}

/// The issues of a profile and, when there are none, the sanitized options.
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
    // A TLS file on a share or a device is refused before anything opens, resolves or stats it
    // (N2): neither the TLS check below nor the DRV-05 path rule may touch it.
    let all_paths = tls_paths(p);
    let tls_refused = all_paths.iter().any(|(_, path)| network_path(path));
    if tls_refused {
        push(None, IssueCode::PathRefused);
    }
    if let ProfileEndpoint::Server { host, .. } = &p.endpoint
        && !file_based
        && p.tls.mode != TlsMode::Disable
        && !tls_refused
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
            // The address class only decides `plaintext_allowed`, which is not checked here;
            // nor are pins (the port only names the pin endpoint).
            let port = match &p.endpoint {
                ProfileEndpoint::Server { port, .. } => *port,
                _ => None,
            }
            .or_else(|| catalog.get(p.engine).and_then(|e| e.default_port))
            .unwrap_or_default();
            if let Err(e) = build_tls_params(&p.tls, host.trim(), port, AddressClass::Private, None)
            {
                let (field, code) = tls_issue(&e);
                push(field, code);
            }
        }
    }
    let paths: Vec<(String, PathBuf)> = all_paths
        .into_iter()
        .filter(|(_, path)| !network_path(path))
        .collect();
    for (key, _) in &p.extra {
        if hacivat_proto::is_secret_param(key) {
            push(None, IssueCode::OptionSecret { key: key.clone() });
        }
    }
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
        // ## m2b-t01-interfaces
        TlsSetupError::KeyEncryptedLegacy(_) => (None, IssueCode::TlsKeyLegacyEncrypted),
        TlsSetupError::KeyPassphraseWrong(_) => (None, IssueCode::TlsKeyEncrypted),
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
    let file_uri = text
        .as_bytes()
        .get(..5)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"file:"));
    if file_uri || network_path(path) || !path.is_absolute() {
        return Some(IssueCode::PathRefused);
    }
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => None,
        Ok(_) => Some(IssueCode::PathRefused),
        Err(_) if create_if_missing => None,
        Err(_) => Some(IssueCode::FileMissing),
    }
}

/// A path that never names a local file, on any OS (`hacivat_net::is_network_path`): a UNC
/// share, `//host`, `\\?\UNC\`, `\\.\` or `\??\`. Checked before any file system call.
fn network_path(path: &Path) -> bool {
    hacivat_net::is_network_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_net::ClientCertPaths;
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

    // ## m2a-finalfix
    /// I2: an extra never overrides a typed setting. A profile whose extras still name a TLS
    /// mode validates (and connects) with its typed mode; the fold that runs where options
    /// enter a profile is what applies them.
    #[test]
    fn extras_never_override_the_typed_tls_settings() {
        let mut p = pg(&[("sslmode", "verify-ca")]);
        p.tls.mode = TlsMode::Require;
        // Verify CA without a CA file would be an issue: the typed Require is what counts.
        assert_eq!(codes(&p, &ApprovedRoots::new(Vec::new())), []);

        let mut my = pg(&[("sslMode", "verify_ca")]);
        my.engine = DbmsKind::MySql;
        assert_eq!(codes(&my, &ApprovedRoots::new(Vec::new())), []);
        crate::profile::fold_typed_options(&mut my);
        assert!(
            validate(&my, EngineCatalog::embedded()).contains(&ValidationIssue {
                field: None,
                code: IssueCode::TlsModeUnsupported {
                    mode: TlsMode::VerifyCa
                }
            })
        );
    }

    /// I1: a password never sits in the extras, whatever its key's spelling.
    #[test]
    fn a_password_option_is_an_issue() {
        for key in [
            "pgpassword",
            "db_password",
            "--pwd",
            "SSLPassword",
            "trustCertificateKeyStorePassword",
            "ssl_passphrase",
        ] {
            let p = pg(&[(key, "hv_test_pw")]);
            assert_eq!(
                codes(&p, &ApprovedRoots::new(Vec::new())),
                [IssueCode::OptionSecret { key: key.into() }],
                "{key}"
            );
        }
        assert_eq!(
            codes(
                &pg(&[("pass_through", "x")]),
                &ApprovedRoots::new(Vec::new())
            ),
            []
        );
    }

    /// I5: a TLS file is never a network or device path, whatever the profile's origin.
    #[test]
    fn tls_files_on_a_share_or_a_device_are_refused_for_every_origin() {
        for bad in [
            r"\\server\share\ca.pem",
            "//server/share/ca.pem",
            r"\\?\UNC\server\share\ca.pem",
            r"\\.\PhysicalDrive0",
            r"\??\C:\ca.pem",
        ] {
            for origin in [
                Origin::UserTyped,
                Origin::Imported {
                    source: ImportSource::Url,
                },
            ] {
                let mut p = pg(&[]);
                p.origin = origin;
                p.tls.mode = TlsMode::VerifyFull;
                p.tls.ca_file = Some(PathBuf::from(bad));
                assert!(
                    codes(&p, &ApprovedRoots::new(Vec::new())).contains(&IssueCode::PathRefused),
                    "{bad}"
                );
                p.tls.ca_file = None;
                p.tls.client_cert = Some(ClientCertPaths {
                    cert: PathBuf::from(bad),
                    key: PathBuf::from(bad),
                });
                assert!(
                    codes(&p, &ApprovedRoots::new(Vec::new())).contains(&IssueCode::PathRefused),
                    "{bad}"
                );
            }
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
            // Folded into the typed CA, as an import or a paste does (I2).
            crate::profile::fold_typed_options(&mut p);
            let roots = ApprovedRoots::new(vec![home.path().to_path_buf()]);
            let found = codes(&p, &roots);
            assert!(found.contains(&IssueCode::PathRefused), "{key}: {found:?}");
            p.extra = vec![(key.into(), inside.display().to_string())];
            crate::profile::fold_typed_options(&mut p);
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
