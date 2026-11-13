//! Connection profiles (HV-SEC-05 profile split, D-004): the profile and its draft, the
//! `connections.toml` store, URL field mapping and naming rules. Types are complete since
//! Task 1; logic in Task 12.

mod fields;
// ## m2a-finalfix
mod options;
mod rules;
mod store;

use std::collections::BTreeSet;
use std::path::PathBuf;

use hacivat_driver::url::FieldId;
use hacivat_net::TlsSettings;
use hacivat_net::ssh::{HopSecrets, SshSpec};
use hacivat_proto::{DbmsKind, EnvTag, Origin, ProfileId, TagSource};
use hacivat_secrets::{AuthProviderId, Retention};
use secrecy::SecretString;

use crate::session::{IsolationLevel, TxMode};

pub use fields::{
    ApplyReport, CopiedUrl, EngineChange, apply_parse, copy_url, display_url, to_fields,
};
pub use options::{FoldedOption, FoldedTo, fold_typed_options, strip_secret_options};
pub use rules::{
    HintSource, duplicate_name, is_local_endpoint, needs_environment_prompt, production_hint,
    suggest_name,
};
pub use store::{LoadedProfiles, ProfileDiag, ProfileStore, SaveReport, StoreError};

/// Where a profile connects (HV-CONN-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProfileEndpoint {
    Server {
        host: String,
        port: Option<u16>,
    },
    Socket {
        path: PathBuf,
        port: Option<u16>,
    },
    File {
        path: PathBuf,
        create_if_missing: bool,
    },
}

/// How the bytes travel (HV-CONN-01). Only `Direct` connects in M2a; M2b adds SSH tunnels
/// (the transport step, Task 14).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TransportSpec {
    Direct,
    // ## m2b-t01-interfaces
    SshTunnel(SshSpec),
    Karagoz {
        org: String,
        agent_id: String,
        resource_id: String,
    },
}

/// A saved connection, without secrets (HV-SEC-05, D-004).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub engine: DbmsKind,
    pub environment: Option<EnvTag>,
    pub env_source: TagSource,
    pub read_only: bool,
    pub endpoint: ProfileEndpoint,
    pub database: Option<String>,
    pub user: Option<String>,
    pub auth: AuthProviderId,
    pub retention: Retention,
    pub tls: TlsSettings,
    pub transport: TransportSpec,
    /// Seconds.
    pub connect_timeout_s: u32,
    /// Seconds; `None` keeps the driver default.
    pub keepalive_s: Option<u32>,
    pub application_name: String,
    /// The connection budget (SQLite: 1).
    pub max_connections: u8,
    pub extra: Vec<(String, String)>,
    pub origin: Origin,
    /// The user chose to connect to a server below the version floor.
    pub allow_unsupported_server: bool,
    // ## m2b-t01-interfaces
    /// Every request goes through one session (HV-SESS-02, D-035).
    pub single_session: bool,
    /// `None` uses the environment's default (`session::default_mode`).
    pub tx_mode: Option<TxMode>,
    pub isolation: Option<IsolationLevel>,
    /// Seconds idle before the connection closes; `None` keeps it open.
    pub idle_disconnect_s: Option<u32>,
    /// How long the SSH passwords and passphrases are kept.
    pub ssh_retention: Retention,
}

impl Default for Profile {
    /// A new PostgreSQL profile with a fresh ID.
    fn default() -> Self {
        Profile {
            id: ProfileId::new(),
            name: String::new(),
            engine: DbmsKind::Postgres,
            environment: None,
            env_source: TagSource::User,
            read_only: false,
            endpoint: ProfileEndpoint::Server {
                host: String::new(),
                port: None,
            },
            database: None,
            user: None,
            auth: AuthProviderId::Password,
            retention: Retention::default(),
            tls: TlsSettings::default(),
            transport: TransportSpec::Direct,
            connect_timeout_s: 15,
            keepalive_s: None,
            application_name: "Hacivat".to_string(),
            max_connections: 4,
            extra: Vec::new(),
            origin: Origin::UserTyped,
            allow_unsupported_server: false,
            single_session: false,
            tx_mode: None,
            isolation: None,
            idle_disconnect_s: None,
            ssh_retention: Retention::default(),
        }
    }
}

/// A profile being edited in the dialog, with its unsaved password (HV-SEC-05). Holds a
/// secret, so `Debug` only.
#[derive(Debug)]
pub struct ProfileDraft {
    pub profile: Profile,
    pub password: Option<SecretString>,
    /// The TLS client key passphrase (a pasted `sslpassword`), stored in
    /// `SecretSlot::TlsKeyPassphrase` on save (Task 12).
    pub tls_key_passphrase: Option<SecretString>,
    pub is_new: bool,
    /// The fields the user edited by hand (a pasted URL never overwrites them).
    pub touched: BTreeSet<FieldId>,
    // ## m2b-t01-interfaces
    /// The SSH secrets typed in the dialog, per hop; `ConnectionsModel::save` stores them under
    /// `SecretSlot::{SshPassword, SshPassphrase} { hop }` with `ssh_retention` (Q23).
    pub ssh_secrets: Vec<HopSecrets>,
}

impl ProfileDraft {
    /// A draft of a new default profile: no password, nothing touched.
    pub fn new() -> Self {
        ProfileDraft {
            profile: Profile::default(),
            password: None,
            tls_key_passphrase: None,
            is_new: true,
            touched: BTreeSet::new(),
            ssh_secrets: Vec::new(),
        }
    }

    /// A draft editing the saved profile `p`.
    pub fn edit(p: Profile) -> Self {
        ProfileDraft {
            profile: p,
            password: None,
            tls_key_passphrase: None,
            is_new: false,
            touched: BTreeSet::new(),
            ssh_secrets: Vec::new(),
        }
    }
}

impl Default for ProfileDraft {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let p = Profile::default();
        assert_eq!(p.engine, DbmsKind::Postgres);
        assert_eq!(
            (
                p.connect_timeout_s,
                p.max_connections,
                p.application_name.as_str()
            ),
            (15, 4, "Hacivat")
        );
        assert_ne!(p.id, Profile::default().id);
        let d = ProfileDraft::new();
        assert!(d.is_new && d.password.is_none() && d.touched.is_empty());
        let e = ProfileDraft::edit(p.clone());
        assert!(!e.is_new);
        assert_eq!(e.profile, p);
    }
}
