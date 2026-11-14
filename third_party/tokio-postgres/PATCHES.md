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

Task 12 (R-M3-18, P5): `Client::bind_with_formats`, `Client::query_portal_raw` and
`Client::query_with_formats`. Not applied yet.
