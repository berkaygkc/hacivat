//! Importing `pg_service.conf` sections as profiles (R24, Task 18).
//!
//! The service files libpq reads (`pg_service_paths()`: the user file, then the system file)
//! are read on the core runtime's blocking pool, never on the UI thread. A section name seen
//! twice keeps its first occurrence (libpq's lookup order). A 480 px modal lists the services,
//! each with a checkbox and an Environment Select: a local endpoint is `local`, a production
//! token in the host or name (R22, P7) preselects Production, anything else starts unset.
//! Import stays disabled until every checked remote row has an environment. A password left
//! out because the file is readable by others is listed (`PasswordIgnoredInsecureFile`).
//!
//! Imported profiles are `Origin::Imported { source: PgService }` (their options go through
//! DRV-05 at connect), named uniquely with `duplicate_name` (R23, R24), and a password from the
//! file is saved with the profile under retention Forever.

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_proto::{DbmsKind, EnvTag, ImportSource, Origin};
use hacivat_secrets::{
    AuthProviderId, Retention, SecretString, ServiceFileWarning, pg_service_paths, read_pg_service,
};
use hacivat_session::profile::{
    Profile, ProfileDraft, ProfileEndpoint, duplicate_name, is_local_endpoint,
    needs_environment_prompt, production_hint,
};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::checkbox::Checkbox;
use hacivat_ui::dialog::{DialogSize, dialog_frame};
use hacivat_ui::select::{Select, SelectEvent, SelectOption};
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    Global, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, SharedString,
    Styled, Subscription, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, TypeStyle};
use tokio::sync::oneshot;

use crate::connections::commands::{refuse_read_only, show_save_error};
use crate::connections::explorer::{endpoint_text, paragraph};
use crate::connections::model::ConnectionsModel;
use crate::l10n_global::{text_fn, tr};
use crate::shell::Workspace;
use crate::shell::layers::{DIALOG_CONTEXT, Layer};

/// The import dialog's layer ID.
pub const IMPORT_LAYER: &str = "import";

/// The service files to read instead of `pg_service_paths()` (tests and the shots harness).
pub struct ImportSources(pub Vec<PathBuf>);

impl Global for ImportSources {}

/// One service, as read (holds the password, so `Debug` is not derived).
struct Service {
    name: String,
    fields: Vec<(String, String)>,
    password: Option<SecretString>,
}

/// What the files gave.
struct ReadOut {
    services: Vec<Service>,
    /// Services whose password was left out (the file is readable by others).
    ignored: Vec<String>,
    unreadable: bool,
}

/// Reads every service file (blocking; on the core runtime). Missing files are skipped; the
/// first section of a name wins.
fn read_services(sources: Option<Vec<PathBuf>>) -> ReadOut {
    let paths = sources.unwrap_or_else(pg_service_paths);
    let mut out = ReadOut {
        services: Vec::new(),
        ignored: Vec::new(),
        unreadable: false,
    };
    let mut seen = HashSet::new();
    for path in paths {
        let file = match read_pg_service(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "import: reading a service file failed");
                out.unreadable = true;
                continue;
            }
        };
        for warning in file.warnings {
            let ServiceFileWarning::PasswordIgnoredInsecureFile { service, .. } = warning;
            if !out.ignored.contains(&service) {
                out.ignored.push(service);
            }
        }
        for s in file.services {
            if seen.insert(s.name.clone()) {
                out.services.push(Service {
                    name: s.name,
                    fields: s.fields,
                    password: s.password,
                });
            }
        }
    }
    out
}

/// Password-like option names that never stay in `Profile.extra` (the file would keep the
/// secret in the clear). This mirrors `hacivat_driver::url`'s `is_secret_param`, which is not
/// public: these exact names, ASCII case and leading dashes ignored...
const SECRET_PARAMS: [&str; 8] = [
    "password",
    "sslpassword",
    "pass",
    "pwd",
    "passwd",
    "pgpassword",
    "mysql_pwd",
    "sslkeypassword",
];
/// ...and these endings (`db_password`, `x-pwd`).
const SECRET_SUFFIXES: [&str; 4] = ["_password", "-password", "_pwd", "-pwd"];

/// Whether the option `key` carries a secret.
fn is_secret_param(key: &str) -> bool {
    let key = key.trim_start_matches('-');
    SECRET_PARAMS.iter().any(|s| s.eq_ignore_ascii_case(key))
        || SECRET_SUFFIXES.iter().any(|s| {
            key.len() >= s.len()
                && key.is_char_boundary(key.len() - s.len())
                && key[key.len() - s.len()..].eq_ignore_ascii_case(s)
        })
}

/// A profile for a service (libpq keeps the first value of a repeated key), and the TLS key
/// passphrase from its `sslpassword` (any ASCII case), which is saved in
/// `SecretSlot::TlsKeyPassphrase` like a pasted one (T7). Options other than the endpoint,
/// database and user stay as extras (DRV-05 checks them at connect), except password-like
/// ones, which are dropped: a secret never lands in `connections.toml`.
fn profile_of(
    name: String,
    fields: Vec<(String, String)>,
    password: bool,
) -> (Profile, Option<SecretString>) {
    let mut tls_key_passphrase = None;
    let mut host: Option<String> = None;
    let mut port: Option<u16> = None;
    let mut database = None;
    let mut user = None;
    let mut extra = Vec::new();
    for (key, value) in fields {
        match key.as_str() {
            "host" => {
                host.get_or_insert(value);
            }
            "port" => {
                if port.is_none() {
                    port = value.trim().parse().ok();
                }
            }
            "dbname" => {
                database.get_or_insert(value);
            }
            "user" => {
                user.get_or_insert(value);
            }
            _ if key.eq_ignore_ascii_case("sslpassword") => {
                if tls_key_passphrase.is_none() {
                    tls_key_passphrase = Some(SecretString::from(value));
                }
            }
            _ if is_secret_param(&key) => {}
            _ => extra.push((key, value)),
        }
    }
    let host = host.unwrap_or_default();
    let endpoint = if host.starts_with('/') {
        ProfileEndpoint::Socket {
            path: PathBuf::from(host),
            port,
        }
    } else {
        ProfileEndpoint::Server { host, port }
    };
    let (auth, retention) = if password {
        (AuthProviderId::Password, Retention::Forever)
    } else {
        (AuthProviderId::PgPass, Retention::default())
    };
    let profile = Profile {
        name,
        engine: DbmsKind::Postgres,
        endpoint,
        database,
        user,
        auth,
        retention,
        extra,
        origin: Origin::Imported {
            source: ImportSource::PgService,
        },
        ..Profile::default()
    };
    (profile, tls_key_passphrase)
}

/// The preselected environment (R22, R24): local endpoints are `local`; a production token in
/// the host or the name preselects Production; otherwise none.
fn preselect(p: &Profile) -> Option<EnvTag> {
    if is_local_endpoint(&p.endpoint) {
        return Some(EnvTag::Local);
    }
    let host = match &p.endpoint {
        ProfileEndpoint::Server { host, .. } => host.as_str(),
        _ => "",
    };
    production_hint(host, &p.name).map(|_| EnvTag::Production)
}

/// The Select's options: "Choose…", then the tags by severity.
const CHOICES: [Option<EnvTag>; 6] = [
    None,
    Some(EnvTag::Production),
    Some(EnvTag::Staging),
    Some(EnvTag::Test),
    Some(EnvTag::Dev),
    Some(EnvTag::Local),
];

fn choice_label(tag: Option<EnvTag>) -> Rc<dyn Fn(&App) -> SharedString> {
    use ids::hv::HvConnImportEnvEnv as E;
    let env = match tag {
        Some(EnvTag::Production) => E::Production,
        Some(EnvTag::Staging) => E::Staging,
        Some(EnvTag::Test) => E::Test,
        Some(EnvTag::Dev) => E::Dev,
        Some(EnvTag::Local) => E::Local,
        None => E::Unset,
    };
    Rc::new(move |cx: &App| {
        tr(
            cx,
            ids::hv::CONN_IMPORT_ENV,
            ids::hv::HvConnImportEnv { env },
        )
    })
}

/// One row of the dialog.
struct Row {
    profile: Profile,
    password: Option<SecretString>,
    tls_key_passphrase: Option<SecretString>,
    env: Option<EnvTag>,
    checked: bool,
    select: Entity<Select>,
    check_focus: FocusHandle,
}

/// The import dialog.
pub struct ImportView {
    rows: Vec<Row>,
    ignored: Vec<String>,
    cancel_focus: FocusHandle,
    confirm_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// What the dialog tells its host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportEvent {
    /// Cancelled, or imported: close the layer.
    Closed,
}

impl EventEmitter<ImportEvent> for ImportView {}

/// The open dialog (tests reach its rows through it).
struct OpenImport(WeakEntity<ImportView>);

impl Global for OpenImport {}

/// The open import dialog, if any.
pub fn open_view(cx: &App) -> Option<Entity<ImportView>> {
    cx.try_global::<OpenImport>()?.0.upgrade()
}

/// `connection.import_pg_service`: reads the service files on the core runtime, then shows the
/// dialog (or a toast when there is nothing to import).
pub fn open(window: &mut Window, cx: &mut App) {
    if refuse_read_only(cx) {
        return;
    }
    let Some(m) = cx.try_global::<ConnectionsModel>() else {
        return;
    };
    let sources = cx.try_global::<ImportSources>().map(|s| s.0.clone());
    let (tx, rx) = oneshot::channel();
    m.runtime().spawn_blocking(move || {
        let _ = tx.send(read_services(sources));
    });
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let Ok(read) = rx.await else {
            return;
        };
        cx.update(|cx| {
            let _ = handle.update(cx, |_, window, cx| show(read, window, cx));
        });
    })
    .detach();
}

fn toast(cx: &mut App, kind: ToastKind, text: Rc<dyn Fn(&App) -> SharedString>) {
    ToastQueue::push(cx, Toast::new(kind, text));
}

/// Shows the dialog over `read`'s services.
fn show(read: ReadOut, window: &mut Window, cx: &mut App) {
    if read.services.is_empty() {
        let (kind, id) = if read.unreadable {
            (ToastKind::Error, ids::hv::CONN_IMPORT_UNREADABLE)
        } else {
            (ToastKind::Info, ids::hv::CONN_IMPORT_EMPTY)
        };
        toast(cx, kind, text_fn(id));
        return;
    }
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    if ws.read(cx).layer_ids().contains(&IMPORT_LAYER) {
        return;
    }
    let ReadOut {
        services, ignored, ..
    } = read;
    let view = cx.new(|cx| ImportView::new(services, ignored, cx));
    // Subscribe before the push; the event's handle keeps the view alive until it is
    // delivered (the `open_dialog` pattern).
    let ws_for_close = ws.downgrade();
    window
        .subscribe(&view, cx, move |view, _: &ImportEvent, window, cx| {
            if let Some(ws) = ws_for_close.upgrade()
                && ws.read(cx).layer_ids().last() == Some(&IMPORT_LAYER)
            {
                ws.update(cx, |ws, cx| ws.pop_layer(window, cx));
            }
            cx.defer(move |_| drop(view));
        })
        .detach();
    let first = view.read(cx).first_focus(cx);
    ws.update(cx, |ws, cx| {
        ws.push_layer(
            Layer {
                id: IMPORT_LAYER,
                context: DIALOG_CONTEXT,
                view: view.clone().into(),
                restore_focus: None,
                modal: true,
            },
            window,
            cx,
        );
    });
    window.focus(&first, cx);
    cx.set_global(OpenImport(view.downgrade()));
}

impl ImportView {
    fn new(services: Vec<Service>, ignored: Vec<String>, cx: &mut Context<Self>) -> Self {
        let existing: Vec<String> = cx
            .try_global::<ConnectionsModel>()
            .map(|m| m.profiles().iter().map(|p| p.name.clone()).collect())
            .unwrap_or_default();
        let mut taken = existing;
        let mut rows = Vec::new();
        let mut subscriptions = Vec::new();
        for service in services {
            let name = if taken.iter().any(|t| t.eq_ignore_ascii_case(&service.name)) {
                let names: Vec<&str> = taken.iter().map(String::as_str).collect();
                duplicate_name(&service.name, &names)
            } else {
                service.name.clone()
            };
            taken.push(name.clone());
            let (profile, tls_key_passphrase) =
                profile_of(name, service.fields, service.password.is_some());
            let env = preselect(&profile);
            let selected = CHOICES.iter().position(|c| *c == env).unwrap_or(0);
            let options = CHOICES
                .iter()
                .map(|c| SelectOption {
                    label: choice_label(*c),
                    hint: None,
                    disabled: false,
                })
                .collect();
            let select = cx.new(|cx| Select::new(options, selected, cx));
            let index = rows.len();
            subscriptions.push(
                cx.subscribe(&select, move |this, _, event: &SelectEvent, cx| {
                    let SelectEvent::Changed(choice) = *event;
                    if let Some(row) = this.rows.get_mut(index) {
                        row.env = CHOICES.get(choice).copied().flatten();
                    }
                    cx.notify();
                }),
            );
            rows.push(Row {
                profile,
                password: service.password,
                tls_key_passphrase,
                env,
                checked: true,
                select,
                check_focus: cx.focus_handle().tab_stop(true),
            });
        }
        ImportView {
            rows,
            ignored,
            cancel_focus: cx.focus_handle().tab_stop(true),
            confirm_focus: cx.focus_handle().tab_stop(true),
            _subscriptions: subscriptions,
        }
    }

    /// The rows' names, top down.
    pub fn row_names(&self) -> Vec<String> {
        self.rows.iter().map(|r| r.profile.name.clone()).collect()
    }

    /// Row `index`'s environment.
    pub fn env(&self, index: usize) -> Option<EnvTag> {
        self.rows.get(index).and_then(|r| r.env)
    }

    /// Picks row `index`'s environment (as its Select does).
    pub fn set_env(&mut self, index: usize, env: Option<EnvTag>, cx: &mut Context<Self>) {
        let choice = CHOICES.iter().position(|c| *c == env).unwrap_or(0);
        if let Some(row) = self.rows.get_mut(index) {
            row.env = env;
            row.select.update(cx, |s, cx| s.set_selected(choice, cx));
        }
        cx.notify();
    }

    /// Whether Import can run: something is checked and every checked remote row has an
    /// environment (R24).
    pub fn can_import(&self) -> bool {
        let checked = self.rows.iter().filter(|r| r.checked);
        let mut any = false;
        for row in checked {
            any = true;
            let mut p = row.profile.clone();
            p.environment = row.env;
            if needs_environment_prompt(&p) {
                return false;
            }
        }
        any
    }

    fn first_focus(&self, cx: &App) -> FocusHandle {
        self.rows
            .first()
            .map(|r| r.select.focus_handle(cx))
            .unwrap_or_else(|| self.cancel_focus.clone())
    }

    /// The tab order: each row's checkbox and Select, Cancel, then Import when enabled.
    fn tab_order(&self, cx: &App) -> Vec<FocusHandle> {
        let mut order = Vec::new();
        for row in &self.rows {
            order.push(row.check_focus.clone());
            order.push(row.select.focus_handle(cx));
        }
        order.push(self.cancel_focus.clone());
        if self.can_import() {
            order.push(self.confirm_focus.clone());
        }
        order
    }

    /// A modal form traps Tab like `Dialog` (the wave 4 rule).
    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let m = &stroke.modifiers;
        if stroke.key != "tab" || m.control || m.alt || m.platform || m.function {
            return;
        }
        let order = self.tab_order(cx);
        let at = order.iter().position(|h| h.is_focused(window));
        let n = order.len();
        let next = match (at, m.shift) {
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
            (None, false) => 0,
            (None, true) => n - 1,
        };
        window.focus(&order[next], cx);
        cx.stop_propagation();
    }

    /// Saves every checked row and closes.
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        if !self.can_import() {
            return;
        }
        let mut tasks = Vec::new();
        for row in self.rows.iter_mut().filter(|r| r.checked) {
            let mut profile = row.profile.clone();
            profile.environment = row
                .env
                .or_else(|| is_local_endpoint(&profile.endpoint).then_some(EnvTag::Local));
            let draft = ProfileDraft {
                password: row.password.take(),
                tls_key_passphrase: row.tls_key_passphrase.take(),
                is_new: true,
                ..ProfileDraft::edit(profile)
            };
            tasks.push(ConnectionsModel::save(cx, draft));
        }
        let count = u64::try_from(tasks.len()).unwrap_or(u64::MAX);
        cx.spawn(async move |_, cx| {
            let mut failure = None;
            for task in tasks {
                if let Err(e) = task.await {
                    failure.get_or_insert(e);
                }
            }
            cx.update(|cx| match failure {
                Some(e) => show_save_error(&e, cx),
                None => {
                    let text = Rc::new(move |cx: &App| {
                        tr(
                            cx,
                            ids::hv::CONN_IMPORT_DONE,
                            ids::hv::HvConnImportDone {
                                count: Count(count),
                            },
                        )
                    });
                    toast(cx, ToastKind::Info, text);
                }
            });
        })
        .detach();
        cx.emit(ImportEvent::Closed);
    }

    fn row(&self, index: usize, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let t = theme.tokens;
        let row = &self.rows[index];
        let target = match &row.profile.endpoint {
            ProfileEndpoint::Server { host, .. } if host.is_empty() => {
                row.profile.database.clone().unwrap_or_default()
            }
            _ => endpoint_text(&row.profile),
        };
        let label = tr(
            cx,
            ids::hv::CONN_IMPORT_ROW,
            ids::hv::HvConnImportRow {
                service: Text(row.profile.name.clone()),
                target: Text(target),
            },
        );
        let weak = cx.entity().downgrade();
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(t.spacing.lg))
            .min_h(px(30.0))
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    Checkbox::new(("import-row", index), label)
                        .checked(row.checked)
                        .focus_handle(row.check_focus.clone())
                        .on_toggle(move |checked, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                if let Some(row) = this.rows.get_mut(index) {
                                    row.checked = checked;
                                }
                                cx.notify();
                            });
                        }),
                ),
            )
            .child(div().w(px(150.0)).flex_none().child(row.select.clone()))
            .into_any_element()
    }
}

impl Render for ImportView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let mut rows = div().flex().flex_col().gap(px(t.spacing.xs));
        let built: Vec<AnyElement> = (0..self.rows.len())
            .map(|i| self.row(i, &theme, cx))
            .collect();
        rows = rows.children(built);
        let notes: Vec<AnyElement> = self
            .ignored
            .iter()
            .map(|service| {
                Label::new(
                    tr(
                        cx,
                        ids::hv::CONN_IMPORT_PASSWORD_IGNORED,
                        ids::hv::HvConnImportPasswordIgnored {
                            service: Text(service.clone()),
                        },
                    ),
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text_2.to_hsla(),
                )
                .into_any_element()
            })
            .collect();
        let enabled = self.can_import();
        let body = paragraph(
            tr(cx, ids::hv::CONN_IMPORT_BODY, NoArgs),
            t.type_ramp.body_sm,
            c.text_2.to_hsla(),
            "import-body",
        );
        let cancel = Button::new("import-cancel", tr(cx, ids::hv::CONN_IMPORT_CANCEL, NoArgs))
            .focus_handle(self.cancel_focus.clone())
            .on_click(cx.listener(|_, _, _, cx| cx.emit(ImportEvent::Closed)));
        let confirm = Button::new(
            "import-confirm",
            tr(cx, ids::hv::CONN_IMPORT_CONFIRM, NoArgs),
        )
        .kind(ButtonKind::Primary)
        .disabled(!enabled)
        .focus_handle(self.confirm_focus.clone())
        .on_click(cx.listener(|this, _, _, cx| this.confirm(cx)));
        dialog_frame(&theme, DialogSize::Wide)
            .id("import-dialog")
            .debug_selector(|| "import-dialog".to_string())
            .on_key_down(cx.listener(Self::on_key))
            .child(
                Label::new(
                    tr(cx, ids::hv::CONN_IMPORT_TITLE, NoArgs),
                    TextSource::Fluent,
                    TypeStyle {
                        weight: 600,
                        ..t.type_ramp.body
                    },
                    c.text.to_hsla(),
                )
                .id("import-title"),
            )
            .child(body)
            .child(rows)
            .children(notes)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(t.spacing.md))
                    .child(cancel)
                    .child(confirm),
            )
    }
}

// ## m2a-t18-explorer: shots fixture (R28, P19)

/// The import dialog over two services, one remote (Production preselected) and one local.
#[cfg(feature = "shots")]
pub fn pg_service_fixture(window: &mut Window, cx: &mut App) {
    let services = vec![
        Service {
            name: "orders-prod".to_string(),
            fields: vec![
                ("host".to_string(), "db-prod.acme.io".to_string()),
                ("dbname".to_string(), "orders".to_string()),
            ],
            password: None,
        },
        Service {
            name: "local".to_string(),
            fields: vec![("host".to_string(), "localhost".to_string())],
            password: None,
        },
    ];
    show(
        ReadOut {
            services,
            ignored: Vec::new(),
            unreadable: false,
        },
        window,
        cx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_never_stay_in_extra_and_sslpassword_is_the_key_passphrase() {
        use secrecy::ExposeSecret;
        let (p, passphrase) = profile_of(
            "orders".to_string(),
            vec![
                ("host".into(), "db".into()),
                ("SSLPassword".into(), "s3cret".into()),
                ("sslpassword".into(), "second".into()),
                ("PGPASSWORD".into(), "hv_test_pw".into()),
                ("db_password".into(), "hv_test_pw".into()),
                ("--pwd".into(), "hv_test_pw".into()),
                ("sslmode".into(), "require".into()),
                ("password_db".into(), "kept".into()),
            ],
            false,
        );
        assert_eq!(
            passphrase
                .as_ref()
                .map(|s| s.expose_secret().to_string())
                .as_deref(),
            Some("s3cret")
        );
        assert_eq!(
            p.extra,
            vec![
                ("sslmode".to_string(), "require".to_string()),
                ("password_db".to_string(), "kept".to_string())
            ]
        );
        let text = format!("{p:?}");
        assert!(!text.contains("s3cret") && !text.contains("hv_test_pw"));
    }

    #[test]
    fn a_service_maps_to_an_imported_profile_with_first_values() {
        let (p, _) = profile_of(
            "orders".to_string(),
            vec![
                ("host".into(), "db-prod".into()),
                ("host".into(), "other".into()),
                ("port".into(), "6432".into()),
                ("dbname".into(), "orders".into()),
                ("sslmode".into(), "require".into()),
            ],
            false,
        );
        assert_eq!(
            p.endpoint,
            ProfileEndpoint::Server {
                host: "db-prod".into(),
                port: Some(6432)
            }
        );
        assert_eq!(p.database.as_deref(), Some("orders"));
        assert_eq!(
            p.extra,
            vec![("sslmode".to_string(), "require".to_string())]
        );
        assert_eq!(
            p.origin,
            Origin::Imported {
                source: ImportSource::PgService
            }
        );
        assert_eq!(p.auth, AuthProviderId::PgPass);
        let (with_password, _) = profile_of("x".into(), vec![], true);
        assert_eq!(
            (with_password.auth, with_password.retention),
            (AuthProviderId::Password, Retention::Forever)
        );
    }

    #[test]
    fn local_and_production_rows_are_preselected() {
        let (local, _) = profile_of("a".into(), vec![("host".into(), "localhost".into())], false);
        assert_eq!(preselect(&local), Some(EnvTag::Local));
        let (socket, _) = profile_of("a".into(), vec![("host".into(), "/tmp".into())], false);
        assert_eq!(preselect(&socket), Some(EnvTag::Local));
        let (prod, _) = profile_of("a".into(), vec![("host".into(), "db-prod".into())], false);
        assert_eq!(preselect(&prod), Some(EnvTag::Production));
        let (named, _) = profile_of(
            "orders-live".into(),
            vec![("host".into(), "10.0.0.5".into())],
            false,
        );
        assert_eq!(preselect(&named), Some(EnvTag::Production));
        let (other, _) = profile_of(
            "delivery".into(),
            vec![("host".into(), "10.0.0.5".into())],
            false,
        );
        assert_eq!(preselect(&other), None);
    }

    #[test]
    fn the_first_section_of_a_name_wins_across_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let user = dir.join("user.conf");
        let system = dir.join("system.conf");
        std::fs::write(&user, "[a]\nhost=first\n[a]\nhost=second\n").unwrap();
        std::fs::write(&system, "[a]\nhost=system\n[b]\nhost=b\n").unwrap();
        let read = read_services(Some(vec![user, dir.join("missing.conf"), system]));
        let names: Vec<(&str, &str)> = read
            .services
            .iter()
            .map(|s| (s.name.as_str(), s.fields[0].1.as_str()))
            .collect();
        assert_eq!(names, [("a", "first"), ("b", "b")]);
        assert!(!read.unreadable);
    }
}
