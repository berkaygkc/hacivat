//! The vendored `tokio-postgres` patch §1 and the typed path over it, against a scripted
//! in-memory PostgreSQL server (Task 12; R-M3-18, P5; the pattern of `tests/wire.rs`). No
//! Docker. The server half reads every frontend message the driver writes, so each test checks
//! the exact wire sequence, and answers every `Sync` so the observer's count stays exact.
//!
//! Hostile-server cases (Review Focus 1, the Task 12 review probes): a `DataRow` with the wrong
//! column count, negative and oversized field lengths, a server error in the middle of a
//! stream.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use futures::TryStreamExt;
use hacivat_driver::{
    CloseHow, Closed, DriverConnect, Endpoint, ExecOpts, InsecureAuth, Protocol, QueryEvent,
    QuerySpec, RawConn, RawErrorKind, TxStatus,
};
use hacivat_driver_pg::{PgConn, connect_conn};
use hacivat_net::{BoxedStream, StreamFactory};
use hacivat_values::{BindValue, LogicalType, OriginHint, Value};
use postgres_types::{ToSql, Type};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------------------------
// Backend messages.

fn frame(typ: u8, body: &[u8]) -> Vec<u8> {
    let mut m = vec![typ];
    m.extend_from_slice(&u32::try_from(body.len() + 4).unwrap().to_be_bytes());
    m.extend_from_slice(body);
    m
}

fn ready(status: u8) -> Vec<u8> {
    frame(b'Z', &[status])
}

fn command_complete(tag: &str) -> Vec<u8> {
    frame(b'C', format!("{tag}\0").as_bytes())
}

fn error(code: &str) -> Vec<u8> {
    let mut body = vec![];
    for (f, v) in [
        (b'S', "ERROR"),
        (b'V', "ERROR"),
        (b'C', code),
        (b'M', "scripted"),
    ] {
        body.push(f);
        body.extend_from_slice(v.as_bytes());
        body.push(0);
    }
    body.push(0);
    frame(b'E', &body)
}

/// One RowDescription field: name, type OID, table OID and attribute number (0 for none).
#[derive(Clone, Copy)]
struct Field {
    name: &'static str,
    oid: u32,
    table: u32,
    attnum: i16,
}

fn field(name: &'static str, oid: u32) -> Field {
    Field {
        name,
        oid,
        table: 0,
        attnum: 0,
    }
}

fn row_description(fields: &[Field]) -> Vec<u8> {
    let mut body = i16::try_from(fields.len()).unwrap().to_be_bytes().to_vec();
    for f in fields {
        body.extend_from_slice(f.name.as_bytes());
        body.push(0);
        body.extend_from_slice(&f.table.to_be_bytes());
        body.extend_from_slice(&f.attnum.to_be_bytes());
        body.extend_from_slice(&f.oid.to_be_bytes());
        body.extend_from_slice(&(-1i16).to_be_bytes());
        body.extend_from_slice(&(-1i32).to_be_bytes());
        body.extend_from_slice(&0i16.to_be_bytes());
    }
    frame(b'T', &body)
}

/// A DataRow whose field count is the number of cells.
fn data_row(cells: &[Option<&[u8]>]) -> Vec<u8> {
    data_row_counted(i16::try_from(cells.len()).unwrap(), cells)
}

/// A DataRow with any field count (hostile servers).
fn data_row_counted(count: i16, cells: &[Option<&[u8]>]) -> Vec<u8> {
    let mut body = count.to_be_bytes().to_vec();
    for cell in cells {
        match cell {
            Some(v) => {
                body.extend_from_slice(&i32::try_from(v.len()).unwrap().to_be_bytes());
                body.extend_from_slice(v);
            }
            None => body.extend_from_slice(&(-1i32).to_be_bytes()),
        }
    }
    frame(b'D', &body)
}

/// An int4 and a tsvector (text format) cell.
fn int_and_text_row(i: i32, text: &str) -> Vec<u8> {
    data_row(&[Some(&i.to_be_bytes()), Some(text.as_bytes())])
}

const INT4: u32 = 23;
const TS_VECTOR: u32 = 3614;

// ---------------------------------------------------------------------------------------------
// Frontend messages, as the server reads them.

#[derive(Debug, Clone, PartialEq, Eq)]
struct Bind {
    portal: String,
    statement: String,
    param_formats: Vec<i16>,
    params: usize,
    result_formats: Vec<i16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Execute {
    portal: String,
    max_rows: i32,
}

/// What the server read, by message, for sequence asserts.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Msg {
    Parse { name: String, query: String },
    Describe(u8, String),
    Bind(Bind),
    Execute(Execute),
    Close(u8, String),
    Flush,
    Sync,
    Query(String),
    Other(u8),
}

fn cstr(body: &[u8], at: &mut usize) -> String {
    let end = body[*at..].iter().position(|b| *b == 0).unwrap() + *at;
    let s = String::from_utf8(body[*at..end].to_vec()).unwrap();
    *at = end + 1;
    s
}

fn i16_at(body: &[u8], at: &mut usize) -> i16 {
    let v = i16::from_be_bytes([body[*at], body[*at + 1]]);
    *at += 2;
    v
}

fn i32_at(body: &[u8], at: &mut usize) -> i32 {
    let v = i32::from_be_bytes(body[*at..*at + 4].try_into().unwrap());
    *at += 4;
    v
}

fn parse_msg(typ: u8, body: &[u8]) -> Msg {
    let mut at = 0;
    match typ {
        b'P' => {
            let name = cstr(body, &mut at);
            let query = cstr(body, &mut at);
            Msg::Parse { name, query }
        }
        b'D' => {
            let kind = body[0];
            at = 1;
            Msg::Describe(kind, cstr(body, &mut at))
        }
        b'B' => {
            let portal = cstr(body, &mut at);
            let statement = cstr(body, &mut at);
            let n = i16_at(body, &mut at);
            let param_formats = (0..n).map(|_| i16_at(body, &mut at)).collect();
            let params = i16_at(body, &mut at);
            for _ in 0..params {
                let len = i32_at(body, &mut at);
                if len > 0 {
                    at += len as usize;
                }
            }
            let n = i16_at(body, &mut at);
            let result_formats = (0..n).map(|_| i16_at(body, &mut at)).collect();
            Msg::Bind(Bind {
                portal,
                statement,
                param_formats,
                params: params as usize,
                result_formats,
            })
        }
        b'E' => {
            let portal = cstr(body, &mut at);
            Msg::Execute(Execute {
                portal,
                max_rows: i32_at(body, &mut at),
            })
        }
        b'C' => {
            let kind = body[0];
            at = 1;
            Msg::Close(kind, cstr(body, &mut at))
        }
        b'S' => Msg::Sync,
        b'H' => Msg::Flush,
        b'Q' => Msg::Query(String::from_utf8(body[..body.len() - 1].to_vec()).unwrap()),
        other => Msg::Other(other),
    }
}

/// The scripted server's end of the pipe.
struct Server {
    s: DuplexStream,
}

/// A bound on every read, so a driver that never writes fails the test instead of hanging it.
const READ_GUARD: Duration = Duration::from_secs(20);

impl Server {
    async fn raw(&mut self) -> (u8, Vec<u8>) {
        tokio::time::timeout(READ_GUARD, async {
            let typ = self.s.read_u8().await.unwrap();
            let len = self.s.read_u32().await.unwrap() as usize;
            let mut body = vec![0; len - 4];
            self.s.read_exact(&mut body).await.unwrap();
            (typ, body)
        })
        .await
        .expect("the client sent the next message")
    }

    async fn msg(&mut self) -> Msg {
        let (typ, body) = self.raw().await;
        parse_msg(typ, &body)
    }

    async fn send(&mut self, bytes: &[u8]) {
        self.s.write_all(bytes).await.unwrap();
    }

    /// Answers a `prepare` (Parse, Describe, Sync) with `fields` (NoData when empty) and
    /// `status`; returns the statement name and the query.
    async fn prepare(&mut self, fields: &[Field], status: u8) -> (String, String) {
        let Msg::Parse { name, query } = self.msg().await else {
            panic!("expected Parse");
        };
        assert_eq!(self.msg().await, Msg::Describe(b'S', name.clone()));
        assert_eq!(self.msg().await, Msg::Sync);
        let description = if fields.is_empty() {
            frame(b'n', &[])
        } else {
            row_description(fields)
        };
        self.send(
            &[
                frame(b'1', &[]),
                frame(b't', &0i16.to_be_bytes()),
                description,
                ready(status),
            ]
            .concat(),
        )
        .await;
        (name, query)
    }

    async fn bind(&mut self) -> Bind {
        match self.msg().await {
            Msg::Bind(b) => b,
            other => panic!("expected Bind, got {other:?}"),
        }
    }

    async fn execute(&mut self) -> Execute {
        match self.msg().await {
            Msg::Execute(e) => e,
            other => panic!("expected Execute, got {other:?}"),
        }
    }

    async fn sync(&mut self) {
        assert_eq!(self.msg().await, Msg::Sync);
    }

    /// Reads a non-suspendable unit's one batch (Bind into the unnamed portal, Flush,
    /// `Execute(0)`, Sync) and returns its Bind.
    async fn unnamed(&mut self) -> Bind {
        let b = self.bind().await;
        assert_eq!(b.portal, "", "the unnamed portal");
        assert_eq!(self.msg().await, Msg::Flush);
        assert_eq!(
            self.execute().await,
            Execute {
                portal: String::new(),
                max_rows: 0
            }
        );
        self.sync().await;
        b
    }

    /// Answers a `Close` + `Sync` of `kind` (`P` or `S`) and returns the name closed.
    async fn close(&mut self, kind: u8, status: u8) -> String {
        let name = match self.msg().await {
            Msg::Close(k, name) if k == kind => name,
            other => panic!("expected Close {}, got {other:?}", kind as char),
        };
        self.sync().await;
        self.send(&[frame(b'3', &[]), ready(status)].concat()).await;
        name
    }

    /// Answers a simple `Query` with its command tag and `status`.
    async fn query(&mut self, tag: &str, status: u8) -> String {
        let Msg::Query(q) = self.msg().await else {
            panic!("expected Query");
        };
        let reply = if q.is_empty() {
            frame(b'I', &[])
        } else {
            command_complete(tag)
        };
        self.send(&[reply, ready(status)].concat()).await;
        q
    }
}

// ---------------------------------------------------------------------------------------------
// Sessions.

fn pipes() -> (StreamFactory, mpsc::UnboundedReceiver<DuplexStream>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let factory = StreamFactory::new(move || {
        let (client, server) = tokio::io::duplex(1 << 20);
        let sent = tx.send(server);
        Box::pin(async move {
            sent.map_err(|_| io::Error::from(io::ErrorKind::ConnectionRefused))?;
            Ok(Box::new(client) as BoxedStream)
        })
    });
    (factory, rx)
}

fn req(factory: StreamFactory) -> DriverConnect {
    DriverConnect {
        endpoint: Endpoint::Network {
            factory,
            direct: None,
            resolved: vec![],
            tls_host: "localhost".into(),
        },
        tls: None,
        user: Some("hv_test".into()),
        password: Some(SecretString::from("s3cret")),
        database: Some("hv_test".into()),
        application_name: "Hacivat".into(),
        read_only: false,
        options: vec![],
        connect_timeout: Duration::from_secs(5),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::Allow,
    }
}

/// A plaintext session with a scripted server, signed in without a password, status Idle.
async fn session() -> (PgConn, Server) {
    let (factory, mut streams) = pipes();
    let server = tokio::spawn(async move {
        let mut s = streams.recv().await.unwrap();
        let len = s.read_u32().await.unwrap() as usize;
        let mut startup = vec![0; len - 4];
        s.read_exact(&mut startup).await.unwrap();
        let mut key = 31i32.to_be_bytes().to_vec();
        key.extend_from_slice(&1i32.to_be_bytes());
        s.write_all(
            &[
                frame(b'R', &0i32.to_be_bytes()),
                frame(b'K', &key),
                ready(b'I'),
            ]
            .concat(),
        )
        .await
        .unwrap();
        Server { s }
    });
    let conn = connect_conn(req(factory)).await.unwrap();
    (conn, server.await.unwrap())
}

/// A session inside a transaction block (the client ran `BEGIN`).
async fn session_in_tx() -> (PgConn, Server) {
    let (mut conn, mut srv) = session().await;
    let (r, q) = tokio::join!(conn.batch("BEGIN"), srv.query("BEGIN", b'T'));
    r.unwrap();
    assert_eq!(q, "BEGIN");
    assert_eq!(conn.tx_status(), TxStatus::InTx);
    (conn, srv)
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

fn no_params() -> std::iter::Empty<&'static (dyn ToSql + Sync)> {
    std::iter::empty()
}

/// The rows of a `Rows` event as (int4, opaque text) pairs.
fn pairs(event: &QueryEvent) -> Vec<(i64, String)> {
    let QueryEvent::Rows(chunk) = event else {
        panic!("expected Rows, got {event:?}");
    };
    (0..chunk.rows())
        .map(|r| {
            let Value::Int(i) = chunk.cell(r, 0).to_value() else {
                panic!("int4 cell");
            };
            let Value::Opaque(o) = chunk.cell(r, 1).to_value() else {
                panic!("opaque cell");
            };
            (i, o.text.to_string())
        })
        .collect()
}

async fn next(conn: &mut PgConn) -> QueryEvent {
    conn.next_event().await.unwrap()
}

// ---------------------------------------------------------------------------------------------
// Patch §1: the three entry points.

#[tokio::test]
async fn bind_with_formats_sends_per_column_codes() {
    let (conn, mut srv) = session().await;
    let fields = [field("a", INT4), field("b", TS_VECTOR)];
    let (stmt, (name, _)) = tokio::join!(
        conn.client().prepare("SELECT a, b FROM t"),
        srv.prepare(&fields, b'T')
    );
    let stmt = stmt.unwrap();
    let server = tokio::spawn(async move {
        let b = srv.bind().await;
        srv.sync().await;
        srv.send(&[frame(b'2', &[]), ready(b'T')].concat()).await;
        (b, srv)
    });
    let portal = conn
        .client()
        .bind_with_formats(&stmt, no_params(), &[1, 0])
        .await
        .unwrap();
    let (b, _srv) = server.await.unwrap();
    assert_eq!(b.result_formats, [1, 0]);
    assert_eq!(b.statement, name);
    assert!(b.portal.starts_with('p'), "a named portal: {b:?}");
    assert!(b.param_formats.is_empty() && b.params == 0);
    drop(portal);
}

/// Binds a portal over `SELECT a, b` and returns the client's portal and the server.
async fn bound_portal(
    conn: &PgConn,
    mut srv: Server,
) -> (
    tokio_postgres::Portal,
    tokio_postgres::Statement,
    String,
    Server,
) {
    let fields = [field("a", INT4), field("b", TS_VECTOR)];
    let (stmt, _) = tokio::join!(
        conn.client().prepare("SELECT a, b FROM t"),
        srv.prepare(&fields, b'T')
    );
    let stmt = stmt.unwrap();
    let server = tokio::spawn(async move {
        let b = srv.bind().await;
        srv.sync().await;
        srv.send(&[frame(b'2', &[]), ready(b'T')].concat()).await;
        (b.portal, srv)
    });
    let portal = conn
        .client()
        .bind_with_formats(&stmt, no_params(), &[1, 0])
        .await
        .unwrap();
    let (name, srv) = server.await.unwrap();
    (portal, stmt, name, srv)
}

#[tokio::test]
async fn query_portal_raw_sends_execute_max_rows() {
    let (conn, srv) = session().await;
    let (portal, _stmt, name, mut srv) = bound_portal(&conn, srv).await;
    let server = tokio::spawn(async move {
        let e = srv.execute().await;
        srv.sync().await;
        srv.send(&[frame(b's', &[]), ready(b'T')].concat()).await;
        e
    });
    let stream = conn.client().query_portal_raw(&portal, 2).await.unwrap();
    let mut stream = std::pin::pin!(stream);
    assert!(stream.try_next().await.unwrap().is_none());
    assert_eq!(
        server.await.unwrap(),
        Execute {
            portal: name,
            max_rows: 2
        }
    );
}

#[tokio::test]
async fn suspended_portal_leaves_rows_affected_none() {
    let (conn, srv) = session().await;
    let (portal, _stmt, _name, mut srv) = bound_portal(&conn, srv).await;
    let server = tokio::spawn(async move {
        for max in [2, 5] {
            assert_eq!(srv.execute().await.max_rows, max);
            srv.sync().await;
            let mut script = vec![];
            if max == 2 {
                script.extend(int_and_text_row(1, "'a'"));
                script.extend(int_and_text_row(2, "'b'"));
                script.extend(frame(b's', &[]));
            } else {
                script.extend(int_and_text_row(3, "'c'"));
                script.extend(command_complete("SELECT 1"));
            }
            script.extend(ready(b'T'));
            srv.send(&script).await;
        }
        srv
    });
    let stream = conn.client().query_portal_raw(&portal, 2).await.unwrap();
    let mut stream = std::pin::pin!(stream);
    let mut n = 0;
    while stream.try_next().await.unwrap().is_some() {
        n += 1;
    }
    assert_eq!(n, 2);
    assert_eq!(
        stream.rows_affected(),
        None,
        "PortalSuspended: no count yet"
    );
    let stream = conn.client().query_portal_raw(&portal, 5).await.unwrap();
    let mut stream = std::pin::pin!(stream);
    while stream.try_next().await.unwrap().is_some() {}
    assert_eq!(stream.rows_affected(), Some(1), "CommandComplete counts");
    server.await.unwrap();
}

#[tokio::test]
async fn query_with_formats_sends_bind_flush_execute_and_sync_in_one_batch() {
    let (conn, mut srv) = session().await;
    let fields = [field("a", INT4), field("b", TS_VECTOR)];
    let (stmt, (name, _)) = tokio::join!(
        conn.client().prepare("SELECT a, b FROM t"),
        srv.prepare(&fields, b'I')
    );
    let stmt = stmt.unwrap();
    let (rows_tx, rows_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        // All four arrive before the server answers anything.
        let seq = [
            srv.msg().await,
            srv.msg().await,
            srv.msg().await,
            srv.msg().await,
        ];
        // The Flush makes a server deliver BindComplete before the statement's rows: the
        // client is done once it has it.
        srv.send(&frame(b'2', &[])).await;
        rows_rx.await.unwrap();
        srv.send(
            &[
                int_and_text_row(7, "'x'"),
                command_complete("SELECT 1"),
                ready(b'I'),
            ]
            .concat(),
        )
        .await;
        seq
    });
    let stream = conn
        .client()
        .query_with_formats(&stmt, no_params(), &[1, 0])
        .await
        .unwrap();
    rows_tx.send(()).unwrap();
    let rows: Vec<tokio_postgres::Row> = stream.try_collect().await.unwrap();
    assert_eq!(rows.len(), 1);
    let seq = server.await.unwrap();
    let Msg::Bind(b) = &seq[0] else {
        panic!("{seq:?}");
    };
    assert_eq!(b.portal, "", "the unnamed portal");
    assert_eq!(b.statement, name);
    assert_eq!(b.result_formats, [1, 0]);
    assert_eq!(
        seq[1..],
        [
            Msg::Flush,
            Msg::Execute(Execute {
                portal: String::new(),
                max_rows: 0
            }),
            Msg::Sync
        ]
    );
}

/// The bytes `tokio-postgres` 0.7.18 writes for `query_raw` of a prepared statement with no
/// parameters (`query::encode`: Bind with every result column binary, `Execute("", 0)`,
/// `Sync`), built from the protocol, never from the library.
fn golden_0_7_18_query_raw(statement: &str) -> Vec<u8> {
    let mut bind = vec![0u8]; // the unnamed portal
    bind.extend_from_slice(statement.as_bytes());
    bind.push(0);
    bind.extend_from_slice(&0i16.to_be_bytes()); // parameter format codes
    bind.extend_from_slice(&0i16.to_be_bytes()); // parameters
    bind.extend_from_slice(&1i16.to_be_bytes()); // one result format code ...
    bind.extend_from_slice(&1i16.to_be_bytes()); // ... binary
    let mut execute = vec![0u8];
    execute.extend_from_slice(&0i32.to_be_bytes());
    [frame(b'B', &bind), frame(b'E', &execute), frame(b'S', &[])].concat()
}

/// The bytes 0.7.18 writes for `query_typed_raw("SELECT 1", [])`: Parse, Bind (binary
/// results), Describe of the unnamed statement, `Execute("", 0)`, `Sync`.
fn golden_0_7_18_query_typed_raw() -> Vec<u8> {
    let mut parse = vec![0u8];
    parse.extend_from_slice(b"SELECT 1\0");
    parse.extend_from_slice(&0i16.to_be_bytes());
    let mut bind = vec![0u8, 0u8];
    bind.extend_from_slice(&0i16.to_be_bytes());
    bind.extend_from_slice(&0i16.to_be_bytes());
    bind.extend_from_slice(&1i16.to_be_bytes());
    bind.extend_from_slice(&1i16.to_be_bytes());
    let mut execute = vec![0u8];
    execute.extend_from_slice(&0i32.to_be_bytes());
    [
        frame(b'P', &parse),
        frame(b'B', &bind),
        frame(b'D', b"S\0"),
        frame(b'E', &execute),
        frame(b'S', &[]),
    ]
    .concat()
}

/// Reads `n` raw frames and returns their bytes, as written.
async fn raw_frames(srv: &mut Server, n: usize) -> Vec<u8> {
    let mut out = vec![];
    for _ in 0..n {
        let (typ, body) = srv.raw().await;
        out.extend(frame(typ, &body));
    }
    out
}

#[tokio::test]
async fn existing_query_path_is_byte_identical() {
    let (conn, mut srv) = session().await;
    let fields = [field("a", INT4)];
    let (stmt, (name, _)) = tokio::join!(
        conn.client().prepare("SELECT a FROM t"),
        srv.prepare(&fields, b'I')
    );
    let stmt = stmt.unwrap();
    let server = tokio::spawn(async move {
        let wire = raw_frames(&mut srv, 3).await;
        srv.send(&[frame(b'2', &[]), command_complete("SELECT 0"), ready(b'I')].concat())
            .await;
        let typed = raw_frames(&mut srv, 5).await;
        srv.send(
            &[
                frame(b'1', &[]),
                frame(b'2', &[]),
                frame(b'n', &[]),
                command_complete("SELECT 1"),
                ready(b'I'),
            ]
            .concat(),
        )
        .await;
        (wire, typed)
    });
    let stream = conn.client().query_raw(&stmt, no_params()).await.unwrap();
    let _: Vec<tokio_postgres::Row> = stream.try_collect().await.unwrap();
    let stream = conn
        .client()
        .query_typed_raw(
            "SELECT 1",
            std::iter::empty::<(&(dyn ToSql + Sync), Type)>(),
        )
        .await
        .unwrap();
    let _: Vec<tokio_postgres::Row> = stream.try_collect().await.unwrap();
    let (wire, typed) = server.await.unwrap();
    assert_eq!(wire, golden_0_7_18_query_raw(&name));
    assert_eq!(typed, golden_0_7_18_query_typed_raw());
}

// ---------------------------------------------------------------------------------------------
// The typed path (`RawConn::query` and friends) over the patch.

#[tokio::test]
async fn autocommit_non_suspendable_unit_runs_in_one_batch() {
    let (mut conn, mut srv) = session().await;
    let fields = [
        Field {
            name: "id",
            oid: INT4,
            table: 16_384,
            attnum: 2,
        },
        field("doc", TS_VECTOR),
    ];
    let server = tokio::spawn(async move {
        let (name, query) = srv.prepare(&fields, b'I').await;
        assert_eq!(query, "SELECT id, doc FROM t");
        let seq = [
            srv.msg().await,
            srv.msg().await,
            srv.msg().await,
            srv.msg().await,
        ];
        srv.send(
            &[
                frame(b'2', &[]),
                int_and_text_row(1, "'a'"),
                data_row(&[None, Some(b"'b'")]),
                command_complete("SELECT 2"),
                ready(b'I'),
            ]
            .concat(),
        )
        .await;
        // The statement is closed after the query.
        assert_eq!(srv.close(b'S', b'I').await, name);
        (name, seq, srv)
    });
    conn.query(spec("SELECT id, doc FROM t", false, 501))
        .await
        .unwrap();
    let QueryEvent::ResultStart(schema) = next(&mut conn).await else {
        panic!("ResultStart first");
    };
    assert_eq!(schema.columns.len(), 2);
    assert_eq!(&*schema.columns[0].name, "id");
    assert_eq!(
        schema.columns[0].logical,
        LogicalType::Int {
            bits: 32,
            signed: true
        }
    );
    assert_eq!(
        schema.columns[0].origin,
        Some(OriginHint::Pg {
            table_oid: 16_384,
            attnum: 2
        })
    );
    assert_eq!(schema.columns[1].origin, None);
    assert!(matches!(
        schema.columns[1].logical,
        LogicalType::Opaque { ref type_name } if &**type_name == "tsvector"
    ));
    let QueryEvent::Rows(chunk) = next(&mut conn).await else {
        panic!("Rows");
    };
    assert_eq!(chunk.rows(), 2);
    assert_eq!(chunk.cell(0, 0).to_value(), Value::Int(1));
    assert_eq!(chunk.cell(1, 0).to_value(), Value::Null);
    let QueryEvent::ResultEnd(end) = next(&mut conn).await else {
        panic!("ResultEnd");
    };
    // A read affects nothing: its count is `rows` (fix round 1, review I3).
    assert_eq!((end.rows, end.kept, end.rows_affected), (2, 2, None));
    assert_eq!(end.command_tag.as_deref(), Some("SELECT 2"));
    let QueryEvent::Done(done) = next(&mut conn).await else {
        panic!("Done");
    };
    assert_eq!(done.tx, TxStatus::Idle);
    assert_eq!(done.skipped_result_sets, 0);
    let (name, seq, _srv) = server.await.unwrap();
    let Msg::Bind(b) = &seq[0] else {
        panic!("{seq:?}");
    };
    assert_eq!(
        (b.portal.as_str(), b.statement.as_str()),
        ("", name.as_str())
    );
    assert_eq!(b.result_formats, [1, 0], "binary int4, text tsvector");
    assert_eq!(
        seq[1..],
        [
            Msg::Flush,
            Msg::Execute(Execute {
                portal: String::new(),
                max_rows: 0
            }),
            Msg::Sync
        ]
    );
    // P5: a non-suspendable unit leaves nothing to resume.
    assert_eq!(
        conn.next_event().await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
}

/// After the refusal the client pings; the ping is the first message the server reads, so
/// nothing was sent before it.
async fn nothing_was_sent(conn: &mut PgConn, srv: &mut Server) {
    let (ping, q) = tokio::join!(conn.ping(), srv.query("", b'I'));
    ping.unwrap();
    assert_eq!(q, "", "the ping is the first message after the refusal");
}

#[tokio::test]
async fn suspendable_outside_a_transaction_is_refused() {
    let (mut conn, mut srv) = session().await;
    assert_eq!(conn.tx_status(), TxStatus::Idle);
    let r = conn.query(spec("SELECT a FROM t", true, 501)).await;
    assert_eq!(r.unwrap_err().kind, RawErrorKind::Unsupported);
    nothing_was_sent(&mut conn, &mut srv).await;
}

#[tokio::test]
async fn copy_is_refused_before_sending() {
    let (mut conn, mut srv) = session().await;
    for sql in [
        "COPY t TO STDOUT",
        "copy t from stdin",
        "  -- export\n /* a /* nested */ comment */ CoPy (SELECT 1) TO STDOUT",
    ] {
        let r = conn.query(spec(sql, false, 501)).await;
        assert_eq!(r.unwrap_err().kind, RawErrorKind::Unsupported, "{sql}");
    }
    nothing_was_sent(&mut conn, &mut srv).await;
}

#[tokio::test]
async fn simple_protocol_and_binds_are_refused() {
    let (mut conn, mut srv) = session().await;
    let mut simple = spec("SELECT 1", false, 501);
    simple.protocol = Protocol::Simple;
    assert_eq!(
        conn.query(simple).await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    let binds = [BindValue::Null];
    let mut bound = spec("SELECT $1", false, 501);
    bound.binds = &binds;
    assert_eq!(
        conn.query(bound).await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    nothing_was_sent(&mut conn, &mut srv).await;
}

#[tokio::test]
async fn a_suspendable_query_in_a_transaction_suspends_and_resumes() {
    let (mut conn, mut srv) = session_in_tx().await;
    assert!(conn.typed_caps().suspend);
    let fields = [field("a", INT4), field("b", TS_VECTOR)];
    let server = tokio::spawn(async move {
        let (stmt, _) = srv.prepare(&fields, b'T').await;
        let b = srv.bind().await;
        srv.sync().await;
        srv.send(&[frame(b'2', &[]), ready(b'T')].concat()).await;
        let first = srv.execute().await;
        srv.sync().await;
        srv.send(
            &[
                int_and_text_row(1, "'a'"),
                int_and_text_row(2, "'b'"),
                frame(b's', &[]),
                ready(b'T'),
            ]
            .concat(),
        )
        .await;
        let second = srv.execute().await;
        srv.sync().await;
        srv.send(
            &[
                int_and_text_row(3, "'c'"),
                command_complete("SELECT 1"),
                ready(b'T'),
            ]
            .concat(),
        )
        .await;
        // Done: the portal, then the statement, each closed with its own Sync.
        let closed_portal = srv.close(b'P', b'T').await;
        let closed_stmt = srv.close(b'S', b'T').await;
        (stmt, b, first, second, closed_portal, closed_stmt)
    });
    conn.query(spec("SELECT a, b FROM t", true, 2))
        .await
        .unwrap();
    assert!(matches!(next(&mut conn).await, QueryEvent::ResultStart(_)));
    let rows = next(&mut conn).await;
    assert_eq!(pairs(&rows), [(1, "'a'".into()), (2, "'b'".into())]);
    assert_eq!(next(&mut conn).await, QueryEvent::Suspended);
    // Suspended: next_event waits for fetch_more or close_query.
    assert_eq!(
        conn.next_event().await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    assert_eq!(conn.tx_status(), TxStatus::InTx);
    conn.fetch_more(3).await.unwrap();
    assert_eq!(pairs(&next(&mut conn).await), [(3, "'c'".into())]);
    let QueryEvent::ResultEnd(end) = next(&mut conn).await else {
        panic!("ResultEnd");
    };
    // PostgreSQL counts only the last Execute in its tag; the driver reports the result's. A
    // read affects nothing.
    assert_eq!((end.rows, end.kept, end.rows_affected), (3, 3, None));
    assert_eq!(end.command_tag.as_deref(), Some("SELECT 3"));
    let QueryEvent::Done(done) = next(&mut conn).await else {
        panic!("Done");
    };
    assert_eq!(done.tx, TxStatus::InTx);
    let (stmt, b, first, second, closed_portal, closed_stmt) = server.await.unwrap();
    assert_eq!(b.statement, stmt);
    assert!(b.portal.starts_with('p'), "{b:?}");
    assert_eq!(b.result_formats, [1, 0]);
    assert_eq!(
        (first.portal.as_str(), first.max_rows),
        (b.portal.as_str(), 2)
    );
    assert_eq!(
        (second.portal.as_str(), second.max_rows),
        (b.portal.as_str(), 3)
    );
    assert_eq!((closed_portal, closed_stmt), (b.portal, stmt));
}

/// A suspended portal over `SELECT a, b` (2 rows delivered), inside a transaction block.
async fn suspended(conn: &mut PgConn, mut srv: Server) -> (String, String, Server) {
    let fields = [field("a", INT4), field("b", TS_VECTOR)];
    let server = tokio::spawn(async move {
        let (stmt, _) = srv.prepare(&fields, b'T').await;
        let b = srv.bind().await;
        srv.sync().await;
        srv.send(&[frame(b'2', &[]), ready(b'T')].concat()).await;
        srv.execute().await;
        srv.sync().await;
        srv.send(
            &[
                int_and_text_row(1, "'a'"),
                int_and_text_row(2, "'b'"),
                frame(b's', &[]),
                ready(b'T'),
            ]
            .concat(),
        )
        .await;
        (stmt, b.portal, srv)
    });
    conn.query(spec("SELECT a, b FROM t", true, 2))
        .await
        .unwrap();
    assert!(matches!(next(conn).await, QueryEvent::ResultStart(_)));
    assert!(matches!(next(conn).await, QueryEvent::Rows(_)));
    assert_eq!(next(conn).await, QueryEvent::Suspended);
    server.await.unwrap()
}

#[tokio::test]
async fn close_now_closes_the_portal_and_the_statement() {
    let (mut conn, srv) = session_in_tx().await;
    let (stmt, portal, mut srv) = suspended(&mut conn, srv).await;
    let server = tokio::spawn(async move {
        let p = srv.close(b'P', b'T').await;
        let s = srv.close(b'S', b'T').await;
        (p, s, srv)
    });
    assert_eq!(
        conn.close_query(CloseHow::Now).await.unwrap(),
        Closed::Closed { rows: 2 }
    );
    let (p, s, mut srv) = server.await.unwrap();
    assert_eq!((p, s), (portal, stmt));
    // The connection is free again, and the observer's count exact.
    let (ping, _) = tokio::join!(conn.ping(), srv.query("", b'T'));
    ping.unwrap();
    assert!(conn.wire_state().synced());
    assert_eq!(conn.tx_status(), TxStatus::InTx);
    // Nothing is open: a second close is a no-op.
    assert_eq!(
        conn.close_query(CloseHow::Now).await.unwrap(),
        Closed::Closed { rows: 0 }
    );
}

#[tokio::test]
async fn drain_runs_one_execute_of_4096() {
    let (mut conn, srv) = session_in_tx().await;
    let (_stmt, portal, mut srv) = suspended(&mut conn, srv).await;
    let server = tokio::spawn(async move {
        let e = srv.execute().await;
        srv.sync().await;
        srv.send(
            &[
                int_and_text_row(3, "'c'"),
                int_and_text_row(4, "'d'"),
                command_complete("SELECT 2"),
                ready(b'T'),
            ]
            .concat(),
        )
        .await;
        srv.close(b'P', b'T').await;
        srv.close(b'S', b'T').await;
        e
    });
    assert_eq!(
        conn.close_query(CloseHow::Drain(Duration::from_secs(10)))
            .await
            .unwrap(),
        Closed::Completed { rows: 4 }
    );
    assert_eq!(
        server.await.unwrap(),
        Execute {
            portal,
            max_rows: 4_096
        }
    );
}

#[tokio::test]
async fn a_statement_runs_between_portal_executes() {
    // The read-only guard's canary (R-M3-17, P5): while the portal is suspended a statement
    // runs in the same transaction, and the portal resumes after it.
    let (mut conn, srv) = session_in_tx().await;
    let (_stmt, portal, mut srv) = suspended(&mut conn, srv).await;
    let (r, q) = tokio::join!(
        conn.execute("SELECT txid_current_if_assigned()", ExecOpts::default()),
        async {
            let Msg::Parse { .. } = srv.msg().await else {
                panic!("the canary is prepared");
            };
            srv.msg().await;
            srv.sync().await;
            srv.send(
                &[
                    frame(b'1', &[]),
                    frame(b't', &0i16.to_be_bytes()),
                    row_description(&[field("t", 20)]),
                    ready(b'T'),
                ]
                .concat(),
            )
            .await;
            srv.bind().await;
            srv.execute().await;
            srv.sync().await;
            srv.send(
                &[
                    frame(b'2', &[]),
                    data_row(&[None]),
                    command_complete("SELECT 1"),
                    ready(b'T'),
                ]
                .concat(),
            )
            .await;
            srv.close(b'S', b'T').await
        }
    );
    assert_eq!(r.unwrap().row_count, 1);
    let _ = q;
    let server = tokio::spawn(async move {
        let e = srv.execute().await;
        srv.sync().await;
        srv.send(
            &[
                int_and_text_row(3, "'c'"),
                command_complete("SELECT 1"),
                ready(b'T'),
            ]
            .concat(),
        )
        .await;
        e
    });
    conn.fetch_more(10).await.unwrap();
    assert_eq!(pairs(&next(&mut conn).await), [(3, "'c'".into())]);
    assert_eq!(
        server.await.unwrap(),
        Execute {
            portal,
            max_rows: 10
        }
    );
}

#[tokio::test]
async fn statements_wait_while_a_query_streams() {
    // One open query per connection: only a suspended PostgreSQL portal lets a statement
    // through (a streaming query's unread rows would hold the statement's answer back).
    let (mut conn, mut srv) = session().await;
    let fields = [field("a", INT4), field("b", TS_VECTOR)];
    let server = tokio::spawn(async move {
        srv.prepare(&fields, b'I').await;
        srv.unnamed().await;
        // Bound, and the rows still to come.
        srv.send(&frame(b'2', &[])).await;
        srv
    });
    conn.query(spec("SELECT a, b FROM t", false, 501))
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    let mut srv = server.await.unwrap();
    for r in [
        conn.execute("SELECT 1", ExecOpts::default()).await.err(),
        conn.batch("SELECT 1").await.err(),
        conn.reset().await.err(),
        conn.query(spec("SELECT 1", false, 501)).await.err(),
        conn.ping().await.err(),
        conn.server_info().await.err(),
        conn.current_user().await.err(),
        conn.list_databases(10).await.err(),
    ] {
        assert_eq!(r.map(|e| e.kind), Some(RawErrorKind::Unsupported));
    }
    // No error type to refuse with: the handshake's facts and no warnings, nothing sent (fix
    // round 1; the server's next message below is the statement's Close).
    assert!(!conn.tls_summary().await.encrypted);
    assert!(conn.probe().await.is_empty());
    // The stream completes; then the statement closes and the connection is free.
    srv.send(&[command_complete("SELECT 0"), ready(b'I')].concat())
        .await;
    while !matches!(next(&mut conn).await, QueryEvent::Done(_)) {}
    srv.close(b'S', b'I').await;
    nothing_was_sent(&mut conn, &mut srv).await;
}

#[tokio::test]
async fn a_dml_statement_reports_one_empty_result() {
    let (mut conn, mut srv) = session().await;
    let server = tokio::spawn(async move {
        srv.prepare(&[], b'I').await;
        let b = srv.unnamed().await;
        srv.send(
            &[
                frame(b'2', &[]),
                command_complete("INSERT 0 3"),
                ready(b'I'),
            ]
            .concat(),
        )
        .await;
        b
    });
    conn.query(spec("INSERT INTO t SELECT 1", false, 501))
        .await
        .unwrap();
    let QueryEvent::ResultStart(schema) = next(&mut conn).await else {
        panic!("ResultStart");
    };
    assert!(schema.columns.is_empty());
    let QueryEvent::ResultEnd(end) = next(&mut conn).await else {
        panic!("ResultEnd");
    };
    assert_eq!((end.rows, end.rows_affected), (0, Some(3)));
    assert_eq!(end.command_tag.as_deref(), Some("INSERT 0 3"));
    assert!(matches!(next(&mut conn).await, QueryEvent::Done(_)));
    assert!(server.await.unwrap().result_formats.is_empty());
}

#[tokio::test]
async fn a_ddl_statement_has_no_count() {
    let (mut conn, mut srv) = session().await;
    let server = tokio::spawn(async move {
        srv.prepare(&[], b'I').await;
        srv.unnamed().await;
        srv.send(
            &[
                frame(b'2', &[]),
                command_complete("CREATE TABLE"),
                ready(b'I'),
            ]
            .concat(),
        )
        .await;
    });
    conn.query(spec("CREATE TABLE t (a int)", false, 501))
        .await
        .unwrap();
    assert!(matches!(next(&mut conn).await, QueryEvent::ResultStart(_)));
    let QueryEvent::ResultEnd(end) = next(&mut conn).await else {
        panic!("ResultEnd");
    };
    assert_eq!(end.rows_affected, None, "a tag without a count");
    assert_eq!(end.command_tag.as_deref(), Some("CREATE TABLE"));
    server.await.unwrap();
}

#[tokio::test]
async fn chunks_split_at_first_rows_then_chunk_rows() {
    let (mut conn, mut srv) = session().await;
    let fields = [field("a", INT4), field("b", TS_VECTOR)];
    let server = tokio::spawn(async move {
        srv.prepare(&fields, b'I').await;
        srv.unnamed().await;
        let mut script = frame(b'2', &[]);
        for i in 1..=5 {
            script.extend(int_and_text_row(i, "'x'"));
        }
        script.extend(command_complete("SELECT 5"));
        script.extend(ready(b'I'));
        srv.send(&script).await;
    });
    let mut s = spec("SELECT a, b FROM t", false, 1);
    s.chunk_rows = 2;
    conn.query(s).await.unwrap();
    assert!(matches!(next(&mut conn).await, QueryEvent::ResultStart(_)));
    let mut sizes = vec![];
    loop {
        match next(&mut conn).await {
            QueryEvent::Rows(c) => sizes.push(c.rows()),
            QueryEvent::ResultEnd(_) => break,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(sizes, [1, 2, 2]);
    server.await.unwrap();
}

#[tokio::test]
async fn a_server_error_mid_stream_leaves_an_exact_status() {
    let (mut conn, srv) = session_in_tx().await;
    let mut srv = srv;
    let fields = [field("a", INT4), field("b", TS_VECTOR)];
    let server = tokio::spawn(async move {
        srv.prepare(&fields, b'T').await;
        srv.unnamed().await;
        srv.send(
            &[
                frame(b'2', &[]),
                int_and_text_row(1, "'a'"),
                error("22012"),
                ready(b'E'),
            ]
            .concat(),
        )
        .await;
        srv.close(b'S', b'E').await;
        srv
    });
    conn.query(spec("SELECT a, b FROM t", false, 501))
        .await
        .unwrap();
    assert!(matches!(next(&mut conn).await, QueryEvent::ResultStart(_)));
    let e = conn.next_event().await.unwrap_err();
    assert_eq!(e.kind, RawErrorKind::Db);
    assert_eq!(
        e.server.unwrap().sqlstate.as_deref(),
        Some("22012"),
        "the server's code"
    );
    assert_eq!(conn.tx_status(), TxStatus::Failed, "exact right after");
    let mut srv = server.await.unwrap();
    let (ping, _) = tokio::join!(conn.ping(), srv.query("", b'E'));
    ping.unwrap();
    assert!(conn.wire_state().synced());
}

/// The broken state after a hostile reply: `Protocol`, the connection never used again.
async fn assert_broken_by(reply: Vec<u8>) {
    let (mut conn, mut srv) = session().await;
    let fields = [field("a", INT4), field("b", TS_VECTOR)];
    let server = tokio::spawn(async move {
        srv.prepare(&fields, b'I').await;
        srv.unnamed().await;
        let mut script = frame(b'2', &[]);
        script.extend(reply);
        script.extend(command_complete("SELECT 1"));
        script.extend(ready(b'I'));
        srv.send(&script).await;
        srv
    });
    conn.query(spec("SELECT a, b FROM t", false, 501))
        .await
        .unwrap();
    assert!(matches!(next(&mut conn).await, QueryEvent::ResultStart(_)));
    let e = conn.next_event().await.unwrap_err();
    assert_eq!(e.kind, RawErrorKind::Protocol);
    assert_eq!(conn.tx_status(), TxStatus::Failed, "never a stale Idle");
    let _srv = server.await.unwrap();
    let again = conn.query(spec("SELECT 1", false, 501)).await.unwrap_err();
    assert_eq!(again.kind, RawErrorKind::Io(io::ErrorKind::NotConnected));
    assert!(conn.execute("SELECT 1", ExecOpts::default()).await.is_err());
}

#[tokio::test]
async fn a_data_row_with_the_wrong_column_count_breaks_the_connection() {
    assert_broken_by(data_row(&[Some(&1i32.to_be_bytes())])).await;
    assert_broken_by(data_row(&[
        Some(&1i32.to_be_bytes()),
        Some(b"'a'"),
        Some(b"extra"),
    ]))
    .await;
}

#[tokio::test]
async fn oversized_field_lengths_and_counts_break_the_connection() {
    // A field longer than the message.
    let mut body = 2i16.to_be_bytes().to_vec();
    body.extend_from_slice(&1_000_000i32.to_be_bytes());
    body.extend_from_slice(b"abcd");
    assert_broken_by(frame(b'D', &body)).await;
    // A field count the body cannot hold.
    assert_broken_by(data_row_counted(i16::MAX, &[None, None])).await;
    // A negative field count (read as 65,535 fields).
    assert_broken_by(data_row_counted(-1, &[None, None])).await;
}

#[tokio::test]
async fn a_negative_field_length_reads_as_null_and_framing_stays_aligned() {
    // `postgres-protocol` 0.6.12 reads every negative field length as NULL (only -1 is valid);
    // the length is consumed and no byte is skipped, so the next field is still the server's.
    // A known limit, recorded in ADR 0002: not patched (that crate is not vendored).
    let (mut conn, mut srv) = session().await;
    let fields = [field("a", INT4), field("b", TS_VECTOR)];
    let server = tokio::spawn(async move {
        srv.prepare(&fields, b'I').await;
        srv.unnamed().await;
        let mut body = 2i16.to_be_bytes().to_vec();
        body.extend_from_slice(&(-2i32).to_be_bytes());
        body.extend_from_slice(&3i32.to_be_bytes());
        body.extend_from_slice(b"'a'");
        srv.send(
            &[
                frame(b'2', &[]),
                frame(b'D', &body),
                command_complete("SELECT 1"),
                ready(b'I'),
            ]
            .concat(),
        )
        .await;
    });
    conn.query(spec("SELECT a, b FROM t", false, 501))
        .await
        .unwrap();
    assert!(matches!(next(&mut conn).await, QueryEvent::ResultStart(_)));
    let QueryEvent::Rows(chunk) = next(&mut conn).await else {
        panic!("Rows");
    };
    assert_eq!(chunk.cell(0, 0).to_value(), Value::Null);
    assert!(matches!(chunk.cell(0, 1).to_value(), Value::Opaque(o) if &*o.text == "'a'"));
    server.await.unwrap();
}

#[tokio::test]
async fn a_hostile_cell_is_an_error_cell_and_the_row_is_kept() {
    // An int4 of 3 bytes is the server's mistake in one cell, not in the stream: the row stays.
    let (mut conn, mut srv) = session().await;
    let fields = [field("a", INT4), field("b", TS_VECTOR)];
    let server = tokio::spawn(async move {
        srv.prepare(&fields, b'I').await;
        srv.unnamed().await;
        srv.send(
            &[
                frame(b'2', &[]),
                data_row(&[Some(&[0, 0, 1]), Some(b"'a'")]),
                command_complete("SELECT 1"),
                ready(b'I'),
            ]
            .concat(),
        )
        .await;
    });
    conn.query(spec("SELECT a, b FROM t", false, 501))
        .await
        .unwrap();
    assert!(matches!(next(&mut conn).await, QueryEvent::ResultStart(_)));
    let QueryEvent::Rows(chunk) = next(&mut conn).await else {
        panic!("Rows");
    };
    assert!(matches!(chunk.cell(0, 0).to_value(), Value::Error(_)));
    assert!(matches!(chunk.cell(0, 1).to_value(), Value::Opaque(_)));
    server.await.unwrap();
}

// ---------------------------------------------------------------------------------------------
// Fix round 1 (the Task 12 review).

fn int_row(i: i32) -> Vec<u8> {
    data_row(&[Some(&i.to_be_bytes())])
}

/// A non-suspendable `UPDATE … RETURNING a` (one int4 column) with one row delivered; the
/// server sends `rest` once `go` fires, then answers the statement's Close with `status`.
async fn closing_unnamed(
    in_tx: bool,
    rest: Vec<u8>,
    status: u8,
) -> (
    PgConn,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<Server>,
) {
    let (mut conn, mut srv) = if in_tx {
        session_in_tx().await
    } else {
        session().await
    };
    let (go_tx, go_rx) = tokio::sync::oneshot::channel::<()>();
    let fields = [field("a", INT4)];
    let before = if in_tx { b'T' } else { b'I' };
    let server = tokio::spawn(async move {
        srv.prepare(&fields, before).await;
        srv.unnamed().await;
        srv.send(&[frame(b'2', &[]), int_row(1)].concat()).await;
        go_rx.await.unwrap();
        srv.send(&rest).await;
        srv.close(b'S', status).await;
        srv
    });
    conn.query(spec("UPDATE t SET a = a RETURNING a", false, 1))
        .await
        .unwrap();
    assert!(matches!(next(&mut conn).await, QueryEvent::ResultStart(_)));
    assert!(matches!(next(&mut conn).await, QueryEvent::Rows(_)));
    (conn, go_tx, server)
}

#[tokio::test]
async fn closing_an_unnamed_portal_that_ran_to_its_end_is_completed() {
    // Review I1: the statement ran to its end on the server (in autocommit its Sync committed
    // it): `Completed` with the total rows, whatever `how` says.
    for how in [CloseHow::Now, CloseHow::Drain(Duration::from_secs(10))] {
        let rest = [
            int_row(2),
            int_row(3),
            command_complete("UPDATE 3"),
            ready(b'I'),
        ]
        .concat();
        let (mut conn, go, server) = closing_unnamed(false, rest, b'I').await;
        go.send(()).unwrap();
        assert_eq!(
            conn.close_query(how).await.unwrap(),
            Closed::Completed { rows: 3 },
            "{how:?}"
        );
        let mut srv = server.await.unwrap();
        assert_eq!(conn.tx_status(), TxStatus::Idle);
        let (ping, _) = tokio::join!(conn.ping(), srv.query("", b'I'));
        ping.unwrap();
        assert!(conn.wire_state().synced());
    }
}

#[tokio::test]
async fn a_server_error_while_closing_an_unnamed_portal_is_the_error() {
    // Review I1: the statement failed while its rest was read (a cancel's 57014): it did not
    // apply, so the close answers the server's error; the connection stays usable and the
    // status is exact (autocommit: Idle; inside the user's transaction: Failed).
    for (in_tx, status, want) in [
        (false, b'I', TxStatus::Idle),
        (true, b'E', TxStatus::Failed),
    ] {
        let rest = [int_row(2), error("57014"), ready(status)].concat();
        let (mut conn, go, server) = closing_unnamed(in_tx, rest, status).await;
        go.send(()).unwrap();
        let e = conn.close_query(CloseHow::Now).await.unwrap_err();
        assert_eq!(e.kind, RawErrorKind::Db);
        assert_eq!(e.server.unwrap().sqlstate.as_deref(), Some("57014"));
        let mut srv = server.await.unwrap();
        assert_eq!(conn.tx_status(), want);
        let (ping, _) = tokio::join!(conn.ping(), srv.query("", status));
        ping.unwrap();
        assert!(conn.wire_state().synced());
        // Nothing is open any more.
        assert_eq!(
            conn.close_query(CloseHow::Now).await.unwrap(),
            Closed::Closed { rows: 0 }
        );
    }
}

#[tokio::test]
async fn a_drain_that_runs_out_of_time_closes_the_portal() {
    // Review M4: the drain's Execute(4096) is slow; at its deadline the rest is left to the
    // connection task and the portal closes: `Closed` with the rows delivered.
    let (mut conn, srv) = session_in_tx().await;
    let (_stmt, portal, mut srv) = suspended(&mut conn, srv).await;
    let (go_tx, go_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let e = srv.execute().await;
        srv.sync().await;
        // Half of the rest, then nothing until the client has given up.
        srv.send(&int_and_text_row(3, "'c'")).await;
        go_rx.await.unwrap();
        srv.send(
            &[
                int_and_text_row(4, "'d'"),
                command_complete("SELECT 2"),
                ready(b'T'),
            ]
            .concat(),
        )
        .await;
        let closed = srv.close(b'P', b'T').await;
        srv.close(b'S', b'T').await;
        (e, closed, srv)
    });
    assert_eq!(
        conn.close_query(CloseHow::Drain(Duration::from_millis(100)))
            .await
            .unwrap(),
        Closed::Closed { rows: 2 }
    );
    go_tx.send(()).unwrap();
    let (e, closed, mut srv) = server.await.unwrap();
    assert_eq!(
        e,
        Execute {
            portal: portal.clone(),
            max_rows: 4_096
        }
    );
    assert_eq!(closed, portal);
    let (ping, _) = tokio::join!(conn.ping(), srv.query("", b'T'));
    ping.unwrap();
    assert!(conn.wire_state().synced());
    assert_eq!(conn.tx_status(), TxStatus::InTx);
}

#[tokio::test]
async fn corruption_below_the_stream_ends_the_connection() {
    // Review M2: a frame the wire observer refuses (a length below 4) and a notice the
    // connection task cannot parse end the connection before the stream sees a message:
    // `Io(ConnectionReset)`, broken (fail-closed, never `Protocol` here).
    for hostile in [vec![b'D', 0, 0, 0, 2], frame(b'N', b"Mno terminator")] {
        let (mut conn, mut srv) = session().await;
        let fields = [field("a", INT4)];
        let (go_tx, go_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            srv.prepare(&fields, b'I').await;
            srv.unnamed().await;
            srv.send(&[frame(b'2', &[]), int_row(1)].concat()).await;
            // In the middle of the stream, after the client read its first row.
            go_rx.await.unwrap();
            let mut script = hostile;
            script.extend(command_complete("SELECT 1"));
            script.extend(ready(b'I'));
            srv.send(&script).await;
            srv
        });
        conn.query(spec("SELECT a FROM t", false, 1)).await.unwrap();
        assert!(matches!(next(&mut conn).await, QueryEvent::ResultStart(_)));
        assert!(matches!(next(&mut conn).await, QueryEvent::Rows(_)));
        go_tx.send(()).unwrap();
        let e = conn.next_event().await.unwrap_err();
        assert_eq!(e.kind, RawErrorKind::Io(io::ErrorKind::ConnectionReset));
        let _srv = server.await.unwrap();
        assert_eq!(conn.tx_status(), TxStatus::Failed, "never a stale Idle");
        let again = conn.query(spec("SELECT 1", false, 501)).await.unwrap_err();
        assert_eq!(again.kind, RawErrorKind::Io(io::ErrorKind::NotConnected));
    }
}

/// A plain `tokio-postgres` client over a scripted server, signed in (for `Transaction`, which
/// needs `&mut Client`).
async fn raw_client() -> (tokio_postgres::Client, Server) {
    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);
    let server = tokio::spawn(async move {
        let len = server_io.read_u32().await.unwrap() as usize;
        let mut startup = vec![0; len - 4];
        server_io.read_exact(&mut startup).await.unwrap();
        let mut key = 32i32.to_be_bytes().to_vec();
        key.extend_from_slice(&1i32.to_be_bytes());
        server_io
            .write_all(
                &[
                    frame(b'R', &0i32.to_be_bytes()),
                    frame(b'K', &key),
                    ready(b'I'),
                ]
                .concat(),
            )
            .await
            .unwrap();
        Server { s: server_io }
    });
    let (client, connection) = tokio_postgres::Config::new()
        .user("hv_test")
        .dbname("hv_test")
        .ssl_mode(tokio_postgres::config::SslMode::Disable)
        .connect_raw(client_io, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    (client, server.await.unwrap())
}

/// The bytes 0.7.18's `bind::bind` writes (`Transaction::bind_raw`, no parameters): Bind into
/// `portal` with every result column binary, then `Sync`; built from the protocol.
fn golden_0_7_18_bind(portal: &str, statement: &str) -> Vec<u8> {
    let mut bind = portal.as_bytes().to_vec();
    bind.push(0);
    bind.extend_from_slice(statement.as_bytes());
    bind.push(0);
    bind.extend_from_slice(&0i16.to_be_bytes());
    bind.extend_from_slice(&0i16.to_be_bytes());
    bind.extend_from_slice(&1i16.to_be_bytes());
    bind.extend_from_slice(&1i16.to_be_bytes());
    [frame(b'B', &bind), frame(b'S', &[])].concat()
}

#[tokio::test]
async fn existing_bind_and_execute_typed_paths_are_byte_identical() {
    // Review M6: the two other callers patch §1 touched, `bind::bind` (through
    // `Transaction::bind_raw`) and `execute_typed`, against the 0.7.18 bytes.
    let (mut client, mut srv) = raw_client().await;
    let server = tokio::spawn(async move {
        let wire = raw_frames(&mut srv, 5).await;
        srv.send(
            &[
                frame(b'1', &[]),
                frame(b'2', &[]),
                frame(b'n', &[]),
                command_complete("SELECT 1"),
                ready(b'I'),
            ]
            .concat(),
        )
        .await;
        (wire, srv)
    });
    client.execute_typed("SELECT 1", &[]).await.unwrap();
    let (wire, mut srv) = server.await.unwrap();
    assert_eq!(wire, golden_0_7_18_query_typed_raw(), "execute_typed");
    let fields = [field("a", INT4)];
    let server = tokio::spawn(async move {
        assert_eq!(
            srv.query("START TRANSACTION", b'T').await,
            "START TRANSACTION"
        );
        let (statement, _) = srv.prepare(&fields, b'T').await;
        let wire = raw_frames(&mut srv, 2).await;
        srv.send(&[frame(b'2', &[]), ready(b'T')].concat()).await;
        (statement, wire, srv)
    });
    let tx = client.transaction().await.unwrap();
    let stmt = tx.prepare("SELECT a FROM t").await.unwrap();
    let portal = tx.bind_raw(&stmt, no_params()).await.unwrap();
    let (statement, wire, _srv) = server.await.unwrap();
    // The portal's name comes from the process-wide counter: read it, then compare the rest.
    let Msg::Bind(b) = parse_msg(b'B', &wire[5..wire.len() - 5]) else {
        panic!("Bind first: {wire:?}");
    };
    assert!(b.portal.starts_with('p'), "{b:?}");
    assert_eq!(wire, golden_0_7_18_bind(&b.portal, &statement), "bind_raw");
    drop(portal);
    drop(tx);
}
