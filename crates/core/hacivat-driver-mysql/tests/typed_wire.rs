//! The typed path (Task 14) and `mysql_async` patch §6 against a scripted in-process server
//! that speaks just enough of the MySQL protocol: rows and headers out of shape (always
//! `Protocol`, the connection broken, never a panic), `Unsupported` while a query is open (and
//! nothing sent), suspension and resume in the binary protocol, the drain-then-kill close, the
//! result-set rules and the 1295 route. No Docker.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hacivat_driver::{
    CloseHow, Closed, DriverConnect, Endpoint, ExecOpts, InsecureAuth, Protocol, QueryDone,
    QueryEvent, QuerySpec, RawConn, RawError, RawErrorKind, ResultEnd, TxStatus, TypedCaps,
};
use hacivat_driver_mysql::{MySqlConn, connect_conn};
use hacivat_net::{Target, direct};
use hacivat_values::{BindValue, Chunk, TsVal, ValueRef};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

// Protocol constants -----------------------------------------------------------------------

const COM_QUIT: u8 = 0x01;
const COM_QUERY: u8 = 0x03;
const COM_PING: u8 = 0x0E;
const COM_STMT_PREPARE: u8 = 0x16;
const COM_STMT_EXECUTE: u8 = 0x17;
const COM_STMT_CLOSE: u8 = 0x19;
const COM_RESET_CONNECTION: u8 = 0x1F;

const T_LONG: u8 = 3;
const T_DOUBLE: u8 = 5;
const T_LONGLONG: u8 = 8;
const T_DATE: u8 = 10;
const T_DATETIME: u8 = 12;
const T_TIME2: u8 = 19;
const T_VAR_STRING: u8 = 253;

const AUTOCOMMIT: u16 = 0x0002;
const IN_TRANS: u16 = 0x0001;
const MORE_RESULTS: u16 = 0x0008;

const BINARY: u16 = 63;
const UTF8MB4: u16 = 255;
/// The connection ID the greeting announces.
const THREAD_ID: u32 = 7;

// Packet builders --------------------------------------------------------------------------

fn lenenc_int(n: u64) -> Vec<u8> {
    match n {
        0..=250 => vec![n as u8],
        251..=0xFFFF => [&[0xFC][..], &(n as u16).to_le_bytes()].concat(),
        0x1_0000..=0xFF_FFFF => [&[0xFD][..], &(n as u32).to_le_bytes()[..3]].concat(),
        _ => [&[0xFE][..], &n.to_le_bytes()].concat(),
    }
}

fn lenenc_str(b: &[u8]) -> Vec<u8> {
    [lenenc_int(b.len() as u64), b.to_vec()].concat()
}

fn ok_packet(affected: u64, status: u16, warnings: u16) -> Vec<u8> {
    [
        vec![0x00],
        lenenc_int(affected),
        lenenc_int(0),
        status.to_le_bytes().to_vec(),
        warnings.to_le_bytes().to_vec(),
    ]
    .concat()
}

/// The end of a result set under `CLIENT_DEPRECATE_EOF`: an OK packet with the `0xFE` header.
fn terminator(status: u16, warnings: u16) -> Vec<u8> {
    let mut p = ok_packet(0, status, warnings);
    p[0] = 0xFE;
    p
}

fn err_packet(code: u16, state: &str, message: &str) -> Vec<u8> {
    [
        vec![0xFF],
        code.to_le_bytes().to_vec(),
        b"#".to_vec(),
        state.as_bytes().to_vec(),
        message.as_bytes().to_vec(),
    ]
    .concat()
}

fn column_def(name: &str, ty: u8, flags: u16, charset: u16, length: u32, decimals: u8) -> Vec<u8> {
    [
        lenenc_str(b"def"),
        lenenc_str(b"hv"),
        lenenc_str(b"t"),
        lenenc_str(b"t"),
        lenenc_str(name.as_bytes()),
        lenenc_str(name.as_bytes()),
        vec![0x0C],
        charset.to_le_bytes().to_vec(),
        length.to_le_bytes().to_vec(),
        vec![ty],
        flags.to_le_bytes().to_vec(),
        vec![decimals, 0, 0],
    ]
    .concat()
}

fn long_col(name: &str) -> Vec<u8> {
    column_def(name, T_LONG, 0, BINARY, 11, 0)
}

fn text_row(cells: &[Option<&[u8]>]) -> Vec<u8> {
    cells
        .iter()
        .flat_map(|c| match c {
            Some(b) => lenenc_str(b),
            None => vec![0xFB],
        })
        .collect()
}

/// A binary row: the header, the NULL bitmap (offset 2) and the non-NULL values as encoded.
fn bin_row(cells: &[Option<Vec<u8>>]) -> Vec<u8> {
    let mut bitmap = vec![0_u8; (cells.len() + 7 + 2) / 8];
    for (i, c) in cells.iter().enumerate() {
        if c.is_none() {
            bitmap[(i + 2) / 8] |= 1 << ((i + 2) % 8);
        }
    }
    let values: Vec<u8> = cells.iter().flatten().flatten().copied().collect();
    [vec![0x00], bitmap, values].concat()
}

fn long(v: i32) -> Option<Vec<u8>> {
    Some(v.to_le_bytes().to_vec())
}

/// The prepare reply for a statement with `columns` and `params` placeholders.
fn prepare_ok(columns: &[Vec<u8>], params: u16) -> Vec<Step> {
    let mut first = vec![0x00];
    first.extend_from_slice(&1_u32.to_le_bytes());
    first.extend_from_slice(&(columns.len() as u16).to_le_bytes());
    first.extend_from_slice(&params.to_le_bytes());
    first.extend_from_slice(&[0, 0, 0]);
    let mut steps = vec![Step::Packet(first)];
    for _ in 0..params {
        steps.push(Step::Packet(long_col("?")));
    }
    steps.extend(columns.iter().cloned().map(Step::Packet));
    steps
}

/// A result set's header: the column count and the definitions.
fn header(columns: &[Vec<u8>]) -> Vec<Step> {
    let mut steps = vec![Step::Packet(lenenc_int(columns.len() as u64))];
    steps.extend(columns.iter().cloned().map(Step::Packet));
    steps
}

fn packets(ps: Vec<Vec<u8>>) -> Vec<Step> {
    ps.into_iter().map(Step::Packet).collect()
}

// The scripted server ----------------------------------------------------------------------

/// One step of a reply: a packet, or a wait until the test releases the rest.
#[derive(Clone)]
enum Step {
    Packet(Vec<u8>),
    Wait(Arc<Notify>),
}

#[derive(Debug, Clone)]
struct Cmd {
    code: u8,
    body: Vec<u8>,
}

impl Cmd {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// The test's replies; `None` falls back to [`default_reply`].
type Handler = Arc<dyn Fn(&Cmd) -> Option<Vec<Step>> + Send + Sync>;

struct Server {
    addr: SocketAddr,
    log: Arc<Mutex<Vec<Cmd>>>,
}

impl Server {
    /// The commands received after the connection's own setup, in order.
    fn commands(&self) -> Vec<Cmd> {
        self.log.lock().unwrap().clone()
    }

    fn count(&self) -> usize {
        self.log.lock().unwrap().len()
    }
}

fn greeting() -> Vec<u8> {
    // LONG_PASSWORD | LONG_FLAG | PROTOCOL_41 | TRANSACTIONS | SECURE_CONNECTION
    // | MULTI_STATEMENTS | MULTI_RESULTS | PS_MULTI_RESULTS | PLUGIN_AUTH | DEPRECATE_EOF
    let caps: u32 = 0x0000_0001
        | 0x0000_0004
        | 0x0000_0200
        | 0x0000_2000
        | 0x0000_8000
        | 0x0001_0000
        | 0x0002_0000
        | 0x0004_0000
        | 0x0008_0000
        | 0x0100_0000;
    let mut p = vec![10_u8];
    p.extend_from_slice(b"8.4.0-hv\0");
    p.extend_from_slice(&THREAD_ID.to_le_bytes());
    p.extend_from_slice(b"abcdefgh\0");
    p.extend_from_slice(&(caps as u16).to_le_bytes());
    p.push(45);
    p.extend_from_slice(&AUTOCOMMIT.to_le_bytes());
    p.extend_from_slice(&((caps >> 16) as u16).to_le_bytes());
    p.push(21);
    p.extend_from_slice(&[0_u8; 10]);
    p.extend_from_slice(b"ijklmnopqrst\0");
    p.extend_from_slice(b"mysql_native_password\0");
    p
}

async fn write_packet(s: &mut TcpStream, seq: u8, body: &[u8]) -> bool {
    let len = (body.len() as u32).to_le_bytes();
    let mut out = vec![len[0], len[1], len[2], seq];
    out.extend_from_slice(body);
    s.write_all(&out).await.is_ok()
}

async fn read_packet(s: &mut TcpStream) -> Option<(u8, Vec<u8>)> {
    let mut hdr = [0_u8; 4];
    s.read_exact(&mut hdr).await.ok()?;
    let len = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], 0]) as usize;
    let mut body = vec![0_u8; len];
    s.read_exact(&mut body).await.ok()?;
    Some((hdr[3], body))
}

/// A text result set of one row of named `BIGINT` text values.
fn variables(cells: &[(&str, &str)]) -> Vec<Step> {
    let defs: Vec<Vec<u8>> = cells
        .iter()
        .map(|(name, _)| column_def(name, T_LONGLONG, 0, BINARY, 21, 0))
        .collect();
    let mut steps = header(&defs);
    let row: Vec<Option<&[u8]>> = cells.iter().map(|(_, v)| Some(v.as_bytes())).collect();
    steps.push(Step::Packet(text_row(&row)));
    steps.push(Step::Packet(terminator(AUTOCOMMIT, 0)));
    steps
}

/// What a server answers that the test does not script: the connection's own setup queries,
/// `SET`s, pings, resets; anything else is an error.
fn default_reply(cmd: &Cmd) -> Vec<Step> {
    match cmd.code {
        COM_QUERY if cmd.text().contains("@@max_allowed_packet") => variables(&[
            ("@@max_allowed_packet", "67108864"),
            ("@@wait_timeout", "28800"),
        ]),
        COM_QUERY if cmd.text().contains("@@net_write_timeout") => {
            variables(&[("@@net_write_timeout", "60")])
        }
        COM_QUERY | COM_PING | COM_RESET_CONNECTION => packets(vec![ok_packet(0, AUTOCOMMIT, 0)]),
        COM_STMT_CLOSE | COM_QUIT => Vec::new(),
        _ => packets(vec![err_packet(1047, "08S01", "Unknown command")]),
    }
}

/// Serves one connection: the greeting, an OK for any sign-in, then every command by
/// `handler` (else [`default_reply`]), recording each.
async fn serve(handler: Handler) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = log.clone();
    tokio::spawn(async move {
        let Ok((mut s, _)) = listener.accept().await else {
            return;
        };
        if !write_packet(&mut s, 0, &greeting()).await {
            return;
        }
        let Some((seq, _response)) = read_packet(&mut s).await else {
            return;
        };
        if !write_packet(&mut s, seq.wrapping_add(1), &ok_packet(0, AUTOCOMMIT, 0)).await {
            return;
        }
        while let Some((_, body)) = read_packet(&mut s).await {
            let Some((&code, rest)) = body.split_first() else {
                return;
            };
            let cmd = Cmd {
                code,
                body: rest.to_vec(),
            };
            seen.lock().unwrap().push(cmd.clone());
            if code == COM_QUIT {
                return;
            }
            let steps = handler(&cmd).unwrap_or_else(|| default_reply(&cmd));
            let mut seq = 1_u8;
            for step in steps {
                match step {
                    Step::Packet(p) => {
                        if !write_packet(&mut s, seq, &p).await {
                            return;
                        }
                        seq = seq.wrapping_add(1);
                    }
                    Step::Wait(n) => n.notified().await,
                }
            }
        }
    });
    Server { addr, log }
}

fn request(addr: SocketAddr, read_only: bool) -> DriverConnect {
    let target = Target::Tcp {
        host: "127.0.0.1".into(),
        port: addr.port(),
    };
    DriverConnect {
        endpoint: Endpoint::Network {
            factory: direct(target.clone(), vec![addr], Duration::from_secs(5)),
            direct: Some(target),
            resolved: vec![addr],
            tls_host: "127.0.0.1".into(),
        },
        tls: None,
        user: Some("hv".into()),
        password: None,
        database: None,
        application_name: "Hacivat tests".into(),
        read_only,
        options: vec![],
        connect_timeout: Duration::from_secs(5),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::Allow,
    }
}

/// A connection to a server scripted by `handler`, and how many commands its setup sent.
async fn scripted(
    read_only: bool,
    handler: impl Fn(&Cmd) -> Option<Vec<Step>> + Send + Sync + 'static,
) -> (MySqlConn, Server, usize) {
    let server = serve(Arc::new(handler)).await;
    let conn = match connect_conn(request(server.addr, read_only)).await {
        Ok(c) => c,
        Err(e) => panic!("connect: {:?}", e.kind),
    };
    let setup = server.count();
    (conn, server, setup)
}

fn spec(sql: &str, suspendable: bool, first_rows: u32) -> QuerySpec<'_> {
    QuerySpec {
        sql,
        binds: &[],
        protocol: Protocol::Extended,
        suspendable,
        first_rows,
        chunk_rows: 4_096,
        chunk_bytes: 4 << 20,
        cell_cap: 1 << 20,
        max_result_sets: 1,
    }
}

fn kind<T>(r: Result<T, RawError>) -> RawErrorKind {
    match r {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.kind,
    }
}

fn vendor(e: &RawError) -> Option<i64> {
    e.server.as_ref().and_then(|s| s.vendor_code)
}

async fn event(c: &mut MySqlConn) -> QueryEvent {
    match c.next_event().await {
        Ok(e) => e,
        Err(e) => panic!("next_event: {:?}", e.kind),
    }
}

fn rows_of(e: QueryEvent) -> Chunk {
    match e {
        QueryEvent::Rows(c) => c,
        other => panic!("expected rows, got {other:?}"),
    }
}

fn int_cells(c: &Chunk, col: u32) -> Vec<i64> {
    (0..c.rows())
        .map(|r| match c.cell(r, col) {
            ValueRef::Int(v) => v,
            other => panic!("{other:?}"),
        })
        .collect()
}

fn end_of(e: QueryEvent) -> ResultEnd {
    match e {
        QueryEvent::ResultEnd(end) => end,
        other => panic!("expected the result's end, got {other:?}"),
    }
}

fn done_of(e: QueryEvent) -> QueryDone {
    match e {
        QueryEvent::Done(done) => done,
        other => panic!("expected done, got {other:?}"),
    }
}

/// A prepared statement of one `INT` column `a` whose execution answers `rows`.
fn one_int_column(rows: Vec<Step>) -> impl Fn(&Cmd) -> Option<Vec<Step>> + Send + Sync + 'static {
    let rows = Arc::new(Mutex::new(Some(rows)));
    move |cmd: &Cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(&[long_col("a")], 0)),
        COM_STMT_EXECUTE => {
            let mut steps = header(&[long_col("a")]);
            steps.extend(rows.lock().unwrap().take().unwrap_or_default());
            Some(steps)
        }
        _ => None,
    }
}

fn int_rows(values: &[i32]) -> Vec<Step> {
    values
        .iter()
        .map(|v| Step::Packet(bin_row(&[long(*v)])))
        .collect()
}

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
    // The kill came too late (the result had ended): AfterKill reads to the end all the same.
    assert_eq!(
        c.close_query(CloseHow::AfterKill(Duration::from_secs(10)))
            .await
            .unwrap(),
        Closed::Closed { rows: 1 }
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
