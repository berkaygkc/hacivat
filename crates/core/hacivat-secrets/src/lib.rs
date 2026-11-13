//! Secrets (HV-SEC-03, -04, -05): the secret store and its backends (OS keyring, memory, the
//! vault interface), credential files (`.pgpass`, `pg_service.conf`), auth providers and URL
//! scrubbing. Secrets travel as [`SecretString`] and never reach `Debug`, logs or files.

pub mod backend;
pub mod files;
pub mod providers;
pub mod scrub;
pub mod store;

pub use secrecy::{ExposeSecret, SecretString};

pub use backend::{
    BackendChoice, FallbackBackend, FileVault, KeyringBackend, MemoryBackend, SecretBackend,
    SecretError, VaultBackend, VaultParams,
};
pub use files::{
    MAX_CREDENTIAL_FILE_BYTES, PgPassMatch, PgPassWarning, PgService, PgServiceFile,
    ServiceFileWarning, pg_service_path, pg_service_paths, pgpass_lookup, pgpass_path,
    read_pg_service,
};
pub use providers::{
    AuthError, AuthMaterial, AuthOutcome, AuthProviderId, AuthRequest, CredentialFiles,
    SecretSource, resolve_auth,
};
pub use scrub::{scrub_message, scrub_options, scrub_url};
pub use store::{MAX_SSH_HOP, Retention, SecretKey, SecretSlot, SecretStore, Stored};
