//! M4 Task 1a seams of `sql_ident::key` and `sql_ident::quote`: the stubs fail closed until
//! Task 3 (`match_key`) and Task 7 (quoting) land.

use sql_dialect::DialectId;
use sql_ident::key::{IdentCtx, NameClass, ServerDependent, match_key};
use sql_ident::quote::{
    QuoteCtx, QuoteError, needs_quoting, quote, quote_qualified, string_literal,
};

const DIALECTS: [DialectId; 4] = [
    DialectId::Postgres,
    DialectId::MySql,
    DialectId::MariaDb,
    DialectId::Sqlite,
];

const CLASSES: [NameClass; 6] = [
    NameClass::Database,
    NameClass::Schema,
    NameClass::Table,
    NameClass::Column,
    NameClass::Routine,
    NameClass::Other,
];

/// Every probe is `ServerDependent` until Task 3: no key, so every lookup is `Unknown`, never a
/// wrong `Found` or `Missing`.
#[test]
fn stub_match_key_is_server_dependent() {
    for d in DIALECTS {
        let ctx = IdentCtx::new(d);
        for class in CLASSES {
            for (text, quoted) in [
                ("users", false),
                ("Users", true),
                ("MÜŞTERİ", false),
                ("", false),
            ] {
                assert_eq!(
                    match_key(d, &ctx, text, quoted, class),
                    Err(ServerDependent),
                    "{d:?} {class:?} {text:?}"
                );
            }
        }
    }
}

// ## m4-t07-quote (named exception): Task 7 filled the quoting bodies, so the stub's
// "always quote, no literal" answers became the real contract (full tests: `tests/quote.rs`).
// Renamed from `stub_quote_always_quotes_and_doubles_the_quote_char` (T7 review M7).
/// A name is bare where the dialect allows it, else quoted with the dialect's quote character
/// doubled inside (MySQL and MariaDB always backtick, T1a review m2); NUL is refused; string
/// literals are produced.
#[test]
fn quote_is_bare_where_allowed_and_doubles_the_quote_char() {
    let plain = QuoteCtx::default();
    let ansi = QuoteCtx {
        ansi_quotes: Some(true),
        ..QuoteCtx::default()
    };
    for d in DIALECTS {
        for name in ["select", "a b", "İ"] {
            assert!(needs_quoting(d, name, &plain), "{d:?} {name}");
        }
        assert!(!needs_quoting(d, "users", &plain), "{d:?}");
    }
    assert_eq!(
        quote(DialectId::Postgres, "a\"b", &plain).as_deref(),
        Ok("\"a\"\"b\"")
    );
    assert_eq!(
        quote(DialectId::Sqlite, "users", &plain).as_deref(),
        Ok("users")
    );
    assert_eq!(
        quote(DialectId::MySql, "a`b", &plain).as_deref(),
        Ok("`a``b`")
    );
    assert_eq!(
        quote(DialectId::MariaDb, "İ ", &plain).as_deref(),
        Ok("`İ `")
    );
    // `ANSI_QUOTES` is never trusted: the backtick quotes identifiers in every `sql_mode`.
    assert_eq!(
        quote(DialectId::MySql, "a\"b", &ansi).as_deref(),
        Ok("`a\"b`")
    );
    for d in DIALECTS {
        assert_eq!(quote(d, "a\0b", &plain), Err(QuoteError::Nul), "{d:?}");
        assert_eq!(
            string_literal(d, "it's", &plain).as_deref(),
            Ok("'it''s'"),
            "{d:?}"
        );
    }
    assert_eq!(
        quote_qualified(DialectId::Postgres, &["public", "T"], &plain).as_deref(),
        Ok("public.\"T\"")
    );
    assert_eq!(
        quote_qualified(DialectId::MySql, &["shop", "a`b"], &plain).as_deref(),
        Ok("shop.`a``b`")
    );
    assert_eq!(
        quote_qualified(DialectId::Sqlite, &["main", "x\0"], &plain),
        Err(QuoteError::Nul)
    );
}
// ## end m4-t07-quote
