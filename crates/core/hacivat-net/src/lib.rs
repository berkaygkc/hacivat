//! Byte transports for the connect pipeline (HV-CONN-01 transport, HV-SEC-02 TLS modes):
//! boxed streams and their factories, direct TCP and Unix connects, address classes, the
//! loopback forwarder (R18), cleanup guards and non-interactive TLS. Function bodies arrive in
//! Task 5 unless a module says it is working. M2b adds SSH (`ssh`) and TLS pins and review.

pub mod address;
pub mod cleanup;
pub mod direct;
pub mod forwarder;
// ## m2a-finalfix
pub mod paths;
// ## m2b-t01-interfaces
pub mod ssh;
pub mod stream;
pub mod tls;

pub use address::{AddressClass, classify_host, classify_ip, ssh_address_class};
pub use cleanup::CleanupGuard;
pub use direct::{NetError, Target, direct, resolve};
pub use forwarder::{ForwarderAddr, ForwarderMode, LoopbackForwarder};
pub use paths::is_network_path;
pub use stream::{AsyncStream, BoxedStream, StreamFactory};
pub use tls::{
    CertPin, CertReview, ChainEntry, ChainRole, ClientCertPaths, PresentedChain, TlsFailure,
    TlsMode, TlsParams, TlsSettings, TlsSetupError, TlsSummary, TlsVerified, build_tls_params,
    cert_review, cert_sha256, fingerprint_lines, install_crypto_provider, pin_endpoint,
    tls_server_end_point,
};
