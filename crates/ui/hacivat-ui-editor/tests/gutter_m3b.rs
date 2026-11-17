//! M3c Task 34: the gutter extras (NOTES surface 17, M3c-15; the M3b mark badges): ▶ on the
//! hovered row of a statement runs it through Run's own request, only while the host says Run
//! would be enabled and never over a mark that needs the user or is still going; its tooltip
//! lists the statement's last three runs; the escalation hourglass, the awaiting stamp and the
//! rolled-back glyph; the fold header ranks an awaiting mark first; in "Tab moves focus" mode
//! the gutter's buttons are tab stops and AccessKit buttons that Enter activates.

#![allow(
    clippy::single_range_in_vec_init,
    clippy::cloned_ref_to_slice_refs,
    reason = "byte ranges as data"
)]

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers, ParentElement as _,
    Render, Role, Styled as _, TestAppContext, VisualTestContext, Window, div, point, px,
};
use hacivat_text::{ChangeDesc, FoldKind, FoldRange, Snapshot, TokenOracle};
use hacivat_ui::icon::{IconName, icon_record, icon_take};
use hacivat_ui::popover::{a11y_record, a11y_take};
use hacivat_ui::text::{TextFn, TextRegistry};
use hacivat_ui_core::theme::{DARK, Theme, ThemeMode};
use hacivat_ui_editor::gutter::GutterButton;
use hacivat_ui_editor::{
    Confidence, DecorationSource, DocContext, DocId, EditorConfig, EditorEvent, EditorView,
    ExecAction, ExecAt, ExecMarkState, ExecPreview, InvalidateFn, LanguageService, MarkKey,
    NoTokens, StatementSpan,
};
use smallvec::SmallVec;

/// Lines 0 and 2–3 hold the two statements.
const SQL: &str = "SELECT 1;\n\nSELECT 2,\n  3;\n";

fn spans() -> Vec<Range<usize>> {
    vec![0..9, 11..SQL.len() - 1]
}

/// A service whose statements are [`spans`] (their headers at their starts), with the folds it
/// is given.
#[derive(Default)]
struct Svc {
    statements: Vec<Range<usize>>,
    folds: Vec<FoldRange>,
    preview: Option<ExecPreview>,
}

impl LanguageService for Svc {
    fn open(&self, _: DocId, _: &Snapshot, _: DocContext, _: &mut gpui::App) {}
    fn set_context(&self, _: DocId, _: DocContext, _: &mut gpui::App) {}
    fn change(&self, _: DocId, _: &Snapshot, _: &ChangeDesc, _: &mut gpui::App) {}
    fn close(&self, _: DocId) {}
    fn on_update(&self, _: DocId, _: InvalidateFn) {}
    fn statements(&self, _: DocId, _: u64, r: Range<usize>) -> Arc<[StatementSpan]> {
        self.statements
            .iter()
            .filter(|s| s.start < r.end && r.start <= s.end)
            .map(|s| StatementSpan {
                smart: s.clone(),
                strict: s.clone(),
                header: s.start,
                confidence: Confidence::Exact,
            })
            .collect()
    }
    fn exec_preview(&self, _: DocId, _: u64, _: &ExecAt, _: ExecAction) -> Option<ExecPreview> {
        self.preview.clone()
    }
    fn token_oracle(&self, _: DocId) -> Rc<dyn TokenOracle> {
        Rc::new(NoTokens)
    }
    fn decoration_sources(&self, _: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
        SmallVec::new()
    }
    fn folding_ranges(&self, _: DocId, _: u64) -> Arc<[FoldRange]> {
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

struct Opened<'a> {
    editor: Entity<EditorView>,
    events: Rc<RefCell<Vec<EditorEvent>>>,
    gate: Rc<std::cell::Cell<bool>>,
    vcx: &'a mut VisualTestContext,
}

fn open<'a>(cx: &'a mut TestAppContext, text: &str, svc: Svc) -> Opened<'a> {
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
                DocId(34),
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
    let events: Rc<RefCell<Vec<EditorEvent>>> = Rc::default();
    let sink = events.clone();
    let gate = Rc::new(std::cell::Cell::new(true));
    let g = gate.clone();
    vcx.update(|window, cx| {
        cx.subscribe(&editor, move |_, e: &EditorEvent, _| {
            sink.borrow_mut().push(e.clone());
        })
        .detach();
        window.activate_window();
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    editor.update(vcx, |v, cx| {
        v.set_gutter_run(Some(Rc::new(move |_: &gpui::App| g.get())), cx);
    });
    draw(vcx);
    Opened {
        editor,
        events,
        gate,
        vcx,
    }
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| {
        cx.global_mut::<TextRegistry>().frame.clear();
        window.refresh();
    });
    vcx.run_until_parked();
}

/// Rows are 18 px from y = 6.
fn row_mid(line: u32) -> f32 {
    6.0 + 18.0 * line as f32 + 9.0
}

/// Moves the pointer over the gutter's mark column on `line`, and draws; answers the icons
/// drawn.
fn hover(o: &mut Opened<'_>, line: u32) -> Vec<hacivat_ui::icon::DrawnIcon> {
    o.vcx
        .simulate_mouse_move(point(px(14.0), px(row_mid(line))), None, Modifiers::none());
    icon_record();
    draw(o.vcx);
    icon_take()
}

/// A key pressed and released (a keyboard click fires on the release).
fn press(o: &mut Opened<'_>, key: &'static str) {
    let keystroke = gpui::Keystroke::parse(key).unwrap();
    o.vcx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    o.vcx.simulate_event(gpui::KeyUpEvent { keystroke });
    o.vcx.run_until_parked();
}

fn plays(icons: &[hacivat_ui::icon::DrawnIcon]) -> usize {
    icons.iter().filter(|i| i.name == IconName::Play).count()
}

fn requests(o: &Opened<'_>) -> Vec<ExecAt> {
    o.events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::ExecuteRequested(r) => {
                assert_eq!(r.action, ExecAction::Execute1);
                Some(r.at.clone())
            }
            _ => None,
        })
        .collect()
}

fn version(o: &Opened<'_>) -> u64 {
    o.vcx.read(|cx| o.editor.read(cx).snapshot().version)
}

fn text_fn(s: &'static str) -> Option<TextFn> {
    Some(Rc::new(move |_: &gpui::App| s.into()))
}

fn ok() -> ExecMarkState {
    ExecMarkState::Succeeded {
        took: Duration::from_millis(3),
        rows: None,
    }
}

#[gpui::test]
fn gutter_play_runs_the_rows_statement(cx: &mut TestAppContext) {
    let mut o = open(
        cx,
        SQL,
        Svc {
            statements: spans(),
            ..Svc::default()
        },
    );
    // Nothing until the pointer is on a statement's first row.
    icon_record();
    draw(o.vcx);
    assert_eq!(plays(&icon_take()), 0);
    assert_eq!(plays(&hover(&mut o, 1)), 0, "a blank row has no statement");
    let icons = hover(&mut o, 2);
    assert_eq!(plays(&icons), 1, "{icons:?}");
    let play = icons.iter().find(|i| i.name == IconName::Play).unwrap();
    assert_eq!(
        play.color,
        DARK.colors.text.to_hsla(),
        "the hovered ▶ is `text`"
    );
    // A click runs that statement through Run's request: a caret at its first code byte.
    o.vcx
        .simulate_click(point(px(14.0), px(row_mid(2))), Modifiers::none());
    o.vcx.run_until_parked();
    let at = requests(&o);
    assert_eq!(at.len(), 1, "{at:?}");
    assert!(
        matches!(&at[0], ExecAt::Carets(c) if c.as_slice() == [11]),
        "{at:?}"
    );
    // The caret did not move.
    let head = o
        .vcx
        .read(|cx| o.editor.read(cx).snapshot().selection.main().head);
    assert_eq!(head, 0);
}

#[gpui::test]
fn no_play_while_run_is_off_or_over_a_live_mark(cx: &mut TestAppContext) {
    let mut o = open(
        cx,
        SQL,
        Svc {
            statements: spans(),
            ..Svc::default()
        },
    );
    // The host says Run is off (a statement runs, waits for an answer or a connection).
    o.gate.set(false);
    assert_eq!(plays(&hover(&mut o, 2)), 0);
    o.gate.set(true);
    // A mark that needs the user or is still going keeps its place.
    let v0 = version(&o);
    let since = o.vcx.executor().now();
    for state in [
        ExecMarkState::Queued,
        ExecMarkState::Running { started: since },
        ExecMarkState::Cancelling { since },
        ExecMarkState::Escalated { since },
        ExecMarkState::Awaiting,
    ] {
        o.editor.update(o.vcx, |v, cx| {
            v.add_exec_marks(v0, vec![(MarkKey(2), 11..SQL.len() - 1)], cx);
            v.set_exec_state(MarkKey(2), state.clone(), None, None, cx);
        });
        o.vcx.executor().advance_clock(Duration::from_millis(60));
        assert_eq!(plays(&hover(&mut o, 2)), 0, "{state:?}");
    }
    // A finished one gives its place to ▶ while the pointer is there (its status stays).
    o.editor.update(o.vcx, |v, cx| {
        v.set_exec_state(MarkKey(2), ok(), None, text_fn("12 ms"), cx);
    });
    let icons = hover(&mut o, 2);
    assert_eq!(plays(&icons), 1, "{icons:?}");
    let green = DARK.colors.green.to_hsla();
    assert_eq!(
        icons
            .iter()
            .filter(|i| i.name == IconName::Check && i.color == green)
            .count(),
        1,
        "only the status's check, the glyph's slot is ▶'s: {icons:?}"
    );
    assert_eq!(plays(&hover(&mut o, 0)), 1, "and back to the other row");
}

#[gpui::test]
fn escalated_mark_paints_the_badge(cx: &mut TestAppContext) {
    let o = open(cx, SQL, Svc::default());
    let v0 = version(&o);
    let since = o.vcx.executor().now();
    o.editor.update(o.vcx, |v, cx| {
        v.add_exec_marks(
            v0,
            vec![(MarkKey(1), 0..9), (MarkKey(2), 11..SQL.len() - 1)],
            cx,
        );
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Escalated { since },
            text_fn("cancel not answered"),
            text_fn("cancel not answered · 14 s"),
            cx,
        );
        v.set_exec_state(MarkKey(2), ExecMarkState::RolledBack, None, None, cx);
    });
    icon_record();
    draw(o.vcx);
    let icons = icon_take();
    let c = &DARK.colors;
    assert!(
        icons
            .iter()
            .any(|i| i.name == IconName::Hourglass && i.color == c.amber.to_hsla()),
        "the amber hourglass: {icons:?}"
    );
    assert!(
        icons
            .iter()
            .any(|i| i.name == IconName::Undo2 && i.color == c.text_2.to_hsla()),
        "rolled back: {icons:?}"
    );
    // Its words are amber and repaint once a second (the escalation's clock).
    assert!(o.vcx.read(|cx| o.editor.read(cx).marks().ticking()));
    let texts: Vec<String> = o.vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .map(|t| t.text.to_string())
            .collect()
    });
    assert!(
        texts.iter().any(|t| t == "cancel not answered · 14 s"),
        "{texts:?}"
    );
}

#[gpui::test]
fn an_awaiting_mark_wins_its_fold_header(cx: &mut TestAppContext) {
    let text = "SELECT a,\n  b,\n  c\nFROM t;\n";
    let mut o = open(
        cx,
        text,
        Svc {
            folds: vec![FoldRange {
                range: 0..text.len() - 1,
                kind: FoldKind::Statement,
            }],
            ..Svc::default()
        },
    );
    let v0 = version(&o);
    // Two statements inside the fold: a failed one, and one waiting for Commit or Roll back.
    o.editor.update(o.vcx, |v, cx| {
        v.add_exec_marks(v0, vec![(MarkKey(1), 10..13), (MarkKey(2), 16..19)], cx);
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Failed { sqlstate: None },
            None,
            None,
            cx,
        );
        v.set_exec_state(
            MarkKey(2),
            ExecMarkState::Awaiting,
            None,
            text_fn("waiting for Commit or Roll back"),
            cx,
        );
    });
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.run_command("editor.fold_all", window, cx);
    });
    icon_record();
    draw(o.vcx);
    let icons = icon_take();
    assert!(
        !icons.iter().any(|i| i.name == IconName::TriangleAlert),
        "the header shows the awaiting stamp, not the failure: {icons:?}"
    );
    let texts: Vec<String> = o.vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .map(|t| t.text.to_string())
            .collect()
    });
    assert!(
        texts.iter().any(|t| t == "waiting for Commit or Roll back"),
        "{texts:?}"
    );
    let _ = &mut o;
}

#[gpui::test]
fn the_play_tooltip_lists_the_last_three_runs(cx: &mut TestAppContext) {
    let mut o = open(
        cx,
        SQL,
        Svc {
            statements: spans(),
            ..Svc::default()
        },
    );
    // Four runs of statement 2, each re-run replacing the last mark.
    let outcomes = [
        (ok(), "run 1"),
        (
            ExecMarkState::Failed {
                sqlstate: Some("42P01".into()),
            },
            "run 2",
        ),
        (ExecMarkState::Cancelled, "run 3"),
        (ok(), "run 4"),
    ];
    for (i, (state, eol)) in outcomes.iter().enumerate() {
        let v0 = version(&o);
        let key = MarkKey(10 + i as u64);
        let eol: &'static str = eol;
        o.editor.update(o.vcx, |v, cx| {
            v.add_exec_marks(v0, vec![(key, 11..SQL.len() - 1)], cx);
            v.set_exec_state(key, state.clone(), None, text_fn(eol), cx);
            v.set_exec_when(key, text_fn("14:02:11"), cx);
        });
    }
    let runs = o
        .vcx
        .read(|cx| o.editor.read(cx).marks().last_runs(MarkKey(13)));
    assert_eq!(runs.len(), 3, "{runs:?}");
    assert!(matches!(runs[0], ExecMarkState::Succeeded { .. }));
    assert_eq!(runs[1], ExecMarkState::Cancelled);
    assert!(matches!(runs[2], ExecMarkState::Failed { .. }));
    // Hover ▶ and wait out the tooltip's delay.
    hover(&mut o, 2);
    o.vcx
        .simulate_mouse_move(point(px(13.0), px(row_mid(2))), None, Modifiers::none());
    o.vcx.executor().advance_clock(Duration::from_secs(2));
    o.vcx.run_until_parked();
    let texts: Vec<String> = o.vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .map(|t| t.text.to_string())
            .collect()
    });
    for t in [
        "hv-gutter-run",
        "hv-gutter-last-runs",
        "run 4",
        "run 3",
        "run 2",
        "42P01",
        "14:02:11",
        "hv-gutter-same-path",
    ] {
        assert!(texts.iter().any(|x| x == t), "{t}: {texts:?}");
    }
    assert!(
        !texts.iter().any(|x| x == "run 1"),
        "three at most: {texts:?}"
    );
    // Statement 1 never ran: "Not run yet."
    hover(&mut o, 0);
    o.vcx
        .simulate_mouse_move(point(px(13.0), px(row_mid(0))), None, Modifiers::none());
    o.vcx.executor().advance_clock(Duration::from_secs(2));
    o.vcx.run_until_parked();
    let texts: Vec<String> = o.vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .map(|t| t.text.to_string())
            .collect()
    });
    assert!(
        texts.iter().any(|x| x == "hv-gutter-never-run"),
        "{texts:?}"
    );
}

#[gpui::test]
fn gutter_buttons_are_keyboard_reachable_buttons(cx: &mut TestAppContext) {
    let text = "SELECT a,\n  b,\n  c\nFROM t;\nSELECT 2;\n";
    let mut o = open(
        cx,
        text,
        Svc {
            statements: vec![
                0..text.find(';').unwrap() + 1,
                text.find("SELECT 2").unwrap()..text.len() - 1,
            ],
            folds: vec![FoldRange {
                range: 0..text.find(';').unwrap() + 1,
                kind: FoldKind::Statement,
            }],
            ..Svc::default()
        },
    );
    // "Tab moves focus" mode (Ctrl+M): the gutter's buttons are tab stops.
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.run_command("editor.toggle_tab_moves_focus", window, cx);
    });
    a11y_record();
    draw(o.vcx);
    let decls = a11y_take();
    let buttons: Vec<_> = decls
        .iter()
        .filter(|d| d.role == Some(Role::Button))
        .map(|d| (d.name.to_string(), d.expanded))
        .collect();
    assert!(
        // ## fix round 1 (M7): its name says the statement's line.
        buttons.iter().any(|(n, _)| n == "hv-gutter-run-line"),
        "▶ is an AccessKit button: {buttons:?}"
    );
    assert!(
        buttons
            .iter()
            .any(|(n, e)| n == "hv-editor-fold-name" && *e == Some(true)),
        "the chevron is an expanded button: {buttons:?}"
    );
    // Tab leaves the editor for the first gutter button: ▶ of line 0, then line 4's.
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.run_command("editor.tab", window, cx);
    });
    draw(o.vcx);
    let focus = o
        .vcx
        .update(|window, cx| o.editor.read(cx).gutter_focus(window));
    assert_eq!(focus, Some(GutterButton::Run(0)));
    // A focused gutter button shows every ▶ of the view.
    icon_record();
    draw(o.vcx);
    assert_eq!(plays(&icon_take()), 2);
    // Enter runs that statement.
    press(&mut o, "enter");
    o.vcx.run_until_parked();
    let at = requests(&o);
    assert!(
        matches!(at.as_slice(), [ExecAt::Carets(c)] if c.as_slice() == [0]),
        "{at:?}"
    );
    // ## fix round 1 (I1): the run gives the focus back to the editor, so Stop and the editor's
    // keys work while the ▶ is gone (the gate turns false while the statement runs).
    let in_editor = |o: &mut Opened<'_>| {
        o.vcx
            .update(|window, cx| o.editor.read(cx).focus_handle(cx).is_focused(window))
    };
    assert!(in_editor(&mut o), "the focus is back in the editor");
    o.gate.set(false);
    draw(o.vcx);
    assert!(in_editor(&mut o), "still in the editor while the ▶ is gone");
    o.gate.set(true);
    draw(o.vcx);
    // On to the chevron (after the ▶ column); Space folds.
    for _ in 0..3 {
        o.vcx.update(|window, cx| window.focus_next(cx));
        draw(o.vcx);
    }
    let focus = o
        .vcx
        .update(|window, cx| o.editor.read(cx).gutter_focus(window));
    assert_eq!(focus, Some(GutterButton::Fold(0)));
    press(&mut o, "space");
    o.vcx.run_until_parked();
    let folded = o.vcx.read(|cx| o.editor.read(cx).folds().folded().len());
    assert_eq!(folded, 1, "Space folded the statement");
}

/// A selection Run sends as one unit (R-M3-60): the band's first line says "as one · 2
/// statements" with the layers glyph (NOTES surface 9; the T24/T29 carry).
#[gpui::test]
fn a_selection_run_as_one_says_so_on_its_first_line(cx: &mut TestAppContext) {
    let o = open(
        cx,
        SQL,
        Svc {
            preview: Some(ExecPreview {
                ranges: smallvec::smallvec![0..SQL.len() - 1],
                state: hacivat_ui_editor::PreviewState::Units,
                choices: 0,
                as_one: 2,
            }),
            ..Svc::default()
        },
    );
    o.editor
        .update(o.vcx, |v, cx| v.select_range(0..SQL.len() - 1, cx));
    o.vcx
        .executor()
        .advance_clock(hacivat_ui_editor::preview::PREVIEW_DELAY);
    o.vcx.run_until_parked();
    icon_record();
    draw(o.vcx);
    let icons = icon_take();
    assert!(
        icons.iter().any(|i| i.name == IconName::Layers),
        "{icons:?}"
    );
    let texts: Vec<String> = o.vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .map(|t| t.text.to_string())
            .collect()
    });
    assert!(
        texts.iter().any(|t| t == "hv-editor-preview-as-one"),
        "{texts:?}"
    );
    let plan = o
        .vcx
        .read(|cx| hacivat_ui_editor::preview::current(o.editor.read(cx)))
        .expect("a band");
    assert_eq!((plan.units.len(), plan.as_one), (1, 2));
}
