//! Shared helpers for the SQL language service tests (Task 24): a document driven through the
//! `LanguageService` seam by a real `EditorModel`, as the editor view drives it.
#![allow(dead_code)]

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::TestAppContext;
use hacivat_sql_editor::SqlLanguageService;
use hacivat_text::display::Decoration;
use hacivat_text::{
    Annotations, ChangeSet, EditorModel, ModelConfig, Origin, Snapshot, Transaction, UserEvent,
};
use hacivat_ui_editor::{
    DocContext, DocId, ExecAction, ExecAt, ExecPreview, HighlightClass, LanguageService, Tier,
};
use smallvec::smallvec;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, ServerVersion};

pub const DOC: DocId = DocId(7);

pub fn pg() -> (DialectCtx, ClientFlavor) {
    (
        DialectCtx::new(DialectId::Postgres, None),
        ClientFlavor::Psql,
    )
}

pub fn mysql() -> (DialectCtx, ClientFlavor) {
    (
        DialectCtx::new(DialectId::MySql, None),
        ClientFlavor::MySqlCli,
    )
}

/// MySQL at a known server version.
pub fn mysql_at(major: u16, minor: u16, patch: u16) -> (DialectCtx, ClientFlavor) {
    let v = ServerVersion {
        major,
        minor,
        patch,
    };
    (
        DialectCtx::new(DialectId::MySql, Some(v)),
        ClientFlavor::MySqlCli,
    )
}

pub fn sqlite() -> (DialectCtx, ClientFlavor) {
    (
        DialectCtx::new(DialectId::Sqlite, None),
        ClientFlavor::Sqlite3,
    )
}

pub fn connected() -> DocContext {
    DocContext {
        connected: true,
        tier: Tier::Full,
    }
}

pub fn caret(at: usize) -> ExecAt {
    ExecAt::Carets(smallvec![at])
}

pub fn selection(r: Range<usize>) -> ExecAt {
    ExecAt::Selections(smallvec![r])
}

/// One document open in a service, edited through an `EditorModel`.
pub struct Doc {
    pub model: EditorModel,
    pub svc: Rc<SqlLanguageService>,
}

impl Doc {
    /// Opens `text` (connected) with the console's SQL context `sql` when given. Nothing is
    /// analysed until the executor runs.
    pub fn open(
        cx: &mut TestAppContext,
        text: &str,
        sql: Option<(DialectCtx, ClientFlavor)>,
    ) -> Doc {
        let svc = SqlLanguageService::new();
        if let Some((ctx, flavor)) = sql {
            svc.set_sql_context(DOC, ctx, flavor);
        }
        let model = EditorModel::new(text, ModelConfig::default());
        let snap = model.snapshot();
        cx.update(|cx| svc.open(DOC, &snap, connected(), cx));
        Doc { model, svc }
    }

    /// Opens `text` and lets the first analysis land.
    pub fn analysed(cx: &mut TestAppContext, text: &str, sql: (DialectCtx, ClientFlavor)) -> Doc {
        let d = Doc::open(cx, text, Some(sql));
        cx.run_until_parked();
        d
    }

    /// Replaces `r` with `text` and tells the service, as the view's `apply_committed` does.
    pub fn edit(&mut self, cx: &mut TestAppContext, r: Range<usize>, text: &str) {
        let tr = Transaction {
            changes: ChangeSet::from_edits(self.model.buffer().len_bytes(), [(r, Arc::from(text))])
                .expect("the edit fits the text"),
            selection: None,
            ann: Annotations {
                user_event: UserEvent::InputType,
                add_to_history: false,
                time: Instant::now(),
                origin: Origin::User,
            },
        };
        let committed = self.model.dispatch(tr).expect("the edit applies");
        let snap = self.model.snapshot();
        cx.update(|cx| self.svc.change(DOC, &snap, &committed.desc, cx));
    }

    pub fn text(&self) -> String {
        self.model.buffer().to_string_with_bom()
    }

    pub fn snap(&self) -> Snapshot {
        self.model.snapshot()
    }

    pub fn version(&self) -> u64 {
        self.model.snapshot().version
    }

    /// The highlight marks over `range` at the current version, in order.
    pub fn marks(&self, range: Range<usize>) -> Vec<(Range<usize>, HighlightClass)> {
        let sources = self.svc.decoration_sources(DOC);
        let snap = self.snap();
        let mut out = Vec::new();
        for s in &sources {
            let set = s.decorations(&snap, self.model.changelog(), range.clone());
            for (r, d) in set.query(range.clone()) {
                if let Decoration::Mark { style } = d
                    && let Some(class) = HighlightClass::from_style(*style)
                {
                    out.push((r, class));
                }
            }
        }
        out
    }

    /// Every mark of the document.
    pub fn all_marks(&self) -> Vec<(Range<usize>, HighlightClass)> {
        self.marks(0..self.model.buffer().len_bytes())
    }

    /// The classes of the marks that cover exactly the `n`th occurrence of `piece`.
    pub fn classes_of(&self, piece: &str, n: usize) -> Vec<HighlightClass> {
        let text = self.text();
        let at = text
            .match_indices(piece)
            .nth(n)
            .unwrap_or_else(|| panic!("{piece:?} occurs {n} times in {text:?}"))
            .0;
        let r = at..at + piece.len();
        self.all_marks()
            .into_iter()
            .filter(|(m, _)| *m == r)
            .map(|(_, c)| c)
            .collect()
    }

    /// The words the token oracle gives for line `line` (0-based), as text.
    pub fn words(&self, line: usize) -> Option<Vec<String>> {
        let buf = self.model.buffer();
        let body = buf.line_range(line, false);
        let words = self.svc.token_oracle(DOC).words_on_line(body)?;
        Some(
            words
                .into_iter()
                .map(|w| buf.slice(w).into_owned())
                .collect(),
        )
    }

    pub fn preview(&self, at: &ExecAt, action: ExecAction) -> Option<ExecPreview> {
        self.svc.exec_preview(DOC, self.version(), at, action)
    }

    /// The texts of a preview's ranges.
    pub fn texts(&self, p: &ExecPreview) -> Vec<String> {
        let text = self.text();
        p.ranges
            .iter()
            .map(|r| text[r.clone()].to_owned())
            .collect()
    }
}

/// A small deterministic generator (xorshift64*), so the edit sequences are reproducible.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(2_685_821_657_736_338_717)
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}
