//! Identifier folding, comparison and qualified names (M3b Task 3, SQL-SAFE-03 names).

use sql_dialect::DialectId;
use sql_ident::{Ident, fold, ident_eq, parse_qualified};

fn id(text: &str, quoted: bool) -> Ident {
    Ident {
        text: text.into(),
        quoted,
    }
}

#[test]
fn pg_folds_unquoted_ascii_only() {
    assert_eq!(&*fold(DialectId::Postgres, "PG_SLEEP", false), "pg_sleep");
    assert_eq!(&*fold(DialectId::Postgres, "PG_SLEEP", true), "PG_SLEEP");
}

#[test]
fn turkish_dotted_capital_never_folds_to_i() {
    // Only ASCII letters are lowered.
    assert_eq!(
        &*fold(DialectId::Postgres, "PG_READ_FİLE", false),
        "pg_read_fİle"
    );
    assert_eq!(&*fold(DialectId::Postgres, "ILIŞKI", false), "iliŞki");
    assert_eq!(&*fold(DialectId::Postgres, "ıI", false), "ıi");
}

#[test]
fn qualified_names_split_on_dots_outside_quotes() {
    let q = parse_qualified(DialectId::Postgres, r#"pg_catalog."pg_sleep""#).unwrap();
    assert_eq!(q.parts.len(), 2);
    assert!(q.parts[1].quoted);
    assert!(!q.parts[0].quoted);
    assert_eq!(&*q.parts[0].text, "pg_catalog");
    assert_eq!(&*q.parts[1].text, "pg_sleep");

    let q = parse_qualified(DialectId::Postgres, r#""a.b"."c""d""#).unwrap();
    assert_eq!(q.parts.len(), 2);
    assert_eq!(&*q.parts[0].text, "a.b");
    assert_eq!(&*q.parts[1].text, r#"c"d"#);
}

#[test]
fn qualified_names_tolerate_space_around_dots_and_keep_case() {
    let q = parse_qualified(DialectId::Postgres, " Public . PG_Sleep ").unwrap();
    assert_eq!(q.parts.len(), 2);
    // The text is kept as written; `fold` resolves it.
    assert_eq!(&*q.parts[1].text, "PG_Sleep");
    assert!(!q.parts[1].quoted);
}

#[test]
fn malformed_names_are_not_recognised() {
    for bad in [
        "",
        "   ",
        "a.",
        ".a",
        "a..b",
        r#""unterminated"#,
        r#""""#,
        "a b",
        "a.b c",
        r#"U&"d\0061t""#,
        "f(x)",
        "a;b",
    ] {
        assert_eq!(parse_qualified(DialectId::Postgres, bad), None, "{bad:?}");
    }
}

#[test]
fn unicode_spaces_belong_to_the_identifier() {
    // The servers' lexers read bytes >= 0x80 as identifier characters: a no-break or ideographic
    // space is part of the name, never a separator (fix round 1, I4).
    for (d, text, want) in [
        (
            DialectId::Postgres,
            "\u{a0}pg_catalog.lower",
            ["\u{a0}pg_catalog", "lower"],
        ),
        (
            DialectId::Postgres,
            "pg_catalog.\u{2003}lower",
            ["pg_catalog", "\u{2003}lower"],
        ),
        (
            DialectId::Postgres,
            "pg_catalog\u{3000}.lower",
            ["pg_catalog\u{3000}", "lower"],
        ),
        (DialectId::MySql, "db.\u{202f}abs", ["db", "\u{202f}abs"]),
    ] {
        let q = parse_qualified(d, text).unwrap_or_else(|| panic!("{text:?}"));
        let got: Vec<&str> = q.parts.iter().map(|p| &*p.text).collect();
        assert_eq!(got, want, "{text:?}");
    }
    // A Unicode space between two names does not split them into qualified parts either.
    assert_eq!(
        parse_qualified(DialectId::Postgres, "a\u{a0}b")
            .unwrap()
            .parts
            .len(),
        1
    );
    // SQL whitespace still surrounds a dot; vertical tab is refused (PG < 16 does not skip it).
    assert_eq!(
        parse_qualified(DialectId::Postgres, "\t pg_catalog\r\n.\x0clower")
            .unwrap()
            .parts
            .len(),
        2
    );
    assert_eq!(
        parse_qualified(DialectId::Postgres, "\x0bpg_catalog.lower"),
        None
    );
}

#[test]
fn mysql_quotes_with_backticks_and_keeps_case() {
    let q = parse_qualified(DialectId::MySql, "`my``db`.SLEEP").unwrap();
    assert_eq!(q.parts.len(), 2);
    assert_eq!(&*q.parts[0].text, "my`db");
    assert!(q.parts[0].quoted);
    assert_eq!(&*fold(DialectId::MySql, "SLEEP", false), "SLEEP");
    // A double-quoted MySQL token is a string, not a name.
    assert_eq!(parse_qualified(DialectId::MySql, r#""sleep""#), None);
    assert!(
        parse_qualified(DialectId::MariaDb, "`sleep`")
            .unwrap()
            .parts[0]
            .quoted
    );
}

#[test]
fn sqlite_accepts_its_three_quote_forms() {
    for (text, want) in [(r#""abs""#, "abs"), ("`abs`", "abs"), ("[abs]", "abs")] {
        let q = parse_qualified(DialectId::Sqlite, text).unwrap();
        assert_eq!(&*q.parts[0].text, want, "{text}");
        assert!(q.parts[0].quoted, "{text}");
    }
    let q = parse_qualified(DialectId::Sqlite, "main.[my.t]").unwrap();
    assert_eq!(q.parts.len(), 2);
    assert_eq!(&*q.parts[1].text, "my.t");
}

#[test]
fn pg_truncates_names_to_63_bytes_on_a_char_boundary() {
    let long = "a".repeat(70);
    assert_eq!(fold(DialectId::Postgres, &long, false).len(), 63);
    // 62 ASCII bytes and a two-byte letter: the letter does not fit and is dropped whole.
    let edge = format!("{}ş", "b".repeat(62));
    assert_eq!(&*fold(DialectId::Postgres, &edge, true), "b".repeat(62));
    // MySQL keeps the text as written.
    assert_eq!(fold(DialectId::MySql, &long, false).len(), 70);
}

#[test]
fn pg_ident_eq_follows_folding() {
    let d = DialectId::Postgres;
    assert!(ident_eq(d, &id("Users", false), &id("users", true)));
    assert!(!ident_eq(d, &id("Users", true), &id("users", true)));
    assert!(!ident_eq(d, &id("FİLE", false), &id("file", false)));
}

#[test]
fn mysql_and_sqlite_ident_eq_is_ascii_case_insensitive() {
    for d in [DialectId::MySql, DialectId::MariaDb, DialectId::Sqlite] {
        assert!(ident_eq(d, &id("T", false), &id("t", true)), "{d:?}");
        assert!(!ident_eq(d, &id("İ", false), &id("i", false)), "{d:?}");
        assert!(!ident_eq(d, &id("ı", false), &id("I", false)), "{d:?}");
    }
}

#[test]
fn folding_ignores_the_process_locale() {
    // The CI turkey-test runs this under `LC_ALL=tr_TR.UTF-8`; ASCII casing never consults it.
    assert_eq!(&*fold(DialectId::Postgres, "INDEX", false), "index");
    assert!(ident_eq(
        DialectId::MySql,
        &id("INFO", false),
        &id("info", false)
    ));
}
