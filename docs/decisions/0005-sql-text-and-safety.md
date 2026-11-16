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

## m3a-t13 — caret and selection to execution units

`sql_split::exec_range::resolve` decides what Run sends for a caret or a selection, for the
preview (`SplitMode::Editor`) and for execution (`SplitMode::Execution`). SQL-SPLIT-04 is
aligned to D-011 here: the default policy is Ask with the one-candidate shortcut, nothing
outside a statement, and a selection as separate statements.

**One agreed split (R-M3-8).** Both modes read the same statements, the editor's split, which
is what the preview draws. They differ in one thing: execution refuses a unit that is not
certain, where the preview returns it to be drawn as uncertain. A unit is certain when its
statement is `Exact` in the editor's split with no uncertain boundary inside, and the execution
split holds a certain statement with the same strict range. So what Run sends is always a range
the preview showed as exact, and an uncertain preview never runs (R-M3-24).
- Rejected: resolving execution mode on the execution split. The two splits differ where the
  editor guesses: after an unterminated comment the execution split has one certain statement
  whose smart range runs to the end of the text, and the editor has provisional statements
  there. A caret in that tail then ran a statement the preview did not show. The properties
  found this.

**What is sent.** A unit's range is the statement's strict range: no terminator (MySQL rejects
it over the protocol) and no leading comments, which are the unit's `title_comment`. A `COPY …
FROM STDIN` statement is `UnitKind::CopyIn` with its data range; the rows are never in a sent
range, by a caret or by a selection. A selection that overlaps a `COPY`'s rows without
starting at or before that `COPY` statement's first byte is refused, wherever the statement
sits: another statement or an empty `;` may follow it on its line (`COPY t FROM STDIN; SELECT
1;`), and the data still starts on the next line. The splitter keeps every `COPY` with its
rows in a list of its own for this; the rule does not look for "the item before the rows".

**The statement at a caret** (`alg/10` SQL-SPLIT-04 step 2), in order:
1. The statement whose smart range holds the caret, ends included. The smart range is the split
   layer's (SQL-SPLIT-02): attached leading comments, the terminator, and a comment after the
   terminator on the same line. Where two statements touch (`SELECT 1;┃SELECT 2;`) the earlier
   one is meant.
2. An unsupported client command at the caret is `UnsupportedClientCommand`.
3. A caret after a statement on its line, with only spaces or tabs between and only white
   space after it on the line, means that statement.
4. Otherwise the caret is outside every statement, and the outside policy decides.

**Comment ownership.** A comment after `;` on the same line belongs to the statement before
it, as the split layer has it. The CARET, SEL and CHOOSE families hold no case that says
otherwise, so `exec_range` adapts nothing. `alg/10` SQL-SPLIT-04's DBeaver-26416 note
(`-- Comments\nselect 10`) reads that comment as the next statement's title; it is not taken.
Either way the sent text is the same strict range.

**The chooser (HV-EXEC-02, R-M3-36).** Candidates are tried in the order subquery, statement,
largest-or-batch, from-caret, whole script. A candidate that runs the same ranges as an earlier
one is dropped, except that a from-caret equal to the whole script is dropped in favour of the
whole script, so from-caret only appears as a strict subset. One candidate runs directly. The
remembered kind is preselected when offered, else the statement.
- In execution mode a candidate that would be refused is left out. If the candidate the
  chooser would open on (the remembered kind) is the refused one, the answer is that refusal:
  Run never runs a smaller range in its place.
- Wherever Run runs or asks, the preview gives the same answer: it leaves out the candidates
  execution refuses too. So when the filter leaves one candidate, the preview shows that one
  statement and Run runs it; a chooser is never previewed over an immediate run (review M4).
  The other way, keeping a chooser with one entry, was rejected: D-011 runs directly when
  there is one candidate. Where Run refuses (an uncertain statement at the caret, an
  uncertain range to open on), the preview keeps the uncertain candidates, to draw them.
- The subquery candidate needs a `SubqueryOracle` (M5). A range it names outside the statement
  at the caret is ignored.
- A label is the candidate's text from its first leading comment, with runs of white space and
  control characters as one space. Every invisible formatting character is shown as
  `<U+XXXX>`: the bidirectional controls, which would reorder the SQL, and the zero-width,
  soft-hyphen, word-joiner, BOM and tag characters (U+00AD, U+200B–U+200F, U+2060–U+2064,
  U+FEFF, U+E0000–U+E007F), which would hide a difference. The set is Unicode's
  `Default_Ignorable_Code_Point` with its reserved ranges (so also U+034F, the Hangul fillers,
  U+180B–U+180F, the variation selectors, U+1BCA0–U+1BCA3, U+1D173–U+1D17A, U+E0000–U+E0FFF)
  plus the interlinear annotation controls U+FFF9–U+FFFB. A label holds at most 1,024
  characters; the popup truncates further.

**Refusals (fail closed).**
- A run of several statements (whole script, from the caret, below the caret, several carets,
  an expanded selection) that spans an unsupported client command is
  `UnsupportedClientCommand`, in both modes. A whole-script run spans the whole text. psql
  `\if` branches are uncertain in the split and so refused as `Ambiguous`.
- A selection has one answer for both modes and never holds a heuristic statement. It is split
  on its own, from the lexer state and client delimiter at its start. Any uncertain boundary or
  unsupported client command in it refuses the whole selection.
- **A selection is only what it looks like when its ends fall between the document's tokens.**
  A unit is sent alone and the server reads it from a clean state, so
  `DocSnapshot::selection_cuts` refuses, from the document's own tokens and `COPY` ranges and
  never from lexing the selected text by itself:
  - a start inside a token, after its first byte: the rest of a string, a quoted name or a
    dollar-quoted body (a prefix such as `E` or `U&` and the opening quote are part of the
    token), of a word, a number or an operator (`UnterminatedConstruct`), and the inside of an
    executable comment or a hint, openers `/*!`, `/*M!` and `/*+` included
    (`AmbiguousCommentEnd`, as the caret path answers there);
  - an end inside a token (`«DROP TABLE cust»omers_backup`);
  - a start inside a `COPY … FROM STDIN` statement, after its first byte, with rows selected:
    the rows are data;
  - a start inside a `--` comment that runs past the comment's end: read alone, a quote in
    the comment's text would pair with the quotes after it;
  - a start or an end strictly inside an `AmbiguousCommentEnd` boundary of the document: after
    an executable comment's or hint's lexer end, but before the end the server may read
    (`/*! , "*/« DROP TABLE t;» -- " */`), the server may still be inside the comment.
  Offsets inside a character are moved back to its start first.
  White space and plain comments are not cut. The part of a comment at either end of a
  selection is left out as trivia, and text selected wholly inside one `--` comment runs as
  selected, so a statement kept in a comment can be run. A selection that starts inside a
  client command's line is `UnsupportedClientCommand`.
- After a client command, a client delimiter or `COPY` data the splitter lexes again, and its
  tokens can differ from the document's one lexing (a quote in `COPY` data opens a "string"
  there). Both readings are asked, and either one saying "inside a token" refuses. The costs
  of that, all fail-closed (the caret still runs each statement):
  - in a dump whose `COPY` data holds an odd number of quotes, a selection after that data is
    refused;
  - the same after an odd quote in a client command's argument (`\echo don't`,
    `source it's.sql`, `.print it's`): the document's one lexing reads the rest of the file
    as a string, so every later selection is refused, a whole clean statement too;
  - after any client command or `COPY` block, a comment of the document's one lexing is not
    trusted, so text selected inside a `--` comment there is refused, and so is a selection
    edge inside a `/* … */` comment. "A statement kept in a comment can be run" holds only
    before the first such place.
  Follow-up for the lexer (Task 4's crate, not done here): end a client command's argument at
  its line end in the document's lexing. That removes the second and third costs for
  commands. Listed for the PO.
- Speed. A selection's edges are looked at once per call, and the splitter's second lexing is
  picked up at a checkpoint: the structural pass records a token start about every 2 KiB,
  where the lexer's state is `Initial`. Without them each look lexed again from the last
  restart, which on a 50,000-line script with a psql command on line 1 is the whole text
  (34 ms p95 for one selection, against HV-EXEC-01's 2 ms).
- The same rule is applied twice: `exec_range` refuses on `selection_cuts` before it splits,
  and `DocSnapshot::split_selection` marks the cuts itself for any other caller. There the
  rest of a token the selection starts inside, with what follows it in its statement, is one
  `Heuristic` statement, and splitting goes on from the document's own state after the token.
  So the split never runs out of step with the document: the rest of a string is never read
  as code, nor the code after it as a string. Where the selection's own split reads a client
  command or `COPY` data that the document does not have at that place (a selection that
  starts in the middle of a line can make the line's rest look like a command line), every
  statement of it is `Heuristic`.
- History of this rule. Task 13's properties first found that `split_selection` returned a
  carried token's tail as a certain statement (`SELECT 'a; DROP TABLE t; '` selected from
  inside the literal gave ` DROP TABLE t; '` as `Exact`) and panicked on an executable
  comment's tail whose third byte is inside a character. The first fix judged "inside a token"
  from the lexer state after re-lexing the token's first bytes. The review found three holes
  in that: a start inside an opener or after a string prefix (that state is "plain comment"
  or "clean"), a start between a backslash and the quote it escapes (the mark landed on a
  one-byte token and the string's rest was split as code), and `COPY` rows reached from the
  `COPY` line. Deciding from the document's tokens and `COPY` ranges closes all three. The
  splitter's own mark on a carried state stays, for callers of `split_from`. The re-review
  then found that the `COPY` rule looked only at the item just before the selection; with
  another item on the `COPY` line the rows were sent again. The rule now asks the list of
  every `COPY`, `split_selection` checks both ways that its own `COPY` data and the
  document's are the same, and the generators hold such a line.
- Cost: statements selected inside a function body (a dollar-quoted string) do not run in
  M3a. Listed for the PO.
- **A selection is its own script.** A psql `\if` before it is not part of it: a statement
  selected inside an `\if false` branch runs as selected, while a caret on that statement is
  refused (`ClientCommand`), because psql may skip the branch. Selecting text is the explicit
  request to run that text (review M6). Listed for the PO beside the dollar-body note.
- `AsOne` (and `SmartExpand` with no boundary inside) returns one `Verbatim` unit only when
  no token is cut at either end and the text, read alone from a clean state, is certain and
  holds no client command and no `COPY` data. The part of a comment at either end is left
  out, and a text that overlaps the document's `COPY` rows is refused whatever it reads as
  alone: sent from inside a block comment, the comment's quote paired with a quote in the
  rows and the whole selection read as one certain statement (found by the properties). No M3a Run action chooses it, and running a
  `Verbatim` unit is M3b. The catalogue sends such a selection as written even when it ends
  inside a string; M3a answers `Ambiguous` (register entry SEL-07).
- Several carets each resolve as a statement: under Ask there is no chooser per caret. Units
  from several carets or selections that overlap without being the same range (a selection
  inside the statement another caret is in) are refused (`AmbiguityReason::OverlappingUnits`):
  running both would run part of a statement twice.

**Goldens (clean room, ALG-TEST-01).** `sql-testkit/corpus/datagrip/{caret,sel,choose}.toml`
hold the CARET, SEL and CHOOSE families of `datagrip/13` §3.6–§3.8. The scenarios and the
fixture SQL are that Hacivat-authored document's. Every expectation names its source in
Hacivat's own specs (`alg/10` SQL-SPLIT-04, `alg/20` HV-EXEC-01/-02, the M3a rulings); the
document's "Expected hint" column is not used and `datagrip_observed` is empty.

| Family | IDs | Run | Run without a CST | Skipped |
|---|---|---|---|---|
| CARET | 30 | 18 | 9 (03, 05, 06, 25–30) | 3: CARET-19, -20 (`dialect:mssql`), CARET-22 (`dialect:oracle`) |
| SEL | 8 | 7 | 0 | 1: SEL-06 (`dialect:mssql`) |
| CHOOSE | 7 | 3 | 0 | 4: CHOOSE-04 (`dialect:mssql`), CHOOSE-05, -06, -07 (`app-ui`) |

A case that needs a parse tree (`requires = ["cst"]`) keeps M5's answer in `expect` and runs
`expect_without_cst`, where "smallest subquery or statement" falls back to the statement.
Every expected unit is written out as text, taken once from the fixture or the case's input:
no case names a statement by its index in the split under test. The meta-test pins the ID lists to the document's numbering, requires every ID exactly once, and
asserts the skip list and the counts above exactly; a requirement the runner lacks is an error
unless the case is skipped with that reason. `divergences.toml` records CHOOSE-01 (no subquery
candidate: correlated) and SEL-07.

**Tests.** `crates/sql/sql-split/tests/{d011_goldens,exec_range,exec_range_props,pg_differential}.rs`:
- the properties run over generated text in every dialect and client flavor, at a caret, a
  selection and two carets, under every policy: execution returns only certain statements,
  each of which read alone is exactly itself; where Run runs or asks the preview is the same
  answer, and what the preview shows as exact, execution runs; a single selection's answer
  equals what `split_selection` and `selection_cuts` say by hand; every answer is well formed;
- the document-level property (`in_step`, in `split_props` and `exec_range_props`): a
  certain unit of a selection never overlaps `COPY` rows, never overlaps a document token an
  edge of the selection is inside, and starts and ends on a token boundary of the
  whole-document lexing (or lies wholly inside one `--` comment). Re-splitting a unit alone
  cannot see these: `DROP TABLE t` alone is itself. The generators hold an escape cut in two
  (`E'a\'`, `'a\'`) and a quoted `*/` inside `/*! … */`, `/*+ … */` and `/*M! … */`;
- the PostgreSQL differential: the ranges a whole-script Run sends equal libpg_query's
  statement ranges on six fixtures and six scripts, and the two fixtures the grammar cannot
  read (psql commands, `COPY` data) are pinned to their answers.

## m3a-f1 — a COPY is the splitter's answer (follow-up F1-a)

**Choice.** Whether a unit is a `COPY` that streams rows to or from the client is decided once,
by the splitter, on the tokens it splits by. `StmtHead::copy_stream` is set when the
statement's first significant token is the word `COPY` and a later word is `STDIN` or
`STDOUT` (any depth, any client flavor). A unit of such a statement is `UnitKind::CopyIn`
when psql's inline rows follow it, else the new `UnitKind::CopyStream` (`COPY … TO STDOUT`,
`COPY … FROM STDIN` without a terminator, a selection without its rows, a flavor without
inline rows). The Run preview (`hacivat-sql-editor`'s `sent`) and Run
(`hacivat_exec::units::refusal`) both refuse by kind only, so they cannot disagree on a
`COPY`.

**Rejected.** The two text-based copies of one rule (Task 33 restated `hacivat-exec`'s
`copy_stream` in the preview, reading "the first *word* is `COPY`"). They skipped a leading
non-word token, so `)COPY t FROM STDIN` read as a `COPY` to them and as a plain statement to
the splitter, and the preview disagreed with `resolve` (the proptest seed `cc 53f69ccc…`).

**`)COPY` and its "rows".** PostgreSQL parses a statement from its first token: `)COPY t FROM
STDIN` is a syntax error at `)` (SQLSTATE 42601; pinned on every PostgreSQL target by
`hacivat-exec`'s `docker_m3a_f1` test, which also shows the session takes the next statement
normally, so it never entered `COPY` mode). The lines after it are therefore never read as
`COPY` data by the server: they are SQL, as the splitter lexes them. In the seed they also
never run at a caret or in a whole script, because the `\.` line is a client command.

**Fails closed.** The PostgreSQL oracle's pieces carry no splitter kind, so an ambiguous region
that holds the word `COPY` anywhere is never settled by the oracle (it stays uncertain, as
before Task 23's oracle).

**Tests.** `sql-split/tests/exec_range.rs` (`a_copy_is_recognised_by_the_statement_head`,
`a_copy_selected_without_its_rows_is_never_a_plain_statement`), `hacivat-exec/tests/units.rs`
(`run_reads_a_copy_from_the_splitters_kind`, `an_oracle_region_holding_copy_stays_uncertain`),
`hacivat-sql-editor/tests/preview_truth.rs` (`the_preview_reads_copy_from_the_splitter` and
the committed seed).

## m3b-t01a — SQL seams

**Two new Core-layer crates under `crates/sql`.** `sql-ident` (identifiers, qualified names,
ASCII-only folding) and `sql-safety` (the classifier) obey every M3a rule: no runtime, no
`hacivat-l10n`, no core crate, `salsa` and `sql-sema` still forbidden. `sql-safety` is pure and
synchronous; its thread pool is `hacivat-exec::classify_pool` (R-M3-53), so the SQL layer stays
runtime-free.

**`BackendOutsideOwner` gains a `sqlparser` arm.** Each parser backend has exactly one owner
that may depend on it directly, by any edge kind: `pg_query` in `sql-oracle-pg`, `sqlparser`
(`=0.64.0`, Apache-2.0, no default features; `std`, `visitor`, `recursive-protection`) in
`sql-safety` (R-M3-54). Neither crate re-exports a backend type through `sql-safety`: the
`backend` and `visit` modules are private, and `tests/stubs.rs` checks the public lines of
`src/` for `sqlparser::` and `pg_query::`. The one `pg_query` re-export is
`sql_oracle_pg::tree::pb`, the protobuf tree the PG backend walks (Task 9).

**Classification is tied to the bytes sent (P-M3b-2).** `SafetyReport` carries
`classified_len` and `classified_hash` (XXH3-64 of the exact text classified,
`sql_safety::classified_hash`). `SafetyReport::matches_text` is the comparison the exec gate
makes before it sends a unit, so `hacivat-exec` needs no hashing crate of its own.

**Fail-closed stubs (F31).** Until their tasks land: `classify` answers `Unknown` with
`request_effects = UNKNOWN` and the pre-decision `[NotImplemented]` (Task 14); `decide` answers
`Deny([NotImplemented])` (Task 4); `gate_class` answers `Unknown` and `opens_write` `true`
(Task 4); `denied_function_names` is empty and `set_allowed` `false` (Task 3);
`sql_ident::{fold, ident_eq, parse_qualified}` change nothing, compare bytes and recognise
nothing (Task 3); `sql_split::scan_params` reports every `:`, `$` and `?` byte as an unbindable
placeholder and rewrites nothing, so Run refuses the unit (Task 6); `hv_norm1` hashes the raw
text at version 0 (Task 6); `StreamSplitter` yields one `Ambiguous` at offset 0 (Task 5);
`Lexed::checkpoint_before` answers `(0, Initial)`, a full relex (Task 5);
`sql_oracle_pg::tree::{parse_tree, scan_tokens, fingerprint}` answer `Err` (Task 9). Nothing
here is wired into Run (R-M3-52).

**`fuzz/` is its own workspace (P-M3b-15).** It is excluded from the root workspace, copies
the root's `third_party` `[patch.crates-io]` entries with `../` paths (so `pg_query` is the
vendored copy) and checks in a `Cargo.lock` generated from the root lock. Cargo warns that the
`mysql_async` and `tokio-postgres` patches are unused; they stay so a later edge cannot reach the
registry crates. `libfuzzer-sys`'s NCSA licence is recorded in Task 20's section.

**xtask stubs.** `gen-pg-visitor` (Task 9), `gen-fn-catalog` (offline `--check` only, Task 3;
the catalog read is a `hacivat-exec` Docker test, P-M3b-13), `bench-sql` and `fetch-pg-regress`
(Task 20) exit 2 and name the task that fills them.

**Data placeholders.** `sql-safety/data/` holds the TOML tables later tasks fill, each with an
`# owner: Task NN` header: `<engine>/{allow,deny,operators,types,set_allow}.toml` and
`functions.digest` (Task 3), `profiles.toml` (Task 4), `pg/statements.toml` (Task 9),
`{mysql,sqlite}/statements.toml` and `mysql/known_gaps.toml` (Task 10).

**Fix round 1.**
- **Empty is not Read.** `EffectSet::default()` is the empty set, and every mapping treats an
  empty set as `Unknown`. `GateClass` defaults to `Unknown`.
- **Fail-closed stub.** `sql_safety::sqlite_read_pragmas` is empty until Task 10. Task 18
  compares it with the guard's `SQLITE_READ_PRAGMAS`.
- **Two more ownership checks** for `BackendOutsideOwner`:
  - a declared backend dependency counts too, so an optional one that no feature turns on is
    still seen;
  - only `sql-oracle-pg` and `sql-safety` may name `sql_oracle_pg::tree`. This is a source scan
    in `check-deps`.
- **`sql-testkit`** no longer depends on `sql-safety`. It reaches `sql-oracle-pg` only through
  its `differential` feature, so the lexer's plain test loop builds neither libpg_query nor
  `sqlparser`.

## m3b-t04 — the policy table

Task 4 fills `sql-safety`'s report model: `class_of`, `merge`, `gate_class`, `opens_write`,
`decide` and the Fluent ids of every class, reason and destructive pattern.

**Every effect bit maps on purpose.** Three tables name each of the 20 effect bits once:
`COARSE_OF_BIT` (the `CoarseClass`), `GATE_OF_BIT` (the `GateClass`) and the policy's
`GROUP_OF_BIT` (a group in `data/profiles.toml`). A unit test checks each table against
`EffectSet::all()`, so a new bit without a row fails the build's tests. At run time these sets
are `Unknown`, never `Read`:

- the empty set;
- a set with a bit no table names;
- a `RiskFlags` value with a bit that has no name.

`GateClass` is declared in its total order (P-M3b-4) and derives `Ord`. Destructive reasons
raise the gate class whatever the effects say: an unsafe-DML kind raises it to at least
`DmlUnsafe`, a destructive-DDL kind to at least `DdlDestructive`. `DYNAMIC_SQL` is `Unknown`.
`ROUTINE_CALL` is `Routine` for the gate but `Unknown` for the coarse class, because a routine
body is opaque (`alg/13` test 13).

**`decide`.** The rule for each (effect group, profile) pair lives in `data/profiles.toml`,
compiled in. The loader rejects unknown and missing keys. If the table doesn't load, `decide`
denies everything. The strictest rule wins: Deny, then Confirm, then Allow. Fail-closed
choices:

- `decide` judges request-level fields as well as statements. A statement's `class` field, the
  request's class, request-level effect bits the statements don't show, and request-level flags
  each count, so a producer that leaves a field behind can't lower the decision.
- A reason is evidence of its effect group. `ExternalFile` on a bare `READ` still denies on
  read-only.
- Doubt codes deny in every profile, `Admin` included. These are the lexing, splitting,
  parsing and limit codes, `LexModeChange` and `PrivilegeChange`.
- `FnUnknown`, `OperatorUnknown`, `CastUnknown`, `DynamicSql` and `ProceduralCode` count as
  `unknown`.
- An executable comment (D-058) or a denylisted function denies on the read-only profiles and
  for an AI agent.
- A `SET_VAR` hint denies unless its variable is harmless.
- `MAX_EXECUTION_TIME` denies when its limit is 0 or can't be read.

**Harmless `SET` (P-M3b-6).** `decide` has no dialect argument, so `SafetyReport` gains
`dialect: Option<DialectId>`. Task 14's `classify` sets it, and `pre_decided` leaves it `None`.
A `SET` statement counts as a read in every profile only when all of these hold:

- its effects are exactly `SESSION_STATE`;
- it has no flag, no destructive pattern and a class no higher than `Session`;
- it has at least one reason, and every reason is `SessionState` with the variable name as its
  argument;
- `set_allowed(dialect, name)` accepts every one of those names.

Backends (Tasks 9 and 10) must emit one `SessionState` reason per `SET` target. Any other `SET`
names `SetNotAllowlisted` for each refused target. A `SET` with no dialect or no named target
is never harmless.

**Differences from `alg/13` SQL-SAFE-04's table.**

- Unbounded `UPDATE`/`DELETE` also confirms under `ddl`. That profile is a superset of
  `read_write_dml`, so it must not ask less.
- `CALL` has its own `routine` row with the `unknown` rules.
- An AI agent is refused `EXPLAIN ANALYZE` on both read-only profiles. Per the plan's test
  name, a human alone may run it there.
- A harmless `SET` is a read in every profile, not only the read-only ones, so the allowlist
  never makes a read-only profile more permissive than a writable one.
- `LEX_MODE_CHANGE` denies in every profile, as `alg/13` asks of Karagöz, because `decide` is
  shared.
- The spec's info-disclosure and resource-heavy rows have no bits in M3b. Task 3's function
  data denies the functions behind them.

**New reason codes.** The policy names every refusal, so `ReasonCode` gains nine codes for
groups that had none: `WriteData`, `SchemaChange`, `TableLock`, `AdvisoryLock`, `ExternalIo`,
`PreparedStatement`, `Notify`, `EffectsUnknown` and `Destructive`.

**Fluent.** The `safety-*` ids live in `hacivat-core.ftl` under the block `## m3b-t04-safety`.
The `safety` prefix is added to three lists, a named exception (P-M3b-T4-1):

- xtask's domain prefixes;
- its id-module list;
- `hacivat-l10n-schema::is_valid_id`.

Function, operator and cast names reach the messages as `$name`, and table names as `$table`.
Fluent isolates both with FSI/PDI. The Turkish messages carry `# Review: PO`.

**Visibility.** `effects::class_of` and `report::merge` are `pub`, not `pub(crate)` as the plan
says. The plan's integration tests call them, and P-M3b-16's report builders must merge the
way `classify` does.

**Fix round 1 (controller rulings on the review).** These rulings override the earlier text of
this section where they differ.

- **The gate reads what `decide` reads (I1).** `gate_class` used to read only the effect bits.
  It now takes the most critical of seven sources:
  - the effect bits;
  - the statement's `class` field (`Unknown` means `Unknown`);
  - its kind;
  - its risk flags;
  - its reasons, through the policy's own `reason_rule` (`reason_gate_floor`);
  - its destructive patterns;
  - an unnamed flag bit, which means `Unknown`.

  The property test `gate_and_decide_agree` holds this invariant: a gate class of `Read` implies
  `decide(ReadOnlyPlusExplain, Human)` allows. `ReadOnly` differs from it only in refusing
  `EXPLAIN ANALYZE` of a read. The gate calls that statement `Read`, because R-M3-55 says
  "class of x", so the invariant is stated against the human read-only profile (R-M3-56).
  Conversely, whatever a read-only profile allows, the gate calls `Read` or `Session`.
- **Flags (I2).** `FLAG_RULES` covers `RiskFlags::all()` and has a coverage test. Each flag's
  meaning:
  - `VOLATILE_FN` means `Unknown` (`alg/13` S-04). Producers set it only for a volatile
    function that no allowlist covers.
  - `LEX_MODE_CHANGE` and `DYNAMIC_SQL` mean `Unknown` for the gate.
  - `EXEC_COMMENT` means `Unknown` for the gate, so production confirms it. `decide` still
    denies it only on the read-only profiles and for an AI agent.
  - `EXPLAIN_ANALYZE` takes the class of the inner statement.
- **Harmless `SET` (I3).** A harmless `SET` is a read only on the two read-only profiles, and
  only for `Human`. Everywhere else a `SET` follows `alg/13`'s Session row: Deny in
  `read_write_dml` and `ddl`, Confirm in `admin`. This reverses the "read in every profile"
  rule above.
- **`EXPLAIN ANALYZE` for AI (I4).** An AI agent on `ReadOnlyPlusExplain` may run
  `EXPLAIN ANALYZE` of a read (`alg/13` :331).
- **Kinds are floors (I5).** `kind_rule` is an exhaustive match. It sets a minimum class for
  each kind that has one; `SELECT`, `VALUES`, `TABLE`, `SHOW`, `COPY`, `PRAGMA`, `Other` and
  plain `EXPLAIN` are left to their effects.

  | Kinds | Minimum class |
  |---|---|
  | `SET` | Session, judged through the harmless list even without its bit |
  | `INSERT`, `UPDATE`, `DELETE`, `MERGE` | Dml |
  | `DROP`, `TRUNCATE` | DdlDestructive |
  | `CREATE…`, `ALTER` | DdlOther |
  | `GRANT`, `REVOKE` | Admin |
  | `CALL`, `DO` | Routine |
  | `EXECUTE` | Unknown |
  | `BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT` | Tx |
  | `LOCK` | Lock |
  | `USE`, `LISTEN`, `PREPARE`, `NOTIFY` | Session |
  | `EXPLAIN ANALYZE` | the inner statement's class, or Unknown |
- **`SET` targets.** `StmtReport` gains `set_targets: Vec<SetTarget { name, value }>`, filled by
  the backends for `SET`/`RESET`. The harmless check passes each target's value to the list,
  for P-M3b-T3-1 (`search_path` is harmless only when `pg_catalog` is absent or first). A
  `None` value is fail-closed. `decide` calls Task 3's `set_allowed(d, name, value)` directly
  since Task 3 merged (`search_path_value_reaches_the_list`). A `SET` that names no target is
  refused with `SessionState`, so every `SetNotAllowlisted` names its variable.
- **Minor fixes (M1–M5).**
  - **M1:** `Decision.class` rises with reason evidence.
  - **M2:** the `PRIVILEGE` bit names the new `PrivilegeGrant`, confirmed under `admin`.
    `PrivilegeChange` is only a role or authorization switch and denies everywhere.
  - **M3:** a bare `Destructive` reason is destructive-DDL evidence.
  - **M5:** `merge` adds `UNKNOWN` to the request's effects for an empty or unnamed statement.
- **Not implemented, recorded (M6–M7).**
  - **M6:** the `MAX_EXECUTION_TIME` ceiling (`alg/13` SAFE-04 step 3). `decide` has no
    ceilings input and the plan names none, so only 0 or an unreadable limit denies.
  - **M7, for M8:** an AI agent's unbounded `UPDATE`/`DELETE` follows `alg/13` step 5 here
    (Deny). The spec's L9 makes it an approvable critical class; M8's broker must reconcile
    the two.
- **The class field's gate floor (D1).** A statement's `class` field floors the gate class at
  the least gate class of its coarse class (Write → Lock, Unknown → Routine). The backends
  write `class_of(effects)`, which is `Unknown` for `ROUTINE_CALL`, so an Unknown floor would
  turn every `CALL` into `Unknown` against P-M3b-4.
  - **The price:** the gate is not monotone in the class *label* from Ddl or Admin up to
    Unknown. A READ statement labelled Ddl or Admin is `DdlOther` or `Admin`, but labelled
    Unknown it is only `Routine`. It is still monotone in *evidence*: adding bits, reasons,
    flags or destructive kinds never lowers it.
  - **No R-M3-55 decision changes because of it.** `DdlOther`, `Admin`, `Routine` and
    `Unknown` are all Confirm on production, Run elsewhere and Deny on read-only. The only
    row keyed on DDL is MySQL's implicit commit, which an Unknown statement doesn't get either.
    In the `EXPLAIN ANALYZE` row, `Routine` is stricter: it is refused, where an `Unknown`
    inner gets the D-062 wrap.
  - **Producer contract (T9, T10, T14):** every Unknown outcome must also set
    `EffectSet::UNKNOWN` or an unknown-group or doubt reason, never only the class field.
- **`SET` reasons and targets agree (re-review R1-M3).** A `SET` is harmless only when its
  `SessionState` reason arguments and its `set_targets` names are the same list, compared
  byte-exact as multisets. A reason naming a variable outside the targets is refused by that
  name.
- **The group-skip rule (re-review R1-M1).** `decide` skips a class field's group when a group
  of the same coarse class was already applied. A unit test pins what makes that safe: every
  group of a coarse class is at least as strict as that class's own group in every profile.

## m3b-t06 — parameters

**What the scan reads.** `sql_split::scan_params` lexes the unit on its own, from the initial
state, as the server receives it. Only code tokens are read. A marker inside a string, quoted
identifier, comment or dollar body is never a placeholder, and after a `COPY`'s first `;` the
rest of a unit is data. An executable comment's body is code to the server, so it is lexed and
read too. The lexer does not recognise psql meta-commands: `\set x :y` reaches the scan as an
unknown `\` followed by code. So a unit that holds any token the lexer could not read cleanly
binds nothing (fix round 1, below).

**One rule for refusal: if one placeholder cannot bind, none does.** When any placeholder
cannot bind, every placeholder is `bindable: false` and `rewritten` is `None`. Run then
refuses the unit and sends nothing. `rewritten` is `Some` exactly when there is at least one
placeholder and every one binds. Every refusal carries a diagnostic:

- `ParamInDdl`: a placeholder in `CREATE`/`ALTER`/`DROP`, unless `in_ddl` is on. The head is
  the caller's `StmtHead` or the unit's own first word, so a stale head cannot unlock DDL. An
  `EXPLAIN` counts as DDL when it holds any `CREATE`/`ALTER`/`DROP` keyword; all three are
  reserved, so none of them can be a bare column name.
- `MixedStyles`: `$n`, `?` and `:name` mixed in one unit. This includes SQLite's `?NNN` beside
  names, because the rewrite would renumber them.
- `TooManyBinds`: more than 65,535 driver slots. That is distinct names on PostgreSQL,
  occurrences elsewhere, and the largest `$n`/`?NNN` number.
- `TemplateVariable`: any template variable in the unit.
- `Unbindable`: a variant this task added, for a marker the server may read as a parameter
  that Hacivat cannot bind:
  - `?` or `:name` inside a MySQL/MariaDB executable comment;
  - `?` under `CALL`/`EXEC`/`EXECUTE` (ODBC escape semantics);
  - psql `:"name"`, which substitutes an identifier: binding it would turn `WHERE :"col" = 1`
    into `WHERE $1 = 1`;
  - psql `:{?name}`;
  - `$0`, `$n` with trailing junk, and any positional with a leading zero (`$01`, `?01`). The
    server reads `$01` as `$1`, so two spellings would become two plan names for one slot;
  - any placeholder in a unit that holds a token the lexer could not read cleanly. That is an
    unknown character (psql's `\`, as in `\set x :y`, `:a\gset` and `\g`), an error or junk, or
    an unterminated string, quoted identifier, dollar body or comment (`SELECT :a '`,
    `SELECT :a /* …`). The diagnostic points at that token. This also covers a comment end
    the server decides (`AmbiguousCommentEnd`), which is flagged as an error;
  - with `pg_question_marks` on, a PostgreSQL `?` whose reading is ambiguous:
    - a `?` beside another `?`-operator (`WHERE ? ? 'k'`, `(j) ? ?`, `j ? ?|…`);
    - a `?` directly followed by a literal (`first ? 'k'`, where `first` may be a column);
    - a known `?`-operator in operand position (`a = ?-1`);
  - an SQLite `:name` while `:name` parameters are off, or a malformed SQLite variable
    (SQLite binds an unbound variable as NULL, so sending it would be silent);
  - any placeholder of a `COPY` or `PREPARE`;
  - any placeholder in a unit of several statements (a verbatim "Run as one" unit). No
    protocol binds values across statements, and the head describes only the first. A `;`
    inside a MySQL/MariaDB executable comment counts, because the server runs it;
  - a PostgreSQL operator that only contains `?` (`=?`) while `?` binds;
  - every `:`, `$`, `?`, `@` byte in a dialect with no lexer profile;
  - a placeholder whose replacement would merge with a neighbouring byte. `:'v'2.5` would
    become `$12.5`, `SELECT?` would become `SELECT$1` (one identifier), and SQLite `$a(x)1`
    would become `?1`. The replacement must have no identifier byte on either side.

**Not placeholders.** These are left as written and never reported: `::` casts, `:=`, the
jsonb and geometric `?` operators, `@`/`@@` variables, and PostgreSQL `:name` inside `[…]`
(array slices). The DBeaver prefix guard also rules out a `:` right after an identifier
character, `$`, `:`, `?`, `\`, `/` or `[`. The server reads all of these the same way whether
or not Hacivat binds. With `pg_question_marks` off, a PostgreSQL `?` in operand position gets
only a `QuestionMarkOff` hint (D-08). `ARRAY[:a]` (inside brackets) and `x/:a` (prefix guard)
get no diagnostic; the server reports a syntax error at `:`. A hint for them needs a new
`ParamDiag` variant and its UI, so it is noted for Task 24.

**Deterministic rewrite.** Only placeholder ranges change, and `edits` lists each one. On
PostgreSQL, `:name` and `:'name'` become `$k` by first occurrence, and a repeated name reuses
its `$k`. With `pg_question_marks` on, `?` becomes `$k` in text order. `$n` is unchanged. On
MySQL, MariaDB, SQLite and the generic dialect, `:name` (and SQLite's `@name`, `$name`,
`#name`) becomes `?`, with one slot per occurrence. `?` and `?NNN` are unchanged. Plan names
are the `:name`'s name (SQLite's other sigils kept: `:a`, `@a` and `$a` are three
parameters), `?k` for the k-th `?`, and a positional placeholder's own text (`$2`, `?2`).
Names compare by bytes; Turkish `İ`/`ı` never fold. The rewritten text is what is classified
and hashed (P-M3b-2). The properties check that every byte outside an edit is unchanged, that
every token outside a placeholder lexes the same, that each replacement lexes as one
placeholder token, and that rescanning the output finds the same placeholders.

**Templates (D-053).** `${var}`, `%var%` and `<var>` are detected in code only; nothing is ever
substituted. `${` is a template anywhere in code. `%var%` and `<var>` also read as arithmetic
and comparisons (`10%n%3`, `a<b>c`), so they count only in operand position. That means after
the start, an operator, `(`, `,`, `[` or an expression keyword, and before an operator,
closer, `,`, `;` or keyword. Text inside strings is data: Hacivat has no "substitute inside
strings" switch (PARAM-05/06).

**`HvNorm1` (version 1).** The text is lexed from the initial state. Whitespace and comments
are dropped. Literals, `COPY` data and parameters become `?`. A MySQL charset introducer
joins its string: always when the two touch, and across whitespace only when it names a known
character set (`_binary X'ff'`), because `_x 'a'` is a column and its alias. A psql
meta-command keeps its `\name`, and every argument up to the end of the line becomes one `?`,
so no secret survives: `\connect db u password=…` → `\connect ?`. A parenthesised list of `?`
only becomes `(?…)`, and mixed lists keep
their shape. A hint becomes `/*+…*/`. An executable comment keeps its opener and its body is
normalised as code. Keywords from the lexer's table are ASCII-uppercased (P-M3b-18).
Identifiers and quoted text stay byte-exact, with no case folding and no NFC. Tokens are
joined by single spaces, with none after `(`/`[`/`.` and none before `)`/`]`/`,`/`.`/`;`.
`hash` is XXH3-64 of the normalised text. Every keyword is uppercased, including unreserved
ones such as `NAME`, so `update` and `UPDATE` agree. alg/10's sample text `AND name = ?`
therefore reads `AND NAME = ?`.

**Corpus.** `sql-testkit/corpus/datagrip/param.toml` holds PARAM rows 01–06, 12–15 and 18–20
for PostgreSQL and MySQL, with each D-053/D-08 divergence recorded. The SQL Server row 12 runs
on both of those dialects. Because `sql-split` has no TOML dependency, the test reads the file
with a small line reader, and the file keeps to that subset.

## m3b-t03 — identifiers and function data

**Names resolve as the server resolves them, ASCII only.** `sql_ident::fold` lowers unquoted
ASCII letters on PostgreSQL (and the PG-wire dialects) and truncates every identifier to 63
bytes at a character boundary, as `downcase_truncate_identifier` does; MySQL, MariaDB and SQLite
keep the text and `ident_eq` compares it ASCII-case-insensitively. No locale casing anywhere:
`PG_READ_FİLE` folds to `pg_read_fİle`, never to `pg_read_file`. `parse_qualified` splits on
dots outside quotes (`"…"` on PostgreSQL, backticks on MySQL/MariaDB, `"…"`/backticks/`[…]` on
SQLite; doubled quotes undone; SQL whitespace (ASCII space, tab, newline, carriage return, form
feed) around a dot allowed). A Unicode space (U+00A0, U+2000–U+200A, U+3000, …) is an identifier
character to the servers' lexers, so it stays part of the name it touches:
`"\u{a0}pg_catalog.lower"` names schema `"\u{a0}pg_catalog"`, never `pg_catalog`. Vertical tab
is whitespace only on PostgreSQL 16+, so it is refused. Anything else (an empty part,
an unterminated quote, an operator character, PostgreSQL's `U&"…"`) is `None`, which callers
treat as unknown. Rejected: Unicode case folding (the Turkish-i lookalike would match a denied or
allowed builtin).

**`ident_eq` is not a server-equivalence test for MySQL tables or non-ASCII columns.** It is
ASCII-case-insensitive for every MySQL/MariaDB/SQLite identifier. It does not model MySQL's
`lower_case_table_names` (table names may compare by bytes) or its Unicode (`general_ci`)
column-name comparison (`É` and `é` are one column to MySQL, two here). Callers that must match
a table exactly (the type-to-confirm name, destructive-target matching: Task 15) compare bytes
themselves.

**`lookup_fn`: deny by bare name first, allow only exact builtins.**
- A call whose last name part, ASCII-lowercased, is on the dialect's `deny.toml` is
  `Denied(group)` in any schema, quoting or server version (`alg/13` SAFE-03 step 2: a user
  function named like a denied builtin is denied too). Dialects without data (Generic, SQL
  Server, ClickHouse, Oracle) read every engine's deny list; PG-wire servers read PostgreSQL's.
- A call is `Allowed` only when it names a builtin the way the server would resolve it:
  unqualified, or `pg_catalog.`-qualified on PostgreSQL, folded; on MySQL, MariaDB and SQLite
  unqualified and unquoted, ASCII-case-insensitively. The row must be `reviewed = true` and its
  version gate (`since` inclusive, `until` exclusive) must hold. Everything else is `Unknown`,
  including every call on a dialect without reviewed data. Arity is not read (every overload of
  a name shares its row).
- **Nothing is proven below the probe floor (fix round 1, I2).** Each engine's `allow.toml`
  records `probe_floor`, the oldest server the catalog read saw: PostgreSQL `14` (builtins change
  only in major releases), MySQL and MariaDB the exact `VERSION()` of their oldest line (they add
  builtins in patch releases), SQLite the bundled library's version (the app always runs it). A
  row without `since` holds only from the floor on. A known older server, or no floor at all,
  answers `Unknown`, because a builtin missing there could be a user function of the same name:
  MySQL resolves an unqualified non-builtin to a stored function of the default database, and
  PostgreSQL resolves it through `search_path`. An unknown version holds a row only when it holds
  on every server from the floor on (its `since` is at or below the floor, and it has no
  `until`). Hand `since` gates below the floor come from the engines' change histories:
  - MySQL: every row `8.0.11` (8.0 GA), except
    - `FORMAT_BYTES`, `FORMAT_PICO_TIME`, `PS_CURRENT_THREAD_ID`, `PS_THREAD_ID`: `8.0.16`;
    - `JSON_OVERLAPS`, `JSON_SCHEMA_VALID`, `JSON_SCHEMA_VALIDATION_REPORT`: `8.0.17`;
    - `JSON_VALUE`: `8.0.21`;
    - `ST_FRECHETDISTANCE`, `ST_HAUSDORFFDISTANCE`: `8.0.23`;
    - `ST_COLLECT`, `ST_LINEINTERPOLATEPOINT`, `ST_LINEINTERPOLATEPOINTS`, `ST_POINTATDISTANCE`:
      `8.0.24`;
    - `ST_LATITUDE`, `ST_LONGITUDE`: `8.0.12`;
    - `ST_TRANSFORM`: `8.0.13`.

    Rows whose first release is uncertain keep no hand gate and hold only from the floor:
    `STATEMENT_DIGEST`, `STATEMENT_DIGEST_TEXT`, `GEOMCOLLECTION`.
  - MariaDB: every row `10.6.0`, except `ADD_MONTHS`, `ROWNUM`, `SYS_GUID` and `TO_CHAR`
    (`10.6.1`). Rows the probe gated on a later line (`10.11`, `11.4`) keep that gate.
  - PostgreSQL: none (below 14 is below the support floor; such a server, accepted with
    `allow_unsupported_server`, gets `Unknown` for every builtin).
- `operator_known` and `cast_known` count only reviewed rows with no version gate. Word operators
  (`LIKE`, `DIV`, `BETWEEN`, `IS`, …) match ASCII-case-insensitively; symbols match exactly.
  Casts drop type modifiers and (PostgreSQL) array brackets, collapse whitespace, lower ASCII
  letters and strip a `pg_catalog.` qualifier; another qualifier is unknown. PostgreSQL's interval
  field qualifiers (`interval day to second`) read as `interval`, the quoted `"char"` as `char`;
  MySQL's `CHAR(n) CHARACTER SET x` reads as `char`. Not covered, so unknown: MySQL's `… ARRAY`
  (multi-valued index casts) and every other quoted or qualified type name.
- **`set_allowed(d, name, value)`** is P-M3b-6's list exactly (`data/<engine>/set_allow.toml`),
  whole names ASCII case-insensitive; SQLite and every other dialect allow no `SET`. MariaDB has
  no `max_execution_time` (it calls it `max_statement_time`); the PO list keeps the MySQL name, so
  that `SET` is simply refused there (the safe side). **PostgreSQL `search_path` is harmless only
  while `pg_catalog` resolves first (P-M3b-T3-1):** `set_allowed` is true only when `value` is
  given and `pg_catalog` is absent from the list (the server then searches it first) or is its
  first schema. `None` and a value that does not parse are refused. The value is the text after
  `TO`/`=`, read as PostgreSQL reads a `search_path` list:
  - an unquoted name folds to ASCII lower case (`PG_CATALOG` is `pg_catalog`);
  - a `"quoted"` name keeps its exact text (`"PG_CATALOG"` is another schema);
  - a `'string'` is one schema with that exact text, as `GUC_LIST_QUOTE` makes it, so
    `'public, pg_catalog'` is one schema named `public, pg_catalog`.

  The other variables ignore `value`. The classifier passes the value of every
  `SET search_path` (Tasks 9 and 14).

**Where the rows come from (P-M3b-13).** `hacivat-exec/tests/gen_fn_catalog.rs`
(`#[ignore = "docker"]`, `HACIVAT_GEN_FN_CATALOG=1`, through the wrapper on `--set full`) reads
`pg_proc`, `pg_operator` and `pg_type` of `pg_catalog` (OID < 16384) on PostgreSQL 14–18 with
read-only queries, `pragma_function_list` of the bundled SQLite, and prepares `SELECT <name>()`
for each name of the hand-curated `data/mysql/manual-index.txt` on MySQL 8.0/8.4 and MariaDB
10.6/10.11/11.4 (prepare only, nothing runs; 1305/1630 mean "no such function"; never
`mysql.help_*`, GPL content). It writes candidates, all `reviewed = false`, and the reasons for
every name it leaves out, to the Docker artifacts; it never edits `data/`. A PostgreSQL function
is a candidate when every overload is a function, aggregate or window function, `IMMUTABLE` or
`STABLE`, in `internal` or `sql`, not `SECURITY DEFINER`, not denied and not one whose text is
SQL or a server path; an operator when no implementation is volatile or denied; a type when it
is a base, domain, enum, range or multirange type outside the array category with a non-volatile
input function. A name on a non-contiguous run of lines, or a candidate on one line and not on
another, is left out.

**Row sources.** `source` names where a row came from:
- `pg_proc@pgNN`, `pg_operator@pgNN` and `pg_type@pgNN`: the PostgreSQL catalog read, on line
  `pg17`, or the newest line the row was read on when it is absent from 17;
- `manual-index`: `data/mysql/manual-index.txt`, probed per line;
- `pragma_function_list`: the bundled SQLite;
- `manual`: a hand row.

The test `every_row_names_its_source_and_review_state` holds this vocabulary per engine.

**The review.** Every candidate row was read by name and flipped by hand. Counts after fix
round 1:
- PostgreSQL: 2,462 function candidates (2,411 reviewed, 51 left unreviewed) plus 21 hand rows;
  74 operators; 80 types plus 20 SQL-standard spellings.
- MySQL: 367 names (360 reviewed).
- MariaDB: 387 names (382 reviewed).
- SQLite: 110 names (107 reviewed).

Operators and cast targets of MySQL, MariaDB and SQLite are hand rows. Left unreviewed (so
`Unknown`, which the read-only profiles deny):
- **Information disclosure.** `alg/13` INFO_DISCLOSURE has no deny group, so the line is drawn
  here (fix round 1, M6). A function is withheld when it reveals one of these about the server
  or about other sessions:
  1. other sessions' activity or connection details;
  2. server settings values, or the server's build or version strings;
  3. the server's network addresses;
  4. server file-system paths or directory contents;
  5. replication peers, slots and subscriptions.

  On that basis the following are withheld:
  - `current_setting`, `pg_show_all_settings`, `pg_config`, `version`;
  - `inet_server_addr`/`_port`;
  - `pg_tablespace_location`, `pg_tablespace_databases`, `pg_relation_filepath`;
  - `pg_stat_get_activity`, `pg_stat_get_backend_*`, `pg_stat_get_progress_info`;
  - `pg_stat_get_wal_senders`, `pg_stat_get_wal_receiver`, `pg_stat_get_subscription`,
    `pg_get_replication_slots`, `pg_stat_get_replication_slot`;
  - MySQL/MariaDB `VERSION`, MySQL `ROLES_GRAPHML` and `ICU_VERSION`.

  Information about the caller's own session, about catalog objects, or aggregate statistics
  stays allowed:
  - `inet_client_addr`, `pg_backend_pid`, `CONNECTION_ID`;
  - `pg_relation_filenode` (a number, not a path), `pg_available_extensions`;
  - `pg_postmaster_start_time`, `pg_conf_load_time`;
  - the per-table and per-database `pg_stat_get_*` counters.

  SQLite's library information (`sqlite_version`, `sqlite_compileoption_*`) describes the
  bundled library on the user's machine, not a server, so it is outside the line.
- hidden reads of a relation named by a value: `table_to_xml`, `table_to_xmlschema`,
  `table_to_xml_and_xmlschema`;
- a type's input run for a type named by value (a domain's CHECK can call a user function):
  `pg_input_is_valid`, `pg_input_error_info`, `domain_in`, `record_in`, `array_in`, `range_in`,
  `multirange_in`, and (fix round 1, I3) the JSON-to-record coercions `json_populate_record`,
  `json_populate_recordset`, `jsonb_populate_record`, `jsonb_populate_recordset`,
  `jsonb_populate_record_valid`, `json_to_record`, `json_to_recordset`, `jsonb_to_record`,
  `jsonb_to_recordset` (they coerce into the record's column types; `populate_domain` runs the
  domain's CHECK);
- the language validators (`fmgr_c_validator` loads a shared library) and their two siblings;
- session state or server files: MySQL/MariaDB `LAST_INSERT_ID` (with an argument it sets the
  session value), MariaDB `BINLOG_GTID_POS`, `DES_ENCRYPT`, `DES_DECRYPT`;
- resource and host callbacks on SQLite: `randomblob`, `zeroblob`, `sqlite_log`;
- MySQL keywords the probe answered with a syntax error but that are no functions in 8.x:
  `CONTAINS`, `PASSWORD`, `SRID`.

**Hand rows, `source = "manual"` (P-M3b-18).** Volatile or stable in `pg_proc`, so the generator
never proposes them, but pure for a read: `random`, `clock_timestamp`, `gen_random_uuid`, `now`,
`timeofday`, `random_normal` (16+), `array_shuffle` and `array_sample` (16+), `uuidv4` and
`uuidv7` (18+); `timezone` (`AT TIME ZONE` parses to it; volatile only because `timetz` reads
today's date); the size reads `pg_relation_size`, `pg_table_size`, `pg_indexes_size`,
`pg_total_relation_size`, `pg_database_size`, `pg_tablespace_size`; `pg_partition_tree` and
`pg_partition_ancestors`; the `TABLESAMPLE` methods `system` and `bernoulli` (their handlers take
`internal`, so SQL cannot call them). The deny lists and the harmless-`SET` lists are hand rows
too: PostgreSQL's covers the `alg/13` catalogue and all 27 names of
`hacivat_guard::PG_NON_TRANSACTIONAL_FUNCTIONS` (Task 18 tests the equality); MySQL/MariaDB
`SLEEP`, `BENCHMARK`, `LOAD_FILE`, the lock functions, the source-position waits, `sys_exec`/
`sys_eval`, and MariaDB's `NEXTVAL`/`SETVAL`/`LASTVAL` (one rule with Task 10's pre-scan,
P-M3b-8); SQLite `load_extension`, `readfile`, `writefile`, `edit`, `fts3_tokenizer` (the
guard's authorizer list).

**Residual risks, covered by the server (L3–L6) and the catalog (M4).**
- An unqualified PostgreSQL call is assumed to reach `pg_catalog` first. Ruling P-M3b-T3-1: a
  read-only session's `SET search_path` is harmless only when `pg_catalog` is absent from the new
  list or first in it, and `set_allowed` checks the value (above). A role's or database's default
  `search_path`, or a connection option, can still list `pg_catalog` after a writable schema. The
  controller has routed a connect-time check of that to Tasks 14 and 18 (review M4).
- A builtin operator symbol is known for all its overloads: a user operator of the same symbol
  over user types is not seen without the catalog.
- Implicit calls (domain CHECKs, column defaults, views) are not assessed without the catalog
  (`alg/13` SAFE-03 step 6).
- Two cast paths bypass `cast_known` (fix round 1, M3):
  - **A user-defined cast from a user type.** `x::text`, where `x`'s type has
    `CREATE CAST … WITH FUNCTION`, calls that function, though the target `text` is a builtin.
  - **A temporary type that shadows a builtin type name.** `pg_temp` is searched before
    `pg_catalog` for types, so a session-created temporary table named `int4` shadows the cast
    target `int4`. Creating one needs a transaction that is not read-only.

  Both are seen only with the catalog (M4); the read-only transaction (L3) still blocks writes.
- MariaDB answered "no such function" for the zero-argument probe of `POINT`, `LINESTRING`,
  `POLYGON` and the `MULTI…` constructors, so those calls are `Unknown` there (too strict, not
  too lax).

**Fail closed.** A data file that does not parse loads as empty (every lookup `Unknown`/`false`;
`compiled_tables_parse` keeps that from shipping); a deny row denies whatever its review state;
a denied name is removed from the allow map whatever `allow.toml` says.

**Freshness (R1-M3).** `data/functions.digest` holds `xxh3-64:<16 hex>`: XXH3-64, seed 0, over
the 21 files of `DIGEST_FILES` in order (`pg`, `mysql` with `manual-index.txt`, `mariadb`,
`sqlite`; each `allow`, `deny`, `operators`, `types`, `set_allow`), each as its path, `\n`, its
byte length, `\n` and its bytes with CRLF read as LF. `sql-safety` and `xtask gen-fn-catalog`
each implement it (xtask takes no SQL crate); xtask's test compares its list with the one
between the `digest-files` markers of `functions/data.rs`, and both crates pin the same known
answer. `cargo xtask gen-fn-catalog --check` exits 1 on a stale digest and `--write` refreshes
it; `cargo test --workspace` runs both freshness tests.

**Tests.** `sql-ident/tests/fold.rs`; `sql-safety/tests/functions.rs` and
`tests/set_allow.rs`; the unit tests of `functions/{mod,data}.rs`; `xtask` `gen_fn_catalog`
tests; the candidate rules of `hacivat-exec/tests/gen_fn_catalog.rs` run without Docker.

## m3b-t10 — interim backend

Task 10 fills the interim classifier backend for MySQL, MariaDB and SQLite (R-M3-54):
`sql-safety/src/backend/{interim,prescan}.rs`, the statement tables
`data/{mysql,sqlite}/statements.toml` (MySQL's serves MariaDB), `data/mysql/known_gaps.toml`, the
SQLite read-pragma list (`pragmas.rs`) and the read corpora `sql-testkit/corpus/read/*.sql`.

**Pipeline per statement range.** Our lexer reads the range first; a lexing error is
`LexError`. Each executable comment is then read at every **version threshold**: with `k`
distinct gate versions there are `k + 1` readings, an unversioned `/*!` is always active, and
an `AmbiguousCommentEnd` comment adds its running reading (the end a server that runs the body
sees). Readings are built by blanking bytes with spaces, so every offset stays the source's.
More than `Limits.max_exec_comment_views` (16) readings is `TooComplex`. On a MySQL connection a
MariaDB `/*M!` comment is read as a view too: MySQL skips it, but a MariaDB server behind a MySQL
profile runs it, and the most dangerous reading wins. Each reading must:

1. lex cleanly again;
2. **agree** with `sqlparser`'s tokenizer: every byte is code, quoted text or trivia for both,
   and every quoted token has the same extent. Spans are mapped from `sqlparser`'s 1-based line
   and character column through `LineIndex`. Code tokens may split differently (`@v` is one
   token for us and for `sqlparser`, `x'0A'` keeps its `x` as code); a string, identifier or
   comment boundary may not differ. A tokenizer error where our lexer read the text is also a
   disagreement. Either is `ParserDisagreement`;
3. hold the same number of statements for both (else `ParserDisagreement`; more than one is
   `MultiStatement`);
4. parse in strict mode: `MySqlDialect` or `SQLiteDialect`, no trailing commas, recursion
   limit 50 (`TooComplex` at the limit). A failure is `ParseError`, except for the narrow
   P-M3b-35 grammars below.

The walk is `sqlparser`'s derived `Visit`. It maps every statement, top-level or nested,
through the table and adds what the tree shows:

- locking clauses and `SELECT … INTO` (variables are `SESSION_STATE`, a table is DDL);
- data-modifying CTEs and `EXPLAIN ANALYZE`;
- every function call, in expressions and in table functions, through Task 3's `lookup_fn`
  (deny list, then the reviewed builtins with their probe floors), then the profile's
  allowlist, then the catalog (volatile is `VOLATILE_FN`, S-04);
- touched objects;
- the SAFE-07 destructive patterns: no `WHERE`, an always-true `WHERE`, `LIMIT` without
  `ORDER BY`, a join without a condition, `DROP`, `TRUNCATE`, and `DROP COLUMN`, a dropped
  constraint or a type change in `ALTER`. The inner statement of a plain `EXPLAIN` adds its
  effects but no destructive pattern (it does not run).

The lexer pre-scan inventory, on each reading:

- a denied `word(` or `"quoted"(` call, by bare name, in any schema or quoting;
- `INTO OUTFILE|DUMPFILE|@var`, `FOR UPDATE|SHARE`, `LOCK IN SHARE MODE`, `:=`, `HANDLER`,
  `XA`, `LOAD DATA|XML` and `RETURNING`;
- the P-M3b-8 MariaDB forms: `NEXT|PREVIOUS VALUE FOR`, `NEXTVAL(`, `LASTVAL(`, `SETVAL(`,
  `seq.nextval`, `FOR SYSTEM_TIME`, `BEGIN NOT ATOMIC`;
- SQLite's `ATTACH`, `DETACH`, `VACUUM [INTO]`, `REINDEX` and `CREATE VIRTUAL TABLE`;
- client commands (`DELIMITER`, `SOURCE`, `SYSTEM`, `\…`, sqlite3's `.commands`) and
  unreadable tokens;
- `SET_VAR` and `MAX_EXECUTION_TIME` hints.

The readings are then **unioned**: effects, flags, reasons, destructive patterns and targets,
with the kind of the most critical reading by `gate_class`. Any doubt code (`reason_rule`
`DenyAlways` or the `unknown` group) adds `UNKNOWN` and makes the class `Unknown`. The class
field never carries an Unknown outcome on its own (the T4 producer contract).

**Grammars of our own.**
- **`SET` (MySQL, MariaDB).** One `SetTarget` and one `SessionState` reason per assignment,
  with the name as written: quotes removed, the `@@session.`/`@@global.` scope stripped, a user
  variable kept as `@v`. The value is kept as written. Targets are de-duplicated by the name's
  byte range, so the two lists always match. When two readings disagree on a value, the value
  is `None`. `GLOBAL`, `PERSIST`, `PERSIST_ONLY`, `@@global.` and `@@persist.` add `ADMIN`.
  `sql_mode`, `character_set_client`, `character_set_connection` and `collation_connection`
  set `LEX_MODE_CHANGE`. The special forms carry no target:
  - `NAMES` and `CHARACTER SET` set `LEX_MODE_CHANGE`;
  - `PASSWORD` and `DEFAULT ROLE` are `PrivilegeGrant`;
  - `ROLE` is `PrivilegeChange`, the only role switch;
  - `TRANSACTION` is `TXN_CONTROL`;
  - `RESOURCE GROUP` is `ADMIN`.

  A `SET` the grammar cannot read is `SESSION_STATE` with an argument-less `SessionState`, never
  harmless.
- **`PRAGMA` (SQLite).** It is a read only without an argument for a `sqlite_read_pragmas()`
  name (the guard's 20), or with one name argument for the six introspection pragmas. Every `=`
  form and every other pragma is `ADMIN` (`AdminControl`). `sqlparser` is not asked: it rejects
  `PRAGMA table_info(t)`.
- **Narrow reads (P-M3b-35).** These apply only where `sqlparser` rejects the form:
  - `SHOW <words> [LIKE … | WHERE …]` with no call, no subquery, no variable and no `:=`;
  - `DESCRIBE <qualified name> [<column or pattern>]`;
  - `EXPLAIN|DESCRIBE [FORMAT = w] <statement>`. Its inner statement is blanked out of the
    reading and classified on its own (depth ≤ 2), and `ANALYZE` never takes this path.

**Choices.**
- **Sleep has no bit.** A denied `Sleep` call adds only `FnDenylisted`, so `SELECT 1 /*!99999 ,
  SLEEP(100) */` is `Read` with `EXEC_COMMENT`, as `alg/13` test 22 says. The gate is `Unknown`
  and read-only denies it. Every other deny group maps to its bits: file I/O is `EXTERNAL_IO`,
  advisory locks `ADVISORY_LOCK`, sequences `SEQUENCE` and dynamic SQL `UNKNOWN`.
- **`EXPLAIN` without `ANALYZE` carries its inner statement's effects** (S-08: an EXPLAIN of a
  non-read is not a read), but no destructive pattern.
- **`SELECT @v := 1` is `Session`.** `alg/13` S-13 allows it on read-only with a flag. M3b has
  no `USER_VAR_ASSIGN` flag, so read-only denies it (known gap).
- **Information disclosure is Unknown** (fix round 1, I3, which replaces the first version's
  `permissive` gap). See the fix-round-1 notes below.
- **`sql_mode=ORACLE` on MariaDB is `UnsupportedDialect`.** No interim parser reads that
  grammar.
- **A lone CR in a reading is `ParserDisagreement`.** `sqlparser` counts only LF as a line
  break, while `LineIndex` also counts a lone CR.
- **Harness entry.** `sql_safety::classify_interim(&str, &ClassifyCtx) -> SafetyReport`
  (`#[doc(hidden)]`, a fenced named exception in `lib.rs`) splits in execution mode, analyses
  each strict range and merges. It is the entry for Task 10's tests and corpus; Task 14's
  `classify` is the production entry. No `sqlparser` type is public:
  `no_sqlparser_type_in_the_interim_public_api` checks every `pub` line of the backend and the
  one re-export.

**Known gaps** (`data/mysql/known_gaps.toml`, each row asserted by
`known_gaps_each_have_a_test`):
- **False denials:** the `MOD` operator, MySQL's `TABLE t`, `CAST … CHARACTER SET`,
  `EXPLAIN EXTENDED`, `EXPLAIN FOR CONNECTION`, `CHECKSUM TABLE`, MariaDB's
  `LIMIT ROWS EXAMINED`, SQLite's table-valued functions (not on Task 3's allowlist),
  `ISNULL`/`NOTNULL`, `AS MATERIALIZED`, `PRAGMA x = name`, and a lone CR.
- **Stricter than `alg/13`:** `LOCK IN SHARE MODE`, `INTO OUTFILE`, `LOAD DATA` and
  `VACUUM INTO` are `Unknown` because `sqlparser` rejects them; the pre-scan still names their
  bits. `SELECT @v := 1` is the S-13 gap above.
- **MariaDB-only forms (P-M3b-8):**
  - Accepted by `sqlparser`: `NEXTVAL`, `SETVAL`, `LASTVAL`, `RETURNING`, and `/*M!` read as a
    plain comment.
  - Rejected by `sqlparser`: `NEXT VALUE FOR`, `PREVIOUS VALUE FOR`, `FOR SYSTEM_TIME`,
    `BEGIN NOT ATOMIC` and `sql_mode=ORACLE`.

  None is `Read` at the gate, and each is denied on read-only.

**Read corpus.** `sql-testkit/corpus/read/{mysql,mariadb,sqlite,pg}.sql` hold 326, 330, 312 and
304 Hacivat-authored reads over the `datagrip/13` §3.1 seed schema. They include
`SHOW`/`DESCRIBE`/`EXPLAIN` and the read `PRAGMA`s. The interim backend calls every MySQL,
MariaDB and SQLite entry `Read`: zero false denials, each allowed on both read-only profiles
and `Read` at the gate. The PostgreSQL file is Task 9's and Task 14's; here each entry is only
checked to be one statement that libpg_query parses.

**Tests.** `sql-safety/tests/interim.rs` (27) and `tests/read_corpus.rs` (4, plus the
`#[ignore = "perf"]` budget test on the perf line); the unit tests of `backend/{interim,
prescan}.rs` and `pragmas.rs`. `tests/data/safe06_interim.toml` holds the offline run A rows
M1–M19 and L1–L9; Task 18 replaces it with the bypass TOML.

**Fix round 1 (review of Task 10).** These rulings override the text above where they differ.

- **Skipped readings (C1).** Every executable comment a server may skip is also read with the
  comment removed. A MariaDB `/*M!` without version digits on a MySQL profile used to be read
  only as code. Its one running reading now has the version `NEVER_ON_MYSQL` (`u32::MAX`), so
  the threshold views always include the skipped reading as well. Each comment now carries a
  list of readings, each with its own opener, end and version:
  - MySQL's six-digit gate is read both as five digits plus a body digit and as six digits
    (M7);
  - an ambiguous end adds the running end;
  - a version gate adds the skipped reading.

  The union of readings is the classification. The views cover an unknown server version, a
  version below each gate and a version above it, whatever version is known.
- **The reading cap (C2).** The number of readings is
  `(distinct versions + 1) × Π readings per comment`, counted with checked arithmetic, and
  counting stops as soon as it passes `max_exec_comment_views`. Past 16, the statement is
  `TooComplex` before any reading is built, with no loop over readings. Readings are then
  enumerated with a mixed-radix counter that the bound limits. 62, 63, 64 and 200 ambiguous
  comments are `TooComplex` within the test's time bound, in debug and in release.
- **`SET` (I1).**
  - `@@global.`, `@@persist.` and `@@persist_only.` take dotted component names (`@@global.validate_password.length`).
  - A `SET` the grammar cannot read keeps the targets it read and is `Unknown`, with `ParseError` and `UNKNOWN`.
  - It is also `ADMIN` when any `GLOBAL`, `PERSIST` or `PERSIST_ONLY` word, or any `@@global`/`@@persist` scope, appears anywhere in it.
- **Constant predicates (I2).** SAFE-07's `AlwaysTrue` uses a shared, parser-free rule in
  `destructive.rs` (`Predicate`, `always_true`), a named exception in Task 4's file.
  - **How it decides.** It evaluates three-valued logic over literals: `NULL IS NULL`, `NOT FALSE`, `TRUE`, `1 = 1`, `'a' = 'a'` and `… OR TRUE` are `AlwaysTrue`. An expression that reads nothing of the row but whose value the rule does not model (`1 + 1 = 2`, a user variable, a bound parameter) counts as always true too, the safe direction. FALSE or NULL for every row, or anything that reads the row, is not.
  - **Strings** compare as equal only when their bytes are equal; any other pair is left to the server's collation, which also counts as constant.
  - **Backends.** The interim backend maps `sqlparser` expressions to `Predicate`. Under ANSI_QUOTES, `"a"` is a column. Task 9 maps PostgreSQL's tree the same way (routed carry).
- **Information disclosure (I3, controller ruling; PO list).** These are `Unknown` (an
  `EffectsUnknown` doubt naming the form) and are denied on read-only. M3b has no
  `INFO_DISCLOSURE` bit, and this matches Task 3's line for `version()` and `current_setting`.
  - The forms: `SHOW [FULL] PROCESSLIST`, `SHOW ENGINE …`, `SHOW {REPLICA|SLAVE|MASTER} STATUS`, `SHOW ALL {REPLICAS|SLAVES} STATUS`, `SHOW BINARY LOG STATUS`, `SHOW REPLICAS`, `SHOW SLAVE HOSTS`, `SHOW {BINLOG|RELAYLOG} EVENTS`, MariaDB's `SHOW {EXPLAIN|ANALYZE} FOR <connection>`, and `SHOW GRANTS FOR` another account.
  - Still reads: `SHOW GRANTS`, `SHOW GRANTS FOR CURRENT_USER[()]` and `SHOW ENGINES`.
  - The corpus drops these forms, and the `permissive` gap is gone.
- **Minors.**
  - `CALL` is always `FnUnknown` with `UNKNOWN`: a procedure is never a builtin function. Its deny-list effects are kept.
  - A query whose body is a data change (`WITH … DELETE`) has that change's kind.
  - MariaDB's quoted pseudo-column ``seq.`nextval` `` is `SEQUENCE`.
  - A golden test pins `sqlparser` at `=0.64.0` and checks the `Debug` text the walk reads: variant names, `Join(None)`, `CrossJoin(None)` and `SetDataType`.
  - The read corpus now also runs at MySQL 8.0.12 and 8.4.0 and at MariaDB 10.6.0, with zero false denials. It gained 8 MariaDB-specific reads: `seq_1_to_n`, `OFFSET … FETCH`, `INTERSECT ALL`/`EXCEPT ALL`, `JSON_TABLE`. Its sizes are now 325, 335, 312 and 304.
