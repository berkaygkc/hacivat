//! M3a Task 19: the editor element: the content-keyed shape cache, long-line windows,
//! row-anchored scrolling, mouse selection, the probe, the caret blink and the cached
//! embedding. Tests wait on conditions (`run_until_parked`) and drive time through the fake
//! clock only.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyView, AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point, Render,
    ScrollDelta, ScrollEvent, StyleRefinement, Styled as _, TestAppContext, VisualTestContext,
    Window, div, point, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::{
    CaretBlink, DocId, EditorConfig, EditorView, FailClosedService, FrameLayout, FrameProbe,
};

/// A view beside the editor that counts its renders (the results tile's stand-in).
struct Sibling {
    renders: Rc<Cell<u32>>,
}

impl Render for Sibling {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div().w(px(800.0)).h(px(100.0))
    }
}

/// A window whose root holds one editor in an 800 × 600 box; with `cached`, the editor and a
/// sibling are embedded as `AnyView::cached`, as the console host does. The root renders once
/// per drawn frame (`frames`).
struct Host {
    editor: Entity<EditorView>,
    sibling: Entity<Sibling>,
    cached: bool,
    frames: Rc<Cell<u32>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.frames.set(self.frames.get() + 1);
        let body = div().w(px(800.0)).h(px(600.0));
        if self.cached {
            div()
                .flex()
                .flex_col()
                .child(
                    body.child(
                        AnyView::from(self.editor.clone())
                            .cached(StyleRefinement::default().size_full()),
                    ),
                )
                .child(AnyView::from(self.sibling.clone()).cached(StyleRefinement::default()))
        } else {
            div().child(body.child(self.editor.clone()))
        }
    }
}

struct Opened<'a> {
    editor: Entity<EditorView>,
    sibling: Entity<Sibling>,
    /// The sibling's renders.
    renders: Rc<Cell<u32>>,
    /// Frames drawn.
    frames: Rc<Cell<u32>>,
    vcx: &'a mut VisualTestContext,
}

fn open_with(cx: &mut TestAppContext, text: String, cfg: EditorConfig, cached: bool) -> Opened<'_> {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let renders = Rc::new(Cell::new(0));
    let frames = Rc::new(Cell::new(0));
    let (out, count, drawn) = (slot.clone(), renders.clone(), frames.clone());
    let (host, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(19),
                &text,
                cfg,
                Rc::new(FailClosedService),
                Rc::new(|_: &gpui::App| "Console 1".into()),
                window,
                cx,
            )
        });
        *out.borrow_mut() = Some(editor.clone());
        let sibling = cx.new(|_| Sibling { renders: count });
        Host {
            editor,
            sibling,
            cached,
            frames: drawn,
        }
    });
    let editor = slot.borrow().clone().unwrap();
    let sibling = vcx.read(|cx| host.read(cx).sibling.clone());
    draw(vcx);
    Opened {
        editor,
        sibling,
        renders,
        frames,
        vcx,
    }
}

fn open(cx: &mut TestAppContext, text: String) -> Opened<'_> {
    open_with(cx, text, EditorConfig::default(), false)
}

/// `n` numbered lines of SQL (none empty).
fn lines(n: usize) -> String {
    (0..n)
        .map(|i| format!("SELECT col_{i} FROM t_{i};\n"))
        .collect()
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

fn focus(o: &mut Opened<'_>) {
    o.vcx.update(|window, _| window.activate_window());
    o.editor.update_in(o.vcx, |v, window, cx| {
        let f = v.focus_handle(cx);
        window.focus(&f, cx);
    });
    draw(o.vcx);
}

fn layout(o: &Opened<'_>) -> FrameLayout {
    o.vcx
        .read(|cx| o.editor.read(cx).last_layout().cloned())
        .expect("a frame was painted")
}

fn shapes(o: &Opened<'_>) -> u64 {
    o.vcx.read(|cx| o.editor.read(cx).layout_stats().shapes)
}

fn hits(o: &Opened<'_>) -> u64 {
    o.vcx.read(|cx| o.editor.read(cx).layout_stats().hits)
}

fn text(o: &Opened<'_>) -> String {
    o.vcx
        .read(|cx| o.editor.read(cx).snapshot().buffer.to_string())
}

fn selection(o: &Opened<'_>) -> (usize, usize) {
    o.vcx.read(|cx| {
        let s = o.editor.read(cx).snapshot();
        let m = s.selection.main();
        (m.anchor, m.head)
    })
}

/// A point just right of byte `b`'s left edge, in the middle of its row.
fn at_byte(o: &Opened<'_>, b: usize) -> Point<Pixels> {
    let l = layout(o);
    let r = l.bounds_for_range(b..b).expect("the byte's row is painted");
    point(
        r.origin.x + px(1.0),
        r.origin.y + l.rows().line_height / 2.0,
    )
}

fn wheel(o: &mut Opened<'_>, dy: f32) {
    let c = layout(o).rows().bounds.center();
    o.vcx.simulate_event(ScrollEvent {
        position: c,
        delta: ScrollDelta::Pixels(point(px(0.0), px(-dy))),
        ..Default::default()
    });
    draw(o.vcx);
}

fn mouse_down(o: &mut Opened<'_>, p: Point<Pixels>, clicks: usize, modifiers: Modifiers) {
    o.vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: p,
        modifiers,
        click_count: clicks,
        first_mouse: false,
    });
}

fn mouse_up(o: &mut Opened<'_>, p: Point<Pixels>) {
    o.vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: p,
        modifiers: Modifiers::none(),
        click_count: 1,
    });
    draw(o.vcx);
}

fn drag_to(o: &mut Opened<'_>, p: Point<Pixels>) {
    o.vcx.simulate_event(MouseMoveEvent {
        position: p,
        modifiers: Modifiers::none(),
        pressed_button: Some(MouseButton::Left),
    });
    draw(o.vcx);
}

#[gpui::test]
fn scrolling_back_hits_the_cache_95_percent(cx: &mut TestAppContext) {
    let mut o = open(cx, lines(50_000));
    for _ in 0..20 {
        wheel(&mut o, 540.0);
    }
    assert!(layout(&o).rows().first_line > 500, "scrolled down");
    let (s0, h0) = (shapes(&o), hits(&o));
    for _ in 0..20 {
        wheel(&mut o, -540.0);
    }
    assert_eq!(layout(&o).rows().first_line, 0, "back at the top");
    let (s, h) = (shapes(&o) - s0, hits(&o) - h0);
    let rate = h as f64 / (h + s) as f64;
    assert!(rate >= 0.95, "hit rate {rate:.3} ({h} hits, {s} shapes)");
}

#[gpui::test]
fn theme_switch_reshapes_only_visible_lines(cx: &mut TestAppContext) {
    let o = open(cx, lines(50_000));
    let rows = *layout(&o).rows();
    let visible = u64::from(rows.last_line - rows.first_line + 1);
    assert!((30..40).contains(&visible), "{visible} rows in 600 px");
    let before = shapes(&o);
    o.vcx.update(|window, cx| {
        Theme::install(cx, ThemeMode::Light);
        window.refresh();
    });
    o.vcx.run_until_parked();
    assert_eq!(
        shapes(&o) - before,
        visible,
        "only the painted rows reshape"
    );
    // Redrawing in the same theme reshapes nothing.
    let before = shapes(&o);
    draw(o.vcx);
    assert_eq!(shapes(&o), before);
}

#[gpui::test]
fn one_line_reshaped_per_typed_character(cx: &mut TestAppContext) {
    let mut o = open(cx, lines(50_000));
    focus(&mut o);
    let before = shapes(&o);
    o.vcx.simulate_input("x");
    draw(o.vcx);
    assert!(text(&o).starts_with("xSELECT col_0"), "typed at the caret");
    assert_eq!(
        shapes(&o) - before,
        1,
        "one line reshaped per typed character"
    );
    for c in ["y", "z"] {
        let before = shapes(&o);
        o.vcx.simulate_input(c);
        draw(o.vcx);
        assert_eq!(shapes(&o) - before, 1, "{c}");
    }
}

#[gpui::test]
fn long_line_shapes_a_window(cx: &mut TestAppContext) {
    let long = "x".repeat(100_000);
    let mut o = open(cx, format!("{long}\nSELECT 1;\n"));
    assert!(
        o.vcx
            .read(|cx| o.editor.read(cx).layout_stats().longest_shaped)
            <= 512 + 128
    );
    // The caret in the middle of the line scrolls there; the window follows it.
    focus(&mut o);
    o.editor
        .update(o.vcx, |v, cx| v.select_range(50_000..50_000, cx));
    draw(o.vcx);
    let stats = o.vcx.read(|cx| o.editor.read(cx).layout_stats());
    assert!(stats.longest_shaped <= 512 + 128, "{stats:?}");
    let l = layout(&o);
    let caret = o
        .vcx
        .read(|cx| o.editor.read(cx).caret_bounds())
        .expect("the caret is painted");
    let area = l.rows().bounds;
    assert!(
        caret.left() > area.left() && caret.right() < area.right(),
        "{caret:?} in {area:?}"
    );
    // A click left of the caret (it sits near the right edge) lands there: x inside the shaped
    // window is exact.
    let p = at_byte(&o, 49_990);
    mouse_down(&mut o, p, 1, Modifiers::none());
    mouse_up(&mut o, p);
    assert_eq!(selection(&o), (49_990, 49_990));
    // Typing there reshapes only the window.
    let before = shapes(&o);
    o.vcx.simulate_input("y");
    draw(o.vcx);
    assert_eq!(shapes(&o) - before, 1);
}

#[gpui::test]
fn click_drag_double_triple(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT id, name FROM users;\nSELECT 2;\n".to_string());
    // A click places the caret (and takes the focus).
    let p7 = at_byte(&o, 7);
    mouse_down(&mut o, p7, 1, Modifiers::none());
    mouse_up(&mut o, p7);
    assert_eq!(selection(&o), (7, 7));
    let focused = o
        .vcx
        .update(|window, cx| o.editor.read(cx).focus_handle(cx).is_focused(window));
    assert!(focused, "a click focuses the editor");
    // A drag extends from the click.
    mouse_down(&mut o, p7, 1, Modifiers::none());
    let p14 = at_byte(&o, 14);
    drag_to(&mut o, p14);
    mouse_up(&mut o, p14);
    assert_eq!(selection(&o), (7, 14));
    // Shift-click extends from the anchor.
    let p20 = at_byte(&o, 20);
    mouse_down(&mut o, p20, 1, Modifiers::shift());
    mouse_up(&mut o, p20);
    assert_eq!(selection(&o), (7, 20));
    // A double click selects the word; a drag extends by words.
    let p8 = at_byte(&o, 8);
    mouse_down(&mut o, p8, 2, Modifiers::none());
    assert_eq!(selection(&o), (7, 9), "the word `id`");
    let p12 = at_byte(&o, 12);
    drag_to(&mut o, p12);
    assert_eq!(selection(&o), (7, 15), "through the word `name`");
    mouse_up(&mut o, p12);
    // A triple click selects the line with its break.
    let p30 = at_byte(&o, 30);
    mouse_down(&mut o, p30, 3, Modifiers::none());
    mouse_up(&mut o, p30);
    assert_eq!(selection(&o), (28, 38));
    // Below the last line: the document's end.
    let l = layout(&o);
    let below = point(
        l.rows().bounds.center().x,
        l.rows().bounds.bottom() - px(4.0),
    );
    mouse_down(&mut o, below, 1, Modifiers::none());
    mouse_up(&mut o, below);
    assert_eq!(selection(&o), (38, 38));
}

#[gpui::test]
fn pointer_hits_snap_to_clusters_and_never_split_crlf(cx: &mut TestAppContext) {
    // "e" + U+0301 is one cluster (bytes 1..4); the line ends in CRLF.
    let mut o = open(cx, "ae\u{301}b\r\nSELECT 1".to_string());
    let l = layout(&o);
    for x in 0..120 {
        let p = point(
            l.rows().bounds.left() + px(x as f32),
            l.rows().first_y + px(9.0),
        );
        if let Some(b) = l.offset_for_point(p) {
            assert!([0, 1, 4, 5].contains(&b), "x {x}: {b}");
        }
    }
    // A click past the line's end lands before the CRLF, never between CR and LF.
    let end = point(
        l.rows().bounds.right() - px(20.0),
        l.rows().first_y + px(9.0),
    );
    mouse_down(&mut o, end, 1, Modifiers::none());
    mouse_up(&mut o, end);
    assert_eq!(selection(&o), (5, 5));
}

#[gpui::test]
fn tabs_and_control_chips_take_their_width(cx: &mut TestAppContext) {
    // A tab to the next stop of 4, then a NUL chip with its own width.
    let o = open(cx, "a\tb\u{0}c".to_string());
    let l = layout(&o);
    let x = |b: usize| f32::from(l.x_for_offset(b).unwrap());
    let x0 = x(0);
    let adv = x(1) - x0;
    assert!(adv > 1.0);
    assert!((x(2) - x0 - 4.0 * adv).abs() < 0.01, "`b` at column 4");
    let chip = x(4) - x(3);
    assert!(
        chip > adv * 2.0,
        "the NUL chip is wider than a cell: {chip}"
    );
    // A point inside the chip lands on one of its edges.
    let mid = point(px(x(3) + chip / 2.0 - 1.0), l.rows().first_y + px(9.0));
    assert_eq!(l.offset_for_point(mid), Some(3));
    let mid = point(px(x(3) + chip / 2.0 + 1.0), l.rows().first_y + px(9.0));
    assert_eq!(l.offset_for_point(mid), Some(4));
}

#[derive(Default)]
struct Counts {
    inputs: Cell<u32>,
    painted: Cell<u32>,
}

impl FrameProbe for Counts {
    fn input(&self, _: Instant) {
        self.inputs.set(self.inputs.get() + 1);
    }
    fn painted(&self, _: &mut gpui::App) {
        self.painted.set(self.painted.get() + 1);
    }
}

fn counts(c: &Counts) -> (u32, u32) {
    (c.inputs.get(), c.painted.get())
}

#[gpui::test]
fn probe_called_once_per_input_and_frame(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT 1;\n".to_string());
    let probe = Rc::new(Counts::default());
    let p: Rc<dyn FrameProbe> = probe.clone();
    o.editor.update(o.vcx, |v, _| v.set_probe(Some(p)));
    focus(&mut o);
    // `painted` once per drawn frame (the root renders once per frame), `input` once per
    // input; the first key also switches GPUI to keyboard modality, which redraws everything.
    for c in ["a", "b", "c"] {
        let (i0, p0) = counts(&probe);
        let f0 = o.frames.get();
        o.vcx.simulate_input(c);
        let frames = o.frames.get() - f0;
        assert!(frames >= 1, "{c} drew a frame");
        assert_eq!(counts(&probe), (i0 + 1, p0 + frames), "{c}: one input");
    }
    // A frame without input.
    let (i0, p0) = counts(&probe);
    let f0 = o.frames.get();
    draw(o.vcx);
    assert_eq!(o.frames.get() - f0, 1);
    assert_eq!(counts(&probe), (i0, p0 + 1));
    // A wheel step is an input too.
    wheel(&mut o, 18.0);
    assert_eq!(counts(&probe).0, i0 + 1);
}

#[gpui::test]
fn caret_blink_rules_with_a_fake_clock(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT 1;\n".to_string());
    let visible = |o: &Opened<'_>| o.vcx.read(|cx| o.editor.read(cx).caret_visible());
    let advance = |o: &mut Opened<'_>, ms: u64| {
        o.vcx.executor().advance_clock(Duration::from_millis(ms));
        o.vcx.run_until_parked();
    };
    assert!(!visible(&o), "no caret without the focus");
    focus(&mut o);
    assert!(visible(&o));
    advance(&mut o, 530);
    assert!(!visible(&o), "the first half-period hides it");
    advance(&mut o, 530);
    assert!(visible(&o));
    // Typing keeps it solid: every input restarts the half-period.
    advance(&mut o, 300);
    o.vcx.simulate_input("x");
    assert!(visible(&o));
    advance(&mut o, 400);
    assert!(visible(&o), "400 ms after the input");
    advance(&mut o, 200);
    assert!(!visible(&o), "600 ms after the input");
    // After 10 s without input: solid, and nothing repaints any more.
    advance(&mut o, 10_000);
    assert!(visible(&o));
    let probe = Rc::new(Counts::default());
    let p: Rc<dyn FrameProbe> = probe.clone();
    o.editor.update(o.vcx, |v, _| v.set_probe(Some(p)));
    advance(&mut o, 5_000);
    assert!(visible(&o));
    assert_eq!(probe.painted.get(), 0, "the blink stopped");
    // Static under reduced motion and with the blink off.
    for cfg in [
        EditorConfig {
            reduced_motion: true,
            ..EditorConfig::default()
        },
        EditorConfig {
            caret_blink: CaretBlink::Off,
            ..EditorConfig::default()
        },
    ] {
        o.editor.update(o.vcx, |v, cx| v.set_config(cfg, cx));
        for _ in 0..4 {
            advance(&mut o, 530);
            assert!(visible(&o), "{cfg:?}");
        }
    }
}

#[gpui::test]
fn embeds_as_a_cached_view(cx: &mut TestAppContext) {
    let mut o = open_with(cx, lines(100), EditorConfig::default(), true);
    focus(&mut o);
    // The first key switches GPUI to keyboard modality, which redraws the whole window.
    o.vcx.simulate_input("w");
    let sibling_renders = o.renders.get();
    // Typing notifies the editor only: the cached sibling is not rebuilt.
    o.vcx.simulate_input("x");
    assert!(text(&o).starts_with("wxSELECT"));
    assert_eq!(
        o.renders.get(),
        sibling_renders,
        "typing never re-renders the sibling"
    );
    assert_eq!(
        layout(&o).version(),
        o.vcx.read(|cx| o.editor.read(cx).snapshot().version),
        "the editor itself repainted"
    );
    // A frame where only the sibling changes reuses the editor's paint: its input handler
    // still takes text.
    o.sibling.update(o.vcx, |_, cx| cx.notify());
    o.vcx.run_until_parked();
    assert_eq!(o.renders.get(), sibling_renders + 1);
    o.vcx.simulate_input("y");
    assert!(text(&o).starts_with("wxySELECT"), "{}", &text(&o)[..12]);
    assert_eq!(o.renders.get(), sibling_renders + 1);
    // And its pointer listeners still take clicks.
    o.sibling.update(o.vcx, |_, cx| cx.notify());
    o.vcx.run_until_parked();
    let p = at_byte(&o, 7);
    mouse_down(&mut o, p, 1, Modifiers::none());
    mouse_up(&mut o, p);
    assert_eq!(selection(&o), (7, 7));
}

// ## m3a-t19-editor-view (review round 1)

fn right_click(o: &mut Opened<'_>, p: Point<Pixels>) {
    o.vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Right,
        position: p,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    o.vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Right,
        position: p,
        modifiers: Modifiers::none(),
        click_count: 1,
    });
    draw(o.vcx);
}

#[gpui::test]
fn right_click_keeps_a_selection_it_falls_in_and_moves_the_caret_otherwise(
    cx: &mut TestAppContext,
) {
    let mut o = open(cx, "SELECT id, name FROM users;\n".to_string());
    o.editor.update(o.vcx, |v, cx| v.select_range(7..15, cx));
    draw(o.vcx);
    // Inside the selection: it stays (the menu acts on it).
    let inside = at_byte(&o, 10);
    right_click(&mut o, inside);
    assert_eq!(selection(&o), (7, 15));
    let focused = o
        .vcx
        .update(|window, cx| o.editor.read(cx).focus_handle(cx).is_focused(window));
    assert!(focused, "a right click focuses the editor");
    // Outside it: the caret moves to the pointer.
    let outside = at_byte(&o, 21);
    right_click(&mut o, outside);
    assert_eq!(selection(&o), (21, 21));
}

#[gpui::test]
fn dragging_past_the_bottom_scrolls_and_extends(cx: &mut TestAppContext) {
    let mut o = open(cx, lines(500));
    let start = at_byte(&o, 3);
    mouse_down(&mut o, start, 1, Modifiers::none());
    let rows = *layout(&o).rows();
    let below = point(rows.bounds.center().x, rows.bounds.bottom() + px(30.0));
    for _ in 0..4 {
        drag_to(&mut o, below);
    }
    mouse_up(&mut o, below);
    let after = *layout(&o).rows();
    assert!(after.first_line > rows.first_line, "{rows:?} → {after:?}");
    let (anchor, head) = selection(&o);
    assert_eq!(anchor, 3);
    let head_line = o
        .vcx
        .read(|cx| o.editor.read(cx).snapshot().buffer.byte_to_line(head) as u32);
    assert!(
        head_line > rows.last_line,
        "the selection grew past the first view"
    );
    // A drag past the top scrolls back.
    let mut o2 = o;
    let above = point(rows.bounds.center().x, rows.bounds.top() - px(30.0));
    let mid = at_byte(&o2, head);
    mouse_down(&mut o2, mid, 1, Modifiers::none());
    for _ in 0..4 {
        drag_to(&mut o2, above);
    }
    mouse_up(&mut o2, above);
    assert!(layout(&o2).rows().first_line < after.first_line);
}

#[gpui::test]
fn shift_wheel_scrolls_sideways(cx: &mut TestAppContext) {
    let o = open(cx, format!("{}\nSELECT 1;\n", "x".repeat(300)));
    let x0 = f32::from(layout(&o).x_for_offset(0).unwrap());
    let c = layout(&o).rows().bounds.center();
    o.vcx.simulate_event(ScrollEvent {
        position: c,
        delta: ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
        modifiers: Modifiers::shift(),
        ..Default::default()
    });
    draw(o.vcx);
    let l = layout(&o);
    assert!(
        (f32::from(l.x_for_offset(0).unwrap()) - (x0 - 120.0)).abs() < 0.01,
        "a vertical wheel with Shift scrolls 120 px sideways"
    );
    assert_eq!(l.rows().first_line, 0, "and not vertically");
}
