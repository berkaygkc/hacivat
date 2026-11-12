# SQL Language Tooling 01 — Landscape and Comparison

Series: `sql-tooling-01` (landscape) · `02` (lexing & splitting) · `03` (parsing architecture: rowan + salsa) · `04` (completion & name resolution) · `05` (safety classification, linting, formatting).

This document surveys the open-source SQL language tooling that is relevant to building (a) the SQL editor inside **Hacivat** and (b) a shared Rust SQL analysis crate that **Karagöz** also uses to classify and authorize statements. It ends with a reuse-vs-build recommendation. Later documents go deep on each subsystem.

---

## 1. Scope and sources

All repositories were cloned shallowly into `.refs/repos/<name>` and read only (no builds, no scripts, no tests run). Line numbers refer to the listed commits.

| Project | Repo | Commit (date) | Language | License | Role in this study |
|---|---|---|---|---|---|
| postgres-language-server ("pgls", formerly postgres_lsp / postgrestools) | supabase-community/postgres-language-server | `083c5d1bbf04` (2026-11-09) | Rust | MIT | Closest prior art for a Rust PG IDE: splitter, completion, catalog, typecheck, lint, format |
| datafusion-sqlparser-rs ("sqlparser-rs") v0.64.0 | apache/datafusion-sqlparser-rs | `8a176d98b37b` (2026-11-09) | Rust | Apache-2.0 | Multi-dialect AST parser |
| sqruff | quarylabs/sqruff | `c3d18575d6d4` (2026-11-08) | Rust | Apache-2.0 | SQLFluff port: lossless segment tree, 26+ dialects, lint + fix |
| pg_query.rs v18.1.0 | pganalyze/pg_query.rs | `20f3a182dc74` (2026-10-27) | Rust (+C) | MIT | Rust bindings to libpg_query |
| libpg_query (PG 18.6 parser) | pganalyze/libpg_query | `27112f8af5de` (2026-11-10) | C | BSD-3-Clause + PostgreSQL License (+ BSD/MIT vendored) | The real PostgreSQL parser extracted as a library |
| tree-sitter-sql | DerekStride/tree-sitter-sql | `4cd7224bfcf1` (2026-11-10) | JS grammar + C scanner | MIT | Permissive incremental grammar |
| pgcli | dbcli/pgcli | `dc9ce9447e0f` (2026-11-04) | Python | BSD-3-Clause | Classic context detection (`sqlcompletion.py`) |
| mycli | dbcli/mycli | `525f0315ef86` (2026-11-11) | Python | BSD-3-Clause | MySQL flavour of the same engine |
| @codemirror/lang-sql | codemirror/lang-sql | `f5b0b0ba1811` (2026-04-15) | TypeScript (Lezer) | MIT | Token-level grammar + dialect config + completion |
| sqls | sqls-server/sqls | `920fde7867c1` (2026-08-20) | Go | MIT | SQL LSP with handwritten parser |
| rust-analyzer | rust-lang/rust-analyzer | `2a362a41a8eb` (2026-11-10) | Rust | MIT OR Apache-2.0 | Event parser, recovery, incremental reparse, completion marker |
| rowan | rust-analyzer/rowan (`0.18.0-alpha.2` in tree) | `d601b195d7bc` (2026-10-21) | Rust | MIT OR Apache-2.0 | Green/red lossless trees |
| salsa v0.28.5 | salsa-rs/salsa | `1f48f4a32954` (2026-11-09) | Rust | Apache-2.0 OR MIT | Incremental query engine |
| biome (sparse: `biome_rowan`, `biome_parser`) | biomejs/biome | `f6cf28e3518f` (2026-11-11) | Rust | MIT OR Apache-2.0 | Rowan fork with slots + bogus nodes; recovery helpers |
| sqlglot | tobymao/sqlglot | `c4130f48d1bc` (2026-11-11) | Python (+Rust tokenizer) | MIT | Dialect transpiler; per-dialect tokenizer configs |

Not cloned (described from public knowledge and marked **[Inferred]**): ruff (MIT), SQLFluff (MIT), PostgreSQL `scan.l`/`psqlscan.l` upstream (PostgreSQL License), MySQL `sql/lex.h` (GPL-2.0 — prose only), SQLite `tokenize.c`/`complete.c` (public domain).

Evidence tags follow the library README: **[Observed: path:lines]**, **[Inferred: basis]**, **[Unknown]**.

---

## 2. Executive summary

1. **No single existing crate covers Hacivat's needs.** Every candidate is strong on one axis and weak on another: libpg_query is exact but PostgreSQL-only, non-lossless, non-error-tolerant and C; sqlparser-rs is multi-dialect but neither lossless nor error-tolerant; sqruff is lossless and multi-dialect but slow-ish, lint-oriented, and its tree is not designed for IDE semantics; tree-sitter-sql is incremental and error-tolerant but permissive (it accepts much invalid SQL and misses dialect constructs).
2. **postgres-language-server is the best architectural reference** for a SQL IDE in Rust. Its key moves: split the document into statements with a cheap token-level splitter, cache everything **per statement keyed by statement text**, use *two* parsers (libpg_query for truth, a tree-sitter fork for positional/partial context), and resolve names against a catalog = DB snapshot + DDL of the current file, with a tri-state `Found/Missing/Unknown` lookup so diagnostics are only reported when certain. [Observed: `ARCHITECTURE.md`, `crates/pgls_workspace/src/workspace/server/document.rs:44-66`, `crates/pgls_catalog/src/lookup.rs:13-23`]
3. **pgls does not do token-level incremental parsing.** It re-splits the whole document on each change and reparses only statements whose text changed (content-addressed caches). This is a strong signal that *statement-granularity incrementality is enough* for SQL editors. [Observed: `document.rs:44-66`, `tree_sitter.rs` (no `Tree::edit`)]
4. **Completion context detection in practice = dummy-identifier insertion + tolerant parse.** pgls inserts `REPLACED_TOKEN` at the cursor and re-parses with tree-sitter; rust-analyzer inserts `raCompletionMarker` and incrementally reparses. pgcli/mycli instead strip the partial word and dispatch on the *last token* of a `sqlparse` token stream. [Observed: `pgls_completions/src/sanitization.rs:7-140`; `rust-analyzer/crates/ide-completion/src/context.rs:32,803-809`; `pgcli/packages/sqlcompletion.py:58-90`]
5. **Keyword completion can be derived from the parser state** (tree-sitter `lookahead_iterator` over the parse state at the cursor) instead of hand lists. [Observed: `pgls_treesitter/src/context/mod.rs:535-553`]
6. **Ranking is additive heuristic scoring** in pgls (fuzzy score + clause-type bonuses + "mentioned relation" bonuses + system-schema penalties) and **lexicographic priority tuples** in pgcli (match quality, type priority, explicit prio, usage frequency, lexical). [Observed: `pgls_completions/src/relevance/scoring.rs`; `pgcli/pgcompleter.py:327-494`]
7. **JOIN completion uses foreign keys first, name-matching second.** pgcli generates `tbl alias ON alias.col = other.col` from FK metadata and "name join" conditions weighted by integer types. [Observed: `pgcompleter.py:575-663`]
8. **libpg_query is the only way to get exact PostgreSQL syntax** (PG 18.6 grammar), plus fingerprint, normalize, deparse, scan, split, PL/pgSQL parse and a statement summary. It is non-lossless (protobuf AST with byte locations, no trivia), aborts on the first syntax error, and needs a C toolchain (`cc` crate). [Observed: `pg_query.rs/src/query.rs:36-433`, `build.rs`; `libpg_query/src/postgres/include/pg_config.h:622`]
9. **sqlparser-rs** offers 16 dialects (15 named + `GenericDialect`; corrected after verification: count of `impl Dialect for` in `src/dialect/`) via a `Dialect` trait with ~220 hook methods (corrected after verification: 220 `fn` items in `src/dialect/mod.rs` trait body at the studied commit; earlier text said ~246), but: tokens carry unescaped values by default, spans are line/char-column (not byte offsets, not UTF-16), 193 AST span implementations return `Span::empty()`, the recursion limit is 50, and `parse_statements` aborts on the first error. Good for "parse a known-good statement in many dialects", unsuitable as an editor CST. [Observed: `src/parser/mod.rs:196,235-254,544-570`; `src/ast/spans.rs` (193 × `Span::empty()`); `src/tokenizer.rs:556-625,817-830`]
10. **sqruff** is the only Rust project with a lossless, multi-dialect (26+) tree and a large rule catalog (≈78 rules across aliasing/ambiguous/capitalisation/convention/layout/references/structure + PG/T-SQL/Oracle specific). Its parser is a combinator-grammar matcher inherited from SQLFluff; unparsable regions become `Unparsable` segments. Apache-2.0 makes rule logic reusable with attribution. [Observed: `crates/lib/src/rules/*`, `crates/lib-core/src/parser/*`]
11. **tree-sitter-sql** is permissive and error-tolerant with an external scanner for `$tag$` strings, but block comments are matched by a non-nesting regex and generated parser files live on a separate branch. pgls forked it and rewrote node types to be IDE-specific (typed identifiers). [Observed: `grammar.js:11-90`, `src/scanner.c`; `pgls_treesitter_grammar/GRAMMAR_GUIDELINES.md`, `NOTICE`]
12. **@codemirror/lang-sql** is deliberately shallow: the Lezer grammar is "a script is a list of statements, each a list of elements", with a hand-written external tokenizer parameterised by a `Dialect` record (backslash escapes, hash comments, `$$` strings, identifier quotes, ...). Completion is namespace-tree + alias scan. Useful as a *lexical configuration model*, not as a parser. [Observed: `src/sql.grammar`, `src/tokens.ts:38-77`, `src/complete.ts`]
13. **sqls** (Go) uses a handwritten tolerant grouping parser and classifies the cursor into ~10 `SyntaxPosition`s by previous keywords. Similar power to pgcli, compiled language. [Observed: `parser/parseutil/position.go:9-80`]
14. **sqlglot** provides the cleanest catalogue of per-dialect lexical differences (quotes, identifier delimiters, escapes, heredoc strings, nested comments, hash comments, hex/bit forms) as tokenizer class attributes — a good data source for Hacivat's dialect lexer tables. [Observed: `sqlglot/tokens.py:43-81`, `sqlglot/dialects/{postgres,mysql,tsql,sqlite,clickhouse,oracle}.py`]
15. **Recommended foundation:** own lexer + own rowan-based lossless, error-recovering parser per dialect, salsa for incremental semantics, and **libpg_query as an oracle** for PostgreSQL (validation, safety classification in Karagöz, fingerprint/normalize for audit). Reuse sqlparser-rs only as an optional "second opinion" for non-PG dialects in the safety classifier, and mine sqruff for rule ideas and dialect keyword lists.
16. **Security posture for Karagöz:** a client-side or agent-side parser can never be the *only* read-only guarantee (parser differentials, side-effecting functions, dynamic SQL). Use the parser to classify and to give good UX/denials, but enforce with server-side mechanisms (read-only transaction with snapshot pinned, least-privilege role, single-statement protocol). Details in `sql-tooling-05`.

---

## 3. Detailed findings

### 3.1 postgres-language-server (pgls)

#### 3.1.1 Architecture
- Two front-ends (LSP server `pgls_lsp`, CLI `pgls_cli`) over one workspace API `pgls_workspace`; feature crates (`pgls_completions`, `pgls_hover`, `pgls_inlay_hints`, `pgls_semantic_tokens`) are "intentionally free of any language-server flavour so they can be reused from the CLI". [Observed: `ARCHITECTURE.md` "Entry Points", "Feature crates"]
- ~50 crates: lexer/tokenizer (+codegen from `kwlist.h`), statement splitter, `pgls_query` (libpg_query wrapper, protobuf-generated node code), `pgls_treesitter` + forked grammar, `pgls_catalog` (snapshot + DDL overlay + resolver + PG typing rules port), `pgls_typecheck`, `pgls_analyse`/`pgls_analyser` (lint), `pgls_pretty_print` (formatter), `pgls_splinter`/`pgls_pglinter`/`pgls_plpgsql_check` (DB-side linters), `pgls_suppressions`, `pgls_line_index` (UTF-8/16/32 conversions), `pgls_wasm`. [Observed: `crates/` listing; `ARCHITECTURE.md`]
- **Lesson:** the line index is a dedicated crate and "positions are converted with it, not by hand" — LSP UTF-16 offsets vs Rust byte offsets vs Postgres character positions is a real source of bugs. [Observed: `ARCHITECTURE.md` "Diagnostics and utilities"]

#### 3.1.2 Document model and caching
- `Document::update_content` re-runs the splitter over the entire file on every change, then retains only cache entries whose statement text still occurs in the new ranges. [Observed: `crates/pgls_workspace/src/workspace/server/document.rs:44-66`]
- `StatementId` is an enum `Root{content}` / `Child{content, parent_content}` — identity is the **statement text itself** (an `Arc<str>`), with children for SQL function bodies. [Observed: `statement_identifier.rs:1-98`]
- Caches: `PgQueryStore` (AST, PL/pgSQL parse, embedded statements) and `TreeSitterStore` (CST per statement), both `Mutex<FxHashMap<StatementId, Arc<...>>>`; the tree-sitter store always parses from scratch (`parser.parse(content, None)`), i.e. no incremental tree-sitter edits. [Observed: `pg_query.rs:13-60`, `tree_sitter.rs:1-58`]
- Consequence: moving a statement (e.g. inserting a line above) does not invalidate its AST because identity is content, not position. Ranges are recomputed and offsets re-applied (`DocumentStatement::document_range`). [Observed: `document.rs:135-170`]

#### 3.1.3 Lexer and splitter
- `pgls_tokenizer` is a rustc_lexer-style cursor; comments cite PostgreSQL `scan.l` rule lines. Handles: `--` comments, **nested** `/* */` comments (depth counter), `U&'...'`, `E'...'`, `B'...'`, `X'...'`, `$tag$...$tag$` vs `$name` named params vs `$1` positional params, `@name`, psql `:name`, `:'name'`, `:"name"`, `::` cast, numeric literals with `0x/0o/0b` prefixes, and counts consecutive newlines (CRLF = one logical break). All operator characters are single-char tokens (multi-char operators are recombined later). [Observed: `crates/pgls_tokenizer/src/lib.rs:58-256,283-310,312-345`]
- Note a subtle deviation: `double_quoted_string` treats `""` as an escape (correct for PG); `single_quoted_string` likewise handles `''`; backslash handling is only via `E''` prefix. [Observed: `lib.rs:473-520`]
- The **splitter** is a small recursive-descent "statement shape" recognizer over lexed tokens (not a parser): it knows statement starts (`WITH SELECT INSERT UPDATE DELETE CREATE ALTER`), skips parenthesised regions, `CASE…END`, `BEGIN ATOMIC…END`, psql backslash commands to end of line, and uses large *previous-token exception lists* to avoid splitting at e.g. `GRANT SELECT`, `UNION SELECT`, `CREATE RULE … DO ALSO INSERT`, `ON CONFLICT DO UPDATE`, `FOR NO KEY UPDATE`, `WHEN MATCHED THEN UPDATE`, `WITH ORDINALITY`, `TIMESTAMP WITH TIME ZONE`. A line break ends a statement unless the previous token was a comma. [Observed: `crates/pgls_statement_splitter/src/splitter/common.rs:26-359`, `data.rs`, `dml.rs`]
- **Two-pass blank-line recovery:** pass 1 ignores blank lines; for every resulting fragment that *contains* a blank line, re-split it with blank lines as boundaries, and keep the split only if the whole fragment does **not** parse under libpg_query or if every piece parses on its own ("what a missing `;` looks like"). Named parameters are normalized to positional (`$1`) first because libpg_query reads `$id` as an unterminated dollar quote. [Observed: `crates/pgls_statement_splitter/src/lib.rs:17-140`]
- Lesson for Hacivat: use a token-level splitter for responsiveness and a real parser as an *oracle* for ambiguous splits.

#### 3.1.4 Tree-sitter fork and completion context
- `pgls_treesitter_grammar` is a fork of DerekStride/tree-sitter-sql (MIT, NOTICE retained) rewritten for LSP use: distinct node types `schema_identifier`, `table_identifier`, `column_identifier`, `function_identifier`, `type_identifier`, and references `table_reference`, `column_reference`, `function_reference`, with `object_reference`/`any_identifier` kept for ambiguous positions. 4,290 lines of `grammar.js`. [Observed: `crates/pgls_treesitter_grammar/NOTICE`, `GRAMMAR_GUIDELINES.md:1-40`]
- **Sanitization (dummy identifier):** if the cursor is between nodes, after a dot, after `(`, inside `()`, before `;`, after an opened `"`, etc., the text is copied with `REPLACED_TOKEN` (or `REPLACED_TOKEN_WITH_QUOTE"` when a quote is unbalanced) inserted at the cursor byte offset and re-parsed with tree-sitter. [Observed: `crates/pgls_completions/src/sanitization.rs:7-140`]
- **Context extraction:** walk from root to the deepest node containing the cursor, recording: `wrapping_clause_type` (≈30 variants: Select/Where/From/Join{on_node}/Update/Delete/Returning/Merge/AlterTable/…/CreatePolicy/CheckOrUsingClause), `wrapping_node_kind` (Relation/BinaryExpression/Assignment/List), identifier qualifiers `(head, tail)` for `a.b.c`, `is_invocation`, statement range. Then tree-sitter queries collect mentioned relations, table aliases, RETURNING `old/new` aliases (only if server ≥ 18), and mentioned columns by clause. Keyword candidates come from `language.lookahead_iterator(parse_state)` of the node under the cursor (or the first child of an `ERROR`). [Observed: `crates/pgls_treesitter/src/context/mod.rs:18-56,137-553`]
- **Scoring** (`relevance/scoring.rs`): keyword −10; fuzzy (Skim) score with +20 prefix bonus, halved/thirded; clause-type table: tables +10 in UPDATE/DELETE/MERGE, +5 in FROM/JOIN-before-ON, −50 elsewhere; columns +10 in SELECT-with-tables/WHERE/RETURNING/MERGE, JOIN-after-ON +20 if primary key else +10; schemas +15 when unqualified in FROM/JOIN/UPDATE/…; functions +30 when invocation; columns of mentioned relations +45 (schema-matched) / +30; system schemas −20; `public` +2; names containing "migrations" −15; qualifier mismatch → skip. [Observed: `crates/pgls_completions/src/relevance/scoring.rs:1-488`]

#### 3.1.5 Catalog, name resolution, typing
- `Snapshot` = introspected schema (tables, columns, functions, types, casts, operators, policies, roles, sequences, indexes, extensions, search_path). `Catalog` overlays the DDL of the current file "without touching the database". Lookups return `Found(T) | Missing | Unknown`; `Unknown` arises e.g. after `DO` blocks or `CREATE EXTENSION`, or without a snapshot — "Never report these". [Observed: `crates/pgls_catalog/src/{snapshot,catalog,lookup.rs:13-23}`; `ARCHITECTURE.md` "pgls_catalog"]
- `resolve/scope.rs` models PG FROM-item semantics faithfully: items with alias/schema/system columns/generated-column restrictions, `JOIN … USING` and `NATURAL` merged columns (with type), unknown-NATURAL flag, unmodelled items flag (`XMLTABLE`, `TABLESAMPLE`), select-list output names visible to `ORDER BY`/`GROUP BY`, name-only items (`old`/`new` in RETURNING), star-expansion order mirroring `ExpandAllTables`/`transformFromClauseItem` (links to PG REL_18_6 source). [Observed: `crates/pgls_catalog/src/resolve/scope.rs:10-271`]
- `typing/` ports PostgreSQL's coercion/common-type/polymorphic/overload resolution. [Observed: `ARCHITECTURE.md`]

#### 3.1.6 Typecheck via database
- Despite `ARCHITECTURE.md` saying "via `EXPLAIN`", the code calls `conn.prepare(sql)` (server-side PREPARE through sqlx) after `SET search_path`, on a fresh connection closed after use (to avoid cached plans across schema changes). Only SELECT/INSERT/UPDATE/DELETE/MERGE/CTE are checked directly; `CREATE TABLE AS`/`CREATE VIEW` have their inner query extracted via tree-sitter and offset-mapped. [Observed: `crates/pgls_typecheck/src/lib.rs:33-165`]
- Relevance for Karagöz: **PREPARE does not execute** the statement, so it is a safe server-side "parse+analyze" probe — but it still resolves names with the session's privileges and can be used to reveal existence of objects; and the agent must never use it with a superuser connection.

#### 3.1.7 Linting and formatting
- Analyser categories with ~80 rules: `correctness` (transaction nesting, concurrent index in txn, uncommitted txn …), `destructive` (ban drop table/column/schema/…, delete/update without WHERE, truncate, renames, type changes), `safety` (migration lock safety: add FK/PK/NOT NULL, require CONCURRENTLY, statement/lock timeouts, avoid wide lock windows …, largely Squawk-inspired), `style`, `typecheck` (unknown relation/column/function/type/schema, ambiguous column, mismatches). [Observed: `crates/pgls_analyser/src/lint/**`]
- Formatter: libpg_query AST → layout events (`GroupStart/GroupEnd/IndentStart/IndentEnd`) → renderer with line width; comments are attached by location (leading/trailing/before `CASE … END`), an unplaceable comment aborts formatting; the output is **re-parsed and the normalized AST compared** to the input; if comments move, up to 6 passes until idempotent. [Observed: `crates/pgls_pretty_print/src/lib.rs:200-330`, `renderer.rs`]

#### 3.1.8 Assessment
Excellent PG-only reference; the libpg_query + tree-sitter duality means two grammars must agree; no multi-dialect story; its splitter is PG-specific heuristic code. MIT-licensed, so algorithms and even code can be reused with attribution.

### 3.2 datafusion-sqlparser-rs (sqlparser-rs)

- **Dialects:** ansi, bigquery, clickhouse, databricks, duckdb, generic, hive, mssql, mysql, oracle, postgresql, redshift, snowflake, spark, sqlite, teradata (+ generic). [Observed: `src/dialect/*.rs`]
- **Dialect model:** a trait with ~220 methods (corrected after verification): lexical predicates (`is_identifier_start/part`, `is_delimited_identifier_start`, `supports_string_literal_backslash_escape`, `supports_nested_comments`, `supports_numeric_prefix`, `requires_single_line_comment_whitespace`, `supports_triple_quoted_string`, `supports_dollar_placeholder`…), syntax feature flags (`supports_filter_during_aggregation`, `supports_connect_by`, `supports_match_recognize`, …), and override hooks `parse_prefix`, `parse_infix`, `get_next_precedence`, `parse_statement`. [Observed: `src/dialect/mod.rs:241-1830`]
- **Tokenizer:** `Token` enum stores *values* (`Word{value, quote_style, keyword}`, `SingleQuotedString(String)`, 20+ string-literal kinds incl. Oracle `Q'{…}'`, BigQuery raw/triple-quoted), comments and whitespace as `Token::Whitespace(...)`. `with_unescape(false)` keeps raw text "to recover the exact original query text". Errors on unterminated literals (`Result<…, TokenizerError>`). [Observed: `src/tokenizer.rs:58-140,468-520,985-1040`]
- **Locations:** `Location{line, column}` 1-based, column counted in Unicode scalar values (incremented per `char`), `Span{start,end}` inclusive. Not byte offsets; LSP needs UTF-16 → conversion layer needed. [Observed: `src/tokenizer.rs:556-625,817-830`]
- **Span coverage:** `Spanned` impls exist for the AST but 193 places return `Span::empty()` — locations are incomplete. [Observed: `grep -c "Span::empty()" src/ast/spans.rs` = 193]
- **Parser:** single 23.6k-line `parser/mod.rs` Pratt + recursive descent; `DEFAULT_REMAINING_DEPTH = 50` recursion guard (`RecursionLimitExceeded`); `ParserOptions{trailing_commas, unescape, require_semicolon_stmt_delimiter}`; `parse_statements` loops over `;`-separated statements and **returns on the first error** — no recovery, no partial tree. [Observed: `src/parser/mod.rs:57-63,196,235-254,544-570`]
- **Keywords:** one global list of 1,138 keywords plus per-context reserved lists (`RESERVED_FOR_TABLE_ALIAS`, `RESERVED_FOR_COLUMN_ALIAS`, `RESERVED_FOR_TABLE_FACTOR`, `RESERVED_FOR_IDENTIFIER`), and dialect `is_reserved_for_identifier`. Not per-dialect reserved word tables. [Observed: `src/keywords.rs:1239-1360`]
- **Assessment:** good for "is this a SELECT/INSERT, which tables?" on complete statements in many dialects; the AST + `Visitor` make classification code compact. Accepts *more* than real servers in many places (permissive by design) and *less* in others — not suitable as the security authority. Not lossless, not error-tolerant, not incremental.

### 3.3 sqruff

- Rust port of SQLFluff by Quary; crates: `lib-core` (lexer, parser, segments), `lib-dialects` (27 dialects (corrected after verification; the list below has 27 entries): ansi, athena, bigquery, clickhouse, databricks, db2, doris, duckdb, exasol, flink, greenplum, hive, impala, mariadb, materialize, mysql, oracle, postgres, redshift, snowflake, sparksql, sqlite, starrocks, teradata, trino, tsql, vertica), `lib` (rules, linter, templaters: raw, placeholder, jinja, python, dbt, sqlmesh), `lsp`, `lineage`, `sqlinference`, `cli`, `lib-wasm`. [Observed: `crates/*`, `crates/lib-dialects/src/*`, `crates/lib/src/templaters/*`]
- **Lexer:** ordered list of `Matcher`s (`string`, `regex`, `native` fn, `legacy`), first match wins; `nested_block_comment` native matcher; a last-resort lexer keeps unlexable spans as tokens so they can produce violations (lossless). Regex engine `fancy-regex` + `regex-automata`. [Observed: `crates/lib-core/src/parser/lexer.rs:88-160,428-460,518-560`; `crates/lib-dialects/src/postgres.rs:446-570`; `crates/lib-core/Cargo.toml`]
- **Parser:** grammar combinators (`Sequence`, `AnyNumberOf/OneOf`, `Delimited`, `Bracketed`, `Ref`, `Conditional`, `NonCode`, `PrecededBy`) with terminators, `longest_match` with option pruning by first raw/type, bracket resolution, greedy matching to terminators, and a parse cache keyed by position + matcher (with care for terminator-sensitive grammars). [Observed: `crates/lib-core/src/parser/match_algorithms.rs:15-635`, `grammar/*.rs`]
- **Tree:** lossless `ErasedSegment` tree; unmatched regions become `Unparsable` segments; linting skips/handles them. [Observed: `crates/lib-core/src/parser/segments.rs:364,1006`]
- **Rules:** AL01–AL10, AM01–AM09, CP01–CP05, CV01–CV13, JJ01, LT01–LT15, OR01, PG01–PG02, RF01–RF07, ST01–ST12, TQ02–TQ04 (≈78). Fix mode loops up to `runaway_limit`. Layout fixes are done by a reflow engine (`utils/reflow/{depth_map,rebreak,reindent,respace,sequence}.rs`) rather than a Wadler printer. [Observed: `crates/lib/src/rules/*`; `crates/lib/src/core/linter/core.rs:684-740`; `crates/lib/src/utils/reflow/*`]
- **Assessment:** best open dialect coverage in Rust with losslessness; grammar-matcher architecture is backtracking-heavy and not designed for keystroke-latency incremental IDE work or for typed AST accessors. Good mine for keyword lists (`*_keywords.rs`) and lint rule semantics (Apache-2.0, attribution required; SQLFluff MIT upstream).

### 3.4 pg_query.rs / libpg_query

- **What libpg_query is:** the PostgreSQL 18.6 server parser (`gram.y` → bison, `scan.l` → flex), plus support code, extracted into a static C library, with outputs serialized as protobuf (upb vendored). [Observed: `libpg_query/src/postgres/include/pg_config.h:613-622`; `pg_query.rs/build.rs:11-60`]
- **API surface (Rust):** `parse` → `ParseResult` (protobuf AST, `nodes()` walker with `Context::{Select,DML,DDL,Call}`, `tables()`, `select_tables()`, `dml_tables()`, `ddl_tables()`, `functions()`, `call_functions()`, `statement_types()`, `deparse()`, `truncate()`); `deparse`, `deparse_comments_for_query`, `normalize` (constants → `$n`), `fingerprint` (structure hash ignoring constants/whitespace), `parse_plpgsql` (JSON), `split_with_parser`, `split_with_scanner`, `scan`/`scan_tokens` (token kinds incl. keyword category), `is_utility_stmt`, `summary`. [Observed: `pg_query.rs/src/query.rs:36-433`, `src/parse_result.rs:22-270`, `src/summary.rs:34`]
- **Keyword categories** (`kwlist.h`, PG 18.6): 495 keywords = 78 RESERVED, 23 TYPE_FUNC_NAME, 63 COL_NAME, 330 UNRESERVED (+1 header row artefact). [Observed: `libpg_query/src/postgres/include/parser/kwlist.h`]
- **Scanner split** (`pg_query_split_with_scanner`): counts parentheses; a statement ends at `;`/EOF at paren depth 0 if any keyword was seen; empty statements skipped. Does **not** understand `BEGIN ATOMIC … END` bodies (PG14+ SQL-standard function bodies with internal `;`) — only the parser-based split handles those. [Observed: `libpg_query/src/pg_query_split.c:60-120`] [Inferred: from the absence of any BEGIN/END depth tracking]
- **Limitations:** first-error abort (one error with cursor position); AST has `location` byte offsets for many nodes but not end positions; no comments/whitespace (comments recoverable through `scan`); C dependency (compiles ~100+ C files via `cc` — slower builds, cross-compilation via Clang/MSVC, WASM possible as pgls_wasm shows); thread-safety: memory contexts are thread-local [Inferred: libpg_query design; pgls calls it from multiple threads].
- **Licensing:** pg_query.rs MIT; libpg_query BSD-3-Clause; embedded PostgreSQL code under PostgreSQL License; vendored upb BSD-3, utf8_range MIT, xxHash BSD-2. All permissive; NOTICE/attribution needed in Hacivat's third-party notices. [Observed: `libpg_query/LICENSE`, `README.md:410-425`]

### 3.5 tree-sitter-sql (DerekStride) and tree-sitter in general

- "A general/permissive SQL grammar"; top rule: `program = repeat(seq(choice(transaction, statement, block), ';')) optional(statement)`. Extras: whitespace, `comment` (`/--.*/`), `marginalia` (C-style block comment regex — **not nested**). External scanner handles dollar-quoted strings and their tags. Generated `parser.c` is not committed on `main` (published on `gh-pages`, crate `tree-sitter-sequel`). [Observed: `grammar.js:1-96`, `src/scanner.c:18-190`, `README.md`]
- Tree-sitter's strengths: incremental reparsing via `Tree::edit` + `Parser::parse(text, Some(old_tree))`, robust ERROR/MISSING recovery, queries (`.scm`) for highlighting, `lookahead_iterator` for "what can come next". Weaknesses for Hacivat: one grammar per dialect is a large C artefact; ERROR nodes give little structure; GLR-ish conflicts make precise dialect grammars painful; nodes are untyped (strings); FFI/C build. [Inferred: tree-sitter docs + pgls usage]
- **Recommendation:** usable for *syntax highlighting fallback* only; Hacivat's own lexer gives highlighting tokens anyway, so tree-sitter is optional.

### 3.6 pgcli and mycli

- Both use `sqlparse` (Python, non-validating, grouping tokenizer) and a **last-token dispatch**: strip the partially typed word, parse text-before-cursor, isolate the current statement (`_split_multiple_statements`), isolate CTEs (`isolate_query_ctes` turns earlier CTEs into local table metadata; if the cursor is inside a CTE body, that body becomes the query), descend into function bodies delimited by `$…$`, then branch on the last token (`select`, `where`, `from`, `join`, `on`, `set`, `::`, `(`, `,`, `=`, `and`, `or`, `as`, `table`, `function`, `column`, `schema`, `database`, …) to produce **suggestion types**: `Special, Database, Schema(quoted), FromClauseItem, Table, View, JoinCondition, Join, Function(usage), Column(table_refs, require_last_table, local_tables, qualifiable, context), Keyword(last_token), NamedQuery, Datatype, Alias, Path`. [Observed: `pgcli/packages/sqlcompletion.py:13-623`, `parseutils/ctes.py`]
- Ranking: `find_matches` builds priority tuples `(match_key, type_priority, prio, usage_count, prio2, lexical)`; fuzzy regex `.*?`-joined characters scoring `(-len(match), -start)`; exact first-word alias match gets `+∞`; keywords use strict substring with `−∞`. `PrevalenceCounter` counts names and keywords from executed queries for frequency bias. [Observed: `pgcompleter.py:327-494`, `packages/prioritization.py`]
- Joins: FK-based `Join` candidates with generated aliases (`generate_alias`: uppercase letters or initials after `_`), priority by table distance from cursor; `JoinCondition` = FK pairs (prio 2000) + same-name-same-type columns (1000 if integer type). [Observed: `pgcompleter.py:63-78,575-663`]
- mycli adds MySQL specifics: backticks, `SHOW`, `USE`, character sets/collations, **ENUM value suggestions inside quotes**, doubled-backtick handling. [Observed: `mycli/packages/completion/completion_engine.py:229-760`]
- Destructive warnings are **prefix-based** (`query_starts_with` after `sqlparse.format(strip_comments=True)`) plus an "unconditional update" check — trivially bypassed by e.g. `WITH x AS (DELETE …) SELECT …` and should not be imitated for security. [Observed: `pgcli/packages/parseutils/__init__.py:17-53`; `mycli/packages/utils/sql_utils.py:341-400`]
- License BSD-3: logic may be reimplemented; we rewrite in our own words/pseudocode.

### 3.7 @codemirror/lang-sql

- Lezer grammar is token-level: `Script { Statement { element* Semi }* Statement { element+ }? }` where `element` is a literal, identifier, composite identifier, keyword, type, operator, punctuation or a bracketed group. All lexing is in an external tokenizer. [Observed: `src/sql.grammar:1-61`]
- `Dialect` record: `backslashEscapes, hashComments, spaceAfterDashes, slashComments, doubleQuotedStrings, doubleDollarQuotedStrings, unquotedBitLiterals, treatBitsAsBytes, charSetCasts, plsqlQuotingMechanism, operatorChars, specialVar, identifierQuotes, caseInsensitiveIdentifiers, words`. [Observed: `src/tokens.ts:157-196`]
- Completion: resolves the node before the cursor, computes `parents` from dotted paths, scans the *statement's* tokens after `FROM` for `name alias` / `name AS alias` until an `EndFrom` keyword (`where group having order union …`), and completes from a nested `SQLNamespace` tree (schema → table → column). No clause awareness beyond that. [Observed: `src/complete.ts:44-120`]
- Lesson: an editor gets 80% of perceived quality from a correct dialect-aware **lexer** (highlighting, bracket matching, statement boundaries) — invest there first.

### 3.8 sqls (Go)

- Own tokenizer (`token/`), handwritten grouping parser (`parser/`), `parseutil` computes `SyntaxPosition` (`col_name`, `select_expr`, `alias_name`, `where_condition`, `case_value`, `table_reference`, `insert_column`, `insert_value`, `join_clause`, `join_on`, `unknown`) from the previous keywords before the cursor, then maps to completion types (`Keyword, Function, Column, Table, ReferencedTable, View, SubQuery, SubQueryColumn, Change, User, Schema, Join, JoinOn`). Sort order by item kind prefix. [Observed: `parser/parseutil/position.go:9-80`; `internal/completer/completer.go:20-212`]
- Includes subquery column extraction for derived tables. Comparable to pgcli; MIT.

### 3.9 rust-analyzer, rowan, salsa, biome, ruff (foundation candidates)

- **rust-analyzer parser:** grammar functions emit `Event`s (`Start{kind, forward_parent}`, `Finish`, `Token{kind, n_raw_tokens}`, `Error`), allowing `CompletedMarker::precede` for left-recursive constructs; tree building attaches trivia; errors are recovered with `err_recover(message, recovery: TokenSet)` which wraps a token into an `ERROR` node unless the token is in the recovery set. [Observed: `crates/parser/src/event.rs:22-62`, `crates/parser/src/parser.rs:279-310`]
- **Incremental reparse:** (1) if the edit is inside one token of a relexable kind (whitespace without newline change, comment, ident, string) and relexing yields the same kind, swap the green token; (2) else find the nearest ancestor with a `Reparser` (block-like) whose text stays balanced and reparse only it. [Observed: `crates/syntax/src/parsing/reparsing.rs:1-140`]
- **Completion marker:** completion reparses the file with `raCompletionMarker` inserted at the cursor and analyses that tree, while resolving names against the original file. [Observed: `crates/ide-completion/src/context.rs:32,790-830`]
- **rowan:** immutable, deduplicated green tree (kind + text length + children; tokens hold text) + lazily-materialised red `SyntaxNode` cursors with parent pointers and absolute offsets; `GreenNodeBuilder` with `NodeCache` for interning; `replace_with` for persistent updates. [Observed: `rowan/src/green/*`, `rowan/src/api.rs:16-130,297`, `rowan/src/green/builder.rs:14-30`]
- **salsa:** `#[salsa::input]`, `#[salsa::tracked]` functions/structs, `#[salsa::interned]`, durability, LRU on tracked fns, cancellation; RA uses LRU 128 on parse queries. [Observed: `salsa/src/*`; `rust-analyzer/crates/base-db/src/editioned_file_id.rs:13-31`, `base-db/src/lib.rs:203-230`]
- **biome_rowan:** fixed **slots** per node kind (absent children are `Slot::Empty{rel_offset}`), `SyntaxFactory::make_syntax` validates children against the grammar and converts invalid lists to **bogus** kinds; trivia stored as leading/trailing pieces on tokens; `ParseRecoveryTokenSet` eats tokens into a bogus node until a recovery-set token (optionally a line break). [Observed: `biome/crates/biome_rowan/src/syntax_factory.rs:1-60`, `green/node.rs:53-60`, `syntax/trivia.rs`; `biome/crates/biome_parser/src/parse_recovery.rs:53-70`]
- **ruff** builds a non-lossless typed AST with `TextRange`s, keeps the token stream separately, and attaches comments in a side table (leading/trailing/dangling) for its formatter. [Inferred: public ruff architecture (`ruff_python_parser`, `ruff_python_formatter` comments module); not cloned]
- Details and the recommended design are in `sql-tooling-03`.

### 3.10 sqlglot

- Python transpiler (31+ dialects) with per-dialect `Tokenizer` class attributes: `QUOTES, IDENTIFIERS, BIT_STRINGS, BYTE_STRINGS, HEX_STRINGS, RAW_STRINGS, HEREDOC_STRINGS, UNICODE_STRINGS, STRING_ESCAPES, IDENTIFIER_ESCAPES, NESTED_COMMENTS, DASH_COMMENT_REQUIRES_BOUNDARY, COMMENTS_TERMINATE_AT_NEWLINE_ONLY, VAR_SINGLE_TOKENS, KEYWORDS, …`. Example values: MySQL `IDENTIFIERS=['`']`, `STRING_ESCAPES=["'",'"','\\']`, `COMMENTS=['--','#',('/*','*/')]`, `NESTED_COMMENTS=False`, `DASH_COMMENT_REQUIRES_BOUNDARY=True`; T-SQL `IDENTIFIERS=[('[',']'),'"']`, `VAR_SINGLE_TOKENS={'@','$','#'}`; SQLite `IDENTIFIERS=['"',('[',']'),'`']`, `NESTED_COMMENTS=False`; ClickHouse `COMMENTS=['--','#','#!',('/*','*/')]`, `IDENTIFIER_ESCAPES=['\\']`, `STRING_ESCAPES=["'",'\\']`, `HEREDOC_STRINGS=['$']`. [Observed: `sqlglot/tokens.py:43-81`; `sqlglot/dialects/{mysql,tsql,sqlite,clickhouse,postgres,oracle}.py` Tokenizer classes]
- Design worth borrowing: dialect = data table + small hook set; a single generic tokenizer; transpilation via a dialect-neutral expression tree. MIT.

---

## 4. Comparison matrix

Legend: ✅ yes · ◐ partial · ❌ no. "Perf" is qualitative for a ~1 MB script on a laptop. [Inferred where not measured.]

| Tool | Lossless tree | Error-tolerant | Incremental | Dialects | Completion | Semantics / resolution | Perf | License | Rust-native | Maintenance (Nov 2026) |
|---|---|---|---|---|---|---|---|---|---|---|
| pgls | ◐ (tree-sitter CST lossless per statement; libpg_query AST not) | ✅ splitter + tree-sitter; libpg_query ❌ | ◐ statement-level, content-addressed | PostgreSQL only | ✅ rich, scored | ✅ catalog overlay, PG typing port, DB typecheck | Fast (per-statement) | MIT | ✅ (+C libpg_query, C tree-sitter) | Very active |
| sqlparser-rs | ❌ (values unescaped; spans partial) | ❌ first error aborts | ❌ | 16 | ❌ | ❌ (AST + Visitor only) | Very fast | Apache-2.0 | ✅ pure Rust | Very active (Apache DataFusion) |
| sqruff | ✅ segments incl. trivia, `Unparsable` | ✅ region-level | ❌ (whole file) | 26+ | ❌ (LSP = diagnostics/format) | ◐ rule-level reference checks | Medium (combinator matching) | Apache-2.0 | ✅ pure Rust | Active |
| libpg_query / pg_query.rs | ❌ (AST + start locations; tokens via `scan`) | ❌ | ❌ | PostgreSQL (exact, 18.6) | ❌ | ◐ tables/functions summary | Fast | MIT + BSD-3 + PostgreSQL | ◐ C core | Active (pganalyze) |
| tree-sitter-sql | ✅ (CST, ERROR nodes) | ✅ | ✅ (`Tree::edit`) | Generic/permissive | ◐ via lookahead | ❌ | Very fast | MIT | ❌ C runtime + Rust binding | Active |
| pgcli / mycli | ❌ (sqlparse token groups) | ✅ (non-validating) | ❌ | PG / MySQL | ✅ classic, FK joins | ◐ tables/aliases/CTEs | Slow-ish (Python) | BSD-3 | ❌ | Active |
| lang-sql (Lezer) | ✅ token tree | ✅ | ✅ (Lezer incremental) | Config-based (Std, PG, MySQL, MariaDB, MSSQL, SQLite, Cassandra, PL/SQL) | ◐ namespace + aliases | ❌ | Very fast | MIT | ❌ TS | Maintained |
| sqls | ❌ | ✅ grouping | ❌ | MySQL, PG, SQLite, MSSQL, Oracle, … (DB drivers) | ✅ position-based | ◐ subquery columns | Fast | MIT | ❌ Go | Low activity |
| sqlglot | ❌ | ◐ (`error_level` IGNORE/WARN) | ❌ | 31+ | ❌ | ✅ optimizer qualify/annotate types | Medium | MIT | ❌ (Rust tokenizer only) | Very active |
| rowan + custom parser (proposal) | ✅ | ✅ | ✅ (token / statement) | Ours | Ours | Ours (salsa) | Very fast | MIT/Apache | ✅ | Ours |

### 4.1 Requirement fit for Hacivat/Karagöz

| Requirement | Best existing | Gap |
|---|---|---|
| Keystroke-latency highlighting & bracket matching, all dialects | lang-sql *approach* (dialect-configured lexer) | Need Rust implementation → our `sql-lexer` |
| Statement boundaries for "run statement at cursor" | pgls splitter (PG); libpg_query scanner split | Multi-dialect, client commands (`GO`, `DELIMITER`, psql `\`), PL blocks → our splitter |
| Lossless CST with recovery for completion/format | rust-analyzer/biome pattern | Must write SQL grammars ourselves |
| Exact PG validation, fingerprint, normalize | libpg_query | C dependency; accept it behind a feature flag |
| Exact validation for MySQL/MSSQL/Oracle/ClickHouse | none open in Rust | Use server-side `PREPARE`/`SET NOEXEC`/`EXPLAIN` probes; sqlparser-rs as heuristic |
| Completion with scope & FK join inference | pgls + pgcli algorithms | Re-implement over our CST + catalog |
| Lint rule catalogue | sqruff (SQLFluff) + pgls analyser (+ Squawk ideas) | Port selected rules to our tree |
| Safety classification for Karagöz | libpg_query (PG), sqlparser-rs (others) + server enforcement | Fail-closed classifier + server-side guards |

---

## 5. Algorithm specifications

### ALG-SQL-LAND-01: Parser backend selection per (dialect, purpose)
- **Problem:** Several parsers with different guarantees exist; each consumer (highlighter, completion, formatter, Karagöz classifier) needs a defined, deterministic choice and fallback.
- **Inputs / outputs:** `(dialect: DialectId, server_version: Option<Version>, purpose: Purpose)` → ordered list of `Backend`s to consult and a combination rule.
- **Algorithm:**
  1. `Purpose::Highlight | BracketMatch | Fold` → `[OwnLexer]` only (never block on parsing).
  2. `Purpose::SplitForEditor` → `[OwnSplitter]`; for PG, when a fragment is ambiguous (blank-line heuristic, see ALG-SQL-SPLIT-01 step 9) consult `[LibPgQueryParseCheck]` asynchronously and apply the result when it arrives.
  3. `Purpose::Completion | Hover | Rename | Format` → `[OwnCst]` (rowan) + `[Catalog]`.
  4. `Purpose::Diagnostics` → `[OwnCst errors]` ∪ (PG: `[LibPgQuery]` first-error, which is authoritative and supersedes own syntax errors in the same statement) ∪ optional `[ServerPrepareProbe]` (opt-in, never on production connections without consent).
  5. `Purpose::SafetyClassify` (Karagöz and Hacivat "read-only mode") → PG: `[LibPgQuery]` mandatory; if it fails to parse → **Deny(ParseError)**. Other dialects: `[OwnCst strict mode]` and `[SqlparserRs]`; classify with both; **combine by taking the most dangerous class**; if either fails to parse → Deny unless the dialect policy allows "unparsed read" (never by default).
  6. `Purpose::AuditFingerprint` → PG: libpg_query `fingerprint` + `normalize`; others: own normalizer (literals → `?`, whitespace/comments collapsed, keywords upper) hashed with a stable hash (e.g. xxh3-64 of the normalized string).
- **Data structures:** `enum Backend { OwnLexer, OwnSplitter, OwnCst, LibPgQuery, LibPgQueryParseCheck, SqlparserRs, ServerPrepareProbe, Catalog }`; static table `BACKENDS[(dialect, purpose)]`.
- **Complexity:** table lookup O(1); cost dominated by chosen parsers.
- **Edge cases:** dialect unknown → Generic lexer, classifier Deny; server version newer than libpg_query (e.g. PG 19 syntax) → libpg_query parse error → Deny with explanation "syntax newer than the agent's parser; upgrade agent"; libpg_query feature disabled at compile time → PG treated like "other dialects" path; MySQL `sql_mode` affecting lexing (ANSI_QUOTES, NO_BACKSLASH_ESCAPES) → backend inputs must include session lexical mode; MariaDB vs MySQL differences → separate DialectIds; Aurora/CockroachDB/YugabyteDB/Redshift (PG-wire but not PG grammar) → own DialectIds, libpg_query only as heuristic, not authority.
- **Test cases:** `(Postgres, SafetyClassify)` with `SELECT 1` → `[LibPgQuery]` → Read; `(Postgres, SafetyClassify)` with `SELEC 1` → Deny(ParseError); `(MySql, SafetyClassify)` with `SELECT 1 /*!50000 , (SELECT 1) */` → both backends must see the executable comment as code (see doc 02) → Read; `(Cockroach, SafetyClassify)` with `SELECT 1` → OwnCst+sqlparser → Read; `(Unknown, SafetyClassify)` → Deny.
- **Prior art and evidence:** pgls dual parser (libpg_query + tree-sitter) [Observed: `ARCHITECTURE.md`]; pgls split oracle [Observed: `pgls_statement_splitter/src/lib.rs:117-140`].
- **Used by:** Hacivat | Karagöz

---

## 6. Implications for Hacivat / Karagöz / Perde

1. **Build** (clean, owned): `sql-lexer` (all dialects, table-driven), `sql-split`, `sql-syntax` (rowan kinds + typed AST), per-dialect parsers (PostgreSQL first, then MySQL/MariaDB, SQLite, SQL Server, ClickHouse, Oracle-basic), `sql-sema` (salsa: scopes, resolution, types-lite), `sql-ide` (completion, hover, diagnostics, folding, semantic tokens, format), `sql-safety` (classifier + policy). See `sql-tooling-03` §5 for crate layout.
2. **Reuse:**
   - `pg_query` crate (MIT) behind feature `pg-oracle` in `sql-safety` (Karagöz: always on) and in `sql-ide` (Hacivat: on by default on desktop; consider off for WASM builds).
   - `sqlparser` crate (Apache-2.0) behind feature `sqlparser-oracle` for non-PG classification second opinion and for quick table extraction in features not yet covered by our parsers.
   - `rowan`, `salsa`, `text-size`, `line-index`-like crate (or our own `pgls_line_index`-style module).
   - Do **not** depend on sqruff as a library (API churn, performance profile); port rule semantics.
   - Do **not** depend on tree-sitter for core features.
3. **Karagöz** links the same `sql-lexer`, `sql-split`, `sql-safety` crates (no IDE or salsa needed) so classification is identical on both sides; the client's classification is advisory (UX: "this will be blocked"), the agent's is authoritative.
4. **Perde** does not parse SQL on the data path. It may store fingerprints/normalized statements for audit search (computed by Karagöz).
5. **Versioning:** record the parser/grammar version (e.g. `pg_query 18.1.0 / PG 18.6`) in each Karagöz audit event so classification decisions are reproducible.

## 7. i18n considerations

- Diagnostics, lint messages, completion detail strings and safety-denial reasons must be message IDs with arguments (Fluent-style), never formatted English strings inside the analysis crates. libpg_query and server errors are English (or server-locale!) text: wrap them as `{ code: "pg-syntax-error", detail: <raw text> }` and show raw text as an untranslated detail.
- PostgreSQL error messages may arrive localized according to the server's `lc_messages`; do not pattern-match on message text — use SQLSTATE codes (pgls does pattern match one message text for `42601` "multiple assignments to same column" [Observed: `pgls_typecheck/src/lib.rs:103-111`] — avoid that pattern).
- Keyword casing in formatter/completion is ASCII-only: never use locale-aware case mapping (Turkish dotted/dotless i: `"insert".to_uppercase()` is safe in Rust because it is locale-independent, but any ICU/locale-based casing must not be used for SQL keywords or identifier folding). PG folds unquoted identifiers with ASCII rules (plus single-byte-encoding `tolower` for high-bit chars) [Inferred: PG `downcase_identifier` behavior].
- Column positions shown to users should be in characters (grapheme-aware rendering in the editor), while internal offsets are bytes; LSP-style UTF-16 conversion only at the protocol boundary.

## 8. Risks, pitfalls, open questions

- **Grammar effort:** writing faithful recovering grammars for 6 dialects is the dominant cost. Mitigation: PG first (largest user value, libpg_query oracle for differential testing), then share an ANSI core.
- **Parser differentials** between our classifier and the server are a security risk (doc 05). Mitigation: libpg_query for PG; server-side enforcement for all.
- **libpg_query lag:** a new PG major adds syntax months before pg_query.rs releases. [Unknown] typical lag; check release history before committing to support windows.
- **C toolchain** for pg_query in Karagöz's single-command install: ship prebuilt static binaries per target (musl, aarch64) — must verify `cc` cross-builds for each target. [Unknown] Windows-on-ARM support.
- **sqlparser-rs semantics drift:** permissive acceptance of syntax a server rejects is harmless for classification only if we fail closed on *our* parse failure, not on theirs succeeding.
- **Open:** do we need tree-sitter at all? Current answer: no for core; maybe for a future "inline grammar injections" feature (SQL inside other languages).
- **Open:** how much of pgls's PG typing port to replicate vs. depending on server PREPARE probes. Decide after Phase 3.
