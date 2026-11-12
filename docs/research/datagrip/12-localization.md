# 12 — Localization in DataGrip / IntelliJ Platform, and Lessons for Hacivat

> Part of the Hacivat research library. Conventions: `../README.md`. Related: 11 (settings catalog — searchable index by key, ALG-SET-03), 06/07/08/09/10 (where strings live per feature), 13 (behavioral corpus — add family L10N from §4 test cases), `../i18n/01-localization-architecture.md` (Fluent/ICU4X architecture, ALG-I18N-01..04) and `../i18n/02-turkish-and-db-specific-pitfalls.md` (Turkish casing, number parsing, server messages, ALG-I18N-05..08). This document focuses on **what DataGrip/IntelliJ actually do** (mechanism, data, coverage, bugs) and which parts Hacivat should copy or improve; the generic Rust/web architecture lives in `../i18n/`.

---

## 1. Scope and sources

**Questions answered.**
1. Where does DataGrip 2024.3.5 (IntelliJ Platform 243, build 243.24978.79) keep every user-visible string, and how are strings referenced from code, plugin descriptors and the settings-search index?
2. How does a language pack plug in at runtime: discovery, locale selection, persistence, restart, the exact bundle lookup chain, per-key fallback to English, and localization of non-`.properties` resources?
3. How are placeholders, plurals (`ChoiceFormat`), ordinals, mnemonics, ellipses, HTML and apostrophes handled, and what does that cost translators?
4. How complete are the bundled ja/ko/zh packs for the parts of the IDE that DataGrip ships (measured, per bundle), and what is intentionally left in English?
5. Help, tips, inspection/intention descriptions, settings search, region, date/number formats, RTL — what is localized and what is not?
6. Which locale bugs did DataGrip have (public tracker), and what should Hacivat design differently, given "English-first, EN + TR in v1, adding a language must need no code changes"?

**Sources.**

| Source | What was used | License |
|---|---|---|
| DataGrip 2024.3.5 app (`/Applications/DataGrip.app/Contents`, build 243.24978.79) | `plugins/localization-{ja,ko,zh}/lib/localization-*.jar` (file listings, `META-INF/plugin.xml`, `messages/*.properties`, directory structure); English bundles extracted from `lib/app.jar`, `lib/product.jar`, `plugins/DatabaseTools/lib/database-plugin.jar`, `plugins/grid-core-impl/lib/grid-core-impl.jar` and others (146 English `messages/*.properties`, index at `.refs/scratch-extracts/bundles/index.txt`); `localization/<lang>/tips/` folders inside `database-plugin.jar` and `datagrip-impl.jar`; `*searchableOptions.json` in `lib/product.jar` and plugin jars; `plugins/DataGripHelp/lib/*.jar` (descriptor, image names) | Proprietary — structure and statistics only, no text reuse beyond short labels |
| IntelliJ Community, branch `243` (fetched via GitHub API) | `platform/util/src/com/intellij/BundleBase.kt`, `platform/util/src/com/intellij/AbstractBundle.kt`, `platform/core-api/src/com/intellij/DynamicBundle.java`, `platform/core-api/src/com/intellij/l10n/LocalizationUtil.kt`, `.../l10n/LocalizationOrder.kt`, `platform/util/ui/src/com/intellij/openapi/util/text/TextWithMnemonic.java`, `platform/util/src/com/intellij/util/DefaultBundleService.java`, `platform/lang-impl/src/com/intellij/lang/LocalizationStateServiceImpl.kt`, `platform/platform-impl/src/com/intellij/ide/LanguageBundleListener.kt`, `.../ide/plugins/LanguagePluginDetectionStartupActivity.kt`, `.../ide/plugins/LocalizationPluginListener.kt`, `plugins/built-in-help/src/com/jetbrains/builtInHelp/BuiltInHelpManager.kt` | Apache-2.0 (mechanisms described in our words; no code copied) |
| Bundled offline help (DataGripHelp, Oct 2024) | `language-and-region`, `date-formats-settings`, `settings-tools-database-data-views`, `system-settings` | JetBrains docs |
| jetbrains.com online help | Existence check of `ja-jp/help/datagrip/…`, `zh-cn/…`, `ko-kr/…` (HTTP 200; Japanese page title) | JetBrains docs |
| Public YouTrack (project DBE) | DBE-7037, DBE-9900, DBE-9986, DBE-23788, DBE-2159, DBE-20272, DBE-17424, DBE-12237, DBE-25126 | public issue text (summarized) |
| Our measurement scripts (scratchpad, not shipped) | key parity, missing/stale keys, identical values, placeholder parity and order, mnemonic/ellipsis/HTML preservation, `ChoiceFormat` usage, searchable-option hit classification | ours |

All numbers below were computed by us over the extracted files. [Observed] means reproduced from those files or sources; [Inferred] states its basis.

---

## 2. Executive summary

1. **All UI text lives in Java `.properties` resource bundles** keyed by stable dotted IDs. DataGrip ships **146 English bundles with 26,010 keys**; the DB layer alone is DatabaseBundle 2,942 + SqlBundle 785 + DataGridBundle 393 + DatabaseDynamicBundle 495 keys. [Observed]
2. **English is embedded in each feature plugin; translations are separate language-pack plugins** (`com.intellij.ja`, `com.intellij.ko`, `com.intellij.zh`), each a single jar with the *same resource paths* (`messages/DatabaseBundle.properties`, `inspectionDescriptions/X.html`, `intentionDescriptions/X/description.html`). One pack serves all JetBrains IDEs (1,066 bundles, 6,513 files, 10–15 MB). [Observed]
3. **A pack is declared by one extension and no code**: `languageBundle locale="ja" displayName="Japanese 日本語"`. Chinese uses a *regioned* tag (`zh-CN`). Packs are pinned to exactly one IDE build (`since-build = until-build`). [Observed]
4. **Locale selection** is an app-level persisted state (`selectedLocale`, `lastSelectedLocale`) with a "restart required" flag; until restart, the old locale keeps being used. If the selected locale has no installed pack, the platform silently uses English, and at startup resets the setting to `en`. An external tool (Toolbox) can preset the locale through an early-access registry key. [Observed: `LocalizationStateServiceImpl.kt`, `LanguageBundleListener.kt`]
5. **Pack matching is exact** on the locale tag (`ja` ≠ `ja-JP`; `zh` ≠ `zh-CN`); a looser language-only match exists only as commented-out code. [Observed: `LocalizationUtil.findLanguageBundle`]
6. **Lookup chain** for bundle `p` and locale `xx-YY`: `localization/xx/YY/p` → `p_xx_YY` → `localization/xx/p` → `p_xx` → `p` inside the language-pack jar → English `p` in the feature plugin. The platform re-links `ResourceBundle` parent pointers so any missing key falls through to English. [Observed: `DynamicBundle.java`, `LocalizationOrder.kt`]
7. **Formatting**: mnemonic conversion first, then `java.text.MessageFormat` only if arguments are passed **and** the value contains `{`; ordinal support patched in; malformed patterns render as `!invalid format: …!`; missing keys render as `!key!`. English resolves with `Locale.ROOT`, so numbers in English messages never pick up the OS locale. [Observed: `BundleBase.kt`, `DynamicBundle.getResolveLocale`]
8. **Plurals use `ChoiceFormat`** (`{0,choice,0#rows|1#row|2#rows}`) — DatabaseBundle 17 and DataGridBundle 7 values; the same syntax is abused as a boolean selector (`0#View|1#Edit`) and as conditional sentence fragments. CJK packs repeat one noun in every branch. ChoiceFormat cannot express CLDR one/few/many/other. [Observed]
9. **Mnemonics are inline**: `&` (most bundles) or `_` (ActionsBundle), `&&` = macOS-only mnemonic, `\&` = literal ampersand; converted to an internal ESC (0x1B) marker. CJK translations append `(&B)` / `(_T)` **before** the ellipsis and keep the English letter — 64/64 DatabaseBundle and 19/19 DataGridBundle mnemonics preserved in all three packs. [Observed]
10. **Coverage is near-total at key level**: each pack has 25,617 of 26,010 DataGrip keys (98.5%); 326 of the 393 missing keys are in intentionally untranslated `*Deprecated*` bundles, 11 in an internal statistics bundle, and the remaining 56 are the newest strings (trial surveys, AI onboarding) — i.e. ~0.2% lag. DB bundles: 100% in ja/ko/zh. [Observed]
11. **Placeholder parity is perfect** in DB bundles (0 mismatches in all packs); translators reorder placeholders in 14–22 DatabaseBundle messages per language; ellipses kept in 103/104; HTML wrappers kept 21/21. [Observed]
12. **Non-string resources localize by path**: all 68 DB inspection descriptions and all 35 DB intention descriptions are translated in each pack (intention *before/after* SQL examples are not); DB tips live inside the DB plugin under `localization/ja/tips`, `localization/ko/tips`, `localization/zh/CN/tips` (10 each; DataGrip product tips 103 each). [Observed]
13. **Settings search is language-independent**: 6,128 of 6,177 searchable-option hits (99.2%) are `|b|bundle|k|key|` tokens resolved at query time; the 49 raw hits are product/language names and legacy labels. The same token shape is what the bundle layer emits in its "keys only" debug policy. [Observed]
14. **Help**: the offline help plugin is English-only (with an English Lucene index); localized help exists only online (`jetbrains.com/ja-jp|zh-cn|ko-kr/help/datagrip/…`). [Observed]
15. **Region ≠ language**: the Region setting (8 values) changes service endpoints, not text; date formats follow the OS unless overridden; data-editor number/date formats are separate DB settings (decimal separator default `.`). The JVM locale leaks into JDBC drivers (Oracle NLS error, Thai Buddhist dates, Turkish dotless-ı in ClickHouse setting names). [Observed: help, bundle, YouTrack]
16. **RTL**: no RTL UI language exists; the editor only offers *Bidi Text Base Direction* (content-based / LTR / RTL) for mixed text. [Observed: ActionsBundle; Inferred: no mirroring]
17. **Turkish bugs were real**: completion inserting `"exısts"` (DBE-7037), routine source shown as `functıon` (DBE-9986), ClickHouse `sessıon_ıd` (DBE-9900) — all from locale-sensitive case mapping of technical identifiers. [Observed: YouTrack]
18. **Recommendation**: Fluent messages with named args and CLDR plurals, per-locale data packs discovered at runtime (zero code), RFC 4647 negotiation (not exact tag match), hot-swappable language, accesskeys as attributes, region/format settings decoupled from UI language **and** from DB session locale, pseudo-localization and key/coverage tooling in CI (ALG-L10N-01..11 below; generic parts in ALG-I18N-01..08).

---

## 3. Detailed findings

### 3.1 Where strings live (inventory)

| Bundle / resource | Location (English) | Keys / files | Content |
|---|---|---|---|
| `messages/DatabaseBundle.properties` | `database-plugin.jar` | 2,942 | Actions (`action.*`, ~412), object property titles (`property.*`, ~298), introspection progress, notifications, dialogs, settings labels, script generator (`scriptGen.*`), transactions, plan view, Mongo translation, error explanations (e.g. Oracle NLS mismatch) |
| `messages/SqlBundle.properties` | `database-openapi.jar` | 785 | SQL language: inspection names/messages, intentions, quick fixes, formatter labels, completion settings, color attributes |
| `messages/DataGridBundle.properties` | `grid-core-impl.jar` | 393 | Grid actions, CSV formats, pagination, aggregators, value editor, position widget |
| `messages/GridCsvBundle.properties` | `grid-core-impl.jar` | 8 | CSV engine messages |
| `messages/DatabaseDynamicBundle.properties` | `database-plugin.jar` | 495 (475 `ObjectKind.*`) | Object-kind vocabulary looked up by computed key (§3.11) |
| `messages/DatabaseProductivityFeatures.properties` | `database-plugin.jar` | 11 | Feature-usage statistics names — **not translated** in any pack |
| `messages/MongoJSBundle.properties` | `database-plugin.jar` | 4 | MongoDB JS inspections |
| `messages/DataGripBundle.properties` | `datagrip-impl.jar` | 26 | Product-level strings |
| `messages/ChartBundle`, `DiagramBundle`, `LLMInstallerBundle` | `charts`, `uml`, AI plugins | 151 / 372 / 153 | Result charts, diagrams, AI onboarding |
| `inspectionDescriptions/*.html` | `database-plugin.jar` | 68 (+ `metaInformation.json`) | Long inspection descriptions with SQL examples |
| `intentionDescriptions/<Name>/description.html` + `before/after.sql.template` | `database-plugin.jar` | 35 | Intention descriptions and examples |
| `tips/*.html` + `localization/<lang>/[<REGION>/]tips/*.html` | `database-plugin.jar` (10), `datagrip-impl.jar` (103) | per language | Tip of the Day |
| Platform bundles (`ActionsBundle` 2,794, `IdeBundle` 2,791, `ApplicationBundle` 888, `OptionsBundle` 545, `LanguageAndRegionBundle` 16, …) | `lib/app.jar`, `lib/product.jar` | 22k+ | Shared platform UI |
| `*searchableOptions.json` | `lib/product.jar`, plugin jars | 6,177 hits | Settings search index (key tokens, §3.14) |

[Observed: file listings and our key counts]

**Key naming conventions.**
- `action.<ActionId>.text` / `.description` and `group.<GroupId>.text` — action presentation is resolved by convention from the plugin's `<resource-bundle>`; the action class never mentions the string. [Observed: keys like `action.Console.Jdbc.Execute.text`]
- Feature prefixes: `settings.*`, `dialog.*`, `notification.*`, `progress.*`, `property.<Name>.title`, `inspection.name.*`, `intention.*`, `scriptGen.*`. [Observed]
- Argument hints embedded in key names: `unable.to.resolve.symbol.0` takes `{0}`; `…0.1` takes two. [Observed] Produced by the platform's "I18nize hard-coded string" refactoring [Inferred from naming regularity].
- Same key in different bundles with different meaning (`inspection.name.resolve` = "Unresolved reference" in SqlBundle vs "Resolution problems" elsewhere) — legal because lookup is always (bundle, key). [Observed]
- Encoding asymmetry: English bundles are ASCII with `\uXXXX` escapes (e.g. `…` for "…", 108 occurrences in DatabaseBundle); translated bundles are raw UTF-8 and escape `:`/`=`/`#` (`ページサイズの制限\:`) — evidence that packs are emitted by a tool, not hand-edited. [Observed]

### 3.2 How strings are referenced

- **From code:** each module defines a bundle object (`DatabaseBundle`, `SqlBundle`, `DataGridBundle` — names in the class inventory) built on `DynamicBundle`; call sites use `message("key", args…)` with the key parameter annotated `@PropertyKey(resourceBundle = …)`, so the IDE validates keys statically; values are typed `@Nls` (user-visible) vs `@NonNls` (technical), which powers a "hard-coded string literal" inspection. [Inferred: IntelliJ conventions visible in `AbstractBundle.kt` signatures; class inventory]
- **Lazy messages:** `getLazyMessage` returns a supplier evaluated at display time; `getPartialMessage` applies some arguments now and renumbers the rest for later (`{2}` becomes `{0}`), which is how a message can be pre-filled and finished by another component. [Observed: `AbstractBundle.kt`, `BundleBase.partialMessage`]
- **From plugin XML (declarative):** inspections (`bundle` + `key`, `groupBundle` + `groupKey`), configurables (`key=`), advanced settings, notification groups, tool windows, actions by convention. [Observed: `SqlPlugin.xml`, `DatabaseRegistryKeys.xml`]
- **From the searchable index:** `|b|<bundle>|k|<key>|` tokens (§3.14, doc 11). [Observed]
- **Not localized by design:** registry descriptions (`description="…"` in `DatabaseRegistryKeys.xml`), plugin names/descriptions of feature plugins (open request DBE-12237 "Localize the plugin name"), log messages, and a few deliberate English strings — e.g. the "Switch back to English" notification action is hard-coded with a suppression so a user stuck in an unreadable language can always find it. [Observed: `LanguagePluginDetectionStartupActivity.kt`, YouTrack]
- **Leaks:** DBE-17424 ("Not localized: Code completion settings") shows the typical failure — labels built outside the bundle system stay English inside an otherwise translated page. [Observed: YouTrack]

### 3.3 Language packs as plugins

**Descriptor** (all three identical in shape) [Observed: `META-INF/plugin.xml` of each pack]:

| Field | ja | ko | zh |
|---|---|---|---|
| Plugin id | `com.intellij.ja` | `com.intellij.ko` | `com.intellij.zh` |
| `languageBundle locale` | `ja` | `ko` | **`zh-CN`** |
| `displayName` | Japanese 日本語 | Korean 한국어 | Chinese (Simplified) 简体中文 |
| Version / since = until | 243.24978.79 | same | same |
| `allow-bundled-update` | true | true | true |
| Category | IDE Localization | same | same |
| Dependencies | `com.intellij.modules.platform` only | same | same |
| Jar size / files | 15 MB / 6,513 | 14 MB / 6,513 | 12 MB / 6,513 |

**Contents** (same file set in every pack) [Observed: listings]:

| Resource type | Files | Notes |
|---|---|---|
| `messages/*.properties` | 1,066 (1,045 distinct base names in our index) | covers every JetBrains IDE (e.g. `RiderDatabaseBundle`, `JupyterSqlBundle`), not only DataGrip |
| `inspectionDescriptions/*.html` | 3,478 | all products |
| `intentionDescriptions/*/description.html` | 1,126 | descriptions only — the `before/after` example templates are not in the pack and therefore come from the English feature plugin |
| `postfixTemplates/*/description.html` | 321 | descriptions only |
| `fileTemplates/**` | ~513 | mostly `.html` descriptions of templates (template bodies are code) |
| `__index__` (jar root) | 1 | class-loader resource index for fast lookup [Inferred from name and the platform's jar-index mechanism] |

Implications [Inferred]: (a) one pack per language for all products keeps translation memory consistent across IDEs; unused bundles are harmless because lookup is by path. (b) Pinning to one build means the pack is rebuilt and re-shipped with every IDE build; `allow-bundled-update` allows a Marketplace fix without an IDE update. (c) Tips are *not* in the packs — they are shipped inside the feature plugins under `localization/...` folders, so the owning team ships English and translations together.

### 3.4 Locale selection, persistence and restart

[Observed: `LocalizationStateServiceImpl.kt`, `LocalizationUtil.kt`, `LanguageBundleListener.kt`, `LanguagePluginDetectionStartupActivity.kt`, `LocalizationPluginListener.kt`, help `language-and-region`]

- **UI:** Settings › Appearance & Behavior › System Settings › **Language and Region** — a *Language* list (English first, then installed packs sorted by display name) with **Get More Languages…** (Marketplace community packs), and a *Region* list (§3.16). Labels come from `LanguageAndRegionBundle` (16 keys, translated in all packs).
- **State:** an application-level persistent component stored in the general IDE settings file holds `selectedLocale` and `lastSelectedLocale` (default `en`). Statistics report the value from a fixed whitelist of ~55 language tags (incl. `tr`, `zh-CN`, `other`) — i.e. JetBrains measures demand for languages it does not ship.
- **Change:** setting a new locale stores the previous value as "last selected" (unless a restart is already pending), sets `restartRequired = (selected ≠ lastSelected)`, publishes a `localeChanged` event, and offers a restart. `getLocale()` returns the **last** selected locale while a restart is pending, so the running UI never mixes languages.
- **Startup:** an app-lifecycle listener (1) applies a locale preset via an early-access registry key `i18n.locale` (written by external tools such as Toolbox, then cleared), (2) resets the setting to `en` if no loaded plugin is an active localization plugin for it, (3) clears all bundle caches, (4) resets date-time formats. On shutdown it re-reads the registry file so an external change made while the IDE ran is not lost.
- **Fallback:** if the selected language is not English and no `languageBundle` extension matches it exactly, `getLocale()` returns English.
- **OS detection:** on first project open, if the UI is English and the OS default locale has a pack (`ja`, `ko`, or `zh` *only when the country is CN*), a suggestion notification offers "Switch to <language> and restart" plus "Don't ask again". After switching, a follow-up notification offers to switch back to English (that action label is intentionally hard-coded English).
- **Uninstall:** unloading the active pack resets the locale to English and shows a restart dialog.
- **Restart is required** in IntelliJ because Swing components cache text, action presentations are computed once, and bundle caches are static (`DynamicBundle.clearCache()` only at startup). [Inferred from the above]

### 3.5 Runtime bundle resolution (DynamicBundle)

[Observed: `DynamicBundle.java`, `LocalizationUtil.kt`, `LocalizationOrder.kt`, `AbstractBundle.kt`, `DefaultBundleService.java`]

1. **Resolve locale:** `getLocale()` as in §3.4; English is mapped to `Locale.ROOT` for resolution so that the base `name.properties` is chosen (no `_en` files exist).
2. **Find the pack class loader:** for a non-English locale, `findLanguageBundle(locale)` returns the matching extension; its plugin class loader is the place to look for the pack copy of the bundle.
3. **Compute candidate paths** for bundle path `p` (dots canonicalized to slashes) and locale `xx-YY`:
   1. `localization/xx/YY/p` (folder, region)
   2. `p_xx_YY` (suffix, region)
   3. `localization/xx/p` (folder, language)
   4. `p_xx` (suffix, language)
   5. plain `p`
   For a locale without region the list has three entries (folder-language, suffix-language, plain).
4. **Load and rank:** the pack copy (loaded from the pack class loader with `Locale.ROOT`) is ranked `DEFAULT_PLUGIN`; the standard `ResourceBundle` lookup in the feature plugin's own class loader yields suffix variants and the plain base; folder variants are loaded explicitly. Each is mapped to an enum rank: `FOLDER_REGION < SUFFIX_REGION < FOLDER_LANGUAGE < SUFFIX_LANGUAGE < DEFAULT_PLUGIN < DEFAULT_PLATFORM`.
5. **Re-link parents:** the platform rewrites each bundle's `parent` pointer (via a method handle to the private setter) so that the chain is exactly that order; the most specific bundle is returned and cached. A `getString` miss walks the chain, ending in English.
6. **Caches:** per base class loader → (path → bundle), with weak keys and soft values (memory pressure can drop them; they are rebuilt on demand). A *separate* cache serves "default bundle" mode.
7. **English on demand:** `DefaultBundleService.compute { … }` sets a thread-local flag so that everything computed inside resolves English bundles from the separate cache — used where the platform needs the canonical English text while the UI is localized (e.g. default action texts). [Observed: class javadoc]
8. **Fallback when `.properties` cannot be read:** a slower class-based loading path is tried and logged. [Observed: `AbstractBundle.resolveResourceBundleWithFallback`]

Quirk: the rank mapping uses the position of the found path inside the candidate list; when the list has three entries, index 2 (the plain path) is ranked `DEFAULT_PLATFORM`, and in the five-entry case the plain path at index 4 collides numerically with `DEFAULT_PLUGIN` and is explicitly remapped to `DEFAULT_PLATFORM`. A pack copy therefore always beats the feature plugin's English base, but a `localization/ja/` folder inside the feature plugin beats the pack. [Observed: `LocalizationOrder.getLocalizationOrder`] This lets a feature team **override** a pack translation from inside its own plugin — the mechanism used for tips.

### 3.6 Non-`.properties` resources

- `LocalizationUtil.getResourceAsStream(loader, path)` probes the same four localized paths in the feature plugin, then the **unmodified** path in the pack class loader, then the plain path. [Observed]
- Inspection descriptions: the pack contains `inspectionDescriptions/<ShortName>.html` at the English path → found in step "pack". All 68 DB inspection descriptions exist in each pack. [Observed]
- Intention descriptions: same mechanism for `description.html` (35/35 DB intentions in each pack); `before.sql.template` / `after.sql.template` are absent from packs, so examples remain in English SQL — correct, since SQL is not translated. [Observed]
- Tips: shipped inside feature plugins at `localization/ja/tips/X.html`, `localization/ko/tips/X.html` and **`localization/zh/CN/tips/X.html`** (region folder because the pack locale is `zh-CN`). DB plugin: 10 per language; DataGrip product plugin: 103 per language. [Observed]

### 3.7 Formatting pipeline

[Observed: `BundleBase.kt` `postprocessValue`, `messageOrDefault…`, `useDefaultValue`, `appendLocalizationSuffix`]

Order of operations for `message(bundle, key, args)`:
1. `getString(key)` through the parent chain; on miss → default value if supplied, else `!key!` (and an error log when assertions are enabled in tests).
2. **Mnemonic conversion** (§3.8) — always, even without arguments.
3. If no arguments **or** no `{` in the value → return as is. Consequence: in argument-less values an apostrophe is literal (`Don't`), but in values formatted with arguments it must be doubled (`Unable to resolve symbol ''{0}''`). SqlBundle has 39 and DatabaseBundle 52 values with doubled apostrophes; all were preserved by all packs. [Observed: our counts]
4. Else build a `MessageFormat` with the **bundle's locale** (ROOT for English), patch in an ordinal sub-format (`{0,number,ordinal}` → "1st"), format; on `IllegalArgumentException` return `!invalid format: `…`!` rather than throwing.
5. Debug decoration (§3.19).

Observations on usage:
- Number sub-formats are rare: DatabaseBundle uses `{0,number,#}` twice — the `#` pattern exists to suppress grouping separators (ports, ids), i.e. to stay locale-invariant. Dates/durations/sizes are formatted in code before being passed in. [Observed; Inferred for code]
- HTML: 21 DatabaseBundle values are full `<html>` documents, 3 in SqlBundle; packs keep the markup. Debug markers are inserted *before* `</body></html>` / `</html>` so they remain visible. [Observed]
- Ellipsis "…" marks "opens a dialog": 104 DatabaseBundle, 23 DataGridBundle, 8 SqlBundle values end with it; packs preserve it in 103/104, 23/23, 8/8 (the one exception is a Redis port-forwarding error message, ja and ko). [Observed]

### 3.8 Mnemonics in depth

**Syntaxes** [Observed: `BundleBase.replaceMnemonicAmpersand`, `TextWithMnemonic.parse`, bundle samples]:

| Marker | Meaning |
|---|---|
| `&X` | X is the mnemonic (most bundles, e.g. `Open results in new ta&b`) |
| `_X` | X is the mnemonic (ActionsBundle style, e.g. `Insert Live _Template…`) |
| `&&X` | mnemonic on macOS only; on other OSes both ampersands vanish and no mnemonic is set from this marker |
| `\&` | literal `&` |
| `__`, `&&` before the real mnemonic in `_`-style text | escaped literal |
| ESC (0x1B) | internal marker after conversion; text containing it is not converted again |

**Conversion rules (`&` family)**: skip if no `&` or already converted; scan left to right; `\&` emits `&`; `&&` sets the mnemonic only on macOS; a single `&` sets the mnemonic unless we are on macOS **and** the text also contains `&&` (then the macOS-specific one wins); only the **first** mnemonic is honored.

**`TextWithMnemonic` model**: text + mnemonic index + optional *suffix*. A trailing `(X)` (optionally preceded by spaces, optionally before an ellipsis) is split off as the suffix, so CJK labels like `新規タブで開く(&B)` become text `新規タブで開く` + suffix `(B)`; the suffix can be shown or hidden independently (a forced drop removes a `(X)` / `(_X)` group). When a mnemonic letter is requested that does not occur in the text, the class synthesizes `(X)` and places it before any trailing `...`/`…`. Parsing rejects text with two ESC markers or a trailing marker.

**Translator practice** [Observed: our counts]:
- DatabaseBundle 64/64, DataGridBundle 19/19, SqlBundle 1/1 mnemonics preserved in ja, ko and zh — always as a `(&X)` suffix, never inside a CJK word.
- The **English letter is kept** (`(&B)` for "ta&b"), preserving muscle memory across languages.
- ActionsBundle: 194 English `_X` action texts; each pack has ~332 `(_X)` suffixes (more because packs cover other products' actions too); the suffix is placed before the ellipsis (`ライブテンプレートの挿入(_T)…`).
- In total the ja pack carries 2,893 `(&X)` suffixes — a large, purely mechanical burden on translators that a format with access-key *attributes* avoids.

### 3.9 Plurals and `ChoiceFormat`

[Observed: grep over bundles]
- **Usage:** 17 DatabaseBundle and 7 DataGridBundle values use `{n,choice,…}`; SqlBundle none.
- **Three patterns:** (1) English plural: `{0,choice,0#cells|1#cell|2#cells}`; (2) **boolean selector**: `{0,choice,0#View|1#Edit}` (the caller passes 0/1); (3) **conditional fragments**: a single export-result message nests five choice clauses to include/omit parenthesized parts and pick "rows"/"row" — effectively a mini template language inside a translation unit.
- **CJK packs** keep the same thresholds and repeat the same noun in all branches (`0\#行|1\#行|2\#行`), escaping `#` as `\#`; they still must replicate the structure of the conditional-fragment messages, reordering fragments inside choice clauses.
- **Limits:** thresholds are numeric ranges, so languages whose plural category depends on the last digits (Russian `1/21/31` one, `2–4/22–24` few, …), Arabic (six categories) or Welsh cannot be expressed. Turkish is fine (no plural after numerals), but a translator copying the English three-branch structure is nudged to pluralize wrongly ("3 tablolar"). Boolean/conditional uses hide grammar from the translator.

### 3.10 Ordinals, dates and numbers in messages

- Ordinals via the platform's patched sub-format — English-centric suffixes ("1st"); no CJK/Turkish ordinal rules (Turkish "1." / "1'inci"). [Observed call; Inferred behavior]
- Bundles locale is ROOT for English, so `{0,number}` in English UI is formatted ROOT-style even on a Turkish OS; in a localized UI it follows the pack locale (`ja`), not the OS region. UI number formatting therefore follows **UI language**, while date stamps follow **OS settings** unless overridden (§3.16) — two different sources. [Inferred from `getResolveLocale` + help]

### 3.11 Dynamic vocabularies (`DatabaseDynamicBundle`)

`DatabaseDynamicBundle` (header: "Dynamically readable localization properties") holds per-kind words used to build messages ("Drop 3 tables"), tree captions and DDL generator comments. Keys are computed from enum names: `ObjectKind.<snake_name>` and a separate plural key `ObjectKind.<snake_name>s` (475 `ObjectKind.*` keys), plus dialect sections (`SqlDbElementType`, `OraElementTypes.Kinds`, `MsElementType.Kinds`, `MysqlElementTypes.Kinds`, `PgElementTypes.Kinds`, `GPlumElementTypes.Kinds`, `Db2ElementTypes.Kinds`, …). The file disables "unused property" and "wrong delimiter" inspections for itself because static key checking cannot see computed keys. Fully translated in each pack (495/495). [Observed] The separate-plural-key approach (`tables`) is another English-plural assumption: a count-dependent form ("1 table"/"2 tables") is selected in code, not by the translator.

### 3.12 Translation coverage (measured)

Method: for each of the 146 English bundles, parse keys (with continuation lines and `\u` decoding), compare with the same path in each pack: *have* = keys present in pack, *missing*, *stale* = pack keys absent in English, *identical* = value equal to English and containing ≥3 Latin letters. Details: ALG-L10N-08. [Observed]

**Totals (146 bundles, 26,010 English keys):**

| Pack | Keys present | Coverage | Missing | Stale |
|---|---|---|---|---|
| ja | 25,617 | 98.5% | 393 | 1 (VcsBundle) |
| ko | 25,617 | 98.5% | 393 | 1 |
| zh-CN | 25,617 | 98.5% | 393 | 1 |

**Where the 393 missing keys are (identical for all three packs):**

| Bundle | English keys | Missing | Reason |
|---|---|---|---|
| `IdeDeprecatedMessagesBundle` | 283 | 283 | deprecated keys kept for compatibility — intentionally untranslated |
| `CoreDeprecatedMessagesBundle` | 32 | 32 | same |
| `ActionsDeprecatedMessagesBundle` | 10 | 10 | same |
| `InspectionsDeprecatedMessagesBundle` | 1 | 1 | same |
| `DatabaseProductivityFeatures` | 11 | 11 | feature-usage statistics names (internal) |
| `FullLineBundle` | 82 | 24 | newest strings: survey/feedback dialog |
| `LLMInstallerBundle` | 153 | 22 | newest strings: AI "text to SQL" onboarding |
| `IdeBundle` | 2,791 | 6 | trial-survey options |
| `SettingsSyncBundle` | 123 | 2 | recent |
| `LicenseBundle` | 251 | 1 | recent |
| `VcsBundle` | 1,184 | 1 | recent |

So the real lag for live UI is 56 keys (0.2%), concentrated in strings added late in the release cycle. [Observed; Inferred: "late additions"]

**DB-relevant bundles (all packs):**

| Bundle | Keys | ja / ko / zh present | Identical to English ja / ko / zh | Placeholder mismatches | Reordered placeholders ja / ko / zh |
|---|---|---|---|---|---|
| DatabaseBundle | 2,942 | 100% / 100% / 100% | 56 / 30 / 52 | 0 | 18 / 22 / 14 |
| SqlBundle | 785 | 100% ×3 | 24 / 25 / 25 | 0 | 0 / 1 / 3 |
| DataGridBundle | 393 | 100% ×3 | 3 / 2 / 4 | 0 | 0 / 1 / 0 |
| DatabaseDynamicBundle | 495 | 100% ×3 | 1 / 5 / 2 | — | — |
| GridCsvBundle | 8 | 100% ×3 | 2 / 2 / 2 | — | — |
| MongoJSBundle | 4 | 100% ×3 | 1 / 1 / 1 | — | — |
| ChartBundle | 151 | 100% ×3 | 8 / 9 / 8 | — | — |
| DiagramBundle | 372 | 100% ×3 | 12 / 13 / 13 | — | — |
| DataGripBundle | 26 | 100% ×3 | 0 | — | — |
| LanguageAndRegionBundle | 16 | 100% ×3 | 1 ("English") | — | — |
| Inspection descriptions (DB) | 68 files | 68 / 68 / 68 | — | — | — |
| Intention descriptions (DB) | 35 files | 35 / 35 / 35 | — | — | — |
| Tips (DB plugin / DataGrip) | 10 / 103 | all | — | — | — |

Identical values in DatabaseBundle (ja) fall into: technical terms and product names ("Explain Plan", "Explain Analyse (Raw)", "DDL", "SSH/SSL", "URL:", "SID", "SCRAM-SHA-256", "GSSAPI (Kerberos)"), PostgreSQL storage parameters used as property titles ("Fill Factor", "Using", "CType"), and angle-bracket sentinels (`<Unnamed>`, `<default>`, `<none>`, `<All exc.>`). The Korean pack translated many sentinels (30 identical vs 56), showing these are translator choices rather than rules. [Observed]

`OptionsBundle` has ~136 identical values per pack — mostly `TBD` placeholders for code-style property descriptions and exportable-settings names ("VCS", "XML", "Emmet") — i.e. English placeholders shipped as source text get "translated" as-is. [Observed]

### 3.13 What stays untranslated (by design or by accident)

| Category | Example | Why |
|---|---|---|
| SQL keywords, identifiers, functions, code examples | `WHERE`, `DELETE`, intention before/after SQL | code is language; also the SQL preview in code-style settings |
| Technical terms / product and protocol names | "Explain Plan", "SSH/SSL", "SID", "Kerberos" | recognizability; matches docs and DB manuals |
| Database server messages | PG/MySQL/Oracle error text | comes from the server (its own `lc_messages`/`NLS_LANGUAGE`) |
| Database object names, data values | table names, cell values | user data |
| Registry, logs, internal statistics bundle | `DatabaseRegistryKeys.xml` descriptions | developer surface |
| Deprecated bundles | `*DeprecatedMessagesBundle` | compatibility only |
| Offline help | DataGripHelp plugin | English only (§3.15) |
| Plugin names/descriptions of feature plugins | "Database Tools and SQL" | open request DBE-12237 |
| Safety escape hatch | "Switch back to English" | must be readable by anyone |
| 49 raw searchable-option hits | "SQL", "XML", "Markdown", "Send usage statistics" | language names / legacy hard-coded labels |
| Late strings | survey and AI onboarding keys | translation lag (§3.12) |

### 3.14 Settings search per locale

- Each configurable's searchable options are prebuilt at product build time into JSON: `{id, name, entries:[{hit, path}]}`; `hit` is a token `|b|messages.DatabaseBundle|k|settings.limit.page.size.to|` naming the bundle and key. Our classification: **6,128 key tokens vs 49 raw strings (99.2%)** across `product.jar`, `database-plugin.jar`, `grid-core-impl.jar`. [Observed]
- At query time tokens are resolved through the current bundle chain, so **one index serves every language** and translation updates need no index rebuild. [Inferred from format; doc 11 ALG-SET-03]
- The bundle layer's `idea.l10n.keys=only` policy returns `|b|<bundle>|k|<key>|<value>` for every resolved message — the same token shape — strongly suggesting the index builder runs the settings UI in this mode and records tokens instead of text. [Observed format; Inferred use]
- Whether a localized UI also matches English queries is not established for settings search [Unknown]; for Hacivat we recommend matching both (ALG-L10N-07).

### 3.15 Help localization

- The bundled help plugin (`bundled-datagrip-help`) serves HTML pages from a built-in web server with a Lucene full-text index; it is **English only** (no locale folders). Its settings: "Try opening online help when available", online base URL, browser. [Observed: plugin descriptor, `BuiltInHelpBundle`]
- When online, the help URL is `<base>/help/<product>/<version>/?<topic>` plus the active keymap as a parameter (pages render shortcuts for the user's keymap) — **no locale segment**. [Observed: `BuiltInHelpManager.kt`]
- Localized online help exists: `jetbrains.com/ja-jp/help/datagrip/…`, `zh-cn/…`, `ko-kr/…` return pages (the Japanese query-execution page has a Japanese title). How users reach them from a localized IDE (site-level language redirect vs explicit link) is [Unknown]. [Observed: HTTP check]
- Lesson: help, lint descriptions and tips should share the localized-resource path convention (ALG-L10N-06) and be available **offline in every shipped language** — Hacivat's "works fully offline" requirement makes online-only localized help unacceptable.

### 3.16 Region, date/number formats and the JVM locale

- **Region** (Language and Region page): Africa, Americas, Asia (except China Mainland), China Mainland, Europe, Middle East, Oceania, Not specified. It changes service URLs, AI provider, proxy routing — not UI text or formats. Asked on first start. [Observed: help, `LanguageAndRegionBundle`]
- **Date Formats** (System Settings › Date Formats): by default follow the OS; override with a pattern and 12/24-hour choice; optional "pretty" relative stamps ("10 minutes ago", "Today"). Applies to IDE stamps (VCS log, local history), not to data cells. [Observed: help `date-formats-settings`]
- **Data editor formats** (Database › Data Editor and Viewer): custom number patterns, decimal separator (default `.`), grouping separator, Infinity/NaN rendering, custom date/time/datetime/timestamp(+tz) patterns, display time zone. These are **DB-data** settings, independent of the UI language. [Observed: help `settings-tools-database-data-views`; doc 11]
- **JVM locale leaks into drivers**: DatabaseBundle carries an explanation for the Oracle error raised when the client process locale differs from the server's NLS settings, advising `-Duser.language/-Duser.region/-Duser.country` VM options for the driver process; DBE-23788 (Oracle `DATE` shown wrong under a Thai locale — Buddhist calendar); DBE-20272 (open since 2024: "honor locale settings displaying query results", users unable to influence formatting through any JVM option); DBE-2159 (old request closed after custom number/datetime formats shipped in 2023.3). [Observed: bundle, YouTrack]
- Lesson: three independent axes — **UI language**, **display formats** (user-chosen, default from OS region), **DB session locale** (never derived implicitly from either).

### 3.17 RTL status

- No RTL language pack exists (ja/ko/zh only; the statistics whitelist includes `ar`, `fa`, `ur` as *requested* values). The platform offers no UI mirroring setting. [Observed: packs, `LocalizationState`; Inferred: no mirroring]
- The editor supports bidirectional text rendering with a **Bidi Text Base Direction** group: *Content-based* (direction from initial characters), *Left-to-Right*, *Right-to-Left*. [Observed: ActionsBundle keys] Relevant for SQL containing Arabic/Hebrew string literals.
- Lesson for Hacivat: even without an RTL UI in v1, isolate interpolated identifiers (FSI/PDI) in messages and keep layout direction a pack property (ALG-L10N-10), so an Arabic/Persian pack later needs no code.

### 3.18 Locale bugs in the public tracker (DataGrip)

| Issue | Year | Symptom | Root cause | Hacivat test |
|---|---|---|---|---|
| DBE-7037 | 2018 | Completing `EXISTS` on a Turkish OS inserted `"exısts"()` (quoted, dotless ı) | keyword case conversion with the default (Turkish) locale | L10N-T1 |
| DBE-9986 | 2020 | Routine editor showed `functıon` | same, in DDL/source generation | L10N-T2 |
| DBE-9900 | 2020 | ClickHouse: "Unknown setting sessıon_ıd" | driver/client lowercased a setting name with the Turkish locale | L10N-T3 |
| DBE-25126 | 2026 (open) | Oracle introspection fails when a `WITH` clause is generated in lowercase | case handling of generated SQL with newer driver | L10N-T4 |
| DBE-23788 | 2025 | Oracle `DATE` wrong under Thai locale | non-Gregorian default calendar from JVM locale | L10N-T5 |
| DBE-20272 | 2024 (open) | Cannot make results honor locale | formats not driven by locale; no user control besides custom patterns | — |
| DBE-17424 | 2023 | Code completion settings page partly English | string not in bundle | L10N-T6 (lint) |
| DBE-12237 | open | Plugin name not localized | descriptor text outside bundles | — |

[Observed: YouTrack summaries/descriptions] The Turkish failures are all **technical text passed through locale-sensitive case mapping** — the single most important pitfall (ALG-I18N-05; ALG-L10N-11).

### 3.19 Developer and test tooling

[Observed: `BundleBase.kt`]
- `-Didea.l10n=true` → append 🔅 to every *localized* string (spot untranslated UI by absence of the marker).
- `-Didea.l10n.keys=true` → append `(key)`; `=only` → emit `|b|bundle|k|key|value` tokens.
- `-Didea.l10n.english=true` → append the English default (read from the parent bundle) in parentheses; combined with `keys=true` shows `(key=English)`.
- Missing key → `!key!`; tests can turn on an assertion that logs an error for every missing key.
- A test-only "translation consumer" hook receives every (key, rendered text) pair — used by a UI-robot plugin to write UI tests for different locales.
- No pseudo-localization mode exists in 243. [Observed: absence in `BundleBase.kt`]

---

## 4. Algorithm specifications

Generic algorithms for Hacivat's i18n stack are in `../i18n/01` (ALG-I18N-01 negotiation, -02 message resolution, -03 pseudo-loc, -04 CI validation) and `../i18n/02` (ALG-I18N-05 invariant case folding, -06 search key, -07 grid numbers, -08 server messages). The ALG-L10N specs below are the DataGrip-derived counterparts: they define behavior precisely where DataGrip's mechanism teaches something, and add the test cases harvested from §3.

### ALG-L10N-01: Locale negotiation and fallback chain
- **Problem:** Choose the UI locale and the ordered list of message sources for every lookup; avoid DataGrip's exact-tag-match trap.
- **Inputs / outputs:** user setting (BCP-47 tag or `system`), OS preferred locales, installed packs `{tag, native_name, app_range, direction}` → `FallbackChain = [LanguageIdentifier]`, e.g. `[tr-TR, tr, en]`; plus `ui_direction`.
- **Algorithm:**
  1. `requested = setting == system ? os_preferred_locales : [setting]`. Canonicalize (`tr_TR` → `tr-TR`, `zh-Hans-CN` kept, `iw` → `he`).
  2. `available = {en} ∪ {pack.tag | pack.app_range ∋ app_version}`.
  3. RFC 4647 lookup with likely-subtags: for each requested tag try exact; then same language+script; then language with any region (`ja-JP` → pack `ja`; `zh-TW` must **not** fall to `zh-CN` because scripts differ: `zh-Hant` vs `zh-Hans`); first hit wins; else `en`.
  4. Chain = negotiated tag, its truncations (`tr-TR`, `tr`), then `en`; dedupe.
  5. `ui_direction` = pack metadata (`ltr`/`rtl`), default from CLDR likely script.
  6. On setting change: rebuild chain, emit `LocaleChanged`; UI re-renders (hot swap, ALG-L10N-10). Persist explicit choice; re-negotiate `system` at each start.
  7. Offer OS-language suggestion once (like DataGrip) when `setting = en` explicitly and an OS-matching pack exists; provide a "Switch back to English" action whose label is **always English** in addition to the localized label.
- **Data structures:** `Vec<LanguageIdentifier>`; pack registry.
- **Complexity:** O(requested × available), trivial.
- **Edge cases (exhaustive):** OS locale without region (`tr`); OS returns POSIX `C`/`POSIX` (→ `en`); `tr-CY`/`tr-DE` (→ `tr`); pack `ja` with OS `ja-JP` (DataGrip: no exact match unless the setting stores `ja`; Hacivat: match); `zh-TW` with only `zh-CN` pack (→ `en`, not Simplified); `sr-Latn` vs `sr-Cyrl`; pseudo-locales `en-XA`/`ar-XB` (only in debug builds); pack newer/older than app (reject, notify, fall back); pack partially translated (allowed); pack removed while selected (→ `en` + notification, mirrors `LocalizationPluginListener`); setting refers to a tag no longer installed at startup (reset to `system` but remember last choice); two packs declaring the same tag (prefer user-dir pack, warn); malformed tag in config (→ `system`); external preset (CLI flag/env `HACIVAT_LOCALE`) overrides once like `i18n.locale`.
- **Test cases:** system/[tr-TR] + tr pack → `[tr-TR, tr, en]`, negotiated `tr`; system/[de-DE] no de → `[en]`; setting `tr` pack missing → `en` + notice; system/[ja-JP] + `ja` pack → `ja`; system/[zh-TW] + `zh-CN` only → `en`; system/[C] → `en`; pack `tr` with `app_range` < current → rejected, `en`.
- **Prior art:** [Observed: `LocalizationUtil.getLocale/findLanguageBundle`, `LanguagePluginDetectionStartupActivity` zh-CN special case]; ALG-I18N-01; `fluent-langneg` (Apache-2.0/MIT).
- **Used by:** Hacivat, Perde web UI, Karagöz CLI.

### ALG-L10N-02: Layered resource-bundle resolution (DataGrip-faithful model, Hacivat adaptation)
- **Problem:** Resolve `(domain, id)` across multiple providers (core, feature module, pack, user override) with per-key fallback and deterministic precedence — the equivalent of DynamicBundle's parent re-linking.
- **Inputs / outputs:** `chain` (ALG-L10N-01), providers each exposing `files(locale, domain) -> Option<Catalog>` → `ResolvedCatalogStack = Vec<(rank, Catalog)>` per domain; `lookup(id) -> Option<(Message, rank, locale)>`.
- **Algorithm:**
  1. For each locale `L` in chain (most specific first) and each provider in precedence order `user_override > feature_module_localized_folder > language_pack > built_in`, load catalog for `(L, domain)` if present; assign `rank = (chain_index, provider_index)`. (DataGrip order: region folder > region suffix > language folder > language suffix > pack > English base; Hacivat drops the suffix variants — one naming scheme only.)
  2. Sort by rank; the stack is the "parent chain".
  3. `lookup(id)`: walk the stack; first catalog containing `id` wins. Attributes (`id.tooltip`, `id.accesskey`) are looked up **independently** — a translated value with an untranslated tooltip falls back per attribute.
  4. Cache stacks per (domain, chain); invalidate on pack install/remove/`LocaleChanged`. Lazy-load domains on first use.
  5. "English-on-demand" API: `with_default_locale(|| …)` resolves against `[en]` only (for logs, telemetry-free diagnostics, MCP tool descriptions that must be stable, and the "also match English" search).
- **Data structures:** `HashMap<(Domain, ChainKey), Arc<Vec<RankedCatalog>>>`; catalogs immutable after parse.
- **Complexity:** lookup O(depth) ≤ ~6; memory = sum of loaded domains.
- **Edge cases (exhaustive):** id only in English (fallback); id only in pack (stale key — ignore and report in debug); same id in two packs for one locale (user dir wins); pack catalog with syntax error (skip that file only, keep others, report); domain file missing in pack (whole domain falls back); empty string translation (treat as missing, not as intentional empty — DataGrip treats empty as a value; Hacivat lint flags it); translation identical to English (valid, counted); case-differing ids (`foo-Bar` vs `foo-bar` → distinct; lint forbids); override dir with a newer app version's ids (ignored stale); hot reload while a lookup is in progress (catalogs are `Arc`, old stack stays valid); attributes present without value; region-specific file present but language file missing (`tr-TR` only → still served for `tr-TR`, not for `tr`).
- **Test cases:** chain `[tr, en]`, id `a` in tr+en → tr; `b` only en → en; tooltip of `c` only en while value in tr → value tr + tooltip en; override dir defines `a` → override; tr file malformed → all tr ids fall back to en, one diagnostic; pack removed → stack rebuilt without tr.
- **Prior art:** [Observed: `DynamicBundle.resolveResourceBundle`, `reorderParents`, `LocalizationOrder`, `DefaultBundleService`].
- **Used by:** Hacivat (desktop + Rust core), Perde web, Karagöz.

### ALG-L10N-03: Message formatting and missing-key behavior
- **Problem:** Turn a resolved message plus arguments into display text, never crash, and make failures visible in development.
- **Inputs / outputs:** message (Fluent pattern), typed args, locale, debug policy ∈ {Value, AppendId, OnlyId, Marker, ShowEnglish, Pseudo} → `String` + diagnostics.
- **Algorithm:**
  1. If id not found in any layer: return `⟦id⟧` (DataGrip `!id!`); record diagnostic (once per id per session); in test builds fail the test.
  2. Format with Fluent: numbers/dates via ICU4X with the **UI locale for UI text**; machine-context messages (clipboard, exported files, logs) use invariant formatting.
  3. On formatting error (missing arg, wrong type): return the partially formatted text with the problematic placeable shown as `{$name}`; diagnostic; never throw (DataGrip returns `!invalid format: …!`).
  4. Bidi: wrap interpolated identifiers in FSI…PDI in UI; disable isolation for terminal/CLI and copy-to-clipboard.
  5. Apply debug policy: AppendId → `text (id)`; OnlyId → `id`; Marker → `text🔅` only for strings from a non-English catalog (insert marker before closing markup); ShowEnglish → `text (english)`; Pseudo → ALG-L10N-09.
  6. Never perform case conversion on the result with locale rules unless the message explicitly asks for display casing (ALG-L10N-11).
- **Edge cases (exhaustive):** zero args but message has placeables (error); extra args (ignored, lint); argument value containing `{`/`}`/`'` (no escaping concerns in Fluent; in DataGrip only an issue for the pattern itself); apostrophes in pattern (no doubling needed — removes DataGrip's `''` trap); numeric arg that is `NaN`/`Infinity`; very large integers (i64/bignum → string with grouping per locale); negative zero; dates before 1582/after 9999; non-Gregorian OS calendar (format with Gregorian unless the user explicitly chooses another — avoids DBE-23788 class); message containing markup (only allowed via structured spans); empty translation; recursive term references (cycle → error); missing English for a pack-only id.
- **Test cases:** `rows-retrieved` with `$count=1` en → "1 row retrieved"; tr → "1 satır alındı"; missing id → `⟦missing-id⟧` + diagnostic; `$count="x"` (string) → falls back to raw text, diagnostic; Marker policy on en catalog → no marker; on tr → marker; Thai OS + Gregorian default → `2025-07-21` shown, not `2568-07-21`.
- **Prior art:** [Observed: `BundleBase.postprocessValue`, `useDefaultValue`, debug policies]; ALG-I18N-02.
- **Used by:** Hacivat, Perde, Karagöz.

### ALG-L10N-04: Mnemonic / access-key parsing and assignment
- **Problem:** (a) Parse IntelliJ-style mnemonic markup (useful when importing terminology or comparing with DataGrip strings) and (b) give Hacivat access keys without burdening translators.
- **Inputs / outputs:** (a) string with `&`/`_`/`&&`/`\&` markup, platform → `(plain_text, mnemonic_index?, suffix?)`; (b) labels of a dialog/menu + optional translator `.accesskey` attributes → `Map<widget, char>`.
- **Algorithm (a) — parse:**
  1. If text contains the internal marker: index = marker position; error if two markers or marker at end.
  2. Else scan: `\&` → literal `&`; `&&` → on macOS mark next char, elsewhere drop both and do not mark; single `&` or `_` before a non-marker char → mark (first only; ignore single `&` on macOS if the text contains `&&`); doubled `__`/`&&` before the first mark → literal.
  3. If the plain text ends with `(X)` optionally followed by `…`/`...` and preceded by spaces, and the mark points at `X`, split into text + suffix.
- **Algorithm (b) — assign (Hacivat):**
  1. Use translator-provided `.accesskey` if the char occurs in the label (case-insensitive, Unicode simple case folding, **not** Turkish-specific) and is unique in the container.
  2. Else pick the first unused letter of the label from the locale's keyboard-typable set (Latin letters/digits; for Turkish include `ç ğ ı ö ş ü` only if the platform delivers them as plain keys — default: ASCII only).
  3. Else (CJK labels, or no free letter) append a suffix `(X)` from the English label's access key — DataGrip's convention — rendered **before** a trailing ellipsis.
  4. On macOS do not render access keys in menus; strip suffixes there.
- **Edge cases (exhaustive):** label without letters ("…", "+"); ellipsis forms `...` vs `…`; mark on a space; mark on the last char; two widgets wanting the same key; label with `&` literal ("Save & Close"); Turkish label where the only free letter is `ı`/`İ` (Alt+ı on TR-Q; prefer ASCII); dynamic labels with placeholders (assign after formatting); RTL labels (suffix goes at logical end); screen readers (expose via accessible name, not by visible underline).
- **Test cases:** `Open results in new ta&b` → text "Open results in new tab", index 22 ('b'); `&&Save &Close` on Linux → mnemonic C; on macOS → S; `A__b_c__d` → "A_bc__d", mnemonic 'c'; `新規タブで開く(&B)` → text "新規タブで開く", suffix "(B)"; `ライブテンプレートの挿入(_T)…` → suffix "(T)…"; `Rock \& Roll` → "Rock & Roll", no mnemonic; Hacivat (b): labels ["Kaydet", "Kapat"] → K, a (or translator choice); ja labels → `(S)`, `(C)` suffixes from English keys.
- **Prior art:** [Observed: `BundleBase.replaceMnemonicAmpersand`, `TextWithMnemonic` (Apache-2.0)]; Fluent `.accesskey` attribute convention (Firefox).
- **Used by:** Hacivat desktop (Windows/Linux), Perde web (keyboard shortcuts are separate).

### ALG-L10N-05: Plural and selector selection (CLDR) with ChoiceFormat migration
- **Problem:** Select the right message variant for counts and categories in every language; convert any imported `ChoiceFormat` patterns.
- **Inputs / outputs:** variant map `{category|exact → pattern}`, numeric value with visible fraction digits, locale, type ∈ {cardinal, ordinal} → chosen pattern.
- **Algorithm:**
  1. Exact matches first (`[0]`, `[1]` when written as exact numbers).
  2. Compute CLDR plural operands (n, i, v, w, f, t, e) from the **formatted** number (so `1.0` with `v=1` is `other` in English).
  3. Category = CLDR rules for locale (ICU4X `PluralRules`); pick that variant, else the default (`*other`).
  4. Select expressions on strings/enums (boolean "View/Edit" → `{ $editable -> [true] Edit *[false] View }`) are **not** plurals and must be separate selectors.
  5. Migration of `{n,choice,t0#A|t1#B|t2#C}`: if thresholds are `0,1,2` with A==C → plural `{ $n -> [one] B *[other] A }` (plus exact `[0]` if A differs from C); if thresholds `0,1` used as boolean → string selector; nested conditional fragments → split into separate messages or selectors per fragment; flag anything else for manual review.
- **Edge cases (exhaustive):** 0 (English "0 rows" = other; Latvian zero category; Arabic zero); 1 vs 1.0 vs "1"; negative numbers (category of absolute value); large numbers (compact forms "1,2 B" — avoid in plural selection, select on exact value); fractional 1.5 (English other; French one for 0–1.99); Turkish: one/other but noun stays singular → both variants identical text is fine; Russian 21 (one), 22 (few), 25 (many), 1.5 (other); ordinals: English 1st/2nd/3rd/4th/11th/12th/13th/21st; Turkish ordinal written "1." in numeric form; count not provided (use other); value is NaN (other).
- **Test cases:** en 0→other, 1→one, 2→other; tr 1→"1 tablo", 5→"5 tablo"; ru 1/2/5/21/22/25/11 → one/few/many/one/few/many/many; ar 0/1/2/3/11/100 → zero/one/two/few/many/other; migration of `{0,choice,0#cells|1#cell|2#cells}` → `{ $count -> [one] cell *[other] cells }`; `{0,choice,0#View|1#Edit}` → string selector.
- **Prior art:** [Observed: 17 + 7 ChoiceFormat values, CJK branch repetition]; CLDR plural rules; ICU4X (Unicode-3.0).
- **Used by:** Hacivat, Perde, Karagöz.

### ALG-L10N-06: Localized resource files by path convention
- **Problem:** Localize long-form resources (lint descriptions with SQL examples, tips, onboarding, help) without code and offline.
- **Inputs / outputs:** logical path (`docs/lints/sql-without-where.md`), chain → file bytes + effective locale + staleness flag.
- **Algorithm:** probe in order `locales/<L>/<path>` for each `L` in chain inside (1) user override dir, (2) feature module, (3) language packs, (4) built-in assets; first existing file wins (mirrors `localization/<lang>/<REGION>/` > pack > base). Shared assets (SQL examples, images) are referenced by relative links and resolved with the same chain, falling back to the English copy. Each translated file carries `source-hash` of the English file it was translated from; mismatch → serve translation + "may be outdated" badge (configurable) and CI warning.
- **Edge cases (exhaustive):** region file only (`tr-TR`); file exists but empty; front matter missing; English file deleted but translation remains (orphan → ignore, CI error); case-insensitive file systems (`Docs` vs `docs`); path traversal in logical path (reject `..`); images with text (provide per-locale variant or avoid text); embedded SQL in translated file differing from English (lint: code blocks must be byte-identical unless marked translatable comments).
- **Test cases:** tr present → tr; only en → en; tr with old hash → tr + badge + CI warning; orphan tr → not served; logical path `../x` → rejected.
- **Prior art:** [Observed: `LocalizationUtil.getResourceAsStream`, tips under `localization/zh/CN/tips`, intention before/after kept English].
- **Used by:** Hacivat (lint docs, tips, offline help), Perde docs.

### ALG-L10N-07: Key-based search over localized UI (settings, actions, commands)
- **Problem:** Search settings/commands in any UI language from one prebuilt index.
- **Algorithm:** index stores message ids per setting/action (DataGrip `|b|…|k|…|` tokens; 99.2% coverage); at query time resolve ids through the active chain **and** English; normalize both query and labels with ALG-I18N-06 (Turkish-tolerant folding: `i/İ/ı/I` all equivalent for matching, diacritics optional); rank exact-word > prefix > substring > English-only match; highlight using the mapping from folded to original offsets. Full spec: doc 11 ALG-SET-03.
- **Edge cases:** label containing placeholders (index the static parts); mnemonic markup (strip before indexing); synonyms per locale (`synonyms` attribute in catalogs); query in English while UI Turkish (match via English layer, show Turkish label).
- **Test cases:** tr UI, query "sayfa" → "Sayfa boyutunu sınırla"; query "page size" → same item via English; query "ISIM" matches "İsim" and "isim".
- **Used by:** Hacivat command palette and settings; Perde admin search.

### ALG-L10N-08: Translation coverage and consistency measurement
- **Problem:** Measure completeness and catch structural translation errors, as we did for DataGrip (§3.12), continuously in CI.
- **Inputs / outputs:** English catalogs + each locale's catalogs → per-domain report `{keys, present, missing, stale, identical, placeholder_mismatch, reordered, accesskey_issues, ellipsis_lost, markup_lost, syntax_errors}` and an overall gate.
- **Algorithm:**
  1. Parse catalogs (Fluent parser; for `.properties` imports: handle `\` continuations, `\uXXXX`, `\#\:\=` escapes, `!`/`#` comments).
  2. `present = |ids_en ∩ ids_L|`, `missing = ids_en − ids_L`, `stale = ids_L − ids_en`.
  3. `identical`: value == English and contains ≥3 letters of the English script; report separately with an allow-list (technical terms, product names) so real misses are visible.
  4. Variables: set equality of `$names` per message and per attribute → mismatch = error; order differences → info (expected in SOV languages: DataGrip ja/ko/zh reorder 14–22 messages in DatabaseBundle).
  5. Ellipsis/terminal punctuation: if English ends with "…" the translation must end with "…" (DataGrip lost it once in 104).
  6. Markup/spans parity; access-key attribute validity (char present or suffix rule).
  7. Exclusions: domains flagged `deprecated`/`internal` (like DataGrip's `*Deprecated*` bundles and statistics names) excluded from the denominator and listed.
  8. Gate: syntax errors and variable mismatches fail CI; missing keys fail only for v1 locales (en, tr) at release branches; others report.
- **Data structures:** per-id record with `source_hash` (from `l10n-lock.json`) to detect English changes since translation (status `outdated`).
- **Complexity:** linear in total ids.
- **Edge cases (exhaustive):** duplicate ids in one file (error); id differing only in case (error); terms (`-brand`) referenced but undefined; messages with only attributes; select/plural variant sets differing from English (allowed if the locale's CLDR categories differ — check against CLDR, not against English); whitespace-only values; BOM/CRLF; non-UTF-8 bytes; extremely long translations (> 2.5× English → warning for UI truncation).
- **Test cases:** DataGrip replay: ja vs en across 146 bundles → present 25,617 / 26,010, missing 393, stale 1 (golden numbers from §3.12); synthetic: tr message using `$name` where en uses `$schema` → error; tr ends without "…" → warning; tr `ru`-style plural variants in `tr` → allowed only `one`/`other`.
- **Prior art:** [Observed: our measurement of DataGrip packs]; ALG-I18N-04.
- **Used by:** Hacivat, Perde, Karagöz CI; translator dashboard.

### ALG-L10N-09: Pseudo-localization (what IntelliJ lacks)
- **Problem:** Find hard-coded strings, concatenation, truncation and bidi bugs before translators do (DataGrip only has markers; DBE-17424 shows leaks).
- **Inputs / outputs:** English catalog → synthetic locales `en-XA` (accented, expanded) and `ar-XB` (mirrored, RTL).
- **Algorithm:** for each message, transform only literal text segments (never placeables, markup, access-key attributes, or ids): map ASCII letters to accented look-alikes (`a→á`, `I→Ĭ`, including **`i→ı`, `I→İ`** to exercise Turkish glyphs and casing), pad by +30–50% depending on length (short labels pad more), wrap in `⟦ … ⟧`; for `ar-XB` wrap text in RLM/RLE semantics and set direction rtl. Strings rendered without brackets are hard-coded or concatenated.
- **Edge cases:** messages that are pure placeables; SQL keywords inside messages (if they are in `code` spans, keep untransformed); plural variants (transform each); access keys (recompute after transform); messages used as machine identifiers by mistake (pseudo build breaks them — desirable).
- **Test cases:** "Open results in new tab" → "⟦Ŏṗéñ ŕéšűļţš ıñ ñéŵ ţáƀ ····⟧"; message with `{ $table }` keeps `$table` untouched; screenshot diff shows no clipped bracket in 1280×800.
- **Prior art:** [Observed: `idea.l10n` marker as the closest IntelliJ facility]; ALG-I18N-03.
- **Used by:** Hacivat UI tests, Perde web tests.

### ALG-L10N-10: Language-pack lifecycle (install, validate, hot-swap, remove)
- **Problem:** Add/remove languages with zero code and no restart, safer than build-pinned packs.
- **Inputs / outputs:** pack directory/archive with `pack.toml {tag, native_name, english_name, direction, app_range, version, completeness, maintainers}` + `*.ftl` + resource files → registered pack or rejection reason.
- **Algorithm:**
  1. Discover packs in built-in dir, user dir, and (later) a signed download location.
  2. Validate: manifest schema; `app_range` (semver range, unlike DataGrip's single build); every `.ftl` parses; no ids outside known domains (stale ids allowed but ignored); size limits; signature if downloaded.
  3. Register; recompute available locales (ALG-L10N-01).
  4. Activation: build stacks (ALG-L10N-02) off the UI thread; swap atomically; emit `LocaleChanged`; UI components re-query strings; cached formatted strings (window titles, tray, OS menus) are rebuilt by subscribers; native OS menus/dialogs that cannot be updated live are flagged "restart to update" (macOS app menu).
  5. Removal of the active pack → fall back to next negotiated locale (not necessarily English) and notify.
- **Edge cases (exhaustive):** pack for unsupported direction (rtl before RTL support → load with warning, layout stays LTR); two versions of the same pack (highest compatible wins); corrupt file mid-pack (skip file); pack updating while active (stage then swap); locale change during a modal dialog (dialog re-renders; text fields keep user input); long-running operations with already-formatted progress text (update on next tick); MCP server responses (tool descriptions stay English for agent stability; user-facing error text localized per request locale).
- **Test cases:** drop `locales/tr` into user dir → Turkish selectable without restart; corrupt `grid.ftl` → other tr domains work, grid strings English; remove tr while active → en, notification; pack with `app_range = "<0.5"` on 0.6 → rejected with reason.
- **Prior art:** [Observed: `languageBundle` EP, `LocalizationStateService` restart flag, `LocalizationPluginListener`].
- **Used by:** Hacivat; Perde web (packs as static JSON/FTL bundles).

### ALG-L10N-11: Locale-safe technical text guard (Turkish-İ class)
- **Problem:** Prevent the DBE-7037/9900/9986 class of bugs: locale-sensitive case mapping of SQL keywords, identifiers, setting names, protocol tokens.
- **Algorithm:** two APIs only — `case_invariant(s)` (Unicode default, locale-independent; ASCII fast path for keywords) and `case_for_display(s, locale)`; a lint forbids the bare platform lowercase/uppercase calls in crates `sql-*`, `driver-*`, `ddl-*`, `introspect-*`; dialect keyword case conversion uses ASCII mapping for ASCII keywords and leaves non-ASCII identifiers unchanged unless quoted rules require otherwise (details ALG-I18N-05). Driver connection parameters and session-setting names are compared/emitted byte-exact.
- **Edge cases:** `i`/`I` in keywords (`EXISTS`, `INTEGER`, `LIMIT`, `function`, `session_id`); identifiers containing `İ` (keep as typed); German `ß` → `SS` (never apply to identifiers); Greek final sigma; identifiers compared case-insensitively by the server (use server's rules, e.g. PG folds unquoted to lower **ASCII only**).
- **Test cases (process locale = tr-TR):** L10N-T1 completion of `ex` → `EXISTS`/`exists`, never `EXıSTS`/quoted; L10N-T2 DDL generation of a function → `function`, never `functıon`; L10N-T3 ClickHouse setting `session_id` sent byte-exact; L10N-T4 generated introspection SQL keyword case consistent and accepted by Oracle; L10N-T5 Thai process locale → Gregorian dates in grid; L10N-T6 lint: no UI string literal outside catalogs.
- **Prior art:** [Observed: YouTrack DBE-7037, DBE-9900, DBE-9986, DBE-23788]; ALG-I18N-05.
- **Used by:** Hacivat, Karagöz (rewrites/audit normalization), Perde.

---

## 5. Implications for Hacivat / Karagöz / Perde

**Copy from DataGrip/IntelliJ:**
1. **Stable message IDs everywhere** — code, declarative descriptors (lints, actions, settings), the search index, MCP-facing error payloads. Never key by English text.
2. **English in-core, other languages as data packs** discovered at runtime; adding a language = drop a folder/install a pack. Pack manifest is the analogue of `languageBundle locale=… displayName=…`.
3. **Per-key fallback** to English; partial packs are welcome (DataGrip ships with 0.2% lag and nobody notices).
4. **Separate vocabulary domain for object kinds** (`db-kinds.ftl`) reused by tree, dialogs, DDL comments, MCP tool descriptions — but with CLDR plurals instead of a second `…s` key.
5. **Localized long-form resources by path convention** (lint docs, tips) with feature modules able to override packs (DataGrip's folder-beats-pack ordering).
6. **One search index for all languages** (ids, resolved at query time).
7. **Debug policies**: show ids, show English, marker for localized strings, missing-key sentinel, a translation-consumer hook for UI tests.
8. **Keep SQL and technical terms untranslated**; keep English access-key letters for muscle memory when the translator supplies none.
9. **Always-English escape hatch** to switch language back.

**Improve on:**
1. **Fluent instead of MessageFormat/ChoiceFormat** — named args, CLDR plurals and selectors, no apostrophe doubling, attributes for tooltip/accesskey/aria-label.
2. **Access keys as attributes** or auto-assigned (ALG-L10N-04) — removes ~2,900 `(&X)` edits per language.
3. **Negotiated matching** (RFC 4647) instead of exact tag match; region/script aware (`zh-Hant` vs `zh-Hans`).
4. **Hot-swappable language**; restart only for OS-native surfaces.
5. **Version ranges** for packs instead of build pinning; per-key fallback tolerates skew.
6. **Offline localized help and lint docs** for every shipped language.
7. **Three separate locale axes**: UI language; display formats (user override, default from OS region; ICU4X); DB session locale (explicit per connection, default neutral: `lc_messages`/`NLS_LANGUAGE` untouched, driver-side case/calendar invariant). Karagöz must never inherit the host OS locale into DB sessions.
8. **Server-originated messages** (Karagöz audit reasons, permission denials, Perde notifications) travel as `id + args` and render in the viewer's locale; DB server text shown verbatim with our localized explanation (ALG-I18N-08).
9. **Pseudo-localization in CI** (ALG-L10N-09) and **coverage gates** (ALG-L10N-08).
10. **Lint against locale-sensitive case mapping** in all SQL/driver code (ALG-L10N-11).

**Adding a language (target workflow, zero code):**
1. `hacivat-l10n new-locale tr` (or copy) → creates `locales/tr/*.ftl` with all ids commented out and `pack.toml`.
2. Translate in a TMS (Weblate/Crowdin/Pontoon) that understands Fluent; translator notes come from `#` comments next to each id (context, max length, screenshot link).
3. CI runs ALG-L10N-08 + pseudo/screenshot checks; reviewers approve.
4. Release: pack is bundled (v1: en, tr) or published as a downloadable pack; app picks it up at runtime.

**Suggested layout:**
```
locales/
  en/   common.ftl  sql-editor.ftl  exec.ftl  grid.ftl  ddl.ftl  settings.ftl  lints.ftl  db-kinds.ftl  errors.ftl  mcp.ftl
        docs/lints/*.md   tips/*.md   help/**/*.md
        pack.toml
  tr/   (same files; partial allowed; pack.toml with source hashes in l10n-lock.json)
crates/hacivat-i18n/   loader, negotiation, stacks (ALG-L10N-01/02), formatter (ICU4X), accesskeys (ALG-L10N-04),
                       t! macro with compile-time id check (build.rs reads locales/en), debug policies, pseudo-locales
tools/hacivat-l10n/    coverage/consistency report (ALG-L10N-08), properties→ftl importer for terminology studies
```

---

## 6. i18n considerations (Turkish-specific and general)

- **Dotted/dotless I** — the #1 bug class, proven in DataGrip (DBE-7037, -9900, -9986): never use locale-sensitive case mapping for identifiers, keywords, setting names, file paths or protocol tokens; use `case_invariant()`; use `case_for_display(tr)` only for Turkish UI text shown in uppercase (`i→İ`). In Rust, `str::to_uppercase` is locale-independent (good), but ICU4X `CaseMapper` with a `tr` locale must be confined to display code.
- **Plurals**: Turkish CLDR `one`/`other`, noun stays singular after numerals ("3 tablo"); translators must not mirror English plural structure. Translator guide states this explicitly.
- **Agglutinative suffixes and vowel harmony**: avoid messages that need a case suffix on an interpolated identifier ("`orders` tablosuna"); prefer "Tablo: { $name }" or "{ $name } adlı tablo…" constructions. For unavoidable cases, provide Fluent selectors on a precomputed `$suffix_class` only for known vocabulary (object kinds), never for user identifiers.
- **Word order**: Turkish is SOV — named variables are essential; DataGrip's CJK packs reorder positional placeholders in up to 22 DatabaseBundle messages.
- **Numbers**: Turkish `,` decimal and `.` grouping for UI display; SQL generation, clipboard SQL, CSV defaults and MCP payloads stay invariant (doc 08, ALG-I18N-07). DataGrip uses `{0,number,#}` to avoid grouping in ports/ids — Hacivat should pass such values as strings or with `useGrouping: false`.
- **Dates**: default display from OS region via ICU4X; force Gregorian unless the user picks another calendar (Thai/Persian OS defaults otherwise leak, DBE-23788).
- **Sorting/collation** in UI lists (settings search, object pickers): ICU4X collator for `tr` (`ç ğ ı ö ş ü` order); DB-side ordering is the server's business.
- **Keyboard**: access keys and shortcuts must be tested on TR-Q and TR-F layouts (AltGr+Q = `@`, AltGr+< = `|`); prefer ASCII access keys.
- **Fonts**: Turkish glyphs are trivial; later CJK packs need fallback fonts; pseudo-locale exercises `ı`/`İ` glyphs.
- **Mnemonic letters**: keep English letters when a Turkish label lacks a suitable letter (DataGrip convention), but prefer native letters when present.

---

## 7. Risks, pitfalls, open questions

- **Plural debt**: once strings are written with English-only plural logic (or ChoiceFormat-style booleans/conditionals), retrofitting CLDR plurals touches every string — start with Fluent and the migration rules of ALG-L10N-05 for anything imported.
- **Concatenation and hard-coded strings** (DBE-17424): ban via lint, catch via pseudo-locale.
- **Over-translation**: SQL keywords, function names, `ASC/DESC`, data type names must stay as-is; follow the DataGrip pack practice.
- **Under-translation by sentinel**: DataGrip's ja pack leaves `<default>`, `<none>`, `<Unnamed>` in English; ko translates them. Decide per locale in the glossary.
- **Exact-tag matching** (`ja` vs `ja-JP`, `zh-CN` only) — avoid; negotiate.
- **Version skew**: build-pinned packs force lockstep releases; Hacivat uses ranges + per-key fallback, but must then detect *outdated* translations (source hashes).
- **Locale leakage into DB sessions** (Oracle NLS, Thai calendar, Turkish case) — keep UI locale, display formats and DB session locale separate; Karagöz runs with a neutral process locale.
- **Server-rendered English** from Karagöz/Perde would violate "TR from day one" — transport ids + args.
- **Hot-swap completeness**: every UI component must subscribe to `LocaleChanged`; native OS menus/tray may need a restart prompt.
- **Help offline in every language** multiplies docs work; scope v1 help to lint docs + tips + key pages, translated with source hashes.
- **Open — community packs location**: TR in main repo (first-class, CI-gated); others in a `hacivat-locales` repo with the same CI (ALG-L10N-08) and signed release bundles.
- **Open — settings search in English while UI is localized**: DataGrip behavior unverified [Unknown]; Hacivat will match both (ALG-L10N-07). Add to doc 13 as a check.
- **Open — how localized online help is reached from a localized DataGrip** (no locale segment in the help URL) [Unknown].
- **Open — RTL**: no evidence of RTL UI support in IntelliJ; Hacivat's UI toolkit choice must support mirroring before an Arabic/Persian pack is accepted (`direction = rtl` packs load with a warning until then).
- **Doc 13 follow-up**: add family L10N — switch DataGrip to ja and verify (1) restart requirement, (2) settings search with English query, (3) mnemonic suffix rendering on macOS vs Windows, (4) behavior of the OS-language suggestion, (5) Turkish OS locale with completion/DDL generation (regression check of DBE-7037/9986).
