//! Non-interactive TLS (HV-SEC-02): modes, CA files and client certificates against loopback
//! rustls servers with rcgen certificates.

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hacivat_net::AddressClass::{Loopback, Private, Public};
use hacivat_net::{
    CertPin, ClientCertPaths, TlsFailure, TlsMode, TlsParams, TlsSettings, TlsSetupError,
    TlsVerified, build_tls_params, cert_sha256, install_crypto_provider, pin_endpoint,
};
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// Not real key material: only the PEM label and the PBES2 algorithm header of an encrypted
/// PKCS #8 key, which is what the detection reads.
const ENCRYPTED_PKCS8_PEM: &str = "-----BEGIN ENCRYPTED PRIVATE KEY-----
MIGbMFcGCSqGSIb3DQEFDTBKMCkGCSqGSIb3DQEFDDAcBAgAAAAAAAAAAAICCAAw
DAYIKoZIhvcNAgkFADAdBglghkgBZQMEASoEEAAAAAAAAAAAAAAAAAAAAAAEQAAA
-----END ENCRYPTED PRIVATE KEY-----
";

/// A legacy OpenSSL-encrypted PKCS #1 key header (no real key material).
const ENCRYPTED_PKCS1_PEM: &str = "-----BEGIN RSA PRIVATE KEY-----
Proc-Type: 4,ENCRYPTED
DEK-Info: AES-256-CBC,00000000000000000000000000000000

AAAA
-----END RSA PRIVATE KEY-----
";

// ---- certificates -------------------------------------------------------------------------

/// A validity window around now that every platform verifier accepts (short, so macOS's
/// maximum-lifetime policy never fires).
fn valid_now(params: &mut CertificateParams) {
    let epoch = rcgen::date_time_ymd(1970, 1, 1);
    let now = epoch + SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    params.not_before = now - Duration::from_secs(86_400);
    params.not_after = now + Duration::from_secs(30 * 86_400);
}

fn leaf_params(name: &str) -> CertificateParams {
    let mut p = CertificateParams::new(vec![name.to_owned()]).unwrap();
    p.distinguished_name.push(DnType::CommonName, name);
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    valid_now(&mut p);
    p
}

fn ca() -> CertifiedIssuer<'static, KeyPair> {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.distinguished_name
        .push(DnType::CommonName, "Hacivat Test CA");
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    valid_now(&mut p);
    CertifiedIssuer::self_signed(p, KeyPair::generate().unwrap()).unwrap()
}

struct Leaf {
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

fn leaf_signed_by(params: CertificateParams, issuer: &CertifiedIssuer<'_, KeyPair>) -> Leaf {
    let key = KeyPair::generate().unwrap();
    let cert = params.signed_by(&key, &**issuer).unwrap();
    Leaf {
        chain: vec![cert.der().clone()],
        key: PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    }
}

// ---- servers ------------------------------------------------------------------------------

struct Srv {
    addr: SocketAddr,
    ca_pem: String,
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Serves TLS on loopback. Each connection gets one byte: 1 if the client sent SNI, else 0,
/// then 2 if it presented a client certificate, else 0, then the server waits for EOF.
async fn serve(config: rustls::ServerConfig) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let conn = tls.get_ref().1;
                let sni = u8::from(conn.server_name().is_some());
                let client_cert = if conn.peer_certificates().is_some() {
                    2
                } else {
                    0
                };
                let _ = tls.write_all(&[sni + client_cert]).await;
                let _ = tls.flush().await;
                let mut sink = [0u8; 1];
                let _ = tls.read(&mut sink).await;
            });
        }
    });
    addr
}

fn server_config(leaf: Leaf) -> rustls::ServerConfig {
    rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(leaf.chain, leaf.key)
        .unwrap()
}

/// A server with a self-signed leaf for `name`.
async fn rcgen_server(name: &str) -> Srv {
    let key = KeyPair::generate().unwrap();
    let cert = leaf_params(name).self_signed(&key).unwrap();
    let leaf = Leaf {
        chain: vec![cert.der().clone()],
        key: PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    };
    Srv {
        addr: serve(server_config(leaf)).await,
        ca_pem: cert.pem(),
    }
}

/// A server whose leaf for `name` is signed by a fresh CA; `ca_pem` is that CA.
async fn rcgen_ca_server(name: &str) -> Srv {
    ca_server_with(leaf_params(name)).await
}

async fn ca_server_with(params: CertificateParams) -> Srv {
    let ca = ca();
    let leaf = leaf_signed_by(params, &ca);
    Srv {
        addr: serve(server_config(leaf)).await,
        ca_pem: ca.pem(),
    }
}

// ---- the client side ----------------------------------------------------------------------

#[derive(Debug)]
struct HandshakeError {
    failure: Option<TlsFailure>,
    #[expect(dead_code, reason = "shown when an assertion fails")]
    error: io::Error,
}

fn write(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

/// Runs a handshake with `mode` toward `host` and returns the server's status byte.
async fn handshake_with(
    srv: &Srv,
    settings: TlsSettings,
    host: &str,
) -> Result<u8, HandshakeError> {
    install_crypto_provider();
    let params = build_tls_params(&settings, host, srv.addr.port(), Loopback, None)
        .unwrap()
        .unwrap();
    let tcp = TcpStream::connect(srv.addr).await.unwrap();
    let connector = TlsConnector::from(params.config.clone());
    let failed = |error| HandshakeError {
        failure: *params.failure.lock().unwrap(),
        error,
    };
    let mut tls = connector
        .connect(params.server_name.clone(), tcp)
        .await
        .map_err(failed)?;
    let mut status = [0u8; 1];
    tls.read_exact(&mut status).await.map_err(failed)?;
    Ok(status[0])
}

async fn handshake(
    srv: &Srv,
    mode: TlsMode,
    ca: Option<&str>,
    host: &str,
) -> Result<u8, HandshakeError> {
    let dir = tempfile::tempdir().unwrap();
    let ca_file = ca.map(|pem| write(dir.path(), "ca.pem", pem.as_bytes()));
    let settings = TlsSettings {
        mode,
        ca_file,
        client_cert: None,
        ..Default::default()
    };
    handshake_with(srv, settings, host).await
}

fn settings(mode: TlsMode) -> TlsSettings {
    TlsSettings {
        mode,
        ..TlsSettings::default()
    }
}

fn with_ca(mode: TlsMode, ca: impl Into<PathBuf>) -> TlsSettings {
    TlsSettings {
        mode,
        ca_file: Some(ca.into()),
        client_cert: None,
        ..Default::default()
    }
}

/// Settings with a valid client certificate and `key_pem` as its key; returns the key path.
fn with_client_key(dir: &Path, key_pem: &str) -> (TlsSettings, PathBuf) {
    let key = KeyPair::generate().unwrap();
    let cert = leaf_params("client").self_signed(&key).unwrap();
    let cert = write(dir, "client.crt", cert.pem().as_bytes());
    let key = write(dir, "client.key", key_pem.as_bytes());
    let settings = TlsSettings {
        mode: TlsMode::Require,
        ca_file: None,
        client_cert: Some(ClientCertPaths {
            cert,
            key: key.clone(),
        }),
        ..Default::default()
    };
    (settings, key)
}

// ---- tests --------------------------------------------------------------------------------

#[tokio::test]
async fn tls_modes_against_a_self_signed_server() {
    let srv = rcgen_server("localhost").await; // self-signed leaf
    let err = handshake(&srv, TlsMode::VerifyFull, None, "localhost")
        .await
        .unwrap_err();
    assert_eq!(err.failure, Some(TlsFailure::UnknownIssuer), "{err:?}");
    assert!(
        handshake(&srv, TlsMode::Require, None, "localhost")
            .await
            .is_ok()
    );
    let ca_srv = rcgen_ca_server("other.example").await; // CA-signed leaf for another name
    assert!(
        handshake(
            &ca_srv,
            TlsMode::VerifyCa,
            Some(&ca_srv.ca_pem),
            "localhost"
        )
        .await
        .is_ok()
    );
    let err = handshake(
        &ca_srv,
        TlsMode::VerifyFull,
        Some(&ca_srv.ca_pem),
        "localhost",
    )
    .await
    .unwrap_err();
    assert_eq!(err.failure, Some(TlsFailure::NameMismatch), "{err:?}");
}

#[tokio::test]
async fn verify_full_accepts_the_right_name_under_the_ca_file() {
    let srv = rcgen_ca_server("localhost").await;
    assert_eq!(
        handshake(&srv, TlsMode::VerifyFull, Some(&srv.ca_pem), "localhost")
            .await
            .unwrap(),
        1,
        "a DNS name is sent as SNI"
    );
}

#[tokio::test]
async fn auto_verifies_like_verify_full() {
    let srv = rcgen_server("localhost").await;
    let err = handshake(&srv, TlsMode::Auto, None, "localhost")
        .await
        .unwrap_err();
    assert_eq!(err.failure, Some(TlsFailure::UnknownIssuer), "{err:?}");
    let ca_srv = rcgen_ca_server("localhost").await;
    assert!(
        handshake(&ca_srv, TlsMode::Auto, Some(&ca_srv.ca_pem), "localhost")
            .await
            .is_ok()
    );
}

/// A self-signed `CA:TRUE` certificate presented as the server's own (common for PostgreSQL),
/// with itself as the anchor, fails with webpki's `CaUsedAsEndEntity`, which rustls wraps in
/// `CertificateError::Other`. That is the genuine error, not a stand-in.
#[tokio::test]
async fn a_ca_certificate_served_as_the_server_certificate_is_an_untrusted_issuer() {
    install_crypto_provider();
    let mut p = leaf_params("localhost");
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let key = KeyPair::generate().unwrap();
    let cert = p.self_signed(&key).unwrap();

    // The verifier directly: the real webpki error maps to UnknownIssuer.
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.der().clone()).unwrap();
    let verifier =
        rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider())
            .build()
            .unwrap();
    let err = rustls::client::danger::ServerCertVerifier::verify_server_cert(
        &*verifier,
        cert.der(),
        &[],
        &ServerName::try_from("localhost").unwrap(),
        &[],
        rustls::pki_types::UnixTime::now(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            rustls::Error::InvalidCertificate(rustls::CertificateError::Other(_))
        ),
        "{err:?}"
    );
    assert_eq!(TlsFailure::from_rustls(&err), TlsFailure::UnknownIssuer);

    // And through the VerifyCa path of the crate.
    let leaf = Leaf {
        chain: vec![cert.der().clone()],
        key: PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    };
    let srv = Srv {
        addr: serve(server_config(leaf)).await,
        ca_pem: cert.pem(),
    };
    let err = handshake(&srv, TlsMode::VerifyCa, Some(&srv.ca_pem), "localhost")
        .await
        .unwrap_err();
    assert_eq!(err.failure, Some(TlsFailure::UnknownIssuer), "{err:?}");
}

#[tokio::test]
async fn verify_ca_still_checks_the_chain() {
    let srv = rcgen_server("localhost").await;
    let unrelated = ca().pem();
    let err = handshake(&srv, TlsMode::VerifyCa, Some(&unrelated), "localhost")
        .await
        .unwrap_err();
    assert_eq!(err.failure, Some(TlsFailure::UnknownIssuer), "{err:?}");
    // Without a CA file VerifyCa trusts nothing.
    let err = handshake(&srv, TlsMode::VerifyCa, None, "localhost")
        .await
        .unwrap_err();
    assert_eq!(err.failure, Some(TlsFailure::UnknownIssuer), "{err:?}");
}

#[tokio::test]
async fn verify_ca_reports_an_expired_leaf() {
    let mut params = leaf_params("db.internal");
    params.not_before = rcgen::date_time_ymd(2020, 1, 1);
    params.not_after = rcgen::date_time_ymd(2020, 2, 1);
    let srv = ca_server_with(params).await;
    let err = handshake(&srv, TlsMode::VerifyCa, Some(&srv.ca_pem), "db.internal")
        .await
        .unwrap_err();
    assert_eq!(err.failure, Some(TlsFailure::Expired), "{err:?}");
}

#[tokio::test]
async fn ip_literals_verify_without_sni() {
    let mut params = leaf_params("localhost");
    params.subject_alt_names = vec![rcgen::SanType::IpAddress("127.0.0.1".parse().unwrap())];
    let srv = ca_server_with(params).await;
    assert_eq!(
        handshake(&srv, TlsMode::VerifyFull, Some(&srv.ca_pem), "127.0.0.1")
            .await
            .unwrap(),
        0,
        "no SNI for an IP literal"
    );
}

#[tokio::test]
async fn require_presents_the_client_certificate() {
    install_crypto_provider();
    let ca = ca();
    let server_leaf = leaf_signed_by(leaf_params("localhost"), &ca);
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca.der().clone()).unwrap();
    let client_verifier =
        rustls::server::WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider())
            .build()
            .unwrap();
    let config = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(server_leaf.chain, server_leaf.key)
        .unwrap();
    let srv = Srv {
        addr: serve(config).await,
        ca_pem: ca.pem(),
    };

    let mut client_params = CertificateParams::new(vec!["client".to_owned()]).unwrap();
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    valid_now(&mut client_params);
    let client_key = KeyPair::generate().unwrap();
    let client_cert = client_params.signed_by(&client_key, &*ca).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let settings = TlsSettings {
        mode: TlsMode::VerifyFull,
        ca_file: Some(write(dir.path(), "ca.pem", srv.ca_pem.as_bytes())),
        client_cert: Some(ClientCertPaths {
            cert: write(dir.path(), "client.crt", client_cert.pem().as_bytes()),
            key: write(
                dir.path(),
                "client.key",
                client_key.serialize_pem().as_bytes(),
            ),
        }),
        ..Default::default()
    };
    assert_eq!(
        handshake_with(&srv, settings, "localhost").await.unwrap(),
        1 + 2,
        "SNI and a client certificate"
    );
}

#[test]
fn tls_params_shape() {
    install_crypto_provider();
    assert!(
        build_tls_params(&settings(TlsMode::Disable), "db", 5432, Public, None)
            .unwrap()
            .is_none()
    );
    assert!(
        build_tls_params(&settings(TlsMode::Auto), "localhost", 5432, Loopback, None)
            .unwrap()
            .unwrap()
            .plaintext_allowed
    );
    assert!(
        build_tls_params(&settings(TlsMode::Auto), "db.lan", 5432, Private, None)
            .unwrap()
            .unwrap()
            .plaintext_allowed
    );
    assert!(
        !build_tls_params(&settings(TlsMode::Auto), "db.acme.io", 5432, Public, None)
            .unwrap()
            .unwrap()
            .plaintext_allowed
    );
    assert!(
        !build_tls_params(
            &settings(TlsMode::Require),
            "localhost",
            5432,
            Loopback,
            None
        )
        .unwrap()
        .unwrap()
        .plaintext_allowed
    );
    let p = build_tls_params(
        &settings(TlsMode::VerifyFull),
        "127.0.0.1",
        5432,
        Loopback,
        None,
    )
    .unwrap()
    .unwrap();
    assert!(matches!(p.server_name, ServerName::IpAddress(_)));
    let p = build_tls_params(
        &settings(TlsMode::VerifyFull),
        "[::1]",
        5432,
        Loopback,
        None,
    )
    .unwrap()
    .unwrap();
    assert!(matches!(p.server_name, ServerName::IpAddress(_)));
    let p = build_tls_params(
        &settings(TlsMode::VerifyFull),
        "db.acme.io",
        5432,
        Public,
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(p.server_name.to_str(), "db.acme.io");
    assert_eq!(p.settings, settings(TlsMode::VerifyFull));
    assert_eq!(*p.failure.lock().unwrap(), None);
    assert_eq!(
        build_tls_params(
            &with_ca(TlsMode::VerifyCa, "/nope.pem"),
            "db",
            5432,
            Public,
            None
        )
        .unwrap_err(),
        TlsSetupError::CaFileUnreadable("/nope.pem".into())
    );
    let dir = tempfile::tempdir().unwrap();
    let (s, key) = with_client_key(dir.path(), ENCRYPTED_PKCS8_PEM);
    assert_eq!(
        build_tls_params(&s, "db", 5432, Public, None).unwrap_err(),
        TlsSetupError::KeyEncrypted(key)
    );
}

#[tokio::test]
async fn tls_1_2_and_1_3_servers_both_handshake_with_ring() {
    for version in [&rustls::version::TLS12, &rustls::version::TLS13] {
        let key = KeyPair::generate().unwrap();
        let cert = leaf_params("localhost").self_signed(&key).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[version])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
            )
            .unwrap();
        let srv = Srv {
            addr: serve(config).await,
            ca_pem: cert.pem(),
        };
        assert!(
            handshake(&srv, TlsMode::Require, None, "localhost")
                .await
                .is_ok(),
            "{:?}",
            version.version
        );
    }
    let p = build_tls_params(&settings(TlsMode::Require), "db", 5432, Public, None)
        .unwrap()
        .unwrap();
    let suites = |p: &rustls::crypto::CryptoProvider| -> Vec<rustls::CipherSuite> {
        p.cipher_suites.iter().map(|s| s.suite()).collect()
    };
    assert_eq!(
        suites(p.config.crypto_provider()),
        suites(&rustls::crypto::ring::default_provider())
    );
}

#[test]
fn tls_file_problems_are_reported_before_connecting() {
    install_crypto_provider();
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();

    // m2b-t03 (B25): a legacy OpenSSL-encrypted key is its own refusal, not "encrypted".
    let (s, key) = with_client_key(d, ENCRYPTED_PKCS1_PEM);
    assert_eq!(
        build_tls_params(&s, "db", 5432, Public, None).unwrap_err(),
        TlsSetupError::KeyEncryptedLegacy(key)
    );
    let (s, key) = with_client_key(d, "not a key\n");
    assert_eq!(
        build_tls_params(&s, "db", 5432, Public, None).unwrap_err(),
        TlsSetupError::KeyUnreadable(key)
    );
    let (mut s, key) = with_client_key(d, "");
    std::fs::remove_file(&key).unwrap();
    assert_eq!(
        build_tls_params(&s, "db", 5432, Public, None).unwrap_err(),
        TlsSetupError::KeyUnreadable(key.clone())
    );
    let missing_cert = d.join("missing.crt");
    s.client_cert.as_mut().unwrap().cert = missing_cert.clone();
    assert_eq!(
        build_tls_params(&s, "db", 5432, Public, None).unwrap_err(),
        TlsSetupError::CertFileUnreadable(missing_cert)
    );
    let empty_ca = write(d, "empty.pem", b"# no certificates here\n");
    assert_eq!(
        build_tls_params(
            &with_ca(TlsMode::VerifyFull, &empty_ca),
            "db",
            5432,
            Public,
            None
        )
        .unwrap_err(),
        TlsSetupError::CaFileUnreadable(empty_ca)
    );
    // A DER CA file works as well as PEM.
    let der_ca = write(d, "ca.der", ca().der());
    assert!(
        build_tls_params(
            &with_ca(TlsMode::VerifyCa, &der_ca),
            "db",
            5432,
            Public,
            None
        )
        .unwrap()
        .is_some()
    );
}

/// Stands in for webpki's `CaUsedAsEndEntity`, which rustls wraps in `CertificateError::Other`;
/// webpki's `Display` prints the variant name.
#[derive(Debug)]
struct NamedError(&'static str);

impl std::fmt::Display for NamedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for NamedError {}

#[test]
fn a_ca_certificate_used_as_the_server_certificate_is_an_untrusted_issuer() {
    use rustls::{CertificateError, Error, OtherError};
    let wrap = |name| {
        Error::InvalidCertificate(CertificateError::Other(OtherError(std::sync::Arc::new(
            NamedError(name),
        ))))
    };
    assert_eq!(
        TlsFailure::from_rustls(&wrap("CaUsedAsEndEntity")),
        TlsFailure::UnknownIssuer
    );
    // Other wrapped errors stay `Other` (off Apple, where no OSStatus is read).
    assert_eq!(
        TlsFailure::from_rustls(&wrap("InvalidCertValidity")),
        TlsFailure::Other
    );
}

#[test]
fn tls_failures_map_from_rustls_errors() {
    use rustls::{AlertDescription, CertificateError, Error};
    for (e, f) in [
        (
            Error::InvalidCertificate(CertificateError::UnknownIssuer),
            TlsFailure::UnknownIssuer,
        ),
        (
            Error::InvalidCertificate(CertificateError::NotValidForName),
            TlsFailure::NameMismatch,
        ),
        (
            Error::InvalidCertificate(CertificateError::Expired),
            TlsFailure::Expired,
        ),
        (
            Error::InvalidCertificate(CertificateError::NotValidYet),
            TlsFailure::NotYetValid,
        ),
        (
            Error::InvalidCertificate(CertificateError::Revoked),
            TlsFailure::Other,
        ),
        // A bad signature is a forged handshake or chain, not an unknown issuer: the
        // "trust this certificate" fixes would be the wrong advice.
        (
            Error::InvalidCertificate(CertificateError::BadSignature),
            TlsFailure::Protocol,
        ),
        (
            Error::AlertReceived(AlertDescription::CertificateRequired),
            TlsFailure::ClientCertRequired,
        ),
        (
            Error::AlertReceived(AlertDescription::ProtocolVersion),
            TlsFailure::Protocol,
        ),
        (Error::DecryptError, TlsFailure::Protocol),
        (Error::General("x".into()), TlsFailure::Other),
    ] {
        assert_eq!(TlsFailure::from_rustls(&e), f, "{e:?}");
    }
}

/// Throwaway test keys and self-signed certificates, generated with `openssl genrsa`,
/// `openssl ecparam -genkey -noout`, `openssl pkcs8 -topk8 -nocrypt -outform DER` and
/// `openssl req -x509` (X.509 v3 with `CA:FALSE`, `clientAuth`; `ec-sec1-v1.crt` is a
/// deliberate v1 certificate made without extensions). They protect nothing. Key files end in
/// `-key.pem`/`-key.der` because the repository ignores `*.key`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn pkcs1_sec1_and_der_pkcs8_keys_load() {
    install_crypto_provider();
    let client = |cert: &str, key: &str| TlsSettings {
        mode: TlsMode::Require,
        ca_file: None,
        client_cert: Some(ClientCertPaths {
            cert: fixture(cert),
            key: fixture(key),
        }),
        ..Default::default()
    };
    for (cert, key) in [
        ("rsa-pkcs1.crt", "rsa-pkcs1-key.pem"),
        ("ec-sec1.crt", "ec-sec1-key.pem"),
        ("ec-pkcs8.crt", "ec-pkcs8-key.der"),
    ] {
        let p = build_tls_params(&client(cert, key), "db", 5432, Public, None);
        assert!(matches!(p, Ok(Some(_))), "{key}: {p:?}");
    }
    // Keys that load but belong to another certificate are still refused.
    assert_eq!(
        build_tls_params(
            &client("rsa-pkcs1.crt", "ec-sec1-key.pem"),
            "db",
            5432,
            Public,
            None
        )
        .unwrap_err(),
        TlsSetupError::KeyUnreadable(fixture("ec-sec1-key.pem"))
    );
    // A certificate rustls cannot use (X.509 v1) is the certificate's fault, not the key's.
    assert_eq!(
        build_tls_params(
            &client("ec-sec1-v1.crt", "ec-sec1-key.pem"),
            "db",
            5432,
            Public,
            None
        )
        .unwrap_err(),
        TlsSetupError::CertFileUnreadable(fixture("ec-sec1-v1.crt"))
    );
}

#[test]
fn an_unusable_server_name_fails_closed() {
    install_crypto_provider();
    for mode in [
        TlsMode::Auto,
        TlsMode::Require,
        TlsMode::VerifyCa,
        TlsMode::VerifyFull,
    ] {
        assert_eq!(
            build_tls_params(&settings(mode), "db bad", 5432, Public, None).unwrap_err(),
            TlsSetupError::InvalidServerName("db bad".into()),
            "{mode:?}"
        );
    }
    // Disable never needs a name.
    assert!(
        build_tls_params(&settings(TlsMode::Disable), "db bad", 5432, Public, None)
            .unwrap()
            .is_none()
    );
    // A trailing dot is a valid absolute name.
    let p = build_tls_params(
        &settings(TlsMode::VerifyFull),
        "db.acme.io.",
        5432,
        Public,
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(p.server_name.to_str(), "db.acme.io");
}

#[test]
fn debug_output_never_shows_the_client_key() {
    install_crypto_provider();
    let dir = tempfile::tempdir().unwrap();
    let key = KeyPair::generate().unwrap();
    let key_pem = key.serialize_pem();

    // A key that does not belong to the certificate is refused.
    let (mismatched, key_path) = with_client_key(dir.path(), &key_pem);
    assert_eq!(
        build_tls_params(&mismatched, "db", 5432, Public, None).unwrap_err(),
        TlsSetupError::KeyUnreadable(key_path)
    );

    let cert = leaf_params("client").self_signed(&key).unwrap();
    let settings = TlsSettings {
        mode: TlsMode::Require,
        ca_file: None,
        client_cert: Some(ClientCertPaths {
            cert: write(dir.path(), "own.crt", cert.pem().as_bytes()),
            key: write(dir.path(), "own.key", key_pem.as_bytes()),
        }),
        ..Default::default()
    };
    let params = build_tls_params(&settings, "db", 5432, Public, None)
        .unwrap()
        .unwrap();
    let debug = format!("{params:?}");
    for line in key_pem.lines().filter(|l| !l.starts_with("-----")) {
        assert!(!debug.contains(line), "a key line leaked into Debug");
    }
    let der = key.serialize_der();
    let tail = format!("{:?}", &der[der.len() - 16..]);
    assert!(!debug.contains(tail.trim_matches(['[', ']'])));
}

// ## m2b-t01-interfaces

/// A handshake toward `localhost` that keeps its params, for the M2b recording checks.
async fn handshake_params(srv: &Srv, settings: TlsSettings) -> (Result<u8, ()>, TlsParams) {
    install_crypto_provider();
    let params = build_tls_params(&settings, "localhost", srv.addr.port(), Loopback, None)
        .unwrap()
        .unwrap();
    let tcp = TcpStream::connect(srv.addr).await.unwrap();
    let connector = TlsConnector::from(params.config.clone());
    let result = async {
        let mut tls = connector
            .connect(params.server_name.clone(), tcp)
            .await
            .map_err(|_| ())?;
        let mut status = [0u8; 1];
        tls.read_exact(&mut status).await.map_err(|_| ())?;
        Ok(status[0])
    }
    .await;
    (result, params)
}

/// Task 3 replaced the Task 1 pin stub: the pinned leaf passes as `Pinned`, any other leaf
/// fails as `PinMismatch`; the verifier records the presented chain either way, and the
/// outcome only on success.
#[tokio::test]
async fn a_pinned_endpoint_is_checked_and_the_chain_is_recorded() {
    let srv = rcgen_server("localhost").await;
    let accepted = settings(TlsMode::Require);
    let (ok, params) = handshake_params(&srv, accepted.clone()).await;
    assert!(ok.is_ok());
    assert_eq!(*params.outcome.lock().unwrap(), Some(TlsVerified::No));
    let leaf = params.presented.lock().unwrap().clone().expect("a chain");
    assert_eq!(leaf.0.len(), 1);

    let pin = CertPin {
        endpoint: pin_endpoint("localhost", srv.addr.port()),
        sha256: cert_sha256(&leaf.0[0]),
    };
    let pinned = TlsSettings {
        pins: vec![pin.clone()],
        ..accepted.clone()
    };
    let (ok, params) = handshake_params(&srv, pinned).await;
    assert!(ok.is_ok(), "the pinned leaf passes");
    assert_eq!(*params.outcome.lock().unwrap(), Some(TlsVerified::Pinned));
    assert!(!params.plaintext_allowed);

    let wrong = TlsSettings {
        pins: vec![CertPin {
            sha256: [0; 32],
            ..pin
        }],
        ..accepted
    };
    let (refused, params) = handshake_params(&srv, wrong).await;
    assert!(refused.is_err(), "another leaf is refused");
    assert_eq!(
        *params.failure.lock().unwrap(),
        Some(TlsFailure::PinMismatch)
    );
    assert_eq!(*params.outcome.lock().unwrap(), None);
    assert!(params.presented.lock().unwrap().is_some());
    assert!(!params.plaintext_allowed);
}
