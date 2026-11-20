// ## m4-t16-pg (fix round 1: the T16 security review's hostile probes, committed)
//! T16 security review: hostile probes the implementer did not write.
//!
//! H1–H3 attack the trusted-text and read-only properties. H4–H6 failed at review (findings
//! I-1, I-2, I-4) and now assert the fixes. Fix round 1 also adds `has_database_privilege`
//! (the CONNECT filter of `ListDatabases`, review m-4) to H2's pure reads and `OPERATOR` (the
//! qualified operator syntax of the joinable sections, I-4) to its syntax words.

use hacivat_introspect::pack::Pack;
use hacivat_introspect::pg::sections::{Binds, ctx, name, pack};
use hacivat_introspect::pg::watermark::{FullReason, Now, Plan, SPAN_LIMIT, ServerIdentity, plan};

fn all_variants(p: &Pack, section: &str, b: &Binds) -> Vec<String> {
    let mut out = Vec::new();
    for v in p.reachable_variants(section) {
        let c = ctx(v.version, v.mode, v.level, b);
        for q in p.render(section, &c).unwrap() {
            out.push(q.sql().to_string());
        }
    }
    out
}

fn strip_comments(sql: &str) -> String {
    sql.lines()
        .map(|l| l.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// H1: the rendered text never depends on a bind's value: hostile schema names, oids and
/// watermarks give byte-identical SQL (same list length).
#[test]
fn h1_bind_values_never_change_the_text() {
    let p = pack().unwrap();
    let benign = Binds {
        ns: Some(1),
        w: Some(3),
        schema: Some("a".into()),
        oids: Some(vec![1, 2, 3]),
    };
    let evil = Binds {
        ns: Some(u32::MAX),
        w: Some(u32::MAX),
        schema: Some("x'); DROP SCHEMA public CASCADE; -- $1 :ns :[oids] #{ident:x} \"q\"".into()),
        oids: Some(vec![4_294_967_295, 0, 16_383]),
    };
    for s in p.sections() {
        assert_eq!(
            all_variants(&p, s, &benign),
            all_variants(&p, s, &evil),
            "{s}: a bind value changed the text"
        );
    }
}

/// H2: every function any PostgreSQL section calls is a known side-effect-free read
/// (no XID, no lock beyond AccessShare, no write, no config change), and the plan's canary
/// function really is refused by Task 5's guard (the premise of ruling R-W3-6).
#[test]
fn h2_every_called_function_is_a_known_pure_read() {
    const PURE: &[&str] = &[
        "age",
        "current_setting",
        "pg_is_in_recovery",
        "pg_postmaster_start_time",
        "has_function_privilege",
        "pg_control_system",
        "pg_control_checkpoint",
        "pg_snapshot_xmin",
        "pg_snapshot_xmax",
        "pg_current_snapshot",
        "pg_encoding_to_char",
        "current_database",
        "pg_get_userbyid",
        "pg_get_expr",
        "pg_get_partkeydef",
        "pg_get_viewdef",
        "format_type",
        "generate_series",
        "pg_get_indexdef",
        "quote_ident",
        "unnest",
        "pg_get_constraintdef",
        "pg_get_triggerdef",
        "pg_get_function_identity_arguments",
        "pg_get_function_result",
        "pg_get_functiondef",
        "pg_get_function_arguments",
        "pg_backend_pid",
        "count",
        "has_database_privilege",
    ];
    const SYNTAX: &[&str] = &[
        "in",
        "exists",
        "array",
        "coalesce",
        "nullif",
        "cast",
        "extract",
        "as",
        "and",
        "or",
        "not",
        "on",
        "from",
        "select",
        "where",
        "join",
        "u",
        "values",
        "materialized",
        "then",
        "end",
        "else",
        "union",
        "all",
        "distinct",
        "by",
        "operator",
    ];
    let p = pack().unwrap();
    let b = Binds {
        ns: Some(1),
        w: Some(3),
        schema: Some("a".into()),
        oids: Some(vec![1]),
    };
    let mut unknown = Vec::new();
    for s in p.sections() {
        for sql in all_variants(&p, s, &b) {
            let sql = strip_comments(&sql);
            let bytes = sql.as_bytes();
            for (i, _) in sql.match_indices('(') {
                let mut j = i;
                while j > 0 && bytes[j - 1] == b' ' {
                    j -= 1;
                }
                let end = j;
                while j > 0 && (bytes[j - 1].is_ascii_alphanumeric() || bytes[j - 1] == b'_') {
                    j -= 1;
                }
                let word = sql[j..end].to_ascii_lowercase();
                if word.is_empty() || SYNTAX.contains(&word.as_str()) {
                    continue;
                }
                // `x.f(` is a qualified call; a bare word followed by `(` is a call too.
                let qualified = j >= 11 && &sql[j - 11..j] == "pg_catalog.";
                if !PURE.contains(&word.as_str()) {
                    unknown.push(format!("{s}: {word}"));
                }
                if !qualified && word != "count" && !SYNTAX.contains(&word.as_str()) {
                    // Table aliases with a column list (`AS u(n, p)`) look like calls.
                    if !(word == "u" || word == "k") {
                        unknown.push(format!("{s}: unqualified {word}"));
                    }
                }
            }
        }
    }
    unknown.sort();
    unknown.dedup();
    assert!(unknown.is_empty(), "{unknown:#?}");
    // R-W3-6's premise: the plan's canary would have been refused.
    let plan_canary = "SELECT pg_catalog.pg_current_xact_id_if_assigned() IS NULL";
    assert!(
        hacivat_session::meta::check_own_text(plan_canary, hacivat_proto::DbmsKind::Postgres)
            .is_err()
    );
    // The replacement reads only this backend's row.
    let canary = all_variants(&p, name::CANARY, &Binds::default()).join("\n");
    assert!(canary.contains("a.backend_xid IS NULL"));
    assert!(canary.contains("a.pid = pg_catalog.pg_backend_pid()"));
}

/// H3: adversarial watermark plans: whatever the 64-bit numbers, an incremental plan only
/// happens inside age()'s positive range, from the previous watermark's low word.
#[test]
fn h3_no_incremental_outside_the_safe_span() {
    let id = ServerIdentity {
        system_identifier: Some("1".into()),
        timeline: None,
        database: 5,
        version: 140_000,
        start_micros: 7,
    };
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    for _ in 0..200_000 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let w = seed % (1u64 << 40);
        let d = (seed >> 20) % (1u64 << 33);
        let xmax = if seed & 1 == 0 {
            w + d
        } else {
            w.saturating_sub(d)
        };
        let now = Now {
            identity: id.clone(),
            in_recovery: false,
            snap_xmin: xmax,
            snap_xmax: xmax,
        };
        match plan(&id.watermark(w), &now) {
            Plan::Incremental { since } => {
                assert!(xmax >= w && xmax - w < SPAN_LIMIT, "w={w} xmax={xmax}");
                assert_eq!(u64::from(since), w & 0xFFFF_FFFF);
            }
            Plan::Full(r) => assert!(
                matches!(r, FullReason::Wraparound | FullReason::Identity),
                "{r:?}"
            ),
        }
    }
}

/// H4 (finding I-1, fixed): `ALTER SCHEMA other RENAME` changes only a pg_namespace
/// row. Triggers (pg_trigger -> pg_proc), column defaults (pg_attrdef -> pg_proc/pg_class),
/// checks and FKs (pg_constraint -> pg_proc/pg_class) and views (pg_rewrite -> pg_class) of the
/// namespace that name objects of `other` are recorded in pg_depend with those depender
/// classes; the two-hop branch must see them, and relations of `other` as referenced objects.
#[test]
fn h4_schema_rename_two_hop_covers_every_depender_class() {
    let p = pack().unwrap();
    let b = Binds {
        ns: Some(1),
        w: Some(3),
        ..Binds::default()
    };
    let sql = all_variants(&p, name::CHANGED_DEPENDENTS, &b).join("\n");
    let branch = &sql[sql.find("'schema-rename'").expect("branch")..];
    let mut missing = Vec::new();
    for cls in ["pg_trigger", "pg_attrdef", "pg_constraint", "pg_rewrite"] {
        if !branch.contains(&format!("pg_catalog.{cls}")) {
            missing.push(format!("depender {cls}"));
        }
    }
    if !branch.contains("relnamespace = n.oid") {
        missing.push("referenced relations of the renamed schema".into());
    }
    assert!(missing.is_empty(), "schema rename misses: {missing:?}");
}

/// H5 (finding I-2, fixed): a column rename updates pg_attribute only, never the
/// referenced relation's pg_class row; a type rename touches pg_type only. A view, FK, check
/// or default stamp must therefore read the referenced column's (and type's) row, or
/// `check_state` never sees the renamed dependency (R-M4-48).
#[test]
fn h5_state_stamps_follow_column_and_type_dependencies() {
    let p = pack().unwrap();
    let b = Binds::default().with_oids(vec![1]);
    let mut missing = Vec::new();
    for s in [name::STATE_OF_RELATION, name::STATE_OF_CONSTRAINT] {
        let sql = all_variants(&p, s, &b).join("\n");
        if !sql.contains("refobjsubid") {
            missing.push(format!("{s}: no pg_attribute row for a column dependency"));
        }
    }
    let rel = all_variants(&p, name::STATE_OF_RELATION, &b).join("\n");
    for tag in ["'rdep'", "'kdep'", "'ddep'"] {
        let at = rel.find(tag).unwrap();
        let next = rel[at + 1..]
            .find("UNION ALL")
            .map_or(rel.len(), |e| at + 1 + e);
        if !rel[at..next].contains("pg_catalog.pg_type") {
            missing.push(format!("StateOfRelation {tag}: no pg_type row"));
        }
    }
    assert!(missing.is_empty(), "{missing:#?}");
}

/// H6 (finding I-4, fixed): joinable sections run on a joined session whose
/// search_path Hacivat cannot pin (R-M4-46). A comparison whose operand types have no exact
/// pg_catalog operator (`oid = regclass`, `oid <> integer`) or an unqualified type name lets an
/// exact-match operator/type in a user-writable schema on that path win (CVE-2018-1058 class).
#[test]
fn h6_joinable_text_does_not_resolve_operators_or_types_through_search_path() {
    let p = pack().unwrap();
    let b = Binds::default().with_oids(vec![1]);
    let mut bad = Vec::new();
    for s in p.sections() {
        if !p.section(s).unwrap().joinable() {
            continue;
        }
        let sql = all_variants(&p, s, &b).join("\n");
        if sql.contains(" AS text)") {
            bad.push(format!("{s}: unqualified type text"));
        }
        if sql.contains("<> 0") {
            bad.push(format!(
                "{s}: oid <> integer (no exact pg_catalog operator)"
            ));
        }
        if sql.contains("= CAST('pg_catalog.") {
            bad.push(format!(
                "{s}: oid = regclass (no exact pg_catalog operator)"
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
