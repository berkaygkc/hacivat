//! The connection dialog's SSH section (NOTES M2a surface 6, Task 20): the "SSH tunnel"
//! disclosure with its live summary; inside it the "Connect through an SSH tunnel" checkbox and
//! Import from `~/.ssh/config…`, the SSH host, port and user, the sign-in method (Agent / Key
//! file / Password / Keyboard-interactive) with its fields, the ordered jump hosts, the route
//! row and the host-key policy.
//!
//! **State.** The section keeps its fields in a [`SshSection`] entity per dialog, made the first
//! time the section draws (or by [`open_with_ssh`]) and dropped with the dialog. The dialog's
//! draft stays the one source of truth: every edit writes `draft.profile.transport` (a
//! `TransportSpec::SshTunnel` while the checkbox is on, `Direct` when it is off; the tunnel's
//! fields are kept, so turning it on again restores them), `ssh_retention` and the typed secrets
//! in `draft.ssh_secrets[hop]` (Q23). A change the section did not make (a paste's Undo, say)
//! is read back when the dialog changes.
//!
//! **Hops.** `SshSpec::hops` lists the path in order: the jump hosts first, then the SSH host
//! (the last hop). A jump row takes `host` or `host:port` (`[v6]:port`); its sign-in method is
//! the agent unless an import gave it a key. Only the SSH host has secret fields; a jump host
//! that asks for one is prompted for when connecting (Task 21).
//!
//! **Issues (Q9).** `ValidationIssue.field` names no SSH field, so the section places the SSH
//! issues itself by `IssueCode`: beside the hop they concern (a missing or unusable host, port 0,
//! a missing user; per jump row too), beside the key file, and under the jump list (too many
//! hops; a jump host's key). New SSH issues open the disclosure.
//!
//! **Import.** The config is read on the core runtime and comes back over a `oneshot` (§5.4);
//! every value passes `carries_secret` there, and an entry that held a secret only says so. The
//! menu lists every alias with what it resolves to; `ProxyCommand` and other unusable entries
//! are disabled with the reason. Picking one fills the fields, tints them like a paste and
//! shows a notice with the config's notices for that chain.
//!
//! **Keys.** Alt+↑ / Alt+↓ move the jump row that holds the focus (an element-level
//! `on_key_down` on the row; `menus::rebuild` owns every key binding, Q34). Tab, Escape and
//! Enter reach the dialog: the Selects stop them only while their menu is open.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use hacivat_driver::EngineCatalog;
use hacivat_driver::url::carries_secret;
use hacivat_l10n::{Count, MsgRef, NoArgs, Text, ids};
use hacivat_net::ssh::{
    ConfigIssue, ConfigNotice, HopSecrets, HopSpec, HostKeyPolicy, SshAuth, SshConfig, SshSpec,
    read_ssh_config, resolve_alias,
};
use hacivat_proto::ProfileId;
use hacivat_secrets::Retention;
use hacivat_session::pipeline::{IssueCode, ValidationIssue};
use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint, TransportSpec};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::checkbox::Checkbox;
use hacivat_ui::disclosure::Disclosure;
use hacivat_ui::form::{FORM_LABEL_PX, form_row, hint_row};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};
use hacivat_ui::notice::{InlineNotice, NoticeIcon};
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented};
use hacivat_ui::select::{Select, SelectEvent, SelectOption};
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    Anchor, AnyElement, App, AppContext as _, Bounds, Context, Div, Entity, EntityId, FocusHandle,
    Focusable as _, Global, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement,
    PathPromptOptions, Pixels, SharedString, Styled, Subscription, Task, WeakEntity, Window,
    anchored, canvas, deferred, div, point, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use secrecy::SecretString;
use tokio::sync::oneshot;

use super::form::retention_options;
use super::paste::{TINT_FADE, TINT_HOLD, paste_filter};
use super::{ConnectionDialog, DialogEnv, DialogFocus, RETENTIONS, open_dialog, open_edit};
use crate::connections::model::ConnectionsModel;
use crate::l10n_global::{text_fn, tr};

/// The SSH port a hop uses when none is typed.
const DEFAULT_SSH_PORT: u16 = 22;
/// The port field (the mock's `.in.port`).
const PORT_PX: f32 = 76.0;
/// A jump row's user field (the mock's `.in.short`).
const JUMP_USER_PX: f32 = 96.0;
/// A jump row's number column.
const JUMP_INDEX_PX: f32 = 12.0;
/// The gap between jump rows (the mock's `.hops`).
const JUMP_GAP_PX: f32 = 6.0;
/// The Remember select (the DB password's width).
const REMEMBER_PX: f32 = 186.0;
/// The host-key select (the mock's `sel-policy`).
const HOST_KEYS_PX: f32 = 240.0;
/// The gap between the stops of the route row (the mock's `.route`).
const ROUTE_GAP_PX: f32 = 6.0;
/// The gap between the import button and its menu.
const MENU_GAP_PX: f32 = 4.0;

/// Example values in placeholders (the mock's): data, never translated.
const EXAMPLE_BASTION: &str = "bastion.acme.io";
const EXAMPLE_JUMP: &str = "jump-eu.acme.io";
const EXAMPLE_USER: &str = "deploy";
const EXAMPLE_KEY: &str = "~/.ssh/id_ed25519";
/// A route stop not typed yet, and the arrow between stops (symbols, not text).
const ROUTE_GAP_MARK: &str = "…";
const ROUTE_ARROW: &str = "→";

/// The host-key policies, in the select's order: never an "accept everything" choice.
const HOST_KEY_POLICIES: [HostKeyPolicy; 3] = HostKeyPolicy::ALL;

/// Where the import reads the config. Without it: `~/.ssh/config` under the user's home, and
/// the login name from the environment (`USER`, `USERNAME`). Tests point it at a fixture.
///
/// An imported key under `home` is written `~/…` only when `home` is the user's real home
/// folder: Validate and the tunnel expand `~` with the real one (`local_path`), so a fixture
/// home keeps its absolute paths (review R1).
#[derive(Debug, Clone)]
pub struct SshImportEnv {
    pub config: PathBuf,
    pub home: PathBuf,
    pub local_user: String,
}

impl Global for SshImportEnv {}

/// The SSH section of each open dialog, by the dialog's entity ID.
#[derive(Default)]
struct Sections(HashMap<EntityId, Entity<SshSection>>);

impl Global for Sections {}

/// An SSH sign-in method, in the segmented control's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthKind {
    Agent,
    Key,
    Password,
    Kbd,
}

impl AuthKind {
    pub const ALL: [AuthKind; 4] = [
        AuthKind::Agent,
        AuthKind::Key,
        AuthKind::Password,
        AuthKind::Kbd,
    ];

    /// The stable name: `agent`, `key`, `password`, `kbd`.
    pub fn id(self) -> &'static str {
        match self {
            AuthKind::Agent => "agent",
            AuthKind::Key => "key",
            AuthKind::Password => "password",
            AuthKind::Kbd => "kbd",
        }
    }

    fn of(auth: &SshAuth) -> AuthKind {
        match auth {
            SshAuth::Agent => AuthKind::Agent,
            SshAuth::KeyFile { .. } => AuthKind::Key,
            SshAuth::Password => AuthKind::Password,
            SshAuth::KeyboardInteractive => AuthKind::Kbd,
        }
    }

    fn label(self) -> MsgRef<NoArgs> {
        match self {
            AuthKind::Agent => ids::hv::CONN_SSH_AUTH_AGENT,
            AuthKind::Key => ids::hv::CONN_SSH_AUTH_KEY,
            AuthKind::Password => ids::hv::CONN_SSH_AUTH_PASSWORD,
            AuthKind::Kbd => ids::hv::CONN_SSH_AUTH_KBD,
        }
    }
}

/// A text field of the section; a jump row's fields by the row's ID (stable when rows move).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SshField {
    Host,
    Port,
    User,
    Key,
    Passphrase,
    Password,
    JumpHost(u64),
    JumpUser(u64),
}

/// One jump host row.
struct JumpRow {
    id: u64,
    host: Entity<TextInput>,
    user: Entity<TextInput>,
    remove: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// The section's focus targets that are not text fields or selects.
struct SshFocus {
    /// The whole section, the disclosure row included (not a tab stop).
    root: FocusHandle,
    /// The sign-in segmented control's wrapper (not a tab stop).
    auth: FocusHandle,
    use_tunnel: FocusHandle,
    import: FocusHandle,
    choose_key: FocusHandle,
    add_jump: FocusHandle,
}

/// What an import entry resolves to (its hint), computed with the config.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resolution {
    /// `user@host`, through these `ProxyJump` hosts or with this key.
    Usable {
        target: String,
        jumps: Option<String>,
        key: Option<String>,
    },
    Unusable(ConfigIssue),
}

/// One alias of the config.
#[derive(Debug, Clone)]
struct ImportEntry {
    alias: String,
    resolution: Resolution,
    spec: Option<SshSpec>,
    /// The aliases of the chain (the alias and its jump hosts), for its notices.
    chain: Vec<String>,
}

/// The config, read on the core runtime.
enum Loaded {
    Config {
        entries: Vec<ImportEntry>,
        notices: Vec<ConfigNotice>,
    },
    Unreadable,
}

/// The open import menu.
struct ImportMenu {
    menu: Entity<PopupMenu>,
    entries: Vec<ImportEntry>,
    notices: Vec<ConfigNotice>,
    _subscriptions: Vec<Subscription>,
}

/// The notice after an import.
struct ImportNotice {
    alias: String,
    notices: Vec<ConfigNotice>,
}

/// The paste tint on the fields an import filled (NOTES cross-cutting #7).
struct Tint {
    fields: Vec<SshField>,
    fade_from: Option<Instant>,
    _timers: [Task<()>; 2],
}

/// What a new section starts from: the dialog's draft.
struct Seed {
    transport: TransportSpec,
    ssh_retention: Retention,
    saved: Option<(usize, AuthKind)>,
}

impl Seed {
    fn of(draft: &ProfileDraft) -> Seed {
        let transport = draft.profile.transport.clone();
        let keeps = !draft.is_new && draft.profile.ssh_retention == Retention::Forever;
        let saved = match &transport {
            TransportSpec::SshTunnel(spec) if keeps => saved_secret(spec),
            _ => None,
        };
        Seed {
            transport,
            ssh_retention: draft.profile.ssh_retention,
            saved,
        }
    }
}

/// The SSH host's hop and the method whose secret a saved tunnel keeps in the keyring: a
/// password (Password) or a key passphrase (Key file). The other methods keep none.
fn saved_secret(spec: &SshSpec) -> Option<(usize, AuthKind)> {
    let last = spec.hops.len().checked_sub(1)?;
    match AuthKind::of(&spec.hops[last].auth) {
        kind @ (AuthKind::Password | AuthKind::Key) => Some((last, kind)),
        AuthKind::Agent | AuthKind::Kbd => None,
    }
}

/// The SSH section of one connection dialog.
pub struct SshSection {
    dialog: WeakEntity<ConnectionDialog>,
    /// "Connect through an SSH tunnel".
    on: bool,
    /// The disclosure is open.
    open: bool,
    /// The tunnel being edited; kept while the checkbox is off.
    spec: SshSpec,
    host: Entity<TextInput>,
    port: Entity<TextInput>,
    user: Entity<TextInput>,
    key: Entity<TextInput>,
    passphrase: Entity<TextInput>,
    password: Entity<TextInput>,
    jumps: Vec<JumpRow>,
    next_row: u64,
    host_keys: Entity<Select>,
    remember: Entity<Select>,
    focus: SshFocus,
    /// The dialog's SSH issues, placed by the section.
    issues: Vec<ValidationIssue>,
    read_only: bool,
    keyring_available: bool,
    import: Option<ImportMenu>,
    import_bounds: Option<Bounds<Pixels>>,
    loading: bool,
    notice: Option<ImportNotice>,
    tint: Option<Tint>,
    /// The saved tunnel's secret (its hop and method): its field says it is saved only while
    /// the SSH host is still that hop and signs in with that method.
    saved: Option<(usize, AuthKind)>,
    _subscriptions: Vec<Subscription>,
}

// ---- entry points --------------------------------------------------------------------------------

/// The SSH section of the form for `draft` (the dialog's hook calls it while drawing).
pub(crate) fn section(draft: &ProfileDraft, window: &mut Window, cx: &mut App) -> AnyElement {
    let Some(dialog) = open_dialog(window, cx) else {
        return div().into_any_element();
    };
    let state = state_for(&dialog, || Seed::of(draft), window, cx);
    render(&state, draft, window, cx)
}

/// The "Connect through SSH" fix: the edit dialog of `id` with the SSH section open, its
/// checkbox on and the SSH host focused (Q9).
pub(crate) fn open_with_ssh(id: ProfileId, window: &mut Window, cx: &mut App) {
    open_edit(id, DialogFocus::Ssh, window, cx);
    let Some(dialog) = open_dialog(window, cx) else {
        return;
    };
    // Another dialog was already open (the seam then only focuses it): leave it alone.
    if dialog.read(cx).draft.profile.id != id {
        return;
    }
    let seed = Seed::of(&dialog.read(cx).draft);
    let state = state_for(&dialog, || seed, window, cx);
    state.update(cx, |s, cx| s.show_tunnel(window, cx));
}

/// The SSH section of the dialog open in `window` (tests and the dialog's own reads).
pub fn section_state(window: &Window, cx: &App) -> Option<Entity<SshSection>> {
    let dialog = open_dialog(window, cx)?;
    cx.try_global::<Sections>()?
        .0
        .get(&dialog.entity_id())
        .cloned()
}

/// Whether the focus is inside the SSH section of the dialog open in `window` (the dialog
/// names it `ssh`).
pub(crate) fn contains_focus(window: &Window, cx: &App) -> bool {
    section_state(window, cx).is_some_and(|s| s.read(cx).focus.root.contains_focused(window, cx))
}

/// Whether `issue` belongs to the SSH section (it draws it; the form's top notice does not).
pub(crate) fn owns_issue(profile: &Profile, issue: &ValidationIssue) -> bool {
    matches!(profile.transport, TransportSpec::SshTunnel(_)) && is_ssh_issue(issue)
}

/// The typed SSH secrets of `draft` for a save or a test of `profile` (Q23): none unless it
/// connects through a tunnel.
pub(crate) fn secrets_for(profile: &Profile, draft: &ProfileDraft) -> Vec<HopSecrets> {
    if !matches!(profile.transport, TransportSpec::SshTunnel(_)) {
        return Vec::new();
    }
    draft
        .ssh_secrets
        .iter()
        .map(|s| HopSecrets {
            password: s.password.clone(),
            passphrase: s.passphrase.clone(),
        })
        .collect()
}

fn is_ssh_issue(issue: &ValidationIssue) -> bool {
    match issue.code {
        IssueCode::SshHostMissing
        | IssueCode::SshUserMissing
        | IssueCode::SshTooManyHops
        | IssueCode::SshKeyFileMissing => true,
        // The database host's and port's carry their field; a hop's carry none.
        IssueCode::HostInvalid | IssueCode::PortInvalid => issue.field.is_none(),
        _ => false,
    }
}

/// The section of `dialog`, made from `seed` the first time.
fn state_for(
    dialog: &Entity<ConnectionDialog>,
    seed: impl FnOnce() -> Seed,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SshSection> {
    let id = dialog.entity_id();
    if let Some(state) = cx.try_global::<Sections>().and_then(|s| s.0.get(&id)) {
        return state.clone();
    }
    let seed = seed();
    let state = cx.new(|cx| SshSection::new(dialog, seed, window, cx));
    cx.default_global::<Sections>().0.insert(id, state.clone());
    cx.observe_release(dialog, move |_, cx| {
        cx.default_global::<Sections>().0.remove(&id);
    })
    .detach();
    state
}

// ---- the section ---------------------------------------------------------------------------------

impl SshSection {
    fn new(
        dialog: &Entity<ConnectionDialog>,
        seed: Seed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = dialog.downgrade();
        let data = |s: &'static str| -> TextFn { Rc::new(move |_: &App| SharedString::from(s)) };
        let stored = |cx: &App| tr(cx, ids::hv::CONN_DLG_PW_SAVED, NoArgs);
        let mut subscriptions = Vec::new();
        let mut make = |field: SshField,
                        placeholder: TextFn,
                        source: TextSource,
                        masked: bool,
                        cx: &mut Context<Self>| {
            let input = new_input(placeholder, source, masked, weak.clone(), cx);
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, _, event: &InputEvent, window, cx| {
                    this.on_input(field, *event, window, cx)
                },
            ));
            input
        };
        let host = make(
            SshField::Host,
            data(EXAMPLE_BASTION),
            TextSource::Data,
            false,
            cx,
        );
        let port = make(
            SshField::Port,
            data(DEFAULT_PORT_TEXT),
            TextSource::Data,
            false,
            cx,
        );
        let user = make(
            SshField::User,
            data(EXAMPLE_USER),
            TextSource::Data,
            false,
            cx,
        );
        let key = make(
            SshField::Key,
            data(EXAMPLE_KEY),
            TextSource::Data,
            false,
            cx,
        );
        // "Saved in the Keychain" only where a secret is kept (I1); the source follows it
        // (`sync_saved_placeholders`).
        let me = cx.weak_entity();
        let saved_text = |field: SshField| -> TextFn {
            let me = me.clone();
            Rc::new(move |cx: &App| {
                let shows = me.upgrade().is_some_and(|s| s.read(cx).shows_saved(field));
                if shows {
                    stored(cx)
                } else {
                    SharedString::default()
                }
            })
        };
        let passphrase = make(
            SshField::Passphrase,
            saved_text(SshField::Passphrase),
            TextSource::Data,
            true,
            cx,
        );
        let password = make(
            SshField::Password,
            saved_text(SshField::Password),
            TextSource::Data,
            true,
            cx,
        );

        let spec = match &seed.transport {
            TransportSpec::SshTunnel(spec) => spec.clone(),
            _ => SshSpec::default(),
        };
        let host_keys = cx.new(|cx| {
            let selected = policy_index(spec.host_keys);
            let mut s = Select::new(host_key_options(), selected, cx);
            s.set_label(text_fn(ids::hv::CONN_SSH_HOSTKEYS));
            s
        });
        let remember = cx.new(|cx| {
            let selected = retention_index(seed.ssh_retention);
            let mut s = Select::new(retention_options(true), selected, cx);
            s.set_label(text_fn(ids::hv::CONN_DLG_REMEMBER));
            s
        });
        subscriptions.push(cx.subscribe_in(
            &host_keys,
            window,
            |this, _, SelectEvent::Changed(i): &SelectEvent, window, cx| {
                if let Some(policy) = HOST_KEY_POLICIES.get(*i) {
                    this.spec.host_keys = *policy;
                    this.push(&[], window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &remember,
            window,
            |this, _, SelectEvent::Changed(i): &SelectEvent, window, cx| {
                if let Some(retention) = RETENTIONS.get(*i).copied() {
                    this.set_retention(retention, window, cx);
                }
            },
        ));
        subscriptions.push(cx.observe_in(dialog, window, |this, dialog, window, cx| {
            this.sync_from(&dialog, window, cx)
        }));

        let stop = |cx: &mut Context<Self>| cx.focus_handle().tab_stop(true);
        let focus = SshFocus {
            root: cx.focus_handle(),
            auth: cx.focus_handle(),
            use_tunnel: stop(cx),
            import: stop(cx),
            choose_key: stop(cx),
            add_jump: stop(cx),
        };
        let mut section = SshSection {
            dialog: weak,
            on: matches!(seed.transport, TransportSpec::SshTunnel(_)),
            open: false,
            spec,
            host,
            port,
            user,
            key,
            passphrase,
            password,
            jumps: Vec::new(),
            next_row: 0,
            host_keys,
            remember,
            focus,
            issues: Vec::new(),
            read_only: false,
            keyring_available: true,
            import: None,
            import_bounds: None,
            loading: false,
            notice: None,
            tint: None,
            saved: seed.saved,
            _subscriptions: subscriptions,
        };
        section.write_fields(window, cx);
        // The dialog is drawing (it is leased): read its state once the frame is done.
        cx.defer_in(window, |this, window, cx| {
            if let Some(dialog) = this.dialog.upgrade() {
                this.sync_from(&dialog, window, cx);
            }
        });
        section
    }

    /// Whether the secret `field` shows "saved in the Keychain": the saved tunnel kept that
    /// field's secret for the hop that is still the SSH host, and it still signs in that way.
    fn shows_saved(&self, field: SshField) -> bool {
        let wanted = match field {
            SshField::Password => AuthKind::Password,
            SshField::Passphrase => AuthKind::Key,
            _ => return false,
        };
        let Some((hop, kind)) = self.saved else {
            return false;
        };
        self.on
            && kind == wanted
            && self.main_auth() == wanted
            && self.spec.hops.len().checked_sub(1) == Some(hop)
    }

    /// The secret fields' placeholder is a message only while it says "saved" (else data).
    fn sync_saved_placeholders(&self, cx: &mut Context<Self>) {
        for (field, input) in [
            (SshField::Password, &self.password),
            (SshField::Passphrase, &self.passphrase),
        ] {
            let source = if self.shows_saved(field) {
                TextSource::Fluent
            } else {
                TextSource::Data
            };
            input.update(cx, |i, cx| {
                i.set_placeholder_source(source);
                cx.notify();
            });
        }
    }

    // ---- reading (tests read the section through these) ----------------------------------------

    /// "Connect through an SSH tunnel" is checked.
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// The disclosure is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The text field `name`: `ssh-host`, `ssh-port`, `ssh-user`, `ssh-key-file`,
    /// `ssh-passphrase`, `ssh-password`, `ssh-jump-host-<i>`, `ssh-jump-user-<i>`.
    pub fn input(&self, name: &str) -> Option<Entity<TextInput>> {
        let fixed = match name {
            "ssh-host" => Some(&self.host),
            "ssh-port" => Some(&self.port),
            "ssh-user" => Some(&self.user),
            "ssh-key-file" => Some(&self.key),
            "ssh-passphrase" => Some(&self.passphrase),
            "ssh-password" => Some(&self.password),
            _ => None,
        };
        if let Some(input) = fixed {
            return Some(input.clone());
        }
        let row = |prefix: &str| {
            name.strip_prefix(prefix)
                .and_then(|i| i.parse::<usize>().ok())
                .and_then(|i| self.jumps.get(i))
        };
        row("ssh-jump-host-")
            .map(|r| r.host.clone())
            .or_else(|| row("ssh-jump-user-").map(|r| r.user.clone()))
    }

    /// The draft of the section's dialog (what a save or a test takes).
    pub fn draft<'a>(&self, cx: &'a App) -> Option<&'a ProfileDraft> {
        self.dialog.upgrade().map(|d| &d.read(cx).draft)
    }

    /// How many jump rows the section shows.
    pub fn jump_count(&self) -> usize {
        self.jumps.len()
    }

    /// The host-key select.
    pub fn host_key_select(&self) -> Entity<Select> {
        self.host_keys.clone()
    }

    /// The IDs of the host-key choices, in the select's order.
    pub fn host_key_choices(&self) -> Vec<&'static str> {
        HOST_KEY_POLICIES.iter().map(|p| p.id()).collect()
    }

    /// The Remember select of the SSH secrets (`ssh_retention`).
    pub fn remember_select(&self) -> Entity<Select> {
        self.remember.clone()
    }

    /// Whether the import menu is open.
    pub fn import_open(&self) -> bool {
        self.import.is_some()
    }

    /// The import menu's entries: each alias, whether it can be picked, and its hint.
    pub fn import_entries(&self, cx: &App) -> Vec<(String, bool, SharedString)> {
        self.import
            .iter()
            .flat_map(|m| &m.entries)
            .map(|e| {
                let usable = e.spec.is_some();
                (e.alias.clone(), usable, resolution_text(cx, &e.resolution))
            })
            .collect()
    }

    /// Picks the import entry `alias` (the menu's click and Enter end here).
    pub fn pick_import(&mut self, alias: &str, window: &mut Window, cx: &mut Context<Self>) {
        let at = self
            .import
            .as_ref()
            .and_then(|m| m.entries.iter().position(|e| e.alias == alias));
        if let Some(at) = at {
            self.pick(at, window, cx);
        }
    }

    /// Whether the field `name` (as [`Self::input`] names it) shows the import tint.
    pub fn is_tinted(&self, name: &str) -> bool {
        let Some(field) = self.field_named(name) else {
            return false;
        };
        self.tint
            .as_ref()
            .is_some_and(|t| t.fields.contains(&field))
    }

    /// The name of the section's focused element (`ssh` for the disclosure row), if any.
    pub fn focused(&self, window: &Window, cx: &App) -> Option<String> {
        let inputs = [
            ("ssh-host", &self.host),
            ("ssh-port", &self.port),
            ("ssh-user", &self.user),
            ("ssh-key-file", &self.key),
            ("ssh-passphrase", &self.passphrase),
            ("ssh-password", &self.password),
        ];
        for (name, input) in inputs {
            if input.read(cx).focus_handle(cx).is_focused(window) {
                return Some(name.to_string());
            }
        }
        for (i, row) in self.jumps.iter().enumerate() {
            if row.host.read(cx).focus_handle(cx).is_focused(window) {
                return Some(format!("ssh-jump-host-{i}"));
            }
            if row.user.read(cx).focus_handle(cx).is_focused(window) {
                return Some(format!("ssh-jump-user-{i}"));
            }
            if row.remove.is_focused(window) {
                return Some(format!("ssh-jump-remove-{i}"));
            }
        }
        let f = &self.focus;
        let owned = [
            ("ssh-use", &f.use_tunnel),
            ("ssh-import", &f.import),
            ("ssh-choose-key", &f.choose_key),
            ("ssh-add-jump", &f.add_jump),
        ];
        if let Some((name, _)) = owned.iter().find(|(_, h)| h.is_focused(window)) {
            return Some(name.to_string());
        }
        let selects = [
            ("ssh-hostkeys", self.host_keys.read(cx).focus_handle(cx)),
            ("ssh-remember", self.remember.read(cx).focus_handle(cx)),
        ];
        if let Some((name, _)) = selects.iter().find(|(_, h)| h.is_focused(window)) {
            return Some(name.to_string());
        }
        if f.auth.contains_focused(window, cx) {
            return Some("ssh-auth".to_string());
        }
        f.root
            .contains_focused(window, cx)
            .then(|| "ssh".to_string())
    }

    fn field_named(&self, name: &str) -> Option<SshField> {
        Some(match name {
            "ssh-host" => SshField::Host,
            "ssh-port" => SshField::Port,
            "ssh-user" => SshField::User,
            "ssh-key-file" => SshField::Key,
            "ssh-passphrase" => SshField::Passphrase,
            "ssh-password" => SshField::Password,
            _ => {
                let row = |prefix: &str| {
                    name.strip_prefix(prefix)
                        .and_then(|i| i.parse::<usize>().ok())
                        .and_then(|i| self.jumps.get(i))
                        .map(|r| r.id)
                };
                if let Some(id) = row("ssh-jump-host-") {
                    SshField::JumpHost(id)
                } else {
                    SshField::JumpUser(row("ssh-jump-user-")?)
                }
            }
        })
    }

    // ---- the draft -------------------------------------------------------------------------------

    /// The SSH host's hop (the last); the tunnel always has one while the checkbox is on.
    fn main(&mut self) -> &mut HopSpec {
        if self.spec.hops.is_empty() {
            self.spec.hops.push(new_hop(String::new()));
        }
        let last = self.spec.hops.len() - 1;
        &mut self.spec.hops[last]
    }

    fn main_ref(&self) -> Option<&HopSpec> {
        self.spec.hops.last()
    }

    fn main_auth(&self) -> AuthKind {
        self.main_ref()
            .map_or(AuthKind::Agent, |h| AuthKind::of(&h.auth))
    }

    /// The typed secrets per hop: only the SSH host has secret fields, and only the one its
    /// sign-in method uses is kept.
    fn secrets(&self, cx: &App) -> Vec<HopSecrets> {
        if !self.on {
            return Vec::new();
        }
        let typed = |input: &Entity<TextInput>| {
            let value = input.read(cx).value();
            (!value.is_empty()).then(|| SecretString::from(value.to_string()))
        };
        let auth = self.main_auth();
        let mut out: Vec<HopSecrets> = (0..self.spec.hops.len())
            .map(|_| HopSecrets {
                password: None,
                passphrase: None,
            })
            .collect();
        if let Some(last) = out.last_mut() {
            last.password = (auth == AuthKind::Password)
                .then(|| typed(&self.password))
                .flatten();
            last.passphrase = (auth == AuthKind::Key)
                .then(|| typed(&self.passphrase))
                .flatten();
        }
        out
    }

    /// Writes the tunnel and the typed secrets into the dialog's draft; the dialog's issues
    /// that `cleared` names go (the fields they concern changed).
    fn push(&mut self, cleared: &[IssueCode], window: &mut Window, cx: &mut Context<Self>) {
        let transport = if self.on {
            TransportSpec::SshTunnel(self.spec.clone())
        } else {
            TransportSpec::Direct
        };
        let secrets = self.secrets(cx);
        let cleared = cleared.to_vec();
        if let Some(dialog) = self.dialog.upgrade() {
            dialog.update(cx, |d, cx| {
                d.draft.profile.transport = transport;
                d.draft.ssh_secrets = secrets;
                d.issues
                    .retain(|i| !(is_ssh_issue(i) && cleared.contains(&i.code)));
                d.after_edit(window, cx);
            });
        }
        self.issues.retain(|i| !cleared.contains(&i.code));
        self.sync_saved_placeholders(cx);
        cx.notify();
    }

    fn set_retention(&mut self, retention: Retention, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dialog) = self.dialog.upgrade() {
            dialog.update(cx, |d, cx| {
                d.draft.profile.ssh_retention = retention;
                d.after_edit(window, cx);
            });
        }
        cx.notify();
    }

    /// Reads what the dialog changed: its tunnel (a paste's Undo), retention, issues, keyring
    /// and whether it can save.
    fn sync_from(
        &mut self,
        dialog: &Entity<ConnectionDialog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let d = dialog.read(cx);
        let transport = d.draft.profile.transport.clone();
        let owned: Vec<ValidationIssue> = d
            .issues
            .iter()
            .filter(|i| owns_issue(&d.draft.profile, i))
            .cloned()
            .collect();
        let retention = retention_index(d.draft.profile.ssh_retention);
        let read_only = d.read_only;
        let keyring = d.keyring_available;
        let mut changed = false;
        let mut reload = false;
        match transport {
            TransportSpec::SshTunnel(spec) => {
                if !self.on || spec != self.spec {
                    self.on = true;
                    self.spec = spec;
                    reload = true;
                }
            }
            _ => {
                if self.on {
                    self.on = false;
                    changed = true;
                }
            }
        }
        if owned != self.issues {
            // New SSH issues (a save that failed) open the section that shows them.
            if owned.iter().any(|i| !self.issues.contains(i)) {
                self.open = true;
            }
            self.issues = owned;
            changed = true;
        }
        if self.remember.read(cx).selected() != retention {
            self.remember
                .update(cx, |s, cx| s.set_selected(retention, cx));
        }
        if keyring != self.keyring_available {
            self.keyring_available = keyring;
            self.remember.update(cx, |s, cx| {
                s.set_options(retention_options(keyring), cx);
                s.set_selected(retention, cx);
            });
            changed = true;
        }
        if read_only != self.read_only {
            self.read_only = read_only;
            for input in self.all_inputs() {
                input.update(cx, |i, cx| {
                    i.set_read_only(read_only);
                    cx.notify();
                });
            }
            changed = true;
        }
        if reload {
            self.write_fields(window, cx);
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    fn all_inputs(&self) -> Vec<Entity<TextInput>> {
        let mut out = vec![
            self.host.clone(),
            self.port.clone(),
            self.user.clone(),
            self.key.clone(),
            self.passphrase.clone(),
            self.password.clone(),
        ];
        for row in &self.jumps {
            out.push(row.host.clone());
            out.push(row.user.clone());
        }
        out
    }

    /// Writes every field from the tunnel (a new section, an import, a change from outside).
    /// Emits nothing. The secret fields keep what was typed.
    fn write_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let set = |input: &Entity<TextInput>, value: String, cx: &mut Context<Self>| {
            input.update(cx, |i, cx| {
                if i.value() != value.as_str() {
                    i.set_value(value);
                    cx.notify();
                }
            });
        };
        let main = self.spec.hops.last().cloned();
        if let Some(main) = &main {
            set(&self.host, main.host.clone(), cx);
            set(&self.port, main.port.to_string(), cx);
            set(&self.user, main.user.clone(), cx);
            if let SshAuth::KeyFile { path } = &main.auth {
                set(&self.key, path.to_string_lossy().into_owned(), cx);
            }
        } else {
            set(&self.port, DEFAULT_SSH_PORT.to_string(), cx);
        }
        let jumps: Vec<HopSpec> = match self.spec.hops.split_last() {
            Some((_, jumps)) => jumps.to_vec(),
            None => Vec::new(),
        };
        while self.jumps.len() > jumps.len() {
            self.jumps.pop();
        }
        while self.jumps.len() < jumps.len() {
            let row = self.new_row(window, cx);
            self.jumps.push(row);
        }
        let rows: Vec<(Entity<TextInput>, Entity<TextInput>)> = self
            .jumps
            .iter()
            .map(|r| (r.host.clone(), r.user.clone()))
            .collect();
        for ((host, user), hop) in rows.iter().zip(&jumps) {
            set(host, hop_address(hop), cx);
            set(user, hop.user.clone(), cx);
        }
        let policy = policy_index(self.spec.host_keys);
        self.host_keys
            .update(cx, |s, cx| s.set_selected(policy, cx));
        self.sync_saved_placeholders(cx);
    }

    fn new_row(&mut self, window: &mut Window, cx: &mut Context<Self>) -> JumpRow {
        let id = self.next_row;
        self.next_row += 1;
        let data = |s: &'static str| -> TextFn { Rc::new(move |_: &App| SharedString::from(s)) };
        let read_only = self.read_only;
        let mut subscriptions = Vec::new();
        let mut make = |field: SshField, placeholder: TextFn, cx: &mut Context<Self>| {
            let input = new_input(
                placeholder,
                TextSource::Data,
                false,
                self.dialog.clone(),
                cx,
            );
            input.update(cx, |i, _| i.set_read_only(read_only));
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, _, event: &InputEvent, window, cx| {
                    this.on_input(field, *event, window, cx)
                },
            ));
            input
        };
        let host = make(SshField::JumpHost(id), data(EXAMPLE_JUMP), cx);
        let user = make(SshField::JumpUser(id), data(EXAMPLE_USER), cx);
        JumpRow {
            id,
            host,
            user,
            remove: cx.focus_handle().tab_stop(true),
            _subscriptions: subscriptions,
        }
    }

    fn row_index(&self, id: u64) -> Option<usize> {
        self.jumps.iter().position(|r| r.id == id)
    }

    // ---- editing ---------------------------------------------------------------------------------

    fn on_input(
        &mut self,
        field: SshField,
        event: InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => self.edited(field, window, cx),
            InputEvent::Cancelled => {
                if let Some(dialog) = self.dialog.upgrade() {
                    dialog.update(cx, |d, cx| d.escape(window, cx));
                }
            }
            _ => {}
        }
    }

    /// A typed change of `field`.
    fn edited(&mut self, field: SshField, window: &mut Window, cx: &mut Context<Self>) {
        if !self.on {
            return;
        }
        let text =
            |input: &Entity<TextInput>, cx: &Context<Self>| input.read(cx).value().to_string();
        let cleared: Vec<IssueCode> = match field {
            SshField::Host => {
                let value = text(&self.host, cx);
                self.main().host = value;
                vec![IssueCode::SshHostMissing, IssueCode::HostInvalid]
            }
            SshField::Port => {
                let value = text(&self.port, cx);
                self.main().port = parse_port(&value);
                vec![IssueCode::PortInvalid]
            }
            SshField::User => {
                let value = text(&self.user, cx);
                self.main().user = value;
                vec![IssueCode::SshUserMissing]
            }
            SshField::Key => {
                let value = text(&self.key, cx);
                self.main().auth = SshAuth::KeyFile {
                    path: PathBuf::from(value),
                };
                vec![IssueCode::SshKeyFileMissing]
            }
            SshField::Passphrase | SshField::Password => Vec::new(),
            SshField::JumpHost(id) | SshField::JumpUser(id) => {
                let Some(at) = self.row_index(id) else {
                    return;
                };
                let row = &self.jumps[at];
                let host = text(&row.host, cx);
                let user = text(&row.user, cx);
                let Some(hop) = self.spec.hops.get_mut(at) else {
                    return;
                };
                if matches!(field, SshField::JumpHost(_)) {
                    (hop.host, hop.port) = split_address(&host);
                    vec![
                        IssueCode::SshHostMissing,
                        IssueCode::HostInvalid,
                        IssueCode::PortInvalid,
                    ]
                } else {
                    hop.user = user;
                    vec![IssueCode::SshUserMissing]
                }
            }
        };
        if let Some(tint) = &mut self.tint {
            tint.fields.retain(|f| *f != field);
        }
        self.push(&cleared, window, cx);
    }

    /// The checkbox.
    fn set_on(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || self.on == on {
            return;
        }
        self.on = on;
        if on && self.spec.hops.is_empty() {
            self.spec.hops.push(new_hop(String::new()));
            self.write_fields(window, cx);
        }
        self.push(&[], window, cx);
    }

    /// Opens the section with the tunnel on and focuses the SSH host (the "Connect through
    /// SSH" fix, Q9).
    fn show_tunnel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        if !self.on && !self.read_only {
            self.set_on(true, window, cx);
        }
        let handle = self.host.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.refresh(cx);
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.open = open;
        self.refresh(cx);
    }

    /// Redraws the dialog (the section draws inside it).
    fn refresh(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        if let Some(dialog) = self.dialog.upgrade() {
            dialog.update(cx, |_, cx| cx.notify());
        }
    }

    /// The sign-in method of the SSH host.
    fn set_auth(&mut self, kind: AuthKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || self.main_auth() == kind {
            return;
        }
        let key = self.key.read(cx).value().to_string();
        let main = self.main();
        main.auth = match kind {
            AuthKind::Agent => SshAuth::Agent,
            AuthKind::Key => SshAuth::KeyFile {
                path: PathBuf::from(key),
            },
            AuthKind::Password => SshAuth::Password,
            AuthKind::Kbd => SshAuth::KeyboardInteractive,
        };
        self.push(&[IssueCode::SshKeyFileMissing], window, cx);
    }

    fn add_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || !self.on {
            return;
        }
        let user = self.main().user.clone();
        let at = self.spec.hops.len() - 1;
        self.spec.hops.insert(at, new_hop(user.clone()));
        let row = self.new_row(window, cx);
        row.user.update(cx, |i, cx| {
            i.set_value(user);
            cx.notify();
        });
        let focus = row.host.read(cx).focus_handle(cx);
        self.jumps.push(row);
        self.push(&[], window, cx);
        window.focus(&focus, cx);
    }

    fn remove_jump(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(at) = self.row_index(id) else {
            return;
        };
        if self.read_only {
            return;
        }
        self.jumps.remove(at);
        self.spec.hops.remove(at);
        // The focus goes to the row that took its place, else to Add jump host.
        let next = self
            .jumps
            .get(at)
            .map(|r| r.remove.clone())
            .unwrap_or_else(|| self.focus.add_jump.clone());
        window.focus(&next, cx);
        self.push(
            &[
                IssueCode::SshTooManyHops,
                IssueCode::SshHostMissing,
                IssueCode::SshUserMissing,
                IssueCode::HostInvalid,
                IssueCode::PortInvalid,
            ],
            window,
            cx,
        );
    }

    /// Moves the jump row `id` one place up (`-1`) or down (`1`); the focus stays in it.
    fn move_jump(&mut self, id: u64, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(at) = self.row_index(id) else {
            return;
        };
        let Some(to) = at
            .checked_add_signed(by)
            .filter(|to| *to < self.jumps.len())
        else {
            return;
        };
        if self.read_only {
            return;
        }
        self.jumps.swap(at, to);
        self.spec.hops.swap(at, to);
        self.push(&[], window, cx);
    }

    /// Alt+↑ / Alt+↓ on a jump row (element-level, Q34).
    fn row_key(
        &mut self,
        id: u64,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let stroke = &event.keystroke;
        let m = &stroke.modifiers;
        if !m.alt || m.control || m.platform || m.shift || m.function {
            return false;
        }
        let by = match stroke.key.as_str() {
            "up" => -1,
            "down" => 1,
            _ => return false,
        };
        self.move_jump(id, by, window, cx);
        true
    }

    fn choose_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
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
            let _ = this.update_in(cx, |s, window, cx| s.picked_key(path, window, cx));
        })
        .detach();
    }

    /// `path` was picked as the key file: it joins the approved roots (T16 ruling A).
    fn picked_key(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(env) = cx.try_global::<DialogEnv>() {
            env.approved_roots.add(path.clone());
        }
        let text = path.to_string_lossy().into_owned();
        self.key.update(cx, |i, cx| {
            i.set_value(text);
            cx.notify();
        });
        self.edited(SshField::Key, window, cx);
    }

    // ---- import ----------------------------------------------------------------------------------

    /// Reads the config on the core runtime, then opens the menu.
    fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || self.loading || self.import.is_some() {
            return;
        }
        let Some(model) = cx.try_global::<ConnectionsModel>() else {
            return;
        };
        let env = cx.try_global::<SshImportEnv>().cloned();
        let (tx, rx) = oneshot::channel();
        model.runtime().spawn_blocking(move || {
            let _ = tx.send(load_config(env));
        });
        self.loading = true;
        cx.spawn_in(window, async move |this, cx| {
            let loaded = rx.await.unwrap_or(Loaded::Unreadable);
            let _ = this.update_in(cx, |s, window, cx| s.show_import(loaded, window, cx));
        })
        .detach();
    }

    fn show_import(&mut self, loaded: Loaded, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = false;
        let (entries, notices, empty) = match loaded {
            Loaded::Config { entries, notices } => {
                (entries, notices, ids::hv::CONN_SSH_IMPORT_NONE)
            }
            Loaded::Unreadable => (Vec::new(), Vec::new(), ids::hv::CONN_SSH_IMPORT_UNREADABLE),
        };
        let items: Vec<MenuItemSpec> = if entries.is_empty() {
            vec![MenuItemSpec::new(text_fn(empty), "none").disabled(true)]
        } else {
            entries
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let alias = SharedString::from(e.alias.clone());
                    let resolution = e.resolution.clone();
                    MenuItemSpec::new(Rc::new(move |_: &App| alias.clone()), i.to_string())
                        .disabled(e.spec.is_none())
                        .hint(Rc::new(move |cx: &App| resolution_text(cx, &resolution)))
                })
                .collect()
        };
        let menu = PopupMenu::embedded(items, window, cx);
        let width = self.import_bounds.map(|b| b.size.width).unwrap_or_default();
        menu.update(cx, |m, _| {
            m.set_min_width(width);
            m.label(text_fn(ids::hv::CONN_SSH_IMPORT_TITLE));
        });
        let events = cx.subscribe_in(&menu, window, Self::on_menu_event);
        let focus = menu.focus_handle(cx);
        let blur = cx.on_focus_out(&focus, window, |this, _, window, cx| {
            this.close_import(false, window, cx)
        });
        self.import = Some(ImportMenu {
            menu,
            entries,
            notices,
            _subscriptions: vec![events, blur],
        });
        self.refresh(cx);
    }

    fn on_menu_event(
        &mut self,
        _: &Entity<PopupMenu>,
        event: &PopupMenuEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PopupMenuEvent::Chosen(action) => {
                let at = action.parse::<usize>().ok();
                if let Some(at) = at {
                    self.pick(at, window, cx);
                }
                self.close_import(true, window, cx);
            }
            PopupMenuEvent::Dismissed => self.close_import(true, window, cx),
        }
    }

    /// Closes the menu; `restore` returns the focus to the import button.
    fn close_import(&mut self, restore: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.import.take() else {
            return;
        };
        // Dropped once this update is done: the event in flight still reaches every subscriber.
        cx.defer(move |_| drop(menu));
        if restore {
            window.focus(&self.focus.import, cx);
        }
        self.refresh(cx);
    }

    /// Fills the tunnel from the import entry `at`.
    fn pick(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.import else {
            return;
        };
        let Some(entry) = menu.entries.get(at) else {
            return;
        };
        let Some(spec) = entry.spec.clone() else {
            return;
        };
        let chain: HashSet<String> = entry.chain.iter().map(|a| a.to_ascii_lowercase()).collect();
        let notices: Vec<ConfigNotice> = menu
            .notices
            .iter()
            .filter(|n| notice_alias(n).is_none_or(|a| chain.contains(&a.to_ascii_lowercase())))
            .cloned()
            .collect();
        let alias = entry.alias.clone();
        self.on = true;
        self.open = true;
        self.spec = spec;
        // An imported tunnel is a new one: nothing of it is in the keyring yet.
        self.saved = None;
        self.write_fields(window, cx);
        let mut fields = vec![SshField::Host, SshField::Port, SshField::User];
        if self.main_auth() == AuthKind::Key {
            fields.push(SshField::Key);
        }
        for row in &self.jumps {
            fields.extend([SshField::JumpHost(row.id), SshField::JumpUser(row.id)]);
        }
        self.start_tint(fields, cx);
        self.notice = Some(ImportNotice { alias, notices });
        self.push(
            &[
                IssueCode::SshHostMissing,
                IssueCode::SshUserMissing,
                IssueCode::SshTooManyHops,
                IssueCode::SshKeyFileMissing,
                IssueCode::HostInvalid,
                IssueCode::PortInvalid,
            ],
            window,
            cx,
        );
    }

    /// Tints `fields` for `TINT_HOLD`, then fades them over `TINT_FADE` (the paste tint).
    fn start_tint(&mut self, fields: Vec<SshField>, cx: &mut Context<Self>) {
        let fade = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TINT_HOLD).await;
            let _ = this.update(cx, |s, cx| {
                let now = cx.background_executor().now();
                if let Some(tint) = &mut s.tint {
                    tint.fade_from = Some(now);
                }
                s.refresh(cx);
            });
        });
        let clear = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TINT_HOLD + TINT_FADE).await;
            let _ = this.update(cx, |s, cx| {
                s.tint = None;
                s.refresh(cx);
            });
        });
        self.tint = Some(Tint {
            fields,
            fade_from: None,
            _timers: [fade, clear],
        });
    }

    /// How far the tint of `field` has faded: `None` when it has none, else 0 (tinted) to 1.
    fn tint_progress(&self, field: SshField, cx: &App) -> Option<f32> {
        let tint = self.tint.as_ref().filter(|t| t.fields.contains(&field))?;
        Some(match tint.fade_from {
            None => 0.0,
            Some(from) => {
                let gone = cx
                    .background_executor()
                    .now()
                    .saturating_duration_since(from);
                (gone.as_secs_f32() / TINT_FADE.as_secs_f32()).clamp(0.0, 1.0)
            }
        })
    }
}

// ---- the config (core runtime) -------------------------------------------------------------------

/// Reads and resolves the config. Blocking: runs on the core runtime.
fn load_config(env: Option<SshImportEnv>) -> Loaded {
    let env = match env {
        Some(env) => env,
        None => {
            let Some(home) = std::env::home_dir() else {
                return Loaded::Unreadable;
            };
            let local_user = std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_default();
            SshImportEnv {
                config: home.join(".ssh").join("config"),
                home,
                local_user,
            }
        }
    };
    let cfg = match read_ssh_config(&env.config, &env.home, &env.local_user, &carries_secret) {
        Ok(cfg) => cfg,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => SshConfig::default(),
        Err(_) => return Loaded::Unreadable,
    };
    let tilde_home = Some(env.home.clone()).filter(|h| std::env::home_dir().as_ref() == Some(h));
    let entries = cfg
        .hosts
        .iter()
        // The parser already drops a `Host` pattern that holds a secret; never list one, nor
        // one shaped like credentials (`admin:password@host`).
        .filter(|h| !carries_secret(&h.alias) && !h.alias.contains('@'))
        .map(|h| entry_of(&cfg, &h.alias, tilde_home.as_deref()))
        .collect();
    Loaded::Config {
        entries,
        notices: cfg.notices,
    }
}

/// The import entry of `alias`: its tunnel and what the menu says about it. A value that
/// `carries_secret` refuses is never shown nor imported.
fn entry_of(cfg: &SshConfig, alias: &str, home: Option<&Path>) -> ImportEntry {
    let chain = chain_of(cfg, alias);
    let resolved = resolve_alias(cfg, alias).and_then(|spec| {
        if spec_carries_secret(&spec) {
            Err(ConfigIssue::SecretInValue)
        } else {
            Ok(spec)
        }
    });
    let (resolution, spec) = match resolved {
        Ok(mut spec) => {
            // The key field shows `~/.ssh/…`, as the menu does (Validate and the tunnel expand
            // a leading `~` the same way: `hacivat_net::paths::local_path`).
            for hop in &mut spec.hops {
                if let (SshAuth::KeyFile { path }, Some(home)) = (&mut hop.auth, home) {
                    *path = PathBuf::from(home_relative(path, home));
                }
            }
            let main = spec.hops.last();
            let target = main.map(hop_target).unwrap_or_default();
            let host = cfg
                .hosts
                .iter()
                .find(|h| h.alias.eq_ignore_ascii_case(alias));
            let jumps = host
                .filter(|h| !h.proxy_jump.is_empty())
                .map(|h| h.proxy_jump.join(", "));
            let key = main.and_then(|m| match &m.auth {
                SshAuth::KeyFile { path } => Some(path.to_string_lossy().into_owned()),
                _ => None,
            });
            (Resolution::Usable { target, jumps, key }, Some(spec))
        }
        Err(issue) => (Resolution::Unusable(issue), None),
    };
    ImportEntry {
        alias: alias.to_string(),
        resolution,
        spec,
        chain,
    }
}

/// Whether any value of `spec` carries a secret (the parser checks every value; this guards
/// what reaches the form). A credential-shaped value counts too (review I2): a host with `@`
/// (`admin:password@host`) or one Validate refuses as a hop host, and a user with `:` (the
/// password separator; a POSIX user name never holds one). A user with `@` is a real login
/// (`alice@corp.acme.io`, review I3). Such an entry only says it held a password; its values
/// are never shown.
fn spec_carries_secret(spec: &SshSpec) -> bool {
    spec.hops.iter().any(|h| {
        host_invalid(&h.host)
            || h.user.contains(':')
            || carries_secret(&h.host)
            || carries_secret(&h.user)
            || h.identity_files
                .iter()
                .chain(match &h.auth {
                    SshAuth::KeyFile { path } => Some(path),
                    _ => None,
                })
                .any(|p| carries_secret(&p.to_string_lossy()))
    })
}

/// The aliases a tunnel to `alias` goes through: `alias`, then its `ProxyJump` hosts (and
/// theirs), each once.
fn chain_of(cfg: &SshConfig, alias: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut queue = vec![alias.to_string()];
    while let Some(next) = queue.pop() {
        if out.iter().any(|a| a.eq_ignore_ascii_case(&next)) {
            continue;
        }
        if let Some(host) = cfg
            .hosts
            .iter()
            .find(|h| h.alias.eq_ignore_ascii_case(&next))
        {
            queue.extend(host.proxy_jump.iter().map(|j| jump_alias(j)));
        }
        out.push(next);
    }
    out
}

/// The host part of a `ProxyJump` entry: `[ssh://][user@]host[:port]`, `[v6]` in brackets.
fn jump_alias(spec: &str) -> String {
    let rest = match spec.get(..6) {
        Some(scheme) if scheme.eq_ignore_ascii_case("ssh://") => &spec[6..],
        _ => spec,
    };
    let rest = rest.rsplit_once('@').map_or(rest, |(_, host)| host);
    if let Some(v6) = rest.strip_prefix('[') {
        return v6.split_once(']').map_or(v6, |(h, _)| h).to_string();
    }
    match rest.split_once(':') {
        Some((host, _)) if rest.matches(':').count() == 1 => host.to_string(),
        _ => rest.to_string(),
    }
}

/// The alias a config notice is about (`None`: a line of the file).
fn notice_alias(n: &ConfigNotice) -> Option<&str> {
    match n {
        ConfigNotice::MatchSkipped { .. } | ConfigNotice::IncludeTooDeep { .. } => None,
        ConfigNotice::UserKnownHostsIgnored { alias }
        | ConfigNotice::StrictNoMappedToAsk { alias }
        | ConfigNotice::IdentityAgentIgnored { alias }
        | ConfigNotice::ProxyJumpPasswordIgnored { alias } => Some(alias),
    }
}

/// `user@host`, or the host alone; an IPv6 host in brackets.
fn hop_target(h: &HopSpec) -> String {
    let host = bracketed(&h.host);
    if h.user.is_empty() {
        host
    } else {
        format!("{}@{}", h.user, host)
    }
}

/// `host`, in brackets when it holds a `:` (an IPv6 address), as OpenSSH writes it.
fn bracketed(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

/// `path` with the home folder written as `~`.
fn home_relative(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if !home.as_os_str().is_empty() => {
            Path::new("~").join(rest).to_string_lossy().into_owned()
        }
        _ => path.to_string_lossy().into_owned(),
    }
}

// ---- small helpers -------------------------------------------------------------------------------

const DEFAULT_PORT_TEXT: &str = "22";

fn new_hop(user: String) -> HopSpec {
    HopSpec {
        host: String::new(),
        port: DEFAULT_SSH_PORT,
        user,
        auth: SshAuth::Agent,
        identities_only: false,
        identity_files: Vec::new(),
    }
}

/// A typed port: empty is 22; text that is no port is 0, which Validate reports.
fn parse_port(text: &str) -> u16 {
    let text = text.trim();
    if text.is_empty() {
        return DEFAULT_SSH_PORT;
    }
    text.parse::<u16>().unwrap_or(0)
}

/// A jump row's text as host and port: `host`, `host:port`, `[v6]` or `[v6]:port`.
fn split_address(text: &str) -> (String, u16) {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix('[')
        && let Some((host, after)) = rest.split_once(']')
    {
        return match after.strip_prefix(':') {
            Some(port) => (host.to_string(), parse_port(port)),
            None => (host.to_string(), DEFAULT_SSH_PORT),
        };
    }
    match text.split_once(':') {
        Some((host, port)) if text.matches(':').count() == 1 => {
            (host.to_string(), parse_port(port))
        }
        _ => (text.to_string(), DEFAULT_SSH_PORT),
    }
}

/// A jump hop as its row shows it: the port only when it is not 22.
fn hop_address(hop: &HopSpec) -> String {
    if hop.port == DEFAULT_SSH_PORT {
        hop.host.clone()
    } else if hop.host.contains(':') {
        format!("[{}]:{}", hop.host, hop.port)
    } else {
        format!("{}:{}", hop.host, hop.port)
    }
}

/// A host that Validate refuses (`HostInvalid`): whitespace, a control character or URL
/// syntax (the rule of `hacivat_session::pipeline::validate`).
fn host_invalid(host: &str) -> bool {
    host.trim()
        .chars()
        .any(|c| c.is_ascii_whitespace() || c.is_control() || "/\\@?#".contains(c))
}

fn policy_index(policy: HostKeyPolicy) -> usize {
    HOST_KEY_POLICIES
        .iter()
        .position(|p| *p == policy)
        .unwrap_or(0)
}

fn retention_index(retention: Retention) -> usize {
    RETENTIONS
        .iter()
        .position(|r| *r == retention)
        .unwrap_or(RETENTIONS.len() - 1)
}

fn host_key_options() -> Vec<SelectOption> {
    HOST_KEY_POLICIES
        .iter()
        .map(|p| SelectOption {
            label: text_fn(match p {
                HostKeyPolicy::Ask => ids::hv::CONN_SSH_HK_ASK,
                HostKeyPolicy::AcceptNew => ids::hv::CONN_SSH_HK_ACCEPT,
                HostKeyPolicy::Strict => ids::hv::CONN_SSH_HK_STRICT,
            }),
            hint: None,
            disabled: false,
        })
        .collect()
}

/// A text field of the section: a 26 px input (its fill comes from the field box), a tab stop;
/// every field that is not masked takes a pasted connection URL, as the form's do (R21).
fn new_input(
    placeholder: TextFn,
    source: TextSource,
    masked: bool,
    dialog: WeakEntity<ConnectionDialog>,
    cx: &mut App,
) -> Entity<TextInput> {
    let input = cx.new(|cx| {
        let mut input = TextInput::new(cx, placeholder);
        input.set_placeholder_source(source);
        input.set_masked(masked);
        input.set_mono(!masked);
        if !masked {
            input.set_paste_filter(Some(paste_filter(false, dialog)));
        }
        input
    });
    let _ = input.read(cx).focus_handle(cx).tab_stop(true);
    input
}

/// What the import menu says about an entry.
fn resolution_text(cx: &App, r: &Resolution) -> SharedString {
    match r {
        Resolution::Usable {
            target,
            jumps: Some(jumps),
            ..
        } => tr(
            cx,
            ids::hv::CONN_SSH_IMPORT_VIA,
            ids::hv::HvConnSshImportVia {
                target: Text(target.clone()),
                jumps: Text(jumps.clone()),
            },
        ),
        Resolution::Usable {
            target,
            key: Some(key),
            ..
        } => tr(
            cx,
            ids::hv::CONN_SSH_IMPORT_KEY,
            ids::hv::HvConnSshImportKey {
                target: Text(target.clone()),
                key: Text(key.clone()),
            },
        ),
        Resolution::Usable { target, .. } => SharedString::from(target.clone()),
        Resolution::Unusable(issue) => match issue {
            ConfigIssue::ProxyCommand => tr(cx, ids::hv::CONN_SSH_PROXYCMD, NoArgs),
            ConfigIssue::SecretInValue => tr(cx, ids::hv::CONN_SSH_UNUSABLE_SECRET, NoArgs),
            ConfigIssue::UnsupportedToken(token) => tr(
                cx,
                ids::hv::CONN_SSH_UNUSABLE_TOKEN,
                ids::hv::HvConnSshUnusableToken {
                    token: Text(token.clone()),
                },
            ),
            ConfigIssue::IncludeTooDeep => tr(cx, ids::hv::CONN_SSH_UNUSABLE_DEPTH, NoArgs),
            ConfigIssue::NoHostName => tr(cx, ids::hv::CONN_SSH_UNUSABLE_UNKNOWN, NoArgs),
        },
    }
}

/// A line of the import notice.
fn notice_text(cx: &App, n: &ConfigNotice) -> SharedString {
    let alias = |a: &str| Text(a.to_string());
    match n {
        ConfigNotice::MatchSkipped { line } => tr(
            cx,
            ids::hv::CONN_SSH_NOTICE_MATCH,
            ids::hv::HvConnSshNoticeMatch {
                line: Count(*line as u64),
            },
        ),
        ConfigNotice::IncludeTooDeep { line } => tr(
            cx,
            ids::hv::CONN_SSH_NOTICE_INCLUDE,
            ids::hv::HvConnSshNoticeInclude {
                line: Count(*line as u64),
            },
        ),
        ConfigNotice::UserKnownHostsIgnored { alias: a } => tr(
            cx,
            ids::hv::CONN_SSH_NOTICE_KNOWN_HOSTS,
            ids::hv::HvConnSshNoticeKnownHosts { alias: alias(a) },
        ),
        ConfigNotice::StrictNoMappedToAsk { alias: a } => tr(
            cx,
            ids::hv::CONN_SSH_NOTICE_STRICT_NO,
            ids::hv::HvConnSshNoticeStrictNo { alias: alias(a) },
        ),
        ConfigNotice::IdentityAgentIgnored { alias: a } => tr(
            cx,
            ids::hv::CONN_SSH_NOTICE_AGENT,
            ids::hv::HvConnSshNoticeAgent { alias: alias(a) },
        ),
        ConfigNotice::ProxyJumpPasswordIgnored { alias: a } => tr(
            cx,
            ids::hv::CONN_SSH_NOTICE_JUMP_PASSWORD,
            ids::hv::HvConnSshNoticeJumpPassword { alias: alias(a) },
        ),
    }
}

// ---- issues ----------------------------------------------------------------------------------------

/// Where an SSH issue shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Place {
    Host,
    Port,
    User,
    Key,
    Jump(usize),
    Jumps,
}

impl Place {
    fn name(self) -> String {
        match self {
            Place::Host => "ssh-host".to_string(),
            Place::Port => "ssh-port".to_string(),
            Place::User => "ssh-user".to_string(),
            Place::Key => "ssh-key-file".to_string(),
            Place::Jump(i) => format!("ssh-jump-{i}"),
            Place::Jumps => "ssh-jumps".to_string(),
        }
    }
}

/// The stable name of an SSH issue's code (element IDs `ssh-issue-<place>-<code>`).
pub fn issue_slug(code: &IssueCode) -> &'static str {
    match code {
        IssueCode::SshHostMissing => "host-missing",
        IssueCode::SshUserMissing => "user-missing",
        IssueCode::SshTooManyHops => "too-many-hops",
        IssueCode::SshKeyFileMissing => "key-file-missing",
        IssueCode::HostInvalid => "host-invalid",
        IssueCode::PortInvalid => "port-invalid",
        _ => "other",
    }
}

impl SshSection {
    /// Each SSH issue with the places it shows at, each place and code once: beside every hop
    /// it concerns, else beside the SSH host's field for it.
    fn placed_issues(&self) -> Vec<(Place, IssueCode)> {
        let main = self.main_ref();
        let jumps: &[HopSpec] = self.spec.hops.split_last().map_or(&[], |(_, j)| j);
        let mut out: Vec<(Place, IssueCode)> = Vec::new();
        let mut put = |place: Place, code: &IssueCode| {
            if !out.iter().any(|(p, c)| *p == place && c == code) {
                out.push((place, code.clone()));
            }
        };
        for issue in &self.issues {
            let code = &issue.code;
            let (main_has, jump_has, fallback): (bool, Vec<usize>, Place) = match code {
                IssueCode::SshHostMissing => (
                    main.is_none_or(|m| m.host.trim().is_empty()),
                    hops_where(jumps, |h| h.host.trim().is_empty()),
                    Place::Host,
                ),
                IssueCode::HostInvalid => (
                    main.is_some_and(|m| host_invalid(&m.host)),
                    hops_where(jumps, |h| host_invalid(&h.host)),
                    Place::Host,
                ),
                IssueCode::PortInvalid => (
                    main.is_some_and(|m| m.port == 0),
                    hops_where(jumps, |h| h.port == 0),
                    Place::Port,
                ),
                IssueCode::SshUserMissing => (
                    main.is_some_and(|m| m.user.trim().is_empty()),
                    hops_where(jumps, |h| h.user.trim().is_empty()),
                    Place::User,
                ),
                IssueCode::SshKeyFileMissing => {
                    let key = self.main_auth() == AuthKind::Key;
                    (key, Vec::new(), if key { Place::Key } else { Place::Jumps })
                }
                IssueCode::SshTooManyHops => (false, Vec::new(), Place::Jumps),
                _ => continue,
            };
            if main_has {
                put(fallback, code);
            }
            for i in &jump_has {
                put(Place::Jump(*i), code);
            }
            if !main_has && jump_has.is_empty() {
                put(fallback, code);
            }
        }
        out
    }
}

fn hops_where(hops: &[HopSpec], f: impl Fn(&HopSpec) -> bool) -> Vec<usize> {
    hops.iter()
        .enumerate()
        .filter(|(_, h)| f(h))
        .map(|(i, _)| i)
        .collect()
}

fn issue_text(cx: &App, place: Place, code: &IssueCode) -> SharedString {
    let msg = match (place, code) {
        (Place::Jump(_), IssueCode::SshHostMissing) => ids::hv::CONN_SSH_JUMP_HOST_MISSING,
        // A jump row has no key field: say where its key comes from (review M5).
        (Place::Jumps, IssueCode::SshKeyFileMissing) => ids::hv::CONN_SSH_JUMP_KEY_MISSING,
        (_, IssueCode::SshHostMissing) => ids::hv::CONN_ISSUE_SSH_HOST_MISSING,
        (_, IssueCode::SshUserMissing) => ids::hv::CONN_ISSUE_SSH_USER_MISSING,
        (_, IssueCode::SshTooManyHops) => ids::hv::CONN_ISSUE_SSH_TOO_MANY_HOPS,
        (_, IssueCode::SshKeyFileMissing) => ids::hv::CONN_ISSUE_SSH_KEY_FILE_MISSING,
        (_, IssueCode::HostInvalid) => ids::hv::CONN_ISSUE_HOST_INVALID,
        _ => ids::hv::CONN_ISSUE_PORT_INVALID,
    };
    tr(cx, msg, NoArgs)
}

// ---- drawing ---------------------------------------------------------------------------------------

/// The section's elements with the form's spacing: 8 px between rows, 4 px above a hint.
struct Rows(Vec<(bool, AnyElement)>);

impl Rows {
    fn row(&mut self, el: impl IntoElement) {
        self.0.push((false, el.into_any_element()));
    }

    fn hint(&mut self, el: impl IntoElement) {
        self.0.push((true, el.into_any_element()));
    }

    fn into_column(self) -> Div {
        let mut column = div().flex().flex_col().w_full();
        for (i, (hint, el)) in self.0.into_iter().enumerate() {
            let top = match (i, hint) {
                (0, _) => 0.0,
                (_, true) => 4.0,
                _ => 8.0,
            };
            column = column.child(div().w_full().mt(px(top)).child(el));
        }
        column
    }
}

/// The well behind the section's fields, with the import tint fading back to it.
fn field_box(
    s: &SshSection,
    field: SshField,
    input: &Entity<TextInput>,
    name: String,
    theme: &Theme,
    window: &mut Window,
    cx: &App,
) -> Div {
    let t = theme.tokens;
    let c = &t.colors;
    let well = match theme.mode {
        ThemeMode::Dark => c.panel,
        ThemeMode::Light => c.ground,
    }
    .to_hsla();
    let selector = format!("ssh-field-{name}");
    let base = div()
        .debug_selector(move || selector)
        .w_full()
        .min_w_0()
        .rounded(px(t.radii.sm));
    match s.tint_progress(field, cx) {
        None => base.bg(well).child(input.clone()),
        Some(progress) => {
            if progress > 0.0 && progress < 1.0 {
                window.request_animation_frame();
            }
            let mut under = well;
            under.a *= progress;
            let mut over = c.blue_tint.to_hsla();
            over.a *= 1.0 - progress;
            base.bg(under).child(
                div()
                    .w_full()
                    .rounded(px(t.radii.sm))
                    .bg(over)
                    .child(input.clone()),
            )
        }
    }
}

/// An issue under its field: the alert glyph and the sentence in ink (no new hue).
fn issue_row(theme: &Theme, id: String, text: SharedString) -> Div {
    let t = theme.tokens;
    let c = &t.colors;
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
                    text,
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text.to_hsla(),
                )
                .id(SharedString::from(id)),
            ),
    )
}

/// A hint under a row: a 12 px icon and a 12 px `text-2` line.
fn note(theme: &Theme, icon: IconName, text: SharedString, id: &'static str) -> Div {
    let t = theme.tokens;
    let c = &t.colors;
    let selector = id.to_string();
    hint_row(
        theme,
        div()
            .debug_selector(move || selector)
            .flex()
            .flex_row()
            .items_start()
            .gap(px(t.spacing.sm))
            .flex_1()
            .min_w_0()
            .child(
                // Level with the first line of the note.
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(t.type_ramp.caption.line_height))
                    .child(
                        Icon::new(icon)
                            .size(IconSize::Chrome)
                            .color(c.text_2.to_hsla()),
                    ),
            )
            .child(
                // Wrapping prose: a long locale never clips the note (review M4).
                div().flex_1().min_w_0().child(
                    paragraph(
                        text,
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id(id),
                ),
            ),
    )
}

/// A form row whose label sits level with the first line of a taller control (the jump list).
fn top_row(theme: &Theme, label: SharedString, control: impl IntoElement) -> Div {
    let t = theme.tokens;
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(t.spacing.xl))
        .w_full()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .w(px(FORM_LABEL_PX))
                .h(px(t.spacing.field_height))
                .child(
                    Label::new(
                        label,
                        TextSource::Fluent,
                        t.type_ramp.body,
                        t.colors.text_2.to_hsla(),
                    )
                    .truncate(true),
                ),
        )
        .child(div().flex().flex_col().flex_1().min_w_0().child(control))
}

/// The section for the dialog's `draft`.
fn render(
    state: &Entity<SshSection>,
    draft: &ProfileDraft,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = *Theme::get(cx);
    let s = state.read(cx);
    let summary = summary(cx, &draft.profile);
    let open = s.open;
    let root = s.focus.root.clone();
    let content = if open {
        Some(render_content(state, s, &draft.profile, &theme, window, cx))
    } else {
        None
    };
    let toggle = state.downgrade();
    let mut disclosure = Disclosure::new("conn-ssh", tr(cx, ids::hv::CONN_SECTION_SSH, NoArgs))
        .summary(summary)
        .open(open)
        .on_toggle(move |open, _, cx| {
            let _ = toggle.update(cx, |s, cx| s.set_open(open, cx));
        });
    if let Some(content) = content {
        disclosure = disclosure.content(content);
    }
    div()
        .debug_selector(|| "conn-ssh-section".to_string())
        .track_focus(&root)
        .child(disclosure)
        .into_any_element()
}

/// The folded section's summary (NOTES surface 1: a folded section still tells the truth).
pub(crate) fn summary(cx: &App, p: &Profile) -> SharedString {
    let TransportSpec::SshTunnel(spec) = &p.transport else {
        return tr(cx, ids::hv::CONN_SSH_SUM_OFF, NoArgs);
    };
    let host = spec
        .hops
        .last()
        .map(|h| h.host.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| ROUTE_GAP_MARK.to_string());
    let jumps = spec.hops.len().saturating_sub(1);
    if jumps == 0 {
        tr(
            cx,
            ids::hv::CONN_SSH_SUM_ON,
            ids::hv::HvConnSshSumOn { host: Text(host) },
        )
    } else {
        tr(
            cx,
            ids::hv::CONN_SSH_SUM_HOPS,
            ids::hv::HvConnSshSumHops {
                host: Text(host),
                n: Count(jumps as u64),
            },
        )
    }
}

fn render_content(
    state: &Entity<SshSection>,
    s: &SshSection,
    p: &Profile,
    theme: &Theme,
    window: &mut Window,
    cx: &App,
) -> AnyElement {
    let t = theme.tokens;
    let c = &t.colors;
    let weak = state.downgrade();
    let placed = s.placed_issues();
    let issues_at = |rows: &mut Rows, place: Place| {
        for (_, code) in placed.iter().filter(|(at, _)| *at == place) {
            let id = format!("ssh-issue-{}-{}", place.name(), issue_slug(code));
            rows.hint(issue_row(theme, id, issue_text(cx, place, code)));
        }
    };
    let mut rows = Rows(Vec::new());

    // The checkbox and Import.
    let on_toggle = weak.clone();
    let checkbox = Checkbox::new("ssh-use", tr(cx, ids::hv::CONN_SSH_USE, NoArgs))
        .checked(s.on)
        .disabled(s.read_only)
        .focus_handle(s.focus.use_tunnel.clone())
        .on_toggle(move |on, window, cx| {
            let _ = on_toggle.update(cx, |s, cx| s.set_on(on, window, cx));
        });
    let on_import = weak.clone();
    let import = Button::new("ssh-import", tr(cx, ids::hv::CONN_SSH_IMPORT, NoArgs))
        .focus_handle(s.focus.import.clone())
        .on_raised(true)
        .disabled(s.read_only)
        .on_click(move |_, window, cx| {
            let _ = on_import.update(cx, |s, cx| s.open_import(window, cx));
        });
    let measure = weak.clone();
    let mut import_cell = div().relative().flex_none().ml_auto().child(import).child(
        canvas(
            move |bounds, _, cx| {
                let _ = measure.update(cx, |s, _| s.import_bounds = Some(bounds));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    );
    if let Some(menu) = &s.import {
        let place = match s.import_bounds {
            Some(bounds) => anchored()
                .attach(bounds, Anchor::BottomLeft)
                .offset(point(px(0.0), px(MENU_GAP_PX))),
            None => anchored(),
        };
        import_cell = import_cell.child(deferred(place.child(menu.menu.clone())).with_priority(1));
    }
    rows.hint(hint_row(
        theme,
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(t.spacing.md))
            .w_full()
            .child(checkbox)
            .child(import_cell),
    ));
    if let Some(n) = &s.notice {
        let mut notice = InlineNotice::new(NoticeIcon::Done).line(tr(
            cx,
            ids::hv::CONN_SSH_IMPORTED,
            ids::hv::HvConnSshImported {
                alias: Text(n.alias.clone()),
            },
        ));
        for line in &n.notices {
            notice = notice.secondary(notice_text(cx, line));
        }
        rows.row(notice);
    }
    if !s.on {
        return rows.into_column().into_any_element();
    }

    // SSH host and port, SSH user.
    let host = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.md))
        .w_full()
        .child(div().flex_1().min_w_0().child(field_box(
            s,
            SshField::Host,
            &s.host,
            "ssh-host".to_string(),
            theme,
            window,
            cx,
        )))
        .child(div().flex_none().child(Label::new(
            tr(cx, ids::hv::CONN_DLG_PORT, NoArgs),
            TextSource::Fluent,
            t.type_ramp.body,
            c.text_2.to_hsla(),
        )))
        .child(div().flex_none().w(px(PORT_PX)).child(field_box(
            s,
            SshField::Port,
            &s.port,
            "ssh-port".to_string(),
            theme,
            window,
            cx,
        )));
    rows.row(form_row(
        theme,
        tr(cx, ids::hv::CONN_SSH_HOST, NoArgs),
        host,
    ));
    issues_at(&mut rows, Place::Host);
    issues_at(&mut rows, Place::Port);
    rows.row(form_row(
        theme,
        tr(cx, ids::hv::CONN_SSH_USER, NoArgs),
        field_box(
            s,
            SshField::User,
            &s.user,
            "ssh-user".to_string(),
            theme,
            window,
            cx,
        ),
    ));
    issues_at(&mut rows, Place::User);

    // The sign-in method and its fields.
    let auth = s.main_auth();
    let read_only = s.read_only;
    let segments = AuthKind::ALL.iter().fold(
        Segmented::new("ssh-auth").in_well(true).inert(read_only),
        |seg, &kind| {
            let label = tr(cx, kind.label(), NoArgs);
            let pick = weak.clone();
            seg.item(SegmentItem {
                content: SegmentContent::Label(label.clone()),
                pressed: auth == kind,
                tooltip: label,
                a11y_name: None,
                on_click: Rc::new(move |_, window, cx| {
                    let _ = pick.update(cx, |s, cx| s.set_auth(kind, window, cx));
                }),
            })
        },
    );
    rows.row(form_row(
        theme,
        tr(cx, ids::hv::CONN_DLG_AUTH, NoArgs),
        div()
            .track_focus(&s.focus.auth)
            .flex()
            .min_w_0()
            .child(segments),
    ));
    let secret_row =
        |field: SshField, input: &Entity<TextInput>, name: &str, window: &mut Window| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.md))
                .w_full()
                .child(div().flex_1().min_w_0().child(field_box(
                    s,
                    field,
                    input,
                    name.to_string(),
                    theme,
                    window,
                    cx,
                )))
                .child(div().flex_none().child(Label::new(
                    tr(cx, ids::hv::CONN_DLG_REMEMBER, NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )))
                .child(
                    div()
                        .flex_none()
                        .w(px(REMEMBER_PX))
                        .child(s.remember.clone()),
                )
        };
    match auth {
        AuthKind::Agent => rows.hint(note(
            theme,
            IconName::FileKey,
            tr(cx, ids::hv::CONN_SSH_AGENT_NOTE, NoArgs),
            "ssh-agent-note",
        )),
        AuthKind::Key => {
            let choose = weak.clone();
            let key = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.md))
                .w_full()
                .child(div().flex_1().min_w_0().child(field_box(
                    s,
                    SshField::Key,
                    &s.key,
                    "ssh-key-file".to_string(),
                    theme,
                    window,
                    cx,
                )))
                .child(
                    Button::new("ssh-choose-key", tr(cx, ids::hv::CONN_DLG_CHOOSE, NoArgs))
                        .focus_handle(s.focus.choose_key.clone())
                        .disabled(read_only)
                        .on_raised(true)
                        .on_click(move |_, window, cx| {
                            let _ = choose.update(cx, |s, cx| s.choose_key(window, cx));
                        }),
                );
            rows.row(form_row(theme, tr(cx, ids::hv::CONN_SSH_KEY, NoArgs), key));
            issues_at(&mut rows, Place::Key);
            let row = secret_row(
                SshField::Passphrase,
                &s.passphrase,
                "ssh-passphrase",
                window,
            );
            rows.row(form_row(
                theme,
                tr(cx, ids::hv::CONN_SSH_PASSPHRASE, NoArgs),
                row,
            ));
        }
        AuthKind::Password => {
            let row = secret_row(SshField::Password, &s.password, "ssh-password", window);
            rows.row(form_row(
                theme,
                tr(cx, ids::hv::CONN_DLG_PASSWORD, NoArgs),
                row,
            ));
        }
        AuthKind::Kbd => rows.hint(note(
            theme,
            IconName::Info,
            tr(cx, ids::hv::CONN_SSH_KBD_NOTE, NoArgs),
            "ssh-kbd-note",
        )),
    }

    // The jump hosts, in order.
    let mut list = div()
        .flex()
        .flex_col()
        .gap(px(JUMP_GAP_PX))
        .w_full()
        .min_w_0();
    for (i, row) in s.jumps.iter().enumerate() {
        let id = row.id;
        let keys = weak.clone();
        let remove = weak.clone();
        let selector = format!("ssh-jump-row-{i}");
        let number = (i + 1).to_string();
        list = list.child(
            div()
                .debug_selector(move || selector)
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    let moved = keys
                        .update(cx, |s, cx| s.row_key(id, event, window, cx))
                        .unwrap_or(false);
                    if moved {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                })
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.md))
                .w_full()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .justify_end()
                        .w(px(JUMP_INDEX_PX))
                        .child(Label::new(
                            number,
                            TextSource::Data,
                            t.type_ramp.mono_micro,
                            c.text_2.to_hsla(),
                        )),
                )
                .child(div().flex_1().min_w_0().child(field_box(
                    s,
                    SshField::JumpHost(id),
                    &row.host,
                    format!("ssh-jump-host-{i}"),
                    theme,
                    window,
                    cx,
                )))
                .child(div().flex_none().w(px(JUMP_USER_PX)).child(field_box(
                    s,
                    SshField::JumpUser(id),
                    &row.user,
                    format!("ssh-jump-user-{i}"),
                    theme,
                    window,
                    cx,
                )))
                .child(
                    Button::new(
                        ("ssh-jump-remove", i),
                        tr(
                            cx,
                            ids::hv::CONN_SSH_REMOVE_JUMP,
                            ids::hv::HvConnSshRemoveJump {
                                n: Count(i as u64 + 1),
                            },
                        ),
                    )
                    .kind(ButtonKind::Icon)
                    .icon(IconName::X)
                    .focus_handle(row.remove.clone())
                    .disabled(read_only)
                    .on_click(move |_, window, cx| {
                        let _ = remove.update(cx, |s, cx| s.remove_jump(id, window, cx));
                    }),
                ),
        );
        let mut row_issues = Rows(Vec::new());
        issues_at(&mut row_issues, Place::Jump(i));
        if !row_issues.0.is_empty() {
            list = list.child(row_issues.into_column());
        }
    }
    let add = weak.clone();
    list = list.child(
        div().child(
            Button::new("ssh-add-jump", tr(cx, ids::hv::CONN_SSH_ADD_JUMP, NoArgs))
                .kind(ButtonKind::Ghost)
                .icon(IconName::Plus)
                .focus_handle(s.focus.add_jump.clone())
                .disabled(read_only)
                .on_click(move |_, window, cx| {
                    let _ = add.update(cx, |s, cx| s.add_jump(window, cx));
                }),
        ),
    );
    rows.row(top_row(
        theme,
        tr(cx, ids::hv::CONN_SSH_JUMPS, NoArgs),
        list,
    ));
    issues_at(&mut rows, Place::Jumps);

    // The route: this computer, the jump hosts, the SSH host, the database.
    rows.row(top_row(
        theme,
        tr(cx, ids::hv::CONN_SSH_ROUTE, NoArgs),
        render_route(s, p, theme, cx),
    ));

    // Host keys.
    rows.row(form_row(
        theme,
        tr(cx, ids::hv::CONN_SSH_HOSTKEYS, NoArgs),
        div()
            .flex_none()
            .w(px(HOST_KEYS_PX))
            .max_w_full()
            .child(s.host_keys.clone()),
    ));
    rows.into_column().into_any_element()
}

/// The route row: every stop whole, wrapping onto the next line before any stop is cut (the
/// Overflow Rule); only a stop wider than the whole row ellipsizes.
fn render_route(s: &SshSection, p: &Profile, theme: &Theme, cx: &App) -> Div {
    let t = theme.tokens;
    let c = &t.colors;
    let os = if cfg!(target_os = "macos") {
        ids::hv::HvConnSshThisMacOs::Mac
    } else {
        ids::hv::HvConnSshThisMacOs::Other
    };
    let mut stops: Vec<(SharedString, TextSource)> = vec![(
        tr(
            cx,
            ids::hv::CONN_SSH_THIS_MAC,
            ids::hv::HvConnSshThisMac { os },
        ),
        TextSource::Fluent,
    )];
    let shown = |host: &str| {
        let host = host.trim();
        if host.is_empty() {
            ROUTE_GAP_MARK.to_string()
        } else {
            bracketed(host)
        }
    };
    for hop in &s.spec.hops {
        stops.push((shown(&hop.host).into(), TextSource::Data));
    }
    stops.push((database_stop(p).into(), TextSource::Data));
    let mut route = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_x(px(ROUTE_GAP_PX))
        .w_full()
        .min_w_0();
    // Every line is a field tall, so a wrapped route keeps its label level with the first.
    let line = || {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(ROUTE_GAP_PX))
            .h(px(t.spacing.field_height))
    };
    for (k, (text, source)) in stops.into_iter().enumerate() {
        let (style, ink) = if k == 0 {
            (t.type_ramp.caption, c.text_2)
        } else {
            (t.type_ramp.mono_data, c.text)
        };
        // An arrow travels with the stop after it: a wrapped line never ends on an arrow.
        let mut item = line().min_w_0().max_w_full();
        if k > 0 {
            let arrow = format!("ssh-route-arrow-{k}");
            item = item.child(
                div()
                    .debug_selector(move || arrow)
                    .flex_none()
                    .child(Label::new(
                        ROUTE_ARROW,
                        TextSource::Data,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )),
            );
        }
        let stop = format!("ssh-route-stop-{k}");
        route = route.child(
            item.child(
                div().debug_selector(move || stop).min_w_0().child(
                    Label::new(text, source, style, ink.to_hsla())
                        .truncate(true)
                        .id(SharedString::from(format!("ssh-route-{k}"))),
                ),
            ),
        );
    }
    route
}

/// The database end of the route: `host:port` (the engine's default port when none is typed).
fn database_stop(p: &Profile) -> String {
    let default_port = || {
        EngineCatalog::embedded()
            .get(p.engine)
            .and_then(|e| e.default_port)
    };
    let (host, port) = match &p.endpoint {
        ProfileEndpoint::Server { host, port } => {
            (host.trim().to_string(), port.or_else(default_port))
        }
        ProfileEndpoint::Socket { path, .. } => (path.to_string_lossy().into_owned(), None),
        ProfileEndpoint::File { path, .. } => (path.to_string_lossy().into_owned(), None),
    };
    let host = if host.is_empty() {
        ROUTE_GAP_MARK.to_string()
    } else {
        host
    };
    match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_jump_address_takes_an_optional_port() {
        assert_eq!(
            split_address("jump-eu.acme.io"),
            ("jump-eu.acme.io".into(), 22)
        );
        assert_eq!(split_address(" j:2222 "), ("j".into(), 2222));
        assert_eq!(split_address("j:abc"), ("j".into(), 0));
        assert_eq!(split_address("[fe80::1]:2200"), ("fe80::1".into(), 2200));
        assert_eq!(split_address("[fe80::1]"), ("fe80::1".into(), 22));
        assert_eq!(split_address("fe80::1"), ("fe80::1".into(), 22));
        for hop in [
            ("j", 22, "j"),
            ("j", 2222, "j:2222"),
            ("fe80::1", 2200, "[fe80::1]:2200"),
        ] {
            let spec = HopSpec {
                port: hop.1,
                ..new_hop(String::new())
            };
            let spec = HopSpec {
                host: hop.0.to_string(),
                ..spec
            };
            assert_eq!(hop_address(&spec), hop.2);
            assert_eq!(
                split_address(&hop_address(&spec)),
                (hop.0.to_string(), hop.1)
            );
        }
    }

    #[test]
    fn a_port_field_reads_empty_as_22_and_junk_as_0() {
        assert_eq!(parse_port(""), 22);
        assert_eq!(parse_port("2222"), 2222);
        assert_eq!(parse_port("70000"), 0);
        assert_eq!(parse_port("x"), 0);
    }

    #[test]
    fn a_jump_alias_drops_the_user_scheme_and_port() {
        for (spec, alias) in [
            ("jump", "jump"),
            ("deploy@jump:2222", "jump"),
            ("ssh://deploy@jump", "jump"),
            ("[fe80::1]:22", "fe80::1"),
        ] {
            assert_eq!(jump_alias(spec), alias, "{spec}");
        }
    }

    #[test]
    fn only_hop_issues_are_the_sections() {
        let issue = |field, code| ValidationIssue { field, code };
        assert!(is_ssh_issue(&issue(None, IssueCode::HostInvalid)));
        assert!(is_ssh_issue(&issue(None, IssueCode::PortInvalid)));
        assert!(!is_ssh_issue(&issue(
            Some(hacivat_driver::url::FieldId::Host),
            IssueCode::HostInvalid
        )));
        assert!(!is_ssh_issue(&issue(None, IssueCode::TransportUnavailable)));
        assert!(is_ssh_issue(&issue(None, IssueCode::SshTooManyHops)));
    }

    /// Review M8: the section claims field-less `HostInvalid` / `PortInvalid` as a hop's. Pins
    /// that Validate gives the database host's and port's their field.
    #[test]
    fn the_database_hosts_issues_carry_their_field() {
        let profile = Profile {
            name: "x".into(),
            endpoint: ProfileEndpoint::Server {
                host: "db host/with@url".into(),
                port: Some(0),
            },
            ..Profile::default()
        };
        let issues = hacivat_session::pipeline::validate(&profile, EngineCatalog::embedded());
        let codes: Vec<_> = issues
            .iter()
            .filter(|i| matches!(i.code, IssueCode::HostInvalid | IssueCode::PortInvalid))
            .collect();
        assert_eq!(codes.len(), 2, "{issues:?}");
        assert!(codes.iter().all(|i| i.field.is_some() && !is_ssh_issue(i)));
    }

    #[test]
    fn only_a_tunnel_takes_the_typed_secrets() {
        let mut draft = ProfileDraft::new();
        draft.ssh_secrets = vec![HopSecrets {
            password: Some(SecretString::from("hv_test_pw")),
            passphrase: None,
        }];
        assert!(secrets_for(&draft.profile, &draft).is_empty());
        let mut tunnel = draft.profile.clone();
        tunnel.transport = TransportSpec::SshTunnel(SshSpec {
            hops: vec![new_hop("deploy".into())],
            ..SshSpec::default()
        });
        let secrets = secrets_for(&tunnel, &draft);
        assert_eq!(secrets.len(), 1);
        assert!(secrets[0].password.is_some() && secrets[0].passphrase.is_none());
    }

    #[test]
    fn credential_shaped_hops_are_refused() {
        let spec = |host: &str, user: &str| SshSpec {
            hops: vec![HopSpec {
                host: host.into(),
                ..new_hop(user.into())
            }],
            ..SshSpec::default()
        };
        assert!(!spec_carries_secret(&spec("h.acme.io", "deploy")));
        assert!(!spec_carries_secret(&spec("fe80::1", "deploy")));
        assert!(spec_carries_secret(&spec("admin:pw@h.acme.io", "deploy")));
        assert!(spec_carries_secret(&spec("h acme", "deploy")));
        assert!(spec_carries_secret(&spec("h.acme.io", "admin:pw")));
        // A UPN-style login is a real user (review I3).
        assert!(!spec_carries_secret(&spec(
            "h.acme.io",
            "alice@corp.acme.io"
        )));
        assert!(spec_carries_secret(&spec("h.acme.io", "alice@corp:pw")));
        let v6 = HopSpec {
            host: "2001:db8::2".into(),
            ..new_hop("ops".into())
        };
        assert_eq!(hop_target(&v6), "ops@[2001:db8::2]");
    }

    #[test]
    fn home_paths_show_with_a_tilde() {
        let home = Path::new("/home/u");
        assert_eq!(
            home_relative(Path::new("/home/u/.ssh/id_ed25519"), home),
            Path::new("~").join(".ssh/id_ed25519").to_string_lossy()
        );
        assert_eq!(home_relative(Path::new("/keys/k"), home), "/keys/k");
    }
}
