//! The attach dialog (HV-SCRIPT-01, R-M4b-8, R-M4b-21, Q6; NOTES surface 3): a chosen folder,
//! its logical database and aliases, and the exact `.hacivat/workspace.toml` text and path it
//! will write (or the file it found, which is read and never rewritten).
//!
//! ## m4b-t01b-seams: nothing yet (Task 16).
//!
//! ## m4b-t16-attach
//! - **Ways in.** "Attach script library…" on a database node (prefilled with that database,
//!   [`Origin`]), the palette's `scripts.attach`, Settings › Script libraries' "Attach a folder…"
//!   and the mapping dialog's "Attach another folder…": the platform folder picker ("Choose a
//!   folder of scripts"), then this 480 px modal layer.
//! - **What it shows is what it writes.** The folder is read on the core runtime
//!   (`manifest::preview`, the no-link reader, and `envscope::root_env_folder`) before anything
//!   is offered. With no manifest there, the inset shows `ManifestV1::to_toml()` of the typed
//!   name and aliases, line by line, and Attach hands that very `ManifestV1` to
//!   `LibraryService::attach_writing`, which writes its `to_toml()` byte for byte and never
//!   over a file that appeared meanwhile (then the dialog reads again and shows it). The type
//!   has exactly `format`, `database` and `aliases` (L14): no connection, host, user or
//!   password can reach the folder from here.
//! - **A found file** is shown read-only (comments included) and never rewritten: Attach
//!   registers the folder (`attach(path, None, false)`).
//! - **Refused up front:** a folder inside an environment folder (`@production/…`, carries T4),
//!   a folder already attached, 32 libraries; the service's own refusals (a link, the home
//!   folder, an app folder, a nested library) come back as an ink line under the fields.
//! - **Untrusted text** (the path, the found file, the binding it names) is shown through
//!   `escape_untrusted` (bidi and invisible characters as `<U+XXXX>`; T3 fr1).
//! - **Keys** (NOTES surface 3): Tab is trapped in the card (Change… → Database → Aliases →
//!   Cancel → Attach; found: Change… → Cancel → Attach), every button is a Tab stop, the
//!   header's × is not; Escape (a field's `Cancelled` too) cancels; Enter presses the focused
//!   button, and in a field attaches. The layer is a focus home (`shell::layers`); closing it
//!   gives the focus back to where it was.

use std::path::{Path, PathBuf};

use hacivat_l10n::ArgValue;
use hacivat_proto::ProfileId;
use hacivat_scripts::manifest::{self, Diag as ManifestDiag, ManifestPreview, ManifestV1};
use hacivat_scripts::names::escape_untrusted;
use hacivat_scripts::service::AttachError;
use hacivat_scripts::{LogicalDb, NameError, RegistryError};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::env_swatch;
use hacivat_ui::dialog::{DialogSize, dialog_frame};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::l10n::ui_text;
use hacivat_ui::popover::{A11yNode, Declare as _};
use hacivat_ui::text::{Label, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Div, Entity, FocusHandle, Focusable, Global,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _,
    PathPromptOptions, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};

use super::projection;
use crate::connections::ConnectionsModel;
use crate::l10n_global::tr_id;
use crate::shell::Workspace;
use crate::shell::layers::{DIALOG_CONTEXT, Layer};

/// The dialog's layer ID.
pub const ATTACH_LAYER: &str = "scripts-attach";

/// How many characters of one untrusted line are shown (a manifest is at most 64 KiB).
const SHOWN_CHARS: usize = 512;
/// ## m4b-t16-attach fix round 1 (m1): how many lines of a file the inset draws.
const MAX_SHOWN_LINES: usize = 200;

/// The database node a dialog was opened from (its exact name on the server).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub profile: ProfileId,
    pub database: String,
}

fn text(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    crate::explorer::live::text(cx, id, args)
}

fn arg(s: &str) -> ArgValue {
    ArgValue::Text(s.to_string())
}

/// A path for display: escaped, never trusted.
pub(crate) fn shown_path(path: &Path) -> String {
    escape_untrusted(&path.display().to_string(), SHOWN_CHARS)
}

// ---- opening ---------------------------------------------------------------------------------

/// "Attach script library…": the folder picker, then the dialog for the chosen folder. A
/// cancelled picker opens nothing.
pub fn start(origin: Option<Origin>, window: &mut Window, cx: &mut App) {
    start_with(origin, Prefill::Node, window, cx);
}

// ## m4b-t16-attach fix round 1 (I5, R-T16-1)
/// Where the Database field's first value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefill {
    /// The database node it was opened from (the menu's "Attach script library…").
    Node,
    /// The chosen folder's name, never the node's: the mapping dialog's "Attach another
    /// folder…" is opened from a node whose name is local by construction (Q5), and a local
    /// name reaches a team's `workspace.toml` only when the user types it.
    Folder,
}

/// [`start`] with the Database field prefilled from `prefill`.
pub fn start_with(origin: Option<Origin>, prefill: Prefill, window: &mut Window, cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(tr_id(cx, "hv-scripts-attach-choose")),
    });
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = picked.await else {
            return;
        };
        let Some(path) = paths.into_iter().next().filter(|p| p.is_absolute()) else {
            return;
        };
        let _ = handle.update(cx, |_, window, cx| {
            open_at_with(path, origin, prefill, window, cx)
        });
    })
    .detach();
}

/// The dialog for `path` (the picker's answer; shots and tests skip the picker).
pub fn open_at(path: PathBuf, origin: Option<Origin>, window: &mut Window, cx: &mut App) {
    open_at_with(path, origin, Prefill::Node, window, cx);
}

/// [`open_at`] with the Database field prefilled from `prefill`.
pub fn open_at_with(
    path: PathBuf,
    origin: Option<Origin>,
    prefill: Prefill,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    // One attach dialog at a time.
    if open_dialog(cx).is_some() {
        close(window, cx);
    }
    let dialog = cx.new(|cx| AttachDialog::new(path, origin, prefill, window, cx));
    let root = dialog.read(cx).root.clone();
    ws.update(cx, |ws, cx| {
        ws.push_layer(
            Layer {
                id: ATTACH_LAYER,
                context: DIALOG_CONTEXT,
                view: dialog.clone().into(),
                restore_focus: None,
                modal: true,
            },
            window,
            cx,
        );
    });
    window.focus(&root, cx);
    cx.set_global(OpenDialog(Some(dialog.downgrade())));
    dialog.update(cx, |d, cx| d.read_folder(window, cx));
}

/// The open attach dialog.
struct OpenDialog(Option<WeakEntity<AttachDialog>>);

impl Global for OpenDialog {}

/// The attach dialog while it is open.
pub fn open_dialog(cx: &App) -> Option<Entity<AttachDialog>> {
    cx.try_global::<OpenDialog>()?.0.as_ref()?.upgrade()
}

/// Closes the attach dialog (its layer) when it is open.
fn close(window: &mut Window, cx: &mut App) {
    let (Some(ws), Some(d)) = (window.root::<Workspace>().flatten(), open_dialog(cx)) else {
        return;
    };
    let id = d.entity_id();
    ws.update(cx, |ws, cx| ws.remove_layer(id, window, cx));
    cx.set_global(OpenDialog(None));
}

// ---- what the folder holds ---------------------------------------------------------------------

/// What reading the chosen folder found (on the core runtime).
#[derive(Debug, Clone)]
struct FolderRead {
    canonical: Option<PathBuf>,
    /// The environment folder the folder sits in (display form).
    env_folder: Option<String>,
    preview: ManifestPreview,
}

fn read_folder(path: &Path) -> FolderRead {
    let canonical = std::fs::canonicalize(path).ok();
    let env_folder = canonical
        .as_deref()
        .and_then(hacivat_scripts::envscope::root_env_folder);
    FolderRead {
        canonical,
        env_folder,
        preview: manifest::preview(path),
    }
}

/// What the dialog offers.
#[derive(Debug, Clone)]
enum Found {
    Reading,
    /// No manifest: Attach writes the one shown.
    New,
    /// A manifest is there: read, never rewritten.
    Existing(ManifestPreview),
}

/// Why Attach is off (or failed), as one ink line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Problem {
    /// `hv-scripts-attach-err`'s variant.
    Name(&'static str),
    /// `hv-scripts-attach-aliases-err`'s variant.
    Alias(&'static str),
    /// `hv-scripts-attach-refused`'s variant, and the environment folder it names.
    Refused(&'static str, String),
}

fn name_reason(e: NameError) -> &'static str {
    match e {
        NameError::Empty => "empty",
        NameError::TooLong => "long",
        NameError::Control | NameError::Bidi => "control",
    }
}

/// The typed aliases: comma-separated, trimmed, empty pieces and repeats of the database or of
/// an earlier alias dropped (as `manifest::read` would).
fn parse_aliases(typed: &str, db: Option<&LogicalDb>) -> Result<Vec<LogicalDb>, &'static str> {
    let mut out: Vec<LogicalDb> = Vec::new();
    for piece in typed.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let alias = LogicalDb::parse(piece).map_err(name_reason)?;
        let repeated =
            db.is_some_and(|d| d.same_db(&alias)) || out.iter().any(|a| a.same_db(&alias));
        if !repeated {
            out.push(alias);
        }
    }
    if out.len() > manifest::MAX_ALIASES {
        return Err("many");
    }
    Ok(out)
}

fn refusal(e: &AttachError) -> (&'static str, String) {
    let reason = match e {
        AttachError::RootInEnvFolder(folder) => return ("envfolder", folder.clone()),
        AttachError::RootIsLink => "link",
        AttachError::Registry(r) => match r {
            RegistryError::HomeFolder => "home",
            RegistryError::AppFolder => "app",
            RegistryError::FilesystemRoot => "root",
            RegistryError::Nested(_) => "nested",
            RegistryError::NotAFolder => "notfolder",
            RegistryError::ReadOnly => "readonly",
            RegistryError::AlreadyRegistered(_) => return ("already", String::new()),
            RegistryError::TooMany => return ("many", String::new()),
            _ => "other",
        },
        AttachError::Manifest(manifest::WriteError::Changed) => "changed",
        _ => "other",
    };
    (reason, String::new())
}

// ---- the dialog ----------------------------------------------------------------------------------

/// The attach dialog (NOTES surface 3).
pub struct AttachDialog {
    path: PathBuf,
    origin: Option<Origin>,
    read: Option<FolderRead>,
    found: Found,
    database: Entity<TextInput>,
    aliases: Entity<TextInput>,
    /// The service refused the last Attach (shown until something is typed or changed).
    refused: Option<Problem>,
    busy: bool,
    // ## m4b-t16-attach fix round 1 (I5): the folder name the field was prefilled with (it
    // follows Change… while untouched), and whether the field has been drawn since the folder
    // was read: Enter attaches only what the user has seen.
    folder_prefill: Option<String>,
    shown: bool,
    root: FocusHandle,
    change_focus: FocusHandle,
    cancel_focus: FocusHandle,
    attach_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for AttachDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl AttachDialog {
    fn new(
        path: PathBuf,
        origin: Option<Origin>,
        from: Prefill,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let folder = folder_name(&path);
        let node = origin
            .as_ref()
            .filter(|_| from == Prefill::Node)
            .map(|o| o.database.clone())
            .filter(|d| LogicalDb::parse(d).is_ok());
        let folder_prefill = node.is_none().then(|| folder.clone().unwrap_or_default());
        let prefill = node.or(folder).unwrap_or_default();
        let database = cx.new(|cx| {
            let mut input = TextInput::new(
                cx,
                crate::l10n_global::text_fn_id("hv-scripts-attach-database"),
            );
            input.set_mono(true);
            input.set_tab_stop(true);
            input.set_value(prefill);
            input
        });
        let aliases = cx.new(|cx| {
            let mut input = TextInput::new(
                cx,
                crate::l10n_global::text_fn_id("hv-scripts-attach-aliases-ph"),
            );
            input.set_mono(true);
            input.set_tab_stop(true);
            input
        });
        let mut subs = Vec::new();
        for input in [&database, &aliases] {
            subs.push(
                cx.subscribe_in(input, window, |this, _, e: &InputEvent, window, cx| {
                    match e {
                        InputEvent::Changed => {
                            this.refused = None;
                            cx.notify();
                        }
                        InputEvent::Submitted => this.attach(window, cx),
                        // F15: a field's Escape is the dialog's.
                        InputEvent::Cancelled => close(window, cx),
                        InputEvent::CompositionChanged(_) => {}
                    }
                }),
            );
        }
        AttachDialog {
            path,
            origin,
            read: None,
            found: Found::Reading,
            database,
            aliases,
            refused: None,
            busy: false,
            folder_prefill,
            shown: false,
            root: cx.focus_handle(),
            change_focus: cx.focus_handle().tab_stop(true),
            cancel_focus: cx.focus_handle().tab_stop(true),
            attach_focus: cx.focus_handle().tab_stop(true),
            _subscriptions: subs,
        }
    }

    /// Reads the folder on the core runtime; the dialog shows "Reading the folder…" meanwhile.
    fn read_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.found = Found::Reading;
        self.read = None;
        self.shown = false;
        // I5: an untouched folder-name prefill follows the folder (Change…).
        if let Some(old) = self.folder_prefill.clone()
            && *self.database.read(cx).value() == *old
        {
            let new = folder_name(&self.path).unwrap_or_default();
            self.database.update(cx, |i, cx| {
                i.set_value(new.clone());
                cx.notify();
            });
            self.folder_prefill = Some(new);
        }
        let Some(rt) = cx.try_global::<ConnectionsModel>().map(|m| m.runtime()) else {
            return;
        };
        let path = self.path.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        rt.spawn_blocking(move || {
            let _ = tx.send(read_folder(&path));
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(read) = rx.await else {
                return;
            };
            let _ = this.update_in(cx, |d, window, cx| d.folder_read(read, window, cx));
        })
        .detach();
    }

    fn folder_read(&mut self, read: FolderRead, window: &mut Window, cx: &mut Context<Self>) {
        self.found = if read.preview.present {
            Found::Existing(read.preview.clone())
        } else {
            Found::New
        };
        self.read = Some(read);
        self.busy = false;
        cx.notify();
        // The first stop the dialog offers takes the focus: the database field for a new
        // file, Attach for a found one (nothing is written into the folder then).
        let focus = match (&self.found, self.problem(cx)) {
            (Found::New, Some(Problem::Refused(..))) => self.change_focus.clone(),
            (Found::New, _) => self.database.read(cx).focus_handle(cx),
            _ => self.attach_focus.clone(),
        };
        window.focus(&focus, cx);
    }

    /// The folder has been read.
    pub fn is_ready(&self) -> bool {
        !matches!(self.found, Found::Reading)
    }

    /// A manifest is there (read, never rewritten).
    pub fn found(&self) -> bool {
        matches!(self.found, Found::Existing(_))
    }

    pub fn database_input(&self) -> &Entity<TextInput> {
        &self.database
    }

    pub fn aliases_input(&self) -> &Entity<TextInput> {
        &self.aliases
    }

    /// The manifest Attach writes now (`None`: a file is there, or the name is not valid).
    fn manifest(&self, cx: &App) -> Result<ManifestV1, Problem> {
        let db = LogicalDb::parse(self.database.read(cx).value())
            .map_err(|e| Problem::Name(name_reason(e)))?;
        let aliases =
            parse_aliases(self.aliases.read(cx).value(), Some(&db)).map_err(Problem::Alias)?;
        Ok(ManifestV1::new(db, aliases))
    }

    /// Why Attach is off now, if it is.
    fn problem(&self, cx: &App) -> Option<Problem> {
        let read = self.read.as_ref()?;
        if let Some(folder) = &read.env_folder {
            return Some(Problem::Refused("envfolder", folder.clone()));
        }
        if let Some(p) = &self.refused {
            return Some(p.clone());
        }
        let snap = projection::service(cx).map(|s| s.snapshot());
        if let (Some(snap), Some(canonical)) = (&snap, &read.canonical)
            && snap
                .libraries
                .iter()
                .any(|l| &l.root.canonical == canonical || l.root.path == self.path)
        {
            return Some(Problem::Name("already"));
        }
        if snap.is_some_and(|s| s.libraries.len() >= hacivat_scripts::registry::MAX_LIBRARIES) {
            return Some(Problem::Name("many"));
        }
        match self.found {
            Found::New => self.manifest(cx).err(),
            _ => None,
        }
    }

    /// Whether Attach can be pressed now.
    pub fn can_attach(&self, cx: &App) -> bool {
        self.is_ready()
            && !self.busy
            && self.problem(cx).is_none()
            // I5: a new file only once its fields and text were drawn.
            && (self.shown || !matches!(self.found, Found::New))
    }

    // ## m4b-t16-attach fix round 1 (m6)
    /// Change…, Cancel and Attach's focus handles (tests assert where the focus is).
    pub fn button_handles(&self) -> [FocusHandle; 3] {
        [
            self.change_focus.clone(),
            self.cancel_focus.clone(),
            self.attach_focus.clone(),
        ]
    }

    fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_attach(cx) {
            return;
        }
        let Some(svc) = projection::ensure_service(cx) else {
            return;
        };
        let plan = match &self.found {
            Found::New => match self.manifest(cx) {
                Ok(m) => Some(m),
                Err(_) => return,
            },
            _ => None,
        };
        self.busy = true;
        cx.notify();
        let path = self.path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = match plan {
                Some(m) => svc.attach_writing(path, m).await,
                None => svc.attach(path, None, false).await,
            };
            let _ = this.update_in(cx, |d, window, cx| match result {
                Ok(_) => close(window, cx),
                // A file appeared after the folder was read: show it, write nothing.
                Err(AttachError::Manifest(manifest::WriteError::Exists)) => {
                    d.read_folder(window, cx);
                }
                Err(e) => {
                    let (reason, folder) = refusal(&e);
                    d.refused = Some(match reason {
                        "already" | "many" => Problem::Name(reason),
                        _ => Problem::Refused(reason, folder),
                    });
                    d.busy = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Change…: the picker again; a chosen folder replaces this one.
    fn change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr_id(cx, "hv-scripts-attach-choose")),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next().filter(|p| p.is_absolute()) else {
                return;
            };
            let _ = this.update_in(cx, |d, window, cx| {
                d.path = path;
                d.refused = None;
                d.read_folder(window, cx);
            });
        })
        .detach();
    }

    /// The Tab stops in order (NOTES surface 3).
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        let mut order = vec![self.change_focus.clone()];
        if matches!(self.found, Found::New) {
            order.push(self.database.read(cx).focus_handle(cx));
            order.push(self.aliases.read(cx).focus_handle(cx));
        }
        order.push(self.cancel_focus.clone());
        order.push(self.attach_focus.clone());
        order
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let m = &stroke.modifiers;
        let plain = !(m.control || m.platform || m.alt || m.shift);
        match stroke.key.as_str() {
            "escape" => close(window, cx),
            "enter" if plain && !event.is_held => {
                if self.cancel_focus.is_focused(window) {
                    close(window, cx);
                } else if self.change_focus.is_focused(window) {
                    self.change(window, cx);
                } else {
                    self.attach(window, cx);
                }
            }
            "tab" if !m.control && !m.platform => {
                let order = self.stops(cx);
                let n = order.len();
                let at = order.iter().position(|h| h.is_focused(window));
                let next = match (at, m.shift) {
                    (Some(i), false) => (i + 1) % n,
                    (Some(i), true) => (i + n - 1) % n,
                    (None, false) => 0,
                    (None, true) => n - 1,
                };
                window.focus(&order[next], cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    // ---- drawing ----------------------------------------------------------------------------

    fn subtitle(&self, theme: &Theme, cx: &App) -> Option<AnyElement> {
        let origin = self.origin.as_ref()?;
        let p = cx.try_global::<ConnectionsModel>()?.get(origin.profile)?;
        let t = theme.tokens;
        let line = text(
            cx,
            "hv-scripts-attach-from",
            vec![
                ("conn", arg(&escape_untrusted(&p.name, SHOWN_CHARS))),
                (
                    "database",
                    arg(&escape_untrusted(&origin.database, SHOWN_CHARS)),
                ),
            ],
        );
        Some(
            div()
                .mt(px(-t.spacing.sm))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.sm))
                .child(env_swatch(
                    crate::connections::capsule::env_kind(p.environment),
                    theme,
                ))
                .child(
                    Label::new(
                        line,
                        TextSource::Data,
                        t.type_ramp.body_sm,
                        t.colors.text_2.to_hsla(),
                    )
                    .truncate(true)
                    .id("scripts-attach-subtitle"),
                )
                .into_any_element(),
        )
    }

    fn folder_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        let change = Button::new(
            "scripts-attach-change",
            tr_id(cx, "hv-scripts-attach-change"),
        )
        .focus_handle(self.change_focus.clone())
        .on_click(cx.listener(|this, _, window, cx| this.change(window, cx)));
        div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.xs))
            .child(caption(tr_id(cx, "hv-scripts-attach-folder"), theme))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.md))
                    .child(
                        div().flex_1().min_w_0().child(
                            paragraph(
                                SharedString::from(shown_path(&self.path)),
                                TextSource::Data,
                                t.type_ramp.mono_data,
                                c.text.to_hsla(),
                            )
                            .id("scripts-attach-folder"),
                        ),
                    )
                    .child(div().flex_none().child(change)),
            )
    }

    fn fields(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.md))
            .child(
                div()
                    .debug_selector(|| "scripts-attach-database".to_string())
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.xs))
                    .child(caption(tr_id(cx, "hv-scripts-attach-database"), theme))
                    .child(self.database.clone())
                    .child(
                        paragraph(
                            tr_id(cx, "hv-scripts-attach-database-hint"),
                            TextSource::Fluent,
                            t.type_ramp.caption,
                            c.text_2.to_hsla(),
                        )
                        .id("scripts-attach-database-hint"),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "scripts-attach-aliases".to_string())
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.xs))
                    .child(caption(tr_id(cx, "hv-scripts-attach-aliases"), theme))
                    .child(self.aliases.clone()),
            )
    }

    fn problem_line(&self, theme: &Theme, cx: &App) -> Option<AnyElement> {
        let message = match self.problem(cx)? {
            Problem::Name(reason) => {
                text(cx, "hv-scripts-attach-err", vec![("reason", arg(reason))])
            }
            Problem::Alias(reason) => text(
                cx,
                "hv-scripts-attach-aliases-err",
                vec![("reason", arg(reason))],
            ),
            Problem::Refused(reason, folder) => text(
                cx,
                "hv-scripts-attach-refused",
                vec![("reason", arg(reason)), ("folder", arg(&folder))],
            ),
        };
        Some(ink_line(message, "scripts-attach-error", theme))
    }

    /// What will be written (or what was found), in the inset.
    fn file(&self, theme: &Theme, cx: &App) -> Option<AnyElement> {
        let t = theme.tokens;
        let c = &t.colors;
        let file = self.path.join(".hacivat").join("workspace.toml");
        match &self.found {
            Found::Reading => None,
            Found::New => {
                if matches!(self.problem(cx), Some(Problem::Refused(..)))
                    || self.read.as_ref().is_some_and(|r| r.env_folder.is_some())
                {
                    return None;
                }
                let text = self.manifest(cx).ok()?.to_toml();
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(t.spacing.sm))
                        .child(
                            paragraph(
                                tr_id(cx, "hv-scripts-attach-will-write"),
                                TextSource::Fluent,
                                t.type_ramp.body_sm,
                                c.text_2.to_hsla(),
                            )
                            .id("scripts-attach-will-write"),
                        )
                        .child(file_inset(&file, &text, &[], theme, cx))
                        .child(
                            paragraph(
                                tr_id(cx, "hv-scripts-attach-file-note"),
                                TextSource::Fluent,
                                t.type_ramp.caption,
                                c.text_2.to_hsla(),
                            )
                            .id("scripts-attach-file-note"),
                        )
                        .into_any_element(),
                )
            }
            Found::Existing(preview) => {
                let binds = preview
                    .read
                    .logical(self.path.file_name().and_then(|n| n.to_str()).unwrap_or(""))
                    .map(|l| l.display_escaped());
                // ## m4b-t16-attach fix round 1 (I4): a file that can't be read (or a newer
                // format) says so, and that the folder's name binds until it is fixed.
                let broken = preview
                    .read
                    .diags
                    .iter()
                    .find(|d| {
                        manifest::is_read_failure(d) || matches!(d, ManifestDiag::NewerFormat(_))
                    })
                    .map(reason_word)
                    .or_else(|| preview.text.is_none().then_some("other"));
                let head = match (broken, binds) {
                    (None, Some(db)) => {
                        text(cx, "hv-scripts-attach-found", vec![("database", arg(&db))])
                    }
                    (reason, _) => {
                        let reason = text(
                            cx,
                            "hv-scripts-manifest-reason",
                            vec![("reason", arg(reason.unwrap_or("other")))],
                        );
                        let folder = escape_untrusted(
                            &folder_name(&self.path).unwrap_or_default(),
                            SHOWN_CHARS,
                        );
                        text(
                            cx,
                            "hv-scripts-attach-found-broken",
                            vec![("reason", arg(&reason)), ("folder", arg(&folder))],
                        )
                    }
                };
                let mut notice = div()
                    .debug_selector(|| "scripts-attach-found".to_string())
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.xs))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_start()
                            .gap(px(t.spacing.sm))
                            .child(
                                div().flex_none().pt(px(2.0)).child(
                                    Icon::new(IconName::Info)
                                        .size(IconSize::Chrome)
                                        .color(c.text_2.to_hsla()),
                                ),
                            )
                            .child(
                                div().flex_1().min_w_0().child(
                                    paragraph(
                                        head,
                                        TextSource::Fluent,
                                        t.type_ramp.body_sm,
                                        c.text.to_hsla(),
                                    )
                                    .id("scripts-attach-found-head"),
                                ),
                            ),
                    )
                    .child(
                        paragraph(
                            tr_id(cx, "hv-scripts-attach-found-note"),
                            TextSource::Fluent,
                            t.type_ramp.caption,
                            c.text_2.to_hsla(),
                        )
                        .id("scripts-attach-found-note"),
                    );
                if let Some(found) = &preview.text {
                    notice = notice.child(file_inset(&file, found, &[], theme, cx));
                }
                Some(notice.into_any_element())
            }
        }
    }

    fn shows_under(&self, theme: &Theme, cx: &App) -> Option<AnyElement> {
        let t = theme.tokens;
        let c = &t.colors;
        let names: Vec<LogicalDb> = match &self.found {
            Found::New => {
                let m = self.manifest(cx).ok()?;
                std::iter::once(m.database).chain(m.aliases).collect()
            }
            Found::Existing(p) => {
                let folder = self.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                p.read
                    .logical(folder)
                    .into_iter()
                    .chain(p.read.aliases.iter().cloned())
                    .collect()
            }
            Found::Reading => return None,
        };
        if self.read.as_ref().is_some_and(|r| r.env_folder.is_some()) {
            return None;
        }
        let places = projection::places_binding(&names, cx);
        let model = cx.try_global::<ConnectionsModel>()?;
        let mut list = div()
            .debug_selector(|| "scripts-attach-shows-under".to_string())
            .flex()
            .flex_col()
            .gap(px(t.spacing.xs))
            .child(caption(tr_id(cx, "hv-scripts-attach-shows-under"), theme));
        if places.is_empty() {
            let first = names
                .first()
                .map(LogicalDb::display_escaped)
                .unwrap_or_default();
            list = list.child(
                paragraph(
                    text(
                        cx,
                        "hv-scripts-attach-shows-none",
                        vec![("database", arg(&first))],
                    ),
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                )
                .id("scripts-attach-shows-none"),
            );
        }
        for (i, (profile, database)) in places.iter().enumerate() {
            let Some(p) = model.get(*profile) else {
                continue;
            };
            let line = text(
                cx,
                "hv-scripts-attach-from",
                vec![
                    ("conn", arg(&escape_untrusted(&p.name, SHOWN_CHARS))),
                    ("database", arg(&escape_untrusted(database, SHOWN_CHARS))),
                ],
            );
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.sm))
                    .child(env_swatch(
                        crate::connections::capsule::env_kind(p.environment),
                        theme,
                    ))
                    .child(
                        Label::new(
                            line,
                            TextSource::Data,
                            t.type_ramp.mono_data,
                            c.text.to_hsla(),
                        )
                        .truncate(true)
                        .id(SharedString::from(format!("scripts-attach-shows-{i}"))),
                    ),
            );
        }
        Some(list.into_any_element())
    }
}

impl Render for AttachDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // ## m4b-t16-attach fix round 1 (I5): this frame draws what Enter would attach.
        if matches!(self.found, Found::New) && !self.shown {
            self.shown = true;
            cx.notify();
        }
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let title = tr_id(cx, "hv-scripts-attach-title");
        let close_x = div()
            .id("scripts-attach-close")
            .debug_selector(|| "scripts-attach-close".to_string())
            .declare(Role::Button, ui_text(cx, "hui-dialog-close"), None)
            .flex()
            .items_center()
            .flex_none()
            .p(px(t.spacing.xs))
            .rounded(px(t.radii.sm))
            .cursor_pointer()
            .on_click(cx.listener(|_, _, window, cx| close(window, cx)))
            .child(
                // Raised Ink Rule: text-2 on the raised dialog surface.
                Icon::new(IconName::X)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            );
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.xl))
            .child(self.folder_row(&theme, cx));
        match &self.found {
            Found::Reading => {
                body = body.child(
                    paragraph(
                        tr_id(cx, "hv-scripts-attach-reading"),
                        TextSource::Fluent,
                        t.type_ramp.body_sm,
                        c.text_2.to_hsla(),
                    )
                    .id("scripts-attach-reading"),
                );
            }
            Found::New => {
                let refused_up_front = self.read.as_ref().is_some_and(|r| r.env_folder.is_some());
                if !refused_up_front {
                    body = body.child(self.fields(&theme, cx));
                }
            }
            Found::Existing(_) => {}
        }
        body = body
            .children(self.problem_line(&theme, cx))
            .children(self.file(&theme, cx))
            .children(self.shows_under(&theme, cx));
        let cancel = Button::new("scripts-attach-cancel", tr_id(cx, "hv-tx-cancel"))
            .focus_handle(self.cancel_focus.clone())
            .on_click(cx.listener(|_, _, window, cx| close(window, cx)));
        let attach = Button::new("scripts-attach-ok", tr_id(cx, "hv-scripts-attach-btn"))
            .kind(ButtonKind::Primary)
            .disabled(!self.can_attach(cx))
            .focus_handle(self.attach_focus.clone())
            .on_click(cx.listener(|this, _, window, cx| this.attach(window, cx)));
        let card = dialog_frame(&theme, DialogSize::Wide)
            .id(ATTACH_LAYER)
            .debug_selector(|| ATTACH_LAYER.to_string())
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            // Enter acts on key down only; the focused button's key-up click is suppressed (as
            // in `Dialog`).
            .capture_key_up(|event, window, _| {
                let stroke = &event.keystroke;
                if stroke.key == "enter" && !stroke.modifiers.modified() {
                    window.prevent_default();
                }
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .justify_between()
                    .gap(px(t.spacing.xl))
                    .child(div().flex_1().min_w_0().child(paragraph(
                        title.clone(),
                        TextSource::Fluent,
                        t.type_ramp.body_strong,
                        c.text.to_hsla(),
                    )))
                    .child(close_x),
            )
            .children(self.subtitle(&theme, cx))
            .child(
                div()
                    .id("scripts-attach-body")
                    .flex()
                    .flex_col()
                    .flex_shrink(1.0)
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(hacivat_ui::text::scroll_viewport())
                    .child(body),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(t.spacing.md))
                    .child(cancel)
                    .child(attach),
            );
        A11yNode::new("scripts-attach-node", Role::Dialog, title, true, card)
    }
}

// ---- shared pieces (the mapping dialog draws them too) -----------------------------------------

/// A field caption on the raised card (text-2: the Raised Ink Rule).
pub(crate) fn caption(label: SharedString, theme: &Theme) -> Label {
    Label::new(
        label,
        TextSource::Fluent,
        theme.tokens.type_ramp.caption,
        theme.tokens.colors.text_2.to_hsla(),
    )
}

/// An ink line with the alert glyph (a refusal or a problem: never a new hue).
pub(crate) fn ink_line(message: SharedString, id: &'static str, theme: &Theme) -> AnyElement {
    let t = theme.tokens;
    let ink = t.colors.text.to_hsla();
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(t.spacing.sm))
        .child(
            div().flex_none().pt(px(2.0)).child(
                Icon::new(IconName::CircleAlert)
                    .size(IconSize::Chrome)
                    .color(ink),
            ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(paragraph(message, TextSource::Fluent, t.type_ramp.body_sm, ink).id(id)),
        )
        .into_any_element()
}

/// The file inset (the fingerprint-inset anatomy): the file's full path, then its text line by
/// line, every line escaped; `marked` lines sit on the selection tint (an added line).
pub(crate) fn file_inset(
    path: &Path,
    text: &str,
    marked: &[usize],
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    let t = theme.tokens;
    let c = &t.colors;
    let well = match theme.mode {
        ThemeMode::Dark => c.panel,
        ThemeMode::Light => c.ground,
    };
    let inset = div()
        .debug_selector(|| "scripts-file-inset".to_string())
        .flex()
        .flex_col()
        .gap(px(t.spacing.xs))
        .py(px(t.spacing.md))
        .px(px(t.spacing.lg))
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .rounded(px(t.radii.sm))
        .bg(well.to_hsla())
        // The whole path, wrapped: where the file goes is never cut.
        .child(
            paragraph(
                SharedString::from(shown_path(path)),
                TextSource::Data,
                t.type_ramp.mono_data,
                c.text_2.to_hsla(),
            )
            .id("scripts-attach-file-path"),
        );
    let mut lines = div().flex().flex_col();
    // ## m4b-t16-attach fix round 1 (m1): at most `MAX_SHOWN_LINES` lines are drawn (a found
    // file is only read, so the rest is never written); the count of the others is said.
    let total = text.lines().count();
    for (i, line) in text.lines().enumerate().take(MAX_SHOWN_LINES) {
        let mut row = div().px(px(t.spacing.xs)).rounded(px(t.radii.sm));
        if marked.contains(&i) {
            row = row.bg(c.blue_tint.to_hsla());
        }
        lines = lines.child(
            row.child(
                Label::new(
                    SharedString::from(escape_untrusted(line, SHOWN_CHARS)),
                    TextSource::Data,
                    t.type_ramp.mono_data,
                    c.text.to_hsla(),
                )
                .truncate(true)
                .id(SharedString::from(format!("scripts-attach-file-line-{i}"))),
            ),
        );
    }
    if total > MAX_SHOWN_LINES {
        let more = u64::try_from(total - MAX_SHOWN_LINES).unwrap_or(u64::MAX);
        lines = lines.child(
            Label::new(
                crate::explorer::live::text(
                    cx,
                    "hv-scripts-attach-more-lines",
                    vec![("n", ArgValue::Count(more))],
                ),
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text_2.to_hsla(),
            )
            .id("scripts-attach-more-lines"),
        );
    }
    inset.child(lines).into_any_element()
}

/// The folder's name, when it is UTF-8.
fn folder_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .filter(|n| LogicalDb::parse(n).is_ok())
        .map(str::to_string)
}

// ## m4b-t16-attach fix round 1 (I4)
/// `hv-scripts-manifest-reason`'s variant for a manifest diagnostic.
fn reason_word(d: &ManifestDiag) -> &'static str {
    use hacivat_scripts::manifest::Field;
    match d {
        ManifestDiag::LinkIgnored => "link",
        ManifestDiag::NotAFolder | ManifestDiag::NotAFile => "notfile",
        ManifestDiag::Changed => "changed",
        ManifestDiag::Unreadable(_) => "unreadable",
        ManifestDiag::TooLarge => "large",
        ManifestDiag::NotUtf8 => "notutf8",
        ManifestDiag::TooDeep => "deep",
        ManifestDiag::InvalidToml => "invalid",
        ManifestDiag::MissingFormat => "noformat",
        ManifestDiag::BadFormat => "badformat",
        ManifestDiag::NewerFormat(_) => "newer",
        ManifestDiag::WrongType(Field::Database) | ManifestDiag::BadName(Field::Database, _) => {
            "database"
        }
        _ => "other",
    }
}

// ---- shots ---------------------------------------------------------------------------------------

/// Shot fixtures: the dialog for `path` with `database` and `aliases` typed.
#[cfg(feature = "test-support")]
pub mod fixtures {
    use super::*;

    /// Puts `database` and `aliases` in the dialog's fields, as typing them does.
    pub fn type_names(database: &str, aliases: &str, cx: &mut App) {
        let Some(d) = open_dialog(cx) else {
            return;
        };
        let (db, al) = {
            let d = d.read(cx);
            (d.database.clone(), d.aliases.clone())
        };
        db.update(cx, |i, cx| {
            i.set_value(database.to_string());
            cx.notify();
        });
        al.update(cx, |i, cx| {
            i.set_value(aliases.to_string());
            cx.notify();
        });
        d.update(cx, |_, cx| cx.notify());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_parsed_like_the_manifest_reads_them() {
        let db = LogicalDb::parse("orders").unwrap();
        let got = parse_aliases(" orders_p, , ORDERS ,sipariş, orders_P ", Some(&db)).unwrap();
        assert_eq!(
            got.iter().map(LogicalDb::as_str).collect::<Vec<_>>(),
            ["orders_p", "sipariş"]
        );
        assert_eq!(parse_aliases("a\u{7}b", None).err(), Some("control"));
        assert_eq!(parse_aliases(&"x".repeat(129), None).err(), Some("long"));
        let many: Vec<String> = (0..33).map(|i| format!("a{i}")).collect();
        assert_eq!(parse_aliases(&many.join(","), None).err(), Some("many"));
    }

    // ## m4b-t16-attach fix round 1 (I4): every read failure names its own reason.
    #[test]
    fn every_read_failure_has_its_reason() {
        use hacivat_scripts::NameError;
        use hacivat_scripts::manifest::Field;
        let cases = [
            (ManifestDiag::LinkIgnored, "link"),
            (ManifestDiag::NotAFolder, "notfile"),
            (ManifestDiag::NotAFile, "notfile"),
            (ManifestDiag::Changed, "changed"),
            (
                ManifestDiag::Unreadable(std::io::ErrorKind::PermissionDenied),
                "unreadable",
            ),
            (ManifestDiag::TooLarge, "large"),
            (ManifestDiag::NotUtf8, "notutf8"),
            (ManifestDiag::TooDeep, "deep"),
            (ManifestDiag::InvalidToml, "invalid"),
            (ManifestDiag::MissingFormat, "noformat"),
            (ManifestDiag::BadFormat, "badformat"),
            (ManifestDiag::NewerFormat(2), "newer"),
            (ManifestDiag::WrongType(Field::Database), "database"),
            (
                ManifestDiag::BadName(Field::Database, NameError::Control),
                "database",
            ),
        ];
        for (d, want) in cases {
            assert!(
                manifest::is_read_failure(&d) || matches!(d, ManifestDiag::NewerFormat(_)),
                "{d:?} is one the dialog reports"
            );
            assert_eq!(reason_word(&d), want, "{d:?}");
        }
    }

    #[test]
    fn refusals_name_their_reason() {
        assert_eq!(
            refusal(&AttachError::RootInEnvFolder("@production".into())),
            ("envfolder", "@production".to_string())
        );
        assert_eq!(refusal(&AttachError::RootIsLink).0, "link");
        assert_eq!(
            refusal(&AttachError::Registry(RegistryError::HomeFolder)).0,
            "home"
        );
        assert_eq!(refusal(&AttachError::Stopped).0, "other");
    }
}
// ## m4b-t16-attach
