# 06 — SQL Language Support and Editor (DataGrip 2024.3.5)

> Part of the Hacivat research library. Conventions: `../README.md`. Related: 07 (execution UX — uses the splitter defined here), 08 (data grid), 09 (DDL generation), 11 (settings catalog: every completion/formatter/smart-key option), 12 (localization), 13 (behavioral test corpus — families SPLIT, CARET, COMPL, JOIN, FMT, INSP). Connectivity, sessions, introspection model and catalogs are covered by documents 00–05 (other agent) and only referenced here.

---

## 1. Scope and sources

**Scope.** Everything DataGrip does *with SQL text*: dialect inventory and inheritance, lexing per dialect, statement splitting, error recovery, name resolution (layers and modes), completion (all providers and settings), JOIN inference, aliasing and qualification, ML ranking, the complete inspection and intention catalogs, the formatter, inlay hints, live/postfix templates, refactorings, navigation, language injections, and function-signature data files.

**Sources (all legitimate, no decompilation):**

| Source | Used for | License |
|---|---|---|
| `plugins/DatabaseTools/lib/database-plugin.jar`, `database-openapi.jar` non-class resources (extracted at `.refs/datagrip-extract/dbplugin`, `edwork`) | dialect descriptors `com/intellij/sql/dialects/*/*.xml`; `functions.xml` (26 dialects), `systemVars.txt`, `exceptions.txt`, `fallback.txt`, `simpleEntities.txt`, `preview/*.sql`; `*.minicat.xml.gzip`; `help/*-help.zip`; `META-INF/SqlPlugin.xml`, `SqlPluginCore.xml`, `sqlInjections.xml`, `intellilang-sql-support.xml`, `database-completion-ml-ranking.xml`; `sql_features/*.json`, `sql_model/model.bin`; `inspectionDescriptions/*.html` (68 + metaInformation.json); `intentionDescriptions/*` (35); `liveTemplates/sqlLiveTemplates.xml`; `postfixTemplates/*`; `inlayProviders/*` | Proprietary — described, not copied |
| Class-name inventories (`classes/database-plugin.txt`, `database-openapi.txt`, 19,710 + 278 names) | naming the mechanisms (e.g. `ScriptModel$SmartRange`, `SqlBatchBlock`, `MysqlDelimiterStatementImpl`, `SqlJoinSmartCompletionProvider$FuzzyKey`) | names only |
| Message bundles `SqlBundle` (785 keys), `DatabaseBundle` (2,942) | labels, inspection names, setting names | Proprietary |
| Bundled offline help (DataGripHelp plugin, Oct 2024) | behavior descriptions: `auto-completing-code`, `settings-code-completion`, `settings-smart-keys-sql`, `virtual-foreign-keys`, `inlay-hints`, `generating-code`, `refactoring-source-code`, `query-consoles` (resolve modes), `settings-languages-sql-dialects`, `settings-languages-sql-resolution-scopes`, `code-style-sql-*`, `using-language-injections`, `query-execution` | JetBrains docs |
| IntelliJ Community (Apache-2.0) | PSI, stub indexes, `ParserDefinition`, Grammar-Kit `pin`/`recoverWhile`, `ILazyParseableElementType` / `IReparseableElementType` incremental reparse, `CompletionContributor` + dummy identifier, `LookupElementWeigher`, `FormattingModelBuilder` (Block/Spacing/Indent/Wrap/Alignment), `MultiHostInjector`, `LanguageSubstitutor`, declarative inlay providers | Apache-2.0 |
| Prior notes `.refs/datagrip-extract/report-editor.md` (Turkish) | starting map; re-verified here | internal |

---

## 2. Executive summary

1. **One PSI vocabulary, ~30 parsers.** Each dialect is a separate IntelliJ *Language* with its own JFlex lexer (`_PgLexer`), Grammar-Kit generated parser (`PgGeneratedParser`) plus hand-written parts split by area (`PgDdlParsing`, `PgDmlParsing`, `PgExpressionParsing`, `PgPlParsing`, `PgOtherParsing`), but all produce shared `Sql*` PSI interfaces — inspections, completion and formatter are written once. [Observed: class names; dialect descriptors]
2. **31 parser languages registered** (PostgreSQL, MySQL, MariaDB, TSQL, AZURE, Oracle, OracleSqlPlus, DB2/DB2_IS/DB2_ZOS, SQLite, H2, HSQLDB, Derby, Exasol, Snowflake, BigQuery, ClickHouse, Cockroach, Vertica, Greenplum, Redshift, Sybase, HiveQL, SparkSQL, CassandraQL, CouchbaseQuery, Dynamo, Redis, MongoDB(SQL), MongoJS, NetSuite, GenericSQL, SQL92); MySQL dialect object also serves MemSQL/Gitbase. [Observed: `sql.dialect dbms=…`, `lang.parserDefinition language=…`]
3. **Generic "SQL" files are re-mapped to a concrete dialect** by `SqlLanguageSubstitutor` using path → project → global dialect settings (doc 11 ALG-SET-01). [Observed: class + help]
4. **Statement splitting is grammar-driven** (`ScriptModel` with `StatementIt`, `ParamIt`, `SmartRange`, `StrictRange`), with dialect delimiters as real syntax: MySQL `DELIMITER` is a statement node (`MysqlDelimiterStatementImpl`), T-SQL `GO` batches are `SqlBatchBlock` nodes, PG bodies are dollar-quoted tokens (`DollarQuoted`), Oracle SQL*Plus is a separate language. Generic/SQL:2016 use a user-selectable split mode. [Observed: class names; settings]
5. **Two resolve modes**: *Playground* (default for consoles; statements independent) and *Script* (default for files; `USE`, `SET search_path`, DDL earlier in the file affect later statements). [Observed: help `query-consoles`]
6. **Resolution layers**: live introspected model → shipped pre-introspected system catalogs (`*.minicat.xml.gzip`, registry `database.snapshots.auto=true`) → in-file DDL (script mode) → DDL data sources (doc 09) — scoped by SQL Resolution Scopes. [Observed: files, registry; Inferred: order]
7. **Completion**: one main `SqlCompletionContributor` + `SqlKeywordCompletionContributor` (parser-expected-token keywords) + `SqlJoinSmartCompletionProvider` (FK → virtual FK → fuzzy name key) + templates/postfix; type-matching completion (Ctrl+Shift+Space); abbreviation matching (camelCase/snake_case initials); window functions auto-insert `OVER()`. [Observed: classes, help]
8. **Completion settings**: suggest scope (search path / current scope / all schemas), qualification policy per level (DB/schema/table/alias × Always/On collisions/Never) per context (basic, JOIN, live templates, refactoring, drag-n-drop), JOIN alias use, ON operand order, non-strict FK by name matching, auto-alias, alias suggestions, custom alias table. [Observed: SO, help] (catalog: doc 11 §3.10)
9. **ML ranking**: SQL has its own feature policy and a shipped model (`sql_model/model.bin`), features version 0.2.0: 10 binary, 23 float, 6 categorical (parent PSI node types, dialect, statement start keyword, nearby SQL word, prefix matching type). [Observed: `sql_features/*.json`]
10. **66 registered inspections** (51 core SQL + 15 dialect: Oracle 3, PostgreSQL 1, MySQL 3, SQL Server 2, MongoJS 6) — names, default severities and enabled flags extracted; 4 disabled by default (named arguments, GOTO, missing column aliases, excessive JOIN count). [Observed]
11. **35 intentions** (incl. 2 DML preview intentions and "Run query in console"), each with before/after examples. [Observed]
12. **Formatter**: `SqlFormattingModelBuilder` + pre-formatters (`SqlCasePreFormatter`, `SqlQueryPreFormatter`) + a pattern-based block model (`SqlBatchBlock`, `SqlBlockStatementBlock`, `SeparatorContinuePattern`…); ~160–190 options per dialect page; 6 predefined styles; INSERT matrix alignment capped at 1,000 rows. [Observed]
13. **Inlay hints**: column names for INSERT VALUES / SELECT expressions in column-defining contexts / `*` / set operations; JOIN cardinality (`1..n<->1`, `0..n<->0..1`) per join type. [Observed: `inlayProviders/*`, help]
14. **13 live templates** (sel, selw, selc, lim, top, first, toprow, ins, upd, del, tab, view, col) with DB-aware macros (`dbObjectName()`, `dbColumns()`), and **5 postfix templates** (`.from`, `.cfrom`, `.afrom`, `.join`, `.cast`). [Observed]
15. **Injections**: 37 rules — type-driven (`json/jsonb` → JSON, `xml/xmltype` → XML, PG `reg*` casts → PostgreSQL fragments) and position-driven (regex operands → RegExp, XPath arguments → XPath2, `dblink` SQL → PostgreSQL, ClickHouse `INSERT … FORMAT JSON`); routine bodies injected by language (`FunctionBodyInjector`). [Observed: `sqlInjections.xml`, dialect descriptors]
16. **Function signature data**: `functions.xml` per dialect — 7,727 `<function>` entries total, prototypes in a compact typed mini-language with optional args `[ , x]`, macros for keyword grammars, flags `aggregate`, `analytic`, `parens=required|optional|prohibited`, `instance`/`self` (method-style calls). ClickHouse 783, Vertica 565, PG 513, Cockroach 506, Snowflake 476, MS SQL 475 … SQL92 29. [Observed: our counts]
17. **For Hacivat**: hand-written lossless lexer+parser per dialect family on a shared CST (rowan-style), a token-level splitter that works on broken code, salsa-style incremental semantics, layered catalogs, and declarative data files (our own function catalogs generated from DB docs/catalog queries). Concrete ALGs below.

---

## 3. Detailed findings

### 3.1 Dialect inventory and inheritance

**Two dialect layers exist** [Observed: directory structure]:
- `com/intellij/sql/dialects/<d>/` — *language* layer: parser definition, lexer, highlighter, commenter, formatter helper, function data, inspections. 28 directories.
- `com/intellij/database/dialects/<d>/` — *database* layer: introspector, model, DDL generator, minicat system catalogs, variants (e.g. `postgres_yugabyte.xml`; `mysql_aurora/gitbase/memsql/oceanbase/tidb/vitess.xml`). SQLite's language layer also lives here (`intellij.database.dialects.sqlite.xml`). Covered in docs 00–05 and 09.

**Module inheritance** (Kotlin module names in `META-INF`): `dialects.base`, `dialects.base.ex`, `postgresbase(.ex)`, `postgresgreenplumbase(.ex)`, `mysqlbase(.ex)`, `hivebase`, then leaf modules (`postgres`, `greenplum`, `redshift`, `cockroach`, `mysql`, `maria`, `hive`, `spark`, …). `.ex` = extended IDE-only features. [Observed] So: PG family = PostgreSQL ← Greenplum, Redshift, Cockroach (+ Yugabyte variant); MySQL family = MySQL ← MariaDB, MemSQL, Gitbase, Aurora, TiDB, Vitess, OceanBase; Hive family = Hive ← Spark; T-SQL family = MSSQL ← Azure (and Sybase separate but similar); Oracle ← OracleSqlPlus, NetSuite. [Observed: descriptors; Inferred: grammar sharing from shared base modules]

**Dialect descriptor content** (per language) [Observed: e.g. `postgres.xml`]: `sql.dialect dbms=… implementationClass=…Dialect`; `sql.dialectCodeStyleProvider`; `stubElementTypeHolder`; `lang.parserDefinition`; `lang.syntaxHighlighterFactory`; `lang.commenter` (`SqlNestedCommenter` for PG, Greenplum, Redshift, ClickHouse, DB2, Derby, Exasol, MSSQL/Azure, Sybase, Vertica, Generic, SQL92 — dialects where block comments nest; `MysqlCommenter` for MySQL/MariaDB which have `#` comments and non-nesting blocks); `sql.formatterHelper`; `multiHostInjector` for routine bodies (PG `FunctionBodyInjector`, Greenplum, Redshift, Snowflake, Cassandra, Exasol script bodies); dialect inspections.

**Version-aware variants**: e.g. `Pg83Lexer`, `Pg83Dialect`; MySQL pre-8.0 grammar check (`MysqlParsingInspection`). [Observed: class names; inspection]

**Generic vs SQL:2016**: Generic SQL = permissive, no error highlighting, keyword/table/column completion only; SQL:2016 = ANSI grammar with errors. Generic is the fallback dialect (`order="first"` on `GenericDialect`). [Observed: help, descriptor]

**Dialect selection for files**: path/project/global settings (doc 11 §3.7); `SqlDialectInspection` warns when a file has no dialect and suggests "the best matching dialect" ("Use <dialect>" link). [Observed: inspection description] Algorithm in ALG-SQL-10.

**Mini-languages** inside SQL: `SQLDateTime` (date/time literal language with per-dialect lexers like `_PgDtLexer`; inspected by `SqlDtInspection` for MySQL, Oracle, Db2, H2), function prototype language, SQL*Plus. [Observed]

### 3.2 Lexing specifics by dialect

DataGrip ships compiled lexers only (no `.flex`), so the table below combines DBMS rules (from vendor documentation; [Inferred: DBMS knowledge]) with DataGrip-specific evidence where available. This is the lexer-flag set Hacivat needs (ALG-SQL-02).

| Feature | PostgreSQL family | MySQL/MariaDB | T-SQL (MSSQL/Azure/Sybase) | Oracle (+SQL*Plus) | SQLite | Notes / evidence |
|---|---|---|---|---|---|---|
| Identifier quotes | `"x"` (`""` escape) | `` `x` `` (` `` ` escape); `"x"` only with ANSI_QUOTES | `[x]` (`]]` escape), `"x"` with QUOTED_IDENTIFIER ON | `"x"` | `"x"`, `` `x` ``, `[x]` all accepted | Code style "Quotation character" option [Observed: SO] |
| String literals | `'…'` (`''`), `E'…'` with backslash escapes, `U&'…'`, `B'…'`, `X'…'`, `$tag$…$tag$` | `'…'` and `"…"` (unless ANSI_QUOTES), backslash escapes by default (NO_BACKSLASH_ESCAPES off), `_utf8'…'`, `X'…'`, `0x…`, `b'…'` | `'…'`, `N'…'` (Unicode) | `'…'`, `N'…'`, `q'[…]'` alternative quoting with any delimiter pair | `'…'`, `X'…'` | `SqlUnicodeStringLiteralInspection` (N prefix) [Observed]; `DollarQuoted` class [Observed] |
| Line comments | `--` | `-- ` (dash-dash-space required), `#` | `--` | `--`, SQL*Plus `REM` | `--` | `MysqlCommenter` [Observed] |
| Block comments | `/* */` **nesting** | `/* */` non-nesting, `/*! … */` and `/*!80000 … */` executable version comments, `/*+ hints */` | `/* */` nesting | `/* */`, `/*+ hints */` | `/* */` | `SqlNestedCommenter` vs `MysqlCommenter` [Observed] |
| Statement delimiter | `;` | `;` or custom via `DELIMITER xx` (client-side) | `;` optional; `GO` batch separator (client-side, own line, optional count) | `;` in SQL; `/` on its own line ends PL/SQL blocks / SQL*Plus buffer | `;` | `MysqlDelimiterStatementImpl`, `SqlBatchBlock` [Observed] |
| Variables | `:name` (psql), PL/pgSQL locals | `@user`, `@@system`, `@@session.x` | `@local`, `@@global` | `:bind`, `&subst`/`&&subst` (SQL*Plus DEFINE), PL/SQL locals | `?`, `?NNN`, `:name`, `@name`, `$name` | `sybase/simpleEntities.txt` lists `@@vars` [Observed] |
| Positional params | `$1` | `?` | `@p1` (driver) | `:1` | `?NNN` | User parameter patterns (doc 11 §3.4, doc 07 ALG-EXEC-03) |
| Operators of note | `::` cast, `->`/`->>` JSON, `@>`, `~`, `||` | `->`, `->>`, `<=>`, `||` = OR unless PIPES_AS_CONCAT | `+` concat | `||`, `=>` named args | `||` | — |
| Case of unquoted identifiers | folded to lower | depends on `lower_case_table_names`/OS | case-insensitive per collation | folded to UPPER | case-insensitive | affects resolve (ALG-SQL-04) and Turkish-I handling |
| Special tokens | `COPY … FROM STDIN` data block terminated by `\.`; psql meta-commands `\x` | `DELIMITER`, `SOURCE` | `GO [n]`, `:setvar`/`:r` (SQLCMD mode) | SQL*Plus commands (`SET`, `PROMPT`, `@file`, `SPOOL`, `DEFINE`, `EXEC`) | dot-commands `.tables` (CLI only) | SQL*Plus = `OraPlusDialect` [Observed] |

### 3.3 Statement splitting

**Evidence of the model** [Observed: class names]: `ScriptModel` (iterators `StatementIt`, `ParamIt`, `ModelIt`, `ExternalIt`; ranges `StrictRange` (exact statement text), `SmartRange` (statement + attached leading comments / trailing delimiter), `PositionRange`, `ChosenRange` (user choice)); `SqlScriptModel` with `DocScript` (live document) and `FileScript` (run a file without opening — "Run SQL script" configurations, doc 07); `ScriptModelUtilCore$ModificationIterator`; `TrackingDocScript` (keeps ranges valid while the user edits during execution — prior notes). Statements are PSI nodes, so splitting = walking top-level statement nodes, including:
- **MySQL** `DELIMITER` lines parsed as statements that switch the active delimiter for subsequent parsing. [Observed: `MysqlDelimiterStatementImpl`]
- **T-SQL** `GO` creates batch nodes (`SqlBatchBlock`); "Largest statement or batch" execute option executes the current batch. [Observed: class; help `query-execution`]
- **Oracle SQL*Plus** dialect understands `/` and SQL*Plus commands; plain Oracle SQL dialect treats PL/SQL blocks by grammar. [Observed: two parser definitions]
- **PostgreSQL** routine bodies are dollar-quoted tokens, injected with the body language (plpgsql/sql) for highlighting/resolve. [Observed: `FunctionBodyInjector`, `DollarQuoted`]
- **Generic/SQL:2016**: user picks *Into valid ANSI SQL statements or by separator* (default), *Into ANSI SQL statements*, *By statement separator* (`;`). [Observed: help]

Because the splitter in DataGrip depends on the full parser, it degrades on broken code into "parse until error, then recover". Hacivat needs a **token-level splitter** that is correct without a full parse (ALG-SQL-01) and a parser-based refinement for subquery-level ranges (doc 07 ALG-EXEC-01).

### 3.4 Error recovery and incremental reparse

- Grammar-Kit parsers use **pin** (once a rule's pinned token matched, the rule commits and missing parts become error elements) and **recoverWhile** (skip tokens until a recovery-set token: e.g. statement keywords, `;`, `)`), so the PSI tree always covers the whole file and contains `PsiErrorElement`s. [Inferred: Grammar-Kit mechanism (IntelliJ Community, Apache-2.0); generated parser classes present]
- Parse-error quick fixes exist ("insert comma", "add AS", "change dialect…"). [Observed: SqlBundle keys in prior notes]
- **Lazy, independently reparseable bodies**: routine bodies (dollar-quoted, BEGIN…END) are lazy-parseable elements; on edit, IntelliJ's `BlockSupport` tries the smallest enclosing reparseable element first, falling back to full-file reparse. [Inferred: platform mechanism; `ReparseType` class name observed]
- **Stub indexes** over SQL files index declared objects (CREATE TABLE/VIEW/ROUTINE names, columns) so DDL in project files can be resolved without parsing (`stubIndex` ×3 and `fileBasedIndex` ×2 in `SqlPluginCore.xml`). [Observed: counts]

### 3.5 Name resolution: layers, scopes and modes

**Layers** (searched in order) [Observed: components; Inferred: precedence]:
1. **Local scopes** in the statement: FROM items (tables, aliases, derived tables with column aliases, LATERAL), CTEs (recursive CTE sees itself), SELECT-list aliases (visible in ORDER BY, dialect-dependent in GROUP BY/HAVING), USING/NATURAL merged columns, outer query scopes (correlated; highlighted with the "Outer query column" color), trigger NEW/OLD and transition tables (`SqlTriggerTransitionInspection`), routine parameters and local variables (with "misleading reference" detection when a name matches both a column and a parameter — `SqlMisleadingReferenceInspection`), labels, cursors.
2. **Script-mode DDL overlay** (Script resolve mode): objects created/dropped/altered earlier in the same file, plus `USE db`, `SET search_path`, `ALTER SESSION SET CURRENT_SCHEMA` changing the default context.
3. **Live database model** of the resolution scope (doc 00–05): data source → databases → schemas, with the *current schema/search path* tracked from the session (`SearchPathSender`/`SearchPathReceiver`, `SearchPathPopup`, "Set Current Schema" action).
4. **Pre-introspected system catalogs** (`postgres.minicat.xml.gzip`, `mysql.minicat.xml.gzip` …) injected when `database.snapshots.auto=true`, so `pg_catalog`, `information_schema`, built-in routines resolve even if not introspected.
5. **Built-in function catalog** (`functions.xml`) and system variables (`systemVars.txt`), exception names (`exceptions.txt` for PL/pgSQL `EXCEPTION WHEN …`, Oracle predefined exceptions), fallback type names (`fallback.txt`).
6. **DDL data sources / DDL mappings** (doc 09) — virtual data sources built from SQL files.

**Resolve modes** [Observed: help `query-consoles`; classes `SqlFileResolveMode`, `SetFileResolveModeActionGroup`, `ResolveModePopup`]:
- *Playground* — default for query consoles; every statement resolves against the console's default context; statements are independent.
- *Script* — default for local SQL files; statements are sequential: `USE`/`SET search_path TO` changes context for later statements, and DDL earlier in the file is visible later.
- Default for consoles configurable in Settings › Database › Other ("Default resolve mode for consoles"). Per-file switch in the editor toolbar.

**Resolution scopes** (Settings › SQL Resolution Scopes) — map paths to data source/DB/schema sets; project mapping as default. [Observed]

**Case and quoting rules** per dialect drive identifier matching (PG folds unquoted to lower; Oracle to UPPER; MySQL table names depend on server settings; T-SQL collation). [Inferred]

**Diagnostics from resolution**: `SqlResolveInspection` (ERROR, unresolved), `SqlAmbiguousColumnInspection`, `SqlMisleadingReferenceInspection`, `SqlCurrentSchemaInspection` (schema not introspected), `SqlNoDataSourceInspection`, `SqlStorageInspection` (object source changed in DB). Quick fixes: create column/table, attach data source, choose schema, introspect/refresh, qualify. [Observed]

### 3.6 Completion

**Providers** [Observed: class names + help]:
- `SqlCompletionContributor` — references (objects/columns/variables) in scope, with qualification policy and alias generation.
- `SqlKeywordCompletionContributor` (`$SqlCompletionState`) — keywords the parser can accept at the caret (Inferred: computed by running the parser with a dummy identifier and collecting expected tokens).
- `SqlJoinSmartCompletionProvider` (`$TableInfo`, `$JoinKind`, `$ColumnQName`, `$FuzzyKey`, `$KeyConsumer`, `$ResultConsumer`) — complete whole `JOIN t ON …` items.
- Specialized behaviors from help: field lists for SELECT/INSERT/GROUP BY ("complete list of fields"), window functions insert `OVER()` and place the caret, abbreviated object names (initials of camelCase / snake_case / hyphenated words), abbreviated statements, names of new objects in DDL (e.g. after ALTER), alias suggestions after table names, type-matching (smart) completion filtering by expected type (Ctrl+Shift+Space), statement completion (Ctrl+Shift+Enter balances parens/adds semicolons), hippie completion, postfix and live templates in the list. [Observed: help `auto-completing-code`]
- Experimental: constraint-name suggestions (`database.completion.experimental.names=false`). [Observed: registry]
- In data grid cells: "Enable immediate completion in grid text cells". [Observed: SO]

**Settings**: see doc 11 §3.10/3.11 (suggest scope, qualification matrix, JOIN options, aliases, custom aliases).

**Insertion rules** [Observed: help + settings; Inferred where noted]:
- Function: parentheses inserted (platform "Insert parentheses automatically"), parameter-info popup shown; `parens=prohibited` functions (`CURRENT_USER`, `SESSION_USER`) inserted without `()`; `parens=optional` either. [Observed: functions.xml flags; Inferred: use in completion]
- Table: optional generated alias (ALG-SQL-06), qualification per policy (ALG-SQL-07).
- JOIN item: `JOIN target alias ON alias.fk = src.pk` with operand order per "Invert order" (default: FROM-side table first). [Observed: help]

### 3.7 ML ranking (completion)

[Observed: `database-completion-ml-ranking.xml`, `sql_features/*`, `sql_model/model.bin`]
- Extension points: `features.policy language="SQL"` (`SqlFeaturesPolicy`), `completion.ml.model` (`SqlMLRankingProvider`), `completion.ml.contextFeatures` (`SqlContextFeatures`), `completion.ml.elementFeatures` (`SqlElementFeatures`).
- Feature set version 0.2.0; `features_order.txt` lists 98 model inputs (one-hot expanded):
  - **Binary (10)**: grouping, liftShorter, middleMatching, recent-places-contains, prefix_exact, proximity in-resolve-scope / opened-in-editor / sdk-or-library, sorter, templates.
  - **Float (23)**: lines diff, stemmed line/parent similarity (max/mean), caret column/indent level/line number, file n-gram scores (forward/reversed), prefix length and many prefix-match scores (greedy, with case, matched words relative/score, same start count), priority, result length, session query number, session sorting order, stats (usage statistics).
  - **Categorical (6)**: `ml_ctx_common_parent_1` (SqlReferenceExpression, SqlSelectStatement, SqlFile, SqlCodeFragment, SqlColumnDefinition, SqlUpdateStatement, OTHER), `ml_ctx_common_parent_2` (Where/Select/GroupBy clauses, BinaryExpression, TypeElement, ReferenceList, FunctionCall, AsExpression, …), `ml_ctx_sql_dialect` (Pg, MySQL, Oracle, MSSQL, Generic, Maria, ClickHouse, SQL92, OraPlus, H2, Db2 LUW, Vertica, SQLite, OTHER), `ml_sql_starts` (first keyword of the statement: LEFT, INSERT, UNION, ORDER, GROUP, TABLE, ON …), `ml_sql_word` (nearest keyword: MAX, INSERT, COUNT, SELECT, CASE, EXISTS, VALUES, AND, FROM, WHERE, UPDATE, LEFT, DISTINCT, AS, LIKE, CREATE …), `prefix_matching_type` (START_WITH, GREEDY, OTHER).
- Model: binary file `model.bin` loaded by the platform's ML ranking (decision-tree ensemble — [Inferred: platform `completion-ml-ranking` uses CatBoost-style tree models]). Data collected locally; ranking only reorders. [Observed: help]

### 3.8 Inspection catalog (complete)

Default severity / enabled-by-default from descriptors [Observed: `SqlPlugin.xml`, dialect XMLs]; code-quality category from `inspectionDescriptions/metaInformation.json` [Observed]; trigger summarized in our words from descriptions.

| ID (shortName) | Name | Sev. | On | Fires when (trigger) |
|---|---|---|---|---|
| SqlResolveInspection | Unresolved reference | ERROR | ✓ | A reference resolves to nothing in any layer (e.g. `users.id` without `users` in FROM). |
| SqlTypeInspection | Types compatibility | WARN | ✓ | Type errors (operands, assignments, arguments). |
| SqlUnusedVariableInspection | Unused variable | WARN | ✓ | Routine parameter/variable never read. |
| SqlUnusedSubqueryItemInspection | Unused subquery item | WARN | ✓ | Subquery select item/alias not referenced by the outer query (grayed). |
| SqlIdentifierInspection | Identifier should be quoted | WARN | ✓ | Reserved keyword used as an identifier unquoted. |
| SqlAmbiguousColumnInspection | Ambiguous reference | WARN | ✓ | Unqualified column exists in ≥2 FROM items. |
| SqlInsertValuesInspection | VALUES clause cardinality | WARN | ✓ | VALUES row length ≠ target column count. |
| SqlDialectInspection | SQL dialect detection | WARN | ✓ | File has no dialect; suggests best match. |
| SqlCurrentSchemaInspection | Current console schema introspected | WARN | ✓ | Session schema/database not introspected. |
| SqlStorageInspection | SQL source modification detection | WARN | ✓ | Object source changed in DB vs local copy (on introspection). |
| SqlNoDataSourceInspection | No data sources configured | WARN | ✓ | No data source in project. |
| SqlDerivedTableAliasInspection | Each derived table should have alias | WARN | ✓ | Subquery in FROM without alias (MySQL/PG requirement). |
| SqlShouldBeInGroupByInspection | Column should be in group by clause | WARN | ✓ | Non-aggregated select column missing from GROUP BY. |
| SqlAutoIncrementDuplicateInspection | Auto-increment duplicate | WARN | ✓ | Two auto-increment columns (MySQL, MSSQL, Db2). |
| SqlDropIndexedColumnInspection | Index is dependent on column | WARN | ✓ | DROP COLUMN used by index (MSSQL, Sybase). |
| SqlAddNotNullColumnInspection | Adding not null column without default value | WARN | ✓ | ALTER ADD NOT NULL without DEFAULT. |
| SqlCheckUsingColumnsInspection | Check using clause columns | WARN | ✓ | USING column missing in one side. |
| SqlSignatureInspection | Function signature | WARN | ✓ | Built-in call has wrong arg count/types/keywords (uses functions.xml). |
| SqlSideEffectsInspection | Statement with side effects | WARN | ✓ | Modifying statement in a read-only data source. |
| SqlWithoutWhereInspection | Delete or update statement without where clauses | WARN | ✓ | DELETE/UPDATE without WHERE (also execution-time warning, doc 07). |
| SqlJoinWithoutOnInspection | Unsafe 'join' clause in 'delete' statement | WARN | ✓ | DELETE … JOIN without ON/WHERE. |
| SqlUnreachableCodeInspection | Unreachable code | WARN | ✓ | Statements after THROW/RETURN/… in routines. |
| SqlIllegalCursorStateInspection | Illegal cursor state | WARN | ✓ | FETCH/CLOSE on possibly closed cursor, OPEN on open. |
| SqlNamedArgumentsInspection | Named arguments should be used | WARN | ✗ | Positional args in routine calls. |
| SqlGotoInspection | Usages of GOTO statements | WARN | ✗ | Backward GOTO / GOTO leaving loop. |
| SqlTriggerTransitionInspection | Suspicious code in triggers | WARN | ✓ | NEW in DELETE trigger, OLD in INSERT trigger, etc. |
| SqlRedundantOrderingDirectionInspection | Redundant ordering direction | WARN | ✓ | Explicit `ASC`. |
| SqlAggregatesInspection | Aggregate-related problems | WARN | ✓ | HAVING/ORDER BY columns not grouped; aggregate misuse (PK grouping exception). |
| SqlCaseVsIfInspection | Using CASE instead of conditional function and vice versa | WARN | ✓ | `CASE` replaceable by `IF()` (MySQL) and back. |
| SqlCaseVsCoalesceInspection | Using CASE instead of COALESCE function and vice versa | WARN | ✓ | `CASE WHEN x IS NOT NULL THEN x ELSE y END`. |
| SqlStringLengthExceededInspection | Implicit string truncation | WARN | ✓ | Literal longer than VARCHAR(n) variable. |
| SqlRedundantAliasInspection | Redundant alias expressions | WARN | ✓ | Alias column list identical to table columns. |
| SqlRedundantLimitInspection | Redundant row limiting in queries | WARN | ✓ | LIMIT/FETCH inside EXISTS without OFFSET. |
| SqlMultipleLimitClausesInspection | Multiple row limiting/offset clauses | WARN | ✓ | TOP + OFFSET/FETCH together. |
| SqlInsertIntoGeneratedColumnInspection | Insertion into generated columns | WARN | ✓ | INSERT target is GENERATED ALWAYS. |
| SqlDuplicateColumnInspection | Duplicating column name in SELECT | WARN | ✓ | Duplicate output alias (where illegal, e.g. Sybase). |
| SqlRedundantElseNullInspection | Redundant ELSE NULL clause | WARN | ✓ | `ELSE NULL` in CASE. |
| SqlInsertNullIntoNotNullInspection | Insert NULL into NOT NULL column | WARN | ✓ | Literal NULL into NOT NULL column. |
| SqlRedundantCodeInCoalesceInspection | Redundant code in COALESCE call | WARN | ✓ | Args after first non-null-guaranteed arg. |
| SqlUnicodeStringLiteralInspection | Unicode usage in SQL | WARN | ✓ | Non-ASCII literal without `N` prefix (T-SQL). |
| SqlUnusedCteInspection | Unused common table expression | WARN | ✓ | CTE never referenced. |
| SqlMissingColumnAliasesInspection | Missing column aliases | WARN | ✗ | Output expression without alias. |
| SqlMisleadingReferenceInspection | Misleading references | WARN | ✓ | Name matches both column and parameter/variable. |
| SqlMissingReturnInspection | Missing return statement | ERROR | ✓ | Function body lacks RETURN. |
| SqlTransactionStatementInTriggerInspection | Use of transaction management statements in triggers | WARN | ✓ | COMMIT/ROLLBACK in trigger body. |
| SqlCallNotationInspection | Using of named and positional arguments | ERROR | ✓ | Positional after named (PG, Oracle, Db2). |
| SqlDeprecateTypeInspection | Deprecated type | WARN | ✓ | Oracle LONG; MSSQL TEXT/NTEXT/IMAGE. |
| SqlShadowingAliasInspection | Column is shadowed by alias | WARN | ✓ | SELECT alias equals a FROM column name. |
| SqlDtInspection (language SQLDateTime) | Ill-formed date/time literals | WARN | ✓ | Bad `DATE/TIME/TIMESTAMP '…'` literal (MySQL, Oracle, Db2, H2). |
| SqlConstantExpressionInspection | Constant expression | WARN | ✓ | Condition always true/false/NULL (`'a'='a'`, `x = NULL`). |
| SqlJoinCountInspection | Excessive JOIN count | WARN | ✗ | More joins than threshold. |
| OraOverloadInspection | Overloading errors | WARN | ✓ | Overloads differing only by subtype. |
| OraUnmatchedForwardDeclarationInspection | Forward declaration without definition | ERROR | ✓ | Declared subprogram never implemented. |
| OraMissingBodyInspection | Missing body for package/object type specification | WARN | ✓ | Spec with routines/cursors but no body. |
| PgSelectFromProcedureInspection | Postgres: Select from procedure call | WARN | ✓ | `SELECT * FROM record_fn()` without column-definition alias; DBLINK too. |
| MysqlParsingInspection | Unsupported syntax in pre-8.0 versions | WARN | ✓ | Parenthesized UNION operands on MySQL < 8. |
| MysqlLoadDataPathInspection | LOAD statement path | WARN | ✓ | `~` in LOAD DATA path. |
| MysqlSpaceAfterFunctionNameInspection | Whitespace between the function name and the open parenthesis | ERROR | ✓ | `MAX (x)` (fails without IGNORE_SPACE). |
| MsBuiltinInspection | Builtin functions | WARN | ✓ | ISNULL replacement longer than check expr (truncation). |
| MsOrderByInspection | ORDER BY in queries | ERROR | ✓ | ORDER BY without TOP/OFFSET/FOR XML in view/subquery/CTE. |
| MongoJSResolveInspection / MongoJSExtResolveInspection | Resolution problems | WARN | ✓ | Unknown collection/method. |
| MongoJSSideEffectsInspection / …Ext… | Statement with side effects | WARN | ✓ | Write in read-only mode. |
| MongoJSDeprecationInspection / …Ext… | Deprecated element | WARN | ✓ | `insert()` → `insertOne()` quick fix. |

Unregistered-but-described: `SqlConstantConditionInspection`, `SqlNullComparisonInspection` (descriptions shipped, no registration found → likely merged into `SqlConstantExpressionInspection`). [Observed: absence; Inferred: merge]

Code-quality categories (metaInformation.json): Reliability, Code Style, Performance, Sanity (e.g. SqlDialect = Sanity; SqlUnusedCte = Performance). [Observed]

### 3.9 Intention catalog (complete)

[Observed: `intentionDescriptions/*`, `SqlPlugin.xml`] (before → after paraphrased)

| Intention | Effect |
|---|---|
| SqlAddExternalRelationIntention | Save a JOIN's relation as a virtual FK in `external-data.xml` ("Store table relation"). |
| SqlAddGroupedColumnToSelectIntention | Add a GROUP BY column to the SELECT list. |
| SqlConvertBetweenToComparisonIntention / …ComparisonToBetween… | `x BETWEEN a AND b` ↔ `x >= a AND x <= b`. |
| SqlConvertEqualityToInIntention / …InToEquality… | `x=1 OR x=2` ↔ `x IN (1,2)`. |
| SqlConvertJoinOnToNatural / JoinOnToUsing / JoinUsingToNatural / JoinUsingToOn / NaturalJoinToOn / NaturalJoinToUsing | Six-way conversions between ON, USING, NATURAL. |
| SqlConvertToSubqueryIntention | Wrap a query as `SELECT * FROM (…) alias`. |
| SqlDropInsertColumnIntention | Remove a column and its values from INSERT. |
| SqlExpandColumnListIntention | Expand `*` to explicit columns. |
| SqlExtractNamedQueryIntention | Subquery → CTE (`WITH b AS (…)`). |
| SqlFlipExpressionIntention | Swap operands (adjusting `<` ↔ `>`). |
| SqlInjectByTypeIntention | Associate a dialect/language with a domain type; injects all literals of that type. |
| SqlInlineCteIntention | Inline a CTE into its use. |
| SqlInlineTableAliasIntention / SqlIntroduceTableAliasIntention | Remove/add table alias, re-qualifying per settings. |
| SqlMergeInsertStatementsIntention / SqlSplitInsertStatementIntention | Multiple single-row INSERTs ↔ one multi-row INSERT. |
| SqlPreviewDmlAction (Delete / Update) | Preview rows a DELETE/UPDATE would affect (runs a derived SELECT; doc 07). |
| SqlQualifyIdentifierIntention / SqlUnqualifyIdentifierIntention | Add/remove qualifiers. |
| SqlQuoteIdentifierIntention / SqlUnquoteIdentifierIntention | Quote/unquote identifiers with dialect quote char. |
| SqlReplaceColumnNameWithNumberIntention / …NumberWithName… | ORDER BY/GROUP BY name ↔ ordinal (not in Cassandra, ClickHouse, Hive, Spark, HSQLDB for ORDER BY). |
| SqlReplaceDistinctWithGroupByIntention / …GroupByWithDistinct… | DISTINCT ↔ GROUP BY all columns. |
| SqlSplitStringLiteralIntention | Split a literal into concatenation. |
| RunQueryInConsoleIntentionAction | Run SQL string (from host language) in a console. |

Plus inspection quick fixes (Replace with IF/COALESCE/CASE, qualify, add alias, quote, change dialect, introspect schema, disable read-only, etc.).

### 3.10 Formatter

**Architecture** [Observed: class names; Inferred: flow]:
1. `SqlPreFormatProcessor` → pre-formatters: `SqlCasePreFormatter` (keyword/identifier/type/function case and quotation changes as *text edits* before layout), `SqlQueryPreFormatter` (query-level normalizations, e.g. AS insertion per "Use AS").
2. `SqlFormattingModelBuilder` → `SqlFormattingModelMaker` builds a block tree from PSI using **phrase patterns** (`SeparatorContinuePattern`, plus Flow/StartUntil/StartStop/Tail/AdjacentGroup patterns per prior notes) mapped onto role blocks (query, clause, parenthesized list, CASE, binary, CREATE TABLE, column, constraint, ALTER, MERGE, loop, declare, `SqlBatchBlock`, `SqlBlockStatementBlock`, comments, hints).
3. Platform formatter computes whitespace from `Spacing`, `Indent`, `Wrap`, `Alignment` of blocks; special aligners do INSERT VALUES matrix alignment (≤ `sql.formatter.insert.matrix.limit.rows` = 1000 rows) and "Align `=` / AS / types".
4. Injected languages inside SQL are formatted too (`sql.formatter.consider.language.injection=true`); error regions are left unchanged [Inferred: platform behavior for error elements].
5. `lineIndentProvider` gives fast Enter indentation without building the full model. [Observed: `SqlPluginCore.xml` `lineIndentProvider`]

**Options** (complete list in doc 11 §3.12): Case, Queries, DDL, Code, Expressions, Tabs and Indents, Wrapping, Code Generation; per-dialect pages with inheritance; presets Modern, Joe Celko, Allman, Whitesmiths, Egypt, Old idea. Preview texts per dialect family in `preview/*.sql` (7 files each for sql92, postgres, mysql, mssql, oracle, sybase, db2, derby, h2, hsql, sqlite). [Observed]

### 3.11 Inlay hints

[Observed: `SqlPlugin.xml` declarative providers; `inlayProviders/*/*.sql` previews; help `inlay-hints`]
- `sql.column.names.inlay.hints` (`SqlColumnNamesInlayHintsProvider`), options: *In INSERT values* (`/*id*/` before each value), *In SELECT expressions* (when context defines column names: `CREATE VIEW v(x, y) AS SELECT …`, `INSERT … SELECT`), *For `*` references* (lists columns behind `*` in such contexts), *For set operations* (column names of UNION/INTERSECT/EXCEPT branches). Only in consoles/files, not injected SQL.
- `sql.join.cardinality.hints` (`SqlJoinCardinalityInlayHintsProvider`), per join type: INNER (`1..n<->1`), LEFT (`1..n<->0..1`), RIGHT (`0..n<->1`), FULL (`0..n<->0..1`) — derived from FK direction, PK/unique on target, nullability of the referencing column and join type (ALG-SQL-11).

### 3.12 Live and postfix templates

**Live templates** [Observed: `sqlLiveTemplates.xml`; template bodies not reproduced]:

| Abbrev. | Context | Purpose | Variables (macro) |
|---|---|---|---|
| sel | SQL query | `SELECT * FROM <table>` | table: `dbObjectName()` |
| selw | SQL query | select with WHERE on an alias | table, alias: `complete()` (default "alias") |
| selc | SQL query | `SELECT COUNT(*)` with WHERE | table, alias |
| lim | SQL query | select with LIMIT n | table, size (default 100) |
| top | MSSQL/Azure/Synapse | `SELECT TOP n` | table, size 100 |
| first | Db2, Derby | `FETCH FIRST n ROWS ONLY` | table, size 100 |
| toprow | Oracle | ROWNUM-limited select | table, size 100 |
| ins | SQL statement | INSERT with column list + parameter info | table, columns: `dbColumns()`, info: `showParameterInfo()` |
| upd | SQL statement | UPDATE SET col = value | table_name, col, value |
| del | SQL statement | DELETE FROM table | table |
| tab | SQL statement | CREATE TABLE skeleton | table, col, type ("int"), null ("not null") |
| view | SQL statement | CREATE VIEW AS SELECT | view, table |
| col | SQL code | column definition | col, type, null |

Template contexts: `SQL_QUERY`, `SQL_STATEMENT`, `SQL_CODE`, and dialect-specific `SQL_QUERY_<DIALECT>`. Custom macros `dbObjectName`, `dbColumns` are `liveTemplateMacro` extensions (2 in `SqlPlugin.xml`). [Observed]

**Postfix templates** [Observed: `postfixTemplates/*`, help]: `.from` (table → `SELECT … FROM table`), `.cfrom` (all columns), `.afrom` (all columns with generated aliases), `.join` (FROM + JOIN if exactly one FK candidate), `.cast` (`CAST(expr AS …)`). Users can add SQL postfix templates with `$EXPR$` and dialect filters; expansion key Tab/Space/Enter; Ctrl+J lists applicable ones.

### 3.13 Refactorings and navigation

- **Rename** (Shift+F6) for DB objects, columns, aliases, variables, with comments/strings/text occurrences; renaming a real DB object generates DDL via the migration dialog (doc 09); in-place rename with gutter suggestion. [Observed: help]
- **Introduce variable** (Ctrl+Alt+V), **Subquery as CTE**, **Extract routine** (table function, Ctrl+Alt+M), **Inline** (CTE, alias) via `inlineActionHandler` ×2. [Observed]
- **Navigation**: go to declaration (to DDL or Database Explorer node; legacy behavior registry keys), find usages including through `*` and alternative names, highlight usages (`highlightUsagesHandlerFactory` ×3: e.g. columns behind `*`, INSERT column↔value pairs), structure view grouped by DDL/DML/SELECT (`fileStructureGroupRuleProvider` ×2), breadcrumbs and sticky lines for SQL, select in Database Explorer, package spec↔body (Oracle), Search Everywhere database tab (advanced setting), routine call hierarchy (experimental registry). [Observed]
- **Structural search** for SQL (platform SSR). [Observed: help topic present]
- **Generate** (Alt+Insert): DB entities (function, procedure, index, rule, trigger, view, materialized view, schema, database, role, user, sequence, enum, domain, composite type, foreign table, aggregate, operator, collation, FDW, access method, extension table) — opens Create dialogs (doc 09). [Observed: help `generating-code`]
- **Editing aids**: smart Enter closes blocks (`BEGIN`→`END`, `LOOP`→`END LOOP`, `BEGIN TRY`→`END TRY`), Enter inside string inserts concatenation, move statement up/down, `enterHandlerDelegate` ×2, `SqlTypedHandler`. [Observed]

### 3.14 Language injections

[Observed: `sqlInjections.xml` (37 rules), `intellilang-sql-support.xml`, dialect `multiHostInjector`s]
- **By cast/type pattern**: values cast to or typed as `json`/`jsonb` → JSON; `xml`/`xmltype` → XML; PG `regproc`, `regprocedure`, `regoper`, `regoperator`, `regclass`, `regtype`, `regrole`, `regnamespace`, `regconfig`, `regdictionary` → PostgreSQL *fragments* (function reference/signature, operator reference/signature, table reference, type element, role, schema, generic reference) — so `'public.users'::regclass` is resolved and navigable.
- **By position** (pattern DSL `sqlElement().forDialects(…).functionArgument(i, sqlFunctionCall().name(…))` and `binaryExpressionArgument`): RegExp for `~`/`SIMILAR`/`REGEXP`/`RLIKE` operands and `regexp_*` functions (PG, Exasol, H2, HSQLDB, MySQL, Oracle, SQLite, ClickHouse); XPath2 for `xpath`, `xmlexists`, `ExtractValue`, `extract`, T-SQL `nodes/exist/query/value`, Sybase `xmlextract`; XML for `xmlparse`/`xmltype`/`UpdateXML`; JSON for Oracle `json_query/json_value`, T-SQL `openjson`, ClickHouse `visitParam*` and `INSERT … FORMAT JSON` data; PostgreSQL for `dblink(…, 'SQL')`; JSON for `CAST(… AS JSON)` (MySQL, PG) and MySQL `CONVERT`.
- **Routine bodies**: PG `FunctionBodyInjector` injects by `LANGUAGE` (plpgsql/sql) into `$$` bodies; similar injectors for Greenplum, Redshift, Snowflake, Cassandra, Exasol scripts.
- **SQL inside other languages** (IntelliLang): SQL injected into host-language strings, with "Enable in string literals with SQL injection" for parameters; `SqlInjectByTypeIntention` for domain types. [Observed]

### 3.15 Function signature data (`functions.xml`) and other data files

**Format** [Observed: structure; described in our words]:
- Root `<functions>` contains `<macro name=… [display-name=…]>` definitions and `<function>` entries.
- `<function>` has one or more `<name>` (aliases, e.g. `ceil`/`ceiling`) and one or more `<prototype>` (overloads).
- Prototype mini-language: `(argName:type, …):returnType`; optional parts in `[ … ]` (nestable: `[, a:B [, b:str]]`); keyword syntax and macro references in braces/brackets for grammar-like arguments (e.g. `EXTRACT(time_unit FROM …)` where `time_unit` is a macro enumerating units); `setof(record)` returns; `ANY` type.
- Type codes are short dialect-local aliases: `str`, `N` (numeric), `dp` (double precision), `dpn` (numeric family), `B` (boolean), `bytes`, plus full names (`timestamp`, `jsonb`, `geometry`, `interval`, …). Most frequent across all files: str (6,105), timestamp, array, dp, geometry, bytes, date, table, geography, oid, datetime, bigint. [Observed: our counts]
- Attributes: `aggregate='true'`, `analytic='true'` (window-capable), `parens='required|optional|prohibited'`, `instance='true|false'` and `self='geometry|geography|SCHEMA'` (method-style calls such as spatial `geom.STArea()`), `location='table_engine_spec|dictionary_source_spec|codec_spec|table_function|…'` (ClickHouse: where in grammar the "function" may appear), `insensitive='true'`.

**Per-dialect counts of `<function>` entries** [Observed]: ClickHouse 783 · Vertica 565 · PostgreSQL 513 · Cockroach 506 · Snowflake 476 · MS SQL 475 · Greenplum 432 · MariaDB 396 · MySQL 385 · Redshift 344 · Spark 307 · BigQuery 282 · Hive 273 · Oracle 255 · NetSuite 255 · Exasol 244 · Db2 213 · H2 197 · Sybase 188 · HSQLDB 176 · Couchbase 148 · Mongo 146 · Derby 78 · Cassandra 56 · SQL92 29 · Dynamo 5. Total 7,727 functions / 8,876 prototypes / 123 macros.

**Other files**: `systemVars.txt` (one name per line; MySQL 711, ClickHouse 530, Oracle 220, PG/Greenplum/Redshift 217, Cockroach 55, BigQuery 14) for `SET`/`SHOW`/`@@` completion; `exceptions.txt` (PL/pgSQL condition names 231; Oracle predefined 21); `fallback.txt` (sections `object-type:` / `format:` with type names usable even without a catalog — PG 108, ClickHouse 118 …); `sybase/simpleEntities.txt` (`@@name : variable : type`). Minicats: `*.minicat.xml.gzip` per DB (pre-introspected system schemas, version-ranged). Offline docs: `help/<db>-help.zip` (16 DBMSs) for Quick Documentation of built-ins. [Observed]

---

## 4. Algorithm specifications

### ALG-SQL-01: Statement splitter (per dialect, token-level, error-tolerant)
- **Problem:** Split a script into statements (and T-SQL batches) for execution, navigation, gutter status and reparse units, correctly on incomplete/broken code and without a full parse.
- **Inputs / outputs:** `text`, `dialect`, `options{generic_mode: ParseOrSeparator|Parse|Separator, custom_delimiter?}` → `Vec<Stmt{strict: Range, smart: Range, kind, batch_id, delimiter: Option<Range>, terminated: bool}>` and `Vec<Batch{range, repeat: u32}>`.
- **Algorithm:**
  1. Lex with dialect flags (ALG-SQL-02) into tokens that already absorb strings, quoted identifiers, comments, dollar bodies, `q'[…]'`, `/*! */`, COPY data blocks. Track line starts.
  2. State: `depth_paren`, `block_stack: Vec<BlockKind>` (BEGIN, CASE, IF, LOOP, WHILE, REPEAT, FOR, TRY, CATCH, DECLARE-section, PACKAGE, TYPE BODY), `active_delim` (default `;`), `stmt_start`, `in_routine_header` flag, `batch_id`.
  3. For each token:
     - Skip whitespace/comments for structure, but remember the first comment run preceding a statement for `smart` range.
     - **MySQL**: if at line start (only whitespace before) and token is word `DELIMITER` (case-insensitive), the rest of the line (trimmed, up to newline) is the new `active_delim`; emit a `Delimiter` pseudo-statement (`kind=ClientCommand`, not sent to server). The active delimiter is matched as raw text, even multi-char (`$$`, `//`), only outside strings/comments.
     - **T-SQL**: if a line contains only `GO` (case-insensitive) optionally followed by whitespace and an integer and/or `-- comment`, end the current statement and the batch; record `repeat=n`; `GO` itself is not sent. `GO` inside string/comment/bracket is ignored by construction.
     - **Oracle (SQL*Plus semantics)**: a line containing only `/` (whitespace allowed) ends the current statement/PL/SQL unit regardless of `;` inside. SQL*Plus commands (`SET`, `PROMPT`, `DEFINE`, `SPOOL`, `@`, `@@`, `REM`, `EXEC`, `WHENEVER`, `COLUMN`, …) at line start form single-line client commands ending at newline (a trailing `-` continues the line).
     - Track blocks: push on `BEGIN` **only when it opens a block** — PG: inside function bodies only (body is one dollar token anyway, so top-level `BEGIN;` is a transaction statement); MySQL: `BEGIN` after `CREATE … PROCEDURE/FUNCTION/TRIGGER/EVENT` header, or after a label `lbl:`; a top-level `BEGIN` followed by `;` or `WORK` is a transaction; T-SQL: `BEGIN TRAN`/`BEGIN TRANSACTION`/`BEGIN DISTRIBUTED` are statements, not blocks, `BEGIN TRY/CATCH` and bare `BEGIN` are blocks; Oracle: `DECLARE`/`BEGIN`/`CREATE [OR REPLACE] {PROCEDURE|FUNCTION|PACKAGE [BODY]|TRIGGER|TYPE BODY}` enter PL/SQL mode where `;` never ends the unit — only `/` (or EOF, or the next `CREATE` at depth 0 heuristically) does; SQLite: `CREATE TRIGGER … BEGIN` until matching `END`. Pop on `END` (consume following `IF`/`LOOP`/`WHILE`/`REPEAT`/`CASE`/`TRY`/`CATCH`/label words). `CASE … END` pushes/pops in expressions too.
     - Parentheses: `depth_paren` ±1 (never below 0).
     - If token equals `active_delim` and `depth_paren == 0` and `block_stack` empty (or dialect does not use block tracking for that delimiter, e.g. MySQL custom delimiter overrides blocks) → end statement (`terminated=true`).
  4. At EOF: close the open statement as `terminated=false` (still executable). Empty statements (only delimiters/comments) are dropped but their delimiters retained for ranges.
  5. **Generic/SQL:2016 modes**: `Separator` → only `;` at depth 0 outside strings/comments; `Parse` → parser-driven top-level statements; `ParseOrSeparator` → parse, and wherever the parser reports an error spanning a separator, fall back to separator split inside that span.
  6. Compute `smart` range = leading comments attached (no blank line between comment and statement) + statement + delimiter.
- **Data structures:** token vector with kinds and byte ranges; statement vector sorted by start (binary-searchable); per-statement hash for incremental reuse (ALG-SQL-09).
- **Complexity:** O(n) tokens; incremental: re-split from the statement containing the edit start until token state re-synchronizes with the old split (same token boundary + same state) — typically one statement.
- **Edge cases (exhaustive list):** semicolons in strings, quoted identifiers (`"a;b"`, `` `a;b` ``, `[a;b]`), comments (`-- ;`, `/* ; */`, nested `/* /* ; */ */` in PG/T-SQL but not MySQL); `$$` vs `$tag$` vs `$1` positional parameter vs `$` inside identifiers (`a$b` Oracle/MySQL); unterminated string/comment/dollar-quote (rest of file becomes one statement; must not hang); `E'\''` escapes; MySQL backslash escapes in `'…\'…'` (and NO_BACKSLASH_ESCAPES mode); `DELIMITER` inside string or comment (ignored); `DELIMITER` with no argument; delimiter equal to `;` reset; delimiter appearing inside a statement as part of a token (`$$` inside a string); `GO` as identifier (`SELECT go FROM t`), `GO` with count, lowercase `go`, `GO;`, `GO -- comment`; T-SQL scripts with no semicolons at all (statement boundaries only by grammar — token splitter returns whole batch as one statement, parser refinement splits further); `BEGIN TRAN` vs `BEGIN` block; `BEGIN TRY`; nested BEGIN…END; `CASE` inside SQL vs PL `CASE` statement (`END CASE`); `END IF`/`END LOOP` must not pop twice; labels (`<<lbl>>` in PL/SQL, `lbl:` in MySQL); Oracle `/` after a plain SQL statement (`SELECT 1 FROM dual;` newline `/` → re-execute buffer in SQL*Plus — treat as terminator with no new statement); `/` as division operator at line start inside an expression (only a line consisting solely of `/` counts); `q'{…;…}'`; SQL*Plus `-` line continuation; PG `COPY … FROM STDIN;` followed by data lines until `\.`; psql meta-commands (`\c`, `\set`) at line start (client commands); `CREATE RULE … DO ALSO (…; …)`; PG `CREATE FUNCTION … BEGIN ATOMIC … END` (SQL-standard body, PG14+; `;` inside!); MySQL `CREATE PROCEDURE` with `BEGIN … END` using default `;` delimiter (DataGrip can parse it without DELIMITER — the splitter must use block tracking even without custom delimiter); MySQL `/*!50003 CREATE TRIGGER … */`; trailing statement without delimiter; `;;;` empty statements; CRLF and lone CR line endings; BOM at file start; Unicode line separators; extremely long single line; tokens split across a CRLF; template parameters (`${x}`) containing `;`.
- **Test cases (input → expected count / boundaries):**
  - PG: `SELECT ';'; SELECT 2` → 2. `DO $$BEGIN RAISE NOTICE 'a;b'; END$$; SELECT 1;` → 2. `CREATE FUNCTION f() RETURNS int AS $x$ SELECT 1; $x$ LANGUAGE sql; SELECT f();` → 2. `/* a /* b; */ c; */ SELECT 1;` → 1. `BEGIN; UPDATE t SET a=1; COMMIT;` → 3 (BEGIN is transaction). `CREATE FUNCTION g() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT 2; END; SELECT 3;` → 2.
  - MySQL: `DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END$$\nDELIMITER ;\nCALL p();` → client cmd, 1 procedure, client cmd, 1 call. `SELECT 1 # ; comment\n; SELECT 2;` → 2. `` SELECT `a;b` FROM t; `` → 1. `CREATE PROCEDURE p() BEGIN SELECT 1; END; SELECT 2;` (no DELIMITER) → 2.
  - T-SQL: `SELECT 1\nGO\nSELECT 2\nSELECT 3\nGO 5` → batch1{1 stmt}, batch2{repeat 5; 1 token-level statement, 2 after parser refinement}. `PRINT 'GO'` → 1, no batch break. `BEGIN TRAN; UPDATE t SET a=1; COMMIT;` → 3. `IF 1=1 BEGIN SELECT 1; SELECT 2; END; SELECT 3;` → 2.
  - Oracle: `BEGIN NULL; END;\n/\nSELECT 1 FROM dual;` → 2. `SELECT q'[a;b]' FROM dual;` → 1. `CREATE OR REPLACE PACKAGE BODY p AS PROCEDURE x IS BEGIN NULL; END; END p;\n/` → 1.
  - SQLite: `CREATE TRIGGER tr AFTER INSERT ON t BEGIN UPDATE u SET c=c+1; END; SELECT 1;` → 2.
  - Generic/Separator: `CREATE PROCEDURE p AS BEGIN SELECT 1; END;` → 2 (naive split, by design).
- **Prior art and evidence:** [Observed: `ScriptModel$*`, `SqlBatchBlock`, `MysqlDelimiterStatementImpl`, `DollarQuoted`, help `query-execution`]; sqlparser-rs (Apache-2.0) tokenizer; pgcli/`sqlparse` (BSD) splitting heuristics; psql lexer `psqlscan.l` (PostgreSQL License) for COPY and meta-commands. Black-box verification: doc 13 family SPLIT.
- **Used by:** Hacivat (editor, execution, MCP `execute_script` tool); Karagöz (server-side re-validation of statements received over P2P before execution/audit).

### ALG-SQL-02: Dialect lexer flags
- **Problem:** One lexer engine parameterized per dialect.
- **Inputs / outputs:** `DialectLexConfig` + text → tokens `{kind, range, flags}`.
- **Algorithm:** a hand-written DFA-style scanner (logos-generated core + manual handlers) consulting flags: `ident_quotes: [('"','"', DoubleEscape), ('`','`', DoubleEscape), ('[',']', DoubleClose)]`; `string_prefixes: {E: backslash, N: national, U&: unicode, X/B: binary, q': alt-quote, _charset}`; `backslash_escapes_default: bool` (MySQL true); `double_quote_is_string: bool` (MySQL w/o ANSI_QUOTES); `dollar_quotes: bool` (PG family, Snowflake `$$`); `nested_block_comments: bool` (PG, T-SQL, Db2, …); `hash_line_comment: bool` (MySQL); `dash_comment_requires_space: bool` (MySQL); `exec_comments: bool` (`/*! */`); `variables: {at: Local, atat: System, colon: Bind, amp: Substitution}`; `positional_params: {question, dollar_n, colon_n, question_n}`; `case_fold: Lower|Upper|None`; `keywords: reserved/non-reserved sets` (DataGrip keeps per-dialect Reserved/Optional keyword classes — [Observed: `PgReservedKeywords`/`PgOptionalKeywords` in prior notes]).
- **Edge cases:** `$1` vs `$tag$` (tag must start with letter/underscore, `$$` empty tag); `a$b` identifiers (Oracle/MySQL allow `$`); `@` in T-SQL vs PG operators (`@>`); `#temp` table names in T-SQL (identifier, not comment!); `::` vs `:name`; `?|`, `?&` PG JSON operators vs `?` parameter (DataGrip has to disambiguate by dialect); Unicode identifiers (Turkish letters valid in all major DBs unquoted? PG yes, MySQL yes, Oracle yes in DB charset); numeric literals `1e10`, `.5`, `0x1F`, `1_000` (PG16+).
- **Test cases:** MySQL `"a"` → string; PG `"a"` → identifier; T-SQL `#tmp` → identifier; PG `x ?| array['a']` → operator, not parameter.
- **Prior art:** sqlparser-rs `Dialect` trait (Apache-2.0); DataGrip per-dialect lexers [Observed: `_PgLexer`, `_PgDtLexer`, `PgInjectionIdentifyingLexer`].
- **Used by:** Hacivat; Karagöz (statement classification).

### ALG-SQL-03: Completion context detection
- **Problem:** Decide what kinds of items are valid at the caret in partially written SQL.
- **Inputs / outputs:** document, caret offset, dialect → `CompletionContext{kind, expected_keywords, scope_ref, qualifier_path, expected_type?, clause, statement_kind, replace_range}`.
- **Algorithm:**
  1. Find the statement containing the caret (ALG-SQL-01).
  2. Copy the statement text and insert a **dummy identifier** at the caret (IntelliJ's `IntellijIdeaRulezzz` trick) — this turns `SELECT a. FROM t` into a parseable reference.
  3. Parse the copy with error recovery; locate the dummy token and walk up: parent chain gives `clause` (SELECT list, FROM, JOIN target, ON, WHERE, GROUP BY, HAVING, ORDER BY, INSERT column list, VALUES, SET, RETURNING, CTE body, function args, type position, DDL name position).
  4. If the token before the dummy is `.`, collect the qualifier path (`schema.table.` / `alias.`) → member completion.
  5. Ask the parser for **expected tokens** at the dummy position (record the FIRST sets tried during recovery) → keyword candidates.
  6. Expected type: if parent is a binary comparison, assignment `SET c = ┃`, function argument with known signature (functions.xml), or VALUES position mapped to a target column → type for type-matching completion.
  7. Determine replace range (identifier prefix before caret + identifier chars after caret for Tab-replace).
- **Data structures:** small `enum ClauseKind`; parser hook recording expected-token sets.
- **Edge cases:** caret in comment/string (no completion except parameter names / injected language); caret after `AS` (alias name suggestions only); caret right after a keyword without space (`SELECT*`); quoted identifier prefix (`"Us┃` → complete and close quote); multi-part names with quoted parts; incomplete statements (`SELECT FROM t` with caret between) — parser must accept empty select list in recovery; `INSERT INTO t (a, ┃` — exclude already listed columns; `ORDER BY ┃` — include select aliases and ordinals; CTE body referencing later CTE; LATERAL visibility; T-SQL `TOP (┃)`; PG `::┃` → types; `CAST(x AS ┃)` → types; `CREATE TABLE t (c ┃` → types; caret inside dollar-quoted body (delegate to injected language context); Turkish characters in prefix (`şi┃`) — case-insensitive matching must use Unicode simple case folding, not Turkish locale rules, unless dialect collation demands.
- **Test cases:** `SELECT ┃ FROM users u` → columns of users (qualified per policy), functions, `*`; `SELECT u.┃ FROM users u` → only users columns; `SELECT * FROM users u JOIN ┃` → tables + JOIN smart items first; `UPDATE users SET name = ┃` → expected type text; `SELECT * FROM t WHERE id IN (SELECT ┃ FROM s)` → s columns then outer t columns (outer marked). Doc 13 family COMPL.
- **Prior art:** IntelliJ `CompletionContributor`, `CompletionUtil.DUMMY_IDENTIFIER` (Apache-2.0); [Observed: `SqlKeywordCompletionContributor$SqlCompletionState`]; postgres-language-server (MIT) completion contexts.
- **Used by:** Hacivat (editor, MCP "suggest" tool).

### ALG-SQL-04: Scope building and name resolution (layers + Script/Playground)
- **Problem:** Resolve each reference to a declaration with deterministic precedence and good diagnostics.
- **Inputs / outputs:** parsed file, resolve mode, resolution scope (data sources/schemas), catalog snapshot(s) → `ResolveResult{target, candidates, kind: Exact|Ambiguous|Unresolved, via_layer}` per reference.
- **Algorithm:**
  1. **Context per statement**: start from the resolution-scope default (data source + current DB + search_path list from session or mapping). In **Script** mode, process statements in order and update context with `USE`, `SET search_path`, `ALTER SESSION SET CURRENT_SCHEMA`, `\c` and apply DDL effects to an **overlay catalog** (create → add; drop → tombstone; alter → patch; rename → move). In **Playground** mode, each statement uses the default context and an empty overlay.
  2. **Scope tree per statement**: build nested scopes: statement → CTE scope (non-recursive CTE visible to following CTEs and the main query; recursive sees itself) → query block scope with FROM items (each with output columns: table columns, derived-table select list, function result columns/column-definition list, VALUES aliases) → nested subqueries with parent link (correlation) → SELECT-list alias scope (visible in ORDER BY; in GROUP BY/HAVING only for dialects that allow it: MySQL, PG for GROUP BY, SQLite) → routine scope (parameters, variables, cursors, labels, NEW/OLD).
  3. **Resolve unqualified column `c`**: search current query block FROM items; if found in exactly one → result; if in several → Ambiguous (unless USING/NATURAL merged); if none → walk to parent query blocks (correlated, flag "outer column"); then routine variables/parameters (or before columns, depending on dialect precedence — PL/pgSQL default is error on conflict ("variable_conflict"), T-SQL `@` vars are lexically distinct, Oracle PL/SQL prefers column → report `SqlMisleadingReference` when both exist); then SELECT aliases where allowed.
  4. **Resolve qualified `q.c`**: `q` first as alias/FROM-item name in local scopes (inner to outer), then as table in default schemas, then `schema.table`, then `db.schema.table`.
  5. **Resolve object names** (tables, routines, types): overlay → live catalog for each schema in search path order → system catalogs (minicat) → DDL data sources; routines additionally resolved by **overload** matching arg count/types (functions.xml for built-ins).
  6. Apply dialect identifier rules: unquoted → fold (lower/UPPER/none) then case-sensitive or collation-based compare; quoted → exact.
  7. Cache per statement keyed by (statement hash, context hash, catalog version).
- **Data structures:** arena of scopes with parent indices; `SymbolTable` per scope (`IndexMap<NormName, Vec<Symbol>>`); overlay catalog as persistent map (im crate) per statement index for O(1) snapshots.
- **Complexity:** O(refs × scope depth); Script mode linear in statements with persistent overlay.
- **Edge cases:** same table twice with different aliases; self-join without aliases (ambiguous); derived table without alias (dialect error, `SqlDerivedTableAlias`); `*` and `t.*` expansion as synthetic symbols (find usages through `*`); lateral references; CTE shadowing a real table; temp tables (`#t`, `CREATE TEMP TABLE`) visible in session/script only; synonyms (Oracle/Db2) → follow; views with unknown columns (not introspected) → don't report unresolved columns, report "not introspected" instead; dynamic SQL strings (not resolved unless injected); unresolved schema because not introspected (`SqlCurrentSchemaInspection`); case-folding with Turkish dotted I — unquoted `ID` in PG folds to `id` with **ASCII/invariant** lowering (PG uses locale-insensitive folding for ASCII and `tolower` for others — document per DB); MySQL `lower_case_table_names`; T-SQL case-insensitive collation; identifiers longer than max length (PG truncates at 63 bytes!).
- **Test cases:** Playground: `SET search_path TO information_schema; SELECT * FROM actor;` → actor resolves to public (default). Script: same → unresolved actor. `CREATE TABLE t(a int); SELECT a FROM t;` in Script mode → resolves against overlay even if DB lacks t. Correlated subquery `SELECT * FROM o WHERE EXISTS (SELECT 1 FROM i WHERE i.o_id = o.id)` → `o.id` outer.
- **Prior art:** [Observed: help resolve modes; `SqlFileResolveMode`, `SearchPathReceiver`, minicat files]; rust-analyzer name resolution design (MIT/Apache-2.0); postgres-language-server (MIT).
- **Used by:** Hacivat; Karagöz (optional: object-level permission checks need resolved table names — see doc 07).

### ALG-SQL-05: JOIN ON inference (FK → virtual FK rules → fuzzy names)
- **Problem:** Given existing FROM items and a newly joined (or to-be-suggested) table, produce ranked JOIN completion items with ON conditions.
- **Inputs / outputs:** `existing: Vec<FromItem{table, alias}>`, `candidate_tables` (all reachable), catalog FKs, virtual FKs (stored + rules), settings → `Vec<JoinSuggestion{target, alias, condition: Vec<(lhs_col, rhs_col)>, kind: Fk|VirtualFk|Rule|Fuzzy, direction, score}>`.
- **Algorithm:**
  1. For each existing item `E` and each candidate `T`:
     a. **Real FKs** in both directions: `E → T` (E has FK referencing T) and `T → E`. Composite FKs produce multi-column `AND` conditions in FK column order.
     b. **Stored virtual FKs** (external-data XML / Hacivat project file) same as a.
     c. **Rule-based virtual FKs**: for each rule `(column_regex, target_template)`, for each column `c` of E (and T): if `column_regex` matches `c`, substitute captures into `target_template` producing a regex for `table.column`; match against candidate T's columns → condition. (DataGrip's Rule Debugger shows exactly this generated pattern.)
     d. **Fuzzy (non-strict FK by name matching)** if enabled: build keys: for column `c` in E named like `<x>_id`/`<x>Id`/`id_<x>`, look for table T named `x`, `xs`, `x+es`, `x→ies` (singular/plural variants, case-insensitive, invariant folding) with PK `id` or `<x>_id`; also equal column names in both tables that are PK/unique on one side (e.g. `actor_id` in `actor` and `film_actor`).
  2. Dedupe by (target, column set); keep best kind (Fk > VirtualFk > Rule > Fuzzy).
  3. Choose join keyword: `JOIN` (inner) by default; [Inferred] LEFT when FK column nullable? — DataGrip inserts plain JOIN; cardinality hint shows optionality.
  4. Alias for T via ALG-SQL-06 if "Use aliases in completion for JOIN"; qualify columns with aliases (or table names) per ALG-SQL-07.
  5. Operand order: default `existing.col = target.col`? — help: "When the checkbox is cleared, the FROM table comes the first in the JOIN condition"; with "Invert order" → target first.
  6. Score: kind weight, whether E is the most recently added FROM item (prefer joining to last table), FK uniqueness (only one FK between pair → higher), name similarity; multiple FKs between same pair produce separate items labelled by columns.
  7. Postfix `.join`: apply only when exactly one suggestion exists.
- **Data structures:** FK graph adjacency `HashMap<TableId, Vec<FkEdge>>` (both directions); compiled rule regexes cached.
- **Edge cases:** self-FK (`employees.manager_id → employees.id`: suggest joining a second alias of same table, alias must differ: `e`, `e2`/`m`); multiple FKs to same table (`orders.billing_address_id`, `shipping_address_id`); composite FK with columns in different order than PK; FK to unique constraint (not PK); cross-schema FK (qualify target); FK to a table not in search path; target is a view (no FKs → only fuzzy/virtual); quoted mixed-case names; tables already joined (avoid duplicates unless self-join); existing item is a derived table/CTE (use underlying column lineage if available, else fuzzy only); case-insensitive pluralization in Turkish data models (`kullanici_id` → `kullanicilar`? — pluralization rules are English-only; allow user rule table).
- **Test cases:** orders(customer_id FK→customers.id) `SELECT * FROM orders o JOIN ┃` → first item `customers c ON c.id = o.customer_id`? (verify operand order in doc 13 family JOIN); no FK, column `visitor_id`, rule `(.*)_(?i)id` → `$1\.(?i)id` → `visitor v ON v.id = a.visitor_id`; fuzzy disabled → no item.
- **Prior art:** [Observed: `SqlJoinSmartCompletionProvider$FuzzyKey`/`$JoinKind`, `FkRuleCheckerDialog`, help `virtual-foreign-keys`, completion settings].
- **Used by:** Hacivat (completion, postfix, diagram, Related Rows in grid doc 08).

### ALG-SQL-06: Alias generation
- **Problem:** Produce short, unique, readable table aliases.
- **Inputs / outputs:** table name, existing aliases/names in the statement, custom alias dictionary, dialect reserved words → alias.
- **Algorithm:**
  1. If custom alias table has an entry for the (schema-qualified or bare) table name → candidate.
  2. Else split name into words: snake_case/kebab (`order_items` → order, items), camelCase/PascalCase (`OrderItems`), digits as separate tokens; drop common prefixes (`tbl_`, `t_`) [Inferred]; candidate = initials (`oi`); single-word name → first letter (`orders` → `o`).
  3. Lowercase with **invariant** rules (`İ`→`i̇`? No: map via Unicode simple lowercase, then strip combining dot; keep Turkish letters otherwise — `şirket` → `ş`; optionally transliterate to ASCII via setting).
  4. If candidate is a reserved/non-reserved keyword in the dialect (`as`, `on`, `or`, `in`, `is`, `by`, `do`, `to`, `if`, `of`) or collides with existing alias/table/CTE name in the scope → append increasing number (`o1`, `o2`) or extend with more letters (`ord`) [DataGrip behavior to verify: doc 13].
  5. Return alias; for "Suggest alias names", return several candidates (initials, first word, full name).
- **Edge cases:** quoted names with spaces (`"Order Details"` → `od`); names with only digits; schema-qualified input; very long names; same table joined twice; alias equals a column name (allowed but shadowing warning); case-sensitive DBs (alias case irrelevant mostly).
- **Test cases:** `film_actor` → `fa`; `actor` → `a`; second `actor` → `a1` or `a2`; `order` (reserved) → `o`; `in_stock` → `is` is keyword → `is1`/`ins`.
- **Prior art:** [Observed: settings "Automatically add aliases…", "Custom aliases", postfix `.afrom` (column aliases `ai`, `fn`, `ln`, `lu` from help example)].
- **Used by:** Hacivat.

### ALG-SQL-07: Qualification policy
- **Problem:** Decide how much to qualify an inserted name (completion, JOIN, templates, refactoring, drag-and-drop).
- **Inputs / outputs:** target object, insertion context kind, current resolve context, settings `{db, schema, table, alias: Always|OnCollisions|Never}`, `apply_in: set<ContextKind>` → inserted text.
- **Algorithm:**
  1. If context kind not in `apply_in` → insert minimal name that resolves unambiguously (equivalent to OnCollisions everywhere).
  2. For columns: if the FROM item has an alias → alias policy; else table policy. `Always` → prefix; `Never` → no prefix even if ambiguous (will raise Ambiguous inspection); `OnCollisions` → prefix only if another visible FROM item has a same-named column (or outer scope).
  3. For tables/routines: schema policy: `OnCollisions` → prefix if the name is not resolvable in current search path or resolves to a different object, or exists in more than one schema of the suggestion scope; database policy similarly vs. current database.
  4. Quote each part if required by the dialect (reserved word, case, special chars), using the code-style quotation character.
  5. Re-run resolution on the result in a scratch copy to verify it resolves to the intended target; if not, escalate one level of qualification.
- **Edge cases:** search_path with duplicates; object in `pg_catalog` (never qualify built-ins); temp schema; synonyms; inserting into a different statement than the one parsed; refactoring "Inline alias" requalification.
- **Test cases:** two tables both with `id`, OnCollisions for table → `u.id`; single table → `id`; table `public.users` with search_path public → `users`; `audit.users` also exists & suggest from all schemas → `public.users`.
- **Prior art:** [Observed: settings CC-2..6; `QualificationType`, `SqlQualifyReferenceFix`].
- **Used by:** Hacivat.

### ALG-SQL-08: Formatter pipeline
- **Problem:** Reformat SQL losslessly according to style, never breaking code.
- **Inputs / outputs:** CST, `SqlStyle` (ALG-SET-02 in doc 11), range → list of text edits.
- **Algorithm:**
  1. **Guard**: if range contains parse errors, format only error-free statements; never touch tokens inside error nodes, strings, dollar bodies (unless the injected language formatter is available and enabled), or `-- @formatter:off` regions [Inferred: platform supports formatter off/on markers].
  2. **Case pass** (token edits): keywords, built-in types/functions, custom types, identifiers, quoted identifiers, aliases → upper/lower/keep; *Use original case* → for resolved identifiers, use declaration spelling. Use invariant ASCII case mapping for keywords; identifiers only when their meaning is preserved (unquoted & case-insensitive in dialect).
  3. **Quotation pass**: quote/unquote identifiers where safe (unquoting only if result is not a keyword, matches folding rules, has only legal chars).
  4. **Normalization pass**: optional `AS` insertion/removal, comma placement — implemented as token edits.
  5. **Layout pass**: build a document tree (Wadler/Leijen) from CST nodes using clause patterns: statement → clauses (each starting with its first keyword, optionally aligned "first word of clause" right-aligned), elements with separators (comma first/last), parenthesized groups (open/close placement), JOIN chains, boolean trees (top-level AND/OR placement), CASE, DDL column lists with alignment columns (name | type | default | nullability | options), INSERT VALUES matrix (column-aligned up to 1,000 rows; thresholds for break/min increase/percentile to avoid outliers forcing huge widths).
  6. **Comment attachment**: line comments attach to the preceding token on the same line (kept at right, optionally aligned), block/standalone comments attach to following element; "Comment at first column" preserved.
  7. **Render** with line width (right margin) and "collapse when short"/"keep elements on one line if ≤ N" heuristics; emit minimal diffs (only whitespace edits + case edits) so undo and cursor positions are stable.
  8. Verify: re-lex result; token stream (ignoring whitespace and case changes for case-insensitive tokens) must equal input; otherwise abort with no edits.
- **Edge cases:** comments between keywords (`SELECT /* x */ DISTINCT`); hints `/*+ … */` must stay adjacent; MySQL `/*! */`; `--` comments at end of a line that becomes merged (must force line break); template parameters; very long string literals; T-SQL batches and `GO`; PG `$$` body formatting via injected plpgsql; Turkish-I in identifiers during upper-casing (`kimlik` → must not become `KİMLİK` unless the user wants identifiers uppercase — and then `KIMLIK` vs `KİMLİK` changes meaning in case-sensitive contexts; default: never change identifier case for quoted, and only ASCII-safe change for unquoted).
- **Test cases:** idempotency (format twice = once); token-equivalence; preset snapshots; doc 13 family FMT.
- **Prior art:** [Observed: `SqlPreFormatProcessor`, `SqlCasePreFormatter`, `SqlFormattingModelBuilder`, model pattern classes, registry]; sqruff (Apache-2.0) and sqlfluff (MIT) rules; `pretty` crate (MIT).
- **Used by:** Hacivat.

### ALG-SQL-09: Incremental reparse unit
- **Problem:** Keep parse/semantic results fresh on every keystroke for big scripts.
- **Algorithm:**
  1. On edit `[a,b) → text`, relex from the token containing `a` until a token boundary where the new lexer state equals the old one and offsets realign (+delta) — usually a few tokens; dollar-quoted bodies and comments are single tokens so edits inside them relex only that token.
  2. Re-split statements locally (ALG-SQL-01 incremental) — find the dirty statement range.
  3. Reparse only dirty statements; reuse green nodes for untouched statements (shift offsets).
  4. Lazy bodies: routine bodies parsed on demand (first highlight/resolve request).
  5. Semantic queries (salsa): `parse(stmt_id)`, `scopes(stmt_id)`, `resolve(stmt_id, context)`, `diagnostics(stmt_id)`; in Script mode `context(stmt_i)` depends on `context(stmt_{i-1})` + `ddl_effects(stmt_{i-1})` → an edit invalidates following statements' contexts only if the edited statement's effects changed (early cutoff).
- **Edge cases:** edit that opens a string/comment/dollar quote (everything after changes — must stay O(n) but cancellable); edit to `DELIMITER` line (re-split rest of file); paste of a huge script (debounce, background thread, cancellation).
- **Prior art:** IntelliJ `BlockSupportImpl`, `IReparseableElementType` (Apache-2.0); rust-analyzer incremental reparsing (MIT/Apache-2.0); tree-sitter edits (MIT).
- **Used by:** Hacivat.

### ALG-SQL-10: Dialect auto-detection for unassigned files
- **Problem:** Suggest "the best matching dialect" for a `.sql` file with no mapping (`SqlDialectInspection`).
- **Algorithm:**
  1. Cheap signals first (score each candidate dialect): distinctive tokens — backticks/`#` comments/`DELIMITER`/`AUTO_INCREMENT`/`ENGINE=` → MySQL; `$$`/`::`/`SERIAL`/`ILIKE`/`RETURNING`/`plpgsql` → PG; `GO`/`[ident]`/`@var`/`IDENTITY(`/`NVARCHAR`/`TOP n` → T-SQL; `VARCHAR2`/`NUMBER(`/`DUAL`/`/` lines/`q'[`/`PL/SQL` → Oracle; `AUTOINCREMENT`/`PRAGMA` → SQLite; ClickHouse `ENGINE = MergeTree`.
  2. Then parse the first N statements (e.g. 20 or 64 KB) with the top-3 dialect parsers; score = (error-free statements, total error count, tokens parsed).
  3. Combine; if the winner's margin is small, suggest top-2; data source of the project (if exactly one family configured) adds a prior.
  4. Show non-blocking banner: "Use <dialect>" / "Change dialect to…"; remember choice per path.
- **Edge cases:** empty file; files with only comments; mixed scripts; generated dumps (header comments mention `pg_dump`/`MySQL dump` → strong signal).
- **Test cases:** mysqldump file → MySQL; pg_dump plain → PostgreSQL; SSMS-generated script → T-SQL.
- **Prior art:** [Observed: inspection description]; [Inferred: scoring by multiple parsers (prior notes)].
- **Used by:** Hacivat.

### ALG-SQL-11: JOIN cardinality hint
- **Problem:** Show `1..n<->1` style cardinality for each JOIN.
- **Algorithm:** for join condition columns: determine if the right side columns cover a PK/unique key (→ right side "1"), same for left; referencing side multiplicity "n"; nullability of referencing columns → optional lower bound 0; join type adjusts lower bounds: LEFT keeps left rows (right side becomes `0..1`/`0..n`), RIGHT symmetric, FULL both optional. Render `left_mult<->right_mult`. Skip when no key info.
- **Test cases (from preview data shape):** inner join child→parent with NOT NULL FK → `1..n<->1`; LEFT → `1..n<->0..1`; RIGHT → `0..n<->1`; FULL → `0..n<->0..1`. [Observed: preview hints]
- **Used by:** Hacivat.

### ALG-SQL-12: INSERT column↔value mapping (inlays, inspections, highlighting)
- **Algorithm:** resolve target column list (explicit list, or all non-generated columns in table order if omitted); for each VALUES row, map i-th expression to i-th column; mismatched length → `SqlInsertValues` warning; inlay `/*col*/` before each value; highlight pair when caret on either; NULL literal into NOT NULL → warning; value into generated column → warning. For `INSERT … SELECT`, map select-list items likewise (inlays in SELECT expressions).
- **Edge cases:** DEFAULT keyword; multi-row VALUES with different lengths; `INSERT … SET` (MySQL); `ON CONFLICT`/`ON DUPLICATE KEY` sections; table not introspected.
- **Used by:** Hacivat.

---

## 5. Implications for Hacivat

1. **Parser architecture.** Lossless CST (rowan) produced by a hand-written recursive-descent/Pratt parser per *dialect family* (PG, MySQL, T-SQL, Oracle, SQLite, Generic) sharing a common node-kind vocabulary (`SELECT_STMT`, `FROM_ITEM`, `REF_EXPR`, …) — DataGrip's key design (one PSI vocabulary). Dialect-specific grammar via feature flags + small overrides. Error recovery with recovery sets (`;`, statement-start keywords, `)`), never abort. Use sqlparser-rs only as a cross-check oracle in tests and for Karagöz statement classification; pg_query.rs for PG validation of executed statements.
2. **Crates.** `sql-lexer` (ALG-SQL-02), `sql-split` (ALG-SQL-01, usable standalone by Karagöz), `sql-syntax` (CST), `sql-catalog` (layered catalogs: live snapshot, embedded system catalogs generated per major version from `pg_catalog`/`information_schema` queries — our own data, not DataGrip's minicat), `sql-sema` (salsa: scopes, resolve, types, diagnostics), `sql-ide` (completion, inlays, intentions, formatter), `sql-functions` (our own function catalog in TOML/JSON: name aliases, overloads, arg names/types, flags aggregate/window/parens/method; generated from DB catalogs where possible: `pg_proc`, MySQL docs, `sys.objects`).
3. **Inspection set for v1** (high value, low cost): Unresolved reference, Ambiguous column, VALUES cardinality, DELETE/UPDATE without WHERE, DELETE JOIN without ON, Side effects in read-only, Missing GROUP BY column, Derived table alias, Constant condition (incl. `= NULL`), Unused CTE, Identifier should be quoted, Function signature, Dialect detection. v2: data-flow ones (unreachable code, cursor state, missing RETURN).
4. **Intentions for v1**: expand `*`, qualify/unqualify, quote/unquote, introduce/inline alias, ON↔USING, IN↔OR, merge/split INSERT, subquery→CTE, preview DML rows (with Karagöz enforcing read-only policies for the preview's SELECT).
5. **Completion** with ALG-SQL-03/05/06/07; ML ranking later — start with a transparent hand-tuned weigher (prefix quality, locality (FROM tables first), type match, FK adjacency, recency/frequency from local history). Expose the same features names so a model can be trained later on local data only.
6. **Editor integration**: Tauri + CodeMirror 6 front-end talking LSP-shaped messages to the Rust language service (completion, hover, signatureHelp, diagnostics, inlayHint, codeAction, formatting, semanticTokens + custom `sql/statements`, `sql/statementAtCursor`). Same service can back a VS Code extension later.
7. **Injections**: JSON/XML/regex/dblink detection rules as data (TOML), mirroring DataGrip's pattern DSL concept (dialect + function name + arg index / operator side / cast type).
8. **Settings**: all knobs in doc 11 (§3.10–3.12) mapped to typed settings.
9. **Karagöz**: reuse `sql-split` + a statement classifier (read/write/DDL/transaction/session) to enforce permissions and audit on statements arriving P2P; never trust client-side classification alone.

## 6. i18n considerations

- Inspection names/descriptions, intention names, quick-fix labels, completion type labels ("table", "column", "function") and formatter option labels are message IDs (doc 12); long descriptions are localized Markdown files per lint (`locales/<lang>/docs/lints/<id>.md`), SQL examples stay untranslated.
- **Case conversion must be locale-invariant** for keywords and identifiers (formatter Case tab, alias generation, completion matching, quoting decisions). Turkish locale `toUpperCase("select")` → `SELECT` with dotted capital I breaks SQL. Use ASCII mapping for keywords; Unicode simple case folding for matching identifiers.
- Completion matching of Turkish identifiers (`müşteri_id`): prefix `mus` should optionally match `müş` (diacritic-insensitive matching as a secondary score), but never auto-transliterate inserted names.
- Pluralization in fuzzy JOIN (`customers` ↔ `customer_id`) is English-centric; offer a user rule table (virtual FK rules) for Turkish schemas (`musteriler` ↔ `musteri_id`).
- Error messages from the server are shown verbatim (localized server messages are possible, e.g. PG `lc_messages=tr_TR`), wrapped in our translated context.

## 7. Risks, pitfalls, open questions

- **Splitter fidelity** is user-visible and safety-critical (running the wrong range). Mitigation: doc 13 family SPLIT/CARET results become golden tests; when ambiguous, show the chooser (DataGrip's "Ask what to execute").
- **Grammar coverage cost**: DataGrip maintains ~30 parsers; Hacivat must limit v1 to PG, MySQL/MariaDB, SQLite, T-SQL with Generic fallback; ClickHouse/Oracle later.
- **Exact DataGrip precedence rules** (variables vs columns, SELECT alias visibility per dialect, alias numbering) are not observable statically → doc 13 COMPL/JOIN families.
- **Function data licensing**: DataGrip's `functions.xml` cannot be copied; we must build our own from DB catalogs/docs (PG's `pg_proc` is queryable; MySQL/T-SQL need doc-derived lists — check licenses of docs; prefer catalog introspection at runtime plus a small curated core).
- **ML ranking** requires training data; keep heuristic ranking explainable.
- **Open**: whether DataGrip's Script-mode overlay also applies DML effects (e.g. temp table created via `SELECT INTO`) — test in doc 13; how `ParseOrSeparator` decides fallback spans exactly.
