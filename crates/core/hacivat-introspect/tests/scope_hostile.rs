//! T12 security review: hostile tests the implementer did not write (M4 Task 12 review,
//! committed in fix round 1 with each finding flipped to assert its fix). Each test asserts the
//! invariant the task promises ("the server returns a superset of what the client keeps";
//! "user text reaches the server only as a bind").

use std::sync::Arc;
use std::time::Instant;

use hacivat_introspect::filter::{CompiledFilter, FilterRole, parse_filter};
use hacivat_introspect::glob::{Glob, GlobCase};
use hacivat_introspect::scope::{CurrentCtx, NsInfo, ScopeErrorKind, parse_scope};
use hacivat_meta::name::{IdentCtx, Name};
use hacivat_meta::{NsKey, ObjectKind};
use hacivat_values::BindValue;
use proptest::prelude::*;
use sql_dialect::DialectId;

const PG: DialectId = DialectId::Postgres;
const MY: DialectId = DialectId::MySql;
const LITE: DialectId = DialectId::Sqlite;

fn n(s: &str) -> Name {
    Arc::from(s)
}

fn f(text: &str, d: DialectId) -> CompiledFilter {
    parse_filter(text, d, FilterRole::Introspect).unwrap()
}

fn bind_text(b: &BindValue) -> String {
    match b {
        BindValue::Typed { text, .. } => text.to_string(),
        other => panic!("not a text bind: {other:?}"),
    }
}

// ---- a reference LIKE (ESCAPE '\'), by characters or by bytes ------------------------------

#[derive(Clone, Copy)]
enum Unit {
    /// PostgreSQL UTF-8 / multibyte (MB_MatchText), MySQL, SQLite: `_` is one character.
    Char,
    /// PostgreSQL `SQL_ASCII` and single-byte encodings (SB_MatchText): `_` is one byte.
    Byte,
}

#[derive(Clone, Copy, Debug)]
enum P {
    Any,
    One,
    Lit(u32),
}

fn units(s: &str, unit: Unit) -> Vec<u32> {
    match unit {
        Unit::Char => s.chars().map(u32::from).collect(),
        Unit::Byte => s.bytes().map(u32::from).collect(),
    }
}

fn like(subject: &str, pattern: &str, ascii_ci: bool, unit: Unit) -> bool {
    let mut ps = Vec::new();
    let mut it = pattern.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => {
                let e = it
                    .next()
                    .expect("LIKE pattern must not end with the escape");
                ps.extend(
                    units(e.encode_utf8(&mut [0; 4]), unit)
                        .into_iter()
                        .map(P::Lit),
                );
            }
            '%' => ps.push(P::Any),
            '_' => ps.push(P::One),
            c => ps.extend(
                units(c.encode_utf8(&mut [0; 4]), unit)
                    .into_iter()
                    .map(P::Lit),
            ),
        }
    }
    let fold = |u: u32| {
        if ascii_ci && u < 128 {
            u32::from((u as u8).to_ascii_lowercase())
        } else {
            u
        }
    };
    let s = units(subject, unit);
    // dp[j]: pattern prefix ps[..j] matches subject prefix s[..i].
    let mut dp = vec![false; ps.len() + 1];
    dp[0] = true;
    for j in 0..ps.len() {
        dp[j + 1] = dp[j] && matches!(ps[j], P::Any);
    }
    for &su in &s {
        let mut next = vec![false; ps.len() + 1];
        for j in 0..ps.len() {
            next[j + 1] = match ps[j] {
                P::Any => next[j] || dp[j + 1],
                P::One => dp[j],
                P::Lit(l) => dp[j] && fold(l) == fold(su),
            };
        }
        dp = next;
    }
    dp[ps.len()]
}

#[test]
fn reference_like_sanity() {
    assert!(like("a_b", r"a\_b", false, Unit::Char));
    assert!(!like("axb", r"a\_b", false, Unit::Char));
    assert!(like("şx", "_x", false, Unit::Char));
    assert!(!like("şx", "_x", false, Unit::Byte));
    assert!(like("ORD1", "ord%", true, Unit::Char));
    assert!(like(r"a\b", r"a\\b", false, Unit::Char));
}

// ---- 1. LIKE translation is a superset of the client glob (should pass) -------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20_000))]

    /// Every name the client glob keeps, the server's `LIKE … ESCAPE '\'` returns, for exact and
    /// ASCII-caseless rules, with hostile metacharacters on both sides.
    #[test]
    fn like_translation_never_selects_fewer(
        glob in r"[ab%_\\*?\[\]!^\-şA]{0,8}",
        names in proptest::collection::vec(r"[abAB%_\\\-ş\[\]x]{0,6}", 12),
    ) {
        for case in [GlobCase::Exact, GlobCase::AsciiInsensitive] {
            let Ok(g) = Glob::new(&glob, case) else { continue };
            let lp = g.like_pattern();
            for name in &names {
                if g.is_match(name) {
                    prop_assert!(
                        like(name, &lp, case == GlobCase::AsciiInsensitive, Unit::Char),
                        "glob {:?} ({:?}) keeps {:?} but LIKE {:?} drops it", glob, case, name, lp
                    );
                }
            }
        }
    }

    /// The bind the filter pushes is exactly the glob's LIKE form; the fixed fragment never
    /// changes; nothing of the pattern reaches the fragment.
    #[test]
    fn pushed_bind_is_the_like_form_and_fragment_is_fixed(
        glob in r"[ab%_\\*?\[\]'\x22;$:-]{1,10}",
    ) {
        let text = format!("table:{glob}");
        let Ok(filter) = parse_filter(&text, PG, FilterRole::Introspect) else { return Ok(()) };
        let ctx = IdentCtx { encoding: Some(Arc::from("UTF8")), ..IdentCtx::new(PG) };
        if let Some(p) = filter.pushdown(ObjectKind::Table, PG, &ctx) {
            prop_assert_eq!(p.sql_fragment, "LIKE :name_like ESCAPE :like_escape");
            let g = Glob::new(glob.trim(), GlobCase::Exact).unwrap();
            prop_assert_eq!(bind_text(&p.binds[0]), g.like_pattern());
        }
    }
}

// ---- 2. PostgreSQL SQL_ASCII: `_` is one byte on the server (I1, fixed) --------------------

#[test]
fn pg_single_char_wildcard_on_a_non_utf8_server_never_selects_fewer() {
    for enc in [Some("SQL_ASCII"), Some("LATIN1"), None] {
        let ident = IdentCtx {
            encoding: enc.map(Arc::from),
            ..IdentCtx::new(PG)
        };
        let filter = f("table:?x||table:[şa]y*", PG).with_ident(ident.clone());
        let p = filter
            .pushdown(ObjectKind::Table, PG, &ident)
            .expect("ASCII patterns still push, widened");
        let pats: Vec<String> = p.binds.iter().map(bind_text).collect();
        assert_eq!(pats, ["%x", "%y%"], "encoding {enc:?}");
        for name in ["şx", "şyz"] {
            assert!(
                filter.matches(ObjectKind::Table, &n(name)),
                "client keeps {name}"
            );
            // A SQL_ASCII database stores UTF-8 names as raw bytes and LIKE runs byte-wise
            // (pg_database_encoding_max_length() == 1 → SB_MatchText): `_` eats one byte.
            assert!(
                pats.iter().any(|lp| like(name, lp, false, Unit::Byte)),
                "encoding {enc:?}: client keeps {name:?}, server LIKE {pats:?} drops it"
            );
        }
    }
}

// ---- 3. The filter's own ident vs the ident passed to pushdown (M2, fixed) -----------------

#[test]
fn pushdown_refuses_an_ident_that_disagrees_with_the_filters_own() {
    let lctn = |l: u8| IdentCtx {
        lower_case_table_names: l,
        ..IdentCtx::new(MY)
    };
    let filter = f("table:ORD*", MY).with_ident(lctn(1));
    assert!(filter.matches(ObjectKind::Table, &n("orders")));
    // Under lctn 0 the server compares TABLE_NAME exactly: `ORD%` would drop `orders`, which
    // the client (lctn 1) keeps. The pushdown refuses the disagreeing rule.
    assert!(filter.pushdown(ObjectKind::Table, MY, &lctn(0)).is_none());
    // The same rule pushes (lctn 1 and 2 are both caseless for table names).
    assert!(filter.pushdown(ObjectKind::Table, MY, &lctn(1)).is_some());
    assert!(filter.pushdown(ObjectKind::Table, MY, &lctn(2)).is_some());
}

// ---- 4. Per-kind pushdown vs a multi-kind catalog section (I2, fixed) ----------------------

#[test]
fn per_kind_pushdown_differs_between_kinds_one_section_may_return() {
    let ident = IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(PG)
    };
    let filter = f("table:ord*", PG);
    // A pg_class section returns tables, views, sequences… together. The Table predicate
    // is `ord%`; views have no positive item, so every view passes the client filter.
    assert!(filter.pushdown(ObjectKind::Table, PG, &ident).is_some());
    assert!(filter.pushdown(ObjectKind::View, PG, &ident).is_none());
    assert!(filter.matches(ObjectKind::View, &n("v_sales")));
    // A section returning several kinds asks with all of them: no predicate unless every kind
    // has its own pushable positive item.
    let relations = [
        ObjectKind::Table,
        ObjectKind::Partition,
        ObjectKind::View,
        ObjectKind::MaterializedView,
        ObjectKind::Sequence,
        ObjectKind::ForeignTable,
    ];
    assert!(filter.pushdown_kinds(&relations, PG, &ident).is_none());
    assert!(
        filter
            .pushdown_kinds(&[ObjectKind::Table, ObjectKind::Partition], PG, &ident)
            .is_some()
    );
}

// ---- 5. Untyped items reach namespace kinds (M1, fixed) -------------------------------------

#[test]
fn an_object_filter_never_filters_namespaces() {
    let my0 = IdentCtx::new(MY);
    let filter = f("ord*", MY);
    // `schema:`/`database:` tokens are refused ("namespaces are the scope's"), and an untyped
    // item never applies to those kinds either.
    for k in [ObjectKind::Schema, ObjectKind::Database, ObjectKind::Root] {
        assert!(filter.matches(k, &n("shop")), "{k:?} hidden");
        assert!(filter.pushdown(k, MY, &my0).is_none(), "{k:?} pushed");
        assert!(!filter.excludes(Some(k), Some("shop")), "{k:?} excluded");
    }
    assert!(!filter.matches(ObjectKind::Table, &n("shop")));
}

// ---- 6. `@` under a caseless MySQL (I3, fixed) ----------------------------------------------

#[test]
fn current_database_matches_by_the_servers_rule() {
    // lower_case_table_names = 2 (macOS default): the database was created `shop`; the session
    // was opened with `USE Shop` (or the profile's database field reads `Shop`).
    let ctx = CurrentCtx {
        database: Some(n("Shop")),
        schema: Some(n("Shop")),
        ident: Some(IdentCtx {
            lower_case_table_names: 2,
            ..IdentCtx::new(MY)
        }),
    };
    let infos = [NsInfo {
        key: NsKey::schema([n("shop")]),
        system: false,
    }];
    // A bare name compares caselessly here …
    assert_eq!(
        parse_scope("Shop", MY).unwrap().select(&infos, &ctx).len(),
        1
    );
    // … and so do `@` and the implicit current namespace, selecting the **listed** key.
    let listed = std::collections::BTreeSet::from([NsKey::schema([n("shop")])]);
    assert_eq!(
        parse_scope("@", MY).unwrap().select_explicit(&infos, &ctx),
        listed
    );
    assert_eq!(parse_scope("", MY).unwrap().select(&infos, &ctx), listed);
    // Under lctn 0 the names differ: nothing is selected, and nothing is invented.
    let exact = CurrentCtx {
        ident: Some(IdentCtx::new(MY)),
        ..ctx.clone()
    };
    assert!(
        parse_scope("", MY)
            .unwrap()
            .select(&infos, &exact)
            .is_empty()
    );
    // SQLite: an attach alias typed `MAIN` names the listed `main`.
    let lite = CurrentCtx {
        database: None,
        schema: Some(n("MAIN")),
        ident: None,
    };
    let main = [NsInfo {
        key: NsKey::schema([n("main")]),
        system: false,
    }];
    assert_eq!(
        parse_scope("", LITE).unwrap().select(&main, &lite),
        std::collections::BTreeSet::from([NsKey::schema([n("main")])])
    );
    // PostgreSQL stays exact.
    let pg_ctx = CurrentCtx {
        database: Some(n("db")),
        schema: Some(n("Public")),
        ident: None,
    };
    let pg_infos = [NsInfo {
        key: NsKey::schema([n("db"), n("public")]),
        system: false,
    }];
    assert!(
        parse_scope("", PG)
            .unwrap()
            .select(&pg_infos, &pg_ctx)
            .is_empty()
    );
}

// ---- 7. Grammar robustness (should pass) ----------------------------------------------------

#[test]
fn catastrophic_and_huge_patterns_stay_linear_or_are_refused() {
    let long_a = "a".repeat(200_000);
    let t = Instant::now();
    let filter = f("table:{(a*)*b}||table:{(a|aa)+c}", PG);
    assert!(!filter.matches(ObjectKind::Table, &n(&long_a)));
    eprintln!("nested-star regex over 200k chars: {:?}", t.elapsed());

    let stars = format!("table:{}b", "*a".repeat(300));
    let t = Instant::now();
    match parse_filter(&stars, PG, FilterRole::Introspect) {
        Ok(g) => {
            assert!(!g.matches(ObjectKind::Table, &n(&long_a)));
            eprintln!("300-star glob over 200k chars: {:?}", t.elapsed());
        }
        Err(e) => eprintln!("300-star glob refused: {e:?}"),
    }

    // Deep nesting is refused, never a stack overflow.
    let deep = format!("{}a{}", "(".repeat(50_000), ")".repeat(50_000));
    assert!(parse_filter(&format!("{{{deep}}}"), PG, FilterRole::Introspect).is_err());
    assert!(parse_scope(&format!("{{{deep}}}:@"), PG).is_err());
    let classes = format!("{}a{}", "[".repeat(50_000), "]".repeat(50_000));
    assert!(parse_scope(&format!("{{{classes}}}:@"), PG).is_err());
    let braces = format!("{}{}", "{".repeat(100_000), "}".repeat(100_000));
    let _ = parse_filter(&braces, PG, FilterRole::Introspect);
    let _ = parse_scope(&braces, PG);
    // A counted repetition past the size limit is refused.
    assert!(parse_filter("table:{(a{1000}){1000}}", PG, FilterRole::Introspect).is_err());

    // 100k items parse in linear time; pushdown refuses them (> 200).
    let many: String = (0..100_000)
        .map(|i| format!("t{i}_*"))
        .collect::<Vec<_>>()
        .join("||");
    let t = Instant::now();
    let filter = f(&many, PG);
    eprintln!("100k items parsed in {:?}", t.elapsed());
    let ident = IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(PG)
    };
    assert!(filter.pushdown(ObjectKind::Table, PG, &ident).is_none());
}

#[test]
fn error_columns_are_characters() {
    let e = parse_scope("şğü{", PG).unwrap_err();
    assert_eq!((e.column, e.kind), (4, ScopeErrorKind::UnclosedBrace));
    let e = parse_scope("İ:ı,", PG).unwrap_err();
    assert_eq!(e.column, 5);
    let e = parse_scope("𝔸𝔹:{(}", PG).unwrap_err();
    assert_eq!((e.column, e.kind), (4, ScopeErrorKind::BadRegex));
}

// ---- 8. Scope precedence and system namespaces (facts) --------------------------------------

#[test]
fn scope_precedence_and_system_namespaces() {
    let ns = |db: &str, s: &str, system: bool| NsInfo {
        key: NsKey::schema([n(db), n(s)]),
        system,
    };
    let infos = [
        ns("db", "public", false),
        ns("db", "app", false),
        ns("db", "pg_catalog", true),
        ns("db", "information_schema", true),
        ns("db", "pg_toast", true),
    ];
    let ctx = CurrentCtx {
        database: Some(n("db")),
        schema: Some(n("public")),
        ident: None,
    };
    let sel = |p: &str| {
        parse_scope(p, PG)
            .unwrap()
            .select_explicit(&infos, &ctx)
            .into_iter()
            .filter(|k| k.path.len() == 2)
            .map(|k| k.path[1].to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(sel("@:*"), ["app", "public"]);
    assert_eq!(sel("@:{.*}"), ["app", "public"]);
    // A negative wins whatever the order.
    assert_eq!(sel("!@:public|@:*|@:public"), ["app"]);
    // Named system schemas are selectable; a negative `*` or regex removes them (M4, fixed).
    assert_eq!(sel("@:pg_catalog"), ["pg_catalog"]);
    assert!(sel("@:pg_catalog|!*:*").is_empty());
    assert!(sel("@:pg_catalog|!@:{pg_.*}").is_empty());
    assert_eq!(sel("@:pg_catalog|@:public|!@:{pg_.*}"), ["public"]);
}

// ---- 9. Turkish İ/ı and the ASCII-only fold (should pass) -----------------------------------

#[test]
fn turkish_letters_never_fold_and_mysql_lctn1_pushdown_is_still_a_superset() {
    let my1 = IdentCtx {
        lower_case_table_names: 1,
        ..IdentCtx::new(MY)
    };
    let filter = f("table:{istanbul_.*}||table:IST*", MY).with_ident(my1.clone());
    assert!(filter.matches(ObjectKind::Table, &n("ISTANBUL_1")));
    assert!(!filter.matches(ObjectKind::Table, &n("İSTANBUL_1")));
    assert!(filter.matches(ObjectKind::Table, &n("istanbul_x")));
    assert!(!filter.matches(ObjectKind::Table, &n("ıstanbul_x")));
    // A regex item stops the pushdown: the server never sees it.
    assert!(filter.pushdown(ObjectKind::Table, MY, &my1).is_none());
    let filter = f("table:IST*", MY).with_ident(my1.clone());
    let p = filter.pushdown(ObjectKind::Table, MY, &my1).unwrap();
    let lp = bind_text(&p.binds[0]);
    // Under lctn 1 the client keeps `istanbul`; a caseless server LIKE returns it.
    assert!(filter.matches(ObjectKind::Table, &n("istanbul")));
    assert!(like("istanbul", &lp, true, Unit::Char));
    // Names are stored lower-case under lctn 1, so the pattern is sent lower-case (M7): even a
    // binary-collation comparison returns them.
    assert_eq!(lp, "ist%");
    assert!(like("istanbul", &lp, false, Unit::Char));
    // lctn 2 keeps the case as typed (names are stored as created).
    let my2 = IdentCtx {
        lower_case_table_names: 2,
        ..IdentCtx::new(MY)
    };
    let p = f("table:IST*", MY)
        .with_ident(my2.clone())
        .pushdown(ObjectKind::Table, MY, &my2)
        .unwrap();
    assert_eq!(bind_text(&p.binds[0]), "IST%");
}

// ---- 10. SQLite's LIKE pattern limit (M3, fixed) --------------------------------------------

#[test]
fn sqlite_never_gets_a_pattern_past_its_like_limit() {
    // SQLITE_MAX_LIKE_PATTERN_LENGTH defaults to 50,000 bytes; past it the whole query fails
    // ("LIKE or GLOB pattern too complex"). Every dialect caps the pushed pattern.
    let filter = f(&format!("table:{}*", "a".repeat(60_000)), LITE);
    assert!(
        filter
            .pushdown(ObjectKind::Table, LITE, &IdentCtx::new(LITE))
            .is_none()
    );
    let cap = hacivat_introspect::filter::MAX_PUSHDOWN_PATTERN_BYTES;
    let at = f(&format!("table:{}*", "a".repeat(cap - 1)), LITE);
    assert!(
        at.pushdown(ObjectKind::Table, LITE, &IdentCtx::new(LITE))
            .is_some()
    );
    let over = f(&format!("table:{}*", "a".repeat(cap)), LITE);
    assert!(
        over.pushdown(ObjectKind::Table, LITE, &IdentCtx::new(LITE))
            .is_none()
    );
    // The client still filters.
    assert!(over.matches(ObjectKind::Table, &n(&"a".repeat(cap + 3))));
}
