//! The declarative engine catalog (`catalog/engines.toml`): connection types, default options
//! with their rationale, version floors, auth providers and option allow-lists (HV-DRV-01).
//! Fallback edges are checked at load: an edge to an engine the catalog does not describe, or a
//! cycle, is a load error that names it.

use std::collections::HashMap;
use std::sync::OnceLock;

use hacivat_proto::DbmsKind;
use serde::Deserialize;

/// The catalog schema version this build reads.
const SCHEMA: u32 = 1;

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
    /// PostgreSQL settings that a `-c` in the `options` option may set when the profile comes
    /// from an untrusted origin (HV-DRV-05). Empty for other engines.
    pub safe_gucs: Vec<String>,
    /// The default connection budget (CONN-02).
    pub default_budget: u8,
}

/// Why a catalog failed to load (HV-DRV-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CatalogError {
    Parse(String),
    /// A fallback cycle, by engine ID, starting and ending at the same engine.
    Cycle(Vec<String>),
    /// A fallback edge to an engine the catalog does not describe, by ID.
    UnknownTarget(String),
}

/// The engine catalog (HV-DRV-01).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EngineCatalog {
    engines: Vec<EngineSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalog {
    schema: u32,
    #[serde(default)]
    engine: Vec<RawEngine>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEngine {
    id: DbmsKind,
    /// Read as text, so that an edge to a non-engine is an `UnknownTarget`, not a parse error.
    fallback: Option<String>,
    default_port: Option<u16>,
    #[serde(default)]
    conn_type: Vec<RawConnType>,
    #[serde(default)]
    default: Vec<RawDefault>,
    floor: Option<String>,
    #[serde(default)]
    tested: Vec<String>,
    #[serde(default)]
    auth: Vec<String>,
    #[serde(default)]
    option_allow: Vec<String>,
    #[serde(default)]
    always_deny: Vec<String>,
    #[serde(default)]
    safe_gucs: Vec<String>,
    default_budget: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConnType {
    id: String,
    template: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDefault {
    key: String,
    value: String,
    rationale: String,
}

impl EngineCatalog {
    /// Parses and validates a catalog: the schema version, one entry per engine, every fallback
    /// target described, and no fallback cycle.
    pub fn parse(toml: &str) -> Result<Self, CatalogError> {
        let raw: RawCatalog =
            toml::from_str(toml).map_err(|e| CatalogError::Parse(e.message().to_string()))?;
        if raw.schema != SCHEMA {
            return Err(CatalogError::Parse(format!(
                "unsupported schema {}",
                raw.schema
            )));
        }
        let mut kinds = Vec::with_capacity(raw.engine.len());
        for e in &raw.engine {
            if kinds.contains(&e.id) {
                return Err(CatalogError::Parse(format!(
                    "duplicate engine {}",
                    e.id.id()
                )));
            }
            kinds.push(e.id);
        }
        let mut engines = Vec::with_capacity(raw.engine.len());
        for e in raw.engine {
            let fallback = match e.fallback {
                None => None,
                Some(id) => match DbmsKind::from_id(&id) {
                    Some(k) if kinds.contains(&k) => Some(k),
                    _ => return Err(CatalogError::UnknownTarget(id)),
                },
            };
            engines.push(EngineSpec {
                kind: e.id,
                fallback,
                default_port: e.default_port,
                conn_types: e
                    .conn_type
                    .into_iter()
                    .map(|c| ConnTypeSpec {
                        id: c.id,
                        template: c.template,
                    })
                    .collect(),
                defaults: e
                    .default
                    .into_iter()
                    .map(|d| DefaultOption {
                        key: d.key,
                        value: d.value,
                        rationale: d.rationale,
                    })
                    .collect(),
                floor: e.floor,
                tested: e.tested,
                auth: e.auth,
                option_allow: e.option_allow,
                always_deny: e.always_deny,
                safe_gucs: e.safe_gucs,
                default_budget: e.default_budget,
            });
        }
        let catalog = EngineCatalog { engines };
        catalog.check_cycles()?;
        Ok(catalog)
    }

    /// Rejects a fallback cycle by DFS with colour marking: white is unvisited, grey is on the
    /// current path, black is finished. Each engine has at most one fallback edge, so the walk
    /// from an engine is a path.
    fn check_cycles(&self) -> Result<(), CatalogError> {
        #[derive(Clone, Copy)]
        enum Colour {
            White,
            Grey,
            Black,
        }
        let mut colour: HashMap<DbmsKind, Colour> = self
            .engines
            .iter()
            .map(|e| (e.kind, Colour::White))
            .collect();
        for start in &self.engines {
            let mut path: Vec<DbmsKind> = Vec::new();
            let mut cur = Some(start.kind);
            while let Some(k) = cur {
                match colour.get(&k).copied().unwrap_or(Colour::Black) {
                    Colour::Black => break,
                    Colour::Grey => {
                        let from = path.iter().position(|p| *p == k).unwrap_or(0);
                        let mut cycle: Vec<String> =
                            path[from..].iter().map(|p| p.id().to_string()).collect();
                        cycle.push(k.id().to_string());
                        return Err(CatalogError::Cycle(cycle));
                    }
                    Colour::White => {
                        colour.insert(k, Colour::Grey);
                        path.push(k);
                        cur = self.get(k).and_then(|e| e.fallback);
                    }
                }
            }
            for k in path {
                colour.insert(k, Colour::Black);
            }
        }
        Ok(())
    }

    /// The catalog embedded at build time (`catalog/engines.toml`). A broken embedded catalog
    /// is a build defect, which the catalog tests catch.
    pub fn embedded() -> &'static EngineCatalog {
        static EMBEDDED: OnceLock<EngineCatalog> = OnceLock::new();
        EMBEDDED.get_or_init(|| {
            EngineCatalog::parse(include_str!("../catalog/engines.toml"))
                .expect("the embedded engine catalog is valid")
        })
    }

    pub fn get(&self, kind: DbmsKind) -> Option<&EngineSpec> {
        self.engines.iter().find(|e| e.kind == kind)
    }

    /// `kind`, then its fallbacks in order (`MariaDb` gives `[MariaDb, MySql]`). An engine the
    /// catalog does not describe is its own chain. `Unknown` is not part of a chain: the
    /// registry tries it last.
    pub fn chain(&self, kind: DbmsKind) -> Vec<DbmsKind> {
        let mut chain = vec![kind];
        let mut cur = kind;
        while let Some(next) = self.get(cur).and_then(|e| e.fallback) {
            // `parse` rejects cycles; this guard only stops a hand-built `Default` catalog.
            if chain.contains(&next) {
                break;
            }
            chain.push(next);
            cur = next;
        }
        chain
    }

    pub fn engines(&self) -> impl Iterator<Item = &EngineSpec> {
        self.engines.iter()
    }
}
