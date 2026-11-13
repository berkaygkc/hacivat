//! The dialog form: engine, server, authentication, TLS and advanced sections (NOTES surface 1,
//! Task 17). The layout follows the mock's `connDialogHTML` (the SSH disclosure is Task 20's):
//! form rows (140 px labels, 8 px apart), small-caps section headings, hint rows 4 px under their
//! row, and the folded sections with live summaries.
//!
//! M2b (Task 19): the TLS section's "Pinned certificate" row (a short fingerprint and Remove
//! pin, or None) and the Disable-with-a-pin notice; the client certificate's masked Key
//! passphrase with Remember; VerifyCa for every engine (B10); the Remember Select's "Forever"
//! label from `vault_dialog::forever_label`.

use std::path::PathBuf;
use std::rc::Rc;

use hacivat_driver::EngineCatalog;
use hacivat_driver::url::FieldId;
use hacivat_l10n::{Count, MsgRef, NoArgs, Text, ids};
use hacivat_net::ssh::SshAuth;
use hacivat_net::{ClientCertPaths, TlsMode};
use hacivat_proto::{DbmsKind, EnvTag, Origin};
use hacivat_secrets::{AuthProviderId, Retention};
use hacivat_session::pipeline::{IssueCode, ValidationIssue};
use hacivat_session::profile::{Profile, ProfileEndpoint, TransportSpec, suggest_name};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::{EnvKind, env_swatch_on_raised};
use hacivat_ui::checkbox::Checkbox;
use hacivat_ui::dialog::{DialogSize, form_dialog};
use hacivat_ui::disclosure::Disclosure;
use hacivat_ui::form::{FORM_LABEL_PX, form_row, hint_row};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::TextInput;
use hacivat_ui::l10n::ui_text;
use hacivat_ui::notice::{InlineNotice, NoticeIcon};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented, natural_width};
use hacivat_ui::select::SelectOption;
use hacivat_ui::text::{Label, TextFn, TextSource};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Div, Entity, Focusable as _, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Role, SharedString, StatefulInteractiveElement as _,
    Styled, WeakEntity, Window, div, px, relative,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use secrecy::ExposeSecret;

use super::{
    ConnectionDialog, ENGINES, KEEPALIVES, SaveProblem, TLS_MODES, is_mysql_family, on_dialog_key,
};
use crate::connections::test_strip::TestStripState;
use crate::connections::{cert_dialog, vault_dialog};
use crate::l10n_global::{text_fn, tr};
use crate::shell::keymap_state::shortcut_label;

/// A text field of the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Url,
    Name,
    Host,
    Port,
    Database,
    User,
    Password,
    File,
    Ca,
    Cert,
    Key,
    KeyPass,
    Timeout,
    AppName,
}

impl Field {
    pub const ALL: [Field; 14] = [
        Field::Url,
        Field::Name,
        Field::Host,
        Field::Port,
        Field::Database,
        Field::User,
        Field::Password,
        Field::File,
        Field::Ca,
        Field::Cert,
        Field::Key,
        Field::KeyPass,
        Field::Timeout,
        Field::AppName,
    ];

    /// The field's stable name (tests, debug selectors `conn-field-<name>`).
    pub fn name(self) -> &'static str {
        match self {
            Field::Url => "url",
            Field::Name => "name",
            Field::Host => "host",
            Field::Port => "port",
            Field::Database => "database",
            Field::User => "user",
            Field::Password => "password",
            Field::File => "file",
            Field::Ca => "ca",
            Field::Cert => "cert",
            Field::Key => "key",
            Field::KeyPass => "key-pass",
            Field::Timeout => "timeout",
            Field::AppName => "app-name",
        }
    }

    pub fn from_name(name: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.name() == name)
    }

    /// A masked field never parses a paste (R21, P16).
    fn masked(self) -> bool {
        matches!(self, Field::Password | Field::KeyPass)
    }

    /// Machine text is drawn in mono (the mock's `.in.mono`): every field but the secrets.
    pub(crate) fn mono(self) -> bool {
        !self.masked()
    }

    /// The field a pasted URL fills.
    pub(crate) fn of(id: FieldId) -> Option<Field> {
        Some(match id {
            FieldId::Host | FieldId::Socket => Field::Host,
            FieldId::Port => Field::Port,
            FieldId::Database => Field::Database,
            FieldId::User => Field::User,
            FieldId::Password => Field::Password,
            FieldId::File => Field::File,
        })
    }
}

/// The pgpass line that matches the endpoint (looked up on the core runtime).
pub(crate) struct PgPassState {
    pub(crate) key: PgPassKey,
    /// The 1-based line, once looked up and found.
    pub(crate) line: Option<u32>,
}

/// What a pgpass lookup matches on: host, port, database, user.
pub(crate) type PgPassKey = (String, u16, String, Option<String>);

pub(crate) fn pgpass_key(p: &Profile) -> PgPassKey {
    (
        host_text(p),
        endpoint_port(&p.endpoint).unwrap_or(0),
        p.database.clone().unwrap_or_default(),
        p.user.clone(),
    )
}

/// The host (or socket path) the profile names.
pub(crate) fn host_text(p: &Profile) -> String {
    match &p.endpoint {
        ProfileEndpoint::Server { host, .. } => host.clone(),
        ProfileEndpoint::Socket { path, .. } | ProfileEndpoint::File { path, .. } => {
            path.to_string_lossy().into_owned()
        }
    }
}

pub(crate) fn endpoint_port(e: &ProfileEndpoint) -> Option<u16> {
    match e {
        ProfileEndpoint::Server { port, .. } | ProfileEndpoint::Socket { port, .. } => *port,
        ProfileEndpoint::File { .. } => None,
    }
}

/// A typed port: empty is the default; text that is no port is 0, which Validate reports.
pub(crate) fn parse_port(text: &str) -> Option<u16> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.parse::<u16>().unwrap_or(0))
}

/// The client certificate from its two fields (none when both are empty).
pub(crate) fn client_cert(cert: &str, key: &str) -> Option<ClientCertPaths> {
    if cert.trim().is_empty() && key.trim().is_empty() {
        return None;
    }
    Some(ClientCertPaths {
        cert: PathBuf::from(cert.trim()),
        key: PathBuf::from(key.trim()),
    })
}

/// The text field `field` shows for the dialog's draft.
pub(crate) fn draft_value(d: &ConnectionDialog, field: Field) -> String {
    let p = &d.draft.profile;
    let path = |p: &Option<PathBuf>| {
        p.as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    match field {
        Field::Url => String::new(),
        Field::Name => p.name.clone(),
        Field::Host => match &p.endpoint {
            ProfileEndpoint::File { .. } => String::new(),
            _ => host_text(p),
        },
        Field::Port => endpoint_port(&p.endpoint)
            .map(|n| n.to_string())
            .unwrap_or_default(),
        Field::Database => p.database.clone().unwrap_or_default(),
        Field::User => p.user.clone().unwrap_or_default(),
        Field::Password => d
            .draft
            .password
            .as_ref()
            .map(|s| s.expose_secret().to_string())
            .unwrap_or_default(),
        Field::KeyPass => d
            .draft
            .tls_key_passphrase
            .as_ref()
            .map(|s| s.expose_secret().to_string())
            .unwrap_or_default(),
        Field::File => match &p.endpoint {
            ProfileEndpoint::File { path, .. } => path.to_string_lossy().into_owned(),
            _ => String::new(),
        },
        Field::Ca => path(&p.tls.ca_file),
        Field::Cert => path(&p.tls.client_cert.as_ref().map(|c| c.cert.clone())),
        Field::Key => path(&p.tls.client_cert.as_ref().map(|c| c.key.clone())),
        Field::Timeout => p.connect_timeout_s.to_string(),
        Field::AppName => p.application_name.clone(),
    }
}

/// The text field `field`: a 26 px input (its fill comes from the field box), a tab stop, with
/// the paste filter on every field that is not masked (P16).
pub(crate) fn new_input(
    field: Field,
    dialog: WeakEntity<ConnectionDialog>,
    cx: &mut Context<ConnectionDialog>,
) -> Entity<TextInput> {
    let (placeholder, source) = placeholder_of(field, dialog.clone());
    let input = cx.new(|cx| {
        let mut input = TextInput::new(cx, placeholder);
        input.set_placeholder_source(source);
        input.set_masked(field.masked());
        if !field.masked() {
            input.set_paste_filter(Some(super::paste::paste_filter(
                field == Field::Url,
                dialog,
            )));
        }
        input
    });
    let _ = input.read(cx).focus_handle(cx).tab_stop(true);
    input
}

/// The placeholder of `field` and where its text comes from.
fn placeholder_of(field: Field, dialog: WeakEntity<ConnectionDialog>) -> (TextFn, TextSource) {
    let data = |f: fn(&ConnectionDialog) -> String| -> TextFn {
        let dialog = dialog.clone();
        Rc::new(move |cx: &App| {
            dialog
                .upgrade()
                .map(|d| SharedString::from(f(d.read(cx))))
                .unwrap_or_default()
        })
    };
    match field {
        Field::Url => (text_fn(ids::hv::CONN_DLG_URL_PH), TextSource::Fluent),
        Field::Ca => (text_fn(ids::hv::CONN_TLS_CA_PH), TextSource::Fluent),
        Field::Password => (
            Rc::new(move |cx: &App| {
                let saved = dialog
                    .upgrade()
                    .is_some_and(|d| d.read(cx).has_stored_password);
                if saved {
                    tr(cx, ids::hv::CONN_DLG_PW_SAVED, NoArgs)
                } else {
                    SharedString::default()
                }
            }),
            // A message only while a password is stored (the dialog switches it).
            TextSource::Data,
        ),
        Field::Name => (data(|d| suggest_name(&d.draft.profile)), TextSource::Data),
        Field::Host => (data(|_| example(EXAMPLE_HOST)), TextSource::Data),
        Field::Port => (
            data(|d| {
                EngineCatalog::embedded()
                    .get(d.draft.profile.engine)
                    .and_then(|e| e.default_port)
                    .map(|p| p.to_string())
                    .unwrap_or_default()
            }),
            TextSource::Data,
        ),
        Field::Database => (
            data(|d| match d.draft.profile.engine {
                DbmsKind::Postgres => example(EXAMPLE_PG_DEFAULT),
                _ => String::new(),
            }),
            TextSource::Data,
        ),
        Field::User => (
            data(|d| match d.draft.profile.engine {
                DbmsKind::Postgres => example(EXAMPLE_PG_DEFAULT),
                _ => example(EXAMPLE_MYSQL_USER),
            }),
            TextSource::Data,
        ),
        Field::File => (data(|_| example(EXAMPLE_FILE)), TextSource::Data),
        Field::Timeout => (
            data(|_| Profile::default().connect_timeout_s.to_string()),
            TextSource::Data,
        ),
        // Fields without a placeholder show none (an empty data string, never a message).
        Field::Cert | Field::Key | Field::KeyPass | Field::AppName => {
            (data(|_| String::new()), TextSource::Data)
        }
    }
}

/// Example values in placeholders (the mock's): data, never translated.
const EXAMPLE_HOST: &str = "db-stg.acme.io";
const EXAMPLE_PG_DEFAULT: &str = "postgres";
const EXAMPLE_MYSQL_USER: &str = "root";
const EXAMPLE_FILE: &str = "~/code/shop/app.db";

fn example(s: &str) -> String {
    s.to_string()
}

// ---- options --------------------------------------------------------------------------------

/// The Remember choices; "Forever" is disabled with its reason without a keyring (P36).
pub(crate) fn retention_options(keyring: bool) -> Vec<SelectOption> {
    super::RETENTIONS
        .iter()
        .map(|r| {
            let (label, hint) = match r {
                Retention::Never => (ids::hv::CONN_RET_NEVER, ids::hv::CONN_RET_NEVER_H),
                Retention::UntilRestart => (ids::hv::CONN_RET_RESTART, ids::hv::CONN_RET_RESTART_H),
                Retention::ForSession => (ids::hv::CONN_RET_SESSION, ids::hv::CONN_RET_SESSION_H),
                // ## m2b-t21-prompts: "Forever (encrypted file)" while the vault holds the
                // Forever secrets (Q23); re-read on every render, so it follows the probe.
                Retention::Forever if keyring => {
                    let (label, hint) = crate::connections::vault_dialog::forever_texts();
                    return SelectOption {
                        label,
                        hint: Some(hint),
                        disabled: false,
                    };
                }
                Retention::Forever => (ids::hv::CONN_RET_FOREVER, ids::hv::CONN_RET_FOREVER_OFF_H),
            };
            // ## m2b-t19-tls: "Forever" names where the secret goes: the keyring, or the vault
            // when it is chosen (Task 21's `forever_label`).
            let label: TextFn = if *r == Retention::Forever {
                Rc::new(|cx: &App| tr(cx, vault_dialog::forever_label(cx), NoArgs))
            } else {
                text_fn(label)
            };
            SelectOption {
                label,
                hint: Some(text_fn(hint)),
                disabled: *r == Retention::Forever && !keyring,
            }
        })
        .collect()
}

pub(crate) fn tls_label(mode: TlsMode) -> MsgRef<NoArgs> {
    match mode {
        TlsMode::Auto => ids::hv::CONN_TLS_AUTO,
        TlsMode::Require => ids::hv::CONN_TLS_REQUIRE,
        TlsMode::VerifyCa => ids::hv::CONN_TLS_VERIFY_CA,
        TlsMode::VerifyFull => ids::hv::CONN_TLS_VERIFY_FULL,
        TlsMode::Disable => ids::hv::CONN_TLS_DISABLE,
    }
}

fn tls_hint(engine: DbmsKind, mode: TlsMode) -> MsgRef<NoArgs> {
    match mode {
        TlsMode::Auto if is_mysql_family(engine) => ids::hv::CONN_TLS_AUTO_LOOPBACK_H,
        TlsMode::Auto => ids::hv::CONN_TLS_AUTO_H,
        TlsMode::Require => ids::hv::CONN_TLS_REQUIRE_H,
        TlsMode::VerifyCa => ids::hv::CONN_TLS_VERIFY_CA_H,
        TlsMode::VerifyFull => ids::hv::CONN_TLS_VERIFY_FULL_H,
        TlsMode::Disable => ids::hv::CONN_TLS_DISABLE_H,
    }
}

/// The TLS modes with their hints. Every mode works for every engine: MySQL and MariaDB verify
/// the chain without the host name too since M2b (B10; M2a's P11 disabled VerifyCa for them).
pub(crate) fn tls_options(engine: DbmsKind) -> Vec<SelectOption> {
    TLS_MODES
        .iter()
        .map(|&mode| SelectOption {
            label: text_fn(tls_label(mode)),
            hint: Some(text_fn(tls_hint(engine, mode))),
            disabled: false,
        })
        .collect()
}

fn unsupported_mode(cx: &App, mode: TlsMode) -> SharedString {
    tr(
        cx,
        ids::hv::CONN_ISSUE_TLS_MODE_UNSUPPORTED,
        ids::hv::HvConnIssueTlsModeUnsupported {
            mode: Text(tr(cx, tls_label(mode), NoArgs).to_string()),
        },
    )
}

// ## m2a-t21-shots
/// The Environment Select's options: "Choose…" (disabled; shown while no tag is set), then
/// the tags in the segmented control's order.
pub(crate) fn env_options() -> Vec<SelectOption> {
    std::iter::once(SelectOption {
        label: text_fn(ids::hv::CONN_IMPORT_CHOOSE),
        hint: None,
        disabled: true,
    })
    .chain(EnvTag::ALL.iter().map(|tag| SelectOption {
        label: text_fn(env_label(*tag)),
        hint: None,
        disabled: false,
    }))
    .collect()
}

/// The Environment Select's index for `env` (0: "Choose…").
pub(crate) fn env_index(env: Option<EnvTag>) -> usize {
    env.and_then(|e| EnvTag::ALL.iter().position(|t| *t == e))
        .map_or(0, |i| i + 1)
}

/// The width of a form row's control column in the 640 px form dialog: the card less its
/// border, the body's side padding, the label column and the gap after it.
fn control_room(theme: &Theme) -> f32 {
    let t = theme.tokens;
    DialogSize::Form.width_px() - 2.0 - 2.0 * t.spacing.s_3xl - FORM_LABEL_PX - t.spacing.xl
}

pub(crate) fn keepalive_options() -> Vec<SelectOption> {
    KEEPALIVES
        .iter()
        .map(|k| SelectOption {
            label: match *k {
                None => text_fn(ids::hv::CONN_ADV_OFF),
                Some(seconds) => Rc::new(move |cx: &App| {
                    tr(
                        cx,
                        ids::hv::CONN_ADV_EVERY,
                        ids::hv::HvConnAdvEvery {
                            seconds: Count(u64::from(seconds)),
                        },
                    )
                }),
            },
            hint: None,
            disabled: false,
        })
        .collect()
}

// ---- issues ---------------------------------------------------------------------------------

/// The field an issue is shown under, if any.
pub(crate) fn field_of_issue(issue: &ValidationIssue) -> Option<Field> {
    match (&issue.field, &issue.code) {
        (_, IssueCode::NameMissing) => Some(Field::Name),
        (Some(FieldId::Host | FieldId::Socket), _) => Some(Field::Host),
        (Some(FieldId::Port), _) | (_, IssueCode::PortInvalid) => Some(Field::Port),
        (Some(FieldId::File), _) => Some(Field::File),
        (Some(FieldId::Database), _) => Some(Field::Database),
        (Some(FieldId::User), _) => Some(Field::User),
        (_, IssueCode::HostMissing | IssueCode::HostInvalid) => Some(Field::Host),
        (_, IssueCode::FileMissing | IssueCode::CreateAndReadOnly) => Some(Field::File),
        (
            _,
            IssueCode::TlsCaUnreadable
            | IssueCode::TlsModeUnsupported { .. }
            | IssueCode::PathRefused,
        ) => Some(Field::Ca),
        (_, IssueCode::TlsCertUnreadable) => Some(Field::Cert),
        // ## m2b-t19-tls: an encrypted key wants its passphrase; a legacy one, another file.
        (_, IssueCode::TlsKeyEncrypted) => Some(Field::KeyPass),
        (_, IssueCode::TlsKeyLegacyEncrypted) => Some(Field::Key),
        (_, IssueCode::OptionRejected { .. } | IssueCode::OptionSecret { .. }) => {
            Some(Field::AppName)
        }
        _ => None,
    }
}

/// Whether Validate refuses `path` of `p` (DRV-05, B26): a share or a device for every origin,
/// and, for a profile from an untrusted origin, a path outside every approved root. No file
/// system call (the dialog draws with it); Validate itself compares canonical forms.
fn path_refused(p: &Profile, path: &std::path::Path, roots: &[PathBuf]) -> bool {
    if path.as_os_str().is_empty() {
        return false;
    }
    let untrusted = !matches!(p.origin, Origin::UserTyped);
    hacivat_net::paths::local_path(path).is_none()
        || (untrusted && !roots.iter().any(|root| path.starts_with(root)))
}

/// The TLS file a path-refused issue is about: the first of the CA, the certificate and the
/// key that Validate refuses, if any.
fn refused_tls_field(p: &Profile, roots: &[PathBuf]) -> Option<Field> {
    let cert = p.tls.client_cert.as_ref();
    [
        (Field::Ca, p.tls.ca_file.clone()),
        (Field::Cert, cert.map(|c| c.cert.clone())),
        (Field::Key, cert.map(|c| c.key.clone())),
    ]
    .into_iter()
    .find_map(|(field, path)| path_refused(p, path.as_deref()?, roots).then_some(field))
}

// ## m2b-t19-tls (carry-in from Task 20)
/// Whether a path-refused issue is (also) about an SSH key: a hop's key file or an imported
/// `IdentityFile` that Validate refuses. The SSH section shows it, never a TLS field.
pub(crate) fn ssh_path_refused(p: &Profile, cx: &App) -> bool {
    let TransportSpec::SshTunnel(spec) = &p.transport else {
        return false;
    };
    let roots = approved_roots(cx);
    spec.hops.iter().any(|hop| {
        let key = match &hop.auth {
            SshAuth::KeyFile { path } => Some(path),
            _ => None,
        };
        key.into_iter()
            .chain(hop.identity_files.iter())
            .any(|path| path_refused(p, path, &roots))
    })
}

/// The approved roots the dialog's checks compare with (none without a `DialogEnv`).
fn approved_roots(cx: &App) -> Vec<PathBuf> {
    cx.try_global::<super::DialogEnv>()
        .map(|e| e.approved_roots.list())
        .unwrap_or_default()
}

/// The SQLite file option "Open the file read-only" (`mode=ro`, ruling A): it can only tighten.
pub(crate) fn sqlite_mode_ro(p: &Profile) -> bool {
    p.extra
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case(SQLITE_MODE) && v.eq_ignore_ascii_case(SQLITE_RO))
}

/// Sets or clears `mode=ro` in the profile's options.
pub(crate) fn set_sqlite_mode_ro(p: &mut Profile, on: bool) {
    if on {
        p.extra
            .retain(|(k, _)| !k.eq_ignore_ascii_case(SQLITE_MODE));
        p.extra
            .push((SQLITE_MODE.to_string(), SQLITE_RO.to_string()));
    } else {
        p.extra.retain(|(k, v)| {
            !(k.eq_ignore_ascii_case(SQLITE_MODE) && v.eq_ignore_ascii_case(SQLITE_RO))
        });
    }
}

/// The SQLite option key and its read-only value (machine text).
// ## m2a-t21-shots: where the password field's "from the pasted URL" tail may start (a share of
// the field), so the dots before it stay visible.
const TAIL_LEFT: f32 = 0.35;
/// The Environment Select's width (the TLS mode Select's).
const ENV_SELECT_PX: f32 = 200.0;

const SQLITE_MODE: &str = "mode";
const SQLITE_RO: &str = "ro";

/// A file that is to be created cannot be opened with `mode=ro` (ruling A; Validate checks the
/// Hacivat guard, the dialog checks the file option).
pub(crate) fn sqlite_mode_issue(p: &Profile) -> Option<ValidationIssue> {
    let create = matches!(
        p.endpoint,
        ProfileEndpoint::File {
            create_if_missing: true,
            ..
        }
    );
    (p.engine == DbmsKind::Sqlite && create && sqlite_mode_ro(p)).then_some(ValidationIssue {
        field: Some(FieldId::File),
        code: IssueCode::CreateAndReadOnly,
    })
}

impl ConnectionDialog {
    /// Where `issue` shows: under its field when that field is on screen, else (`None`) in the
    /// notice at the top of the form.
    pub(crate) fn issue_field(&self, issue: &ValidationIssue, cx: &App) -> Option<Field> {
        let p = &self.draft.profile;
        let field = match (&issue.field, &issue.code) {
            // ## m2b-t19-tls: a refused TLS file shows under its field; a refused SSH key in
            // the SSH section (Task 20 carry-in); neither told apart, under the CA.
            (None, IssueCode::PathRefused) if p.engine != DbmsKind::Sqlite => {
                match refused_tls_field(p, &approved_roots(cx)) {
                    Some(field) => Some(field),
                    None if ssh_path_refused(p, cx) => None,
                    None => Some(Field::Ca),
                }
            }
            _ => field_of_issue(issue),
        }?;
        self.field_visible(field).then_some(field)
    }

    /// Whether `field` is part of the form as it is drawn now (an issue of a hidden field goes
    /// to the top notice).
    pub(crate) fn field_visible(&self, field: Field) -> bool {
        let p = &self.draft.profile;
        let sqlite = p.engine == DbmsKind::Sqlite;
        match field {
            Field::Url | Field::Name => true,
            Field::File => sqlite,
            Field::Host
            | Field::Port
            | Field::Database
            | Field::User
            | Field::Ca
            | Field::Timeout
            | Field::AppName => !sqlite,
            Field::Password => !sqlite && p.auth == AuthProviderId::Password,
            // ## m2b-t19-tls: the key passphrase with the client certificate (B25).
            Field::Cert | Field::Key | Field::KeyPass => {
                !sqlite && p.auth == AuthProviderId::ClientCert
            }
        }
    }
}

/// The sentence for `issue` (one message per `IssueCode`).
fn issue_text(cx: &App, issue: &ValidationIssue) -> SharedString {
    let msg = match &issue.code {
        IssueCode::NameMissing => ids::hv::CONN_ISSUE_NAME_MISSING,
        IssueCode::HostMissing => ids::hv::CONN_ISSUE_HOST_MISSING,
        IssueCode::HostInvalid => ids::hv::CONN_ISSUE_HOST_INVALID,
        IssueCode::PortInvalid => ids::hv::CONN_ISSUE_PORT_INVALID,
        IssueCode::FileMissing => ids::hv::CONN_ISSUE_FILE_MISSING,
        IssueCode::PathRefused => ids::hv::CONN_ISSUE_PATH_REFUSED,
        IssueCode::CreateAndReadOnly => ids::hv::CONN_ISSUE_CREATE_AND_READ_ONLY,
        IssueCode::TransportUnavailable => ids::hv::CONN_ISSUE_TRANSPORT_UNAVAILABLE,
        IssueCode::TlsCaUnreadable => ids::hv::CONN_ISSUE_TLS_CA_UNREADABLE,
        IssueCode::TlsCertUnreadable => ids::hv::CONN_ISSUE_TLS_CERT_UNREADABLE,
        // ## m2b-t19-tls: the field below can open an encrypted key (B25).
        IssueCode::TlsKeyEncrypted => ids::hv::CONN_ISSUE_TLS_KEY_PASSPHRASE,
        IssueCode::TlsModeUnsupported { mode } => return unsupported_mode(cx, *mode),
        IssueCode::OptionRejected { key, .. } => {
            return tr(
                cx,
                ids::hv::CONN_ISSUE_OPTION_REJECTED,
                ids::hv::HvConnIssueOptionRejected {
                    key: Text(key.clone()),
                },
            );
        }
        // ## m2a-finalfix
        IssueCode::OptionSecret { key } => {
            return tr(
                cx,
                ids::hv::CONN_ISSUE_OPTION_SECRET,
                ids::hv::HvConnIssueOptionSecret {
                    key: Text(key.clone()),
                },
            );
        }
        // ## m2b-t01-interfaces: placeholders until Tasks 19 and 20 (Q3).
        IssueCode::SshHostMissing => ids::hv::CONN_ISSUE_SSH_HOST_MISSING,
        IssueCode::SshUserMissing => ids::hv::CONN_ISSUE_SSH_USER_MISSING,
        IssueCode::SshTooManyHops => ids::hv::CONN_ISSUE_SSH_TOO_MANY_HOPS,
        IssueCode::SshKeyFileMissing => ids::hv::CONN_ISSUE_SSH_KEY_FILE_MISSING,
        IssueCode::TlsKeyLegacyEncrypted => ids::hv::CONN_ISSUE_TLS_KEY_LEGACY_ENCRYPTED,
        // ## m2b-t19-tls
        IssueCode::TlsDisableIgnoresPin => ids::hv::CONN_ISSUE_TLS_DISABLE_IGNORES_PIN,
    };
    tr(cx, msg, NoArgs)
}

// ---- rendering ------------------------------------------------------------------------------

/// How a body element is spaced from the one before it (the mock's sibling margins).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Row,
    Hint,
    Section,
    Disclosure,
}

/// The form body: elements with the mock's spacing between them.
struct Rows {
    items: Vec<(Kind, AnyElement)>,
}

impl Rows {
    fn push(&mut self, kind: Kind, el: impl IntoElement) {
        self.items.push((kind, el.into_any_element()));
    }

    fn into_column(self) -> Div {
        let mut column = div().flex().flex_col().w_full();
        let mut previous: Option<Kind> = None;
        for (kind, el) in self.items {
            let top = match (previous, kind) {
                (None, _) => 0.0,
                (_, Kind::Section) => 14.0,
                (Some(Kind::Section), _) => 0.0,
                (Some(Kind::Disclosure), Kind::Disclosure) => 0.0,
                (_, Kind::Disclosure) => 14.0,
                (_, Kind::Hint) => 4.0,
                _ => 8.0,
            };
            column = column.child(div().w_full().mt(px(top)).child(el));
            previous = Some(kind);
        }
        column
    }
}

fn env_kind(tag: EnvTag) -> EnvKind {
    match tag {
        EnvTag::Local => EnvKind::Local,
        EnvTag::Dev => EnvKind::Dev,
        EnvTag::Test => EnvKind::Test,
        EnvTag::Staging => EnvKind::Staging,
        EnvTag::Production => EnvKind::Production,
    }
}

fn env_label(tag: EnvTag) -> MsgRef<NoArgs> {
    // ## m2a-t21-shots: one mapping for every environment word.
    crate::connections::capsule::env_word_id(Some(tag))
}

fn auth_label(auth: AuthProviderId) -> MsgRef<NoArgs> {
    match auth {
        AuthProviderId::Password => ids::hv::CONN_AUTH_PASSWORD,
        AuthProviderId::PgPass => ids::hv::CONN_AUTH_PGPASS,
        AuthProviderId::ClientCert => ids::hv::CONN_AUTH_CERT,
        AuthProviderId::None => ids::hv::CONN_AUTH_NONE,
    }
}

/// The sign-in methods an engine offers: pgpass only for PostgreSQL.
pub(crate) fn auth_choices(engine: DbmsKind) -> Vec<AuthProviderId> {
    let mut out = vec![AuthProviderId::Password];
    if engine == DbmsKind::Postgres {
        out.push(AuthProviderId::PgPass);
    }
    out.extend([AuthProviderId::ClientCert, AuthProviderId::None]);
    out
}

impl ConnectionDialog {
    /// The whole card: header, scrolling body, test strip and footer.
    pub(crate) fn render_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *Theme::get(cx);
        let title = if self.is_new {
            tr(cx, ids::hv::CONN_DLG_TITLE_NEW, NoArgs)
        } else {
            tr(
                cx,
                ids::hv::CONN_DLG_TITLE_EDIT,
                ids::hv::HvConnDlgTitleEdit {
                    name: Text(self.title_name.clone()),
                },
            )
        };
        let header = self.render_header(title.clone(), &theme, cx);
        let body = self.render_body(&theme, window, cx);
        let footer = self.render_footer(&theme, cx);
        let card = form_dialog(
            &theme,
            header,
            body.into_any_element(),
            Some(self.strip.clone().into_any_element()),
            footer,
        )
        .id("connection-dialog")
        .track_focus(&self.focus.root)
        .on_key_down(cx.listener(on_dialog_key))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        A11yNode::new("connection-dialog-node", Role::Dialog, title, true, card).into_any_element()
    }

    fn render_header(
        &self,
        title: SharedString,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = theme.tokens;
        let c = &t.colors;
        let sub = if self.is_new {
            tr(cx, ids::hv::CONN_DLG_SUB_NEW, NoArgs)
        } else {
            tr(cx, ids::hv::CONN_DLG_SUB_EDIT, NoArgs)
        };
        let close = div()
            .id("conn-dialog-close")
            .debug_selector(|| "conn-dialog-close".to_string())
            .flex()
            .items_center()
            .flex_none()
            .p(px(t.spacing.xs))
            .rounded(px(t.radii.sm))
            .cursor_pointer()
            .hover(|s| s.bg(c.hover.to_hsla()))
            .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
            // Raised Ink Rule: text-2 on the raised card.
            .child(
                Icon::new(IconName::X)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            );
        let close = A11yNode::new(
            "conn-dialog-close-node",
            Role::Button,
            ui_text(cx, "hui-dialog-close"),
            false,
            close,
        );
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.xl))
            .w_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        Label::new(
                            title,
                            TextSource::Fluent,
                            t.type_ramp.body_strong,
                            c.text.to_hsla(),
                        )
                        .id("conn-dialog-title"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(t.spacing.sm))
                            .mt(px(t.spacing.s_2xs))
                            .children(
                                self.draft
                                    .profile
                                    .environment
                                    .map(|e| env_swatch_on_raised(env_kind(e), theme)),
                            )
                            .child(
                                Label::new(
                                    sub,
                                    TextSource::Fluent,
                                    t.type_ramp.body_sm,
                                    c.text_2.to_hsla(),
                                )
                                .id("conn-dialog-subtitle"),
                            ),
                    ),
            )
            .child(close)
            .into_any_element()
    }

    fn render_footer(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let t = theme.tokens;
        let running = matches!(self.strip.read(cx).state(), TestStripState::Running(_));
        let hint = |cx: &App, id: &str| shortcut_label(cx, id).map(SharedString::from);
        let read_only = self.read_only
            || cx
                .try_global::<crate::connections::ConnectionsModel>()
                .is_some_and(|m| m.store_read_only());
        // ## m2a-dialog-retries: the destination's attempts are used up; the Password field's
        // hint says why (P8). Save alone stays.
        let used_up = self.attempts_used_up();
        let mut test = if running {
            Button::new("conn-test", tr(cx, ids::hv::CONN_DLG_TEST_STOP, NoArgs))
                .icon(IconName::Square)
        } else {
            let mut b = Button::new("conn-test", tr(cx, ids::hv::CONN_DLG_TEST, NoArgs))
                .icon(IconName::Plug);
            if let Some(h) = hint(cx, "connection_dialog.test") {
                b = b.key_hint(h);
            }
            b
        }
        .focus_handle(self.focus.test.clone())
        .on_raised(true)
        .disabled(used_up && !running)
        .on_click(cx.listener(|this, _, window, cx| this.toggle_test(window, cx)));
        if running && let Some(h) = escape_label(cx) {
            test = test.key_hint(h);
        }
        let mut save = Button::new("conn-save", tr(cx, ids::hv::CONN_DLG_SAVE, NoArgs))
            .focus_handle(self.focus.save.clone())
            .on_raised(true)
            .disabled(read_only)
            .on_click(cx.listener(|this, _, window, cx| this.save(false, window, cx)));
        if let Some(h) = hint(cx, "connection_dialog.save") {
            save = save.key_hint(h);
        }
        let mut save_connect = Button::new(
            "conn-save-connect",
            tr(cx, ids::hv::CONN_DLG_SAVE_CONNECT, NoArgs),
        )
        .kind(ButtonKind::Primary)
        .focus_handle(self.focus.save_connect.clone())
        .on_raised(true)
        .disabled(read_only || used_up)
        .on_click(cx.listener(|this, _, window, cx| this.save(true, window, cx)));
        if let Some(h) = hint(cx, "connection_dialog.save_and_connect") {
            save_connect = save_connect.key_hint(h);
        }
        // ## m2a-t21-shots: when the buttons do not fit one line (a long locale), the right
        // group wraps under Test, still right-aligned; no button runs past the card.
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(t.spacing.md))
            .w_full()
            .child(test)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_none()
                    .ml_auto()
                    .gap(px(t.spacing.md))
                    .child(
                        Button::new("conn-cancel", tr(cx, ids::hv::CONN_DLG_CANCEL, NoArgs))
                            .kind(ButtonKind::Ghost)
                            .focus_handle(self.focus.cancel.clone())
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    )
                    .child(save)
                    .child(save_connect),
            )
            .into_any_element()
    }

    /// The fill behind a field: the well, or the paste tint fading back to it (NOTES
    /// cross-cutting #7: 1.6 s, then 240 ms).
    fn field_box(&self, field: Field, theme: &Theme, window: &mut Window, cx: &App) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        let well = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        }
        .to_hsla();
        let input = self.inputs[&field].clone();
        let name = field.name();
        let base = div()
            .debug_selector(move || format!("conn-field-{name}"))
            .w_full()
            .min_w_0()
            .rounded(px(t.radii.sm));
        match self
            .tint
            .as_ref()
            .filter(|tint| tint.fields.contains(&field))
        {
            None => base.bg(well).child(input),
            Some(tint) => {
                let progress = tint.progress(cx);
                if tint.fading() {
                    window.request_animation_frame();
                }
                let mut under = well;
                under.a *= progress;
                let mut over = c.blue_tint.to_hsla();
                over.a *= 1.0 - progress;
                base.bg(under).child(
                    div()
                        .debug_selector(move || format!("conn-tint-{name}"))
                        .w_full()
                        .rounded(px(t.radii.sm))
                        .bg(over)
                        .child(input),
                )
            }
        }
    }

    /// A row label next to a control, in `text-2` (the mock's `.inl`).
    fn inline_label(theme: &Theme, text: SharedString) -> Div {
        let t = theme.tokens;
        div().flex_none().child(Label::new(
            text,
            TextSource::Fluent,
            t.type_ramp.body,
            t.colors.text_2.to_hsla(),
        ))
    }

    /// The issues of `field` as hint rows (an alert glyph and the sentence, in ink: no new hue).
    fn issue_rows(&self, rows: &mut Rows, field: Field, theme: &Theme, cx: &App) {
        let t = theme.tokens;
        let c = &t.colors;
        for issue in self
            .issues
            .iter()
            .filter(|i| self.issue_field(i, cx) == Some(field))
        {
            let id = format!("issue-{}", field.name());
            rows.push(
                Kind::Hint,
                hint_row(
                    theme,
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(t.spacing.sm))
                        .child(
                            Icon::new(IconName::TriangleAlert)
                                .size(IconSize::Chrome)
                                .color(c.text.to_hsla()),
                        )
                        .child(
                            Label::new(
                                issue_text(cx, issue),
                                TextSource::Fluent,
                                t.type_ramp.caption,
                                c.text.to_hsla(),
                            )
                            .id(SharedString::from(id)),
                        ),
                ),
            );
        }
    }

    /// A hint under a row: an optional 12 px icon and a 12 px `text-2` line.
    fn hint(theme: &Theme, icon: Option<IconName>, text: SharedString, id: &'static str) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        hint_row(
            theme,
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.sm))
                .children(icon.map(|i| {
                    Icon::new(i)
                        .size(IconSize::Chrome)
                        .color(c.text_2.to_hsla())
                }))
                .child(
                    Label::new(
                        text,
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id(id),
                ),
        )
    }

    fn section(theme: &Theme, text: SharedString) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(7.0))
            .h(px(22.0))
            .mb(px(t.spacing.md))
            .child(Label::new(
                text,
                TextSource::Fluent,
                t.type_ramp.label_sc,
                c.text_2.to_hsla(),
            ))
            .child(div().flex_1().h(px(1.0)).bg(c.seam.to_hsla()))
    }

    fn choose_button(
        &self,
        id: &'static str,
        field: Field,
        icon: Option<IconName>,
        cx: &mut Context<Self>,
    ) -> Button {
        let handle = match field {
            Field::File => self.focus.choose_file.clone(),
            Field::Ca => self.focus.choose_ca.clone(),
            Field::Cert => self.focus.choose_cert.clone(),
            _ => self.focus.choose_key.clone(),
        };
        let mut b = Button::new(id, tr(cx, ids::hv::CONN_DLG_CHOOSE, NoArgs))
            .focus_handle(handle)
            .disabled(self.read_only)
            .on_raised(true)
            .on_click(cx.listener(move |this, _, window, cx| this.choose_file(field, window, cx)));
        if let Some(icon) = icon {
            b = b.icon(icon);
        }
        b
    }

    fn render_body(&mut self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = self.draft.profile.clone();
        let mut rows = Rows { items: Vec::new() };

        if let Some(problem) = self.problem {
            let msg = match problem {
                SaveProblem::StoreReadOnly => ids::hv::CONN_DLG_STORE_READ_ONLY,
                SaveProblem::Failed => ids::hv::CONN_DLG_SAVE_FAILED,
                SaveProblem::Gone => ids::hv::CONN_DLG_SAVE_GONE,
                SaveProblem::SecretNotStored => ids::hv::CONN_DLG_SAVE_SECRET,
                // ## m2b-t21-prompts (fix round 1, I1)
                SaveProblem::Unchanged => ids::hv::CONN_SAVE_UNCHANGED,
            };
            rows.push(
                Kind::Row,
                InlineNotice::new(NoticeIcon::Info).line(tr(cx, msg, NoArgs)),
            );
        }
        // ## m2b-t01-interfaces: the SSH section draws its own issues (Task 20, Q9).
        for issue in self.issues.iter().filter(|i| {
            self.issue_field(i, cx).is_none() && !super::ssh::owns_issue(&self.draft.profile, i, cx)
        }) {
            rows.push(
                Kind::Row,
                InlineNotice::new(NoticeIcon::Info).line(issue_text(cx, issue)),
            );
        }

        // URL row, with the copy button inside the field.
        // The button sits on the field's own well, so a long URL never runs under it.
        let well = match theme.mode {
            ThemeMode::Dark => theme.tokens.colors.panel,
            ThemeMode::Light => theme.tokens.colors.ground,
        }
        .to_hsla();
        let copy = div()
            .absolute()
            .top(px(1.0))
            .bottom(px(1.0))
            .right(px(1.0))
            .pl(px(theme.tokens.spacing.xs))
            .flex()
            .items_center()
            .rounded(px(theme.tokens.radii.sm))
            .bg(well)
            .child(
                Button::new("conn-copy-url", tr(cx, ids::hv::CONN_DLG_COPY_URL, NoArgs))
                    .kind(ButtonKind::Icon)
                    .icon(IconName::Copy)
                    .focus_handle(self.focus.copy_url.clone())
                    .on_click(cx.listener(|this, _, _, cx| this.copy_as_url(cx))),
            );
        let url = div()
            .relative()
            .w_full()
            .min_w_0()
            .child(self.field_box(Field::Url, theme, window, cx))
            .child(copy);
        rows.push(
            Kind::Row,
            form_row(theme, tr(cx, ids::hv::CONN_DLG_URL, NoArgs), url),
        );
        if let Some(notice) = self.render_notice(theme, cx) {
            rows.push(Kind::Row, notice);
        }

        // Engine.
        let engines = ENGINES.iter().fold(
            Segmented::new("conn-engine")
                .in_well(true)
                .inert(self.read_only),
            |seg, &kind| {
                seg.item(SegmentItem {
                    content: SegmentContent::Verbatim(kind.display_name().into()),
                    pressed: p.engine == kind,
                    tooltip: kind.display_name().into(),
                    a11y_name: None,
                    on_click: Rc::new({
                        let me = cx.weak_entity();
                        move |_, window, cx| {
                            let _ = me.update(cx, |d, cx| d.set_engine(kind, window, cx));
                        }
                    }),
                })
            },
        );
        rows.push(
            Kind::Row,
            form_row(
                theme,
                tr(cx, ids::hv::CONN_DLG_ENGINE, NoArgs),
                div()
                    .track_focus(&self.focus.engine)
                    .flex()
                    .min_w_0()
                    .child(engines),
            ),
        );
        rows.push(
            Kind::Row,
            form_row(
                theme,
                tr(cx, ids::hv::CONN_DLG_NAME, NoArgs),
                self.field_box(Field::Name, theme, window, cx),
            ),
        );
        self.issue_rows(&mut rows, Field::Name, theme, cx);

        // Environment, with its marks.
        let envs = EnvTag::ALL.iter().fold(
            Segmented::new("conn-env")
                .in_well(true)
                .inert(self.read_only),
            |seg, &tag| {
                let label = tr(cx, env_label(tag), NoArgs);
                seg.item(SegmentItem {
                    content: SegmentContent::Marked(env_kind(tag), label.clone()),
                    pressed: p.environment == Some(tag),
                    tooltip: label,
                    a11y_name: None,
                    on_click: Rc::new({
                        let me = cx.weak_entity();
                        move |_, window, cx| {
                            let _ = me.update(cx, |d, cx| d.set_environment(tag, window, cx));
                        }
                    }),
                })
            },
        );
        // ## m2a-t21-shots: when the row cannot show every label whole, a Select instead of
        // five ellipsized segments (the Overflow Rule: no label truncates while a whole
        // control is possible).
        let labels: Vec<(SharedString, bool)> = EnvTag::ALL
            .iter()
            .map(|tag| (tr(cx, env_label(*tag), NoArgs), true))
            .collect();
        let as_select = natural_width(&labels, theme, window) > px(control_room(theme));
        self.env_as_select.set(as_select);
        let control = if as_select {
            div()
                .debug_selector(|| "conn-env-select".to_string())
                .w(px(ENV_SELECT_PX))
                .max_w_full()
                .child(self.env_select.clone())
                .into_any_element()
        } else {
            envs.into_any_element()
        };
        rows.push(
            Kind::Row,
            form_row(
                theme,
                tr(cx, ids::hv::CONN_DLG_ENV, NoArgs),
                div()
                    .track_focus(&self.focus.environment)
                    .flex()
                    .min_w_0()
                    .child(control),
            ),
        );
        let has_host = !host_text(&p).trim().is_empty();
        let local = hacivat_session::profile::is_local_endpoint(&p.endpoint);
        if !self.env_touched && p.engine != DbmsKind::Sqlite && has_host {
            if local {
                rows.push(
                    Kind::Hint,
                    Self::hint(
                        theme,
                        None,
                        tr(cx, ids::hv::CONN_DLG_ENV_LOCAL, NoArgs),
                        "env-hint",
                    ),
                );
            } else {
                rows.push(
                    Kind::Hint,
                    Self::hint(
                        theme,
                        Some(IconName::Info),
                        tr(cx, ids::hv::CONN_DLG_ENV_ASK, NoArgs),
                        "env-hint",
                    ),
                );
            }
        }
        let me = cx.weak_entity();
        rows.push(
            Kind::Row,
            hint_row(
                theme,
                Checkbox::new("conn-read-only", tr(cx, ids::hv::CONN_DLG_RO, NoArgs))
                    .sub_label(tr(cx, ids::hv::CONN_DLG_RO_HINT, NoArgs))
                    .checked(p.read_only)
                    .disabled(self.read_only)
                    .focus_handle(self.focus.read_only.clone())
                    .on_toggle(move |on, window, cx| {
                        let _ = me.update(cx, |d, cx| {
                            d.draft.profile.read_only = on;
                            d.after_edit(window, cx);
                        });
                    }),
            ),
        );

        rows.push(
            Kind::Section,
            Self::section(theme, tr(cx, ids::hv::CONN_SECTION_SERVER, NoArgs)),
        );
        if p.engine == DbmsKind::Sqlite {
            self.render_sqlite(&mut rows, &p, theme, window, cx);
        } else {
            self.render_server(&mut rows, theme, window, cx);
            self.render_auth(&mut rows, &p, theme, window, cx);
            self.render_tls(&mut rows, &p, theme, window, cx);
            // ## m2b-t01-interfaces: the SSH section's hook. Task 20: on every server form,
            // between TLS and Advanced (the mock's order; its checkbox turns the tunnel on).
            rows.push(
                Kind::Disclosure,
                super::ssh::section(&self.draft, window, cx),
            );
            self.render_advanced(&mut rows, &p, theme, window, cx);
        }
        rows.into_column()
    }

    fn render_sqlite(
        &mut self,
        rows: &mut Rows,
        p: &Profile,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let t = theme.tokens;
        let file = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.md))
            .w_full()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(self.field_box(Field::File, theme, window, cx)),
            )
            .child(self.choose_button(
                "conn-choose-file",
                Field::File,
                Some(IconName::FolderOpen),
                cx,
            ));
        rows.push(
            Kind::Row,
            form_row(theme, tr(cx, ids::hv::CONN_DLG_FILE, NoArgs), file),
        );
        self.issue_rows(rows, Field::File, theme, cx);
        let create = matches!(
            p.endpoint,
            ProfileEndpoint::File {
                create_if_missing: true,
                ..
            }
        );
        let (me_ro, me_create) = (cx.weak_entity(), cx.weak_entity());
        rows.push(
            Kind::Row,
            hint_row(
                theme,
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(t.spacing.md))
                    .child(
                        Checkbox::new(
                            "conn-sqlite-ro",
                            tr(cx, ids::hv::CONN_DLG_SQLITE_RO, NoArgs),
                        )
                        .checked(sqlite_mode_ro(p))
                        .disabled(self.read_only)
                        .focus_handle(self.focus.sqlite_ro.clone())
                        .on_toggle(move |on, window, cx| {
                            let _ = me_ro.update(cx, |d, cx| {
                                // The SQLite option `mode=ro`, apart from the Hacivat guard
                                // (ruling A).
                                set_sqlite_mode_ro(&mut d.draft.profile, on);
                                d.rebuild_extra_rows(window, cx);
                                d.after_edit(window, cx);
                            });
                        }),
                    )
                    .child(
                        Checkbox::new(
                            "conn-sqlite-create",
                            tr(cx, ids::hv::CONN_DLG_SQLITE_CREATE, NoArgs),
                        )
                        .checked(create)
                        .disabled(self.read_only)
                        .focus_handle(self.focus.sqlite_create.clone())
                        .on_toggle(move |on, window, cx| {
                            let _ = me_create.update(cx, |d, cx| {
                                if let ProfileEndpoint::File {
                                    create_if_missing, ..
                                } = &mut d.draft.profile.endpoint
                                {
                                    *create_if_missing = on;
                                }
                                d.after_edit(window, cx);
                            });
                        }),
                    ),
            ),
        );
    }

    fn render_server(
        &mut self,
        rows: &mut Rows,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let t = theme.tokens;
        let host = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.md))
            .w_full()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(self.field_box(Field::Host, theme, window, cx)),
            )
            .child(Self::inline_label(
                theme,
                tr(cx, ids::hv::CONN_DLG_PORT, NoArgs),
            ))
            .child(div().flex_none().w(px(76.0)).child(self.field_box(
                Field::Port,
                theme,
                window,
                cx,
            )));
        rows.push(
            Kind::Row,
            form_row(theme, tr(cx, ids::hv::CONN_DLG_HOST, NoArgs), host),
        );
        self.issue_rows(rows, Field::Host, theme, cx);
        self.issue_rows(rows, Field::Port, theme, cx);
        rows.push(
            Kind::Row,
            form_row(
                theme,
                tr(cx, ids::hv::CONN_DLG_DB, NoArgs),
                self.field_box(Field::Database, theme, window, cx),
            ),
        );
        self.issue_rows(rows, Field::Database, theme, cx);
        rows.push(
            Kind::Row,
            form_row(
                theme,
                tr(cx, ids::hv::CONN_DLG_USER, NoArgs),
                self.field_box(Field::User, theme, window, cx),
            ),
        );
        self.issue_rows(rows, Field::User, theme, cx);
    }

    /// A masked secret field, its "Remember" label and the retention select.
    fn secret_row(
        &self,
        field: Field,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        let mut secret = div()
            .relative()
            .flex_1()
            .min_w_0()
            .child(self.field_box(field, theme, window, cx));
        if field == Field::Password && self.password_from_url && self.draft.password.is_some() {
            // The mock's tail: where the password came from.
            // ## m2a-t21-shots: the tail keeps to the field's right part (the dots stay
            // visible) and ellipsizes there instead of spilling over the field and its label.
            secret = secret.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(TAIL_LEFT))
                    .right(px(t.spacing.sm))
                    .flex()
                    .items_center()
                    .justify_end()
                    .min_w_0()
                    .child(
                        Label::new(
                            tr(cx, ids::hv::CONN_DLG_PW_FROM_URL, NoArgs),
                            TextSource::Fluent,
                            t.type_ramp.caption,
                            c.text_2.to_hsla(),
                        )
                        .truncate(true),
                    ),
            );
        }
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.md))
            .w_full()
            .child(secret)
            .child(Self::inline_label(
                theme,
                tr(cx, ids::hv::CONN_DLG_REMEMBER, NoArgs),
            ))
            .child(div().flex_none().w(px(186.0)).child(self.remember.clone()))
    }

    fn render_auth(
        &mut self,
        rows: &mut Rows,
        p: &Profile,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let t = theme.tokens;
        let c = &t.colors;
        rows.push(
            Kind::Section,
            Self::section(theme, tr(cx, ids::hv::CONN_SECTION_AUTH, NoArgs)),
        );
        let auths = auth_choices(p.engine).into_iter().fold(
            Segmented::new("conn-auth")
                .in_well(true)
                .inert(self.read_only),
            |seg, auth| {
                seg.item(SegmentItem {
                    content: SegmentContent::Label(tr(cx, auth_label(auth), NoArgs)),
                    pressed: p.auth == auth,
                    tooltip: tr(cx, auth_label(auth), NoArgs),
                    a11y_name: None,
                    on_click: Rc::new({
                        let me = cx.weak_entity();
                        move |_, window, cx| {
                            let _ = me.update(cx, |d, cx| d.set_auth(auth, window, cx));
                        }
                    }),
                })
            },
        );
        rows.push(
            Kind::Row,
            form_row(
                theme,
                tr(cx, ids::hv::CONN_DLG_AUTH, NoArgs),
                div()
                    .track_focus(&self.focus.auth)
                    .flex()
                    .min_w_0()
                    .child(auths),
            ),
        );
        match p.auth {
            AuthProviderId::Password => {
                let row = self.secret_row(Field::Password, theme, window, cx);
                rows.push(
                    Kind::Row,
                    form_row(theme, tr(cx, ids::hv::CONN_DLG_PASSWORD, NoArgs), row),
                );
                // ## m2a-dialog-retries: after a rejection, which of the 3 attempts the next
                // test or connect is (the error dialog's caption); once all 3 are used, why
                // Test and Save and connect send nothing (P8).
                let rejections = self.rejections();
                if self.attempts_used_up() {
                    rows.push(
                        Kind::Hint,
                        Self::hint(
                            theme,
                            Some(IconName::TriangleAlert),
                            tr(cx, ids::hv::CONN_DLG_ATTEMPTS_USED, NoArgs),
                            "password-attempts-used",
                        ),
                    );
                } else if rejections > 0 {
                    rows.push(
                        Kind::Hint,
                        Self::hint(
                            theme,
                            None,
                            tr(
                                cx,
                                ids::hv::CONN_ERR_ATTEMPT,
                                ids::hv::HvConnErrAttempt {
                                    n: Count(u64::from(rejections) + 1),
                                },
                            ),
                            "password-attempt",
                        ),
                    );
                }
            }
            AuthProviderId::PgPass => {
                let line = self.pgpass.as_ref().and_then(|s| s.line);
                let matched = match line {
                    Some(n) => {
                        let (host, port, db, user) = pgpass_key(p);
                        let port = if port == 0 {
                            EngineCatalog::embedded()
                                .get(p.engine)
                                .and_then(|e| e.default_port)
                                .unwrap_or_default()
                        } else {
                            port
                        };
                        let entry = [
                            host,
                            port.to_string(),
                            db,
                            user.unwrap_or_default(),
                            "••••".to_string(),
                        ]
                        .join(":");
                        hint_row(
                            theme,
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(t.spacing.sm))
                                .child(
                                    Icon::new(IconName::Check)
                                        .size(IconSize::Chrome)
                                        .color(c.text_2.to_hsla()),
                                )
                                .child(
                                    Label::new(
                                        tr(
                                            cx,
                                            ids::hv::CONN_PGPASS_MATCH,
                                            ids::hv::HvConnPgpassMatch {
                                                n: Count(u64::from(n)),
                                            },
                                        ),
                                        TextSource::Fluent,
                                        t.type_ramp.caption,
                                        c.text_2.to_hsla(),
                                    )
                                    .id("pgpass-line"),
                                )
                                .child(Label::new(
                                    entry,
                                    TextSource::Data,
                                    t.type_ramp.mono_data,
                                    c.text.to_hsla(),
                                )),
                        )
                    }
                    None => Self::hint(
                        theme,
                        Some(IconName::Info),
                        tr(cx, ids::hv::CONN_PGPASS_NONE, NoArgs),
                        "pgpass-line",
                    ),
                };
                rows.push(Kind::Hint, matched);
                rows.push(
                    Kind::Hint,
                    Self::hint(
                        theme,
                        None,
                        tr(cx, ids::hv::CONN_PGPASS_NOTE, NoArgs),
                        "pgpass-note",
                    ),
                );
            }
            AuthProviderId::ClientCert => {
                for (field, label, id) in [
                    (Field::Cert, ids::hv::CONN_DLG_CERT, "conn-choose-cert"),
                    (Field::Key, ids::hv::CONN_DLG_KEY, "conn-choose-key"),
                ] {
                    let row = div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(t.spacing.md))
                        .w_full()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(self.field_box(field, theme, window, cx)),
                        )
                        .child(self.choose_button(id, field, None, cx));
                    rows.push(Kind::Row, form_row(theme, tr(cx, label, NoArgs), row));
                    self.issue_rows(rows, field, theme, cx);
                }
                // ## m2b-t19-tls: the key's passphrase, masked, kept as the profile remembers
                // its secrets (B25). A pasted `sslpassword` fills it too.
                let row = self.secret_row(Field::KeyPass, theme, window, cx);
                rows.push(
                    Kind::Row,
                    form_row(theme, tr(cx, ids::hv::CONN_DLG_KEY_PASS, NoArgs), row),
                );
                self.issue_rows(rows, Field::KeyPass, theme, cx);
            }
            AuthProviderId::None => {
                rows.push(
                    Kind::Hint,
                    Self::hint(
                        theme,
                        Some(IconName::Info),
                        tr(cx, ids::hv::CONN_AUTH_NONE_NOTE, NoArgs),
                        "auth-none-note",
                    ),
                );
            }
        }
    }

    fn render_tls(
        &mut self,
        rows: &mut Rows,
        p: &Profile,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let t = theme.tokens;
        let c = &t.colors;
        let summary = match p.tls.mode {
            TlsMode::Auto => tr(cx, ids::hv::CONN_TLS_SUM_AUTO, NoArgs),
            mode => tr(cx, tls_label(mode), NoArgs),
        };
        let mut content = Rows { items: Vec::new() };
        let mode = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.md))
            .w_full()
            .child(div().flex_none().w(px(200.0)).child(self.tls_mode.clone()))
            .child(
                div().flex_1().min_w_0().child(
                    Label::new(
                        tr(cx, tls_hint(p.engine, p.tls.mode), NoArgs),
                        TextSource::Fluent,
                        t.type_ramp.body_sm,
                        c.text_2.to_hsla(),
                    )
                    .truncate(true),
                ),
            );
        content.push(
            Kind::Row,
            form_row(theme, tr(cx, ids::hv::CONN_TLS_MODE, NoArgs), mode),
        );
        if p.tls.mode == TlsMode::Disable && is_mysql_family(p.engine) {
            // Controller ruling (T14): say plainly what Disable exposes.
            content.push(
                Kind::Row,
                InlineNotice::new(NoticeIcon::Info).line(tr(
                    cx,
                    ids::hv::CONN_TLS_DISABLE_WARN,
                    NoArgs,
                )),
            );
        }
        let ca = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.md))
            .w_full()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(self.field_box(Field::Ca, theme, window, cx)),
            )
            .child(self.choose_button("conn-choose-ca", Field::Ca, None, cx));
        content.push(
            Kind::Row,
            form_row(theme, tr(cx, ids::hv::CONN_TLS_CA, NoArgs), ca),
        );
        self.issue_rows(&mut content, Field::Ca, theme, cx);
        // ## m2b-t19-tls: the pinned certificate of this endpoint (NOTES M2a surface 1).
        let pin_row = self.render_pin(theme, cx);
        content.push(
            Kind::Row,
            form_row(theme, tr(cx, ids::hv::CONN_TLS_PIN, NoArgs), pin_row),
        );
        let (host, port) = self.tls_endpoint();
        if p.tls.disable_ignores_pin(&host, port) {
            // Disable wins over the pin (T3 ruling M2): say that the pin is not checked.
            content.push(
                Kind::Row,
                InlineNotice::new(NoticeIcon::Info).line(issue_text(
                    cx,
                    &ValidationIssue {
                        field: None,
                        code: IssueCode::TlsDisableIgnoresPin,
                    },
                )),
            );
        }
        let me = cx.weak_entity();
        let disclosure = Disclosure::new("conn-tls", tr(cx, ids::hv::CONN_SECTION_TLS, NoArgs))
            .summary(summary)
            .open(self.open_tls)
            .on_toggle(move |open, _, cx| {
                let _ = me.update(cx, |d, cx| {
                    d.open_tls = open;
                    cx.notify();
                });
            })
            .content(content.into_column().into_any_element());
        rows.push(
            Kind::Disclosure,
            div().track_focus(&self.focus.tls).child(disclosure),
        );
    }

    // ## m2b-t19-tls
    /// The "Pinned certificate" value: `SHA-256` and a short fingerprint (its first four and last
    /// two bytes, the mock's form; the review shows all 32) with Remove pin, or None.
    fn render_pin(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let t = theme.tokens;
        let c = &t.colors;
        let Some(pin) = self.current_pin() else {
            return Label::new(
                tr(cx, ids::hv::CONN_TLS_PIN_NONE, NoArgs),
                TextSource::Fluent,
                t.type_ramp.body,
                c.text_2.to_hsla(),
            )
            .id("tls-pin-row")
            .into_any_element();
        };
        let [first, second] = hacivat_net::fingerprint_lines(&pin.sha256);
        let short = format!("{}{ELLIPSIS}{}", &first[..11], &second[second.len() - 5..]);
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.md))
            .min_w_0()
            .child(div().flex_none().child(Label::new(
                SharedString::from(cert_dialog::SHA256_LABEL),
                TextSource::Data,
                t.type_ramp.mono_data,
                c.text_2.to_hsla(),
            )))
            .child(
                div().flex_none().child(
                    Label::new(
                        SharedString::from(short),
                        TextSource::Data,
                        t.type_ramp.mono_data,
                        c.text.to_hsla(),
                    )
                    .id("tls-pin-row"),
                ),
            )
            .child(
                Button::new(
                    "tls-remove-pin",
                    tr(cx, ids::hv::CONN_TLS_REMOVE_PIN, NoArgs),
                )
                .kind(ButtonKind::Ghost)
                .focus_handle(self.focus.remove_pin.clone())
                .disabled(self.read_only)
                .on_raised(true)
                .on_click(cx.listener(|this, _, window, cx| this.remove_pin(window, cx))),
            )
            .into_any_element()
    }

    fn render_advanced(
        &mut self,
        rows: &mut Rows,
        p: &Profile,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let t = theme.tokens;
        let c = &t.colors;
        let keepalive = p.keepalive_s;
        let summary = tr(
            cx,
            ids::hv::CONN_ADV_SUM,
            ids::hv::HvConnAdvSum {
                timeout: Count(u64::from(p.connect_timeout_s)),
                keepalive: if keepalive.is_some() {
                    ids::hv::HvConnAdvSumKeepalive::On
                } else {
                    ids::hv::HvConnAdvSumKeepalive::Off
                },
                seconds: Count(u64::from(keepalive.unwrap_or(0))),
            },
        );
        let mut content = Rows { items: Vec::new() };
        let timeout = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.md))
            .child(div().flex_none().w(px(96.0)).child(self.field_box(
                Field::Timeout,
                theme,
                window,
                cx,
            )))
            .child(Self::inline_label(
                theme,
                tr(cx, ids::hv::CONN_ADV_TIMEOUT_UNIT, NoArgs),
            ));
        content.push(
            Kind::Row,
            form_row(theme, tr(cx, ids::hv::CONN_ADV_TIMEOUT, NoArgs), timeout),
        );
        content.push(
            Kind::Row,
            form_row(
                theme,
                tr(cx, ids::hv::CONN_ADV_KEEPALIVE, NoArgs),
                div().flex_none().w(px(120.0)).child(self.keepalive.clone()),
            ),
        );
        content.push(
            Kind::Row,
            form_row(
                theme,
                tr(cx, ids::hv::CONN_ADV_APP, NoArgs),
                self.field_box(Field::AppName, theme, window, cx),
            ),
        );
        content.push(
            Kind::Row,
            form_row(
                theme,
                tr(cx, ids::hv::CONN_ADV_BUDGET, NoArgs),
                Label::new(
                    tr(
                        cx,
                        ids::hv::CONN_ADV_BUDGET_V,
                        ids::hv::HvConnAdvBudgetV {
                            max: Count(u64::from(p.max_connections)),
                        },
                    ),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                ),
            ),
        );
        let well = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        }
        .to_hsla();
        let mut extras = div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.sm))
            .flex_1()
            .min_w_0();
        for (i, row) in self.extras.iter().enumerate() {
            extras = extras.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.md))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .rounded(px(t.radii.sm))
                            .bg(well)
                            .child(row.key.clone()),
                    )
                    .child(div().flex_none().child(Label::new(
                        "=",
                        TextSource::Data,
                        t.type_ramp.body,
                        c.text_2.to_hsla(),
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .rounded(px(t.radii.sm))
                            .bg(well)
                            .child(row.value.clone()),
                    )
                    .child(
                        Button::new(
                            ("conn-extra-remove", i),
                            tr(cx, ids::hv::CONN_ADV_EXTRA_REMOVE, NoArgs),
                        )
                        .kind(ButtonKind::Icon)
                        .icon(IconName::X)
                        .focus_handle(row.remove.clone())
                        .disabled(self.read_only)
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.remove_extra_row(i, window, cx),
                        )),
                    ),
            );
        }
        extras = extras.child(
            div().child(
                Button::new(
                    "conn-extra-add",
                    tr(cx, ids::hv::CONN_ADV_EXTRA_ADD, NoArgs),
                )
                .kind(ButtonKind::Ghost)
                .icon(IconName::Plus)
                .focus_handle(self.focus.add_extra.clone())
                .disabled(self.read_only)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.push_extra_row("", "", window, cx);
                    if let Some(row) = this.extras.last() {
                        let handle = row.key.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                    }
                    cx.notify();
                })),
            ),
        );
        content.push(
            Kind::Row,
            form_row(theme, tr(cx, ids::hv::CONN_ADV_EXTRA, NoArgs), extras),
        );
        self.issue_rows(&mut content, Field::AppName, theme, cx);
        let me = cx.weak_entity();
        let disclosure = Disclosure::new(
            "conn-advanced",
            tr(cx, ids::hv::CONN_SECTION_ADVANCED, NoArgs),
        )
        .summary(summary)
        .open(self.open_adv)
        .on_toggle(move |open, _, cx| {
            let _ = me.update(cx, |d, cx| {
                d.open_adv = open;
                cx.notify();
            });
        })
        .content(content.into_column().into_any_element());
        rows.push(
            Kind::Disclosure,
            div().track_focus(&self.focus.advanced).child(disclosure),
        );
    }
}

/// The ellipsis between the two ends of a short fingerprint (a symbol, not text).
const ELLIPSIS: char = '…';

/// The Escape key as the active keymap writes it (the Stop hint).
fn escape_label(cx: &App) -> Option<SharedString> {
    let active = cx.try_global::<crate::shell::ActiveKeymap>()?;
    let chord = hacivat_ui_core::keymap::parse_chord("escape").ok()?;
    Some(hacivat_ui_core::keymap::display_chord(&chord, active.platform).into())
}
