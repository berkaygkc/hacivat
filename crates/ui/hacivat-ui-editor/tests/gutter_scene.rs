//! M3a Task 28: the gutter's paint, checked on the rendered scene (`scene-check`), against
//! `design/m3a-query/NOTES.md` surfaces 1, 3 and 5: the 1 px `green` executed bar and the 1 px
//! `blue` preview bar at x = 4 (the preview bar at x = 2 beside an executed one), the dashed
//! `text_2` bar of an uncertain boundary, the band's `blue_tint` at .55 (dark) and .8 (light) of
//! its alpha from the gutter's edge to the right edge, the exact tint on a shared line, the
//! glyphs' 7 px squares in the 12 px slot at x = 8, no red for any state, the scrollbar track's
//! markers and its own hit target.
#![cfg_attr(not(feature = "scene-check"), allow(dead_code, unused_imports))]
#![allow(clippy::single_range_in_vec_init, reason = "byte ranges as data")]

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AppContext as _, BorderStyle, Context, Entity, Focusable as _, Hsla, IntoElement, Modifiers,
    MouseButton, ParentElement as _, Render, Rgba, Styled as _, TestAppContext, VisualTestContext,
    Window, div, point, px,
};
use hacivat_text::{ChangeDesc, Snapshot, TokenOracle};
use hacivat_ui_core::theme::ThemeTokens as Tokens;
use hacivat_ui_core::theme::{DARK, LIGHT, Theme, ThemeMode};
use hacivat_ui_editor::preview::PREVIEW_DELAY;
use hacivat_ui_editor::{
    DecorationSource, DocContext, DocId, EditorConfig, EditorView, ExecAction, ExecAt,
    ExecMarkState, ExecPreview, InvalidateFn, LanguageService, MarkKey, NoTokens, PreviewState,
    StatementSpan,
};
use smallvec::SmallVec;

/// A service whose preview the test sets.
#[derive(Default)]
struct Fixed(RefCell<Option<ExecPreview>>);

struct Shared(Rc<Fixed>);

impl LanguageService for Shared {
    fn open(&self, _: DocId, _: &Snapshot, _: DocContext, _: &mut gpui::App) {}
    fn set_context(&self, _: DocId, _: DocContext, _: &mut gpui::App) {}
    fn change(&self, _: DocId, _: &Snapshot, _: &ChangeDesc, _: &mut gpui::App) {}
    fn close(&self, _: DocId) {}
    fn on_update(&self, _: DocId, _: InvalidateFn) {}
    fn statements(&self, _: DocId, _: u64, _: Range<usize>) -> Arc<[StatementSpan]> {
        Arc::from(Vec::new())
    }
    fn exec_preview(&self, _: DocId, _: u64, _: &ExecAt, _: ExecAction) -> Option<ExecPreview> {
        self.0.0.borrow().clone()
    }
    fn token_oracle(&self, _: DocId) -> Rc<dyn TokenOracle> {
        Rc::new(NoTokens)
    }
    fn decoration_sources(&self, _: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
        SmallVec::new()
    }
}

struct Host {
    editor: Entity<EditorView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(800.0)).h(px(600.0)).child(self.editor.clone())
    }
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    mode: ThemeMode,
) -> (Entity<EditorView>, Rc<Fixed>, &'a mut VisualTestContext) {
    cx.update(|cx| Theme::install(cx, mode));
    let service = Rc::new(Fixed::default());
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let (out, svc) = (slot.clone(), service.clone());
    let text = text.to_string();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(28),
                &text,
                EditorConfig::default(),
                Rc::new(Shared(svc)),
                Rc::new(|_: &gpui::App| "Console 1".into()),
                window,
                cx,
            )
        });
        *out.borrow_mut() = Some(editor.clone());
        Host { editor }
    });
    let editor = slot.borrow().clone().unwrap();
    draw(vcx);
    (editor, service, vcx)
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

fn preview(
    e: &Entity<EditorView>,
    svc: &Fixed,
    vcx: &mut VisualTestContext,
    state: PreviewState,
    ranges: &[Range<usize>],
    caret: usize,
) {
    *svc.0.borrow_mut() = Some(ExecPreview {
        ranges: ranges.iter().cloned().collect(),
        state,
        choices: 0,
    });
    e.update(vcx, |v, cx| v.select_range(caret..caret, cx));
    vcx.executor().advance_clock(PREVIEW_DELAY);
    vcx.run_until_parked();
    draw(vcx);
}

fn same(a: Hsla, b: Hsla) -> bool {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    (a.r - b.r).abs() < 1.5 / 255.0
        && (a.g - b.g).abs() < 1.5 / 255.0
        && (a.b - b.b).abs() < 1.5 / 255.0
        && (a.a - b.a).abs() < 1.5 / 255.0
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.3
}

/// A painted quad, logical (x, y, w, h), with its fill and border.
#[derive(Debug, Clone, Copy)]
struct Q {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    fill: Option<Hsla>,
    border: Hsla,
    border_w: f32,
    dashed: bool,
}

#[cfg(feature = "scene-check")]
fn scene(vcx: &mut VisualTestContext) -> Vec<Q> {
    vcx.update(|window, _| {
        let s = window.scale_factor();
        window
            .rendered_scene()
            .quads
            .iter()
            .map(|q| Q {
                x: q.bounds.origin.x.0 / s,
                y: q.bounds.origin.y.0 / s,
                w: q.bounds.size.width.0 / s,
                h: q.bounds.size.height.0 / s,
                fill: q.background.as_solid(),
                border: q.border_color,
                border_w: q.border_widths.left.0.max(q.border_widths.bottom.0) / s,
                dashed: q.border_style == BorderStyle::Dashed,
            })
            .collect()
    })
}

fn filled(qs: &[Q], c: Hsla) -> Vec<Q> {
    qs.iter()
        .filter(|q| q.fill.is_some_and(|f| same(f, c)))
        .copied()
        .collect()
}

/// The band's ink: `blue_tint` with its alpha × .55 (dark) or × .8 (light).
fn band_ink(t: &Tokens, light: bool) -> Hsla {
    let mut c = t.colors.blue_tint.to_hsla();
    c.a *= if light { 0.8 } else { 0.55 };
    c
}

/// Rows are 18 px from y = 6.
fn row_top(line: u32) -> f32 {
    6.0 + 18.0 * line as f32
}

const SQL: &str = "SELECT 1;\nSELECT 2,\n  3;\nSELECT 4; SELECT 5;\n";

#[cfg(feature = "scene-check")]
#[gpui::test]
fn gutter_geometry_and_hues(cx: &mut TestAppContext) {
    let (e, svc, vcx) = open(cx, SQL, ThemeMode::Dark);
    let c = &DARK.colors;
    let v0 = vcx.read(|cx| e.read(cx).snapshot().version);
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v0, vec![(MarkKey(1), 0..9), (MarkKey(2), 10..24)], cx);
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Succeeded {
                took: Duration::from_millis(3),
                rows: None,
            },
            None,
            None,
            cx,
        );
        v.set_exec_state(MarkKey(2), ExecMarkState::Queued, None, None, cx);
    });
    // The preview: the statement on lines 2–3 (index 1..=2), which owns its lines.
    preview(&e, &svc, vcx, PreviewState::Units, &[10..24], 12);
    let qs = scene(vcx);
    // The executed bar: 1 px green at x = 4 over line 0, inset 1 px at each end.
    let green = filled(&qs, c.green.to_hsla());
    assert!(
        green.iter().any(|q| close(q.x, 4.0)
            && close(q.w, 1.0)
            && close(q.y, row_top(0) + 1.0)
            && close(q.h, 16.0)),
        "{green:?}"
    );
    // The preview bar: 1 px blue at x = 4 over lines 1..=2.
    let blue = filled(&qs, c.blue.to_hsla());
    assert!(
        blue.iter().any(|q| close(q.x, 4.0)
            && close(q.w, 1.0)
            && close(q.y, row_top(1))
            && close(q.h, 36.0)),
        "{blue:?}"
    );
    // The full-row band from the gutter's edge (58) to the right edge, on both lines.
    let band = filled(&qs, band_ink(&DARK, false));
    for line in [1, 2] {
        assert!(
            band.iter().any(|q| close(q.x, 58.0)
                && close(q.w, 800.0 - 58.0)
                && close(q.y, row_top(line))
                && close(q.h, 18.0)),
            "line {line}: {band:?}"
        );
    }
    // The queued glyph: a 7 px hollow seam_strong square centred in the 12 px slot at x = 8.
    let queued: Vec<&Q> = qs
        .iter()
        .filter(|q| close(q.w, 7.0) && close(q.h, 7.0) && same(q.border, c.seam_strong.to_hsla()))
        .collect();
    assert!(
        queued
            .iter()
            .any(|q| close(q.x, 8.0 + 2.5) && close(q.y, row_top(1) + 5.5) && q.border_w > 0.5),
        "{queued:?}"
    );

    // The preview over the executed statement: its bar moves to x = 2.
    preview(&e, &svc, vcx, PreviewState::Units, &[0..8], 3);
    let blue = filled(&scene(vcx), c.blue.to_hsla());
    assert!(
        blue.iter()
            .any(|q| close(q.x, 2.0) && close(q.w, 1.0) && close(q.y, row_top(0))),
        "{blue:?}"
    );

    // Two statements on line 3: the exact tint on the second's characters, no terminator.
    preview(&e, &svc, vcx, PreviewState::Units, &[33..42], 35);
    let (x0, x1) = vcx.read(|cx| {
        let l = e.read(cx).last_layout().unwrap();
        (
            f32::from(l.x_for_offset(33).unwrap()),
            f32::from(l.x_for_offset(41).unwrap()),
        )
    });
    let band = filled(&scene(vcx), band_ink(&DARK, false));
    assert!(
        band.iter()
            .any(|q| close(q.x, x0) && close(q.w, x1 - x0) && close(q.y, row_top(3))),
        "exact tint {x0}..{x1}: {band:?}"
    );
    assert!(
        !band
            .iter()
            .any(|q| close(q.x, 58.0) && close(q.y, row_top(3))),
        "no full row on a shared line"
    );

    // Uncertain: the neutral fill, a dashed 1 px text_2 bar at x = 4.
    preview(&e, &svc, vcx, PreviewState::Uncertain, &[10..24], 12);
    let qs = scene(vcx);
    assert!(
        filled(&qs, c.hover.to_hsla())
            .iter()
            .any(|q| close(q.x, 58.0) && close(q.y, row_top(1))),
        "the neutral band"
    );
    assert!(
        qs.iter().any(|q| q.dashed
            && same(q.border, c.text_2.to_hsla())
            && close(q.x, 4.0)
            && close(q.w, 1.0)),
        "the dashed bar"
    );

    // Every state, never red: running (after 50 ms), failed, cancelling, cancelled, skipped.
    let now = vcx.executor().now();
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v0, vec![(MarkKey(3), 33..42)], cx);
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Running { started: now },
            None,
            None,
            cx,
        );
        v.set_exec_state(
            MarkKey(2),
            ExecMarkState::Failed {
                sqlstate: Some("42601".into()),
            },
            None,
            None,
            cx,
        );
        v.set_exec_state(
            MarkKey(3),
            ExecMarkState::Cancelling { since: now },
            None,
            None,
            cx,
        );
    });
    vcx.executor().advance_clock(Duration::from_millis(60));
    vcx.run_until_parked();
    draw(vcx);
    let qs = scene(vcx);
    let amber = c.amber.to_hsla();
    assert!(
        filled(&qs, amber)
            .iter()
            .any(|q| close(q.w, 7.0) && close(q.x, 10.5) && close(q.y, row_top(0) + 5.5)),
        "the amber run marker"
    );
    assert!(
        qs.iter().any(|q| close(q.w, 7.0)
            && same(q.border, amber)
            && q.fill.is_none_or(|f| f.a == 0.0)
            && close(q.y, row_top(3) + 5.5)),
        "the hollow amber square"
    );
    let reds = [c.red, c.red_line, c.red_tint, c.cp_fill].map(|r| r.to_hsla());
    for q in &qs {
        assert!(
            !q.fill.is_some_and(|f| reds.iter().any(|r| same(f, *r)))
                && !(q.border_w > 0.0 && reds.iter().any(|r| same(q.border, *r))),
            "a red quad: {q:?}"
        );
    }
}

/// Light theme: the band's alpha is .8 of `blue_tint`'s.
#[cfg(feature = "scene-check")]
#[gpui::test]
fn the_band_is_lighter_in_the_light_theme(cx: &mut TestAppContext) {
    let (e, svc, vcx) = open(cx, SQL, ThemeMode::Light);
    preview(&e, &svc, vcx, PreviewState::Units, &[0..8], 3);
    let band = filled(&scene(vcx), band_ink(&LIGHT, true));
    assert!(
        band.iter()
            .any(|q| close(q.x, 58.0) && close(q.y, row_top(0))),
        "{band:?}"
    );
    assert!(filled(&scene(vcx), band_ink(&LIGHT, false)).is_empty());
}

/// Past three digits the gutter grows by one advance per digit, and the line numbers stay
/// right-aligned 14 px from its edge: the band starts at the new gutter edge.
#[cfg(feature = "scene-check")]
#[gpui::test]
fn a_five_digit_gutter_is_wider(cx: &mut TestAppContext) {
    let text: String = (0..10_000).map(|i| format!("SELECT {i};\n")).collect();
    let (e, svc, vcx) = open(cx, &text, ThemeMode::Dark);
    preview(&e, &svc, vcx, PreviewState::Units, &[0..9], 3);
    let (left, advance) = vcx.read(|cx| {
        let l = e.read(cx).last_layout().unwrap();
        let x = |b: usize| f32::from(l.x_for_offset(b).unwrap());
        (f32::from(l.rows().bounds.left()), x(1) - x(0))
    });
    assert!(close(left, 58.0 + 2.0 * advance), "{left} {advance}");
    let band = filled(&scene(vcx), band_ink(&DARK, false));
    assert!(band.iter().any(|q| close(q.x, left)), "{band:?}");
}

/// The scrollbar track has its own hit target: a click there places no caret (the text runs
/// under it), and the markers show the marks and the current statement in a long document.
#[cfg(feature = "scene-check")]
#[gpui::test]
fn the_track_takes_its_own_clicks_and_shows_markers(cx: &mut TestAppContext) {
    let text: String = (0..400).map(|i| format!("SELECT {i};\n")).collect();
    let (e, svc, vcx) = open(cx, &text, ThemeMode::Dark);
    let c = &DARK.colors;
    let v0 = vcx.read(|cx| e.read(cx).snapshot().version);
    let at = text.find("SELECT 200;").unwrap();
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v0, vec![(MarkKey(1), at..at + 11)], cx);
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Succeeded {
                took: Duration::from_millis(1),
                rows: None,
            },
            None,
            None,
            cx,
        );
    });
    preview(&e, &svc, vcx, PreviewState::Units, &[0..10], 3);
    let qs = scene(vcx);
    // The track is the right 8 px: a 3 px green marker at half its height, a 2 px blue one at
    // its top.
    let green: Vec<Q> = filled(&qs, c.green.to_hsla())
        .into_iter()
        .filter(|q| q.x >= 792.0)
        .collect();
    assert!(
        green
            .iter()
            .any(|q| close(q.x, 796.0) && close(q.w, 3.0) && (q.y - 300.0).abs() < 3.0),
        "{green:?}"
    );
    assert!(
        filled(&qs, c.blue.to_hsla())
            .iter()
            .any(|q| close(q.x, 793.0) && close(q.w, 2.0) && q.y < 3.0),
        "the current statement's marker"
    );
    // A click on the track over the first line's text leaves the caret where it was.
    let before = vcx.read(|cx| e.read(cx).snapshot().selection.main().head);
    e.update_in(vcx, |v, window, cx| {
        let f = v.focus_handle(cx);
        window.focus(&f, cx);
    });
    vcx.simulate_click(point(px(796.0), px(row_top(0) + 9.0)), Modifiers::none());
    vcx.run_until_parked();
    let after = vcx.read(|cx| e.read(cx).snapshot().selection.main().head);
    assert_eq!(after, before, "the track placed a caret");
    // A click left of the track, on the same row, does.
    vcx.simulate_click(point(px(70.0), px(row_top(1) + 9.0)), Modifiers::none());
    vcx.run_until_parked();
    let moved = vcx.read(|cx| e.read(cx).snapshot().selection.main().head);
    assert_ne!(moved, before);
    let _ = MouseButton::Left;
}

// ## m3a-t28-gutter (fix round 1)

/// Separate units: one blue bar per unit, each inset 1 px at both ends (a 2 px gap between
/// neighbours, also beside an executed bar at x = 2), a 1 px `panel` separator on the top edge of
/// each later unit's band; no connection: a solid 1 px `text_3` bar; an uncertain exact tint: a
/// dashed `text_3` lower edge. Review round 1, Minor 4.
#[cfg(feature = "scene-check")]
#[gpui::test]
fn separate_units_and_neutral_states_draw_their_bars(cx: &mut TestAppContext) {
    let text = "SELECT 1;\nSELECT 2;\nSELECT 3;\nSELECT 4; SELECT 5;\n";
    let (e, svc, vcx) = open(cx, text, ThemeMode::Dark);
    let c = &DARK.colors;
    let v0 = vcx.read(|cx| e.read(cx).snapshot().version);
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v0, vec![(MarkKey(1), 10..19)], cx);
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Succeeded {
                took: Duration::from_millis(1),
                rows: None,
            },
            None,
            None,
            cx,
        );
    });
    preview(
        &e,
        &svc,
        vcx,
        PreviewState::Units,
        &[0..8, 10..18, 20..28],
        3,
    );
    let qs = scene(vcx);
    let blue = filled(&qs, c.blue.to_hsla());
    for (line, x) in [(0, 4.0), (1, 2.0), (2, 4.0)] {
        assert!(
            blue.iter().any(|q| close(q.x, x)
                && close(q.w, 1.0)
                && close(q.y, row_top(line) + 1.0)
                && close(q.h, 16.0)),
            "unit on line {line} at x = {x}: {blue:?}"
        );
    }
    let separators: Vec<Q> = filled(&qs, c.panel.to_hsla())
        .into_iter()
        .filter(|q| close(q.h, 1.0) && close(q.x, 58.0))
        .collect();
    for line in [1, 2] {
        assert!(
            separators.iter().any(|q| close(q.y, row_top(line))),
            "separator on line {line}: {separators:?}"
        );
    }
    assert!(!separators.iter().any(|q| close(q.y, row_top(0))));

    preview(&e, &svc, vcx, PreviewState::NoConnection, &[20..28], 22);
    let qs = scene(vcx);
    assert!(
        filled(&qs, c.text_3.to_hsla())
            .iter()
            .any(|q| close(q.x, 4.0)
                && close(q.w, 1.0)
                && close(q.y, row_top(2))
                && close(q.h, 18.0)),
        "the solid text_3 bar"
    );
    assert!(
        filled(&qs, c.hover.to_hsla())
            .iter()
            .any(|q| close(q.x, 58.0) && close(q.y, row_top(2))),
        "the neutral band"
    );

    // "SELECT 5" shares line 3 with "SELECT 4;": the exact tint, its lower edge dashed.
    preview(&e, &svc, vcx, PreviewState::Uncertain, &[40..48], 42);
    let qs = scene(vcx);
    assert!(
        qs.iter().any(|q| q.dashed
            && same(q.border, c.text_3.to_hsla())
            && close(q.h, 1.0)
            && close(q.y, row_top(3) + 17.0)),
        "the dashed lower edge"
    );
}

/// The scrollbar keeps the gutter's 50 ms grace (no amber flash for a fast statement), and
/// marks that land on the same pixel rows merge into one marker in the worse hue. Review
/// round 1, Minor 5.
#[cfg(feature = "scene-check")]
#[gpui::test]
fn scrollbar_markers_keep_the_grace_and_merge(cx: &mut TestAppContext) {
    let text: String = (0..400).map(|i| format!("SELECT {i};\n")).collect();
    let (e, _svc, vcx) = open(cx, &text, ThemeMode::Dark);
    let c = &DARK.colors;
    let v0 = vcx.read(|cx| e.read(cx).snapshot().version);
    let at = |n: usize| {
        let s = text.find(&format!("SELECT {n};")).unwrap();
        s..s + format!("SELECT {n};").len()
    };
    let now = vcx.executor().now();
    e.update(vcx, |v, cx| {
        v.add_exec_marks(
            v0,
            vec![
                (MarkKey(1), at(200)),
                (MarkKey(2), at(201)),
                (MarkKey(3), at(300)),
            ],
            cx,
        );
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Succeeded {
                took: Duration::from_millis(1),
                rows: None,
            },
            None,
            None,
            cx,
        );
        v.set_exec_state(
            MarkKey(2),
            ExecMarkState::Failed { sqlstate: None },
            None,
            None,
            cx,
        );
        v.set_exec_state(
            MarkKey(3),
            ExecMarkState::Running { started: now },
            None,
            None,
            cx,
        );
    });
    draw(vcx);
    let in_track = |qs: &[Q], color: Hsla| -> Vec<Q> {
        filled(qs, color)
            .into_iter()
            .filter(|q| q.x >= 792.0)
            .collect()
    };
    let qs = scene(vcx);
    assert!(
        in_track(&qs, c.green.to_hsla()).is_empty(),
        "the success under the failure"
    );
    let ink = in_track(&qs, c.text.to_hsla());
    assert_eq!(ink.len(), 1, "one merged marker: {ink:?}");
    assert!((ink[0].y - 300.0).abs() < 3.0, "{ink:?}");
    assert!(
        in_track(&qs, c.amber.to_hsla()).is_empty(),
        "running for under 50 ms"
    );
    vcx.executor().advance_clock(Duration::from_millis(60));
    vcx.run_until_parked();
    draw(vcx);
    assert_eq!(in_track(&scene(vcx), c.amber.to_hsla()).len(), 1);
}
