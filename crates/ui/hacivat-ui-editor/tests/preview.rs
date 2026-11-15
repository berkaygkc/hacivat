//! M3a Task 28: the Run preview band (UI-EDIT-21): asked 50 ms after a change or a move, drawn
//! only from an answer of the text's own version, every state of the verdict, the chooser's
//! override, the status bar's sentence. Time moves through the fake clock only.
//!
//! The composition freeze is a crate test (`preview::tests`), because opening a composition
//! needs the IME core, which the public input handler reaches only with Task 27.

#![allow(clippy::single_range_in_vec_init, reason = "byte ranges as data")]

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div, px,
};
use hacivat_text::{ChangeDesc, Snapshot, TokenOracle};
use hacivat_ui::text::TextRegistry;
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::preview::{self, BandStyle, PREVIEW_DELAY};
use hacivat_ui_editor::{
    DecorationSource, DocContext, DocId, EditorConfig, EditorView, ExecAction, ExecAt, ExecPreview,
    InvalidateFn, LanguageService, NoTokens, PreviewState, StatementSpan,
};
use smallvec::{SmallVec, smallvec};

/// A service whose answer the test sets; it counts the questions and records the version and
/// the place of the last one.
#[derive(Default)]
struct Scripted {
    answer: RefCell<Option<ExecPreview>>,
    asked: Cell<u32>,
    last: RefCell<Option<(u64, ExecAt)>>,
}

impl Scripted {
    fn answer(&self, state: PreviewState, ranges: &[Range<usize>]) {
        *self.answer.borrow_mut() = Some(ExecPreview {
            ranges: ranges.iter().cloned().collect(),
            state,
        });
    }
}

struct Shared(Rc<Scripted>);

impl LanguageService for Shared {
    fn open(&self, _: DocId, _: &Snapshot, _: DocContext, _: &mut gpui::App) {}
    fn set_context(&self, _: DocId, _: DocContext, _: &mut gpui::App) {}
    fn change(&self, _: DocId, _: &Snapshot, _: &ChangeDesc, _: &mut gpui::App) {}
    fn close(&self, _: DocId) {}
    fn on_update(&self, _: DocId, _: InvalidateFn) {}
    fn statements(&self, _: DocId, _: u64, _: Range<usize>) -> Arc<[StatementSpan]> {
        Arc::from(Vec::new())
    }
    fn exec_preview(
        &self,
        _: DocId,
        version: u64,
        at: &ExecAt,
        action: ExecAction,
    ) -> Option<ExecPreview> {
        assert_eq!(action, ExecAction::Execute1, "the band previews Run");
        self.0.asked.set(self.0.asked.get() + 1);
        *self.0.last.borrow_mut() = Some((version, at.clone()));
        self.0.answer.borrow().clone()
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
        div().w(px(800.0)).h(px(400.0)).child(self.editor.clone())
    }
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    service: Rc<Scripted>,
) -> (Entity<EditorView>, &'a mut VisualTestContext) {
    open_with(cx, text, Rc::new(Shared(service)))
}

fn open_with<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    service: Rc<dyn LanguageService>,
) -> (Entity<EditorView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
    });
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let out = slot.clone();
    let text = text.to_string();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(28),
                &text,
                EditorConfig::default(),
                service,
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
    (editor, vcx)
}

fn draw(vcx: &mut VisualTestContext) {
    // A frame that draws no label leaves the registry as it was: start each one empty.
    vcx.update(|window, cx| {
        cx.global_mut::<TextRegistry>().frame.clear();
        window.refresh();
    });
    vcx.run_until_parked();
}

/// Moves the caret to `at` and lets the 50 ms timer fire, then draws.
fn caret_at(e: &Entity<EditorView>, vcx: &mut VisualTestContext, at: usize) {
    e.update(vcx, |v, cx| v.select_range(at..at, cx));
    vcx.executor().advance_clock(PREVIEW_DELAY);
    vcx.run_until_parked();
    draw(vcx);
}

fn band(e: &Entity<EditorView>, vcx: &VisualTestContext) -> Option<preview::BandPlan> {
    vcx.read(|cx| preview::current(e.read(cx)))
}

fn drawn(vcx: &VisualTestContext) -> Vec<String> {
    vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .map(|t| t.text.to_string())
            .collect()
    })
}

const SQL: &str = "SELECT 1;\nSELECT 2;\nSELECT 3;\n";

/// Each verdict draws its own band, hint and status: `Units` (one, and several with "i of n"),
/// `AskPreselected` as the candidate Enter would run, `Uncertain` neutral with "boundary
/// uncertain" and the status sentence, `NoConnection` neutral with "not connected", `Nothing`
/// no band and "Nothing to run here".
#[gpui::test]
fn every_preview_state_renders(cx: &mut TestAppContext) {
    let service = Rc::new(Scripted::default());
    let (e, vcx) = open(cx, SQL, service.clone());
    // No question before a change or a move, and none sooner than 50 ms after one.
    assert_eq!(service.asked.get(), 0);
    service.answer(PreviewState::Units, &[0..8]);
    e.update(vcx, |v, cx| v.select_range(3..3, cx));
    vcx.executor().advance_clock(PREVIEW_DELAY / 2);
    vcx.run_until_parked();
    assert_eq!(service.asked.get(), 0, "asked before 50 ms");
    vcx.executor().advance_clock(PREVIEW_DELAY / 2);
    vcx.run_until_parked();
    assert_eq!(service.asked.get(), 1);
    let (version, at) = service.last.borrow().clone().unwrap();
    assert_eq!(version, vcx.read(|cx| e.read(cx).snapshot().version));
    assert_eq!(at, ExecAt::Carets(smallvec![3]));
    draw(vcx);
    let p = band(&e, vcx).expect("a band");
    assert_eq!(
        (p.style, p.units.to_vec(), p.from_override),
        (BandStyle::Run, vec![0..8], false)
    );
    let status = vcx.read(|cx| preview::status_message(e.read(cx), cx));
    assert_eq!(status, None);

    // A selection is asked as a selection; several units say "i of n".
    service.answer(PreviewState::Units, &[0..8, 10..18, 20..28]);
    e.update(vcx, |v, cx| v.select_range(0..28, cx));
    vcx.executor().advance_clock(PREVIEW_DELAY);
    vcx.run_until_parked();
    draw(vcx);
    assert_eq!(
        service.last.borrow().clone().unwrap().1,
        ExecAt::Selections(smallvec![0..28])
    );
    assert_eq!(band(&e, vcx).unwrap().units.len(), 3);
    let texts = drawn(vcx);
    assert_eq!(
        texts
            .iter()
            .filter(|t| t.as_str() == "hv-editor-preview-unit")
            .count(),
        3,
        "{texts:?}"
    );

    // Ask, preselected: the candidate Enter would run, drawn like Units.
    service.answer(PreviewState::AskPreselected, &[10..18]);
    caret_at(&e, vcx, 12);
    let p = band(&e, vcx).unwrap();
    assert_eq!((p.style, p.units.to_vec()), (BandStyle::Run, vec![10..18]));

    // Uncertain: neutral, the hint, the status sentence.
    service.answer(PreviewState::Uncertain, &[0..28]);
    caret_at(&e, vcx, 13);
    assert_eq!(band(&e, vcx).unwrap().style, BandStyle::Uncertain);
    assert!(drawn(vcx).contains(&"hv-editor-preview-uncertain".to_string()));
    let status = vcx.read(|cx| preview::status_message(e.read(cx), cx));
    assert_eq!(status.as_deref(), Some("hv-editor-uncertain"));

    // No connection: neutral, "not connected".
    service.answer(PreviewState::NoConnection, &[10..18]);
    caret_at(&e, vcx, 14);
    assert_eq!(band(&e, vcx).unwrap().style, BandStyle::Offline);
    assert!(drawn(vcx).contains(&"hv-editor-preview-offline".to_string()));

    // Nothing: no band; the status bar says so.
    service.answer(PreviewState::Nothing, &[]);
    caret_at(&e, vcx, 9);
    assert_eq!(band(&e, vcx), None);
    let status = vcx.read(|cx| preview::status_message(e.read(cx), cx));
    assert_eq!(status.as_deref(), Some("hv-editor-nothing-to-run"));
    let texts = drawn(vcx);
    assert!(
        !texts.iter().any(|t| t.starts_with("hv-editor-preview")),
        "{texts:?}"
    );

    // The uncertain stub (Task 1b's fail-closed service) has no range: no band.
    service.answer(PreviewState::Uncertain, &[]);
    caret_at(&e, vcx, 2);
    assert_eq!(band(&e, vcx), None);
}

/// The band is never drawn from an answer about other text: after an edit it is gone until the
/// service answers at the new version, a `None` (no analysis of this version yet) draws none,
/// and a range that is not of this text (past its end, inside a character) draws none.
#[gpui::test]
fn stale_service_answer_draws_no_band(cx: &mut TestAppContext) {
    let service = Rc::new(Scripted::default());
    let (e, vcx) = open(cx, "SELECT 'ş';\nSELECT 2;\n", service.clone());
    service.answer(PreviewState::Units, &[0..11]);
    caret_at(&e, vcx, 3);
    assert!(band(&e, vcx).is_some());
    // Type: the answer is of the old version, so no band, even before the timer fires.
    e.update_in(vcx, |v, window, cx| {
        gpui::EntityInputHandler::replace_text_in_range(v, None, "x", window, cx);
    });
    draw(vcx);
    assert_eq!(band(&e, vcx), None, "the old version's band");
    // The service has no analysis of the new version yet.
    *service.answer.borrow_mut() = None;
    vcx.executor().advance_clock(PREVIEW_DELAY);
    vcx.run_until_parked();
    draw(vcx);
    assert_eq!(band(&e, vcx), None);
    // A range inside 'ş' (now bytes 9..11) or past the end is refused.
    service.answer(PreviewState::Units, &[0..10]);
    caret_at(&e, vcx, 2);
    assert_eq!(band(&e, vcx), None, "a range off a character boundary");
    service.answer(PreviewState::Units, &[0..400]);
    caret_at(&e, vcx, 1);
    assert_eq!(band(&e, vcx), None, "a range past the end");
    // A fresh answer at this version draws again.
    service.answer(PreviewState::Units, &[0..13]);
    caret_at(&e, vcx, 3);
    assert_eq!(band(&e, vcx).unwrap().units.to_vec(), vec![0..13]);
}

/// The chooser's hover replaces the service's band, at once and whatever it answers; clearing
/// it brings the answer back.
#[gpui::test]
fn the_chooser_override_replaces_the_band(cx: &mut TestAppContext) {
    let service = Rc::new(Scripted::default());
    let (e, vcx) = open(cx, SQL, service.clone());
    service.answer(PreviewState::AskPreselected, &[0..8]);
    caret_at(&e, vcx, 3);
    e.update(vcx, |v, cx| {
        v.set_preview_override(Some(smallvec![10..18]), cx)
    });
    let p = band(&e, vcx).unwrap();
    assert_eq!(
        (p.style, p.units.to_vec(), p.from_override),
        (BandStyle::Run, vec![10..18], true)
    );
    service.answer(PreviewState::Nothing, &[]);
    caret_at(&e, vcx, 4);
    assert_eq!(band(&e, vcx).unwrap().units.to_vec(), vec![10..18]);
    e.update(vcx, |v, cx| v.set_preview_override(None, cx));
    assert_eq!(band(&e, vcx), None);
}

// ## m3a-t28-gutter (fix round 1): a service that reads where it is asked.

/// The statement around the first caret (split on `;`), as a fresh analysis answers: `Units`
/// when connected, `NoConnection` otherwise, `Nothing` between statements; a selection is
/// answered as itself.
#[derive(Default)]
struct Around {
    text: RefCell<String>,
    connected: Cell<bool>,
}

impl Around {
    fn statement(&self, caret: usize) -> Option<Range<usize>> {
        let text = self.text.borrow();
        let caret = caret.min(text.len());
        let start = text[..caret].rfind(';').map_or(0, |i| i + 1);
        let end = text[caret..].find(';').map_or(text.len(), |i| caret + i);
        let lead = text[start..end].len() - text[start..end].trim_start().len();
        let r = start + lead..start + text[start..end].trim_end().len();
        (r.start < r.end).then_some(r)
    }
}

struct AroundService(Rc<Around>);

impl LanguageService for AroundService {
    fn open(&self, _: DocId, snap: &Snapshot, ctx: DocContext, _: &mut gpui::App) {
        *self.0.text.borrow_mut() = snap.buffer.slice(0..snap.buffer.len_bytes()).into_owned();
        self.0.connected.set(ctx.connected);
    }
    fn set_context(&self, _: DocId, ctx: DocContext, _: &mut gpui::App) {
        self.0.connected.set(ctx.connected);
    }
    fn change(&self, _: DocId, snap: &Snapshot, _: &ChangeDesc, _: &mut gpui::App) {
        *self.0.text.borrow_mut() = snap.buffer.slice(0..snap.buffer.len_bytes()).into_owned();
    }
    fn close(&self, _: DocId) {}
    fn on_update(&self, _: DocId, _: InvalidateFn) {}
    fn statements(&self, _: DocId, _: u64, _: Range<usize>) -> Arc<[StatementSpan]> {
        Arc::from(Vec::new())
    }
    fn exec_preview(&self, _: DocId, _: u64, at: &ExecAt, _: ExecAction) -> Option<ExecPreview> {
        let state = if self.0.connected.get() {
            PreviewState::Units
        } else {
            PreviewState::NoConnection
        };
        let ranges: SmallVec<[Range<usize>; 2]> = match at {
            ExecAt::Carets(c) => c
                .first()
                .and_then(|&c| self.0.statement(c))
                .into_iter()
                .collect(),
            ExecAt::Selections(s) => s.iter().cloned().collect(),
        };
        Some(if ranges.is_empty() {
            ExecPreview {
                ranges,
                state: PreviewState::Nothing,
            }
        } else {
            ExecPreview { ranges, state }
        })
    }
    fn token_oracle(&self, _: DocId) -> Rc<dyn TokenOracle> {
        Rc::new(NoTokens)
    }
    fn decoration_sources(&self, _: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
        SmallVec::new()
    }
}

const THREE: &str = "SELECT 1;\nSELECT 2;\nSELECT 3;\n\n";

fn status(e: &Entity<EditorView>, vcx: &VisualTestContext) -> Option<String> {
    vcx.read(|cx| preview::status_message(e.read(cx), cx).map(|s| s.to_string()))
}

/// The band follows the caret (NOTES surface 3, Keyboard): right after a move, before the
/// 50 ms answer, nothing from the old position is drawn (no band, no gutter bar, no scrollbar
/// marker, no status sentence), also through a key repeat faster than the timer; then the new
/// position's statement. (Review round 1, Critical 1.)
#[gpui::test]
fn a_caret_move_hides_the_old_band_until_the_new_answer(cx: &mut TestAppContext) {
    let (e, vcx) = open_with(cx, THREE, Rc::new(AroundService(Rc::default())));
    caret_at(&e, vcx, 3);
    assert_eq!(band(&e, vcx).unwrap().units.to_vec(), vec![0..8]);
    e.update(vcx, |v, cx| v.select_range(13..13, cx));
    draw(vcx);
    assert_eq!(
        band(&e, vcx),
        None,
        "the first statement's band at the second's caret"
    );
    // Key repeat: three moves 30 ms apart, the timer never fires.
    for at in [23, 14, 24] {
        e.update(vcx, |v, cx| v.select_range(at..at, cx));
        vcx.executor().advance_clock(PREVIEW_DELAY * 3 / 5);
        vcx.run_until_parked();
        assert_eq!(band(&e, vcx), None, "at {at}");
    }
    vcx.executor().advance_clock(PREVIEW_DELAY);
    vcx.run_until_parked();
    assert_eq!(band(&e, vcx).unwrap().units.to_vec(), vec![20..28]);
    // The status sentence follows the caret too.
    caret_at(&e, vcx, THREE.len());
    assert_eq!(status(&e, vcx).as_deref(), Some("hv-editor-nothing-to-run"));
    e.update(vcx, |v, cx| v.select_range(3..3, cx));
    assert_eq!(
        status(&e, vcx),
        None,
        "the blank line's sentence at a statement"
    );
    assert_eq!(band(&e, vcx), None);
}

/// A new document context drops the answer asked under the old one, even when the caret went
/// back to where it was asked and a timer is pending: "not connected" never survives a connect.
#[gpui::test]
fn a_context_change_drops_the_old_answer(cx: &mut TestAppContext) {
    let (e, vcx) = open_with(cx, THREE, Rc::new(AroundService(Rc::default())));
    caret_at(&e, vcx, 3);
    assert_eq!(band(&e, vcx).unwrap().style, BandStyle::Offline);
    e.update(vcx, |v, cx| {
        v.select_range(4..4, cx);
        v.select_range(3..3, cx);
        v.set_doc_context(
            DocContext {
                connected: true,
                tier: hacivat_ui_editor::Tier::Full,
            },
            cx,
        );
    });
    assert_eq!(
        band(&e, vcx),
        None,
        "the not-connected answer after a connect"
    );
    vcx.executor().advance_clock(PREVIEW_DELAY);
    vcx.run_until_parked();
    assert_eq!(band(&e, vcx).unwrap().style, BandStyle::Run);
}

/// The chooser's override is of the text it was hovered on: a text change clears it.
/// (Review round 1, Important 3.)
#[gpui::test]
fn a_text_change_clears_the_chooser_override(cx: &mut TestAppContext) {
    let service = Rc::new(Scripted::default());
    let (e, vcx) = open(cx, SQL, service.clone());
    service.answer(PreviewState::Units, &[0..8]);
    caret_at(&e, vcx, 3);
    e.update(vcx, |v, cx| {
        v.set_preview_override(Some(smallvec![10..18]), cx)
    });
    assert!(band(&e, vcx).unwrap().from_override);
    e.update_in(vcx, |v, window, cx| {
        gpui::EntityInputHandler::replace_text_in_range(v, None, "x", window, cx);
    });
    assert_eq!(band(&e, vcx), None, "the override at old offsets");
    vcx.executor().advance_clock(PREVIEW_DELAY);
    vcx.run_until_parked();
    let p = band(&e, vcx).unwrap();
    assert_eq!((p.units.to_vec(), p.from_override), (vec![0..8], false));
}
