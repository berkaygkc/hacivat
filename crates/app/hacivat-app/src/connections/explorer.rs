//! The explorer: the profile list of M2a became the database explorer tree (NOTES surface 8,
//! Task 18; M4 Task 14, R-M4-18, design/m4-explorer surfaces 1, 2, 4).
//!
//! - **Filter.** A `TextInput` ("Filter objects") matched through `PreparedQuery` against each
//!   profile's name and host (M2a) and every loaded catalog name (Task 14, D-074): Contains,
//!   Turkish-tolerant while the toggle in the field is on (its default is on for a Turkish UI
//!   language or region). Matches reveal their ancestors; the match count sits in the field;
//!   "No objects match" with Clear filter when nothing does. More than
//!   [`SYNC_FILTER_NAMES`] names are matched off the UI thread, latest wins. Its
//!   `InputEvent::Cancelled` (Escape in the field) runs the shell's Escape hook (P16).
//! - **Tree** (`hacivat_ui::tree`, Task 8) over [`ExplorerModel`] (Task 14): the environment
//!   headers of M2a (labels: by `EnvTag::severity_rank`, then "No environment" last, R31; with
//!   a count) and the profile rows as roots, each with its catalog below (R-M4-18). A profile
//!   row keeps its M2a content: the engine glyph, the name (strong while connected), the
//!   environment mark, the read-only lock (red only on production), the SSH route and the
//!   plaintext lock-open flags, and the live mark and meta (✓ and latency, the run marker, ⚠,
//!   the unplug glyph after an idle disconnect; otherwise the engine).
//! - **Selection and keys.** The tree's focus registers the key context `Explorer` (R20, P4).
//!   Selecting any row selects its profile in [`ConnectionsModel`] (the sheet follows it); the
//!   profile commands (Enter, ⌘E, ⌘D, ⌘⇧C, ⌫, Shift-F10) act only while the keyboard's row is a
//!   profile root ([`is_visible`]): elsewhere their chords pass through to the tree (Enter
//!   opens or toggles, Shift-F10 asks for the row's menu). Focusing the tree with nothing
//!   selected selects the first profile. Right-click on a profile row opens the connection menu
//!   of that row (the event's key, never the selection); a double-click connects.
//! - **Empty state** (NOTES §8): the title, one sentence, New connection, Paste URL and a ghost
//!   Import pg_service.conf…
//!
//! Debug selectors: `explorer-list` (the tree's frame) and the tree's own (`hv-tree-name-<key>`,
//! `hv-tree-flag-<key>-<i>`, `hv-tree-mark-<key>`); [`shown_rows`] lists what the tree shows.

use std::collections::HashMap;
use std::panic::Location;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_session::LiveState;
use hacivat_session::profile::{Profile, ProfileEndpoint, TransportSpec};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui::text::{Label, RenderedText, TextFn, TextRegistry, TextSource};
use hacivat_ui::tree::{
    ChildState, KeySet, MetaMark, RowFlag, TreeEvent, TreeKey, TreeModel, TreeRowData, TreeView,
};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable,
    Global, Hsla, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels, Point,
    Render, SharedString, Styled, Subscription, Task, WeakEntity, Window, canvas, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, TypeStyle};

use crate::commands::palette::search_opts;
use crate::connections::capsule::{engine_label, env_kind, is_unencrypted, latency_label};
use crate::connections::model::ConnectionsModel;
use crate::connections::session_toasts;
use crate::console::find_bar::{ToggleSpec, toggle_element};
use crate::explorer::adapter::{
    self, ContributedLoad, ContributedPlace, ExplorerCache, ExplorerInputs, ExplorerModel,
    FilterResult, NodeInfo, ProfileInput, SYNC_FILTER_NAMES, profile_key,
};
use crate::explorer::contrib::{ContributedChildren, contributors};
use crate::l10n_global::{L10nGlobal, is_turkic, text_fn, tr, tr_id, ui_language};
use crate::shell::Workspace;
use crate::shell::hooks::ShellHooks;
use crate::shell::hooks::{add_focus_context, remove_focus_context};
use crate::shell::keymap_state::shortcut_label;
use crate::shell::tiles::{self, panel_header, panel_title};

/// The key context of the connection rows (R20): pushed while the focused tree's cursor is on a
/// profile root (ruling R-W2-12b).
pub const EXPLORER_CONTEXT: &str = "Explorer";
/// The key context of the M4 object-level rows (F12, F5, ⌘←, ⌥⌘C, ⌥⌘F): pushed whenever the
/// tree has the focus, on every row (ruling R-W2-12b).
pub const EXPLORER_TREE_CONTEXT: &str = "ExplorerTree";

/// The explorer the shell built, for the focus commands.
struct ExplorerHandle(WeakEntity<ExplorerView>);

impl Global for ExplorerHandle {}

// ## m4-t14-explorer: the tree in place of the M2a list (R-M4-18).

/// What the model was last built from (a rebuild is skipped when nothing changed).
#[derive(Clone, PartialEq)]
struct Stamp {
    profiles: Vec<(usize, LiveState, bool)>,
    catalogs: u64,
    l10n: u64,
    contributors: u64,
    filter: Option<usize>,
    loads: usize,
}

/// The explorer view (the `explorer` hook).
pub struct ExplorerView {
    filter: Entity<TextInput>,
    tree: Entity<TreeView<ExplorerModel>>,
    /// The filter's Turkish-tolerant toggle; `None` follows the default (D-074).
    tolerant: Option<bool>,
    tolerant_focus: FocusHandle,
    cache: Rc<ExplorerCache>,
    stamp: Option<Stamp>,
    generation: u64,
    /// The filter's result for the current query (`None`: no filter).
    result: Option<Arc<FilterResult>>,
    /// The expansion before the filter revealed its matches (restored when it clears).
    saved_expansion: Option<KeySet>,
    /// The newest filter job; a result of an older one is dropped (latest wins).
    filter_seq: u64,
    filter_task: Option<Task<()>>,
    /// The newest job's number, read by running jobs: an older job stops (fix round 1, I3).
    latest: Arc<AtomicU64>,
    /// The catalogs' change counter the filter last ran over (fix round 1, m10).
    catalogs_seen: u64,
    /// Whether the tree's focus carries the `Explorer` key context (cursor on a profile root).
    context_on: bool,
    /// The children of contributed nodes expanded so far.
    loads: HashMap<TreeKey, ContributedLoad>,
    _subscriptions: Vec<Subscription>,
}

/// The `explorer` hook: builds the view. The tree's focus carries `ExplorerTree` on every row
/// and `Explorer` (P4) while its cursor is on a profile root (rulings R-W2-12, R-W2-12b,
/// [`ExplorerView::on_tree_changed`]).
pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let view = cx.new(|cx| ExplorerView::new(window, cx));
    cx.set_global(ExplorerHandle(view.downgrade()));
    let list = view.read(cx).list_focus(cx);
    add_focus_context(cx, list, EXPLORER_TREE_CONTEXT);
    view.update(cx, |v, cx| v.on_tree_changed(cx));
    view.into()
}

fn explorer(cx: &App) -> Option<Entity<ExplorerView>> {
    cx.try_global::<ExplorerHandle>()?.0.upgrade()
}

/// The explorer's tree (tests, Task 24).
pub fn tree(cx: &App) -> Option<Entity<TreeView<ExplorerModel>>> {
    Some(explorer(cx)?.read(cx).tree.clone())
}

/// Focuses the filter field (`explorer.focus_filter`).
pub fn focus_filter(window: &mut Window, cx: &mut App) {
    if let Some(view) = explorer(cx) {
        let handle = view.read(cx).filter_focus(cx);
        window.focus(&handle, cx);
    }
}

/// Whether `id`'s root row is shown under the current filter (true without an explorer view).
///
/// ## m4-t14-explorer (fix round 1, ruling R-W2-12): the M2a meaning. The commands act on the
/// profile that owns the tree's cursor row ([`ConnectionsModel::selected`] follows it), from
/// the palette and the welcome page wherever the cursor is; the `Explorer` key context, and so
/// the Explorer keymap rows, exist only while the cursor is on a profile root.
pub fn is_visible(cx: &App, id: ProfileId) -> bool {
    match explorer(cx) {
        Some(view) => view.read(cx).visible_ids(cx).contains(&id),
        None => true,
    }
}

/// Focuses the tree (its `Explorer` bindings then apply).
pub fn focus_list(window: &mut Window, cx: &mut App) {
    if let Some(view) = explorer(cx) {
        let handle = view.read(cx).list_focus(cx);
        window.focus(&handle, cx);
    }
}

/// The search options of the filter: contains, Turkish-tolerant for Turkish users.
fn default_tolerant(cx: &App) -> bool {
    search_opts(cx).turkish_tolerant
}

/// A profile's host or file, as the filter and the sheet see it.
pub fn endpoint_text(p: &Profile) -> String {
    match &p.endpoint {
        ProfileEndpoint::Server { host, .. } => host.clone(),
        ProfileEndpoint::Socket { path, .. } | ProfileEndpoint::File { path, .. } => {
            path.display().to_string()
        }
    }
}

/// One row the tree shows (for tests and later tasks): its key, depth and row data.
#[derive(Debug, Clone)]
pub struct ShownRow {
    pub key: TreeKey,
    pub depth: u16,
    pub data: TreeRowData,
    /// The status text of a loading or failed child row (`None` for a node).
    pub status: Option<SharedString>,
}

/// The rows the tree shows, top down (as of its last flatten).
pub fn shown_rows(cx: &App) -> Vec<ShownRow> {
    let Some(tree) = tree(cx) else {
        return Vec::new();
    };
    let tree = tree.read(cx);
    let model = tree.model();
    tree.flat()
        .rows()
        .iter()
        .filter_map(|r| {
            use hacivat_ui::tree::flat::RowKind;
            match r.kind {
                RowKind::Node => Some(ShownRow {
                    key: r.key,
                    depth: r.depth,
                    data: model.row(r.key)?,
                    status: None,
                }),
                RowKind::Loading | RowKind::Failed => {
                    let label = match model.children(r.key) {
                        ChildState::Loading { label, .. } | ChildState::Failed { label, .. } => {
                            label
                        }
                        _ => return None,
                    };
                    Some(ShownRow {
                        key: r.key,
                        depth: r.depth,
                        data: TreeRowData::new(label.clone(), TextSource::Fluent),
                        status: Some(label),
                    })
                }
            }
        })
        .collect()
}

/// The profile root rows shown (in order), as `(profile, bounds of the row)`.
pub fn profile_rows(cx: &App) -> Vec<(ProfileId, Option<Bounds<Pixels>>)> {
    let Some(tree) = tree(cx) else {
        return Vec::new();
    };
    let tree = tree.read(cx);
    tree.flat()
        .rows()
        .iter()
        .filter_map(|r| {
            let id = tree.model().profile_root(r.key)?;
            Some((id, tree.row_bounds(r.key)))
        })
        .collect()
}

/// A wrapping paragraph recorded in the text registry like a label (`Label` draws one line),
/// so the pseudo-locale and test checks see it. `id` is appended to its path as `#id`.
#[track_caller]
pub(crate) fn paragraph(
    text: SharedString,
    style: TypeStyle,
    color: Hsla,
    id: &'static str,
) -> AnyElement {
    let at = Location::caller();
    let recorded = text.clone();
    // Recorded while painted (as `Label`), so the registry keeps paint order.
    let record = canvas(
        move |_, _, cx| {
            TextRegistry::record(cx, || RenderedText {
                text: recorded.clone(),
                source: TextSource::Fluent,
                size_px: style.size_px,
                painted_px: 0.0,
                clipped: false,
                truncated: false,
                // ## m2b-t24-shots
                visible: true,
                outside_card: false,
                path: format!(
                    "{}:{}:{}#{id}",
                    at.file().replace('\\', "/"),
                    at.line(),
                    at.column()
                )
                .into(),
            });
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0();
    div()
        .relative()
        .child(record)
        .w_full()
        .font(font_for(&style))
        .text_size(px(style.size_px))
        .line_height(px(style.line_height))
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// The connection menu's items for `id` (NOTES §8): Connect or Disconnect, Edit…,
/// Duplicate, Copy as URL, a separator, Delete…. They run on the selection.
pub fn menu_items(cx: &App, id: ProfileId) -> Vec<MenuItemSpec> {
    let live = cx
        .try_global::<ConnectionsModel>()
        .map(|m| m.live(id))
        .unwrap_or_default();
    let connected = matches!(live, LiveState::Connected(_) | LiveState::Connecting(_));
    let enabled = |action: &str| {
        cx.try_global::<ActionRegistry>()
            .and_then(|r| r.get(action))
            .is_some_and(|d| (d.enabled)(cx))
    };
    let item = |label: TextFn, action: &'static str| {
        // A command that cannot run now (a read-only store) shows disabled.
        let mut item = MenuItemSpec::new(label, action).disabled(!enabled(action));
        if shortcut_label(cx, action).is_some() {
            let keys: TextFn =
                Rc::new(move |cx: &App| shortcut_label(cx, action).unwrap_or_default().into());
            item = item.keys(keys);
        }
        item
    };
    vec![
        item(
            text_fn(if connected {
                ids::hv::CONN_MENU_DISCONNECT
            } else {
                ids::hv::CONN_MENU_CONNECT
            }),
            "connection.toggle_connect",
        ),
        item(text_fn(ids::hv::CONN_MENU_EDIT), "connection.edit"),
        item(text_fn(ids::hv::CONN_MENU_DUP), "connection.duplicate"),
        item(text_fn(ids::hv::CONN_MENU_COPY_URL), "connection.copy_url"),
        MenuItemSpec::separator(),
        // ## m2a-t21-shots: "Delete… in red" (NOTES §8).
        item(text_fn(ids::hv::CONN_MENU_DELETE), "connection.delete").danger(true),
    ]
}

/// Opens the connection menu of the selection at `anchor` (or under the selected row).
pub fn open_context_menu(anchor: Option<Point<Pixels>>, window: &mut Window, cx: &mut App) {
    let Some(id) = cx
        .try_global::<ConnectionsModel>()
        .and_then(|m| m.selected())
    else {
        return;
    };
    open_menu_for(id, anchor, window, cx);
}

/// Opens profile `id`'s connection menu at `anchor` (or under its row).
fn open_menu_for(id: ProfileId, anchor: Option<Point<Pixels>>, window: &mut Window, cx: &mut App) {
    let anchor = anchor
        .or_else(|| {
            let tree = tree(cx)?;
            Some(tree.read(cx).row_bounds(profile_key(id))?.bottom_left())
        })
        .unwrap_or_default();
    let items = menu_items(cx, id);
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    ws.update(cx, |ws, cx| {
        ws.open_popup_menu(anchor, items, window, cx);
    });
}

/// What profile `p`'s root row shows besides its name (M2a content, mock root meta).
fn profile_input(p: &Arc<Profile>, cx: &App) -> ProfileInput {
    let m = cx.global::<ConnectionsModel>();
    let live = m.live(p.id);
    let production = p.environment == Some(EnvTag::Production);
    let mut flags = vec![RowFlag::Env(env_kind(p.environment))];
    let mut words: Vec<SharedString> = Vec::new();
    if let Some(tag) = p.environment {
        words.push(tr_id(
            cx,
            match tag {
                EnvTag::Production => "hv-conn-env-production",
                EnvTag::Staging => "hv-conn-env-staging",
                EnvTag::Test => "hv-conn-env-test",
                EnvTag::Dev => "hv-conn-env-dev",
                EnvTag::Local => "hv-conn-env-local",
            },
        ));
    }
    // Read-only: red only on production (DESIGN.md).
    if p.read_only {
        flags.push(RowFlag::Lock { production });
        words.push(tr(cx, ids::hv::CAPSULE_ACCESS_READ_ONLY, NoArgs));
    }
    if matches!(p.transport, TransportSpec::SshTunnel(_)) {
        let text = tr_id(cx, "hv-conn-section-ssh");
        words.push(text.clone());
        flags.push(RowFlag::Icon {
            icon: IconName::Route,
            text,
        });
    }
    let info = match &live {
        LiveState::Connected(info) => Some(info.clone()),
        _ => None,
    };
    if let Some(info) = &info
        && is_unencrypted(p, info)
    {
        let text = tr_id(cx, "hv-conn-enc-unencrypted");
        words.push(text.clone());
        flags.push(RowFlag::Icon {
            icon: IconName::LockOpen,
            text,
        });
    }
    let engine = || Some(engine_label(p, None));
    let mut tooltip = None;
    let (mark, meta) = match &live {
        LiveState::Connected(info) => (Some(MetaMark::Ok), Some(latency_label(cx, info))),
        // ## m2b-t01-interfaces: a reconnect runs like a connect.
        LiveState::Connecting(_) | LiveState::Reconnecting { .. } => {
            (Some(MetaMark::Running), engine())
        }
        LiveState::Failed(_) | LiveState::Lost { .. } => {
            (Some(MetaMark::Glyph(IconName::TriangleAlert)), engine())
        }
        // ## m2b-t22-live: disconnected while idle (B18).
        LiveState::Disconnected if session_toasts::is_idle(cx, p.id) => {
            tooltip = Some(tr_id(cx, "hv-conn-idle-cap"));
            (Some(MetaMark::Glyph(IconName::Unplug)), engine())
        }
        LiveState::Disconnected => (None, engine()),
    };
    let a11y = adapter::join_words(&words);
    ProfileInput {
        profile: p.clone(),
        connected: info.is_some(),
        mark,
        meta,
        flags,
        a11y,
        tooltip,
        catalog: adapter::catalog_of(p.id, cx),
    }
}

/// The contributed nodes of every database a profile shows (R-M4-19).
fn contributed_places(profiles: &[ProfileInput], cx: &App) -> Vec<ContributedPlace> {
    let list = contributors(cx);
    if list.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for p in profiles {
        let Some(cat) = p.catalog.as_ref().filter(|c| c.generation > 0) else {
            continue;
        };
        let mut dbs: Vec<String> = Vec::new();
        match hacivat_meta::kind::database_level(p.profile.engine) {
            hacivat_meta::kind::DbLevel::File => dbs.push("main".to_string()),
            level => {
                for k in cat
                    .namespaces
                    .keys()
                    .chain(cat.states.keys().filter_map(|c| match c {
                        hacivat_meta::snapshot::ContainerKey::Namespace(k) => Some(k),
                        _ => None,
                    }))
                {
                    let name = match (level, k.kind) {
                        (hacivat_meta::kind::DbLevel::Database, _) => k.path.first(),
                        (_, hacivat_meta::NsKind::Schema) => k.path.last(),
                        _ => None,
                    };
                    if let Some(n) = name
                        && !dbs.iter().any(|d| d == n.as_ref())
                    {
                        dbs.push(n.to_string());
                    }
                }
                if let Some(db) = &p.profile.database
                    && level == hacivat_meta::kind::DbLevel::Database
                    && !dbs.contains(db)
                {
                    dbs.push(db.clone());
                }
            }
        }
        for db in dbs {
            let nodes: Vec<_> = list
                .iter()
                .flat_map(|c| c.database_children(p.profile.id, &db, cx))
                .collect();
            if !nodes.is_empty() {
                out.push(ContributedPlace {
                    profile: p.profile.id,
                    database: db,
                    nodes,
                });
            }
        }
    }
    out
}

fn contributors_generation(cx: &App) -> u64 {
    contributors(cx).iter().fold(0u64, |acc, c| {
        acc.wrapping_mul(31).wrapping_add(c.generation())
    })
}

impl ExplorerView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| TextInput::new(cx, text_fn(ids::hv::EXPLORER_FILTER_OBJECTS)));
        let tree = cx.new(|cx| {
            let mut t = TreeView::new(ExplorerModel::default(), cx);
            t.set_a11y_label(text_fn(ids::hv::EXPLORER_TREE_LABEL));
            t
        });
        let tolerant_focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            cx.subscribe_in(
                &filter,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Changed => this.query_changed(cx),
                    // ## fix round 1 (m8, mock surface 4): Escape clears the field; on an empty
                    // field it is the shell's Escape (P16: cancel a connect) and the focus
                    // returns to the tree.
                    InputEvent::Cancelled => {
                        if !this.filter.read(cx).value().is_empty() {
                            this.filter.update(cx, |f, cx| {
                                f.set_value("");
                                cx.notify();
                            });
                            this.query_changed(cx);
                            return;
                        }
                        let escape = cx.global::<ShellHooks>().escape.clone();
                        if let Some(escape) = escape {
                            escape(window, cx);
                        }
                        let handle = this.list_focus(cx);
                        window.focus(&handle, cx);
                    }
                    InputEvent::Submitted => {
                        let handle = this.list_focus(cx);
                        window.focus(&handle, cx);
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(&tree, window, |this, _, event: &TreeEvent, window, cx| {
                this.on_tree_event(event, window, cx)
            }),
            // Every move of the cursor (a header included, which emits no event).
            cx.observe(&tree, |this, _, cx| this.on_tree_changed(cx)),
            cx.observe_global::<ConnectionsModel>(|this, cx| {
                let names_moved = this.refresh(cx);
                this.sync_selection(cx);
                // ## fix round 1 (m1): an added or renamed profile is matched again.
                if names_moved && !this.query(cx).is_empty() {
                    this.query_changed(cx);
                }
                cx.notify();
            }),
            cx.observe_global::<adapter::ExplorerCatalogs>(|this, cx| {
                let renames = adapter::take_renames(cx);
                this.refresh(cx);
                if !renames.is_empty() {
                    this.apply_renames(&renames, cx);
                }
                // The filter's keys belong to the snapshots it ran over; a notification with
                // no new catalog (the rename drain's) runs nothing again (fix round 1, m10).
                let generation = adapter::catalogs_generation(cx);
                if this.result.is_some() && generation != this.catalogs_seen {
                    this.query_changed(cx);
                }
                cx.notify();
            }),
        ];
        let mut view = ExplorerView {
            filter,
            tree,
            tolerant: None,
            tolerant_focus,
            cache: Rc::default(),
            stamp: None,
            generation: 0,
            result: None,
            saved_expansion: None,
            filter_seq: 0,
            filter_task: None,
            latest: Arc::default(),
            catalogs_seen: 0,
            context_on: false,
            loads: HashMap::new(),
            _subscriptions: subscriptions,
        };
        view.refresh(cx);
        view.sync_fold(cx);
        view.sync_selection(cx);
        view
    }

    fn list_focus(&self, cx: &App) -> FocusHandle {
        self.tree.read(cx).focus_handle(cx)
    }

    fn filter_focus(&self, cx: &App) -> FocusHandle {
        self.filter.read(cx).focus_handle(cx)
    }

    fn query(&self, cx: &App) -> String {
        self.filter.read(cx).value().trim().to_string()
    }

    /// The filter's Turkish-tolerant toggle (D-074).
    fn tolerant(&self, cx: &App) -> bool {
        self.tolerant.unwrap_or_else(|| default_tolerant(cx))
    }

    /// The profile that owns the tree's cursor row, and whether the cursor is on its root.
    fn cursor_owner(&self, cx: &App) -> (Option<ProfileId>, bool) {
        let tree = self.tree.read(cx);
        let Some(cursor) = tree.cursor().filter(|k| tree.flat().index_of(*k).is_some()) else {
            return (None, false);
        };
        let model = tree.model();
        (
            model.profile_of(cursor),
            model.profile_root(cursor).is_some(),
        )
    }

    /// The tree changed (its cursor, its rows): ruling R-W2-12.
    /// - The `Explorer` key context is registered for the tree's focus only while the cursor
    ///   is on a profile root, so the Explorer keymap rows (Enter, ⌘E, ⌘D, ⌘⇧C, ⌫, Shift-F10)
    ///   act only there; on any other row their chords reach the tree.
    /// - [`ConnectionsModel::selected`] follows the profile that owns the cursor row, so the
    ///   sheet and the palette's and welcome page's connection commands name the same one.
    fn on_tree_changed(&mut self, cx: &mut Context<Self>) {
        let (owner, on_root) = self.cursor_owner(cx);
        if on_root != self.context_on {
            self.context_on = on_root;
            let handle = self.list_focus(cx);
            if on_root {
                add_focus_context(cx, handle, EXPLORER_CONTEXT);
            } else {
                remove_focus_context(cx, &handle, EXPLORER_CONTEXT);
            }
        }
        if let Some(owner) = owner
            && cx
                .try_global::<ConnectionsModel>()
                .is_some_and(|m| m.selected() != Some(owner))
        {
            ConnectionsModel::select(cx, Some(owner));
        }
    }

    // ---- the model --------------------------------------------------------------------------

    fn stamp(&self, cx: &App) -> Stamp {
        let profiles = cx
            .try_global::<ConnectionsModel>()
            .map(|m| {
                m.profiles()
                    .iter()
                    .map(|p| {
                        (
                            Arc::as_ptr(p) as usize,
                            m.live(p.id),
                            session_toasts::is_idle(cx, p.id),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Stamp {
            profiles,
            catalogs: adapter::catalogs_generation(cx),
            l10n: cx
                .try_global::<L10nGlobal>()
                .map_or(0, |g| g.l10n.generation()),
            contributors: contributors_generation(cx),
            filter: self.result.as_ref().map(|r| Arc::as_ptr(r) as usize),
            loads: self.loads.len(),
        }
    }

    /// Rebuilds the model when anything it shows changed. Answers whether a profile (its name
    /// or host) or the language changed: an active filter then runs again (fix round 1, m1).
    fn refresh(&mut self, cx: &mut Context<Self>) -> bool {
        let stamp = self.stamp(cx);
        if self.stamp.as_ref() == Some(&stamp) {
            return false;
        }
        let ptrs = |s: &Stamp| s.profiles.iter().map(|p| p.0).collect::<Vec<_>>();
        let names_moved = self
            .stamp
            .as_ref()
            .is_none_or(|s| ptrs(s) != ptrs(&stamp) || s.l10n != stamp.l10n);
        if self
            .stamp
            .as_ref()
            .is_some_and(|s| s.contributors != stamp.contributors)
        {
            // A contributor changed: its expanded nodes are asked again.
            self.loads.clear();
        }
        self.stamp = Some(self.stamp(cx));
        let profiles: Vec<ProfileInput> = cx
            .try_global::<ConnectionsModel>()
            .map(|m| m.profiles().to_vec())
            .unwrap_or_default()
            .iter()
            .map(|p| profile_input(p, cx))
            .collect();
        let contributed = contributed_places(&profiles, cx);
        let inputs = ExplorerInputs {
            contributed,
            profiles,
            l10n: cx.try_global::<L10nGlobal>().map(|g| g.l10n.clone()),
            lang: ui_language(cx).map(|l| l.to_string()).unwrap_or_default(),
            contributed_children: self.loads.clone(),
            filter: self.result.clone(),
        };
        self.generation += 1;
        let model = ExplorerModel::build(inputs, self.cache.clone(), self.generation);
        self.tree.update(cx, |t, cx| t.set_model(model, cx));
        names_moved
    }

    /// The tree's type-ahead fold follows the filter's toggle (R-M4-50).
    fn sync_fold(&mut self, cx: &mut Context<Self>) {
        let turkic = ui_language(cx).is_some_and(|l| is_turkic(&l));
        let fold = adapter::typeahead_fold(self.tolerant(cx), turkic);
        self.tree.update(cx, |t, _| t.set_typeahead_fold(fold));
    }

    /// The tree's cursor follows [`ConnectionsModel::selected`] when it names another profile
    /// (a cursor row below that profile keeps its place).
    fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let Some(selected) = cx
            .try_global::<ConnectionsModel>()
            .and_then(|m| m.selected())
        else {
            return;
        };
        let (owner, _) = self.cursor_owner(cx);
        if owner != Some(selected) {
            let key = profile_key(selected);
            self.tree.update(cx, |t, cx| t.select(key, cx));
        }
    }

    /// The profiles whose roots are shown, top down.
    fn visible_ids(&self, cx: &App) -> Vec<ProfileId> {
        let tree = self.tree.read(cx);
        let model = tree.model();
        model
            .roots()
            .iter()
            .filter_map(|k| model.profile_root(*k))
            .collect()
    }

    /// A selection the filter hides moves to the first visible profile, or is cleared when
    /// none is visible: the `Explorer` commands never act on a hidden row.
    fn keep_selection_visible(&mut self, cx: &mut Context<Self>) {
        let ids = self.visible_ids(cx);
        let selected = cx.global::<ConnectionsModel>().selected();
        let shown = selected.is_some_and(|s| {
            let tree = self.tree.read(cx);
            ids.contains(&s)
                && tree
                    .selection()
                    .iter()
                    .all(|k| tree.flat().index_of(*k).is_some())
        });
        if selected.is_some() && !shown {
            let first = ids.first().copied();
            ConnectionsModel::select(cx, first);
            if let Some(first) = first {
                let key = profile_key(first);
                self.tree.update(cx, |t, cx| t.select(key, cx));
            }
        }
    }

    // ---- tree events ------------------------------------------------------------------------

    fn on_tree_event(&mut self, event: &TreeEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            // The selected profile follows the cursor's owner (`on_tree_changed`); a selection
            // a collapse cleared leaves the cursor on an ancestor of the same profile.
            TreeEvent::SelectionChanged(_) => self.on_tree_changed(cx),
            TreeEvent::Activate(key) => self.activate(*key, window, cx),
            TreeEvent::ContextMenu(key, at) => {
                // The menu of the row the event names, never the selection's (carries-w2).
                let model = self.tree.read(cx).model().clone();
                if let Some(id) = model.profile_root(*key) {
                    ConnectionsModel::select(cx, Some(id));
                    let at = *at;
                    window.defer(cx, move |window, cx| {
                        open_menu_for(id, Some(at), window, cx)
                    });
                }
                // Every other node's menu is Task 24's (mock 5); a header's commands act on
                // the header's own key (R-W1-5).
            }
            TreeEvent::Expand(key) => self.load_contributed(*key, cx),
            // Retry, the level pill and "Add to scope" are Tasks 24 and 20.
            TreeEvent::Collapse(_)
            | TreeEvent::Retry(_)
            | TreeEvent::Pill(..)
            | TreeEvent::RowAction(_) => {}
        }
    }

    /// Enter or a double click: a profile connects or disconnects (M2a), a table or view
    /// opens its data (R-M4-13, the handler Task 24 installs), a container toggles.
    fn activate(&mut self, key: TreeKey, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.tree.read(cx).model().clone();
        match model.info(key) {
            Some(NodeInfo::Profile(id)) => {
                ConnectionsModel::select(cx, Some(id));
                // Deferred: the action reads this view (the visible-row check), which is
                // leased to this listener until it returns.
                window.defer(cx, |window, cx| {
                    ActionRegistry::run("connection.toggle_connect", window, cx);
                });
            }
            Some(NodeInfo::Object {
                profile,
                key: object,
            }) if matches!(
                object.kind,
                hacivat_meta::ObjectKind::Table
                    | hacivat_meta::ObjectKind::View
                    | hacivat_meta::ObjectKind::MaterializedView
                    | hacivat_meta::ObjectKind::ForeignTable
            ) =>
            {
                window.defer(cx, move |window, cx| {
                    crate::explorer::actions::open_data(profile, object, window, cx);
                });
            }
            Some(_) if model.children(key).expandable() => {
                self.tree.update(cx, |t, cx| {
                    let open = t.is_expanded(key);
                    t.set_expanded(key, !open, cx);
                });
            }
            Some(_) | None => {}
        }
    }

    /// An expanded contributed node with lazy children asks its contributor (UI thread).
    fn load_contributed(&mut self, key: TreeKey, cx: &mut Context<Self>) {
        let model = self.tree.read(cx).model().clone();
        if !matches!(model.info(key), Some(NodeInfo::Contributed { .. }))
            || self.loads.contains_key(&key)
        {
            return;
        }
        let top = model
            .inputs()
            .contributed
            .iter()
            .flat_map(|p| p.nodes.iter())
            .chain(self.loads.values().flat_map(|l| l.nodes.iter()))
            .find(|n| n.key == key)
            .cloned();
        let Some(node) = top else {
            return;
        };
        let list = contributors(cx);
        let state = match &node.children {
            ContributedChildren::Lazy(f) => f(cx),
            ContributedChildren::Leaf => return,
        };
        let nodes = match &state {
            ChildState::Loaded(keys) => keys
                .iter()
                .filter_map(|k| list.iter().find_map(|c| c.child_node(*k, cx)))
                .collect(),
            _ => Vec::new(),
        };
        self.loads.insert(key, ContributedLoad { state, nodes });
        self.refresh(cx);
        if self.result.is_some() {
            self.query_changed(cx);
        }
    }

    /// Applies rename hints to the tree's expansion (T10's `CatalogDelta::renamed`).
    fn apply_renames(&mut self, renames: &[(TreeKey, TreeKey)], cx: &mut Context<Self>) {
        self.tree.update(cx, |t, cx| {
            let mut keys = t.expanded().clone();
            let mut moved = false;
            for (from, to) in renames {
                if keys.remove(from) {
                    keys.insert(*to);
                    moved = true;
                }
            }
            if moved {
                t.set_expanded_keys(keys, cx);
            }
        });
        if let Some(saved) = self.saved_expansion.as_mut() {
            for (from, to) in renames {
                if saved.remove(from) {
                    saved.insert(*to);
                }
            }
        }
    }

    // ---- the filter -------------------------------------------------------------------------

    /// The query or the toggle changed: match again (latest wins).
    fn query_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.filter_seq += 1;
        // A running job sees the new number and stops (fix round 1, I3).
        self.latest.store(self.filter_seq, Ordering::Relaxed);
        self.filter_task = None;
        self.catalogs_seen = adapter::catalogs_generation(cx);
        if query.is_empty() {
            if self.result.take().is_some() {
                self.refresh(cx);
                if let Some(saved) = self.saved_expansion.take() {
                    self.tree.update(cx, |t, cx| t.set_expanded_keys(saved, cx));
                }
            }
            self.keep_selection_visible(cx);
            cx.notify();
            return;
        }
        let job = self
            .tree
            .read(cx)
            .model()
            .filter_job(&query, self.tolerant(cx));
        if job.names() <= SYNC_FILTER_NAMES {
            let result = job.run();
            self.apply_filter(result, cx);
            return;
        }
        let seq = self.filter_seq;
        let job = job.with_cancel(self.latest.clone(), seq);
        let run = cx.background_spawn(async move { job.run_cancellable() });
        self.filter_task = Some(cx.spawn(async move |this, cx| {
            let Some(result) = run.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.filter_seq == seq {
                    this.apply_filter(result, cx);
                }
            });
        }));
    }

    fn apply_filter(&mut self, result: FilterResult, cx: &mut Context<Self>) {
        if self.saved_expansion.is_none() {
            self.saved_expansion = Some(self.tree.read(cx).expanded().clone());
        }
        let ancestors = result.ancestors.clone();
        self.result = Some(Arc::new(result));
        self.refresh(cx);
        // Matches reveal their ancestors.
        self.tree
            .update(cx, |t, cx| t.set_expanded_keys(ancestors, cx));
        self.keep_selection_visible(cx);
        cx.notify();
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |f, cx| {
            f.set_value("");
            cx.notify();
        });
        self.query_changed(cx);
        let handle = self.filter_focus(cx);
        window.focus(&handle, cx);
    }

    fn flip_tolerant(&mut self, cx: &mut Context<Self>) {
        self.tolerant = Some(!self.tolerant(cx));
        self.sync_fold(cx);
        if !self.query(cx).is_empty() {
            self.query_changed(cx);
        }
        cx.notify();
    }

    // ---- rendering --------------------------------------------------------------------------

    fn filter_row(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let t = theme.tokens;
        let c = &t.colors;
        let handle = self.filter_focus(cx).tab_stop(true);
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .mx(px(t.spacing.md))
            .mt(px(t.spacing.md))
            .mb(px(t.spacing.xs))
            .gap(px(t.spacing.sm))
            .child(
                Icon::new(IconName::Search)
                    .size(IconSize::Chrome)
                    .color(c.text_3.to_hsla()),
            )
            .child(
                // A tab stop for the field (the input's own handle is not one).
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .track_focus(&handle)
                    // ## fix round 1 (m8, mock surface 4): ↓ moves into the tree.
                    .capture_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                        let m = e.keystroke.modifiers;
                        let plain = !(m.control || m.alt || m.shift || m.platform || m.function);
                        let composing = this.filter.read(cx).is_composing();
                        if e.keystroke.key == "down" && plain && !composing {
                            let handle = this.list_focus(cx);
                            window.focus(&handle, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(self.filter.clone()),
            );
        if let Some(result) = &self.result {
            row = row.child(
                div().flex_none().child(
                    Label::new(
                        tr(
                            cx,
                            ids::hv::EXPLORER_FILTER_COUNT,
                            ids::hv::HvExplorerFilterCount {
                                n: Count(result.count as u64),
                            },
                        ),
                        TextSource::Fluent,
                        t.type_ramp.mono_micro,
                        c.text_3.to_hsla(),
                    )
                    .id("explorer-filter-count"),
                ),
            );
        }
        // The Turkish-tolerant toggle (D-074; the find bar's, surface 4).
        let name = tr_id(cx, "hv-find-tolerant");
        let tip = tr_id(cx, "hv-find-tolerant-tip");
        let entity = cx.entity();
        row = row.child(toggle_element(
            ToggleSpec {
                id: "explorer-tolerant",
                node: ("explorer-tolerant-node", 0),
                icon: IconName::Languages,
                name,
                tip: tip.clone(),
                description: Some(tip),
                on: self.tolerant(cx),
                handle: self.tolerant_focus.clone(),
            },
            window,
            move |_, cx| entity.update(cx, |v, cx| v.flip_tolerant(cx)),
            cx,
        ));
        if theme.mode == ThemeMode::Light {
            row = row.bg(c.panel.to_hsla());
        }
        row.into_any_element()
    }

    /// "Searches loaded names only…" under the field: when connections, databases or schemas
    /// have no loaded catalog, and when relations' columns are not loaded (fix round 1, I2:
    /// what the filter could not search is said, never shown as "no match").
    fn filter_hints(&self, theme: &Theme, cx: &App) -> Vec<AnyElement> {
        let Some(result) = self.result.as_ref() else {
            return Vec::new();
        };
        let t = theme.tokens;
        let c = &t.colors;
        let line = |text: SharedString, id: &'static str| {
            div()
                .flex()
                .flex_row()
                .flex_none()
                .gap(px(t.spacing.sm))
                .mx(px(t.spacing.md))
                .mb(px(t.spacing.xs))
                .child(
                    div().flex_none().pt(px(2.0)).child(
                        Icon::new(IconName::Info)
                            .size(IconSize::Chrome)
                            .color(c.text_2.to_hsla()),
                    ),
                )
                .child(paragraph(text, t.type_ramp.body_sm, c.text_2.to_hsla(), id))
                .into_any_element()
        };
        let mut out = Vec::new();
        if result.unsearched > 0 {
            out.push(line(
                tr(
                    cx,
                    ids::hv::EXPLORER_FILTER_NOT_LOADED,
                    ids::hv::HvExplorerFilterNotLoaded {
                        n: Count(result.unsearched as u64),
                    },
                ),
                "explorer-filter-not-loaded",
            ));
        }
        if result.unloaded > 0 {
            out.push(line(
                tr(
                    cx,
                    ids::hv::EXPLORER_FILTER_UNLOADED,
                    ids::hv::HvExplorerFilterUnloaded {
                        n: Count(result.unloaded as u64),
                    },
                ),
                "explorer-filter-unloaded",
            ));
        }
        out
    }

    fn empty(&self, theme: &Theme, cx: &mut App) -> AnyElement {
        let t = theme.tokens;
        let c = &t.colors;
        let button =
            |id: &'static str, label: SharedString, kind, action: &'static str, cx: &App| {
                let mut b = Button::new(id, label)
                    .kind(kind)
                    .on_click(move |_, window, cx| {
                        ActionRegistry::run(action, window, cx);
                    });
                if let Some(keys) = shortcut_label(cx, action) {
                    b = b.key_hint(keys.into());
                }
                b
            };
        let body = paragraph(
            tr(cx, ids::hv::EXPLORER_EMPTY_BODY, NoArgs),
            t.type_ramp.body_sm,
            c.text_2.to_hsla(),
            "explorer-empty-body",
        );
        div()
            .flex()
            .flex_col()
            .flex_1()
            .justify_center()
            .px(px(t.spacing.s_3xl))
            .gap(px(t.spacing.lg))
            .child(
                Label::new(
                    tr(cx, ids::hv::EXPLORER_EMPTY, NoArgs),
                    TextSource::Fluent,
                    TypeStyle {
                        weight: 600,
                        ..t.type_ramp.body
                    },
                    c.text.to_hsla(),
                )
                .id("explorer-empty"),
            )
            .child(body)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(t.spacing.sm))
                    .child(button(
                        "explorer-new",
                        tr(cx, ids::hv::EXPLORER_NEW, NoArgs),
                        ButtonKind::Primary,
                        "connection.new",
                        cx,
                    ))
                    .child(button(
                        "explorer-paste",
                        tr(cx, ids::hv::EXPLORER_PASTE, NoArgs),
                        ButtonKind::Default,
                        "connection.paste_url",
                        cx,
                    ))
                    .child(button(
                        "explorer-import",
                        tr(cx, ids::hv::EXPLORER_IMPORT, NoArgs),
                        ButtonKind::Ghost,
                        "connection.import_pg_service",
                        cx,
                    )),
            )
            .into_any_element()
    }

    /// "No objects match" (surface 4): the query quoted, and Clear filter.
    fn no_match(&self, theme: &Theme, query: &str, cx: &mut Context<Self>) -> AnyElement {
        let t = theme.tokens;
        let c = &t.colors;
        let entity = cx.entity();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .px(px(t.spacing.s_3xl))
            .pt(px(t.spacing.s_2xl))
            .gap(px(t.spacing.md))
            .child(
                Label::new(
                    tr(cx, ids::hv::EXPLORER_FILTER_NONE, NoArgs),
                    TextSource::Fluent,
                    TypeStyle {
                        weight: 600,
                        ..t.type_ramp.body
                    },
                    c.text.to_hsla(),
                )
                .id("explorer-filter-none"),
            )
            .child(paragraph(
                tr(
                    cx,
                    ids::hv::EXPLORER_FILTER_NONE_BODY,
                    ids::hv::HvExplorerFilterNoneBody {
                        query: Text(query.to_string()),
                    },
                ),
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
                "explorer-filter-none-body",
            ))
            .child(
                div().flex().flex_row().child(
                    Button::new(
                        "explorer-filter-clear",
                        tr(cx, ids::hv::EXPLORER_FILTER_CLEAR, NoArgs),
                    )
                    .kind(ButtonKind::Default)
                    .on_click(move |_, window, cx| {
                        entity.update(cx, |v, cx| v.clear_filter(window, cx));
                    }),
                ),
            )
            .into_any_element()
    }

    fn list(&self, theme: &Theme) -> AnyElement {
        let t = theme.tokens;
        div()
            .id("explorer-list")
            .debug_selector(|| "explorer-list".to_string())
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden()
            .pb(px(t.spacing.md))
            // ## m2a-t21-shots: the frame checks treat this clip as a scroll viewport.
            .child(hacivat_ui::text::scroll_viewport())
            .child(self.tree.clone())
            .into_any_element()
    }
}

impl Render for ExplorerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        if cx.try_global::<ConnectionsModel>().is_none() {
            // P17: no model, the M1 explorer.
            return tiles::explorer(&theme, cx).into_any_element();
        }
        // A language switch or a contributor's change shows on the next frame.
        let stamp = self.stamp(cx);
        if self.stamp.as_ref() != Some(&stamp) {
            let relang = self.stamp.as_ref().is_some_and(|s| s.l10n != stamp.l10n);
            self.refresh(cx);
            if relang {
                self.sync_fold(cx);
            }
        }
        // The focused tree with nothing selected selects its first profile (after this
        // frame), so Enter and the other `Explorer` bindings have a target.
        if self.list_focus(cx).is_focused(window)
            && cx.global::<ConnectionsModel>().selected().is_none()
            && let Some(first) = self.visible_ids(cx).first().copied()
        {
            let tree = self.tree.downgrade();
            window.defer(cx, move |_, cx| {
                ConnectionsModel::select(cx, Some(first));
                if let Some(tree) = tree.upgrade() {
                    tree.update(cx, |t, cx| t.select(profile_key(first), cx));
                }
            });
        }
        let m = cx.global::<ConnectionsModel>();
        let empty = m.profiles().is_empty();
        let c = &theme.tokens.colors;
        let query = self.query(cx);
        let body = if empty {
            self.empty(&theme, cx)
        } else if self.result.as_ref().is_some_and(|r| r.nothing_matches()) {
            self.no_match(&theme, &query, cx)
        } else {
            self.list(&theme)
        };
        let filter_row = self.filter_row(&theme, window, cx);
        let hint = self.filter_hints(&theme, cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(c.side.to_hsla())
            .child(panel_header(&theme).child(panel_title(
                &theme,
                tr(cx, ids::hv::PANEL_EXPLORER, NoArgs),
                "panel-explorer-title",
            )))
            .child(filter_row)
            .children(hint)
            .child(body)
            .into_any_element()
    }
}

// ## m2a-t18-explorer: shots fixtures (R28, P19)

#[cfg(feature = "shots")]
pub use fixtures::*;

#[cfg(feature = "shots")]
mod fixtures {
    use std::sync::Arc;

    use hacivat_driver::Support;
    use hacivat_proto::{DbmsKind, EnvTag, ErrClass};
    use hacivat_session::LiveState;
    use hacivat_session::pipeline::{Progress, TrailStep};
    use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint};
    use hacivat_ui_core::gpui_surface::{App, Window};

    use crate::connections::capsule::sample_live_info;
    use crate::connections::error_view::sample_error;
    use crate::connections::model::ConnectionsModel;

    fn add(cx: &mut App, name: &str, engine: DbmsKind, env: Option<EnvTag>, read_only: bool) {
        let endpoint = if engine == DbmsKind::Sqlite {
            ProfileEndpoint::File {
                path: "/Users/demo/app.db".into(),
                create_if_missing: false,
            }
        } else {
            ProfileEndpoint::Server {
                host: format!("{name}.db.internal"),
                port: None,
            }
        };
        let profile = Profile {
            name: name.to_string(),
            engine,
            environment: env,
            read_only,
            endpoint,
            ..Profile::default()
        };
        ConnectionsModel::save(
            cx,
            ProfileDraft {
                is_new: true,
                ..ProfileDraft::edit(profile)
            },
        )
        .detach();
    }

    /// Shows `name` in `state` (the mirror's value; nothing connects).
    fn live(cx: &mut App, name: &str, state: LiveState) {
        let id = cx
            .global::<ConnectionsModel>()
            .profiles()
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.id);
        if let Some(id) = id {
            cx.global_mut::<ConnectionsModel>().set_live(id, state);
        }
    }

    /// The explorer with no connection (NOTES §8 empty state).
    pub fn empty_fixture(_window: &mut Window, _cx: &mut App) {}

    /// A list over every group, one row selected.
    pub fn list_fixture(window: &mut Window, cx: &mut App) {
        add(
            cx,
            "orders-prod",
            DbmsKind::Postgres,
            Some(EnvTag::Production),
            true,
        );
        add(cx, "billing", DbmsKind::MySql, Some(EnvTag::Staging), false);
        add(
            cx,
            "orders-test",
            DbmsKind::MariaDb,
            Some(EnvTag::Test),
            false,
        );
        add(cx, "scratch", DbmsKind::Postgres, Some(EnvTag::Dev), false);
        add(cx, "app.db", DbmsKind::Sqlite, Some(EnvTag::Local), false);
        add(cx, "legacy", DbmsKind::Postgres, None, false);
        // ## m2a-t21-shots: every live mark (NOTES §8): connected with latency (and the
        // plaintext flag), connecting (the run marker), failed (the alert).
        live(
            cx,
            "billing",
            LiveState::Connected(Arc::new(sample_live_info(false, false, Support::Supported))),
        );
        live(
            cx,
            "orders-test",
            LiveState::Connecting(Progress {
                trail: Vec::new(),
                current: Some(TrailStep::SignIn),
                started: std::time::Instant::now(),
            }),
        );
        live(
            cx,
            "scratch",
            LiveState::Failed(Arc::new(sample_error(
                ErrClass::NetworkRefused,
                TrailStep::Connect,
                None,
                "",
            ))),
        );
        let first = cx
            .global::<ConnectionsModel>()
            .profiles()
            .first()
            .map(|p| p.id);
        ConnectionsModel::select(cx, first);
        super::focus_list(window, cx);
    }

    /// The list with the connection menu open on the selection.
    pub fn menu_fixture(window: &mut Window, cx: &mut App) {
        list_fixture(window, cx);
        super::open_context_menu(None, window, cx);
    }
}
