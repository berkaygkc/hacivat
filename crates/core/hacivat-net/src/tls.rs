//! Non-interactive TLS (HV-SEC-02): modes, settings, the rustls client configuration and what
//! a handshake reports. rustls uses the ring provider only (P5), with TLS 1.2 as the minimum.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::verify_server_cert_signed_by_trust_anchor;
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{DigitallySignedStruct, RootCertStore, SignatureScheme};

use crate::address::{AddressClass, parse_ip_literal};

/// The libpq-style TLS mode (HV-SEC-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TlsMode {
    Disable,
    #[default]
    Auto,
    Require,
    VerifyCa,
    VerifyFull,
}

impl TlsMode {
    pub const ALL: [TlsMode; 5] = [
        TlsMode::Disable,
        TlsMode::Auto,
        TlsMode::Require,
        TlsMode::VerifyCa,
        TlsMode::VerifyFull,
    ];

    /// The stable ID: `disable`, `auto`, `require`, `verify_ca`, `verify_full`.
    pub fn id(self) -> &'static str {
        match self {
            TlsMode::Disable => "disable",
            TlsMode::Auto => "auto",
            TlsMode::Require => "require",
            TlsMode::VerifyCa => "verify_ca",
            TlsMode::VerifyFull => "verify_full",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }
}

/// A client certificate and its key, as files (HV-SEC-02).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientCertPaths {
    pub cert: PathBuf,
    pub key: PathBuf,
}

/// A profile's TLS settings (HV-SEC-02).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct TlsSettings {
    pub mode: TlsMode,
    pub ca_file: Option<PathBuf>,
    pub client_cert: Option<ClientCertPaths>,
}

/// Why a handshake failed, as the verifier saw it (HV-CONN-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TlsFailure {
    UnknownIssuer,
    NameMismatch,
    Expired,
    NotYetValid,
    Protocol,
    ClientCertRequired,
    Other,
}

impl TlsFailure {
    /// Maps a rustls error, from a verifier or a handshake, to a failure. Drivers use it for
    /// errors the verifier never sees, such as the server's `certificate_required` alert.
    pub fn from_rustls(e: &rustls::Error) -> TlsFailure {
        use rustls::{AlertDescription as A, CertificateError as C, Error as E};
        match e {
            E::InvalidCertificate(c) => match c {
                C::UnknownIssuer => TlsFailure::UnknownIssuer,
                // A signature that does not verify (in the chain or the handshake) looks like
                // tampering, not an unknown CA: "trust this certificate" would be wrong advice.
                C::BadSignature => TlsFailure::Protocol,
                C::NotValidForName | C::NotValidForNameContext { .. } => TlsFailure::NameMismatch,
                C::Expired | C::ExpiredContext { .. } => TlsFailure::Expired,
                C::NotValidYet | C::NotValidYetContext { .. } => TlsFailure::NotYetValid,
                C::Other(other) => platform_failure(other),
                _ => TlsFailure::Other,
            },
            E::AlertReceived(A::CertificateRequired) => TlsFailure::ClientCertRequired,
            E::AlertReceived(_)
            | E::InappropriateMessage { .. }
            | E::InappropriateHandshakeMessage { .. }
            | E::InvalidMessage(_)
            | E::NoCertificatesPresented
            | E::UnsupportedNameType
            | E::DecryptError
            | E::PeerIncompatible(_)
            | E::PeerMisbehaved(_)
            | E::PeerSentOversizedRecord => TlsFailure::Protocol,
            _ => TlsFailure::Other,
        }
    }
}

/// The platform verifier on Apple maps only a few Security.framework results to rustls
/// errors and reports the rest as `Other("<description>: <OSStatus>")`. This reads the
/// trailing numeric code only, never the (localized) description. Elsewhere `Other` stays
/// `Other`.
///
/// `errSecNotTrusted` (-67843) is generic: Security.framework also reports it for policy
/// failures (a lifetime over Apple's limit, a missing server-auth usage, a weak key), so on
/// Apple those show as `UnknownIssuer` too.
fn platform_failure(other: &rustls::OtherError) -> TlsFailure {
    if !cfg!(target_vendor = "apple") {
        return TlsFailure::Other;
    }
    let text = other.to_string();
    let code = text.rsplit(' ').next().and_then(|c| c.parse::<i32>().ok());
    match code {
        // errSecNotTrusted, errSecCreateChainFailed, errSecInvalidRoot
        Some(-67843 | -67810 | -67612) => TlsFailure::UnknownIssuer,
        // errSecHostNameMismatch
        Some(-67602) => TlsFailure::NameMismatch,
        // errSecCertificateExpired
        Some(-67818) => TlsFailure::Expired,
        // errSecCertificateNotValidYet
        Some(-67819) => TlsFailure::NotYetValid,
        _ => TlsFailure::Other,
    }
}

/// Everything a driver needs to run a handshake (HV-SEC-02). `failure` is filled by the
/// verifier so a failed handshake can be classified.
#[derive(Debug, Clone)]
pub struct TlsParams {
    pub settings: TlsSettings,
    pub config: Arc<rustls::ClientConfig>,
    pub server_name: ServerName<'static>,
    /// Auto may fall back to plaintext (never on a public address without TLS, P9).
    pub plaintext_allowed: bool,
    pub failure: Arc<Mutex<Option<TlsFailure>>>,
}

/// How far a server certificate was verified (HV-SEC-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TlsVerified {
    Full,
    CaOnly,
    No,
}

/// What a live connection's TLS looks like (HV-SEC-02).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TlsSummary {
    pub encrypted: bool,
    pub version: Option<String>,
    pub cipher: Option<String>,
    pub verified: TlsVerified,
}

/// A TLS file problem found before connecting (HV-SEC-02).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TlsSetupError {
    CaFileUnreadable(PathBuf),
    CertFileUnreadable(PathBuf),
    KeyUnreadable(PathBuf),
    KeyEncrypted(PathBuf),
    /// The TLS host is neither an IP literal nor a DNS name rustls can verify or send as SNI.
    InvalidServerName(String),
}

/// Builds the client configuration for `settings` toward `tls_host`; `None` means Disable.
///
/// The configuration uses the ring provider (whatever the process default is) with TLS 1.3
/// and 1.2. The server certificate check depends on the mode:
/// - `Auto` and `VerifyFull`: the platform verifier, with the CA file's certificates as extra
///   roots;
/// - `VerifyCa`: the chain must lead to a CA-file certificate, and the host name is not
///   checked. Without a CA file nothing is trusted (as libpq's `verify-ca` without a root
///   file);
/// - `Require`: any certificate (the handshake signature is still checked).
///
/// Each verifier writes its first error, mapped to a [`TlsFailure`], into `failure`.
/// `server_name` is the address for an IP literal, so no SNI is sent. A host that is neither
/// an IP literal nor a valid DNS name fails closed with [`TlsSetupError::InvalidServerName`].
/// `plaintext_allowed` is true only for `Auto` toward a non-public address (P9).
///
/// The CA, certificate and key files are read here (small blocking reads). A missing or
/// unusable file, or an encrypted key, is a [`TlsSetupError`].
pub fn build_tls_params(
    settings: &TlsSettings,
    tls_host: &str,
    class: AddressClass,
) -> Result<Option<TlsParams>, TlsSetupError> {
    if settings.mode == TlsMode::Disable {
        return Ok(None);
    }
    let server_name = server_name(tls_host)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let ca = match &settings.ca_file {
        Some(path) => Some((read_ca_file(path)?, path.as_path())),
        None => None,
    };
    let client = match &settings.client_cert {
        Some(paths) => Some(read_client_cert(paths)?),
        None => None,
    };

    let inner: Arc<dyn ServerCertVerifier> = match settings.mode {
        TlsMode::Require => Arc::new(AcceptAnyCert {
            provider: provider.clone(),
        }),
        TlsMode::VerifyCa => Arc::new(ChainOnly {
            roots: ca.map_or_else(RootCertStore::empty, |(ca, _)| ca.store),
            provider: provider.clone(),
        }),
        TlsMode::Disable | TlsMode::Auto | TlsMode::VerifyFull => platform_verifier(ca, &provider)?,
    };
    let failure = Arc::new(Mutex::new(None));
    let verifier = Arc::new(Recording {
        inner,
        failure: failure.clone(),
    });

    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .expect("the ring provider supports TLS 1.3 and 1.2")
        .dangerous()
        .with_custom_certificate_verifier(verifier);
    let config = match client {
        Some(client) => builder
            .with_client_auth_cert(client.chain, client.key)
            .map_err(|e| match e {
                // rustls could not use the certificate itself (a v1 certificate, say).
                rustls::Error::InvalidCertificate(_) => {
                    TlsSetupError::CertFileUnreadable(client.cert_path)
                }
                // An unsupported key, or one that does not belong to the certificate.
                _ => TlsSetupError::KeyUnreadable(client.key_path),
            })?,
        None => builder.with_no_client_auth(),
    };

    Ok(Some(TlsParams {
        settings: settings.clone(),
        config: Arc::new(config),
        server_name,
        plaintext_allowed: settings.mode == TlsMode::Auto && class != AddressClass::Public,
        failure,
    }))
}

/// Installs rustls's ring provider as the process default, once (P5). Calling it again, or
/// after another provider was installed, does nothing. `CoreRuntime::new` calls it.
pub fn install_crypto_provider() {
    // `Err` only means a provider is already installed.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// The name to verify and send as SNI. An IP literal is an address (no SNI); a trailing dot
/// is dropped; anything else rustls cannot use fails closed.
fn server_name(host: &str) -> Result<ServerName<'static>, TlsSetupError> {
    if let Some(ip) = parse_ip_literal(host) {
        return Ok(ServerName::IpAddress(ip.into()));
    }
    let name = host.strip_suffix('.').unwrap_or(host);
    ServerName::try_from(name.to_owned())
        .map_err(|_| TlsSetupError::InvalidServerName(host.to_owned()))
}

struct CaFile {
    certs: Vec<CertificateDer<'static>>,
    store: RootCertStore,
}

/// Reads a CA file: PEM with any number of certificates, or one DER certificate. At least one
/// of them must be usable as a trust anchor.
fn read_ca_file(path: &Path) -> Result<CaFile, TlsSetupError> {
    let unreadable = || TlsSetupError::CaFileUnreadable(path.to_path_buf());
    let bytes = std::fs::read(path).map_err(|_| unreadable())?;
    let certs = parse_certs(&bytes).ok_or_else(unreadable)?;
    let mut store = RootCertStore::empty();
    let (added, _ignored) = store.add_parsable_certificates(certs.iter().cloned());
    if added == 0 {
        return Err(unreadable());
    }
    Ok(CaFile { certs, store })
}

fn parse_certs(bytes: &[u8]) -> Option<Vec<CertificateDer<'static>>> {
    let certs: Vec<CertificateDer<'static>> = if is_pem(bytes) {
        CertificateDer::pem_slice_iter(bytes)
            .collect::<Result<_, _>>()
            .ok()?
    } else {
        vec![CertificateDer::from(bytes.to_vec())]
    };
    (!certs.is_empty()).then_some(certs)
}

fn is_pem(bytes: &[u8]) -> bool {
    contains(bytes, b"-----BEGIN ")
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

struct ClientCert {
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    cert_path: PathBuf,
    key_path: PathBuf,
}

/// Reads a client certificate chain and its unencrypted key (PKCS #8, PKCS #1 or SEC1, as PEM
/// or DER). An encrypted key is refused before any parsing: M2a has no passphrase prompt.
fn read_client_cert(paths: &ClientCertPaths) -> Result<ClientCert, TlsSetupError> {
    let cert_err = || TlsSetupError::CertFileUnreadable(paths.cert.clone());
    let key_err = || TlsSetupError::KeyUnreadable(paths.key.clone());
    let cert_bytes = std::fs::read(&paths.cert).map_err(|_| cert_err())?;
    let chain = parse_certs(&cert_bytes).ok_or_else(cert_err)?;
    let key_bytes = std::fs::read(&paths.key).map_err(|_| key_err())?;
    if is_encrypted_key(&key_bytes) {
        return Err(TlsSetupError::KeyEncrypted(paths.key.clone()));
    }
    let key = if is_pem(&key_bytes) {
        PrivateKeyDer::from_pem_slice(&key_bytes).map_err(|_| key_err())?
    } else {
        PrivateKeyDer::try_from(key_bytes).map_err(|_| key_err())?
    };
    Ok(ClientCert {
        chain,
        key,
        cert_path: paths.cert.clone(),
        key_path: paths.key.clone(),
    })
}

/// An encrypted PKCS #8 PEM block, or a legacy OpenSSL-encrypted PEM key (`Proc-Type:
/// 4,ENCRYPTED`).
fn is_encrypted_key(bytes: &[u8]) -> bool {
    contains(bytes, b"-----BEGIN ENCRYPTED PRIVATE KEY-----")
        || contains(bytes, b"Proc-Type: 4,ENCRYPTED")
}

/// The platform verifier, with the CA file's certificates as extra roots. If the platform has
/// no roots at all (a bare Linux container) and there is no CA file, nothing is trusted.
fn platform_verifier(
    ca: Option<(CaFile, &Path)>,
    provider: &Arc<CryptoProvider>,
) -> Result<Arc<dyn ServerCertVerifier>, TlsSetupError> {
    use rustls_platform_verifier::Verifier;
    match ca {
        Some((ca, path)) => Verifier::new_with_extra_roots(ca.certs, provider.clone())
            .map(|v| Arc::new(v) as Arc<dyn ServerCertVerifier>)
            .map_err(|_| TlsSetupError::CaFileUnreadable(path.to_path_buf())),
        None => Ok(match Verifier::new(provider.clone()) {
            Ok(v) => Arc::new(v),
            Err(_) => Arc::new(ChainOnly {
                roots: RootCertStore::empty(),
                provider: provider.clone(),
            }),
        }),
    }
}

/// Wraps a verifier and records its first error into `TlsParams::failure`.
#[derive(Debug)]
struct Recording {
    inner: Arc<dyn ServerCertVerifier>,
    failure: Arc<Mutex<Option<TlsFailure>>>,
}

impl Recording {
    fn record<T>(&self, result: Result<T, rustls::Error>) -> Result<T, rustls::Error> {
        if let Err(e) = &result {
            let mut slot = self.failure.lock().unwrap_or_else(PoisonError::into_inner);
            if slot.is_none() {
                *slot = Some(TlsFailure::from_rustls(e));
            }
        }
        result
    }
}

impl ServerCertVerifier for Recording {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.record(self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.record(self.inner.verify_tls12_signature(message, cert, dss))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.record(self.inner.verify_tls13_signature(message, cert, dss))
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.inner.requires_raw_public_keys()
    }

    fn root_hint_subjects(&self) -> Option<&[rustls::DistinguishedName]> {
        self.inner.root_hint_subjects()
    }
}

/// Handshake signatures are checked with the provider's algorithms in every mode, so the peer
/// always proves it holds the certificate's key.
macro_rules! provider_signatures {
    () => {
        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls12_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls13_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.provider
                .signature_verification_algorithms
                .supported_schemes()
        }
    };
}

/// `Require`: encrypt without checking the certificate.
#[derive(Debug)]
struct AcceptAnyCert {
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    provider_signatures!();
}

/// `VerifyCa`: the chain (validity, signatures, server-auth usage) must lead to one of
/// `roots`; the host name is not checked.
#[derive(Debug)]
struct ChainOnly {
    roots: RootCertStore,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for ChainOnly {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let cert = ParsedCertificate::try_from(end_entity)?;
        verify_server_cert_signed_by_trust_anchor(
            &cert,
            &self.roots,
            intermediates,
            now,
            self.provider.signature_verification_algorithms.all,
        )?;
        Ok(ServerCertVerified::assertion())
    }

    provider_signatures!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_auto_is_the_default() {
        for m in TlsMode::ALL {
            assert_eq!(TlsMode::from_id(m.id()), Some(m));
        }
        assert_eq!(TlsMode::default(), TlsMode::Auto);
        assert_eq!(TlsSettings::default().mode, TlsMode::Auto);
    }

    #[test]
    fn the_ring_provider_is_installed_once() {
        install_crypto_provider();
        install_crypto_provider();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
        // A config built without naming a provider (as mysql_async does) uses ring's suites.
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        let suites = |p: &rustls::crypto::CryptoProvider| -> Vec<rustls::CipherSuite> {
            p.cipher_suites.iter().map(|s| s.suite()).collect()
        };
        assert_eq!(
            suites(config.crypto_provider()),
            suites(&rustls::crypto::ring::default_provider())
        );
    }

    #[test]
    fn apple_status_codes_are_read_from_other_errors() {
        let other = |text: &str| {
            rustls::Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
                Arc::new(std::io::Error::other(text.to_owned())),
            )))
        };
        let apple = cfg!(target_vendor = "apple");
        for (text, on_apple) in [
            (
                "“db” certificate is not trusted: -67843",
                TlsFailure::UnknownIssuer,
            ),
            ("x: -67818", TlsFailure::Expired),
            ("x: -67819", TlsFailure::NotYetValid),
            ("x: -67602", TlsFailure::NameMismatch),
            ("x: -1", TlsFailure::Other),
            ("no code", TlsFailure::Other),
        ] {
            let expected = if apple { on_apple } else { TlsFailure::Other };
            assert_eq!(TlsFailure::from_rustls(&other(text)), expected, "{text}");
        }
    }

    #[test]
    fn invalid_names_fail_closed() {
        for bad in ["db bad", "", ".", "db..acme.io"] {
            assert_eq!(
                server_name(bad),
                Err(TlsSetupError::InvalidServerName(bad.into())),
                "{bad:?}"
            );
        }
        assert_eq!(server_name("db.acme.io.").unwrap().to_str(), "db.acme.io");
        assert!(matches!(server_name("[::1]"), Ok(ServerName::IpAddress(_))));
    }
}
