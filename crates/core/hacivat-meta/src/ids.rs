//! Identity (HV-MODEL-01 "Two identities"): [`ObjectKey`] is stable across snapshot
//! generations and refreshes; [`ObjId`] is a handle valid only inside the generation that
//! produced it. M4 Task 1a seam (spike origin `d86313b`, declarations only).

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::kind::ObjectKind;
use crate::name::Name;

/// The server's own id for an object, when the DBMS has a stable one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ServerId {
    /// PostgreSQL OID (reusable after wraparound; HV-INTRO-11 checks the creation marker).
    Oid(u32),
    /// SQL Server `object_id` (per database).
    ObjectId(i32),
    /// Oracle `obj#`, or a packed composite id.
    ObjNum(u64),
    /// ClickHouse table uuid, MongoDB collection uuid.
    Uuid([u8; 16]),
    /// An integer row id that is stable for the object's lifetime.
    RowId(i64),
}

/// The per-object change marker incremental introspection compares (HV-INTRO-05..08).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StateMark {
    #[default]
    None,
    /// PostgreSQL row `xmin` (32-bit; age-compared, never ordered).
    Xmin(u32),
    /// Server modification time, microseconds since the Unix epoch.
    Modified(i64),
    /// Server creation time, microseconds since the Unix epoch (MySQL `CREATE_TIME`).
    Created(i64),
    /// R-M4-39's MySQL table stamp: `CREATE_TIME`, `max(TRIGGERS.CREATED)` and the trigger
    /// count (which catches a dropped trigger), microseconds since the Unix epoch.
    MyTable {
        created: i64,
        trigger_max: i64,
        trigger_count: u32,
    },
    /// SQLite `schema_version`.
    SchemaVersion(u32),
    /// A digest of the object's definition rows.
    Hash(u64),
}

/// Where a namespace sits in the server hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NsKind {
    /// Server-level objects (roles, tablespaces). Path `[]`.
    Server,
    /// Database-level objects (PostgreSQL extensions, casts, languages). Path `[db]`.
    Database,
    /// A schema: PostgreSQL `[db, schema]`; MySQL/MariaDB `[schema]`; SQLite `[main]`,
    /// `[temp]`, `[<attached>]`.
    Schema,
}

/// The identity of a namespace (one arena, one cache row): its kind and exact server path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(from = "NsKeyRepr", into = "NsKeyRepr")]
pub struct NsKey {
    pub kind: NsKind,
    pub path: SmallVec<[Name; 2]>,
}

impl NsKey {
    /// The server namespace.
    pub fn server() -> Self {
        NsKey {
            kind: NsKind::Server,
            path: SmallVec::new(),
        }
    }

    /// The database namespace of `db`.
    pub fn database(db: impl Into<Name>) -> Self {
        NsKey {
            kind: NsKind::Database,
            path: smallvec::smallvec![db.into()],
        }
    }

    /// A schema namespace; `path` is `[db, schema]` on PostgreSQL, `[schema]` elsewhere.
    pub fn schema<I, N>(path: I) -> Self
    where
        I: IntoIterator<Item = N>,
        N: Into<Name>,
    {
        NsKey {
            kind: NsKind::Schema,
            path: path.into_iter().map(Into::into).collect(),
        }
    }
}

/// The stable identity of an object (alg/02 HV-MODEL-01): used by the explorer tree, user data
/// (virtual FKs, notes), filters, history, MCP and diff matching.
///
/// `Eq`/`Hash` are **structural** (every field). Identity across a refresh is
/// [`ObjectKey::same_object`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(from = "ObjectKeyRepr", into = "ObjectKeyRepr")]
pub struct ObjectKey {
    pub kind: ObjectKind,
    /// Exact server names, outermost first, ending with the object's own name (PostgreSQL
    /// `[db, schema, table, column]`, MySQL `[schema, table, column]`).
    pub path: SmallVec<[Name; 4]>,
    pub server_id: Option<ServerId>,
    /// The normalized argument type list of an overloaded routine.
    pub overload_sig: Option<Name>,
}

impl ObjectKey {
    /// Whether `self` and `other` denote the same server object: by server id when both have
    /// one (a rename keeps identity), else by kind, path and overload signature.
    ///
    /// M4 Task 1a stub, fail-closed until Task 3: structural equality only (two equal keys are
    /// always the same object; a renamed object is not recognised yet, so a refresh treats it
    /// as dropped and created — never two objects as one).
    pub fn same_object(&self, other: &ObjectKey) -> bool {
        self == other
    }
}

/// A reference to another object (FK target, partition parent, sequence owner, trigger
/// function): the target's kind, exact path and server id, never an arena index, so it stays
/// valid when either side's namespace is reloaded; resolved on demand (R-M4-26).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ref {
    pub kind: ObjectKind,
    pub path: SmallVec<[Name; 4]>,
    pub server_id: Option<ServerId>,
}

/// A generation-local handle: the namespace's position in its generation and the arena index.
/// Valid only for the [`crate::snapshot::CatalogSnapshot`] that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjId {
    pub ns: u32,
    pub idx: u32,
}

/// Source text (view, routine, trigger bodies) lives in the content-addressed source store
/// (HV-MODEL-03 step 6); the model keeps only its hash and length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceRef {
    /// XXH3-128 of the exact UTF-8 text.
    pub hash: u128,
    pub len: u32,
}

// Serde forms. `Name` is `Arc<str>` and serde's `rc` feature is off in the workspace, so the
// keys travel through plain-`String` mirrors; a kind id this build does not know reads back as
// `ObjectKind::Unknown` (the cache-code rule).

#[derive(Serialize, Deserialize)]
struct NsKeyRepr {
    kind: NsKind,
    path: Vec<String>,
}

impl From<NsKeyRepr> for NsKey {
    fn from(r: NsKeyRepr) -> Self {
        NsKey {
            kind: r.kind,
            path: r.path.into_iter().map(Name::from).collect(),
        }
    }
}

impl From<NsKey> for NsKeyRepr {
    fn from(k: NsKey) -> Self {
        NsKeyRepr {
            kind: k.kind,
            path: k.path.iter().map(|n| n.to_string()).collect(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ObjectKeyRepr {
    kind: String,
    path: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    server_id: Option<ServerId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    overload_sig: Option<String>,
}

impl From<ObjectKeyRepr> for ObjectKey {
    fn from(r: ObjectKeyRepr) -> Self {
        ObjectKey {
            kind: ObjectKind::from_id(&r.kind).unwrap_or(ObjectKind::Unknown),
            path: r.path.into_iter().map(Name::from).collect(),
            server_id: r.server_id,
            overload_sig: r.overload_sig.map(Name::from),
        }
    }
}

impl From<ObjectKey> for ObjectKeyRepr {
    fn from(k: ObjectKey) -> Self {
        ObjectKeyRepr {
            kind: k.kind.id().to_string(),
            path: k.path.iter().map(|n| n.to_string()).collect(),
            server_id: k.server_id,
            overload_sig: k.overload_sig.as_deref().map(str::to_string),
        }
    }
}
