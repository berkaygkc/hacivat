//! The palette view (Task 11): keyboard navigation and events; the app does the ranking.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Entity, Focusable as _, SharedString, TestAppContext, VisualTestContext};
use hacivat_ui::input::FocusedInput;
use hacivat_ui::palette::{Palette, PaletteEvent, PaletteItem};
use hacivat_ui::text::{TextRegistry, TextSource};
use hacivat_ui_core::theme::{Theme, ThemeMode};

fn item(id: &str) -> PaletteItem {
    PaletteItem {
        id: SharedString::from(id.to_string()),
        title: SharedString::from(format!("Title {id}")),
        keys: None,
        highlight: Vec::new(),
    }
}

fn palette_with<'a>(
    cx: &'a mut TestAppContext,
    ids: &[&str],
) -> (Entity<Palette>, &'a mut VisualTestContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let items: Vec<_> = ids.iter().map(|id| item(id)).collect();
    let (pal, vcx) = cx.add_window_view(|_, cx| {
        let mut p = Palette::new(cx, Rc::new(|_| "Type a command".into()));
        p.set_items(items);
        p
    });
    vcx.update(|w, cx| {
        w.activate_window();
        pal.focus_handle(cx).focus(w, cx);
    });
    vcx.run_until_parked();
    (pal, vcx)
}

fn record_palette_events(
    pal: &Entity<Palette>,
    vcx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<PaletteEvent>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(pal, move |_, e: &PaletteEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach()
    });
    events
}

#[gpui::test]
fn palette_keyboard_navigation(cx: &mut TestAppContext) {
    let (pal, vcx) = palette_with(cx, &["a", "b", "c"]);
    let events = record_palette_events(&pal, vcx);
    vcx.simulate_keystrokes("down down enter");
    assert_eq!(events.take(), vec![PaletteEvent::Confirmed("c".into())]);
    vcx.simulate_keystrokes("up enter");
    assert_eq!(events.take(), vec![PaletteEvent::Confirmed("b".into())]);
    vcx.simulate_keystrokes("escape");
    assert_eq!(events.take(), vec![PaletteEvent::Dismissed]);
}

#[gpui::test]
fn navigation_stops_at_both_ends(cx: &mut TestAppContext) {
    let (pal, vcx) = palette_with(cx, &["a", "b"]);
    let events = record_palette_events(&pal, vcx);
    vcx.simulate_keystrokes("up enter");
    assert_eq!(events.take(), vec![PaletteEvent::Confirmed("a".into())]);
    vcx.simulate_keystrokes("down down down enter");
    assert_eq!(events.take(), vec![PaletteEvent::Confirmed("b".into())]);
}

#[gpui::test]
fn typing_reports_the_query_and_resets_the_active_row(cx: &mut TestAppContext) {
    let (pal, vcx) = palette_with(cx, &["a", "b", "c"]);
    let events = record_palette_events(&pal, vcx);
    vcx.simulate_keystrokes("down");
    vcx.simulate_input("ay");
    assert_eq!(
        events.take(),
        vec![
            PaletteEvent::QueryChanged("a".into()),
            PaletteEvent::QueryChanged("ay".into())
        ]
    );
    assert_eq!(pal.read_with(vcx, |p, _| p.query()), "ay");
    // The app re-ranks; the first row is active again.
    pal.update(vcx, |p, cx| {
        p.set_items(vec![item("x"), item("y")]);
        cx.notify();
    });
    vcx.simulate_keystrokes("enter");
    assert_eq!(events.take(), vec![PaletteEvent::Confirmed("x".into())]);
}

#[gpui::test]
fn enter_with_no_results_confirms_nothing(cx: &mut TestAppContext) {
    let (pal, vcx) = palette_with(cx, &[]);
    let events = record_palette_events(&pal, vcx);
    vcx.simulate_keystrokes("down enter");
    assert!(events.take().is_empty());
    vcx.update(|_, cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        })
    });
    pal.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    let fluent: Vec<String> = vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .filter(|r| r.source == TextSource::Fluent)
            .map(|r| r.text.to_string())
            .collect()
    });
    // Without a provider `ui_text` renders the ID; the placeholder comes from the TextFn.
    assert!(
        fluent.iter().any(|t| t == "hui-palette-no-results"),
        "{fluent:?}"
    );
    assert!(fluent.iter().any(|t| t == "Type a command"), "{fluent:?}");
}

#[gpui::test]
fn palette_focus_is_its_input(cx: &mut TestAppContext) {
    let (pal, vcx) = palette_with(cx, &["a"]);
    let input = pal.read_with(vcx, |p, _| p.input().clone());
    assert_eq!(
        vcx.read(|cx| cx.global::<FocusedInput>().entity),
        Some(input.entity_id())
    );
}

#[gpui::test]
fn keys_are_not_navigation_while_composing(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler as _;
    let (pal, vcx) = palette_with(cx, &["a", "b"]);
    let events = record_palette_events(&pal, vcx);
    let input = pal.read_with(vcx, |p, _| p.input().clone());
    input.update_in(vcx, |i, w, cx| {
        i.replace_and_mark_text_in_range(None, "か", None, w, cx)
    });
    // While composing, Down and Escape belong to the IME (the test platform has no IME to
    // swallow Enter's text, so Enter is not simulated here).
    vcx.simulate_keystrokes("down escape");
    assert_eq!(events.take(), vec![PaletteEvent::QueryChanged("か".into())]);
    input.update_in(vcx, |i, w, cx| i.replace_text_in_range(None, "感", w, cx));
    vcx.simulate_keystrokes("down enter");
    assert_eq!(
        events.take(),
        vec![
            PaletteEvent::QueryChanged("感".into()),
            PaletteEvent::Confirmed("b".into())
        ]
    );
}
