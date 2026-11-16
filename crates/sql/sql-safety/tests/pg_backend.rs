//! The PostgreSQL backend (M3b Task 9; `alg/13` SQL-SAFE-02 PG, SAFE-03, SAFE-07): the
//! `alg/13` cases, the CVE T12 shapes, the combine rule (parse errors, statement count, token
//! boundaries, newer servers), the destructive patterns and hostile inputs. Expectations come
//! from Hacivat's specs (`alg/13`), never from another product.
//!
//! The backend is reached through the hidden `pg_backend_hooks` (`classify` is Task 14's): the
//! text is split in execution mode, each statement analysed, the reports merged.

use std::sync::Arc;

use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes, ServerVersion};
use sql_ident::QualifiedName;
use sql_safety::effects::class_of;
use sql_safety::pg_backend_hooks::{analyse_pg, classify_pg as classify_with};
use sql_safety::report::merge;
use sql_safety::{
    Actor, ClassifyCtx, CoarseClass, Confidence, DestructiveKind, EffectSet, Expect, FnCatalog,
    FnLookup, GateClass, Profile, ReasonCode, RiskFlags, SafetyReport, StmtKind, StmtReport,
    Verdict, decide, gate_class,
};
use sql_text::{TextRange, TextSize};

fn ctx_at(version: Option<ServerVersion>) -> ClassifyCtx {
    ClassifyCtx::new(
        DialectCtx::new(DialectId::Postgres, version),
        ClientFlavor::Psql,
        Expect::SingleStatement,
    )
}

fn ctx() -> ClassifyCtx {
    ctx_at(Some(ServerVersion::new(17, 2, 0)))
}

fn classify_pg(sql: &str) -> SafetyReport {
    let r = classify_with(sql, &ctx());
    invariants(sql, &r);
    r
}

/// What every report must hold: the class field is `class_of(effects)` (Task 14's debug
/// assertion), and every `Unknown` statement says why with `UNKNOWN` or a doubt reason (the
/// producer contract of Task 4's review).
fn invariants(sql: &str, r: &SafetyReport) {
    for s in &r.items {
        assert_eq!(s.class, class_of(s.effects), "{sql}: {s:?}");
        assert!(!s.effects.is_empty(), "{sql}: {s:?}");
        if s.class == CoarseClass::Unknown || gate_class(s) == GateClass::Unknown {
            let doubt = s.effects.contains(EffectSet::UNKNOWN)
                || s.reasons.iter().any(|x| {
                    matches!(
                        x.code,
                        ReasonCode::ParseError
                            | ReasonCode::ParserDisagreement
                            | ReasonCode::MultiStatement
                            | ReasonCode::SyntaxNewerThanAgent
                            | ReasonCode::FnUnknown
                            | ReasonCode::FnDenylisted
                            | ReasonCode::OperatorUnknown
                            | ReasonCode::CastUnknown
                            | ReasonCode::DynamicSql
                            | ReasonCode::ProceduralCode
                            | ReasonCode::EffectsUnknown
                            | ReasonCode::VolatileFunction
                            | ReasonCode::LexModeChange
                            | ReasonCode::PrivilegeChange
                    )
                });
            assert!(doubt, "{sql}: Unknown without a reason: {s:?}");
        }
    }
}

fn class(sql: &str) -> CoarseClass {
    classify_pg(sql).request_class
}

fn codes(r: &SafetyReport) -> Vec<ReasonCode> {
    r.items
        .iter()
        .flat_map(|s| s.reasons.iter().map(|x| x.code))
        .collect()
}

fn destructive(sql: &str) -> Vec<DestructiveKind> {
    classify_pg(sql)
        .items
        .iter()
        .flat_map(|s| s.destructive.iter().map(|d| d.kind))
        .collect()
}

fn read_only(r: &SafetyReport) -> &'static str {
    match decide(r, Profile::ReadOnly, Actor::Human).verdict {
        Verdict::Allow => "Allow",
        Verdict::Confirm(_) => "Confirm",
        Verdict::Deny(_) => "Deny",
    }
}

/// `alg/13` :213-248 row: the request class, and the decision under `read_only`.
fn row(sql: &str, want: CoarseClass, decision: &str) {
    let r = classify_pg(sql);
    assert_eq!(r.request_class, want, "{sql}: {r:?}");
    assert_eq!(read_only(&r), decision, "{sql}");
}

fn one(sql: &str) -> StmtReport {
    let r = classify_pg(sql);
    assert_eq!(r.items.len(), 1, "{sql}: {r:?}");
    r.items[0].clone()
}

/// The backend on one item covering the whole text, merged.
fn as_one_item(sql: &str) -> SafetyReport {
    let range = TextRange::new(TextSize::from(0), TextSize::of(sql));
    let items = analyse_pg(sql, &[range], &ctx());
    let (request_class, request_effects, flags) = merge(&items);
    let mut r = classify_pg("SELECT 1");
    r.items = items;
    r.request_class = request_class;
    r.request_effects = request_effects;
    r.flags = flags;
    r
}

/// A catalog that answers one thing for every function.
#[derive(Debug)]
struct Catalog(FnLookup);

impl FnCatalog for Catalog {
    fn lookup(&self, _f: &QualifiedName, _arity: Option<u16>) -> FnLookup {
        self.0
    }
}

fn with_catalog(sql: &str, l: FnLookup) -> SafetyReport {
    let mut c = ctx();
    c.catalog = Some(Arc::new(Catalog(l)));
    let r = classify_with(sql, &c);
    invariants(sql, &r);
    r
}

#[test]
fn safe02_cases_1_to_21_and_33_34() {
    use CoarseClass as C;
    // 1
    row("SELECT * FROM t", C::Read, "Allow");
    assert_eq!(one("SELECT * FROM t").touched.reads.len(), 1);
    // 2
    let r = classify_pg("WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d");
    assert_eq!(r.request_class, C::Write);
    assert!(codes(&r).contains(&ReasonCode::DataModifyingCte));
    assert_eq!(read_only(&r), "Deny");
    // 3
    row("SELECT * INTO x FROM t", C::Ddl, "Deny");
    // 4
    row("EXPLAIN SELECT 1", C::Read, "Allow");
    row("EXPLAIN ANALYZE DELETE FROM t", C::Write, "Deny");
    row("EXPLAIN (ANALYZE off) DELETE FROM t", C::Write, "Deny");
    // 5
    row("SELECT pg_terminate_backend(123)", C::Admin, "Deny");
    // 6
    row("SELECT nextval('s')", C::Write, "Deny");
    // 7
    row("SELECT * FROM t FOR UPDATE", C::Write, "Deny");
    // 8
    row("SET TRANSACTION READ WRITE", C::Session, "Deny");
    row(
        "SELECT set_config('transaction_read_only','off',true)",
        C::Admin,
        "Deny",
    );
    // 9: COMMIT alone; two statements the splitter took for one unit are a disagreement and
    // MultiStatement (the orchestrator's split refuses them earlier, Task 14).
    row("COMMIT", C::Session, "Deny");
    for sql in [
        "SELECT 1; DELETE FROM t",
        "COMMIT; DROP SCHEMA public CASCADE;",
    ] {
        let r = as_one_item(sql);
        assert_eq!(r.request_class, C::Unknown, "{sql}");
        assert!(codes(&r).contains(&ReasonCode::ParserDisagreement), "{sql}");
        assert!(codes(&r).contains(&ReasonCode::MultiStatement), "{sql}");
        assert_eq!(read_only(&r), "Deny", "{sql}");
    }
    // 10 (S-12, export only through the export API, is the policy's; here the effects)
    assert_eq!(class("COPY t TO STDOUT"), C::Read);
    row("COPY t TO PROGRAM 'x'", C::Admin, "Deny");
    // 11
    let r = classify_pg("SELECT dblink_exec('host=x', 'DROP TABLE t')");
    assert_eq!(r.request_class, C::Admin);
    assert!(r.flags.contains(RiskFlags::DYNAMIC_SQL));
    assert_eq!(read_only(&r), "Deny");
    // 12
    row(
        "SELECT query_to_xml('select 1', true, true, '')",
        C::Unknown,
        "Deny",
    );
    // 13
    row("DO $$BEGIN END$$", C::Unknown, "Deny");
    row("CALL p()", C::Unknown, "Deny");
    // 14
    row("SELECT 1 ### 2", C::Unknown, "Deny");
    // 15: with a catalog that knows the function as stable and non-writing
    let known = FnLookup::Known {
        volatile: false,
        writes: false,
    };
    let r = with_catalog("SELECT my_stable_sql_fn(x) FROM t", known);
    assert_eq!(r.request_class, C::Read);
    assert_eq!(r.items[0].confidence, Confidence::Conservative);
    assert_eq!(read_only(&r), "Allow");
    // 16: the catalog cannot clear it (`dblink` installed: the catalog answers Unknown)
    let r = with_catalog("SELECT my_stable_sql_fn(x) FROM t", FnLookup::Unknown);
    assert_eq!(r.request_class, C::Unknown);
    assert_eq!(read_only(&r), "Deny");
    // 17
    row(
        "SELECT * FROM pg_read_file('/etc/passwd')",
        C::Admin,
        "Deny",
    );
    row(
        "SELECT x FROM t, LATERAL pg_catalog.pg_ls_dir('.') d",
        C::Admin,
        "Deny",
    );
    // 18
    row("SELECT txid_current()", C::Write, "Deny");
    // 19
    row("LISTEN ch", C::Session, "Deny");
    row(
        "DECLARE c CURSOR WITH HOLD FOR SELECT 1",
        C::Session,
        "Deny",
    );
    // 20
    let r = classify_pg("SELEC 1");
    assert_eq!(r.request_class, C::Unknown);
    assert_eq!(codes(&r), [ReasonCode::ParseError]);
    assert_eq!(read_only(&r), "Deny");
    // 21: U+0130 is not folded; without a catalog the name is unknown
    row("SELECT PG_READ_FİLE('/x')", C::Unknown, "Deny");
    // 33
    row("SELECT \"DELETE\" FROM t", C::Read, "Allow");
    // 34
    let r = classify_pg("SELECT 1;;");
    assert_eq!(r.items.len(), 1);
    assert_eq!(r.request_class, C::Read);
    assert_eq!(read_only(&r), "Allow");
}

#[test]
fn cve_t12_shapes_are_not_read() {
    for s in [
        "SELECT * FROM (SELECT f_write()) s",
        "WITH w AS (SELECT f_write()) SELECT * FROM w",
        "SELECT * FROM t, LATERAL (SELECT f_write()) l",
        "SELECT * FROM t ORDER BY (SELECT f_write())",
    ] {
        assert_ne!(classify_pg(s).request_class, CoarseClass::Read, "{s}");
    }
    // Deeper: a function in a FROM inside a LATERAL inside a CTE, and in a window frame.
    for s in [
        "WITH c AS (SELECT * FROM t, LATERAL (SELECT * FROM (SELECT * FROM f_write()) a) b) SELECT * FROM c",
        "SELECT sum(x) OVER (ORDER BY (SELECT f_write())) FROM t",
        "SELECT * FROM t WHERE x IN (SELECT y FROM u WHERE EXISTS (SELECT f_write()))",
        "SELECT ARRAY(SELECT f_write())",
        "SELECT * FROM t GROUP BY ROLLUP ((SELECT f_write()))",
        "SELECT (SELECT f_write())::text",
        "VALUES ((SELECT f_write()))",
        "SELECT * FROM ROWS FROM (generate_series(1, 2), f_write()) r",
    ] {
        let r = classify_pg(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}");
        assert!(codes(&r).contains(&ReasonCode::FnUnknown), "{s}");
    }
}

#[test]
fn data_modifying_cte_is_write() {
    assert_eq!(
        classify_pg("WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d").request_class,
        CoarseClass::Write
    );
    for s in [
        "WITH u AS (UPDATE t SET a = 1 WHERE b = 2 RETURNING *) SELECT * FROM u",
        "WITH i AS (INSERT INTO t VALUES (1) RETURNING *) SELECT * FROM i",
        "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3), \
         d AS (DELETE FROM t WHERE id IN (SELECT n FROM r) RETURNING *) SELECT * FROM d",
        "WITH m AS (MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN UPDATE SET a = s.a \
         RETURNING t.*) SELECT * FROM m",
        "INSERT INTO a SELECT * FROM (WITH d AS (DELETE FROM t WHERE x = 1 RETURNING *) SELECT * FROM d) s",
    ] {
        let r = classify_pg(s);
        assert_eq!(r.request_class, CoarseClass::Write, "{s}");
        assert!(
            codes(&r).contains(&ReasonCode::DataModifyingCte),
            "{s}: {r:?}"
        );
    }
    // A CTE-wrapped DELETE without WHERE is still unbounded (UNSAFE-05) and the target is a
    // write.
    let r = one("WITH x AS (SELECT 1) DELETE FROM audit_log");
    assert_eq!(r.kind, StmtKind::Delete);
    assert_eq!(r.touched.writes.len(), 1);
    assert!(r.touched.reads.is_empty());
}

#[test]
fn select_into_is_ddl() {
    let r = one("SELECT * INTO x FROM t");
    assert_eq!(r.class, CoarseClass::Ddl);
    assert_eq!(r.kind, StmtKind::CreateTable);
    assert!(r.effects.contains(EffectSet::DDL | EffectSet::WRITE_DATA));
    let temp = one("SELECT * INTO TEMP x FROM t");
    assert_eq!(temp.class, CoarseClass::Ddl);
    assert!(
        temp.effects
            .contains(EffectSet::DDL_TEMP | EffectSet::WRITE_TEMP)
    );
    assert!(!temp.effects.contains(EffectSet::DDL));
    // INTO inside a UNION branch and through CREATE TABLE AS
    assert_eq!(
        class("SELECT a INTO x FROM t UNION SELECT a FROM u"),
        CoarseClass::Ddl
    );
    let ctas = one("CREATE TABLE x AS SELECT * FROM t");
    assert_eq!(
        (ctas.class, ctas.kind),
        (CoarseClass::Ddl, StmtKind::CreateTable)
    );
    assert_eq!(
        class("CREATE MATERIALIZED VIEW mv AS SELECT 1"),
        CoarseClass::Ddl
    );
    assert_eq!(read_only(&classify_pg("SELECT * INTO x FROM t")), "Deny");
}

#[test]
fn explain_analyze_option_spellings() {
    for o in [
        "ANALYZE",
        "ANALYZE true",
        "ANALYZE on",
        "ANALYZE 1",
        "ANALYZE yes",
        "ANALYSE",
        "FORMAT JSON, ANALYZE",
        "analyze TRUE, COSTS off",
    ] {
        let r = classify_pg(&format!("EXPLAIN ({o}) DELETE FROM t"));
        assert!(r.flags.contains(RiskFlags::EXPLAIN_ANALYZE), "{o}");
        assert_eq!(r.items[0].kind, StmtKind::Explain { analyze: true }, "{o}");
        assert_eq!(r.request_class, CoarseClass::Write, "{o}");
    }
    for o in ["ANALYZE false", "ANALYZE off", "ANALYZE 0", "COSTS off"] {
        let r = classify_pg(&format!("EXPLAIN ({o}) DELETE FROM t"));
        assert!(!r.flags.contains(RiskFlags::EXPLAIN_ANALYZE), "{o}");
        assert_eq!(r.items[0].kind, StmtKind::Explain { analyze: false }, "{o}");
        // S-08: the inner statement's effects count without ANALYZE too.
        assert_eq!(r.request_class, CoarseClass::Write, "{o}");
    }
    // The legacy forms
    for s in [
        "EXPLAIN ANALYZE SELECT 1",
        "EXPLAIN ANALYSE VERBOSE SELECT 1",
    ] {
        let r = classify_pg(s);
        assert!(r.flags.contains(RiskFlags::EXPLAIN_ANALYZE), "{s}");
        assert_eq!(r.request_class, CoarseClass::Read, "{s}");
    }
    // A plain EXPLAIN runs nothing: no destructive reason; EXPLAIN ANALYZE runs it.
    assert!(destructive("EXPLAIN DELETE FROM t").is_empty());
    assert_eq!(
        destructive("EXPLAIN ANALYZE DELETE FROM t"),
        [DestructiveKind::NoWhere]
    );
}

#[test]
fn set_config_with_a_non_literal_name_is_unknown() {
    for s in [
        "SELECT set_config(current_user, 'x', false)",
        "SELECT set_config(name, setting, true) FROM pg_settings",
        "SELECT pg_catalog.set_config('a' || 'b', 'x', true)",
    ] {
        let r = classify_pg(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}");
        assert!(r.request_effects.contains(EffectSet::UNKNOWN), "{s}");
    }
    let role = classify_pg("SELECT set_config('role', 'admin', false)");
    assert!(codes(&role).contains(&ReasonCode::PrivilegeChange));
    let lex = classify_pg("SELECT set_config('standard_conforming_strings', 'off', true)");
    assert!(lex.flags.contains(RiskFlags::LEX_MODE_CHANGE));
    let plain = classify_pg("SELECT set_config('work_mem', '64MB', true)");
    assert_eq!(plain.request_class, CoarseClass::Admin);
    assert!(codes(&plain).contains(&ReasonCode::FnDenylisted));
}

#[test]
fn copy_variants() {
    let read = one("COPY t TO STDOUT");
    assert_eq!((read.kind, read.class), (StmtKind::Copy, CoarseClass::Read));
    assert_eq!(read.touched.reads.len(), 1);
    assert_eq!(
        class("COPY (SELECT * FROM t WHERE a > 1) TO STDOUT WITH (FORMAT csv)"),
        CoarseClass::Read
    );
    let write = one("COPY t FROM STDIN");
    assert_eq!(write.class, CoarseClass::Write);
    assert_eq!(write.touched.writes.len(), 1);
    let program = one("COPY t TO PROGRAM 'gzip > /tmp/t.gz'");
    assert!(program.effects.contains(EffectSet::PROCESS_EXEC));
    assert_eq!(program.class, CoarseClass::Admin);
    assert!(
        program
            .reasons
            .iter()
            .any(|r| r.code == ReasonCode::ProcessExec)
    );
    let from_program = one("COPY t FROM PROGRAM 'curl http://x'");
    assert!(
        from_program
            .effects
            .contains(EffectSet::PROCESS_EXEC | EffectSet::WRITE_DATA)
    );
    let file = one("COPY t FROM '/etc/passwd'");
    assert!(
        file.effects
            .contains(EffectSet::EXTERNAL_IO | EffectSet::WRITE_DATA)
    );
    assert_eq!(file.class, CoarseClass::Admin);
    assert_eq!(class("COPY t TO '/tmp/out.csv'"), CoarseClass::Admin);
    // A writing query inside COPY TO STDOUT is a write.
    assert_eq!(
        class("COPY (DELETE FROM t RETURNING *) TO STDOUT"),
        CoarseClass::Write
    );
    assert_eq!(
        class("COPY (SELECT f_write()) TO STDOUT"),
        CoarseClass::Unknown
    );
}

#[test]
fn unsafe_01_to_10() {
    use DestructiveKind as D;
    // 01
    assert_eq!(destructive("DELETE FROM audit_log;"), [D::NoWhere]);
    // 02
    assert_eq!(
        destructive("UPDATE customers SET city='x' WHERE 1=1;"),
        [D::AlwaysTrue]
    );
    // 03
    assert_eq!(
        destructive("UPDATE customers SET city='x' WHERE true;"),
        [D::AlwaysTrue]
    );
    // 04 (the MySQL multi-table form is Task 10's; PostgreSQL's join forms)
    assert_eq!(
        destructive("DELETE FROM orders o USING customers c WHERE c.active;"),
        [D::JoinWithoutOn]
    );
    assert_eq!(
        destructive("UPDATE orders SET x = 1 FROM customers c WHERE c.id = 5;"),
        [D::JoinWithoutOn]
    );
    assert!(destructive("DELETE FROM orders o USING customers c WHERE o.cid = c.id;").is_empty());
    assert!(destructive("DELETE FROM orders USING customers c WHERE cid = c.id;").is_empty());
    // 05
    assert_eq!(
        destructive("WITH x AS (SELECT 1) DELETE FROM audit_log;"),
        [D::NoWhere]
    );
    // 06
    assert_eq!(
        destructive("DELETE FROM audit_log WHERE NULL IS NULL;"),
        [D::AlwaysTrue]
    );
    for w in [
        "'a' = 'a'",
        "NOT FALSE",
        "id = 5 OR 1 = 1",
        "1 < 2 AND TRUE",
        "'t'::boolean",
        "TRUE IS TRUE",
        "2.5 >= 2",
    ] {
        assert_eq!(
            destructive(&format!("DELETE FROM audit_log WHERE {w}")),
            [D::AlwaysTrue],
            "{w}"
        );
    }
    for w in ["id = id", "1 = 2", "NULL = NULL", "id = 5 AND 1 = 1", "x"] {
        assert!(
            destructive(&format!("DELETE FROM audit_log WHERE {w}")).is_empty(),
            "{w}"
        );
    }
    // 07
    let t = one("TRUNCATE audit_log;");
    assert_eq!(t.destructive.len(), 1);
    assert_eq!(t.destructive[0].kind, D::Truncate);
    assert_eq!(
        t.destructive[0]
            .target
            .as_ref()
            .map(|o| o.name.parts[0].text.to_string()),
        Some("audit_log".to_string())
    );
    // 08: no temp-table exemption is configured in M3b (SAFE-07 step 4 "when configured"),
    // so a temp table's unbounded DELETE still warns.
    assert_eq!(destructive("DELETE FROM pg_temp.scratch"), [D::NoWhere]);
    // 09
    assert!(destructive("UPDATE customers SET city = NULL WHERE id = 1;").is_empty());
    // 10: a body that does not run now gives no warning
    assert!(
        destructive(
            "CREATE FUNCTION f() RETURNS void LANGUAGE sql BEGIN ATOMIC DELETE FROM audit_log; END"
        )
        .is_empty()
    );
    assert!(
        destructive("CREATE FUNCTION f() RETURNS void LANGUAGE sql AS $$ DELETE FROM audit_log $$")
            .is_empty()
    );
    assert!(
        destructive("CREATE RULE r AS ON INSERT TO t DO ALSO DELETE FROM audit_log").is_empty()
    );
    assert!(destructive("PREPARE p AS DELETE FROM audit_log").is_empty());
    // The rest of SAFE-07 step 3
    assert_eq!(destructive("DROP TABLE t"), [D::Drop { cascade: false }]);
    assert_eq!(
        destructive("DROP TABLE a, b CASCADE"),
        [D::Drop { cascade: true }; 2]
    );
    assert_eq!(
        destructive("DROP SCHEMA public CASCADE"),
        [D::Drop { cascade: true }]
    );
    assert_eq!(
        destructive("DROP DATABASE prod"),
        [D::Drop { cascade: false }]
    );
    assert_eq!(destructive("ALTER TABLE t DROP COLUMN c"), [D::DropColumn]);
    assert_eq!(
        destructive("ALTER TABLE t DROP CONSTRAINT k"),
        [D::DropConstraint]
    );
    assert_eq!(
        destructive("ALTER TABLE t ALTER COLUMN c TYPE bigint"),
        [D::AlterType]
    );
    assert_eq!(
        destructive("MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN DELETE"),
        [D::MergeDelete]
    );
    let drop = one("DROP TABLE app.orders");
    let target = drop.destructive[0].target.as_ref().unwrap();
    let parts: Vec<&str> = target.name.parts.iter().map(|p| &*p.text).collect();
    assert_eq!(parts, ["app", "orders"]);
    // Gate classes follow (Task 4's floors)
    assert_eq!(gate_class(&one("DELETE FROM t")), GateClass::DmlUnsafe);
    assert_eq!(
        gate_class(&one("DELETE FROM t WHERE id = 1")),
        GateClass::Dml
    );
    assert_eq!(gate_class(&one("DROP TABLE t")), GateClass::DdlDestructive);
}

#[test]
fn token_disagreement_is_parser_disagreement() {
    // PostgreSQL 14 reads `0x1F` as `0 AS x1F`; libpg_query (PostgreSQL 18) as one number.
    let old = classify_with("SELECT 0x1F", &ctx_at(Some(ServerVersion::new(14, 0, 0))));
    assert_eq!(old.request_class, CoarseClass::Unknown);
    assert!(codes(&old).contains(&ReasonCode::ParserDisagreement));
    let new = classify_with("SELECT 0x1F", &ctx_at(Some(ServerVersion::new(17, 0, 0))));
    assert_eq!(new.request_class, CoarseClass::Read);
    let sep = classify_with("SELECT 1_000", &ctx_at(Some(ServerVersion::new(15, 0, 0))));
    assert_eq!(sep.request_class, CoarseClass::Unknown);

    // standard_conforming_strings off: the server reads backslashes; libpg_query does not.
    let mut off = ctx();
    off.dialect.modes = LexModes {
        standard_conforming_strings: false,
        ..LexModes::default_for(DialectId::Postgres)
    };
    for s in ["SELECT 'a\\' , 1 -- '", "SELECT 'C:\\temp'"] {
        let r = classify_with(s, &off);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}");
        assert!(codes(&r).contains(&ReasonCode::ParserDisagreement), "{s}");
    }
    assert_eq!(
        classify_with("SELECT E'C:\\\\temp', 'x'", &off).request_class,
        CoarseClass::Read
    );

    // Our items must be libpg_query's statements.
    let sql = "SELECT 1; SELECT 2";
    let wrong = TextRange::new(TextSize::from(0), TextSize::from(12));
    let r = analyse_pg(sql, &[wrong], &ctx());
    assert!(r.iter().all(|s| s.class == CoarseClass::Unknown));
    assert!(
        r[0].reasons
            .iter()
            .any(|x| x.code == ReasonCode::ParserDisagreement)
    );
    let shifted = [
        TextRange::new(TextSize::from(0), TextSize::from(10)),
        TextRange::new(TextSize::from(10), TextSize::from(18)),
    ];
    assert!(
        analyse_pg(sql, &shifted, &ctx())
            .iter()
            .all(|s| s.class == CoarseClass::Unknown)
    );
    // An array slice: `:n` is one placeholder token to us, `:` and `n` to PostgreSQL.
    assert_eq!(class("SELECT a[1:n] FROM t"), CoarseClass::Read);
}

#[test]
fn a_server_newer_than_the_grammar_is_unknown() {
    let r = classify_with("SELECT 1", &ctx_at(Some(ServerVersion::new(19, 0, 0))));
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(codes(&r).contains(&ReasonCode::SyntaxNewerThanAgent));
    assert_eq!(
        classify_with("SELECT 1", &ctx_at(Some(ServerVersion::new(18, 6, 0)))).request_class,
        CoarseClass::Read
    );
    assert_eq!(
        classify_with("SELECT 1", &ctx_at(None)).request_class,
        CoarseClass::Read
    );
}

#[test]
fn prost_depth_limit_denies_not_panics() {
    let s = format!("SELECT {}1{}", "ARRAY[".repeat(47), "]".repeat(47));
    assert_eq!(classify_pg(&s).request_class, CoarseClass::Unknown);
    let unions: Vec<String> = (0..100).map(|i| format!("SELECT {i}")).collect();
    let r = classify_pg(&unions.join(" UNION ALL "));
    assert_eq!(r.request_class, CoarseClass::Unknown);
    // A limit, not a syntax error (round 1, M4).
    assert_eq!(codes(&r), [ReasonCode::TooComplex]);
    assert_eq!(codes(&classify_pg(&s)), [ReasonCode::TooComplex]);
    assert_eq!(read_only(&r), "Deny");
}

#[test]
fn set_targets_name_each_variable_with_its_value() {
    let s = one("SET search_path TO \"$user\", public");
    assert_eq!(s.kind, StmtKind::Set);
    assert_eq!(s.set_targets.len(), 1);
    assert_eq!(&*s.set_targets[0].name, "search_path");
    assert_eq!(s.set_targets[0].value.as_deref(), Some("\"$user\", public"));
    let session: Vec<_> = s
        .reasons
        .iter()
        .filter(|r| r.code == ReasonCode::SessionState)
        .collect();
    assert_eq!(session.len(), 1);
    assert_eq!(session[0].arg.as_deref(), Some("search_path"));
    for (sql, decision) in [
        ("SET search_path TO \"$user\", public", "Allow"),
        ("SET search_path = public, pg_catalog", "Deny"),
        ("SET search_path = pg_catalog, public", "Allow"),
        ("SET SCHEMA 'app'", "Allow"),
        ("SET statement_timeout = '5s'", "Allow"),
        ("SET LOCAL work_mem = '64MB'", "Allow"),
        ("SET TIME ZONE 'UTC'", "Allow"),
        ("SET DateStyle TO ISO, DMY", "Allow"),
        ("SET search_path TO DEFAULT", "Deny"),
        ("RESET statement_timeout", "Allow"),
        ("RESET ALL", "Deny"),
        ("SET transaction_read_only = off", "Deny"),
        ("SET ROLE admin", "Deny"),
        ("SET SESSION AUTHORIZATION postgres", "Deny"),
        ("RESET ROLE", "Deny"),
        ("SET NAMES 'LATIN1'", "Deny"),
        ("SET standard_conforming_strings = off", "Deny"),
    ] {
        let r = classify_pg(sql);
        assert_eq!(read_only(&r), decision, "{sql}: {r:?}");
    }
    let tz = one("SET TIME ZONE 'UTC'");
    assert_eq!(&*tz.set_targets[0].name, "timezone");
    assert_eq!(tz.set_targets[0].value.as_deref(), Some("'UTC'"));
    let reset = one("RESET statement_timeout");
    assert_eq!(reset.set_targets[0].value, None);
    assert!(one("RESET ALL").set_targets.is_empty());
    // A role switch is denied in every profile; a lexing change too.
    for sql in ["SET ROLE admin", "SET NAMES 'LATIN1'"] {
        let r = classify_pg(sql);
        for p in [Profile::ReadWriteDml, Profile::Ddl, Profile::Admin] {
            assert!(
                matches!(decide(&r, p, Actor::Human).verdict, Verdict::Deny(_)),
                "{sql} {p:?}"
            );
        }
    }
    let role = one("SET ROLE admin");
    assert!(role.effects.contains(EffectSet::PRIVILEGE));
    assert!(
        role.reasons
            .iter()
            .any(|r| r.code == ReasonCode::PrivilegeChange)
    );
    assert!(
        !role
            .reasons
            .iter()
            .any(|r| r.code == ReasonCode::PrivilegeGrant)
    );
    // GRANT is a privilege grant, never a role switch.
    let grant = one("GRANT SELECT ON t TO bob");
    assert_eq!(
        (grant.kind, grant.class),
        (StmtKind::Grant, CoarseClass::Admin)
    );
    assert!(
        !grant
            .reasons
            .iter()
            .any(|r| r.code == ReasonCode::PrivilegeChange)
    );
    assert_eq!(one("REVOKE SELECT ON t FROM bob").kind, StmtKind::Revoke);
}

#[test]
fn statement_kinds_and_classes() {
    use CoarseClass as C;
    use StmtKind as K;
    for (sql, kind, class) in [
        ("SELECT 1", K::Select, C::Read),
        ("VALUES (1), (2)", K::Values, C::Read),
        ("TABLE t", K::Select, C::Read),
        ("SHOW ALL", K::Show, C::Read),
        ("INSERT INTO t VALUES (1)", K::Insert, C::Write),
        ("UPDATE t SET a = 1 WHERE b = 2", K::Update, C::Write),
        ("DELETE FROM t WHERE b = 2", K::Delete, C::Write),
        (
            "MERGE INTO t USING s ON t.id = s.id WHEN NOT MATCHED THEN INSERT VALUES (s.id)",
            K::Merge,
            C::Write,
        ),
        ("TRUNCATE t", K::Truncate, C::Ddl),
        ("CREATE TABLE t (a int)", K::CreateTable, C::Ddl),
        ("CREATE TEMP TABLE t (a int)", K::CreateTable, C::Ddl),
        ("CREATE INDEX ON t (lower(a))", K::CreateOther, C::Ddl),
        ("ALTER TABLE t ADD COLUMN b int", K::Alter, C::Ddl),
        ("DROP VIEW v", K::Drop, C::Ddl),
        ("BEGIN", K::Begin, C::Session),
        ("START TRANSACTION READ ONLY", K::Begin, C::Session),
        ("COMMIT", K::Commit, C::Session),
        ("END", K::Commit, C::Session),
        ("ROLLBACK", K::Rollback, C::Session),
        ("ABORT", K::Rollback, C::Session),
        ("SAVEPOINT s", K::Savepoint, C::Session),
        ("RELEASE SAVEPOINT s", K::Savepoint, C::Session),
        ("ROLLBACK TO SAVEPOINT s", K::Savepoint, C::Session),
        ("PREPARE TRANSACTION 'x'", K::Rollback, C::Admin),
        ("COMMIT PREPARED 'x'", K::Commit, C::Admin),
        ("LOCK TABLE t IN ACCESS SHARE MODE", K::Lock, C::Write),
        ("PREPARE p AS SELECT 1", K::Prepare, C::Session),
        ("EXECUTE p", K::Execute, C::Unknown),
        ("LISTEN c", K::Listen, C::Session),
        ("UNLISTEN *", K::Listen, C::Session),
        ("NOTIFY c, 'x'", K::Notify, C::Session),
        ("CALL p()", K::Call, C::Unknown),
        ("DO $$ BEGIN NULL; END $$", K::Do, C::Unknown),
        ("CHECKPOINT", K::Other, C::Admin),
        ("ALTER SYSTEM SET work_mem = '1GB'", K::Other, C::Admin),
        ("LOAD 'plugin'", K::Other, C::Admin),
        ("VACUUM t", K::Other, C::Ddl),
        ("DISCARD PLANS", K::Other, C::Session),
        ("DISCARD ALL", K::Other, C::Admin),
        ("CREATE ROLE r", K::CreateOther, C::Admin),
        ("COPY t TO STDOUT", K::Copy, C::Read),
    ] {
        let s = one(sql);
        assert_eq!((s.kind, s.class), (kind, class), "{sql}: {s:?}");
    }
}

// ## Hostile inputs (Task 9 brief)

#[test]
fn hostile_comment_tricks() {
    for s in [
        "SELECT 1 /* ; DELETE FROM t */",
        "SELECT 1 -- ; DELETE FROM t",
        "SELECT 1 /* /* nested */ ; DELETE FROM t; */",
        "/* DELETE FROM t; */ SELECT 1",
        "SELECT 1 --\n;",
    ] {
        let r = classify_pg(s);
        assert_eq!(r.items.len(), 1, "{s}");
        assert_eq!(r.request_class, CoarseClass::Read, "{s}");
    }
    // A comment that hides nothing from the server hides nothing from the classifier.
    let r = classify_pg("/* SELECT */ DELETE FROM t /* WHERE id = 1 */");
    assert_eq!(r.request_class, CoarseClass::Write);
    assert_eq!(
        r.items[0]
            .destructive
            .iter()
            .map(|d| d.kind)
            .collect::<Vec<_>>(),
        [DestructiveKind::NoWhere]
    );
    // An unterminated nested comment is a parse error, never a read: `/*/` opens a comment
    // and the second `/*/` opens a nested one that never closes.
    for s in [
        "SELECT 1 /* /* */ DELETE FROM t",
        "SELECT /*/ DELETE FROM t /*/ 1",
    ] {
        let r = classify_pg(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}");
        assert_eq!(read_only(&r), "Deny", "{s}");
    }
}

#[test]
fn hostile_dollar_bodies_and_do_blocks() {
    for s in [
        "SELECT $$; DELETE FROM t; $$",
        "SELECT $a$ $$ ; DELETE FROM t; $$ $a$",
        "SELECT $_$it's$_$ AS x",
    ] {
        assert_eq!(class(s), CoarseClass::Read, "{s}");
    }
    for s in [
        "DO $$ BEGIN DELETE FROM t; END $$",
        "DO LANGUAGE plpgsql $x$ BEGIN PERFORM 1; END $x$",
        "DO 'BEGIN NULL; END'",
    ] {
        let r = classify_pg(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}");
        assert!(r.flags.contains(RiskFlags::DYNAMIC_SQL), "{s}");
        assert_eq!(read_only(&r), "Deny", "{s}");
    }
    // A $$ function body is a string to the parser: the CREATE is DDL, the body does not run.
    let f = one(
        "CREATE FUNCTION f() RETURNS int LANGUAGE plpgsql AS $$ BEGIN DELETE FROM t; RETURN 1; END $$",
    );
    assert_eq!(f.class, CoarseClass::Ddl);
    assert!(f.destructive.is_empty());
}

#[test]
fn hostile_explain_analyze_write() {
    for s in [
        "EXPLAIN ANALYZE DELETE FROM t",
        "EXPLAIN ANALYZE INSERT INTO t VALUES (1)",
        "EXPLAIN (ANALYZE, BUFFERS) UPDATE t SET a = 1",
        "EXPLAIN ANALYZE WITH d AS (DELETE FROM t RETURNING 1) SELECT * FROM d",
        "EXPLAIN ANALYZE CREATE TABLE x AS SELECT 1",
    ] {
        let r = classify_pg(s);
        assert!(r.request_class >= CoarseClass::Write, "{s}");
        assert!(r.flags.contains(RiskFlags::EXPLAIN_ANALYZE), "{s}");
        assert_eq!(read_only(&r), "Deny", "{s}");
        assert!(matches!(
            decide(&r, Profile::ReadOnlyPlusExplain, Actor::Human).verdict,
            Verdict::Deny(_)
        ));
    }
    assert_eq!(
        class("EXPLAIN ANALYZE SELECT f_write()"),
        CoarseClass::Unknown
    );
    assert_eq!(class("EXPLAIN EXECUTE p"), CoarseClass::Unknown);
    let r = classify_pg("EXPLAIN ANALYZE SELECT * FROM t");
    assert!(matches!(
        decide(&r, Profile::ReadOnlyPlusExplain, Actor::Human).verdict,
        Verdict::Allow
    ));
}

#[test]
fn hostile_writes_hidden_in_reads() {
    use CoarseClass as C;
    for (s, want) in [
        ("SELECT * FROM t FOR NO KEY UPDATE SKIP LOCKED", C::Write),
        ("SELECT * FROM (SELECT * FROM t FOR SHARE) s", C::Write),
        (
            "WITH l AS (SELECT * FROM t FOR KEY SHARE) SELECT 1",
            C::Write,
        ),
        ("SELECT * INTO TEMP x FROM t", C::Ddl),
        ("CREATE TEMP TABLE x (a int)", C::Ddl),
        ("CREATE TEMP SEQUENCE s", C::Ddl),
        ("CREATE TEMP VIEW v AS SELECT 1", C::Ddl),
        ("LOCK TABLE t", C::Write),
        ("LOCK t IN ACCESS EXCLUSIVE MODE NOWAIT", C::Write),
        ("SELECT pg_advisory_lock(1)", C::Write),
        ("SELECT pg_catalog.pg_advisory_xact_lock(1)", C::Write),
        ("SELECT setval('s', 1)", C::Write),
        ("SELECT lo_unlink(1)", C::Write),
        ("SELECT pg_create_physical_replication_slot('x')", C::Admin),
        ("SELECT lo_export(1, '/tmp/x')", C::Admin),
        ("SELECT pg_notify('c', 'x')", C::Unknown),
        ("SELECT pg_sleep(10)", C::Read),
    ] {
        let r = classify_pg(s);
        assert_eq!(r.request_class, want, "{s}: {r:?}");
        assert_eq!(read_only(&r), "Deny", "{s}");
    }
    let temp = one("CREATE TEMP TABLE x (a int)");
    assert!(temp.effects.contains(EffectSet::DDL_TEMP));
    assert!(!temp.effects.contains(EffectSet::DDL));
    let lock = one("LOCK TABLE t");
    assert_eq!(gate_class(&lock), GateClass::Lock);
}

#[test]
fn hostile_writing_functions_called_from_a_select() {
    for s in [
        "SELECT f_write()",
        "SELECT * FROM f_write()",
        "SELECT f_write() FROM t",
        "SELECT x FROM t WHERE f_write() > 0",
        "SELECT x FROM t ORDER BY f_write()",
        "SELECT count(*) FILTER (WHERE f_write()) FROM t",
        "SELECT public.lower('x')",
        "SELECT \"lower\"('x')",
        "SELECT pg_catalog.\"lower\"('x')",
        "SELECT U&\"\\006Cower\"('x')",
        "SELECT x::my_type FROM t",
        "SELECT CAST(x AS \"INT4\") FROM t",
        "SELECT x FROM t WHERE x ### 1",
        "SELECT x FROM t WHERE x OPERATOR(public.=) 1",
        "SELECT x FROM t ORDER BY x USING ###",
        "SELECT * FROM t TABLESAMPLE my_sampler (10)",
        "SELECT current_setting('x')",
    ] {
        let r = classify_pg(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}: {r:?}");
        assert_eq!(read_only(&r), "Deny", "{s}");
    }
    // Builtins (Task 3's reviewed rows) written as the server resolves them stay reads.
    for s in [
        "SELECT lower(x), LOWER(x), pg_catalog.lower(x) FROM t",
        "SELECT x::int, CAST(x AS bigint), x::text[], 'a'::varchar(3) FROM t",
        "SELECT count(*), sum(a) OVER (PARTITION BY b ORDER BY c) FROM t GROUP BY a, b, c",
        "SELECT extract(year FROM now()), substring(x FROM 2 FOR 3), trim(BOTH ' ' FROM x) FROM t",
        "SELECT x FROM t WHERE x BETWEEN 1 AND 2 AND y LIKE 'a%' AND z IN (1, 2) AND w IS DISTINCT FROM 3",
        "SELECT * FROM t TABLESAMPLE SYSTEM (10)",
        "SELECT * FROM generate_series(1, 10) g",
        "SELECT now() AT TIME ZONE 'UTC'",
    ] {
        let r = classify_pg(s);
        assert_eq!(r.request_class, CoarseClass::Read, "{s}: {r:?}");
        assert_eq!(read_only(&r), "Allow", "{s}");
    }
    // The profile's allowlist clears an exact name (conservatively).
    let mut c = ctx();
    c.fn_allowlist = Arc::from(vec![Arc::from("f_report"), Arc::from("app.f_stats")]);
    for (s, want) in [
        ("SELECT f_report()", CoarseClass::Read),
        ("SELECT F_REPORT()", CoarseClass::Read),
        ("SELECT app.f_stats()", CoarseClass::Read),
        ("SELECT f_stats()", CoarseClass::Unknown),
        ("SELECT other.f_report()", CoarseClass::Unknown),
        ("SELECT pg_sleep(1)", CoarseClass::Read),
    ] {
        let r = classify_with(s, &c);
        assert_eq!(r.request_class, want, "{s}");
    }
    let r = classify_with("SELECT pg_sleep(1)", &c);
    assert!(codes(&r).contains(&ReasonCode::FnDenylisted));
    // A catalog's volatile function sets VOLATILE_FN and is Unknown (S-04).
    let v = with_catalog(
        "SELECT my_fn()",
        FnLookup::Known {
            volatile: true,
            writes: false,
        },
    );
    assert!(v.flags.contains(RiskFlags::VOLATILE_FN));
    assert_eq!(v.request_class, CoarseClass::Unknown);
    let w = with_catalog(
        "SELECT my_fn()",
        FnLookup::Known {
            volatile: false,
            writes: true,
        },
    );
    assert_eq!(w.request_class, CoarseClass::Unknown);
    assert!(w.request_effects.contains(EffectSet::WRITE_DATA));
}

#[test]
fn hostile_call_prepare_and_execute() {
    for s in [
        "CALL p()",
        "CALL app.p(1, 'x')",
        "EXECUTE p(1)",
        "EXPLAIN EXECUTE p(1)",
        "CREATE TABLE x AS EXECUTE p",
    ] {
        let r = classify_pg(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}");
        assert_eq!(read_only(&r), "Deny", "{s}");
    }
    let p = one("PREPARE p AS DELETE FROM t");
    assert_eq!(p.kind, StmtKind::Prepare);
    assert!(
        p.effects
            .contains(EffectSet::PREPARE | EffectSet::WRITE_DATA)
    );
    assert_eq!(p.class, CoarseClass::Write);
    let q = one("PREPARE q(int) AS SELECT * FROM t WHERE id = $1");
    assert_eq!(q.class, CoarseClass::Session);
    assert_eq!(read_only(&classify_pg("PREPARE q AS SELECT 1")), "Deny");
    assert_eq!(class("DEALLOCATE ALL"), CoarseClass::Session);
}

#[test]
fn turkish_identifiers_and_text() {
    for s in [
        "SELECT ad, soyad FROM müşteri WHERE şehir = 'İstanbul'",
        "SELECT \"Şube\".\"Kod\" FROM \"Şube\"",
        "SELECT lower(ünvan) FROM çalışan ORDER BY ılık",
        "SELECT ıd FROM İşlem",
        "SELECT 'ığüşöç İĞÜŞÖÇ' AS metin",
    ] {
        let r = classify_pg(s);
        assert_eq!(r.request_class, CoarseClass::Read, "{s}: {r:?}");
        assert_eq!(read_only(&r), "Allow", "{s}");
    }
    // Non-ASCII letters are never folded: these are not the builtins they resemble.
    for s in [
        "SELECT LOWER(x) FROM t WHERE PG_SLEEP(1) IS NULL",
        "SELECT LİMİT(1)",
        "SELECT ıntval(1)",
        "SELECT PG_READ_FİLE('/x')",
        "SELECT pg_sleep(1)",
    ] {
        assert_eq!(read_only(&classify_pg(s)), "Deny", "{s}");
        assert_ne!(gate_class(&one(s)), GateClass::Read, "{s}");
    }
    let d = one("DELETE FROM sipariş");
    assert_eq!(d.destructive[0].kind, DestructiveKind::NoWhere);
    assert_eq!(
        d.destructive[0]
            .target
            .as_ref()
            .map(|o| o.name.parts[0].text.to_string()),
        Some("sipariş".to_string())
    );
    // The deny list matches the bare name ASCII-lowercased in any form.
    assert!(codes(&classify_pg("SELECT PG_SLEEP(1)")).contains(&ReasonCode::FnDenylisted));
    assert!(codes(&classify_pg("SELECT \"pg_sleep\"(1)")).contains(&ReasonCode::FnDenylisted));
    assert!(codes(&classify_pg("SELECT public.pg_sleep(1)")).contains(&ReasonCode::FnDenylisted));
}

/// P-M3b-T3-1 at classification: a session whose known `search_path` puts another schema
/// before `pg_catalog` may resolve an unqualified name to a shadowing user object, so
/// unqualified builtins, operators and casts are unknown there; `pg_catalog.`-qualified ones
/// stay builtins.
#[test]
fn a_search_path_with_pg_catalog_after_another_schema_unproves_unqualified_builtins() {
    let mut shadowed = ctx();
    shadowed.search_path = Arc::from(vec![Arc::from("app"), Arc::from("pg_catalog")]);
    for s in [
        "SELECT lower(x) FROM t",
        "SELECT x FROM t WHERE x = 1",
        "SELECT x FROM t WHERE x BETWEEN 1 AND 2",
        "SELECT x::text FROM t",
    ] {
        let r = classify_with(s, &shadowed);
        invariants(s, &r);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}");
    }
    assert_eq!(
        classify_with("SELECT pg_catalog.lower(x), x::int FROM t", &shadowed).request_class,
        CoarseClass::Read
    );
    for path in [vec![], vec!["pg_catalog", "app"], vec!["$user", "public"]] {
        let mut c = ctx();
        c.search_path = Arc::from(path.iter().map(|p| Arc::from(*p)).collect::<Vec<_>>());
        assert_eq!(
            classify_with("SELECT lower(x) FROM t WHERE x = 1", &c).request_class,
            CoarseClass::Read,
            "{path:?}"
        );
    }
}

#[test]
fn touched_objects() {
    let s = one("INSERT INTO app.orders (id) SELECT id FROM app.staging JOIN app.users u ON true");
    let names = |v: &Vec<sql_safety::ObjectRef>| -> Vec<String> {
        v.iter()
            .map(|o| {
                o.name
                    .parts
                    .iter()
                    .map(|p| p.text.to_string())
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .collect()
    };
    assert_eq!(names(&s.touched.writes), ["app.orders"]);
    assert_eq!(names(&s.touched.reads), ["app.staging", "app.users"]);
    let c = one("SELECT lower(x), lower(y) FROM t");
    assert_eq!(names(&c.touched.calls), ["lower"]);
}

// ## Fix round 1 (review task-m3b-09-review.md)

fn classify_script(sql: &str, c: &ClassifyCtx) -> SafetyReport {
    let mut c = c.clone();
    c.expect = Expect::Script;
    let r = classify_with(sql, &c);
    invariants(sql, &r);
    r
}

/// I1: every place that names a target type coerces to it; a domain or user type there is
/// as unproven as `CAST(x AS my_type)`.
#[test]
fn round1_coercions_to_named_types_are_checked() {
    for s in [
        "SELECT JSON_VALUE('1', '$' RETURNING my_domain)",
        "SELECT JSON_QUERY('{}', '$' RETURNING my_type)",
        "SELECT JSON_OBJECT('a': 1 RETURNING my_type)",
        "SELECT JSON_ARRAY(1, 2 RETURNING my_type)",
        "SELECT JSON_ARRAY(SELECT 1 RETURNING my_type)",
        "SELECT JSON_SERIALIZE('{}' RETURNING my_type)",
        "SELECT JSON_ARRAYAGG(a RETURNING my_type) FROM t",
        "SELECT JSON_OBJECTAGG(k : v RETURNING my_type) FROM t",
        "SELECT * FROM JSON_TABLE('[1]', '$[*]' COLUMNS (a my_domain PATH '$')) j",
        "SELECT * FROM JSON_TABLE('[1]', '$[*]' COLUMNS (NESTED PATH '$' COLUMNS (b my_domain PATH '$'))) j",
        "SELECT * FROM XMLTABLE('/r' PASSING '<r/>' COLUMNS a my_domain PATH 'a') x",
        "SELECT XMLSERIALIZE(CONTENT '<a/>'::xml AS my_type)",
        "SELECT ROW(1, 2)::my_type",
        "SELECT ARRAY[1]::my_domain[]",
        "SELECT JSON_VALUE('1', '$' RETURNING \"INT4\")",
        "SELECT JSON_VALUE('1', '$' RETURNING public.int4)",
    ] {
        let r = classify_pg(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}: {r:?}");
        assert!(codes(&r).contains(&ReasonCode::CastUnknown), "{s}");
        assert_eq!(read_only(&r), "Deny", "{s}");
    }
    // A function's column definition list coerces its result too.
    let r = classify_pg("SELECT * FROM json_to_record('{}') AS x(a my_domain)");
    assert!(
        r.items[0]
            .reasons
            .iter()
            .any(|x| x.code == ReasonCode::CastUnknown && x.arg.as_deref() == Some("my_domain"))
    );
    // Builtin targets stay reads.
    for s in [
        "SELECT JSON_VALUE('1', '$' RETURNING int)",
        "SELECT JSON_QUERY('{}', '$' RETURNING jsonb)",
        "SELECT JSON_OBJECT('a': 1 RETURNING text)",
        "SELECT * FROM JSON_TABLE('[1]', '$[*]' COLUMNS (o FOR ORDINALITY, a int PATH '$', b text PATH '$')) j",
        "SELECT * FROM XMLTABLE('/r' PASSING '<r/>' COLUMNS o FOR ORDINALITY, a int PATH 'a') x",
        "SELECT XMLSERIALIZE(CONTENT '<a/>'::xml AS text)",
        "SELECT ROW(1, 2)::text, ARRAY[1]::bigint[]",
        "SELECT 'a'::\"char\"",
    ] {
        let r = classify_pg(s);
        assert_eq!(r.request_class, CoarseClass::Read, "{s}: {r:?}");
    }
}

/// I2a: a statement that may change the search path, the lexing modes or the role makes every
/// later statement of the same text `Unknown`.
#[test]
fn round1_session_changes_make_later_items_unknown() {
    for s in [
        "SET search_path = evil, pg_catalog; SELECT lower('A')",
        "SET search_path TO DEFAULT; SELECT lower('A')",
        "RESET search_path; SELECT 1",
        "SELECT set_config('search_path', 'evil,pg_catalog', false); SELECT 1 + 1",
        "SELECT set_config(current_user, 'x', false); SELECT 1",
        "SET standard_conforming_strings = off; SELECT 'a\\'' ; DELETE FROM t; -- '",
        "SET ROLE admin; SELECT 1",
        "SET SESSION AUTHORIZATION bob; SELECT 1",
        "RESET ALL; SELECT 1",
        "DISCARD ALL; SELECT 1",
        "SELECT f_unknown(); SELECT 1",
        "CALL p(); SELECT 1",
        "DO $$ BEGIN PERFORM set_config('search_path', 'evil', false); END $$; SELECT lower('A')",
    ] {
        let r = classify_script(s, &ctx());
        assert_eq!(r.items.len(), 2, "{s}");
        let later = &r.items[1];
        assert_eq!(later.class, CoarseClass::Unknown, "{s}: {later:?}");
        assert!(later.effects.contains(EffectSet::UNKNOWN), "{s}");
        assert_eq!(read_only(&r), "Deny", "{s}");
    }
    // The scs-off case: as the server reads it, item 2 runs a DELETE; it is never a read.
    let r = classify_script(
        "SET standard_conforming_strings = off; SELECT 'a\\'' ; DELETE FROM t; -- '",
        &ctx(),
    );
    assert_ne!(r.items[1].class, CoarseClass::Read);
    // Settings that keep names resolving the same leave later items alone.
    for s in [
        "SET search_path = pg_catalog, public; SELECT lower('A')",
        "SET search_path TO app; SELECT lower('A')",
        "SET statement_timeout = '5s'; SELECT 1",
        "SELECT 1; SELECT lower('A')",
    ] {
        let r = classify_script(s, &ctx());
        assert_eq!(r.items[1].class, CoarseClass::Read, "{s}: {r:?}");
    }
    // Only items after the change.
    let r = classify_script(
        "SELECT 1; SET search_path = evil, pg_catalog; SELECT 2",
        &ctx(),
    );
    assert_eq!(r.items[0].class, CoarseClass::Read);
    assert_eq!(r.items[2].class, CoarseClass::Unknown);
}

/// I2b: the `ClassifyCtx.search_path` contract (one folded schema name per element, `$user`
/// literal) and its fail-closed reading.
#[test]
fn round1_search_path_contract_fails_closed() {
    let read = |path: &[&str]| {
        let mut c = ctx();
        c.search_path = Arc::from(path.iter().map(|p| Arc::from(*p)).collect::<Vec<_>>());
        classify_with("SELECT lower(x) FROM t", &c).request_class
    };
    for path in [
        &["pg_catalog", "$user", "public"][..],
        &["$user", "public"],
        &["pg_temp", "app"],
        &["Mixed Case Schema", "pg_catalog"][..1],
        &[],
    ] {
        assert_eq!(read(path), CoarseClass::Read, "{path:?}");
    }
    for path in [
        &["evil", "pg_catalog"][..],
        &["evil", "PG_CATALOG"],
        &["Pg_Catalog"],
        &["\"$user\", public, pg_catalog"],
        &["$user, public"],
        &["\"evil\""],
        &[""],
        &[" pg_catalog"],
        &["pg_catalog "],
        &["$user", "public", "pg_catalog"],
    ] {
        assert_eq!(read(path), CoarseClass::Unknown, "{path:?}");
    }
}

/// M2: implicit `=` lookups go through the search path too.
#[test]
fn round1_implicit_equals_follow_the_search_path() {
    let mut shadowed = ctx();
    shadowed.search_path = Arc::from(vec![Arc::from("app"), Arc::from("pg_catalog")]);
    for s in [
        "SELECT CASE x WHEN 1 THEN 'a' END FROM t",
        "SELECT * FROM t JOIN u USING (a)",
        "SELECT * FROM t NATURAL JOIN u",
    ] {
        assert_eq!(
            classify_with(s, &shadowed).request_class,
            CoarseClass::Unknown,
            "{s}"
        );
        assert_eq!(class(s), CoarseClass::Read, "{s}");
    }
    // A searched CASE and JOIN … ON name their operators.
    assert_eq!(
        class("SELECT CASE WHEN x = 1 THEN 'a' END FROM t"),
        CoarseClass::Read
    );
}

/// M3: an item must be its raw statement exactly, not a part of it.
#[test]
fn round1_items_must_match_raw_statements_exactly() {
    let sql = "DELETE FROM t WHERE a = 1";
    let part = TextRange::new(TextSize::from(0), TextSize::from(13));
    let r = analyse_pg(sql, &[part], &ctx());
    assert_eq!(r[0].class, CoarseClass::Unknown);
    assert!(
        r[0].reasons
            .iter()
            .any(|x| x.code == ReasonCode::ParserDisagreement)
    );
    let whole = TextRange::new(TextSize::from(0), TextSize::of(sql));
    assert_eq!(
        analyse_pg(sql, &[whole], &ctx())[0].class,
        CoarseClass::Write
    );
    // Leading comments and the terminator are outside the item, as our splitter cuts it.
    let sql = "/* c */ SELECT 1; -- x\nSELECT 2;";
    let r = classify_pg(sql);
    assert_eq!(r.items.len(), 2);
    assert!(
        r.items.iter().all(|s| s.class == CoarseClass::Read),
        "{r:?}"
    );
}

/// M4: limits are `TooComplex`, syntax errors `ParseError`.
#[test]
fn round1_limits_are_too_complex() {
    let deep = format!("SELECT {}1{}", "(".repeat(300), ")".repeat(300));
    let concat = format!("SELECT {}", vec!["a"; 60].join(" || "));
    for s in [deep.as_str(), concat.as_str()] {
        let r = classify_pg(s);
        assert_eq!(codes(&r), [ReasonCode::TooComplex], "{}", &s[..20]);
    }
    assert_eq!(codes(&classify_pg("SELEC 1")), [ReasonCode::ParseError]);
}

/// M5, M6: session resets, `OWNER TO`, `ALTER DOMAIN … DROP CONSTRAINT`.
#[test]
fn round1_resets_owners_and_domain_constraints() {
    for s in ["DISCARD ALL", "RESET ALL"] {
        let r = one(s);
        assert!(r.flags.contains(RiskFlags::LEX_MODE_CHANGE), "{s}");
        assert!(
            r.reasons
                .iter()
                .any(|x| x.code == ReasonCode::PrivilegeChange),
            "{s}"
        );
        let req = classify_pg(s);
        for p in [Profile::ReadOnly, Profile::ReadWriteDml, Profile::Admin] {
            assert!(
                matches!(decide(&req, p, Actor::Human).verdict, Verdict::Deny(_)),
                "{s} {p:?}"
            );
        }
    }
    assert!(
        !one("DISCARD PLANS")
            .flags
            .contains(RiskFlags::LEX_MODE_CHANGE)
    );
    let owner = one("ALTER TABLE t OWNER TO bob");
    assert!(owner.effects.contains(EffectSet::PRIVILEGE));
    assert_eq!(owner.class, CoarseClass::Admin);
    assert_eq!(
        destructive("ALTER DOMAIN d DROP CONSTRAINT positive"),
        [DestructiveKind::DropConstraint]
    );
    assert!(destructive("ALTER DOMAIN d ADD CONSTRAINT c CHECK (VALUE > 0)").is_empty());
}

/// M7: a destructive reason's range is where its target's name is written.
#[test]
fn round1_destructive_ranges_cover_the_written_target() {
    let target_text = |sql: &str| -> Vec<String> {
        classify_pg(sql)
            .items
            .iter()
            .flat_map(|s| s.destructive.iter())
            .map(|d| sql[usize::from(d.range.start())..usize::from(d.range.end())].to_string())
            .collect()
    };
    assert_eq!(
        target_text("DELETE FROM \"Sipariş\" WHERE true"),
        ["\"Sipariş\""]
    );
    assert_eq!(
        target_text("TRUNCATE app.audit_log, other"),
        ["app.audit_log", "other"]
    );
    assert_eq!(
        target_text("DROP TABLE IF EXISTS Foo, bar.\"Baz\""),
        ["Foo", "bar.\"Baz\""]
    );
    assert_eq!(target_text("DROP SCHEMA public CASCADE"), ["public"]);
    assert_eq!(target_text("DROP DATABASE prod"), ["prod"]);
    assert_eq!(
        target_text("ALTER TABLE app . t DROP COLUMN c"),
        ["app . t"]
    );
    assert_eq!(target_text("UPDATE /* x */ müşteri SET a = 1"), ["müşteri"]);
    // The written target, not an earlier name that reads the same.
    let sql = "WITH t AS (SELECT 1) DELETE FROM t";
    let r = one(sql);
    assert_eq!(
        usize::from(r.destructive[0].range.start()),
        sql.rfind('t').unwrap()
    );
}

/// M9: a leading byte-order mark is whitespace, as our lexer reads it.
#[test]
fn round1_a_leading_bom_is_whitespace() {
    let r = classify_pg("\u{feff}SELECT 1");
    assert_eq!(r.request_class, CoarseClass::Read, "{r:?}");
    let d = classify_pg("\u{feff}DELETE FROM t");
    assert_eq!(d.request_class, CoarseClass::Write);
    assert_eq!(d.items[0].destructive[0].kind, DestructiveKind::NoWhere);
}
