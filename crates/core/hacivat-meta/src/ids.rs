//! Identity (HV-MODEL-01 "Two identities"): [`ObjectKey`] is stable across snapshot
//! generations and refreshes; [`ObjId`] is a handle valid only inside the generation that
//! produced it. Declarations from M4 Task 1a; `same_object`, `raw_kind` (T1a review m10) and
//! the helpers are Task 3's (spike origin `d86313b`).

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

impl NsKind {
    /// The kind of the namespace's root object.
    pub fn root_kind(self) -> ObjectKind {
        match self {
            NsKind::Server => ObjectKind::Root,
            NsKind::Database => ObjectKind::Database,
            NsKind::Schema => ObjectKind::Schema,
        }
    }
}

impl NsKey {
    /// The root object's name: the last path element (empty for the server namespace).
    pub fn root_name(&self) -> &str {
        self.path.last().map_or("", |n| n)
    }
}

/// The stable identity of an object (alg/02 HV-MODEL-01): used by the explorer tree, user data
/// (virtual FKs, notes), filters, history, MCP and diff matching.
///
/// `Eq`/`Hash`/`Ord` are **structural** (every field), so they stay transitive and keep map
/// invariants. Identity across a refresh is [`ObjectKey::same_object`], which prefers the
/// server id so a rename keeps identity.
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
    /// The raw kind id of an [`ObjectKind::Unknown`] object (the server's kind text, or a kind
    /// id a newer build wrote), kept so that two kinds this build does not know stay two keys
    /// and a write-back keeps the id (T1a review m10). `None` for every known kind.
    pub raw_kind: Option<Name>,
}

impl ObjectKey {
    /// Whether `self` and `other` denote the same server object (HV-MODEL-01 "Two
    /// identities"): the kinds (and an unknown kind's raw id) must agree; then by server id when
    /// both have one (a rename keeps identity; a reused id on another path is still that id),
    /// else by path and overload signature.
    pub fn same_object(&self, other: &ObjectKey) -> bool {
        if self.kind != other.kind || self.raw_kind != other.raw_kind {
            return false;
        }
        match (self.server_id, other.server_id) {
            (Some(a), Some(b)) => a == b,
            _ => self.path == other.path && self.overload_sig == other.overload_sig,
        }
    }

    /// The object's own name (the last path element).
    pub fn name(&self) -> &str {
        self.path.last().map_or("", |n| n)
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

impl SourceRef {
    /// The reference of `text` (its XXH3-128 and byte length, saturated at `u32::MAX`).
    pub fn of(text: &str) -> Self {
        SourceRef {
            hash: twox_hash::XxHash3_128::oneshot(text.as_bytes()),
            len: u32::try_from(text.len()).unwrap_or(u32::MAX),
        }
    }
}

// Serde forms. `Name` is `Arc<str>` and serde's `rc` feature is off in the workspace, so the
// keys travel through plain-`String` mirrors. A kind id this build does not know reads back as
// `ObjectKind::Unknown` with the id kept in `raw_kind`, and is written back unchanged (m10).

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
    /// Only for an unknown kind whose raw text equals a known kind id (`kind` is then
    /// `"unknown"`), so it cannot read back as that known kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raw_kind: Option<String>,
}

impl From<ObjectKeyRepr> for ObjectKey {
    fn from(r: ObjectKeyRepr) -> Self {
        let (kind, raw_kind) = match (ObjectKind::from_id(&r.kind), r.raw_kind) {
            (Some(ObjectKind::Unknown), Some(raw)) => (ObjectKind::Unknown, Some(Name::from(raw))),
            (Some(kind), _) => (kind, None),
            (None, _) => (ObjectKind::Unknown, Some(Name::from(r.kind))),
        };
        ObjectKey {
            kind,
            path: r.path.into_iter().map(Name::from).collect(),
            server_id: r.server_id,
            overload_sig: r.overload_sig.map(Name::from),
            raw_kind,
        }
    }
}

impl From<ObjectKey> for ObjectKeyRepr {
    fn from(k: ObjectKey) -> Self {
        // The raw id stands in for an unknown kind (a newer build reads its own id back); a raw
        // text that collides with a known id goes in `raw_kind` beside `"unknown"`. A known kind
        // always writes its own id.
        let (kind, raw_kind) = match (k.kind, k.raw_kind.as_deref()) {
            (ObjectKind::Unknown, Some(raw)) if ObjectKind::from_id(raw).is_some() => {
                (ObjectKind::Unknown.id().to_string(), Some(raw.to_string()))
            }
            (ObjectKind::Unknown, Some(raw)) => (raw.to_string(), None),
            (kind, _) => (kind.id().to_string(), None),
        };
        ObjectKeyRepr {
            kind,
            path: k.path.iter().map(|n| n.to_string()).collect(),
            server_id: k.server_id,
            overload_sig: k.overload_sig.as_deref().map(str::to_string),
            raw_kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn key(path: &[&str], sid: Option<ServerId>) -> ObjectKey {
        ObjectKey {
            kind: ObjectKind::Table,
            path: path.iter().map(|s| Arc::from(*s)).collect(),
            server_id: sid,
            overload_sig: None,
            raw_kind: None,
        }
    }

    #[test]
    fn rename_keeps_identity_by_server_id() {
        let before = key(&["db", "public", "t"], Some(ServerId::Oid(16400)));
        let after = key(&["db", "public", "u"], Some(ServerId::Oid(16400)));
        assert!(before.same_object(&after));
        assert_ne!(before, after);
        assert_eq!(after.name(), "u");
    }

    #[test]
    fn without_server_id_rename_is_a_new_object() {
        let before = key(&["shop", "t"], None);
        let after = key(&["shop", "u"], None);
        assert!(!before.same_object(&after));
        assert!(before.same_object(&key(&["shop", "t"], None)));
    }

    #[test]
    fn overloads_are_distinct() {
        let mut f1 = key(&["db", "s", "f"], None);
        f1.kind = ObjectKind::Routine;
        f1.overload_sig = Some(Arc::from("integer"));
        let mut f2 = f1.clone();
        f2.overload_sig = Some(Arc::from("text"));
        assert!(!f1.same_object(&f2));
    }

    #[test]
    fn unknown_kinds_differ_by_raw_id() {
        let mut a = key(&["db", "s", "x"], Some(ServerId::Oid(1)));
        a.kind = ObjectKind::Unknown;
        a.raw_kind = Some(Arc::from("q"));
        let mut b = a.clone();
        b.raw_kind = Some(Arc::from("z"));
        assert!(!a.same_object(&b));
        assert_ne!(a, b);
    }

    #[test]
    fn ns_key_root_name_and_kind() {
        assert_eq!(NsKey::schema(["db", "public"]).root_name(), "public");
        assert_eq!(NsKey::server().root_name(), "");
        assert_eq!(NsKey::database("db").root_name(), "db");
        assert_eq!(NsKind::Server.root_kind(), ObjectKind::Root);
        assert_eq!(NsKind::Schema.root_kind(), ObjectKind::Schema);
    }

    #[test]
    fn source_ref_hashes_exact_text() {
        let a = SourceRef::of("SELECT 1");
        assert_eq!(a.len, 8);
        assert_eq!(a, SourceRef::of("SELECT 1"));
        assert_ne!(a.hash, SourceRef::of("select 1").hash);
    }
}
