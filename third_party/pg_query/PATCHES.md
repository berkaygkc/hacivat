# Hacivat patches to `pg_query` 18.1.0

This directory is a vendored copy of the crates.io package `pg_query` 18.1.0 (PostgreSQL 18.6
grammar through libpg_query), used through `[patch.crates-io]` in the workspace `Cargo.toml`
(M3a Task 1a; R-M3-1, pre-flight P1). The workspace excludes `third_party/`, so the copy is not a
workspace member: its own lints and tests are not Hacivat's. Only `crates/sql/sql-oracle-pg`
depends on it (`cargo xtask check-deps`, `BackendOutsideOwner`).

Package checksum (crates.io, `Cargo.lock` before the patch):
`457bfe74085c505ecaf5468416450969e2e52a66d26bc1a4b2f6944a2daee73f`. Upstream commit
(`.cargo_vcs_info.json`): `pganalyze/pg_query.rs@20f3a182dc74ef0406752548bc6fb79fbfb591b1`,
whose `libpg_query` submodule is `pganalyze/libpg_query@c1546e7e97edc93fe8474d7a59881e91fbf48cfc`.

## §0 Vendored copy; test, bench and CI files removed; upstream licence files restored (Task 1a)

The copy is byte-identical to the registry source (`build.rs` and `src/**` apart from §1,
`README.md`, `libpg_query/**`, `.cargo_vcs_info.json`), except for:

- `Cargo.toml`: the four `[dev-dependencies.*]` tables (`brunch`, `easy-parallel`,
  `pretty_assertions`, `regex`) are removed; the tests and benches that used them are not in the
  package. The normalised manifest has no `[[test]]`, `[[bench]]`, `[[example]]` or profile
  sections. §1 removes one build-dependency. Nothing else changed.
- Not copied: `Cargo.lock`, `Cargo.toml.orig` and cargo's extraction marker `.cargo-ok`. The
  package has no `tests/`, `benches/` or CI files.
- **Added**: the upstream licence files that the package's `include` list leaves out. Each is
  byte-identical to the file at the commit above (git blob SHA in brackets):
  - `LICENSE`: pg_query.rs, MIT (`ba39f22b3c8ce2405f3b00faa4cc03058051fd8a`).
  - `libpg_query/LICENSE`: libpg_query, BSD-3-Clause (`5fddafa15b7cbe3cd1d043ed332af844e17246b1`).
  - `libpg_query/src/postgres/COPYRIGHT`: the PostgreSQL License
    (`0a397648dcd3c2177acc58bd7daecd11ad64be62`).
  - `libpg_query/vendor/upb/LICENSE`: upb, BSD-3-Clause (`19b305b00060a774a9180fb916c14b49edb2008f`).
  - `libpg_query/vendor/upb/third_party/utf8_range/LICENSE`: utf8_range, MIT
    (`e59f14fa55de789cc2aa634e9788e0514be74611`).

  Why: this repository redistributes the C sources, and BSD-3-Clause requires a source
  redistribution to retain the notice. `deny.toml`'s `[[licenses.clarify]]` for `pg_query` hashes
  `LICENSE` and `libpg_query/LICENSE`. `THIRD_PARTY_NOTICES.md` quotes all five, together with
  the notices that live in source headers (xxHash in `vendor/xxhash/xxhash.h`, Todd C. Miller's
  ISC notice in `src/postgres/src_port_strlcpy.c`, Hideaki Ohno's MIT notice in
  `src/postgres/include/lib/hyperloglog.h`).

`diff -r ~/.cargo/registry/src/*/pg_query-18.1.0 third_party/pg_query` shows exactly these
lines: `Cargo.toml` and `build.rs` differ; `Cargo.lock`, `Cargo.toml.orig` and `.cargo-ok` exist
only in the registry; this file and the five licence files exist only here.

Every source change is fenced by a `HACIVAT PATCH §N` comment.

## §1 `build.rs`: no protobuf regeneration (Task 1a; R-M3-1, P1)

- `build.rs` (upstream l.84-114): the branch that ran prost-build is removed. It ran when
  `REGENERATE_PROTOBUF` was set or `protoc --version` could be spawned (not when it succeeded),
  generated `pg_query.rs` from `libpg_query/protobuf/pg_query.proto` and copied it over
  `src/protobuf.rs` inside `CARGO_MANIFEST_DIR`, that is inside the cargo registry or this
  directory. The two names only that branch used go with it: the `std::process::Command` import
  and `out_protobuf_path`.
- `Cargo.toml`: `[build-dependencies.prost-build]` is removed; nothing else used it.
- Effect: the shipped, pre-generated `src/protobuf.rs` (prost 0.13 code for the same
  `pg_query.proto`) is always used. A `protoc` on `PATH`, as on the maintainer machine, changes
  nothing, so two parallel builds cannot race on the file and no build rewrites the source.
  bindgen is kept: every build still generates `bindings.rs` from `pg_query.h` into `OUT_DIR`,
  so libclang is still needed and proven on macOS, Windows and Linux.
- Test: `crates/sql/sql-oracle-pg/tests/build_proof.rs` (statements are counted through the
  protobuf decode); CI runs `git diff --exit-code -- third_party/pg_query` after the build.
- Upstream: not proposed. Regenerating on a developer machine is upstream's intended behaviour.
