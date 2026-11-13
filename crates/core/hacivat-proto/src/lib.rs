//! The shared vocabulary of the connect stack (alg/01 §0.4): engine kinds, profile IDs,
//! departments, environment tags, origins, error classes and quick-fix IDs. Every ID here is a
//! stable, locale-free machine string; nothing in this crate is user-visible text except the
//! untranslated product names of [`DbmsKind::display_name`].

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

// ## m2a-finalfix
mod secret_param;
pub use secret_param::is_secret_param;

/// Serializes a type as its `id()` string and reads it back through `from_id`.
macro_rules! id_serde {
    ($ty:ty, $what:literal) => {
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.id())
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let id = String::deserialize(d)?;
                <$ty>::from_id(&id)
                    .ok_or_else(|| serde::de::Error::custom(format!("unknown {}: {id}", $what)))
            }
        }
    };
}

/// A database engine or wire-compatible fork (HV-DRV-01, alg/01 §0.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DbmsKind {
    Postgres,
    Redshift,
    Cockroach,
    Yugabyte,
    Timescale,
    MySql,
    MariaDb,
    TiDb,
    MsSql,
    AzureSql,
    Sqlite,
    ClickHouse,
    Redis,
    Valkey,
    MongoDb,
    DocumentDb,
    Oracle,
    Unknown,
}

impl DbmsKind {
    /// Every kind, in declaration order.
    pub const ALL: [DbmsKind; 18] = [
        DbmsKind::Postgres,
        DbmsKind::Redshift,
        DbmsKind::Cockroach,
        DbmsKind::Yugabyte,
        DbmsKind::Timescale,
        DbmsKind::MySql,
        DbmsKind::MariaDb,
        DbmsKind::TiDb,
        DbmsKind::MsSql,
        DbmsKind::AzureSql,
        DbmsKind::Sqlite,
        DbmsKind::ClickHouse,
        DbmsKind::Redis,
        DbmsKind::Valkey,
        DbmsKind::MongoDb,
        DbmsKind::DocumentDb,
        DbmsKind::Oracle,
        DbmsKind::Unknown,
    ];

    /// The stable catalog ID (`postgres`, `mysql`, `azure_sql`, ...).
    pub fn id(self) -> &'static str {
        match self {
            DbmsKind::Postgres => "postgres",
            DbmsKind::Redshift => "redshift",
            DbmsKind::Cockroach => "cockroach",
            DbmsKind::Yugabyte => "yugabyte",
            DbmsKind::Timescale => "timescale",
            DbmsKind::MySql => "mysql",
            DbmsKind::MariaDb => "mariadb",
            DbmsKind::TiDb => "tidb",
            DbmsKind::MsSql => "mssql",
            DbmsKind::AzureSql => "azure_sql",
            DbmsKind::Sqlite => "sqlite",
            DbmsKind::ClickHouse => "clickhouse",
            DbmsKind::Redis => "redis",
            DbmsKind::Valkey => "valkey",
            DbmsKind::MongoDb => "mongodb",
            DbmsKind::DocumentDb => "documentdb",
            DbmsKind::Oracle => "oracle",
            DbmsKind::Unknown => "unknown",
        }
    }

    /// The kind with catalog ID `id` (exact, case-sensitive).
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }

    /// The product name, never translated (`PostgreSQL`, `MySQL`, `MariaDB`, `SQLite`, ...).
    /// `Unknown` has no product; the UI names an unknown engine through Fluent instead.
    pub fn display_name(self) -> &'static str {
        match self {
            DbmsKind::Postgres => "PostgreSQL",
            DbmsKind::Redshift => "Amazon Redshift",
            DbmsKind::Cockroach => "CockroachDB",
            DbmsKind::Yugabyte => "YugabyteDB",
            DbmsKind::Timescale => "TimescaleDB",
            DbmsKind::MySql => "MySQL",
            DbmsKind::MariaDb => "MariaDB",
            DbmsKind::TiDb => "TiDB",
            DbmsKind::MsSql => "SQL Server",
            DbmsKind::AzureSql => "Azure SQL",
            DbmsKind::Sqlite => "SQLite",
            DbmsKind::ClickHouse => "ClickHouse",
            DbmsKind::Redis => "Redis",
            DbmsKind::Valkey => "Valkey",
            DbmsKind::MongoDb => "MongoDB",
            DbmsKind::DocumentDb => "Amazon DocumentDB",
            DbmsKind::Oracle => "Oracle",
            DbmsKind::Unknown => "Unknown",
        }
    }
}

id_serde!(DbmsKind, "engine");

/// A connection profile's stable ID, never reused (alg/01 §0.4). Displays lowercase and
/// hyphenated; serializes as that string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProfileId(pub Uuid);

impl ProfileId {
    /// A fresh random (v4) ID.
    pub fn new() -> Self {
        ProfileId(Uuid::new_v4())
    }
}

impl Default for ProfileId {
    /// A fresh random ID, like [`ProfileId::new`].
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.hyphenated(), f)
    }
}

impl FromStr for ProfileId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(ProfileId)
    }
}

impl Serialize for ProfileId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ProfileId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Who asks for a session: routing, priority and the Karagöz audit `source` (alg/01 §0.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Department {
    Console,
    DataEditor,
    Procedure,
    Function,
    Executor,
    RunConfiguration,
    DataImport,
    Introspection,
    QuickDoc,
    CodeGeneration,
    Refactoring,
    DataExport,
    TextSearch,
    Debugger,
    TestRunner,
    Mcp,
    Cancel,
    Unknown,
}

/// A profile's environment tag, most severe first (D-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EnvTag {
    Local,
    Dev,
    Test,
    Staging,
    Production,
}

impl EnvTag {
    pub const ALL: [EnvTag; 5] = [
        EnvTag::Local,
        EnvTag::Dev,
        EnvTag::Test,
        EnvTag::Staging,
        EnvTag::Production,
    ];

    /// The stable ID: `local`, `dev`, `test`, `staging`, `production`.
    pub fn id(self) -> &'static str {
        match self {
            EnvTag::Local => "local",
            EnvTag::Dev => "dev",
            EnvTag::Test => "test",
            EnvTag::Staging => "staging",
            EnvTag::Production => "production",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// Sort key, most severe first: Production 0, Staging 1, Test 2, Dev 3, Local 4.
    pub fn severity_rank(self) -> u8 {
        match self {
            EnvTag::Production => 0,
            EnvTag::Staging => 1,
            EnvTag::Test => 2,
            EnvTag::Dev => 3,
            EnvTag::Local => 4,
        }
    }
}

id_serde!(EnvTag, "environment");

/// Who set an environment tag: the user, or the organization's policy (D-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TagSource {
    #[default]
    User,
    Org,
}

/// What a profile was imported from (HV-DRV-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportSource {
    PgService,
    Url,
}

/// Where a profile came from; it decides which options are trusted (HV-DRV-05).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Origin {
    #[default]
    UserTyped,
    Imported {
        source: ImportSource,
    },
    PerdeManaged {
        org: String,
    },
    DeepLink,
    Mcp,
}

/// The family of an [`ErrClass`] (HV-CONN-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrFamily {
    Network,
    Tls,
    Ssh,
    Auth,
    Authz,
    Config,
    Server,
    Karagoz,
    Cancelled,
    Internal,
}

/// Declares [`ErrClass`] with its IDs and families in one table, so they cannot drift.
macro_rules! err_classes {
    ($($variant:ident = $id:literal in $family:ident),* $(,)?) => {
        /// A connect-error class: what went wrong, independent of the engine and the locale
        /// (HV-CONN-03). The ID is the Fluent stem's source and the `errors.toml` key.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum ErrClass {
            $($variant),*
        }

        impl ErrClass {
            /// Every class, in taxonomy order.
            pub const ALL: &'static [ErrClass] = &[$(ErrClass::$variant),*];

            /// The stable ID, e.g. `auth.invalid`.
            pub fn id(self) -> &'static str {
                match self {
                    $(ErrClass::$variant => $id),*
                }
            }

            pub fn family(self) -> ErrFamily {
                match self {
                    $(ErrClass::$variant => ErrFamily::$family),*
                }
            }
        }
    };
}

err_classes! {
    NetworkDns = "network.dns" in Network,
    NetworkRefused = "network.refused" in Network,
    NetworkTimeout = "network.timeout" in Network,
    NetworkReset = "network.reset" in Network,
    NetworkUnreachable = "network.unreachable" in Network,
    NetworkIpv6Unavailable = "network.ipv6_unavailable" in Network,
    TlsHandshake = "tls.handshake" in Tls,
    TlsUntrusted = "tls.untrusted" in Tls,
    TlsHostname = "tls.hostname" in Tls,
    TlsProtocol = "tls.protocol" in Tls,
    TlsClientCertRequired = "tls.client_cert_required" in Tls,
    TlsNotSupportedByServer = "tls.not_supported_by_server" in Tls,
    // ## m2b-t01-interfaces
    TlsPinChanged = "tls.pin_changed" in Tls,
    SshHostKeyUnknown = "ssh.host_key_unknown" in Ssh,
    SshHostKeyChanged = "ssh.host_key_changed" in Ssh,
    SshAuth = "ssh.auth" in Ssh,
    SshChannelOpen = "ssh.channel_open" in Ssh,
    SshAgentUnavailable = "ssh.agent_unavailable" in Ssh,
    // ## m2b-t01-interfaces
    SshHostKeyRevoked = "ssh.host_key_revoked" in Ssh,
    AuthInvalid = "auth.invalid" in Auth,
    AuthExpired = "auth.expired" in Auth,
    AuthExpiring = "auth.expiring" in Auth,
    AuthLocked = "auth.locked" in Auth,
    AuthMethodUnsupported = "auth.method_unsupported" in Auth,
    AuthMfaRequired = "auth.mfa_required" in Auth,
    // ## m2b-t01-interfaces
    AuthInsecureMethod = "auth.insecure_method" in Auth,
    AuthzDatabase = "authz.database" in Authz,
    AuthzObject = "authz.object" in Authz,
    ConfigInvalid = "config.invalid" in Config,
    ConfigUnknownDatabase = "config.unknown_database" in Config,
    ConfigModeMismatch = "config.mode_mismatch" in Config,
    ConfigWrongPortProtocol = "config.wrong_port_protocol" in Config,
    ConfigPoolerPreparedStatements = "config.pooler_prepared_statements" in Config,
    ServerTooManyConnections = "server.too_many_connections" in Server,
    ServerStarting = "server.starting" in Server,
    ServerShutdown = "server.shutdown" in Server,
    ServerReadOnly = "server.read_only" in Server,
    ServerUnsupportedVersion = "server.unsupported_version" in Server,
    KaragozForbidden = "karagoz.forbidden" in Karagoz,
    KaragozGrantExpired = "karagoz.grant_expired" in Karagoz,
    KaragozRevoked = "karagoz.revoked" in Karagoz,
    KaragozUpstreamUnreachable = "karagoz.upstream_unreachable" in Karagoz,
    KaragozUpstreamAuth = "karagoz.upstream_auth" in Karagoz,
    KaragozOverloaded = "karagoz.overloaded" in Karagoz,
    KaragozProtocol = "karagoz.protocol" in Karagoz,
    KaragozSessionTimeout = "karagoz.session_timeout" in Karagoz,
    KaragozResumeExpired = "karagoz.resume_expired" in Karagoz,
    KaragozDirectPathUnavailable = "karagoz.direct_path_unavailable" in Karagoz,
    KaragozPeerIdentityMismatch = "karagoz.peer_identity_mismatch" in Karagoz,
    Cancelled = "cancelled" in Cancelled,
    Internal = "internal" in Internal,
}

impl ErrClass {
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.id() == id)
    }

    /// The Fluent message stem: `error-conn-` plus the ID with `.` and `_` turned into `-`
    /// (`auth.invalid` → `error-conn-auth-invalid`).
    pub fn fluent_stem(self) -> String {
        let tail: String = self
            .id()
            .chars()
            .map(|c| if c == '.' || c == '_' { '-' } else { c })
            .collect();
        format!("error-conn-{tail}")
    }

    /// Whether a reconnect may simply try again: only `network.{timeout,reset,refused}`,
    /// `server.starting`, `karagoz.overloaded` and `karagoz.upstream_unreachable`.
    pub fn retryable_on_reconnect(self) -> bool {
        matches!(
            self,
            ErrClass::NetworkTimeout
                | ErrClass::NetworkReset
                | ErrClass::NetworkRefused
                | ErrClass::ServerStarting
                | ErrClass::KaragozOverloaded
                | ErrClass::KaragozUpstreamUnreachable
        )
    }
}

id_serde!(ErrClass, "error class");

/// A quick fix offered next to a connect error (HV-CONN-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FixId {
    EditConnection,
    TryAgain,
    ReenterPassword,
    UseDatabase,
    SupportedVersions,
    ConnectAnyway,
    EncryptWithoutVerifying,
    ReviewCertificate,
    ImportCa,
    ConnectThroughSsh,
    ChangePassword,
    UseSingleSessionMode,
    LowerConnectionBudget,
    SwitchToCluster,
    SwitchToStandalone,
    DisableServerPreparedStatements,
    ReviewHostKey,
}

impl FixId {
    pub const ALL: [FixId; 17] = [
        FixId::EditConnection,
        FixId::TryAgain,
        FixId::ReenterPassword,
        FixId::UseDatabase,
        FixId::SupportedVersions,
        FixId::ConnectAnyway,
        FixId::EncryptWithoutVerifying,
        FixId::ReviewCertificate,
        FixId::ImportCa,
        FixId::ConnectThroughSsh,
        FixId::ChangePassword,
        FixId::UseSingleSessionMode,
        FixId::LowerConnectionBudget,
        FixId::SwitchToCluster,
        FixId::SwitchToStandalone,
        FixId::DisableServerPreparedStatements,
        FixId::ReviewHostKey,
    ];

    /// The snake_case ID (`edit_connection`, `try_again`, ...).
    pub fn id(self) -> &'static str {
        match self {
            FixId::EditConnection => "edit_connection",
            FixId::TryAgain => "try_again",
            FixId::ReenterPassword => "reenter_password",
            FixId::UseDatabase => "use_database",
            FixId::SupportedVersions => "supported_versions",
            FixId::ConnectAnyway => "connect_anyway",
            FixId::EncryptWithoutVerifying => "encrypt_without_verifying",
            FixId::ReviewCertificate => "review_certificate",
            FixId::ImportCa => "import_ca",
            FixId::ConnectThroughSsh => "connect_through_ssh",
            FixId::ChangePassword => "change_password",
            FixId::UseSingleSessionMode => "use_single_session_mode",
            FixId::LowerConnectionBudget => "lower_connection_budget",
            FixId::SwitchToCluster => "switch_to_cluster",
            FixId::SwitchToStandalone => "switch_to_standalone",
            FixId::DisableServerPreparedStatements => "disable_server_prepared_statements",
            FixId::ReviewHostKey => "review_host_key",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
    }
}

id_serde!(FixId, "quick fix");

/// How a live connection guards writes, as the capsule labels it (D-035).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessLabel {
    ReadWrite,
    ReadOnlyGuarded,
    ReadOnlyEnforced,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for k in [
            DbmsKind::Postgres,
            DbmsKind::MySql,
            DbmsKind::MariaDb,
            DbmsKind::Sqlite,
        ] {
            assert_eq!(DbmsKind::from_id(k.id()), Some(k));
        }
        for c in ErrClass::ALL {
            assert_eq!(ErrClass::from_id(c.id()), Some(*c));
        }
        assert_eq!(
            ErrClass::AuthInvalid.fluent_stem(),
            "error-conn-auth-invalid"
        );
        assert_eq!(
            ErrClass::ServerUnsupportedVersion.fluent_stem(),
            "error-conn-server-unsupported-version"
        );
        assert_eq!(ErrClass::ALL.len(), 51);
        assert_eq!(EnvTag::Production.severity_rank(), 0);
        assert_eq!(EnvTag::Local.severity_rank(), 4);
    }

    #[test]
    fn profile_id_display_is_lowercase_hyphenated() {
        let id: ProfileId = "6F9619FF-8B86-D011-B42D-00C04FC964FF".parse().unwrap();
        assert_eq!(id.to_string(), "6f9619ff-8b86-d011-b42d-00c04fc964ff");
    }

    #[test]
    fn every_id_is_unique_and_round_trips() {
        for k in DbmsKind::ALL {
            assert_eq!(DbmsKind::from_id(k.id()), Some(k));
        }
        for t in EnvTag::ALL {
            assert_eq!(EnvTag::from_id(t.id()), Some(t));
        }
        for f in FixId::ALL {
            assert_eq!(FixId::from_id(f.id()), Some(f));
        }
        let mut ids: Vec<&str> = ErrClass::ALL.iter().map(|c| c.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 51);
        assert_eq!(DbmsKind::from_id("MySQL"), None); // IDs are exact, never case-folded
    }

    #[test]
    fn families_and_reconnect_retry() {
        assert_eq!(
            ErrClass::NetworkIpv6Unavailable.family(),
            ErrFamily::Network
        );
        assert_eq!(ErrClass::ConfigInvalid.family(), ErrFamily::Config);
        assert_eq!(ErrClass::KaragozProtocol.family(), ErrFamily::Karagoz);
        let retryable: Vec<&str> = ErrClass::ALL
            .iter()
            .filter(|c| c.retryable_on_reconnect())
            .map(|c| c.id())
            .collect();
        assert_eq!(
            retryable,
            [
                "network.refused",
                "network.timeout",
                "network.reset",
                "server.starting",
                "karagoz.upstream_unreachable",
                "karagoz.overloaded"
            ]
        );
    }

    #[test]
    fn profile_ids_are_fresh_v4() {
        let (a, b) = (ProfileId::new(), ProfileId::new());
        assert_ne!(a, b);
        assert_eq!(a.0.get_version_num(), 4);
        assert_eq!(a.to_string().parse::<ProfileId>().unwrap(), a);
    }
}
