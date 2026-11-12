//! Non-interactive TLS (HV-SEC-02): modes, settings, the rustls client configuration and what
//! a handshake reports. rustls uses the ring provider only (P5). Bodies in Task 5 except
//! [`install_crypto_provider`], which works since Task 1.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rustls::pki_types::ServerName;

use crate::address::AddressClass;

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
}

/// Builds the client configuration for `settings` toward `tls_host`; `None` means Disable
/// (Task 5). The stub answers `None`.
pub fn build_tls_params(
    _settings: &TlsSettings,
    _tls_host: &str,
    _class: AddressClass,
) -> Result<Option<TlsParams>, TlsSetupError> {
    Ok(None)
}

/// Installs rustls's ring provider as the process default, once (P5). Calling it again, or
/// after another provider was installed, does nothing. `CoreRuntime::new` calls it.
pub fn install_crypto_provider() {
    // `Err` only means a provider is already installed.
    let _ = rustls::crypto::ring::default_provider().install_default();
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
}
