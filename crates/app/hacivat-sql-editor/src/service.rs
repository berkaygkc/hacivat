//! `SqlLanguageService` (Task 24).

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_text::{ChangeDesc, Snapshot, TokenOracle};
use hacivat_ui_core::gpui_surface::App;
use hacivat_ui_editor::{
    DecorationSource, DocContext, DocId, ExecAction, ExecAt, ExecPreview, FailClosedService,
    InvalidateFn, LanguageService, StatementSpan,
};
use smallvec::SmallVec;
use sql_dialect::{ClientFlavor, DialectCtx};
use sql_split::DocSnapshot;

/// The SQL language service: one per app, every console's documents.
#[derive(Debug, Default)]
pub struct SqlLanguageService {
    /// What Task 1b answers with.
    fallback: FailClosedService,
}

impl SqlLanguageService {
    pub fn new() -> Rc<Self> {
        Rc::new(SqlLanguageService::default())
    }

    /// The dialect and client flavor `doc` is analysed with (the console's SQL context,
    /// R-M3-39). Task 1b: ignored.
    pub fn set_sql_context(&self, doc: DocId, ctx: DialectCtx, flavor: ClientFlavor) {
        let _ = (doc, ctx, flavor);
    }

    /// The analysis of `doc` at exactly `version`, if there is one (Run reuses it; R-M3-22).
    /// Task 1b: none.
    pub fn doc_snapshot(&self, doc: DocId, version: u64) -> Option<DocSnapshot> {
        let _ = (doc, version);
        None
    }
}

impl LanguageService for SqlLanguageService {
    fn open(&self, doc: DocId, snap: &Snapshot, ctx: DocContext, cx: &mut App) {
        self.fallback.open(doc, snap, ctx, cx);
    }

    fn set_context(&self, doc: DocId, ctx: DocContext, cx: &mut App) {
        self.fallback.set_context(doc, ctx, cx);
    }

    fn change(&self, doc: DocId, snap: &Snapshot, changes: &ChangeDesc, cx: &mut App) {
        self.fallback.change(doc, snap, changes, cx);
    }

    fn close(&self, doc: DocId) {
        self.fallback.close(doc);
    }

    fn on_update(&self, doc: DocId, notify: InvalidateFn) {
        self.fallback.on_update(doc, notify);
    }

    fn statements(&self, doc: DocId, version: u64, range: Range<usize>) -> Arc<[StatementSpan]> {
        self.fallback.statements(doc, version, range)
    }

    fn exec_preview(
        &self,
        doc: DocId,
        version: u64,
        at: &ExecAt,
        action: ExecAction,
    ) -> Option<ExecPreview> {
        self.fallback.exec_preview(doc, version, at, action)
    }

    fn token_oracle(&self, doc: DocId) -> Rc<dyn TokenOracle> {
        self.fallback.token_oracle(doc)
    }

    fn decoration_sources(&self, doc: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
        self.fallback.decoration_sources(doc)
    }
}
