//! Secrets (HV-SEC-03, -04, -05): the secret store and its backends (OS keyring, memory, the
//! vault interface), credential files (`.pgpass`, `pg_service.conf`), auth providers and URL
//! scrubbing. Secrets travel as [`SecretString`] and never reach `Debug`, logs or files.
//! Bodies in Tasks 4 and 8 unless an item says it is working.

pub mod backend;
pub mod files;
pub mod providers;
pub mod scrub;
pub mod store;

pub use secrecy::{ExposeSecret, SecretString};

pub use backend::{KeyringBackend, MemoryBackend, SecretBackend, SecretError, VaultBackend};
pub use files::{
    PgPassMatch, PgPassWarning, PgService, pg_service_path, pgpass_lookup, pgpass_path,
    read_pg_service,
};
pub use providers::{
    AuthError, AuthMaterial, AuthOutcome, AuthProviderId, AuthRequest, CredentialFiles,
    SecretSource, resolve_auth,
};
pub use scrub::{scrub_options, scrub_url};
pub use store::{Retention, SecretKey, SecretSlot, SecretStore, Stored};
