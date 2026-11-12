//! `ImeState` and `TextInput` (Task 11): UTF-16 offsets, composition, focus.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, Entity, EntityInputHandler as _, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, TestAppContext,
    VisualTestContext, Window, div,
};
use hacivat_ui::input::ime::ImeState;
use hacivat_ui::input::{FocusedInput, InputEvent, TextInput};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::theme::{Theme, ThemeMode};

#[test]
fn replace_and_mark_sets_marked_range() {
    let mut s = ImeState::new("SELECT ");
    s.replace_and_mark(None, "ç", None);
    assert_eq!(s.marked_utf16(), Some(7..8));
}

#[test]
fn unmark_commits_marked_text() {
    let mut s = ImeState::new("x");
    s.replace_and_mark(None, "ş", None);
    s.unmark();
    assert_eq!((s.text(), s.marked_utf16()), ("xş", None));
}

#[test]
fn composition_replaced_by_commit() {
    let mut s = ImeState::new("");
    s.replace_and_mark(None, "k", None);
    s.replace_and_mark(None, "か", None);
    s.replace(None, "感");
    assert_eq!(s.text(), "感");
}

#[test]
fn utf16_with_turkish_and_emoji() {
    let s = ImeState::new("İı😀x");
    assert_eq!(s.selection_utf16(), 5..5);
}

#[test]
fn backspace_removes_one_grapheme() {
    let mut s = ImeState::new("ai\u{0307}");
    s.backspace();
    assert_eq!(s.text(), "a");
}

// --- Further model behaviour (M0 ImeCore cases, re-pinned on the M1 model) ---

#[test]
fn composition_selection_is_relative_to_the_marked_text() {
    let mut s = ImeState::new("ab");
    s.replace_and_mark(None, "かな", Some(0..1));
    assert_eq!(s.selection_utf16(), 2..3);
    assert_eq!(s.marked_utf16(), Some(2..4));
}

#[test]
fn explicit_range_wins_over_composition() {
    let mut s = ImeState::new("abc");
    s.replace_and_mark(None, "x", None);
    s.replace(Some(0..1), "Z");
    assert_eq!(s.text(), "Zbcx");
    assert_eq!(s.marked_utf16(), None);
}

#[test]
fn utf16_offsets_count_surrogate_pairs() {
    let mut s = ImeState::new("😀");
    s.replace_and_mark(None, "ş", None);
    assert_eq!(s.marked_utf16(), Some(2..3));
    assert_eq!(s.selection_utf16(), 3..3);
}

#[test]
fn dead_key_commit_replaces_the_marked_dead_key() {
    let mut s = ImeState::new("SELECT ");
    s.replace_and_mark(None, "ʼ", Some(1..1));
    s.replace(None, "ʼa");
    assert_eq!(s.text(), "SELECT ʼa");
    assert_eq!(s.marked_utf16(), None);
    assert_eq!(s.selection_utf16(), 9..9);
}

#[test]
fn empty_mark_cancels_the_composition() {
    let mut s = ImeState::new("x");
    s.replace_and_mark(None, "ş", None);
    s.replace_and_mark(None, "", None);
    assert_eq!(
        (s.text(), s.marked_utf16(), s.is_composing()),
        ("x", None, false)
    );
}

// Bug 1 of the M0 IME check (PO, macOS): an app switch leaves the composition with the
// platform, which re-delivers it on return; a local commit would duplicate it.
#[test]
fn blur_on_app_switch_keeps_composition_for_the_platform() {
    let mut s = ImeState::new("SELECT ");
    s.replace_and_mark(None, "感じ", Some(2..2));
    assert!(!s.blur(false));
    assert_eq!(s.marked_utf16(), Some(7..9));
    s.replace(None, "感じ");
    assert_eq!(s.text(), "SELECT 感じ");
}

#[test]
fn blur_within_active_window_commits_composition() {
    let mut s = ImeState::new("x");
    s.replace_and_mark(None, "ş", None);
    assert!(s.blur(true));
    assert_eq!((s.text(), s.marked_utf16()), ("xş", None));
}

#[test]
fn text_for_range_clamps_and_reports_the_adjusted_range() {
    let s = ImeState::new("İş");
    let mut adjusted = None;
    assert_eq!(s.text_for_range_utf16(1..99, &mut adjusted), "ş");
    assert_eq!(adjusted, Some(1..2));
}

#[test]
fn caret_moves_by_grapheme_and_select_all_replaces() {
    let mut s = ImeState::new("ai\u{0307}😀");
    s.move_left();
    assert_eq!(s.selection_utf16(), 3..3);
    s.move_left();
    assert_eq!(s.selection_utf16(), 1..1);
    s.move_right();
    assert_eq!(s.selection_utf16(), 3..3);
    s.select_all();
    assert_eq!(s.selection_utf16(), 0..5);
    s.replace(None, "ğ");
    assert_eq!((s.text(), s.selection_utf16()), ("ğ", 1..1));
}

#[test]
fn backspace_deletes_the_selection_first() {
    let mut s = ImeState::new("İstanbul");
    s.set_selection_utf16(0..2);
    s.backspace();
    assert_eq!(s.text(), "tanbul");
    assert_eq!(s.selection_utf16(), 0..0);
    s.backspace();
    assert_eq!(s.text(), "tanbul");
}

// --- TextInput in a window ---

struct Host {
    input: Entity<TextInput>,
    other: FocusHandle,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(self.input.clone())
            .child(div().track_focus(&self.other).child("other"))
    }
}

fn placeholder() -> TextFn {
    Rc::new(|_| "Search".into())
}

fn focused_input_and_other(
    cx: &mut TestAppContext,
) -> (Entity<TextInput>, FocusHandle, &mut VisualTestContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (host, vcx) = cx.add_window_view(|_, cx| Host {
        input: cx.new(|cx| TextInput::new(cx, placeholder())),
        other: cx.focus_handle(),
    });
    let (input, other) = host.read_with(vcx, |h, _| (h.input.clone(), h.other.clone()));
    vcx.update(|w, cx| {
        // The window is key, as when the user types into it.
        w.activate_window();
        input.focus_handle(cx).focus(w, cx);
    });
    vcx.run_until_parked();
    (input, other, vcx)
}

fn focused_input(cx: &mut TestAppContext) -> (Entity<TextInput>, &mut VisualTestContext) {
    let (input, _, vcx) = focused_input_and_other(cx);
    (input, vcx)
}

fn record_input_events(
    input: &Entity<TextInput>,
    vcx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<InputEvent>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(input, move |_, e: &InputEvent, _| {
            sink.borrow_mut().push(*e)
        })
        .detach()
    });
    events
}

#[gpui::test]
fn typing_turkish_through_input_handler(cx: &mut TestAppContext) {
    let (input, vcx) = focused_input(cx);
    vcx.simulate_input("ğüşıöçİ");
    input.read_with(vcx, |i, _| assert_eq!(i.value(), "ğüşıöçİ"));
}

#[gpui::test]
fn blur_commits_composition(cx: &mut TestAppContext) {
    let (input, other, vcx) = focused_input_and_other(cx);
    let events = record_input_events(&input, vcx);
    input.update_in(vcx, |i, w, cx| {
        i.replace_and_mark_text_in_range(None, "ş", None, w, cx)
    });
    assert!(input.read_with(vcx, |i, _| i.is_composing()));
    assert!(vcx.read(|cx| cx.global::<FocusedInput>().composing));
    vcx.update(|w, cx| other.focus(w, cx));
    vcx.run_until_parked();
    input.read_with(vcx, |i, _| {
        assert_eq!(i.value(), "ş");
        assert!(!i.is_composing());
    });
    assert!(!vcx.read(|cx| cx.global::<FocusedInput>().composing));
    let ev = events.take();
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(e, InputEvent::CompositionChanged(_)))
            .collect::<Vec<_>>(),
        [
            &InputEvent::CompositionChanged(true),
            &InputEvent::CompositionChanged(false)
        ]
    );
}

/// The M0 app-switch fix (PO-verified on macOS): when the window resigns key mid-composition,
/// the platform keeps the composition, so the input must keep its marked range too.
#[gpui::test]
fn app_switch_keeps_composition_for_the_platform(cx: &mut TestAppContext) {
    let (input, vcx) = focused_input(cx);
    let events = record_input_events(&input, vcx);
    input.update_in(vcx, |i, w, cx| {
        i.replace_and_mark_text_in_range(None, "感じ", Some(2..2), w, cx)
    });
    vcx.deactivate_window();
    input.read_with(vcx, |i, _| {
        assert!(i.is_composing());
        assert_eq!(i.value(), "感じ");
    });
    // The window lost key: no input is focused for the key dispatcher.
    assert_eq!(
        vcx.read(|cx| *cx.global::<FocusedInput>()),
        FocusedInput::default()
    );
    // Back in the app the IME commits its composition: no second copy.
    vcx.update(|w, _| w.activate_window());
    vcx.run_until_parked();
    let id = input.entity_id();
    assert_eq!(
        vcx.read(|cx| *cx.global::<FocusedInput>()),
        FocusedInput {
            entity: Some(id),
            composing: true
        }
    );
    input.update_in(vcx, |i, w, cx| i.replace_text_in_range(None, "感じ", w, cx));
    input.read_with(vcx, |i, _| {
        assert_eq!(i.value(), "感じ");
        assert!(!i.is_composing());
    });
    assert!(!vcx.read(|cx| cx.global::<FocusedInput>().composing));
    let ev = events.take();
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(e, InputEvent::CompositionChanged(_)))
            .collect::<Vec<_>>(),
        [
            &InputEvent::CompositionChanged(true),
            &InputEvent::CompositionChanged(false)
        ]
    );
}

#[gpui::test]
fn focus_and_blur_keep_focused_input_current(cx: &mut TestAppContext) {
    let (input, other, vcx) = focused_input_and_other(cx);
    let id = input.entity_id();
    assert_eq!(
        vcx.read(|cx| *cx.global::<FocusedInput>()),
        FocusedInput {
            entity: Some(id),
            composing: false
        }
    );
    vcx.update(|w, cx| other.focus(w, cx));
    vcx.run_until_parked();
    assert_eq!(
        vcx.read(|cx| *cx.global::<FocusedInput>()),
        FocusedInput::default()
    );
}

#[gpui::test]
fn keys_edit_submit_and_cancel(cx: &mut TestAppContext) {
    let (input, vcx) = focused_input(cx);
    let events = record_input_events(&input, vcx);
    vcx.simulate_input("İzmir");
    vcx.simulate_keystrokes("backspace left left");
    vcx.simulate_input("ı");
    input.read_with(vcx, |i, _| assert_eq!(i.value(), "İzımi"));
    vcx.simulate_keystrokes("enter escape");
    let ev = events.take();
    assert_eq!(ev.iter().filter(|e| **e == InputEvent::Changed).count(), 7);
    assert_eq!(
        ev.iter()
            .filter(|e| **e != InputEvent::Changed)
            .collect::<Vec<_>>(),
        [&InputEvent::Submitted, &InputEvent::Cancelled]
    );
}

#[gpui::test]
fn set_value_replaces_text_and_puts_caret_at_end(cx: &mut TestAppContext) {
    let (input, vcx) = focused_input(cx);
    input.update(vcx, |i, cx| {
        i.set_value("ayar");
        cx.notify();
    });
    vcx.simulate_input("lar");
    input.read_with(vcx, |i, _| assert_eq!(i.value(), "ayarlar"));
}

#[gpui::test]
fn bounds_for_range_come_from_the_shaped_line(cx: &mut TestAppContext) {
    let (input, vcx) = focused_input(cx);
    vcx.simulate_input("ab😀c");
    vcx.run_until_parked();
    let (a, b, all) = input.update_in(vcx, |i, w, cx| {
        let el = gpui::Bounds::default();
        (
            i.bounds_for_range(0..1, el, w, cx).unwrap(),
            i.bounds_for_range(1..2, el, w, cx).unwrap(),
            i.bounds_for_range(0..5, el, w, cx).unwrap(),
        )
    });
    assert!(a.size.width > gpui::px(0.0));
    assert_eq!(b.origin.x, a.origin.x + a.size.width);
    assert!(all.size.width > a.size.width + b.size.width);
    assert_eq!(a.origin.y, all.origin.y);
}

/// A host whose input can be created already focused, or dropped.
struct Slot {
    input: Option<Entity<TextInput>>,
}

impl Render for Slot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().children(self.input.clone())
    }
}

#[gpui::test]
fn input_focused_before_its_first_render_is_published(cx: &mut TestAppContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (slot, vcx) = cx.add_window_view(|_, _| Slot { input: None });
    let input = slot.update_in(vcx, |s, w, cx| {
        let input = cx.new(|cx| TextInput::new(cx, placeholder()));
        input.focus_handle(cx).focus(w, cx);
        s.input = Some(input.clone());
        cx.notify();
        input
    });
    vcx.run_until_parked();
    assert_eq!(
        vcx.read(|cx| *cx.global::<FocusedInput>()),
        FocusedInput {
            entity: Some(input.entity_id()),
            composing: false
        }
    );
}

#[gpui::test]
fn dropping_a_composing_input_clears_focused_input(cx: &mut TestAppContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (slot, vcx) = cx.add_window_view(|_, cx| Slot {
        input: Some(cx.new(|cx| TextInput::new(cx, placeholder()))),
    });
    let input = slot.read_with(vcx, |s, _| s.input.clone().unwrap());
    vcx.update(|w, cx| {
        w.activate_window();
        input.focus_handle(cx).focus(w, cx);
    });
    vcx.run_until_parked();
    input.update_in(vcx, |i, w, cx| {
        i.replace_and_mark_text_in_range(None, "か", None, w, cx)
    });
    assert!(vcx.read(|cx| cx.global::<FocusedInput>().composing));
    let weak = input.downgrade();
    drop(input);
    slot.update(vcx, |s, cx| {
        s.input = None;
        cx.notify();
    });
    vcx.run_until_parked();
    // The frame dropped the last reference; GPUI runs release listeners in the next effect
    // cycle (on a real platform, the one that ends the frame's own update).
    vcx.update(|_, _| {});
    assert!(weak.upgrade().is_none());
    assert_eq!(
        vcx.read(|cx| *cx.global::<FocusedInput>()),
        FocusedInput::default()
    );
}

#[gpui::test]
fn clipboard_chords_copy_cut_and_paste(cx: &mut TestAppContext) {
    let (input, vcx) = focused_input(cx);
    vcx.simulate_input("şehir");
    vcx.simulate_keystrokes("secondary-a secondary-c");
    vcx.simulate_keystrokes("right");
    vcx.simulate_keystrokes("secondary-v");
    input.read_with(vcx, |i, _| assert_eq!(i.value(), "şehirşehir"));
    vcx.simulate_keystrokes("secondary-a secondary-x");
    input.read_with(vcx, |i, _| assert_eq!(i.value(), ""));
    vcx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("a\nb".into())));
    vcx.simulate_keystrokes("secondary-v");
    // A single-line input turns line breaks into spaces.
    input.read_with(vcx, |i, _| assert_eq!(i.value(), "a b"));
}
