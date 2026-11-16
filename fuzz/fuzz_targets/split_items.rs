//! Fuzz target `split_items` (SQL-T13; Task 20): the splitter's items for every dialect and
//! client flavour, in both modes, are in text order, never overlap, keep each strict range
//! inside its smart range and are never empty statements; every significant token is in an
//! item of the execution split; the split is a pure function of its input. The seeded PR form is
//! `sql-split/tests/split_props.rs`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, lex_profile};
use sql_lexer::{LexState, lex};
use sql_split::{Item, SplitMode, SplitStart, split_from};
use sql_text::TextRange;

fn ranges(i: &Item) -> (TextRange, TextRange) {
    match i {
        Item::Statement(s) => (s.strict, s.smart),
        Item::Empty { range } => (*range, *range),
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    for (d, flavor) in [
        (DialectId::Postgres, ClientFlavor::None),
        (DialectId::Postgres, ClientFlavor::Psql),
        (DialectId::MySql, ClientFlavor::None),
        (DialectId::MySql, ClientFlavor::MySqlCli),
        (DialectId::MariaDb, ClientFlavor::MySqlCli),
        (DialectId::Sqlite, ClientFlavor::Sqlite3),
        (DialectId::Sqlite, ClientFlavor::None),
        (DialectId::Generic, ClientFlavor::None),
    ] {
        let ctx = DialectCtx::new(d, None);
        for mode in [SplitMode::Editor, SplitMode::Execution] {
            let split = split_from(text, &ctx, flavor, mode, &SplitStart::default());
            let again = split_from(text, &ctx, flavor, mode, &SplitStart::default());
            assert!(split == again, "{d:?} {flavor:?} {mode:?}: not pure");
            let (mut last_strict, mut last_smart) = (0u32, 0u32);
            for (n, i) in split.items.iter().enumerate() {
                let (strict, smart) = ranges(i);
                assert!(
                    smart.contains_range(strict),
                    "{d:?}: item {n} strict outside smart"
                );
                assert!(
                    u32::from(strict.start()) >= last_strict
                        && u32::from(smart.start()) >= last_smart,
                    "{d:?}: item {n} overlaps the one before"
                );
                assert!(
                    !(strict.is_empty() && matches!(i, Item::Statement(_))),
                    "{d:?}: item {n} is an empty statement"
                );
                assert!(
                    usize::from(smart.end()) <= text.len(),
                    "{d:?}: item {n} outside"
                );
                last_strict = strict.end().into();
                last_smart = smart.end().into();
            }
            // Coverage is the execution split's (what Run sends); the editor split's recovery
            // after an unterminated token may cut through it, as `split_props` allows.
            if flavor != ClientFlavor::None || mode != SplitMode::Execution {
                continue;
            }
            let Ok(p) = lex_profile(&ctx) else {
                continue;
            };
            let l = lex(text, p, ctx.modes, &LexState::Initial);
            for k in 0..l.len() {
                if l.is_trivia(k) {
                    continue;
                }
                let r = l.range(k);
                let inside = split.items.iter().any(|it| match it {
                    Item::Statement(s) => {
                        s.strict.contains_range(r)
                            || s.terminator.is_some_and(|t| t.contains_range(r))
                    }
                    Item::Empty { range } => range.contains_range(r),
                });
                assert!(
                    inside,
                    "{d:?} {mode:?}: a significant token at {r:?} is in no item"
                );
            }
        }
    }
});
