//! Auth providers: where a connect's credentials come from (HV-SEC-04).

use std::path::PathBuf;

use hacivat_proto::{DbmsKind, ProfileId};
use secrecy::SecretString;

use crate::backend::SecretError;
use crate::files::{PgPassWarning, pg_service_path, pgpass_lookup, pgpass_path};
use crate::store::{Retention, SecretKey, SecretSlot, SecretStore};

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
    /// The files the environment points at ([`pgpass_path`], [`pg_service_path`]). Blocking:
    /// it checks which `pg_service.conf` exists.
    pub fn from_env() -> Self {
        CredentialFiles {
            pgpass: pgpass_path(),
            pg_service: pg_service_path(),
        }
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

/// Resolves a connect's credentials (HV-SEC-04), in this order:
///
/// 1. `None` and `ClientCert` need no password: `NotNeeded`, without reading anything.
/// 2. The dialog's draft, when there is one.
/// 3. `.pgpass`, when the provider is `PgPass`. No match (or no file) is `PgPassNoMatch`; an
///    ignored file is `PgPassFile`. The database defaults to the user name, as in libpq.
/// 4. The store, under the request's retention.
/// 5. `NeedsPrompt`.
///
/// A failure is returned as is, never retried with another source. Blocking: it reads files
/// and may call the keyring; run it on the core runtime's blocking pool.
pub fn resolve_auth(
    req: &AuthRequest<'_>,
    store: &SecretStore,
    files: &CredentialFiles,
) -> AuthOutcome {
    let user = req.user.map(str::to_string);
    if matches!(
        req.provider,
        AuthProviderId::None | AuthProviderId::ClientCert
    ) {
        return AuthOutcome::Ready(AuthMaterial {
            user,
            password: None,
            source: SecretSource::NotNeeded,
        });
    }
    if let Some(draft) = req.draft {
        return AuthOutcome::Ready(AuthMaterial {
            user,
            password: Some(draft.clone()),
            source: SecretSource::Draft,
        });
    }
    if req.provider == AuthProviderId::PgPass {
        let Some(path) = &files.pgpass else {
            return AuthOutcome::Failed(AuthError::PgPassNoMatch);
        };
        let database = req.database.or(req.user).unwrap_or_default();
        return match pgpass_lookup(path, req.host, req.port, database, req.user) {
            Ok(Some(m)) => AuthOutcome::Ready(AuthMaterial {
                user: m.user,
                password: m.password,
                source: SecretSource::PgPass {
                    path: path.clone(),
                    line: m.line,
                },
            }),
            Ok(None) => AuthOutcome::Failed(AuthError::PgPassNoMatch),
            Err(warning) => AuthOutcome::Failed(AuthError::PgPassFile(warning)),
        };
    }
    let key = SecretKey {
        profile: req.profile,
        slot: SecretSlot::DbPassword,
    };
    match store.get(&key, req.retention) {
        Ok(Some(password)) => AuthOutcome::Ready(AuthMaterial {
            user,
            password: Some(password),
            source: SecretSource::Store(req.retention),
        }),
        Ok(None) => AuthOutcome::NeedsPrompt {
            user: user.unwrap_or_default(),
        },
        Err(e) => AuthOutcome::Failed(AuthError::Store(e)),
    }
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
