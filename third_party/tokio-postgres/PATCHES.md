# Hacivat patches to `tokio-postgres` 0.7.18

This directory is a vendored copy of the crates.io package `tokio-postgres` 0.7.18, used through
`[patch.crates-io]` in the workspace `Cargo.toml` (M3a Task 1b, R-M3-18; the `mysql_async`
pattern of M2b Q2, Q29). The workspace excludes `third_party/`, so the copy is not a workspace
member: its own lints, layers and tests are not Hacivat's.

## §0 Vendored copy, test, bench and CI files removed (Task 1b)

The copy is byte-identical to the registry source (`src/**`, `README.md`, `CHANGELOG.md`, the
licences, `.cargo_vcs_info.json`), except for:

- `Cargo.toml`: the `[[test]]` and `[[bench]]` targets and every `[dev-dependencies.*]` table
  are removed (the tests and benches are not shipped, and their dev-dependencies would enter
  the lock file). Nothing else changed.
- Not copied: `tests/`, `benches/`, `Cargo.lock`, `Cargo.toml.orig` and cargo's extraction
  marker `.cargo-ok`.

`diff -r ~/.cargo/registry/src/*/tokio-postgres-0.7.18/src third_party/tokio-postgres/src` is
empty after Task 1b.

**Known warning (Windows only, not patched):** `src/client.rs:33` imports `std::path::PathBuf`
under `#[cfg(feature = "runtime")]`, and only Unix-socket code uses it, so a Windows build warns
`unused import: std::path::PathBuf` (seen in CI run 43545681877). It is upstream's code, the
copy stays identical to the registry source, and the warning does not fail CI: `cargo clippy
-- -D warnings` passes its arguments to workspace members only, and the copy is not one.

Every patch is fenced in the source by a `// HACIVAT PATCH §N` comment, and each section below
lists the files and line ranges it touches, its tests and an upstream PR text.

## §1 Portals outside `Transaction`, with per-column result formats (Task 12)

Task 12 (M3a, R-M3-18, P5). Hacivat's typed streaming needs two things 0.7.18 does not offer:

1. **A named portal that is not tied to a `Transaction<'_>`.** `bind_raw` and
   `query_portal_raw` exist only on `Transaction`, which borrows `&mut Client`, sends its own
   `BEGIN` and rolls back on drop. Hacivat's connection owns its `Client`, and a console cursor
   outlives the request that opened it, inside a transaction block that Hacivat (`BEGIN`, or
   `SAVEPOINT hv_cursor` in a user transaction), the user or the read-only guard opened.
2. **The result format of each column.** `encode_bind_raw` asks for every result column in
   binary (`Some(1)`), so a type without a binary decoder (`hstore`, PostGIS, `money` under
   `lc_monetary`, arrays and composites until M3b) arrives as bytes nobody can show. Hacivat
   asks for those in text.

### Entry points (all new; the existing ones are unchanged)

- `Client::bind_with_formats(&self, statement: &Statement, params, result_formats: &[i16]) ->
  Result<Portal, Error>`: Bind into a new named portal (the existing `p{N}` counter of
  `bind.rs`) with the given result-format codes (0 text, 1 binary: one per column, one for all,
  or none for text everywhere), then one `Sync`. The caller keeps the connection inside a
  transaction block, else that `Sync` ends the implicit transaction and the portal with it.
- `Client::query_portal_raw(&self, portal: &Portal, max_rows: i32) -> Result<RowStream, Error>`:
  `Transaction::query_portal_raw` without the `Transaction` (Execute + `Sync`; the stream ends
  after `PortalSuspended` with `rows_affected()` still `None`, after `CommandComplete` with it
  set).
- `Client::query_with_formats(&self, statement: &Statement, params, result_formats: &[i16]) ->
  Result<RowStream, Error>`: `query_raw` of a prepared statement with the given result
  formats: Bind into the unnamed portal, `Flush`, `Execute(0)` and `Sync` in one batch. The
  `Flush` makes the server send `BindComplete` at once (it keeps it in its output buffer until a
  `Flush` or `Sync`), so the call returns when the statement is bound instead of when its first
  buffer of rows, or its end, arrives: a slow statement can be cancelled while it runs (found by
  Task 12's Docker test `protocol_cancel_during_execute`, where `pg_sleep(60)` held the call for
  60 s). `Flush` draws no `ReadyForQuery`, so the wire observer's `Sync` count is unchanged.

The portal is closed, with its own `Sync`, when its last `Portal` clone is dropped, as before.

### Files and ranges (line numbers after the patch; every change fenced `// HACIVAT PATCH §1`)

| File | Lines | Change |
|---|---|---|
| `src/query.rs` | 77–78, 139–140 | `query_typed`, `execute_typed`: pass `Some(1)` to `encode_bind_raw` (its old constant) |
| `src/query.rs` | 197–228 | new `query_with_formats` |
| `src/query.rs` | 298–299 | `encode`: passes `Some(1)` to `encode_bind` |
| `src/query.rs` | 306–350 | `encode_bind` and `encode_bind_raw` take `result_formats: K` (`K: IntoIterator<Item = i16>`) |
| `src/query.rs` | 370–371 | `encode_bind_raw`: hands `result_formats` to `frontend::bind` (was `Some(1)`) |
| `src/bind.rs` | 23–44 | `bind` calls the new `bind_with_formats` with `Some(1)`; `bind_with_formats` is the old body with the codes as a parameter |
| `src/client.rs` | 19–20 | `use crate::{Portal, bind};` |
| `src/client.rs` | 563–627 | the three `Client` methods |

`git diff --stat` against the pristine copy: 3 files, 141 insertions, 7 deletions; `diff -r` of
`src/` against the registry's 0.7.18 shows exactly these hunks.

### No change to existing calls

Every existing caller passes `Some(1)`, the value both encoders hard-coded, so their messages
are byte-identical: `existing_query_path_is_byte_identical` (`crates/core/hacivat-driver-pg/
tests/tp_patches.rs`) compares the bytes `query_raw` and `query_typed_raw` write with the 0.7.18
encoding built by hand from the protocol (it fails when `encode`'s codes change, checked by a
mutation run), and `existing_bind_and_execute_typed_paths_are_byte_identical` does the same for
the other two callers, `bind::bind` (through `Transaction::bind_raw`) and `execute_typed` (Task 12
fix round 1), so all four call sites are covered. The M2b suites (`tests/wire.rs`,
`tests/docker_exec.rs`, `tests/docker_pg.rs`) pass unchanged.

### Tests

`crates/core/hacivat-driver-pg/tests/tp_patches.rs` (a scripted in-memory server):
`bind_with_formats_sends_per_column_codes`, `query_portal_raw_sends_execute_max_rows`,
`suspended_portal_leaves_rows_affected_none`,
`query_with_formats_sends_bind_flush_execute_and_sync_in_one_batch`,
`existing_query_path_is_byte_identical`, and the typed path over them. Docker:
`tests/docker_typed.rs` (a suspended portal resumes across a statement, closes, and a text-format
column arrives as text).

### Upstream PR text

> **Allow portals and per-column result formats outside `Transaction`**
>
> `bind_raw`/`query_portal_raw` exist only on `Transaction`, which needs `&mut Client` and owns
> the `BEGIN`/`ROLLBACK`. Applications that manage transaction blocks themselves (a SQL console
> that runs the user's `BEGIN`, or a savepoint inside it) cannot keep a portal across calls. And
> every Bind asks for all result columns in binary, so types without a `FromSql` impl cannot be
> read as text.
>
> This adds `Client::bind_with_formats`, `Client::query_portal_raw` and
> `Client::query_with_formats`, which take the result-format codes of the Bind message
> (`&[i16]`, as the protocol defines them). `encode_bind`/`encode_bind_raw` take the codes as a
> parameter; every existing caller passes `Some(1)`, so their bytes do not change.
> `query_with_formats` sends a `Flush` after the Bind so that `BindComplete` arrives before the
> rows. The caller is responsible for being inside a transaction block when it binds a named
> portal, as the docs say.
