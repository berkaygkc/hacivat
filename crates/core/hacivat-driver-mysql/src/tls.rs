//! TLS through Hacivat's own rustls configuration (Q2, vendored patch §5) and the
//! outcome-based password guard (Q20, patch §2).
//!
//! - Every TLS mode (Auto, Require, VerifyCa, VerifyFull) hands `mysql_async` the attempt's
//!   `TlsParams.config` as is: the pin-aware recording verifier, the platform verifier (or
//!   `ChainOnly` for VerifyCa), the CA file and the client identity Hacivat read and decrypted.
//!   `mysql_async` builds no `ClientConfig`, root store or `DangerousVerifier` of its own, and
//!   never reads a certificate or key file.
//! - The name to verify (and send as SNI) is always the profile's TLS host, through
//!   `with_danger_tls_hostname_override` (the forwarder dials `127.0.0.1`).
//! - `mysql_async` caches one connector per `SslOpts` value, so a fresh `SslOpts` is built for
//!   every attempt (T8 carry-in).

use std::sync::{Arc, PoisonError};

use hacivat_driver::InsecureAuth;
use hacivat_net::{TlsMode, TlsParams, TlsVerified};
use mysql_async::SslOpts;

/// The TLS options of one handshake attempt: Hacivat's `ClientConfig` and the name to verify.
pub(crate) fn ssl_opts(tls: &TlsParams, tls_host: &str) -> SslOpts {
    SslOpts::default()
        .with_rustls_client_config(tls.config.clone())
        .with_danger_tls_hostname_override(Some(tls_host.to_owned()))
}

/// Whether the password may go to a server that asked for it by full authentication
/// (`caching_sha2_password` "perform full authentication": the cleartext password over TLS, or
/// the RSA-encrypted one without) (B8, Q20).
///
/// `mode` is the mode the attempt's TLS ran in: `Disable` for a plaintext attempt (including
/// Auto's plaintext fallback), whose `outcome` is never read. For every TLS mode the answer
/// depends on the outcome only, never on the mode: `Allow`, or a server verified as `Full`,
/// `CaOnly` or `Pinned` (so Require with a pin passes and Require without one does not).
#[doc(hidden)]
pub fn guard_allows(mode: TlsMode, outcome: Option<TlsVerified>, insecure: InsecureAuth) -> bool {
    insecure == InsecureAuth::Allow
        || (mode != TlsMode::Disable
            && matches!(
                outcome,
                Some(TlsVerified::Full | TlsVerified::CaOnly | TlsVerified::Pinned)
            ))
}

/// The patch §2 guard of one attempt. With TLS it reads the attempt's `TlsParams.outcome` at
/// the moment the server asks for the password, after this connection's handshake ran the
/// verifier (session resumption is off, so every handshake writes it); `None` (no TLS, or a
/// slot another handshake just reset) refuses.
pub(crate) fn password_guard(
    tls: Option<&TlsParams>,
    insecure: InsecureAuth,
) -> Arc<dyn Fn() -> bool + Send + Sync> {
    match tls {
        Some(tls) => {
            let mode = tls.settings.mode;
            let outcome = tls.outcome.clone();
            Arc::new(move || {
                let seen = *outcome.lock().unwrap_or_else(PoisonError::into_inner);
                guard_allows(mode, seen, insecure)
            })
        }
        None => Arc::new(move || guard_allows(TlsMode::Disable, None, insecure)),
    }
}

/// How far the server of a TLS connection was verified: the outcome its handshake recorded.
/// A missing outcome is `No`, never a guess from the mode.
pub(crate) fn verified(tls: &TlsParams) -> TlsVerified {
    tls.outcome
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .unwrap_or(TlsVerified::No)
}
