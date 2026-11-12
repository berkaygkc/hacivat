# SQL Language Tooling 03 — Parsing Architecture: Lossless Trees (rowan), Error Recovery, Incremental Computation (salsa)

Series: `01` landscape · `02` lexing & splitting · **`03` parsing architecture** · `04` completion & resolution · `05` safety, lint, format.

This document specifies the syntax layer of Hacivat's shared SQL analysis stack: how to build lossless, error-tolerant concrete syntax trees for several SQL dialects, how to reparse incrementally, and how to organise semantic computation incrementally with salsa. It closes with the **recommended crate architecture and phased plan** for the whole SQL stack (client + agent).

---

## 1. Scope and sources

| Source | Commit | License | What was read |
|---|---|---|---|
| rust-analyzer `crates/parser`, `crates/syntax`, `crates/ide-completion`, `crates/base-db` | `2a362a41a8eb` (2026-11-10) | MIT OR Apache-2.0 | Event parser, markers, recovery, trivia attachment, step limit, incremental reparse, completion marker, salsa usage |
| rowan | `d601b195d7bc` (2026-10-21), `0.18.0-alpha.2` | MIT OR Apache-2.0 | Green/red trees, node cache, builder, `replace_with` |
| salsa | `1f48f4a32954` (2026-11-09), `0.28.5` | Apache-2.0 OR MIT | Inputs/tracked/interned, durability, cancellation, LRU, red-green algorithm, backdating |
| biome (`biome_rowan`, `biome_parser`, sparse) | `f6cf28e3518f` (2026-11-11) | MIT OR Apache-2.0 | Slots, bogus nodes, syntax factory, recovery token sets, token-attached trivia |
| postgres-language-server | `083c5d1bbf04` | MIT | Statement-level content-addressed caching (no salsa) |
| datafusion-sqlparser-rs | `8a176d98b37b` | Apache-2.0 | Pratt parser structure, recursion guard, dialect hooks |
| sqruff | `c3d18575d6d4` | Apache-2.0 | Combinator grammar + `Unparsable` segments |
| ruff (not cloned) | — | MIT | AST + tokens + comment side table [Inferred] |
| PostgreSQL `gram.y` precedence (not in clone; libpg_query ships generated C) | PG 18 | PostgreSQL License | Operator precedence [Inferred: PG docs "Operator Precedence" table + gram.y `%left/%right/%nonassoc` declarations] |

---

## 2. Executive summary

1. **Use a rowan-style lossless green/red tree** for every SQL dialect. The green tree is immutable, position-independent, structurally shared and cheap to splice; red nodes give parent pointers and absolute offsets on demand. Lossless = every byte (incl. whitespace, comments, invalid tokens) is in the tree, which is what formatting, refactoring, highlighting and "exact text of statement N" need. [Observed: `rowan/src/green/node.rs:30-60`, `rowan/src/api.rs:16-130`]
2. **Parse with a hand-written recursive-descent + Pratt parser that emits events**, not a tree directly. Events (`Start{kind, forward_parent}`, `Token`, `Finish`, `Error`) decouple grammar code from tree construction, allow `precede()` for left-recursive constructs (e.g. `a.b` → `FieldExpr`, `x::int`, `t1 JOIN t2`), and allow an `abandon()`ed marker to vanish. [Observed: `rust-analyzer/crates/parser/src/event.rs:22-62`]
3. **Error recovery = "never stop, always produce a tree":** expected-but-missing nodes are simply absent; unexpected tokens are wrapped in `ERROR` nodes unless they belong to a **recovery set** (tokens that likely start the next construct, e.g. clause keywords `FROM WHERE GROUP HAVING ORDER LIMIT`, `)` , `;`). [Observed: `rust-analyzer/crates/parser/src/parser.rs:279-310` `err_recover`; `biome/crates/biome_parser/src/parse_recovery.rs:53-70`]
4. **Guard against adversarial input:** a step counter (RA asserts < 15,000,000 steps in release builds — "the parser seems stuck"), a nesting-depth limit (sqlparser-rs uses 50), and iterative (not recursive) handling of long lists and binary operator chains. Karagöz must convert limit hits into a *deny*, not a crash. [Observed: `parser.rs:39-72`; sqlparser-rs `src/parser/mod.rs:57-63,196`]
5. **Trivia attachment** is a tree-building policy: RA attaches leading comments to item nodes unless separated by a blank line; for SQL, attach leading comments to the *statement* node (so "statement text" includes its header comment) and to select-list items/columns for formatter fidelity. [Observed: `rust-analyzer/crates/parser/src/shortcuts.rs:150-280`]
6. **biome's slot model** (fixed child slots per node kind, `Empty` slots for missing children, invalid lists become *bogus* kinds) makes typed AST accessors O(1) and robust; rowan's plain children list is simpler. Recommendation: plain rowan + codegen'd typed accessors (RA `ungrammar` style) for v1; revisit slots if accessor cost or ambiguity shows up. [Observed: `biome_rowan/src/syntax_factory.rs:1-60`, `green/node.rs:53`]
7. **Incrementality at two levels:** (a) *document → statements* via the splitter (doc 02) and content-addressed statement identity (pgls's proven approach); (b) *within a statement* via RA-style reparse: single-token relex when the token kind is stable, else reparse the smallest enclosing **reparsable node** (for SQL: the statement, a parenthesised subquery, a `BEGIN … END` block, a dollar-quoted body). Statements are typically < 10 KB, so reparsing one statement per keystroke is well under 1 ms. [Observed: `rust-analyzer/crates/syntax/src/parsing/reparsing.rs:1-140`; pgls `document.rs:44-66`]
8. **pgls shows full-statement reparse is sufficient in practice** — it never uses tree-sitter incremental edits and still delivers interactive completion. Keep token-level incremental reparse as an optimisation for very large statements (generated INSERTs, big DDL), not a prerequisite. [Observed: `pgls_workspace/src/workspace/server/tree_sitter.rs`]
9. **salsa for semantics, not for parsing:** model `SourceText` and `Catalog` snapshots as inputs, `statements(file)`, `parse(stmt)`, `scope(stmt)`, `resolve(stmt)`, `diagnostics(stmt)` as tracked functions keyed by interned statement content. Salsa's red-green algorithm with **backdating** ("value unchanged → dependants not re-executed") gives early cutoff when an edit doesn't change a statement's AST shape. [Observed: `salsa/book/src/overview.md:137`, `SUMMARY.md` "Backdate"; RA `base-db/src/editioned_file_id.rs:13-31` uses `#[salsa::tracked(lru = 128)]` on parse]
10. **Durability matters for catalogs:** schema snapshots change rarely → `Durability::HIGH`; editor text → `LOW`. A text edit then never re-validates catalog-derived queries. [Observed: `salsa/src/durability.rs:5-110`]
11. **Cancellation:** salsa cancels in-flight queries when an input changes (`Cancelled::PendingWrite`); completion/diagnostics run on a snapshot thread and unwind on cancellation — needed for keystroke latency. [Observed: `salsa/src/cancelled.rs:12-51`]
12. **Karagöz does not need salsa or rowan-red trees**: it parses one statement, classifies, drops. It should still use the *same* lexer, splitter and parser crates (no IDE features) so client and agent agree; the classifier works on the typed AST layer.
13. **One parser crate, dialect capability flags, shared ANSI core.** Each dialect is a `ParserDialect` value (feature flags + statement table + precedence table + keyword table), like sqlparser-rs's `Dialect` trait but data-first. The `SyntaxKind` enum is a superset across dialects; typed AST nodes are shared where shape is shared (`SelectStmt`, `JoinExpr`) and dialect-specific otherwise (`PgCopyStmt`, `MySqlLoadData`, `TsqlExecuteStmt`).
14. **PostgreSQL gets an oracle:** libpg_query parses each statement in the background; differential testing (our tree accepts/rejects the same corpus — PG regression SQL as pgls does via `pgls_postgres_regress`) keeps our grammar honest. [Observed: `postgres-language-server/crates/pgls_postgres_regress`]
15. **Embedded languages** (PL/pgSQL bodies, T-SQL procedure bodies, MySQL stored programs, SQL inside `EXECUTE 'string'`) are parsed as *child documents* with their own offset mapping, as pgls does for SQL function bodies (`StatementId::Child`). [Observed: `statement_identifier.rs:1-60`; `pg_query.rs` `get_or_cache_embedded`]

---

## 3. Detailed findings

### 3.1 Requirements the syntax layer must satisfy

| # | Requirement | Consumer | Implication |
|---|---|---|---|
| R1 | Lossless round-trip (`tree.text() == source`) | formatter, refactor, statement extraction, audit "exact SQL" | rowan green tree incl. trivia and error tokens |
| R2 | Total: any input → tree (+ errors) | editor | recovery, no panics, step/depth limits |
| R3 | Keystroke latency (< 5 ms p95 for completion context on 1 MB file) | editor | statement-granular work, incremental reparse, cancellation |
| R4 | Precise positions | diagnostics, LSP | byte offsets (`TextSize` u32), line index at the boundary |
| R5 | Typed, ergonomic AST | completion, lint, classifier | codegen'd wrappers over red nodes |
| R6 | Multi-dialect with shared core | all | dialect flags + statement tables |
| R7 | Fail-closed strict mode | Karagöz | "strict" parse result: any `ERROR` node or missing required child ⇒ classification Deny |
| R8 | Deterministic and versioned | Karagöz audit | parser version recorded with decisions |
| R9 | Cheap to embed (no C) | WASM / mobile future | pure Rust core; libpg_query behind a feature |

### 3.2 rowan's model

- **Green nodes** store `kind`, total `text_len` and children (`GreenChild` = node or token with *relative* offset; 2 words each on 64-bit); green tokens store `kind` + text. Equality is structural; `GreenNode` is a `ThinArc` → cheap clone and sharing. [Observed: `rowan/src/green/node.rs:30-60`]
- **NodeCache** deduplicates tokens and small nodes: nodes with more than 3 children are never cached (`children_ref.len() > 3` → build without hash); child hashes of 0 (uncached) disable caching of the parent. This interning makes identical subtrees (e.g. repeated `, ` + identifier patterns, `NULL` literals) share memory across statements and reparses. [Observed: `rowan/src/green/node_cache.rs:72-95`]
- **Red layer** (`SyntaxNode`) is created lazily while traversing: parent pointer + absolute offset + index; `replace_with(green)` returns a new root green with the path copied (persistent update). [Observed: `rowan/src/api.rs:118`, `cursor.rs:70,371`]
- `GreenNodeBuilder::with_cache(&mut NodeCache)` allows sharing the cache across builders (e.g. across statements in a document). [Observed: `rowan/src/green/builder.rs:14-30`]
- Language binding via `rowan::Language` trait mapping `SyntaxKind(u16)` ↔ our enum.

### 3.3 rust-analyzer's parser pipeline

1. **Lexing** to `LexedStr` (kinds + offsets + errors), then `to_input()` drops trivia and records "joint" flags (adjacency) for multi-char punctuation composition. [Observed: `crates/parser/src/lexed_str.rs`, `input.rs`]
2. **Grammar functions** (`crates/parser/src/grammar/*.rs`) operate on the trivia-free input with `p.at(kind)`, `p.eat`, `p.expect`, `p.bump`, markers `let m = p.start(); …; m.complete(p, KIND)`; `CompletedMarker::precede(p)` for left recursion; `m.abandon(p)` when a speculative node is not needed. [Observed: `event.rs:22-62`]
3. **Recovery**: `err_and_bump(msg)` wraps one token into an `ERROR` node; `err_recover(msg, recovery_set)` does the same unless the current token is in the recovery set (or is a brace), in which case it only records the error and leaves the token for an outer rule. Recovery sets are unions of constants like `EXPR_RECOVERY_SET`, `ITEM_RECOVERY_SET`. [Observed: `parser.rs:279-300`; `grammar/paths.rs:83-84`; `grammar/generic_args.rs:45`]
4. **Stuck-parser guard**: each `nth`/lookahead increments `steps`; `assert!(steps < PARSER_STEP_LIMIT)` (150k debug / 15M release) and reset on bump. Ensures no infinite loop when a rule fails to consume. [Observed: `parser.rs:39-72,306-310`]
5. **Tree building** (`shortcuts.rs`): walks events, re-inserts trivia between tokens, decides how many leading trivia attach *inside* a node (`n_attached_trivias`: comments directly above an item attach to it unless a blank line separates them). [Observed: `shortcuts.rs:150-280`]
6. **Errors** are stored beside the tree (`Parse { green, errors }`), not as tree nodes, except `ERROR` nodes for skipped tokens.
7. **Incremental reparse** (`reparsing.rs`): token-level relex when edit is inside a single relexable token and kind unchanged (with a check that the next char wouldn't merge into it), else block-level reparse of the nearest ancestor with a `Reparser` (e.g. `{}` blocks) if its new text is bracket-balanced; errors outside the reparsed range are shifted (`merge_errors`). [Observed: `crates/syntax/src/parsing/reparsing.rs:1-140`]
8. **Completion** reparses with an inserted fake identifier (`raCompletionMarker`) to get a well-formed tree at the cursor. [Observed: `ide-completion/src/context.rs:32,803-809`] (See doc 04.)

### 3.4 biome's variations

- **Slots:** every node kind has a fixed number of slots; `SyntaxFactory::make_syntax` maps parsed children to slots, filling `Slot::Empty{rel_offset}` for absent ones; list kinds validate elements and degrade to a **bogus** kind if any element is invalid. Typed accessors become `slot(n)` lookups, and "missing required child" is explicit. [Observed: `biome_rowan/src/syntax_factory.rs:13-60`; `green/node.rs:53-60,375`]
- **Trivia on tokens:** leading/trailing trivia pieces are stored on tokens instead of as sibling tokens; simplifies formatter comment handling. [Observed: `biome_rowan/src/syntax/trivia.rs:259-263`]
- **Recovery helper:** `ParseRecoveryTokenSet::new(bogus_kind, recovery_set)` (+ optional `line_break` stop) eats tokens into a bogus node. [Observed: `biome_parser/src/parse_recovery.rs:53-70`]
- Trade-off: slots + bogus require more codegen and per-kind factories; benefit is accessor robustness. For SQL, where clauses are mostly optional and order-constrained, slots map naturally (`SelectStmt` slots: `with, select_kw, distinct, targets, into, from, where, group_by, having, window, order_by, limit, offset, fetch, locking`).

### 3.5 ruff (for contrast) [Inferred: public ruff architecture]

- Produces a typed, non-lossless AST with ranges; tokens kept in a separate vector; comments mapped to nodes (leading/trailing/dangling) in a side table used by the formatter. Fast and simple, but refactorings must go through text edits computed from ranges. For SQL tools that must reproduce exact text (audit, formatting with unknown constructs), the lossless CST is preferable.

### 3.6 sqlparser-rs and sqruff as parsing references

- sqlparser-rs: Pratt expression parser (`parse_prefix`, `parse_infix`, `get_next_precedence` overridable per dialect), recursive descent for statements, depth guard 50. Useful as a *catalogue of syntax per dialect* (its tests are a corpus), not as a structure to copy. [Observed: `src/dialect/mod.rs:740,949-1124`; `src/parser/mod.rs:196`]
- sqruff: declarative grammar combinators interpreted at runtime with backtracking and caching; unmatched regions → `Unparsable`. Great dialect breadth; runtime interpretation and backtracking make it slower and harder to reason about recovery precisely. [Observed: `crates/lib-core/src/parser/match_algorithms.rs`]

### 3.7 salsa essentials

- **Inputs** (`#[salsa::input]`): set by the host; each field has a durability. **Tracked functions** (`#[salsa::tracked]`): memoised, dependency-recorded; options `returns(ref|clone|copy|as_deref)`, `lru = N`, `no_eq`. **Interned** (`#[salsa::interned]`): value → small id, equality by value. **Tracked structs**: entities created inside tracked fns with identity stable across revisions. [Observed: `salsa/src/{input,function,interned,tracked_struct}.rs`; `salsa/examples/calc/*.rs`; RA `base-db/src/lib.rs:50,203-230`]
- **Red-green algorithm**: on a new revision, a memo is reused if none of its inputs changed (checked lazily, deepest-first "maybe changed after"); if a dependency was recomputed but produced an equal value, the memo is *backdated* — dependants see no change (early cutoff). [Observed: `salsa/book/src/overview.md:137`; `SUMMARY.md:39` Backdate]
- **Durability** LOW/MEDIUM/HIGH: a revision that changes only LOW inputs lets HIGH-only queries skip validation entirely. [Observed: `salsa/src/durability.rs:5-110`]
- **Cancellation**: writers bump the revision; readers on snapshots get `Cancelled::PendingWrite` at their next query boundary and unwind. [Observed: `salsa/src/cancelled.rs:12-51`]
- **Cycles**: supported with fixpoint/fallback configuration (`cycles.md`) — relevant for recursive CTEs and view-of-view resolution. [Observed: `salsa/book/src/cycles.md`]

### 3.8 SQL-specific parsing concerns

1. **Contextual keywords.** The lexer emits `Word` with an optional keyword id (doc 02). The parser checks `p.at_kw(KW_SELECT)` and, for identifier positions, accepts a `Word` whose keyword category permits that position (PG: `ColId` = IDENT ∪ UNRESERVED ∪ COL_NAME; `type_function_name` = IDENT ∪ UNRESERVED ∪ TYPE_FUNC_NAME; `ColLabel` = any keyword; bare column labels only `BARE_LABEL`). The tree records the token as `IDENT` or the keyword kind via `remap` at bump time (RA does this for contextual keywords). [Observed: kwlist categories; pgls `is_col_id` `splitter.rs:27-37`]
2. **Expression precedence per dialect** (Pratt binding powers from a table). PostgreSQL, lowest → highest [Inferred: PG docs operator precedence table / gram.y]: `OR` < `AND` < `NOT` (right) < `IS ISNULL NOTNULL` < comparison `< > = <= >= <>` < `BETWEEN IN LIKE ILIKE SIMILAR` < any other operator (`Op`, incl. `||`, `->`, `@>`) < `+ -` < `* / %` < `^` < `AT TIME ZONE` < `COLLATE` < unary minus < subscript `[]` < `::` < `.`. MySQL differs (`||` is OR unless `PIPES_AS_CONCAT`; `!` high precedence; `NOT` low unless `HIGH_NOT_PRECEDENCE`; `:=` lowest, right-assoc; `^` is XOR with high precedence; `DIV`, `MOD`; `COLLATE` highest). T-SQL: `~` highest, then `* / %`, `+ - & ^ |`, comparisons, `NOT`, `AND`, `ALL ANY BETWEEN IN LIKE OR SOME`, `=` assignment. Tables are data in `ParserDialect`.
3. **Ambiguities requiring lookahead or speculation:**
   - Alias without `AS`: `SELECT a b FROM t` vs `SELECT a FROM` — decided by keyword category (PG BARE_LABEL) and dialect reserved lists (sqlparser-rs `RESERVED_FOR_COLUMN_ALIAS`). [Observed: sqlparser-rs `keywords.rs:1239-1360`]
   - `(` after `FROM`/`IN`/`EXISTS`/in expressions: subquery vs parenthesised join vs row/expression list → peek for `SELECT|WITH|VALUES|TABLE|(` chains; for `((SELECT …))` recurse.
   - Function call vs keyword construct: `LEFT(…)` function vs `LEFT JOIN`; `EXTRACT(x FROM y)`, `SUBSTRING(a FROM 1 FOR 2)`, `TRIM(BOTH 'x' FROM y)`, `POSITION(a IN b)`, `CAST(x AS t)`, `OVERLAY(...)` — special-form table per dialect.
   - Type names with spaces/params: `DOUBLE PRECISION`, `CHARACTER VARYING(10)`, `TIMESTAMP(3) WITH TIME ZONE`, `INTERVAL '1' DAY TO SECOND`, PG arrays `int[]`, `int ARRAY[3]`.
   - Generic type-prefixed literals: `DATE '2020-01-01'`, `int4 '1'` (PG `func_name Sconst`).
   - PG `a.b.c.d` names: database.schema.table.column — parse as a generic `NameRef` path; resolution decides.
   - T-SQL: statements without terminators require each statement rule to stop at the first token that cannot continue it; `WITH` needs preceding `;`.
4. **Recovery sets for SQL** (union per context): statement-level: all statement-start keywords + `;` + EOF; clause-level in SELECT: `FROM WHERE GROUP HAVING WINDOW QUALIFY ORDER LIMIT OFFSET FETCH FOR UNION INTERSECT EXCEPT ) ;`; expression-level: `, ) ] THEN ELSE END WHEN AS` + clause set; list-level: `,` + closer; DDL column-def: `, )` + constraint keywords (`CONSTRAINT PRIMARY UNIQUE CHECK FOREIGN REFERENCES`).
5. **Nesting limits:** cap expression depth (e.g. 1,000 for editor, 256 for Karagöz strict) and use iterative Pratt loops for left-associative chains (`a + b + … + z` with 100k terms must not recurse 100k deep); `UNION` chains likewise iterative. Hitting a limit produces an `ERROR` node + `TooDeep` diagnostic (editor) / Deny (Karagöz).
6. **Embedded bodies:** dollar-quoted function bodies are a single `DollarString` token in the outer tree; a separate *child parse* (SQL or PL/pgSQL grammar chosen from `LANGUAGE`) is keyed by `(parent statement id, token range)` with an offset map. T-SQL and MySQL procedure bodies are inline (not quoted), so they are parsed in the main tree with procedural-statement grammar.
7. **String continuation** (PG): `'a'\n'b'` is one literal node with two string tokens; validation (newline required) is a post-parse check.
8. **Placeholders & client variables** are `Param` expression nodes; psql `:'var'` in expression position parses as `Param` too.

### 3.9 Strict vs tolerant mode

The same parser runs in two modes:
- **Tolerant** (editor): recovery on, `ERROR` nodes allowed, missing children allowed.
- **Strict** (Karagöz classifier, Hacivat "safe mode" pre-check): same parse, then a validation pass: reject if any `ERROR` node, any missing required slot, any unknown token, any `TooDeep`, or any dialect feature flagged "unsupported by classifier". This avoids maintaining two grammars.

---

## 4. Algorithm specifications

### ALG-SQL-PARSE-01: Lossless tree building with error recovery
- **Problem:** Build a rowan green tree for one statement (or a whole document of statements) from lexer tokens using a recursive-descent/Pratt grammar, such that the tree is lossless and always produced, with precise errors.
- **Inputs / outputs:** `(tokens: &[Token], text: &str, dialect: &ParserDialect, mode: Tolerant|Strict, limits: {max_depth, max_steps})` → `Parse { green: GreenNode, errors: Vec<SyntaxError{range, code, args}>, limits_hit: bool }`.
- **Data structures:**
  - `Input`: significant tokens only (`kinds: Vec<SyntaxKind>`, `kw: Vec<Option<KeywordId>>`, `joint: BitVec` for adjacency like `:` `:` → `::` when lexer didn't merge), mapping to raw token indices.
  - `Event` enum: `Start{kind, forward_parent: Option<u32>}`, `Finish`, `Token{kind_remap: Option<SyntaxKind>, n_raw: u8}`, `Error{code}`; `TOMBSTONE` kind for abandoned starts.
  - `Marker{pos}` (drop-bomb in debug builds), `CompletedMarker{pos, kind}` with `precede()`.
  - `TokenSet` = bitset over `SyntaxKind` (u128 or `[u64; N]`) + keyword bitset.
- **Algorithm:**
  1. **Parse phase** (`Parser` over `Input`):
     - `source_file`: loop `while !eof`: if at `;` → bump as `SEMICOLON` (empty statement node `EMPTY_STMT` if consecutive); else `statement()`; then `eat(;)` or, if not at EOF and not at a statement-start token, `err_recover("expected ;", STMT_START_SET)` (Tolerant) .
     - `statement()`: dispatch on (first keyword, second keyword) via the dialect statement table; unknown head → `ERROR_STMT` node that consumes tokens until a recovery token at paren depth 0 (`;` or a statement-start keyword at line start), so an unknown statement never swallows the next valid one.
     - Clause parsers check FIRST sets; when a required element is missing: `p.error(code)` without consuming if the current token is in the context's recovery set; else wrap the offending token in `ERROR` and continue (`err_recover`).
     - Lists (`delimited(p, item, sep=',', closer, recovery)`) loop: parse item; if `,` → continue; if at closer or recovery → stop; trailing comma before closer → error `TrailingComma` (unless dialect allows); missing comma between items when the next token starts an item → error `MissingComma` and continue (classic typing case `SELECT a b c FROM`… careful: `a b` may be an alias; only report missing comma after an alias was already taken).
     - Expressions: Pratt loop `expr_bp(min_bp)`: prefix (literal, name/path, `(`→ subquery/row/paren, `CASE`, `CAST`, special forms, unary ops, `EXISTS`, `ARRAY[`, `ROW(`, `INTERVAL`, params) then loop over infix/postfix ops with binding powers from `dialect.precedence`; `lhs.precede()` to wrap `BIN_EXPR`, `CAST_EXPR` (`::`), `INDEX_EXPR`, `FIELD_EXPR`, `IS_EXPR`, `BETWEEN_EXPR` (special ternary), `IN_EXPR`, `LIKE_EXPR` (+ `ESCAPE`), `AT_TIME_ZONE`, `COLLATE`. Depth counter incremented on prefix recursion (parens, subqueries, CASE), not on loop iterations.
     - Contextual keywords: when an identifier is expected and the current token is a `Word` with keyword `k`, accept if `dialect.keyword_allowed(k, position)`; emit `Token{kind_remap: IDENT}`.
     - Step guard: every lookahead increments `steps`; on bump reset; if `steps > max_steps` → stop parsing, emit `Error{ParserStuck}`, wrap remaining tokens in one `ERROR` node, set `limits_hit` (never panic in production builds).
  2. **Build phase** (events → green):
     - Walk events; resolve `forward_parent` chains (start the outermost parent first); skip TOMBSTONEs.
     - Before entering a node, compute how many preceding trivia tokens attach *inside* it: for `STATEMENT` kinds attach line comments/block comments directly above (no blank line between comment and statement); for other nodes attach none (trivia go to the parent between children). After the last significant token of the root, attach trailing trivia to the root `SOURCE_FILE`.
     - For each `Token` event, push the `n_raw` raw tokens (merged into one green token if `n_raw > 1`, e.g. `::` composed) with the remapped kind.
     - Lexer error tokens (`Unknown`, unterminated strings) are pushed as tokens with their kinds; their errors are copied into `errors`.
     - Use a shared `NodeCache` per document to dedupe small nodes/tokens.
  3. **Strict validation** (mode = Strict): traverse the tree: any `ERROR*` kind, any lexer error flag, any node with a missing *required* child (per codegen'd grammar metadata), `limits_hit` → return `Err(StrictParseError{first_error})`.
- **Complexity / performance:** O(n) tokens with bounded lookahead (≤ 4 tokens typical; speculative parsing restricted to bounded windows like `(`-classification); allocation: one event vec + green nodes; NodeCache amortises. Target: 50–100 MB/s parse throughput; a 5 KB statement in ~50 µs.
- **Edge cases (exhaustive list):** empty statement list; only comments; statement ending at EOF without `;`; `;` inside unbalanced parens (recovery stops at `;` only at depth 0 — but an unclosed `(` must not swallow the rest of the document: the statement-level recovery treats `;` as a hard stop that closes all open parens with errors); unclosed `CASE` (stop at `;`/statement start; `MissingEnd`); stray `)` at statement level (ERROR token, continue); keyword used as identifier where allowed (`SELECT name, type, value FROM settings`); reserved keyword used as identifier where not allowed (`SELECT select FROM t` → error but tree continues with IDENT remap in Tolerant mode); `SELECT` with trailing comma before `FROM`; `SELECT FROM t` (PG allows empty target list); `SELECT * FROM` (missing table → empty slot + error; completion relies on it); `WHERE` at end; `JOIN` without `ON` (PG error, MySQL allowed for INNER JOIN); `GROUP BY` with trailing `,`; extremely long `IN (…)` list (iterative); 10k-deep nested parentheses (depth limit); 1 MB single string token; statements mixing dialects (dialect flags off → feature tokens produce errors, not panics); `::` split by whitespace `a: :int` (not joint → error); PG `OPERATOR(pg_catalog.+)` syntax; `$1` params in DDL positions; unknown statement heads (`VACUUM`-like utility statements not yet in grammar → `UNKNOWN_STMT` node with raw tokens, Strict mode rejects); embedded dollar-quoted bodies not parsed in outer pass.
- **Test cases:**
  1. `SELECT a, FROM t;` → tree with `SELECT_STMT{targets[a, <missing>], FROM_CLAUSE{t}}`, error `ExpectedExpression` at `FROM`; `text()` round-trips.
  2. `SELECT a FROM t WHERE ;` → `WHERE_CLAUSE{<missing expr>}` + error; next statement unaffected.
  3. `SELEC a FROM t; SELECT 1;` → `UNKNOWN_STMT` + valid `SELECT_STMT`.
  4. `SELECT (1 + ;` → errors `ExpectedExpression`, `MissingRParen`; statement closed at `;`.
  5. `SELECT 1 + 2 * 3` → `BIN(+, 1, BIN(*, 2, 3))`.
  6. PG `SELECT x::int[]::text` → nested `CAST_EXPR`.
  7. `SELECT -- c\n a /* d */ FROM t` → comments preserved as trivia tokens at correct positions.
  8. `/* header */\nSELECT 1;` → comment inside `SELECT_STMT` node (attached leading trivia); with a blank line between → comment outside.
  9. 100,000 × `a +` chain → parses without stack overflow.
  10. Strict mode `SELECT 1 FROM t WHERE` → `Err(StrictParseError)`.
- **Prior art and evidence:** RA events/markers/recovery/step limit/trivia attachment [Observed: `crates/parser/src/{event.rs,parser.rs,shortcuts.rs}`]; biome slots/bogus/recovery [Observed: `biome_rowan/src/syntax_factory.rs`, `biome_parser/src/parse_recovery.rs`]; sqlparser-rs depth guard and dialect precedence hooks [Observed: `src/parser/mod.rs:57-63,196`; `src/dialect/mod.rs:949-963`].
- **Used by:** Hacivat | Karagöz (Strict)

### ALG-SQL-PARSE-02: Incremental reparse of a statement
- **Problem:** After an edit, update the document's statement trees with minimal work while keeping identity stable for caches.
- **Inputs / outputs:** `(old_doc: DocSyntax{text, tokens, items: Vec<StatementItem{range, id: StmtId, green}>}, edit: TextEdit{range, insert})` → `new_doc` + `changed: Vec<StmtId>` (statements whose green changed) + `moved: Vec<StmtId>` (only offset changed).
- **Data structures:** `StmtId` = interned `(dialect, statement_text)` (content-addressed, pgls-style) or, under salsa, an interned key over the text; per-document `NodeCache`; token arrays from ALG-SQL-LEX-02.
- **Algorithm:**
  1. **Relex** incrementally (ALG-SQL-LEX-02) → changed token window `[t0, t1)`.
  2. **Resplit locally**: find the first statement item whose range intersects or touches the changed window (and the one before it — an edit may delete a `;`); run ALG-SQL-SPLIT-01 from that item's start until a produced boundary coincides with an old boundary (shifted by delta) → affected items list `A`. Items before are unchanged; items after are moved by `delta`.
  3. For each item in `A`:
     a. If exactly one old item maps to it, and the edit lies within it: try **token-level reparse**: if the changed window is a single token whose kind is unchanged and the kind is in `{WHITESPACE (no newline count change affecting trivia attachment), COMMENT, IDENT/Word (keyword id unchanged), STRING, NUMBER, DOLLAR_STRING (tag unchanged)}` → replace the green token via `replace_with`; errors re-evaluated only for that token.
     b. Else try **node-level reparse**: find the smallest ancestor whose kind is reparsable — `PAREN_SUBQUERY`, `BEGIN_END_BLOCK`, `CASE_EXPR` (only if balanced `CASE…END`), `ARG_LIST`, `COLUMN_DEF_LIST` — and whose new text keeps its delimiters balanced (`(`/`)` count, `BEGIN`/`END` count at its level); reparse only that node with its entry rule (the dialect provides `reparser_for(kind) -> fn(&mut Parser)`); splice with `replace_with`.
     c. Else **reparse the whole statement**.
  4. Recompute `StmtId`s for changed items (content changed ⇒ new id); unchanged-content items keep their id (pure moves).
  5. Merge errors: drop old errors inside reparsed ranges, shift others, add new ones.
  6. Emit `changed`/`moved` for downstream caches (salsa inputs updated only for changed statements — ALG-SQL-PARSE-03).
- **Complexity:** O(edit + resync) lexing + O(size of reparsed node); typical keystroke: one token or one statement (< 10 KB).
- **Edge cases:** edit deletes a `;` (two statements merge → both old ids gone, one new); edit inserts `;` inside a statement (split into two); edit inside a dollar-quoted body (token-level reparse of the outer token; child document for the body is reparsed separately); edit converting an identifier into a keyword (`selec` → `select`) — keyword id changed ⇒ not token-level; edit changing newline count before a statement (trivia attachment changes ⇒ statement reparse); edit in a comment that makes it end early (`*/` typed) ⇒ token kinds change ⇒ resplit; paste of a large block (fallback to full document parse when > N% changed); undo/redo bursts (coalesce edits; salsa cancellation handles stale work); two identical statements in one document (same content id → share parse; positions differ — keep id → range mapping per occurrence).
- **Test cases:** typing `x` into `SELECT a FROM t` (→ `SELECT ax FROM t`) → token-level reparse, 1 changed statement; typing `;` after `t` in `SELECT a FROM t SELECT b` → split; deleting `;` between two statements → merge; editing inside `( SELECT … )` subquery of a 50 KB statement → node-level reparse of the subquery only; renaming in a statement above → statements below "moved" only, no reparse.
- **Prior art and evidence:** RA token/block reparse [Observed: `crates/syntax/src/parsing/reparsing.rs:20-140`]; pgls content-addressed statements and cache retention [Observed: `document.rs:44-66`, `statement_identifier.rs`].
- **Used by:** Hacivat

### ALG-SQL-PARSE-03: Incremental semantic pipeline with salsa
- **Problem:** Organise per-statement analysis (parse, scopes, resolution, types-lite, diagnostics, completion data) so edits and catalog refreshes recompute only what is necessary, with cancellation.
- **Inputs / outputs:** salsa database `SqlDb`; host writes inputs; IDE queries read memoised outputs.
- **Data structures (salsa items):**
  - `#[salsa::input] SourceFile { uri, text: Arc<str>, dialect: DialectId, lex_modes: LexModes }` — Durability LOW.
  - `#[salsa::input] CatalogSnapshot { connection_id, version: u64, data: Arc<Catalog> }` — Durability HIGH (refreshed on schema change / manual refresh).
  - `#[salsa::input] SessionState { connection_id, search_path, temp_objects, settings }` — MEDIUM.
  - `#[salsa::input] FileSettings { uri, connection_id: Option<..>, lint_config }` — MEDIUM.
  - `#[salsa::interned] StmtText { dialect, lex_modes, text: Arc<str> }`.
  - Tracked fns:
    - `split(file) -> Arc<[StmtItem{range, stmt: StmtText}]>` (uses ALG-SQL-SPLIT-01 editor mode; incremental help from ALG-SQL-PARSE-02 via the host keeping previous splits and feeding them as a hint is optional).
    - `parse(stmt: StmtText) -> Arc<Parse>` with `lru = 512` (keyed by content → reused across files and positions).
    - `file_ddl_effects(file, upto_index) -> Arc<CatalogDelta>` — DDL of statements before index (pgls "catalog as of the statement").
    - `effective_catalog(file, index) -> CatalogView` = snapshot ⊕ deltas (persistent maps; cheap).
    - `scopes(stmt, catalog_view) -> Arc<ScopeTree>`; `resolve(stmt, catalog_view) -> Arc<Resolution>`; `diagnostics(file, index)`; `safety_class(stmt) -> SafetyReport` (no catalog needed for syntax-only classification; catalog-aware variant uses function volatility).
- **Algorithm:**
  1. On edit: host applies ALG-SQL-PARSE-02 for immediate UI (highlight/fold use the host's green tree directly, bypassing salsa for lowest latency) and sets `SourceFile.text` (new revision).
  2. `split(file)` re-executes (cheap); statements whose `StmtText` is identical re-intern to the same id → `parse` memo reused; **backdating**: if `split` output equals the old one (e.g. edit only inside a comment that doesn't affect items… it does affect text; equality holds only for whitespace-insensitive projection, so additionally expose `split_shape(file)` = list of `(kind, StmtText)` for consumers that don't need ranges).
  3. Per-statement queries depend on `parse(stmt)` and `effective_catalog(file, index)`. `effective_catalog` depends only on DDL statements before `index` → a non-DDL edit leaves it equal → backdated → `resolve` for later statements is not recomputed.
  4. Catalog refresh sets `CatalogSnapshot.data` with HIGH durability; queries depending on it revalidate.
  5. Readers (completion, hover, diagnostics) run on `db.snapshot()` in worker threads; on a new write they receive `Cancelled` and are retried by the scheduler; diagnostics are published per statement as they complete (progressive).
  6. Memory: LRU on `parse`/`resolve`; drop file data on close; statements interned by content are GC'd when no longer referenced (salsa interned revisions/LRU policy).
- **Complexity:** per keystroke O(split) + O(changed statements × analysis). Splitting a 1 MB file is O(n) but fast (< 2 ms); if needed, make `split` incremental by keeping it outside salsa and inputting per-statement texts instead (`#[salsa::input] StatementSlot`), trading simplicity for speed.
- **Edge cases:** the same statement text in two files with different connections/dialects (key includes dialect + lex modes; catalog-dependent queries take the catalog view as an argument, so no mixing); DDL statement edited (invalidates all later statements' resolution in the file — expected); `SET search_path` statements in the file (session-state delta like DDL); cyclic views / recursive CTE resolution (salsa cycle handling or local fixpoint inside `resolve`); catalog refresh during typing (HIGH durability write cancels readers; acceptable); very large generated files (disable semantic queries above a size threshold, keep lexing/splitting).
- **Test cases:** edit inside statement 50 of 100 → only `parse`/`resolve` of statement 50 rerun (assert via salsa event log); add a column in a `CREATE TABLE` at statement 1 → `resolve` of all later statements rerun; comment-only edit in statement 3 → `parse(stmt3)` reruns (text changed), `resolve(stmt3)` backdates if the AST projection used by resolve is equal (make resolve depend on a trivia-free `ast_shape(stmt)` query to enable this).
- **Prior art and evidence:** RA base-db inputs + tracked parse with LRU [Observed: `base-db/src/lib.rs:203-230`, `editioned_file_id.rs:13-31`]; salsa durability/backdating/cancellation [Observed: `salsa/src/durability.rs`, `cancelled.rs`, book]; pgls catalog-as-of-statement overlay [Observed: `ARCHITECTURE.md` pgls_catalog, `pgls_catalog/src/catalog/overlay.rs`].
- **Used by:** Hacivat

### ALG-SQL-PARSE-04: Dialect-parameterised Pratt expression parser
- **Problem:** Parse expressions with correct precedence/associativity per dialect and session mode, iteratively, with recovery.
- **Inputs / outputs:** parser state, `min_bp`, `dialect.precedence: &PrecTable`, modes (MySQL `PIPES_AS_CONCAT`, `HIGH_NOT_PRECEDENCE`) → events for one expression; `CompletedMarker` or `None` (missing expression).
- **Data structures:** `PrecTable { infix: Map<OpKey, (l_bp, r_bp, NodeKind)>, prefix: Map<OpKey, (r_bp, NodeKind)>, postfix: Map<OpKey, (l_bp, NodeKind)>, nonassoc: Set<OpKey> }` where `OpKey` = token kind, keyword, or keyword pair (`IS NOT`, `NOT LIKE`, `NOT IN`, `NOT BETWEEN`, `SIMILAR TO`, `AT TIME ZONE`, `IS DISTINCT FROM`).
- **Algorithm:**
  1. `lhs = prefix()`: if `prefix` table matches → bump op, `rhs = expr_bp(r_bp)`, wrap `PREFIX_EXPR`; else primary (literal, param, name path, `(`…, special forms, `CASE`, `EXISTS(`, `ARRAY`, `ROW`, `INTERVAL`, `CAST`, function call with `FILTER`/`OVER`/`WITHIN GROUP`). If nothing matches and token ∈ recovery set → return `None` (caller reports missing expression).
  2. Loop: look at the next operator key (multi-token keys via lookahead); if postfix with `l_bp ≥ min_bp` → wrap; if infix with `l_bp ≥ min_bp` → for `BETWEEN` parse `low AND high` with `AND` consumed as part of BETWEEN (r_bp above logical AND); for `IN` parse `( list | subquery )` or PG `IN` with expression; for `LIKE/ILIKE/SIMILAR TO` optional `ESCAPE`; for `IS [NOT] (NULL|TRUE|FALSE|UNKNOWN|DISTINCT FROM x|DOCUMENT|NORMALIZED|JSON …)`; for `::` parse a type name (not an expression); for `[` parse subscript/slice; for `.` field/`*`; for `COLLATE` parse collation name; else `rhs = expr_bp(r_bp)`. Non-associative operators: if the previous operator in this loop had the same precedence and is nonassoc → error `NonAssocChain` (PG `a < b < c`), continue.
  3. Iterative: the loop handles left-assoc chains without recursion; right recursion only for right-assoc/prefix operators (depth counted).
- **Complexity:** O(n) per expression.
- **Edge cases:** unary minus vs binary minus (`a - -b`); PG custom operators (any `Op` token → generic infix at "other operator" precedence; prefix `@`/`~`/`!!`); `OPERATOR(schema.op)`; `NOT` precedence (PG: `NOT a = b` → `NOT (a = b)`); MySQL `!a = b` → `(!a) = b`; `a NOT BETWEEN b AND c AND d`; `x IS NOT DISTINCT FROM y`; `a = ANY(ARRAY[…])`/`= ALL (subquery)`; `ESCAPE` missing operand; `AT TIME ZONE` vs alias `at`; `COLLATE "C"`; `a.b.c[1].d` (PG requires parens for field selection of composite from subscripted?); `CASE` inside operators; JSON operators `->`, `->>`, `#>`, `@>`, `?|`; MySQL `:=` inside SELECT; T-SQL `+` string concat (same token).
- **Test cases:** PG `1 + 2 * 3 ^ 2` → `+(1, *(2, ^(3,2)))`; PG `NOT a = b AND c` → `AND(NOT(=(a,b)), c)`; PG `a < b < c` → error NonAssoc; MySQL with PIPES_AS_CONCAT off: `a || b && c` → `OR(a, AND(b,c))`; `a BETWEEN 1 AND 2 AND b` → `AND(BETWEEN(a,1,2), b)`; `x::int + 1` → `+(CAST(x,int), 1)`; `-2 ^ 2` (PG: unary minus binds tighter than `^`? PG precedence lists `^` above unary minus? — PG docs: unary minus has higher precedence than `^`, so `-2 ^ 2 = 4`) → `^(-2, 2)`.
- **Prior art and evidence:** sqlparser-rs `get_next_precedence` / `parse_infix` hooks [Observed: `src/dialect/mod.rs:949-1124`]; PG precedence [Inferred: PG docs].
- **Used by:** Hacivat | Karagöz

---

## 5. Implications: recommended architecture for Hacivat (client + agent)

### 5.1 Crate layout

```
crates/sql/
  sql-text        # TextSize/TextRange re-export, LineIndex (UTF-8/16/32), edits
  sql-dialect     # DialectId, versions, LexDialect/ParserDialect tables, keyword tables (build-time data)
  sql-lexer       # ALG-SQL-LEX-01/02 (no deps but sql-text, sql-dialect)
  sql-split       # ALG-SQL-SPLIT-01/02, ALG-SQL-PARAM-01
  sql-syntax      # SyntaxKind superset, rowan Language, typed AST (codegen from an ungrammar-like .ungram per dialect family), tree utils
  sql-parser      # grammar: core (ANSI) + modules pg/, mysql/, sqlite/, mssql/, clickhouse/, oracle/; ALG-SQL-PARSE-01/02/04; strict mode
  sql-oracle-pg   # optional (feature `pg-oracle`): pg_query wrapper — parse check, fingerprint, normalize, summary, scan
  sql-catalog     # catalog model (dialect-neutral core + dialect extensions), introspection query sets per dialect, Snapshot (de)serialisation, CatalogDelta from DDL
  sql-sema        # salsa db (ALG-SQL-PARSE-03), scopes, name resolution, light typing, catalog overlay
  sql-ide         # completion, hover, signature help, diagnostics aggregation, folding, semantic tokens, rename, code actions; LSP-shaped (positions as offsets; no lsp-types dependency)
  sql-fmt         # formatter (doc 05 ALG-SQL-FMT-01)
  sql-lint        # rule framework + rules (doc 05)
  sql-safety      # classifier + policy evaluation (doc 05 ALG-SQL-SAFE-01/02); depends on lexer/split/parser/syntax (+ sql-oracle-pg); NO salsa
  sql-testkit     # corpus runners, differential tests vs libpg_query / real servers, snapshot tests
```

- **Hacivat** links `sql-ide`, `sql-fmt`, `sql-lint`, `sql-safety` (advisory "read-only mode" checks, confirmations for destructive statements).
- **Karagöz** links `sql-safety` (+ `sql-split`, `sql-parser`, `sql-oracle-pg`) only. Build size and attack surface stay small; no salsa.
- **Local MCP server** in Hacivat uses `sql-safety` to label tool calls and enforce the user's MCP policy (e.g. read-only by default).

### 5.2 Reuse vs build

| Component | Decision | Reason |
|---|---|---|
| rowan | Reuse (crate) | Proven, MIT/Apache, exactly the needed model |
| salsa | Reuse (crate) | Incremental engine; pin version (API churn between 0.x releases) |
| pg_query (libpg_query) | Reuse behind feature | Exact PG grammar; oracle + Karagöz authority for PG |
| sqlparser-rs | Optional reuse in `sql-safety` (feature `sqlparser-second-opinion`) and `sql-testkit` corpora | Multi-dialect second opinion; Apache-2.0 |
| tree-sitter / tree-sitter-sql | Do not use in core | Our lexer/parser cover highlighting; avoids C runtime |
| sqruff | Do not depend; port rule semantics with attribution | Performance/API mismatch; Apache-2.0 allows porting |
| pgls crates | Do not depend (API not stable, PG-only); port algorithms (MIT) with attribution where code-level reuse happens | Same domain, clean licence |
| Lexer, splitter, parser, sema, IDE features, safety | Build | No existing crate meets lossless + recovering + multi-dialect + Rust |

### 5.3 Phased plan

| Phase | Scope | Exit criteria |
|---|---|---|
| P0 (2–3 wks) | `sql-text`, `sql-dialect`, `sql-lexer` for all six dialects + Generic; `sql-split` editor+execution modes; keyword tables; highlighting & run-statement-at-cursor in Hacivat | Lexer fuzzed (no panics, lossless property), conformance corpus per dialect; PG splitter matches libpg_query split on PG regression corpus (≥ 99.9%) |
| P1 (4–6 wks) | `sql-syntax` + `sql-parser` PG core (SELECT/INSERT/UPDATE/DELETE/MERGE/WITH, common DDL, utility statements as `UNKNOWN_STMT`), strict mode; `sql-safety` v1 for PG using libpg_query as authority + our tree as cross-check; Karagöz enforcement pipeline | Differential accept/reject vs libpg_query on PG regress ≥ 98% for supported statement kinds; safety test suite (doc 05) green |
| P2 (4 wks) | `sql-catalog` introspection for PG/MySQL/SQLite; `sql-sema` scopes + resolution; completion v1 (tables, columns, aliases, CTEs, keywords from parser expectations, FK joins); hover | Completion latency p95 < 20 ms on 1 MB file; scenario suite from pgcli/pgls tests ported |
| P3 (6 wks) | MySQL/MariaDB + SQLite parsers; T-SQL batch-level parser; diagnostics; lint framework + first 20 rules; formatter v1 for PG/MySQL | Formatter idempotency + AST-equivalence checks on corpora |
| P4 | ClickHouse, Oracle-basic parsers; PL/pgSQL child documents; type inference light; rename/refactors; more lint rules; safety for all dialects with sqlparser-rs second opinion | Per-dialect conformance suites vs Docker servers |

---

## 6. i18n considerations

- `SyntaxError` and diagnostics carry `code` + typed args; rendering via Fluent message IDs (`sql-parse-expected-expression = Expected an expression`, Turkish bundle provided). Grammar code never formats English strings.
- Expected-token lists in messages ("expected one of FROM, WHERE") use keyword spellings (untranslated SQL) inside translated templates.
- Offsets are bytes; display columns via the editor's grapheme model; LSP-shaped APIs take/return UTF-16 only at the adapter (`sql-text::LineIndex`).
- Codegen'd node kind names (`SELECT_STMT`) are developer-facing; user-facing names for node kinds in UI (e.g. breadcrumbs "SELECT statement › FROM clause") need message IDs too.

## 7. Risks, pitfalls, open questions

- **Grammar breadth vs. depth:** PG alone has ~200 statement kinds. Mitigation: `UNKNOWN_STMT` raw nodes for unsupported statements (still lossless, still highlighted, safety via libpg_query), prioritise by usage telemetry-free heuristics (DML/DQL/common DDL first).
- **Two grammars for PG (ours + libpg_query) may disagree.** Policy: libpg_query is authoritative for validity/safety; ours for IDE structure. Differential CI catches drift.
- **salsa API churn** (0.x): pin and wrap behind `sql-sema`'s own API.
- **Interning by content** means identical statements share analysis even when their *context* differs (catalog-as-of-statement); ensure all context-dependent queries take the context as an explicit argument (as in ALG-SQL-PARSE-03).
- **Memory** for huge files: NodeCache + LRU; skip semantic analysis beyond thresholds; Karagöz has per-request limits (statement size, token count, depth).
- **Open:** slots (biome) vs plain children (rowan) — decide after prototyping typed accessors for `SelectStmt` with missing clauses.
- **Open:** whether to run libpg_query in-process in Hacivat on every keystroke (cheap, µs-ms) or debounce; pgls runs it per statement on demand.
- **Open:** T-SQL statement-level parsing inside batches without terminators is hard; v1 may support batch-level classification via sqlparser-rs (MsSqlDialect) + our partial parser.
