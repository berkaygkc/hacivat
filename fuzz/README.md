# Hacivat SQL fuzz targets

A separate cargo workspace for `cargo fuzz` (nightly). It is excluded from the root workspace,
never shipped, and not checked by the root `deny.toml`. `libfuzzer-sys` is
`(MIT OR Apache-2.0) AND NCSA`; NCSA is accepted for this workspace only (ADR 0005 `m3b-t20`).

The five targets under `fuzz_targets/` (`lex_lossless`, `relex_equivalence`, `split_items`,
`params_rewrite`, `classify_no_panic`) are declared as `[[bin]]` rows in `Cargo.toml` by Task
1a (fix round 1); Task 20 wrote their bodies (no manifest edit). Each states its property in
its header and names the seeded test that runs the same property in a PR (R-M3-80).
`relex_equivalence` includes the shared edit generator
`crates/sql/sql-lexer/tests/support/edits.rs` by path. Good seeds: the files of
`crates/sql/sql-testkit/corpus/read/` and `corpus/fixtures/` (the nightly job copies them), and
the review inputs of `corpus/fuzz/classifier_seeds.toml`.

## Patches and the lockfile (P-M3b-15)

A separate workspace does not inherit the root `[patch.crates-io]`. `Cargo.toml` copies the
root's `third_party` entries with `../third_party/...` paths, so `pg_query` resolves to the
vendored copy, never the registry one. Cargo prints "patch … was not used" for `mysql_async`
and `tokio-postgres`: they are not in this graph yet, and keeping them means a future edge
can never reach the registry crates.

`Cargo.lock` is generated from the root lock: copy `../Cargo.lock` here, then run
`cargo metadata` in this directory, which keeps every shared version and drops the rest.
CI builds with `--locked` and checks that `git status --porcelain third_party` is empty.

```sh
cd fuzz
cargo metadata --locked --format-version 1 > /dev/null   # resolves pg_query to ../third_party
cargo +nightly fuzz run <target> corpus/<target> -- -max_total_time=600
```

Without nightly, `cargo build --release --locked --bins` here builds the same targets with
libFuzzer's driver but no coverage instrumentation: a random-input smoke run only. Point
`-artifact_prefix=` outside the repository.

## The M4 metadata targets (`fuzz/meta`)

`catalog_row_decode`, `system_snapshot_decode`, `scope_pattern` and `pack_parse` live in their
own package `fuzz/meta` (own workspace and `Cargo.lock`, generated the same way from
`../../Cargo.lock`), so this package's graph stays the SQL crates'. It takes
`hacivat-introspect` without its default feature `session` (ADR 0008 `m4-t01a`): the session,
network and SSH crates are never compiled for fuzzing. cargo-fuzz picks `fuzz/` from the
repository root, so name the package explicitly:

```sh
cargo +nightly fuzz build --fuzz-dir fuzz/meta --release
cargo +nightly fuzz run --fuzz-dir fuzz/meta --release pack_parse fuzz/meta/corpus/pack_parse -- -max_total_time=600
```
