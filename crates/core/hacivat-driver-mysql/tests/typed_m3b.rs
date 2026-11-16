//! M3b Task 13 (HV-EXEC-03 driver half; R-M3-60, R-M3-66, R-M3-77, R-M3-79) on MySQL against
//! the scripted in-process server (`support`): the exact `COM_STMT_EXECUTE` parameters a bind
//! becomes (never text spliced into the statement), the placeholder count, Run as one
//! (`COM_QUERY` multi-statements on read-write connections only) and `SHOW WARNINGS` after
//! `Done`. No Docker (`docker_typed_m3b.rs` runs the same rules on real servers).

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

mod support;

use std::sync::Arc;

use hacivat_driver::{NoticeSeverity, Protocol, QueryEvent, RawConn, RawErrorKind};
use hacivat_values::{BindValue, LogicalType};
use support::*;

// ## m3b-t13-drivers

fn typed(hint: LogicalType, text: &str) -> BindValue {
    BindValue::Typed {
        hint,
        text: Arc::from(text),
    }
}

fn int(signed: bool, text: &str) -> BindValue {
    typed(LogicalType::Int { bits: 64, signed }, text)
}

fn text(s: &str) -> BindValue {
    typed(
        LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        s,
    )
}

/// One parameter of a `COM_STMT_EXECUTE` as the server reads it: its type byte, flag byte and
/// value bytes (`None`: NULL by the bitmap).
#[derive(Debug, PartialEq, Eq)]
struct Param {
    ty: u8,
    flags: u8,
    value: Option<Vec<u8>>,
}

/// Reads the parameters of a `COM_STMT_EXECUTE` body (after its command byte) for a statement
/// with `n` parameters; only the types this driver sends (LONGLONG, VAR_STRING, NULL).
fn exec_params(body: &[u8], n: usize) -> Vec<Param> {
    // stmt_id (4), flags (1), iteration count (4).
    let mut at = 9;
    let bitmap = &body[at..at + n.div_ceil(8)];
    at += n.div_ceil(8);
    assert_eq!(body[at], 1, "new-params-bound flag");
    at += 1;
    let types: Vec<(u8, u8)> = (0..n)
        .map(|i| (body[at + 2 * i], body[at + 2 * i + 1]))
        .collect();
    at += 2 * n;
    let mut out = Vec::new();
    for (i, (ty, flags)) in types.into_iter().enumerate() {
        let null = bitmap[i / 8] & (1 << (i % 8)) != 0;
        let value = if null {
            None
        } else if ty == T_LONGLONG {
            let v = body[at..at + 8].to_vec();
            at += 8;
            Some(v)
        } else {
            let len = usize::from(body[at]);
            assert!(len < 251, "short strings only in this test");
            let v = body[at + 1..at + 1 + len].to_vec();
            at += 1 + len;
            Some(v)
        };
        out.push(Param { ty, flags, value });
    }
    assert_eq!(at, body.len(), "every byte read");
    out
}

const T_NULL: u8 = 6;
const UNSIGNED: u8 = 0x80;

/// A prepared one-column statement with `params` placeholders, answering one row.
fn prepared(params: u16) -> impl Fn(&Cmd) -> Option<Vec<Step>> + Send + Sync + 'static {
    move |cmd: &Cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[long_col("a")], params)),
        COM_STMT_EXECUTE => {
            let mut steps = header(&[long_col("a")]);
            steps.extend(int_rows(&[1]));
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
            Some(steps)
        }
        _ => None,
    }
}

async fn drain(c: &mut hacivat_driver_mysql::MySqlConn) -> Vec<QueryEvent> {
    let mut out = Vec::new();
    loop {
        let e = event(c).await;
        let done = matches!(e, QueryEvent::Done(_));
        out.push(e);
        if done {
            return out;
        }
    }
}

#[tokio::test]
async fn mysql_binds_integer_hint_as_int() {
    let (mut c, server, setup) = scripted(false, prepared(6)).await;
    assert!(c.typed_caps().binds);
    let hostile = "x'); DROP TABLE t; --";
    let binds = [
        int(true, "-5"),
        int(false, "18446744073709551615"),
        // Not an integer: the text, for the server to convert.
        int(true, "4x"),
        // A sign is no unsigned integer.
        int(false, "-1"),
        text(hostile),
        BindValue::Null,
    ];
    let sql = "SELECT a FROM t WHERE a IN (?, ?, ?, ?, ?, ?)";
    let mut s = spec(sql, false, 501);
    s.binds = &binds;
    c.query(s).await.unwrap();
    drain(&mut c).await;
    let cmds = server.commands();
    let sent = &cmds[setup..];
    assert_eq!(sent[0].code, COM_STMT_PREPARE);
    // The statement text is exactly the caller's: no value in it.
    assert_eq!(sent[0].text(), sql);
    assert_eq!(sent[1].code, COM_STMT_EXECUTE);
    assert_eq!(
        exec_params(&sent[1].body, 6),
        vec![
            Param {
                ty: T_LONGLONG,
                flags: 0,
                value: Some((-5_i64).to_le_bytes().to_vec()),
            },
            Param {
                ty: T_LONGLONG,
                flags: UNSIGNED,
                value: Some(u64::MAX.to_le_bytes().to_vec()),
            },
            Param {
                ty: T_VAR_STRING,
                flags: 0,
                value: Some(b"4x".to_vec()),
            },
            Param {
                ty: T_VAR_STRING,
                flags: 0,
                value: Some(b"-1".to_vec()),
            },
            Param {
                ty: T_VAR_STRING,
                flags: 0,
                value: Some(hostile.as_bytes().to_vec()),
            },
            Param {
                ty: T_NULL,
                flags: 0,
                value: None,
            },
        ]
    );
    assert!(sent.iter().all(|cmd| cmd.code != COM_QUERY));
}

#[tokio::test]
async fn a_placeholder_count_that_is_not_the_bind_count_is_prepared_never_executed() {
    for (params, binds) in [
        (2_u16, vec![int(true, "1")]),
        (1, vec![int(true, "1"), int(true, "2")]),
        (1, vec![]),
        (0, vec![int(true, "1")]),
    ] {
        let (mut c, server, setup) = scripted(false, prepared(params)).await;
        let mut s = spec("SELECT a FROM t WHERE a = ?", false, 501);
        s.binds = &binds;
        assert_eq!(
            kind(c.query(s).await),
            RawErrorKind::Unsupported,
            "{params}"
        );
        let codes: Vec<u8> = server.commands()[setup..].iter().map(|c| c.code).collect();
        assert_eq!(codes, vec![COM_STMT_PREPARE], "{params}");
        c.ping().await.unwrap();
    }
}

#[tokio::test]
async fn binds_never_travel_over_com_query() {
    // A `SET` routes over `COM_QUERY`, which has no parameters: refused, nothing sent.
    let (mut c, server, setup) = scripted(false, |_| None).await;
    let binds = [int(true, "1")];
    let mut s = spec("SET @a = ?", false, 501);
    s.binds = &binds;
    assert_eq!(kind(c.query(s).await), RawErrorKind::Unsupported);
    assert_eq!(server.count(), setup);
    // Neither does a statement the server cannot prepare (1295): never re-sent as text.
    let (mut c, server, setup) = scripted(false, |cmd: &Cmd| match cmd.code {
        COM_STMT_PREPARE => Some(packets(vec![err_packet(
            1295,
            "HY000",
            "This command is not supported in the prepared statement protocol yet",
        )])),
        _ => None,
    })
    .await;
    let mut s = spec("DO ?", false, 501);
    s.binds = &binds;
    assert_eq!(kind(c.query(s).await), RawErrorKind::Unsupported);
    let codes: Vec<u8> = server.commands()[setup..].iter().map(|c| c.code).collect();
    assert_eq!(codes, vec![COM_STMT_PREPARE]);
}

#[tokio::test]
async fn mysql_multi_statement_refused_on_read_only() {
    let (mut c, server, setup) = scripted(true, |_| None).await;
    let mut s = spec("SELECT 1; DELETE FROM t", false, 501);
    s.protocol = Protocol::Simple;
    s.max_result_sets = 100;
    assert_eq!(kind(c.query(s).await), RawErrorKind::Unsupported);
    assert_eq!(server.count(), setup, "refused before anything was sent");
    c.ping().await.unwrap();
}

/// Three text result sets for one `COM_QUERY`, then the final OK packet with `warnings`.
fn three_sets(warnings: u16) -> impl Fn(&Cmd) -> Option<Vec<Step>> + Send + Sync + 'static {
    move |cmd: &Cmd| match cmd.code {
        COM_QUERY if cmd.text().starts_with("SELECT 1;") => {
            let mut steps = Vec::new();
            for n in ["1", "2", "3"] {
                steps.extend(header(&[text_col("n")]));
                steps.push(Step::Packet(text_row(&[Some(n.as_bytes())])));
                steps.push(Step::Packet(terminator(AUTOCOMMIT | MORE_RESULTS, 0)));
            }
            steps.push(Step::Packet(ok_packet(0, AUTOCOMMIT, warnings)));
            Some(steps)
        }
        COM_QUERY if cmd.text() == "SHOW WARNINGS" => {
            let cols = [text_col("Level"), long_col("Code"), text_col("Message")];
            let mut steps = header(&cols);
            steps.push(Step::Packet(text_row(&[
                Some("Warning".as_bytes()),
                Some("1265".as_bytes()),
                Some("Data truncated for column 'b' at row 1".as_bytes()),
            ])));
            let long = vec![b'w'; 20_000];
            steps.push(Step::Packet(text_row(&[
                Some("Note".as_bytes()),
                Some("1050".as_bytes()),
                Some(long.as_slice()),
            ])));
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
            Some(steps)
        }
        _ => None,
    }
}

#[tokio::test]
async fn run_as_one_is_one_com_query_with_its_sets_capped_and_counted() {
    let (mut c, server, setup) = scripted(false, three_sets(0)).await;
    let sql = "SELECT 1; SELECT 2; SELECT 3";
    let mut s = spec(sql, true, 1);
    s.protocol = Protocol::Simple;
    s.max_result_sets = 2;
    c.query(s).await.unwrap();
    let events = drain(&mut c).await;
    let shape: Vec<&str> = events
        .iter()
        .map(|e| match e {
            QueryEvent::ResultStart(_) => "start",
            QueryEvent::Rows(_) => "rows",
            QueryEvent::ResultEnd(_) => "end",
            QueryEvent::Done(_) => "done",
            QueryEvent::Suspended => "suspended",
            QueryEvent::Notice(_) => "notice",
        })
        .collect();
    // Never suspends, though asked to.
    assert_eq!(
        shape,
        ["start", "rows", "end", "start", "rows", "end", "done"]
    );
    let QueryEvent::Done(done) = events.last().unwrap() else {
        unreachable!()
    };
    assert_eq!(done.skipped_result_sets, 1);
    let tags: Vec<Option<&str>> = events
        .iter()
        .filter_map(|e| match e {
            QueryEvent::ResultEnd(end) => Some(end.command_tag.as_deref()),
            _ => None,
        })
        .collect();
    assert_eq!(tags, [Some("SELECT"), None]);
    let sent = &server.commands()[setup..];
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].code, sent[0].text()), (COM_QUERY, sql.to_string()));
    // Binds never go with it.
    let binds = [int(true, "1")];
    let mut s = spec("SELECT ?; SELECT 2", false, 1);
    s.protocol = Protocol::Simple;
    s.binds = &binds;
    assert_eq!(kind(c.query(s).await), RawErrorKind::Unsupported);
    assert_eq!(server.count(), setup + 1);
}

#[tokio::test]
async fn warnings_are_read_after_done_capped_and_never_on_read_only() {
    let (mut c, server, setup) = scripted(false, three_sets(2)).await;
    // Nothing to read before a statement with warnings.
    assert!(c.warnings().await.unwrap().is_empty());
    assert_eq!(server.count(), setup);
    let mut s = spec("SELECT 1; SELECT 2; SELECT 3", false, 501);
    s.protocol = Protocol::Simple;
    c.query(s).await.unwrap();
    // Not while the query is open.
    assert_eq!(kind(c.warnings().await), RawErrorKind::Unsupported);
    let QueryEvent::Done(done) = drain(&mut c).await.pop().unwrap() else {
        unreachable!()
    };
    assert_eq!(done.warnings, 2);
    let w = c.warnings().await.unwrap();
    assert_eq!(w.len(), 2);
    assert_eq!(w[0].severity, NoticeSeverity::Warning);
    assert_eq!(w[0].code.as_deref(), Some("1265"));
    assert_eq!(&*w[0].text, "Data truncated for column 'b' at row 1");
    assert_eq!(w[1].severity, NoticeSeverity::Notice);
    assert_eq!(w[1].text.len(), 16 * 1024, "cut at 16 KiB");
    let last = server.commands().last().unwrap().clone();
    assert_eq!(
        (last.code, last.text()),
        (COM_QUERY, "SHOW WARNINGS".to_string())
    );
    c.ping().await.unwrap();

    // A read-only connection reports the count only: no SHOW WARNINGS.
    let (mut c, server, setup) = scripted(true, |cmd: &Cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[long_col("a")], 0)),
        COM_STMT_EXECUTE => {
            let mut steps = header(&[long_col("a")]);
            steps.extend(int_rows(&[1]));
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 3)));
            Some(steps)
        }
        _ => None,
    })
    .await;
    c.query(spec("SELECT a FROM t", false, 501)).await.unwrap();
    let QueryEvent::Done(done) = drain(&mut c).await.pop().unwrap() else {
        unreachable!()
    };
    assert_eq!(done.warnings, 3);
    let before = server.count();
    assert!(c.warnings().await.unwrap().is_empty());
    assert_eq!(server.count(), before);
    assert!(
        server.commands()[setup..]
            .iter()
            .all(|cmd| !cmd.text().contains("WARNINGS"))
    );
}

// ## m3b-t13-drivers (fix round 1)

fn bytes(s: &str) -> BindValue {
    typed(LogicalType::Bytes { max: None }, s)
}

#[test]
fn only_a_canonical_integer_text_binds_as_an_integer() {
    use hacivat_driver_mysql::typed::bind_value;
    use mysql_async::Value as V;
    let b = |s: &str| V::Bytes(s.as_bytes().to_vec());
    for (bind, want) in [
        (int(true, "0"), V::Int(0)),
        (int(true, "-5"), V::Int(-5)),
        (int(true, "-9223372036854775808"), V::Int(i64::MIN)),
        (int(false, "18446744073709551615"), V::UInt(u64::MAX)),
        (int(false, "+5"), b("+5")),
        (int(true, "+5"), b("+5")),
        (int(true, "007"), b("007")),
        (int(false, "007"), b("007")),
        (int(true, "-0"), b("-0")),
        (int(false, "-0"), b("-0")),
        (int(false, "-1"), b("-1")),
        (int(true, " 5"), b(" 5")),
        (int(true, "٣"), b("٣")),
        (int(true, "１"), b("１")),
        (int(true, "0x10"), b("0x10")),
        (int(true, "1_0"), b("1_0")),
        (
            int(false, "18446744073709551616"),
            b("18446744073709551616"),
        ),
        (BindValue::Null, V::NULL),
    ] {
        assert_eq!(bind_value(&bind), Some(want), "{bind:?}");
    }
}

#[test]
fn bytes_hints_decode_backslash_x_hex_and_refuse_it_malformed() {
    use hacivat_driver_mysql::typed::{HexText, bind_value, hex_bytes};
    use mysql_async::Value as V;
    assert_eq!(hex_bytes("\\x00ff"), HexText::Bytes(vec![0x00, 0xff]));
    assert_eq!(hex_bytes("\\xABcd"), HexText::Bytes(vec![0xab, 0xcd]));
    assert_eq!(hex_bytes("\\x"), HexText::Bytes(vec![]));
    for bad in ["\\x0", "\\x0g", "\\xzz", "\\X00", "\\x 0", "\\x00f"] {
        assert_eq!(hex_bytes(bad), HexText::Malformed, "{bad:?}");
        assert_eq!(bind_value(&bytes(bad)), None, "{bad:?}");
    }
    for plain in ["", "abc", "x00", "\\\\x00", "0x00"] {
        assert_eq!(hex_bytes(plain), HexText::NotHex, "{plain:?}");
        assert_eq!(
            bind_value(&bytes(plain)),
            Some(V::Bytes(plain.as_bytes().to_vec()))
        );
    }
    // Only a bytes hint reads hex: a text that looks like it stays text.
    assert_eq!(
        bind_value(&text("\\x00ff")),
        Some(V::Bytes(b"\\x00ff".to_vec()))
    );
}

#[tokio::test]
async fn bytes_binds_go_as_decoded_bytes_and_malformed_ones_send_nothing() {
    let (mut c, server, setup) = scripted(false, prepared(3)).await;
    let binds = [bytes("\\x00ff"), bytes("abc"), bytes("\\x")];
    let mut s = spec("SELECT a FROM t WHERE a IN (?, ?, ?)", false, 501);
    s.binds = &binds;
    c.query(s).await.unwrap();
    drain(&mut c).await;
    let sent = server.commands();
    let exec = sent[setup..]
        .iter()
        .find(|cmd| cmd.code == COM_STMT_EXECUTE)
        .unwrap();
    let values: Vec<Option<Vec<u8>>> = exec_params(&exec.body, 3)
        .into_iter()
        .map(|p| {
            assert_eq!(p.ty, T_VAR_STRING);
            p.value
        })
        .collect();
    assert_eq!(
        values,
        [Some(vec![0x00, 0xff]), Some(b"abc".to_vec()), Some(vec![])]
    );
    // Malformed: refused before anything is sent (not even the prepare).
    let before = server.count();
    let bad = [bytes("\\x0f0")];
    let mut s = spec("SELECT a FROM t WHERE a = ?", false, 501);
    s.binds = &bad;
    assert_eq!(kind(c.query(s).await), RawErrorKind::Unsupported);
    assert_eq!(server.count(), before);
    c.ping().await.unwrap();
}

#[tokio::test]
async fn show_warnings_keeps_at_most_one_hundred_rows() {
    let (mut c, _server, _) = scripted(false, |cmd: &Cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[long_col("a")], 0)),
        COM_STMT_EXECUTE => {
            let mut steps = header(&[long_col("a")]);
            steps.extend(int_rows(&[1]));
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 150)));
            Some(steps)
        }
        COM_QUERY if cmd.text() == "SHOW WARNINGS" => {
            let cols = [text_col("Level"), long_col("Code"), text_col("Message")];
            let mut steps = header(&cols);
            for i in 0..150 {
                let message = format!("w{i}");
                steps.push(Step::Packet(text_row(&[
                    Some("Warning".as_bytes()),
                    Some("1292".as_bytes()),
                    Some(message.as_bytes()),
                ])));
            }
            steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
            Some(steps)
        }
        _ => None,
    })
    .await;
    c.query(spec("SELECT a FROM t", false, 501)).await.unwrap();
    let QueryEvent::Done(done) = drain(&mut c).await.pop().unwrap() else {
        unreachable!()
    };
    assert_eq!(done.warnings, 150);
    let w = c.warnings().await.unwrap();
    let texts: Vec<String> = w.iter().map(|n| n.text.to_string()).collect();
    let want: Vec<String> = (0..100).map(|i| format!("w{i}")).collect();
    assert_eq!(texts, want);
    // The rows past the cap were read: the connection is in step.
    c.ping().await.unwrap();
}
