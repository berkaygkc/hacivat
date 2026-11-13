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
use hacivat_net::paths::{fs_probe, local_path};
use hacivat_net::ssh::{SshAuth, SshSpec};
use hacivat_net::{AddressClass, TlsMode, TlsSetupError, build_tls_params};
use hacivat_proto::{DbmsKind, Origin};
use hacivat_secrets::MAX_SSH_HOP;

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
    /// MySQL and MariaDB with `VerifyCa` in M2a (P11). No longer produced since M2b, which
    /// verifies the chain through Hacivat's own TLS configuration for them too (B10).
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
    // ## m2b-t19-tls
    /// `Disable` while the endpoint has a pin (`TlsSettings::disable_ignores_pin`, T3 ruling
    /// M2): the pin is not checked. A notice, never a refusal: Validate does not report it
    /// (Disable is the user's explicit choice and wins, B5); the connection dialog shows it.
    TlsDisableIgnoresPin,
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
/// - The Karagöz transport cannot connect (R4, `TransportUnavailable`), and neither can an SSH
///   tunnel until the pipeline's SSH step exists (Task 14). An SSH tunnel needs at least one
///   hop and at most nine (`SshTooManyHops`: `MAX_SSH_HOP` is the last hop with a secret slot),
///   each with a host and a user (`SshHostMissing`, `SshUserMissing`), a host as usable as the
///   database host's (`HostInvalid`) and a port that is not 0 (`PortInvalid`); a hop that signs in with
///   a key file needs a path naming an existing file (`SshKeyFileMissing`). A key file or an
///   imported `IdentityFile` on a share or a device is `PathRefused`.
/// - TLS: `VerifyCa` needs a CA file, for every engine (libpq parity: `verify-ca` without a
///   root file trusts nothing; MySQL and MariaDB verify the chain since M2b, B10). The CA,
///   certificate and key files must be readable, the key not encrypted the legacy OpenSSL way
///   (`TlsKeyLegacyEncrypted`), and the host usable as a TLS name. For every origin, a TLS file
///   is never a UNC share or a device path (`\\server\share`, `//`, `\\?\UNC\`, `\\.\`,
///   `\??\`): `PathRefused`, as for SQLite.
/// - **Approved roots (B26, Q22).** For a profile from an untrusted origin (anything the user
///   did not type), every TLS file, the SQLite file and every SSH key path must lie under one of
///   `roots`, compared on canonical forms: the path and each root are canonicalised (symlinks
///   resolved, so `/var/folders/…` and `/private/var/…` match, and a link out of a root is
///   outside it), a Windows `\\?\C:\` is read as `C:\`, and a missing file (a new SQLite
///   database) is resolved through its parent directory. A path is outside when it cannot be
///   canonicalised (relative, a dangling symlink, a missing folder) or canonicalises to a share
///   or a device. Outside is `PathRefused`, and a TLS file outside is never read. The text
///   rules above run first, on the path as typed.
/// - An extra option whose key carries a password is `OptionSecret`.
/// - Options go through DRV-05 [`sanitize`] with `roots` approved; a path option outside them
///   (untrusted origins only) is `PathRefused`.
///
/// Blocking: it reads the TLS files and checks the SQLite and key paths. Call it off the UI
/// thread. The paths are checked, not handed over open: the driver and the tunnel open them
/// again by path (a check-then-use window, accepted in ADR 0003, `m2b-t12`).
pub fn validate_with(
    p: &Profile,
    catalog: &EngineCatalog,
    roots: &ApprovedRoots,
) -> Vec<ValidationIssue> {
    checked(p, catalog, &roots.list(), KeyCheck::Passphrase(None)).0
}

// ## m2b-t14-pipeline
/// [`validate_with`] opening an encrypted TLS client key with `passphrase` (the dialog's field,
/// T3 carry-in): with the right one there is no `TlsKeyEncrypted` issue; a wrong one is
/// `TlsKeyEncrypted` too.
pub fn validate_with_passphrase(
    p: &Profile,
    catalog: &EngineCatalog,
    roots: &ApprovedRoots,
    passphrase: Option<&secrecy::SecretString>,
) -> Vec<ValidationIssue> {
    checked(p, catalog, &roots.list(), KeyCheck::Passphrase(passphrase)).0
}

// ## m2b-t14-pipeline
/// How Validate treats an encrypted TLS client key.
#[derive(Clone, Copy)]
pub(crate) enum KeyCheck<'a> {
    /// Open it with this passphrase; without one an encrypted key is `TlsKeyEncrypted`.
    Passphrase(Option<&'a secrecy::SecretString>),
    /// The connect pipeline: its TLS step opens the key with the overlay's, the stored or a
    /// typed passphrase, so an encrypted key is not an issue at step 0.
    Deferred,
}

/// The issues of a profile and, when there are none, the sanitized options.
pub(crate) fn checked(
    p: &Profile,
    catalog: &EngineCatalog,
    roots: &[PathBuf],
    key: KeyCheck<'_>,
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
    // ## m2b-t12-profiles: the approved roots apply to untrusted origins, on canonical forms.
    let untrusted = !matches!(p.origin, Origin::UserTyped);
    let canonical_roots: Vec<PathBuf> = if untrusted {
        roots.iter().filter_map(|r| canonical(r)).collect()
    } else {
        Vec::new()
    };
    let outside = |path: &Path| untrusted && !inside_roots(path, &canonical_roots);
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
            } else {
                let issue = sqlite_path_issue(path, *create_if_missing);
                let typed_ok = matches!(issue, None | Some(IssueCode::FileMissing));
                if let Some(code) = issue {
                    push(Some(FieldId::File), code);
                }
                // ## m2b-t12-profiles: after the text rules, the roots (B26).
                let named = !path.as_os_str().is_empty() && path.as_os_str() != SQLITE_MEMORY;
                if typed_ok && named && outside(path) {
                    push(Some(FieldId::File), IssueCode::PathRefused);
                }
            }
            if file_based && *create_if_missing && p.read_only {
                push(Some(FieldId::File), IssueCode::CreateAndReadOnly);
            }
        }
    }
    match &p.transport {
        TransportSpec::Direct => {}
        TransportSpec::SshTunnel(spec) => {
            // ## m2b-t14-pipeline (named exception): the SSH step exists, so an SSH tunnel is
            // no longer `TransportUnavailable`.
            ssh_issues(spec, &mut push, &outside);
        }
        TransportSpec::Karagoz { .. } => push(None, IssueCode::TransportUnavailable),
    }
    // A TLS file on a share or a device is refused before anything opens, resolves or stats it
    // (N2): neither the TLS check below nor the roots check may touch it.
    let all_paths = tls_paths(p);
    // ## m2b-t12-profiles (fix round 1, M2): also once `~` is expanded, as the reader opens it.
    let tls_refused = all_paths.iter().any(|path| local_path(path).is_none());
    if tls_refused {
        push(None, IssueCode::PathRefused);
    }
    // ## m2b-t12-profiles: a TLS file of an untrusted profile outside the roots is never read.
    let tls_outside = !tls_refused && all_paths.iter().any(|path| outside(path));
    if tls_outside {
        push(None, IssueCode::PathRefused);
    }
    if let ProfileEndpoint::Server { host, .. } = &p.endpoint
        && !file_based
        && p.tls.mode != TlsMode::Disable
        && !tls_refused
        && !tls_outside
    {
        if p.tls.mode == TlsMode::VerifyCa && p.tls.ca_file.is_none() {
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
            // ## m2b-t14-pipeline: the key passphrase (T3 carry-in).
            let passphrase = match key {
                KeyCheck::Passphrase(passphrase) => passphrase,
                KeyCheck::Deferred => None,
            };
            match build_tls_params(&p.tls, host.trim(), port, AddressClass::Private, passphrase) {
                Err(TlsSetupError::KeyEncrypted(_)) if matches!(key, KeyCheck::Deferred) => {}
                Err(e) => {
                    let (field, code) = tls_issue(&e);
                    push(field, code);
                }
                Ok(_) => {}
            }
        }
    }
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
            // The profile's own files took the canonical roots check above (B26).
            paths: &[],
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

/// The profile's TLS files.
fn tls_paths(p: &Profile) -> Vec<&Path> {
    let mut paths = Vec::new();
    if let Some(file) = &p.tls.ca_file {
        paths.push(file.as_path());
    }
    if let Some(client) = &p.tls.client_cert {
        paths.push(client.cert.as_path());
        paths.push(client.key.as_path());
    }
    paths
}

// ## m2b-t12-profiles
/// The issues of an SSH tunnel (see [`validate_with`]). `outside` says whether a path lies
/// outside the approved roots (always `false` for a profile the user typed).
fn ssh_issues(
    spec: &SshSpec,
    push: &mut impl FnMut(Option<FieldId>, IssueCode),
    outside: &dyn Fn(&Path) -> bool,
) {
    if spec.hops.is_empty() {
        push(None, IssueCode::SshHostMissing);
    }
    if spec.hops.len() > usize::from(MAX_SSH_HOP) + 1 {
        push(None, IssueCode::SshTooManyHops);
    }
    for hop in &spec.hops {
        // The database host's rules (fix round 1, I1): a hop host with credentials
        // (`user:pw@host`), whitespace, a control character or URL syntax never reaches a
        // resolver or russh, and port 0 is never dialled.
        let host = hop.host.trim();
        if host.is_empty() {
            push(None, IssueCode::SshHostMissing);
        } else if host_invalid(host) {
            push(None, IssueCode::HostInvalid);
        }
        if hop.port == 0 {
            push(None, IssueCode::PortInvalid);
        }
        if hop.user.trim().is_empty() {
            push(None, IssueCode::SshUserMissing);
        }
        if let SshAuth::KeyFile { path } = &hop.auth
            && let Some(code) = ssh_key_issue(path, outside)
        {
            push(None, code);
        }
        // An imported `IdentityFile` that does not exist is skipped, as OpenSSH does; one on a
        // share or a device, or outside the roots, is refused.
        if hop
            .identity_files
            .iter()
            .any(|file| local_path(file).is_none() || outside(file))
        {
            push(None, IssueCode::PathRefused);
        }
    }
}

/// The issue of a hop's key file, if any: none set, on a share or a device (as typed or once
/// `~` is expanded) or `~` without a home folder, outside the roots, or not naming an existing
/// file (a relative path names none: the tunnel has no folder to read it from). The file
/// checked is `hacivat_net::paths::local_path(path)`, the one the tunnel must open (Task 14).
fn ssh_key_issue(path: &Path, outside: &dyn Fn(&Path) -> bool) -> Option<IssueCode> {
    if path.as_os_str().is_empty() {
        return Some(IssueCode::SshKeyFileMissing);
    }
    let Some(full) = local_path(path) else {
        return Some(IssueCode::PathRefused);
    };
    if outside(path) {
        return Some(IssueCode::PathRefused);
    }
    if !full.is_absolute() {
        return Some(IssueCode::SshKeyFileMissing);
    }
    fs_probe::note(&full);
    match std::fs::metadata(&full) {
        Ok(meta) if meta.is_file() => None,
        _ => Some(IssueCode::SshKeyFileMissing),
    }
}

/// Whether `path` lies under one of `roots` (canonical already), on canonical forms.
fn inside_roots(path: &Path, roots: &[PathBuf]) -> bool {
    canonical(path).is_some_and(|c| roots.iter().any(|r| c.starts_with(r)))
}

/// The canonical form of `path` (B26, Q22): `~` expanded, symlinks resolved, and a Windows
/// verbatim disk (`\\?\C:\`) read as the plain disk (`C:\`). A missing file canonicalises its
/// parent directory and appends its name. `None` (inside no root) for a share or a device,
/// typed or canonical, a relative path, a dangling symlink (what it names may lie anywhere,
/// and SQLite would create the file there), a missing folder, or an I/O error.
fn canonical(path: &Path) -> Option<PathBuf> {
    // Before any file system call (N2), on the typed and the `~`-expanded path (M2): the same
    // expansion the readers use (M1).
    let path = local_path(path)?;
    if !path.is_absolute() {
        return None;
    }
    fs_probe::note(&path);
    let resolved = match std::fs::canonicalize(&path) {
        Ok(resolved) => resolved,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if path.symlink_metadata().is_ok() {
                return None;
            }
            let name = path.file_name()?;
            std::fs::canonicalize(path.parent()?).ok()?.join(name)
        }
        Err(_) => return None,
    };
    let resolved = strip_verbatim_disk(resolved);
    (!network_path(&resolved)).then_some(resolved)
}

/// `\\?\C:\x` as `C:\x`; any other path as it is (a verbatim UNC or device path stays, and is
/// then refused as a network path). Text-based, so it behaves the same on every OS.
fn strip_verbatim_disk(path: PathBuf) -> PathBuf {
    let Some(rest) = path.to_str().and_then(|t| t.strip_prefix(r"\\?\")) else {
        return path;
    };
    match rest.as_bytes() {
        [drive, b':'] | [drive, b':', b'\\', ..] if drive.is_ascii_alphabetic() => {
            PathBuf::from(rest)
        }
        _ => path,
    }
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
        // ## m2b-t12-profiles: VerifyCa is a MySQL mode since M2b (B10); without a CA file it
        // trusts nothing, as for PostgreSQL.
        assert_eq!(
            validate(&my, EngineCatalog::embedded()),
            [ValidationIssue {
                field: None,
                code: IssueCode::TlsCaUnreadable
            }]
        );
    }

    // ## m2b-t12-profiles
    #[test]
    fn verbatim_disks_are_read_as_plain_disks_and_nothing_else_is() {
        for (given, expected) in [
            (r"\\?\C:\Users\me\ca.pem", r"C:\Users\me\ca.pem"),
            (r"\\?\d:\", r"d:\"),
            (r"\\?\C:", "C:"),
            (r"\\?\UNC\host\share\x", r"\\?\UNC\host\share\x"),
            (r"\\?\GLOBALROOT\Device\x", r"\\?\GLOBALROOT\Device\x"),
            (r"\\?\1:\x", r"\\?\1:\x"),
            (r"C:\x", r"C:\x"),
            ("/private/var/x", "/private/var/x"),
        ] {
            assert_eq!(
                strip_verbatim_disk(PathBuf::from(given)),
                PathBuf::from(expected),
                "{given}"
            );
        }
        // What stays verbatim is a share or a device: never inside a root.
        assert!(network_path(&strip_verbatim_disk(PathBuf::from(
            r"\\?\UNC\host\share\x"
        ))));
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
