//! The workspace view: title bar, tiles, status bar, layers and toasts, laid out like harman
//! (DESIGN.md Layout). It owns the tile layout, the pane tree with its tabs, the capsule and the
//! layer stack, and saves the layout to `workspace.json` (debounced, on the background
//! executor) whenever it changes.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use hacivat_l10n::{NoArgs, Status, Text, ids};
use hacivat_settings::{LanguagePref, ThemePref, workspace_state};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::capsule::{Capsule, CapsuleModel, EnvKind};
use hacivat_ui::dialog::{Dialog, DialogEvent, DialogSpec};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};
use hacivat_ui::panes::{Axis, PaneId, PaneNode, PaneTree, TabId, TabSpec, TabStrip};
use hacivat_ui::popover::Declare as _;
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented};
use hacivat_ui::statusbar::{StatusBar, status_label};
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui::titlebar::TitleBar;
use hacivat_ui::toast::ToastStack;
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, Context, Decorations, Div, Entity, EntityId,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, ResizeEdge, Role,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, WindowHandle,
    div, px, relative,
};
use hacivat_ui_core::keymap::ContextStack;
use hacivat_ui_core::theme::{Theme, ThemeMode, popover_surface};
use hacivat_ui_core::window::{
    ChromeKind, WorkspaceWindowSpec, chrome_kind, open_workspace_window,
};
use serde::{Deserialize, Serialize};

use crate::PendingChord;
use crate::l10n_global::{L10nGlobal, WINDOW_TITLE, switch_locale, tr, ui_language};
use crate::shell::hooks::key_capture;
use crate::shell::keymap_state::shortcut_label;
use crate::shell::layers::{DIALOG_CONTEXT, Layer, Layers, MENU_CONTEXT};
use crate::shell::tab_content::{TabContentRegistry, TabKind};
use crate::shell::tabs::{CONSOLE_CONTEXT, context_of, title_of};
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
    layers: Layers,
    focus: FocusHandle,
    drag: Option<Seam>,
    state_path: Option<PathBuf>,
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
        ];
        let mut ws = Self {
            tiles: TileLayout::default(),
            panes: PaneTree::new(TabId(0)),
            tabs: BTreeMap::from([(TabId(0), TabKind::Console { n: 1 })]),
            tab_views: HashMap::new(),
            kind_contexts: HashMap::new(),
            next_tab: 1,
            capsule: demo,
            layers: Layers::default(),
            focus: cx.focus_handle(),
            drag: None,
            state_path,
            save_task: None,
            _subscriptions: subscriptions,
        };
        ws.restore(cx);
        ws
    }

    // ---- persistence -------------------------------------------------------------------------

    /// Restores `workspace.json` when it is readable; otherwise keeps one `Console 1`.
    fn restore(&mut self, cx: &App) {
        let Some(path) = &self.state_path else {
            return;
        };
        let Some(value) = workspace_state::load(path, WORKSPACE_STATE_VERSION) else {
            return;
        };
        let Ok(saved) = serde_json::from_value::<SavedState>(value) else {
            return;
        };
        let registry = cx.try_global::<TabContentRegistry>();
        let mut tabs = BTreeMap::new();
        for tab in saved.tabs {
            let kind = match tab.kind {
                SavedKind::Console { n } => Some(TabKind::Console { n }),
                SavedKind::Registered { name } => registry
                    .and_then(|r| r.kind_named(&name))
                    .map(TabKind::Registered),
            };
            if let Some(kind) = kind {
                tabs.insert(tab.id, kind);
            }
        }
        let mut panes = saved.panes;
        // A tab the tree names but nobody can show any more (an unregistered kind) is closed.
        let orphans: Vec<(PaneId, TabId)> = panes
            .panes
            .iter()
            .flat_map(|(pane, p)| p.tabs.iter().map(move |t| (*pane, *t)))
            .filter(|(_, t)| !tabs.contains_key(t))
            .collect();
        for (pane, tab) in orphans {
            panes.close_tab(pane, tab);
        }
        tabs.retain(|id, _| panes.panes.values().any(|p| p.tabs.contains(id)));
        self.next_tab = tabs.keys().next_back().map_or(0, |t| t.0 + 1);
        for kind in tabs.values() {
            self.note_context(*kind, cx);
        }
        self.tiles = saved.tiles;
        self.panes = panes;
        self.tabs = tabs;
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
        let executor = cx.background_executor().clone();
        self.save_task = Some(cx.background_spawn(async move {
            executor.timer(SAVE_DEBOUNCE).await;
            // A failed save keeps the previous file; the next change tries again.
            let _ = workspace_state::save(&path, WORKSPACE_STATE_VERSION, &value);
        }));
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

    /// The title of `tab` in the current language.
    pub fn tab_title(&self, tab: TabId, cx: &App) -> SharedString {
        self.tabs
            .get(&tab)
            .map(|kind| title_of(*kind, cx))
            .unwrap_or_default()
    }

    fn fresh_tab(&mut self, kind: TabKind) -> TabId {
        let id = TabId(self.next_tab);
        self.next_tab += 1;
        self.tabs.insert(id, kind);
        id
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
            .map_or(1, |n| n + 1)
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
        let tab = self.fresh_tab(TabKind::Console { n });
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
        let tab = self.fresh_tab(TabKind::Console { n });
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
            let tab = self.fresh_tab(TabKind::Registered(kind));
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
            move |this, _, _: &DialogEvent, window, cx| {
                this.remove_layer(view, window, cx);
            },
        );
        let layer = Layer {
            id: "dialog",
            context: DIALOG_CONTEXT,
            view: dialog.clone().into(),
            restore_focus: restore,
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
            move |this, _, event: &PopupMenuEvent, window, cx| {
                this.remove_layer(view, window, cx);
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
        };
        self.push_layer_with(layer, Some(subscription), window, cx);
        menu
    }

    /// The active key contexts, innermost last: `Workspace`, the active tab's context, then each
    /// layer's context from the bottom up.
    pub fn context_stack(&self, _window: &Window) -> ContextStack {
        let mut stack = vec![WORKSPACE_CONTEXT];
        stack.extend(self.active_tab_kind().and_then(|k| self.tab_context(k)));
        stack.extend(self.layers.contexts());
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

    /// What the title-bar capsule shows, in the current language.
    pub fn capsule_model(&self, cx: &App) -> CapsuleModel {
        match &self.capsule {
            None => CapsuleModel::Placeholder {
                label: tr(cx, ids::hv::CAPSULE_NO_CONNECTION, NoArgs),
            },
            Some(demo) => CapsuleModel::Connection {
                env: EnvKind::Production,
                env_label: tr(cx, ids::hv::CAPSULE_ENV_PRODUCTION, NoArgs),
                name: demo.name.clone(),
                engine: demo.engine.clone(),
                access: tr(cx, ids::hv::CAPSULE_ACCESS_READ_ONLY, NoArgs),
                path: tr(cx, ids::hv::CAPSULE_PATH_DIRECT, NoArgs),
            },
        }
    }
}

// ---- rendering -------------------------------------------------------------------------------

impl Workspace {
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && !self.layers.is_empty() {
            self.pop_layer(window, cx);
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
        let size = window.viewport_size();
        let window_px = (f32::from(size.width), f32::from(size.height));
        self.tiles.drag(
            seam,
            event.position,
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
        let mut bar = TitleBar::new(chrome)
            .brand(tr(cx, ids::cm::BRAND_HACIVAT_CAPS, NoArgs))
            .capsule(Capsule::new(self.capsule_model(cx)))
            .palette_field(
                tr(cx, ids::hv::TITLEBAR_PALETTE, NoArgs),
                keys,
                |_, window, cx| {
                    ActionRegistry::run(PALETTE_ACTION, window, cx);
                },
            )
            .right(vec![language_switch(theme, cx), theme_switch(theme, cx)]);
        if matches!(chrome, ChromeKind::WindowsCaption | ChromeKind::LinuxClient) {
            bar = bar.app_menu(|_, window, cx| {
                ActionRegistry::run(APP_MENU_ACTION, window, cx);
            });
        }
        bar
    }

    fn status_bar(&self, theme: &Theme, cx: &App) -> StatusBar {
        let mut bar = StatusBar::new().left(status_label(
            theme,
            tr(cx, ids::hv::STATUS_NO_CONNECTION, NoArgs),
            TextSource::Fluent,
        ));
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
                        .child(tiles::explorer(theme, cx)),
                )
                .child(
                    div()
                        .h(px(l.conn_panel_h))
                        .flex_none()
                        .overflow_hidden()
                        .child(tiles::connection_panel(theme, cx)),
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
        let capture = move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            let Some(hook) = key_capture(cx) else {
                return;
            };
            let Some(stack) = weak.upgrade().map(|ws| ws.read(cx).context_stack(window)) else {
                return;
            };
            if hook(event, &stack, window, cx) {
                cx.stop_propagation();
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
            .on_key_down(cx.listener(Self::on_key_down))
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

/// The editor area of a console with no connection.
fn editor_empty(theme: &Theme, cx: &App) -> AnyElement {
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
fn set_language(cx: &mut App, tag: &str) {
    let settings = update_settings(cx, |s| s.ui.language = LanguagePref::Tag(tag.to_string()));
    switch_locale(cx, &settings).detach();
}

/// Applies `pref` and saves it as `ui.theme`.
fn set_theme(cx: &mut App, pref: ThemePref) {
    update_settings(cx, |s| s.ui.theme = pref);
    apply_theme(cx, pref);
}

/// The title bar's language control: one segment per catalog `short_name` (DESIGN.md Segmented
/// Controls; language segments in 11.5 px mono). The names are catalog data, not messages.
fn language_switch(theme: &Theme, cx: &App) -> AnyElement {
    let t = theme.tokens;
    let c = &t.colors;
    let pressed_fill = match theme.mode {
        ThemeMode::Dark => c.raised,
        ThemeMode::Light => c.panel,
    };
    let mut group = div()
        .debug_selector(|| "language-switch".to_string())
        .flex()
        .flex_row()
        .items_center()
        .flex_none()
        .h(px(t.spacing.field_height))
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .rounded(px(t.radii.sm))
        .overflow_hidden();
    for (i, choice) in language_choices(cx).into_iter().enumerate() {
        let mut style = t.type_ramp.mono_data;
        if choice.active {
            style.weight = 600;
        }
        let ink = if choice.active { c.text } else { c.text_2 };
        let tag = choice.tag.clone();
        let mut segment = div()
            .id(("language-segment", i))
            .declare(
                Role::Button,
                choice.native_name.clone(),
                Some(choice.active),
            )
            .flex()
            .flex_row()
            .items_center()
            .h_full()
            .px(px(t.spacing.md))
            .cursor_pointer()
            .on_click(move |_, _, cx| set_language(cx, &tag))
            .child(Label::new(
                choice.short_name,
                TextSource::Data,
                style,
                ink.to_hsla(),
            ));
        if i > 0 {
            segment = segment.border_l_1().border_color(c.seam_strong.to_hsla());
        }
        segment = if choice.active {
            segment.bg(pressed_fill.to_hsla())
        } else {
            segment.hover(|s| s.bg(c.hover.to_hsla()))
        };
        group = group.child(segment);
    }
    group.into_any_element()
}

/// The title bar's theme control: moon (dark) and sun (light).
fn theme_switch(theme: &Theme, cx: &App) -> AnyElement {
    let segment = |icon, pressed, tooltip, pref: ThemePref| SegmentItem {
        content: SegmentContent::Icon(icon),
        pressed,
        tooltip,
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
