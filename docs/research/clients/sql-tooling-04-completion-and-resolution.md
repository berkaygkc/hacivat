# SQL Language Tooling 04 — Completion, Scopes and Name Resolution

Series: `01` landscape · `02` lexing & splitting · `03` parsing architecture · **`04` completion & resolution** · `05` safety, lint, format.

This document specifies how Hacivat's SQL editor determines *what* to complete at the cursor, *which* names are in scope, how names resolve against layered catalogs (live schema snapshot, DDL earlier in the file, session state, local query scope), how JOIN conditions are inferred, and how candidates are ranked. The same scope/resolution machinery powers hover, go-to-definition, diagnostics ("unknown column"), and Karagöz's object-level authorization (which tables/columns a statement touches).

---

## 1. Scope and sources

| Source | Commit | License | Read |
|---|---|---|---|
| postgres-language-server `pgls_completions`, `pgls_treesitter`, `pgls_catalog` | `083c5d1bbf04` | MIT | Sanitization, context, scoring, filtering (speculative keyword validation), scope model, lookup tri-state |
| pgcli `pgcli/packages/sqlcompletion.py`, `parseutils/*`, `pgcompleter.py`, `prioritization.py` | `dc9ce9447e0f` | BSD-3-Clause | Suggestion types, last-token dispatch, CTE isolation, FK join suggestions, priority tuples |
| mycli `packages/completion/completion_engine.py`, `sql_completer.py` | `525f0315ef86` | BSD-3-Clause | MySQL specifics, enum-value suggestions |
| sqls `parser/parseutil/*.go`, `internal/completer/*.go` | `920fde7867c1` | MIT | Syntax positions, completion types, subquery columns |
| @codemirror/lang-sql `src/complete.ts` | `f5b0b0ba1811` | MIT | Alias scan, namespace tree completion |
| rust-analyzer `crates/ide-completion/src/context.rs` | `2a362a41a8eb` | MIT OR Apache-2.0 | Completion marker + original/fake tree pairing |
| PostgreSQL semantics references (`parse_clause.c`, `parse_target.c`, `parse_relation.c`) as linked from pgls | REL_18_6 | PostgreSQL License | FROM-item visibility, star expansion, USING/NATURAL merge [via pgls comments] |

---

## 2. Executive summary

1. **Context detection technique of choice: dummy identifier + tolerant parse.** Insert a marker identifier at the cursor (pgls `REPLACED_TOKEN`, RA `raCompletionMarker`), parse the *current statement* with the recovering parser, locate the marker node, and read its syntactic role (table ref, column ref, function name, type name, alias definition, keyword position…). Our own recovering parser (doc 03) makes this robust without a second grammar. [Observed: `pgls_completions/src/sanitization.rs:7-140`; `rust-analyzer/crates/ide-completion/src/context.rs:32,803-809`]
2. **Keyword completion from parser expectations, validated speculatively.** pgls collects keywords from tree-sitter's `lookahead_iterator(parse_state)` and then *validates* each candidate by injecting it at the cursor and reparsing (accepting only if it does not break a finished clause). Our parser can record its "expected set" at the marker position directly (the set of tokens `p.at()` checked there), which is cheaper and exact for our grammar. [Observed: `pgls_treesitter/src/context/mod.rs:535-553`; `pgls_completions/src/relevance/filtering.rs:537-640`]
3. **Token-based heuristics (pgcli/sqls) remain valuable as fallback** when the parse is too broken (e.g. cursor inside an `UNKNOWN_STMT`): dispatch on the last significant keyword before the cursor (`FROM`/`JOIN` → relations; `SELECT`/`WHERE`/`ON` → columns+functions; `::` → types; `SET` → columns (UPDATE) or settings). [Observed: `pgcli/packages/sqlcompletion.py:270-480`; `sqls/parser/parseutil/position.go:22-80`]
4. **Scopes must model SQL faithfully:** FROM items (tables, views, derived tables with column alias lists, table functions with column definition lists, `VALUES`, `unnest … WITH ORDINALITY`, `LATERAL` left-to-right visibility), CTEs (including recursive self-reference and CTE shadowing of tables), correlated subqueries (outer scopes), `JOIN … USING`/`NATURAL` merged columns (unqualified references unambiguous), select-list aliases visible only in `ORDER BY` (and `GROUP BY` in PG/MySQL, not in SQL Server), DML-specific scopes (`UPDATE … FROM`, `DELETE … USING`, `INSERT` target column list, `ON CONFLICT … EXCLUDED`, `RETURNING` with PG18 `old`/`new`, `MERGE` target/source visibility per `WHEN` branch). pgls's `resolve/scope.rs` encodes most of these and links the PG source functions. [Observed: `pgls_catalog/src/resolve/scope.rs:10-271`; `pgls_treesitter/src/context/mod.rs:229-340` (RETURNING aliases gated on server ≥ 18; MERGE hidden relation)]
5. **Name resolution is layered** — local query scope → statement-level (CTEs) → session (temp objects, `search_path`) → file DDL before the statement → live catalog snapshot — and **tri-state**: `Found | Missing | Unknown`. Only `Missing` (certain) produces "unknown X" diagnostics; `Unknown` (e.g. after a `DO` block, `CREATE EXTENSION`, dynamic SQL, no snapshot) is silent. [Observed: `pgls_catalog/src/lookup.rs:13-23`]
6. **Identifier folding and quoting are dialect rules** applied at resolution time: PG folds unquoted names to lower case, Oracle to upper, MySQL table-name sensitivity depends on `lower_case_table_names`, SQL Server on collation, ClickHouse is case-sensitive. Completion must insert the *minimal correct spelling*: unquoted when the stored name equals its folded form and is not a reserved word in that position, quoted otherwise.
7. **JOIN inference:** (a) after `JOIN` suggest `table alias ON alias.fk = other.pk` built from foreign keys between the new table and tables already in scope (both directions); (b) after `ON` suggest complete conditions from FKs (highest), then same-name columns (bonus for integer/uuid key-like types and `id`/`<table>_id` naming); prefer tables nearer the cursor. [Observed: `pgcli/pgcompleter.py:575-663`]
8. **Ranking = filter, then score, then stable tiebreak.** Filter by syntactic role (no tables in a column-only slot), qualifier consistency (unknown qualifier → no column items), visibility (MERGE branch), and fuzzy match; score with additive features: match quality (prefix ≫ word-boundary ≫ fuzzy), role fit, scope proximity (columns of tables in this query ≫ others), schema preferences (search_path order, `public`, penalise system schemas), object kind, usage frequency, key-ness for joins; tiebreak lexically with shorter first. [Observed: `pgls_completions/src/relevance/scoring.rs`; `pgcli/pgcompleter.py:327-494`; `prioritization.py`]
9. **"Mentioned but undefined" relations:** while typing `SELECT u.| FROM users u`, the FROM clause is *after* the cursor — scope building must use the whole statement, not just text before the cursor (pgcli uses `full_text` for tables, `text_before_cursor` for the trigger; pgls queries the whole statement range). [Observed: `sqlcompletion.py:93-107`; `context/mod.rs:229-236`]
10. **Insertion details matter more than ranking tweaks:** replace range = current identifier part (including an opened quote), keep user's case style for keywords, add alias qualification when ≥ 2 relations are in scope and the column name is ambiguous, function insertion with `()` and cursor inside (or signature snippet), `*` expansion as a code action, auto-generated aliases (`generate_alias`: capitals or initials after `_`). [Observed: `pgcompleter.py:63-78`]
11. **Catalog snapshot model:** schemas, relations (kind: table/view/matview/foreign/partitioned/temp), columns (type, nullability, default, generated/identity, PK/FK membership, comment), functions (signature, return type, kind: normal/aggregate/window/procedure, volatility, language, set-returning), types/domains/enums (+ enum labels for value completion — mycli does this for MySQL ENUM), sequences, indexes, constraints (FKs with column pairs), roles, settings (for `SET`), extensions, search_path. Introspection is per dialect and incremental (refresh one schema). [Observed: `pgls_catalog/src/snapshot/*` file list; mycli `_enum_value_suggestion`]
12. **Karagöz reuses resolution** (without completion) to compute the *set of objects touched* by a statement (relations read/written, functions called, columns accessed), the input for object-level authorization and audit. This must be conservative: anything `Unknown` is treated as "may touch anything" for deny decisions.

---

## 3. Detailed findings

### 3.1 Context-detection techniques compared

| Technique | Used by | How | Strengths | Weaknesses |
|---|---|---|---|---|
| Last-token dispatch on a grouping tokenizer | pgcli, mycli | Remove partial word; `sqlparse.parse(text_before_cursor)`; branch on last token (`Comparison`, `Where`, `Identifier`, keyword) and walk back with `find_prev_keyword` for `,`/`=`/`and`/`or` [Observed: `sqlcompletion.py:270-480`] | Works on any garbage; easy to extend; battle-tested | Shallow: no real scopes; ad-hoc rules per keyword; `type` keyword vs column named `type` needs special cases [Observed: `_is_datatype_keyword` lines 252-268] |
| Previous-keywords → syntax position | sqls | `CheckSyntaxPosition` matches previous nodes against keyword groups (`SET`/`ORDER BY`/`GROUP BY` → col_name, `SELECT`/`DISTINCT` → select_expr, `WHERE/HAVING/AND/OR/XOR` → where_condition, `AS` → alias…) [Observed: `position.go:22-80`] | Simple, compiled | Same shallowness |
| Flat token tree + FROM scan | lang-sql | Resolve node before cursor; parents from dotted path; aliases from tokens after `FROM` until `EndFrom` keyword [Observed: `complete.ts:44-95`] | Very fast, dialect-agnostic | No clause awareness; aliases only from top-level FROM |
| Dummy identifier + tree-sitter CST | pgls | Insert `REPLACED_TOKEN` (quote-aware), reparse statement, walk to node under cursor, collect clause/node kinds and queries for relations/aliases/columns [Observed: `sanitization.rs`, `context/mod.rs`] | Structural; clause-aware; handles quotes | Two grammars (tree-sitter + libpg_query) must agree; ERROR nodes lose structure |
| Fake ident + incremental reparse of own CST | rust-analyzer | `parse.reparse(empty_range_at(offset), "raCompletionMarker")`; analyse fake tree for syntax, original tree for semantics [Observed: `ide-completion/src/context.rs:790-830`] | One grammar; precise; cheap with incremental reparse | Requires a good recovering parser (we will have one) |

**Decision:** RA-style marker on our own CST as the primary path; pgcli-style last-keyword fallback when the marker lands in `ERROR`/`UNKNOWN_STMT` regions.

### 3.2 pgls completion pipeline details worth copying

- **When to sanitize:** cursor between nodes, after a trailing space at end, before `;`, on a `.`, between/after parentheses, after an opened `"` or inside an unterminated quote. Inserting the marker inside an unterminated quoted identifier uses `REPLACED_TOKEN_WITH_QUOTE"` to close it. Cursor positions are byte offsets, adjusted forward to a char boundary. [Observed: `sanitization.rs:57-140`]
- **Qualifiers:** `(head, tail)` for `a.b.<cursor>` — tail is the immediate qualifier (`schema` for tables/functions; table-or-alias for columns), head the outer one (`schema` in `schema.table.col`). [Observed: `context/mod.rs:156-166`]
- **Mentioned relations/aliases/columns** collected over the statement range via queries; RETURNING `old/new` aliases only when the cursor is inside the RETURNING list and server ≥ 18; MERGE hides the target or source in `WHEN NOT MATCHED [BY SOURCE]` branches. [Observed: `context/mod.rs:229-340,590-660`]
- **Filtering rules** (`filtering.rs`): no completions right after `*`, or between two identifiers without separator; quoted literal nodes may be identifiers; columns in JOIN only after `ON`; only the left side of binary expressions in some clauses; keywords like `and`, `as`, `like` require a typed prefix; keyword validity by speculative parse. [Observed: `relevance/filtering.rs:116-640`]
- **Scoring weights** (see doc 01 §3.1.4) — a good initial calibration for our feature weights.

### 3.3 pgcli suggestion taxonomy (re-expressed)

`Keyword(last_token)`, `Special` (psql backslash), `Database`, `Schema(quoted)`, `FromClauseItem(schema, table_refs, local_tables)` (tables, views, set-returning functions), `Table`, `View`, `Join(table_refs, schema)`, `JoinCondition(table_refs, parent)`, `Function(schema, usage: from|signature|special|None)`, `Column(table_refs, require_last_table, local_tables, qualifiable, context)`, `Datatype(schema)`, `Alias(aliases)`, `NamedQuery`, `Path`, `TableFormat`. Notable rules [Observed: `sqlcompletion.py`]:
- `USING (` → columns present in more than one of the tables before the cursor (`require_last_table`).
- `INSERT INTO t (` → columns of `t` only (`context="insert"`).
- `ON` without qualifier → `Alias` + `JoinCondition`; `ON a.` → columns of `a` + join conditions with `a` as parent.
- `::` or `TYPE` (in DDL) → datatypes + tables (composite types) + schemas.
- `SET` as first token → settings keywords; `UPDATE t SET` → columns.
- `AS` → nothing (user is naming an alias).
- `(` in WHERE: `EXISTS (` → keywords (subquery); `= ANY(` → columns+functions+keywords.
- Inside `CREATE FUNCTION … $$ … $$` body → recurse into the body text.
- Inside a CTE body → complete as if that body were the query; earlier CTEs become local tables with their column lists.

### 3.4 Scope semantics checklist (PostgreSQL baseline + dialect deltas)

| Construct | Visibility rule | Dialect deltas |
|---|---|---|
| FROM list `a, b` | all items visible to SELECT/WHERE/GROUP/HAVING/ORDER; not to each other's expressions unless LATERAL | MySQL: comma joins have lower precedence than JOIN (`a, b JOIN c ON a.x` fails) |
| `JOIN … ON` | ON sees items of the join's left and right subtrees only (not later FROM items) | — |
| `JOIN … USING (c)` / `NATURAL` | merged column `c` (one copy, unqualified unambiguous); qualified `a.c`/`b.c` still valid; star expands merged columns first, then left, then right | MySQL/SQL Server similar; SQL Server has no USING/NATURAL |
| `LATERAL (subquery)` / table function in FROM | sees items to its left in the same FROM list | PG implicit LATERAL for functions; T-SQL `CROSS/OUTER APPLY` = lateral |
| Derived table `(SELECT …) AS d(x, y)` | columns = alias list or select-list output names; inner scope cannot see outer FROM items (unless LATERAL); PG requires alias before v16 [Inferred: PG16 made subquery alias optional] | MySQL requires alias |
| Table function `f(…) AS t(a int, b text)` | column definition list (record-returning) or function's OUT params / composite type | — |
| `unnest(a, b) WITH ORDINALITY AS u(x, y, n)` | ordinality column last | — |
| `VALUES (…) AS v(a, b)` | default names `column1…` | MySQL `VALUES ROW(…)` |
| CTE `WITH x(c1, c2) AS (…)` | visible to the main query and to later CTEs; `WITH RECURSIVE` makes `x` visible inside its own body (only in the recursive term); CTE name shadows tables of the same name | MySQL 8+/SQL Server/SQLite similar; SQL Server: recursion without RECURSIVE keyword |
| Data-modifying CTEs | `WITH d AS (DELETE … RETURNING *)` columns = RETURNING list | PG only |
| Correlated subquery | inner scope sees outer scopes (all levels) — innermost-first lookup | — |
| Select-list aliases | visible in ORDER BY (bare name); PG/MySQL also GROUP BY/HAVING (MySQL HAVING yes; PG GROUP BY yes, HAVING no); never in WHERE | SQL Server: ORDER BY only; SQLite: aliases visible broadly (even WHERE) |
| ORDER BY of set operation | only output column names/positions of the first branch | — |
| Window `WINDOW w AS (…)` | named windows visible in OVER of same level | — |
| `UPDATE t SET … FROM u WHERE …` | `t` + `u` visible; SET targets must be columns of `t` (unqualified) | MySQL multi-table UPDATE `UPDATE a JOIN b SET a.x=…`; SQL Server `UPDATE alias SET … FROM t alias JOIN …` |
| `DELETE FROM t USING u` | `t` + `u` | MySQL `DELETE a FROM a JOIN b`; SQL Server `DELETE a FROM a JOIN b` |
| `INSERT INTO t (cols) SELECT …` | target columns from `t`; the SELECT has its own scope (cannot see `t`) | — |
| `ON CONFLICT (…) DO UPDATE SET c = EXCLUDED.c` | `t` (or its alias) + pseudo-relation `excluded` | MySQL `ON DUPLICATE KEY UPDATE c = VALUES(c)` / new-row alias `AS new` (8.0.19+); SQLite `excluded` |
| `RETURNING` | target table columns; PG18 `old`/`new` row aliases (and `WITH (OLD AS o, NEW AS n)`) | SQL Server `OUTPUT inserted.*, deleted.*` |
| `MERGE INTO t USING s ON …` | `WHEN MATCHED`: t and s; `WHEN NOT MATCHED [BY TARGET]`: only s; `WHEN NOT MATCHED BY SOURCE`: only t | SQL Server same; Oracle MERGE |
| Triggers / rules | `NEW`, `OLD` pseudo-rows | MySQL `NEW.`/`OLD.`; SQL Server `inserted`/`deleted` tables; Oracle `:NEW`/`:OLD` |
| SQL function bodies | parameters by name (`p`) or position (`$1`), function-qualified `f.p` | — |
| PL/pgSQL | variables, `FOUND`, record fields, loop variables; SQL inside sees PL variables (name conflicts → `plpgsql.variable_conflict`) | T-SQL `@vars`; MySQL local vars |
| Column vs table-as-row | PG `SELECT t FROM t` → whole-row composite; `t.*` | — |
| System columns | `ctid`, `xmin`, `xmax`, `cmin`, `cmax`, `tableoid` (PG) visible when qualified/unqualified but not in `*` | Oracle `ROWID`, `ROWNUM`; SQLite `rowid`/`oid`/`_rowid_` unless shadowed |

[Observed for PG: `pgls_catalog/src/resolve/scope.rs:10-271`; Inferred for other dialects: vendor docs]

### 3.5 Catalog layers and identifier rules

Layers consulted in order (first `Found` wins; any `Unknown` on the path makes a miss `Unknown` instead of `Missing`):
1. **Local query scope** (FROM items, CTEs, outer query levels) — always certain for names it defines; unknown columns for items whose columns are unknown (e.g. function returning `record` without a column definition list).
2. **Statement context**: trigger `NEW/OLD`, function parameters, PL variables.
3. **Session state**: temporary objects created earlier in the session (`pg_temp` searched first for relations/types, never for functions unless explicit), `SET search_path`, `USE db` (MySQL/SQL Server), `ATTACH` (SQLite schema names), `SET SCHEMA`/`ALTER SESSION SET CURRENT_SCHEMA` (Oracle). Hacivat tracks executed session-changing statements per connection; for unsaved scripts, the file's own `SET`/`USE` statements before the cursor are simulated.
4. **File DDL before the statement** (`CatalogDelta`): CREATE/ALTER/DROP/RENAME applied in order (pgls overlay); after unmodelled statements (`DO`, `CREATE EXTENSION`, `EXECUTE` dynamic SQL, `CALL` to unknown procedures) the affected namespaces become `Unknown`.
5. **Live snapshot** from introspection (per connection; refreshed on demand and after DDL executed in Hacivat).
6. **Built-ins**: pg_catalog functions/types/operators, MySQL built-in functions, T-SQL system functions — shipped as versioned data (PG can also introspect `pg_catalog`).

Identifier matching rules per dialect (applied to `NameRef` text → lookup key):
- **PG:** unquoted → ASCII lower-case; quoted → exact; truncation to 63 bytes; relation lookup through `search_path` (implicit `pg_temp` first, `pg_catalog` first unless listed explicitly); functions resolved by name + argument types (overloads) — completion lists all overloads.
- **MySQL/MariaDB:** column/routine names case-insensitive; database/table names per `lower_case_table_names` (0 = exact, 1/2 = case-insensitive); default database from `USE`/connection.
- **SQL Server:** case-insensitive or -sensitive per database collation (read `DATABASEPROPERTYEX(db, 'Collation')`, check `_CS_`); default schema per user (`dbo` typically); 4-part names `server.db.schema.obj`.
- **SQLite:** case-insensitive ASCII; schemas `main`, `temp`, attached names; `temp` searched first.
- **ClickHouse:** case-sensitive; `db.table`; default database.
- **Oracle:** unquoted → upper case; schema = user; synonyms (private then `PUBLIC`) must be resolved; DB links `@link`.

### 3.6 Ranking signals (union of pgls and pgcli)

| Signal | Source | Suggested weight |
|---|---|---|
| Match quality: exact > prefix (case-insensitive) > word-boundary/camel/underscore initials (`ui` → `user_id`) > subsequence fuzzy | pgls Skim + prefix bonus; pgcli `-len(match), -start` | primary tier |
| Role fit (item kind expected at the position) | pgls clause table (−50 for misfit) | hard filter for impossible kinds; soft for plausible |
| In-scope proximity: columns of relations mentioned in this statement (+45 schema-exact / +30 name-only in pgls); relations already used in the file | pgls `check_relations_in_stmt` | high |
| Not-yet-mentioned columns preferred in SELECT lists | pgls snapshot `prefers_not_mentioned_columns` | low |
| Primary/foreign key columns in JOIN ON | pgls +20 PK | medium |
| Schema preference: search_path order; `public` slight +; system schemas (`pg_catalog`, `information_schema`, `pg_toast`, `sys`, `mysql`, `performance_schema`) − | pgls `check_is_user_defined` | medium |
| Object kind order for mixed lists: columns > tables > functions > keywords in expression positions; tables > schemas > functions in FROM | pgcli `prio_order`, pgls keyword −10 | medium |
| Usage frequency (names executed successfully in this connection, decayed) | pgcli `PrevalenceCounter` | low–medium |
| Noise penalties: names containing `migration`, `_old`, `backup`, partitions children (`*_p2024_01`) when parent exists | pgls migrations −15 | low |
| Length / lexical tiebreak (shorter first, then lexicographic, case-insensitive) | pgcli lexical priority | tiebreak |

---

## 4. Algorithm specifications

### ALG-SQL-COMP-01: Completion context detection
- **Problem:** Given a document and cursor offset, determine the syntactic role(s) of the text being completed, the replace range, the qualifier path, and the expected keyword set — robustly on incomplete SQL.
- **Inputs / outputs:** `(file, offset, trigger_char: Option<char>)` → `CompletionContext { stmt_id, replace_range, typed_prefix, quote: Option<QuoteKind>, qualifier: Vec<NamePart>, roles: Vec<Role>, expected_keywords: Vec<KeywordId>, clause: ClauseKind, in_invocation: Option<FnCallInfo>, scope_anchor: SyntaxNodePtr }` where `Role ∈ {RelationRef{allowed_kinds}, ColumnRef, QualifiedColumnRef, FunctionName{allowed: scalar|aggregate|window|setof|procedure}, TypeName, SchemaName, DatabaseName, RoleName, SettingName, IndexName, ConstraintName, SequenceName, NewName (no completion), AliasDef (no completion), EnumLiteral{type}, Keyword, ParamName, FileOrPath}`.
- **Algorithm:**
  1. Find the statement item containing `offset` (or the nearest preceding one if `offset` is in trailing trivia after it on the same line; if inside a comment → return no context, unless the comment is a hint/executable comment). Inside a string literal → only `EnumLiteral` (if the string is compared/assigned to an enum/`SET`-typed column) or `FileOrPath` (COPY/`\i`), else none. Inside a dollar-quoted function body → recurse into the child document with its own dialect/grammar (SQL or PL/pgSQL).
  2. Compute the **identifier part at cursor** using the dialect `ident_continue` predicate and quote handling: walk left from `offset` over identifier chars; if an opening quote char of the dialect precedes them with no closing quote → `quote = Some(kind)`; replace range = from part start (including the opening quote) to the end of the identifier part right of the cursor (so completing in the middle of a word replaces the whole word — configurable).
  3. **Qualifier path:** walk left from the part start over `.` and preceding name parts (quoted or not, skipping no trivia — `a . b` with spaces is still a path in SQL; allow whitespace around dots) → `qualifier = [db?, schema?, table?]`.
  4. **Marker parse:** build `text' = stmt_text[..rel_offset] + MARKER + stmt_text[rel_offset_end_of_part..]` where `MARKER` is a valid unquoted identifier in all dialects (e.g. `hacivat_c0mpl3t10n_mark`), or, when `quote` is open, `MARKER + closing_quote`; if the typed prefix is non-empty, replace it with the marker (keep prefix separately for matching). Parse `text'` with the tolerant parser, recording the parser's **expected-token set** at the marker position (instrument `p.at`/`p.at_kw` calls whose position equals the marker token index).
  5. Locate the marker token in the tree; derive `roles` from its ancestors:
     - parent `NAME_REF` under `FROM_ITEM`/`JOIN`/`UPDATE target`/`INSERT INTO`/`DELETE FROM`/`TRUNCATE`/`LOCK`/`ALTER TABLE`/`DROP TABLE` → `RelationRef{allowed_kinds}` (+ `SchemaName` if qualifier empty; + `FunctionName{setof}` in FROM).
     - under `COLUMN_REF` in an expression → `ColumnRef` (+ `FunctionName{scalar|aggregate|window per clause}`, + `Keyword`s from expected set, + `ParamName` if dialect has named params); qualified → `QualifiedColumnRef` (qualifier resolved later as alias/table/schema).
     - under `TYPE_NAME` (after `::`, `CAST(… AS`, column definitions) → `TypeName` (+ `SchemaName`).
     - under `ALIAS` / `NEW_NAME` positions (after `AS`, `CREATE TABLE <name>`, column definition names) → `AliasDef`/`NewName` → return empty list but keep `expected_keywords` only if the user typed a prefix matching a keyword.
     - `INSERT INTO t (` column list → `ColumnRef` restricted to `t`; `USING (` → columns common to both sides.
     - `SET <name>` (PG `SET`/`RESET`/`SHOW`) → `SettingName`; `SET ROLE`/`GRANT … TO` → `RoleName`; `USE` → `DatabaseName`.
     - Inside function call argument list → `in_invocation` with function name + argument index (for signature help and type-directed ranking).
  6. **Clause kind** = nearest ancestor clause (`SELECT_LIST`, `FROM`, `JOIN_ON`, `WHERE`, `GROUP_BY`, `HAVING`, `ORDER_BY`, `SET_CLAUSE`, `VALUES`, `RETURNING`, `MERGE_WHEN{branch}`, `DDL_COLUMN_DEF`, …).
  7. **Keywords**: `expected_keywords` from step 4 ∩ dialect keyword table; drop statement-start keywords unless at statement start; drop noisy keywords (`AND`, `OR`, `AS`, `LIKE`, `IN`, `IS`, `NOT`) unless the typed prefix is non-empty.
  8. **Fallback** (marker landed in an `ERROR`/`UNKNOWN_STMT` region or the expected set is empty): run token heuristics — previous significant keyword(s) before the part start: `FROM|JOIN|INTO|UPDATE|TABLE|TRUNCATE|DESCRIBE|COPY` → RelationRef; `SELECT|WHERE|AND|OR|ON|HAVING|BY|SET (non-first)|,|=|(|WHEN|THEN|ELSE` → ColumnRef+FunctionName; `::|TYPE (DDL)` → TypeName; `AS` → none; `USE|DATABASE` → DatabaseName; `SCHEMA` → SchemaName; first token → statement keywords. (pgcli logic re-expressed.)
  9. Return context; the scope is built from the **original** statement tree (not the marker tree) when the original parses the scope-relevant parts at least as well; otherwise from the marker tree (RA uses original for semantics).
- **Data structures:** marker constant; `ExpectedSet` bitset captured by the parser in "probe" mode; ancestry → role table per dialect.
- **Complexity:** one statement reparse (or incremental reparse of the containing node) per request; O(statement).
- **Edge cases (exhaustive list):** cursor at document start; cursor in whitespace after `FROM ` (empty prefix); cursor right after `.` (`u.|`); after `"` (`"My|`); inside closed quotes (`"My|Table"` → replace whole quoted ident); after `*` (`SELECT *|` → no completions except keywords after a space); between two identifiers without separator (`SELECT a b|` → alias def, no completion); after a complete statement's `;` (new statement → statement keywords); inside an empty statement `;|;`; inside comments/strings; inside `$$` function body (child doc); MySQL executable comment body (complete as SQL); psql `\d |` (special: relation names); T-SQL `[My|` bracket quote; trailing backslash; cursor in the middle of a keyword (`SEL|ECT`); typed prefix that equals a full keyword (`FROM|` — also offer `FROM` itself? show tables after a space only); `SELECT u.| FROM users u` (alias declared after cursor); multiple statements on one line; extremely long statement (use incremental reparse); dialect mismatch (file dialect differs from connection) — use file dialect for syntax, connection catalog for names; no connection (syntax-only completion: keywords + names from file DDL + CTEs/aliases).
- **Test cases (input with `|` cursor → roles / notable items):**
  1. `SELECT | FROM users` → ColumnRef+FunctionName; columns of `users` ranked first.
  2. `SELECT * FROM |` → RelationRef+SchemaName (+ set-returning functions).
  3. `SELECT * FROM public.|` → RelationRef in `public` only.
  4. `SELECT u.| FROM users u JOIN posts p ON …` → QualifiedColumnRef(u) → columns of `users` only.
  5. `SELECT * FROM users u JOIN posts p ON |` → JoinCondition candidates + aliases + columns.
  6. `INSERT INTO users (|` → columns of `users`, excluding generated `ALWAYS` identity unless `OVERRIDING`.
  7. `UPDATE users SET |` → columns of `users`.
  8. `SELECT x::|` → TypeName.
  9. `WITH a AS (SELECT 1 AS id) SELECT | FROM a` → column `id`.
  10. `SELECT "Us|` → relations/columns with quote preserved, replace range starts at `"`.
  11. `SET |` (PG) → setting names; `SET search_path TO |` → schema names.
  12. `MERGE INTO t USING s ON t.id = s.id WHEN NOT MATCHED THEN INSERT VALUES (s.|` → columns of `s` only; `t.` gives none.
  13. `SELECT * FROM t WHERE status = '|'` with `status` an enum → enum labels.
  14. `SELECT 1 |` → keywords from the parser's expected set after a select item (`FROM`, `AS`, `UNION`, `ORDER`, `LIMIT`, `INTO`, …), with `FROM` ranked first.
- **Prior art and evidence:** pgls sanitization/context/filtering [Observed: `sanitization.rs`, `context/mod.rs`, `filtering.rs:116-640`]; RA marker [Observed: `context.rs:790-830`]; pgcli last-token dispatch [Observed: `sqlcompletion.py:270-480`]; sqls positions [Observed: `position.go`].
- **Used by:** Hacivat

### ALG-SQL-SEMA-01: Scope builder
- **Problem:** Build, for a statement tree, the lexical scope structure that determines which names (relations, columns, CTEs, aliases, pseudo-rows, parameters) are visible at every expression position.
- **Inputs / outputs:** `(stmt: typed AST, dialect, catalog_view)` → `ScopeTree { levels: Arena<QueryLevel>, expr_scope: Map<SyntaxNodePtr, LevelId + clause>, ctes: Arena<Cte> }` with `QueryLevel { parent: Option<LevelId>, ctes_visible: Vec<CteId>, items: Vec<FromItem>, merged: Vec<MergedColumn>, output_names: Vec<OutputCol>, output_visible_in: ClauseSet, name_only: Vec<FromItem> /* old/new, excluded */, flags: {unmodelled_items, unknown_natural} }`, `FromItem { alias: Option<Name>, source: Table(RelationId) | Cte(CteId) | Derived(LevelId) | Function(FnRef) | Values | Unknown, columns: Option<Vec<ColumnInfo>>, schema_qualifier: Option<Name>, lateral: bool, position: u32, join_group: JoinTreeNodeId }`.
- **Algorithm:**
  1. `build_query(node, parent_level)`:
     a. If `WITH`: create CTE entries in order. For each CTE `c`: visible-CTE list for its body = earlier CTEs (+ itself if `RECURSIVE` and the body is a `UNION` whose second branch references `c`); recursively build the body level; `c.columns` = explicit column list if present, else the body's output names (first branch for set operations); data-modifying CTE → RETURNING output.
     b. New `QueryLevel L` with `parent = parent_level`, `ctes_visible` = outer visible CTEs + this WITH's CTEs (inner shadows outer by name).
     c. Process the FROM clause **left to right**, building a join tree: for each item compute its `FromItem` (resolve base relation name through layers — ALG-SQL-SEMA-02 — CTE names first, then catalog); derived tables/table functions are built as child levels whose parent is `L` **only if** `LATERAL` (or dialect-implicit lateral: PG function calls, T-SQL `APPLY`) — otherwise their parent is `L.parent` (they cannot see sibling items) — and with the restriction that a LATERAL item sees only items to its left (`position < self.position`).
     d. For each JOIN node: ON-condition expressions are attached to a *join-local* scope that sees only items in the join's subtree; `USING (cols)`: add `MergedColumn{name, from: [left_item, right_item], type: common type}`; `NATURAL`: merged columns = intersection of known column names (if any side unknown → `flags.unknown_natural`).
     e. Attach SELECT list, WHERE, GROUP BY, HAVING, WINDOW, QUALIFY to `L` (WHERE: no output names; GROUP BY: output names visible per dialect after input columns (PG prefers input column for GROUP BY ambiguity? PG: GROUP BY name resolves to input column first, then output name; ORDER BY prefers output name) — store per-clause preference order); ORDER BY of a simple SELECT: output names first, then input columns; ORDER BY of a set operation: output names/positions only.
     f. Compute `output_names` from the select list (`expr AS alias`, bare column → column name, function → function name (PG), `?column?` otherwise; `*`/`t.*` expansion recorded lazily).
     g. Subqueries inside expressions (scalar, `EXISTS`, `IN`, `ANY`) → child levels with `parent = L` (correlated access).
     h. Set operations: each branch is its own level with the same parent; the combined level's output names come from the first branch.
  2. DML statements:
     - `INSERT INTO t [AS a] (cols) {VALUES | query} [ON CONFLICT … DO UPDATE SET … WHERE …] [RETURNING …]`: target item `t` (alias `a`); the source query is built with `parent = None` (cannot see `t`); `ON CONFLICT` scope: `t` + name-only `excluded` (PG/SQLite) / MySQL new-row alias; RETURNING scope: `t` + PG18 `old`/`new` name-only items.
     - `UPDATE t [AS a] SET … FROM items WHERE … RETURNING …`: level with `t` + FROM items; SET targets resolve only against `t`'s columns.
     - `DELETE FROM t USING items …`: similar.
     - `MERGE INTO t USING s ON …`: level with `t`, `s`; each `WHEN` branch gets a view hiding `t` (`NOT MATCHED [BY TARGET]`) or `s` (`NOT MATCHED BY SOURCE`).
  3. DDL contexts: `CREATE TABLE … (col …, CHECK (expr))` → scope with the table being defined; `CREATE INDEX ON t (expr)` → `t`; `CREATE POLICY … USING (expr)` → policy table; `CREATE TRIGGER` / rule → `NEW`/`OLD`; `CREATE VIEW v AS query` → normal query; `ALTER TABLE t ALTER COLUMN c TYPE x USING expr` → `t`.
  4. Record `expr_scope` for every expression node (or compute on demand by walking ancestors to the nearest level + clause).
- **Data structures:** arenas with `u32` ids; columns behind `Arc` to share with catalog; small-vec for items.
- **Complexity:** O(statement size + Σ columns materialised); column lists materialised lazily (only when a column lookup or `*` expansion needs them).
- **Edge cases (exhaustive list):** CTE name equal to a real table (CTE wins inside the statement); CTE referenced before definition (error, still resolvable as table); recursive CTE referencing itself in the non-recursive term (error); nested WITH inside subqueries; CTE column list shorter/longer than body outputs (error, use list); derived table without alias (PG16+ allowed; others error); duplicate aliases at one level (error `DuplicateAlias`; resolution ambiguous); alias shadowing table name (`FROM users AS orders` — `orders.` refers to alias only; the base name `users.` no longer valid in PG); same table twice without alias (error in PG); `JOIN` nested with parentheses `(a JOIN b ON …) JOIN c ON …` with alias on the parenthesised join (PG `(a JOIN b USING (x)) AS j` hides a/b); `NATURAL JOIN` with no common columns (cross join); `USING` column missing on one side (error); LATERAL referencing a right-side item (error); `SELECT` without FROM; `VALUES` as top-level query; `TABLE t` statement (PG); `SELECT … INTO` (PG creates table; MySQL/T-SQL variables); window `PARTITION BY` using output alias (not allowed in PG); `GROUP BY 1` positional; `ORDER BY` expression using alias inside expression (PG: not allowed — only bare alias); HAVING with alias (MySQL yes, PG no); correlated reference to an outer alias of the same name as an inner column (inner wins); SQLite alias visibility in WHERE; system columns; whole-row references; set-returning functions in SELECT list; unknown function in FROM (columns unknown → `Unknown` lookups).
- **Test cases:**
  1. `SELECT a.x FROM t a JOIN u b USING (id) WHERE id = 1` → `id` resolves to merged column (no ambiguity).
  2. `SELECT * FROM t, LATERAL (SELECT t.x) s` → `t.x` inside LATERAL resolves; without LATERAL → error `MissingFromEntry` (certain).
  3. `WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM r) SELECT n FROM r` → `r` visible in its own recursive term; `n` resolves.
  4. `SELECT x AS y FROM t ORDER BY y` → ok; `… WHERE y > 0` → error (PG); SQLite → ok.
  5. `MERGE INTO t USING s ON t.id = s.id WHEN NOT MATCHED THEN INSERT VALUES (t.id)` → `t` hidden → error.
  6. `INSERT INTO t SELECT t.x FROM u` → `t` not visible in source query → error.
  7. `UPDATE t SET x = u.y FROM u WHERE t.id = u.id` → resolves.
  8. `SELECT (SELECT t.x FROM u) FROM t` → correlated `t.x` resolves to outer.
- **Prior art and evidence:** pgls `resolve/scope.rs` (items, merged columns, output names visibility, name-only items, star order, links to PG `transformFromClauseItem`/`ExpandAllTables`) [Observed: `pgls_catalog/src/resolve/scope.rs:10-271`]; pgcli CTE isolation [Observed: `parseutils/ctes.py:1-60`]; sqls subquery columns [Observed: `internal/completer/completer.go` `CompletionTypeSubQueryColumn`].
- **Used by:** Hacivat | Karagöz (object-access extraction)

### ALG-SQL-SEMA-02: Name resolution against catalog layers
- **Problem:** Resolve a `NameRef` path (1–4 parts) of a given expected kind at a scope position to a definition, with certainty levels, honouring dialect folding, search paths, overloads and layered catalogs.
- **Inputs / outputs:** `(path: [NamePart{text, quoted}], kind: Relation|Column|Function{arg_types?}|Type|Schema|Sequence|…, scope_pos, ScopeTree, CatalogView{session, file_delta, snapshot, builtins}, dialect)` → `Lookup<Resolved>` = `Found(Def{id, kind, layer, ambiguity: Vec<Def>})` | `Missing(reason)` | `Unknown(reason)`.
- **Algorithm:**
  1. Normalise each part: `key = quoted ? text_unescaped : fold(dialect, text)` (PG lower ASCII + truncate 63 bytes; Oracle upper; others: keep text, compare with dialect/collation-aware comparator).
  2. **Columns** (`[c]`, `[q, c]`, `[s, t, c]`, `[db, s, t, c]`):
     a. Walk levels from the innermost (at `scope_pos`) outward. At each level:
        - if qualified: find FROM items whose alias = `q` (or, when unaliased, base name = `q` and optional schema matches `s`); none → continue outward (correlated); found item with known columns → column lookup → `Found` or `Missing`; columns unknown → `Unknown`.
        - unqualified: merged columns first (USING/NATURAL); then all items with known columns containing `c`; count matches: 1 → `Found`; >1 → `Found` with ambiguity (diagnostic `AmbiguousColumn`); 0 and any item at this level has unknown columns or `unknown_natural` → `Unknown` (stop; do not look outward, because the unknown item might define it); 0 otherwise → continue outward.
        - clause-specific output names (ORDER BY/GROUP BY rules from ALG-SQL-SEMA-01) consulted in the dialect's preference order.
        - name-only items (`excluded`, `old`, `new`, `NEW`, `OLD`, `inserted`, `deleted`) only when qualified.
     b. Whole-row: unqualified name equal to an item alias with no column of that name (PG) → `Found(WholeRow(item))`.
     c. Exhausted all levels → statement context (function parameters, PL variables) → `Found`/continue; finally `Missing` unless any `Unknown` was encountered on the way.
  3. **Relations** (`[t]`, `[s, t]`, `[db, s, t]`):
     a. Unqualified: CTEs visible at the position (innermost first) → `Found(Cte)`.
     b. Search order: dialect search path (PG: `pg_temp` (relations/types only) → explicit `search_path` entries (with `$user` expansion) → `pg_catalog` implicitly first if not listed; MySQL: current database; SQL Server: user's default schema → `dbo`; SQLite: `temp` → `main` → attached in order; Oracle: current schema → private synonyms → public synonyms; ClickHouse: current database). For each schema in order, consult layers **file delta → snapshot** (delta can create, drop, or rename; a drop makes the name `Missing` in that schema even if the snapshot has it; an "unmodelled statement" marker for that schema makes it `Unknown`).
     c. Qualified: same but only in the given schema/db; unknown schema → `Missing` (certain) unless schema list itself is `Unknown`.
  4. **Functions:** collect candidates by name across the search path (all schemas, PG semantics — overloads across schemas compete by search-path order then best match); if argument types known (from light typing), apply dialect overload resolution (pgls ports PG `func_select_candidate` [Observed: `ARCHITECTURE.md` typing/]); otherwise `Found` with all candidates (completion shows overloads); variadic/default args considered.
  5. **Types**, **sequences**, **indexes**, **roles**, **schemas**, **settings**: straightforward lookups in their namespaces with the same layer rules.
  6. Return the first certain result; propagate `Unknown` when any layer on the path is `Unknown` and no `Found` occurred.
- **Data structures:** `CatalogView` = persistent (immutable, structurally shared — e.g. `im`/`rpds` maps) overlay chain; name indices per namespace keyed by folded name; comparator per dialect/collation.
- **Complexity:** O(levels × items) per column lookup with per-level hash indices; relations O(|search_path|).
- **Edge cases (exhaustive list):** quoted vs unquoted same text (`"Users"` vs `users`); PG identifiers > 63 bytes (truncated equality); Oracle quoted lowercase names (`"users"` ≠ `USERS`); MySQL `lower_case_table_names=1` with mixed-case DDL in file; SQL Server CS collation; ClickHouse case; schema named like a table alias (`s.t` where `s` is both an alias in scope and a schema — PG: in column context `s.t` means alias `s` column `t`; in relation context means schema `s`); 3-part column refs `schema.table.col`; 4-part `db.schema.table.col` (PG: db must equal current db); search_path with nonexistent schemas (skip); `$user` expansion; temp schema per session (`pg_temp_N`); objects created inside a transaction later rolled back in the file (the file simulation cannot know → treat ROLLBACK as invalidating deltas since the matching BEGIN → `Unknown`); `DROP … IF EXISTS`; `ALTER … RENAME`; `CREATE OR REPLACE VIEW` changing columns; `ALTER TABLE ADD COLUMN IF NOT EXISTS`; inheritance/partitions (children have parent's columns); views whose definitions are invalid (columns from snapshot); functions returning `SETOF record` without column list (columns Unknown); polymorphic functions (`anyelement`); `search_path` set by `SET LOCAL` inside a transaction in the file; synonyms pointing to synonyms (Oracle; loop guard); DB links (Unknown); MySQL `db.table` across databases; SQLite attached DB not attached in this session (Missing vs Unknown → Unknown if `ATTACH` appears in the file).
- **Test cases:** PG `search_path = app, public`, tables `public.users`, `app.users` → `users` resolves to `app.users`; file has `DROP TABLE app.users;` before → resolves to `public.users`; `"Users"` with only `users` existing → Missing; Oracle `select * from users` with table `USERS` → Found; MySQL lctn=0 with `Users` table and query `users` → Missing; after `DO $$ … $$;` in file → unknown relations in PG become Unknown (no diagnostic); `SELECT id FROM a, b` both having `id` → AmbiguousColumn; `SELECT x FROM f()` where `f` returns `record` → Unknown (no diagnostic).
- **Prior art and evidence:** pgls `Lookup` tri-state and overlay [Observed: `lookup.rs:13-23`, `catalog/overlay.rs`]; pgls search_path snapshot [Observed: `snapshot/search_path.rs`]; pgcli schema-qualified suggestions and search path filtering [Observed: `pgcompleter.py` `get_function_matches` filt].
- **Used by:** Hacivat | Karagöz

### ALG-SQL-COMP-02: JOIN and ON-condition inference
- **Problem:** Suggest complete join clauses after `JOIN` and complete conditions after `ON`, using FK metadata and naming heuristics.
- **Inputs / outputs:** `(context: role=RelationRef in JOIN or JoinCondition after ON, in-scope items left of the cursor with resolved relations, catalog FKs, columns, settings{generate_aliases, alias_map, qualify_style})` → `Vec<Candidate{text, kind: Join|JoinCondition, priority, synonyms}>`.
- **Algorithm:**
  1. **After `JOIN` (relation position):** let `L` = in-scope items before the cursor (ordered by distance: nearest first).
     a. For each item `l ∈ L` with a base relation `R_l`, for each FK `fk` where `R_l` is child or parent: the *other* relation `R_o` is a candidate. Build `ON` pairs for all FK column pairs (composite FKs → `AND`-joined).
     b. Alias for `R_o`: if `alias_map[R_o]` exists use it; else generated (`generate_alias`: uppercase letters of the name, or first letter + letters after `_`, lower-cased; ensure uniqueness against aliases in scope by appending digits). Use alias only when `generate_aliases` setting is on or `R_o` already appears in scope (self/second join requires alias).
     c. Qualification of `R_o`: schema-qualify if `R_o.schema` ∉ effective search path or differs from `R_l.schema` and `R_l` was written qualified.
     d. Text: `R_o alias ON alias.fk_col = l_ref.pk_col [AND …]`. Priority: `2·distance_rank(l) + (R_o already in scope ? 0 : 1)` inverted (closer & new tables first); synonyms include the alias-first form for matching (`u` matches `users u ON …`).
     e. Also offer plain relation names (normal RelationRef completion) — joins are additional candidates.
  2. **After `ON` (condition position):** let `r` = the item just joined (rightmost before `ON`), `L` = items left of it in the same join tree.
     a. FK conditions between `r` and each `l ∈ L` (both directions), composite FKs as `AND` chains → priority tier 1 (pgcli 2000).
     b. Name-match conditions: for each column `c` of `r` and each `l` having a column with the same name and *compatible type* → tier 2 (pgcli 1000 if integer-like); extend to key-like patterns: `r.l_id = l.id` and `r.id = l.r_id` where `l_id` matches `<l_singular>_id`/`<l>_id`/`<l>id` (case-insensitive, singularisation light: strip trailing `s`/`es`) → tier 2.
     c. Type-compatible PK/unique columns → tier 3 (low).
     d. If the user typed a qualifier (`ON u.`), restrict to conditions whose left side is `u`, and render without repeating the qualifier.
     e. Exclude conditions already present in the ON clause; for `USING (` offer common column names instead.
  3. Rank within tier by distance of `l` (nearest first), then FK naming confidence, then lexical.
- **Data structures:** FK index: `relation → Vec<Fk{child: (rel, cols[]), parent: (rel, cols[])}>`; column-name index per relation.
- **Complexity:** O(|L| × FKs + |cols_r| × |L|).
- **Edge cases:** self-referencing FK (`employees.manager_id → employees.id`) — requires distinct alias (`e2`/`m`); multiple FKs between the same pair (e.g. `created_by`, `updated_by` → both offered, labelled); composite FKs; FKs to unique (non-PK) columns; cross-schema FKs; FKs to/from views (none in catalog → name heuristics only); partitioned tables (FK on parent); MySQL MyISAM (no FKs → name heuristics); SQLite FKs from `PRAGMA foreign_key_list` (unnamed); CTEs/derived tables in scope (no FKs → name heuristics only, columns from output names); quoted identifiers in generated text; alias collisions with keywords (`or`, `as`, `is` from initials → choose next variant); alias collision with existing alias; `LEFT JOIN`, `CROSS JOIN` (no ON → no join suggestions after CROSS), `NATURAL JOIN` (no ON); `JOIN LATERAL`.
- **Test cases:** tables `users(id PK)`, `posts(id, user_id FK→users.id)`: `SELECT * FROM users u JOIN |` → `posts p ON p.user_id = u.id` first; `SELECT * FROM posts p JOIN |` → `users u ON u.id = p.user_id`; `… JOIN posts p ON |` → `p.user_id = u.id` first, then name-join `p.id = u.id` (tier 2? same name `id`, integer — offered lower); self-join `employees e JOIN |` → `employees m ON m.id = e.manager_id`; composite FK `(a,b)` → `x.a = y.a AND x.b = y.b`.
- **Prior art and evidence:** pgcli `get_join_matches`, `get_join_condition_matches`, `generate_alias` [Observed: `pgcompleter.py:63-78,575-663`]; pgls PK bonus in join ON [Observed: `scoring.rs:196-202`].
- **Used by:** Hacivat

### ALG-SQL-COMP-03: Completion ranking
- **Problem:** Order candidates from several providers (columns, relations, functions, keywords, joins, snippets) into one list that puts the intended item first as often as possible, deterministically.
- **Inputs / outputs:** `(context from ALG-SQL-COMP-01, candidates: Vec<Candidate{label, kind, detail, schema, relation, flags}>, user_history: UsageStats, settings)` → sorted `Vec<CompletionItem{label, insert_text, replace_range, kind, detail, sort_key, preselect}>` (truncated to N, e.g. 200, after sorting; providers may pre-limit catalog scans).
- **Algorithm:**
  1. **Hard filters:** drop candidates whose kind is not in `context.roles`; drop columns not belonging to the qualifier's resolved item(s) when a qualifier exists (unresolvable qualifier → drop all columns/relations and keep only keywords; pgls "skip"); drop MERGE-hidden items; drop system objects if `settings.hide_system` and prefix doesn't start with `pg_`/`sys`.
  2. **Match:** compute `match = matcher(prefix, label)` on case-folded ASCII (and also on `alias.label`, `relation.label` synonyms): `Exact` > `Prefix` > `WordPrefix` (initials across `_`/camel: `ui` → `user_id`) > `Substring` > `Fuzzy(score)`; no match → drop (empty prefix → all `Prefix`).
  3. **Score** (integers, additive; tune with a test corpus):
     - match tier base: Exact 1000, Prefix 800, WordPrefix 600, Substring 400, Fuzzy 200 + fuzzy_score/4;
     - role fit: primary role for the position +100 (e.g. ColumnRef in WHERE), secondary +0, keyword −20 unless prefix typed;
     - scope: column of an item in the current level +60; outer level +30; relation already mentioned in statement +20 (for relation completions in JOIN, a relation *not* yet mentioned +10); CTE names +40 in relation positions;
     - schema: in first search-path schema +15, later +10 − index, `public` +2, system schemas −40;
     - join tier (ALG-SQL-COMP-02): tier1 +150, tier2 +90, tier3 +30; minus 10 × distance rank;
     - PK/FK column in ON +20; NOT NULL/indexed columns in WHERE +5;
     - usage: + min(30, 10·log2(1 + count_recent)) with exponential decay (half-life 14 days) per connection;
     - noise: migration/backup/old/partition-child −15;
     - type compatibility (in invocation arguments or comparisons with a known-type left side): compatible +25, incompatible −25.
  4. **Sort** by `(score desc, label.len() asc, case-insensitive label asc, label asc, kind order)` → deterministic.
  5. **Presentation:** `preselect` the first item only if its tier ≥ Prefix and the gap to the second ≥ threshold (pgls `should_preselect_first_item` [Observed: `builder.rs:91`]); keyword casing follows user setting or the case of the typed prefix; identifiers inserted with minimal quoting (§3.5); qualify columns with alias when ambiguous across in-scope items or when `settings.always_qualify`; functions insert `name()` with cursor inside (snippet) and show signature detail; relations show kind/schema/row estimate in detail.
- **Data structures:** candidate arena; matcher with precomputed lowercase labels and word-boundary indices; `UsageStats` persisted per connection profile (local only, privacy).
- **Complexity:** O(C · L) for matching (C candidates, label length L); catalog providers should prefilter by first character / prefix index when C > 10k (large schemas: 50k+ columns).
- **Edge cases:** empty prefix in huge schemas (show scoped items first, cap others); prefix with quote; prefix containing `.`; Unicode labels (match on NFC-normalised lowercase? — no: SQL compares exact code points; for *matching* use simple case-folding of ASCII + Unicode lowercase for non-ASCII, never Turkish-specific folding); duplicate labels from different schemas (show schema in detail and qualify on insert if ambiguous); keyword equal to an object name (`user` table vs `USER` keyword — prefer object in identifier roles, insert quoted if reserved in that position); identical scores (deterministic tiebreak); user history poisoning by many generated names (cap).
- **Test cases:** prefix `us` in FROM with tables `users`, `user_settings`, `auth.users`, `pg_catalog.pg_user` → `users`, `user_settings`, `auth.users`, …, `pg_user` last; prefix `ui` in SELECT over `users(user_id, uid_legacy, email)` → `uid_legacy` (Prefix tier) before `user_id` (WordPrefix tier), `email` dropped; prefix `` (empty) after `WHERE` with `FROM orders o` → columns of `orders` first, then functions, then keywords; `SELECT count(|` → `*` first (special-case bonus +50 for `count`), then columns of in-scope items.
- **Prior art and evidence:** pgls scoring [Observed: `scoring.rs:1-488`]; pgcli priority tuples and lexical priority [Observed: `pgcompleter.py:327-494`]; pgcli PrevalenceCounter [Observed: `prioritization.py`]; sqls sort prefixes by kind [Observed: `completer.go:204-212`].
- **Used by:** Hacivat

---

## 5. Implications for Hacivat / Karagöz / Perde

1. `sql-sema` owns scopes and resolution (salsa queries `scopes(stmt, view)`, `resolve(stmt, view)`); `sql-ide` owns context detection, providers, ranking, insertion. Providers are small structs implementing `fn candidates(&Ctx, &mut Sink)`.
2. **Catalog introspection** (`sql-catalog`): one query set per dialect returning the common model; incremental refresh per schema; snapshot cached on disk per connection profile (encrypted at rest with the profile's key; contains no data rows) for instant completion on reconnect/offline.
3. **Karagöz object extraction:** `touched_objects(stmt) -> {reads: Set<ObjRef>, writes: Set<ObjRef>, calls: Set<FnRef>, columns: Map<ObjRef, Set<Col>>, unknown: bool}` from `resolve`; if `unknown` (dynamic SQL, unresolved function bodies, views not expanded) the policy must assume worst case. Views: Karagöz should either expand view definitions from the catalog for column-level policies or treat views as objects in their own right (simpler; matches DB privilege model).
4. **Perde** stores catalog-free policies (object names/patterns); Karagöz evaluates them with its local catalog. Perde never needs the parser.
5. Port pgcli/pgls completion test scenarios (snapshot names listed in `pgls_completions/src/snapshots/*`) into `sql-testkit` as behavioural specs (rewritten, not copied).

## 6. i18n considerations

- Completion item `detail`/`documentation` strings (e.g. "table · public · ~12k rows", "function · returns int") use message IDs with arguments; object comments from the database are shown as-is (user content).
- Matching must not apply locale-specific case mapping: Turkish users typing `i` must match `ID`/`id` (ASCII folding). For non-ASCII letters use Unicode *simple* case folding (one code point → one code point), never full folding (`İ` → `i̇` adds a combining dot and breaks prefix matching) and never Turkish tailoring (`I` ↔ `ı`). Add tests with `İşlem`, `ılık`, `ID`, `id`.
- Generated aliases from non-ASCII table names (`müşteriler` → `m`) must produce valid unquoted identifiers for the dialect, else fall back to `t1`, `t2`.
- Ranking by "lexical order" uses codepoint order on case-folded labels for determinism (not locale collation) — document this.

## 7. Risks, pitfalls, open questions

- **Large catalogs** (100k+ columns, multi-tenant schemas): provider prefiltering, lazy column loading per relation, and a cap on unscoped column suggestions are mandatory.
- **Stale snapshots** produce false "unknown table" diagnostics: tie diagnostics to snapshot age; refresh after Hacivat-executed DDL; tri-state lookups help.
- **Dialect scope deltas** (alias visibility, GROUP BY rules) are easy to get subtly wrong; build table-driven tests per dialect, verified against real servers.
- **Security:** catalog introspection queries run with the user's credentials; Karagöz-side introspection must use the agent's metadata role, not the user's session, and must not leak objects the user cannot access into completion (filter by privileges: `has_table_privilege`, etc.). [Open] whether completion through Karagöz should show only authorised objects (recommended: yes).
- **Open:** type inference depth — pgls ports PG's full overload resolution; Hacivat may start with "light typing" (column types, literals, casts, common functions) and rely on server `PREPARE` probes for precise checks (opt-in).
- **Open:** cross-statement session simulation (temp tables, `SET`) in scripts vs. the connection's actual session state — show which source a resolution came from in hover to reduce confusion.
