//! The metadata model (HV-MODEL-01..03, HV-INTRO-11, -14, -16 consumer side; alg/02 §1).
//!
//! - [`kind`]: the flat [`ObjectKind`] enum with stable cache codes, kind groups, the kind
//!   schema.
//! - [`name`]: exact server names ([`Name`]) and the identifier keys of `sql_ident::key`.
//! - [`ids`]: stable identity ([`ObjectKey`], [`ServerId`]), namespace keys ([`NsKey`]),
//!   references resolved on demand ([`Ref`]), generation-local handles ([`ObjId`]).
//! - [`props`]: per-kind properties and facets.
//! - [`namespace`]: one immutable arena per schema/database ([`NamespaceSnapshot`]).
//! - [`snapshot`], [`view`], [`resolve`], [`search_path`], [`search`]: the Arc-swapped
//!   [`Catalog`], its generations, the tri-state [`ModelView`].
//! - [`merge`], [`delta`]: the existence-diff merge and its [`CatalogDelta`].
//! - [`codec`], `persist` (feature `persist`), [`fingerprint`]: the cache format and store.
//! - [`overlay`], [`system`], [`userdata`], [`sql_view`]: overlays, the system-catalog layer,
//!   per-profile user data, and `sql_catalog::CatalogView` over a snapshot.
//!
//! Core layer (`MetaBelowIntrospect`): no session, driver, exec or l10n crate; no async
//! runtime (feature `catalog` takes `tokio::sync` only). Cache I/O is blocking; callers run it
//! on a blocking thread (spec §5.4).
//!
//! M4 Task 1a seams: every type is declared; every fact-producing body is a fail-closed stub
//! until its owning task lands (Global Constraints, "No silent weakening").

pub mod codec;
pub mod delta;
pub mod fingerprint;
#[cfg(feature = "test-support")]
pub mod fixture;
pub mod ids;
pub mod kind;
pub mod merge;
pub mod name;
pub mod namespace;
pub mod overlay;
#[cfg(feature = "persist")]
pub mod persist;
pub mod props;
pub mod resolve;
pub mod search;
pub mod search_path;
pub mod snapshot;
pub mod sql_view;
pub mod system;
pub mod userdata;
pub mod view;

pub use delta::CatalogDelta;
pub use ids::{NsKey, NsKind, ObjId, ObjectKey, Ref, ServerId, SourceRef, StateMark};
pub use kind::{KindGroup, ObjectKind};
pub use name::{IdentKey, Name};
pub use namespace::{Level, ModelError, NamespaceSnapshot, Obj, ObjFlags, Subtree, Watermark};
pub use snapshot::{
    Catalog, CatalogChange, CatalogEvent, CatalogSnapshot, LoadState, ObjHandle, Publisher,
    ServerFacts,
};
pub use view::{Lookup, MissingReason, ModelView, SnapshotView, UnknownReason};
