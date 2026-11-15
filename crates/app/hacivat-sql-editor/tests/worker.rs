//! The per-document worker (R-M3-22; Task 24): `change()` only hands the snapshot over, the
//! newest snapshot alone is analysed on the background executor, the result equals a fresh
//! document, and Run's `doc_snapshot` answers only at the exact version under the console's
//! own SQL context.

mod support;

use std::rc::Rc;

use gpui::{TestAppContext, VisualTestContext};
use hacivat_sql_editor::{RopeChunks, SqlLanguageService, effective_flavor};
use hacivat_text::TextBuffer;
use hacivat_ui_editor::{
    DocId, EditorConfig, EditorView, ExecAction, LanguageService, PreviewState,
};
use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_split::SqlDocument;
use sql_text::TextChunks;
use support::{DOC, Doc, Rng, caret, mysql, pg, sqlite};

/// Pieces the random edits insert: statements, quotes, dollar quotes, comments, client
/// commands, `COPY` data and non-ASCII text, so the edits cut through every construct.
const PIECES: &[&str] = &[
    "SELECT 1;",
    " ",
    "\n",
    ";",
    "'",
    "\"",
    "$$",
    "$f$",
    "--",
    "/*",
    "*/",
    "/*!50000 ",
    "\\set x 1\n",
    "COPY t FROM STDIN;\n",
    "\\.\n",
    "BEGIN ",
    "END;",
    "DELIMITER $$\n",
    "(",
    ")",
    "müşteri",
    "İ",
    "SELECT 'a;b' FROM t WHERE x = $1;\n",
];

fn random_edit(d: &mut Doc, cx: &mut TestAppContext, rng: &mut Rng) {
    let text = d.text();
    let snap = |mut at: usize| {
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let a = snap(rng.below(text.len() + 1));
    let b = if rng.below(3) == 0 {
        snap((a + rng.below(24)).min(text.len()))
    } else {
        a
    };
    // Keep the text from growing without bound: delete more once it is long.
    let insert = if text.len() > 4_000 && rng.below(2) == 0 {
        ""
    } else {
        PIECES[rng.below(PIECES.len())]
    };
    let (a, b) = if text.len() > 4_000 && insert.is_empty() {
        (a, snap((a + 200).min(text.len())))
    } else {
        (a, b)
    };
    d.edit(cx, a..b, insert);
}

fn assert_equals_fresh(d: &Doc, ctx: DialectCtx, flavor: ClientFlavor) {
    let text = d.text();
    let got = d
        .svc
        .doc_snapshot(DOC, d.version())
        .expect("an analysis at the editor's version");
    let fresh = SqlDocument::new(text.clone(), ctx, flavor).snapshot();
    assert_eq!(got.text(), text);
    assert_eq!(got.lexed(), fresh.lexed(), "lexing of {text:?}");
    assert_eq!(
        got.editor_split(),
        fresh.editor_split(),
        "editor split of {text:?}"
    );
    assert_eq!(
        got.execution_split(),
        fresh.execution_split(),
        "execution split of {text:?}"
    );
    assert_eq!(*got.ctx(), ctx);
    assert_eq!(got.flavor(), flavor);
}

/// 10⁴ random edits through the service, the executor run at random points: whenever it has
/// run, the analysis at the editor's version equals `SqlDocument::new` of the editor's text.
#[gpui::test]
fn worker_result_equals_a_fresh_document_after_random_edits(cx: &mut TestAppContext) {
    for (seed, (ctx, flavor), edits) in [
        (1u64, pg(), 10_000),
        (2, mysql(), 2_000),
        (3, sqlite(), 2_000),
    ] {
        let mut d = Doc::open(cx, "SELECT 1;\n", Some((ctx, flavor)));
        let mut rng = Rng(seed.wrapping_mul(6_364_136_223_846_793_005) | 1);
        let mut checks = 0;
        for _ in 0..edits {
            random_edit(&mut d, cx, &mut rng);
            if rng.below(40) == 0 {
                cx.run_until_parked();
                assert_equals_fresh(&d, ctx, flavor);
                checks += 1;
            }
        }
        cx.run_until_parked();
        assert_equals_fresh(&d, ctx, flavor);
        assert!(checks > edits / 80, "checked {checks} times");
    }
}

/// R-M3-22: `open` and `change` never analyse on the calling thread; only the newest snapshot
/// is analysed once the executor runs.
#[gpui::test]
fn change_never_lexes_on_the_calling_thread(cx: &mut TestAppContext) {
    let mut d = Doc::open(cx, "SELECT 1;", Some(pg()));
    assert_eq!(d.svc.analyses(), 0, "open analysed on the calling thread");
    cx.run_until_parked();
    assert_eq!(d.svc.analyses(), 1);
    for i in 0..50 {
        d.edit(cx, i..i, "x");
        assert_eq!(d.svc.analyses(), 1, "change analysed on the calling thread");
    }
    assert!(d.svc.doc_snapshot(DOC, d.version()).is_none());
    cx.run_until_parked();
    assert_eq!(d.svc.analyses(), 2, "only the newest snapshot is analysed");
    assert!(d.svc.doc_snapshot(DOC, d.version()).is_some());
}

/// Run reuses the analysis only at the exact version and only under the console's own SQL
/// context: never one made under another dialect, or before the context was set.
#[gpui::test]
fn doc_snapshot_is_exact_and_under_the_consoles_context(cx: &mut TestAppContext) {
    let mut d = Doc::open(cx, "SELECT `a;b`;", None);
    cx.run_until_parked();
    assert!(
        d.svc.doc_snapshot(DOC, d.version()).is_none(),
        "no SQL context yet: Run analyses its own snapshot"
    );
    let (my, my_flavor) = mysql();
    cx.update(|cx| d.svc.set_sql_context(DOC, my, my_flavor, cx));
    assert!(d.svc.doc_snapshot(DOC, d.version()).is_none());
    cx.run_until_parked();
    let s = d.svc.doc_snapshot(DOC, d.version()).expect("analysed");
    assert_eq!(s.ctx().id, DialectId::MySql);
    assert_eq!(s.flavor(), ClientFlavor::MySqlCli);

    let old = d.version();
    d.edit(cx, 0..0, " ");
    assert!(d.svc.doc_snapshot(DOC, d.version()).is_none(), "stale");
    assert!(d.svc.doc_snapshot(DOC, old + 7).is_none());
    cx.run_until_parked();
    assert_eq!(
        d.svc.doc_snapshot(DOC, d.version()).unwrap().text(),
        " SELECT `a;b`;"
    );

    // A new context: the analysis under the old one is never served again.
    let (p, f) = pg();
    cx.update(|cx| d.svc.set_sql_context(DOC, p, f, cx));
    assert!(d.svc.doc_snapshot(DOC, d.version()).is_none());
    cx.run_until_parked();
    assert_eq!(
        d.svc.doc_snapshot(DOC, d.version()).unwrap().ctx().id,
        DialectId::Postgres
    );
}

/// The ledger's T24 rule: a PostgreSQL console never splits with `ClientFlavor::None` (with
/// it a pg_dump `COPY` row such as `DROP TABLE users` would become a certain statement).
#[gpui::test]
fn postgres_never_analyses_without_its_client_flavor(cx: &mut TestAppContext) {
    let pg_ctx = DialectCtx::new(DialectId::Postgres, None);
    assert_eq!(
        effective_flavor(&pg_ctx, ClientFlavor::None),
        ClientFlavor::Psql
    );
    let my = DialectCtx::new(DialectId::MySql, None);
    assert_eq!(
        effective_flavor(&my, ClientFlavor::None),
        ClientFlavor::MySqlCli
    );
    let lite = DialectCtx::new(DialectId::Sqlite, None);
    assert_eq!(
        effective_flavor(&lite, ClientFlavor::None),
        ClientFlavor::Sqlite3
    );
    assert_eq!(
        effective_flavor(&pg_ctx, ClientFlavor::Psql),
        ClientFlavor::Psql
    );
    // A client that is not the dialect's own never applies (review M3): PostgreSQL with the
    // mysql or sqlite3 rules would read pg_dump `COPY` rows as statements.
    for wrong in [ClientFlavor::MySqlCli, ClientFlavor::Sqlite3] {
        assert_eq!(effective_flavor(&pg_ctx, wrong), ClientFlavor::Psql);
    }
    let pgwire = DialectCtx::new(
        DialectId::PgWire(sql_dialect::PgWireFlavor::Cockroach),
        None,
    );
    assert_eq!(
        effective_flavor(&pgwire, ClientFlavor::Sqlite3),
        ClientFlavor::Psql
    );
    for wrong in [
        ClientFlavor::Psql,
        ClientFlavor::Sqlite3,
        ClientFlavor::None,
    ] {
        assert_eq!(effective_flavor(&my, wrong), ClientFlavor::MySqlCli);
    }
    let maria = DialectCtx::new(DialectId::MariaDb, None);
    assert_eq!(
        effective_flavor(&maria, ClientFlavor::Psql),
        ClientFlavor::MySqlCli
    );
    for wrong in [ClientFlavor::Psql, ClientFlavor::MySqlCli] {
        assert_eq!(effective_flavor(&lite, wrong), ClientFlavor::Sqlite3);
    }

    let text = "COPY users FROM STDIN;\nDROP TABLE users\n\\.\n";
    let d = Doc::open(cx, text, Some((pg_ctx, ClientFlavor::None)));
    cx.run_until_parked();
    let s = d.svc.doc_snapshot(DOC, d.version()).expect("analysed");
    assert_eq!(s.flavor(), ClientFlavor::Psql);
    let row = text.find("DROP").unwrap() + 2;
    let p = d
        .preview(&caret(row), ExecAction::Execute1)
        .expect("a preview");
    assert!(
        !matches!(p.state, PreviewState::Units | PreviewState::AskPreselected),
        "a COPY row never previews as runnable: {p:?}"
    );
}

/// Closing a document stops its worker: later calls for it are ignored, nothing is analysed.
#[gpui::test]
fn close_stops_the_worker(cx: &mut TestAppContext) {
    let mut d = Doc::analysed(cx, "SELECT 1;", pg());
    d.svc.close(DOC);
    let n = d.svc.analyses();
    d.edit(cx, 0..0, " ");
    cx.run_until_parked();
    assert_eq!(d.svc.analyses(), n);
    assert!(d.svc.doc_snapshot(DOC, d.version()).is_none());
    assert!(d.svc.decoration_sources(DOC).is_empty());
}

/// A closed document leaves nothing behind: the setters and `on_update` that arrive after
/// `close` re-create no entry (review M6).
#[gpui::test]
fn closed_documents_leave_no_entry(cx: &mut TestAppContext) {
    let d = Doc::analysed(cx, "SELECT 1;", pg());
    assert_eq!(d.svc.open_documents(), 1);
    d.svc.close(DOC);
    assert_eq!(d.svc.open_documents(), 0);
    let (ctx, flavor) = mysql();
    cx.update(|cx| {
        d.svc.set_sql_context(DOC, ctx, flavor, cx);
        d.svc.set_remembered_kind(DOC, None, cx);
    });
    d.svc.on_update(DOC, Rc::new(|_: &mut gpui::App| {}));
    assert_eq!(d.svc.open_documents(), 0);
    assert!(d.svc.decoration_sources(DOC).is_empty());
    // Opening it again works as a first open.
    let snap = d.snap();
    cx.update(|cx| d.svc.open(DOC, &snap, support::connected(), cx));
    cx.run_until_parked();
    assert_eq!(d.svc.open_documents(), 1);
    assert!(
        d.svc.doc_snapshot(DOC, d.version()).is_none(),
        "the context set after close was ignored"
    );
    cx.update(|cx| d.svc.set_sql_context(DOC, ctx, flavor, cx));
    cx.run_until_parked();
    assert_eq!(
        d.svc.doc_snapshot(DOC, d.version()).unwrap().ctx().id,
        DialectId::MySql
    );
}

/// A panic inside an analysis is contained (review M1): nothing is published for that
/// version (no preview, no `doc_snapshot`), the worker lives on, and the next edit is analysed.
#[gpui::test]
fn a_worker_panic_publishes_nothing_and_the_worker_lives_on(cx: &mut TestAppContext) {
    let mut d = Doc::analysed(cx, "SELECT 1;", pg());
    d.svc.inject_worker_panic();
    d.edit(cx, 0..0, " ");
    cx.run_until_parked();
    assert_eq!(d.svc.worker_panics(), 1);
    assert!(d.svc.doc_snapshot(DOC, d.version()).is_none());
    assert!(d.preview(&caret(3), ExecAction::Execute1).is_none());
    // Highlighting keeps the previous analysis, rebased.
    assert!(!d.all_marks().is_empty());
    d.edit(cx, 0..0, " ");
    cx.run_until_parked();
    assert_eq!(
        d.svc.doc_snapshot(DOC, d.version()).unwrap().text(),
        "  SELECT 1;"
    );
}

/// Two documents have their own workers and answers.
#[gpui::test]
fn documents_are_analysed_apart(cx: &mut TestAppContext) {
    let d = Doc::analysed(cx, "SELECT 1;", pg());
    let other = DocId(8);
    let buf = hacivat_text::EditorModel::new("SELECT 2;", Default::default());
    let (ctx, flavor) = mysql();
    cx.update(|cx| d.svc.set_sql_context(other, ctx, flavor, cx));
    cx.update(|cx| d.svc.open(other, &buf.snapshot(), support::connected(), cx));
    cx.run_until_parked();
    assert_eq!(
        d.svc.doc_snapshot(DOC, d.version()).unwrap().text(),
        "SELECT 1;"
    );
    let s = d.svc.doc_snapshot(other, buf.snapshot().version).unwrap();
    assert_eq!((s.text(), s.ctx().id), ("SELECT 2;", DialectId::MySql));
}

/// The rope is read chunk by chunk as the lexer's input, without copying it.
#[test]
fn rope_chunks_read_the_buffer() {
    let mut text = String::new();
    for i in 0..4_000 {
        text.push_str(&format!("SELECT 'müşteri {i}';\n"));
    }
    let buf = TextBuffer::from_str(&text);
    let chunks = RopeChunks(buf);
    assert_eq!(chunks.len(), text.len());
    assert!(
        chunks.chunks().count() > 1,
        "a long rope has several chunks"
    );
    assert_eq!(chunks.chunks().collect::<String>(), text);
    let r = text.find("müşteri 3999").unwrap()..text.len();
    assert_eq!(chunks.slice(r.clone()), &text[r]);
    let rope = ropey::Rope::from_str(&text);
    assert_eq!(chunks.len(), rope.len_bytes());
    assert!(TextChunks::is_empty(&RopeChunks(TextBuffer::from_str(""))));
}

/// Through the real editor view: an edit hands the snapshot over, the analysis lands on the
/// foreground after the executor runs, and its repaint callback updates the view (outside the
/// view's own update: GPUI would refuse a re-entrant one).
#[gpui::test]
fn an_editor_view_gets_its_analysis_off_thread(cx: &mut TestAppContext) {
    let svc = SqlLanguageService::new();
    let (p, f) = pg();
    cx.update(|cx| svc.set_sql_context(DocId(3), p, f, cx));
    let service: Rc<dyn LanguageService> = svc.clone();
    let (editor, vcx) = cx.add_window_view(move |window, cx| {
        EditorView::new(
            DocId(3),
            "",
            EditorConfig::default(),
            service,
            Rc::new(|_: &gpui::App| "Console 1".into()),
            window,
            cx,
        )
    });
    let vcx: &mut VisualTestContext = vcx;
    vcx.run_until_parked();
    let before = svc.analyses();
    editor.update(vcx, |v, cx| v.set_text("SELECT 1;\nSELECT 2;", cx));
    assert_eq!(svc.analyses(), before, "nothing analysed inside the edit");
    vcx.run_until_parked();
    let version = editor.read_with(vcx, |v, _| v.snapshot().version);
    let s = svc
        .doc_snapshot(DocId(3), version)
        .expect("the analysis landed");
    assert_eq!(s.editor_split().statements().count(), 2);
    assert!(svc.analyses() > before);
}
