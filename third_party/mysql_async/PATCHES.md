# Hacivat patches to `mysql_async` 0.37.1

This directory is a vendored copy of the crates.io package `mysql_async` 0.37.1, used through
`[patch.crates-io]` in the workspace `Cargo.toml` (M2b, Q2, Q29). The workspace excludes
`third_party/`, so the copy is not a workspace member: its own lints, layers and tests are not
Hacivat's.

## §0 Vendored copy, test, bench and CI files removed (Task 1)

The copy is byte-identical to the registry source (`src/**`, `README.md`, `README.tpl`, the
licences, `.gitignore`, `.cargo_vcs_info.json`), except for:

- `Cargo.toml`: the `[[test]]` targets, every `[dev-dependencies.*]` table and `[profile.bench]`
  are removed (cargo warns about profiles in a non-root package on every build, and the tests
  are not shipped). Nothing else changed.
- Not copied: `tests/`, `Cargo.lock`, `Cargo.toml.orig`, `azure-pipelines.yml` and cargo's
  extraction marker `.cargo-ok`.

`diff -r ~/.cargo/registry/src/*/mysql_async-0.37.1/src third_party/mysql_async/src` is empty
after Task 1.

## §1 Task 8

## §2 Task 8

## §3 Task 8

## §4 Task 8

## §5 Task 8

`SslOpts::with_rustls_client_config`: the hook through which the MySQL driver uses Hacivat's
own `TlsParams.config` (Q2). Task 13 wires it.
