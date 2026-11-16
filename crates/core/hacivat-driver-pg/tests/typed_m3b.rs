//! M3b Task 13 (HV-EXEC-03 driver half; R-M3-60, R-M3-66, R-M3-77, R-M3-79) on PostgreSQL,
//! against a scripted in-memory server (the pattern of `tests/tp_patches.rs`; no Docker): the
//! exact Bind a bound query sends (text-format parameters, the SQL text untouched), the
//! parameter count checked before any Bind, notice text kept and capped as `QueryEvent::Notice`
//! (and never taken from a statement run beside a suspended query), and Run as one over the
//! simple protocol (one untyped result per statement, the server's text only, sets past the
//! cap counted). `docker_typed_m3b.rs` runs the same rules on real servers.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::io;
use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::{
    DriverConnect, Endpoint, InsecureAuth, NoticeSeverity, Protocol, QueryEvent, QuerySpec,
    RawConn, RawErrorKind, TxStatus,
};
use hacivat_driver_pg::{PgConn, connect_conn};
use hacivat_net::{BoxedStream, StreamFactory};
use hacivat_values::{BindValue, LogicalType, NativeType, Value};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::mpsc;

// ## m3b-t13-drivers

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

/// An ErrorResponse (`E`) or NoticeResponse (`N`) with these fields.
fn fields_msg(typ: u8, severity: &str, code: &str, message: &str) -> Vec<u8> {
    let mut body = vec![];
    for (f, v) in [
        (b'S', severity),
        (b'V', severity),
        (b'C', code),
        (b'M', message),
    ] {
        body.push(f);
        body.extend_from_slice(v.as_bytes());
        body.push(0);
    }
    body.push(0);
    frame(typ, &body)
}

fn notice(severity: &str, code: &str, message: &str) -> Vec<u8> {
    fields_msg(b'N', severity, code, message)
}

fn error(code: &str) -> Vec<u8> {
    fields_msg(b'E', "ERROR", code, "scripted")
}

const INT4: u32 = 23;
const TEXT: u32 = 25;
const INT4_ARRAY: u32 = 1007;

fn row_description(fields: &[(&str, u32)]) -> Vec<u8> {
    let mut body = i16::try_from(fields.len()).unwrap().to_be_bytes().to_vec();
    for (name, oid) in fields {
        body.extend_from_slice(name.as_bytes());
        body.push(0);
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&0i16.to_be_bytes());
        body.extend_from_slice(&oid.to_be_bytes());
        body.extend_from_slice(&(-1i16).to_be_bytes());
        body.extend_from_slice(&(-1i32).to_be_bytes());
        body.extend_from_slice(&0i16.to_be_bytes());
    }
    frame(b'T', &body)
}

fn data_row(cells: &[Option<&[u8]>]) -> Vec<u8> {
    let mut body = i16::try_from(cells.len()).unwrap().to_be_bytes().to_vec();
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

fn param_description(oids: &[u32]) -> Vec<u8> {
    let mut body = i16::try_from(oids.len()).unwrap().to_be_bytes().to_vec();
    for oid in oids {
        body.extend_from_slice(&oid.to_be_bytes());
    }
    frame(b't', &body)
}

// ---------------------------------------------------------------------------------------------
// Frontend messages, as the server reads them.

#[derive(Debug, Clone, PartialEq, Eq)]
struct Bind {
    portal: String,
    statement: String,
    param_formats: Vec<i16>,
    /// Each parameter's bytes (`None`: NULL).
    params: Vec<Option<Vec<u8>>>,
    result_formats: Vec<i16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Msg {
    Parse { name: String, query: String },
    Describe(u8, String),
    Bind(Bind),
    Execute(String, i32),
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
            at = 1;
            Msg::Describe(body[0], cstr(body, &mut at))
        }
        b'B' => {
            let portal = cstr(body, &mut at);
            let statement = cstr(body, &mut at);
            let n = i16_at(body, &mut at);
            let param_formats = (0..n).map(|_| i16_at(body, &mut at)).collect();
            let count = i16_at(body, &mut at);
            let mut params = Vec::new();
            for _ in 0..count {
                let len = i32_at(body, &mut at);
                if len < 0 {
                    params.push(None);
                } else {
                    let len = len as usize;
                    params.push(Some(body[at..at + len].to_vec()));
                    at += len;
                }
            }
            let n = i16_at(body, &mut at);
            let result_formats = (0..n).map(|_| i16_at(body, &mut at)).collect();
            Msg::Bind(Bind {
                portal,
                statement,
                param_formats,
                params,
                result_formats,
            })
        }
        b'E' => {
            let portal = cstr(body, &mut at);
            Msg::Execute(portal, i32_at(body, &mut at))
        }
        b'C' => {
            at = 1;
            Msg::Close(body[0], cstr(body, &mut at))
        }
        b'S' => Msg::Sync,
        b'H' => Msg::Flush,
        b'Q' => Msg::Query(String::from_utf8(body[..body.len() - 1].to_vec()).unwrap()),
        other => Msg::Other(other),
    }
}

struct Server {
    s: DuplexStream,
}

/// A bound on every read, so a driver that never writes fails the test instead of hanging it.
const READ_GUARD: Duration = Duration::from_secs(20);

impl Server {
    async fn msg(&mut self) -> Msg {
        tokio::time::timeout(READ_GUARD, async {
            let typ = self.s.read_u8().await.unwrap();
            let len = self.s.read_u32().await.unwrap() as usize;
            let mut body = vec![0; len - 4];
            self.s.read_exact(&mut body).await.unwrap();
            parse_msg(typ, &body)
        })
        .await
        .expect("the client sent the next message")
    }

    async fn send(&mut self, bytes: &[u8]) {
        self.s.write_all(bytes).await.unwrap();
    }

    /// Answers a `prepare` (Parse, Describe, Sync) with parameters of `params` (their OIDs) and
    /// result `fields` (NoData when empty); returns the statement name and the query.
    async fn prepare(
        &mut self,
        params: &[u32],
        fields: &[(&str, u32)],
        status: u8,
    ) -> (String, String) {
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
                param_description(params),
                description,
                ready(status),
            ]
            .concat(),
        )
        .await;
        (name, query)
    }

    async fn sync(&mut self) {
        assert_eq!(self.msg().await, Msg::Sync);
    }

    /// Reads a non-suspendable unit's one batch (Bind into the unnamed portal, Flush,
    /// `Execute(0)`, Sync) and returns its Bind.
    async fn unnamed(&mut self) -> Bind {
        let Msg::Bind(b) = self.msg().await else {
            panic!("expected Bind");
        };
        assert_eq!(b.portal, "");
        assert_eq!(self.msg().await, Msg::Flush);
        assert_eq!(self.msg().await, Msg::Execute(String::new(), 0));
        self.sync().await;
        b
    }

    /// Answers a `Close` + `Sync` of `kind` and returns the name closed.
    async fn close(&mut self, kind: u8, status: u8) -> String {
        let name = match self.msg().await {
            Msg::Close(k, name) if k == kind => name,
            other => panic!("expected Close {}, got {other:?}", kind as char),
        };
        self.sync().await;
        self.send(&[frame(b'3', &[]), ready(status)].concat()).await;
        name
    }

    /// Reads a simple `Query` and returns its text.
    async fn query_text(&mut self) -> String {
        match self.msg().await {
            Msg::Query(q) => q,
            other => panic!("expected Query, got {other:?}"),
        }
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

fn spec(sql: &str) -> QuerySpec<'_> {
    QuerySpec {
        sql,
        binds: &[],
        protocol: Protocol::Extended,
        suspendable: false,
        first_rows: 501,
        chunk_rows: 4_096,
        chunk_bytes: 4 << 20,
        cell_cap: 1 << 20,
        max_result_sets: 1,
    }
}

fn simple(sql: &str, max_result_sets: u32) -> QuerySpec<'_> {
    QuerySpec {
        protocol: Protocol::Simple,
        max_result_sets,
        ..spec(sql)
    }
}

fn typed(hint: LogicalType, text: &str) -> BindValue {
    BindValue::Typed {
        hint,
        text: Arc::from(text),
    }
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

fn int(s: &str) -> BindValue {
    typed(
        LogicalType::Int {
            bits: 32,
            signed: true,
        },
        s,
    )
}

/// Every event to `Done`.
async fn events(conn: &mut PgConn) -> Vec<QueryEvent> {
    let mut out = Vec::new();
    loop {
        let e = conn.next_event().await.unwrap();
        let done = matches!(e, QueryEvent::Done(_));
        out.push(e);
        if done {
            return out;
        }
    }
}

fn cell(e: &QueryEvent, row: u32, col: u32) -> Value {
    let QueryEvent::Rows(chunk) = e else {
        panic!("expected Rows, got {e:?}");
    };
    chunk.cell(row, col).to_value()
}

fn txt(s: &str) -> Value {
    Value::Text(Arc::from(s))
}

// ---------------------------------------------------------------------------------------------
// Binds.

#[tokio::test]
async fn pg_param_count_mismatch_is_refused_before_send() {
    for binds in [vec![int("1")], vec![int("1"), text("a"), text("b")], vec![]] {
        let (mut conn, mut srv) = session().await;
        assert!(conn.typed_caps().binds);
        let server = tokio::spawn(async move {
            let (name, _) = srv.prepare(&[INT4, TEXT], &[("a", INT4)], b'I').await;
            // The statement is closed at once: no Bind, no Execute.
            assert_eq!(srv.close(b'S', b'I').await, name);
            srv
        });
        let mut s = spec("SELECT a FROM t WHERE a = $1 AND b = $2");
        s.binds = &binds;
        assert_eq!(
            conn.query(s).await.unwrap_err().kind,
            RawErrorKind::Unsupported,
            "{} binds",
            binds.len()
        );
        let mut srv = server.await.unwrap();
        // The next message the server reads is the ping's: nothing else was sent.
        let (ping, q) = tokio::join!(conn.ping(), async {
            let q = srv.query_text().await;
            srv.send(&[frame(b'I', &[]), ready(b'I')].concat()).await;
            q
        });
        ping.unwrap();
        assert_eq!(q, "");
    }
}

#[tokio::test]
async fn binds_go_as_text_format_parameters_and_never_into_the_text() {
    let (mut conn, mut srv) = session().await;
    let hostile = "x'); DROP TABLE t; --";
    let sql = "SELECT a FROM t WHERE a = $1 AND b = $2 AND c IS NOT DISTINCT FROM $3";
    let server = tokio::spawn(async move {
        let (_, query) = srv.prepare(&[INT4, TEXT, TEXT], &[("a", INT4)], b'I').await;
        let bind = srv.unnamed().await;
        srv.send(
            &[
                frame(b'2', &[]),
                data_row(&[Some(&7i32.to_be_bytes())]),
                command_complete("SELECT 1"),
                ready(b'I'),
            ]
            .concat(),
        )
        .await;
        (query, bind, srv)
    });
    let binds = [int("7"), text(hostile), BindValue::Null];
    let mut s = spec(sql);
    s.binds = &binds;
    conn.query(s).await.unwrap();
    let evs = events(&mut conn).await;
    assert_eq!(cell(&evs[1], 0, 0), Value::Int(7));
    let (query, bind, _srv) = server.await.unwrap();
    // The statement text is the caller's, byte for byte.
    assert_eq!(query, sql);
    // Every parameter in text format (0); the values as typed; NULL as NULL.
    assert_eq!(bind.param_formats, [0, 0, 0]);
    assert_eq!(
        bind.params,
        [Some(b"7".to_vec()), Some(hostile.as_bytes().to_vec()), None]
    );
    // The result columns keep their own formats (int4 binary).
    assert_eq!(bind.result_formats, [1]);
}

// ---------------------------------------------------------------------------------------------
// Notices.

#[tokio::test]
async fn pg_notice_text_is_kept_and_capped() {
    let (mut conn, mut srv) = session().await;
    let long = "é".repeat(10_000); // 20,000 bytes, two per character
    let long2 = long.clone();
    let server = tokio::spawn(async move {
        srv.prepare(&[], &[("a", INT4)], b'I').await;
        srv.unnamed().await;
        let mut out = vec![frame(b'2', &[])];
        out.push(notice("WARNING", "01000", &long2));
        for i in 1..150 {
            out.push(notice("NOTICE", "00000", &format!("n{i}")));
        }
        out.push(data_row(&[Some(&1i32.to_be_bytes())]));
        out.push(command_complete("SELECT 1"));
        out.push(ready(b'I'));
        srv.send(&out.concat()).await;
        srv
    });
    conn.query(spec("SELECT f()")).await.unwrap();
    let evs = events(&mut conn).await;
    let notices: Vec<_> = evs
        .iter()
        .filter_map(|e| match e {
            QueryEvent::Notice(n) => Some(n.clone()),
            _ => None,
        })
        .collect();
    // The first 100 of the statement's notices, in order.
    assert_eq!(notices.len(), 100);
    assert_eq!(notices[0].severity, NoticeSeverity::Warning);
    assert_eq!(notices[0].code.as_deref(), Some("01000"));
    // 16 KiB, cut at a character boundary: 8,192 two-byte characters.
    assert_eq!(notices[0].text.len(), 16 * 1024);
    assert!(long.starts_with(&*notices[0].text));
    for (i, n) in notices.iter().enumerate().skip(1) {
        assert_eq!(n.severity, NoticeSeverity::Notice);
        assert_eq!(&*n.text, format!("n{i}"));
    }
    // Every notice comes before Done, and the grammar around them is unchanged.
    let rest: Vec<&str> = evs
        .iter()
        .filter_map(|e| match e {
            QueryEvent::Notice(_) => None,
            QueryEvent::ResultStart(_) => Some("start"),
            QueryEvent::Rows(_) => Some("rows"),
            QueryEvent::ResultEnd(_) => Some("end"),
            QueryEvent::Done(_) => Some("done"),
            QueryEvent::Suspended => Some("suspended"),
        })
        .collect();
    assert_eq!(rest, ["start", "rows", "end", "done"]);
    assert!(matches!(evs.last(), Some(QueryEvent::Done(_))));
    let _srv = server.await.unwrap();
    // The next query starts with none of them.
}

#[tokio::test]
async fn notices_before_a_server_error_come_out_before_it() {
    let (mut conn, mut srv) = session().await;
    let server = tokio::spawn(async move {
        srv.prepare(&[], &[("a", INT4)], b'I').await;
        srv.unnamed().await;
        srv.send(
            &[
                frame(b'2', &[]),
                notice("NOTICE", "00000", "x = 41"),
                notice("NOTICE", "00000", "x = 42"),
                error("P0001"),
                ready(b'I'),
            ]
            .concat(),
        )
        .await;
        srv
    });
    conn.query(spec("SELECT f()")).await.unwrap();
    let mut texts = Vec::new();
    let err = loop {
        match conn.next_event().await {
            Ok(QueryEvent::Notice(n)) => texts.push(n.text.to_string()),
            Ok(QueryEvent::ResultStart(_)) => {}
            Ok(other) => panic!("unexpected {other:?}"),
            Err(e) => break e,
        }
    };
    assert_eq!(texts, ["x = 41", "x = 42"]);
    assert_eq!(err.kind, RawErrorKind::Db);
    assert_eq!(
        err.server.as_ref().and_then(|s| s.sqlstate.as_deref()),
        Some("P0001")
    );
    // The query is over.
    assert_eq!(
        conn.next_event().await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    let _srv = server.await.unwrap();
}

#[tokio::test]
async fn a_statement_beside_a_suspended_query_never_lends_it_notices() {
    let (mut conn, mut srv) = session().await;
    let (r, q) = tokio::join!(conn.batch("BEGIN"), async {
        let q = srv.query_text().await;
        srv.send(&[command_complete("BEGIN"), ready(b'T')].concat())
            .await;
        q
    });
    r.unwrap();
    assert_eq!(q, "BEGIN");
    assert_eq!(conn.tx_status(), TxStatus::InTx);
    let server = tokio::spawn(async move {
        srv.prepare(&[], &[("a", INT4)], b'T').await;
        let Msg::Bind(_) = srv.msg().await else {
            panic!("Bind");
        };
        srv.sync().await;
        srv.send(&[frame(b'2', &[]), ready(b'T')].concat()).await;
        let Msg::Execute(..) = srv.msg().await else {
            panic!("Execute");
        };
        srv.sync().await;
        srv.send(
            &[
                notice("NOTICE", "00000", "mine"),
                data_row(&[Some(&1i32.to_be_bytes())]),
                frame(b's', &[]),
                ready(b'T'),
            ]
            .concat(),
        )
        .await;
        // The canary beside it raises a notice of its own.
        let q = srv.query_text().await;
        srv.send(
            &[
                notice("NOTICE", "00000", "theirs"),
                command_complete("SET"),
                ready(b'T'),
            ]
            .concat(),
        )
        .await;
        let Msg::Execute(..) = srv.msg().await else {
            panic!("Execute");
        };
        srv.sync().await;
        srv.send(
            &[
                data_row(&[Some(&2i32.to_be_bytes())]),
                command_complete("SELECT 1"),
                ready(b'T'),
            ]
            .concat(),
        )
        .await;
        (q, srv)
    });
    let mut s = spec("SELECT a FROM t");
    s.suspendable = true;
    s.first_rows = 1;
    conn.query(s).await.unwrap();
    let mut evs = Vec::new();
    loop {
        let e = conn.next_event().await.unwrap();
        let suspended = e == QueryEvent::Suspended;
        evs.push(e);
        if suspended {
            break;
        }
    }
    conn.batch("SET x = 1").await.unwrap();
    conn.fetch_more(10).await.unwrap();
    evs.extend(events(&mut conn).await);
    let texts: Vec<String> = evs
        .iter()
        .filter_map(|e| match e {
            QueryEvent::Notice(n) => Some(n.text.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["mine"]);
    let (q, _srv) = server.await.unwrap();
    assert_eq!(q, "SET x = 1");
}

// ---------------------------------------------------------------------------------------------
// Run as one (the simple protocol).

/// The scripted reply to `SELECT 1, 'a'; INSERT …; SELECT '{1,2}'::int4[]; SELECT 'x'`.
fn three_statements() -> Vec<u8> {
    [
        row_description(&[("n", INT4), ("s", TEXT)]),
        data_row(&[Some(b"1"), Some(b"a")]),
        data_row(&[Some(b"2"), None]),
        command_complete("SELECT 2"),
        command_complete("INSERT 0 3"),
        row_description(&[("arr", INT4_ARRAY)]),
        // Text, as the simple protocol sends it; never read as binary (T7 m-10).
        data_row(&[Some(b"{1,2}")]),
        // Not UTF-8 (a SQL_ASCII database): an error cell, the row kept.
        data_row(&[Some(&[0xff, 0xfe])]),
        command_complete("SELECT 2"),
        row_description(&[("x", TEXT)]),
        data_row(&[Some(b"x")]),
        command_complete("SELECT 1"),
        ready(b'I'),
    ]
    .concat()
}

const AS_ONE: &str = "SELECT 1, 'a' UNION ALL SELECT 2, NULL; INSERT INTO t VALUES (1), (2), (3); \
                      SELECT '{1,2}'::int4[]; SELECT 'x'";

#[tokio::test]
async fn pg_simple_protocol_yields_one_result_per_statement_untyped() {
    let (mut conn, mut srv) = session().await;
    let server = tokio::spawn(async move {
        let q = srv.query_text().await;
        srv.send(&three_statements()).await;
        (q, srv)
    });
    conn.query(simple(AS_ONE, 100)).await.unwrap();
    let evs = events(&mut conn).await;
    let (q, _srv) = server.await.unwrap();
    // One Query of the whole text.
    assert_eq!(q, AS_ONE);
    let shape: Vec<&str> = evs
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
    assert_eq!(
        shape,
        [
            "start", "rows", "end", "start", "end", "start", "rows", "end", "start", "rows", "end",
            "done"
        ]
    );
    // Every column text, of unknown native type, whatever the server's type was.
    for i in [0, 5, 8] {
        let QueryEvent::ResultStart(schema) = &evs[i] else {
            panic!("start at {i}");
        };
        for c in schema.columns.iter() {
            assert_eq!(
                c.logical,
                LogicalType::Text {
                    max_chars: None,
                    fixed: false
                }
            );
            assert_eq!(c.native, NativeType::Unknown);
        }
    }
    let QueryEvent::ResultStart(empty) = &evs[3] else {
        panic!("the INSERT's result");
    };
    assert!(empty.columns.is_empty());
    assert_eq!(cell(&evs[1], 0, 0), txt("1"));
    assert_eq!(cell(&evs[1], 0, 1), txt("a"));
    assert_eq!(cell(&evs[1], 1, 1), Value::Null);
    assert_eq!(cell(&evs[6], 0, 0), txt("{1,2}"));
    assert!(matches!(cell(&evs[6], 1, 0), Value::Error(e) if &*e.code == "pg-text"));
    let ends: Vec<_> = evs
        .iter()
        .filter_map(|e| match e {
            QueryEvent::ResultEnd(end) => Some((end.rows, end.kept, end.rows_affected)),
            _ => None,
        })
        .collect();
    assert_eq!(
        ends,
        [(2, 2, None), (0, 0, Some(3)), (2, 2, None), (1, 1, None)]
    );
    let QueryEvent::Done(done) = evs.last().unwrap() else {
        unreachable!()
    };
    assert_eq!(done.skipped_result_sets, 0);
    assert_eq!(done.tx, TxStatus::Idle);
}

#[tokio::test]
async fn simple_protocol_sets_past_the_cap_are_read_and_counted() {
    let (mut conn, mut srv) = session().await;
    let server = tokio::spawn(async move {
        srv.query_text().await;
        srv.send(&three_statements()).await;
        srv
    });
    // Asked to suspend: Run as one never does.
    let mut s = simple(AS_ONE, 2);
    s.suspendable = true;
    s.first_rows = 1;
    conn.query(s).await.unwrap();
    let evs = events(&mut conn).await;
    let starts = evs
        .iter()
        .filter(|e| matches!(e, QueryEvent::ResultStart(_)))
        .count();
    assert_eq!(starts, 2);
    assert!(!evs.contains(&QueryEvent::Suspended));
    let QueryEvent::Done(done) = evs.last().unwrap() else {
        unreachable!()
    };
    assert_eq!(done.skipped_result_sets, 2);
    let _srv = server.await.unwrap();
    // The connection is in step.
    assert!(conn.next_event().await.is_err());
}

#[tokio::test]
async fn simple_protocol_takes_no_binds_and_a_later_error_ends_it() {
    let (mut conn, mut srv) = session().await;
    let binds = [int("1")];
    let mut s = simple("SELECT $1", 100);
    s.binds = &binds;
    assert_eq!(
        conn.query(s).await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    let server = tokio::spawn(async move {
        let q = srv.query_text().await;
        srv.send(
            &[
                row_description(&[("n", INT4)]),
                data_row(&[Some(b"1")]),
                command_complete("SELECT 1"),
                notice("NOTICE", "00000", "before the error"),
                error("22012"),
                ready(b'I'),
            ]
            .concat(),
        )
        .await;
        (q, srv)
    });
    conn.query(simple("SELECT 1; SELECT 1/0", 100))
        .await
        .unwrap();
    let mut seen = Vec::new();
    let err = loop {
        match conn.next_event().await {
            Ok(e) => seen.push(e),
            Err(e) => break e,
        }
    };
    let (q, _srv) = server.await.unwrap();
    // The refused query sent nothing: the first message is this one.
    assert_eq!(q, "SELECT 1; SELECT 1/0");
    assert!(matches!(seen[0], QueryEvent::ResultStart(_)));
    assert!(seen.iter().any(|e| matches!(e, QueryEvent::ResultEnd(_))));
    // The notice raised before the error comes out before it (after the first result's rows).
    let notices: Vec<&str> = seen
        .iter()
        .filter_map(|e| match e {
            QueryEvent::Notice(n) => Some(&*n.text),
            _ => None,
        })
        .collect();
    assert_eq!(notices, ["before the error"]);
    assert_eq!(seen.len(), 4, "{seen:?}");
    assert_eq!(err.kind, RawErrorKind::Db);
    assert_eq!(
        err.server.as_ref().and_then(|s| s.sqlstate.as_deref()),
        Some("22012")
    );
}
