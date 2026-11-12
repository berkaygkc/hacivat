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
