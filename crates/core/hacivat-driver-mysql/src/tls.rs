//! `SslOpts` from TLS settings, with OS roots as DER (P10, HV-SEC-02).
//!
//! `mysql_async` builds its own rustls `ClientConfig` (a `WebPkiServerVerifier` over the roots
//! it is given), so Hacivat's `TlsParams::config` cannot be used here. Trust is the CA file
//! plus the OS roots from `rustls-native-certs`, each handed over as DER; the compiled-in
//! `webpki-roots` are switched off.

use std::sync::OnceLock;

use hacivat_net::{TlsMode, TlsParams, TlsVerified};
use mysql_async::{ClientIdentity, SslOpts};
use rustls::RootCertStore;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;

/// The mode part of the TLS options; `None` sends no TLS request (Disable).
///
/// Every other mode trusts only the roots the connect path adds (never `webpki-roots`):
/// - Require: any certificate, any name (`accept_invalid_certs` and `skip_domain_validation`);
/// - Auto and VerifyFull: the chain and the host name are verified;
/// - VerifyCa: never reached, because Validate rejects it for MySQL and MariaDB (P11) and the
///   connect path refuses it; it maps to the VerifyFull checks, the stricter choice.
pub fn ssl_opts(mode: TlsMode) -> Option<SslOpts> {
    let base = SslOpts::default().with_disable_built_in_roots(true);
    match mode {
        TlsMode::Disable => None,
        TlsMode::Require => Some(
            base.with_danger_accept_invalid_certs(true)
                .with_danger_skip_domain_validation(true),
        ),
        TlsMode::Auto | TlsMode::VerifyCa | TlsMode::VerifyFull => Some(base),
    }
}

/// How far a connection in `mode` verified the server (HV-SEC-02).
pub(crate) fn verified(mode: TlsMode) -> TlsVerified {
    match mode {
        TlsMode::Auto | TlsMode::VerifyFull => TlsVerified::Full,
        TlsMode::VerifyCa => TlsVerified::CaOnly,
        TlsMode::Disable | TlsMode::Require => TlsVerified::No,
    }
}

/// The OS trust roots as DER, loaded once (a blocking read of the system store). Only roots
/// that rustls accepts as trust anchors are kept: `mysql_async` adds each root with
/// `RootCertStore::add` and fails the whole connect on the first one it cannot parse.
pub fn os_roots() -> &'static [Vec<u8>] {
    static ROOTS: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        rustls_native_certs::load_native_certs()
            .certs
            .into_iter()
            .filter(|c| RootCertStore::empty().add(c.clone()).is_ok())
            .map(|c| c.as_ref().to_vec())
            .collect()
    })
}

/// The certificates of a CA file (PEM with any number of certificates, or one DER
/// certificate), as DER. An unreadable file gives no certificates: `build_tls_params` already
/// checked it, and a file that vanished since then simply adds no trust.
pub(crate) fn ca_file_der(bytes: &[u8]) -> Vec<Vec<u8>> {
    if bytes.windows(11).any(|w| w == b"-----BEGIN ") {
        CertificateDer::pem_slice_iter(bytes)
            .filter_map(Result::ok)
            .map(|c| c.as_ref().to_vec())
            .collect()
    } else if bytes.is_empty() {
        Vec::new()
    } else {
        vec![bytes.to_vec()]
    }
}

/// Reads the CA file of `tls`, if any (a small blocking read).
pub(crate) fn read_ca(tls: &TlsParams) -> Vec<Vec<u8>> {
    tls.settings
        .ca_file
        .as_deref()
        .and_then(|p| std::fs::read(p).ok())
        .map(|b| ca_file_der(&b))
        .unwrap_or_default()
}

/// The full TLS options for a connect: the mode part, the CA-file and OS roots as DER, the
/// client identity and, through the forwarder, the real host name to verify.
///
/// The client certificate and key are passed as paths, so no key material is copied into
/// `Opts`; `mysql_async` reads them during the handshake.
pub(crate) fn full_ssl_opts(
    tls: &TlsParams,
    ca: Vec<Vec<u8>>,
    host_override: Option<&str>,
) -> Option<SslOpts> {
    let mut roots: Vec<Vec<u8>> = ca;
    roots.extend(os_roots().iter().cloned());
    let identity = tls
        .settings
        .client_cert
        .as_ref()
        .map(|c| ClientIdentity::new(c.cert.clone().into(), c.key.clone().into()));
    let mut opts = ssl_opts(tls.settings.mode)?
        .with_root_certs(roots.into_iter().map(Into::into).collect())
        .with_client_identity(identity);
    if let Some(host) = host_override {
        opts = opts.with_danger_tls_hostname_override(Some(host.to_owned()));
    }
    Some(opts)
}
