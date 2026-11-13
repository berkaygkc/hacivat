//! The wire observer and execution against scripted servers, no Docker (Task 10; B8, B9, B23,
//! Q10, Q11, Q12).
//!
//! The first half feeds byte scripts straight into an [`Observer`]; the second half runs the
//! real driver over an in-memory pipe against a scripted PostgreSQL server (TLS included).

use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use hacivat_driver::{
    DriverConnect, Endpoint, ExecOpts, InsecureAuth, Protocol, RawConn, RawErrorKind, TxStatus,
};
use hacivat_driver_pg::{
    ChannelBindingMode, InsecureAuthRefusedMarker, NOTICE_CAP, Observer, ObserverMode, PgConn,
    RESET_SQL, SQL_PREPARED, TlsRefusedMarker, WireState, connect_conn, connect_conn_with,
};
use hacivat_net::{
    AddressClass, BoxedStream, CertPin, StreamFactory, TlsMode, TlsParams, TlsSettings,
    TlsVerified, build_tls_params, cert_sha256,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use secrecy::SecretString;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------------------------
// Backend messages.

fn frame(typ: u8, body: &[u8]) -> Vec<u8> {
    let mut m = vec![typ];
    m.extend_from_slice(&u32::try_from(body.len() + 4).unwrap().to_be_bytes());
    m.extend_from_slice(body);
    m
}

fn auth_ok() -> Vec<u8> {
    frame(b'R', &0i32.to_be_bytes())
}

/// An authentication request: 3 cleartext, 5 MD5 (with a salt), 10 SASL (SCRAM-SHA-256).
fn auth_request(code: i32) -> Vec<u8> {
    let mut body = code.to_be_bytes().to_vec();
    match code {
        5 => body.extend_from_slice(&[1, 2, 3, 4]),
        10 => body.extend_from_slice(b"SCRAM-SHA-256\0\0"),
        _ => {}
    }
    frame(b'R', &body)
}

fn ready(status: u8) -> Vec<u8> {
    frame(b'Z', &[status])
}

fn backend_key_data(pid: i32, key: i32) -> Vec<u8> {
    let mut body = pid.to_be_bytes().to_vec();
    body.extend_from_slice(&key.to_be_bytes());
    frame(b'K', &body)
}

fn command_complete(tag: &str) -> Vec<u8> {
    frame(b'C', format!("{tag}\0").as_bytes())
}

fn fields(kind: u8, code: &str) -> Vec<u8> {
    let severity = if kind == b'E' { "ERROR" } else { "WARNING" };
    let mut body = vec![];
    for (f, v) in [
        (b'S', severity),
        (b'V', severity),
        (b'C', code),
        (b'M', "scripted"),
    ] {
        body.push(f);
        body.extend_from_slice(v.as_bytes());
        body.push(0);
    }
    body.push(0);
    frame(kind, &body)
}

fn notice(code: &str) -> Vec<u8> {
    fields(b'N', code)
}

fn error(code: &str) -> Vec<u8> {
    fields(b'E', code)
}

fn row_description(names: &[&str]) -> Vec<u8> {
    let mut body = i16::try_from(names.len()).unwrap().to_be_bytes().to_vec();
    for name in names {
        body.extend_from_slice(name.as_bytes());
        body.push(0);
        body.extend_from_slice(&0i32.to_be_bytes()); // table
        body.extend_from_slice(&0i16.to_be_bytes()); // column
        body.extend_from_slice(&25i32.to_be_bytes()); // text
        body.extend_from_slice(&(-1i16).to_be_bytes());
        body.extend_from_slice(&(-1i32).to_be_bytes());
        body.extend_from_slice(&0i16.to_be_bytes());
    }
    frame(b'T', &body)
}

fn data_row(cells: &[Option<&str>]) -> Vec<u8> {
    let mut body = i16::try_from(cells.len()).unwrap().to_be_bytes().to_vec();
    for cell in cells {
        match cell {
            Some(v) => {
                body.extend_from_slice(&i32::try_from(v.len()).unwrap().to_be_bytes());
                body.extend_from_slice(v.as_bytes());
            }
            None => body.extend_from_slice(&(-1i32).to_be_bytes()),
        }
    }
    frame(b'D', &body)
}

// ---------------------------------------------------------------------------------------------
// The observer, fed by scripts.

/// A stream that hands out its queued bytes `chunk` at a time and swallows writes.
struct Script {
    data: Vec<u8>,
    pos: usize,
    chunk: usize,
}

impl AsyncRead for Script {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let n = self
            .chunk
            .min(self.data.len() - self.pos)
            .min(buf.remaining());
        let start = self.pos;
        buf.put_slice(&self.data[start..start + n]);
        self.pos += n;
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for Script {
    fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, b: &[u8]) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(b.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

struct Obs {
    o: Observer<Script>,
    /// Bytes the observer passed up so far.
    delivered: usize,
}

impl Obs {
    fn state(&self) -> &WireState {
        self.o.state()
    }
    fn tx(&self) -> TxStatus {
        self.state().tx()
    }
    fn backend_pid(&self) -> Option<i32> {
        self.state().backend_pid()
    }
    fn ready_count(&self) -> u64 {
        self.state().ready_count()
    }
    fn write(&mut self, bytes: &[u8]) {
        let mut cx = Context::from_waker(Waker::noop());
        match Pin::new(&mut self.o).poll_write(&mut cx, bytes) {
            Poll::Ready(Ok(n)) => assert_eq!(n, bytes.len()),
            other => panic!("{other:?}"),
        }
    }
}

fn make(policy: InsecureAuth, outcome: Option<TlsVerified>, mode: ObserverMode) -> Obs {
    let state = WireState::new(policy);
    state.set_outcome(outcome);
    let script = Script {
        data: vec![],
        pos: 0,
        chunk: usize::MAX,
    };
    Obs {
        o: Observer::new(script, state, mode),
        delivered: 0,
    }
}

/// A framed observer (no `SSLRequest`, or TLS with `outcome`).
fn observer(policy: InsecureAuth, outcome: Option<TlsVerified>) -> Obs {
    make(policy, outcome, ObserverMode::Framed)
}

/// The raw observer of a connection that sent an `SSLRequest`.
fn observer_ssl(policy: InsecureAuth) -> Obs {
    make(policy, None, ObserverMode::AwaitSslResponse)
}

/// Feeds `bytes` in reads of at most `chunk` bytes, as `tokio-postgres` would pull them.
fn feed_chunked(o: &mut Obs, bytes: &[u8], chunk: usize) -> io::Result<()> {
    let script = o.o.get_mut();
    script.data.extend_from_slice(bytes);
    script.chunk = chunk;
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        let s = o.o.get_ref();
        if s.pos == s.data.len() {
            return Ok(());
        }
        let mut storage = [0u8; 4096];
        let mut buf = ReadBuf::new(&mut storage);
        match Pin::new(&mut o.o).poll_read(&mut cx, &mut buf) {
            Poll::Ready(Ok(())) => o.delivered += buf.filled().len(),
            Poll::Ready(Err(e)) => {
                assert!(buf.filled().is_empty(), "a failed read passes nothing up");
                return Err(e);
            }
            Poll::Pending => unreachable!("scripts never block"),
        }
    }
}

fn feed_bytewise(o: &mut Obs, bytes: &[u8]) {
    feed_chunked(o, bytes, 1).unwrap();
}

fn feed_ok(o: &mut Obs, bytes: &[u8]) -> bool {
    feed_chunked(o, bytes, usize::MAX).is_ok()
}

fn feed_err(o: &mut Obs, bytes: &[u8]) -> io::Error {
    feed_chunked(o, bytes, usize::MAX).unwrap_err()
}

fn is_insecure_marker(e: &io::Error) -> bool {
    e.get_ref()
        .is_some_and(|inner| inner.is::<InsecureAuthRefusedMarker>())
}

#[test]
fn observer_frames_one_byte_reads() {
    let mut o = observer(InsecureAuth::Allow, None);
    feed_bytewise(&mut o, &[auth_ok(), ready(b'T')].concat());
    assert_eq!(o.tx(), TxStatus::InTx);
}

#[test]
fn observer_refuses_cleartext_and_md5_unless_verified() {
    for code in [3, 5] {
        for outcome in [None, Some(TlsVerified::No)] {
            let mut o = observer(InsecureAuth::RefuseUnlessVerified, outcome);
            let e = feed_err(&mut o, &auth_request(code));
            assert!(is_insecure_marker(&e), "{code} {outcome:?}");
        }
    }
}

#[test]
fn observer_allows_when_verified_or_allowed() {
    for outcome in [TlsVerified::Full, TlsVerified::CaOnly, TlsVerified::Pinned] {
        assert!(
            feed_ok(
                &mut observer(InsecureAuth::RefuseUnlessVerified, Some(outcome)),
                &auth_request(5)
            ),
            "{outcome:?}"
        );
    }
    assert!(feed_ok(
        &mut observer(InsecureAuth::Allow, None),
        &auth_request(3)
    ));
}

#[test]
fn observer_lets_scram_through_and_sees_ssl_response() {
    assert!(feed_ok(
        &mut observer(InsecureAuth::RefuseUnlessVerified, None),
        &auth_request(10)
    ));
    let mut o = observer_ssl(InsecureAuth::RefuseUnlessVerified);
    assert!(feed_ok(&mut o, b"N"));
    assert_eq!(o.o.mode(), ObserverMode::Framed);
    assert!(is_insecure_marker(&feed_err(&mut o, &auth_request(3))));
}

#[test]
fn failed_status_is_e() {
    let mut o = observer(InsecureAuth::Allow, None);
    feed_bytewise(&mut o, &[auth_ok(), ready(b'E')].concat());
    assert_eq!(o.tx(), TxStatus::Failed);
}

#[test]
fn observer_records_backend_key_data_pid() {
    // Q11, Q12
    let mut o = observer(InsecureAuth::Allow, None);
    feed_bytewise(
        &mut o,
        &[auth_ok(), backend_key_data(4242, 7), ready(b'I')].concat(),
    );
    assert_eq!(o.backend_pid(), Some(4242));
    assert_eq!(o.ready_count(), 1);
}

#[test]
fn a_split_password_request_never_completes_upward() {
    // Byte by byte, the reader above gets at most the request minus its last code byte: it can
    // never decode the request, so it never answers it.
    for code in [3, 5] {
        let request = auth_request(code);
        let mut o = observer(InsecureAuth::RefuseUnlessVerified, None);
        let e = feed_chunked(&mut o, &request, 1).unwrap_err();
        assert!(is_insecure_marker(&e));
        assert_eq!(o.delivered, 8, "header and three code bytes, {code}");
        // In one read, nothing of it at all.
        let mut o = observer(InsecureAuth::RefuseUnlessVerified, None);
        assert!(is_insecure_marker(&feed_err(&mut o, &request)));
        assert_eq!(o.delivered, 0);
    }
}

#[test]
fn a_password_request_after_a_fake_auth_ok_in_one_segment_is_refused() {
    let mut o = observer(InsecureAuth::RefuseUnlessVerified, None);
    let e = feed_err(&mut o, &[auth_ok(), auth_request(3)].concat());
    assert!(is_insecure_marker(&e));
    // Also later, after the session is up.
    let mut o = observer(InsecureAuth::RefuseUnlessVerified, None);
    feed_bytewise(&mut o, &[auth_ok(), ready(b'I')].concat());
    assert!(is_insecure_marker(&feed_err(&mut o, &auth_request(5))));
}

#[test]
fn lengths_outside_4_to_1_gib_are_invalid_data() {
    for len in [0u32, 3, (1 << 30) + 1, u32::MAX] {
        let mut o = observer(InsecureAuth::Allow, None);
        let mut msg = vec![b'D'];
        msg.extend_from_slice(&len.to_be_bytes());
        let e = feed_err(&mut o, &msg);
        assert_eq!(e.kind(), io::ErrorKind::InvalidData, "{len}");
    }
    // 1 GiB itself is a legal length (a header alone reads fine).
    let mut o = observer(InsecureAuth::Allow, None);
    let mut msg = vec![b'D'];
    msg.extend_from_slice(&(1u32 << 30).to_be_bytes());
    assert!(feed_ok(&mut o, &msg));
}

#[test]
fn large_data_rows_and_copy_data_never_desync_the_framer() {
    // Bodies full of bytes that look like ReadyForQuery and password requests.
    let decoy = [ready(b'E'), auth_request(3)].concat();
    let body: Vec<u8> = decoy.iter().copied().cycle().take(200_000).collect();
    let script = [
        auth_ok(),
        backend_key_data(77, 1),
        ready(b'I'),
        frame(b'D', &body),
        frame(b'd', &body),
        frame(b'A', &body),
        ready(b'T'),
    ]
    .concat();
    // Seven-byte reads split every header and every decoy; byte-wise is covered above.
    for chunk in [7, 4096, usize::MAX] {
        let mut o = observer(InsecureAuth::RefuseUnlessVerified, None);
        feed_chunked(&mut o, &script, chunk).unwrap();
        assert_eq!(o.tx(), TxStatus::InTx, "{chunk}");
        assert_eq!(o.ready_count(), 2, "{chunk}");
        assert_eq!(o.backend_pid(), Some(77));
    }
}

#[test]
fn key_data_after_the_first_ready_is_ignored_and_before_auth_too() {
    let mut o = observer(InsecureAuth::Allow, None);
    feed_bytewise(
        &mut o,
        &[
            backend_key_data(1, 1),
            auth_ok(),
            ready(b'I'),
            backend_key_data(99, 9),
        ]
        .concat(),
    );
    assert_eq!(o.backend_pid(), None);
}

#[test]
fn an_unknown_status_byte_is_invalid_data() {
    let mut o = observer(InsecureAuth::Allow, None);
    let e = feed_err(&mut o, &[auth_ok(), ready(b'X')].concat());
    assert_eq!(e.kind(), io::ErrorKind::InvalidData);
    let mut o = observer(InsecureAuth::Allow, None);
    let e = feed_err(&mut o, &[auth_ok(), frame(b'Z', &[])].concat());
    assert_eq!(e.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn every_query_and_sync_is_owed_one_ready_for_query() {
    let mut o = observer(InsecureAuth::Allow, None);
    // The startup message has no type byte and is never counted.
    o.write(&[0, 0, 0, 8, 0, 3, 0, 0]);
    feed_bytewise(&mut o, &[auth_ok(), ready(b'I')].concat());
    assert!(o.state().synced());
    // A simple query, written in two pieces.
    let q = frame(b'Q', b"SELECT 1\0");
    o.write(&q[..3]);
    assert!(!o.state().synced());
    o.write(&q[3..]);
    // An extended round: Parse, Bind, Execute, Sync: one more.
    let extended = [
        frame(b'P', b"\0SELECT 1\0\0\0"),
        frame(b'B', &[0; 8]),
        frame(b'E', &[0; 5]),
        frame(b'S', &[]),
    ]
    .concat();
    o.write(&extended);
    feed_ok(&mut o, &[error("22012"), ready(b'E')].concat());
    assert!(!o.state().synced(), "the extended round is still owed");
    assert_eq!(o.tx(), TxStatus::Failed);
    feed_ok(&mut o, &ready(b'E'));
    assert!(o.state().synced());
}

#[test]
fn bytes_after_an_ssl_accept_are_never_framed() {
    // Q12: after `S` the raw observer sees TLS records, which may look like anything.
    let mut o = observer_ssl(InsecureAuth::RefuseUnlessVerified);
    assert!(feed_ok(
        &mut o,
        &[b"S".to_vec(), auth_request(3), ready(b'E')].concat()
    ));
    assert_eq!(o.o.mode(), ObserverMode::Passthrough);
    assert_eq!(o.ready_count(), 0);
    let mut header = vec![b'D'];
    header.extend_from_slice(&3u32.to_be_bytes());
    assert!(feed_ok(&mut o, &header));
}

#[test]
fn a_cancel_stream_of_a_tls_session_refuses_a_plaintext_answer() {
    let mut o = make(InsecureAuth::Allow, None, ObserverMode::AwaitSslAccept);
    let e = feed_err(&mut o, b"N");
    assert!(e.get_ref().is_some_and(|i| i.is::<TlsRefusedMarker>()));
    let mut o = make(InsecureAuth::Allow, None, ObserverMode::AwaitSslAccept);
    assert!(feed_ok(&mut o, b"S"));
    assert_eq!(o.o.mode(), ObserverMode::Passthrough);
}

#[test]
fn the_last_command_tag_is_kept_up_to_64_bytes() {
    let mut o = observer(InsecureAuth::Allow, None);
    feed_bytewise(
        &mut o,
        &[auth_ok(), ready(b'I'), command_complete("INSERT 0 5")].concat(),
    );
    assert_eq!(o.state().last_tag().as_deref(), Some("INSERT 0 5"));
    let long = "X".repeat(100);
    feed_ok(&mut o, &command_complete(&long));
    assert_eq!(o.state().last_tag().map(|t| t.len()), Some(64));
    o.state().clear_tag();
    assert_eq!(o.state().last_tag(), None);
}

// ---------------------------------------------------------------------------------------------
// The driver against a scripted server.

/// The PostgreSQL `SSLRequest` and `CancelRequest` codes.
const SSL_REQUEST: u32 = 80_877_103;
const CANCEL_REQUEST: u32 = 80_877_102;

/// A factory that opens a fresh in-memory pipe per call and hands the server end to the test.
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

fn req(factory: StreamFactory, tls: Option<TlsParams>, policy: InsecureAuth) -> DriverConnect {
    DriverConnect {
        endpoint: Endpoint::Network {
            factory,
            direct: None,
            resolved: vec![],
            tls_host: "localhost".into(),
        },
        tls,
        user: Some("hv_test".into()),
        password: Some(SecretString::from("s3cret")),
        database: Some("hv_test".into()),
        application_name: "Hacivat".into(),
        read_only: false,
        options: vec![],
        connect_timeout: Duration::from_secs(5),
        events: Arc::new(|_| {}),
        insecure_auth: policy,
    }
}

fn server_cert() -> CertificateDer<'static> {
    CertificateDer::from(include_bytes!("fixtures/server.der").to_vec())
}

/// Require (no verification: outcome `No`), or with a pin of the server's leaf (`Pinned`).
fn tls_params(pinned: bool) -> TlsParams {
    let pins = if pinned {
        vec![CertPin {
            endpoint: "localhost:5432".into(),
            sha256: cert_sha256(&server_cert()),
        }]
    } else {
        vec![]
    };
    let settings = TlsSettings {
        mode: TlsMode::Require,
        pins,
        ..TlsSettings::default()
    };
    build_tls_params(&settings, "localhost", 5432, AddressClass::Private, None)
        .unwrap()
        .unwrap()
}

fn acceptor() -> tokio_rustls::TlsAcceptor {
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        include_bytes!("fixtures/server-key.der").to_vec(),
    ));
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![server_cert()], key)
    .unwrap();
    // A cancel client hangs up right after its request: no tickets to write after the
    // handshake.
    config.send_tls13_tickets = 0;
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

/// The server side of one stream: plaintext or TLS.
trait Wire: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Wire for T {}

/// Reads one length-prefixed message without a type byte (`SSLRequest`, startup, cancel).
async fn read_untyped(s: &mut (impl Wire + ?Sized)) -> Vec<u8> {
    let len = s.read_u32().await.unwrap() as usize;
    let mut body = vec![0; len - 4];
    s.read_exact(&mut body).await.unwrap();
    body
}

async fn read_typed(s: &mut (impl Wire + ?Sized)) -> (u8, Vec<u8>) {
    let typ = s.read_u8().await.unwrap();
    let len = s.read_u32().await.unwrap() as usize;
    let mut body = vec![0; len - 4];
    s.read_exact(&mut body).await.unwrap();
    (typ, body)
}

/// Reads the next `Query` and returns its text.
async fn read_query(s: &mut (impl Wire + ?Sized)) -> String {
    let (typ, body) = read_typed(s).await;
    assert_eq!(typ, b'Q');
    String::from_utf8(body[..body.len() - 1].to_vec()).unwrap()
}

/// Answers the `SSLRequest` with `S` and runs the TLS handshake.
async fn accept_tls(mut s: DuplexStream) -> Box<dyn Wire> {
    let req = read_untyped(&mut s).await;
    assert_eq!(
        u32::from_be_bytes(req[..4].try_into().unwrap()),
        SSL_REQUEST
    );
    s.write_all(b"S").await.unwrap();
    Box::new(acceptor().accept(s).await.unwrap())
}

/// Takes the startup message and signs the client in without a password.
async fn sign_in(s: &mut (impl Wire + ?Sized), pid: i32, key: i32) {
    read_untyped(s).await;
    s.write_all(&[auth_ok(), backend_key_data(pid, key), ready(b'I')].concat())
        .await
        .unwrap();
}

/// A plaintext session (no TLS parameters: `sslmode=disable`) with a scripted server.
async fn plaintext_session() -> (PgConn, DuplexStream) {
    let (factory, mut streams) = pipes();
    let server = tokio::spawn(async move {
        let mut s = streams.recv().await.unwrap();
        sign_in(&mut s, 31, 1).await;
        s
    });
    let conn = connect_conn(req(factory, None, InsecureAuth::Allow))
        .await
        .unwrap();
    (conn, server.await.unwrap())
}

fn simple(max_rows: usize) -> ExecOpts {
    ExecOpts {
        max_rows,
        protocol: Protocol::Simple,
    }
}

#[tokio::test]
async fn observer_on_tls_path_sees_decrypted_messages() {
    // TLS over a pipe: an R(5) after the handshake is refused (Q12), and no password follows.
    let (factory, mut streams) = pipes();
    let server = tokio::spawn(async move {
        let mut tls = accept_tls(streams.recv().await.unwrap()).await;
        read_untyped(&mut tls).await;
        tls.write_all(&auth_request(5)).await.unwrap();
        let mut rest = vec![];
        let _ = tls.read_to_end(&mut rest).await;
        rest
    });
    let r = req(
        factory,
        Some(tls_params(false)),
        InsecureAuth::RefuseUnlessVerified,
    );
    let e = connect_conn(r).await.err().unwrap();
    assert_eq!(e.kind, RawErrorKind::InsecureAuthRefused);
    assert!(server.await.unwrap().is_empty(), "no password message");
}

#[tokio::test]
async fn a_pinned_tls_session_answers_md5() {
    // Q20: outcome-based. Require with a pin is `Pinned`, so the password may go out.
    let (factory, mut streams) = pipes();
    let server = tokio::spawn(async move {
        let mut tls = accept_tls(streams.recv().await.unwrap()).await;
        read_untyped(&mut tls).await;
        tls.write_all(&auth_request(5)).await.unwrap();
        let (typ, _) = read_typed(&mut tls).await;
        tls.write_all(&error("28P01")).await.unwrap();
        typ
    });
    let r = req(
        factory,
        Some(tls_params(true)),
        InsecureAuth::RefuseUnlessVerified,
    );
    let e = connect_conn(r).await.err().unwrap();
    assert_eq!(e.kind, RawErrorKind::Db);
    assert_eq!(server.await.unwrap(), b'p');
}

#[tokio::test]
async fn scram_binds_to_the_tls_server_end_point() {
    // B9: the client picks SCRAM-SHA-256-PLUS with tls-server-end-point when offered.
    let (factory, mut streams) = pipes();
    let server = tokio::spawn(async move {
        let mut tls = accept_tls(streams.recv().await.unwrap()).await;
        read_untyped(&mut tls).await;
        let mut body = 10i32.to_be_bytes().to_vec();
        body.extend_from_slice(b"SCRAM-SHA-256-PLUS\0SCRAM-SHA-256\0\0");
        tls.write_all(&frame(b'R', &body)).await.unwrap();
        let (typ, body) = read_typed(&mut tls).await;
        assert_eq!(typ, b'p');
        let end = body.iter().position(|b| *b == 0).unwrap();
        let mechanism = String::from_utf8(body[..end].to_vec()).unwrap();
        let first = String::from_utf8(body[end + 5..].to_vec()).unwrap();
        (mechanism, first)
    });
    let r = req(factory, Some(tls_params(false)), InsecureAuth::Allow);
    let e = connect_conn_with(r, |c| {
        c.channel_binding(ChannelBindingMode::Require);
    })
    .await;
    assert!(
        e.is_err(),
        "the scripted server stops after the first message"
    );
    let (mechanism, first) = server.await.unwrap();
    assert_eq!(mechanism, "SCRAM-SHA-256-PLUS");
    assert!(first.starts_with("p=tls-server-end-point,"), "{first}");
}

#[tokio::test]
async fn cancel_goes_over_tls_with_the_session_key() {
    // B23: a new stream from the same factory, TLS like the session, PID and key from
    // BackendKeyData.
    let (factory, mut streams) = pipes();
    let (keep_tx, keep_rx) = tokio::sync::oneshot::channel::<()>();
    let session = tokio::spawn(async move {
        let mut tls = accept_tls(streams.recv().await.unwrap()).await;
        sign_in(&mut tls, 4242, 7).await;
        let mut cancel = accept_tls(streams.recv().await.unwrap()).await;
        let body = read_untyped(&mut cancel).await;
        let _ = keep_rx.await;
        drop(tls);
        body
    });
    let conn = connect_conn(req(factory, Some(tls_params(false)), InsecureAuth::Allow))
        .await
        .unwrap();
    assert_eq!(conn.backend_id(), Some(4242));
    let handle = conn.cancel_handle().unwrap();
    handle.cancel().await.unwrap();
    let _ = keep_tx.send(());
    let body = session.await.unwrap();
    let word = |i: usize| u32::from_be_bytes(body[i..i + 4].try_into().unwrap());
    assert_eq!(word(0), CANCEL_REQUEST);
    assert_eq!(word(4), 4242);
    assert_eq!(word(8), 7);
}

#[tokio::test]
async fn tx_status_is_exact_right_after_an_error() {
    // Q12: tokio-postgres returns at ErrorResponse; the ReadyForQuery comes later.
    let (mut conn, mut s) = plaintext_session().await;
    let server = tokio::spawn(async move {
        assert_eq!(read_query(&mut s).await, "BEGIN; SELECT 1/0");
        s.write_all(&error("22012")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        s.write_all(&ready(b'E')).await.unwrap();
        s
    });
    let e = conn.batch("BEGIN; SELECT 1/0").await.unwrap_err();
    assert_eq!(
        e.server.unwrap().sqlstate.as_deref(),
        Some("22012"),
        "server code kept"
    );
    assert_eq!(conn.tx_status(), TxStatus::Failed);
    drop(server.await.unwrap());
}

#[tokio::test]
async fn notices_and_the_command_tag_reach_execute() {
    let (mut conn, mut s) = plaintext_session().await;
    let server = tokio::spawn(async move {
        assert_eq!(read_query(&mut s).await, "ROLLBACK");
        s.write_all(&[notice("25P01"), command_complete("ROLLBACK"), ready(b'I')].concat())
            .await
            .unwrap();
        // 150 notices on one statement: only the newest 100 are kept.
        assert_eq!(read_query(&mut s).await, "SELECT noisy()");
        let mut script = vec![];
        for i in 0..150 {
            script.extend(notice(&format!("{i:05}")));
        }
        script.extend([command_complete("SELECT 0"), ready(b'I')].concat());
        s.write_all(&script).await.unwrap();
        s
    });
    let r = conn.execute("ROLLBACK", simple(10)).await.unwrap();
    assert_eq!(r.notice_codes, ["25P01"]);
    assert_eq!(r.command_tag.as_deref(), Some("ROLLBACK"));
    assert_eq!(r.tx, TxStatus::Idle);
    let r = conn.execute("SELECT noisy()", simple(10)).await.unwrap();
    assert_eq!(r.notice_codes.len(), NOTICE_CAP);
    assert_eq!(r.notice_codes.last().map(String::as_str), Some("00149"));
    drop(server.await.unwrap());
}

#[tokio::test]
async fn simple_rows_are_text_kept_up_to_max_rows() {
    let (mut conn, mut s) = plaintext_session().await;
    let server = tokio::spawn(async move {
        read_query(&mut s).await;
        s.write_all(
            &[
                row_description(&["a", "b"]),
                data_row(&[Some("1"), Some("ş")]),
                data_row(&[Some("2"), None]),
                data_row(&[Some("3"), Some("c")]),
                command_complete("SELECT 3"),
                ready(b'T'),
            ]
            .concat(),
        )
        .await
        .unwrap();
        s
    });
    let r = conn.execute("SELECT a, b FROM t", simple(2)).await.unwrap();
    assert_eq!(r.columns, ["a", "b"]);
    assert_eq!(
        r.rows,
        [
            vec![Some("1".to_string()), Some("ş".to_string())],
            vec![Some("2".to_string()), None]
        ]
    );
    assert_eq!(r.row_count, 3);
    assert!(r.truncated);
    assert_eq!(r.rows_affected, Some(3));
    assert_eq!(r.command_tag.as_deref(), Some("SELECT 3"));
    assert_eq!(r.tx, TxStatus::InTx);
    drop(server.await.unwrap());
}

#[tokio::test]
async fn reset_never_deallocates_all() {
    // Q10: the DISCARD ALL sequence without DEALLOCATE ALL, then by-name deallocation.
    let (mut conn, mut s) = plaintext_session().await;
    let seen = Arc::new(Mutex::new(vec![]));
    let log = seen.clone();
    let server = tokio::spawn(async move {
        let q = read_query(&mut s).await;
        log.lock().unwrap().push(q);
        s.write_all(&ready(b'I')).await.unwrap();
        let q = read_query(&mut s).await;
        log.lock().unwrap().push(q);
        s.write_all(
            &[
                row_description(&["name"]),
                data_row(&[Some("p")]),
                data_row(&[Some("we\"ird")]),
                command_complete("SELECT 2"),
                ready(b'I'),
            ]
            .concat(),
        )
        .await
        .unwrap();
        let q = read_query(&mut s).await;
        log.lock().unwrap().push(q);
        s.write_all(&ready(b'I')).await.unwrap();
        s
    });
    conn.reset().await.unwrap();
    drop(server.await.unwrap());
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        [
            RESET_SQL.to_string(),
            SQL_PREPARED.to_string(),
            "DEALLOCATE \"p\"; DEALLOCATE \"we\"\"ird\";".to_string()
        ]
    );
    assert!(
        seen.iter()
            .all(|q| !q.to_ascii_uppercase().contains("DEALLOCATE ALL"))
    );
}
