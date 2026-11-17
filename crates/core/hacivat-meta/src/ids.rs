//! Identity (HV-MODEL-01 "Two identities"): [`ObjectKey`] is stable across snapshot
//! generations and refreshes; [`ObjId`] is a handle valid only inside the snapshot that
//! produced it.

use smallvec::SmallVec;

use crate::kind::ObjectKind;
use crate::name::Name;

/// The server's own id for an object, when the DBMS has a stable one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ServerId {
    /// PostgreSQL OID (reusable after wraparound; HV-INTRO-11 checks the creation marker).
    Oid(u32),
    /// SQL Server `object_id` (per database).
    ObjectId(i32),
    /// Oracle `obj#`, or a packed composite id (SQL Server `(object_id, index_id)`).
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
    /// SQLite `schema_version`.
    SchemaVersion(u32),
    /// A digest of the object's definition rows.
    Hash(u64),
}

/// Where a namespace sits in the server hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NsKind {
    /// Server-level objects (roles, tablespaces). Path `[]`.
    Server,
    /// Database-level objects (PostgreSQL extensions, casts, languages). Path `[db]`.
    Database,
    /// A schema: PostgreSQL `[db, schema]`; MySQL/MariaDB `[schema]`; SQLite `[main]`,
    /// `[temp]`, `[<attached>]`.
    Schema,
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

    pub(crate) fn code(self) -> u8 {
        match self {
            NsKind::Server => 0,
            NsKind::Database => 1,
            NsKind::Schema => 2,
        }
    }

    pub(crate) fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            0 => NsKind::Server,
            1 => NsKind::Database,
            2 => NsKind::Schema,
            _ => return None,
        })
    }
}

/// The identity of a namespace (one arena, one cache row): its kind and exact server path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NsKey {
    pub kind: NsKind,
    pub path: SmallVec<[Name; 2]>,
}

impl NsKey {
    pub fn server() -> Self {
        NsKey {
            kind: NsKind::Server,
            path: SmallVec::new(),
        }
    }

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

    /// The root object's name (the last path element; empty for the server namespace).
    pub fn root_name(&self) -> &str {
        self.path.last().map_or("", |n| n)
    }
}

/// The stable identity of an object (alg/02 HV-MODEL-01): used by the explorer tree, user
/// data (virtual FKs, notes), filters, history, MCP and diff matching.
///
/// Structural `Eq`/`Hash` compare every field. Identity across a refresh is
/// [`ObjectKey::same_object`], which prefers the server id so a rename keeps identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectKey {
    pub kind: ObjectKind,
    /// Exact server names, outermost first, ending with the object's own name
    /// (PostgreSQL `[db, schema, table, column]`, MySQL `[schema, table, column]`).
    pub path: SmallVec<[Name; 4]>,
    pub server_id: Option<ServerId>,
    /// The normalized argument type list of an overloaded routine.
    pub overload_sig: Option<Name>,
}

impl ObjectKey {
    /// Whether `self` and `other` denote the same server object: by kind and server id when
    /// both have one (a rename keeps identity), else by kind, path and overload signature.
    pub fn same_object(&self, other: &ObjectKey) -> bool {
        if self.kind != other.kind {
            return false;
        }
        match (self.server_id, other.server_id) {
            (Some(a), Some(b)) => a == b,
            _ => self.path == other.path && self.overload_sig == other.overload_sig,
        }
    }

    /// The object's own name.
    pub fn name(&self) -> &str {
        self.path.last().map_or("", |n| n)
    }
}

/// A reference to another object (FK target, partition parent, sequence owner, trigger
/// function). It stores the target's path and server id, never an arena index, so it stays
/// valid when either side's namespace is reloaded; [`crate::CatalogSnapshot::resolve`]
/// resolves it on demand (HV-MODEL-01 "References").
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ref {
    pub kind: ObjectKind,
    /// Exact server path of the target, as in [`ObjectKey::path`].
    pub path: SmallVec<[Name; 3]>,
    pub server_id: Option<ServerId>,
}

/// A generation-local handle: the namespace and the arena index. Valid only for the
/// [`crate::CatalogSnapshot`] that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjId {
    pub ns: NsKey,
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
    pub fn of(text: &str) -> Self {
        SourceRef {
            hash: twox_hash::XxHash3_128::oneshot(text.as_bytes()),
            len: u32::try_from(text.len()).unwrap_or(u32::MAX),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::smallvec;
    use std::sync::Arc;

    fn key(path: &[&str], sid: Option<ServerId>) -> ObjectKey {
        ObjectKey {
            kind: ObjectKind::Table,
            path: path.iter().map(|s| Arc::from(*s)).collect(),
            server_id: sid,
            overload_sig: None,
        }
    }

    #[test]
    fn rename_keeps_identity_by_server_id() {
        let before = key(&["db", "public", "t"], Some(ServerId::Oid(16400)));
        let after = key(&["db", "public", "u"], Some(ServerId::Oid(16400)));
        assert!(before.same_object(&after));
        assert_ne!(before, after);
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
    fn ns_key_root_name() {
        let k = NsKey::schema(["db", "public"]);
        assert_eq!(k.root_name(), "public");
        assert_eq!(NsKey::server().root_name(), "");
        let _: SmallVec<[Name; 2]> = smallvec![Arc::from("x")];
    }
}
