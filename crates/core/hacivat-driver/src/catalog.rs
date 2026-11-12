//! The declarative engine catalog (`catalog/engines.toml`): connection types, default options
//! with their rationale, version floors, auth providers and option allow-lists (HV-DRV-01).
//! Bodies in Task 2.

use std::sync::OnceLock;

use hacivat_proto::DbmsKind;

/// One connection type of an engine and its URL template (HV-DRV-02).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnTypeSpec {
    pub id: String,
    pub template: String,
}

/// A default driver option and why it is set (HV-DRV-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefaultOption {
    pub key: String,
    pub value: String,
    pub rationale: String,
}

/// One engine's catalog entry (HV-DRV-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EngineSpec {
    pub kind: DbmsKind,
    /// The engine whose capabilities this one falls back to (`MariaDb → MySql`).
    pub fallback: Option<DbmsKind>,
    pub default_port: Option<u16>,
    pub conn_types: Vec<ConnTypeSpec>,
    pub defaults: Vec<DefaultOption>,
    /// The oldest supported server version; `None` (SQLite) is always supported.
    pub floor: Option<String>,
    pub tested: Vec<String>,
    /// Auth provider IDs offered.
    pub auth: Vec<String>,
    pub option_allow: Vec<String>,
    pub always_deny: Vec<String>,
    /// The default connection budget (CONN-02).
    pub default_budget: u8,
}

/// Why a catalog failed to load (HV-DRV-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CatalogError {
    Parse(String),
    /// A fallback cycle, by engine ID.
    Cycle(Vec<String>),
    UnknownTarget(String),
}

/// The engine catalog (HV-DRV-01).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EngineCatalog {
    engines: Vec<EngineSpec>,
}

impl EngineCatalog {
    /// Parses and validates a catalog (Task 2). The stub returns an empty catalog.
    pub fn parse(_toml: &str) -> Result<Self, CatalogError> {
        Ok(EngineCatalog::default())
    }

    /// The catalog embedded at build time (Task 2). The stub is empty.
    pub fn embedded() -> &'static EngineCatalog {
        static EMBEDDED: OnceLock<EngineCatalog> = OnceLock::new();
        EMBEDDED.get_or_init(EngineCatalog::default)
    }

    pub fn get(&self, kind: DbmsKind) -> Option<&EngineSpec> {
        self.engines.iter().find(|e| e.kind == kind)
    }

    /// `kind`, then its fallbacks in order, ending at `Unknown` (Task 2). The stub is empty.
    pub fn chain(&self, _kind: DbmsKind) -> Vec<DbmsKind> {
        Vec::new()
    }

    pub fn engines(&self) -> impl Iterator<Item = &EngineSpec> {
        self.engines.iter()
    }
}
