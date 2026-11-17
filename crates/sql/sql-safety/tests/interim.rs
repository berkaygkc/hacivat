//! The interim backend for MySQL, MariaDB and SQLite (Task 10; R-M3-54): `sqlparser` strict per
//! threshold view, token agreement, the lexer pre-scan union. Hostile cases first-class: the
//! T4 producer contract (every Unknown carries `UNKNOWN` or a doubt reason; SET reasons match
//! targets) is checked on every report these tests build.

use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes, ServerVersion};
use sql_ident::QualifiedName;
use sql_safety::{
    Actor, ClassifyCtx, CoarseClass, Confidence, DestructiveKind, EffectSet, Expect, FnCatalog,
    FnLookup, GateClass, Profile, ReasonCode, RiskFlags, SafetyReport, StmtKind, Verdict,
    classify_interim, decide, gate_class, sqlite_read_pragmas,
};

// ---------------------------------------------------------------------------------------------
// Helpers

fn ctx_with(d: DialectId, version: Option<ServerVersion>, modes: Option<LexModes>) -> ClassifyCtx {
    let mut dialect = DialectCtx::new(d, version);
    if let Some(m) = modes {
        dialect.modes = m;
    }
    ClassifyCtx::new(dialect, ClientFlavor::None, Expect::SingleStatement)
}

fn run(d: DialectId, sql: &str) -> SafetyReport {
    let r = classify_interim(sql, &ctx_with(d, None, None));
    assert_contract(&r, sql);
    r
}

fn classify_mysql(sql: &str) -> SafetyReport {
    run(DialectId::MySql, sql)
}

fn classify_mariadb(sql: &str) -> SafetyReport {
    run(DialectId::MariaDb, sql)
}

fn classify_sqlite(sql: &str) -> SafetyReport {
    run(DialectId::Sqlite, sql)
}

fn mysql_ansi() -> LexModes {
    LexModes::from_mysql_sql_mode("ANSI_QUOTES")
}

fn mysql_default() -> LexModes {
    LexModes::default_for(DialectId::MySql)
}

fn classify_with_modes(modes: LexModes, sql: &str) -> SafetyReport {
    let r = classify_interim(sql, &ctx_with(DialectId::MySql, None, Some(modes)));
    assert_contract(&r, sql);
    r
}

fn reasons(r: &SafetyReport) -> Vec<ReasonCode> {
    r.items
        .iter()
        .flat_map(|i| i.reasons.iter().map(|x| x.code))
        .chain(r.pre_decision.iter().flatten().map(|x| x.code))
        .collect()
}

fn has(r: &SafetyReport, code: ReasonCode) -> bool {
    reasons(r).contains(&code)
}

fn read_only_denies(r: &SafetyReport) -> bool {
    matches!(
        decide(r, Profile::ReadOnly, Actor::Human).verdict,
        Verdict::Deny(_)
    )
}

fn gate(r: &SafetyReport) -> GateClass {
    r.items
        .iter()
        .map(gate_class)
        .max()
        .unwrap_or(GateClass::Unknown)
}

fn destructive(r: &SafetyReport) -> Vec<DestructiveKind> {
    r.items
        .iter()
        .flat_map(|i| i.destructive.iter().map(|d| d.kind))
        .collect()
}

/// The T4 producer contract, on every report: an `Unknown` statement carries `UNKNOWN` or a
/// doubt reason (never the class field alone); a `SET`'s `SessionState` arguments are exactly
/// its targets' names; the backends and the dialect are recorded.
fn assert_contract(r: &SafetyReport, sql: &str) {
    for item in &r.items {
        if item.class == CoarseClass::Unknown {
            assert!(
                item.effects.contains(EffectSet::UNKNOWN),
                "{sql}: Unknown without UNKNOWN: {item:?}"
            );
            assert!(!item.reasons.is_empty(), "{sql}: Unknown without a reason");
        }
        assert!(!item.effects.is_empty(), "{sql}: empty effects");
        if item.kind == StmtKind::Set && !item.set_targets.is_empty() {
            let mut named: Vec<&str> = item
                .reasons
                .iter()
                .filter(|x| x.code == ReasonCode::SessionState)
                .filter_map(|x| x.arg.as_deref())
                .collect();
            let mut targets: Vec<&str> = item.set_targets.iter().map(|t| &*t.name).collect();
            named.sort_unstable();
            targets.sort_unstable();
            assert_eq!(named, targets, "{sql}");
        }
        assert!(
            !item
                .reasons
                .iter()
                .any(|x| x.code == ReasonCode::PrivilegeChange)
                || item.kind == StmtKind::Set,
            "{sql}: PrivilegeChange is for role switches only"
        );
    }
    assert!(r.dialect.is_some(), "{sql}");
    let names: Vec<(&str, &str)> = r.backends.iter().map(|b| (b.name, b.version)).collect();
    assert_eq!(
        names,
        [("sqlparser", "0.64.0"), ("lexer-prescan", "1")],
        "{sql}"
    );
    assert!(r.matches_text(sql), "{sql}");
}

// ---------------------------------------------------------------------------------------------
// The plan's tests (Task 10 Step 1)

/// `alg/13` SQL-SAFE-02 test cases 22-27 and 31 (classes as the spec gives them; see the
/// notes where M3b's model is stricter).
#[test]
fn safe02_cases_22_to_27_and_31() {
    // 22: Read + RESOURCE_HEAVY + EXECUTABLE_COMMENT → Deny (SLEEP has no bit in M3b: the
    // FnDenylisted reason carries it).
    let r = classify_mysql("SELECT 1 /*!99999 , SLEEP(100) */");
    assert_eq!(r.request_class, CoarseClass::Read);
    assert!(r.flags.contains(RiskFlags::EXEC_COMMENT));
    assert!(has(&r, ReasonCode::FnDenylisted) && has(&r, ReasonCode::ExecutableComment));
    assert!(read_only_denies(&r));
    assert_eq!(gate(&r), GateClass::Unknown);
    // 23: Ddl + EXECUTABLE_COMMENT → Deny.
    let r = classify_mysql("/*!50000 DROP TABLE t */");
    assert_eq!(r.request_class, CoarseClass::Ddl);
    assert_eq!(r.items[0].kind, StmtKind::Drop);
    assert!(r.flags.contains(RiskFlags::EXEC_COMMENT));
    assert!(read_only_denies(&r));
    // 24: Admin (file) → Deny. INTO OUTFILE does not parse: Unknown, with EXTERNAL_IO named.
    let r = classify_mysql("SELECT * FROM t INTO OUTFILE '/tmp/x'");
    assert!(r.request_effects.contains(EffectSet::EXTERNAL_IO));
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(read_only_denies(&r));
    let r = classify_mysql("SELECT LOAD_FILE('/etc/passwd')");
    assert_eq!(r.request_class, CoarseClass::Admin);
    assert!(has(&r, ReasonCode::ExternalFile));
    assert!(read_only_denies(&r));
    // 25: MultiStatement → Deny.
    let r = classify_mysql("SELECT 1--1;DROP TABLE t");
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(has(&r, ReasonCode::MultiStatement));
    assert!(read_only_denies(&r));
    // 26: Write (ADVISORY_LOCK) → Deny.
    let r = classify_mysql("SELECT GET_LOCK('a', 1)");
    assert_eq!(r.request_class, CoarseClass::Write);
    assert!(r.request_effects.contains(EffectSet::ADVISORY_LOCK));
    assert!(read_only_denies(&r));
    // 27: Session (USER_VAR_ASSIGN). alg/13 S-13 allows it flagged; M3b has no flag for it, so
    // SESSION_STATE denies on read-only (known gap `mysql-user-variable-assignment`).
    let r = classify_mysql("SELECT @v := 1");
    assert_eq!(r.request_class, CoarseClass::Session);
    assert!(r.request_effects.contains(EffectSet::SESSION_STATE));
    assert!(read_only_denies(&r));
    // 31: Admin; Admin; Admin (VACUUM INTO does not parse: Unknown with EXTERNAL_IO); Read.
    let r = classify_sqlite("PRAGMA query_only=0");
    assert_eq!(r.request_class, CoarseClass::Admin);
    assert!(read_only_denies(&r));
    let r = classify_sqlite("ATTACH 'x.db' AS x");
    assert_eq!(r.request_class, CoarseClass::Admin);
    assert!(read_only_denies(&r));
    let r = classify_sqlite("VACUUM INTO 'y.db'");
    assert!(r.request_effects.contains(EffectSet::EXTERNAL_IO));
    assert_ne!(r.request_class, CoarseClass::Read);
    assert!(read_only_denies(&r));
    let r = classify_sqlite("PRAGMA table_info(t)");
    assert_eq!(r.request_class, CoarseClass::Read);
    assert!(!read_only_denies(&r));
}

#[derive(Debug, Deserialize)]
struct Safe06 {
    version: u32,
    row: Vec<Safe06Row>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code, reason = "notes document the rows")]
struct Safe06Row {
    id: String,
    dialect: String,
    sql: String,
    read_only: String,
    note: Option<String>,
}

fn dialect_named(s: &str) -> DialectId {
    match s {
        "mysql" => DialectId::MySql,
        "mariadb" => DialectId::MariaDb,
        "sqlite" => DialectId::Sqlite,
        other => panic!("dialect {other}"),
    }
}

/// SQL-SAFE-06 run A offline: every M/L catalogue row is denied on the read-only profile
/// before execution (except the rows the catalogue itself does not deny), and is never `Read`
/// at the gate when denied. Task 18 moves this to the bypass TOML.
#[test]
fn safe06_m1_to_m19_and_l1_to_l9_offline() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/safe06_interim.toml");
    let file: Safe06 = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(file.version, 1);
    let mut ids: Vec<&str> = file.row.iter().map(|r| r.id.as_str()).collect();
    ids.dedup();
    for n in 1..=19 {
        assert!(ids.contains(&format!("M{n}").as_str()), "M{n}");
    }
    for n in 1..=9 {
        assert!(ids.contains(&format!("L{n}").as_str()), "L{n}");
    }
    for row in &file.row {
        let r = run(dialect_named(&row.dialect), &row.sql);
        match row.read_only.as_str() {
            "deny" => {
                assert!(read_only_denies(&r), "{} {}: {r:?}", row.id, row.sql);
                assert_ne!(gate(&r), GateClass::Read, "{} {}", row.id, row.sql);
            }
            "allow" => assert!(!read_only_denies(&r), "{} {}", row.id, row.sql),
            other => panic!("{other}"),
        }
    }
}

#[test]
fn executable_comment_views_are_all_classified() {
    assert_eq!(
        classify_mysql("SELECT 1 /*!80000 , (SELECT 1 FROM t FOR UPDATE) */").request_class,
        CoarseClass::Write
    );
    // Every threshold is read both ways: an inactive gate (99999) and an unknown server.
    let r = classify_mysql("SELECT 1 /*!99999 , GET_LOCK('a', 1) */");
    assert_eq!(r.request_class, CoarseClass::Write);
    // A known server version does not hide a body: the classifier never trusts the gate.
    let r = classify_interim(
        "SELECT 1 /*!90000 , GET_LOCK('a', 1) */",
        &ctx_with(DialectId::MySql, Some(ServerVersion::new(8, 4, 0)), None),
    );
    assert_eq!(r.request_class, CoarseClass::Write);
    // Two thresholds: three readings; the middle one alone writes. Both bodies are read.
    let r = classify_mysql(
        "SELECT 1 /*!50000 , (SELECT 1 FROM t FOR UPDATE) */ /*!80000 , GET_LOCK('b', 1) */",
    );
    assert!(
        r.request_effects
            .contains(EffectSet::LOCK_ROW | EffectSet::ADVISORY_LOCK)
    );
    assert_eq!(r.items[0].confidence, Confidence::Conservative);
    // An unversioned body always runs.
    let r = classify_mysql("SELECT 1 /*! , SLEEP(1) */");
    assert!(has(&r, ReasonCode::FnDenylisted));
    // MySQL reads five version digits; the sixth is body text (`/*!800001 */` runs `1` from
    // 8.0.0): `SELECT 2 1` does not parse. MariaDB reads six (version 80.0.1, empty body).
    let r = classify_mysql("SELECT 2 /*!800001 */");
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(has(&r, ReasonCode::ParseError));
    let r = classify_mariadb("SELECT 2 /*!800001 */");
    assert_eq!(r.request_class, CoarseClass::Read);
    assert!(r.flags.contains(RiskFlags::EXEC_COMMENT));
    // `/*!` inside a string is text, not a comment.
    let r = classify_mysql("SELECT '/*!50000 DROP TABLE t */'");
    assert_eq!(r.request_class, CoarseClass::Read);
    assert!(!r.flags.contains(RiskFlags::EXEC_COMMENT));
    // More readings than `max_exec_comment_views` (16): TooComplex.
    let many: String = (0..16)
        .map(|k| format!(" /*!{} , {k} */", 50000 + k))
        .collect();
    let r = classify_mysql(&format!("SELECT 1{many}"));
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(has(&r, ReasonCode::TooComplex));
    let fifteen: String = (0..15)
        .map(|k| format!(" /*!{} , {k} */", 50000 + k))
        .collect();
    assert_eq!(
        classify_mysql(&format!("SELECT 1{fifteen}")).request_class,
        CoarseClass::Read
    );
}

/// An executable comment whose end depends on how its body reads (a `*/` inside a `#` comment
/// or quotes; ledger 126/138): both readings are classified and unioned, and the statement is
/// `Unknown` (`AmbiguousBoundary`, Task 14's rule).
#[test]
fn ambiguous_comment_end_is_read_both_ways() {
    // The skipping reading ends at the first `*/`; a server that runs the body ends it at the
    // second, after `GET_LOCK` (inside `#`, the first `*/` is comment text).
    let r = classify_mysql("SELECT 1 /*!50000 , GET_LOCK('a', 1) # */\n */");
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(has(&r, ReasonCode::AmbiguousBoundary));
    assert!(
        r.request_effects.contains(EffectSet::ADVISORY_LOCK),
        "{r:?}"
    );
    // The running reading of a quoted `*/` reaches a second statement.
    let r = classify_mysql("SELECT 1 /*!50000 , '*/' */ ; DROP TABLE t; -- '");
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(r.request_effects.contains(EffectSet::DDL), "{r:?}");
    assert!(read_only_denies(&r));
}

#[test]
fn ansi_quotes_token_mismatch_denies() {
    assert_eq!(
        classify_with_modes(mysql_ansi(), r#"SELECT "a" FROM t"#).request_class,
        CoarseClass::Read
    );
    assert_eq!(
        classify_with_modes(mysql_default(), r#"SELECT "a"" FROM t"#).request_class,
        CoarseClass::Unknown
    );
}

/// The plan's `token_disagreement_denies`: where our lexer (with the session's modes) and
/// `sqlparser`'s tokenizer split a string differently, the statement is `Unknown`.
#[test]
fn token_disagreement_denies() {
    let nbe = LexModes::from_mysql_sql_mode("NO_BACKSLASH_ESCAPES");
    // With NO_BACKSLASH_ESCAPES the server reads `'a\'` as a string and runs SLEEP; sqlparser
    // reads one string to the last quote.
    let r = classify_with_modes(nbe, r"SELECT 'a\' , SLEEP(1) -- '");
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(has(&r, ReasonCode::ParserDisagreement), "{r:?}");
    assert!(r.request_effects.contains(EffectSet::UNKNOWN));
    // Default modes: both read the backslash escape alike.
    assert_eq!(
        classify_with_modes(mysql_default(), r"SELECT 'a\'b' FROM t").request_class,
        CoarseClass::Read
    );
}

#[test]
fn mariadb_sequence_functions_are_not_read() {
    for s in [
        "SELECT NEXT VALUE FOR s",
        "SELECT NEXTVAL(s)",
        "SELECT SETVAL(s, 5)",
        "SELECT LASTVAL(s)",
        "SELECT PREVIOUS VALUE FOR s",
        "SELECT nextval(s) FROM customers",
    ] {
        let r = classify_mariadb(s);
        assert_ne!(r.request_class, CoarseClass::Read, "{s}");
        assert!(r.request_effects.contains(EffectSet::SEQUENCE), "{s}");
        assert!(read_only_denies(&r), "{s}");
    }
}

#[test]
fn show_describe_explain_are_read() {
    for s in [
        "SHOW TABLES",
        "SHOW FULL COLUMNS FROM t LIKE 'a%'",
        "DESCRIBE t",
        "DESC db.t c",
        "EXPLAIN FORMAT=JSON SELECT * FROM t",
    ] {
        assert_eq!(classify_mysql(s).request_class, CoarseClass::Read, "{s}");
    }
    for s in [
        "SHOW CREATE DATABASE hv",
        "SHOW FUNCTION STATUS WHERE Db = 'hv'",
        "EXPLAIN FORMAT=TRADITIONAL SELECT 1",
        "DESCRIBE hv.customers 'e%'",
    ] {
        assert_eq!(classify_mysql(s).request_class, CoarseClass::Read, "{s}");
        assert_eq!(classify_mariadb(s).request_class, CoarseClass::Read, "{s}");
    }
    // The narrow grammars take no call, subquery or variable (P-M3b-35).
    for s in [
        "SHOW FUNCTION STATUS WHERE Db = SLEEP(1)",
        "SHOW FUNCTION STATUS WHERE Db IN (SELECT GET_LOCK('a', 1))",
        "SHOW FUNCTION STATUS WHERE Db = @x := 1",
        "DESC t SLEEP(1)",
    ] {
        let r = classify_mysql(s);
        assert_ne!(r.request_class, CoarseClass::Read, "{s}");
        assert!(read_only_denies(&r), "{s}");
    }
    // EXPLAIN of a write is not a read; EXPLAIN ANALYZE runs its statement.
    let r = classify_mysql("EXPLAIN DELETE FROM t");
    assert_eq!(r.request_class, CoarseClass::Write);
    assert!(
        destructive(&r).is_empty(),
        "plain EXPLAIN does not run the DELETE"
    );
    let r = classify_mysql("EXPLAIN FORMAT=TRADITIONAL DELETE FROM t");
    assert_eq!(r.request_class, CoarseClass::Write);
    let r = classify_mysql("EXPLAIN ANALYZE SELECT * FROM t");
    assert_eq!(r.items[0].kind, StmtKind::Explain { analyze: true });
    assert!(r.flags.contains(RiskFlags::EXPLAIN_ANALYZE));
    assert!(read_only_denies(&r));
    assert!(decide(&r, Profile::ReadOnlyPlusExplain, Actor::Human).allows());
    let r = classify_mysql("EXPLAIN ANALYZE DELETE FROM t");
    assert_eq!(r.request_class, CoarseClass::Write);
    assert_eq!(destructive(&r), [DestructiveKind::NoWhere]);
}

#[test]
fn mariadb_only_syntax_is_unknown() {
    for s in [
        "DELETE FROM t RETURNING *",
        "SELECT NEXT VALUE FOR s",
        "/*M!100000 DROP TABLE t */ SELECT 1",
    ] {
        assert_ne!(classify_mariadb(s).request_class, CoarseClass::Read, "{s}");
    }
    for s in [
        "SELECT * FROM t FOR SYSTEM_TIME AS OF '2026-01-01'",
        "SELECT * FROM t FOR SYSTEM_TIME ALL",
        "BEGIN NOT ATOMIC SELECT 1; END",
        "SELECT NEXT VALUE FOR s",
    ] {
        let r = classify_mariadb(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}");
        assert!(read_only_denies(&r), "{s}");
    }
    // MariaDB's `/*M!` runs on MariaDB; on a MySQL profile it is read as a view too (a MariaDB
    // server behind a MySQL profile would run it).
    let r = classify_mariadb("SELECT 1 /*M!100000 , GET_LOCK('a', 1) */ FROM DUAL");
    assert_eq!(r.request_class, CoarseClass::Write);
    let r = classify_mysql("SELECT 1 /*M!100000 , GET_LOCK('a', 1) */ FROM DUAL");
    assert_eq!(r.request_class, CoarseClass::Write);
    assert!(r.flags.contains(RiskFlags::EXEC_COMMENT));
    // A comment after the last token is not sent (the strict range): nothing to read.
    let r = classify_mysql("SELECT 1 /*M!100000 , GET_LOCK('a', 1) */");
    assert_eq!(r.items[0].range.len(), 8.into());
    // More MariaDB-only forms: none is a read.
    for s in [
        "SELECT s.nextval",
        "SELECT s.NEXTVAL FROM DUAL",
        "EXECUTE IMMEDIATE 'DELETE FROM t'",
        "SET STATEMENT max_statement_time = 1 FOR DELETE FROM t",
        "SET STATEMENT max_statement_time = 1 FOR SELECT 1",
        "DELETE HISTORY FROM t",
        "ANALYZE SELECT * FROM t",
        "CREATE SEQUENCE s",
        "SELECT * FROM t LIMIT ROWS EXAMINED 10",
        "SET sql_mode = 'ORACLE'",
    ] {
        let r = classify_mariadb(s);
        assert_ne!(r.request_class, CoarseClass::Read, "{s}");
        assert!(read_only_denies(&r), "{s}");
        assert_ne!(gate(&r), GateClass::Read, "{s}");
    }
    // `sql_mode=ORACLE` is a different grammar.
    let oracle = LexModes::from_mysql_sql_mode("ORACLE");
    let r = classify_interim(
        "SELECT 1",
        &ctx_with(DialectId::MariaDb, None, Some(oracle)),
    );
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(has(&r, ReasonCode::UnsupportedDialect));
}

#[test]
fn sqlite_pragma_allowlist() {
    let list: Vec<&str> = sqlite_read_pragmas().collect();
    assert_eq!(list.len(), 20);
    for p in &list {
        let r = classify_sqlite(&format!("PRAGMA {p}"));
        assert_eq!(r.request_class, CoarseClass::Read, "{p}");
        assert_eq!(r.items[0].kind, StmtKind::Pragma);
        let r = classify_sqlite(&format!("PRAGMA main.{}", p.to_ascii_uppercase()));
        assert_eq!(r.request_class, CoarseClass::Read, "{p}");
        // Every `=` form sets.
        let r = classify_sqlite(&format!("PRAGMA {p} = 1"));
        assert_ne!(r.request_class, CoarseClass::Read, "{p} = 1");
        assert!(read_only_denies(&r), "{p} = 1");
    }
    // With an argument, only the introspection pragmas read (a table or index name).
    for p in [
        "table_info",
        "table_xinfo",
        "index_list",
        "index_info",
        "index_xinfo",
        "foreign_key_list",
    ] {
        for arg in ["t", "'t'", "\"t\""] {
            let r = classify_sqlite(&format!("PRAGMA {p}({arg})"));
            assert_eq!(r.request_class, CoarseClass::Read, "{p}({arg})");
        }
    }
    for s in [
        "PRAGMA user_version(7)",
        "PRAGMA journal_mode(WAL)",
        "PRAGMA query_only(0)",
        "PRAGMA writable_schema",
        "PRAGMA integrity_check",
        "PRAGMA table_info(t, u)",
        "PRAGMA table_info(1 + 1)",
        "PRAGMA",
        "PRAGMA table_\u{130}nfo(t)",
    ] {
        let r = classify_sqlite(s);
        assert_ne!(r.request_class, CoarseClass::Read, "{s}");
        assert!(read_only_denies(&r), "{s}");
    }
}

#[derive(Debug, Deserialize)]
struct Gaps {
    version: u32,
    gap: Vec<Gap>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Gap {
    id: String,
    kind: String,
    dialects: Vec<String>,
    sql: String,
    #[serde(default)]
    sql_mode: Option<String>,
    expect: String,
    note: String,
}

fn class_named(s: &str) -> CoarseClass {
    CoarseClass::ALL
        .into_iter()
        .find(|c| format!("{c:?}") == s)
        .unwrap_or_else(|| panic!("class {s}"))
}

/// Every row of `known_gaps.toml` is classified as the row says, in every dialect it names;
/// the MariaDB-only forms (P-M3b-8) are never `Read` at the gate and always denied on
/// read-only.
#[test]
fn known_gaps_each_have_a_test() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/mysql/known_gaps.toml");
    let gaps: Gaps = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(gaps.version, 1);
    let mut ids: Vec<&str> = gaps.gap.iter().map(|g| g.id.as_str()).collect();
    ids.sort_unstable();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate gap ids");
    for g in &gaps.gap {
        assert!(!g.note.is_empty(), "{}", g.id);
        assert!(!g.dialects.is_empty(), "{}", g.id);
        for d in &g.dialects {
            let modes = g.sql_mode.as_deref().map(LexModes::from_mysql_sql_mode);
            let r = classify_interim(&g.sql, &ctx_with(dialect_named(d), None, modes));
            assert_contract(&r, &g.sql);
            assert_eq!(
                r.request_class,
                class_named(&g.expect),
                "{} on {d}: {r:?}",
                g.id
            );
            match g.kind.as_str() {
                "false-denial" => assert_ne!(r.request_class, CoarseClass::Read, "{}", g.id),
                "stricter" | "accepted-by-sqlparser" | "rejected-by-sqlparser" => {
                    assert_ne!(gate(&r), GateClass::Read, "{} on {d}", g.id);
                    assert!(read_only_denies(&r), "{} on {d}", g.id);
                }
                other => panic!("{}: kind {other}", g.id),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Hostile cases (the brief's list)

#[test]
fn write_forms_are_not_read() {
    for (s, effect) in [
        ("REPLACE INTO t VALUES (1)", EffectSet::WRITE_DATA),
        ("REPLACE t SET a = 1", EffectSet::WRITE_DATA),
        (
            "INSERT INTO t VALUES (1) ON DUPLICATE KEY UPDATE a = 1",
            EffectSet::WRITE_DATA,
        ),
        ("INSERT IGNORE INTO t VALUES (1)", EffectSet::WRITE_DATA),
        ("INSERT INTO t SELECT * FROM u", EffectSet::WRITE_DATA),
        (
            "LOAD DATA INFILE '/tmp/x' INTO TABLE t",
            EffectSet::EXTERNAL_IO,
        ),
        (
            "LOAD DATA LOCAL INFILE '/tmp/x' REPLACE INTO TABLE t",
            EffectSet::WRITE_DATA,
        ),
        (
            "LOAD XML INFILE '/tmp/x.xml' INTO TABLE t",
            EffectSet::EXTERNAL_IO,
        ),
        ("HANDLER t OPEN", EffectSet::SESSION_STATE),
        ("HANDLER t READ FIRST", EffectSet::SESSION_STATE),
        (
            "UPDATE t1 JOIN t2 ON t1.a = t2.a SET t1.b = 1",
            EffectSet::WRITE_DATA,
        ),
        (
            "DELETE t1 FROM t1 JOIN t2 ON t1.a = t2.a",
            EffectSet::WRITE_DATA,
        ),
    ] {
        for d in [DialectId::MySql, DialectId::MariaDb] {
            let r = run(d, s);
            assert_ne!(r.request_class, CoarseClass::Read, "{s}");
            assert!(r.request_effects.contains(effect), "{d:?} {s}: {r:?}");
            assert!(read_only_denies(&r), "{s}");
            assert_ne!(gate(&r), GateClass::Read, "{s}");
        }
    }
}

#[test]
fn select_into_file_and_locking_reads_are_not_read() {
    for (s, effect) in [
        (
            "SELECT * FROM t INTO OUTFILE '/tmp/x'",
            EffectSet::EXTERNAL_IO,
        ),
        (
            "SELECT a FROM t INTO DUMPFILE '/tmp/x'",
            EffectSet::EXTERNAL_IO,
        ),
        (
            "SELECT a INTO OUTFILE '/tmp/x' FROM t",
            EffectSet::EXTERNAL_IO,
        ),
        ("SELECT a FROM t INTO @v", EffectSet::SESSION_STATE),
        ("SELECT a INTO @v FROM t", EffectSet::SESSION_STATE),
        ("SELECT * FROM t FOR UPDATE", EffectSet::LOCK_ROW),
        (
            "SELECT * FROM t FOR UPDATE SKIP LOCKED",
            EffectSet::LOCK_ROW,
        ),
        ("SELECT * FROM t FOR SHARE", EffectSet::LOCK_ROW),
        ("SELECT * FROM t LOCK IN SHARE MODE", EffectSet::LOCK_ROW),
        (
            "SELECT * FROM (SELECT * FROM t FOR UPDATE) x",
            EffectSet::LOCK_ROW,
        ),
        (
            "SELECT a FROM t WHERE a IN (SELECT b FROM u FOR UPDATE)",
            EffectSet::LOCK_ROW,
        ),
    ] {
        let r = classify_mysql(s);
        assert_ne!(r.request_class, CoarseClass::Read, "{s}");
        assert!(r.request_effects.contains(effect), "{s}: {r:?}");
        assert!(read_only_denies(&r), "{s}");
    }
}

#[test]
fn set_global_and_persist_are_admin_and_session_sets_name_their_targets() {
    for s in [
        "SET @@global.max_connections = 1",
        "SET @@GLOBAL.max_connections = 1",
        "SET GLOBAL max_connections = 1",
        "SET PERSIST max_connections = 1",
        "SET PERSIST_ONLY max_connections = 1",
        "SET @@persist.max_connections = 1",
        "SET GLOBAL time_zone = '+00:00'",
        "SET time_zone = '+00:00', GLOBAL max_connections = 1",
    ] {
        let r = classify_mysql(s);
        assert!(r.request_effects.contains(EffectSet::ADMIN), "{s}: {r:?}");
        assert!(r.request_class >= CoarseClass::Admin, "{s}");
        assert!(read_only_denies(&r), "{s}");
        assert!(gate(&r) >= GateClass::Admin, "{s}");
    }
    // A session SET of harmless variables runs on read-only (P-M3b-6): one SessionState reason
    // per target, the targets with their values as written.
    let r = classify_mysql("SET time_zone = '+00:00', @@session.max_execution_time = 1000");
    let item = &r.items[0];
    assert_eq!(item.kind, StmtKind::Set);
    assert_eq!(item.effects, EffectSet::SESSION_STATE);
    let targets: Vec<(&str, Option<&str>)> = item
        .set_targets
        .iter()
        .map(|t| (&*t.name, t.value.as_deref()))
        .collect();
    assert_eq!(
        targets,
        [
            ("time_zone", Some("'+00:00'")),
            ("max_execution_time", Some("1000"))
        ]
    );
    assert!(!read_only_denies(&r));
    // The same harmless name twice: two targets, two reasons, still harmless.
    let r = classify_mysql("SET time_zone = '+00:00', time_zone = '+01:00'");
    assert_eq!(r.items[0].set_targets.len(), 2);
    assert!(!read_only_denies(&r));
    for s in [
        "SET autocommit = 0",
        "SET @a = 1",
        "SET @a := SLEEP(1)",
        "SET time_zone = '+00:00', sql_safe_updates = 0",
        "SET SESSION TRANSACTION READ WRITE",
        "SET @@transaction_read_only = 0",
        "SET sql_mode = ''",
        "SET NAMES utf8mb4",
        "SET CHARACTER SET latin1",
        "SET ROLE ALL",
        "SET PASSWORD = 'x'",
        "SET DEFAULT ROLE r TO u",
        "SET time_zone = my_fn()",
        "SET time_zone = (SELECT SLEEP(1))",
        "SET time_zone = '+00:00' /*!50000 , GLOBAL max_connections = 1 */",
    ] {
        let r = classify_mysql(s);
        assert!(read_only_denies(&r), "{s}: {r:?}");
        assert_ne!(gate(&r), GateClass::Read, "{s}");
    }
    let r = classify_mysql("SET sql_mode = 'ANSI_QUOTES'");
    assert!(r.flags.contains(RiskFlags::LEX_MODE_CHANGE));
    assert!(has(&r, ReasonCode::LexModeChange));
    let r = classify_mysql("SET ROLE r");
    assert!(has(&r, ReasonCode::PrivilegeChange));
    let r = classify_mysql("GRANT SELECT ON hv.* TO u");
    assert!(
        has(&r, ReasonCode::PrivilegeGrant) || r.request_effects.contains(EffectSet::PRIVILEGE)
    );
    assert!(!has(&r, ReasonCode::PrivilegeChange));
}

#[test]
fn call_do_prepare_execute_are_not_read() {
    for s in [
        "CALL p()",
        "CALL db.p(1, 'x')",
        "DO SLEEP(1)",
        "DO 1",
        "PREPARE s FROM 'DELETE FROM t'",
        "PREPARE s FROM @q",
        "EXECUTE s",
        "EXECUTE s USING @a",
        "DEALLOCATE PREPARE s",
        "EXECUTE IMMEDIATE 'DELETE FROM t'",
    ] {
        for d in [DialectId::MySql, DialectId::MariaDb] {
            let r = run(d, s);
            assert_ne!(r.request_class, CoarseClass::Read, "{s}");
            assert!(read_only_denies(&r), "{s}");
            assert!(gate(&r) >= GateClass::Session, "{s}");
        }
    }
    let r = classify_mysql("CALL p()");
    assert_eq!(r.items[0].kind, StmtKind::Call);
    assert!(r.request_effects.contains(EffectSet::ROUTINE_CALL));
}

#[test]
fn sqlite_attach_detach_pragma_writes_and_extensions_are_not_read() {
    for s in [
        "ATTACH 'x.db' AS x",
        "ATTACH DATABASE 'file:x.db?mode=rwc' AS x",
        "DETACH x",
        "DETACH DATABASE x",
        "PRAGMA query_only = 0",
        "PRAGMA writable_schema = 1",
        "PRAGMA user_version = 7",
        "PRAGMA main.journal_mode = WAL",
        "PRAGMA journal_mode = 'DELETE'",
        "PRAGMA foreign_keys = OFF",
        "SELECT load_extension('evil')",
        "SELECT LOAD_EXTENSION('evil', 'entry')",
        "SELECT \"load_extension\"('evil')",
        "VACUUM",
        "VACUUM INTO '/tmp/copy.db'",
        "VACUUM main INTO 'copy.db'",
        "CREATE VIRTUAL TABLE v USING fts5(a)",
        "CREATE VIRTUAL TABLE v USING csv(filename = '/etc/passwd')",
        "REINDEX",
        "ANALYZE",
        "INSERT OR REPLACE INTO t VALUES (1)",
        "INSERT INTO t VALUES (1) RETURNING *",
        "SELECT * FROM t WHERE a = (SELECT writefile('/tmp/x', 'y'))",
    ] {
        let r = classify_sqlite(s);
        assert_ne!(r.request_class, CoarseClass::Read, "{s}");
        assert!(read_only_denies(&r), "{s}: {r:?}");
        assert_ne!(gate(&r), GateClass::Read, "{s}");
    }
}

#[test]
fn turkish_identifiers_read_and_lookalikes_never_match() {
    for s in [
        "SELECT * FROM `müşteriler` WHERE şehir = 'İstanbul'",
        "SELECT ad AS müşteri_adı, ıslak FROM ürünler",
        "SELECT `İ`, `ı` FROM `çizelge`",
    ] {
        assert_eq!(classify_mysql(s).request_class, CoarseClass::Read, "{s}");
        assert_eq!(classify_mariadb(s).request_class, CoarseClass::Read, "{s}");
    }
    for s in [
        "SELECT * FROM müşteriler WHERE şehir = 'İstanbul'",
        "SELECT \"ürün\", [ölçü] FROM \"sipariş\"",
    ] {
        assert_eq!(classify_sqlite(s).request_class, CoarseClass::Read, "{s}");
    }
    // `LOAD_FİLE` (U+0130) is not `LOAD_FILE` and not a builtin either: unknown, not read.
    let r = classify_mysql("SELECT LOAD_F\u{130}LE('/etc/passwd')");
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(has(&r, ReasonCode::FnUnknown));
    assert!(!has(&r, ReasonCode::FnDenylisted));
    // A Turkish-cased keyword is a word, not `LIMIT`: the statement does not parse.
    let r = classify_mysql("SELECT * FROM t L\u{130}M\u{130}T 1");
    assert_ne!(r.request_class, CoarseClass::Read);
    // `SLEEP` reached through a dotless-i look-alike of nothing: still denied by bare name
    // in any case and quoting.
    for s in ["SELECT sleep(1)", "SELECT `SLEEP`(1)", "SELECT db.Sleep(1)"] {
        let r = classify_mysql(s);
        assert!(has(&r, ReasonCode::FnDenylisted), "{s}");
        assert!(read_only_denies(&r), "{s}");
    }
}

#[test]
fn client_commands_and_unreadable_tokens_are_unknown() {
    for s in [
        "DELIMITER //",
        "SOURCE /tmp/x.sql",
        "SYSTEM rm -rf /",
        "SELECT 1 \\G",
        "SELECT 1 \u{a0}FROM t",
        "SELECT 'unterminated",
    ] {
        let r = classify_mysql(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}: {r:?}");
        assert!(read_only_denies(&r), "{s}");
    }
    let r = classify_interim(
        "DELIMITER //\nSELECT 1//",
        &ClassifyCtx::new(
            DialectCtx::new(DialectId::MySql, None),
            ClientFlavor::MySqlCli,
            Expect::Script,
        ),
    );
    assert_eq!(r.request_class, CoarseClass::Unknown);
    let r = classify_sqlite(".tables");
    assert_eq!(r.request_class, CoarseClass::Unknown);
}

#[test]
fn hints_are_judged() {
    let r = classify_mysql("SELECT /*+ MAX_EXECUTION_TIME(0) */ 1");
    assert!(has(&r, ReasonCode::HintExecTime));
    assert!(read_only_denies(&r));
    let r = classify_mysql("SELECT /*+ MAX_EXECUTION_TIME(1000) */ 1");
    assert!(!read_only_denies(&r));
    let r = classify_mysql("SELECT /*+ SET_VAR(transaction_read_only = 0) */ 1");
    assert!(has(&r, ReasonCode::HintSetVar));
    assert!(read_only_denies(&r));
    let r = classify_mysql("SELECT /*+ SET_VAR(time_zone = '+00:00') */ 1");
    assert!(!read_only_denies(&r));
    let r = classify_mysql("SELECT /*+ SET_VAR(`x` = 1) */ 1");
    assert!(read_only_denies(&r));
}

#[test]
fn destructive_patterns_are_named() {
    let cases: [(&str, &[DestructiveKind]); 10] = [
        ("DELETE FROM t", &[DestructiveKind::NoWhere]),
        ("DELETE FROM t WHERE 1", &[DestructiveKind::AlwaysTrue]),
        (
            "DELETE FROM t WHERE TRUE OR a = 2",
            &[DestructiveKind::AlwaysTrue],
        ),
        (
            "UPDATE t SET a = 1 WHERE 1 = 1",
            &[DestructiveKind::AlwaysTrue],
        ),
        (
            "UPDATE t SET a = 1 LIMIT 3",
            &[
                DestructiveKind::NoWhere,
                DestructiveKind::UpdateLimitNoOrder,
            ],
        ),
        ("UPDATE t SET a = 1 WHERE b = 2 ORDER BY c LIMIT 3", &[]),
        ("DROP TABLE t", &[DestructiveKind::Drop { cascade: false }]),
        ("TRUNCATE TABLE t", &[DestructiveKind::Truncate]),
        (
            "ALTER TABLE t DROP COLUMN a",
            &[DestructiveKind::DropColumn],
        ),
        (
            "ALTER TABLE t MODIFY a BIGINT",
            &[DestructiveKind::AlterType],
        ),
    ];
    for (s, kinds) in cases {
        let r = classify_mysql(s);
        assert_eq!(destructive(&r), kinds, "{s}");
    }
    let r = classify_mysql("DELETE FROM t");
    assert_eq!(gate(&r), GateClass::DmlUnsafe);
    let r = classify_mysql("DROP TABLE t");
    assert_eq!(gate(&r), GateClass::DdlDestructive);
}

#[derive(Debug)]
struct Catalog;

impl FnCatalog for Catalog {
    fn lookup(&self, f: &QualifiedName, _arity: Option<u16>) -> FnLookup {
        match &*f.parts.last().unwrap().text {
            "stable_fn" => FnLookup::Known {
                volatile: false,
                writes: false,
            },
            "volatile_fn" => FnLookup::Known {
                volatile: true,
                writes: false,
            },
            "writing_fn" => FnLookup::Known {
                volatile: false,
                writes: true,
            },
            _ => FnLookup::Unknown,
        }
    }
}

#[test]
fn functions_go_through_deny_allow_allowlist_and_catalog() {
    // A user function without a catalog: unknown (S-04 without metadata).
    let r = classify_mysql("SELECT my_fn(1)");
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(has(&r, ReasonCode::FnUnknown));
    // On the profile's allowlist (D-009): a conservative read.
    let mut ctx = ctx_with(DialectId::MySql, None, None);
    ctx.fn_allowlist = Arc::from(vec![Arc::from("my_fn"), Arc::from("sleep")]);
    let r = classify_interim("SELECT MY_FN(1)", &ctx);
    assert_eq!(r.request_class, CoarseClass::Read);
    assert_eq!(r.items[0].confidence, Confidence::Conservative);
    // The deny list wins over the allowlist.
    let r = classify_interim("SELECT SLEEP(1)", &ctx);
    assert!(has(&r, ReasonCode::FnDenylisted));
    assert!(read_only_denies(&r));
    // The catalog: stable reads, volatile is Unknown with VOLATILE_FN, writing is Unknown.
    let mut ctx = ctx_with(DialectId::MySql, None, None);
    ctx.catalog = Some(Arc::new(Catalog));
    let r = classify_interim("SELECT stable_fn(a) FROM t", &ctx);
    assert_eq!(r.request_class, CoarseClass::Read);
    let r = classify_interim("SELECT volatile_fn(a) FROM t", &ctx);
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(r.flags.contains(RiskFlags::VOLATILE_FN));
    assert!(has(&r, ReasonCode::VolatileFunction));
    let r = classify_interim("SELECT writing_fn(a) FROM t", &ctx);
    assert_eq!(r.request_class, CoarseClass::Unknown);
    // Below the probe floor a builtin is not proven (Task 3).
    let r = classify_interim(
        "SELECT ABS(1)",
        &ctx_with(DialectId::MySql, Some(ServerVersion::new(5, 7, 0)), None),
    );
    assert_eq!(r.request_class, CoarseClass::Unknown);
    let r = classify_interim(
        "SELECT ABS(1)",
        &ctx_with(DialectId::MySql, Some(ServerVersion::new(8, 4, 0)), None),
    );
    assert_eq!(r.request_class, CoarseClass::Read);
    // Calls hidden in table functions, nested subqueries, VALUES, ORDER BY and CTEs are found
    // (the CVE-2026-85620 shapes).
    for s in [
        "SELECT * FROM (SELECT * FROM (SELECT GET_LOCK('a', 1)) x) y",
        "WITH c AS (SELECT GET_LOCK('a', 1)) SELECT * FROM c",
        "SELECT a FROM t ORDER BY (SELECT GET_LOCK('a', 1))",
        "SELECT a FROM t WHERE EXISTS (SELECT 1 FROM u WHERE GET_LOCK('a', 1))",
        "SELECT a FROM t GROUP BY a HAVING GET_LOCK('a', 1)",
        "SELECT * FROM JSON_TABLE(GET_LOCK('a', 1), '$' COLUMNS (v INT PATH '$')) j",
    ] {
        let r = classify_mysql(s);
        assert!(r.request_effects.contains(EffectSet::ADVISORY_LOCK), "{s}");
        assert!(read_only_denies(&r), "{s}");
    }
}

#[test]
fn data_modifying_ctes_and_nested_writes_are_writes() {
    // sqlparser accepts a DML CTE on MySqlDialect; MySQL rejects it, but it is never a read.
    let r = classify_mysql("WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d");
    assert_ne!(r.request_class, CoarseClass::Read);
    assert!(read_only_denies(&r));
    let r = classify_sqlite("WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x");
    assert_eq!(r.request_class, CoarseClass::Write);
}

/// No `sqlparser` type leaves the backend: the only public item `backend/` exposes is
/// `classify_interim(&str, &ClassifyCtx) -> SafetyReport` (the pub-api grep, R-M3-54).
#[test]
fn no_sqlparser_type_in_the_interim_public_api() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in ["backend/interim.rs", "backend/prescan.rs"] {
        let text = std::fs::read_to_string(src.join(file)).unwrap();
        let public: Vec<&str> = text
            .lines()
            .map(str::trim_start)
            .filter(|l| l.starts_with("pub ") || l.starts_with("pub fn"))
            .collect();
        for l in &public {
            assert!(!l.contains("sqlparser"), "{file}: {l}");
        }
        if file == "backend/interim.rs" {
            assert_eq!(
                public,
                ["pub fn classify_interim(sql: &str, ctx: &ClassifyCtx) -> SafetyReport {"]
            );
        } else {
            assert!(public.is_empty(), "{file}: {public:?}");
        }
    }
    let lib = std::fs::read_to_string(src.join("lib.rs")).unwrap();
    let reexports: Vec<&str> = lib.lines().filter(|l| l.contains("backend::")).collect();
    // m3b merge (T9 + T10): the PG backend's hidden test hooks are the other re-export; neither
    // carries a sqlparser type (checked per file above).
    assert_eq!(
        reexports,
        [
            "pub use backend::interim::classify_interim;",
            "pub use backend::pg::hooks as pg_backend_hooks;"
        ]
    );
}

/// Every statement table row names a real kind, effect, reason and flag (they load), and the
/// table's kind is the report's kind.
#[test]
fn statement_tables_load() {
    for (d, s, kind) in [
        (DialectId::MySql, "SELECT 1", StmtKind::Select),
        (DialectId::MySql, "SHOW TABLES", StmtKind::Show),
        (
            DialectId::MySql,
            "INSERT INTO t VALUES (1)",
            StmtKind::Insert,
        ),
        (DialectId::MySql, "TRUNCATE t", StmtKind::Truncate),
        (
            DialectId::MySql,
            "CREATE INDEX i ON t (a)",
            StmtKind::CreateOther,
        ),
        (DialectId::MySql, "LOCK TABLES t READ", StmtKind::Lock),
        (DialectId::MySql, "START TRANSACTION", StmtKind::Begin),
        (DialectId::MySql, "USE hv", StmtKind::Use),
        (DialectId::MySql, "KILL 3", StmtKind::Other),
        (DialectId::Sqlite, "SELECT 1", StmtKind::Select),
        (DialectId::Sqlite, "BEGIN IMMEDIATE", StmtKind::Begin),
        (
            DialectId::Sqlite,
            "CREATE TABLE t (a)",
            StmtKind::CreateTable,
        ),
    ] {
        let r = run(d, s);
        assert_eq!(r.items[0].kind, kind, "{s}");
    }
    // A statement with no table row is never a read.
    let r = classify_mysql("CREATE ROLE r");
    assert_ne!(r.request_class, CoarseClass::Read);
    let r = classify_sqlite("CREATE INDEX i ON t (a)");
    assert_eq!(r.request_class, CoarseClass::Ddl);
}

#[test]
fn multi_statement_and_empty_inputs_are_unknown() {
    for s in ["SELECT 1; SELECT 2", "SELECT 1; DELETE FROM t", ""] {
        let r = classify_mysql(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s:?}");
        assert!(read_only_denies(&r), "{s:?}");
    }
    let r = classify_mysql("-- only a comment");
    assert_eq!(r.request_class, CoarseClass::Unknown);
    // A script of reads (Expect::Script) is a read.
    let r = classify_interim(
        "SELECT 1; SELECT 2",
        &ClassifyCtx::new(
            DialectCtx::new(DialectId::MySql, None),
            ClientFlavor::None,
            Expect::Script,
        ),
    );
    assert_eq!(r.request_class, CoarseClass::Read);
    assert_eq!(r.items.len(), 2);
}

#[test]
fn unsupported_dialects_are_unknown() {
    let r = classify_interim(
        "SELECT 1",
        &ClassifyCtx::new(
            DialectCtx::new(DialectId::Postgres, None),
            ClientFlavor::None,
            Expect::SingleStatement,
        ),
    );
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(has(&r, ReasonCode::UnsupportedDialect));
}

#[test]
fn deep_nesting_is_too_complex_not_a_panic() {
    let deep = format!("SELECT {}1{}", "(".repeat(200), ")".repeat(200));
    let r = classify_mysql(&deep);
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(has(&r, ReasonCode::TooComplex) || has(&r, ReasonCode::ParseError));
}

// ---------------------------------------------------------------------------------------------
// Fix round 1

fn rw_dml_allows(r: &SafetyReport) -> bool {
    decide(r, Profile::ReadWriteDml, Actor::Human).allows()
}

/// C1: every executable comment the server may skip is also read skipped. A MariaDB `/*M!`
/// without digits on a MySQL profile is skipped by MySQL, so the reading where it is a comment
/// (and the quote inside it pairs nothing) is classified.
#[test]
fn skipped_readings_of_executable_comments_are_classified() {
    // The reviewer's repro: MySQL runs `SELECT 1 INTO OUTFILE "/tmp/x" FROM DUAL`.
    let r = classify_mysql("SELECT 1 /*M! ' */ INTO OUTFILE \"/tmp/x\" -- '\n FROM DUAL");
    assert_ne!(r.request_class, CoarseClass::Read, "{r:?}");
    assert!(r.request_effects.contains(EffectSet::EXTERNAL_IO), "{r:?}");
    assert!(!rw_dml_allows(&r));
    assert!(read_only_denies(&r));
    // The UDF variant: MySQL calls an unknown routine.
    let r = classify_mysql("SELECT /*M! ' */ my_write_udf() -- '\n AS x");
    assert_eq!(r.request_class, CoarseClass::Unknown, "{r:?}");
    assert!(has(&r, ReasonCode::FnUnknown));
    assert!(!rw_dml_allows(&r));
    // A versioned `/*!` with an unknown version, below it, and above it: the skipped reading
    // is classified in every case.
    for version in [
        None,
        Some(ServerVersion::new(5, 7, 0)),
        Some(ServerVersion::new(8, 4, 0)),
    ] {
        let sql = "SELECT 1 /*!80000 ' */ INTO OUTFILE '/tmp/x' -- '\n FROM DUAL";
        let r = classify_interim(sql, &ctx_with(DialectId::MySql, version, None));
        assert_contract(&r, sql);
        assert!(
            r.request_effects.contains(EffectSet::EXTERNAL_IO),
            "{version:?}: {r:?}"
        );
        assert!(!rw_dml_allows(&r), "{version:?}");
    }
    // An unversioned `/*!` always runs: no skipped reading is needed.
    let r = classify_mysql("SELECT 1 /*! , GET_LOCK('a', 1) */ FROM DUAL");
    assert_eq!(r.request_class, CoarseClass::Write);
}

/// C2: past `max_exec_comment_views` readings the statement is `TooComplex` at once, for any
/// count (no overflow, no loop over readings), in debug and in release.
#[test]
fn too_many_comment_readings_fail_closed_fast() {
    for n in [62usize, 63, 64, 200] {
        let sql = format!("SELECT 1{}", " /*!50000 # */\n */".repeat(n));
        let start = std::time::Instant::now();
        let r = classify_mysql(&sql);
        let took = start.elapsed();
        assert_eq!(r.request_class, CoarseClass::Unknown, "{n}");
        assert!(has(&r, ReasonCode::TooComplex), "{n}: {r:?}");
        assert!(took < std::time::Duration::from_secs(2), "{n}: {took:?}");
    }
    // Many distinct gates, and six-digit gates (two readings each).
    let gates: String = (0..200)
        .map(|k| format!(" /*!{} , {k} */", 50000 + k))
        .collect();
    assert!(has(
        &classify_mysql(&format!("SELECT 1{gates}")),
        ReasonCode::TooComplex
    ));
    let six: String = (0..8)
        .map(|k| format!(" /*!{} , {k} */", 800000 + k))
        .collect();
    assert!(has(
        &classify_mysql(&format!("SELECT 1{six}")),
        ReasonCode::TooComplex
    ));
}

/// M7: a six-digit gate on MySQL is read as five digits plus a body digit and as six digits.
#[test]
fn six_digit_mysql_gates_read_every_threshold() {
    // As five digits (8.0.0) the body is `0 , GET_LOCK(…)`, which does not parse; as six
    // digits (80.0.0) it is `, GET_LOCK(…)`: the union holds the lock.
    let r = classify_mysql("SELECT 1 /*!800000 , GET_LOCK('a', 1) */ FROM DUAL");
    assert!(
        r.request_effects.contains(EffectSet::ADVISORY_LOCK),
        "{r:?}"
    );
    assert_eq!(r.request_class, CoarseClass::Unknown);
}

/// I1: a `SET` the grammar cannot read is Unknown with a doubt reason; global and persisted
/// scopes are Admin with each target named.
#[test]
fn set_scopes_are_admin_and_unreadable_sets_are_unknown() {
    let cases: [(&str, &[&str]); 5] = [
        (
            "SET @@global.validate_password.length = 4",
            &["validate_password.length"],
        ),
        (
            "SET @@persist.validate_password.length = 4",
            &["validate_password.length"],
        ),
        (
            "SET @@persist_only.max_connections = 4",
            &["max_connections"],
        ),
        ("SET @@global.x = 1, @@global.y.z = 2", &["x", "y.z"]),
        (
            "SET GLOBAL validate_password.length = 4",
            &["validate_password.length"],
        ),
    ];
    for (s, names) in cases {
        let r = classify_mysql(s);
        assert!(r.request_effects.contains(EffectSet::ADMIN), "{s}: {r:?}");
        assert!(r.request_class >= CoarseClass::Admin, "{s}");
        let got: Vec<&str> = r.items[0].set_targets.iter().map(|t| &*t.name).collect();
        assert_eq!(got, names, "{s}");
        assert!(read_only_denies(&r) && !rw_dml_allows(&r), "{s}");
    }
    // Unreadable: Unknown, a doubt reason, the targets read so far kept, ADMIN when a global
    // scope appears anywhere.
    for (s, admin) in [
        ("SET STATEMENT max_statement_time = 1 FOR SELECT 1", false),
        ("SET time_zone = '+00:00', GLOBAL", true),
        ("SET time_zone = '+00:00', @@global.", true),
        ("SET @@persist.", true),
        ("SET = 1", false),
    ] {
        let r = classify_mysql(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}: {r:?}");
        assert!(has(&r, ReasonCode::ParseError), "{s}");
        assert!(r.request_effects.contains(EffectSet::UNKNOWN), "{s}");
        assert_eq!(r.request_effects.contains(EffectSet::ADMIN), admin, "{s}");
        assert!(read_only_denies(&r), "{s}");
    }
    let r = classify_mysql("SET time_zone = '+00:00', GLOBAL");
    assert_eq!(r.items[0].set_targets.len(), 1);
}

/// I2: SAFE-07 `AlwaysTrue` over constant predicates (`alg/13` :529, UNSAFE-01..07, 09).
#[test]
fn always_true_where_is_destructive() {
    for d in [DialectId::MySql, DialectId::Sqlite] {
        for w in [
            "NULL IS NULL",
            "NOT FALSE",
            "TRUE",
            "1 = 1",
            "'a' = 'a'",
            "1 + 1 = 2",
            "id = 7 OR TRUE",
            "id = 7 OR 1 = 1",
            "NOT (1 = 0)",
            "'x' IS NOT NULL",
            "(TRUE)",
            "1",
            "2 > 1",
            // ## m3b-t26-prod-unlock (fix round 2, A R1-m1: the fail-closed rule): a WHERE that
            // reads no column of the statement's tables is constant, so it counts as keeping
            // every row (these three were bounded before), as does `id = id`.
            "1 = 0",
            "NULL",
            "NULL = NULL",
            "id = id",
        ] {
            for s in [
                format!("DELETE FROM audit_log WHERE {w}"),
                format!("UPDATE customers SET city = 'x' WHERE {w}"),
            ] {
                let r = run(d, &s);
                assert_eq!(destructive(&r), [DestructiveKind::AlwaysTrue], "{d:?}: {s}");
                assert_eq!(gate(&r), GateClass::DmlUnsafe, "{s}");
                assert!(!rw_dml_allows(&r), "{s}: unbounded writes confirm");
            }
        }
        for w in [
            "id = 1",
            "id = 1 AND TRUE",
            "id IS NULL",
            "FALSE OR id = 2",
            "city = 'a'",
            "id IN (SELECT 1)",
        ] {
            let s = format!("DELETE FROM audit_log WHERE {w}");
            assert!(destructive(&run(d, &s)).is_empty(), "{d:?}: {s}");
        }
    }
    // UNSAFE-01, -04, -05, -07, -09.
    assert_eq!(
        destructive(&classify_mysql("DELETE FROM audit_log")),
        [DestructiveKind::NoWhere]
    );
    let r = classify_mysql("DELETE o FROM orders o JOIN customers c");
    assert!(
        destructive(&r).contains(&DestructiveKind::JoinWithoutOn),
        "{r:?}"
    );
    let r = classify_mysql("UPDATE orders o JOIN customers c SET o.note = 'x' WHERE o.id = 1");
    assert!(
        destructive(&r).contains(&DestructiveKind::JoinWithoutOn),
        "{r:?}"
    );
    let r = classify_mysql("WITH x AS (SELECT 1) DELETE FROM audit_log");
    assert_eq!(destructive(&r), [DestructiveKind::NoWhere]);
    assert_eq!(
        r.items[0].kind,
        StmtKind::Delete,
        "M5: the kind is the body's"
    );
    assert_eq!(
        destructive(&classify_mysql("TRUNCATE audit_log")),
        [DestructiveKind::Truncate]
    );
    assert!(
        destructive(&classify_mysql(
            "UPDATE customers SET city = NULL WHERE id = 1"
        ))
        .is_empty()
    );
    // Under ANSI_QUOTES `"a" = "a"` compares a column with itself: not a constant.
    let r = classify_with_modes(mysql_ansi(), r#"DELETE FROM t WHERE "a" = "a""#);
    assert!(destructive(&r).is_empty(), "{r:?}");
}

/// I3 (controller ruling): information disclosure is Unknown and denied on read-only.
#[test]
fn information_disclosure_is_unknown() {
    for s in [
        "SHOW PROCESSLIST",
        "SHOW FULL PROCESSLIST",
        "SHOW ENGINE INNODB STATUS",
        "SHOW ENGINE INNODB MUTEX",
        "SHOW REPLICA STATUS",
        "SHOW SLAVE STATUS",
        "SHOW MASTER STATUS",
        "SHOW BINARY LOG STATUS",
        "SHOW ALL SLAVES STATUS",
        "SHOW REPLICAS",
        "SHOW SLAVE HOSTS",
        "SHOW BINLOG EVENTS",
        "SHOW RELAYLOG EVENTS",
        "SHOW GRANTS FOR 'root'@'%'",
        "SHOW GRANTS FOR other",
        "SHOW EXPLAIN FOR 1",
        "SHOW ANALYZE FOR 1",
    ] {
        for d in [DialectId::MySql, DialectId::MariaDb] {
            let r = run(d, s);
            assert_eq!(r.request_class, CoarseClass::Unknown, "{d:?}: {s}");
            assert!(has(&r, ReasonCode::EffectsUnknown), "{s}");
            assert!(read_only_denies(&r), "{s}");
        }
    }
    for s in [
        "SHOW GRANTS",
        "SHOW GRANTS FOR CURRENT_USER",
        "SHOW GRANTS FOR CURRENT_USER()",
        "SHOW ENGINES",
        "SHOW STATUS",
    ] {
        assert_eq!(classify_mysql(s).request_class, CoarseClass::Read, "{s}");
    }
}

/// M1: `CALL` of a builtin's name is a procedure, never the builtin: Unknown with evidence.
#[test]
fn call_is_unknown_with_evidence() {
    for s in ["CALL abs(1)", "CALL now()", "CALL p()", "CALL sleep(1)"] {
        let r = classify_mysql(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}");
        assert!(r.request_effects.contains(EffectSet::UNKNOWN), "{s}");
        assert!(has(&r, ReasonCode::FnUnknown), "{s}");
    }
    assert!(has(
        &classify_mysql("CALL sleep(1)"),
        ReasonCode::FnDenylisted
    ));
}

/// M3: MariaDB's quoted pseudo-column.
#[test]
fn quoted_nextval_pseudo_column_is_a_sequence() {
    let r = classify_mariadb("SELECT s.`nextval`");
    assert!(r.request_effects.contains(EffectSet::SEQUENCE), "{r:?}");
}

// ---------------------------------------------------------------------------------------------
// Fix round 2

/// RR-I1 and the same-category gaps: the remaining `SHOW` routes to other sessions, accounts
/// and the binary log are Unknown and denied on read-only.
#[test]
fn information_disclosure_show_forms_round_2() {
    let both = [
        "SHOW SLAVE 'conn1' STATUS",
        "SHOW REPLICA 'conn1' STATUS",
        "SHOW REPLICA STATUS FOR CHANNEL c1",
        "SHOW BINARY LOGS",
        "SHOW MASTER LOGS",
        "SHOW CREATE USER other",
        "SHOW CREATE USER 'root'@'%'",
    ];
    let mariadb = [
        "SHOW EXPLAIN FORMAT=JSON FOR 1",
        "SHOW ANALYZE FORMAT=JSON FOR 1",
        "SHOW EXPLAIN FOR 1",
        "SHOW USER_STATISTICS",
        "SHOW CLIENT_STATISTICS",
    ];
    for (d, list) in [
        (DialectId::MySql, &both[..]),
        (DialectId::MariaDb, &both[..]),
        (DialectId::MariaDb, &mariadb[..]),
    ] {
        for s in list {
            let r = run(d, s);
            assert_eq!(r.request_class, CoarseClass::Unknown, "{d:?}: {s}");
            assert!(has(&r, ReasonCode::EffectsUnknown), "{d:?}: {s}");
            assert!(read_only_denies(&r), "{d:?}: {s}");
        }
    }
}

/// RR-I2 (controller ruling): the table routes to the same data, in any qualification,
/// quoting or letter case, and wherever they stand in the statement.
#[test]
fn information_disclosure_tables_are_unknown() {
    let routes = [
        "SELECT * FROM information_schema.PROCESSLIST",
        "SELECT * FROM INFORMATION_SCHEMA.processlist",
        "SELECT * FROM `information_schema`.`PROCESSLIST`",
        "SELECT * FROM information_schema . processlist",
        "SELECT id, info FROM information_schema.processlist WHERE id <> CONNECTION_ID()",
        "SELECT * FROM performance_schema.threads",
        "SELECT * FROM `performance_schema`.`threads`",
        "SELECT * FROM performance_schema.events_statements_current",
        "SELECT * FROM performance_schema.events_statements_history",
        "SELECT * FROM performance_schema.events_statements_history_long",
        "SELECT * FROM sys.processlist",
        "SELECT * FROM sys.session",
        "SELECT * FROM sys.`x$processlist`",
        "SELECT * FROM sys.x$session",
        "SELECT * FROM SYS.SESSION",
        "SELECT * FROM processlist",
        "SELECT * FROM events_statements_current",
        "SELECT 1 WHERE EXISTS (SELECT 1 FROM performance_schema.threads)",
        "SELECT c.id FROM customers c JOIN information_schema.processlist p ON p.id = c.id",
        "WITH x AS (SELECT * FROM sys.processlist) SELECT * FROM x",
        "SELECT * FROM (SELECT info FROM information_schema.processlist) AS d",
    ];
    for s in routes {
        for d in [DialectId::MySql, DialectId::MariaDb] {
            let r = run(d, s);
            assert_eq!(r.request_class, CoarseClass::Unknown, "{d:?}: {s}");
            assert!(has(&r, ReasonCode::EffectsUnknown), "{d:?}: {s}");
            assert!(read_only_denies(&r), "{d:?}: {s}");
        }
    }
    for s in [
        "SELECT * FROM information_schema.USER_STATISTICS",
        "SELECT * FROM information_schema.client_statistics",
    ] {
        let r = classify_mariadb(s);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{s}");
        assert!(read_only_denies(&r), "{s}");
    }
    // Not routes: other catalog tables, a user table named `threads` or `session`, text.
    for s in [
        "SELECT * FROM information_schema.tables",
        "SELECT * FROM performance_schema.global_status",
        "SELECT * FROM threads",
        "SELECT * FROM session",
        "SELECT 'information_schema.processlist'",
        "SHOW TABLES FROM performance_schema LIKE 'threads'",
    ] {
        assert_eq!(classify_mysql(s).request_class, CoarseClass::Read, "{s}");
    }
}
