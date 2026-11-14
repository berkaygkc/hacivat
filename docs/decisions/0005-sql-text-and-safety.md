# ADR 0005 — SQL text and safety

- Status: Accepted
- Date: 2026-11-14
- Scope: M3a (the `crates/sql` family: SQL-LEX-01 to -05, SQL-SPLIT-01 to -04, the PostgreSQL
  parser oracle; R-M3-1, R-M3-3, R-M3-4, R-M3-35); M3b appends the classifier (R-M3-2)

## Context

M3a adds the SQL text layer: a lossless table-driven lexer, a splitter with an editor mode and
an execution mode, `SqlDocument`, the caret-to-units chooser, and `sql-oracle-pg`, which wraps
libpg_query (PostgreSQL's own parser) to resolve PostgreSQL boundaries the token splitter
cannot prove. What this layer calls a statement is what Run executes, so a wrong boundary is
the "wrong-range Run" anti-goal of spec §2.3. M3b builds the safety classifier on the same
crates.

libpg_query is C, compiled by the `pg_query` crate's build script with `cc` and bound with
bindgen (libclang). It is the one native build in the SQL stack, so it has to be proven on
macOS, Windows and Linux before any task depends on it.

## Rule

Each task that makes a choice in this area appends one section headed `## m3a-tNN — <topic>`
(M3b: `## m3b-tNN — <topic>`), append-only. A choice made in a fix round goes into the same
task's section before re-review. A section states the choice, the alternatives it rejected, how
it fails closed, and the tests that hold it. M3a's sections come from Tasks 1a, 1b and 13.

## m3a-t01a — proofs

**libpg_query through a vendored `pg_query` 18.1.0 (R-M3-1, P1).** The crates.io package is
copied to `third_party/pg_query` and used through `[patch.crates-io]`, as `mysql_async` is.
`third_party/pg_query/PATCHES.md` holds the details:
- §0: a pristine copy. The dev-dependencies, `Cargo.lock`, `Cargo.toml.orig` and `.cargo-ok`
  are removed, and the five upstream licence files the package leaves out are restored
  byte-identical.
- §1: `build.rs` no longer regenerates `src/protobuf.rs`. Upstream regenerates it whenever
  `protoc --version` can be spawned, rewriting the file inside the crate directory, that is
  inside the cargo registry. This machine has Homebrew's `protoc` (`libprotoc 32.0`) on `PATH`,
  so every local build rewrote the registry and parallel builds raced. The `prost-build`
  build-dependency, used only by that branch, is gone too.
- bindgen is kept, so libclang is needed, and proven, on every OS.
- Alternatives rejected:
  - Keeping the registry crate and running cargo without `protoc` on `PATH`: one forgotten
    shell, and the registry is rewritten again.
  - Pre-generated `bindings.rs`: kept as R-M3-1's second step, in case the Windows or
    universal-macOS proof fails.
- The version is pinned `=18.1.0`. A bump is a manual re-vendor that re-applies §1.

**Licences.** The package declares only `MIT`, but the binary also compiles:
- libpg_query (BSD-3-Clause);
- the PostgreSQL parser sources (PostgreSQL License), which include three files under other
  licences: `snprintf.c` (BSD-3-Clause; Eric P. Allman and the UC Regents), the NetBSD qsort in
  `sort_template.h` (BSD-3-Clause; UC Regents) and OpenBSD's `strlcpy` (ISC);
- upb (BSD-3-Clause) with utf8_range (MIT);
- xxHash (BSD-2-Clause).

`deny.toml` handles this in two entries:
- A `[[licenses.clarify]]` sets
  `MIT AND BSD-3-Clause AND BSD-2-Clause AND ISC AND PostgreSQL`. Its `license-files` are the
  hashes of `LICENSE` and `libpg_query/LICENSE`, the XXH32 that cargo-deny computes. A changed
  file voids the clarification.
- A crate-scoped `[[licenses.exceptions]]` allows `PostgreSQL` for `pg_query` alone. Without it
  `cargo deny check licenses` rejects the crate (checked).

The plan named four licences; the rest came from a licence pass over the vendored copy.
- Task 1a's first pass was a `grep` of copyright lines, its output cut to the 40 most frequent
  and read by hand. The two BSD notices of `snprintf.c` and `sort_template.h` fell below that
  cut, and review round 1 found them.
- The pass of fix round 1 is mechanical and has three steps:
  1. It scans every file of `third_party/pg_query` for licence wording (grant, condition and
     disclaimer phrases, "public domain", SPDX tags) and lists every copyright line that is not
     the standard PostgreSQL header pair.
  2. It runs `clang -MM -MG` over all 96 C files `build.rs` compiles, with its include paths, for
     `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`. Together
     they compile or include 659 files of the tree.
  3. For each flagged file that is compiled or included, it checks whether code under that
     licence reaches the binary: callers in the compiled sources, and symbols in the built
     `libpg_query.a`.

A notice is quoted when code under it is compiled into the binary.
`THIRD_PARTY_NOTICES.md` quotes nine notices verbatim (checked mechanically against their
sources), plus the extra Google copyright lines of the compiled upb and utf8_range headers.

The file ships through a new `resources` key in `packaging/Packager.toml`. cargo-packager
resolves that key from the rendered config's directory, so the path is
`../THIRD_PARTY_NOTICES.md`. A local `cargo packager --formats app` put it in
`Hacivat.app/Contents/Resources`, byte-identical. cargo-packager places resources next to the
NSIS executable and in the AppImage and deb trees; the `nightly-package` run checks those
bundles.

Several flagged files need no notice, each for a recorded reason:
- Bison's parser skeleton exception covers the generated parsers.
- `wcwidth` is in the public domain.
- Henry Spencer's `regex.h` is declarations only, and its licence binds source form only.
- `pg_getopt.h` and Hideaki Ohno's HyperLogLog header are declarations only; their
  implementations are not compiled. Task 1a first quoted the HyperLogLog notice; it moved to
  this list in fix round 1.
- Zilong Tan's fasthash (MIT, `hashfn_unstable.h`) is `static inline` code that nothing
  compiled calls. The built library has no fasthash symbol.

The vendored files keep all these notices for the source redistribution.

**The build proof.** `sql-oracle-pg` (Task 1a's minimal crate) works with three items:
- `version()` reads `PG_VERSION_NUM` from the linked C library at run time:
  `pg_query 18.1.0 (PostgreSQL 18.6)`.
- `statement_count()` parses through the protobuf decode, so the shipped `protobuf.rs` is
  checked against the C library.
- `PgError { message, cursor }` holds the error. `cursor` is `None` until Task 9 maps
  libpg_query's position.

No `pg_query` type is public: the error conversion is a private function, not a `From` impl.
`tests/build_proof.rs` runs in `build-test` on all three OSes.

`--smoke` prints `libpg-query=pg_query 18.1.0 (PostgreSQL 18.6) statements=1`, and
`launch-smoke` checks that line. Until Task 1b nothing else in the `hacivat` binary reaches
`pg_query`; with this line, the universal macOS bundle compiles libpg_query for both
architectures and the size delta is real.

**CI.** A composite action, `.github/actions/libclang`, runs in every job that builds a crate
reaching `pg_query`: `build-test`, `turkey-test`, `switch`, `launch-smoke`, `bench`,
`nightly-package`, `pseudo-screens`, `docker-pr` and `docker-full`. It does three things:
- On Linux it installs `libclang-dev` (`linux-deps` installs it too).
- On Windows it exports `LIBCLANG_PATH` to the runner's LLVM.
- It prints `clang --version` and, when present, `protoc --version` (unused).

`build-test` prints the release binary's size and fails if a build changed or added anything
under `third_party/pg_query`.

Local record (macOS arm64):
- `clang --version`: `Apple clang version 17.0.0 (clang-1700.6.3.2)`.
- A cold libpg_query build takes about 25 s on 14 cores, debug at `opt-level = 1`.
- `third_party/pg_query` stays unchanged after every build, with `protoc` on `PATH`.

**Binary size.** `target/release/hacivat`, macOS arm64:
- Before: 37,138,656 bytes (35.42 MiB), the M2b gate's build, measured before anything rebuilt
  it.
- After: 40,375,712 bytes (38.50 MiB), the GPUI rev move plus libpg_query reached through
  `--smoke`.
- Delta: +3,237,056 bytes (+3.09 MiB), inside the ≤ +10 MiB per-architecture budget. No
  `opt-level = "s"` or LTO ruling is needed.
- The universal (x86_64 + arm64) number comes from the controller's `nightly-package` run.

**The TestScheduler teardown deadlock (R-M3-13).** The GPUI fork moves from `a1eaa20` to
`6607045`, "scheduler: take state before rng in block_until (Hacivat carry)":
- What it fixes: `TestScheduler::block_until` held its `rng` lock while it locked `state` for
  a timed block. `schedule_*` from another thread locks them in the other order.
- Licence: the scheduler crate is Apache-2.0, checked at the fork head before patching.
- Record: `crates/ui/hacivat-ui-core/patches/scheduler-block-until-lock-order.patch`.
- The `rev` edit moved the 21 fork entries of `Cargo.lock` and nothing else.

`crates/app/hacivat-app/tests/teardown_stress.rs` holds the proof, each part under a watchdog
that ends the test binary when a cycle hangs:
- `fifty_core_boot_and_quit_cycles_never_hang`: 50 cycles that boot the app with the core
  runtime, connect through `FakeDriver` and quit for real. It never hung locally at either rev
  (1,000 cycles at `a1eaa20`), because the window is one expression per timed block.
- `timed_blocks_while_another_thread_schedules_never_hang`: timed blocks while another thread
  spawns without pause. It hung in 5 of 5 runs at `a1eaa20` and passes in about a second at
  `6607045`.
- `watchdog_fails_a_hung_cycle`: re-runs the binary with a cycle that never returns, and
  expects exit 86 and a message naming the cycle.

Production is not affected: it runs GPUI's platform scheduler.

## m3a-t01b — seams

**The SQL crates stay runtime-free and core-free.** `check-deps` gains four rules for M3a:

- `SqlNotRuntimeFree`: a crate under `crates/sql` (except the test kit) may not depend, normally,
  on `tokio`, `futures`, `async-std`, `smol`, `gpui`, `hacivat-l10n`, `salsa` or `sql-sema`
  (`SQL_FORBIDDEN`: the guard's `GUARD_FORBIDDEN`, whole, plus the two other runtimes), so text
  analysis stays synchronous and testable without a runtime.
- `SqlReachesCore`: no SQL crate depends on a core `hacivat-*` crate (any `hacivat-*` package
  outside the UI and app layers counts as core, so a new one fails closed); the SQL layer sits
  below the core, and the app adapts between them.
- `BackendOutsideOwner`: only `sql-oracle-pg` depends on `pg_query` (libpg_query stays behind one
  crate and its stack guard).
- `TextCoreNotHeadless`: `hacivat-text` never reaches `hacivat-ui-core` or `gpui`.

**M5's exemptions, named now (P22).** Spec §5.1 rule 3 restricts only `sql-safety`,
`sql-resolve`, `sql-parser` and `hacivat-guard`; M5's incremental analysis crates `sql-sema` and
`sql-ide` will use `salsa`. M3a keeps `salsa` and `sql-sema` forbidden for every M3a SQL crate;
when M5 adds `sql-sema` and `sql-ide`, they (and only they) are exempted from the `salsa` part of
`SqlNotRuntimeFree`, by name, in that task's section of this ADR.

**Fail-closed stubs.** Until their tasks land, the lexer returns one `Unknown` token for the whole
text, the splitter one `Heuristic` statement, `exec_range::resolve` `NothingToRun`,
`head::cursor_eligible` `false`, `split_with_parser` an error, and `lex_profile` an error for
the engines M3a does not lex (SQL Server, ClickHouse, Oracle). `SqlDocument` and `DocSnapshot`
work (they hold the text and its version; the execution split is computed once per snapshot).

## m3a-t09 — the PostgreSQL parser oracle

**One parser thread, not a thread per call (R-M3-3, R-M3-35).** Every libpg_query call runs as
a job on one long-lived thread with a 16 MiB stack, `hacivat-pg-parser`. That covers `version`,
`statement_count` and `split_with_parser`, and the scans and prefix parses behind them.
`on_parser_stack` queues the job and waits for it:
- A job that calls `on_parser_stack` again runs inline.
- A job that panics returns `None`, and the thread stays.
- Jobs from all threads run one at a time.
- It blocks the caller until its job, and every job queued before it, has run. There is no
  timeout, and a C call cannot be cancelled. No caller may use the oracle on the UI thread:
  callers run it on a background executor and coalesce requests so only the latest waits
  (Tasks 23 and 24). A timeout variant, which would only let a caller give up early, is left
  for M3b.

The plan asked for a fresh thread per call. That was rejected for three reasons:
- libpg_query points fd 2 at a pipe for each parse and then points it back. The redirect is in
  `pg_query_raw_parse` (compiled without `DEBUG`), which is behind both C calls this crate
  parses with: `pg_query_parse_protobuf_opts` (`pg_query::parse`) and
  `pg_query_split_with_parser` (`pg_query::split_with_parser`). The scans
  (`pg_query_scan_tokens`) do not redirect. Two parses at once can restore each other's pipe
  and leave stderr on a closed pipe for the rest of the process. A thread pool would race in
  the same way.
- A fresh thread per call leaks memory. Measured on macOS: about 8 KiB of resident memory per
  thread (5,000 threads: +40 MiB; the same 5,000 calls on one thread: +64 KiB).
- libpg_query also creates a pthread key per thread and never deletes it, and on Windows it
  frees nothing when a thread exits.

The cost: calls are serialised. The oracle runs only for ambiguous PostgreSQL units (R-M3-24),
and a 1 MiB statement takes about 9 ms (release, macOS arm64).

**The guard before every parser call.** In order:
1. A NUL is an `Err` with its cursor. libpg_query takes a C string, so a NUL would cut the
   text and the parser would see a shorter, possibly valid, statement.
2. A text over 8 MiB is an `Err`, before any scan, so the caller keeps its boundary uncertain.
   The oracle's input is one ambiguous statement region. Its peak memory is about 135 bytes
   per token: libpg_query's token array and its copy, this crate's two token lists, and then
   the raw parse tree. An 8 MiB flat `SELECT 1,1,…` peaked at +1.1 GiB (review T9 M1), so the
   cap bounds one call near that, and its time on the one thread to seconds. The cap was
   64 MiB before the review.
3. On the parser thread, PostgreSQL's own scanner reads the text. It is flex-generated and
   iterative, so no input exhausts its stack.
4. Brackets (`(` and `[` together) nested deeper than `MAX_ORACLE_NESTING` (256) are an `Err`.
   Its cursor is on the first bracket past the limit, and nothing is parsed.

If a lexical error hides the tokens and cannot be placed, the bound is the number of bracket
bytes in the whole text. The bracket count cannot see some deep trees. For those,
libpg_query's own stack-depth check (at most 2 MB, sized from the thread it runs on) and
bison's 10,000-entry stack end the parse with an error. The tests cover 100,000 `NOT`s, 20,000
nested `CASE`s and 30,000 unary minus signs. Left-deep trees parse: 2 MB of `+` and 50,000
`UNION`s.

**Statement ranges.** `split_with_parser` returns one range per statement:
- A range runs from the statement's first token to the end of its last token. Whitespace,
  comments and the `;` are outside it.
- The text is parsed as written first. Only if that fails are named placeholders normalised
  and the text parsed again. `:name`, `:'name'` and `:"name"` become `$1` plus spaces, so
  every offset stays the caller's. The rewrite applies only to a `:` token directly followed
  by the name token, and not after a word byte or `$`. Without that rule, `$t:x` would become
  `$t$1`, which opens a dollar quote.
- Parsing as written first keeps valid array slices out of the rewrite: `a[1 :n]`,
  `a[(1):n]`, `a["i":n]` and `a[:n]`. Those are the only grammatical `:`-name in PostgreSQL
  (review T9 M2). A text with both such a slice and a placeholder is still an `Err` (fail
  closed).
- The ranges are checked against the scanner's tokens. Every token outside them must be a `;`,
  and consecutive statements must have one between them. Any mismatch is an `Err`, so the
  caller keeps the boundary uncertain.

`statement_count` counts the raw parser's statements and builds no output tree. prost's
protobuf decode stops at 100 nested messages, and ordinary SQL reaches that: `unions(100)` and
50 nested `ARRAY[…]` failed. Task 1a's decode proof stays in `version`, which reads
`PG_VERSION_NUM` from a full parse of `SELECT 1` and requires one decoded `SelectStmt`.
`version` keeps only a successful read, so a transient first failure (the thread cannot start)
does not stick as `unknown`. `statement_count` does not normalise placeholders: `SELECT :a` is
an `Err` there and one statement in `split_with_parser`. Only the build proof and `--smoke` use
it.

**Error cursors without `cursorpos`.** The `pg_query` crate drops libpg_query's cursor, and the
workspace denies the `unsafe` that reading the C struct would need. So `error::locate`
rebuilds the cursor from the message and the text. It gives an exact byte offset or `None`,
never a guess:
- For `at end of input`, the cursor is the end of the text.
- For an unterminated construct, the cursor is the length of the text minus the rest the
  scanner printed.
- For text that occurs once, the cursor is that occurrence.
- Otherwise, the cursor is the first candidate token whose prefix fails with the same message.
  For a keyword the lookahead filter recodes, the prefix includes the next token.

The cursor is `None` for grammar-action errors, for neighbouring recoded keywords (`NOT NOT`),
for any `U&` token and for a repeated lexical-error text.

A scratch C harness read `cursorpos` from the same static library to check this. Over 100,000
generated mutations (75,362 errors), the result was 75,353 exact, 9 `None` and 0 wrong. The
ranges of 25,919 splittable texts, 20,000 of them multi-statement, matched libpg_query's own
ranges. `tests/oracle.rs` keeps representative cases, each with libpg_query's value.

**Upstream defects found and contained:**
- `pg_query::scan_tokens` builds a slice from a null pointer when a text has no tokens (a panic
  in debug builds, undefined behaviour in release). Whitespace-only texts never reach it.
- The fd 2 swap and the per-thread leak are contained by the single parser thread.
- prost's decode limit is avoided by building no tree. M3b note (R-M3-2): a classifier that
  decodes full parse trees fails on SQL nested more than about 50 levels.

Tests: `crates/sql/sql-oracle-pg/tests/{oracle,threads,depth,perf}.rs`, plus unit tests in
`error.rs` and `split.rs`.
