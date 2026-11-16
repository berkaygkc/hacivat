//! Keeping the focused control in view (Task 24 fix round 1, I2). GPUI has no scroll-into-view:
//! a scroll container that follows the focus ([`FollowFocus`], the connection dialog's body)
//! opens a scope while its children prepaint, every focusable widget reports its bounds there
//! while it holds the focus ([`focus_probe`], [`note_focus_bounds`]), and when the focus has
//! moved to a control the container shows only in part (or not at all), the container scrolls
//! by the least amount that shows it, 8 px clear of the edge. A control the user scrolled away
//! from stays where it is until the focus moves again; leaving the container (a footer button)
//! and coming back to the same control is such a move. Nothing is drawn.

// ## m2b-t24-shots

use std::cell::RefCell;
use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    App, Bounds, FocusHandle, Global, IntoElement, Pixels, ScrollHandle, Styled, Window, canvas, px,
};

/// Where the focused control was laid out inside a following container, with its focus.
type Found = Rc<RefCell<Option<(FocusHandle, Bounds<Pixels>)>>>;

/// The following containers being prepainted, innermost last.
#[derive(Default)]
struct FollowScopes(Vec<Found>);

impl Global for FollowScopes {}

/// How far from the container's edge a revealed control ends.
const MARGIN_PX: f32 = 8.0;

/// Records `bounds` as the focused control's when `handle` holds the focus and a following
/// container is being prepainted around it. Widgets with their own element call it from their
/// prepaint; plain divs add a [`focus_probe`].
pub fn note_focus_bounds(
    handle: &FocusHandle,
    bounds: Bounds<Pixels>,
    window: &Window,
    cx: &mut App,
) {
    if !handle.is_focused(window) {
        return;
    }
    if let Some(found) = cx.try_global::<FollowScopes>().and_then(|s| s.0.last()) {
        *found.borrow_mut() = Some((handle.clone(), bounds));
    }
}

/// An absolute probe, drawing nothing, that records its parent's bounds while `handle` holds
/// the focus. Its parent must be positioned (`relative`).
pub fn focus_probe(handle: FocusHandle) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| note_focus_bounds(&handle, bounds, window, cx),
        |_, (), _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// A scroll container that follows the focus (see the module docs). Keep one per container
/// across frames; it owns the container's [`ScrollHandle`].
#[derive(Clone, Default)]
pub struct FollowFocus {
    scroll: ScrollHandle,
    found: Found,
    followed: Rc<RefCell<Option<FocusHandle>>>,
}

impl FollowFocus {
    pub fn new() -> Self {
        Self::default()
    }

    /// The container's scroll handle (`track_scroll`).
    pub fn scroll_handle(&self) -> &ScrollHandle {
        &self.scroll
    }

    /// The container's first child: opens the scope its children report into.
    pub fn scope_start(&self) -> impl IntoElement {
        let found = self.found.clone();
        canvas(
            move |_, _, cx| {
                *found.borrow_mut() = None;
                cx.default_global::<FollowScopes>().0.push(found.clone());
            },
            |_, (), _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_0()
    }

    /// The container's `on_children_prepainted`: closes the scope and, when the focus moved to
    /// a control the container does not show whole, scrolls to it (applied in the next frame).
    pub fn scope_end(&self, window: &mut Window, cx: &mut App) {
        cx.default_global::<FollowScopes>().0.pop();
        let found = self.found.borrow_mut().take();
        // ## m2b-t24-shots (follow-up, R3)
        if !is_new_focus(
            found.as_ref().map(|(focus, _)| focus),
            &mut self.followed.borrow_mut(),
        ) {
            return;
        }
        let Some((focus, bounds)) = found else {
            return;
        };
        let view = self.scroll.bounds();
        if view.size.height <= px(0.0) {
            // Not laid out yet: the next frame tries again.
            return;
        }
        *self.followed.borrow_mut() = Some(focus);
        let y = reveal_offset(
            f32::from(self.scroll.offset().y),
            f32::from(self.scroll.max_offset().y),
            (f32::from(view.top()), f32::from(view.bottom())),
            (f32::from(bounds.top()), f32::from(bounds.bottom())),
        );
        let mut offset = self.scroll.offset();
        if (f32::from(offset.y) - y).abs() > 0.5 {
            offset.y = px(y);
            self.scroll.set_offset(offset);
            window.refresh();
        }
    }
}

// ## m2b-t24-shots (follow-up, R3)
/// Whether `found`, the control holding the focus inside the container this frame (`None` when
/// the focus is elsewhere), is one to reveal: not the one `followed` last. With the focus outside
/// the container, `followed` is forgotten, so coming back to the same control reveals it again.
fn is_new_focus<T: PartialEq>(found: Option<&T>, followed: &mut Option<T>) -> bool {
    match found {
        None => {
            *followed = None;
            false
        }
        Some(found) => followed.as_ref() != Some(found),
    }
}

/// The scroll offset (0 at the top, negative further down, never past `-max`) that shows the
/// control spanning `control` (top, bottom) inside the viewport spanning `view`, given the
/// current offset `y`: unchanged when it already shows, else the least move that shows it with
/// [`MARGIN_PX`] to spare; a control taller than the viewport shows its top.
fn reveal_offset(y: f32, max: f32, view: (f32, f32), control: (f32, f32)) -> f32 {
    let (view_top, view_bottom) = view;
    let (top, bottom) = control;
    let wanted =
        if top - MARGIN_PX < view_top || bottom - top + 2.0 * MARGIN_PX > view_bottom - view_top {
            y + (view_top - (top - MARGIN_PX))
        } else if bottom + MARGIN_PX > view_bottom {
            y - (bottom + MARGIN_PX - view_bottom)
        } else {
            y
        };
    wanted.clamp(-max.max(0.0), 0.0)
}

#[cfg(test)]
mod tests {
    use super::{is_new_focus, reveal_offset};

    // ## m2b-t24-shots (follow-up, R3)
    #[test]
    fn leaving_the_container_forgets_the_followed_control() {
        let mut followed = None;
        // The focus lands on control 1: revealed (`scope_end` then remembers it).
        assert!(is_new_focus(Some(&1), &mut followed));
        followed = Some(1);
        // Still on 1, the user scrolled it away: left where it is.
        assert!(!is_new_focus(Some(&1), &mut followed));
        assert_eq!(followed, Some(1));
        // Another control: revealed.
        assert!(is_new_focus(Some(&2), &mut followed));
        // Tab to a footer button, outside the container: nothing to reveal, 1 is forgotten.
        assert!(!is_new_focus(None, &mut followed));
        assert_eq!(followed, None);
        // Shift-Tab back onto 1: revealed again.
        assert!(is_new_focus(Some(&1), &mut followed));
    }

    #[test]
    fn a_control_is_revealed_by_the_least_move() {
        // A 300 px viewport from 100 to 400, nothing scrolled, 500 px of room to scroll.
        let view = (100.0, 400.0);
        assert_eq!(
            reveal_offset(0.0, 500.0, view, (150.0, 176.0)),
            0.0,
            "already shown"
        );
        // Below the fold: scrolled up until its bottom is 8 px clear of the viewport's bottom.
        assert_eq!(reveal_offset(0.0, 500.0, view, (500.0, 526.0)), -134.0);
        // Above the top after scrolling down: back until its top is 8 px clear.
        assert_eq!(reveal_offset(-200.0, 500.0, view, (60.0, 86.0)), -152.0);
        // Never past either end.
        assert_eq!(reveal_offset(0.0, 50.0, view, (900.0, 926.0)), -50.0);
        assert_eq!(reveal_offset(-10.0, 500.0, view, (20.0, 46.0)), 0.0);
        // Taller than the viewport: its top shows.
        assert_eq!(reveal_offset(0.0, 500.0, view, (300.0, 700.0)), -192.0);
    }
}
