// ## m4-t16-pg
//! PostgreSQL introspector, pure tests (M4 Task 16): the watermark decision (R-M4-8, R-M4-9),
//! the exact change predicate (finding 6), the facet cascade (finding 5), the mass-change
//! rule, and properties of every rendered `packs/pg` section (no XID-assigning call, the
//! lint, every catalog `pg_catalog.`-qualified, names only as binds).

use hacivat_introspect::pack::{Pack, lint};
use hacivat_introspect::pg::changes::{Changes, Facets};
use hacivat_introspect::pg::sections::{Binds, ctx, name, pack};
use hacivat_introspect::pg::watermark::{
    FullReason, MASS_CHANGE_MIN_RELATIONS, Now, Plan, SPAN_LIMIT, ServerIdentity, is_mass_change,
    plan,
};
use hacivat_meta::{Level, Watermark};
use sql_dialect::DialectId;

fn identity() -> ServerIdentity {
    ServerIdentity {
        system_identifier: Some("7300000000000000001".into()),
        timeline: Some(1),
        database: 16_384,
        version: 170_005,
        start_micros: 1_763_300_000_123_456,
    }
}

fn now_at(xmax: u64) -> Now {
    Now {
        identity: identity(),
        in_recovery: false,
        snap_xmin: xmax,
        snap_xmax: xmax,
    }
}

fn wm(w: u64) -> Watermark {
    identity().watermark(w)
}

#[test]
fn span_check_is_64_bit_and_forces_full_at_2_31_minus_2_20() {
    // `pg_resetwal -x 2^31+2000` after a pass at W = 738 (ver/01 #pg-xmin-freeze).
    assert_eq!(
        plan(&wm(738), &now_at((1u64 << 31) + 2000)),
        Plan::Full(FullReason::Wraparound)
    );
    assert_eq!(SPAN_LIMIT, (1u64 << 31) - (1u64 << 20));
    assert_eq!(
        plan(&wm(738), &now_at(738 + SPAN_LIMIT - 1)),
        Plan::Incremental { since: 738 }
    );
    assert_eq!(
        plan(&wm(738), &now_at(738 + SPAN_LIMIT)),
        Plan::Full(FullReason::Wraparound)
    );
    // 64-bit: an epoch-extended W passes only its low 32 bits to age(), and the span is taken
    // on the full 64 bits (epoch 3, not wrapped).
    let w = (3u64 << 32) + 77;
    assert_eq!(
        plan(&wm(w), &now_at(w + 13)),
        Plan::Incremental { since: 77 }
    );
    assert_eq!(
        plan(&wm(w), &now_at(w + SPAN_LIMIT)),
        Plan::Full(FullReason::Wraparound)
    );
}

#[test]
fn watermark_ahead_of_next_xid_forces_full() {
    // A restore or `pg_resetwal -x <lower>`: the server's next XID is below the watermark.
    assert_eq!(
        plan(&wm(5_000), &now_at(4_000)),
        Plan::Full(FullReason::Identity)
    );
    assert_eq!(
        plan(&wm(5_000), &now_at(5_000)),
        Plan::Incremental { since: 5_000 }
    );
}

/// Every reachable variant of `section`, rendered with every bind a PostgreSQL section uses.
fn rendered(p: &Pack, section: &str) -> Vec<String> {
    let binds = Binds {
        ns: Some(2200),
        w: Some(1234),
        schema: Some("public".into()),
        oids: Some(vec![16_400, 16_401]),
    };
    let mut out = Vec::new();
    for v in p.reachable_variants(section) {
        let c = ctx(v.version, v.mode, v.level, &binds);
        for q in p
            .render(section, &c)
            .unwrap_or_else(|e| panic!("{section}: {e:?}"))
        {
            out.push(q.sql().to_string());
        }
    }
    assert!(!out.is_empty(), "{section} renders");
    out
}

#[test]
fn change_predicate_is_between_zero_and_age_of_w() {
    let p = pack().expect("the PostgreSQL pack");
    for section in [name::CHANGED_PARENTS, name::CHANGED_DEPENDENTS] {
        for sql in rendered(&p, section) {
            assert!(
                sql.contains("BETWEEN 0 AND pg_catalog.age($"),
                "{section}: the exact R-M4-8 shape with W as a bind"
            );
            // No ordering comparison on an age anywhere: every `age(` is half of the BETWEEN.
            let ages = sql.matches("pg_catalog.age(").count();
            let betweens = sql.matches("BETWEEN 0 AND pg_catalog.age(").count();
            assert_eq!(ages, 2 * betweens, "{section}");
            assert!(
                !sql.contains("<= pg_catalog.age") && !sql.contains("age(x"),
                "{section}"
            );
        }
    }
    // The client computes no `txage` (finding 6): only the 64-bit span check.
    let src = include_str!("../src/pg/watermark.rs");
    assert!(!src.to_ascii_lowercase().contains("txage ="));
    let pass = include_str!("../src/pg/pass.rs");
    assert!(!pass.to_ascii_lowercase().contains("txage"));
}

#[test]
fn identity_or_version_change_forces_full() {
    let prev = wm(1_000);
    let mut n = now_at(1_010);
    n.identity.start_micros += 1;
    assert_eq!(plan(&prev, &n), Plan::Full(FullReason::Restart));
    let mut n = now_at(1_010);
    n.identity.system_identifier = Some("7300000000000000002".into());
    assert_eq!(plan(&prev, &n), Plan::Full(FullReason::Identity));
    let mut n = now_at(1_010);
    n.identity.timeline = Some(2);
    assert_eq!(plan(&prev, &n), Plan::Full(FullReason::Identity));
    let mut n = now_at(1_010);
    n.identity.database = 1;
    assert_eq!(plan(&prev, &n), Plan::Full(FullReason::Identity));
    let mut n = now_at(1_010);
    n.identity.version = 180_001;
    assert_eq!(plan(&prev, &n), Plan::Full(FullReason::Version));
    // A minor upgrade alone keeps the identity (it restarts the server, which the start time
    // catches).
    let mut n = now_at(1_010);
    n.identity.version = 170_006;
    assert_eq!(plan(&prev, &n), Plan::Incremental { since: 1_000 });
    // No watermark, or another engine's.
    assert_eq!(
        plan(&Watermark::None, &now_at(1)),
        Plan::Full(FullReason::NoWatermark)
    );
    assert_eq!(FullReason::NoWatermark.code(), "no-watermark");
    assert_eq!(FullReason::SchemaRename.code(), "schema-rename");
    assert_eq!(FullReason::MassChange.code(), "mass-change");
}

#[test]
fn recovery_forces_full_with_reason_standby() {
    let mut n = now_at(1_010);
    n.in_recovery = true;
    assert_eq!(plan(&wm(1_000), &n), Plan::Full(FullReason::Standby));
    assert_eq!(FullReason::Standby.code(), "standby");
}

#[test]
fn negative_age_rows_are_not_changed() {
    // A frozen row older than 2^31 XIDs has a negative age and a bootstrap row age INT_MAX:
    // the lower bound 0 and the upper bound age(W) leave both out. The rendered predicate is
    // exactly that closed range, with no other bound and no NOT.
    let p = pack().unwrap();
    for section in [name::CHANGED_PARENTS, name::CHANGED_DEPENDENTS] {
        for sql in rendered(&p, section) {
            assert!(!sql.contains("NOT BETWEEN"), "{section}");
            assert!(!sql.contains("BETWEEN -"), "{section}");
            assert!(!sql.contains("SYMMETRIC"), "{section}");
            for (i, _) in sql.match_indices("BETWEEN ") {
                assert!(
                    sql[i..].starts_with("BETWEEN 0 AND pg_catalog.age($"),
                    "{section}"
                );
            }
        }
    }
}

#[test]
fn columns_change_sets_rel_and_rel_sets_indexes_constraints_triggers() {
    let mut f = Facets {
        columns: true,
        ..Facets::default()
    };
    f.cascade();
    assert!(f.rel && f.indexes && f.constraints && f.triggers && f.columns);
    let mut f = Facets {
        rel: true,
        ..Facets::default()
    };
    f.cascade();
    assert!(f.indexes && f.constraints && f.triggers && !f.columns && !f.comments);
    // An index alone re-reads keys only, never the relation row or its columns.
    let mut f = Facets {
        indexes: true,
        ..Facets::default()
    };
    f.cascade();
    assert!(f.keys() && !f.rel && !f.columns && !f.triggers);
    // A comment alone is patched (the Comments section), nothing else.
    let mut f = Facets {
        comments: true,
        ..Facets::default()
    };
    f.cascade();
    assert!(f.comments_only() && !f.rel && !f.keys());
    // The pass's change set applies the cascade per relation.
    let mut c = Changes::default();
    c.add("columns", 10).unwrap();
    c.add("triggers", 11).unwrap();
    c.settle(&[10, 11].into_iter().collect());
    assert!(c.relations[&10].rel && c.relations[&10].constraints);
    assert!(!c.relations[&11].rel && c.relations[&11].triggers);
}

#[test]
fn changed_parents_over_25_percent_runs_full_with_reason_mass_change() {
    assert!(is_mass_change(26, 100));
    assert!(!is_mass_change(25, 100));
    assert!(is_mass_change(6, 20));
    assert!(!is_mass_change(5, 20));
    // Below the minimum, a few re-reads beat a full pass (and keep the narrow reads).
    assert!(!is_mass_change(1, 1));
    assert!(!is_mass_change(
        MASS_CHANGE_MIN_RELATIONS - 1,
        MASS_CHANGE_MIN_RELATIONS - 1
    ));
    assert!(is_mass_change(3_751, 15_000));
    assert_eq!(FullReason::MassChange.code(), "mass-change");
}

#[test]
fn no_query_assigns_an_xid() {
    let p = pack().unwrap();
    for section in p.sections() {
        for sql in rendered(&p, section) {
            let lower = sql.to_ascii_lowercase();
            assert!(!lower.contains("txid_current("), "{section}");
            assert!(!lower.contains("pg_current_xact_id("), "{section}");
            assert!(
                !lower.contains("nextval") && !lower.contains("set_config"),
                "{section}"
            );
        }
    }
    // The canary reads the backend's own XID (it assigns none) and passes Task 5's token
    // guard, which refuses every `pg_current_xact_id*`/`txid_current*` name.
    let canary = rendered(&p, name::CANARY).join("\n");
    assert!(canary.contains("a.backend_xid IS NULL"));
    for section in p.sections() {
        for sql in rendered(&p, section) {
            assert_eq!(
                hacivat_session::meta::check_own_text(&sql, hacivat_proto::DbmsKind::Postgres),
                Ok(()),
                "{section}: the metadata connection's guard refuses the text"
            );
        }
    }
}

#[test]
fn the_pg_pack_lints_clean_and_qualifies_every_catalog() {
    let p = pack().unwrap();
    assert_eq!(lint(&p, DialectId::Postgres), Vec::new());
    for section in p.sections() {
        for sql in rendered(&p, section) {
            // Comment lines (a section's trailing notes) are not code.
            let sql: String = sql
                .lines()
                .filter(|l| !l.trim_start().starts_with("--"))
                .collect::<Vec<_>>()
                .join("\n");
            // Every `pg_` word is a catalog written `pg_catalog.<name>` (R-M4-46).
            for (i, _) in sql.match_indices("pg_") {
                let before = &sql[..i];
                assert!(
                    before.ends_with("pg_catalog.")
                        || sql[i..].starts_with("pg_catalog")
                        || before.ends_with('\''),
                    "{section}: an unqualified catalog near byte {i}"
                );
            }
            for bad in [" ilike ", "lower(", "upper(", " collate "] {
                assert!(!sql.to_ascii_lowercase().contains(bad), "{section}: {bad}");
            }
        }
    }
}

#[test]
fn names_and_oids_reach_the_server_only_as_binds() {
    let p = pack().unwrap();
    let binds = Binds {
        ns: Some(987_654),
        w: Some(123_457),
        schema: Some("hvm4_Ünïcode \"x\"".into()),
        oids: Some(vec![555_001, 555_002]),
    };
    for section in p.sections() {
        for v in p.reachable_variants(section) {
            let c = ctx(v.version, v.mode, v.level, &binds);
            for q in p.render(section, &c).unwrap() {
                for needle in ["987654", "123457", "hvm4_", "555001", "555002"] {
                    assert!(!q.sql().contains(needle), "{section}: {needle} spliced");
                }
            }
        }
    }
    // An oid list chunks at 1,000 per statement (R-M4-3).
    let many = Binds::ns(1).with_oids((1..=2_500).collect());
    let c = ctx(
        170_000,
        hacivat_introspect::Mode::Incremental,
        Level::Structure,
        &many,
    );
    assert_eq!(p.render(name::RETRIEVE_COLUMNS, &c).unwrap().len(), 3);
    // An empty list sends nothing.
    let none = Binds::ns(1).with_oids(Vec::new());
    let c = ctx(
        170_000,
        hacivat_introspect::Mode::Incremental,
        Level::Structure,
        &none,
    );
    assert!(p.render(name::RETRIEVE_COLUMNS, &c).unwrap().is_empty());
}

#[test]
fn version_gates_pick_the_right_columns() {
    let p = pack().unwrap();
    let b = Binds::ns(1);
    let at = |v: u32, s: &str| {
        p.render(
            s,
            &ctx(v, hacivat_introspect::Mode::Full, Level::Structure, &b),
        )
        .unwrap()[0]
            .sql()
            .to_string()
    };
    assert!(!at(140_010, name::RETRIEVE_INDEXES).contains("indnullsnotdistinct"));
    assert!(at(150_000, name::RETRIEVE_INDEXES).contains("i.indnullsnotdistinct"));
    assert!(!at(170_005, name::RETRIEVE_CONSTRAINTS).contains("conenforced"));
    assert!(at(180_000, name::RETRIEVE_CONSTRAINTS).contains("k.conenforced"));
    // Sources only at the Sources level.
    let src = |l: Level, s: &str| {
        p.render(s, &ctx(170_000, hacivat_introspect::Mode::Full, l, &b))
            .unwrap()[0]
            .sql()
            .to_string()
    };
    assert!(!src(Level::Structure, name::RETRIEVE_RELATIONS).contains("pg_get_viewdef"));
    assert!(src(Level::Sources, name::RETRIEVE_RELATIONS).contains("pg_get_viewdef"));
    assert!(!src(Level::Structure, name::RETRIEVE_ROUTINES).contains("pg_get_functiondef"));
    assert!(src(Level::Sources, name::RETRIEVE_ROUTINES).contains("pg_get_functiondef"));
}

/// Review I-4 (fix round 1): a joinable section runs where Hacivat cannot pin the
/// `search_path` (R-M4-46), so nothing in it may resolve through the path: every operator is
/// `OPERATOR(pg_catalog.op)`, every cast names a `pg_catalog` type (no `::`), every function
/// is `pg_catalog`-qualified. The list bind's `IN` is the one fixed by the pack grammar
/// (oid against oid parameters); a joined read must refuse a session whose path does not
/// start with `pg_catalog` (carried to T15b and M7).
#[test]
fn joinable_sections_resolve_no_operator_cast_or_function_through_the_path() {
    let p = pack().unwrap();
    let literal = regex::Regex::new(r"'[^']*'").unwrap();
    let operator = regex::Regex::new(r"OPERATOR\(pg_catalog\.[^)\s]+\)").unwrap();
    let bare_cast = regex::Regex::new(r"(?i)\bAS\s+(\w+)\s*\)").unwrap();
    let call = regex::Regex::new(r"([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap();
    const SYNTAX: [&str; 13] = [
        "cast", "coalesce", "in", "operator", "exists", "array", "nullif", "from", "join", "on",
        "and", "where", "not",
    ];
    let mut bad = Vec::new();
    for section in p.sections() {
        if !p.section(section).unwrap().joinable() {
            continue;
        }
        for sql in rendered(&p, section) {
            let code: String = sql
                .lines()
                .map(|l| l.split("--").next().unwrap_or(""))
                .collect::<Vec<_>>()
                .join("\n");
            let code = literal.replace_all(&code, "''");
            let code = operator.replace_all(&code, " OPQ ");
            for op in ["=", "<", ">", "!", "|", "+", "*", "/", "::"] {
                if code.contains(op) {
                    bad.push(format!("{section}: a bare operator {op:?}"));
                }
            }
            for c in bare_cast.captures_iter(&code) {
                bad.push(format!("{section}: an unqualified cast to {}", &c[1]));
            }
            for c in call.captures_iter(&code) {
                let word = c[1].to_ascii_lowercase();
                let at = c.get(1).unwrap().start();
                if SYNTAX.contains(&word.as_str()) || code[..at].ends_with("pg_catalog.") {
                    continue;
                }
                bad.push(format!("{section}: an unqualified function {word}"));
            }
        }
    }
    bad.sort();
    bad.dedup();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Review m-6: the canary relies on being the first reader of the backend status in its
/// transaction (the status snapshot is taken once per transaction), so no other section reads
/// a `pg_stat_*` view or function.
#[test]
fn only_the_canary_reads_the_backend_status() {
    let p = pack().unwrap();
    for section in p.sections() {
        if &**section == name::CANARY {
            continue;
        }
        for sql in rendered(&p, section) {
            assert!(!sql.contains("pg_catalog.pg_stat_"), "{section}");
        }
    }
}

#[test]
fn state_sections_are_joinable() {
    let p = pack().unwrap();
    for s in [
        name::STATE_OF_RELATION,
        name::STATE_OF_ROUTINE,
        name::STATE_OF_TYPE,
        name::STATE_OF_CONSTRAINT,
        name::STATE_OF_TRIGGER,
    ] {
        assert!(p.section(s).expect(s).joinable(), "{s}");
        for sql in rendered(&p, s) {
            // A joined read passes the guard of every lexer (its engine is not known there).
            assert_eq!(hacivat_session::meta::check_read_text(&sql), Ok(()), "{s}");
            assert!(
                !sql.contains("format_type") && !sql.contains("pg_get_"),
                "{s}: R-M4-46"
            );
        }
    }
    for s in [name::RETRIEVE_COLUMNS, name::CHANGED_PARENTS] {
        assert!(!p.section(s).unwrap().joinable(), "{s}");
    }
}

/// CI 44640824635 (fix round 2): a parameter the server must type from its context fails
/// to prepare where the context does not decide (`$2 IS NULL`: 42P18). Every scalar bind of
/// every rendered PostgreSQL variant carries a qualified cast (`$n::pg_catalog.oid`, `…text`,
/// `…name`, `…xid`); the only uncast binds are the items of a list bind, `<column> IN ($a,
/// $b)`, which the column types (the pack grammar fixes that form).
#[test]
fn every_scalar_bind_carries_a_qualified_cast() {
    let p = pack().unwrap();
    let list = regex::Regex::new(r"IN \((\$\d+)(, \$\d+)*\)").unwrap();
    let bind = regex::Regex::new(r"\$\d+").unwrap();
    let mut bad = Vec::new();
    for section in p.sections() {
        for sql in rendered(&p, section) {
            let rest = list.replace_all(&sql, "IN (LIST)");
            for m in bind.find_iter(&rest) {
                if !rest[m.end()..].starts_with("::pg_catalog.") {
                    bad.push(format!("{section}: {} is not cast", m.as_str()));
                }
            }
        }
    }
    bad.sort();
    bad.dedup();
    assert!(bad.is_empty(), "{bad:#?}");
}

// ## m4-t16-pg (R-W3-2, R-W3-3, the progress sink)

fn filter(text: &str) -> hacivat_introspect::filter::CompiledFilter {
    hacivat_introspect::filter::parse_filter(
        text,
        DialectId::Postgres,
        hacivat_introspect::filter::FilterRole::Introspect,
    )
    .expect("a filter")
}

#[test]
fn relation_sections_push_the_scope_filter_down_as_like_binds() {
    use hacivat_introspect::pg::pass::relation_runs;
    use hacivat_introspect::pg::sections::ctx_like;
    use hacivat_values::BindValue;
    // Nothing to push: one run, the predicate bound to NULL.
    assert_eq!(relation_runs(None), vec![None]);
    assert_eq!(relation_runs(Some(&filter(""))), vec![None]);
    // A negative item alone is never pushed (the server must return a superset).
    assert_eq!(relation_runs(Some(&filter("-tmp_*"))), vec![None]);
    // A positive glob for every relation kind: one LIKE run per pattern.
    let runs = relation_runs(Some(&filter("orders*||audit_?")));
    assert_eq!(runs.len(), 2);
    let p = pack().unwrap();
    for section in [
        name::LIST_EXISTENT,
        name::RETRIEVE_RELATIONS,
        name::CHANGED_PARENTS,
    ] {
        let b = Binds {
            ns: Some(2200),
            w: Some(9),
            schema: None,
            oids: Some(vec![1]),
        };
        let q = p
            .render(
                section,
                &ctx_like(
                    170_000,
                    hacivat_introspect::Mode::Full,
                    Level::Structure,
                    &b,
                    runs[0].as_ref(),
                ),
            )
            .unwrap();
        assert!(q[0].sql().contains("relname LIKE $"), "{section}");
        assert!(q[0].sql().contains(" ESCAPE $"), "{section}");
        let texts: Vec<&str> = q[0]
            .binds()
            .iter()
            .filter_map(|b| match b {
                BindValue::Typed { text, .. } => Some(&**text),
                BindValue::Null => None,
            })
            .collect();
        assert!(texts.contains(&"orders%"), "{section}: {texts:?}");
        // The pattern is a bind, never text.
        assert!(!q[0].sql().contains("orders"), "{section}");
    }
}

#[test]
fn identity_is_unknown_without_the_system_identifier() {
    use hacivat_introspect::introspector::ServerIdentity;
    use hacivat_introspect::pg::decode::IdentityRow;
    use hacivat_introspect::pg::identity_of;
    let row = |sysid: Option<&str>, version: u32, db: u32| IdentityRow {
        version,
        in_recovery: false,
        start_epoch: "1".into(),
        db_oid: db,
        db_name: "hv_test".into(),
        system_identifier: sysid.map(str::to_string),
        timeline: Some(1),
        snap_xmin: 1,
        snap_xmax: 1,
        ns_oid: None,
        version_text: "17.5".into(),
        encoding: "UTF8".into(),
        standard_conforming_strings: Some(true),
        db_collation: None,
    };
    assert_eq!(identity_of(&row(None, 170_005, 5)), ServerIdentity::Unknown);
    let a = identity_of(&row(Some("7300"), 170_005, 5));
    assert!(matches!(a, ServerIdentity::Known(_)));
    // A minor upgrade keeps the identity; a major one, another database or cluster does not.
    assert_eq!(a, identity_of(&row(Some("7300"), 170_006, 5)));
    assert_ne!(a, identity_of(&row(Some("7300"), 180_000, 5)));
    assert_ne!(a, identity_of(&row(Some("7300"), 170_005, 6)));
    assert_ne!(a, identity_of(&row(Some("7301"), 170_005, 5)));
}

#[test]
fn sections_report_progress_in_the_engines_phases() {
    use hacivat_introspect::Phase;
    use hacivat_introspect::pg::sections::phase_of;
    assert_eq!(phase_of(name::RETRIEVE_RELATIONS), Phase::Tables);
    assert_eq!(phase_of(name::RETRIEVE_COLUMNS), Phase::Columns);
    assert_eq!(phase_of(name::RETRIEVE_INDEXES), Phase::Indexes);
    assert_eq!(phase_of(name::LIST_EXISTENT), Phase::Existence);
    assert_eq!(phase_of(name::CHANGED_PARENTS), Phase::Validate);
    assert_eq!(phase_of(name::STATE_OF_RELATION), Phase::Validate);
    assert_eq!(phase_of(name::LIST_DATABASES), Phase::ListNamespaces);
}
