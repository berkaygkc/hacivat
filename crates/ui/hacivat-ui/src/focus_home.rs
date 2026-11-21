//! Focus recovery (ruling R-T33d-5): when the focused control stops being drawn (a button its own
//! press removes or disables, a toolbar item that overflows into ⋯), the focus goes to the
//! primary target of the surface that held it, so Tab and the surface's keys keep working.
//!
//! GPUI moves no focus by itself: the window keeps the focus on a handle no frame draws, Tab
//! does nothing from there and the surface's key context is gone. GPUI does report it, once per
//! frame: `Context::on_focus_lost` runs when the frame just drawn no longer contains the focused
//! element. [`recover_lost_focus`] listens there, at the window's root (the shell), and moves the
//! focus to:
//!
//! 1. the primary of the innermost [`FocusHome`] around the lost control, if that primary is
//!    drawn in this frame; else the next home out, and so on;
//! 2. else the root's own fallback (the shell: the active pane's default).
//!
//! It never acts while the focused element is drawn (GPUI does not report that), and never for
//! a home that is not drawn this frame (a hidden tab or pane, a closed dialog): such a home is
//! passed over. It acts only for a control whose [`focus_probe`] saw it focused in the last frame
//! that drew it, and only while the focus is still there: a program that moves the focus to a
//! handle it has not drawn yet is left alone.
//!
//! Homes are recorded while the frame prepaints: a [`FocusHome`] opens a scope around its
//! children, and every focusable widget's [`focus_probe`] reports into it. A home's primary is
//! a handle its owner names ([`focus_home`]: a console's editor, the results grid) or the first
//! Tab stop drawn inside it ([`first_stop_home`]: a dialog, a pane). A home inside a
//! first-stop home gives that home its primary. Nothing is drawn.
//!
//! [`focus_probe`]: crate::follow::focus_probe

// ## m4-t33d-fr1 (ruling R-T33d-5)

use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::Location;
use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyWindowHandle, App, Bounds, Context, Element, ElementId, FocusHandle, Global,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels, Styled, Subscription,
    WeakFocusHandle, Window, canvas,
};

/// Where a home's primary target is kept across frames. The owner keeps one per home (a view
/// field), so a control that is gone still finds the home it was drawn in.
#[derive(Clone, Default)]
pub struct HomeSlot(Rc<RefCell<Option<WeakFocusHandle>>>);

impl HomeSlot {
    pub fn new() -> Self {
        Self::default()
    }

    /// The primary the last frame that drew this home found, if it is still alive.
    pub fn primary(&self) -> Option<FocusHandle> {
        self.0.borrow().as_ref().and_then(WeakFocusHandle::upgrade)
    }

    fn set(&self, primary: Option<WeakFocusHandle>) {
        *self.0.borrow_mut() = primary;
    }
}

/// A home's primary target.
#[derive(Clone)]
enum Primary {
    /// A handle the owner names.
    Handle(FocusHandle),
    /// The first Tab stop drawn inside the home (or the primary of the first home inside it).
    FirstStop,
}

/// One open home while the frame prepaints.
struct Scope {
    slot: HomeSlot,
    found: Option<WeakFocusHandle>,
}

/// The focused control as its probe last saw it: its handle and the homes around it, outermost
/// first.
struct Mark {
    handle: WeakFocusHandle,
    homes: Vec<HomeSlot>,
}

#[derive(Default)]
struct WindowState {
    scopes: Vec<Scope>,
    mark: Option<Mark>,
}

/// Per window: the homes being prepainted and the focused control's mark.
#[derive(Default)]
struct FocusHomes(HashMap<AnyWindowHandle, WindowState>);

impl Global for FocusHomes {}

fn state<'a>(window: &Window, cx: &'a mut App) -> &'a mut WindowState {
    cx.default_global::<FocusHomes>()
        .0
        .entry(window.window_handle())
        .or_default()
}

/// A home whose primary target is `primary` (a surface's editor, grid or tree).
pub fn focus_home(slot: &HomeSlot, primary: FocusHandle, child: impl IntoElement) -> FocusHome {
    FocusHome {
        slot: slot.clone(),
        primary: Primary::Handle(primary),
        child: Some(child.into_any_element()),
    }
}

/// A home whose primary target is the first Tab stop drawn inside it, or the primary of the
/// first home inside it (a dialog: its first stop; a pane: its content's home).
pub fn first_stop_home(slot: &HomeSlot, child: impl IntoElement) -> FocusHome {
    FocusHome {
        slot: slot.clone(),
        primary: Primary::FirstStop,
        child: Some(child.into_any_element()),
    }
}

/// Called by [`crate::follow::focus_probe`] while a focusable widget prepaints: offers it as the
/// first stop of the open first-stop homes and, while it holds the focus, marks it with the homes
/// around it.
pub(crate) fn note_drawn(handle: &FocusHandle, window: &Window, cx: &mut App) {
    let state = state(window, cx);
    if handle.tab_stop {
        for scope in state.scopes.iter_mut().filter(|s| s.found.is_none()) {
            scope.found = Some(handle.downgrade());
        }
    }
    if handle.is_focused(window) {
        state.mark = Some(Mark {
            handle: handle.downgrade(),
            homes: state.scopes.iter().map(|s| s.slot.clone()).collect(),
        });
    }
}

/// The root's first child: at the start of every frame, forgets a mark the focus has left, so a
/// later loss of some other element never acts on it.
pub fn frame_start() -> impl IntoElement {
    canvas(
        |_, window, cx| {
            let focused = window.focused(cx);
            // A closed window's state goes.
            let open = cx.windows();
            cx.default_global::<FocusHomes>()
                .0
                .retain(|w, _| open.contains(w));
            let state = state(window, cx);
            state.scopes.clear();
            let stale = state
                .mark
                .as_ref()
                .is_some_and(|m| focused.as_ref().is_none_or(|f| m.handle != *f));
            if stale {
                state.mark = None;
            }
        },
        |_, (), _, _| {},
    )
    .absolute()
    .size_0()
}

/// Installs the recovery on `cx`'s view, the window's root: `root` is the handle its root
/// element tracks (a target counts as drawn when it is drawn under it), `fallback` gives the
/// target when no home around the lost control has a drawn primary (the shell: the active
/// pane's default; `None` to leave the focus where it is). The root must draw
/// [`frame_start`] as its first child. Keep the subscription for the window's life.
pub fn recover_lost_focus<T: 'static>(
    root: FocusHandle,
    window: &mut Window,
    cx: &mut Context<T>,
    fallback: impl Fn(&mut T, &mut Window, &mut Context<T>) -> Option<FocusHandle> + 'static,
) -> Subscription {
    cx.on_focus_lost(window, move |this, window, cx| {
        let Some(mark) = state(window, cx).mark.take() else {
            return;
        };
        // The focus moved on to a handle not drawn yet (a program's choice): not ours.
        if window.focused(cx).is_some_and(|f| mark.handle != f) {
            return;
        }
        let drawn = |target: &FocusHandle, window: &Window| {
            *target != root && root.contains(target, window)
        };
        let target = mark
            .homes
            .iter()
            .rev()
            .filter_map(HomeSlot::primary)
            .find(|p| drawn(p, window))
            .or_else(|| fallback(this, window, cx).filter(|p| drawn(p, window)));
        if let Some(target) = target {
            window.focus(&target, cx);
            // GPUI draws nothing for a focus a focus-lost listener moves: the next frame shows it
            // (the focus ring, the caret) and sends its focus events.
            cx.notify();
        }
    })
}

/// See [`focus_home`] and [`first_stop_home`].
pub struct FocusHome {
    slot: HomeSlot,
    primary: Primary,
    child: Option<AnyElement>,
}

impl Element for FocusHome {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        let mut child = self.child.take().expect("a focus home is laid out once");
        let layout = child.request_layout(window, cx);
        (layout, child)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        let found = match &self.primary {
            Primary::Handle(handle) => Some(handle.downgrade()),
            Primary::FirstStop => None,
        };
        let state = state(window, cx);
        // A named primary is the first stop of the first-stop homes around it.
        if let Some(found) = &found {
            for scope in state.scopes.iter_mut().filter(|s| s.found.is_none()) {
                scope.found = Some(found.clone());
            }
        }
        state.scopes.push(Scope {
            slot: self.slot.clone(),
            found,
        });
        child.prepaint(window, cx);
        if let Some(scope) = state_pop(window, cx) {
            scope.slot.set(scope.found);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut AnyElement,
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

fn state_pop(window: &Window, cx: &mut App) -> Option<Scope> {
    state(window, cx).scopes.pop()
}
