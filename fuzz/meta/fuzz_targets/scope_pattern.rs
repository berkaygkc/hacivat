//! Fuzz target `scope_pattern` (M4 Task 12): parsing a scope pattern (HV-INTRO-02) or an
//! object filter (HV-INTRO-03) never panics, and every answer passes these oracles:
//! - a scope error points inside the text: column 1 ..= characters + 1 (Unicode scalars);
//! - a parsed scope's canonical text parses back to the same pattern and the same text, and
//!   selecting over a few namespaces (one named like the input's first line) never panics;
//! - a parsed filter's matcher answers, and its pushdown never puts user text in SQL: the
//!   fragment is the one fixed pack text, every bind is a `LIKE` pattern whose unescaped
//!   characters come from the input, with no NUL.
//!
//! The first byte picks the dialect and the filter role; the rest is the pattern (UTF-8 only).
//! Built without `hacivat-introspect`'s `session` feature (ADR 0008 `m4-t01a`).

#![no_main]

use std::sync::Arc;

use hacivat_introspect::filter::{FilterRole, SqlPredicate, parse_filter};
use hacivat_introspect::pack::{BindValue, DialectId};
use hacivat_introspect::scope::{CurrentCtx, NsInfo, parse_scope};
use hacivat_meta::name::IdentCtx;
use hacivat_meta::{NsKey, ObjectKind};
use libfuzzer_sys::fuzz_target;

const DIALECTS: [DialectId; 4] = [
    DialectId::Postgres,
    DialectId::MySql,
    DialectId::MariaDb,
    DialectId::Sqlite,
];

fuzz_target!(|data: &[u8]| {
    let Some((&head, rest)) = data.split_first() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    let dialect = DIALECTS[usize::from(head) % DIALECTS.len()];
    let role = if head & 0x80 == 0 {
        FilterRole::Introspect
    } else {
        FilterRole::View
    };
    let chars = text.chars().count();

    match parse_scope(text, dialect) {
        Ok(p) => {
            let again = parse_scope(p.to_text(), dialect).expect("the canonical text parses");
            assert_eq!(again, p);
            assert_eq!(again.to_text(), p.to_text());
            let word: Arc<str> = Arc::from(text.lines().next().unwrap_or(""));
            let two = dialect == DialectId::Postgres;
            let key = |s: &Arc<str>| {
                if two {
                    NsKey::schema([Arc::from("db"), s.clone()])
                } else {
                    NsKey::schema([s.clone()])
                }
            };
            let infos: Vec<NsInfo> = [word.clone(), Arc::from("public"), Arc::from("pg_catalog")]
                .iter()
                .enumerate()
                .map(|(i, s)| NsInfo {
                    key: key(s),
                    system: i == 2,
                })
                .collect();
            let ctx = CurrentCtx {
                database: Some(Arc::from("db")),
                schema: Some(word),
                ident: Some(IdentCtx {
                    lower_case_table_names: head % 3,
                    ..IdentCtx::new(dialect)
                }),
            };
            let all = p.select(&infos, &ctx);
            assert!(p.select_explicit(&infos, &ctx).is_subset(&all));
            assert!(all.iter().all(|k| infos.iter().any(|i| &i.key == k)));
        }
        Err(e) => assert!(
            e.column >= 1 && e.column as usize <= chars + 1,
            "column {} of {chars}",
            e.column
        ),
    }

    let Ok(f) = parse_filter(text, dialect, role) else {
        return;
    };
    let ident = IdentCtx {
        lower_case_table_names: head % 3,
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(dialect)
    };
    let f = f.with_ident(ident.clone());
    let name: Arc<str> = Arc::from(text);
    for kind in [
        ObjectKind::Table,
        ObjectKind::View,
        ObjectKind::Routine,
        ObjectKind::Column,
        ObjectKind::Trigger,
    ] {
        let _ = f.matches(kind, &name);
        let _ = f.matches_signature(kind, &name, text);
        let _ = f.excludes(Some(kind), Some(text));
        let Some(p) = f.pushdown(kind, dialect, &ident) else {
            continue;
        };
        assert_eq!(role, FilterRole::Introspect);
        assert_eq!(p.sql_fragment, SqlPredicate::FRAGMENT);
        assert!(!p.binds.is_empty());
        for b in &p.binds {
            let BindValue::Typed { text: like, .. } = b else {
                panic!("a pattern bind is text");
            };
            assert!(!like.contains('\0'));
            // Every unescaped character is a wildcard or came from the input (ASCII-lowercased
            // under MySQL `lower_case_table_names = 1`).
            let mut it = like.chars();
            while let Some(c) = it.next() {
                let c = if c == '\\' {
                    it.next().expect("an escape is followed by its character")
                } else {
                    c
                };
                assert!(
                    matches!(c, '%' | '_') || text.chars().any(|t| t.eq_ignore_ascii_case(&c)),
                    "{like:?}"
                );
            }
        }
        assert_eq!(p.runs().count(), p.binds.len());
    }
    // A multi-kind section gets a predicate only when every kind has one, and then every
    // kind's own patterns are in it (never fewer rows than the client keeps).
    let kinds = [ObjectKind::Table, ObjectKind::View];
    if let Some(all) = f.pushdown_kinds(&kinds, dialect, &ident) {
        for k in kinds {
            let one = f.pushdown(k, dialect, &ident).expect("each kind pushes");
            assert!(one.binds.iter().all(|b| all.binds.contains(b)));
        }
    }
    for k in [ObjectKind::Schema, ObjectKind::Database] {
        assert!(f.matches(k, &name));
        assert!(f.pushdown(k, dialect, &ident).is_none());
    }
});
