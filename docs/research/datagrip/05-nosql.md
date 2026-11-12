# DataGrip 2024.3.5 — NoSQL support (Redis, MongoDB, Cassandra, DynamoDB, Couchbase, Elasticsearch)

> Covers: how DataGrip connects to, introspects, displays, queries and (partly) edits non-relational stores, and which parts are implemented by JetBrains-maintained **open-source** JDBC drivers. Connection/auth details are in `02` (§3.8), sessions/read-only in `03`, generic introspection/model in `04`. Conventions: `00-method-and-legal.md`.

---

## 1. Scope and sources

| Source | Version | Licence |
|---|---|---|
| `database-plugin.jar` dialect XML: `dialects/{redis,mongo,dynamo,couchbase,cassandra}/*.xml`, `sql/dialects/{redis,mongo,dynamo,couchbase,cassandra}/*.xml`, `mongo-js-external.xml`, `mongo-sql.xml` | DataGrip 2024.3.5 | Proprietary |
| Mongo signature DSL files `sql/dialects/mongo/js/psi/resolve/{types/members/*.txt, symbols/*.txt, symbols/operators/*.txt}` (417 lines) | 2024.3.5 | Proprietary — described, not copied |
| Dynamo PartiQL function prototypes `sql/dialects/dynamo/functions.xml` | 2024.3.5 | Proprietary |
| `CassIntroQueries.sql`, `cassandra.minicat.xml.gzip` | 2024.3.5 | Proprietary |
| Class names: `dialects/redis/**`, `dialects/mongo/**` (incl. ~100 `translator/*`), `sql/dialects/mongo/js/**`, `dialects/{dynamo,couchbase,cassandra}/model/*` | 2024.3.5 | names only |
| Help: `redis`, `mongodb`, `how-to-fetch-more-fields-for-mongodb-introspection`, `sql-for-mongodb`, `editing-data-in-mongodb`, `documentdb`, `dynamodb`, `couchbase-query`, `apache-cassandra`, `how-to-connect-to-cassandra-with-ssl`, `other-databases` | 2024.3.5 offline (2024) | Proprietary docs; online `https://www.jetbrains.com/help/datagrip/<topic>.html` |
| `DataGrip/redis-jdbc-driver` tag **v1.5** (`QueryParser.java`, `Analyzer.java`, `RedisJedisClusterClient.java`, `RedisResultSetBase.java`, `RedisListResultSet.java`, `ResultParsers.java`, README) | the version listed in `database-artifacts.xml` | **Apache-2.0** |
| `DataGrip/mongo-jdbc-driver` tag **v1.20** (`DriverPropertyInfoHelper.java`, `schema/MetaCollection.java`, `build.gradle`), HEAD for drift | listed version 1.20 | **Apache-2.0** |
| User's Redis model cache (element names/counts only) | DataGrip 2024.3 | n/a |

---

## 2. Executive summary

1. All NoSQL stores go through **JDBC drivers** (mostly JetBrains-built): Redis `jdbc.RedisDriver` (Jedis-based), MongoDB `com.dbschema.MongoJdbcDriver` (official Java driver + **embedded mongosh via GraalJS**), DynamoDB `org.jetbrains.DynamoDriver`, Couchbase `com.intellij.CouchbaseJdbcDriver`, Cassandra `com.dbschema.CassandraJdbcDriver`; Elasticsearch uses Elastic's X-Pack SQL JDBC driver as a *basic-support* generic DBMS `[Observed: drivers XML; oss-mongo build.gradle@v1.20; oss-redis README@v1.5]`.
2. Each store is **mapped onto the relational skeleton** Root → Schema → table-like → column-like, plus store-specific kinds (Redis key types; Cassandra keyspace/UDT/MV; Dynamo keys/indexes/provisioning; Couchbase bucket/scope/collection) `[Observed: model classes, kinds]`.
3. Each has its **own console language**: Redis commands (`.redis` file type), MongoJS (JavaScript subset + "SQL for MongoDB"), PartiQL for DynamoDB (dialect "Dynamo"), N1QL/SQL++ ("CouchbaseQuery"), CQL ("CassandraQL", `.cql`) `[Observed]`.
4. **Redis**: introspection = server info + DB count + keyspace + `SCAN` with a user **key filter** and **COUNT hint** + `TYPE` per key + modules + cluster slots; keys become typed leaf objects (`string`, `list`, `set`, `sorted_set`, `hash_table`, `stream`, `json_document`, `data_structure`, `unknown`); the explorer shows a **delimiter-based virtual folder tree**; per-type delegates generate read queries; **data editing is not supported** in 2024.3 `[Observed: help:redis; classes; model cache]`.
5. **Redis console grammar** (driver v1.5): exactly **one command per query**; optional comment hints `-- node = host:port` (route to a cluster node) and `-- <column> : v1 v2 …` (adds a synthetic leading column, e.g. key names); `SELECT n` switches DB; `KEYS`/`SCAN MATCH` are "key pattern" queries; in cluster mode they require a **hash-tag pattern**, and `DBSIZE`, `WAIT`, `WAITAOF` are rejected; blocking commands get a dedicated timeout `[Observed: oss-redis@v1.5]`.
6. **MongoDB**: console language MongoJS — DataGrip parses JS for completion/resolve/inspections using a **signature DSL** (methods with argument *contexts* selecting operator lists, return types, `[pure]` flags), while execution is done by **mongosh inside the driver** (JMongosh + GraalJS) `[Observed]`.
7. **Mongo schema inference** (driver v1.20): read the **first N documents** of each collection (`fetch_documents_for_metainfo`, default 10; HEAD later switched to `$sample`), union fields recursively (sub-documents → nested field groups; arrays of documents → nested; other arrays → `array`), map Java types to SQL types (Integer, Double, Date/Timestamp, else VARCHAR), a field becomes *non-mandatory* when missing in any sampled document; indexes from `listIndexes` (`_id_` = PK) `[Observed: oss-mongo MetaCollection.java@v1.20]`.
8. **SQL for MongoDB**: `SELECT` with JOIN (INNER/LEFT, single equality `ON`), WHERE, GROUP BY, HAVING, ORDER BY/SORT BY, LIMIT, OFFSET, DISTINCT, nested SELECT with alias, array/map literals, object constructors, named arguments (`=>`, `:=`) — translated to an aggregation pipeline (*Show JS Script*); limits: `COUNT(*)` only, no `USING`, no sub-select as JOIN target, no `map/reduce/filter/let` operators, `LIKE` needs a literal, aliases without dots, no aggregate-in-aggregate `[Observed: help:sql-for-mongodb]`.
9. **Mongo grid editing is supported** (edit cells, add/delete documents, add/delete columns(fields), value editor with type change, `<unset>` vs null) `[Observed: help:editing-data-in-mongodb]`; Couchbase and Redis editing are not `[Observed]`.
10. Cassandra introspects `system_schema.*` (and legacy `system.schema_*` for 2.x, `system_virtual_schema.*`, `system_auth.*`) via a native pack; DynamoDB via `ListTables/DescribeTable` `[Inferred]` with PartiQL functions (`LIST_APPEND`, `SET_ADD`, `SET_REMOVE`, `ATTRIBUTE_TYPE`, `BEGINS_WITH`, `CONTAINS`, `SIZE`) `[Observed]`.

---

## 3. Detailed findings

### 3.1 Redis

#### 3.1.1 Connection

- Templates `standalone` `jdbc:redis://[[user:]password@]host[:6379][/db][?props]` and `cluster` `jdbc:redis:cluster://[[user:]password@]h1[:p1],h2[:p2]…[/db][?props]` `[Observed: redis-drivers.xml]`.
- Driver properties `[Observed: oss-redis README@v1.5]`: `user`, `password`, `database` (int, 0), `connectionTimeout` (2000 ms), `socketTimeout` (2000 ms), `blockingSocketTimeout` (0 = infinite), `clientName`, `ssl` (false), `verifyServerCertificate` (true), `hostAndPortMapping` (`{toHost:toPort=fromHost:fromPort,…}` — required in cluster mode behind tunnels: node addresses announced by the server are rewritten), `verifyConnectionMode` (true; checks URL prefix vs server mode). Sentinel: not supported.
- Errors/fixes: mode mismatch → *Use "jdbc:redis:cluster://"*; mode retrieval failure → *Disable connection mode verification*; SSH + cluster requires all nodes in URL; outdated driver (v1.0 lacks column hints); unknown command; "The key does not exist"; "The value for the key of type '{0}' cannot be represented as a table" `[Observed: bundle; RedisErrorHandler$*]`.
- Auth: User & Password (ACL user optional), No auth `[Observed: help]`.

#### 3.1.2 Introspection

- Holders `RedisIntroQueries$ServerInfo, DbCountInfo, KeyspaceInfo, KeysScanInfo, ModulesInfo, SlotsInfo`; `RedisIntrospector$Mode`, `$DefaultNature`, `$RedisSchemaRetriever` `[Observed]`.
- `[Inferred mapping]` ServerInfo ← `INFO server` (version, mode); DbCountInfo ← `CONFIG GET databases` (fallback 16; managed services often forbid CONFIG); KeyspaceInfo ← `INFO keyspace` (`db0:keys=…,expires=…`) to list non-empty DBs and sizes; KeysScanInfo ← `SCAN cursor MATCH <filter> COUNT <hint>` (+ `TYPE` per key, possibly pipelined or `SCAN … TYPE`); ModulesInfo ← `MODULE LIST` (ReJSON → `json_document`, RedisBloom/TS → `data_structure`); SlotsInfo ← `CLUSTER SLOTS` (nodes for per-node scans via `-- node = host:port` hints, since `SCAN` is node-local in a cluster).
- Options `[Observed: help:redis; bundle]`: **Default key filter** ("Pattern for filtering keys during introspection"; docs call it a regular expression for the introspector, while Redis `MATCH` is a glob — `[Unknown]` whether DataGrip converts or post-filters), **COUNT option for SCAN command** ("Hint for SCAN… number of elements obtained in one iteration"), **Key parts delimiter** ("A character or a couple of characters that is used in a key to organize keys in a hierarchy").
- Model: `RedisRoot` (`ServerVersion`) → `RedisSchema` (one per DB index; `Current` flag; cluster: only db 0 — cluster client rejects other indexes with "DB index is out of range") → keys as kinds `string`, `list`, `set`, `sorted-set`, `hash-table`, `stream`, `json-document`, `data-structure`, `unknown`; **no TTL/size/encoding stored** `[Observed: cache element names; oss-redis RedisJedisClusterClient]`. A real cache contained 5,006 `hash-table` keys under 16 schemas `[Observed counts]` → keys are fully materialized in the model (scale concern).
- "By default, only the default database is introspected" — other DB indexes via Schemas tab `[Observed: help]`.

#### 3.1.3 Explorer, console, grid

- `RedisDvStructureExtension` `[Observed]` + delimiter option ⇒ virtual folders by key prefix (model stays flat).
- Console: file type `Redis` (`.redis`), own lexer/parser, DDL vs DML statement PSI (`RedisDdlStatement`, `RedisUpsertStatement`), stub index of upserted key names (`RedisDmlNameIndex`) for completion, resolve extension, formatter `[Observed]`.
- Driver-level query syntax (v1.5) `[Observed: QueryParser.java]`:
  - Tokenized into lines; a line starting with `--` is a comment; `-- node = host:port` is a **node hint** (exactly 4 tokens); `-- <name> : v1 v2 …` is a **column hint**; at most one of each; exactly one **command line**.
  - `SELECT <n>` (single param) → set-database query; `KEYS <pattern>` / `SCAN … MATCH <pattern>` → key-pattern queries; module commands parsed by dedicated parsers (JSON, Search, Bloom, Cuckoo, CMS, TopK, TDigest, TimeSeries).
  - Blocking commands (`BLMOVE, BLMPOP, BLPOP, BRPOP, BRPOPLPUSH, BZMPOP, BZPOPMAX, BZPOPMIN`) flagged and executed with blocking semantics.
  - Cluster: commands routed by sample key (`sendCommand(sampleKey, …)`); key-pattern queries require a hash-tag (`{…}`) pattern; `DBSIZE`, `WAIT`, `WAITAOF` unsupported; node hint executes on a specific node connection.
  - Results: per-command result parsers map replies to list/map/object result sets (e.g. `ZRANGE … WITHSCORES` → tuple member/score; `HRANDFIELD … WITHVALUES` → map); unknown → default parser.
  - Column hint: prepends a synthetic string column named by the hint, row *i* gets the *i*-th hint value (used e.g. to show key names next to `MGET`-style results) `[Observed: RedisResultSetBase]`.
- Grid: per-type delegates `Redis{String,List,Set,SortedSet,HashTable,Stream,JsonDocument,Unknown}{DmlHelper,HookUpHelper}Delegate`, `Redis*SelectGenerator`, `Redis*WrapInSelectGenerator`, `RedisWhereClauseGenerator`, `RedisTableKindGenerator` `[Observed]` → `[Inferred]` opening a key generates a type-specific read command (string `GET`, list `LRANGE 0 -1`, set `SSCAN/SMEMBERS`, zset `ZRANGE 0 -1 WITHSCORES`, hash `HGETALL/HSCAN`, stream `XRANGE - +`, JSON `JSON.GET`) with a column hint, and grid filters become command-side or client-side predicates.
- DDL producers only `RedisAlterKey` (rename/TTL `[Inferred]`) and `RedisDropKey` (`DEL`/`UNLINK`) `[Observed]`. **"For Redis, data editing is currently not supported."** `[Observed: help:redis]`.

### 3.2 MongoDB and DocumentDB

#### 3.2.1 Connection and driver

- Templates: standard `[jdbc:]mongodb[+srv]://[user[:password]@]h1[:27017],h2…[/db][?authSource=…&authMechanism=…&authMechanismProperties=…&…]` and Atlas SRV `[jdbc:]mongodb+srv://…` (no port) `[Observed]`. Drivers `mongo.4` (≥ 4.0), `mongo` (3.x), `documentdb` (`retryWrites=false`) `[Observed]`.
- Driver properties (v1.20) `[Observed: DriverPropertyInfoHelper]`: `authMechanism` (GSSAPI, MONGODB-AWS, MONGODB-X509, PLAIN, SCRAM-SHA-1, SCRAM-SHA-256), `authSource`, `AWS_SESSION_TOKEN`, `SERVICE_NAME`, `SERVICE_REALM`, `CANONICALIZE_HOST_NAME`, `uuidRepresentation` (standard | javaLegacy | csharpLegacy | pythonLegacy; default standard), `serverSelectionTimeoutMS` (10000), `connectTimeoutMS` (10000), `fetch_documents_for_metainfo` (10), `max_connection_pool_size` (3 per IDE connection), `auto_encode_username_and_password` (true), `appName`, `tlsAllowInvalidCertificates` (false), `tlsAllowInvalidHostnames` (false), `retryWrites`. (HEAD adds `MONGODB-OIDC` + `oidc*` properties — not in 2024.3.5's driver.)
- Execution engine: `mongodb-driver-sync 4.11.1`, `org.graalvm.js 22.3.1`, `JMongosh-0.9.jar`; classes `MongoshScriptEngine`, `LazyShellHolder`, `PrecalculatingShellHolder` `[Observed: build.gradle, tree@v1.20]` ⇒ console scripts run in an embedded mongosh, so shell helpers (`use`, `db.getSiblingDB`, cursor methods, `show …`-like commands per tests) behave like real mongosh `[Inferred]`.
- Tests in the driver repo enumerate supported shell operations: aggregate, bulkWrite, count/countDocuments/estimatedDocumentCount, createIndex(es), dataSize/storageSize/totalSize/totalIndexSize, deleteOne/Many, distinct, drop, dropIndex(es), find/findOne (+projection, regex, options), findOneAnd{Delete,Replace,Update}, getIndexes, insertMany, mapReduce, replaceOne, update/updateOne/updateMany, cursor modifiers (limit, skip, max, collation, comment, map, showRecordId), db-level adminCommand, createCollection, createView, getCollection, listCommands, runCommand, serverBuildInfo, `use`, fsyncLock… `[Observed: test resource names]`.

#### 3.2.2 Introspection / schema inference

- No Mongo introspector registration → generic JDBC introspector over the driver's `DatabaseMetaData` `[Observed]`.
- Sampling (v1.20) `[Observed: MetaCollection.java@v1.20]`: `find()` cursor, take up to N documents (N = `fetch_documents_for_metainfo`); there is also a "random records" variant in the code; error code 13 (unauthorized) → empty schema without failing. HEAD replaced it with `$sample` aggregation `[Observed: HEAD]`.
- Docs: "DataGrip fetches the first 10 documents from each collection to get information about the fields. You can customize this … `fetch_documents_for_metainfo`" `[Observed: help]`.
- Field typing: value class name (simple name) as type name; SQL type: Integer→INTEGER, Timestamp→TIMESTAMP, Date→DATE, Double→DOUBLE, else VARCHAR; sub-document → nested `MetaJson` map; list of documents (or empty list) → nested list field; other list → `array`; null → "String"; first type seen wins on later documents (no type union) `[Observed]`.
- Mandatory: true unless a later document lacks the field ⇒ "present in all sampled docs" `[Observed]`.
- Indexes: `listIndexes`; `_id_` → primary; `unique` flag; key directions 1/-1, 0 for text/2dsphere/hashed; index fields not seen in samples are fetched with a targeted `find({path:{$exists:true}}).projection(path).limit(1)` or stubbed as VARCHAR (HEAD) `[Observed]`.
- Explorer: `DocumentDvStructureExtension` shows nested fields hierarchically (shared with Couchbase) `[Observed]`.

#### 3.2.3 MongoJS console (DataGrip side)

- Languages `MongoJS` (console) and `MongoJSExt` (inject into plain `.js` files: completion, resolve, docs, goto, inspections) `[Observed: mongo.xml, mongo-js-external.xml]`.
- Parser: JavaScript grammar (ES6+ error messages: arrow functions, classes, template strings, import/export) `[Observed: MongoJSBundle parser messages]`; resolve model symbols `MongoJSDatabase`, `MongoJSCollection`, `MongoJSField`, `MongoJSMethod`, `MongoJSOperator`, `MongoJSUseStatementScope` (`use db` changes resolve scope) `[Observed]`.
- **Signature DSL** `[Observed format; described]`: one member per line: `name(ARGCTX, ARGCTX…)[: RETURN] [flags]`. Member files: `collection.txt` (48), `cursor.txt` (31), `database.txt` (65), `date.txt`; top-level functions (41: `ObjectId`, `ISODate(): DATE`, `NumberLong`, `NumberDecimal`, `UUID`, `BinData`, `Timestamp`, `MinKey`, `MaxKey`, `DBRef`, `Code`, `BSONSymbol`, …). Argument contexts: `QUERY`, `PROJECTION`, `UPDATE_STAGE`, `AGGREGATION_STAGE`, `INDEX`, `DOCUMENT`, `SORTING`, `FIELD`, `COLLECTION`, `DATABASE`; returns `CURSOR`, `COLLECTION`, `DATABASE`, `DATE`; flags `pure` (no side effects) and `typed` (result type known for grid). 54 `[pure]` members observed.
- Operator lists per context (`symbols/operators/*.txt`): `query` (7: `$and/$or/$nor/$expr/$jsonSchema/$text/$where`…), `query_expression` (23), `query_geospec` (8), `query_text` (3), `projection_expression` (2), `update_stage` (13), `update_expression` (3), `aggregation_stage` (30), `aggregation_expression` (131), `aggregation_accumulation` (12) `[Observed counts]`. Each operator line can declare its own argument context (`$expr(AGGREGATION_EXPRESSION)`, `$text(QUERY_TEXT)`) → recursive context switching inside object literals.
- Inspections: Resolve (unknown collection/field), SideEffects (non-pure call on a read-only connection), Deprecation (e.g. legacy `insert` → `insertOne`, with quick fix) — for console and external JS `[Observed]`.
- Evaluation helper / execution-flow analyzer / notebook line markers for MongoJS `[Observed]`.

#### 3.2.4 SQL for MongoDB

- Separate dialect `MongoSqlDialect` (lexer/parser) + `MongoToDatabaseScriptTranslator` (`toDatabaseScriptTranslator language="MongoDB"`) + ~100 translator classes (builders for select, from, join, where, group by, having, order by, limit, offset, literals, function calls, named arguments, column references, aliasing; operators: arithmetic, comparison, IN, regex(LIKE)) `[Observed]`.
- Behaviour and limits `[Observed: help:sql-for-mongodb]` (see summary #8). Output example in docs: `db.getSiblingDB("admin").getCollection("companies").aggregate([{ $project: {"A": {$literal: …}, …, "_id": 0} }])` → projections hide `_id` unless selected; constructors get `new` added except `Date`.

#### 3.2.5 Grid and editing

- `MongoGridColumnsManager(Factory)`, `MongoGridHelper`, `MongoHookUpHelper`, `MongoDmlHelper`, `MongoPredicatesHelper`, `MongoDomains`, `MongoTypeSystem` `[Observed]`.
- Editing `[Observed: help:editing-data-in-mongodb]`: double-click edit, *Preview Pending Changes* shows generated DML; Add Row = new document with fields `<unset>` (omitted) by default; Add/Delete Column; Delete Row; *Open in Value Editor* for raw JSON with soft-wrap and **Change Type**.
- `[Inferred]` DML generation: `updateOne({_id: <id>}, {$set: {path: value}})`, `$unset` for deleted fields/columns, `insertOne(doc)`, `deleteOne({_id: <id>})`; documents without `_id` cannot be safely edited.

### 3.3 Apache Cassandra

- Driver `com.dbschema.CassandraJdbcDriver` ("DataGrip-patch" builds), URL `jdbc:cassandra://h1[:9042],h2…/[keyspace][?props]`, `loadBalancing=DcInferringLoadBalancingPolicy`; SSL via `sslenabled=true` + JVM keystore/truststore VM options `[Observed]`.
- Introspection pack sections: RetrieveSchemas(2|3), RetrieveVirtualSchemas, RetrieveTables(2|3), RetrieveVirtualTables4, RetrieveVirtualColumns4, RetrieveMatViews, RetrieveColumns(2|3), RetrieveIndices(2|3), RetrieveTriggers, RetrieveFunctions, RetrieveAggregates, RetrieveObjectTypes, RetrieveRoles(21|22) — version-suffixed for Cassandra 2.x/3.x/4.x; catalogs `system_schema.{keyspaces,tables,columns,indexes,views,types,functions,aggregates,triggers}`, `system_virtual_schema.{keyspaces,tables,columns}`, legacy `system.schema_{keyspaces,columnfamilies,columns,usertypes,functions,aggregates,triggers}`, `system_auth.{roles,users}` `[Observed]`.
- Model: Root → {Role, Schema(keyspace)}; Schema → {Table → {Column, Key (partition/clustering), Index, Trigger}, MatView → MatViewColumn, DefType(UDT) → Attribute, Function/Routine → Argument, Aggregate} `[Observed: Cass* model classes]`; minicat for system keyspaces; own importer (`CassImportManager`) and object editor `[Observed]`.
- Console language CassandraQL (`.cql`) `[Observed]`.

### 3.4 Amazon DynamoDB

- Driver `org.jetbrains.DynamoDriver`, URL `jdbc:dynamodb://[host[:port]][?region=eu-west-1&…]` (host for DynamoDB Local / custom endpoints), auth AWS Profile (named profile + region) or default chain via plain URL `[Observed: drivers XML, help:dynamodb]`.
- Dialect `Dynamo` (PartiQL); functions `LIST_APPEND`, `SET_ADD`, `SET_REMOVE`, `ATTRIBUTE_TYPE`, `BEGINS_WITH`, `CONTAINS`, `SIZE` `[Observed: functions.xml]`.
- Model: `DynamoRoot` → `DynamoSchema` → `DynamoTable` → {`DynamoTableColumn`, `DynamoKey` (hash/range), `DynamoIndex` (GSI/LSI)}, `DynamoProvisioned` (capacity) `[Observed]`; driver process uses `RemoteDynamoConnection` `[Observed]`.
- Grid: `DynamoGridColumnsManagerFactory`, `DynamoHookUpHelper`, `DynamoDmlHelper`, script generator `[Observed]` — `[Unknown]` editing support level in 2024.3 (docs silent).

### 3.5 Couchbase Query

- "DataGrip supports the Couchbase Query service, not the Couchbase Analytics service"; "data editing is currently not supported" `[Observed: help]`.
- URL `jdbc:couchbase:h1[:port],h2…[/default_bucket][?user=…]`; docs say default port 11210 `[Observed]`.
- Dialect `CouchbaseQuery` (N1QL/SQL++: `USE KEYS`, `LET`, …), model `CouchbaseRoot` → `CouchbaseSchema` → `CouchbaseTable` → `CouchbaseTableColumn` + `CouchbaseIndex` (bucket/scope/collection mapping `[Inferred]`), nested fields via `DocumentDvStructureExtension` `[Observed]`.

### 3.6 Elasticsearch

- Basic support only: generic dialect + JDBC metadata; driver `org.elasticsearch.xpack.sql.jdbc.EsDriver` (`jdbc:es://` or `jdbc:elasticsearch://`, optional `http(s)://`, path prefix, port 9200) `[Observed: drivers XML; help:other-databases]`. Requires the (licensed) SQL plugin on the cluster `[Inferred: Elastic docs]`.

### 3.7 Comparison table

| Store | Driver (licence) | Console language | Introspection source | Namespaces | Editing (2024.3) |
|---|---|---|---|---|---|
| Redis | JetBrains redis-jdbc-driver (Apache-2.0) | Redis commands + hints | INFO, CONFIG, SCAN+TYPE, MODULE LIST, CLUSTER SLOTS | DB index | **No** (read + key drop/alter) |
| MongoDB/DocumentDB | JetBrains mongo-jdbc-driver (Apache-2.0) + mongosh/GraalJS | MongoJS, SQL for MongoDB | first N docs + listIndexes | database | **Yes** |
| Cassandra | DbSchema-based (patched) | CQL | `system_schema.*` pack | keyspace | `[Unknown]` |
| DynamoDB | JetBrains | PartiQL | DescribeTable `[Inferred]` | (region) | `[Unknown]` |
| Couchbase | JetBrains | N1QL/SQL++ | `system:` catalogs `[Inferred]` | bucket/scope | **No** |
| Elasticsearch | Elastic X-Pack SQL JDBC | SQL | JDBC metadata | — | no (basic) |

---

## 4. Algorithm specifications

### ALG-REDIS-01: Bounded SCAN-based key introspection

- **Problem:** Enumerate keys and their types without blocking the server (`KEYS *` is O(N) and blocking) and without unbounded memory.
- **Inputs / outputs:** `conn`, `db`, `filter_glob`, `count_hint`, `max_keys`, `deadline` → `Vec<KeyInfo{name: Bytes, type: KeyType}>` + `truncated: bool`.
- **Algorithm:**
  1. Standalone: `SELECT db`. Cluster: obtain master nodes from `CLUSTER SHARDS`/`CLUSTER SLOTS` (map announced addresses through the tunnel mapping), run steps 2–5 per master (only db 0).
  2. `cursor = 0`; loop: `SCAN cursor MATCH filter COUNT count_hint` (Redis ≥ 6: optionally `TYPE t` per pass when the UI asks for one type).
  3. Pipeline `TYPE k` for the returned batch (one round-trip per batch); optionally `PTTL`, `MEMORY USAGE … SAMPLES 0` lazily on demand (not during introspection).
  4. Append; dedupe (SCAN may return duplicates across iterations) with a hash set; stop when `cursor == 0`, `len ≥ max_keys` (set `truncated`), or deadline/cancel.
  5. Map types: `string, list, set, zset→sorted_set, hash, stream, ReJSON-RL→json_document`, module types (`MBbloom--`, `TSDB-TYPE`, …) → `data_structure`, else `unknown`.
- **Data structures:** `HashSet<Bytes>`; result arena; keys as raw bytes (binary-safe) plus lossy UTF-8 display name.
- **Complexity:** O(N) total server work spread across calls; client memory O(min(N, max_keys)).
- **Edge cases (exhaustive):** keys expiring during scan (TYPE returns `none` → drop); keys renamed during scan (may appear twice/never — SCAN guarantee only for keys present for the whole iteration); `COUNT` is a hint (server may return more/less, even 0 with non-zero cursor — keep looping); `MATCH` filters after fetching (sparse matches → many empty iterations; enforce deadline); binary keys and invalid UTF-8; glob special chars `* ? [ ] \` in user filter (escape when the user intends a literal); cluster `MATCH` with hash tag (`{user}:*`) can be routed to one node; ACL restricting `SCAN`/`TYPE`/key patterns (`%R~pattern`) → partial results with warning; `CONFIG GET databases` forbidden on managed services → fall back to `INFO keyspace` DBs + 16; Redis < 2.8 (no SCAN) → refuse or `KEYS` with explicit user consent; replicas (READONLY in cluster for replica scans); very large DBs (10⁷ keys) → never materialize all: introspection should store **prefix aggregates** and load leaves lazily (improvement over DataGrip's full key list).
- **Test cases:** 10,000 keys, COUNT 1000 → ~10 iterations, 10,000 unique; filter `user:*` → only matching; key expiring mid-scan → absent; cluster 3 masters → union of 3 scans; ACL `~cache:*` user with filter `*` → only `cache:*` keys + warning.
- **Prior art and evidence:** `[Observed: KeysScanInfo, options Default key filter / COUNT; help:redis]`; Redis SCAN docs.
- **Used by:** Hacivat; Karagöz (server-side scan to avoid P2P round-trips).

### ALG-REDIS-02: Delimiter key tree

- **Problem:** Show a flat keyspace as a hierarchy using a delimiter (e.g. `:`), with counts, lazily.
- **Inputs / outputs:** sorted key names, delimiter `d` (1+ chars) → tree nodes `{segment, full_prefix, child_count, leaf: Option<KeyInfo>}`.
- **Algorithm:** for each key split on `d` (non-overlapping, left to right); walk/insert nodes per segment; a key that is also a prefix of others yields a node that is both folder and leaf (render as folder with a leaf child named by the full key); counts aggregated bottom-up; children sorted naturally (numeric-aware), folders first; lazy mode: build only first level from a prefix histogram and expand on demand with `SCAN MATCH <prefix><d>*`.
- **Data structures:** trie keyed by segment (`BTreeMap<String, Node>` or interned segments).
- **Complexity:** O(total key length).
- **Edge cases:** empty segments (`a::b`, leading/trailing delimiter) → keep empty segment shown as `∅`; delimiter absent → all keys at root; multi-char delimiter overlapping (`::` in `a:::b`) → non-overlapping greedy split; binary keys (display escaped); delimiter contained in glob specials when used in MATCH (escape `[`, `*`, `?`); huge fan-out folders (10⁵ children) → paginate; changing delimiter re-renders without re-scan (model stays flat — DataGrip behaviour).
- **Test cases:** keys `u:1`, `u:2`, `u` with `:` → folder `u` (count 2) + leaf `u`; `a::b` → `a` / `` / `b`; delimiter `/` on `x/y/z` → depth 3.
- **Prior art:** `[Observed: RedisHierarchyOption "Key parts delimiter", RedisDvStructureExtension]`.
- **Used by:** Hacivat.

### ALG-REDIS-03: Redis console statement parsing (hints) and routing

- **Problem:** Execute user-entered Redis commands safely and render results as grids, including cluster routing.
- **Inputs / outputs:** text block → `RedisQuery{command, args, column_hint?, node_hint?, blocking, kind}` → result grid.
- **Algorithm:** tokenize with Redis-CLI-like quoting (double quotes with escapes `\n \t \xHH`, single quotes literal); classify lines (comment `--`, node hint `-- node = host:port`, column hint `-- name : v…`, command); require exactly one command per statement (split the editor buffer into statements by blank lines/newlines first); look up the command in the command table (arity, key positions, flags from `COMMAND DOCS/INFO`) to: validate arity, find the routing key (cluster), detect blocking/write/admin flags (read-only guard, ALG-RO-01); special-case `SELECT n` (session db change — reject in cluster unless 0), `MULTI/EXEC` (keep on one connection), `SUBSCRIBE`/`MONITOR` (switch to streaming view with stop button); route by key slot (CRC16 of hash-tag or key mod 16384) or node hint; render via per-command result shape (scalar, list, map, tuple (member,score), stream entries (id + field map), nested arrays → tree view); column hint adds a synthetic first column.
- **Edge cases:** commands with no keys in cluster (`INFO`, `CONFIG`) → choose node (all/one/hint); `CROSSSLOT` errors on multi-key commands → suggest hash tags; RESP3 maps vs RESP2 flat arrays; very large replies (cap rows; stream with `COUNT`); binary values (hex/escaped display); `EVAL` keys count argument; module commands unknown to the table (pass through, assume write for safety).
- **Test cases:** `-- node = 10.0.0.2:7001\nINFO` → executed on that node; `-- key : a b c\nMGET a b c` → grid columns `key`, `value`; `SELECT 3` in cluster → error; `BLPOP q 0` → blocking with cancel = close connection.
- **Prior art:** `[Observed: oss-redis QueryParser/Analyzer/RedisJedisClusterClient@v1.5]` (Apache-2.0).
- **Used by:** Hacivat; Karagöz (classification + routing next to the cluster).

### ALG-REDIS-04: Per-type value grid projection (read + Hacivat editing)

- **Problem:** Display a key's value as a table and (Hacivat improvement) translate edits into minimal commands.
- **Mapping:**

| Type | Read (paged) | Columns | Edit → command |
|---|---|---|---|
| string | `GET` (or `GETRANGE` for >1 MB) | value | `SET k v KEEPTTL` |
| list | `LRANGE k i j` | index, value | `LSET k i v`; insert `RPUSH`/`LINSERT`; delete by index: `LSET k i <tombstone>` + `LREM k 1 <tombstone>` |
| set | `SSCAN k cursor COUNT n` | member | `SREM`+`SADD` (rename member); add `SADD`; delete `SREM` |
| sorted_set | `ZRANGE k i j WITHSCORES` / `ZSCAN` | member, score | `ZADD k XX score member` (score), member rename = `ZREM`+`ZADD`; delete `ZREM` |
| hash | `HSCAN k cursor COUNT n` | field, value | `HSET k f v`; rename field = `HDEL`+`HSET` (in `MULTI`); delete `HDEL` |
| stream | `XRANGE k start end COUNT n` / `XREVRANGE` | id, field… (union) | append `XADD`; delete `XDEL`; (entries immutable) |
| json_document | `JSON.GET k $` | tree/JSON | `JSON.SET k <path> <json>` |
| data_structure/unknown | module-specific info (`BF.INFO`, `TS.INFO`) | info | none |

- **Edge cases:** concurrent modification between read and write (use `WATCH` + `MULTI/EXEC` for compound edits); TTL preservation (`KEEPTTL` ≥ 6.0; else re-`PEXPIRE` with remaining PTTL); huge collections (paging by cursor for set/hash/zset; random access only for lists/zsets); member/field binary-safety; key deleted meanwhile ("The key does not exist" — DataGrip message).
- **Test cases:** edit hash field value → `HSET`; rename hash field atomically; edit string preserving TTL.
- **Prior art:** `[Observed: per-type delegates/select generators; help: editing not supported]`.
- **Used by:** Hacivat (feature beyond DataGrip parity).

### ALG-MONGO-01: Schema sampling and type lattice

- **Problem:** Infer a collection's field tree, types, and presence statistics from a bounded sample.
- **Inputs / outputs:** collection, `n` (default 100 in Hacivat; DataGrip 10), strategy (`first` | `$sample` | `recent` (sort `_id` desc)) → `FieldTree{path → {types: Map<BsonType, count>, present: count, nested?}}`.
- **Algorithm:**
  1. Fetch sample (`$sample: {size:n}` if collection is large and server ≥ 3.2 and not a view/time-series restriction; else `find().limit(n)`), with `maxTimeMS` and projection none.
  2. For each document, walk recursively: object → descend; array → record `array` type with element-type sub-stats (`path.[]`), descend into object elements; scalar → increment `types[bson_type]`; mark `present[path] += 1`.
  3. Fields absent in a document are implicitly counted as missing (`present < n_docs`).
  4. Type lattice for a column's display/editing type: if one type ≥ 99% → that type; `int32 ⊂ int64 ⊂ double ⊂ decimal128` → widen numeric; `null` + T → T nullable; mixed incompatible → `mixed` (show per-cell type badges); `objectId` stays exact; dates (`date`, `timestamp`) don't merge with strings.
  5. Index fields: from `listIndexes`; if an index path never appears in the sample → targeted `find({path:{$exists:true}},{path:1}).limit(1)` (as driver HEAD does).
  6. Mark `_id` as primary key; unique indexes as unique keys.
- **Data structures:** `BTreeMap<FieldPath, FieldStats>`; `FieldPath` as segments (escape `.` and `$` in names — Mongo allows dots in keys since 5.0).
- **Complexity:** O(total sampled document size).
- **Edge cases (exhaustive):** empty collection; unauthorized (code 13) → empty schema, no error; views (no `$sample` restriction issue but no indexes); time-series collections (`listIndexes` differs); capped collections; documents > 16 MB not possible; deeply nested (cap depth, e.g. 20); arrays of mixed scalars/objects; heterogeneous `_id` types; field names with `.`/`$`/empty string/unicode; huge arrays (cap elements per array sampled); `uuidRepresentation` affecting UUID decoding (legacy subtype 3); DocumentDB lacking `$sample` features; sharded clusters (`$sample` fine); sampling bias of `first` (insertion order) — DataGrip's default.
- **Test cases:** docs `{a:1}`, `{a:2.5}`, `{b:"x"}` → `a: int32(1)+double(1) → double, present 2/3`, `b: string present 1/3`; `{arr:[{x:1},{y:2}]}` → `arr.[].x`, `arr.[].y`; unauthorized → `{}`.
- **Prior art and evidence:** `[Observed: oss-mongo MetaCollection@v1.20 & HEAD; help fetch_documents_for_metainfo]`; MongoDB Compass schema analyzer (prior art in `clients/`).
- **Used by:** Hacivat; Karagöz (sample server-side).

### ALG-MONGO-02: Context-driven completion via a signature table

- **Problem:** Provide completion/validation/read-only classification for `db.<coll>.<method>(…)` chains.
- **Inputs / outputs:** JS AST (tree-sitter-javascript), caret, model (databases/collections/fields), signature table → completion items / diagnostics / purity.
- **Algorithm:**
  1. Resolve receiver chain: `db` → Database (current db, `use x` changes it for following statements); `db.x` / `db.getCollection("x")` / `db.getSiblingDB("d").x` → Collection; method calls return types per signature (`find → CURSOR`, `CURSOR.sort → CURSOR`).
  2. For a caret inside argument *i* of method *m*, get `ARGCTX = sig[m].args[i]`; inside an object literal at depth d, the context selects the operator list (`QUERY` → field names + query operators; `UPDATE_STAGE` → `$set/$inc/…`; inside `$set: { … }` → field names; `AGGREGATION_STAGE` (array of stages) → stage names; inside `$match` → `QUERY` again; operator entries may declare their own nested context (`$expr(AGGREGATION_EXPRESSION)`)).
  3. Field names from the collection's inferred schema (ALG-MONGO-01), dotted paths for nested.
  4. Purity: chain is pure iff every call is `[pure]` (and no `$out/$merge` stage inside `aggregate`); read-only sessions block non-pure chains (SideEffects inspection).
  5. Deprecation table → warnings + quick fixes.
- **Data structures:** signature table (Hacivat-authored, from MongoDB docs), operator tables per context, scope stack.
- **Edge cases:** variables holding collections (`const c = db.x; c.find()`), loops, functions (best-effort data-flow), `$` in field names, string-keyed access `db["my-coll"]`, collections with dots (`db.getCollection("a.b")`), shell helpers (`show dbs`, `it`), unknown methods (no error unless strict).
- **Test cases:** `db.users.find({age: {$|}})` → query-expression operators; `db.users.updateOne({}, {$set: {|}})` → fields of users; `db.users.aggregate([{$out:"x"}])` in read-only → blocked.
- **Prior art:** `[Observed: signature DSL files, MongoJS inspections]`.
- **Used by:** Hacivat.

### ALG-MONGO-03: SQL → aggregation pipeline translation (subset)

- **Problem:** Let SQL users query collections; translate a restricted `SELECT` to a pipeline.
- **Inputs / outputs:** SQL AST (subset) → `db.getSiblingDB(db).getCollection(c).aggregate([...])`.
- **Algorithm (stage order):**
  1. `FROM c [AS a]` → base collection; nested `(SELECT …) AS a` → translate inner pipeline first then continue on its output.
  2. Each `[LEFT] JOIN d ON a.x = d.y` (single equality) → `$lookup {from:d, localField:x, foreignField:y, as:"d"}` + `$unwind {path:"$d", preserveNullAndEmptyArrays: LEFT}`.
  3. `WHERE` → `$match` (use `$expr` when comparing two fields or with functions; `LIKE 'a%'` → regex `^a` (escape regex metachars; `_` → `.`; `%` → `.*`), `IN`, `BETWEEN` → `$gte/$lte`, `IS NULL` → `{$in:[null]}`/`$exists`).
  4. `GROUP BY` → `$group {_id: {k1:"$k1",…}, agg: {$sum/$avg/$min/$max:…}, cnt: {$sum:1}}` (`COUNT(*)` only); `HAVING` → `$match` after `$group`.
  5. `SELECT` list → `$project` (computed expressions via aggregation operators; `_id: 0` unless selected; aliases without dots); `DISTINCT` → `$group` on projected fields then `$replaceRoot`.
  6. `ORDER BY` → `$sort`; `OFFSET` → `$skip`; `LIMIT` → `$limit`.
- **Limits (match DataGrip, documented as errors):** single equality ON; no USING; no sub-select as JOIN target; `COUNT(expr)` unsupported; aggregates not nestable; no non-aggregate use of AVG/SUM/MIN/MAX; `LIKE` only with literal; alias without dots; cross-database joins not supported; operators `map/reduce/filter/let` excluded.
- **Edge cases:** NULL semantics (SQL three-valued vs Mongo missing/null); type coercion in comparisons; case sensitivity of LIKE (regex `i` flag only if `ILIKE`); `ORDER BY` alias resolution; `GROUP BY` on nested paths; dotted field references `a.b.c` vs table alias `a`.
- **Test cases:** `SELECT name FROM users WHERE age > 30 ORDER BY name LIMIT 10` → `[$match{age:{$gt:30}}, $sort{name:1}, $limit 10, $project{name:1,_id:0}]` (order normalized: project last or `$project` before sort with alias mapping); `SELECT u.name, o.total FROM users u JOIN orders o ON u._id = o.user_id` → `$lookup`+`$unwind`+`$project`; `SELECT COUNT(x) FROM t` → error.
- **Prior art:** `[Observed: help:sql-for-mongodb; translator class inventory]`.
- **Used by:** Hacivat (optional, after MongoJS).

---

## 5. Implications for Hacivat / Karagöz / Perde

1. **Do not force NoSQL into JDBC-like tables at the driver layer.** Use native Rust clients (`redis`/`fred`, `mongodb`, `scylla`, `aws-sdk-dynamodb`, `couchbase` via HTTP query service) behind capability traits (`KeyValueSession`, `DocumentSession`, `WideColumnSession`) and provide *projections* to the grid (ALG-REDIS-04, ALG-MONGO-01).
2. Redis: implement ALG-REDIS-01..04 with lazy prefix aggregation instead of materializing millions of keys (DataGrip's cache held 5k keys for one DS — fine for dev, not for prod). Add editing (differentiator), TTL column, memory usage on demand, Pub/Sub & MONITOR streaming views, and reuse the Redis driver's **hint comment syntax** (Apache-2.0 idea, re-implemented) for routing and labeling.
3. Mongo: don't embed GraalJS/mongosh. Parse a JS subset with tree-sitter, execute recognized chains through the Rust driver, and optionally delegate unsupported scripts to a user-installed `mongosh` subprocess. Sampling default 100 docs with `$sample`; show presence % and type mix (like Compass).
4. Read-only enforcement for NoSQL (Redis command flags, Mongo `[pure]` table) must also exist in **Karagöz** (P2P path), sharing the same tables.
5. Perde/Karagöz discovery: Redis/Mongo containers are common Karagöz targets (Docker image heuristics); publish descriptors with mode (standalone/cluster/replica set) so Hacivat chooses templates correctly; cluster node address mapping is done by Karagöz locally (no SSH tunnel per node needed).

## 6. i18n considerations

- Key names, field names and values are user data: never translate; render binary/invalid UTF-8 safely (escaped `\xHH`) independent of locale.
- Type names (`hash`, `sorted set`, `ObjectId`) are technical terms — keep English identifiers, translate descriptions/tooltips only.
- Delimiter tree sorting: natural sort with locale-independent comparison for identity; display collation may use the UI locale (Turkish `İ/ı` ordering) but must not merge distinct keys.
- MongoJS/SQL error messages from translators need message keys with placeholders (operator names stay untranslated).

## 7. Risks, pitfalls, open questions

- `[Unknown]` Whether DataGrip's Redis "Default key filter" is a glob passed to `MATCH` or a regex applied client-side (docs say regex).
- `[Unknown]` Default key delimiter value and whether the introspector stores per-key TTL anywhere (cache shows none).
- `[Unknown]` DynamoDB/Cassandra editing capabilities in 2024.3.
- `[Inferred]` Redis per-type read commands; mapping of introspection holders to commands.
- Risk: sampling the first N Mongo documents biases towards oldest schema versions; `$sample` costs more on huge collections (random cursor) but is better; make it configurable.
- Risk: Redis cluster introspection through SSH requires every node in the URL (DataGrip limitation) — Karagöz avoids this.
