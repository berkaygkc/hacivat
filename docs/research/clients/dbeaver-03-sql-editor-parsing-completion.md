# DBeaver CE — SQL Editor: Dialects, Script Parsing, Parameters, Completion, Formatting, Execution

> Research document 3 of 5 on DBeaver Community Edition. Audience: the Hacivat team (not necessarily database experts). Every non-obvious term is explained the first time it appears. Evidence tags follow `docs/research/README.md`.

---

## 1. Scope and sources

### 1.1 What this document covers

Everything that happens between "the user types SQL into an editor tab" and "the server receives a statement":

1. **SQL dialect model** — how DBeaver describes the differences between databases (keywords, quotes, comments, delimiters, block syntax).
2. **Script parsing and statement splitting** — how a script of many statements is cut into individual statements, including procedural blocks (`BEGIN … END`), PostgreSQL dollar quoting (`$$ … $$`), MySQL `DELIMITER`, SQL Server `GO`, and "smart" blank-line handling.
3. **Parameters and variables** — `:name`, `?`, `${var}`, `{{var}}`, and DBeaver's own script commands (`@set`, `@include`, `@echo`, `@export`, `@unset`).
4. **Code completion** — the legacy heuristic engine (`SQLCompletionAnalyzer`) and the newer grammar/semantic engine (`SQLQueryCompletionAnalyzer` on top of an ANTLR4 grammar called "LSM"), how both coexist, join completion and alias generation.
5. **Syntax and semantic highlighting**.
6. **Formatting** — the built-in tokenized formatter, the compact formatter, the external-process formatter.
7. **Templates**.
8. **Execution** — `SQLQueryJob`, multiple result sets, transactions in scripts, explain plans (PostgreSQL in detail), the Query Manager (history/log).

Result-set viewing/editing is covered in `dbeaver-04-results-viewer-data-editor.md`; metadata loading in `dbeaver-02-metadata-and-ddl.md`; connections in `dbeaver-01-architecture-drivers-connections.md`.

### 1.2 Sources

| Source | Version / commit | License |
|---|---|---|
| DBeaver Community, `github.com/dbeaver/dbeaver` | shallow clone, commit `f78ac453859084a4654eac0599fb4c0514ef405b` (2026-11-11), local path `.refs/repos/dbeaver` | Apache-2.0 |
| DBeaver grammar files `plugins/org.jkiss.dbeaver.model.lsm/grammar/SQLStandardLexer.g4`, `SQLStandardParser.g4` | same commit | Apache-2.0 (header in file) |
| DBeaver unit tests under `test/` (cited as test-case sources) | same commit | Apache-2.0 |
| DataGrip 2024.3.5 editor teardown notes (Turkish), `.refs/datagrip-extract/report-editor.md` and `report-connectivity.md` | n/a | DataGrip is proprietary — used only as behavioral comparison, no code |

All file paths below are relative to `.refs/repos/dbeaver/`. Line numbers refer to the commit above. Because DBeaver is Apache-2.0, code reuse with attribution would be legally possible, but per the README we describe algorithms in prose/pseudocode and name the exact file so an implementer can decide.

### 1.3 Package map (where the code lives)

| Bundle / package | Role |
|---|---|
| `plugins/org.jkiss.dbeaver.model/src/org/jkiss/dbeaver/model/sql/` | Core interfaces: `SQLDialect`, `SQLSyntaxManager`, `SQLQuery`, `SQLQueryParameter`, `SQLUtils`, `SQLConstants` |
| `plugins/org.jkiss.dbeaver.model/src/org/jkiss/dbeaver/model/impl/sql/` | `AbstractSQLDialect`, `BasicSQLDialect` |
| `plugins/org.jkiss.dbeaver.model.jdbc/.../impl/jdbc/JDBCSQLDialect.java` | Dialect that learns from JDBC driver metadata |
| `plugins/org.jkiss.dbeaver.model.sql/src/org/jkiss/dbeaver/model/text/parser/` | Generic rule-based tokenizer framework (`TPRuleBasedScanner`, `TPRule`, `TPToken`) — a UI-free clone of Eclipse JFace text scanning |
| `.../model/sql/parser/` | `SQLScriptParser` (splitter), `SQLRuleManager` (builds token rules per dialect), `SQLWordPartDetector`, `DefaultSQLQueryParameterParser` |
| `.../model/sql/parser/rules/` | Individual token rules: dollar quote, delimiter, batch delimiter (`GO`), `DELIMITER` redefinition, parameters, variables, words |
| `.../model/sql/parser/tokens/predicates/` | Token-sequence predicates (trie-based) for dialect-specific block detection (Oracle, BigQuery…) |
| `.../model/sql/completion/` | Legacy completion engine `SQLCompletionAnalyzer` |
| `.../model/sql/semantics/` and `.../semantics/completion/` | New semantic model (`SQLQueryModelRecognizer`), document syntax context, new completion engine |
| `plugins/org.jkiss.dbeaver.model.lsm/` | "LSM" = DBeaver's ANTLR4 grammar + inspection utilities (`LSMInspections`) |
| `.../model/sql/format/` | Formatters (tokenized, compact, external) |
| `.../model/sql/commands/` | `@set`, `@unset`, `@echo`, `@export` command handlers |
| `plugins/org.jkiss.dbeaver.ui.editors.sql/` | The Eclipse editor: completion processor, background parsing job, `SQLQueryJob` (execution), templates |
| `plugins/org.jkiss.dbeaver.ext.<db>/` | Per-database dialects (`PostgreDialect`, `MySQLDialect`, `SQLServerDialectBase`, `OracleSQLDialect`…) |

---

## 2. Executive summary

1. **Two parsing worlds coexist.** A fast, regex-like *rule-based tokenizer* (ported from Eclipse JFace) drives highlighting, partitioning and statement splitting; a full *ANTLR4 grammar* ("LSM", `SQLStandardParser.g4`, 774 lines) drives the semantic model, semantic highlighting and the new completion engine. [Observed: `model/text/parser/TPRuleBasedScanner.java`; `model.lsm/grammar/*.g4`]
2. **A dialect is a big bag of booleans and string arrays** (≈80 methods on `SQLDialect`): quote pairs, comment markers, delimiters, block begin/end words, "block header" words (`DECLARE`), keywords/functions/types, case rules. Generic JDBC databases fill part of it from `DatabaseMetaData` at connect time. [Observed: `model/sql/SQLDialect.java` L63–606; `JDBCSQLDialect.java` L86–228, L408–437]
3. **Statement splitting is a single forward token loop** with a stack of "blocks" (brackets, `BEGIN…END`, toggles such as `$$`, header blocks such as `DECLARE`). A delimiter ends a statement only when the block stack is empty. Dozens of dialect hacks are layered on top (`END CASE`, `BEGIN;` is a transaction, `CASE…END` is an expression, Oracle predicates). [Observed: `SQLScriptParser.java` L110–498]
4. **"Smart" delimiter mode** joins fragments split by blank lines unless the next fragment starts with a statement-start keyword (computed from the ANTLR grammar's predicted first tokens plus dialect keyword lists). Default preference, however, is `BLANK_LINE_AND_SEPARATOR` ("blank line is a delimiter"). [Observed: `ModelPreferences.java` L78–126, L334; `SQLScriptParser.java` L996–1288]
5. **Dialect-specific splitting rules**: PostgreSQL `$$`/`$tag$` is a *block toggle* by default (not a string), configurable per connection; MySQL `DELIMITER xx` must start a line and changes the delimiter rule live; SQL Server `GO [n]` is a line-isolated *batch delimiter* (count `0` and trailing text rejected); Oracle uses token-sequence predicates for `CREATE PACKAGE BODY`, `CREATE FUNCTION … AS/IS`, `IF` blocks. [Observed: `PostgreDialect.java` L1159–1177; `SQLDelimiterSetRule.java`; `TransactSQLBatchDelimiterRule.java`; `OracleSQLDialect.java` L641–700]
6. **Oracle `/` (SQL*Plus slash) is not built-in in CE**; it only appears in tests as a user-configured extra batch delimiter. [Observed: absence in `ext.oracle`; `SQLScriptParserTest.java` L1042–1062]
7. **Parameters**: `:name` (prefix configurable, plus dialect prefixes, e.g. `$` for PostgreSQL), anonymous `?` (off by default), variables `${var}` / `{{var}}` (also found inside strings and comments). Parameters are *ignored in DDL and inside `$$` bodies* unless "parameters in embedded code" is enabled. [Observed: `DefaultSQLQueryParameterParser.java` L60–255; `ModelPreferences.java` L387–393]
8. **Parameter values are substituted textually** into the SQL string (no escaping) unless a provider marks them "native binding". This is simple but injection-prone and type-unaware. [Observed: `SQLUtils.fillQueryParameters` L1446–1461; `SQLScriptContext.java` L299–353]
9. **Script commands** (`@set x = 1`, `@@set … @@` multi-line, `@include file`, `@echo`, `@export`, `@unset`, `@ai`) are an extension point (`org.jkiss.dbeaver.sqlCommand`), recognized by token rules and returned by the splitter as `SQLControlCommand` elements. [Observed: `model.sql/plugin.xml` L71–76; `SQLRuleManager.java` L140–155]
10. **Legacy completion** works by scanning *backwards* from the cursor for the previous keyword (`FROM`, `JOIN`, `WHERE`, `ON`…) and classifies the request as TABLE / JOIN / COLUMN / EXEC. Aliases are resolved by parsing the active query with the ANTLR grammar and collecting `tableName`/`correlationName` pairs. [Observed: `SQLCompletionAnalyzer.java` L118–466; `TableReferencesAnalyzerImpl.java`]
11. **New completion (default since the `NEW` mode default)** runs ATN (ANTLR automaton) prediction at the cursor position to know which grammar rules are reachable (table name? column reference? join condition?) and resolves names against a scoped semantic model of the query (subqueries, CTEs, joins, lateral, USING, correlated scopes). [Observed: `SQLCompletionProcessor.java` L176–233; `LSMInspections.java` L192–545; `SQLQueryCompletionContext.java`]
12. **New engine is only used when** mode allows it, *advanced highlighting* and *read metadata for semantic analysis* are both on, and the dialect extends `BasicSQLDialect`; otherwise the legacy engine runs. `COMBINED` mode runs both. [Observed: `SQLCompletionProcessor.java` L178–183]
13. **Join completion**: legacy proposes one `ON` condition derived from a foreign key (either direction), falling back to "left table's best unique key columns exist with the same names in the right table"; new engine proposes `a.col = b.col` items for every single-column FK pair between the two join sides (composite FKs are a TODO). [Observed: `SQLUtils.generateTableJoin` L1295–1400; `SQLQueryCompletionContext.java` L1062–1102, L2116–2122]
14. **Alias generation**: initials of name parts (`order_items` → `oi`, `OrderItems` → `oi`), fallback `t`, numbered on collision (`t2`, `oi2`), never a keyword, never equal to the name; mode `NONE`/`PLAIN`/`EXTENDED` (`AS`). [Observed: `SQLUtils.generateEntityAlias` L969–1013; `SQLTableAliasInsertMode.java`]
15. **Background semantic parsing** re-parses only the visible part of the document, debounced 250 ms (immediately after typing a `.`), with per-statement script items stored in an offset-keyed tree that shifts on edits. Files > 2 MB disable rule-based features. [Observed: `SQLBackgroundParsingJob.java` L57, L577–622, L658–800; `SQLDocumentSyntaxContext.applyDelta` L199–278; `SQLEditorBase.java` L117]
16. **The default formatter is a flat token-list rewriter**, not a tree formatter: case transform → merge two-word keywords → indent by keyword rules → spacing rules. It has only a handful of options (keyword case, indent string, comma placement, break before close bracket, blank-line→delimiter) and ships an "external command" formatter with a 2 s timeout. [Observed: `SQLFormatterTokenized.java` L51–117; `IndentFormatter.java`; `SQLFormatterExternal.java`]
17. **Execution** runs each script element in a single session: fill parameters → optional confirmation for non-SELECT on "confirm" connection types → execute → loop over multiple result sets (asks after 20 result tabs) and update counts (max 1000 update-count reads) → commit/rollback according to script commit type and error-handling preferences. [Observed: `SQLQueryJob.java` L213–373, L487–866; `SQLEditorPreferencesInitializer.java` L47–54, L140]
18. **PostgreSQL explain** uses `EXPLAIN (FORMAT XML, …)` for modern servers (text parsing for old ones) and always runs inside a transaction that is rolled back — so `EXPLAIN ANALYZE` of DML does not persist changes. [Observed: `PostgreExecutionPlan.java` L102–229]
19. **Query Manager** keeps an in-memory ring of the last 10 000 events and optionally writes daily `dbeaver_sql_<date>.log` files with retention by days. [Observed: `QMMCollectorImpl.java` L49, L261; `QMLogFileWriter.java` L46–110]
20. **Turkish-locale hazard**: several keyword/variable comparisons use `toUpperCase()`/`toLowerCase()` *without* `Locale.ENGLISH` (e.g. `SQLWordRule`, `SQLScriptParser` L1215, `SQLScriptContext` L459), which breaks for `i/I` under a Turkish JVM locale. Hacivat must use ASCII-only case folding for SQL keywords. [Observed: those lines; consequence Inferred]

---

## 3. Detailed findings

### 3.1 The dialect model

**What a "dialect" is.** Every database speaks a slightly different SQL. A *dialect* object answers questions like "which character quotes identifiers?" (`"` in PostgreSQL, `` ` `` in MySQL, `[ ]` in SQL Server), "which words are keywords?", "what ends a statement?".

**Interface.** `plugins/org.jkiss.dbeaver.model/src/org/jkiss/dbeaver/model/sql/SQLDialect.java` (606 lines) declares the full contract. The groups most relevant to the editor [Observed: method list at L63–606]:

| Group | Methods (selection) | Meaning |
|---|---|---|
| Identity | `getDialectId()`, `getDialectName()` | e.g. `postgresql` |
| Lexical | `getIdentifierQuoteStrings()` (pairs), `getStringQuoteStrings()`, `getStringEscapeCharacter()`, `getSingleLineComments()`, `getMultiLineComments()`, `supportsNestedComments()`, `isEscapeBackslash()` | Used by the tokenizer rules |
| Words | `getReservedWords()`, `getFunctions()`, `getDataTypes()`, `getKeywordType(word)`, `getQueryKeywords()`, `getDMLKeywords()`, `getDDLKeywords()`, `getExecuteKeywords()`, `getCompletionStatementKeywords()` | Highlighting + completion + statement classification |
| Script structure | `getScriptDelimiters()`, `getBatchDelimiters()`, `getScriptDelimiterRedefiner()`, `getBlockBoundStrings()` (pairs like `{BEGIN, END}`), `getBlockHeaderStrings()` (`DECLARE`, `PACKAGE`), `getInnerBlockPrefixes()` (`AS`, `IS`), `isDelimiterAfterQuery()`, `isDelimiterAfterBlock()`, `needsDelimiterFor(first,last)`, `isStripCommentsBeforeBlocks()`, `getSkipTokenPredicates()`, `getScriptVariableScope()` | Used by `SQLScriptParser` |
| Names | `storesUnquotedCase()`, `storesQuotedCase()`, `getQuotedIdentifier()`, `getUnquotedIdentifier()`, `validIdentifierStart/Part()`, `getCatalogSeparator()`, `getStructSeparator()`, `isCatalogAtStart()` | Quoting and case normalization |
| Parameters | `getParametersPrefixes()` | Extra named-parameter prefixes |
| Capabilities | `supportsAliasInSelect()`, `supportsAliasInUpdate()`, `supportsSubqueries()`, `supportsQualifiedColumnNames()`, `getDialectFeatures()` | Completion and SQL generation |

**Class hierarchy.**

```
SQLDialect (interface)
└─ AbstractSQLDialect          keyword tables (TreeMap by upper-case word), defaults
   └─ BasicSQLDialect          SQL:2003 reserved words + functions, default BEGIN/END block,
      │                        TABLE_KEYWORDS / COLUMN_KEYWORDS for completion
      └─ JDBCSQLDialect        learns quote string, case rules, separators, keywords,
         │                     functions from java.sql.DatabaseMetaData at connect time
         ├─ PostgreDialect, MySQLDialect, SQLServerDialectBase, OracleSQLDialect, …
         └─ GenericSQLDialect  fallback for "any JDBC driver"
```

- `AbstractSQLDialect` stores keywords in a `TreeMap<String, KeywordHolder>` keyed by `word.toUpperCase(Locale.ENGLISH)` plus concurrent sorted maps for reserved words, functions, types, "table query words" and "column query words". [Observed: `impl/sql/AbstractSQLDialect.java` L71–91, L243–246]
- `BasicSQLDialect.loadStandardKeywords()` adds `SQL2003_RESERVED_KEYWORDS`, `SQL_EX_KEYWORDS`, `SQL2003_FUNCTIONS`, and registers `TABLE_KEYWORDS` = `FROM, INSERT, UPDATE, DELETE, INTO, TABLE, VIEW, JOIN, TRUNCATE, MERGE` and `COLUMN_KEYWORDS` = `SELECT, WHERE, SET, ON, AND, OR, BY, HAVING`. These two lists are the backbone of the *legacy* completion's context guess. [Observed: `BasicSQLDialect.java` L347–358; `SQLConstants.java` L112–134]
- Default blocks: `BasicSQLDialect` uses `{BEGIN, END}`; `DML_KEYWORDS = INSERT, DELETE, UPDATE, MERGE, UPSERT, TRUNCATE`; non-transactional keywords `SELECT, EXPLAIN, DESCRIBE, DESC, USE, SET, COMMIT, ROLLBACK`. [Observed: `BasicSQLDialect.java` L36–74]
- `JDBCSQLDialect.initDriverSettings(...)` calls `getIdentifierQuoteString()`, `getSQLStateType()`, the `supportsSubqueries*` family, `supportsMixedCase*Identifiers()`, `stores{Upper,Lower}Case[Quoted]Identifiers()`, `getSearchStringEscape()`, `getCatalogSeparator()`, `supports{Catalogs,Schemas}In*()`, `getExtraNameCharacters()`, `isCatalogAtStart()`; every call is wrapped in `try/catch Throwable` with a fallback because drivers are unreliable. Then `loadDriverKeywords` adds `getSQLKeywords()` and all `get*Functions()` lists, removing functions that clash with keywords. [Observed: `JDBCSQLDialect.java` L86–228, L408–437]

**Per-database specifics (examples).**

| Dialect | Notable settings | Evidence |
|---|---|---|
| PostgreSQL | `getBlockBoundStrings()` returns an **empty** array with the comment "In PgSQL there are no blocks. DO $$ ... $$ queries are processed as strings"; parameter prefix `$`; nested comments on; string quotes only `'`; adds ~25 function groups (admin, aggregate, JSON, PostGIS, window, …); removes `LENGTH, JSON, TEXT, FORMAT, WORK` from keywords; forces unquoted case LOWER (Redshift driver reports it wrong, #12723); `extendRules()` adds `SQLDollarQuoteRule` + `PostgreEscapeStringRule` (E'…') | `ext.postgresql/.../PostgreDialect.java` L133–138, L881–967, L1005–1018, L1090–1122, L1159–1177 |
| MySQL | `getScriptDelimiterRedefiner()` = `DELIMITER`; no anonymous blocks (`getBlockBoundStrings()` = null); backslash escapes; single-line comments `-- `, `--\t`, `#` (note: `--` must be followed by space/tab, matching MySQL's own rule); script variable scope SESSION; `SOURCE file` line command mapped to `mysql.source` include handler | `ext.mysql/.../MySQLDialect.java` L106, L300–316, L383–385; `ext.mysql/.../sql/MySQLDialectRules.java` L30–43 |
| SQL Server | script delimiters `;` and `GO`; batch delimiters `GO`; `BEGIN/END` blocks; delimiter is kept after each query (`isDelimiterAfterQuery()` true for SQL Server, false for Sybase); `MERGE` always needs its trailing `;` (`needsDelimiterFor`); `@` is deliberately *not* a parameter prefix (#5674); variable scope BATCH; multi-word rules `BEGIN TRAN[SACTION]`, `BEGIN DISTRIBUTED TRAN[SACTION]` so they are keywords, not block begins; `SQLVariableRule` for `@var` | `ext.mssql/.../SQLServerDialectBase.java` L51–53, L119–195, L372–389 |
| Oracle | blocks `{BEGIN,END}`, `{LOOP,END LOOP}`, `{CASE,END CASE}`; headers `DECLARE`, `PACKAGE`; inner-block prefixes `AS`, `IS`; `isDelimiterAfterBlock()` true (PL/SQL blocks keep their `;`); token-sequence predicates (§3.2.6) | `ext.oracle/.../OracleSQLDialect.java` L72–86, L394–407, L467–470, L641–700 |

**Dialect registration.** Dialects are also declared through the `org.jkiss.dbeaver.sqlDialect` extension point with properties (`keywords`, `ddlKeywords`, `dmlKeywords`, `execKeywords`, `types`, `functions`). [Observed: `model.sql/plugin.xml` L78–88]

**Dialect "features" for the semantic engine.** `SQLDialectFeatures` carries knobs only the new engine reads: type-modifier keywords, signature modifiers (`VARIADIC` for PG), where projection aliases are visible (`GROUP BY`, `ORDER BY` for PG), procedural variable shadowing policy, procedural syntax family, trailing `SELECT … INTO` support, global pseudo-columns (`current_date`, `current_user`, …). [Observed: `model/sql/SQLDialectFeatures.java` L36–90; `PostgreDialect.java` L56–122]

**Syntax manager.** `SQLSyntaxManager` merges dialect + user preferences into the effective settings the tokenizer uses [Observed: `model/sql/SQLSyntaxManager.java` L148–213]:
- statement delimiters = dialect's script delimiters (lower-cased) unless "ignore native delimiter" is set, plus user-configured extra delimiters (default `;`), split on space/tab/comma;
- batch delimiters = dialect batch delimiters that are also statement delimiters;
- delimiter mode (§3.2.4); parameters enabled (default **true**), anonymous `?` (default **false**), mark `?`, named prefix (default `:`) + dialect prefixes, control command prefix (default `@`), variables enabled (default **true**). [Observed: `ModelPreferences.java` L332–335, L387–393; `SQLConstants.java` L31, L690–695]

### 3.2 Script parsing and statement splitting

#### 3.2.1 Vocabulary

- **Token**: smallest meaningful piece of text (keyword, identifier, string literal, comment, delimiter…).
- **Delimiter**: the text that ends a statement, usually `;`.
- **Batch delimiter**: a line that splits a script into batches that are sent separately (`GO` in SQL Server tools). It is *not* SQL — the server never sees it.
- **Block**: a region in which delimiters do not end the statement, e.g. a procedure body `BEGIN … ; … END`.
- **Toggle**: a token that both opens and closes a block, e.g. `$$`.
- **Header block**: a statement prefix like `DECLARE` that starts a block without an explicit `BEGIN`.
- **Partition**: a coarse classification of document regions (code / string / quoted identifier / comment) used by Eclipse to avoid scanning inside strings and comments.

#### 3.2.2 The tokenizer framework

`TPRuleBasedScanner.nextToken()` tries every rule in order; the first rule that returns a defined token wins; if no rule matches, one character is consumed and returned as the default token. Rules must restore the scanner position when they fail (the scanner logs and fixes "rule inconsistency" otherwise). [Observed: `model/text/parser/TPRuleBasedScanner.java` L200–223]

`SQLRuleManager.loadRules(container, minimalRules)` builds the ordered rule list per dialect. **Order matters**, it is [Observed: `parser/SQLRuleManager.java` L104–321]:

1. Dialect `INITIAL` rules (e.g. PostgreSQL dollar-quote rule, SQL Server `GO` rule). Provided batch-delimiter rules are set aside for later.
2. Dialect `CONTROL` rules (e.g. MySQL `SOURCE`).
3. (non-minimal) Control command rules for every registered command: `@cmd …` (single line) and `@@cmd … @@` (multi-line).
4. (non-minimal) Variable rule `${…}`/`{{…}}` **before** parameter rules (#18354), then one `ScriptParameterRule` per named prefix.
5. Identifier quote rules (`MultiLineRule(open, close, quoted, escapeChar, breaksOnEOF=true)`) — always including a `"` rule; string quote rules. `breaksOnEOF` means an unterminated quote is accepted up to end of text (#11773) so typing an opening quote does not destroy highlighting of everything after.
6. Dialect `QUOTES` rules.
7. Single-line comment rules (a comment marker starting with `^` means "only at line start" → `LineCommentRule`); multi-line comment rule (nested variant if `supportsNestedComments()`).
8. (non-minimal) whitespace rule, number rule.
9. Batch delimiter rules (dialect-provided ones that match configured batch delimiters, plus a generic fallback `SQLBatchDelimiterRule` for any unclaimed one).
10. Statement delimiter rule `SQLDelimiterRule` for all non-batch delimiters.
11. `SQLDelimiterSetRule` (`DELIMITER x`) inserted at position **0** if the dialect has a redefiner that is not itself a delimiter.
12. Dialect `KEYWORDS` rules.
13. (non-minimal) `SQLWordRule`: reserved words → keyword token (or function token for `FUNCTION`/`OTHER` types, #8710), types → type token, functions (function token only when followed by `(` or not otherwise a keyword), block header strings → `T_BLOCK_HEADER`, block bound strings → `T_BLOCK_BEGIN` / `T_BLOCK_END`.
14. Dialect `FINAL` rules (e.g. SQL Server `@variable` rule).

Token types are an enum (`SQLTokenType`): `T_KEYWORD, T_STRING, T_QUOTED, T_TYPE, T_NUMBER, T_TABLE, T_TABLE_ALIAS, T_COLUMN, T_COLUMN_DERIVED, T_SCHEMA, T_COMPOSITE_FIELD, T_SQL_VARIABLE, T_FUNCTION, T_SEMANTIC_ERROR, T_UNKNOWN, T_BLOCK_BEGIN, T_BLOCK_END, T_BLOCK_TOGGLE, T_BLOCK_HEADER, T_COMMENT, T_CONTROL, T_DELIMITER, T_SET_DELIMITER, T_PARAMETER, T_VARIABLE, T_OTHER`. The semantic ones (`T_TABLE`, `T_COLUMN`…) are only produced by the semantic layer for highlighting. [Observed: `parser/tokens/SQLTokenType.java` L25–55]

Word rule detail: a "word" may contain `$` (so PG identifiers like `a$b` stay whole), but `$$` inside a word stops the word; a word also stops if the following characters spell a multi-character delimiter. [Observed: `rules/SQLWordRule.java` L70–161]

#### 3.2.3 The splitter: `SQLScriptParser.parseQueryImpl`

One call extracts **one** element starting at `startPos`; `extractScriptQueries` calls it repeatedly. Its state [Observed: `parser/SQLScriptParser.java` L110–498]:

| Variable | Meaning |
|---|---|
| `statementStart` | start offset of the current statement |
| `hasValuableTokens` | have we seen anything other than whitespace/comments/delimiters? |
| `curBlock` (linked stack of `ScriptBlockInfo{parent, togglePattern, isHeader, isPredicateHeaderBlock, beginToken}`) | open blocks |
| `hasBlocks` | did this statement contain any block (affects keeping the trailing `;`) |
| `prevNotEmptyTokenType` | type of the previous non-whitespace token |
| `firstKeyword`, `lastKeyword` | first/last word-like tokens |
| `lastTokenLineFeeds` | newline count at the end of a preceding line comment (so `-- c\n\n` counts as a blank line) |
| `predicateEvaluator` | dialect token-sequence predicate engine (§3.2.6) |
| `hasVariableKeyword` | statement contains `DECLARE` or starts with `SET` (for variable tracking) |

Per token (simplified, full spec in ALG-DBV-SQL-01/02):

1. Feed non-whitespace, non-comment token text to the predicate evaluator.
2. `isDelimiter` = token is `T_DELIMITER`, or (newline is a configured delimiter and the whitespace contains `\n`), or (blank-line mode and whitespace + trailing comment newlines ≥ 2).
3. Single-character `(`, `{`, `[` push an anonymous block; `)`, `}`, `]` pop.
4. `T_BLOCK_BEGIN` right after `T_BLOCK_END` is demoted to `T_UNKNOWN` (handles `END LOOP`, `END CASE`, `END IF` when `LOOP/CASE/IF` are also block-begin words).
5. `T_DELIMITER` right after `T_BLOCK_BEGIN` pops the block (`BEGIN;` is a transaction start, #7821).
6. If the dialect strips comments before blocks (SAP HANA) and a `T_BLOCK_HEADER` follows a comment, the statement start moves to the header.
7. `T_BLOCK_HEADER` pushes a header block. `T_BLOCK_TOGGLE` pops if it equals the top toggle's text, else pushes a toggle block (`$a$ … $a$` nesting with different tags works). `T_BLOCK_BEGIN` first drops a header block on top unless the previous keyword is an inner-block prefix (`AS`/`IS`) — because `DECLARE … BEGIN … END` has no separate terminator for `DECLARE` — then pushes a regular block remembering its begin word. `T_BLOCK_END` pops (marks if it closed a `CASE`, issue 40779).
8. A delimiter while any block is open is ignored (`continue`).
9. `T_SET_DELIMITER` and `T_CONTROL` are treated as delimiters and, if nothing valuable precedes them (or the cursor is inside them), as control commands (pro#3935).
10. Predicate evaluation may force BEGIN_BLOCK / END_BLOCK / BLOCK_HEADER / SKIP_SUFFIX_TERM (do not end statement here).
11. Batch delimiter with nothing before it can be returned as its own `SQLBatchDelimiterElement` (structural mode).
12. When a statement ends: trim whitespace, skip empty statements, decide whether to keep the trailing `;` (keepDelimiters, or blocks + `isDelimiterAfterQuery`, or `needsDelimiterAfterBlock`), normalise line feeds, return `SQLQuery` (or a `SQLControlCommand` if the last matched predicate was CAPTURE_COMMAND).

`needsDelimiterAfterBlock(first, last)`: true if `dialect.needsDelimiterFor` (SQL Server `MERGE`); else, only for dialects with `isDelimiterAfterBlock()` (Oracle), true when the statement's last keyword is `END` and its first keyword is a block begin word, a block header word or a DDL keyword (`CREATE`…). [Observed: `SQLScriptParser.java` L528–559]

#### 3.2.4 Delimiter modes and "smart" expansion

`SQLScriptStatementDelimiterMode` [Observed: `ModelPreferences.java` L78–126]:

| Mode | `useBlankLine` | `useSmart` | UI title |
|---|---|---|---|
| `BLANK_LINE_AND_SEPARATOR` (**default**, L334) | true | false | "Always" |
| `ONLY_SEPARATOR` | false | false | "Never" |
| `SMART` | true | true | "Smart" |

In script mode blank lines are delimiters only when `useSmart` is on; when executing the single statement under the cursor, they are delimiters whenever `useBlankLine` is on (L134–136).

**Smart expansion** (`ScriptElementContinuationDetector`, L1146–1288): after splitting, a fragment that does **not** "start properly" is glued to the previous fragment if that one did not end with an explicit delimiter. "Starts properly" = its first non-hidden ANTLR token is in the set of tokens the grammar predicts at the start of a statement (`LSMInspections.prepareOffquerySyntaxInspection().predictedTokenIds()`), or its first word (upper-cased) is in a per-dialect set: block headers, block-begin words, commit/rollback keywords, non-transactional keywords, execute keywords, `@cmd`/`@@cmd`, query/DML/DDL keywords. The set is cached per dialect in a `WeakHashMap`. [Observed: L1146–1218]

Cursor variant (`tryPrepareExtendedElement`): walk *backwards* over fragments that do not start properly and do not end with `;`, then *forwards* over following fragments that do not start properly — e.g. `UPDATE t SET a=1 <blank line> WHERE id=2` becomes one statement in smart mode. [Observed: L1220–1258; test `parseFromCursorPositionUpdateWithWhereAndBlankLineInBetween`, `SQLScriptParserTest.java` L852–880]

#### 3.2.5 Extracting the statement at the cursor

`extractQueryAtPosImpl(context, pos)` [Observed: `SQLScriptParser.java` L581–727]:

1. If the cursor sits in a trailing `--` comment of a line that has code before the comment, move the cursor to the last letter before the comment.
2. If inside a multi-line comment, move forward until outside.
3. In blank-line mode: if the cursor line is empty, use the previous line; if that is empty too → no statement.
4. Find a start position: walk lines upwards until a blank line (in default partition) — on the cursor line, if a non-letter delimiter occurs before the cursor and there is non-whitespace text between it and the cursor, start right after the delimiter.
5. If a delimiter on the cursor line has only whitespace between it and the cursor, move the cursor *before* that delimiter (so "cursor just after `;`" executes the statement before it).
6. Call `parseQueryImpl(start, docEnd, cursor, scriptMode=false)` which returns the first statement whose end is ≥ cursor.

`extractActiveQuery` (execute selection): if text is selected, the selection is executed *as-is* (after optionally trimming a trailing delimiter, default true, `QUERY_REMOVE_TRAILING_DELIMITER`), unless the selection parses as a control command; otherwise the statement at cursor. Parameters are then parsed. Multiple selection regions are joined with line separators. [Observed: L844–897; `ModelPreferences.java` L335]

`extractNextQuery` (navigate to next/previous statement): skip comments/whitespace/delimiter chars forward, or skip comments back to the previous letter, then extract at that position. [Observed: L738–837]

#### 3.2.6 Token-sequence predicates (dialect-specific block recognition)

Some constructs cannot be recognised by single words. Oracle example: `CREATE OR REPLACE FUNCTION f RETURN NUMBER IS … BEGIN … END;` — here `IS` must open a header block. DBeaver lets a dialect declare **conditions** `(actionKind, prefixPattern, suffixPattern)` built with a small combinator API (`sequence`, `optional`, `alternative`, `not`, token-type matchers). [Observed: `OracleSQLDialect.java` L641–700]

Oracle's set:
- `BEGIN_BLOCK` on prefix `CREATE [OR REPLACE] [EDITIONABLE|NONEDITIONABLE] PACKAGE BODY`;
- `SKIP_SUFFIX_TERM` for `CREATE … FUNCTION|PROCEDURE` when the suffix is e.g. `RETURN <type> ;`, `) ;`, `deterministic ;`… (meaning: a `;` here is *not* the end — it is a forward declaration context);
- `BEGIN_BLOCK` when the suffix is `IF` not preceded by `END` and not followed by `EXISTS`;
- `BLOCK_HEADER` for `CREATE … FUNCTION|PROCEDURE … AS|IS`;
- on Oracle ≥ 12.1: `SKIP_SUFFIX_TERM` for statements starting with `WITH` whose suffix is `END ;` (WITH FUNCTION clause).

Engine: `SQLTokenPredicateEvaluator` keeps the statement prefix (up to the longest prefix pattern) and a sliding suffix window; prefix matching walks a **trie** incrementally per token, suffix matching uses a trie over the reversed window; a predicate fires when both prefix and suffix match; more than one simultaneous match logs "Ambiguous token predicates match" and fires nothing. [Observed: `parser/tokens/predicates/SQLTokenPredicateEvaluator.java` (whole file, 179 lines); action kinds in `model/sql/parser/SQLParserActionKind.java`]. Predicates are compiled once per data source using the dialect's own rule manager (`JDBCSQLDialect.makeDialectSkipTokenPredicates`, L392–399) and can be disabled with system property `dbeaver.debug.sql.disable-skip-token-evaluation` (L63–68).

#### 3.2.7 Dialect delimiters in detail

- **PostgreSQL `$$` / `$tag$`** — `SQLDollarQuoteRule(partitionRule, allowNamedQuotes, fullyConsumeNamed, fullyConsumeUnnamed)`. For the *partition* scanner it always consumes the whole quoted body as a string region. For the token scanner it either consumes the whole body as `T_STRING` (if the per-connection options `postgresql.dd.tag.string` / `postgresql.dd.plain.string` are on; both default **false**) or returns a `SQLBlockToggleToken` (`T_BLOCK_TOGGLE`) for just the `$$`/`$tag$` — so the body is tokenized normally (useful for highlighting PL/pgSQL), and the splitter treats the region as a block. Tag syntax: `$` + letter/underscore + letters/digits/underscore + `$`; `$1` is *not* a tag (positional parameter). When consuming a body, a different tag inside (`$b$` inside `$a$`) is skipped. Unterminated body → rule fails and rewinds. [Observed: `rules/SQLDollarQuoteRule.java` L43–157; `PostgreDialect.java` L1159–1177; `PostgreDataSourceProvider.java` L56–57]
- **MySQL `DELIMITER`** — `SQLDelimiterSetRule`: must start at line start; word match case-insensitive; must be followed by whitespace (or end of line = reset to default); the new delimiter is the first whitespace-free run until end of line. In *evaluation mode* (when actually splitting a script, `TPEvalScanner.isEvalMode()`) it calls `SQLDelimiterRule.changeDelimiter(new)`, which (a) resets to original delimiters on empty, (b) strips a trailing original delimiter (`DELIMITER $$;` → `$$`) unless that would cut a repeated char (`;;` stays `;;`), (c) stores it upper-cased. The scanner state therefore changes while splitting, which is why `extractScriptQueries` wraps the loop in `startScriptEvaluation()/endScriptEvaluation()`. [Observed: `rules/SQLDelimiterSetRule.java` L38–93; `rules/SQLDelimiterRule.java` L98–124; `SQLScriptParser.java` L951–966]
- **Delimiter matching** — case-insensitive; an alphanumeric delimiter (e.g. `GO`) only matches when the next char is not an identifier part (so `GOTO` is not `GO`). [Observed: `rules/SQLDelimiterRule.java` L55–86]
- **SQL Server `GO [count]`** — `SQLBatchDelimiterRule`: only whitespace may precede it on the line; after the word (and optional suffix) only inline whitespace, then end of line, end of text (only if the scan range really ends at a line end), or a line comment. `TransactSQLBatchDelimiterRule.consumeSuffix` accepts an optional positive integer (`GO 2`; `GO 0` invalid). [Observed: `rules/SQLBatchDelimiterRule.java` L69–145; `ext.mssql/.../TransactSQLBatchDelimiterRule.java`]
- **Oracle `/`** — not provided by the Oracle dialect in CE [Observed: grep finds no slash delimiter in `ext.oracle`]; the generic fallback batch rule can serve any configured batch delimiter (test `provideFallbackForUnclaimedBatchDelimiter` configures `GO` and `/`) [Observed: `SQLScriptParserTest.java` L1042–1062]. Users typically add `/` as an extra statement delimiter in preferences [Inferred from `SQLSyntaxManager.init` L176–183].

### 3.3 Parameters, variables and script commands

**Terms.** A *parameter* is a placeholder in SQL whose value is supplied at execution (`WHERE id = :id`). An *anonymous* parameter is `?` (position only). A *variable* is DBeaver's client-side template value `${name}` that is expanded before sending.

**Tokenization.**
- `ScriptParameterRule` (one per prefix) [Observed: `rules/ScriptParameterRule.java` L52–120]: refuses to start if the previous char is an identifier char, the prefix itself, the anonymous mark, `\`, `/`, or `[` when the prefix is `:` (avoids `a::int` casts, `x:=1`, `[:alpha:]`, URLs). Reads either a quoted name (`:"My Var"`, must be > 3 chars incl. quotes) or a run of Java identifier chars (no `.`, #22335). `?` alone is a parameter only if anonymous parameters are enabled.
- `ScriptVariableRule`: `${name}` (also Jasper `$P{}`/`$P!{}` if enabled) and `{{name}}`; name = any non-whitespace chars up to `}`. [Observed: `rules/ScriptVariableRule.java` L38–118]

**Collection** — `DefaultSQLQueryParameterParser.parseParametersAndVariables()` [Observed: L60–140]:
1. Only if parameters are enabled: scan tokens; the first non-comment token decides `ddl` (first word ∈ DDL keywords → parameters disabled) or `execute` (∈ execute keywords → anonymous `?` ignored, because `CALL p(?)` is native).
2. Each `T_BLOCK_TOGGLE` flips `dollarQuote`; named and anonymous parameters inside `$$` bodies or DDL are dropped unless `sql.parameter.ddl.enabled` (default false).
3. If the dialect wants raw `?` markers different from the user setting, `?` inside non-string tokens is also collected.
4. Variables are then found with a **regex over the raw text** — including inside strings and comments — `\$P?!?\{(?<pn>[a-z0-9_."]+)\}|\{\{(?<cpn>[a-z0-9_."]+)\}\}` (case-insensitive); each is inserted in offset order unless already present. [Observed: `SQLQueryParameter.java` L29–33; parser L142–194]
5. Each parameter links to the previous occurrence of the same name (`setPrevious`) so the UI asks once per name.

**Value filling** — `SQLScriptContext.fillQueryParameters` [Observed: L299–353]: a UI provider (bind dialog) may supply values; otherwise values come from script variables (`@set`), then default parameter values. Names are normalised: quoted → unquoted as-is, unquoted → `toUpperCase()` (default locale!). Then `SQLUtils.fillQueryParameters` replaces each non-native parameter token **textually**, right-to-left, with its value string; empty value → `NULL`. [Observed: `SQLUtils.java` L1446–1461]

**Script commands** — extension point `org.jkiss.dbeaver.sqlCommand`. CE registers `set`, `unset`, `echo`, `export` (model.sql), `include` (editor, interactive), `mysql.source`, `exasol.define`, `ai`; `export` also exists as a *pragma*. [Observed: `model.sql/plugin.xml` L71–76; `ui.editors.sql/plugin.xml` L54–56; `ext.mysql.ui/plugin.xml` L79; `ext.exasol/plugin.xml` L495; `model.ai/plugin.xml` L51–56; `data.transfer.ui/plugin.xml` L117–119]
- `@set name = value`: name parsed with the same parameter-name rules; `=` required; the value has `${…}` expanded using existing variables, then stored. Errors are explicit ("Missing variable name. Expected syntax: @set varName = value or expression"). [Observed: `commands/SQLCommandSet.java` L35–60]
- Multi-line form `@@set name = text … @@`. [Observed: test `parseMultilineParametersFromSetCommand`, `SQLScriptParserGenericsTest.java` L515–531]

### 3.4 Code completion

#### 3.4.1 How the editor chooses an engine

`SQLCompletionProcessor.computeCompletionProposals` [Observed: `ui.editors.sql/.../syntax/SQLCompletionProcessor.java` L150–255]:
1. Template lookup mode → template proposals.
2. If the word is preceded by the command prefix `@` → command proposals.
3. `mode = SQLAutocompletionMode` (`DEFAULT`=old only, `NEW`=new only, `COMBINED`=both). **Default is `NEW`** [Observed: `SQLEditorPreferencesInitializer.java` L86].
4. New engine used iff `mode.useNewAnalyzer && ADVANCED_HIGHLIGHTING_ENABLE && READ_METADATA_FOR_SEMANTIC_ANALYSIS && dataSource != null && dialect instanceof BasicSQLDialect` (both prefs default true, L115–116).
5. Old engine used if `(wordPart != null && mode.useOldAnalyzer) || !useNewCompletionEngine`.
6. Both run as background jobs; the processor waits; if the user kept typing and the result's origin offset no longer fits, the job is restarted (`INVALID`), or results are filtered (`PARTIALLY_VALID`). [Observed: L257–300]

Other defaults worth copying (L84–108): auto-activation on, delay 0, insert single proposal automatically, case "default", replace word off, hide duplicates off, show values off, qualified column names off, insert space after proposal on, sort alphabetically on, "match contains" on, hippie (words from the document) off.

#### 3.4.2 Legacy engine — `SQLCompletionAnalyzer`

**Word detection** — `SQLWordPartDetector` walks backwards from the cursor over word chars (and quoted parts, keeping `schema."long name"` together, #13004) to get `wordPart`; then keeps walking back collecting `prevWords` and the first *previous keyword* that is an entity query word (`FROM`, `JOIN`, `INTO`, `UPDATE`…), attribute query word (`SELECT`, `WHERE`, `ON`, `AND`, `BY`…) or exec keyword; stops at any statement delimiter; also records the non-whitespace delimiter text immediately before the word (`prevDelimiter`, e.g. `,`, `(`, `.`, `*`) and the next word after the cursor. [Observed: `parser/SQLWordPartDetector.java` L60–200]

**Classification** (ALG-DBV-SQL-04 gives the full table) produces `QueryType ∈ {TABLE, JOIN, COLUMN, EXEC, null}`. [Observed: `SQLCompletionAnalyzer.java` L118–184]

**Proposal sources** [Observed: L186–466]:
- COLUMN + empty word: tables referenced in the query (via alias analyzer) → their columns; after `ON` also a generated join condition; after `WHERE/AND/OR/SET/ON` with a value expected and "show values" on → up to 50 distinct values of the column (`MAX_ATTRIBUTE_VALUE_PROPOSALS`).
- COLUMN + `alias.` / `table.` / `schema.table.`: resolve the prefix via alias map, else via FQN search, else (bare word equal to an alias) stop.
- TABLE: children of default schema and catalog of the execution context, plus top-level objects; `schema.` prefixes navigate the object tree (`makeDataSourceProposals`, L831–957), falling back to a *structure assistant* search by mask (max 2 hits for path resolution) — this is a server-side metadata search.
- JOIN: tables, then **filter to those with an FK to/from the left table** and append ` ON`.
- EXEC: procedures.
- Then aliases/table names from the query itself (`makeProposalsFromQueryParts`, L706–754), keywords (fuzzy-sorted), optional hippie words, de-duplication by display string, "hide duplicates from non-active schema", and navigator object filters. [Observed: L631–704]

**Alias analysis** — `TableReferencesAnalyzerImpl` parses the active query with the **ANTLR** grammar (error-skipping listener) and collects `tableName → correlationName` pairs from `tableReference` subtrees. So even the legacy engine depends on the LSM grammar. [Observed: `model/sql/analyzer/TableReferencesAnalyzerImpl.java` L54–170]

**Proposal text** — `makeProposalsFromObject` [Observed: L1236–1398]: after `FROM/INTO/JOIN` and when the alias mode is not `NONE` and the dialect supports aliases (and for DML only if `supportsAliasInUpdate`), it generates an alias avoiding aliases already present (from a JSQLParser `TablesNamesFinder` visit), keywords and existing references; after `WHERE/AND` it prefixes column names with `alias.` (or table name) when the word is empty; FQN is used when the object lives outside the active schema.

#### 3.4.3 New engine — LSM grammar + semantic model

**The grammar.** `SQLStandardLexer.g4` (596 lines) and `SQLStandardParser.g4` (774 lines) describe a broad "standard SQL plus common extensions" language (DML, DDL, procedural blocks for several families, dollar-quoted bodies, variables, parameters). Error tolerance is built into the grammar with optional "anything" rules: `anyUnexpected??`, `anyWord` sprinkled after select lists, search conditions, table references, column lists, set clauses. [Observed: `SQLStandardParser.g4` L219, L291, L299, L365, L377, L387, L578, L588, L620]. The lexer is parameterised by `LSMAnalyzerParameters` (identifier quotes, parameter settings and prefixes, variables, `#` comments, procedural family, type names). [Observed: `LSMAnalyzerParameters.java`; `SQLStandardLexer.g4` L44–80]

**Recognition.** `SQLQueryModelRecognizer.recognizeQuery(text)` parses with `STMSkippingErrorListener`, then builds a model (`SQLQueryModel`) of rows sources (tables, subqueries, CTEs, joins, VALUES, table functions), result tuples, expressions, DML/DDL/procedural statements, and *symbol entries* with classes (`TABLE`, `COLUMN`, `TABLE_ALIAS`, `COLUMN_DERIVED`, `SQL_BATCH_VARIABLE`, `DBEAVER_PARAMETER`, `ERROR`…) used for semantic highlighting; it resolves relations against live metadata when allowed; if no content model is possible it falls back to classifying symbols without a model. [Observed: `semantics/SQLQueryModelRecognizer.java` L96–174; `SQLQuerySymbolClass.java` L20–40; model packages `semantics/model/{select,dml,ddl,expressions,programmaticals}`]

**Document context.** `SQLDocumentSyntaxContext` stores one `SQLDocumentScriptItemSyntaxContext` per statement in an `OffsetKeyedTreeMap` (a balanced tree keyed by offsets that can shift all following keys in O(log n)); on edit, `applyDelta` either shifts items (pure insertion inside one item) or drops affected items and shifts the tail. [Observed: `semantics/SQLDocumentSyntaxContext.java` L36–50, L199–278; `OffsetKeyedTreeMap.java` 1218 lines]

**Background parsing.** `SQLBackgroundParsingJob` [Observed: `ui.editors.sql/.../semantics/SQLBackgroundParsingJob.java`]: listens to document changes and viewport scrolls; queues dirty regions; schedules with 250 ms delay (0 ms if the typed text is `.` and nothing is running; doubled if a parse is running, L603–622); parses only the **visible fragment**, drops script items outside it (L658–700), extends the work range to whole statements via `extractQueryAtPos` and `extractNextQuery` (L746–790), then recognizes each statement and tracks script variables (MySQL session scope / SQL Server batch scope).

**Completion context** — `SQLQueryCompletionContext.prepareCompletionContext(scriptItem, offset, …)` [Observed: `semantics/completion/SQLQueryCompletionContext.java` L1648–1717]:
1. No model → empty context. Cursor after a statement that ends with a context boundary (delimiter) → "off-query" context (statement-start keywords only). Short text that is a single word prefix of a keyword → off-query.
2. `LSMInspections.prepareAbstractSyntaxInspection(position)` → `SyntaxInspectionResult{predictedTokenIds, predictedWords, expectingTableReference, expectingColumnName, expectingColumnReference, expectingIdentifier, expectingTableSourceIntroduction, expectingColumnIntroduction, expectingValue, expectingJoinCondition}`. [Observed: `LSMInspections.java` L401–428]
3. `model.findLexicalContext(position)` → the scope (which sources are visible) and possibly the lexical item under the cursor.
4. `collectNameNodes(position)` → the dotted name being typed (`a.b.c|`), whether it has a trailing period, and the current term.
5. `prepareProposal` dispatches: lexical item → member/column completions in its origin; dotted name → identifier completions; known symbol origin → completions from that origin; identifier expected → identifier completions; else "free" completions (columns if a column is expected and no name typed, tables if a table is expected). Keywords from `predictedWords` are added unless after a period. [Observed: L307–343]

**Proposal kinds and ranking.** `SQLQueryCompletionItemKind` carries a sort order (lower = earlier): `JOIN_CONDITION 0`, `DERIVED_COLUMN_NAME 100`, `TABLE_COLUMN_NAME 100`, `COMPOSITE_FIELD_NAME 100`, `RELATED_TABLE_NAME 150`, `RELATED_SUBQUERY_ALIAS 150`, `SUBQUERY_ALIAS 200`, `NEW_TABLE_NAME 200`, `USED_TABLE_NAME 200`, `SCRIPT_VARIABLE 225`, `LOCAL_VARIABLE 225`, `SCHEMA 250`, `TYPE 250`, `CATALOG 275`, `PROCEDURE 300`, `RESERVED 400`, `GLOBAL_PSEUDO_COLUMN 500`. "Related" = has an FK association with something already in the query. Each item also has a fuzzy match score. A trailing space is appended to keywords and to table/column names when the grammar expects them. [Observed: `SQLQueryCompletionItemKind.java` L19–101; `SQLQueryCompletionAnalyzer.java` L188–202]

**Also exposed via LSP.** `model.lsp/.../LspSQLCompletionContextParser.java` L154–167 instantiates either engine for the Language Server Protocol endpoint (used by CloudBeaver/VS Code-like clients) [Observed].

#### 3.4.4 Join completion and alias handling

See ALG-DBV-SQL-05 for the spec; key facts:
- Legacy: after `JOIN` the table list is filtered to FK-connected tables and each proposal gets ` ON` appended; after `ON` one condition string is generated by `SQLUtils.generateTableJoin(left, right)`: FK left→right, else FK right→left, else "left best identifier columns all exist by name in right", else the reverse. Multi-column FK conditions are joined with ` AND `. [Observed: `SQLCompletionAnalyzer.java` L765–829; `SQLUtils.java` L1295–1400]
- Bug-like detail: the by-name fallback (`generateTableJoinByColumns`, L1326–1355) appends multiple column equalities **without** an ` AND ` separator — only harmless for single-column identifiers. [Observed]
- New: for each left-side column with a real attribute and each right-side column with a real attribute, if one is FK-associated to the other, emit `left = right` with a fuzzy score; associations are built per entity from single-column FKs only (`aa.size() == 1`, "TODO consider compound keys"). [Observed: `SQLQueryCompletionContext.java` L1062–1102, L2105–2165]

### 3.5 Highlighting

- **Syntactic**: `SQLRuleScanner` adapts the model-level `TPRule`s to Eclipse JFace rules and maps `SQLTokenType` to theme colors; keyword bold style is configurable (`SQL_FORMAT_BOLD_KEYWORDS`, default true). [Observed: `ui.editors.sql/.../syntax/SQLRuleScanner.java` L48–227]
- **Partitioning**: `SQLPartitionScanner` uses only the control, comment, quoted and string rules (+ dialect `PARTITION` rules, e.g. dollar quotes consumed whole) to split the document into partitions. Partitions let the splitter, word detector and completion know "am I inside a string/comment". [Observed: `model.sql/.../SQLPartitionScanner.java` L37–100]
- **Semantic**: symbol classes from the semantic model map to token types `T_TABLE`, `T_COLUMN`, `T_TABLE_ALIAS`, `T_SEMANTIC_ERROR`… and drive extra coloring and problem markers (`PROBLEM_MARKERS_ENABLED` default true). [Observed: `SQLQuerySymbolClass.java`; `SQLEditorPreferencesInitializer.java` L113]
- **Large files**: rules are disabled above 2 MB (`MAX_FILE_LENGTH_FOR_RULES = 1024 * 1000 * 2`, configurable as `SCRIPT_BIG_FILE_LENGTH_BOUNDARY`). [Observed: `SQLEditorBase.java` L117; `SQLEditorPreferencesInitializer.java` L80–82]

### 3.6 Formatting

Registered via extension point `org.jkiss.dbeaver.sqlFormatter`: `default` (`SQLFormatterTokenized`), `compact` (`SQLFormatterCompact` = tokenized with `isCompact=true`), `external` (`SQLFormatterExternal`, disabled in multi-user mode), and in the editor bundle a `sqlworkbenchj` formatter. [Observed: `model.sql/plugin.xml` L17–21; `ui.editors.sql/plugin.xml` L1701–1705]

- **Tokenizer** (`SQLTokensParser.nextToken`, L115–300): its *own* character tokenizer, independent from the editor rules: SPACE, `;`, numbers (incl. hex, exponent), single-line comments (per dialect), `@?` (PG JSONPath operator), words (KEYWORD if `getKeywordType == KEYWORD`, else NAME; `DELIMITER …` line → COMMAND), `/* */` comments, quoted strings/identifiers with doubled-quote and escape-char handling (unterminated → just a SYMBOL), symbols with two-char combos, else UNKNOWN.
- **Pipeline** (`SQLFormatterTokenized.format`, L51–117): transform keyword case → optional "blank lines become delimiters" → normalise spaces around comments → merge `ORDER BY`, `GROUP BY`, `CONNECT BY`, `START WITH`, `(+)` → `IndentFormatter.format` → collapse `( x )` → insert single spaces between tokens with exceptions.
- **IndentFormatter** (593 lines): a keyword-driven state machine with an indent counter and three parallel bracket stacks (function bracket? condition bracket? constraint bracket?). See ALG-DBV-SQL-06.
- **Options** (all in `ModelPreferences`): keyword case (`""` = dialect default), LF before comma (false), break before close bracket (false), insert delimiters in empty lines (false), external command / use temp file `${file}` / timeout (2000 ms). [Observed: `ModelPreferences.java` L251–258, L396–402]
- **External**: runs the configured command, writes source to stdin or a temp file, waits up to `timeout` in 10 slices, kills a hung process and returns the original text. [Observed: `format/external/SQLFormatterExternal.java`]

### 3.7 Templates

Eclipse template engine with SQL-specific variable resolvers (`SQLEntityResolver`, `SQLAttributeResolver`, `SQLContainerResolver`, `SQLDataTypeResolver`) and per-driver context types. Default templates: `scount` → `select count(*) from ${table};`, `swhere`, `scgb` (`${col:column(table)}` — a column of the chosen table), `sob`, `sf`. [Observed: `ui.editors.sql/templates/default-templates.xml`; `ui.editors.sql/.../templates/`]

### 3.8 Execution

`SQLQueryJob.run` [Observed: `ui.editors.sql/.../execute/SQLQueryJob.java` L213–373]:
1. Open one `DBCSession` with purpose `USER` (single) or `USER_SCRIPT` (many); notify Query Manager "script begin".
2. If the connection is in manual-commit mode but the script commit type is `AUTOCOMMIT`, switch to auto-commit for the run.
3. For each element: `executeSingleQuery` (control commands go to their handlers; queries to `executeSingleElement`).
   - On "confirm execution" connection types (e.g. Production), non-SELECT statements show a Yes/No/Yes-to-all dialog (L487–507).
   - Fill parameters (may show bind dialog, may cancel) (L514–523).
   - Apply result-viewer filters by wrapping/rewriting the query via `dialect.addFiltersToQuery` (L533–549).
   - `executeStatement` (L689–864): make statement with offset/maxRows, set fetch size, statement timeout (pref, default 0 = none); execute; then loop: open result set → fetch into a receiver (a result tab), ask the user after `RESULT_SET_MAX_TABS_PER_QUERY` = 20 result sets; else read update count; `nextResults()` while the driver supports multiple results (with workarounds for drivers that fail on the n-th result, #2792; stop after 1000 update-count reads).
   - On error: dialog Stop / Retry / Ignore / Ignore all.
4. Commit/rollback at end depending on `SQLScriptCommitType` (`AT_END`, `AUTOCOMMIT`, `NLINES`, `NO_COMMIT`; default `NO_COMMIT`) and `SQLScriptErrorHandling` (`STOP_ROLLBACK` default, `STOP_COMMIT`, `IGNORE`). [Observed: `SQLScriptCommitType.java`; `SQLScriptErrorHandling.java`; `SQLEditorPreferencesInitializer.java` L47–49]

**Explain plans.** Interface `DBCQueryPlanner.planQueryExecution(session, query, config) → DBCPlan` with plan style; plans are trees of `DBCPlanNode` (optional cost/rows/duration features) and can be serialized/saved. [Observed: `model/exec/plan/*.java`; `impl/plan/AbstractExecutionPlanSerializer.java`]. PostgreSQL: modern servers → `EXPLAIN (FORMAT XML[,ANALYZE][,VERBOSE][,BUFFERS]…[,COSTS FALSE][,TIMING FALSE])`, `TIMING` only with `ANALYZE`; old servers → `EXPLAIN [VERBOSE]` text parsed by indentation and `->` prefixes. Auto-commit is turned off for the explain and **always rolled back** afterward. [Observed: `ext.postgresql/.../plan/PostgreExecutionPlan.java` L97–229]

**Query Manager / history.** Every statement/transaction/session event is collected (`QMMCollectorImpl`, in-memory past events capped at 10 000) and shown in the Query Manager view; optionally written to daily files `dbeaver_sql_<date>.log` in the metadata folder or a configured directory, with purge by `PROP_HISTORY_DAYS`. [Observed: `runtime/qm/QMMCollectorImpl.java` L49, L184–262; `runtime/qm/QMLogFileWriter.java` L46–110]

### 3.9 Comparison with DataGrip (behavioral, from teardown notes)

| Aspect | DBeaver CE | DataGrip (per `.refs/datagrip-extract/report-editor.md`) |
|---|---|---|
| Parser | Rule tokenizer for splitting + one generic ANTLR grammar for semantics | One generated lexer/parser per dialect (~30), shared PSI interfaces |
| Splitting | Token loop + block stack + dialect hacks + predicates | Statement boundaries come from the parse tree; `DELIMITER`, `GO`, `$$`, `/` are grammar elements |
| Execute choices | Statement at cursor / selection / script | Smallest statement, smallest subquery, largest statement, batch, whole script, from cursor, "ask" popup |
| Resolution layers | Live metadata only | Live model → bundled versioned catalog → in-file DDL overlay → project DDL data source |
| Join completion | FK-based (single condition legacy; single-column pairs new) | FK → virtual FK rules → fuzzy name match; settings for alias use and operand order |
| Ranking | Static kind order + fuzzy score | ML ranking with ~98 features |
| Formatter | Flat token rewriter, ~5 options | Tree-based, ~160–190 options per dialect family, presets |
| Parameters | `:name`, `?`, `${}`, `{{}}`; textual substitution | User pattern table per dialect + scope; native binding where possible |
| Explain | Per-DB provider, PG via XML | Per-DB provider → unified plan model; tree, diagram, flame graph |

---

## 4. Algorithm specifications

### ALG-DBV-SQL-01: Statement splitter (token loop with block stack)

- **Problem:** Split a script into executable statements (and script commands / batch markers) without a full parser, robustly for broken or partial SQL.
- **Inputs / outputs:** Input: text, dialect config (delimiters, batch delimiters, delimiter redefiner, block words, header words, inner-block prefixes, predicates, flags), mode (`script` vs `cursor`), delimiter mode (always / never / smart), `keepDelimiters`, `includeBatchDelimiterElements`. Output: ordered list of elements `Query{text, offset, length, endsWithDelimiter, hasVariableKeyword}` | `ControlCommand{text, commandId, offset, length}` | `BatchDelimiter{text, offset}`.
- **Algorithm (pseudocode, our own; mirrors `SQLScriptParser.parseQueryImpl` L110–498 and `extractScriptQueries` L921–994):**

```
fn split(text, cfg, mode) -> Vec<Element>:
  out = []; pos = 0
  lexer.begin_eval()                       # DELIMITER changes take effect while splitting
  loop:
    e = next_element(text, pos, cfg, mode); if e is None: break
    out.push(e); pos = e.end
  lexer.end_eval()
  if cfg.delimiter_mode == SMART: merge_continuations(out)   # ALG-DBV-SQL-07
  return out

fn next_element(text, start, cfg, mode):
  st = State{stmt_start=start, valuable=false, blocks=Stack(), had_blocks=false,
             prev_type=None, first_kw=None, last_kw=None, lf_after_comment=0}
  use_blank = mode.script ? cfg.mode.smart : cfg.mode.blank_line
  for tok in lexer.tokens(text[start..]):
    pred.capture(tok) if tok not whitespace/comment
    is_delim = tok.type == DELIMITER
               || (cfg.newline_is_delim && tok.is_ws && tok.text.contains('\n'))
               || (use_blank && tok.is_ws && st.lf_after_comment + count('\n', tok) >= 2)
    st.lf_after_comment = 0
    if tok.len == 1 and tok.text in "({[": push(Bracket)
    if tok.len == 1 and tok.text in ")}]": pop_if_any()
    if tok.type == BLOCK_BEGIN and st.prev_type == BLOCK_END: tok.type = UNKNOWN   # END LOOP / END CASE
    if tok.type == DELIMITER and st.prev_type == BLOCK_BEGIN: pop_if_any()          # "BEGIN;"
    match tok.type:
      BLOCK_HEADER: push(Header); st.had_blocks = true
      BLOCK_TOGGLE: if top is Toggle(tok.text): pop() else push(Toggle(tok.text)); st.had_blocks = true
      BLOCK_BEGIN:  if top is Header and (top.from_predicate or st.last_kw not in cfg.inner_prefixes): pop()
                    push(Block(begin=tok.text)); st.had_blocks = true
      BLOCK_END:    if top is Block: (closed_case = top.begin == "CASE"); pop()
      _ if is_delim and blocks not empty: continue                                    # delimiter inside block
      SET_DELIMITER | CONTROL: is_delim = true; is_control = !st.valuable || cursor_inside(tok)
      COMMENT: st.lf_after_comment = trailing_newlines(tok)
    update first_kw/last_kw (skip END that closed a CASE)
    action = pred.evaluate()
    if action == BEGIN_BLOCK: pop all Header; push(Block)
    if action == END_BLOCK: pop_if_any()
    if blocks not empty: continue
    if action == SKIP_SUFFIX_TERM: continue
    if action == BLOCK_HEADER: push(Header(from_predicate)); continue-on-next
    if cfg.include_batch && tok is BATCH_DELIMITER && !st.valuable: return BatchDelimiter(tok)
    if is_control and (no valuable before or at EOF or delimiter past cursor): return ControlCommand(tok) if registered
    if st.valuable and (tok.eof or (is_delim and tok.end >= cursor)):
      text = trim(text[st.stmt_start .. tok.start])
      if empty: if tok.eof: return None; st.stmt_start = tok.end; continue
      if is_delim and (keep_delims or (st.had_blocks and cfg.delim_after_query)
                       or needs_delim_after_block(st.first_kw, st.last_kw)) and tok.text == ";":
          text += ";"
      return Query(text, offset=stmt_start, ends_with_delim = tok.type == DELIMITER)
    if is_delim: st.stmt_start = tok.end; reset(valuable, first_kw, pred, had_blocks)
    if tok.eof: return None
    if !st.valuable and !tok.is_ws and !is_control and !is_delim:
      st.valuable = tok.type != COMMENT || cfg.comment_is_query || force_comments
    st.prev_type = tok.type unless whitespace/eof
```

- **Data structures:** linked stack of block frames `{kind: Bracket|Header|Block|Toggle, toggle_text, begin_word, from_predicate}`; predicate tries (prefix trie + reversed-suffix trie); a lexer whose delimiter set is mutable during evaluation.
- **Complexity / performance notes:** O(n) tokens per statement; but `extractScriptQueries` restarts the tokenizer for each statement from its start offset, and the cursor variant (`extractQueryAtPos`) rescans from a computed start line — overall linear in script size, but called on every execute/background parse. DBeaver uses a `LinkedList` for results so smart merging is O(1) per removal (comment at L943). Predicate evaluation per token is O(active trie nodes).
- **Edge cases (exhaustive list):**
  1. Empty text / only whitespace → no element.
  2. Only comments: element only if dialect `supportsCommentQuery()` (PG: true) or comments are forced.
  3. Delimiter inside string, quoted identifier or comment → not a delimiter (tokenizer consumes them whole).
  4. Unterminated string/comment → token runs to EOF (`breaksOnEOF`) — the rest of the script is one statement.
  5. Delimiter inside `( … )`, `[ … ]`, `{ … }` → ignored.
  6. Unbalanced `)` → pop is a no-op (never negative depth).
  7. `BEGIN;` / `begin transaction;` → transaction, not a block (#7821; tests `parseBeginTransaction`).
  8. `END IF;`, `END LOOP;`, `END CASE;` → only one pop.
  9. `CASE … END` in an expression (Oracle view) → the trailing `;` must *not* be kept (issue 40779).
  10. `DECLARE … BEGIN … END;` → header popped at `BEGIN` unless preceded by `AS`/`IS` (procedure inside DECLARE).
  11. `CREATE FUNCTION … IS … BEGIN … END;` (Oracle) → header via predicate.
  12. Nested `$a$ … $b$ … $b$ … $a$` → nested toggles.
  13. `$1` positional parameter → not a toggle.
  14. Unclosed block at EOF → statement extends to EOF ("leftover blocks" trace).
  15. `DELIMITER //` then `…//` then `DELIMITER ;` → delimiter changes mid-script; `DELIMITER $$;` → `$$`.
  16. `GO`, `GO 5`, `  go  `, `GO -- comment` → batch separators; `GO 0`, `GO 2 text`, `GO /* c */`, `select GO value`, `'GO'`, `-- GO` → not.
  17. `GOTO label` → not `GO`.
  18. Blank-line delimiter in script mode only in smart mode; in cursor mode whenever "blank line" is on.
  19. Line comment followed by a blank line (`-- c\n\nSELECT`) → counts as blank line.
  20. `@set a = 1` lines between statements → separate ControlCommand elements; `@@set … @@` spans lines.
  21. Control command directly after a statement without delimiter → the statement is extracted first (pro#3935).
  22. Leading comment before a HANA `DO BEGIN` → stripped from the statement.
  23. SQL Server `MERGE …;` → `;` kept.
  24. Oracle `CREATE … END;` → `;` kept; plain Oracle DML `;` dropped (driver rejects `;`, ORA-00911).
  25. Selection execution → selection text used as-is (trailing delimiter trimmed by preference).
  26. Token offset beyond document end (JFace EOF bug) → clamped / statement aborted.
  27. CRLF line endings → normalised via `fixLineFeeds`.
  28. Text > 2 MB in the editor → rule-based features off (splitting still used on execute).
- **Test cases (input → expected)** — from DBeaver tests:
  - PG, `SQLScriptParserTest.parsePostgresDoubleDollar` (L98–133): two `CREATE … FUNCTION … AS $$ BEGIN …; …; END; $$ LANGUAGE plpgsql;` statements separated by a blank line → exactly 2 statements, each ending in `LANGUAGE plpgsql` (no trailing `;`), inner `;` kept inside.
  - Oracle, `parseOracleDeclareBlock` (L135+): four consecutive `DECLARE … BEGIN … END;` blocks incl. nested `PROCEDURE … IS BEGIN … END;` and nested `DECLARE BEGIN END` → 4 statements.
  - Oracle, `parseOracleWithBlock` (L380–431): `WITH FUNCTION f … IS BEGIN RETURN p_id; END; … SELECT …;` → one statement each.
  - Oracle, `parseOracleCreateViewWithCaseEnd` (L557–580): `CREATE OR REPLACE view VM_TEST as( … WHERE CASE WHEN C = 1 THEN Q ELSE 1 END <> 0\n);` → text ends with `)`, no `;`.
  - PG & SQL Server, `parseBeginTransaction` (L710–720): `begin transaction;\nselect 1 from dual;` → `["begin transaction", "select 1 from dual"]`.
  - SQL Server, `extractSqlServerBatchDelimiterAfterTerminatedStatement` (L902–912): `select 1;\n  GO\nselect 2\nGO -- final batch` → `["select 1", "select 2"]`.
  - SQL Server structural mode (L914–940): `select 1;\nGO 2\nselect 2;\nGO` → `[Query, Batch("GO 2"), Query, Batch("GO")]`.
  - `extractLeadingTrailingConsecutiveSqlServerBatchDelimiters` (L977–988): `GO\n go \nselect 1;\nGO\nGO` → `["GO","go","select 1","GO","GO"]`, 4 batch elements.
  - `extractSqlServerBatchDelimiterWithRepeatCount` (L990–1000): `select 1;\nGO 2 -- repeat batch\nselect 2` → `["select 1", "-- repeat batch\nselect 2"]`.
  - `rejectInvalidSqlServerBatchDelimiters` (L1074–1078): `select 1\nGO 0\nselect 2;\nGO 2 text\nselect 3;\nGO /* comment */` → no batch separators.
  - `doNotClassifyInlineStringOrCommentSqlServerBatchDelimiters` (L1102–1106): `select GO value;\nselect 'GO';\n-- GO\n/*\nGO\n*/\nselect 1 -- GO` → none.
  - `rejectBatchDelimiterAtArtificialRangeEnd` (L1064–1072): text `GOfoo`, scan range limited to `GO` → not a batch delimiter.
  - `extractControlCommandAfterSqlServerBatchDelimiter` (L1080–1100): `select 1;\nGO\n@set value = 1\nselect 2` → queries `["select 1","select 2"]`, command `@set value = 1`.
  - Snowflake, `SQLScriptParserGenericsTest.parseSnowflakeIfExistsStatements` (L299–354): `DROP TABLE\r\nIF\nEXISTS dim_appt;` is one statement; procedures with `IF … END IF;` inside `BEGIN … END;` are one statement each.
  - HANA, `parseQueryBeforeBlockDeclaration` (L109–151): `/* Issue */\nDO BEGIN\nSELECT * FROM dummy;\nEND;` → `DO BEGIN\nSELECT * FROM dummy;\nEND` (comment stripped); same on Snowflake keeps the comment; on Oracle keeps comment *and* `;`.
- **Prior art and evidence:** `SQLScriptParser.java` L110–559, L921–994; `SQLRuleManager.java` L104–321; rules package. DataGrip: splitting comes from per-dialect grammars and a script model with "smart/strict ranges" and live tracking during execution [report-editor §1]; our earlier recommendation for Hacivat (same report §7) is a token splitter with paren depth, block stack and active delimiter.
- **Recommendation for Hacivat:** Implement exactly this *shape* (token stream + explicit block stack + mutable delimiter) as a pure function in a `sql-syntax` crate, but (a) make the lexer lossless and dialect-flagged (quote styles, `$tag$`, nested comments, `#` comments, `--` needing a space in MySQL), (b) represent dialect structure as data (block pairs, header words, inner prefixes, "BEGIN followed by ; is a transaction", "END X closes one block"), (c) avoid hidden global state — return the delimiter changes as part of the element stream, (d) treat `GO [n]` and `/` as first-class line-isolated batch separators selectable per dialect (`/` for Oracle by default, unlike DBeaver), (e) keep the predicate idea but as a small declarative table per dialect, tested by fixture files. Run it incrementally: re-split only from the statement before the edit to the first statement whose start and lexer state match the previous run.
- **Used by:** Hacivat (editor, execution, MCP `execute_script`), Karagöz (server-side statement classification for audit/permissions — must split identically to the client).

### ALG-DBV-SQL-02: Block detection (which constructs suspend delimiters)

- **Problem:** Decide when a `;` inside procedural code must not end the statement.
- **Inputs / outputs:** Token stream with types `BLOCK_BEGIN`, `BLOCK_END`, `BLOCK_TOGGLE`, `BLOCK_HEADER`, brackets, keywords; dialect lists. Output: at each token, "inside block?" boolean and the block stack.
- **Algorithm:**
  1. Word classification at tokenization: `getBlockBoundStrings()` pairs → first word BEGIN-type, second END-type (a multi-word end such as `END LOOP` is effectively matched as `END` + demoted `LOOP`); `getBlockHeaderStrings()` → HEADER; dollar quotes → TOGGLE (PG/DuckDB/Snowflake/Vertica/Denodo); dialect multi-word rules turn `BEGIN TRAN` etc. into plain keywords *before* the word rule runs.
  2. Stack transitions as in ALG-01 step 7, plus the two "previous token" fixes (`END`→`BEGIN` demotion and `BEGIN` + `;` pop).
  3. Dialect predicates (trie-matched prefix/suffix token patterns) can push/pop/skip.
  4. Header frames are dropped when a real `BEGIN` arrives unless the previous keyword is an inner-block prefix (`AS`, `IS`); predicate-created headers are always dropped.
- **Data structures:** `ScriptBlockInfo` linked list (`SQLScriptParser.java` L1107–1132); predicate tries (`tokens/predicates/Trie.java`, `TokenPredicateSet.java`).
- **Complexity:** O(1) per token for the stack; predicate matching amortised O(k) with k = active trie branches.
- **Edge cases:** PostgreSQL has *no* BEGIN/END block words at all (bodies live in `$$`), so `BEGIN; … COMMIT;` and `DO $$ BEGIN … END $$;` both work; with `ddTagIsString=true` the body is one string token and toggles never appear. MySQL has no block words either (`getBlockBoundStrings()` null): procedures must be wrapped in `DELIMITER`. SQL Server `BEGIN TRAN`, `BEGIN DISTRIBUTED TRANSACTION`, `BEGIN TRY/CATCH` (TRY is not special → it is a block with END TRY handled by demotion), `IF … BEGIN … END ELSE BEGIN … END`. Oracle `CASE` as block word conflicts with `CASE` expressions → issue 40779 fix. `IF EXISTS` in DDL (Snowflake/BigQuery tests) must not open a block. Snowflake `$$` procedural bodies with `IF … END IF;` inside. Unbalanced `END` → ignored.
- **Test cases:** `parseOracleProcedureFunctionAs` (Issue 37925, `SQLScriptParserTest.java` L525–555): `CREATE OR REPLACE FUNCTION TEST_1.FUNC_1 \nRETURN NUMBER IS \nn NUMBER := 0;\nBEGIN\nSELECT 1 INTO n from dual;\nRETURN n;\nEND;` → one statement ending with `END;`. `parseSnowflakeCreateProcedureWithIfStatements` (`SQLScriptParserGenericsTest.java` L154–175) → one statement. `parseBigQueryDdlStatementsContainingIf` (L177–217): 20+ `CREATE … IF NOT EXISTS …` / `DROP … IF EXISTS` statements each split correctly. Grammar acceptance `recognizesNestedSqlAndClosingDollarTag` (`SQLStandardGrammarAcceptanceTest.java` L283–295): `DO $outer$ BEGIN SELECT 1; SELECT '$outer$'; END; $outer$;` → one anonymous block, 2 inner statements, 0 syntax errors.
- **Prior art:** `SQLScriptParser.java` L196–321; `SQLWordRule.java`; `OracleSQLDialect.java` L641–700; `BQSQLDialect.makeDialectSkipTokenPredicatesImpl` L102. DataGrip: blocks are grammar nodes; lazy reparse of bodies [report-editor §1].
- **Recommendation for Hacivat:** In v1, implement block detection as in DBeaver but per dialect *profiles* with explicit tests per construct; in v2, replace it with the error-tolerant parser's statement nodes (statement = top-level node; splitter falls back to the token algorithm on parse failure). Make "is `$$` a string or a block" a dialect constant (PostgreSQL bodies are always strings to the server; tokenizing inside is a highlighting concern, solved by *language injection* — parse the body as PL/pgSQL in a sub-tree).
- **Used by:** Hacivat, Karagöz.

### ALG-DBV-SQL-03: Parameter and variable parsing

- **Problem:** Find placeholders the user wants to supply at execution time, without false positives in casts, strings, labels, URLs, regexes or DDL.
- **Inputs / outputs:** Statement text + tokens; settings (named prefixes, anonymous mark, enabled flags, embedded-code flag). Output: list of `Param{name, var_name, offset, length, ordinal, previous_same_name, native_binding}`.
- **Algorithm:**
  1. Tokenize (parameter/variable rules run before quote rules, so `:x` inside a string is never a parameter, but variables in strings are found by step 5).
  2. Named token rule: prefix char at position p is a parameter start iff `text[p-1]` is not an identifier char, not the prefix itself (`::`), not `?`, not `\`, not `/`, and not `[` when the prefix is `:`. Name = quoted identifier (`:"x y"`) or run of identifier chars (no `.`).
  3. First significant token decides: DDL → ignore all named/anonymous parameters; execute keyword (`CALL`, `EXEC`) → ignore `?`.
  4. Inside dollar-quote toggles → ignore (unless embedded-code option).
  5. Variables: regex `${name}` / `$P{name}` / `$P!{name}` / `{{name}}` over the raw text including strings/comments; merge into the list by offset.
  6. Link each named param to the previous one with the same name.
- **Data structures:** list sorted by offset; per-name back-links.
- **Complexity:** O(n).
- **Edge cases:** `a::int` (PG cast) → none; `x := 1` (PL assignment) → `:` followed by `=` is not an identifier char → none; `:=` already excluded; `'2024-01-01 10:00:00'` → inside string → none; `ARRAY[1:2]` → `[` guard prevents `:2`; `http://h:8080` in a comment → comments are separate tokens → none (variables `${…}` *are* found in comments by design); `:1` (numeric name) → allowed (`isJavaIdentifierPart('1')`); `:"SYs_B_1"` → name keeps quotes; `:&6^34`, `:%#2`, `:""""` → rejected; `?` in `SELECT '?', ? -- ?` → exactly 1 parameter (the middle one) when anonymous enabled, none when disabled; `CREATE TABLE t (id INT DEFAULT :value, other_id INT DEFAULT ?)` → none; `CALL p(?)` → `?` ignored; `$1` in PG → treated as parameter only if `$` prefix (PG dialect adds it) and not dollar-quote; MySQL `@var` / SQL Server `@var` → not parameters (SQL variables, own rule); empty value at fill time → literal `NULL` substituted.
- **Test cases** (`SQLScriptParserGenericsTest.java`): `parseNamedParameters` (L356–371): `select :1, :"SYs_B_1", :"MyVar8", :AbC, :"#d2", :&6^34, :%#2, :"""" from dual` → names `["1", "\"SYs_B_1\"", "\"MyVar8\"", "AbC", "\"#d2\""]`. `parseAnonymousParametersOnlyWhenEnabled` (L373–385): `SELECT '?', ? -- ?` → disabled: `null`; enabled: 1 parameter at offset of `, ?`+2, not native. `skipParametersInDDL` (L387–392) → `null`. `parseVariables` (L394–407): `select ${aBc}, ${PrE#%&@T}, ${a@c=} from dual` → `["aBc","PrE#%&@T","a@c="]`. `parseDoubleCurlyVariables` (L409–422): same with `{{…}}`. `parseVariablesInStrings` (L424–437): `select '${aBc}', '${PrET}', '${ac}' from dual` → `["aBc","PrET","ac"]`. `parseVariablesInComment` (L454–467): `-- ${aBc}, ${PrET}, ${ac} ` → 3 names. `parseParameterFromSetCommand` (L484–513): `@set aBc = 1`, `@set "aBc" = 1`, `@set "a@c=" = 1` → names `aBc`, `"aBc"`, `"a@c="`.
- **Prior art:** `DefaultSQLQueryParameterParser.java`; `ScriptParameterRule.java`; `ScriptVariableRule.java`; `SQLQueryParameter.java` L29–33; `SQLScriptContext.fillQueryParameters` L299–353; `SQLUtils.fillQueryParameters` L1446–1461. DataGrip: user-editable pattern table with dialect sets and scopes, review-before-run dialog, native binding preferred [report-editor §5].
- **Recommendation for Hacivat:** Same detection rules, but **bind natively** (`$1`/`?`/`@p1` depending on driver) whenever the parameter sits in an expression position; only allow textual substitution for explicitly marked "template variables" (`${}`) and show a diff preview before running. Make the pattern table data-driven (like DataGrip) with defaults `:name`, `?`, `$n` (PG), `${name}`, `{{name}}`. Case-fold names with ASCII rules. Remember last values per connection+name. MCP-originated queries must never use textual substitution.
- **Used by:** Hacivat (editor, MCP), Karagöz (must see final SQL + bound values for audit).

### ALG-DBV-SQL-04: Completion context analysis

- **Problem:** At the cursor, decide what kind of thing the user is typing (keyword, table, column, alias, join condition, procedure…) and which names are in scope.
- **Inputs / outputs:** Document, cursor offset, active statement, dialect, metadata cache. Output: request kind + scope + typed prefix (+ dotted qualifier).
- **Algorithm (legacy, `SQLCompletionAnalyzer.runAnalyzer` L118–184):**
  1. Word part = chars before cursor that form an identifier (possibly dotted/quoted). Walk back to the previous "context keyword" (entity, attribute or exec word); stop at a statement delimiter; ignore it if it lies before the active statement's start.
  2. Classify by previous keyword K, previous word W, previous delimiter D:

     | Condition | Kind |
     |---|---|
     | K ∈ {DELETE, INSERT} | none (keyword only: FROM / INTO) |
     | K = INTO and W present and D ∈ {`(`, `,`} | COLUMN (insert column list) |
     | K = INTO and D ∈ {`(*`, `{*`, `[*`} | COLUMN with `*` expansion |
     | K = JOIN and no W | JOIN |
     | K entity word, W present, D empty | none (probably typing an alias) |
     | K = INTO inside string literal | stop (no proposals) |
     | other entity K | TABLE |
     | K attribute word (`SELECT`, `WHERE`, `SET`, `ON`, `AND`, `OR`, `BY`, `HAVING`) | COLUMN (+ `*` expansion if D=`*` and next word present) |
     | K exec keyword | EXEC |
  3. Keyword filtering afterwards: no K → statement-start keywords; K ∈ query keywords and an expression already ends (space or `)`) → only `FROM`; K entity → `FROM`/`INTO`/`SET`/`WHERE` respectively.
- **Algorithm (new, `prepareCompletionContext` L1648–1717 + `LSMInspections.prepareAbstractSyntaxInspection` L192–289):**
  1. Locate the script item (statement) containing the cursor in the document context; get its parsed tree.
  2. Find the last *non-error* terminal at or before the cursor (binary search by start offset). Choose the ATN (parser automaton) state: inside a term → its start state; right after a term or on a separator (`,`, `(`, `.`…) → the previous term's follow state; at an "anything" placeholder → the previous real term.
  3. Rebuild the rule stack from the tree path; simulate the ATN forward from that state collecting reachable terminal transitions (`collectFollowingTerms`), excluding rules that would flood the list with identifiers; also record *reachability* of interesting rules (`tableName`, `columnReference`, `identifier`, `columnName`, `nonjoinedTableReference`, `derivedColumn`, `pattern`) and *presence* in the current stack (`tableName`, `columnReference`, `identifier`, `nonjoinedTableReference`, `joinCondition`).
  4. Booleans: `expectingTableReference = reach(tableName) || present(tableName)`; `expectingColumnReference = reach(columnReference) || present(columnReference)`; `expectingJoinCondition = present(joinCondition) && current term is ON`; etc.
  5. If the tree has only error terms (very short/garbled input), run the ATN naively over the raw tokens before the cursor to predict next words.
  6. Resolve lexical scope from the semantic model at the cursor (sources visible incl. outer scopes for correlated subqueries, LATERAL, USING-merged columns, CTE names, projection aliases where the dialect allows them).
- **Data structures:** ANTLR ATN, rule-index stack (persistent linked list), sorted terminal list, semantic scopes (`SQLQueryLexicalScope`), offset-keyed statement tree.
- **Complexity:** ATN walk bounded by grammar size; per request O(statement tokens + reachable states). Metadata calls are cached (`LocalCacheProgressMonitor`).
- **Edge cases:** cursor at very start of a statement; cursor after a delimiter (off-query); cursor inside a string (legacy: values only; new: none); inside quoted identifier (`"Tab|`); partially typed keyword (`SEL|`); `*|` expansion; `alias.|` where alias defined *later* in text (`SELECT t.| FROM Table1 t`); aliases with the same name as tables; self-join (`test t1 join test t2`); parenthesized joins (scope of inner `ON` excludes later sources); derived tables in joins; `USING (|i)`; `GROUP BY |` (no `AS` aliases); correlated subqueries; `STRAIGHT_JOIN`; table hints before `WHERE`; statement tail without delimiter; user typing during analysis (restart).
- **Test cases** (legacy `SQLCompletionAnalyzerTest.java`): `SEL|` → `SELECT` (L83–85); `SELECT * |` → `FROM` (L89–91); `SELECT * FROM T |` → `WHERE` (L95–97); `SELECT | FROM Table1` → `Col1, Col2, Col3` (L154–159); `SELECT * FROM Table1 WHERE |` → `Table1.Col1…` (L164–169); `SELECT * FROM Table1 t WHERE t.|` → `Col1…` (L184–189); `SELECT t2.| FROM Table1 t, Table2 t2` → `Col4…Col6` (L214–219); `SELECT *| FROM Table1` → `Col1, Col2, Col3` (L261–264); `SELECT t.*| FROM Table1 t` → `Col1, t.Col2, t.Col3` (L269–272); `SELECT * FROM |` → `Table1 t`, `Table2 t`, `Table3 t` (L298–302); `SELECT * FROM Database3.|` → `"a.schema"` at replacement offset 24 (L399–402). New engine (`SQLQueryCompletionAnalyzerTest.java`): `SELECT '' AS "xxx", | FROM J1_TBL t1 CROSS JOIN J2_TBL t2` → contains `t1.i, t1.j, t2.i, t2.k`, none with ` AS ` (L721–739); `select * from (x left join y on (|x1 = y1)) left join x xx(xx1, xx2) on (…)` → contains `x1,x2,y1,y2`, not `xx.xx1` (L742–759); `select t1.| from test t1 join test t2 on true` → `id, value` (L1059–1067); `select * from tab1 as t group by |` → `id, statdate, enddate`, none with ` AS ` (L1017–1027).
- **Prior art:** above files. DataGrip: inserts a dummy identifier at the cursor, reparses, classifies the parent PSI node; resolution through 3–4 catalog layers [report-editor §2, §7].
- **Recommendation for Hacivat:** Follow the *new* engine's architecture but with a hand-written, error-tolerant recursive-descent parser that records, at every point, the set of expected token kinds/"slots" (this is cheaper and more controllable than ATN simulation and needs no ANTLR runtime). Context = (slot kinds expected at cursor, visible scope, dotted qualifier, typed prefix). Keep the legacy "previous keyword" heuristic as a fallback when parsing yields nothing. Always cancel/restart on newer keystrokes (DBeaver's restart logic is a good model).
- **Used by:** Hacivat (editor, MCP `complete`-like tools for agents).

### ALG-DBV-SQL-05: Join completion and alias generation

- **Problem:** After `JOIN` propose sensible tables; after `ON` propose ready-made join conditions; generate short readable aliases.
- **Inputs / outputs:** Left side sources (tables/aliases already in FROM), candidate right table, FK metadata (associations), unique keys; output proposals `Table alias [ON]` and `l.c = r.c [AND …]`.
- **Algorithm:**
  - *Table after JOIN (legacy, L246–254, L796–812):* compute table proposals as usual; keep only those with an FK from or to the left table; if none qualifies, keep all; append ` ON` to kept ones.
  - *Condition after ON (legacy, `SQLUtils.generateTableJoin` L1295–1323):*
    1. FK in left referencing right → `l.fk1 = r.pk1 AND l.fk2 = r.pk2 …`.
    2. Else FK in right referencing left → same, reversed.
    3. Else left's best identifier (PK, else unique key) columns all exist by name in right → equalities (DBeaver omits ` AND ` here — fix in ours).
    4. Else the reverse of (3). Else no proposal.
  - *Conditions (new, L1062–1102):* for every pair (leftColumn ∈ left side's columns with a real attribute, rightColumn ∈ right side's) where one is in the other's FK association set → item `left = right`, score = max(fuzzy(left name), fuzzy(right name)) against the typed prefix; kind `JOIN_CONDITION` (ranked first). Associations per entity: for each FK with exactly one column, map `source attr → target attr` normalised so the key is the entity's own attribute.
  - *Alias generation (`SQLUtils.generateEntityAlias` L969–1013):* take the first letter of each "word" in the name (word boundaries: after a non-letter, or lower→upper camel-case transition), only valid identifier chars; lower-case; if empty or a reserved word → `t`; if the alias is used (existing alias in statement, a keyword, or an existing table reference name) try `alias2…alias499`; if alias equals the table name (case-insensitive) → no alias. Mode NONE / PLAIN (`tbl t`) / EXTENDED (`tbl AS t`), default PLAIN. Only when the dialect supports aliases in SELECT, and for DML only if aliases are allowed in UPDATE.
- **Data structures:** per-entity association cache `{entity → attr → {target attrs, target entities}}`.
- **Complexity:** O(|L|·|R|) column pairs per request; FK metadata fetched once per entity per request (cached by monitor-backed caches).
- **Edge cases:** self-join (FK to same table: both directions match → two conditions); multiple FKs between the same pair (`created_by`, `updated_by` → several conditions, rank by name similarity); composite FKs (legacy handles, new engine skips); views (no FKs → nothing unless virtual keys); quoted/mixed-case names (alias case preserved, issues #36708/#36887); derived tables (real attributes traced through subquery); alias collision across nested scopes; the generated alias being a keyword (`as`, `on`, `is` → avoided); names starting with digits/underscores; non-Latin names (letters still count — Turkish `ö` is a letter, fine).
- **Test cases:** new engine `testJoinConditionBeforeFollowingClause` (`SQLQueryCompletionAnalyzerTest.java` L921–945): FK `Album.ArtistId → Artist.ArtistId`; `select * from Artist art join Album al on |` → contains `art.ArtistId = al.ArtistId`; same with `where …`/`group by …` after the cursor → also plain columns, and **not** table proposals `Artist a`/`Album a`; `select * from Artist art join |` → contains `Album a`, `Artist a`. `testJoinConditionBeforeAnotherJoin` (L948–970): in the first ON only `art.ArtistId = al.ArtistId` (not `al.AlbumId = tr.AlbumId`); in the second ON `al.AlbumId = tr.AlbumId` and not the first. `testPro4058ColumnsAtJoinConditionTail` (L902–918): `select * FROM Artist art join Album al ON |` (no FK) → all columns of both aliases. Legacy alias: `SELECT * FROM |` → `Table1 t` (`SQLCompletionAnalyzerTest.java` L298–302).
- **Prior art:** `SQLCompletionAnalyzer.java` L765–829; `SQLUtils.java` L969–1013, L1295–1400; `SQLQueryCompletionContext.java` L1062–1102, L1987–2170. DataGrip: FK → virtual FK rule (`{table}_id` patterns) → fuzzy name match; options for alias usage, ON operand order, qualification policy (Always / On collisions / Never), user alias table [report-editor §2].
- **Recommendation for Hacivat:** Combine both: rank (1) real FKs incl. composite (render with `AND`), (2) user/virtual FKs (rules like `{t}_id → t.id`, stored per connection and shareable via Perde), (3) name-based fallback (identical column names that are a key on one side). Put the right-hand (newly joined) table's columns on the right of `=` by default (configurable). Alias generator as DBeaver's, plus a user override table (`order_items → oi`). Qualification policy "on collision" by default.
- **Used by:** Hacivat.

### ALG-DBV-SQL-06: Tokenized SQL formatter

- **Problem:** Pretty-print SQL text deterministically with keyword-driven line breaks and indentation.
- **Inputs / outputs:** SQL text; config: keyword case, indent string, LF-before-comma, break-before-close-bracket, blank-lines→delimiters, compact flag, dialect (keywords, comments, delimiters, block words, delimiter redefiner). Output: formatted text (trailing newline preserved).
- **Algorithm (`SQLFormatterTokenized` L51–117, `IndentFormatter` L74–468):**
  1. Tokenize into SPACE/SYMBOL/KEYWORD/NAME/VALUE/COMMAND/COMMENT.
  2. Upper/lower-case KEYWORD tokens.
  3. (opt) Whitespace containing ≥ 2 newlines not preceded by a delimiter → insert first delimiter.
  4. Spaces adjacent to comments: drop single-line spaces next to comments; other spaces → one space.
  5. Merge `ORDER BY`, `GROUP BY`, `CONNECT BY`, `START WITH`, `( + )`.
  6. Indent pass over tokens with state `{indent, bracketDepth, stacks: functionBracket, conditionBracket, constraintBracket, bracketIndent, encounterBetween, isFirstConditionInBrackets}`:
     - `(`: push "is function/identifier call?" (previous token is a known function or identifier), "is condition?" (a `WHERE/ON/HAVING` precedes), "is constraint column list?" (`UNIQUE (`, `PRIMARY KEY (`, `FOREIGN KEY (`, `REFERENCES x (`); remember indent; optionally newline+indent++.
     - `)`: restore indent, optionally newline.
     - `,`: newline (before or after the comma) unless inside function call, after `IN (`, or in constraint list.
     - delimiter: indent=0, newline + blank line.
     - KEYWORD: block headers / block begins (not inside SELECT) → newline at indent-1, indent++; block end → indent--; `SELECT/INSERT/UPDATE/DELETE/INTO/TRUNCATE` → newline (indent 0 at top level, or current inside brackets), indent++, newline; `FROM/WHERE/ORDER BY/GROUP BY/HAVING/START WITH/CONNECT BY` → newline at indent-1 and newline after; join keyword sequences (`LEFT OUTER JOIN`) → newline at indent-1 at the sequence start; `ON` → newline after; `AND` → newline unless after `BETWEEN`; `OR`/`WHEN`/`ELSE` → newline (except `CREATE OR`, `CASE WHEN`); `CASE` indent++; `END` indent--; `VALUES/LIMIT` → indent-- newline; `UNION/INTERSECT/EXCEPT` → indent-=2 newline indent++; `SET` (not `CHARACTER SET`) newline after; `USING` newline at indent+1; `CREATE OR REPLACE` newline after.
     - COMMENT: newline before single-line comments, after block comments; COMMAND (`DELIMITER x`) resets indent and updates the delimiter list.
     - `insertReturnAndIndent` is suppressed entirely while inside any function-call bracket (so `COALESCE(a, b)` stays on one line).
  7. Collapse `( x )` → `(x)`.
  8. Insert single spaces between non-space tokens except around `(`/`)`, before `,`/delimiters, between function name and `(`, between NAME and VALUE, around "embedded" symbols (`: . > < [ ] # - ' % " \``), between two symbols, inside exponent numbers (`5E+6`).
- **Data structures:** mutable `List<FormatterToken>`; several parallel stacks.
- **Complexity:** O(n²) worst case (list insertions, backward keyword scans); fine for editor-sized statements.
- **Edge cases:** empty / single space → `""`; trailing newline preserved; `@var` and `@?` kept intact; backslash-escaped quotes in strings; unterminated string → treated as symbol (rest formatted as code); dollar-quoted bodies are **not** recognised by this tokenizer (`$$` is read as a word) — PG function bodies get reformatted [Inferred from `SQLTokensParser.nextToken` L115–300 having no `$` handling]; `CASE` nested in functions; constraint column lists on one line; Oracle `(+)`; comments inside queries; MSSQL `TOP n`; `CREATE TABLE` column list with `BREAK_BEFORE_CLOSE_BRACKET` on vs off.
- **Test cases** (`SQLFormatterTokenizedTest.java`, indent = `\t`): `select * from mytable;` → `SELECT\n\t*\nFROM\n\tmytable;` (L96–106); ` ` → `` (L83–93); `-- test\nSELECT * FROM mytable;` → `-- test\nSELECT\n\t*\nFROM\n\tmytable;` (L122–132); `SELECT my_field FROM my_table` → `SELECT\n\tmy_field\nFROM\n\tmy_table` (L179–192); `CREATE TABLE Persons (PersonID int, LastName varchar(255), CONSTRAINT PersonPK PRIMARY KEY (PersonID, LastName), UNIQUE (PersonID, LastName), FOREIGN KEY (PersonID, LastName) REFERENCES Parent (PersonID, LastName));` with break-before-close-bracket → one column/constraint per line, constraint column lists inline, `);` on its own line (L231–246); default multi-statement script (L67–80, expected L350–395): `… JOIN JT1 j1 ON j1.a = t.a LEFT OUTER JOIN JT2 j2 ON …` → lines `FROM`, `\tTABLE1 t`, `JOIN JT1 j1 ON`, `\tj1.a = t.a`, `LEFT OUTER JOIN JT2 j2 ON`, `\tj2.a = t.a`, `\tAND j2.b = j1.b`; `'D d\'D D'` → unchanged (L599–608); `SELECT * FROM t WHERE c @? CAST('$.x' AS JSONPATH)` → contains `c @? CAST` (L611–623); `SELECT @myvar FROM mytable` → contains `@myvar` (L626–638).
- **Prior art:** above; DataGrip: ~550-class formatting model with phrase patterns, roles and per-dialect ~160–190 options and presets [report-editor §4].
- **Recommendation for Hacivat:** Do **not** copy the flat token approach as the main formatter — it cannot respect structure (subqueries, CTEs, `$$` bodies). Build on the lossless syntax tree: (1) case pass, (2) clause grouping, (3) Wadler-style layout (`pretty` crate) with width-aware breaking, (4) special aligners (`AS`, `=`, `VALUES` matrix with a row cap), (5) never touch error nodes or injected bodies unless their language is known. Still, DBeaver's keyword rules make a good **v1 "compact/simple" formatter** and a source of golden tests. Also evaluate `sqruff`/`sqlformat` (Rust). External-command formatter: support it but sandboxed, with timeout and kill (DBeaver's 2 s default is sensible).
- **Used by:** Hacivat.

### ALG-DBV-SQL-07: Smart-mode continuation merge (blank lines that are not delimiters)

- **Problem:** Users often format one statement across blank lines; with "blank line is delimiter" this breaks statements. Smart mode merges fragments when the second one obviously continues the first.
- **Inputs / outputs:** list of fragments with `endsWithDelimiter`; dialect start-keyword set; grammar's statement-start tokens. Output: merged list.
- **Algorithm:** for each fragment F not ending with an explicit delimiter: while the next fragment G does not *start properly* (first real token ∉ statement-start tokens/keywords) and the previous one did not end with a delimiter → absorb G into F. For a cursor request: extend backwards similarly, then forwards. The merged element's text spans from the head start to the tail's original text end. [Observed: `SQLScriptParser.java` L996–1034, L1146–1288]
- **Data structures:** linked list with in-place iterator removal; per-dialect cached keyword set.
- **Complexity:** O(fragments); lexing only the first token of each fragment.
- **Edge cases:** a fragment starting with `AND`, `WHERE`, `JOIN`, `)` → merged; one starting with `SELECT` → separate (so `SELECT …\n\nSELECT …` without `;` stays two); a fragment ending with `;` stops merging; `WITH` CTE before `SELECT`; comments before the first token are skipped (hidden channel); a fragment starting with `@set` → separate.
- **Test cases:** `parseFromCursorPositionSmartModeNoDelimiterAndSpaceInside` (Issue 34731, `SQLScriptParserTest.java` L753–779): `select *\nfrom (values(random())\n   \n) as s(v)\nwhere s.v is not null\n` with cursor at two places → whole text. `parseFromCursorPositionSmartModeShowStatement` (Issue 34815, L782–827): `select *\nfrom film_actor\n\nshow search_path;` → cursor in first part gives `select *\nfrom film_actor`, in second `show search_path`. `parseFromCursorPositionSelectWithWhereAndDelimiterBeforeSecondCondition` (Issue 26843, L883–899): `SELECT *\nFROM foo\nWHERE 1=1;\n\nAND bar=1` cursor at 8 → `SELECT *\nFROM foo\nWHERE 1=1`.
- **Prior art:** as above. DataGrip: no blank-line mode needed — parser knows statement boundaries.
- **Recommendation for Hacivat:** Default to *parser-aware* boundaries: a blank line is a boundary only if the next line starts a new statement per the grammar's first-set (same idea as DBeaver smart mode, but default on). Show the execution range highlight before running (DataGrip-style) so users see what will run.
- **Used by:** Hacivat.

### ALG-DBV-SQL-08: Statement-at-cursor extraction

- **Problem:** "Execute current statement" must pick the intended statement even with the cursor after `;`, in a trailing comment, or on a blank line.
- **Inputs / outputs:** document, cursor; output: one element or none.
- **Algorithm:** see §3.2.5 (steps 1–6). [Observed: `SQLScriptParser.java` L581–727]
- **Edge cases:** cursor in `-- comment` at end of a code line → statement of that line; cursor inside `/* … */` → move forward out of it; cursor on an empty line between statements → previous line's statement if that is non-empty, else nothing; cursor right after `;` with only spaces → statement before; literal (alphanumeric) delimiters such as `GO` are skipped when searching the line for delimiters; `@set` lines (control commands).
- **Test cases:** `parseFromCursorPositionDelimitersAndMultilineComments` (Issue 26416, L830–849): text `select 10 ; -- Comments\nselect 10 ; /* Comments */` → cursor in the first gives `select 10 `; cursor in the second gives `-- Comments\nselect 10 ` (the preceding line comment is included). `parseCurrentControlCommandsCursorHead/Tail` (L582–627): five `@set colN = 'N'` lines, cursor at end of line 5 → `@set col5 = '5'`; cursor at end of line 1 → `@set col1 = '1'`. `parseMultilineCommandFromCursorPosition` (L629–649): cursor anywhere inside `@@set var1 = 'I have a long text for\nmultiple lines'@@` → the whole command.
- **Prior art / DataGrip:** DataGrip offers explicit policies (smallest statement, largest, subquery, ask) [report-editor §5].
- **Recommendation for Hacivat:** Implement the DBeaver rules for "statement at cursor" plus DataGrip's choice policies and an "ask" popup when ambiguous (nested subquery vs. whole statement). Note that DBeaver includes a preceding line comment in the statement text — good for result-tab titles (DataGrip derives tab titles from that comment).
- **Used by:** Hacivat.

### ALG-DBV-SQL-09: Dollar-quote lexing (PostgreSQL family)

- **Problem:** Recognise `$$…$$` and `$tag$…$tag$` bodies without confusing `$1` parameters or identifiers containing `$`.
- **Inputs / outputs:** character stream at a `$`; flags (named allowed, consume named body, consume plain body, partition mode). Output: STRING token over the whole body, or TOGGLE token over just the tag, or no match.
- **Algorithm (`SQLDollarQuoteRule` L64–151):** read `$`; if next is `$` → tag `$$`; else if named tags allowed and next is a letter or `_` → read `[A-Za-z0-9_]*` until `$` → tag; digits/other → no match (rewind). If consuming: scan forward; at each `$` try to read a tag; equal → done (STRING); different → treat as content and continue; EOF → rewind fully and fail. If not consuming → return TOGGLE for the tag only (and the splitter pairs toggles by exact text).
- **Edge cases:** `$1`, `$12` → not tags; `$_$` valid; `$a$ … $b$ … $a$` → inner tag ignored; unterminated body → no token (falls back to normal tokens → statement may split inside the body unless toggles are used); `a$b` identifier → word rule keeps `$` inside words; `$` at EOF; tags are case-sensitive in PostgreSQL (`$A$` ≠ `$a$`) — DBeaver compares exactly, correct.
- **Test cases:** grammar-level `recognizesNestedSqlAndClosingDollarTag` (`SQLStandardGrammarAcceptanceTest.java` L283–295) and `recognizesSqlInsideFunctionAndTriggerBodies` (L297–304): `CREATE FUNCTION f() RETURNS int AS $fn$ BEGIN SELECT 1; END; $fn$` → recognized routine with SQL inside; splitter `parsePostgresDoubleDollar` (ALG-01).
- **Recommendation for Hacivat:** Lexer always produces a `DollarString{tag, body_range}` token (server semantics), then the editor *injects* a PL/pgSQL/SQL sub-parse for the body when the statement's `LANGUAGE` is known. This avoids DBeaver's per-connection "is it a string or a block" switch.
- **Used by:** Hacivat, Karagöz (classification must not split inside bodies).

### ALG-DBV-SQL-10: Multiple-result-set execution loop

- **Problem:** One statement (or a T-SQL batch / procedure call) may return any sequence of result sets and update counts.
- **Inputs / outputs:** executed statement; limits; output: ordered list of `ResultSet` tabs and `UpdateCount`s.
- **Algorithm (`SQLQueryJob.executeStatement` L689–864):** `has = execute()`; loop { if has: (after N=20 result sets ask the user whether to continue) open result set, fetch into a new receiver; else read update count (stop if `< 0`, or after 1000 non-positive reads); if driver supports multiple results: `has = nextResults()` (stop early if max-rows reached and driver breaks on that; on error, if driver is known-broken, retry once) else break }.
- **Edge cases:** driver says "has result" but returns null; "result set missing" errors → skip to next; fetch error after some rows → keep rows, record warning; cancellation; warnings collected from the statement.
- **Test cases:** none in CE unit tests for this loop [Unknown]; behavior described from code.
- **Recommendation for Hacivat:** Model results as a stream of `Columns | Rows(batch) | UpdateCount | Notice | End` events per statement (as recommended in report-connectivity §6); the UI decides when to stop/ask; same cap (configurable, default 20 tabs).
- **Used by:** Hacivat, Karagöz (relay/audit of row counts).

---

## 5. Implications for Hacivat / Karagöz / Perde

1. **One lexer, two consumers.** DBeaver maintains *three* tokenizers (editor rules, formatter tokenizer, ANTLR lexer) that disagree (e.g. formatter has no `$$`, ANTLR lexer has its own parameter logic). Hacivat should have exactly one dialect-flagged, lossless lexer in a `sql-syntax` crate, used by highlighter, splitter, formatter, parser and Karagöz's classifier.
2. **Splitter as a shared, versioned library.** Because authorization happens at Karagöz (P2P traffic), Karagöz must split and classify statements *identically* to Hacivat. Ship the splitter in a crate both depend on; include the dialect profile version in the audit record.
3. **Dialect = data + small trait.** Use DBeaver's dialect field list (§3.1) as a checklist of what a dialect profile must declare. Keep runtime-learned bits (case rules, quote string, keywords from server) in a per-connection overlay, never mutating the static profile.
4. **Parameters bind natively.** Replace textual substitution with driver-level binding; keep `${var}` templating only as an explicit, previewed feature. Karagöz audit logs must record SQL text and bound values separately.
5. **Completion engine.** Adopt the new-engine architecture (grammar-driven expected-slot prediction + scoped semantic model), with the legacy keyword heuristic as fallback. Ranking: DBeaver's kind order is a reasonable v1 table; add recency/frequency later (DataGrip uses ML).
6. **Join completion** with composite FKs, virtual FKs (shareable via Perde org settings), and name-based fallback.
7. **Background analysis** limited to visible statements, debounced (~250 ms, immediate on `.`), cancellable; store per-statement results in an offset-shifting structure (DBeaver's `OffsetKeyedTreeMap` idea; in Rust a rope-indexed interval map).
8. **Formatter**: tree-based; DBeaver tokenized rules as "compact" preset and as golden tests.
9. **Execution**: per-statement event streams; script commit/error policies (`NO_COMMIT` + `STOP_ROLLBACK` defaults are safe); confirmation for non-SELECT on production-tagged connections — Perde can push connection "type" (production/test) to Hacivat and Karagöz can enforce it server-side.
10. **Explain**: always wrap `EXPLAIN ANALYZE` of DML in a rolled-back transaction (DBeaver's PG behavior) and warn that ANALYZE executes the statement.
11. **Query history**: persistent store (SQLite + FTS) rather than DBeaver's 10 000-event in-memory ring + flat log files; Karagöz keeps its own authoritative audit.
12. **Script commands**: provide a small, documented command set (`@set`, `@include`, `@echo`, `@export`) — but make them visibly non-SQL in the editor and never forward them to the server or MCP agents unexpanded.

## 6. i18n considerations

- **Locale-independent case folding is mandatory.** DBeaver mostly uses `Locale.ENGLISH` for keywords (`AbstractSQLDialect.getKeywordType` L243–246; `SQLDelimiterRule.changeDelimiter`), but several paths use the JVM default locale: `SQLWordRule.addWord/evaluate` (`toLowerCase()`, L54–83), `SQLScriptParser` continuation detector (`toUpperCase()`, L1195–1215), `SQLScriptContext.getNormalizedVarName` (L459), `SQLUtils.needQueryDelimiter` (`query.toUpperCase()`, L1468). Under a Turkish locale `"limit".toUpperCase()` yields `LİMİT` and `"LIMIT".toLowerCase()` yields `lımıt`, so keyword highlighting, smart-mode statement detection and variable lookup can silently fail. [Observed lines; failure mode Inferred]. Hacivat: SQL keywords and unquoted identifiers are folded with ASCII-only rules (`to_ascii_uppercase`), and *database-specific* identifier folding follows the server's rules (PostgreSQL folds unquoted names with its own ASCII-ish `downcase_identifier`).
- **UI strings vs. SQL**: DBeaver keeps all user-visible editor strings in NLS bundles (`SQLEditorMessages`, `ModelSQLMessages` — even semantic symbol class descriptions are localized, `SQLQuerySymbolClass` L20–40) while exception messages stay English (`AGENTS.md`). Hacivat: completion item kinds, diagnostics ("Unresolved table"), command help and formatter option labels must be message IDs (Fluent), not inline strings; error messages returned by servers are shown as-is.
- **Keywords are not translated** — completion item *labels* stay SQL; only descriptions/tooltips are localized.
- **Identifiers with non-ASCII letters** (e.g. Turkish table names `müşteri`) must be valid unquoted identifiers where the dialect allows (DBeaver uses `Character.isLetter`/`isUnicodeIdentifierPart`, which is right); alias generation must keep letters like `ş` (initial letters) and still be a valid identifier.
- **Line endings and encodings**: DBeaver normalises statement line feeds (`SQLUtils.fixLineFeeds`) and the external formatter uses the file encoding; Hacivat must preserve user line endings in the document but send `\n`-normalised text only where the driver needs it.
- **Templates**: template *descriptions* should be localized; template bodies are SQL and not.

## 7. Risks, pitfalls, open questions

1. **Heuristic splitter fragility.** DBeaver's splitter accumulated many special cases (each with an issue number: #7821, 40779, 37925, 34731, 34815, 26416, 22489, 26843, pro#3935). Expect a long tail; invest early in a fixture corpus (per dialect, per construct) and fuzzing (random concatenations of valid statements must round-trip).
2. **Oracle `/`** is not handled by default in DBeaver CE; SQL*Plus scripts are common — Hacivat should handle `/` on its own line for Oracle from day one. [Observed absence]
3. **Default delimiter mode differs from intuition**: DBeaver defaults to "blank line is a delimiter (Always)", which surprises users with multi-paragraph queries; smart mode exists but is not default. Decide Hacivat's default consciously (recommend parser-aware smart).
4. **Textual parameter substitution** = SQL-injection and quoting bugs (e.g. strings must be typed with quotes). Avoid.
5. **Two completion engines** doubled maintenance in DBeaver; the new one only works with a connection and metadata reading enabled (`TODO don't we want to be able to accomplish subqueries and such even without the connection?`, `SQLQueryCompletionAnalyzer.java` L77). Hacivat should support offline completion from cached catalogs (DataGrip-like layered catalogs).
6. **Composite FK join completion** missing in the new engine; by-name join fallback lacks `AND` in legacy — copy neither bug.
7. **Formatter vs. dollar-quoted bodies** — DBeaver's formatter likely mangles PL/pgSQL bodies [Inferred]; tree-based formatter with injection avoids it.
8. **Performance**: background parsing is visible-range only; > 2 MB files disable rules. Hacivat needs a documented large-file mode (plain text + on-demand splitting) and should test 10 MB migration scripts.
9. **ANTLR dependency**: DBeaver's grammar is Apache-2.0 and could serve as a *reference* for coverage (which constructs exist), but adopting ANTLR in Rust (antlr4rust) is immature; prefer a hand-written parser. [Inferred]
10. **Open questions**: (a) exact behavior of `NLINES` commit type (commit every N statements — value source not traced) [Unknown]; (b) how `SQLQueryParameterBindDialog` decides native binding [Unknown — `setNativeBinding` callers not found in model bundles]; (c) quality of the LSM grammar on vendor-specific DDL (MySQL partitions, PG `CREATE POLICY`) — acceptance tests suggest broad but not complete coverage [Inferred from `SQLStandardGrammarAcceptanceTest.java`].
