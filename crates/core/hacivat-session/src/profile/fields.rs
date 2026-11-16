//! Field mapping between profiles and URLs: fields for a template, applying a parsed URL to a
//! draft, display and copy URLs (HV-DRV-03, -04).
//!
//! A profile's URL is its engine's catalog template for the endpoint (`default` for a server,
//! `unix_socket` for a socket, `file` or `memory` for SQLite), filled from the profile. Settings
//! that a pasted URL maps into the profile go back out as parameters, before the profile's own
//! extras: a TLS mode other than Auto (`sslmode=verify-full` for the PostgreSQL family,
//! `ssl-mode=VERIFY_IDENTITY` for the MySQL family), a non-default `application_name` and
//! `connect_timeout`, and SQLite's `mode=ro`. So a copied URL pasted back gives the same
//! profile. The password is never in a URL from here: Display masks it, Copy leaves it out.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use hacivat_driver::EngineCatalog;
use hacivat_driver::url::{
    FieldId, FieldMap, MappedParam, MappedTo, ParseOutcome, ParseWarning, RenderError,
    RenderTarget, Template, compile, render,
};
use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use hacivat_secrets::AuthProviderId;

use super::options::{FoldedOption, fold_typed_options};
use super::{Profile, ProfileDraft, ProfileEndpoint, TransportSpec};

/// The SQLite path of an in-memory database (the `memory` connection type).
const MEMORY_PATH: &str = ":memory:";

/// How applying a URL changed the engine (HV-DRV-04).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EngineChange {
    Switched {
        from: DbmsKind,
        to: DbmsKind,
    },
    /// The URL names another engine and the form keeps its own (not forced).
    Mismatch {
        url: DbmsKind,
        form: DbmsKind,
    },
}

/// What applying a parsed URL did to a draft, for the inline notice (HV-DRV-04).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApplyReport {
    pub filled: Vec<FieldId>,
    pub engine: Option<EngineChange>,
    pub password_moved: bool,
    pub mapped: Vec<MappedParam>,
    /// Parameters the parser kept as extras that have a typed setting (`sslrootcert`,
    /// `sslmode=allow`, ...): they went to it, not to `extra` (M2a final review I2).
    pub folded: Vec<FoldedOption>,
    pub extras_kept: Vec<String>,
    pub warnings: Vec<ParseWarning>,
}

/// A URL for the clipboard, never with the password (HV-DRV-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CopiedUrl {
    pub url: String,
    pub omitted_password: bool,
    pub omitted_tunnel: bool,
}

/// The engines whose URLs take libpq parameters (`sslmode`).
fn is_pg_family(kind: DbmsKind) -> bool {
    matches!(
        kind,
        DbmsKind::Postgres
            | DbmsKind::Redshift
            | DbmsKind::Cockroach
            | DbmsKind::Yugabyte
            | DbmsKind::Timescale
    )
}

/// The engines whose URLs take MySQL parameters (`ssl-mode`).
fn is_mysql_family(kind: DbmsKind) -> bool {
    matches!(kind, DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb)
}

/// The compiled catalog template of `kind`'s connection type `conn_type`.
fn template(kind: DbmsKind, conn_type: &str) -> Option<&'static Template> {
    static TEMPLATES: OnceLock<Vec<(DbmsKind, Template)>> = OnceLock::new();
    TEMPLATES
        .get_or_init(|| {
            EngineCatalog::embedded()
                .engines()
                .flat_map(|e| {
                    e.conn_types
                        .iter()
                        .filter(|ct| !ct.template.is_empty())
                        .filter_map(move |ct| Some((e.kind, compile(&ct.template, &ct.id).ok()?)))
                })
                .collect()
        })
        .iter()
        .find(|(k, t)| *k == kind && t.id == conn_type)
        .map(|(_, t)| t)
}

/// The profile's connection type: `default`, `unix_socket`, `file` or `memory`.
fn conn_type_of(p: &Profile) -> &'static str {
    match &p.endpoint {
        ProfileEndpoint::Server { .. } => "default",
        ProfileEndpoint::Socket { .. } => "unix_socket",
        ProfileEndpoint::File { path, .. } if path.as_os_str() == MEMORY_PATH => "memory",
        ProfileEndpoint::File { .. } => "file",
    }
}

/// The settings a URL carries as parameters (see the module docs), in a fixed order.
fn url_settings(p: &Profile) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut push = |k: &str, v: String| out.push((k.to_string(), v));
    if p.engine == DbmsKind::Sqlite {
        if p.read_only {
            push("mode", "ro".to_string());
        }
        return out;
    }
    let tls = p.tls.mode;
    if is_pg_family(p.engine) {
        let mode = match tls {
            TlsMode::Auto => None,
            TlsMode::Disable => Some("disable"),
            TlsMode::Require => Some("require"),
            TlsMode::VerifyCa => Some("verify-ca"),
            TlsMode::VerifyFull => Some("verify-full"),
        };
        if let Some(mode) = mode {
            push("sslmode", mode.to_string());
        }
    } else if is_mysql_family(p.engine) {
        let mode = match tls {
            TlsMode::Auto => None,
            TlsMode::Disable => Some("DISABLED"),
            TlsMode::Require => Some("REQUIRED"),
            TlsMode::VerifyCa => Some("VERIFY_CA"),
            TlsMode::VerifyFull => Some("VERIFY_IDENTITY"),
        };
        if let Some(mode) = mode {
            push("ssl-mode", mode.to_string());
        }
    }
    let defaults = Profile::default();
    if p.application_name != defaults.application_name {
        push("application_name", p.application_name.clone());
    }
    if p.connect_timeout_s != defaults.connect_timeout_s {
        push("connect_timeout", p.connect_timeout_s.to_string());
    }
    out
}

/// The profile's connection type, field values and extra parameters: the settings a URL
/// carries (see the module docs), then the profile's own extras. A field the type's template
/// cannot carry (a MySQL socket's port) is left out.
pub fn to_fields(p: &Profile) -> (String, FieldMap, Vec<(String, String)>) {
    let conn_type = conn_type_of(p);
    let mut fields = FieldMap::default();
    let mut set = |f: FieldId, v: String| {
        if !v.is_empty() {
            fields.values.insert(f, v);
        }
    };
    let port = match &p.endpoint {
        ProfileEndpoint::Server { host, port } => {
            set(FieldId::Host, host.clone());
            *port
        }
        ProfileEndpoint::Socket { path, port } => {
            set(FieldId::Socket, path.to_string_lossy().into_owned());
            *port
        }
        ProfileEndpoint::File { path, .. } => {
            if conn_type == "file" {
                set(FieldId::File, path.to_string_lossy().into_owned());
            }
            None
        }
    };
    if let Some(port) = port {
        set(FieldId::Port, port.to_string());
    }
    if p.engine != DbmsKind::Sqlite {
        set(FieldId::Database, p.database.clone().unwrap_or_default());
        set(FieldId::User, p.user.clone().unwrap_or_default());
    }
    if let Some(t) = template(p.engine, conn_type) {
        fields.values.retain(|f, _| t.field_set.contains(f));
    }
    let mut extra = url_settings(p);
    extra.retain(|(k, _)| !p.extra.iter().any(|(e, _)| e.eq_ignore_ascii_case(k)));
    extra.extend(p.extra.iter().cloned());
    (conn_type.to_string(), fields, extra)
}

/// Renders the profile's URL for `target`; the template's refusal, or `CannotRedirect`
/// (`no_template`) for an engine without a template for this endpoint.
fn render_profile(
    p: &Profile,
    target: RenderTarget,
) -> Result<(String, &'static Template), RenderError> {
    let (conn_type, fields, extra) = to_fields(p);
    let t = template(p.engine, &conn_type)
        .ok_or_else(|| RenderError::CannotRedirect("no_template".to_string()))?;
    Ok((render(t, &fields, &extra, target)?, t))
}

/// The URL row's text, a mask where the password goes; `None` while the profile cannot be
/// rendered (no host yet).
pub fn display_url(p: &Profile, has_password: bool) -> Option<String> {
    render_profile(p, RenderTarget::Display { has_password })
        .ok()
        .map(|(url, _)| url)
}

/// The URL "Copy as URL" puts on the clipboard: never the password (`omitted_password` when
/// the profile signs in with one), never a tunnel (`omitted_tunnel` when it uses one).
pub fn copy_url(p: &Profile) -> Result<CopiedUrl, RenderError> {
    let (url, t) = render_profile(p, RenderTarget::Copy)?;
    Ok(CopiedUrl {
        url,
        omitted_password: t.field_set.contains(&FieldId::Password)
            && p.auth == AuthProviderId::Password,
        omitted_tunnel: p.transport != TransportSpec::Direct,
    })
}

/// Switches `p` to `to`: a SQLite profile has a file and one connection; leaving SQLite gives
/// back a server endpoint and the default budget.
fn switch_engine(p: &mut Profile, to: DbmsKind) {
    let from = p.engine;
    p.engine = to;
    if to == DbmsKind::Sqlite {
        p.max_connections = 1;
    } else if from == DbmsKind::Sqlite {
        p.max_connections = Profile::default().max_connections;
        if matches!(p.endpoint, ProfileEndpoint::File { .. }) {
            p.endpoint = ProfileEndpoint::Server {
                host: String::new(),
                port: None,
            };
        }
    }
    if !is_pg_family(to) && p.auth == AuthProviderId::PgPass {
        p.auth = AuthProviderId::Password;
    }
}

/// Applies a parsed URL to `draft` (HV-DRV-04):
/// - **Engine.** A URL for another engine switches an untouched draft (`Switched`); a draft
///   with hand-edited fields keeps its engine and nothing is applied (`Mismatch`), unless
///   `force_engine` (the user accepted the switch).
/// - **Fields** the URL carries fill the draft, except the ones the user edited by hand
///   (`draft.touched`). A URL never clears a field it does not carry.
/// - **The password** moves to `draft.password` (and the auth becomes Password); an
///   `sslpassword` moves to `draft.tls_key_passphrase`.
/// - **Mapped settings** (TLS mode, application name, connect timeout, SQLite read-only) are
///   applied; unknown parameters replace the draft's extras, and those with a typed setting
///   (`sslrootcert`, `sslcert`, ...) are folded into it (`fold_typed_options`).
pub fn apply_parse(
    draft: &mut ProfileDraft,
    outcome: ParseOutcome,
    force_engine: bool,
) -> ApplyReport {
    let ParseOutcome {
        kind,
        conn_type,
        fields,
        extra,
        secrets,
        tls_key_passphrase,
        mapped,
        warnings,
    } = outcome;
    let mut report = ApplyReport {
        filled: Vec::new(),
        engine: None,
        password_moved: false,
        mapped: Vec::new(),
        folded: Vec::new(),
        extras_kept: Vec::new(),
        warnings,
    };
    let form = draft.profile.engine;
    if kind != form {
        if !draft.touched.is_empty() && !force_engine {
            report.engine = Some(EngineChange::Mismatch { url: kind, form });
            return report;
        }
        switch_engine(&mut draft.profile, kind);
        report.engine = Some(EngineChange::Switched {
            from: form,
            to: kind,
        });
    }

    let touched = draft.touched.clone();
    let free = |f: FieldId| !touched.contains(&f);
    let given = |f: FieldId| fields.values.get(&f).filter(|v| !v.is_empty()).cloned();
    let given_port = given(FieldId::Port).and_then(|p| p.parse::<u16>().ok());
    let mut filled = BTreeSet::new();
    let p = &mut draft.profile;

    if conn_type == "memory" || given(FieldId::File).is_some() {
        if free(FieldId::File) {
            let path = given(FieldId::File).unwrap_or_else(|| MEMORY_PATH.to_string());
            let create = matches!(
                p.endpoint,
                ProfileEndpoint::File {
                    create_if_missing: true,
                    ..
                }
            );
            p.endpoint = ProfileEndpoint::File {
                path: PathBuf::from(path),
                create_if_missing: create,
            };
            filled.insert(FieldId::File);
        }
    } else if let Some(socket) = given(FieldId::Socket) {
        if free(FieldId::Socket) {
            let old_port = match p.endpoint {
                ProfileEndpoint::Socket { port, .. } => port,
                _ => None,
            };
            let port = match given_port {
                Some(port) if free(FieldId::Port) => {
                    filled.insert(FieldId::Port);
                    Some(port)
                }
                _ => old_port,
            };
            p.endpoint = ProfileEndpoint::Socket {
                path: PathBuf::from(socket),
                port,
            };
            filled.insert(FieldId::Socket);
        }
    } else {
        let (mut host, mut port) = match &p.endpoint {
            ProfileEndpoint::Server { host, port } => (host.clone(), *port),
            _ => (String::new(), None),
        };
        if let Some(h) = given(FieldId::Host).filter(|_| free(FieldId::Host)) {
            host = h;
            filled.insert(FieldId::Host);
        }
        if let Some(n) = given_port.filter(|_| free(FieldId::Port)) {
            port = Some(n);
            filled.insert(FieldId::Port);
        }
        p.endpoint = ProfileEndpoint::Server { host, port };
    }
    if let Some(db) = given(FieldId::Database).filter(|_| free(FieldId::Database)) {
        p.database = Some(db);
        filled.insert(FieldId::Database);
    }
    if let Some(user) = given(FieldId::User).filter(|_| free(FieldId::User)) {
        p.user = Some(user);
        filled.insert(FieldId::User);
    }

    for m in &mapped {
        match &m.to {
            MappedTo::TlsMode(mode) => p.tls.mode = *mode,
            MappedTo::ApplicationName => p.application_name = m.value.clone(),
            MappedTo::ConnectTimeout(s) => p.connect_timeout_s = *s,
            MappedTo::ReadOnly(ro) => p.read_only = *ro,
            // Kept in the extras as well; Validate checks its `-c` settings.
            MappedTo::PgOptions => {}
        }
    }
    report.mapped = mapped;
    if !extra.is_empty() {
        p.extra = extra;
        // An option with a typed setting goes to it, as the mapped ones did.
        report.folded = fold_typed_options(p);
        report.extras_kept = p.extra.iter().map(|(k, _)| k.clone()).collect();
    }

    let password = secrets
        .into_iter()
        .find_map(|(f, s)| (f == FieldId::Password).then_some(s));
    if let Some(password) = password.filter(|_| free(FieldId::Password)) {
        p.auth = AuthProviderId::Password;
        draft.password = Some(password);
        report.password_moved = true;
        filled.insert(FieldId::Password);
    }
    if let Some(passphrase) = tls_key_passphrase {
        draft.tls_key_passphrase = Some(passphrase);
    }
    report.filled = filled.into_iter().collect();
    report
}
