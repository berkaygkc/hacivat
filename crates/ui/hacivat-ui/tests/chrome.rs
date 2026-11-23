mod support;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{TestAppContext, VisualTestContext, point, px};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::{Capsule, CapsuleModel, EnvKind};
use hacivat_ui::icon::IconName;
use hacivat_ui::panes::{PaneId, TabId, TabSpec, TabStrip};
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented};
use hacivat_ui::statusbar::StatusBar;
use hacivat_ui::text::{RenderedText, TextRegistry, TextSource};
use hacivat_ui::titlebar::{OVERFLOW_BUTTON_PX, TitleBarFit, titlebar_fit};
use hacivat_ui_core::gpui_surface::{IntoElement, ParentElement, Styled, div};
use support::host;

fn tab(id: u32, title: &'static str) -> TabSpec {
    TabSpec {
        id: TabId(id),
        title: title.into(),
        title_source: TextSource::Data,
        dirty: false,
        env: None,
        // ## m4b-t21-shots (named line)
        parts: None,
    }
}

fn draw_chrome_probe(cx: &mut TestAppContext) -> &mut VisualTestContext {
    host(cx, |_, _| {
        div()
            .flex()
            .flex_col()
            .child(
                StatusBar::new()
                    .left(div().child("left"))
                    .right(div().child("right")),
            )
            .child(TabStrip::new(
                PaneId(0),
                vec![tab(1, "orders.sql"), tab(2, "scratch.sql")],
            ))
            .into_any_element()
    })
}

#[test]
fn titlebar_collapses_segments_before_shrinking_palette() {
    assert_eq!(
        titlebar_fit(1440.0, 600.0, 360.0, 180.0),
        TitleBarFit {
            segments_in_overflow: false,
            palette_width: 360.0
        }
    );
    assert_eq!(
        titlebar_fit(1024.0, 600.0, 360.0, 180.0),
        TitleBarFit {
            segments_in_overflow: true,
            palette_width: 360.0
        }
    );
    assert_eq!(
        titlebar_fit(1100.0, 600.0, 360.0, 180.0),
        TitleBarFit {
            segments_in_overflow: true,
            palette_width: 360.0
        }
    );
    assert_eq!(
        titlebar_fit(800.0, 600.0, 360.0, 180.0),
        TitleBarFit {
            segments_in_overflow: true,
            palette_width: 176.0
        }
    );
    assert_eq!(OVERFLOW_BUTTON_PX, 24.0);
}

#[test]
fn titlebar_never_returns_a_negative_palette() {
    assert_eq!(
        titlebar_fit(300.0, 600.0, 360.0, 180.0),
        TitleBarFit {
            segments_in_overflow: true,
            palette_width: 0.0
        }
    );
}

#[gpui::test]
fn status_bar_and_tabs_use_tokens(cx: &mut TestAppContext) {
    let vcx = draw_chrome_probe(cx);
    assert_eq!(vcx.debug_bounds("status-bar").unwrap().size.height, px(24.));
    assert_eq!(vcx.debug_bounds("tab-strip").unwrap().size.height, px(30.));
    assert_eq!(vcx.debug_bounds("tab-1").unwrap().size.height, px(30.));
    assert_eq!(vcx.debug_bounds("tab-2").unwrap().size.height, px(30.));
}

#[gpui::test]
fn buttons_and_segments_have_the_design_heights(cx: &mut TestAppContext) {
    let vcx = host(cx, |_, _| {
        div()
            .flex()
            .flex_col()
            .child(Button::new("save", "Save".into()).kind(ButtonKind::Primary))
            .child(
                Button::new("more", "More".into())
                    .kind(ButtonKind::Icon)
                    .icon(IconName::Ellipsis),
            )
            .child(
                Segmented::new("lang")
                    .item(SegmentItem {
                        content: SegmentContent::Label("EN".into()),
                        pressed: true,
                        tooltip: "English".into(),
                        a11y_name: None,
                        on_click: Rc::new(|_, _, _| {}),
                    })
                    .item(SegmentItem {
                        content: SegmentContent::Icon(IconName::Moon),
                        pressed: false,
                        tooltip: "Dark".into(),
                        a11y_name: None,
                        on_click: Rc::new(|_, _, _| {}),
                    }),
            )
            .into_any_element()
    });
    assert_eq!(
        vcx.debug_bounds("button-save").unwrap().size.height,
        px(24.)
    );
    assert_eq!(
        vcx.debug_bounds("button-more").unwrap().size.height,
        px(24.)
    );
    assert_eq!(
        vcx.debug_bounds("segmented-lang").unwrap().size.height,
        px(26.)
    );
}

// ## m3a-f2-visual: the cursor strip's compact buttons (NOTES "Strip buttons": 20 px, padding
// 0/7, a 12 px label); the default size is unchanged.
#[gpui::test]
fn a_compact_button_is_20_px_with_a_12_px_label(cx: &mut TestAppContext) {
    let vcx = host(cx, |_, _| {
        div()
            .flex()
            .flex_col()
            .items_start()
            .child(Button::new("plain", "Fetch all".into()))
            .child(Button::new("small", "Fetch all".into()).compact(true))
            .into_any_element()
    });
    vcx.update(|_, cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        })
    });
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let plain = vcx.debug_bounds("button-plain").unwrap();
    let small = vcx.debug_bounds("button-small").unwrap();
    assert_eq!(plain.size.height, px(24.));
    assert_eq!(small.size.height, px(20.));
    let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
    let size = |id: &str| {
        frame
            .iter()
            .find(|r| r.path.ends_with(id))
            .map(|r| (r.size_px, r.painted_px))
            .unwrap()
    };
    let (plain_px, plain_w) = size("#button-label-plain");
    let (small_px, small_w) = size("#button-label-small");
    assert_eq!((plain_px, small_px), (12.5, 12.0));
    // 7 px padding instead of 9 on each side, and the smaller label.
    assert!(
        (f32::from(plain.size.width) - f32::from(small.size.width) - 4.0 - (plain_w - small_w))
            .abs()
            < 0.5,
        "{plain:?} {small:?}"
    );
}

#[gpui::test]
fn button_click_fires_and_disabled_button_does_not(cx: &mut TestAppContext) {
    let hits = Rc::new(Cell::new(0));
    let handle = hits.clone();
    let vcx = host(cx, move |_, _| {
        let (h1, h2) = (handle.clone(), handle.clone());
        div()
            .flex()
            .child(Button::new("on", "On".into()).on_click(move |_, _, _| h1.set(h1.get() + 1)))
            .child(
                Button::new("off", "Off".into())
                    .disabled(true)
                    .on_click(move |_, _, _| h2.set(h2.get() + 10)),
            )
            .into_any_element()
    });
    let on = vcx.debug_bounds("button-on").unwrap();
    let off = vcx.debug_bounds("button-off").unwrap();
    vcx.simulate_click(on.center(), gpui::Modifiers::none());
    assert_eq!(hits.get(), 1);
    vcx.simulate_click(off.center(), gpui::Modifiers::none());
    assert_eq!(hits.get(), 1, "a disabled button never runs its handler");
}

// ## m4-t33d (R-T33-1): a Button is a Tab stop by default (HTML parity); `.tab_stop(false)`
// opts out; a disabled button is never a stop; a handle the owner passes keeps its own flag.
#[gpui::test]
fn a_button_is_a_tab_stop_unless_it_opts_out(cx: &mut TestAppContext) {
    use std::cell::RefCell;
    let log = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let plain = cx.update(|cx| cx.focus_handle());
    let sink = log.clone();
    let vcx = host(cx, move |_, _| {
        let button = |id: &'static str| {
            let sink = sink.clone();
            Button::new(id, id.into()).on_click(move |_, _, _| sink.borrow_mut().push(id))
        };
        div()
            .flex()
            .child(button("first"))
            .child(button("out").tab_stop(false))
            .child(button("off").disabled(true))
            .child(button("passed").focus_handle(plain.clone()))
            .child(button("last"))
            .into_any_element()
    });
    vcx.run_until_parked();
    let enter = |vcx: &mut VisualTestContext| {
        vcx.simulate_keystrokes("enter");
        vcx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        vcx.run_until_parked();
    };
    let next = |vcx: &mut VisualTestContext| {
        vcx.update(|window, cx| window.focus_next(cx));
        vcx.run_until_parked();
    };
    next(vcx);
    enter(vcx);
    next(vcx);
    enter(vcx);
    next(vcx);
    enter(vcx);
    assert_eq!(
        *log.borrow(),
        vec!["first", "last", "first"],
        "Tab visits the default buttons only, and wraps"
    );
    vcx.update(|window, cx| window.focus_prev(cx));
    vcx.run_until_parked();
    enter(vcx);
    assert_eq!(log.borrow().last(), Some(&"last"), "Shift-Tab goes back");
}

#[gpui::test]
fn the_tab_strip_reports_activation_and_close(cx: &mut TestAppContext) {
    let log = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let sink = log.clone();
    let vcx = host(cx, move |_, _| {
        let (a, c) = (sink.clone(), sink.clone());
        div()
            .child(
                TabStrip::new(PaneId(7), vec![tab(1, "one.sql"), tab(2, "two.sql")])
                    .active(1)
                    .on_activate(move |pane, tab, _, _| {
                        a.borrow_mut().push(format!("activate {pane:?} {tab:?}"))
                    })
                    .on_close(move |pane, tab, _, _| {
                        c.borrow_mut().push(format!("close {pane:?} {tab:?}"))
                    }),
            )
            .into_any_element()
    });
    let two = vcx.debug_bounds("tab-2").unwrap();
    vcx.simulate_click(
        point(two.left() + px(4.), two.center().y),
        gpui::Modifiers::none(),
    );
    let close = vcx.debug_bounds("tab-close-2").unwrap();
    vcx.simulate_click(close.center(), gpui::Modifiers::none());
    assert_eq!(
        *log.borrow(),
        vec![
            "activate PaneId(7) TabId(2)".to_string(),
            "close PaneId(7) TabId(2)".to_string()
        ]
    );
}

#[gpui::test]
fn capsule_is_26_px_and_only_production_has_the_fill_block(cx: &mut TestAppContext) {
    use hacivat_ui::capsule::{Capsule, CapsuleModel, EnvKind};
    for (env, fill) in [(EnvKind::Production, true), (EnvKind::Staging, false)] {
        let vcx = host(cx, move |_, _| {
            div()
                .flex()
                .child(Capsule::new(CapsuleModel::Connection {
                    env,
                    env_label: "PRODUCTION".into(),
                    name: "orders-prod".into(),
                    engine: "PostgreSQL 16".into(),
                    access: "read-only".into(),
                    path: "Direct".into(),
                    read_only: true,
                    direct: true,
                    latency: None,
                    unencrypted: None,
                    unsupported: None,
                }))
                .into_any_element()
        });
        assert_eq!(vcx.debug_bounds("capsule").unwrap().size.height, px(26.));
        assert_eq!(vcx.debug_bounds("cp-fill").is_some(), fill, "{env:?}");
    }
}

#[gpui::test]
fn title_bar_is_40_px_with_window_controls_on_windows(cx: &mut TestAppContext) {
    use hacivat_ui::titlebar::TitleBar;
    use hacivat_ui_core::window::ChromeKind;
    let vcx = host(cx, |_, _| {
        div()
            .child(
                TitleBar::new(ChromeKind::WindowsCaption)
                    .brand("HACIVAT".into())
                    .palette_field("Search".into(), "Ctrl+K".into(), |_, _, _| {})
                    .app_menu(|_, _, _| {}),
            )
            .into_any_element()
    });
    assert_eq!(vcx.debug_bounds("title-bar").unwrap().size.height, px(40.));
    assert!(vcx.debug_bounds("window-close").is_some());
    assert!(vcx.debug_bounds("palette-field").is_some());
}

fn toast(text: &'static str) -> hacivat_ui::toast::Toast {
    hacivat_ui::toast::Toast::new(
        hacivat_ui::toast::ToastKind::Info,
        Rc::new(move |_: &gpui::App| text.into()),
    )
}

#[gpui::test]
fn toasts_show_three_at_a_time_and_expire_after_six_seconds(cx: &mut TestAppContext) {
    use hacivat_ui::toast::ToastQueue;
    use std::time::Duration;
    cx.update(|cx| {
        for text in ["a", "b", "c", "d"] {
            ToastQueue::push(cx, toast(text));
        }
        assert_eq!(ToastQueue::visible_ids(cx).len(), 3);
        assert_eq!(ToastQueue::waiting(cx), 1);
    });
    cx.executor().advance_clock(Duration::from_millis(5900));
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(ToastQueue::visible_ids(cx).len(), 3));
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            ToastQueue::visible_ids(cx).len(),
            1,
            "the waiting toast took a slot"
        );
        assert_eq!(ToastQueue::waiting(cx), 0);
    });
}

// ## m4-t33d (R-T33-1): a toast's buttons are out of the window's Tab order until the focus is
// in the toast (F6, `focus_newest_sticky`); then Tab walks its actions and its ✕, and goes on
// out of it.
#[gpui::test]
fn a_toast_is_in_the_tab_order_only_once_it_has_the_focus(cx: &mut TestAppContext) {
    use hacivat_ui::toast::{ToastAction, ToastQueue, ToastStack, focus_newest_sticky};
    use std::cell::RefCell;
    let log = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let sink = log.clone();
    cx.update(|cx| {
        let mut t = toast("sticky");
        t.sticky = true;
        t.actions = ["one", "two"]
            .into_iter()
            .map(|name| {
                let sink = sink.clone();
                ToastAction {
                    label: Rc::new(move |_: &gpui::App| name.into()),
                    run: Rc::new(move |_, _| sink.borrow_mut().push(name)),
                }
            })
            .collect();
        ToastQueue::push(cx, t);
    });
    let sink = log.clone();
    let vcx = host(cx, move |_, _| {
        let sink = sink.clone();
        div()
            .size_full()
            .child(
                Button::new("outside", "Outside".into())
                    .on_click(move |_, _, _| sink.borrow_mut().push("outside")),
            )
            .child(ToastStack::new())
            .into_any_element()
    });
    vcx.run_until_parked();
    let enter = |vcx: &mut VisualTestContext| {
        vcx.simulate_keystrokes("enter");
        vcx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        vcx.run_until_parked();
    };
    let next = |vcx: &mut VisualTestContext| {
        vcx.update(|window, cx| window.focus_next(cx));
        vcx.run_until_parked();
    };
    // Out of the toast, Tab only knows the outside button.
    next(vcx);
    next(vcx);
    enter(vcx);
    assert_eq!(
        *log.borrow(),
        vec!["outside"],
        "the toast is not a Tab stop"
    );
    // F6: the first action; Tab, Tab: the second, then the ✕ (Enter dismisses, no action).
    vcx.update(|window, cx| assert!(focus_newest_sticky(window, cx)));
    vcx.run_until_parked();
    next(vcx);
    next(vcx);
    enter(vcx);
    assert_eq!(*log.borrow(), vec!["outside"], "no action ran");
    assert!(
        vcx.update(|_, cx| ToastQueue::visible_ids(cx).is_empty()),
        "Tab, Tab from the first action is the ✕"
    );
}

// ## m4-t33d (R-T33-1): Tab from a toast's first action is its second.
#[gpui::test]
fn tab_moves_between_a_focused_toasts_actions(cx: &mut TestAppContext) {
    use hacivat_ui::toast::{ToastAction, ToastQueue, ToastStack, focus_newest_sticky};
    use std::cell::RefCell;
    let log = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let sink = log.clone();
    cx.update(|cx| {
        let mut t = toast("sticky");
        t.sticky = true;
        t.actions = ["one", "two"]
            .into_iter()
            .map(|name| {
                let sink = sink.clone();
                ToastAction {
                    label: Rc::new(move |_: &gpui::App| name.into()),
                    run: Rc::new(move |_, _| sink.borrow_mut().push(name)),
                }
            })
            .collect();
        ToastQueue::push(cx, t);
    });
    let vcx = host(cx, move |_, _| {
        div()
            .size_full()
            .child(ToastStack::new())
            .into_any_element()
    });
    vcx.run_until_parked();
    vcx.update(|window, cx| assert!(focus_newest_sticky(window, cx)));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.focus_next(cx));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
    });
    vcx.run_until_parked();
    assert_eq!(*log.borrow(), vec!["two"]);
}

#[gpui::test]
fn dismissing_a_toast_promotes_the_waiting_one(cx: &mut TestAppContext) {
    use hacivat_ui::toast::ToastQueue;
    cx.update(|cx| {
        for text in ["a", "b", "c", "d"] {
            ToastQueue::push(cx, toast(text));
        }
        let first = ToastQueue::visible_ids(cx)[0];
        ToastQueue::dismiss(cx, first);
        assert_eq!(ToastQueue::visible_ids(cx).len(), 3);
        assert_eq!(ToastQueue::waiting(cx), 0);
    });
}

#[gpui::test]
fn popup_menu_navigates_skips_disabled_and_chooses(cx: &mut TestAppContext) {
    use hacivat_ui::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};
    use hacivat_ui_core::theme::{Theme, ThemeMode};
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let events = Rc::new(std::cell::RefCell::new(Vec::<PopupMenuEvent>::new()));
    let sink = events.clone();
    let label = |s: &'static str| -> hacivat_ui::text::TextFn { Rc::new(move |_| s.into()) };
    let (host_view, vcx) = cx.add_window_view(|window, cx| {
        let menu = PopupMenu::new(
            point(px(10.), px(10.)),
            vec![
                MenuItemSpec::new(label("Cut"), "edit.cut"),
                MenuItemSpec::new(label("Copy"), "edit.copy").disabled(true),
                MenuItemSpec::separator(),
                MenuItemSpec::new(label("Paste"), "edit.paste"),
            ],
            window,
            cx,
        );
        cx.subscribe(&menu, move |_, _, event: &PopupMenuEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach();
        MenuHost(menu)
    });
    let menu = host_view.read_with(vcx, |h, _| h.0.clone());
    assert_eq!(menu.read_with(vcx, |m, _| m.selected()), Some(0));
    vcx.simulate_keystrokes("down");
    assert_eq!(
        menu.read_with(vcx, |m, _| m.selected()),
        Some(3),
        "skips disabled and separator"
    );
    vcx.simulate_keystrokes("down");
    assert_eq!(menu.read_with(vcx, |m, _| m.selected()), Some(0), "wraps");
    vcx.simulate_keystrokes("up enter");
    assert_eq!(
        events.take(),
        vec![PopupMenuEvent::Chosen("edit.paste".into())]
    );
    vcx.simulate_keystrokes("escape");
    assert_eq!(events.take(), vec![PopupMenuEvent::Dismissed]);
}

// ## m3a-t35-shots: a menu held to a maximum width ellipsizes a row's data hint, never its
// label (the chooser's SQL label, NOTES surface 4).
// ## m3a-t37-gate: with and without access keys. Where they show (the default on Linux and
// Windows) the label is drawn as runs around its key letter, so it is found with
// `find_label_runs` (P15) and every run must be whole; the exact-run lookup failed on Linux CI.
#[gpui::test]
fn a_menu_at_its_max_width_ellipsizes_the_hint_not_the_label(cx: &mut TestAppContext) {
    use hacivat_ui::menu::{MenuItemSpec, PopupMenu};
    use hacivat_ui_core::theme::{Theme, ThemeMode};
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let label = |s: &'static str| -> hacivat_ui::text::TextFn { Rc::new(move |_| s.into()) };
    let long =
        "SELECT o.id, o.total, o.status FROM orders o JOIN customers c ON c.id = o.customer_id";
    let name = "Whole script · 2 statements";
    for access_keys in [false, true] {
        cx.update(|cx| {
            cx.set_global(TextRegistry {
                enabled: true,
                frame: Vec::new(),
            })
        });
        let (_host, vcx) = cx.add_window_view(|window, cx| {
            let menu = PopupMenu::new(
                point(px(10.), px(10.)),
                vec![
                    MenuItemSpec::new(label(name), "run.whole")
                        .hint(label(long))
                        .hint_source(TextSource::Data),
                ],
                window,
                cx,
            );
            menu.update(cx, |m, _| {
                m.set_access_keys(access_keys);
                m.set_min_width(px(300.0));
                m.set_max_width(px(300.0));
            });
            MenuHost(menu)
        });
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
        let hint = frame
            .iter()
            .find(|r| r.text.as_ref() == long)
            .expect("the hint");
        assert!(
            hint.truncated && !hint.clipped,
            "access keys {access_keys}: {hint:?}"
        );
        let runs = support::find_label_runs(&frame, name)
            .unwrap_or_else(|| panic!("access keys {access_keys}: the label is not drawn"));
        if access_keys {
            assert!(runs.len() > 1, "the access key splits the label: {runs:?}");
        }
        for run in &runs {
            assert!(
                !run.truncated && !run.clipped,
                "access keys {access_keys}: {run:?}"
            );
        }
    }
}

// ## m3a-f2-visual: the chooser's anatomy (NOTES surface 4): the active row reads as a
// selection (`blue-tint-strong`), and a 24 px footer on `head` lists the keys.
#[gpui::test]
fn a_menu_can_tint_its_active_row_and_list_its_keys(cx: &mut TestAppContext) {
    use hacivat_ui::menu::{MenuItemSpec, PopupMenu};
    use hacivat_ui_core::theme::{Theme, ThemeMode};
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
    });
    let label = |s: &'static str| -> hacivat_ui::text::TextFn { Rc::new(move |_| s.into()) };
    let (_host, vcx) = cx.add_window_view(|window, cx| {
        let menu = PopupMenu::new(
            point(px(10.), px(10.)),
            vec![
                MenuItemSpec::new(label("Statement"), "run.0").hint(label("SELECT 1")),
                MenuItemSpec::new(label("Whole script"), "run.1").hint(label("SELECT 1; …")),
            ],
            window,
            cx,
        );
        menu.update(cx, |m, _| {
            m.set_min_width(px(300.0));
            m.set_active_tint(true);
            m.set_key_hints(vec![
                ("up down", "hui-palette-hint-navigate"),
                ("enter", "hui-palette-hint-run"),
                ("escape", "hui-palette-hint-close"),
            ]);
        });
        MenuHost(menu)
    });
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let menu = vcx.debug_bounds("popup-menu").expect("the menu");
    let footer = vcx.debug_bounds("popup-menu-footer").expect("the footer");
    assert_eq!(footer.size.height, px(24.0));
    // Edge to edge inside the menu's 1 px border, flush with its bottom.
    assert_eq!(footer.left(), menu.left() + px(1.0));
    assert_eq!(footer.right(), menu.right() - px(1.0));
    assert_eq!(footer.bottom(), menu.bottom() - px(1.0));
    let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
    for id in [
        "hui-palette-hint-navigate",
        "hui-palette-hint-run",
        "hui-palette-hint-close",
    ] {
        let r = frame
            .iter()
            .find(|r| r.text.as_ref() == id)
            .unwrap_or_else(|| panic!("{id}: {frame:?}"));
        assert!(r.visible && !r.clipped, "{r:?}");
    }
    #[cfg(feature = "scene-check")]
    {
        use hacivat_ui_core::theme::DARK;
        let active = vcx.debug_bounds("menu-item-0").expect("the active row");
        let fills: Vec<(gpui::Bounds<gpui::Pixels>, gpui::Hsla)> = vcx.update(|window, _| {
            let s = window.scale_factor();
            window
                .rendered_scene()
                .quads
                .iter()
                .filter_map(|q| {
                    let b = gpui::Bounds::new(
                        point(px(q.bounds.origin.x.0 / s), px(q.bounds.origin.y.0 / s)),
                        gpui::size(
                            px(q.bounds.size.width.0 / s),
                            px(q.bounds.size.height.0 / s),
                        ),
                    );
                    q.background.as_solid().map(|c| (b, c))
                })
                .collect()
        });
        let at_row: Vec<gpui::Hsla> = fills
            .iter()
            .filter(|(b, _)| *b == active)
            .map(|(_, c)| *c)
            .collect();
        assert!(
            at_row.contains(&DARK.colors.blue_tint_strong.to_hsla()),
            "the active row is blue-tint-strong: {at_row:?}"
        );
        assert!(
            !at_row.contains(&DARK.colors.hover.to_hsla()),
            "not the hover fill: {at_row:?}"
        );
        assert!(
            fills
                .iter()
                .any(|(b, c)| *b == footer && *c == DARK.colors.head.to_hsla()),
            "the footer is on head"
        );
    }
}

// ## m3a-f5-demo (follow-up; review m-2, m-3): a menu taller than the room under its anchor
// shows a window of whole rows (separators included) that never reaches the status bar, and
// keeps it whole as the keys and the wheel move it; a short one near the bottom flips above its
// anchor; one that fits the room but neither side of its anchor moves up until it fits.
fn layout_of(vcx: &mut VisualTestContext) -> hacivat_ui::menu::MenuLayout {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    vcx.read(|cx| cx.global::<hacivat_ui::menu::MenuLayout>().clone())
}

/// The item indices of the rows `layout` drew.
fn shown_rows(layout: &hacivat_ui::menu::MenuLayout) -> Vec<usize> {
    layout.rows.iter().map(|(i, _)| *i).collect()
}

fn menu_violations(vcx: &mut VisualTestContext) -> Vec<String> {
    vcx.read(hacivat_ui::menu::MenuLayout::violations)
}

fn registry_on(cx: &mut TestAppContext) {
    use hacivat_ui_core::theme::{Theme, ThemeMode};
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
    });
}

/// `n` items: a separator wherever `sep(i)`, else a row.
fn items(n: usize, sep: impl Fn(usize) -> bool) -> Vec<hacivat_ui::menu::MenuItemSpec> {
    use hacivat_ui::menu::MenuItemSpec;
    (0..n)
        .map(|i| {
            if sep(i) {
                MenuItemSpec::separator()
            } else {
                let text: hacivat_ui::text::TextFn = Rc::new(move |_| format!("Item {i}").into());
                MenuItemSpec::new(text, "menu.item")
            }
        })
        .collect()
}

fn open_menu(
    cx: &mut TestAppContext,
    anchor_y: f32,
    list: Vec<hacivat_ui::menu::MenuItemSpec>,
) -> (gpui::Entity<MenuHost>, &mut VisualTestContext) {
    let (host, vcx) = cx.add_window_view(move |window, cx| {
        MenuHost(hacivat_ui::menu::PopupMenu::new(
            point(px(900.), px(anchor_y)),
            list,
            window,
            cx,
        ))
    });
    vcx.simulate_resize(gpui::size(px(1280.), px(800.)));
    (host, vcx)
}

#[gpui::test]
fn a_long_menu_scrolls_whole_rows_above_the_status_bar(cx: &mut TestAppContext) {
    registry_on(cx);
    let (_host, vcx) = open_menu(cx, 40.0, items(60, |i| i % 6 == 5));
    let layout = layout_of(vcx);
    assert_eq!(layout.window_height, 800.0);
    assert!(
        menu_violations(vcx).is_empty(),
        "{:?}",
        menu_violations(vcx)
    );
    let shown = shown_rows(&layout);
    assert!(
        shown.first() == Some(&0) && shown.len() > 20 && shown.len() < 60,
        "the menu shows a window: rows {shown:?}"
    );
    let column = layout.viewport.expect("the column");
    assert!(f32::from(column.bottom()) <= 800.0 - layout.statusbar_height);
    // End highlights the last row (58; item 59 is a separator) and brings it into view, whole.
    vcx.simulate_keystrokes("end");
    let shown = shown_rows(&layout_of(vcx));
    assert!(shown.contains(&58) && !shown.contains(&0), "{shown:?}");
    assert!(
        menu_violations(vcx).is_empty(),
        "{:?}",
        menu_violations(vcx)
    );
}

// Review m-2: ↓ through a menu whose separators fall unevenly (R R S R R R …): every step
// shows whole rows only, with the highlighted row among them; Home goes back to the top.
#[gpui::test]
fn arrow_keys_keep_whole_rows_across_separators(cx: &mut TestAppContext) {
    registry_on(cx);
    let (host, vcx) = open_menu(cx, 40.0, items(70, |i| i % 6 == 2));
    let menu = host.read_with(vcx, |h, _| h.0.clone());
    let _ = layout_of(vcx);
    for step in 0..60 {
        vcx.simulate_keystrokes("down");
        let layout = layout_of(vcx);
        let selected = menu.read_with(vcx, |m, _| m.selected()).expect("a row");
        assert!(
            shown_rows(&layout).contains(&selected),
            "step {step}: row {selected} not in {:?}",
            shown_rows(&layout)
        );
        let violations = menu_violations(vcx);
        assert!(violations.is_empty(), "step {step}: {violations:?}");
    }
    vcx.simulate_keystrokes("home");
    assert_eq!(shown_rows(&layout_of(vcx)).first(), Some(&0));
}

// Review m-2: the wheel moves the window a row at a time; it stays whole.
#[gpui::test]
fn the_wheel_moves_the_window_by_whole_rows(cx: &mut TestAppContext) {
    registry_on(cx);
    let (_host, vcx) = open_menu(cx, 40.0, items(70, |i| i % 6 == 2));
    let layout = layout_of(vcx);
    let column = layout.viewport.expect("the column");
    vcx.simulate_event(gpui::ScrollEvent {
        position: column.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-3.0 * 24.0))),
        ..Default::default()
    });
    let layout = layout_of(vcx);
    assert_eq!(
        shown_rows(&layout).first(),
        Some(&3),
        "{:?}",
        shown_rows(&layout)
    );
    assert!(
        menu_violations(vcx).is_empty(),
        "{:?}",
        menu_violations(vcx)
    );
}

#[gpui::test]
fn a_short_menu_near_the_bottom_opens_above_its_anchor(cx: &mut TestAppContext) {
    registry_on(cx);
    let (_host, vcx) = open_menu(cx, 770.0, items(3, |_| false));
    let layout = layout_of(vcx);
    assert_eq!(shown_rows(&layout), vec![0, 1, 2]);
    assert!(
        f32::from(layout.rows[2].1.bottom()) <= 770.0,
        "{:?}",
        layout.rows
    );
    assert!(
        menu_violations(vcx).is_empty(),
        "{:?}",
        menu_violations(vcx)
    );
}

// Review m-3: a menu that fits the room but on neither side of its anchor moves up until it
// fits, every row shown (no window).
#[gpui::test]
fn a_menu_that_fits_the_room_moves_up_rather_than_scroll(cx: &mut TestAppContext) {
    registry_on(cx);
    let (_host, vcx) = open_menu(cx, 400.0, items(25, |_| false));
    let layout = layout_of(vcx);
    assert_eq!(shown_rows(&layout), (0..25).collect::<Vec<_>>());
    assert!(
        menu_violations(vcx).is_empty(),
        "{:?}",
        menu_violations(vcx)
    );
}

// ## m3c-shotfix F1: a menu wider than the room right of its anchor moves left until it ends at
// the window's right edge, every row whole; one that fits keeps its anchor's left.
#[gpui::test]
fn a_wide_menu_near_the_right_edge_moves_left_into_the_window(cx: &mut TestAppContext) {
    registry_on(cx);
    let wide: hacivat_ui::text::TextFn =
        Rc::new(|_| "A menu row whose label is far wider than the room right of its anchor".into());
    let list = vec![
        hacivat_ui::menu::MenuItemSpec::new(wide, "menu.wide"),
        items(1, |_| false).remove(0),
    ];
    let (_host, vcx) = cx.add_window_view(move |window, cx| {
        MenuHost(hacivat_ui::menu::PopupMenu::new(
            point(px(1100.), px(100.)),
            list,
            window,
            cx,
        ))
    });
    vcx.simulate_resize(gpui::size(px(1280.), px(800.)));
    let layout = layout_of(vcx);
    let column = layout.viewport.expect("the column");
    assert!(f32::from(column.right()) <= 1280.0, "{column:?}");
    assert!(
        f32::from(column.left()) < 1100.0,
        "it moved left: {column:?}"
    );
    let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
    let clipped: Vec<_> = frame.iter().filter(|r| r.clipped).collect();
    assert!(clipped.is_empty(), "{clipped:?}");
    // A menu that fits keeps its left at the anchor.
    let (_host, vcx) = open_menu(cx, 100.0, items(3, |_| false));
    let column = layout_of(vcx).viewport.expect("the column");
    assert!((f32::from(column.left()) - 900.0).abs() < 8.0, "{column:?}");
}

// ## m3b-ci (controller fix): CI run 44051024805 (Linux, wider fonts) drew the transaction
// menu past the window's right side at 960x600, so its hints were cut there. A menu wider than
// the room right of its anchor moves left and shows whole; one wider than the window is held
// inside it and ellipsizes its hints, never its label; the frame check sees a row past a side.
fn hinted(hint: &'static str) -> Vec<hacivat_ui::menu::MenuItemSpec> {
    use hacivat_ui::menu::MenuItemSpec;
    let text = |s: &'static str| -> hacivat_ui::text::TextFn { Rc::new(move |_| s.into()) };
    vec![
        MenuItemSpec::new(text("Manual"), "tx.manual").hint(text(hint)),
        MenuItemSpec::new(text("Auto-commit"), "tx.auto"),
    ]
}

fn open_menu_at(
    cx: &mut TestAppContext,
    anchor_x: f32,
    window_w: f32,
    list: Vec<hacivat_ui::menu::MenuItemSpec>,
) -> &mut VisualTestContext {
    let (_host, vcx) = cx.add_window_view(move |window, cx| {
        let menu = hacivat_ui::menu::PopupMenu::new(point(px(anchor_x), px(40.)), list, window, cx);
        // Access keys shown on every platform, as Linux and Windows draw them (the label in
        // runs around its key letter), so macOS checks the same drawing.
        menu.update(cx, |m, _| m.set_access_keys(true));
        MenuHost(menu)
    });
    vcx.simulate_resize(gpui::size(px(window_w), px(800.)));
    vcx
}

fn drawn(vcx: &mut VisualTestContext, text: &str) -> RenderedText {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    vcx.read(|cx| cx.global::<TextRegistry>().frame.clone())
        .into_iter()
        .find(|r| r.text.as_ref() == text)
        .unwrap_or_else(|| panic!("{text:?} is not drawn"))
}

#[gpui::test]
fn a_menu_wider_than_the_room_right_of_its_anchor_moves_left(cx: &mut TestAppContext) {
    registry_on(cx);
    let hint = "Changes wait for Commit · Server default · READ COMMITTED";
    // ## m3c (m3b forward merge, controller ruling): also the ruled case, 60 px from the side.
    let vcx = open_menu_at(cx, 1220.0, 1280.0, hinted(hint));
    let column = layout_of(vcx).viewport.expect("the column");
    assert!(f32::from(column.right()) <= 1280.0, "{column:?}");
    assert!(
        menu_violations(vcx).is_empty(),
        "{:?}",
        menu_violations(vcx)
    );
    let vcx = open_menu_at(cx, 1180.0, 1280.0, hinted(hint));
    let layout = layout_of(vcx);
    let column = layout.viewport.expect("the column");
    assert!(
        f32::from(column.size.width) > 100.0,
        "wider than the 100 px right of the anchor: {column:?}"
    );
    assert!(f32::from(column.right()) <= 1280.0, "{column:?}");
    assert!(f32::from(column.left()) < 1180.0, "moved left: {column:?}");
    assert!(
        menu_violations(vcx).is_empty(),
        "{:?}",
        menu_violations(vcx)
    );
    let hint = drawn(vcx, hint);
    assert!(!hint.clipped && !hint.truncated, "shown whole: {hint:?}");
}

#[gpui::test]
fn a_menu_wider_than_the_window_stays_inside_and_ellipsizes_its_hint(cx: &mut TestAppContext) {
    registry_on(cx);
    let hint = "SELECT o.id, o.total, o.status FROM orders o JOIN customers c ON c.id = \
                o.customer_id WHERE o.status = 'open' ORDER BY o.total DESC";
    let vcx = open_menu_at(cx, 20.0, 320.0, hinted(hint));
    let layout = layout_of(vcx);
    let column = layout.viewport.expect("the column");
    assert!(f32::from(column.right()) <= 320.0, "{column:?}");
    assert!(
        menu_violations(vcx).is_empty(),
        "{:?}",
        menu_violations(vcx)
    );
    let shown = drawn(vcx, hint);
    assert!(shown.truncated && !shown.clipped, "{shown:?}");
    // Where access keys show (Linux, Windows) the label is drawn as runs around its key letter
    // (P15): every run must be whole.
    let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
    let runs = support::find_label_runs(&frame, "Manual").expect("the label is drawn");
    for run in &runs {
        assert!(!run.truncated && !run.clipped, "{run:?}");
    }
}

#[gpui::test]
fn the_menu_check_sees_a_row_past_the_windows_side(cx: &mut TestAppContext) {
    use gpui::{Bounds, size};
    let row = |x: f32, w: f32| Bounds::new(point(px(x), px(40.)), size(px(w), px(24.)));
    let layout = hacivat_ui::menu::MenuLayout {
        viewport: None,
        rows: vec![(0, row(700.0, 300.0)), (1, row(-4.0, 100.0))],
        window_height: 600.0,
        statusbar_height: 22.0,
        window_width: 960.0,
    };
    cx.update(|cx| cx.set_global(layout));
    let found = cx.read(hacivat_ui::menu::MenuLayout::violations);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().all(|v| v.contains("past the window's side")));
}

struct MenuHost(gpui::Entity<hacivat_ui::menu::PopupMenu>);

impl gpui::Render for MenuHost {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}

/// What the registry check does under en-XA: Fluent output is bracketed as a whole, so a
/// Fluent-sourced text without brackets is a hardcoded string.
fn unbracketed_fluent(frame: &[RenderedText]) -> Vec<String> {
    frame
        .iter()
        .filter(|r| r.source == TextSource::Fluent)
        .filter(|r| !(r.text.starts_with('[') && r.text.ends_with(']')))
        .map(|r| r.text.to_string())
        .collect()
}

#[gpui::test]
fn pseudo_locale_brackets_fluent_segments_but_not_verbatim_ones_or_the_capsule_name(
    cx: &mut TestAppContext,
) {
    let segment = |content, pressed| SegmentItem {
        content,
        pressed,
        tooltip: "[Language]".into(),
        a11y_name: None,
        on_click: Rc::new(|_, _, _| {}),
    };
    let vcx = host(cx, move |_, _| {
        div()
            .flex()
            .flex_col()
            .child(
                Segmented::new("theme")
                    .item(segment(SegmentContent::Label("[Light]".into()), true))
                    .item(segment(SegmentContent::Label("[Dark]".into()), false)),
            )
            .child(
                Segmented::new("lang")
                    .mono(true)
                    .item(segment(SegmentContent::Verbatim("EN".into()), true))
                    .item(segment(SegmentContent::Verbatim("TR".into()), false)),
            )
            .child(Capsule::new(CapsuleModel::Connection {
                env: EnvKind::Local,
                env_label: "[LOCAL]".into(),
                name: "orders-db".into(),
                engine: "PostgreSQL".into(),
                access: "[Read only]".into(),
                path: "[Direct]".into(),
                read_only: true,
                direct: true,
                latency: None,
                unencrypted: None,
                unsupported: None,
            }))
            .into_any_element()
    });
    vcx.update(|_, cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        })
    });
    vcx.simulate_resize(gpui::size(px(800.0), px(300.0)));
    vcx.run_until_parked();
    let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
    let source_of = |text: &str| {
        frame
            .iter()
            .find(|r| r.text == text)
            .unwrap_or_else(|| panic!("{text} not drawn: {frame:?}"))
            .source
    };
    assert_eq!(source_of("[Light]"), TextSource::Fluent);
    assert_eq!(source_of("[Dark]"), TextSource::Fluent);
    assert_eq!(source_of("EN"), TextSource::Data);
    assert_eq!(source_of("TR"), TextSource::Data);
    assert_eq!(source_of("orders-db"), TextSource::Data);
    assert_eq!(source_of("PostgreSQL"), TextSource::Data);
    assert_eq!(source_of("[Read only]"), TextSource::Fluent);
    // Nothing is flagged: every Fluent text is bracketed, user data and verbatim text is not.
    assert_eq!(unbracketed_fluent(&frame), Vec::<String>::new());
}

// ## m3c-t41-chrome (fix round 1, m5): a menu wider than its minimum, anchored near the
// window's right edge, is placed inside the window by the frame that first draws it: it is
// not drawn past the edge and then placed again.
#[gpui::test]
fn a_wide_menu_near_the_right_edge_fits_on_its_first_frame(cx: &mut TestAppContext) {
    use hacivat_ui::menu::{MenuItemSpec, PopupMenu};
    use hacivat_ui_core::theme::{Theme, ThemeMode};
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let label = |s: &'static str| -> hacivat_ui::text::TextFn { Rc::new(move |_| s.into()) };
    let (host_view, vcx) = cx.add_window_view(|window, cx| {
        let width = window.viewport_size().width;
        let menu = PopupMenu::new(
            point(width - px(60.), px(20.)),
            vec![
                MenuItemSpec::new(label("Auto-commit"), "mode.auto").hint(label(
                    "Each statement commits as it runs, with nothing to commit by hand",
                )),
                MenuItemSpec::new(label("Manual"), "mode.manual")
                    .hint(label("Changes wait for Commit or Roll back")),
            ],
            window,
            cx,
        );
        MenuHost(menu)
    });
    let menu = host_view.read_with(vcx, |h, _| h.0.clone());
    let renders = Rc::new(std::cell::Cell::new(0u32));
    let seen = renders.clone();
    vcx.update(|_, cx| {
        cx.observe(&menu, move |_, _| seen.set(seen.get() + 1))
            .detach()
    });
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let bounds = vcx.debug_bounds("popup-menu").expect("the menu");
    let width = vcx.update(|window, _| window.viewport_size().width);
    assert!(
        bounds.size.width > px(MENU_WIDE_PX),
        "the menu is wider than its minimum: {bounds:?}"
    );
    assert!(
        bounds.left() >= px(0.) && bounds.right() <= width,
        "{bounds:?} past the window's edge ({width:?})"
    );
    assert_eq!(renders.get(), 0, "the menu was placed again after it drew");
}

/// Wider than the menu's 200 px minimum.
const MENU_WIDE_PX: f32 = 240.0;

// ## m3c-shotfix F1 (fix round 1, controller ruling): a menu wider than the window is capped at
// the window less its 4 px margins and snapped inside them; its hint ellipsizes, its label
// shows whole, and the menu's own frame checks find nothing.
#[gpui::test]
fn a_menu_wider_than_the_window_is_capped_and_its_hint_gives_way(cx: &mut TestAppContext) {
    use hacivat_ui::menu::{MenuItemSpec, PopupMenu};
    registry_on(cx);
    let label = |s: &'static str| -> hacivat_ui::text::TextFn { Rc::new(move |_| s.into()) };
    let hint = "A hint far longer than any window could hold, so the menu that carries it would \
                be wider than the whole window if nothing capped it at the window's width, which \
                is exactly what this case needs to show";
    let (_host, vcx) = cx.add_window_view(|window, cx| {
        let menu = PopupMenu::new(
            point(px(440.), px(40.)),
            vec![MenuItemSpec::new(label("Normalize to LF"), "eol.lf").hint(label(hint))],
            window,
            cx,
        );
        MenuHost(menu)
    });
    vcx.simulate_resize(gpui::size(px(500.), px(400.)));
    let layout = layout_of(vcx);
    assert!(
        menu_violations(vcx).is_empty(),
        "{:?}",
        menu_violations(vcx)
    );
    let bounds = vcx.debug_bounds("popup-menu").expect("the menu");
    assert!(
        bounds.left() >= px(4.) - px(0.5) && bounds.right() <= px(496.) + px(0.5),
        "inside the 4 px margins: {bounds:?}"
    );
    assert!(layout.viewport.is_some());
    let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
    let find = |id: &str| {
        frame
            .iter()
            .find(|r| r.path.ends_with(id))
            .unwrap_or_else(|| panic!("{id}: {frame:?}"))
            .clone()
    };
    let label = find("#menu-item-label");
    assert!(!label.truncated && !label.clipped, "{label:?}");
    let hint = find("#menu-item-hint");
    assert!(hint.truncated && !hint.clipped, "{hint:?}");
}

// ## m4b-t21-shots (ruling (d), R-T2-5 d; fix round 2: R-T21-1, width-based)
/// A tab with a two-part title (`short`: its connection's short form).
fn script_tab(id: u32, head: &str, tail: &str, short: Option<&str>) -> TabSpec {
    TabSpec {
        parts: Some(hacivat_ui::panes::TitleParts {
            head: head.to_string().into(),
            tail: tail.to_string().into(),
            tail_short: short.map(|t| t.to_string().into()),
        }),
        ..tab(id, "full title")
    }
}

/// Draws `tabs` (the second active) in a strip `width` px wide; the frame's records once the
/// strip has planned with its own width (the next frame).
fn draw_strip(cx: &mut TestAppContext, width: f32, tabs: Vec<TabSpec>) -> Vec<RenderedText> {
    cx.update(|cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        })
    });
    let vcx = host(cx, move |_, _| {
        div()
            .w(px(width))
            .flex()
            .flex_row()
            .child(TabStrip::new(PaneId(0), tabs.clone()).active(1))
            .into_any_element()
    });
    vcx.run_until_parked();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    vcx.cx.read(|cx| cx.global::<TextRegistry>().frame.clone())
}

const LONG: &str = " · orders-reporting-replica-eu-west-1-read-staging";
const LONG_SHORT: &str = " · orders-reporting-rep…west-1-read-staging";

fn conns(frame: &[RenderedText]) -> Vec<&RenderedText> {
    frame
        .iter()
        .filter(|r| r.path.ends_with("#tab-title-conn"))
        .collect()
}

/// Ruling (d): a script tab's connection never ellipsizes. In a strip too narrow for both
/// tabs, the active tab's file name gives way, its connection stays whole, and the inactive
/// tab collapses: no cut connection is drawn anywhere.
#[gpui::test]
fn a_script_tabs_connection_never_ellipsizes(cx: &mut TestAppContext) {
    let tabs = vec![
        script_tab(1, "orders_report.sql", " · orders-prod", None),
        script_tab(2, "orders_report.sql", LONG, Some(LONG_SHORT)),
    ];
    let frame = draw_strip(cx, 420.0, tabs.clone());
    for r in conns(&frame) {
        assert!(!(r.visible && (r.truncated || r.clipped)), "{r:?}");
    }
    let active = conns(&frame)
        .into_iter()
        .find(|r| r.visible && r.text.contains("orders-reporting"))
        .expect("the active tab's connection");
    assert!(!active.truncated && !active.clipped, "{active:?}");
    // With room, both tabs draw both parts whole.
    let frame = draw_strip(cx, 1200.0, tabs);
    let whole = frame
        .iter()
        .filter(|r| r.path.contains("#tab-title") && r.visible && !r.truncated && !r.clipped)
        .count();
    assert_eq!(whole, 4, "{frame:?}");
}

/// R-T21-1: the 47-character connection is whole while the active tab fits (1280), and shown
/// in its short form (both ends kept) only in a strip where it would overflow.
#[gpui::test]
fn the_connection_is_shortened_only_where_the_tab_would_overflow(cx: &mut TestAppContext) {
    let tabs = vec![
        script_tab(1, "orders_report.sql", " · orders-prod", None),
        script_tab(2, "orders_report.sql", LONG, Some(LONG_SHORT)),
    ];
    let shown = |frame: &[RenderedText]| -> Vec<String> {
        conns(frame)
            .into_iter()
            .filter(|r| r.visible && r.text.contains("orders-reporting"))
            .map(|r| r.text.to_string())
            .collect()
    };
    let wide = draw_strip(cx, 1280.0, tabs.clone());
    assert_eq!(shown(&wide), [LONG], "whole at 1280");
    let narrow = draw_strip(cx, 330.0, tabs);
    assert_eq!(
        shown(&narrow),
        [LONG_SHORT],
        "short where it would overflow"
    );
    let r = conns(&narrow)
        .into_iter()
        .find(|r| r.visible && r.text.contains("orders-reporting"))
        .unwrap();
    assert!(!r.truncated && !r.clipped, "{r:?}");
}

/// R-T21-1: two connections that differ only in the middle read apart on their tabs at 1280.
#[gpui::test]
fn names_that_differ_in_the_middle_read_apart(cx: &mut TestAppContext) {
    let us = " · orders-reporting-replica-us-west-1-read-staging";
    let tabs = vec![
        script_tab(1, "a.sql", LONG, Some(LONG_SHORT)),
        script_tab(
            2,
            "a.sql",
            us,
            Some(" · orders-reporting-rep…west-1-read-staging"),
        ),
    ];
    let frame = draw_strip(cx, 1280.0, tabs);
    let texts: Vec<String> = conns(&frame)
        .into_iter()
        .filter(|r| r.visible && !r.truncated && !r.clipped)
        .map(|r| r.text.to_string())
        .collect();
    assert!(texts.contains(&LONG.to_string()), "{texts:?}");
    assert!(texts.contains(&us.to_string()), "{texts:?}");
}
