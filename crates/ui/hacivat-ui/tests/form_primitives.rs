//! The M2a form primitives (NOTES.md "NEW primitives"): dialog sizes, checkbox, select,
//! disclosure, choice list, roving segmented, masked input, paste filter and inline notice.

mod support;

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClipboardItem, Context, Entity, EntityInputHandler,
    Focusable, Font, FontId, FontMetrics, FontRun, GlyphId, InteractiveElement, IntoElement,
    LineLayout, Modifiers, MouseButton, NoopTextSystem, ParentElement, Pixels, PlatformTextSystem,
    Render, RenderGlyphParams, Role, SharedString, Size, Styled, TestAppContext, TestDispatcher,
    TextRenderingMode, VisualTestContext, Window, div, px,
};
use hacivat_ui::button::ButtonKind;
use hacivat_ui::capsule::EnvKind;
use hacivat_ui::checkbox::Checkbox;
use hacivat_ui::choice_list::{Choice, ChoiceEvent, ChoiceList};
use hacivat_ui::dialog::DialogEvent;
use hacivat_ui::dialog::{Dialog, DialogButton, DialogSize, DialogSpec, form_dialog};
use hacivat_ui::disclosure::Disclosure;
use hacivat_ui::input::TextInput;
use hacivat_ui::notice::{InlineNotice, NoticeIcon};
use hacivat_ui::popover::{A11yDecl, a11y_record, a11y_take};
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented};
use hacivat_ui::select::{Select, SelectEvent, SelectOption};
use hacivat_ui::text::{Label, TextFn, TextRegistry, TextSource};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::theme::{DARK, Theme, ThemeMode};
use support::host;

// ---- shared helpers ---------------------------------------------------------------------------

fn text(s: &'static str) -> TextFn {
    Rc::new(move |_: &App| s.into())
}

fn selector(name: impl Into<String>) -> &'static str {
    Box::leak(name.into().into_boxed_str())
}

fn center(vcx: &mut VisualTestContext, name: impl Into<String>) -> gpui::Point<Pixels> {
    let name = name.into();
    vcx.debug_bounds(selector(name.clone()))
        .unwrap_or_else(|| panic!("{name} was not drawn"))
        .center()
}

/// A press and release on the element (a pointer click).
fn press(vcx: &mut VisualTestContext, name: impl Into<String>) {
    let at = center(vcx, name);
    vcx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
}

/// A press without a release: focuses the element under the pointer, clicks nothing.
fn mouse_down(vcx: &mut VisualTestContext, name: impl Into<String>) {
    let at = center(vcx, name);
    vcx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
}

fn enable_registry(vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
        window.refresh();
    });
    vcx.run_until_parked();
}

/// Every string the last frame drew.
fn texts(vcx: &mut VisualTestContext) -> Vec<SharedString> {
    vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .map(|r| r.text.clone())
            .collect()
    })
}

fn has_id(vcx: &mut VisualTestContext, name: &'static str) -> bool {
    vcx.debug_bounds(name).is_some()
}

fn read<V: 'static, R>(vcx: &mut VisualTestContext, view: &Entity<V>, f: impl Fn(&V) -> R) -> R {
    view.read_with(vcx, |v, _| f(v))
}

fn focus_view<V: Focusable>(vcx: &mut VisualTestContext, view: &Entity<V>) {
    let view = view.clone();
    vcx.update(|window, cx| {
        window.activate_window();
        view.focus_handle(cx).focus(window, cx);
    });
    vcx.run_until_parked();
}

/// What a view emitted since it was mounted.
trait Recorded: 'static {
    type Event: Clone;
    fn log() -> &'static std::thread::LocalKey<RefCell<Vec<Self::Event>>>;
}

thread_local! {
    static SELECT_EVENTS: RefCell<Vec<SelectEvent>> = const { RefCell::new(Vec::new()) };
    static CHOICE_EVENTS: RefCell<Vec<ChoiceEvent>> = const { RefCell::new(Vec::new()) };
}

impl Recorded for Select {
    type Event = SelectEvent;
    fn log() -> &'static std::thread::LocalKey<RefCell<Vec<SelectEvent>>> {
        &SELECT_EVENTS
    }
}

impl Recorded for ChoiceList {
    type Event = ChoiceEvent;
    fn log() -> &'static std::thread::LocalKey<RefCell<Vec<ChoiceEvent>>> {
        &CHOICE_EVENTS
    }
}

fn record<V: Recorded + gpui::EventEmitter<V::Event>>(
    vcx: &mut VisualTestContext,
    view: &Entity<V>,
) {
    V::log().with(|l| l.borrow_mut().clear());
    let view = view.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, |_, event: &V::Event, _| {
            let event = event.clone();
            V::log().with(|l| l.borrow_mut().push(event));
        })
        .detach();
    });
}

fn events<V: Recorded>(_: &mut VisualTestContext, _: &Entity<V>) -> Vec<V::Event> {
    V::log().with(|l| l.borrow().clone())
}

// ---- dialog sizes -----------------------------------------------------------------------------

fn form_dialog_fixture() -> impl Fn(&mut Window, &mut App) -> AnyElement + 'static {
    |_, cx| {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let line = |s: &'static str| {
            Label::new(
                s,
                TextSource::Fluent,
                t.type_ramp.body,
                t.colors.text.to_hsla(),
            )
            .into_any_element()
        };
        form_dialog(
            &theme,
            line("New connection"),
            line("Host"),
            None,
            line("Save"),
        )
        .into_any_element()
    }
}

fn bounds_of(
    cx: &mut TestAppContext,
    build: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
) -> Bounds<Pixels> {
    let vcx = host(cx, build);
    vcx.simulate_resize(gpui::size(px(1200.0), px(900.0)));
    vcx.run_until_parked();
    vcx.debug_bounds("form-dialog").expect("form dialog drawn")
}

struct DialogHost {
    dialog: Entity<Dialog>,
}

impl Render for DialogHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(self.dialog.clone())
    }
}

#[gpui::test]
fn dialog_sizes(cx: &mut TestAppContext) {
    assert_eq!(DialogSize::Standard.width_px(), 420.0);
    assert_eq!(DialogSize::Wide.width_px(), 480.0);
    assert_eq!(DialogSize::Form.width_px(), 640.0);
    assert_eq!(bounds_of(cx, form_dialog_fixture()).size.width, px(640.0));

    // The M1 dialog takes its width from the spec's size.
    let spec = DialogSpec {
        title: text("Connection failed"),
        body: text("The server refused the password."),
        buttons: vec![DialogButton {
            label: text("Close"),
            kind: ButtonKind::Default,
            accesskey: None,
            english_accesskey: None,
        }],
        cancel: 0,
        primary: 0,
        size: DialogSize::Wide,
    };
    let (_, vcx) = cx.add_window_view(|window, cx| DialogHost {
        dialog: Dialog::new(spec, window, cx),
    });
    vcx.run_until_parked();
    assert_eq!(vcx.debug_bounds("dialog").unwrap().size.width, px(480.0));
}

#[gpui::test]
fn form_dialog_has_header_body_and_footer_with_seams(cx: &mut TestAppContext) {
    let vcx = host(cx, form_dialog_fixture());
    vcx.simulate_resize(gpui::size(px(1200.0), px(900.0)));
    vcx.run_until_parked();
    let card = vcx.debug_bounds("form-dialog").unwrap();
    let header = vcx.debug_bounds("form-dialog-header").unwrap();
    let body = vcx.debug_bounds("form-dialog-body").unwrap();
    let footer = vcx.debug_bounds("form-dialog-footer").unwrap();
    // Stacked top to bottom, full card width inside the 1 px border.
    assert!(header.bottom() <= body.top() && body.bottom() <= footer.top());
    assert_eq!(header.size.width, card.size.width - px(2.0));
    // Header 12/16 padding plus a 1 px seam around a 17.55 px line; footer 12/16 plus a seam;
    // body padding 12 top, 14 bottom (layout rounds to the device pixel).
    let near = |a: Pixels, b: f32| (f32::from(a) - b).abs() <= 0.5;
    assert!(
        near(header.size.height, 12.0 + 12.0 + 1.0 + 17.55),
        "{header:?}"
    );
    assert!(
        near(footer.size.height, 12.0 + 12.0 + 1.0 + 17.55),
        "{footer:?}"
    );
    assert!(near(body.size.height, 12.0 + 14.0 + 17.55), "{body:?}");
}

// ---- checkbox ---------------------------------------------------------------------------------

type Log = Rc<RefCell<Vec<bool>>>;

fn mount_checkbox(cx: &mut TestAppContext, disabled: bool) -> (&mut VisualTestContext, Log) {
    let log: Log = Rc::default();
    let state = Rc::new(Cell::new(false));
    let sink = log.clone();
    let vcx = host(cx, move |_, _| {
        let (state, sink) = (state.clone(), sink.clone());
        div()
            .p(px(20.0))
            .child(
                Checkbox::new("cb", "Read-only".into())
                    .sub_label("blocks writes".into())
                    .checked(state.get())
                    .disabled(disabled)
                    .on_toggle(move |checked, window, _| {
                        state.set(checked);
                        sink.borrow_mut().push(checked);
                        window.refresh();
                    }),
            )
            .into_any_element()
    });
    vcx.run_until_parked();
    (vcx, log)
}

fn focus(vcx: &mut VisualTestContext, id: &str) {
    mouse_down(vcx, format!("checkbox-{id}"));
}

fn click(vcx: &mut VisualTestContext, id: &str) {
    let at = center(vcx, format!("checkbox-{id}"));
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
}

#[gpui::test]
fn checkbox_toggles_on_space_and_click_but_not_when_disabled(cx: &mut TestAppContext) {
    let (vcx, log) = mount_checkbox(cx, false);
    focus(vcx, "cb");
    vcx.simulate_keystrokes("space");
    // The key-up GPUI would turn into a click is not a second toggle.
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("space").unwrap(),
    });
    click(vcx, "cb");
    assert_eq!(*log.borrow(), vec![true, false]);
    // Enter belongs to the dialog: it never toggles a checkbox.
    vcx.simulate_keystrokes("enter");
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
    });
    assert_eq!(*log.borrow(), vec![true, false]);

    let (vcx, log) = mount_checkbox(cx, true);
    click(vcx, "cb");
    vcx.simulate_keystrokes("space");
    assert!(log.borrow().is_empty());
}

#[gpui::test]
fn checkbox_box_metrics(cx: &mut TestAppContext) {
    let (vcx, _) = mount_checkbox(cx, false);
    let b = vcx.debug_bounds("checkbox-cb-box").unwrap();
    assert_eq!((b.size.width, b.size.height), (px(14.0), px(14.0)));
}

// ---- select -----------------------------------------------------------------------------------

struct SelectHost {
    select: Entity<Select>,
    escapes: Rc<Cell<usize>>,
}

impl Render for SelectHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let escapes = self.escapes.clone();
        // A layer under the select: it closes on Escape (bubble phase, as the workspace does).
        div()
            .size_full()
            .p(px(40.0))
            .on_key_down(move |event: &gpui::KeyDownEvent, _, _| {
                if event.keystroke.key == "escape" {
                    escapes.set(escapes.get() + 1);
                }
            })
            .child(div().w(px(240.0)).child(self.select.clone()))
    }
}

fn mount_select<'a>(
    cx: &'a mut TestAppContext,
    labels: [&'static str; 4],
    selected: usize,
) -> (&'a mut VisualTestContext, Entity<Select>, Rc<Cell<usize>>) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let escapes = Rc::new(Cell::new(0));
    let parent = escapes.clone();
    let (host_view, vcx) = cx.add_window_view(move |_, cx| {
        let options = labels
            .iter()
            .map(|l| SelectOption {
                label: text(l),
                hint: Some(text("hint")),
                disabled: false,
            })
            .collect();
        SelectHost {
            select: cx.new(|cx| Select::new(options, selected, cx)),
            escapes: parent,
        }
    });
    vcx.simulate_resize(gpui::size(px(800.0), px(600.0)));
    vcx.run_until_parked();
    let select = host_view.read_with(vcx, |h, _| h.select.clone());
    record(vcx, &select);
    (vcx, select, escapes)
}

fn focus_select(vcx: &mut VisualTestContext, select: &Entity<Select>) {
    focus_view(vcx, select);
}

fn popup_row_height(vcx: &mut VisualTestContext) -> Pixels {
    vcx.debug_bounds("menu-item-0")
        .expect("menu open")
        .size
        .height
}

fn checked_rows(vcx: &mut VisualTestContext) -> Vec<usize> {
    (0..4)
        .filter(|i| {
            vcx.debug_bounds(selector(format!("menu-item-check-{i}")))
                .is_some()
        })
        .collect()
}

const RETENTION: [&str; 4] = [
    "Never",
    "Until restart",
    "For this session",
    "Forever (Keychain)",
];

#[gpui::test]
fn select_keyboard_and_escape_does_not_propagate(cx: &mut TestAppContext) {
    let (vcx, select, parent_escapes) = mount_select(cx, RETENTION, 3);
    focus_select(vcx, &select);
    vcx.simulate_keystrokes("space");
    assert!(read(vcx, &select, Select::is_open));
    vcx.simulate_keystrokes("up up enter");
    assert_eq!(read(vcx, &select, Select::selected), 1);
    assert!(!read(vcx, &select, Select::is_open));
    assert_eq!(events(vcx, &select), [SelectEvent::Changed(1)]);
    vcx.simulate_keystrokes("down");
    assert!(read(vcx, &select, Select::is_open));
    vcx.simulate_keystrokes("escape");
    assert!(!read(vcx, &select, Select::is_open));
    assert_eq!(read(vcx, &select, Select::selected), 1);
    assert_eq!(parent_escapes.get(), 0);
    // Focus came back to the select; with it closed, Escape reaches the layer again.
    vcx.simulate_keystrokes("escape");
    assert_eq!(parent_escapes.get(), 1);
    // Picking the current option again emits nothing.
    vcx.simulate_keystrokes("enter enter");
    assert_eq!(events(vcx, &select), [SelectEvent::Changed(1)]);
}

#[gpui::test]
fn select_rows_reuse_the_popup_menu(cx: &mut TestAppContext) {
    let (vcx, select, _) = mount_select(cx, RETENTION, 3);
    focus_select(vcx, &select);
    vcx.simulate_keystrokes("space");
    assert_eq!(popup_row_height(vcx), px(24.0));
    assert_eq!(checked_rows(vcx), [3]);
    // The menu opens below the field, at least as wide as the field and never under 200 px.
    let field = vcx.debug_bounds("select-field").unwrap();
    let menu = vcx.debug_bounds("popup-menu").unwrap();
    assert!(menu.top() >= field.bottom(), "{menu:?} vs {field:?}");
    assert!(menu.size.width >= field.size.width.max(px(200.0)));
}

#[gpui::test]
fn select_opens_and_closes_by_pointer(cx: &mut TestAppContext) {
    let (vcx, select, _) = mount_select(cx, RETENTION, 0);
    let field = center(vcx, "select-field");
    vcx.simulate_click(field, Modifiers::none());
    vcx.run_until_parked();
    assert!(read(vcx, &select, Select::is_open));
    // A click on the field while open closes it (and does not reopen it).
    vcx.simulate_click(field, Modifiers::none());
    vcx.run_until_parked();
    assert!(!read(vcx, &select, Select::is_open));
    vcx.simulate_click(field, Modifiers::none());
    vcx.run_until_parked();
    let row = center(vcx, "menu-item-2");
    vcx.simulate_click(row, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(read(vcx, &select, Select::selected), 2);
    assert_eq!(events(vcx, &select), [SelectEvent::Changed(2)]);
}

// ---- disclosure -------------------------------------------------------------------------------

const TLS_SUMMARY: &str = "Auto: verify when the server offers TLS";

fn mount_disclosure(cx: &mut TestAppContext, open: bool) -> &mut VisualTestContext {
    let state = Rc::new(Cell::new(open));
    let vcx = host(cx, move |_, cx| {
        let t = Theme::get(cx).tokens;
        let state = state.clone();
        div()
            .w(px(600.0))
            .child(
                Disclosure::new("tls", "TLS".into())
                    .summary(TLS_SUMMARY.into())
                    .open(state.get())
                    .on_toggle(move |open, window, _| {
                        state.set(open);
                        window.refresh();
                    })
                    .content(
                        Label::new(
                            "Mode",
                            TextSource::Fluent,
                            t.type_ramp.body,
                            t.colors.text_2.to_hsla(),
                        )
                        .into_any_element(),
                    ),
            )
            .into_any_element()
    });
    enable_registry(vcx);
    // Focus the row with a press (no release: no click).
    mouse_down(vcx, "disclosure-tls");
    vcx
}

#[gpui::test]
fn disclosure_shows_summary_only_when_closed(cx: &mut TestAppContext) {
    let vcx = mount_disclosure(cx, false);
    assert!(texts(vcx).contains(&TLS_SUMMARY.into()));
    assert!(!has_id(vcx, "disclosure-content"));
    assert_eq!(
        vcx.debug_bounds("disclosure-tls").unwrap().size.height,
        px(32.0)
    );
    vcx.simulate_keystrokes("space");
    assert!(!texts(vcx).contains(&TLS_SUMMARY.into()));
    assert!(has_id(vcx, "disclosure-content"));
    // The content is indented 22 px from the row.
    let row = vcx.debug_bounds("disclosure-tls").unwrap();
    let content = vcx.debug_bounds("disclosure-content").unwrap();
    assert_eq!(content.left() - row.left(), px(22.0));
}

// ---- choice list ------------------------------------------------------------------------------

struct ChoiceHost {
    list: Entity<ChoiceList>,
}

impl Render for ChoiceHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(440.0)).child(self.list.clone())
    }
}

fn mount_choices(
    cx: &mut TestAppContext,
    selected: Option<usize>,
) -> (&mut VisualTestContext, Entity<ChoiceList>) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (host_view, vcx) = cx.add_window_view(move |_, cx| {
        let choice = |mark, label: &'static str, description: &'static str| Choice {
            mark: Some(mark),
            label: text(label),
            description: text(description),
        };
        let choices = vec![
            choice(EnvKind::Production, "Production", "Live data"),
            choice(EnvKind::Staging, "Staging", "Shared pre-release"),
            choice(EnvKind::Test, "Test", "Disposable data"),
            choice(EnvKind::Dev, "Development", "Your own work"),
        ];
        ChoiceHost {
            list: cx.new(|cx| ChoiceList::new(choices, selected, cx)),
        }
    });
    vcx.run_until_parked();
    let list = host_view.read_with(vcx, |h, _| h.list.clone());
    record(vcx, &list);
    (vcx, list)
}

#[gpui::test]
fn choice_list_arrows_move_and_select(cx: &mut TestAppContext) {
    let (vcx, list) = mount_choices(cx, None);
    focus_view(vcx, &list);
    vcx.simulate_keystrokes("down down");
    assert_eq!(read(vcx, &list, ChoiceList::selected), Some(1));
    assert_eq!(events(vcx, &list).last(), Some(&ChoiceEvent::Selected(1)));
    vcx.simulate_keystrokes("up up");
    assert_eq!(read(vcx, &list, ChoiceList::selected), Some(3));
    // Rows are at least 34 px; a click selects.
    assert!(vcx.debug_bounds("choice-2").unwrap().size.height >= px(34.0));
    press(vcx, "choice-2");
    assert_eq!(read(vcx, &list, ChoiceList::selected), Some(2));
    assert_eq!(events(vcx, &list).last(), Some(&ChoiceEvent::Selected(2)));
}

// ---- segmented --------------------------------------------------------------------------------

fn mount_segmented(
    cx: &mut TestAppContext,
    count: usize,
    pressed: usize,
) -> (&mut VisualTestContext, Rc<Cell<usize>>) {
    let state = Rc::new(Cell::new(pressed));
    let shared = state.clone();
    let vcx = host(cx, move |_, _| {
        let mut group = Segmented::new("seg");
        for i in 0..count {
            let state = shared.clone();
            group = group.item(SegmentItem {
                content: SegmentContent::Label(format!("S{i}").into()),
                pressed: shared.get() == i,
                tooltip: "tip".into(),
                a11y_name: None,
                on_click: Rc::new(move |_, window, _| {
                    state.set(i);
                    window.refresh();
                }),
            });
        }
        div().p(px(20.0)).child(group).into_any_element()
    });
    vcx.run_until_parked();
    (vcx, state)
}

fn focus_seg(vcx: &mut VisualTestContext, index: usize) {
    mouse_down(vcx, format!("segmented-seg-{index}"));
}

/// The distinct focus stops Tab visits in the window (the host holds only the group).
fn tab_stops(vcx: &mut VisualTestContext, _group: &str) -> usize {
    vcx.update(|window, cx| {
        window.blur(cx);
        let mut seen = Vec::new();
        for _ in 0..8 {
            window.focus_next(cx);
            if let Some(handle) = window.focused(cx)
                && !seen.contains(&handle)
            {
                seen.push(handle);
            }
        }
        seen.len()
    })
}

#[gpui::test]
fn segmented_roving_arrows(cx: &mut TestAppContext) {
    let (vcx, pressed) = mount_segmented(cx, 4, 0);
    focus_seg(vcx, 0);
    vcx.simulate_keystrokes("right right left");
    assert_eq!(pressed.get(), 1);
    assert_eq!(tab_stops(vcx, "seg"), 1);
    // Wraps at the ends.
    focus_seg(vcx, 0);
    vcx.simulate_keystrokes("left");
    assert_eq!(pressed.get(), 3);
}

// ---- masked input and paste filter ------------------------------------------------------------

struct InputHost {
    input: Entity<TextInput>,
}

impl Render for InputHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(300.0)).child(self.input.clone())
    }
}

fn mount_input(
    cx: &mut TestAppContext,
    masked: bool,
) -> (&mut VisualTestContext, Entity<TextInput>) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (host_view, vcx) = cx.add_window_view(|_, cx| InputHost {
        input: cx.new(|cx| {
            let mut input = TextInput::new(cx, text("Password"));
            input.set_masked(masked);
            input
        }),
    });
    let input = host_view.read_with(vcx, |h, _| h.input.clone());
    focus_view(vcx, &input);
    (vcx, input)
}

fn type_text(vcx: &mut VisualTestContext, s: &str) {
    vcx.simulate_input(s);
    vcx.run_until_parked();
}

fn primary(key: &str) -> String {
    format!("secondary-{key}")
}

fn painted_text(vcx: &mut VisualTestContext, input: &Entity<TextInput>) -> String {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    read(vcx, input, |i| i.painted_text().to_string())
}

fn clipboard_text(vcx: &mut VisualTestContext) -> Option<String> {
    vcx.update(|_, cx| cx.read_from_clipboard())
        .and_then(|item| item.text())
}

/// Every accessibility declaration of the next frame.
fn a11y_frame(vcx: &mut VisualTestContext) -> Vec<A11yDecl> {
    a11y_record();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    a11y_take()
}

fn a11y_input(vcx: &mut VisualTestContext) -> A11yDecl {
    a11y_frame(vcx)
        .into_iter()
        .find(|d| matches!(d.role, Some(Role::TextInput | Role::PasswordInput)))
        .expect("the input declared itself")
}

fn a11y_value(vcx: &mut VisualTestContext, _input: &Entity<TextInput>) -> Option<String> {
    a11y_input(vcx).value.map(|v| v.to_string())
}

#[gpui::test]
fn masked_input_hides_value_everywhere(cx: &mut TestAppContext) {
    let (vcx, input) = mount_input(cx, true);
    type_text(vcx, "s3cret");
    assert_eq!(read(vcx, &input, |i| i.value().to_string()), "s3cret");
    assert_eq!(painted_text(vcx, &input), "••••••");
    vcx.simulate_keystrokes(&primary("a"));
    vcx.simulate_keystrokes(&primary("c"));
    assert_eq!(clipboard_text(vcx), None);
    vcx.simulate_keystrokes(&primary("x"));
    assert_eq!(clipboard_text(vcx), None);
    assert_eq!(read(vcx, &input, |i| i.value().to_string()), "s3cret");
    assert_eq!(a11y_value(vcx, &input), Some(String::new()));

    // One bullet per grapheme, not per byte or char.
    let (vcx, input) = mount_input(cx, true);
    type_text(vcx, "ğe\u{301}");
    assert_eq!(painted_text(vcx, &input), "••");

    // Unmasked, the value is painted and exposed as is.
    let (vcx, input) = mount_input(cx, false);
    type_text(vcx, "orders");
    assert_eq!(painted_text(vcx, &input), "orders");
    assert_eq!(a11y_value(vcx, &input), Some("orders".to_string()));
}

type Seen = Rc<RefCell<Vec<String>>>;

fn mount_input_with_filter(
    cx: &mut TestAppContext,
    masked: bool,
    filter: impl Fn(&str) -> bool + 'static,
) -> (&mut VisualTestContext, Entity<TextInput>, Seen) {
    let seen: Seen = Rc::default();
    let sink = seen.clone();
    let filter = Rc::new(filter);
    let (vcx, input) = mount_input(cx, masked);
    input.update(vcx, |input, _| {
        input.set_paste_filter(Some(Rc::new(
            move |text: &str, _: &mut Window, _: &mut App| {
                sink.borrow_mut().push(text.to_string());
                filter(text)
            },
        )));
    });
    (vcx, input, seen)
}

fn paste(vcx: &mut VisualTestContext, input: &Entity<TextInput>, text: &str) {
    let item = ClipboardItem::new_string(text.to_string());
    vcx.update(|_, cx| cx.write_to_clipboard(item));
    focus_view(vcx, input);
    vcx.simulate_keystrokes(&primary("v"));
}

#[gpui::test]
fn paste_filter_consumes_and_masked_never_filters(cx: &mut TestAppContext) {
    let (vcx, input, seen) =
        mount_input_with_filter(cx, false, |text| text.starts_with("postgres://"));
    paste(vcx, &input, "postgres://h/d");
    assert_eq!(read(vcx, &input, |i| i.value().to_string()), "");
    paste(vcx, &input, "hello");
    assert_eq!(read(vcx, &input, |i| i.value().to_string()), "hello");
    assert_eq!(*seen.borrow(), ["postgres://h/d", "hello"]);

    let (vcx, input, seen) = mount_input_with_filter(cx, true, |_| true);
    paste(vcx, &input, "postgres://x@y/z");
    assert_eq!(
        read(vcx, &input, |i| i.value().to_string()),
        "postgres://x@y/z"
    );
    assert!(seen.borrow().is_empty());
}

#[gpui::test]
fn escape_in_a_field_still_cancels(cx: &mut TestAppContext) {
    let (vcx, input) = mount_input(cx, true);
    let log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&input, move |_, e: &hacivat_ui::input::InputEvent, _| {
            sink.borrow_mut().push(*e)
        })
        .detach()
    });
    vcx.simulate_keystrokes("escape");
    assert!(
        log.borrow()
            .contains(&hacivat_ui::input::InputEvent::Cancelled)
    );
}

// ---- inline notice ----------------------------------------------------------------------------

/// A platform text system that records the font family every shaped line was laid out in. GPUI's
/// test platform shapes every font alike, so the family is read from the shaping request.
#[derive(Default)]
struct FontRecorder {
    families: Mutex<HashMap<String, usize>>,
    lines: Mutex<Vec<(String, String)>>,
}

struct RecordingTextSystem {
    inner: NoopTextSystem,
    rec: Arc<FontRecorder>,
}

impl PlatformTextSystem for RecordingTextSystem {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> gpui::Result<()> {
        self.inner.add_fonts(fonts)
    }
    fn all_font_names(&self) -> Vec<String> {
        self.inner.all_font_names()
    }
    fn font_id(&self, descriptor: &Font) -> gpui::Result<FontId> {
        let mut families = self.rec.families.lock().unwrap();
        let next = families.len() + 1;
        Ok(FontId(
            *families
                .entry(descriptor.family.to_string())
                .or_insert(next),
        ))
    }
    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        self.inner.font_metrics(font_id)
    }
    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> gpui::Result<Bounds<f32>> {
        self.inner.typographic_bounds(font_id, glyph_id)
    }
    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> gpui::Result<Size<f32>> {
        self.inner.advance(font_id, glyph_id)
    }
    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        self.inner.glyph_for_char(font_id, ch)
    }
    fn glyph_raster_bounds(
        &self,
        params: &RenderGlyphParams,
    ) -> gpui::Result<Bounds<gpui::DevicePixels>> {
        self.inner.glyph_raster_bounds(params)
    }
    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        raster_bounds: Bounds<gpui::DevicePixels>,
    ) -> gpui::Result<(Size<gpui::DevicePixels>, Vec<u8>)> {
        self.inner.rasterize_glyph(params, raster_bounds)
    }
    fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
        if let Some(run) = runs.first() {
            let families = self.rec.families.lock().unwrap();
            let family = families
                .iter()
                .find(|(_, id)| **id == run.font_id.0)
                .map(|(f, _)| f.clone())
                .unwrap_or_default();
            self.rec
                .lines
                .lock()
                .unwrap()
                .push((text.to_string(), family));
        }
        self.inner.layout_line(text, font_size, runs)
    }
    fn recommended_rendering_mode(&self, font_id: FontId, font_size: Pixels) -> TextRenderingMode {
        self.inner.recommended_rendering_mode(font_id, font_size)
    }
}

struct NoticeProbe {
    _cx: TestAppContext,
    rec: Arc<FontRecorder>,
}

struct LabelStyle {
    font_family: String,
}

fn mount_notice(_: &mut TestAppContext) -> NoticeProbe {
    let rec = Arc::new(FontRecorder::default());
    let mut cx = TestAppContext::build_with_text_system(
        TestDispatcher::new(0),
        None,
        Arc::new(RecordingTextSystem {
            inner: NoopTextSystem,
            rec: rec.clone(),
        }),
    );
    let vcx = host(&mut cx, |_, _| {
        div()
            .w(px(560.0))
            .child(
                InlineNotice::new(NoticeIcon::Done)
                    .line("Filled from the URL".into())
                    .secondary("Nothing is saved until you save.".into())
                    .param(
                        "sslmode=verify-full".into(),
                        "set TLS to Verify full".into(),
                    ),
            )
            .into_any_element()
    });
    vcx.run_until_parked();
    NoticeProbe { _cx: cx, rec }
}

fn label_style(probe: &NoticeProbe, text: &str) -> LabelStyle {
    let lines = probe.rec.lines.lock().unwrap();
    let (_, family) = lines
        .iter()
        .find(|(t, _)| t == text)
        .unwrap_or_else(|| panic!("{text} was not shaped: {lines:?}"));
    LabelStyle {
        font_family: family.clone(),
    }
}

fn mono_family() -> String {
    font_for(&DARK.type_ramp.mono_data).family.to_string()
}

#[gpui::test]
fn notice_params_use_mono(cx: &mut TestAppContext) {
    let probe = mount_notice(cx);
    assert_eq!(
        label_style(&probe, "sslmode=verify-full").font_family,
        mono_family()
    );
    // Prose stays in the sans family.
    assert_ne!(
        label_style(&probe, "set TLS to Verify full").font_family,
        mono_family()
    );
    assert_ne!(
        label_style(&probe, "Filled from the URL").font_family,
        mono_family()
    );
}

// ---- fix round 1 ------------------------------------------------------------------------------

fn release_key(vcx: &mut VisualTestContext, key: &'static str) {
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse(key).unwrap(),
    });
    vcx.run_until_parked();
}

/// Enter down and up, as a keyboard sends it.
fn press_enter(vcx: &mut VisualTestContext) {
    vcx.simulate_keystrokes("enter");
    release_key(vcx, "enter");
}

struct NestedHost {
    dialog: Entity<Dialog>,
    select: Entity<Select>,
}

impl Render for NestedHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.dialog.clone())
    }
}

#[gpui::test]
fn a_select_inside_a_dialog_keeps_escape_and_enter_to_itself(cx: &mut TestAppContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (host_view, vcx) = cx.add_window_view(|window, cx| {
        let spec = DialogSpec {
            title: text("Remember the password?"),
            body: text("Choose how long."),
            buttons: ["Cancel", "Save"]
                .into_iter()
                .map(|l| DialogButton {
                    label: text(l),
                    kind: ButtonKind::Default,
                    accesskey: None,
                    english_accesskey: None,
                })
                .collect(),
            cancel: 0,
            primary: 1,
            size: DialogSize::Wide,
        };
        let dialog = Dialog::new(spec, window, cx);
        let options = RETENTION
            .iter()
            .map(|l| SelectOption {
                label: text(l),
                hint: None,
                disabled: false,
            })
            .collect();
        let select = cx.new(|cx| Select::new(options, 3, cx));
        dialog.update(cx, |d, _| d.set_content(select.clone().into()));
        NestedHost { dialog, select }
    });
    vcx.simulate_resize(gpui::size(px(900.0), px(700.0)));
    vcx.run_until_parked();
    let (dialog, select) = host_view.read_with(vcx, |h, _| (h.dialog.clone(), h.select.clone()));
    let pressed: Rc<RefCell<Vec<DialogEvent>>> = Rc::default();
    let sink = pressed.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&dialog, move |_, e: &DialogEvent, _| {
            sink.borrow_mut().push(*e)
        })
        .detach()
    });
    record(vcx, &select);

    focus_select(vcx, &select);
    // Enter on the closed select opens it; it is not the dialog's default button.
    press_enter(vcx);
    assert!(read(vcx, &select, Select::is_open));
    assert_eq!(*pressed.borrow(), []);
    // Escape closes only the menu.
    vcx.simulate_keystrokes("escape");
    assert!(!read(vcx, &select, Select::is_open));
    assert_eq!(*pressed.borrow(), []);
    // Enter in the open menu picks a row and presses no dialog button.
    vcx.simulate_keystrokes("space up up");
    press_enter(vcx);
    assert_eq!(read(vcx, &select, Select::selected), 1);
    assert!(!read(vcx, &select, Select::is_open));
    assert_eq!(*pressed.borrow(), []);
    assert_eq!(events(vcx, &select), [SelectEvent::Changed(1)]);
    // With the menu closed, Escape belongs to the dialog again.
    vcx.simulate_keystrokes("escape");
    assert_eq!(*pressed.borrow(), [DialogEvent::Cancelled]);
}

#[gpui::test]
fn masked_input_gives_text_services_only_bullets(cx: &mut TestAppContext) {
    let (vcx, input) = mount_input(cx, true);
    type_text(vcx, "s3cret");
    let mut adjusted = None;
    let services = input
        .update_in(vcx, |i, window, cx| {
            i.text_for_range(0..6, &mut adjusted, window, cx)
        })
        .unwrap();
    assert!(!services.contains("s3cret") && !services.contains('s'));
    assert_eq!(services, "••••••");
    // One bullet per UTF-16 unit, so the IME's range arithmetic holds.
    let (vcx, input) = mount_input(cx, true);
    type_text(vcx, "a😀");
    let services = input
        .update_in(vcx, |i, window, cx| {
            i.text_for_range(0..3, &mut None, window, cx)
        })
        .unwrap();
    assert_eq!(services.chars().count(), 3);
    assert!(services.chars().all(|c| c == '•'));
    assert_eq!(a11y_input(vcx).role, Some(Role::PasswordInput));

    let (vcx, input) = mount_input(cx, false);
    type_text(vcx, "orders");
    let services = input
        .update_in(vcx, |i, window, cx| {
            i.text_for_range(0..6, &mut None, window, cx)
        })
        .unwrap();
    assert_eq!(services, "orders");
    assert_eq!(a11y_input(vcx).role, Some(Role::TextInput));
}

#[gpui::test]
fn select_reports_expanded_state(cx: &mut TestAppContext) {
    let (vcx, select, _) = mount_select(cx, RETENTION, 3);
    let combo = |vcx: &mut VisualTestContext| {
        a11y_frame(vcx)
            .into_iter()
            .find(|d| d.role == Some(Role::ComboBox))
            .expect("the select declared itself")
    };
    let closed = combo(vcx);
    assert_eq!(closed.expanded, Some(false));
    assert_eq!(closed.value.as_deref(), Some("Forever (Keychain)"));
    focus_select(vcx, &select);
    vcx.simulate_keystrokes("space");
    assert_eq!(combo(vcx).expanded, Some(true));
    vcx.simulate_keystrokes("escape");
    assert_eq!(combo(vcx).expanded, Some(false));
}

#[gpui::test]
fn select_press_released_off_the_field_does_not_eat_the_next_click(cx: &mut TestAppContext) {
    let (vcx, select, _) = mount_select(cx, RETENTION, 0);
    let field = center(vcx, "select-field");
    vcx.simulate_click(field, Modifiers::none());
    vcx.run_until_parked();
    assert!(read(vcx, &select, Select::is_open));
    // Press on the field (closes the menu), release far away: no click.
    vcx.simulate_mouse_down(field, MouseButton::Left, Modifiers::none());
    let away = gpui::point(px(700.0), px(500.0));
    vcx.simulate_mouse_move(away, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(away, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    assert!(!read(vcx, &select, Select::is_open));
    // The next click on the field opens it.
    vcx.simulate_click(field, Modifiers::none());
    vcx.run_until_parked();
    assert!(read(vcx, &select, Select::is_open));
}

#[gpui::test]
fn paste_filter_may_defer_an_edit_of_its_own_input(cx: &mut TestAppContext) {
    let (vcx, input) = mount_input(cx, false);
    let target = input.downgrade();
    input.update(vcx, |i, _| {
        i.set_paste_filter(Some(Rc::new(
            move |text: &str, window: &mut Window, cx: &mut App| {
                let Some(host) = text.strip_prefix("postgres://") else {
                    return false;
                };
                let host = host.split('/').next().unwrap_or_default().to_string();
                let target = target.clone();
                window.defer(cx, move |_, cx| {
                    target
                        .update(cx, |i, cx| {
                            i.set_value(host);
                            cx.notify();
                        })
                        .ok();
                });
                true
            },
        )));
    });
    paste(vcx, &input, "postgres://db.internal/orders");
    vcx.run_until_parked();
    assert_eq!(read(vcx, &input, |i| i.value().to_string()), "db.internal");
}

#[gpui::test]
fn checkbox_and_choice_list_accessibility(cx: &mut TestAppContext) {
    // A checked, disabled checkbox still reports its state; the sub-label is the description.
    let vcx = host(cx, |_, _| {
        div()
            .child(
                Checkbox::new("ro", "Read-only".into())
                    .sub_label("blocks writes".into())
                    .checked(true)
                    .disabled(true),
            )
            .child(
                Checkbox::new("tls", "Verify".into())
                    .sub_label("the server certificate".into())
                    .checked(false),
            )
            .into_any_element()
    });
    let decls = a11y_frame(vcx);
    let find = |name: &str| {
        decls
            .iter()
            .find(|d| d.role == Some(Role::CheckBox) && d.name.as_ref() == name)
            .unwrap_or_else(|| panic!("no checkbox {name}: {decls:?}"))
            .clone()
    };
    let ro = find("Read-only");
    assert!(ro.disabled);
    assert_eq!(ro.toggled, Some(true));
    assert_eq!(ro.description.as_deref(), Some("blocks writes"));
    let tls = find("Verify");
    assert_eq!(tls.toggled, Some(false));
    assert_eq!(tls.description.as_deref(), Some("the server certificate"));

    let (vcx, list) = mount_choices(cx, Some(1));
    list.update(vcx, |l, _| l.set_label(text("Environment")));
    let decls = a11y_frame(vcx);
    assert!(
        decls
            .iter()
            .any(|d| d.role == Some(Role::RadioGroup) && d.name.as_ref() == "Environment"),
        "{decls:?}"
    );
    let staging = decls
        .iter()
        .find(|d| d.role == Some(Role::RadioButton) && d.name.as_ref() == "Staging")
        .unwrap();
    assert_eq!(
        (staging.selected, staging.toggled),
        (Some(true), Some(true))
    );
}

// ## m2a-t21-shots: an in-well group shrinks, but never below a minimum per segment

fn long_well_group(width: f32) -> impl Fn(&mut gpui::Window, &mut App) -> gpui::AnyElement {
    move |_, _| {
        let mut group = Segmented::new("well").in_well(true);
        for i in 0..5 {
            group = group.item(SegmentItem {
                content: SegmentContent::Label(format!("A rather long label {i}").into()),
                pressed: i == 0,
                tooltip: "tip".into(),
                a11y_name: None,
                on_click: Rc::new(|_, _, _| {}),
            });
        }
        div()
            .flex()
            .w(px(width))
            .child(div().flex().min_w(px(0.0)).child(group))
            .into_any_element()
    }
}

#[gpui::test]
fn a_shrinking_well_group_keeps_a_minimum_per_segment(cx: &mut TestAppContext) {
    use hacivat_ui::segmented::SEGMENT_MIN_PX;
    let vcx = host(cx, long_well_group(120.0));
    vcx.simulate_resize(gpui::size(px(800.0), px(200.0)));
    vcx.run_until_parked();
    for i in 0..5 {
        let w = vcx
            .debug_bounds(selector(format!("segmented-well-{i}")))
            .expect("segment drawn")
            .size
            .width;
        assert!(w >= px(SEGMENT_MIN_PX - 0.5), "segment {i}: {w:?}");
    }
}

#[gpui::test]
fn the_natural_width_counts_whole_labels_and_marks(cx: &mut TestAppContext) {
    use hacivat_ui::segmented::natural_width;
    let vcx = host(cx, |_, _| div().into_any_element());
    vcx.update(|window, cx| {
        let theme = *Theme::get(cx);
        let short = natural_width(&[("ab".into(), false)], &theme, window);
        let long = natural_width(&[("abcdefghij".into(), false)], &theme, window);
        let marked = natural_width(&[("ab".into(), true)], &theme, window);
        let two = natural_width(
            &[("ab".into(), false), ("ab".into(), false)],
            &theme,
            window,
        );
        assert!(long > short, "{long:?} > {short:?}");
        // The 7 px mark and its 6 px gap.
        assert_eq!(marked - short, px(13.0));
        // A second segment: its label, padding and one divider.
        assert_eq!(two - short, short - px(2.0) + px(1.0));
    });
}

// ## m2b-t24-shots

/// T20 review M1a: the folded summary may lead with a 12 px text-2 glyph; it goes with the
/// summary when the section opens, and the row declares its expanded state.
#[gpui::test]
fn disclosure_summary_icon_shows_only_while_folded(cx: &mut TestAppContext) {
    use hacivat_ui::icon::{IconName, icon_record, icon_take};

    let open = Rc::new(Cell::new(false));
    let state = open.clone();
    let vcx = host(cx, move |_, _| {
        let state = state.clone();
        div()
            .w(px(600.0))
            .child(
                Disclosure::new("ssh", "SSH tunnel".into())
                    .summary("On · bastion.acme.io".into())
                    .summary_icon(Some(IconName::Route))
                    .open(state.get())
                    .on_toggle(move |open, window, _| {
                        state.set(open);
                        window.refresh();
                    }),
            )
            .into_any_element()
    });
    let frame = |vcx: &mut VisualTestContext| {
        icon_record();
        a11y_record();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let icons: Vec<IconName> = icon_take().into_iter().map(|i| i.name).collect();
        let row = a11y_take()
            .into_iter()
            .find(|d| d.role == Some(Role::Button) && d.name.as_ref() == "SSH tunnel")
            .expect("the row declared itself");
        (icons, row.expanded)
    };
    let (icons, expanded) = frame(vcx);
    assert_eq!(icons, [IconName::ChevronRight, IconName::Route]);
    assert_eq!(expanded, Some(false));
    open.set(true);
    let (icons, expanded) = frame(vcx);
    assert_eq!(icons, [IconName::ChevronDown], "no summary, no glyph");
    assert_eq!(expanded, Some(true));
    // Without the option the folded row draws only its chevron (M2a, unchanged).
    let vcx = host(cx, |_, _| {
        Disclosure::new("tls", "TLS".into())
            .summary(TLS_SUMMARY.into())
            .into_any_element()
    });
    icon_record();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let icons: Vec<IconName> = icon_take().into_iter().map(|i| i.name).collect();
    assert_eq!(icons, [IconName::ChevronRight]);
}

/// A placeholder wider than its field ends in an ellipsis (the long-locale rule for one-line
/// values) and is recorded as truncated, never clipped; a short one is drawn whole.
#[gpui::test]
fn a_long_placeholder_ends_in_an_ellipsis(cx: &mut TestAppContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let long = "[şȧȧȧṽḗḗḗḓ ıııƞ ŧħḗḗḗ Ķḗḗḗẏçħȧȧȧiiƞ, şȧȧȧṽḗḗḗḓ ıııƞ ŧħḗḗḗ Ķḗḗḗẏçħȧȧȧiiƞ]";
    let (_, vcx) = cx.add_window_view(|_, cx| InputHost {
        input: cx.new(|cx| TextInput::new(cx, text(long))),
    });
    enable_registry(vcx);
    let placeholder = |vcx: &mut VisualTestContext| {
        vcx.read(|cx| {
            cx.global::<TextRegistry>()
                .frame
                .iter()
                .find(|r| r.path.ends_with("#placeholder"))
                .cloned()
                .expect("the placeholder is drawn")
        })
    };
    let drawn = placeholder(vcx);
    assert!(drawn.truncated && !drawn.clipped, "{drawn:?}");
    assert_eq!(drawn.text.as_ref(), long, "the record keeps the whole text");
    assert!(drawn.painted_px <= 300.5, "{}", drawn.painted_px);

    let (_, vcx) = cx.add_window_view(|_, cx| InputHost {
        input: cx.new(|cx| TextInput::new(cx, text("Password"))),
    });
    enable_registry(vcx);
    let drawn = placeholder(vcx);
    assert!(!drawn.truncated && !drawn.clipped, "{drawn:?}");
}
