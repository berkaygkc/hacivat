//! M3b Task 12 (C11 fallback, ledger 294; named exception in `connect.rs`): a pooler that
//! refuses the startup `options` parameter gets a read-only connect's settings by `SET` after
//! connect. A scripted in-memory server over an injected stream; no socket.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hacivat_driver::{Driver, DriverConnect, Endpoint, RawErrorKind};
use hacivat_driver_pg::PgDriver;
use hacivat_net::{BoxedStream, StreamFactory};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

const PGBOUNCER: &str = "unsupported startup parameter: options";
const SETS: &str =
    "SET default_transaction_read_only = on; SET idle_in_transaction_session_timeout = '30s'";

/// A factory that hands out the client ends of `n` in-memory pipes, one per connect; the test
/// drives the server ends.
fn pipes(n: usize) -> (StreamFactory, Vec<DuplexStream>) {
    let mut clients = VecDeque::new();
    let mut servers = Vec::new();
    for _ in 0..n {
        let (client, server) = tokio::io::duplex(1 << 16);
        clients.push_back(client);
        servers.push(server);
    }
    let slot = Arc::new(Mutex::new(clients));
    let factory = StreamFactory::new(move || {
        let s = slot.lock().unwrap().pop_front();
        Box::pin(async move {
            s.map(|s| Box::new(s) as BoxedStream)
                .ok_or_else(|| io::Error::from(io::ErrorKind::ConnectionRefused))
        })
    });
    (factory, servers)
}

fn request(
    factory: StreamFactory,
    read_only: bool,
    options: Vec<(String, String)>,
) -> DriverConnect {
    DriverConnect {
        endpoint: Endpoint::Network {
            factory,
            direct: None,
            resolved: vec![],
            tls_host: "127.0.0.1".into(),
        },
        tls: None,
        user: Some("hv_test".into()),
        password: None,
        database: Some("hv_test".into()),
        application_name: "Hacivat".into(),
        read_only,
        options,
        connect_timeout: Duration::from_secs(5),
        events: Arc::new(|_| {}),
        insecure_auth: Default::default(),
    }
}

/// Reads one length-prefixed untyped message (the StartupMessage).
async fn read_untyped(s: &mut DuplexStream) -> Vec<u8> {
    let len = s.read_u32().await.unwrap() as usize;
    let mut body = vec![0; len - 4];
    s.read_exact(&mut body).await.unwrap();
    body
}

/// Reads one typed message: its tag and body.
async fn read_typed(s: &mut DuplexStream) -> (u8, Vec<u8>) {
    let tag = s.read_u8().await.unwrap();
    let len = s.read_u32().await.unwrap() as usize;
    let mut body = vec![0; len - 4];
    s.read_exact(&mut body).await.unwrap();
    (tag, body)
}

/// The `key\0value\0` pairs of a StartupMessage body (after the protocol version).
fn startup_params(body: &[u8]) -> Vec<(String, String)> {
    let mut parts = body[4..]
        .split(|b| *b == 0)
        .map(|p| String::from_utf8(p.to_vec()).unwrap());
    let mut out = vec![];
    while let (Some(k), Some(v)) = (parts.next(), parts.next()) {
        if k.is_empty() {
            break;
        }
        out.push((k, v));
    }
    out
}

fn message(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut msg = vec![tag];
    msg.extend_from_slice(&u32::try_from(body.len() + 4).unwrap().to_be_bytes());
    msg.extend_from_slice(body);
    msg
}

fn error_response(code: &str, text: &str) -> Vec<u8> {
    let mut body = vec![];
    for (field, value) in [(b'S', "FATAL"), (b'V', "FATAL"), (b'C', code), (b'M', text)] {
        body.push(field);
        body.extend_from_slice(value.as_bytes());
        body.push(0);
    }
    body.push(0);
    message(b'E', &body)
}

/// Sign-in without a password, the backend key, and ready.
async fn accept(s: &mut DuplexStream) {
    s.write_all(&message(b'R', &0u32.to_be_bytes()))
        .await
        .unwrap();
    let mut key = 4242u32.to_be_bytes().to_vec();
    key.extend_from_slice(&7u32.to_be_bytes());
    s.write_all(&message(b'K', &key)).await.unwrap();
    s.write_all(&message(b'Z', b"I")).await.unwrap();
}

/// Answers one simple query with one `SET` completion per statement in it.
async fn answer_query(s: &mut DuplexStream) -> String {
    let (tag, body) = read_typed(s).await;
    assert_eq!(tag, b'Q');
    let sql = String::from_utf8(body[..body.len() - 1].to_vec()).unwrap();
    for _ in sql.split(';') {
        s.write_all(&message(b'C', b"SET\0")).await.unwrap();
    }
    s.write_all(&message(b'Z', b"I")).await.unwrap();
    sql
}

fn get(params: &[(String, String)], key: &str) -> Option<String> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

#[tokio::test]
async fn startup_option_refused_falls_back_to_set() {
    let (factory, mut servers) = pipes(2);
    let mut second = servers.pop().unwrap();
    let mut first = servers.pop().unwrap();
    let server = tokio::spawn(async move {
        let refused = startup_params(&read_untyped(&mut first).await);
        first
            .write_all(&error_response("08P01", PGBOUNCER))
            .await
            .unwrap();
        let retried = startup_params(&read_untyped(&mut second).await);
        accept(&mut second).await;
        let sql = answer_query(&mut second).await;
        // ## m3b-t18-safe06 (named exception): then the read-only connect's `search_path`
        // read, last (answered without a row here: the setting stays unread).
        let read = answer_query(&mut second).await;
        assert_eq!(read, "SELECT pg_catalog.current_setting('search_path')");
        (refused, retried, sql, second)
    });
    let conn = PgDriver::new()
        .connect(request(factory, true, Vec::new()))
        .await;
    let (refused, retried, sql, _keep) = server.await.unwrap();
    assert!(conn.is_ok(), "{:?}", conn.err());
    assert_eq!(
        get(&refused, "options").as_deref(),
        Some("-c default_transaction_read_only=on -c idle_in_transaction_session_timeout=30s")
    );
    // Once more without the parameter, the rest of the startup as it was.
    assert_eq!(get(&retried, "options"), None);
    assert_eq!(get(&retried, "user").as_deref(), Some("hv_test"));
    assert_eq!(
        get(&retried, "application_name").as_deref(),
        Some("Hacivat")
    );
    // The same settings, by `SET`, before the connection is handed out.
    assert_eq!(sql, SETS);
}

#[tokio::test]
async fn a_failed_set_after_the_fallback_fails_the_connect() {
    let (factory, mut servers) = pipes(2);
    let mut second = servers.pop().unwrap();
    let mut first = servers.pop().unwrap();
    let server = tokio::spawn(async move {
        read_untyped(&mut first).await;
        first
            .write_all(&error_response("08P01", PGBOUNCER))
            .await
            .unwrap();
        read_untyped(&mut second).await;
        accept(&mut second).await;
        let (tag, _) = read_typed(&mut second).await;
        assert_eq!(tag, b'Q');
        second
            .write_all(&error_response(
                "42501",
                "permission denied to set parameter",
            ))
            .await
            .unwrap();
        second.write_all(&message(b'Z', b"I")).await.unwrap();
        second
    });
    let conn = PgDriver::new()
        .connect(request(factory, true, Vec::new()))
        .await;
    let _keep = server.await.unwrap();
    let e = conn.err().expect("the connect fails");
    assert_eq!(e.kind, RawErrorKind::Db);
}

/// No retry where it would lose something or does not apply: a read-write connect, a profile's
/// own `options` beside Hacivat's, or another refusal.
#[tokio::test]
async fn other_refusals_are_not_retried() {
    // (read-only, the profile's options, the server's code, its text)
    type Case = (bool, Vec<(String, String)>, &'static str, &'static str);
    let cases: Vec<Case> = vec![
        (
            false,
            vec![("extra_float_digits".into(), "3".into())],
            "08P01",
            PGBOUNCER,
        ),
        (
            true,
            vec![("options".into(), "-c search_path=app".into())],
            "08P01",
            PGBOUNCER,
        ),
        (true, Vec::new(), "28P01", "password authentication failed"),
        (true, Vec::new(), "08P01", "invalid startup packet layout"),
    ];
    for (read_only, options, code, text) in cases {
        let (factory, mut servers) = pipes(1);
        let mut first = servers.pop().unwrap();
        let (code_s, text_s) = (code.to_string(), text.to_string());
        let server = tokio::spawn(async move {
            read_untyped(&mut first).await;
            first
                .write_all(&error_response(&code_s, &text_s))
                .await
                .unwrap();
            first
        });
        let conn = PgDriver::new()
            .connect(request(factory, read_only, options.clone()))
            .await;
        let _keep = server.await.unwrap();
        let e = conn.err().expect("the connect fails");
        assert_eq!(e.kind, RawErrorKind::Db, "{read_only} {options:?} {code}");
        assert_eq!(
            e.server.and_then(|m| m.sqlstate).as_deref(),
            Some(code),
            "the server's own error, not a second attempt's"
        );
    }
}
