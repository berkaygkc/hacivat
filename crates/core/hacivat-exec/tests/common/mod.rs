//! Shared fixtures of the run-service tests (append-only: each task adds one block headed
//! `// ## m3a-tNN-<slug>`; Task 23's helpers, then Task 31's matrix helpers).
#![allow(dead_code, reason = "each test file uses a subset of the fixtures")]
#![cfg_attr(not(unix), allow(unused_imports, unused_variables))]

// ## m3a-t01b-seams

use hacivat_proto::DbmsKind;
use hacivat_secrets::{AuthProviderId, Retention};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes};
use sql_split::exec_range::CaretOrSelection;
use sql_split::{DocSnapshot, SqlDocument};
use sql_text::TextSize;

/// A profile of `engine` that a `FakeDriver` session connects without a password or a socket.
pub fn profile_of(engine: DbmsKind) -> Profile {
    let endpoint = match engine {
        DbmsKind::Sqlite => ProfileEndpoint::File {
            path: std::env::temp_dir().join("hv-exec-test.db"),
            create_if_missing: true,
        },
        _ => ProfileEndpoint::Server {
            host: "127.0.0.1".into(),
            port: None,
        },
    };
    Profile {
        name: "orders".into(),
        engine,
        endpoint,
        user: Some("deploy".into()),
        auth: AuthProviderId::None,
        retention: Retention::Never,
        ..Profile::default()
    }
}

/// A PostgreSQL 18 context with the default modes.
pub fn pg18() -> DialectCtx {
    DialectCtx {
        id: DialectId::Postgres,
        version: Some(sql_dialect::ServerVersion {
            major: 18,
            minor: 0,
            patch: 0,
        }),
        modes: LexModes::default_for(DialectId::Postgres),
    }
}

/// A PostgreSQL document's snapshot.
pub fn doc(text: &str) -> DocSnapshot {
    SqlDocument::new(text.to_string(), pg18(), ClientFlavor::None).snapshot()
}

/// A caret at byte `at`.
pub fn caret(at: u32) -> CaretOrSelection {
    CaretOrSelection::Caret(TextSize::from(at))
}
