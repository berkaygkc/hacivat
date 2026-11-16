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

## m3b-t06 — parameters

**What the scan reads.** `sql_split::scan_params` lexes the unit on its own, from the initial
state, as the server receives it. Only code tokens are read. A marker inside a string, quoted
identifier, comment, dollar body, client command or `COPY` data is never a placeholder; after
a `COPY`'s first `;` the rest of a unit is data. An executable comment's body is code to the
server, so it is lexed and read too.

**One rule for refusal: if one placeholder cannot bind, none does.** When any placeholder
cannot bind, every placeholder is `bindable: false` and `rewritten` is `None`. Run then
refuses the unit and sends nothing. `rewritten` is `Some` exactly when there is at least one
placeholder and every one binds. Every refusal carries a diagnostic:

- `ParamInDdl`: a placeholder in `CREATE`/`ALTER`/`DROP`, unless `in_ddl` is on. The head is
  the caller's `StmtHead` or the unit's own first word, so a stale head cannot unlock DDL.
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
  - `$0`, or `$n` with trailing junk;
  - an SQLite `:name` while `:name` parameters are off, or a malformed SQLite variable
    (SQLite binds an unbound variable as NULL, so sending it would be silent);
  - any placeholder of a `COPY` or `PREPARE`;
  - any placeholder in a unit of several statements (a verbatim "Run as one" unit). No
    protocol binds values across statements, and the head describes only the first;
  - any placeholder beside a comment end the server decides (`AmbiguousCommentEnd`);
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
only a `QuestionMarkOff` hint (D-08).

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
are dropped. Literals, `COPY` data and parameters become `?`, and a MySQL charset introducer
joins its string. A parenthesised list of `?` only becomes `(?…)`, and mixed lists keep
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
