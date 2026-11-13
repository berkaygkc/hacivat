//! The connection dialog (NOTES surfaces 1 and 2): the form, the URL row, paste and copy
//! (Task 17).
//!
//! **The layer.** [`open_connection_dialog`] pushes a modal layer `connection-dialog` (context
//! `ConnectionDialog`, a 640 px form card). The workspace subscribes to the dialog before the
//! push and keeps a strong clone until the close event is delivered (the `open_dialog`
//! pattern). The dialog traps Tab and Shift-Tab (focus wraps inside it) and handles Escape
//! itself, from the card or from any field's `InputEvent::Cancelled` (P16): a running test stops
//! first, otherwise the layer closes.
//!
//! **The draft.** The dialog edits a [`ProfileDraft`]. Typed fields write it on every change and
//! mark the field as touched (a pasted URL never overwrites a touched field). A masked field's
//! text becomes a `SecretString` at once. Programmatic changes (paste, undo, a quick fix) write
//! the fields back.
//!
//! **Extra properties (M2a final review I1, I2).** A row whose key owns a secret slot
//! (`password`, `pgpassword`, `mysql_pwd`, ...; `sslpassword` for the key passphrase) never
//! reaches the draft's options: when the row is left (its fields lose the focus) or before a
//! save or a test, its value moves into the masked password (or passphrase) slot and the row
//! goes. Any other password-like key (`db_password`, `x-pwd`) stays out of the options too and
//! is refused with `IssueCode::OptionSecret`, which blocks a save. A row with a typed setting
//! (`sslmode`, `sslrootcert`, `connect_timeout`, ...) is folded into that setting the same way,
//! so the form, the fixes and the connect agree.
//!
//! **Threading (§5.4).** Validation (it reads TLS files), the keyring probe (P36) and the pgpass
//! lookup run on the core runtime; the dialog awaits only `oneshot` channels.

pub mod form;
pub mod paste;
pub mod url_row;

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use hacivat_driver::EngineCatalog;
use hacivat_driver::url::FieldId;
use hacivat_driver::url::native::{OwnerSlot, secret_owner_slot};
use hacivat_l10n::{Text, ids};
use hacivat_net::TlsMode;
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_secrets::{AuthProviderId, Retention};
use hacivat_session::pipeline::{ApprovedRoots, IssueCode, ValidationIssue, validate_with};
use hacivat_session::profile::{
    Profile, ProfileDraft, ProfileEndpoint, fold_typed_options, is_local_endpoint,
    needs_environment_prompt, production_hint, suggest_name,
};
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::select::{Select, SelectEvent};
use hacivat_ui::text::{TextFn, TextSource};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Global,
    IntoElement, KeyDownEvent, PathPromptOptions, Render, SharedString, Subscription, WeakEntity,
    Window,
};
use secrecy::SecretString;
use tokio::sync::oneshot;

use crate::connections::fixes::FixAction;
use crate::connections::model::{ConnectionsModel, SaveError};
use crate::connections::prod_prompt::ask_environment;
use crate::connections::test_strip::{TestStrip, TestStripEvent, TestStripState};
use crate::l10n_global::{text_fn, tr};
use crate::shell::{Layer, Workspace};

use form::{Field, PgPassState};
use paste::{Notice, Tint};

/// The layer ID of the connection dialog.
pub const DIALOG_LAYER_ID: &str = "connection-dialog";
/// The key context of the connection dialog (its keymap bindings, P47).
pub const DIALOG_CONTEXT: &str = "ConnectionDialog";
/// Where "Supported versions" points (R11; a placeholder URL, an open item for the PO).
pub const SUPPORTED_VERSIONS_URL: &str = "https://hacivat.dev/docs/engines";

/// What the dialog opens on (NOTES surface 1). `Debug` never shows the pasted text: it may
/// carry a password.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum DialogMode {
    New,
    /// A new profile prefilled from pasted connection text.
    NewFromText(String),
    Edit(ProfileId),
}

impl std::fmt::Debug for DialogMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DialogMode::New => f.write_str("New"),
            DialogMode::NewFromText(_) => f.write_str("NewFromText(<redacted>)"),
            DialogMode::Edit(id) => f.debug_tuple("Edit").field(id).finish(),
        }
    }
}

/// What the dialog emits: it wants its layer closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionDialogEvent {
    Closed,
}

/// What the dialog needs from the boot (T16 ruling A): the approved roots a picked file joins
/// (the same list `ConnectDeps` checks), and the pgpass file connects read. `boot::install`
/// sets it. Without it (a shell booted without connections) a picked file is approved nowhere,
/// a save validates with the user's home as the only root, and no pgpass line is looked up.
pub struct DialogEnv {
    pub approved_roots: ApprovedRoots,
    pub pgpass: Option<PathBuf>,
}

impl Global for DialogEnv {}

/// The open connection dialog of each window (by window ID), for its commands.
#[derive(Default)]
struct OpenDialogs(HashMap<u64, WeakEntity<ConnectionDialog>>);

impl Global for OpenDialogs {}

fn window_key(window: &Window) -> u64 {
    window.window_handle().window_id().as_u64()
}

/// Why the last save did not land, shown at the top of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SaveProblem {
    StoreReadOnly,
    Failed,
    Gone,
    SecretNotStored,
}

/// The focus targets of the dialog that are not text fields (tests name them, P16 keyboard
/// path). Segmented groups and disclosures are reached through a wrapper that is not a tab stop.
pub(crate) struct Focus {
    pub(crate) root: FocusHandle,
    pub(crate) engine: FocusHandle,
    pub(crate) environment: FocusHandle,
    pub(crate) auth: FocusHandle,
    pub(crate) tls: FocusHandle,
    pub(crate) advanced: FocusHandle,
    pub(crate) read_only: FocusHandle,
    pub(crate) sqlite_ro: FocusHandle,
    pub(crate) sqlite_create: FocusHandle,
    pub(crate) copy_url: FocusHandle,
    pub(crate) choose_file: FocusHandle,
    pub(crate) choose_ca: FocusHandle,
    pub(crate) choose_cert: FocusHandle,
    pub(crate) choose_key: FocusHandle,
    pub(crate) add_extra: FocusHandle,
    pub(crate) notice_primary: FocusHandle,
    pub(crate) notice_secondary: FocusHandle,
    pub(crate) test: FocusHandle,
    pub(crate) cancel: FocusHandle,
    pub(crate) save: FocusHandle,
    pub(crate) save_connect: FocusHandle,
}

impl Focus {
    fn new(cx: &mut App) -> Self {
        let stop = |cx: &mut App| cx.focus_handle().tab_stop(true);
        let wrapper = |cx: &mut App| cx.focus_handle();
        Focus {
            root: cx.focus_handle(),
            engine: wrapper(cx),
            environment: wrapper(cx),
            auth: wrapper(cx),
            tls: wrapper(cx),
            advanced: wrapper(cx),
            read_only: stop(cx),
            sqlite_ro: stop(cx),
            sqlite_create: stop(cx),
            // The mock keeps the URL row's copy button out of the tab order (⌘⇧C reaches it).
            copy_url: cx.focus_handle(),
            choose_file: stop(cx),
            choose_ca: stop(cx),
            choose_cert: stop(cx),
            choose_key: stop(cx),
            add_extra: stop(cx),
            notice_primary: stop(cx),
            notice_secondary: stop(cx),
            test: stop(cx),
            cancel: stop(cx),
            save: stop(cx),
            save_connect: stop(cx),
        }
    }
}

/// One extra property row (Advanced). Its subscriptions go with it.
pub(crate) struct ExtraRow {
    pub(crate) key: Entity<TextInput>,
    pub(crate) value: Entity<TextInput>,
    pub(crate) remove: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// The connection dialog (NOTES surfaces 1 and 2).
pub struct ConnectionDialog {
    pub(crate) is_new: bool,
    /// The saved name the edit title shows.
    pub(crate) title_name: String,
    pub(crate) draft: ProfileDraft,
    /// The user picked the environment (or the profile has one): no automatic local tag.
    pub(crate) env_touched: bool,
    /// The profile keeps its password in the keyring (an edit; the keyring is not read).
    pub(crate) has_stored_password: bool,
    pub(crate) password_from_url: bool,
    pub(crate) inputs: HashMap<Field, Entity<TextInput>>,
    pub(crate) extras: Vec<ExtraRow>,
    pub(crate) remember: Entity<Select>,
    pub(crate) tls_mode: Entity<Select>,
    pub(crate) keepalive: Entity<Select>,
    // ## m2a-t21-shots
    /// The Environment control as a Select, drawn instead of the segmented control when the
    /// row cannot show every environment's label whole (a long locale).
    pub(crate) env_select: Entity<Select>,
    /// Whether the last frame drew `env_select` (tests).
    pub(crate) env_as_select: std::cell::Cell<bool>,
    pub(crate) strip: Entity<TestStrip>,
    pub(crate) open_tls: bool,
    pub(crate) open_adv: bool,
    pub(crate) notice: Option<Notice>,
    pub(crate) undo: Option<paste::Snapshot>,
    pub(crate) tint: Option<Tint>,
    pub(crate) issues: Vec<ValidationIssue>,
    pub(crate) problem: Option<SaveProblem>,
    pub(crate) keyring_available: bool,
    pub(crate) pgpass: Option<PgPassState>,
    pub(crate) saving: bool,
    /// `connections.toml` cannot be written: fields are read-only, Save is disabled.
    pub(crate) read_only: bool,
    pub(crate) focus: Focus,
    subscriptions: Vec<Subscription>,
}

impl EventEmitter<ConnectionDialogEvent> for ConnectionDialog {}

impl Focusable for ConnectionDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.root.clone()
    }
}

/// The connection dialog open in `window`, if one is.
pub fn open_dialog(window: &Window, cx: &App) -> Option<Entity<ConnectionDialog>> {
    cx.try_global::<OpenDialogs>()
        .and_then(|g| g.0.get(&window_key(window)))
        .and_then(WeakEntity::upgrade)
}

/// Whether a connection dialog is open (the `enabled` of its commands; `run` then finds the
/// one of the window the command runs in).
fn dialog_is_open(cx: &App) -> bool {
    cx.try_global::<OpenDialogs>()
        .is_some_and(|g| g.0.values().any(|d| d.upgrade().is_some()))
}

/// Opens the connection dialog as a modal layer (NOTES surface 1). Needs the connections model
/// (after `connections::boot`) and the workspace as the window's root; a second call while one
/// is open focuses the open one.
pub fn open_connection_dialog(mode: DialogMode, window: &mut Window, cx: &mut App) {
    if !cx.has_global::<ConnectionsModel>() {
        return;
    }
    let Some(Some(workspace)) = window.root::<Workspace>() else {
        return;
    };
    if let Some(open) = open_dialog(window, cx) {
        let host = open.read(cx).initial_focus(cx);
        window.focus(&host, cx);
        return;
    }
    let text = match &mode {
        DialogMode::NewFromText(text) => Some(text.clone()),
        _ => None,
    };
    let dialog = cx.new(|cx| ConnectionDialog::new(&mode, window, cx));
    workspace.update(cx, |ws, cx| {
        // Subscribed before the push; the strong clone lives until the event was delivered.
        cx.subscribe_in(
            &dialog,
            window,
            |ws, dialog, _: &ConnectionDialogEvent, window, cx| {
                let keep = dialog.clone();
                // By its view: another layer (the environment prompt) may sit on top.
                ws.remove_layer(dialog.entity_id(), window, cx);
                cx.defer(move |_| drop(keep));
            },
        )
        .detach();
        ws.push_layer(
            Layer {
                id: DIALOG_LAYER_ID,
                context: DIALOG_CONTEXT,
                view: dialog.clone().into(),
                restore_focus: None,
                modal: true,
            },
            window,
            cx,
        );
    });
    let dialogs = cx.default_global::<OpenDialogs>();
    dialogs.0.retain(|_, d| d.upgrade().is_some());
    dialogs.0.insert(window_key(window), dialog.downgrade());
    let first = dialog.read(cx).initial_focus(cx);
    window.focus(&first, cx);
    dialog.update(cx, |d, cx| {
        d.probe_keyring(cx);
        if let Some(text) = text {
            d.apply_text(&text, window, cx);
        }
    });
}

/// Registers the `connection_dialog.*` commands, enabled while a dialog is open (P41).
pub fn install(cx: &mut App) {
    type Run = fn(&mut ConnectionDialog, &mut Window, &mut Context<ConnectionDialog>);
    let defs: [(&'static str, &'static str, Run); 4] = [
        (
            "connection_dialog.test",
            ids::hv::CMD_CONNECTION_DIALOG_TEST.id,
            |d, window, cx| d.toggle_test(window, cx),
        ),
        (
            "connection_dialog.save",
            ids::hv::CMD_CONNECTION_DIALOG_SAVE.id,
            |d, window, cx| d.save(false, window, cx),
        ),
        (
            "connection_dialog.save_and_connect",
            ids::hv::CMD_CONNECTION_DIALOG_SAVE_AND_CONNECT.id,
            |d, window, cx| d.save(true, window, cx),
        ),
        (
            "connection_dialog.copy_url",
            ids::hv::CMD_CONNECTION_DIALOG_COPY_URL.id,
            |d, _, cx| d.copy_as_url(cx),
        ),
    ];
    let reg = cx.default_global::<ActionRegistry>();
    for (id, title_id, run) in defs {
        // A duplicate keeps the first definition.
        let _ = reg.register(ActionDef {
            id,
            title_id,
            context: DIALOG_CONTEXT,
            run: Rc::new(move |window, cx| {
                if let Some(dialog) = open_dialog(window, cx) {
                    dialog.update(cx, |d, cx| run(d, window, cx));
                }
            }),
            enabled: dialog_is_open,
        });
    }
}

/// The retention choices, in the Remember select's order.
pub(crate) const RETENTIONS: [Retention; 4] = Retention::ALL;
/// The TLS modes, in the mode select's order (the mock's `TLSM`).
pub(crate) const TLS_MODES: [TlsMode; 5] = [
    TlsMode::Auto,
    TlsMode::Require,
    TlsMode::VerifyCa,
    TlsMode::VerifyFull,
    TlsMode::Disable,
];
/// The keep-alive choices: off, then every 30, 60 or 300 s.
pub(crate) const KEEPALIVES: [Option<u32>; 4] = [None, Some(30), Some(60), Some(300)];

/// The engines the form offers, in its order.
pub(crate) const ENGINES: [DbmsKind; 4] = [
    DbmsKind::Postgres,
    DbmsKind::MySql,
    DbmsKind::MariaDb,
    DbmsKind::Sqlite,
];

/// MySQL and MariaDB: no VerifyCa in M2a (P11), plaintext under Auto only to loopback.
pub(crate) fn is_mysql_family(kind: DbmsKind) -> bool {
    matches!(kind, DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb)
}

impl ConnectionDialog {
    fn new(mode: &DialogMode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = match mode {
            DialogMode::Edit(id) => cx.try_global::<ConnectionsModel>().and_then(|m| m.get(*id)),
            _ => None,
        };
        let (draft, is_new) = match saved {
            Some(p) => (ProfileDraft::edit((*p).clone()), false),
            None => (ProfileDraft::new(), true),
        };
        let p = &draft.profile;
        let has_stored_password =
            !is_new && p.auth == AuthProviderId::Password && p.retention == Retention::Forever;
        let env_touched = p.environment.is_some();
        let title_name = p.name.clone();
        let retention = p.retention;
        let tls = p.tls.mode;
        let keepalive = p.keepalive_s;
        let engine = p.engine;

        let me = cx.weak_entity();
        let strip = cx.new(TestStrip::new);
        let remember = cx.new(|cx| {
            let selected = RETENTIONS.iter().position(|r| *r == retention).unwrap_or(3);
            let mut s = Select::new(form::retention_options(true), selected, cx);
            s.set_label(text_fn(ids::hv::CONN_DLG_REMEMBER));
            s
        });
        let tls_mode = cx.new(|cx| {
            let selected = TLS_MODES.iter().position(|m| *m == tls).unwrap_or(0);
            let mut s = Select::new(form::tls_options(engine), selected, cx);
            s.set_label(text_fn(ids::hv::CONN_TLS_MODE));
            s
        });
        let keepalive = cx.new(|cx| {
            let selected = KEEPALIVES.iter().position(|k| *k == keepalive).unwrap_or(0);
            let mut s = Select::new(form::keepalive_options(), selected, cx);
            s.set_label(text_fn(ids::hv::CONN_ADV_KEEPALIVE));
            s
        });
        let environment = p.environment;
        let env_select = cx.new(|cx| {
            let mut s = Select::new(form::env_options(), form::env_index(environment), cx);
            s.set_label(text_fn(ids::hv::CONN_DLG_ENV));
            s
        });

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &strip,
            window,
            |this, _, event: &TestStripEvent, window, cx| {
                if let TestStripEvent::Fix(fix) = event {
                    this.apply_fix(fix.clone(), window, cx);
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &remember,
            window,
            |this, _, SelectEvent::Changed(i): &SelectEvent, window, cx| {
                if let Some(r) = RETENTIONS.get(*i) {
                    this.draft.profile.retention = *r;
                    this.after_edit(window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &tls_mode,
            window,
            |this, _, SelectEvent::Changed(i): &SelectEvent, window, cx| {
                if let Some(m) = TLS_MODES.get(*i) {
                    this.draft.profile.tls.mode = *m;
                    this.after_edit(window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &env_select,
            window,
            |this, _, SelectEvent::Changed(i): &SelectEvent, window, cx| {
                if let Some(tag) = i.checked_sub(1).and_then(|i| EnvTag::ALL.get(i)) {
                    this.set_environment(*tag, window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &keepalive,
            window,
            |this, _, SelectEvent::Changed(i): &SelectEvent, window, cx| {
                if let Some(k) = KEEPALIVES.get(*i) {
                    this.draft.profile.keepalive_s = *k;
                    this.after_edit(window, cx);
                }
            },
        ));

        let mut inputs = HashMap::new();
        for field in Field::ALL {
            let input = form::new_input(field, me.clone(), cx);
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, _, event: &InputEvent, window, cx| {
                    this.on_input(field, *event, window, cx)
                },
            ));
            inputs.insert(field, input);
        }
        // Machine text is mono (the mock's `.in.mono`); a store that cannot be written makes
        // every field read-only, and the form says why from the start.
        let read_only = cx
            .try_global::<ConnectionsModel>()
            .is_some_and(|m| m.store_read_only());
        for (field, input) in &inputs {
            let mono = field.mono();
            input.update(cx, |i, _| {
                i.set_mono(mono);
                i.set_read_only(read_only);
            });
        }
        // The password placeholder is a message only while a stored password stands behind it.
        if has_stored_password {
            inputs[&Field::Password]
                .update(cx, |i, _| i.set_placeholder_source(TextSource::Fluent));
        }
        // The URL row applies its text when it loses the focus with a change (mock `change`).
        let url_focus = inputs[&Field::Url].read(cx).focus_handle(cx);
        subscriptions.push(cx.on_blur(&url_focus, window, |this, window, cx| {
            this.apply_url_row(window, cx)
        }));

        let mut dialog = ConnectionDialog {
            is_new,
            title_name,
            draft,
            env_touched,
            has_stored_password,
            password_from_url: false,
            inputs,
            extras: Vec::new(),
            remember,
            tls_mode,
            keepalive,
            env_select,
            env_as_select: std::cell::Cell::new(false),
            strip,
            open_tls: false,
            open_adv: false,
            notice: None,
            undo: None,
            tint: None,
            issues: Vec::new(),
            problem: read_only.then_some(SaveProblem::StoreReadOnly),
            keyring_available: true,
            pgpass: None,
            read_only,
            saving: false,
            focus: Focus::new(cx),
            subscriptions,
        };
        let extras = dialog.draft.profile.extra.clone();
        for (k, v) in extras {
            dialog.push_extra_row(&k, &v, window, cx);
        }
        dialog.write_fields(cx);
        dialog.sync_url(window, cx);
        dialog.refresh_pgpass(cx);
        dialog
    }

    /// The field that takes the initial focus: Host (File for SQLite).
    fn initial_focus(&self, cx: &App) -> FocusHandle {
        let field = if self.draft.profile.engine == DbmsKind::Sqlite {
            Field::File
        } else {
            Field::Host
        };
        self.inputs[&field].read(cx).focus_handle(cx)
    }

    // ---- reading (tests and the shots harness read the dialog through these) -------------------

    /// The profile being edited.
    pub fn profile(&self) -> &Profile {
        &self.draft.profile
    }

    /// The value of the text field `name` (`url`, `host`, `port`, `password`, …).
    pub fn field_value(&self, name: &str, cx: &App) -> Option<String> {
        let field = Field::from_name(name)?;
        Some(self.inputs[&field].read(cx).value().to_string())
    }

    /// The text field `name`.
    pub fn input(&self, name: &str) -> Option<Entity<TextInput>> {
        Field::from_name(name).map(|f| self.inputs[&f].clone())
    }

    /// Whether field `name` still shows the paste tint.
    pub fn is_tinted(&self, name: &str) -> bool {
        Field::from_name(name)
            .is_some_and(|f| self.tint.as_ref().is_some_and(|t| t.fields.contains(&f)))
    }

    /// Whether a save is in flight (validating, asking for an environment, or writing).
    pub fn is_saving(&self) -> bool {
        self.saving
    }

    /// The TLS mode select.
    pub fn tls_select(&self) -> Entity<Select> {
        self.tls_mode.clone()
    }

    /// How many subscriptions the dialog and its extra rows hold.
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
            + self
                .extras
                .iter()
                .map(|r| r._subscriptions.len())
                .sum::<usize>()
    }

    /// The extra properties' key inputs, in order.
    pub fn extra_keys(&self) -> Vec<Entity<TextInput>> {
        self.extras.iter().map(|r| r.key.clone()).collect()
    }

    // ## m2a-finalfix
    /// The extra properties' value inputs, in order.
    pub fn extra_values(&self) -> Vec<Entity<TextInput>> {
        self.extras.iter().map(|r| r.value.clone()).collect()
    }

    /// The issues the form shows now.
    pub fn issues(&self) -> &[ValidationIssue] {
        &self.issues
    }

    /// The Remember select.
    pub fn remember_select(&self) -> Entity<Select> {
        self.remember.clone()
    }

    /// The test strip (Task 20 fills it).
    // ## m2a-t21-shots
    /// Whether the last frame drew the Environment control as a Select (a long locale).
    pub fn environment_is_select(&self) -> bool {
        self.env_as_select.get()
    }

    /// The Environment Select (drawn only when [`Self::environment_is_select`]).
    pub fn env_select(&self) -> Entity<Select> {
        self.env_select.clone()
    }

    pub fn strip(&self) -> Entity<TestStrip> {
        self.strip.clone()
    }

    /// Whether the TLS mode select offers `mode` disabled (P11).
    pub fn tls_option_disabled(&self, mode: TlsMode) -> bool {
        form::tls_mode_disabled(self.draft.profile.engine, mode)
    }

    /// Whether the Remember select offers `retention` disabled (no keyring, P36).
    pub fn remember_option_disabled(&self, retention: Retention) -> bool {
        retention == Retention::Forever && !self.keyring_available
    }

    /// The name of the focused element of the dialog (`url`, `engine`, `read-only`, `remember`,
    /// `tls`, `save-connect`, …), if the focus is in the dialog.
    pub fn focused_field(&self, window: &Window, cx: &App) -> Option<&'static str> {
        for field in Field::ALL {
            if self.inputs[&field]
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
            {
                return Some(field.name());
            }
        }
        let f = &self.focus;
        let selects = [
            ("remember", self.remember.read(cx).focus_handle(cx)),
            ("tls-mode", self.tls_mode.read(cx).focus_handle(cx)),
            ("keepalive", self.keepalive.read(cx).focus_handle(cx)),
            (
                "environment-select",
                self.env_select.read(cx).focus_handle(cx),
            ),
        ];
        let owned = [
            ("read-only", &f.read_only),
            ("sqlite-ro", &f.sqlite_ro),
            ("sqlite-create", &f.sqlite_create),
            ("copy-url", &f.copy_url),
            ("choose-file", &f.choose_file),
            ("choose-ca", &f.choose_ca),
            ("choose-cert", &f.choose_cert),
            ("choose-key", &f.choose_key),
            ("add-extra", &f.add_extra),
            ("notice-primary", &f.notice_primary),
            ("notice-secondary", &f.notice_secondary),
            ("test", &f.test),
            ("cancel", &f.cancel),
            ("save", &f.save),
            ("save-connect", &f.save_connect),
        ];
        if let Some((name, _)) = selects.iter().find(|(_, h)| h.is_focused(window)) {
            return Some(name);
        }
        if let Some((name, _)) = owned.iter().find(|(_, h)| h.is_focused(window)) {
            return Some(name);
        }
        let groups = [
            ("engine", &f.engine),
            ("environment", &f.environment),
            ("auth", &f.auth),
            ("tls", &f.tls),
            ("advanced", &f.advanced),
        ];
        groups
            .iter()
            .find(|(_, h)| h.contains_focused(window, cx))
            .map(|(name, _)| *name)
    }

    // ---- editing --------------------------------------------------------------------------------

    fn on_input(
        &mut self,
        field: Field,
        event: InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed if field != Field::Url => {
                let value = self.inputs[&field].read(cx).value().to_string();
                self.edit_field(field, value, window, cx);
            }
            InputEvent::Submitted if field == Field::Url => self.apply_url_row(window, cx),
            InputEvent::Cancelled => self.escape(window, cx),
            _ => {}
        }
    }

    /// Writes a typed `value` of `field` into the draft and marks the field as touched.
    pub(crate) fn edit_field(
        &mut self,
        field: Field,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let p = &mut self.draft.profile;
        let touched = match field {
            Field::Url => None,
            Field::Name => {
                p.name = value;
                None
            }
            Field::Host => {
                let port = form::endpoint_port(&p.endpoint);
                if value.starts_with('/') {
                    p.endpoint = ProfileEndpoint::Socket {
                        path: PathBuf::from(value),
                        port,
                    };
                    Some(FieldId::Socket)
                } else {
                    p.endpoint = ProfileEndpoint::Server { host: value, port };
                    Some(FieldId::Host)
                }
            }
            Field::Port => {
                let port = form::parse_port(&value);
                match &mut p.endpoint {
                    ProfileEndpoint::Server { port: slot, .. }
                    | ProfileEndpoint::Socket { port: slot, .. } => *slot = port,
                    ProfileEndpoint::File { .. } => {}
                }
                Some(FieldId::Port)
            }
            Field::Database => {
                p.database = (!value.is_empty()).then_some(value);
                Some(FieldId::Database)
            }
            Field::User => {
                p.user = (!value.is_empty()).then_some(value);
                Some(FieldId::User)
            }
            Field::Password => {
                // The masked field's text becomes a secret at once (R21).
                self.draft.password = (!value.is_empty()).then(|| SecretString::from(value));
                self.password_from_url = false;
                Some(FieldId::Password)
            }
            Field::KeyPass => {
                self.draft.tls_key_passphrase =
                    (!value.is_empty()).then(|| SecretString::from(value));
                None
            }
            Field::File => {
                let create = matches!(
                    p.endpoint,
                    ProfileEndpoint::File {
                        create_if_missing: true,
                        ..
                    }
                );
                p.endpoint = ProfileEndpoint::File {
                    path: PathBuf::from(value),
                    create_if_missing: create,
                };
                Some(FieldId::File)
            }
            Field::Ca => {
                p.tls.ca_file = (!value.trim().is_empty()).then(|| PathBuf::from(value));
                None
            }
            Field::Cert | Field::Key => {
                let cert = self.inputs[&Field::Cert].read(cx).value().to_string();
                let key = self.inputs[&Field::Key].read(cx).value().to_string();
                self.draft.profile.tls.client_cert = form::client_cert(&cert, &key);
                None
            }
            Field::Timeout => {
                if let Ok(s) = value.trim().parse::<u32>() {
                    p.connect_timeout_s = s;
                }
                None
            }
            Field::AppName => {
                p.application_name = value;
                None
            }
        };
        if let Some(id) = touched {
            self.draft.touched.insert(id);
        }
        let issues = std::mem::take(&mut self.issues);
        self.issues = issues
            .into_iter()
            .filter(|i| self.issue_field(i, cx) != Some(field))
            .collect();
        if let Some(tint) = &mut self.tint {
            tint.fields.retain(|f| *f != field);
        }
        self.after_edit(window, cx);
    }

    /// After any change of the draft: the automatic environment, the URL row, the pgpass line.
    pub(crate) fn after_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_env_rule();
        self.sync_env_select(cx);
        self.sync_url(window, cx);
        self.refresh_pgpass(cx);
        if !matches!(self.strip.read(cx).state(), TestStripState::Running(_)) {
            self.problem = None;
        }
        cx.notify();
    }

    // ## m2a-t21-shots
    /// Shows the draft's environment in the Environment Select (the long-locale fallback of
    /// the segmented control), whatever changed it: a pick, the automatic `local` rule, a paste.
    pub(crate) fn sync_env_select(&mut self, cx: &mut Context<Self>) {
        let want = form::env_index(self.draft.profile.environment);
        if self.env_select.read(cx).selected() != want {
            self.env_select.update(cx, |s, cx| s.set_selected(want, cx));
        }
    }

    /// The automatic environment (NOTES surface 1): until the user picks one, a SQLite file, a
    /// socket and a loopback host are `local`, anything else has none (asked at save).
    pub(crate) fn apply_env_rule(&mut self) {
        if self.env_touched {
            return;
        }
        let p = &mut self.draft.profile;
        p.environment = match &p.endpoint {
            _ if p.engine == DbmsKind::Sqlite => Some(EnvTag::Local),
            ProfileEndpoint::Server { host, .. } if host.trim().is_empty() => None,
            e if is_local_endpoint(e) => Some(EnvTag::Local),
            _ => None,
        };
    }

    /// The user picked an environment.
    pub(crate) fn set_environment(
        &mut self,
        env: EnvTag,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.draft.profile.environment = Some(env);
        self.env_touched = true;
        self.after_edit(window, cx);
    }

    /// Switches the form's engine (the Engine segmented control, or an accepted URL switch).
    pub(crate) fn set_engine(
        &mut self,
        kind: DbmsKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let p = &mut self.draft.profile;
        if p.engine == kind {
            return;
        }
        let from = p.engine;
        p.engine = kind;
        if kind == DbmsKind::Sqlite {
            p.max_connections = 1;
            if !matches!(p.endpoint, ProfileEndpoint::File { .. }) {
                p.endpoint = ProfileEndpoint::File {
                    path: PathBuf::new(),
                    create_if_missing: false,
                };
            }
        } else if from == DbmsKind::Sqlite {
            p.max_connections = Profile::default().max_connections;
            p.endpoint = ProfileEndpoint::Server {
                host: String::new(),
                port: None,
            };
        } else if let ProfileEndpoint::Server { port, .. } = &mut p.endpoint {
            // Each engine has its own default port (the mock clears the field).
            *port = None;
        }
        if kind != DbmsKind::Postgres && p.auth == AuthProviderId::PgPass {
            p.auth = AuthProviderId::Password;
        }
        self.issues.clear();
        let options = form::tls_options(kind);
        self.tls_mode.update(cx, |s, cx| s.set_options(options, cx));
        self.write_fields(cx);
        self.after_edit(window, cx);
    }

    /// The sign-in method.
    pub(crate) fn set_auth(
        &mut self,
        auth: AuthProviderId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let leaving_cert = self.draft.profile.auth == AuthProviderId::ClientCert
            && auth != AuthProviderId::ClientCert;
        self.draft.profile.auth = auth;
        if leaving_cert {
            // The client certificate belongs to that sign-in method only.
            self.draft.profile.tls.client_cert = None;
            self.write_fields(cx);
        }
        self.after_edit(window, cx);
    }

    /// Writes every field from the draft (after a paste, an undo or a quick fix). Emits nothing.
    pub(crate) fn write_fields(&mut self, cx: &mut Context<Self>) {
        for field in Field::ALL {
            if field == Field::Url {
                continue;
            }
            let value = form::draft_value(self, field);
            self.inputs[&field].update(cx, |input, cx| {
                if input.value() != value.as_str() {
                    input.set_value(value);
                    cx.notify();
                }
            });
        }
        let p = &self.draft.profile;
        let retention = RETENTIONS
            .iter()
            .position(|r| *r == p.retention)
            .unwrap_or(3);
        let tls = TLS_MODES.iter().position(|m| *m == p.tls.mode).unwrap_or(0);
        let keepalive = KEEPALIVES
            .iter()
            .position(|k| *k == p.keepalive_s)
            .unwrap_or(0);
        self.remember
            .update(cx, |s, cx| s.set_selected(retention, cx));
        self.tls_mode.update(cx, |s, cx| s.set_selected(tls, cx));
        self.keepalive
            .update(cx, |s, cx| s.set_selected(keepalive, cx));
        let env = form::env_index(p.environment);
        self.env_select.update(cx, |s, cx| s.set_selected(env, cx));
    }

    /// The profile a save or a test uses: an empty name takes the suggested one (P30).
    pub(crate) fn profile_to_save(&self) -> Profile {
        let mut p = self.draft.profile.clone();
        if p.name.trim().is_empty() {
            p.name = suggest_name(&p);
        }
        p
    }

    /// A draft of `profile` with its password (when it signs in with one) and the TLS key
    /// passphrase a pasted `sslpassword` gave (kept as `TlsKeyPassphrase`; the field is hidden
    /// until encrypted keys are supported, ruling B).
    fn draft_of(&self, profile: Profile) -> ProfileDraft {
        let password = match profile.auth {
            AuthProviderId::Password => self.draft.password.clone(),
            _ => None,
        };
        let tls_key_passphrase = self.draft.tls_key_passphrase.clone();
        ProfileDraft {
            profile,
            password,
            tls_key_passphrase,
            is_new: self.draft.is_new,
            touched: self.draft.touched.clone(),
        }
    }

    // ---- extras --------------------------------------------------------------------------------

    pub(crate) fn push_extra_row(
        &mut self,
        key: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let read_only = self.read_only;
        let me = cx.weak_entity();
        let make = |placeholder: TextFn, text: &str, cx: &mut Context<Self>| {
            let filter = paste::paste_filter(false, me.clone());
            let input = cx.new(|cx| {
                let mut i = TextInput::new(cx, placeholder);
                i.set_value(text.to_string());
                i.set_mono(true);
                i.set_read_only(read_only);
                // Paste works in every field (R21): a connection URL fills the form.
                i.set_paste_filter(Some(filter));
                i
            });
            let handle = input.read(cx).focus_handle(cx);
            let _ = handle.tab_stop(true);
            input
        };
        let key_input = make(text_fn(ids::hv::CONN_ADV_EXTRA_KEY), key, cx);
        let value_input = make(text_fn(ids::hv::CONN_ADV_EXTRA_VALUE), value, cx);
        let mut subscriptions = Vec::new();
        for input in [&key_input, &value_input] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Changed => this.sync_extras(window, cx),
                    InputEvent::Cancelled => this.escape(window, cx),
                    _ => {}
                },
            ));
            // ## m2a-finalfix: leaving a row moves its secret, or its typed setting (I1, I2).
            let handle = input.read(cx).focus_handle(cx);
            // Blur listeners run while the window draws: the rows change after the frame.
            subscriptions.push(cx.on_blur(&handle, window, |_, window, cx| {
                cx.defer_in(window, |this, window, cx| {
                    this.commit_extra_rows(window, cx)
                });
            }));
        }
        self.extras.push(ExtraRow {
            key: key_input,
            value: value_input,
            remove: cx.focus_handle().tab_stop(true),
            _subscriptions: subscriptions,
        });
    }

    pub(crate) fn remove_extra_row(
        &mut self,
        at: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if at < self.extras.len() {
            self.extras.remove(at);
            self.sync_extras(window, cx);
        }
    }

    /// The draft's extra options from the rows. A row without a key is left out, and so is a
    /// row whose key carries a password: an owner key (`password`, `sslpassword`, ...) waits
    /// to move into its slot ([`Self::commit_extra_rows`]); any other password-like key is an
    /// `OptionSecret` issue (M2a final review I1). A password never enters the draft's options.
    pub(crate) fn sync_extras(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_extras_quietly(cx);
        let refused = self.extra_issues(cx);
        self.issues
            .retain(|i| !matches!(i.code, IssueCode::OptionSecret { .. }));
        self.issues.extend(refused);
        self.after_edit(window, cx);
    }

    // ## m2a-finalfix
    /// The `OptionSecret` issue of every row whose key carries a password and owns no slot
    /// (`db_password`, `x-pwd`): it can never be saved, and it is never moved.
    pub(crate) fn extra_issues(&self, cx: &App) -> Vec<ValidationIssue> {
        let mut issues: Vec<ValidationIssue> = Vec::new();
        for row in &self.extras {
            let key = row.key.read(cx).value().trim().to_string();
            if secret_owner_slot(&key).is_none() && hacivat_proto::is_secret_param(&key) {
                let issue = ValidationIssue {
                    field: None,
                    code: IssueCode::OptionSecret { key },
                };
                if !issues.contains(&issue) {
                    issues.push(issue);
                }
            }
        }
        issues
    }

    /// Settles the extra rows (when a row is left, and before a save or a test): a row with
    /// an owner key and a value moves it into the password (or, for `sslpassword`, the TLS key
    /// passphrase) slot, as a pasted URL's password does, and goes; a row with a typed setting
    /// (`sslmode=require`, `connect_timeout=9`, ...) is folded into it and goes (I1, I2).
    pub(crate) fn commit_extra_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut moved = false;
        let rows = std::mem::take(&mut self.extras);
        for row in rows {
            let key = row.key.read(cx).value().trim().to_string();
            let value = row.value.read(cx).value().to_string();
            match secret_owner_slot(&key) {
                Some(OwnerSlot::Password) if !value.is_empty() => {
                    self.draft.password = Some(SecretString::from(value));
                    self.draft.profile.auth = AuthProviderId::Password;
                    self.draft.touched.insert(FieldId::Password);
                    self.has_stored_password = false;
                    self.password_from_url = false;
                    moved = true;
                }
                Some(OwnerSlot::TlsPassphrase) if !value.is_empty() => {
                    self.draft.tls_key_passphrase = Some(SecretString::from(value));
                    moved = true;
                }
                _ => self.extras.push(row),
            }
        }
        self.sync_extras_quietly(cx);
        let folded = fold_typed_options(&mut self.draft.profile);
        if folded.is_empty() && !moved {
            return;
        }
        self.extras.retain(|row| {
            let key = row.key.read(cx).value().trim().to_string();
            let value = row.value.read(cx).value().to_string();
            !folded.iter().any(|f| f.key == key && f.value == value)
        });
        self.write_fields(cx);
        self.sync_extras(window, cx);
    }

    /// The draft's options from the rows, without the follow-up of an edit.
    fn sync_extras_quietly(&mut self, cx: &App) {
        let mut extra = Vec::new();
        for row in &self.extras {
            let key = row.key.read(cx).value().trim().to_string();
            if key.is_empty()
                || secret_owner_slot(&key).is_some()
                || hacivat_proto::is_secret_param(&key)
            {
                continue;
            }
            extra.push((key, row.value.read(cx).value().to_string()));
        }
        self.draft.profile.extra = extra;
    }

    /// Rebuilds the extra rows from the draft (after a paste or an undo).
    pub(crate) fn rebuild_extra_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.extras.clear();
        for (k, v) in self.draft.profile.extra.clone() {
            self.push_extra_row(&k, &v, window, cx);
        }
    }

    // ---- file pickers --------------------------------------------------------------------------

    /// Opens the platform file picker for `field`; the picked file joins the approved roots (T16
    /// ruling A) and fills the field.
    pub(crate) fn choose_file(
        &mut self,
        field: Field,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |d, window, cx| d.picked(field, path, window, cx));
        })
        .detach();
    }

    /// `path` was picked for `field`.
    pub fn picked(
        &mut self,
        field: Field,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(env) = cx.try_global::<DialogEnv>() {
            env.approved_roots.add(path.clone());
        }
        let text = path.to_string_lossy().into_owned();
        self.inputs[&field].update(cx, |input, cx| {
            input.set_value(text.clone());
            cx.notify();
        });
        self.edit_field(field, text, window, cx);
    }

    // ---- keyring and pgpass (core runtime) -----------------------------------------------------

    /// Asks the core runtime once whether the keyring can store secrets (P36).
    fn probe_keyring(&mut self, cx: &mut Context<Self>) {
        let Some(model) = cx.try_global::<ConnectionsModel>() else {
            return;
        };
        let secrets = model.secrets().clone();
        let (tx, rx) = oneshot::channel();
        model.runtime().spawn_blocking(move || {
            let _ = tx.send(secrets.keyring_available());
        });
        cx.spawn(async move |this, cx| {
            if let Ok(available) = rx.await {
                let _ = this.update(cx, |d, cx| d.set_keyring_available(available, cx));
            }
        })
        .detach();
    }

    /// Without a keyring, "Forever" is disabled with its reason; a new profile falls back to
    /// "Until restart".
    pub(crate) fn set_keyring_available(&mut self, available: bool, cx: &mut Context<Self>) {
        self.keyring_available = available;
        if !available && self.is_new && self.draft.profile.retention == Retention::Forever {
            self.draft.profile.retention = Retention::UntilRestart;
        }
        let options = form::retention_options(available);
        let selected = RETENTIONS
            .iter()
            .position(|r| *r == self.draft.profile.retention)
            .unwrap_or(1);
        self.remember.update(cx, |s, cx| {
            s.set_options(options, cx);
            s.set_selected(selected, cx);
        });
        cx.notify();
    }

    /// Looks the endpoint up in pgpass on the core runtime when pgpass signs in and the endpoint
    /// changed since the last lookup.
    pub(crate) fn refresh_pgpass(&mut self, cx: &mut Context<Self>) {
        let p = &self.draft.profile;
        if p.auth != AuthProviderId::PgPass {
            return;
        }
        let key = form::pgpass_key(p);
        if self.pgpass.as_ref().is_some_and(|s| s.key == key) {
            return;
        }
        self.pgpass = Some(PgPassState {
            key: key.clone(),
            line: None,
        });
        let path = cx.try_global::<DialogEnv>().and_then(|e| e.pgpass.clone());
        let (Some(path), Some(model)) = (path, cx.try_global::<ConnectionsModel>()) else {
            return;
        };
        let (tx, rx) = oneshot::channel();
        let lookup = key.clone();
        model.runtime().spawn_blocking(move || {
            let (host, port, db, user) = &lookup;
            let line =
                hacivat_secrets::files::pgpass_lookup(&path, host, *port, db, user.as_deref())
                    .ok()
                    .flatten()
                    // The matched password is dropped here; only the line number crosses over.
                    .map(|m| m.line);
            let _ = tx.send(line);
        });
        cx.spawn(async move |this, cx| {
            if let Ok(line) = rx.await {
                let _ = this.update(cx, |d, cx| {
                    if let Some(state) = d.pgpass.as_mut().filter(|s| s.key == key) {
                        state.line = line;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    // ---- commands ------------------------------------------------------------------------------

    /// Escape, from the card or a field (P16): stops a running test, else closes the dialog.
    pub(crate) fn escape(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let stopped = self.strip.update(cx, |strip, cx| strip.stop(cx));
        if !stopped {
            cx.emit(ConnectionDialogEvent::Closed);
        }
        cx.notify();
    }

    /// Closes the dialog (Cancel, the close button, a save). A test still running is stopped
    /// first, so it never outlives the dialog (T20 integration).
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.strip.update(cx, |strip, cx| strip.stop(cx));
        cx.emit(ConnectionDialogEvent::Closed);
    }

    /// Test connection (⌘T), or Stop while a test runs.
    pub(crate) fn toggle_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.strip.update(cx, |strip, cx| strip.stop(cx)) {
            cx.notify();
            return;
        }
        self.run_test(window, cx);
    }

    pub(crate) fn run_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // ## m2a-finalfix: the rows' secrets and typed settings first (I1, I2).
        self.commit_extra_rows(window, cx);
        let draft = self.draft_of(self.profile_to_save());
        self.strip.update(cx, |strip, cx| strip.run(draft, cx));
        cx.notify();
    }

    /// A quick fix from the test strip (P40).
    pub(crate) fn apply_fix(
        &mut self,
        fix: FixAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match fix {
            FixAction::EditConnection => {
                let field = self
                    .issues
                    .iter()
                    .find_map(|i| self.issue_field(i, cx))
                    .unwrap_or(Field::Host);
                self.focus_field(field, window, cx);
            }
            FixAction::TryAgain => self.run_test(window, cx),
            FixAction::ReenterPassword => {
                self.draft.password = None;
                self.has_stored_password = false;
                self.inputs[&Field::Password].update(cx, |input, cx| {
                    input.set_value("");
                    input.set_placeholder_source(TextSource::Data);
                    cx.notify();
                });
                self.after_edit(window, cx);
                self.focus_field(Field::Password, window, cx);
            }
            FixAction::UseDatabase(db) => {
                self.draft.profile.database = Some(db);
                self.draft.touched.insert(FieldId::Database);
                self.write_fields(cx);
                self.after_edit(window, cx);
                self.run_test(window, cx);
            }
            FixAction::ConnectAnyway => {
                self.draft.profile.allow_unsupported_server = true;
                self.run_test(window, cx);
            }
            FixAction::EncryptWithoutVerifying => {
                self.draft.profile.tls.mode = TlsMode::Require;
                self.write_fields(cx);
                self.after_edit(window, cx);
                self.run_test(window, cx);
            }
            FixAction::SupportedVersions => cx.open_url(SUPPORTED_VERSIONS_URL),
        }
    }

    /// Focuses `field`, opening its section first.
    pub(crate) fn focus_field(
        &mut self,
        field: Field,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match field {
            Field::Ca => self.open_tls = true,
            Field::Timeout | Field::AppName => self.open_adv = true,
            _ => {}
        }
        let handle = self.inputs[&field].read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Save (⌘S) or Save and connect (⌘↵): validate on the core runtime, ask for an environment
    /// when the host is not on this machine, then save through the model, close and toast.
    pub(crate) fn save(&mut self, connect: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(model) = cx.try_global::<ConnectionsModel>() else {
            return;
        };
        if model.store_read_only() || self.read_only {
            self.problem = Some(SaveProblem::StoreReadOnly);
            cx.notify();
            return;
        }
        let runtime = model.runtime();
        // ## m2a-finalfix: the rows' secrets and typed settings first (I1, I2).
        self.commit_extra_rows(window, cx);
        let roots = cx
            .try_global::<DialogEnv>()
            .map(|e| e.approved_roots.clone())
            .unwrap_or_else(ApprovedRoots::for_user);
        let profile = self.profile_to_save();
        let checked = profile.clone();
        let (tx, rx) = oneshot::channel();
        // `validate` reads the TLS files and stats the SQLite path: never on the UI thread.
        runtime.spawn_blocking(move || {
            let _ = tx.send(validate_with(&checked, EngineCatalog::embedded(), &roots));
        });
        self.saving = true;
        cx.spawn_in(window, async move |this, cx| {
            let checked = rx.await;
            let _ = this.update_in(cx, |d, window, cx| match checked {
                Ok(issues) => d.validated(profile, issues, connect, window, cx),
                // Validation never answered (it panicked, or the runtime is gone): that is a
                // failed save, never "no issues".
                Err(_) => d.validation_lost(cx),
            });
        })
        .detach();
    }

    /// Validation never answered (it panicked, or the core runtime is gone): the save failed.
    pub fn validation_lost(&mut self, cx: &mut Context<Self>) {
        self.saving = false;
        self.problem = Some(SaveProblem::Failed);
        cx.notify();
    }

    fn validated(
        &mut self,
        profile: Profile,
        issues: Vec<ValidationIssue>,
        connect: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut issues = issues;
        if let Some(issue) = form::sqlite_mode_issue(&profile)
            && !issues.contains(&issue)
        {
            issues.push(issue);
        }
        // ## m2a-finalfix: a password-like extra row (never in the options) blocks the save.
        for issue in self.extra_issues(cx) {
            if !issues.contains(&issue) {
                issues.push(issue);
            }
        }
        if !issues.is_empty() {
            self.saving = false;
            self.issues = issues;
            let fields: Vec<_> = self
                .issues
                .iter()
                .filter_map(|i| self.issue_field(i, cx))
                .collect();
            self.open_tls |= fields.contains(&Field::Ca);
            self.open_adv |= fields
                .iter()
                .any(|f| matches!(f, Field::Timeout | Field::AppName));
            cx.notify();
            return;
        }
        self.issues.clear();
        if !needs_environment_prompt(&profile) {
            self.commit(profile, connect, window, cx);
            return;
        }
        let host = form::host_text(&profile);
        let hint = production_hint(&host, &profile.name);
        let answer = ask_environment(profile.name.clone(), host, hint, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let env = answer.await.ok().flatten();
            let _ = this.update_in(cx, |d, window, cx| match env {
                Some(env) => {
                    let mut profile = profile;
                    profile.environment = Some(env);
                    d.draft.profile.environment = Some(env);
                    d.env_touched = true;
                    d.sync_env_select(cx);
                    d.commit(profile, connect, window, cx);
                }
                // Cancelled: the dialog stays open, unchanged.
                None => {
                    d.saving = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn commit(
        &mut self,
        profile: Profile,
        connect: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let draft = self.draft_of(profile);
        let password = draft.password.clone();
        let name = draft.profile.name.clone();
        let saved = ConnectionsModel::save(cx, draft);
        cx.spawn_in(window, async move |this, cx| {
            let result = saved.await;
            let _ = this.update_in(cx, |d, _window, cx| {
                d.saving = false;
                match result {
                    Ok(id) => d.saved(id, name, connect.then_some(password), cx),
                    Err(SaveError::Secret(_)) => d.problem = Some(SaveProblem::SecretNotStored),
                    Err(SaveError::StoreReadOnly) => d.problem = Some(SaveProblem::StoreReadOnly),
                    Err(SaveError::NotFound) => d.problem = Some(SaveProblem::Gone),
                    Err(SaveError::Io(_)) => d.problem = Some(SaveProblem::Failed),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The profile `id` was saved: toast, connect when asked, close.
    fn saved(
        &mut self,
        id: ProfileId,
        name: String,
        connect: Option<Option<SecretString>>,
        cx: &mut Context<Self>,
    ) {
        let text: TextFn = Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::CONN_TOAST_SAVED,
                ids::hv::HvConnToastSaved {
                    name: Text(name.clone()),
                },
            )
        });
        ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
        if let Some(password) = connect {
            ConnectionsModel::connect(cx, id, password);
        }
        self.close(cx);
    }

    /// Copy as URL (⌘⇧C, P31): the URL without the password on the clipboard, and a toast with
    /// the exact URL (data) and, when a password was left out, a line saying so.
    pub(crate) fn copy_as_url(&mut self, cx: &mut Context<Self>) {
        let Ok(copied) = hacivat_session::profile::copy_url(&self.draft.profile) else {
            return;
        };
        cx.write_to_clipboard(hacivat_ui_core::gpui_surface::ClipboardItem::new_string(
            copied.url.clone(),
        ));
        let text: TextFn = if copied.omitted_password {
            text_fn(ids::hv::CONN_TOAST_NOPW)
        } else {
            Rc::new(|_: &App| SharedString::default())
        };
        let mut toast = Toast::new(ToastKind::Info, text);
        toast.title = Some(text_fn(ids::hv::CONN_TOAST_COPIED));
        toast.data = Some(copied.url.into());
        toast.wide = true;
        ToastQueue::push(cx, toast);
    }
}

impl Render for ConnectionDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_dialog(window, cx)
    }
}

/// The handler of a dialog key down: Tab and Shift-Tab move inside the dialog (wrapping),
/// Escape stops a test or closes.
pub(crate) fn on_dialog_key(
    this: &mut ConnectionDialog,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut Context<ConnectionDialog>,
) {
    let stroke = &event.keystroke;
    let m = &stroke.modifiers;
    match stroke.key.as_str() {
        "tab" if !(m.control || m.platform || m.alt || m.function) => {
            if m.shift {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }
            if !this.focus.root.contains_focused(window, cx) {
                // Wrapped out of the dialog: the focus stays trapped inside it.
                let target = if m.shift {
                    this.focus.save_connect.clone()
                } else {
                    this.inputs[&Field::Url].read(cx).focus_handle(cx)
                };
                window.focus(&target, cx);
            }
        }
        "escape" if !m.modified() => this.escape(window, cx),
        _ => return,
    }
    cx.stop_propagation();
}

/// `ConnectionsBoot::for_tests` connections for a shots fixture when the shell started without
/// them (R28, P19).
#[cfg(feature = "shots")]
fn ensure_connections(cx: &mut App) {
    use hacivat_driver::DriverRegistry;
    use hacivat_driver::fake::FakeDriver;

    if cx.has_global::<ConnectionsModel>() {
        return;
    }
    let paths = cx.global::<crate::AppState>().paths.clone();
    let mut drivers = DriverRegistry::new();
    for kind in ENGINES {
        drivers.register(std::sync::Arc::new(FakeDriver::new(kind)));
    }
    crate::connections::boot(
        cx,
        crate::connections::ConnectionsBoot::for_tests(&paths, drivers),
    );
}

/// Shots fixture (R28, P19): the new connection dialog.
#[cfg(feature = "shots")]
pub fn new_fixture(window: &mut Window, cx: &mut App) {
    ensure_connections(cx);
    open_connection_dialog(DialogMode::New, window, cx);
}

/// Shots fixture: the dialog right after a `postgres://` URL was pasted (NOTES surface 2).
#[cfg(feature = "shots")]
pub fn pasted_fixture(window: &mut Window, cx: &mut App) {
    ensure_connections(cx);
    open_connection_dialog(
        DialogMode::NewFromText(
            "postgres://deploy:hv_shot_pw@inv-db.acme.io:5432/inventory?sslmode=verify-full&application_name=hacivat-inventory&target_session_attrs=read-write"
                .to_string(),
        ),
        window,
        cx,
    );
}

/// Shots fixture: a MySQL URL pasted into a PostgreSQL form that already has a host.
#[cfg(feature = "shots")]
pub fn engine_switch_fixture(window: &mut Window, cx: &mut App) {
    ensure_connections(cx);
    open_connection_dialog(DialogMode::New, window, cx);
    if let Some(dialog) = open_dialog(window, cx) {
        dialog.update(cx, |d, cx| {
            d.inputs[&Field::Host].update(cx, |i, cx| {
                i.set_value("db-stg.acme.io");
                cx.notify();
            });
            d.edit_field(Field::Host, "db-stg.acme.io".to_string(), window, cx);
            d.apply_text("mysql://root@db-stg.acme.io:3306/shop", window, cx);
        });
    }
}

/// Shots fixture: the SQLite form.
#[cfg(feature = "shots")]
pub fn sqlite_fixture(window: &mut Window, cx: &mut App) {
    ensure_connections(cx);
    open_connection_dialog(DialogMode::New, window, cx);
    if let Some(dialog) = open_dialog(window, cx) {
        dialog.update(cx, |d, cx| {
            d.set_engine(DbmsKind::Sqlite, window, cx);
            d.inputs[&Field::File].update(cx, |i, cx| {
                i.set_value("~/code/shop/app.db");
                cx.notify();
            });
            d.edit_field(Field::File, "~/code/shop/app.db".to_string(), window, cx);
        });
    }
}
