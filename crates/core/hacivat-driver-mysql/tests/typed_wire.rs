//! The typed path (Task 14) and `mysql_async` patches §6 and §7 against a scripted in-process
//! server (`support`) that speaks just enough of the MySQL protocol: rows and headers out of
//! shape (always `Protocol`, the connection abandoned, never a panic, never read on),
//! `Unsupported` while a query is open (and nothing sent), suspension and resume in the binary
//! protocol, the drain-then-kill close and its outcomes, the result-set rules, the 1295 route,
//! a dropped call poisoning the connection. No Docker.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hacivat_driver::{
    CloseHow, Closed, ExecOpts, Protocol, QueryEvent, RawConn, RawErrorKind, ResultEnd, TxStatus,
    TypedCaps,
};
use hacivat_driver_mysql::connect_conn;
use hacivat_values::{BindValue, TsVal, ValueRef};
use support::*;
use tokio::sync::Notify;

const NOT_CONNECTED: RawErrorKind = RawErrorKind::Io(std::io::ErrorKind::NotConnected);
/// How long a test waits for the server to see the connection end (a condition, polled).
const SEE_END: Duration = Duration::from_secs(10);

// Tests ------------------------------------------------------------------------------------

#[tokio::test]
async fn caps_follow_the_servers_net_write_timeout() {
    let (c, _server, _) = scripted(false, |_| None).await;
    assert_eq!(
        c.typed_caps(),
        TypedCaps {
            suspend: true,
            binds: false,
            close_needs_kill: true,
            idle_bound: Some(Duration::from_secs(45)),
        }
    );
    assert_eq!(c.backend_id(), Some(u64::from(THREAD_ID)));
}

#[tokio::test]
async fn a_binary_row_short_of_its_columns_breaks_the_connection() {
    let (mut c, _server, _) = scripted(false, |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[long_col("a"), long_col("b")], 0)),
        COM_STMT_EXECUTE => {
            let mut steps = header(&[long_col("a"), long_col("b")]);
            // Both values announced (no NULL bit), one sent.
            steps.push(Step::Packet(
                [vec![0x00, 0x00], 5_i32.to_le_bytes().to_vec()].concat(),
            ));
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
            Some(steps)
        }
        _ => None,
    })
    .await;
    c.query(spec("SELECT a, b FROM t", false, 501))
        .await
        .unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    assert_eq!(kind(c.next_event().await), RawErrorKind::Protocol);
    // Never used again.
    assert_eq!(
        kind(c.ping().await),
        RawErrorKind::Io(std::io::ErrorKind::NotConnected)
    );
}

#[tokio::test]
async fn a_text_row_with_a_value_too_many_is_protocol() {
    // Patch §6: a row must use its whole packet. The statement is not preparable (1295), so a
    // read-write connection runs it once more over COM_QUERY.
    let (mut c, server, setup) = scripted(false, |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(packets(vec![err_packet(
            1295,
            "HY000",
            "This command is not supported in the prepared statement protocol yet",
        )])),
        COM_QUERY if cmd.text() == "EXECUTE s" => {
            let mut steps = header(&[long_col("a")]);
            steps.push(Step::Packet(text_row(&[Some(b"1"), Some(b"2")])));
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
            Some(steps)
        }
        _ => None,
    })
    .await;
    c.query(spec("EXECUTE s", false, 501)).await.unwrap();
    let codes: Vec<u8> = server.commands()[setup..].iter().map(|c| c.code).collect();
    assert_eq!(codes, vec![COM_STMT_PREPARE, COM_QUERY]);
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    assert_eq!(kind(c.next_event().await), RawErrorKind::Protocol);
    assert_eq!(
        kind(c.ping().await),
        RawErrorKind::Io(std::io::ErrorKind::NotConnected)
    );
}

#[tokio::test]
async fn a_binary_row_with_a_value_too_many_is_protocol() {
    // Patch §6 in the binary protocol: one INT column, two values in the row.
    let (mut c, _server, _) = scripted(
        false,
        one_int_column(packets(vec![
            [bin_row(&[long(1)]), 2_i32.to_le_bytes().to_vec()].concat(),
            terminator(AUTOCOMMIT, 0),
        ])),
    )
    .await;
    c.query(spec("SELECT a FROM t", false, 501)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    assert_eq!(kind(c.next_event().await), RawErrorKind::Protocol);
}

#[tokio::test]
async fn an_unknown_column_type_byte_breaks_the_connection() {
    let (mut c, _server, _) = scripted(false, |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[long_col("a")], 0)),
        COM_STMT_EXECUTE => {
            let mut steps = header(&[column_def("a", 100, 0, BINARY, 1, 0)]);
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
            Some(steps)
        }
        _ => None,
    })
    .await;
    assert_eq!(
        kind(c.query(spec("SELECT a FROM t", false, 501)).await),
        RawErrorKind::Protocol
    );
    assert_eq!(
        kind(c.ping().await),
        RawErrorKind::Io(std::io::ErrorKind::NotConnected)
    );
}

#[tokio::test]
async fn a_packet_past_max_allowed_packet_breaks_the_connection() {
    // The server says 1024 bytes; a row of 2,000 bytes is refused by the packet codec.
    let (mut c, _server, _) = scripted(false, |cmd| match cmd.code {
        COM_QUERY if cmd.text().contains("@@max_allowed_packet") => Some(variables(&[
            ("@@max_allowed_packet", "1024"),
            ("@@wait_timeout", "28800"),
        ])),
        COM_STMT_PREPARE => Some(prepare_ok(
            &[column_def("s", T_VAR_STRING, 0, UTF8MB4, 8_000, 0)],
            0,
        )),
        COM_STMT_EXECUTE => {
            let mut steps = header(&[column_def("s", T_VAR_STRING, 0, UTF8MB4, 8_000, 0)]);
            steps.push(Step::Packet(bin_row(&[Some(lenenc_str(&[b'x'; 2_000]))])));
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
            Some(steps)
        }
        _ => None,
    })
    .await;
    c.query(spec("SELECT s FROM t", false, 501)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    assert_eq!(kind(c.next_event().await), RawErrorKind::Protocol);
    assert_eq!(
        kind(c.ping().await),
        RawErrorKind::Io(std::io::ErrorKind::NotConnected)
    );
}

#[tokio::test]
async fn an_empty_packet_among_the_rows_is_protocol_not_a_panic() {
    // Patch §6: `is_last_result_set_packet` indexed `packet[0]`.
    let (mut c, _server, _) = scripted(
        false,
        one_int_column(packets(vec![Vec::new(), terminator(AUTOCOMMIT, 0)])),
    )
    .await;
    c.query(spec("SELECT a FROM t", false, 501)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    assert_eq!(kind(c.next_event().await), RawErrorKind::Protocol);
}

#[tokio::test]
async fn a_binary_result_with_an_internal_column_type_is_refused_before_its_rows() {
    // mysql_common's binary row parser panics (`unimplemented!`) on TIME2; the header is
    // refused before any row is read.
    let col = || column_def("t", T_TIME2, 0, BINARY, 10, 0);
    let (mut c, server, setup) = scripted(false, move |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[col()], 0)),
        COM_STMT_EXECUTE => {
            let mut steps = header(&[col()]);
            steps.push(Step::Packet(bin_row(&[Some(vec![0; 3])])));
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
            Some(steps)
        }
        _ => None,
    })
    .await;
    assert_eq!(
        kind(c.query(spec("SELECT t FROM t", false, 501)).await),
        RawErrorKind::Protocol
    );
    assert_eq!(server.commands().len(), setup + 2);
    assert_eq!(
        kind(c.ping().await),
        RawErrorKind::Io(std::io::ErrorKind::NotConnected)
    );
}

#[tokio::test]
async fn a_column_count_of_zero_is_protocol_not_a_panic() {
    // Patch §6: `read_column_defs` asserted `num > 0` (a debug assertion, so a panic in tests).
    let (mut c, _server, _) = scripted(false, |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[long_col("a")], 0)),
        // A zero column count in the two-byte length-encoded form (the one-byte 0 is an OK).
        COM_STMT_EXECUTE => Some(packets(vec![vec![0xFC, 0, 0]])),
        _ => None,
    })
    .await;
    assert_eq!(
        kind(c.query(spec("SELECT a FROM t", false, 501)).await),
        RawErrorKind::Protocol
    );
}

#[tokio::test]
async fn a_column_count_past_65535_is_protocol_not_an_allocation() {
    // Patch §6: `read_packets` reserved room for every announced column definition up front;
    // 2^40 of them aborts the process.
    let (mut c, _server, _) = scripted(false, |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[long_col("a")], 0)),
        COM_STMT_EXECUTE => Some(packets(vec![lenenc_int(1 << 40)])),
        _ => None,
    })
    .await;
    assert_eq!(
        kind(c.query(spec("SELECT a FROM t", false, 501)).await),
        RawErrorKind::Protocol
    );
}

#[tokio::test]
async fn a_hostile_cell_is_an_error_cell_and_the_row_is_kept() {
    let date = || column_def("d", T_DATE, 0, BINARY, 10, 0);
    let (mut c, _server, _) = scripted(false, move |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(packets(vec![err_packet(1295, "HY000", "not preparable")])),
        COM_QUERY if cmd.text().starts_with("EXECUTE") => {
            let mut steps = header(&[date(), long_col("n")]);
            steps.push(Step::Packet(text_row(&[Some(b"2024-13-45"), Some(b"1")])));
            steps.push(Step::Packet(text_row(&[Some(b"2024-01-02"), Some(b"2")])));
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
            Some(steps)
        }
        _ => None,
    })
    .await;
    c.query(spec("EXECUTE s", false, 501)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    let chunk = rows_of(event(&mut c).await);
    assert_eq!(chunk.rows(), 2);
    assert!(matches!(chunk.cell(0, 0), ValueRef::Error(e) if &*e.code == "mysql-date"));
    assert_eq!(int_cells(&chunk, 1), vec![1, 2]);
    assert!(matches!(chunk.cell(1, 0), ValueRef::Date(_)));
    end_of(event(&mut c).await);
    done_of(event(&mut c).await);
}

#[tokio::test]
async fn statements_while_a_query_is_open_answer_unsupported_and_send_nothing() {
    let (mut c, server, _) = scripted(
        false,
        one_int_column(
            [
                int_rows(&[1, 2, 3]),
                packets(vec![terminator(AUTOCOMMIT, 0)]),
            ]
            .concat(),
        ),
    )
    .await;
    c.query(spec("SELECT a FROM t", true, 1)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    assert_eq!(int_cells(&rows_of(event(&mut c).await), 0), vec![1]);
    assert_eq!(event(&mut c).await, QueryEvent::Suspended);
    let before = server.count();
    let unsupported = RawErrorKind::Unsupported;
    assert_eq!(
        kind(c.execute("SELECT 1", ExecOpts::default()).await),
        unsupported
    );
    assert_eq!(
        kind(c.execute("SET @a = 1", ExecOpts::values(10)).await),
        unsupported
    );
    assert_eq!(kind(c.batch("ROLLBACK").await), unsupported);
    assert_eq!(kind(c.ping().await), unsupported);
    assert_eq!(kind(c.server_info().await), unsupported);
    assert_eq!(kind(c.current_user().await), unsupported);
    assert_eq!(kind(c.list_databases(10).await), unsupported);
    assert_eq!(kind(c.reset().await), unsupported);
    assert_eq!(kind(c.query(spec("SELECT 2", false, 1)).await), unsupported);
    assert_eq!(kind(c.next_event().await), unsupported);
    // Answered from what the connection knows, without a round trip.
    assert!(!c.tls_summary().await.encrypted);
    assert!(c.probe().await.is_empty());
    assert_eq!(c.backend_id(), Some(u64::from(THREAD_ID)));
    assert_eq!(server.count(), before, "nothing reached the server");
    c.fetch_more(10).await.unwrap();
    assert_eq!(int_cells(&rows_of(event(&mut c).await), 0), vec![2, 3]);
    let end = end_of(event(&mut c).await);
    assert_eq!((end.rows, end.kept), (3, 3));
    assert_eq!(done_of(event(&mut c).await).tx, TxStatus::Idle);
    c.ping().await.unwrap();
}

#[tokio::test]
async fn a_resumed_query_keeps_the_binary_protocol() {
    // DATETIME(6) and DOUBLE values in the binary encoding: read as text after a resume, the
    // row would not even parse.
    let defs = || {
        vec![
            column_def("at", T_DATETIME, 0, BINARY, 26, 6),
            column_def("x", T_DOUBLE, 0, BINARY, 22, 31),
        ]
    };
    let row = |sec: u8, x: f64| {
        let mut dt = vec![11_u8];
        dt.extend_from_slice(&2024_u16.to_le_bytes());
        dt.extend_from_slice(&[2, 29, 13, 14, sec]);
        dt.extend_from_slice(&123_456_u32.to_le_bytes());
        bin_row(&[Some(dt), Some(x.to_le_bytes().to_vec())])
    };
    let (mut c, _server, _) = scripted(false, move |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&defs(), 0)),
        COM_STMT_EXECUTE => {
            let mut steps = header(&defs());
            steps.extend(packets(vec![
                row(1, 0.5),
                row(2, -1.25),
                row(3, f64::MAX),
                terminator(AUTOCOMMIT, 0),
            ]));
            Some(steps)
        }
        _ => None,
    })
    .await;
    c.query(spec("SELECT at, x FROM t", true, 1)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    assert_eq!(rows_of(event(&mut c).await).rows(), 1);
    assert_eq!(event(&mut c).await, QueryEvent::Suspended);
    c.fetch_more(5).await.unwrap();
    let chunk = rows_of(event(&mut c).await);
    assert_eq!(chunk.rows(), 2);
    let base = 1_709_164_800 + 13 * 3_600 + 14 * 60;
    for (r, (sec, x)) in [(2_i64, -1.25), (3, f64::MAX)].into_iter().enumerate() {
        assert_eq!(
            chunk.cell(r as u32, 0),
            ValueRef::Timestamp(TsVal {
                secs: base + sec,
                nanos: 123_456_000
            })
        );
        assert_eq!(chunk.cell(r as u32, 1), ValueRef::F64(x));
    }
    assert_eq!(end_of(event(&mut c).await).rows, 3);
}

#[tokio::test]
async fn a_result_ending_at_the_limit_ends_there_instead_of_suspending() {
    let (mut c, _server, _) = scripted(
        false,
        one_int_column([int_rows(&[1, 2]), packets(vec![terminator(AUTOCOMMIT, 0)])].concat()),
    )
    .await;
    c.query(spec("SELECT a FROM t", true, 2)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    assert_eq!(int_cells(&rows_of(event(&mut c).await), 0), vec![1, 2]);
    assert_eq!(end_of(event(&mut c).await).rows, 2);
    done_of(event(&mut c).await);
}

#[tokio::test]
async fn drain_runs_out_then_after_kill_resumes_the_same_read_to_the_kill() {
    let release = Arc::new(Notify::new());
    let rows = [
        int_rows(&[1, 2]),
        vec![Step::Wait(release.clone())],
        int_rows(&[3]),
        packets(vec![err_packet(
            1317,
            "70100",
            "Query execution was interrupted",
        )]),
    ]
    .concat();
    let (mut c, server, setup) = scripted(false, one_int_column(rows)).await;
    c.query(spec("SELECT a FROM t", true, 1)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    assert_eq!(rows_of(event(&mut c).await).rows(), 1);
    assert_eq!(event(&mut c).await, QueryEvent::Suspended);
    // The rest does not come within the drain's time: the session must kill this thread.
    assert_eq!(
        c.close_query(CloseHow::Drain(Duration::from_millis(50)))
            .await
            .unwrap(),
        Closed::NeedsServerKill {
            thread_id: u64::from(THREAD_ID)
        }
    );
    // The drain holds the connection: still nothing else runs, and the thread ID is known.
    assert_eq!(kind(c.ping().await), RawErrorKind::Unsupported);
    assert_eq!(c.backend_id(), Some(u64::from(THREAD_ID)));
    // The "kill": the server sends the rest and ER_QUERY_INTERRUPTED.
    release.notify_one();
    assert_eq!(
        c.close_query(CloseHow::AfterKill(Duration::from_secs(10)))
            .await
            .unwrap(),
        Closed::Closed { rows: 1 }
    );
    // The connection is in step: the status was read again with a ping, and it is usable.
    let codes: Vec<u8> = server.commands()[setup..].iter().map(|c| c.code).collect();
    assert_eq!(codes, vec![COM_STMT_PREPARE, COM_STMT_EXECUTE, COM_PING]);
    c.ping().await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::Idle);
}

#[tokio::test]
async fn now_asks_for_the_kill_without_reading() {
    let (mut c, server, _) = scripted(
        false,
        one_int_column(
            [
                int_rows(&[1, 2, 3]),
                packets(vec![terminator(AUTOCOMMIT, 0)]),
            ]
            .concat(),
        ),
    )
    .await;
    c.query(spec("SELECT a FROM t", true, 1)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    rows_of(event(&mut c).await);
    assert_eq!(event(&mut c).await, QueryEvent::Suspended);
    assert_eq!(
        c.close_query(CloseHow::Now).await.unwrap(),
        Closed::NeedsServerKill {
            thread_id: u64::from(THREAD_ID)
        }
    );
    // The kill came too late (the result had ended): AfterKill reads to the end, and the
    // statement finished, so the result is complete (Task 12's rule).
    assert_eq!(
        c.close_query(CloseHow::AfterKill(Duration::from_secs(10)))
            .await
            .unwrap(),
        Closed::Completed { rows: 3 }
    );
    let before = server.count();
    c.ping().await.unwrap();
    assert_eq!(server.count(), before + 1);
}

#[tokio::test]
async fn drain_completes_a_short_rest() {
    let (mut c, _server, _) = scripted(
        false,
        one_int_column(
            [
                int_rows(&[1, 2, 3]),
                packets(vec![terminator(AUTOCOMMIT, 0)]),
            ]
            .concat(),
        ),
    )
    .await;
    c.query(spec("SELECT a FROM t", true, 1)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    rows_of(event(&mut c).await);
    assert_eq!(event(&mut c).await, QueryEvent::Suspended);
    assert_eq!(
        c.close_query(CloseHow::Drain(Duration::from_secs(10)))
            .await
            .unwrap(),
        Closed::Completed { rows: 3 }
    );
    c.ping().await.unwrap();
    // Nothing open any more.
    assert_eq!(
        c.close_query(CloseHow::Drain(Duration::from_secs(1)))
            .await
            .unwrap(),
        Closed::Closed { rows: 0 }
    );
}

/// A `CALL` whose procedure returns these result sets (one `INT` row each), then its OK.
fn call_with(
    sets: usize,
    error_after: Option<usize>,
) -> impl Fn(&Cmd) -> Option<Vec<Step>> + Send + Sync {
    move |cmd: &Cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[], 0)),
        COM_STMT_EXECUTE => {
            let mut steps = Vec::new();
            for i in 0..sets {
                if error_after == Some(i) {
                    steps.push(Step::Packet(err_packet(
                        1146,
                        "42S02",
                        "Table 'hv.x' doesn't exist",
                    )));
                    return Some(steps);
                }
                steps.extend(header(&[long_col("n")]));
                steps.push(Step::Packet(bin_row(&[long(i as i32 + 1)])));
                steps.push(Step::Packet(terminator(AUTOCOMMIT | MORE_RESULTS, 0)));
            }
            steps.push(Step::Packet(ok_packet(4, AUTOCOMMIT, 1)));
            Some(steps)
        }
        _ => None,
    }
}

#[tokio::test]
async fn call_with_three_sets_keeps_the_first_and_counts_two() {
    let (mut c, _server, _) = scripted(false, call_with(3, None)).await;
    // Suspendable, but a CALL never suspends.
    c.query(spec("CALL hv.p()", true, 1)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    assert_eq!(int_cells(&rows_of(event(&mut c).await), 0), vec![1]);
    let end = end_of(event(&mut c).await);
    assert_eq!((end.rows, end.kept), (1, 1));
    // The unit's final OK packet: the procedure's write count.
    assert_eq!(end.rows_affected, Some(4));
    assert_eq!(end.command_tag.as_deref(), Some("CALL"));
    let done = done_of(event(&mut c).await);
    assert_eq!(done.skipped_result_sets, 2);
    assert_eq!(done.warnings, 1);
    c.ping().await.unwrap();
}

#[tokio::test]
async fn two_kept_sets_when_the_spec_allows_them() {
    let (mut c, _server, _) = scripted(false, call_with(3, None)).await;
    let mut s = spec("CALL hv.p()", false, 501);
    s.max_result_sets = 2;
    c.query(s).await.unwrap();
    for n in [1, 2] {
        assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
        assert_eq!(int_cells(&rows_of(event(&mut c).await), 0), vec![n]);
        end_of(event(&mut c).await);
    }
    assert_eq!(done_of(event(&mut c).await).skipped_result_sets, 1);
}

#[tokio::test]
async fn a_later_statements_error_comes_after_the_kept_results_end() {
    let (mut c, _server, _) = scripted(false, call_with(3, Some(1))).await;
    c.query(spec("CALL hv.p()", false, 501)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    rows_of(event(&mut c).await);
    assert_eq!(end_of(event(&mut c).await).rows, 1);
    let e = c.next_event().await.unwrap_err();
    assert_eq!(vendor(&e), Some(1146));
    assert_eq!(e.kind, RawErrorKind::Db);
    // In step: the status was read again and the connection works.
    c.ping().await.unwrap();
}

#[tokio::test]
async fn a_server_error_mid_stream_ends_the_query_and_keeps_the_connection() {
    let (mut c, _server, _) = scripted(
        false,
        one_int_column(
            [
                int_rows(&[1]),
                packets(vec![err_packet(
                    1317,
                    "70100",
                    "Query execution was interrupted",
                )]),
            ]
            .concat(),
        ),
    )
    .await;
    c.query(spec("SELECT a FROM t", false, 501)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    let e = c.next_event().await.unwrap_err();
    assert_eq!(vendor(&e), Some(1317));
    assert_eq!(e.kind, RawErrorKind::Cancelled);
    assert_eq!(kind(c.next_event().await), RawErrorKind::Unsupported);
    c.ping().await.unwrap();
}

#[tokio::test]
async fn a_dml_statement_reports_one_empty_result() {
    let (mut c, _server, _) = scripted(false, |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[], 0)),
        COM_STMT_EXECUTE => Some(packets(vec![ok_packet(3, AUTOCOMMIT | IN_TRANS, 0)])),
        _ => None,
    })
    .await;
    c.query(spec("UPDATE t SET a = 1", true, 501))
        .await
        .unwrap();
    match event(&mut c).await {
        QueryEvent::ResultStart(schema) => assert!(schema.columns.is_empty()),
        other => panic!("{other:?}"),
    }
    let end = end_of(event(&mut c).await);
    assert_eq!(
        end,
        ResultEnd {
            rows: 0,
            kept: 0,
            rows_affected: Some(3),
            command_tag: Some("UPDATE".into()),
        }
    );
    assert_eq!(done_of(event(&mut c).await).tx, TxStatus::InTx);
    assert_eq!(c.tx_status(), TxStatus::InTx);
}

#[tokio::test]
async fn a_set_runs_over_com_query_as_m2b_routes_it() {
    let (mut c, server, setup) = scripted(false, |_| None).await;
    c.query(spec("SET @a = 1", false, 501)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    end_of(event(&mut c).await);
    done_of(event(&mut c).await);
    let cmds = server.commands();
    assert_eq!(cmds[setup].code, COM_QUERY);
    assert_eq!(cmds[setup].text(), "SET @a = 1");
}

#[tokio::test]
async fn read_only_1295_stays_unsupported_and_is_never_resent_as_text() {
    let (mut c, server, setup) = scripted(true, |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(packets(vec![err_packet(1295, "HY000", "not preparable")])),
        _ => None,
    })
    .await;
    assert_eq!(
        kind(c.query(spec("EXECUTE s", false, 501)).await),
        RawErrorKind::Unsupported
    );
    let codes: Vec<u8> = server.commands()[setup..].iter().map(|c| c.code).collect();
    assert_eq!(codes, vec![COM_STMT_PREPARE]);
    c.ping().await.unwrap();
}

#[tokio::test]
async fn binds_placeholders_and_the_simple_protocol_are_refused() {
    let (mut c, server, setup) = scripted(false, |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[long_col("a")], 1)),
        _ => None,
    })
    .await;
    let mut simple = spec("SELECT 1", false, 1);
    simple.protocol = Protocol::Simple;
    assert_eq!(kind(c.query(simple).await), RawErrorKind::Unsupported);
    let binds = [BindValue::Null];
    let mut bound = spec("SELECT ?", false, 1);
    bound.binds = &binds;
    assert_eq!(kind(c.query(bound).await), RawErrorKind::Unsupported);
    assert_eq!(server.count(), setup, "refused before anything was sent");
    // A placeholder: prepared, never executed.
    assert_eq!(
        kind(c.query(spec("SELECT a FROM t WHERE a = ?", false, 1)).await),
        RawErrorKind::Unsupported
    );
    let codes: Vec<u8> = server.commands()[setup..].iter().map(|c| c.code).collect();
    assert_eq!(codes, vec![COM_STMT_PREPARE]);
    c.ping().await.unwrap();
}

#[tokio::test]
async fn text_columns_decode_lossless_over_com_query() {
    let (mut c, _server, _) = scripted(false, |cmd| match cmd.code {
        COM_QUERY if cmd.text().starts_with("SET") => {
            let mut steps = header(&[column_def("s", T_VAR_STRING, 0, UTF8MB4, 40, 0)]);
            steps.push(Step::Packet(text_row(&[Some(&[0x66, 0xFC])])));
            steps.push(Step::Packet(text_row(&[None])));
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
            Some(steps)
        }
        _ => None,
    })
    .await;
    // A `SET` with rows only a scripted server sends: the text route decodes them.
    c.query(spec("SET @s = 1", false, 501)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    let chunk = rows_of(event(&mut c).await);
    assert_eq!(chunk.cell(0, 0), ValueRef::Lossy(&[0x66, 0xFC]));
    assert_eq!(chunk.cell(1, 0), ValueRef::Null);
}

// Review round 1 ---------------------------------------------------------------------------

/// The server, stalled on `release`, then sends a rest far larger than every buffer. A client
/// that abandoned the connection never reads it: the server's writes fail and it sees the end,
/// with no `COM_QUIT`.
async fn assert_abandoned(server: &Server, release: &Notify) {
    release.notify_one();
    assert!(
        wait_until(SEE_END, || server.ended()).await,
        "the server never saw the connection end"
    );
    assert!(server.write_failed(), "the client read the rest");
    assert!(
        !server.quit_seen(),
        "an abandoned connection sends no COM_QUIT"
    );
}

#[tokio::test]
async fn a_protocol_violation_abandons_the_connection_without_reading_on() {
    let release = Arc::new(Notify::new());
    let rows = [
        // A text row with a value too many (patch §6), then a valid multi-megabyte rest.
        packets(vec![text_row(&[Some(b"x"), Some(b"y")])]),
        vec![Step::Wait(release.clone())],
        wide_text_rows(WIDE_ROWS),
        packets(vec![terminator(AUTOCOMMIT, 0)]),
    ]
    .concat();
    let rows = Arc::new(std::sync::Mutex::new(Some(rows)));
    let (mut c, server, _) = scripted(false, move |cmd| match cmd.code {
        COM_QUERY if cmd.text().starts_with("SET") => {
            let mut steps = header(&[text_col("s")]);
            steps.extend(rows.lock().unwrap().take().unwrap_or_default());
            Some(steps)
        }
        _ => None,
    })
    .await;
    c.query(spec("SET @s = 1", false, 501)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    assert_eq!(kind(c.next_event().await), RawErrorKind::Protocol);
    assert_abandoned(&server, &release).await;
    assert_eq!(kind(c.ping().await), NOT_CONNECTED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_column_type_is_never_parsed_in_the_background() {
    // Before patch §7 the dropped `Conn`'s cleanup read this row with `mysql_common`'s binary
    // parser and panicked (`unimplemented!`) in a tokio worker.
    let panics = record_panics();
    let release = Arc::new(Notify::new());
    let col = || column_def("t", T_TIME2, 0, BINARY, 10, 0);
    let held = release.clone();
    let (mut c, server, _) = scripted(false, move |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[col()], 0)),
        COM_STMT_EXECUTE => {
            let mut steps = header(&[col()]);
            steps.push(Step::Wait(held.clone()));
            steps.extend((0..WIDE_ROWS).map(|_| Step::Packet(bin_row(&[Some(vec![0; 3])]))));
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
            Some(steps)
        }
        _ => None,
    })
    .await;
    assert_eq!(
        kind(c.query(spec("SELECT t FROM t", false, 501)).await),
        RawErrorKind::Protocol
    );
    release.notify_one();
    assert!(wait_until(SEE_END, || server.ended()).await);
    // The socket is closed, so nothing can read the rows any more.
    let seen = panics.lock().unwrap().clone();
    assert!(
        !seen.iter().any(|m| m.contains("Unsupported column type")),
        "{seen:?}"
    );
    assert!(!server.quit_seen());
}

#[tokio::test]
async fn close_with_a_suspended_query_abandons_the_connection() {
    let release = Arc::new(Notify::new());
    let rows = [
        wide_bin_rows(3),
        vec![Step::Wait(release.clone())],
        wide_bin_rows(WIDE_ROWS),
        packets(vec![terminator(AUTOCOMMIT, 0)]),
    ]
    .concat();
    let (mut c, server, _) = scripted(false, one_column(text_col("s"), rows)).await;
    c.query(spec("SELECT s FROM t", true, 1)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    rows_of(event(&mut c).await);
    assert_eq!(event(&mut c).await, QueryEvent::Suspended);
    Box::new(c).close().await;
    assert_abandoned(&server, &release).await;
}

#[tokio::test]
async fn a_plain_close_still_quits_cleanly() {
    let (c, server, _) = scripted(false, |_| None).await;
    Box::new(c).close().await;
    assert!(wait_until(SEE_END, || server.ended()).await);
    assert!(server.quit_seen());
}

#[tokio::test]
async fn an_after_kill_that_runs_out_abandons_the_drains_connection() {
    let release = Arc::new(Notify::new());
    let rows = [
        wide_bin_rows(2),
        vec![Step::Wait(release.clone())],
        wide_bin_rows(WIDE_ROWS),
        packets(vec![terminator(AUTOCOMMIT, 0)]),
    ]
    .concat();
    let (mut c, server, _) = scripted(false, one_column(text_col("s"), rows)).await;
    c.query(spec("SELECT s FROM t", true, 1)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    rows_of(event(&mut c).await);
    assert_eq!(event(&mut c).await, QueryEvent::Suspended);
    assert!(matches!(
        c.close_query(CloseHow::Drain(Duration::from_millis(20)))
            .await
            .unwrap(),
        Closed::NeedsServerKill { .. }
    ));
    // The kill never lands: the AfterKill runs out and the drain's `Conn` is abandoned.
    assert_eq!(
        kind(
            c.close_query(CloseHow::AfterKill(Duration::from_millis(20)))
                .await
        ),
        RawErrorKind::Io(std::io::ErrorKind::TimedOut)
    );
    assert_abandoned(&server, &release).await;
    assert_eq!(kind(c.ping().await), NOT_CONNECTED);
}

#[tokio::test]
async fn a_server_error_while_draining_is_reported_not_swallowed() {
    let rows = [
        int_rows(&[1, 2, 3]),
        packets(vec![err_packet(
            1205,
            "HY000",
            "Lock wait timeout exceeded",
        )]),
    ]
    .concat();
    let (mut c, _server, _) = scripted(false, one_int_column(rows)).await;
    c.query(spec("SELECT a FROM t", true, 1)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    rows_of(event(&mut c).await);
    assert_eq!(event(&mut c).await, QueryEvent::Suspended);
    let e = c
        .close_query(CloseHow::Drain(Duration::from_secs(10)))
        .await
        .unwrap_err();
    assert_eq!(vendor(&e), Some(1205));
    assert_eq!(e.kind, RawErrorKind::Db);
    // In step: closed, the status read again, the connection usable.
    c.ping().await.unwrap();
}

#[tokio::test]
async fn a_later_statements_error_is_reported_by_close_after_the_kept_end() {
    let (mut c, _server, _) = scripted(false, |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[], 0)),
        COM_STMT_EXECUTE => {
            let mut steps = header(&[long_col("n")]);
            steps.push(Step::Packet(bin_row(&[long(1)])));
            steps.push(Step::Packet(terminator(AUTOCOMMIT | MORE_RESULTS, 0)));
            steps.push(Step::Packet(err_packet(
                1062,
                "23000",
                "Duplicate entry '1' for key 'PRIMARY'",
            )));
            Some(steps)
        }
        _ => None,
    })
    .await;
    c.query(spec("CALL hv.p()", false, 501)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    rows_of(event(&mut c).await);
    end_of(event(&mut c).await);
    let e = c
        .close_query(CloseHow::Drain(Duration::from_secs(10)))
        .await
        .unwrap_err();
    assert_eq!(vendor(&e), Some(1062));
    assert_eq!(e.kind, RawErrorKind::Db);
    c.ping().await.unwrap();
}

#[tokio::test]
async fn an_after_kill_that_reads_to_the_end_is_completed() {
    // The kill came too late: the statement finished, so the result is complete.
    let release = Arc::new(Notify::new());
    let rows = [
        int_rows(&[1, 2]),
        vec![Step::Wait(release.clone())],
        int_rows(&[3]),
        packets(vec![terminator(AUTOCOMMIT, 0)]),
    ]
    .concat();
    let (mut c, _server, _) = scripted(false, one_int_column(rows)).await;
    c.query(spec("SELECT a FROM t", true, 1)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    rows_of(event(&mut c).await);
    assert_eq!(event(&mut c).await, QueryEvent::Suspended);
    assert!(matches!(
        c.close_query(CloseHow::Drain(Duration::from_millis(50)))
            .await
            .unwrap(),
        Closed::NeedsServerKill { .. }
    ));
    release.notify_one();
    assert_eq!(
        c.close_query(CloseHow::AfterKill(Duration::from_secs(10)))
            .await
            .unwrap(),
        Closed::Completed { rows: 3 }
    );
    c.ping().await.unwrap();
}

#[tokio::test]
async fn now_finishes_here_when_nothing_is_left_unread() {
    // A DML statement (its OK packet read, nothing on the wire): no kill.
    let (mut c, server, setup) = scripted(false, |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[], 0)),
        COM_STMT_EXECUTE => Some(packets(vec![ok_packet(2, AUTOCOMMIT, 0)])),
        _ => None,
    })
    .await;
    c.query(spec("DELETE FROM t", false, 501)).await.unwrap();
    assert_eq!(
        c.close_query(CloseHow::Now).await.unwrap(),
        Closed::Completed { rows: 0 }
    );
    assert_eq!(server.count(), setup + 2, "nothing more was sent");
    c.ping().await.unwrap();

    // A result read to its end (the `ResultEnd` out, `Done` not yet asked for).
    let (mut c, _server, _) = scripted(
        false,
        one_int_column([int_rows(&[1, 2]), packets(vec![terminator(AUTOCOMMIT, 0)])].concat()),
    )
    .await;
    c.query(spec("SELECT a FROM t", false, 501)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    rows_of(event(&mut c).await);
    end_of(event(&mut c).await);
    assert_eq!(
        c.close_query(CloseHow::Now).await.unwrap(),
        Closed::Completed { rows: 2 }
    );
    c.ping().await.unwrap();
}

#[tokio::test]
async fn a_select_reports_no_affected_rows() {
    let (mut c, _server, _) = scripted(
        false,
        one_int_column([int_rows(&[1, 2]), packets(vec![terminator(AUTOCOMMIT, 0)])].concat()),
    )
    .await;
    c.query(spec("SELECT a FROM t", false, 501)).await.unwrap();
    assert!(matches!(event(&mut c).await, QueryEvent::ResultStart(_)));
    rows_of(event(&mut c).await);
    assert_eq!(end_of(event(&mut c).await).rows_affected, None);
}

#[tokio::test]
async fn a_dropped_query_future_poisons_the_connection() {
    // Dropped while the prepare reply is in flight: the reply stays in the socket, and the next
    // prepare would read it as its own (a wrong schema with no error).
    let release = Arc::new(Notify::new());
    let first = Arc::new(AtomicBool::new(true));
    let held = release.clone();
    let (mut c, server, setup) = scripted(false, move |cmd| match cmd.code {
        COM_STMT_PREPARE => {
            let mut steps = Vec::new();
            if first.swap(false, Ordering::SeqCst) {
                steps.push(Step::Wait(held.clone()));
            }
            steps.extend(prepare_ok(&[long_col("a")], 0));
            Some(steps)
        }
        COM_STMT_EXECUTE => {
            let mut steps = header(&[long_col("a")]);
            steps.push(Step::Packet(bin_row(&[long(1)])));
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
            Some(steps)
        }
        _ => None,
    })
    .await;
    // The server holds its reply until released, so this always drops the future mid-prepare.
    let dropped = tokio::time::timeout(
        Duration::from_millis(100),
        c.query(spec("SELECT a FROM t", false, 501)),
    )
    .await;
    assert!(dropped.is_err());
    release.notify_one();
    assert_eq!(
        kind(c.query(spec("SELECT a FROM t", false, 501)).await),
        NOT_CONNECTED
    );
    assert_eq!(
        kind(c.execute("SELECT 1", ExecOpts::default()).await),
        NOT_CONNECTED
    );
    assert_eq!(kind(c.ping().await), NOT_CONNECTED);
    assert!(wait_until(SEE_END, || server.ended()).await);
    // Only the first prepare reached the server.
    let codes: Vec<u8> = server.commands()[setup..].iter().map(|c| c.code).collect();
    assert_eq!(codes, vec![COM_STMT_PREPARE]);
}

#[tokio::test]
async fn a_1295_from_the_execute_is_not_retried_as_text() {
    // Only a refused prepare (nothing ran) is retried; a statement that was sent never is.
    let (mut c, server, setup) = scripted(false, |cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[long_col("a")], 0)),
        COM_STMT_EXECUTE => Some(packets(vec![err_packet(1295, "HY000", "not supported")])),
        _ => None,
    })
    .await;
    let e = c
        .query(spec("SELECT a FROM t", false, 501))
        .await
        .unwrap_err();
    assert_eq!(vendor(&e), Some(1295));
    let codes: Vec<u8> = server.commands()[setup..].iter().map(|c| c.code).collect();
    assert_eq!(codes, vec![COM_STMT_PREPARE, COM_STMT_EXECUTE, COM_PING]);
}

#[tokio::test]
async fn a_connection_that_cannot_answer_net_write_timeout_fails_the_connect() {
    let server = serve(Arc::new(|cmd: &Cmd| {
        (cmd.code == COM_QUERY && cmd.text().contains("@@net_write_timeout"))
            .then(|| vec![Step::Hangup])
    }))
    .await;
    let e = match connect_conn(request(server.addr, false)).await {
        Ok(_) => panic!("connected without an answer to SELECT @@net_write_timeout"),
        Err(e) => e,
    };
    assert!(matches!(e.kind, RawErrorKind::Io(_)), "{:?}", e.kind);
}

#[tokio::test]
async fn a_server_without_net_write_timeout_connects_without_suspension() {
    let (c, _server, _) = scripted(false, |cmd| {
        (cmd.code == COM_QUERY && cmd.text().contains("@@net_write_timeout")).then(|| {
            packets(vec![err_packet(
                1193,
                "HY000",
                "Unknown system variable 'net_write_timeout'",
            )])
        })
    })
    .await;
    assert_eq!(
        c.typed_caps(),
        TypedCaps {
            suspend: false,
            binds: false,
            close_needs_kill: true,
            idle_bound: None,
        }
    );
}
