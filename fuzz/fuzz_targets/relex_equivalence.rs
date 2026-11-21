//! Fuzz target `relex_equivalence` (SQL-T13; Task 20): incremental analysis equals a fresh
//! one. The input seeds the one shared generator (`sql-lexer/tests/support/edits.rs`, T5 I1:
//! scripts of up to 16 KiB, runs of edits, edits biased to checkpoints, line starts and
//! statement starts); after every edit, `relex` equals a full lex, the lexer's checkpoints
//! hold, and `SqlDocument::apply` equals a fresh document (lexing, statements, both splits).
//! The seeded PR forms are `sql-lexer/tests/incremental.rs` and
//! `sql-split/tests/incremental.rs`.

#![no_main]

#[path = "../../crates/sql/sql-lexer/tests/support/edits.rs"]
mod edits;

use libfuzzer_sys::fuzz_target;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes, lex_profile};
use sql_lexer::{LexState, lex, relex};
use sql_split::SqlDocument;

fuzz_target!(|data: &[u8]| {
    let mut r = edits::Rng::from_bytes(data);
    let (d, flavor) = [
        (DialectId::Postgres, ClientFlavor::None),
        (DialectId::Postgres, ClientFlavor::Psql),
        (DialectId::MySql, ClientFlavor::None),
        (DialectId::MySql, ClientFlavor::MySqlCli),
        (DialectId::MariaDb, ClientFlavor::None),
        (DialectId::Sqlite, ClientFlavor::None),
    ][r.below(6)];
    let ctx = DialectCtx::new(d, None);
    let Ok(p) = lex_profile(&ctx) else {
        return;
    };
    let modes = LexModes::default_for(d);
    let mut text = edits::doc(&mut r);
    let mut l = lex(&text, p, modes, &LexState::Initial);
    let mut doc = SqlDocument::new(text.clone(), ctx, flavor);
    for _ in 0..1 + r.below(6) {
        let mut marks: Vec<usize> = l.checkpoints().iter().map(|c| usize::from(*c)).collect();
        marks.extend(
            doc.statements()
                .iter()
                .map(|s| usize::from(s.smart().start())),
        );
        let e = edits::edit(&mut r, &text, &marks);
        if e.apply(&mut text).is_err() {
            return;
        }
        relex(&mut l, &text.as_str(), &e, p, modes);
        assert!(
            l == lex(&text, p, modes, &LexState::Initial),
            "relex differs from a full lex"
        );
        edits::checkpoints_hold(&text, &l, p, modes).unwrap();
        doc.apply(std::slice::from_ref(&e)).unwrap();
        let fresh = SqlDocument::new(text.clone(), ctx, flavor);
        assert!(
            doc.lexed() == fresh.lexed(),
            "the document's lexing differs"
        );
        assert!(
            doc.statements() == fresh.statements(),
            "the statements differ"
        );
        let (a, b) = (doc.snapshot(), fresh.snapshot());
        assert!(
            a.editor_split() == b.editor_split(),
            "the editor split differs"
        );
        assert!(
            a.execution_split() == b.execution_split(),
            "the execution split differs"
        );
    }
});
