//! M3b Task 14: the classifier pool (R-M3-3, R-M3-53): two std threads with 16 MiB stacks,
//! answers through `oneshot`, every failure `Unknown`; the read-only guard's
//! `StatementClassifier` (R-M3-56, P-M3b-10); units classified one at a time; libpg_query
//! called from both pool threads at once; deep inputs.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_exec::ClassifyPool;
use hacivat_exec::classify_pool::{read_only_check, session_effects};
use hacivat_proto::DbmsKind;
use hacivat_session::session::{RoCheck, SessionEffects, StatementClassifier};
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes, ServerVersion};
use sql_safety::{ClassifyCtx, CoarseClass, Expect, ReasonCode, SafetyReport, classify};

// ---------------------------------------------------------------------------------------------
// Fixtures

fn ctx_for(id: DialectId, expect: Expect) -> ClassifyCtx {
    ClassifyCtx::new(
        DialectCtx {
            id,
            version: (id == DialectId::Postgres).then_some(ServerVersion {
                major: 18,
                minor: 0,
                patch: 0,
            }),
            modes: LexModes::default_for(id),
        },
        ClientFlavor::None,
        expect,
    )
}

fn pg() -> ClassifyCtx {
    ctx_for(DialectId::Postgres, Expect::SingleStatement)
}

fn pool() -> Arc<ClassifyPool> {
    ClassifyPool::new(2)
}

/// `SELECT (SELECT (… (SELECT 1) …))`, `n` deep: brackets within the 256 limit, a tree past
/// prost's decode depth (P-M3b-18).
fn nested_scalar_subqueries(n: usize) -> String {
    format!("SELECT {}1{}", "(SELECT ".repeat(n), ")".repeat(n))
}

fn none() -> Arc<[Arc<str>]> {
    Arc::from(Vec::new())
}

fn reasons(r: &SafetyReport) -> Vec<ReasonCode> {
    r.pre_decision
        .iter()
        .flatten()
        .chain(r.items.iter().flat_map(|i| &i.reasons))
        .map(|x| x.code)
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The plan's tests

#[tokio::test]
async fn pool_classifies_deep_input_without_overflowing() {
    let s = nested_scalar_subqueries(250); // SELECT (SELECT (… (SELECT 1) …)): past prost's decode depth → Unknown (P-M3b-18)
    assert_ne!(
        pool().classify(s.into(), pg()).await.unwrap().request_class,
        CoarseClass::Read
    );
    // Every dialect, deep brackets just inside the L0 limit: the backends' own limits answer,
    // and nothing overflows a pool thread's stack.
    let p = pool();
    for d in [
        DialectId::Postgres,
        DialectId::MySql,
        DialectId::MariaDb,
        DialectId::Sqlite,
    ] {
        for s in [
            nested_scalar_subqueries(250),
            format!("SELECT {}1{}", "(".repeat(256), ")".repeat(256)),
            format!("SELECT {}1{}", "-".repeat(5000), ""),
            format!("SELECT 1{}", " + 1".repeat(20_000)),
        ] {
            let r = p
                .classify(s.clone().into(), ctx_for(d, Expect::SingleStatement))
                .await
                .unwrap();
            assert!(r.matches_text(&s), "{d:?}");
            assert!(
                r.request_class != CoarseClass::Read || r.pre_decision.is_none(),
                "{d:?}"
            );
        }
    }
}

/// The longest operator chains the token limit lets through (200,000 significant tokens):
/// `sqlparser` builds them as left-deep trees, which its derived walk and drop recurse through.
/// Measured while writing this: a 2 MiB thread (a tokio worker's default) aborts the process on
/// `SELECT 1 + 1 + …` from ~50,000 terms in a debug build and below 99,000 in release. Since
/// fix round 1 the interim path refuses chains over `MAX_OPERATOR_CHAIN` before `sqlparser`
/// (C1), and PostgreSQL's parser has its own limits; every one answers on the pool.
#[tokio::test]
async fn the_longest_chains_the_token_limit_allows_answer_on_the_pool() {
    let p = pool();
    let plus = format!("SELECT 1{}", " + 1".repeat(99_000));
    let and = format!(
        "SELECT 1 FROM t WHERE {}",
        vec!["a = 1"; 49_000].join(" AND ")
    );
    let or = format!(
        "SELECT 1 FROM t WHERE {}",
        vec!["a = 1"; 49_000].join(" OR ")
    );
    for d in [
        DialectId::Postgres,
        DialectId::MySql,
        DialectId::MariaDb,
        DialectId::Sqlite,
    ] {
        for s in [&plus, &and, &or] {
            let r = p
                .classify(s.as_str().into(), ctx_for(d, Expect::SingleStatement))
                .await
                .unwrap();
            assert!(r.matches_text(s), "{d:?}");
            // Fix round 1, C1: on the interim path the chains are over the operator cap.
            if d != DialectId::Postgres {
                assert_eq!(r.request_class, CoarseClass::Unknown, "{d:?}");
                assert!(reasons(&r).contains(&ReasonCode::TooComplex), "{d:?}");
            }
        }
    }
    // The reviewer's quadratic chains (C1) answer at once on the pool, Unknown.
    for (d, s) in [
        (
            DialectId::Sqlite,
            format!("SELECT a{}", " REGEXP a".repeat(5_000)),
        ),
        (
            DialectId::MySql,
            format!("SELECT a{}", " DIV a".repeat(20_000)),
        ),
    ] {
        let start = Instant::now();
        let r = p
            .classify(s.as_str().into(), ctx_for(d, Expect::SingleStatement))
            .await
            .unwrap();
        assert!(start.elapsed() < Duration::from_secs(2), "{d:?}");
        assert!(reasons(&r).contains(&ReasonCode::TooComplex), "{d:?}");
    }
}

/// C2 (controller ruling): the read-only guard has no slot for the session's lexing modes, so
/// a text the modes read differently is checked under each, and denied unless every reading
/// allows (the reviewer's repros).
#[tokio::test]
async fn read_only_check_denies_what_any_lexing_mode_reading_denies() {
    let p = pool();
    for (kind, s) in [
        (DbmsKind::MySql, "SELECT 'a\\' INTO OUTFILE '/tmp/x' -- '"),
        (DbmsKind::MariaDb, "SELECT 'a\\' INTO OUTFILE '/tmp/x' -- '"),
        (DbmsKind::MySql, "SELECT \"a\\\" , SLEEP(1) -- \""),
        (
            DbmsKind::Postgres,
            "SELECT 'a\\' || ' , dblink_exec($q$h$q$, $q$DELETE FROM t$q$) --'",
        ),
        (
            DbmsKind::Postgres,
            "SELECT 'a\\' || ' , pg_terminate_backend(1) --'",
        ),
    ] {
        assert!(
            matches!(
                p.check_read_only(kind, s.into(), none()).await,
                RoCheck::Deny(_)
            ),
            "{kind:?} {s}"
        );
    }
    // A plain read keeps its verdict.
    assert_eq!(
        p.check_read_only(DbmsKind::MySql, "SELECT \"a\" FROM t".into(), none())
            .await,
        RoCheck::Allow
    );
}

#[tokio::test]
async fn read_only_check_allows_select_denies_insert() {
    let p = pool();
    for kind in [
        DbmsKind::Postgres,
        DbmsKind::MySql,
        DbmsKind::MariaDb,
        DbmsKind::Sqlite,
    ] {
        assert_eq!(
            p.check_read_only(kind, "SELECT 1".into(), none()).await,
            RoCheck::Allow,
            "{kind:?}"
        );
        match p
            .check_read_only(kind, "INSERT INTO t VALUES (1)".into(), none())
            .await
        {
            RoCheck::Deny(d) => {
                assert_eq!(d.class_id, "safety-class-write", "{kind:?}");
                assert!(!d.reasons.is_empty(), "{kind:?}");
                assert!(d.reasons.iter().all(|r| r.starts_with("safety-reason-")));
            }
            RoCheck::Allow => panic!("{kind:?}: INSERT allowed on read-only"),
        }
    }
    // The profile's allowlist (P-M3b-10) lets a user function through, and nothing else.
    let q = "SELECT report_total(1)";
    assert!(matches!(
        p.check_read_only(DbmsKind::Postgres, q.into(), none())
            .await,
        RoCheck::Deny(_)
    ));
    let allow: Arc<[Arc<str>]> = Arc::from(vec![Arc::from("report_total")]);
    assert_eq!(
        p.check_read_only(DbmsKind::Postgres, q.into(), Arc::clone(&allow))
            .await,
        RoCheck::Allow
    );
    assert!(matches!(
        p.check_read_only(DbmsKind::Postgres, "SELECT other_fn(1)".into(), allow)
            .await,
        RoCheck::Deny(_)
    ));
    // A run of two statements is not one statement; neither is nothing, nor a PG-only trick.
    for s in [
        "SELECT 1; SELECT 2",
        "",
        "SELECT 1; COMMIT; DROP TABLE t",
        "COPY t TO STDOUT",
    ] {
        assert!(
            matches!(
                p.check_read_only(DbmsKind::Postgres, s.into(), none())
                    .await,
                RoCheck::Deny(_)
            ),
            "{s:?}"
        );
    }
    // Engines without a classifier deny everything.
    for kind in [
        DbmsKind::MsSql,
        DbmsKind::Oracle,
        DbmsKind::ClickHouse,
        DbmsKind::Unknown,
    ] {
        match p.check_read_only(kind, "SELECT 1".into(), none()).await {
            RoCheck::Deny(d) => {
                assert_eq!(d.class_id, "safety-class-unknown");
                assert_eq!(d.reasons, ["safety-reason-unsupported-dialect"], "{kind:?}");
            }
            RoCheck::Allow => panic!("{kind:?}"),
        }
    }
}

#[test]
#[ignore = "perf"]
fn classify_10kb_unit_under_1ms() {
    if std::env::var("HACIVAT_PERF").as_deref() != Ok("1") {
        return;
    }
    // Through the pool: one round trip per 10 KB unit adds no more than the pool's queueing
    // to `sql-safety`'s own figure (`sql-safety --test classify`, interim 2 ms on the best).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let p = pool();
    let mut cols = Vec::new();
    let mut k = 0;
    while cols.iter().map(|c: &String| c.len() + 2).sum::<usize>() < 10_000 {
        cols.push(format!(
            "CASE WHEN o.id > {k} THEN UPPER(c.full_name) ELSE LOWER(c.email) END AS c{k}"
        ));
        k += 1;
    }
    let unit: Arc<str> = format!(
        "SELECT {} FROM customers c JOIN orders o ON o.customer_id = c.id",
        cols.join(", ")
    )
    .into();
    for d in [DialectId::Postgres, DialectId::MySql, DialectId::Sqlite] {
        let ctx = ctx_for(d, Expect::SingleStatement);
        let mut times: Vec<f64> = (0..100)
            .map(|_| {
                let start = Instant::now();
                let r = rt
                    .block_on(p.classify(Arc::clone(&unit), ctx.clone()))
                    .unwrap();
                assert_eq!(r.request_class, CoarseClass::Read);
                start.elapsed().as_secs_f64()
            })
            .collect();
        times.sort_by(f64::total_cmp);
        let per = |t: f64| t / (unit.len() as f64 / 10_240.0) * 1e3;
        let (best, p95) = (per(times[0]), per(times[95]));
        eprintln!("{d:?}: pool classify best {best:.3}, p95 {p95:.3} ms per 10 KB");
        assert!(best < 2.0, "{d:?}: {best:.3} ms per 10 KB");
    }
}

// ---------------------------------------------------------------------------------------------
// More: the pool fails closed

#[tokio::test]
async fn the_pool_runs_two_threads_and_answers_what_classify_answers() {
    let p = pool();
    assert_eq!(p.threads(), 2);
    for (s, d) in [
        ("SELECT 1", DialectId::Postgres),
        ("DELETE FROM t", DialectId::Postgres),
        ("SELECT 1", DialectId::MySql),
        ("UPDATE t SET a = 1", DialectId::Sqlite),
        ("", DialectId::Postgres),
    ] {
        let ctx = ctx_for(d, Expect::SingleStatement);
        let r = p.classify(s.into(), ctx.clone()).await.unwrap();
        assert_eq!(r, classify(s, &ctx), "{s:?} on {d:?}");
    }
    assert_eq!(ClassifyPool::new(0).threads(), 1, "at least one thread");
}

#[tokio::test]
async fn a_shut_down_pool_answers_unknown_and_never_reads() {
    let p = pool();
    p.shutdown();
    let r = p.classify("SELECT 1".into(), pg()).await.unwrap();
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert_eq!(reasons(&r), [ReasonCode::EffectsUnknown]);
    assert!(r.matches_text("SELECT 1"));
    assert_eq!(r.dialect, Some(DialectId::Postgres));
    let within = p
        .classify_within("SELECT 1".into(), pg(), Duration::from_secs(5))
        .await;
    assert_eq!(within.request_class, CoarseClass::Unknown);
    let units = p
        .classify_units(vec!["SELECT 1".into(), "SELECT 2".into()], pg())
        .await
        .unwrap();
    assert_eq!(units.len(), 2);
    assert!(
        units
            .iter()
            .all(|r| r.request_class == CoarseClass::Unknown)
    );
    assert!(matches!(
        p.check_read_only(DbmsKind::Postgres, "SELECT 1".into(), none())
            .await,
        RoCheck::Deny(_)
    ));
    assert_eq!(
        p.effects(DbmsKind::Postgres, "SET a = 1".into()).await,
        None
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn both_threads_classify_postgres_at_once_through_the_one_parser_thread() {
    // Two pool threads, many PG jobs in flight: every libpg_query call queues on the oracle's
    // serial thread, so each answer equals the one classified alone.
    let p = pool();
    let texts: Vec<String> = (0..64)
        .map(|i| match i % 4 {
            0 => format!("SELECT a, b FROM t{i} WHERE id = {i}"),
            1 => format!("DELETE FROM t{i} WHERE id = {i}"),
            2 => format!("SELECT lower(name) FROM t{i} JOIN u ON u.id = t{i}.uid"),
            _ => format!("SELEC broken {i}"),
        })
        .collect();
    let expected: Vec<SafetyReport> = texts.iter().map(|s| classify(s, &pg())).collect();
    let pending: Vec<_> = texts
        .iter()
        .map(|s| p.classify(s.as_str().into(), pg()))
        .collect();
    for (rx, want) in pending.into_iter().zip(&expected) {
        assert_eq!(&rx.await.unwrap(), want);
    }
    // Concurrent unit runs, too.
    let a = p.classify_units(texts[..8].iter().map(|s| s.as_str().into()).collect(), pg());
    let b = p.classify_units(
        texts[8..16].iter().map(|s| s.as_str().into()).collect(),
        pg(),
    );
    let (a, b) = (a.await.unwrap(), b.await.unwrap());
    assert_eq!(a.len() + b.len(), 16);
}

#[tokio::test]
async fn units_run_in_order_and_a_context_change_taints_the_rest() {
    let p = pool();
    let reports = p
        .classify_units(
            vec![
                "SELECT lower('a')".into(),
                "SET search_path = evil, pg_catalog".into(),
                "SELECT lower('a')".into(),
            ],
            pg(),
        )
        .await
        .unwrap();
    assert_eq!(reports[0].request_class, CoarseClass::Read);
    assert_eq!(reports[2].request_class, CoarseClass::Unknown);
    assert!(reports[2].matches_text("SELECT lower('a')"));
    // The carry's FETCH: the unit that runs the cursor taints what follows.
    let reports = p
        .classify_units(vec!["FETCH c".into(), "SELECT 1".into()], pg())
        .await
        .unwrap();
    assert_eq!(reports[1].request_class, CoarseClass::Unknown);
}

// ---------------------------------------------------------------------------------------------
// Effects for the session (C8, Smart)

#[tokio::test]
async fn effects_map_to_session_effects_and_unknown_is_none() {
    let p = pool();
    let pg_cases: &[(&str, Option<SessionEffects>)] = &[
        ("SELECT 1", Some(SessionEffects::empty())),
        (
            "SET statement_timeout = 1000",
            Some(SessionEffects::SESSION_STATE),
        ),
        (
            "INSERT INTO t VALUES (1)",
            Some(SessionEffects::OPENS_WRITE),
        ),
        ("PREPARE p AS SELECT 1", Some(SessionEffects::PREPARE)),
        ("CALL p()", None),
        ("SELECT f(1)", None),
        ("", None),
    ];
    for (s, want) in pg_cases {
        let got = p.effects(DbmsKind::Postgres, (*s).into()).await;
        assert_eq!(got, *want, "{s:?}");
    }
    let ddl = p
        .effects(DbmsKind::Postgres, "CREATE TABLE t (a int)".into())
        .await
        .unwrap();
    assert!(ddl.contains(SessionEffects::DDL | SessionEffects::OPENS_WRITE));
    let temp = p
        .effects(DbmsKind::Postgres, "CREATE TEMP TABLE t (a int)".into())
        .await
        .unwrap();
    assert!(temp.contains(SessionEffects::WRITE_TEMP), "{temp:?}");
    let lock = p
        .effects(DbmsKind::MySql, "LOCK TABLES t WRITE".into())
        .await;
    assert!(
        lock.is_none_or(|e| e.contains(SessionEffects::LOCK_TABLES)),
        "{lock:?}"
    );
    // The pure mapping agrees with the pool's.
    let r = classify("SELECT 1", &pg());
    assert_eq!(session_effects(&r), Some(SessionEffects::empty()));
    assert_eq!(read_only_check(&r), RoCheck::Allow);
}
