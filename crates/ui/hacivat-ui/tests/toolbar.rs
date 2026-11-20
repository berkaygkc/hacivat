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
use hacivat_ui_core::gpui_surface::{
    AnyElement, IntoElement, ParentElement as _, RunCommand, Styled as _,
};
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

// ## m3a-f2-visual: the wrapping stage (the cursor strip's reason, NOTES surface 11).

/// A strip-like bar: a wrapping reason, then a pinned button.
fn wrapping_bar() -> AnyElement {
    const REASON: &str = "Cursor closed after 100 rows: another statement needed the session. \
                          Hacivat's transaction was rolled back.";
    let style = |cx: &gpui::App| {
        let t = Theme::get(cx).tokens;
        hacivat_ui_core::theme::TypeStyle {
            line_height: 16.0,
            ..t.type_ramp.caption
        }
    };
    let wrapped: Build = Rc::new(move |_, cx| {
        let t = Theme::get(cx).tokens;
        gpui::div()
            .flex_1()
            .min_w(px(0.0))
            .child(
                hacivat_ui::text::paragraph(REASON, TextSource::Fluent, style(cx), {
                    t.colors.text.to_hsla()
                })
                .id("reason"),
            )
            .into_any_element()
    });
    let one_line: Build = Rc::new(move |_, cx| {
        let t = Theme::get(cx).tokens;
        Label::new(
            REASON,
            TextSource::Fluent,
            style(cx),
            t.colors.text.to_hsla(),
        )
        .id("reason")
        .into_any_element()
    });
    gpui::div()
        .w_full()
        .child(
            Toolbar::new("strip")
                .height(26.0)
                .item(
                    ToolbarItem::pinned(one_line, Rc::new(|_| REASON.into())).wraps(
                        hacivat_ui::toolbar::Wrap {
                            text: REASON.into(),
                            style: hacivat_ui_core::theme::TypeStyle {
                                line_height: 16.0,
                                ..hacivat_ui_core::theme::DARK.type_ramp.caption
                            },
                            element: wrapped,
                        },
                    ),
                )
                .item(button_item("Re-run")),
        )
        .into_any_element()
}

/// Where the row fits, the bar keeps its height and the reason its one line; where it does
/// not, the reason wraps in the room the button leaves and the bar grows to hold its lines
/// (padding 4, line height 16), the button whole, nothing clipped.
#[gpui::test]
fn a_wrapping_item_wraps_last_and_the_bar_grows(cx: &mut TestAppContext) {
    let vcx = host(cx, |_, _| wrapping_bar());
    vcx.update(|_, cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
    });
    vcx.simulate_resize(gpui::size(px(1400.0), px(600.0)));
    vcx.run_until_parked();
    let bar = vcx.debug_bounds("toolbar-strip").expect("the bar");
    assert_eq!(bar.size.height, px(26.0));
    for width in [700.0, 520.0, 400.0] {
        vcx.simulate_resize(gpui::size(px(width), px(600.0)));
        vcx.run_until_parked();
        let bar = vcx.debug_bounds("toolbar-strip").expect("the bar");
        let button = vcx.debug_bounds("button-Re-run").expect("the button");
        let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
        let reason = frame
            .iter()
            .find(|r| r.path.ends_with("#reason"))
            .expect("the reason");
        assert!(!reason.clipped && reason.visible, "{width}: {reason:?}");
        // Fix round 1 (M5): inside the row's own edge, the bar's 10 px right padding.
        assert!(
            button.right() <= bar.right() - px(INNER_RIGHT),
            "{width}: {button:?} in {bar:?}"
        );
        let lines = ((f32::from(bar.size.height) - 8.0) / 16.0).round();
        assert!(lines >= 2.0, "{width}: the bar grew to its lines: {bar:?}");
        assert_eq!(
            bar.size.height,
            px(lines * 16.0 + 8.0),
            "{width}: padding 4 and 16 px lines"
        );
    }
}

// ## m3a-f2-visual (fix round 1)

/// The bar's right padding (`spacing.lg`): the row's own edge.
const INNER_RIGHT: f32 = 10.0;

/// A console-like bar: pinned Run and name on the left, a pinned trust chip on the right,
/// rows that wrap when nothing else is left (I2: a narrow pane).
fn trust_bar() -> AnyElement {
    gpui::div()
        .w_full()
        .child(
            Toolbar::new("console")
                .height(32.0)
                .wrap_row()
                .item(button_item("Run"))
                .item(ToolbarItem::pinned(
                    text_element("orders-prod".into()),
                    Rc::new(|_| "orders-prod".into()),
                ))
                .trailing(ToolbarItem::pinned(
                    Rc::new(|_, _| {
                        Button::new("lock", "read-only".into())
                            .kind(ButtonKind::Ghost)
                            .into_any_element()
                    }),
                    Rc::new(|_| "read-only".into()),
                )),
        )
        .into_any_element()
}

/// I2: in a narrow pane a row-wrapping bar puts its items on lines instead of pushing the
/// trailing trust chip past its edge; at a wide one it keeps its height.
#[gpui::test]
fn a_wrapping_row_never_pushes_a_pinned_item_off_the_bar(cx: &mut TestAppContext) {
    let vcx = host(cx, |_, _| trust_bar());
    vcx.simulate_resize(gpui::size(px(800.0), px(600.0)));
    vcx.run_until_parked();
    assert_eq!(
        vcx.debug_bounds("toolbar-console").unwrap().size.height,
        px(32.0)
    );
    let mut grew = false;
    for width in (110..=420).rev().step_by(9) {
        vcx.simulate_resize(gpui::size(px(width as f32), px(600.0)));
        vcx.run_until_parked();
        let bar = vcx.debug_bounds("toolbar-console").expect("the bar");
        for id in ["button-Run", "button-lock"] {
            let b = vcx.debug_bounds(id).expect(id);
            assert!(
                b.left() >= bar.left() && b.right() <= bar.right() - px(INNER_RIGHT),
                "{width}: {id} {b:?} outside {bar:?}"
            );
            assert!(
                b.top() >= bar.top() && b.bottom() <= bar.bottom(),
                "{width}: {id} {b:?} outside {bar:?}"
            );
        }
        grew |= bar.size.height > px(32.0);
    }
    assert!(grew, "the narrowest bars put their items on lines");
}

/// M7: items that left the row with nothing to choose (no command, no action) offer no ⋯
/// menu: a menu of disabled rows would take the focus with nowhere to go.
#[gpui::test]
fn no_more_button_without_a_row_to_choose(cx: &mut TestAppContext) {
    let vcx = host(cx, |_, _| {
        gpui::div()
            .w_full()
            .child(
                Toolbar::new("plain")
                    .item(button_item("Run"))
                    .item(label_item("a crumb that can move".into(), 1)),
            )
            .into_any_element()
    });
    vcx.update(|_, cx| cx.set_global(ToolbarProbe::default()));
    vcx.simulate_resize(gpui::size(px(140.0), px(600.0)));
    vcx.run_until_parked();
    let staged = probe(vcx).staged.expect("a frame");
    assert_eq!(staged.fit.overflow, vec![1], "the crumb left the row");
    assert!(
        vcx.debug_bounds("toolbar-overflow").is_none(),
        "no more-button for a menu with no row to choose"
    );
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

// ## m3c-t41-chrome: `ToolbarItem::narrow` (R-M3-75; design/m3b-safe surface 24) and the ⋯
// reservation (F2 review N3).

/// A narrow pane's bar keeps 8 px of padding on each side (NOTES "Narrow toolbar": 4/8).
const NARROW_RIGHT: f32 = 8.0;

/// The chip's label, long enough that its own form is wider than a 120 px pane.
const GUARDED_LONG: &str = "read-only (guarded) by the profile";

/// The trust chip's sentence: 12 px caption on 15 px lines (NOTES "Narrow toolbar").
fn chip_style() -> hacivat_ui_core::theme::TypeStyle {
    hacivat_ui_core::theme::TypeStyle {
        line_height: 15.0,
        ..hacivat_ui_core::theme::DARK.type_ramp.caption
    }
}

/// A read-only-like chip: a 12 px glyph and its label; the narrow form's label wraps inside the
/// chip (it fills the slot the bar gives it).
fn chip(label: &'static str, narrow: bool) -> Build {
    use hacivat_ui_core::gpui_surface::InteractiveElement as _;
    Rc::new(move |_, cx| {
        let t = Theme::get(cx).tokens;
        let ink = t.colors.text_2.to_hsla();
        let text: AnyElement = if narrow {
            gpui::div()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    hacivat_ui::text::paragraph(label, TextSource::Fluent, chip_style(), ink)
                        .id("chip-label"),
                )
                .into_any_element()
        } else {
            Label::new(label, TextSource::Fluent, chip_style(), ink)
                .id("chip-label")
                .into_any_element()
        };
        let body = gpui::div()
            .debug_selector(move || if narrow { "chip-narrow" } else { "chip-full" }.into())
            .flex()
            .flex_row()
            .items_start()
            .gap(px(4.0))
            .px(px(7.0))
            .py(px(2.0))
            .border_1()
            .child(gpui::div().size(px(12.0)).flex_none().mt(px(1.5)))
            .child(text);
        if narrow {
            body.flex_1().min_w(px(0.0)).into_any_element()
        } else {
            body.into_any_element()
        }
    })
}

fn narrow_bar(label: &'static str, extra: bool) -> AnyElement {
    let run_narrow: Build = Rc::new(|_, _| {
        Button::new("Run-narrow", "Run".into())
            .kind(ButtonKind::Icon)
            .icon(hacivat_ui::icon::IconName::Play)
            .into_any_element()
    });
    let mut bar = Toolbar::new("narrow")
        .height(32.0)
        .wrap_row()
        .item(button_item("Run").narrow(run_narrow))
        .item(ToolbarItem::pinned(
            text_element("orders-prod".into()),
            Rc::new(|_| "orders-prod".into()),
        ));
    if extra {
        // Movable, with nothing to choose: it leaves the row and has no ⋯ row (M7).
        bar = bar.item(label_item("a crumb that can move".into(), 1));
    }
    let bar = bar.trailing(
        ToolbarItem::pinned(chip(label, false), Rc::new(move |_| label.into()))
            .narrow(chip(label, true))
            .narrow_wraps(label.into(), chip_style()),
    );
    gpui::div().w_full().child(bar).into_any_element()
}

/// R-M3-75: a wide bar draws every item's own form; once an item no longer fits whole on a
/// line (a split pane down to its 120 px floor), the items with a narrow form take it, the
/// chip's label wraps inside the chip, the bar grows, and nothing is cut by its edge.
#[gpui::test]
fn a_narrow_form_is_taken_only_below_where_every_item_fits_whole(cx: &mut TestAppContext) {
    let vcx = host(cx, |_, _| narrow_bar(GUARDED_LONG, false));
    vcx.update(|_, cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
        cx.set_global(ToolbarProbe::default());
    });
    vcx.simulate_resize(gpui::size(px(800.0), px(600.0)));
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("chip-full").is_some());
    assert!(vcx.debug_bounds("chip-narrow").is_none());
    assert!(vcx.debug_bounds("button-Run").is_some());
    assert!(!probe(vcx).narrow, "a wide bar is not narrow");
    assert_eq!(
        vcx.debug_bounds("toolbar-narrow").unwrap().size.height,
        px(32.0)
    );
    let mut went_narrow = false;
    for width in (120..=420).rev().step_by(7) {
        vcx.simulate_resize(gpui::size(px(width as f32), px(600.0)));
        vcx.run_until_parked();
        let bar = vcx.debug_bounds("toolbar-narrow").expect("the bar");
        let narrow = probe(vcx).narrow;
        assert_eq!(
            narrow,
            vcx.debug_bounds("chip-narrow").is_some(),
            "{width}: the probe says what is drawn"
        );
        let right = if narrow { NARROW_RIGHT } else { INNER_RIGHT };
        let (chip_id, run_id) = if narrow {
            ("chip-narrow", "button-Run-narrow")
        } else {
            ("chip-full", "button-Run")
        };
        for id in [chip_id, run_id] {
            let b = vcx.debug_bounds(id).expect(id);
            assert!(
                b.left() >= bar.left()
                    && b.right() <= bar.right() - px(right) + px(0.5)
                    && b.top() >= bar.top()
                    && b.bottom() <= bar.bottom(),
                "{width}: {id} {b:?} outside {bar:?}"
            );
        }
        let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
        let label = frame
            .iter()
            .find(|r| r.path.ends_with("#chip-label"))
            .expect("the chip's label");
        assert!(
            !label.clipped && !label.truncated && label.visible && label.painted_px > 0.0,
            "{width}: {label:?}"
        );
        went_narrow |= narrow;
    }
    assert!(went_narrow, "the 120 px pane takes the narrow forms");
    let bar = vcx.debug_bounds("toolbar-narrow").unwrap();
    assert!(bar.size.height > px(32.0), "the narrow bar grew: {bar:?}");
}

/// F2 review N3: rows that cannot be chosen draw no ⋯ button (M7), so the bar reserves no room
/// for one: a row-wrapping bar whose only moved item has nothing to choose opens no extra line.
#[gpui::test]
fn the_menu_button_is_not_reserved_for_rows_that_cannot_be_chosen(cx: &mut TestAppContext) {
    for width in [150.0, 190.0, 240.0] {
        let heights: Vec<gpui::Pixels> = [false, true]
            .into_iter()
            .map(|extra| {
                let vcx = host(cx, move |_, _| narrow_bar("read-only", extra));
                vcx.update(|_, cx| cx.set_global(ToolbarProbe::default()));
                vcx.simulate_resize(gpui::size(px(width), px(600.0)));
                vcx.run_until_parked();
                if extra {
                    let staged = probe(vcx).staged.expect("a frame");
                    assert_eq!(staged.fit.overflow, vec![2], "{width}: the crumb moved");
                    assert!(vcx.debug_bounds("toolbar-overflow").is_none());
                }
                vcx.debug_bounds("toolbar-narrow").unwrap().size.height
            })
            .collect();
        assert_eq!(
            heights[0], heights[1],
            "{width}: no line for an empty ⋯ slot"
        );
    }
}

// ## m4-t33b-carries (T25 carry: the overflow's check marks): an on/off item (a checkbox) that
// moved into the More menu keeps its state there: a menu checkbox, checked while it is on;
// plain rows stay plain menu items, and the highlight does not start on a checked toggle.
#[gpui::test]
fn an_on_off_item_keeps_its_check_in_the_overflow_menu(cx: &mut TestAppContext) {
    let vcx = host(cx, |_, _| {
        let toggle = |label: &'static str, on: bool, priority: u8| {
            label_item(label.into(), priority)
                .checked(on)
                .on_choose(|_, _| {})
        };
        gpui::div()
            .w_full()
            .child(
                Toolbar::new("strip")
                    .item(button_item("Run"))
                    .item(label_item("a plain row".into(), 1).on_choose(|_, _| {}))
                    .item(toggle("Grants", true, 2))
                    .item(toggle("Owner", false, 3)),
            )
            .into_any_element()
    });
    vcx.simulate_resize(gpui::size(px(120.0), px(600.0)));
    vcx.run_until_parked();
    let at = vcx
        .debug_bounds("toolbar-overflow")
        .expect("a More button")
        .center();
    vcx.simulate_click(at, gpui::Modifiers::none());
    assert!(vcx.debug_bounds("popup-menu").is_some());
    a11y_record();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let decls = a11y_take();
    let row = |name: &str| {
        decls
            .iter()
            .find(|d| d.name.as_ref() == name)
            .cloned()
            .unwrap_or_else(|| panic!("no `{name}` row: {decls:?}"))
    };
    use hacivat_ui_core::gpui_surface::Role;
    let grants = row("Grants");
    assert_eq!(grants.role, Some(Role::MenuItemCheckBox));
    assert_eq!(grants.toggled, Some(true));
    let owner = row("Owner");
    assert_eq!(owner.role, Some(Role::MenuItemCheckBox));
    assert_eq!(owner.toggled, Some(false));
    let plain = row("a plain row");
    assert_eq!(plain.role, Some(Role::MenuItem));
    assert_eq!(plain.toggled, None);
    assert!(
        plain.active_descendant && !grants.active_descendant,
        "the highlight starts on the first row, not on the checked toggle"
    );
}
