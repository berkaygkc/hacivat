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
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Rgba, Styled as _, TestAppContext, VisualTestContext,
    Window, div, px,
};
use hacivat_text::display::{Decoration, RangeSet};
use hacivat_text::{ChangeDesc, ChangeLog, Snapshot, TokenOracle};
use hacivat_ui_core::theme::{DARK, Theme, ThemeMode};
use hacivat_ui_editor::{
    DecorationSource, DocContext, DocId, EditorConfig, EditorView, ExecAction, ExecAt, ExecPreview,
    FailClosedService, HighlightClass, InvalidateFn, LanguageService, NoTokens, SourceId,
    StatementSpan,
};
use smallvec::SmallVec;

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
    open_with(cx, text, Rc::new(FailClosedService))
}

fn open_with(
    cx: &mut TestAppContext,
    text: String,
    service: Rc<dyn LanguageService>,
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
                service,
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
    ordered_quads(vcx, color)
        .into_iter()
        .map(|(_, q)| q)
        .collect()
}

/// [`quads`] with each quad's draw order.
#[cfg(feature = "scene-check")]
fn ordered_quads(vcx: &mut VisualTestContext, color: Hsla) -> Vec<(u32, (f32, f32, f32, f32))> {
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
                    q.order,
                    (
                        b.origin.x.0 / scale,
                        b.origin.y.0 / scale,
                        b.size.width.0 / scale,
                        b.size.height.0 / scale,
                    ),
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

// ## m3a-t19-editor-view (review round 1)

#[cfg(feature = "scene-check")]
#[gpui::test]
fn a_selection_across_lines_paints_one_rect_per_row(cx: &mut TestAppContext) {
    let c = &DARK.colors;
    let (e, _, vcx) = open(cx, "SELECT a,\n  b\nFROM t;\n".to_string());
    // From inside line 0 to inside line 2: line 1 is selected whole, with its break.
    e.update(vcx, |v, cx| v.select_range(7..18, cx));
    draw(vcx);
    let layout = vcx.read(|cx| e.read(cx).last_layout().cloned()).unwrap();
    let x = |b: usize| f32::from(layout.x_for_offset(b).unwrap());
    let adv = x(1) - x(0);
    let mut sel = quads(vcx, c.blue_tint_strong.to_hsla());
    sel.sort_by(|a, b| a.1.total_cmp(&b.1));
    assert_eq!(sel.len(), 3, "{sel:?}");
    // GPUI snaps a quad's far edge to the pixel grid: a fractional width may lose up to half a
    // pixel.
    let snapped = |a: f32, b: f32| (a - b).abs() <= 0.5;
    // Row 0: from `a` to the line's end, plus half an advance for the selected break.
    assert!(
        close(sel[0].0, x(7)) && snapped(sel[0].2, x(9) - x(7) + adv / 2.0),
        "{sel:?}"
    );
    // Row 1 ("  b"): the whole line and its break.
    assert!(
        close(sel[1].0, x(10)) && snapped(sel[1].2, 3.0 * adv + adv / 2.0),
        "{sel:?}"
    );
    // Row 2: from the line's start through `M` (to byte 18, column 4).
    assert!(
        close(sel[2].0, x(14)) && close(sel[2].2, x(18) - x(14)),
        "{sel:?}"
    );
    for (i, q) in sel.iter().enumerate() {
        assert!(
            close(q.1, 6.0 + 18.0 * i as f32) && close(q.3, 18.0),
            "{sel:?}"
        );
    }
}

/// Marks each range with a highlight class (a decoration source, the SQL adapter's stand-in).
struct Marks {
    priority: i16,
    marks: Vec<(Range<usize>, HighlightClass)>,
}

impl DecorationSource for Marks {
    fn id(&self) -> SourceId {
        SourceId(self.priority as u16)
    }
    fn priority(&self) -> i16 {
        self.priority
    }
    fn decorations(
        &self,
        _: &Snapshot,
        _: &ChangeLog,
        _: Range<usize>,
    ) -> Arc<RangeSet<Decoration>> {
        let mut v: Vec<_> = self
            .marks
            .iter()
            .map(|(r, c)| (r.clone(), Decoration::Mark { style: c.style() }))
            .collect();
        v.sort_by_key(|(r, _)| r.start);
        Arc::new(RangeSet::from_sorted(v))
    }
}

/// A service whose only knowledge is its decoration sources.
struct Highlighting(SmallVec<[Rc<dyn DecorationSource>; 2]>);

impl LanguageService for Highlighting {
    fn open(&self, _: DocId, _: &Snapshot, _: DocContext, _: &mut gpui::App) {}
    fn set_context(&self, _: DocId, _: DocContext, _: &mut gpui::App) {}
    fn change(&self, _: DocId, _: &Snapshot, _: &ChangeDesc, _: &mut gpui::App) {}
    fn close(&self, _: DocId) {}
    fn on_update(&self, _: DocId, _: InvalidateFn) {}
    fn statements(&self, _: DocId, _: u64, _: Range<usize>) -> Arc<[StatementSpan]> {
        Arc::from(Vec::new())
    }
    fn exec_preview(&self, _: DocId, _: u64, _: &ExecAt, _: ExecAction) -> Option<ExecPreview> {
        None
    }
    fn token_oracle(&self, _: DocId) -> Rc<dyn TokenOracle> {
        Rc::new(NoTokens)
    }
    fn decoration_sources(&self, _: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
        self.0.clone()
    }
}

/// A MySQL executable comment sits on the `seam_soft` inlay well: one quad over the whole
/// comment, the row's height, painted before the selection (which stays visible on it), with
/// the tokens inside it highlighted by a lower-priority source (review round 1).
#[cfg(feature = "scene-check")]
#[gpui::test]
fn an_executable_comment_sits_on_its_well_under_the_selection(cx: &mut TestAppContext) {
    let c = &DARK.colors;
    let text = "/*!40101 SET x = 1 */ SELECT 2;";
    let tokens: Rc<dyn DecorationSource> = Rc::new(Marks {
        priority: 0,
        marks: vec![
            (9..12, HighlightClass::Keyword),
            (17..18, HighlightClass::Number),
            (22..28, HighlightClass::Keyword),
        ],
    });
    let well: Rc<dyn DecorationSource> = Rc::new(Marks {
        priority: 1,
        marks: vec![(0..21, HighlightClass::ExecComment)],
    });
    let service = Rc::new(Highlighting(SmallVec::from_vec(vec![tokens, well])));
    let (e, _, vcx) = open_with(cx, text.to_string(), service);
    e.update(vcx, |v, cx| v.select_range(9..12, cx));
    draw(vcx);
    let layout = vcx.read(|cx| e.read(cx).last_layout().cloned()).unwrap();
    let x = |b: usize| f32::from(layout.x_for_offset(b).unwrap());
    let wells = ordered_quads(vcx, c.seam_soft.to_hsla());
    assert_eq!(wells.len(), 1, "{wells:?}");
    let (well_order, (wx, wy, ww, wh)) = wells[0];
    assert!(close(wx, x(0)) && close(ww, x(21) - x(0)), "{wells:?}");
    assert!(close(wy, 6.0) && close(wh, 18.0), "{wells:?}");
    let sel = ordered_quads(vcx, c.blue_tint_strong.to_hsla());
    assert_eq!(sel.len(), 1, "{sel:?}");
    assert!(well_order < sel[0].0, "the well is under the selection");
    // Outside the comment, no well.
    assert!(wx + ww < x(22));
}
