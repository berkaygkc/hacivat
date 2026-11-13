//! PostgreSQL driver tests that need no server (Task 13): sslmode and startup options, version
//! detection, and the handshake against a scripted in-memory server over an injected stream.

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hacivat_driver::{
    Driver, DriverConnect, DriverEvent, Endpoint, EngineCatalog, RawErrorKind, ServerInfo, Support,
    support_status,
};
use hacivat_driver_pg::{PgDriver, SslMode, config_for, server_version, ssl_mode};
use hacivat_net::{
    AddressClass, BoxedStream, StreamFactory, TlsFailure, TlsMode, TlsParams, TlsSettings,
    build_tls_params,
};
use hacivat_proto::DbmsKind;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

/// The PostgreSQL `SSLRequest` code (protocol 1234.5679).
const SSL_REQUEST: u32 = 80_877_103;

fn noop_factory() -> StreamFactory {
    StreamFactory::new(|| Box::pin(async { Err(io::Error::from(io::ErrorKind::NotConnected)) }))
}

fn base_req(factory: StreamFactory) -> DriverConnect {
    DriverConnect {
        endpoint: Endpoint::Network {
            factory,
            direct: None,
            resolved: vec![],
            tls_host: "127.0.0.1".into(),
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
    }
}

fn req_ro(app: &str) -> DriverConnect {
    let mut r = base_req(noop_factory());
    r.application_name = app.into();
    r.read_only = true;
    r
}

fn tls(mode: TlsMode, class: AddressClass) -> Option<TlsParams> {
    build_tls_params(
        &TlsSettings {
            mode,
            ..TlsSettings::default()
        },
        "127.0.0.1",
        class,
    )
    .unwrap()
}

/// A factory that hands out one end of an in-memory pipe; the test drives the other end.
fn piped() -> (StreamFactory, DuplexStream) {
    let (client, server) = tokio::io::duplex(1 << 16);
    let slot = Arc::new(Mutex::new(Some(client)));
    let factory = StreamFactory::new(move || {
        let s = slot.lock().unwrap().take();
        Box::pin(async move {
            s.map(|s| Box::new(s) as BoxedStream)
                .ok_or_else(|| io::Error::from(io::ErrorKind::ConnectionRefused))
        })
    });
    (factory, server)
}

/// Reads one length-prefixed untyped message (SSLRequest, StartupMessage).
async fn read_untyped(s: &mut DuplexStream) -> Vec<u8> {
    let len = s.read_u32().await.unwrap() as usize;
    let mut body = vec![0; len - 4];
    s.read_exact(&mut body).await.unwrap();
    body
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

/// An `ErrorResponse` with the given fields.
fn error_response(fields: &[(u8, &str)]) -> Vec<u8> {
    let mut body = vec![];
    for (code, text) in fields {
        body.push(*code);
        body.extend_from_slice(text.as_bytes());
        body.push(0);
    }
    body.push(0);
    let mut msg = vec![b'E'];
    msg.extend_from_slice(&u32::try_from(body.len() + 4).unwrap().to_be_bytes());
    msg.extend_from_slice(&body);
    msg
}

fn recorded_events() -> (hacivat_driver::DriverEvents, Arc<Mutex<Vec<DriverEvent>>>) {
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = seen.clone();
    (Arc::new(move |e| sink.lock().unwrap().push(e)), seen)
}

#[test]
fn sslmode_and_startup_options() {
    assert_eq!(ssl_mode(TlsMode::Auto, true), SslMode::Prefer);
    assert_eq!(ssl_mode(TlsMode::Auto, false), SslMode::Require);
    assert_eq!(ssl_mode(TlsMode::VerifyCa, true), SslMode::Require);
    assert_eq!(ssl_mode(TlsMode::Disable, true), SslMode::Disable);
    let c = config_for(&req_ro("hv"));
    assert_eq!(c.get_application_name(), Some("hv"));
    assert_eq!(c.get_options(), Some("-c default_transaction_read_only=on"));
}

#[test]
fn every_verifying_mode_requires_tls() {
    for mode in [TlsMode::Require, TlsMode::VerifyCa, TlsMode::VerifyFull] {
        assert_eq!(ssl_mode(mode, true), SslMode::Require, "{mode:?}");
        assert_eq!(ssl_mode(mode, false), SslMode::Require, "{mode:?}");
    }
    assert_eq!(ssl_mode(TlsMode::Disable, false), SslMode::Disable);
}

#[test]
fn config_carries_identity_and_redacts_the_password() {
    let c = config_for(&base_req(noop_factory()));
    assert_eq!(c.get_user(), Some("hv_test"));
    assert_eq!(c.get_dbname(), Some("hv_test"));
    assert_eq!(c.get_password(), Some(&b"s3cret"[..]));
    assert_eq!(c.get_options(), None);
    let shown = format!("{c:?}");
    assert!(!shown.contains("s3cret"), "{shown}");
}

#[test]
fn extras_join_the_options_and_read_only_comes_last() {
    let mut r = req_ro("hv");
    r.options = vec![
        (
            "options".into(),
            "-c search_path=app -c default_transaction_read_only=off".into(),
        ),
        ("extra_float_digits".into(), "3".into()),
        // libpq client-side keys are the pipeline's, never startup options.
        ("sslmode".into(), "disable".into()),
        ("connect_timeout".into(), "1".into()),
        ("application_name".into(), "other".into()),
    ];
    let c = config_for(&r);
    assert_eq!(
        c.get_options(),
        Some(
            "-c search_path=app -c default_transaction_read_only=off -c extra_float_digits=3 \
             -c default_transaction_read_only=on"
        )
    );
    assert_eq!(c.get_application_name(), Some("hv"));
}

#[test]
fn a_dangling_backslash_cannot_swallow_the_read_only_option() {
    let mut r = req_ro("hv");
    r.options = vec![("options".into(), "-c search_path=app\\".into())];
    assert_eq!(
        config_for(&r).get_options(),
        Some("-c search_path=app -c default_transaction_read_only=on")
    );
    let mut r = req_ro("hv");
    r.options = vec![("options".into(), "-c search_path=a\\\\".into())];
    assert_eq!(
        config_for(&r).get_options(),
        Some("-c search_path=a\\\\ -c default_transaction_read_only=on")
    );
}

#[test]
fn generated_values_escape_spaces_and_backslashes() {
    let mut r = base_req(noop_factory());
    r.options = vec![("extra_float_digits".into(), "1 \\x".into())];
    assert_eq!(
        config_for(&r).get_options(),
        Some("-c extra_float_digits=1\\ \\\\x")
    );
}

fn info(num: Option<&str>, version: &str) -> ServerInfo {
    ServerInfo {
        kind: DbmsKind::Postgres,
        product: "PostgreSQL".into(),
        version: server_version(num, version),
        raw_version: version.into(),
        is_replica: Some(false),
    }
}

#[test]
fn versions_come_from_server_version_num() {
    let v = server_version(
        Some("170011"),
        "PostgreSQL 17.11 (Debian 17.11-1.pgdg13+1) on aarch64-unknown-linux-gnu",
    )
    .unwrap();
    assert_eq!((v.major, v.minor, v.patch), (17, 11, 0));
    assert!(v.raw.starts_with("PostgreSQL 17.11"));
    let old = server_version(Some("90624"), "PostgreSQL 9.6.24 on x86_64").unwrap();
    assert_eq!((old.major, old.minor, old.patch), (9, 6, 24));
    // Without a usable number, the text is parsed loosely.
    let loose = server_version(None, "PostgreSQL 16.4 on x86_64").unwrap();
    assert_eq!((loose.major, loose.minor), (16, 4));
    let loose = server_version(Some("n/a"), "PostgreSQL 15.2").unwrap();
    assert_eq!((loose.major, loose.minor), (15, 2));
    assert_eq!(server_version(None, "no digits"), None);
}

#[test]
fn pg_pre_releases_are_not_tested() {
    let cat = EngineCatalog::embedded();
    for text in [
        "PostgreSQL 17beta1 on x86_64-pc-linux-gnu",
        "PostgreSQL 18devel on x86_64-pc-linux-gnu",
        "PostgreSQL 16rc1 on x86_64-pc-linux-gnu",
    ] {
        let num = if text.contains("18") {
            "180000"
        } else {
            "170000"
        };
        assert_eq!(
            support_status(cat, &info(Some(num), text)),
            Support::NotTested,
            "{text}"
        );
    }
    assert_eq!(
        support_status(
            cat,
            &info(Some("170011"), "PostgreSQL 17.11 (Debian 17.11-1)")
        ),
        Support::Supported
    );
    assert!(matches!(
        support_status(cat, &info(Some("130023"), "PostgreSQL 13.23")),
        Support::Unsupported { .. }
    ));
}

#[test]
fn the_descriptor_is_postgres_with_stream_injection() {
    let d = PgDriver::new();
    let desc = d.descriptor();
    assert_eq!(desc.kind, DbmsKind::Postgres);
    assert!(desc.caps.stream_injection && desc.caps.in_protocol_tls);
}

#[tokio::test]
async fn auto_on_a_public_host_never_sends_the_password_in_plaintext() {
    let (factory, mut server) = piped();
    let mut r = base_req(factory);
    r.tls = tls(TlsMode::Auto, AddressClass::Public);
    let srv = tokio::spawn(async move {
        let req = read_untyped(&mut server).await;
        assert_eq!(
            u32::from_be_bytes(req[..4].try_into().unwrap()),
            SSL_REQUEST
        );
        server.write_all(b"N").await.unwrap();
        // Nothing else may arrive: the client gives up before any startup or password.
        let mut rest = vec![];
        server.read_to_end(&mut rest).await.unwrap();
        rest
    });
    let e = PgDriver::new().connect(r).await.err().unwrap();
    assert_eq!(e.kind, RawErrorKind::TlsNotOffered);
    assert!(e.server.is_none());
    assert!(srv.await.unwrap().is_empty());
}

#[tokio::test]
async fn auto_on_a_private_host_continues_in_plaintext_and_keeps_server_errors() {
    let (factory, mut server) = piped();
    let mut r = base_req(factory);
    r.tls = tls(TlsMode::Auto, AddressClass::Loopback);
    r.read_only = true;
    r.application_name = "hv-app".into();
    let srv = tokio::spawn(async move {
        let req = read_untyped(&mut server).await;
        assert_eq!(
            u32::from_be_bytes(req[..4].try_into().unwrap()),
            SSL_REQUEST
        );
        server.write_all(b"N").await.unwrap();
        let startup = read_untyped(&mut server).await;
        server
            .write_all(&error_response(&[
                (b'S', "ÖLÜMCÜL"),
                (b'V', "FATAL"),
                (b'C', "28P01"),
                (
                    b'M',
                    "\"hv_test\" kullanıcısı için şifre doğrulaması başarısız oldu",
                ),
                (b'D', "ayrıntı"),
                (b'H', "ipucu"),
            ]))
            .await
            .unwrap();
        startup_params(&startup)
    });
    let e = PgDriver::new().connect(r).await.err().unwrap();
    assert_eq!(e.kind, RawErrorKind::Db);
    let m = e.server.unwrap();
    assert_eq!(m.sqlstate.as_deref(), Some("28P01"));
    assert_eq!(m.severity.as_deref(), Some("ÖLÜMCÜL"));
    assert_eq!(
        m.text,
        "\"hv_test\" kullanıcısı için şifre doğrulaması başarısız oldu"
    );
    assert_eq!(m.detail.as_deref(), Some("ayrıntı"));
    assert_eq!(m.hint.as_deref(), Some("ipucu"));
    // A localized severity marks a non-English message, so fragile text is never read.
    assert_eq!(m.lang_hint.as_deref(), Some("und"));
    let params = srv.await.unwrap();
    let get = |k: &str| {
        params
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(get("application_name").as_deref(), Some("hv-app"));
    assert_eq!(
        get("options").as_deref(),
        Some("-c default_transaction_read_only=on")
    );
    assert_eq!(get("user").as_deref(), Some("hv_test"));
    assert_eq!(get("database").as_deref(), Some("hv_test"));
}

#[tokio::test]
async fn disable_skips_the_ssl_request() {
    let (factory, mut server) = piped();
    let r = base_req(factory);
    let srv = tokio::spawn(async move {
        let first = read_untyped(&mut server).await;
        server
            .write_all(&error_response(&[
                (b'S', "FATAL"),
                (b'V', "FATAL"),
                (b'C', "3D000"),
                (b'M', "database \"hv_test\" does not exist"),
            ]))
            .await
            .unwrap();
        first
    });
    let e = PgDriver::new().connect(r).await.err().unwrap();
    let m = e.server.unwrap();
    assert_eq!(m.sqlstate.as_deref(), Some("3D000"));
    assert_eq!(m.lang_hint, None);
    let first = srv.await.unwrap();
    // Protocol 3.0 startup, not an SSLRequest.
    assert_eq!(u32::from_be_bytes(first[..4].try_into().unwrap()), 196_608);
}

#[tokio::test]
async fn a_broken_handshake_is_a_tls_failure_and_is_recorded() {
    let (factory, mut server) = piped();
    let mut r = base_req(factory);
    let params = tls(TlsMode::Require, AddressClass::Loopback).unwrap();
    let failure = params.failure.clone();
    r.tls = Some(params);
    let (events, seen) = recorded_events();
    r.events = events;
    tokio::spawn(async move {
        read_untyped(&mut server).await;
        server.write_all(b"S").await.unwrap();
        let mut hello = [0u8; 5];
        server.read_exact(&mut hello).await.unwrap();
        server
            .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
            .await
            .unwrap();
        let mut sink = vec![];
        let _ = server.read_to_end(&mut sink).await;
    });
    let e = PgDriver::new().connect(r).await.err().unwrap();
    assert_eq!(e.kind, RawErrorKind::Tls(TlsFailure::Protocol));
    assert_eq!(*failure.lock().unwrap(), Some(TlsFailure::Protocol));
    let seen = seen.lock().unwrap().clone();
    assert!(seen.contains(&DriverEvent::TlsStarted), "{seen:?}");
    assert!(!seen.contains(&DriverEvent::TlsDone), "{seen:?}");
}

#[tokio::test]
async fn a_stale_failure_is_cleared_before_the_attempt() {
    let (factory, mut server) = piped();
    let mut r = base_req(factory);
    let params = tls(TlsMode::Auto, AddressClass::Public).unwrap();
    *params.failure.lock().unwrap() = Some(TlsFailure::Expired);
    let failure = params.failure.clone();
    r.tls = Some(params);
    tokio::spawn(async move {
        read_untyped(&mut server).await;
        server.write_all(b"N").await.unwrap();
    });
    let e = PgDriver::new().connect(r).await.err().unwrap();
    assert_eq!(e.kind, RawErrorKind::TlsNotOffered);
    assert_eq!(*failure.lock().unwrap(), None);
}

#[tokio::test]
async fn a_silent_server_times_out() {
    let (factory, mut server) = piped();
    let mut r = base_req(factory);
    r.connect_timeout = Duration::from_millis(100);
    r.tls = tls(TlsMode::Require, AddressClass::Loopback);
    let srv = tokio::spawn(async move {
        read_untyped(&mut server).await;
        tokio::time::sleep(Duration::from_secs(5)).await;
        drop(server);
    });
    let e = PgDriver::new().connect(r).await.err().unwrap();
    assert_eq!(e.kind, RawErrorKind::Timeout);
    srv.abort();
}

#[tokio::test]
async fn transport_errors_keep_their_io_kind() {
    let r = base_req(StreamFactory::new(|| {
        Box::pin(async { Err(io::Error::from(io::ErrorKind::ConnectionRefused)) })
    }));
    let e = PgDriver::new().connect(r).await.err().unwrap();
    assert_eq!(e.kind, RawErrorKind::Io(io::ErrorKind::ConnectionRefused));

    // The server hangs up in the middle of the startup.
    let (factory, mut server) = piped();
    tokio::spawn(async move {
        read_untyped(&mut server).await;
        drop(server);
    });
    let e = PgDriver::new()
        .connect(base_req(factory))
        .await
        .err()
        .unwrap();
    assert!(matches!(e.kind, RawErrorKind::Io(_)), "{:?}", e.kind);
}

#[tokio::test]
async fn a_password_request_without_a_password_is_password_required() {
    let (factory, mut server) = piped();
    let mut r = base_req(factory);
    r.password = None;
    let srv = tokio::spawn(async move {
        read_untyped(&mut server).await;
        // AuthenticationCleartextPassword: 'R', length 8, code 3.
        server
            .write_all(&[b'R', 0, 0, 0, 8, 0, 0, 0, 3])
            .await
            .unwrap();
        server
    });
    let e = PgDriver::new().connect(r).await.err().unwrap();
    assert_eq!(e.kind, RawErrorKind::PasswordRequired);
    assert!(e.server.is_none());
    drop(srv.await.unwrap());
}

#[tokio::test]
async fn a_file_endpoint_is_unsupported() {
    let mut r = base_req(noop_factory());
    r.endpoint = Endpoint::File {
        path: "db.sqlite".into(),
        create_if_missing: false,
        read_only: false,
    };
    let e = PgDriver::new().connect(r).await.err().unwrap();
    assert_eq!(e.kind, RawErrorKind::Unsupported);
}
