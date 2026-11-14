//! M3a Task 19: the editor's paint, checked on the rendered scene (`scene-check`): the caret,
//! the selection, the gutter width, where the text starts, the inks, and no red quad.
//!
//! The geometry follows `design/m3a-query/NOTES.md` surface 1 (binding over the plan's older
//! paint list): the code area and the gutter are one `panel` ground, the gutter is 58 px for
//! three digits and one advance wider per extra digit, the text starts 8 px right of it and
//! 6 px below the top, the caret is 2 × 16 px `blue` 1 px below its row's top, the selection
//! `blue_tint_strong` when focused and `seam_strong` when not.
#![cfg_attr(not(feature = "scene-check"), allow(dead_code, unused_imports))]

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Rgba, Styled as _, TestAppContext, VisualTestContext,
    Window, div, px,
};
use hacivat_ui_core::theme::{DARK, Theme, ThemeMode};
use hacivat_ui_editor::{DocId, EditorConfig, EditorView, FailClosedService};

struct Host {
    editor: Entity<EditorView>,
    other: FocusHandle,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(800.0))
            .h(px(600.0))
            .track_focus(&self.other)
            .child(self.editor.clone())
    }
}

/// The editor and a second focus target (to blur it).
type Opened = (Entity<EditorView>, FocusHandle);

fn open(
    cx: &mut TestAppContext,
    text: String,
) -> (Entity<EditorView>, FocusHandle, &mut VisualTestContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let slot: Rc<RefCell<Option<Opened>>> = Rc::default();
    let out = slot.clone();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(91),
                &text,
                EditorConfig::default(),
                Rc::new(FailClosedService),
                Rc::new(|_: &gpui::App| "Console 1".into()),
                window,
                cx,
            )
        });
        let other = cx.focus_handle();
        *out.borrow_mut() = Some((editor.clone(), other.clone()));
        Host { editor, other }
    });
    let (editor, other) = slot.borrow().clone().unwrap();
    vcx.update(|window, _| window.activate_window());
    editor.update_in(vcx, |v, window, cx| {
        let f = v.focus_handle(cx);
        window.focus(&f, cx);
    });
    draw(vcx);
    (editor, other, vcx)
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

fn same(a: Hsla, b: Hsla) -> bool {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    (a.r - b.r).abs() < 1.5 / 255.0
        && (a.g - b.g).abs() < 1.5 / 255.0
        && (a.b - b.b).abs() < 1.5 / 255.0
        && (a.a - b.a).abs() < 1.5 / 255.0
}

/// The solid quads of the last frame filled with `color`, as logical (x, y, w, h).
#[cfg(feature = "scene-check")]
fn quads(vcx: &mut VisualTestContext, color: Hsla) -> Vec<(f32, f32, f32, f32)> {
    vcx.update(|window, _| {
        let scale = window.scale_factor();
        window
            .rendered_scene()
            .quads
            .iter()
            .filter(|q| q.background.as_solid().is_some_and(|c| same(c, color)))
            .map(|q| {
                let b = q.bounds;
                (
                    b.origin.x.0 / scale,
                    b.origin.y.0 / scale,
                    b.size.width.0 / scale,
                    b.size.height.0 / scale,
                )
            })
            .collect()
    })
}

#[cfg(feature = "scene-check")]
fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

#[cfg(feature = "scene-check")]
#[gpui::test]
fn caret_selection_gutter_geometry(cx: &mut TestAppContext) {
    let c = &DARK.colors;
    let (e, other, vcx) = open(cx, "SELECT 1;\nSELECT 2;\n".to_string());
    e.update(vcx, |v, cx| v.select_range(0..6, cx));
    draw(vcx);
    let layout = vcx.read(|cx| e.read(cx).last_layout().cloned()).unwrap();
    let rows = *layout.rows();
    let x = |b: usize| f32::from(layout.x_for_offset(b).unwrap());
    let adv = x(1) - x(0);
    // The gutter: 58 px for three digits; the text 8 px right of it, 6 px below the top.
    assert!(close(f32::from(rows.bounds.left()), 58.0), "{rows:?}");
    assert!(close(x(0), 58.0 + 8.0));
    assert!(close(f32::from(rows.first_y), 6.0));
    // One panel ground under the code and the gutter (no gutter fill, no seam: NOTES §1).
    let ground = quads(vcx, c.panel.to_hsla());
    assert!(
        ground
            .iter()
            .any(|&(x, y, w, h)| x == 0.0 && y == 0.0 && w == 800.0 && h == 600.0),
        "{ground:?}"
    );
    assert!(quads(vcx, c.gutter.to_hsla()).is_empty());
    // The selection: `blue_tint_strong` over "SELECT" on row 0.
    let sel = quads(vcx, c.blue_tint_strong.to_hsla());
    assert_eq!(sel.len(), 1, "{sel:?}");
    let (sx, sy, sw, sh) = sel[0];
    assert!(close(sx, 66.0) && close(sy, 6.0) && close(sw, 6.0 * adv) && close(sh, 18.0));
    // The caret: 2 × 16 px `blue`, centred on the boundary after "SELECT", 1 px below the row.
    let caret = quads(vcx, c.blue.to_hsla());
    assert_eq!(caret.len(), 1, "{caret:?}");
    let (cx_, cy, cw, ch) = caret[0];
    assert!(close(cx_, 66.0 + 6.0 * adv - 1.0), "{caret:?}");
    assert!(
        close(cy, 7.0) && close(cw, 2.0) && close(ch, 16.0),
        "{caret:?}"
    );
    // No red quad, ever, in a non-production editor.
    for red in [c.red, c.red_line, c.red_tint, c.cp_fill] {
        assert!(quads(vcx, red.to_hsla()).is_empty());
    }
    // Unfocused: no caret, a neutral selection.
    vcx.update(|window, cx| window.focus(&other, cx));
    draw(vcx);
    assert!(
        quads(vcx, c.blue.to_hsla()).is_empty(),
        "no caret when unfocused"
    );
    assert!(quads(vcx, c.blue_tint_strong.to_hsla()).is_empty());
    assert_eq!(quads(vcx, c.seam_strong.to_hsla()).len(), 1);
}

#[cfg(feature = "scene-check")]
#[gpui::test]
fn the_gutter_grows_with_the_last_line_number_and_controls_are_chips(cx: &mut TestAppContext) {
    let c = &DARK.colors;
    // 10,000 lines: five digits, so two advances wider than 58 px.
    let mut text: String = (0..9_999).map(|i| format!("SELECT {i};\n")).collect();
    text.insert_str(0, "a\u{0}b\n");
    let (e, _, vcx) = open(cx, text);
    let layout = vcx.read(|cx| e.read(cx).last_layout().cloned()).unwrap();
    let x = |b: usize| f32::from(layout.x_for_offset(b).unwrap());
    let adv = x(1) - x(0);
    assert!(close(
        f32::from(layout.rows().bounds.left()),
        58.0 + 2.0 * adv
    ));
    assert!(close(x(0), 58.0 + 2.0 * adv + 8.0));
    // The NUL is a chip on `seam_soft` between `a` and `b`.
    let chips = quads(vcx, c.seam_soft.to_hsla());
    assert_eq!(chips.len(), 1, "{chips:?}");
    let (cx_, _, cw, ch) = chips[0];
    assert!(cx_ > x(1) && cx_ + cw < x(2), "{chips:?}");
    assert!(close(ch, 14.0));
}
