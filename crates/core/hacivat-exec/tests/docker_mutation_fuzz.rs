//! The PostgreSQL classifier mutation fuzz on Docker pg17 (Task 20; SQL-T13; `alg/13` :399;
//! P-M3b-13: a `crates/sql` test never reaches a server, so the leg lives here). Run it with
//! `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-exec --test docker_mutation_fuzz`.
//!
//! The corpus statements (the PG read corpus, T14's escaped literals, the T9/T10/T14 review
//! seeds) and their mutants (`sql-safety/tests/support/mutate.rs`, driven by the shared
//! `edits.rs` generator) are classified through the real `ClassifyPool` with pg17's lexing
//! modes known. Every text the read-only profile allows is run on a **read-only console as the
//! test account** (a superuser: no privilege stops anything, only the read-only plan does) in a
//! database of its own (`hv_t20`, dropped first and last), and:
//!
//! - the server never refuses it as a write (`25006`, read-only transaction), and the guard's
//!   post-check never finds an effect (`gap`): either means the classifier called a write a
//!   read;
//! - the **side channel** sees nothing changed. Every read-only request ends with the guard's
//!   `ROLLBACK` and `DISCARD ALL` before Run returns, so the side channel watches what
//!   outlives both (round 1, review I1), from connections outside the guarded session, before
//!   and after every statement:
//!   - replication slots (`pg_replication_slots`);
//!   - server files: every `/tmp/hv_t20*` file's size and modification time (`pg_ls_dir`,
//!     `pg_stat_file`), the paths the mutants' `COPY … TO` / `lo_export` write;
//!   - other backends: a sentinel connection (`application_name = 'hv_t20_sentinel'`) and the
//!     side channel's own two backends are still alive;
//!   - statistics resets (`pg_stat_database`, `pg_stat_bgwriter`, `pg_stat_wal`,
//!     `pg_stat_archiver` reset times) and configuration reloads (`pg_conf_load_time()`);
//!   - prepared (two-phase) transactions.
//!
//!   The fixture rows, the sequence, relations, temporary tables, large objects and advisory
//!   locks are probed too, as a belt-and-braces check of the guard's `ROLLBACK` / `DISCARD
//!   ALL`, not as the side channel. A self-check first proves each probe fires (a slot, a file,
//!   a terminated sentinel, a statistics reset and a reload made from the admin connection).
//!
//! Other outcomes are expected for mutants and only counted, each one explained: a server error
//! (a SQLSTATE: syntax, missing objects, the database's 5 s statement timeout), a guard refusal
//! before any I/O (`1a`, `function`, `input`, `engine`), `protocol` (a unit with a `$n`
//! placeholder: M3a's Run has no binds, so the read-only guard refuses the extended protocol
//! before sending it), or "not sent" (Run's resolution refused the text). Anything else fails,
//! and the console must still run `SELECT 1` at the end (a lost connection would otherwise turn
//! later cases into silent refusals). Every case is recorded in the Docker artifact
//! `t20-mutation-fuzz.tsv`. `HACIVAT_FUZZ_CASES` sets the number of mutants (default 10,000, a
//! third of them meaning-preserving; the PR set's budget is ≤ 5 min).

#![allow(
    clippy::result_large_err,
    reason = "the session's Reply is a fixed interface"
)]

mod common;
#[path = "../../../sql/sql-lexer/tests/support/edits.rs"]
mod edits;
#[path = "../../../sql/sql-safety/tests/support/mutate.rs"]
mod mutate;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use common::matrix::*;
use edits::Rng;
use hacivat_exec::ClassifyPool;
use mutate::Family;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes, ServerVersion};
use sql_safety::{Actor, ClassifyCtx, Expect, Profile, decide};

/// The database the leg owns.
const DB: &str = "hv_t20";

fn corpus(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../sql/sql-testkit/corpus")
        .join(rel)
}

/// The entries of the PG read corpus (`--` comment lines first; an entry ends with `;` at a
/// line end).
fn pg_reads() -> Vec<String> {
    let text = std::fs::read_to_string(corpus("read/pg.sql")).unwrap();
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        if cur.is_empty() && (line.starts_with("--") || line.trim().is_empty()) {
            continue;
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(line);
        if let Some(stmt) = cur.strip_suffix(';') {
            out.push(stmt.to_string());
            cur.clear();
        }
    }
    out
}

#[derive(serde::Deserialize)]
struct Rows {
    row: Vec<Row>,
}

#[derive(serde::Deserialize)]
struct Row {
    #[serde(default)]
    dialect: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    sql: String,
}

/// The PostgreSQL rows of a corpus TOML (escaped literals: `dialect = "pg"`; seeds: a `pg`
/// tag without a client flavour).
fn pg_rows(rel: &str) -> Vec<String> {
    let text = std::fs::read_to_string(corpus(rel)).unwrap();
    toml::from_str::<Rows>(&text)
        .unwrap()
        .row
        .into_iter()
        .filter(|r| {
            r.dialect.as_deref() == Some("pg")
                || r.tag
                    .as_deref()
                    .is_some_and(|t| t.split(',').next() == Some("pg") && !t.contains("psql"))
        })
        .map(|r| r.sql)
        .collect()
}

fn cases() -> usize {
    std::env::var("HACIVAT_FUZZ_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000)
}

/// The fixtures: the corpus's seed schema (`docs/research/datagrip/13-behavioral-test-corpus.md`
/// §3.1.2, FX-SEED-PG), the table, sequence and writing function the mutants aim at, and a
/// statement timeout so no mutant runs long.
const FIXTURES: &str = "\
CREATE TYPE order_status AS ENUM ('new','paid','shipped','cancelled');
CREATE TABLE customers (id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, email text NOT NULL UNIQUE, full_name text NOT NULL, city text, referred_by bigint REFERENCES customers(id), created_at timestamptz NOT NULL DEFAULT now(), is_active boolean NOT NULL DEFAULT true, external_ref uuid DEFAULT gen_random_uuid());
CREATE TABLE products (sku text PRIMARY KEY, title text NOT NULL, price numeric(12,2) NOT NULL CHECK (price >= 0), weight_g integer, tags text[] DEFAULT '{}', attrs jsonb, photo bytea);
CREATE TABLE orders (id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, customer_id bigint NOT NULL REFERENCES customers(id), status order_status NOT NULL DEFAULT 'new', ordered_at timestamptz NOT NULL DEFAULT now(), note text);
CREATE TABLE order_items (order_id bigint REFERENCES orders(id) ON DELETE CASCADE, line_no integer, sku text NOT NULL REFERENCES products(sku), qty integer NOT NULL CHECK (qty > 0), unit_price numeric(12,2) NOT NULL, line_total numeric(14,2) GENERATED ALWAYS AS (qty * unit_price) STORED, PRIMARY KEY (order_id, line_no));
CREATE TABLE audit_log (happened_at timestamptz DEFAULT now(), actor text, message text);
CREATE TABLE warehouses (code text NOT NULL, name text, CONSTRAINT warehouses_code_uk UNIQUE (code));
CREATE TABLE visits (visit_id bigint PRIMARY KEY, customer_id bigint, page text);
INSERT INTO customers(email, full_name, city) VALUES ('ada@example.com','Ada Yılmaz','İstanbul'),('bora@example.com','Bora Işık','Iğdır'),('cem@example.com','Cem Öztürk','Ankara'),('deniz@example.com','Deniz Çelik', NULL);
UPDATE customers SET referred_by = 1 WHERE id IN (2,3);
INSERT INTO products VALUES ('SKU-1','Çay bardağı', 12.50, 120, '{kitchen}', '{\"color\":\"red\"}', '\\x89504e47'),('SKU-2','Kahve; çekirdek', 249.90, 1000, '{food,coffee}', '{\"roast\":\"dark\"}', NULL),('SKU-3','Kitap \"SQL\"', 0.00, NULL, '{}', NULL, NULL);
INSERT INTO orders(customer_id, status, note) VALUES (1,'paid','ilk'),(2,'new',NULL),(1,'shipped','tab inside');
INSERT INTO order_items(order_id,line_no,sku,qty,unit_price) VALUES (1,1,'SKU-1',2,12.50),(1,2,'SKU-2',1,249.90),(3,1,'SKU-3',5,0);
INSERT INTO audit_log(actor,message) VALUES ('sys','boot'),('sys','boot');
INSERT INTO warehouses VALUES ('IST','İstanbul Depo'),('ANK','Ankara Depo');
INSERT INTO visits VALUES (10,1,'/'),(11,2,'/cart'),(12,NULL,'/');
CREATE TABLE numbers AS SELECT g AS n, md5(g::text) AS h FROM generate_series(1,1234) g;
ALTER TABLE numbers ADD PRIMARY KEY (n);
CREATE TABLE hv_t20_t (a int);
INSERT INTO hv_t20_t VALUES (1), (2), (3);
CREATE SEQUENCE hv_t20_seq;
CREATE FUNCTION hv_t20_writer() RETURNS int LANGUAGE plpgsql AS $$ BEGIN INSERT INTO hv_t20_t VALUES (99); RETURN 1; END $$;
";

/// The connections the side channel reads through, outside the guarded session: `root` (the
/// test database, server-wide probes), `admin` (the leg's database: its statistics and the
/// belt-and-braces probes) and the `sentinel` whose survival is probed.
struct Channel {
    root: Side,
    admin: Side,
    sentinel: Side,
    pids: String,
}

impl Channel {
    /// A sentinel connection on the leg's database, named so the mutants' terminate fragment
    /// aims at it, and the three backends' process ids.
    async fn open(line: &Line, root: Side, mut admin: Side) -> Channel {
        let mut sentinel = Side::open(line, Some(DB)).await;
        sentinel
            .batch("SET application_name = 'hv_t20_sentinel'")
            .await;
        let ids = [root_pid(&mut sentinel).await, root_pid(&mut admin).await];
        let mut c = Channel {
            root,
            admin,
            sentinel,
            pids: String::new(),
        };
        let own = root_pid(&mut c.root).await;
        c.pids = format!("{own},{},{}", ids[0], ids[1]);
        c
    }

    /// The side channel: effects that outlive a rolled-back read-only transaction and
    /// `DISCARD ALL`, then the belt-and-braces probes of the guard's rollback.
    async fn snapshot(&mut self) -> String {
        let durable = self
            .root
            .scalar(&format!(
                "SELECT concat_ws('|', \
                 'slots=' || (SELECT count(*) FROM pg_replication_slots), \
                 'files=' || coalesce((SELECT string_agg(f || ':' || (pg_stat_file('/tmp/' || f, true)).size \
                   || '@' || (pg_stat_file('/tmp/' || f, true)).modification, ',' ORDER BY f) \
                   FROM pg_ls_dir('/tmp', true, false) f WHERE f LIKE 'hv\\_t20%'), ''), \
                 'alive=' || (SELECT count(*) FROM pg_stat_activity WHERE pid IN ({})), \
                 'dbreset=' || coalesce(pg_stat_get_db_stat_reset_time((SELECT oid FROM pg_database WHERE datname = '{DB}'))::text, '-'), \
                 'bgw=' || coalesce((SELECT stats_reset FROM pg_stat_bgwriter)::text, '-'), \
                 'wal=' || coalesce((SELECT stats_reset FROM pg_stat_wal)::text, '-'), \
                 'arch=' || coalesce((SELECT stats_reset FROM pg_stat_archiver)::text, '-'), \
                 'conf=' || pg_conf_load_time(), \
                 '2pc=' || (SELECT count(*) FROM pg_prepared_xacts))",
                self.pids
            ))
            .await;
        // Belt and braces: what the guard's `ROLLBACK` and `DISCARD ALL` must already undo.
        let rolled_back = self
            .admin
            .scalar(
                "SELECT concat_ws('|', \
                 (SELECT count(*) || ':' || coalesce(sum(a), 0) FROM hv_t20_t), \
                 (SELECT last_value || ':' || is_called FROM hv_t20_seq), \
                 (SELECT count(*) FROM pg_class WHERE relnamespace = 'public'::regnamespace), \
                 (SELECT count(*) FROM pg_class WHERE relpersistence = 't'), \
                 (SELECT count(*) FROM pg_largeobject_metadata), \
                 (SELECT count(*) FROM pg_locks WHERE locktype = 'advisory'), \
                 (SELECT count(*) FROM customers), (SELECT count(*) FROM orders), \
                 (SELECT count(*) FROM audit_log))",
            )
            .await;
        format!("{durable} || {rolled_back}")
    }

    /// Undoes what can be undone after a finding (the slots, a dead sentinel), so the next
    /// statement is judged on its own: the caller then takes a fresh baseline.
    async fn restore(&mut self, line: &Line) {
        let _ = self
            .root
            .try_batch(
                "SELECT pg_drop_replication_slot(slot_name) FROM pg_replication_slots \
                 WHERE slot_name LIKE 'hv\\_t20%' AND NOT active",
            )
            .await;
        let _ = self.root.try_batch("SELECT pg_advisory_unlock_all()").await;
        let alive = self
            .root
            .count(&format!(
                "SELECT count(*) FROM pg_stat_activity WHERE pid IN ({})",
                self.pids
            ))
            .await;
        if alive != 3 {
            let mut sentinel = Side::open(line, Some(DB)).await;
            sentinel
                .batch("SET application_name = 'hv_t20_sentinel'")
                .await;
            let new = root_pid(&mut sentinel).await;
            let old = std::mem::replace(&mut self.sentinel, sentinel);
            drop(old);
            let mut parts: Vec<String> = self.pids.split(',').map(str::to_string).collect();
            parts[1] = new;
            self.pids = parts.join(",");
        }
    }
}

async fn root_pid(s: &mut Side) -> String {
    s.scalar("SELECT pg_backend_pid()").await
}

/// Whether `got` is an explained outcome for `sql` (the module docs).
fn explained(got: &str, sql: &str) -> bool {
    let sqlstate = got.len() == 5 && got.bytes().all(|b| b.is_ascii_alphanumeric());
    let has_positional = sql
        .as_bytes()
        .windows(2)
        .any(|w| w[0] == b'$' && w[1].is_ascii_digit());
    sqlstate
        || got == "ran"
        || matches!(got, "1a" | "function" | "input" | "engine")
        || got.starts_with("not sent: ")
        || (got == "protocol" && has_positional)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn classifier_read_verdicts_never_violate_read_only_on_pg17() {
    if !docker_enabled() {
        return;
    }
    let Some(target) = hacivat_testkit::require_target("pg17") else {
        return;
    };
    let line = line_of(target);

    // The database of its own, fresh.
    let mut root = Side::open(&line, None).await;
    let _ = root
        .try_batch(&format!("DROP DATABASE IF EXISTS {DB} WITH (FORCE)"))
        .await;
    root.batch(&format!("CREATE DATABASE {DB}")).await;
    root.batch(&format!("ALTER DATABASE {DB} SET statement_timeout = '5s'"))
        .await;
    // Slots a red run left behind (the slots are server-wide).
    let _ = root
        .try_batch(
            "SELECT pg_drop_replication_slot(slot_name) FROM pg_replication_slots \
             WHERE slot_name LIKE 'hv\\_t20%' AND NOT active",
        )
        .await;
    let mut admin = Side::open(&line, Some(DB)).await;
    admin.batch(FIXTURES).await;
    let mut side = Channel::open(&line, root, admin).await;
    self_check(&mut side, &line).await;

    // The inputs: the corpus, then mutants of it.
    let mut base = pg_reads();
    base.extend(pg_rows("read/escaped_literals.toml"));
    base.extend(pg_rows("fuzz/classifier_seeds.toml"));
    let donors: Vec<&str> = base.iter().map(String::as_str).collect();
    let mut r = Rng::new(0x7432_3044);
    let mut inputs: Vec<(String, String)> = base
        .iter()
        .map(|s| ("corpus".to_string(), s.clone()))
        .collect();
    for k in 0..cases() {
        let seed = donors[r.below(donors.len())];
        if k % 3 == 2 {
            // A near miss: the meaning kept, the bytes moved (case, whitespace, comments).
            let mut m = seed.to_string();
            for _ in 0..3 {
                if let Some(next) = mutate::preserve(&mut r, &m, Family::Pg) {
                    m = next;
                }
            }
            inputs.push(("preserved".into(), m));
        } else {
            inputs.push((
                "mutant".into(),
                mutate::mutate(&mut r, seed, Family::Pg, &donors),
            ));
        }
    }

    let mut c = Console::open(
        &line,
        Opts {
            read_only: true,
            database: Some(DB.to_string()),
            ..Opts::dev()
        },
    )
    .await;
    assert!(c.snap().read_only, "the console is read-only");
    let pool = ClassifyPool::new(2);
    let ctx = ClassifyCtx::new(
        DialectCtx::new(DialectId::Postgres, Some(ServerVersion::new(17, 0, 0))),
        ClientFlavor::None,
        Expect::SingleStatement,
    )
    .with_known_modes(LexModes::default_for(DialectId::Postgres));

    let mut outcomes: BTreeMap<String, usize> = BTreeMap::new();
    let (mut allowed, mut findings, mut record) = (0usize, Vec::new(), Vec::new());
    let mut unexplained = Vec::new();
    for (kind, sql) in &inputs {
        let report = pool
            .classify(Arc::from(sql.as_str()), ctx.clone())
            .await
            .expect("infallible");
        if !decide(&report, Profile::ReadOnlyPlusExplain, Actor::Human).allows() {
            continue;
        }
        allowed += 1;
        // Before and after each statement, from outside the guarded session.
        let before = side.snapshot().await;
        let got = match c.try_run(sql).await {
            Ok(ran) => ran.outcome(),
            Err(resolution) => format!("not sent: {resolution}"),
        };
        if got == "gap" {
            c.recover().await;
        }
        let after = side.snapshot().await;
        *outcomes.entry(got.clone()).or_default() += 1;
        record.push(format!("{kind}\t{got}\t{}", sql.replace(['\n', '\t'], " ")));
        if got == "25006" || got == "gap" {
            findings.push(format!(
                "{kind}: allowed as a read, the server says write ({got}): {sql:?}"
            ));
        } else if !explained(&got, sql) {
            unexplained.push(format!("{kind}: {got}: {sql:?}"));
        }
        if after != before {
            findings.push(format!(
                "{kind}: allowed as a read, and the side channel changed ({before} → {after}): {sql:?}"
            ));
            // Undo what can be undone (round 1, m6); the next statement takes its own
            // `before`, so one finding never hides or repeats into the next.
            side.restore(&line).await;
        }
    }
    // The console is still alive: no case turned the rest into silent refusals.
    let health = match c.try_run("SELECT 1").await {
        Ok(ran) => ran.outcome(),
        Err(resolution) => format!("not sent: {resolution}"),
    };
    eprintln!(
        "pg17 mutation fuzz: {} inputs ({} corpus), {allowed} allowed on read-only and run; \
         outcomes {outcomes:?}",
        inputs.len(),
        base.len()
    );

    let path = hacivat_testkit::artifact("t20-mutation-fuzz.tsv");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("the artifact directory");
    }
    std::fs::write(&path, record.join("\n") + "\n").expect("the record is written");

    c.shutdown().await;
    let Channel {
        mut root,
        admin,
        sentinel,
        ..
    } = side;
    sentinel.close().await;
    admin.close().await;
    let _ = root
        .try_batch(
            "SELECT pg_drop_replication_slot(slot_name) FROM pg_replication_slots \
             WHERE slot_name LIKE 'hv\\_t20%' AND NOT active",
        )
        .await;
    let _ = root
        .try_batch(&format!("DROP DATABASE IF EXISTS {DB} WITH (FORCE)"))
        .await;
    root.close().await;

    assert_eq!(health, "ran", "the console is no longer healthy");
    assert!(
        outcomes.get("ran").copied().unwrap_or(0) >= 100,
        "too few reads ran: {outcomes:?}"
    );
    assert!(findings.is_empty(), "{}", findings.join("\n"));
    assert!(
        unexplained.is_empty(),
        "unexplained outcomes:\n{}",
        unexplained.join("\n")
    );
}

/// Each side-channel probe fires on its effect, made from the admin connections (never the
/// console), and the restore undoes what it can; then the baseline is stable.
async fn self_check(side: &mut Channel, line: &Line) {
    let a = side.snapshot().await;
    assert_eq!(a, side.snapshot().await, "the side channel is not stable");
    let checks: [(&str, &str); 5] = [
        (
            "a replication slot",
            "SELECT pg_create_physical_replication_slot('hv_t20_selfcheck')",
        ),
        (
            "a server file",
            "COPY (SELECT 1) TO '/tmp/hv_t20_selfcheck'",
        ),
        (
            "a terminated sentinel",
            "SELECT count(pg_terminate_backend(pid)) FROM pg_stat_activity \
             WHERE application_name = 'hv_t20_sentinel'",
        ),
        ("a configuration reload", "SELECT pg_reload_conf()"),
        (
            "a shared statistics reset",
            "SELECT pg_stat_reset_shared('wal')",
        ),
    ];
    for (what, sql) in checks {
        let before = side.snapshot().await;
        side.root.batch(sql).await;
        let mut changed = false;
        // A terminated backend leaves `pg_stat_activity` and a reload is seen a moment later:
        // poll the condition, bounded.
        let deadline = std::time::Instant::now() + secs(10);
        while std::time::Instant::now() < deadline {
            if side.snapshot().await != before {
                changed = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(changed, "the side channel does not see {what}");
        side.restore(line).await;
    }
    // The database's own statistics reset, from its database.
    let before = side.snapshot().await;
    side.admin.batch("SELECT pg_stat_reset()").await;
    assert_ne!(
        before,
        side.snapshot().await,
        "the side channel does not see a statistics reset"
    );
    let b = side.snapshot().await;
    assert_eq!(
        b,
        side.snapshot().await,
        "the side channel is not stable after the restore"
    );
}
