//! Per-profile user data about the catalog (A-19): the introspection scope, view and scope
//! filters, level overrides, virtual foreign keys, annotations and the DDL view's options.
//!
//! Types and (Task 26) merge logic only: the file I/O is `hacivat_settings::profile_data`.
//! Serde, versioned ([`PROFILE_USER_DATA_VERSION`]); maps keyed by [`ObjectKey`] travel as
//! lists of pairs, so TOML and JSON (string-keyed maps only) can hold them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::ObjectKey;
use crate::namespace::Level;
use crate::overlay::VirtualFk;

/// The current version of [`ProfileUserData`]'s serialized form.
pub const PROFILE_USER_DATA_VERSION: u32 = 1;

/// Schema qualification in generated DDL (HV-DDL-10; R-M4-23 default `Auto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Qualify {
    #[default]
    Auto,
    Never,
    Always,
}

/// Constraint placement in generated DDL (HV-DDL-09; R-M4-23 default `InsideTable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Placement {
    InsideColumn,
    #[default]
    InsideTable,
    AfterTable,
}

/// The DDL view's options strip, remembered per profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DdlViewOptions {
    pub qualify: Qualify,
    pub placement: Placement,
    /// "Definitions provided by server".
    pub server_definitions: bool,
    pub grants: bool,
    pub owner: bool,
}

impl Default for DdlViewOptions {
    fn default() -> Self {
        DdlViewOptions {
            qualify: Qualify::Auto,
            placement: Placement::InsideTable,
            server_definitions: false,
            grants: true,
            owner: true,
        }
    }
}

/// One profile's catalog user data.
// ## m4-t27-refresh-ui (named exception, ruling R-W5-13): (de)serialized through [`Repr`], so
// the keys of the nested `ddl_view` table this build does not know survive a write-back too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Repr", into = "Repr")]
pub struct ProfileUserData {
    #[serde(default = "current_version")]
    pub version: u32,
    /// The scope pattern (HV-INTRO-02); `None` = the engine's default (R-M4-14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, with = "pairs")]
    pub view_filters: BTreeMap<ObjectKey, String>,
    #[serde(default, with = "pairs")]
    pub scope_filters: BTreeMap<ObjectKey, String>,
    #[serde(default, with = "pairs")]
    pub level_overrides: BTreeMap<ObjectKey, Level>,
    #[serde(default)]
    pub virtual_fks: Vec<VirtualFk>,
    #[serde(default, with = "pairs")]
    pub annotations: BTreeMap<ObjectKey, String>,
    #[serde(default)]
    pub ddl_view: DdlViewOptions,
    // ## m4-t27-refresh-ui (named exception, T26 carry: persist "Introspect live")
    /// The system namespaces the user chose to introspect live rather than read from the
    /// bundled snapshot (`overlay::Overlays::live_system`, Task 26), sorted, no duplicates.
    /// Absent from older files (empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub live_system: Vec<crate::ids::NsKey>,
    // ## m4-t27-refresh-ui (named exception, ruling R-W5-12; the carries-w1 m10 write-back rule)
    /// Every key this build does not know (a newer build's), kept as read and written back
    /// unchanged, so no build drops another's data.
    #[serde(flatten)]
    pub extra: BTreeMap<String, ExtraValue>,
    // ## m4-t27-refresh-ui (named exception, ruling R-W5-13)
    /// The keys of the `ddl_view` table this build does not know, kept as read.
    #[serde(skip)]
    pub ddl_view_extra: BTreeMap<String, ExtraValue>,
}

// ## m4-t27-refresh-ui (named exception, ruling R-W5-13)
/// The serialized form of [`ProfileUserData`] (the same keys; `ddl_view` keeps its unknown keys).
#[derive(Serialize, Deserialize)]
struct Repr {
    #[serde(default = "current_version")]
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(default, with = "pairs")]
    view_filters: BTreeMap<ObjectKey, String>,
    #[serde(default, with = "pairs")]
    scope_filters: BTreeMap<ObjectKey, String>,
    #[serde(default, with = "pairs")]
    level_overrides: BTreeMap<ObjectKey, Level>,
    #[serde(default)]
    virtual_fks: Vec<VirtualFk>,
    #[serde(default, with = "pairs")]
    annotations: BTreeMap<ObjectKey, String>,
    #[serde(default)]
    ddl_view: DdlViewTable,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    live_system: Vec<crate::ids::NsKey>,
    #[serde(flatten)]
    extra: BTreeMap<String, ExtraValue>,
}

/// The `ddl_view` table: the options this build knows, then every other key as read (a
/// flattened struct takes its own keys first, so `extra` holds only the unknown ones).
#[derive(Default, Serialize, Deserialize)]
struct DdlViewTable {
    #[serde(flatten)]
    opts: DdlViewOptions,
    #[serde(flatten)]
    extra: BTreeMap<String, ExtraValue>,
}

impl From<Repr> for ProfileUserData {
    fn from(r: Repr) -> Self {
        ProfileUserData {
            version: r.version,
            scope: r.scope,
            view_filters: r.view_filters,
            scope_filters: r.scope_filters,
            level_overrides: r.level_overrides,
            virtual_fks: r.virtual_fks,
            annotations: r.annotations,
            ddl_view: r.ddl_view.opts,
            live_system: r.live_system,
            extra: r.extra,
            ddl_view_extra: r.ddl_view.extra,
        }
    }
}

impl From<ProfileUserData> for Repr {
    fn from(d: ProfileUserData) -> Self {
        Repr {
            version: d.version,
            scope: d.scope,
            view_filters: d.view_filters,
            scope_filters: d.scope_filters,
            level_overrides: d.level_overrides,
            virtual_fks: d.virtual_fks,
            annotations: d.annotations,
            ddl_view: DdlViewTable {
                opts: d.ddl_view,
                extra: d.ddl_view_extra,
            },
            live_system: d.live_system,
            extra: d.extra,
        }
    }
}
// ## m4-t27-refresh-ui

// ## m4-t27-refresh-ui (named exception, ruling R-W5-12)
/// A value this build does not know, kept as read (any TOML or JSON value; a TOML date or time
/// comes back as its table form).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExtraValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<ExtraValue>),
    Table(BTreeMap<String, ExtraValue>),
}

impl PartialEq for ExtraValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ExtraValue::Bool(a), ExtraValue::Bool(b)) => a == b,
            (ExtraValue::Int(a), ExtraValue::Int(b)) => a == b,
            // Bit equality: a kept value equals itself, NaN included.
            (ExtraValue::Float(a), ExtraValue::Float(b)) => a.to_bits() == b.to_bits(),
            (ExtraValue::Text(a), ExtraValue::Text(b)) => a == b,
            (ExtraValue::List(a), ExtraValue::List(b)) => a == b,
            (ExtraValue::Table(a), ExtraValue::Table(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for ExtraValue {}
// ## m4-t27-refresh-ui

fn current_version() -> u32 {
    PROFILE_USER_DATA_VERSION
}

impl Default for ProfileUserData {
    fn default() -> Self {
        ProfileUserData {
            version: PROFILE_USER_DATA_VERSION,
            scope: None,
            view_filters: BTreeMap::new(),
            scope_filters: BTreeMap::new(),
            level_overrides: BTreeMap::new(),
            virtual_fks: Vec::new(),
            annotations: BTreeMap::new(),
            ddl_view: DdlViewOptions::default(),
            // ## m4-t27-refresh-ui (named exception)
            live_system: Vec::new(),
            extra: BTreeMap::new(),
            ddl_view_extra: BTreeMap::new(),
        }
    }
}

/// A `BTreeMap<ObjectKey, V>` as a list of `(key, value)` pairs.
mod pairs {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::ids::ObjectKey;

    pub fn serialize<V: Serialize, S: Serializer>(
        map: &BTreeMap<ObjectKey, V>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.collect_seq(map.iter())
    }

    pub fn deserialize<'de, V: Deserialize<'de>, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<ObjectKey, V>, D::Error> {
        let list: Vec<(ObjectKey, V)> = Vec::deserialize(d)?;
        Ok(list.into_iter().collect())
    }
}
