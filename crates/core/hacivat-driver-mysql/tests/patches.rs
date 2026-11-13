//! Tests for the vendored `mysql_async` patches (PATCHES.md §1-§5). No Docker: §5 runs against an
//! in-process server that speaks only the MySQL greeting and the TLS switch.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use mysql_async::{Conn, DriverError, Opts, OptsBuilder, SslOpts};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error, SignatureScheme};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;

#[test]
fn knobs_default_to_upstream_behaviour() {
    let o = Opts::from(OptsBuilder::default());
    assert!(o.password_guard().is_none());
    assert!(o.multi_statements() && o.local_infile_capability());
    assert!(SslOpts::default().rustls_client_config().is_none());
}

#[test]
fn knobs_set() {
    let o = Opts::from(
        OptsBuilder::default()
            .password_guard(Some(Arc::new(|| false)))
            .multi_statements(false)
            .local_infile_capability(false),
    );
    assert!(!(o.password_guard().unwrap())());
    assert!(!o.multi_statements() && !o.local_infile_capability());
}

#[test]
fn insecure_password_refused_error_exists() {
    let e = DriverError::InsecurePasswordRefused;
    assert!(!format!("{e}").is_empty());
}

#[test]
fn opts_with_a_guard_compare_by_guard_identity() {
    let g: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(|| true);
    let a = Opts::from(OptsBuilder::default().password_guard(Some(g.clone())));
    let b = Opts::from(OptsBuilder::default().password_guard(Some(g)));
    let c = Opts::from(OptsBuilder::default().password_guard(Some(Arc::new(|| true))));
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// Greeting server -------------------------------------------------------------------------

struct Server {
    port: u16,
    cert_pem: String,
    sni: oneshot::Receiver<Option<String>>,
}

fn greeting(caps: u32) -> Vec<u8> {
    let mut p = vec![10u8];
    p.extend_from_slice(b"8.4.0-hv\0");
    p.extend_from_slice(&1u32.to_le_bytes());
    p.extend_from_slice(b"abcdefgh\0");
    p.extend_from_slice(&(caps as u16).to_le_bytes());
    p.push(45);
    p.extend_from_slice(&2u16.to_le_bytes());
    p.extend_from_slice(&((caps >> 16) as u16).to_le_bytes());
    p.push(21);
    p.extend_from_slice(&[0u8; 10]);
    p.extend_from_slice(b"ijklmnopqrst\0");
    p.extend_from_slice(b"caching_sha2_password\0");
    let mut out = (p.len() as u32).to_le_bytes()[..3].to_vec();
    out.push(0);
    out.extend(p);
    out
}

async fn greeting_tls_server(name: &str) -> Server {
    let ck = rcgen::generate_simple_self_signed(vec![name.to_string()]).unwrap();
    let cert_pem = ck.cert.pem();
    let cert = CertificateDer::from(ck.cert.der().to_vec());
    let key = PrivateKeyDer::try_from(ck.signing_key.serialize_der()).unwrap();
    let cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(cfg));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, sni) = oneshot::channel();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        // CLIENT_PROTOCOL_41 | SECURE_CONNECTION | SSL | PLUGIN_AUTH | DEPRECATE_EOF
        let caps = 0x0200 | 0x8000 | 0x0800 | 0x0008_0000 | 0x0100_0000;
        s.write_all(&greeting(caps)).await.unwrap();
        let mut hdr = [0u8; 4];
        s.read_exact(&mut hdr).await.unwrap();
        let len = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], 0]) as usize;
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).await.unwrap();
        match acceptor.accept(s).await {
            Ok(tls) => {
                let _ = tx.send(tls.get_ref().1.server_name().map(str::to_owned));
            }
            Err(_) => {
                let _ = tx.send(None);
            }
        }
    });
    Server {
        port,
        cert_pem,
        sni,
    }
}

#[derive(Debug)]
struct Counting(Arc<AtomicUsize>);

impl ServerCertVerifier for Counting {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(
            m,
            c,
            d,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            m,
            c,
            d,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[tokio::test]
async fn custom_client_config_is_used() {
    let srv = greeting_tls_server("db").await;
    let calls = Arc::new(AtomicUsize::new(0));
    let cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Counting(calls.clone())))
        .with_no_client_auth();
    let opts = OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(srv.port)
        .prefer_socket(false)
        .ssl_opts(
            SslOpts::default()
                .with_rustls_client_config(Arc::new(cfg))
                .with_danger_tls_hostname_override(Some("db")),
        );
    let _ = Conn::new(opts).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(srv.sni.await.unwrap(), Some("db".to_string()));
}

// Plain server for §2/§3/§4 -----------------------------------------------------------------

struct Plain {
    port: u16,
    /// (client capability flags, bytes the client sent after the auth-more-data packet)
    seen: oneshot::Receiver<(u32, Vec<u8>)>,
}

async fn plain_full_auth_server() -> Plain {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, seen) = oneshot::channel();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        // The TLS greeting's capability set, without SSL.
        let caps = 0x0200
            | 0x8000
            | 0x0008_0000
            | 0x0100_0000
            | CLIENT_LOCAL_FILES
            | CLIENT_MULTI_STATEMENTS;
        s.write_all(&greeting(caps)).await.unwrap();
        let mut hdr = [0u8; 4];
        s.read_exact(&mut hdr).await.unwrap();
        let len = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], 0]) as usize;
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).await.unwrap();
        let client_caps = u32::from_le_bytes(body[..4].try_into().unwrap());
        // auth more data: "perform full authentication"
        s.write_all(&[2, 0, 0, 2, 0x01, 0x04]).await.unwrap();
        let mut rest = vec![0u8; 64];
        let n = s.read(&mut rest).await.unwrap_or(0);
        rest.truncate(n);
        let _ = tx.send((client_caps, rest));
    });
    Plain { port, seen }
}

fn plain_opts(port: u16) -> OptsBuilder {
    OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(port)
        .prefer_socket(false)
        .user(Some("hv"))
        .pass(Some("hv_test_pw"))
}

const CLIENT_LOCAL_FILES: u32 = 0x80;
const CLIENT_MULTI_STATEMENTS: u32 = 1 << 16;

#[tokio::test]
async fn password_guard_false_refuses_before_any_password_byte() {
    let srv = plain_full_auth_server().await;
    let err = Conn::new(plain_opts(srv.port).password_guard(Some(Arc::new(|| false))))
        .await
        .err()
        .unwrap();
    assert!(
        matches!(
            err,
            mysql_async::Error::Driver(DriverError::InsecurePasswordRefused)
        ),
        "unexpected error kind"
    );
    let (_, after) = srv.seen.await.unwrap();
    assert!(after.is_empty(), "client sent bytes after the refusal");
}

#[tokio::test]
async fn password_guard_true_or_absent_keeps_upstream_flow() {
    for guard in [
        None,
        Some(Arc::new(|| true) as Arc<dyn Fn() -> bool + Send + Sync>),
    ] {
        let srv = plain_full_auth_server().await;
        let _ = Conn::new(plain_opts(srv.port).password_guard(guard)).await;
        let (_, after) = srv.seen.await.unwrap();
        // Upstream asks for the server's RSA key (0x02) over a non-TLS link.
        assert_eq!(after.get(4), Some(&0x02));
    }
}

#[tokio::test]
async fn capability_knobs_shape_the_handshake_response() {
    let srv = plain_full_auth_server().await;
    let _ = Conn::new(plain_opts(srv.port)).await;
    let (caps, _) = srv.seen.await.unwrap();
    assert!(caps & CLIENT_LOCAL_FILES != 0 && caps & CLIENT_MULTI_STATEMENTS != 0);

    let srv = plain_full_auth_server().await;
    let _ = Conn::new(
        plain_opts(srv.port)
            .multi_statements(false)
            .local_infile_capability(false),
    )
    .await;
    let (caps, _) = srv.seen.await.unwrap();
    assert!(caps & CLIENT_LOCAL_FILES == 0 && caps & CLIENT_MULTI_STATEMENTS == 0);
}

#[tokio::test]
async fn skip_domain_validation_matches_the_name_error_by_variant() {
    // The server certificate is trusted but issued for "db"; the connection asks for "other".
    let srv = greeting_tls_server("db").await;
    let opts = OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(srv.port)
        .prefer_socket(false)
        .ssl_opts(
            SslOpts::default()
                .with_disable_built_in_roots(true)
                .with_root_certs(vec![srv.cert_pem.clone().into_bytes().into()])
                .with_danger_skip_domain_validation(true)
                .with_danger_tls_hostname_override(Some("other")),
        );
    let _ = Conn::new(opts).await;
    assert_eq!(srv.sni.await.unwrap(), Some("other".to_string()));
}
