//! M3a Task 23: the output-log model (HV-EXEC-11): the ring and its drop count, the
//! locale-free copy, and every message ID in the English and Turkish core files.

use std::time::{Duration, UNIX_EPOCH};

use hacivat_driver::ServerMessage;
use hacivat_exec::{
    ErrorView, LogArg, LogLine, LogMsg, RefuseReason, SessionLog, Severity, copy_invariant,
    duration_message, error_title_id, error_view, size_message, uncertain_message_id,
};
use hacivat_proto::DbmsKind;
use hacivat_results::CloseReason;
use hacivat_session::GapReason;
use hacivat_session::session::{DenyLayer, ExecError, ExecErrorKind, RunSummary, TxError};
use sql_split::AmbiguityReason;

fn line(seq: u64, msg: LogMsg) -> LogLine {
    LogLine {
        seq,
        at: UNIX_EPOCH,
        severity: msg.severity(),
        msg,
        source: None,
        unit: None,
    }
}

#[test]
fn log_ring_keeps_10k_and_counts_drops() {
    let log = SessionLog::new();
    let mut changes = log.changes();
    let cap = SessionLog::CAPACITY as u64;
    // A burst outruns the ring before anyone reads it: 5 lines are lost unseen.
    for seq in 0..cap + 5 {
        log.push(line(seq, LogMsg::Completed { ms: seq }));
    }
    let lines = log.lines();
    assert_eq!(lines.len(), SessionLog::CAPACITY + 1);
    assert_eq!(lines[0].msg, LogMsg::Dropped { n: 5 });
    assert_eq!(lines[0].msg.fluent_id(), "exec-events-dropped");
    assert_eq!(lines[0].severity, Severity::Warning);
    assert_eq!(lines[1].seq, 5);
    assert_eq!(lines[SessionLog::CAPACITY].seq, cap + 4);
    assert!(changes.has_changed().unwrap());
    assert_eq!(*changes.borrow_and_update(), cap + 5);
    // Lines read before they leave are not lost: the count stays.
    for seq in cap + 5..cap + 8 {
        log.push(line(seq, LogMsg::Completed { ms: seq }));
    }
    let lines = log.lines();
    assert_eq!(lines.len(), SessionLog::CAPACITY + 1);
    assert_eq!(lines[0].msg, LogMsg::Dropped { n: 5 });
    assert_eq!(lines[1].seq, 8);
    // Another burst adds to it.
    for seq in cap + 8..2 * cap + 10 {
        log.push(line(seq, LogMsg::Completed { ms: seq }));
    }
    assert_eq!(log.lines()[0].msg, LogMsg::Dropped { n: 5 + 2 });
    // Clear starts over.
    log.clear();
    assert!(log.lines().is_empty());
    assert!(changes.has_changed().unwrap());
    log.append(LogMsg::TxCommitted, Some(3..9), None);
    let lines = log.lines();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].msg, LogMsg::TxCommitted);
    assert_eq!(lines[0].source, Some(3..9));
    assert_eq!(lines[0].seq, 2 * cap + 10, "append continues the sequence");
}

fn server_error(text: &str) -> ExecError {
    ExecError {
        kind: ExecErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate: Some("42P01".into()),
            vendor_code: None,
            severity: Some("ERROR".into()),
            text: text.into(),
            detail: None,
            hint: None,
            lang_hint: None,
        }),
    }
}

#[test]
fn copy_invariant_is_iso_and_ascii_under_turkish() {
    // 2024-11-15 05:20:30.123 UTC.
    let at = UNIX_EPOCH + Duration::from_millis(1_731_648_030_123);
    let server = "\"İŞLEMLER\" ilişkisi yok\nsatır 1";
    let view = error_view(
        DbmsKind::Postgres,
        &server_error(server),
        "SELECT * FROM İŞLEMLER",
        0,
        None,
    );
    let lines = [
        LogLine {
            seq: 0,
            at,
            severity: Severity::Info,
            msg: LogMsg::RowsRetrieved {
                rows: 1_234_567,
                more: true,
                exec_ms: 1_200,
                fetch_ms: 300,
            },
            source: Some(0..8),
            unit: None,
        },
        LogLine {
            seq: 1,
            at: at + Duration::from_secs(86_400 * 47),
            severity: Severity::Error,
            msg: LogMsg::Error { n: 3, view },
            source: None,
            unit: None,
        },
    ];
    let text = copy_invariant(&lines);
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(
        rows[0],
        "2024-11-15T05:20:30.123Z\tinfo\texec-rows-retrieved-more\trows=1234567\tduration=1500ms"
    );
    assert_eq!(
        rows[1],
        "2025-01-01T05:20:30.123Z\terror\texec-error\tn=3\texec-error-server\tcode=42P01\t\"İŞLEMLER\" ilişkisi yok\\nsatır 1"
    );
    assert_eq!(
        rows.len(),
        2,
        "a line break in server text stays inside its line"
    );
    // Everything Hacivat wrote is ASCII: digits ungrouped, no locale casing; the server's
    // text is verbatim (never case-mapped).
    let ours = rows[0];
    assert!(ours.is_ascii());
    assert!(rows[1].ends_with(&server.replace('\n', "\\n")));
}

#[test]
fn durations_and_sizes_are_one_message_with_the_unit_inside() {
    let dur = |ms| {
        let m = duration_message(ms);
        (m.id, m.args)
    };
    assert_eq!(dur(0), ("exec-dur-lt-ms", vec![]));
    assert_eq!(dur(340), ("exec-dur-ms", vec![("ms", 340)]));
    assert_eq!(dur(2_000), ("exec-dur-s", vec![("s", 2)]));
    assert_eq!(dur(2_340), ("exec-dur-s-ms", vec![("s", 2), ("ms", 340)]));
    assert_eq!(dur(180_000), ("exec-dur-min", vec![("min", 3)]));
    assert_eq!(dur(185_400), ("exec-dur-min-s", vec![("min", 3), ("s", 5)]));
    assert_eq!(
        dur(2 * 3_600_000 + 1_000),
        ("exec-dur-min-s", vec![("min", 120), ("s", 1)])
    );
    let size = |b| {
        let m = size_message(b);
        (m.id, m.args)
    };
    assert_eq!(size(512), ("exec-size-kb", vec![("n", 5)]));
    assert_eq!(size(12_800), ("exec-size-kb", vec![("n", 125)]));
    assert_eq!(size(5_452_595), ("exec-size-mb", vec![("n", 52)]));
    assert_eq!(size(256 << 20), ("exec-size-mb", vec![("n", 2_560)]));
    assert_eq!(size(3 << 30), ("exec-size-gb", vec![("n", 30)]));
    // Never "1024.0 KB" or "1024.0 MB": a value that rounds to the next unit is shown in it.
    assert_eq!(size((1 << 20) - 1), ("exec-size-mb", vec![("n", 10)]));
    assert_eq!(size((1 << 20) - 51), ("exec-size-mb", vec![("n", 10)]));
    assert_eq!(size((1 << 20) - 52), ("exec-size-kb", vec![("n", 10_239)]));
    assert_eq!(size((1 << 30) - 1), ("exec-size-gb", vec![("n", 10)]));
    // Durations reach messages as a duration argument, never as text joined to a unit.
    assert_eq!(
        LogMsg::Completed { ms: 7 }.args(),
        vec![("duration", LogArg::DurationMs(7))]
    );
}

#[test]
fn an_error_line_never_puts_server_text_in_its_arguments() {
    let secret = "s3cret appears in the server text";
    let msg = LogMsg::Error {
        n: 2,
        view: error_view(DbmsKind::Postgres, &server_error(secret), "x", 0, None),
    };
    assert_eq!(msg.fluent_id(), "exec-error");
    assert_eq!(msg.args(), vec![("n", LogArg::Str("2".into()))]);
    let LogMsg::Error { view, .. } = &msg else {
        unreachable!()
    };
    assert_eq!(view.server_text.as_deref(), Some(secret));
    assert_eq!(view.code.as_deref(), Some("42P01"));
}

#[test]
fn sqlite_busy_gets_hacivats_hint() {
    let busy = |code: i64| ExecError {
        kind: ExecErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate: None,
            vendor_code: Some(code),
            severity: None,
            text: "database is locked".into(),
            detail: None,
            hint: None,
            lang_hint: None,
        }),
    };
    let view = |engine, code| error_view(engine, &busy(code), "SELECT 1", 0, None);
    // SQLITE_BUSY and an extended busy code (SQLITE_BUSY_SNAPSHOT = 517).
    for code in [5, 517] {
        let v = view(DbmsKind::Sqlite, code);
        assert_eq!(v.hint_id, Some("error-exec-sqlite-busy-hint"));
        assert_eq!(v.code.as_deref(), Some(code.to_string().as_str()));
    }
    assert_eq!(view(DbmsKind::Sqlite, 6).hint_id, None, "SQLITE_LOCKED");
    assert_eq!(view(DbmsKind::MySql, 1029).hint_id, None, "not SQLite");
}

fn every_close_reason() -> [CloseReason; 10] {
    [
        CloseReason::SessionNeeded,
        CloseReason::IdleTimeout,
        CloseReason::TabClosed,
        CloseReason::TxEnded,
        CloseReason::UserClosed,
        CloseReason::MemoryLimit,
        CloseReason::ConnectionLost,
        CloseReason::ServerTerminated,
        CloseReason::Cancelled,
        CloseReason::EnforcementGap,
    ]
}

fn every_deny_layer() -> [DenyLayer; 5] {
    [
        DenyLayer::Input,
        DenyLayer::Protocol,
        DenyLayer::Transaction,
        DenyLayer::Engine,
        DenyLayer::Function,
    ]
}

/// One message of every kind and shape the log has.
fn all_log_msgs() -> Vec<LogMsg> {
    let view = ErrorView {
        title_id: "exec-error-server",
        code: None,
        server_text: None,
        hint_id: None,
        doc_offset: None,
    };
    let mut all = vec![
        LogMsg::RowsRetrieved {
            rows: 1,
            more: false,
            exec_ms: 1,
            fetch_ms: 1,
        },
        LogMsg::RowsRetrieved {
            rows: 1,
            more: true,
            exec_ms: 1,
            fetch_ms: 1,
        },
        LogMsg::RowsAffected { rows: 1, ms: 1 },
        LogMsg::Completed { ms: 1 },
        LogMsg::TxCommitted,
        LogMsg::TxRolledBack,
        LogMsg::MoreResultSets { skipped: 1 },
        LogMsg::Summary(RunSummary::default()),
        LogMsg::Error { n: 1, view },
        LogMsg::Cancelled { ms: 1 },
        LogMsg::Dropped { n: 1 },
        LogMsg::CopyUnsupported,
        LogMsg::NotStarted(ErrorView {
            title_id: "exec-error-not-connected",
            code: None,
            server_text: None,
            hint_id: None,
            doc_offset: None,
        }),
    ];
    for reason in every_close_reason() {
        for rolled_back in [false, true] {
            all.push(LogMsg::CursorClosed {
                rows: 1,
                reason,
                rolled_back,
                idle_ms: 1,
            });
        }
    }
    all.extend(every_deny_layer().map(LogMsg::ReadOnlyDenied));
    all
}

/// Every other ID the crate hands the app.
fn other_ids() -> Vec<&'static str> {
    let mut ids: Vec<&'static str> = [0, 340, 2_000, 2_340, 180_000, 185_400]
        .iter()
        .map(|ms| duration_message(*ms).id)
        .collect();
    ids.extend([512, 5 << 20, 5 << 30].iter().map(|b| size_message(*b).id));
    let kinds = [
        ExecErrorKind::Db,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: false,
        },
        ExecErrorKind::ConnectionLost {
            outcome_unknown: true,
        },
        ExecErrorKind::Cancelled,
        ExecErrorKind::Tx(TxError::FailedNeedsRollback),
        ExecErrorKind::Tx(TxError::InProgress),
        ExecErrorKind::Tx(TxError::NeedsClassifier),
        ExecErrorKind::Tx(TxError::Unsupported),
        ExecErrorKind::EnforcementGap(GapReason::WriteSucceeded),
        ExecErrorKind::NotConnected,
        ExecErrorKind::Internal,
    ];
    ids.extend(kinds.iter().map(error_title_id));
    ids.extend(
        every_deny_layer()
            .iter()
            .map(|l| error_title_id(&ExecErrorKind::ReadOnlyDenied(*l))),
    );
    ids.extend(
        [
            AmbiguityReason::SemicolonInExecutableComment,
            AmbiguityReason::UnterminatedConstruct,
            AmbiguityReason::UnknownBlock,
            AmbiguityReason::DelimiterChange,
            AmbiguityReason::MissingSemicolon,
            AmbiguityReason::AmbiguousCommentEnd,
            AmbiguityReason::ClientCommand,
            AmbiguityReason::OverlappingUnits,
        ]
        .map(uncertain_message_id),
    );
    ids.extend([RefuseReason::CopyStream, RefuseReason::RunAsOne].map(RefuseReason::fluent_id));
    ids
}

/// The IDs `locale`'s `hacivat-core.ftl` defines (a message line starts with its ID and ` =`).
fn ftl_ids(locale: &str) -> std::collections::BTreeSet<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../locales")
        .join(locale)
        .join("hacivat-core.ftl");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    text.lines()
        .filter_map(|l| l.split_once(" ="))
        .filter(|(id, _)| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        .map(|(id, _)| id.to_string())
        .collect()
}

fn ftl_has(locale: &str, id: &str) -> bool {
    ftl_ids(locale).contains(id)
}

#[test]
fn every_log_id_exists_in_en_and_tr() {
    for m in all_log_msgs() {
        assert!(
            ftl_has("en", m.fluent_id()) && ftl_has("tr", m.fluent_id()),
            "{}",
            m.fluent_id()
        );
    }
    let (en, tr) = (ftl_ids("en"), ftl_ids("tr"));
    for id in other_ids() {
        assert!(en.contains(id) && tr.contains(id), "{id}");
    }
    // The rolled-back forms are distinct messages (R-M3-23, P26).
    let closed: std::collections::BTreeSet<_> = all_log_msgs()
        .iter()
        .filter(|m| matches!(m, LogMsg::CursorClosed { .. }))
        .map(LogMsg::fluent_id)
        .collect();
    assert_eq!(closed.len(), 20);
    // The idle-timeout reason names how long the cursor sat idle.
    let idle = LogMsg::CursorClosed {
        rows: 9,
        reason: CloseReason::IdleTimeout,
        rolled_back: true,
        idle_ms: 20_000,
    };
    assert_eq!(
        idle.args(),
        vec![
            ("rows", LogArg::U64(9)),
            ("duration", LogArg::DurationMs(20_000))
        ]
    );
}
