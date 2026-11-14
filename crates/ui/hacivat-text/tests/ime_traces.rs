//! UI-EDIT-12 model (M3a Task 10): the window-free IME core. Marked text is real buffer text
//! inside an IME history bracket; the platform's UTF-16 ranges convert through the buffer and
//! never split a surrogate pair or a CRLF; an app-switch blur leaves the composition with the
//! platform, an in-window focus move commits it; a programmatic edit commits it first.
//!
//! The M0 spike's 14 traces (`spikes/spike-editor/src/input.rs:362-496`) run here on the real
//! model, with their expectations in bytes (and the spike's UTF-16 ones kept beside them).

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_text::ime::{bytes_to_utf16, utf16_to_bytes};
use hacivat_text::{
    Annotations, ChangeSet, EditorModel, ImeCore, ModelConfig, Origin, Section, SelectionSet,
    Transaction, TransactionFilter, UserEvent, Veto,
};
use proptest::prelude::*;

/// A model over `text` with the caret at its end (the spike's start state, M0 ruling R4).
fn model(text: &str) -> EditorModel {
    let mut m = EditorModel::new(text, ModelConfig::default());
    let len = m.buffer().len_bytes();
    m.dispatch(select(len, SelectionSet::cursor(len))).unwrap();
    m
}

fn ann(user_event: UserEvent, time: Instant) -> Annotations {
    Annotations {
        user_event,
        add_to_history: true,
        time,
        origin: Origin::User,
    }
}

fn select(len: usize, sel: SelectionSet) -> Transaction {
    Transaction {
        changes: ChangeSet::identity(len),
        selection: Some(sel),
        ann: ann(UserEvent::Select, Instant::now()),
    }
}

fn insert(m: &EditorModel, at: usize, text: &str, time: Instant) -> Transaction {
    Transaction {
        changes: ChangeSet::from_edits(m.buffer().len_bytes(), [(at..at, Arc::from(text))])
            .unwrap(),
        selection: Some(SelectionSet::cursor(at + text.len())),
        ann: ann(UserEvent::InputType, time),
    }
}

fn text(m: &EditorModel) -> String {
    m.buffer().to_string()
}

/// The main selection in UTF-16 (what `selected_text_range` reports).
fn sel16(m: &EditorModel) -> Range<usize> {
    bytes_to_utf16(m.buffer(), m.selection().main().range())
}

/// One step of a trace.
#[derive(Debug, Clone)]
enum Step {
    /// `replace_and_mark_text_in_range(range, text, selected)`.
    Mark(Option<Range<usize>>, &'static str, Option<Range<usize>>),
    /// `replace_text_in_range(range, text)`.
    Commit(Option<Range<usize>>, &'static str),
    Unmark,
    /// Blur with the window active or not; whether it committed.
    Blur(bool, bool),
    Text(&'static str),
    /// The marked text's bytes.
    Marked(Option<Range<usize>>),
    /// The marked text in UTF-16 (the spike's assertion).
    Marked16(Option<Range<usize>>),
    /// The main selection's bytes.
    Sel(Range<usize>),
    /// The main selection in UTF-16 (the spike's assertion).
    Sel16(Range<usize>),
    /// `text_for_range(range16)` → its text and the adjusted UTF-16 range.
    TextFor(Range<usize>, &'static str, Range<usize>),
}

use Step::*;

struct Trace {
    name: &'static str,
    start: &'static str,
    steps: Vec<Step>,
}

fn trace(name: &'static str, start: &'static str, steps: Vec<Step>) -> Trace {
    Trace { name, start, steps }
}

/// The M0 spike's tests, in their order.
fn spike_traces() -> Vec<Trace> {
    vec![
        trace(
            "replace_and_mark_sets_marked_range",
            "SELECT ",
            vec![
                Mark(None, "ç", None),
                Marked(Some(7..9)),
                Marked16(Some(7..8)),
            ],
        ),
        trace(
            "unmark_after_focus_loss_commits_marked_text",
            "x",
            vec![Mark(None, "ş", None), Unmark, Text("xş"), Marked(None)],
        ),
        trace(
            "replace_in_marked_range_replaces_composition",
            "",
            vec![
                Mark(None, "k", None),
                Mark(None, "か", None),
                Commit(None, "感"),
                Text("感"),
                Marked(None),
                Sel(3..3),
            ],
        ),
        trace(
            "dead_key_sequence_produces_single_char",
            "",
            vec![Mark(None, "^", None), Commit(None, "â"), Text("â")],
        ),
        trace(
            "utf16_offsets_count_surrogate_pairs",
            "😀",
            vec![
                Mark(None, "ş", None),
                Marked(Some(4..6)),
                Marked16(Some(2..3)),
                Sel(6..6),
                Sel16(3..3),
            ],
        ),
        trace(
            "composition_selection_is_relative_to_the_marked_text",
            "ab",
            vec![Mark(None, "かな", Some(0..1)), Sel(2..5), Sel16(2..3)],
        ),
        trace(
            "explicit_range_wins_over_composition",
            "abc",
            vec![
                Mark(None, "x", None),
                Commit(Some(0..1), "Z"),
                Text("Zbcx"),
                Marked(None),
            ],
        ),
        trace(
            "blur_duplication_app_switch_keeps_composition_for_the_platform",
            "SELECT ",
            vec![
                Mark(None, "感じ", Some(2..2)),
                Blur(false, false),
                Marked(Some(7..13)),
                Marked16(Some(7..9)),
                Mark(None, "幹事", Some(2..2)),
                Commit(None, "幹事"),
                Text("SELECT 幹事"),
            ],
        ),
        trace(
            "blur_duplication_ime_commit_on_return_replaces_composition",
            "",
            vec![
                Mark(None, "感じ", Some(2..2)),
                Blur(false, false),
                Commit(None, "感じ"),
                Text("感じ"),
                Marked(None),
            ],
        ),
        trace(
            "blur_within_active_window_commits_composition",
            "x",
            vec![
                Mark(None, "ş", None),
                Blur(true, true),
                Text("xş"),
                Marked(None),
            ],
        ),
        trace(
            "dead_key_commit_with_nsnotfound_replaces_the_marked_dead_key",
            "SELECT ",
            vec![
                Mark(None, "ʼ", Some(1..1)),
                Commit(None, "ʼa"),
                Text("SELECT ʼa"),
                Marked(None),
                Sel(10..10),
                Sel16(9..9),
            ],
        ),
        trace(
            "dead_key_commit_with_explicit_replacement_range_replaces_the_mark",
            "x",
            vec![
                Mark(None, "ˆ", Some(1..1)),
                Commit(Some(1..2), "â"),
                Text("xâ"),
            ],
        ),
        trace(
            "dead_key_split_commit_mark_then_letter_yields_both_once",
            "",
            vec![
                Mark(None, "ʼ", Some(1..1)),
                Commit(None, "ʼ"),
                Commit(None, "a"),
                Text("ʼa"),
            ],
        ),
        trace(
            "text_for_range_clamps_and_reports_the_adjusted_range",
            "İş",
            vec![TextFor(1..99, "ş", 1..2)],
        ),
    ]
}

/// Runs `t` and checks the core's invariants after every step: the marked range holds the
/// last marked text, and the selection sits on character boundaries.
fn run_trace(t: &Trace) {
    let mut m = model(t.start);
    let mut ime = ImeCore::default();
    let mut last_mark: Option<&str> = None;
    for (i, step) in t.steps.iter().enumerate() {
        let at = format!("{} step {i} ({step:?})", t.name);
        match step.clone() {
            Mark(r, s, sel) => {
                ime.mark(&mut m, r, s, sel).expect(&at);
                last_mark = (!s.is_empty()).then_some(s);
            }
            Commit(r, s) => {
                ime.commit(&mut m, r, s).expect(&at);
                last_mark = None;
            }
            Unmark => {
                ime.unmark(&mut m);
                last_mark = None;
            }
            Blur(active, committed) => {
                assert_eq!(ime.blur(&mut m, active), committed, "{at}");
                if committed {
                    last_mark = None;
                }
            }
            Text(s) => assert_eq!(text(&m), s, "{at}"),
            Marked(r) => assert_eq!(ime.marked(&m), r, "{at}"),
            Marked16(r) => assert_eq!(ime.marked_utf16(m.buffer()), r, "{at}"),
            Sel(r) => assert_eq!(m.selection().main().range(), r, "{at}"),
            Sel16(r) => assert_eq!(sel16(&m), r, "{at}"),
            TextFor(r, s, adjusted) => {
                let mut adj = None;
                assert_eq!(ime.text_for_range(m.buffer(), r, &mut adj), s, "{at}");
                assert_eq!(adj, Some(adjusted), "{at}");
            }
        }
        assert_eq!(ime.is_composing(), last_mark.is_some(), "{at}");
        if let (Some(r), Some(s)) = (ime.marked(&m), last_mark) {
            assert_eq!(
                m.buffer().slice(r),
                s,
                "{at}: the marked range holds the marked text"
            );
        }
        let main = m.selection().main();
        assert!(
            m.buffer().is_char_boundary(main.anchor) && m.buffer().is_char_boundary(main.head),
            "{at}"
        );
    }
}

#[test]
fn spike_ime_traces_on_byte_offsets() {
    let traces = spike_traces();
    assert_eq!(traces.len(), 14, "every test of the spike's input.rs");
    for t in &traces {
        run_trace(t);
    }
}

/// An empty document composing `marked`.
fn composing(marked: &str) -> (ImeCore, EditorModel) {
    let mut m = model("");
    let mut ime = ImeCore::default();
    ime.mark(&mut m, None, marked, None).unwrap();
    (ime, m)
}

#[test]
fn marked_text_across_a_surrogate_pair_reports_the_adjusted_range() {
    let (ime, m) = composing("a😀b");
    let mut adj = None;
    let s = ime.text_for_range(&m.snapshot().buffer, 2..3, &mut adj);
    assert_eq!(adj, Some(1..3));
    assert_eq!(s, "😀");
    // UI-EDIT-12 test 2: `😀|`, `text_for_range(1..2)` snaps out to `0..2`.
    let m = model("😀");
    let mut adj = None;
    assert_eq!(ime.text_for_range(m.buffer(), 1..2, &mut adj), "😀");
    assert_eq!(adj, Some(0..2));
    assert_eq!(sel16(&m), 2..2);
}

#[test]
fn utf16_ranges_never_split_a_surrogate_pair_or_a_crlf() {
    let m = model("a\r\n😀b");
    let b = m.buffer();
    // Bytes: a 0, CR 1, LF 2, 😀 3..7, b 7. UTF-16: a 0, CR 1, LF 2, 😀 3..5, b 5.
    assert_eq!(
        utf16_to_bytes(b, 2..3),
        1..3,
        "half a CRLF widens to all of it"
    );
    assert_eq!(
        utf16_to_bytes(b, 4..5),
        3..7,
        "half a pair widens to all of it"
    );
    assert_eq!(
        utf16_to_bytes(b, 4..4),
        3..3,
        "a caret inside a pair goes before it"
    );
    assert_eq!(
        utf16_to_bytes(b, 2..2),
        1..1,
        "a caret inside a CRLF goes before it"
    );
    assert_eq!(utf16_to_bytes(b, 5..99), 7..8, "past the end clamps");
    assert_eq!(
        utf16_to_bytes(b, Range { start: 4, end: 1 }),
        3..3,
        "a reversed range is its start"
    );
    assert_eq!(bytes_to_utf16(b, 3..7), 3..5);
}

#[test]
fn an_empty_mark_cancels_the_composition() {
    let mut m = model("SELECT ");
    let mut ime = ImeCore::default();
    ime.mark(&mut m, None, "k", None).unwrap();
    ime.mark(&mut m, None, "ky", None).unwrap();
    assert!(ime.is_composing());
    ime.mark(&mut m, None, "", None).unwrap();
    assert!(!ime.is_composing());
    assert_eq!(ime.marked(&m), None);
    assert_eq!(text(&m), "SELECT ");
    assert_eq!(m.selection().main().range(), 7..7);
    assert_eq!(
        m.undo_depth(),
        0,
        "a cancelled composition leaves no undo group"
    );
}

#[test]
fn a_mark_replacing_a_selection_deletes_it_once() {
    let mut m = model("abc");
    m.dispatch(select(
        3,
        SelectionSet::single(hacivat_text::SelectionRange::new(1, 2)),
    ))
    .unwrap();
    let mut ime = ImeCore::default();
    ime.mark(&mut m, None, "x", None).unwrap();
    assert_eq!(text(&m), "axc");
    ime.mark(&mut m, None, "xy", None).unwrap();
    assert_eq!(
        text(&m),
        "axyc",
        "the second mark replaces the marked text only"
    );
    ime.commit(&mut m, None, "XY").unwrap();
    assert_eq!(text(&m), "aXYc");
    assert_eq!(m.selection().main().range(), 3..3);
    // One undo group restores the selected text and the selection.
    assert!(m.undo().is_some());
    assert_eq!(text(&m), "abc");
    assert_eq!(m.selection().main().range(), 1..2);
    assert_eq!(m.undo_depth(), 0);
}

#[test]
fn programmatic_edit_during_composition_commits_first() {
    let mut m = model("SELECT ");
    let mut ime = ImeCore::default();
    ime.mark(&mut m, None, "感じ", Some(2..2)).unwrap();
    let tr = Transaction {
        ann: Annotations {
            origin: Origin::Program,
            ..ann(UserEvent::Command("format"), Instant::now())
        },
        ..insert(&m, 0, "-- ", Instant::now())
    };
    ime.dispatch(&mut m, tr).unwrap();
    assert!(!ime.is_composing(), "the composition was committed first");
    assert_eq!(text(&m), "-- SELECT 感じ");
    // The edit did not join the composition's group: two undo steps.
    assert_eq!(m.undo_depth(), 2);
    m.undo();
    assert_eq!(text(&m), "SELECT 感じ");
    m.undo();
    assert_eq!(text(&m), "SELECT ");
    // A new composition starts at the selection, never over the old marked text.
    let mut m = model("SELECT ");
    ime.mark(&mut m, None, "感", None).unwrap();
    let len = m.buffer().len_bytes();
    ime.dispatch(&mut m, select(len, SelectionSet::cursor(0)))
        .unwrap();
    ime.mark(&mut m, None, "x", None).unwrap();
    assert_eq!(text(&m), "xSELECT 感");
}

/// Composes, then blurs with the window active or not: whether the blur committed.
fn blur(window_active: bool) -> bool {
    let mut m = model("x");
    let mut ime = ImeCore::default();
    ime.mark(&mut m, None, "ş", None).unwrap();
    let committed = ime.blur(&mut m, window_active);
    assert_eq!(text(&m), "xş", "a blur never changes the text");
    assert_eq!(ime.is_composing(), !committed);
    committed
}

#[test]
fn app_switch_blur_keeps_the_composition_and_focus_move_commits() {
    assert!(!blur(false));
    assert!(blur(true));
    // Nothing to commit: a blur without a composition reports nothing.
    let mut m = model("x");
    assert!(!ImeCore::default().blur(&mut m, true));
}

#[test]
fn one_composition_is_one_undo_group_that_joins_no_typing() {
    let mut m = model("");
    let t0 = Instant::now();
    m.dispatch(insert(&m, 0, "a", t0)).unwrap();
    let mut ime = ImeCore::default();
    ime.mark(&mut m, None, "か", None).unwrap();
    ime.mark(&mut m, None, "かな", None).unwrap();
    ime.mark(&mut m, None, "仮名", None).unwrap();
    ime.commit(&mut m, None, "仮名").unwrap();
    m.dispatch(insert(&m, 7, "b", t0 + Duration::from_millis(1)))
        .unwrap();
    assert_eq!(text(&m), "a仮名b");
    assert_eq!(m.undo_depth(), 3, "typing, the composition, typing");
    m.undo();
    assert_eq!(text(&m), "a仮名");
    m.undo();
    assert_eq!(text(&m), "a", "the whole composition in one step");
    m.undo();
    assert_eq!(text(&m), "");
}

#[test]
fn reconversion_over_existing_text_is_one_undo_group() {
    let mut m = model("SELECT 感");
    let mut ime = ImeCore::default();
    // The IME marks the committed `感` (UTF-16 7..8) again, then commits another candidate.
    ime.mark(&mut m, Some(7..8), "かん", None).unwrap();
    assert_eq!(text(&m), "SELECT かん");
    assert_eq!(ime.marked(&m), Some(7..13));
    ime.commit(&mut m, None, "漢").unwrap();
    assert_eq!(text(&m), "SELECT 漢");
    m.undo();
    assert_eq!(text(&m), "SELECT 感");
}

#[test]
fn a_dotless_i_commit_takes_the_plain_typing_path() {
    // `ı` is a plain key (no composition): commits outside a composition are typing and join
    // the typing group.
    let mut m = model("");
    let mut ime = ImeCore::default();
    ime.commit(&mut m, None, "ı").unwrap();
    assert!(!ime.is_composing());
    ime.commit(&mut m, None, "s").unwrap();
    assert_eq!(text(&m), "ıs");
    assert_eq!(m.selection().main().range(), 3..3);
    assert_eq!(m.undo_depth(), 1);
}

#[test]
fn a_commit_with_an_explicit_range_outside_a_composition_replaces_it() {
    let mut m = model("SELECT x");
    let mut ime = ImeCore::default();
    ime.commit(&mut m, Some(7..8), "y").unwrap();
    assert_eq!(text(&m), "SELECT y");
    assert_eq!(m.selection().main().range(), 8..8);
}

#[test]
fn an_edit_that_bypassed_the_core_is_mapped_over() {
    // An insertion before the composition: the marked text is followed, so the platform's
    // next commit replaces it rather than landing beside it.
    let mut m = model("ab");
    let mut ime = ImeCore::default();
    ime.mark(&mut m, None, "ş", None).unwrap();
    m.dispatch(Transaction {
        selection: None,
        ..insert(&m, 0, "X", Instant::now())
    })
    .unwrap();
    assert_eq!(ime.marked(&m), Some(3..5));
    ime.commit(&mut m, None, "ş").unwrap();
    assert_eq!(text(&m), "Xabş");
    // An edit that deleted the marked text ends the composition.
    let mut m = model("ab");
    ime.mark(&mut m, None, "ş", None).unwrap();
    let len = m.buffer().len_bytes();
    m.dispatch(Transaction {
        changes: ChangeSet::from_edits(len, [(1..len, Arc::from(""))]).unwrap(),
        selection: None,
        ann: ann(UserEvent::DeleteSelection, Instant::now()),
    })
    .unwrap();
    assert_eq!(ime.marked(&m), None);
    ime.commit(&mut m, None, "z").unwrap();
    assert_eq!(text(&m), "az");
    assert!(!ime.is_composing());
}

/// Refuses any transaction that inserts `0`.
struct VetoText(&'static str);

impl TransactionFilter for VetoText {
    fn filter(&self, _m: &EditorModel, tr: Transaction) -> Result<Transaction, Veto> {
        let hit = tr
            .changes
            .sections()
            .iter()
            .any(|s| matches!(s, Section::Replace { ins, .. } if &**ins == self.0));
        if hit { Err(Veto("test-veto")) } else { Ok(tr) }
    }
}

#[test]
fn a_vetoed_first_mark_opens_no_composition() {
    let mut m = model("");
    m.add_filter(Box::new(VetoText("k")));
    let mut ime = ImeCore::default();
    assert_eq!(ime.mark(&mut m, None, "k", None), Err(Veto("test-veto")));
    assert!(!ime.is_composing());
    // No history bracket was left open: typing 2 s apart makes two groups, not one.
    let t0 = Instant::now();
    m.dispatch(insert(&m, 0, "a", t0)).unwrap();
    m.dispatch(insert(&m, 1, "b", t0 + Duration::from_secs(2)))
        .unwrap();
    assert_eq!(m.undo_depth(), 2);
}

#[test]
fn a_vetoed_mark_keeps_the_composition_and_a_vetoed_commit_ends_it() {
    let mut m = model("");
    m.add_filter(Box::new(VetoText("k")));
    let mut ime = ImeCore::default();
    ime.mark(&mut m, None, "a", None).unwrap();
    assert!(ime.mark(&mut m, None, "k", None).is_err());
    assert_eq!(
        ime.marked(&m),
        Some(0..1),
        "the composition stays as it was"
    );
    // The platform has ended its composition: so does the core, keeping the text.
    assert!(ime.commit(&mut m, None, "k").is_err());
    assert!(!ime.is_composing());
    assert_eq!(text(&m), "a");
}

/// Base text pieces and IME texts: every line break is a CRLF, so a split one shows.
fn arb_piece() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec!["a", "ş", "İ", "😀", "\r\n", "感", "e\u{301}", ""])
}

#[derive(Debug, Clone)]
enum Op {
    Mark(Option<Range<usize>>, &'static str, Option<Range<usize>>),
    Commit(Option<Range<usize>>, &'static str),
    Unmark,
    Blur(bool),
    Select(usize, usize),
}

fn arb_range16() -> impl Strategy<Value = Option<Range<usize>>> {
    prop::option::of((0usize..24, 0usize..6).prop_map(|(s, n)| s..s + n))
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (arb_range16(), arb_piece(), prop::option::of((0usize..3, 0usize..3).prop_map(|(s, n)| s..s + n)))
            .prop_map(|(r, t, s)| Op::Mark(r, t, s)),
        2 => (arb_range16(), arb_piece()).prop_map(|(r, t)| Op::Commit(r, t)),
        1 => Just(Op::Unmark),
        1 => any::<bool>().prop_map(Op::Blur),
        1 => (0usize..24, 0usize..24).prop_map(|(a, h)| Op::Select(a, h)),
    ]
}

/// `PROPTEST_CASES` when set (a stress run), else `default`.
fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(512), ..ProptestConfig::default() })]

    #[test]
    fn ime_sessions_keep_every_crlf_and_undo_back_to_the_start(
        start in proptest::collection::vec(arb_piece(), 0..8).prop_map(|v| v.concat()),
        ops in proptest::collection::vec(arb_op(), 1..24),
    ) {
        let mut m = model(&start);
        let mut ime = ImeCore::default();
        for op in ops {
            match op {
                Op::Mark(r, t, s) => { ime.mark(&mut m, r, t, s).unwrap(); }
                Op::Commit(r, t) => { ime.commit(&mut m, r, t).unwrap(); }
                Op::Unmark => ime.unmark(&mut m),
                Op::Blur(active) => { ime.blur(&mut m, active); }
                Op::Select(a, h) => {
                    let b = m.buffer();
                    let (a, h) = (utf16_to_bytes(b, a..a).start, utf16_to_bytes(b, h..h).start);
                    let len = b.len_bytes();
                    ime.dispatch(&mut m, select(len, SelectionSet::single(
                        hacivat_text::SelectionRange::new(a, h)))).unwrap();
                }
            }
            let t = text(&m);
            // No CRLF was ever split: every CR is followed by a LF and every LF follows a CR.
            prop_assert_eq!(t.matches('\r').count(), t.matches("\r\n").count(), "{:?}", t);
            prop_assert_eq!(t.matches('\n').count(), t.matches("\r\n").count(), "{:?}", t);
            if let Some(r) = ime.marked(&m) {
                prop_assert!(m.buffer().is_char_boundary(r.start) && m.buffer().is_char_boundary(r.end));
                prop_assert!(!r.is_empty());
            }
        }
        ime.unmark(&mut m);
        while m.undo().is_some() {}
        prop_assert_eq!(text(&m), start);
    }
}
