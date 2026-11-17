//! The metadata model (HV-MODEL-01..03; alg/02 §1).
//!
//! - [`kind`]: the flat [`ObjectKind`] enum with stable numeric codes, kind groups and the
//!   per-DBMS kind schema (which kinds may contain which).
//! - [`name`]: server names ([`Name`], exact text) and lookup keys ([`IdentKey`], built only by
//!   a [`FoldRule`], never by locale casing).
//! - [`ids`]: stable identity ([`ObjectKey`], [`ServerId`]), namespace keys ([`NsKey`]),
//!   references that survive reloads ([`Ref`]) and generation-local handles ([`ObjId`]).
//! - [`props`]: per-kind properties ([`Props`]) with per-engine extensions.
//! - [`namespace`]: one immutable arena per schema/database ([`NamespaceSnapshot`]), built
//!   from a [`Subtree`], with contiguous children and a name index; subtree replacement.
//! - [`snapshot`]: the Arc-swapped [`Catalog`] of namespaces and its incremental update API
//!   ([`CatalogChange`]).
//! - [`codec`]: the versioned binary encoding of a namespace (the cache row payload).
//! - [`persist`]: the per-profile SQLite catalog cache ([`CacheStore`]).
//!
//! Core layer: no UI crate, no async runtime. Cache I/O is blocking; callers run it on a
//! blocking thread (§5.4).

pub mod codec;
#[cfg(feature = "test-support")]
pub mod fixture;
pub mod ids;
pub mod kind;
pub mod name;
pub mod namespace;
pub mod persist;
pub mod props;
pub mod snapshot;

pub use codec::{DecodeError, FORMAT_MAJOR, FORMAT_MINOR, decode_namespace, encode_namespace};
pub use ids::{NsKey, NsKind, ObjId, ObjectKey, Ref, ServerId, SourceRef, StateMark};
pub use kind::{KindGroup, ObjectKind};
pub use name::{FoldPolicy, FoldRule, IdentKey, Name};
pub use namespace::{
    Level, ModelError, NamespaceSnapshot, NodeId, NsMeta, Obj, ObjFlags, Subtree, Watermark,
};
pub use persist::{CacheError, CacheStore, LoadAll, Opened, RebuildReason};
pub use props::Props;
pub use snapshot::{Catalog, CatalogChange, CatalogSnapshot, Published};
