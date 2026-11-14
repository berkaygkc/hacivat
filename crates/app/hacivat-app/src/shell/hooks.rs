//! `ShellHooks`: behaviour a later task plugs into the shell without editing it (F3).

use std::rc::Rc;

use hacivat_ui::capsule::{CapsuleModel, CapsuleSegment};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, FocusHandle, Global, KeyDownEvent, Pixels, Point, Window,
};
// ## m3a-t01b-seams
use hacivat_proto::ProfileId;
use hacivat_ui::panes::{EnvMark, TabId};
use hacivat_ui::text::TextSource;
use hacivat_ui_core::gpui_surface::SharedString;
use hacivat_ui_core::keymap::ContextStack;
use hacivat_ui_core::theme::Theme;

/// Sees every key down before any element of the workspace does, with the workspace's context
/// stack. Returning `true` consumes the key (propagation stops). Task 16's dispatcher fills it.
pub type KeyCaptureHook = Rc<dyn Fn(&KeyDownEvent, &ContextStack, &mut Window, &mut App) -> bool>;

/// Builds a view the workspace shows in place of an M1 tile (built once, in `Workspace::new`).
pub type ViewBuilder = Rc<dyn Fn(&mut Window, &mut App) -> AnyView>;
/// Draws an element in place of an M1 one.
pub type ElementFn = Rc<dyn Fn(&Theme, &App) -> AnyElement>;
/// What the title-bar capsule shows; `None` keeps the M1 capsule.
pub type CapsuleFn = Rc<dyn Fn(&App) -> Option<CapsuleModel>>;
/// A click on a capsule segment, with the segment's bottom-left corner (the `open_anchored`
/// anchor).
pub type SegmentFn = Rc<dyn Fn(CapsuleSegment, Point<Pixels>, &mut Window, &mut App)>;
/// The status bar's left segments, in place of the M1 "No connection" label.
pub type StatusFn = Rc<dyn Fn(&Theme, &App) -> Vec<AnyElement>>;
/// Escape with no layer open; returning `true` consumes it.
pub type EscapeFn = Rc<dyn Fn(&mut Window, &mut App) -> bool>;

// ## m3a-t01b-seams
/// Builds a console tab's view, once per tab (like a registered tab's): the tab, its console
/// number and its saved binding (P12). The view's root element takes the tab's content area
/// as the M2 home does, so an unbound console looks exactly like today's.
pub type ConsoleViewFn =
    Rc<dyn Fn(TabId, u32, Option<ProfileId>, &mut Window, &mut App) -> AnyView>;

/// How a tab is decorated: its title in place of the kind's (with its text source), its
/// environment mark, and the 1 px production hairline at the pane's top edge.
#[derive(Debug, Clone, PartialEq)]
pub struct TabDecor {
    pub title: Option<SharedString>,
    pub title_source: TextSource,
    pub env: Option<EnvMark>,
    pub hairline: bool,
}

impl Default for TabDecor {
    /// No decoration: the kind's title (Fluent), no mark, no hairline.
    fn default() -> Self {
        TabDecor {
            title: None,
            title_source: TextSource::Fluent,
            env: None,
            hairline: false,
        }
    }
}

/// A tab's decoration, read on every render.
pub type TabDecorFn = Rc<dyn Fn(TabId, &App) -> TabDecor>;

/// Whether a close (of a tab, or of the app) goes ahead now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloseDecision {
    Proceed,
    /// The guard asks first (the open-transaction prompt) and closes later itself.
    Defer,
}

/// Asked before a tab closes; `Defer`: the guard closes it later with
/// `Workspace::force_close_tab`.
pub type TabCloseGuardFn = Rc<dyn Fn(TabId, &mut Window, &mut App) -> CloseDecision>;

/// Asked before `app.quit`; `Defer`: the guard calls `cx.quit()` later.
pub type QuitGuardFn = Rc<dyn Fn(&mut App) -> CloseDecision>;

/// The profile a new console tab binds to (R-M3-37: the capsule's connected profile); `None`
/// opens it unbound.
pub type ConsoleBindingFn = Rc<dyn Fn(&App) -> Option<ProfileId>>;

/// The shell's hook slots. With a slot empty, the shell draws its M1 element.
///
/// Keys reach commands through the key-capture hook (Task 16's dispatcher) and the keymap data,
/// never through GPUI key bindings: GPUI `KeyBinding`s are owned by `crate::menus::rebuild`,
/// which clears them on every rebuild, so widgets must not use `bind_keys`.
///
/// The view hooks (`explorer`, `connection_panel`) are read once, when `Workspace::new` builds
/// them, so they are set before the workspace opens (`connections::boot`, P17).
#[derive(Clone, Default)]
pub struct ShellHooks {
    pub key_capture: Option<KeyCaptureHook>,
    // M2a connections (Task 1).
    pub explorer: Option<ViewBuilder>,
    pub connection_panel: Option<ViewBuilder>,
    pub editor_empty: Option<ElementFn>,
    pub capsule: Option<CapsuleFn>,
    pub capsule_segment: Option<SegmentFn>,
    pub status_left: Option<StatusFn>,
    pub escape: Option<EscapeFn>,
    // ## m3a-t01b-seams (empty slots keep today's rendering)
    /// A console tab's view (Task 32's host; Task 1b's stub shows the M2 home or a registered
    /// test console's editor).
    pub console_view: Option<ConsoleViewFn>,
    /// The results tile (read once, in `Workspace::new`; Task 30).
    pub results_view: Option<ViewBuilder>,
    pub tab_decor: Option<TabDecorFn>,
    pub tab_close_guard: Option<TabCloseGuardFn>,
    pub quit_guard: Option<QuitGuardFn>,
    pub console_binding: Option<ConsoleBindingFn>,
}

impl Global for ShellHooks {}

/// Fills the key-capture slot (replacing any earlier hook).
pub fn set_key_capture(cx: &mut App, hook: KeyCaptureHook) {
    cx.default_global::<ShellHooks>().key_capture = Some(hook);
}

/// Fills the explorer tile's slot.
pub fn set_explorer(cx: &mut App, hook: ViewBuilder) {
    cx.default_global::<ShellHooks>().explorer = Some(hook);
}

/// Fills the connection panel's slot.
pub fn set_connection_panel(cx: &mut App, hook: ViewBuilder) {
    cx.default_global::<ShellHooks>().connection_panel = Some(hook);
}

/// Fills the slot of the editor area's empty state.
pub fn set_editor_empty(cx: &mut App, hook: ElementFn) {
    cx.default_global::<ShellHooks>().editor_empty = Some(hook);
}

/// Fills the capsule model's slot.
pub fn set_capsule(cx: &mut App, hook: CapsuleFn) {
    cx.default_global::<ShellHooks>().capsule = Some(hook);
}

/// Fills the capsule segment click's slot.
pub fn set_capsule_segment(cx: &mut App, hook: SegmentFn) {
    cx.default_global::<ShellHooks>().capsule_segment = Some(hook);
}

/// Fills the status bar's left slot.
pub fn set_status_left(cx: &mut App, hook: StatusFn) {
    cx.default_global::<ShellHooks>().status_left = Some(hook);
}

/// Fills the Escape slot.
pub fn set_escape(cx: &mut App, hook: EscapeFn) {
    cx.default_global::<ShellHooks>().escape = Some(hook);
}

// ## m3a-t01b-seams

/// Fills the console view's slot.
pub fn set_console_view(cx: &mut App, hook: ConsoleViewFn) {
    cx.default_global::<ShellHooks>().console_view = Some(hook);
}

/// Fills the results tile's slot (before the workspace opens).
pub fn set_results_view(cx: &mut App, hook: ViewBuilder) {
    cx.default_global::<ShellHooks>().results_view = Some(hook);
}

/// Fills the tab decoration's slot.
pub fn set_tab_decor(cx: &mut App, hook: TabDecorFn) {
    cx.default_global::<ShellHooks>().tab_decor = Some(hook);
}

/// Fills the tab close guard's slot.
pub fn set_tab_close_guard(cx: &mut App, hook: TabCloseGuardFn) {
    cx.default_global::<ShellHooks>().tab_close_guard = Some(hook);
}

/// Fills the quit guard's slot.
pub fn set_quit_guard(cx: &mut App, hook: QuitGuardFn) {
    cx.default_global::<ShellHooks>().quit_guard = Some(hook);
}

/// Fills the new console's binding slot.
pub fn set_console_binding(cx: &mut App, hook: ConsoleBindingFn) {
    cx.default_global::<ShellHooks>().console_binding = Some(hook);
}

/// The quit guard's decision (`Proceed` without a guard).
pub(crate) fn quit_decision(cx: &mut App) -> CloseDecision {
    match hooks(cx).quit_guard {
        Some(guard) => guard(cx),
        None => CloseDecision::Proceed,
    }
}

/// The tab close guard's decision for `tab` (`Proceed` without a guard). Called outside any
/// workspace update, so the guard may open a layer on the workspace.
pub(crate) fn tab_close_decision(tab: TabId, window: &mut Window, cx: &mut App) -> CloseDecision {
    match hooks(cx).tab_close_guard {
        Some(guard) => guard(tab, window, cx),
        None => CloseDecision::Proceed,
    }
}

/// The installed hooks (a clone: the slots are `Rc`s), or none.
pub(crate) fn hooks(cx: &App) -> ShellHooks {
    cx.try_global::<ShellHooks>().cloned().unwrap_or_default()
}

/// The installed key-capture hook, if any.
pub(crate) fn key_capture(cx: &App) -> Option<KeyCaptureHook> {
    cx.try_global::<ShellHooks>()
        .and_then(|h| h.key_capture.clone())
}

/// Views that add a key context while focused (the explorer list, P4).
#[derive(Default)]
pub struct FocusContexts(Vec<(FocusHandle, &'static str)>);

impl Global for FocusContexts {}

/// Pushes `context` onto `Workspace::context_stack` while `handle` is focused. A view built
/// inside `Workspace::new` calls it with the `App` it gets (P4). Registering a handle again
/// replaces its context.
pub fn register_focus_context(cx: &mut App, handle: FocusHandle, context: &'static str) {
    let contexts = &mut cx.default_global::<FocusContexts>().0;
    match contexts.iter_mut().find(|(h, _)| *h == handle) {
        Some(entry) => entry.1 = context,
        None => contexts.push((handle, context)),
    }
}

/// The registered contexts whose handle is focused in `window`, in registration order.
pub(crate) fn focused_contexts(window: &Window, cx: &App) -> Vec<&'static str> {
    cx.try_global::<FocusContexts>()
        .map(|f| {
            f.0.iter()
                .filter(|(h, _)| h.is_focused(window))
                .map(|(_, c)| *c)
                .collect()
        })
        .unwrap_or_default()
}
