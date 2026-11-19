# M4 "Metadata and explorer": design notes

**Status: awaiting PO review.** Task 2 Steps 1–2 (mocks and these notes) are done; Step 3 (PO review, changes, approval) is the PO's. Tasks 14, 20, 24, 25, 27 and 32 do not start before approval, or before the controller's fallback ruling (plan Task 2 Step 3).

Clickable mock: `design/m4-explorer/index.html` (one file, one app, a surface switcher). It is the M3b + M3c mock's world, unchanged: the same CSS (copied, with the M4 additions after it), the same Lucide 0.460 sprite (ISC) plus five icons, the same mock bar idea, Notes drawer, toggles and deep links. It runs with no network except Google Fonts. The M2a/M2b strings (`i18n-prior`) and the shipped Fluent strings (`i18n-app`) are reused verbatim; M4 never redefines an earlier Fluent id.

**How to drive it**
- The dashed **mock bar** is not product UI. Row 1 holds the fourteen surfaces. Row 2 holds the scenarios of the current surface (`[` / `]`), the window size (Fit, 1440 × 900, 1280 × 800, 960 × 600; `W`), EN/TR (`L`), dark/light (`T`) and Notes (`N`).
- Keys `1`–`9`, `0`, `-`, `=` jump to surfaces 1–12. Surfaces 13 and 14 are reached with the bar or a deep link. These keys work while the tree doesn't have the focus; in the tree, letters are type-ahead.
- Deep links: `?s=10&sc=options&theme=light&lang=tr&size=960&bar=0`. `bar=0` hides the mock bar, so the window is exactly the viewport. `shot=1` freezes the progress counts for screenshots. `check=all` runs the verification pass (see Verification).
- Every scenario named "960 · TR" pins Turkish at 960 × 600; an explicit `lang` or `size` in the link wins.
- The Notes drawer repeats the decisions per surface and lists the surface's strings (Fluent id, EN, TR with `PO review`).
- Numbers and dates follow the language toggle as a stand-in for the format locale (the M2a stand-in): `1,204` / `1.204`, `Nov 17, 2026, 14:05` / `17 Kasım 2026 14:05`. The product follows the OS region (L3, L15(d)).

**Flows that run**
- **Tree.** Click selects; the chevron or a double-click on a container toggles it; a double-click (or ↵) on a table or view shows what Open data runs. Keyboard (click the tree first): ↑ ↓ Home End, → expands or enters, ← collapses or goes to the parent, kind-group headers take the cursor but never the selection, type-ahead (Turkish-tolerant in TR), F12 View DDL, F5 refresh (the phase row shows briefly), ⌥⌘C copies the qualified name, ⌘← collapses all, ⇧F10 opens the menu.
- **Filter.** Type in the field; the Turkish-tolerant toggle (languages glyph) changes the matches live; Escape or Clear filter empties it.
- **Menus.** Right-click any row; Introspection level › opens the submenu (→ from the keyboard); a level re-reads the namespace; Copy name / Copy qualified name / Hide objects like this / Show hidden objects work; Edit scope… opens surface 7; Clear cached metadata… opens the confirmation.
- **Pill.** Click a level pill to open the level list directly.
- **Scope editor.** Check or uncheck schemas and databases (the pattern follows); All databases asks first; Encrypt cached metadata asks before it deletes the cache.
- **DDL view.** Qualification, Constraints (its Select opens a menu), Grants, Owner and Definitions provided by server re-render the text; Copy and Open in console confirm with a toast; at 960 in Turkish the overflow ⋯ holds what didn't fit; Reload clears the stale notice.
- **Settings.** The Segmented changes Ask/Always/Never; the search field filters rows (Turkish-tolerant); Clear cached metadata… asks first.
- **Refresh after DDL.** The Ask toast's buttons dismiss it and log the refresh in Output.

**Product truth used:** the M4 plan (`plan-m4.md`: "New UI surfaces" 1–14, Task 2, Tasks 14, 20, 24, 25, 27 for what they consume, the Global Constraints "Design rules", "Copy" and "Keys", rulings R-M4-6, -13, -14, -15, -18, -20, -21, -22, -24, -30, -37, -42, -49, -50), `po-questions-m4.md` (Q2, Q3, Q5, Q6, Q9, Q10, Q13–Q16, Q19, Q21 defaults in force), the tree spike report (row anatomy, status slot, group headers, keyboard), `docs/research/datagrip/04-introspection-and-metadata-model.md` §3.1 (level pills: blue explicit, grey inherited) and §3.6 ("N of M" scope, pattern syntax) for behaviour only, DESIGN.md (Named Rules, Explorer Tree, tokens, the M2a/M2b/M3 component rows), `design/m3b-safe/` and `design/m3a-query/` (precedents, binding where an M3 surface is touched), the shipped keymap (`crates/ui/hacivat-ui/keymaps/default-macos.toml`: ⌘F focuses the filter, ⌘1 toggles the explorer, the M2a Explorer bindings on profile roots) and the shipped Fluent files (`locales/{en,tr}`).

---

## Cross-cutting decisions

1. **No new hue, no new filled block.** Red: production marks (the root's 7 px swatch and lock, the capsule, the status segment, the editor hairline) and Delete… only. Amber: a pass or a change that is running or pending (the static 7 px run marker, the "waits for COMMIT" stamp). Green: a check that something succeeded (the root's ✓ latency). Blue: selection, focus, the primary button, and a level **you** set (the explicit pill). Every failure, refusal, partial read or unresolved reference is an ink glyph and words. The capsule's cp-fill block stays the window's one filled badge (checked in every render). Badges and pills are outline-only.
2. **No ambient motion.** Progress is a static phase label plus a determinate count, repainted only when the count changes (once a second at most in the mock); nothing spins, shimmers or slides. A collapsed node being read carries the run marker and a percent as its meta.
3. **The Raised Ink Rule holds.** Menus, tooltips, toasts, dialogs and toolbars map text-3 to text-2; the explorer, the options strip and the code area sit on the panel/side tone where text-3 is allowed (group headers, meta, the grey pill). Field wells (the scope tree, the pattern field) restore text-3.
4. **Names are user data.** Shown verbatim (never case-mapped or translated), bidi-isolated, with invisible characters badged; display order is the UI locale's collation (`users` before `Users`; `ı` before `i` in Turkish); a filter or type-ahead never changes identity. Counts and dates are Number and DateTime arguments that `hacivat_l10n` formats in the format locale (`TABLES 1,204`, `TABLOLAR 1.204`; ruling 22).
5. **Trust signals never shrink.** On a crowded root row the meta ellipsizes first, then badges collapse to their glyph, then the name (keeping about 7 characters); the environment mark and the lock never shrink or clip. The capsule, the status bar's environment segment and the DDL view's read-only chip stay whole at 960 in Turkish (checked in every render).
6. **The Overflow Rule decides, not ellipsis.** Toolbars and strips hide low-priority items into ⋯ before any label truncates; only data (names, types, SQL, hosts) ellipsizes, with the whole value in a tooltip. Status rows (phase, waiting, failed) drop their label before their count in very deep rows, keeping it as the accessible name.
7. **Server text is never inside a translated sentence.** A refused catalog query's message and SQLSTATE sit verbatim in their own mono box in the tooltip; sentences only interpolate names and counts Hacivat chose.
8. **The safe answer is the default.** Mass deletion opens on Keep cached; Clear cached metadata opens on Cancel; the all-databases and encryption questions open on the answer that changes nothing.
9. **GPUI-reproducible.** Solid fills, 1 px borders, radii 0/3/4/6, the popover shadow only; no transforms, filters, blur or masks. Every row is 23 px (the spike's flat, virtualized model; the mock virtualizes too and scrolls 15,000 rows).
10. **Fits 960 × 600 in Turkish.** A headless pass rendered every scenario in EN/TR × dark/light at 1440 × 900 and 960 × 600 (see Verification).

---

## Surfaces

### 1 · Explorer tree (connected PostgreSQL profile; MySQL and SQLite variants)
**Purpose.** The M2a connection list becomes a tree in place (R-M4-18): profile rows are the roots; below them the catalog, read from the snapshot without I/O, down to columns and constraints.

**Anatomy.**
- Panel header and the 28 px filter field as shipped (placeholder now "Filter objects"; the ⌘F hint; the Turkish-tolerant toggle inside the well, surface 4).
- **Rows: 23 px**, 14 px per level from an 8 px base: chevron (12 px, swapped right/down, never rotated) · kind icon (12 px, text-2; blue when selected) · the name (13 px, 600 on a connected root) · type text (Fragment Mono 11, text-2: `bigint`, `(id)`, `→ İSTANBUL_ŞUBE (id)`) · badges (outlined, surface 2/3/12) · flags (roots only: env mark and lock, red only on production) · the level pill (database and schema rows, surface 6) · a right-aligned mono meta (11 px, text-3).
- **Root meta:** connected: green ✓ and latency (M2a); offline with a cache: the history glyph and "cached"; encrypted and not opened: the key glyph (no lock: the lock already means read-only); a pass running below a collapsed root: the run marker and a percent (surface 14).
- **Kind groups** are small-caps headers at text-3 with a seam-soft rule and a mono count (`TABLES 1,204` / `TABLOLAR 1.204`): Tables, Views, Materialized views, Functions, Procedures, Sequences, Types (PG); Events too on MySQL. Under a table: Columns (ordinal order, never sorted), Keys, Foreign keys, Indexes, Checks, Triggers. Headers take the keyboard cursor and expand/collapse with ← → ↵, but never the selection (R-M4-49).
- **Environment headers** keep their M2a look (small caps, rule, count) at 23 px and stay labels: no chevron, not collapsible, skipped by the keys.
- **PostgreSQL:** profile → databases (the connected one first, with the scope count `3 of 4` as meta; other databases and out-of-scope schemas dim at text-3 with an "Add to scope" button on hover and selection, R-M4-14) → schemas → kind groups → objects → child groups → leaves; a collapsed **Built-in** group last per database (surface 3).
- **MySQL/MariaDB:** profile → the server's databases (no schema level; Task 14 labels them "database"); a database not opened yet shows "Loads when you open it". **SQLite:** `main`, `temp` and each attached schema (its file name as meta); no Built-in group (SQLite never uses a snapshot, R-M4-6).
- **Selection:** blue-tint with a 1 px blue left edge and a blue icon. **Keyboard focus:** a 1 px blue ring overlay on the cursor row, shown after keyboard input only (no layout shift).
- **Fixtures:** `public` has 1,204 tables including `müşteri`, `İSTANBUL_ŞUBE`, `users` beside `Users` (two objects, two rows), and `fatura‍lar` with a zero-width joiner; `telemetri` has 15,000 tables (surface 2).
- Table tooltip: "About 48,213 rows (the server's estimate)" (PO-Q14: tooltip only, never in the row).

**States.** PostgreSQL · `users` beside `Users` · Production root (connected, read-only: red swatch + red lock, the capsule's PRODUCTION block, the red editor hairline) · MySQL · SQLite · Keyboard focus on a kind group · Long names (a production root named `orders-reporting-replica-eu-west-1` keeps its swatch and lock; a 54-character table name ellipsizes) · 960 × 600 in Turkish.

**Keyboard.** ↑/↓ move (group headers included, status rows skipped), Home/End, PageUp/PageDown; → expands or moves to the first child; ← collapses or moves to the parent; ↵ activates (profile root: the M2a binding, connect or disconnect; table/view: Open data, R-M4-13; containers: toggle); F12 View DDL; ⌥⌘C Copy qualified name; F5 Refresh; ⌘← Collapse all; ⇧F10 or the menu key: the context menu; type-ahead on printable keys (a 1 s buffer, cycling, wraparound; Turkish-tolerant fold when the UI language or region is Turkish, otherwise exact, R-M4-50). ⌘F focuses the filter (shipped binding; ⌥⌘F joins it per PO-Q10). Expand all children has no default key (menu and palette). No binding fires during an IME composition. Every default follows VS Code where it has one and is remappable; nothing uses ⌘K, and fold-all chords stay unbound.

**Keys, macOS and Windows/Linux** (Task 1b's keymap rows; checked against the AltGr layers in `crates/ui/hacivat-ui/keymaps/altgr/` for TR-Q, TR-F, DE, FR and PL: a Ctrl+Alt chord is AltGr on Windows):

| Command | macOS | Windows / Linux | AltGr check |
|---|---|---|---|
| View DDL (`explorer.view_ddl`) | F12 | F12 | no modifier: passes |
| Copy qualified name (`explorer.copy_qualified_name`) | ⌥⌘C | Shift+Alt+C (`shift-alt-c`, VS Code "Copy Path") | no Ctrl: passes. `ctrl-alt-c` would be AltGr+C = "ć" on PL and is not used |
| Collapse all, tree-wide (`explorer.collapse_all`) | ⌘← | Ctrl+← | no Alt: passes |
| Focus the filter (`explorer.focus_filter`) | ⌘F (shipped) | Ctrl+F (shipped) | passes |
| Focus the filter, PO-Q10's extra chord | ⌥⌘F | none proposed | `ctrl-alt-f` is AltGr+F = "@" on TR-F: fails; the shipped Ctrl+F already does it (reported, not silently re-bound) |
| Refresh (`explorer.refresh`, Explorer context) | F5 | F5 | passes |
| Open data / activate | ↵ | Enter | passes |
| Context menu | ⇧F10, menu key | Shift+F10, menu key | passes |
| Expand all children, Edit scope, Set level, Clear cached metadata | palette only | palette only | — |

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-explorer-tree-label` | Database explorer | Veritabanı gezgini |
| `hv-explorer-kind-tables` | TABLES | TABLOLAR |
| `hv-explorer-kind-views` | VIEWS | GÖRÜNÜMLER |
| `hv-explorer-kind-matviews` | MATERIALIZED VIEWS | SOMUT GÖRÜNÜMLER |
| `hv-explorer-kind-functions` | FUNCTIONS | FONKSİYONLAR |
| `hv-explorer-kind-procedures` | PROCEDURES | PROSEDÜRLER |
| `hv-explorer-kind-sequences` | SEQUENCES | SEKANSLAR |
| `hv-explorer-kind-types` | TYPES | TİPLER |
| `hv-explorer-kind-events` | EVENTS | OLAYLAR |
| `hv-explorer-kind-columns` | COLUMNS | SÜTUNLAR |
| `hv-explorer-kind-keys` | KEYS | ANAHTARLAR |
| `hv-explorer-kind-foreign-keys` | FOREIGN KEYS | YABANCI ANAHTARLAR |
| `hv-explorer-kind-indexes` | INDEXES | İNDEKSLER |
| `hv-explorer-kind-checks` | CHECKS | DENETİMLER |
| `hv-explorer-kind-triggers` | TRIGGERS | TETİKLEYİCİLER |
| `hv-explorer-builtin` | Built-in | Yerleşik |
| `hv-explorer-builtin-badge` | version {version} | sürüm {version} |
| `hv-explorer-builtin-tip` | Built-in catalog (version {version}): the server's own objects, shipped with Hacivat and read without a connection. | Yerleşik katalog (sürüm {version}): sunucunun kendi nesneleri; Hacivat ile gelir, bağlantı olmadan okunur. |
| `hv-explorer-scope-count` | {n} of {total} | {n} / {total} |
| `hv-explorer-scope-count-tip` | {n} of {total} schemas are in scope. Edit scope… changes which ones are read. · [one] {n} of {total} schema is in scope. Edit scope… changes which ones are read. | {total} şemadan {n} tanesi kapsamda. Kapsamı düzenle… hangilerinin okunacağını değiştirir. |
| `hv-explorer-add-scope` | Add to scope | Kapsama ekle |
| `hv-explorer-out-of-scope-tip` | {name} isn't in this connection's scope, so it isn't read. | {name} bu bağlantının kapsamında değil, bu yüzden okunmuyor. |
| `hv-explorer-out-of-scope-db-tip` | {name} isn't in this connection's scope, so it isn't read. Reading it opens one more connection. | {name} bu bağlantının kapsamında değil, bu yüzden okunmuyor. Okumak bir bağlantı daha açar. |
| `hv-explorer-row-estimate-tip` | About {rows} rows (the server's estimate) · [one] About {rows} row (the server's estimate) | Yaklaşık {rows} satır (sunucunun tahmini) |
| `hv-explorer-loads-on-open` | Loads when you open it | Açtığınızda yüklenir |

**Out of scope.** Multi-select and bulk actions (R-M4-50: single selection in M4); type-ahead for IME scripts; row estimates in rows (PO-Q14); drag and drop; virtual-FK editing (M7, PO-Q8); diagrams; the M4b Scripts node (the contributor seam exists, no node is drawn); the status slot (M4b git glyphs).

### 2 · Loading states
**Purpose.** Show what is being read without blocking anything and without motion.

**Anatomy.**
- **First connect:** the root's single child row "Loading…" with the static amber run marker in the chevron column, until the first names arrive.
- **Names first (Brief):** the namespace's names paint at once; a table opened before its columns arrive shows "Waiting for columns" (no marker: it waits, it doesn't run).
- **Phase row:** a child row of the node being read: the run marker, the phase label at 12 px text-2 ("Reading columns…", "Reading keys and indexes…", "Reading definitions…", "Reading names…") and the count in Fragment Mono 11 (`3,400/15,000`; `3.400/15.000` in Turkish), repainted only when it changes. In a row too deep for both, the label drops first and stays the row's accessible name and tooltip.
- **Partial:** an outlined badge "partial" with the ink alert glyph on the namespace; tooltip: "Some objects could not be read" (title) + what is missing + the server's message in its own mono box.
- **Couldn't read:** a failed child row: the ink alert glyph, "Couldn't read audit" in text ink, and a ghost Retry icon button (rotate-cw; its word in the tooltip and accessible name); tooltip: the sentence, then the server's message and SQLSTATE verbatim.

**States.** First connect · Names first · Phase line (15,000 tables, an explicit Structure pill) · Partial badge (tooltip) · Couldn't read (tooltip) · 960 × 600 in Turkish.

**Keyboard.** Status rows are not cursor stops (the spike); Retry is reached through the namespace's menu (Refresh) or the pointer; the badge tooltip is the row's accessible description.

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-explorer-loading` | Loading… | Yükleniyor… |
| `hv-explorer-phase-names` | Reading names… | Adlar okunuyor… |
| `hv-explorer-phase-columns` | Reading columns… | Sütunlar okunuyor… |
| `hv-explorer-phase-constraints` | Reading keys and indexes… | Anahtarlar ve indeksler okunuyor… |
| `hv-explorer-phase-sources` | Reading definitions… | Tanımlar okunuyor… |
| `hv-explorer-progress-count` | {n}/{total} | {n}/{total} |
| `hv-explorer-waiting-pass` | Waiting for columns | Sütunlar bekleniyor |
| `hv-explorer-partial` | partial | eksik |
| `hv-explorer-partial-title` | Some objects could not be read | Bazı nesneler okunamadı |
| `hv-explorer-partial-body` | This login can't read the definitions of {n} functions in {schema}. They are listed by name; their definitions are missing. · [one] This login can't read the definition of {n} function in {schema}. It is listed by name; its definition is missing. | Bu kullanıcı {schema} içindeki {n} fonksiyonun tanımını okuyamıyor. Adlarıyla listelenirler; tanımları eksiktir. |
| `hv-explorer-failed` | Couldn't read {name} | {name} okunamadı |
| `hv-explorer-failed-body` | The server refused the catalog query. Retry reads it again. | Sunucu katalog sorgusunu reddetti. Yeniden dene ile tekrar okunur. |
| `hv-explorer-retry` | Retry | Yeniden dene |

**Out of scope.** Cancel for a running pass (it never blocks; disconnect stops the engine, Task 24); per-object timings; a progress bar.

### 3 · Cached and offline
**Purpose.** Offline, the tree is the last saved copy and says so; built-ins come from the bundled catalog.

**Anatomy.**
- **Cached notice:** the inline-notice anatomy under the filter (history glyph, 12.5 px): the connection's env mark and mono name with a compact **Connect** button on the first line, then "Showing cached metadata from Nov 17, 2026, 14:05" (`17 Kasım 2026 14:05`), wrapping. It follows the selection's connection, so with several offline profiles it names the one you're looking at; the root's meta says "cached" with the history glyph for the others.
- **Out of date:** an outlined badge (history glyph, "out of date") on the namespace; tooltip: the server changed it after it was read; refresh to load the changes.
- **Built-in:** a collapsed row "Built-in" (library icon) last in each database, with the outlined badge "version 17" (MySQL: "version 8.4"); its tooltip: "Built-in catalog (version 17): the server's own objects, shipped with Hacivat and read without a connection." Inside: `pg_catalog`, `information_schema` with their kind groups.
- **Encrypted, not opened (PO-Q19):** the root stays collapsed with the key glyph as meta; tooltip: opening it reads the key from the Keychain. Expanding it is the only thing that touches the Keychain.
- **Keychain missing:** a toast (key glyph): "The Keychain isn't available, so orders-prod's metadata is kept in memory until you quit." (Task 1c may own this id as a `model-…` message; the wording is binding.)

**States.** Cached notice · Out of date (tooltip) · Built-in (tooltip) · Encrypted, unopened (tooltip) · Keychain missing · 960 × 600 in Turkish.

**Keyboard.** Connect in the notice is a Tab stop after the filter; the tree's keys are unchanged offline (every node from the cache is browsable).

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-explorer-cached` | Showing cached metadata from {date} | {date} tarihli önbellekteki üst veri gösteriliyor |
| `hv-explorer-cached-meta` | cached | önbellek |
| `hv-explorer-cached-tip` | Not connected. The tree is the copy saved on {date}; connect to check it against the server. | Bağlı değil. Ağaç, {date} tarihinde kaydedilen kopyadır; sunucuyla karşılaştırmak için bağlanın. |
| `hv-explorer-stale` | out of date | güncel değil |
| `hv-explorer-stale-tip` | The server changed {schema} after it was read. Refresh to load the changes. | {schema} okunduktan sonra sunucuda değişti. Değişiklikleri yüklemek için yenileyin. |
| `hv-explorer-encrypted-tip` | The cached metadata is encrypted. Opening this connection reads its key from the Keychain. | Önbellekteki üst veri şifreli. Bu bağlantıyı açmak anahtarını Anahtar Zinciri'nden okur. |
| `hv-explorer-memory-only` | The Keychain isn't available, so {name}'s metadata is kept in memory until you quit. | Anahtar Zinciri kullanılamıyor; {name} üst verisi uygulamadan çıkana dek yalnız bellekte tutulur. |

**Out of scope.** Choosing which cache to keep, exporting a cache, cache diagnostics (Settings › Metadata covers clearing).

### 4 · Filter field
**Purpose.** Narrow the tree to loaded names, keeping each match in its context.

**Anatomy.**
- The 28 px field: search glyph, the query, the match count in Fragment Mono 11 text-3 ("4 matches", "1 match"), the 20 × 18 Turkish-tolerant toggle inside the well (the M3c find toggle, languages glyph; pressed: blue-tint-strong + 1 px blue). The ⌘F hint shows while the field is empty.
- Matches are highlighted with blue-tint-strong on the matching characters; ancestors are revealed and drawn at text-2; group counts read "2 of 1,204" (`2 / 1.204`).
- When some tables' columns are not loaded, an info line under the field says the filter searches loaded names only and how many tables aren't loaded.
- **Tolerant on** (default when the UI language or region is Turkish, D-074): İ, I, ı and i find each other and accents are ignored (`istanbul` finds `İSTANBUL_ŞUBE`). **Off:** ASCII case-insensitive only (`istanbul` finds nothing there). Invisible characters are skipped in both modes (they can't be typed).
- **No match:** "No objects match", the query quoted, and Clear filter.

**States.** Matches (`müş`, tolerant off) · Turkish-tolerant (`istanbul`, on) · Tolerant off (`istanbul`, no match) · No objects match · 960 × 600 in Turkish.

**Keyboard.** ⌘F focuses the field; typing filters live; ↓ moves into the tree; Escape clears the field, and on an empty field returns the focus to the tree. The toggle is a Tab stop inside the field (Space toggles).

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-explorer-filter-objects` | Filter objects | Nesnelerde ara |
| `hv-explorer-filter-count` | {n} matches · [one] {n} match | {n} eşleşme |
| `hv-explorer-filter-group-count` | {n} of {total} | {n} / {total} |
| `hv-explorer-filter-unloaded` | Searches loaded names only. The columns of {n} tables aren't loaded yet. · [one] Searches loaded names only. The columns of {n} table aren't loaded yet. | Yalnız yüklenmiş adlarda arar. {n} tablonun sütunları henüz yüklenmedi. |
| `hv-explorer-filter-none` | No objects match | Eşleşen nesne yok |
| `hv-explorer-filter-none-body` | Nothing loaded matches “{query}”. | Yüklenmiş adlardan hiçbiri “{query}” ile eşleşmiyor. |
| `hv-explorer-filter-clear` | Clear filter | Filtreyi temizle |

**Out of scope.** Regular expressions and kind prefixes in the filter (the scope pattern has them, surface 7); searching objects that aren't loaded (the M8 search is a different surface); saved filters.

### 5 · Context menus
**Purpose.** A short menu per kind: the frequent action first, then copying, then refresh and scope; nothing that doesn't work yet except Modify (PO-Q13).

**Anatomy.** The PopupMenu anatomy (raised, 6 px, popover shadow, 24 px rows, keys trailing at text-2, separators 1 px seam, the M3c keyboard-active row). Disabled rows keep their place in the disabled ink with their reason as the hint.
- **Profile:** the M2a items (Connect/Disconnect ↵, Edit… ⌘E, Duplicate ⌘D, Copy as URL ⇧⌘C) | Refresh F5, Edit scope…, Introspection level ›, Clear cached metadata…, Collapse all children | Delete… ⌫ (red label).
- **Database:** Refresh, Edit scope…, Introspection level › | Copy qualified name, Expand all children, Collapse all children. Out of scope: Add to scope, Copy qualified name.
- **Schema:** Refresh, Introspection level ›, Remove from scope | Copy qualified name, Expand all children, Collapse all children.
- **Kind group:** Refresh, (Show hidden objects, when a view filter hides some), Expand all children, Collapse all children.
- **Collapse in a node menu acts on its node** (fix round 1): "Collapse all children" collapses everything below the row the menu belongs to, pairing with "Expand all children"; the tree-wide **Collapse all** (VS Code `list.collapseAll`) is the ⌘← key and the palette command (`hv-explorer-menu-collapse-all` is its title) and never sits in a node menu.
- **Table / view / materialized view:** Open data ↵, View DDL F12 | Copy name, Copy qualified name ⌥⌘C, Copy DDL | Refresh F5, Hide objects like this | Modify table… / Modify view… (disabled: "Arrives in a later version").
- **Column:** Copy name, Copy qualified name, View table DDL F12 | Modify column… (disabled).
- **Copy qualified name** uses the dialect's quoting only where needed: `public."Users"`, `public.users`, ``api.`order` ``, `main."a b"`; a PostgreSQL name never carries its database.
- **Hide objects like this** adds a view filter for the name's prefix (`yedek_fatura_*`): the group header gains a 12 px filter glyph at text-3 and counts what is visible (`1,144`); the glyph's tooltip says how many are hidden and how to show them (Show hidden objects in the group's menu). Lookup and completion still find hidden objects (Task 24).
- Export, Import and Edit source are hidden until they work (PO-Q16); a provider that fills the Modify slot replaces the disabled placeholder (R-M4-19).

**States.** Profile · Database · Schema · Kind group · Table · View · Column (Modify disabled) · Hidden like this (tooltip) · 960 × 600 in Turkish.

**Keyboard.** ⇧F10 / the menu key opens it under the row, starting on its first enabled row (a pointer opens with none active); ↑/↓ skip separators and disabled rows; → opens Introspection level, ← closes it; ↵ picks; Escape closes and returns the focus to the tree. No access keys (labels sit next to names).

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-explorer-menu-refresh` | Refresh | Yenile |
| `hv-explorer-menu-edit-scope` | Edit scope… | Kapsamı düzenle… |
| `hv-explorer-menu-level` | Introspection level | Okuma düzeyi |
| `hv-explorer-menu-clear-cache` | Clear cached metadata… | Önbellekteki üst veriyi temizle… |
| `hv-explorer-menu-open-data` | Open data | Veriyi aç |
| `hv-explorer-menu-view-ddl` | View DDL | DDL'i göster |
| `hv-explorer-menu-view-table-ddl` | View table DDL | Tablonun DDL'ini göster |
| `hv-explorer-menu-copy-name` | Copy qualified name | Tam adı kopyala |
| `hv-explorer-menu-copy-plain` | Copy name | Adı kopyala |
| `hv-explorer-menu-hide-like` | Hide objects like this | Benzer nesneleri gizle |
| `hv-explorer-menu-show-hidden` | Show hidden objects | Gizli nesneleri göster |
| `hv-explorer-hidden-tip` | {n} objects are hidden by a view filter ({pattern}). Show hidden objects, in this group's menu, removes it. · [one] {n} object is hidden by a view filter ({pattern}). Show hidden objects, in this group's menu, removes it. | {n} nesne bir görünüm filtresiyle gizlendi ({pattern}). Bu grubun menüsündeki Gizli nesneleri göster filtreyi kaldırır. |
| `hv-explorer-menu-copy-ddl` | Copy DDL | DDL'i kopyala |
| `hv-explorer-menu-modify-table` | Modify table… | Tabloyu değiştir… |
| `hv-explorer-menu-modify-view` | Modify view… | Görünümü değiştir… |
| `hv-explorer-menu-modify-column` | Modify column… | Sütunu değiştir… |
| `hv-explorer-menu-modify-tip` | Arrives in a later version | Sonraki bir sürümde gelecek |
| `hv-explorer-menu-remove-scope` | Remove from scope | Kapsamdan çıkar |
| `hv-explorer-menu-expand-all` | Expand all children | Tüm alt öğeleri aç |
| `hv-explorer-menu-collapse-all` | Collapse all | Tümünü kapat |
| `hv-explorer-copied` | Copied {name} | {name} kopyalandı |
| `hv-explorer-open-data-note` | Opens a console on {conn} and runs: | {conn} üzerinde bir konsol açar ve şunu çalıştırır: |
| `hv-explorer-menu-collapse-children` | Collapse all children | Tüm alt öğeleri kapat |

**Out of scope.** New object…, Drop…, Rename… (M7 with Modify); Diagrams; Compare; Export/Import (M6); Edit source (M7).

### 6 · Introspection level
**Purpose.** How much Hacivat reads per database or schema, visible at a glance and changeable in two clicks (datagrip/04 §3.1).

**Anatomy.**
- The **pill** after the name on database and schema rows: 16 px, 1 px border, 3 px radius, 11 px weight 500, the effective level in a word: Names / Structure / Sources (Adlar / Yapı / Kaynaklar). **Blue** border and text = set on this node; **grey** (seam-strong border, text-3) = inherited from a parent or chosen automatically. Outline only.
- **Tooltip:** "Introspection level: Structure, set here. Schemas below inherit it." / "…, inherited from orders." / "…, chosen automatically for 1,568 objects." (D-038 thresholds, R-M4-21).
- **Introspection level ›** (menu) and a click on the pill open the same list: Automatic (hint "now Structure"), Names only ("fastest, no columns"), Structure ("columns, keys, indexes"), Structure and sources ("and view and function text"), with the blue check on the current setting. Picking one re-reads that namespace (its phase row shows).

**States.** Automatic (grey, tooltip) · Set here (blue, tooltip) · Inherited (grey under a blue database) · Submenu · From the pill · 960 × 600 in Turkish.

**Keyboard.** The pill is not a separate Tab stop (the tree is one stop); the level list is reached with ⇧F10 › Introspection level ›, or the palette ("Set introspection level…", Task 1b). In the list: ↑/↓, ↵, Escape.

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-explorer-level-auto` | Automatic | Otomatik |
| `hv-explorer-level-names` | Names only | Yalnız adlar |
| `hv-explorer-level-structure` | Structure | Yapı |
| `hv-explorer-level-sources` | Structure and sources | Yapı ve kaynaklar |
| `hv-explorer-level-auto-hint` | now Structure · [names] now Names only · [sources] now Structure and sources | şu an Yapı · [names] şu an Yalnız adlar · [sources] şu an Yapı ve kaynaklar |
| `hv-explorer-level-names-hint` | fastest, no columns | en hızlısı, sütunsuz |
| `hv-explorer-level-structure-hint` | columns, keys, indexes | sütunlar, anahtarlar, indeksler |
| `hv-explorer-level-sources-hint` | and view and function text | ve görünüm, fonksiyon metinleri |
| `hv-explorer-pill-names` | Names | Adlar |
| `hv-explorer-pill-structure` | Structure | Yapı |
| `hv-explorer-pill-sources` | Sources | Kaynaklar |
| `hv-explorer-pill-tip-explicit` | Introspection level: Structure, set here. Schemas below inherit it. · [names] Introspection level: Names only, set here. Schemas below inherit it. · [sources] Introspection level: Structure and sources, set here. Schemas below inherit it. | Okuma düzeyi: Yapı, burada ayarlandı. Alttaki şemalar bunu devralır. · [names] Okuma düzeyi: Yalnız adlar, burada ayarlandı. Alttaki şemalar bunu devralır. · [sources] Okuma düzeyi: Yapı ve kaynaklar, burada ayarlandı. Alttaki şemalar bunu devralır. |
| `hv-explorer-pill-tip-inherited` | Introspection level: Structure, inherited from {parent}. · [names] Introspection level: Names only, inherited from {parent}. · [sources] Introspection level: Structure and sources, inherited from {parent}. | Okuma düzeyi: Yapı; {parent} öğesinden devralındı. · [names] Okuma düzeyi: Yalnız adlar; {parent} öğesinden devralındı. · [sources] Okuma düzeyi: Yapı ve kaynaklar; {parent} öğesinden devralındı. |
| `hv-explorer-pill-tip-auto` | Introspection level: Structure, chosen automatically for {n} objects. · [names] Introspection level: Names only, chosen automatically for {n} objects. · [sources] Introspection level: Structure and sources, chosen automatically for {n} objects. | Okuma düzeyi: Yapı; {n} nesne için otomatik seçildi. · [names] Okuma düzeyi: Yalnız adlar; {n} nesne için otomatik seçildi. · [sources] Okuma düzeyi: Yapı ve kaynaklar; {n} nesne için otomatik seçildi. |

**Out of scope.** Levels per object kind; a setting to hide the pills (DataGrip's registry key) unless the PO asks; level on MySQL server roots (levels sit on databases and schemas only).

### 7 · Scope editor (connection dialog, "Schemas" section)
**Purpose.** Which databases and schemas a connection reads, as a checkbox tree and as a pattern (datagrip/04 §3.6), in a new section of the M2a connection dialog (640 px form).

**Anatomy.**
- A **Disclosure** "Schemas" with its live summary ("3 of 4 schemas"; MySQL "All 3 databases"; not connected: the pattern).
- **All databases** checkbox (PostgreSQL), then the **checkbox tree** in a field well (1 px seam-strong, 4 px radius, up to 178 px, scrolls): 22 px rows; databases with a chevron, the M2a checkbox (mixed state = blue fill with the minus glyph), the cylinder icon, the mono name and a text-2 hint ("current database" / "needs its own connection"); schemas indented with the folder-tree icon. Other databases are dim.
- **Pattern** (mono field): the same scope as text: `@` current, `*` all, `{regex}`, `|` between clauses; it round-trips with the tree. A hint line under it. **Error:** the field border goes text-2, an ink line names the problem and the character position ("Expected a schema name after “:” (character 20).") and the caret sits there; Save and Save and connect are disabled until it parses. Never red.
- **All databases on PostgreSQL** asks in place first (an inline notice, never a modal): "Read every database on this server?" with the cost and **Keep the current database** (focused) / **Read all 9 databases**. Asked once.
- "Built-in schemas come from the bundled catalog and are not listed here."
- **Encrypt cached metadata:** a checkbox with its sub-label, on by default for production (PO-Q2). Changing it asks first in an inline notice (it deletes the cache): Keep it as it is (focused) / Delete the cache and encrypt (or … and stop encrypting).
- **Not connected and no cache:** the tree's place says "Connect to list the schemas. The pattern below still decides what is read."
- **MySQL/MariaDB:** databases only, all checked by default (`*`), with "Every database you can see is listed. Each one loads the first time you open it."

**States.** Schemas · Pattern error · All databases? · MySQL · Production (encrypt on) · Encrypt · confirm · Not connected · 960 × 600 in Turkish (the body scrolls; the footer stays).

**Keyboard.** Tab moves Disclosure → All databases → the tree (one stop; ↑/↓ move, Space toggles, → / ← expand/collapse a database) → Pattern → Encrypt → Advanced → footer. The inline questions take the focus on their safe answer. Escape cancels the dialog without saving (Task 20 `escape_cancels_without_saving`).

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-scope-section` | Schemas | Şemalar |
| `hv-scope-summary` | {n} of {total} schemas · [one] {n} of {total} schema | {total} şemadan {n} |
| `hv-scope-summary-mysql` | All {total} databases · [one] {total} database | {total} veritabanının tümü |
| `hv-scope-current-db` | current database | geçerli veritabanı |
| `hv-scope-other-db-hint` | needs its own connection | ayrı bağlantı gerekir |
| `hv-scope-pattern` | Pattern | Desen |
| `hv-scope-pattern-hint` | @ is the current database or schema, * is all, {…} holds a regular expression, \| separates clauses. | @ geçerli veritabanı ya da şema, * tümü, {…} bir düzenli ifade içerir; \| ifadeleri ayırır. |
| `hv-scope-pattern-error` | Expected a schema name after “:” (character {pos}). | “:” işaretinden sonra bir şema adı bekleniyordu ({pos}. karakter). |
| `hv-scope-builtin-note` | Built-in schemas come from the bundled catalog and are not listed here. | Yerleşik şemalar paketle gelen katalogdan okunur, burada listelenmez. |
| `hv-scope-mysql-note` | Every database you can see is listed. Each one loads the first time you open it. | Görebildiğiniz her veritabanı listelenir. Her biri ilk açtığınızda yüklenir. |
| `hv-scope-all-dbs` | All databases | Tüm veritabanları |
| `hv-scope-all-dbs-title` | Read every database on this server? | Bu sunucudaki tüm veritabanları okunsun mu? |
| `hv-scope-all-dbs-body` | This server has {n} databases. Reading them opens one connection per database and can take a while on a large server. · [one] This server has {n} database. Reading it opens one more connection. | Bu sunucuda {n} veritabanı var. Hepsini okumak her veritabanı için bir bağlantı açar ve büyük bir sunucuda zaman alabilir. |
| `hv-scope-all-dbs-yes` | Read all {n} databases · [one] Read {n} database | {n} veritabanının tümünü oku |
| `hv-scope-all-dbs-no` | Keep the current database | Geçerli veritabanında kal |
| `hv-scope-encrypt` | Encrypt cached metadata | Önbellekteki üst veriyi şifrele |
| `hv-scope-encrypt-sub` | The saved catalog is encrypted with a key kept in the Keychain. On by default for production. | Kaydedilen katalog, Anahtar Zinciri'nde tutulan bir anahtarla şifrelenir. Üretimde varsayılan olarak açıktır. |
| `hv-scope-encrypt-confirm-on` | Turning this on deletes the cached metadata of {name}. It is read again, encrypted, on the next connect. | Bunu açmak {name} için önbellekteki üst veriyi siler. Bir sonraki bağlanışta şifreli olarak yeniden okunur. |
| `hv-scope-encrypt-confirm-off` | Turning this off deletes the encrypted cache of {name}. From the next connect it is saved as a plain file only your account can read. | Bunu kapatmak {name} için şifreli önbelleği siler. Bir sonraki bağlanıştan sonra yalnız hesabınızın okuyabildiği düz bir dosyaya kaydedilir. |
| `hv-scope-encrypt-apply` | Delete the cache and encrypt | Önbelleği sil ve şifrele |
| `hv-scope-encrypt-apply-off` | Delete the cache and stop encrypting | Önbelleği sil ve şifrelemeyi bırak |
| `hv-scope-encrypt-keep` | Keep it as it is | Olduğu gibi bırak |
| `hv-scope-connect-to-list` | Connect to list the schemas. The pattern below still decides what is read. | Şemaları listelemek için bağlanın. Neyin okunacağına yine aşağıdaki desen karar verir. |

**Out of scope.** The object filter (`table:-OLD_.*`, datagrip/04 ALG-SCOPE-02) beyond "Hide objects like this"; per-schema levels in the dialog (the pills do that); listing system schemas.

### 8 · Mass-deletion confirmation (D-039, R-M4-22)
**Purpose.** A user refresh that would drop many objects at once asks first; a background pass never removes anything.

**Anatomy.**
- **Dialog** (480 px alertdialog, the M2b anatomy): title "312 objects in public are no longer visible"; subtitle: env mark, mono connection and `orders.public`; body: "They may have been dropped, or this login may have lost access to them. Removing them from the cache changes nothing on the server."; a well listing five names (kind icon + mono name) and "and 307 more".
- Footer: **Remove from cache** (Default, eraser glyph, far left) · **Keep cached** (Primary, focused). Neither is Danger: nothing on the server changes.
- **Background pass:** nothing removed; the namespace carries the "out of date" badge; its tooltip: "312 objects disappeared in the last background check. Refresh public to review them; nothing was removed."
- **A namespace dropped from the scope listing** is not guarded: a toast says it was removed from the explorer.

**States.** Confirmation · Background pass (tooltip) · Schema dropped (toast) · 960 × 600 in Turkish.

**Keyboard.** Initial focus Keep cached; Enter acts on the focused button (key-down, no repeat); Escape and the close icon answer Keep cached; Tab is trapped in the card.

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-metadata-mass-title` | {count} objects in {schema} are no longer visible · [one] {count} object in {schema} is no longer visible | {schema} içindeki {count} nesne artık görünmüyor |
| `hv-metadata-mass-body` | They may have been dropped, or this login may have lost access to them. Removing them from the cache changes nothing on the server. | Silinmiş olabilirler ya da bu kullanıcı onlara erişimini kaybetmiş olabilir. Önbellekten kaldırmak sunucuda hiçbir şeyi değiştirmez. |
| `hv-metadata-mass-more` | and {n} more | ve {n} tane daha |
| `hv-metadata-mass-keep` | Keep cached | Önbellekte tut |
| `hv-metadata-mass-remove` | Remove from cache | Önbellekten kaldır |
| `hv-metadata-mass-pending-tip` | {count} objects disappeared in the last background check. Refresh {schema} to review them; nothing was removed. · [one] {count} object disappeared in the last background check. Refresh {schema} to review it; nothing was removed. | Son arka plan denetiminde {count} nesne kayboldu. İncelemek için {schema} şemasını yenileyin; hiçbir şey kaldırılmadı. |
| `hv-metadata-ns-dropped` | {schema} is no longer on the server, so it was removed from the explorer. | {schema} artık sunucuda yok, bu yüzden gezginden kaldırıldı. |

**Out of scope.** Choosing objects one by one; restoring removed rows (the next refresh reads them again if they exist).

### 9 · Refresh after DDL
**Purpose.** After your own DDL the explorer follows (Always by default, PO-Q6); Ask and Never are settings.

**Anatomy.**
- **Ask:** a wide sticky toast (380 px, refresh glyph, no TTL, never takes the focus; F6 reaches it): "Refresh the database explorer to reflect 3 changed objects?", "Your script in orders-dev changed them.", then **Refresh** (refresh glyph), **Always refresh** (sets the preference to Always), **Not now** (ghost).
- **Inside a transaction** (PostgreSQL, SQLite; MySQL commits DDL at once, R-M4-42): nothing refreshes until COMMIT. The namespace row carries the 7 px amber stamp (pending) with the tooltip "Refreshes after COMMIT" / "2 changes from this transaction wait for COMMIT. ROLLBACK discards them and the explorer stays as it is."; Output logs "Explorer refresh waits for COMMIT (2 objects)" with the same stamp. Turkish keeps COMMIT/ROLLBACK/transaction in English ("COMMIT sonrası yenilenir").
- **Dynamic SQL** (`EXECUTE`, `DO`, a routine): the whole current schema is refreshed (its "Reading names…" row shows) and a toast says why: "Refreshing all of public" / "The script ran dynamic SQL (EXECUTE), so Hacivat can't tell which objects changed."
- **Refreshed:** one Output line ("Explorer refreshed 2 objects in public"); the new table appears in place, selected, no motion.
- **Never:** nothing refreshes; the namespace shows the "out of date" badge until a refresh.

**States.** Ask · In a transaction · Dynamic SQL · Refreshed · Never · out of date · 960 × 600 in Turkish.

**Keyboard.** The toast is reached with F6 (the M3b cycle); Escape there answers Not now. Nothing here blocks a run.

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-metadata-refresh-ask` | Refresh the database explorer to reflect {count} changed objects? · [one] Refresh the database explorer to reflect {count} changed object? | Gezgin, değişen {count} nesneyi gösterecek şekilde yenilensin mi? |
| `hv-metadata-refresh-ask-body` | Your script in {console} changed them. | {console} içindeki betiğiniz bunları değiştirdi. |
| `hv-metadata-refresh` | Refresh | Yenile |
| `hv-metadata-refresh-always` | Always refresh | Her zaman yenile |
| `hv-metadata-refresh-not-now` | Not now | Şimdi değil |
| `hv-metadata-after-commit` | Refreshes after COMMIT | COMMIT sonrası yenilenir |
| `hv-metadata-after-commit-tip` | {count} changes from this transaction wait for COMMIT. ROLLBACK discards them and the explorer stays as it is. · [one] {count} change from this transaction waits for COMMIT. ROLLBACK discards it and the explorer stays as it is. | Bu transaction'daki {count} değişiklik COMMIT'i bekliyor. ROLLBACK bunları atar ve gezgin olduğu gibi kalır. |
| `hv-metadata-broad` | Refreshing all of {schema} | {schema} tamamen yenileniyor |
| `hv-metadata-broad-body` | The script ran dynamic SQL ({what}), so Hacivat can't tell which objects changed. | Betik dinamik SQL çalıştırdı ({what}); Hacivat hangi nesnelerin değiştiğini bilemiyor. |
| `hv-metadata-refreshed` | Explorer refreshed {count} objects in {schema} · [one] Explorer refreshed {count} object in {schema} | Gezgin {schema} içinde {count} nesneyi yeniledi |
| `hv-metadata-log-pending` | Explorer refresh waits for COMMIT ({count} objects) · [one] Explorer refresh waits for COMMIT ({count} object) | Gezgin yenilemesi COMMIT'i bekliyor ({count} nesne) |

**Out of scope.** Session overlays (an uncommitted CREATE visible in completion, M5, R-M4-11); refreshing other connections to the same server.

### 10 · DDL view tab
**Purpose.** A read-only tab with an object's DDL, exactly as Hacivat generates it or as the server returned it, and the options that change it (R-M4-24).

**Anatomy.**
- **Tab:** the kind icon and "{name} · DDL" in mono (the name is an isolated argument); close button. The tab is bound to the object's connection: a production object keeps the 1 px red hairline over the pane.
- **Toolbar** (raised 32 px): the env mark and connection (mono, text-2), a chevron, the kind icon and the qualified name (mono, data, may ellipsize); the read-only chip (lock 12 px text-2, "read-only"; never hides); right: Refresh F5 (ghost), Copy, Open in console.
- **Options strip** (panel, 34 px): Qualification (22 px Segmented: Auto / Never / Always), Constraints (a Select: Inside column / Inside table / After table), Definitions provided by server (checkbox; hidden on PostgreSQL, which has no table definition function), Grants, Owner; right: where the text comes from ("Generated from the catalog" with the table-properties glyph, or "Text from the server" with the server glyph). When the shown text is the server's, Qualification and Constraints hide (they only shape generated text). Defaults: Auto, Inside table (PO-Q9, R-M4-23).
- **Code:** the editor's read-only code area (Fragment Mono 12.5/18, syntax colours, the 58 px gutter, a 4 px thumb when a line is wider than the pane). Generated SQL is never localized and is byte-stable; Copy puts the raw text with `\n` on the clipboard.
- **Notices** (inline-notice anatomy under the strip): MySQL executable comments (triangle-alert, ink): "This text contains MySQL executable comments (/*! … */). MySQL runs what is inside them as SQL." (`ddl-warn-executable-comment`); the comments themselves show verbatim on the inlay well (the M3a executable-comment look). Grants partial (alert, ink). Stale states: surface 11.
- **Reading the definition** (an object below Sources is elevated first, Task 25): the run marker and "Reading the definition of müşteri…" where the code goes; Copy and Open in console wait.
- **Unsupported kind:** an info notice "Hacivat can't show DDL for a type yet." / "sipariş_durum stays in the explorer, and Copy qualified name works."; no code; Copy and Open in console disabled.
- **Overflow Rule:** the source caption hides first, then Owner, Grants and the server toggle move into ⋯ (the More menu keeps their checks), then Open in console and Refresh. With PostgreSQL generated text the strip fits 960 in Turkish once the caption hides; with MySQL generated text (server toggle off, five options) it doesn't, and ⋯ appears: scenario "960 · TR · overflow ⋯" shows it open (Owner, Grants ✓, Definitions provided by server). Labels never truncate; the read-only chip never hides.

**States.** Table (generated) · Options (Always, After table, Grants, Owner) · Reading the definition · Grants · partial · View (server text) · MySQL server text with `/*! */` · SQLite server text · Unsupported kind · 960 · TR · overflow ⋯ (MySQL generated text, the More menu open) · 960 × 600 in Turkish (overflow, horizontal scroll).

**Keyboard.** F12 opens it from the tree (or the palette); the toolbar and the strip are Tab stops in visual order; the code area takes the focus for selection and ⌘C (the standard copy, `ddl_view.copy`); ⌘A selects all; Escape returns to the editor's normal state; ⌘W closes the tab.

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-ddl-view-tab-title` | {name} · DDL | {name} · DDL |
| `hv-ddl-view-qualification` | Qualification | Nitelendirme |
| `hv-ddl-view-qual-auto` | Auto | Otomatik |
| `hv-ddl-view-qual-never` | Never | Hiçbir zaman |
| `hv-ddl-view-qual-always` | Always | Her zaman |
| `hv-ddl-view-constraints` | Constraints | Kısıtlar |
| `hv-ddl-view-constraints-inside-column` | Inside column | Sütunun içinde |
| `hv-ddl-view-constraints-inside-table` | Inside table | Tablonun içinde |
| `hv-ddl-view-constraints-after-table` | After table | Tablodan sonra |
| `hv-ddl-view-server` | Definitions provided by server | Sunucunun verdiği tanımlar |
| `hv-ddl-view-grants` | Grants | Yetkiler |
| `hv-ddl-view-owner` | Owner | Sahip |
| `hv-ddl-view-options` | Options | Seçenekler |
| `hv-ddl-view-copy` | Copy | Kopyala |
| `hv-ddl-view-open-console` | Open in console | Konsolda aç |
| `hv-ddl-view-refresh` | Refresh | Yenile |
| `hv-ddl-view-source-generated` | Generated from the catalog | Katalogdan üretildi |
| `hv-ddl-view-source-server` | Text from the server | Sunucudan gelen metin |
| `hv-ddl-view-copied` | Copied the DDL of {name} | {name} DDL'i kopyalandı |
| `hv-ddl-view-unsupported` | Hacivat can't show DDL for this kind of object yet. · [type] Hacivat can't show DDL for a type yet. · [domain] Hacivat can't show DDL for a domain yet. · [policy] Hacivat can't show DDL for a row security policy yet. · [rule] Hacivat can't show DDL for a rule yet. · [extension] Hacivat can't show DDL for an extension yet. · [operator] Hacivat can't show DDL for an operator yet. | Hacivat bu tür nesnelerin DDL'ini henüz gösteremiyor. · [type] Hacivat tiplerin DDL'ini henüz gösteremiyor. · [domain] Hacivat domain'lerin DDL'ini henüz gösteremiyor. · [policy] Hacivat satır güvenliği ilkelerinin DDL'ini henüz gösteremiyor. · [rule] Hacivat kuralların DDL'ini henüz gösteremiyor. · [extension] Hacivat eklentilerin DDL'ini henüz gösteremiyor. · [operator] Hacivat işleçlerin DDL'ini henüz gösteremiyor. |
| `hv-ddl-view-unsupported-body` | {name} stays in the explorer, and Copy qualified name works. | {name} gezginde kalır; Tam adı kopyala çalışır. |
| `hv-ddl-view-loading` | Reading the definition of {name}… | {name} tanımı okunuyor… |
| `hv-ddl-view-grants-partial` | Some grants could not be read, so the list may be incomplete. | Bazı yetkiler okunamadı; liste eksik olabilir. |
| `ddl-warn-executable-comment` | This text contains MySQL executable comments (/*! … */). MySQL runs what is inside them as SQL. | Bu metin MySQL çalıştırılabilir yorumları (/*! … */) içeriyor. MySQL bunların içindekini SQL olarak çalıştırır. |

**Out of scope.** Editing (M7 Modify / Edit source); formatting the generated text (M5 formatter, R-M4-23); diffing against the server; DDL for several objects at once (Copy DDL of a selection).

### 11 · Stale object
**Purpose.** When the server changed an object after it was loaded, the DDL view and the explorer say so and offer Reload.

**Anatomy.**
- **DDL view, changed:** an inline notice above the code (history glyph): "müşteri changed on the server since it was loaded" with **Reload** (rotate-cw).
- **DDL view, couldn't check** (`check_state` answers Unknown, Task 25): "Hacivat couldn't check whether müşteri changed on the server since it was loaded" (info glyph) with Reload.
- **Explorer:** the same sentence as the row's tooltip (history glyph, title weight).
- **No longer exists:** "müşteri no longer exists on the server" (alert glyph, ink) and "The text below is the last one loaded." with **Close tab**; the code stays (gutter numbers at text-3).

**States.** Changed (DDL view) · Changed (explorer tooltip) · Couldn't check · No longer exists · 960 × 600 in Turkish.

**Keyboard.** The notice's button is the first Tab stop after the toolbar; Reload is also F5 in the DDL view.

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-ddl-view-stale` | {name} changed on the server since it was loaded | {name} yüklendikten sonra sunucuda değişti |
| `hv-ddl-view-reload` | Reload | Yeniden yükle |
| `hv-ddl-view-gone` | {name} no longer exists on the server | {name} artık sunucuda yok |
| `hv-ddl-view-gone-body` | The text below is the last one loaded. | Aşağıdaki metin en son yüklenen metindir. |
| `hv-ddl-view-unknown` | Hacivat couldn't check whether {name} changed on the server since it was loaded | Hacivat, {name} yüklendikten sonra sunucuda değişip değişmediğini denetleyemedi |

**Out of scope.** Showing what changed (a diff); auto-reload.

### 12 · Invisible characters and broken objects
**Purpose.** A name that hides characters, or an object whose reference can't be resolved, is flagged without hiding it.

**Anatomy.**
- **Invisible characters:** an outlined badge with the eye-off glyph and "invisible characters"; when the row is crowded (always at table depth in the 276 px explorer) it collapses to the glyph, keeping the words as its accessible name. Tooltip: "This name contains an invisible character: U+200D ZERO WIDTH JOINER. Copy and DDL keep it exactly." and the name with the character made visible in a mono box (`fatura⟨U+200D⟩lar`).
- **Unresolved reference:** the ink alert glyph replaces the kind icon and the name drops to text-2; tooltip: "Index ix_sipariş_eski references a column that could not be resolved" (title), "The column may have been dropped or renamed after the last read. Refresh the table to check.", and the unresolved name in a mono box.

**States.** Invisible character (tooltip) · Unresolved column (tooltip) · 960 × 600 in Turkish.

**Keyboard.** The tooltip text is the row's accessible description; nothing else changes.

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-explorer-invisible` | invisible characters | görünmez karakter |
| `hv-explorer-invisible-tip` | This name contains an invisible character: {what}. Copy and DDL keep it exactly. | Bu ad görünmez bir karakter içeriyor: {what}. Kopyalama ve DDL onu aynen korur. |
| `hv-explorer-broken` | {name} references a column that could not be resolved · [index] Index {name} references a column that could not be resolved · [foreign-key] Foreign key {name} references a column that could not be resolved · [check] Check constraint {name} references a column that could not be resolved · [trigger] Trigger {name} references a column that could not be resolved | {name}, çözümlenemeyen bir sütuna başvuruyor · [index] {name} indeksi, çözümlenemeyen bir sütuna başvuruyor · [foreign-key] {name} yabancı anahtarı, çözümlenemeyen bir sütuna başvuruyor · [check] {name} denetim kısıtı, çözümlenemeyen bir sütuna başvuruyor · [trigger] {name} tetikleyicisi, çözümlenemeyen bir sütuna başvuruyor |
| `hv-explorer-broken-body` | The column may have been dropped or renamed after the last read. Refresh the table to check. | Sütun son okumadan sonra silinmiş ya da yeniden adlandırılmış olabilir. Denetlemek için tabloyu yenileyin. |

**Out of scope.** Renaming or repairing the object; listing every invisible code point type (bidi controls use the same badge, HV-MODEL-01 i18n notes).

### 13 · Settings › Metadata
**Purpose.** The explorer's background behaviour and the cache, in the shipped Settings page.

**Anatomy.** The M3c page (176 px navigation, the search field, a small-caps section title, rows with a 200 px title column, the control, then its note at 12 px text-2). The navigation gains **Metadata** (dashed "M4" tag is mock-only).
- **Refresh the explorer after DDL:** Segmented Always / Ask / Never; note: "After a console runs a statement that changes the schema. Inside a transaction it waits for COMMIT."
- **Check for changes in the background:** a 150 px Select: Automatic / Off / Every 5 min / Every 10 min / Every 30 min (Task 27); note: "Automatic checks local and dev connections every 10 minutes, and the others when you connect or press Refresh." (PO-Q6).
- **Cache per connection** 512 MB and **Cache for all connections** 4 GB: the M3c number field + unit Select; note on eviction (R-M4-25).
- **Cached metadata:** "Clear cached metadata…" (Default, eraser glyph); note: "orders-dev is the active console's connection. Its cached metadata uses 38.4 MB on this computer." It opens a 420 px alertdialog: "Clear cached metadata for orders-dev?", the size in the subtitle, "The explorer reads it again from the server on the next connect. Nothing on the server changes.", **Clear** (Default) · **Cancel** (Primary, focused).
- Search filters titles and notes (Turkish-tolerant) and names the section after each control.

**States.** Metadata · Ask · Clear cache (dialog) · Search (`cache` / `önbellek`) · 960 × 600 in Turkish.

**Keyboard.** As the shipped page: Tab through rows, arrows in the Segmented, Space/↵/↓ open a Select. The dialog opens on Cancel; Escape cancels.

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-settings-nav-metadata` | Metadata | Üst veri |
| `hv-settings-section-metadata` | METADATA | ÜST VERİ |
| `hv-metadata-set-refresh` | Refresh the explorer after DDL | DDL sonrası gezgini yenile |
| `hv-metadata-set-always` | Always | Her zaman |
| `hv-metadata-set-ask` | Ask | Sor |
| `hv-metadata-set-never` | Never | Hiçbir zaman |
| `hv-metadata-set-refresh-note` | After a console runs a statement that changes the schema. Inside a transaction it waits for COMMIT. | Bir konsol şemayı değiştiren bir ifade çalıştırdıktan sonra. Transaction içindeyse COMMIT'i bekler. |
| `hv-metadata-set-check` | Check for changes in the background | Değişiklikleri arka planda denetle |
| `hv-metadata-check-auto` | Automatic | Otomatik |
| `hv-metadata-set-check-note` | Automatic checks local and dev connections every 10 minutes, and the others when you connect or press Refresh. | Otomatik: yerel ve geliştirme bağlantıları 10 dakikada bir, diğerleri bağlandığınızda ya da Yenile'ye bastığınızda denetlenir. |
| `hv-metadata-every-min` | Every {n} min | {n} dakikada bir |
| `hv-metadata-set-cache-conn` | Cache per connection | Bağlantı başına önbellek |
| `hv-metadata-set-cache-total` | Cache for all connections | Tüm bağlantılar için önbellek |
| `hv-metadata-set-cache-note` | Past the limit, the schemas loaded longest ago leave first. The current database always stays. | Sınır aşılınca en uzun süre önce yüklenen şemalar önce çıkar. Geçerli veritabanı her zaman kalır. |
| `hv-metadata-unit-mb` | MB | MB |
| `hv-metadata-unit-gb` | GB | GB |
| `hv-metadata-set-clear` | Cached metadata | Önbellekteki üst veri |
| `hv-metadata-clear` | Clear cached metadata… | Önbellekteki üst veriyi temizle… |
| `hv-metadata-clear-note` | {name} is the active console's connection. Its cached metadata uses {size} on this computer. | {name} etkin konsolun bağlantısı. Önbellekteki üst verisi bu bilgisayarda {size} kullanıyor. |
| `hv-metadata-clear-title` | Clear cached metadata for {name}? | {name} için önbellekteki üst veri temizlensin mi? |
| `hv-metadata-clear-body` | The explorer reads it again from the server on the next connect. Nothing on the server changes. | Gezgin bunu bir sonraki bağlanışta sunucudan yeniden okur. Sunucuda hiçbir şey değişmez. |
| `hv-metadata-clear-btn` | Clear | Temizle |
| `hv-metadata-cleared` | Cleared the cached metadata of {name} | {name} için önbellekteki üst veri temizlendi |
| `hv-metadata-size-mb` | {n} MB | {n} MB |

**Out of scope.** Per-connection overrides of these settings (the connection dialog has scope and encryption); the cache location; a "show level pills" switch.

### 14 · Status bar during a long introspection — decision: **explorer only**
**Purpose.** Decide where a long pass shows. **Decision: it stays in the explorer; the status bar gets no segment.**

**Why.**
1. The status bar follows the active console and its result tab (Status Follows The Tab); a pass belongs to a profile, which may not be the active console's, so a segment would describe the wrong thing half the time.
2. At 960 in Turkish the bar is already full; a segment would enter the collapse order next to trust signals.
3. Nothing waits on a pass: the console runs, the tree is browsable, completion uses what is loaded. A status segment would suggest otherwise.
4. The explorer already has the place: the phase row under the node being read, and on the nearest visible ancestor when that node is collapsed.

**Anatomy.** The phase row (surface 2) when its node is open. When the node (or an ancestor) is collapsed, the nearest visible ancestor's meta shows the run marker and a percent ("23%", "%23"), down to the profile root; its tooltip: "Reading telemetri: columns 3,400 of 15,000. The explorer stays usable meanwhile." With the explorer hidden (⌘1) nothing shows elsewhere.

**States.** Explorer only (collapsed schema, tooltip) · Collapsed to the root · Explorer hidden · Status segment (rejected, drawn for comparison with a dashed "rejected" tag) · 960 × 600 in Turkish.

**Keyboard.** None new; the meta's tooltip is the row's accessible description.

**Copy.**

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-explorer-progress-meta` | {percent} | {percent} |
| `hv-explorer-progress-tip` | Reading {schema}: the columns of {n} of {total} tables so far. The explorer stays usable meanwhile. · [one] Reading {schema}: the columns of {n} of {total} table so far. The explorer stays usable meanwhile. | {schema} okunuyor: şu ana dek {total} tablonun {n} tanesinin sütunları. Bu sırada gezgin kullanılabilir. |

**Out of scope.** A global activity center; notifications when a pass ends (it ends quietly: the rows fill in).

---

## New primitives

All from existing tokens; DESIGN.md rows are Task 32's.
- **Tree row** (23 px) with **status rows** (loading, waiting, phase with count, failed + Retry icon) and **kind-group headers** (focusable, never selected).
- **Outlined badge** (16 px, 1 px seam-strong, 3 px radius, micro 11.5 text-2, optional 12 px glyph; collapses to the glyph): partial, out of date, version 17, invisible characters.
- **Level pill** (16 px, 1 px border, 3 px radius, 11 px weight 500; blue explicit, grey inherited/automatic).
- **Pending stamp** on a namespace (the 7 px amber square of M3b, "waits for COMMIT").
- **Phase count** in Fragment Mono 11 (`n/total`), **progress meta** (run marker + percent).
- **Hidden-objects glyph** on a group header (filter 12 px text-3).
- **Options strip** (panel, 34 px) under a raised toolbar; **source caption**.
- **Checkbox tree** in a field well; the checkbox's **mixed** state (blue fill, minus glyph).
- **Inline question** (the inline notice with two Default buttons, the safe one focused): all databases, encryption.
- **Icons** (Lucide 0.460, ISC; the Task 1b icon addendum): `columns-3`, `zap` (trigger), `type`, `folder-tree` (schema), `library` (Built-in) are new; `table-2`, `eye`, `table-properties`, `key-round`, `link`, `list-tree`, `shield-check`, `square-function`, `list-ordered`, `cylinder`, `database`, `filter`, `history`, `eraser`, `server`, `plug` exist in the mock sprite (some of them not yet in the app's set: `link`, `shield-check`, `server`, `cylinder`… Task 1b checks each against `assets/icons/lucide/`).

---

## Open questions and rulings

Each is ruled so the build is not blocked; the PO can overrule any.

1. **Environment headers in the tree.** *Ruling:* they stay labels (no chevron, not collapsible, skipped by the keys) and drop from 26 px to the tree's 23 px row (the flat model has one row height). *Cost if wrong:* a variable-height row in Task 8's model, or a chevron on each header.
2. **Partial badge text.** The plan quotes "Some objects could not be read" as the badge. *Ruling:* the badge says "partial"; that sentence is its tooltip's title (a sentence doesn't fit a 276 px row). *Cost if wrong:* copy.
3. **Built-in badge.** *Ruling:* the badge says "version 17"; "Built-in catalog (version 17)" opens its tooltip. *Cost if wrong:* copy.
4. **Pill words.** *Ruling:* Names / Structure / Sources (words, not DataGrip's L1–L3); blue = set here is "your choice", the selection meaning of blue. *Cost if wrong:* three strings, or a hue question if the PO reads blue as decorative.
5. **Progress on a collapsed node.** *Ruling:* the run marker and a percent as meta (a count does not fit beside a pill); the phase row shows `3,400/15,000`; the tooltip has the words. *Cost if wrong:* a meta format.
6. **Surface 14.** *Ruling:* explorer only (see surface 14). *Cost if wrong:* one status segment and its collapse priority in Task 24.
7. **Cached notice placement.** *Ruling:* one notice under the filter that follows the selection's connection, plus "cached" on each offline root; not a row inside each tree. *Cost if wrong:* a notice row per root.
8. **Remove from cache is not Danger.** *Ruling:* Default outline: it deletes cache rows, not data (Hue Is Meaning). *Cost if wrong:* one variant.
9. **Ask is a toast, not a dialog.** *Ruling:* a sticky toast that never takes the focus, so a run is never blocked (Task 27 probe "the prompt blocking a run"); "Always refresh" sets the preference in one click. *Cost if wrong:* a dialog in Task 27.
10. **The pending stamp sits on the namespace**, not on the (not yet visible) object. *Cost if wrong:* where Task 27 draws it.
11. **Hide objects like this** derives a prefix pattern (the name up to its last `_`, then `*`). *Cost if wrong:* the pattern rule in Task 24; a dialog to edit it would be new UI.
12. **Retry in a failed row is icon-only** (its word in the tooltip), so the Turkish sentence fits at schema depth. *Cost if wrong:* a wrapped row.
13. **Status rows drop their label before their count** in very deep rows (Overflow Rule; the label stays the accessible name). *Cost if wrong:* a second line.
14. **Background check is one Select** (Automatic / Off / 5 / 10 / 30 min) as Task 27 lists, not one Select per environment. *Cost if wrong:* two rows.
15. **The encryption toggle asks inline** before deleting the cache (Task 20 says "after a confirmation"); the safe answer keeps it as it is. *Cost if wrong:* a dialog.
16. **Qualification and Constraints hide when the shown text is the server's** (they only shape generated text). *Cost if wrong:* two disabled controls with a reason.
17. **Columns keep ordinal order**; every other child group sorts by the UI collation. *Cost if wrong:* a sort switch.
18. **The DDL tab title** is "{name} · DDL" with the kind icon. *Cost if wrong:* copy.
19. **Turkish words** (all `# Review: PO`): Gezgin (shipped), "üst veri" for metadata, "Okuma düzeyi" for introspection level, Adlar / Yapı / Kaynaklar, SOMUT GÖRÜNÜMLER, SEKANSLAR, İNDEKSLER, DENETİMLER (checks), TETİKLEYİCİLER, "Yerleşik" (built-in), "Benzer nesneleri gizle"; transaction, commit and rollback stay English.
20. **`ddl-warn-executable-comment`** keeps the plan's id (R-M4-24) without the `hv-` prefix: it belongs to the core block that emits warnings (Task 1a's `## m4-t01a-model` in `hacivat-core.ftl`, or the DDL task that owns the warning), not Task 1b's UI block. The wording here is binding.
21. **No sentence is assembled from fragments** (fix round 1). `hv-ddl-view-unsupported` and `hv-explorer-broken` are one message each with a `$kind` selector holding a whole sentence per kind (article and case right in both languages; Turkish puts the kind after the name: "ix indeksi, …"); the pill tooltips and the level hint use a `$level` selector instead of inserting a level word. The `hv-explorer-word-*` messages are gone. *Cost if wrong:* none; it is the Fluent rule.
22. **Counts are Number arguments.** The shipped precedent (`hv-conn-import-done`, `hv-conn-file-entries-skipped`, `hv-history-count`) passes one Number that both selects the plural and is displayed; `hacivat_l10n`'s bundle formatter (`crates/core/hacivat-l10n/src/format.rs`) renders every Number, `NUMBER()` and `DATETIME()` in the format locale while plural rules follow the UI language. So no preformatted count and no second display variable: percents are `NUMBER($fraction, style: "percent")`, the cache date is `DATETIME($date, …)`, the error position `NUMBER($pos, useGrouping: "false")`. *Cost if wrong:* argument types in Task 1b's FTL.
23. **Node menus collapse their node** ("Collapse all children"); the tree-wide Collapse all stays on ⌘← and the palette. *Cost if wrong:* one menu row.
24. **No extra Windows/Linux chord for the filter**: PO-Q10's `ctrl-alt-f` fails the AltGr check on TR-F; the shipped Ctrl+F stays the binding there. *Cost if wrong:* a PO-chosen alternative.

### For the controller (Task 1b / 1c pick-up)
1. **`ddl-warn-executable-comment` needs an owning block** in `locales/{en,tr}/hacivat-core.ftl` (ruling 20): Task 1a's `## m4-t01a-model` or the DDL task that emits the warning; the wording here is binding.
2. **Five new icons** for Task 1b's icon addendum (Lucide 0.460, ISC; SVGs, `assets.rs` rows, `IconName` variants): `columns-3` (column), `zap` (trigger), `type` (type), `folder-tree` (schema), `library` (Built-in). Also check `link`, `shield-check`, `server`, `cylinder`, `filter`, `history`, `eraser`, `key-round` against `crates/ui/hacivat-ui-core/assets/icons/lucide/`.
3. `hv-explorer-memory-only` may be routed through Task 1c's `model-…` keyring message; the wording here is binding.
4. The Windows/Linux key rows above (rulings 23, 24): `shift-alt-c` for Copy qualified name; no `ctrl-alt-f`.

### Open items for the PO
1. **Turkish copy:** every TR string below is a draft (`# Review: PO`), in particular the words in ruling 19.
2. **Surface 14 decision** (ruling 6): explorer only.
3. **Environment headers at 23 px without a chevron** (ruling 1): every approved explorer shot changes anyway (PO-Q11).
4. **Badge and pill wording** (rulings 2–4).
5. **Ask as a toast** (ruling 9) and **Remove from cache as Default** (ruling 8).
6. **Hide objects like this** pattern (ruling 11).

---

## Verification

Run on the committed file (the harness is the mock's own `?check=all` mode, driven by headless Chrome with `--dump-dom`; the driver scripts live in the session scratchpad and are not committed):
- **598 renders, 0 findings** (after fix round 1): 86 scenarios of the fourteen surfaces × EN/TR × dark/light × 1440 × 900 and 960 × 600 (the fifteen pinned "960 · TR" scenarios render dark and light only).
- Each render was checked for: exceptions; the number of cp-fill blocks larger than a mark (exactly one when the capsule is production, else none); text below 11 px (small caps excepted); text-3 on raised (dark; disabled exempt); `undefined`, `NaN` or an unfilled `{placeholder}` in any drawn text; horizontal overflow in toolbars, the options strip, the status bar, the title bar, dialog heads and footers, menus, tooltips, toasts, settings rows, tree rows, notices, panel headers, the filter field, tabs, scope rows and the settings navigation; chrome text that ellipsizes (data exempt: names, types, meta, mono values, the crumb); trust flags on tree roots clipped or outside their row and the explorer; the capsule's production block and access, the DDL view's read-only chip and the status bar's environment segment hidden or outside the window; floating layers (menus, tooltips, toasts) outside the window or over the status bar; a modal without the focus inside it.
- **27 behaviour probes, all PASS** (21 from the first round, 6 added by fix round 1: a schema's Collapse all children collapses only below it; whole-sentence kind variants in EN and TR; EN singular variants; the level selector; the 960 · TR overflow ⋯ drawn and open with its checks), driven through real DOM events: ↓ moves the cursor; ← / → collapse and expand a kind-group header, which never takes the selection; type-ahead `ü` lands on `ürün`; ↵ on a table opens data with `SELECT * FROM public.müşteri`; F12 opens the DDL tab; ⌘← collapses all; right-click opens the table menu with Modify disabled and its reason; ↵ in the menu copies `public.müşteri`; `istanbul` finds nothing with Turkish-tolerant off and nine objects with it on; no match shows Clear filter, which restores the tree; Qualification Always and Owner change the generated text; checking a schema rewrites the pattern to `@:*`; All databases asks first, focused on the safe answer; mass deletion opens on Keep cached and Escape closes it; the pill opens the level list with Automatic checked.
- The PNGs were looked at (every scenario at 1440 in dark or light, every "960 · TR" in light). That review caught what the checks did not, all fixed in the design, never by relaxing a check: the environment header colliding with the `.env` mark class; columns sorted by name instead of ordinal order; a label giving way before its type text; badges truncating instead of collapsing to their glyph; the progress meta ellipsizing its count (now a percent); the Constraints Select shrinking to an empty box at 960 instead of moving options into ⋯ (the strip's items no longer shrink); a long DDL line clipped without a scroll thumb; the settings note's mono name splitting across lines and the search highlight eating a space (flex children; now wrapped); a dialog cell picking up the tile grid's `.col` background; the light theme's mixed checkbox drawn empty; Qualification shown over server text; the console tab not following the capsule's connection on production; the waiting row's label ellipsized (now "Waiting for columns"); the phase label truncated in Turkish (the status row's marker now sits in the chevron column).
- The copy tables below are generated from the mock's own `i18n` block, so the two cannot drift.

---

## Copy strings (EN / TR)

The id is the Fluent id. Variables are Fluent variables (`{ $count }`). **Counts are Number arguments** (ruling 22): one Number both selects the plural and is shown, and `hacivat_l10n` formats it in the format locale (L3; P31) while plural rules follow the UI language, as the shipped `hv-conn-import-done` does; percents, the cache date and the error position use `NUMBER()` / `DATETIME()` the same way. **Every EN message with a count has a `[one]` / `*[other]` selector** (the shipped style at `locales/en/hacivat-ui.ftl:156, 171, 309`); Turkish needs none after a numeral and never puts a case suffix on one. **No sentence is assembled from fragments** (ruling 21): kinds and levels are `$kind` / `$level` selectors holding whole sentences. Names inside a message (`{ $name }`, `{ $schema }`, `{ $console }`, `{ $conn }`, `{ $parent }`) are FSI/PDI-isolated by the caller. Small-caps labels are pre-cased in the file, never cased in code (the shipped `hv-explorer-group-*` rule). Turkish keeps transaction, commit and rollback in English. In the tables, `· [one] …`, `· [index] …` and `· [names] …` list a message's other variants; the FTL below is the authoritative shape.

| Prefix | File | Owner (own block) |
|---|---|---|
| `hv-explorer-*`, `hv-scope-*`, `hv-ddl-view-*`, `hv-metadata-*`, `hv-settings-nav-metadata`, `hv-settings-section-metadata` | `locales/{en,tr}/hacivat-ui.ftl` | Task 1b, `## m4-t01b-ui` |
| `ddl-warn-executable-comment` | `locales/{en,tr}/hacivat-core.ftl` | the block that emits DDL warnings (ruling 20) |
| `hv-explorer-memory-only` | `locales/{en,tr}/hacivat-ui.ftl` | Task 1b (Task 1c may route it through its `model-…` message; wording binding) |

Reused verbatim, not redefined: `hv-explorer-group-*`, `hv-conn-menu-*`, `hv-capsule-latency`, `hv-capsule-access-read-only`, `hv-find-tolerant`, `hv-find-tolerant-tip`, `hv-settings-*` (title, search, navigation, off, no results), `hv-results-output`, `hv-results-empty`, `hv-editor-lncol`, `hv-tx-open`, `hui-tab-close`, `hui-menu-more`, `hui-toast-dismiss` and the M2a dialog strings.

### Proposed FTL, English (`locales/en/hacivat-ui.ftl`)

```ftl
## m4-t01b-ui

# Surface 1 (Explorer tree).
hv-explorer-tree-label = Database explorer
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-tables = TABLES
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-views = VIEWS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-matviews = MATERIALIZED VIEWS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-functions = FUNCTIONS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-procedures = PROCEDURES
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-sequences = SEQUENCES
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-types = TYPES
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-events = EVENTS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-columns = COLUMNS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-keys = KEYS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-foreign-keys = FOREIGN KEYS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-indexes = INDEXES
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-checks = CHECKS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-triggers = TRIGGERS
# Surface 1 (Explorer tree). The Built-in group of a database.
hv-explorer-builtin = Built-in
# Surface 1 (Explorer tree). The Built-in group of a database.
# Variables:
#   $version (String) - the bundled catalog's server version, e.g. "17".
hv-explorer-builtin-badge = version { $version }
# Surface 1 (Explorer tree). The Built-in group of a database.
# Variables:
#   $version (String) - the bundled catalog's server version, e.g. "17".
hv-explorer-builtin-tip = Built-in catalog (version { $version }): the server's own objects, shipped with Hacivat and read without a connection.
# Surface 1 (Explorer tree). The database row's schema scope count (N of M).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
hv-explorer-scope-count = { $n } of { $total }
# Surface 1 (Explorer tree). The database row's schema scope count (N of M).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
hv-explorer-scope-count-tip = { $total ->
        [one] { $n } of { $total } schema is in scope. Edit scope… changes which ones are read.
       *[other] { $n } of { $total } schemas are in scope. Edit scope… changes which ones are read.
    }
# Surface 1 (Explorer tree).
hv-explorer-add-scope = Add to scope
# Surface 1 (Explorer tree).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-explorer-out-of-scope-tip = { $name } isn't in this connection's scope, so it isn't read.
# Surface 1 (Explorer tree).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-explorer-out-of-scope-db-tip = { $name } isn't in this connection's scope, so it isn't read. Reading it opens one more connection.
# Surface 1 (Explorer tree).
# Variables:
#   $rows (Number) - the server's row estimate, e.g. 48213.
hv-explorer-row-estimate-tip = { $rows ->
        [one] About { $rows } row (the server's estimate)
       *[other] About { $rows } rows (the server's estimate)
    }
# Surface 1 (Explorer tree).
hv-explorer-loads-on-open = Loads when you open it
# Surface 2 (Loading states).
hv-explorer-loading = Loading…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
hv-explorer-phase-names = Reading names…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
hv-explorer-phase-columns = Reading columns…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
hv-explorer-phase-constraints = Reading keys and indexes…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
hv-explorer-phase-sources = Reading definitions…
# Surface 2 (Loading states). Progress of a pass in the explorer (surface 14: explorer only).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
hv-explorer-progress-count = { $n }/{ $total }
# Surface 2 (Loading states).
hv-explorer-waiting-pass = Waiting for columns
# Surface 2 (Loading states). The partial-load badge and its tooltip.
hv-explorer-partial = partial
# Surface 2 (Loading states). The partial-load badge and its tooltip.
hv-explorer-partial-title = Some objects could not be read
# Surface 2 (Loading states). The partial-load badge and its tooltip.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-explorer-partial-body = { $n ->
        [one] This login can't read the definition of { $n } function in { $schema }. It is listed by name; its definition is missing.
       *[other] This login can't read the definitions of { $n } functions in { $schema }. They are listed by name; their definitions are missing.
    }
# Surface 2 (Loading states). A namespace whose catalog query was refused.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-explorer-failed = Couldn't read { $name }
# Surface 2 (Loading states). A namespace whose catalog query was refused.
hv-explorer-failed-body = The server refused the catalog query. Retry reads it again.
# Surface 2 (Loading states).
hv-explorer-retry = Retry
# Surface 3 (Cached and offline). A profile shown from its cache while offline.
# Variables:
#   $date (DateTime) - when the cache was saved, e.g. 2026-11-17 14:05.
hv-explorer-cached = Showing cached metadata from { DATETIME($date, dateStyle: "medium", timeStyle: "short") }
# Surface 3 (Cached and offline). A profile shown from its cache while offline.
hv-explorer-cached-meta = cached
# Surface 3 (Cached and offline). A profile shown from its cache while offline.
# Variables:
#   $date (DateTime) - when the cache was saved, e.g. 2026-11-17 14:05.
hv-explorer-cached-tip = Not connected. The tree is the copy saved on { DATETIME($date, dateStyle: "medium", timeStyle: "short") }; connect to check it against the server.
# Surface 3 (Cached and offline).
hv-explorer-stale = out of date
# Surface 3 (Cached and offline).
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-explorer-stale-tip = The server changed { $schema } after it was read. Refresh to load the changes.
# Surface 3 (Cached and offline).
hv-explorer-encrypted-tip = The cached metadata is encrypted. Opening this connection reads its key from the Keychain.
# Surface 3 (Cached and offline).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-explorer-memory-only = The Keychain isn't available, so { $name }'s metadata is kept in memory until you quit.
# Surface 4 (Filter field). The explorer filter field.
hv-explorer-filter-objects = Filter objects
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-explorer-filter-count = { $n ->
        [one] { $n } match
       *[other] { $n } matches
    }
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
hv-explorer-filter-group-count = { $n } of { $total }
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-explorer-filter-unloaded = { $n ->
        [one] Searches loaded names only. The columns of { $n } table aren't loaded yet.
       *[other] Searches loaded names only. The columns of { $n } tables aren't loaded yet.
    }
# Surface 4 (Filter field). The explorer filter field.
hv-explorer-filter-none = No objects match
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $query (String) - the filter text as typed, e.g. "müş".
hv-explorer-filter-none-body = Nothing loaded matches “{ $query }”.
# Surface 4 (Filter field). The explorer filter field.
hv-explorer-filter-clear = Clear filter
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-refresh = Refresh
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-edit-scope = Edit scope…
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-level = Introspection level
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-clear-cache = Clear cached metadata…
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-open-data = Open data
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-view-ddl = View DDL
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-view-table-ddl = View table DDL
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-copy-name = Copy qualified name
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-copy-plain = Copy name
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-hide-like = Hide objects like this
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-show-hidden = Show hidden objects
# Surface 5 (Context menus).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $pattern (String) - the view filter's pattern, e.g. "yedek_fatura_*".
hv-explorer-hidden-tip = { $n ->
        [one] { $n } object is hidden by a view filter ({ $pattern }). Show hidden objects, in this group's menu, removes it.
       *[other] { $n } objects are hidden by a view filter ({ $pattern }). Show hidden objects, in this group's menu, removes it.
    }
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-copy-ddl = Copy DDL
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-modify-table = Modify table…
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-modify-view = Modify view…
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-modify-column = Modify column…
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-modify-tip = Arrives in a later version
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-remove-scope = Remove from scope
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-expand-all = Expand all children
# Surface 5 (Context menus). The tree-wide command (palette, ⌘←); node menus use hv-explorer-menu-collapse-children.
hv-explorer-menu-collapse-all = Collapse all
# Surface 5 (Context menus).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-explorer-copied = Copied { $name }
# Surface 5 (Context menus).
# Variables:
#   $conn (String) - the connection name, e.g. "orders-dev".
hv-explorer-open-data-note = Opens a console on { $conn } and runs:
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-collapse-children = Collapse all children
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-auto = Automatic
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-names = Names only
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-structure = Structure
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-sources = Structure and sources
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Variables:
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
hv-explorer-level-auto-hint = { $level ->
        [names] now Names only
        [sources] now Structure and sources
       *[structure] now Structure
    }
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-names-hint = fastest, no columns
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-structure-hint = columns, keys, indexes
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-sources-hint = and view and function text
# Surface 6 (Introspection level). The level pill's word.
hv-explorer-pill-names = Names
# Surface 6 (Introspection level). The level pill's word.
hv-explorer-pill-structure = Structure
# Surface 6 (Introspection level). The level pill's word.
hv-explorer-pill-sources = Sources
# Surface 6 (Introspection level). The level pill's tooltip and accessible name: one whole sentence per level.
# Variables:
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
hv-explorer-pill-tip-explicit = { $level ->
        [names] Introspection level: Names only, set here. Schemas below inherit it.
        [sources] Introspection level: Structure and sources, set here. Schemas below inherit it.
       *[structure] Introspection level: Structure, set here. Schemas below inherit it.
    }
# Surface 6 (Introspection level). The level pill's tooltip and accessible name: one whole sentence per level.
# Variables:
#   $parent (String) - the database or schema the level comes from, e.g. "orders".
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
hv-explorer-pill-tip-inherited = { $level ->
        [names] Introspection level: Names only, inherited from { $parent }.
        [sources] Introspection level: Structure and sources, inherited from { $parent }.
       *[structure] Introspection level: Structure, inherited from { $parent }.
    }
# Surface 6 (Introspection level). The level pill's tooltip and accessible name: one whole sentence per level.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
hv-explorer-pill-tip-auto = { $level ->
        [names] Introspection level: Names only, chosen automatically for { $n ->
                [one] { $n } object
               *[other] { $n } objects
            }.
        [sources] Introspection level: Structure and sources, chosen automatically for { $n ->
                [one] { $n } object
               *[other] { $n } objects
            }.
       *[structure] Introspection level: Structure, chosen automatically for { $n ->
                [one] { $n } object
               *[other] { $n } objects
            }.
    }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-section = Schemas
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
hv-scope-summary = { $total ->
        [one] { $n } of { $total } schema
       *[other] { $n } of { $total } schemas
    }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $total (Number) - the total, e.g. 15000.
hv-scope-summary-mysql = { $total ->
        [one] { $total } database
       *[other] All { $total } databases
    }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-current-db = current database
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-other-db-hint = needs its own connection
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-pattern = Pattern
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-pattern-hint = @ is the current database or schema, * is all, {"{"}…{"}"} holds a regular expression, | separates clauses.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $pos (Number) - the 1-based character position of the error, e.g. 20.
hv-scope-pattern-error = Expected a schema name after “:” (character { NUMBER($pos, useGrouping: "false") }).
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-builtin-note = Built-in schemas come from the bundled catalog and are not listed here.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-mysql-note = Every database you can see is listed. Each one loads the first time you open it.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-all-dbs = All databases
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-all-dbs-title = Read every database on this server?
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-scope-all-dbs-body = { $n ->
        [one] This server has { $n } database. Reading it opens one more connection.
       *[other] This server has { $n } databases. Reading them opens one connection per database and can take a while on a large server.
    }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-scope-all-dbs-yes = { $n ->
        [one] Read { $n } database
       *[other] Read all { $n } databases
    }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-all-dbs-no = Keep the current database
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-encrypt = Encrypt cached metadata
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-encrypt-sub = The saved catalog is encrypted with a key kept in the Keychain. On by default for production.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-scope-encrypt-confirm-on = Turning this on deletes the cached metadata of { $name }. It is read again, encrypted, on the next connect.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-scope-encrypt-confirm-off = Turning this off deletes the encrypted cache of { $name }. From the next connect it is saved as a plain file only your account can read.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-encrypt-apply = Delete the cache and encrypt
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-encrypt-apply-off = Delete the cache and stop encrypting
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-encrypt-keep = Keep it as it is
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-connect-to-list = Connect to list the schemas. The pattern below still decides what is read.
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-metadata-mass-title = { $count ->
        [one] { $count } object in { $schema } is no longer visible
       *[other] { $count } objects in { $schema } are no longer visible
    }
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
hv-metadata-mass-body = They may have been dropped, or this login may have lost access to them. Removing them from the cache changes nothing on the server.
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-metadata-mass-more = and { $n } more
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
hv-metadata-mass-keep = Keep cached
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
hv-metadata-mass-remove = Remove from cache
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-metadata-mass-pending-tip = { $count ->
        [one] { $count } object disappeared in the last background check. Refresh { $schema } to review it; nothing was removed.
       *[other] { $count } objects disappeared in the last background check. Refresh { $schema } to review them; nothing was removed.
    }
# Surface 8 (Mass-deletion confirmation).
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-metadata-ns-dropped = { $schema } is no longer on the server, so it was removed from the explorer.
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
hv-metadata-refresh-ask = { $count ->
        [one] Refresh the database explorer to reflect { $count } changed object?
       *[other] Refresh the database explorer to reflect { $count } changed objects?
    }
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Variables:
#   $console (String) - the console's title, e.g. "orders-dev".
hv-metadata-refresh-ask-body = Your script in { $console } changed them.
# Surface 9 (Refresh after DDL). Refresh after DDL.
hv-metadata-refresh = Refresh
# Surface 9 (Refresh after DDL). Refresh after DDL.
hv-metadata-refresh-always = Always refresh
# Surface 9 (Refresh after DDL). Refresh after DDL.
hv-metadata-refresh-not-now = Not now
# Surface 9 (Refresh after DDL). Refresh after DDL inside a transaction (the pending stamp).
hv-metadata-after-commit = Refreshes after COMMIT
# Surface 9 (Refresh after DDL). Refresh after DDL inside a transaction (the pending stamp).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
hv-metadata-after-commit-tip = { $count ->
        [one] { $count } change from this transaction waits for COMMIT. ROLLBACK discards it and the explorer stays as it is.
       *[other] { $count } changes from this transaction wait for COMMIT. ROLLBACK discards them and the explorer stays as it is.
    }
# Surface 9 (Refresh after DDL). Refresh after DDL: dynamic SQL.
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-metadata-broad = Refreshing all of { $schema }
# Surface 9 (Refresh after DDL). Refresh after DDL: dynamic SQL.
# Variables:
#   $what (String) - the SQL keyword that made the change opaque, e.g. "EXECUTE".
hv-metadata-broad-body = The script ran dynamic SQL ({ $what }), so Hacivat can't tell which objects changed.
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-metadata-refreshed = { $count ->
        [one] Explorer refreshed { $count } object in { $schema }
       *[other] Explorer refreshed { $count } objects in { $schema }
    }
# Surface 9 (Refresh after DDL).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
hv-metadata-log-pending = { $count ->
        [one] Explorer refresh waits for COMMIT ({ $count } object)
       *[other] Explorer refresh waits for COMMIT ({ $count } objects)
    }
# Surface 10 (DDL view). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-tab-title = { $name } · DDL
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-qualification = Qualification
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-qual-auto = Auto
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-qual-never = Never
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-qual-always = Always
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-constraints = Constraints
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-constraints-inside-column = Inside column
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-constraints-inside-table = Inside table
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-constraints-after-table = After table
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-server = Definitions provided by server
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-grants = Grants
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-owner = Owner
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-options = Options
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-copy = Copy
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-open-console = Open in console
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-refresh = Refresh
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-source-generated = Generated from the catalog
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-source-server = Text from the server
# Surface 10 (DDL view). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-copied = Copied the DDL of { $name }
# Surface 10 (DDL view). One whole sentence per kind ($kind selector); never a kind word inserted.
# Variables:
#   $kind (String) - the object kind code, a selector, e.g. "index".
hv-ddl-view-unsupported = { $kind ->
        [domain] Hacivat can't show DDL for a domain yet.
        [extension] Hacivat can't show DDL for an extension yet.
        [operator] Hacivat can't show DDL for an operator yet.
        [policy] Hacivat can't show DDL for a row security policy yet.
        [rule] Hacivat can't show DDL for a rule yet.
        [type] Hacivat can't show DDL for a type yet.
       *[other] Hacivat can't show DDL for this kind of object yet.
    }
# Surface 10 (DDL view). One whole sentence per kind ($kind selector); never a kind word inserted.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-unsupported-body = { $name } stays in the explorer, and Copy qualified name works.
# Surface 10 (DDL view). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-loading = Reading the definition of { $name }…
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-grants-partial = Some grants could not be read, so the list may be incomplete.
# Surface 11 (Stale object). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-stale = { $name } changed on the server since it was loaded
# Surface 11 (Stale object). The DDL view tab.
hv-ddl-view-reload = Reload
# Surface 11 (Stale object). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-gone = { $name } no longer exists on the server
# Surface 11 (Stale object). The DDL view tab.
hv-ddl-view-gone-body = The text below is the last one loaded.
# Surface 11 (Stale object). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-unknown = Hacivat couldn't check whether { $name } changed on the server since it was loaded
# Surface 12 (Invisible characters and broken objects). The invisible-characters badge and its tooltip.
hv-explorer-invisible = invisible characters
# Surface 12 (Invisible characters and broken objects). The invisible-characters badge and its tooltip.
# Variables:
#   $what (String) - the SQL keyword that made the change opaque, e.g. "EXECUTE".
hv-explorer-invisible-tip = This name contains an invisible character: { $what }. Copy and DDL keep it exactly.
# Surface 12 (Invisible characters and broken objects). One whole sentence per kind ($kind selector); the name stays an isolated argument.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
#   $kind (String) - the object kind code, a selector, e.g. "index".
hv-explorer-broken = { $kind ->
        [check] Check constraint { $name } references a column that could not be resolved
        [foreign-key] Foreign key { $name } references a column that could not be resolved
        [index] Index { $name } references a column that could not be resolved
        [trigger] Trigger { $name } references a column that could not be resolved
       *[other] { $name } references a column that could not be resolved
    }
# Surface 12 (Invisible characters and broken objects). One whole sentence per kind ($kind selector); the name stays an isolated argument.
hv-explorer-broken-body = The column may have been dropped or renamed after the last read. Refresh the table to check.
# Surface 13 (Settings › Metadata).
hv-settings-nav-metadata = Metadata
# Surface 13 (Settings › Metadata).
hv-settings-section-metadata = METADATA
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-refresh = Refresh the explorer after DDL
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-always = Always
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-ask = Ask
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-never = Never
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-refresh-note = After a console runs a statement that changes the schema. Inside a transaction it waits for COMMIT.
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-check = Check for changes in the background
# Surface 13 (Settings › Metadata).
hv-metadata-check-auto = Automatic
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-check-note = Automatic checks local and dev connections every 10 minutes, and the others when you connect or press Refresh.
# Surface 13 (Settings › Metadata).
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-metadata-every-min = Every { $n } min
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-cache-conn = Cache per connection
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-cache-total = Cache for all connections
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-cache-note = Past the limit, the schemas loaded longest ago leave first. The current database always stays.
# Surface 13 (Settings › Metadata).
hv-metadata-unit-mb = MB
# Surface 13 (Settings › Metadata).
hv-metadata-unit-gb = GB
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-clear = Cached metadata
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
hv-metadata-clear = Clear cached metadata…
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
#   $size (String) - a size (an hv-metadata-size-mb message), e.g. "38.4 MB".
hv-metadata-clear-note = { $name } is the active console's connection. Its cached metadata uses { $size } on this computer.
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-metadata-clear-title = Clear cached metadata for { $name }?
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
hv-metadata-clear-body = The explorer reads it again from the server on the next connect. Nothing on the server changes.
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
hv-metadata-clear-btn = Clear
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-metadata-cleared = Cleared the cached metadata of { $name }
# Surface 13 (Settings › Metadata).
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-metadata-size-mb = { $n } MB
# Surface 14 (Long introspection). Progress of a pass in the explorer (surface 14: explorer only).
# Variables:
#   $fraction (Number) - the share read so far, from 0 to 1, e.g. 0.23.
hv-explorer-progress-meta = { NUMBER($fraction, style: "percent") }
# Surface 14 (Long introspection). Progress of a pass in the explorer (surface 14: explorer only).
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
hv-explorer-progress-tip = { $total ->
        [one] Reading { $schema }: the columns of { $n } of { $total } table so far. The explorer stays usable meanwhile.
       *[other] Reading { $schema }: the columns of { $n } of { $total } tables so far. The explorer stays usable meanwhile.
    }

## m4-tNN-ddl-warnings (hacivat-core.ftl: the block of the task that emits DDL warnings, ruling 20)
# Surface 10 (DDL view). Shown over MySQL server text that contains executable comments (R-M4-24).
ddl-warn-executable-comment = This text contains MySQL executable comments (/*! … */). MySQL runs what is inside them as SQL.
```

### Proposed FTL, Turkish (`locales/tr/hacivat-ui.ftl`)

```ftl
## m4-t01b-ui

# Surface 1 (Explorer tree).
# Review: PO
hv-explorer-tree-label = Veritabanı gezgini
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-tables = TABLOLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-views = GÖRÜNÜMLER
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-matviews = SOMUT GÖRÜNÜMLER
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-functions = FONKSİYONLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-procedures = PROSEDÜRLER
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-sequences = SEKANSLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-types = TİPLER
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-events = OLAYLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-columns = SÜTUNLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-keys = ANAHTARLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-foreign-keys = YABANCI ANAHTARLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-indexes = İNDEKSLER
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-checks = DENETİMLER
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-triggers = TETİKLEYİCİLER
# Surface 1 (Explorer tree). The Built-in group of a database.
# Review: PO
hv-explorer-builtin = Yerleşik
# Surface 1 (Explorer tree). The Built-in group of a database.
# Variables:
#   $version (String) - the bundled catalog's server version, e.g. "17".
# Review: PO
hv-explorer-builtin-badge = sürüm { $version }
# Surface 1 (Explorer tree). The Built-in group of a database.
# Variables:
#   $version (String) - the bundled catalog's server version, e.g. "17".
# Review: PO
hv-explorer-builtin-tip = Yerleşik katalog (sürüm { $version }): sunucunun kendi nesneleri; Hacivat ile gelir, bağlantı olmadan okunur.
# Surface 1 (Explorer tree). The database row's schema scope count (N of M).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-explorer-scope-count = { $n } / { $total }
# Surface 1 (Explorer tree). The database row's schema scope count (N of M).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-explorer-scope-count-tip = { $total } şemadan { $n } tanesi kapsamda. Kapsamı düzenle… hangilerinin okunacağını değiştirir.
# Surface 1 (Explorer tree).
# Review: PO
hv-explorer-add-scope = Kapsama ekle
# Surface 1 (Explorer tree).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-explorer-out-of-scope-tip = { $name } bu bağlantının kapsamında değil, bu yüzden okunmuyor.
# Surface 1 (Explorer tree).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-explorer-out-of-scope-db-tip = { $name } bu bağlantının kapsamında değil, bu yüzden okunmuyor. Okumak bir bağlantı daha açar.
# Surface 1 (Explorer tree).
# Variables:
#   $rows (Number) - the server's row estimate, e.g. 48213.
# Review: PO
hv-explorer-row-estimate-tip = Yaklaşık { $rows } satır (sunucunun tahmini)
# Surface 1 (Explorer tree).
# Review: PO
hv-explorer-loads-on-open = Açtığınızda yüklenir
# Surface 2 (Loading states).
# Review: PO
hv-explorer-loading = Yükleniyor…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
# Review: PO
hv-explorer-phase-names = Adlar okunuyor…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
# Review: PO
hv-explorer-phase-columns = Sütunlar okunuyor…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
# Review: PO
hv-explorer-phase-constraints = Anahtarlar ve indeksler okunuyor…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
# Review: PO
hv-explorer-phase-sources = Tanımlar okunuyor…
# Surface 2 (Loading states). Progress of a pass in the explorer (surface 14: explorer only).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-explorer-progress-count = { $n }/{ $total }
# Surface 2 (Loading states).
# Review: PO
hv-explorer-waiting-pass = Sütunlar bekleniyor
# Surface 2 (Loading states). The partial-load badge and its tooltip.
# Review: PO
hv-explorer-partial = eksik
# Surface 2 (Loading states). The partial-load badge and its tooltip.
# Review: PO
hv-explorer-partial-title = Bazı nesneler okunamadı
# Surface 2 (Loading states). The partial-load badge and its tooltip.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-explorer-partial-body = Bu kullanıcı { $schema } içindeki { $n } fonksiyonun tanımını okuyamıyor. Adlarıyla listelenirler; tanımları eksiktir.
# Surface 2 (Loading states). A namespace whose catalog query was refused.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-explorer-failed = { $name } okunamadı
# Surface 2 (Loading states). A namespace whose catalog query was refused.
# Review: PO
hv-explorer-failed-body = Sunucu katalog sorgusunu reddetti. Yeniden dene ile tekrar okunur.
# Surface 2 (Loading states).
# Review: PO
hv-explorer-retry = Yeniden dene
# Surface 3 (Cached and offline). A profile shown from its cache while offline.
# Variables:
#   $date (DateTime) - when the cache was saved, e.g. 2026-11-17 14:05.
# Review: PO
hv-explorer-cached = { DATETIME($date, dateStyle: "medium", timeStyle: "short") } tarihli önbellekteki üst veri gösteriliyor
# Surface 3 (Cached and offline). A profile shown from its cache while offline.
# Review: PO
hv-explorer-cached-meta = önbellek
# Surface 3 (Cached and offline). A profile shown from its cache while offline.
# Variables:
#   $date (DateTime) - when the cache was saved, e.g. 2026-11-17 14:05.
# Review: PO
hv-explorer-cached-tip = Bağlı değil. Ağaç, { DATETIME($date, dateStyle: "medium", timeStyle: "short") } tarihinde kaydedilen kopyadır; sunucuyla karşılaştırmak için bağlanın.
# Surface 3 (Cached and offline).
# Review: PO
hv-explorer-stale = güncel değil
# Surface 3 (Cached and offline).
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-explorer-stale-tip = { $schema } okunduktan sonra sunucuda değişti. Değişiklikleri yüklemek için yenileyin.
# Surface 3 (Cached and offline).
# Review: PO
hv-explorer-encrypted-tip = Önbellekteki üst veri şifreli. Bu bağlantıyı açmak anahtarını Anahtar Zinciri'nden okur.
# Surface 3 (Cached and offline).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-explorer-memory-only = Anahtar Zinciri kullanılamıyor; { $name } üst verisi uygulamadan çıkana dek yalnız bellekte tutulur.
# Surface 4 (Filter field). The explorer filter field.
# Review: PO
hv-explorer-filter-objects = Nesnelerde ara
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-explorer-filter-count = { $n } eşleşme
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-explorer-filter-group-count = { $n } / { $total }
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-explorer-filter-unloaded = Yalnız yüklenmiş adlarda arar. { $n } tablonun sütunları henüz yüklenmedi.
# Surface 4 (Filter field). The explorer filter field.
# Review: PO
hv-explorer-filter-none = Eşleşen nesne yok
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $query (String) - the filter text as typed, e.g. "müş".
# Review: PO
hv-explorer-filter-none-body = Yüklenmiş adlardan hiçbiri “{ $query }” ile eşleşmiyor.
# Surface 4 (Filter field). The explorer filter field.
# Review: PO
hv-explorer-filter-clear = Filtreyi temizle
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-refresh = Yenile
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-edit-scope = Kapsamı düzenle…
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-level = Okuma düzeyi
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-clear-cache = Önbellekteki üst veriyi temizle…
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-open-data = Veriyi aç
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-view-ddl = DDL'i göster
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-view-table-ddl = Tablonun DDL'ini göster
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-copy-name = Tam adı kopyala
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-copy-plain = Adı kopyala
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-hide-like = Benzer nesneleri gizle
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-show-hidden = Gizli nesneleri göster
# Surface 5 (Context menus).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $pattern (String) - the view filter's pattern, e.g. "yedek_fatura_*".
# Review: PO
hv-explorer-hidden-tip = { $n } nesne bir görünüm filtresiyle gizlendi ({ $pattern }). Bu grubun menüsündeki Gizli nesneleri göster filtreyi kaldırır.
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-copy-ddl = DDL'i kopyala
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-modify-table = Tabloyu değiştir…
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-modify-view = Görünümü değiştir…
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-modify-column = Sütunu değiştir…
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-modify-tip = Sonraki bir sürümde gelecek
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-remove-scope = Kapsamdan çıkar
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-expand-all = Tüm alt öğeleri aç
# Surface 5 (Context menus). The tree-wide command (palette, ⌘←); node menus use hv-explorer-menu-collapse-children.
# Review: PO
hv-explorer-menu-collapse-all = Tümünü kapat
# Surface 5 (Context menus).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-explorer-copied = { $name } kopyalandı
# Surface 5 (Context menus).
# Variables:
#   $conn (String) - the connection name, e.g. "orders-dev".
# Review: PO
hv-explorer-open-data-note = { $conn } üzerinde bir konsol açar ve şunu çalıştırır:
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-collapse-children = Tüm alt öğeleri kapat
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-auto = Otomatik
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-names = Yalnız adlar
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-structure = Yapı
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-sources = Yapı ve kaynaklar
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Variables:
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
# Review: PO
hv-explorer-level-auto-hint = { $level ->
        [names] şu an Yalnız adlar
        [sources] şu an Yapı ve kaynaklar
       *[structure] şu an Yapı
    }
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-names-hint = en hızlısı, sütunsuz
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-structure-hint = sütunlar, anahtarlar, indeksler
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-sources-hint = ve görünüm, fonksiyon metinleri
# Surface 6 (Introspection level). The level pill's word.
# Review: PO
hv-explorer-pill-names = Adlar
# Surface 6 (Introspection level). The level pill's word.
# Review: PO
hv-explorer-pill-structure = Yapı
# Surface 6 (Introspection level). The level pill's word.
# Review: PO
hv-explorer-pill-sources = Kaynaklar
# Surface 6 (Introspection level). The level pill's tooltip and accessible name: one whole sentence per level.
# Variables:
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
# Review: PO
hv-explorer-pill-tip-explicit = { $level ->
        [names] Okuma düzeyi: Yalnız adlar, burada ayarlandı. Alttaki şemalar bunu devralır.
        [sources] Okuma düzeyi: Yapı ve kaynaklar, burada ayarlandı. Alttaki şemalar bunu devralır.
       *[structure] Okuma düzeyi: Yapı, burada ayarlandı. Alttaki şemalar bunu devralır.
    }
# Surface 6 (Introspection level). The level pill's tooltip and accessible name: one whole sentence per level.
# Variables:
#   $parent (String) - the database or schema the level comes from, e.g. "orders".
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
# Review: PO
hv-explorer-pill-tip-inherited = { $level ->
        [names] Okuma düzeyi: Yalnız adlar; { $parent } öğesinden devralındı.
        [sources] Okuma düzeyi: Yapı ve kaynaklar; { $parent } öğesinden devralındı.
       *[structure] Okuma düzeyi: Yapı; { $parent } öğesinden devralındı.
    }
# Surface 6 (Introspection level). The level pill's tooltip and accessible name: one whole sentence per level.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
# Review: PO
hv-explorer-pill-tip-auto = { $level ->
        [names] Okuma düzeyi: Yalnız adlar; { $n } nesne için otomatik seçildi.
        [sources] Okuma düzeyi: Yapı ve kaynaklar; { $n } nesne için otomatik seçildi.
       *[structure] Okuma düzeyi: Yapı; { $n } nesne için otomatik seçildi.
    }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-section = Şemalar
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-scope-summary = { $total } şemadan { $n }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-scope-summary-mysql = { $total } veritabanının tümü
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-current-db = geçerli veritabanı
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-other-db-hint = ayrı bağlantı gerekir
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-pattern = Desen
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-pattern-hint = @ geçerli veritabanı ya da şema, * tümü, {"{"}…{"}"} bir düzenli ifade içerir; | ifadeleri ayırır.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $pos (Number) - the 1-based character position of the error, e.g. 20.
# Review: PO
hv-scope-pattern-error = “:” işaretinden sonra bir şema adı bekleniyordu ({ NUMBER($pos, useGrouping: "false") }. karakter).
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-builtin-note = Yerleşik şemalar paketle gelen katalogdan okunur, burada listelenmez.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-mysql-note = Görebildiğiniz her veritabanı listelenir. Her biri ilk açtığınızda yüklenir.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-all-dbs = Tüm veritabanları
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-all-dbs-title = Bu sunucudaki tüm veritabanları okunsun mu?
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-scope-all-dbs-body = Bu sunucuda { $n } veritabanı var. Hepsini okumak her veritabanı için bir bağlantı açar ve büyük bir sunucuda zaman alabilir.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-scope-all-dbs-yes = { $n } veritabanının tümünü oku
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-all-dbs-no = Geçerli veritabanında kal
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-encrypt = Önbellekteki üst veriyi şifrele
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-encrypt-sub = Kaydedilen katalog, Anahtar Zinciri'nde tutulan bir anahtarla şifrelenir. Üretimde varsayılan olarak açıktır.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-scope-encrypt-confirm-on = Bunu açmak { $name } için önbellekteki üst veriyi siler. Bir sonraki bağlanışta şifreli olarak yeniden okunur.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-scope-encrypt-confirm-off = Bunu kapatmak { $name } için şifreli önbelleği siler. Bir sonraki bağlanıştan sonra yalnız hesabınızın okuyabildiği düz bir dosyaya kaydedilir.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-encrypt-apply = Önbelleği sil ve şifrele
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-encrypt-apply-off = Önbelleği sil ve şifrelemeyi bırak
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-encrypt-keep = Olduğu gibi bırak
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-connect-to-list = Şemaları listelemek için bağlanın. Neyin okunacağına yine aşağıdaki desen karar verir.
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-mass-title = { $schema } içindeki { $count } nesne artık görünmüyor
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Review: PO
hv-metadata-mass-body = Silinmiş olabilirler ya da bu kullanıcı onlara erişimini kaybetmiş olabilir. Önbellekten kaldırmak sunucuda hiçbir şeyi değiştirmez.
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-metadata-mass-more = ve { $n } tane daha
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Review: PO
hv-metadata-mass-keep = Önbellekte tut
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Review: PO
hv-metadata-mass-remove = Önbellekten kaldır
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-mass-pending-tip = Son arka plan denetiminde { $count } nesne kayboldu. İncelemek için { $schema } şemasını yenileyin; hiçbir şey kaldırılmadı.
# Surface 8 (Mass-deletion confirmation).
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-ns-dropped = { $schema } artık sunucuda yok, bu yüzden gezginden kaldırıldı.
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
# Review: PO
hv-metadata-refresh-ask = Gezgin, değişen { $count } nesneyi gösterecek şekilde yenilensin mi?
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Variables:
#   $console (String) - the console's title, e.g. "orders-dev".
# Review: PO
hv-metadata-refresh-ask-body = { $console } içindeki betiğiniz bunları değiştirdi.
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Review: PO
hv-metadata-refresh = Yenile
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Review: PO
hv-metadata-refresh-always = Her zaman yenile
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Review: PO
hv-metadata-refresh-not-now = Şimdi değil
# Surface 9 (Refresh after DDL). Refresh after DDL inside a transaction (the pending stamp).
# Review: PO
hv-metadata-after-commit = COMMIT sonrası yenilenir
# Surface 9 (Refresh after DDL). Refresh after DDL inside a transaction (the pending stamp).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
# Review: PO
hv-metadata-after-commit-tip = Bu transaction'daki { $count } değişiklik COMMIT'i bekliyor. ROLLBACK bunları atar ve gezgin olduğu gibi kalır.
# Surface 9 (Refresh after DDL). Refresh after DDL: dynamic SQL.
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-broad = { $schema } tamamen yenileniyor
# Surface 9 (Refresh after DDL). Refresh after DDL: dynamic SQL.
# Variables:
#   $what (String) - the SQL keyword that made the change opaque, e.g. "EXECUTE".
# Review: PO
hv-metadata-broad-body = Betik dinamik SQL çalıştırdı ({ $what }); Hacivat hangi nesnelerin değiştiğini bilemiyor.
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-refreshed = Gezgin { $schema } içinde { $count } nesneyi yeniledi
# Surface 9 (Refresh after DDL).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
# Review: PO
hv-metadata-log-pending = Gezgin yenilemesi COMMIT'i bekliyor ({ $count } nesne)
# Surface 10 (DDL view). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-tab-title = { $name } · DDL
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-qualification = Nitelendirme
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-qual-auto = Otomatik
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-qual-never = Hiçbir zaman
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-qual-always = Her zaman
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-constraints = Kısıtlar
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-constraints-inside-column = Sütunun içinde
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-constraints-inside-table = Tablonun içinde
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-constraints-after-table = Tablodan sonra
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-server = Sunucunun verdiği tanımlar
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-grants = Yetkiler
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-owner = Sahip
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-options = Seçenekler
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-copy = Kopyala
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-open-console = Konsolda aç
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-refresh = Yenile
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-source-generated = Katalogdan üretildi
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-source-server = Sunucudan gelen metin
# Surface 10 (DDL view). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-copied = { $name } DDL'i kopyalandı
# Surface 10 (DDL view). One whole sentence per kind ($kind selector); never a kind word inserted.
# Variables:
#   $kind (String) - the object kind code, a selector, e.g. "index".
# Review: PO
hv-ddl-view-unsupported = { $kind ->
        [domain] Hacivat domain'lerin DDL'ini henüz gösteremiyor.
        [extension] Hacivat eklentilerin DDL'ini henüz gösteremiyor.
        [operator] Hacivat işleçlerin DDL'ini henüz gösteremiyor.
        [policy] Hacivat satır güvenliği ilkelerinin DDL'ini henüz gösteremiyor.
        [rule] Hacivat kuralların DDL'ini henüz gösteremiyor.
        [type] Hacivat tiplerin DDL'ini henüz gösteremiyor.
       *[other] Hacivat bu tür nesnelerin DDL'ini henüz gösteremiyor.
    }
# Surface 10 (DDL view). One whole sentence per kind ($kind selector); never a kind word inserted.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-unsupported-body = { $name } gezginde kalır; Tam adı kopyala çalışır.
# Surface 10 (DDL view). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-loading = { $name } tanımı okunuyor…
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-grants-partial = Bazı yetkiler okunamadı; liste eksik olabilir.
# Surface 11 (Stale object). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-stale = { $name } yüklendikten sonra sunucuda değişti
# Surface 11 (Stale object). The DDL view tab.
# Review: PO
hv-ddl-view-reload = Yeniden yükle
# Surface 11 (Stale object). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-gone = { $name } artık sunucuda yok
# Surface 11 (Stale object). The DDL view tab.
# Review: PO
hv-ddl-view-gone-body = Aşağıdaki metin en son yüklenen metindir.
# Surface 11 (Stale object). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-unknown = Hacivat, { $name } yüklendikten sonra sunucuda değişip değişmediğini denetleyemedi
# Surface 12 (Invisible characters and broken objects). The invisible-characters badge and its tooltip.
# Review: PO
hv-explorer-invisible = görünmez karakter
# Surface 12 (Invisible characters and broken objects). The invisible-characters badge and its tooltip.
# Variables:
#   $what (String) - the SQL keyword that made the change opaque, e.g. "EXECUTE".
# Review: PO
hv-explorer-invisible-tip = Bu ad görünmez bir karakter içeriyor: { $what }. Kopyalama ve DDL onu aynen korur.
# Surface 12 (Invisible characters and broken objects). One whole sentence per kind ($kind selector); the name stays an isolated argument.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
#   $kind (String) - the object kind code, a selector, e.g. "index".
# Review: PO
hv-explorer-broken = { $kind ->
        [check] { $name } denetim kısıtı, çözümlenemeyen bir sütuna başvuruyor
        [foreign-key] { $name } yabancı anahtarı, çözümlenemeyen bir sütuna başvuruyor
        [index] { $name } indeksi, çözümlenemeyen bir sütuna başvuruyor
        [trigger] { $name } tetikleyicisi, çözümlenemeyen bir sütuna başvuruyor
       *[other] { $name }, çözümlenemeyen bir sütuna başvuruyor
    }
# Surface 12 (Invisible characters and broken objects). One whole sentence per kind ($kind selector); the name stays an isolated argument.
# Review: PO
hv-explorer-broken-body = Sütun son okumadan sonra silinmiş ya da yeniden adlandırılmış olabilir. Denetlemek için tabloyu yenileyin.
# Surface 13 (Settings › Metadata).
# Review: PO
hv-settings-nav-metadata = Üst veri
# Surface 13 (Settings › Metadata).
# Review: PO
hv-settings-section-metadata = ÜST VERİ
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-refresh = DDL sonrası gezgini yenile
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-always = Her zaman
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-ask = Sor
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-never = Hiçbir zaman
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-refresh-note = Bir konsol şemayı değiştiren bir ifade çalıştırdıktan sonra. Transaction içindeyse COMMIT'i bekler.
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-check = Değişiklikleri arka planda denetle
# Surface 13 (Settings › Metadata).
# Review: PO
hv-metadata-check-auto = Otomatik
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-check-note = Otomatik: yerel ve geliştirme bağlantıları 10 dakikada bir, diğerleri bağlandığınızda ya da Yenile'ye bastığınızda denetlenir.
# Surface 13 (Settings › Metadata).
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-metadata-every-min = { $n } dakikada bir
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-cache-conn = Bağlantı başına önbellek
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-cache-total = Tüm bağlantılar için önbellek
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-cache-note = Sınır aşılınca en uzun süre önce yüklenen şemalar önce çıkar. Geçerli veritabanı her zaman kalır.
# Surface 13 (Settings › Metadata).
# Review: PO
hv-metadata-unit-mb = MB
# Surface 13 (Settings › Metadata).
# Review: PO
hv-metadata-unit-gb = GB
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-clear = Önbellekteki üst veri
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Review: PO
hv-metadata-clear = Önbellekteki üst veriyi temizle…
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
#   $size (String) - a size (an hv-metadata-size-mb message), e.g. "38.4 MB".
# Review: PO
hv-metadata-clear-note = { $name } etkin konsolun bağlantısı. Önbellekteki üst verisi bu bilgisayarda { $size } kullanıyor.
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-metadata-clear-title = { $name } için önbellekteki üst veri temizlensin mi?
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Review: PO
hv-metadata-clear-body = Gezgin bunu bir sonraki bağlanışta sunucudan yeniden okur. Sunucuda hiçbir şey değişmez.
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Review: PO
hv-metadata-clear-btn = Temizle
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-metadata-cleared = { $name } için önbellekteki üst veri temizlendi
# Surface 13 (Settings › Metadata).
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-metadata-size-mb = { $n } MB
# Surface 14 (Long introspection). Progress of a pass in the explorer (surface 14: explorer only).
# Variables:
#   $fraction (Number) - the share read so far, from 0 to 1, e.g. 0.23.
# Review: PO
hv-explorer-progress-meta = { NUMBER($fraction, style: "percent") }
# Surface 14 (Long introspection). Progress of a pass in the explorer (surface 14: explorer only).
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-explorer-progress-tip = { $schema } okunuyor: şu ana dek { $total } tablonun { $n } tanesinin sütunları. Bu sırada gezgin kullanılabilir.

## m4-tNN-ddl-warnings (hacivat-core.ftl: the block of the task that emits DDL warnings, ruling 20)
# Surface 10 (DDL view). Shown over MySQL server text that contains executable comments (R-M4-24).
# Review: PO
ddl-warn-executable-comment = Bu metin MySQL çalıştırılabilir yorumları (/*! … */) içeriyor. MySQL bunların içindekini SQL olarak çalıştırır.
```
