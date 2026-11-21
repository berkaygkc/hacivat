//! M4 Task 7 review hostile test (fix round 1), for `sql_ident::quote`. It renders an
//! arbitrary identifier or string literal and re-lexes it with the repo's own lexer under
//! EVERY server mode the quoting context does not pin down. It requires exactly one
//! identifier/string token that decodes to the original text, followed by an untouched
//! sentinel token.
//!
//! It lives in `sql-safety` (a named exception, new file) because this is the one crate that
//! already depends on both `sql-ident` and `sql-lexer` and has `proptest` as a
//! dev-dependency. Neither `sql-ident` nor `hacivat-ddl` reaches `sql-lexer`, and no manifest
//! may change.

use proptest::prelude::*;
use sql_dialect::{DialectCtx, DialectId, LexModes, PgWireFlavor, lex_profile};
use sql_ident::quote::{QuoteCtx, QuoteError, needs_quoting, quote as qid, string_literal};
use sql_lexer::{LexState, TokenFlags, TokenKind, lex};

const SENT: &str = " zz9";

fn toks(d: DialectId, modes: LexModes, text: &str) -> Vec<(TokenKind, String, TokenFlags)> {
    let ctx = DialectCtx {
        id: d,
        version: None,
        modes,
    };
    let p = lex_profile(&ctx).expect("profile");
    let l = lex(text, p, modes, &LexState::Initial);
    (0..l.len())
        .map(|i| {
            let r = l.range(i);
            (
                l.kind(i),
                text[usize::from(r.start())..usize::from(r.end())].to_string(),
                l.flags(i),
            )
        })
        .collect()
}

fn is_pg(d: DialectId) -> bool {
    matches!(d, DialectId::Postgres | DialectId::PgWire(_))
}
fn is_my(d: DialectId) -> bool {
    matches!(d, DialectId::MySql | DialectId::MariaDb)
}

/// Every server lexical mode that matters for `d`.
fn server_modes(d: DialectId) -> Vec<LexModes> {
    let mut v = Vec::new();
    if is_pg(d) {
        for scs in [true, false] {
            v.push(LexModes {
                standard_conforming_strings: scs,
                ..LexModes::default()
            });
        }
    } else if is_my(d) {
        for ansi in [false, true] {
            for nbe in [false, true] {
                v.push(LexModes {
                    ansi_quotes: ansi,
                    no_backslash_escapes: nbe,
                    ..LexModes::default()
                });
            }
        }
    } else {
        v.push(LexModes::default_for(d));
    }
    v
}

fn ctxs() -> Vec<QuoteCtx> {
    let o = [None, Some(false), Some(true)];
    let mut v = Vec::new();
    for a in o {
        for n in o {
            for s in o {
                v.push(QuoteCtx {
                    ansi_quotes: a,
                    no_backslash_escapes: n,
                    standard_conforming_strings: s,
                });
            }
        }
    }
    v
}

/// The server mode `m` is one `ctx` allows (a known setting pins its mode).
fn consistent(ctx: &QuoteCtx, m: &LexModes) -> bool {
    ctx.ansi_quotes.is_none_or(|x| x == m.ansi_quotes)
        && ctx
            .no_backslash_escapes
            .is_none_or(|x| x == m.no_backslash_escapes)
        && ctx
            .standard_conforming_strings
            .is_none_or(|x| x == m.standard_conforming_strings)
}

const DIALECTS: [DialectId; 6] = [
    DialectId::Postgres,
    DialectId::PgWire(PgWireFlavor::Cockroach),
    DialectId::MySql,
    DialectId::MariaDb,
    DialectId::Sqlite,
    DialectId::Generic,
];

fn check_rest(t: &[(TokenKind, String, TokenFlags)], what: &str) {
    assert_eq!(t.len(), 3, "{what}: token count {t:?}");
    assert_eq!(t[1].0, TokenKind::Whitespace, "{what}: {t:?}");
    assert_eq!(
        (t[2].0, t[2].1.as_str()),
        (TokenKind::Word, "zz9"),
        "{what}: {t:?}"
    );
    assert!(t[0].2.is_empty(), "{what}: flags {t:?}");
}

fn check_ident(s: &str) {
    for d in DIALECTS {
        for ctx in ctxs() {
            let r = qid(d, s, &ctx);
            if s.contains('\0') {
                assert_eq!(r, Err(QuoteError::Nul));
                continue;
            }
            let r = r.expect("quote");
            // An identifier must read the same under EVERY server mode: the identifier quote
            // never depends on a reported setting.
            for m in server_modes(d) {
                let text = format!("{r}{SENT}");
                let t = toks(d, m, &text);
                let what = format!("ident {d:?} {m:?} {ctx:?} {s:?} -> {r:?}");
                check_rest(&t, &what);
                match t[0].0 {
                    TokenKind::Word => {
                        assert_eq!(t[0].1, s, "{what}");
                        assert!(!needs_quoting(d, s, &ctx), "{what}");
                    }
                    TokenKind::QuotedIdent => {
                        let q = t[0].1.chars().next().unwrap();
                        assert!(t[0].1.ends_with(q) && t[0].1.len() >= 2, "{what}");
                        let inner = &t[0].1[q.len_utf8()..t[0].1.len() - q.len_utf8()];
                        let dq: String = [q, q].iter().collect();
                        assert_eq!(inner.replace(&dq, &q.to_string()), s, "{what}");
                    }
                    k => panic!("{what}: first token {k:?}"),
                }
            }
        }
    }
}

/// `'…'` content as the server reads it in mode `m`.
fn decode_single(inner: &str, backslash: bool, what: &str) -> String {
    let mut out = String::new();
    let mut it = inner.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\'' => {
                assert_eq!(it.next(), Some('\''), "{what}: lone quote");
                out.push('\'');
            }
            '\\' if backslash => {
                // The renderer only ever produces `\\`.
                assert_eq!(it.next(), Some('\\'), "{what}: escape other than \\\\");
                out.push('\\');
            }
            c => out.push(c),
        }
    }
    out
}

fn check_literal(s: &str) {
    for d in DIALECTS {
        for ctx in ctxs() {
            let r = string_literal(d, s, &ctx);
            if s.contains('\0') {
                assert_eq!(r, Err(QuoteError::Nul));
                continue;
            }
            let r = r.expect("literal");
            assert!(!r.starts_with(['E', 'e', 'U', 'u', 'N', 'n', 'B', 'b', 'X', 'x']));
            for m in server_modes(d).into_iter().filter(|m| consistent(&ctx, m)) {
                let text = format!("{r}{SENT}");
                let t = toks(d, m, &text);
                let what = format!("lit {d:?} {m:?} {ctx:?} {s:?} -> {r:?}");
                check_rest(&t, &what);
                let value = match t[0].0 {
                    TokenKind::String => {
                        let inner = &t[0].1[1..t[0].1.len() - 1];
                        let bs = (is_pg(d) && !m.standard_conforming_strings)
                            || (is_my(d) && !m.no_backslash_escapes);
                        decode_single(inner, bs, &what)
                    }
                    TokenKind::DollarString => {
                        assert!(is_pg(d), "{what}");
                        let tag_end = t[0].1[1..].find('$').unwrap() + 2;
                        let tag = &t[0].1[..tag_end];
                        assert!(
                            t[0].1.ends_with(tag) && t[0].1.len() >= 2 * tag.len(),
                            "{what}"
                        );
                        t[0].1[tag.len()..t[0].1.len() - tag.len()].to_string()
                    }
                    k => panic!("{what}: first token {k:?}"),
                };
                // The one documented inexact case: MySQL with NO_BACKSLASH_ESCAPES unknown
                // and actually on keeps a doubled backslash (never a breakout).
                let expected =
                    if is_my(d) && ctx.no_backslash_escapes.is_none() && m.no_backslash_escapes {
                        s.replace('\\', "\\\\")
                    } else {
                        s.to_string()
                    };
                assert_eq!(value, expected, "{what}");
            }
        }
    }
}

const FRAG: &[&str] = &[
    "'", "''", "\"", "`", "``", "\\", "\\\\", "\\'", "'\\", "$", "$$", "$hv", "$hv$", "$hv1",
    "$hv1$", "hv", "$h", "--", "/*", "*/", "/*!", "#", ";", "\n", "\r", "\r\n", "\t", " ", "a",
    "Z", "_", "1", "e", "E'", "U&", "N'", "x'", "İ", "ı", "ß", "\u{feff}", "\u{a0}", "😀", "[",
    "]", "select", "order", "user", "id", "\0", "?", ":a", "@", "%", "zz9",
];

fn hostile() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::collection::vec(prop::sample::select(FRAG), 0..24).prop_map(|v| v.concat()),
        2 => any::<String>(),
        1 => "[\\x00-\\x7f]{0,24}",
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 6000, ..ProptestConfig::default() })]
    #[test]
    fn identifiers_round_trip_through_the_lexer(s in hostile()) {
        check_ident(&s);
    }

    #[test]
    fn literals_round_trip_through_the_lexer(s in hostile()) {
        check_literal(&s);
    }
}

#[test]
fn fixed_hostile_cases() {
    for s in [
        "x\\\" OR 1=1 -- ",
        "x\" OR 1=1 -- \"",
        "a` OR 1=1 -- `",
        "\\",
        "a\\",
        "\\'",
        "'\\",
        "\\' OR 1=1 --",
        "$hv$",
        "x$hv",
        "$hv$hv1$hv2$",
        "a$",
        "$",
        "",
        " ",
        "a ",
        "İ",
        "\u{feff}x",
        "select",
        "SELECT",
        "_utf8mb4",
        "_binary",
    ] {
        check_ident(s);
        check_literal(s);
    }
}

/// Review I1: the declared dialects without a verified rule are refused, never given a plain
/// `"…"`/`'…'`. ClickHouse reads `\` as an escape inside '…', "…" and `…`, so the
/// doubled-quote form alone would let a trailing backslash escape the closing quote; SQL
/// Server's `"` is an identifier quote only under `QUOTED_IDENTIFIER ON`.
#[test]
fn unruled_dialects_are_refused() {
    for d in [
        DialectId::ClickHouse,
        DialectId::SqlServer,
        DialectId::Oracle,
    ] {
        for ctx in ctxs() {
            assert_eq!(
                string_literal(d, "\\' OR 1=1 --", &ctx),
                Err(QuoteError::NotImplemented),
                "{d:?}"
            );
            assert_eq!(
                qid(d, "x\\\" OR 1=1 --", &ctx),
                Err(QuoteError::NotImplemented),
                "{d:?}"
            );
            assert_eq!(qid(d, "a", &ctx), Err(QuoteError::NotImplemented), "{d:?}");
        }
    }
}
