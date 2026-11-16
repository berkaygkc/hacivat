//! M2a Task 1: the shell's hook slots, focus contexts and anchored layers. With a slot empty
//! the M1 element renders (the M1 suites cover that); these tests fill the slots.

mod support;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, InteractiveElement as _, IntoElement, Modifiers, Render, Styled as _,
    TestAppContext, Window, div, point, px,
};
use hacivat_app::shell::{
    POPOVER_CONTEXT, register_focus_context, set_capsule, set_editor_empty, set_escape,
    set_explorer, set_status_left,
};
use hacivat_ui::capsule::CapsuleModel;
use hacivat_ui::statusbar::status_label;
use hacivat_ui::text::TextSource;
use support::*;

/// A view that marks itself with debug selector `probe-view`.
struct Probe;

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "probe-view".to_string())
            .size(px(40.0))
    }
}

#[gpui::test]
fn filled_slots_replace_the_m1_elements(cx: &mut TestAppContext) {
    let built = Rc::new(Cell::new(0));
    let count = built.clone();
    cx.update(|cx| {
        set_capsule(
            cx,
            Rc::new(|_| {
                Some(CapsuleModel::Placeholder {
                    label: "hooked capsule".into(),
                })
            }),
        );
        set_status_left(
            cx,
            Rc::new(|theme, _| {
                vec![status_label(theme, "hooked status", TextSource::Data).into_any_element()]
            }),
        );
        set_editor_empty(
            cx,
            Rc::new(|_, _| {
                div()
                    .debug_selector(|| "hooked-editor".to_string())
                    .size(px(10.0))
                    .into_any_element()
            }),
        );
        set_explorer(
            cx,
            Rc::new(move |_, cx| {
                count.set(count.get() + 1);
                cx.new(|_| Probe).into()
            }),
        );
    });
    let vcx = shell(cx, "en");
    let texts: Vec<String> = registry_frame(vcx)
        .into_iter()
        .map(|r| r.text.to_string())
        .collect();
    assert!(texts.contains(&"hooked capsule".to_string()), "{texts:?}");
    assert!(status_bar_text(vcx).starts_with("hooked status"));
    assert!(!status_bar_text(vcx).contains("No connection"));
    assert!(vcx.debug_bounds("hooked-editor").is_some());
    assert!(vcx.debug_bounds("probe-view").is_some());
    // The explorer view is built once, however often the workspace renders.
    with_ws(vcx, |_, _, cx| cx.notify());
    vcx.run_until_parked();
    assert_eq!(built.get(), 1);
}

#[gpui::test]
fn escape_goes_to_layers_first_then_to_the_hook(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let calls = Rc::new(Cell::new(0));
    let seen = calls.clone();
    vcx.cx.update(|cx| {
        set_escape(
            cx,
            Rc::new(move |_, _| {
                seen.set(seen.get() + 1);
                true
            }),
        );
    });
    push_test_layer(vcx, "a", "LayerA");
    vcx.simulate_keystrokes("escape");
    assert!(layer_ids(vcx).is_empty());
    assert_eq!(calls.get(), 0);
    vcx.simulate_keystrokes("escape");
    assert_eq!(calls.get(), 1);
}

#[gpui::test]
fn a_registered_focus_context_joins_the_stack_while_focused(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let handle = focus_test_input(vcx);
    vcx.cx
        .update(|cx| register_focus_context(cx, handle.clone(), "Explorer"));
    assert_eq!(context_stack(vcx).0, ["Workspace", "Test", "Explorer"]);
    push_test_layer(vcx, "a", "LayerA"); // the layer takes the focus
    assert_eq!(context_stack(vcx).0, ["Workspace", "Test", "LayerA"]);
}

#[gpui::test]
fn an_anchored_layer_is_a_non_modal_popover_closed_by_a_press_outside(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    with_ws(vcx, |w, window, cx| {
        let view = cx.new(|_| Probe).into();
        w.open_anchored("explain", point(px(100.0), px(120.0)), view, window, cx);
    });
    vcx.run_until_parked();
    assert_eq!(layer_ids(vcx), ["explain"]);
    assert_eq!(context_stack(vcx).0.last(), Some(&POPOVER_CONTEXT));
    assert_eq!(with_ws(vcx, |w, _, _| w.modal_context()), None);
    let probe = vcx
        .debug_bounds("probe-view")
        .expect("the popover is drawn");
    assert_eq!(probe.origin, point(px(100.0), px(120.0)));
    vcx.simulate_click(point(px(600.0), px(500.0)), Modifiers::none());
    assert!(layer_ids(vcx).is_empty());
}

#[gpui::test]
fn a_dialog_is_modal_and_names_its_context(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    assert_eq!(with_ws(vcx, |w, _, _| w.modal_context()), None);
    run(vcx, "app.about");
    assert_eq!(with_ws(vcx, |w, _, _| w.modal_context()), Some("Dialog"));
}
