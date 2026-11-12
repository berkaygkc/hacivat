# 02 — Turkish-Specific and Database-Client-Specific i18n Pitfalls

> Status: research, implementation-grade. Language of this document: English.
> Companion: `01-localization-architecture.md` (message format, runtime, process; ALG-I18N-01 … 04).
> This document specifies ALG-I18N-05 (locale-invariant identifier case folding), ALG-I18N-06 (Turkish-aware UI search), ALG-I18N-07 (grid number parsing/formatting with round-trip fidelity) and ALG-I18N-08 (database server message presentation).

---

## 1. Scope and sources

### 1.1 Questions answered

1. What exactly goes wrong with Turkish dotted/dotless I, where has it broken real database software, and what rule set prevents it in Hacivat, Karagöz and Perde?
2. How must identifiers and keywords be case-folded per database engine, independent of any locale?
3. How should UI lists be sorted, and how should UI search/filter be case-insensitive for Turkish users without breaking ASCII identifiers?
4. How should numbers and dates be displayed, edited, parsed, copied, pasted and exported in a data grid so that values round-trip exactly?
5. What Turkish-language issues (grammar, keyboard layouts, fonts) affect UI strings and shortcuts?
6. What must never be localized in a database client; how should server error messages (which may themselves be localized) be presented?
7. What is needed for future RTL languages (Arabic, Hebrew) when SQL and identifiers are LTR?
8. How do we test all of this (pseudo-localization, length expansion)?

### 1.2 Sources studied

| Source | Version / date | License | How used |
|---|---|---|---|
| PostgreSQL `src/common/kwlookup.c`, `src/backend/parser/scansup.c` | `master`, fetched 2026-11-11 | PostgreSQL License (permissive) | Keyword and identifier folding comments/logic (quoted short phrases; logic described in prose) |
| PostgreSQL docs 18 (18.6): runtime-config-client (`lc_messages` etc.), protocol error fields | fetched 2026-11-11 | PostgreSQL docs license | `lc_messages` privilege rule; `S`/`V`/`C` fields |
| pgsql-bugs/hackers 2001 "Turkish locale bug" (Tom Lane reply, msg `10734.982636214@sss.pgh.pa.us`) | 2001-02-20 | Mailing list | History of keyword folding bug |
| pgsql-bugs BUG #3693 (2007) and "AbstractJdbc2ResultSet getColumnIndex…" thread | 2007 | Mailing list | pgJDBC Turkish bugs |
| DBeaver issue #40901, PRs #40885 (closed, not merged), #42781 (open) | opened 2026-04-24, open on 2026-11-11 [Observed: `gh issue view 40901 -R dbeaver/dbeaver`] | DBeaver CE: Apache-2.0 | Turkish JVM locale bugs (cache lookups, double LIMIT) |
| MySQL 8.4 Reference Manual "Setting the Error Message Language" | fetched 2026-11-11 | Oracle docs | Session `lc_messages` |
| SQL Server docs (sys.messages, SET LANGUAGE) + community threads on Turkish_CI_AS | various | Microsoft docs / forums | Collation-dependent identifier resolution |
| CLDR 49 JSON test data inside ICU4X (`provider/source/tests/data/cldr`) | ICU4X HEAD `a391f6c33b91fec2ec87f581b13716baf68f5fe0` | Unicode License V3 | Turkish number symbols/patterns, dates, plurals, list patterns, week data |
| CLDR `common/collation/tr.xml` | `main`, fetched 2026-11-11 | Unicode License V3 | Turkish collation tailoring rules |
| ICU4X `components/casemap/src/casemapper.rs` | same commit | Unicode License V3 | Turkic case mapping/folding APIs |
| DBeaver docs "Managing Data Formats" | fetched 2026-11-11 | — | Grid formatting options in a competitor |
| JetBrains Inspectopedia "StringToUpperWithoutLocale", IntelliJ support thread on Turkish code completion | fetched via search | — | IDE-side evidence |
| Apache Derby DERBY-329, NetBeans bug 98694, Apache SeaTunnel #12995, ArcadeDB #7900 | via search | various | Same bug class in other DB tools |
| Python 3 `str` case operations (local runtime) | macOS system Python | PSF | Verified Unicode default mappings used in test vectors |

### 1.3 Evidence conventions

[Observed: …], [Inferred], [Unknown] as defined in the library README. Code from PostgreSQL (permissive) is described, not pasted, beyond short quoted comment phrases.

---

## 2. Executive summary

1. **Two different "case" worlds exist and must never mix**: (a) *machine casing* — SQL keywords, identifiers, dialect folding, config keys, enum names, protocol tokens — must be ASCII-only/locale-invariant; (b) *human casing* — UI text — must be locale-aware (Turkish `i`→`İ`, `ı`→`I`). The classic "Turkish locale bug" is (b)'s rules leaking into (a). [Observed: PostgreSQL `kwlookup.c` comment "We must not use tolower() since it may produce the wrong translation in some locales (eg, Turkish)"]
2. **The bug is alive in 2026**: DBeaver #40901 (opened 2026-04-24, still open) — on a Turkish JVM, `items` is cached under `İTEMS` so lookups for `ITEMS` miss, and the auto-limit transformer fails to see a lowercase `limit` and appends a second `LIMIT 200`, producing invalid SQL. [Observed: github.com/dbeaver/dbeaver/issues/40901]
3. **Rust's standard casing is locale-independent**, so the Java/.NET default-locale trap does not exist in Rust — but three Rust-specific traps do: using full Unicode `to_lowercase()` where the database uses ASCII-only folding (`İ`→`i̇`, two code points), passing the UI locale into ICU4X case mapping for identifiers, and JavaScript `toLocaleUpperCase()`/`localeCompare()`/CSS `text-transform` in a Tauri/web frontend. [Observed: Python/Rust share Unicode default mappings; verified `'İ'.lower() == 'i̇'` (len 2)]
4. **Server-side collation can reintroduce the bug**: SQL Server databases with `Turkish_CI_AS` resolve identifiers with Turkish rules — community reports of `Invalid object name 'information_schema.tables'` and `@ID` ≠ `@id`. Hacivat's catalog queries must use the exact canonical case of system objects (`INFORMATION_SCHEMA.TABLES`, `sys.objects`) and never `lower()/upper()` identifiers in SQL. [Observed: SolarWinds/DNN/Umbraco threads via search; Inferred: rule] [Observed: docker test, mcr.microsoft.com/mssql/server:2022-latest (16.0.4315.2, amd64 emulation) — `information_schema.tables` → Msg 208 in a `Turkish_CI_AS` database; `@ID` ≠ `@id` and `#Items` ≠ `#items` only when the **instance** collation is Turkish (variables and tempdb follow the instance, not the database); see verification/01-db-behavior-tests.md#mssql-turkish]
5. **Identifier folding is per dialect**: PostgreSQL folds unquoted identifiers by ASCII-only `A–Z → a–z` and leaves non-ASCII bytes alone in multibyte encodings (so `MÜŞTERİ` becomes `mÜŞterİ`); SQLite compares ASCII case-insensitively; Oracle/Db2/Snowflake fold to upper; MySQL depends on `lower_case_table_names` and object kind; SQL Server depends on collation. Specified in ALG-I18N-05. [Observed: PostgreSQL `scansup.c` logic] [Observed: docker test, postgres:16/17/18 in an ICU `tr-TR` database (`ITEMS`→`items`, `MÜŞTERİ`→`mÜŞterİ`) and SQLite 3.54.1 (`müşteri` matches `Müşteri`, `MÜŞTERİ` does not), see verification/01-db-behavior-tests.md#pg-turkish and #sqlite-case]
6. **UI search must be "Turkish-tolerant", not "Turkish-correct"**: a Turkish user typing `ID` in the object filter must still find `id`. Plain Turkic folding would turn `I` into `ı` and miss it. Our search key folds all of `I i İ ı` (and `i`+U+0307) to one class, with optional accent-insensitivity (`ş→s`, `ğ→g` …) for users typing on US keyboards. ALG-I18N-06.
7. **Sorting UI lists uses CLDR Turkish collation** (`c<ç`, `g<ğ`, `ı<i` as distinct letters, `ö`, `ş`, `ü`) with numeric ordering, plus a code-point tiebreak so distinct identifiers (`Users` vs `users`) never compare equal. Collation is for *ordering*, never for *identity*. [Observed: CLDR `tr.xml` rules]
8. **Turkish numbers use `,` decimal and `.` grouping; percent is a prefix (`%25`); the CLDR list separator is `;`** — the reason Excel in Turkish locale expects `;`-delimited CSV. [Observed: CLDR 49 tr `symbols-numberSystem-latn`: decimal `,`, group `.`, list `;`; percent pattern `%#,##0`]
9. **Grid default = raw machine representation** (no grouping, `.` decimal, ISO-8601 dates) with an opt-in localized display mode. Editing always pre-fills the raw value; parsing tries machine format first, locale format second, and flags ambiguity (`1.234` in Turkish) instead of guessing. Values are carried as exact decimal strings, never through `f64`. ALG-I18N-07.
10. **Copy defaults to raw/machine-readable** (TSV, `.` decimal) with "Copy for spreadsheet (regional format)" as an explicit alternative; paste goes through the same parser.
11. **PostgreSQL `money` output depends on the server's `lc_monetary`** — a Turkish server prints `₺1.234,56`-style values; editing/exports should cast to `numeric` [Inferred: from PostgreSQL docs on `money`/`lc_monetary`].
12. **Server error messages**: PostgreSQL's `lc_messages` can only be changed by superusers or roles granted `SET` on it — a normal client cannot force English per session; MySQL allows per-session `SET lc_messages`; SQL Server uses `SET LANGUAGE`. Always show the server text verbatim, plus the non-localized identifiers (`SQLSTATE` field `C`, non-localized severity `V` since PG 9.6) and a localized *explanation* keyed by SQLSTATE/error number. ALG-I18N-08. [Observed: PostgreSQL 18 docs; MySQL 8.4 docs]
13. **Never localize**: SQL keywords, identifiers, data values, `NULL`, booleans as stored, error codes/SQLSTATEs, connection-string keys, JSON/YAML keys, CLI flags, log lines, MCP metadata.
14. **Turkish grammar breaks string concatenation**: suffixes after placeholders depend on vowel harmony and pronunciation (`PostgreSQL'e`, `MongoDB'ye`, `SQLite'a`); messages must be structured so variables never carry inflectional suffixes.
15. **Turkish Q layout puts `ı` where US has `I`** and makes `[ ] { } \ | @ ~` AltGr combinations; on Windows AltGr = Ctrl+Alt, so default shortcuts on `Ctrl+Alt+Q/7/8/9/0/-/*` would steal characters needed to type SQL. Shortcut matching must normalize `ı`/`I`/`i`. [Inferred: layout knowledge; verify against Microsoft KBDTUQ/KBDTUF layout tables]
16. **Fonts**: `ğ Ğ ı İ ş Ş` are in Latin Extended-A; a web font subset limited to "latin" silently falls back for exactly these letters. `fi` ligatures erase the dot distinction (`fi` vs `fı`) unless `lang="tr"` enables the font's Turkish `locl` behavior. [Inferred: font engineering knowledge]
17. **RTL readiness**: SQL editor always `dir="ltr"`; grid cells `dir="auto"`; Fluent's FSI/PDI isolation on all placeables; bidi control characters inside identifiers/data shown visibly (Trojan-Source class, CVE-2021-42574). The UI toolkit's bidi/shaping capability is a stack-selection criterion.
18. **Expansion budget**: design short labels for +100 %, long sentences for +30 %; Turkish typically +20–35 % over English and words are long (agglutination) — avoid fixed-width buttons. [Inferred: industry guidance]

---

## 3. Detailed findings

### 3.1 The dotted/dotless I problem — Unicode facts

#### 3.1.1 The four letters

| Code point | Glyph | Name | Default (root) lower | Default upper | Turkish/Azeri lower | Turkish/Azeri upper |
|---|---|---|---|---|---|---|
| U+0049 | I | LATIN CAPITAL LETTER I | `i` (U+0069) | `I` | **`ı` (U+0131)** | `I` |
| U+0069 | i | LATIN SMALL LETTER I | `i` | `I` | `i` | **`İ` (U+0130)** |
| U+0130 | İ | LATIN CAPITAL LETTER I WITH DOT ABOVE | **`i̇` = U+0069 U+0307** (2 code points) | `İ` | `i` | `İ` |
| U+0131 | ı | LATIN SMALL LETTER DOTLESS I | `ı` | `I` | `ı` | `I` |

[Observed: verified with Python 3: `'İ'.lower()` → `'i̇'` (length 2), `'ı'.upper()` → `'I'`, `'İ'.casefold()` → `'i̇'`; ICU4X doc tests: `uppercase_to_string("istanbul", tr) == "İSTANBUL"`, `lowercase_to_string("CONSTANTINOPLE", tr) == "constantınople"` at `icu4x/components/casemap/src/casemapper.rs:273,307`]

Consequences:

- **Casing is not reversible**: under default rules `lower(upper("ı")) = "i"`; under Turkish rules `upper(lower("I")) = "I"` but `lower(upper("i"))` in default then Turkish mixes produce `ı`.
- **Length changes**: `İ` lower-cases to two code points in default rules; byte offsets and character counts shift (matters for highlighting search hits and cursor positions).
- **Case folding has a Turkic option** (CaseFolding.txt status `T`): default fold maps `I→i`, `İ→i̇`; Turkic fold maps `I→ı`, `İ→i`. ICU4X exposes both: `fold_string` and `fold_turkic_string` (e.g. `fold_turkic_string("İstanbul") == "istanbul"`, `fold_turkic_string("Istanbul") == "ıstanbul"`). [Observed: `casemapper.rs:408-440`]
- **NFD of `İ`** is `I` + U+0307 COMBINING DOT ABOVE. [Observed: Python `unicodedata.normalize('NFD','İ')` → `[0x49, 0x307]`] Text arriving decomposed (some macOS file names, pasted text) must be NFC-normalized before comparisons.
- Other non-Turkish casing surprises in the same code paths: `ß`.upper → `SS`, `ﬁ`.upper → `FI`, final sigma `Σ`→`ς` at word end; Lithuanian retains dots in some contexts; these are why "length-preserving ASCII" is the only safe folding for machine tokens. [Observed: Python checks `'ß'.upper()=='SS'`, `'ﬁ'.upper()=='FI'`]

#### 3.1.2 Behavior by platform (why the bug class exists)

| Platform | Default upper/lower | Locale-sensitive API | Notes |
|---|---|---|---|
| Java | `String.toUpperCase()` uses **default JVM locale** | `toUpperCase(Locale)` | Root cause of pgJDBC (2007), Derby, NetBeans, DBeaver (2026), SeaTunnel, ArcadeDB bugs. JetBrains ships an inspection "Call to `String.toUpperCase()` or `toLowerCase()` without locale". [Observed: jetbrains.com/help/inspectopedia/StringToUpperWithoutLocale.html] Kotlin 1.5 introduced locale-agnostic `uppercase()/lowercase()` and deprecated `toUpperCase()` [Inferred: Kotlin release notes]. |
| .NET | `ToUpper()`/`ToLower()` use **CurrentCulture** | `ToUpperInvariant()`, `ToUpper(CultureInfo)` | Microsoft docs: only Turkish and Azerbaijani cultures are affected for I/i. [Observed: learn.microsoft.com "Performing culture-insensitive case changes"] Analyzers CA1304/CA1311 [Inferred]. |
| C/POSIX | `toupper`/`tolower` depend on `LC_CTYPE` | `toupper_l` | PostgreSQL's lexer history. |
| Rust | `str::to_uppercase/to_lowercase` are **locale-independent** Unicode default mappings; `to_ascii_*` ASCII-only | ICU4X `CaseMapper` with `LanguageIdentifier` | No default-locale trap, but the *full* Unicode mapping is still wrong for SQL machine tokens (length changes, non-ASCII folding). |
| JavaScript | `toUpperCase/toLowerCase` locale-independent | `toLocaleUpperCase(locale?)` uses host locale; `localeCompare`, `Intl.Collator` default to host locale; CSS `text-transform` uses the element's `lang` | Tauri/Perde web risks. |
| Python | locale-independent | — | Used here for vector verification. |
| PostgreSQL server | `upper()/lower()/ILIKE` follow the column/database **collation** (libc or ICU) | `COLLATE "C"` / `"tr-x-icu"` | Turkish-collated databases produce `upper('i') = 'İ'`. [Observed: pgsql-bugs #12806 "Upper and Lower functions is not working as expected at some Turkish character sets on Windows OS" via search] [Observed: docker test, postgres:16/17/18, ICU `tr-TR` database: `upper('i')='İ'`, `lower('ITEMS')='ıtems'`, and `'ITEMS' ILIKE 'items'` is **false** while `'İTEMS' ILIKE 'items'` is true; the official image has no libc `tr_TR` locale; see verification/01-db-behavior-tests.md#pg-turkish] |

### 3.2 Real-world bug record (the "Turkish locale bug" in database software)

| Year | Software | Symptom | Root cause | Fix pattern | Evidence |
|---|---|---|---|---|---|
| 2001 | PostgreSQL server (lexer) | Under a Turkish locale, upper-case keywords like `INSERT`, `UNION` fail to parse (folded to forms with dotless ı) | Identifier/keyword case-folding used locale-dependent `isupper()`/`tolower()` | Initially contested (Tom Lane wanted folding consistent with `lower()`); eventually keywords use ASCII-only folding | [Observed: postgresql.org/message-id/10734.982636214@sss.pgh.pa.us; `kwlookup.c` header comment "deliberately use a dumbed-down case conversion that will only translate 'A'-'Z' into 'a'-'z'"] |
| current | PostgreSQL identifier downcasing | — (design) | Compromise: ASCII-only downcasing for 7-bit chars; `tolower()` only for high-bit chars in **single-byte** encodings; comment cites Turkish | — | [Observed: `scansup.c` `downcase_identifier` comment: "Turkish may do strange things with 'i' and 'I'"] |
| 2007 | pgJDBC (8.2-era) | `ResultSet.getLong("columnName")` throws "missing column" under `tr_TR` (also Swedish reported); Hibernate breaks | `getColumnIndex` lower-cased column labels with default locale | Use `Locale.ENGLISH`/`Locale.ROOT` | [Observed: pgsql-bugs BUG #3693 (msg 200710241623.l9OGNblf022002); postgrespro thread 1932312] |
| ~2005 | Apache Derby | Identifier upper-casing broken on Turkish machines | default-locale upper-casing | Always upper-case in `Locale.ENGLISH` | [Observed: issues.apache.org/jira/browse/DERBY-329 via search] |
| ~2007 | NetBeans | Turkish-locale failures | missing `Locale.ENGLISH` in `to{Upper,Lower}Case` | explicit locale | [Observed: netbeans bugzilla 98694 via search] |
| — | IntelliJ platform | Code completion fails to match `i`/`I` names under Turkish regional settings | Turkish case mapping in matching | — | [Observed: intellij-support community post 207009025 via search] |
| — | SQL Server with `Turkish_CI_AS` | `Invalid object name 'information_schema.tables'`; `@ID` and `@id` treated as different variables; invalid column `ID` in apps | Identifier resolution follows database/server collation | Use exact case or a non-Turkish collation | [Observed: SolarWinds thwack, DNN forum, Umbraco forum threads via search] |
| — | Telerik Reporting | "SQL Query using the 'i' character fails when using Turkish culture settings" | culture-sensitive casing in SQL handling | — | [Observed: feedback.telerik.com/reporting/1547336 via search] |
| 2026 | DBeaver CE 26.0.4 | (1) metadata cache misses for any identifier containing `i` → repeated catalog queries; (2) `select 1 limit 10` + auto-limit → second `LIMIT 200` appended → syntax error | `toUpperCase()/toLowerCase()` without locale in `AbstractObjectCache` and `QueryTransformerLimit` | `Locale.ROOT` (PR #40885 closed unmerged; PR #42781 open as of 2026-11-11 — corrected after verification) | [Observed: github.com/dbeaver/dbeaver/issues/40901] |
| 2026 | Apache SeaTunnel JDBC connector | Turkish locale bypasses SQL set-operator/complex-clause guards and rewrites the user's query | default-locale keyword matching | — | [Observed: github.com/apache/seatunnel/issues/12995 (title via search)] |
| 2026 | ArcadeDB | On a Turkish-locale server `COLLATE ci` silently builds a case-sensitive index; `NULL_STRATEGY index` rejected | keyword arguments normalized with default locale | — | [Observed: github.com/ArcadeData/arcadedb/issues/7900 (title via search)] |

Pattern: **every** instance is a machine token (keyword, identifier, option name) passed through human-language casing. The DBeaver auto-limit case is directly relevant to Hacivat, which will also implement auto-limit/pagination rewriting.

### 3.3 Rule set for Hacivat, Karagöz and Perde ("Turkish rules")

| Rule | Statement | Enforcement |
|---|---|---|
| TR-1 | SQL keyword recognition uses ASCII case-insensitive comparison (`eq_ignore_ascii_case`) on the raw token; never Unicode folding, never ICU with a locale. | Lexer/parser code review; clippy `disallowed_methods` on `str::to_lowercase/to_uppercase` in `sql-*` crates (allow-list exceptions). |
| TR-2 | Keyword *re-casing* (formatter "UPPER keywords") uses `to_ascii_uppercase`/`to_ascii_lowercase` and only touches tokens classified as keywords. | Same. |
| TR-3 | Identifier normalization follows ALG-I18N-05 per dialect; result used for *matching*, never displayed or sent unless the dialect would produce the identical string. | Unit tests with Turkish vectors. |
| TR-4 | Identifier identity in caches/maps uses the server-reported exact name (`pg_class.relname`, etc.) as key; case-insensitive lookups use ALG-I18N-05 keys, never ICU collation equality. | Cache API types (`IdentKey` newtype). |
| TR-5 | Catalog/introspection SQL never applies `lower()/upper()/ILIKE` to identifiers server-side; compares exact names or filters in Rust; uses canonical case for system objects (`INFORMATION_SCHEMA.TABLES` upper-case; `sys.objects` lower-case on SQL Server; `pg_catalog` lower-case) so Turkish server collations cannot break them. | Introspection query review; test against SQL Server DB with `Turkish_CI_AS`, PostgreSQL DB with `tr-x-icu`/`tr_TR.utf8` default collation. |
| TR-6 | UI text casing (titles, buttons) is done by translators in the catalog, not by code; if code must case-transform human text, use ICU4X `CaseMapper` with the *text's* language (UI language), and on the web set `lang` correctly. Prefer no `text-transform` on translated strings. | Lint for `text-transform: uppercase` in CSS of translatable elements; review. |
| TR-7 | Data values are never case-transformed for display; filters that compare data case-insensitively are executed by the database (with its collation) unless explicitly client-side (then ALG-I18N-06 semantics, documented as "approximate"). | Grid filter design. |
| TR-8 | Any locale-sensitive operation names the locale explicitly in the API (`ui_locale`, `format_locale`, `Locale::UND`); no hidden "current locale" global in non-UI crates. | Crate boundaries: `sql-*`, `driver-*`, `karagoz-*` crates do not depend on `hacivat-l10n`. |
| TR-9 | Frontend JS: never call `toLocaleUpperCase/LowerCase()` without an explicit locale; `localeCompare` and `Intl.Collator` only via our helpers with explicit locale and only for *ordering*; identifiers/SQL rendered inside elements with `lang="en"` and `translate="no"`. | ESLint rules (`no-restricted-syntax` on `toLocaleUpperCase`, `localeCompare`). |
| TR-10 | The whole Rust and JS test suites also run under `LANG=tr_TR.UTF-8`/`LC_ALL=tr_TR.UTF-8` and browser locale `tr-TR` (a "Turkey test" CI job). | CI matrix. |
| TR-11 | Text comparisons are performed on NFC-normalized input (identifiers from servers are taken verbatim; user input is NFC-normalized before matching but never before being sent as SQL text). | Input layer. |
| TR-12 | Hash keys, file names, config keys, enum-from-string parsing (`"INFO".parse::<Level>()`) use ASCII-insensitive or exact matching. | Review; `serde(rename_all)` with exact names. |

### 3.4 Identifier semantics per database (what "case-insensitive" means)

| Engine | Keywords | Unquoted identifiers | Quoted identifiers | Server-side influence | Hacivat consequence |
|---|---|---|---|---|---|
| PostgreSQL | ASCII-insensitive (`kwlookup.c` ASCII-only) | Folded to lower case: `A–Z → a–z` only; high-bit bytes left alone in multibyte encodings (UTF-8); in single-byte encodings high-bit uppercase may be `tolower()`ed per server locale; truncated to `NAMEDATALEN-1` (63) bytes | Case-sensitive, exact | Single-byte encodings (e.g. `LATIN5`/ISO-8859-9 Turkish) make non-ASCII folding depend on server `LC_CTYPE` [Observed: `scansup.c` `enc_is_single_byte && IS_HIGHBIT_SET(ch) && isupper(ch)`] | Quote any identifier that is not entirely `[a-z_][a-z0-9_$]*` and not a reserved keyword; to predict folding for UTF-8 DBs use ASCII-only lowering; for single-byte encodings, ask the server (`SELECT quote_ident($1)` / test via `select $$…$$::regclass`) rather than guessing. |
| MySQL / MariaDB | ASCII-insensitive | Database/table names: case sensitivity depends on file system and `lower_case_table_names` (0/1/2); column, index, stored routine, event names are case-insensitive; aliases differ by version | Backticks; same rules for case | Column-name comparisons use the server's identifier collation (`utf8mb3_general_ci`-like) [Observed: docker test, mysql:8.0/8.4 — column `Müşteri` matched as `MÜŞTERİ`, `ID` as `İD`, but not as `ıd` or `MUSTERI`; with `lower_case_table_names=0` table `Items` ≠ `items`; see verification/01-db-behavior-tests.md#mysql-turkish] | Read `lower_case_table_names` at connect; build keys accordingly; never apply locale casing. |
| SQL Server | ASCII-insensitive | Case sensitivity of object names = **database collation** (catalog collation); variables/labels follow server/instance collation | `[]` or `""` (QUOTED_IDENTIFIER) | `Turkish_CI_AS` makes `i`/`I` different letters (`I`↔`ı`, `i`↔`İ`) → `information_schema.tables` fails; `ID` ≠ `id` | Read database collation (`DATABASEPROPERTYEX(DB_NAME(),'Collation')`); if Turkish (`Turkish_*`, `Azeri_*`), use exact-case identifiers everywhere; generated SQL quotes identifiers with their exact catalog case. |
| SQLite | ASCII-insensitive | Case-insensitive **ASCII only** (`A–Z` = `a–z`; non-ASCII compared exactly) [Observed: docker test, SQLite 3.54.1 — `upper('iı')='Iı'`, `lower('İI')='İi'`, `'I' LIKE 'i'`=1, `'İ' LIKE 'i'`=0, `'Ş' LIKE 'ş'`=0; see verification/01-db-behavior-tests.md#sqlite-case]; `LIKE` is ASCII-only case-insensitive by default; `upper()/lower()` ASCII-only unless ICU extension | `""`, `[]`, backticks | None (no locale) | Same ASCII keying as PostgreSQL but without folding on creation (names preserved as written). |
| Oracle (future) | ASCII-insensitive | Folded to UPPER (ASCII semantics for keywords; identifier upper-casing of non-ASCII per database character set rules [Unknown]) | Exact | NLS settings affect *data* functions, not identifier folding [Inferred] | Upper-ASCII keying. |
| ClickHouse | Case-insensitive for many keywords; some functions case-insensitive | **Case-sensitive** identifiers | backticks/`""` | — | Exact keys. |
| MongoDB | n/a | Field/collection names case-sensitive, exact | n/a | Collation on *queries* only | Exact keys; never fold. |
| Redis | Commands ASCII-insensitive | Keys are binary-safe byte strings | n/a | — | Never fold or normalize keys (not even NFC). |

### 3.5 Server-side collation traps in introspection queries

1. **Do not write** `WHERE lower(table_name) = lower($1)` in catalog queries: on a database whose default collation is Turkish, `lower('ITEMS') = 'ıtems'` ≠ `lower('items') = 'items'`. Compare exact names (`relname = $1`) or fetch and filter in Rust with ALG-I18N-05 keys. [Observed: docker test, postgres:16/17/18 ICU `tr-TR` database — `WHERE lower(relname) = lower('ITEMS')` and the `information_schema.tables` variant return 0 rows while `relname = 'items'` returns the table; see verification/01-db-behavior-tests.md#pg-turkish]
2. **Ordering in catalog queries**: `ORDER BY relname` uses the database default collation; with `tr_TR` the order differs from `C`. If Hacivat merges/diffs lists from two servers (schema compare), sort in Rust with an explicit comparator; or use `ORDER BY relname COLLATE "C"` in PostgreSQL. [Observed: docker test, postgres:16/17/18 — `text` columns order differently under the tr-TR default vs `"C"`; note `relname` itself (type `name`) already has collation `"C"`; see verification/01-db-behavior-tests.md#pg-turkish]
3. **SQL Server**: system catalog views exist with fixed names; with a case-sensitive or Turkish database collation, `information_schema.tables` (lower) fails while `INFORMATION_SCHEMA.TABLES` works; `sys.objects` must be lower-case. Hacivat's query templates must use canonical case (tests against a `Turkish_CI_AS` database and a `Latin1_General_CS_AS` database). [Observed: thwack.solarwinds.com community report; Inferred: canonical-case rule] [Observed: docker test, SQL Server 2022 — Turkish_CI_AS DB: `information_schema.tables`, `Information_Schema.Tables`, `SYS.INDEXES` → Msg 208, `SYS.OBJECTS` works (no `i`); CS_AS DB: `SYS.OBJECTS` and `information_schema.tables` → Msg 208; canonical case works in both; see verification/01-db-behavior-tests.md#mssql-turkish]
4. **MySQL**: `information_schema` table and column names are upper-case (`TABLES`, `TABLE_NAME`); use the canonical case anyway for portability. (corrected after verification) *Value* comparisons there are **not** case-insensitive on Linux defaults: `TABLE_NAME` has collation `utf8mb3_bin` with `lower_case_table_names=0`, so `TABLE_NAME = 'ITEMS'` does not find `Items`. MySQL `LOWER()/UPPER()` are **not** Turkish-tailored even under `utf8mb4_tr_0900_ai_ci` (`LOWER('ITEMS')='items'`, `UPPER('ı')='I'`), whereas comparisons/`LIKE` under that collation are (`'ITEMS' LIKE 'items'` = 0, `'İTEMS' LIKE 'items'` = 1). [Observed: docker test, mysql:8.0 (8.0.46)/8.4 (8.4.13), see verification/01-db-behavior-tests.md#mysql-turkish]
5. **Search in the database** (e.g. "find column named like `%id%` across all schemas"): pushdown with `ILIKE` on PostgreSQL uses the column collation of `name` columns (`C` for `name` type in `pg_catalog` → ASCII-only case-insensitivity) [Observed: docker test, postgres:16/18 — `pg_collation_for(relname)` = `"C"` (also after `::text`); `relname ILIKE 'ITEMS'` matches `items`, `relname ILIKE 'İTEMS'` does not, even in a tr-TR database; see verification/01-db-behavior-tests.md#pg-turkish], so `ILIKE '%id%'` will not match `İD` — acceptable and predictable; Hacivat's client-side search (ALG-I18N-06) is where Turkish tolerance lives.

### 3.6 Collation and sorting in the UI

#### 3.6.1 Turkish collation

CLDR's Turkish tailoring (standard collation) [Observed: CLDR `common/collation/tr.xml`]:

```
&C<ç<<<Ç
&G<ğ<<<Ğ
&[before 1]i<ı<<<I
&i<<<İ
&O<ö<<<Ö
&S<ş<<<Ş
&U<ü<<<Ü
```

Meaning: `ç`, `ğ`, `ö`, `ş`, `ü` are separate letters sorted after their base letters (primary difference, not accents); `ı` is a separate letter *before* `i`, with `I` as its upper-case; `İ` is the upper-case of `i`. Alphabet: a b c ç d e f g ğ h ı i j k l m n o ö p r s ş t u ü v y z (q, w, x sort in their Latin positions).

Worked example — the words `çay cam zil ılık istek Irmak İzmir ördek oda şeker sabun ğ gül ünlü uçak`:

| Comparator | Result |
|---|---|
| Code point (byte) order | `Irmak cam gül istek oda sabun uçak zil çay ördek ünlü ğ İzmir ılık şeker` [Observed: Python `sorted()`] — useless for humans |
| Root/English collation (ICU `und`) | `cam çay ğ gül Irmak istek İzmir ılık oda ördek sabun şeker uçak ünlü zil` [Observed: docker test, PostgreSQL 16/17/18 `"und-x-icu"` and MySQL 8.0/8.4 `utf8mb4_0900_ai_ci` produce exactly this order; see verification/01-db-behavior-tests.md#pg-turkish] |
| Turkish collation (`tr`) | `cam çay gül ğ ılık Irmak istek İzmir oda ördek sabun şeker uçak ünlü zil` [Observed: docker test, PostgreSQL 16/17/18 `"tr-x-icu"`/ICU `tr-TR` database default and MySQL 8.0/8.4 `utf8mb4_tr_0900_ai_ci` produce exactly this order; see verification/01-db-behavior-tests.md#pg-turkish and #mysql-turkish] |

#### 3.6.2 Rules for Hacivat lists

| List | Comparator | Rationale |
|---|---|---|
| Object tree (schemas, tables, columns) | ICU4X `Collator` for the `regional_format` locale (not the UI language: a Turkish-region user running an English UI still expects Turkish alphabet order), strength tertiary, numeric ordering on (`table2 < table10`), then tiebreak by code point of the exact name | Humans scan alphabetically; numeric ordering helps partitioned tables (`events_2026_02`, `events_2026_10`); tiebreak keeps `Users` and `users` (both legal in PostgreSQL) distinct and stable. Toggle in tree settings: "Sort like the database (binary)". |
| Columns in a table | **Ordinal position** by default (DB order is meaningful); alphabetical as option | DB clients conventionally show column order. |
| Connection list, saved queries, Perde users/orgs | Collator (regional format) + numeric | Human names. |
| Result grid client-side sort | Default: re-query with `ORDER BY` (server collation = correct semantics). Client-side sort (for already-fetched, non-re-queryable results): if the column's collation is known and maps to an ICU locale (`tr-x-icu`, `tr_TR.utf8`, `Turkish_CI_AS`, `utf8mb4_tr_0900_ai_ci`), use that ICU collator; else code-point order for text, numeric for numbers; show a hint "sorted locally (may differ from database order)". | Data must not silently be ordered by UI-language rules. |
| Auto-completion candidates | Ranking by match quality first (prefix > word-start > fuzzy), then by code point | Collation is irrelevant for ranking; determinism matters. |
| Perde audit log | Time order | — |

Collation **equality is never identity**: `Collator::compare(a,b) == Equal` (at secondary strength `İ`=`i`) must not deduplicate identifiers or keys (TR-4).

### 3.7 Search and filter case-insensitivity in the UI

Problem cases for a Turkish user filtering the object tree:

| User types | Wants to find | Default Unicode fold | Turkic fold | Turkish-tolerant (ours) |
|---|---|---|---|---|
| `ID` (Turkish keyboard, Caps or Shift+ı-key gives `I`) | `id`, `user_id` | `id` ✓ | `ıd` ✗ | ✓ |
| `id` | `ID`, `Id` | ✓ | ✗ (`ID`→`ıd`) | ✓ |
| `istanbul` | `İSTANBUL_SUBE` | ✗ (`İ`→`i̇` ≠ `i`) | ✓ | ✓ |
| `ılık` | `ILIK` | ✗ (`I`→`i`) | ✓ | ✓ |
| `musteri` (US keyboard) | `müşteri` | ✗ | ✗ | ✓ only with accent-insensitive option (default ON for search) |
| `Strasse` | `straße` | ✓ (`ß`→`ss`) | ✓ | ✓ |

Hence ALG-I18N-06: a search key that collapses `{I, i, İ, ı, i+◌̇}` to `i`, applies full default case folding for the rest, optionally strips diacritics (Mn after NFD) and maps Turkish letters to ASCII bases (`ç→c, ğ→g, ı→i, ö→o, ş→s, ü→u`), and keeps an index map for highlighting. The *precision* loss (`sık` "frequent" ~ `sik` vulgar) is acceptable in a filter box; it never affects SQL.

### 3.8 Number formatting, editing, copy-paste

#### 3.8.1 Turkish number conventions (CLDR 49)

| Item | Turkish (`tr`) | English (`en`) | Evidence |
|---|---|---|---|
| Decimal separator | `,` | `.` | [Observed: CLDR 49 `tr/numbers.json` `symbols-numberSystem-latn.decimal`] |
| Grouping separator | `.` | `,` | same, `group` |
| List separator | `;` | `;` (CLDR en also `;`, but Excel/Windows en-US uses `,`) | same, `list`; Windows list-separator behavior [Inferred] |
| Decimal pattern | `#,##0.###` | `#,##0.###` | `decimalFormats…standard` |
| Percent pattern | `%#,##0` → `%25` | `#,##0%` → `25%` | `percentFormats…standard` |
| Currency pattern | `¤#,##0.00` → `₺1.234,56`; accounting `¤#,##0.00;(¤#,##0.00)` | `¤#,##0.00` | `currencyFormats…standard/accounting` |
| Minus / plus | `-` / `+` | same | `minusSign`, `plusSign` |
| Exponent | `E` | `E` | `exponential` |
| Infinity / NaN | `∞` / `NaN` | same | `infinity`, `nan` |
| Plural (cardinal) | `one: n = 1` (incl. 1.0), `other` | `one: i = 1 and v = 0`, `other` | [Observed: CLDR 49 `plurals.json`] |
| Plural (ordinal) | only `other`; ordinals written `1.` `2.` (period suffix) | one/two/few/other | [Observed: `ordinals.json` tr has only `other`] |

Ordinal note: in Turkish, `3.` *means* "third" — a trailing period after a number is not a decimal point; never parse `3.` from Turkish text as `3.0`. [Inferred: Turkish orthography]

#### 3.8.2 Display modes in the result grid

| Mode | Integers | Decimals | Dates | Use |
|---|---|---|---|---|
| **Raw (default)** | `1234567` | `1234.50` (scale as returned by server) | ISO-8601: `2026-11-11`, `2026-11-11 14:05:09.123456+03` | Fidelity: what you see is what SQL accepts; copy-paste into queries works; matches DataGrip/psql habits. |
| Localized | `1.234.567` (tr) | `1.234,50` | `11.11.2026 14:05:09` (tr short date + medium time) | Reading large numbers, business users. |
| Mixed (opt) | grouping on, `.` decimal | — | ISO | Some users want only digit grouping. |

DBeaver offers per-profile locale, "Disable numeric formatting" (use driver/native formatting), grouping toggles, min/max fraction digits, rounding mode, and an option to skip formatting ID columns. [Observed: dbeaver.com/docs/dbeaver/Managing-Data-Formats/] We adopt a simpler model: raw by default, localized as opt-in per user/connection/column, and **never round silently** (if a localized display limits fraction digits, the cell shows an indicator and the tooltip shows the full raw value).

Rules:

1. Exact values: NUMERIC/DECIMAL arrive as text from drivers (PostgreSQL text protocol, or binary NUMERIC decoded to a decimal string); keep the string; format via ICU4X `Decimal::try_from_str` (arbitrary precision) — **never via `f64`**.
2. BIGINT beyond 2^53 must never pass through JS `number` in the Tauri frontend — send strings to the frontend.
3. Floats (`real`, `double precision`): display the shortest round-trip representation (as servers do with `extra_float_digits=1`, PostgreSQL ≥12 default [Inferred]); `-0`, `NaN`, `Infinity`, `-Infinity` displayed as the server spells them in raw mode; localized mode uses CLDR symbols except NaN.
4. Scale is significant: `1.50` (numeric(10,2)) displays `1.50` raw / `1,50` localized; never `1.5`.
5. ID-like columns (primary keys, columns named `*_id`, integer columns with sequence defaults) are **never grouped** even in localized mode (DBeaver has the same idea). [Observed: DBeaver option to skip ID columns]
6. `money` (PostgreSQL): server output depends on `lc_monetary`; raw mode shows server text; editing casts via `::numeric` in generated SQL; localized mode re-formats the numeric value with the regional currency only if the user configures it (currency is not inferable). [Inferred]

#### 3.8.3 Editing and parsing

Specified in ALG-I18N-07. Core decisions:

- Editor pre-fills the **raw** value (even in localized mode).
- Parse order: strict machine grammar → locale grammar (format locale) → error. Ambiguous inputs (machine-valid and locale-valid with different values, e.g. `1.234` in `tr`) are accepted as machine format **but** the live preview below the editor shows the interpreted value in the user's format ("= 1,234" in tr) and a one-click alternative ("did you mean 1234?") when the column type makes the alternative plausible (integer columns; `1.234` for an integer column is not machine-valid → locale grouping interpretation `1234` is the only candidate → accepted with preview).
- The value sent to the database is a canonical machine string bound as a parameter of the column's type (or a literal formatted in canonical form) — never a localized string.

#### 3.8.4 Copy and paste

| Action | Default output | Alternatives |
|---|---|---|
| Copy cell(s) | Raw values, TSV (tab, `\n`), `NULL` per setting (empty or `NULL`) | "Copy as displayed", "Copy for spreadsheet (regional)" (locale decimal `,`, no grouping, dates in regional short format so Excel-tr parses them), "Copy as SQL INSERT/VALUES", "Copy as JSON/CSV/Markdown" |
| Clipboard formats | `text/plain` (TSV raw) + `text/html` table + private `application/x-hacivat-cells` (raw values + types, for lossless internal paste) | Spreadsheet apps prefer HTML when present and parse its cell text with the OS regional settings [Inferred]; the HTML flavor therefore follows the setting "Spreadsheet paste target: regional / raw" (default regional). Verify with Excel/LibreOffice (§7 item 6). |
| Paste into grid | Each cell parsed with ALG-I18N-07 (machine first, then locale); summary dialog if any cell is ambiguous or invalid | — |

#### 3.8.5 CSV/TSV export

| Option | Default | Turkish/European preset | Notes |
|---|---|---|---|
| Delimiter | `,` | `;` | CLDR list symbol for tr is `;` [Observed]; Excel uses the OS list separator [Inferred]. |
| Decimal | `.` | `,` | Only in the preset; raw export is always `.`. |
| Quote | `"` with RFC 4180 doubling | same | Quote fields containing delimiter, quote, CR/LF, leading/trailing spaces. |
| Encoding | UTF-8 without BOM | UTF-8 **with BOM** | Excel on Windows needs the BOM to detect UTF-8, otherwise `ş` → `ÅŸ`. [Inferred: well-known Excel behavior] |
| Line ending | `\n` | `\r\n` | — |
| Dates | ISO-8601 | regional short date + time | — |
| NULL | empty | empty | configurable `\N` for COPY compatibility. |
| Header | yes | yes | Column names verbatim (never localized). |

Presets: "Standard (RFC 4180)", "Excel — regional (detected: Turkish)", "PostgreSQL COPY text", "TSV". The regional preset is derived from the format locale's CLDR data (decimal and list symbols) — no per-language code.

### 3.9 Dates and times

#### 3.9.1 Turkish conventions (CLDR 49)

| Item | Turkish | Evidence |
|---|---|---|
| Short date | `d.MM.y` → `11.11.2026`, `1.02.2026` | [Observed: CLDR 49 `tr/ca-gregorian.json` dateFormats.short] |
| Medium date | `d MMM y` → `11 Kas 2026` | same, medium |
| Long date | `d MMMM y` → `11 Kasım 2026` | same, long |
| Full date | `d MMMM y EEEE` → `11 Kasım 2026 Çarşamba` | same, full |
| Time | `HH:mm` / `HH:mm:ss` (24-hour) | timeFormats |
| Date-time glue (medium) | `{1} {0}` (date then time) | dateTimeFormats.medium |
| Months | Ocak, Şubat, Mart, Nisan, Mayıs, Haziran, Temmuz, Ağustos, Eylül, Ekim, Kasım, Aralık | months.format.wide |
| Weekdays | Pazar, Pazartesi, Salı, Çarşamba, Perşembe, Cuma, Cumartesi | days.format.wide |
| AM/PM | ÖÖ / ÖS (rarely used; 24h default) | dayPeriods |
| First day of week | Monday | [Observed: CLDR 49 `weekData.json` firstDay TR = mon] |
| List patterns | `{0}, {1}` … `{0} ve {1}`; "or": `{0} veya {1}` | [Observed: `tr/listPatterns.json`] |

Note `d.MM.y` (no leading zero on day, leading zero on month) — a parser must accept `1.2.2026`, `01.02.2026`, `1.02.2026`.

#### 3.9.2 Rules

1. Grid raw mode: ISO-8601 with the server's precision (`timestamp(6)` → microseconds); `timestamptz` displayed in the **session time zone** that Hacivat sets on the connection (default: OS time zone; configurable per connection; shown in the column header tooltip). PostgreSQL `infinity`/`-infinity` and BC dates (`0044-03-15 BC`) shown as server spells them.
2. Localized mode: regional short date + medium time; tooltip shows ISO + zone.
3. Date input: accept ISO first; then regional short pattern (tr: `d.M.y`, `d.M.y H:mm[:ss]`); **never** accept ambiguous US `M/d/y` unless the format locale is `en-US`; two-digit years rejected in editing (accepted only with explicit pivot setting).
4. Relative times in UI chrome ("3 dakika önce") via ICU4X relative-time (experimental) or Fluent messages with plural; never in grid data.
5. Perde audit log: absolute timestamps in the viewer's time zone with UTC offset, ISO tooltip; e-mails: recipient time zone + UTC.
6. DB server `DateStyle`/`lc_time` (PostgreSQL) influence `to_char` and text output of dates (`DateStyle=German` prints `11.11.2026`); Hacivat sets `DateStyle=ISO, MDY` and `IntervalStyle=postgres` on its sessions so parsing of server text is deterministic, and formats for humans client-side. [Inferred: PostgreSQL session settings; verify interplay with user expectations — document in connection settings]

---
### 3.10 Turkish grammar and style issues in UI strings

#### 3.10.1 Suffixes after placeholders (vowel harmony, consonant assimilation)

Turkish attaches case and possessive suffixes to nouns; the suffix form depends on the last vowel (front/back, rounded/unrounded), the final sound (voiced/voiceless), and for abbreviations/foreign names on *pronunciation*, separated by an apostrophe for proper nouns:

| Name | Dative ("to …") | Locative ("in …") | Why |
|---|---|---|---|
| PostgreSQL | PostgreSQL'e | PostgreSQL'de | read "…es-kü-el" → front vowel e |
| MySQL | MySQL'e | MySQL'de | same |
| SQLite | SQLite'a | SQLite'ta | read "…lâyt" → back vowel a, voiceless t → `ta` |
| MongoDB | MongoDB'ye | MongoDB'de | read "…di-bi" → front i → `ye` after vowel |
| Redis | Redis'e | Redis'te | final s voiceless → `te` |
| ClickHouse | ClickHouse'a | ClickHouse'ta | read "…haus" → back a; voiceless s → `ta` |
| prod-db (user name) | ? | ? | unknowable from spelling |

[Inferred: Turkish grammar; examples verified by native-speaker knowledge — reviewer must confirm]

Consequence: **a placeholder must never be followed by an inflectional suffix** in a Turkish message. Restructure:

| English source | Bad Turkish (suffix on variable) | Good Turkish |
|---|---|---|
| Connected to { $name }. | `{ $name }'e bağlanıldı.` | `Bağlantı kuruldu: { $name }` or `{ $name } bağlantısı kuruldu.` (suffix on fixed noun "bağlantı") |
| Could not drop table { $table }. | `{ $table }'ı silemedi.` | `{ $table } tablosu silinemedi.` |
| Copied { $count } rows from { $source } | `{ $source }'tan …` | `Kaynak: { $source } — { $count } satır kopyalandı.` |

Known nouns (engine names, our brand names) can be terms with case variants (doc 01 §3.2.1) — but for engine names the simpler path is the fixed-noun pattern ("PostgreSQL sunucusuna" — suffix on "sunucu"). The CI validator warns on `}'` patterns in `tr` (ALG-I18N-04 step 5.12).

#### 3.10.2 Plurals

Turkish does not pluralize nouns after numerals ("3 tablo", not "3 tablolar"), so most Turkish messages need no selector: `{ $count } satır getirildi`. CLDR still defines `one`/`other` for Turkish (`n = 1` vs everything else) [Observed: CLDR 49], which translators may use for wording such as "Tek satır" vs "{ $count } satır". Zero is usually expressed with an exact key `[0] Satır yok`.

#### 3.10.3 Ordinals

Written with a period (`1.`, `2.`); spelled "birinci", "ikinci". CLDR has only the `other` ordinal category for Turkish [Observed: `ordinals.json`] — never port English `st/nd/rd/th` logic.

#### 3.10.4 Capitalization and casing

- Sentence case is standard in Turkish UI; English "Title Case Menus" should not be imitated.
- `İ` in capitals: "İptal", "İçe aktar", "Bilgi" → "BİLGİ". Code must not uppercase translated strings (TR-6); if a design calls for all-caps labels, the translator supplies them or CSS `text-transform` is applied **only** inside an element with the correct `lang` (`lang="tr"` produces `BİLGİ`; `lang="en"` would produce `BILGI`, a spelling error).

#### 3.10.5 Register and terminology

- Use the formal/impersonal register: imperatives for buttons ("Kaydet", "İptal", "Bağlan"), impersonal passive for statuses ("Bağlantı kuruldu", "Sorgu çalıştırıldı"), "siz" (formal you) in sentences addressing the user.
- Keep SQL keywords and product names untranslated: `COMMIT`, `ROLLBACK`, `EXPLAIN`, `VACUUM`, `NULL`, `PostgreSQL`.
- Avoid mixing anglicisms when an established Turkish database term exists; where the community uses the English term (e.g. "index" ↔ "indeks"), prefer the community term.

#### 3.10.6 Starting glossary (en → tr) for Weblate

| English | Turkish (proposed) | Notes |
|---|---|---|
| connection | bağlantı | |
| database | veritabanı | one word |
| server | sunucu | |
| schema | şema | |
| table | tablo | |
| view | görünüm | |
| materialized view | gerçekleştirilmiş görünüm | [Inferred: verify common usage] |
| column | sütun | "kolon" also common; pick one |
| row | satır | |
| record | kayıt | |
| index | indeks | "dizin" means directory in UI contexts; avoid ambiguity |
| primary key | birincil anahtar | |
| foreign key | yabancı anahtar | |
| unique constraint | benzersizlik kısıtı | |
| constraint | kısıt | |
| trigger | tetikleyici | |
| stored procedure | saklı yordam | |
| function | fonksiyon | "işlev" also used |
| sequence | sıra (dizisi) | [Inferred: verify]; often left as "sequence" |
| transaction | işlem | collides with "operation"; use "işlem (transaction)" in tooltips |
| query | sorgu | |
| query plan / explain | sorgu planı | `EXPLAIN` stays as keyword |
| result set | sonuç kümesi | |
| filter | filtre | |
| sort | sırala / sıralama | |
| export / import | dışa aktar / içe aktar | |
| permission | izin / yetki | role-based access: "yetki" |
| role | rol | |
| audit log | denetim kaydı | |
| agent (Karagöz) | ajan | brand term stays "Karagöz" |
| approve / approval | onayla / onay | |
| read-only | salt okunur | |
| NULL | NULL | never translated |
| true / false (UI labels) | doğru / yanlış | only in UI filters, never in grid data |

### 3.11 Keyboard shortcuts on Turkish Q and F layouts

#### 3.11.1 Layout facts relevant to shortcuts

[Inferred: from the standard Windows Turkish Q (KBDTUQ) and Turkish F (KBDTUF) layouts and macOS Turkish layouts; must be verified against the official layout tables before implementation — search could not surface an authoritative key-by-key chart]

| Character | US physical key | Turkish Q | Turkish F |
|---|---|---|---|
| `ı` | — | key at US `I` position (`KeyI`) | own key |
| `i` | `KeyI` | key at US `'` position (`Quote`) | different position |
| `I` | Shift+`KeyI` | Shift+`KeyI` (Shift+ı) | — |
| `İ` | — | Shift+`Quote` (Shift+i) | — |
| `/` | `Slash` | Shift+7 | — |
| `[` `]` | `BracketLeft/Right` | AltGr+8 / AltGr+9 | AltGr combos |
| `{` `}` | Shift+brackets | AltGr+7 / AltGr+0 | AltGr combos |
| `\` | `Backslash` | AltGr+`*`-key (right of 0) | AltGr |
| `|` | Shift+`Backslash` | AltGr+`-`-key | AltGr |
| `@` | Shift+2 | **AltGr+Q** | AltGr |
| `;` | `Semicolon` | Shift+`,` | — |
| `"` | Shift+`Quote` | key left of 1 | — |

Consequences:

1. **`Ctrl+/` (toggle comment)** is `Ctrl+Shift+7` physically on Turkish Q; many editors bind by physical key and break. VS Code documents that bindings reflect the layout at launch and are scan-code based on Linux. [Observed: github.com/microsoft/vscode/wiki/Keybinding-Issues]
2. **The key labeled `I` produces `ı`**: a shortcut defined as `Ctrl+I` matched by *character* receives `ı` (or `I` with Shift) — matching must normalize `ı→i`, `İ→i`, `I→i` for shortcut purposes.
3. **AltGr on Windows is Ctrl+Alt**: default bindings on `Ctrl+Alt+Q` (=`@`), `Ctrl+Alt+7/8/9/0` (`{ [ ] }`), `Ctrl+Alt+-` (`|`), `Ctrl+Alt+*` (`\`), `Ctrl+Alt+E` (`€`) would make it impossible to type SQL brackets, JSON braces, e-mail addresses and pipes in the editor on Turkish Windows. Our defaults must never use these combinations. [Inferred]
4. **F layout** moves letters entirely (`F` layout home row starts `u i e a ü t k m l y ş x`) [Inferred]: mnemonic shortcuts (`Ctrl+F` find) should follow the *character* the user sees, not the physical position.

#### 3.11.2 Shortcut design rules

1. Bindings are defined by **logical character** (what `event.key`/the OS reports after layout mapping) with modifiers, plus a fallback match on **physical code** (`event.code`/scan code) only for non-character keys (F-keys, arrows, Enter, Tab) and for chords where the character requires AltGr.
2. Normalize the produced character for matching: NFC, then map `ı İ I → i`, then ASCII-lowercase. With Shift held, compare against the unshifted character derived from the layout when available (web: `event.code` + `navigator.keyboard.getLayoutMap()` where supported; native: OS layout APIs). [Inferred]
3. Provide alternatives for symbol shortcuts: `Ctrl+/` **and** `Ctrl+Shift+7`-equivalent detection (character `/` produced with Ctrl), plus a non-symbol default (`Ctrl+K Ctrl+C`-style chord or `Ctrl+Shift+C`).
4. Reserved-combination check: a CI test enumerates default bindings and fails if any uses `Ctrl+Alt+<key>` where `<key>` produces a printable character with AltGr on Turkish Q/F, German, French, Polish layouts (table in the test).
5. Shortcut *display* uses the user's layout ("Ctrl+Shift+7" vs "Ctrl+/" — show the character the user must type plus a tooltip with the physical key). macOS: `⌘` glyphs are universal; Windows/Linux modifier names are localized in Turkish UI? Keyboards in Turkey print "Ctrl", "Alt", "Shift" (some "Üst Karakter") — keep English modifier names; it is what is printed on keys. [Inferred]
6. Access keys/mnemonics (`&File` → Alt+F) come from the catalog (`.accesskey` attributes); CI checks uniqueness per menu per locale (Turkish letters `Ş`, `İ` allowed).

### 3.12 Fonts and text rendering

1. **Coverage**: Turkish needs `ç Ç ö Ö ü Ü` (Latin-1) and `ğ Ğ ı İ ş Ş` (Latin Extended-A). Web-font subsets named "latin" (Google Fonts style) exclude Latin Extended-A; Turkish letters then render in a fallback font (visibly different weight/metrics) — always ship "latin" + "latin-ext" subsets or full fonts. [Inferred: Google Fonts subsetting convention]
2. **Monospace editor font**: choose one with full Latin Extended-A, distinct `ı`/`i`/`l`/`1`/`I`/`|` glyphs, and box-drawing for grid borders (e.g. JetBrains Mono, Fira Code, Cascadia Code, Source Code Pro — licenses OFL [Inferred]). Data grids display Turkish data constantly; `I` vs `l` vs `ı` confusion is a real readability issue.
3. **Ligatures**: `fi`/`ffi` ligatures merge the dot of `i` into the `f` hook; in Turkish `fi` vs `fı` are different syllables. Fonts with Turkish `locl` handling disable `fi` when text is tagged Turkish; therefore set `lang="tr"` on Turkish UI text and `lang="en"` (plus `font-variant-ligatures: none` in code/grid) on code and data. Programming ligatures (`!=`, `->`) in the SQL editor are a user preference, default off for grid. [Inferred: OpenType `locl` feature behavior]
4. **Dotted capital in small caps / uppercase transforms**: covered by TR-6; CSS `text-transform` follows `lang`.
5. **Native toolkits**: font fallback must include a font with Latin Extended-A by default; verify `İ`'s dot isn't clipped by tight line-height in buttons (common rendering bug). [Inferred]
6. **Sorting/searching font-agnostic**: never rely on glyph appearance.

### 3.13 Database-client i18n: what must never be localized

| Artifact | Localize? | Notes |
|---|---|---|
| SQL keywords, functions, operators in editor, completion, formatter, generated DDL/DML | **Never** | Including `NULL`, `TRUE/FALSE`, `ASC/DESC`. Generated SQL comments (e.g. "-- generated by Hacivat") stay English. |
| Identifiers (schema/table/column/constraint/index names) | **Never** | Identifiers are *data*: display verbatim with `translate="no"` and `dir="auto"`; never apply `text-transform` or any casing change. |
| Data values, including booleans, enums, JSON keys, `t/f` from PostgreSQL | **Never** | Grid shows server values; localized *display* only for numbers/dates in opt-in mode. |
| `NULL` marker in grid | **No** — show `NULL` (styled) | A localized "<boş>" would be confused with empty string `''`; NULL is a SQL concept users search for. Configurable symbol (`NULL`, `∅`, `<null>`). |
| Error codes: SQLSTATE, MySQL errno, SQL Server error number, Mongo `codeName`, Redis `WRONGTYPE`/`ERR` prefixes | **Never** | Always shown next to localized explanation. |
| Server error/notice message text | Not by us (passthrough) | ALG-I18N-08 adds a localized explanation. |
| Connection string/URL keys, driver option names, SSL modes (`verify-full`) | **Never** | Labels around them are localized; the values are not. |
| Data type names (`integer`, `timestamptz`, `nvarchar(max)`) | **Never** | Column header tooltips show server type names. |
| Explain plan node names (`Seq Scan`, `Hash Join`) | **Never** as primary; optional localized tooltip | Plans are copied into issues/forums. |
| Units in metrics (`ms`, `MB`, `rows`) | Yes (via ICU/Fluent) in UI chrome | Not inside copied plan text. |
| Default file names of exports | No | ASCII defaults built from object names + ISO date (`orders_2026-11-11.csv`) avoid cross-OS encoding issues; the user can rename in the save dialog. |
| Log files, audit events, CLI `--json`, MCP | **Never** | doc 01 §3.5.4. |
| Settings file keys (`settings.toml`) | **Never** | Values like language tags are machine identifiers. |
| Keyboard shortcut identifiers (`editor.toggle_comment`) | **Never** | Display names localized. |

### 3.14 Database server messages (summary; algorithm ALG-I18N-08)

| Engine | Message language control | Who can change it | Stable identifiers |
|---|---|---|---|
| PostgreSQL | `lc_messages` (server/role/database default; `SET` per session) | **Only superusers and users with the appropriate SET privilege** [Observed: PostgreSQL 18 docs, runtime-config-client] | `C` SQLSTATE (always present, not localizable), `V` non-localized severity (PG ≥ 9.6), plus `s/t/c/d/n` object fields [Observed: protocol-error-fields] |
| MySQL | `lc_messages` global **and session**; any client can `SET lc_messages = 'en_US'`; default English; languages include Czech…Swedish (no Turkish error-message file listed) [Observed: MySQL 8.4 manual] | Any session | errno + SQLSTATE |
| MariaDB | Same mechanism (`lc_messages`) [Inferred] | Session | errno + SQLSTATE |
| SQL Server | Messages stored per language in `sys.messages` (`language_id`); session language via `SET LANGUAGE`/login default language [Observed: Microsoft docs via search] — **side effects**: `SET LANGUAGE` also changes `DATEFORMAT`/`DATEFIRST` and month names for string↔date conversion [Inferred: T-SQL docs] | Any session | error number, state, class |
| SQLite | English only | — | primary/extended result codes |
| MongoDB, Redis, ClickHouse | English | — | `code`/`codeName`; Redis error prefix; ClickHouse numeric code + name |

Implications:

1. A Turkish PostgreSQL server (`lc_messages = 'tr_TR.UTF-8'`) sends Turkish messages, and a non-superuser client cannot switch them to English per session. Never parse message text; rely on `C`, `V` and object fields.
2. Do not put `lc_messages` in the startup packet's `options` for non-superusers: setting a superuser-only parameter at startup fails the connection. [Inferred: PostgreSQL permission check applies at startup; verify] If the user enables "Prefer English server messages", run `SET lc_messages = 'C'` *after* connecting, inside error handling, and only if `has_parameter_privilege(current_user, 'lc_messages', 'SET')` (PG 15+) or superuser.
3. MySQL: offer "Prefer English server messages" → `SET SESSION lc_messages='en_US'`; default *off* (respect server/admin choice) but **on for MCP-originated sessions** (LLMs reason better over English messages; see `mcp/02-…`). [Inferred: design]
4. SQL Server: never change `LANGUAGE` implicitly (date-literal semantics would change under the user's queries); show `@@LANGUAGE` in connection info.
5. Authentication/startup errors arrive before any session setting — always handled by code (PG `28P01`, `28000`, `3D000`), never by text.
6. Client-side (driver) errors from Rust drivers are English text — map them to our error codes (`HCV-CONN-*`) and localize ourselves.

### 3.15 Grid rendering hazards that interact with i18n

1. **Bidi controls in identifiers and data** (U+202A–U+202E, U+2066–U+2069, U+200E/F, U+061C): render them visibly (e.g. `⟨RLO⟩` chip) in the grid, object tree and editor, and warn on SQL text containing them outside string literals/comments (Trojan Source class, CVE-2021-42574). Copy-raw keeps them; copy-as-displayed does not. [Inferred: CVE-2021-42574 "Trojan Source" research; design]
2. **Invisible and confusable characters** in identifiers: zero-width (U+200B–U+200D, U+FEFF), non-breaking spaces, Turkish `ı` vs Latin `i`, Cyrillic `і` (U+0456) vs Latin `i`. Object tree shows a warning badge for identifiers that are not NFC or contain invisible/mixed-script characters (UTS #39-style check). [Inferred]
3. **Line breaks and tabs in cells**: shown as `↵`/`→` glyphs in single-line cells; never let a data value change row layout.
4. **Per-cell direction**: `dir="auto"` (first strong character) per cell for text columns; numbers always LTR; mixed RTL text plus numbers handled by UBA.
5. **Truncation** by grapheme clusters (ICU4X segmenter), never inside a combining sequence or emoji ZWJ sequence; ellipsis character `…` (locale-independent).
6. **Binary/bytea**: hex (`\x…`) raw; never decode as text in the user's locale encoding.
7. **Encoding labels**: server `client_encoding` forced to UTF-8 (PostgreSQL `client_encoding=UTF8`, MySQL `SET NAMES utf8mb4`); legacy single-byte Turkish databases (`LATIN5`/ISO-8859-9, Windows-1254) converted by the server; invalid sequences (possible with SQL_ASCII databases) shown as `\xNN` escapes with a warning, never silently replaced in editable cells.

### 3.16 RTL readiness (Arabic, Hebrew, Persian, Urdu)

| Area | Rule |
|---|---|
| Layout | Mirror the shell (sidebar on the right, tree expanders pointing left, progress fills right-to-left) driven by `locale.toml` `direction`; CSS logical properties (`margin-inline-start`, `inset-inline-end`) everywhere in the web UI; native toolkit must support mirrored layouts. |
| SQL editor | Always `dir="ltr"` with per-line base direction LTR; RTL string literals and comments render via UBA within the LTR line (CodeMirror 6 has bidi-aware cursor motion [Inferred: CM6 docs]). Never mirror the editor. |
| Grid | Column order does **not** follow UI direction: keep column order as in the result set (left-to-right = ordinal order) but anchor the grid to the inline-start edge; cell text `dir="auto"`; numeric columns LTR, aligned to inline-end. [Inferred: design choice; DataGrip/DBeaver behavior in RTL Unknown] |
| Messages | Fluent FSI/PDI isolation around placeables (identifiers inside Arabic sentences) [Observed: fluent-rs default]; whole fallback (English) messages shown in RTL UI get `dir="auto"`. |
| Digits | UI chrome may use the locale's default numbering system (CLDR: e.g. Arabic-Indic for some `ar` regions [Inferred]); grid data and editor never convert digits. Parsing (ALG-I18N-07) accepts any Unicode `Nd` digits and normalizes to ASCII. |
| Icons | Directional icons (back/forward, undo/redo arrows, "send") mirrored; non-directional (play, clock, checkmarks, database) not mirrored. |
| Keyboard | Left/Right arrows in tree: in RTL, Right collapses and Left expands (follow platform conventions). |
| Text input | Arabic-script shaping, ligatures, cursor movement in mixed text: requires a text engine with full UBA + shaping (browser engines yes; native Rust toolkits vary → stack criterion). |
| Testing | `ar-XB` pseudo-locale (ALG-I18N-03 bidi mode) and real Arabic sample data in the grid. |

### 3.17 Pseudo-localization for a database client

In addition to doc 01 ALG-I18N-03 modes, the DB client needs **data-side stress fixtures** (not pseudo-localization of UI, but test data):

1. A fixture database (SQLite + PostgreSQL + SQL Server containers in CI) with identifiers `items`, `ITEMS` (quoted), `İtems`, `ıtems`, `müşteri`, `MÜŞTERİ` (quoted), `"Users"`, `users`, an identifier with U+202E, one with NBSP, one in Arabic (`"العملاء"`), one 63-byte multibyte name (PostgreSQL truncation boundary).
2. Columns with numeric values at edge scales (`numeric(38,18)`, `1e-30`, `-0`, `NaN`, `Infinity`), money with `lc_monetary=tr_TR.UTF-8`, dates (`infinity`, BC dates), Turkish text values for sorting/search tests.
3. Databases created with Turkish collations: PostgreSQL `LC_COLLATE/LC_CTYPE=tr_TR.UTF-8` and ICU `tr-x-icu`; SQL Server `Turkish_CI_AS`; MySQL `utf8mb4_tr_0900_ai_ci`.
4. The full UI test suite runs with UI language `tr`, format locale `tr-TR`, process `LANG=tr_TR.UTF-8`, and with `en-XA`/`tr-XT`/`ar-XB` pseudo-locales.

### 3.18 String length expansion

Guideline (IBM globalization guidance, widely reproduced) for UI text expansion from English [Inferred: commonly cited figures; treat as design budget, not measurement]:

| English length (chars) | Allow expansion |
|---|---|
| 1–10 | +100 % to +200 % |
| 11–20 | +80 % to +100 % |
| 21–30 | +60 % to +80 % |
| 31–50 | +40 % to +60 % |
| 51–70 | +30 % to +40 % |
| > 70 | +30 % |

Turkish specifics: typical +20–35 % for sentences; long single words from agglutination ("Bağlantılarınızdan", "yapılandırılamadı") make *word-level* wrapping hard — buttons need flexible width and hyphenation is unreliable [Inferred]. German is usually the worst-case Latin language for width and is a good second pseudo target. Rules: no fixed-width buttons; tooltips for truncated labels; status bar segments elastic; menus auto-size; tab titles truncate with ellipsis and full-title tooltip; `# MaxLength:` comments for genuinely constrained places (tray menu, Windows jump lists). Pseudo-locale `en-XA` uses the table above (doc 01 ALG-I18N-03).

---

## 4. Algorithm specifications

### ALG-I18N-05: Locale-invariant identifier and keyword case folding (per dialect)

- **Problem**: Decide (a) whether a token is a given SQL keyword, (b) what name the server will store/look up for an unquoted identifier, (c) whether an identifier must be quoted when Hacivat generates SQL, and (d) what key to use for case-insensitive identifier lookups in caches — all independent of the process locale and the UI language, but faithful to each server's semantics (including server collations that are themselves Turkish).
- **Inputs / outputs**:
  - Inputs: `dialect ∈ {Postgres, MySql, MariaDb, SqlServer, Sqlite, Oracle, ClickHouse}`; `ServerCtx { encoding_single_byte: bool (PG), lower_case_table_names: 0|1|2 (MySQL), db_collation: Option<String> (SQL Server), identifier_max_bytes }`; `token/name: &str`; `quoted: bool`; `object_kind` (table/column/routine/…).
  - Outputs: `is_keyword(token, kw) -> bool`; `fold_unquoted(name) -> Folded::Exact(String) | Folded::ServerDependent`; `needs_quoting(name) -> bool`; `match_key(name, quoted, kind) -> IdentKey`; `quote(name) -> String`.
- **Algorithm**:
  1. **Keyword test** (all dialects): `token.len() == kw.len() && token.bytes().zip(kw.bytes()).all(|(a,b)| a.to_ascii_lowercase() == b)` where `kw` is stored lower-case ASCII. Non-ASCII tokens are never keywords. (Equivalent to `str::eq_ignore_ascii_case`.)
  2. **Keyword re-casing** (formatter): `to_ascii_uppercase()`/`to_ascii_lowercase()` on keyword tokens only.
  3. **fold_unquoted**:
     - Postgres: for each byte `b`: if `b'A' ≤ b ≤ b'Z'` → `b + 32`; else if `b ≥ 0x80` and `encoding_single_byte` → return `ServerDependent` (server applies `tolower()` under its `LC_CTYPE`, e.g. Turkish `LATIN5` maps `İ`→`i`, `I`→? (ASCII rule wins: `I`→`i`)); else keep. Then truncate to `identifier_max_bytes` (63) at a character boundary (server uses multibyte-aware clipping) and record a `Truncated` notice.
     - Oracle: ASCII `a–z → A–Z`; non-ASCII: `ServerDependent` [Unknown].
     - SQLite, MySQL, MariaDB, SQL Server, ClickHouse: no folding on creation → `Exact(name)` (names are stored as written).
  4. **needs_quoting(name)** (when Hacivat generates SQL):
     - Postgres: quote if `name` is empty, or `fold_unquoted(name) != Exact(name)`, or `ServerDependent`, or first char not in `[a-z_]`∪non-ASCII-letter, or any char not in `[a-z0-9_$]`∪non-ASCII-letter, or `name` (ASCII-lowercased) is a reserved/type-function-name keyword per the server version's keyword list, or it contains bidi/invisible characters (always quote + warn).
     - SQL Server: quote with `[]` if not a regular identifier (`[A-Za-z_@#][A-Za-z0-9_@#$]*` + Unicode letters per docs) or reserved keyword. Quoting does not change case semantics in SQL Server, so with Turkish/Azeri or case-sensitive collations the rule is to always emit the **exact catalog case**.
     - MySQL/MariaDB: backtick-quote if not `[A-Za-z0-9_$]+`-ish, all-digits, or reserved.
     - SQLite: quote if not `[A-Za-z_][A-Za-z0-9_]*` or keyword.
     - Oracle: quote if `fold_unquoted(name) != name` or not a valid nonquoted identifier.
     - ClickHouse: quote if not `[A-Za-z_][A-Za-z0-9_]*`.
  5. **quote(name)**: double the quote character inside (`"` → `""`, `]` → `]]`, `` ` `` → ``` `` ```); never transform case.
  6. **match_key(name, quoted, kind)** (cache keys for case-insensitive lookups; exact server name always kept alongside):
     - Postgres: quoted → `Exact(name)`; unquoted user input → `fold_unquoted(name)` (if `ServerDependent` → resolve via server: `SELECT $1::regclass` or `quote_ident` round trip; cache the answer). Catalog names from the server are already folded → key = name as-is.
     - SQLite: `ascii_lowercase(name)` (SQLite compares ASCII case-insensitively, non-ASCII exactly).
     - MySQL/MariaDB: tables/databases: `lower_case_table_names == 0` → exact; `1|2` → Unicode **default** simple lowercase (never Turkish), because MySQL lower-cases with the system character set's rules [Inferred]; columns/routines/indexes: Unicode default simple case fold (case-insensitive).
     - SQL Server: from `db_collation`: `*_CS_*` or `*_BIN*` → exact; `Turkish_*_CI_*` or `Azeri_*_CI_*` → **Turkic** fold (`fold_turkic`) — the one place where locale-sensitive folding is correct, because it mirrors the server; other `*_CI_*` → default Unicode case fold. Accent-insensitive collations (`_AI`) → additionally strip diacritics (NFD + remove Mn).
     - Oracle: quoted → exact; unquoted → ASCII upper.
     - ClickHouse: exact.
  7. Return.
- **Data structures**: `IdentKey(String)` newtype (not constructible from arbitrary strings outside this module); per-dialect static reserved-keyword tables (lower-case ASCII, versioned); `ServerCtx` captured at connect (PG: `server_encoding`, `pg_encoding_max_length`; MySQL: `@@lower_case_table_names`; SQL Server: `DATABASEPROPERTYEX(DB_NAME(),'Collation')` and `SERVERPROPERTY('Collation')` per database switch).
- **Complexity / performance notes**: O(n) per identifier; keyword lookup via perfect hash or binary search on ASCII-lowered bytes (no allocation for ASCII). Cache `match_key` results for catalog names (computed once per metadata load).
- **Edge cases (exhaustive list)**:
  1. Process locale `tr_TR.UTF-8` → results identical to `C` (property test runs under both).
  2. `İ`, `ı` in unquoted PostgreSQL identifiers (UTF-8 DB) → preserved byte-for-byte.
  3. Same in `LATIN5` DB → `ServerDependent` → ask server.
  4. Identifier exactly 63 bytes ending in a 2-byte char → no truncation; 64 bytes → truncation drops the whole last char (not half).
  5. Identifiers that differ only by case (`"Users"` and `users`) → distinct `IdentKey`s in PostgreSQL; distinct in SQL Server CS; same key in SQL Server CI → server would also reject creating both (no conflict).
  6. Turkish SQL Server DB: `ITEMS` vs `items` → distinct keys (as server); `İTEMS` vs `items` → same key (as server).
  7. Keyword with Unicode look-alikes (`SELECT` with Cyrillic `Е`) → not a keyword (correct: server also rejects).
  8. Fullwidth `ＳＥＬＥＣＴ` → not a keyword.
  9. `limit` in any ASCII case → keyword (auto-limit detection) — DBeaver #40901 regression.
  10. Identifier `select` → needs quoting; `"select"` round trip preserved.
  11. Empty quoted identifier `""` → invalid in PostgreSQL (zero-length delimited identifier error) → reject before sending.
  12. Identifier with embedded quote `a"b` → `"a""b"`.
  13. Identifier with NUL → reject (PostgreSQL disallows NUL).
  14. Non-NFC identifier from the server (`s`+U+0327 instead of `ş`) → keys built on the exact string (no normalization) because servers compare bytes; UI shows a "not NFC" badge.
  15. MySQL `lower_case_table_names=2` (macOS default) → stored as given, compared lower-case → key = lowercase.
  16. MySQL column `Ünvan` vs `ünvan` → same key (case-insensitive columns).
  17. SQLite `ÄBC` vs `äbc` → different keys (ASCII-only).
  18. Oracle quoted lowercase `"items"` vs unquoted `items` (→ `ITEMS`) → different.
  19. Accent-insensitive SQL Server collation (`Turkish_CI_AI`) → `ş` ~ `s` in key.
  20. Collation unknown (permissions) → default to *exact* keys (safe: may cause extra metadata fetches, never wrong matches).
- **Test cases (input → expected)**:
  | # | Function / dialect / ctx | Input | Expected |
  |---|---|---|---|
  | 1 | is_keyword any | `LIMIT` vs `limit` | true |
  | 2 | is_keyword any (process LANG=tr_TR.UTF-8) | `lımıt` | false |
  | 3 | is_keyword any | `lİmİt` | false |
  | 4 | fold_unquoted PG UTF-8 | `ITEMS` | `items` |
  | 5 | fold_unquoted PG UTF-8 | `MÜŞTERİ` | `mÜŞterİ` |
  | 6 | fold_unquoted PG UTF-8 | `İd` | `İd` |
  | 7 | fold_unquoted PG LATIN5 | `İd` | ServerDependent |
  | 8 | needs_quoting PG | `items` / `Items` / `müşteri` / `user` / `select` / `2fa` | false / true / false / true / true / true |
  | 9 | quote PG | `a"b` | `"a""b"` |
  | 10 | match_key SQLite | `ÄBC` vs `äbc` | `Äbc` ≠ `äbc` |
  | 11 | match_key SQL Server `Turkish_CI_AS` | `ITEMS` vs `items` | `ıtems` ≠ `items` |
  | 12 | match_key SQL Server `Turkish_CI_AS` | `İTEMS` vs `items` | equal (`items`) |
  | 13 | match_key SQL Server `Latin1_General_CI_AS` | `ITEMS` vs `items` | equal |
  | 14 | match_key SQL Server `Latin1_General_CS_AS` | `ITEMS` vs `items` | different |
  | 15 | auto-limit detector (uses is_keyword) PG | `select 1 limit 10` with auto-limit 200 | unchanged query |
  | 16 | auto-limit detector | `select 1 LİMİT 10` | `LİMİT` is not a keyword, the statement does not parse → detector does **not** rewrite (policy: unparsable statements are never rewritten); the server reports the syntax error |
  | 17 | Rust `to_lowercase` (anti-test) | `İd` | `i̇d` (3 chars) — demonstrates why it is forbidden |
  | 18 | truncation PG | 64-byte name `"a"*62 + "ş"` | 62 bytes `a…a` + notice (ş dropped entirely) |
- **Prior art and evidence**: PostgreSQL `kwlookup.c` ASCII-only keyword folding and `scansup.c` identifier downcasing [Observed]; DBeaver #40901 (cache keys and auto-limit) [Observed]; pgJDBC BUG #3693 [Observed]; SQL Server Turkish collation reports [Observed: community threads]; ICU4X `fold_turkic` [Observed: `casemapper.rs:240,438`].
- **Used by**: Hacivat | Karagöz (query classification for permissions must use the same keyword test) | Perde (policy rules that match object names)

### ALG-I18N-06: Turkish-tolerant, locale-aware UI search key with highlight mapping

- **Problem**: Filter and highlight UI lists (object tree, connection list, command palette, settings search, Perde tables) case-insensitively in a way that works for Turkish users typing on Turkish or US keyboards, for identifiers that are mostly ASCII, and for Turkish/other text — without false negatives on `I/i/İ/ı`, and with accurate highlighting in the original string.
- **Inputs / outputs**: Input: `text: &str` (candidate), `query: &str`, `opts { accent_insensitive: bool = true, width_insensitive: bool = true, mode: Contains | Prefix | WordPrefix | Fuzzy }`. Output: `SearchKey { folded: String, map: Vec<Range<usize>> }` for text; `Option<Match { ranges_in_original: Vec<Range<usize>>, score }>`.
- **Algorithm**:
  1. If `width_insensitive`: apply NFKC to a *copy* used only for folding (keep a map from NFKC chars to source byte ranges — do NFKC per grapheme cluster to retain mapping); else NFC.
  2. Iterate characters `c` of the normalized text with their source ranges:
     1. If `c ∈ {U+0049 I, U+0069 i, U+0130 İ, U+0131 ı}`: emit `i`; if the next char is U+0307 COMBINING DOT ABOVE and `c ∈ {I, i}`, consume it too (map both to this output char).
     2. Else: `f = full_default_case_fold(c)` (ICU4X `CaseMapper::fold` — not Turkic; `ß→ss`, `ﬁ→fi`, `ς→σ`).
     3. If `accent_insensitive`: for each char of `f`: decompose NFD; drop chars with General_Category `Mn`; apply a small extra table for letters without decompositions: `ø→o, đ→d, ł→l, ħ→h, ŧ→t, æ→ae, œ→oe, ı→i` (already handled), `ß` already `ss`.
     4. Emit resulting chars, each mapped to the source range of `c`.
  3. Compute query key the same way (map not needed); trim; collapse internal whitespace runs to one space; if empty → match everything (mode-specific) with no highlight.
  4. Match:
     - Contains: substring search of `qkey` in `tkey.folded` (all occurrences; highlight first or all per UI).
     - Prefix: `tkey.folded.starts_with(qkey)`.
     - WordPrefix: word boundaries computed on the folded key by: start, after `_`, `.`, `-`, space, digit↔letter transitions, and lower→upper transitions computed on the **original** text (camelCase) mapped forward; match query as prefix of any word (also multi-word acronym match "ui" → `user_id`).
     - Fuzzy: subsequence scoring (consecutive bonus, word-start bonus, prefix bonus, gap penalty) on folded keys.
  5. Map matched folded ranges back: union of `map[i]` for matched indices; extend to grapheme-cluster boundaries of the original (ICU4X segmenter) to avoid splitting `İ` (if decomposed) or emoji.
  6. Return ranges in original byte offsets.
- **Data structures**: `SearchKey` cached per item (keyed by text + opts version); for 100k-item trees store keys in a contiguous arena; optional n-gram index for very large lists (> 500k).
- **Complexity / performance notes**: Key building O(n) per item once; per keystroke O(total key length) for Contains with `memchr::memmem` (two-way). 100k identifiers × 20 bytes ≈ 2 MB scan < 2 ms [Inferred estimate]. Fuzzy limited to top-N after a cheap prefilter (all query chars present).
- **Edge cases (exhaustive list)**:
  1. `ID` matches `id`, `user_id`, `İD`, `ıd`.
  2. `istanbul` matches `İSTANBUL`, `Istanbul`, `ıstanbul` (tolerant; accepted false-positive class).
  3. Query `ı` matches all of `i ı I İ` in text.
  4. Decomposed input `I`+U+0307 treated as `İ`.
  5. `ß` in text, query `ss` → match; highlight covers `ß` entirely; query `s` → matches first half of `ss` → highlight covers `ß` (whole grapheme).
  6. Ligature `ﬁ` (U+FB01) in text, query `fi` → match; highlight covers the ligature.
  7. Accent-insensitive off: `musteri` does not match `müşteri`; on: matches.
  8. Query contains accents (`müş`) with accent-insensitive on → query also stripped → matches `mus…`, `müş…` (symmetric).
  9. Combining marks without base char at start of string → kept/dropped consistently, map to their own source range.
  10. Emoji/ZWJ sequences → folded unchanged; highlights align to cluster boundaries.
  11. Fullwidth `ＩＤ` with width-insensitive → matches `id`.
  12. Greek final sigma `ς` vs `σ` → match.
  13. Bidi controls inside text → dropped from key (they are `Cf`, not letters): add rule "drop Default_Ignorable_Code_Point" in step 2 with empty mapping contribution; highlight ranges skip them.
  14. Leading/trailing whitespace in query ignored; internal multiple spaces collapsed.
  15. Very long text (10k chars, SQL in saved-query search) → still linear; cap highlight count at 100.
  16. Empty text → no match unless query empty.
  17. Query longer than text → no match quickly.
  18. Case-sensitive mode toggle (power users) → bypass folding entirely: exact substring on NFC.
  19. Regex mode (power users) → separate engine; Unicode case-insensitive flag uses default (non-Turkic) folding plus the explicit `[iıİI]` class expansion for `i`/`I` in patterns [Inferred: design].
  20. Highlight when matching text is inside an identifier displayed with quoting (`"Users"`) → map against displayed string, not the raw name (display layer passes the displayed text).
- **Test cases (input → expected)** (defaults: accent_insensitive=true, Contains):
  | # | Text | Query | Match? | Highlight (original substring) |
  |---|---|---|---|---|
  | 1 | `user_id` | `ID` | yes | `id` |
  | 2 | `İSTANBUL_SUBE` | `istanbul` | yes | `İSTANBUL` |
  | 3 | `ILIK` | `ılık` | yes | `ILIK` |
  | 4 | `müşteri` | `musteri` | yes | `müşteri` |
  | 5 | `müşteri` | `musteri` (accent_insensitive=false) | no | — |
  | 6 | `Straße` | `strasse` | yes | `Straße` |
  | 7 | `Straße` | `stras` | yes | `Straß` (whole `ß` included) |
  | 8 | `ﬁle_list` | `file` | yes | `ﬁle` |
  | 9 | `I`+U+0307+`zmir` (decomposed) | `izmir` | yes | entire `İzmir` (both code points) |
  | 10 | `orders` | `ıd` | no | — |
  | 11 | `ＵＳＥＲ_ＩＤ` | `user_id` | yes | whole |
  | 12 | `ad⁧min` (with U+2067) | `admin` | yes | `ad⁧min` with the control included in range |
  | 13 | `events_2026_10` | `e10` (Fuzzy) | yes (score > `events_2026_01`?) | `e`, `1`,`0` positions |
  | 14 | `` | `a` | no | — |
  | 15 | `abc` | `   ` | yes (empty query) | none |
- **Prior art and evidence**: ICU4X default and Turkic folding [Observed: `casemapper.rs:223-240,408-440`]; Unicode default case folding behavior verified [Observed: Python checks]; IntelliJ Turkish code-completion bug as motivation [Observed: JetBrains support thread]; VS Code-style fuzzy matching [Inferred].
- **Used by**: Hacivat | Perde (web: same algorithm implemented in TS with identical test vectors) | Karagöz (CLI `list --filter`)

### ALG-I18N-07: Grid number parsing and formatting with round-trip fidelity

- **Problem**: Display numeric cells raw or localized; accept user input (typing, pasting from Hacivat or from external apps) in either machine or regional format; never lose precision; detect ambiguity (Turkish `.` grouping vs machine `.` decimal) instead of guessing wrongly; produce canonical text for the database.
- **Inputs / outputs**:
  - Inputs: `text: &str`; `kind: Int{bits, signed} | Decimal{precision: Option<u32>, scale: Option<u32>} | Float{bits} | Money`; `fmt: FormatSymbols { decimal, group, minus, plus, exp, group_is_space }` from CLDR for `format_locale`; `mode: Raw | Localized`; `source: Typed | PasteInternal(raw) | PasteExternal`; `column_evidence: Option<ColumnEvidence>` (for batch paste).
  - Outputs: `ParseResult::Ok { canonical: String, notes: Vec<Note> } | Ambiguous { machine: String, regional: String } | Err(ParseError{ kind, hint })`; formatter `format(canonical, kind, mode, fmt, is_id_column) -> Display { text, truncated_indicator: bool }`.
- **Algorithm (parse)**:
  1. If `source == PasteInternal(raw)` → return `Ok(raw)` (Hacivat's private clipboard format carries raw values; skip parsing).
  2. Preprocess: trim Unicode whitespace (incl. U+00A0, U+202F, U+2009, U+3000); remove bidi/format chars (U+200E, U+200F, U+061C, U+2066–U+2069, U+202A–U+202E); map any `Nd` digit to ASCII by numeric value (note `digits_normalized`); map U+2212 MINUS SIGN, U+FE63, U+FF0D to `-`; fullwidth `＋`, `．`, `，` to ASCII.
  3. If empty → `Err(Empty)` (setting NULL is a separate explicit action; optional setting "empty input means NULL").
  4. Special values (ASCII case-insensitive): `nan`, `infinity`, `+infinity`, `-infinity`, `inf`, `-inf`, `∞`, `-∞`: allowed for `Float` (canonical `NaN`, `Infinity`, `-Infinity`) and for PostgreSQL `numeric` (NaN; ±Infinity on PG ≥ 14 [Inferred]); else `Err(NotFinite)`.
  5. Machine grammar M: `^[+-]?(\d+(\.\d*)?|\.\d+)([eE][+-]?\d+)?$`.
  6. Regional grammar R (built from `fmt`): `^[+-]?` + integer part either `\d+` or `\d{1,3}(G\d{3})+` where `G` = `fmt.group` (if `group_is_space`, accept any of U+0020, U+00A0, U+202F, U+2009 as G) + optional `D\d+` (D = `fmt.decimal`) + optional `E[+-]?\d+`. If `fmt.decimal == '.'` and `fmt.group == ','` (en), R's integer-without-grouping branch equals M's — fine.
  7. Candidates: `m = M.matches ? canonicalize_machine(text) : None`; `r = R.matches ? canonicalize_regional(text) : None` (remove G, replace D with `.`).
  8. Kind validity filter: for `Int`, a candidate with a non-zero fractional part or exponent producing a fraction is invalid (a zero fraction `5.0`/`5,0` → valid with note `fraction_dropped` → canonical `5`); out-of-range for `bits` → invalid with `Err(OutOfRange)` if no other candidate is valid.
  9. Decide:
     - none valid → `Err(Invalid{ hint: examples "1234.5" and regional "1.234,5" })`.
     - one valid → `Ok(that)` (+ note `regional_interpretation` if it is `r`).
     - both valid and numerically equal → `Ok(m)`.
     - both valid and different → if `column_evidence` decides (see batch step) → that; else `Ambiguous{ machine: m, regional: r }`.
  10. Decimal constraints: if `scale` known and the canonical fraction has more digits than `scale` → note `will_round(scale)` (server rounds on insert — PostgreSQL rounds numeric to the column scale) and show the rounded preview; if integer digits exceed `precision - scale` → `Err(Overflow)`.
  11. Float: verify parse to `f64`/`f32` is finite unless special; note `precision_loss` if the decimal string has > 17 (f64) / 9 (f32) significant digits; canonical text = user's digits (server parses).
  12. Canonicalize: strip leading `+`; `-0` → `0` for Int/Decimal, keep `-0` for Float; remove leading zeros of the integer part (keep one `0`); `.5` → `0.5`; `5.` → `5`; keep trailing zeros of the fraction for Decimal (scale significance: `1.50` stays `1.50`); for Decimal expand exponent into plain notation (`1.5E3` → `1500`, `1E-3` → `0.001`) with a cap of 1000 digits (else `Err(TooLong)`); Float keeps exponent form normalized to `e`.
- **Batch disambiguation (paste of many cells / CSV import)**: per column, collect evidence before parsing: if any value contains `fmt.decimal` (`,` in tr) followed by digits not in a 3-digit group pattern, or contains both `.` and `,` with `,` last → evidence "regional"; if any value contains `.` followed by ≠3 digits (e.g. `3.14`) → evidence "machine"; conflicting evidence → ask once per column ("In column `price`, is `.` a decimal point or a thousands separator?"); single evidence → apply to all ambiguous cells in the column; no evidence → one prompt per column with the default choice = machine.
- **Algorithm (format)**:
  1. Raw mode → return `canonical` as received from the server (server spelling for specials).
  2. Localized → parse `canonical` with `fixed_decimal::Decimal::try_from_str` (exact); apply ICU4X `DecimalFormatter` for `format_locale` with grouping **off** if `is_id_column`; never limit fraction digits unless the user configured a display maximum (then round half-even for display and set `truncated_indicator = true`); Float: format the shortest round-trip decimal string (server's text) through the same path; specials: `NaN` → `NaN`, `Infinity` → `∞` (CLDR), `-Infinity` → `-∞`.
  3. Editing a localized cell pre-fills the *raw* canonical value.
- **Round-trip property**: for every canonical value `x` and every locale `L`: `parse(format_localized(x, L), source=PasteExternal)` ∈ {`Ok(x')` with `x' == x` numerically and scale-preserving, `Ambiguous{…}` containing `x`}; and `parse(raw(x)) == Ok(x)`. Copy → paste inside Hacivat is lossless by construction (PasteInternal).
- **Data structures**: `FormatSymbols` derived from CLDR (ICU4X decimal symbols) per format locale; compiled grammars (hand-written scanners, no regex needed); `ColumnEvidence { saw_regional: bool, saw_machine: bool }`.
- **Complexity / performance notes**: O(n) per cell; batch paste of 100k cells is linear; formatting cached per visible cell (virtualized grid renders ~2k cells per frame at most).
- **Edge cases (exhaustive list)**:
  1. tr: `1.234` for Decimal → Ambiguous(1.234, 1234).
  2. tr: `1.234` for Int → only regional valid → `1234`.
  3. tr: `1.234,5` → regional only → `1234.5`.
  4. tr: `1,5` → regional only → `1.5`.
  5. tr: `1.23` (2 digits after dot) → machine only (R requires groups of 3) → `1.23`.
  6. tr: `1.2345` → machine only → `1.2345`.
  7. tr: `12.345.678` → regional only (multiple groups) → `12345678`.
  8. tr: `12.34.56` → neither → error.
  9. en: `1,234` → regional only → `1234`; `1,234.5` → `1234.5`; `1.234` → both equal → `1.234`.
  10. fr (group U+202F): `1 234,5` → `1234.5`.
  11. de-CH (group `’` U+2019) → handled by `fmt.group` from CLDR.
  12. Arabic-Indic digits `١٢٣` → `123` with note.
  13. `−5` (U+2212) → `-5`.
  14. `+5` → `5`; `-0` Int → `0`; `-0` Float → `-0`.
  15. `007` → `7`; `0.0` Decimal → `0.0` (scale kept).
  16. `.5` → `0.5`; `5.` → `5`.
  17. `1e3` Int → `1000` (exponents are **rejected** for Int to avoid surprises: `Err(ExponentNotAllowedForInt)`); Decimal → `1000`; Float → `1e3`.
  18. `1,5e3` tr Float → regional with exponent → `1.5e3`.
  19. Int64 overflow `9223372036854775808` → `Err(OutOfRange)`; unsigned (MySQL BIGINT UNSIGNED) up to `18446744073709551615` valid.
  20. Decimal(10,2) `123.456` → note will_round → preview `123.46`.
  21. Decimal(5,2) `1234.5` → `Err(Overflow)`.
  22. Float `0.1` → canonical `0.1` (server stores nearest binary).
  23. `NaN` for Int → error.
  24. `∞` for Float → `Infinity`.
  25. `%25` (tr percent) → not accepted in grid numeric editing → error with hint (percent is a display concept).
  26. Currency symbols `₺1.234,56` for Money → strip currency symbol and spaces when kind is Money only; canonical numeric; for PostgreSQL `money`, bind as numeric text and cast (`$1::numeric::money`) so server `lc_monetary` doesn't affect input parsing. [Inferred]
  27. Paste from Hacivat itself → PasteInternal raw → no parsing.
  28. Paste of a localized value copied from Hacivat's "Copy as displayed" (no private format because user used another app in between) → treated as external → may be Ambiguous → resolved by column evidence/prompt.
  29. Text with thousands separators in raw mode → still accepted (parse is mode-independent; mode affects only display and prompts).
  30. Locale with `decimal == group`? (does not occur in CLDR) → assert at startup.
- **Test cases (input → expected)**:
  | # | fmt | kind | Input | Expected |
  |---|---|---|---|---|
  | 1 | tr | Decimal | `1.234` | Ambiguous{`1.234`, `1234`} |
  | 2 | tr | Int32 | `1.234` | Ok `1234` (+regional note) |
  | 3 | tr | Decimal | `1.234,5` | Ok `1234.5` |
  | 4 | tr | Decimal | `1,5` | Ok `1.5` |
  | 5 | tr | Decimal | `3.14` | Ok `3.14` |
  | 6 | tr | Decimal | `12.34.56` | Err Invalid |
  | 7 | en | Decimal | `1,234.5` | Ok `1234.5` |
  | 8 | en | Decimal | `1.234` | Ok `1.234` |
  | 9 | fr | Decimal | `1 234,5` | Ok `1234.5` |
  | 10 | tr | Decimal(10,2), PostgreSQL | `0,125` | Ok `0.125` + will_round(2) with preview `0.13` (preview must use the **server's** rounding; PostgreSQL numeric rounds half away from zero [Inferred], not half-even) |
  | 11 | tr | Int64 | `9223372036854775808` | Err OutOfRange |
  | 12 | any | Float64 | `-0` | Ok `-0` |
  | 13 | any | Decimal | `-0` | Ok `0` |
  | 14 | ar / tr | Decimal | `١٢٣٫٥` (Arabic-Indic digits, Arabic decimal separator U+066B) | fmt=ar (arab numbering): Ok `123.5`; fmt=tr: Err (U+066B is not the tr decimal symbol) |
  | 15 | any | Float64 | `∞` | Ok `Infinity` |
  | 16 | tr | Decimal | `%25` | Err Invalid (hint) |
  | 17 | tr | format Localized Decimal | canonical `1234567.50` | `1.234.567,50` |
  | 18 | tr | format Localized Int, ID column | canonical `1234567` | `1234567` |
  | 19 | tr | format Raw | canonical `1234567.50` | `1234567.50` |
  | 20 | tr | round-trip | format_localized(`1234`)=`1.234` → parse Int | Ok `1234` |
  | 21 | tr | round-trip | format_localized(`1234`)=`1.234` → parse Decimal | Ambiguous containing `1234` |
  | 22 | tr | batch paste Decimal column [`1.234`, `5,5`] | evidence regional → [`1234`, `5.5`] |
  | 23 | tr | batch paste Decimal column [`1.234`, `3.14`] | evidence machine → [`1.234`, `3.14`] |
  | 24 | tr | batch paste [`1.234`, `2.000`] | no evidence → one prompt for the column |
- **Prior art and evidence**: CLDR 49 Turkish symbols/patterns [Observed]; ICU4X `fixed_decimal` exact decimal parsing (machine format only) and `DecimalFormatter` [Observed: `utils/fixed_decimal/src/signed_decimal.rs:59`; `components/decimal/src`]; DBeaver data-format options incl. disabling numeric formatting and skipping ID columns [Observed: DBeaver docs]; PostgreSQL numeric semantics (scale rounding on insert) [Inferred].
- **Used by**: Hacivat | Perde (web forms with numeric inputs reuse the TS port of the parser) 

### ALG-I18N-08: Database server message presentation (localized explanation, verbatim server text)

- **Problem**: Show database errors and notices helpfully in the user's UI language when servers may send messages in an arbitrary language (or English), without ever parsing localized message text, while keeping copyable, English, code-based details for support and for AI tools.
- **Inputs / outputs**:
  - Inputs: engine-specific error/notice structure; `ServerLangInfo` (PG: `lc_messages` if readable via `SHOW lc_messages`; MySQL `@@lc_messages`; SQL Server `@@LANGUAGE`); UI chain; the SQL text sent (for position mapping); `sink: Ui | Machine(MCP/log)`.
  - Outputs: `DbErrorView { title: String (localized), code_chip: String, server_message: String (verbatim, sanitized), detail, hint, context, position: Option<EditorRange>, severity: Error|Warning|Notice, lang_hint: Option<LanguageIdentifier>, copy_text: String (English template) }`.
- **Algorithm**:
  1. **Normalize** into `DbError`:
     - PostgreSQL: `severity = V if present else map(S)`; if `S` not in {ERROR, FATAL, PANIC, WARNING, NOTICE, DEBUG, INFO, LOG} → `server_localized = true` and severity derived from message type (ErrorResponse → ERROR unless `C` class `57P0x` admin shutdown → FATAL-like); `code = C` (SQLSTATE); fields `M, D, H, P, p, q, W, s, t, c, d, n` kept. [Observed: protocol-error-fields]
     - MySQL/MariaDB: `errno`, `sqlstate`, message.
     - SQL Server: number, state, class (severity), procedure, line, message; class ≤ 10 → informational.
     - SQLite: extended result code (e.g. `SQLITE_CONSTRAINT_UNIQUE` 2067) + message.
     - MongoDB: `code`, `codeName`, `errmsg`; Redis: first token of error (`WRONGTYPE`, `ERR`, `NOSCRIPT`…); ClickHouse: numeric code + exception name.
  2. **Sanitize server text for display**: replace C0/C1 controls except `\n`/`\t` with visible escapes; render bidi controls visibly (§3.15); strip ANSI escape sequences; cap at 16 KiB with "show more"; invalid UTF-8 → U+FFFD with raw bytes preserved in details.
  3. **Explanation lookup** (Fluent IDs, all in `hacivat-core.ftl`):
     1. Specific: `db-pg-sqlstate-23505`, `db-mysql-errno-1062`, `db-mssql-error-2627`, `db-sqlite-code-2067`, `db-mongo-code-11000`, `db-redis-wrongtype`, `db-clickhouse-code-60` …
     2. Class-level: `db-sqlstate-class-23` (integrity constraint violation), `-42` (syntax error or access rule violation), `-08` (connection exception), `-28` (invalid authorization), `-40` (transaction rollback, incl. `40001` serialization failure / `40P01` deadlock), `-53` (insufficient resources), `-57` (operator intervention), `-22` (data exception), `-0A` (feature not supported), `-25` (invalid transaction state), `-P0` (PL/pgSQL) …
     3. Generic: `db-error-generic` ("The database reported an error").
     Arguments come only from structured fields (`constraint = n`, `table = t`, `column = c`, `schema = s`, `datatype = d`) — never from message text. Missing fields → message variants without them (Fluent select on `$has_constraint`).
  4. **Language hint**: if `server_localized` and `ServerLangInfo` known → `lang_hint` = parsed language of `lc_messages` (`tr_TR.UTF-8` → `tr`) for the `lang` attribute (correct hyphenation/casing/screen-reader voice); else none.
  5. **Position mapping**: PostgreSQL `P` is a 1-based **character** index into the query string as sent [Observed: protocol docs "index into the original query string"]; convert to editor offsets: account for (a) the statement's offset within the user's script when Hacivat splits scripts, (b) any rewriting Hacivat did (auto-limit appended at the end does not shift earlier positions; prefixes do — keep an offset map from rewriting), (c) characters vs UTF-16 code units in a web editor. Ignore `p` (internal position) for the editor; show `q` and `W` in details.
  6. **Severity/notice routing**: errors → error panel + editor squiggle; PG `WARNING/NOTICE/INFO` and SQL Server class ≤ 10 → messages pane, verbatim, with code chip.
  7. **Copy text** (English, stable): `"[{engine} {code}] {server_message}\nDETAIL: …\nHINT: …\nCONTEXT: …\n(Hacivat {version}, server {server_version}, lc_messages={…})"`.
  8. **Machine sink (MCP, logs)**: emit `{ engine, code, severity_nonlocalized, server_message (verbatim), detail, hint, position, constraint, table, column, schema }`; no localized explanation; no Fluent output; no bidi isolation marks; logs omit `detail` by default (it can contain row data, e.g. `Key (email)=(alice@example.com) already exists`) unless debug logging is enabled — privacy.
  9. **Optional English preference** (per connection setting "Server message language: Server default | English if allowed"): PostgreSQL after-connect `SET lc_messages = 'C'` only if `has_parameter_privilege(current_user,'lc_messages','SET')` (PG ≥ 15) or superuser, else skip silently; MySQL/MariaDB `SET SESSION lc_messages='en_US'`; SQL Server: never automatic (side effects). MCP-originated sessions use "English if allowed" by default.
- **Data structures**: `DbError` struct (engine-tagged); static tables mapping specific codes to message IDs are not needed — the ID is derived by naming convention, and CI (ALG-I18N-04, dynamic family `db-*`) checks that every code in our curated `db-codes.toml` has an English message.
- **Complexity / performance notes**: trivial; avoid formatting explanations for every NOTICE in tight loops (PL/pgSQL `RAISE NOTICE` floods) — batch notices and render lazily.
- **Edge cases (exhaustive list)**:
  1. PostgreSQL < 9.6 with Turkish `S = "HATA"` and no `V` → severity from message type; code from `C`.
  2. Turkish server message with `C = 23505` and `n = users_email_key` → explanation uses `n`; server text shown verbatim with `lang="tr"`.
  3. Errors during startup (auth) → no `SET` possible; handled by `C` (`28P01`) → localized "Password authentication failed for user { $user }" uses *our* known user name, not server text.
  4. `FATAL` termination (`57P01` admin shutdown) mid-query → connection-lost flow.
  5. `SET lc_messages` denied → swallow, record in connection diagnostics, keep server default.
  6. MySQL server with `lc_messages_dir` missing translations → server falls back to English silently [Observed: MySQL manual] — fine.
  7. SQL Server batch with several errors and info messages → list in order with line numbers relative to the batch; map to editor lines.
  8. Position beyond query length (server bug or rewritten query) → clamp; do not crash.
  9. Error text containing user data with RTL characters → sanitized & isolated; detail redacted in logs.
  10. Message in wrong encoding (server `client_encoding` mismatch) → forced UTF-8 at connect; residual invalid bytes → U+FFFD with raw hex in details.
  11. Unknown SQLSTATE (extension-defined, e.g. `XX` or custom `P0001` from `RAISE EXCEPTION`) → class-level or generic explanation; `P0001` → "Raised by a function" explanation + verbatim message (which is user-authored, any language).
  12. Redis error without a known prefix → generic.
  13. Driver/transport errors (TLS, DNS, timeout) → our own `HCV-CONN-*` codes, not DB codes.
  14. UI language `en` with Turkish server → explanation English, server text Turkish (`lang="tr"`).
  15. Machine sink requested while UI language Turkish → output contains no Turkish UI text.
- **Test cases (input → expected)**:
  | # | Input | UI | Expected title | Expected chip | Server text |
  |---|---|---|---|---|---|
  | 1 | PG `V=ERROR, S=HATA, C=23505, n=users_email_key, t=users, M=<Turkish text>` | tr | "Benzersizlik kısıtı ihlal edildi: users_email_key" (illustrative wording) | `SQLSTATE 23505` | verbatim, `lang="tr"` |
  | 2 | same | en | "Unique constraint violated: users_email_key" | `SQLSTATE 23505` | verbatim |
  | 3 | PG `C=42P01` (undefined table) no `t` field | en | "Table or view does not exist" | `SQLSTATE 42P01` | verbatim |
  | 4 | PG `C=40P01` | tr | class/specific deadlock explanation + "retry" hint | `SQLSTATE 40P01` | verbatim |
  | 5 | MySQL errno 1062 sqlstate 23000 | en | "Duplicate value for a unique key" | `MySQL 1062 (23000)` | verbatim |
  | 6 | SQL Server 2627 class 14 | en | "Unique/primary key constraint violated" | `SQL Server 2627` | verbatim |
  | 7 | PG `P=15` for query `SELECT * FROM  tbl` sent after Hacivat prefixed nothing | — | squiggle at char 15 | — | — |
  | 8 | Machine sink, case 1 | tr | (no title) JSON with `code:"23505"`, `server_message` verbatim, no `detail` in logs | — | — |
  | 9 | PG FATAL `28P01` at startup | tr | "Kullanıcı { $user } için parola doğrulaması başarısız" (illustrative) | `SQLSTATE 28P01` | verbatim |
  | 10 | Error text containing U+202E | any | — | — | `⟨RLO⟩` visible |
- **Prior art and evidence**: PostgreSQL error fields, non-localized `V` (9.6+), non-localizable `C` [Observed: PostgreSQL 18 protocol docs]; `lc_messages` privilege [Observed: PostgreSQL 18 runtime-config-client]; MySQL session `lc_messages` [Observed: MySQL 8.4 manual]; SQL Server per-language `sys.messages` [Observed: Microsoft docs via search].
- **Used by**: Hacivat | Karagöz (agent-side errors forwarded to Hacivat/Perde as structured fields) | Perde (audit viewer shows failed-statement codes)

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 Concrete design recommendations

| ID | Recommendation | Component |
|---|---|---|
| D-TR-01 | Create `sql-ident` crate implementing ALG-I18N-05 with `IdentKey` newtype; all metadata caches keyed by it; lint forbids `to_lowercase/to_uppercase` in SQL/driver crates. | Hacivat, Karagöz |
| D-TR-02 | Auto-limit/pagination rewriting only on parsed statements (sqlparser-rs AST); keyword checks via ASCII-insensitive comparison; regression test from DBeaver #40901. | Hacivat |
| D-TR-03 | Introspection queries use canonical system-object case; no server-side `lower()/upper()` on identifiers; tested against Turkish-collated PostgreSQL, SQL Server (`Turkish_CI_AS`) and MySQL (`utf8mb4_tr_0900_ai_ci`). | Hacivat, Karagöz (discovery) |
| D-TR-04 | UI search via ALG-I18N-06 (Rust + TS ports sharing a JSON test-vector file). | Hacivat, Perde web |
| D-TR-05 | Object-tree sorting with ICU4X collator (regional format) + numeric + code-point tiebreak; "sort like database" toggle. | Hacivat |
| D-TR-06 | Grid default raw mode; localized opt-in; ALG-I18N-07 for editing/paste; private clipboard format for lossless internal copy/paste; CSV presets incl. "Excel — regional". | Hacivat |
| D-TR-07 | DB errors via ALG-I18N-08; curated `db-codes.toml` with English explanations for the top ~150 SQLSTATEs/errnos at v1, Turkish translations for all. | Hacivat, Perde |
| D-TR-08 | Turkish style guide + glossary (§3.10) and CI suffix lint. | Process |
| D-TR-09 | Default keymap passes the AltGr-collision test for Turkish Q/F and major European layouts; shortcut matching normalizes `ı/İ/I/i`. | Hacivat |
| D-TR-10 | Bundle fonts with Latin Extended-A; `lang` attributes everywhere; no `text-transform` on translated strings; ligatures off in grid/editor by default. | Hacivat, Perde web |
| D-TR-11 | Visible rendering of bidi controls and invisible characters in identifiers, data and SQL. | Hacivat |
| D-TR-12 | "Turkey test" CI job (process locale `tr_TR.UTF-8`, browser `tr-TR`, Turkish-collated test databases). | All |

### 5.2 Karagöz-specific

- Karagöz classifies statements for permission enforcement (read vs write) — keyword detection must use ALG-I18N-05 step 1 exactly; e.g. `İNSERT INTO t …` is not a keyword match, the statement is unparsable, and Karagöz **denies by default** instead of guessing. [Inferred: security design; see `access/` and `mcp/` docs]
- Docker discovery labels/heuristics compare image names and label keys exactly/ASCII-insensitively; never locale-sensitive.
- CLI output localized; JSON and audit records locale-free.

### 5.3 Perde-specific

- Policies that reference object names (e.g. "deny `DROP` on schema `prod`") must be evaluated with the target engine's `match_key` semantics (ALG-I18N-05 step 6), not with the Perde server's locale or the UI language.
- Web panel tables use the same search algorithm (TS port) and `Intl.Collator(regionalFormat, {numeric: true})` with a code-point tiebreak.
- E-mails render numbers/dates in the recipient's regional format and time zone.

---

## 6. i18n considerations

- Every rule here reduces to: **machine text is locale-free; human text is locale-aware with an explicit locale; data is neither — it is shown and stored exactly.**
- Turkish is the v1 stress language; RTL is designed-for but not shipped in v1. Choosing a UI toolkit without bidi/shaping would make RTL a rewrite later — weigh it now.
- All test vectors in §4 must exist as shared JSON fixtures (`tools/l10n/vectors/*.json`) consumed by both Rust and TypeScript test suites so the two implementations cannot drift.

---

## 7. Risks, pitfalls, open questions

| # | Item | Type | Mitigation / next step |
|---|---|---|---|
| 1 | Turkish Q/F key positions stated here are from general knowledge | Unknown | Verify with Microsoft keyboard layout tables (KBDTUQ/KBDTUF) and macOS layouts before implementing the collision test. |
| 2 | Root vs Turkish collation sample orders are inferred from CLDR rules | Resolved — Observed (PG ICU + MySQL 0900, verification/01-db-behavior-tests.md#pg-turkish) | Still assert in unit tests with ICU4X. |
| 3 | SQL Server identifier resolution under Turkish collations documented mainly by community reports | Resolved — Observed (SQL Server 2022 container, verification/01-db-behavior-tests.md#mssql-turkish): DB collation governs object/column names (`information_schema.tables`, `SYS.INDEXES`, `items` vs `Items` fail); variables and `#temp` names follow the instance collation | Keep the CI case with both a Turkish database and a Turkish instance. |
| 4 | PostgreSQL behavior when non-superuser sends `lc_messages` in startup options (connection failure vs warning) | Unknown | Test against PG 13–18. |
| 5 | MySQL `lower_case_table_names` folding for non-ASCII names | Partially resolved — Linux `lower_case_table_names=0`: table names exact (`Items` ≠ `items`), column names Unicode case-insensitive (`MÜŞTERİ` = `Müşteri`), verification/01-db-behavior-tests.md#mysql-turkish | Test lctn=1/2 on macOS/Windows servers. |
| 6 | Excel's paste parsing of HTML vs text clipboard under Turkish regional settings | Unknown | Manual test matrix on Windows/macOS Excel and LibreOffice; tune "Copy for spreadsheet". |
| 7 | Accent-insensitive search default may surprise users searching for exact Turkish words | Risk | Toggle in search box; remember per user. |
| 8 | PostgreSQL numeric rounding mode for preview (half away from zero) | Inferred | Confirm with tests; for MySQL DECIMAL and SQL Server DECIMAL verify rounding vs truncation per engine and use engine-specific preview. |
| 9 | Bidi-control visibility may break legitimate RTL data display | Risk | Show visible chips only for *override/embedding/isolate* controls and in identifiers/SQL; for data cells show them subtly with an option to hide. |
| 10 | Developers bypassing `sql-ident` with ad-hoc `to_lowercase` | Pitfall | clippy `disallowed-methods` config + "Turkey test" CI. |
| 11 | Webfont subsetting in Perde web dropping Latin Extended-A | Pitfall | Visual regression test with `tr-XT` pseudo-locale. |
| 12 | Glossary choices (e.g. "sütun" vs "kolon", "işlem" for transaction) need community validation | Open | Collect feedback from Turkish DB users during beta. |
