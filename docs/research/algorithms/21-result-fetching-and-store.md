# 21 — Result Fetching and Result Store: Canonical Algorithm Catalog (HV-RES)

> Status: canonical, implementation-ready. Date: 2026-11-11. Scope: how rows get from the database into Hacivat's memory and back out to the grid, MCP and exporters: the typed value model, the columnar result store, the in-process window API that replaces the obsolete webview transport, per-DBMS cursors/portals/streaming, the cursor lifecycle (no re-read from row 0), table-editor paging (keyset/offset with a look-ahead row), memory caps and fetch-all, LOB truncation and lazy loading, column origin resolution, row counting, server-side filter/sort rewriting, filter-by and related-row predicates, auto-refresh, and non-SQL value paging.
>
> Synthesis document; conventions in `../README.md`. Companions: `20-query-execution.md` (HV-EXEC), `22-data-grid-component.md` (HV-GRID), `23-data-editing.md` (HV-EDIT), `24-clipboard-import-export.md` (HV-XFER).

## Sources and licenses

| Source document | Specs consumed | Underlying license |
|---|---|---|
| `datagrip/08-data-grid.md` | ALG-GRID-07, -08, -09, -14, -22; §3.1, §3.6, §3.7, §3.13 | DataGrip proprietary (behaviour only) |
| `datagrip/07-query-execution-ux.md` | §3.10 registry keys, §3.24 per-DBMS mechanics | DataGrip proprietary |
| `datagrip/11-settings-catalog.md` | DV-1…DV-7 | DataGrip proprietary |
| `clients/dbeaver-04-results-viewer-data-editor.md` | ALG-DBV-GRID-03, -04, -05, -08; §3.4–3.8, §3.14, Q8/Q9 defects | DBeaver CE Apache-2.0 @ `f78ac45` |
| `clients/dbeaver-05-data-transfer-localization-lessons.md` | §3.13 (OOM issues #34854, #11931, #17099, #3568) | Apache-2.0 |
| `clients/oss-clients-beekeeper-and-tui.md` | ALG-DATA-01 (limit+1, keyset, opaque tokens), Rust learnings §5 | Beekeeper GPL-3.0 (behaviour only); harlequin, rainfrog, gobang MIT |
| `clients/oss-clients-redisinsight-compass.md` | ALG-REDIS-02, ALG-MONGO-02 (Missing vs NULL) | RedisInsight/Compass SSPL (behaviour only); mongodb-schema Apache-2.0 |
| `clients/ui-01-stack-decision.md` | ALG-UI-01 (**obsolete transport**, concepts reused: generation, page grid, truncated cells, streaming row count) | — |
| `clients/ui-02-code-editor-and-data-grid-components.md` | §3.3.9 native grids | — |
| `i18n/02-turkish-and-db-specific-pitfalls.md` | §3.8.2 (exact decimals, never through f64), §3.9.2 (session `DateStyle`, time zone), §3.15 (encoding) | — |
| `decisions/0001-native-ui-on-gpui.md` | in-process result windows replace ALG-UI-01 | — |
| `verification/01-db-behavior-tests.md` | #3, #4 (`DECLARE … WITH HOLD` allowed in RO txn), #26 (`xmin` freezing), #39–#42 (Turkish collation) | — |

Clean-room: own pseudocode only. Permissive code (DBeaver Apache-2.0, harlequin MIT) is cited by file for behaviour; nothing is copied.

---

## 0. Dependency map and priorities

```
 HV-RES-01 Value model ─────────────┬───────────────────────────────────────────────┐
        │                           ▼                                               │
        │                 HV-RES-02 columnar store ◄── HV-RES-07 memory budget/spill │
        │                    │            ▲                                         │
        ▼                    ▼            │                                         │
 HV-RES-04 driver fetch ─► HV-RES-05 cursor lifecycle (console results)             │
   (per DBMS)        │       │                                                      │
        │            │       ▼                                                      │
        │            └─► HV-RES-06 table paging (keyset/offset, limit+1)            │
        │                    │                                                      │
        ▼                    ▼                                                      ▼
 HV-RES-09 column origin  HV-RES-11 server filter/sort ◄── HV-RES-12 filter-by / FK nav
        │                    │
        ▼                    ▼
 HV-EDIT-01 identity     HV-RES-10 row count     HV-RES-08 LOB truncation/lazy load
                                                  HV-RES-13 auto-refresh
 HV-RES-03 in-process window API ──► HV-GRID (doc 22), MCP, HV-XFER (doc 24)
 HV-RES-14 Redis/Mongo paging (v1.x)
```

| ID | Name | Priority | Depends on | Consumed by |
|---|---|---|---|---|
| HV-RES-01 | Typed value model and logical types | **v1** | — | everything |
| HV-RES-02 | Columnar result store with snapshots | **v1** | 01 | 03, 05, 06, grid, MCP, export |
| HV-RES-03 | In-process result window API | **v1** | 02 | HV-GRID-01, MCP, HV-XFER |
| HV-RES-04 | Per-DBMS fetch layer (cursors, portals, streaming, decoding) | **v1** (PG, MySQL/MariaDB, SQLite, MSSQL); v1.x (ClickHouse) | 01 | 05, 06, HV-EXEC-05 |
| HV-RES-05 | Cursor lifecycle for console results (read-ahead, suspend, release) | **v1** | 02, 04, HV-EXEC-05 | grid, MCP |
| HV-RES-06 | Table-editor paging: keyset/offset with limit+1 | **v1** | 04, HV-EDIT-01 (key) | grid |
| HV-RES-07 | Memory budget, fetch-all and spill | **v1** (caps, fetch-all); **v1.x** (spill to disk) | 02 | 05, 06 |
| HV-RES-08 | LOB truncation and lazy load | **v1** | 02, 04, HV-EDIT-01 | grid, copy, edit |
| HV-RES-09 | Column origin resolution | **v1** (PG, MySQL, SQLite); v1.x (MSSQL) | 04, catalog cache | HV-EDIT-01, HV-GRID-10 |
| HV-RES-10 | Row counting (estimate + exact on demand) | **v1** | 04 | grid status |
| HV-RES-11 | Server-side filter and sort rewriting | **v1** | hacivat-sql, 04 | grid |
| HV-RES-12 | Filter-by and related-rows predicate builder | **v1** (filter-by); **v1.x** (related rows incl. virtual FKs) | 11, HV-EDIT-09 literal renderer | grid, MCP |
| HV-RES-13 | Auto-refresh scheduler | **v1.x** | 05, 06 | grid |
| HV-RES-14 | Non-SQL paging (Redis per-type, Mongo cursors) | **v1.x** | 01, 02 | Redis/Mongo views |

## 1. Cross-cutting decisions for this catalog

1. **No transport layer.** ALG-UI-01 (windowed Arrow IPC from core to a webview) is obsolete (ADR 0001). The grid reads immutable store snapshots **in process** via `Arc`, with no serialization and no copies. Concepts kept from ALG-UI-01: `generation` counters, truncated-cell flags, streaming row counts, "newest request cancels older ones".
2. **Own columnar layout, Arrow-shaped.** Validity bitmaps plus typed buffers plus offsets, so that conversion to Arrow `RecordBatch` for Parquet/Arrow export (doc 24) is cheap. **No `arrow-rs` dependency in the v1 core.** It is heavy for compile time and binary size, and its type system does not carry DB-specific facts (PG `numeric` per-value scale, `infinity`, MySQL zero dates, MSSQL `datetimeoffset` offsets). Rejected: Arrow as the store format (dbeaver-04/ui-01 suggestion); rejected: row-oriented `Vec<Vec<Value>>` (DBeaver `Object[]` → OOM at 25k wide rows, #34854).
3. **Exact values, typed end to end.** NUMERIC and DECIMAL are never converted through `f64` (i18n/02 §3.8.2). 64-bit integers stay `i64`/`u64`. Timestamps keep server precision. Unknown types are kept as verbatim server text (`Opaque`), never dropped. Fetch-time decode failures become a per-cell `Error` value instead of failing the page (DBeaver `DBDValueError` idea).
4. **Never re-read from row 0.** Console results stream from **one** execution and keep the server cursor while useful (HV-RES-05). Table editors use keyset paging (HV-RES-06). DBeaver's default re-read-on-scroll (O(m²) traffic, Q8) and client-side offset skipping (Q9) are rejected.
5. **Look-ahead row.** Every bounded fetch asks for `n + 1` rows; `has_more = received > n` (fixes DBeaver's phantom "more" at exact multiples).
6. **Session settings for deterministic decoding** (i18n/02 §3.9.2, §3.15): PG `client_encoding=UTF8`, `DateStyle=ISO, MDY`, `IntervalStyle=postgres`, `extra_float_digits=1` (PG < 12 only; PG ≥ 12 already returns the shortest round-trip form), session `TimeZone` = the connection's configured display zone (default: OS zone); MySQL `SET NAMES utf8mb4`, `time_zone` per connection; MSSQL `SET DATEFORMAT ymd`, `SET LANGUAGE us_english` only when the user allows it (side effects on messages).
7. **Rust placement.** `hacivat-core::results` (value model, store, window API, budget, paging cursors), `hacivat-core::driver::{pg, mysql, mssql, sqlite, clickhouse, redis, mongo}` (fetch + decode), `hacivat-core::results::rewrite` (filter/sort/paging SQL), `hacivat-core::catalog` (cache used for origins and estimates, owned by the introspection docs). The UI side only sees `hacivat-core::results::window`.
8. **Crate versions to check** (local cache / refs, 2026-11-11): `parking_lot 0.12.5`, `indexmap 2.14.2`, `lru 0.12.5`, `zstd 0.13.3` (spill), `rust_decimal 1.37.1` (fast-path arithmetic only; storage is our own `i128` mantissa + scale), `jiff 0.2.17` (time zones, calendar formatting), `rusqlite 0.32.1` (cached; check newer; needs features `bundled`, `column_metadata`), `tokio-postgres`/`postgres-types`, `mysql_async`, `tiberius`, an HTTP client for ClickHouse (`reqwest`/`hyper`), `arc-swap` (snapshot publication), `roaring` (tombstones) — versions not cached locally; check at implementation.

---

## 2. Specifications

### HV-RES-01: Typed value model and logical types

- **Priority / depends on:** v1. Foundation; no dependencies.
- **Sources merged:** datagrip/08 §5.1 (Value enum incl. `Truncated`, `Unparsed`, `Default`), dbeaver-04 §5 (adds `Error`, `Composite`; value-handler registry), ALG-MONGO-02 (Missing ≠ NULL), dbeaver-05 ALG-DBV-XFER-01 recommendation (logical types shared with transfer), i18n/02 §3.8.2/§3.9.2 (exact decimals, temporal specials), Beekeeper/TUI §5.2 ("typed values end-to-end").
- **Problem:** One value vocabulary for decoding, storage, display, editing, clipboard, MCP, scripting and transfer that never loses information and distinguishes NULL, missing, DEFAULT, empty string, truncated, unparsable input and decode errors.
- **Inputs / outputs:** driver column metadata and wire values → `LogicalType` per column and `Value` per cell (row-oriented exchange type) or column buffers (doc HV-RES-02).
- **Decision:**
  - Two layers. `LogicalType` describes a column (portable, engine-neutral, carries precision, scale, time zone and length). `NativeType` keeps the engine's exact type (`pg: oid + typmod + name`, `mysql: column_type + flags + charset`, `mssql: TypeInfo`, `sqlite: declared type + affinity`). Every column carries both.
  - `Value` is the exchange type. The store keeps typed column buffers and materializes `Value` / `ValueRef` only for the cells requested.
  - Special states are first-class variants: `Null`, `Missing` (document field absent), `Default` (edit intent only), `Truncated`, `Unparsed` (edit overlay only), `Opaque` (unknown server type, verbatim text or bytes), `Error` (decode failure).
  - Rejected: everything-as-string (rainfrog/gobang, loses type info); `serde_json::Value` (loses decimal precision, binary, temporal types).
- **Algorithm (type mapping):**
  1. At `ResultSetStart`, map each column's `NativeType` → `LogicalType` with the per-DBMS table below. Unknown → `Opaque{type_name}`.
  2. Choose the column buffer kind from the logical type (HV-RES-02).
  3. Decode each wire value into the buffer. On a decode error, store `Error{code, message}` in the cell's exception slot and log once per (column, error kind) (DBeaver rule).
  4. Values that do not fit the buffer kind (SQLite dynamic typing, PG `numeric` NaN/±Infinity, `infinity` dates, MySQL zero dates, 0-byte MSSQL `datetime` oddities) are stored as exceptions holding a full `Value`.
- **Data structures and Rust placement** (`hacivat-core::results::value`):
  ```rust
  pub enum LogicalType {
      Bool, Int { bits: u8, signed: bool }, Decimal { precision: Option<u32>, scale: Option<i32> },
      Float { bits: u8 }, Money,
      Text { max_chars: Option<u32>, fixed: bool }, Bytes { max: Option<u32> }, Uuid,
      Date, Time { precision: u8, tz: bool }, Timestamp { precision: u8, tz: bool }, Interval,
      Json { binary: bool }, Xml, Array(Box<LogicalType>), Composite(Arc<[(Arc<str>, LogicalType)]>),
      Enum(Arc<[Arc<str>]>), Range(Box<LogicalType>), Network(NetKind), BitString { len: Option<u32> },
      Geometry { srid: Option<i32> }, Document /* BSON */, Opaque { type_name: Arc<str> },
  }
  pub enum Value {
      Null, Missing, Default,
      Bool(bool), Int(i64), UInt(u64), F32(f32), F64(f64), Decimal(DecimalVal),
      Text(Arc<str>), Bytes(bytes::Bytes), Uuid([u8; 16]),
      Date(DateVal), Time(TimeVal), TimeTz(TimeVal, i32 /*offset s*/),
      Timestamp(TsVal), TimestampTz { utc: TsVal, offset: Option<i32> }, Interval { months: i32, days: i32, micros: i64 },
      Json(Arc<str>), Xml(Arc<str>), Array(Arc<ArrayVal>), Composite(Arc<[Value]>),
      Geometry { srid: Option<i32>, wkb: bytes::Bytes }, Bson(Arc<RawBsonValue>),
      Special(SpecialVal),                 // ±infinity, MySQL zero date, NaN for decimals, BC dates out of range
      Opaque { type_name: Arc<str>, text: Arc<str> },
      Truncated(Box<TruncatedVal>),        // { prefix: Value(Text|Bytes|Json), total_len: Option<u64>, unit: Bytes|Chars }
      Unparsed { text: Arc<str>, err: ParseErrorKind },   // edit overlay only (HV-EDIT-03)
      Error { code: Arc<str>, message: Arc<str> },        // per-cell decode failure
  }
  pub struct DecimalVal { mantissa: i128, scale: i16 }      // or DecimalVal::Big(Arc<str>) for > 38 digits
  pub struct DateVal(i32);                 // days since 1970-01-01, proleptic Gregorian (covers PG's full range)
  pub struct TsVal { secs: i64, nanos: u32 }   // naive (Timestamp) or UTC (TimestampTz)
  pub struct TimeVal { nanos_of_day: u64 }  // allows PG 24:00:00
  ```
  `DecimalVal` keeps the server's scale (`1.50` stays `1.50`). F32 is kept as f32 so that its shortest form prints as the server shows it (`0.1`, not `0.10000000149011612`).
- **Per-DBMS matrix (selected mappings; the full table is golden test data):**

  | Logical | PostgreSQL | MySQL/MariaDB | SQL Server | SQLite | ClickHouse |
  |---|---|---|---|---|---|
  | Bool | `bool` | `TINYINT(1)` (only with `tinyint1_is_bool` connection option, default **off**: shown as Int) / `BOOLEAN` alias | `bit` | declared `BOOLEAN` affinity NUMERIC → Int unless the column is declared bool | `Bool` |
  | Int | `int2/4/8` | `TINYINT…BIGINT [UNSIGNED]` (`BIGINT UNSIGNED` → UInt) | `tinyint` (unsigned 8), `smallint`, `int`, `bigint` | INTEGER | `(U)Int8…64`; `(U)Int128/256` → Decimal |
  | Decimal | `numeric(p,s)`, unconstrained `numeric` (per-value scale) | `DECIMAL(p,s)` (arrives as text) | `decimal/numeric`, `money/smallmoney` → Money | NUMERIC affinity values that are reals → F64 (SQLite has no decimal) | `Decimal(P,S)` |
  | Float | `float4/8` | `FLOAT/DOUBLE` | `real/float` | REAL | `Float32/64` |
  | Text | `text, varchar, bpchar, name, citext` | `CHAR/VARCHAR/TEXT` (charset ≠ binary) | `(n)char/(n)varchar(max)/text/ntext` | TEXT | `String`, `FixedString` (bytes if not UTF-8), `LowCardinality(String)` |
  | Bytes | `bytea` | `BINARY/VARBINARY/BLOB` | `binary/varbinary/image` | BLOB | `String` with invalid UTF-8 → Bytes per cell |
  | Uuid | `uuid` | — (BINARY(16) shown per display type, HV-GRID-10) | `uniqueidentifier` (mixed-endian on the wire) | — | `UUID` |
  | Date/Time | `date, time, timetz, timestamp, timestamptz, interval` | `DATE, TIME, DATETIME(n), TIMESTAMP(n)` (session TZ), `YEAR` → Int | `date, time(n), datetime, datetime2(n), smalldatetime, datetimeoffset(n)` | text/real/int per the app's convention → Text (display-type opt-in) | `Date, Date32, DateTime([tz]), DateTime64(p,[tz])` |
  | Json | `json, jsonb` | `JSON` | `nvarchar(max)` (json is a convention; SQL Server 2025 `json` type → Json) | — | `JSON` (new) / `Object('json')` |
  | Array/Composite | `T[]`, composite types, ranges | `SET` → Text, `ENUM` → Enum | — | — | `Array(T)`, `Tuple`, `Map` → Composite |
  | Opaque | unknown OIDs (extensions: PostGIS without decoder, `hstore`, `ltree`) | spatial (WKB) → Geometry | `sql_variant`, `hierarchyid`, `geometry` | — | `AggregateFunction` |
- **Complexity / performance budget:** decode cost ≤ 50 ns per scalar cell on a modern CPU; `Value` is ≤ 24 bytes (large payloads behind `Arc`/`Bytes`).
- **Edge cases (exhaustive list):** NULL vs empty string vs Missing (three distinct states); PG `numeric` NaN and ±Infinity (PG ≥ 14) → `Special`; PG `infinity`/`-infinity` dates and timestamps → `Special`; BC dates and years > 9999 (DateVal covers them; the formatter falls back to ISO with sign); PG `time 24:00:00` (allowed); MySQL `0000-00-00` and partial zero dates → `Special(ZeroDate)` (never converted to NULL silently); MySQL `TIMESTAMP` is converted to the session time zone by the server; MSSQL `datetimeoffset` keeps its original offset; MSSQL `uniqueidentifier` byte order (mixed endian on the wire → canonical display); `BIGINT UNSIGNED` > `i64::MAX` → UInt; ClickHouse `UInt256` → Decimal mantissa overflow → `DecimalVal::Big`; `-0.0` floats (kept); NaN floats (kept, display `NaN`); invalid UTF-8 in text columns (e.g. `SQL_ASCII` PG databases) → keep bytes, display `\xNN` escapes, mark the cell lossy (never replace silently in editable cells, i18n/02 §3.15 #7); text containing NUL (PG rejects; MySQL/SQLite allow → kept); SQLite column declared INTEGER containing text (per-cell exception); SQLite with no declared type (expressions) → per-cell storage class; arrays of composites; multi-dimensional PG arrays with custom lower bounds (`'[0:1]={1,2}'`) → kept as Opaque text when bounds ≠ 1; enums added after connect (unknown label → Text); domains (map to the base type, keep the domain name in `NativeType`); PG `money` (text depends on `lc_monetary` → Money with the server text kept for display); geometry without PostGIS decoder → Opaque hex; very large values (Truncated, HV-RES-08).
- **Test cases (input → expected):** PG `numeric(10,2)` value `1.50` → `Decimal{150, 2}`, displays `1.50`; PG unconstrained `numeric` value `12345678901234567890.123456789` → `Decimal{…, 9}` exact; PG `'infinity'::timestamptz` → `Special(PosInfinity)`; MySQL `DATETIME '0000-00-00 00:00:00'` → `Special(ZeroDate)`; MySQL `BIGINT UNSIGNED 18446744073709551615` → `UInt(u64::MAX)`; MSSQL `datetimeoffset '2026-11-11 10:00:00 +03:00'` → `TimestampTz{utc 07:00, offset +10800}`; SQLite INTEGER column holding `'abc'` → per-cell exception `Text("abc")`; PG `float4 0.1` → displays `0.1`; PG bytea in a `SQL_ASCII` text column with byte `0xFF` → lossy flag, display `\xff`.
- **i18n notes:** value model is locale-free. Labels for special states: `value-null = NULL` (default ASCII "NULL", configurable; not translated by default because it is copied around), `value-missing = (missing)`, `value-default = DEFAULT`, `value-error = Error: { $message }`, `value-truncated = { $shown } of { $total }`.
- **Verification status:** type facts are [Inferred: vendor docs]; DBeaver handler choices [Observed: `PostgreValueHandlerProvider.java`, `JDBCStandardValueHandlerProvider.java`]. A golden decode test per DBMS (Docker images of verification/01) is required before v1.
- **Prior art and evidence:** datagrip/08 §5; dbeaver-04 §3.6, §5.
- **Used by:** Hacivat (everything), MCP (typed cells), Karagöz (result shape metadata, max-rows/bytes enforcement).

### HV-RES-02: Columnar result store with snapshots

- **Priority / depends on:** v1. Depends on HV-RES-01; budgets from HV-RES-07.
- **Sources merged:** ui-01 ALG-UI-01 (append-only batches, generation, spill index — adapted to in-process), dbeaver-04 §5 (`ResultModel`, per-cell errors), datagrip/08 §5.1 (Arrow-like pages), dbeaver-05 §7.2 open question (Arrow vs custom columnar → decided here).
- **Problem:** Hold millions of rows compactly, append while streaming, let the UI thread read any cell in O(1) without locks during painting, and let edits/refreshes patch rows without reloading.
- **Inputs / outputs:** decoded rows from HV-RES-04 → `StoreSnapshot`s published to readers (grid, MCP, export, aggregators).
- **Decision:**
  - **Fixed-size chunks of 4,096 rows** (power of two → `chunk = row >> 12`, O(1) lookup). Sealed chunks are immutable `Arc<Chunk>`. The partially filled tail chunk is republished as an immutable copy at most every 16 ms while streaming (≤ 4,096 rows copied → cheap).
  - **Snapshot publication** through `ArcSwap<StoreSnapshot>`: readers `load()` without locking. The writer builds a new snapshot (`Arc` clone of the chunk vector + new tail) and swaps.
  - **Overlay layers inside the snapshot** for post-submit reconciliation: `patches: Arc<HashMap<RowIdx, Arc<[Value]>>>` (rows refreshed after edits), `tombstones: RoaringTreemap` (rows deleted by submits), `local_appends: Vec<Arc<Chunk>>` (rows inserted by submits). They avoid full reloads and keep scroll position (HV-EDIT-08).
  - `generation` bumps only when the row set is *replaced* (re-run, server-side sort/filter change, reload). Appends and patches do not bump it; they bump `version`.
- **Algorithm:**
  1. `ResultSetStart(columns)` → create `ResultStore { schema, writer: ChunkBuilder, published: ArcSwap }`, generation = parent generation + 1 (or 1).
  2. For each decoded row: append into the builder's column builders. When 4,096 rows are reached → seal the chunk (shrink buffers to fit, compute byte size), push it to `sealed`.
  3. Publish: at most every 16 ms, or immediately after the first 500 rows (first paint), or at the end. Build `StoreSnapshot { generation, version += 1, sealed: Arc<[Arc<Chunk>]> (cheap: copy-on-write vector of Arcs), tail: Option<Arc<Chunk>>, row_count, completeness, bytes }` and swap it in. Notify subscribers through a `watch` channel carrying `(generation, version)`.
  4. Cell access: `snapshot.cell(row, col)`: check `tombstones` → hidden (the view mapping excludes them, HV-GRID-06); check `patches[row]` → patched value; else `chunk = row >> 12`, `i = row & 4095`, read the column buffer: validity bit → `Null`; exception map → stored `Value`; else the typed slot → `ValueRef`.
  5. Patch after submit (HV-EDIT-08): build new `patches` / `tombstones` / `local_appends` from the reconciled rows and publish a new version.
  6. Drop: when the result tab closes or is replaced, the store is dropped once the last snapshot `Arc` is gone (readers hold snapshots for at most a frame or a bounded export).
- **Data structures and Rust placement** (`hacivat-core::results::store`):
  ```rust
  pub struct Chunk { pub rows: u32, pub cols: Box<[ColumnBuf]>, pub bytes: u64 }
  pub enum ColumnBuf {
      Bool   { valid: Bitmap, bits: Bitmap, exc: Exceptions },
      I64    { valid: Bitmap, v: Box<[i64]>, exc: Exceptions },     // also u64 via UInt variant
      F64    { valid: Bitmap, v: Box<[f64]>, exc: Exceptions },
      Dec    { valid: Bitmap, mant: Box<[i128]>, scale: Box<[i16]>, exc: Exceptions },
      Var    { valid: Bitmap, offs: Box<[u32]>, data: Box<[u8]>, kind: VarKind /*Text|Bytes|Json|Xml|Opaque*/, exc: Exceptions },
      Fixed16{ valid: Bitmap, v: Box<[[u8;16]]>, exc: Exceptions }, // uuid
      Temporal { valid: Bitmap, a: Box<[i64]>, b: Box<[u32]>, kind: TempKind, exc: Exceptions },
      Values { v: Box<[Value]> },                                   // heterogeneous fallback (documents, arrays)
  }
  pub type Exceptions = Option<Box<HashMap<u32, Value>>>;            // sparse: Truncated, Error, Special, foreign storage class
  pub struct StoreSnapshot { pub generation: u64, pub version: u64, pub schema: Arc<ResultSchema>,
      sealed: Arc<[Arc<Chunk>]>, tail: Option<Arc<Chunk>>, pub row_count: u64, pub completeness: Completeness,
      patches: Arc<HashMap<u64, Arc<[Value]>>>, tombstones: Arc<roaring::RoaringTreemap>, local_appends: Arc<[Arc<Chunk>]>,
      pub bytes: u64 }
  pub enum Completeness { Streaming, Suspended { cursor: CursorInfo }, Complete, Closed { reason: CloseReason }, Cancelled, Truncated { reason: TruncReason } }
  ```
- **Complexity / performance budget:** `cell()` O(1), ≤ 30 ns (two pointer hops + bit test); append ≥ 2M cells/s per core; publish ≤ 50 µs; memory overhead ≤ 10% over raw payload for typical tables (bitmaps + offsets). 10M rows × 50 columns of mixed ints/short text ≈ 3–6 GB raw — beyond the default budget, so HV-RES-07 caps or spills (the B1 benchmark of ADR 0001 uses a synthetic in-memory source for the grid itself).
- **Edge cases (exhaustive list):** zero columns (no store; outcome is an update count); zero rows (header only, `Complete`); a column whose type changes mid-stream (SQLite per-row types; exceptions); text offsets exceeding `u32` within a chunk (chunk sealed early when the data buffer reaches 2 GiB; chunks may hold fewer than 4,096 rows → lookup via a prefix table for that chunk only, flagged); extremely wide rows (10k columns → the column vector per chunk is large; chunk size is reduced to 512 rows when `cols > 1000` and the per-row lookup uses a shift per store); readers holding an old snapshot during re-run (they see the old generation until they reload; the grid discards stale frames by generation); patches for rows that are tombstoned (tombstone wins); concurrent writers (exactly one writer per store: the session actor); memory pressure (HV-RES-07 evicts or spills sealed chunks; the snapshot holds `ChunkRef::Spilled`, v1.x).
- **Test cases (input → expected):** append 10,000 rows → 2 sealed chunks + a tail of 1,808; `cell(9_999, c)` reads the tail; publish cadence: first snapshot after 500 rows, then ≤ 62 per second; patch row 5 → `cell(5, c)` returns the patched value and `generation` is unchanged; tombstone row 7 → the view count decreases by 1 (HV-GRID-06); a reader holding generation 3 while a re-run publishes generation 4 → both valid, the grid swaps on the next frame.
- **i18n notes:** none (data layer).
- **Verification status:** design decision; DBeaver OOM evidence [Observed: issues #34854, #17099 via verification/02 S27].
- **Prior art and evidence:** ui-01 ALG-UI-01 (concepts); harlequin Arrow tables (MIT) [Observed: `query.py:89-93`]; Arrow columnar format (Apache-2.0) as layout inspiration.
- **Used by:** Hacivat (grid, aggregators, copy, export), MCP (`fetch_rows` reads snapshots).

### HV-RES-03: In-process result window API

- **Priority / depends on:** v1. Depends on HV-RES-02. Replaces ALG-UI-01.
- **Sources merged:** ui-01 ALG-UI-01 (window requests, overscan, de-duplication, newest-wins, row-count streaming, truncated cells, generation discard), datagrip/08 §3.1 (`GridDataHookUp` = loader + paging + sorting + filtering + mutator), ADR 0001 (in-process).
- **Problem:** Give the grid (and MCP, export, scripts) a uniform, synchronous, lock-free way to read the cells it needs, and an asynchronous way to ask for more rows or full values, without coupling `hacivat-ui` to database concepts.
- **Inputs / outputs:** viewport range from the grid → cell values (sync) + fetch hints (async); full-value requests → `Task<Value>`.
- **Decision:**
  - `hacivat-core::results::window::ResultSource` is the core-side API. `hacivat-app` adapts it to the generic `hacivat-ui::grid::GridDataSource` trait (HV-GRID-01). `hacivat-ui` never depends on `hacivat-core`.
  - Reads are synchronous on the paint path (snapshot load + O(1) cell access). Formatting to display strings happens in the adapter with a per-(generation, version, row, col) cache (HV-GRID-03).
  - Fetch hints are fire-and-forget, debounced (16 ms) and coalesced: `ensure_rows(upto)` triggers `FetchMore` on the session queue only when the cursor is `Suspended` and `upto + margin > row_count`.
- **Algorithm:**
  1. Each frame the grid calls `source.begin_frame()` → `Arc<StoreSnapshot>` (one atomic load). All cell reads in the frame use this snapshot (consistency within a frame).
  2. For visible rows beyond `row_count`: paint skeleton placeholders and call `ensure_rows(last_visible + overscan)`.
  3. `ensure_rows` → if `Suspended` → enqueue `FetchMore(result, max(read_ahead, upto − row_count + margin))` with coalescing; if `Streaming` → nothing; if `Closed`/`Truncated` → nothing (the banner offers actions).
  4. Snapshot change notifications (`watch`) wake the GPUI entity through a `cx.spawn` loop → `cx.notify()`; at most one notify per frame.
  5. Full value: `request_full_value(row, col)` → HV-RES-08 lazy load → `Task<Result<Value>>`; the grid shows a spinner in the value viewer, not in the cell.
  6. MCP and export use the same snapshot API (`rows(range) -> impl Iterator<Item = RowRef>`), or a streaming export that bypasses the store (doc 24).
- **Data structures and Rust placement:**
  ```rust
  // hacivat-core::results::window
  pub trait ResultSource: Send + Sync + 'static {
      fn snapshot(&self) -> Arc<StoreSnapshot>;
      fn changes(&self) -> watch::Receiver<(u64 /*generation*/, u64 /*version*/)>;
      fn ensure_rows(&self, upto: u64);
      fn request_full_value(&self, row: u64, col: u32) -> BoxFuture<'static, Result<Value, FetchError>>;
      fn capabilities(&self) -> SourceCaps;     // server_filter, server_sort, editable, has_cursor, can_count …
  }
  // hacivat-app::results::GridAdapter implements hacivat-ui::grid::GridDataSource for any ResultSource
  ```
- **Complexity / performance budget:** per frame ≤ 1 atomic load + O(visible cells) reads (~1–3k cells ≤ 0.1 ms); `ensure_rows` coalesced to ≤ 1 queue message per 16 ms; notification latency (row decoded → painted) ≤ 2 frames.
- **Edge cases (exhaustive list):** row count unknown while streaming (scroll extent grows; the scrollbar shows a "≥ n" state); cancel mid-stream (`Cancelled`, rows remain browsable); re-run while the user scrolls (the grid detects a generation change and resets row-dependent caches; scroll position is kept by row index clamped to the new count); column reorder/hide (column ids are stable; the adapter maps view → model, HV-GRID-06); window closed while a full-value request is pending (the task is dropped; the core cancels the side query); a snapshot held across an `await` by MCP (allowed; bounded by the request); several grids on one result (record view + table view share the source).
- **Test cases (input → expected):** a synthetic 10,000,000-row source, jump to row 9,999,744 → cells read in ≤ 0.1 ms per frame, no allocation per cell beyond the format cache; scrolling to the end of a Suspended 50,000-row result → exactly one `FetchMore` enqueued per crossing; a re-run during scroll → no frame paints a mix of old and new generation cells.
- **i18n notes:** none (data API). Placeholder accessibility label `grid-loading-row = Loading row { $row }`.
- **Verification status:** design decision. The GPUI notify/spawn pattern [Observed: `zed@912159c` `crates/gpui` `Context::notify`, `gpui_tokio`].
- **Prior art and evidence:** ui-01 ALG-UI-01; datagrip/08 §3.1.
- **Used by:** Hacivat grid, record view, aggregators, MCP `fetch_rows`, scripted extractors (doc 24).

### HV-RES-04: Per-DBMS fetch layer (cursors, portals, streaming, decoding)

- **Priority / depends on:** v1 for PostgreSQL, MySQL/MariaDB, SQLite, SQL Server; v1.x for ClickHouse (Redis/Mongo: HV-RES-14). Depends on HV-RES-01.
- **Sources merged:** datagrip/07 §3.24 (per-DBMS mechanics table: multi-statement, cancel, tx status, read-only session, paging, explain), dbeaver-04 §3.4–3.5, ALG-DBV-GRID-03 (fetch size, `setMaxRows` pitfalls, SQL Server `setMaxRows` capping DML counts), dbeaver-05 ALG-DBV-XFER-02 edge case 30 (PG cursors need a transaction; MySQL streaming), Beekeeper/TUI §5 (extended protocol for parameters, simple protocol only for multi-statement scripts, stream with back-pressure).
- **Problem:** Implement, per engine, "execute and give me rows incrementally with back-pressure, multiple result sets, notices, cancel and transaction status", choosing the right protocol mechanism.
- **Inputs / outputs:** `(sql, binds, ExecOptions)` → `ExecStream` of `ExecEvent`s (HV-EXEC-05) with `ResultSetStart`, `Rows(DecodedChunk)`, `ResultSetEnd`, `UpdateCount`, `Notice`, `TxStatus`, `Error`, `Done`; plus a `CursorControl` (fetch more / close) for suspended results.
- **Decision:** one mechanism per (engine, statement shape):

  | Engine | Single statement with binds (extended) | Unit with several statements (no binds) | Incremental fetch & suspend | Release (close early) | Read-only session | Cancel |
  |---|---|---|---|---|---|---|
  | PostgreSQL (`tokio-postgres`) | Parse/Bind/Execute; **binary** result format for known OIDs, text for unknown | Simple Query protocol; all result sets streamed | Named portal inside a transaction: `Execute(max_rows = chunk)` → `PortalSuspended`. In autocommit we open the transaction ourselves (`BEGIN`) for Read-class statements | `Close portal` + `COMMIT` (our own tx) or nothing (user tx) | `SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY` + `default_transaction_read_only=on` | CancelRequest |
  | MySQL/MariaDB (`mysql_async`) | `COM_STMT_PREPARE/EXECUTE` (binary protocol) | `COM_QUERY` with `CLIENT_MULTI_STATEMENTS` (text protocol) | Unbuffered streaming (rows read as polled; the connection is busy until the result is drained) | `KILL QUERY <id>` on a side connection, then drain the remaining packets | `SET SESSION TRANSACTION READ ONLY` (+ `START TRANSACTION READ ONLY` per statement) | `KILL QUERY` |
  | SQL Server (`tiberius`) | `sp_executesql` (RPC) | batch text | TDS token stream read as polled | attention signal, drain to DONE(attn); fallback: close the connection [Unknown: tiberius attention API — verify] | none native → app-level gate + permissions; `ApplicationIntent=ReadOnly` for AG secondaries | attention / `KILL` |
  | SQLite (`rusqlite`, dedicated blocking thread per session) | prepare/bind/step | we split; one statement at a time (`execute_batch` has no results) | `Statement` kept alive between steps (holds a read snapshot in WAL mode) | `finalize` (drop the statement) | open with `SQLITE_OPEN_READ_ONLY` and `PRAGMA query_only=1` | `InterruptHandle::interrupt()` |
  | ClickHouse (HTTP) | query parameters `{name:Type}` | one query per request | streamed `RowBinaryWithNamesAndTypes` (or `Native`) body read as polled | `KILL QUERY WHERE query_id=…` + close the HTTP body | `readonly=1` setting per query | `KILL QUERY` by our `query_id` |

  Rejected: JDBC-style `setMaxRows(offset+limit)` with client skipping (DBeaver Q9); rejected: injecting `LIMIT` into user SQL for console results (DBeaver regex approach misses `TOP`/`FETCH`, string literals, side-effect statements).
- **Algorithm:**
  1. `execute()` chooses the protocol: binds present → extended/prepared; several statements in the unit → simple/multi; else extended without binds (PG: unnamed statement; MySQL: text protocol is fine).
  2. PG Read-class statement in autocommit mode with `cursor_mode = Portal` (default): send `BEGIN` (the gate already decided the statement is Read), Parse/Bind into a named portal `hv_<result_id>`, `Execute(max_rows = first_chunk + 1)`. Then follow HV-RES-05 for further `Execute(max_rows)` calls. Non-Read statements (DML `RETURNING`, `CALL`) execute with `max_rows = 0` and stream all rows (their side effects happen at once anyway; HV-RES-07 caps the kept rows).
  3. Decode rows into `DecodedChunk` (column builders of HV-RES-02) on the driver task; push chunks through a bounded channel (capacity 4 chunks) to the store writer → back-pressure reaches the socket.
  4. Notices and warnings: PG `NoticeResponse` as it comes; MySQL `warning_count > 0` → `SHOW WARNINGS` after the result is complete (only when the setting is on; never in the middle of a stream); MSSQL `INFO` tokens; ClickHouse `X-ClickHouse-Summary` / progress headers for written rows.
  5. Transaction status after each round trip (HV-EXEC-10 matrix).
  6. MySQL: connect with `CLIENT_FOUND_ROWS` so that UPDATE affected-row counts mean "matched" (HV-EDIT-06). [Inferred: `mysql_async` option name — verify].
  7. MSSQL: track `SET NOCOUNT`; never use `setMaxRows`-like row caps on DML.
- **Data structures and Rust placement** (`hacivat-core::driver`):
  ```rust
  #[async_trait] pub trait DriverSession: Send {
      async fn execute(&mut self, sql: &str, binds: &[BindValue], o: ExecOptions) -> Result<ExecStream, DriverError>;
      async fn fetch_more(&mut self, c: &CursorId, max_rows: u64) -> Result<ExecStream, DriverError>;
      async fn close_cursor(&mut self, c: &CursorId, how: CloseHow) -> Result<(), DriverError>;
      fn capabilities(&self) -> &Capabilities;     // portal, multi_stmt, server_cancel, terminate, row_values, nulls_order, returning…
  }
  pub struct ExecOptions { pub first_chunk: u32, pub cursor_mode: CursorMode, pub max_result_sets: u32,
                           pub classification: Classification, pub read_only: bool, pub query_id: Uuid }
  pub enum CursorMode { Portal, Stream, Materialize }
  ```
- **Complexity / performance budget:** first row painted ≤ 50 ms after it arrives on the wire (local DB); decode throughput ≥ 1M simple cells/s per session; bounded channel depth 4 chunks (≤ 16k rows in flight).
- **Edge cases (exhaustive list):** PG portal in a user's manual transaction (no extra BEGIN; closing the tx closes the portal → HV-RES-05 release); PG `default_transaction_read_only` sessions with Read-class statements calling volatile functions (server error → shown verbatim); PG statement with unbound `$n` (prompt, HV-EXEC-03); PG binary decode of an unknown OID (fall back to text format for that column — Bind requests per-column formats); PG `COPY TO STDOUT` (rows via the CopyOut stream → treated as a single text column or routed to export); MySQL multi-results with an error in the middle (results before the error are kept); MySQL streaming result abandoned without KILL (would block the connection until drained → always KILL for large remainders); MySQL `LOAD DATA LOCAL` (disabled unless explicitly enabled; security); MSSQL `DONEINPROC` row counts inside procedures; MSSQL `SET NOCOUNT ON` (no counts); MSSQL multiple result sets with different schemas; SQLite `SQLITE_BUSY` (busy timeout 5 s, then error); SQLite statement kept while another statement wants to write on the same connection (release first); ClickHouse exceptions in the middle of a streamed body (HTTP 200 already sent → detect the exception marker in the stream → `Error` event); ClickHouse `max_execution_time`; network stalls (TCP keepalive + per-request idle timeout); connection lost mid-fetch (rows kept, `Closed{ConnectionLost}`); server-side `idle_in_transaction_session_timeout` killing a suspended PG portal (connection lost path with a specific message).
- **Test cases (input → expected):** PG `SELECT generate_series(1, 1000000)` → first 501 rows arrive in one `Execute`, the cursor is Suspended, the session reports an open transaction; PG `UPDATE … RETURNING *` 10k rows → all streamed, no portal; MySQL `SELECT SLEEP(0.01), n FROM seq_1_to_100000` streaming → first chunk painted before the query finishes; MySQL release after 10k of 100k rows → KILL QUERY issued on a side connection, the connection is usable within 1 s; SQLite `SELECT` on a 1M-row table → first chunk ≤ 20 ms; ClickHouse exception mid-stream → rows before the exception kept, error shown.
- **i18n notes:** driver errors map to `HCV-DRV-*` codes with Fluent ids (`drv-connection-lost`, `drv-protocol-error`); server errors follow ALG-I18N-08 (HV-EXEC-11).
- **Verification status:** [Observed: verification/01 #4] `DECLARE … WITH HOLD` and `LISTEN` are allowed in a PG read-only transaction (so a WITH HOLD cursor is a viable alternative to an open portal on read-only connections). [Inferred: protocol docs] portal suspension, MySQL unbuffered results, TDS attention. **Must verify in Docker before v1:** MySQL KILL QUERY + drain timing, tiberius attention support, PG portal behaviour under `idle_in_transaction_session_timeout`.
- **Prior art and evidence:** datagrip/07 §3.24; dbeaver-04 §3.4–3.5; harlequin RowLimit (MIT) [Observed: `query.py:35-63`].
- **Used by:** Hacivat; Karagöz (same driver layer for proxied sessions, with max rows/bytes enforcement).

### HV-RES-05: Cursor lifecycle for console results (read-ahead, suspend, release)

- **Priority / depends on:** v1. Depends on HV-RES-02, -04, HV-EXEC-05 (queue, release before other requests), HV-RES-07 (budget).
- **Sources merged:** dbeaver-04 ALG-DBV-GRID-03 recommendation ("stream from one execution; keep the cursor open while the tab is open with an idle timeout; never re-execute for paging unless side-effect-free") and §7 open question (cursor vs spill), datagrip/07 §3.10 (`database.scrollable.result.sets`, prefetch size), datagrip/11 DV-2, dbeaver-05 #3568 (OFFSET segments are slow), ui-01 ALG-UI-01 step 9 (streaming row counts).
- **Problem:** Show the first rows immediately, let the user scroll through millions of rows without re-executing, but never hold a server cursor (and its transaction, snapshot and locks) longer than useful, and never block other work on the session.
- **Inputs / outputs:** a result in `Streaming` state; user scrolling (via `ensure_rows`); session needs; timers → state transitions and fetch requests.
- **Decision:**
  - **States:** `Streaming → Suspended → (Streaming)* → Complete | Closed{reason, rows_kept}`.
  - **Defaults:** `first_chunk = 500` rows; read-ahead in chunks of 4,096 rows until `read_ahead_target = 10,000` rows or 16 MiB, whichever comes first, then suspend; on scroll, when the last visible row is within `prefetch_margin = max(2 × viewport rows, 1,000)` of `row_count`, fetch another `read_ahead_target`.
  - **Release triggers:** the session needs the connection for another request (HV-EXEC-05); `cursor_idle_timeout` (default **10 min** without fetch activity); tab closed or result replaced; commit/rollback of the user transaction that owns the cursor; memory cap reached and the user chooses "Stop here"; app quit.
  - **Release action:** drain for at most `release_drain_ms = 250 ms` (small remainders finish and the result becomes Complete); otherwise close the cursor (per-engine close from HV-RES-04) and mark `Closed{reason, rows_kept}`. The banner offers **Re-run** and, for Read-class statements, **Re-run and fetch all** (subject to the budget).
  - **PG autocommit:** the portal lives in a transaction we opened. While suspended, the status bar shows "Cursor open — holding a snapshot" with a close button; we commit when closing. On read-only connections, `DECLARE … CURSOR WITH HOLD` is an allowed alternative [verified] but materializes the remaining rows on the server at commit; not the default.
  - Rejected: DBeaver's re-read from row 0 (O(m²)); rejected: keeping cursors forever (blocks DDL by others via AccessShareLock, holds back vacuum horizon); rejected: always draining everything (memory blow-up on billion-row selects).
- **Algorithm:**
  1. On `ResultSetStart`: state `Streaming`; request `first_chunk + 1` rows. Publish as soon as `first_chunk` rows are decoded (first paint).
  2. Continue fetching in chunks while `rows < read_ahead_target && bytes < 16 MiB && budget_ok`. If the stream ends → `Complete` (exact row count known).
  3. Otherwise → `Suspended{cursor}`; arm the idle timer.
  4. `ensure_rows(upto)` with `upto + margin > row_count` → enqueue `FetchMore(read_ahead_target)` (coalesced). On execution: `Streaming` until the chunk target, then `Suspended` again (re-arm the timer) or `Complete`.
  5. Budget check before each chunk (HV-RES-07): if the next chunk would exceed the result's soft cap → pause in `Suspended` with banner "Memory limit for this result reached (N rows, X MB)" and actions **Fetch more anyway** (raises the cap for this result up to the global budget), **Export the rest to file** (doc 24 streams from the open cursor directly — no re-execution), **Close cursor**.
  6. Release (triggers above): `drain(250 ms)` → `Complete`, else `close` → `Closed{reason, rows_kept}`. The reason is shown in the result header (`cursor-closed-reason-*`).
  7. If the server kills the session while suspended (idle-in-transaction timeout, failover) → `Closed{ServerTerminated}`, rows kept, session reconnects (HV-EXEC-05 connection-lost path).
  8. The row count shown is `row_count` + "+" while not Complete (e.g. "10,000+ rows").
- **Data structures and Rust placement** (`hacivat-core::results::cursor`):
  ```rust
  pub struct CursorInfo { pub id: CursorId, pub session: SessionId, pub opened_at: Instant, pub last_fetch: Instant,
                          pub holds_tx: bool /* we opened BEGIN */, pub engine_kind: CursorKind }
  pub enum CloseReason { SessionNeeded, IdleTimeout, TabClosed, TxEnded, UserClosed, MemoryLimit, ConnectionLost, ServerTerminated, Cancelled }
  pub struct FetchPolicy { pub first_chunk: u32, pub read_ahead_rows: u32, pub read_ahead_bytes: u64,
                           pub prefetch_margin_rows: u32, pub idle_timeout: Duration, pub release_drain: Duration }
  ```
- **Per-DBMS matrix (what "Suspended" costs on the server):**

  | Engine | Suspended means | Server cost while suspended | Release |
  |---|---|---|---|
  | PostgreSQL | open portal in an open tx | snapshot (vacuum horizon), AccessShareLocks, one backend | close portal + COMMIT |
  | MySQL/MariaDB | unbuffered result not fully read | the connection is blocked; the server thread waits on the socket; possible metadata locks | KILL QUERY + drain |
  | SQL Server | TDS stream not fully read | worker and locks per isolation level (shared locks under READ COMMITTED are released per row; under REPEATABLE READ held) | attention + drain |
  | SQLite | `Statement` alive | read snapshot; blocks WAL checkpoint from completing | finalize |
  | ClickHouse | HTTP body not fully read | a running query and its memory | KILL QUERY + close |
- **Complexity / performance budget:** first paint ≤ 50 ms after the first 500 rows arrive (local DB); scroll-triggered fetch visible within ≤ 1 frame for rows already read ahead; release ≤ 300 ms p95 (250 ms drain + close); zero re-execution for scrolling.
- **Edge cases (exhaustive list):** user runs another statement on the same session (release first, banner explains); single-session mode with several consoles sharing a suspended cursor (release on any other request); user transaction committed while suspended (the portal dies → `Closed{TxEnded}`); idle timeout during an open user transaction (cursor closed, tx stays open); the user keeps scrolling past the read-ahead before the fetch returns (placeholders; one coalesced fetch); a result set that is not the last one in a multi-result unit (earlier result sets must be fully drained before the next arrives: protocol constraint → only the **last** result set of a unit can be suspended; earlier ones are drained up to the budget, then marked `Truncated{FollowingResultSets}`); sorting a suspended result (local sort only on loaded rows with a hint, or server-side re-run, HV-RES-11); export from a suspended cursor (continues from the cursor, no re-execution); MCP fetching beyond loaded rows (same `FetchMore`); app sleep/resume (idle timer uses wall time; on resume, check the connection); server failover (connection lost path).
- **Test cases (input → expected):** PG `SELECT * FROM generate_series(1, 1e7)` → 500 rows painted, read-ahead to 10,000, Suspended, status bar "Cursor open"; scroll to row 9,500 → one FetchMore → 20,000 rows; run `SELECT 1` on the same session → banner "Cursor closed after 20,000 rows (another statement needed the session)" and `SELECT 1` runs; wait 10 min idle → `Closed{IdleTimeout}`, transaction committed; MySQL 100k-row result scrolled to 30k, then tab closed → KILL QUERY + drain, the connection is reusable; a 3-result-set T-SQL batch → result sets 1 and 2 fully drained (under budget), result set 3 may be suspended; a result exactly 10,000 rows long → read-ahead reads 10,001 → Complete with an exact count.
- **i18n notes:** `cursor-open-status = Cursor open — holding a database snapshot`, `cursor-closed-reason-session-needed = Cursor closed after { $rows } rows: another statement needed the session.`, `-idle`, `-tx-ended`, `-memory`, `-connection-lost`, `rows-count-partial = { $rows }+ rows`, actions `cursor-rerun`, `cursor-rerun-fetch-all`, `cursor-export-rest`.
- **Verification status:** strategy is our decision; [Observed: verification/01 #4] WITH HOLD allowed in read-only transactions; engine costs are [Inferred: vendor docs]. **Docker test needed** for MySQL KILL+drain and MSSQL attention+drain timings, and for PG `idle_in_transaction_session_timeout` interaction.
- **Prior art and evidence:** dbeaver-04 ALG-DBV-GRID-03; datagrip/07 §3.10.
- **Used by:** Hacivat console results, MCP paged reads, exports from a live result.

### HV-RES-06: Table-editor paging: keyset/offset with a look-ahead row

- **Priority / depends on:** v1. Depends on HV-RES-04, HV-RES-11 (WHERE/ORDER composition), HV-EDIT-01 (identity columns as the tiebreaker).
- **Sources merged:** Beekeeper/TUI ALG-DATA-01 (limit+1 overflow probe, keyset with PK tiebreaker, opaque tokens, estimated totals, jump to last by reversing), datagrip/08 ALG-GRID-08 (keyset vs offset, previous via reverse, MSSQL lacks row values), dbeaver-04 ALG-DBV-GRID-03 (edge cases, `has_more` heuristic defect), datagrip/08 §3.6 (page size 500, presets, Count Rows), datagrip/13 PAGE family.
- **Problem:** Browse whole tables (opened from the explorer) fast at any depth, without COUNT(*) on open, with stable pages under concurrent writes where possible.
- **Inputs / outputs:** table, visible column list (+ hidden key columns), sort spec (user, header, or default), WHERE (raw + structured), page size `n`, cursor → rows (≤ n), `has_more`, the next cursor; optional estimated/exact totals.
- **Decision:**
  - **Infinite scroll with appended keyset pages** by default (page size 500, presets 100/500/1,000/5,000/All). A page bar (First / Previous / Next / Last / page size / Count) remains available. "Previous" is just scrolling up, because loaded pages stay in the store.
  - **Mode choice:** `Keyset` when the effective ORDER BY is a total order (append the identity columns as a tiebreaker); `Offset` when there is no identity (views without keys, heap tables without PK) — with a warning chip "unstable paging"; `Token` for engines with opaque paging state (Cassandra-like; not in v1 engines).
  - **Default order:** by identity ascending (DataGrip's "sort tables by numeric PK" default) when the user gave no ORDER BY — required for keyset; configurable to "none" (then Offset mode).
  - **Last page:** reverse the order, fetch `n`, reverse in memory; this starts a *tail segment* (row numbers shown as "last − i" until a count is known).
  - Every fetch requests `n + 1` rows.
  - Rejected: DataGrip's LIMIT/OFFSET-only paging (`database.grid.paging.offset=true`) as the default; rejected: DBeaver re-read-from-0.
- **Algorithm:**
  1. Effective sort `S = user_sort ++ identity_columns (if not already a prefix-unique)`, each with direction and **explicit NULL placement** `nulls ∈ {First, Last}` (from the user, else the engine default: PG/Oracle ASC→Last, DESC→First; MySQL/SQLite/MSSQL ASC→First, DESC→Last).
  2. First page: `SELECT <cols>, <hidden key cols> FROM <t> [WHERE (<filter>)\n] ORDER BY <S with explicit NULLS> <limit n+1>`.
  3. Next page (Keyset) with last kept row values `v`: predicate `after(S, v)` built lexicographically:
     - `after_i(v_i)` for one key: v non-NULL: ASC NULLS LAST → `(c > v OR c IS NULL)`; ASC NULLS FIRST → `c > v`; DESC NULLS FIRST → `c < v`; DESC NULLS LAST → `(c < v OR c IS NULL)`. v NULL: NULLS LAST → `FALSE` (nothing after a NULL among NULLs except via later keys); NULLS FIRST → `c IS NOT NULL`.
     - `eq_i(v_i)`: v non-NULL → `c = v`; v NULL → `c IS NULL`.
     - `after(S, v) = after_1 OR (eq_1 AND (after_2 OR (eq_2 AND … after_k)))`.
     - **Row-value fast path:** if all keys are NOT NULL, all directions are equal, and the dialect supports row-value comparison (PG, MySQL 8, SQLite ≥ 3.15), render `(c1, …, ck) > (v1, …, vk)` (or `<` for DESC) — index-friendly.
     - Values are bound parameters; `NULLS FIRST/LAST` rendered natively (PG, SQLite ≥ 3.30, Oracle) or emulated by a leading `CASE WHEN c IS NULL THEN 0 ELSE 1 END` sort key (MySQL, MSSQL).
  4. Offset mode: `… ORDER BY <S or none> LIMIT n+1 OFFSET k` per dialect syntax (MSSQL `OFFSET k ROWS FETCH NEXT n+1 ROWS ONLY` needs ORDER BY → `ORDER BY (SELECT NULL)` with an "unstable order" warning).
  5. `has_more = received > n`; drop the extra row; the cursor = the sort-key values of the last kept row (Keyset) or `k + n` (Offset).
  6. Totals: show `≈ estimate` from the catalog (HV-RES-10) on open; exact count only on request.
  7. Invalidate (new generation) when the sort, filter or visible column set changes, or after a submit that changed key values (HV-EDIT-08 decides patch vs reload).
- **Data structures and Rust placement** (`hacivat-core::results::paging`):
  ```rust
  pub enum PageCursor { Keyset { last: Arc<[Value]> }, Offset(u64), Token(Arc<[u8]>), Start, Tail }
  pub struct SortKey { pub col: ColumnRef, pub desc: bool, pub nulls: NullsPos }
  pub fn keyset_predicate(keys: &[SortKey], last: &[Value], d: &dyn SqlDialect) -> (SqlFragment, Vec<BindValue>);
  ```
- **Per-DBMS matrix:**

  | Engine | Limit syntax | Row values | NULLS FIRST/LAST | Notes |
  |---|---|---|---|---|
  | PostgreSQL | `LIMIT n+1 [OFFSET k]` | yes | yes | |
  | MySQL 8 / MariaDB | `LIMIT n+1 [OFFSET k]` | yes (`(a,b) > (x,y)`) | emulate | index use of row values: verify per version [Inferred] |
  | SQL Server | `OFFSET k ROWS FETCH NEXT n+1 ROWS ONLY` (ORDER BY required) or `TOP (n+1)` | no → OR chain | emulate | |
  | SQLite | `LIMIT n+1 [OFFSET k]` | ≥ 3.15 | ≥ 3.30 | |
  | ClickHouse | `LIMIT n+1 [OFFSET k]` | tuples `(a,b) > (x,y)` | `NULLS FIRST/LAST` | no PK uniqueness guarantee → Offset or ORDER BY all sort-key columns |
  | Oracle (later) | `FETCH FIRST n+1 ROWS ONLY` (12c+) | no | yes | |
- **Complexity / performance budget:** keyset page O(log N + n) with a matching index; offset O(k + n). Budget: next page ≤ 50 ms server time on an indexed 100M-row PG table at any depth (keyset); the predicate builder ≤ 50 µs.
- **Edge cases (exhaustive list):** table without identity (Offset + warning, editing read-only); sort on nullable columns (NULL-aware predicate); sort on expressions or with a collation (values compared server-side only; never filter client-side); duplicates in the sort key (identity tiebreaker); concurrent inserts/deletes (keyset stable; offset may skip/duplicate — documented); views and materialized views (offset unless a virtual key exists); SQL Server without explicit ORDER BY; page size 0 (invalid → 1); last page exactly n rows (the probe proves there is no next); empty table; very wide rows (reduce n so that `n × est_row_bytes ≤ 16 MiB`); LOB columns (server-side prefix, HV-RES-08); filters with parameters (always bound); time-zone-sensitive sort keys (`timestamptz` compared server-side); Turkish collation (`tr-TR` orders `ı` before `i`; the server decides — verified orders in verification/01 #39–#40); float sort keys with NaN (PG sorts NaN after +Infinity; the predicate binds exact values); a key value edited by the user then submitted (the cursor uses the server's new value after reconciliation; reload if the edited row moves out of the loaded window); ClickHouse ReplacingMergeTree duplicates before merge (FINAL is not added automatically; warn).
- **Test cases (input → expected):** 250 rows, n = 100, Keyset → pages 100/100/50 with has_more true/true/false; exactly 200 rows → page 2 has_more false (the probe returned 100); sort by `last_name` (non-unique) → predicate on `(last_name, id)`; `created_at DESC NULLS LAST` with NULLs → the next page continues into the NULL block: after the last non-NULL `v` the predicate is `(created_at < v OR created_at IS NULL) OR (created_at = v AND id > $id)`, and after a NULL `v` it is `(created_at IS NULL AND id > $id)`; SQL Server without a key → `ORDER BY (SELECT NULL)` + warning; Last on a 1M-row PG table → `ORDER BY id DESC LIMIT 501`, reversed, tail segment; PAGE-01…13 (doc 13) as differential cases.
- **i18n notes:** `paging-unstable-warning = This table has no unique key; rows may repeat or be skipped while paging.`, `paging-rows-loaded = { $rows } rows loaded`, `paging-estimate = ≈ { $count }`. Counts formatted with the locale's grouping (display only).
- **Verification status:** [Observed: verification/01 #39–#40] Turkish collation orders in PG ICU `tr-TR` and MySQL `utf8mb4_tr_0900_ai_ci` (server-side ordering is the reference). DataGrip uses OFFSET [Observed: registry]; PAGE family not yet run.
- **Prior art and evidence:** Beekeeper (behaviour) [Observed: `TableTable.vue:2013-2100`]; harlequin `detect_overflow` (MIT); Winand "Use the index, Luke" [Inferred: general knowledge].
- **Used by:** Hacivat table editor; MCP `read_table` (paged); Karagöz (max rows per request).

### HV-RES-07: Memory budget, fetch-all and spill

- **Priority / depends on:** v1 (accounting, caps, fetch-all confirmation, "export the rest"); v1.x (spill to disk). Depends on HV-RES-02.
- **Sources merged:** dbeaver-04 ALG-DBV-GRID-03 recommendation (memory cap per result ~256 MB, "export instead"), dbeaver-05 §3.13/§5.4 (OOM issues, memory budget with spill and a "showing first N rows" banner), ui-01 ALG-UI-01 (spill to on-disk file with a batch index), datagrip/08 §3.6 ("All" page size), dbeaver-04 §3.5 (fetch-all with confirmation).
- **Problem:** Never let results exhaust memory, while letting users see as much as they ask for and offering a non-memory path for "all rows".
- **Inputs / outputs:** chunk sizes, settings, system memory → admission decisions for fetches; eviction/spill of chunks; banners.
- **Decision:**
  - **Per-result soft cap** default 256 MiB; **global results budget** default `min(25% of physical RAM, 4 GiB)`, configurable. Accounting is exact per sealed chunk (buffer lengths) and estimated for the tail.
  - **Fetch-all** ("All" page size or "Fetch all rows") asks for confirmation when `estimated_total × avg_row_bytes > remaining per-result cap` (estimate from HV-RES-10, else unknown → always ask). The dialog offers **Export to file** (streams from the open cursor, no store) as the recommended alternative.
  - **When the global budget is exceeded:** evict sealed chunks of results that are not visible and not pinned, least recently viewed first. In v1 an evicted result keeps its schema and the first chunk and shows "Data released to free memory — Re-run". In v1.x, chunks are **spilled** to a per-result temporary file (zstd-compressed, own chunk format) and re-read on demand with an LRU of decoded chunks.
  - Rejected: row-count caps (meaningless for wide rows); rejected: unbounded fetch-all (DBeaver #34854).
- **Algorithm:**
  1. `admit(result, next_chunk_estimate)`: if `result.bytes + est > result.cap` → `Pause(ResultCap)`; if `global.bytes + est > global.budget` → try `evict(est)`; if still insufficient → `Pause(GlobalBudget)`.
  2. `evict(need)`: candidates = sealed chunks of results not visible in any window for ≥ 30 s and not pinned and not holding pending edits (doc 23); order by last view time; v1: drop chunks (keep chunk 0); v1.x: write to spill and replace with `ChunkRef::Spilled{offset, len}`.
  3. Reading a spilled chunk (v1.x): synchronous read from the page cache is too slow for painting → the grid paints placeholders and the source loads the chunk on a background thread; the decoded chunk enters an LRU of 64 MiB.
  4. Fetch-all: confirm per rule above; then raise the result's cap to the global budget minus others' usage; stream; if still too big → pause with banner.
  5. Banner actions: **Fetch more anyway**, **Export the rest to file**, **Close cursor**.
- **Data structures and Rust placement** (`hacivat-core::results::budget`): `pub struct MemoryBudget { global: AtomicU64, limit: u64, per_result_default: u64 }`, `pub enum Admission { Ok, Pause(PauseReason) }`, `pub enum ChunkRef { Hot(Arc<Chunk>), Spilled { file: Arc<SpillFile>, off: u64, len: u32 } }` (v1.x).
- **Complexity / performance budget:** admission O(1); eviction O(results × chunks) worst case, amortized; spill write ≥ 200 MB/s (zstd level 1); spilled chunk reload ≤ 20 ms for a 4,096-row chunk.
- **Edge cases (exhaustive list):** a single cell larger than the per-result cap (HV-RES-08 truncation applies first); many small results (global budget); a result with pending edits (never evicted; edits keep originals in the overlay anyway); the visible result itself exceeding the budget (pause with banner; never evict visible chunks); spill directory not writable or disk full (fall back to v1 behaviour, warn once); app crash (spill files in a per-session temp dir cleaned at next start); OS memory pressure notifications (macOS/Windows) → shrink budget by 25% temporarily; 32-bit platforms (not supported); MCP requesting a large page (served from the store or streamed; MCP has its own per-call row/byte caps).
- **Test cases (input → expected):** a 1M-row result of 400-byte rows (≈ 400 MB) with a 256 MiB cap → pauses at ≈ 640k rows with the banner; "Export the rest" → the remaining rows go to the file from the same cursor; two background results using 3.5 GiB and a new visible result needing 1 GiB with a 4 GiB budget → background chunks evicted (v1) or spilled (v1.x); fetch-all with unknown estimate → confirmation dialog.
- **i18n notes:** `budget-result-cap = Memory limit for this result reached ({ $rows } rows, { $size }).`, `budget-released = Data was released to free memory.`, `fetch-all-confirm = Fetching all rows may need about { $size } of memory.` Sizes formatted with ICU4X units (MB/GB; Turkish uses `MB`/`GB` with comma decimals).
- **Verification status:** design decision; DBeaver OOM issues [Observed: verification/02 S27].
- **Prior art and evidence:** dbeaver-04 §3.5; dbeaver-05 §5.4.
- **Used by:** Hacivat; Karagöz uses its own per-request caps.

### HV-RES-08: LOB truncation and lazy load

- **Priority / depends on:** v1. Depends on HV-RES-02, -04, HV-EDIT-01 (identity for re-fetch), HV-EXEC-05 (side vs main session).
- **Sources merged:** datagrip/08 ALG-GRID-09 and §3.7 ("{0} of {1}", partial-copy warning, clone refusal, Save LOB, Change Size Limit → Apply and Reload), dbeaver-04 §3.14 (CLOB < 10,000 chars in memory, larger to temp file; LOB keys must hold originals), ui-01 ALG-UI-01 step 5 and edge 8 (`max_cell_bytes`, truncated bit, `CellRequest`), dbeaver-05 ALG-DBV-XFER-02 edge 18 (never read a 1 GB CLOB into a string during export).
- **Problem:** Keep huge text/binary/JSON values from dominating memory and transfer time, while making the full value available for viewing, editing, copying and exporting.
- **Inputs / outputs:** per-column logical type, settings → cells stored as full values or `Truncated{prefix, total_len}`; full-value requests → values loaded on demand.
- **Decision:**
  - **Table editor (we write the SELECT):** for LOB-like columns (text without length limit, `bytea`, BLOB/CLOB, JSON, XML, `nvarchar(max)`, `varbinary(max)`), select a server-side prefix plus the total length: PG `substring(c from 1 for N)`, `octet_length(c)` (bytes) / `char_length(c)`; MySQL `LEFT(c, N)`/`SUBSTRING`, `LENGTH`/`CHAR_LENGTH`; MSSQL `SUBSTRING(c, 1, N)`, `DATALENGTH`; SQLite `substr(c, 1, N)`, `length(c)` (bytes for blobs, chars for text). Default `N = 16 KiB` (chars for text, bytes for binary).
  - **Console results (user SQL):** the server sends full values; keep at most `max_value_bytes = 1 MiB` per cell in the store and record the total length (client-side truncation). In v1.x, values above the limit are kept whole in the result's spill file instead, so no re-fetch is needed.
  - **Full value** = re-fetch by identity `SELECT c FROM t WHERE <key> = …` (side session or main session per HV-EXEC-05) when the result has identity; otherwise "value truncated at 1 MiB — raise the limit and re-run" (and in v1.x the spill copy).
  - **Exports never truncate** (doc 24 re-executes or streams with truncation disabled and writes LOBs streaming).
  - **Editing a truncated value** first loads the full value (HV-EDIT-03); cloning a row with a truncated cell loads it or refuses (DataGrip refuses; we load when identity exists).
  - Rejected: DataGrip's 1,024-byte default (too small for text browsing).
- **Algorithm:**
  1. Classify columns as LOB-like by logical type and declared length (`Text{max_chars: None | > 4,000}`, Bytes, Json, Xml).
  2. Table editor SELECT rewriting adds `prefix(c, N) AS c, length(c) AS hv_len_c` per LOB-like column (hidden length columns).
  3. Decode: if `len > prefix_len` → `Truncated{prefix, total_len, unit}`; truncation of text prefixes ends at a char boundary (PG `substring` on text is char-based; for bytes-based truncation on the client, back off to a UTF-8 boundary).
  4. Render "prefix… (12.4 MB)" via HV-GRID-10; the value viewer shows "Showing 16 KiB of 12.4 MB — Load full value".
  5. `request_full_value(row, col)` → build the re-select by key, run it, stream large values into a temp file when > 64 MiB (the viewer pages through the file), return `Value` or `LobHandle`.
  6. Copy of a truncated cell → load the full value with progress if identity exists, else copy the prefix and warn "Copied partially".
  7. "Change size limit" → setting change + **Apply and Reload** (re-run the table query with the new N).
- **Data structures and Rust placement:** `hacivat-core::results::lob { LobPolicy, LobHandle (temp file + length + kind), fetch_full_value() }`; the rewrite of table-editor SELECTs in `hacivat-core::results::rewrite`.
- **Per-DBMS matrix:** prefix and length functions as listed in the Decision; ClickHouse `substring`/`length` (bytes); MongoDB: large strings in documents → `$substrBytes` in a projection is possible but not used in v1 (documents ≤ 16 MiB; client-side truncation).
- **Complexity / performance budget:** table-editor page transfer bounded by `rows × (N + small)`; full-value fetch ≤ 100 ms + transfer time for values ≤ 10 MB on a LAN; viewer pages through temp files at ≥ 100 MB/s.
- **Edge cases (exhaustive list):** multibyte truncation on a char boundary; image header sniffing for the viewer (prefix ≥ 64 bytes is enough for magic numbers; full load for display); truncated JSON (no pretty-print; "load full to format"); unknown total length (driver cannot tell; show "≥ n"); export must bypass truncation; key column that is itself a LOB (never use as identity; HV-EDIT-01); values changed on the server between the prefix read and the full read (the full value shown is newer; the edit conflict check catches writes, HV-EDIT-05); a result without identity (no re-fetch; v1.x spill); NULL LOBs (no length query cost: `length(NULL)` is NULL); PG TOAST compression (`octet_length` returns the uncompressed length; fine); MySQL `max_allowed_packet` smaller than the value (server error on full fetch → shown verbatim); MSSQL `TEXTSIZE` silently truncating `text`/`ntext`/`(n)varchar(max)` values (send `SET TEXTSIZE 2147483647` on connect so that only our own truncation applies); SQLite blobs larger than 1 GB (SQLite max is 1 GB by default).
- **Test cases (input → expected):** PG table `docs(id pk, body text)` with a 5 MB body, N = 16 KiB → the cell shows the prefix and "5.0 MB"; the value viewer "Load full value" → `SELECT body FROM docs WHERE id = $1`; console `SELECT repeat('x', 3000000)` → stored prefix 1 MiB, total 3,000,000, no identity → viewer says "raise the limit and re-run"; copy of a truncated cell with identity → full value copied after a progress toast; PAGE-12 (doc 13: max bytes 4) differential.
- **i18n notes:** `lob-truncated = { $shown } of { $total }`, `lob-load-full = Load full value`, `lob-copied-partially = Only the first { $shown } were copied. Raise the size limit to copy the full value.`, `lob-no-identity = This result has no row identity; re-run with a larger size limit to see the full value.`
- **Verification status:** DataGrip behaviour [Observed: bundle "{0} of {1}", clone refusal, Change Size Limit]; DBeaver thresholds [Observed: `JDBCContentCLOB.java:88-136`; some preference keys unused — dbeaver-04 §3.14 Inferred]. Defaults are our decision.
- **Prior art and evidence:** datagrip/08 §3.7; dbeaver-04 §3.14.
- **Used by:** Hacivat grid, value viewer, copy, edit; MCP (`max_cell_bytes` parameter on fetch tools).

### HV-RES-09: Column origin resolution

- **Priority / depends on:** v1 for PostgreSQL, MySQL/MariaDB, SQLite; v1.x for SQL Server. Depends on HV-RES-04 (protocol metadata), the catalog cache, `hacivat-sql` resolver (parser fallback).
- **Sources merged:** dbeaver-04 ALG-DBV-GRID-05 and §3.8 (driver metadata + parser + catalog; never bind expressions; type sanity check; view vs base table), datagrip/08 §3.5 and ALG-GRID-05 edge "result aliases → base columns (PG RowDescription OID/attnum, MySQL org_table/org_name, MSSQL `sp_describe_first_result_set`)".
- **Problem:** For an arbitrary SELECT, map each output column to `(range variable, table, column)` so that results can be edited, navigated through FKs and formatted with table metadata — without guessing by name.
- **Inputs / outputs:** protocol column metadata, the parsed query (when available), the catalog cache → per column `Option<ColumnOrigin>` with a confidence and a read-only reason when absent.
- **Decision:** **protocol first, parser second, never guess by name alone.** Range variables (aliases) are resolved by the parser so that self-joins do not share an identity. When the query targets a view and the protocol reports a base table, the column is **not editable by default** (the user can enable "edit through view" per view).
- **Algorithm:**
  1. Protocol origin per engine:
     - PG: RowDescription `table_oid`, `column_attnum` (0 = expression) → catalog lookup `pg_class`/`pg_attribute` (cached; refresh on miss).
     - MySQL/MariaDB: column definition `schema`, `org_table`, `org_name` (`table`/`name` are aliases).
     - SQLite: `sqlite3_column_database_name/table_name/origin_name` (rusqlite feature `column_metadata`).
     - SQL Server: lazily on the first edit attempt, `sp_describe_first_result_set @tsql, NULL, 1` (browse information mode) on a side connection → `source_schema`, `source_table`, `source_column`, `is_part_of_unique_key`.
     - ClickHouse: none → read-only.
  2. Parser pass (if the unit parses): list FROM range variables with their resolved tables; for each select item that is a plain column reference, record `(range_var, column)`; expand `*` and `t.*` in catalog column order to align positions.
  3. Merge: protocol table + parser range variable at the same position; if the parser shows the item is an expression or aggregate → `None(Expression)` even if the protocol (incorrectly) reports a table; if two range variables reference the same table (self-join) and the protocol cannot disambiguate → use the parser's range variable; if the parser failed → `ambiguous` for duplicated tables → read-only for those columns.
  4. View check: if the parser's range variable is a view and the protocol reports a base table → `origin = view column` (no editing unless enabled).
  5. Type sanity: if the result column's logical type differs from the catalog column's (e.g. alias collision with a different type) and the confidence is Parser-only → drop the origin.
  6. Attach `ColumnOrigin` to the result schema. HV-EDIT-01 computes identity per range variable.
- **Data structures and Rust placement** (`hacivat-core::results::origin`):
  ```rust
  pub struct ColumnOrigin { pub range_var: Option<Arc<str>>, pub table: TableId, pub column: ColumnId,
                            pub confidence: Confidence /* Protocol | ProtocolAndParser | Parser */, pub via_view: Option<TableId> }
  pub enum NoOrigin { Expression, Aggregate, SetOperation, Function, Ambiguous, ViewNotEditable, Unknown }
  ```
- **Per-DBMS matrix:** listed in step 1. Notes: PG reports base-table origins through simple views [Inferred: verify]; MySQL reports the base table for view columns [Observed: dbeaver-04 §3.8 code comment].
- **Complexity / performance budget:** O(columns) with cached catalog lookups; ≤ 5 ms after `ResultSetStart` (never delays first paint; editing is enabled when done). MSSQL lookup ≤ 50 ms on first edit.
- **Edge cases (exhaustive list):** expressions, aggregates, DISTINCT, GROUP BY, window functions, UNION (no origin); `SELECT *, now()` (star expansion alignment); quoted vs unquoted names and server case folding (PG lower, Oracle upper, MySQL OS-dependent `lower_case_table_names`; SQL Server Turkish collations make `ID` ≠ `id` — always use catalog spelling, verified); self-join (range variables); subqueries/CTEs (origin through derived tables only via the resolver; protocol may still report it for simple derivations); table functions (`generate_series`, `unnest`) → none; temporary tables missing from the catalog cache (refresh on miss); a column selected twice (two result columns, one origin — HV-EDIT keeps them in sync); a dropped/renamed column after fetch (re-validated on submit); RLS-protected tables (origin fine; edits may affect 0 rows → HV-EDIT-06); foreign tables (editable only if the FDW supports it — server error otherwise); `RETURNING` results (origin from protocol, but the result is not re-queryable; editing allowed only through identity); PG `oid`/`ctid` system columns (attnum < 0 → system column, never editable).
- **Test cases (input → expected):** PG `SELECT u.id, u.name AS n FROM users u` → origins users.id, users.name (Protocol+Parser); `SELECT id + 1 FROM users` → `None(Expression)`; MySQL `SELECT * FROM v_users` → origin via view, not editable by default; `SELECT a.id, b.id FROM t a JOIN t b ON a.p = b.id` → two origins with range vars `a` and `b`; SQL Server `SELECT TOP 10 * FROM dbo.Orders` → origins after `sp_describe_first_result_set` on the first edit; SQLite `SELECT name FROM t` with `column_metadata` → origin t.name.
- **i18n notes:** read-only reasons are Fluent ids: `ro-reason-expression`, `-aggregate`, `-no-origin`, `-ambiguous`, `-view-not-editable`, `-no-identity` (HV-EDIT-01). Shown in header tooltips.
- **Verification status:** [Observed: verification/01 #42] SQL Server `Turkish_CI_AS` makes `information_schema.tables` and `id` vs `ID` fail — identifiers must be emitted exactly as cataloged. PG view behaviour [Inferred: verify]. DBeaver logic [Observed: `DBExecUtils.java:759-985`].
- **Prior art and evidence:** dbeaver-04 §3.8; datagrip/08 §3.5.
- **Used by:** HV-EDIT-01, HV-RES-12 (FK navigation from query results), HV-GRID-10 (column header badges).

### HV-RES-10: Row counting (estimate on open, exact on demand)

- **Priority / depends on:** v1. Depends on HV-RES-04, catalog cache.
- **Sources merged:** Beekeeper/TUI ALG-DATA-01 step 4 (estimated total labelled "≈", exact only on request with timeout), datagrip/08 §3.6 (Count Rows → `SELECT COUNT(*)`, "of N", "Querying…"), dbeaver-04 §3.5 (`SQLQueryTransformerCount` wrapping, removing ORDER BY, refusing GROUP BY in rewrite mode), dbeaver-05 #3568 (never COUNT on every open).
- **Problem:** Tell users how big a table or result is without paying a full scan on every open.
- **Inputs / outputs:** table or Read-class query → `RowCount { estimate: Option<u64>, exact: Option<u64>, loaded: u64, complete: bool }`.
- **Decision:** estimates on open for tables (catalog statistics); exact counts only on explicit "Count rows" (cancellable, statement timeout default 30 s, side session unless manual tx); when a result is fully fetched, the exact count is the loaded count. For queries, the count wraps the query: `SELECT COUNT(*) FROM (<query without top-level ORDER BY>) hv_c` (Read class only).
- **Algorithm:**
  1. Table open: estimate from PG `pg_class.reltuples` (−1 → unknown; for partitioned tables sum children), MySQL `information_schema.TABLES.TABLE_ROWS` (InnoDB estimate), MSSQL `SUM(rows) FROM sys.partitions WHERE index_id IN (0,1)`, SQLite `sqlite_stat1` if present else none, ClickHouse `system.tables.total_rows`.
  2. Query results: optional estimate from `EXPLAIN` plan rows (PG/MySQL) — off by default (costs a round trip).
  3. Count rows: strip a trailing top-level `ORDER BY` (keep it if `LIMIT`/`TOP`/`FETCH` present at the same level), wrap, run with a timeout and cancel button; "Counting…" in the status bar.
  4. Display: "500 of ≈ 1.2M", "500 of 1,234,567", "10,000+ rows" while streaming, "1,234,567 rows" when complete.
- **Data structures and Rust placement:** `hacivat-core::results::count { estimate(), exact_count_sql() }`.
- **Complexity / performance budget:** estimate ≤ 10 ms (catalog cache hit) on open; exact count is a server cost, cancellable at any time.
- **Edge cases (exhaustive list):** stale statistics (estimate labelled "≈" with a tooltip "from statistics"); PG `reltuples = -1` (never analyzed); MySQL InnoDB estimates off by ±50%; partitioned tables; views (no estimate); queries with side effects (no count); queries ending with `;` or a trailing comment (strip; newline before the closing parenthesis); `SELECT … FOR UPDATE` (Lock class, not Read: the count is refused with a hint instead of silently dropping the locking clause); MSSQL CTE queries (hoist `WITH` before `SELECT COUNT(*)`); counts > 2^53 (u64 display); count while a filter changes (cancel the previous count).
- **Test cases (input → expected):** PG table with reltuples 1.2e6 → "≈ 1.2M" on open, no COUNT issued; Count rows → `SELECT COUNT(*) FROM "public"."t"`; console `SELECT * FROM t ORDER BY a` → count SQL `SELECT COUNT(*) FROM (SELECT * FROM t\n) hv_c`; `SELECT * FROM t ORDER BY a LIMIT 10` → ORDER BY kept; PAGE-04 (doc 13) differential.
- **i18n notes:** `count-estimate = ≈ { $count }`, `count-of = { $loaded } of { $total }`, `count-counting = Counting…`. Compact notation (`1.2M` / Turkish `1,2 Mn`) via ICU4X compact decimal formatting when available [Inferred: ICU4X experimental compact formatting — else full digits].
- **Verification status:** DataGrip Count Rows [Observed: help]; DBeaver transformer [Observed: `SQLQueryTransformerCount.java:43-149`].
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat; MCP `describe_table` (estimate only).

### HV-RES-11: Server-side filter and sort rewriting

- **Priority / depends on:** v1. Depends on `hacivat-sql` (expression parser for validation), HV-RES-04, -06.
- **Sources merged:** datagrip/08 ALG-GRID-07 (wrap in SELECT, user text in parentheses followed by a newline, validate as expression, Oracle no `AS`, MSSQL rules), dbeaver-04 ALG-DBV-GRID-04 (injection via parser vs wrapping; duplicate labels; NULL semantics; `= ANY($1)` arrays; edge cases), dbeaver-04 §3.11 (SMART ordering: local when fully loaded, else server), datagrip/08 §3.13–3.14 (WHERE/ORDER BY fields with history; "Sort via ORDER BY" setting; numeric PK default sort).
- **Problem:** Apply grid filters and sorting on the server for tables and for arbitrary query results, predictably and safely.
- **Inputs / outputs:** base (table or query text), `Filter { chips: Vec<Predicate>, raw_where: Option<String>, order: Vec<SortKey>, raw_order: Option<String> }`, dialect → `(sql, binds)` or a validation error.
- **Decision:**
  - **Tables:** build `SELECT <cols> FROM <t> [WHERE (<chips AND raw>)] ORDER BY …` with bound values for chips; raw WHERE/ORDER text inserted verbatim (it is the user's own SQL) after validation.
  - **Query results:** **wrap by default** (`SELECT * FROM (\n<q>\n) hv_q WHERE … ORDER BY …`) — "filter what you see", including after LIMIT/aggregation. Rejected: DBeaver's JSQLParser injection (re-prints the user's SQL, loses comments, changes semantics before/after LIMIT). An explicit "push filter into query" action may come later via our lossless CST rewrite.
  - Only Read-class single-statement results can be wrapped (re-execution must be side-effect free). Otherwise sorting is local only (on loaded rows, with a hint) and filtering is local only (HV-GRID-08).
  - **Header-click sort:** table editor → server ORDER BY (setting "Sort via ORDER BY", default on); console result → local if `Complete`, else server re-run via wrapping (Read class) or local-on-loaded with a hint (non-Read).
- **Algorithm:**
  1. Validate `raw_where` by parsing it as a boolean expression with `hacivat-sql` for the dialect: reject statement separators at depth 0, statement keywords at depth 0 (`SELECT` outside parentheses, `INSERT`, `;`), unbalanced parentheses → `filter-invalid` (DataGrip "Invalid filter criteria"). Same for `raw_order` as an ORDER BY list.
  2. Render chips (HV-RES-12) as bound predicates; combine with `AND`; `raw_where` appended as `AND (<raw>\n)` (the newline protects against a trailing `--` comment).
  3. Tables: column references are quoted catalog names; aliases are not used.
  4. Query results: strip one trailing `;` and trailing whitespace; wrap with a newline before `)`; alias `hv_q` (Oracle: no `AS`); reference result columns by their **labels**, quoted; if a label is duplicated (`a.id`, `b.id`) → WHERE on it is refused ("Ambiguous column name in result"), ORDER BY falls back to the ordinal position.
  5. MSSQL specifics: a top-level `ORDER BY` inside the derived table without `TOP`/`OFFSET` is invalid → append `OFFSET 0 ROWS` to the inner query (keeps its semantics) or drop the inner ORDER BY when an outer ORDER BY is applied; a leading `WITH` (CTE) cannot sit inside a derived table → hoist: `WITH … SELECT * FROM (<body>) hv_q …`.
  6. NULL semantics: `=` with NULL → `IS NULL`; `NOT IN (…, NULL)` → `NOT IN (…non-null…) AND col IS NOT NULL`; IN with NULL → `(col IN (…) OR col IS NULL)`.
  7. Case-insensitive text predicates: PG `ILIKE`; others `LOWER(col) LIKE LOWER(?)` — with a tooltip that Turkish `İ/ı` behaviour depends on the column collation (verified: `'ITEMS' ILIKE 'items'` is false in a `tr-TR` ICU database).
  8. Execute as a new generation; history of WHERE/ORDER texts per table (`filter_history_size` default 30).
- **Data structures and Rust placement** (`hacivat-core::results::rewrite`):
  ```rust
  pub struct Filter { pub chips: Vec<Predicate>, pub raw_where: Option<String>, pub order: Vec<SortKey>, pub raw_order: Option<String>, pub combinator: Combinator }
  pub enum Predicate { Cmp { col: ColumnRef, op: CmpOp, value: Value }, In { col: ColumnRef, values: Vec<Value>, negate: bool },
                       Null { col: ColumnRef, negate: bool }, Like { col: ColumnRef, pattern: String, ci: bool, negate: bool },
                       RowIn { cols: Vec<ColumnRef>, tuples: Vec<Vec<Value>> } }
  pub trait SqlDialect { fn quote_ident(&self, s: &str) -> String; fn wrap_query(&self, q: &str) -> Result<WrappedQuery, RewriteError>;
                         fn limit(&self, n: u64, offset: Option<u64>) -> LimitClause; fn nulls(&self, pos: NullsPos, desc: bool) -> NullsRender; … }
  ```
- **Per-DBMS matrix:**

  | Engine | Wrap alias | Inner ORDER BY | CTE wrapping | Case-insensitive | Large IN |
  |---|---|---|---|---|---|
  | PostgreSQL | `AS hv_q` | allowed | allowed inside derived table | `ILIKE` (collation-dependent for `İ`) | `= ANY($1)` array |
  | MySQL/MariaDB | `AS hv_q` | allowed (ignored without LIMIT) | allowed (8.0+) | depends on collation (`_ci`) | chunked IN (≤ 1,000 per list, OR-ed) |
  | SQL Server | `AS hv_q` | needs TOP/OFFSET → add `OFFSET 0 ROWS` | hoist WITH | collation (`_CI_`) | TVP (later) or chunked IN; ≤ 2,100 params |
  | SQLite | `AS hv_q` | allowed | allowed | `LIKE` ASCII-only → client-side note | `IN (…)` up to `SQLITE_MAX_VARIABLE_NUMBER` |
  | Oracle (later) | `hv_q` (no AS) | allowed | allowed | `UPPER()` both sides | ≤ 1,000 per IN |
  | ClickHouse | `AS hv_q` | allowed | allowed | `ilike` | `IN` arrays |
- **Complexity / performance budget:** validation + rendering ≤ 2 ms; the server cost is the re-run.
- **Edge cases (exhaustive list):** trailing `;` or comment without newline (handled by step 4 and the newline); query already containing ORDER BY + LIMIT (wrapping filters the limited set — intended, documented); duplicate column names; expressions as columns (filterable only through wrapping, which works because labels are columns of `hv_q`); window functions/aggregates (wrapping is correct); UNION/INTERSECT (wrapping works); procedure calls and non-Read statements (not wrappable → local only); `FOR UPDATE` queries (Lock class → not wrappable); dialect syntax our parser cannot validate (raw text passed with a warning "not validated" instead of refusing, since the server validates); bind-like `?` in raw WHERE (treated as text, warning); literal rendering needs dialect syntax (HV-EDIT-09 renderer) only in the visible WHERE text; very large IN lists (arrays/chunking); NULL ordering differences (explicit NULLS); ORDER BY ordinal unsupported in some dialects (labels instead); composite fields `(col).field` in PG; Turkish `İ` in `LOWER()` depends on collation (MySQL `LOWER` is not Turkish-tailored — verified); SQL Server `Turkish_CI_AS` identifier case sensitivity (quote catalog spelling — verified).
- **Test cases (input → expected):** FILTER-01 `city = 'Ankara'` on a table → `SELECT … FROM "public"."customers" WHERE (city = 'Ankara'\n) ORDER BY "id"`; FILTER-02 `id > 1 -- x` → the newline keeps `)` outside the comment; console `SELECT a, b FROM t ORDER BY a`, filter `b > 5`, order `a DESC` → `SELECT * FROM (\nSELECT a, b FROM t ORDER BY a\n) AS hv_q WHERE (b > 5\n) ORDER BY "a" DESC`; MSSQL `WITH x AS (SELECT 1 AS a) SELECT a FROM x ORDER BY a` → `WITH x AS (SELECT 1 AS a) SELECT * FROM (SELECT a FROM x ORDER BY a OFFSET 0 ROWS) AS hv_q ORDER BY [a]`; `SELECT a.id, b.id …` + WHERE on `id` → refused (ambiguous); `IN (1, NULL, 3)` → `(c IN (1, 3) OR c IS NULL)`; raw WHERE `1=1; DROP TABLE t` → refused (statement separator).
- **i18n notes:** `filter-invalid = Invalid filter: { $detail }`, `filter-not-validated`, `filter-ambiguous-column = Column { $name } appears more than once in the result.`, `sort-local-hint = Sorted locally — the database might order differently.`, `filter-ci-collation-note`.
- **Verification status:** [Observed: verification/01 #39, #40, #42] Turkish ILIKE/LOWER/identifier behaviours. DBeaver parser tests [Observed: `SQLSemanticProcessorLineCommentsTest.java:80-158`] are reused as wrapping-safety vectors. DataGrip wrapping [Inferred in datagrip/08].
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat grid; MCP `read_table {where, order}` (structured chips only; raw text only with an explicit flag); Karagöz evaluates policy on the final rendered SQL.

### HV-RES-12: Filter-by and related-rows predicate builder

- **Priority / depends on:** v1 (filter-by cell values, NULL, LIKE, placeholder); v1.x (related rows incl. virtual FKs, multi-target popups). Depends on HV-RES-11, HV-RES-09 (origins), HV-EDIT-09 (dialect literal renderer).
- **Sources merged:** datagrip/08 ALG-GRID-14 (templates, operands from cells/clipboard/NULL/placeholder, IN lists, NULL handling, literal rendering, combining with old WHERE), dbeaver-04 ALG-DBV-GRID-08 (FK navigation, DNF for composite keys, dedupe, NULL tuples), datagrip/08 §3.20 (Related Rows: navigate-and-filter-all default), datagrip/13 FILTER-06…12.
- **Problem:** Turn selected cells into correct filter predicates, and selected rows into "open the related table filtered to the related rows" for single and composite foreign keys.
- **Inputs / outputs:** selected cells (column, value, type) or rows, template (`= <> < <= > >= LIKE NOT LIKE IS NULL IS NOT NULL`), operand source, FK definition (or virtual FK) → filter chips (+ the visible WHERE text) and/or a navigation target.
- **Decision:** predicates are **structured chips** (bound values, shown as removable chips above the grid) rather than text spliced into the WHERE field; the WHERE field stays the user's raw text. A "Convert to text" action renders chips into the WHERE text with dialect literals (DataGrip-like editing). Composite multi-row FK navigation uses row-value `IN` where supported, DNF otherwise. NULL tuples are skipped (MATCH SIMPLE semantics).
- **Algorithm:**
  1. Group selected cells by column; collect distinct values per column (exact stored values, not display strings — float/timestamp precision).
  2. Per column: `=` with one value → `Cmp(=)`; with > 1 values → `In`; `<>` with > 1 → `In{negate}`; comparisons use the first value (disabled for multi-value selections); NULL among values → `Null` or `(In OR Null)`; `LIKE` → pattern = value as is ("exact" variant escapes `%`/`_` with `ESCAPE '\'`).
  3. Unsupported equality types (json, arrays, geometry, LOBs) → offer the dialect's text cast (`col::text = $1` in PG) or disable with a tooltip; truncated LOBs → refuse.
  4. Combine chips with AND; columns combine with AND, values within a column with OR (via IN).
  5. Placeholder operand → insert a chip with an empty value editor focused.
  6. Related rows: for FK `(c1..ck) → T(p1..pk)` and selected source rows: collect tuples of `(c1..ck)` values, drop tuples containing NULL, dedupe; target filter = `p = v` (one tuple, k = 1), `p IN (…)` (many, k = 1), `(p1..pk) IN ((…),(…))` (k > 1, row values supported), else DNF `(p1 = a AND p2 = b) OR (…)`. Reverse navigation (referencing rows) swaps roles. Large tuple sets: PG arrays/`unnest` join, else chunks ≤ 1,000 tuples.
  7. Navigation behaviour: default "navigate and filter all" (DataGrip); open in the same tab with back/forward history, or a new tab (setting).
- **Data structures and Rust placement:** `hacivat-core::results::rewrite::{Predicate, filter_by(), related_rows()}`; virtual FKs from the per-connection virtual model (HV-EDIT-01 store).
- **Complexity / performance budget:** O(selected cells); building 10k-tuple IN lists ≤ 10 ms; never block the UI.
- **Edge cases (exhaustive list):** values with quotes/newlines (bound, so safe; literal rendering escapes them); very long values; JSON/array/geometry columns; floats (exact stored value); timestamps with TZ (bound typed); binary display types (UUID shown vs bytes stored → bind bytes); case-insensitive collations (server semantics); clipboard operand with multiple lines → IN list; existing raw WHERE ending with `--` (chips are separate, no splicing problem); raw WHERE with OR (the final SQL parenthesizes it); composite FK with partial NULLs; duplicate values across rows; FK and PK type mismatch (cast the bound parameter); missing source column in the result ("Column … is not in the result"); cross-connection virtual FKs (target on another data source → opens there); self-referencing FKs; cyclic navigation (history stack).
- **Test cases (input → expected):** cell `name = O'Brien` → chip `name = $1` bound `O'Brien`, "Convert to text" → `name = 'O''Brien'`; cells 1, 2 in `id` → `id IN ($1, $2)`; NULL cell with `=` → `name IS NULL`; FK `(trip_id, segment_id)` rows `(1,10)`, `(2,20)` → PG `(trip_id, segment_id) IN (($1,$2),($3,$4))` (pairs preserved, not the cross product); SQL Server same → `(trip_id = @P1 AND segment_id = @P2) OR (…)`; FILTER-11/12 differential.
- **i18n notes:** template labels `filterby-equals`, `-not-equals`, `-less`, `-like`, `-is-null`, `-is-not-null`, `filterby-from-clipboard`, `related-rows-none = No related rows.`; chips render values with the grid formatter (raw by default) and isolate RTL values (FSI/PDI).
- **Verification status:** DBeaver DNF test [Observed: `DBDReferenceUtilsTest.java:37-75`]; DataGrip operand kinds [Observed: class names]; exact DataGrip menu labels [Unknown].
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat; MCP `filter_table` (structured predicates).

### HV-RES-13: Auto-refresh scheduler

- **Priority / depends on:** v1.x. Depends on HV-RES-05/06, HV-EXEC-05.
- **Sources merged:** datagrip/08 ALG-GRID-22 (refresh only while shown, skip on pending changes, no overlap), dbeaver-04 §3.4 (auto-refresh timers cancelled on new reads).
- **Problem:** Periodically reload a grid (dashboards, queues) without overlapping requests, losing edits or hammering the server.
- **Inputs / outputs:** interval, visibility, session state, pending edits → reload requests.
- **Decision:** a timer per grid, active only while the grid is visible; minimum interval 1 s; skip a tick (with a badge) when a request is running on the session, when pending edits exist, or when a cell editor is open; reload preserves selection by row identity and the scroll position by row index; exponential back-off on errors (×2 up to 5 min).
- **Algorithm:** `on_tick`: if `!visible || paused || session.busy() || pending_edits || editing` → skip and mark; else enqueue `Reload(result)` (newest-wins coalescing). Next tick is scheduled after the reload completes + interval (never concurrent). On wake from sleep fire once.
- **Data structures and Rust placement:** `hacivat-app::results::AutoRefresh { interval: Option<Duration>, paused: bool, backoff: Duration }` (UI-side timer using GPUI's executor timer).
- **Complexity / performance budget:** negligible; the server cost is the reload.
- **Edge cases (exhaustive list):** connection lost (back-off); queries longer than the interval (no overlap); manual tx open (the reload sees own uncommitted changes); several grids on one table (independent timers); sleep/wake; a suspended cursor (reload releases it first); console results of non-Read statements (auto-refresh disabled).
- **Test cases (input → expected):** interval 5 s, query takes 7 s → refreshes every ~12 s (7 + 5), never concurrent; pending edit present → tick skipped with a badge; tab hidden → no ticks.
- **i18n notes:** `autorefresh-every = Every { $seconds } s`, `autorefresh-paused`, `autorefresh-skipped-pending`.
- **Verification status:** DataGrip `GridAutoRefresher` refresh-only-when-shown [Observed: class/behaviour names]; preset intervals [Unknown].
- **Prior art and evidence:** datagrip/08 §3.6.
- **Used by:** Hacivat.

### HV-RES-14: Non-SQL paging (Redis per-type values, Mongo cursors)

- **Priority / depends on:** v1.x. Depends on HV-RES-01/02.
- **Sources merged:** RedisInsight/Compass doc ALG-REDIS-02 (per-type paging with SCAN family, ZRANGE by rank/score, XRANGE by ID, GETRANGE, overflow buffers, seen-sets), ALG-MONGO-02 (document → table columns, Missing vs NULL), Beekeeper/TUI ALG-DATA-01 (opaque token mode).
- **Problem:** Page through Redis aggregates and MongoDB query results into the same store and grid, with each engine's native cursor semantics.
- **Inputs / outputs:** key/type or collection query, page size → rows in a result store (`ColumnBuf::Values` for heterogeneous documents) + an engine-specific cursor.
- **Decision:** reuse the ALG-REDIS-02 algorithm unchanged (cursor kinds `HashScan`, `Offset`, `ScoreCursor`, `StreamId`, `ByteOffset`; never `HGETALL/SMEMBERS/LRANGE 0 -1` on unknown sizes). MongoDB: `find`/`aggregate` with `batchSize = page`, `getMore` on scroll, cursor kept like HV-RES-05 (server default cursor timeout 10 min; we do not set `noCursorTimeout`); columns derived from the schema sample + keys seen (ALG-MONGO-02); Missing ≠ NULL in cells.
- **Algorithm:** see ALG-REDIS-02 steps 1–9 and ALG-MONGO-02 steps 1–5 (cited, not repeated). Store integration: each page appends rows; SCAN-based types keep a bounded seen-set to remove duplicates; the result's `Completeness` mirrors cursor state (`Suspended` while the server cursor/scan cursor is non-zero).
- **Data structures and Rust placement:** `hacivat-core::driver::redis::paging`, `hacivat-core::driver::mongo::paging`; `PageCursor::Token` for Mongo cursor ids.
- **Complexity / performance budget:** SCAN pages O(1) amortized per call; Mongo `getMore` per page; first page ≤ 100 ms on LAN.
- **Edge cases (exhaustive list):** as listed in ALG-REDIS-02 (key deleted/type changed between pages, listpack overflow, duplicates across SCAN pages, equal zset scores, stream trimming, Redis < 6.2 exclusive ranges, cluster routing) and for Mongo: cursor timeout (`CursorNotFound` → `Closed{IdleTimeout}`), sharded cursors, `allowDiskUse`, tailable cursors (capped collections, out of scope), sessions and causal consistency (out of scope v1.x).
- **Test cases (input → expected):** ALG-REDIS-02 tests 1–6; Mongo collection of 100k documents, batch 500 → first page then `getMore` on scroll; idle 11 min → `Closed{IdleTimeout}` with "Re-run".
- **i18n notes:** `redis-scan-continue = Scanned { $scanned } keys — continue?`, `mongo-cursor-expired`.
- **Verification status:** RedisInsight behaviour [Observed: RI per-type services, SSPL behaviour only]; Mongo cursor timeout default [Inferred: MongoDB docs].
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat Redis/Mongo views.

---

## 3. Conflicts resolved in this catalog

| Topic | Source A | Source B | Decision |
|---|---|---|---|
| Result transport | ui-01 ALG-UI-01 Arrow IPC windows | ADR 0001 in-process | In-process snapshots (HV-RES-03) |
| Store format | datagrip/08, dbeaver-04: Arrow-like | dbeaver-05 open question | Own Arrow-shaped columnar chunks; no arrow-rs in v1 core |
| Console paging | DBeaver: re-read from 0 | DataGrip: cursor + scrollable result sets | One execution, suspended cursor, release protocol |
| Table paging | DataGrip: OFFSET | Beekeeper/dbeaver-04 rec: keyset | Keyset with identity tiebreaker; Offset fallback with a warning |
| `has_more` | DBeaver: `received ≥ n` | Beekeeper/harlequin: n+1 probe | n+1 probe everywhere |
| Page size | DataGrip 500 | DBeaver 200 | 500 |
| Filter on query results | DBeaver: parser injection | DataGrip: wrap | Wrap; injection only via future explicit CST action |
| LOB default | DataGrip 1,024 B (image context) | DBeaver 10,000 chars in memory | 16 KiB server-side prefix (tables), 1 MiB client cap (console) |
| Header sort | DBeaver SMART (local when loaded) | DataGrip: ORDER BY for tables | Tables: server; console: local if complete, else wrap (Read only) |
| Filter-by insertion | DataGrip: splices text into WHERE | — | Structured chips + "convert to text" |

## 4. Open questions that need a product decision

- **PD-6 (cursor retention defaults).** Keep a suspended cursor (with its transaction/snapshot) for up to 10 min idle (recommended), or drain-and-close immediately after read-ahead (safer for servers, but "fetch more" then requires a re-run). Includes the read-ahead target (10,000 rows / 16 MiB).
- **PD-7 (memory budgets).** 256 MiB per result and `min(25% RAM, 4 GiB)` globally (recommended) — affects how often users see the "memory limit" banner.
- **PD-8 (editing through views).** Off by default (recommended) vs on for simple auto-updatable views.
- **PD-9 (tinyint(1) as boolean).** Show MySQL `TINYINT(1)` as Int by default (recommended; data fidelity) vs Bool (DBeaver/DataGrip-like familiarity).
