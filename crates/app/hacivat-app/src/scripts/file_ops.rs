//! File operations in the Scripts tree (R-M4b-18, Q11; NOTES surface 2): New script, New
//! folder, Rename (F2, `scripts.rename`), Move to Trash (no chord), Reveal (`scripts.reveal`),
//! Copy path, Copy relative path and Refresh.
//!
//! ## m4b-t01b-seams: nothing yet (Task 13); its commands are registered disabled in
//! [`super::install`].
//!
//! ## m4b-t13-explorer
//! - **Menus** per row kind (NOTES surface 2): the Scripts node and a library row (New script,
//!   New folder | Refresh, Reveal, Copy path | Script library settings…, Collapse all children;
//!   with several libraries the Scripts node keeps Refresh, settings and Collapse), a folder
//!   (New script, New folder | Rename, Move to Trash | Reveal, Copy path, Copy relative path |
//!   Refresh), a file (Open, Open under another connection… | Rename, Move to Trash | Reveal,
//!   Copy path, Copy relative path) and a link pointing outside (Open disabled with its reason |
//!   Reveal, Copy path, Copy relative path). "Open under another connection…" is Task 15's
//!   chooser and "Script library settings…" Task 16's page: both are shown disabled until then.
//! - **Commands.** Every item is a registered command acting on the explorer's cursor row
//!   (`ExplorerTree`), enabled only on the rows it applies to: F2 renames script rows only
//!   (controller ruling c), and Move to Trash has no chord. Words that differ per host take the
//!   `$os` selector ([`host_os`]); the palette passes it too.
//! - **The name field** (New script, New folder, Rename) is a field over the row, in a
//!   non-modal layer: the name selected, the reason under it while it is not a valid name
//!   (live), Enter commits, Escape cancels; the focus goes back to the tree, the explorer's
//!   focus home (R-M4b-43).
//! - **The work** goes to `LibraryService::{create, rename, trash, refresh}` off the UI thread,
//!   with the row's connection's environment (R-T5-6). A refusal is a toast with a Fluent
//!   message; the OS error kind, when there is one, is its data line only.

use std::rc::Rc;

use hacivat_l10n::ArgValue;
use hacivat_scripts::pathsafe::NameProblem;
use hacivat_scripts::{EntryKind, FileOpError, LibraryId, LinkState, PathError};
use hacivat_ui::actions::{ActionDef, ActionFn, ActionRegistry};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui::text::{TextFn, TextSource, paragraph};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui::tree::TreeKey;
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Bounds, ClipboardItem, Context, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    Styled as _, Subscription, Window, div, px,
};
use hacivat_ui_core::theme::Theme;

use super::projection::{self, Node, Place};
use crate::explorer::EXPLORER_TREE_CONTEXT;
use crate::l10n_global::{text_fn_id, tr_id};
use crate::shell::{Layer, Workspace};

/// The host the words of Reveal and Move to Trash are for (`$os`: `macos`, `windows`,
/// `linux`).
pub fn host_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

fn text(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    crate::explorer::live::text(cx, id, args)
}

fn os_text(id: &'static str) -> TextFn {
    Rc::new(move |cx: &App| text(cx, id, vec![("os", ArgValue::Text(host_os().into()))]))
}

// ---- commands --------------------------------------------------------------------------------

/// The Scripts tree's commands Task 13 adds (menu and palette; no keymap row but F2's).
pub(crate) const COMMANDS: [(&str, &str); 7] = [
    ("scripts.open", "hv-cmd-scripts-open"),
    ("scripts.new_script", "hv-cmd-scripts-new-script"),
    ("scripts.new_folder", "hv-cmd-scripts-new-folder"),
    ("scripts.trash", "hv-cmd-scripts-trash"),
    ("scripts.copy_path", "hv-cmd-scripts-copy-path"),
    (
        "scripts.copy_relative_path",
        "hv-cmd-scripts-copy-relative-path",
    ),
    ("scripts.refresh", "hv-cmd-scripts-refresh"),
];

type Enabled = fn(&App) -> bool;
type Run = fn(&mut Window, &mut App);

/// The run and enabled functions of a filled command (`None`: not Task 13's).
pub(crate) fn filled(id: &str) -> Option<(ActionFn, Enabled)> {
    let (run, enabled): (Run, Enabled) = match id {
        "scripts.open" => (open_command, can_open),
        "scripts.new_script" => (|w, cx| start_new(EntryKind::Sql, w, cx), can_create),
        "scripts.new_folder" => (|w, cx| start_new(EntryKind::Folder, w, cx), can_create),
        "scripts.rename" => (rename_command, can_rename),
        "scripts.trash" => (trash_command, can_rename),
        "scripts.reveal" => (reveal_command, can_reveal),
        "scripts.copy_path" => (|_, cx| copy_path(false, cx), can_reveal),
        "scripts.copy_relative_path" => (|_, cx| copy_path(true, cx), can_copy_relative),
        "scripts.refresh" => (refresh_command, can_reveal),
        _ => return None,
    };
    Some((Rc::new(run), enabled))
}

/// Registers Task 13's own commands (after Task 1b's, `scripts::install`).
pub(crate) fn install(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    for (id, title_id) in COMMANDS {
        let Some((run, enabled)) = filled(id) else {
            continue;
        };
        let _ = reg.register(ActionDef {
            id,
            title_id,
            context: EXPLORER_TREE_CONTEXT,
            run,
            enabled,
        });
    }
}

fn cursor(cx: &App) -> Option<(TreeKey, Place)> {
    projection::cursor_place(cx)
}

/// The cursor's file or folder row: its library and entry.
fn cursor_entry(cx: &App) -> Option<(TreeKey, Place, LibraryId, hacivat_scripts::ScriptEntry)> {
    let (key, place) = cursor(cx)?;
    let Node::Entry { library, entry, .. } = &place.node else {
        return None;
    };
    let (library, entry) = (*library, entry.clone());
    Some((key, place, library, entry))
}

fn outside(entry: &hacivat_scripts::ScriptEntry) -> bool {
    matches!(entry.link, LinkState::OutsideLink | LinkState::Loop)
}

fn can_open(cx: &App) -> bool {
    cursor_entry(cx).is_some_and(|(_, _, _, e)| e.kind != EntryKind::Folder && !outside(&e))
}

fn can_rename(cx: &App) -> bool {
    cursor_entry(cx).is_some_and(|(_, _, _, e)| !outside(&e))
}

/// Where New script and New folder create: the library and folder of the cursor row (a
/// Scripts node over one library, a library row or a folder).
fn create_target(cx: &App) -> Option<(Place, LibraryId, String)> {
    let (_, place) = cursor(cx)?;
    let (library, folder) = match &place.node {
        Node::Scripts { libraries } if libraries.len() == 1 => (libraries[0], String::new()),
        Node::Library { id, .. } => (*id, String::new()),
        Node::Entry { library, entry, .. } if entry.kind == EntryKind::Folder => {
            (*library, entry.rel.as_str().to_owned())
        }
        _ => return None,
    };
    Some((place, library, folder))
}

fn can_create(cx: &App) -> bool {
    create_target(cx).is_some()
}

fn can_reveal(cx: &App) -> bool {
    cursor(cx)
        .is_some_and(|(_, p)| p.library().is_some() && !matches!(p.node, Node::Problems { .. }))
}

fn can_copy_relative(cx: &App) -> bool {
    cursor_entry(cx).is_some()
}

fn open_command(window: &mut Window, cx: &mut App) {
    if let Some((key, _)) = cursor(cx)
        && let Some(owner) = crate::connections::explorer::contributed_owner(key, cx)
    {
        owner.activate(key, window, cx);
    }
}

/// The cursor row's path on disk (`relative`: inside its library, `/`-separated).
fn cursor_path(relative: bool, cx: &App) -> Option<String> {
    let (_, place) = cursor(cx)?;
    let library = place.library()?;
    let rel = place.entry().map(|e| e.rel.as_str().to_owned());
    if relative {
        return rel;
    }
    let path = projection::entry_path(&place, library, rel.as_deref(), cx)?;
    Some(path.to_string_lossy().into_owned())
}

fn copy_path(relative: bool, cx: &mut App) {
    if let Some(p) = cursor_path(relative, cx) {
        cx.write_to_clipboard(ClipboardItem::new_string(p));
    }
}

fn reveal_command(_: &mut Window, cx: &mut App) {
    let Some((_, place)) = cursor(cx) else {
        return;
    };
    let Some(library) = place.library() else {
        return;
    };
    let rel = place.entry().map(|e| e.rel.as_str().to_owned());
    if let Some(path) = projection::entry_path(&place, library, rel.as_deref(), cx) {
        cx.reveal_path(&path);
    }
}

fn refresh_command(_: &mut Window, cx: &mut App) {
    let (Some((_, place)), Some(svc)) = (cursor(cx), projection::service(cx)) else {
        return;
    };
    let libraries: Vec<LibraryId> = match &place.node {
        Node::Scripts { libraries } => libraries.clone(),
        _ => place.library().into_iter().collect(),
    };
    cx.spawn(async move |_| {
        for id in libraries {
            svc.refresh(id).await;
        }
    })
    .detach();
}

// ---- menus -----------------------------------------------------------------------------------

fn item(label: TextFn, action: &'static str, cx: &App) -> MenuItemSpec {
    let enabled = cx
        .try_global::<ActionRegistry>()
        .and_then(|r| r.get(action))
        .is_some_and(|d| (d.enabled)(cx));
    let mut spec = MenuItemSpec::new(label, action).disabled(!enabled);
    if crate::shell::keymap_state::shortcut_label(cx, action).is_some() {
        let keys: TextFn = Rc::new(move |cx: &App| {
            crate::shell::keymap_state::shortcut_label(cx, action)
                .unwrap_or_default()
                .into()
        });
        spec = spec.keys(keys);
    }
    spec
}

/// An item of a later task, shown disabled (with its reason when it has one).
fn later(label: &'static str, action: &'static str, hint: Option<&'static str>) -> MenuItemSpec {
    let mut spec = MenuItemSpec::new(text_fn_id(label), action).disabled(true);
    if let Some(h) = hint {
        spec = spec.hint(text_fn_id(h));
    }
    spec
}

/// Whether another connection lists the file a row shows ("Open under another connection…").
// ## m4b-t13-explorer fix round 1 (review m10): the chooser's candidate rule.
fn listed_elsewhere(place: &Place, cx: &App) -> bool {
    match (place.library(), place.entry()) {
        (Some(library), Some(entry)) => {
            projection::shown_under_another_connection(place, library, &entry.rel, cx)
        }
        _ => false,
    }
}

/// The context menu of a Scripts row (the cursor is moved onto it before it opens).
pub(crate) fn menu(place: &Place, cx: &App) -> Vec<MenuItemSpec> {
    let sep = MenuItemSpec::separator;
    let i = |label: &'static str, action: &'static str| item(text_fn_id(label), action, cx);
    let reveal = || item(os_text("hv-scripts-menu-reveal"), "scripts.reveal", cx);
    let trash = || item(os_text("hv-scripts-menu-trash"), "scripts.trash", cx);
    let refresh = || i("hv-explorer-menu-refresh", "scripts.refresh");
    let settings = || later("hv-scripts-menu-settings", "scripts.library_settings", None);
    let collapse = || {
        i(
            "hv-explorer-menu-collapse-children",
            "explorer.collapse_children",
        )
    };
    // Every item names a command that acts on the cursor row; the explorer puts the cursor on
    // the row before its menu opens (`open_node_menu`), and the items are read for that row.
    match &place.node {
        Node::Scripts { libraries } if libraries.len() != 1 => {
            vec![refresh(), sep(), settings(), collapse()]
        }
        Node::Scripts { .. } | Node::Library { .. } => vec![
            i("hv-scripts-menu-new-script", "scripts.new_script"),
            i("hv-scripts-menu-new-folder", "scripts.new_folder"),
            sep(),
            refresh(),
            reveal(),
            i("hv-scripts-menu-copy-path", "scripts.copy_path"),
            sep(),
            settings(),
            collapse(),
        ],
        Node::Entry { entry, .. } if entry.kind == EntryKind::Folder => vec![
            i("hv-scripts-menu-new-script", "scripts.new_script"),
            i("hv-scripts-menu-new-folder", "scripts.new_folder"),
            sep(),
            i("hv-scripts-menu-rename", "scripts.rename"),
            trash(),
            sep(),
            reveal(),
            i("hv-scripts-menu-copy-path", "scripts.copy_path"),
            i("hv-scripts-menu-copy-rel", "scripts.copy_relative_path"),
            sep(),
            refresh(),
        ],
        Node::Entry { entry, .. } if outside(entry) => vec![
            later(
                "hv-scripts-menu-open",
                "scripts.open",
                Some("hv-scripts-menu-outside-tip"),
            ),
            sep(),
            reveal(),
            i("hv-scripts-menu-copy-path", "scripts.copy_path"),
            i("hv-scripts-menu-copy-rel", "scripts.copy_relative_path"),
        ],
        Node::Entry { .. } => {
            let under_hint =
                (!listed_elsewhere(place, cx)).then_some("hv-scripts-menu-open-under-none");
            vec![
                i("hv-scripts-menu-open", "scripts.open"),
                // Task 15's chooser fills it.
                later(
                    "hv-scripts-menu-open-under",
                    "scripts.open_under",
                    under_hint,
                ),
                sep(),
                i("hv-scripts-menu-rename", "scripts.rename"),
                trash(),
                sep(),
                reveal(),
                i("hv-scripts-menu-copy-path", "scripts.copy_path"),
                i("hv-scripts-menu-copy-rel", "scripts.copy_relative_path"),
            ]
        }
        Node::Problems { .. } => Vec::new(),
    }
}

// ---- names -----------------------------------------------------------------------------------

/// Why a typed name cannot be used (`None`: it can). `siblings` are the names the folder shows.
pub(crate) fn name_problem(
    name: &str,
    kind: EntryKind,
    siblings: &[String],
    current: Option<&str>,
) -> Option<(&'static str, &'static str)> {
    const ERR: &str = "hv-scripts-name-err";
    const MORE: &str = "hv-scripts-name-err-more";
    if name.is_empty() {
        return Some((ERR, "empty"));
    }
    // Every host's rules: a library is shared with teammates on other systems.
    let checked = hacivat_scripts::pathsafe::component_on(
        name.as_ref(),
        hacivat_scripts::pathsafe::Host::Windows,
    )
    .and_then(|_| hacivat_scripts::pathsafe::component(name.as_ref()));
    if let Err(hacivat_scripts::PathError::Name(p)) = checked {
        return Some(match p {
            NameProblem::Separator => (ERR, "slash"),
            NameProblem::Reserved => (ERR, "reserved"),
            NameProblem::Dot | NameProblem::DotDot | NameProblem::TrailingDotOrSpace => {
                (ERR, "dots")
            }
            NameProblem::Empty => (ERR, "empty"),
            NameProblem::TooLong => (MORE, "long"),
            NameProblem::Skipped => (MORE, "hidden"),
            _ => (MORE, "chars"),
        });
    }
    if checked.is_err() {
        return Some((MORE, "chars"));
    }
    if hacivat_scripts::pathsafe::is_skipped(name) {
        return Some((MORE, "hidden"));
    }
    if kind != EntryKind::Folder && hacivat_scripts::scan::script_kind(name).is_none() {
        return Some((ERR, "ext"));
    }
    // ## m4b-t13-explorer fix round 1 (R-T13-1): any ASCII case is the same name on a shared
    // library (the core refuses it on every system); the row's own name may change case.
    let taken = siblings
        .iter()
        .any(|s| Some(s.as_str()) != current && (s == name || s.eq_ignore_ascii_case(name)));
    if current != Some(name) && taken {
        return Some((ERR, "exists"));
    }
    None
}

/// What the name field does on Enter.
#[derive(Debug, Clone)]
enum Mode {
    New(EntryKind),
    Rename { from: String, kind: EntryKind },
}

/// The layer of the name field.
pub const NAME_FIELD_LAYER: &str = "scripts-name-field";
/// Its key context (no binding: the field handles Enter and Escape).
const NAME_FIELD_CONTEXT: &str = "ScriptsNameField";

/// The name field over a tree row (NOTES "Inline name field").
pub struct NameField {
    input: Entity<TextInput>,
    place: Place,
    library: LibraryId,
    folder: String,
    mode: Mode,
    error: Option<(&'static str, &'static str)>,
    anchor: Bounds<Pixels>,
    // ## m4b-t13-explorer fix round 1 (review m12, m3)
    /// The row the field sits over: re-anchored on every frame, so a library change that
    /// moves the rows never leaves it over another row.
    key: TreeKey,
    /// The name as shown when it holds invisible or bidi characters (`<U+XXXX>`).
    hidden: Option<String>,
    indent: f32,
    busy: bool,
    _subscriptions: Vec<Subscription>,
}

impl NameField {
    fn kind(&self) -> EntryKind {
        match &self.mode {
            Mode::New(k) => *k,
            Mode::Rename { kind, .. } => *kind,
        }
    }

    fn current(&self) -> Option<String> {
        match &self.mode {
            Mode::New(_) => None,
            Mode::Rename { from, .. } => Some(from.rsplit('/').next().unwrap_or(from).to_owned()),
        }
    }

    /// The field's text (tests).
    pub fn value(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// The reason shown under the field (tests): `(message id, reason)`.
    pub fn error(&self) -> Option<(&'static str, &'static str)> {
        self.error
    }

    /// Puts `text` in the field and checks it, as typing would (the screenshot harness).
    #[cfg(feature = "test-support")]
    pub fn type_for_tests(&mut self, text: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| {
            i.set_value(text.to_string());
            cx.notify();
        });
        self.check(cx);
    }

    fn check(&mut self, cx: &mut Context<Self>) {
        let name = self.input.read(cx).value().to_string();
        let siblings = projection::shown_names(&self.place, self.library, &self.folder, cx);
        self.error = name_problem(&name, self.kind(), &siblings, self.current().as_deref());
        cx.notify();
    }

    fn rel_for(&self, name: &str) -> Option<hacivat_scripts::RelPath> {
        let text = if self.folder.is_empty() {
            name.to_owned()
        } else {
            format!("{}/{name}", self.folder)
        };
        hacivat_scripts::RelPath::parse(&text).ok()
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.check(cx);
        if self.error.is_some() {
            return;
        }
        let name = self.input.read(cx).value().to_string();
        if self.current().as_deref() == Some(name.as_str()) {
            close_field(window, cx);
            return;
        }
        let (Some(rel), Some(svc)) = (self.rel_for(&name), projection::service(cx)) else {
            return;
        };
        let env = projection::env_of(&self.place, cx);
        let library = self.library;
        let mode = self.mode.clone();
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let (op, out) = match &mode {
                Mode::New(kind) => ("create", svc.create(library, &rel, *kind, env).await),
                Mode::Rename { from, .. } => {
                    let from = hacivat_scripts::RelPath::parse(from).ok();
                    match from {
                        Some(from) => ("rename", svc.rename(library, &from, &rel, env).await),
                        None => ("rename", Err(FileOpError::NotFound)),
                    }
                }
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match out {
                    Ok(()) => close_field(window, cx),
                    // A name already taken shows in the field, as the live check does; so does
                    // another spelling of a name on disk (review m2).
                    Err(FileOpError::Exists | FileOpError::Path(PathError::Alias)) => {
                        this.error = Some(("hv-scripts-name-err", "exists"));
                        cx.notify();
                    }
                    Err(e) => {
                        close_field(window, cx);
                        failed(op, &name, &e, cx);
                    }
                }
            });
        })
        .detach();
    }
}

impl Render for NameField {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx).tokens;
        let c = t.colors;
        if let Some(now) = crate::connections::explorer::tree(cx)
            .and_then(|tree| tree.read(cx).row_bounds(self.key))
        {
            self.anchor = now;
        }
        let b = self.anchor;
        let left = f32::from(b.left()) + self.indent;
        let width = (f32::from(b.size.width) - self.indent - t.spacing.md).max(120.0);
        let mut card = div()
            .id("scripts-name-field")
            .absolute()
            .left(px(left))
            .top(px(f32::from(b.top())))
            .w(px(width))
            .flex()
            .flex_col()
            .child(self.input.clone());
        let mut notes: Vec<(&'static str, SharedString)> = Vec::new();
        if let Some(shown) = &self.hidden {
            notes.push((
                "scripts-name-hidden",
                text(
                    cx,
                    "hv-scripts-name-hidden-chars",
                    vec![("name", ArgValue::Text(shown.clone()))],
                ),
            ));
        }
        if let Some((id, reason)) = self.error {
            let message = text(
                cx,
                id,
                vec![
                    ("reason", ArgValue::Text(reason.into())),
                    (
                        "name",
                        ArgValue::Text(self.input.read(cx).value().to_string()),
                    ),
                ],
            );
            notes.push(("scripts-name-error", message));
        }
        for (id, message) in notes {
            // The tooltip anatomy, pinned under the field, with its text ink (Raised Ink).
            card = card.child(
                div()
                    .id(id)
                    .mt(px(t.spacing.xs))
                    .px(px(t.spacing.sm))
                    .py(px(t.spacing.xs))
                    .bg(c.raised.to_hsla())
                    .border_1()
                    .border_color(c.seam_strong.to_hsla())
                    .rounded(px(t.radii.sm))
                    .child(paragraph(
                        message,
                        TextSource::Fluent,
                        t.type_ramp.body_sm,
                        c.text.to_hsla(),
                    )),
            );
        }
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .key_context(NAME_FIELD_CONTEXT)
            .child(card)
    }
}

/// Pops the name field when it is the topmost layer (the focus goes back to the tree).
fn close_field(window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    if ws.read(cx).layer_ids().last() == Some(&NAME_FIELD_LAYER) {
        ws.update(cx, |ws, cx| ws.pop_layer(window, cx));
    }
}

/// The open name field, if any (tests).
pub fn open_field(window: &Window, cx: &App) -> Option<Entity<NameField>> {
    let _ = window;
    cx.try_global::<OpenField>()?.0.as_ref()?.upgrade()
}

#[derive(Default)]
struct OpenField(Option<hacivat_ui_core::gpui_surface::WeakEntity<NameField>>);

impl hacivat_ui_core::gpui_surface::Global for OpenField {}

/// Where the name field's entry goes: the row's place, its library and the folder.
type FieldTarget = (Place, LibraryId, String);

/// Opens the name field over row `key` (a new entry is drawn one level below it).
fn open_name_field(
    key: TreeKey,
    (place, library, folder): FieldTarget,
    mode: Mode,
    initial: String,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(tree) = crate::connections::explorer::tree(cx) else {
        return;
    };
    let (anchor, depth, tree_focus) = {
        let t = tree.read(cx);
        let depth = t
            .flat()
            .index_of(key)
            .map_or(0, |ix| t.flat().rows()[ix].depth);
        (t.row_bounds(key), depth, t.focus_handle(cx))
    };
    let Some(anchor) = anchor else {
        return;
    };
    let t = Theme::get(cx).tokens;
    // Over the row's name: past the indent, the chevron and the icon.
    let new_child = matches!(mode, Mode::New(_));
    let level = f32::from(depth) + if new_child { 1.0 } else { 0.0 };
    let indent = t.spacing.md + level * t.spacing.tree_indent + 2.0 * (12.0 + t.spacing.xs);
    let ws = window.root::<Workspace>().flatten();
    let Some(ws) = ws else {
        return;
    };
    let field = cx.new(|cx| {
        let input = cx.new(|cx| {
            let mut i = TextInput::new(cx, text_fn_id("hv-scripts-new-name"));
            i.set_mono(true);
            i.set_value(initial.clone());
            i.select_all();
            i
        });
        let subscriptions =
            vec![
                cx.subscribe(&input, |this: &mut NameField, _, e: &InputEvent, cx| {
                    if matches!(e, InputEvent::Changed) {
                        this.check(cx);
                    }
                }),
            ];
        NameField {
            input,
            place,
            library,
            folder,
            mode,
            error: None,
            anchor,
            key,
            hidden: {
                let shown = hacivat_scripts::pathsafe::display_name(&initial);
                (shown != initial).then_some(shown)
            },
            indent,
            busy: false,
            _subscriptions: subscriptions,
        }
    });
    let input = field.read(cx).input.clone();
    // Enter and Escape need the window: subscribed with it, before the layer is pushed.
    let weak = field.downgrade();
    let sub = window.subscribe(&input, cx, move |_, e: &InputEvent, window, cx| match e {
        InputEvent::Submitted => {
            if let Some(f) = weak.upgrade() {
                f.update(cx, |f, cx| f.submit(window, cx));
            }
        }
        InputEvent::Cancelled => close_field(window, cx),
        _ => {}
    });
    field.update(cx, |f, _| f._subscriptions.push(sub));
    ws.update(cx, |ws, cx| {
        ws.push_layer(
            Layer {
                id: NAME_FIELD_LAYER,
                context: NAME_FIELD_CONTEXT,
                view: field.clone().into(),
                restore_focus: Some(tree_focus),
                modal: false,
            },
            window,
            cx,
        );
    });
    let focus = input.read(cx).focus_handle(cx);
    window.focus(&focus, cx);
    cx.set_global(OpenField(Some(field.downgrade())));
    field.update(cx, |f, cx| f.check(cx));
}

fn start_new(kind: EntryKind, window: &mut Window, cx: &mut App) {
    let Some((key, _)) = cursor(cx) else {
        return;
    };
    let Some((place, library, folder)) = create_target(cx) else {
        return;
    };
    let initial = match kind {
        EntryKind::Folder => tr_id(cx, "hv-scripts-new-folder-name"),
        _ => tr_id(cx, "hv-scripts-new-name"),
    };
    open_name_field(
        key,
        (place, library, folder),
        Mode::New(kind),
        initial.to_string(),
        window,
        cx,
    );
}

fn rename_command(window: &mut Window, cx: &mut App) {
    let Some((key, place, library, entry)) = cursor_entry(cx) else {
        return;
    };
    if outside(&entry) {
        return;
    }
    let rel = entry.rel.as_str().to_owned();
    let folder = rel
        .rfind('/')
        .map_or(String::new(), |i| rel[..i].to_owned());
    let name = entry.rel.name().to_owned();
    open_name_field(
        key,
        (place, library, folder),
        Mode::Rename {
            from: rel,
            kind: entry.kind,
        },
        name,
        window,
        cx,
    );
}

// ---- Move to Trash ---------------------------------------------------------------------------

/// The row the cursor goes to when `key`'s row goes away: the next sibling, else the previous
/// one, else the parent (review Minor 9).
fn next_cursor(key: TreeKey, cx: &App) -> Option<TreeKey> {
    let tree = crate::connections::explorer::tree(cx)?;
    let t = tree.read(cx);
    let rows = t.flat().rows();
    let ix = t.flat().index_of(key)?;
    let depth = rows[ix].depth;
    let after = rows[ix + 1..]
        .iter()
        .take_while(|r| r.depth >= depth)
        .find(|r| r.depth == depth)
        .map(|r| r.key);
    let before = rows[..ix]
        .iter()
        .rev()
        .take_while(|r| r.depth >= depth)
        .find(|r| r.depth == depth)
        .map(|r| r.key);
    let parent = rows[..ix]
        .iter()
        .rev()
        .find(|r| r.depth < depth)
        .map(|r| r.key);
    after.or(before).or(parent)
}

fn trash_command(window: &mut Window, cx: &mut App) {
    let Some((key, place, library, entry)) = cursor_entry(cx) else {
        return;
    };
    if outside(&entry) {
        return;
    }
    let Some(svc) = projection::service(cx) else {
        return;
    };
    let env = projection::env_of(&place, cx);
    let next = next_cursor(key, cx);
    let name = hacivat_scripts::pathsafe::display_name(entry.rel.name());
    let rel = entry.rel.clone();
    window
        .spawn(cx, async move |cx| {
            let out = svc.trash(library, &rel, env).await;
            let changed = matches!(out, Err(FileOpError::TrashedButChanged));
            let _ = cx.update(|_, cx| match out {
                // ## m4b-t13-explorer fix round 1 (review m13): taken, but a folder changed
                // meanwhile: said so, never "nothing changed".
                Ok(()) | Err(FileOpError::TrashedButChanged) => {
                    if let (Some(next), Some(tree)) = (next, crate::connections::explorer::tree(cx))
                    {
                        tree.update(cx, |t, cx| t.select(next, cx));
                    }
                    let id = if changed {
                        "hv-scripts-trashed-changed"
                    } else {
                        "hv-scripts-trashed"
                    };
                    let label: TextFn = Rc::new(move |cx: &App| {
                        text(
                            cx,
                            id,
                            vec![
                                ("os", ArgValue::Text(host_os().into())),
                                ("name", ArgValue::Text(name.clone())),
                            ],
                        )
                    });
                    ToastQueue::push(cx, Toast::new(ToastKind::Info, label));
                }
                Err(e) => failed("trash", &name, &e, cx),
            });
        })
        .detach();
}

/// The toast of a refused operation: a Fluent title and reason; the OS error kind, if any, as
/// its data line.
fn failed(op: &'static str, name: &str, e: &FileOpError, cx: &mut App) {
    let reason = match e {
        // ## m4b-t13-explorer fix round 1: another spelling of a name on disk (review m2), and
        // the new refusals.
        FileOpError::Exists | FileOpError::Path(PathError::Alias) => "exists",
        FileOpError::ScopeChange => "scope",
        FileOpError::NotFullyListed => "large",
        FileOpError::NoSafeRename => "nohardlinks",
        FileOpError::TrashedButChanged => "changed",
        FileOpError::NotFound => "gone",
        FileOpError::NotListed => "listed",
        FileOpError::HoldsHidden => "hidden",
        FileOpError::OtherFolder | FileOpError::BadName(_) | FileOpError::NotAScript => "name",
        FileOpError::Path(_) => "path",
        FileOpError::Unavailable => "unavailable",
        FileOpError::Trash(_) => "trash",
        FileOpError::Io(_) => "io",
    };
    let detail = match e {
        FileOpError::Io(k) | FileOpError::Trash(k) => Some(SharedString::from(format!("{k:?}"))),
        _ => None,
    };
    let name = name.to_string();
    let title: TextFn = Rc::new(move |cx: &App| {
        text(
            cx,
            "hv-scripts-op-failed-title",
            vec![
                ("op", ArgValue::Text(op.into())),
                ("os", ArgValue::Text(host_os().into())),
                ("name", ArgValue::Text(name.clone())),
            ],
        )
    });
    let body: TextFn = Rc::new(move |cx: &App| {
        text(
            cx,
            "hv-scripts-op-failed",
            vec![("reason", ArgValue::Text(reason.into()))],
        )
    });
    let mut toast = Toast::new(ToastKind::Error, body);
    toast.title = Some(title);
    toast.data = detail;
    ToastQueue::push(cx, toast);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_problems_name_their_reason() {
        let sib = vec!["a.sql".to_string(), "raporlar".to_string()];
        let p = |n: &str, k| name_problem(n, k, &sib, None).map(|(_, r)| r);
        assert_eq!(p("b.sql", EntryKind::Sql), None);
        assert_eq!(p("müşteri.sql", EntryKind::Sql), None);
        assert_eq!(p("", EntryKind::Sql), Some("empty"));
        assert_eq!(p("a/b.sql", EntryKind::Sql), Some("slash"));
        assert_eq!(p("..", EntryKind::Folder), Some("dots"));
        assert_eq!(p("x.sql.", EntryKind::Sql), Some("dots"));
        assert_eq!(p("NUL.sql", EntryKind::Sql), Some("reserved"));
        assert_eq!(p("a.sql", EntryKind::Sql), Some("exists"));
        assert_eq!(p("b.txt", EntryKind::Sql), Some("ext"));
        assert_eq!(p("x:y.sql", EntryKind::Sql), Some("chars"));
        assert_eq!(p(".git", EntryKind::Folder), Some("hidden"));
        // Renaming to its own name is no clash.
        assert_eq!(
            name_problem("a.sql", EntryKind::Sql, &sib, Some("a.sql")),
            None
        );
    }
}
