//! Commands: registry wiring, the key dispatcher, the palette and debug hooks. Task 16.

#[cfg(debug_assertions)]
pub mod debug;
pub mod dispatch;
pub mod palette;
pub mod registry;

pub use dispatch::{DispatchOutcome, Dispatcher, PENDING_TIMEOUT};
pub use registry::register_workspace_actions;
// ## m3a-t01b-seams
#[cfg(feature = "test-support")]
pub use registry::quit_requests;
pub use registry::request_quit;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::input::FocusedInput;
use hacivat_ui_core::gpui_surface::{
    AnyWindowHandle, App, Global, KeyDownEvent, RunCommand, SharedString, Window, WindowHandle,
};
use hacivat_ui_core::keymap::{ContextStack, KeyboardLayoutId, Keymap};

use crate::PendingChord;
use crate::menus;
use crate::shell::{ActiveKeymap, Workspace, set_key_capture};

/// The app's dispatcher, shared by the key-capture hook, the keymap observer and the pending
/// chord's expiry timer.
#[derive(Clone)]
struct DispatcherState(Rc<RefCell<Dispatcher>>);

impl Global for DispatcherState {}

/// Registers the commands, installs the dispatcher in the key-capture hook slot, follows the
/// `ActiveKeymap` and `FocusedInput` globals, routes `RunCommand` (menus) to the registry and
/// builds the menus.
pub fn install(cx: &mut App) {
    register_workspace_actions(cx.default_global::<ActionRegistry>());

    let (keymap, layout) = match cx.try_global::<ActiveKeymap>() {
        Some(a) => (a.keymap.clone(), a.layout.clone()),
        None => (empty_keymap(), KeyboardLayoutId("us".to_string())),
    };
    let state = DispatcherState(Rc::new(RefCell::new(Dispatcher::new(keymap, layout))));
    cx.set_global(state.clone());
    set_key_capture(cx, Rc::new(on_key_down));

    // F11: a preset switch or a keyboard layout change replaces the keymap.
    cx.observe_global::<ActiveKeymap>(|cx| {
        let active = cx.global::<ActiveKeymap>();
        let (keymap, layout) = (active.keymap.clone(), active.layout.clone());
        if let Some(state) = cx.try_global::<DispatcherState>().cloned() {
            state.0.borrow_mut().set_keymap(keymap, layout);
        }
        set_pending_chord(cx, None);
        menus::rebuild(cx);
    })
    .detach();
    // F15: a composition that starts ends any chord in progress.
    cx.observe_global::<FocusedInput>(|cx| {
        if !cx.global::<FocusedInput>().composing {
            return;
        }
        if let Some(state) = cx.try_global::<DispatcherState>().cloned() {
            state.0.borrow_mut().clear_pending();
        }
        set_pending_chord(cx, None);
    })
    .detach();

    // F1: menu items (and their macOS key equivalents) carry `RunCommand`.
    cx.on_action(|action: &RunCommand, cx: &mut App| {
        let id = action.id.clone();
        cx.defer(move |cx| run_in_workspace_window(&id, cx));
    });

    menus::rebuild(cx);

    #[cfg(debug_assertions)]
    debug::install_from_env(cx);
}

fn empty_keymap() -> Arc<Keymap> {
    let platform = crate::shell::keymap_state::current_platform();
    Arc::new(Keymap::from_sources(platform, "", "", &[], None).0)
}

/// Runs `id` in the active workspace window (or the first one). A key equivalent pressed during
/// an IME composition reaches the menu when the window lets the key through: like every
/// binding, it does not run then (F15). Under a modal layer only `app.quit` runs (P47), as
/// with the keys: the macOS menu greys the other items, and this guard covers a menu built
/// before the layer opened.
fn run_in_workspace_window(id: &str, cx: &mut App) {
    if cx.try_global::<FocusedInput>().is_some_and(|f| f.composing) {
        return;
    }
    if id != QUIT_ACTION && modal_open(cx) {
        return;
    }
    // Through the window alone: the command may update the workspace itself.
    if let Some(window) = workspace_window(cx).map(AnyWindowHandle::from) {
        let _ = window.update(cx, |_, window, cx| ActionRegistry::run(id, window, cx));
    }
}

/// The workspace window the menus act on: the active window, or else the first workspace one.
fn workspace_window(cx: &App) -> Option<WindowHandle<Workspace>> {
    cx.active_window()
        .into_iter()
        .chain(cx.windows())
        .find_map(|w| w.downcast::<Workspace>())
}

/// Whether a modal layer is open in the workspace window the menus act on; while one is, the
/// menus run only `app.quit` (P47). False while that window is being updated.
pub(crate) fn modal_open(cx: &App) -> bool {
    workspace_window(cx)
        .and_then(|w| w.read(cx).ok())
        .is_some_and(|ws| ws.modal_context().is_some())
}

/// Sets the `PendingChord` global when it changes (the status bar observes it).
fn set_pending_chord(cx: &mut App, chord: Option<SharedString>) {
    if cx.try_global::<PendingChord>().map(|p| &p.0) != Some(&chord) {
        cx.set_global(PendingChord(chord));
    }
}

/// The key-capture hook: returns `true` when the key ran a command or started a chord.
fn on_key_down(ev: &KeyDownEvent, ctx: &ContextStack, window: &mut Window, cx: &mut App) -> bool {
    let Some(state) = cx.try_global::<DispatcherState>().cloned() else {
        return false;
    };
    // A plain Escape belongs to the topmost layer (it closes it), never to a binding.
    let m = &ev.keystroke.modifiers;
    let plain = !(m.control || m.alt || m.shift || m.platform || m.function);
    let layers_open = window
        .root::<Workspace>()
        .flatten()
        .is_some_and(|ws| !ws.read(cx).layer_ids().is_empty());
    if ev.keystroke.key == "escape" && plain && layers_open {
        state.0.borrow_mut().clear_pending();
        set_pending_chord(cx, None);
        return false;
    }
    let composing = cx.try_global::<FocusedInput>().is_some_and(|f| f.composing);
    let now = cx.background_executor().now();
    let modal = window
        .root::<Workspace>()
        .flatten()
        .and_then(|ws| ws.read(cx).modal_context());
    let outcome = match modal {
        None => state.0.borrow_mut().on_key_down(ev, ctx, composing, now),
        Some(top) => modal_outcome(&state, ev, ctx, top, composing, now),
    };
    match outcome {
        DispatchOutcome::Run(id) => {
            set_pending_chord(cx, None);
            ActionRegistry::run(&id, window, cx)
        }
        DispatchOutcome::Pending(label) => {
            set_pending_chord(cx, Some(label));
            expire_later(cx, now);
            true
        }
        DispatchOutcome::PassThrough => {
            set_pending_chord(cx, None);
            false
        }
    }
}

/// The action that quits the app; its binding and menu item stay live under a modal layer
/// (P47).
pub(crate) const QUIT_ACTION: &str = "app.quit";

/// Resolves a key while a modal layer is open (P47): only the bindings of the top layer's
/// context `top`, plus `app.quit` from the full stack `ctx`.
fn modal_outcome(
    state: &DispatcherState,
    ev: &KeyDownEvent,
    ctx: &ContextStack,
    top: &'static str,
    composing: bool,
    now: Instant,
) -> DispatchOutcome {
    let outcome = state
        .0
        .borrow_mut()
        .on_key_down(ev, &ContextStack(vec![top]), composing, now);
    if outcome != DispatchOutcome::PassThrough {
        return outcome;
    }
    // A probe on a copy, so a Workspace chord never starts pending under the modal layer.
    let mut probe = state.0.borrow().clone();
    match probe.on_key_down(ev, ctx, composing, now) {
        DispatchOutcome::Run(id) if id == QUIT_ACTION => DispatchOutcome::Run(id),
        _ => DispatchOutcome::PassThrough,
    }
}

/// Drops the chord that became pending at `since` once [`PENDING_TIMEOUT`] has passed without a
/// newer press.
fn expire_later(cx: &mut App, since: Instant) {
    let timer = cx.background_executor().timer(PENDING_TIMEOUT);
    cx.spawn(async move |cx| {
        timer.await;
        cx.update(|cx| {
            let Some(state) = cx.try_global::<DispatcherState>().cloned() else {
                return;
            };
            let expired = {
                let mut d = state.0.borrow_mut();
                let expired = d.since == Some(since);
                if expired {
                    d.clear_pending();
                }
                expired
            };
            if expired {
                set_pending_chord(cx, None);
            }
        });
    })
    .detach();
}
