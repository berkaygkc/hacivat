//! A scripted in-process MySQL server for the driver's wire tests (`typed_wire.rs`) and the
//! `mysql_async` patch tests (`patches.rs`): just enough of the protocol (greeting, any sign-in,
//! the connection's setup queries, prepare/execute/query replies), every command recorded, and
//! a few hooks a test needs: a `Wait` step the test releases, a `Hangup` step, and whether the
//! server saw the client close the socket or a write fail (a client that never reads the rest).

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use hacivat_driver::{
    DriverConnect, Endpoint, InsecureAuth, Protocol, QueryDone, QueryEvent, QuerySpec, RawConn,
    RawError, RawErrorKind, ResultEnd,
};
use hacivat_driver_mysql::{MySqlConn, connect_conn};
use hacivat_net::{Target, direct};
use hacivat_values::{Chunk, ValueRef};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

// Protocol constants -----------------------------------------------------------------------

pub const COM_QUIT: u8 = 0x01;
pub const COM_QUERY: u8 = 0x03;
pub const COM_PING: u8 = 0x0E;
pub const COM_STMT_PREPARE: u8 = 0x16;
pub const COM_STMT_EXECUTE: u8 = 0x17;
pub const COM_STMT_CLOSE: u8 = 0x19;
pub const COM_RESET_CONNECTION: u8 = 0x1F;

pub const T_LONG: u8 = 3;
pub const T_DOUBLE: u8 = 5;
pub const T_LONGLONG: u8 = 8;
pub const T_DATE: u8 = 10;
pub const T_DATETIME: u8 = 12;
pub const T_TIME2: u8 = 19;
pub const T_VAR_STRING: u8 = 253;

pub const AUTOCOMMIT: u16 = 0x0002;
pub const IN_TRANS: u16 = 0x0001;
pub const MORE_RESULTS: u16 = 0x0008;

pub const BINARY: u16 = 63;
pub const UTF8MB4: u16 = 255;
/// The connection ID the greeting announces.
pub const THREAD_ID: u32 = 7;

// Packet builders --------------------------------------------------------------------------

pub fn lenenc_int(n: u64) -> Vec<u8> {
    match n {
        0..=250 => vec![n as u8],
        251..=0xFFFF => [&[0xFC][..], &(n as u16).to_le_bytes()].concat(),
        0x1_0000..=0xFF_FFFF => [&[0xFD][..], &(n as u32).to_le_bytes()[..3]].concat(),
        _ => [&[0xFE][..], &n.to_le_bytes()].concat(),
    }
}

pub fn lenenc_str(b: &[u8]) -> Vec<u8> {
    [lenenc_int(b.len() as u64), b.to_vec()].concat()
}

pub fn ok_packet(affected: u64, status: u16, warnings: u16) -> Vec<u8> {
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
pub fn terminator(status: u16, warnings: u16) -> Vec<u8> {
    let mut p = ok_packet(0, status, warnings);
    p[0] = 0xFE;
    p
}

pub fn err_packet(code: u16, state: &str, message: &str) -> Vec<u8> {
    [
        vec![0xFF],
        code.to_le_bytes().to_vec(),
        b"#".to_vec(),
        state.as_bytes().to_vec(),
        message.as_bytes().to_vec(),
    ]
    .concat()
}

pub fn column_def(
    name: &str,
    ty: u8,
    flags: u16,
    charset: u16,
    length: u32,
    decimals: u8,
) -> Vec<u8> {
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

pub fn long_col(name: &str) -> Vec<u8> {
    column_def(name, T_LONG, 0, BINARY, 11, 0)
}

pub fn text_col(name: &str) -> Vec<u8> {
    column_def(name, T_VAR_STRING, 0, UTF8MB4, 65_535, 0)
}

pub fn text_row(cells: &[Option<&[u8]>]) -> Vec<u8> {
    cells
        .iter()
        .flat_map(|c| match c {
            Some(b) => lenenc_str(b),
            None => vec![0xFB],
        })
        .collect()
}

/// A binary row: the header, the NULL bitmap (offset 2) and the non-NULL values as encoded.
pub fn bin_row(cells: &[Option<Vec<u8>>]) -> Vec<u8> {
    let mut bitmap = vec![0_u8; (cells.len() + 7 + 2) / 8];
    for (i, c) in cells.iter().enumerate() {
        if c.is_none() {
            bitmap[(i + 2) / 8] |= 1 << ((i + 2) % 8);
        }
    }
    let values: Vec<u8> = cells.iter().flatten().flatten().copied().collect();
    [vec![0x00], bitmap, values].concat()
}

pub fn long(v: i32) -> Option<Vec<u8>> {
    Some(v.to_le_bytes().to_vec())
}

/// The prepare reply for a statement with `columns` and `params` placeholders.
pub fn prepare_ok(columns: &[Vec<u8>], params: u16) -> Vec<Step> {
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
pub fn header(columns: &[Vec<u8>]) -> Vec<Step> {
    let mut steps = vec![Step::Packet(lenenc_int(columns.len() as u64))];
    steps.extend(columns.iter().cloned().map(Step::Packet));
    steps
}

pub fn packets(ps: Vec<Vec<u8>>) -> Vec<Step> {
    ps.into_iter().map(Step::Packet).collect()
}

pub fn int_rows(values: &[i32]) -> Vec<Step> {
    values
        .iter()
        .map(|v| Step::Packet(bin_row(&[long(*v)])))
        .collect()
}

/// Bytes past every socket buffer on the way (`WIDE_ROWS` rows of `WIDE_BYTES`): a client that
/// abandons the connection makes the server's writes of them fail.
pub const WIDE_ROWS: usize = 4_000;
pub const WIDE_BYTES: usize = 4_096;

/// `n` valid one-column binary rows of `WIDE_BYTES` of text each.
pub fn wide_bin_rows(n: usize) -> Vec<Step> {
    let cell = lenenc_str(&[b'x'; WIDE_BYTES]);
    (0..n)
        .map(|_| Step::Packet(bin_row(&[Some(cell.clone())])))
        .collect()
}

/// `n` valid one-column text rows of `WIDE_BYTES` each.
pub fn wide_text_rows(n: usize) -> Vec<Step> {
    let cell = [b'x'; WIDE_BYTES];
    (0..n)
        .map(|_| Step::Packet(text_row(&[Some(&cell)])))
        .collect()
}

// The scripted server ----------------------------------------------------------------------

/// One step of a reply: a packet, a wait until the test releases the rest, or hanging up.
#[derive(Clone)]
pub enum Step {
    Packet(Vec<u8>),
    Wait(Arc<Notify>),
    Hangup,
}

#[derive(Debug, Clone)]
pub struct Cmd {
    pub code: u8,
    pub body: Vec<u8>,
}

impl Cmd {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// The test's replies; `None` falls back to [`default_reply`].
pub type Handler = Arc<dyn Fn(&Cmd) -> Option<Vec<Step>> + Send + Sync>;

pub struct Server {
    pub addr: SocketAddr,
    log: Arc<Mutex<Vec<Cmd>>>,
    ended: Arc<AtomicBool>,
    write_failed: Arc<AtomicBool>,
}

impl Server {
    /// The commands received, in order.
    pub fn commands(&self) -> Vec<Cmd> {
        self.log.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.log.lock().unwrap().len()
    }

    /// The connection is over: the client closed it (or the server hung up).
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }

    /// A write failed: the client had closed the socket without reading what was sent.
    pub fn write_failed(&self) -> bool {
        self.write_failed.load(Ordering::SeqCst)
    }

    pub fn quit_seen(&self) -> bool {
        self.commands().iter().any(|c| c.code == COM_QUIT)
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
pub fn variables(cells: &[(&str, &str)]) -> Vec<Step> {
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
pub fn default_reply(cmd: &Cmd) -> Vec<Step> {
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
pub async fn serve(handler: Handler) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let ended = Arc::new(AtomicBool::new(false));
    let write_failed = Arc::new(AtomicBool::new(false));
    let (seen, over, failed) = (log.clone(), ended.clone(), write_failed.clone());
    tokio::spawn(async move {
        let Ok((mut s, _)) = listener.accept().await else {
            return;
        };
        serve_one(&mut s, handler, &seen, &failed).await;
        over.store(true, Ordering::SeqCst);
    });
    Server {
        addr,
        log,
        ended,
        write_failed,
    }
}

async fn serve_one(
    s: &mut TcpStream,
    handler: Handler,
    seen: &Mutex<Vec<Cmd>>,
    failed: &AtomicBool,
) {
    let write = |ok: bool| {
        if !ok {
            failed.store(true, Ordering::SeqCst);
        }
        ok
    };
    if !write(write_packet(s, 0, &greeting()).await) {
        return;
    }
    let Some((seq, _response)) = read_packet(s).await else {
        return;
    };
    if !write(write_packet(s, seq.wrapping_add(1), &ok_packet(0, AUTOCOMMIT, 0)).await) {
        return;
    }
    while let Some((_, body)) = read_packet(s).await {
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
                    if !write(write_packet(s, seq, &p).await) {
                        return;
                    }
                    seq = seq.wrapping_add(1);
                }
                Step::Wait(n) => n.notified().await,
                Step::Hangup => return,
            }
        }
    }
}

/// Waits (a condition, polled) until `done`, at most `limit`; `false` if it never held.
pub async fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Every panic message since the first call, recorded by a hook that also runs the default one.
pub fn record_panics() -> &'static Mutex<Vec<String>> {
    static PANICS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    static HOOK: OnceLock<()> = OnceLock::new();
    let panics = PANICS.get_or_init(|| Mutex::new(Vec::new()));
    HOOK.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let message = info
                .payload()
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| info.payload().downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            if let Some(p) = PANICS.get() {
                p.lock().unwrap_or_else(|e| e.into_inner()).push(message);
            }
            previous(info);
        }));
    });
    panics
}

// The driver over it -----------------------------------------------------------------------

pub fn request(addr: SocketAddr, read_only: bool) -> DriverConnect {
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
pub async fn scripted(
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

pub fn spec(sql: &str, suspendable: bool, first_rows: u32) -> QuerySpec<'_> {
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

pub fn kind<T>(r: Result<T, RawError>) -> RawErrorKind {
    match r {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.kind,
    }
}

pub fn vendor(e: &RawError) -> Option<i64> {
    e.server.as_ref().and_then(|s| s.vendor_code)
}

pub async fn event(c: &mut MySqlConn) -> QueryEvent {
    match c.next_event().await {
        Ok(e) => e,
        Err(e) => panic!("next_event: {:?}", e.kind),
    }
}

pub fn rows_of(e: QueryEvent) -> Chunk {
    match e {
        QueryEvent::Rows(c) => c,
        other => panic!("expected rows, got {other:?}"),
    }
}

pub fn int_cells(c: &Chunk, col: u32) -> Vec<i64> {
    (0..c.rows())
        .map(|r| match c.cell(r, col) {
            ValueRef::Int(v) => v,
            other => panic!("{other:?}"),
        })
        .collect()
}

pub fn end_of(e: QueryEvent) -> ResultEnd {
    match e {
        QueryEvent::ResultEnd(end) => end,
        other => panic!("expected the result's end, got {other:?}"),
    }
}

pub fn done_of(e: QueryEvent) -> QueryDone {
    match e {
        QueryEvent::Done(done) => done,
        other => panic!("expected done, got {other:?}"),
    }
}

/// A prepared statement of one column (`col`) whose execution answers `rows`.
pub fn one_column(
    col: Vec<u8>,
    rows: Vec<Step>,
) -> impl Fn(&Cmd) -> Option<Vec<Step>> + Send + Sync + 'static {
    let rows = Arc::new(Mutex::new(Some(rows)));
    move |cmd: &Cmd| match cmd.code {
        COM_STMT_PREPARE => Some(prepare_ok(std::slice::from_ref(&col), 0)),
        COM_STMT_EXECUTE => {
            let mut steps = header(std::slice::from_ref(&col));
            steps.extend(rows.lock().unwrap().take().unwrap_or_default());
            Some(steps)
        }
        _ => None,
    }
}

/// A prepared statement of one `INT` column `a` whose execution answers `rows`.
pub fn one_int_column(
    rows: Vec<Step>,
) -> impl Fn(&Cmd) -> Option<Vec<Step>> + Send + Sync + 'static {
    one_column(long_col("a"), rows)
}
