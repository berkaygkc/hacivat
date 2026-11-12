//! Auth providers: where a connect's credentials come from (HV-SEC-04). Bodies in Task 8.

use std::path::PathBuf;

use hacivat_proto::{DbmsKind, ProfileId};
use secrecy::SecretString;

use crate::backend::SecretError;
use crate::files::PgPassWarning;
use crate::store::{Retention, SecretStore};

/// How a profile authenticates (HV-SEC-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthProviderId {
    Password,
    None,
    PgPass,
    ClientCert,
}

impl AuthProviderId {
    pub const ALL: [AuthProviderId; 4] = [
        AuthProviderId::Password,
        AuthProviderId::None,
        AuthProviderId::PgPass,
        AuthProviderId::ClientCert,
    ];

    /// The stable ID: `password`, `none`, `pgpass`, `client_cert`.
    pub fn id(self) -> &'static str {
        match self {
            AuthProviderId::Password => "password",
            AuthProviderId::None => "none",
            AuthProviderId::PgPass => "pgpass",
            AuthProviderId::ClientCert => "client_cert",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }
}

/// Where the password in an [`AuthMaterial`] came from (HV-SEC-04).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SecretSource {
    /// The dialog's unsaved field.
    Draft,
    Store(Retention),
    PgPass {
        path: PathBuf,
        line: u32,
    },
    Prompt,
    NotNeeded,
}

/// The credentials a connect uses (HV-SEC-04).
#[derive(Debug)]
pub struct AuthMaterial {
    pub user: Option<String>,
    pub password: Option<SecretString>,
    pub source: SecretSource,
}

/// The credential files in effect (HV-SEC-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CredentialFiles {
    pub pgpass: Option<PathBuf>,
    pub pg_service: Option<PathBuf>,
}

impl CredentialFiles {
    /// The files the environment points at (`pgpass_path`, `pg_service_path`; Task 8).
    pub fn from_env() -> Self {
        CredentialFiles::default()
    }
}

/// One credential lookup (HV-SEC-04).
#[derive(Debug)]
pub struct AuthRequest<'a> {
    pub profile: ProfileId,
    pub provider: AuthProviderId,
    pub engine: DbmsKind,
    pub host: &'a str,
    pub port: u16,
    pub database: Option<&'a str>,
    pub user: Option<&'a str>,
    pub retention: Retention,
    /// The dialog's password field, when a test or connect runs from it.
    pub draft: Option<&'a SecretString>,
}

/// What [`resolve_auth`] found (HV-SEC-04).
#[derive(Debug)]
pub enum AuthOutcome {
    Ready(AuthMaterial),
    NeedsPrompt { user: String },
    Failed(AuthError),
}

/// Why credentials could not be resolved (HV-SEC-04).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AuthError {
    PgPassNoMatch,
    PgPassFile(PgPassWarning),
    Store(SecretError),
}

/// Draft, then `.pgpass` (provider `PgPass`), then the store, then `NeedsPrompt`; `None` needs
/// nothing (Task 8). Blocking.
pub fn resolve_auth(
    _req: &AuthRequest<'_>,
    _store: &SecretStore,
    _files: &CredentialFiles,
) -> AuthOutcome {
    AuthOutcome::Failed(AuthError::Store(SecretError::Unavailable))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_round_trip() {
        for a in AuthProviderId::ALL {
            assert_eq!(AuthProviderId::from_id(a.id()), Some(a));
        }
        assert_eq!(AuthProviderId::ClientCert.id(), "client_cert");
    }
}
