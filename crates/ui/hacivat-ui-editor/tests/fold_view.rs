//! M3c Task 32: folding in the editor (UI-EDIT-10; NOTES surface 15): the gutter's toggles and
//! the palette commands fold the language service's ranges; folded lines take no row, leave
//! the accessibility runs and open when a caret lands in them; a vertical move steps over a
//! fold; a mark inside a fold paints on its header with the order that most needs the user;
//! Run at a caret on a folded statement's header previews the whole statement.

#![allow(
    clippy::single_range_in_vec_init,
    clippy::cloned_ref_to_slice_refs,
    reason = "byte ranges as data"
)]

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div, px,
};
use hacivat_text::{ChangeDesc, FoldKind, FoldRange, Snapshot, TokenOracle};
use hacivat_ui::text::{TextFn, TextRegistry};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::{
    DecorationSource, DocContext, DocId, EditorConfig, EditorView, ExecAction, ExecAt,
    ExecMarkState, ExecPreview, InvalidateFn, LanguageService, MarkKey, NoTokens, PreviewState,
    StatementSpan, a11y, fold_gutter, preview,
};
use smallvec::SmallVec;

/// Folds the ranges it is given; previews the statement holding the first caret.
#[derive(Default)]
struct Svc {
    folds: Vec<FoldRange>,
    statements: Vec<Range<usize>>,
    /// Answers no folding range until set (an analysis that lands late, fix round 1 I1).
    late: Option<Rc<Cell<bool>>>,
    /// The editor's repaint callback (`on_update`).
    notify: Rc<RefCell<Option<InvalidateFn>>>,
}

impl LanguageService for Svc {
    fn open(&self, _: DocId, _: &Snapshot, _: DocContext, _: &mut gpui::App) {}
    fn set_context(&self, _: DocId, _: DocContext, _: &mut gpui::App) {}
    fn change(&self, _: DocId, _: &Snapshot, _: &ChangeDesc, _: &mut gpui::App) {}
    fn close(&self, _: DocId) {}
    fn on_update(&self, _: DocId, notify: InvalidateFn) {
        *self.notify.borrow_mut() = Some(notify);
    }
    fn statements(&self, _: DocId, _: u64, _: Range<usize>) -> Arc<[StatementSpan]> {
        Arc::from(Vec::new())
    }
    fn exec_preview(&self, _: DocId, _: u64, at: &ExecAt, _: ExecAction) -> Option<ExecPreview> {
        let ranges: SmallVec<[Range<usize>; 2]> = match at {
            ExecAt::Carets(c) => c
                .first()
                .and_then(|&c| {
                    self.statements
                        .iter()
                        .find(|s| s.start <= c && c <= s.end)
                        .cloned()
                })
                .into_iter()
                .collect(),
            ExecAt::Selections(s) => s.iter().cloned().collect(),
        };
        let state = if ranges.is_empty() {
            PreviewState::Nothing
        } else {
            PreviewState::Units
        };
        Some(ExecPreview {
            ranges,
            state,
            choices: 0,
            as_one: 0, // ## m3c-t34-editor (named exception)
        })
    }
    fn token_oracle(&self, _: DocId) -> Rc<dyn TokenOracle> {
        Rc::new(NoTokens)
    }
    fn decoration_sources(&self, _: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
        SmallVec::new()
    }
    fn folding_ranges(&self, _: DocId, _: u64) -> Arc<[FoldRange]> {
        if self.late.as_ref().is_some_and(|l| !l.get()) {
            return Arc::from([]);
        }
        self.folds.clone().into()
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
    svc: Svc,
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
    let svc = Rc::new(svc);
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(32),
                &text,
                EditorConfig::default(),
                svc,
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
    vcx.update(|window, cx| {
        cx.global_mut::<TextRegistry>().frame.clear();
        window.refresh();
    });
    vcx.run_until_parked();
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

/// The buffer lines the last frame painted (0-based).
fn painted(e: &Entity<EditorView>, vcx: &VisualTestContext) -> Vec<u32> {
    vcx.read(|cx| {
        e.read(cx)
            .last_layout()
            .map(|l| l.painted_lines())
            .unwrap_or_default()
    })
}

fn caret_at(e: &Entity<EditorView>, vcx: &mut VisualTestContext, at: usize) {
    e.update(vcx, |v, cx| v.select_range(at..at, cx));
    vcx.executor().advance_clock(Duration::from_millis(100));
    vcx.run_until_parked();
    draw(vcx);
}

fn head(e: &Entity<EditorView>, vcx: &VisualTestContext) -> usize {
    vcx.read(|cx| e.read(cx).snapshot().selection.main().head)
}

fn folded(e: &Entity<EditorView>, vcx: &VisualTestContext) -> Vec<Range<usize>> {
    vcx.read(|cx| e.read(cx).folds().folded().to_vec())
}

fn run(e: &Entity<EditorView>, vcx: &mut VisualTestContext, id: &str) {
    let ran = e.update_in(vcx, |v, window, cx| v.run_command(id, window, cx));
    assert!(ran, "{id}");
    draw(vcx);
}

fn at(text: &str, needle: &str) -> usize {
    text.find(needle).unwrap_or_else(|| panic!("{needle:?}"))
}

fn text_fn(s: &'static str) -> Option<TextFn> {
    Some(Rc::new(move |_: &gpui::App| s.into()))
}

/// Lines (0-based): 0 the statement's header, 1–3 hidden when it folds, 4 the next statement.
const SCRIPT: &str = "SELECT id,\n  total,\n  owner\nFROM orders;\nSELECT 2;\n";

fn script_svc() -> Svc {
    let stmt = 0..at(SCRIPT, "orders;") + 7;
    let two = at(SCRIPT, "SELECT 2")..at(SCRIPT, "SELECT 2") + 9;
    Svc {
        folds: vec![FoldRange {
            range: stmt.clone(),
            kind: FoldKind::Statement,
        }],
        statements: vec![stmt, two],
        ..Svc::default()
    }
}

#[gpui::test]
fn a_gutter_toggle_folds_the_statement_and_the_header_shows_the_inlay(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, SCRIPT, script_svc());
    assert_eq!(painted(&e, vcx), vec![0, 1, 2, 3, 4, 5]);
    assert!(!drawn(vcx).iter().any(|t| t == "hv-editor-fold-lines"));
    // Not a header: nothing to fold there.
    assert!(!e.update(vcx, |v, cx| v.toggle_fold_at(1, cx)));
    assert!(e.update(vcx, |v, cx| v.toggle_fold_at(0, cx)));
    draw(vcx);
    assert_eq!(folded(&e, vcx), vec![0..at(SCRIPT, "orders;") + 7]);
    // The hidden lines take no row; the next statement moves up under the header.
    assert_eq!(painted(&e, vcx), vec![0, 4, 5]);
    let rows = vcx.read(|cx| {
        let l = e.read(cx).last_layout().unwrap();
        (l.rows().first_line, l.rows().last_line)
    });
    assert_eq!(rows, (0, 5));
    // The inlay ("⋯ 3 lines") on the header row, and its chevron.
    assert!(
        drawn(vcx).iter().any(|t| t == "hv-editor-fold-lines"),
        "{:?}",
        drawn(vcx)
    );
    // Toggling the header again unfolds.
    assert!(e.update(vcx, |v, cx| v.toggle_fold_at(0, cx)));
    draw(vcx);
    assert!(folded(&e, vcx).is_empty());
    assert_eq!(painted(&e, vcx), vec![0, 1, 2, 3, 4, 5]);
}

#[gpui::test]
fn a_caret_in_the_hidden_lines_moves_to_the_header_and_landing_there_opens_the_fold(
    cx: &mut TestAppContext,
) {
    let (e, vcx) = open(cx, SCRIPT, script_svc());
    caret_at(&e, vcx, at(SCRIPT, "owner"));
    run(&e, vcx, "editor.fold");
    // The caret was inside: it moves to the header's end, and the fold stays.
    assert_eq!(folded(&e, vcx).len(), 1);
    assert_eq!(head(&e, vcx), at(SCRIPT, "\n  total"));
    // A vertical move steps over the fold.
    run(&e, vcx, "editor.move_down");
    // The goal column (the header's end, 10) clamps to "SELECT 2;"'s end.
    assert_eq!(head(&e, vcx), at(SCRIPT, "SELECT 2") + 9);
    assert_eq!(folded(&e, vcx).len(), 1);
    run(&e, vcx, "editor.move_up");
    assert_eq!(
        vcx.read(|cx| e.read(cx).snapshot().buffer.byte_to_line(head(&e, vcx))),
        0
    );
    assert_eq!(folded(&e, vcx).len(), 1);
    // "Go to" a hidden line (a selection set by the app): the fold opens.
    caret_at(&e, vcx, at(SCRIPT, "FROM"));
    assert!(folded(&e, vcx).is_empty());
    assert_eq!(painted(&e, vcx), vec![0, 1, 2, 3, 4, 5]);
}

/// Select all holds the fold whole: folding keeps the selection, and the selection keeps the
/// fold.
#[gpui::test]
fn select_all_keeps_the_fold_and_the_fold_keeps_select_all(cx: &mut TestAppContext) {
    let text = "SELECT a,\n  b\nFROM t;";
    let svc = Svc {
        folds: vec![FoldRange {
            range: 0..text.len(),
            kind: FoldKind::Statement,
        }],
        statements: vec![0..text.len()],
        ..Svc::default()
    };
    let (e, vcx) = open(cx, text, svc);
    run(&e, vcx, "editor.select_all");
    assert!(e.update(vcx, |v, cx| v.toggle_fold_at(0, cx)));
    draw(vcx);
    let sel = vcx.read(|cx| e.read(cx).snapshot().selection.main().range());
    assert_eq!(sel, 0..text.len());
    assert_eq!(folded(&e, vcx).len(), 1);
    run(&e, vcx, "editor.select_all");
    assert_eq!(folded(&e, vcx).len(), 1);
}

#[gpui::test]
fn the_palette_commands_fold_unfold_fold_all_and_unfold_all(cx: &mut TestAppContext) {
    let text = "SELECT a,\n  b\nFROM t;\n/* one\n   two\n   three */\nSELECT 3;\n";
    let stmt = 0..at(text, "t;") + 2;
    let comment = at(text, "/*")..at(text, "*/") + 2;
    let svc = Svc {
        folds: vec![
            FoldRange {
                range: stmt.clone(),
                kind: FoldKind::Statement,
            },
            FoldRange {
                range: comment.clone(),
                kind: FoldKind::Comment,
            },
        ],
        statements: vec![stmt.clone()],
        ..Svc::default()
    };
    let (e, vcx) = open(cx, text, svc);
    caret_at(&e, vcx, 3);
    run(&e, vcx, "editor.fold");
    assert_eq!(folded(&e, vcx), vec![stmt.clone()]);
    run(&e, vcx, "editor.unfold");
    assert!(folded(&e, vcx).is_empty());
    run(&e, vcx, "editor.fold_all");
    assert_eq!(folded(&e, vcx), vec![stmt.clone(), comment.clone()]);
    assert_eq!(painted(&e, vcx), vec![0, 3, 6, 7]);
    run(&e, vcx, "editor.unfold_all");
    assert!(folded(&e, vcx).is_empty());
    // None of the four is a stub any more, and none has a default chord (controller ruling 7:
    // palette only; the keymap rows are Task 1b's).
    for id in [
        "editor.fold",
        "editor.unfold",
        "editor.fold_all",
        "editor.unfold_all",
    ] {
        let c = hacivat_ui_editor::commands()
            .iter()
            .find(|c| c.id == id)
            .unwrap();
        assert!(!c.stub && !c.seals_history, "{id}");
    }
}

#[gpui::test]
fn folded_lines_are_not_in_a11y_runs(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, SCRIPT, script_svc());
    let values = |e: &Entity<EditorView>, vcx: &mut VisualTestContext| -> Vec<String> {
        e.update(vcx, |v, cx| {
            a11y::declaration_of(v, true, cx)
                .children
                .iter()
                .filter_map(|(_, n)| n.value().map(str::to_string))
                .collect()
        })
    };
    let before = values(&e, vcx);
    assert!(before.iter().any(|v| v.contains("owner")), "{before:?}");
    assert!(e.update(vcx, |v, cx| v.toggle_fold_at(0, cx)));
    draw(vcx);
    let after = values(&e, vcx);
    assert!(
        after.iter().any(|v| v.starts_with("SELECT id,")),
        "{after:?}"
    );
    assert!(
        after.iter().any(|v| v.starts_with("SELECT 2;")),
        "{after:?}"
    );
    for hidden in ["total", "owner", "FROM orders"] {
        assert!(
            !after.iter().any(|v| v.contains(hidden)),
            "{hidden}: {after:?}"
        );
    }
    // Unfolded, the lines come back.
    assert!(e.update(vcx, |v, cx| v.toggle_fold_at(0, cx)));
    assert!(values(&e, vcx).iter().any(|v| v.contains("owner")));
}

/// Two statements inside a region fold: the mark that most needs the user wins the header
/// (NOTES M3c-7: a cancel not answered > failed > … > done), and a failure says where it is.
#[gpui::test]
fn exec_mark_inside_a_fold_paints_on_the_header(cx: &mut TestAppContext) {
    let text =
        "-- region load\nSELECT 1;\nSELECT x FROM missing;\nSELECT 3;\n-- endregion\nSELECT 9;\n";
    let region = 0..at(text, "-- endregion") + 12;
    let s1 = at(text, "SELECT 1")..at(text, "SELECT 1") + 9;
    let s2 = at(text, "SELECT x")..at(text, "missing;") + 8;
    let s3 = at(text, "SELECT 3")..at(text, "SELECT 3") + 9;
    let svc = Svc {
        folds: vec![FoldRange {
            range: region.clone(),
            kind: FoldKind::Region,
        }],
        statements: vec![s1.clone(), s2.clone(), s3.clone()],
        ..Svc::default()
    };
    let (e, vcx) = open(cx, text, svc);
    let version = vcx.read(|cx| e.read(cx).snapshot().version);
    e.update(vcx, |v, cx| {
        v.add_exec_marks(
            version,
            vec![(MarkKey(1), s1), (MarkKey(2), s2), (MarkKey(3), s3)],
            cx,
        );
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Succeeded {
                took: Duration::from_millis(3),
                rows: None,
            },
            text_fn("done tip"),
            text_fn("executed · 3 ms"),
            cx,
        );
        v.set_exec_state(
            MarkKey(2),
            ExecMarkState::Failed {
                sqlstate: Some("42P01".into()),
            },
            text_fn("failed tip"),
            text_fn("failed"),
            cx,
        );
    });
    draw(vcx);
    // Unfolded: each statement's own status on its own line.
    assert!(drawn(vcx).contains(&"executed · 3 ms".to_string()));
    assert!(drawn(vcx).contains(&"failed".to_string()));
    assert!(e.update(vcx, |v, cx| v.toggle_fold_at(0, cx)));
    draw(vcx);
    assert_eq!(painted(&e, vcx), vec![0, 5, 6]);
    let d = drawn(vcx);
    // The failure wins the header and says where it is, with its SQLSTATE chip; the success
    // inside the fold is not listed.
    assert!(d.contains(&"hv-editor-fold-failed".to_string()), "{d:?}");
    assert!(d.contains(&"42P01".to_string()), "{d:?}");
    assert!(!d.contains(&"executed · 3 ms".to_string()), "{d:?}");
    // A cancel not answered needs the user more than a failure.
    let since = vcx.executor().now();
    e.update(vcx, |v, cx| {
        v.set_exec_state(
            MarkKey(3),
            ExecMarkState::Escalated { since },
            text_fn("escalated tip"),
            text_fn("cancel not answered · 0 s"),
            cx,
        );
    });
    draw(vcx);
    let d = drawn(vcx);
    assert!(
        d.contains(&"cancel not answered · 0 s".to_string()),
        "{d:?}"
    );
    assert!(!d.contains(&"hv-editor-fold-failed".to_string()), "{d:?}");
}

/// Run at a caret on a folded statement's header previews (and runs) the whole statement.
#[gpui::test]
fn run_at_a_caret_on_a_folded_statement_runs_the_whole_statement(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, SCRIPT, script_svc());
    assert!(e.update(vcx, |v, cx| v.toggle_fold_at(0, cx)));
    caret_at(&e, vcx, 3);
    assert_eq!(
        folded(&e, vcx).len(),
        1,
        "the header line is visible: no unfold"
    );
    let units = vcx.read(|cx| preview::current(e.read(cx)).map(|p| p.units.to_vec()));
    assert_eq!(units, Some(vec![0..at(SCRIPT, "orders;") + 7]));
}

/// The fold follows its text: typing above moves it; deleting its header's break unfolds it.
#[gpui::test]
fn the_fold_follows_edits_and_a_deleted_header_unfolds(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, SCRIPT, script_svc());
    assert!(e.update(vcx, |v, cx| fold_gutter::fold_range(
        v,
        0..at(SCRIPT, "orders;") + 7,
        cx
    )));
    caret_at(&e, vcx, 0);
    e.update_in(vcx, |v, window, cx| {
        use gpui::EntityInputHandler as _;
        v.replace_text_in_range(None, "-- top\n", window, cx);
    });
    draw(vcx);
    // Typed at its start: the new line goes above the fold, which keeps its header.
    assert_eq!(folded(&e, vcx), vec![7..at(SCRIPT, "orders;") + 7 + 7]);
    assert_eq!(painted(&e, vcx), vec![0, 1, 5, 6]);
    // Delete the header's line break (joins the first hidden line): unfolded.
    let brk = 7 + at(SCRIPT, "\n  total");
    e.update(vcx, |v, cx| v.select_range(brk..brk + 1, cx));
    e.update_in(vcx, |v, window, cx| {
        use gpui::EntityInputHandler as _;
        v.replace_text_in_range(None, "", window, cx);
    });
    draw(vcx);
    assert!(folded(&e, vcx).is_empty());
}

// ---- fix round 1 ---------------------------------------------------------------------------

/// I1: an analysis that lands after the first frame, at the same text version, brings the
/// chevrons and the fold commands without an edit.
#[gpui::test]
fn ranges_from_a_late_analysis_fold_without_an_edit(cx: &mut TestAppContext) {
    let ready = Rc::new(Cell::new(false));
    let svc = Svc {
        late: Some(ready.clone()),
        ..script_svc()
    };
    let notify = svc.notify.clone();
    let (e, vcx) = open(cx, SCRIPT, svc);
    assert!(!e.update(vcx, |v, cx| v.toggle_fold_at(0, cx)));
    run(&e, vcx, "editor.fold_all");
    assert!(folded(&e, vcx).is_empty());
    let version = vcx.read(|cx| e.read(cx).snapshot().version);
    // The analysis lands: the service says so through the editor's callback.
    ready.set(true);
    let f = notify
        .borrow()
        .clone()
        .expect("the editor registered its callback");
    vcx.update(|_, cx| f(cx));
    draw(vcx);
    assert_eq!(vcx.read(|cx| e.read(cx).snapshot().version), version);
    run(&e, vcx, "editor.fold_all");
    assert_eq!(folded(&e, vcx), vec![0..at(SCRIPT, "orders;") + 7]);
}

/// I2: Enter or a key before a folded block's visible closing `)` never joins the fold.
#[gpui::test]
fn typing_before_a_folded_blocks_closing_line_keeps_the_fold(cx: &mut TestAppContext) {
    let text = "SELECT *\nFROM (\n  SELECT 1\n  FROM u\n) s;\n";
    let paren = at(text, "(")..at(text, ") s");
    let svc = Svc {
        folds: vec![FoldRange {
            range: paren.clone(),
            kind: FoldKind::Block,
        }],
        ..Svc::default()
    };
    let (e, vcx) = open(cx, text, svc);
    assert!(e.update(vcx, |v, cx| v.toggle_fold_at(1, cx)));
    draw(vcx);
    assert_eq!(painted(&e, vcx), vec![0, 1, 4, 5]);
    caret_at(&e, vcx, paren.end);
    e.update_in(vcx, |v, window, cx| {
        use gpui::EntityInputHandler as _;
        v.replace_text_in_range(None, "\n", window, cx);
    });
    draw(vcx);
    assert_eq!(folded(&e, vcx), vec![paren.clone()]);
    assert_eq!(painted(&e, vcx), vec![0, 1, 4, 5, 6], "the new line shows");
    e.update_in(vcx, |v, window, cx| {
        use gpui::EntityInputHandler as _;
        v.replace_text_in_range(None, "x", window, cx);
    });
    draw(vcx);
    assert_eq!(folded(&e, vcx), vec![paren.clone()]);
    assert_eq!(painted(&e, vcx), vec![0, 1, 4, 5, 6]);
}

/// M1: ↓ on a folded last row that reaches the text's end keeps the fold; the caret goes to
/// that row's end.
#[gpui::test]
fn down_on_a_folded_last_row_keeps_the_fold(cx: &mut TestAppContext) {
    let text = "SELECT 1;\nSELECT a,\n  b\nFROM t;";
    let svc = Svc {
        folds: vec![FoldRange {
            range: at(text, "SELECT a")..text.len(),
            kind: FoldKind::Statement,
        }],
        ..Svc::default()
    };
    let (e, vcx) = open(cx, text, svc);
    assert!(e.update(vcx, |v, cx| v.toggle_fold_at(1, cx)));
    caret_at(&e, vcx, at(text, "SELECT a") + 2);
    run(&e, vcx, "editor.move_down");
    assert_eq!(folded(&e, vcx).len(), 1);
    assert_eq!(head(&e, vcx), at(text, "\n  b"));
}

/// M9: folding and unfolding are said politely; a folded header's run says what it hides.
#[gpui::test]
fn folds_are_announced_and_described(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, SCRIPT, script_svc());
    // Toggles line 0 and answers what was queued for the live region (read before a frame
    // without assistive technology drops it).
    let toggle = |e: &Entity<EditorView>, vcx: &mut VisualTestContext| -> Vec<String> {
        e.update(vcx, |v, cx| {
            assert!(v.toggle_fold_at(0, cx));
            a11y::state(v)
                .nodes()
                .pending
                .iter()
                .map(|t| t.to_string())
                .collect()
        })
    };
    assert_eq!(toggle(&e, vcx), vec!["hv-editor-fold-done".to_string()]);
    let described: Vec<String> = e.update(vcx, |v, cx| {
        a11y::declaration_of(v, true, cx)
            .children
            .iter()
            .filter_map(|(_, n)| n.description().map(str::to_string))
            .collect()
    });
    assert_eq!(described, vec!["hv-editor-fold-hidden".to_string()]);
    assert!(toggle(&e, vcx).contains(&"hv-editor-unfold-done".to_string()));
}

/// M11: of two failures inside one fold, the first in the text is named (the root cause).
#[gpui::test]
fn the_first_failure_inside_a_fold_is_named(cx: &mut TestAppContext) {
    let text = "-- region load\nSELECT x FROM a;\nSELECT y FROM b;\n-- endregion\nSELECT 9;\n";
    let s1 = at(text, "SELECT x")..at(text, "a;") + 2;
    let s2 = at(text, "SELECT y")..at(text, "b;") + 2;
    let svc = Svc {
        folds: vec![FoldRange {
            range: 0..at(text, "-- endregion") + 12,
            kind: FoldKind::Region,
        }],
        ..Svc::default()
    };
    let (e, vcx) = open(cx, text, svc);
    let version = vcx.read(|cx| e.read(cx).snapshot().version);
    e.update(vcx, |v, cx| {
        v.add_exec_marks(version, vec![(MarkKey(1), s1), (MarkKey(2), s2)], cx);
        for (k, code) in [(1, "42P01"), (2, "22012")] {
            v.set_exec_state(
                MarkKey(k),
                ExecMarkState::Failed {
                    sqlstate: Some(code.into()),
                },
                text_fn("failed tip"),
                text_fn("failed"),
                cx,
            );
        }
    });
    assert!(e.update(vcx, |v, cx| v.toggle_fold_at(0, cx)));
    draw(vcx);
    let d = drawn(vcx);
    assert!(d.contains(&"42P01".to_string()), "{d:?}");
    assert!(!d.contains(&"22012".to_string()), "{d:?}");
}
