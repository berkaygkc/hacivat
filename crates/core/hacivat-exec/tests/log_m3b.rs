//! M3b Task 16: the full log message set (HV-EXEC-11): every new line's Fluent ID in English and
//! Turkish, `lines_since`, DDL `exec-completed` on every engine (ledger 340, PO), the command
//! tag kept, server notices and MySQL warnings verbatim, notices past the cap, a Run as one
//! stopped early or failed on PostgreSQL, a retried statement, an eviction, and the one
//! duration rule.

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use common::m3b::{request, run_all, units};
use common::{fake_pg, generated, pg_error, service_over};
use hacivat_driver::fake::{FakeDriver, FakeQuery};
use hacivat_driver::{Notice, NoticeSeverity};
use hacivat_exec::{
    ErrorPolicy, LogLine, LogMsg, RunChoice, RunRequest, SessionLog, copy_invariant,
    duration_message,
};
use hacivat_proto::DbmsKind;
use hacivat_session::session::RunEvent;

/// The message IDs of `locale`'s `hacivat-core.ftl`.
fn ftl_ids(locale: &str) -> HashSet<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../locales")
        .join(locale)
        .join("hacivat-core.ftl");
    let text = std::fs::read_to_string(&path).expect("the FTL file");
    text.lines()
        .filter_map(|l| {
            let (id, _) = l.split_once(" =")?;
            (!id.starts_with(['#', ' ']) && !id.is_empty()).then(|| id.to_string())
        })
        .collect()
}

/// Every line Task 16 writes (D10: the run service owns the log lines' IDs). The classifier's
/// denial (`exec-deny-classifier`, `exec-deny-read-only-classifier`: Task 18) and the
/// confirm-in-transaction and implicit-commit lines (`exec-confirm-*`, `exec-implicit-commit`:
/// Task 19) come with their tasks.
fn t16_messages() -> Vec<LogMsg> {
    let mut all: Vec<LogMsg> = [
        NoticeSeverity::Debug,
        NoticeSeverity::Log,
        NoticeSeverity::Info,
        NoticeSeverity::Notice,
        NoticeSeverity::Warning,
    ]
    .into_iter()
    .map(|severity| LogMsg::Notice {
        severity,
        code: None,
        text: "".into(),
    })
    .collect();
    all.extend([
        LogMsg::ResultSetLimit {
            total: 150,
            limit: 100,
        },
        LogMsg::SkippedByUser { n: 2 },
        LogMsg::Retried { n: 3 },
        LogMsg::Evicted { rows: 1_000 },
        LogMsg::NoticesDropped { n: 4 },
        LogMsg::ClosedEarly {
            ran: 2,
            rest_not_run: false,
        },
        LogMsg::ClosedEarly {
            ran: 2,
            rest_not_run: true,
        },
        LogMsg::RunAsOneRolledBack,
        LogMsg::Dropped { n: 3 },
    ]);
    all
}

#[test]
fn every_new_log_id_exists_in_en_and_tr() {
    let (en, tr) = (ftl_ids("en"), ftl_ids("tr"));
    let mut ids: Vec<&'static str> = t16_messages().iter().map(LogMsg::fluent_id).collect();
    ids.extend([1_000, 2_340, 185_400, 0, 999].map(|ms| duration_message(ms).id));
    for id in ids {
        assert!(en.contains(id), "{id} is missing in en");
        assert!(tr.contains(id), "{id} is missing in tr");
    }
    // A notice's sentence takes no argument: its code and text are the server's.
    for m in t16_messages() {
        if let LogMsg::Notice { .. } = m {
            assert!(m.args().is_empty());
        }
    }
}

#[test]
fn lines_since_reports_a_gap_after_the_ring_wraps() {
    let log = SessionLog::new();
    for _ in 0..10 {
        log.append(LogMsg::TxCommitted, None, None);
    }
    let (lines, gap) = log.lines_since(4);
    assert_eq!(lines.first().map(|l| l.seq), Some(4));
    assert_eq!(lines.len(), 6);
    assert!(!gap);
    // A reader that last saw line 9 reads on after the ring wrapped past it: lines 0..=19
    // left the ring for room.
    for _ in 0..SessionLog::CAPACITY + 10 {
        log.append(LogMsg::TxRolledBack, None, None);
    }
    let (lines, gap) = log.lines_since(10);
    assert!(gap, "lines 10..=19 left the ring unread");
    assert_eq!(lines.first().map(|l| l.seq), Some(20));
    assert_eq!(lines.len(), SessionLog::CAPACITY);
    // A reader that already saw them reads on without a gap.
    assert!(!log.lines_since(20).1);
    // Reading on from the newest line is no gap.
    let next = lines.last().unwrap().seq + 1;
    log.append(LogMsg::TxCommitted, None, None);
    let (lines, gap) = log.lines_since(next);
    assert_eq!(lines.len(), 1);
    assert!(!gap);
}

/// `CREATE TABLE` on `kind`, whose server counts `affected` rows for it.
async fn ddl_on(kind: DbmsKind, affected: Option<u64>) -> Vec<&'static str> {
    let fake = FakeDriver::new(kind)
        .tracks_transactions()
        .queries(vec![FakeQuery {
            prefix: "CREATE",
            rows_affected: affected,
            ..Default::default()
        }]);
    let (b, s, _f) = service_over(fake).await;
    let mut t = b.svc.run(&s, b.req(&["CREATE TABLE t (a int)"]));
    (&mut t.done).await.unwrap();
    b.log_ids()
}

#[tokio::test]
async fn ddl_logs_completed_on_every_engine() {
    for (kind, affected) in [
        (DbmsKind::Postgres, None),
        (DbmsKind::MySql, Some(0)),
        (DbmsKind::MariaDb, Some(0)),
        (DbmsKind::Sqlite, Some(0)),
    ] {
        assert_eq!(ddl_on(kind, affected).await, ["exec-completed"], "{kind:?}");
    }
    // A DML statement keeps its count.
    let fake = FakeDriver::new(DbmsKind::MySql).queries(vec![FakeQuery {
        prefix: "UPDATE",
        rows_affected: Some(0),
        ..Default::default()
    }]);
    let (b, s, _f) = service_over(fake).await;
    let mut t = b.svc.run(&s, b.req(&["UPDATE t SET a = 1"]));
    (&mut t.done).await.unwrap();
    assert_eq!(b.log_ids(), ["exec-rows-affected"]);
}

fn notice(severity: NoticeSeverity, code: &str, text: &str) -> Notice {
    Notice::new(severity, Some(code), text)
}

#[tokio::test]
async fn notices_and_warnings_reach_the_log_verbatim() {
    // PostgreSQL: notices, the text as the server sent it.
    let text = "tablo \"İŞLEM\" yok, atlanıyor\nikinci satır";
    let fake = fake_pg().queries(vec![FakeQuery {
        prefix: "DROP",
        notices: vec![notice(NoticeSeverity::Notice, "00000", text)],
        ..Default::default()
    }]);
    let (b, s, _f) = service_over(fake).await;
    let mut t = b.svc.run(&s, b.req(&["DROP TABLE IF EXISTS işlem"]));
    (&mut t.done).await.unwrap();
    let msgs = b.log_msgs();
    assert!(
        msgs.contains(&LogMsg::Notice {
            severity: NoticeSeverity::Notice,
            code: Some("00000".into()),
            text: text.into(),
        }),
        "{msgs:?}"
    );
    let copied = copy_invariant(&b.log.lines());
    assert!(
        copied.contains(
            "exec-notice-notice\tcode=00000\ttablo \"İŞLEM\" yok, atlanıyor\\nikinci satır"
        )
    );
    // MySQL: the warnings of a statement are read after it (read-write sessions only).
    let fake = FakeDriver::new(DbmsKind::MySql).queries(vec![FakeQuery {
        prefix: "INSERT",
        rows_affected: Some(1),
        warnings: vec![notice(
            NoticeSeverity::Warning,
            "1265",
            "Data truncated for column 'a' at row 1",
        )],
        ..Default::default()
    }]);
    let (b, s, _f) = service_over(fake).await;
    let mut t = b.svc.run(&s, b.req(&["INSERT INTO t VALUES ('uzun')"]));
    (&mut t.done).await.unwrap();
    let ids = b.log_ids();
    assert!(ids.contains(&"exec-notice-warning"), "{ids:?}");
    // Past the per-statement cap (100), the rest is counted, never shown.
    let many = (0..105)
        .map(|i| notice(NoticeSeverity::Notice, "00000", &format!("n{i}")))
        .collect();
    let fake = fake_pg().queries(vec![FakeQuery {
        prefix: "DO",
        notices: many,
        ..Default::default()
    }]);
    let (b, s, _f) = service_over(fake).await;
    let mut t = b.svc.run(&s, b.req(&["DO $$ BEGIN END $$"]));
    (&mut t.done).await.unwrap();
    let msgs = b.log_msgs();
    assert_eq!(
        msgs.iter()
            .filter(|m| matches!(m, LogMsg::Notice { .. }))
            .count(),
        100
    );
    assert!(
        msgs.contains(&LogMsg::NoticesDropped { n: 5 }),
        "{:?}",
        b.log_ids()
    );
}

#[tokio::test]
async fn a_failed_pg_run_as_one_says_the_statements_before_were_rolled_back() {
    let failing = FakeQuery {
        prefix: "INSERT INTO a",
        sets: vec![generated(1)],
        fail_after: Some((0, pg_error("23505", "duplicate key value"))),
        ..Default::default()
    };
    for (kind, expected) in [(DbmsKind::Postgres, true), (DbmsKind::MySql, false)] {
        let fake = FakeDriver::new(kind)
            .tracks_transactions()
            .queries(vec![failing.clone()]);
        let (b, s, _f) = service_over(fake).await;
        let req = RunRequest {
            run_as_one: true,
            ..request(
                &b,
                units(&["INSERT INTO a VALUES (1); INSERT INTO a VALUES (1)"]),
                run_all(1),
            )
        };
        let mut t = b.svc.run(&s, req);
        (&mut t.done).await.unwrap();
        let ids = b.log_ids();
        assert_eq!(
            ids.contains(&"exec-run-as-one-rolled-back"),
            expected,
            "{kind:?}: {ids:?}"
        );
    }
    // The same failure not run as one says nothing more.
    let (b, s, _f) = service_over(fake_pg().queries(vec![failing])).await;
    let mut t = b.svc.run(
        &s,
        request(&b, units(&["INSERT INTO a VALUES (1)"]), run_all(1)),
    );
    (&mut t.done).await.unwrap();
    assert_eq!(b.log_ids(), ["exec-error"]);
}

#[tokio::test]
async fn a_retried_statement_is_logged() {
    let fake = fake_pg().queries(vec![FakeQuery {
        prefix: "UPDATE nope",
        sets: vec![generated(1)],
        fail_after: Some((0, pg_error("42P01", "relation \"nope\" does not exist"))),
        ..Default::default()
    }]);
    let (b, s, _f) = service_over(fake).await;
    let mut req = request(
        &b,
        units(&["UPDATE nope SET a = 1", "UPDATE b SET a = 1"]),
        run_all(2),
    );
    req.policy = ErrorPolicy::Ask;
    let mut t = b.svc.run(&s, req);
    let (seq, q) = {
        let p = t.choices.wait_for(Option::is_some).await.unwrap();
        match p.clone() {
            Some((seq, RunEvent::NeedsChoice { question, .. })) => (seq, question),
            other => panic!("{other:?}"),
        }
    };
    assert!(s.choose(seq, q, RunChoice::Retry).await.unwrap());
    // The script fails it once: the retry succeeds, and the run goes on.
    let summary = (&mut t.done).await.unwrap();
    assert_eq!((summary.executed, summary.failed), (2, 0));
    assert_eq!(
        b.log_ids(),
        [
            "exec-error",
            "exec-retried",
            "exec-completed",
            "exec-completed",
            "exec-summary"
        ]
    );
    assert!(
        b.log_msgs().contains(&LogMsg::Retried { n: 1 }),
        "{:?}",
        b.log_ids()
    );
}

#[tokio::test]
async fn an_evicted_result_is_logged_on_its_console() {
    let fake = fake_pg().queries(vec![FakeQuery {
        prefix: "SELECT",
        sets: vec![generated(20_000)],
        ..Default::default()
    }]);
    let (b, s, _f) = service_over(fake).await;
    let req = RunRequest {
        units: units(&["SELECT * FROM big"]),
        ..b.req(&[])
    };
    let mut t = b.svc.run(&s, req);
    (&mut t.done).await.unwrap();
    let tab = t.results.borrow()[0].clone();
    assert!(tab.source.snapshot().row_count() > 1_000);
    tab.source.set_visible(false);
    assert!(b.svc.budget().evict_for(u64::MAX) > 0);
    let evicted = b
        .log
        .lines()
        .iter()
        .find(|l| matches!(l.msg, LogMsg::Evicted { .. }))
        .cloned()
        .expect("the eviction is logged");
    assert_eq!(evicted.unit, Some(tab.unit));
    assert_eq!(evicted.source, Some(0..17));
    let LogMsg::Evicted { rows } = evicted.msg else {
        unreachable!()
    };
    assert_eq!(rows, tab.source.snapshot().row_count());
}

/// The idle bound of a released cursor is a setting: it reads in whole minutes and seconds,
/// where a run's time follows the one duration rule.
#[test]
fn an_idle_bound_reads_as_its_setting() {
    let span = |ms| {
        let m = hacivat_exec::span_message(ms);
        (m.id, m.args)
    };
    assert_eq!(span(600_000), ("exec-dur-min", vec![("min", 10)]));
    assert_eq!(
        span(90_500),
        ("exec-dur-min-s", vec![("min", 1), ("s", 30)])
    );
    assert_eq!(span(2_500).0, "exec-dur-seconds");
    assert_eq!(duration_message(600_000).id, "exec-dur-seconds");
}

#[test]
fn the_command_tag_is_copied_verbatim() {
    let log = SessionLog::new();
    log.append_tagged(
        LogMsg::RowsAffected { rows: 3, ms: 1_234 },
        Some(Arc::from("INSERT 0 3")),
        None,
        None,
    );
    let copied = copy_invariant(&log.lines());
    assert!(
        copied.contains("exec-rows-affected\trows=3\tduration=1234ms\ttag=INSERT 0 3"),
        "{copied}"
    );
    let line: &LogLine = &log.lines()[0];
    assert_eq!(line.tag.as_deref(), Some("INSERT 0 3"));
}
