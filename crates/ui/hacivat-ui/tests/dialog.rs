use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, Context, Entity, IntoElement, ParentElement, Render, TestAppContext, VisualTestContext,
    Window,
};
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{Dialog, DialogButton, DialogEvent, DialogSpec};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::theme::{Theme, ThemeMode};

pub type Events = Rc<RefCell<Vec<DialogEvent>>>;

struct DialogHost {
    dialog: Entity<Dialog>,
}

impl Render for DialogHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().child(self.dialog.clone())
    }
}

fn text(s: &'static str) -> TextFn {
    Rc::new(move |_: &App| s.into())
}

pub fn spec_two_buttons(cancel: usize, primary: usize) -> DialogSpec {
    spec_titled("Delete?", &["Cancel", "Delete"], cancel, primary)
}

pub fn spec_titled(
    title: &'static str,
    labels: &[&'static str],
    cancel: usize,
    primary: usize,
) -> DialogSpec {
    DialogSpec {
        title: text(title),
        body: text("This cannot be undone."),
        buttons: labels
            .iter()
            .map(|l| DialogButton {
                label: text(l),
                kind: ButtonKind::Default,
                accesskey: None,
                english_accesskey: None,
            })
            .collect(),
        cancel,
        primary,
    }
}

pub fn open_test_dialog(
    cx: &mut TestAppContext,
    spec: DialogSpec,
) -> (Entity<Dialog>, &mut VisualTestContext, Events) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let events: Events = Rc::default();
    let sink = events.clone();
    let (host, vcx) = cx.add_window_view(|window, cx| {
        let dialog = Dialog::new(spec, window, cx);
        cx.subscribe(&dialog, move |_, _, event: &DialogEvent, _| {
            sink.borrow_mut().push(*event)
        })
        .detach();
        DialogHost { dialog }
    });
    let dialog = host.read_with(vcx, |h, _| h.dialog.clone());
    (dialog, vcx, events)
}

fn focused_button(d: &Entity<Dialog>, vcx: &mut VisualTestContext) -> Option<usize> {
    let d = d.clone();
    vcx.update(|window, cx| d.read(cx).focused_button(window))
}

#[gpui::test]
fn dialog_traps_focus_and_escape_cancels(cx: &mut TestAppContext) {
    let (d, vcx, events) = open_test_dialog(cx, spec_two_buttons(0, 1));
    assert_eq!(focused_button(&d, vcx), Some(1));
    vcx.simulate_keystrokes("tab");
    assert_eq!(focused_button(&d, vcx), Some(0));
    vcx.simulate_keystrokes("tab");
    assert_eq!(focused_button(&d, vcx), Some(1));
    vcx.simulate_keystrokes("shift-tab shift-tab shift-tab");
    assert_eq!(focused_button(&d, vcx), Some(0));
    vcx.simulate_keystrokes("escape");
    assert_eq!(events.take(), vec![DialogEvent::Cancelled]);
}

#[gpui::test]
fn dialog_enter_presses_primary(cx: &mut TestAppContext) {
    let (_, vcx, events) = open_test_dialog(cx, spec_two_buttons(0, 1));
    vcx.simulate_keystrokes("enter");
    assert_eq!(events.take(), vec![DialogEvent::Pressed(1)]);
}

#[gpui::test]
fn enter_presses_the_focused_button_not_a_hidden_default(cx: &mut TestAppContext) {
    let (_, vcx, events) = open_test_dialog(cx, spec_two_buttons(0, 1));
    vcx.simulate_keystrokes("tab enter");
    assert_eq!(events.take(), vec![DialogEvent::Pressed(0)]);
}

#[gpui::test]
fn alt_access_key_presses_its_button_when_enabled(cx: &mut TestAppContext) {
    let (d, vcx, events) = open_test_dialog(cx, spec_two_buttons(0, 1));
    d.update(vcx, |d, _| d.set_access_keys(true));
    vcx.simulate_keystrokes("alt-c");
    assert_eq!(events.take(), vec![DialogEvent::Pressed(0)]);
    vcx.simulate_keystrokes("alt-d");
    assert_eq!(events.take(), vec![DialogEvent::Pressed(1)]);
    d.update(vcx, |d, _| d.set_access_keys(false));
    vcx.simulate_keystrokes("alt-c");
    assert_eq!(events.take(), vec![]);
}

fn danger_spec() -> DialogSpec {
    let mut spec = spec_two_buttons(0, 1);
    spec.buttons[1].kind = ButtonKind::Danger;
    spec
}

#[gpui::test]
fn a_danger_primary_does_not_get_the_initial_focus(cx: &mut TestAppContext) {
    let (d, vcx, events) = open_test_dialog(cx, danger_spec());
    assert_eq!(focused_button(&d, vcx), Some(0), "focus starts on cancel");
    vcx.simulate_keystrokes("enter");
    assert_eq!(events.take(), vec![DialogEvent::Pressed(0)]);
}

#[gpui::test]
fn a_held_or_modified_enter_presses_nothing(cx: &mut TestAppContext) {
    let (_, vcx, events) = open_test_dialog(cx, spec_two_buttons(0, 1));
    vcx.simulate_event(gpui::KeyDownEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
        is_held: true,
        prefer_character_input: false,
    });
    assert_eq!(events.take(), vec![]);
    vcx.simulate_keystrokes("shift-enter");
    vcx.simulate_keystrokes("cmd-enter");
    assert_eq!(events.take(), vec![]);
    vcx.simulate_keystrokes("enter");
    assert_eq!(events.take(), vec![DialogEvent::Pressed(1)]);
}

#[gpui::test]
#[should_panic(expected = "at least one button")]
fn a_dialog_without_buttons_trips_the_debug_assertion(cx: &mut TestAppContext) {
    let _ = open_test_dialog(cx, spec_titled("Empty", &[], 0, 0));
}

#[gpui::test]
fn turkic_alt_keys_fold_dotted_and_dotless_i(cx: &mut TestAppContext) {
    use hacivat_ui::accesskey::AccessKeyLocale;
    let (d, vcx, events) = open_test_dialog(cx, spec_titled("T", &["İptal", "Tamam"], 0, 1));
    d.update(vcx, |d, _| d.set_access_keys(true));
    vcx.update(|_, cx| cx.set_global(AccessKeyLocale { turkic: true }));
    vcx.simulate_keystrokes("alt-i");
    assert_eq!(events.take(), vec![DialogEvent::Pressed(0)]);
    vcx.update(|_, cx| cx.set_global(AccessKeyLocale { turkic: false }));
    vcx.simulate_keystrokes("alt-i");
    assert_eq!(events.take(), vec![], "İ is exact outside Turkic locales");
}

#[gpui::test]
fn turkic_mode_still_matches_ordinary_letters_in_a_dialog(cx: &mut TestAppContext) {
    use hacivat_ui::accesskey::AccessKeyLocale;
    let (d, vcx, events) = open_test_dialog(cx, spec_titled("T", &["Çık", "Save"], 0, 1));
    d.update(vcx, |d, _| d.set_access_keys(true));
    for turkic in [false, true] {
        vcx.update(|_, cx| cx.set_global(AccessKeyLocale { turkic }));
        vcx.simulate_keystrokes("alt-ç");
        assert_eq!(
            events.take(),
            vec![DialogEvent::Pressed(0)],
            "turkic={turkic}"
        );
        vcx.simulate_keystrokes("alt-s");
        assert_eq!(
            events.take(),
            vec![DialogEvent::Pressed(1)],
            "turkic={turkic}"
        );
    }
}

#[gpui::test]
fn info_matches_i_only_outside_turkic_mode(cx: &mut TestAppContext) {
    use hacivat_ui::accesskey::AccessKeyLocale;
    let (d, vcx, events) = open_test_dialog(cx, spec_titled("T", &["Info", "Tamam"], 0, 1));
    d.update(vcx, |d, _| d.set_access_keys(true));
    vcx.update(|_, cx| cx.set_global(AccessKeyLocale { turkic: false }));
    vcx.simulate_keystrokes("alt-i");
    assert_eq!(events.take(), vec![DialogEvent::Pressed(0)]);
    vcx.update(|_, cx| cx.set_global(AccessKeyLocale { turkic: true }));
    vcx.simulate_keystrokes("alt-i");
    assert_eq!(events.take(), vec![]);
}

#[gpui::test]
fn dotless_i_selects_isik_in_turkic_mode(cx: &mut TestAppContext) {
    use hacivat_ui::accesskey::AccessKeyLocale;
    let (d, vcx, events) = open_test_dialog(cx, spec_titled("T", &["Işık", "Tamam"], 0, 1));
    d.update(vcx, |d, _| d.set_access_keys(true));
    vcx.update(|_, cx| cx.set_global(AccessKeyLocale { turkic: true }));
    vcx.simulate_keystrokes("alt-ı");
    assert_eq!(events.take(), vec![DialogEvent::Pressed(0)]);
}

#[gpui::test]
fn clicking_a_button_presses_it(cx: &mut TestAppContext) {
    let (_, vcx, events) = open_test_dialog(cx, spec_two_buttons(0, 1));
    let cancel = vcx.debug_bounds("button-dialog-button-0").unwrap();
    vcx.simulate_click(cancel.center(), gpui::Modifiers::none());
    assert_eq!(events.take(), vec![DialogEvent::Pressed(0)]);
}
