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
use secrecy::SecretString;
use secrecy::zeroize::Zeroizing;

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

/// A pinned server certificate (M2b, HV-SEC-02): the SHA-256 of the leaf's DER for one
/// endpoint, `pin_endpoint(host, port)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CertPin {
    pub endpoint: String,
    pub sha256: [u8; 32],
}

/// A profile's TLS settings (HV-SEC-02).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct TlsSettings {
    pub mode: TlsMode,
    pub ca_file: Option<PathBuf>,
    pub client_cert: Option<ClientCertPaths>,
    // ## m2b-t01-interfaces
    /// Pinned leaf certificates (Task 3). A pin for the endpoint forbids the plaintext
    /// fallback (Q13).
    pub pins: Vec<CertPin>,
    /// The user answered the public-plaintext question with "remember" (Task 14).
    pub allow_public_plaintext: bool,
}

// ## m2b-t03-tls-pins
impl TlsSettings {
    /// The pin for the endpoint `pin_endpoint(tls_host, port)`, if any. A stored endpoint is
    /// compared in the same normal form (ASCII lowercase, no trailing dot, IPv6 in brackets),
    /// so `DB:5432` and `db.:5432` name the pin of `db:5432`; another port is another
    /// endpoint. The first matching pin wins.
    pub fn pin_for(&self, tls_host: &str, port: u16) -> Option<&CertPin> {
        let endpoint = pin_endpoint(tls_host, port);
        self.pins
            .iter()
            .find(|p| normalized_endpoint(&p.endpoint) == endpoint)
    }

    /// Disable while the endpoint has a pin (T1 review M2, Q13). A pin means "expect TLS with
    /// this certificate", and Disable never runs TLS, so the pin cannot be checked: this is
    /// the flag Validate and the connection dialog show. [`build_tls_params`] still returns
    /// `None` for Disable (B5: pins apply in every mode except Disable), because Disable is the
    /// user's explicit choice, never a fallback.
    pub fn disable_ignores_pin(&self, tls_host: &str, port: u16) -> bool {
        self.mode == TlsMode::Disable && self.pin_for(tls_host, port).is_some()
    }
}

/// A stored pin endpoint (`host:port`, `[v6]:port`) in [`pin_endpoint`]'s normal form; text
/// that is not an endpoint stays as it is (and matches nothing [`pin_endpoint`] produces).
fn normalized_endpoint(endpoint: &str) -> String {
    match endpoint.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => match port.parse::<u16>() {
            Ok(port) => pin_endpoint(host, port),
            Err(_) => endpoint.to_owned(),
        },
        _ => endpoint.to_owned(),
    }
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
    // ## m2b-t01-interfaces
    /// The leaf is not the pinned certificate (Task 3).
    PinMismatch,
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
                C::Other(other) => other_failure(other),
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

/// rustls has no variant for webpki's `CaUsedAsEndEntity` (a `CA:TRUE` certificate presented as
/// the server's own, as common self-signed PostgreSQL certificates are) and wraps it in
/// `CertificateError::Other`. webpki's `Display` is its variant name, so that name is the
/// stable key. The certificate cannot be trusted as it is, which is `UnknownIssuer` (the
/// "Encrypt without verifying" fix applies). Everything else is left to the platform verifier.
///
/// M2b (Task 3): the pin check reports a changed certificate as its own error type inside
/// `Other`, so the failure is `PinMismatch` from the rustls error alone, whether or not the
/// verifier's `failure` slot is the one a driver reads (T1 review M1).
fn other_failure(other: &rustls::OtherError) -> TlsFailure {
    if other.0.downcast_ref::<PinMismatchError>().is_some() {
        return TlsFailure::PinMismatch;
    }
    if other.to_string() == "CaUsedAsEndEntity" {
        return TlsFailure::UnknownIssuer;
    }
    platform_failure(other)
}

/// The platform verifier on Apple maps only a few Security.framework results to rustls
/// errors and reports the rest as `Other("<description>: <OSStatus>")`. This reads the
/// trailing numeric code only, never the (localized) description. Elsewhere `Other` stays
/// `Other`.
///
/// `errSecNotTrusted` (-67843) is generic: Security.framework also reports it for policy
/// failures (a lifetime over Apple's limit, a missing server-auth usage, a weak key), so on
/// Apple those show as `UnknownIssuer` too. Some policy failures arrive with their own code
/// (a validity period over Apple's maximum, a CA certificate used as the server's), and they
/// are `UnknownIssuer` for the same reason: the certificate cannot be trusted as it is.
fn platform_failure(other: &rustls::OtherError) -> TlsFailure {
    if !cfg!(target_vendor = "apple") {
        return TlsFailure::Other;
    }
    let text = other.to_string();
    let code = text.rsplit(' ').next().and_then(|c| c.parse::<i32>().ok());
    match code {
        // errSecNotTrusted (-67843), errSecCreateChainFailed (-25318), errSecInvalidRoot
        // (-67612)
        Some(-67843 | -25318 | -67612) => TlsFailure::UnknownIssuer,
        // errSecCertificateValidityPeriodTooLong, errSecCertificateIsCA
        Some(-67901 | -67902) => TlsFailure::UnknownIssuer,
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
    // ## m2a-finalfix
    /// The CA file's certificates (DER), read once by [`build_tls_params`]; empty without a CA
    /// file. A driver that builds its own TLS configuration (MySQL) trusts exactly these and
    /// never reads the file again, so a file that became unreadable after the check cannot
    /// silently drop the CA (M2a final review M6).
    pub ca_certs: Arc<[CertificateDer<'static>]>,
    // ## m2b-t01-interfaces
    /// The chain the server presented (leaf first), recorded by the verifier for the
    /// certificate review (Task 3).
    pub presented: Arc<Mutex<Option<PresentedChain>>>,
    /// How far the server certificate was verified, written on a successful verify.
    pub outcome: Arc<Mutex<Option<TlsVerified>>>,
}

/// How far a server certificate was verified (HV-SEC-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TlsVerified {
    Full,
    CaOnly,
    // ## m2b-t01-interfaces
    /// The leaf matched the endpoint's pin (Task 3).
    Pinned,
    No,
}

// ## m2b-t01-interfaces

/// The certificate chain a server presented, leaf first (M2b).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PresentedChain(pub Vec<CertificateDer<'static>>);

impl PresentedChain {
    /// A chain from DER blobs, leaf first (for crates that do not depend on rustls, such as
    /// the fake driver).
    pub fn from_der(ders: Vec<Vec<u8>>) -> PresentedChain {
        PresentedChain(ders.into_iter().map(CertificateDer::from).collect())
    }
}

/// A certificate's place in a presented chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChainRole {
    Leaf,
    Intermediate,
    Root,
}

/// One certificate of a chain, as the review shows it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChainEntry {
    pub subject: String,
    pub role: ChainRole,
}

/// What the certificate review shows (Task 3, Task 19).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CertReview {
    pub endpoint: String,
    pub subject: String,
    pub issuer: String,
    pub not_before_unix: i64,
    pub not_after_unix: i64,
    pub sha256: [u8; 32],
    pub chain: Vec<ChainEntry>,
    /// The pin the endpoint had, when the review is for a changed pin.
    pub pinned: Option<[u8; 32]>,
}

/// The pin endpoint of `tls_host` and `port`: the host in ASCII lowercase without a trailing
/// dot, an IPv6 literal in brackets (`db:5432`, `[::1]:5432`).
///
/// M2b (Task 3 fix round 1): an IP literal is written in its canonical form, so `0::1`,
/// `0:0:0:0:0:0:0:1` and `::1` are one endpoint, and an IPv4-mapped IPv6 address
/// (`::ffff:10.0.0.5`) is its IPv4 address (`10.0.0.5`), which is where it connects.
pub fn pin_endpoint(tls_host: &str, port: u16) -> String {
    let bare = tls_host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(tls_host);
    let bare = bare.strip_suffix('.').unwrap_or(bare).to_ascii_lowercase();
    // ## m2b-t03-tls-pins
    let bare = match bare.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.to_canonical().to_string(),
        Err(_) => bare,
    };
    if bare.contains(':') {
        format!("[{bare}]:{port}")
    } else {
        format!("{bare}:{port}")
    }
}

/// The SHA-256 of a certificate's DER.
pub fn cert_sha256(der: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(der).into()
}

/// A SHA-256 fingerprint as two lines of 16 colon-separated uppercase hex bytes.
pub fn fingerprint_lines(sha256: &[u8; 32]) -> [String; 2] {
    let line = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    };
    [line(&sha256[..16]), line(&sha256[16..])]
}

// ## m2b-t03-tls-pins

/// The review of a presented chain for `endpoint` (B6): the leaf's subject, issuer, validity
/// and SHA-256, and every certificate's subject with its role by position (the leaf first,
/// then intermediates, and the last entry as `Root` when it is self-issued). Names are
/// RFC 4514 strings with their values verbatim (UTF-8 is never escaped). Any certificate that
/// is not one well-formed DER certificate (trailing bytes included), or an empty chain, gives
/// `None`: a caller without a review offers no "trust" action.
pub fn cert_review(
    chain: &PresentedChain,
    endpoint: &str,
    pinned: Option<[u8; 32]>,
) -> Option<CertReview> {
    let leaf_der = chain.0.first()?;
    let parsed = chain
        .0
        .iter()
        .map(|der| parse_certificate(der))
        .collect::<Option<Vec<_>>>()?;
    let last = parsed.len() - 1;
    let entries = parsed
        .iter()
        .enumerate()
        .map(|(i, cert)| ChainEntry {
            subject: rfc4514(cert.subject()),
            role: match i {
                0 => ChainRole::Leaf,
                i if i == last && cert.subject().as_raw() == cert.issuer().as_raw() => {
                    ChainRole::Root
                }
                _ => ChainRole::Intermediate,
            },
        })
        .collect();
    let leaf = &parsed[0];
    Some(CertReview {
        endpoint: endpoint.to_owned(),
        subject: rfc4514(leaf.subject()),
        issuer: rfc4514(leaf.issuer()),
        not_before_unix: leaf.validity().not_before.timestamp(),
        not_after_unix: leaf.validity().not_after.timestamp(),
        sha256: cert_sha256(leaf_der),
        chain: entries,
        pinned,
    })
}

/// One DER certificate with nothing after it; `None` for anything else. x509-parser returns
/// errors for malformed input and never panics on it (a fuzz test holds this).
fn parse_certificate(der: &[u8]) -> Option<x509_parser::certificate::X509Certificate<'_>> {
    match x509_parser::parse_x509_certificate(der) {
        Ok(([], cert)) => Some(cert),
        _ => None,
    }
}

/// A name as RFC 4514 writes it: the RDNs last first, joined by `,`; the attributes of one
/// RDN joined by `+`; RFC 4514's short names (else the dotted OID); string values verbatim
/// with RFC 4514's escapes, other values as `#` and the hex of their DER.
fn rfc4514(name: &x509_parser::x509::X509Name<'_>) -> String {
    let rdns: Vec<&x509_parser::x509::RelativeDistinguishedName<'_>> = name.iter().collect();
    rdns.iter()
        .rev()
        .map(|rdn| {
            rdn.iter()
                .map(|atv| {
                    let oid = atv.attr_type().to_id_string();
                    let key = attribute_short_name(&oid).map_or(oid.clone(), str::to_owned);
                    let value = match attribute_text(atv) {
                        Some(text) => escape_rfc4514(&text),
                        None => format!("#{}", hex(&value_der(atv.attr_value()))),
                    };
                    format!("{key}={value}")
                })
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// RFC 4514 §3's attribute type names.
fn attribute_short_name(oid: &str) -> Option<&'static str> {
    Some(match oid {
        "2.5.4.3" => "CN",
        "2.5.4.7" => "L",
        "2.5.4.8" => "ST",
        "2.5.4.10" => "O",
        "2.5.4.11" => "OU",
        "2.5.4.6" => "C",
        "2.5.4.9" => "STREET",
        "0.9.2342.19200300.100.1.25" => "DC",
        "0.9.2342.19200300.100.1.1" => "UID",
        _ => return None,
    })
}

/// The value of a string-typed attribute: the ASN.1 string types x509-parser reads as UTF-8,
/// and `BMPString` (UTF-16BE).
fn attribute_text(atv: &x509_parser::x509::AttributeTypeAndValue<'_>) -> Option<String> {
    if let Ok(s) = atv.as_str() {
        return Some(s.to_owned());
    }
    let value = atv.attr_value();
    if value.tag() != x509_parser::der_parser::asn1_rs::Tag::BmpString || value.data.len() % 2 != 0
    {
        return None;
    }
    let units = value
        .data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_be_bytes(*pair));
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .ok()
}

/// RFC 4514 §2.4: `"+,;<>\` and NUL anywhere, `#` or a space first, and a space last are
/// escaped. Everything else, non-ASCII text included, stays as it is, except (Task 3 fix
/// round 1) control characters (C0, DEL, C1) and Unicode bidirectional formatting characters,
/// which are written as RFC 4514 hex pairs of their UTF-8 (`\E2\80\AE` for U+202E), so a
/// hostile name cannot reorder or break the text the review shows.
fn escape_rfc4514(value: &str) -> String {
    let last = value.chars().count().saturating_sub(1);
    let mut out = String::with_capacity(value.len());
    for (i, c) in value.chars().enumerate() {
        match c {
            '"' | '+' | ',' | ';' | '<' | '>' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\0' => out.push_str("\\00"),
            c if c.is_control() || is_bidi_format(c) => {
                let mut utf8 = [0u8; 4];
                for b in c.encode_utf8(&mut utf8).bytes() {
                    out.push_str(&format!("\\{b:02X}"));
                }
            }
            '#' if i == 0 => out.push_str("\\#"),
            ' ' if i == 0 || i == last => out.push_str("\\ "),
            _ => out.push(c),
        }
    }
    out
}

/// Unicode's bidirectional formatting characters: ALM, LRM, RLM, the embeddings and
/// overrides (U+202A to U+202E) and the isolates (U+2066 to U+2069).
fn is_bidi_format(c: char) -> bool {
    matches!(
        c,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

/// The DER of a non-string attribute value, for RFC 4514's `#hex` form.
fn value_der(value: &x509_parser::der_parser::asn1_rs::Any<'_>) -> Vec<u8> {
    let header = &value.header;
    let leading = ((header.class() as u8) << 6) | if header.is_constructed() { 0x20 } else { 0 };
    let mut der = Vec::with_capacity(value.data.len() + 8);
    match u8::try_from(header.tag().0) {
        Ok(tag) if tag < 31 => der.push(leading | tag),
        _ => {
            // The high-tag-number form: base 128, most significant group first.
            der.push(leading | 31);
            let tag = header.tag().0;
            let groups = (0..5)
                .rev()
                .map(|i| ((tag >> (7 * i)) & 0x7f) as u8)
                .skip_while(|g| *g == 0)
                .collect::<Vec<_>>();
            let last = groups.len() - 1;
            der.extend(
                groups
                    .iter()
                    .enumerate()
                    .map(|(i, g)| if i == last { *g } else { g | 0x80 }),
            );
        }
    }
    der.extend(der_length(value.data.len()));
    der.extend_from_slice(value.data);
    der
}

/// A DER length: the short form below 128, else the long form.
fn der_length(len: usize) -> Vec<u8> {
    match u8::try_from(len) {
        Ok(short) if short < 0x80 => vec![short],
        _ => {
            let bytes: Vec<u8> = len
                .to_be_bytes()
                .into_iter()
                .skip_while(|b| *b == 0)
                .collect();
            let mut out = vec![0x80 | bytes.len() as u8];
            out.extend(bytes);
            out
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The RFC 5929 `tls-server-end-point` channel-binding hash of a leaf (B9): the hash of the
/// certificate's `signatureAlgorithm`, with MD5 and SHA-1 replaced by SHA-256. RSASSA-PSS uses
/// the hash named in its parameters (SHA-1 when absent, RFC 4055), so SHA-256 then (Q36).
/// Ed25519, Ed448, any other algorithm and malformed DER give `None` (no binding offered).
pub fn tls_server_end_point(leaf_der: &[u8]) -> Option<Vec<u8>> {
    use sha2::Digest as _;
    let cert = parse_certificate(leaf_der)?;
    let alg = &cert.signature_algorithm;
    let hash = match alg.algorithm.to_id_string().as_str() {
        // RSASSA-PSS: the hash from its parameters.
        "1.2.840.113549.1.1.10" => match &alg.parameters {
            None => EndPointHash::Sha256,
            Some(params) => {
                let pss =
                    x509_parser::signature_algorithm::RsaSsaPssParams::try_from(params).ok()?;
                hash_by_digest_oid(&pss.hash_algorithm_oid().to_id_string())?
            }
        },
        oid => hash_by_signature_oid(oid)?,
    };
    Some(match hash {
        EndPointHash::Sha224 => sha2::Sha224::digest(leaf_der).to_vec(),
        EndPointHash::Sha256 => sha2::Sha256::digest(leaf_der).to_vec(),
        EndPointHash::Sha384 => sha2::Sha384::digest(leaf_der).to_vec(),
        EndPointHash::Sha512 => sha2::Sha512::digest(leaf_der).to_vec(),
    })
}

enum EndPointHash {
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

/// The end-point hash of a single-hash signature algorithm (RSA PKCS #1 v1.5, ECDSA, DSA).
fn hash_by_signature_oid(oid: &str) -> Option<EndPointHash> {
    Some(match oid {
        // md5WithRSA, sha1WithRSA, ecdsa-with-SHA1, dsa-with-sha1: SHA-256 (RFC 5929 §4.1).
        "1.2.840.113549.1.1.4"
        | "1.2.840.113549.1.1.5"
        | "1.2.840.10045.4.1"
        | "1.2.840.10040.4.3" => EndPointHash::Sha256,
        // sha224WithRSA, ecdsa-with-SHA224, dsa-with-sha224
        "1.2.840.113549.1.1.14" | "1.2.840.10045.4.3.1" | "2.16.840.1.101.3.4.3.1" => {
            EndPointHash::Sha224
        }
        // sha256WithRSA, ecdsa-with-SHA256, dsa-with-sha256
        "1.2.840.113549.1.1.11" | "1.2.840.10045.4.3.2" | "2.16.840.1.101.3.4.3.2" => {
            EndPointHash::Sha256
        }
        // sha384WithRSA, ecdsa-with-SHA384
        "1.2.840.113549.1.1.12" | "1.2.840.10045.4.3.3" => EndPointHash::Sha384,
        // sha512WithRSA, ecdsa-with-SHA512
        "1.2.840.113549.1.1.13" | "1.2.840.10045.4.3.4" => EndPointHash::Sha512,
        _ => return None,
    })
}

/// The end-point hash for a digest named by OID (RSASSA-PSS parameters).
fn hash_by_digest_oid(oid: &str) -> Option<EndPointHash> {
    Some(match oid {
        // SHA-1: SHA-256 (RFC 5929 §4.1)
        "1.3.14.3.2.26" => EndPointHash::Sha256,
        "2.16.840.1.101.3.4.2.4" => EndPointHash::Sha224,
        "2.16.840.1.101.3.4.2.1" => EndPointHash::Sha256,
        "2.16.840.1.101.3.4.2.2" => EndPointHash::Sha384,
        "2.16.840.1.101.3.4.2.3" => EndPointHash::Sha512,
        _ => return None,
    })
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
    // ## m2b-t01-interfaces
    /// A legacy OpenSSL-encrypted PEM key (`Proc-Type: 4,ENCRYPTED`), which Hacivat cannot
    /// decrypt (Task 3).
    KeyEncryptedLegacy(PathBuf),
    /// The key passphrase does not decrypt the key (Task 3).
    KeyPassphraseWrong(PathBuf),
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
///
/// M2b: `port` names the pin endpoint ([`TlsSettings::pin_for`]). With a pin, the verifier
/// checks only the pin and the leaf's validity window (B5: a pin overrides the chain and name
/// checks, never the dates), in every mode; without one, the mode's check above applies. The
/// verifier records the presented chain before its verdict, and the outcome (`Pinned`,
/// `Full`, `CaOnly` or `No`) on success. A pin also turns the plaintext fallback off (Q13),
/// even with `allow_public_plaintext`. Disable ignores pins (see
/// [`TlsSettings::disable_ignores_pin`]).
///
/// An encrypted PKCS #8 client key is decrypted with `key_passphrase` (PBES2 with PBKDF2
/// (HMAC-SHA-1 or SHA-2) or scrypt, AES-CBC): no passphrase is `KeyEncrypted`, a wrong one
/// `KeyPassphraseWrong`, and a legacy `Proc-Type: 4,ENCRYPTED` key `KeyEncryptedLegacy`.
///
/// The configuration is engine-neutral: PostgreSQL uses it through `tokio-rustls`, MySQL
/// through vendored patch §5 (Q2). TLS session resumption is off, so every handshake runs the
/// verifier and records its own chain and outcome. One `TlsParams` serves one connect attempt:
/// its slots are shared by every handshake through `config`, and each certificate check resets
/// them first.
pub fn build_tls_params(
    settings: &TlsSettings,
    tls_host: &str,
    port: u16,
    class: AddressClass,
    key_passphrase: Option<&SecretString>,
) -> Result<Option<TlsParams>, TlsSetupError> {
    if settings.mode == TlsMode::Disable {
        return Ok(None);
    }
    let server_name = server_name(tls_host)?;
    let pin = settings.pin_for(tls_host, port).map(|p| p.sha256);
    let pinned = pin.is_some();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let ca = match &settings.ca_file {
        Some(path) => Some((read_ca_file(path)?, path.as_path())),
        None => None,
    };
    let ca_certs: Arc<[CertificateDer<'static>]> = ca
        .as_ref()
        .map_or_else(Vec::new, |(ca, _)| ca.certs.clone())
        .into();
    let client = match &settings.client_cert {
        Some(paths) => Some(read_client_cert(paths, key_passphrase)?),
        None => None,
    };

    let failure = Arc::new(Mutex::new(None));
    let inner: Arc<dyn ServerCertVerifier> = match settings.mode {
        // ## m2b-t03-tls-pins: with a pin, `Recording` checks the certificate itself and the
        // inner verifier only checks handshake signatures, so no chain or name check runs.
        _ if pinned => Arc::new(AcceptAnyCert {
            provider: provider.clone(),
        }),
        TlsMode::Require => Arc::new(AcceptAnyCert {
            provider: provider.clone(),
        }),
        TlsMode::VerifyCa => Arc::new(ChainOnly {
            roots: ca.map_or_else(RootCertStore::empty, |(ca, _)| ca.store),
            provider: provider.clone(),
        }),
        TlsMode::Disable | TlsMode::Auto | TlsMode::VerifyFull => platform_verifier(ca, &provider)?,
    };
    let presented = Arc::new(Mutex::new(None));
    let outcome = Arc::new(Mutex::new(None));
    let verifier = Arc::new(Recording {
        inner,
        pin,
        failure: failure.clone(),
        presented: presented.clone(),
        outcome: outcome.clone(),
        verified: match settings.mode {
            _ if pinned => TlsVerified::Pinned,
            TlsMode::Require => TlsVerified::No,
            TlsMode::VerifyCa => TlsVerified::CaOnly,
            TlsMode::Disable | TlsMode::Auto | TlsMode::VerifyFull => TlsVerified::Full,
        },
    });

    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .expect("the ring provider supports TLS 1.3 and 1.2")
        .dangerous()
        .with_custom_certificate_verifier(verifier);
    let mut config = match client {
        // The checks of rustls's `with_client_auth_cert`, on a key ring already holds (so the
        // key's DER never reaches rustls as an owned copy).
        Some(client) => {
            let certified = rustls::sign::CertifiedKey::new(client.chain, client.key);
            match certified.keys_match() {
                // rustls does not treat an unknown consistency as an error either.
                Ok(())
                | Err(rustls::Error::InconsistentKeys(rustls::InconsistentKeys::Unknown)) => {}
                // rustls could not use the certificate itself (a v1 certificate, say).
                Err(rustls::Error::InvalidCertificate(_)) => {
                    return Err(TlsSetupError::CertFileUnreadable(client.cert_path));
                }
                // A key that does not belong to the certificate.
                Err(_) => return Err(TlsSetupError::KeyUnreadable(client.key_path)),
            }
            builder.with_client_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(
                certified,
            )))
        }
        None => builder.with_no_client_auth(),
    };
    // ## m2b-t03-tls-pins: a resumed session skips the verifier, which would leave `presented`
    // and `outcome` empty (and the insecure-auth guard refusing) on a side connection that
    // reuses this configuration (MySQL, patch §5).
    config.resumption = rustls::client::Resumption::disabled();

    Ok(Some(TlsParams {
        settings: settings.clone(),
        config: Arc::new(config),
        server_name,
        plaintext_allowed: settings.mode == TlsMode::Auto
            && class != AddressClass::Public
            && !pinned,
        failure,
        ca_certs,
        presented,
        outcome,
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
    // A share or a device is never opened (N2): reading it could send credentials.
    // ## m2b-t12-profiles: `local_path` also expands `~`, exactly as Validate checked it.
    let Some(local) = crate::paths::local_path(path) else {
        return Err(unreadable());
    };
    crate::paths::fs_probe::note(&local);
    let bytes = std::fs::read(&local).map_err(|_| unreadable())?;
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
    /// The key as ring holds it; the DER it was loaded from is zeroized (Task 3 fix round 1).
    key: Arc<dyn rustls::sign::SigningKey>,
    cert_path: PathBuf,
    key_path: PathBuf,
}

/// Reads a client certificate chain and its key (PKCS #8, PKCS #1 or SEC1, as PEM or DER). An
/// encrypted PKCS #8 key (PEM or DER) is decrypted with `passphrase` (M2b, B25); a legacy
/// OpenSSL-encrypted PEM key is refused before any parsing.
fn read_client_cert(
    paths: &ClientCertPaths,
    passphrase: Option<&SecretString>,
) -> Result<ClientCert, TlsSetupError> {
    let cert_err = || TlsSetupError::CertFileUnreadable(paths.cert.clone());
    let key_err = || TlsSetupError::KeyUnreadable(paths.key.clone());
    // A share or a device is never opened (N2): reading it could send credentials.
    // ## m2b-t12-profiles: `local_path` also expands `~`, exactly as Validate checked it.
    let Some(cert) = crate::paths::local_path(&paths.cert) else {
        return Err(cert_err());
    };
    let Some(key) = crate::paths::local_path(&paths.key) else {
        return Err(key_err());
    };
    crate::paths::fs_probe::note(&cert);
    let cert_bytes = std::fs::read(&cert).map_err(|_| cert_err())?;
    let chain = parse_certs(&cert_bytes).ok_or_else(cert_err)?;
    crate::paths::fs_probe::note(&key);
    let key_bytes = std::fs::read(&key).map_err(|_| key_err())?;
    // ## m2b-t03-tls-pins: every copy of the key's DER is zeroized once ring has loaded it
    // (`Zeroizing`, or pkcs8's `SecretDocument` for a decrypted key, borrowed, never copied).
    let key_bytes = Zeroizing::new(key_bytes);
    let load = |der: &PrivateKeyDer<'_>| {
        rustls::crypto::ring::sign::any_supported_type(der).map_err(|_| key_err())
    };
    let key = match encrypted_key(&key_bytes) {
        Some(EncryptedKey::Legacy) => {
            return Err(TlsSetupError::KeyEncryptedLegacy(paths.key.clone()));
        }
        Some(EncryptedKey::Pkcs8(der)) => {
            let Some(passphrase) = passphrase else {
                return Err(TlsSetupError::KeyEncrypted(paths.key.clone()));
            };
            let plain = decrypt_pkcs8(&der, passphrase, &paths.key)?;
            load(&PrivateKeyDer::Pkcs8(plain.as_bytes().into()))?
        }
        None if is_pem(&key_bytes) => load(&Zeroizing::new(
            PrivateKeyDer::from_pem_slice(&key_bytes).map_err(|_| key_err())?,
        ))?,
        None => load(&PrivateKeyDer::try_from(&key_bytes[..]).map_err(|_| key_err())?)?,
    };
    Ok(ClientCert {
        chain,
        key,
        cert_path: paths.cert.clone(),
        key_path: paths.key.clone(),
    })
}

enum EncryptedKey {
    /// `Proc-Type: 4,ENCRYPTED` (OpenSSL's legacy PEM encryption).
    Legacy,
    /// The DER of an `EncryptedPrivateKeyInfo`, from PEM or as the file was.
    Pkcs8(Vec<u8>),
}

const ENCRYPTED_PEM_BEGIN: &[u8] = b"-----BEGIN ENCRYPTED PRIVATE KEY-----";
const ENCRYPTED_PEM_END: &[u8] = b"-----END ENCRYPTED PRIVATE KEY-----";

/// Whether a key file holds an encrypted key, and which kind. An `ENCRYPTED PRIVATE KEY` PEM
/// block that does not decode is still encrypted (an empty DER): it is reported as encrypted
/// until a passphrase is offered, and then as unreadable.
fn encrypted_key(bytes: &[u8]) -> Option<EncryptedKey> {
    if contains(bytes, b"Proc-Type: 4,ENCRYPTED") {
        return Some(EncryptedKey::Legacy);
    }
    if let Some(start) = find(bytes, ENCRYPTED_PEM_BEGIN) {
        let block = &bytes[start..];
        let block = match find(block, ENCRYPTED_PEM_END) {
            Some(end) => &block[..end + ENCRYPTED_PEM_END.len()],
            None => block,
        };
        let der = pkcs8::der::pem::decode_vec(block)
            .ok()
            .filter(|(label, _)| *label == "ENCRYPTED PRIVATE KEY")
            .map(|(_, der)| der)
            .unwrap_or_default();
        return Some(EncryptedKey::Pkcs8(der));
    }
    if !is_pem(bytes) && pkcs8::EncryptedPrivateKeyInfo::try_from(bytes).is_ok() {
        return Some(EncryptedKey::Pkcs8(bytes.to_vec()));
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The most PBKDF2 iterations and scrypt memory a key file may ask for (OpenSSL writes 2048
/// iterations, or scrypt with N = 2^14 and r = 8, 16 MiB). A crafted file asking for more
/// would hold the core runtime for minutes, so it is refused as unreadable.
const MAX_PBKDF2_ITERATIONS: u32 = 10_000_000;
const MAX_SCRYPT_BYTES: u64 = 256 * 1024 * 1024;

/// Decrypts an `EncryptedPrivateKeyInfo` (PBES2) with `passphrase`. A decryption that fails,
/// or one that yields something other than a PKCS #8 key, is a wrong passphrase (AES-CBC
/// padding can be valid by chance); an unsupported or too costly scheme is unreadable.
fn decrypt_pkcs8(
    der: &[u8],
    passphrase: &SecretString,
    path: &Path,
) -> Result<pkcs8::SecretDocument, TlsSetupError> {
    use pkcs8::pkcs5::{EncryptionScheme, pbes2::Kdf};
    use secrecy::ExposeSecret as _;
    let unreadable = || TlsSetupError::KeyUnreadable(path.to_path_buf());
    let info = pkcs8::EncryptedPrivateKeyInfo::try_from(der).map_err(|_| unreadable())?;
    let EncryptionScheme::Pbes2(params) = &info.encryption_algorithm else {
        return Err(unreadable());
    };
    let affordable = match &params.kdf {
        Kdf::Pbkdf2(p) => p.iteration_count <= MAX_PBKDF2_ITERATIONS,
        Kdf::Scrypt(p) => {
            let bytes = 128u64
                .saturating_mul(u64::from(p.block_size))
                .saturating_mul(p.cost_parameter)
                .saturating_mul(u64::from(p.parallelization));
            bytes <= MAX_SCRYPT_BYTES
        }
        // A KDF this pkcs5 version does not know cannot decrypt either.
        _ => false,
    };
    if !affordable {
        return Err(unreadable());
    }
    let wrong = || TlsSetupError::KeyPassphraseWrong(path.to_path_buf());
    let plain = match info.decrypt(passphrase.expose_secret().as_bytes()) {
        Ok(plain) => plain,
        // pkcs5 0.7 reports bad padding as `EncryptFailed`.
        Err(pkcs8::Error::EncryptedPrivateKey(
            pkcs8::pkcs5::Error::DecryptFailed | pkcs8::pkcs5::Error::EncryptFailed,
        )) => return Err(wrong()),
        // Valid padding by chance leaves bytes that are not DER (or not a key).
        Err(pkcs8::Error::Asn1(_)) => return Err(wrong()),
        Err(_) => return Err(unreadable()),
    };
    if pkcs8::PrivateKeyInfo::try_from(plain.as_bytes()).is_err() {
        return Err(wrong());
    }
    Ok(plain)
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

/// Wraps a verifier and records its first error into `TlsParams::failure`; M2b: also the
/// presented chain, and the outcome of a successful certificate check. With a pin, it checks
/// the certificate itself (the pin and the leaf's dates) and uses `inner` only for handshake
/// signatures.
#[derive(Debug)]
struct Recording {
    inner: Arc<dyn ServerCertVerifier>,
    // ## m2b-t03-tls-pins
    /// The endpoint's pin (the SHA-256 of the leaf DER).
    pin: Option<[u8; 32]>,
    failure: Arc<Mutex<Option<TlsFailure>>>,
    presented: Arc<Mutex<Option<PresentedChain>>>,
    outcome: Arc<Mutex<Option<TlsVerified>>>,
    /// What a successful check of `inner` means.
    verified: TlsVerified,
}

impl Recording {
    /// Records the first error into `failure`. M2b (Task 3 fix round 1): any verifier error
    /// also clears `outcome`, so a certificate check that passed is not reported once the
    /// handshake signature fails (an impostor replaying the pinned leaf): `outcome` holds a
    /// value only when every check passed.
    fn record<T>(&self, result: Result<T, rustls::Error>) -> Result<T, rustls::Error> {
        if let Err(e) = &result {
            *self.outcome.lock().unwrap_or_else(PoisonError::into_inner) = None;
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
        let chain = std::iter::once(end_entity)
            .chain(intermediates)
            .map(|c| c.clone().into_owned())
            .collect();
        // ## m2b-t03-tls-pins: this is a handshake's first verifier call, so it starts the
        // record afresh (the chain before any verdict, no outcome or failure yet).
        *self
            .presented
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(PresentedChain(chain));
        *self.outcome.lock().unwrap_or_else(PoisonError::into_inner) = None;
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let checked = match self.pin {
            Some(pin) => check_pin(end_entity, &pin, now),
            None => self.inner.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            ),
        };
        let verified = self.record(checked)?;
        *self.outcome.lock().unwrap_or_else(PoisonError::into_inner) = Some(self.verified);
        Ok(verified)
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

// ## m2b-t03-tls-pins

/// The leaf is not the pinned certificate. It travels inside `CertificateError::Other`, and
/// [`TlsFailure::from_rustls`] maps it back to [`TlsFailure::PinMismatch`] (T1 review M1).
#[derive(Debug)]
struct PinMismatchError;

impl std::fmt::Display for PinMismatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the server certificate is not the pinned certificate")
    }
}

impl std::error::Error for PinMismatchError {}

/// B5: the leaf's SHA-256 must be the pin, and now must lie in the leaf's validity window (a
/// pin never extends the dates). Nothing else about the chain or the name is checked.
fn check_pin(
    end_entity: &CertificateDer<'_>,
    pin: &[u8; 32],
    now: UnixTime,
) -> Result<ServerCertVerified, rustls::Error> {
    use rustls::CertificateError as C;
    if cert_sha256(end_entity) != *pin {
        return Err(rustls::Error::InvalidCertificate(C::Other(
            rustls::OtherError(Arc::new(PinMismatchError)),
        )));
    }
    let cert =
        parse_certificate(end_entity).ok_or(rustls::Error::InvalidCertificate(C::BadEncoding))?;
    let now = i64::try_from(now.as_secs()).unwrap_or(i64::MAX);
    let validity = cert.validity();
    if now < validity.not_before.timestamp() {
        return Err(rustls::Error::InvalidCertificate(C::NotValidYet));
    }
    if now > validity.not_after.timestamp() {
        return Err(rustls::Error::InvalidCertificate(C::Expired));
    }
    Ok(ServerCertVerified::assertion())
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
            // The pg17-tls fixture (a 10-year self-signed CA:TRUE certificate) on macOS.
            (
                "“localhost” certificate is not standards compliant: -67901",
                TlsFailure::UnknownIssuer,
            ),
            ("x: -67902", TlsFailure::UnknownIssuer),
            // errSecCreateChainFailed is -25318; -67810 is errSecBlockSizeMismatch.
            ("x: -25318", TlsFailure::UnknownIssuer),
            ("x: -67612", TlsFailure::UnknownIssuer),
            ("x: -67810", TlsFailure::Other),
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

    // ## m2b-t01-interfaces

    #[test]
    fn pin_endpoints_and_fingerprints() {
        assert_eq!(pin_endpoint("db", 5432), "db:5432");
        assert_eq!(pin_endpoint("DB.Acme.IO.", 5432), "db.acme.io:5432");
        assert_eq!(pin_endpoint("::1", 5432), "[::1]:5432");
        assert_eq!(pin_endpoint("[::1]", 5432), "[::1]:5432");
        let sha = cert_sha256(b"abc");
        assert_eq!(
            fingerprint_lines(&sha),
            [
                "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23",
                "B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD"
            ]
        );
    }

    #[test]
    fn a_pin_forbids_plaintext_on_its_endpoint_only() {
        let settings = TlsSettings {
            pins: vec![CertPin {
                endpoint: "10.0.0.5:5432".into(),
                sha256: [9; 32],
            }],
            ..Default::default()
        };
        let params = build_tls_params(&settings, "10.0.0.5", 5432, AddressClass::Private, None)
            .unwrap()
            .unwrap();
        assert!(
            !params.plaintext_allowed,
            "a pin forbids the fallback (Q13)"
        );
        // Another port is another endpoint: no pin applies there.
        let other = build_tls_params(&settings, "10.0.0.5", 6432, AddressClass::Private, None)
            .unwrap()
            .unwrap();
        assert!(other.plaintext_allowed);
    }

    // ## m2b-t03-tls-pins

    fn pinned(endpoint: &str) -> TlsSettings {
        TlsSettings {
            pins: vec![CertPin {
                endpoint: endpoint.into(),
                sha256: [9; 32],
            }],
            ..Default::default()
        }
    }

    /// The endpoint rule (ADR 0003, m2b-t03): the host is compared in ASCII lowercase without
    /// a trailing dot, IPv6 in brackets, and the port must be the same.
    #[test]
    fn pin_lookup_normalizes_the_host_and_keeps_the_port() {
        let s = pinned("DB.Acme.IO.:5432");
        assert!(s.pin_for("db.acme.io", 5432).is_some());
        assert!(s.pin_for("DB.ACME.IO", 5432).is_some());
        assert!(s.pin_for("db.acme.io", 5433).is_none());
        assert!(s.pin_for("db2.acme.io", 5432).is_none());
        for stored in ["::1:5432", "[::1]:5432"] {
            assert!(pinned(stored).pin_for("::1", 5432).is_some(), "{stored}");
            assert!(pinned(stored).pin_for("[::1]", 5432).is_some(), "{stored}");
        }
        assert!(pinned("db").pin_for("db", 5432).is_none());
        assert!(pinned("db:x").pin_for("db", 5432).is_none());
        assert!(pinned("").pin_for("db", 5432).is_none());
    }

    /// T1 review M2 (Q13): Disable never checks a pin, so a pin under Disable is flagged.
    #[test]
    fn disable_with_a_pin_is_flagged() {
        let s = TlsSettings {
            mode: TlsMode::Disable,
            ..pinned("db:5432")
        };
        assert!(s.disable_ignores_pin("db", 5432));
        assert!(!s.disable_ignores_pin("db", 5433));
        let auto = TlsSettings {
            mode: TlsMode::Auto,
            ..s.clone()
        };
        assert!(!auto.disable_ignores_pin("db", 5432));
        assert!(
            build_tls_params(&s, "db", 5432, AddressClass::Public, None)
                .unwrap()
                .is_none()
        );
    }

    /// T1 review M1: the rustls error alone classifies a mismatch, so a driver that reads a
    /// fresh `failure` slot still reports `tls.pin_changed`.
    #[test]
    fn a_pin_mismatch_classifies_from_the_rustls_error() {
        let err = check_pin(
            &CertificateDer::from(vec![0x30, 0x00]),
            &[9; 32],
            UnixTime::now(),
        )
        .unwrap_err();
        assert_eq!(TlsFailure::from_rustls(&err), TlsFailure::PinMismatch);
        // An unrelated `Other` error is still left to the platform mapping.
        let other = rustls::Error::InvalidCertificate(rustls::CertificateError::Other(
            rustls::OtherError(Arc::new(std::io::Error::other("PinMismatch"))),
        ));
        assert_ne!(TlsFailure::from_rustls(&other), TlsFailure::PinMismatch);
    }

    #[test]
    fn rfc4514_escapes_specials_and_keeps_utf8() {
        assert_eq!(escape_rfc4514("a,b+c"), "a\\,b\\+c");
        assert_eq!(escape_rfc4514("#x "), "\\#x\\ ");
        assert_eq!(escape_rfc4514(" x"), "\\ x");
        assert_eq!(escape_rfc4514("q\"<>;\\"), "q\\\"\\<\\>\\;\\\\");
        assert_eq!(escape_rfc4514("İş\0"), "İş\\00");
        assert_eq!(escape_rfc4514(""), "");
    }

    /// Review Minor 4: controls and bidi formatting characters are hex pairs; letters stay.
    #[test]
    fn rfc4514_escapes_controls_and_bidi_marks() {
        assert_eq!(escape_rfc4514("a\u{202e}b"), "a\\E2\\80\\AEb");
        assert_eq!(escape_rfc4514("a\nb\rc\u{7f}"), "a\\0Ab\\0Dc\\7F");
        assert_eq!(escape_rfc4514("\u{85}"), "\\C2\\85");
        for c in [
            '\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{2066}', '\u{2069}',
        ] {
            let escaped = escape_rfc4514(&format!("x{c}y"));
            assert!(!escaped.contains(c), "{c:?}");
            assert!(
                escaped.starts_with("x\\") && escaped.ends_with('y'),
                "{escaped}"
            );
        }
        assert_eq!(escape_rfc4514("Müşteri İşlemleri"), "Müşteri İşlemleri");
    }

    /// Review Minor 3: IP literals are canonical, IPv4-mapped addresses are IPv4.
    #[test]
    fn pin_endpoints_canonicalize_ip_literals() {
        for host in ["::1", "[::1]", "0::1", "0:0:0:0:0:0:0:1", "[0:0::1]"] {
            assert_eq!(pin_endpoint(host, 5432), "[::1]:5432", "{host}");
        }
        assert_eq!(pin_endpoint("2001:DB8::0:1", 5432), "[2001:db8::1]:5432");
        assert_eq!(pin_endpoint("::ffff:10.0.0.5", 5432), "10.0.0.5:5432");
        assert_eq!(pin_endpoint("[::FFFF:a00:5]", 5432), "10.0.0.5:5432");
        assert_eq!(pin_endpoint("10.0.0.5", 5432), "10.0.0.5:5432");
        // Not an IP literal: as typed, in ASCII lowercase (a zone id included).
        assert_eq!(pin_endpoint("fe80::1%EN0", 5432), "[fe80::1%en0]:5432");
        assert_eq!(pin_endpoint("010.0.0.5", 5432), "010.0.0.5:5432");
        // Stored endpoints compare in the same form.
        assert!(pinned("[0:0::1]:5432").pin_for("::1", 5432).is_some());
        assert!(
            pinned("::ffff:10.0.0.5:5432")
                .pin_for("10.0.0.5", 5432)
                .is_some()
        );
        assert!(
            pinned("10.0.0.5:5432")
                .pin_for("[::ffff:10.0.0.5]", 5432)
                .is_some()
        );
    }

    #[test]
    fn der_lengths_use_the_short_and_long_forms() {
        assert_eq!(der_length(5), [5]);
        assert_eq!(der_length(127), [127]);
        assert_eq!(der_length(128), [0x81, 128]);
        assert_eq!(der_length(300), [0x82, 1, 44]);
    }
}
