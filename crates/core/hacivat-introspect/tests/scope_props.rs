//! alg/02 HV-INTRO-02 test 10 (M4 Task 12): `to_text(parse(p)) == normalize(p)` for 10k
//! generated patterns, and the canonical text parses back to the same syntax tree.

use hacivat_introspect::scope::parse_scope;
use proptest::prelude::*;
use sql_dialect::DialectId;

/// A term as `(text with optional spacing inside, canonical text)`; terms carry no spacing.
fn term() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("@".to_owned()),
        Just("*".to_owned()),
        prop::sample::select(vec![
            "{t.*}",
            "{a|b,c}",
            r"{x\}y}",
            r"{[a-z]+\d{1,2}}",
            "{(?i)ab}",
            r"{tenant_\d+}",
            "{é|ş}",
        ])
        .prop_map(str::to_owned),
        // Bare names: none of `: , | { } ! " @ *` or whitespace.
        "[a-zA-Z0-9_$.#éşİı-]{1,8}",
        // Quoted names: anything, `"` doubled.
        "[a-zA-Z :,|{}!@*\"éİ]{1,8}"
            .prop_map(|s: String| format!("\"{}\"", s.replace('"', "\"\""))),
    ]
}

/// Optional whitespace (the parser ignores it around terms and separators).
fn sp() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec!["", "", " ", "  ", "\t"])
}

fn terms(max: usize) -> impl Strategy<Value = (String, String)> {
    prop::collection::vec((term(), sp(), sp()), 1..=max).prop_map(|ts| {
        let spaced = ts
            .iter()
            .map(|(t, a, b)| format!("{a}{t}{b}"))
            .collect::<Vec<_>>()
            .join(",");
        let canon = ts
            .iter()
            .map(|(t, _, _)| t.as_str())
            .collect::<Vec<_>>()
            .join(",");
        (spaced, canon)
    })
}

/// A clause: `!`? then `db` alone, `db:list` (both dialects), or a bare list (one level).
fn clause(two_level: bool) -> impl Strategy<Value = (String, String)> {
    let shape = if two_level { 0..2u8 } else { 0..3u8 };
    (any::<bool>(), term(), terms(4), shape, sp()).prop_map(
        move |(neg, db, (list_sp, list), shape, s)| {
            let bang = if neg { "!" } else { "" };
            match shape {
                0 => (format!("{bang}{s}{db}"), format!("{bang}{db}")),
                1 => (
                    format!("{bang}{db}{s}:{list_sp}"),
                    format!("{bang}{db}:{list}"),
                ),
                _ => (format!("{bang}{list_sp}"), format!("{bang}{list}")),
            }
        },
    )
}

fn scope_pattern_strategy() -> impl Strategy<Value = (DialectId, String, String)> {
    prop_oneof![Just(DialectId::Postgres), Just(DialectId::MySql)].prop_flat_map(|d| {
        let two = d == DialectId::Postgres;
        prop::collection::vec((clause(two), sp(), sp()), 1..=5).prop_map(move |cs| {
            let spaced = cs
                .iter()
                .map(|((t, _), a, b)| format!("{a}{t}{b}"))
                .collect::<Vec<_>>()
                .join("|");
            let canon = cs
                .iter()
                .map(|((_, c), _, _)| c.as_str())
                .collect::<Vec<_>>()
                .join("|");
            (d, spaced, canon)
        })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn to_text_parse_round_trip(p in scope_pattern_strategy()) {
        let (dialect, spaced, canon) = p;
        let parsed = parse_scope(&spaced, dialect)
            .unwrap_or_else(|e| panic!("{spaced:?} parses on {dialect:?}: {e:?}"));
        prop_assert_eq!(parsed.to_text(), canon.as_str());
        let again = parse_scope(parsed.to_text(), dialect).expect("the canonical text parses");
        prop_assert_eq!(&again, &parsed);
        prop_assert_eq!(again.to_text(), parsed.to_text());
    }

    #[test]
    fn any_text_parses_or_errors_inside_it(s in "\\PC{0,40}") {
        for d in [DialectId::Postgres, DialectId::MySql, DialectId::Sqlite] {
            match parse_scope(&s, d) {
                Ok(p) => {
                    let again = parse_scope(p.to_text(), d).expect("the canonical text parses");
                    prop_assert_eq!(again.to_text(), p.to_text());
                }
                Err(e) => {
                    let len = u32::try_from(s.chars().count()).unwrap();
                    prop_assert!(e.column >= 1 && e.column <= len + 1, "{:?} at {}", s, e.column);
                }
            }
        }
    }
}
