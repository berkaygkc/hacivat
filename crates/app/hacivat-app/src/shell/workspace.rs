//! The workspace view: title bar, tiles, status bar, layers and toasts, laid out like harman
//! (DESIGN.md Layout). It owns the tile layout, the pane tree with its tabs, the capsule and the
//! layer stack, and saves the layout to `workspace.json` (debounced, on the background
//! executor) whenever it changes.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use hacivat_l10n::{NoArgs, Status, Text, ids};
use hacivat_settings::{LanguagePref, ThemePref, fsio, workspace_state};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::capsule::{Capsule, CapsuleModel, EnvKind};
use hacivat_ui::dialog::{Dialog, DialogEvent, DialogSpec};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};
use hacivat_ui::panes::{
    Axis, MAX_RATIO, MIN_RATIO, PaneId, PaneNode, PaneTree, TabId, TabSpec, TabStrip,
};
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented};
use hacivat_ui::statusbar::{StatusBar, status_label};
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui::titlebar::TitleBar;
use hacivat_ui::toast::ToastStack;
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, Context, Decorations, Div, Entity, EntityId,
    EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, ResizeEdge,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, WindowHandle,
    anchored, div, point, px, relative,
};
use hacivat_ui_core::keymap::ContextStack;
use hacivat_ui_core::theme::{Theme, ThemeMode, popover_surface};
use hacivat_ui_core::window::{
    ChromeKind, WorkspaceWindowSpec, chrome_kind, open_workspace_window,
};
use serde::{Deserialize, Serialize};

use crate::PendingChord;
use crate::l10n_global::{L10nGlobal, WINDOW_TITLE, switch_locale, tr, ui_language};
use crate::shell::hooks::{focused_contexts, hooks, key_capture};
use crate::shell::keymap_state::shortcut_label;
use crate::shell::layers::{DIALOG_CONTEXT, Layer, Layers, MENU_CONTEXT};
use crate::shell::persist::LatestWriter;
use crate::shell::tab_content::{TabContentRegistry, TabKind};
use crate::shell::tabs::{CONSOLE_CONTEXT, context_of, title_of};
use crate::shell::tiles::TileBounds;
use crate::shell::tiles::{self, Seam, TileLayout};
use crate::shell::{AppState, update_settings};
use crate::theme_pref::{apply_theme, follow_appearance};

/// The `workspace.json` format version this build writes and reads.
pub const WORKSPACE_STATE_VERSION: u32 = 1;
/// How long a layout change waits before it is saved; a later change restarts the wait.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);
/// The workspace's own key context, the bottom of every context stack.
pub const WORKSPACE_CONTEXT: &str = "Workspace";
/// The action the title bar's palette field runs (registered by Task 16).
const PALETTE_ACTION: &str = "workspace.command_palette";
/// The action the Windows/Linux app-menu button runs (registered by Task 16, F14).
const APP_MENU_ACTION: &str = "app.show_menu";
/// The key context of an anchored popover layer (`open_anchored`).
pub const POPOVER_CONTEXT: &str = "Popover";

/// The harman demo connection (`--vf-demo`): a production capsule. The name and engine are
/// connection data, not chrome text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoCapsule {
    pub name: SharedString,
    pub engine: SharedString,
}

impl DemoCapsule {
    /// The connection the approved harman mock shows.
    pub fn harman() -> Self {
        Self {
            name: "orders-prod".into(),
            engine: "PostgreSQL 16".into(),
        }
    }
}

/// A tab as `workspace.json` stores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SavedKind {
    Console { n: u32 },
    Registered { name: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SavedTab {
    id: TabId,
    #[serde(flatten)]
    kind: SavedKind,
}

/// The persisted part of the workspace (`"version"` is added by `workspace_state::save`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SavedState {
    tiles: TileLayout,
    panes: PaneTree,
    tabs: Vec<SavedTab>,
}

/// The workspace window's root view.
pub struct Workspace {
    tiles: TileLayout,
    panes: PaneTree,
    tabs: BTreeMap<TabId, TabKind>,
    /// Built views of registered tabs, by tab.
    tab_views: HashMap<TabId, AnyView>,
    /// The key context of each registered kind in use.
    kind_contexts: HashMap<&'static str, &'static str>,
    next_tab: u32,
    capsule: Option<DemoCapsule>,
    /// Views from the `explorer` and `connection_panel` hooks, built once (P17).
    explorer_view: Option<AnyView>,
    connection_panel_view: Option<AnyView>,
    layers: Layers,
    focus: FocusHandle,
    drag: Option<Seam>,
    state_path: Option<PathBuf>,
    /// Writes `workspace.json`; a pending state is flushed on quit and when the view goes.
    saver: LatestWriter<(PathBuf, serde_json::Value)>,
    save_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Opens the workspace window (restoring `workspace.json` from `AppState`'s paths) and focuses
/// it. `demo` shows the harman production capsule.
pub fn open_workspace(
    cx: &mut App,
    demo: Option<DemoCapsule>,
) -> anyhow::Result<WindowHandle<Workspace>> {
    let spec = WorkspaceWindowSpec {
        title: WINDOW_TITLE.into(),
        ..Default::default()
    };
    let handle = open_workspace_window(cx, spec, move |window, cx| {
        cx.new(|cx| Workspace::new(demo, window, cx))
    })?;
    handle.update(cx, |ws, window, cx| window.focus(&ws.focus, cx))?;
    Ok(handle)
}

impl Workspace {
    pub fn new(demo: Option<DemoCapsule>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state_path = cx
            .try_global::<AppState>()
            .map(|s| s.paths.workspace_json());
        let subscriptions = vec![
            cx.observe_global::<PendingChord>(|_, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, window, cx| {
                follow_appearance(cx, window.appearance());
            }),
            // A layout change still waiting for its debounce is written before the app quits.
            cx.on_app_quit(|this, _| {
                this.saver.flush();
                async {}
            }),
        ];
        // ... and before the window's view goes (the window closed).
        cx.on_release(|this, _| this.saver.flush()).detach();
        // The hook views are built once; hooks are set before the workspace opens (P17).
        let slots = hooks(cx);
        let explorer_view = slots.explorer.map(|build| build(window, cx));
        let connection_panel_view = slots.connection_panel.map(|build| build(window, cx));
        let mut ws = Self {
            tiles: TileLayout::default(),
            panes: PaneTree::new(TabId(0)),
            tabs: BTreeMap::from([(TabId(0), TabKind::Console { n: 1 })]),
            tab_views: HashMap::new(),
            kind_contexts: HashMap::new(),
            next_tab: 1,
            capsule: demo,
            explorer_view,
            connection_panel_view,
            layers: Layers::default(),
            focus: cx.focus_handle(),
            drag: None,
            state_path,
            saver: LatestWriter::new(write_state),
            save_task: None,
            _subscriptions: subscriptions,
        };
        let t = Theme::get(cx).tokens;
        let size = window.viewport_size();
        let bounds = TileBounds {
            window: (f32::from(size.width), f32::from(size.height)),
            top: t.spacing.titlebar_height,
            bottom: t.spacing.statusbar_height,
        };
        ws.restore(bounds, cx);
        ws
    }

    // ---- persistence -------------------------------------------------------------------------

    /// Restores `workspace.json` when it is readable and consistent; otherwise keeps one
    /// `Console 1`. A file that parses as JSON but does not describe a valid workspace is set
    /// aside (`*.corrupt-<ts>`) before anything overwrites it.
    fn restore(&mut self, bounds: TileBounds, cx: &App) {
        let Some(path) = &self.state_path else {
            return;
        };
        let Some(value) = workspace_state::load(path, WORKSPACE_STATE_VERSION) else {
            return;
        };
        let registry = cx.try_global::<TabContentRegistry>();
        let restored = serde_json::from_value::<SavedState>(value)
            .map_err(|e| e.to_string())
            .and_then(|saved| {
                validate(
                    saved,
                    |name| registry.and_then(|r| r.kind_named(name)),
                    bounds,
                )
                .map_err(str::to_string)
            });
        let restored = match restored {
            Ok(restored) => restored,
            Err(reason) => {
                tracing::warn!(path = %path.display(), %reason, "workspace: state set aside");
                let _ = fsio::quarantine(path, "corrupt", SystemTime::now());
                return;
            }
        };
        for kind in restored.tabs.values() {
            self.note_context(*kind, cx);
        }
        self.next_tab = restored.next_tab;
        self.tiles = restored.tiles;
        self.panes = restored.panes;
        self.tabs = restored.tabs;
    }

    fn saved_state(&self) -> SavedState {
        SavedState {
            tiles: self.tiles,
            panes: self.panes.clone(),
            tabs: self
                .tabs
                .iter()
                .map(|(id, kind)| SavedTab {
                    id: *id,
                    kind: match kind {
                        TabKind::Console { n } => SavedKind::Console { n: *n },
                        TabKind::Registered(name) => SavedKind::Registered {
                            name: (*name).to_string(),
                        },
                    },
                })
                .collect(),
        }
    }

    /// Repaints and saves the layout after `SAVE_DEBOUNCE`; a later change restarts the wait.
    fn layout_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        let Some(path) = self.state_path.clone() else {
            return;
        };
        let Ok(value) = serde_json::to_value(self.saved_state()) else {
            return;
        };
        self.save_task = Some(self.saver.submit((path, value), Some(SAVE_DEBOUNCE), cx));
    }

    /// The pane tree and tile layout that `workspace.json` persists.
    pub fn layout(&self) -> (PaneTree, TileLayout) {
        (self.panes.clone(), self.tiles)
    }

    // ---- tabs --------------------------------------------------------------------------------

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// The focused pane's active tab.
    pub fn active_tab(&self) -> Option<TabId> {
        let pane = self.panes.panes.get(&self.panes.focused)?;
        pane.tabs.get(pane.active).copied()
    }

    pub fn active_tab_kind(&self) -> Option<TabKind> {
        self.active_tab().and_then(|t| self.tabs.get(&t).copied())
    }

    /// Writes a pending `workspace.json` now (before a restart; quit and close do it themselves).
    pub fn flush_layout(&self) {
        self.saver.flush();
    }

    /// The title of `tab` in the current language.
    pub fn tab_title(&self, tab: TabId, cx: &App) -> SharedString {
        self.tabs
            .get(&tab)
            .map(|kind| title_of(*kind, cx))
            .unwrap_or_default()
    }

    /// A new tab ID for `kind`; `None` once the ID space is used up (never in practice: restore
    /// rejects a state whose IDs leave no room).
    fn fresh_tab(&mut self, kind: TabKind) -> Option<TabId> {
        let id = TabId(self.next_tab);
        self.next_tab = self.next_tab.checked_add(1)?;
        self.tabs.insert(id, kind);
        Some(id)
    }

    /// The next console number: one more than the highest open console.
    fn next_console(&self) -> u32 {
        self.tabs
            .values()
            .filter_map(|k| match k {
                TabKind::Console { n } => Some(*n),
                TabKind::Registered(_) => None,
            })
            .max()
            .map_or(1, |n| n.saturating_add(1))
    }

    /// Adds `tab` to the focused pane and activates it. With no pane left (every tab closed),
    /// the tree starts over with this tab.
    fn add_to_focused(&mut self, tab: TabId) {
        let focused = self.panes.focused;
        match self.panes.panes.get_mut(&focused) {
            Some(pane) => {
                pane.tabs.push(tab);
                pane.active = pane.tabs.len() - 1;
            }
            None => self.panes = PaneTree::new(tab),
        }
    }

    /// Opens `Console <n>` in the focused pane.
    pub fn new_console_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let n = self.next_console();
        let Some(tab) = self.fresh_tab(TabKind::Console { n }) else {
            return;
        };
        self.add_to_focused(tab);
        self.layout_changed(cx);
    }

    /// Closes the focused pane's active tab.
    pub fn close_active_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        self.panes.close_tab(self.panes.focused, tab);
        self.tabs.remove(&tab);
        self.tab_views.remove(&tab);
        self.layout_changed(cx);
    }

    pub fn activate_next_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.panes.next_tab();
        self.layout_changed(cx);
    }

    pub fn activate_prev_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.panes.prev_tab();
        self.layout_changed(cx);
    }

    /// Splits the focused pane along `axis`; the new pane opens a new console and takes the
    /// focus.
    pub fn split_focused(&mut self, axis: Axis, _window: &mut Window, cx: &mut Context<Self>) {
        let n = self.next_console();
        let Some(tab) = self.fresh_tab(TabKind::Console { n }) else {
            return;
        };
        if self.panes.panes.contains_key(&self.panes.focused) {
            self.panes.split(self.panes.focused, axis, tab);
        } else {
            self.panes = PaneTree::new(tab);
        }
        self.layout_changed(cx);
    }

    /// Moves the focus to the next pane in reading order, wrapping around.
    pub fn focus_next_pane(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let leaves = self.panes.leaves();
        if let Some(at) = leaves.iter().position(|p| *p == self.panes.focused) {
            self.panes.focused = leaves[(at + 1) % leaves.len()];
            self.layout_changed(cx);
        }
    }

    pub fn toggle_explorer(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.tiles.explorer_visible = !self.tiles.explorer_visible;
        self.layout_changed(cx);
    }

    /// Activates the tab of registered `kind`, or opens one in the focused pane. A kind nobody
    /// registered opens a tab that shows an empty state.
    pub fn open_registered_tab(
        &mut self,
        kind: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = self
            .tabs
            .iter()
            .find(|(_, k)| **k == TabKind::Registered(kind))
            .map(|(id, _)| *id);
        if let Some(tab) = existing {
            let found =
                self.panes.panes.iter().find_map(|(pane, p)| {
                    p.tabs.iter().position(|t| *t == tab).map(|i| (*pane, i))
                });
            if let Some((pane, idx)) = found {
                self.panes.activate(pane, idx);
            }
        } else {
            let Some(tab) = self.fresh_tab(TabKind::Registered(kind)) else {
                return;
            };
            self.note_context(TabKind::Registered(kind), cx);
            self.build_view(tab, window, cx);
            self.add_to_focused(tab);
        }
        self.layout_changed(cx);
    }

    /// Builds the view of a registered tab once.
    fn build_view(&mut self, tab: TabId, window: &mut Window, cx: &mut App) -> Option<AnyView> {
        if let Some(view) = self.tab_views.get(&tab) {
            return Some(view.clone());
        }
        let TabKind::Registered(kind) = *self.tabs.get(&tab)? else {
            return None;
        };
        let build = cx
            .try_global::<TabContentRegistry>()?
            .get(kind)?
            .build
            .clone();
        let view = build(window, cx);
        self.tab_views.insert(tab, view.clone());
        Some(view)
    }

    fn activate_tab(&mut self, pane: PaneId, tab: TabId, cx: &mut Context<Self>) {
        if let Some(idx) = self
            .panes
            .panes
            .get(&pane)
            .and_then(|p| p.tabs.iter().position(|t| *t == tab))
        {
            self.panes.activate(pane, idx);
            self.layout_changed(cx);
        }
    }

    fn close_tab(&mut self, pane: PaneId, tab: TabId, cx: &mut Context<Self>) {
        self.panes.close_tab(pane, tab);
        self.tabs.remove(&tab);
        self.tab_views.remove(&tab);
        self.layout_changed(cx);
    }

    // ---- layers ------------------------------------------------------------------------------

    /// Pushes `layer` on top. Without its own `restore_focus` it records the focused element, so
    /// popping it gives the focus back.
    pub fn push_layer(&mut self, layer: Layer, window: &mut Window, cx: &mut Context<Self>) {
        self.push_layer_with(layer, None, window, cx);
    }

    fn push_layer_with(
        &mut self,
        mut layer: Layer,
        subscription: Option<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if layer.restore_focus.is_none() {
            layer.restore_focus = window.focused(cx);
        }
        self.layers.push(layer, subscription);
        cx.notify();
    }

    /// Removes the topmost layer, gives the focus back and returns the layer's ID.
    pub fn pop_layer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<&'static str> {
        let layer = self.layers.pop()?;
        Some(self.closed(layer, window, cx))
    }

    /// Removes the layer showing `view` (wherever it is in the stack).
    fn remove_layer(&mut self, view: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(layer) = self.layers.remove_view(view) {
            self.closed(layer, window, cx);
        }
    }

    fn closed(
        &mut self,
        layer: Layer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> &'static str {
        match &layer.restore_focus {
            Some(handle) => window.focus(handle, cx),
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
        layer.id
    }

    /// The layer IDs, bottom to top.
    pub fn layer_ids(&self) -> Vec<&'static str> {
        self.layers.ids()
    }

    /// Opens a modal dialog as layer `dialog` (context `Dialog`); it closes on any of its events
    /// (a button or Escape). Subscribe to the returned dialog for which button was pressed.
    pub fn open_dialog(
        &mut self,
        spec: DialogSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Dialog> {
        let restore = window.focused(cx);
        let dialog = Dialog::new(spec, window, cx);
        let view = dialog.entity_id();
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            move |this, dialog, _: &DialogEvent, window, cx| {
                // Removing the layer drops the last strong handle; keep the dialog alive until
                // this event has reached every subscriber (a caller's included).
                let keep = dialog.clone();
                this.remove_layer(view, window, cx);
                cx.defer(move |_| drop(keep));
            },
        );
        let layer = Layer {
            id: "dialog",
            context: DIALOG_CONTEXT,
            view: dialog.clone().into(),
            restore_focus: restore,
            modal: true,
        };
        self.push_layer_with(layer, Some(subscription), window, cx);
        dialog
    }

    /// Opens a popup menu at `anchor` as layer `menu` (context `Menu`). The chosen item's action
    /// runs through `ActionRegistry::run` after the menu closes.
    pub fn open_popup_menu(
        &mut self,
        anchor: Point<Pixels>,
        items: Vec<MenuItemSpec>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<PopupMenu> {
        let restore = window.focused(cx);
        let menu = PopupMenu::new(anchor, items, window, cx);
        let view = menu.entity_id();
        let subscription = cx.subscribe_in(
            &menu,
            window,
            move |this, menu, event: &PopupMenuEvent, window, cx| {
                // As in `open_dialog`: the caller's subscription still gets this event.
                let keep = menu.clone();
                this.remove_layer(view, window, cx);
                cx.defer(move |_| drop(keep));
                if let PopupMenuEvent::Chosen(action) = event {
                    let action = action.clone();
                    // Runs once this update is done, so an action may update the workspace.
                    window.defer(cx, move |window, cx| {
                        ActionRegistry::run(&action, window, cx);
                    });
                }
            },
        );
        let layer = Layer {
            id: "menu",
            context: MENU_CONTEXT,
            view: menu.clone().into(),
            restore_focus: restore,
            modal: false,
        };
        self.push_layer_with(layer, Some(subscription), window, cx);
        menu
    }

    /// Opens `view` as a non-modal layer `id` (context `Popover`) with its top-left at `anchor`
    /// (window coordinates); a press outside it closes it.
    pub fn open_anchored(
        &mut self,
        id: &'static str,
        anchor: Point<Pixels>,
        view: AnyView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let restore = window.focused(cx);
        let host = cx.new(|_| AnchoredHost { anchor, view });
        let host_id = host.entity_id();
        let subscription = cx.subscribe_in(
            &host,
            window,
            move |this, host, _: &AnchorDismissed, window, cx| {
                // As in `open_dialog`: the host lives until this event reached every subscriber.
                let keep = host.clone();
                this.remove_layer(host_id, window, cx);
                cx.defer(move |_| drop(keep));
            },
        );
        let layer = Layer {
            id,
            context: POPOVER_CONTEXT,
            view: host.into(),
            restore_focus: restore,
            modal: false,
        };
        self.push_layer_with(layer, Some(subscription), window, cx);
    }

    /// The top layer's context while a modal layer is open: the only context whose bindings
    /// resolve then, besides `app.quit` (P47).
    pub fn modal_context(&self) -> Option<&'static str> {
        self.layers.modal_context()
    }

    /// The active key contexts, innermost last: `Workspace`, the active tab's context, each
    /// layer's context from the bottom up, then each registered focus context whose handle is
    /// focused (`register_focus_context`).
    pub fn context_stack(&self, window: &Window, cx: &App) -> ContextStack {
        let mut stack = vec![WORKSPACE_CONTEXT];
        stack.extend(self.active_tab_kind().and_then(|k| self.tab_context(k)));
        stack.extend(self.layers.contexts());
        stack.extend(focused_contexts(window, cx));
        ContextStack(stack)
    }

    fn tab_context(&self, kind: TabKind) -> Option<&'static str> {
        match kind {
            TabKind::Console { .. } => Some(CONSOLE_CONTEXT),
            TabKind::Registered(name) => self.kind_contexts.get(name).copied(),
        }
    }

    /// Remembers the key context of registered `kind` (the context stack has no `App`).
    fn note_context(&mut self, kind: TabKind, cx: &App) {
        if let (TabKind::Registered(name), Some(context)) = (kind, context_of(kind, cx)) {
            self.kind_contexts.insert(name, context);
        }
    }

    // ---- capsule -----------------------------------------------------------------------------

    /// What the title-bar capsule shows, in the current language: the demo connection, else
    /// the `capsule` hook's model, else the M1 placeholder.
    pub fn capsule_model(&self, cx: &App) -> CapsuleModel {
        match &self.capsule {
            None => hooks(cx)
                .capsule
                .and_then(|model| model(cx))
                .unwrap_or_else(|| CapsuleModel::Placeholder {
                    label: tr(cx, ids::hv::CAPSULE_NO_CONNECTION, NoArgs),
                }),
            Some(demo) => CapsuleModel::Connection {
                env: EnvKind::Production,
                env_label: tr(cx, ids::hv::CAPSULE_ENV_PRODUCTION, NoArgs),
                name: demo.name.clone(),
                engine: demo.engine.clone(),
                access: tr(cx, ids::hv::CAPSULE_ACCESS_READ_ONLY, NoArgs),
                read_only: true,
                path: tr(cx, ids::hv::CAPSULE_PATH_DIRECT, NoArgs),
                direct: true,
                latency: None,
                unencrypted: None,
                unsupported: None,
            },
        }
    }
}

/// Emitted by an [`AnchoredHost`] when a press lands outside its view.
struct AnchorDismissed;

/// Places a popover view at its anchor and reports a press outside it (`open_anchored`).
struct AnchoredHost {
    anchor: Point<Pixels>,
    view: AnyView,
}

impl EventEmitter<AnchorDismissed> for AnchoredHost {}

impl Render for AnchoredHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            anchored().position(self.anchor).snap_to_window().child(
                div()
                    .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(AnchorDismissed)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(self.view.clone()),
            ),
        )
    }
}

/// Writes `workspace.json`; a failure keeps the previous file and the next change tries again.
fn write_state((path, value): &(PathBuf, serde_json::Value)) {
    if let Err(err) = workspace_state::save(path, WORKSPACE_STATE_VERSION, value) {
        tracing::warn!(path = %path.display(), %err, "workspace: saving the layout failed");
    }
}

/// A restored workspace, consistent and within the window.
#[derive(Debug, Clone, PartialEq)]
struct Restored {
    tiles: TileLayout,
    panes: PaneTree,
    tabs: BTreeMap<TabId, TabKind>,
    next_tab: u32,
}

/// Checks a saved state and repairs what can be repaired:
///
/// - rejected (the caller sets the file aside): a pane tree whose leaves are not exactly its
///   panes, a tab listed twice or in two panes, a pane naming a tab the state does not
///   describe, or tab IDs that leave no room for another;
/// - repaired: tile sizes (finite, within `bounds`), split ratios, active indexes, a focused
///   pane that does not exist, and tabs of kinds nobody registered (closed).
fn validate(
    saved: SavedState,
    registered: impl Fn(&str) -> Option<&'static str>,
    bounds: TileBounds,
) -> Result<Restored, &'static str> {
    let SavedState {
        tiles,
        mut panes,
        tabs: saved_tabs,
    } = saved;
    let leaves = panes.leaves();
    let unique: BTreeSet<PaneId> = leaves.iter().copied().collect();
    if unique.len() != leaves.len() || unique != panes.panes.keys().copied().collect() {
        return Err("the pane tree does not match its panes");
    }
    let mut described: BTreeMap<TabId, Option<TabKind>> = BTreeMap::new();
    for tab in saved_tabs {
        let kind = match tab.kind {
            SavedKind::Console { n } => Some(TabKind::Console { n }),
            SavedKind::Registered { name } => registered(&name).map(TabKind::Registered),
        };
        if described.insert(tab.id, kind).is_some() {
            return Err("a tab is listed twice");
        }
    }
    let mut seen = BTreeSet::new();
    for pane in panes.panes.values() {
        for tab in &pane.tabs {
            if !seen.insert(*tab) {
                return Err("a tab is in two places");
            }
            if !described.contains_key(tab) {
                return Err("a pane names an unknown tab");
            }
        }
    }
    let next_tab = match seen.last() {
        Some(max) => max.0.checked_add(1).ok_or("the tab IDs are used up")?,
        None => 0,
    };
    // Tabs nobody can show any more (an unregistered kind) close; their siblings move up.
    let orphans: Vec<(PaneId, TabId)> = panes
        .panes
        .iter()
        .flat_map(|(pane, p)| p.tabs.iter().map(move |t| (*pane, *t)))
        .filter(|(_, t)| described.get(t).is_some_and(Option::is_none))
        .collect();
    for (pane, tab) in orphans {
        panes.close_tab(pane, tab);
    }
    for pane in panes.panes.values_mut() {
        pane.active = pane.active.min(pane.tabs.len().saturating_sub(1));
    }
    repair_ratios(&mut panes.root);
    if !panes.panes.contains_key(&panes.focused) {
        panes.focused = panes.leaves().first().copied().unwrap_or(PaneId(0));
    }
    let tabs: BTreeMap<TabId, TabKind> = described
        .into_iter()
        .filter_map(|(id, kind)| Some((id, kind?)))
        .filter(|(id, _)| panes.panes.values().any(|p| p.tabs.contains(id)))
        .collect();
    Ok(Restored {
        tiles: tiles.sanitized(bounds),
        panes,
        tabs,
        next_tab,
    })
}

/// Clamps every split ratio into `[MIN_RATIO, MAX_RATIO]`; a non-finite one becomes even.
fn repair_ratios(node: &mut PaneNode) {
    if let PaneNode::Split { ratio, a, b, .. } = node {
        *ratio = if ratio.is_finite() {
            ratio.clamp(MIN_RATIO, MAX_RATIO)
        } else {
            0.5
        };
        repair_ratios(a);
        repair_ratios(b);
    }
}

// ---- rendering -------------------------------------------------------------------------------

impl Workspace {
    /// Escape pops the topmost layer; with no layer open it goes to the `escape` hook. Runs
    /// without borrowing the workspace, so the hook may update it.
    fn on_escape(ws: &Entity<Workspace>, event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
        if event.keystroke.key != "escape" {
            return;
        }
        if !ws.read(cx).layers.is_empty() {
            ws.update(cx, |ws, cx| ws.pop_layer(window, cx));
            cx.stop_propagation();
            return;
        }
        if let Some(hook) = hooks(cx).escape
            && hook(window, cx)
        {
            cx.stop_propagation();
        }
    }

    fn on_drag_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(seam) = self.drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            self.layout_changed(cx);
            return;
        }
        let t = Theme::get(cx).tokens;
        // Under Linux client decorations the content sits inside the resize inset.
        let inset = match chrome_kind(window) {
            ChromeKind::LinuxClient => window.client_inset().map_or(0.0, f32::from),
            _ => 0.0,
        };
        let size = window.viewport_size();
        let window_px = (
            f32::from(size.width) - 2.0 * inset,
            f32::from(size.height) - 2.0 * inset,
        );
        let position = point(event.position.x - px(inset), event.position.y - px(inset));
        self.tiles.drag(
            seam,
            position,
            window_px,
            t.spacing.titlebar_height,
            t.spacing.statusbar_height,
        );
        cx.notify();
    }

    fn on_drag_end(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            self.layout_changed(cx);
        }
    }

    fn title_bar(&self, theme: &Theme, window: &Window, cx: &App) -> TitleBar {
        let chrome = chrome_kind(window);
        let keys: SharedString = shortcut_label(cx, PALETTE_ACTION)
            .unwrap_or_default()
            .into();
        let mut capsule = Capsule::new(self.capsule_model(cx));
        if let Some(on_segment) = hooks(cx).capsule_segment {
            capsule = capsule.on_segment(move |segment, anchor, window, cx| {
                on_segment(segment, anchor, window, cx)
            });
        }
        let mut bar = TitleBar::new(chrome)
            .inert(self.layers.has_modal())
            .brand(tr(cx, ids::cm::BRAND_HACIVAT_CAPS, NoArgs))
            .capsule(capsule)
            .palette_field(
                tr(cx, ids::hv::TITLEBAR_PALETTE, NoArgs),
                keys,
                |_, window, cx| {
                    ActionRegistry::run(PALETTE_ACTION, window, cx);
                },
            )
            .right(vec![language_switch(cx), theme_switch(theme, cx)]);
        if matches!(chrome, ChromeKind::WindowsCaption | ChromeKind::LinuxClient) {
            bar = bar.app_menu(|_, window, cx| {
                ActionRegistry::run(APP_MENU_ACTION, window, cx);
            });
        }
        bar
    }

    fn status_bar(&self, theme: &Theme, cx: &App) -> StatusBar {
        let mut bar = StatusBar::new();
        match hooks(cx).status_left {
            Some(left) => {
                for segment in left(theme, cx) {
                    bar = bar.left(segment);
                }
            }
            None => {
                bar = bar.left(status_label(
                    theme,
                    tr(cx, ids::hv::STATUS_NO_CONNECTION, NoArgs),
                    TextSource::Fluent,
                ));
            }
        }
        if let Some(chord) = cx.try_global::<PendingChord>().and_then(|p| p.0.clone()) {
            let text = tr(
                cx,
                ids::hv::STATUS_PENDING_CHORD,
                ids::hv::HvStatusPendingChord {
                    chord: Text(chord.to_string()),
                },
            );
            bar = bar.left(status_label(theme, text, TextSource::Fluent));
        }
        bar.right(status_language(theme, cx))
    }

    fn tiles_row(
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &theme.tokens.colors;
        let l = self.tiles;
        let explorer = match &self.explorer_view {
            Some(view) => view.clone().into_any_element(),
            None => tiles::explorer(theme, cx).into_any_element(),
        };
        let connection_panel = match &self.connection_panel_view {
            Some(view) => view.clone().into_any_element(),
            None => tiles::connection_panel(theme, cx).into_any_element(),
        };
        let left = l.explorer_visible.then(|| {
            div()
                .flex()
                .flex_col()
                .w(px(l.explorer_w))
                .h_full()
                .flex_none()
                .gap(px(1.0))
                .bg(c.seam.to_hsla())
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .overflow_hidden()
                        .child(explorer),
                )
                .child(
                    div()
                        .h(px(l.conn_panel_h))
                        .flex_none()
                        .overflow_hidden()
                        .child(connection_panel),
                )
        });
        let root = self.panes.root.clone();
        let editor = self.pane_node(&root, theme, window, cx);
        let centre = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .gap(px(1.0))
            .bg(c.seam.to_hsla())
            .child(
                div()
                    .h(px(l.editor_h))
                    .flex_none()
                    .overflow_hidden()
                    .child(editor),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(tiles::results(theme, cx)),
            );
        let grabs: Vec<Div> = tiles::seam_grabs(&l)
            .into_iter()
            .map(|(seam, grab)| {
                grab.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.drag = Some(seam);
                        cx.stop_propagation();
                    }),
                )
            })
            .collect();
        div()
            .relative()
            .flex()
            .flex_row()
            .w_full()
            .flex_1()
            .min_h(px(0.0))
            .gap(px(1.0))
            .bg(c.seam.to_hsla())
            .children(left)
            .child(centre)
            .children(grabs)
            .into_any_element()
    }

    fn pane_node(
        &mut self,
        node: &PaneNode,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            PaneNode::Leaf(pane) => self.pane(*pane, theme, window, cx),
            PaneNode::Split { axis, ratio, a, b } => {
                let first = self.pane_node(a, theme, window, cx);
                let second = self.pane_node(b, theme, window, cx);
                let (container, sized) = match axis {
                    Axis::Horizontal => {
                        (div().flex().flex_row(), div().h_full().w(relative(*ratio)))
                    }
                    Axis::Vertical => (div().flex().flex_col(), div().w_full().h(relative(*ratio))),
                };
                container
                    .size_full()
                    .gap(px(1.0))
                    .bg(theme.tokens.colors.seam.to_hsla())
                    .child(sized.flex_none().overflow_hidden().child(first))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .overflow_hidden()
                            .child(second),
                    )
                    .into_any_element()
            }
        }
    }

    fn pane(
        &mut self,
        id: PaneId,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = theme.tokens;
        let c = &t.colors;
        let Some(pane) = self.panes.panes.get(&id).cloned() else {
            return div().into_any_element();
        };
        let specs: Vec<TabSpec> = pane
            .tabs
            .iter()
            .map(|tab| TabSpec {
                id: *tab,
                title: self.tab_title(*tab, cx),
                title_source: TextSource::Fluent,
                dirty: false,
                env: None,
            })
            .collect();
        let weak = cx.entity().downgrade();
        let on_close = weak.clone();
        let strip = TabStrip::new(id, specs)
            .active(pane.active)
            .on_activate(move |pane, tab, _, cx| {
                let _ = weak.update(cx, |w, cx| w.activate_tab(pane, tab, cx));
            })
            .on_close(move |pane, tab, _, cx| {
                let _ = on_close.update(cx, |w, cx| w.close_tab(pane, tab, cx));
            });
        let content = match pane.tabs.get(pane.active) {
            Some(tab) => self.tab_content(*tab, theme, window, cx),
            None => editor_empty(theme, cx),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(c.panel.to_hsla())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this.panes.focused != id {
                        this.panes.focused = id;
                        this.layout_changed(cx);
                    }
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .w_full()
                    .h(px(t.spacing.panel_header_height))
                    .flex_none()
                    .bg(c.head.to_hsla())
                    .child(strip),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(content),
            )
            .into_any_element()
    }

    fn tab_content(
        &mut self,
        tab: TabId,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.tabs.get(&tab) {
            Some(TabKind::Registered(_)) => match self.build_view(tab, window, cx) {
                Some(view) => div().size_full().child(view).into_any_element(),
                None => editor_empty(theme, cx),
            },
            _ => editor_empty(theme, cx),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        // The hook may run actions that update this view, so it runs without borrowing it.
        let weak = cx.entity().downgrade();
        let weak_escape = weak.clone();
        let capture = move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            let Some(hook) = key_capture(cx) else {
                return;
            };
            let Some(stack) = weak
                .upgrade()
                .map(|ws| ws.read(cx).context_stack(window, cx))
            else {
                return;
            };
            if hook(event, &stack, window, cx) {
                cx.stop_propagation();
            }
        };
        let escape = move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            if let Some(ws) = weak_escape.upgrade() {
                Self::on_escape(&ws, event, window, cx);
            }
        };
        let title_bar = self.title_bar(&theme, window, cx);
        let tiles = self.tiles_row(&theme, window, cx);
        let status_bar = self.status_bar(&theme, cx);
        let root = div()
            .id("workspace")
            .track_focus(&self.focus)
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(c.ground.to_hsla())
            .text_color(c.text.to_hsla())
            .capture_key_down(capture)
            .on_key_down(escape)
            .on_mouse_move(cx.listener(Self::on_drag_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_drag_end))
            .child(title_bar)
            .child(tiles)
            .child(status_bar)
            .children(self.layers.elements(&theme))
            .child(ToastStack::new().above(t.spacing.statusbar_height));
        client_decorations(root.into_any_element(), window, &theme)
    }
}

/// The editor area of a console with no connection: the `editor_empty` hook's element, else
/// the M1 empty state.
fn editor_empty(theme: &Theme, cx: &App) -> AnyElement {
    if let Some(element) = hooks(cx).editor_empty {
        return element(theme, cx);
    }
    tiles::empty_state(theme, tr(cx, ids::hv::EDITOR_EMPTY, NoArgs), "editor-empty")
        .into_any_element()
}

/// One language the switches offer.
struct LanguageChoice {
    tag: String,
    short_name: SharedString,
    native_name: SharedString,
    active: bool,
}

/// The stable and beta catalogs, the UI language's one marked active.
fn language_choices(cx: &App) -> Vec<LanguageChoice> {
    let Some(global) = cx.try_global::<L10nGlobal>() else {
        return Vec::new();
    };
    let ui = ui_language(cx);
    global
        .catalogs
        .locales()
        .iter()
        .filter(|l| l.status != Status::Experimental)
        .map(|l| LanguageChoice {
            tag: l.tag.to_string(),
            short_name: l.short_name.clone().into(),
            native_name: l.native_name.clone().into(),
            active: ui.as_ref().is_some_and(|u| u.language == l.tag.language),
        })
        .collect()
}

/// Switches the UI language to `tag` and saves it as `ui.language`.
pub(crate) fn set_language(cx: &mut App, tag: &str) {
    let settings = update_settings(cx, |s| s.ui.language = LanguagePref::Tag(tag.to_string()));
    switch_locale(cx, &settings).detach();
}

/// Applies `pref` and saves it as `ui.theme`.
pub(crate) fn set_theme(cx: &mut App, pref: ThemePref) {
    update_settings(cx, |s| s.ui.theme = pref);
    apply_theme(cx, pref);
}

/// The title bar's language control: a `Segmented` with one segment per catalog `short_name`
/// (11.5 px mono, DESIGN.md Segmented Controls). The names are catalog data, not messages, so the
/// segments are `Verbatim`; the tooltip and the accessible name of a segment is the language's
/// native name (the visible short name `TR` is announced as `Türkçe`). Focusable and keyboard-operable like every `Segmented`.
fn language_switch(cx: &App) -> AnyElement {
    let mut group = Segmented::new("language-switch").mono(true);
    for choice in language_choices(cx) {
        let tag = choice.tag;
        group = group.item(SegmentItem {
            content: SegmentContent::Verbatim(choice.short_name),
            pressed: choice.active,
            a11y_name: Some(choice.native_name.clone()),
            tooltip: choice.native_name,
            on_click: Rc::new(move |_, _, cx| set_language(cx, &tag)),
        });
    }
    group.into_any_element()
}

/// The title bar's theme control: moon (dark) and sun (light).
fn theme_switch(theme: &Theme, cx: &App) -> AnyElement {
    let segment = |icon, pressed, tooltip, pref: ThemePref| SegmentItem {
        content: SegmentContent::Icon(icon),
        pressed,
        tooltip,
        a11y_name: None,
        on_click: Rc::new(move |_, _, cx| set_theme(cx, pref)),
    };
    Segmented::new("theme-switch")
        .item(segment(
            IconName::Moon,
            theme.mode == ThemeMode::Dark,
            tr(cx, ids::hv::THEME_DARK, NoArgs),
            ThemePref::Dark,
        ))
        .item(segment(
            IconName::Sun,
            theme.mode == ThemeMode::Light,
            tr(cx, ids::hv::THEME_LIGHT, NoArgs),
            ThemePref::Light,
        ))
        .into_any_element()
}

/// The status bar's language toggle, `EN / TR` with the active language in bold; a click moves
/// to the next language.
fn status_language(theme: &Theme, cx: &App) -> AnyElement {
    let t = theme.tokens;
    let c = &t.colors;
    let choices = language_choices(cx);
    let next = choices
        .iter()
        .position(|l| l.active)
        .map_or(0, |i| (i + 1) % choices.len().max(1));
    let next_tag = choices.get(next).map(|l| l.tag.clone());
    let mut names = div().flex().flex_row().items_center();
    for (i, choice) in choices.into_iter().enumerate() {
        if i > 0 {
            names = names.child(
                div().mx(px(t.spacing.xs)).child(
                    Label::new(
                        "/",
                        TextSource::Data,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id(("status-language-separator", i)),
                ),
            );
        }
        let mut style = t.type_ramp.caption;
        if choice.active {
            style.weight = 600;
        }
        let ink = if choice.active { c.text } else { c.text_2 };
        names = names.child(
            Label::new(choice.short_name, TextSource::Data, style, ink.to_hsla())
                .id(("status-language", i)),
        );
    }
    div()
        .id("status-language")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.sm))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            if let Some(tag) = &next_tag {
                set_language(cx, tag);
            }
        })
        .child(
            Icon::new(IconName::Languages)
                .size(IconSize::Chrome)
                .color(c.text_2.to_hsla()),
        )
        .child(names)
        .into_any_element()
}

/// The invisible band around a Linux client-decorated window that resizes it.
const CSD_INSET_PX: f32 = 10.0;

/// Linux with client-side decorations (T8 carry-over): insets the window for its frame shadow,
/// draws the frame (a floating surface: radius, hairline, shadow) unless fully tiled, and adds
/// resize edges on the untiled sides. Elsewhere `root` is returned as is; the macOS title bar
/// drags through `start_window_move` in `TitleBar` itself.
fn client_decorations(root: AnyElement, window: &mut Window, theme: &Theme) -> AnyElement {
    if chrome_kind(window) != ChromeKind::LinuxClient {
        return root;
    }
    let Decorations::Client { tiling } = window.window_decorations() else {
        return root;
    };
    let inset = px(CSD_INSET_PX);
    window.set_client_inset(inset);
    let pad = |tiled: bool| if tiled { px(0.0) } else { inset };
    let all_tiled = tiling.top && tiling.bottom && tiling.left && tiling.right;
    let frame = if all_tiled {
        div().size_full().child(root)
    } else {
        popover_surface(theme)
            .size_full()
            .overflow_hidden()
            .child(root)
    };
    let edge = |edge: ResizeEdge| {
        div()
            .absolute()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                window.start_window_resize(edge);
            })
    };
    let mut edges: Vec<Div> = Vec::new();
    if !tiling.top {
        edges.push(
            edge(ResizeEdge::Top)
                .top_0()
                .left(inset)
                .right(inset)
                .h(inset)
                .cursor_ns_resize(),
        );
    }
    if !tiling.bottom {
        edges.push(
            edge(ResizeEdge::Bottom)
                .bottom_0()
                .left(inset)
                .right(inset)
                .h(inset)
                .cursor_ns_resize(),
        );
    }
    if !tiling.left {
        edges.push(
            edge(ResizeEdge::Left)
                .left_0()
                .top(inset)
                .bottom(inset)
                .w(inset)
                .cursor_ew_resize(),
        );
    }
    if !tiling.right {
        edges.push(
            edge(ResizeEdge::Right)
                .right_0()
                .top(inset)
                .bottom(inset)
                .w(inset)
                .cursor_ew_resize(),
        );
    }
    if !tiling.top && !tiling.left {
        edges.push(
            edge(ResizeEdge::TopLeft)
                .top_0()
                .left_0()
                .size(inset)
                .cursor_nwse_resize(),
        );
    }
    if !tiling.top && !tiling.right {
        edges.push(
            edge(ResizeEdge::TopRight)
                .top_0()
                .right_0()
                .size(inset)
                .cursor_nesw_resize(),
        );
    }
    if !tiling.bottom && !tiling.left {
        edges.push(
            edge(ResizeEdge::BottomLeft)
                .bottom_0()
                .left_0()
                .size(inset)
                .cursor_nesw_resize(),
        );
    }
    if !tiling.bottom && !tiling.right {
        edges.push(
            edge(ResizeEdge::BottomRight)
                .bottom_0()
                .right_0()
                .size(inset)
                .cursor_nwse_resize(),
        );
    }
    div()
        .relative()
        .size_full()
        .pt(pad(tiling.top))
        .pb(pad(tiling.bottom))
        .pl(pad(tiling.left))
        .pr(pad(tiling.right))
        .child(frame)
        .children(edges)
        .into_any_element()
}

#[cfg(test)]
mod validate_tests {
    use super::*;
    use hacivat_ui::panes::Pane;

    fn bounds() -> TileBounds {
        TileBounds {
            window: (1440.0, 900.0),
            top: 40.0,
            bottom: 24.0,
        }
    }

    fn console(id: u32, n: u32) -> SavedTab {
        SavedTab {
            id: TabId(id),
            kind: SavedKind::Console { n },
        }
    }

    fn two_panes() -> SavedState {
        let mut panes = PaneTree::new(TabId(0));
        panes.split(PaneId(0), Axis::Horizontal, TabId(1));
        SavedState {
            tiles: TileLayout::default(),
            panes,
            tabs: vec![console(0, 1), console(1, 2)],
        }
    }

    fn none(_: &str) -> Option<&'static str> {
        None
    }

    #[test]
    fn a_valid_state_is_kept() {
        let saved = two_panes();
        let r = validate(saved.clone(), none, bounds()).unwrap();
        assert_eq!(
            (r.panes, r.tiles, r.next_tab),
            (saved.panes, saved.tiles, 2)
        );
        assert_eq!(r.tabs.len(), 2);
    }

    #[test]
    fn tile_sizes_are_made_finite_and_fit_the_window() {
        let mut saved = two_panes();
        saved.tiles = TileLayout {
            explorer_w: f32::NAN,
            conn_panel_h: 1.0e9,
            editor_h: -5.0,
            inspector_w: f32::INFINITY,
            explorer_visible: true,
        };
        let t = validate(saved, none, bounds()).unwrap().tiles;
        assert_eq!(t.explorer_w, 276.0);
        assert_eq!(t.conn_panel_h, 900.0 - 40.0 - 24.0 - tiles::MIN_TILE_PX);
        assert_eq!(t.editor_h, tiles::MIN_TILE_PX);
        assert_eq!(t.inspector_w, 252.0);
    }

    #[test]
    fn a_missing_focused_pane_and_bad_indexes_are_repaired() {
        let mut saved = two_panes();
        saved.panes.focused = PaneId(9);
        saved.panes.panes.get_mut(&PaneId(0)).unwrap().active = 7;
        saved.panes.set_ratio(&[], 0.5);
        if let PaneNode::Split { ratio, .. } = &mut saved.panes.root {
            *ratio = f32::NAN;
        }
        let r = validate(saved, none, bounds()).unwrap();
        assert_eq!(r.panes.focused, PaneId(0));
        assert_eq!(r.panes.panes[&PaneId(0)].active, 0);
        assert!(matches!(r.panes.root, PaneNode::Split { ratio, .. } if ratio == 0.5));
    }

    #[test]
    fn inconsistent_states_are_rejected() {
        let mut extra_pane = two_panes();
        extra_pane.panes.panes.insert(
            PaneId(5),
            Pane {
                tabs: vec![],
                active: 0,
            },
        );
        assert!(validate(extra_pane, none, bounds()).is_err());

        let mut twice = two_panes();
        twice.tabs.push(console(1, 3));
        assert!(validate(twice, none, bounds()).is_err());

        let mut unknown = two_panes();
        unknown.tabs.pop();
        assert!(validate(unknown, none, bounds()).is_err());

        let mut exhausted = SavedState {
            tiles: TileLayout::default(),
            panes: PaneTree::new(TabId(u32::MAX)),
            tabs: vec![console(u32::MAX, 1)],
        };
        assert_eq!(
            validate(exhausted.clone(), none, bounds()),
            Err("the tab IDs are used up")
        );
        exhausted.panes = PaneTree::new(TabId(u32::MAX - 1));
        exhausted.tabs = vec![console(u32::MAX - 1, 1)];
        assert_eq!(
            validate(exhausted, none, bounds()).unwrap().next_tab,
            u32::MAX
        );
    }

    #[test]
    fn tabs_of_unregistered_kinds_are_closed() {
        let mut saved = two_panes();
        saved.tabs[1] = SavedTab {
            id: TabId(1),
            kind: SavedKind::Registered {
                name: "gone".to_string(),
            },
        };
        let r = validate(saved, none, bounds()).unwrap();
        assert_eq!(r.panes.panes.len(), 1);
        assert_eq!(r.tabs.keys().copied().collect::<Vec<_>>(), [TabId(0)]);
    }
}
