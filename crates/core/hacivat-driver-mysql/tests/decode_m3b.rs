//! M3b Task 7 (R-M3-67): the MySQL and MariaDB type matrix over synthetic column definitions,
//! read back through a real chunk. `BIT(n)` is a bit string (`0101`, as PostgreSQL's `bit`;
//! P-M3b-26), geometry is typed (its internal value as `HEX()` prints it), `VECTOR` stays opaque,
//! a `TIMESTAMP` column carries the session `time_zone` (ledger 194), and the header label comes
//! from the native type (ledger 246). Hostile bytes give an `Error` cell, never a panic.

mod support;

use std::sync::Arc;

use hacivat_driver::{QueryEvent, RawConn};
use hacivat_driver_mysql::decode::{
    BINARY_CHARSET, Plan, buf_kind_of, decode_cell_bin, decode_cell_text, logical_of,
};
use hacivat_values::valfmt::type_label;
use hacivat_values::{BufKind, LogicalType, NativeType, Value, VarKind};
use mysql_async::Column;
use mysql_async::Value as My;
use mysql_async::consts::{ColumnFlags, ColumnType};
use support::{
    AUTOCOMMIT, BINARY, COM_QUERY, COM_STMT_EXECUTE, COM_STMT_PREPARE, Step, T_DATETIME,
    T_LONGLONG, T_VAR_STRING, UTF8MB4, column_def, err_packet, event, header, packets, prepare_ok,
    scripted, spec, terminator, text_row,
};

const MIB: u32 = 1 << 20;

fn col(t: ColumnType) -> Column {
    Column::new(t).with_name(b"c")
}

fn bit_col(n: u32) -> Column {
    col(ColumnType::MYSQL_TYPE_BIT)
        .with_column_length(n)
        .with_flags(ColumnFlags::UNSIGNED_FLAG)
        .with_character_set(BINARY_CHARSET)
}

fn geometry_col() -> Column {
    col(ColumnType::MYSQL_TYPE_GEOMETRY)
        .with_character_set(BINARY_CHARSET)
        .with_column_length(u32::MAX)
        .with_flags(ColumnFlags::BLOB_FLAG | ColumnFlags::BINARY_FLAG)
}

fn text_of(v: Value) -> String {
    match v {
        Value::Text(t) => t.to_string(),
        other => panic!("not a text cell: {other:?}"),
    }
}

/// A `BIT(n)` cell in the binary protocol.
fn decode_mysql_bit(bytes: &[u8], n: u32) -> Value {
    decode_cell_bin(&bit_col(n), &My::Bytes(bytes.to_vec()), MIB)
}

#[test]
fn mysql_bit_is_a_bit_string() {
    assert_eq!(text_of(decode_mysql_bit(&[0b0000_0101], 4)), "0101");
}

#[test]
fn bit_columns_are_typed_bit_strings_in_both_protocols() {
    assert_eq!(
        logical_of(&bit_col(5)),
        LogicalType::BitString { len: Some(5) }
    );
    assert_eq!(buf_kind_of(&bit_col(5)), BufKind::Var(VarKind::Text));
    assert_eq!(text_of(decode_mysql_bit(&[0b10110], 5)), "10110");
    assert_eq!(text_of(decode_mysql_bit(&[0], 1)), "0");
    assert_eq!(
        text_of(decode_cell_text(
            &bit_col(64),
            Some(&[0x80, 0, 0, 0, 0, 0, 0, 1]),
            MIB
        )),
        format!("1{}1", "0".repeat(62))
    );
    // `BIT(12)` arrives in two bytes; the leading pad bits are not part of the value.
    assert_eq!(text_of(decode_mysql_bit(&[0x0f, 0xff], 12)), "111111111111");
    // Longer than 8 bytes: no server sends it.
    assert!(matches!(
        decode_mysql_bit(&[0; 9], 64),
        Value::Error(e) if &*e.code == "mysql-bit"
    ));
    assert_eq!(decode_cell_text(&bit_col(5), None, MIB), Value::Null);
}

#[test]
fn geometry_is_typed_with_its_internal_value_as_hex() {
    let geom = geometry_col();
    assert_eq!(logical_of(&geom), LogicalType::Geometry { srid: None });
    assert_eq!(buf_kind_of(&geom), BufKind::Var(VarKind::Text));
    // POINT(1 1) with SRID 0: the SRID (4 bytes, little-endian), then the WKB.
    let mut point = vec![0, 0, 0, 0, 1, 1, 0, 0, 0];
    point.extend_from_slice(&1.0_f64.to_le_bytes());
    point.extend_from_slice(&1.0_f64.to_le_bytes());
    assert_eq!(
        text_of(decode_cell_bin(&geom, &My::Bytes(point.clone()), MIB)),
        "000000000101000000000000000000F03F000000000000F03F"
    );
    assert_eq!(
        text_of(decode_cell_text(&geom, Some(&point), MIB)),
        "000000000101000000000000000000F03F000000000000F03F"
    );
    // Past the cap the hex text is truncated, its whole length counted.
    match decode_cell_bin(&geom, &My::Bytes(vec![0xAB; 10]), 4) {
        Value::Truncated(t) => {
            assert_eq!(t.total_len, Some(20));
            assert_eq!(t.prefix, Value::Text("ABAB".into()));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn vector_stays_opaque() {
    let vector = col(ColumnType::MYSQL_TYPE_VECTOR)
        .with_character_set(BINARY_CHARSET)
        .with_column_length(8);
    assert_eq!(
        logical_of(&vector),
        LogicalType::Opaque {
            type_name: "vector".into()
        }
    );
    assert!(matches!(
        decode_cell_text(&vector, Some(b"[1,2]"), MIB),
        Value::Opaque(o) if &*o.text == "[1,2]"
    ));
}

fn time_zone_of(native: &NativeType) -> Option<Arc<str>> {
    match native {
        NativeType::MySql { time_zone, .. } => time_zone.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn timestamp_columns_carry_the_session_time_zone() {
    let columns = [
        col(ColumnType::MYSQL_TYPE_TIMESTAMP)
            .with_decimals(3)
            .with_character_set(BINARY_CHARSET),
        col(ColumnType::MYSQL_TYPE_DATETIME).with_character_set(BINARY_CHARSET),
        col(ColumnType::MYSQL_TYPE_LONG).with_character_set(BINARY_CHARSET),
    ];
    let zone: Arc<str> = "+03:00".into();
    let plan = Plan::for_columns_in(&columns, Some(&zone));
    let cols = &plan.schema.columns;
    assert_eq!(time_zone_of(&cols[0].native).as_deref(), Some("+03:00"));
    assert_eq!(time_zone_of(&cols[1].native), None, "DATETIME has no zone");
    assert_eq!(time_zone_of(&cols[2].native), None);
    assert_eq!(plan.time_zone.as_deref(), Some("+03:00"));
    // Without a session zone (a connection that could not read it) nothing is claimed.
    let plan = Plan::for_columns(&columns);
    assert_eq!(time_zone_of(&plan.schema.columns[0].native), None);
}

/// `TIMESTAMP`'s column type code.
const T_TIMESTAMP: u8 = 7;

/// The connection's setup query, answered with a time zone beside `net_write_timeout`.
fn timeout_and_zone(zone: &str) -> Vec<Step> {
    let mut steps = header(&[
        column_def("@@net_write_timeout", T_LONGLONG, 0, BINARY, 21, 0),
        column_def("@@session.time_zone", T_VAR_STRING, 0, UTF8MB4, 256, 0),
    ]);
    steps.push(Step::Packet(text_row(&[
        Some(b"60"),
        Some(zone.as_bytes()),
    ])));
    steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
    steps
}

#[tokio::test]
async fn the_connection_reads_the_session_time_zone_in_its_setup_query() {
    let ts = column_def("ts", T_TIMESTAMP, 0, BINARY, 23, 3);
    let dt = column_def("dt", T_DATETIME, 0, BINARY, 19, 0);
    let cols = [ts.clone(), dt.clone()];
    let (mut c, server, _) = scripted(false, move |cmd| {
        if cmd.code == COM_QUERY && cmd.text().contains("@@net_write_timeout") {
            return Some(timeout_and_zone("Europe/Istanbul"));
        }
        match cmd.code {
            COM_STMT_PREPARE => Some(prepare_ok(&cols, 0)),
            COM_STMT_EXECUTE => {
                let mut steps = header(&cols);
                steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
                Some(steps)
            }
            _ => None,
        }
    })
    .await;
    // One round trip reads both, as before (no extra query for the zone).
    let setup: Vec<String> = server
        .commands()
        .iter()
        .filter(|cmd| cmd.text().contains("time_zone"))
        .map(|cmd| cmd.text())
        .collect();
    // ## m3b-t18-safe06 (named exception): the same round trip reads the session `sql_mode`
    // too (R-M3-57, the T18 carry: no round trip of its own).
    assert_eq!(
        setup,
        ["SELECT @@net_write_timeout, @@session.time_zone, @@session.sql_mode, DATABASE()"]
    );
    c.query(spec("SELECT ts, dt FROM t", false, 501))
        .await
        .unwrap();
    let QueryEvent::ResultStart(schema) = event(&mut c).await else {
        panic!("ResultStart first");
    };
    assert_eq!(
        time_zone_of(&schema.columns[0].native).as_deref(),
        Some("Europe/Istanbul")
    );
    assert_eq!(time_zone_of(&schema.columns[1].native), None);
    assert_eq!(type_label(&schema.columns[0].native), "timestamp(3)");
}

#[tokio::test]
async fn a_server_that_refuses_the_zone_still_reports_net_write_timeout() {
    let ts = column_def("ts", T_TIMESTAMP, 0, BINARY, 19, 0);
    let cols = [ts];
    let (mut c, server, _) = scripted(false, move |cmd| {
        if cmd.code == COM_QUERY && cmd.text().contains("time_zone") {
            return Some(packets(vec![err_packet(
                1193,
                "HY000",
                "Unknown system variable 'time_zone'",
            )]));
        }
        match cmd.code {
            COM_STMT_PREPARE => Some(prepare_ok(&cols, 0)),
            COM_STMT_EXECUTE => {
                let mut steps = header(&cols);
                steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
                Some(steps)
            }
            _ => None,
        }
    })
    .await;
    // The fallback asked for `net_write_timeout` alone: suspension keeps its idle bound.
    assert!(c.typed_caps().suspend);
    assert!(
        server
            .commands()
            .iter()
            .any(|cmd| cmd.text() == "SELECT @@net_write_timeout")
    );
    c.query(spec("SELECT ts FROM t", false, 501)).await.unwrap();
    let QueryEvent::ResultStart(schema) = event(&mut c).await else {
        panic!("ResultStart first");
    };
    assert_eq!(time_zone_of(&schema.columns[0].native), None);
}

#[test]
fn the_plans_native_types_give_mysql_labels() {
    let columns = [
        col(ColumnType::MYSQL_TYPE_NEWDECIMAL)
            .with_column_length(16)
            .with_decimals(2)
            .with_character_set(BINARY_CHARSET),
        col(ColumnType::MYSQL_TYPE_LONGLONG)
            .with_flags(ColumnFlags::UNSIGNED_FLAG)
            .with_column_length(20)
            .with_character_set(BINARY_CHARSET),
        bit_col(5),
    ];
    let plan = Plan::for_columns(&columns);
    let labels: Vec<String> = plan
        .schema
        .columns
        .iter()
        .map(|c| type_label(&c.native))
        .collect();
    assert_eq!(labels, ["decimal(14,2)", "bigint unsigned", "bit(5)"]);
}

// ## m3b-t18-safe06 (named exception; R-M3-57, plan amendment C2(b)/I1)
#[tokio::test]
async fn the_connection_reads_the_session_sql_mode_in_its_setup_query() {
    // The third value of the same round trip is the session's `sql_mode`, verbatim, and the
    // fourth the current database (the T20 carry); a server that answers only the first two
    // leaves both unread.
    for (answer, want) in [
        (
            Some("ANSI_QUOTES,NO_BACKSLASH_ESCAPES"),
            Some("ANSI_QUOTES,NO_BACKSLASH_ESCAPES"),
        ),
        (None, None),
    ] {
        let (c, server, _) = scripted(true, move |cmd| {
            if cmd.code == COM_QUERY && cmd.text().contains("@@net_write_timeout") {
                let Some(mode) = answer else {
                    return Some(timeout_and_zone("SYSTEM"));
                };
                let mut steps = header(&[
                    column_def("@@net_write_timeout", T_LONGLONG, 0, BINARY, 21, 0),
                    column_def("@@session.time_zone", T_VAR_STRING, 0, UTF8MB4, 256, 0),
                    column_def("@@session.sql_mode", T_VAR_STRING, 0, UTF8MB4, 256, 0),
                    column_def("DATABASE()", T_VAR_STRING, 0, UTF8MB4, 256, 0),
                ]);
                steps.push(Step::Packet(text_row(&[
                    Some(b"60"),
                    Some(b"SYSTEM"),
                    Some(mode.as_bytes()),
                    Some(b"hv_test"),
                ])));
                steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
                return Some(steps);
            }
            None
        })
        .await;
        assert_eq!(c.session_settings().lex_modes.as_deref(), want);
        assert_eq!(
            c.session_settings().search_path.as_deref(),
            want.map(|_| "hv_test")
        );
        // No round trip of its own.
        let reads = server
            .commands()
            .iter()
            .filter(|cmd| cmd.text().contains("sql_mode"))
            .count();
        assert_eq!(reads, 1);
    }
}
