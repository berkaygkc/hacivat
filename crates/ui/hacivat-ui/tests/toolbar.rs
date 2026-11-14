//! The Overflow Rule toolbar (Task 21): the pure fit, the measured-equals-painted width, the
//! collapse order at 960 in long locales, and the overflow menu from the keyboard.

mod support;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{TestAppContext, VisualTestContext, px};
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::popover::{a11y_record, a11y_take};
use hacivat_ui::text::{Label, TextRegistry, TextSource};
use hacivat_ui::toolbar::{FitItem, Toolbar, ToolbarItem, ToolbarProbe, fit, stage_fit};
use hacivat_ui_core::gpui_surface::{AnyElement, IntoElement, RunCommand};
use hacivat_ui_core::theme::Theme;
use support::host;

// ---- the pure fit -------------------------------------------------------------------------

#[test]
fn width_math_moves_lowest_priority_first() {
    let got = fit(&[(80.0, 2), (60.0, 1), (90.0, 3)], 200.0);
    assert_eq!(got.shown, vec![0, 2]);
    assert_eq!(got.overflow, vec![1]);
    // Everything fits: nothing moves, and no button is reserved.
    let all = fit(&[(80.0, 2), (60.0, 1), (90.0, 3)], 230.0);
    assert_eq!(all.shown, vec![0, 1, 2]);
    assert!(all.overflow.is_empty());
}

#[test]
fn the_overflow_button_counts_against_the_room() {
    // Dropping the 60 leaves 170, which fits 190 only without the 24 px button (194).
    let got = fit(&[(80.0, 2), (60.0, 1), (90.0, 3)], 190.0);
    assert_eq!(
        got.overflow,
        vec![0, 1],
        "a second item moves when the button does not fit"
    );
}

#[test]
fn ties_move_the_later_item_first_and_pinned_items_stay() {
    let mut items = vec![
        FitItem::new(50.0, 3),
        FitItem::new(50.0, 3),
        FitItem::new(50.0, 3),
    ];
    items[0].movable = false;
    let got = stage_fit(&items, 100.0, 0.0, 24.0);
    assert_eq!(got.fit.overflow, vec![1, 2]);
    assert_eq!(got.fit.shown, vec![0]);
    // Nothing movable is left: the pinned item stays even when it cannot fit.
    let squeezed = stage_fit(&items, 10.0, 0.0, 24.0);
    assert_eq!(squeezed.fit.shown, vec![0]);
}

#[test]
fn a_compact_form_comes_before_the_menu_and_can_add_a_row() {
    let crumb = FitItem::new(100.0, 1);
    let notice = FitItem {
        compact: Some(30.0),
        compact_in_menu: true,
        movable: false,
        ..FitItem::new(200.0, 2)
    };
    let waiting = FitItem {
        compact: Some(120.0),
        movable: false,
        ..FitItem::new(300.0, 3)
    };
    let items = [crumb, notice, waiting];
    let a = stage_fit(&items, 600.0, 0.0, 24.0);
    assert_eq!(a.fit.shown, vec![0, 1, 2]);
    // The crumb leaves first.
    let b = stage_fit(&items, 580.0, 0.0, 24.0);
    assert_eq!(b.fit.overflow, vec![0], "{b:?}");
    assert!(b.compact.is_empty());
    // Then the notice compacts and lists a row.
    let c = stage_fit(&items, 480.0, 0.0, 24.0);
    assert_eq!(c.compact, vec![1], "{c:?}");
    assert_eq!(
        c.menu,
        vec![0, 1],
        "the crumb's row and the notice's, by priority"
    );
    // Then the waiting sentence takes its short form.
    let d = stage_fit(&items, 200.0, 0.0, 24.0);
    assert_eq!(d.compact, vec![1, 2], "{d:?}");
    assert_eq!(d.menu, vec![0, 1]);
}

#[test]
fn gaps_count_between_children() {
    let items = [FitItem::new(50.0, 1), FitItem::new(50.0, 2)];
    assert_eq!(
        stage_fit(&items, 108.0, 8.0, 24.0).fit.overflow,
        Vec::<usize>::new()
    );
    assert_eq!(stage_fit(&items, 107.0, 8.0, 24.0).fit.overflow, vec![0]);
}

// ---- the element --------------------------------------------------------------------------

/// How many times each phrase repeats: English as written, Turkish about twice as wide as the
/// mock's slack allows, the pseudo-locale a third wider again.
#[derive(Clone, Copy, Debug)]
struct Locale {
    name: &'static str,
    stretch: usize,
}

const EN: Locale = Locale {
    name: "en",
    stretch: 1,
};
const TR: Locale = Locale {
    name: "tr",
    stretch: 2,
};
const XT: Locale = Locale {
    name: "tr-XT",
    stretch: 3,
};

fn phrase(word: &str, locale: Locale) -> String {
    vec![word; locale.stretch].join(" ")
}

type Build = Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) -> AnyElement>;

fn text_element(text: String) -> Build {
    Rc::new(move |_, cx| {
        let t = Theme::get(cx).tokens;
        Label::new(
            text.clone(),
            TextSource::Fluent,
            t.type_ramp.body,
            t.colors.text_2.to_hsla(),
        )
        .into_any_element()
    })
}

fn label_item(text: String, priority: u8) -> ToolbarItem {
    let name = text.clone();
    ToolbarItem::new(
        priority,
        text_element(text),
        Rc::new(move |_| name.clone().into()),
    )
}

fn button_item(label: &'static str) -> ToolbarItem {
    ToolbarItem::pinned(
        Rc::new(move |_, _| {
            Button::new(label, label.into())
                .kind(ButtonKind::Primary)
                .into_any_element()
        }),
        Rc::new(move |_| label.into()),
    )
}

const ITEMS: [&str; 6] = [
    "toolbar-item-0",
    "toolbar-item-1",
    "toolbar-item-2",
    "toolbar-item-3",
    "toolbar-item-4",
    "toolbar-item-5",
];
const WHY: &str = "Why this console is read-only";
const WAITING: &str = "Waiting for a free connection to orders-dev (4 of 4 in use)";
const CRUMB: &str = "orders-dev ▸ warehouse_catalog";

/// The M3a editor toolbar's shape (items 0 to 5): Run, run state, crumb, notice (compact: a
/// chip; its menu row is the explanation), waiting sentence (short form), and on the right the
/// rows and timing.
fn editor_bar(locale: Locale, chosen: Rc<Cell<u32>>) -> Toolbar {
    let text = phrase("Production is read-only in this version", locale);
    let name = text.clone();
    let mut notice = ToolbarItem::pinned(text_element(text), Rc::new(move |_| name.clone().into()))
        .priority(2)
        .compact(text_element("read-only".into()))
        .compact_in_menu(true)
        .on_choose(move |_, _| chosen.set(chosen.get() + 1));
    let why = phrase(WHY, locale);
    notice.overflow_label = Rc::new(move |_| why.clone().into());
    let waiting = label_item(phrase(WAITING, locale), 3)
        .compact(text_element("Waiting (4 of 4)".into()))
        .priority(3);
    Toolbar::new("tb")
        .height(32.0)
        .item(button_item("Run"))
        .item(label_item("Running".into(), 200).priority(u8::MAX))
        .item(label_item(phrase(CRUMB, locale), 1))
        .item(notice)
        .item(waiting)
        .trailing(label_item(phrase("10,000 rows", locale), 4))
}

fn mount(cx: &mut TestAppContext, locale: Locale, width: f32) -> &mut VisualTestContext {
    let vcx = host(cx, move |_, _| {
        editor_bar(locale, Rc::new(Cell::new(0))).into_any_element()
    });
    vcx.update(|_, cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
        cx.set_global(ToolbarProbe::default());
    });
    vcx.simulate_resize(gpui::size(px(width), px(600.0)));
    vcx.run_until_parked();
    vcx
}

fn probe(vcx: &mut VisualTestContext) -> ToolbarProbe {
    vcx.read(|cx| cx.global::<ToolbarProbe>().clone())
}

#[gpui::test]
fn the_bar_is_the_height_asked_for_and_nothing_collapses_when_it_fits(cx: &mut TestAppContext) {
    let vcx = mount(cx, EN, 1280.0);
    assert_eq!(
        vcx.debug_bounds("toolbar-tb").unwrap().size.height,
        px(32.0)
    );
    let staged = probe(vcx).staged.expect("a frame was drawn");
    assert_eq!(staged.fit.shown, vec![0, 1, 2, 3, 4, 5]);
    assert!(staged.menu.is_empty());
    assert!(
        vcx.debug_bounds("toolbar-overflow").is_none(),
        "no more-button without a menu row"
    );
}

#[gpui::test]
fn painted_width_is_the_measured_width(cx: &mut TestAppContext) {
    let vcx = mount(cx, EN, 1800.0);
    let p = probe(vcx);
    assert!(p.measured.len() == 6 && p.measured.iter().all(|w| *w > 0.0));
    assert!(
        p.staged.unwrap().menu.is_empty(),
        "everything is shown at 1800"
    );
    for (i, measured) in p.measured.iter().enumerate() {
        let painted = f32::from(vcx.debug_bounds(ITEMS[i]).unwrap().size.width);
        assert!(
            (painted - measured).abs() < 0.01,
            "item {i}: measured {measured}, painted {painted}"
        );
    }
}

#[gpui::test]
fn nothing_truncates_at_960_in_tr_xt(cx: &mut TestAppContext) {
    for locale in [EN, TR, XT] {
        let vcx = mount(cx, locale, 960.0);
        let bar = vcx.debug_bounds("toolbar-tb").unwrap();
        let mut spans = Vec::new();
        for (i, name) in ITEMS.iter().enumerate() {
            if let Some(b) = vcx.debug_bounds(name) {
                assert!(
                    b.right() <= bar.right(),
                    "{}: item {i} runs past the bar",
                    locale.name
                );
                spans.push((b.left(), b.right()));
            }
        }
        if let Some(b) = vcx.debug_bounds("toolbar-overflow") {
            assert!(
                b.right() <= bar.right(),
                "{}: the more-button runs past the bar",
                locale.name
            );
            spans.push((b.left(), b.right()));
        }
        assert!(
            spans.len() >= 3,
            "{}: Run and the run state stay: {spans:?}",
            locale.name
        );
        spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for pair in spans.windows(2) {
            assert!(
                pair[0].1 <= pair[1].0,
                "{}: items overlap: {spans:?}",
                locale.name
            );
        }
        let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
        let bad: Vec<_> = frame.iter().filter(|r| r.clipped || r.truncated).collect();
        assert!(
            bad.is_empty(),
            "{}: clipped or ellipsized text: {bad:?}",
            locale.name
        );
    }
}

#[gpui::test]
fn tr_xt_at_960_collapses_in_the_documented_order(cx: &mut TestAppContext) {
    let vcx = mount(cx, XT, 960.0);
    let staged = probe(vcx).staged.unwrap();
    // The crumb (2) left first; Run (0) and the run state (1) stayed.
    assert!(staged.fit.overflow.contains(&2), "{staged:?}");
    assert!(staged.fit.shown.contains(&0) && staged.fit.shown.contains(&1));
    // The more-button exists because a row is in the menu.
    assert!(vcx.debug_bounds("toolbar-overflow").is_some());
}

#[gpui::test]
fn a_collapsed_segment_keeps_its_text_as_tooltip_and_accessible_name(cx: &mut TestAppContext) {
    a11y_record();
    let vcx = mount(cx, XT, 960.0);
    let staged = probe(vcx).staged.unwrap();
    let declared = a11y_take();
    assert!(
        staged.compact.contains(&3),
        "the notice compacts at 960 in tr-XT: {staged:?}"
    );
    let name = phrase(WHY, XT);
    assert!(
        declared.iter().any(|d| d.name.as_ref() == name),
        "the compact notice lost its accessible name {name:?}"
    );
    // The menu rows of what left the row carry the texts.
    tab_to_more(vcx);
    press(vcx, "enter");
    let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
    for want in [phrase(CRUMB, XT), phrase(WHY, XT)] {
        // On Linux and Windows a menu row is drawn as runs around its access-key letter (P15).
        assert!(
            support::find_label_runs(&frame, &want).is_some(),
            "{want:?} is not in the menu"
        );
    }
}

/// Moves the focus the way Tab does: Run is the first stop, the more-button the second (the
/// keymap's Tab binding belongs to the app, so the tests ask the window).
fn tab_to_more(vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| {
        window.focus_next(cx);
        window.focus_next(cx);
    });
    redraw(vcx);
}

/// Paints once so the element that just took the focus registers its key handlers (the real
/// app draws after every event; the test platform draws when asked).
fn redraw(vcx: &mut VisualTestContext) {
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
}

/// A key press: the test platform's `simulate_keystrokes` sends only the key down, and a
/// focused button turns Enter or Space into a click on the key up.
fn press(vcx: &mut VisualTestContext, key: &'static str) {
    vcx.simulate_keystrokes(key);
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse(key).unwrap(),
    });
    vcx.run_until_parked();
}

fn registry(ran: Rc<Cell<u32>>) -> ActionRegistry {
    let mut reg = ActionRegistry::default();
    reg.register(ActionDef {
        id: "toolbar.test",
        title_id: "hv-cmd-test",
        context: "Workspace",
        run: Rc::new(move |_, _| ran.set(ran.get() + 1)),
        enabled: |_| true,
    })
    .unwrap();
    reg
}

#[gpui::test]
fn overflow_menu_works_from_the_keyboard(cx: &mut TestAppContext) {
    let chosen = Rc::new(Cell::new(0));
    let vcx = {
        let chosen = chosen.clone();
        host(cx, move |_, _| {
            editor_bar(XT, chosen.clone()).into_any_element()
        })
    };
    vcx.update(|_, cx| cx.set_global(ToolbarProbe::default()));
    vcx.simulate_resize(gpui::size(px(960.0), px(600.0)));
    vcx.run_until_parked();
    let staged = probe(vcx).staged.unwrap();
    assert_eq!(
        staged.menu,
        vec![2, 3],
        "the crumb's disabled row, then the notice's"
    );
    // Tab reaches the more-button; Enter opens the menu.
    // Run takes the first Tab; the more-button is the second stop.
    tab_to_more(vcx);
    press(vcx, "enter");
    assert!(vcx.debug_bounds("popup-menu").is_some(), "the menu is open");
    // The crumb's row is disabled, so the highlight starts on the notice's row.
    press(vcx, "enter");
    assert_eq!(chosen.get(), 1, "Enter chose the notice's row");
    assert!(
        vcx.debug_bounds("popup-menu").is_none(),
        "choosing closes the menu"
    );
    // Escape closes it and the focus returns to the button, which Space opens again.
    redraw(vcx);
    press(vcx, "enter");
    assert!(vcx.debug_bounds("popup-menu").is_some());
    vcx.simulate_keystrokes("escape");
    assert!(vcx.debug_bounds("popup-menu").is_none());
    redraw(vcx);
    press(vcx, "space");
    assert!(
        vcx.debug_bounds("popup-menu").is_some(),
        "the focus came back to the more-button"
    );
}

#[gpui::test]
fn a_registered_command_runs_from_its_menu_row(cx: &mut TestAppContext) {
    let ran = Rc::new(Cell::new(0));
    let vcx = host(cx, |_, _| {
        Toolbar::new("cmd")
            .item(button_item("Run"))
            .item(
                label_item(
                    "a label that wants a lot of room in this narrow bar".into(),
                    1,
                )
                .action(RunCommand {
                    id: "toolbar.test".into(),
                }),
            )
            .into_any_element()
    });
    {
        let ran = ran.clone();
        vcx.update(|_, cx| cx.set_global(registry(ran)));
    }
    vcx.simulate_resize(gpui::size(px(160.0), px(600.0)));
    vcx.run_until_parked();
    tab_to_more(vcx);
    press(vcx, "enter");
    press(vcx, "enter");
    assert_eq!(ran.get(), 1);
}

#[gpui::test]
fn a_click_toggles_the_menu_and_a_press_outside_closes_it(cx: &mut TestAppContext) {
    let vcx = mount(cx, XT, 960.0);
    let at = vcx.debug_bounds("toolbar-overflow").unwrap().center();
    vcx.simulate_click(at, gpui::Modifiers::none());
    assert!(
        vcx.debug_bounds("popup-menu").is_some(),
        "the first click opens"
    );
    vcx.simulate_click(at, gpui::Modifiers::none());
    assert!(
        vcx.debug_bounds("popup-menu").is_none(),
        "the second click on the button closes it and does not reopen it"
    );
    vcx.simulate_click(at, gpui::Modifiers::none());
    assert!(vcx.debug_bounds("popup-menu").is_some());
    vcx.simulate_click(gpui::point(px(300.0), px(300.0)), gpui::Modifiers::none());
    assert!(
        vcx.debug_bounds("popup-menu").is_none(),
        "a press elsewhere closes it"
    );
}
