# M4 benchmarks: G1, the cached catalog on screen (Task 29)

G1 is the M4 exit criterion "cached catalog shown in < 300 ms at the 160k-column scale" (spec §6
M4; §3.3 "Cached catalog load (160k columns) < 300 ms", HV-MODEL-03). Task 29 builds the bench
and records a first number. The graded run is the PO's, on the PO's Mac (L2), at the gate
(Task 33 Step 3). Until then the G1 cell is **PENDING-PO**. The figures in section 4 come from
the implementer's machine under heavy parallel load, and they do **not** gate.

## 1. Measurement definition (verbatim from the plan, Task 29)

- **Fixtures:** (a) synthetic: 15,000 tables, 160,000 columns, 15,000 PKs, 5,000 FKs, 20,000 indexes in one schema (Task 11's generator, seeded); (b) real: `hv_m4_big` on `pg17` (Task 16's fixture), introspected at `Structure` level, persisted, cache file copied out through `hacivat_testkit::artifact`.
- **t0:** the app's `CatalogHost` begins opening the profile's cache file (app start, profile already expanded in the restored workspace, server unreachable: `--offline` bench flag).
- **t1:** the first presented frame in which the explorer shows that schema's kind groups with their counts and the first visible table rows painted (frame-presented hook of the existing bench harness, `crates/app/hacivat-app/src/bench/{b1_real,b3}.rs` precedent).
- **Budget:** `t1 − t0 < 300 ms` p95 over 10 cold-process runs **on the PO's Mac (authoritative, L2)**, plaintext cache (the default for non-production profiles). **Also measured with the same t0, recorded and reported separately (not gating):** the encrypted variant (an encrypted production profile expanded in the restored workspace, R-M4-15), with the `CatalogKey` keyring share broken out; encrypted > 300 ms is a PO item **listed in the demo notes** (`po-demo-m4.md`), not hidden in `m4.md`. Also recorded: the core-only share from `persist_perf` (≤ 200 ms on CI as a relative signal), memory per column and per table, and §3.3 "≤ 1 s to an interactive workspace with a cached catalog" from process start.
- **D-038 evidence (R-M4-21):** cold `Names` and `Structure` times on `hv_m4_big` with the local Docker server, printed with the thresholds.

## 2. How the bench realises it

The bench is `crates/app/hacivat-app/src/bench/catalog.rs`, run as the app example
`catalog_cold` (feature `bench`, P-M4-19).

- **Fixture (a).** `hacivat-meta` `tests/bench_fixture.rs` writes it. This is Task 11's
  `persist_perf` shape (`fixture::table_subtree`, an index per table plus a second on every
  third, a foreign key on every third), with all 15,000 tables in `app.public`, plus the server
  and database namespaces. The generator is deterministic, so every run writes the same file
  (about 7.2 MB). `bench_fixture_is_15k_tables_160k_columns` checks the exact counts.
- **Fixture (b).** `hacivat-introspect` `tests/docker_big_catalog.rs` writes
  `<artifacts>/g1-hv_m4_big.v1.sqlite`. Its test `big_catalog_cold_times_recorded_and_cache_artifact_written`
  prints `HV-FACT g1 …`: the cold `Structure` and `Names` times and the D-038 thresholds (current
  namespace: `Sources` ≤ 2,000, `Structure` ≤ 20,000; other namespaces: `Structure` ≤ 5,000;
  15,000 tables is therefore `Structure` when current and `Names` otherwise).
  - "Cold" means each level runs on a fresh connection, `Structure` first, so neither pass
    reuses the other's backend caches. The server's shared buffers stay warm, because the fixture
    was just ensured.
  - The artifact holds one namespace, `[<db>, "hv_m4_big"]` (`hv_test.hv_m4_big` on the matrix's
    `pg17`), not `app.public`. Pass it as `--fixture`. The bench watches the fixture's schema
    namespace with the most tables, prints it on the `fixture` line and records it in each run's
    `schema`. `--schema db.schema` names another one; a schema the fixture lacks is a usage
    error.
  - The real fixture's cache was not measured in Task 29 (section 4).
- **The fixture check.** The bench checks the fixture's shape on a copy, so the user's file is
  never opened. If the fixture is smaller than 15,000 tables / 160,000 columns, the run is
  **INVALID** (exit 3, no JSON). `--any-fixture` runs it anyway, ungraded.
- **Cold process, warm disk.** Each run is a new child process (`--child`) with its own
  throwaway root. The cache file is copied into the profile's place just before the child boots,
  so the OS file cache still holds it. Dropping the file cache needs root on macOS, so the bench
  does not.
- **The app start.** The run uses a saved development profile (`127.0.0.1:9`, a fake driver that
  never dials; nothing connects). Its database is the watched schema's database. Its
  `workspace.json` restores the explorer with the profile, that database, the schema and the
  schema's `TABLES` group expanded (`app`, `public`, `TABLES` for fixture (a)).
- **t0.** The app's own source is wrapped as `TimedSource`: the same `CatalogHub` that
  `live::install` builds, installed before any explorer task runs. t0 is stamped when the host's
  lifecycle task calls `CatalogSource::ensure` on the core runtime. The key fetch, the cache
  open, the read, the decode and the publish all happen inside that call.
- **t1.** A `Window::on_next_frame` loop spawns, every frame, a task that runs on the next
  main-thread turn, after that frame's draw and present (the `FrameCpu` rule of the M3 benches).
  That task checks the tree's rows **built in that frame**. They must hold the watched schema's
  `TABLES` header with its count (`15,000`) and at least one table row under it. t1 is never
  taken from the model alone.
- **Phases per run.**
  - `key_fetch_ms`: the `CatalogKey` read, timed in the secret backend.
  - `cache_load_ms`: the rest of `ensure`.
  - `host_ms`: from `ensure`'s answer until the explorer holds the snapshot, sampled per frame.
  - `paint_ms`: from there to t1.
  - `process_to_t1_ms`: from the first line of the example's `main` (§3.3's ≤ 1 s). The clock
    starts at `main`, so the exec, the dynamic loader and the Rust runtime start before it are
    not counted. That understates "from process start" by that time, typically a few to some
    tens of milliseconds.
  - Also recorded: the RSS at t1, the display rate and scale, and the viewport.
- **The encrypted variant.** A production profile (`catalog_sensitive`) whose sealed copy of the
  same fixture is opened with its `CatalogKey`. The secret store is a memory backend, so the
  Keychain is never touched. `--key-delay-ms N` adds a fixed delay to every key read to model a
  slow keyring. The variant is recorded and never graded.
- **Validity.** A run whose window is not key at t1 makes the result **INVALID** (PENDING-PO):
  GPUI throttles inactive windows. `--allow-inactive` lets the run go on, ungraded.
- **Output.**
  - Each variant writes `target/bench/<label>-<os>-….json` in the shared bench schema
    (`hacivat-bench`'s latency dump: one `values_ms` entry, `t1 − t0`, per run). The dump is
    extended with `gate`, `t0`, `t1`, `budget_ms`, `graded`, `outcome`, `fixture`, `figures`,
    `phases_p95` (with `key_fetch_ms`), `rss_bytes_p50` and every run.
  - `cargo xtask bench-report <dumps> --milestone m4` adds the rows to `m4.md`. Its verdict
    column reads `n/a`, because `hacivat-bench` has no G1 threshold; the bench's `RESULT:` line
    is the verdict.

## 3. The PO's run (authoritative)

On the gate's `m4` commit, with the display awake, the session unlocked and the bench window
left frontmost:

```text
HACIVAT_G1_FIXTURE=$PWD/target/bench/g1-160k.v1.sqlite \
  cargo test --release -p hacivat-meta --test bench_fixture -- --ignored write_g1_fixture
HACIVAT_BENCH_HOST=po-mac cargo run --release -p hacivat-app --features bench --example catalog_cold -- \
  --fixture target/bench/g1-160k.v1.sqlite --runs 10 --offline
cargo xtask bench-report target/bench/g1-catalog-cold-*.json --milestone m4
```

The last line is `RESULT: PASS|FAIL|INVALID g1-catalog-cold`, and the exit code follows it. The
encrypted line comes just before it (`RESULT: n/a g1-catalog-cold-encrypted`). To model a
Keychain read, add `--key-delay-ms N` with the delay the PO measures (`po-demo-m4.md`, item
"G1 encrypted: the real Keychain read"). Any encrypted p95 above 300 ms goes into
`po-demo-m4.md`.

For fixture (b), take the artifact of the controller's Docker run:

```text
cargo run --release -p hacivat-app --features bench --example catalog_cold -- \
  --fixture <artifacts>/g1-hv_m4_big.v1.sqlite --runs 10 --offline
```

## 4. Implementer's run (Task 29; not authoritative)

Conditions:
- **Build:** `b5cbfad`, release, macOS.
- **Window:** 1440 × 900 at 120 Hz, 1×.
- **Machine load:** load average about 44, from other agents' parallel cargo builds.
- **Host label:** `unlabelled`.
- **Fixture:** (a), 3 namespaces, 15,000 tables, 160,000 columns, 7.2 MB.
- **Runs:** 10 cold processes per variant. Every window was key at t1.

| Variant | p50 | p95 | max | `key_fetch_ms` p95 | `cache_load_ms` p95 | `host_ms` p95 | `paint_ms` p95 | process start → t1 p95 | RSS at t1 (p50) | Result |
|---|---|---|---|---|---|---|---|---|---|---|
| plaintext (graded) | 81.2 ms | **164.9 ms** | 164.9 ms | 0.0 ms | 67.1 ms | 81.6 ms | 30.8 ms | 1,047 ms | 229 MiB | **PASS** (< 300 ms) |
| encrypted (recorded) | 82.2 ms | 147.3 ms | 147.3 ms | < 0.01 ms (memory store) | 76.8 ms | 68.8 ms | 9.8 ms | 958 ms | 236 MiB | n/a |

Reading the table:
- **The shares.**
  - The hub's load and publish (`cache_load_ms`) is a steady 53–77 ms per run. This matches
    C-3's core-only `persist_perf` figure (plaintext p95 59 ms, encrypted 78 ms). The same test
    re-run on `b5cbfad` (`HACIVAT_PERF=1`, release, the same load) gives plaintext p95 54.5 ms,
    encrypted 66.6 ms and decode alone 46.9 ms, within its 200 ms budget.
  - The UI side (`host_ms` + `paint_ms`) is usually 11–40 ms. The tail (up to 98 ms in one run,
    with `host_ms` 82 ms) follows the machine's load.
  - Row encryption adds about 5–10 ms to the load.
- **Memory.** The RSS at t1 is the whole app (GPUI, fonts, the runtime) plus the catalog. That
  is an upper bound of about 1.5 KB per column, or 16 KB per table. The core model alone is
  224 B per column (ledger C-3).
- **§3.3 "≤ 1 s to an interactive workspace with a cached catalog".** From the example's `main`
  to t1: p50 about 0.87 s, two runs of ten just over 1 s under this load. This is recorded, not
  gated here. Most of it comes before t0: fonts, window, workspace restore. Exec and the loader
  come before `main` and are not counted.
- **Fixture (b), `hv_m4_big` on `pg17`.** Not run here: the Docker matrix containers were down,
  and implementers never start them. The controller's `--set pr` run prints the D-038 facts and
  writes the artifact. Before fix round 1, the bench watched only `app.public`. Since then it
  watches the fixture's own schema, tested headless with a fixture in `hv_test.hv_m4_big`.
