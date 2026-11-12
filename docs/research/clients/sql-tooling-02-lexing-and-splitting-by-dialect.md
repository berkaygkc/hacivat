# SQL Language Tooling 02 — Lexing and Statement Splitting by Dialect

Series: `01` landscape · **`02` lexing & splitting** · `03` parsing architecture · `04` completion & resolution · `05` safety, lint, format.

This document specifies the lexical layer shared by the Hacivat editor and the Karagöz classifier: per-dialect lexical rules (identifiers, quoting, literals, escapes, comments, numbers, operators, parameters, keywords), statement splitting for **execution** and for **incremental parsing**, client-side command layers (`psql` meta-commands, `DELIMITER`, `GO`, SQL*Plus `/`), and placeholder detection.

---

## 1. Scope and sources

| Source | Version / commit | License | Used for |
|---|---|---|---|
| libpg_query (PostgreSQL 18.6 `scan.l` → `src_backend_parser_scan.c`, `kwlist.h`, `pg_query_split.c`) | `27112f8af5de` | BSD-3 + PostgreSQL License | Authoritative PG lexical rules, keyword categories, scanner split |
| postgres-language-server `pgls_tokenizer`, `pgls_lexer`, `pgls_statement_splitter` | `083c5d1bbf04` | MIT | Rust PG tokenizer + heuristic splitter with parser oracle |
| datafusion-sqlparser-rs `src/tokenizer.rs`, `src/dialect/*` | `8a176d98b37b` (v0.64.0) | Apache-2.0 | Multi-dialect tokenizer feature flags |
| sqruff `crates/lib-dialects/src/*`, `crates/lib-core/src/parser/lexer.rs` | `c3d18575d6d4` | Apache-2.0 | Per-dialect lexer matchers, keyword lists |
| sqlglot `sqlglot/tokens.py`, `sqlglot/dialects/*.py` | `c4130f48d1bc` | MIT | Per-dialect tokenizer configuration tables |
| @codemirror/lang-sql `src/tokens.ts`, `src/sql.ts` | `f5b0b0ba1811` | MIT | Editor dialect record |
| tree-sitter-sql `grammar.js`, `src/scanner.c` | `4cd7224bfcf1` | MIT | Dollar-quote external scanner |
| Vendor documentation (PostgreSQL "Lexical Structure", MySQL "Language Structure", T-SQL "Identifiers"/"Reserved Keywords", SQLite "SQL Language Expressions"/"Keywords"/`sqlite3_complete`, ClickHouse "Syntax", Oracle "Basic Elements of Oracle SQL") | current public docs | n/a (facts) | Rules for dialects without an open source we can read here |

Facts taken from vendor documentation without source verification at a commit are tagged **[Inferred: vendor docs]** and listed in §7 as items to verify with conformance tests against real servers.

---

## 2. Executive summary

1. **One lexer engine, many dialect tables.** All six target dialects can be lexed by one hand-written, table-driven Rust lexer whose behaviour is controlled by a `LexDialect` record (quote kinds, escape rules, comment forms, nesting, numeric prefixes, parameter sigils, operator charset) plus *session lexical modes* (PG `standard_conforming_strings`; MySQL `ANSI_QUOTES`, `NO_BACKSLASH_ESCAPES`, `PIPES_AS_CONCAT`; T-SQL `QUOTED_IDENTIFIER`). sqlglot and lang-sql prove the table approach. [Observed: `sqlglot/tokens.py:43-81`; `lang-sql/src/tokens.ts:157-196`]
2. **The lexer must be total and lossless:** every byte belongs to exactly one token; malformed input yields tokens with `error` flags (e.g. `terminated=false`), never panics and never drops text. pgls documents the same requirement ("a panic on unexpected input takes down the server"). [Observed: `pgls_tokenizer/src/lib.rs:1-2`]
3. **Keywords are not token kinds at lexing time.** In PostgreSQL 330 of 495 keywords are UNRESERVED and usable as identifiers; T-SQL, SQLite (parser fallback) and ClickHouse (contextual keywords) are similar. Emit `Word` tokens with a looked-up `KeywordId` + category, and let the parser decide ("contextual keywords"). [Observed: `kwlist.h` counts]
4. **PostgreSQL specifics that break naïve lexers:** nested block comments; `$tag$` strings (tag must not follow an identifier character, `$1` is a parameter); string continuation across a newline; `E''` backslash escapes vs standard strings; `U&''`/`U&""` + `UESCAPE`; operator tokenization rules (`--` / `/*` inside operator; trailing `+`/`-` stripped unless the operator contains `~!@#%^&|`?`); PG16 numeric `0x/0o/0b` and `_` separators; PG15+ "trailing junk" errors. [Observed: `src_backend_parser_scan.c:8790-8870,9940-10040`]
5. **MySQL specifics:** `#` comments; `-- ` needs trailing whitespace (`1--1` is arithmetic); `/*! … */` and `/*!50700 … */` **executable comments are code**, MariaDB adds `/*M! … */`; backslash escapes in strings by default; `"` is a string unless `ANSI_QUOTES`; adjacent string literals concatenate; `0x` / `0b` prefixes are case-sensitive; `@user` / `@@system` variables; `$` is an identifier character. [Inferred: vendor docs; partially Observed: `sqlglot/dialects/mysql.py`]
6. **SQL Server specifics:** `[ident]` with `]]` escape; `"` depends on `QUOTED_IDENTIFIER`; nested block comments; `N''`; `0x` binary; money literals with currency prefix; `@var`, `@@fn`, `#temp`, `##global`; `GO` is a client batch separator, not SQL; statement terminators are optional so statement boundaries inside a batch are grammar-determined. [Inferred: vendor docs; Observed: `sqlglot/dialects/tsql.py`, sqlparser-rs `mssql.rs` nested comments]
7. **SQLite specifics:** identifiers in `"…"`, `[…]`, `` `…` ``; double-quoted strings may silently become string literals (DQS misfeature); five parameter forms (`?`, `?NNN`, `:a`, `@a`, `$a` incl. Tcl `::` and `(…)` suffix); unterminated `/*` runs to EOF legally; triggers contain `;` inside `BEGIN … END`. [Inferred: vendor docs; Observed: `sqlglot/dialects/sqlite.py`]
8. **ClickHouse specifics:** identifiers in `"` or `` ` `` with backslash escapes; strings only in `'` with backslash escapes; `#`, `#!`, `--` comments; heredoc `$tag$`; query parameters `{name:Type}`; `INSERT … FORMAT <fmt>` is followed by **raw data** that must not be lexed as SQL; keywords are contextual. [Observed: `sqlglot/dialects/clickhouse.py`; sqlparser-rs `clickhouse.rs` nested comments] [Inferred: vendor docs]
9. **Oracle basics:** unquoted identifiers fold to UPPER case; `q'[…]'` alternative quoting (+ `nq'…'`); `#` and `$` in identifiers; binds `:name`/`:1`; PL/SQL units terminated by `/` on its own line (SQL*Plus convention); SQL statements sent through OCI/JDBC must not carry the trailing `;` while PL/SQL blocks must. [Inferred: vendor docs; Observed: `sqlglot/dialects/oracle.py`, sqlparser-rs Oracle `Q'{…}'` tokens `tokenizer.rs:104-109`]
10. **Statement splitting has two consumers with different contracts.** *Editor splitting* must be total, fast, stable under typing and tolerant of garbage (statement-at-cursor, per-statement caching). *Execution splitting* must be exact or refuse: wrong splits change semantics (and in Karagöz, safety). Use the same token-level splitter, but execution mode fails closed on ambiguity and prefers server/engine splitting where available (SQLite `sqlite3_prepare` tail, libpg_query parser split, T-SQL batches).
11. **Client-command layers sit *above* SQL:** `psql` backslash commands and `:var` interpolation, MySQL `DELIMITER`/`\G`, `sqlcmd`/SSMS `GO [n]` and `:setvar`, SQL*Plus `/`, `&var`, `@file`, SQLite dot-commands. They must be recognised before dialect lexing (MySQL `DELIMITER $$` can make `END$$` lex as one identifier otherwise). Hacivat should support the subset that users paste from those tools and execute them client-side or reject them clearly.
12. **Block constructs that contain `;`** and must be tracked by the splitter: PG `BEGIN ATOMIC … END` and `CREATE RULE … DO (…; …)`; MySQL stored-program bodies (`BEGIN … END`, `IF … END IF`, `CASE … END CASE`, `LOOP/WHILE/REPEAT … END x`); SQLite `CREATE TRIGGER … BEGIN … END`; T-SQL `BEGIN … END`/`BEGIN TRY … END CATCH`; Oracle PL/SQL units. Dollar-quoted bodies are single tokens and need no tracking.
13. **libpg_query's scanner split does not track `BEGIN ATOMIC`**, only parentheses; its parser split does. pgls handles `BEGIN ATOMIC` explicitly. [Observed: `pg_query_split.c:60-120`; `pgls_statement_splitter/src/splitter/common.rs:205-230`]
14. **Parameter/placeholder detection is dialect- and context-sensitive:** PG `?` is a jsonb operator, `::` is a cast, `:=` is assignment, `arr[1:2]` is a slice; T-SQL `@x` is a local variable if declared in the batch; MySQL `@x` is a user variable (session state), not a bind parameter. Placeholders are only recognised in token positions where an expression may start.
15. **Keyword lists must be data, versioned per server version,** sourced from: PG `kwlist.h` (via libpg_query, permissive); MySQL `INFORMATION_SCHEMA.KEYWORDS` at runtime (do not copy GPL `sql/lex.h`); T-SQL reserved keyword doc table; SQLite `sqlite3_keyword_name()`; Oracle `V$RESERVED_WORDS`; ClickHouse: none fixed (contextual) — use sqruff/sqlglot lists as hints only.

---

## 3. Detailed findings

### 3.1 Token model common to all dialects

A token is `(kind: TokenKind, len: u32, flags: TokenFlags)`; text is a slice of the source (lossless). Proposed kinds (dialect-independent; dialects enable subsets):

| Group | Kinds |
|---|---|
| Trivia | `Whitespace`, `Newline` (with count of logical line breaks, CRLF=1 — pgls keeps counts for blank-line heuristics [Observed: `pgls_tokenizer/src/lib.rs:312-345`]), `LineComment{style: Dash/Hash/DoubleSlash}`, `BlockComment{nested_depth_reached, terminated}`, `ExecutableComment{vendor: MySql/MariaDb, version: Option<u32>}` (**not trivia**, see §3.3), `HintComment` (Oracle/MySQL `/*+ … */`, trivia for parsing but surfaced for formatting and classification) |
| Words | `Word` (unquoted identifier or keyword; attached `KeywordId?`), `QuotedIdent{quote: Double/Backtick/Bracket, terminated}`, `UnicodeQuotedIdent` (PG `U&"…"`) |
| Literals | `String{prefix: None/E/N/U&/B/X/Introducer(charset)/Q(delim)/NQ, quote: Single/Double, terminated}`, `DollarString{tag_len, terminated}`, `Number{kind: Int/Dec/Float/Hex/Oct/Bin, has_separators, suffix}`, `BitString`, `HexString`, `Money` (T-SQL) |
| Parameters & variables | `PositionalParam` (`$1`), `QuestionParam` (`?`, `?NNN`), `NamedParam{sigil: Colon/At/Dollar/Brace}` (`:a`, `@a`, `$a`, `{a:T}`), `UserVar` (MySQL `@a`), `SystemVar` (`@@a`), `ClientVar` (psql `:a`, `:'a'`, `:"a"`, SQL*Plus `&a`, sqlcmd `$(a)`) |
| Punctuation | `(` `)` `[` `]` `{` `}` `,` `;` `.` `..` `:` `::` `:=` `=>` |
| Operators | `Op` (maximal munch over the dialect operator charset with dialect rules) and pre-classified common operators (`=`, `<>`, `!=`, `<=`, `>=`, `||`, `->`, `->>`, `<=>`, `==`, …) |
| Client layer | `ClientCommand` (psql `\…`, sqlcmd `:…`, SQLite `.…`, MySQL `DELIMITER …`, SQL*Plus `/`, `GO`) — produced only by the client-command pre-pass (ALG-SQL-SPLIT-02) |
| Errors | `Unknown` (any char not valid in this dialect), plus `flags.error` on malformed tokens |

Design notes:
- **Keyword lookup** is a perfect hash or `phf` map over ASCII-uppercased word text; only for unquoted `Word`s. Each dialect × server version has its own table of `(KeywordId, category)`; categories are dialect-specific enums mapped to a common lattice `{Reserved, ReservedForTypeOrFunction, ReservedForColumnName, NonReserved}` (PG kwlist semantics) [Observed: `pgls_lexer_codegen/src/syntax_kind.rs:69-100` maps kwlist categories].
- **Case folding is ASCII-only** for keyword lookup (Turkish `ı/İ` must never enter keyword matching). Identifier folding is dialect-defined (§3.2–3.7) and done in `sql-sema`, not in the lexer.
- **Positions:** byte offsets (`u32`, `TextSize`) everywhere; line/UTF-16 column only in the LSP/editor adapter.

### 3.2 PostgreSQL (and psql)

Source of truth: `scan.l` as compiled in libpg_query (PG 18.6). [Observed: `libpg_query/src/postgres/src_backend_parser_scan.c`, header comments at lines ~8730-8870]

**Whitespace and newlines**
- Space, tab, LF, CR, form feed (and vertical tab in recent versions [Inferred: PG16 `scanner_isspace` change]); `\r\n` counts as two newlines to PG (irrelevant except for continuation rules). [Observed: scan.c comment "we accept either \n or \r as a newline"]
- `--` comment runs to the next newline or to end of input. [Observed: same comment block]

**Comments**
- `/* … */` **nest**: `/* a /* b */ c */` is one comment. Unterminated → error "unterminated /* comment". Interaction with operators: `+/*` lexes as `+` then comment start; `*/` inside an operator stops the operator. [Observed: scan.c ~8850-8865 comment; operator rule lines 9955-9972]

**Identifiers**
- Unquoted: start `[A-Za-z\200-\377_]`, continue `[A-Za-z\200-\377_0-9$]` (bytes ≥ 0x80 count as letters, so any non-ASCII UTF-8 sequence is an identifier character). [Observed: `pgls_tokenizer/src/lib.rs:10-19` quoting scan.l]
- Folded to lower case (ASCII A–Z; high-bit chars lowered only in single-byte encodings). Truncated to `NAMEDATALEN-1` = 63 **bytes** (NOTICE, truncation at a character boundary). [Inferred: PG `downcase_truncate_identifier`; pgls_pretty_print tracks truncated identifiers `identifiers.rs`]
- Quoted `"…"`: any char except NUL, `""` = one `"`, empty `""` is an error ("zero-length delimited identifier"). Case preserved.
- `U&"d\0061t\+000061"` Unicode escapes, optional `UESCAPE '!'` changing the escape char. [Observed: pgls tokenizer `UnicodeEscStr` lines 95-105; scan.c `<xui>` state]

**Strings**
- `'…'` with `''` escape. With `standard_conforming_strings = on` (default since 9.1) backslashes are literal; if `off`, backslash escapes apply in plain strings (and `backslash_quote` governs `\'`). Lexer needs the session GUC as input. [Observed: scan.c `standard_conforming_strings` at 8695, 9434-9461; `backslash_quote` 9659]
- `E'…'`: C-style escapes `\b \f \n \r \t`, octal `\o`–`\ooo`, hex `\xh`/`\xhh`, `\uXXXX`, `\UXXXXXXXX`, any other `\c` = `c`; `\'` allowed. Prefix may be `e`/`E`, no space between prefix and quote.
- `U&'…'` with `\XXXX`, `\+XXXXXX`, `UESCAPE`; error if `standard_conforming_strings` is off.
- `B'0101'` bit strings; `X'1FF'` hex bit strings (not bytea!). `N'…'` national character strings (lexed as string + NCHAR). 
- **Continuation:** `'foo'` *newline-containing whitespace (comments allowed)* `'bar'` = `'foobar'`; on the same line it is a syntax error. Applies to E/B/X/U& prefixed strings too (continuation segments have no prefix). [Observed: scan.c comments ~8800-8818 `{quotecontinue}`]
- **Dollar quoting:** `$tag$ … $tag$`, tag = optional identifier without `$` (cannot start with a digit), case-sensitive, no escapes. `$` immediately after an identifier character is part of that identifier (`a$b`, `x$$y$$` is an identifier). `$1` is a parameter. pgls additionally lexes `$name` (no closing `$`) as a named parameter for client-side templating. [Observed: `pgls_tokenizer/src/lib.rs:194-225`; tree-sitter-sql external scanner `src/scanner.c:53-160`]

**Numbers**
- `42`, `3.5`, `4.`, `.001`, `5e2`, `1.925e-3`. PG16+: `0x1F`, `0o17`, `0b101`, and `_` separators between digits (`1_000_000`, `0x_FF` invalid). PG15+: "trailing junk after numeric literal" errors for `123abc`, `0x`, `1_` (older versions lexed `123abc` as `123 AS abc`) — **version-dependent**. [Observed: `pgls_tokenizer/src/lib.rs:370-470` (prefixes); Inferred: PG 15/16 release notes for junk/underscore rules]

**Operators and special tokens**
- Operator chars: `+ - * / < > = ~ ! @ # % ^ & | ` ?`. Maximal munch with two rules: (1) an embedded `--` or `/*` ends the operator; (2) a multi-char operator may not end in `+`/`-` unless it contains one of `~ ! @ # % ^ & | ` ?` (`=-` → `=`,`-`; `?-` stays). After stripping, 1-char results that are "self" chars and 2-char `>= <= => <> !=` become their dedicated tokens (`!=` ≡ `<>`). [Observed: scan.c lines 9940-10040]
- Self chars: `, ( ) [ ] . ; : + - * / % ^ < > =`. Special: `::` (cast), `:=`, `=>` (named argument), `..` (PL/pgSQL ranges).
- Positional parameters `$n`; `$1abc` → "trailing junk after parameter" (PG15+).

**Keywords** — `kwlist.h` columns `(name, token, category, BARE_LABEL|AS_LABEL)`. PG 18.6: 495 keywords; 78 RESERVED (e.g. `all analyse analyze and any array as asc asymmetric both case cast check collate column constraint create current_* default deferrable desc distinct do else end except false fetch for foreign from grant group having in initially intersect into lateral leading limit localtime localtimestamp not null offset on only or order placing primary references returning select session_user some symmetric system_user table then to trailing true union unique user using variadic when where window with`), 23 TYPE_FUNC_NAME (`authorization binary collation concurrently cross current_schema freeze full ilike inner is isnull join left like natural notnull outer overlaps right similar tablesample verbose`), 63 COL_NAME, 330 UNRESERVED. The `BARE_LABEL`/`AS_LABEL` flag decides whether a keyword can be a column alias without `AS` — required for correct alias parsing and completion. [Observed: `libpg_query/src/postgres/include/parser/kwlist.h`]

**psql client layer** (not part of the server grammar)
- Backslash meta-commands from `\` to end of line (or to the next `\\`), arguments with their own quoting (`'…'`, backticks for shell), `\g`, `\gx`, `\gset`, `\gexec`, `\;` (literal semicolon without terminating), `\if/\elif/\else/\endif`, `\copy`, `\set`, `\i`. [Inferred: psql docs; pgls splitter treats `\…` to EOL as its own statement: Observed `common.rs:28-38,124-143`]
- Interpolation `:name`, `:'name'` (literal-quoted), `:"name"` (identifier-quoted), `:{?name}` (is-defined test). Not interpolated inside quotes or dollar quotes. [Observed: pgls tokenizer `NamedParamKind::ColonRaw/ColonString/ColonIdentifier` lines 150-182]
- `COPY … FROM STDIN;` is followed by inline data lines terminated by `\.` — splitter must swallow them as a data block.
- psql tracks `BEGIN ATOMIC` depth for `CREATE FUNCTION/PROCEDURE … BEGIN ATOMIC … END;` [Inferred: psqlscan.l since PG14].

### 3.3 MySQL and MariaDB

[Inferred: MySQL 8.4 / MariaDB 11 Reference Manuals "Language Structure"; cross-checked with Observed `sqlglot/dialects/mysql.py` Tokenizer and sqlparser-rs `mysql.rs`.]

**Session lexical modes** (`sql_mode`): `ANSI_QUOTES` (`"` quotes identifiers), `NO_BACKSLASH_ESCAPES` (backslash literal in strings), `PIPES_AS_CONCAT` (`||` concatenation instead of OR), `HIGH_NOT_PRECEDENCE`, `IGNORE_SPACE` (function name may be followed by space). MariaDB `sql_mode=ORACLE` changes syntax substantially (PL/SQL-like). The lexer and parser take `MySqlModes` as input; Hacivat reads `@@sql_mode` on connect and after `SET sql_mode` statements it observes.

**Comments**
- `#` to end of line.
- `-- ` only when the second dash is followed by whitespace or a control character (space, tab, newline, or end of input [Inferred]); `SELECT 1--1` is `1 - -1`. [Observed: sqlglot `DASH_COMMENT_REQUIRES_BOUNDARY = True`; sqlparser-rs `requires_single_line_comment_whitespace` hook `dialect/mod.rs:1812`]
- `/* … */` **not nested** (sqlglot `NESTED_COMMENTS = False`).
- **Executable comments** `/*! body */`: the server parses `body` as SQL. `/*!NNNNN body */` (5–6 digit version): executed only if server version ≥ NNNNN, otherwise treated as a comment. MariaDB: `/*M! body */`, `/*M!NNNNNN body */` executed only by MariaDB; MySQL ignores `/*M!`. [Inferred: vendor docs] → The lexer emits `ExecutableComment{vendor, version}` whose *body is re-lexed as SQL tokens* (nested token stream) when the gate is satisfied for the connected server; for safety classification it is **always treated as code** regardless of the version gate (fail closed, see doc 05).
- Optimizer hints `/*+ … */` directly after `SELECT`, `INSERT`, `REPLACE`, `UPDATE`, `DELETE` keywords. Trivia for parsing, preserved by formatter.

**Identifiers**
- Unquoted: `[0-9a-zA-Z$_]` and U+0080–U+FFFF; may start with a digit but cannot be all digits; `1e5` is a number, `1e5x`?? is an identifier (`1ex` identifier) — ambiguity rule: if it parses as a number prefix followed by identifier chars, MySQL treats it as an identifier. [Inferred: vendor docs "Identifiers may begin with a digit"]. 
- Backticks `` `…` `` with ` `` ` ` escape; `"…"` only under `ANSI_QUOTES`. Max 64 chars (aliases 256).
- Case sensitivity of database/table names depends on `lower_case_table_names` (0: case-sensitive storage & compare; 1: stored lowercase, compare case-insensitive; 2: stored as given, compared lowercase) and OS; column, index, routine names, column aliases are case-insensitive; table aliases follow table name rules. [Inferred: vendor docs]

**Strings**
- `'…'` and `"…"` (unless ANSI_QUOTES); doubled quote escape; backslash escapes `\0 \' \" \b \n \r \t \Z \\ \% \_` (unless `NO_BACKSLASH_ESCAPES`); unknown `\x` → `x`.
- **Adjacent literals concatenate**, even on the same line: `'a' 'b'` = `'ab'`, `'a' "b"`.
- National `N'…'`/`n'…'`; charset introducers `_utf8mb4'…'`, `_binary'…'`, `_latin1 X'4D'`, optional `COLLATE x`.
- Hex `X'4D79'` (even digit count required), `0x4D79` (**lower-case `x` only**, `0X…` is not hex); bit `b'101'`, `0b101` (lower-case `b` only). [Observed: sqlglot `HEX_STRINGS = [("x'", "'"), ("X'", "'"), ("0x", "")]`, `BIT_STRINGS` similar]

**Numbers** — integers, decimals, `.5`, `1.`, exponents; no underscore separators [Inferred].

**Variables and parameters**
- User variables `@name`, `@'quoted'`, `` @`quoted` ``, `@"quoted"`; system variables `@@name`, `@@GLOBAL.name`, `@@SESSION.name`, `@@LOCAL.name`; assignment `:=`. These are session state, not bind parameters.
- Bind placeholders `?` (binary protocol prepared statements, `PREPARE … FROM`).
- Stored-program local variables are plain identifiers.

**Operators** — `= <=> <> != < <= > >= + - * / % DIV MOD & | ^ ~ << >> && || ! -> ->> :=`, word operators `AND OR XOR NOT IS LIKE REGEXP RLIKE SOUNDS LIKE MEMBER OF`. `||` meaning depends on `PIPES_AS_CONCAT`.

**Keywords** — reserved vs non-reserved per version. Source of truth: `INFORMATION_SCHEMA.KEYWORDS(WORD, RESERVED)` (MySQL ≥ 8.0), which Hacivat can read from the live server; bundled fallback lists derived from the public reference manual tables. Do **not** copy `sql/lex.h` (GPL-2.0). MariaDB: no `KEYWORDS` table [Inferred]; use its "Reserved Words" doc page; sqruff has `mysql_keywords.rs`/`mariadb_keywords.rs` (Apache-2.0) as a starting point with attribution. [Observed: `sqruff/crates/lib-dialects/src/mariadb_keywords.rs`]

**Client layer (`mysql` CLI)**: `DELIMITER <string>` (rest of line; any string) changes the statement terminator; `\G` (vertical output) and `\g` terminate statements; `ego`, `go`; `\c` clears; `source file`. [Inferred: vendor docs]

### 3.4 SQL Server (T-SQL)

[Inferred: Microsoft Learn "Database identifiers", "Reserved keywords (Transact-SQL)", "SET QUOTED_IDENTIFIER", "sqlcmd utility"; cross-checked with Observed `sqlglot/dialects/tsql.py`, sqlparser-rs `mssql.rs` (`supports_nested_comments` → true at line 127).]

- **Identifiers:** regular identifiers start with a Unicode letter, `_`, `@` (local variable/parameter), `#` (local temp object), `##` (global temp object); continue with letters, digits, `@`, `$`, `#`, `_`. Delimited `[…]` (`]]` escape) or `"…"` when `QUOTED_IDENTIFIER ON` (default for ODBC/OLE DB/SSMS). Max 128 chars (116 for local temp tables). Case-sensitivity follows the database/server **collation** (`_CI_` vs `_CS_`) — affects resolution, not lexing.
- **Strings:** `'…'` with `''`; `N'…'` Unicode; when `QUOTED_IDENTIFIER OFF`, `"…"` is a string. A backslash immediately followed by a newline inside a string literal is a **line continuation** (both removed). No other backslash escapes.
- **Binary:** `0x…` (any digit count, `0x` alone is an empty binary). **Money:** `$12.34`, `€12`, `£1` etc. (currency symbol prefix). [Observed: sqlparser-rs has `supports_dollar_as_money_prefix` hook `dialect/mod.rs:1341`]
- **Comments:** `--` to end of line; `/* … */` **nested**.
- **Variables:** `@local`, `@@ROWCOUNT`-style system functions; `DECLARE @x int` creates a batch-scoped variable.
- **Placeholders:** ADO.NET/TDS RPC named `@p1`; ODBC `?`.
- **Batch separator `GO`** (sqlcmd, SSMS, Azure Data Studio — *not* T-SQL): must be alone on a line, case-insensitive, optional positive integer count (`GO 5` executes the batch 5 times), trailing comment allowed in SSMS [Inferred]. The separator word is configurable in SSMS. sqlcmd adds `:setvar`, `$(var)` substitution, `:r file`, `:connect`, `:on error`, `!!` shell.
- **Terminators:** `;` is optional for most statements but required before a CTE `WITH` that is not the first statement, after `MERGE`, and before `THROW` in some positions [Inferred]. Consequence: **statement boundaries within a batch are grammar-determined**; a token-level splitter can only guess them with statement-start heuristics.
- **First-in-batch statements:** `CREATE PROCEDURE/FUNCTION/TRIGGER/VIEW/SCHEMA/DEFAULT/RULE` must be the first statement in a batch — so execution splitting for T-SQL is by `GO`, not by statement.
- **Keywords:** "Reserved keywords (Transact-SQL)" table (~185 words) + ODBC reserved + future keywords; most non-reserved words are usable as identifiers.

### 3.5 SQLite

[Inferred: sqlite.org "SQL As Understood By SQLite", "SQLite Keywords", `sqlite3_complete`, `sqlite3_prepare_v2`; cross-checked with Observed `sqlglot/dialects/sqlite.py`.]

- **Identifiers:** unquoted `[A-Za-z_$\x80-…][A-Za-z0-9_$\x80-…]*`; quoted `"…"` (`""` escape), `[…]`, `` `…` ``. Keywords may be used as identifiers when quoted, and many keywords are accepted unquoted through the parser's `%fallback ID` mechanism. Case-insensitive for ASCII.
- **DQS misfeature:** a `"…"` token that does not resolve to a column is treated as a string literal (controlled by `SQLITE_DBCONFIG_DQS_DML/DDL`; CLI disables it in recent versions). Lexer: emit `QuotedIdent{Double}`; resolver may reinterpret.
- **Strings:** `'…'` with `''`; no backslash escapes. **Blobs:** `X'…'`/`x'…'` (even number of hex digits).
- **Numbers:** decimal, real with exponent, `0x` hex integers; digit separators `_` in recent versions (3.46+) [Inferred: release notes; verify].
- **Comments:** `--` to EOL; `/* … */` not nested; **an unterminated `/*` is legal and runs to end of input**.
- **Parameters:** `?`, `?NNN` (1..`SQLITE_MAX_VARIABLE_NUMBER`), `:AAAA`, `@AAAA`, `$AAAA` where `$` names may contain `::` namespace separators and a trailing `(…)` suffix (Tcl). Named parameters with the same name share an index.
- **Operators:** `|| * / % + - << >> & | < <= > >= = == != <> IS ISNOT IN LIKE GLOB MATCH REGEXP AND OR -> ->> ~ NOT`.
- **Keywords:** 147 keywords (count varies by compile options); runtime API `sqlite3_keyword_count()`, `sqlite3_keyword_name()`, `sqlite3_keyword_check()` — Hacivat can query the linked SQLite (rusqlite) to build the table at runtime.
- **Splitting:** `CREATE [TEMP] TRIGGER … BEGIN stmt; stmt; END;`. The engine itself splits: `sqlite3_prepare_v2` returns `pzTail` (rest of input) — exact for execution. `sqlite3_complete()` decides whether text ends a complete statement (handles triggers). The `sqlite3` CLI has dot-commands (`.tables`, `.read`) at line start.

### 3.6 ClickHouse

[Inferred: ClickHouse docs "Syntax", "Query parameters", "Formats"; Observed: `sqlglot/dialects/clickhouse.py` Tokenizer; sqlparser-rs `clickhouse.rs:156` nested comments true.]

- **Identifiers:** unquoted `[a-zA-Z_][0-9a-zA-Z_]*`; quoted `"…"` or `` `…` `` with backslash escapes and doubled-quote escapes. **Identifiers and most function names are case-sensitive**; SQL-standard keywords and some functions are case-insensitive.
- **Strings:** only single quotes; escapes `\b \f \r \n \t \0 \a \v \xHH \\ \'` and `''`. Heredoc `$tag$…$tag$` / `$$…$$` [Observed: sqlglot `HEREDOC_STRINGS = ["$"]`].
- **Numbers:** decimal, exponent, `0x` hex, `0b` binary, `inf`, `nan`; [Unknown] underscore separators.
- **Comments:** `--`, `#` (sqlglot also lists `#!`), `/* … */` nested (sqlparser-rs says yes; verify against server).
- **Query parameters:** `{name:Type}` substituted server-side (HTTP `param_name=…`, client `--param_name`). These are the native "bind parameters".
- **Data after `FORMAT`:** `INSERT INTO t FORMAT CSV\n1,2\n3,4` and `INSERT INTO t VALUES (…)…` — for `FORMAT`, the remainder of the query text is raw data in that format; the lexer must switch to a raw `InlineData` token after `INSERT … FORMAT <ident>` and the splitter must not split inside it. [Unknown] exact end-of-data rule in `clickhouse-client --multiquery` (believed: data runs to the end of the query text; verify).
- **Keywords:** no fixed reserved list; keywords are contextual and aliases sometimes require `AS` to disambiguate. Use sqruff `clickhouse_keywords.rs` as completion hints only.
- **Execution:** HTTP interface accepts one statement per request; native protocol one query per `Query` packet; `clickhouse-client --multiquery` splits client-side.

### 3.7 Oracle (basics)

[Inferred: Oracle Database SQL Language Reference "Basic Elements"; SQL*Plus User's Guide; Observed: `sqlglot/dialects/oracle.py` (`VAR_SINGLE_TOKENS = {"@", "$", "#"}`, `NESTED_COMMENTS = False`), sqlparser-rs `QuoteDelimitedStringLiteral` `tokenizer.rs:104-109`.]

- **Identifiers:** unquoted start with a letter, continue with letters, digits, `_`, `$`, `#`; folded to **UPPER** case; quoted `"…"` case-sensitive and may not contain `"` at all. Max 128 bytes (12.2+; 30 before).
- **Strings:** `'…'` with `''`; alternative quoting `q'[…]'`, `q'{…}'`, `q'(…)'`, `q'<…>'` or `q'X…X'` with any non-space delimiter (closing is `X'`); `N'…'`, `nq'…'`. No backslash escapes.
- **Numbers:** `25`, `+6.34`, `0.5`, `25e-03`, suffixes `f`/`F` (BINARY_FLOAT) and `d`/`D` (BINARY_DOUBLE).
- **Comments:** `--`, `/* … */` (not nested); hints `/*+ … */` and `--+ …`.
- **Binds:** `:name`, `:1`. SQL*Plus client: `&var`, `&&var` substitution (unless `SET DEFINE OFF`), `@file`, `@@file`, `/` alone on a line = execute buffer / end of PL/SQL block, `SET`, `SPOOL`, `REM`.
- **Splitting:** SQL statements end with `;` (client strips it; OCI/JDBC raise ORA-00911 if a plain SQL statement carries `;` [Inferred]); PL/SQL units (`DECLARE`, `BEGIN`, `CREATE [OR REPLACE] [EDITIONABLE|NONEDITIONABLE] PROCEDURE|FUNCTION|PACKAGE [BODY]|TRIGGER|TYPE [BODY]|LIBRARY`) contain `;` and end at `/` on its own line (or EOF); their trailing `;` (the `END;`) **must** be sent.
- **Keywords:** `V$RESERVED_WORDS(KEYWORD, LENGTH, RESERVED, RES_TYPE, RES_ATTR, RES_SEMI, DUPLICATE)` at runtime.

### 3.8 Cross-dialect lexical matrix

| Feature | PostgreSQL | MySQL / MariaDB | SQL Server | SQLite | ClickHouse | Oracle |
|---|---|---|---|---|---|---|
| Identifier quotes | `"` (`""`), `U&"…"` | `` ` `` (``` `` ```), `"` if ANSI_QUOTES | `[…]` (`]]`), `"` if QUOTED_IDENTIFIER | `"`, `[…]`, `` ` `` | `"`, `` ` `` (backslash esc) | `"` (no escape) |
| Unquoted folding | lower | none (table names: lower_case_table_names) | none (collation) | none (CI compare) | none (case-sensitive) | UPPER |
| Extra ident chars | `$` (not first), ≥0x80 | `$`, ≥U+0080, may start with digit | `@ # $ _`, `#`/`##` prefixes | `$`, ≥0x80 | — | `$ #` |
| String quotes | `'` | `'`, `"` (unless ANSI_QUOTES) | `'`, `"` if !QUOTED_IDENTIFIER | `'` | `'` | `'`, `q'…'` |
| Backslash escapes | only `E''` (or scs=off) | yes (unless NO_BACKSLASH_ESCAPES) | only `\`+newline continuation | no | yes | no |
| Prefixed strings | `E N U& B X` | `N`, `_charset`, `X`, `B` | `N` | `X` | — | `N`, `Q`, `NQ` |
| Adjacent string concat | only across newline | yes, any whitespace | no | no | no [Unknown] | no |
| Dollar/heredoc | `$tag$` | — | — | — | `$tag$` | — |
| Line comments | `--` | `#`, `-- ` (needs space) | `--` | `--` | `--`, `#`, `#!` | `--` |
| Block comments nested | yes | no (+ `/*!`, `/*M!` executable, `/*+` hints) | yes | no (unterminated OK) | yes (verify) | no (`/*+` hints) |
| Hex / bin numbers | `0x 0o 0b` (16+) | `0x`, `0b` (lower-case only) | `0x` (binary type) | `0x` | `0x`, `0b` | — |
| Digit separators | `_` (16+) | no | no | `_` (3.46+, verify) | [Unknown] | no |
| Bind params | `$n` | `?` | `@p`, `?` (ODBC) | `? ?N :a @a $a` | `{a:T}` | `:a`, `:1` |
| Session vars | `current_setting()` | `@u`, `@@s` | `@v`, `@@fn` | — | — | — |
| Client commands | psql `\…`, `:v` | `DELIMITER`, `\G`, `\g` | `GO [n]`, `:setvar`, `$(v)` | `.cmd` | (client) | `/`, `&v`, `@f` |
| Keyword source | `kwlist.h` | `I_S.KEYWORDS` | docs table | `sqlite3_keyword_*` | none (contextual) | `V$RESERVED_WORDS` |

### 3.9 Statement splitting — purposes and contracts

| | Editor splitting | Execution splitting (Hacivat "run") | Karagöz enforcement |
|---|---|---|---|
| Must be total | yes (garbage → statements anyway) | yes, but may return `Ambiguous` | yes |
| Must be exact | no (heuristics OK) | yes or refuse | yes or **deny** |
| Latency | per keystroke (< 1 ms for 100 KB after warm-up) | per run | per request |
| Stability under edits | important (cache keys) | n/a | n/a |
| Oracle available | libpg_query (PG, async) | server/engine splitting (SQLite tail, PG parser split, T-SQL batch) | libpg_query (PG) |

**Protocol facts that affect execution splitting** [Inferred: protocol docs]:
- **PostgreSQL** simple query protocol accepts multiple statements in one `Query` message and runs them in one *implicit transaction* unless the text contains explicit transaction control; one result per statement; error aborts the rest. Extended protocol (`Parse`) accepts **exactly one** statement ("cannot insert multiple commands into a prepared statement"). Karagöz must use the extended protocol or validate single-statement-ness. 
- **MySQL** accepts multiple statements in `COM_QUERY` only if the client set `CLIENT_MULTI_STATEMENTS`; otherwise a second statement is a syntax error. Karagöz must never enable it. Prepared statements (`COM_STMT_PREPARE`) accept one statement.
- **SQL Server** executes whole batches (`SQL_BATCH`), any number of statements; `sp_executesql` also accepts multi-statement text. A classifier must analyse the whole batch.
- **SQLite** `sqlite3_prepare_v2` compiles the first statement and returns the tail; `sqlite3_exec` runs all.
- **ClickHouse** one statement per HTTP request / native `Query` packet.
- **Oracle** OCI executes one statement or one PL/SQL block per call.

### 3.10 Splitting pitfalls (exhaustive checklist)

Lexical (solved by a correct lexer, listed because naïve splitters fail them):
1. `;` inside `'…'`, `"…"`, `` `…` ``, `[…]`, `$tag$…$tag$`, `q'[…]'`, `E'\';'` (escaped quote — PG `E''` backslash, MySQL default), comments (incl. nested PG/T-SQL comments, MySQL `#`).
2. `--` that is not a comment in MySQL (`1--1`), `#` that is an operator in PG (bitwise XOR) but a comment in MySQL/ClickHouse.
3. `$` ambiguity in PG: `$1` param, `$$` / `$a$` quote, `a$$b` identifier.
4. MySQL executable comments `/*!…; …*/` — contain code, possibly `;` (with multi-statements off this is a syntax error on the server; with DELIMITER-based clients it splits).
5. Unterminated string/comment/dollar quote: the rest of the file is one token; the splitter must not loop or panic and should keep earlier boundaries stable (ALG-SQL-SPLIT-01 step 10).
6. PG string continuation across newlines (no effect on `;`, but the "statement ends at line break" heuristics of pgls must not cut between `'a'` and `'b'`).
7. BOM at file start (U+FEFF) — trivia, not an identifier character.
8. NUL bytes — PG rejects; treat as `Unknown` token.
9. `\r`-only line endings (classic Mac) — newline in all dialects.

Structural (need the splitter's block tracking):
10. PG `CREATE FUNCTION/PROCEDURE … BEGIN ATOMIC … END` (internal `;`); `BEGIN;`/`BEGIN TRANSACTION`/`BEGIN ISOLATION LEVEL …` are transaction starts, not blocks. [Observed: pgls `common.rs:205-230` distinguishes these by lookahead]
11. PG `CREATE RULE r AS ON INSERT TO t DO ALSO (INSERT …; UPDATE …);` — parentheses contain `;`.
12. PG `DO $$ … $$;` and function bodies in dollar quotes — single token.
13. MySQL stored programs: `CREATE [DEFINER = u] PROCEDURE|FUNCTION|TRIGGER|EVENT … BEGIN … END` with nested `IF … END IF`, `CASE … END CASE`, `LOOP … END LOOP`, `WHILE … DO … END WHILE`, `REPEAT … UNTIL … END REPEAT`, labels `lbl: BEGIN … END lbl`; `IF(` function vs `IF` statement vs `IF EXISTS`/`IF NOT EXISTS`; `CASE` expression vs statement; `BEGIN` vs `BEGIN WORK`/`START TRANSACTION`.
14. SQLite `CREATE TRIGGER … BEGIN … END;` with `CASE … END` expressions inside.
15. T-SQL `BEGIN … END`, `BEGIN TRY … END TRY BEGIN CATCH … END CATCH`, `BEGIN TRAN[SACTION]` (not a block), `BEGIN DISTRIBUTED TRAN`, `BEGIN DIALOG`, `BEGIN CONVERSATION TIMER`; `IF … ELSE` without `BEGIN` (single statement bodies) — no `;` required anywhere.
16. Oracle PL/SQL units ending at `/`; a `/` inside an expression (`a / b`) is not a terminator unless alone on a line.
17. ClickHouse `INSERT … FORMAT X` inline data.
18. PG `COPY … FROM STDIN` inline data until `\.`.
19. Empty statements `;;`, comment-only "statements", whitespace-only input, final statement without `;`.
20. Statements whose leading keyword is ambiguous for "statement start" heuristics: PG `WITH` in `TIMESTAMP WITH TIME ZONE`, `WITH ORDINALITY`, `WITH CHECK OPTION`, `GRANT … WITH GRANT OPTION`, `WITH (fillfactor=…)`; `SELECT` in `GRANT SELECT`, `UNION SELECT`, `CREATE VIEW … AS SELECT`, `INSERT … SELECT`; `UPDATE` in `FOR UPDATE`, `ON CONFLICT DO UPDATE`, `ON UPDATE CASCADE`, `BEFORE UPDATE`, `WHEN MATCHED THEN UPDATE`; `DELETE` as hstore function `delete(…)`. [Observed: pgls exception lists `common.rs:226-345`]
21. Missing `;` between two statements on separate lines (user error): editor splitting should still separate them (pgls blank-line recovery + "each piece parses" oracle) [Observed: `pgls_statement_splitter/src/lib.rs:17-140`]; execution splitting must *not* silently separate them for PG (the server would see one statement and error) — report "possible missing semicolon".

Client-layer:
22. psql meta-commands mid-script; `\;`; `:var` inside SQL; `\gexec` executing result rows as statements (Karagöz: deny).
23. MySQL `DELIMITER` changing the terminator to arbitrary strings (`//`, `$$`, `;;`, `|`), including strings that are valid token prefixes (`END$$` must split as `END` + delimiter).
24. `GO` inside a string or comment is not a separator; `GO` as an identifier in a statement line (`SELECT go FROM t`) is not a separator; `GO 3` repeats.
25. SQL*Plus `/` vs division, `&` substitution inside strings (SQL*Plus substitutes even inside strings unless `SET DEFINE OFF` — Hacivat should not emulate).

---

## 4. Algorithm specifications

### ALG-SQL-LEX-01: Dialect-aware total lossless lexer
- **Problem:** Convert source text into a lossless token sequence for a given dialect and session lexical mode, never failing, fast enough for every keystroke.
- **Inputs / outputs:** `(text: &str, dialect: &LexDialect, modes: LexModes, start: LexState)` → `Vec<Token{kind, len, flags}>` (+ `end_state: LexState` for incremental use; + error list `(range, LexErrorCode)`). Concatenating token texts reproduces `text` byte-for-byte.
- **Data structures:**
  - `LexDialect` (static table): `ident_start(char)->bool`, `ident_continue(char)->bool`, `ident_quotes: &[(open, close, EscapeRule)]`, `string_quotes: &[(open, close, EscapeRule)]`, `string_prefixes: &[(prefix_ci, StringPrefix)]`, `dollar_quotes: bool`, `q_quotes: bool`, `line_comment_starts: &[LineCommentRule]` (incl. MySQL "`--` + whitespace" rule, `#`, `#!`), `block_comment_nesting: bool`, `executable_comments: Option<Vendor>`, `hint_comments: bool`, `numeric_prefixes: {hex, oct, bin, case_sensitive}`, `digit_separators: bool`, `numeric_suffixes`, `operator_chars: CharSet`, `operator_rule: Maximal | PgOperatorRule`, `param_sigils`, `var_sigils`, `money_prefix: bool`, `keywords: &'static KeywordTable`.
  - `LexModes`: `standard_conforming_strings`, `ansi_quotes`, `no_backslash_escapes`, `pipes_as_concat`, `quoted_identifier`, `server_version`.
  - `LexState` (for incremental relex; normally `Initial`): `Initial | InInlineData{format} | InCopyData` — all multi-line constructs (strings, comments) are single tokens, so only data-block modes need state.
- **Algorithm (per token, at byte position `p`):**
  1. If `state == InCopyData`: consume lines until a line equal to `\.` (inclusive) → `InlineData`; state `Initial`. If `InInlineData`: consume to end of statement data rule (dialect-defined; default EOF) → `InlineData`.
  2. `c = text[p]`. Whitespace run → `Whitespace`; newline run → `Newline{count}` (CRLF = 1, lone CR = 1).
  3. Comments (checked before operators):
     - `--`: if dialect requires boundary (MySQL), accept only if `text[p+2]` is whitespace/control/EOF; Oracle `--+` → hint. Consume to before `\n`/`\r` or EOF → `LineComment`.
     - `#` when enabled (MySQL/ClickHouse) → `LineComment{Hash}`; ClickHouse `#!` same.
     - `/*`: if `executable_comments` and next is `!` (MySQL) or `M!` (MariaDB): parse optional 5–6 digit version, find matching `*/` (non-nesting), emit `ExecutableComment{vendor, version, body_range}`; if `hint_comments` and next is `+` → `HintComment`; else block comment with depth counter if nesting enabled (`/*` inc, `*/` dec), non-nesting otherwise; EOF before close → `terminated=false` (+ error unless dialect allows, SQLite).
  4. String prefixes (case-insensitive, no space allowed): try longest of `U&'`, `E'`, `B'`, `X'`, `N'`, `NQ'`, `Q'`, `_charset'` (MySQL introducer only if the word is a known charset name **and** immediately followed by `'` or `X'`/`B'`), then fall through to identifier if no quote follows. For `Q'`: delimiter = next char; closing = matching bracket for `[{(<`, else same char; end at `close'`.
  5. Quoted identifiers / strings by quote char per `ident_quotes`/`string_quotes` and modes (MySQL `"` → string unless `ansi_quotes`; T-SQL `"` → ident unless `!quoted_identifier`). Scan with the EscapeRule: `Doubled` (`''`), `Backslash` (skip next char; MySQL unless `no_backslash_escapes`; PG `E''`; PG plain strings only if `!standard_conforming_strings`), `BackslashNewlineContinuation` (T-SQL), `None`. EOF → `terminated=false`. **PG continuation is not merged at lex level** (two `String` tokens + trivia); the parser joins them and validates "newline present".
  6. `$`: if `dollar_quotes` and previous token is not an identifier-continuation (i.e. `p` is a token start, which it is by construction) — read `$` + optional tag (ident_start then ident_continue minus `$`) + `$`: if well-formed → scan for the identical closing delimiter (byte compare, case-sensitive) → `DollarString{terminated}`. Else if digits follow → `PositionalParam` (PG: trailing ident chars → flag `junk` error ≥ PG15). Else if `$name` (SQLite/pgls-style) → `NamedParam{Dollar}` (SQLite: allow `::` and a trailing `(…)` suffix). T-SQL with `money_prefix` and digits → `Money`.
  7. Numbers: digit, or `.` followed by digit. Prefix check `0x/0o/0b` per dialect (respect case-sensitivity: MySQL lower-case only); separators `_` only between digits when enabled; fraction/exponent; Oracle suffix `f|d`; then if an identifier character follows: PG ≥15 → flag `trailing_junk`, MySQL → re-lex as `Word` from `p` if the dialect allows digit-leading identifiers and the whole run is not all digits; others → separate tokens.
  8. Parameters/variables by sigil (before operators): `?` + digits (SQLite/`?NNN`), `?` alone → `QuestionParam` (but PG: `?` is an operator char — emit `Op` and let ALG-SQL-PARAM-01 decide); `:` + ident (Oracle, SQLite, psql, pgls templating; not after `:` → `::`; not `:=`) → `NamedParam{Colon}`/`ClientVar`; `@@`+ident → `SystemVar`; `@`+ident/quote → `UserVar` (MySQL) or `NamedParam{At}` (T-SQL/SQLite); `{ident:Type}` (ClickHouse) → `NamedParam{Brace}`.
  9. Words: `ident_start` → consume `ident_continue` → `Word{kw: lookup(upper_ascii(text))}`. T-SQL `#`/`##` prefixed temp names start a `Word`.
  10. Operators: maximal munch over `operator_chars`; apply PG rule (stop at embedded `--`//`/*`; strip trailing `+`/`-` unless the operator contains one of `~!@#%^&|`?`); map known 1–3 char operators to dedicated kinds (`::`, `:=`, `=>`, `->`, `->>`, `<=>`, `||`, `<>`/`!=`, `<=`, `>=`, `==`, `..`).
  11. Punctuation `( ) [ ] { } , ; .`.
  12. Anything else → `Unknown` (one char, UTF-8 aware) + error.
- **Complexity / performance:** single pass O(n); no regex; ASCII fast paths with `memchr` for string/comment terminators; keyword lookup via perfect hash on ≤ 32-byte uppercase stack buffer. Target ≥ 200 MB/s on typical SQL (pgls/rustc_lexer class). Tokens stored as `(kind: u8/u16, len: u32)` SoA.
- **Edge cases (exhaustive list):** empty input; input of only a BOM; invalid UTF-8 (input is `&str` — enforce at file load; replace with U+FFFD and record); NUL char; lone `\r`; `--` at EOF without newline (PG: comment to EOF is correct); `/*/` (not closed); `/**/`; `/* /* */` nested unterminated (PG) vs closed (MySQL); `*/` without opener (operator chars `*` `/`); `'it''s'`; `''` empty string; `E'\''`; `E'\\'`; `'\'` under scs=on (complete string `\`) vs off (unterminated); `U&'\0041' UESCAPE '!'`; `$$`; `$$$$` (one empty dollar-quoted string); `$a$ $b$ $a$` (inner different tag is content); `$A$…$a$` (case mismatch, unterminated); `x$$y` (identifier); `$1`, `$12a`, `$`, `$ 1`; `1.`, `.1`, `1..2` (PG lexes `1` `..` `2` for PL/pgSQL ranges via a dedicated integer-followed-by-`..` rule, not `1.` + `.2`) ; `1e`, `1e+`, `0x`, `0xG`, `1_`, `1__0`, `0b2`; `123abc`; MySQL `1e5x`, `0X12`, `0x12`; T-SQL `$`, `$12`, `0x`; `@@`, `@`, `@1`; `#temp` vs `#` comment in MySQL vs `#` operator in PG; `->>` vs `->` `>`; `=-1` (PG: `=` `-` `1`); `@-` (PG custom operator `@-`); `*/` inside operator; very long operator (> 63 chars PG error); 10 MB single-line string (no quadratic behaviour); deeply nested comments (depth counter `u32`, no recursion); MySQL executable comment with nested `/*`; `/*!` unterminated; `/*!12345SELECT*/` (version immediately followed by code); `/*M!100100 … */`; `q'X…X'` with `X` = space (invalid); `nq'[…]'`; ClickHouse `{x:UInt8}` vs `{` object literal (map) — only when `{ident:` pattern; psql `:'x'` inside a string (not a var); `::text` vs `:text`.
- **Test cases (input → expected token kinds, `·` = whitespace):**
  1. PG `SELECT $$a;b$$;` → `Word(SELECT) · DollarString Semicolon`.
  2. PG `x$$y$$` → `Word("x$$y$$")`.
  3. PG `/* a /* b */ c */ SELECT` → `BlockComment · Word`.
  4. MySQL `/* a /* b */ c */` → `BlockComment("/* a /* b */") · Word(c) · Op(*) Op(/)`.
  5. MySQL `SELECT 1--1` → `Word · Number Op(-) Op(-) Number` (no comment: `--` is not followed by whitespace).
  6. MySQL `SELECT 1 -- c` → `… Number · LineComment`.
  7. MySQL `'a\'b'` → `String(terminated)`; with `no_backslash_escapes` → `String('a\')` `Word(b)` `String(unterminated)`.
  8. PG `'a\'b'` (scs=on) → `String('a\')` `Word(b)` `String(unterminated)`.
  9. PG `E'a\'b'` → `String{E}`.
  10. PG `=-1` → `Op(=) Op(-) Number`; `@-1` → `Op(@-) Number`; `+/*c*/1` → `Op(+) BlockComment Number`.
  11. T-SQL `[a]]b]` → `QuotedIdent{Bracket}` with value `a]b`.
  12. T-SQL (QUOTED_IDENTIFIER OFF) `"x"` → `String`.
  13. SQLite `/* open` → `BlockComment{terminated: false}` with **no error**.
  14. ClickHouse `SELECT {id:UInt32}` → `Word · NamedParam{Brace}`.
  15. Oracle `q'[it's]'` → `String{Q}`.
  16. MySQL `/*!50110 KEY_BLOCK_SIZE=1024 */` → `ExecutableComment{MySql, 50110}`.
  17. PG16 `1_000` → `Number{has_separators}`; PG15 dialect version → `Number(1)` + `Word(_000)` with junk error.
- **Prior art and evidence:** pgls tokenizer [Observed: `crates/pgls_tokenizer/src/lib.rs:58-616`]; PG scan.l rules [Observed: `src_backend_parser_scan.c:8730-10040`]; sqlglot tokenizer tables [Observed: `sqlglot/tokens.py:43-81`]; sqlparser-rs dialect hooks [Observed: `src/dialect/mod.rs:250-1830`]; lang-sql `Dialect` record [Observed: `src/tokens.ts:157-196`]; rustc_lexer cursor style [Inferred].
- **Used by:** Hacivat | Karagöz

### ALG-SQL-LEX-02: Incremental relexing after an edit
- **Problem:** On each keystroke, update the token vector without relexing the whole document.
- **Inputs / outputs:** old tokens (with cumulative offsets), edit `(range: old byte range, new_text)` → new token vector (spliced) + changed token index range.
- **Algorithm:**
  1. Find the first token `i` whose end ≥ `edit.start`; step back one more token (`i-1`) because the edit may merge with the previous token (`-` + `-` → comment, `$a` + `$` → dollar quote, word + word).
  2. Relex from `start = tokens[i-1].start` over the new text, with `LexState` from the token boundary (usually `Initial`).
  3. After each produced token whose end (in new coordinates) is ≥ `edit.start + new_text.len()`, compute the corresponding old position `old_pos = new_end - delta`. If `old_pos` is an old token boundary `j` **and** the lex state there equals the old state **and** the newly produced token's kind equals the old token kind at `j-1` (cheap sanity check), stop: splice `old[i-1..j]` with the new tokens.
  4. If relexing reaches EOF, replace the tail.
- **Data structures:** token kinds + lengths arrays; prefix-sum offsets maintained lazily (Fenwick tree or recompute from changed index; for ≤ 100k tokens recompute is fine).
- **Complexity:** O(size of changed region + resync distance). Worst case O(n) when the edit opens a string/comment/dollar quote (everything after changes) — acceptable; this is inherent.
- **Edge cases:** edit inside a multi-MB string; opening `'` then closing it on the next keystroke (two full relexes; fine); edit that changes a `DELIMITER` line (MySQL) — client-layer pre-pass rerun from that line (ALG-SQL-SPLIT-02); edit that changes `SET standard_conforming_strings` statement — lex modes are *document-wide inputs*; changing them invalidates everything after that statement (rare; recompute fully); CRLF split by edit (`\r` | `\n`); edit at offset 0; deletion of whole document.
- **Test cases:** typing `-` before `- comment` turns `Op(-)` into a comment; deleting the closing `$$` makes tail one `DollarString{unterminated}`; inserting `x` inside `SELECT` yields one changed token, resync immediately; inserting `'` at start of a 1 MB file relexes once to EOF.
- **Prior art and evidence:** rust-analyzer `reparse_token` checks single-token relex + "next char would merge" [Observed: `crates/syntax/src/parsing/reparsing.rs:44-90`]; Lezer/tree-sitter incremental lexing [Inferred].
- **Used by:** Hacivat

### ALG-SQL-SPLIT-01: Token-level statement splitter (editor and execution modes)
- **Problem:** Partition a token stream into statement ranges (plus client-command and data-block items) for a dialect, robust to incomplete code (editor) and exact-or-refuse (execution).
- **Inputs / outputs:** `(tokens, dialect, mode: Editor | Execution, delimiter: from client pre-pass)` → `Vec<Item>` where `Item = Statement{range, terminator: Option<range>, confidence: Exact|Heuristic} | ClientCommand{range, cmd} | Data{range} | Empty{range}` + diagnostics (`MissingSemicolonSuspected`, `UnterminatedBlock`, `AmbiguousBoundary`).
- **Data structures:** stack of open blocks `Vec<Block{kind: Paren|BeginAtomic|BeginEnd|IfEnd|CaseEnd|LoopEnd|WhileEnd|RepeatEnd|TryCatch|PlSqlUnit|RuleParens, opened_at}>`; per-dialect `StatementStartTable` with `(keyword, prev_token_exceptions, next_token_exceptions)` like pgls's lists; `BlockOpenerRules`.
- **Algorithm:**
  1. Skip trivia; if at a `ClientCommand`/`Data` item (from ALG-SQL-SPLIT-02), emit it and continue.
  2. Begin a statement at the first non-trivia token `s`. Determine the statement head (first 1–6 significant tokens) to decide body mode:
     - PG: `CREATE [OR REPLACE] FUNCTION|PROCEDURE … BEGIN ATOMIC` → block opened at `BEGIN ATOMIC`; `CREATE RULE` → parenthesis handles `;`.
     - MySQL/MariaDB: `CREATE [DEFINER=…] [AGGREGATE] PROCEDURE|FUNCTION|TRIGGER|EVENT` → *stored-program mode* (compound statement tracking enabled).
     - SQLite: `CREATE [TEMP|TEMPORARY] TRIGGER` → trigger mode (`BEGIN … END`).
     - T-SQL: always block-aware (`BEGIN`/`END`, `TRY`/`CATCH`).
     - Oracle: `DECLARE`, `BEGIN`, `CREATE [OR REPLACE] [EDITIONABLE|NONEDITIONABLE] PROCEDURE|FUNCTION|PACKAGE|TRIGGER|TYPE|LIBRARY|JAVA` → PL/SQL unit mode: terminator is a `/` alone on a line, not `;`.
  3. Walk tokens. Maintain the block stack:
     - `(`/`[` push Paren; `)`/`]` pop if top is Paren (mismatch → record diagnostic, do not pop other kinds).
     - Mode-specific openers/closers (only when enabled by step 2): `BEGIN` (not followed by `TRANSACTION|TRAN|WORK|ISOLATION|READ|DEFERRABLE|NOT|;|DISTRIBUTED|DIALOG|CONVERSATION`) push BeginEnd; MySQL statement-position `IF` (previous significant token ∈ {`;`, `BEGIN`, `THEN`, `ELSE`, `DO`, `LOOP`, `REPEAT`, label `:`, body start} and next is not `(`… but see edge cases) push IfEnd; statement `CASE`/expression `CASE` push CaseEnd; `LOOP`/`WHILE`/`REPEAT` push their kinds (but `END LOOP` etc. are closers); `END` pops the top block (consuming an optional following `IF|CASE|LOOP|WHILE|REPEAT|TRY|CATCH|label` token); T-SQL `BEGIN TRY`/`BEGIN CATCH` push TryCatch.
  4. Termination conditions when the stack is empty:
     - token is the active delimiter (`;` or client `DELIMITER` string) → close statement (terminator = that token);
     - Oracle PL/SQL unit: a `/` token that is the only significant token on its line → close;
     - EOF → close (no terminator);
     - **Editor mode only:** a *statement-start keyword* at a position allowed by `StatementStartTable` (pgls-style prev/next exception lists), or a blank line followed by a statement-start keyword when the dialect is not T-SQL → close previous statement *heuristically* (`confidence = Heuristic`, diagnostic `MissingSemicolonSuspected`).
     - **T-SQL (both modes):** statements are not split inside a GO batch for execution; for editor features, statement-start keywords at depth 0 split heuristically.
  5. Close statement: range = `[s, last non-trivia token]` (trailing comments on the same line attach to the statement for editor purposes; for execution the range excludes trailing trivia).
  6. If the stack is non-empty at EOF → `UnterminatedBlock` diagnostic; close at EOF.
  7. Empty statements (`;` immediately) → `Empty` items (skipped by execution).
  8. Execution mode post-check: any `Heuristic` boundary or `UnterminatedBlock` → return `Ambiguous` for the affected items; the UI asks the user (or, for PG, consults the parser split).
  9. **PG oracle refinement** (editor mode, async): for each statement containing a blank line or a heuristic boundary, run libpg_query `split_with_parser` (after normalizing named params to `$n`, as pgls does) on the fragment; keep the heuristic split only if the whole fragment fails to parse or every piece parses alone. [Observed: `pgls_statement_splitter/src/lib.rs:17-140`]
  10. **Stability rule** (editor mode): if the edit that produced this token stream introduced an unterminated string/comment/dollar quote at position `e`, keep the *previous* statement boundaries for items entirely after `e` as provisional (`confidence = Heuristic`) instead of merging the rest of the document into one statement, so per-statement caches and "run at cursor" for statements below remain usable while the user types the closing quote.
- **Complexity:** O(tokens) with constant-size lookbehind/lookahead; stack depth ≤ nesting.
- **Edge cases (exhaustive list):** all items of §3.10; `END` without block (pop nothing, diagnostic); `END;` closing an outer `BEGIN` in T-SQL followed by `END` keyword misuse; MySQL `IF (cond) THEN` (statement with parenthesised condition: rule: at statement position, `IF` is a statement unless the dialect context is an expression — decide by position, not by `(`); `IF(cond, a, b)` in a SELECT (expression position → no push); `CREATE TABLE IF NOT EXISTS` (not stored-program mode → no block tracking); `CASE` expression inside SQLite trigger body (push/pop balanced); labels `outer: LOOP … END LOOP outer;`; `END$$` with DELIMITER `$$` (handled by client pre-pass splitting before lexing); PG `BEGIN ATOMIC` with nested `CASE … END` inside the body (CASE increments, its END pops CASE not ATOMIC); PG procedure with `BEGIN ATOMIC END` (empty); `CREATE FUNCTION … AS $$ … $$ LANGUAGE sql;` (no block); PG `CREATE RULE … DO INSTEAD NOTHING;`; T-SQL `BEGIN TRANSACTION; … COMMIT;` (not a block); T-SQL `WAITFOR` / `RAISERROR` statements without `;`; Oracle `CREATE TABLE … ;` (plain SQL, `;` terminates) vs `CREATE TYPE t AS OBJECT (…);` (Oracle: object type spec is PL/SQL-unit-like and needs `/` in SQL*Plus [Inferred]); a `/` line inside a PL/SQL comment (ignored, token is comment); ClickHouse `FORMAT` data; PG `COPY FROM STDIN` data; statement consisting only of `\` meta-command; extremely long single statement (1 MB INSERT); unbalanced `)` at depth 0.
- **Test cases:**
  1. PG `select 1; select 2` → 2 statements (Exact).
  2. PG `create function f() returns int language sql begin atomic select 1; select 2; end; select 3;` → 2 statements.
  3. PG `begin; select 1; commit;` → 3 statements.
  4. PG editor mode `select * from t\n\nselect 1` → 2 statements (Heuristic, diagnostic MissingSemicolonSuspected); execution mode → Ambiguous.
  5. PG editor `select a,\n\n b from t` → 1 statement (oracle: whole fragment parses, pieces don't).
  6. MySQL `CREATE PROCEDURE p() BEGIN IF x THEN SELECT 1; END IF; SELECT 2; END; CALL p();` (delimiter `;`, stored-program mode) → 2 statements.
  7. SQLite `CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE b SET c = CASE WHEN 1 THEN 2 END; END; SELECT 1;` → 2 statements.
  8. T-SQL `SELECT 1 SELECT 2` (editor) → 2 statements (Heuristic); execution within one GO batch → 1 batch.
  9. Oracle `BEGIN NULL; END;\n/\nSELECT 1 FROM dual;` → 2 items: PL/SQL unit (text includes `END;`), SQL statement (terminator `;` stripped for sending).
  10. PG `select ';'; select $$;$$;` → 2 statements.
  11. PG `grant select on t to r; with x as (select 1) select * from x;` → 2 statements (no split at `SELECT` after `GRANT`).
  12. Unterminated `select 'abc` followed by 50 lines of statements, typed in the middle of a document → earlier and later boundaries retained provisionally.
- **Prior art and evidence:** pgls splitter structure and exception lists [Observed: `crates/pgls_statement_splitter/src/splitter/common.rs`, `data.rs`, `dml.rs`, `lib.rs`]; libpg_query scanner split (parens + keyword-seen) [Observed: `libpg_query/src/pg_query_split.c:60-120`]; SQLite `sqlite3_complete` trigger state machine [Inferred: sqlite docs]; psql BEGIN ATOMIC depth [Inferred].
- **Used by:** Hacivat | Karagöz (Execution mode only; Karagöz additionally requires `Exact` confidence and exactly one statement per request)

### ALG-SQL-SPLIT-02: Client-command pre-pass (psql, mysql, sqlcmd/SSMS, SQL*Plus, sqlite3)
- **Problem:** Scripts pasted from CLI tools contain non-SQL directives that change delimiters, substitute variables, repeat batches or carry inline data. They must be recognised *before* dialect lexing/splitting.
- **Inputs / outputs:** `(text, client_flavor: Psql|MySqlCli|Sqlcmd|SqlPlus|Sqlite3|None, dialect)` → segments `Vec<Segment{range, kind: Sql{delimiter} | Command{name, args} | Data | Batch{repeat}}>`.
- **Algorithm:**
  1. Scan line by line, but carry a *quote/comment context* computed by running ALG-SQL-LEX-01 restricted to strings/comments/dollar-quotes, so that directives inside strings or comments are ignored. (For MySQL, the lexer must be run with the *current delimiter* awareness: a delimiter occurrence outside quotes/comments terminates even if it would otherwise be part of a word token — this is why this pass works on characters, not dialect tokens.)
  2. psql: a line whose first non-blank char (outside SQL quotes, at statement start or anywhere outside quotes) is `\` → `Command` to EOL (or to the next `\\`); `\;` inside SQL → literal `;` without termination; after `COPY … FROM STDIN` statement terminator, subsequent lines up to `\.` → `Data`; `:var` forms annotated for ALG-SQL-PARAM-01.
  3. MySQL CLI: a statement-start line matching `DELIMITER\s+(\S+)` (case-insensitive) sets the delimiter (rest of the first whitespace-free token; `DELIMITER ;` restores) → `Command`; subsequent SQL segments end at the delimiter string (outside quotes/comments) or `\G`/`\g`.
  4. sqlcmd/SSMS: a line matching `^\s*GO(\s+\d+)?\s*(--.*)?$` (case-insensitive, outside block comments/strings) → batch boundary with repeat count; `:setvar NAME value`, `:r file`, `:on error`, `!!cmd` → `Command`; `$(NAME)` substitution recorded for preview (and denied by Karagöz unless substituted client-side).
  5. SQL*Plus: line equal to `/` → executes the buffer (PL/SQL unit terminator); lines starting with `SET `, `SPOOL`, `REM`, `PROMPT`, `@`, `@@`, `DEFINE`, `COLUMN`, `TTITLE` at statement start → `Command`; `&var` recorded (Hacivat: prompt for values only if `define` emulation enabled).
  6. sqlite3: lines beginning with `.` at statement start → `Command`.
  7. Emit `Sql` segments with their effective delimiter for ALG-SQL-SPLIT-01.
- **Complexity:** O(n).
- **Edge cases:** `DELIMITER` inside a string or comment; `DELIMITER` without argument (error); delimiter set to a string containing quotes or `\`; delimiter `;;`; `GO` with trailing text other than a count or comment (SSMS: error; treat as SQL); `GO` in lower case; `GO` preceded by spaces; `GO` at EOF without newline; `go` as column name in `SELECT go\nFROM t` (not on its own line → SQL); psql `\` inside `$$` body (not a command); `\.` in a COPY data line with trailing spaces; `/` on a line with trailing spaces (SQL*Plus accepts; treat as terminator); SQL*Plus `/` after a plain SQL statement that already ended with `;` (re-executes buffer — Hacivat: ignore duplicate).
- **Test cases:** MySQL `DELIMITER //\nCREATE PROCEDURE p() BEGIN SELECT 1; END//\nDELIMITER ;\nCALL p();` → [Command, Sql(p…END), Command, Sql(CALL p())]; MySQL with `DELIMITER $$` and `END$$` → `END` + delimiter; T-SQL `SELECT 1\nGO 3\nSELECT 2` → [Batch(SELECT 1, repeat 3), Batch(SELECT 2)]; psql `COPY t FROM STDIN;\n1\t2\n\\.\nSELECT 1;` → [Sql, Data, Sql]; Oracle `BEGIN NULL; END;\n/` → one unit.
- **Prior art and evidence:** pgls backslash handling [Observed: `common.rs:124-143,199-217`]; pgcli strips `\ns` named query prefix and handles specials [Observed: `pgcli/packages/sqlcompletion.py:158-170`]; mysql/sqlcmd/SQL*Plus documented behaviour [Inferred].
- **Used by:** Hacivat | Karagöz (Karagöz rejects any `Command` segment and any `Data` segment except via explicit COPY APIs)

### ALG-SQL-PARAM-01: Parameter / placeholder detection
- **Problem:** Find bind placeholders and client variables in a statement to (a) prompt the user for values, (b) bind them via the driver, (c) normalise for fingerprints, without false positives (operators, casts, local variables).
- **Inputs / outputs:** `(tokens of one statement, dialect, client_flavor, user_settings: {enable_colon_params, enable_dollar_brace_params, custom_patterns})` → `Vec<Placeholder{range, kind: Positional(n) | Question(index) | Named(name) | Client(name, quoting), bind_style}>` + rewritten SQL for the driver (e.g. `:name` → `$1` for PG, `?` for MySQL).
- **Algorithm:**
  1. Only consider tokens outside strings/comments/dollar quotes (guaranteed by the lexer).
  2. Classify per dialect:
     - PG: `PositionalParam` → `Positional(n)`. `?`: placeholder **only** if the user enabled JDBC-style `?` and the token is in an *operand position* (previous significant token is an operator, `(`, `,`, keyword expecting an expression such as `WHERE`, `AND`, `=`, `VALUES`, `LIMIT`, `SET col =`), never when between two operands (`a ? 'k'` jsonb operator), never `?|`/`?&`. `:name` (ClientVar): placeholder unless previous token is `:` or the colon is part of `::`/`:=` or inside `[a:b]` array slice (inside `[` … `]` at depth); pgls additionally treats `:name` after `FROM/TABLE/SCHEMA/…/.` as an *identifier* placeholder [Observed: `pgls_lexer/src/params.rs:5-25`].
     - MySQL: `?` → Question; `@name` → session user variable (not a placeholder; flagged as session-state reference).
     - T-SQL: `@name` → placeholder **unless** declared in the same batch by `DECLARE @name`, or is a procedure/function parameter in a `CREATE PROCEDURE` header, or is assigned via `SELECT @name =`/`SET @name =` before use; `@@name` → system function.
     - SQLite: `?`, `?NNN`, `:a`, `@a`, `$a` → placeholders; same name → same index.
     - ClickHouse: `{name:Type}` → Named with type.
     - Oracle: `:name`/`:1` → placeholders, except inside PL/SQL where `:new`/`:old` in triggers are pseudo-records (not binds) and `:=` assignment.
  3. Number placeholders for the driver: positional dialects get `$1..$n` in order of first occurrence of each distinct name (PG) or `?` repeated per occurrence (MySQL; values duplicated).
  4. For audit fingerprints, replace literals and placeholders uniformly (`?`) after classification.
- **Complexity:** O(tokens).
- **Edge cases:** `$1` inside `$1$…$1$`? (invalid tag: tags cannot start with digits → `$1` param then `$`); `::numeric` after `:x` (`:x::int` → var then cast); `a[:2]`, `a[1:]`, `a[:]`; `?` inside PG `jsonb ? 'k'` vs `WHERE id = ?`; `@>` PG operator vs `@a` T-SQL; T-SQL `DECLARE @x INT = @y` (`@y` unresolved → placeholder); `EXEC sp @p = @v`; MySQL `SELECT @x := 1`; SQLite `$a::b(c)`; ClickHouse `{` in map literal `{'a':1}` (not `{ident:` → not param); user-defined template syntaxes `${var}` (DataGrip-like) handled only if enabled; placeholders inside client commands.
- **Test cases:** PG `select * from t where a = :a and b::int = :b and c ? 'k'` → [`:a`, `:b`] (not `?`); PG JDBC mode `where a = ? and j ? 'k'` → [first `?`]; T-SQL `DECLARE @x int = 1; SELECT * FROM t WHERE id = @x AND y = @y` → [`@y`]; SQLite `select ?2, :a, @a, $a` → 4 placeholders (`:a`,`@a`,`$a` distinct names); ClickHouse `select {n:UInt8}` → Named(n, UInt8); PG `select arr[1:2]` → none.
- **Prior art and evidence:** pgls named param conversion with identifier-context keywords and restorable flag [Observed: `crates/pgls_lexer/src/params.rs:1-60`]; pgls tokenizer param kinds [Observed: `pgls_tokenizer/src/lib.rs:139-225`].
- **Used by:** Hacivat | Karagöz (fingerprint normalisation; Karagöz never substitutes client variables itself)

---

## 5. Implications for Hacivat / Karagöz / Perde

1. **Crate `sql-lexer`** (no dependencies besides `text-size`): `LexDialect` tables for `Postgres{version}`, `MySql{version, modes}`, `MariaDb{version, modes}`, `SqlServer{quoted_identifier}`, `Sqlite`, `ClickHouse`, `Oracle`, `Generic`; `KeywordTable` generated at build time from data files (PG from libpg_query `kwlist.h` — permissive; others from curated JSON with source URLs). Runtime override: Hacivat may refresh MySQL/SQLite/Oracle keyword tables from the live server.
2. **Crate `sql-split`**: ALG-SQL-SPLIT-01 + SPLIT-02 + PARAM-01. Pure, no IO. Karagöz depends on it.
3. **Session lexical modes are part of the connection state** in Hacivat and Karagöz: read `standard_conforming_strings`, `sql_mode`, `QUOTED_IDENTIFIER` at connect, and update them when a successfully executed statement changes them (Hacivat) — Karagöz should instead *deny* statements that change lexical modes (doc 05).
4. **Execution UX:** "Run statement at cursor" uses editor split; "Run script" uses execution split and stops with a dialog on `Ambiguous`. T-SQL scripts run per `GO` batch; Oracle units sent with/without trailing `;` per kind; SQLite uses engine tail splitting.
5. **Karagöz:** accept only single-statement requests (re-split server-side, require exactly one `Exact` item, no `Command`/`Data` items); use PG extended protocol / MySQL without multi-statements; for SQL Server, classify the entire batch.
6. **Perde:** nothing on the data path; may index fingerprints produced by Karagöz.

## 6. i18n considerations

- Identifier characters include all non-ASCII letters in every target dialect: highlighting, word-boundary detection (double-click select, completion prefix extraction) must use the dialect `ident_continue` predicate, not Unicode word segmentation (e.g. PG allows any byte ≥ 0x80, including emoji; T-SQL allows only Unicode letters).
- **Turkish casing:** keyword matching and PG identifier folding are ASCII-only. A Turkish user typing `SELECT * FROM İşlemler` gets a quoted-identifier-free name `İşlemler` that PG folds to `İşlemler` (only ASCII `I` would fold to `i`; `İ` is preserved) — completion must show and insert names exactly as stored (PG lower-cased ASCII only), and suggest quoting when the stored name differs from its folded form. Oracle folds to upper with ASCII rules for unquoted identifiers; `ı` (dotless i) never maps to `I` in SQL.
- Error and diagnostic messages from the lexer are codes (`lex.unterminated-string`, `lex.mysql-dash-comment-needs-space` …) rendered via Fluent bundles; no English text in `sql-lexer`.
- Byte offsets internally; the editor converts to grapheme-based columns for display and to UTF-16 for any LSP-shaped API.
- Full-width punctuation (`；`, `（`, `＇`) pasted from CJK input methods is `Unknown` in every dialect; offer a quick-fix "replace full-width punctuation".

## 7. Risks, pitfalls, open questions

- **Vendor-doc facts need conformance tests** against real servers (Docker matrix: PG 13–18, MySQL 8.0/8.4/9.x, MariaDB 10.6/11.x, SQL Server 2019/2022, SQLite 3.4x, ClickHouse LTS, Oracle Free 23ai). Items to verify first: ClickHouse nested comments and `FORMAT` data termination; SQLite digit separators; MySQL `-- ` boundary set (control chars); T-SQL `GO` trailing comment; Oracle `;` acceptance in OCI 23ai; PG vertical-tab whitespace.
- **Parser differential (security):** for MySQL, executable comments and `sql_mode`-dependent lexing are classic WAF bypass vectors; Karagöz must lex with the *server's* modes and treat all `/*!`/`/*M!` bodies as code (doc 05).
- **psql `:var` vs PG `::`** and **`?` vs jsonb** are perennial false-positive sources; default to *no* `?` placeholders for PG.
- **Keyword tables drift** across server versions; [Unknown] whether we can obtain MariaDB reserved words programmatically.
- **Heuristic editor splitting can disagree with the server** — never use editor-mode output for execution.
- **Open question:** do we support SQL*Plus/sqlcmd variable substitution, or only detect and warn? Proposal: detect + warn in v1; implement `$(var)`/`&var` prompts later.
