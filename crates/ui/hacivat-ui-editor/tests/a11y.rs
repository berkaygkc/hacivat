//! M3a Task 29: the editor's accessibility baseline (UI-EDIT-24 subset, D-073): the ±100-line
//! window of text runs split at 255 characters (CRLF counts as one), node keys that follow
//! their lines, the text selection across runs, the role and the label, polite announcements,
//! and that nothing is built while no assistive technology is attached.
//!
//! The test platforms never attach assistive technology (`Window::is_a11y_active` stays
//! false; `hacivat-ui/tests/a11y.rs`), so the tree GPUI would build is checked through the
//! pure [`a11y::declaration`] the editor hands to `a11y_synthetic_children`, and the frames
//! are checked for what they do not do.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AnyView, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Role,
    StyleRefinement, Styled as _, TestAppContext, VisualTestContext, Window, accesskit, div, px,
};
use hacivat_text::{
    Annotations, ChangeLog, ChangeSet, EditorModel, ModelConfig, Origin, SelectionRange,
    SelectionSet, Snapshot, TextBuffer, Transaction, UserEvent,
};
use hacivat_ui::l10n::{EnglishDefaults, UiText};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::a11y::{self, A11yFrame, A11yState, ChildKey, Declaration, Pos, RunNode};
use hacivat_ui_editor::{
    DocId, EditorConfig, EditorView, ExecMarkState, FailClosedService, FrameProbe, RowsOutcome,
    Tier,
};

// ---- the pure window builder ---------------------------------------------------------------

fn window_of(text: impl AsRef<str>) -> Vec<RunNode> {
    a11y::window(&TextBuffer::from_str(text.as_ref()), 0, 100)
}

/// `n` numbered lines, each ending in LF.
fn lines(n: usize) -> String {
    (0..n).map(|i| format!("SELECT {i};\n")).collect()
}

#[test]
fn window_is_plus_minus_100_lines() {
    let buf = TextBuffer::from_str(&lines(1000));
    let w = a11y::window(&buf, 500, 100);
    assert_eq!(w.len(), 201);
    assert_eq!((w[0].line, w[200].line), (400, 600));
    // Each line is one entry with the bytes of the line and its break.
    for n in &w {
        assert_eq!(n.bytes, buf.line_range(n.line, true));
    }
    // Clamped at the top and the bottom (the text ends in LF: its last line is empty).
    let top = a11y::window(&buf, 10, 100);
    assert_eq!((top.len(), top[0].line), (111, 0));
    let last = buf.len_lines() - 1;
    let bottom = a11y::window(&buf, last, 100);
    assert_eq!(
        (bottom[0].line, bottom.last().unwrap().line),
        (last - 100, last)
    );
    // The empty last line still has one run, with no characters (the caret can sit there).
    let end = bottom.last().unwrap();
    assert_eq!(end.runs.len(), 1);
    assert!(end.runs[0].char_lens.is_empty());
    // Another window size.
    assert_eq!(a11y::window(&buf, 500, 3).len(), 7);
}

#[test]
fn runs_split_at_255_chars_and_crlf_is_one() {
    let w = window_of("x".repeat(600) + "\r\n");
    assert_eq!(w[0].runs.len(), 3);
    let counts: Vec<usize> = w[0].runs.iter().map(|r| r.char_lens.len()).collect();
    assert_eq!(counts, [255, 255, 91]);
    assert_eq!(a11y::RUN_MAX_CHARS, 255);
    // The CRLF is the last run's last character, two bytes long.
    assert_eq!(w[0].runs[2].char_lens.last(), Some(&2));
    assert_eq!(w[0].runs[2].bytes, 510..602);
    // Runs tile the line.
    assert_eq!(w[0].runs[0].bytes, 0..255);
    assert_eq!(w[0].runs[1].bytes, 255..510);
    // An empty line is a run holding its break alone; a lone CR is a break too.
    let w = window_of("a\n\nb\rc");
    let lens: Vec<Vec<u8>> = w.iter().map(|n| n.runs[0].char_lens.clone()).collect();
    assert_eq!(lens, [vec![1, 1], vec![1], vec![1, 1], vec![1]]);
}

#[test]
fn characters_are_graphemes_and_no_character_passes_255_bytes() {
    // "é" as e + combining acute is one character of three bytes; "ğ" is one of two.
    let w = window_of("e\u{301}ğ");
    assert_eq!(w[0].runs[0].char_lens, [3, 2]);
    // One grapheme of 301 bytes (e + 150 combining marks) cannot be one AccessKit character
    // (lengths are `u8`): it is cut at char boundaries into characters of at most 255 bytes.
    let big = format!("e{}", "\u{301}".repeat(150));
    let w = window_of(&big);
    let lens = &w[0].runs[0].char_lens;
    assert_eq!(lens.iter().map(|&l| usize::from(l)).sum::<usize>(), 301);
    assert!(lens.len() >= 2 && lens.iter().all(|&l| l > 0));
    let mut at = 0;
    for &l in lens {
        at += usize::from(l);
        assert!(big.is_char_boundary(at));
    }
}

/// A 200 KB line (a minified dump; T1's 15 MB single line) is not segmented whole on every
/// edit: the window exposes 64 KB of it, around the caret when the caret is on it.
#[test]
fn a_very_long_line_exposes_64_kb_around_the_caret() {
    let text = "x".repeat(200_000) + "\nSELECT 1;";
    let cap = a11y::LINE_MAX_BYTES;
    // Not the caret's line (the pure window puts the caret at line 1): its first 64 KB.
    let w = a11y::window(&TextBuffer::from_str(&text), 1, 100);
    let shown = |n: &RunNode| n.runs[0].bytes.start..n.runs.last().unwrap().bytes.end;
    assert_eq!(shown(&w[0]), 0..cap);
    assert_eq!(
        w[0].bytes,
        0..200_001,
        "the node still knows the whole line"
    );
    // The caret's line: centred on the caret, and the selection lands inside.
    let f = frame_of(
        &text,
        SelectionRange::caret(150_000),
        EditorConfig::default(),
        Tier::Reduced,
    );
    let a = A11yState::default();
    let d = declare(&a, &f);
    let runs: Vec<&accesskit::Node> = d
        .children
        .iter()
        .filter(|(k, _)| matches!(k, ChildKey::Run { .. }))
        .map(|(_, n)| n)
        .collect();
    let exposed: usize = runs
        .iter()
        .take_while(|n| n.value().is_some_and(|v| v.starts_with('x')))
        .map(|n| n.value().unwrap().len())
        .sum();
    assert_eq!(exposed, cap);
    let (_, focus) = d.selection.unwrap();
    let ChildKey::Run { run, .. } = focus.key else {
        panic!("{focus:?}")
    };
    // 150,000 is 32 KB into the exposed part: run 128, 128 characters in.
    assert_eq!((run, focus.index), (128, 128));
}

// ---- the declaration ---------------------------------------------------------------------

fn frame_of(text: &str, selection: SelectionRange, config: EditorConfig, tier: Tier) -> A11yFrame {
    A11yFrame {
        snapshot: Snapshot {
            buffer: TextBuffer::from_str(text),
            selection: SelectionSet::single(selection),
            version: 0,
        },
        label: "Console 1".into(),
        config,
        tier,
        focused: true,
    }
}

/// A fixed moment for the pure declarations (only differences from it matter).
fn t0() -> Instant {
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *T0.get_or_init(Instant::now)
}

fn declare(a: &A11yState, f: &A11yFrame) -> Declaration {
    declare_at(a, f, t0())
}

fn declare_at(a: &A11yState, f: &A11yFrame, now: Instant) -> Declaration {
    a11y::declaration(a, f, &ChangeLog::default(), None, now)
}

/// The keys of the window's run children, in order.
fn run_keys(d: &Declaration) -> Vec<(u64, u32)> {
    d.children
        .iter()
        .filter_map(|(k, _)| match k {
            ChildKey::Run { line, run } => Some((*line, *run)),
            ChildKey::Live(_) => None,
        })
        .collect()
}

#[test]
fn selection_250_to_260_spans_runs_1_and_2() {
    let text = "x".repeat(600) + "\n";
    let f = frame_of(
        &text,
        SelectionRange::new(250, 260),
        EditorConfig::default(),
        Tier::Full,
    );
    let d = declare(&A11yState::default(), &f);
    let keys = run_keys(&d);
    let line = keys[0].0;
    // Three runs on the long line, then the empty line after its break.
    assert_eq!(keys.iter().filter(|k| k.0 == line).count(), 3);
    assert_eq!(keys.len(), 4);
    let (anchor, focus) = d.selection.expect("a selection");
    assert_eq!(
        (anchor, focus),
        (
            Pos {
                key: ChildKey::Run { line, run: 0 },
                index: 250
            },
            Pos {
                key: ChildKey::Run { line, run: 1 },
                index: 5
            }
        )
    );
    // A reversed selection keeps its anchor where it started.
    let f = frame_of(
        &text,
        SelectionRange::new(260, 250),
        EditorConfig::default(),
        Tier::Full,
    );
    let (anchor, focus) = declare(&A11yState::default(), &f).selection.unwrap();
    assert_eq!((anchor.index, focus.index), (5, 250));
    // A caret at the end of a line sits on its break, not after it.
    let f = frame_of(
        "ab\ncd",
        SelectionRange::caret(2),
        EditorConfig::default(),
        Tier::Full,
    );
    let (anchor, focus) = declare(&A11yState::default(), &f).selection.unwrap();
    assert_eq!(anchor, focus);
    assert_eq!(focus.index, 2);
    // At the very end of the text: past the last character of the last run.
    let f = frame_of(
        "ab\ncd",
        SelectionRange::caret(5),
        EditorConfig::default(),
        Tier::Full,
    );
    let d = declare(&A11yState::default(), &f);
    let last = run_keys(&d)[1];
    assert_eq!(
        d.selection.unwrap().1,
        Pos {
            key: ChildKey::Run {
                line: last.0,
                run: last.1
            },
            index: 2
        }
    );
}

#[test]
fn a_selection_reaching_outside_the_window_is_clamped_to_it() {
    let text = lines(1000);
    let buf = TextBuffer::from_str(&text);
    // From line 10 to line 500: the window is around the head (line 500), lines 400..=600.
    let anchor = buf.line_to_byte(10);
    let head = buf.line_to_byte(500) + 3;
    let f = frame_of(
        &text,
        SelectionRange::new(anchor, head),
        EditorConfig::default(),
        Tier::Full,
    );
    let d = declare(&A11yState::default(), &f);
    let keys = run_keys(&d);
    let (a, h) = d.selection.unwrap();
    assert_eq!(
        a.key,
        ChildKey::Run {
            line: keys[0].0,
            run: 0
        }
    );
    assert_eq!(a.index, 0);
    assert_eq!(
        h.key,
        ChildKey::Run {
            line: keys[100].0,
            run: 0
        }
    );
    assert_eq!(h.index, 3);
}

#[test]
fn children_are_text_runs_with_their_text() {
    let f = frame_of(
        "SELECT 1;\r\nSELECT ğ;",
        SelectionRange::caret(0),
        EditorConfig::default(),
        Tier::Full,
    );
    let d = declare(&A11yState::default(), &f);
    assert_eq!(d.role, Role::MultilineTextInput);
    let runs: Vec<&accesskit::Node> = d.children.iter().map(|(_, n)| n).collect();
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|n| n.role() == Role::TextRun));
    assert_eq!(runs[0].value(), Some("SELECT 1;\r\n"));
    assert_eq!(runs[0].character_lengths().len(), 10);
    assert_eq!(runs[1].value(), Some("SELECT ğ;"));
    assert_eq!(runs[1].character_lengths(), [1, 1, 1, 1, 1, 1, 1, 2, 1]);
}

#[test]
fn optimized_mode_grows_the_window_and_reduced_mode_does_not() {
    let text = lines(1000);
    let optimized = EditorConfig {
        a11y_optimized: true,
        ..EditorConfig::default()
    };
    let lines_in = |config, tier| {
        let f = frame_of(&text, SelectionRange::caret(0), config, tier);
        let keys = run_keys(&declare(&A11yState::default(), &f));
        keys.len()
    };
    // The whole document (1000 lines and the empty last one) up to 2 MB (D-073)...
    assert_eq!(lines_in(optimized, Tier::Full), 1001);
    // ...but not in reduced mode, and not by default.
    assert_eq!(lines_in(optimized, Tier::Reduced), 101);
    assert_eq!(lines_in(EditorConfig::default(), Tier::Full), 101);
    // Past 2 MB the optimised mode falls back to the window.
    let big = "SELECT 1;\n".repeat(a11y::OPTIMIZED_MAX_BYTES / 10 + 1);
    let f = frame_of(&big, SelectionRange::caret(0), optimized, Tier::Full);
    assert_eq!(run_keys(&declare(&A11yState::default(), &f)).len(), 101);
}

#[test]
fn the_label_description_and_read_only_go_on_the_editor_node() {
    let ro = EditorConfig {
        read_only: true,
        ..EditorConfig::default()
    };
    let f = frame_of("SELECT 1", SelectionRange::caret(0), ro, Tier::Reduced);
    let d = a11y::declaration(
        &A11yState::default(),
        &f,
        &ChangeLog::default(),
        Some("Reduced mode".into()),
        t0(),
    );
    let mut node = accesskit::Node::new(d.role);
    d.write_parent(&mut node);
    assert_eq!(node.label(), Some("Console 1"));
    assert_eq!(node.description(), Some("Reduced mode"));
    assert!(node.is_read_only());
    let f = frame_of(
        "SELECT 1",
        SelectionRange::caret(0),
        EditorConfig::default(),
        Tier::Full,
    );
    let mut node = accesskit::Node::new(Role::MultilineTextInput);
    declare(&A11yState::default(), &f).write_parent(&mut node);
    assert!(!node.is_read_only());
    assert_eq!(node.description(), None);
}

// ---- stable keys -----------------------------------------------------------------------------

fn edit(m: &mut EditorModel, at: std::ops::Range<usize>, text: &str) {
    let len = m.snapshot().buffer.len_bytes();
    let changes = ChangeSet::from_edits(len, [(at, Arc::from(text))]).unwrap();
    m.dispatch(Transaction {
        changes,
        selection: None,
        ann: Annotations {
            user_event: UserEvent::Command("test"),
            add_to_history: true,
            time: Instant::now(),
            origin: Origin::Program,
        },
    })
    .unwrap();
}

/// `(line, key)` of the window around `caret_line`.
fn keys_at(a: &A11yState, m: &EditorModel, caret_line: usize) -> Vec<(usize, u64)> {
    a11y::keyed_window(a, &m.snapshot(), m.changelog(), caret_line, 100)
        .iter()
        .map(|n| (n.line, n.key))
        .collect()
}

#[test]
fn keys_stay_stable_when_lines_are_inserted_above() {
    let mut m = EditorModel::new(&lines(1000), ModelConfig::default());
    let a = A11yState::default();
    let before = keys_at(&a, &m, 500);
    // Keys are unique and non-zero.
    let mut seen: Vec<u64> = before.iter().map(|(_, k)| *k).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), before.len());
    assert!(!seen.contains(&0));
    // Three lines inserted above the window, then one inside it, above line 450.
    edit(&mut m, 0..0, "-- a\n-- b\n-- c\n");
    let at = m.snapshot().buffer.line_to_byte(453);
    edit(&mut m, at..at, "-- new\n");
    let after = keys_at(&a, &m, 504);
    let key_of =
        |ks: &[(usize, u64)], line: usize| ks.iter().find(|(l, _)| *l == line).map(|(_, k)| *k);
    // Every old line still in the window keeps its key at its new line number.
    for line in 401..=600 {
        let moved = if line < 450 { line + 3 } else { line + 4 };
        assert_eq!(key_of(&before, line), key_of(&after, moved), "line {line}");
    }
    // The inserted line got a key nobody had.
    let fresh = key_of(&after, 453).unwrap();
    assert!(!seen.contains(&fresh));
    // Typing at a line's start, or joining it to the line above, keeps the upper line's key.
    let k500 = key_of(&after, 504).unwrap();
    let start = m.snapshot().buffer.line_to_byte(504);
    edit(&mut m, start..start, "x");
    assert_eq!(key_of(&keys_at(&a, &m, 504), 504), Some(k500));
    let k503 = key_of(&keys_at(&a, &m, 504), 503).unwrap();
    edit(&mut m, start - 1..start, "");
    let joined = keys_at(&a, &m, 503);
    assert_eq!(key_of(&joined, 503), Some(k503));
    assert!(
        !joined.iter().any(|(_, k)| *k == k500),
        "the joined line's key is gone"
    );
    // Rebuilding at the same version reuses the window (no new work per frame).
    let built = a11y::stats(&a).windows_built;
    keys_at(&a, &m, 503);
    assert_eq!(a11y::stats(&a).windows_built, built);
}

#[test]
fn keys_start_over_when_the_change_log_no_longer_reaches_back() {
    let mut m = EditorModel::new(&lines(300), ModelConfig::default());
    let a = A11yState::default();
    let before = keys_at(&a, &m, 100);
    // More changes than the log keeps (256), all far below the window.
    for _ in 0..300 {
        let end = m.snapshot().buffer.len_bytes();
        edit(&mut m, end..end, "x");
    }
    let after = keys_at(&a, &m, 100);
    let old: Vec<u64> = before.iter().map(|(_, k)| *k).collect();
    assert!(
        after.iter().all(|(_, k)| !old.contains(k)),
        "stale keys are never reused"
    );
}

// ---- the live region ------------------------------------------------------------------------

fn live(d: &Declaration) -> Option<(u64, accesskit::Node)> {
    d.children.iter().find_map(|(k, n)| match k {
        ChildKey::Live(seq) => Some((*seq, n.clone())),
        ChildKey::Run { .. } => None,
    })
}

#[test]
fn announcements_are_polite_and_each_one_is_a_new_node() {
    let a = A11yState::default();
    let f = frame_of(
        "SELECT 1",
        SelectionRange::caret(0),
        EditorConfig::default(),
        Tier::Full,
    );
    assert!(
        live(&declare(&a, &f)).is_none(),
        "no announcement, no live node"
    );
    a11y::announce(&a, "Statement 1 executed".into());
    let (first, node) = live(&declare(&a, &f)).expect("a live node");
    assert_eq!(node.role(), Role::Status);
    assert_eq!(node.live(), Some(accesskit::Live::Polite));
    assert_eq!(node.value(), Some("Statement 1 executed"));
    // ## m3a-t29 fix round 1 (I1): Windows and Linux announce a live region by its name.
    assert_eq!(node.label(), Some("Statement 1 executed"));
    // The node stays, unchanged, on later frames (nothing is said twice).
    assert_eq!(live(&declare(&a, &f)).map(|(k, _)| k), Some(first));
    assert!(a.nodes().pending.is_empty());
    // The same sentence again is a new node, so it is read again; two at once are one node.
    a11y::announce(&a, "Statement 1 executed".into());
    a11y::announce(&a, "Statement 2 failed".into());
    let (second, node) = live(&declare(&a, &f)).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        node.value(),
        Some("Statement 1 executed\nStatement 2 failed")
    );
}

// ---- in a window ------------------------------------------------------------------------------

/// A window whose root holds one editor and a sibling; `active` stands in for
/// `Window::is_a11y_active` in [`a11y::embed_with`].
struct Host {
    editor: Entity<EditorView>,
    sibling: Entity<Sibling>,
    active: bool,
}

struct Sibling;

impl Render for Sibling {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(800.0)).h(px(40.0))
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .child(div().w(px(800.0)).h(px(500.0)).child(a11y::embed_with(
                &self.editor,
                StyleRefinement::default().size_full(),
                self.active,
            )))
            .child(AnyView::from(self.sibling.clone()).cached(StyleRefinement::default()))
    }
}

struct Opened<'a> {
    editor: Entity<EditorView>,
    sibling: Entity<Sibling>,
    label_reads: Rc<Cell<u32>>,
    vcx: &'a mut VisualTestContext,
}

fn open_with<'a>(
    cx: &'a mut TestAppContext,
    text: &'static str,
    cfg: EditorConfig,
    active: bool,
) -> Opened<'a> {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(UiText(Arc::new(EnglishDefaults::new())));
    });
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let reads = Rc::new(Cell::new(0));
    let (out, counted) = (slot.clone(), reads.clone());
    let (host, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(29),
                text,
                cfg,
                Rc::new(FailClosedService),
                Rc::new(move |_: &gpui::App| {
                    counted.set(counted.get() + 1);
                    "Console 1".into()
                }),
                window,
                cx,
            )
        });
        *out.borrow_mut() = Some(editor.clone());
        Host {
            editor,
            sibling: cx.new(|_| Sibling),
            active,
        }
    });
    let editor = slot.borrow().clone().unwrap();
    let sibling = vcx.read(|cx| host.read(cx).sibling.clone());
    draw(vcx);
    Opened {
        editor,
        sibling,
        label_reads: reads,
        vcx,
    }
}

/// Fluent's bidi isolation marks removed (the English defaults wrap placeables in them).
fn plain(s: &str) -> String {
    s.replace(['\u{2068}', '\u{2069}'], "")
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

#[gpui::test]
fn editor_declares_multiline_text_input_with_its_label(cx: &mut TestAppContext) {
    let o = open_with(cx, "SELECT 1;\nSELECT 2;", EditorConfig::default(), false);
    let d = o
        .editor
        .update(o.vcx, |v, cx| a11y::declaration_of(v, true, cx));
    assert_eq!(d.role, Role::MultilineTextInput);
    assert_eq!(d.label.as_ref(), "Console 1");
    assert_eq!(run_keys(&d).len(), 2);
    let mut node = accesskit::Node::new(d.role);
    d.write_parent(&mut node);
    assert_eq!(node.label(), Some("Console 1"));
    // The label a host passes: the console's name inside the editor's own words.
    let label = a11y::label("Console 3".into());
    let text = o.vcx.read(|cx| label(cx));
    assert_eq!(plain(&text), "SQL editor, Console 3");
}

#[gpui::test]
fn nothing_is_built_without_assistive_technology(cx: &mut TestAppContext) {
    let o = open_with(cx, "SELECT 1;\nSELECT 2;", EditorConfig::default(), false);
    o.editor.update_in(o.vcx, |v, window, cx| {
        let f = gpui::Focusable::focus_handle(v, cx);
        window.focus(&f, cx);
    });
    for i in 0..3 {
        o.editor.update_in(o.vcx, |v, window, cx| {
            use gpui::EntityInputHandler as _;
            v.replace_text_in_range(None, &i.to_string(), window, cx);
        });
        draw(o.vcx);
    }
    o.editor
        .update(o.vcx, |v, cx| v.announce("Statement 1 executed".into(), cx));
    draw(o.vcx);
    assert_eq!(o.label_reads.get(), 0, "the label is never evaluated");
    let a = o.vcx.read(|cx| a11y::state(o.editor.read(cx)).clone());
    assert_eq!(a11y::stats(&a).windows_built, 0, "no window is built");
    assert_eq!(a11y::stats(&a).keyed_lines, 0, "no line is anchored");
    // An announcement nobody could hear is dropped at the next frame, not read out later.
    assert!(a.nodes().pending.is_empty());
}

#[gpui::test]
fn execution_status_is_announced_politely(cx: &mut TestAppContext) {
    let o = open_with(cx, "SELECT 1;\nSELECT 2;", EditorConfig::default(), false);
    let notified = Rc::new(Cell::new(0));
    let count = notified.clone();
    o.vcx.update(|_, cx| {
        cx.observe(&o.editor, move |_, _| count.set(count.get() + 1))
            .detach();
    });
    let failed = ExecMarkState::Failed {
        sqlstate: Some("42P01".into()),
    };
    let said = o
        .vcx
        .read(|cx| a11y::exec_announcement(cx, 2, &failed))
        .expect("a failure is announced");
    assert_eq!(plain(&said), "Statement 2 failed: 42P01");
    // `announce` asks for a frame: the live region reaches assistive technology now, not at
    // the next unrelated repaint. That frame's declaration holds it.
    let d = o.editor.update(o.vcx, |v, cx| {
        v.announce(said.clone(), cx);
        a11y::declaration_of(v, true, cx)
    });
    assert_eq!(notified.get(), 1);
    let (_, node) = live(&d).expect("the live region");
    assert_eq!(node.live(), Some(accesskit::Live::Polite));
    assert_eq!(
        node.value().map(plain).as_deref(),
        Some("Statement 2 failed: 42P01")
    );
    // What each state says (queued and cancelling say nothing).
    let say = |state: ExecMarkState| {
        o.vcx
            .read(|cx| a11y::exec_announcement(cx, 3, &state))
            .map(|s: gpui::SharedString| plain(s.as_ref()))
    };
    let now = Instant::now();
    assert_eq!(say(ExecMarkState::Queued), None);
    assert_eq!(say(ExecMarkState::Cancelling { since: now }), None);
    assert_eq!(
        say(ExecMarkState::Running { started: now }).as_deref(),
        Some("Statement 3 is running")
    );
    let ok = |rows| ExecMarkState::Succeeded {
        took: std::time::Duration::from_millis(5),
        rows,
    };
    assert_eq!(say(ok(None)).as_deref(), Some("Statement 3 executed"));
    assert_eq!(
        say(ok(Some(RowsOutcome::Fetched {
            rows: 1,
            more: false
        })))
        .as_deref(),
        Some("Statement 3 executed: 1 row")
    );
    assert_eq!(
        say(ok(Some(RowsOutcome::Fetched {
            rows: 40,
            more: true
        })))
        .as_deref(),
        Some("Statement 3 executed: 40+ rows")
    );
    assert_eq!(
        say(ok(Some(RowsOutcome::Affected(7)))).as_deref(),
        Some("Statement 3 executed: 7 rows affected")
    );
    assert_eq!(
        say(ExecMarkState::Failed { sqlstate: None }).as_deref(),
        Some("Statement 3 failed")
    );
    assert_eq!(
        say(ExecMarkState::Cancelled).as_deref(),
        Some("Statement 3 cancelled")
    );
    assert_eq!(
        say(ExecMarkState::Skipped).as_deref(),
        Some("Statement 3 skipped: an earlier statement failed")
    );
}

/// Counts painted frames.
#[derive(Default)]
struct Painted(Cell<u32>);

impl FrameProbe for Painted {
    fn input(&self, _: Instant) {}
    fn painted(&self, _: &mut gpui::App) {
        self.0.set(self.0.get() + 1);
    }
}

/// GPUI's `reuse_prepaint`/`reuse_paint` carry no accessibility state: on a frame where a
/// cached editor is reused, its node (built while it prepaints) would drop out of the tree.
/// `a11y::embed_with` caches the editor only while no assistive technology is attached, so
/// with it attached the editor prepaints, and declares its node, on every frame.
#[gpui::test]
fn the_node_survives_a_frame_where_only_a_sibling_changes(cx: &mut TestAppContext) {
    for (active, repaints) in [(false, 0), (true, 1)] {
        let o = open_with(cx, "SELECT 1;", EditorConfig::default(), active);
        let probe = Rc::new(Painted::default());
        let p: Rc<dyn FrameProbe> = probe.clone();
        o.editor.update(o.vcx, |v, _| v.set_probe(Some(p)));
        draw(o.vcx);
        let before = probe.0.get();
        o.sibling.update(o.vcx, |_, cx| cx.notify());
        o.vcx.run_until_parked();
        assert_eq!(
            probe.0.get() - before,
            repaints,
            "assistive technology: {active}"
        );
    }
}

// ## m3a-t29 fix round 1 ------------------------------------------------------------------------

/// I2: the live region leaves the tree once read, so the editor's subtree re-entering the tree
/// later does not read an old status again.
#[test]
fn the_live_region_goes_after_its_hold() {
    let a = A11yState::default();
    let f = frame_of(
        "SELECT 1",
        SelectionRange::caret(0),
        EditorConfig::default(),
        Tier::Full,
    );
    a11y::announce(&a, "Statement 2 failed: 42P01".into());
    let (first, _) = live(&declare_at(&a, &f, t0())).expect("declared");
    let just_before = t0() + a11y::LIVE_HOLD - std::time::Duration::from_millis(1);
    assert_eq!(
        live(&declare_at(&a, &f, just_before)).map(|(k, _)| k),
        Some(first)
    );
    assert!(live(&declare_at(&a, &f, t0() + a11y::LIVE_HOLD)).is_none());
    assert!(live(&declare_at(&a, &f, t0() + a11y::LIVE_HOLD * 10)).is_none());
    // A later announcement is held from its own first declaration.
    let later = t0() + a11y::LIVE_HOLD * 20;
    a11y::announce(&a, "Statement 3 executed".into());
    let (second, node) = live(&declare_at(&a, &f, later)).expect("declared");
    assert_ne!(first, second);
    assert_eq!(node.value(), Some("Statement 3 executed"));
    assert!(live(&declare_at(&a, &f, later + a11y::LIVE_HOLD / 2)).is_some());
}

/// I2: a console tab shown again after the hold declares no live region. End to end on the
/// headless platform, where the frames between the two declarations run without assistive
/// technology and clear the region too; the hold itself, on the clock `declaration_of` reads,
/// is `the_live_region_goes_after_its_hold`.
#[gpui::test]
fn a_tab_shown_again_does_not_re_announce(cx: &mut TestAppContext) {
    let o = open_with(cx, "SELECT 1;", EditorConfig::default(), false);
    let d = o.editor.update(o.vcx, |v, cx| {
        v.announce("Statement 1 executed".into(), cx);
        a11y::declaration_of(v, true, cx)
    });
    assert!(live(&d).is_some());
    o.vcx.executor().advance_clock(a11y::LIVE_HOLD);
    let d = o
        .editor
        .update(o.vcx, |v, cx| a11y::declaration_of(v, true, cx));
    assert!(live(&d).is_none(), "the old status is not read again");
}

/// I2: assistive technology detaching and re-attaching does not read the last status again: a
/// frame without it clears the live region.
#[gpui::test]
fn assistive_technology_re_attaching_does_not_re_announce(cx: &mut TestAppContext) {
    let o = open_with(cx, "SELECT 1;", EditorConfig::default(), false);
    let d = o.editor.update(o.vcx, |v, cx| {
        v.announce("Statement 1 executed".into(), cx);
        a11y::declaration_of(v, true, cx)
    });
    assert!(live(&d).is_some());
    // A frame without assistive technology (the test platform never attaches one).
    draw(o.vcx);
    let d = o
        .editor
        .update(o.vcx, |v, cx| a11y::declaration_of(v, true, cx));
    assert!(live(&d).is_none());
}

/// I3: a caret at the very end of a long last line is on that line: the exposed 64 KB end at
/// the caret, and the reader's caret is after the last character.
#[test]
fn a_long_last_line_with_the_caret_at_its_end() {
    // Letters cycling a–z, so which 64 KB is exposed shows in the text.
    let text: String = (0..200_000u32)
        .map(|i| char::from(b'a' + u8::try_from(i % 26).unwrap()))
        .collect();
    let f = frame_of(
        &text,
        SelectionRange::caret(200_000),
        EditorConfig::default(),
        Tier::Reduced,
    );
    let d = declare(&A11yState::default(), &f);
    let runs: Vec<(ChildKey, &accesskit::Node)> = d
        .children
        .iter()
        .filter(|(k, _)| matches!(k, ChildKey::Run { .. }))
        .map(|(k, n)| (*k, n))
        .collect();
    let exposed: String = runs.iter().map(|(_, n)| n.value().unwrap()).collect();
    assert_eq!(exposed.len(), a11y::LINE_MAX_BYTES);
    assert!(
        exposed == text[200_000 - a11y::LINE_MAX_BYTES..],
        "the exposed 64 KB end at the caret"
    );
    let (last_key, last) = runs.last().unwrap();
    let (anchor, focus) = d.selection.unwrap();
    assert_eq!(anchor, focus);
    assert_eq!(
        focus,
        Pos {
            key: *last_key,
            index: last.character_lengths().len()
        }
    );
}

/// M2: the default window exposes at most 256 KB, the lines nearest the caret first.
#[test]
fn the_window_is_capped_in_bytes() {
    let line = "x".repeat(50_000) + "\n";
    let text = line.repeat(30);
    let w = a11y::window(&TextBuffer::from_str(&text), 15, 100);
    let lines: Vec<usize> = w.iter().map(|n| n.line).collect();
    assert_eq!(lines, [13, 14, 15, 16, 17]);
    let bytes: usize = w.iter().map(|n| n.bytes.len()).sum();
    assert!(bytes <= a11y::WINDOW_MAX_BYTES);
    // The caret's line always shows, even past the budget on its own.
    let huge = "x".repeat(300_000) + "\nSELECT 1;";
    let w = a11y::window(&TextBuffer::from_str(&huge), 0, 100);
    assert_eq!(w[0].line, 0);
}

/// M2: the run nodes are built with the window, not per frame: frames at the same text and
/// lines (a blink, a selection inside the window) build none.
#[test]
fn run_nodes_are_built_once_per_window() {
    let a = A11yState::default();
    let text = lines(50);
    let f = frame_of(
        &text,
        SelectionRange::caret(0),
        EditorConfig::default(),
        Tier::Full,
    );
    let first = declare(&a, &f);
    assert_eq!(a11y::stats(&a).nodes_built, 1);
    let again = declare(&a, &f);
    let moved_frame = frame_of(
        &text,
        SelectionRange::new(3, 7),
        EditorConfig::default(),
        Tier::Full,
    );
    let moved = declare(&a, &moved_frame);
    assert_eq!(a11y::stats(&a).nodes_built, 1);
    assert_eq!(run_keys(&first), run_keys(&again));
    assert_eq!(run_keys(&first), run_keys(&moved));
    assert_ne!(first.selection, moved.selection);
}
