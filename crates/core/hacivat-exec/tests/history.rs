//! M3b Task 17: the query history store (HV-EXEC-12, D-061, R-M3-62): SQLite + FTS5, exact-text
//! dedup with a run count, `HvNorm1` fingerprints, redaction before hashing, retention per data
//! source and by age with pins kept, Turkish-tolerant search, prefix recall, a corrupt file
//! moved aside, the 64 KiB statement cap, an owner-only file and a bounded shutdown.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hacivat_exec::{
    HistoryConfig, HistoryError, HistoryHit, HistoryOrigin, HistoryQuery, HistoryRecord,
    HistoryStore, Outcome,
};
use hacivat_proto::ProfileId;
use hacivat_session::session::{RunId, TxMode};
use proptest::prelude::*;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------------------------
// Fixtures

/// The obvious fixture secret; it appears only in assertions that prove it is not stored.
const SECRET: &str = "s3cret";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn pid(n: u32) -> ProfileId {
    format!("00000000-0000-4000-8000-{n:012}").parse().unwrap()
}

fn rec_at(profile: u32, sql: &str, ts: i64) -> HistoryRecord {
    HistoryRecord {
        ts_start_ms: ts,
        duration_ms: 7,
        profile: pid(profile),
        database: Some("db".into()),
        schema: Some("public".into()),
        console: 3,
        origin: HistoryOrigin::Console,
        statement_masked: Arc::from(sql),
        // The producer's values: the store recomputes both from the redacted text.
        text_hash: [0; 32],
        fingerprint: 0,
        params_masked: None,
        outcome: Outcome::Ok,
        rows_affected: None,
        rows_fetched: Some(1),
        error_code: None,
        error_message: None,
        tx_mode: TxMode::Auto,
        run: Some(RunId(9)),
        redacted: false,
    }
}

fn rec(profile: u32, sql: &str) -> HistoryRecord {
    rec_at(profile, sql, now_ms())
}

struct Tmp {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn tmp() -> Tmp {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    Tmp { _dir: dir, path }
}

async fn open(path: &Path) -> Arc<HistoryStore> {
    open_with(path, HistoryConfig::default()).await
}

async fn open_with(path: &Path, cfg: HistoryConfig) -> Arc<HistoryStore> {
    HistoryStore::open(path.to_path_buf(), cfg)
        .await
        .expect("history opens")
}

async fn put(store: &HistoryStore, r: HistoryRecord) {
    store.sink().send(Arc::new(r)).await.unwrap();
}

fn all() -> HistoryQuery {
    HistoryQuery {
        limit: 100_000,
        ..HistoryQuery::default()
    }
}

async fn hits(store: &HistoryStore, q: HistoryQuery) -> Vec<HistoryHit> {
    store.search(q).await.expect("search answers")
}

async fn texts(store: &HistoryStore) -> Vec<String> {
    hits(store, all())
        .await
        .into_iter()
        .map(|h| h.record.statement_masked.to_string())
        .collect()
}

/// What the history keeps of `sql`.
async fn stored(sql: &str) -> HistoryHit {
    let t = tmp();
    let store = open(&t.path).await;
    put(&store, rec(1, sql)).await;
    let mut h = hits(&store, all()).await;
    assert_eq!(h.len(), 1, "{sql}");
    h.pop().unwrap()
}

async fn stored_text(sql: &str) -> String {
    stored(sql).await.record.statement_masked.to_string()
}

/// Records `stored` and searches it for `q`.
async fn search(stored: &str, q: &str) -> Vec<HistoryHit> {
    let t = tmp();
    let store = open(&t.path).await;
    put(&store, rec(1, stored)).await;
    put(&store, rec(1, "SELECT 1")).await;
    hits(
        &store,
        HistoryQuery {
            text: Some(q.into()),
            ..all()
        },
    )
    .await
}

fn sha256(s: &str) -> [u8; 32] {
    Sha256::digest(s.as_bytes()).into()
}

/// Every file of the database (the main file, the WAL and the shared memory).
fn db_bytes(path: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let p = PathBuf::from(format!("{}{suffix}", path.display()));
        if let Ok(b) = std::fs::read(&p) {
            out.extend(b);
        }
    }
    out
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

// ---------------------------------------------------------------------------------------------
// The plan's tests (Task 17 Step 1)

#[tokio::test]
async fn dedup_increments_run_count() {
    let t = tmp();
    let store = open(&t.path).await;
    put(&store, rec_at(1, "SELECT 1", 1_000 + now_ms())).await;
    put(&store, rec_at(1, "SELECT 1", 2_000 + now_ms())).await;
    let h = hits(&store, all()).await;
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].run_count, 2);
    // The latest run's start and metrics win; the text hash is the redacted text's.
    assert_eq!(h[0].record.text_hash, sha256("SELECT 1"));

    // The same text on another data source is its own entry; `select 1` is a new text with
    // the same fingerprint (HV-EXEC-12 test cases).
    put(&store, rec(2, "SELECT 1")).await;
    put(&store, rec(1, "select 1")).await;
    let h = hits(&store, all()).await;
    assert_eq!(h.len(), 3);
    let lower = h
        .iter()
        .find(|h| &*h.record.statement_masked == "select 1")
        .unwrap();
    assert_eq!(lower.run_count, 1);
    assert!(
        h.iter()
            .all(|x| x.record.fingerprint == lower.record.fingerprint)
    );
}

#[tokio::test]
async fn fingerprint_groups_literal_variants() {
    let t = tmp();
    let store = open(&t.path).await;
    put(&store, rec(1, "SELECT * FROM t WHERE id = 1")).await;
    put(&store, rec(1, "select *  from t where id = 42 -- again")).await;
    put(&store, rec(1, "SELECT * FROM t WHERE name IN ('a', 'b')")).await;
    put(&store, rec(1, "SELECT * FROM u WHERE id = 1")).await;
    let h = hits(&store, all()).await;
    assert_eq!(h.len(), 4, "different texts are different entries");
    let fp = |s: &str| {
        h.iter()
            .find(|x| &*x.record.statement_masked == s)
            .unwrap()
            .record
            .fingerprint
    };
    assert_eq!(
        fp("SELECT * FROM t WHERE id = 1"),
        fp("select *  from t where id = 42 -- again")
    );
    assert_ne!(
        fp("SELECT * FROM t WHERE id = 1"),
        fp("SELECT * FROM u WHERE id = 1")
    );
    // "Similar": the entries that share a fingerprint.
    let similar = hits(
        &store,
        HistoryQuery {
            fingerprint: Some(fp("SELECT * FROM t WHERE id = 1")),
            ..all()
        },
    )
    .await;
    assert_eq!(similar.len(), 2);
}

#[tokio::test]
async fn redaction_patterns_before_hashing() {
    assert!(
        !stored_text("CREATE USER u IDENTIFIED BY 's3cret'")
            .await
            .contains("s3cret")
    );
    // The hash is the redacted text's: a hash of the secret text would be a guessing oracle.
    let h = stored("ALTER ROLE app PASSWORD 's3cret'").await;
    assert_eq!(&*h.record.statement_masked, "ALTER ROLE app PASSWORD '***'");
    assert!(h.record.redacted);
    assert_eq!(h.record.text_hash, sha256("ALTER ROLE app PASSWORD '***'"));
    assert_ne!(
        h.record.text_hash,
        sha256("ALTER ROLE app PASSWORD 's3cret'")
    );
    // Two different passwords for the same role are one entry run twice.
    let t = tmp();
    let store = open(&t.path).await;
    put(&store, rec(1, "ALTER ROLE app PASSWORD 's3cret'")).await;
    put(&store, rec(1, "ALTER ROLE app PASSWORD 'hv_test_pw'")).await;
    let h = hits(&store, all()).await;
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].run_count, 2);
}

#[tokio::test]
async fn retention_keeps_10000_per_source_and_pins() {
    let t = tmp();
    let store = open(&t.path).await;
    let base = now_ms() - 60_000;
    for i in 0..10_000 {
        put(&store, rec_at(1, &format!("SELECT {i}"), base + i)).await;
    }
    put(&store, rec_at(2, "SELECT 'other source'", base)).await;
    assert_eq!(hits(&store, all()).await.len(), 10_001);

    let oldest = hits(
        &store,
        HistoryQuery {
            profile: Some(pid(1)),
            ..all()
        },
    )
    .await
    .into_iter()
    .find(|h| &*h.record.statement_masked == "SELECT 0")
    .unwrap();
    store.pin(oldest.id, true).await.unwrap();

    // Two more on source 1: the cap counts unpinned entries (9,999 + 2), so the oldest
    // unpinned one goes and the pinned one stays.
    put(&store, rec_at(1, "SELECT 10000", base + 10_000)).await;
    put(&store, rec_at(1, "SELECT 10001", base + 10_001)).await;
    let one: Vec<String> = hits(
        &store,
        HistoryQuery {
            profile: Some(pid(1)),
            ..all()
        },
    )
    .await
    .into_iter()
    .map(|h| h.record.statement_masked.to_string())
    .collect();
    assert_eq!(one.len(), 10_001, "10,000 unpinned plus the pinned one");
    assert!(one.iter().any(|s| s == "SELECT 0"), "pinned is kept");
    assert!(
        !one.iter().any(|s| s == "SELECT 1"),
        "the oldest unpinned goes"
    );
    assert!(one.iter().any(|s| s == "SELECT 2"));
    assert!(one.iter().any(|s| s == "SELECT 10001"));
    // The other source is untouched.
    assert!(
        texts(&store)
            .await
            .iter()
            .any(|s| s == "SELECT 'other source'")
    );
}

#[tokio::test]
async fn turkish_search_matches_both_ways() {
    for (stored, q) in [
        ("SELECT * FROM İSTANBUL", "istanbul"),
        ("SELECT müşteri", "musteri"),
    ] {
        assert_eq!(search(stored, q).await.len(), 1, "{q}");
    }
}

#[tokio::test]
async fn recall_by_prefix() {
    let t = tmp();
    let store = open(&t.path).await;
    let base = now_ms() - 10_000;
    put(&store, rec_at(1, "SELECT a FROM t", base + 1)).await;
    put(&store, rec_at(1, "UPDATE t SET a = 1", base + 2)).await;
    put(&store, rec_at(1, "SELECT b FROM t", base + 3)).await;
    put(&store, rec_at(2, "SELECT c FROM t", base + 4)).await;
    let r = store.recall(pid(1), "SEL".into(), 10).await.unwrap();
    assert_eq!(
        r.iter().map(|s| &**s).collect::<Vec<_>>(),
        ["SELECT b FROM t", "SELECT a FROM t"]
    );
    assert_eq!(
        store.recall(pid(1), "SEL".into(), 1).await.unwrap().len(),
        1
    );
    // A re-run moves the entry to the front.
    put(&store, rec_at(1, "SELECT a FROM t", base + 5)).await;
    let r = store.recall(pid(1), "SEL".into(), 10).await.unwrap();
    assert_eq!(&*r[0], "SELECT a FROM t");
    // The prefix is text, never a pattern; the empty prefix recalls the newest.
    assert!(
        store
            .recall(pid(1), "%".into(), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .recall(pid(1), "S_L".into(), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.recall(pid(1), String::new(), 10).await.unwrap().len(),
        3
    );
}

#[tokio::test]
async fn corrupt_file_rotates() {
    let t = tmp();
    let junk = b"this is not a database, it is a corrupt history file".repeat(200);
    std::fs::write(&t.path, &junk).unwrap();
    let store = open(&t.path).await;
    let moved = store
        .recovered_from()
        .expect("the corrupt file was moved aside")
        .to_path_buf();
    let name = moved.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("history.sqlite.corrupt-"), "{name}");
    assert_eq!(std::fs::read(&moved).unwrap(), junk, "moved, never wiped");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&moved).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the moved file stays owner-only");
    }
    // The fresh file works.
    put(&store, rec(1, "SELECT 1")).await;
    assert_eq!(texts(&store).await, ["SELECT 1"]);
    store.shutdown(Duration::from_secs(2)).await;
    // A healthy file is not rotated again.
    let again = open(&t.path).await;
    assert!(again.recovered_from().is_none());
    assert_eq!(texts(&again).await, ["SELECT 1"]);
}

#[tokio::test]
async fn statement_over_64_kib_is_capped_with_the_full_hash() {
    let t = tmp();
    let store = open(&t.path).await;
    // 2-byte characters, so the cap falls inside one: the prefix ends on a char boundary.
    let long = format!("SELECT x{}", "ş".repeat(40_000));
    let other = format!("{long}y");
    put(&store, rec(1, &long)).await;
    put(&store, rec(1, &long)).await;
    put(&store, rec(1, &other)).await;
    let h = hits(&store, all()).await;
    assert_eq!(h.len(), 2, "the hash of the whole text keeps them apart");
    for hit in &h {
        let s = &*hit.record.statement_masked;
        assert!(s.len() <= 64 << 10, "{}", s.len());
        assert!(s.len() > (64 << 10) - 4);
        assert!(long.starts_with(s));
    }
    let first = h
        .iter()
        .find(|x| x.record.text_hash == sha256(&long))
        .unwrap();
    assert_eq!(first.run_count, 2);
    assert!(h.iter().any(|x| x.record.text_hash == sha256(&other)));
}

#[cfg(unix)]
#[tokio::test]
async fn file_mode_is_0600() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let t = tmp();
    let store = open(&t.path).await;
    put(&store, rec(1, "SELECT 1")).await;
    assert_eq!(texts(&store).await.len(), 1);
    assert_eq!(mode(&t.path), 0o600);
    let wal = PathBuf::from(format!("{}-wal", t.path.display()));
    let shm = PathBuf::from(format!("{}-shm", t.path.display()));
    for side in [&wal, &shm] {
        if side.exists() {
            assert_eq!(mode(side), 0o600, "{}", side.display());
        }
    }
    store.shutdown(Duration::from_secs(2)).await;

    // A file another program left readable is tightened on open.
    std::fs::set_permissions(&t.path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let again = open(&t.path).await;
    assert_eq!(mode(&t.path), 0o600);
    assert_eq!(texts(&again).await.len(), 1);
}

#[tokio::test]
async fn shutdown_flushes_within_the_grace() {
    let t = tmp();
    let store = open(&t.path).await;
    let sink = store.sink();
    for i in 0..500 {
        sink.try_send(Arc::new(rec(1, &format!("SELECT {i}"))))
            .unwrap();
    }
    let started = Instant::now();
    store.shutdown(Duration::from_secs(2)).await;
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(matches!(
        store.search(all()).await,
        Err(HistoryError::Disabled)
    ));
    drop(store);
    let again = open(&t.path).await;
    assert_eq!(hits(&again, all()).await.len(), 500);
}

// ---------------------------------------------------------------------------------------------
// Secrets (Review Focus 2; controller notes)

/// Statements that carry a secret, in every dialect form the redaction knows.
const SECRET_STATEMENTS: &[&str] = &[
    "ALTER ROLE app PASSWORD 's3cret'",
    "alter role app with login encrypted password 's3cret' valid until '2030-01-01'",
    "CREATE ROLE r LOGIN PASSWORD E's3cret'",
    "ALTER ROLE r PASSWORD $$s3cret$$",
    "ALTER ROLE r PASSWORD $pw$s3cret$pw$",
    "CREATE USER u IDENTIFIED BY 's3cret'",
    "CREATE USER 'u'@'%' IDENTIFIED BY \"s3cret\"",
    "ALTER USER u IDENTIFIED WITH mysql_native_password BY 's3cret'",
    "ALTER USER u IDENTIFIED BY 'n3w' REPLACE 's3cret'",
    "CREATE USER u IDENTIFIED BY s3cret",
    "SET PASSWORD FOR 'u'@'localhost' = 's3cret'",
    "SET PASSWORD = PASSWORD('s3cret')",
    "ALTER LOGIN l WITH PASSWORD = 's3cret'",
    "CHANGE MASTER TO MASTER_HOST='h', MASTER_PASSWORD='s3cret'",
    "CHANGE REPLICATION SOURCE TO SOURCE_PASSWORD = 's3cret'",
    "CREATE SUBSCRIPTION s CONNECTION 'host=h user=u password=s3cret' PUBLICATION p",
    "SELECT dblink_connect('postgresql://u:s3cret@h/db')",
    "CREATE USER MAPPING FOR u SERVER s OPTIONS (user 'u', password 's3cret')",
    "\\connect db u password=s3cret",
    "\\c \"dbname=db password=s3cret\"",
    "SELECT 1;\n\\connect host=h password=s3cret\nSELECT 2",
    // A backslash read both ways: MySQL escapes the quote, PostgreSQL ends the string there.
    "SELECT 'it\\'s'; ALTER USER u IDENTIFIED BY 's3cret'",
    "SELECT 'a\\'; ALTER ROLE r PASSWORD 's3cret'",
    "SELECT 'a\\', 'b'; CREATE USER u IDENTIFIED BY s3cret",
    // String prefixes, comments, code quoted in code, other engines' credentials.
    "CREATE LOGIN l WITH PASSWORD = N's3cret'",
    "ALTER ROLE r PASSWORD U&'s3cret'",
    "ALTER ROLE r PASSWORD /* new */ 's3cret'",
    "CREATE ROLE r PASSWORD's3cret'",
    "DO $$ BEGIN EXECUTE 'ALTER ROLE r PASSWORD ''s3cret'''; END $$",
    "CREATE DATABASE SCOPED CREDENTIAL c WITH IDENTITY = 'u', SECRET = 's3cret'",
    "CREATE SERVER s FOREIGN DATA WRAPPER postgres_fdw OPTIONS (host 'h', password 's3cret')",
    "\\set pw s3cret",
];

#[tokio::test]
async fn secret_forms_are_redacted() {
    for sql in SECRET_STATEMENTS {
        let h = stored(sql).await;
        let s = &*h.record.statement_masked;
        assert!(!s.contains(SECRET), "{sql} → {s}");
        assert!(h.record.redacted, "{sql}");
    }
    assert_eq!(
        stored_text("\\connect db u password=s3cret").await,
        "\\connect ?"
    );
}

#[tokio::test]
async fn statements_without_secrets_are_kept_verbatim() {
    for sql in [
        "SELECT * FROM t WHERE a = 'x'",
        "SELECT password_reset_at FROM users",
        "UPDATE t SET note = 'İstanbul' WHERE id = 1",
    ] {
        let h = stored(sql).await;
        assert_eq!(&*h.record.statement_masked, sql);
        assert!(!h.record.redacted, "{sql}");
    }
}

#[tokio::test]
async fn unmasked_sql_never_reaches_history() {
    let t = tmp();
    let store = open(&t.path).await;
    for (i, sql) in SECRET_STATEMENTS.iter().enumerate() {
        let mut r = rec(1, sql);
        // MySQL's syntax errors quote the statement near the error.
        r.outcome = Outcome::Failed;
        r.error_code = Some("1064".into());
        r.error_message = Some(format!(
            "You have an error in your SQL syntax; check the manual near 'BY 's3cret'' at line {i}"
        ));
        r.params_masked = Some("password=s3cret".into());
        put(&store, r).await;
    }
    assert!(!hits(&store, all()).await.is_empty());
    for q in [SECRET, "s3cr", "password s3cret"] {
        let found = hits(
            &store,
            HistoryQuery {
                text: Some(q.into()),
                ..all()
            },
        )
        .await;
        assert!(found.is_empty(), "search {q:?} found a secret");
    }
    for h in hits(&store, all()).await {
        let r = &h.record;
        for field in [
            Some(r.statement_masked.to_string()),
            r.error_message.clone(),
            r.params_masked.clone(),
        ]
        .into_iter()
        .flatten()
        {
            assert!(!field.contains(SECRET), "{field}");
        }
    }
    store.shutdown(Duration::from_secs(2)).await;
    let bytes = db_bytes(&t.path);
    assert!(!bytes.is_empty());
    assert!(!contains(&bytes, SECRET), "a secret reached the file");
}

#[tokio::test]
async fn do_not_record_profiles_record_nothing() {
    let t = tmp();
    let store = open(&t.path).await;
    store.set_do_not_record(pid(1), true);
    put(&store, rec(1, "SELECT 'hidden'")).await;
    put(&store, rec(2, "SELECT 'shown'")).await;
    assert_eq!(texts(&store).await, ["SELECT 'shown'"]);
    store.set_do_not_record(pid(1), false);
    put(&store, rec(1, "SELECT 'again'")).await;
    assert_eq!(texts(&store).await.len(), 2);
}

#[tokio::test]
async fn error_messages_are_capped_at_4_kib() {
    let t = tmp();
    let store = open(&t.path).await;
    let mut r = rec(1, "SELECT broken");
    r.outcome = Outcome::Failed;
    r.error_message = Some("ğ".repeat(5_000));
    put(&store, r).await;
    let h = hits(&store, all()).await;
    let m = h[0].record.error_message.as_deref().unwrap();
    assert!(m.len() <= 4 << 10 && m.len() > (4 << 10) - 2, "{}", m.len());
    assert_eq!(h[0].record.outcome, Outcome::Failed);
}

// ---------------------------------------------------------------------------------------------
// The review's hostile probe (T17 review round 1): every case is caught, or is a gap that the
// redaction module documents ("What redaction does not catch").

/// The probe's secret (`{P}` in the table).
const PROBE: &str = "s3cretQZ8w";

#[derive(Debug, Clone, Copy)]
enum Probe {
    /// Masked in the statement, the error message and the parameter text, never searchable,
    /// never in the file.
    Caught,
    /// A documented limit: the phrase must appear in `redact.rs`'s gap list.
    Gap(&'static str),
}

/// `(statement, error message, parameter text, expectation)`; `{P}` is [`PROBE`].
const PROBE_TABLE: &[(&str, Option<&str>, Option<&str>, Probe)] = {
    use Probe::{Caught as C, Gap as G};
    &[
        // IDENTIFIED BY in every quoting.
        ("CREATE USER u IDENTIFIED BY '{P}'", None, None, C),
        ("CREATE USER u IDENTIFIED BY \"{P}\"", None, None, C),
        ("CREATE USER u IDENTIFIED BY `{P}`", None, None, C),
        ("CREATE USER u IDENTIFIED BY {P}", None, None, C),
        ("CREATE USER u IDENTIFIED BY $${P}$$", None, None, C),
        ("CREATE USER u IDENTIFIED BY q'[{P}]'", None, None, C),
        ("ALTER ROLE r PASSWORD E'{P}'", None, None, C),
        ("CREATE USER u IDENTIFIED BY _utf8mb4'{P}'", None, None, C),
        ("create user u identified by '{P}'", None, None, C),
        // Roles and SET PASSWORD.
        ("CREATE ROLE r LOGIN PASSWORD '{P}'", None, None, C),
        (
            "CREATE ROLE r LOGIN ENCRYPTED PASSWORD '{P}'",
            None,
            None,
            C,
        ),
        (
            "ALTER ROLE r VALID UNTIL 'infinity' PASSWORD '{P}'",
            None,
            None,
            C,
        ),
        ("ALTER ROLE r\n  PASSWORD\n  '{P}'", None, None, C),
        ("alter role r with password '{P}'", None, None, C),
        ("SET PASSWORD = '{P}'", None, None, C),
        ("SET PASSWORD FOR 'u'@'%' = '{P}'", None, None, C),
        ("SET PASSWORD FOR u = PASSWORD('{P}')", None, None, C),
        // Plugins, MariaDB, GRANT, several users.
        (
            "ALTER USER u IDENTIFIED WITH mysql_native_password BY '{P}'",
            None,
            None,
            C,
        ),
        (
            "ALTER USER u IDENTIFIED WITH 'caching_sha2_password' BY '{P}'",
            None,
            None,
            C,
        ),
        (
            "CREATE USER u IDENTIFIED VIA ed25519 USING PASSWORD('{P}')",
            None,
            None,
            C,
        ),
        ("GRANT ALL ON *.* TO u IDENTIFIED BY '{P}'", None, None, C),
        (
            "CREATE USER a IDENTIFIED BY '{P}', b IDENTIFIED BY '{P}'",
            None,
            None,
            C,
        ),
        // Turkish passwords, escapes, prefixes.
        ("CREATE USER u IDENTIFIED BY 'ŞİFRE_{P}'", None, None, C),
        ("CREATE USER u IDENTIFIED BY şifre_{P}", None, None, C),
        ("ALTER ROLE r PASSWORD 'it''s{P}'", None, None, C),
        ("ALTER USER u IDENTIFIED BY 'it\\'s{P}'", None, None, C),
        ("ALTER ROLE r PASSWORD U&'{P}'", None, None, C),
        // SQL Server.
        (
            "ALTER LOGIN l WITH PASSWORD = '{P}' OLD_PASSWORD = 'old1'",
            None,
            None,
            C,
        ),
        (
            "CREATE CREDENTIAL c WITH IDENTITY = 'u', SECRET = '{P}'",
            None,
            None,
            C,
        ),
        // Comments between the keyword and the value.
        ("ALTER ROLE r PASSWORD /* new */ '{P}'", None, None, C),
        ("-- it's\nALTER ROLE r PASSWORD '{P}'", None, None, C),
        // psql meta-commands.
        ("\\password {P}", None, None, C),
        ("\\connect \"dbname=d password={P}\"", None, None, C),
        (
            "\\c -reuse-previous=on \"host=h password={P}\"",
            None,
            None,
            C,
        ),
        ("\\c postgresql://u:{P}@h/db", None, None, C),
        ("\\setenv PGPASSWORD {P}", None, None, C),
        ("\\set pw {P}", None, None, C),
        ("\\! PGPASSWORD={P} psql -h h", None, None, C),
        ("SELECT 1 \\g\n\\c \"password={P}\"", None, None, C),
        // Connection strings.
        (
            "SELECT * FROM dblink('host=h password={P}', 'SELECT 1') AS t(a int)",
            None,
            None,
            C,
        ),
        (
            "SELECT dblink_connect('c', 'host=h password = {P}')",
            None,
            None,
            C,
        ),
        (
            "SELECT dblink_connect('postgresql://u:{P}@h/db')",
            None,
            None,
            C,
        ),
        (
            "SELECT dblink_connect('postgresql://u%40x:{P}@h/db')",
            None,
            None,
            C,
        ),
        (
            "SELECT f('jdbc:postgresql://h/db?user=u&password={P}')",
            None,
            None,
            C,
        ),
        (
            "SELECT f('Driver={x};Server=h;Uid=u;Pwd={P};')",
            None,
            None,
            C,
        ),
        (
            "CREATE TABLE t (a int) ENGINE=FEDERATED CONNECTION='mysql://u:{P}@h:3306/db/t'",
            None,
            None,
            C,
        ),
        (
            "CREATE SERVER s FOREIGN DATA WRAPPER postgres_fdw OPTIONS (host 'h', password '{P}')",
            None,
            None,
            C,
        ),
        (
            "CREATE USER MAPPING FOR u SERVER s OPTIONS (user 'u', password '{P}')",
            None,
            None,
            C,
        ),
        (
            "ALTER USER MAPPING FOR u SERVER s OPTIONS (SET password '{P}')",
            None,
            None,
            C,
        ),
        (
            "CREATE SUBSCRIPTION s CONNECTION 'host=h password={P}' PUBLICATION p",
            None,
            None,
            C,
        ),
        (
            "CREATE SERVER s FOREIGN DATA WRAPPER mysql OPTIONS (USER 'u', PASSWORD '{P}')",
            None,
            None,
            C,
        ),
        (
            "CHANGE MASTER TO MASTER_HOST='h', MASTER_PASSWORD='{P}'",
            None,
            None,
            C,
        ),
        (
            "CHANGE REPLICATION SOURCE TO SOURCE_PASSWORD = '{P}'",
            None,
            None,
            C,
        ),
        (
            "DO $$ BEGIN EXECUTE format('ALTER ROLE r PASSWORD %L', '{P}'); END $$",
            None,
            None,
            C,
        ),
        // Echoes in the error and the parameter text.
        (
            "CREATE USER u IDENTIFIED BY '{P}'",
            Some("You have an error in your SQL syntax; near 'BY '{P}'' at line 1"),
            None,
            C,
        ),
        ("SELECT 1", None, Some("password={P}"), C),
        // I1: comments that change the lexing, and MySQL executable comments.
        (
            "# Bob's account\nCREATE USER u IDENTIFIED BY '{P}'",
            None,
            None,
            C,
        ),
        (
            "/* a /* b */ it's */ ALTER ROLE r PASSWORD '{P}'",
            None,
            None,
            C,
        ),
        (
            "/*!80000 ALTER USER u IDENTIFIED BY '{P}' */",
            None,
            None,
            C,
        ),
        (
            "/*M!100000 ALTER USER u IDENTIFIED BY '{P}' */",
            None,
            None,
            C,
        ),
        // I2: quoted secret names.
        (
            "CREATE USER MAPPING FOR u SERVER s OPTIONS (\"user\" 'u', \"password\" '{P}')",
            None,
            None,
            C,
        ),
        ("UPDATE t SET \"password\" = '{P}'", None, None, C),
        ("UPDATE t SET `passwd` = '{P}'", None, None, C),
        // I3: the Turkish keyboard.
        ("ALTER USER X İDENTİFİED BY '{P}'", None, None, C),
        ("alter user x ıdentıfıed by '{P}'", None, None, C),
        // m1: comments carrying secrets.
        ("-- password {P}\nSELECT 1", None, None, C),
        ("SELECT 1 /* postgresql://u:{P}@h */", None, None, C),
        ("SELECT 1 /* host=h password={P} */", None, None, C),
        // m2: crypto functions and secret settings.
        ("SELECT pgp_sym_encrypt('x', '{P}')", None, None, C),
        ("SELECT pgp_sym_decrypt(d, '{P}') FROM t", None, None, C),
        ("SELECT AES_ENCRYPT('x', '{P}')", None, None, C),
        ("SELECT crypt('{P}', gen_salt('bf'))", None, None, C),
        ("SELECT decrypt(d, '{P}', 'aes') FROM t", None, None, C),
        (
            "SELECT set_config('app.secret', '{P}', false)",
            None,
            None,
            C,
        ),
        // m3: other secret words. ## m3b-po-redact (named exception, PO M3b-23): caught.
        ("SET @pw = '{P}'", None, None, C),
        ("PRAGMA key = '{P}'", None, None, C),
        ("ATTACH DATABASE 'x.db' AS x KEY '{P}'", None, None, C),
        (
            "CREATE SERVER s FOREIGN DATA WRAPPER w OPTIONS (token '{P}')",
            None,
            None,
            C,
        ),
        (
            "SET @pw := '{P}'; CREATE USER u IDENTIFIED BY @pw",
            None,
            None,
            C,
        ),
        ("SELECT @db_pass := '{P}'", None, None, C),
        ("PRAGMA main.rekey = \"{P}\"", None, None, C),
        ("PRAGMA hexkey = \"x'{P}'\"", None, None, C),
        ("PRAGMA key = {P}", None, None, C),
        (
            "UPDATE settings SET api_key = '{P}' WHERE id = 1",
            None,
            None,
            C,
        ),
        (
            "ALTER SERVER s OPTIONS (SET ACCESS_TOKEN '{P}')",
            None,
            None,
            C,
        ),
        (
            "SELECT set_config('app.private_key', '{P}', false)",
            None,
            None,
            C,
        ),
        (
            "SELECT dblink_connect('host=h auth_token={P}')",
            None,
            None,
            C,
        ),
        (
            "SET @pw = '{P}'",
            Some("Unknown error near '{P}' at line 1"),
            None,
            C,
        ),
        // ## m3b-po-redact (fix round 1, review I2, m1-m3)
        (
            "INSERT INTO cfg VALUES ('{\"access_token\": \"{P}\"}')",
            None,
            None,
            C,
        ),
        (
            "UPDATE integ SET cfg = '{\"api_key\":\"{P}\"}'",
            None,
            None,
            C,
        ),
        ("SELECT '{\"token\":\"{P}\"}'::jsonb", None, None, C),
        ("ATTACH DATABASE 'file:x.db?key={P}' AS x", None, None, C),
        ("EXPLAIN PRAGMA key = '{P}'", None, None, C),
        ("SELECT '{P}' INTO @pw", None, None, C),
        (
            "SELECT http_get('https://h', 'Authorization: Bearer {P}')",
            None,
            None,
            C,
        ),
        // m4: URI edge cases and COPY PROGRAM (documented gaps).
        (
            "SELECT f('postgresql://u:a/{P}@h/db')",
            None,
            None,
            G("unencoded"),
        ),
        (
            "SELECT f('mysql://u:a#{P}@h/db')",
            None,
            None,
            G("unencoded"),
        ),
        (
            "SELECT f('postgresql://u:a?{P}@h')",
            None,
            None,
            G("unencoded"),
        ),
        (
            "SELECT f('postgresql://u:a {P}@h')",
            None,
            None,
            G("unencoded"),
        ),
        ("SELECT f('u:{P}@h')", None, None, G("scheme-less")),
        (
            "COPY t FROM PROGRAM 'curl -u u:{P} http://h/x'",
            None,
            None,
            G("COPY … PROGRAM"),
        ),
        // m5: echoes the scrub cannot recognise (documented gaps).
        (
            "CREATE USER u IDENTIFIED BY '{P}{P}{P}'",
            Some("near 'BY '{P}{P}' at line 1"),
            None,
            G("cut short"),
        ),
        (
            "ALTER ROLE r PASSWORD 'abc' '{P}'",
            Some("near ''{P}'' at line 1"),
            None,
            G("second of two adjacent strings"),
        ),
        // A bound value quoted by the server (T16 scrubs it; the store never sees binds).
        (
            "SELECT * FROM t WHERE id = $1",
            Some("invalid input syntax for type integer: \"{P}\""),
            None,
            G("bound value"),
        ),
    ]
};

#[tokio::test]
async fn review_probe_every_case_is_caught_or_a_documented_gap() {
    // The module docs as one line of words (a phrase may wrap).
    let docs = include_str!("../src/history/redact.rs")
        .replace("//!", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let caught_dir = tmp();
    let gaps_dir = tmp();
    let caught = open(&caught_dir.path).await;
    let gaps = open(&gaps_dir.path).await;
    let mut n_caught = 0;
    let mut undocumented = Vec::new();
    for (i, (sql, err, params, probe)) in PROBE_TABLE.iter().enumerate() {
        let fill = |s: &str| s.replace("{P}", PROBE);
        // A distinct data source per case, so dedup never merges two cases.
        let mut r = rec(1_000 + i as u32, &fill(sql));
        r.error_message = err.map(fill);
        r.params_masked = params.map(fill);
        match probe {
            Probe::Caught => {
                n_caught += 1;
                put(&caught, r).await;
            }
            Probe::Gap(phrase) => {
                if !docs.contains(phrase) {
                    undocumented.push(*phrase);
                }
                put(&gaps, r).await;
            }
        }
    }
    // The gaps are recorded like any other statement (no panic, no refusal).
    assert_eq!(hits(&gaps, all()).await.len(), PROBE_TABLE.len() - n_caught);

    let h = hits(&caught, all()).await;
    assert_eq!(h.len(), n_caught);
    // Cases are named by their table row (data source `1000 + row`), never printed with
    // their secret.
    let row = |r: &HistoryRecord| {
        let s = r.profile.to_string();
        s[s.len() - 4..].parse::<usize>().unwrap() - 1_000
    };
    let mut leaked: Vec<usize> = h
        .iter()
        .filter(|hit| {
            let r = &hit.record;
            !r.redacted
                || [
                    Some(r.statement_masked.to_string()),
                    r.error_message.clone(),
                    r.params_masked.clone(),
                ]
                .into_iter()
                .flatten()
                .any(|f| f.contains(PROBE))
        })
        .map(|hit| row(&hit.record))
        .collect();
    leaked.sort_unstable();
    assert!(leaked.is_empty(), "probe rows leaked: {leaked:?}");
    let found = hits(
        &caught,
        HistoryQuery {
            text: Some(PROBE.into()),
            ..all()
        },
    )
    .await;
    assert!(found.is_empty(), "a probe secret is searchable");
    caught.shutdown(Duration::from_secs(2)).await;
    assert!(
        !contains(&db_bytes(&caught_dir.path), PROBE),
        "a probe secret reached the file"
    );
    gaps.shutdown(Duration::from_secs(2)).await;
    assert!(
        undocumented.is_empty(),
        "undocumented gaps: {undocumented:?}"
    );
}

#[tokio::test]
async fn comments_without_secrets_are_kept_verbatim() {
    for sql in [
        "-- it's fine\nSELECT 'x'",
        "# note\nSELECT 1",
        "/* a /* nested */ comment */ SELECT 2",
        "/*!40101 SET NAMES utf8mb4 */",
        "SELECT 1--1",
    ] {
        let h = stored(sql).await;
        assert_eq!(&*h.record.statement_masked, sql);
        assert!(!h.record.redacted, "{sql}");
    }
}

// ---------------------------------------------------------------------------------------------
// Retention by age

#[tokio::test]
async fn entries_older_than_the_max_age_go_unless_pinned() {
    let t = tmp();
    let long = HistoryConfig {
        max_age_days: 365,
        ..HistoryConfig::default()
    };
    let store = open_with(&t.path, long).await;
    let old = now_ms() - 100 * DAY_MS;
    put(&store, rec_at(1, "SELECT 'old pinned'", old)).await;
    put(&store, rec_at(1, "SELECT 'old'", old)).await;
    put(&store, rec(1, "SELECT 'new'")).await;
    let pinned = hits(&store, all())
        .await
        .into_iter()
        .find(|h| &*h.record.statement_masked == "SELECT 'old pinned'")
        .unwrap();
    store.pin(pinned.id, true).await.unwrap();
    store.shutdown(Duration::from_secs(2)).await;

    // Reopened with the default 90 days: the unpinned old entry goes at open.
    let store = open(&t.path).await;
    let mut left = texts(&store).await;
    left.sort();
    assert_eq!(left, ["SELECT 'new'", "SELECT 'old pinned'"]);
    let h = hits(&store, all()).await;
    assert!(
        h.iter()
            .find(|x| &*x.record.statement_masked == "SELECT 'old pinned'")
            .unwrap()
            .pinned
    );
    // A new record that is already too old is not kept either.
    put(&store, rec_at(1, "SELECT 'stale'", now_ms() - 91 * DAY_MS)).await;
    assert_eq!(texts(&store).await.len(), 2);
}

// ---------------------------------------------------------------------------------------------
// Search

#[tokio::test]
async fn turkish_search_more_pairs() {
    for (stored, q) in [
        ("SELECT * FROM istanbul", "İSTANBUL"),
        ("SELECT * FROM ISPARTA", "ısparta"),
        ("SELECT ılık FROM t", "ILIK"),
        ("SELECT * FROM ŞEHİR", "sehir"),
        ("SELECT çiğdem", "CIGDEM"),
        ("SELECT * FROM öğrenci", "ogrenci"),
        // Decomposed input: `I` + U+0307, `s` + U+0327.
        ("SELECT * FROM I\u{307}stanbul", "istanbul"),
        ("SELECT s\u{327}ehir", "şehir"),
        // A prefix finds the longer word.
        ("SELECT müşteri_no FROM müşteriler", "müşt"),
    ] {
        assert_eq!(search(stored, q).await.len(), 1, "{stored} / {q}");
    }
    assert!(search("SELECT * FROM ankara", "istanbul").await.is_empty());
}

#[tokio::test]
async fn search_filters_by_profile_outcome_and_time() {
    let t = tmp();
    let store = open(&t.path).await;
    let base = now_ms() - 10_000;
    put(&store, rec_at(1, "SELECT 'a'", base)).await;
    let mut failed = rec_at(1, "SELECT 'b'", base + 5_000);
    failed.outcome = Outcome::Failed;
    failed.error_message = Some("relation \"kayıp\" does not exist".into());
    put(&store, failed).await;
    put(&store, rec_at(2, "SELECT 'c'", base + 6_000)).await;

    let q = |f: fn(&mut HistoryQuery)| {
        let mut q = all();
        f(&mut q);
        q
    };
    assert_eq!(hits(&store, q(|q| q.profile = Some(pid(1)))).await.len(), 2);
    let f = hits(&store, q(|q| q.outcome = Some(Outcome::Failed))).await;
    assert_eq!(f.len(), 1);
    assert_eq!(&*f[0].record.statement_masked, "SELECT 'b'");
    let since = base + 4_000;
    let mut s = all();
    s.since_ms = Some(since);
    assert_eq!(hits(&store, s).await.len(), 2);
    // Newest first.
    assert_eq!(
        texts(&store).await,
        ["SELECT 'c'", "SELECT 'b'", "SELECT 'a'"]
    );
    // The error message is searched too, Turkish-tolerant.
    assert_eq!(
        hits(&store, q(|q| q.text = Some("KAYIP".into())))
            .await
            .len(),
        1
    );
    // A limit.
    let mut l = all();
    l.limit = 1;
    assert_eq!(hits(&store, l).await.len(), 1);
}

/// User text is never FTS5 syntax: operators, quotes, column filters and SQL are all words.
const HOSTILE_QUERIES: &[&str] = &[
    "\"",
    "\"\"",
    "*",
    "^",
    "-",
    "(",
    ")",
    "NEAR(",
    "a OR",
    "AND",
    "NOT x",
    "a NEAR/2 b",
    "search_key:x",
    "{search_key error_message}: x",
    "x*\"y",
    "'); DROP TABLE history; --",
    "\0",
    "   ",
    "",
    "🙂",
    "\u{307}",
];

#[tokio::test]
async fn malformed_fts_text_is_answered_not_injected() {
    let t = tmp();
    let store = open(&t.path).await;
    put(&store, rec(1, "SELECT * FROM history WHERE x = 'NEAR'")).await;
    for q in HOSTILE_QUERIES {
        let r = store
            .search(HistoryQuery {
                text: Some((*q).into()),
                ..all()
            })
            .await;
        assert!(r.is_ok(), "{q:?} → {r:?}");
    }
    // Operator words are plain words.
    let near = hits(
        &store,
        HistoryQuery {
            text: Some("NEAR".into()),
            ..all()
        },
    )
    .await;
    assert_eq!(near.len(), 1);
    // The store is intact.
    assert_eq!(texts(&store).await.len(), 1);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]
    #[test]
    fn any_search_text_answers(q in "\\PC{0,40}") {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let t = tmp();
            let store = open(&t.path).await;
            put(&store, rec(1, "SELECT 1")).await;
            let r = store.search(HistoryQuery { text: Some(q.clone()), ..all() }).await;
            prop_assert!(r.is_ok(), "{:?} → {:?}", q, r);
            prop_assert_eq!(texts(&store).await.len(), 1);
            Ok(())
        })?;
    }
}

// ---------------------------------------------------------------------------------------------
// Concurrency

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_stores_on_one_file_do_not_corrupt_it() {
    let t = tmp();
    let a = open(&t.path).await;
    let b = open(&t.path).await;
    let (sa, sb) = (a.sink(), b.sink());
    let wa = tokio::spawn(async move {
        for i in 0..300 {
            sa.send(Arc::new(rec(1, &format!("SELECT 'a{i}'"))))
                .await
                .unwrap();
        }
    });
    let wb = tokio::spawn(async move {
        for i in 0..300 {
            sb.send(Arc::new(rec(1, &format!("SELECT 'b{i}'"))))
                .await
                .unwrap();
        }
    });
    wa.await.unwrap();
    wb.await.unwrap();
    a.shutdown(Duration::from_secs(2)).await;
    b.shutdown(Duration::from_secs(2)).await;
    let c = open(&t.path).await;
    assert!(c.recovered_from().is_none());
    assert_eq!(hits(&c, all()).await.len(), 600);
}

// ---------------------------------------------------------------------------------------------
// ## m4-t32-shots (named exception; PO R-W5-14 b): "Clear query history…" per connection.

#[tokio::test]
async fn clear_removes_one_connections_entries_pinned_too_and_keeps_the_others() {
    let t = tmp();
    let store = open(&t.path).await;
    put(&store, rec(1, "SELECT 'one a'")).await;
    put(&store, rec(1, "SELECT 'one b'")).await;
    put(&store, rec(2, "SELECT 'two'")).await;
    let pinned = hits(&store, all())
        .await
        .into_iter()
        .find(|h| &*h.record.statement_masked == "SELECT 'one b'")
        .expect("recorded");
    store.pin(pinned.id, true).await.unwrap();
    // A record sent just before the clear is cleared with the rest (the writer drains first).
    put(&store, rec(1, "SELECT 'one c'")).await;
    let removed = store.clear(pid(1)).await.expect("clear answers");
    assert_eq!(removed, 3);
    assert_eq!(texts(&store).await, ["SELECT 'two'"]);
    // The full-text index forgot them too.
    let found = hits(
        &store,
        HistoryQuery {
            text: Some("one".into()),
            ..all()
        },
    )
    .await;
    assert!(found.is_empty(), "{found:?}");
    // Clearing again removes nothing and is not an error; the store keeps recording.
    assert_eq!(store.clear(pid(1)).await, Ok(0));
    put(&store, rec(1, "SELECT 'after'")).await;
    assert_eq!(
        store.recall(pid(1), String::new(), 5).await.unwrap(),
        vec![Arc::<str>::from("SELECT 'after'")]
    );
}

#[tokio::test]
async fn clear_after_shutdown_answers_disabled() {
    let t = tmp();
    let store = open(&t.path).await;
    store.shutdown(Duration::from_secs(2)).await;
    assert_eq!(store.clear(pid(1)).await, Err(HistoryError::Disabled));
}

// ## m4-t32-shots (fix round 1, review I-1 and T-2)

/// Whether `needle` is in the raw bytes of the history file or its `-wal` / `-shm`.
fn on_disk(path: &Path, needle: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let p = PathBuf::from(format!("{}{suffix}", path.display()));
        if let Ok(b) = std::fs::read(&p)
            && contains(&b, needle)
        {
            hits.push(format!("history.sqlite{suffix}"));
        }
    }
    hits
}

/// I-1: "deleted from this computer". After a clear and a clean close, the cleared statements'
/// text (the table's row, its search key, the full-text index's tokens) is gone from the file's
/// raw bytes, not just unlinked; the other connection's row stays readable.
#[tokio::test]
async fn cleared_text_is_gone_from_the_file_bytes() {
    let t = tmp();
    let store = open(&t.path).await;
    let marker = "zqxmarkerclearedtext";
    // One statement big enough for its own pages, one sharing a page with the kept row.
    put(
        &store,
        rec(1, &format!("SELECT '{marker}' || '{}'", "x".repeat(6000))),
    )
    .await;
    put(&store, rec(1, &format!("SELECT {marker}small FROM t"))).await;
    put(&store, rec(2, "SELECT 'keptrowzq'")).await;
    assert_eq!(store.clear(pid(1)).await, Ok(2));
    // Right after the clear, before any close: the clear itself overwrote and checkpointed.
    assert!(
        on_disk(&t.path, marker).is_empty(),
        "cleared text on disk right after the clear: {:?}",
        on_disk(&t.path, marker)
    );
    store.shutdown(Duration::from_secs(2)).await;
    let hits = on_disk(&t.path, marker);
    assert!(
        hits.is_empty(),
        "cleared statement text still on disk in {hits:?}"
    );
    let again = open(&t.path).await;
    assert_eq!(texts(&again).await, ["SELECT 'keptrowzq'"]);
}

/// T-2: ids sharing a long prefix; the other connection's pinned and unpinned rows and its
/// full-text hits survive the clear.
#[tokio::test]
async fn clear_is_exact_and_keeps_the_other_connections_pins_and_search() {
    let t = tmp();
    let store = open(&t.path).await;
    put(&store, rec(1, "SELECT 'alpha'")).await;
    put(&store, rec(10, "SELECT 'beta pinned'")).await;
    put(&store, rec(10, "SELECT 'beta loose'")).await;
    let pinned = hits(&store, all())
        .await
        .into_iter()
        .find(|h| &*h.record.statement_masked == "SELECT 'beta pinned'")
        .expect("recorded");
    store.pin(pinned.id, true).await.unwrap();
    assert_eq!(store.clear(pid(1)).await, Ok(1));
    let mut left = texts(&store).await;
    left.sort();
    assert_eq!(left, ["SELECT 'beta loose'", "SELECT 'beta pinned'"]);
    let found = hits(
        &store,
        HistoryQuery {
            text: Some("beta".into()),
            ..all()
        },
    )
    .await;
    assert_eq!(found.len(), 2);
}

/// T-2: records streamed while a clear runs: every one sent before the clear is gone, the
/// other connection keeps all of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clear_during_a_stream_of_records() {
    let t = tmp();
    let store = open(&t.path).await;
    for i in 0..200 {
        put(&store, rec(1, &format!("SELECT {i} AS before"))).await;
        put(&store, rec(2, &format!("SELECT {i} AS other"))).await;
    }
    let s2 = store.clone();
    let writer = tokio::spawn(async move {
        for i in 0..500 {
            put(&s2, rec(1, &format!("SELECT {i} AS during"))).await;
        }
    });
    let n = store.clear(pid(1)).await.unwrap();
    writer.await.unwrap();
    assert!(n >= 200, "cleared {n}");
    let left = texts(&store).await;
    assert!(
        !left.iter().any(|t| t.contains("AS before")),
        "a pre-clear entry survived"
    );
    assert_eq!(left.iter().filter(|t| t.contains("AS other")).count(), 200);
}

// ## m4-t33b-carries (T32 carry: rotated corrupt files keep old history)
/// The damaged copies an open set aside are listed one per copy (their `-wal`/`-shm` go with
/// them) and deleted on request; the live file and unrelated files are never touched.
#[tokio::test]
async fn damaged_backups_are_listed_and_deleted_on_request() {
    let t = tmp();
    assert!(
        HistoryStore::damaged_backups(t.path.clone())
            .await
            .unwrap()
            .is_empty()
    );
    let junk = b"not a database".repeat(200);
    std::fs::write(&t.path, &junk).unwrap();
    let first = open(&t.path).await;
    let moved = first.recovered_from().unwrap().to_path_buf();
    first.shutdown(Duration::from_secs(2)).await;
    // A second rotation, with a log beside the damaged file.
    std::fs::write(&t.path, &junk).unwrap();
    let wal = {
        let mut s = t.path.as_os_str().to_owned();
        s.push("-wal");
        PathBuf::from(s)
    };
    std::fs::write(&wal, b"old log").unwrap();
    let second = open(&t.path).await;
    let moved2 = second.recovered_from().unwrap().to_path_buf();
    put(&second, rec(1, "SELECT 'live'")).await;
    let dir = t.path.parent().unwrap().to_path_buf();
    let unrelated = dir.join("history.sqlite.notes");
    std::fs::write(&unrelated, b"keep").unwrap();
    let mut listed = HistoryStore::damaged_backups(t.path.clone()).await.unwrap();
    listed.sort();
    let mut want = vec![moved.clone(), moved2.clone()];
    want.sort();
    assert_eq!(listed, want, "one entry per copy");
    let n = HistoryStore::delete_damaged_backups(t.path.clone())
        .await
        .unwrap();
    assert_eq!(n, 2);
    assert!(!moved.exists() && !moved2.exists());
    let mut moved2_wal = moved2.as_os_str().to_owned();
    moved2_wal.push("-wal");
    assert!(
        !PathBuf::from(moved2_wal).exists(),
        "the copy's log goes too"
    );
    assert!(unrelated.exists(), "an unrelated file stays");
    assert!(
        HistoryStore::damaged_backups(t.path.clone())
            .await
            .unwrap()
            .is_empty()
    );
    // The live history is untouched.
    assert_eq!(texts(&second).await, ["SELECT 'live'"]);
}
