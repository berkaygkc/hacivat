// ## m3b-t01c-core-seams
//! "Count rows" (R-M3-76, P-M3b-27; Task 16): `SELECT COUNT(*) FROM (\n<q>\n) hv_c`, with a
//! trailing `;` and a top-level `ORDER BY` without `LIMIT`/`FETCH` stripped by lexer tokens and
//! the trailing trivia trimmed; classified again and refused unless `Read`.
//!
//! ## m3b-t16-run-service (ADR 0006 `m3b-t16`)
//! - [`count_sql`] reads the unit's tokens (never its text): the first word must be `SELECT`,
//!   `WITH`, `VALUES` or `TABLE`; a locking clause (`FOR UPDATE`/`SHARE`/`NO KEY UPDATE`/
//!   `KEY SHARE`, MySQL's `LOCK IN SHARE MODE`), `INTO` (a `SELECT … INTO` creates a table or
//!   sets variables), an executable comment, a client command or a lexer error refuses it.
//!   Trailing `;` and trivia go; a top-level `ORDER BY` goes when no top-level `LIMIT`,
//!   `FETCH` or `OFFSET` follows it (the order then decides which rows count).
//! - The statement is a separate query that never runs by itself: only the user's Count rows
//!   asks for it ([`crate::ExecService::count`]). It is classified again there and refused
//!   unless the read-only check allows it, and it runs through the session like any statement,
//!   so a read-only (or production) session's guard applies to it unchanged (R-M3-52).

use std::sync::{Arc, Mutex, PoisonError};

use hacivat_results::{MemoryBudget, ResultStore, ResultWriter};
use hacivat_session::session::{RunEvent, RunObserver, UnitId};
use hacivat_values::{ResultSchema, ValueRef};
use sql_lexer::TokenKind;
use sql_split::DocSnapshot;
use sql_text::{TextRange, TextSize};

use crate::units::ExecUnit;

/// The count statement of `unit` in `doc`, or `None` when it cannot be counted (the module
/// docs).
pub fn count_sql(doc: &DocSnapshot, unit: &ExecUnit) -> Option<String> {
    let text = doc.text();
    let lexed = doc.lexed();
    let start = TextSize::try_from(unit.doc_range.start).ok()?;
    let end = TextSize::try_from(unit.doc_range.end).ok()?;
    if start >= end || unit.doc_range.end > text.len() {
        return None;
    }
    let range = TextRange::new(start, end);
    if lexed
        .errors()
        .iter()
        .any(|e| e.range.intersect(range).is_some())
    {
        return None;
    }
    // The unit's significant tokens: (kind, byte range, depth).
    let mut toks: Vec<(TokenKind, std::ops::Range<usize>, u32)> = Vec::new();
    let mut depth = 0u32;
    for i in lexed.tokens_in(range) {
        let r = lexed.range(i);
        // A token the unit's range cuts belongs to no statement of it.
        if r.start() < start || r.end() > end {
            return None;
        }
        if lexed.is_trivia(i) {
            continue;
        }
        let kind = lexed.kind(i);
        let span = usize::from(r.start())..usize::from(r.end());
        match kind {
            TokenKind::ExecutableComment | TokenKind::ClientCommand | TokenKind::Unknown => {
                return None;
            }
            TokenKind::LParen => {
                toks.push((kind, span, depth));
                depth += 1;
                continue;
            }
            TokenKind::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
        toks.push((kind, span, depth));
    }
    // Trailing `;`.
    while toks
        .last()
        .is_some_and(|(k, _, _)| *k == TokenKind::Semicolon)
    {
        toks.pop();
    }
    // One statement only.
    if toks.iter().any(|(k, _, _)| *k == TokenKind::Semicolon) {
        return None;
    }
    let word = |t: &(TokenKind, std::ops::Range<usize>, u32), w: &str| {
        t.0 == TokenKind::Word && text[t.1.clone()].eq_ignore_ascii_case(w)
    };
    let first = toks.first()?;
    if !["SELECT", "WITH", "VALUES", "TABLE"]
        .iter()
        .any(|w| word(first, w))
    {
        return None;
    }
    // Locking clauses and `INTO`, at any depth (a locking subquery locks as well).
    for (i, t) in toks.iter().enumerate() {
        let next = |n: usize| toks.get(i + n);
        let is = |n: usize, w: &str| next(n).is_some_and(|t| word(t, w));
        if word(t, "INTO") {
            return None;
        }
        if word(t, "FOR")
            && (is(1, "UPDATE")
                || is(1, "SHARE")
                || (is(1, "NO") && is(2, "KEY"))
                || (is(1, "KEY") && is(2, "SHARE")))
        {
            return None;
        }
        if word(t, "LOCK") && is(1, "IN") && is(2, "SHARE") {
            return None;
        }
    }
    // A top-level `ORDER BY` with no top-level `LIMIT`, `FETCH` or `OFFSET` after it.
    let mut cut = toks.len();
    if let Some(order) = toks.iter().enumerate().rposition(|(i, t)| {
        t.2 == 0 && word(t, "ORDER") && toks.get(i + 1).is_some_and(|n| word(n, "BY"))
    }) && !toks[order..]
        .iter()
        .any(|t| t.2 == 0 && ["LIMIT", "FETCH", "OFFSET"].iter().any(|w| word(t, w)))
    {
        cut = order;
    }
    let last = toks[..cut].last()?;
    let q = &text[first.1.start..last.1.end];
    Some(format!("SELECT COUNT(*) FROM (\n{q}\n) hv_c"))
}

/// The run observer of a count: keeps the store of its one result and reads its first cell.
pub(crate) struct CountObserver {
    budget: Arc<MemoryBudget>,
    store: Mutex<Option<Arc<ResultStore>>>,
}

impl CountObserver {
    pub(crate) fn new(budget: Arc<MemoryBudget>) -> Self {
        CountObserver {
            budget,
            store: Mutex::new(None),
        }
    }

    /// The count: the first cell of the first row, a whole number of rows.
    pub(crate) fn count(&self) -> Option<u64> {
        let store = self
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()?;
        let snap = store.snapshot();
        if snap.row_count() == 0 {
            return None;
        }
        match snap.cell(0, 0) {
            ValueRef::Int(n) => u64::try_from(n).ok(),
            ValueRef::UInt(n) => Some(n),
            ValueRef::Decimal(d) => d.to_string().parse().ok(),
            ValueRef::Text(t) => t.trim().parse().ok(),
            _ => None,
        }
    }
}

impl RunObserver for CountObserver {
    fn open_result(&self, _unit: UnitId, _ordinal: u32, schema: Arc<ResultSchema>) -> ResultWriter {
        let id = crate::observer::next_result_id();
        let (store, writer) = ResultStore::new(id, schema, self.budget.clone(), 1);
        self.store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(store);
        writer
    }

    fn event(&self, _ev: &RunEvent) {}
}
