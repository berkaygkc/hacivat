//! Task 1b's tests of crate-private seams (review round 2): every `Committed` reaches the same
//! post-commit fan-out, and nothing accessibility-related is evaluated without assistive
//! technology.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div,
};
use hacivat_text::{
    Annotations, ChangeDesc, ChangeSet, Origin, SelectionSet, Snapshot, TokenOracle, Transaction,
    UserEvent,
};
use smallvec::SmallVec;

use crate::config::{DocContext, DocId, EditorConfig};
use crate::service::{
    DecorationSource, ExecAction, ExecAt, ExecPreview, InvalidateFn, LanguageService, NoTokens,
    StatementSpan,
};
use crate::view::{EditorEvent, EditorView};

/// A service that records the calls the fan-out makes.
#[derive(Default)]
struct Recording {
    changes: RefCell<Vec<u64>>,
}

impl LanguageService for Recording {
    fn open(&self, _: DocId, _: &Snapshot, _: DocContext, _: &mut gpui::App) {}
    fn set_context(&self, _: DocId, _: DocContext, _: &mut gpui::App) {}
    fn change(&self, _: DocId, snap: &Snapshot, _: &ChangeDesc, _: &mut gpui::App) {
        self.changes.borrow_mut().push(snap.version);
    }
    fn close(&self, _: DocId) {}
    fn on_update(&self, _: DocId, _: InvalidateFn) {}
    fn statements(&self, _: DocId, _: u64, _: std::ops::Range<usize>) -> Arc<[StatementSpan]> {
        Arc::from(Vec::new())
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
}

struct Host {
    editor: Entity<EditorView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.editor.clone())
    }
}

struct Fixture<'a> {
    editor: Entity<EditorView>,
    service: Rc<Recording>,
    events: Rc<RefCell<Vec<String>>>,
    label_reads: Rc<Cell<u32>>,
    vcx: &'a mut VisualTestContext,
}

fn fixture<'a>(cx: &'a mut TestAppContext, text: &'static str) -> Fixture<'a> {
    let service = Rc::new(Recording::default());
    let label_reads = Rc::new(Cell::new(0));
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let (out, svc, reads) = (slot.clone(), service.clone(), label_reads.clone());
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(3),
                text,
                EditorConfig::default(),
                svc,
                Rc::new(move |_: &gpui::App| {
                    reads.set(reads.get() + 1);
                    "Console 1".into()
                }),
                window,
                cx,
            )
        });
        *out.borrow_mut() = Some(editor.clone());
        Host { editor }
    });
    let editor = slot.borrow().clone().unwrap();
    let events: Rc<RefCell<Vec<String>>> = Rc::default();
    let seen = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&editor, move |_, ev: &EditorEvent, _| {
            // The kind of each event, without its version.
            let name = format!("{ev:?}");
            seen.borrow_mut().push(
                name.split([' ', '('])
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            );
        })
        .detach();
    });
    Fixture {
        editor,
        service,
        events,
        label_reads,
        vcx,
    }
}

fn insert(len: usize, at: usize, text: &str, event: UserEvent) -> Transaction {
    Transaction {
        changes: ChangeSet::from_edits(len, [(at..at, Arc::from(text))]).unwrap(),
        selection: Some(SelectionSet::cursor(at + text.len())),
        ann: Annotations {
            user_event: event,
            add_to_history: true,
            time: Instant::now(),
            origin: Origin::User,
        },
    }
}

/// An undo (or an IME mark) produces its `Committed` in the model, without a `Transaction` in
/// the view; through `apply_committed` it reaches exactly the fan-out a typed edit does.
#[gpui::test]
fn every_committed_reaches_the_same_fan_out(cx: &mut TestAppContext) {
    let f = fixture(cx, "SELECT 1");
    // A typed edit, through `commit`.
    f.editor.update(f.vcx, |v, cx| {
        v.commit(insert(8, 8, ";", UserEvent::InputType), cx);
    });
    let typed = std::mem::take(&mut *f.events.borrow_mut());
    // What Task 25's undo does: the model makes the `Committed`, the view fans it out.
    f.editor.update(f.vcx, |v, cx| {
        let len = v.model.snapshot().buffer.len_bytes();
        let committed = v
            .model
            .dispatch(insert(len, 0, "-- ", UserEvent::Undo))
            .expect("the change applies");
        v.apply_committed(committed, UserEvent::Undo, cx);
    });
    let undone = std::mem::take(&mut *f.events.borrow_mut());
    assert_eq!(typed, ["Changed", "StatusChanged"]);
    assert_eq!(undone, typed, "the same events");
    assert_eq!(
        *f.service.changes.borrow(),
        [1, 2],
        "the service heard both"
    );
    let text = f
        .vcx
        .read(|cx| f.editor.read(cx).snapshot().buffer.to_string());
    assert_eq!(text, "-- SELECT 1;");
}

/// A `Committed` whose version did not move is selection-only (Task 5's model keeps the version
/// for a selection change): no service change, no `Changed`, whatever its other fields say.
/// The test repeats one `Committed` rather than building one (Task 5 adds fields).
#[gpui::test]
fn a_commit_at_the_same_version_is_selection_only(cx: &mut TestAppContext) {
    let f = fixture(cx, "SELECT 1");
    f.editor.update(f.vcx, |v, cx| {
        let committed = v
            .model
            .dispatch(insert(8, 8, ";", UserEvent::InputType))
            .expect("the change applies");
        v.apply_committed(committed.clone(), UserEvent::InputType, cx);
        v.apply_committed(committed, UserEvent::InputType, cx);
    });
    assert_eq!(
        *f.events.borrow(),
        [
            "Changed",
            "StatusChanged",
            "SelectionChanged",
            "StatusChanged"
        ]
    );
    assert_eq!(*f.service.changes.borrow(), [1]);
}

/// Without assistive technology the element builds no accessibility frame, so the label is
/// never evaluated by a frame (R1-M3); `announce` queues at most four texts.
#[gpui::test]
fn no_a11y_work_without_assistive_technology(cx: &mut TestAppContext) {
    let f = fixture(cx, "SELECT 1");
    for _ in 0..3 {
        f.vcx.update(|window, _| window.refresh());
        f.vcx.run_until_parked();
    }
    assert_eq!(f.label_reads.get(), 0);
    f.editor.update(f.vcx, |v, _| {
        for i in 0..6 {
            v.announce(format!("{i}").into());
        }
        let nodes = v.a11y.nodes();
        assert_eq!(
            nodes
                .pending
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>(),
            ["2", "3", "4", "5"]
        );
    });
    // When a frame is built, the label is read once per UI-text generation.
    f.editor.update(f.vcx, |v, cx| {
        let a = v.a11y_frame(false, cx);
        let b = v.a11y_frame(true, cx);
        assert_eq!((a.label.as_ref(), b.focused), ("Console 1", true));
    });
    assert_eq!(f.label_reads.get(), 1);
}
