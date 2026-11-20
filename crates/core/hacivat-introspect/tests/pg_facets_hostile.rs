// ## m4-t16b-pg-facets
//! T16b security review: the reviewer's hostile probes (`task-m4-16b-review-hostile.rs`),
//! adopted in the minors round. Pure: no Docker. HR1 (m-1) and HR2 (m-2) failed at review and
//! now assert the fixes; HR3's function rows take a routine key (privileges are checked per
//! object kind since m-2).
// ## m4-t33a-hygiene (named exception): the modules this file uses exist only with the
// `session` feature, so `--no-default-features --tests` builds (T16b/T17 carry).
#![cfg(feature = "session")]

use std::time::Instant;

use hacivat_introspect::pg::decode::row_estimate;
use hacivat_introspect::pg::facets::{
    self, AclItem, AclRow, AclSrc, DepRow, decide_deps, decide_grants,
};
use hacivat_introspect::pg::sections::{Binds, ctx, pack};
use hacivat_meta::{Name, ObjectKey, ObjectKind, ServerId};
use sql_dialect::DialectId;
use sql_ident::quote::{QuoteCtx, quote};

const OWNER: u32 = 10;

fn n(s: &str) -> Name {
    Name::from(s)
}

fn table() -> ObjectKey {
    ObjectKey {
        kind: ObjectKind::Table,
        path: ["db", "s", "t"].into_iter().map(n).collect(),
        server_id: Some(ServerId::Oid(100)),
        overload_sig: None,
        raw_kind: None,
    }
}

fn item(src: AclSrc, grantee: u32, who: Option<&str>, privilege: &str, grantable: bool) -> AclRow {
    AclRow {
        oid: 100,
        src,
        own: "t".into(),
        kind: "r".into(),
        col: None,
        item: Some(AclItem {
            grantee,
            grantee_name: who.map(str::to_string),
            privilege: privilege.into(),
            grantable,
        }),
    }
}

fn obj() -> AclRow {
    AclRow {
        item: None,
        ..item(AclSrc::Obj, 0, None, "", false)
    }
}

/// `acldefault('r', owner)` as rows of `src` (PG 16 set).
fn owner_defaults(src: AclSrc) -> Vec<AclRow> {
    [
        "INSERT",
        "SELECT",
        "UPDATE",
        "DELETE",
        "TRUNCATE",
        "REFERENCES",
        "TRIGGER",
    ]
    .into_iter()
    .map(|p| item(src, OWNER, Some("own"), p, false))
    .collect()
}

fn defaults_plus(extra: Vec<AclRow>) -> Vec<AclRow> {
    let mut rows = vec![obj()];
    rows.extend(owner_defaults(AclSrc::Acl));
    rows.extend(owner_defaults(AclSrc::Default));
    rows.extend(extra);
    rows
}

/// HR1. A grantee whose name the renderer cannot write as that role. PostgreSQL's grammar maps
/// the role spec `public` (bare **or quoted**) to PUBLIC, so a role row named `public` (only
/// reachable by a superuser catalog edit) would be re-granted to everyone. The decider treats
/// only the exact text `PUBLIC` as opaque.
#[test]
fn hr1_a_role_named_lowercase_public_is_opaque() {
    // The renderer path: `ctx.ident(grantee)` keeps `public` bare (not a PG keyword).
    assert_eq!(
        quote(DialectId::Postgres, "public", &QuoteCtx::default()).unwrap(),
        "public",
        "sql_ident leaves `public` bare, so T19 writes `TO public` = PUBLIC"
    );
    let got = decide_grants(
        table(),
        &defaults_plus(vec![item(
            AclSrc::Acl,
            4242,
            Some("public"),
            "SELECT",
            false,
        )]),
    );
    eprintln!("hr1: {got:?}");
    assert!(
        !got.complete(),
        "a role named `public` (oid 4242) is carried as a grantee the renderer turns into PUBLIC"
    );
}

/// HR2. A privilege text outside PostgreSQL's closed set of aclexplode keywords. Only a
/// hostile or broken server can send one; the decider's rule is "upper case and spaces".
#[test]
fn hr2_a_privilege_outside_the_closed_keyword_set_is_opaque() {
    let mut accepted = Vec::new();
    for p in [
        "ALL",
        "ALL PRIVILEGES",
        "SELECT ON ALL TABLES IN SCHEMA PUBLIC TO PUBLIC WITH GRANT OPTION",
    ] {
        let got = decide_grants(
            table(),
            &defaults_plus(vec![item(AclSrc::Acl, 4243, Some("r"), p, false)]),
        );
        if got.complete() {
            accepted.push(p);
        }
    }
    assert!(accepted.is_empty(), "accepted as complete: {accepted:?}");
}

/// HR3. ACL shapes a real server produces that are easy to get wrong.
#[test]
fn hr3_grantor_chains_empty_acls_and_owner_revokes() {
    // A function: owner A, `=X/A` revoked by A, but `=X/B` (B held the grant option) remains.
    // PUBLIC still has EXECUTE, so nothing was revoked; B's grant option is an entry.
    let f = |src, grantee, who: Option<&str>, p: &str, g| AclRow {
        kind: "f".into(),
        ..item(src, grantee, who, p, g)
    };
    let rows = vec![
        obj(),
        f(AclSrc::Acl, OWNER, Some("a"), "EXECUTE", false),
        f(AclSrc::Acl, 77, Some("b"), "EXECUTE", true),
        f(AclSrc::Acl, 0, None, "EXECUTE", false),
        f(AclSrc::Default, 0, None, "EXECUTE", false),
        f(AclSrc::Default, OWNER, Some("a"), "EXECUTE", false),
    ];
    let routine = ObjectKey {
        kind: ObjectKind::Routine,
        ..table()
    };
    let got = decide_grants(routine, &rows);
    assert!(got.complete(), "{got:?}");
    assert_eq!(got.facet.entries.len(), 1, "{got:?}");
    assert!(got.facet.entries[0].grantable && &*got.facet.entries[0].grantee == "b");

    // `REVOKE ALL ON t FROM owner` leaves relacl = '{}': aclexplode returns no item.
    let mut rows = vec![obj()];
    rows.extend(owner_defaults(AclSrc::Default));
    let got = decide_grants(table(), &rows);
    assert!(
        !got.complete(),
        "an empty ACL revokes every default: {got:?}"
    );
    assert_eq!(got.revoked.len(), 7);

    // A grantee oid with no role row, only in a column ACL.
    let mut c = item(AclSrc::Column, 999, None, "SELECT", false);
    c.col = Some((1, "a".into()));
    let got = decide_grants(table(), &defaults_plus(vec![c]));
    assert!(got.opaque && !got.complete(), "{got:?}");

    // The owner kept every default but someone gave the owner a grant option on top: the
    // default is kept and the extra option is an entry, never a revocation.
    let got = decide_grants(
        table(),
        &defaults_plus(vec![item(AclSrc::Acl, OWNER, Some("own"), "SELECT", true)]),
    );
    assert!(got.complete(), "{got:?}");
}

fn dep(oid: u32, refcat: &str, refid: u32, name: &str, refowner: u32) -> DepRow {
    DepRow {
        oid,
        cls: "rel".into(),
        own: "t".into(),
        kind: "r".into(),
        selfrel: oid,
        refcat: refcat.into(),
        refid,
        refsub: 0,
        refkind: Some(if refcat == "proc" { "f" } else { "r" }.into()),
        refschema: Some("s".into()),
        refname: Some(name.into()),
        refparent: Some("t".into()),
        refowner,
    }
}

/// HR4. A dependency "bomb": a huge, cyclic, duplicated row set (self references, references
/// to the object's own parts, the same reference many times, a cycle back to the depender).
/// The decider is one pass with a hash set: linear, deduplicated, and never follows a cycle.
#[test]
fn hr4_dependency_bomb_is_linear_and_deduplicated() {
    const OBJ: u32 = 100;
    let mut rows = Vec::new();
    for i in 0..50_000u32 {
        // three copies of each distinct routine reference
        for _ in 0..3 {
            rows.push(dep(OBJ, "proc", 10_000 + i, &format!("f{i}\"/,= "), 0));
        }
        // a self reference and a reference to an own column, which must be dropped
        rows.push(dep(OBJ, "class", OBJ, "t", OBJ));
        let mut c = dep(OBJ, "column", OBJ, "a", OBJ);
        c.refsub = 1;
        rows.push(c);
        rows.push(dep(OBJ, "none", 0, "", 0));
    }
    let t0 = Instant::now();
    let got = decide_deps(&n("db"), OBJ, &rows).expect("decided");
    eprintln!(
        "hr4: {} rows -> {} refs in {:?}",
        rows.len(),
        got.depends_on.len(),
        t0.elapsed()
    );
    assert_eq!(got.depends_on.len(), 50_000);
    assert!(
        got.depends_on
            .iter()
            .all(|r| r.kind != ObjectKind::Table && r.kind != ObjectKind::Column)
    );
    // Sorted and unique.
    assert!(got.depends_on.windows(2).all(|w| w[0] != w[1]));
}

/// HR5. `reltuples::text` in every float4 output form PostgreSQL 13–18 prints, through the
/// same `str::parse::<f64>` the section decoder uses: -1 (never analyzed, 14+) is no estimate,
/// exponents parse, non-finite values are no estimate, huge values saturate.
#[test]
fn hr5_row_estimate_handles_every_float4_text() {
    let est = |s: &str| row_estimate(s.parse::<f64>().expect(s));
    assert_eq!(est("-1"), None);
    assert_eq!(est("0"), Some(0));
    assert_eq!(est("-0"), Some(0));
    assert_eq!(est("1000"), Some(1000));
    assert_eq!(est("1e+06"), Some(1_000_000));
    assert_eq!(est("1.2345679e+09"), Some(1_234_567_900));
    assert_eq!(est("NaN"), None);
    assert_eq!(est("Infinity"), None);
    assert_eq!(est("-Infinity"), None);
    assert_eq!(est("3.4028235e+38"), Some(u64::MAX));
}

fn strip_comments(sql: &str) -> String {
    sql.lines()
        .map(|l| l.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// HR6. The facet text, every variant with hostile binds: no secret-bearing catalog, no
/// writing or locking construct, every function call schema-qualified (or SQL syntax), every
/// bind cast. Also reports the unqualified type casts (harmless only under the pinned path).
#[test]
fn hr6_facet_text_reads_no_secret_and_calls_only_qualified_functions() {
    let p = pack().unwrap();
    let hostile = Binds {
        ns: Some(1),
        w: Some(2),
        schema: Some(n("x'); SELECT pg_authid; --")),
        oids: Some(vec![3, 4]),
    };
    let call = regex::Regex::new(r#"(?i)([A-Za-z_][A-Za-z0-9_.]*)\s*\("#).unwrap();
    let cast = regex::Regex::new(r"::\s*([A-Za-z_][A-Za-z0-9_.]*)").unwrap();
    const SYNTAX: &[&str] = &[
        "in", "coalesce", "lateral", "and", "or", "on", "as", "from", "select", "where", "join",
        "not", "exists", "when", "then", "else", "case", "all", "union",
    ];
    let mut unqualified_casts = std::collections::BTreeSet::new();
    for section in facets::name::ALL {
        for v in p.reachable_variants(section) {
            let c = ctx(v.version, v.mode, v.level, &hostile);
            for q in p.render(section, &c).unwrap() {
                let sql = strip_comments(q.sql());
                let low = sql.to_ascii_lowercase();
                for bad in [
                    "pg_authid",
                    "rolpassword",
                    "pg_shadow",
                    "pg_user_mapping",
                    "umoptions",
                    "srvoptions",
                    "pg_largeobject",
                    // Minors round (m-3): `pg_statistic_ext` (the definitions of extended
                    // statistics) is a part the Dependencies section reads; the probe keeps
                    // refusing the statistics views and the sampled values.
                    "pg_stat_",
                    "pg_statistic_ext_data",
                    "for update",
                    "for share",
                    "nextval",
                    "setval",
                    "advisory",
                    "lo_",
                    "dblink",
                    "set_config",
                    "pg_sleep",
                ] {
                    assert!(!low.contains(bad), "{section}: `{bad}` in the text");
                }
                assert!(
                    !low.replace("pg_statistic_ext", "").contains("pg_statistic"),
                    "{section}: pg_statistic holds sampled values"
                );
                for m in call.captures_iter(&sql) {
                    let f = m[1].to_ascii_lowercase();
                    assert!(
                        f.starts_with("pg_catalog.") || SYNTAX.contains(&f.as_str()),
                        "{section}: unqualified call `{f}(`"
                    );
                }
                for m in cast.captures_iter(&sql) {
                    let t = m[1].to_ascii_lowercase();
                    if !t.starts_with("pg_catalog.") {
                        unqualified_casts.insert(t);
                    }
                }
            }
        }
    }
    eprintln!("hr6: unqualified casts (pinned path only): {unqualified_casts:?}");
}
