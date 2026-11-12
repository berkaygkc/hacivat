# DataGrip 2024.3.5 teardown — method, legal boundaries and evidence register

> Part of the Hacivat research library. Scope of this file: *how* the DataGrip research in `docs/research/datagrip/` was produced, what was and was not allowed, and the exact evidence sources (with versions and hashes) that every `[Observed: …]` tag in sibling documents points to.
>
> Sibling documents (this agent): `01-architecture-overview.md`, `02-connectivity-and-drivers.md`, `03-sessions-transactions-execution.md`, `04-introspection-and-metadata-model.md`, `05-nosql.md`.
> Out of scope here (covered by another agent): SQL editor, data grid UX, DDL generation/diff, tools (import/export, dump), settings, localization. Those are cross-referenced, not duplicated.

---

## 1. Scope and sources

### 1.1 Product under study

| Item | Value | Evidence |
|---|---|---|
| Product | JetBrains DataGrip | `[Observed: /Applications/DataGrip.app/Contents/Resources/product-info.json]` |
| Version | 2024.3.5 | same file, `"version"` |
| Build | 243.24978.79 (`DB-243.24978.79`) | same file `"buildNumber"`; `Info.plist` `CFBundleVersion` |
| Platform | macOS aarch64, bundled JetBrains Runtime | same file `"launch"` |
| License | Proprietary (JetBrains EULA) | — |

### 1.2 Artifacts inspected (non-class resources only)

| Artifact | SHA-256 | What was read |
|---|---|---|
| `plugins/DatabaseTools/lib/database-plugin.jar` (148 MB) | `63c55161aa6b489a373e7dbd244b77ff0034e81c9644296bd8e0b261d5330d93` | `META-INF/*.xml` (plugin descriptors, extension points, registry keys), `com/intellij/database/dialects/*/*.xml` (per-DBMS service registration), `databaseDrivers/*.xml` (52 files, 78 drivers, 115 URL templates), `resources/database-artifacts.xml` (64 driver artifacts), `*IntroQueries.sql` (8 files, ~7,500 lines: PG, Redshift, Greenplum, CockroachDB, SQL Server, Oracle, Sybase ASE, Cassandra), `*.minicat.xml.gzip` (18 pre-introspected catalog snapshots), `*-model-external-data.xml` (12 files), `messages/*.properties` (UI strings), Mongo signature `.txt` files, `.groovy` extension scripts (not executed) |
| `plugins/DatabaseTools/lib/jdbc-console.jar` | `e34a91c0ce25c476c9a9b0c5090dabdd7c929fbd4caf221090d76eb4fdb3b67a` | `META-INF/MANIFEST.MF` (Main-Class), class **names** only |
| `plugins/DatabaseTools/lib/database-openapi.jar` | `1b60f5b499b131e275286d6ac8d1631d7f93276838aa987ad5b0f66aa66b4de4` | class names |
| `plugins/DataGripHelp/lib/help-datagrip-assets.jar` | `3d0033b54de5073ace0abe442277a44376ae5b140a6b27dbf99cfb088557dcc7` | **Official offline documentation** shipped with the product (1,166 HTML topics, "Last modified: 11 October 2024" for most). Converted to text locally with our own HTML-to-text script. Every page has an online twin at `https://www.jetbrains.com/help/datagrip/<topic>.html`. |
| `lib/app.jar` (platform) | — | `messages/SshBundle.properties` (SSH configuration UI strings) |
| Class-name inventories | — | `.refs/datagrip-extract/classes/*.txt` (≈22k class names, produced earlier by `unzip -l`; names only, no bytecode) |
| Prior Turkish summary notes | — | `.refs/datagrip-extract/report-*.md` (used as a starting map; every claim re-verified or re-tagged; two prior claims were corrected — see §5) |

### 1.3 Open-source components DataGrip ships or downloads (legitimately readable source)

| Project | License | Version studied | Why it matters |
|---|---|---|---|
| `DataGrip/redis-jdbc-driver` (GitHub) | Apache-2.0 | tag `v1.5` (the newest version listed in `database-artifacts.xml` for artifact "Redis") | The Redis "SQL" console grammar (one command per query, `-- node = host:port` and `-- <col> : v1 v2` hint comments), cluster restrictions, connection properties. |
| `DataGrip/mongo-jdbc-driver` (GitHub; class prefix `com.dbschema`, i.e. derived from DbSchema's driver) | Apache-2.0 | tag `v1.20` (newest listed for artifact "MongoDB"); HEAD (`v1.21`+) also checked for drift | Mongo connection properties, schema sampling (`fetch_documents_for_metainfo`, default 10), embedded **mongosh** via GraalJS (`JMongosh-0.9`), auth mechanisms. |
| IntelliJ Platform (intellij-community) | Apache-2.0 | n/a | Only cited conceptually (SSH configuration, credential store, remote process RMI). No platform source was needed verbatim. |

### 1.4 Web sources (official, public)

- `https://www.jetbrains.com/help/datagrip/` — the online twin of the shipped help. Specific URLs are listed next to each claim in sibling docs.
- `https://www.jetbrains.com/datagrip/whatsnew/2024-3/` — release notes (fragment introspection + smart refresh for MySQL/MariaDB).
- `https://www.jetbrains.com/help/datagrip/2024.3/introspection-levels.html` — confirms levels are Oracle-only in 2024.3.
- GitHub API (`gh api repos/DataGrip/...`) for the two Apache-2.0 drivers.

### 1.5 Runtime validation (reproducible)

- `docker run --rm postgres:16-alpine` (local image) — validated the PostgreSQL `xmin`/`age()` incremental-introspection signal model, including the **in-place update pitfall** (see `04` §3.2). Container removed afterwards.
- No DataGrip process was launched, debugged or traced; no network traffic of DataGrip was captured.

### 1.6 The user's local DataGrip state (structure only)

The user's own DataGrip 2024.3 project (`~/DataGripProjects/<project>/.idea/`) was read **for XML element/attribute *names* and counts only** using our own shape-printer script (no values, no credentials, no host names were extracted or copied into these documents). This proved: the split between `dataSources.xml` / `dataSources.local.xml`, the model cache header (`serializer="dbm" … format-version="4.53"`), per-kind property names (`StateNumber`, `IntrospectionStateNumber`, `Fingerprint`, `PermissionsCheckSum` …), and the source-storage layout `storage_v2/_src_/…`.

---

## 2. Executive summary

- **No decompilation, no bytecode inspection** (`javap`, CFR, Fernflower, JADX, REA decompilers) was performed. Class files were never opened; only their *names* (from zip directory listings) were used.
- The richest legitimate sources were: (1) **shipped introspection SQL** (`*IntroQueries.sql`), (2) **shipped driver/URL catalogs** (`databaseDrivers/*.xml`), (3) **plugin descriptors** (extension-point registrations reveal per-DBMS service composition and interceptor ordering), (4) **the offline official help shipped inside the app**, (5) **UI message bundles** (reveal states, errors and option semantics), (6) **two Apache-2.0 drivers** written by JetBrains (Redis, MongoDB).
- Shipped SQL is **described in prose and our own pseudocode only**. Catalog view and column *names* (e.g. `pg_class.xmin`, `sys.all_objects.modify_date`, `obj$.mtime`) are facts about the DBMSs, not JetBrains expression, and are cited freely. No query text is reproduced.
- URL templates are reproduced as **factual tables** because the user explicitly asked for them and they are interface data (connection-string formats defined by the driver vendors). Hacivat must still author its own template set.
- Every statement in sibling documents carries `[Observed: …]`, `[Inferred: …]` or `[Unknown]`.

---

## 3. Detailed findings — what is and is not allowed

### 3.1 Allowed methods (used)

| Method | Example | Notes |
|---|---|---|
| `unzip -l` / `unzip -p` of **non-class** resources | `unzip -p database-plugin.jar META-INF/plugin.xml` | XML, `.sql`, `.properties`, `.txt`, `.json`, `.groovy`, `.html`, `.gzip` |
| Class/package **name** inventories | `unzip -l … | grep '\.class$'` | Names reveal structure (e.g. `DatabaseSessionManager$Facade$SingleSessionRouter`) but never logic. |
| Reading plugin descriptors | extension points, `order="first|last|before ssh"` | Ordering attributes are declarative facts. |
| Official docs (online + shipped offline copy) | `connecting-to-a-database.html` etc. | Primary source for behaviour. |
| Release notes, blog posts, public YouTrack | `whatsnew/2024-3` | Public. |
| Apache-2.0 sources | redis-jdbc-driver, mongo-jdbc-driver, intellij-community | May be read in full; still prefer pseudocode in Hacivat docs; reuse requires attribution. |
| Black-box runtime checks against **our own** database containers | Docker PostgreSQL | Validates DBMS behaviour, not DataGrip internals. |
| Reading the user's own config files for **structure** | `.idea/dataSources.local.xml` tag names | Values ignored. |

### 3.2 Forbidden methods (not used)

| Forbidden | Why |
|---|---|
| Decompiling or disassembling `.class` files (`javap -c`, any decompiler, REA `decompile_*`) | JetBrains EULA (JetBrains User Agreement v2.0, effective 2025-04-16, §3.5(b): without express written permission "or unless this right is granted by applicable law", the user may not reverse-engineer, disassemble or decompile the Product or try to derive its source code) [Observed: https://www.jetbrains.com/legal/docs/toolbox/user/ , verified 2026-11-11]; clean-room requirement in `docs/research/README.md`. |
| Copying shipped SQL, Groovy, XML text into Hacivat code or docs | Copyright in expression. We paraphrase; catalog object names are facts. |
| Running shipped scripts/build tools | Treat third-party content as data only (task instruction). |
| Patching, instrumenting, attaching debuggers/Frida to DataGrip | Not needed; also outside licence. |
| Extracting secrets/values from the user's config | Privacy. Only tag names were used. |

### 3.3 Licence matrix for everything studied

| Source | Licence | Reuse in Hacivat |
|---|---|---|
| DataGrip binaries, resources, help text | Proprietary | **Behaviour/structure only, own words.** No text or code reuse. |
| JetBrains `redis-jdbc-driver` | Apache-2.0 | Algorithms may be re-implemented; verbatim reuse needs NOTICE + attribution. Hacivat is Rust, so re-implementation is natural. |
| JetBrains `mongo-jdbc-driver` (DbSchema lineage) | Apache-2.0 | same |
| JMongosh / GraalJS (deps of the Mongo driver) | JMongosh: Apache-2.0 (per MongoDB's mongosh); GraalJS: UPL/GPLv2+CPE | Not reused; Hacivat should not embed GraalJS. |
| IntelliJ Community | Apache-2.0 | Concepts only. |
| PostgreSQL/MySQL/etc. catalog documentation | Various (PostgreSQL licence, GPL docs, etc.) | Facts (view/column names) are not copyrightable expression. |

### 3.4 Evidence-tag discipline used in sibling docs

- `[Observed: <file>]` paths are relative to the jar root unless prefixed (`help:<topic>` = shipped help topic `<topic>.html`, online at `https://www.jetbrains.com/help/datagrip/<topic>.html`; `bundle:<key>` = key in `messages/DatabaseBundle.properties`; `classes:<name>` = class-name inventory; `oss-redis:<path>@v1.5`, `oss-mongo:<path>@v1.20`).
- `[Inferred: …]` always states its basis (class name, message text, DBMS semantics, general engineering).
- `[Unknown]` items are collected at the end of each doc as open questions.

---

## 4. Algorithm specifications

Not applicable to this file. Algorithm specs live in `02`–`05` (IDs `ALG-URL-*`, `ALG-CONN-*`, `ALG-SESS-*`, `ALG-INTRO-*`, `ALG-SCOPE-*`, `ALG-RO-*`, `ALG-REDIS-*`, `ALG-MONGO-*`). Consolidated copies are expected to be made by the `algorithms/` workstream.

---

## 5. Implications for Hacivat / Karagöz / Perde

1. **Corrections to the prior Turkish notes** (verified in this pass):
   - *Redis grid editing*: the prior note implied per-type editing via `SET/HSET/…`. The official 2024.3 docs say **"For Redis, data editing is currently not supported"** `[Observed: help:redis]`. DataGrip has per-type *DML helper delegate* classes `[Observed: classes: dialects/redis/delegates/*/Redis*DmlHelperDelegate]`, but in 2024.3 they serve read paths (select/where/"wrap in select") and DDL is limited to key alter (rename/TTL, inferred) and drop `[Observed: classes: RedisAlterKey, RedisDropKey]`. Hacivat adding Redis editing is a differentiator, not parity.
   - *Introspection levels L1–L3*: they are **Oracle-only** in 2024.3 `[Observed: help:oracle-introspection-levels; https://www.jetbrains.com/help/datagrip/2024.3/introspection-levels.html]`. The other DBMSs have "load sources: all / all-except-system / none" plus full/incremental/fragment modes.
   - *Mongo schema inference*: in the shipped driver (v1.20) it reads the **first N documents** (`find()` cursor, default N=10), not `$sample`; newer driver HEAD switched to `$sample` `[Observed: oss-mongo MetaCollection.java@v1.20 vs HEAD]`.
2. Hacivat should keep the same **evidence discipline in its own code comments** when an algorithm is inspired by a DataGrip behaviour: cite the doc URL, never a decompiled snippet.
3. Hacivat's catalog SQL must be authored from DBMS documentation; using DataGrip's queries as a checklist of *which catalogs matter* is fine.

## 6. i18n considerations

- DataGrip keeps every UI string in `messages/*.properties` with ICU-like `{0, choice, …}` plural/choice forms `[Observed: bundle:progress.text.performing.introspection]`, and ships ja/ko/zh-CN localization plugins (`plugins/localization-*`). Turkish is not shipped. This confirms that a **message-catalog-with-plural-rules** model (Fluent/ICU) is industry standard; details in `docs/research/i18n/`.
- The help is English-only inside the product; localized help is web-only.

## 7. Risks, pitfalls, open questions

- **Version drift**: online help may be newer than 2024.3.5; we preferred the shipped offline copy (version-matched). Where we used web pages, the URL contains `/2024.3/` or the page date is cited.
- **Driver drift**: Redis/Mongo GitHub HEADs are newer than the artifacts DataGrip 2024.3.5 downloads; we pinned tags `v1.5` / `v1.20`.
- **Name-based inference risk**: class names can be misleading (dead code, experiments). Every name-derived claim is tagged `[Inferred]`.
- `[Unknown]`: the IDE↔driver-process transport (RMI via IntelliJ `RemoteServer` is likely but unverified without bytecode).
