//! UI-EDIT-01 model (M3a Task 5): dispatch runs filters → buffer apply → selection (given or
//! mapped) → fields in order → version += 1 → observers → dirty range; a dispatch from an
//! observer is queued, never re-entrant; snapshots are O(1) and readable on another thread.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_text::model::{ObserverQueue, TransactionObserver};
use hacivat_text::{
    Annotations, Bias, ChangeSet, Committed, EditorModel, Field, ModelConfig, Origin,
    SelectionRange, SelectionSet, Snapshot, TextBuffer, Transaction, TransactionFilter, UserEvent,
    Veto,
};

fn ann(user_event: UserEvent) -> Annotations {
    Annotations {
        user_event,
        add_to_history: true,
        time: Instant::now(),
        origin: Origin::User,
    }
}

fn insert_tr(buf: &TextBuffer, at: usize, text: &str) -> Transaction {
    Transaction {
        changes: ChangeSet::from_edits(buf.len_bytes(), [(at..at, Arc::from(text))]).unwrap(),
        selection: None,
        ann: ann(UserEvent::InputType),
    }
}

fn replace_tr(m: &EditorModel, r: Range<usize>, text: &str) -> Transaction {
    Transaction {
        changes: ChangeSet::from_edits(m.buffer().len_bytes(), [(r, Arc::from(text))]).unwrap(),
        selection: None,
        ann: ann(UserEvent::InputType),
    }
}

fn select_tr(m: &EditorModel, sel: SelectionSet) -> Transaction {
    Transaction {
        changes: ChangeSet::identity(m.buffer().len_bytes()),
        selection: Some(sel),
        ann: ann(UserEvent::Select),
    }
}

fn text(m: &EditorModel) -> String {
    m.buffer().to_string()
}

/// Refuses any change that touches `[0, until)`.
struct ReadOnlyPrefix(usize);

impl TransactionFilter for ReadOnlyPrefix {
    fn filter(&self, _m: &EditorModel, tr: Transaction) -> Result<Transaction, Veto> {
        let mut at = 0;
        for s in tr.changes.sections() {
            match s {
                hacivat_text::Section::Retain(n) => at += n,
                hacivat_text::Section::Replace { del, .. } => {
                    if at < self.0 {
                        return Err(Veto("editor-readonly-region"));
                    }
                    at += del;
                }
            }
        }
        Ok(tr)
    }
}

#[test]
fn ui_edit_01_cases_1_to_5() {
    // 1. Type `a` into an empty document.
    let mut m = EditorModel::new("", ModelConfig::default());
    let c = m.dispatch(insert_tr(m.buffer(), 0, "a")).unwrap();
    assert_eq!(c.version, 1);
    assert!(c.buffer_changed);
    assert_eq!(text(&m), "a");
    assert_eq!(m.selection().main().head, 1);
    assert_eq!(m.undo_depth(), 1);

    // 2. A read-only range [0, 10) refuses deleting [5, 6): nothing changes.
    let mut m = EditorModel::new("SELECT 1; SELECT 2;", ModelConfig::default());
    m.add_filter(Box::new(ReadOnlyPrefix(10)));
    let before = m.snapshot();
    let refused = m.dispatch(replace_tr(&m, 5..6, ""));
    assert_eq!(refused, Err(Veto("editor-readonly-region")));
    assert_eq!(text(&m), "SELECT 1; SELECT 2;");
    assert_eq!(m.version(), before.version);
    assert_eq!(m.undo_depth(), 0);
    // ... but past the range it edits, and selections are still allowed.
    m.dispatch(replace_tr(&m, 17..18, "3")).unwrap();
    assert_eq!(text(&m), "SELECT 1; SELECT 3;");
    m.dispatch(select_tr(&m, SelectionSet::cursor(2))).unwrap();
    assert_eq!(m.selection().main().head, 2);

    // 3. An observer's dispatch is queued and applied after the first; two versions.
    let mut m = EditorModel::new("x", ModelConfig::default());
    let seen = Rc::new(RefCell::new(Vec::new()));
    m.add_observer(Box::new(AppendOnce {
        text: "!",
        done: false,
        seen: seen.clone(),
    }));
    let c = m.dispatch(insert_tr(m.buffer(), 0, ">")).unwrap();
    assert_eq!(text(&m), ">x!");
    assert_eq!(m.version(), 2);
    assert_eq!(c.version, 2);
    // The observer saw the first commit before its own transaction applied.
    assert_eq!(seen.borrow()[0], (1, ">x".to_owned()));
    // The commit describes both: from ">"-less "x" to ">x!".
    assert_eq!(c.desc.len_old(), 1);
    assert_eq!(c.desc.len_new(), 3);
    assert_eq!(c.dirty, Some(0..3));

    // 4. A snapshot taken before an edit is unchanged when read on another thread after it.
    let mut m = EditorModel::new("SELECT 1", ModelConfig::default());
    let snap = m.snapshot();
    m.dispatch(replace_tr(&m, 7..8, "2")).unwrap();
    let read = std::thread::spawn(move || (snap.buffer.to_string(), snap.version))
        .join()
        .unwrap();
    assert_eq!(read, ("SELECT 1".to_owned(), 0));
    assert_eq!(text(&m), "SELECT 2");

    // 5. A selection-only transaction leaves the text version alone.
    let v = m.version();
    let c = m.dispatch(select_tr(&m, SelectionSet::cursor(3))).unwrap();
    assert!(!c.buffer_changed);
    assert_eq!(c.version, v);
    assert_eq!(c.dirty, None);
    assert_eq!(m.version(), v);
    assert_eq!(m.changelog().version(), v);
    assert_eq!(m.snapshot().selection.main().head, 3);
}

/// Appends `text` once, from inside the observer, and records what it saw.
struct AppendOnce {
    text: &'static str,
    done: bool,
    seen: Rc<RefCell<Vec<(u64, String)>>>,
}

impl TransactionObserver for AppendOnce {
    fn observe(
        &mut self,
        _tr: &Transaction,
        c: &Committed,
        buffer: &TextBuffer,
        queue: &mut ObserverQueue,
    ) {
        self.seen.borrow_mut().push((c.version, buffer.to_string()));
        if !self.done {
            self.done = true;
            let len = buffer.len_bytes();
            queue.dispatch(insert_tr(buffer, len, self.text));
        }
    }
}

#[test]
fn snapshots_are_send_and_sync() {
    fn check<T: Send + Sync + Clone>() {}
    check::<Snapshot>();
    check::<TextBuffer>();
}

#[test]
fn two_observers_queue_against_the_same_version_and_both_apply() {
    let mut m = EditorModel::new("mid", ModelConfig::default());
    let seen = Rc::new(RefCell::new(Vec::new()));
    // Each queues an edit built against the version it saw; the second is rebased over the
    // first.
    m.add_observer(Box::new(AppendOnce {
        text: "]",
        done: false,
        seen: seen.clone(),
    }));
    m.add_observer(Box::new(PrependOnce { done: false }));
    let c = m.dispatch(insert_tr(m.buffer(), 3, "!")).unwrap();
    assert_eq!(text(&m), "[mid!]");
    assert_eq!(c.version, 3);
    assert_eq!(m.version(), 3);
}

struct PrependOnce {
    done: bool,
}

impl TransactionObserver for PrependOnce {
    fn observe(
        &mut self,
        _tr: &Transaction,
        _c: &Committed,
        buffer: &TextBuffer,
        queue: &mut ObserverQueue,
    ) {
        if !self.done {
            self.done = true;
            queue.dispatch(insert_tr(buffer, 0, "["));
        }
    }
}

/// Queues another edit after every commit, forever.
struct Runaway;

impl TransactionObserver for Runaway {
    fn observe(&mut self, _: &Transaction, _: &Committed, b: &TextBuffer, q: &mut ObserverQueue) {
        q.dispatch(insert_tr(b, b.len_bytes(), "."));
    }
}

#[test]
fn a_runaway_observer_is_bounded() {
    let mut m = EditorModel::new("", ModelConfig::default());
    m.add_observer(Box::new(Runaway));
    let c = m.dispatch(insert_tr(m.buffer(), 0, ">")).unwrap();
    let dots = text(&m).len() - 1;
    assert!(dots > 0, "queued transactions apply");
    assert!(dots <= 256, "the queue is bounded per dispatch: {dots}");
    assert_eq!(c.version, m.version());
    // The next dispatch starts from an empty queue.
    let before = m.version();
    m.dispatch(insert_tr(m.buffer(), 0, ">")).unwrap();
    assert!(m.version() > before);
}

/// Rewrites typed tabs to four spaces.
struct TabsToSpaces;

impl TransactionFilter for TabsToSpaces {
    fn filter(&self, m: &EditorModel, tr: Transaction) -> Result<Transaction, Veto> {
        let mut at = 0;
        let mut edits = Vec::new();
        for s in tr.changes.sections() {
            match s {
                hacivat_text::Section::Retain(n) => at += n,
                hacivat_text::Section::Replace { del, ins } => {
                    edits.push((at..at + del, Arc::from(ins.replace('\t', "    ").as_str())));
                    at += del;
                }
            }
        }
        let changes = ChangeSet::from_edits(m.buffer().len_bytes(), edits)
            .map_err(|_| Veto("bad rewrite"))?;
        Ok(Transaction { changes, ..tr })
    }
}

/// Records what it sees, to check the filter order.
struct Spy(Rc<RefCell<Vec<String>>>, &'static str);

impl TransactionFilter for Spy {
    fn filter(&self, _m: &EditorModel, tr: Transaction) -> Result<Transaction, Veto> {
        let ins: String = tr
            .changes
            .sections()
            .iter()
            .filter_map(|s| match s {
                hacivat_text::Section::Replace { ins, .. } => Some(ins.to_string()),
                hacivat_text::Section::Retain(_) => None,
            })
            .collect();
        self.0.borrow_mut().push(format!("{}:{ins}", self.1));
        Ok(tr)
    }
}

impl Field for Spy {
    fn on_commit(&mut self, _tr: &Transaction, old: &TextBuffer, new: &TextBuffer, version: u64) {
        self.0.borrow_mut().push(format!(
            "{}:{}->{}@{version}",
            self.1,
            old.len_bytes(),
            new.len_bytes()
        ));
    }
}

struct ObserverSpy(Rc<RefCell<Vec<String>>>);

impl TransactionObserver for ObserverSpy {
    fn observe(&mut self, _: &Transaction, c: &Committed, _: &TextBuffer, _: &mut ObserverQueue) {
        self.0.borrow_mut().push(format!("observer@{}", c.version));
    }
}

#[test]
fn filters_rewrite_in_order_then_fields_then_observers() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut m = EditorModel::new("ab", ModelConfig::default());
    m.add_filter(Box::new(Spy(log.clone(), "f1")));
    m.add_filter(Box::new(TabsToSpaces));
    m.add_filter(Box::new(Spy(log.clone(), "f2")));
    m.add_field(Box::new(Spy(log.clone(), "fieldA")));
    m.add_field(Box::new(Spy(log.clone(), "fieldB")));
    m.add_observer(Box::new(ObserverSpy(log.clone())));
    m.dispatch(insert_tr(m.buffer(), 1, "\t")).unwrap();
    assert_eq!(text(&m), "a    b");
    assert_eq!(
        *log.borrow(),
        [
            "f1:\t",
            "f2:    ",
            "fieldA:2->6@1",
            "fieldB:2->6@1",
            "observer@1"
        ]
    );
    // Fields follow selection-only commits too, at the unchanged version.
    log.borrow_mut().clear();
    m.dispatch(select_tr(&m, SelectionSet::cursor(0))).unwrap();
    assert_eq!(
        *log.borrow(),
        ["f1:", "f2:", "fieldA:6->6@1", "fieldB:6->6@1", "observer@1"]
    );
}

#[test]
fn edits_snap_to_character_boundaries() {
    // "çx": `ç` is bytes 0..2.
    let mut m = EditorModel::new("çx", ModelConfig::default());
    // A deletion that starts inside `ç` widens to all of it.
    let c = m.dispatch(replace_tr(&m, 1..2, "")).unwrap();
    assert_eq!(text(&m), "x");
    assert_eq!((c.desc.len_old(), c.desc.len_new()), (3, 1));
    // An insertion inside a character goes before it, deleting nothing.
    let mut m = EditorModel::new("çx", ModelConfig::default());
    m.dispatch(replace_tr(&m, 1..1, "y")).unwrap();
    assert_eq!(text(&m), "yçx");
    // Inside a four-byte character, at both ends.
    let mut m = EditorModel::new("a😀b", ModelConfig::default());
    m.dispatch(replace_tr(&m, 2..4, "-")).unwrap();
    assert_eq!(text(&m), "a-b");
    // Two edits inside one character overlap once snapped: refused, and nothing changes.
    let mut m = EditorModel::new("😀", ModelConfig::default());
    let both = Transaction {
        changes: ChangeSet::from_edits(4, [(0..1, Arc::from("x")), (2..3, Arc::from("y"))])
            .unwrap(),
        selection: None,
        ann: ann(UserEvent::InputType),
    };
    assert!(m.dispatch(both).is_err());
    assert_eq!(text(&m), "😀");
    assert_eq!(m.version(), 0);
}

#[test]
fn a_change_set_for_another_length_is_vetoed() {
    let mut m = EditorModel::new("abc", ModelConfig::default());
    let stale = Transaction {
        changes: ChangeSet::from_edits(2, [(0..0, Arc::from("x"))]).unwrap(),
        selection: None,
        ann: ann(UserEvent::InputType),
    };
    assert!(m.dispatch(stale).is_err());
    assert_eq!(text(&m), "abc");
    assert_eq!(m.version(), 0);
}

#[test]
fn a_given_selection_is_clamped_to_the_text_and_snapped() {
    let mut m = EditorModel::new("aç", ModelConfig::default());
    let sel = SelectionSet::single(SelectionRange {
        anchor: 2,
        head: 99,
        goal_x: None,
        assoc: Bias::After,
    });
    m.dispatch(select_tr(&m, sel)).unwrap();
    assert_eq!(m.selection().main().range(), 1..3);
    // A mapped selection keeps to its text: a caret after the edit point moves with it.
    m.dispatch(select_tr(&m, SelectionSet::cursor(3))).unwrap();
    m.dispatch(insert_tr(m.buffer(), 0, "--")).unwrap();
    assert_eq!(m.selection().main().head, 5);
}

#[test]
fn undo_and_redo_go_through_the_filters() {
    let locked = Rc::new(Cell::new(false));
    struct Lock(Rc<Cell<bool>>);
    impl TransactionFilter for Lock {
        fn filter(&self, _m: &EditorModel, tr: Transaction) -> Result<Transaction, Veto> {
            if self.0.get() && !tr.changes.is_identity() {
                Err(Veto("editor-read-only"))
            } else {
                Ok(tr)
            }
        }
    }
    let mut m = EditorModel::new("", ModelConfig::default());
    m.add_filter(Box::new(Lock(locked.clone())));
    m.dispatch(insert_tr(m.buffer(), 0, "a")).unwrap();
    locked.set(true);
    assert!(m.undo().is_none());
    assert_eq!(text(&m), "a");
    assert_eq!(m.undo_depth(), 1);
    locked.set(false);
    let c = m.undo().unwrap();
    assert_eq!(text(&m), "");
    assert_eq!(c.version, 2);
    locked.set(true);
    assert!(m.redo().is_none());
    assert_eq!(m.redo_depth(), 1);
    locked.set(false);
    assert!(m.redo().is_some());
    assert_eq!(text(&m), "a");
}

#[test]
#[ignore = "perf"]
fn typing_into_20_mb_is_not_o_document() {
    if std::env::var_os("HACIVAT_PERF").is_none_or(|v| v != "1") {
        return;
    }
    let line = "SELECT id, name FROM customers WHERE id = 1;\r\n";
    let mut m = EditorModel::new(
        &line.repeat((20 << 20) / line.len()),
        ModelConfig::default(),
    );
    let mid = m.buffer().len_bytes() / 2;
    m.dispatch(select_tr(&m, SelectionSet::cursor(mid)))
        .unwrap();
    let start = Instant::now();
    for i in 0..1_000 {
        let at = m.selection().main().head;
        let tr = Transaction {
            changes: ChangeSet::from_edits(m.buffer().len_bytes(), [(at..at, Arc::from("x"))])
                .unwrap(),
            selection: Some(SelectionSet::cursor(at + 1)),
            ann: Annotations {
                time: Instant::now() + Duration::from_millis(i),
                ..ann(UserEvent::InputType)
            },
        };
        m.dispatch(tr).unwrap();
        let _ = m.snapshot();
    }
    let each = start.elapsed() / 1_000;
    assert!(
        each < Duration::from_micros(200),
        "a keystroke took {each:?}"
    );
}
