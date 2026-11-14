//! UI-EDIT-04 (M3a Task 5): grouped linear history. Typing, delete-backward and
//! delete-forward group within 500 ms when adjacent; whitespace or `;` ends a word's group;
//! paste, drop and undo/redo never join; brackets (IME, commands) nest; identity groups are
//! dropped; 1,000 groups and 64 MiB at most; seals on Run, focus, a non-adjacent move and
//! commands; non-history edits rebase the stored groups.
//!
//! Every transaction carries an explicit time from the test's own clock: no test reads the
//! wall clock.

use std::collections::HashSet;
use std::ops::{Deref, DerefMut, Range};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_text::{
    Annotations, Bias, ChangeSet, EditorModel, ModelConfig, Origin, SelectionRange, SelectionSet,
    Transaction, UserEvent,
};
use proptest::prelude::*;
use proptest::strategy::BoxedStrategy;

/// A model and the test's clock.
struct Ed {
    m: EditorModel,
    now: Instant,
}

impl Deref for Ed {
    type Target = EditorModel;
    fn deref(&self) -> &EditorModel {
        &self.m
    }
}

impl DerefMut for Ed {
    fn deref_mut(&mut self) -> &mut EditorModel {
        &mut self.m
    }
}

/// A model over `t` with the caret at its end.
fn model(t: &str) -> Ed {
    model_with(t, ModelConfig::default())
}

fn model_with(t: &str, cfg: ModelConfig) -> Ed {
    let mut e = Ed {
        m: EditorModel::new(t, cfg),
        now: Instant::now(),
    };
    let len = e.m.buffer().len_bytes();
    let tr = e.tr(
        ChangeSet::identity(len),
        Some(SelectionSet::cursor(len)),
        UserEvent::Select,
    );
    e.m.dispatch(tr).unwrap();
    e
}

impl Ed {
    fn tr(
        &mut self,
        changes: ChangeSet,
        selection: Option<SelectionSet>,
        user_event: UserEvent,
    ) -> Transaction {
        self.now += Duration::from_millis(10);
        Transaction {
            changes,
            selection,
            ann: Annotations {
                user_event,
                add_to_history: true,
                time: self.now,
                origin: Origin::User,
            },
        }
    }

    fn caret(&self) -> usize {
        self.m.selection().main().head
    }

    /// Replaces `r` with `text`, the caret after it.
    fn edit(&mut self, r: Range<usize>, text: &str, ev: UserEvent) {
        let len = self.m.buffer().len_bytes();
        let caret = r.start + text.len();
        let changes = ChangeSet::from_edits(len, [(r, Arc::from(text))]).unwrap();
        let tr = self.tr(changes, Some(SelectionSet::cursor(caret)), ev);
        self.m.dispatch(tr).unwrap();
    }

    fn move_to(&mut self, at: usize) {
        let len = self.m.buffer().len_bytes();
        let tr = self.tr(
            ChangeSet::identity(len),
            Some(SelectionSet::cursor(at)),
            UserEvent::Move,
        );
        self.m.dispatch(tr).unwrap();
    }

    fn wait(&mut self, ms: u64) {
        self.now += Duration::from_millis(ms);
    }
}

/// Types `s` one character at a time at the caret, 10 ms apart.
fn type_text(e: &mut Ed, s: &str) {
    for ch in s.chars() {
        let at = e.caret();
        e.edit(at..at, ch.encode_utf8(&mut [0; 4]), UserEvent::InputType);
    }
}

fn backspace(e: &mut Ed) {
    let at = e.caret();
    let text = e.buffer().to_string();
    let prev = text[..at].chars().next_back().map_or(0, char::len_utf8);
    e.edit(at - prev..at, "", UserEvent::DeleteBackward);
}

fn delete_forward(e: &mut Ed) {
    let at = e.caret();
    let text = e.buffer().to_string();
    let next = text[at..].chars().next().map_or(0, char::len_utf8);
    e.edit(at..at + next, "", UserEvent::DeleteForward);
}

fn paste(e: &mut Ed, s: &str) {
    let at = e.caret();
    e.edit(at..at, s, UserEvent::InputPaste);
}

fn text(e: &Ed) -> String {
    e.buffer().to_string()
}

/// Undoes until nothing is left, returning each text on the way.
fn undo_all(e: &mut Ed) -> Vec<String> {
    let mut out = Vec::new();
    while e.undo().is_some() {
        out.push(text(e));
    }
    out
}

#[test]
fn ui_edit_04_cases_1_to_8() {
    // 1. `SELECT` typed quickly is one group; undo empties the document, caret at 0.
    let mut e = model("");
    type_text(&mut e, "SELECT");
    assert_eq!(e.undo_depth(), 1);
    e.undo().unwrap();
    assert_eq!(text(&e), "");
    assert_eq!(e.caret(), 0);

    // 2. The word rule: `SELECT 1` is `SELECT ` then `1`.
    let mut e = model("");
    type_text(&mut e, "SELECT 1");
    assert_eq!(undo_all(&mut e), ["SELECT ", ""]);

    // 3. A 600 ms pause starts a new group.
    let mut e = model("");
    type_text(&mut e, "abc");
    e.wait(600);
    type_text(&mut e, "d");
    assert_eq!(undo_all(&mut e), ["abc", ""]);

    // 4. Three cursors type `x` in one transaction: one group; undo restores three carets.
    let mut e = model("a\nb\nc");
    let carets = SelectionSet::normalized([1, 3, 5].map(SelectionRange::caret), 2);
    let tr = e.tr(
        ChangeSet::identity(5),
        Some(carets.clone()),
        UserEvent::Select,
    );
    e.dispatch(tr).unwrap();
    let edits = [1, 3, 5].map(|at| (at..at, Arc::from("x")));
    let after = SelectionSet::normalized([2, 5, 8].map(SelectionRange::caret), 2);
    let tr = e.tr(
        ChangeSet::from_edits(5, edits).unwrap(),
        Some(after),
        UserEvent::InputType,
    );
    e.dispatch(tr).unwrap();
    assert_eq!(text(&e), "ax\nbx\ncx");
    assert_eq!(e.undo_depth(), 1);
    e.undo().unwrap();
    assert_eq!(text(&e), "a\nb\nc");
    assert_eq!(e.selection(), &carets);
    assert_eq!(e.undo_depth(), 0);

    // 5. An IME composition か → かな → 仮名 is one group inserting 仮名.
    let mut e = model("x");
    let b = e.begin_bracket(UserEvent::InputTypeCompose);
    e.edit(1..1, "か", UserEvent::InputTypeCompose);
    e.edit(1..4, "かな", UserEvent::InputTypeCompose);
    e.edit(1..7, "仮名", UserEvent::InputTypeCompose);
    e.end_bracket(b);
    assert_eq!(text(&e), "x仮名");
    assert_eq!(e.undo_depth(), 1);
    e.undo().unwrap();
    assert_eq!(text(&e), "x");
    e.redo().unwrap();
    assert_eq!(text(&e), "x仮名");
    assert_eq!(e.redo_depth(), 0);

    // 6. `a` then Backspace within 100 ms: the group is the identity and is dropped.
    let mut e = model("");
    type_text(&mut e, "a");
    backspace(&mut e);
    assert_eq!(text(&e), "");
    assert_eq!(e.undo_depth(), 0);
    assert!(e.undo().is_none());

    // 7. A Run between two bursts within 500 ms seals: two groups.
    let mut e = model("");
    type_text(&mut e, "ab");
    e.seal_history();
    type_text(&mut e, "cd");
    assert_eq!(undo_all(&mut e), ["ab", ""]);

    // 8. 1,001 separate groups: the oldest is evicted, 1,000 remain.
    let mut e = model("");
    for _ in 0..1_001 {
        paste(&mut e, "x");
    }
    assert_eq!(e.undo_depth(), 1_000);
    let texts = undo_all(&mut e);
    assert_eq!(texts.len(), 1_000);
    assert_eq!(text(&e), "x");
}

#[test]
fn run_seals_the_open_group() {
    let mut m = model("x");
    type_text(&mut m, "ab");
    m.seal_history();
    type_text(&mut m, "c");
    m.undo();
    assert_eq!(text(&m), "xab");
}

#[test]
fn the_word_rule_keeps_spaces_and_semicolons_with_the_word_before() {
    let mut e = model("");
    type_text(&mut e, "SELECT  1;x");
    assert_eq!(undo_all(&mut e), ["SELECT  1;", "SELECT  ", ""]);
    // Deleting the space reopens the word.
    let mut e = model("");
    type_text(&mut e, "ab ");
    backspace(&mut e);
    type_text(&mut e, "c");
    assert_eq!(undo_all(&mut e), [""]);
}

#[test]
fn paste_and_drop_never_join_and_nothing_joins_them() {
    let mut e = model("");
    type_text(&mut e, "ab");
    paste(&mut e, "XY");
    paste(&mut e, "Z");
    type_text(&mut e, "c");
    let at = e.caret();
    e.edit(at..at, "D", UserEvent::InputDrop);
    assert_eq!(text(&e), "abXYZcD");
    assert_eq!(undo_all(&mut e), ["abXYZc", "abXYZ", "abXY", "ab", ""]);
}

#[test]
fn deletes_group_by_their_own_family() {
    // A Backspace run is one group; typing after it is another.
    let mut e = model("SELECT 1");
    backspace(&mut e);
    backspace(&mut e);
    type_text(&mut e, "2");
    assert_eq!(undo_all(&mut e), ["SELECT", "SELECT 1"]);
    // Delete-forward runs group; mixing the two directions does not.
    let mut e = model("abcdef");
    e.move_to(2);
    delete_forward(&mut e);
    delete_forward(&mut e);
    backspace(&mut e);
    assert_eq!(text(&e), "aef");
    assert_eq!(undo_all(&mut e), ["abef", "abcdef"]);
    // Backspace past the start of what the group typed starts a new group.
    let mut e = model("ab");
    type_text(&mut e, "c");
    backspace(&mut e);
    backspace(&mut e);
    assert_eq!(text(&e), "a");
    assert_eq!(undo_all(&mut e), ["ab"]);
}

#[test]
fn a_non_adjacent_move_seals_and_an_adjacent_one_does_not() {
    let mut e = model("");
    type_text(&mut e, "abc");
    e.move_to(1);
    e.move_to(3);
    type_text(&mut e, "d");
    assert_eq!(undo_all(&mut e), ["abc", ""]);
    // A selection commit that leaves the caret where typing left it keeps the group open.
    let mut e = model("");
    type_text(&mut e, "abc");
    e.move_to(3);
    type_text(&mut e, "d");
    assert_eq!(undo_all(&mut e), [""]);
}

#[test]
fn brackets_nest_and_seal_when_the_outermost_ends() {
    let mut e = model("");
    let outer = e.begin_bracket(UserEvent::Command("editor.format"));
    e.edit(0..0, "a", UserEvent::InputPaste);
    let inner = e.begin_bracket(UserEvent::InputTypeCompose);
    e.edit(1..1, "b", UserEvent::InputTypeCompose);
    e.end_bracket(inner);
    e.wait(2_000);
    e.edit(0..0, "c", UserEvent::DeleteForward);
    e.end_bracket(outer);
    // Typing right after the bracket does not join it.
    type_text(&mut e, "d");
    assert_eq!(text(&e), "cdab");
    assert_eq!(undo_all(&mut e), ["cab", ""]);
    // Ending an unknown or already-ended bracket does nothing.
    e.end_bracket(outer);
    assert_eq!(e.undo_depth(), 0);
}

#[test]
fn a_cancelled_composition_leaves_no_group() {
    let mut e = model("");
    type_text(&mut e, "ab");
    e.seal_history();
    let b = e.begin_bracket(UserEvent::InputTypeCompose);
    e.edit(2..2, "か", UserEvent::InputTypeCompose);
    e.edit(2..5, "", UserEvent::InputTypeCompose);
    e.end_bracket(b);
    assert_eq!(text(&e), "ab");
    assert_eq!(e.undo_depth(), 1);
}

#[test]
fn undo_restores_selections_and_redo_reapplies_them() {
    let mut e = model("SELECT 1");
    let sel = SelectionSet::single(SelectionRange {
        anchor: 7,
        head: 8,
        goal_x: None,
        assoc: Bias::After,
    });
    let tr = e.tr(ChangeSet::identity(8), Some(sel.clone()), UserEvent::Select);
    e.dispatch(tr).unwrap();
    e.edit(7..8, "42", UserEvent::InputType);
    assert_eq!(e.caret(), 9);
    e.undo().unwrap();
    assert_eq!(text(&e), "SELECT 1");
    assert_eq!(e.selection(), &sel);
    e.redo().unwrap();
    assert_eq!(text(&e), "SELECT 42");
    assert_eq!(e.selection(), &SelectionSet::cursor(9));
    // Undo and redo never join what comes next.
    type_text(&mut e, "0");
    assert_eq!(undo_all(&mut e), ["SELECT 42", "SELECT 1"]);
}

#[test]
fn a_new_edit_clears_redo() {
    let mut e = model("");
    type_text(&mut e, "ab");
    e.undo().unwrap();
    assert_eq!(e.redo_depth(), 1);
    type_text(&mut e, "c");
    assert_eq!(e.redo_depth(), 0);
    assert!(e.redo().is_none());
}

#[test]
fn history_evicts_by_byte_budget() {
    // 17 pastes of 4 MiB are 68 MiB of history: the oldest goes, the newest 16 stay undoable.
    let chunk = "SELECT 1234567;\n".repeat((4 << 20) / 16);
    assert_eq!(chunk.len(), 4 << 20);
    let mut e = model("");
    for _ in 0..17 {
        paste(&mut e, &chunk);
    }
    assert_eq!(e.undo_depth(), 16);
    for _ in 0..16 {
        e.undo().unwrap();
    }
    assert!(e.undo().is_none());
    assert_eq!(e.buffer().len_bytes(), 4 << 20);
    // A single group over the budget is still kept (and evicts everything older).
    let mut e = model_with(
        "",
        ModelConfig {
            history_bytes: 1 << 10,
            ..ModelConfig::default()
        },
    );
    paste(&mut e, "y");
    paste(&mut e, &"z".repeat(4 << 10));
    assert_eq!(e.undo_depth(), 1);
    e.undo().unwrap();
    assert_eq!(text(&e), "y");
}

#[test]
fn programmatic_set_text_rebases_history() {
    let mut e = model("SELECT 1");
    type_text(&mut e, " + 2");
    // A non-history program edit above the typing.
    let tr = Transaction {
        changes: ChangeSet::from_edits(12, [(0..0, Arc::from("-- restored\n"))]).unwrap(),
        selection: None,
        ann: Annotations {
            user_event: UserEvent::Command("set_text"),
            add_to_history: false,
            time: e.now,
            origin: Origin::Program,
        },
    };
    e.dispatch(tr).unwrap();
    assert_eq!(text(&e), "-- restored\nSELECT 1 + 2");
    // `" "`, `"+ "`, `"2"` (the word rule).
    assert_eq!(e.undo_depth(), 3);
    // Undo removes the typing at its new place and keeps the program's text.
    assert_eq!(
        undo_all(&mut e),
        [
            "-- restored\nSELECT 1 + ",
            "-- restored\nSELECT 1 ",
            "-- restored\nSELECT 1"
        ]
    );
    // The restored caret moved with the text.
    assert_eq!(e.caret(), 20);
    while e.redo().is_some() {}
    assert_eq!(text(&e), "-- restored\nSELECT 1 + 2");
    assert_eq!(e.caret(), 24);
}

#[test]
fn a_program_edit_over_typed_text_drops_the_history_it_overlaps() {
    let mut e = model("");
    type_text(&mut e, "ab");
    e.seal_history();
    type_text(&mut e, "cd");
    // The program rewrites "bc": both groups touched it, so neither can undo safely.
    let tr = Transaction {
        changes: ChangeSet::from_edits(4, [(1..3, Arc::from("XY"))]).unwrap(),
        selection: None,
        ann: Annotations {
            user_event: UserEvent::Command("format"),
            add_to_history: false,
            time: e.now,
            origin: Origin::Service,
        },
    };
    e.dispatch(tr).unwrap();
    assert_eq!(text(&e), "aXYd");
    assert_eq!(e.undo_depth(), 0);
    assert!(e.undo().is_none());
    assert_eq!(text(&e), "aXYd");
    // History taken after the program edit works as usual.
    type_text(&mut e, "!");
    e.undo().unwrap();
    assert_eq!(text(&e), "aXYd");
}

#[test]
fn a_history_set_text_is_one_group() {
    let mut e = model("SELECT 1");
    let tr = e.tr(
        ChangeSet::from_edits(8, [(0..8, Arc::from("SELECT 2;\nSELECT 3;"))]).unwrap(),
        Some(SelectionSet::cursor(0)),
        UserEvent::Command("set_text"),
    );
    e.dispatch(tr).unwrap();
    type_text(&mut e, "x");
    assert_eq!(undo_all(&mut e), ["SELECT 2;\nSELECT 3;", "SELECT 1"]);
}

/// One step of a random editing session.
#[derive(Debug, Clone)]
enum Op {
    Type(char),
    Backspace,
    DeleteForward,
    Paste(String),
    /// Moves the caret to the n-th character boundary (modulo their count).
    Move(usize),
    Seal,
    Wait,
    Undo,
    Redo,
    /// An IME composition: each string replaces the marked text; the last one commits.
    Compose(Vec<String>),
    /// A non-history insertion at the n-th character boundary.
    Program(usize, String),
}

fn op_s(program: bool) -> BoxedStrategy<Op> {
    let ch = prop::sample::select(vec!['a', 'b', 'ç', 'İ', 'ı', '😀', ' ', ';', '\n', '\r']);
    let s = prop::collection::vec(
        prop::sample::select(vec!["x", "ğ", "😀", " ", "\r\n", ";"]),
        0..4,
    )
    .prop_map(|p| p.concat());
    let base = prop_oneof![
        6 => ch.prop_map(Op::Type),
        2 => Just(Op::Backspace),
        1 => Just(Op::DeleteForward),
        1 => s.clone().prop_map(Op::Paste),
        1 => (0..64usize).prop_map(Op::Move),
        1 => Just(Op::Seal),
        1 => Just(Op::Wait),
        2 => Just(Op::Undo),
        1 => Just(Op::Redo),
        1 => prop::collection::vec(s.clone(), 1..4).prop_map(Op::Compose),
    ];
    if program {
        let with_program = (0..64usize, s).prop_map(|(at, t)| Op::Program(at, t));
        prop_oneof![9 => base, 1 => with_program].boxed()
    } else {
        base.boxed()
    }
}

fn boundary(e: &Ed, n: usize) -> usize {
    let t = text(e);
    let b: Vec<usize> = t
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(t.len()))
        .collect();
    b[n % b.len()]
}

fn run_op(e: &mut Ed, op: &Op) {
    match op {
        Op::Type(c) => type_text(e, &c.to_string()),
        Op::Backspace if e.caret() > 0 => backspace(e),
        Op::DeleteForward if e.caret() < e.buffer().len_bytes() => delete_forward(e),
        Op::Backspace | Op::DeleteForward => {}
        Op::Paste(s) => paste(e, s),
        Op::Move(n) => {
            let at = boundary(e, *n);
            e.move_to(at);
        }
        Op::Seal => e.seal_history(),
        Op::Wait => e.wait(600),
        Op::Undo => {
            e.undo();
        }
        Op::Redo => {
            e.redo();
        }
        Op::Compose(marks) => {
            let start = e.caret();
            let b = e.begin_bracket(UserEvent::InputTypeCompose);
            let mut len = 0;
            for m in marks {
                e.edit(start..start + len, m, UserEvent::InputTypeCompose);
                len = m.len();
            }
            e.end_bracket(b);
        }
        Op::Program(n, s) => {
            let at = boundary(e, *n);
            let len = e.buffer().len_bytes();
            let tr = Transaction {
                changes: ChangeSet::from_edits(len, [(at..at, Arc::from(s.as_str()))]).unwrap(),
                selection: None,
                ann: Annotations {
                    user_event: UserEvent::Command("program"),
                    add_to_history: false,
                    time: e.now,
                    origin: Origin::Program,
                },
            };
            e.dispatch(tr).unwrap();
        }
    }
}

fn caret_is_valid(e: &Ed) -> bool {
    let t = text(e);
    e.selection()
        .ranges()
        .iter()
        .all(|r| t.is_char_boundary(r.anchor) && t.is_char_boundary(r.head))
}

proptest! {
    #[test]
    fn undo_lands_on_texts_the_document_had_and_redo_comes_back(
        start in "[a-c ;]{0,6}",
        ops in prop::collection::vec(op_s(false), 0..40),
    ) {
        let mut e = model(&start);
        let mut seen = HashSet::from([text(&e)]);
        for op in &ops {
            run_op(&mut e, op);
            prop_assert!(caret_is_valid(&e), "after {:?}", op);
            seen.insert(text(&e));
        }
        let end = text(&e);
        let depth = e.undo_depth();
        let undone = undo_all(&mut e);
        prop_assert_eq!(undone.len(), depth);
        for t in &undone {
            prop_assert!(seen.contains(t), "undo made {:?}, which the document never had", t);
        }
        // As many redos bring the text back (more may follow, from undos in `ops`).
        for _ in 0..depth {
            prop_assert!(e.redo().is_some());
        }
        prop_assert_eq!(text(&e), end);
    }

    #[test]
    fn program_edits_keep_undo_and_redo_consistent(
        start in "[a-c ;]{0,6}",
        ops in prop::collection::vec(op_s(true), 0..40),
    ) {
        let mut e = model(&start);
        for op in &ops {
            run_op(&mut e, op);
            prop_assert!(caret_is_valid(&e), "after {:?}", op);
        }
        let end = text(&e);
        let depth = e.undo_depth();
        prop_assert!(depth <= 1_000);
        prop_assert_eq!(undo_all(&mut e).len(), depth);
        for _ in 0..depth {
            prop_assert!(e.redo().is_some());
        }
        prop_assert_eq!(text(&e), end);
    }
}
