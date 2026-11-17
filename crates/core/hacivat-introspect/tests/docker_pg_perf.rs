//! Spike measurement: introspection time on a generated schema of 10,000 tables and 160,000
//! columns (PostgreSQL 17), and the incremental refresh after one `ALTER`. Runs only with
//! `HACIVAT_PERF=1` (it creates and drops ~10k tables); the timings go to stderr and to the
//! Docker artifact `m4-spike-pg-intro-perf.txt`.
//!
//! `HACIVAT_PERF=1 run-docker-tests.sh --set pr -p hacivat-introspect --test docker_pg_perf`

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

mod common;

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use common::*;
use hacivat_introspect::pg::*;
use hacivat_testkit::{DockerTarget, artifact, perf_enabled, require_target};

const TABLES: u32 = 10_000;
/// Tables per transaction: each takes several locks (table, toast, toast index, PK index);
/// one transaction for all of them would exhaust the lock table (`max_locks_per_transaction`).
const BATCH: u32 = 250;

/// 16 columns per table, every family the decoder handles; a PK on `c1`; every 10th table a
/// foreign key to the previous one and a comment.
fn create_batch(s: &str, from: u32, to: u32) -> String {
    format!(
        "DO $$ BEGIN FOR i IN {from}..{to} LOOP \
         EXECUTE format('CREATE TABLE %I.%I (c1 int PRIMARY KEY, c2 bigint NOT NULL, c3 text, \
         c4 varchar(40), c5 numeric(12,2) DEFAULT 0, c6 timestamptz DEFAULT now(), c7 boolean, \
         c8 jsonb, c9 int[], c10 uuid, c11 date, c12 double precision, c13 smallint, \
         c14 bytea, c15 interval, c16 text COLLATE \"C\")', '{s}', 't' || i); \
         IF i % 10 = 0 AND i > 1 THEN \
         EXECUTE format('ALTER TABLE %I.%I ADD CONSTRAINT %I FOREIGN KEY (c2) REFERENCES %I.%I (c1)', \
         '{s}', 't' || i, 'fk' || i, '{s}', 't' || (i - 1)); \
         EXECUTE format('COMMENT ON TABLE %I.%I IS %L', '{s}', 't' || i, 'table ' || i); \
         END IF; END LOOP; END $$"
    )
}

fn drop_batch(s: &str, from: u32, to: u32) -> String {
    format!(
        "DO $$ BEGIN FOR i IN {from}..{to} LOOP \
         EXECUTE format('DROP TABLE IF EXISTS %I.%I CASCADE', '{s}', 't' || i); \
         END LOOP; END $$"
    )
}

async fn drop_all(t: &DockerTarget, s: &str) {
    let mut c = connect(t).await;
    let mut to = TABLES;
    while to >= 1 {
        let from = to.saturating_sub(BATCH - 1).max(1);
        run(&mut c, &drop_batch(s, from, to)).await;
        to = from - 1;
    }
    run(&mut c, &format!("DROP SCHEMA IF EXISTS {s} CASCADE")).await;
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn sections(out: &mut String, stats: &PassStats) {
    for s in &stats.sections {
        let _ = writeln!(
            out,
            "    {:<28} rows {:>7}  query {:>10}  decode {:>9}",
            s.name,
            s.rows,
            ms(s.query),
            ms(s.decode)
        );
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn ten_thousand_tables_full_and_incremental() {
    if !perf_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let s = unique_name("hvpgi_perf");
    guarded(
        async {
            let mut c = connect(&t).await;
            let mut out = String::new();
            let version = scalar(&mut c, "SELECT version()").await;
            let _ = writeln!(out, "server: {version}");
            run(&mut c, &format!("CREATE SCHEMA {s}")).await;
            let t0 = Instant::now();
            let mut from = 1;
            while from <= TABLES {
                let to = (from + BATCH - 1).min(TABLES);
                run(&mut c, &create_batch(&s, from, to)).await;
                from = to + 1;
            }
            let _ = writeln!(out, "fixture created in {}", ms(t0.elapsed()));
            run(
                &mut c,
                "ANALYZE pg_catalog.pg_class, pg_catalog.pg_attribute",
            )
            .await;
            let oid = schema_oid(&mut c, &s).await;

            let mut last = None;
            for (label, opts) in [
                ("full, Structure (cold)", IntrospectOpts { sources: false }),
                ("full, Structure (warm)", IntrospectOpts { sources: false }),
                ("full, Sources", IntrospectOpts { sources: true }),
            ] {
                let pass = full(&mut c, oid, opts).await.unwrap();
                assert!(!pass.stats.xid_assigned);
                let ns = &pass.result;
                let cols: usize = ns.relations.iter().map(|r| r.columns.len()).sum();
                assert_eq!(ns.relations.len(), TABLES as usize);
                assert_eq!(cols, (TABLES * 16) as usize);
                let _ = writeln!(
                    out,
                    "{label}: {} ({} relations, {cols} columns)",
                    ms(pass.stats.total),
                    ns.relations.len()
                );
                sections(&mut out, &pass.stats);
                last = Some(pass);
            }
            let base = last.unwrap();
            let opts = IntrospectOpts { sources: true };

            let pass = refresh(&mut c, &base.result, Some(&base.watermark), opts)
                .await
                .unwrap();
            let Refresh::Delta(d) = &pass.result else {
                panic!("expected a delta");
            };
            assert!(d.reads_nothing(), "{d:?}");
            let _ = writeln!(out, "incremental, no change: {}", ms(pass.stats.total));
            sections(&mut out, &pass.stats);

            run(
                &mut c,
                &format!("ALTER TABLE {s}.t5000 ADD COLUMN extra int"),
            )
            .await;
            let pass = refresh(&mut c, &base.result, Some(&pass.watermark), opts)
                .await
                .unwrap();
            assert!(!pass.stats.xid_assigned);
            let Refresh::Delta(d) = &pass.result else {
                panic!("expected a delta");
            };
            let patched: Vec<u32> = d.relations.iter().map(|p| p.oid).collect();
            let _ = writeln!(
                out,
                "incremental after ALTER TABLE t5000 ADD COLUMN: {} ({} relation(s) re-read)",
                ms(pass.stats.total),
                patched.len()
            );
            sections(&mut out, &pass.stats);
            let t1 = Instant::now();
            let merged = apply(&base.result, d);
            let _ = writeln!(out, "apply (reference merge): {}", ms(t1.elapsed()));
            let now = full(&mut c, oid, opts).await.unwrap();
            assert_eq!(merged, now.result, "incremental + apply equals full");

            eprintln!("{out}");
            std::fs::write(artifact("m4-spike-pg-intro-perf.txt"), &out).unwrap();
        },
        drop_all(&t, &s),
    )
    .await;
}
