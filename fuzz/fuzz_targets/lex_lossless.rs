//! Fuzz target `lex_lossless` (SQL-T13; Task 20): the lexer is total and lossless for every
//! profile and mode (SQL-LEX-02). Every token is non-empty, the tokens tile the text in order
//! and their bytes concatenate to it; every error lies inside the text; lexing is a pure
//! function. The seeded PR form is `sql-lexer/tests/props.rs`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sql_dialect::{DialectCtx, DialectId, LexModes, PgWireFlavor, lex_profile};
use sql_lexer::{LexState, lex};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    for d in [
        DialectId::Postgres,
        DialectId::PgWire(PgWireFlavor::Redshift),
        DialectId::MySql,
        DialectId::MariaDb,
        DialectId::Sqlite,
        DialectId::Generic,
    ] {
        let ctx = DialectCtx::new(d, None);
        let Ok(p) = lex_profile(&ctx) else {
            continue;
        };
        let base = LexModes::default_for(d);
        let mut variants = vec![base];
        variants.push(LexModes {
            standard_conforming_strings: !base.standard_conforming_strings,
            ..base
        });
        variants.push(LexModes {
            no_backslash_escapes: !base.no_backslash_escapes,
            ansi_quotes: !base.ansi_quotes,
            ..base
        });
        for modes in variants {
            let l = lex(text, p, modes, &LexState::Initial);
            let mut at = 0usize;
            let mut joined = String::with_capacity(text.len());
            for i in 0..l.len() {
                let r = l.range(i);
                let (a, b) = (usize::from(r.start()), usize::from(r.end()));
                assert_eq!(
                    a, at,
                    "{d:?}: token {i} does not start where the last ended"
                );
                assert!(b > a, "{d:?}: empty token {i}");
                joined.push_str(&text[a..b]);
                at = b;
            }
            assert_eq!(at, text.len(), "{d:?}: the tokens stop short");
            assert_eq!(joined, text, "{d:?}: the tokens are not the text");
            for e in l.errors() {
                assert!(
                    usize::from(e.range.end()) <= text.len(),
                    "{d:?}: error outside"
                );
            }
            // Lexing the same text again is the same lexing (a pure function).
            assert!(
                lex(text, p, modes, &LexState::Initial) == l,
                "{d:?}: not pure"
            );
        }
    }
});
