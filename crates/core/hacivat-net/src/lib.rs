//! Byte transports for the connect pipeline (HV-CONN-01 transport, HV-SEC-02 TLS modes):
//! boxed streams and their factories, direct TCP and Unix connects, address classes, the
//! loopback forwarder (R18), cleanup guards and non-interactive TLS. Function bodies arrive in
//! Task 5 unless a module says it is working.

pub mod address;
pub mod cleanup;
pub mod direct;
pub mod forwarder;
pub mod stream;
pub mod tls;

pub use address::{AddressClass, classify_host, classify_ip};
pub use cleanup::CleanupGuard;
pub use direct::{NetError, Target, direct, resolve};
pub use forwarder::{ForwarderAddr, ForwarderMode, LoopbackForwarder};
pub use stream::{AsyncStream, BoxedStream, StreamFactory};
pub use tls::{
    ClientCertPaths, TlsFailure, TlsMode, TlsParams, TlsSettings, TlsSetupError, TlsSummary,
    TlsVerified, build_tls_params, install_crypto_provider,
};
