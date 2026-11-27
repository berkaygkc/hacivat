//! The script workspace (L14, local scope; HV-SCRIPT-01..10, -12 in
//! `docs/research/algorithms/28-script-workspace.md`, ADR 0009): a folder of plain `.sql`/`.md`
//! files becomes a script library bound to a logical database, projected under every
//! connection that shows that database.
//!
//! Everything read from a library is untrusted data (ADR 0009 D6), and Hacivat writes into a
//! library only the bytes of a script the user saved and `.hacivat/workspace.toml` (D7). All
//! file, watcher and status I/O runs on the core runtime; the UI reads [`WorkspaceSnapshot`]s.
//!
//! Core layer: no `gpui`, no UI crate, and never `hacivat-exec`, `hacivat-meta` or
//! `hacivat-session` as a normal dependency (R-M4b-28). No `notify`, `notify-debouncer-full`,
//! `encoding_rs` or `trash` type is public (`BackendInPublicApi`, `cargo xtask check-deps`).
//!
//! ## m4b-t01a-seams
//! Task 1a: every module and the types later tasks code against, with fail-closed stubs (F31;
//! none was wired into a user path until its task merged). ## m4b-gatefix-1 (E9): every stub
//! below has been filled, and the stub-only error variants are gone; the list says what each
//! stub did:
//! - [`LibraryService::open`] published an empty snapshot of unavailable libraries and scanned
//!   nothing; every other service call refused and touched no file (Task 10);
//! - [`binding::resolve`] bound nothing (Task 3); [`envscope::visible`] answered `false`
//!   (Task 4);
//! - [`scan_secrets`] answered one finding over the whole text (a save asks, never passes);
//!   Task 6 filled it (`## m4b-t06-secrets`, named exception for this line);
//! - [`fileio::read`] and [`fileio::save`] refused and read or wrote nothing (Task 5);
//! - [`manifest::read`] read nothing and bound nothing (Task 3); [`pathsafe::component`] and
//!   [`pathsafe::join`] refused every name (Task 4);
//! - [`watch::OsEventSource`] was built only with `follow_symlinks(false)` (R-T0-2) and watched
//!   nothing (Task 9; gatefix-1 replaced its debouncer, still only with that configuration).
//!
//! ## m4b-t04-scanner (named exception, fix round 1 m9)
//! Task 4 replaced three of those stubs: [`envscope::visible`], [`pathsafe::component`] and
//! [`pathsafe::join`] now apply the real rules, and [`scan::scan`] lists a library (ADR 0009
//! `m4b-t04`).
//!
//! ## m4b-t05-fileio (named exception, fix round 1 m10)
//! Task 5 replaced the [`fileio::read`] and [`fileio::save`] stubs: they read and save for
//! real (exact encodings, atomic or restoring saves), and [`recovery`] keeps the unsaved-text
//! copies (ADR 0009 `m4b-t05`). The service's own `read`/`save`/`open_stream` stay stubs until
//! Task 10.
//!
//! ## m4b-t10-service (named exception)
//! Task 10 replaced the [`LibraryService`] stubs: one actor on the core runtime owns the
//! registry, the scans, the event source and the status refresh and publishes
//! [`WorkspaceSnapshot`]s; its `read`, `save` and `open_stream` go through `fileio` with the
//! tab's environment. [`ScriptIndex`] is the "Go to script" index (ADR 0009 `m4b-t10`). The
//! file operations (`create`, `rename`, `trash`) stay fail-closed stubs until Task 13.
//!
//! Module owners: `registry`, `manifest`, `binding`, `names` (Task 3); `scan`, `pathsafe`,
//! `envscope`, `tree`, `dataless` (Task 4); `fileio`, `encoding`, `recovery` (Task 5); `secrets`
//! (Task 6); `watch` (Task 9); `service`, `index`, `events` (Task 10).

pub mod binding;
pub mod dataless;
pub mod encoding;
pub mod envscope;
pub mod events;
pub mod fileio;
// ## m4b-t13-explorer (named exception): the Scripts tree's file operations.
pub(crate) mod fileops;
pub mod index;
pub mod manifest;
pub mod names;
pub mod pathsafe;
pub mod recovery;
pub mod registry;
pub mod scan;
pub mod secrets;
pub mod service;
pub mod tree;
pub mod watch;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use binding::{DbNode, resolve};
pub use envscope::{EnvScope, EnvSet, ScopeProblem, visible};
pub use events::LibraryEvent;
pub use fileio::{
    Decoded, DiskStamp, FileMeta, LineEnding, ReadError, SaveError, SaveExpect, SaveText, SizeTier,
    TextEncoding,
};
pub use names::{DatabaseName, LogicalDb, NameError};
pub use pathsafe::{PathError, RelPath};
pub use registry::{LibraryId, LibraryRoot, LocalMapping, RegistryError};
// ## m4b-t16-attach (named line; R-T9-1)
pub use registry::WatchMode;
pub use scan::{EntryKind, LinkState, ScriptEntry};
pub use secrets::{SecretFinding, SecretKind, SpanKind, scan_secrets};
pub use service::{
    AttachError, ChunkError, FileOpError, LibraryDiag, LibraryService, LibrarySnapshot,
    LibraryState, PollReason, ScriptChunks, UnavailableReason, WorkspaceSnapshot,
};
// ## m4b-t10-service (named exception): the service's new public types.
pub use index::{Hit, Rank, ScriptIndex};
pub use service::{MapError, ServiceOptions, SourceFactory, Sources};
// fix round 1: the env-filtered view and events, the root refusals, the two-pass stream.
pub use fileio::{PassRecord, ScriptStream};
pub use service::{EnvView, GitSummary, LibraryView, RootRefusal, Subscription};
// ## m4b-t10-service
pub use watch::{EventSource, HintSink, RawHint, WatchError};
// ## m4b-t01a-seams
