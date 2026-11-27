//! ## m4-t33d-fr1 (ruling R-T33d-5): a focused control that stops being drawn hands the focus
//! to its home's primary target (`hacivat_ui::focus_home`).

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, FocusHandle, IntoElement, Render, Subscription, TestAppContext,
    VisualTestContext, Window,
};
use hacivat_ui::button::Button;
use hacivat_ui::focus_home::{
    HomeSlot, first_stop_home, focus_home, frame_start, recover_lost_focus,
};
use hacivat_ui_core::gpui_surface::{InteractiveElement, ParentElement, Styled, div};
use hacivat_ui_core::theme::{Theme, ThemeMode};

type Build = Box<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// A window root that installs the recovery, like the shell's workspace.
struct Root {
    focus: FocusHandle,
    fallback: Option<FocusHandle>,
    build: Build,
    _recovery: Subscription,
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .child(frame_start())
            .child((self.build)(window, cx))
    }
}

fn root(
    cx: &mut TestAppContext,
    fallback: Option<FocusHandle>,
    build: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
) -> &mut VisualTestContext {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let focus = cx.focus_handle();
        let recovery = recover_lost_focus(focus.clone(), window, cx, |root: &mut Root, _, _| {
            root.fallback.clone()
        });
        Root {
            focus,
            fallback,
            build: Box::new(build),
            _recovery: recovery,
        }
    });
    vcx.run_until_parked();
    vcx
}

/// A focusable area that is not a Tab stop (an editor's or grid's handle).
fn area(handle: &FocusHandle) -> impl IntoElement {
    div().track_focus(handle).w_full().h_8()
}

fn press_enter(vcx: &mut VisualTestContext) {
    vcx.simulate_keystrokes("enter");
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
    });
    vcx.run_until_parked();
}

fn focus(vcx: &mut VisualTestContext, handle: &FocusHandle) {
    vcx.update(|window, cx| window.focus(handle, cx));
    vcx.run_until_parked();
}

fn tab(vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| window.focus_next(cx));
    vcx.run_until_parked();
}

fn focused(vcx: &mut VisualTestContext) -> Option<FocusHandle> {
    vcx.update(|window, cx| window.focused(cx))
}

fn handles(cx: &mut TestAppContext, n: usize) -> Vec<FocusHandle> {
    cx.update(|cx| (0..n).map(|_| cx.focus_handle()).collect())
}

/// A button whose press removes it (`gone`), with a handle the test can name.
fn self_removing(id: &'static str, handle: &FocusHandle, gone: &Rc<Cell<bool>>) -> Button {
    let gone = gone.clone();
    Button::new(id, id.into())
        .focus_handle(handle.clone())
        .tab_stop(true)
        .on_click(move |_, window, _| {
            gone.set(true);
            window.refresh();
        })
}

#[gpui::test]
fn a_button_its_press_removes_hands_the_focus_to_its_homes_primary(cx: &mut TestAppContext) {
    let h = handles(cx, 3);
    let (editor, go, after) = (h[0].clone(), h[1].clone(), h[2].clone());
    let gone = Rc::new(Cell::new(false));
    let slot = HomeSlot::new();
    let (e, g, a, flag) = (editor.clone(), go.clone(), after.clone(), gone.clone());
    let vcx = root(cx, None, move |_, _| {
        let mut body = div().flex().flex_col();
        if !flag.get() {
            body = body.child(self_removing("go", &g, &flag));
        }
        body = body.child(area(&e)).child(
            Button::new("after", "after".into())
                .focus_handle(a.clone())
                .tab_stop(true),
        );
        focus_home(&slot, e.clone(), body).into_any_element()
    });
    focus(vcx, &go);
    press_enter(vcx);
    assert!(gone.get(), "Enter pressed the button");
    assert_eq!(
        focused(vcx),
        Some(editor.clone()),
        "the focus is on the home's primary"
    );
    tab(vcx);
    assert_eq!(focused(vcx), Some(after), "Tab moves on from there");
}

#[gpui::test]
fn a_button_that_turns_disabled_while_focused_hands_the_focus_on(cx: &mut TestAppContext) {
    let h = handles(cx, 2);
    let (editor, go) = (h[0].clone(), h[1].clone());
    let off = Rc::new(Cell::new(false));
    let slot = HomeSlot::new();
    let (e, g, flag) = (editor.clone(), go.clone(), off.clone());
    let vcx = root(cx, None, move |_, _| {
        let button = Button::new("go", "go".into())
            .focus_handle(g.clone())
            .tab_stop(true)
            .disabled(flag.get());
        focus_home(&slot, e.clone(), div().child(button).child(area(&e))).into_any_element()
    });
    focus(vcx, &go);
    off.set(true);
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert_eq!(focused(vcx), Some(editor));
}

#[gpui::test]
fn a_home_whose_primary_is_not_drawn_is_passed_over(cx: &mut TestAppContext) {
    // The inner home (a hidden tab's content) goes with the button; the outer first-stop home
    // (a pane) gives its first stop.
    let h = handles(cx, 4);
    let (first, inner_primary, go, _) = (h[0].clone(), h[1].clone(), h[2].clone(), h[3].clone());
    let gone = Rc::new(Cell::new(false));
    let (outer, inner) = (HomeSlot::new(), HomeSlot::new());
    let (f, ip, g, flag) = (
        first.clone(),
        inner_primary.clone(),
        go.clone(),
        gone.clone(),
    );
    let vcx = root(cx, None, move |_, _| {
        let mut body = div().flex().flex_col().child(
            Button::new("first", "first".into())
                .focus_handle(f.clone())
                .tab_stop(true),
        );
        if !flag.get() {
            body = body.child(focus_home(
                &inner,
                ip.clone(),
                div().child(self_removing("go", &g, &flag)).child(area(&ip)),
            ));
        }
        first_stop_home(&outer, body).into_any_element()
    });
    focus(vcx, &go);
    press_enter(vcx);
    assert_eq!(focused(vcx), Some(first), "the outer home's first stop");
}

#[gpui::test]
fn a_first_stop_home_takes_the_primary_of_a_home_inside_it(cx: &mut TestAppContext) {
    // A pane (first-stop home) around a console (its editor): a button outside both, with no
    // home of its own, goes to the fallback, here the pane's primary: the editor, not the
    // console's first button.
    let h = handles(cx, 3);
    let (editor, run, go) = (h[0].clone(), h[1].clone(), h[2].clone());
    let gone = Rc::new(Cell::new(false));
    let (pane, console) = (HomeSlot::new(), HomeSlot::new());
    let pane_primary = pane.clone();
    let (e, r, g, flag) = (editor.clone(), run.clone(), go.clone(), gone.clone());
    let vcx = root(cx, None, move |_, _| {
        let content = focus_home(
            &console,
            e.clone(),
            div()
                .child(
                    Button::new("run", "run".into())
                        .focus_handle(r.clone())
                        .tab_stop(true),
                )
                .child(area(&e)),
        );
        let mut status = div();
        if !flag.get() {
            status = status.child(self_removing("go", &g, &flag));
        }
        div()
            .child(first_stop_home(&pane, content))
            .child(status)
            .into_any_element()
    });
    assert_eq!(pane_primary.primary(), Some(editor.clone()));
    // Install the pane's primary as the root's fallback.
    vcx.update(|window, cx| {
        let root = window.root::<Root>().flatten().expect("the root view");
        root.update(cx, |root, _| root.fallback = pane_primary.primary());
    });
    focus(vcx, &go);
    press_enter(vcx);
    assert_eq!(
        focused(vcx),
        Some(editor),
        "the fallback: the pane's editor"
    );
}

#[gpui::test]
fn a_program_that_moves_the_focus_to_an_undrawn_handle_is_left_alone(cx: &mut TestAppContext) {
    let h = handles(cx, 3);
    let (editor, go, elsewhere) = (h[0].clone(), h[1].clone(), h[2].clone());
    let slot = HomeSlot::new();
    let (e, g) = (editor.clone(), go.clone());
    let vcx = root(cx, Some(editor.clone()), move |_, _| {
        focus_home(
            &slot,
            e.clone(),
            div()
                .child(
                    Button::new("go", "go".into())
                        .focus_handle(g.clone())
                        .tab_stop(true),
                )
                .child(area(&e)),
        )
        .into_any_element()
    });
    focus(vcx, &go);
    focus(vcx, &elsewhere);
    assert_eq!(focused(vcx), Some(elsewhere), "not drawn, but not ours");
    // A blur is left alone too.
    focus(vcx, &go);
    vcx.update(|window, cx| window.blur(cx));
    vcx.run_until_parked();
    assert_eq!(focused(vcx), None, "a blur stays a blur");
}

#[gpui::test]
fn a_drawn_control_keeps_the_focus(cx: &mut TestAppContext) {
    // A sibling goes; the focused button stays drawn and keeps the focus.
    let h = handles(cx, 3);
    let (editor, stay, go) = (h[0].clone(), h[1].clone(), h[2].clone());
    let gone = Rc::new(Cell::new(false));
    let slot = HomeSlot::new();
    let (e, s, g, flag) = (editor.clone(), stay.clone(), go.clone(), gone.clone());
    let vcx = root(cx, Some(editor.clone()), move |_, _| {
        let mut body = div().child(
            Button::new("stay", "stay".into())
                .focus_handle(s.clone())
                .tab_stop(true),
        );
        if !flag.get() {
            body = body.child(self_removing("go", &g, &flag));
        }
        focus_home(&slot, e.clone(), body.child(area(&e))).into_any_element()
    });
    focus(vcx, &stay);
    gone.set(true);
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert_eq!(focused(vcx), Some(stay));
}

#[gpui::test]
fn without_a_home_the_fallback_takes_the_focus(cx: &mut TestAppContext) {
    let h = handles(cx, 2);
    let (editor, go) = (h[0].clone(), h[1].clone());
    let gone = Rc::new(Cell::new(false));
    let (e, g, flag) = (editor.clone(), go.clone(), gone.clone());
    let vcx = root(cx, Some(editor.clone()), move |_, _| {
        let mut body = div().child(area(&e));
        if !flag.get() {
            body = body.child(self_removing("go", &g, &flag));
        }
        body.into_any_element()
    });
    focus(vcx, &go);
    press_enter(vcx);
    assert_eq!(focused(vcx), Some(editor));
}
