//! The SQLite splitter against SQLite itself (Task 8): `rusqlite::Batch` prepares one statement
//! at a time and continues with the tail, so the engine's own parser draws every boundary. Over
//! scripts on a small seed schema, execution mode must find the same statements (as strict
//! texts) and no uncertain boundary.

use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::{Batch, Connection};
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, lex_profile};
use sql_lexer::{LexState, TokenKind, lex};
use sql_split::{SplitMode, SplitStart, split_from};

const SEED: &str = "CREATE TABLE orders (id INTEGER PRIMARY KEY, note TEXT, total REAL);
CREATE TABLE audit_log (actor TEXT, message TEXT);
CREATE TABLE müşteri (ad TEXT, şehir TEXT);
CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT);";

fn sqlite() -> DialectCtx {
    DialectCtx::new(DialectId::Sqlite, None)
}

fn strict_texts_sqlite(script: &str) -> Vec<String> {
    let s = split_from(
        script,
        &sqlite(),
        ClientFlavor::Sqlite3,
        SplitMode::Execution,
        &SplitStart::default(),
    );
    assert!(s.ambiguous.is_empty(), "{script:?}: {:?}", s.ambiguous);
    s.statements()
        .map(|st| script[st.strict].to_string())
        .collect()
}

/// A statement's text as SQLite reports it, trimmed to its first and last significant tokens
/// (no leading comments or empty `;` statements, which SQLite folds into the next statement's
/// text, and no terminating `;`): the strict-range convention.
fn strict_of(sql: &str) -> String {
    let ctx = sqlite();
    let lexed = lex(
        sql,
        lex_profile(&ctx).unwrap(),
        ctx.modes,
        &LexState::Initial,
    );
    let sig: Vec<usize> = (0..lexed.len())
        .filter(|i| !lexed.is_trivia(*i))
        .skip_while(|i| lexed.kind(*i) == TokenKind::Semicolon)
        .collect();
    let mut last = *sig.last().expect("a prepared statement has a token");
    if lexed.kind(last) == TokenKind::Semicolon && sig.len() > 1 {
        last = sig[sig.len() - 2];
    }
    let start = usize::from(lexed.range(sig[0]).start());
    let end = usize::from(lexed.range(last).end());
    sql[start..end].to_string()
}

fn rusqlite_batch_texts(script: &str) -> Vec<String> {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SEED).unwrap();
    let mut batch = Batch::new(&conn, script);
    let mut texts = Vec::new();
    while let Some(mut stmt) = batch.next().unwrap_or_else(|e| panic!("{script:?}: {e}")) {
        texts.push(strict_of(&stmt.expanded_sql().unwrap()));
        let mut rows = stmt.raw_query();
        while rows.next().unwrap().is_some() {}
    }
    texts
}

fn sqlite_scripts() -> Vec<String> {
    let mut scripts: Vec<String> = [
        sql_testkit::split::fixture("FX-SPLIT-SQLITE-01"),
        "CREATE TRIGGER t1 AFTER INSERT ON orders BEGIN UPDATE orders SET note = CASE WHEN new.total > 10 THEN 'big;' ELSE 'small' END WHERE id = new.id; END; SELECT 1;".into(),
        "CREATE TEMP TRIGGER IF NOT EXISTS t2 BEFORE DELETE ON orders WHEN CASE WHEN old.id > 0 THEN 1 END BEGIN INSERT INTO audit_log VALUES ('d', 'x;y'); SELECT RAISE(IGNORE); END;\nDELETE FROM orders;".into(),
        "BEGIN IMMEDIATE; INSERT INTO kv VALUES ('a', '1'); COMMIT; BEGIN DEFERRED TRANSACTION; UPDATE kv SET v = '2'; END TRANSACTION; BEGIN EXCLUSIVE; ROLLBACK;".into(),
        "SELECT ';', [a;b], `c;d`, \"e;f\" FROM (SELECT 1 AS [a;b], 2 AS `c;d`, 3 AS \"e;f\");\nSELECT 'it''s; fine';".into(),
        "-- leading ; comment\nSELECT 1; /* between ; */ SELECT 2 -- trailing ;\n;\n;;\nSELECT 3".into(),
        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 5)\nSELECT x FROM n;\nINSERT INTO kv (k, v)\nSELECT 'k' || x, x FROM (SELECT 1 AS x)\nWHERE 1\nON CONFLICT (k) DO UPDATE SET v = excluded.v;".into(),
        "INSERT OR REPLACE INTO kv VALUES ('z', 'z');\nREPLACE INTO kv VALUES ('y', 'y');\nCREATE VIEW v AS\nSELECT k FROM kv;\nDROP VIEW v;".into(),
        "INSERT INTO müşteri (ad, şehir) VALUES ('Ayşe', 'İzmir'), ('Işık', 'Iğdır');\nSELECT ad FROM müşteri WHERE şehir = 'İzmir';".into(),
        "CREATE TABLE t3 (a INTEGER CHECK (a > 0), b TEXT DEFAULT 'x;');\nALTER TABLE t3\nADD COLUMN c INTEGER;\nCREATE INDEX i3 ON t3 (a)\nWHERE a > 1;\nDROP TABLE t3;".into(),
        "CREATE TRIGGER t4 AFTER UPDATE OF note ON orders FOR EACH ROW WHEN new.note IS NOT NULL\nBEGIN\n  INSERT INTO audit_log VALUES ('u', new.note);\n  UPDATE kv SET v = 'u' WHERE k = 'a';\nEND;\nUPDATE orders SET note = 'n';".into(),
        "PRAGMA foreign_keys = ON;\nSAVEPOINT s;\nRELEASE s;\nANALYZE;\nEXPLAIN QUERY PLAN SELECT * FROM orders;\nSELECT count(*) FROM orders".into(),
        "SELECT 1\n;\nSELECT /* ; */ 2;\nSELECT x'41' || 'b' -- c\n;".into(),
    ]
    .into_iter()
    .collect();
    // CRLF variants are generated, never stored (P15).
    let crlf: Vec<String> = scripts.iter().map(|s| s.replace('\n', "\r\n")).collect();
    scripts.extend(crlf);
    scripts
}

#[test]
fn sqlite_splitter_matches_rusqlite_batch() {
    for script in sqlite_scripts() {
        assert_eq!(
            strict_texts_sqlite(&script),
            rusqlite_batch_texts(&script),
            "{script:?}"
        );
    }
}
