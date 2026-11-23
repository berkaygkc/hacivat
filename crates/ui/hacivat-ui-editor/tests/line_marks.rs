// ## m4b-t17-secrets-ui fix round 1 (I1, R-T17-2)
//! The host's line marks against the gutter ▶ (M3c surface 17), the way M3c treats a finished
//! execution mark: a shown ▶ (hover, a focused column, "Tab moves focus" mode) takes the mark
//! slot and carries the row's labels in its tooltip and accessible name; the key shows
//! otherwise; a line mark never removes a ▶ (the column's arrows visit marked rows, and a
//! focused ▶ stays when a scan marks its row).

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers, ParentElement as _,
    Render, Role, Styled as _, TestAppContext, VisualTestContext, Window, div, point, px,
};
use hacivat_text::{ChangeDesc, FoldRange, Snapshot, TokenOracle};
use hacivat_ui::icon::IconName;
use hacivat_ui::popover::{a11y_record, a11y_take};
use hacivat_ui::text::{TextFn, TextRegistry};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::gutter::GutterButton;
use hacivat_ui_editor::{
    Confidence, DecorationSource, DocContext, DocId, EditorConfig, EditorView, ExecAction, ExecAt,
    ExecPreview, InvalidateFn, LanguageService, LineMark, NoTokens, StatementSpan,
};
use smallvec::SmallVec;

/// Three statements, on lines 0, 3 and 6; line 1 is inside the first.
const SQL: &str = "SELECT a,\n  b\nFROM t;\nSELECT c,\n  d\nFROM u;\nSELECT 3;\n";

fn statements() -> Vec<Range<usize>> {
    let first = 0..SQL.find(';').unwrap() + 1;
    let second = SQL.find("SELECT c").unwrap()..SQL.find("FROM u;").unwrap() + 7;
    let third = SQL.find("SELECT 3").unwrap()..SQL.len() - 1;
    vec![first, second, third]
}

struct Svc(Vec<Range<usize>>);

impl LanguageService for Svc {
    fn open(&self, _: DocId, _: &Snapshot, _: DocContext, _: &mut gpui::App) {}
    fn set_context(&self, _: DocId, _: DocContext, _: &mut gpui::App) {}
    fn change(&self, _: DocId, _: &Snapshot, _: &ChangeDesc, _: &mut gpui::App) {}
    fn close(&self, _: DocId) {}
    fn on_update(&self, _: DocId, _: InvalidateFn) {}
    fn statements(&self, _: DocId, _: u64, r: Range<usize>) -> Arc<[StatementSpan]> {
        self.0
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
        None
    }
    fn token_oracle(&self, _: DocId) -> Rc<dyn TokenOracle> {
        Rc::new(NoTokens)
    }
    fn decoration_sources(&self, _: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
        SmallVec::new()
    }
    fn folding_ranges(&self, _: DocId, _: u64) -> Arc<[FoldRange]> {
        Arc::from([])
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
    vcx: &'a mut VisualTestContext,
}

fn open(cx: &mut TestAppContext) -> Opened<'_> {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
    });
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let out = slot.clone();
    let svc = Rc::new(Svc(statements()));
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(17),
                SQL,
                EditorConfig::default(),
                svc,
                Rc::new(|_: &gpui::App| "Script".into()),
                window,
                cx,
            )
        });
        *out.borrow_mut() = Some(editor.clone());
        Host { editor }
    });
    let editor = slot.borrow().clone().unwrap();
    vcx.update(|window, cx| {
        window.activate_window();
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    editor.update(vcx, |v, cx| {
        v.set_gutter_run(Some(Rc::new(|_: &gpui::App| true)), cx);
    });
    draw(vcx);
    Opened { editor, vcx }
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| {
        cx.global_mut::<TextRegistry>().frame.clear();
        window.refresh();
    });
    vcx.run_until_parked();
}

fn row_mid(line: u32) -> f32 {
    6.0 + 18.0 * line as f32 + 9.0
}

fn hover(o: &mut Opened<'_>, line: u32) {
    o.vcx
        .simulate_mouse_move(point(px(14.0), px(row_mid(line))), None, Modifiers::none());
    draw(o.vcx);
}

fn press(o: &mut Opened<'_>, key: &'static str) {
    let keystroke = gpui::Keystroke::parse(key).unwrap();
    o.vcx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    o.vcx.simulate_event(gpui::KeyUpEvent { keystroke });
    o.vcx.run_until_parked();
    draw(o.vcx);
}

fn text_fn(s: &'static str) -> TextFn {
    Rc::new(move |_: &gpui::App| s.into())
}

/// A key mark over `needle`'s first occurrence (the host's scan, at the current version).
fn mark(o: &mut Opened<'_>, needles: &[&'static str]) {
    let marks: Vec<LineMark> = needles
        .iter()
        .map(|n| {
            let at = SQL.find(n).unwrap();
            LineMark {
                range: at..at + n.len(),
                icon: IconName::KeyRound,
                label: text_fn("looks-like-a-password"),
                detail: vec![text_fn("saving-asks-first")],
            }
        })
        .collect();
    o.editor.update(o.vcx, |v, cx| {
        let version = v.snapshot().version;
        v.set_line_marks(version, marks, cx);
    });
    draw(o.vcx);
}

fn keys(o: &Opened<'_>) -> Vec<u32> {
    o.vcx.read(|cx| o.editor.read(cx).line_mark_rows().to_vec())
}

fn plays(o: &Opened<'_>) -> Vec<u32> {
    o.vcx
        .read(|cx| o.editor.read(cx).gutter_play_rows().to_vec())
}

fn frame_texts(o: &Opened<'_>) -> Vec<String> {
    o.vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .map(|t| t.text.to_string())
            .collect()
    })
}

/// The ▶ takes the slot while it shows (hover); the key shows otherwise, on a statement's first
/// row and on any other row. The covering ▶'s tooltip lists the key's label.
#[gpui::test]
fn a_shown_play_takes_the_key_slot_and_carries_its_label(cx: &mut TestAppContext) {
    let mut o = open(cx);
    // Line 3 starts a statement; line 4 does not.
    mark(&mut o, &["SELECT c", "  d"]);
    assert_eq!(keys(&o), vec![3, 4], "no pointer: both keys");
    assert_eq!(plays(&o), Vec::<u32>::new());
    hover(&mut o, 3);
    assert_eq!(plays(&o), vec![3], "the ▶ shows on its row");
    assert_eq!(keys(&o), vec![4], "and takes the key's slot there");
    // The ▶'s tooltip carries the key's label.
    o.vcx
        .simulate_mouse_move(point(px(13.0), px(row_mid(3))), None, Modifiers::none());
    o.vcx.executor().advance_clock(Duration::from_secs(2));
    o.vcx.run_until_parked();
    let texts = frame_texts(&o);
    assert!(texts.iter().any(|t| t == "hv-gutter-run"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t == "looks-like-a-password"),
        "{texts:?}"
    );
    // A row with no ▶ keeps its key under the pointer.
    hover(&mut o, 4);
    assert_eq!(plays(&o), Vec::<u32>::new());
    assert_eq!(keys(&o), vec![3, 4]);
    // The key's own accessible description is the hint (fix round 1, m3).
    a11y_record();
    draw(o.vcx);
    let decls = a11y_take();
    assert!(
        decls.iter().any(|d| d.role == Some(Role::Image)
            && d.name.as_ref() == "looks-like-a-password"
            && d.description.as_deref() == Some("saving-asks-first")),
        "{:?}",
        decls
            .iter()
            .map(|d| (d.role, d.name.to_string(), d.description.clone()))
            .collect::<Vec<_>>()
    );
}

/// In "Tab moves focus" mode every ▶ is an AccessKit button; a marked row's says the key's label.
#[gpui::test]
fn the_play_name_carries_the_marks_label(cx: &mut TestAppContext) {
    let mut o = open(cx);
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.run_command("editor.toggle_tab_moves_focus", window, cx);
    });
    mark(&mut o, &["SELECT c"]);
    a11y_record();
    draw(o.vcx);
    let names: Vec<String> = a11y_take()
        .iter()
        .filter(|d| d.role == Some(Role::Button))
        .map(|d| d.name.to_string())
        .collect();
    assert_eq!(
        names
            .iter()
            .filter(|n| n.starts_with("hv-gutter-run-line"))
            .count(),
        3,
        "no ▶ is lost to a mark: {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| n == "hv-gutter-run-line · looks-like-a-password"),
        "{names:?}"
    );
    // Not shown (no pointer, no column focus): the key keeps its slot.
    assert_eq!(keys(&o), vec![3]);
}

/// ↑/↓ along the ▶ column visit a marked row; a focused ▶ stays when a scan marks its row.
#[gpui::test]
fn arrows_visit_marked_rows_and_a_focused_play_stays(cx: &mut TestAppContext) {
    let mut o = open(cx);
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.run_command("editor.toggle_tab_moves_focus", window, cx);
    });
    mark(&mut o, &["SELECT c"]);
    let focus = |o: &mut Opened<'_>| {
        o.vcx
            .update(|window, cx| o.editor.read(cx).gutter_focus(window))
    };
    // The caret is on line 0: Tab reaches its ▶.
    o.vcx.update(|window, cx| window.focus_next(cx));
    draw(o.vcx);
    assert_eq!(focus(&mut o), Some(GutterButton::Run(0)));
    assert_eq!(plays(&o), vec![0, 3, 6], "a focused column shows every ▶");
    assert_eq!(keys(&o), Vec::<u32>::new(), "the shown ▶ holds the slot");
    press(&mut o, "down");
    assert_eq!(focus(&mut o), Some(GutterButton::Run(3)), "the marked row");
    press(&mut o, "down");
    assert_eq!(focus(&mut o), Some(GutterButton::Run(6)));
    press(&mut o, "up");
    assert_eq!(focus(&mut o), Some(GutterButton::Run(3)));
    // A new scan marks the focused row too: the ▶ stays focused and drawn.
    mark(&mut o, &["SELECT c", "SELECT 3"]);
    assert_eq!(focus(&mut o), Some(GutterButton::Run(3)));
    assert!(plays(&o).contains(&3), "{:?}", plays(&o));
    press(&mut o, "down");
    assert_eq!(focus(&mut o), Some(GutterButton::Run(6)));
    // Escape: back to the code; the keys show again.
    press(&mut o, "escape");
    assert_eq!(focus(&mut o), None);
    assert_eq!(keys(&o), vec![3, 6]);
}
