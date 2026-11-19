//! M4 Task 7: `sql_ident::quote` (R-M4-28, R-M4-37; alg/03 HV-DDL-07 quoting and literals;
//! T1a review m2: MySQL and MariaDB always backtick, and an unknown session setting gives the
//! fail-safe literal).

use sql_dialect::DialectId;
use sql_ident::quote::{
    QuoteCtx, QuoteError, needs_quoting, quote, quote_qualified, string_literal,
};

const DIALECTS: [DialectId; 4] = [
    DialectId::Postgres,
    DialectId::MySql,
    DialectId::MariaDb,
    DialectId::Sqlite,
];

/// No session setting known (what `DdlTarget::new` starts with).
fn qc() -> QuoteCtx {
    QuoteCtx::default()
}

fn known(nbe: bool, scs: bool) -> QuoteCtx {
    QuoteCtx {
        ansi_quotes: Some(false),
        no_backslash_escapes: Some(nbe),
        standard_conforming_strings: Some(scs),
    }
}

#[test]
fn string_literal_doubles_quotes_and_refuses_nul() {
    assert_eq!(
        string_literal(DialectId::Postgres, "PM's", &qc()).unwrap(),
        "'PM''s'"
    );
    assert!(matches!(
        string_literal(DialectId::Postgres, "a\0", &qc()),
        Err(QuoteError::Nul)
    ));
    for d in DIALECTS {
        assert_eq!(
            string_literal(d, "it's", &known(false, true)).as_deref(),
            Ok("'it''s'"),
            "{d:?}"
        );
        // Newlines, comment markers and `"` stay inside the one literal.
        assert_eq!(
            string_literal(d, "a\n-- b /* c */ \"d\"", &known(true, true)).as_deref(),
            Ok("'a\n-- b /* c */ \"d\"'"),
            "{d:?}"
        );
        assert_eq!(
            string_literal(d, "x\0y", &known(true, true)),
            Err(QuoteError::Nul)
        );
    }
}

#[test]
fn mysql_backslash_doubled_unless_no_backslash_escapes() {
    for d in [DialectId::MySql, DialectId::MariaDb] {
        assert_eq!(
            string_literal(d, r"it's C:\tmp", &known(false, true)).unwrap(),
            r"'it''s C:\\tmp'",
            "{d:?}"
        );
        assert_eq!(
            string_literal(d, r"it's C:\tmp", &known(true, true)).unwrap(),
            r"'it''s C:\tmp'",
            "{d:?}"
        );
        // Unknown `sql_mode`: the backslash is doubled, so a trailing backslash can never
        // escape the closing quote.
        assert_eq!(string_literal(d, r"a\", &qc()).unwrap(), r"'a\\'", "{d:?}");
    }
    // SQLite has no backslash escapes.
    assert_eq!(
        string_literal(DialectId::Sqlite, r"a\b", &qc()).unwrap(),
        r"'a\b'"
    );
}

#[test]
fn pg_literal_respects_standard_conforming_strings_and_never_uses_e_strings() {
    // On: the backslash is an ordinary character.
    assert_eq!(
        string_literal(DialectId::Postgres, r"a'b\c", &known(false, true)).unwrap(),
        r"'a''b\c'"
    );
    // Off or unknown: a dollar-quoted string, which no setting reads differently.
    for ctx in [known(false, false), qc()] {
        assert_eq!(
            string_literal(DialectId::Postgres, r"a'b\c", &ctx).unwrap(),
            r"$hv$a'b\c$hv$"
        );
        // Without a backslash the plain literal reads the same in both modes.
        assert_eq!(
            string_literal(DialectId::Postgres, "plain", &ctx).unwrap(),
            "'plain'"
        );
    }
    // The tag never occurs in the text (a text ending in `$hv` cannot close early).
    assert_eq!(
        string_literal(DialectId::Postgres, r"\ $hv$ x$hv", &qc()).unwrap(),
        r"$hv1$\ $hv$ x$hv$hv1$"
    );
    for ctx in [known(false, false), known(false, true), qc()] {
        let lit = string_literal(DialectId::Postgres, r"C:\x", &ctx).unwrap();
        assert!(!lit.starts_with('E') && !lit.starts_with('e'), "{lit}");
    }
}

#[test]
fn mysql_backtick_doubled_and_ansi_quotes_respected() {
    assert_eq!(quote(DialectId::MySql, "a`b", &qc()).unwrap(), "`a``b`");
    // `ANSI_QUOTES` is never trusted: the backtick quotes identifiers in every `sql_mode`, so
    // a name stays a name even when the setting reported for another session is wrong
    // (T1a review T-d: `"x\"" OR 1=1 -- "` under a wrong `ANSI_QUOTES`).
    let ansi = QuoteCtx {
        ansi_quotes: Some(true),
        ..QuoteCtx::default()
    };
    for d in [DialectId::MySql, DialectId::MariaDb] {
        for ctx in [qc(), ansi] {
            assert_eq!(quote(d, "a`b", &ctx).unwrap(), "`a``b`", "{d:?}");
            assert_eq!(
                quote(d, "x\\\" OR 1=1 -- ", &ctx).unwrap(),
                "`x\\\" OR 1=1 -- `",
                "{d:?}"
            );
            assert_eq!(quote(d, "order", &ctx).unwrap(), "`order`", "{d:?}");
        }
    }
    // PostgreSQL and SQLite double `"`.
    assert_eq!(
        quote(DialectId::Postgres, "say \"hi\"", &qc()).unwrap(),
        "\"say \"\"hi\"\"\""
    );
    assert_eq!(
        quote(DialectId::Sqlite, "a\"b", &qc()).unwrap(),
        "\"a\"\"b\""
    );
    for d in DIALECTS {
        assert_eq!(quote(d, "a\0b", &qc()), Err(QuoteError::Nul), "{d:?}");
    }
}

#[test]
fn trailing_space_and_turkish_names_need_quoting() {
    for d in DIALECTS {
        for name in [
            "a ",
            " a",
            "MÜŞTERİ",
            "müşteri",
            "ılık",
            "İ",
            "a-b",
            "1st",
            "",
        ] {
            assert!(needs_quoting(d, name, &qc()), "{d:?} {name:?}");
        }
    }
    assert_eq!(
        quote(DialectId::Postgres, "MÜŞTERİ", &qc()).unwrap(),
        "\"MÜŞTERİ\""
    );
    assert_eq!(quote(DialectId::MySql, "İ ", &qc()).unwrap(), "`İ `");
    assert_eq!(quote(DialectId::Sqlite, "ad ", &qc()).unwrap(), "\"ad \"");
}

#[test]
fn pg_quotes_like_the_servers_quote_identifier() {
    let pg = DialectId::Postgres;
    for bare in ["users", "_x", "a1", "t_2", "name", "status", "test_schema"] {
        assert!(!needs_quoting(pg, bare, &qc()), "{bare}");
        assert_eq!(quote(pg, bare, &qc()).unwrap(), bare);
    }
    // Upper case, `$`, and keywords other than unreserved ones (reserved, column-name and
    // type/function-name keywords).
    for quoted in ["Users", "a$", "order", "select", "user", "between", "int"] {
        assert!(needs_quoting(pg, quoted, &qc()), "{quoted}");
    }
    assert_eq!(
        quote(
            DialectId::PgWire(sql_dialect::PgWireFlavor::Cockroach),
            "Users",
            &qc()
        )
        .unwrap(),
        "\"Users\""
    );
}

#[test]
fn reserved_words_per_dialect() {
    // MySQL quotes reserved words only; mixed case stays bare (it is preserved).
    for bare in [
        "Users",
        "CamelCase",
        "status",
        "name",
        "comment",
        "manual",
        "_x",
    ] {
        assert!(!needs_quoting(DialectId::MySql, bare, &qc()), "{bare}");
    }
    // A word reserved on any MySQL line is quoted: `master_bind` up to 8.4, `qualify` from 8.4.
    for quoted in ["order", "select", "SELECT", "master_bind", "qualify", "$x"] {
        assert!(needs_quoting(DialectId::MySql, quoted, &qc()), "{quoted}");
    }
    // MariaDB reports no reserved flag: every keyword it lists is quoted.
    assert!(needs_quoting(DialectId::MariaDb, "status", &qc()));
    assert!(needs_quoting(DialectId::MariaDb, "row_number", &qc()));
    assert!(!needs_quoting(DialectId::MariaDb, "customers", &qc()));
    // SQLite quotes its keywords and non-identifiers.
    assert!(!needs_quoting(DialectId::Sqlite, "Notes", &qc()));
    assert!(needs_quoting(DialectId::Sqlite, "order", &qc()));
    assert!(needs_quoting(DialectId::Sqlite, "a.b", &qc()));
    // A dialect without rules quotes everything.
    assert!(needs_quoting(DialectId::SqlServer, "users", &qc()));
    assert_eq!(
        quote(DialectId::Generic, "users", &qc()).unwrap(),
        "\"users\""
    );
}

#[test]
fn quote_qualified_quotes_each_part() {
    assert_eq!(
        quote_qualified(DialectId::Postgres, &["public", "MÜŞTERİ"], &qc()).unwrap(),
        "public.\"MÜŞTERİ\""
    );
    assert_eq!(
        quote_qualified(DialectId::MySql, &["shop", "a`b"], &qc()).unwrap(),
        "shop.`a``b`"
    );
    assert_eq!(
        quote_qualified(DialectId::Sqlite, &["aux", "order"], &qc()).unwrap(),
        "aux.\"order\""
    );
    assert_eq!(
        quote_qualified(DialectId::Sqlite, &["main", "x\0"], &qc()),
        Err(QuoteError::Nul)
    );
}

/// The probe set of [`quote_is_identical_under_tr_locale`]: `i`/`I`-heavy names whose casing a
/// Turkish locale would change, quoted per dialect, plus literals.
fn probe_lines() -> Vec<String> {
    let names = [
        "INDEX", "index", "ındex", "İNDEX", "Id", "ID", "id", "İ", "ı", "TITLE",
    ];
    let mut out = Vec::new();
    for d in DIALECTS {
        for n in names {
            out.push(format!(
                "{d:?}|{n}|{}|{}",
                needs_quoting(d, n, &qc()),
                quote(d, n, &qc()).unwrap()
            ));
        }
        out.push(format!(
            "{d:?}|lit|{}",
            string_literal(d, r"İstanbul'ın \i", &qc()).unwrap()
        ));
    }
    out
}

const BEGIN: &str = "<<<hv-t07-quote-begin>>>";
const END: &str = "<<<hv-t07-quote-end>>>";

/// Prints [`probe_lines`]; run by [`quote_is_identical_under_tr_locale`] in a child process.
#[test]
#[ignore = "helper: run in a child process by quote_is_identical_under_tr_locale"]
fn quote_locale_probe() {
    println!("{BEGIN}");
    for l in probe_lines() {
        println!("{l}");
    }
    println!("{END}");
}

#[test]
fn quote_is_identical_under_tr_locale() {
    // The in-process answers are fixed values: keyword matching is ASCII-only.
    assert!(needs_quoting(DialectId::MySql, "INDEX", &qc()));
    assert!(needs_quoting(DialectId::MySql, "ındex", &qc()));
    assert!(needs_quoting(DialectId::MySql, "İNDEX", &qc()));
    assert!(!needs_quoting(DialectId::MySql, "TITLE", &qc()));
    assert_eq!(quote(DialectId::Postgres, "id", &qc()).unwrap(), "id");
    assert_eq!(quote(DialectId::Postgres, "ID", &qc()).unwrap(), "\"ID\"");

    // The same probes in a child process under a Turkish locale give the same bytes.
    let exe = std::env::current_exe().expect("test binary");
    let out = std::process::Command::new(exe)
        .args([
            "--exact",
            "quote_locale_probe",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("LC_ALL", "tr_TR.UTF-8")
        .env("LANG", "tr_TR.UTF-8")
        .output()
        .expect("run the probe");
    assert!(out.status.success(), "probe failed: {out:?}");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let start = stdout.find(BEGIN).expect("begin marker") + BEGIN.len();
    let end = stdout.find(END).expect("end marker");
    let child: Vec<&str> = stdout[start..end]
        .lines()
        .filter(|l| !l.is_empty())
        .collect();
    let here = probe_lines();
    assert_eq!(child, here.iter().map(String::as_str).collect::<Vec<_>>());
}
