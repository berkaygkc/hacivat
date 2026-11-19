//! Shared fixtures of the `hacivat-introspect` tests. Append-only: each task adds a fenced
//! block `// ## m4-tNN-<slug>` and never edits another task's.

#![allow(dead_code, reason = "each test file uses a subset of the fixtures")]

// ## m4-t01c-core

use std::sync::Arc;

use hacivat_meta::{Name, NsKey};
use hacivat_proto::DbmsKind;
use hacivat_session::profile::Profile;

/// The PostgreSQL schema namespace `[db, schema]`.
pub fn pg_ns(db: &str, schema: &str) -> NsKey {
    NsKey::schema([db, schema])
}

/// A name.
pub fn name(s: &str) -> Name {
    Name::from(s)
}

/// A saved profile of `engine` (no connection is ever opened from it here).
pub fn profile(engine: DbmsKind) -> Arc<Profile> {
    Arc::new(Profile {
        name: "seams".into(),
        engine,
        ..Profile::default()
    })
}
