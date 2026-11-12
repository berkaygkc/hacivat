# 01 — Localization Architecture for Hacivat, Karagöz and Perde

> Status: research, implementation-grade. Language of this document: English.
> Companion document: `02-turkish-and-db-specific-pitfalls.md` (Turkish casing, collation, number/date handling, database-client-specific rules, RTL readiness).
> Algorithm IDs in this library: ALG-I18N-01 … ALG-I18N-04 are specified here; ALG-I18N-05 … ALG-I18N-08 are specified in document 02.

---

## 1. Scope and sources

### 1.1 Questions this document answers

1. Which message format should be the single source of truth for every user-facing string in the product family (Hacivat desktop client, Karagöz agent CLI, Perde server, Perde web panel, Perde e-mails)?
2. Which Rust crates and which web libraries implement that format well enough, and what are their exact limitations?
3. How do comparable Rust applications (Firefox, COSMIC, Zed, Lapce) localize, and what can be learned from them?
4. How do we share one catalog between a Rust backend and a web frontend without semantic drift?
5. What is the end-to-end process: key naming, extraction, CI checks, translation platform, contributor workflow, runtime language switching, fallback chains, per-component catalogs, documentation translation?
6. How does a contributor add a new language **without any code change**?

Turkish-specific and database-client-specific pitfalls are deliberately in document 02; this document references them where architecture depends on them.

### 1.2 Sources studied

Repositories were shallow-cloned into `.refs/repos/<name>` and read as data only (no builds, scripts or tests were run).

| Source | Version / commit | License | How used |
|---|---|---|---|
| `projectfluent/fluent-rs` | HEAD `0f87afbbc3fe7d20c61a3fc75cb5d0e123aefdda`; crates: fluent-bundle 0.16.0, fluent-fallback 0.7.2, fluent-syntax 0.12.0, fluent-pseudo 0.3.3, fluent-resmgr 0.0.8, intl-memoizer 0.5.3, fluent 0.17.0 | Apache-2.0 OR MIT (`fluent-rs/Cargo.toml` `[workspace.package] license`) | Read bundle, resolver, builtins, number type, fallback, pseudo sources |
| `kellpossible/cargo-i18n` (i18n-embed 0.16.0, i18n-embed-fl 0.10.1, i18n-config 0.4.8) | HEAD `11b49ea83a12d96bd3689b561262d4db47615311` | MIT (`cargo-i18n/LICENSE`, `Cargo.toml:115`) | Read loader, requester, `fl!` macro checks |
| `longbridge/rust-i18n` 4.2.4 | HEAD `5f3ea5ba03395f2f2908e3587e01788cfdfd3555` | MIT | Read README/feature list |
| `unicode-org/icu4x` (workspace 2.4.0) | HEAD `a391f6c33b91fec2ec87f581b13716baf68f5fe0` | Unicode License V3 (`icu4x/LICENSE`) | Read casemap, collator options, components list, host_info, fixed_decimal, CLDR 49 test data |
| `projectfluent/fluent.js` (@fluent/bundle 0.19.1, @fluent/react 0.15.2, @fluent/dom 0.10.2, @fluent/langneg 0.7.0, @fluent/syntax 0.19.0) | HEAD `9a925d2a38b893be735ff4429be8ad62132a204d` | Apache-2.0 | Read NUMBER builtin implementation |
| `zed-industries/zed` | HEAD `912159c7ad77ce6296a93be77efc298b0b00fd50` | Apache-2.0 and GPL-3.0 (dual files `LICENSE-APACHE`, `LICENSE-GPL`; crate-level choice) | Read workspace deps and `crates/time_format` — prose only, no code reuse (GPL parts) |
| `pop-os/cosmic-files` `src/localize.rs` | `master` via raw.githubusercontent.com, fetched 2026-11-11 | GPL-3.0 [Inferred: COSMIC apps are GPL-3.0; verify before any reuse] | Read structure only; described in prose |
| Unicode MessageFormat 2.0 | Stable in CLDR/LDML 47 (March 2025) | Unicode License | Unicode blog + TR35 part "Messages" |
| Weblate, Crowdin, Tolgee, Pontoon docs/pages | fetched 2026-11-11 | n/a | Platform features/licensing |
| Web search results | 2026-11-11 | n/a | Lapce, Zed, paraglide-js, TC39 status, Firefox migration |

### 1.3 Evidence conventions

Tags follow the library README: **[Observed: …]** (file path + line, URL), **[Inferred]** (with basis), **[Unknown]** (open question). Code from permissively-licensed sources is described in prose or our own pseudocode; no source is pasted.

---

## 2. Executive summary

1. **Use Project Fluent (FTL) as the single source-of-truth message format for every component.** It is the only format with a mature, permissively licensed Rust implementation (fluent-rs, Apache-2.0/MIT), a first-party JS implementation (fluent.js, Apache-2.0), CLDR plurals, terms, attributes, asymmetric localization and built-in bidi isolation. [Observed: fluent-rs, fluent.js repos]
2. **Unicode MessageFormat 2 (MF2) is now stable (LDML 47, March 2025) but has no production-grade Rust implementation** in ICU4X as of the studied commit; `Intl.MessageFormat` is still a TC39 Stage 1 proposal. MF2 is the long-term direction; a Fluent→MF2 converter (`@messageformat/fluent`) already exists, so choosing Fluent now does not strand us. [Observed: Unicode blog; icu4x tree has no MF2 component; tc39 draft "Stage 1, May 12 2025"]
3. **fluent-bundle's built-in `NUMBER()` is not locale-aware in Rust** — it only pads fraction digits and prints `1234.5` regardless of locale; fluent.js uses `Intl.NumberFormat` and prints `1.234,5` for Turkish. To share catalogs without drift, Rust must install a custom formatter (`FluentBundle::set_formatter`) and custom `NUMBER`/`DATETIME` functions backed by ICU4X. [Observed: `fluent-bundle/src/builtins.rs:4-13`, `fluent-bundle/src/types/number.rs:148-160`, `fluent.js/fluent-bundle/src/builtins.ts:34-92`]
4. **Use i18n-embed + i18n-embed-fl for Rust binaries**: the `fl!` macro checks at compile time that a message ID exists in the fallback (English) catalog and that the named arguments match. This makes English-first a type-checked contract. [Observed: `cargo-i18n/i18n-embed-fl/src/lib.rs:542,585,649`]
5. **Do not use rust-i18n** for this product: YAML/JSON/TOML key-value catalogs with `format!`-style interpolation and no CLDR plural selection make correct Turkish/Arabic/Polish plurals impossible without code. [Observed: `rust-i18n/README.md` feature list]
6. **Use ICU4X 2.x for all locale-sensitive formatting and text operations** (plurals, decimal, date/time, collation, case mapping including `fold_turkic`, list formatting, segmentation; relative time/display names/units are still in `icu_experimental`). [Observed: `icu4x/components/*`, `components/casemap/src/casemapper.rs:240,273,307`]
7. **Separate three locale settings**: UI language (message catalogs), regional format locale (numbers/dates/collation in UI chrome), and data display mode (raw vs. localized in the result grid). Many Turkish developers run English UIs with Turkish regional formats, or vice versa. [Inferred: from OS designs (macOS/Windows separate "language" and "region") and DB-client practice; see doc 02]
8. **Repository layout: `locales/<bcp47-tag>/<component>.ftl` plus `locales/<tag>/locale.toml`**. Adding a language = add a folder (copy of `en`), translate, open a PR. A build script enumerates folders; no Rust/TS code changes. [Inferred: design]
9. **Fallback is per message, not per file**: requested chain e.g. `[tr-TR → tr, en]`; a missing Turkish message falls back to English individually. This is exactly fluent-fallback's semantics. [Observed: `fluent-fallback/src/bundles.rs:145-195`]
10. **Keep bidi isolation ON** (`use_isolating = true`, default). FSI/PDI (U+2068/U+2069) around every placeable is mandatory for future Arabic/Hebrew and harmless elsewhere — but must be stripped when output is machine-consumed (CLI `--json`, logs, e-mail subjects in some clients, MCP output). [Observed: `fluent-bundle/src/resolver/pattern.rs:50-67`]
11. **Never localize machine contracts**: logs, JSON output keys, error codes, SQL, identifiers, data values, MCP tool names/descriptions (see `mcp/02-…`). Human output is localized; machine output is English and stable. [Inferred: design]
12. **Perde web panel: use `@fluent/bundle` + `@fluent/react` (or the framework-equivalent binding) consuming the very same `.ftl` files**, plus generated TypeScript types for message IDs and variables to recover the compile-time checking Rust gets from `fl!`. [Observed: fluent.js package versions; Inferred: codegen design]
13. **Translation platform: Weblate** (GPL-3.0, self-hostable, gratis "Libre" hosting for public libre projects, Fluent support since 4.8). Crowdin's free OSS license excludes applicants with "commercial products related to the open-source project", which conflicts with a paid hosted Perde. Pontoon has the best Fluent editor but is heavy to self-host. Tolgee's official images bundle a proprietary EE module. [Observed: docs.weblate.org Fluent page; crowdin.com OSS form; tolgee.io licensing]
14. **CI must validate catalogs** (syntax/Junk, missing/obsolete IDs, variable-set equality, attribute-set equality, term references, plural/selector sanity, forbidden characters, key naming, unused IDs, length budgets) — specified as ALG-I18N-04.
15. **Pseudo-localization is a first-class build mode** (`en-XA` accented+elongated, `ar-XB` mirrored/bidi, plus a Turkish stress mode injecting `İıŞşĞğ`). fluent-pseudo provides a base transform but lacks bidi markers and Turkish stress; we extend it (ALG-I18N-03). [Observed: `fluent-pseudo/src/lib.rs`]
16. **Comparable Rust apps**: Firefox (origin of Fluent, uses Pontoon), COSMIC apps (i18n-embed + `fl!` + rust-embed, ICU collator for sorting) are the closest models; Zed has no runtime i18n (hard-coded English, e.g. "Today"/"Yesterday"), and Lapce declared no i18n plans — both illustrate the cost of not designing i18n in from day one. [Observed: zed `crates/time_format/src/time_format.rs:175-195`; Lapce discussion #2743]
17. **UI-stack implication**: a Tauri/web UI gets bidi layout, complex shaping, `lang`-aware casing/hyphenation and font fallback from the browser engine; native Rust toolkits vary (Slint has gettext-based `@tr()`; iced/egui/GPUI have no i18n layer). Our Fluent layer must be toolkit-agnostic either way. [Inferred: framework documentation knowledge; verify at stack decision time]
18. **Runtime language switching without restart** is supported by design: catalogs and formatters live behind a versioned `L10n` handle; UI re-renders on a `locale-changed` event; native menus/tray are rebuilt.
19. **Documentation**: v1 docs in English only; UI links to English docs. Later, use a gettext-based docs pipeline (e.g. mdbook-i18n-helpers) through the same Weblate instance. [Inferred]
20. **Turkish grammar forces catalog style rules** (no runtime suffix concatenation after placeholders, no CSS `text-transform` on translated strings, terms with grammatical-case variants). Details in doc 02 §3.10.

---

## 3. Detailed findings

### 3.1 Requirements restated and derived constraints

| # | Requirement (README) | Derived constraint |
|---|---|---|
| R1 | Source strings are English-first | English catalog (`en`) is the *reference* catalog: every message must exist there; it is the last element of every fallback chain; compile-time checks run against it. |
| R2 | v1 ships English + Turkish | Turkish exercises the hardest Latin-script issues (casing, collation, number format, agglutinative grammar). Getting Turkish right validates most of the architecture for other Latin/Cyrillic languages. |
| R3 | Adding a language is fast, requires no code changes | Locale discovery must be data-driven (directory enumeration + manifest), not an enum in code. The language picker reads native names from the catalog itself. Formatting data (CLDR) must already cover all locales (ICU4X compiled data does). |
| R4 | Hacivat works offline, no login | All catalogs embedded in the binary (rust-embed) — no download at runtime. Optional user overlay directory for testing translations. |
| R5 | Three products, five surfaces (desktop UI, agent CLI, server API, web panel, e-mail) | One format, one repository layout, one CI validator, one translation project with per-surface components. |
| R6 | P2P architecture: Karagöz enforces permissions and audit locally | Audit records and logs must be locale-independent (message ID + structured args), rendered in the viewer's language at display time. |
| R7 | Native MCP server | Tool names/descriptions stay English (LLM-facing contract); results carry data verbatim. Covered in `mcp/02-…`. |

### 3.2 Message formats

#### 3.2.1 Project Fluent (FTL)

Fluent is Mozilla's localization system; its syntax (FTL) reached 1.0 in 2019 and is the format used across Firefox's UI. [Inferred: Fluent 1.0 release date from general knowledge; Firefox use Observed via Firefox source docs "Fluent for Firefox Developers" (firefox-source-docs.mozilla.org/l10n/fluent/tutorial.html), which states DTD is deprecated in favor of Fluent]

**Core concepts** (examples are our own, written for Hacivat):

```ftl
### Resource comment: applies to the whole file.
### Component: Hacivat desktop UI — connection dialog.

## Group comment: Connection dialog

# Simple message. ID uses kebab-case.
conn-dialog-title = New connection

# Message with a variable (placeable). Variables start with $.
# Variables:
#   $name (String) - user-chosen connection name, e.g. "prod-replica".
conn-saved = Connection { $name } saved.

# Attributes: several related strings under one ID (value is optional).
conn-host-input =
    .label = Host
    .placeholder = localhost
    .aria-label = Database server host name

# Terms: reusable, translator-owned fragments. IDs start with "-".
# Terms cannot be retrieved directly by code; only referenced from messages.
-brand-hacivat = Hacivat
-brand-perde = Perde

about-title = About { -brand-hacivat }

# Select expression on a plural category (CLDR rules for the active locale).
# The asterisk marks the mandatory default variant.
grid-rows-fetched =
    { $count ->
        [0] No rows fetched
        [one] { $count } row fetched
       *[other] { $count } rows fetched
    }

# Select on a string variable (e.g. a grammatical gender or a mode).
tx-mode-label =
    { $mode ->
        [auto] Auto-commit
        [manual] Manual commit
       *[other] { $mode }
    }

# Message reference.
grid-empty-hint = { grid-rows-fetched } Try removing the filter.

# Escaping a literal brace uses a string literal placeable.
sql-brace-hint = Use { "{" } and { "}" } for JSON objects.
```

Key semantics:

- **Plural selection is implicit**: when the selector is a number, Fluent compares numeric literal keys first (`[0]`, `[1]`), then the CLDR plural category of the *bundle's* locale (`zero/one/two/few/many/other`). Rust uses the `intl_pluralrules` crate for categories. [Observed: `fluent-bundle/src/types/plural.rs:3,15,20`]
- **Default variant `*[...]` is mandatory**, so a message always resolves even if a locale has categories the source did not anticipate. This is what makes **asymmetric localization** possible: English may have two variants while Turkish has one, Arabic six, and Polish four — without code changes.
- **Terms with parameters** allow grammatical case, which matters for Turkish:

```ftl
# tr/hacivat-ui.ftl (illustrative)
-brand-hacivat =
    { $case ->
       *[nominative] Hacivat
        [accusative] Hacivat'ı
        [dative] Hacivat'a
        [genitive] Hacivat'ın
        [ablative] Hacivat'tan
    }
update-available = { -brand-hacivat(case: "genitive") } yeni bir sürümü var.
```

  The English source does not need to know these cases exist. [Inferred: standard Fluent term-parameter feature; syntax per Fluent spec]
- **Attributes** keep a widget's label, tooltip, placeholder, accessibility label and access key under one ID so translators see them together; code requests `id` + `attr`.
- **Comments** at three levels (`#` message, `##` group, `###` resource) flow into translation platforms as context. Variable documentation in comments is our convention (§3.6.1).
- **Error recovery**: unparsable entries become `Junk` and the rest of the file still loads; resolver errors (unknown variable, cyclic reference) still produce a best-effort string with errors reported separately. [Observed: fluent-fallback returns `Some(result)` even when `format_errors` is non-empty, `fluent-fallback/src/bundles.rs:157-167`]
- **Bidi isolation**: every placeable's output is wrapped in U+2068 FIRST STRONG ISOLATE … U+2069 POP DIRECTIONAL ISOLATE when `use_isolating` is true (default). [Observed: `fluent-bundle/src/resolver/pattern.rs:50-67`; default true at `fluent-bundle/src/bundle.rs:641` and `concurrent.rs:39`; toggle `set_use_isolating` at `bundle.rs:331`] This prevents a Hebrew table name from reordering the surrounding Turkish/English sentence, and an English identifier from breaking an Arabic sentence. Consequence: strings contain invisible characters — tests must compare with isolation disabled or with marks stripped; machine outputs must strip them (§3.5.10).
- **Built-in functions**: `NUMBER()` and `DATETIME()` in fluent.js (both backed by `Intl`), but **only `NUMBER()` in fluent-rs**, and it merely merges options into the value. [Observed: `fluent-bundle/src/builtins.rs:1-13` defines only `NUMBER`; `fluent.js/fluent-bundle/src/builtins.ts:34-92` uses `Intl.NumberFormat`]
- **Custom functions**: `FluentBundle::add_function(id, fn)` registers named functions callable from FTL. [Observed: `fluent-bundle/src/bundle.rs:535`]
- **Formatter hook**: `set_formatter(fn(&FluentValue, &IntlMemoizer) -> Option<String>)` lets the host format every number/custom value. [Observed: `fluent-bundle/src/bundle.rs:352-354`]
- **Transform hook**: `set_transform(fn(&str) -> Cow<str>)` is applied to literal text — this is how pseudo-localization is wired in. [Observed: `fluent-bundle/src/bundle.rs:342`]
- **Overrides**: `add_resource_overriding` replaces existing messages instead of erroring on duplicates — useful for a user overlay directory. [Observed: `fluent-bundle/src/bundle.rs:297`]

Fluent's known weaknesses for us:

1. Rust number/date formatting is the host's job (point 3 of the summary).
2. Fluent syntax evolution is effectively frozen at 1.0 while the ecosystem moves toward MF2. [Inferred: no Fluent 2.0 spec activity; Mozilla engineers drive MF2 and the `@messageformat/fluent` converter]
3. Translation platforms' Fluent editors vary: Weblate lists "Plurals: No" for Fluent (translators edit selector syntax as text), Crowdin's FTL format details likewise say pluralization is not supported in its UI; Pontoon has a native Fluent-aware editor. [Observed: docs.weblate.org/en/latest/formats/fluent.html; store.crowdin.com/ftl]

#### 3.2.2 GNU gettext / PO

- Source English string is the key (`msgid`), optional `msgctxt` disambiguates, `msgid_plural` + `msgstr[n]` handles plurals, with a per-language `Plural-Forms` header containing a C expression (Turkish: `nplurals=2; plural=(n != 1);` — some historical Turkish catalogs used `nplurals=1`). [Inferred: gettext manual knowledge]
- Strengths: universal tool support (every platform, Weblate/Crowdin/Pontoon/Transifex, poedit, msgmerge, fuzzy matching, translation memory), extraction from source with `xgettext`, Slint uses gettext natively.
- Weaknesses for us: plural logic lives in a header expression instead of CLDR; only `n`-based plurals (no select on gender/strings, no attributes); source-string-as-key means fixing an English typo invalidates all translations (fuzzy); no terms; no built-in bidi isolation; Rust runtime crates (`gettext-rs` binds libintl; `gettext` pure Rust) are less ergonomic and give no compile-time checks of arguments; web parity requires a separate JS runtime.

#### 3.2.3 ICU MessageFormat 1 (MF1)

- Syntax: `{count, plural, =0 {No rows} one {# row} other {# rows}}`, `select`, `selectordinal`, nested arguments, apostrophe quoting.
- Ubiquitous on the web (FormatJS/react-intl, i18next-icu, Lingui, Android, ICU4J/ICU4C).
- Weaknesses: the apostrophe escaping rules are a perennial source of translator bugs (`'` vs `''`, and Turkish uses apostrophes constantly for suffixes on proper nouns — `PostgreSQL'e`); no terms/attributes; inconsistent implementations of `#`, offsets and `=N` across libraries; Rust support is thin (no maintained, ICU4X-backed MF1 formatter of note). [Inferred: ecosystem knowledge; the apostrophe/Turkish interaction is a design inference with high confidence because MF1 treats `'` as an escape when followed by `{`, `}`, `#` or `|`]

#### 3.2.4 Unicode MessageFormat 2 (MF2)

- Status: **Stable since CLDR/LDML 47 (March 2025)**; stability policy normative; some default functions in the `u:` namespace remained Draft; LDML 49 text is a draft. [Observed: blog.unicode.org/2025/03/unicode-cldr-47-release-messageformat-2.html; unicode.org/reports/tr35/49/tr35-messageFormat.html]
- Syntax sketch (our own example):

```
.input {$count :number}
.match $count
0   {{No rows fetched}}
one {{{$count} row fetched}}
*   {{{$count} rows fetched}}
```

  Simple messages are plain text with `{$var}` placeables; functions are annotations (`{$d :datetime dateStyle=medium}`); markup is `{#b}…{/b}`; bidi isolation is a defined formatting option.
- Implementations: ICU4J/ICU4C ship MF2 as technical preview APIs [Inferred: ICU release notes 75/76; Observed: ICU 76 header `messageformat2.h` exists in Debian sources]; JavaScript `messageformat` v4 (MF2) [Observed: npm `messageformat@4.0.0` README]; **ICU4X has no MF2 formatter** in the studied tree (only TC minutes `documents/minutes/icu4x-tc_2025-02_MessageFormat.md` recommending to the WG; ecosystem table lists `icu::MessageFormat` as "Uncertain"). [Observed: `icu4x/documents/research/ecosystem.md:30`]
- `Intl.MessageFormat` in ECMAScript: Stage 1 per the published draft (May 12, 2025). [Observed: tc39.es/proposal-intl-messageformat] A third-party blog claims a later stage; unverified. [Unknown]
- Migration tooling: `@messageformat/fluent` converts Fluent resources to MF2 and back (BETA, Apache-2.0). [Observed: npmjs.com/package/@messageformat/fluent; messageformat.github.io API docs]

#### 3.2.5 Comparison and decision

| Criterion | Fluent | gettext/PO | ICU MF1 | MF2 |
|---|---|---|---|---|
| Rust runtime maturity | Good (fluent-rs; used by Firefox's Rust localization stack [Inferred]) | Medium | Weak | None production-grade (ICU4X lacks it) |
| JS runtime | Good (@fluent/*) | Medium (gettext.js etc.) | Excellent | Good (`messageformat` v4), native Intl pending |
| CLDR plurals | Yes (implicit on numbers) | No (header expression) | Yes | Yes |
| Select on strings | Yes | No | Yes | Yes |
| Asymmetric localization | Yes (core design) | No | Partially | Yes |
| Terms / shared fragments | Yes (`-term`, parameterized) | No | No | No (proposals only) |
| Attributes (label/tooltip/aria together) | Yes | No | No | No |
| Bidi isolation built in | Yes (default on) | No | No | Yes (option) |
| Compile-time key/arg checks in Rust | Yes (`fl!`) | No | No | No |
| Translator tooling | Pontoon (native), Weblate, Crowdin (text-level selectors) | Universal | Universal | Emerging |
| Escaping pain for Turkish apostrophes | None (apostrophe is plain text) | None | High | None |
| Stability | 1.0 frozen | Frozen | Frozen | Stable core since 2025 |

**Decision: Fluent now, with an MF2 exit path.** Constraints for exit-path friendliness: avoid exotic Fluent features with no MF2 analog in *source* (English) messages — keep English messages to plain text, variables, plural/string selectors and term references. Terms remain fine (converter maps them to messages). [Inferred]

### 3.3 Rust ecosystem

#### 3.3.1 fluent-rs crates

| Crate | Role | Notes for us |
|---|---|---|
| `fluent-syntax` 0.12 | Parser/AST/serializer for FTL | Used by our CI validator (ALG-I18N-04) to inspect variables, selectors, attributes, Junk. |
| `fluent-bundle` 0.16 | `FluentResource` (parsed file), `FluentBundle<R, M>` (locale-specific message set + functions), `FluentArgs`, `FluentValue` | Default `use_isolating=true`; `concurrent` module provides a `Sync` bundle for multi-threaded servers (`fluent-bundle/src/concurrent.rs`). |
| `intl-memoizer` 0.5 | Caches per-locale formatters (e.g. plural rules) | Our ICU4X formatters should be memoized the same way (per locale + options). |
| `fluent-fallback` 0.7 | `Localization`: ordered list of locales × resource IDs → bundle iterator; per-message fallback; sync/async | Its README itself warns the API "is in the early stages" ergonomically. [Observed: `fluent-fallback/src/lib.rs` doc header] |
| `fluent-resmgr` 0.0.8 | File-system resource manager | We use embedded assets instead. |
| `fluent-pseudo` 0.3 | Pseudo-localization transforms | Accented map, "flipped" map, elongation by duplicating vowels, DOM-aware variant skipping XML entities/tags, optional `[` `]` markers. [Observed: `fluent-pseudo/src/lib.rs:6-75`] |
| `fluent-langneg` 0.13 | Language negotiation (Filtering/Matching/Lookup) | Used by i18n-embed with `Filtering`. [Observed: `i18n-embed/src/lib.rs:186-190`] |

**Observed limitation — numbers**: `FluentNumber::as_string` converts the `f64` with Rust's `to_string` and only appends zeros to satisfy `minimumFractionDigits`; grouping, locale decimal separator and `maximumFractionDigits` are not applied. The `NUMBER()` builtin only merges named options. [Observed: `fluent-bundle/src/types/number.rs:148-160`; `builtins.rs:4-13`] Hence in Rust, `{ $count }` with `count = 1234.5` renders `1234.5` even in a Turkish bundle, while fluent.js renders `1.234,5`. **This is a correctness and parity bug we must close** (§3.4.4).

**Observed limitation — dates**: no `DATETIME()` builtin in Rust. We register our own.

**Value precision**: `FluentValue::Number` holds an `f64` plus options. Never pass database values (NUMERIC with 30 digits, BIGINT above 2^53) into Fluent as numbers; pass pre-formatted strings for data, and numbers only for counts used in plural selection. [Inferred: from type definition in `fluent-bundle/src/types/number.rs`]

#### 3.3.2 i18n-embed and i18n-embed-fl

- `i18n-embed` 0.16 provides `LanguageLoader` (trait), `FluentLanguageLoader` (loads `{assets}/{lang}/{domain}.ftl` from an `I18nAssets` source such as a `rust-embed` struct), `DefaultLocalizer`, `LanguageRequester` with listener notification (for runtime switching), `DesktopLanguageRequester` (OS languages), optional autoreload. [Observed: `i18n-embed/src/lib.rs:77-336`; `requester.rs:6-290`]
- Negotiation uses `fluent_langneg::negotiate_languages(..., NegotiationStrategy::Filtering)` against available languages, with the configured fallback language always appended. [Observed: `i18n-embed/src/lib.rs:172-200`; `fluent.rs:482-495`]
- `i18n-embed-fl` 0.10 `fl!(loader, "id", arg = value)`: at compile time it loads the fallback-language FTL (path from `i18n.toml`) and aborts compilation if the message ID does not exist, and checks the provided argument names against the variables referenced by the message. [Observed: `i18n-embed-fl/src/lib.rs:542` and `585` call `check_message_args`; `649`/`679` emit "does not exist in the `fallback_language`" errors; README example]
- Convention: one *domain* per crate (`i18n/{lang}/{crate_name}.ftl`). We may override the asset path so all crates read from the shared top-level `locales/` (i18n.toml `assets_dir`). [Observed: README `i18n.toml` keys `fallback_language`, `[fluent] assets_dir`]
- Gap: `fl!` checks only the fallback language. Translations are validated by our CI (ALG-I18N-04).

#### 3.3.3 rust-i18n

YAML/JSON/TOML key→string maps compiled into the binary, `t!("key", var = value)`, configurable fallback chain, territory fallback (`zh-CN`→`zh`), minified hashed keys, `cargo i18n` extractor. [Observed: `rust-i18n/README.md` Features] No CLDR plural categories, no select, no terms, no bidi isolation. **Rejected** for this product.

#### 3.3.4 ICU4X (2.x)

Components present in the studied tree: `calendar, casemap, collator, collections, datetime, decimal, experimental, list, locale, locale_core, locale_fallback, normalizer, pattern, plurals, properties, segmenter, time`; experimental: `dimension, displaynames, duration, measure, personnames, relativetime, transliterate, units`. [Observed: `ls icu4x/components`, `components/experimental/src`]

| Need | ICU4X API | Notes |
|---|---|---|
| Locale parsing/canonicalization, likely subtags | `icu_locale` (`Locale`, `LocaleExpander`, `LocaleFallbacker`) | Replaces `unic-langid` long term; fluent-rs still uses `unic-langid` 0.9 [Observed: `fluent-rs/Cargo.toml` workspace deps], so a conversion boundary exists. |
| Plural categories | `icu_plurals` | For our own selectors outside Fluent (e.g. CLI tables). |
| Number formatting | `icu_decimal::DecimalFormatter` over `fixed_decimal::Decimal` | Decimal is arbitrary precision; parse machine strings with `Decimal::try_from_str` (no locale parsing). [Observed: `utils/fixed_decimal/src/signed_decimal.rs:59,156`] Locale-aware *parsing* is not provided → we specify our own (ALG-I18N-07, doc 02). |
| Dates/times | `icu_datetime` with field sets (`fieldsets::YMD::long()` etc.) | Host preferences via `icu_host_info` (OS regional settings → ICU preferences). [Observed: `utils/host_info/README.md`] |
| Collation | `icu_collator` with strength, numeric ordering, case-first etc. | [Observed: `components/collator/src/options.rs:360-751`] |
| Case mapping / folding | `icu_casemap::CaseMapper::{lowercase,uppercase,titlecase_segment…}` with a `LanguageIdentifier`; `fold`/`fold_turkic` | `uppercase_to_string("istanbul", tr) == "İSTANBUL"`; `fold_turkic_string("İstanbul") == "istanbul"`. [Observed: `components/casemap/src/casemapper.rs:240,273,307,438`] |
| List formatting | `icu_list` | Turkish: "{0} ve {1}", "{0} veya {1}". [Observed: CLDR 49 test data `cldr-misc-full/main/tr/listPatterns.json`] |
| Relative time | `icu_experimental::relativetime` | Experimental API; pin versions. |
| Display names (language names) | `icu_experimental::displaynames` | For "Türkçe (Turkish)" in the language picker; experimental. Alternative: store native name in `locale.toml` (preferred — zero runtime dependency, translator-controlled). |
| Segmentation | `icu_segmenter` | Word/grapheme boundaries for cursor movement and truncation with ellipsis (never cut inside a grapheme). Zed depends on `icu_segmenter` 2.3. [Observed: `zed/Cargo.toml:656`] |

Data: with `compiled_data` all CLDR locales are baked into the binary; this is what makes "add a language, no code change" true for formatting too. Binary-size cost is real but acceptable for a desktop app; for Karagöz (server agent) we can restrict to a smaller locale set via a blob provider or datagen filters if size matters. [Inferred: ICU4X data model; quantify at implementation time — Unknown exact MB]

#### 3.3.5 Locale detection crates

- `sys-locale` (MIT/Apache-2.0): returns the OS's preferred UI locale(s) as BCP-47 strings on Windows, macOS, Linux (env vars), Android, iOS, WASM. Zed uses it. [Observed: `zed/Cargo.toml:926`; crate behavior Inferred from docs]
- `icu_host_info`: OS *regional preferences* (calendar, hour cycle, first weekday, number system) mapped into ICU4X preferences. [Observed: `icu4x/utils/host_info/README.md`]
- POSIX: `LC_ALL` > `LC_MESSAGES` (for UI language) and `LC_ALL` > `LC_NUMERIC`/`LC_TIME` (for formats) > `LANG`; GNU `LANGUAGE` is a colon-separated priority list for messages. COSMIC reads `LC_TIME`, `LC_ALL`, `LANG` in that order for its formatting locale. [Observed: cosmic-files `src/localize.rs`, fetched 2026-11-11]

#### 3.3.6 UI toolkit implications (stack still undecided)

| Stack | Built-in i18n | Bidi/shaping | Consequence |
|---|---|---|---|
| Tauri + web frontend | None (bring a JS lib) | Browser engine: full UBA, HarfBuzz-class shaping, `lang`-aware `text-transform`, font fallback, `dir="auto"` | Strongest RTL/Turkish story; Fluent JS + shared FTL. |
| Slint | gettext `@tr("…")` macro with `.po` files bundled at build [Inferred: Slint docs] | Supported to a degree [Unknown: verify RTL completeness] | Would push us toward PO; or bypass `@tr` and feed strings from our Fluent layer via properties. |
| iced | None | cosmic-text (bidi + shaping via rustybuzz) [Inferred] | COSMIC proves Fluent + iced works (libcosmic is iced-based). [Observed: cosmic-files uses i18n-embed + `fl!`] |
| egui | None | Limited bidi/complex-script support [Inferred: long-standing egui issues] | RTL future is a risk. |
| GPUI (Zed) | None | Platform text systems [Inferred] | Zed itself has no i18n. |

Decision rule: **the l10n layer (`hacivat-l10n` crate) must not depend on the UI toolkit**; it exposes `fn msg(id, args) -> String`, `fn attr(id, attr, args)`, formatters, collators and a change-notification channel.

### 3.4 Web side (Tauri frontend and Perde web panel)

#### 3.4.1 Candidate libraries

| Library | Format | License | Notes |
|---|---|---|---|
| `@fluent/bundle` 0.19.1, `@fluent/react` 0.15.2, `@fluent/dom` 0.10.2, `@fluent/langneg` 0.7.0, `@fluent/syntax` 0.19.0 | Fluent | Apache-2.0 [Observed: package.json files] | Same format as Rust; NUMBER/DATETIME via `Intl`. `@fluent/react` provides `LocalizationProvider`, `<Localized id attrs>`, `useLocalization()`. |
| i18next (+ react-i18next, + `i18next-fluent` plugin) | JSON (own syntax), ICU via plugin, Fluent via plugin | MIT [Inferred] | Huge ecosystem; Fluent plugin is a translation layer, not native. |
| FormatJS / react-intl | ICU MF1 | [Inferred: MIT/BSD; verify] | Excellent `Intl` polyfills; MF1 apostrophes problematic for Turkish. |
| Lingui 5 | ICU MF1 / PO, compiled catalogs, macros | MIT [Inferred] | Extraction from source with English as key. |
| Paraglide JS 2 (inlang) | inlang format; plugins for others (ICU MF1, i18next…) | MIT per README (older 1.x package.json said Apache-2.0) [Observed: github.com/opral/paraglide-js; socket.dev diffs] | Compiles each message into a tree-shakable function; no first-party Fluent plugin found. [Unknown] |
| `messageformat` v4 | MF2 | Apache-2.0 [Observed: https://registry.npmjs.org/messageformat/latest — 4.1.0, Apache-2.0, checked 2026-11-11] | Future MF2 path; `@messageformat/fluent` converter. |

**Recommendation**: `@fluent/bundle` (+ `@fluent/react` if the panel/front-end is React; for Svelte/Solid/Vue a ~150-line binding over `@fluent/bundle` is sufficient). Rationale: one format, one validator, one translation project, identical selector semantics.

#### 3.4.2 Recovering compile-time safety in TypeScript

Generate `l10n-ids.d.ts` from `locales/en/*.ftl` in a build step (our own small script using `@fluent/syntax` or the Rust validator emitting JSON):

```ts
// generated — do not edit
export type MessageId = "conn-dialog-title" | "grid-rows-fetched" | /* … */;
export interface MessageArgs {
  "grid-rows-fetched": { count: number };
  "conn-saved": { name: string };
  "conn-dialog-title": Record<string, never>;
}
export interface MessageAttrs { "conn-host-input": "label" | "placeholder" | "aria-label" }
```

and a typed wrapper `t<K extends MessageId>(id: K, args: MessageArgs[K])`. Variable types are taken from the `# Variables:` comment convention (§3.6.1); untyped variables default to `string | number`. CI fails if the generated file differs from the committed one (or generate at build time and never commit).

#### 3.4.3 Sharing one catalog between Rust and web

```
                      locales/
                      ├── en/ (reference)          ├── tr/              ├── <new>/
                      │   common.ftl               │   common.ftl       │   ...
                      │   hacivat-ui.ftl           │   hacivat-ui.ftl
                      │   hacivat-core.ftl         │   ...
                      │   karagoz-cli.ftl
                      │   perde-server.ftl
                      │   perde-web.ftl
                      │   perde-email.ftl
                      │   locale.toml
                               │
            ┌──────────────────┼──────────────────────────────┐
            ▼                  ▼                              ▼
  Rust binaries (rust-embed     Tauri frontend /             CI validator
  include filters per binary)   Perde web (Vite import of    (ALG-I18N-04) + TS
  fluent-rs + ICU4X formatter   .ftl as raw text; lazy-load   type generator
                                per locale; @fluent/bundle)
```

Rules for parity:

1. **Same bundle construction**: same resource order (`common.ftl` first, then component file), same `use_isolating`, same functions available under the same names (`NUMBER`, `DATETIME`, plus our `LIST`?), same option subset.
2. **Rust NUMBER/DATETIME implemented with ICU4X** honoring the ECMA-402 option names translators use: `minimumFractionDigits`, `maximumFractionDigits`, `useGrouping`, `style: percent` (CLDR Turkish percent pattern is `%#,##0`, prefix percent sign [Observed: CLDR 49 `tr/numbers.json`]); `DATETIME(dateStyle, timeStyle)`. Unsupported options → error + ignore (consistent in both runtimes by restricting the allowed set in CI).
3. **Golden parity tests**: a fixtures file `l10n-parity.ftl` with representative messages and a JSON list of `(locale, id, args) → expected` run by both a Rust test and a JS test in CI. Any divergence (e.g. `Intl` vs ICU4X CLDR version differences in Turkish month abbreviations) is caught. [Inferred: design] Note the CLDR version skew risk: the browser's `Intl` data (WebView2/WKWebView/WebKitGTK version) may differ from ICU4X's baked CLDR; parity tests should run against the WebView engines we ship (at least in nightly CI) and tolerate documented differences.
4. **Formatting authority in Tauri**: for values that are *also* shown by Rust (e.g. a row count in the status bar and in a native notification), prefer formatting in one place. Practical rule: data-derived formatting (grid cells, sizes, durations) is done in Rust and sent as strings; purely presentational messages are formatted where rendered.
5. **Message IDs are global across components** (prefix by component, §3.6.1) so the frontend and backend never collide when both load `common.ftl`.

#### 3.4.4 Rust formatter integration (design, own pseudocode)

```text
struct L10nRuntime {
    ui_chain: Vec<LanguageIdentifier>,       // e.g. [tr, en]
    format_locale: icu::locale::Locale,      // e.g. tr-TR (may differ from ui_chain[0])
    bundles: Vec<FluentBundle>,              // one per ui_chain entry
    decimal_fmt_cache: Memo<(Locale, NumberOptions) -> DecimalFormatter>,
    date_fmt_cache:    Memo<(Locale, DateOptions)   -> DateTimeFormatter>,
}

fn build_bundle(lang, resources, format_locale):
    b = FluentBundle::new([lang])
    b.set_use_isolating(true)
    b.set_formatter(Some(format_value))              // numbers → ICU4X with format_locale
    b.add_function("NUMBER", number_fn)              // validates allowed options, returns FluentValue::Number with options
    b.add_function("DATETIME", datetime_fn)          // accepts ISO-8601 string or epoch ms; returns custom FluentValue
    for r in resources: b.add_resource(r) (collect duplicate-id errors → CI already prevents)
    if pseudo_mode: b.set_transform(Some(pseudo_transform))
    return b
```

Important subtlety: **plural selection uses the bundle's language, formatting uses `format_locale`**. With UI language `en` and format locale `tr-TR`, "1.234 rows fetched" must select English `other` and print the Turkish-formatted number. Fluent's plural selection reads the number value, not the formatted string, so this works as long as the formatter only affects output. [Inferred: from resolver design — selection compares `FluentValue::Number` category before formatting]

### 3.5 Architecture for the product family

#### 3.5.1 Repository layout

```
locales/
  en/
    locale.toml            # manifest (see below)
    common.ftl             # brand terms, units, generic verbs, generic errors
    hacivat-ui.ftl         # desktop UI chrome
    hacivat-core.ftl       # Hacivat engine errors/notifications (shared by UI and CLI if any)
    karagoz-cli.ftl        # Karagöz human-facing CLI output
    perde-server.ftl       # API error messages for humans (rendered by web panel or API clients)
    perde-web.ftl          # web panel
    perde-email.ftl        # e-mail subjects/bodies (one message per template part)
  tr/
    locale.toml
    ... same file names ...
  _pseudo/                 # not a locale; config for pseudo modes (dev builds only)
tools/l10n/                # validator + TS type generator (Rust xtask)
```

If Hacivat, Karagöz and Perde live in separate repositories, keep `locales/` in a dedicated `hacivat-l10n` repository consumed as a git submodule or a published crate + npm package with the FTL files; one translation project then serves all repos. [Inferred: design trade-off; monorepo is simpler]

`locale.toml` schema (our own):

```toml
tag = "tr"                       # BCP-47; must equal the folder name
native_name = "Türkçe"           # shown in the language picker
english_name = "Turkish"
direction = "ltr"                # "ltr" | "rtl"
status = "stable"                # "stable" | "beta" | "experimental" | "disabled"
min_completeness = 0.9           # CI warning threshold per component (stable only)
maintainers = ["@handle"]        # review owners (GitHub handles; optional)
plural_categories = ["one", "other"]   # informational; CI derives real ones from CLDR
fonts_note = ""                  # e.g. require Noto Sans Arabic for "ar"
```

Nothing else is required to add a language. `direction` feeds `dir` on the web root and layout mirroring in native UIs; `status` decides visibility (experimental languages appear only behind "Show incomplete languages").

#### 3.5.2 Discovery: build-time and runtime

- **Build time (Rust)**: a `build.rs` in `hacivat-l10n` enumerates `locales/*/locale.toml`, validates tags, and generates a static table `AVAILABLE: &[(&str tag, &str native_name, Direction, Status)]`. The `rust-embed` struct embeds `locales/` with per-binary include filters (e.g. Karagöz embeds only `common.ftl` and `karagoz-cli.ftl`). Adding a folder changes data, not code. [Inferred: design; rust-embed supports `#[include]`/`#[exclude]` globs — verify version]
- **Build time (web)**: Vite `import.meta.glob('/locales/*/perde-web.ftl', { query: '?raw' })` creates one lazy chunk per locale automatically; `locale.toml` parsed at build time for the picker. Again no code change per language.
- **Runtime overlay (Hacivat desktop and Karagöz)**: optional directory `<config_dir>/hacivat/locales/<tag>/*.ftl` (and `HACIVAT_L10N_DIR`). Loaded *after* embedded resources with `add_resource_overriding`, so translators can preview edits without rebuilding; a new tag present only in the overlay becomes selectable when it has a `locale.toml`. Overlay is disabled for Perde server in production (security: no file-system-controlled strings in e-mails) unless an admin sets `PERDE_L10N_OVERLAY_DIR`. [Inferred: design]

#### 3.5.3 Locale settings model

| Setting | Where stored | Default source | Affects |
|---|---|---|---|
| `ui_language` | Hacivat: local settings file; Karagöz: flag/env/config; Perde: user profile (server-side), cookie for anonymous pages | ALG-I18N-01 over OS/browser languages | Message catalogs |
| `regional_format` | Same | OS regional settings (`icu_host_info`, `LC_NUMERIC`/`LC_TIME`), browser `navigator.languages[0]` for web, else `ui_language` | Numbers, dates, collation, list formatting, first weekday in UI chrome |
| `grid_data_display` (Hacivat only) | Per user, overridable per connection | `raw` (see doc 02 §3.8) | Result-grid cell rendering/editing |
| `time_zone` | Per user (Perde: profile; Hacivat: OS) | OS | Date rendering in UI; DB session time zone is separate |

#### 3.5.4 Per-component catalog policy

| Component | Localized? | Notes |
|---|---|---|
| Hacivat UI | Yes | Everything visible in chrome: menus, dialogs, tooltips, status bar, notifications, accessibility labels. |
| Hacivat engine errors (connection failures, parser diagnostics) | Yes (message) + stable code | `HCV-CONN-0003` code + localized text + English text available on "Copy error details". |
| DB server messages | No (passed through) + localized *explanation* by SQLSTATE | See doc 02 ALG-I18N-08. |
| Hacivat MCP server | **No** — English tool names, descriptions, schemas and result framing | LLM-facing contract; localization would change model behavior and break prompt caching/evals. Covered in `mcp/02-…`. The *approval dialogs* shown to the human in Hacivat UI are localized. |
| Karagöz CLI human output | Yes | `karagoz status`, prompts, help text (clap help can be fed from Fluent [Inferred]). |
| Karagöz CLI machine output (`--json`, exit codes) | No | Keys, enum values and codes stable English; human `message` field may be included in English plus `message_id` + `args` so a UI can re-render it. |
| Karagöz logs / audit events | No (English) + `message_id` + structured args | Ops grep-ability, log pipelines, support. Audit viewer in Perde renders localized text from ID+args. |
| Perde server API | Error body: `code` (stable), `message` (English), optional `localized_message` per `Accept-Language`, `message_id` + `args` | Web panel prefers rendering from `message_id` + `args` itself. |
| Perde web panel | Yes | Same FTL as server's `perde-web.ftl`. |
| Perde e-mails | Yes, **recipient's** language | Never the sender's or the admin's. Subject, preheader, HTML and plain-text parts all from Fluent. |
| CLI `--help` man pages, shell completions | Help yes (runtime), man pages English in v1 | Man pages generated per locale later. |
| Documentation | English in v1 | §3.6.8. |

#### 3.5.5 Fallback chains

- **Message-level**: chain from ALG-I18N-01, e.g. `[tr, en]`; each message resolved by the first bundle that contains it (ALG-I18N-02). English always last.
- **Regional**: `tr-CY` requested → `tr` catalog (language match) with `tr-CY` formats (CLDR has `tr-CY`). `pt-BR` with only `pt-PT` available → configurable "accept regional sibling" (default yes for UI text, never for formats). [Inferred: design]
- **Never cross-language**: `az` (Azerbaijani) must not fall back to `tr` even though they are close; falls back to `en`. Users can choose Turkish explicitly. (CLDR does not define az→tr parent.) [Inferred]
- **Script-aware**: `sr-Latn` must not take `sr-Cyrl`; `zh-HK` → `zh-Hant`; implemented by comparing maximized scripts (likely subtags).

#### 3.5.6 Runtime language switching

1. User changes `ui_language` (or `regional_format`).
2. `L10nRuntime` builds new bundles/formatters off the UI thread, swaps an `Arc` atomically, increments `generation`.
3. Emits `locale-changed { generation, ui_chain, format_locale, direction }`.
4. Subscribers re-render: native UI re-evaluates bound strings (toolkit-specific); Tauri frontend reloads its `@fluent/bundle` set for the new chain and sets `<html lang dir>`; native menus/tray menus are rebuilt (OS menus are static objects); open dialogs re-render; the SQL editor and grid are *not* affected except UI chrome (cell values are data).
5. Things that cannot switch live and are documented: OS-provided file dialogs (follow OS language), macOS app menu items provided by the OS, already-sent notifications.
6. Karagöz: language is per invocation (flag/env); the daemon's *logs* are English, so no switching needed.
7. Perde: per request (user profile → cookie → `Accept-Language` → org default → `en`); e-mails per recipient at send time; queued e-mails store `locale` at enqueue time to avoid races.

#### 3.5.7 OS/package metadata localization (generated, still no code change)

- macOS: `CFBundleLocalizations` (declare `en`, `tr`, …) so system-provided UI elements in the app follow the selected language; `InfoPlist.strings` per `.lproj` for privacy-usage descriptions. Generate from FTL messages `os-macos-*` at bundle time.
- Windows: installer (MSI/NSIS) language selection and strings; Start-menu shortcut names.
- Linux: `.desktop` `Name[tr]=`, `Comment[tr]=`, `Keywords[tr]=`; AppStream metainfo `<name xml:lang="tr">`.
- The bundling script iterates `locales/*` — adding a language automatically adds these. [Inferred: platform conventions; verify against Tauri bundler capabilities at stack decision time]

#### 3.5.8 Error architecture (codes + messages)

- Every Hacivat/Karagöz/Perde error has a stable code `AREA-NNNN` (e.g. `HCV-CONN-0003`, `KRG-DISC-0012`, `PRD-AUTH-0001`) and a Fluent message ID `error-hcv-conn-0003` (Fluent IDs must start with a letter and contain only `[a-zA-Z0-9_-]`).
- Logs record `code`, `message_id`, args, and the English rendering.
- UI shows localized message + code (searchable in docs) + "Copy details" (English + code + args, so support can read it).

#### 3.5.9 Strings that must be assembled carefully

- **No concatenation of translated fragments** (`msg("delete") + " " + name`). Whole sentences only.
- **No runtime suffix logic** for Turkish (doc 02 §3.10): restructure messages so placeholders never take inflectional suffixes, or use terms with case variants for known nouns.
- **Lists of items** via `LIST` function backed by `icu_list` (Turkish "a, b ve c"), not `join(", ")`.
- **Units and sizes** ("12,5 MB", "3 sn") via ICU4X/our unit messages in `common.ftl`.
- **Rich text**: Fluent messages may contain a small allow-list of markup (`<b>`, `<code>`, `<a data-l10n-name="docs">`) rendered by the UI layer (fluent.js `@fluent/react` supports element overlays); the validator ensures markup parity with English.

#### 3.5.10 Bidi marks in machine contexts

Strip U+2068/U+2069 (and never insert U+200E/U+200F) when the rendered string goes to: CLI output with `--json`/`--porcelain`, log lines, clipboard copies of *data*, HTTP headers, e-mail `Subject` (keep isolation in bodies; many mail clients render FSI/PDI fine but some legacy clients show boxes [Inferred]), file names, MCP outputs. Implementation: render with a bundle variant whose `use_isolating=false` for those sinks (cheap: two bundle sets sharing parsed resources via `Rc`/`Arc<FluentResource>`). For Arabic/Hebrew CLI output in terminals, keep isolation (terminals with bidi support benefit; others show nothing harmful). [Inferred]

### 3.6 Process

#### 3.6.1 Key naming and comment conventions

- **Format**: kebab-case, `<component>-<area>-<element>[-<qualifier>]`.
  - Components: `cm` (common), `hv` (Hacivat UI), `hc` (Hacivat core), `kg` (Karagöz CLI), `ps` (Perde server), `pw` (Perde web), `pe` (Perde e-mail), `error` (codes).
  - Examples: `hv-conn-dialog-title`, `hv-grid-rows-fetched`, `hv-grid-cell-null-tooltip`, `kg-status-agent-online`, `pw-audit-filter-actor`, `pe-invite-subject`, `error-hcv-conn-0003`.
- **Attributes** for the same widget: `.label`, `.tooltip`, `.placeholder`, `.aria-label`, `.accesskey`, `.title`.
- **Terms** for brand/product nouns: `-brand-hacivat`, `-brand-karagoz` (display "Karagöz"), `-brand-perde`; the CLI binary name stays ASCII `karagoz` and is never a term (it is a command, not prose).
- **Variable documentation** (mandatory for every message with variables; enforced by CI):

```ftl
# Shown in the status bar after a query finishes.
# Variables:
#   $count (Number) - number of rows fetched so far.
#   $elapsed (String) - pre-formatted duration, e.g. "1,2 sn".
hv-status-rows-fetched = …
```

- **Meaning change ⇒ new ID** (Fluent convention: changing an ID invalidates translations everywhere, which is what we want when meaning changes; pure typo fixes in English keep the ID and translations stay valid). [Inferred: Mozilla l10n practice]
- **No reuse across contexts**: "Open" (verb, menu) and "Open" (adjective, status) are different IDs.
- **No sentence fragments as IDs** and **no English text in IDs beyond a short hint**.
- **Never put SQL, identifiers or format patterns in FTL** except as examples inside comments.

#### 3.6.2 Extraction

Fluent inverts gettext's flow: developers author `en/*.ftl` directly (there is no `xgettext` pass). "Extraction" therefore means *usage scanning* to find unused or undefined IDs:

- Rust: `fl!` literals (compile-time checked for existence) and any dynamic IDs (forbidden except via an explicit `l10n_dynamic!(…)` allow-list macro, e.g. for error codes) → scanner collects literals with a syn-based or regex-based pass.
- TS: typed `t("…")`/`<Localized id="…">` (type generator enforces existence) → scanner collects literal IDs.
- Dynamic families (error codes `error-*`, enum-derived `hv-tx-mode-*`) are declared in `tools/l10n/dynamic-ids.toml` with patterns, so the unused-ID check does not flag them.

#### 3.6.3 CI checks (summary; full algorithm in ALG-I18N-04)

Errors (block merge): FTL syntax errors/Junk in any locale; duplicate IDs; English message missing variable documentation; translation references a variable not in English; translation drops a variable that English uses in *all* variants (warning if only some variants); attribute set mismatch; reference to an undefined message/term; markup mismatch; ID naming violation; forbidden characters (raw U+202A–U+202E bidi overrides, U+2066–U+2069 typed manually, NUL, tabs in single-line); `locale.toml` invalid or tag ≠ folder; obsolete IDs in translations (IDs absent from English) — auto-removed by bot or error.
Warnings (report, do not block): missing translations (fallback to English); completeness below `min_completeness` for `stable` locales; plural-category coverage gaps (e.g. Arabic lacking `few`); translation identical to English for long strings (possible untranslated paste); length ratio > budget for IDs tagged `# MaxLength: 24` comments.

#### 3.6.4 Translation platforms

| Platform | License | Hosting for OSS | Fluent support | Fit |
|---|---|---|---|---|
| **Weblate** | GPL-3.0-or-later (server); using it does not affect our code license | Hosted "Libre" plan gratis for eligible public libre projects (application, limits like the 160k-string plan); self-hosting needs no license [Observed: weblate.org/hosting; third-party reviews] | Fluent format since 4.8, monolingual (`locales/*/hacivat-ui.ftl`, base `locales/en/…`), syntax/parts/references/inner-HTML checks; plurals not modeled (selectors edited as text) [Observed: docs.weblate.org formats/fluent] | **Recommended**: git-native (PRs back to repo), per-file components, glossary, translation memory, MT suggestions, review workflow. |
| Crowdin | Proprietary SaaS | Free for OSS on application; criteria include OSI license, public source, **"You do not have any commercial products related to the open-source project"**, being the project lead; translations contribute to Crowdin Global TM [Observed: crowdin.com/product/for-open-source; request form mirrors] | Native FTL support (no extra install), pluralization not supported in UI, source text not editable in Crowdin [Observed: store.crowdin.com/ftl] | Risky: hosted Perde is a paid product related to the project → eligibility doubt. |
| Pontoon (Mozilla) | BSD-3-Clause [Observed: `gh api repos/mozilla/pontoon/license` → BSD-3-Clause, 2026-11-11] | Mozilla's instance serves Mozilla projects; others must self-host (Django + PostgreSQL) [Inferred] | Best-in-class Fluent editor (renders selectors/attributes as forms) [Observed: locize page citing native Fluent support; Inferred: UX] | Great UX, heavy ops; consider only if Weblate's text-level selector editing proves painful. |
| Tolgee | Apache-2.0 core; proprietary "Tolgee EE License" for `ee/` and `webapp/src/ee`; official binaries/images bundle EE modules [Observed: tolgee.io/licencing] | Cloud free tier limited; no specific OSS program found [Unknown] | [Unknown: Fluent import/export support] | Not recommended now. |

Weblate setup: one Weblate *project* "Hacivat", one *component* per FTL file (`hacivat-ui`, `karagoz-cli`, …), monolingual base `locales/en/<file>.ftl`, file mask `locales/*/<file>.ftl`, new-language template empty, "Add-on: Squash Git commits" and "Update ALL_LINGUAS"-like add-ons not needed. Weblate pushes to a branch and opens PRs; CI runs ALG-I18N-04 on them. [Observed: Weblate Fluent file-mask example]

#### 3.6.5 Contributor workflow: adding a language (target: folder + files, zero code)

1. **Request**: open an issue "Add language: <name> (<tag>)" (template asks for native name, direction, at least one reviewer).
2. **Create folder**: maintainer or contributor runs `cargo xtask l10n new <tag>` *or* manually copies `locales/en/` to `locales/<tag>/`, empties message values (the tool writes a skeleton with comments preserved), and fills `locale.toml` (`status = "experimental"`). In Weblate, "Start new translation" does the equivalent and opens a PR.
3. **Translate** in Weblate or by editing FTL directly; run the app with `HACIVAT_LANG=<tag>` or select it under "Show incomplete languages"; use the overlay directory for instant preview without rebuild.
4. **CI** validates (ALG-I18N-04). No Rust/TS changes are needed: discovery is by directory; CLDR data for formats is already compiled in; plural categories come from CLDR.
5. **Promotion**: `experimental → beta → stable` by editing `locale.toml` once completeness and review thresholds are met (stable requires ≥ 90 % per component and a second reviewer).
6. **Fonts/RTL**: if `direction = "rtl"` or a script not covered by bundled fonts, the PR template asks for screenshots; the font fallback list is data (`fonts_note`) — still no code; actual RTL layout support is a one-time engineering investment (doc 02 §3.16), after which RTL languages are also data-only.

#### 3.6.6 String freeze and release flow

- Feature branches add English strings freely; translators work on `main` continuously.
- **String freeze** one week (or one release-candidate) before a release: only English typo fixes; Weblate announcement to translators.
- Releases ship whatever is translated; missing strings fall back to English per message.
- Release notes list per-language completeness (generated by the validator's JSON report).

#### 3.6.7 Machine translation and AI assistance

- Weblate MT suggestions (DeepL/LibreTranslate etc.) are allowed as *suggestions*; strings accepted from MT are marked "needs review" until a human approves.
- For Turkish v1, a native Turkish reviewer approves every string; the glossary (doc 02 §3.10.6) is loaded into Weblate's glossary.
- AI-drafted translations must preserve placeables, selectors and markup — the validator guarantees structure, not quality.

#### 3.6.8 Documentation translation

- v1: English-only docs (mdBook or similar). The UI links to docs via `cm-docs-url = https://…/en/…` messages so a later translated docs site can be targeted per locale by translating the URL message (falls back to English URL).
- Later: gettext-based docs pipeline (e.g. `mdbook-i18n-helpers` — PO per book, used by Google's Comprehensive Rust [Inferred: project knowledge]) through the same Weblate instance as a separate project; screenshots regenerated per locale by an automated UI script.
- API reference and CLI man pages stay English.

#### 3.6.9 Testing strategy

- Unit: ALG-I18N-01/02 test vectors (§4).
- Snapshot tests of key screens in `en`, `tr`, `en-XA` (accented, +40 %), `ar-XB` (mirrored) — screenshot diffs in CI for the web/Tauri UI.
- Parity tests Rust vs JS (§3.4.3).
- Turkish-locale process tests: run the whole Rust test suite and the frontend tests with `LANG=tr_TR.UTF-8`, `LC_ALL=tr_TR.UTF-8`, browser locale `tr-TR`, JVM-style "Turkey test" equivalents for any embedded scripting (doc 02 ALG-I18N-05 tests).

### 3.7 How other applications localize (lessons)

| App | Approach | Lesson |
|---|---|---|
| Firefox | Fluent (origin); migrated from DTD/.properties with `fluent.migrate` recipes; Pontoon for community translation [Observed: firefox-source-docs l10n migrations; mozilla-l10n docs] | Fluent scales to ~100 locales and a huge string base; migration tooling matters; Pontoon is optimized for Fluent. |
| COSMIC (cosmic-files) | `i18n-embed` `FluentLanguageLoader` + `fl!` wrapper bound to a static loader; `rust-embed` embeds `i18n/`; `DesktopLanguageRequester` for OS languages; ICU collator with numeric ordering for file sorting (current language → fallback → `en-US`); formatting locale from `LC_TIME`/`LC_ALL`/`LANG` with encoding stripped and `_`→`-` [Observed: cosmic-files `src/localize.rs`] | Exactly our intended Rust pattern, and it already separates "UI language" from "formatting locale" and uses ICU collation for user-visible lists. |
| Zed | No runtime i18n framework; English strings in code (e.g. "Today", "Yesterday" literals in `crates/time_format/src/time_format.rs:175-195`); OS date formatters on macOS/Windows (`time_format.rs:73-127`); `sys-locale` dependency [Observed] — community forks translate by rewriting string literals in source and recompiling (~66k keys) [Observed: x6nux/zed-globalization release notes via search] | Retro-fitting i18n into a large Rust UI is so expensive that the community resorts to source patching. Design in from day one. |
| Lapce | Maintainers stated (Nov 2023) there are no plans for i18n [Observed: github.com/lapce/lapce/discussions/2743 via search] | Same lesson. |
| DBeaver / DataGrip | Java resource bundles / IntelliJ bundles; DBeaver ships many community translations; Turkish casing bugs persist in Java code (doc 02) | Localization breadth is a competitive feature for DB clients in non-English markets; the Turkish bug class follows the platform's casing defaults. |

---

## 4. Algorithm specifications

### ALG-I18N-01: Locale negotiation and fallback chain

- **Problem**: From heterogeneous preference sources (explicit setting, CLI flag, environment variables, OS preferred-language list, HTTP `Accept-Language`, stored user profile, e-mail recipient preference), compute (a) an ordered **UI language chain** over the catalogs that actually exist, always ending in `en`, and (b) a **formatting locale** that may be a locale with no catalog at all.
- **Inputs / outputs**:
  - Inputs: `sources: Vec<PreferenceSource>` in priority order, each yielding raw strings; `available: Vec<CatalogLocale { tag, status, direction }>` from the generated table (+ overlay); `context: Desktop | Cli | PerdeRequest | Email`; `allow_experimental: bool` (true only when the user explicitly picked a language or enabled "show incomplete languages"); `region_hint: Option<String>` (OS regional format, e.g. from `icu_host_info` or `LC_NUMERIC`).
  - Outputs: `UiChain { tags: Vec<LanguageIdentifier> /* len ≥ 1, last == en */ , direction }`, `format_locale: Locale`, `diagnostics: Vec<Diag>`.
- **Algorithm**:
  1. **Collect raw preferences** by context:
     - Desktop: `settings.ui_language` (if not "system") → `HACIVAT_LANG` → OS list (`sys-locale` `get_locales()` order) → (Unix) `LANGUAGE` (split on `:`) → `LC_ALL` → `LC_MESSAGES` → `LANG`.
     - CLI (Karagöz): `--lang` → `KARAGOZ_LANG` → `LANGUAGE` → `LC_ALL` → `LC_MESSAGES` → `LANG` → OS list.
     - Perde request: `user.profile.language` → `perde_lang` cookie → `Accept-Language` (parse, drop `q=0`, stable-sort by q descending keeping header order for ties, cap at 16 entries, ignore `*`) → `org.default_language`.
     - E-mail: `recipient.profile.language` → `org.default_language` (never the actor's language).
  2. **Normalize each raw string** `s`:
     1. Trim; if empty → skip.
     2. Strip POSIX codeset and modifier: cut at first `.` or `@` (`tr_TR.UTF-8@euro` → `tr_TR`).
     3. If `s ∈ {"C", "POSIX"}` → treat as `en` and mark `diag: posix_default`.
     4. Replace `_` with `-`.
     5. Map deprecated codes: `iw→he`, `in→id`, `ji→yi`, `no→nb` (only for catalog lookup; keep original for formatting) — or rely on ICU4X canonicalization (`LocaleCanonicalizer`) when available.
     6. Parse as BCP-47 with ICU4X (`Locale::try_from_str`); on failure → skip with `diag: invalid_tag(s)`.
     7. Reserved pseudo tags `en-XA`, `ar-XB`, `tr-XT` (our Turkish stress mode) are accepted only in dev builds or when `HACIVAT_PSEUDO=1`; otherwise skipped.
     8. Deduplicate (first occurrence wins).
  3. **Match against available catalogs** (Filtering semantics, but producing one best match per request):
     For each requested `r` in order:
     1. Exact match on canonical tag (`tr-TR` vs available `tr-TR`).
     2. Else compute `r_max = maximize(r)` (likely subtags: `tr` → `tr-Latn-TR`, `zh-HK` → `zh-Hant-HK`, `sr` → `sr-Cyrl-RS`). For each available `a`, compute `a_max`.
        - Score 3: same language, script, region.
        - Score 2: same language + script (region differs or absent in `a`, e.g. `tr-CY` → `tr`).
        - Score 1: same language + script, `a` is a *regional sibling* (`pt-BR` → `pt-PT`) — only if `context` allows siblings (default yes for UI text).
        - Never match if languages differ (no `az → tr`) or scripts differ (`sr-Latn` ≠ `sr-Cyrl`).
     3. Pick the highest-scoring `a`; ties broken by: `status` (stable > beta > experimental), then the more generic tag (`tr` over `tr-TR` when request was `tr-CY`), then lexical order for determinism.
     4. Drop `a` if `status == disabled`, or `status == experimental && !allow_experimental`.
     5. Append `a` to `chain` if not already present.
  4. **Insert language-only parents**: for each `a` in chain with a region/script (e.g. `pt-BR`), if a more generic catalog (`pt`) exists and is not in chain, insert it directly after `a`.
  5. **Append default** `en` if absent. If the chain is empty before this step, `diag: no_match → en`.
  6. **Direction** = `available[chain[0]].direction`.
  7. **Formatting locale**:
     1. If `settings.regional_format` is explicit → parse and use it (any CLDR locale, catalog not required).
     2. Else, Desktop/CLI: OS regional preference (`icu_host_info` or `LC_ALL`→`LC_NUMERIC`/`LC_TIME`→`LANG`), normalized by step 2.
     3. Else Perde/web: the first *requested* tag (not the first matched catalog) — e.g. `de-DE` request → English UI, German formats.
     4. Else `chain[0]` maximized with its likely region (`tr` → `tr-TR`; `en` → `en-US`).
     5. Validate that ICU4X has data for it via `LocaleFallbacker` (it always resolves to at least `und`); record the resolved data locale in diagnostics.
  8. Return.
- **Data structures**: `CatalogLocale { tag: LanguageIdentifier, max: LanguageIdentifier, status: Status, direction: Dir }` precomputed at build time (maximized tags cached); a small `Vec` (≤ ~50 locales) — linear scans are fine.
- **Complexity / performance notes**: O(|requested| × |available|) with tiny constants; runs at startup, on settings change, and per Perde request (cache by the raw `Accept-Language` string + profile language in an LRU of ~1k entries).
- **Edge cases (exhaustive list)**:
  1. Empty/absent everything → `[en]`, format `en-US`.
  2. `LANG=C` / `POSIX` / `C.UTF-8` → `en` UI; formats: if `LC_NUMERIC` is set use it, else `en-US`.
  3. `LANG=tr_TR.ISO-8859-9` → `tr` (codeset irrelevant; we are UTF-8 internally).
  4. `LANGUAGE=tr:en_GB:en` → `[tr, en]`.
  5. `LANGUAGE` set but `LANG=C` → GNU gettext ignores `LANGUAGE` when locale is `C`; we deliberately **do not** emulate that quirk (document it). [Inferred: gettext manual behavior]
  6. Request `tr-CY` → `tr` catalog; formats `tr-CY` (if regional_format unset and request came from OS list with region).
  7. Request `az-Latn-AZ` → no match → `[en]`; never `tr`.
  8. Request `zh-HK` with catalogs `zh-Hans`, `zh-Hant` → `zh-Hant` (via maximization `zh-Hant-HK`).
  9. Request `zh` alone → `zh-Hans` (likely script Hans).
  10. Request `sr-Latn` with only `sr` (Cyrillic) → no match → `en`.
  11. Request `pt-BR` with only `pt-PT` → `pt-PT` (score 1) unless siblings disabled.
  12. Request `en-GB` with only `en` → `en`; formats `en-GB` (dd/MM/yyyy).
  13. `Accept-Language: tr;q=0, en` → `tr` excluded → `[en]`.
  14. `Accept-Language: *` → ignored → org default / `en`.
  15. `Accept-Language` with 200 entries or 8 KB → only first 16 by q considered; no quadratic blowup.
  16. Malformed tags (`tr_TR_`, `123`, `x-klingon`) → skipped with diagnostics; never panic.
  17. Uppercase/lowercase variants (`TR-tr`) → canonicalized to `tr-TR`.
  18. Experimental catalog `ku` present, OS requests `ku` → skipped unless `allow_experimental`; explicit user choice of `ku` → allowed (user setting implies allow).
  19. Overlay-only locale (only in `<config>/locales/xx/`) with valid `locale.toml` → treated as `experimental`.
  20. Pseudo tags in release builds → skipped.
  21. Duplicate entries (`tr`, `tr-TR`, `tr`) → chain `[tr, en]`.
  22. User picks a locale whose catalog later disappears (downgrade to a version without it) → diag + fall back through remaining sources; setting is preserved, not overwritten.
  23. RTL first catalog with LTR fallback (`ar` partial → `en`) → direction stays `rtl` (from chain[0]); English fallback strings are isolated per message by Fluent's FSI/PDI where they are placeables, and rendered with `dir="auto"` per text node when they are whole messages [Inferred: needed for mixed-direction UIs].
  24. E-mail recipient without profile language and org without default → `en`; never `Accept-Language` of the *sender's* request.
  25. Windows OS returns `tr-TR` and also a "display language" vs "regional format" split → UI from display language list, formats from regional setting.
- **Test cases (input → expected)**:
  | # | Context | Inputs | available | Expected chain | Expected format locale |
  |---|---|---|---|---|---|
  | 1 | Desktop | OS `[tr-TR, en-US]` | en, tr | `[tr, en]` | `tr-TR` (OS regional) |
  | 2 | Desktop | setting `en`, OS regional `tr-TR` | en, tr | `[en]` | `tr-TR` |
  | 3 | Desktop | OS `[de-DE]`, regional `de-DE` | en, tr | `[en]` | `de-DE` |
  | 4 | CLI | `LANG=tr_TR.UTF-8` | en, tr | `[tr, en]` | `tr-TR` |
  | 5 | CLI | `LANG=C`, `LC_NUMERIC=tr_TR.UTF-8` | en, tr | `[en]` | `tr-TR` |
  | 6 | CLI | `--lang az`, no `LC_*`/`LANG` | en, tr | `[en]` + diag no_match | `en-US` (an explicit UI language never sets the format locale) |
  | 7 | Perde | `Accept-Language: tr-TR,tr;q=0.9,en;q=0.8` | en, tr | `[tr, en]` | `tr-TR` |
  | 8 | Perde | profile `en` (no profile regional format), header `tr-TR` | en, tr | `[en]` (profile wins for UI) | `tr-TR` (first tag of the header) |
  | 9 | Perde | `Accept-Language: tr;q=0,en` | en, tr | `[en]` | `en-US` |
  | 10 | Desktop | OS `[zh-HK]` | en, zh-Hans, zh-Hant | `[zh-Hant, en]` | `zh-HK` |
  | 11 | Desktop | OS `[sr-Latn-RS]` | en, sr | `[en]` | `sr-Latn-RS` |
  | 12 | Desktop | OS `[pt-BR]` | en, pt-PT | `[pt-PT, en]` | `pt-BR` |
  | 13 | Desktop (dev) | `HACIVAT_PSEUDO=1`, setting `en-XA` | en, tr | `[en-XA, en]` | `en-US` |
  | 14 | Desktop (release) | setting `en-XA` | en, tr | `[en]` + diag | OS regional |
  | 15 | Email | recipient `tr`, sender UI `en` | en, tr | `[tr, en]` | `tr-TR` |
  | 16 | Desktop | OS `[]`, no env | en, tr | `[en]` | `en-US` |
  | 17 | CLI | `LANGUAGE=tr:en`, `LANG=en_US.UTF-8` | en, tr | `[tr, en]` | `en-US` |
  | 18 | Desktop | OS `[TR-tr]` (bad case) | en, tr | `[tr, en]` | `tr-TR` |
  Rows 6 and 8 define the rule explicitly: an explicit UI language never implies a format locale; formats come from the format sources in step 7.
- **Prior art and evidence**: fluent-langneg Filtering strategy as used by i18n-embed [Observed: `i18n-embed/src/lib.rs:186-190`]; i18n-embed always appends the fallback language [Observed: same function]; COSMIC formatting-locale derivation from `LC_TIME`/`LC_ALL`/`LANG` with codeset stripping [Observed: cosmic-files `src/localize.rs`]; RFC 4647 lookup/filtering; CLDR likely subtags (ICU4X `LocaleExpander`); Android/Chromium pseudo-locale tags `en-XA`/`ar-XB` [Inferred].
- **Used by**: Hacivat | Karagöz | Perde

### ALG-I18N-02: Message resolution with per-message fallback, plural and select

- **Problem**: Render message `id` (optionally attribute `attr`) with arguments, choosing the first locale in the chain that has a *usable* translation; resolve selectors (plural/string) correctly per the bundle's language; format values with the formatting locale; never show raw IDs or broken placeholders to end users in release builds.
- **Inputs / outputs**:
  - Inputs: `id: &str`, `attr: Option<&str>`, `args: FluentArgs` (values: `String`, `Number{f64, options}`, `DateTime{iso8601|epoch_ms}`, `List(Vec<String>)`), `chain: [Bundle]`, `sink: Ui | Machine`, `build: Dev | Release`.
  - Outputs: `Rendered { text: String, locale_used: LanguageIdentifier, fell_back: bool, errors: Vec<L10nError> }`.
- **Algorithm**:
  1. If `sink == Machine` select the non-isolating bundle set (§3.5.10).
  2. For each `bundle` in chain (index `k`):
     1. `msg = bundle.get_message(id)`; if none → record `MissingMessage(locale)`; continue.
     2. `pattern = attr.map(|a| msg.get_attribute(a).value()).unwrap_or(msg.value())`; if none → record `MissingValue/MissingAttribute(locale)`; continue.
     3. `errors_k = []`; `text = bundle.format_pattern(pattern, args, &mut errors_k)`. Inside Fluent resolution:
        - Select expression with a **number** selector: compute `n` as the `FluentValue::Number`; first try **exact numeric keys** (`[0]`, `[1]`, compared numerically: `1` matches `[1]` and `1.0` matches `[1]`); else compute the CLDR plural category of `n` *in the bundle's language* (with visible fraction digits from options: `minimumFractionDigits: 1` makes `1` → "1.0" → English `other`, Turkish `one`) and match the variant key by category name; else default variant.
        - Select with a **string** selector: exact string match on the key; else default.
        - Select on a **missing variable**: error `UnknownVariable`, default variant chosen.
        - Placeable of a missing variable: output `{$name}` and error. [Observed: fluent resolver behavior — missing references render as `{…}`; Inferred exact form]
        - Numbers are rendered by our formatter (ICU4X `DecimalFormatter` for `format_locale` with options); dates by our `DATETIME`.
     4. If `errors_k` is empty → return `Rendered{text, locale_used: bundle.lang, fell_back: k > 0}`.
     5. Classify errors:
        - **Translation-structural** (unknown variable not present in English signature, unknown message/term reference, cyclic reference, too-many-placeables) → in Release: record, **discard `text`, continue to next bundle** (we diverge from fluent-fallback, which returns the erroneous text [Observed: `fluent-fallback/src/bundles.rs:157-167`]); in Dev: return `text` wrapped in `⚠[...]` with errors so the bug is visible.
        - **Caller errors** (argument missing that English also needs; wrong type, e.g. string passed for a plural selector) → do not fall back (the same error would occur in English); return `text`, log at `error` with the call site; in Dev panic in tests (`debug_assert!`).
        - **Function errors** (bad `NUMBER` option value) → return `text` (formatter fell back to default formatting), log once per (id, locale).
  3. Chain exhausted:
     - Release: return `"…"`-free fallback: the English message cannot be missing (compile-time `fl!` check), so this only happens for dynamic IDs (error codes) → return the generic message `cm-error-unknown` with the code appended, e.g. "An unexpected error occurred (HCV-CONN-9999)".
     - Dev: return `⟦id⟧` visibly.
  4. Telemetry: none (offline-first). Missing/fallback events go to the local debug log only, rate-limited (one line per (id, locale) per session).
- **Data structures**: `Arc<[FluentBundle]>` per chain (two sets: isolating and non-isolating) sharing `Arc<FluentResource>`; `HashSet<(id, locale)>` for log de-duplication; English *signature table* `HashMap<MessageId, Signature{vars: Set, attrs: Set, plural_selectors: Set<var>}>` generated at build time (also used by CI).
- **Complexity / performance notes**: `get_message` is a hash lookup; formatting is linear in pattern size. Memoize ICU4X formatters per `(locale, options)` (intl-memoizer pattern). Grid status-bar updates may call this tens of times per second — budget < 5 µs per simple message [Inferred target]. Never format inside per-cell rendering loops (cells are data, not messages).
- **Edge cases (exhaustive list)**:
  1. Message exists in `tr` but only as attributes and value requested → MissingValue → fall back to `en` value.
  2. Attribute requested exists in `en` but not in `tr` → fall back for that attribute only (label may be Turkish while tooltip is English — acceptable).
  3. `tr` translation uses `$count` in `[one]` but English uses `$count` in all variants → fine (Turkish often writes "Tek satır" without number).
  4. `tr` references `$rows` (typo) not in English signature → structural → fall back to English in release; CI should have caught it.
  5. Plural selector receives a string "5" → Fluent treats it as string; no plural match; default variant → caller error logged; fix: pass numbers.
  6. `count = 1.0` with `minimumFractionDigits: 1` → English `other` ("1.0 rows"), Turkish `one` (CLDR tr `one: n = 1` includes `1.0` [Observed: CLDR 49 `plurals.json` tr rule "n = 1 @decimal 1.0, 1.00…"]).
  7. `count = 0` with `[0]` variant present in English but not Turkish → each locale independent; Turkish uses `other` ("0 satır") — acceptable; CI warns when English has an exact-number variant missing in a translation? No — asymmetric by design; no warning.
  8. Negative counts (`-1`) → CLDR uses absolute value; English `one`? (`n = 1` uses absolute value `n`) [Inferred: CLDR operand n is absolute value] → "−1 row"; avoid by not pluralizing negative numbers in UI.
  9. NaN/Infinity count → plural `other`; formatter prints locale NaN/∞ symbol (`∞` for tr per CLDR symbols [Observed: CLDR 49 tr `infinity: "∞"`, `nan: "NaN"`]).
  10. Huge counts (1e21) → formatting via `Decimal` from string, not f64 `to_string` (which would print exponent).
  11. Arg value contains bidi controls or newlines (user-supplied connection name) → value inserted inside FSI/PDI; newlines preserved; UI layer is responsible for single-line display (ellipsis).
  12. Arg value contains Fluent syntax (`{ $x }`) → inserted literally (values are never parsed).
  13. Message reference cycle `a = { b }`, `b = { a }` → Cyclic error → structural → fallback; CI blocks.
  14. Term with parameter missing in translation (`-brand(case: "dative")` but term lacks `[dative]`) → default variant of term; no error in Fluent; CI warns (term-variant coverage check for referenced keys).
  15. Machine sink → isolation marks absent; Ui sink → present.
  16. Pseudo-locale active → transform applied to literals only, never to placeable values (data stays real).
  17. Chain `[tr, en]` and `tr` bundle failed to load (corrupt overlay file) → bundle constructed with Junk dropped; messages that parsed are used; overlay errors surfaced in "Language diagnostics" panel.
  18. Concurrent locale switch while formatting → caller holds an `Arc` of the old chain; result consistent with old generation; UI re-renders after event.
- **Test cases (input → expected)** (isolation disabled for readability; `fmt=tr-TR` unless noted):
  | # | Chain | Message (en / tr) | Args | Expected |
  |---|---|---|---|---|
  | 1 | [tr,en] | en `{ $count -> [one] {$count} row *[other] {$count} rows }`; tr `{ $count } satır` | count=1 | `1 satır` |
  | 2 | [tr,en] | same | count=1234.5 | `1.234,5 satır` |
  | 3 | [en] (fmt=tr-TR) | same | count=1234 | `1.234 rows` |
  | 4 | [en] (fmt=en-US) | same | count=1 | `1 row` |
  | 5 | [tr,en] | tr missing | count=2 | `2 rows` (fell_back=true) |
  | 6 | [tr,en] | tr `{ $cuont } satır` (typo) | count=2 | Release: `2 rows`; Dev: `⚠[{$cuont} satır]` |
  | 7 | [en] | en `[0] No rows` exact | count=0 | `No rows` |
  | 8 | [en] | en plural with `NUMBER($count, minimumFractionDigits: 1)` | count=1 | `1.0 rows` (fmt en-US) |
  | 9 | [tr,en] | tr `{ NUMBER($count, minimumFractionDigits: 1) -> [one] Tam bir *[other] … }` | count=1 | `Tam bir` |
  | 10 | [tr,en] | attr `.tooltip` only in en | — | Turkish label + English tooltip |
  | 11 | [en] | dynamic `error-hcv-conn-9999` absent | — | `An unexpected error occurred (HCV-CONN-9999)` |
  | 12 | [tr,en], Ui sink | `Bağlantı { $name } kaydedildi.` | name=`prod` | `Bağlantı ⁨prod⁩ kaydedildi.` |
  | 13 | [tr,en], Machine sink | same | name=`prod` | `Bağlantı prod kaydedildi.` |
  | 14 | [en] | `{ $mode -> [auto] Auto-commit *[other] {$mode} }` | mode=`manual` | `manual` |
  | 15 | [en] | plural | count=`"5"` (string) | default variant + caller error logged |
- **Prior art and evidence**: fluent-fallback per-message fallback loop [Observed: `fluent-fallback/src/bundles.rs:145-195`]; error taxonomy [Observed: `fluent-fallback/src/errors.rs:6-24`]; plural rules via `intl_pluralrules` [Observed: `fluent-bundle/src/types/plural.rs`]; CLDR 49 Turkish plural rule [Observed: icu4x test data `cldr-core/supplemental/plurals.json`].
- **Used by**: Hacivat | Karagöz | Perde

### ALG-I18N-03: Pseudo-localization transform

- **Problem**: Detect hard-coded strings, truncation, concatenation, missing `lang`/`dir` handling, font coverage gaps, Turkish casing bugs and bidi layout bugs *before* real translations exist — without breaking placeables, markup, access keys or machine-readable fragments.
- **Inputs / outputs**: Input: a Fluent *literal text element* (never placeable values), mode ∈ {`accented` (en-XA), `bidi` (ar-XB), `turkish` (tr-XT)}, options {`expansion` (default 0.4), `markers` (default true)}. Output: transformed text.
- **Algorithm**:
  1. Tokenize the literal into protected and translatable spans. Protected: XML/HTML tags `<…>`, entities `&…;`, URLs (`https?://\S+`), `%`-free tokens that look like keyboard shortcuts in our convention (`Ctrl+K` is *not* in FTL text; shortcuts come from code), sequences inside backticks or `<code>` elements (SQL examples), and any span listed in a per-message `# PseudoSkip:` comment. (Fluent placeables are separate pattern elements, so they are inherently protected because `set_transform` only receives text elements. [Observed: transform applies to text; `fluent-bundle/src/bundle.rs:342`])
  2. Skip transformation if the whole literal is 1 grapheme (access keys) — fluent-pseudo does the same for 1-byte strings. [Observed: `fluent-pseudo/src/lib.rs` `transform_dom` early return on `s.len() == 1`]
  3. Mode `accented`: map ASCII letters through an accented table (à la fluent-pseudo's `TRANSFORM_*_MAP`); elongate by duplicating vowels until the transformed length ≥ `ceil(len × (1 + expansion(len)))` where `expansion(len)` follows the length-budget table (doc 02 §3.18: ≤10 chars → +100 %, 11–20 → +80 %, 21–30 → +60 %, 31–50 → +40 %, 51–70 → +30 %, >70 → +20 %); pad with `·` if vowels are insufficient.
  4. Mode `bidi`: replace letters with the "flipped" table (fluent-pseudo `FLIPPED_*_MAP`), wrap each *word* in U+202E RIGHT-TO-LEFT OVERRIDE … U+202C POP DIRECTIONAL FORMATTING (Chromium's ar-XB style) and set the bundle direction to `rtl` so layout mirrors. [Inferred: ar-XB convention]
  5. Mode `turkish`: first apply `accented`, then substitute to exercise Turkish risks: every `i`→`ı` in odd word positions and `i`→`i` kept elsewhere, `I`→`İ` alternately, `s`→`ş`, `g`→`ğ`, `c`→`ç`, `o`→`ö`, `u`→`ü` (where accented map didn't already change them); this surfaces font fallback (latin-ext subset), `text-transform` misuse (doc 02 §3.12) and casing bugs when strings are programmatically upper/lower-cased.
  6. If `markers`: wrap the whole message (at bundle-formatting level, not per text element) in `[` … `]` so truncation shows as a missing `]` and concatenation shows as `][`.
  7. Return.
- **Data structures**: static maps (52 entries per mode), precompiled protected-span regex (or a small hand-written scanner to avoid regex dependency in release builds — pseudo modes are compiled only with a `pseudo` feature).
- **Complexity / performance notes**: O(n) per literal; applied at format time via `set_transform`, cached by the formatted-string cache if any; dev-only feature.
- **Edge cases (exhaustive list)**:
  1. Empty literal → empty.
  2. Literal only whitespace → unchanged (layout spacing).
  3. Literal containing markup `Click <a data-l10n-name="docs">here</a>` → tag untouched, "Click"/"here" transformed.
  4. Entities `&nbsp;` untouched.
  5. Numbers and punctuation unchanged (`3`, `%`, `:`).
  6. URLs untouched.
  7. Inline `<code>SELECT 1</code>` untouched (SQL never pseudo-localized).
  8. Single-character access key `.accesskey = S` untouched.
  9. Multiline literals → each line transformed, newlines kept.
  10. Combining marks already present (e.g. English source with "café") → transform base letters only if ASCII; keep the rest.
  11. Bidi mode with placeables → placeables are separate elements wrapped by FSI/PDI; overrides inside literals must be closed (PDF) before the end of every literal so they don't leak into placeables.
  12. Strings later used as identifiers or file names (should not come from Fluent at all) → pseudo mode reveals them as bugs.
  13. Plural selection unaffected (transform applies to text, not to selector keys).
  14. Turkish mode applied to messages that are themselves Turkish (when chain is `[tr-XT, en]`, base is English text) — never apply to real `tr` catalogs.
- **Test cases (input → expected)**:
  | # | Mode | Input | Expected |
  |---|---|---|---|
  | 1 | accented, markers | `Save` | `[Şåååvéé]`-like: begins `[`, ends `]`, ≥ 8 visible chars, no ASCII letters remain |
  | 2 | accented | `S` (access key) | `S` |
  | 3 | accented | `Read <a data-l10n-name="d">docs</a>` | tag bytes identical; `Read`/`docs` transformed |
  | 4 | accented | `Visit https://example.org now` | URL identical |
  | 5 | bidi | `Open file` | each word wrapped U+202E…U+202C, letters flipped (`uədO`-style) |
  | 6 | turkish | `Find in table` | contains at least one of `ı İ ş ğ ç ö ü`; length ≥ 1.6× for ≤ 20 chars |
  | 7 | any | `<code>SELECT 1</code>` | unchanged |
  | 8 | accented | `` (empty) | `` |
  | 9 | accented | `3 rows` (literal part ` rows` after placeable) | `3` comes from placeable (unchanged); ` ŕöööŵş`-like |
- **Prior art and evidence**: fluent-pseudo maps, vowel duplication for ~30 % expansion, DOM-aware skipping, markers [Observed: `fluent-pseudo/src/lib.rs:6-131`]; Android/Chromium `en-XA`/`ar-XB` pseudo-locales [Inferred]; IBM/W3C expansion guidance [Inferred, see doc 02 §3.18].
- **Used by**: Hacivat | Karagöz | Perde

### ALG-I18N-04: Catalog validation in CI

- **Problem**: Guarantee that every catalog in `locales/` is syntactically valid, structurally compatible with the English reference, safe to render, and that code and catalogs agree (no undefined or unused IDs) — so that "add a folder, translate, open a PR" can be merged with confidence and without code review by an i18n expert.
- **Inputs / outputs**:
  - Inputs: `locales/` tree; source tree (Rust + TS); `tools/l10n/dynamic-ids.toml`; CLDR plural data (from ICU4X `icu_plurals` at tool build time); configuration (thresholds, budgets).
  - Outputs: exit code (0 ok / 1 errors); human report; machine report `l10n-report.json` (per locale × component: total, translated, missing, obsolete, errors, warnings, completeness); optionally generated `l10n-ids.d.ts` and the English signature table.
- **Algorithm**:
  1. **Discover locales**: list directories in `locales/` (skip names starting with `_`); for each read `locale.toml`; validate: TOML parses; `tag` parses as BCP-47; `tag == folder name` (case-sensitive canonical form); `direction ∈ {ltr, rtl}`; `status` valid; `native_name` non-empty and NFC-normalized. Error otherwise.
  2. **Parse every `.ftl`** with `fluent-syntax` (full AST incl. comments and spans). Any `Junk` entry → error with file:line and the parser's annotation. Files must be UTF-8 without BOM, LF line endings, NFC-normalized text (error on non-NFC since translators' tools sometimes emit NFD Turkish `ş` = `s`+U+0327). Also error on files present in a locale but absent in `en` (unknown component).
  3. **Build English reference signatures** per component: for each message/term in `en`: `vars` (all `$var` referenced anywhere, incl. selectors and nested placeables), `selector_vars` (vars used as selectors and whether numeric-plural-like), `attrs` (set of attribute names), `has_value`, `refs` (message and term references), `markup` (multiset of tag names and `data-l10n-name` values), `max_length` from `# MaxLength: N` comment, `variables_documented` (parse `# Variables:` block; every var must be documented with a type in {String, Number, DateTime, List}).
  4. **English-only checks**: duplicate IDs across *all* components in the same locale (IDs are global) → error; ID regex `^(cm|hv|hc|kg|ps|pw|pe|error)-[a-z0-9]+(-[a-z0-9]+)*$` for messages and `^-[a-z0-9]+(-[a-z0-9]+)*$` for terms → error; undocumented variables → error; message with neither value nor attributes → error; select expressions without default → (parser already errors); English text containing typographic hazards: leading/trailing whitespace not inside `{" "}` → warning; raw U+2066–U+2069 or U+202A–U+202E → error; apostrophe style: English source uses the typographic `’` consistently, ASCII `'` → warning; `{ NUMBER(...) }` options outside the allowed set {minimumFractionDigits, maximumFractionDigits, useGrouping, style(percent|decimal)} → error; `DATETIME` options outside {dateStyle, timeStyle} → error.
  5. **Per-translation checks** (for each locale L ≠ en, each component, each entry):
     1. **Obsolete**: ID not in English → error (or auto-fix in a bot PR).
     2. **Missing**: English ID absent in L → counted (warning, not error).
     3. **Value/attribute shape**: `has_value` must match English (translation may not add a value English lacks; may not drop a value English has → error); attribute set must equal English's (missing attribute → warning, extra attribute → error).
     4. **Variables**: `vars(L) ⊆ vars(en)` else error (unknown variable). For each var in `vars(en)`: if absent from *all* variants of L → warning "dropped variable" unless the var is a plural selector in English (Turkish may legitimately omit `$count` in a `[one]` variant but must use it somewhere unless the message is pure selection — warning only).
     5. **Selectors**: if L selects on a var, the var must exist in English (covered) and be of compatible type: selecting with CLDR category keys (`zero|one|two|few|many|other`) requires the English documented type `Number`. For numeric selectors in L: compute CLDR cardinal categories for L; **warning** if a category in L's set (excluding `other`) is missing as a key and there is no catch-all difference (e.g. Arabic missing `few`) — because the default variant will be used, which may be grammatically wrong; **error** if L uses a category key not in L's CLDR set (e.g. Turkish `[few]`) since it can never match (dead variant).
     6. **References**: every message/term reference resolves within L ∪ en (terms are locale-owned and Fluent does not resolve references across bundles, so a term referenced by L but defined only in English fails at runtime → **error**).
     7. **Term parameters**: for `-term(case: "x")` calls, check term in L has a variant `[x]` or a default → warning if only default.
     8. **Markup parity**: multiset of tags and `data-l10n-name` values equal to English → error otherwise (prevents broken links/injection).
     9. **Forbidden content**: raw bidi controls U+202A–U+202E, U+2066–U+2069 (except in `ar-XB` pseudo, which is generated, not committed), C0 controls except `\n`, U+FEFF, zero-width chars outside allowed contexts (ZWJ/ZWNJ allowed for scripts that need them per `locale.toml` flag), `<script`, `javascript:` → error.
     10. **Untranslated-copy heuristic**: translation byte-identical to English, length > 12, not in an allow-list (brand names, "PostgreSQL", "SQL") → warning.
     11. **Length budget**: if English has `# MaxLength: N`, `graphemes(L_rendered_worst_case) > N` → warning (render worst case by substituting long sample values per documented type).
     12. **Turkish-specific lint (L = tr)**: placeholder immediately followed by `'` + suffix letters (regex `\}\s*'[a-zçğıöşü]+`) → warning "suffix attached to variable; restructure" (doc 02 §3.10.1); ASCII `I` in words that are clearly Turkish lowercase-context? (skip — too noisy).
  6. **Code ↔ catalog checks**:
     1. Scan Rust for `fl!(…, "id"` and our `l10n_dynamic!` patterns; scan TS for typed `t("id"`/`<Localized id="id"`. Undefined IDs in English → error (Rust already fails to compile; TS types fail typecheck — CI double-checks).
     2. Unused English IDs (not referenced and not matched by `dynamic-ids.toml` patterns) → warning; error after a grace period (configurable).
     3. Generate/verify `l10n-ids.d.ts` and signature table; mismatch with committed version → error.
  7. **Render smoke test**: for each locale and each message, render with synthetic args (by documented type; numbers 0, 1, 2, 5, 21, 1.5, 1000000) through the *Rust runtime* (ALG-I18N-02) with Dev mode; any runtime error → error. Optionally the same in JS for parity on a sample.
  8. **Completeness report**: per locale/component completeness = translated / English messages (attributes counted as separate units). For `status = stable` below `min_completeness` → warning (release checklist turns it into an error on release branches).
  9. Emit reports; exit non-zero on any error.
- **Data structures**: `Signature` map per component (`HashMap<String, Signature>`), `Report` struct serializable to JSON; CLDR plural category sets per language (from ICU4X `PluralRules::categories()` at tool runtime).
- **Complexity / performance notes**: Linear in total FTL size × locales; source scan linear in code size. Expect < 2 s for tens of thousands of messages. Run on every PR touching `locales/`, Rust/TS sources, and nightly.
- **Edge cases (exhaustive list)**:
  1. New locale folder with only `locale.toml` → valid; 0 % complete; all missing.
  2. Locale folder name not canonical (`tr_TR`, `TR`) → error with suggested name.
  3. `.ftl` file with Windows CRLF → error (auto-fixable).
  4. BOM present → error (auto-fixable).
  5. NFD text → error (auto-fix: NFC).
  6. Translation adds `[zero]` for Turkish → error (dead variant; tr has only one/other).
  7. Arabic translation lacking `[two]` → warning.
  8. Translation selects on `$count` where English doesn't select → fine if `$count` exists in English.
  9. Translation uses a numeric exact key `[1]` → always allowed.
  10. Comment-only change in English → no translation impact.
  11. English message's variable renamed (`$n` → `$count`) without ID change → all translations now reference unknown `$n` → errors in every locale → policy: variable rename requires ID change (validator message suggests it) or bot rewrites translations mechanically.
  12. Multi-component duplicate ID (`hv-…` defined in both `hacivat-ui.ftl` and `common.ftl`) → error.
  13. Message referencing a message from another component file (cross-file reference) → allowed only into `common.ftl` (bundles load common + component); else error.
  14. Pseudo-locale folders must not be committed (`en-XA` etc.) → error if present.
  15. Overlay directory is not validated by CI (user-local) but by the app's "Language diagnostics" panel using the same library.
  16. Extremely long lines / large files → no special handling; enforce max file size 1 MB per FTL (error).
  17. Dynamic ID families with placeholders (`error-*`) → each family member must exist in English for every code defined in the code registry (`errors.toml`) → error on missing.
  18. A translation PR that also modifies English → validated against the PR's English.
- **Test cases (input → expected)**:
  | # | Setup | Expected |
  |---|---|---|
  | 1 | `tr/hacivat-ui.ftl` contains `hv-x = { $cuont }` while en has `$count` | error unknown variable `$cuont` (tr, hv-x, line N) |
  | 2 | `tr` adds `hv-old = …` not in en | error obsolete |
  | 3 | `tr` missing `hv-y` | warning missing; completeness decremented |
  | 4 | `tr` `hv-z = { $count -> [few] … *[other] … }` | error dead variant `few` for `tr` |
  | 5 | `ar` `hv-z` lacks `[two]`,`[few]`,`[many]` | warnings (3) |
  | 6 | `tr` message contains U+202E | error forbidden bidi control |
  | 7 | en `hv-link = Read <a data-l10n-name="docs">docs</a>`; tr drops the `<a>` | error markup parity |
  | 8 | en message has `$name` but no `# Variables:` doc | error undocumented variable |
  | 9 | en ID `HV-Title` | error naming |
  | 10 | folder `locales/tr_TR/` | error non-canonical tag, suggest `tr-TR` |
  | 11 | Rust code `fl!("hv-missing")` | compile error (fl!) + CI error |
  | 12 | en `hv-unused` never referenced, not dynamic | warning unused |
  | 13 | `tr` `= { $db }'e bağlanıldı` | warning Turkish suffix after placeable |
  | 14 | `tr` file NFD `ş` (U+0073 U+0327) | error NFC |
  | 15 | Clean `tr` with all messages | exit 0; completeness 100 % |
- **Prior art and evidence**: Mozilla `compare-locales` (missing/obsolete/placeable checks; MPL-2.0) and `moz-fluent-linter` [Inferred: tool names/purpose]; Weblate Fluent checks (syntax, parts, references, inner HTML) [Observed: docs.weblate.org formats/fluent]; `fl!` compile-time checks [Observed: `i18n-embed-fl/src/lib.rs:542,585`]; CLDR plural sets [Observed: CLDR 49 plurals.json for tr/az].
- **Used by**: Hacivat | Karagöz | Perde

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 Decisions (recommended)

| ID | Decision | Applies to |
|---|---|---|
| D-I18N-01 | Fluent (FTL) is the only message format; English `en` is the reference catalog. | All |
| D-I18N-02 | Shared top-level `locales/<tag>/<component>.ftl` + `locale.toml`; directory-driven discovery; no per-language code. | All |
| D-I18N-03 | Rust runtime = fluent-bundle (+ our thin fallback layer or fluent-fallback) + i18n-embed loader + `fl!` checks + ICU4X formatter/functions. | Hacivat, Karagöz, Perde server |
| D-I18N-04 | Web runtime = `@fluent/bundle` (+ `@fluent/react` if React) + generated TS types; Rust↔JS parity tests. | Tauri frontend, Perde web |
| D-I18N-05 | Three settings: UI language, regional format, grid data display mode (+ time zone). | Hacivat (all three), Perde (first two) |
| D-I18N-06 | Machine outputs (logs, JSON, audit, MCP, error codes) are English and locale-independent; human outputs localized; bidi marks stripped in machine sinks. | All |
| D-I18N-07 | Per-message fallback; release builds fall back on translation-structural errors. | All |
| D-I18N-08 | Weblate (hosted Libre if accepted, else self-hosted) with one component per FTL file; PR-based flow; CI validator gates merges. | Process |
| D-I18N-09 | Pseudo-locales `en-XA`, `ar-XB`, `tr-XT` in dev builds; screenshot tests in en/tr/en-XA/ar-XB. | Hacivat, Perde web |
| D-I18N-10 | Turkish style guide + glossary maintained in-repo (`locales/tr/STYLE.md` is documentation, allowed) and mirrored to Weblate glossary. | Process |

### 5.2 Crate/API sketch for `hacivat-l10n` (our own design)

```text
pub struct L10n { inner: ArcSwap<Runtime> }            // cheap clone, atomic swap on switch
pub struct Runtime { generation: u64, ui_chain: Vec<LangId>, format_locale: Locale,
                     dir: Direction, ui: Bundles, machine: Bundles, fmt: Formatters }

impl L10n {
  pub fn init(cfg: L10nConfig) -> (L10n, Diagnostics)  // runs ALG-I18N-01
  pub fn set_preferences(&self, prefs: Prefs)          // re-runs ALG-I18N-01, swaps, emits event
  pub fn subscribe(&self) -> Receiver<LocaleChanged>
  pub fn msg(&self, id: &'static str, args: Args) -> String           // ALG-I18N-02, Ui sink
  pub fn attr(&self, id: &'static str, attr: &'static str, args: Args) -> String
  pub fn msg_machine(&self, id: &'static str, args: Args) -> String   // no isolation
  pub fn number(&self, d: &Decimal, opts: NumberOpts) -> String      // ICU4X
  pub fn datetime(&self, t: &ZonedDateTime, opts: DateOpts) -> String
  pub fn list(&self, items: &[String], kind: And|Or) -> String
  pub fn collator(&self) -> &Collator                                  // UI sorting (doc 02)
  pub fn ui_search_key(&self, s: &str) -> SearchKey                    // ALG-I18N-06 (doc 02)
  pub fn available_languages(&self) -> &[LanguageInfo]                // from generated table
}
macro_rules! t { ($id:literal $(, $k:ident = $v:expr)*) => { /* wraps i18n_embed_fl::fl! for compile-time checks, then calls L10n::msg */ } }
```

### 5.3 Per-product action items

- **Hacivat**: implement `hacivat-l10n` first (before UI stack choice is final); route *every* UI string through it; add `--pseudo` dev flag; language & region pickers in Settings (native names from `locale.toml`); language diagnostics panel (shows chain, format locale, missing messages for current screen, overlay load errors).
- **Karagöz**: same crate with `karagoz-cli.ftl` + `common.ftl` only; `--lang`; English logs with `message_id`; `--json` outputs stable English keys and `message`, `message_id`, `args`.
- **Perde server**: concurrent bundles (`fluent_bundle::concurrent`) built once per locale at startup; per-request chain cached; e-mail rendering uses recipient chain; API errors carry `code`, `message`, `message_id`, `args`, optional `localized_message`.
- **Perde web**: Vite glob import of `perde-web.ftl` + `common.ftl` per locale; typed `t`; `<html lang dir>` updates on switch; `Intl` formatting via fluent.js; parity tests.
- **Repo/process**: `cargo xtask l10n check|new <tag>|report|types`; Weblate project; CONTRIBUTING section "Adding a language"; string-freeze calendar.

---

## 6. i18n considerations

This entire document is about i18n; the cross-cutting rules it imposes on *other* research areas:

1. Every design doc that introduces user-visible text must name its component catalog and message-ID prefix.
2. Any feature that emits text into machine contexts (logs, audit, MCP, JSON, clipboard of data, file names) must use the machine sink or no Fluent at all.
3. Any case conversion, sorting, searching or number/date parsing in any component must use the rules in document 02 (ALG-I18N-05 … 07); plain `to_lowercase()` on identifiers is a review-blocking defect.
4. UI layout must tolerate +100 % expansion for short labels and RTL mirroring (doc 02 §3.16–3.18).
5. MCP: tool metadata English; approval UI localized (see `mcp/02-…`).

---

## 7. Risks, pitfalls, open questions

| # | Item | Type | Mitigation / next step |
|---|---|---|---|
| 1 | fluent-rs number formatting not locale-aware; easy to forget the custom formatter in one binary | Pitfall | Single constructor in `hacivat-l10n`; parity tests; test case "1.234,5 satır". |
| 2 | Browser `Intl` CLDR version ≠ ICU4X CLDR version → small formatting diffs between Tauri frontend and Rust | Risk | Format data-derived values in Rust; parity test tolerances; document. |
| 3 | Fluent ecosystem stagnation vs MF2 | Risk | Keep English messages simple; track `@messageformat/fluent`; re-evaluate when ICU4X ships MF2 [Unknown: timeline]. |
| 4 | Weblate Fluent editing exposes raw selector syntax to translators | Risk | Translator guide with copy-paste templates; CI catches structural mistakes; Pontoon fallback option. |
| 5 | Weblate Libre approval not guaranteed (Perde hosted is commercial) | Unknown | Apply early; budget self-hosting (Docker) as plan B. |
| 6 | Crowdin OSS eligibility conflicts with commercial hosted Perde | Risk | Don't depend on Crowdin. |
| 7 | i18n-embed `fl!` only checks the fallback language; macro compile cost with large catalogs | Pitfall | CI validator covers translations; measure compile time; split catalogs per crate if needed. |
| 8 | ICU4X `icu_experimental` (relative time, display names) API churn | Risk | Pin versions; wrap behind our API; store native names in `locale.toml`. |
| 9 | Binary size from ICU4X compiled data (all locales) | Unknown | Measure; for Karagöz consider blob data with fewer locales — but "no code change per language" then requires the blob build to include all catalog locales automatically (generate locale list from `locales/`). |
| 10 | UI toolkit without bidi/shaping (if native toolkit chosen) blocks future RTL | Risk | Weight RTL capability in the UI-stack decision (doc 02 §3.16). |
| 11 | Bidi isolation marks leaking into copied text, file names, logs | Pitfall | Machine sink; tests that grep outputs for U+2068/U+2069. |
| 12 | Dynamic message IDs defeating compile-time checks | Pitfall | `l10n_dynamic!` allow-list + registry-driven CI checks. |
| 13 | Translators changing meaning of terms (brand names) | Pitfall | Brand terms locked (Weblate "read-only" flag on `-brand-*` [Inferred: Weblate supports read-only strings via flags; Fluent format page says read-only "not supported" — Unknown, verify]). |
| 14 | Perde e-mail rendered in wrong language for queued jobs after profile change | Pitfall | Store locale at enqueue; explicit tests. |
| 15 | `Intl.MessageFormat` stage status conflicting reports | Unknown | Irrelevant for v1; revisit annually. |
| 16 | Whether Slint (if chosen) can bypass `@tr` cleanly | Unknown | Prototype during stack evaluation. |
