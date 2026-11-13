//! M2b Task 3 (HV-SEC-02; B5, B25, Q13, Q33, Q36): TLS pins on the leaf certificate against
//! loopback rustls servers with rcgen certificates, and encrypted PKCS #8 client keys.
//!
//! The servers listen on a random loopback port; the profile's port is always
//! [`PROFILE_PORT`], so pin endpoints read `db:5432` as in a profile (the dial goes to the real
//! port, as through a tunnel or forwarder).

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hacivat_net::{
    AddressClass, CertPin, ClientCertPaths, TlsFailure, TlsMode, TlsParams, TlsSettings,
    TlsSetupError, TlsVerified, build_tls_params, cert_review, cert_sha256,
    install_crypto_provider, pin_endpoint,
};
use pkcs8::pkcs5::pbes2;
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// The port the profile names (the pin endpoint's port).
const PROFILE_PORT: u16 = 5432;

// ---- certificates -------------------------------------------------------------------------

/// A server certificate chain (leaf first) and the leaf's key.
struct TestLeaf {
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

const DAY: Duration = Duration::from_secs(86_400);

/// Sets the validity window, each end some time before or after now.
fn window(p: &mut CertificateParams, from: Offset, to: Offset) {
    let now =
        rcgen::date_time_ymd(1970, 1, 1) + SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let at = |o: Offset| match o {
        Offset::Ago(d) => now - d,
        Offset::Ahead(d) => now + d,
    };
    p.not_before = at(from);
    p.not_after = at(to);
}

#[derive(Clone, Copy)]
enum Offset {
    Ago(Duration),
    Ahead(Duration),
}

/// A server leaf for `san` (a DNS name or an IP literal), valid from a day ago for `days`.
fn leaf_params(san: &str, days: u64) -> CertificateParams {
    let mut p = CertificateParams::new(vec![san.to_owned()]).unwrap();
    p.distinguished_name.push(DnType::CommonName, san);
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    window(
        &mut p,
        Offset::Ago(DAY),
        Offset::Ahead(DAY * u32::try_from(days).unwrap()),
    );
    p
}

fn self_signed_with(params: CertificateParams) -> TestLeaf {
    let key = KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    TestLeaf {
        chain: vec![cert.der().clone()],
        key: PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    }
}

fn self_signed(name: &str) -> TestLeaf {
    self_signed_with(leaf_params(name, 30))
}

fn self_signed_valid_days(name: &str, days: u64) -> TestLeaf {
    self_signed_with(leaf_params(name, days))
}

/// The common self-signed PostgreSQL certificate: `CA:TRUE` presented as the server's own.
fn self_signed_ca_true(name: &str) -> TestLeaf {
    let mut p = leaf_params(name, 30);
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.key_usages.push(KeyUsagePurpose::KeyCertSign);
    self_signed_with(p)
}

/// A certificate whose only SAN is an IP address (Q33).
fn self_signed_ip_san(ip: &str) -> TestLeaf {
    self_signed_with(leaf_params(ip, 30))
}

fn expired_self_signed(name: &str) -> TestLeaf {
    let mut p = leaf_params(name, 30);
    window(&mut p, Offset::Ago(DAY * 30), Offset::Ago(DAY));
    self_signed_with(p)
}

fn not_yet_valid_self_signed(name: &str) -> TestLeaf {
    let mut p = leaf_params(name, 30);
    window(&mut p, Offset::Ahead(DAY), Offset::Ahead(DAY * 30));
    self_signed_with(p)
}

/// A CA whose certificate is written to a file the settings can name.
struct TestCa {
    issuer: CertifiedIssuer<'static, KeyPair>,
    dir: PathBuf,
}

impl TestCa {
    fn leaf(&self, name: &str) -> TestLeaf {
        let key = KeyPair::generate().unwrap();
        let cert = leaf_params(name, 30)
            .signed_by(&key, &*self.issuer)
            .unwrap();
        TestLeaf {
            chain: vec![cert.der().clone()],
            key: PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        }
    }

    fn pem_file(&self) -> PathBuf {
        let path = self.dir.join("ca.pem");
        std::fs::write(&path, self.issuer.pem()).unwrap();
        path
    }
}

fn test_ca() -> TestCa {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.distinguished_name
        .push(DnType::CommonName, "Hacivat Test CA");
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    window(&mut p, Offset::Ago(DAY), Offset::Ahead(DAY * 30));
    TestCa {
        issuer: CertifiedIssuer::self_signed(p, KeyPair::generate().unwrap()).unwrap(),
        dir: fixture_dir(),
    }
}

/// A fresh directory under cargo's per-target temporary directory.
fn fixture_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "tls-pins-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---- servers ------------------------------------------------------------------------------

struct Srv {
    addr: SocketAddr,
    leaf: CertificateDer<'static>,
}

impl Srv {
    fn port(&self) -> u16 {
        self.addr.port()
    }

    fn leaf_der(&self) -> &[u8] {
        &self.leaf
    }
}

/// Serves TLS with `leaf` on loopback (inside a Tokio runtime). After each handshake the server
/// writes one byte (and, under TLS 1.3, its session tickets) and waits for EOF.
fn server(leaf: TestLeaf) -> Srv {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let addr = std_listener.local_addr().unwrap();
    let listener = TcpListener::from_std(std_listener).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(leaf.chain.clone(), leaf.key)
    .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let _ = tls.write_all(&[1]).await;
                let _ = tls.flush().await;
                let mut sink = [0u8; 1];
                let _ = tls.read(&mut sink).await;
            });
        }
    });
    Srv {
        addr,
        leaf: leaf.chain[0].clone(),
    }
}

// ---- settings -----------------------------------------------------------------------------

fn auto_with(pins: &[CertPin]) -> TlsSettings {
    TlsSettings {
        mode: TlsMode::Auto,
        pins: pins.to_vec(),
        ..Default::default()
    }
}

fn mode(mode: TlsMode) -> TlsSettings {
    TlsSettings {
        mode,
        ..Default::default()
    }
}

/// The pin of `srv`'s leaf for `db` at the profile port.
fn pin_of(srv: &Srv) -> CertPin {
    pin_of_as(srv, "db")
}

fn pin_of_as(srv: &Srv, host: &str) -> CertPin {
    CertPin {
        endpoint: pin_endpoint(host, PROFILE_PORT),
        sha256: cert_sha256(srv.leaf_der()),
    }
}

/// A pin for `endpoint` that no test certificate has.
fn pin(endpoint: &str) -> CertPin {
    CertPin {
        endpoint: endpoint.into(),
        sha256: [7; 32],
    }
}

fn other_pin(endpoint: &str) -> CertPin {
    CertPin {
        endpoint: endpoint.into(),
        sha256: [8; 32],
    }
}

// ---- the client side ----------------------------------------------------------------------

async fn handshake(settings: TlsSettings, port: u16) -> Result<TlsVerified, TlsFailure> {
    handshake_keep(settings, port).await.0
}

async fn handshake_as(
    settings: TlsSettings,
    host: &str,
    port: u16,
) -> Result<TlsVerified, TlsFailure> {
    handshake_keep_as(settings, host, port).await.0
}

async fn handshake_keep(
    settings: TlsSettings,
    port: u16,
) -> (Result<TlsVerified, TlsFailure>, TlsParams) {
    handshake_keep_as(settings, "db", port).await
}

/// One handshake as `host` at the profile port, dialled to `port` on loopback.
async fn handshake_keep_as(
    settings: TlsSettings,
    host: &str,
    port: u16,
) -> (Result<TlsVerified, TlsFailure>, TlsParams) {
    install_crypto_provider();
    let params = build_tls_params(&settings, host, PROFILE_PORT, AddressClass::Private, None)
        .unwrap()
        .unwrap();
    let result = handshake_with(&params, port).await;
    (result, params)
}

/// A handshake with existing params. A failure is classified from the rustls error alone (what
/// a driver falls back to when its failure slot is empty, T1 review M1), and must agree with
/// what the verifier recorded.
async fn handshake_with(params: &TlsParams, port: u16) -> Result<TlsVerified, TlsFailure> {
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let connector = TlsConnector::from(params.config.clone());
    match connector.connect(params.server_name.clone(), tcp).await {
        Ok(mut tls) => {
            let mut status = [0u8; 1];
            tls.read_exact(&mut status).await.unwrap();
            Ok(params
                .outcome
                .lock()
                .unwrap()
                .expect("an outcome on success"))
        }
        Err(e) => {
            let from_error = rustls_failure(&e);
            assert_eq!(
                *params.failure.lock().unwrap(),
                Some(from_error),
                "the recorded failure and the rustls error agree"
            );
            assert_eq!(*params.outcome.lock().unwrap(), None);
            Err(from_error)
        }
    }
}

fn rustls_failure(e: &io::Error) -> TlsFailure {
    e.get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .map_or(TlsFailure::Other, TlsFailure::from_rustls)
}

// ---- pins ---------------------------------------------------------------------------------

#[tokio::test]
async fn ca_true_self_signed_can_be_pinned() {
    let srv = server(self_signed_ca_true("db"));
    let pin = pin_of(&srv);
    assert_eq!(
        handshake(auto_with(&[pin]), srv.port()).await,
        Ok(TlsVerified::Pinned)
    );
}

#[tokio::test]
async fn ip_only_san_can_be_pinned() {
    let srv = server(self_signed_ip_san("10.0.0.5")); // Q33
    assert_eq!(
        handshake_as(
            auto_with(&[pin_of_as(&srv, "10.0.0.5")]),
            "10.0.0.5",
            srv.port()
        )
        .await,
        Ok(TlsVerified::Pinned)
    );
}

#[tokio::test]
async fn long_validity_cert_can_be_pinned() {
    let srv = server(self_signed_valid_days("db", 3650)); // the 825-day case (Q33)
    assert_eq!(
        handshake(auto_with(&[pin_of(&srv)]), srv.port()).await,
        Ok(TlsVerified::Pinned)
    );
}

#[test]
fn pinned_endpoint_never_allows_plaintext() {
    // Q13
    let s = TlsSettings {
        allow_public_plaintext: true,
        ..auto_with(&[pin("db:5432")])
    };
    assert!(
        !build_tls_params(&s, "db", 5432, AddressClass::Private, None)
            .unwrap()
            .unwrap()
            .plaintext_allowed
    );
    assert!(
        build_tls_params(
            &auto_with(&[pin("db:5433")]),
            "db",
            5432,
            AddressClass::Private,
            None
        )
        .unwrap()
        .unwrap()
        .plaintext_allowed
    );
}

#[tokio::test]
async fn unknown_issuer_captures_the_presented_chain() {
    let srv = server(self_signed("db"));
    let (r, params) = handshake_keep(auto_with(&[]), srv.port()).await;
    assert_eq!(r, Err(TlsFailure::UnknownIssuer));
    let rev = cert_review(
        params.presented.lock().unwrap().as_ref().unwrap(),
        "db:5432",
        None,
    )
    .unwrap();
    assert_eq!(rev.sha256, cert_sha256(srv.leaf_der()));
    assert_eq!(rev.chain[0].role, hacivat_net::ChainRole::Leaf);
}

#[tokio::test]
async fn rotated_certificate_is_pin_mismatch_with_both_fingerprints() {
    let old = pin_of(&server(self_signed("db")));
    let srv = server(self_signed("db"));
    let (r, p) = handshake_keep(auto_with(std::slice::from_ref(&old)), srv.port()).await;
    assert_eq!(r, Err(TlsFailure::PinMismatch));
    let rev = cert_review(
        p.presented.lock().unwrap().as_ref().unwrap(),
        &old.endpoint,
        Some(old.sha256),
    )
    .unwrap();
    assert_eq!(rev.pinned, Some(old.sha256));
    assert_ne!(rev.sha256, old.sha256);
    assert_eq!(rev.sha256, cert_sha256(srv.leaf_der()));
}

#[tokio::test]
async fn pin_mismatch_even_when_the_chain_is_trusted() {
    let ca = test_ca();
    let srv = server(ca.leaf("db"));
    let trusted = TlsSettings {
        ca_file: Some(ca.pem_file()),
        ..auto_with(&[])
    };
    // Without the pin the chain is trusted, so the pin alone refuses.
    assert_eq!(
        handshake(trusted.clone(), srv.port()).await,
        Ok(TlsVerified::Full)
    );
    let s = TlsSettings {
        pins: vec![other_pin("db:5432")],
        ..trusted
    };
    assert_eq!(handshake(s, srv.port()).await, Err(TlsFailure::PinMismatch));
}

#[tokio::test]
async fn pin_does_not_override_expiry() {
    let srv = server(expired_self_signed("db"));
    assert_eq!(
        handshake(auto_with(&[pin_of(&srv)]), srv.port()).await,
        Err(TlsFailure::Expired)
    );
    let srv = server(not_yet_valid_self_signed("db"));
    assert_eq!(
        handshake(auto_with(&[pin_of(&srv)]), srv.port()).await,
        Err(TlsFailure::NotYetValid)
    );
}

#[tokio::test]
async fn pin_overrides_name_mismatch() {
    let srv = server(self_signed("localhost")); // connect as "db"
    assert_eq!(
        handshake(auto_with(&[pin_of_as(&srv, "db")]), srv.port()).await,
        Ok(TlsVerified::Pinned)
    );
}

#[tokio::test]
async fn require_records_chain_and_reports_not_verified() {
    let srv = server(self_signed("db"));
    let (r, p) = handshake_keep(mode(TlsMode::Require), srv.port()).await;
    assert_eq!(r, Ok(TlsVerified::No));
    assert!(p.presented.lock().unwrap().is_some());
}

#[test]
fn pins_never_apply_in_disable() {
    assert!(
        build_tls_params(
            &TlsSettings {
                mode: TlsMode::Disable,
                ..auto_with(&[pin("db:5432")])
            },
            "db",
            5432,
            AddressClass::Public,
            None
        )
        .unwrap()
        .is_none()
    );
}

/// B5: a pin is checked in every mode except Disable; Require with a pin verifies the pin, and
/// VerifyCa without a CA file trusts the pinned leaf.
#[tokio::test]
async fn every_verifying_mode_checks_the_pin() {
    let srv = server(self_signed("localhost"));
    for m in [
        TlsMode::Auto,
        TlsMode::Require,
        TlsMode::VerifyCa,
        TlsMode::VerifyFull,
    ] {
        let right = TlsSettings {
            pins: vec![pin_of(&srv)],
            ..mode(m)
        };
        assert_eq!(
            handshake(right, srv.port()).await,
            Ok(TlsVerified::Pinned),
            "{m:?}"
        );
        let wrong = TlsSettings {
            pins: vec![pin("db:5432")],
            ..mode(m)
        };
        assert_eq!(
            handshake(wrong, srv.port()).await,
            Err(TlsFailure::PinMismatch),
            "{m:?}"
        );
    }
}

/// The endpoint rule: a pin names one host (compared in ASCII lowercase) and one port.
#[tokio::test]
async fn a_pin_applies_to_its_own_endpoint_only() {
    let srv = server(self_signed("localhost"));
    let upper = CertPin {
        endpoint: "DB:5432".into(),
        ..pin_of(&srv)
    };
    assert_eq!(
        handshake(auto_with(&[upper]), srv.port()).await,
        Ok(TlsVerified::Pinned),
        "DB and db are one DNS name"
    );
    // The same certificate pinned on another port does not apply here: the platform check
    // runs and refuses the self-signed leaf.
    let other_port = CertPin {
        endpoint: "db:5433".into(),
        ..pin_of(&srv)
    };
    assert_eq!(
        handshake(auto_with(&[other_port]), srv.port()).await,
        Err(TlsFailure::UnknownIssuer)
    );
}

/// One `TlsParams` per attempt is the contract; a reused one still never reports a stale
/// outcome, and session resumption is off so each handshake runs (and records) the check.
#[tokio::test]
async fn each_handshake_records_afresh() {
    let a = server(self_signed("db"));
    let b = server(self_signed("db"));
    let (first, params) = handshake_keep(auto_with(&[pin_of(&a)]), a.port()).await;
    assert_eq!(first, Ok(TlsVerified::Pinned));

    // Again to the same server: no resumed session, so the verifier records again.
    *params.outcome.lock().unwrap() = None;
    *params.presented.lock().unwrap() = None;
    assert_eq!(
        handshake_with(&params, a.port()).await,
        Ok(TlsVerified::Pinned)
    );
    assert!(params.presented.lock().unwrap().is_some());

    // To another server: the earlier `Pinned` does not survive the mismatch.
    assert_eq!(
        handshake_with(&params, b.port()).await,
        Err(TlsFailure::PinMismatch)
    );
    assert_eq!(*params.outcome.lock().unwrap(), None);
    let presented = params.presented.lock().unwrap().clone().unwrap();
    assert_eq!(presented.0[0].as_ref(), b.leaf_der());
}

// ---- encrypted client keys (B25, Q36) -----------------------------------------------------

/// One client key for every fixture, so each encrypted copy belongs to the certificate.
fn client_key() -> &'static KeyPair {
    static KEY: OnceLock<KeyPair> = OnceLock::new();
    KEY.get_or_init(|| KeyPair::generate().unwrap())
}

const SALT: [u8; 16] = [3; 16];
const IV: [u8; 16] = [5; 16];

fn encrypted_key_file(params: pbes2::Parameters<'_>, passphrase: &str) -> PathBuf {
    let der = client_key().serialize_der();
    let info = pkcs8::PrivateKeyInfo::try_from(der.as_slice()).unwrap();
    let doc = info.encrypt_with_params(params, passphrase).unwrap();
    let pem = doc
        .to_pem("ENCRYPTED PRIVATE KEY", pkcs8::LineEnding::LF)
        .unwrap();
    let path = fixture_dir().join("client-enc.key");
    std::fs::write(&path, pem.as_bytes()).unwrap();
    path
}

/// A client certificate and its key as OpenSSL 1.1+ encrypts it by default (PBES2,
/// PBKDF2-HMAC-SHA-256, AES-256-CBC).
fn client_cert_encrypted(passphrase: &str) -> (PathBuf, PathBuf) {
    let mut p = CertificateParams::new(vec!["client".to_owned()]).unwrap();
    p.distinguished_name.push(DnType::CommonName, "client");
    window(&mut p, Offset::Ago(DAY), Offset::Ahead(DAY * 30));
    let cert = p.self_signed(client_key()).unwrap();
    let cert_path = fixture_dir().join("client.crt");
    std::fs::write(&cert_path, cert.pem()).unwrap();
    let params = pbes2::Parameters::pbkdf2_sha256_aes256cbc(2048, &SALT, &IV).unwrap();
    (cert_path, encrypted_key_file(params, passphrase))
}

/// OpenSSL 1.0's PKCS #8 output: PBES2 with the default HMAC-SHA-1 PRF (Q36).
fn client_key_pbes2_sha1_prf(passphrase: &str) -> PathBuf {
    let params = pbes2::Parameters {
        kdf: pbes2::Pbkdf2Params {
            salt: &SALT,
            iteration_count: 2048,
            key_length: None,
            prf: pbes2::Pbkdf2Prf::HmacWithSha1,
        }
        .into(),
        encryption: pbes2::EncryptionScheme::Aes256Cbc { iv: &IV },
    };
    encrypted_key_file(params, passphrase)
}

/// PBES2 with scrypt (OpenSSL's `-scrypt`), with a small cost for the test.
fn client_key_scrypt(passphrase: &str) -> PathBuf {
    let cost = pkcs8::pkcs5::scrypt::Params::new(10, 8, 1, 32).unwrap();
    let params = pbes2::Parameters::scrypt_aes256cbc(cost, &SALT, &IV).unwrap();
    encrypted_key_file(params, passphrase)
}

/// A legacy OpenSSL-encrypted PEM key (no real key material: the header is what is read).
fn legacy_encrypted_pem() -> PathBuf {
    let path = fixture_dir().join("legacy.key");
    std::fs::write(
        &path,
        "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\n\
         DEK-Info: AES-256-CBC,00000000000000000000000000000000\n\nAAAA\n\
         -----END RSA PRIVATE KEY-----\n",
    )
    .unwrap();
    path
}

fn with_client(cert: &std::path::Path, key: &std::path::Path) -> TlsSettings {
    TlsSettings {
        client_cert: Some(ClientCertPaths {
            cert: cert.to_path_buf(),
            key: key.to_path_buf(),
        }),
        ..mode(TlsMode::Require)
    }
}

fn build_with(settings: &TlsSettings, passphrase: Option<&str>) -> Result<(), TlsSetupError> {
    install_crypto_provider();
    let passphrase = passphrase.map(SecretString::from);
    build_tls_params(
        settings,
        "db",
        5432,
        AddressClass::Private,
        passphrase.as_ref(),
    )
    .map(|_| ())
}

fn err_of(settings: &TlsSettings, passphrase: Option<&str>) -> TlsSetupError {
    build_with(settings, passphrase).unwrap_err()
}

#[test]
fn encrypted_pkcs8_keys() {
    install_crypto_provider();
    let (cert, key_enc) = client_cert_encrypted("pw1");
    assert!(
        build_tls_params(
            &with_client(&cert, &key_enc),
            "db",
            5432,
            AddressClass::Private,
            Some(&SecretString::from("pw1"))
        )
        .is_ok()
    );
    assert_eq!(
        err_of(&with_client(&cert, &key_enc), Some("nope")),
        TlsSetupError::KeyPassphraseWrong(key_enc.clone())
    );
    assert_eq!(
        err_of(&with_client(&cert, &key_enc), None),
        TlsSetupError::KeyEncrypted(key_enc.clone())
    );
    assert!(matches!(
        err_of(&with_client(&cert, &legacy_encrypted_pem()), Some("pw1")),
        TlsSetupError::KeyEncryptedLegacy(_)
    ));
    assert!(
        build_tls_params(
            &with_client(&cert, &client_key_pbes2_sha1_prf("pw1")),
            "db",
            5432,
            AddressClass::Private,
            Some(&SecretString::from("pw1"))
        )
        .is_ok()
    ); // Q36
}

#[test]
fn encrypted_keys_beyond_the_default() {
    let (cert, _) = client_cert_encrypted("pw1");
    assert_eq!(
        build_with(&with_client(&cert, &client_key_scrypt("pw1")), Some("pw1")),
        Ok(())
    );
    let scrypt = client_key_scrypt("pw1");
    assert_eq!(
        err_of(&with_client(&cert, &scrypt), Some("pw2")),
        TlsSetupError::KeyPassphraseWrong(scrypt)
    );
    // The legacy key is refused whether or not a passphrase is offered.
    let legacy = legacy_encrypted_pem();
    assert_eq!(
        err_of(&with_client(&cert, &legacy), None),
        TlsSetupError::KeyEncryptedLegacy(legacy)
    );
    // An encrypted key as DER (no PEM armour) is decrypted too.
    let pem = std::fs::read(client_key_pbes2_sha1_prf("pw1")).unwrap();
    let (_, der) = pkcs8::der::pem::decode_vec(&pem).unwrap();
    let der_path = fixture_dir().join("client-enc.der");
    std::fs::write(&der_path, der).unwrap();
    assert_eq!(
        build_with(&with_client(&cert, &der_path), Some("pw1")),
        Ok(())
    );
    assert_eq!(
        err_of(&with_client(&cert, &der_path), None),
        TlsSetupError::KeyEncrypted(der_path)
    );
}

/// A key file that asks for an absurd KDF cost is refused before any derivation, so a crafted
/// file cannot hold the core runtime.
#[test]
fn an_excessive_kdf_cost_is_unreadable() {
    let (cert, _) = client_cert_encrypted("pw1");
    // Over Hacivat's cap (10 million), under pkcs5's own limit (100 million): unchecked, this
    // would take most of a minute.
    let params = pbes2::Parameters {
        kdf: pbes2::Pbkdf2Params {
            salt: &SALT,
            iteration_count: 50_000_000,
            key_length: None,
            prf: pbes2::Pbkdf2Prf::HmacWithSha256,
        }
        .into(),
        encryption: pbes2::EncryptionScheme::Aes256Cbc { iv: &IV },
    };
    // Only the header matters: encrypt the key cheaply, then rewrite the iteration count.
    let cheap = pbes2::Parameters::pbkdf2_sha256_aes256cbc(1, &SALT, &IV).unwrap();
    let der = client_key().serialize_der();
    let info = pkcs8::PrivateKeyInfo::try_from(der.as_slice()).unwrap();
    let doc = info.encrypt_with_params(cheap, "pw1").unwrap();
    let parsed = pkcs8::EncryptedPrivateKeyInfo::try_from(doc.as_bytes()).unwrap();
    let costly = pkcs8::EncryptedPrivateKeyInfo {
        encryption_algorithm: params.into(),
        encrypted_data: parsed.encrypted_data,
    };
    let der = pkcs8::der::Encode::to_der(&costly).unwrap();
    let path = fixture_dir().join("costly.der");
    std::fs::write(&path, der).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        err_of(&with_client(&cert, &path), Some("pw1")),
        TlsSetupError::KeyUnreadable(path)
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// The passphrase never appears in what the params or errors print.
#[test]
fn the_passphrase_never_shows() {
    let (cert, key_enc) = client_cert_encrypted("hv_test_pw");
    let pass = SecretString::from("hv_test_pw");
    let params = build_tls_params(
        &with_client(&cert, &key_enc),
        "db",
        5432,
        AddressClass::Private,
        Some(&pass),
    )
    .unwrap()
    .unwrap();
    assert!(!format!("{params:?}").contains("hv_test_pw"));
    let err = err_of(&with_client(&cert, &key_enc), Some("hv_test_pw_wrong"));
    assert!(!format!("{err:?}").contains("hv_test_pw"));
}
