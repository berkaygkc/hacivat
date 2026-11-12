# Open-Source DB Clients — RedisInsight and MongoDB Compass (+ mongodb-schema)

Status: research input. Date: 2026-11-11.
Related: `ui-02-code-editor-and-data-grid-components.md` (grid algorithms ALG-GRID-01..04), `oss-clients-beekeeper-and-tui.md` (paging, edits, cancellation).

---

## 1. Scope and sources

| Project | Version / commit | License | Handling |
|---|---|---|---|
| RedisInsight | 3.8.0, commit `7b3be3c977c34cc423f23edeed0914830a17a687` (2026-11-11) | **SSPL v1** (`LICENSE`) | Prose and own pseudocode only; no code reuse |
| MongoDB Compass | monorepo commit `4f1421c605102367198357782c56021ce2583075` (2026-11-11); `compass-crud` 14.9.0, `compass-editor` 0.73.1, `compass-components` 1.71.1 | **SSPL v1** (`LICENSE`) | Prose and own pseudocode only |
| mongodb-schema | 12.7.0, commit `cfb4e184474793837d0eb083d23e96aa7473b68d` | **Apache-2.0** | Reuse possible with attribution; still specified in own pseudocode |

Paths below are relative to `.refs/repos/RedisInsight/redisinsight/` (RI), `.refs/repos/compass/packages/` (Compass) and `.refs/repos/mongodb-schema/src/` (MS). No builds or scripts executed.

---

## 2. Executive summary

1. **RedisInsight architecture**: Electron desktop + React UI + NestJS API (ioredis / node-redis) that can also run as a Docker web app **[Observed: RI api/package.json:110,120; root package.json; Dockerfile]**.
2. **Key scanning**: per-request loop of `SCAN cursor MATCH p COUNT min(2000, count) [TYPE t]` until `count` keys are collected, the cursor returns 0, or `scanned ≥ scanThreshold` (default 10,000), with `DBSIZE` (fallback `INFO keyspace`) for totals **[Observed: RI api/src/modules/browser/keys/scanner/strategies/standalone.scanner.strategy.ts:82-115; config/default.ts:207-212; modules/redis/utils/keys.util.ts:5-39]**.
3. **Exact-key fast path**: if the pattern contains no glob characters, RI skips SCAN and fetches that key's info directly (TTL −2 means missing) **[Observed: standalone.scanner.strategy.ts:135-156]** — same for hash fields (`HGET`), set members (`SISMEMBER`), zset members (`ZSCORE`) **[Observed: hash.service.ts:154-165; set.service.ts:111-115; z-set.service.ts:311-315]**.
4. **Cluster scanning**: scans all primaries in parallel; the "cursor" sent to the UI is a composite string `host:port@cursor||host:port@cursor` with `-1` marking finished nodes **[Observed: browser/utils/clusterCursor.ts:4-46; cluster.scanner.strategy.ts:76-110]**.
5. Key metadata (type, TTL, `MEMORY USAGE key SAMPLES 0`) is fetched in **pipelines** for loaded keys, and in tree view only for **visible** nodes, debounced 100 ms **[Observed: standalone.scanner.strategy.ts:24-78; ui/src/pages/browser/components/virtual-tree/VirtualTree.tsx:150-200]**.
6. **Tree view** is built in a **Web Worker** from the *currently loaded* keys, splitting names by one or more delimiters (regex alternation), with **Redis Cluster hash-tag awareness** (delimiters inside `{…}` don't split) and an optional fixed prefix length; folders sort before leaves; folder `keyCount` is the number of loaded leaves below it and `keyApproximate` = percentage of loaded keys **[Observed: ui/src/helpers/constructKeysToTree.ts:12-185; KeyTree.tsx:89-118]**. Tree mode scans 10,000 keys per batch vs 500 in list mode **[Observed: ui/src/config/default.ts:90-91]**.
7. Per-type value paging: hash `HSCAN` loop until `count` fields; list `LRANGE offset offset+count−1`; zset `ZRANGE/ZREVRANGE` by rank (or `ZSCAN` for patterns); set `SSCAN`; stream `XRANGE/XREVRANGE start end COUNT n`; string `GETRANGE` for partial reads and UI cap of 4,999 chars **[Observed: hash.service.ts:355-400; list.service.ts:131-153; z-set.service.ts:99-121,354-380,436; set.service.ts:265; stream.service.ts:122-154; string.service.ts:79-95; ui/src/constants/string.ts:1]**. Hash-field TTLs (`HTTL … FIELDS n …`) shown when supported **[Observed: hash.service.ts:170-190]**.
8. Value decoders: Unicode, ASCII, JSON, HEX, Binary, Msgpack, PHP serialized, Java serialized, Protobuf, Pickle, Vector 32/64-bit, DateTime, Markdown **[Observed: ui/src/constants/keys.ts:151-166]**.
9. **Database analysis**: SCAN a sample of keys, pipeline `MEMORY USAGE`, `TYPE`, `TTL`, per-type length; aggregate top namespaces (first delimiter), top 15 keys by memory/length, memory by TTL buckets (No expiry, <1h, 1–4h, 4–12h, 12–24h, 1–7d, >7d) **[Observed: database-analysis/scanner/keys-scanner.ts:49-80; providers/database-analyzer.ts:15-257]**.
10. **Bulk actions** (delete/unlink): server-side SCAN loop + pipelined `UNLINK` (fallback `DEL`), progress over WebSocket, per-key failure report **[Observed: bulk-actions/models/runners/simple/*.ts]**.
11. **Workbench**: Monaco editor, completions/signature help generated from Redis `commands.json` files (core + modules), fetched from upstream URLs with bundled fallback; raw vs formatted result modes **[Observed: api/config/default.ts:302-343; ui/src/utils/monaco/*; workbench-commands.executor.ts:114-181]**. Profiler uses `MONITOR` per shard with optional log file; slow log reads/updates `slowlog-*` config **[Observed: profiler/models/redis.observer.ts:55-97; slow-log.service.ts:115-160]**.
12. **RedisInsight i18n is a retrofit in progress**: i18next + react-i18next, flat keys, single namespace, `en` source of truth and only `bg` as second language (selected via `?lang=bg` query for now), extraction with `i18next-cli` keeping removed keys **[Observed: i18next.config.mjs; ui/src/i18n/README.md; i18n.ts]**.
13. **Compass schema analysis**: `$match` (if filter) → `$sample {size: 1000}` (or min(limit, 1000)) → `$project` (if projection), `allowDiskUse`, read preference fallback `secondaryPreferred`, `maxTimeMS` capped by preference, `promoteValues: false` **[Observed: Compass compass-schema/src/stores/schema-analysis-reducer.ts:24,310-322; modules/schema-analysis.ts:40-70; data-service/src/data-service.ts:2609-2648]**.
14. **mongodb-schema** algorithm: streaming per-document recursion maintaining field → type → count trees; arrays recurse into element types under the same path and record lengths; documents recurse into fields; values kept in **reservoir samples** (100 for String/Binary/Code, 10,000 otherwise; strings cropped to 100 chars in Compass); final pass computes probabilities, `Undefined` pseudo-type for missing fields, unique/duplicate stats **[Observed: MS schema-analyzer.ts:340-470, 520-640; Compass schema-analysis.ts:63]**.
15. Compass document views: list (HadronDocument tree, 25 visible elements per level with "show more"), JSON (CodeMirror per document, EJSON), table (AG Grid; columns = union of keys at current path; per-column type or `Mixed`; drill-down into nested docs/arrays via breadcrumb), all **virtualized** with estimated item heights **[Observed: compass-crud/src/components/*; hadron-document/src/document.ts:29; element.ts:67-100; table-view/document-table-view.tsx:905-1000; virtualized-document-list-view.tsx:21-45]**.
16. Compass edits use **optimistic concurrency**: `findOneAndUpdate` with a query containing `_id` + the *original values of changed fields* (+ shard keys), so the update fails if someone changed them in the meantime **[Observed: hadron-document/src/document.ts:173-200; compass-crud/src/stores/crud-store.ts:672-700]**.
17. Query bar autocompletion = CodeMirror sources combining schema field names, query operators and BSON constructors, filtered by case-insensitive substring; aggregation builder previews each stage with `$limit` 10 (and inserts `$limit 100000` before full-scan stages on large collections), debounced 700 ms **[Observed: compass-editor/src/codemirror/query-autocompleter.ts; compass-aggregations/src/modules/pipeline-builder/pipeline-preview-manager.ts:20-80]**.
18. **Explain plan** visualization: stage tree from `inputStage/inputStages/innerStage/outerStage/thenStage/elseStage/shards/executionStages` children, laid out with d3-flextree; per-stage *cumulative* `executionTimeMillisEstimate` with child time = max over children (assumes parallel shards) → self time = cur − prev **[Observed: explain-plan-helper/src/index.ts:308-345; compass-explain-plan/src/components/explain-tree/explain-tree-data.ts:25-74; tree-layout.tsx:1-5]**.
19. Compass has **no UI i18n** (no i18n libraries in any package.json) **[Observed: grep across packages/*/package.json]**.
20. Six algorithms specified for Hacivat: ALG-REDIS-01 (key tree with lazy SCAN, counts, virtualization), ALG-REDIS-02 (per-type paging), ALG-MONGO-01 (schema inference), ALG-MONGO-02 (document → table flattening), ALG-MONGO-03 (JSON tree virtualization), ALG-MONGO-04 (explain tree → flame graph).

---

## 3. Detailed findings — RedisInsight

### 3.1 Architecture

- `redisinsight/api` (NestJS service: modules `browser`, `bulk-actions`, `cli`, `cluster-monitor`, `database-analysis`, `profiler`, `pub-sub`, `slow-log`, `workbench`, `database-recommendation`, `rdi`, `ai`, …) **[Observed: RI api/src/modules listing]**; `redisinsight/ui` (React, Redux slices, react-virtualized, Monaco); `redisinsight/desktop` (Electron) **[Observed: listing; package.json:313-334]**.
- The UI talks to the API over HTTP + WebSockets (profiler, bulk actions progress, pub/sub) **[Observed: bulk-actions.gateway.ts; profiler.gateway.ts]**. Hacivat analogue: Rust core + Tauri events.

### 3.2 Key browser: list mode

- API request carries `cursor`, `count`, `match`, optional `type`, `keysInfo` flag, `scanThreshold` **[Observed: standalone.scanner.strategy.ts:120-125]**.
- Loop invariant (behavioral): accumulate keys from successive SCAN calls; each iteration adds `COUNT` to a `scanned` counter; stop when enough keys, full cycle (cursor 0) or threshold reached. `COUNT` is clamped to 2000 per call **[Observed: lines 82-115]**. The UI shows "Scanned X / total" and a "Scan more" button when the threshold stops the loop **[Inferred: scanned/total fields returned; UI behavior]**.
- Totals: `DBSIZE`; if it fails (ACL), parse `INFO keyspace` for the current DB; workaround for Pika-like servers where totals are 0 but keys exist → `total = null` (unknown) **[Observed: keys.util.ts:5-39; standalone.scanner.strategy.ts:166-169]**.
- `TYPE` filter uses `SCAN … TYPE t` (Redis ≥ 6) **[Observed: lines 99-101]**.
- Key info pipelines: `MEMORY USAGE key SAMPLES 0` (exact for aggregates — can be expensive for huge keys), `TYPE`, `TTL` **[Observed: lines 24-78]**. Error entries map to `null`.
- UI list uses react-virtualized `InfiniteLoader` with threshold 100 rows and minimum batch = scan count (500) **[Observed: ui/src/components/virtual-table/VirtualTable.tsx:49, 511-615; ui/src/config/default.ts:90]**.

### 3.3 Key browser: tree mode

- Delimiters: user-provided list → escaped → regex alternation; prefix length option (first N characters never split) **[Observed: KeyTree.tsx:89-118]**.
- Build (worker): for each key, split name; walk/create folder nodes; leaf stored under a synthetic child name (`name + "keys" + delimiter + "keys"`) to avoid collisions with folder names; then convert to a sorted array tree: folders first, then leaves; `localeCompare(…, 'en')` for ASC/DESC; each folder gets `keyCount` (sum of leaves) and `keyApproximate` = `keyCount / loadedKeys × 100`, `fullName` = path joined by the delimiter (or `-` if multiple delimiters) **[Observed: constructKeysToTree.ts:71-185]**.
- Hash-tag rule: if the name contains `{…}` with non-empty content, delimiter matches inside the braces are skipped **[Observed: constructKeysToTree.ts:29-66]**.
- Random string ids per node via `Math.random().toString(36)` with collision retry **[Observed: lines 98-110]**.
- Folder auto-open when a single folder at top level; metadata loaded lazily: as rows render, their key names are queued; a 100 ms debounced batch request fetches type/TTL/size for those keys and patches nodes by path **[Observed: VirtualTree.tsx:80-200]**.
- Limitation (by design): counts reflect only **loaded** keys, not the whole keyspace; deep folders may be partially populated until more SCAN batches arrive.

### 3.4 Key types and editors

- Types: hash, list, set, zset, string, ReJSON-RL/json, stream, vectorset, array, plus module types (graph, timeseries…) **[Observed: ui/src/constants/keys.ts:6-30]**; per-type key-info strategies on the API **[Observed: keys/key-info/strategies/*.ts]**.
- Hash: `HLEN` total; exact-field fast path; `HSCAN` loop; per-field TTL via `HTTL` when the server supports hash field expiration; big-hash recommendation trigger **[Observed: hash.service.ts:135-200, 355-400]**.
- List: `LLEN`, `LRANGE` by offset/count, `LINDEX` for single element search by index; push/pop endpoints with count **[Observed: list.service.ts:131-281]**.
- Set: `SCARD`, `SISMEMBER` fast path, `SSCAN` **[Observed: set.service.ts:100-265]**.
- ZSet: `ZCARD`, sort order selects `ZRANGE … WITHSCORES` vs `ZREVRANGE`, `ZSCORE` fast path, `ZSCAN` for patterns **[Observed: z-set.service.ts:99-121, 299-436]**.
- Stream: `XINFO STREAM`, `XRANGE/XREVRANGE` with `start/end/count`; consumer groups via `XINFO GROUPS`, pending via `XPENDING` **[Observed: stream.service.ts:45-154; consumer-group.service.ts; consumer.service.ts]**.
- String: `GETRANGE start end` partial loads, else `GET`; UI truncates display at 4,999 characters with "load all" **[Observed: string.service.ts:79-95; constants/string.ts:1]**.
- TTL: key TTL displayed in rows and editable (EXPIRE/PERSIST) **[Observed: key-row-ttl component]**.

### 3.5 Bulk actions, analysis, profiler, slow log, workbench

- Bulk delete: total from `DBSIZE`; loop `SCAN cursor <filter args>` → pipeline `UNLINK` (or `DEL` if UNLINK unsupported) for returned keys; record processed/succeeded/failed counts and per-key errors; stop when cursor = −1 (finished) or status changes **[Observed: abstract.bulk-action.simple.runner.ts:25-110; unlink.bulk-action.simple.runner.ts:17-26]**. Bulk import from files also exists **[Observed: bulk-import.service.ts]**.
- Database analysis: scan up to `filter.count` keys with COUNT = min(2000, count), pipelines for memory/type/ttl, per-type length via key-info strategies; namespace = bytes up to first delimiter (Buffer-level search, binary-safe); top-15 lists; TTL histogram buckets as listed in §2 **[Observed: keys-scanner.ts:49-104; database-analyzer.ts:15-257]**. Recommendations module flags patterns (big hashes, etc.) **[Observed: recommendationService.check in hash.service.ts:194-198]**.
- Profiler: one `MONITOR` observer per shard, listener fan-out to clients, optional persisted log file with idle cleanup **[Observed: redis.observer.ts:55-97; log-file.ts:91-135]**.
- Slow log: reads `CONFIG GET slowlog*`, updates `slowlog-log-slower-than` / `slowlog-max-len`, lists `SLOWLOG GET` **[Observed: slow-log.service.ts:115-160]**.
- Workbench: Monaco with Redis Monarch tokens, completion and signature-help providers built from command specs (arguments with optional/multiple/block/oneof/enum tokens) **[Observed: ui/src/utils/monaco/monacoRedisCompletionProvider.ts:28; monacoRedisSignatureHelpProvider.ts]**; specs fetched from `redis-doc` and module repos' `commands.json` with local fallback **[Observed: commands-json.provider.ts:15-60; api/config/default.ts:302-343]**; results rendered raw/ASCII/formatted **[Observed: workbench-commands.executor.ts:114-181]**.

### 3.6 i18n

- See §2 item 12. Notable mechanics: flat keys with literal dots (`keySeparator: false`), compile-time type-safe keys from `en.json`, real i18n instance in tests so text assertions keep working during migration, `returnEmptyString: false` so empty `bg` values fall back to English **[Observed: i18n.ts; README.md]**. 2,304 lines per locale file **[Observed: wc]**.

---

## 4. Detailed findings — MongoDB Compass and mongodb-schema

### 4.1 Architecture

- Electron + React; many workspace packages (connections, crud, schema, query bar, aggregations, explain plan, indexes, shell, import/export, data modeling, generative AI) **[Observed: Compass packages listing]**. `data-service` wraps the Node driver with cancellable operations (`AbortSignal`), session kill (`killSessions`) and read preference fallbacks **[Observed: data-service.ts:2609-2700]**.

### 4.2 Schema analysis (Compass + mongodb-schema)

- Sampling pipeline as in §2 item 13. Analysis consumes the cursor as an async iterable; abort signal checked between documents; analysis yields to the event loop periodically (`allowAbortDuringAnalysis`) **[Observed: MS schema-analyzer.ts:520-526, 650-665; Compass schema-analysis.ts:40-98]**.
- `distinctFieldsAbortThreshold` aborts on schemas with too many distinct fields **[Observed: MS schema-analyzer.ts:528-534]**.
- Semantic types (optional detectors, e.g., email/geo) override the type name while keeping `bsonType` **[Observed: MS schema-analyzer.ts:496-513, 536-545, 553-560]**; Compass checks GeoJSON types for map rendering **[Observed: Compass schema-analysis.ts:100-108]**.
- Output per field: `name`, `path`, `count`, `probability = count / parentCount`, `type` (single or list), `types[]` sorted by probability, each type with `count`, `probability`, `unique`, `hasDuplicates`, `values` (reservoir); arrays add `types` of elements, `totalCount`, `lengths`, `averageLength`; documents add nested `fields` **[Observed: MS schema-analyzer.ts:404-470]**.
- `cropString` keeps surrogate pairs intact when cutting **[Observed: MS schema-analyzer.ts:344-348]** (a detail Hacivat should replicate).

### 4.3 CRUD views

- Page size 25 docs; `countDocuments` with `maxTimeMS` capped at 5,000 ms (`COUNT_MAX_TIME_MS_CAP`) **[Observed: crud-store.ts:217, 250-259; utils/cancellable-queries.ts:6-25]**.
- List view: expandable tree per document; elements beyond 25 hidden behind "show more" in steps (25 when editing, else 1000) **[Observed: compass-components document-list/element.tsx:944-948; hadron-document element.ts:67-100]**; virtualization via react-window `VariableSizeList` with a ResizeObserver-fed size cache and initial height estimates computed from the number of top-level fields (cap 25) **[Observed: virtual-list.tsx:3, 119-195; virtualized-document-list-view.tsx:21-45]**.
- JSON view: each document rendered by a CodeMirror multiline editor in EJSON; edits parsed back with `HadronDocument.FromEJSON` **[Observed: json-editor.tsx:77-145, 305]**.
- Table view: AG Grid; row-number pinned column; columns created from the union of element keys at the current path across loaded docs; column header type = the element type if consistent across docs else `Mixed`; drill-down into a nested Object/Array changes the path; documents whose element at that path has a different type are skipped; row actions pinned right **[Observed: document-table-view.tsx:905-1000]**.
- Edit semantics: see §2 item 16; full replace uses `findOneAndReplace` with original values of specified keys; sharded collections include shard keys in the filter **[Observed: crud-store.ts:672-806, 2342-2355]**.
- Bulk update/delete modals with previews **[Observed: bulk-update-modal.tsx, bulk-delete-modal.tsx]**.

### 4.4 Query bar and editor

- Query bar fields (filter, project, sort, collation, skip, limit, hint, maxTimeMS) **[Inferred: query bar UI; query object fields used in schema reducer]**.
- Autocompleters: query (fields + operators + BSON values), aggregation stages, validation, search index, document; history-aware autocompleter **[Observed: compass-editor/src/codemirror/*.ts listing]**. Completion contexts derived from the Lezer JS syntax tree (`resolveTokenAtCursor`, property value vs key) **[Observed: query-autocompleter.ts]**. Lint for unsafe integers **[Observed: use-safe-integer-linter.ts]**.

### 4.5 Aggregation pipeline builder

- Stage editor/wizard/preview components; preview per stage with debounce 700 ms, `$limit` 10 for previews, insertion of `$limit 100000` before full-scan stages (e.g., `$group`, `$sort`) when the collection is larger than the sample size; output stages (`$out`, `$merge`) cannot be previewed **[Observed: pipeline-preview-manager.ts:20-80]**. Each stage's preview request uses its own AbortController keyed by stage index, so editing stage k cancels stale previews **[Observed: lines 82-90]**.

### 4.6 Explain plan

- `ExplainPlan` helper normalizes planner info (including sharded `shards` structures), execution stats (`executionTimeMillis`, `totalKeysExamined`, `totalDocsExamined`, `nReturned`), index usage (`MULTIPLE` when differing across shards) **[Observed: explain-plan-helper/src/index.ts:34-216; get-execution-stats.ts]**.
- Tree visualization: recursion over child stages, d3-flextree layout, a "clock" glyph per stage showing the stage's share of total time **[Observed: explain-tree-data.ts; clock.tsx:72-130; tree-layout.tsx]**.

### 4.7 Indexes

- Index list combines `listIndexes`, `$indexStats` (usage counts) and `$currentOp` filtered to in-progress `createIndexes` (build progress) via `Promise.allSettled` **[Observed: data-service.ts:2205-2415]**; regular vs search indexes, rolling index builds, create-index form **[Observed: compass-indexes/src/modules listing]**.

### 4.8 i18n

- None (English-only UI) **[Observed]**.

---

## 5. Algorithm specifications

### ALG-REDIS-01: Redis key tree with lazy SCAN, counts and virtualization

- **Problem:** Present potentially hundreds of millions of keys as a navigable namespace tree (by delimiters such as `:`), without `KEYS *`, without loading everything, with counts that are honest about being partial, and with smooth scrolling.
- **Inputs / outputs:** Inputs: connection (standalone/cluster/sentinel), DB index, pattern `match` (glob), optional type filter, delimiters `D[]`, prefix length `P`, batch target `B` (e.g., 10,000 keys for tree mode), per-call COUNT `c` (≤ 2000), scan threshold `T`, sort order. Outputs: a flattened visible row list for the virtual tree, folder rows with `loadedCount`, optional `exactCount` (on demand), leaf rows with lazily fetched `type/ttl/memory`.
- **Algorithm:**
  1. **Fast path**: if `match` contains no unescaped glob metacharacters (`* ? [ ] \`), run `EXISTS`/`TYPE`/`PTTL` on the literal key and show 0 or 1 result (RI behavior **[Observed]**).
  2. **Cursor state**: standalone → `u64` cursor; cluster → map `node_id → cursor` (finished nodes removed); replicas allowed for scanning if configured (read-only) to spare primaries.
  3. **Batch scan** (Rust task, cancellable): repeat `SCAN cursor MATCH match COUNT c [TYPE t]` (cluster: per node in parallel, bounded concurrency) until `keysCollected ≥ B` or all cursors done or `scannedBudget ≥ T`; record `scannedApprox += c` per call (Redis COUNT is a hint; actual work varies). Stream key names to the tree builder in chunks (e.g., every 1000) rather than at the end.
  4. **Dedup**: SCAN may return duplicates across iterations (guarantee is "at least once" for keys present throughout) — maintain a hash set of loaded keys (binary-safe, `Vec<u8>`); skip duplicates.
  5. **Tree insert** (incremental, no full rebuild): for each new key, split into segments:
     a. If `P > 0`, the first `P` bytes are an unsplittable prefix (operate on bytes, but never split inside a UTF-8 sequence — adjust `P` to the next char boundary when the key is valid UTF-8).
     b. Find delimiter matches (any of `D`, longest-first to handle multi-char delimiters like `::` vs `:`); skip matches located inside a Redis Cluster hash tag `{…}` (first `{` and the first `}` after it, non-empty content) **[Observed: RI hash-tag rule]**.
     c. Segments = text between matches; empty segments allowed (e.g., `a::b` with `:` → `a`, ``, `b`) and rendered as `(empty)`.
     d. Walk from root creating folder nodes keyed by `(segment bytes)`; the last segment creates a **leaf** in the folder's separate leaf map (folders and leaves live in different maps, so a key `a:b` and a folder `a:b:` never collide — RI uses a synthetic suffix for this **[Observed]**).
     e. Increment `loadedCount` on every folder on the path (O(depth)).
  6. **Sorting**: children kept in two sorted containers (folders, leaves) using **byte-wise comparison** by default (keys are binary; locale collation would be misleading and costly); optional natural sort (numeric segments compared numerically) as a UI setting.
  7. **Flattening for virtualization**: maintain an order-statistics structure over the *visible* rows (expanded folders' descendants). Implementation: each node stores `visibleSubtreeSize` (1 for itself + sum of children if expanded). Row index → node by descending with prefix sums over children (children containers keep a Fenwick tree or a balanced tree with subtree sizes). Expand/collapse updates sizes along the ancestor path (O(depth × log fanout)).
  8. **Virtual list**: fixed row height; visible range per ALG-GRID-01 (rows only); overscan 10 rows.
  9. **Lazy metadata**: when rows become visible, enqueue leaf keys whose metadata is unknown; every 100 ms (debounce) send one pipeline of `TYPE`, `PTTL`, and optionally `MEMORY USAGE key SAMPLES 0` for ≤ 500 keys; patch nodes; drop requests for rows that scrolled away before dispatch. Memory usage is opt-in for huge keys (can be slow on big aggregates; use `SAMPLES 5` default and label as estimate).
  10. **Counts**: show `loadedCount` with a "≥" or "loaded" hint when the scan isn't complete; on demand "Count exactly" for a folder runs a dedicated full SCAN with `MATCH <folderPrefix><delim>*` (cancellable, progress shown; cluster: all nodes) and caches `exactCount` with a timestamp (stale indicator).
  11. **Scan more**: user (or auto when scrolling near the end of the loaded list) continues the cursor; tree inserts are incremental so expanded folders stay expanded and scroll position stays anchored to the top visible node id.
  12. **Mutations**: deleting a key/folder removes nodes and decrements counts; renaming re-inserts; TTL expiry discovered lazily (metadata fetch returns missing → remove node).
  13. **Refresh**: full refresh resets cursor and tree but preserves expanded paths and selection by path.
- **Data structures:** `Node { segment: Bytes, folders: SortedMap<Bytes, NodeId>, leaves: SortedMap<Bytes, LeafId>, loaded_count: u64, exact_count: Option<(u64, Instant)>, expanded: bool, visible_size: u64 }`; arena allocation (`Vec<Node>`); `HashSet<Bytes>` for dedup; metadata cache `HashMap<LeafId, Meta>`.
- **Complexity / performance notes:** Insert O(L + depth·log fanout) per key (L = key length); 1M keys ≈ seconds in Rust and tens of MB of memory (keys stored once in the dedup set; nodes store segments). Row lookup O(depth·log fanout). Never build the tree on the UI thread (RI uses a Web Worker **[Observed]**; Hacivat builds in the Rust core and sends visible rows via ALG-UI-01-style windows).
- **Edge cases (exhaustive list):** binary keys (non-UTF-8 → display with escapes `\xNN`, tree split still byte-based); keys containing the delimiter at start/end (`:a`, `a:`) → empty segments; consecutive delimiters; multiple delimiters configured where one is a prefix of another (`:` and `::`); delimiter inside hash tag; unbalanced `{`; empty hash tag `{}` (no protection, per Redis cluster rules); prefix length longer than key; extremely long keys (≥ 1 KB) → truncate display; huge fan-out folder (1M children) → children container must be scalable and virtualized; millions of keys with no delimiter → flat list; type filter + pattern; ACL denying `SCAN`/`TYPE`/`MEMORY` → degrade (no metadata columns, show permission notice); `DBSIZE` denied → INFO keyspace fallback, else unknown total **[Observed: RI fallback]**; Redis-compatible servers (KeyDB, Dragonfly, Valkey, Pika, Garnet) with different SCAN guarantees or `TYPE` option unsupported (detect via error and filter client-side with `TYPE` pipeline); cluster resharding during scan (MOVED/ASK on metadata fetch; restart node scan on topology change; duplicates handled by dedup); replica lag when scanning replicas; keys expiring during scan; `SCAN` on a DB being flushed; cancellation mid-batch; DB index switching (cluster only DB 0); very slow SCAN on large DBs (budget T with "continue" UI).
- **Test cases (input → expected):**
  1. Keys `user:1`, `user:2`, `user:1:profile`, `order:9` with `:` → root folders `order(1)`, `user(3)`; `user` has folder `1(1)` and leaves `1`, `2`; leaf `user:1` and folder `user:1:` coexist.
  2. `{user:1}:cart` with `:` → segments `{user:1}`, `cart` (hash-tag protected).
  3. `a::b` with delimiters [`::`, `:`] → `a`, `b` (longest match first).
  4. Pattern `session:abc` (no glob) → single EXISTS call, no SCAN.
  5. 1,000,000 keys in batches of 10,000 → tree remains interactive; expanded folders persist across batches.
  6. Duplicate key returned twice by SCAN → counted once.
  7. Binary key `\xff\xfe:x` → folder `\xff\xfe` rendered escaped.
  8. Exact count on folder `user` → SCAN MATCH `user:*` result equals ground truth.
- **Prior art and evidence:** RI scanner strategies, cluster cursor, tree worker, lazy metadata (SSPL — behavior only) **[Observed]**.
- **Used by:** Hacivat (Redis browser); Karagöz (scan budgets/ACL enforcement for remote sessions).

### ALG-REDIS-02: Per-type value paging (HSCAN/SSCAN/ZSCAN/XRANGE/LRANGE)

- **Problem:** Display and edit values of very large Redis aggregates (hashes/sets/zsets with millions of members, long lists, huge streams, big strings) incrementally and safely.
- **Inputs / outputs:** Inputs: key, type, page size `n`, filter pattern (optional), sort/order where applicable, cursor/offset/ID position. Outputs: page of entries, next position, total (cheap cardinality command), flags (`approximate`, `complete`).
- **Algorithm (per type):**
  1. **Totals** (O(1) commands): `HLEN`, `SCARD`, `ZCARD`, `LLEN`, `XLEN`, `STRLEN`, `JSON.TYPE/JSON.OBJLEN/ARRLEN` for JSON.
  2. **Hash**: no filter → `HSCAN key cursor COUNT n` repeated until ≥ n fields or cursor 0 (RI loop **[Observed]**); dedup fields across calls (HSCAN may repeat); filter without glob → `HGET`/`HMGET` exact; filter with glob → `HSCAN … MATCH p`, with a scan budget and "continue" UI because matches may be sparse. Small hashes (listpack encoding) return everything in one call regardless of COUNT — handle >n results by buffering the overflow for the next page. Optional: field TTLs via `HTTL key FIELDS k f1..fk` (Redis ≥ 7.4) **[Observed]**, values truncated server-side? (not possible with HSCAN → limit display and fetch full value via `HGET` on demand).
  3. **Set**: like hash with `SSCAN`; exact filter → `SISMEMBER` **[Observed]**.
  4. **Sorted set**: browse by rank → `ZRANGE key start stop WITHSCORES` (or `REV`) for stable offset paging **[Observed: RI ZRANGE/ZREVRANGE]**; alternative score-based browsing → `ZRANGE key (lastScore +inf BYSCORE LIMIT 0 n WITHSCORES` with tie handling by member (`ZRANGE … BYLEX` not applicable across different scores — keep `(score, member)` cursor and skip members ≤ last member at equal score); filter by member glob → `ZSCAN` (unordered!) — label results unordered; exact member → `ZSCORE`.
  5. **List**: `LRANGE key i i+n−1` (offset paging is O(S+N) from the nearer end — acceptable; for deep middle offsets on huge lists warn about cost); negative indices for "from tail" browsing; search by value is a full scan (`LPOS key value COUNT k` on Redis ≥ 6.0.6) with budget.
  6. **Stream**: forward `XRANGE key (lastId + COUNT n` (exclusive start with `(` on Redis ≥ 6.2, else increment the sequence part of the ID); backward `XREVRANGE key (firstId - COUNT n`; jump by time → construct ID `<ms>-0`; consumer groups via `XINFO GROUPS`, pending via `XPENDING` summary/extended form **[Observed]**.
  7. **String**: `STRLEN`; if ≤ limit (e.g., 64 KB) → `GET`; else `GETRANGE key 0 limit−1` with "load more"/"load all" **[Observed: RI GETRANGE + 4,999-char UI cap]**; detect binary and choose decoder (UTF-8, hex, JSON, msgpack, …).
  8. **JSON (RedisJSON)**: `JSON.TYPE key $path`; for objects `JSON.OBJKEYS` + per-key `JSON.GET` on expand; arrays `JSON.ARRLEN` + `JSON.GET key '$[i:j]'` slices — lazy tree (ALG-MONGO-03 rendering).
  9. **Editing**: per-item commands (`HSET`, `HDEL`, `SADD/SREM`, `ZADD/ZREM`, `LSET` by index with race warning, `XADD/XDEL`, `SET` with `KEEPTTL`) and optimistic checks where possible (`WATCH key` + `MULTI/EXEC` for multi-step edits; for list `LSET`, verify `LINDEX` matches the displayed value first inside WATCH).
- **Data structures:** `ValueCursor` enum { HashScan(u64, overflow buffer), Offset(i64), ScoreCursor(f64, member), StreamId(String), ByteOffset(u64) }; seen-set for SCAN-based types (bounded; when it grows too large, accept duplicates with a "may contain duplicates" flag).
- **Complexity / performance notes:** SCAN family O(1) per call amortized; ZRANGE by rank O(log N + n); LRANGE O(S + n); XRANGE O(log N + n). Never use `HGETALL/SMEMBERS/ZRANGE 0 -1/LRANGE 0 -1` on unknown-size keys (blocks server).
- **Edge cases (exhaustive list):** key deleted or type changed between pages (`WRONGTYPE` → refresh key); key expiring mid-browse; huge members (MB values) → truncate display; binary field names; listpack small-encoding returning all at once; MATCH yielding zero results for many calls (budget + continue); duplicates across SCAN pages; zset with equal scores; NaN impossible but ±inf scores; list modified concurrently (offset drift — show "list changed" via LLEN check); stream trimming (MAXLEN) while browsing (IDs disappear — fine with ID cursors); stream IDs with explicit sequence; Redis < 6.2 (no exclusive range `(`); cluster (key lives on one node — route; `MOVED` handling); ACL restrictions per command; Redis-compatible servers lacking `HTTL`/`LPOS`/`GETRANGE` semantics; huge strings ≥ 512 MB (Redis max) — never fetch fully without explicit confirm.
- **Test cases (input → expected):**
  1. Hash with 1,000,000 fields, n = 500 → first page ≥ 500 fields, cursor ≠ 0; subsequent pages no duplicates (seen-set).
  2. Hash with 50 fields (listpack) and n = 20 → first call returns 50 → page 1 shows 20, overflow buffered for page 2/3, no extra server calls.
  3. ZSet scores [1,1,1,2] members [a,b,c,d], score cursor after `b` (score 1) → next page `c, d`.
  4. Stream: last ID `1700000000000-5` → next page starts strictly after it.
  5. String 10 MB → first 64 KB displayed; "load all" requires confirmation.
  6. Exact hash field filter `email` → single `HGET`.
- **Prior art and evidence:** RI per-type services (SSPL — behavior) **[Observed]**.
- **Used by:** Hacivat; Karagöz (command allow-lists per role).

### ALG-MONGO-01: Schema inference (sampling + type lattice + presence + nested paths + arrays)

- **Problem:** Infer a useful schema for a schemaless collection from a sample: field paths, type distribution, presence probability, nested documents, arrays (element types, lengths), example values — fast, cancellable, memory-bounded.
- **Inputs / outputs:** Inputs: namespace, filter, projection, sample size `S` (default 1000), `maxTimeMS`, read preference, limits (`maxDistinctFields`, `reservoirSizes`, `maxStoredValueLength`). Output: `Schema { count, fields: [Field] }` with `Field { name, path[], count, probability, types: [Type] }`, `Type { name, bsonType, count, probability, unique?, hasDuplicates?, values?, (Array: lengths[], averageLength, totalCount, types[]), (Document: fields[]) }`.
- **Algorithm:**
  1. **Sample**: aggregation `[$match filter]? → $sample {size: S} → [$project proj]?`, `allowDiskUse: true`, `maxTimeMS`, prefer secondaries, do not promote BSON values to native types (keep Int32/Int64/Decimal128/Double distinct) **[Observed: Compass sampling]**. For S ≥ 5% of the collection, `$sample` falls back to a collection scan + random sort internally **[Inferred: MongoDB $sample behavior]**; for views/time-series, `$sample` semantics differ — show a notice.
  2. **Streaming analysis** (per document, root count += 1): for each top-level `(k, v)`: `addField(root.fields, k, [k], v)`.
  3. `addField(fields, name, path, v)`: get/create `Field{name, path, count:0, types:{}}` (on creation increment `distinctFieldCounter`; abort if > `maxDistinctFields`) **[Observed: MS increaseFieldCount]**; `field.count += 1`; `addType(field.types, path, v)`.
  4. `addType(types, path, v)`: `t = bsonTypeOf(v)` (Document for embedded objects; `Null`; `Undefined` never stored here); get/create `Type{name:t, count:0}`; `count += 1`;
     - Array: `lengths.push(len(v))`; for each element `e`: `addType(type.types, path, e)` (elements share the **same path**; nested arrays recurse) **[Observed: MS addToType]**.
     - Document: for each `(k2, v2)`: `addField(type.fields, k2, path+[k2], v2)`.
     - Scalar (not Null): reservoir-sample the (length-capped) value: reservoir 100 for String/Binary/Code, 10,000 others **[Observed]**; cap strings to `maxStoredValueLength` without splitting surrogate pairs **[Observed: cropString]**.
  5. Yield/cancel check every ~1000 field/type visits **[Observed: allowAbortDuringAnalysis cadence]**.
  6. **Finalize** (recursive): for a field map with `parentCount` (root count for top-level; for a Document type, its `count`; for documents inside arrays, the number of array elements that were documents):
     - `field.probability = field.count / parentCount`;
     - `undefinedCount = parentCount − field.count`; if > 0 add pseudo-type `Undefined {count: undefinedCount, probability: undefinedCount/parentCount}` **[Observed]**;
     - each type: `probability = type.count / parentCount` (not `/ field.count`): mongodb-schema divides by the parent count, so a field's types plus its `Undefined` pseudo-type sum to 1 **[Observed: finalizeSchemaFieldTypes(field.types, parentCount)]**. Hacivat adopts the same convention and additionally exposes `conditionalProbability = type.count / field.count` ("given the field is present") for UI tooltips.
     - Array types: element `totalCount = Σ element type counts`; element type probabilities relative to `totalCount`; `averageLength = totalCount / lengths.length` **[Observed]**.
     - `unique = |distinct(values)|` over the reservoir (approximate), `hasDuplicates = unique ≠ |values|`; Null: unique = 1 if count > 0 **[Observed]**.
     - Sort types by probability desc; sort fields by name (case-insensitive, `_id` first) **[Inferred: fieldComparator]**.
  7. **Type lattice for downstream consumers** (Hacivat addition, for table flattening, completion and DDL suggestions): `Null ⊑ everything`; numeric widening `Int32 ⊑ Int64 ⊑ Decimal128`, `Int32/Int64 ⊑ Double` (lossy above 2^53 — flag); `ObjectId`, `Date`, `Timestamp`, `String`, `Binary`, `Boolean`, `Document`, `Array` incomparable → join = `Mixed(set)`. `dominantType = argmax count excluding Null/Undefined`; `isNullable = Null or Undefined present`; `presence = 1 − P(Undefined)`.
  8. **Path catalog**: emit dotted paths (`a.b.c`, arrays as `a.b` with `[]` marker for element paths, e.g., `tags[]`, `items[].sku`) for completion and flattening (MS `getSchemaPaths` provides nested paths including through arrays of documents **[Observed: schemaToPaths]**).
- **Data structures:** recursive maps keyed by field name (use insertion-order map; avoid prototype pollution — MS uses `Object.create(null)` **[Observed]**); reservoirs (Algorithm R with seeded RNG for reproducible tests); counters u64.
- **Complexity / performance notes:** O(total BSON elements in sample). Memory bounded by distinct fields × types × reservoir sizes — reservoir of 10,000 for numbers is large; Hacivat default 100 for all scalar types, 1,000 opt-in.
- **Edge cases (exhaustive list):** empty collection (count 0 → "no documents" state); sample smaller than requested; fields named `__proto__`, `constructor`, `$`-prefixed or containing `.` (legal in modern MongoDB) → paths must be arrays of segments, dotted rendering escaped; duplicate field names in one BSON document (possible at BSON level) → count once per occurrence (document as anomaly); very deep nesting (100 levels max in BSON) → recursion depth guard; huge arrays (100k elements) → cap element sampling (e.g., first 1,000 + reservoir) and record `truncated`; arrays of arrays; empty arrays (length 0, no element types — show `Array<empty>`); mixed arrays; `Null` vs missing; `Undefined` BSON (deprecated) vs missing; Decimal128/Int64 precision; Dates before 1970/after 9999; `MinKey/MaxKey`, `DBRef`, `Code`, `CodeWScope`, `Regex`, `Symbol`; Binary subtypes (UUID 4 vs legacy 3); encrypted fields (CSFLE/QE, Binary subtype 6) → show "encrypted"; GeoJSON detection; `maxTimeMS` timeout (partial results? — report failure, offer smaller sample); distinct field threshold exceeded (abort with message, offer projection); cancellation; views with `$sample` restrictions; time-series collections (bucket internals hidden); sharded collections (sample spans shards).
- **Test cases (input → expected):**
  1. Docs `{a:1}`, `{a:"x"}`, `{}` → field `a`: count 2, probability 2/3, types Int32 (1/3), String (1/3), Undefined (1/3).
  2. `{t:[1,2,"x"]}`, `{t:[]}` → `t` Array count 2, lengths [3,0], averageLength 1.5, element types Int32 (2/3), String (1/3) of totalCount 3.
  3. `{o:{p:1}}`, `{o:null}` → `o` types Document (1/2) with field `p` probability 1 (relative to Document count 1), Null (1/2).
  4. `{items:[{sku:"a"},{sku:"b",q:2}]}` → path `items[].sku` presence 1.0, `items[].q` 0.5 (relative to 2 element documents).
  5. String of 200 chars with emoji at position 99–100 → cropped value does not end with a lone surrogate.
  6. 10,001 distinct field names with threshold 10,000 → abort with error.
- **Prior art and evidence:** mongodb-schema (Apache-2.0) `SchemaAnalyzer` **[Observed: schema-analyzer.ts]**; Compass sampling **[Observed]**.
- **Used by:** Hacivat (schema panel, completion, table flattening, MCP "describe collection" tool).

### ALG-MONGO-02: Document → table flattening

- **Problem:** Show documents in a grid (ALG-GRID-01) with stable columns, handling nested documents, arrays and heterogeneous types, supporting drill-down and editing back to the right paths.
- **Inputs / outputs:** Inputs: schema (ALG-MONGO-01) and/or the current page of documents, current base path `B` (root by default), flatten depth `d` (0 = top-level only like Compass; 1–2 = expand nested documents into dotted columns), column visibility prefs. Output: ordered column list `Col { path[], header, typeSummary (single | Mixed), editable }`, and cell accessor `cell(doc, col) → Present(value) | Missing | TypeMismatch`.
- **Algorithm:**
  1. Resolve base: for each document, `node = get(doc, B)`; if `B` points into an array element (`items[3]`) use that element; skip documents where `node` is missing or of a different kind than the drill-down kind (Compass skips mismatched types **[Observed: document-table-view.tsx:941-950]**).
  2. Column discovery: union of keys of `node` across (a) the schema field list at path `B` (preferred: stable across pages, ordered by presence desc then name) and (b) keys seen in the current page (to include fields not in the sample). `_id` first.
  3. Expansion: for depth `d > 0`, a column whose dominant type is Document with ≤ K sub-fields (e.g., 10) is replaced by its sub-field columns (`address.city`, `address.zip`); arrays are **never** expanded into columns (cardinality unbounded) — they show a summary cell (`[3 items]`, preview of first elements) with a drill-down action.
  4. Header type summary: if all present values share a type → that type; else `Mixed` with tooltip of distribution (Compass computes per-column type across loaded docs **[Observed]**; Hacivat prefers schema probabilities).
  5. Cell rendering: Present → formatted by BSON type (ObjectId short form, Date in user TZ with UTC tooltip, Decimal128 exact, Int64 exact, Binary/UUID, nested Document `{n fields}`, Array `[n]`); Missing → distinct "absent" style (not NULL); Null → NULL style.
  6. Drill-down: clicking a Document/Array cell pushes `B' = B + [key]` (or `[key, index]`) onto a breadcrumb; the grid switches to the sub-table; back navigation pops.
  7. Editing: cell edit maps to `$set: {"<dotted path>": value}` (escape keys containing `.`/`$` using `$setField` aggregation-pipeline updates when needed); "remove field" → `$unset`; type change via type dropdown (Compass offers types dropdown **[Observed: types-dropdown.tsx]**); concurrency via original-value predicate (ALG-EDIT-01 / Compass pattern **[Observed]**).
  8. Copy (ALG-GRID-04): Present scalar → canonical text (Extended JSON relaxed for complex types), Missing → empty with private JSON `missing: true`.
- **Data structures:** `ColumnModel { cols: Vec<Col>, index: HashMap<PathKey, usize> }`; path keys as `Vec<Segment>` where `Segment = Key(String) | Index(u32)`.
- **Complexity / performance notes:** Column discovery O(page fields); cell access O(depth).
- **Edge cases (exhaustive list):** documents with thousands of fields (column virtualization essential); keys with `.`/`$`/empty string `""`; same key with Document in some docs and scalar in others at depth > 0 (don't expand; show Mixed); arrays of mixed docs/scalars in drill-down (sub-table of elements where scalar elements show in a single `value` column); sparse fields (presence 1%) → hide by default under "+N sparse fields"; schema/sample missing fields present in later pages (append columns at the end, never reorder silently); editing Missing cell → creates field; Int64/Decimal display precision; Dates outside JS range; Binary subtype 4 UUID formatting; encrypted fields read-only; projection queries (only projected paths); `_id` not ObjectId (string/number/compound documents) → still first; timeseries/views read-only.
- **Test cases (input → expected):**
  1. Docs `{_id:1,a:{b:1,c:2}}`, `{_id:2,a:{b:3}}` with d = 1 → columns `_id`, `a.b`, `a.c`; doc 2 `a.c` Missing.
  2. Docs `{a:1}`, `{a:"x"}` → column `a` typed Mixed.
  3. Drill into `items` (array of docs) of a document → sub-table with columns union of element keys, rows = elements, breadcrumb `items`.
  4. Edit Missing `a.c` to 5 on doc 2 → `$set {"a.c": 5}` with filter `{_id:2, "a.c": {$exists:false}}`.
  5. Key `"x.y"` (literal dot) → column header shows escaped `x\.y`; edit uses `$setField`.
- **Prior art and evidence:** Compass table view (SSPL — behavior) **[Observed]**; MS paths **[Observed]**.
- **Used by:** Hacivat (Mongo table view; also JSON columns in SQL results).

### ALG-MONGO-03: JSON tree view virtualization

- **Problem:** Render large JSON/BSON documents (MBs, deep nesting, arrays with 100k elements) and lists of documents as an expandable tree with smooth scrolling, keyboard navigation, search and editing.
- **Inputs / outputs:** Inputs: list of root values (documents or a single JSON value), expansion state, row height, viewport, search query. Outputs: visible rows `[ {depth, keyLabel, valuePreview, kind, nodeId, expandable, expanded} ]`.
- **Algorithm:**
  1. **Node model**: lazily materialized tree; each node knows `childCount` without materializing children (for BSON: parse lazily using offsets — BSON documents are length-prefixed, so children can be indexed by scanning element headers; for JSON from SQL columns: parse with a streaming parser into a compact tape (simd-json style) with child offsets).
  2. **Visible-size bookkeeping**: `visibleSize(node) = 1 + (expanded ? Σ visibleSize(children) : 0)`; for nodes with large child counts store children's visible sizes in a Fenwick tree **only when expanded** (collapsed children contribute 1 each, so an expanded node with n collapsed children has `visibleSize = 1 + n` without materialization).
  3. **Row → node mapping**: descend from the root list: subtract sizes using the Fenwick prefix search to find which child contains the row (O(depth × log n)).
  4. **Chunking huge arrays/objects**: if `childCount > CHUNK` (e.g., 100), present synthetic group rows `[0…99]`, `[100…199]`, … (recursively chunked for > 10^4) — each group expandable; Compass instead uses "show more" in steps of 25/1000 **[Observed]**; chunking keeps scroll extents bounded and navigation fast.
  5. **Expansion**: expanding a node updates ancestors' visible sizes (O(depth × log n)); "expand all" is bounded (refuse beyond, e.g., 100k visible rows, or expand to depth k).
  6. **Rendering**: rows rendered by ALG-GRID-01 (single column, fixed height); key label + value preview truncated to width; long strings shown with "…" and an inspector panel for full value (paged load for very large strings).
  7. **Search**: incremental scan over the document tape in a background task, producing matching node paths; navigating to a match expands ancestors and scrolls; matches inside collapsed chunks counted per chunk.
  8. **Editing** (Mongo): edit actions produce update operators per path (ALG-MONGO-02 step 7); for JSON columns in SQL, edit produces the new JSON text for the cell.
  9. **Height estimation** for list-of-documents views (each document a collapsible card): estimate from number of top-level fields (Compass estimates `min(25, fields) × rowHeight + paddings` **[Observed: virtualized-document-list-view.tsx:21-45]**); real heights measured after render and corrected without scroll jumps (anchor at top visible row).
- **Data structures:** node arena with `parent`, `firstChildOffset`, `childCount`, `expanded`, optional `Fenwick<u64>` per expanded large container; path-to-node cache.
- **Complexity / performance notes:** Open a 50 MB JSON: tape build O(size) once (background), first paint after root rows only. Row lookup O(depth log n).
- **Edge cases (exhaustive list):** extremely deep nesting (stack-safe iterative traversal); 1M-element arrays (chunking); very long keys; duplicate keys in JSON objects (keep all, show warning); invalid JSON in a text column (show raw with error position); BSON types without JSON equivalents (render Extended JSON); huge strings (multi-MB) — preview only; binary blobs; numbers beyond f64 precision in JSON (keep source text); NaN/Infinity (Extended JSON); RTL text in values (`dir=auto`); expand state persistence across refresh (by path); document replaced while expanded (generation bump; attempt to restore expansions by path); keyboard: Right expands/goes to first child, Left collapses/goes to parent, Home/End, type-ahead by key.
- **Test cases (input → expected):**
  1. Array of 1,000,000 numbers expanded with CHUNK = 100 and two-level chunking → visible rows = 1 (array) + 100 group rows `[0…9,999]`, `[10,000…19,999]`, …; expanding one group adds 100 sub-group rows of 100 elements each; no element nodes materialized until a sub-group is expanded; scrolling instant.
  2. Expand node with 3 children inside a list of 10 docs → total visible rows increase by 3.
  3. Search "foo" matches inside chunk `[500…599]` → chunk auto-expands on navigation.
  4. 64-digit JSON number → displayed exactly.
- **Prior art and evidence:** Compass list view & "show more" (SSPL — behavior) **[Observed]**; CodeMirror height map & BigScaler (MIT) for huge extents **[Observed: ui-02]**; react-window VariableSizeList measurement approach **[Observed: compass-components virtual-list.tsx]**.
- **Used by:** Hacivat (Mongo documents, JSON/JSONB cells, Redis JSON values, MCP result inspection).

### ALG-MONGO-04: Explain-plan tree → flame graph transform

- **Problem:** Turn a query plan tree with cumulative timings (MongoDB `executionStats`, also PostgreSQL `EXPLAIN (ANALYZE, FORMAT JSON)`, MySQL `EXPLAIN ANALYZE`) into a flame graph (or icicle chart) where width = time, nesting = plan structure, and color/labels highlight expensive stages.
- **Inputs / outputs:** Inputs: plan tree with per-node `name`, `children`, `cumulativeTime` (may be missing), `rows` (nReturned / actual rows), `loops`, `docsExamined/keysExamined`, shard markers. Output: list of rectangles `{nodeId, x, width, depth, label, selfTime, totalTime, pctOfTotal, color}` plus a self-time table.
- **Algorithm:**
  1. **Normalize children**: MongoDB children are found under `inputStage`, `inputStages[]`, `innerStage`, `outerStage`, `thenStage`, `elseStage`, `shards[]`, `executionStages` (in that order) **[Observed: explain-plan-helper getChildStages]**; aggregation explains with `stages[]` (pipeline) become a chain: stage k is the parent of stage k−1 (later stages consume earlier output). PostgreSQL: `Plans[]`; MySQL: nested `inputs`.
  2. **Compute times bottom-up** (post-order):
     - `childTime = combine(children totals)` where `combine = max` for parallel children (shards; PG parallel workers' Gather children) and `sum` for sequential children (e.g., PG nested loop inner/outer are interleaved — use sum of exclusive times; MongoDB OR/SORT_MERGE inputs are sequential → sum). Compass uses **max for all children** with a NOTE that this assumes parallel execution **[Observed: explain-tree-data.ts:36-42]** — Hacivat uses a per-stage-kind table (parallel: `SHARD_MERGE`/`SHARDING_FILTER` shard children, PG `Gather`, `Gather Merge` worker subplans; sequential otherwise).
     - `total = node.cumulativeTime` if present else `childTime` (Compass keeps input time when the node has no time or is a shard **[Observed: lines 44-55]**); for PG, `total = actual_total_time × loops` (per-loop times!), with workers adjustments.
     - `self = max(0, total − childTime)` (clamp: estimates can make children exceed parent due to timer granularity — MongoDB `executionTimeMillisEstimate` is millisecond-granular).
  3. **Layout (flame graph)**: root `x = 0`, `width = root.total` (or the sum of root self + children if root missing time); for each node, children laid out left-to-right in plan order starting at the parent's `x`, child width = child.total scaled so that `Σ child widths ≤ parent width − parent.self` (if Σ children > available because of `max` semantics or estimation noise, scale children proportionally to fit and mark `scaled = true`); depth = tree depth (icicle: root at top).
  4. **Zero-time handling**: nodes with total 0 get a minimum visual width (e.g., 2 px) so they remain clickable; label "<1 ms".
  5. **Labels/color**: label = stage name + key detail (index name for IXSCAN, collection for COLLSCAN, `Seq Scan on t`); color by category (scan: COLLSCAN red-ish if large docsExamined/nReturned ratio; index: green; sort/blocking: orange; fetch; network/shard: blue); accessibility: patterns + text, not color only.
  6. **Highlights**: compute ratios `docsExamined / nReturned`, `keysExamined / nReturned`, `rows estimate vs actual` (PG) and flag outliers; in-memory SORT and `usedDisk` flags.
  7. **Self-time table**: aggregate self time by stage name/kind (sorted desc) — the "top-down vs bottom-up" view.
  8. **Interactions**: click to zoom (set selected node as new root: x-scale = node.total), breadcrumb to zoom out, hover tooltip with all details; keyboard navigation (arrow keys traverse tree); screen-reader alternative = an indented tree table with the same metrics.
- **Data structures:** normalized `PlanNode { id, kind, label, children: Vec<NodeId>, total: f64, self: f64, rows, loops, flags }`; rectangle list.
- **Complexity / performance notes:** O(nodes) for time computation and layout. Plans are small (≤ thousands of nodes) except huge `$lookup` / PG partition plans (thousands of partitions → collapse identical siblings into a group node "Append × 1,000 partitions").
- **Edge cases (exhaustive list):** no execution stats (queryPlanner verbosity) → layout by node count (uniform widths) with "no timing" banner; rejected plans (show separately); sharded explain with per-shard winning plans and differing indexes (`MULTIPLE`) **[Observed]**; aggregation explain where `$cursor` stage embeds a find plan; `$lookup`/`$unionWith` subpipelines (child plans inside stage details); PG `loops > 1` (multiply), `never executed` nodes, CTE scans and InitPlans/SubPlans (attach as children, mark), parallel workers; MySQL iterator trees; timer granularity (0 ms everywhere) → fall back to work metrics (docsExamined) as width with a toggle; negative self times (clamp); cyclic references impossible but guard; extremely wide trees (Append of 10k children) → grouping.
- **Test cases (input → expected):**
  1. Mongo: FETCH(total 10) → IXSCAN(total 7) → FETCH self 3, IXSCAN self 7; widths 10 and 7.
  2. SHARD_MERGE with shards A(total 40), B(total 25), node total 42 → childTime = max = 40, self = 2. Default "fit" mode: children sum 65 > available 40 → scaled proportionally to widths 24.6 and 15.4 with `scaled = true`. Alternative "parallel lanes" mode: each shard drawn at full width 40/25 in its own lane below the parent.
  3. PG: Nested Loop (actual_total_time 5.0, loops 1) with inner Index Scan (0.01 ms × 1000 loops = 10 ms) → inner total 10 > parent 5 → parent self clamped 0; flag "timing inconsistency (instrumentation overhead)".
  4. All zero times → uniform widths, banner shown.
- **Prior art and evidence:** Compass explain tree (SSPL — behavior) **[Observed: explain-tree-data.ts; explain-plan-helper]**; flame graphs (Brendan Gregg) and PG plan visualizers (e.g., PEV2 shows per-node exclusive times) **[Inferred: general knowledge]**.
- **Used by:** Hacivat (plan view for all engines); Perde (optional: show captured plans in audit details).

---

## 6. Implications for Hacivat / Karagöz / Perde

**Redis (Hacivat):**
1. Rust core implements ALG-REDIS-01/02 using `fred` or `redis-rs` (cluster-aware, pipelining). Prefer `fred` for built-in cluster routing, RESP3, and pipelines **[Inferred: crate capabilities]**; verify SCAN per node API.
2. Default budgets: list mode SCAN COUNT 1000/batch target 500 keys; tree mode batch target 10,000 (RI uses 500 / 10,000 **[Observed]**); scan threshold 10,000 per click (RI default **[Observed]**) with "continue".
3. Never call `KEYS`, `HGETALL`, `SMEMBERS`, `LRANGE 0 -1`, `MONITOR` (except in an explicit Profiler with warnings about server load), `MEMORY USAGE … SAMPLES 0` by default on large aggregates.
4. Workbench: CodeMirror with a Redis command grammar and completions generated at build time from vendored `commands.json` (license check: redis-doc content licensing) and at runtime from `COMMAND DOCS` (Redis ≥ 7) — this avoids network fetches and matches the actual server version.
5. Value viewers: decoders behind a trait (UTF-8, hex, JSON, msgpack, protobuf w/ user descriptor, pickle (display-only, never unpickle), PHP/Java serialized parsers (display-only), vectors f32/f64, DateTime) — RI's list **[Observed]** is a good checklist; **never execute** deserialized content.
6. Bulk actions server-side in the core with progress events, cancel, UNLINK, per-key error report, and a pre-flight "dry run" count using SCAN + TYPE.
7. Analysis report: sampled memory/TTL/namespaces as in RI, all computed in the core; export as JSON/Markdown.

**MongoDB (Hacivat):**
1. Use the official `mongodb` Rust crate (async, Tokio); keep BSON types (no lossy JSON conversion); `$sample`-based schema inference implementing ALG-MONGO-01 (consider porting mongodb-schema's Apache-2.0 logic with attribution).
2. Views: list (ALG-MONGO-03), JSON (CodeMirror per doc or one virtualized editor), table (ALG-MONGO-02 on ALG-GRID-01).
3. Query bar: CodeMirror with completion from the schema path catalog + operators; aggregation builder with per-stage previews (limit 10, guard `$limit` before blocking stages, 700 ms debounce, cancellation per stage).
4. Edits: optimistic concurrency predicate on original values; shard key inclusion; Extended JSON relaxed/canonical toggles.
5. Explain: ALG-MONGO-04 shared with SQL engines.
6. Count: `estimatedDocumentCount` for unfiltered, `countDocuments` with `maxTimeMS` cap (5 s like Compass **[Observed]**) for filtered.

**Karagöz:** enforce command allow-lists (Redis: deny `FLUSHALL/FLUSHDB/CONFIG SET/MONITOR/DEBUG/KEYS` unless role permits; Mongo: deny `dropDatabase` etc.), scan budgets per session, and audit bulk actions with counts.

**Perde:** display audit of Redis bulk actions and Mongo bulk updates; per-role policies (e.g., "read-only Redis browser").

---

## 7. i18n considerations

1. RedisInsight's retrofit shows the cost of late i18n: thousands of keys, test fallout, and a temporary query-param language switch **[Observed]**. Hacivat: Fluent from day one.
2. Flat vs hierarchical keys: RI uses flat dotted keys with `keySeparator: false` **[Observed]**; Fluent ids are flat by design (`redis-browser-scan-more = Scan more`), with attributes for tooltips/aria labels.
3. Plurals and numbers: "Scanned { $scanned } of { $total } keys", "{ $count -> [one] key *[other] keys }"; totals formatted with `NUMBER($total)`; unknown totals ("≥") as a separate message variant.
4. TTL formatting: human durations ("2 d 3 h") must be localized (Intl.DurationFormat where available, else Fluent patterns); Turkish abbreviations ("2 g 3 sa").
5. Sorting keys/fields: byte-wise for data (keys are binary); never `localeCompare(…, 'en')` (RI does **[Observed: constructKeysToTree.ts:124-128]**) — for Turkish users, `en` collation is merely wrong; byte order is predictable and matches Redis.
6. Schema field names and BSON type names are data/technical terms — keep untranslated ("ObjectId", "Int64"); translate descriptive labels ("Mixed", "Missing", "Undefined" → "Karışık", "Yok", …) consistently with documentation.
7. Explain plan stage names stay as engine identifiers; descriptions/tooltips translated.
8. Dates in documents displayed in the user's chosen data locale and time zone with UTC tooltip.
9. Search within JSON/tree is case-insensitive using Unicode default case folding (not locale-tailored) so Turkish `I/ı` behaves predictably for technical data; offer a "Turkish case-insensitive" option if users request it.

---

## 8. Risks, pitfalls, open questions

- **License**: RedisInsight and Compass are SSPL — no code reuse; behavior-level learnings only. mongodb-schema is Apache-2.0 (reusable with attribution/NOTICE).
- Pitfall: SCAN COUNT is a hint; budgets must count calls and elapsed time, not only COUNT sums (RI's `scanned += COUNT` is an approximation **[Observed]**).
- Pitfall: `MEMORY USAGE … SAMPLES 0` on big aggregates is O(N) — RI uses it for every loaded key **[Observed]**; Hacivat defaults to sampled estimates.
- Pitfall: Compass's `max` combination of child times can under-report sequential children (acknowledged in its comment **[Observed]**).
- Pitfall: tree counts from partial scans can mislead — always label as "loaded".
- Pitfall: `$sample` on huge collections with a filter may still scan heavily; respect `maxTimeMS` and secondaries.
- **[Unknown]** Exact RedisInsight UI behavior of "scan more" thresholds across cluster nodes with very uneven key distribution.
- **[Unknown]** Licensing of `commands.json` content in redis-doc for bundling (verify before vendoring).
- **[Unknown]** Whether `fred`'s cluster SCAN helpers expose per-node cursors suitable for resumable composite cursors (spike).
- Open question: Should Hacivat's schema inference also run on SQL JSON/JSONB columns (same ALG-MONGO-01 over sampled JSON values)? Recommended yes — same code path with JSON type mapping.
