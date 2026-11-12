# 09 — DDL Generation, Object Modification and Schema Compare (DataGrip 2024.3.5)

> Part of the Hacivat research library. Conventions: `../README.md`. Related: 01 (architecture: source store `storage_v2/_src_`, external-data files), 04 (introspection, *Load Sources*, outdated-DDL check, incremental source queries), 06 (SQL language, DDL parsing for DDL data sources, Rename/Extract refactorings, live/context templates), 07 (execution of generated scripts, transaction modes), 08 (data compare, grid Submit/pending changes — distinct from DDL Submit), 10 (diagrams, native dump tools, Local History), 11 (settings OT-1..OT-5, Code Style › SQL › DDL and Code Generation, registry `database.schema.diff.*`, `database.script.generator.*`), 13 (family DDL, DDL-I-01..25). The introspected model (object kinds, properties, dialect metamodels) is covered by docs 00–05.

---

## 1. Scope and sources

**Scope.** How DataGrip turns user intent into DDL: the Create/Modify dialogs (per object kind, every documented field), the auxiliary dialogs (Confirm Drop, Rename, Truncate, Recompile, Modify Comment, Modify Grants, Batch Modify/Add Indexes), the SQL Generator tool window and the one-shot "Generate DDL to …" actions, editing stored object source in place (DDL editor, Database Changes tool window, Object Migration dialog, conflict handling), Compare Structure and the Migration dialog (every option and viewer control), DDL data sources, DDL mappings and linked data sources, file layouts and layout scripts, scripted extensions and context templates, virtual objects, naming templates, related settings and registry keys — plus the algorithms Hacivat needs: model diff, rename detection, implicit-object classification, source normalization, minimal-change script generation, dependency ordering with cycle breaking, ALTER-vs-recreate decisions, constraint placement, qualification, source-edit round trip and DDL-file sync.

**Sources (legitimate only, no decompilation).**

| Source | Used for | License |
|---|---|---|
| Offline help (DataGripHelp plugin, Oct 2024), text under `.refs/scratch-extracts/help/txt/`: `create-and-modify-dialogs`, `columns`, `primary-keys`, `foreign-keys`, `indexes`, `views`, `schemas`, `database-users-and-roles`, `virtual-columns`, `virtual-views`, `virtual-foreign-keys`, `confirm-drop-dialog`, `rename-dialog-for-a-table-or-column`, `refactoring-source-code`, `export-data` (SQL Generator), `database-explorer` (action table), `modifying-source-code-of-database-objects`, `update-source-code-of-database-objects`, `submitting-and-reverting-changes`, `database-changes-tool-window`, `schema-comparison-and-migration`, `ddl-data-sources`, `ddl-mappings`, `create-ddl-data-source-using-sql-files`, `databases-in-the-version-control-system`, `code-style-sql-code-generation`, `code-style-sql-ddl`, `settings-tools-database-other`, `generating-code`, `context-live-templates` | documented behavior, field lists | JetBrains docs |
| Help screenshots in `plugins/DataGripHelp/lib/help-datagrip-assets.jar` (viewed, not redistributed): `db_modify_table`, `db_modify_column`, `db_modify_create_dialogs_preview_pane`, `db_modify_create_dialogs_context_menu_objects_editor`, `db_create_user_or_role`, `db_confirm_drop_dialog`, `db_sql_generator`, `db_sql_generator_output_settings`, `db_create-new-layout`, `db_schema_migration_dialog`, `db_differences_between_object_properties`, `db_tutorial_migration_script_complex` | field labels, defaults, control layout | JetBrains docs |
| `DatabaseBundle` (en): `scriptGen.*` (112 keys), `TableDialogSettings.*`, `migration.*`, `tab.title.migration.*`, `action.ignore.*`, `changed.in.database.*`, `DatabaseTable*Editor.*`, `dialog.title.batch.*`, DDL-mapping and dump messages | exact labels and messages (short labels quoted) | Proprietary |
| Action inventory (`.refs/scratch-extracts/actions.txt`) | action ids and labels (`DatabaseView.Ddl.*`, `DatabaseView.SqlGenerator`, `DDL.Editor.Regenerate`, `DatabaseView.ClearSrcStorage`, `recompile`, …) | Proprietary (names) |
| Class-name inventory (`.refs/datagrip-extract/classes/database-plugin.txt`): `dialects/*/generator/producers/*` (1,282 producer classes), generator core (`Scenario`, `Effect`, `LinkedPriorityQueue`, `ObjectPathTrie`, `PreOrderComparator`, `Reviser`, capability invocators), `diff/*`, `migration/*` (`DbMigrationDialog`, `DbMigrationOptions[Service]`, `DbDiffElementFilter`, `DiffStatus`), `applier/*` (model appliers), `build/De*Builder`, `srcStorage/*` (`DbSrcChangesTracker`, `DbSrcChangeList`, `DbConflictChange`, `DbContentRevision`, `DbSrcFileStatus`), `backend/DbSrc*` (zip-backed source store), `ddl/*` (`DbModifyObjectDialog`, `ModifyGrantsAction`) | architecture inference | names only |
| `dbplugin/extensions/schema.layouts/*.groovy` (5) and `extensions/schema/Generate POJOs.groovy` | layout scripting contract | Proprietary — **described only, not copied** |
| `inspectionDescriptions/SqlStorageInspection.html` | outdated-source detection scope | Proprietary (described) |
| Registry `META-INF/DatabaseRegistryKeys.xml`; searchable options (`so_resolved.txt`, page `database.other`) | defaults, setting inventory | Proprietary |
| migra (Apache-2.0), Skeema (Apache-2.0), Atlas (Apache-2.0), pg_dump `pg_dump_sort.c` (PostgreSQL License), SQLite ALTER TABLE docs | prior art for diff rules, dependency sorting, rebuild procedure | as noted |

---

## 2. Executive summary

1. Every schema change in DataGrip is a **model-to-model transition**: a dialog, an edited DDL text, a compare, or a DDL-mapping "apply" produces a *target model*; a dialect **script generator** computes the SQL that turns the *current* model into it. Generators are split into ~1,282 **producer** classes (Create/Alter/Drop × element kind, dialect overrides) that declare **effects** and are scheduled by a priority-queue **Scenario**. [Observed: names; Inferred: semantics]
2. **One dialog for Create and Modify** (`DbModifyObjectDialog`): left *structure tree* (family nodes with counts), right *objects editor* with **one tab per opened object** (pin/close/revert per tab), bottom **Preview** with editable SQL, a settings gear and *Open query in console*. Fields are **generated from the introspected property metamodel** since 2022.1, so the field set is per DBMS. [Observed: help, screenshots]
3. The preview contains **only the delta**: defaults are omitted (e.g. creating a PG role emits only the non-default flags), comments become separate `COMMENT ON` statements, and editing the preview text **updates the structure** ("Updating structure from preview"). [Observed: screenshots, bundle progress title]
4. The preview gear exposes dialog-level script settings: **Action** (Modify existing objects / Create modified objects / Create all objects), **Script** (Open in editor / Execute in database / Replace existing DDL) and **Rename in** (Comments and strings / Text occurrences / Loaded sources). [Observed: `TableDialogSettings.*`; Inferred: semantics]
5. **SQL Generator** (Ctrl+Alt+G) offers 14 categories (Creation script completely / before data import / after data import, Definitions 'as is', Definitions provided by RDBMS server, Enable/Disable constraints and triggers, Recompile, Refresh materialized views, Execute procedures and functions, Delete data, Drop objects, Truncate table, Alter elements) and ~45 options; output to clipboard, file (with **layout**) or console. [Observed]
6. Observed SQL Generator defaults: Qualify = **Auto**, Place constraints = **Inside column**, every checkbox (surrogate names, Skip DEFINER, CREATE OR REPLACE, IF NOT EXISTS, sequence next value, Skip grants) **off**; File Output layout starts as `<unknown>`. Confirm Drop defaults: Qualify Auto, IF EXISTS off, CASCADE off, **Reformat on**. [Observed: screenshots]
7. **Go to DDL** (Ctrl+B) opens an *unqualified* editable DDL; edits are tracked as **local versions** with gutter markers; the **Database Changes** tool window lists them (Submit Ctrl+K, Roll back, Show Diff Ctrl+D, Group by, Expand/Collapse). Submit opens **Object Migration** (e.g. a changed function signature becomes DROP + CREATE). [Observed]
8. Conflict model: object-name colors black/blue/red; *Possible Loss of Changes* → **Force Refactoring** / **Abort Refactoring and Synchronize**; then *Revert local changes* / *Keep local changes* / *Merge* (3-way). Outdated-source detection works for PostgreSQL, SQL Server, Oracle, Sybase ASE. [Observed: help, inspection description]
9. The source-edit machinery reuses the IntelliJ **VCS change framework** (change lists, content revisions, file statuses, conflict changes) over a virtual file system of object sources stored in zip files. [Observed: class names; Inferred: reuse]
10. **Compare Structure** (Ctrl+D) on two objects of the same kind (any data sources, including DDL data sources) opens the **Migration** dialog: side-by-side table-tree (Origin | include checkboxes | Target), colors green/blue/red, tabs *Script Preview*, *Object Properties Diff* (Property/Origin/Target), *DDL Diff* (full IntelliJ diff viewer); options Ignore Order / Case / Implicit Objects / Source Code / Owner; Swap; Show identical; Execute or *Open in New Query Console*. Script makes **Target equal to Origin**. [Observed]
11. Comparing schemas with **different names** (e.g. `public` vs `testing`) works: schema-qualified references inside defaults (`nextval('testing.seq')` vs `nextval('seq')`) are treated as equal. [Observed: screenshot — no change checkbox on that column]
12. Help for *Show preview of valid script when updating source text* is **self-contradictory** about its default (one page: "by default not shown"; settings page: "by default displayed"). [Observed — see §7]
13. **DDL data sources** = SQL files parsed with a chosen dialect (scopes, auto sync, extend a real data source, default schema names); **DDL mappings** link a real data source to a DDL data source with a schema scope and offer *Update/Apply from*, *Apply to* (Migration dialog), *Dump to* (warns before deleting files of missing objects). Not supported for MongoDB. [Observed]
14. **Layouts**: 5 bundled Groovy layout scripts; only *File per object with order* honours dependency order (3-digit prefix); the other four set "ignore dependencies". Names sanitize `/` to the word `slash`; objects without a database go under an `anonymous/` folder in the by-database layout. Users can add layouts (*New Layout* name dialog). [Observed]
15. **Naming templates** (Code Style › SQL › Code Generation) for PK/FK/index with `{var}` and `{flag?yes:no}`; **Auto Generate** on every Name field. [Observed]
16. Registry: `database.schema.diff.table.view=true`, `database.schema.diff.show.root=true`, `database.legacy.schema.diff=false`, `database.script.generator.separate.disable=false`, `database.script.generator.show.dump.option=false`, `database.migration.source.revert.on.success=true`, `database.automatically.load.sources=true`. [Observed]
17. Hacivat: crates `schema-model`, `schema-diff` (ALG-DDL-01/10/11/12), `ddl-gen` (ALG-DDL-03/07/08/13/14), `ddl-schedule` (ALG-DDL-02), `ddl-files` (ALG-DDL-05/06/15), `src-edit` (ALG-DDL-09). Karagöz authorizes and audits each migration operation individually.

---

## 3. Detailed findings

### 3.1 Entry points and action inventory

[Observed: `actions.txt`, `DatabaseBundle`, help `database-explorer`]

| Action (id) | Label / shortcut | What it does |
|---|---|---|
| `GenerateGroup` › *New* (`DatabaseView.AddActionGroupPopup`) | New › <object> (Alt+Insert in explorer and editor) | opens the Create dialog; entities listed in help: function, procedure, index, rule, trigger, view, materialized view, schema, database, role, user, sequence, enum, domain, composite type, foreign table, aggregate, operator, collation, foreign data wrapper, access method, extension table (PG list; per-DBMS) |
| `DatabaseView.Ddl.AlterObject` | Modify Object… / *Modify <kind>* (Ctrl+F6) | Modify dialog |
| `ModifyObject` | Modify Object… (Old UI) | legacy dialog kept for compatibility |
| `DatabaseView.Ddl.Add{Table,Column,PrimaryKey,ForeignKey,Index}` | "(Old UI)" variants | legacy per-kind dialogs |
| `DatabaseView.BatchModifyIndices` | Modify Indexes | *Batch Modify Indexes* dialog |
| `DatabaseView.Ddl.BatchAddIndices` | Indexes per Column (Old UI) | *Batch Add Indexes* — one index per selected column, names from a pattern (`{table}`, `{columns}`) |
| `DatabaseView.Ddl.CommentOnObject` | Modify Comment… | small comment editor with *Set NULL* |
| `DatabaseView.Ddl.ModifyGrants` | Modify Grants… | grants-only dialog ("Modify Grants of …") |
| `DatabaseView.Ddl.DropForeignKey` / `DropPrimaryKey` | Drop Foreign Key / Drop Primary Key | from a column node |
| `DatabaseView.DropAction` | Drop… (Delete) | *Confirm Drop* dialog |
| `DatabaseView.Tools.TruncateTableAction` | SQL Scripts › Truncate… | *Truncate Table* dialog (generator category Truncate) |
| `recompile`, `recompile.selected.objects` | SQL Scripts › Recompile… | *Recompile* dialog (With DEBUG / REUSE SETTINGS; Oracle) |
| `RefactoringMenu` › Rename | Shift+F6 | Rename dialog for table/column with SQL preview |
| `DatabaseView.SqlGenerator` | SQL Scripts › SQL Generator… (Ctrl+Alt+G) | SQL Generator tool window |
| `DatabaseView.CopyDdlAction` | Generate DDL to Clipboard (Ctrl+Alt+Shift+G) | default-option DDL to clipboard |
| `DatabaseView.OpenDdlInConsole` | Generate DDL to Query Console | default-option DDL into console |
| `DatabaseView.CopyDdlFromDbAction` | Request and Copy Original DDL | server-provided DDL (category *Definitions provided by RDBMS server*) |
| `Jdbc.OpenEditor.DDL` / `.Grid.DDL` | Go to DDL (Ctrl+B) | DDL editor (editable source for source-bearing objects) |
| `DDL.Editor.Regenerate` (group `DDL.Editor.Specific.StorageFile`) | Regenerate definition | regenerate the DDL editor text from the model |
| `DatabaseView.Tools.SubmitChanges` | Submit (Ctrl+K) | submit edited sources → Object Migration |
| `DatabaseView.ClearSrcStorage` | Clear Stored Sources | "Clear stored sources and local modifications" |
| `DatabaseView.ShowDiff` | Tools › Compare Structure… (Ctrl+D) | Migration dialog |
| `DatabaseView.ShowDiff.Configured` | Apply DDL Mapping… | Migration dialog driven by a mapping |
| `DatabaseView.ShowContentDiff` | Compare Data | data compare (doc 08) |
| `DatabaseView.DumpToSqlDataSource` | Import/Export › Dump to DDL Data Source | create/fill a DDL data source |
| `DatabaseView.DumpToConfiguredSqlDataSource` | Dump to associated DDL Data Source | dump via existing mapping |
| `DatabaseView.LayoutSqlDataSource` | Regenerate DDL files | re-lay-out a DDL data source |
| `DatabaseView.CreateDdlMapping` | Create DDL Mapping… | label varies: "Create DDL Mapping" / "Mapping: X" / "Mapping: X and N more" (ChoiceFormat) |
| group `DatabaseView.DdlMapping.Actions` | DDL Mapping › Update from / Apply to / Dump to | mapping actions (bundle also has "Apply from {0}") |
| group `DatabaseView.LinkedDataSource.Actions` | Linked Data Sources › Map to / Clear mapping / Navigate / New… | relation between data sources ("mapped", "derived", "parent" data source) |
| `DatabaseView.ConfigureLayout` | Configure Layout | pick/edit layout scripts |
| `DatabaseView.EditExternalDataAction` | Edit External Objects File | open `external-data-<ds>.xml` (virtual objects) |
| `DatabaseView.Scripted.Extensions` / `RunExtensionScriptGroup` | Tools › Scripted Extensions / Database Scripts | run user Groovy scripts (e.g. Generate POJOs) |
| `DatabaseView.Context.Templates` | SQL Scripts › <context template> | live templates using `dbObjectName`/`dbColumns` applied to a selected object |
| `DatabaseView.ShowScriptDiagnosticTool` | Show Script Diagnostic Tool | internal generator diagnostics |

### 3.2 Create / Modify dialog — anatomy

[Observed: help `create-and-modify-dialogs`; screenshots `db_modify_table`, `db_modify_column`, `db_modify_create_dialogs_context_menu_objects_editor`]

- **Title**: "Create" or "Modify" (`action.CreateObject.dialog.title.create`, `action.ModifyObject.dialog.title.modify`); old UI used "Modify Table".
- **Structure tree** (upper left; tables and other containers only): root = object; family nodes with child counts ("columns 4", "keys 1", "indexes 2"); families for a PG table: columns, keys, foreign keys, indexes, checks, virtual columns, virtual foreign keys (plus triggers/rules where the DBMS has them [Inferred]). Rows show name + type/default summary (`actor_id integer = nextval(…)`). Newly added objects are drawn **green** in the tree; changed tabs show a green title. [Observed]
- **Toolbar**: Add (Alt+Insert; on a family node adds that kind, else a popup of kinds), Remove (Alt+Delete), Move Up / Move Down (column order; screenshot), Edit (F4), Edit On Selection (toggle single-click vs double-click opening), Back (Ctrl+Alt+Left), Forward (Ctrl+Alt+Right). Double-click on a family node shows **similar objects as a list**.
- **Objects editor** (upper right): one **tab per opened object**, each tab has a pin icon; tab context menu: *Pin Tab*, *Close Tab*, *Revert* (Ctrl+Alt+Z / ⌥⌘Z — reverts edits of that object only), *Select Next Tab* (Ctrl+→), *Select Previous Tab* (Ctrl+←). [Observed: screenshot]
- **Name field** has a ⋮ menu: *Auto Generate* (naming template, §3.12) and context conversions *Make {0}* (e.g. *Make Primary Key* on a column, which adds a key node and opens its tab). [Observed: help `primary-keys`, bundle]
- **Comment** fields are one-line with an expand button (multi-line editor). Expression fields (Data Type, Default Expression, Predicate, Condition, Source Text, Query) are **embedded SQL editors** with highlighting/completion. [Observed: screenshot coloring]
- Fields unsupported for modification by the DBMS (e.g. PG *Persistence*, *Partition Key*, *Access Method* of an existing table) are **shown disabled**. [Observed: screenshot]
- **Preview** (bottom, collapsible "Preview" separator): editable SQL; gear = script settings (§3.4); ↗ = *Open query in console* (closes dialog, moves script to a console). Other object editors use a toggle "SQL Script" pane (`ObjectEditorWithPreview`, action *Toggle SQL Script Preview*). [Observed]
- **Cancel/Esc** asks for confirmation when *Confirm cancellation for dialogs that modify schema* is on (doc 11 OT-1). [Observed]
- Validation messages: "Empty table is not allowed", "'{0}' is not a valid {1} name", "Name is not unique — Object with the name '{0}' already exists". [Observed: bundle]
- Progress titles reveal the pipeline: "Prepare dialog", "Loading preview", "Generating code", "Updating structure from preview", "Creating/Modifying/Removing {0}". [Observed: bundle] → the preview is generated asynchronously and the reverse direction (text → structure) exists.
- Internally the dialog edits a **copy of the model** through typed *appliers* (`DbNameModelApplier`, `DbSourceTextModelApplier`, `DbObjectGrantsModelApplier`, `DbUserGrantsModelApplier`, `DbKeyColumnsModelApplier`, `DbIndexColumnsModelApplier` with PG/MySQL variants, `DbForeignKeyColumnsModelApplier`, `DbSeqIdModelApplier`, `DbListPropModelApplier`, `DbSinglePropModelApplier`, `DbStructureFamilyModelApplier`…) and builders (`DeTableBuilder`, `DeColumnBuilder`, `DeKeyBuilder`, `DeIndexBuilder`, `DeForeignKeyBuilder` with `*Raw` variants) and regenerates the preview from the difference. [Observed: names; Inferred: flow → ALG-DDL-07]

### 3.3 Per-object field reference

Generic fields are documented for all DBMSs; vendor fields come from the property metamodel ("The IDE generates fields for the Modify dialog automatically according to the properties received during the introspection", help `columns`). PG examples are from screenshots. [Observed unless marked]

| Object | Generic fields (help) | Vendor fields seen (PG screenshots) / notes |
|---|---|---|
| **Schema** | Name (+Auto Generate), Comment, Owner, Grants (grantee ↔ permissions; add/remove/up/down) | — ; created from data source, database or schema node (*New › Schema*) |
| **Table** | Name, Comment, Owner, Grants | Persistence (PERSISTENT/…; disabled on modify), With OIDs, Partition Expression, Partition Key, Options (storage parameters), Access Method (`heap`), Tablespace |
| **Column** | Name, Comment, Data Type, Not Null, Default Expression | Identity Kind (dropdown), Identity checkbox, Min, Max, Step, Cache Size, Cycled (enabled only with identity); Auto Increment for MySQL/SQLite-style DBMSs; PG column checks editable since 2022.1. SQLite: removing a column in *Modify Table* **recreates the table** (help) |
| **Key** (PK/unique) | Name, Comment, Primary (checkbox), Columns (Column Name, Target Name*) | composite keys by adding several columns; *Make Primary Key* from column ⋮. (*"Target Name" listed in help for keys looks copy-pasted from FK — [Unknown]) |
| **Foreign key** | Name, Comment, Target Table, Columns (Column Name ↔ Target Name), Deferrable, Initially Deferred, On Delete, On Update | referential actions list per DBMS (NO ACTION/RESTRICT/CASCADE/SET NULL/SET DEFAULT) [Inferred] |
| **Index** | Name, Comment, Unique, Columns (Column Name, Order, Collation) | per column: Order (`NONE`/ASC/DESC), Collation, **Operator Class**; Condition (partial index), Include Columns, Access Method (`btree`…), Tablespace. Not supported for MongoDB |
| **Check** | Name, Comment, Predicate | — |
| **View** | Name, Comment, Owner, **Source Text**, Grants | modify of an existing view goes through Go to DDL + Submit (help `views`) |
| **User / Role** | Name, Comment, Create DB, Grants | PG: Bypass RLS, Can Log in, Create DB, Create Role, Inherit, Replication, Super Role, Connection Limit (`-1`), Config, Valid Until; Grants rows = object ↔ privilege list, each privilege *Grant* or *Grant with option*. Not supported for BigQuery, Couchbase Query, HSQLDB, MongoDB, Redis. Some DBMSs require creating at database level |
| **Virtual column** | Name, Expression | stored in `external-data-<ds>.xml` (path in data source Options › *Virtual objects and attributes*); cannot be indexed; "Save external data for <ds>" prompt on first save |
| **Virtual foreign key** | Name, Target Table, Columns (Column ↔ Target) | same storage; rule patterns in Settings › Other › Virtual Foreign Keys |
| **Virtual view** | Name, Query (SELECT or e.g. `show processlist`, `exec sp_who2`) | under *Database Objects*; same storage |
| **Routines, triggers, sequences, types, materialized views, events, synonyms, packages…** | not described field-by-field in help | created via *New › <kind>* (template-like dialog/console) and modified **through Go to DDL + Submit** (source-bearing objects); sequence/identity details in column editor; MySQL events, partitions and subpartitions have producers [Observed: producer names `MysqlBaseCreateEvent`, `…Partition`, `…Subpartition`] |

**Observed generation examples** (screenshots, paraphrased):
- Adding a column with default and NOT NULL plus comment on PG → one `ALTER TABLE … ADD <col> <type> DEFAULT … NOT NULL` statement followed by a separate `COMMENT ON COLUMN` statement.
- Creating a PG role with *Can Log in*, *Super Role*, *Create DB*, *Create Role* (Inherit on, connection limit −1) → `CREATE USER <name>` with only `SUPERUSER CREATEDB CREATEROLE`; the defaults (LOGIN implied by CREATE USER, INHERIT, CONNECTION LIMIT −1) are **not** emitted.
- Changing an OUT parameter of a PG function in the DDL editor → Object Migration shows `DROP FUNCTION <old signature>` + `CREATE FUNCTION …` (no OR REPLACE, because the signature changed).

### 3.4 Preview settings (`TableDialogSettings`)

[Observed: bundle labels; Inferred: semantics from labels and from the DDL-data-source workflow]

| Setting | Values | Inferred meaning |
|---|---|---|
| Action: | *Modify existing objects* · *Create modified objects* · *Create all objects* | emit ALTERs against the current state (default for real data sources) · emit CREATE statements only for changed objects · emit full CREATE for everything in the dialog |
| Script: | *Open in editor* · *Execute in database* · *Replace existing DDL* | where the result goes: console/editor, run immediately on OK, or rewrite the object's CREATE text in a DDL data source file (message "Unable to replace DDL for some objects (they are in DB)") |
| Rename in: | *Comments and strings* · *Text occurrences* · *Loaded sources* | when a name changes in the dialog, also update usages (same categories as the Rename dialog) |
| (generator options) | Qualify, Place constraints, IF EXISTS, CASCADE, Reformat… | the gear also hosts the subset of SQL Generator options relevant to the dialog (Confirm Drop shows exactly: Qualify, IF EXISTS, DROP CASCADE, Reformat) [Observed: Confirm Drop screenshot] |

### 3.5 Auxiliary dialogs

[Observed: help, bundle, screenshots]
- **Confirm Drop** (Delete / Edit › Drop): options *Qualify objects with schema names* (Auto/Never/Always; default Auto), *Use IF EXISTS syntax* (off), *Use DROP CASCADE syntax* (off; "views that depend on it will be deleted"), *Reformat generated code* (on); editable Preview; OK executes; ↗ opens in console.
- **Rename (table/column)**: "Rename <x> and its usages to", *Search in loaded sources*, *Search for text occurrences*, *Search in comments and strings*, Scope (named or custom scope), editable statement preview, buttons *Preview* (Find tool window with usages; exclude/remove items, *Do Refactor*) and *Refactor*. Inline rename (Shift+F6) offers the same toggles via gutter icons; Shift+F6 twice opens the dialog. Help: clear both checkboxes to rename the object only. "Usages won't be renamed, because indexing is in progress. Proceed?".
- **Truncate Table**: generator category *Truncate table* with options Restart identity ("Restart sequences owned by a table"), Use CASCADE, Use DELETE statement instead of TRUNCATE, Purge materialized view log, Reuse storage, RESTRICT WHEN DELETE TRIGGERS, Immediate (DBMS-specific subsets: PG, Oracle, DB2 [Inferred from keywords]).
- **Recompile** (Oracle PL/SQL): *With DEBUG option* (needed to debug PL/SQL), *With REUSE SETTINGS option*.
- **Modify Comment**: Comment text + *Set NULL*.
- **Modify Grants**: Grants pane only, for any object.
- **Batch Modify Indexes / Batch Add Indexes**: multi-object index editing; name template hint "Template can contain {table} and {columns}; default `{table}_{columns}_index`". [Observed: bundle `hint.text.template.can.contain`]

### 3.6 SQL Generator tool window and one-shot DDL actions

[Observed: bundle `scriptGen.*`, help `export-data`, screenshots `db_sql_generator`, `db_sql_generator_output_settings`]

**Layout.** Tool window "SQL Generator". Left vertical tabs: ⚙ *SQL Generator Options*, 💾 *File Output Options*. Middle: option panel. Right: generated SQL (read-only editor with line numbers), toolbar *Copy to Clipboard*, *Save into File*, *Open in Console*. Status: "Generating SQL script…", "Generation finished", "Generation finished with errors", "Generating script to file". The generator runs on the **selection** (one object, many objects, a whole schema).

**Categories** (`Generate:` combo):

| Category | Purpose | Category-specific options |
|---|---|---|
| Creation script completely | full CREATE for selection incl. nested items | most options below |
| Creation script before data import | objects needed before bulk load (tables without secondary constraints/indexes/triggers) [Inferred] | same |
| Creation script after data import | the deferred part: indexes, FKs, checks, triggers [Inferred] | same |
| Definitions 'as is' | each object's definition as the model holds it | Reformat, Regenerate DB-provided code |
| Definitions provided by RDBMS server | server text (PG `pg_get_*def`, MySQL `SHOW CREATE`, Oracle `DBMS_METADATA`, MSSQL module text) [Inferred mapping] | — |
| Enable / Disable constraints and triggers | toggles for FKs, checks, triggers | With disabling clauses or statements |
| Recompile | Oracle-style recompilation | With DEBUG, With REUSE SETTINGS |
| Refresh materialized views | `REFRESH MATERIALIZED VIEW` | Add WITH NO DATA clause |
| Execute procedures and functions | call skeletons | — |
| Delete data | DELETE per table | — |
| Drop objects | DROP per object in reverse dependency order | Use IF EXISTS, Use DROP CASCADE, Purge |
| Truncate table | TRUNCATE | Restart identity, Use CASCADE, DELETE instead of TRUNCATE, Purge MV log, Reuse storage, RESTRICT WHEN DELETE TRIGGERS, Immediate |
| Alter elements | ALTER-style re-definition (e.g. MS "ALTER instead of CREATE") | MS options |

**Options** (label — meaning in our words; default when observed):

| Option | Meaning | Default |
|---|---|---|
| Qualify objects with schema names: Auto / Never / Always | Auto omits the schema when all objects are in one schema or the schema is the current (login) schema; affects generated code only, not server-provided text | Auto [Observed] |
| Place constraints: Inside column / Inside table / After table | single-column constraint inline in the column; table-level after columns; separate `ALTER TABLE ADD CONSTRAINT`. "Processing of some foreign key declarations might be postponed in order to resolve cyclic dependencies" | Inside column [Observed] |
| Include nested elements | include triggers, bodies and other nested items | [Unknown] |
| Use surrogate (auto-generated) names | include server-generated constraint names that the author omitted; otherwise omit them "like the author did" | off [Observed] |
| Skip DEFINER clause | MySQL/MariaDB routines/views/triggers without DEFINER | off [Observed] |
| Use CREATE OR REPLACE syntax | when applicable | off [Observed] |
| Use CREATE IF NOT EXISTS syntax | when applicable | off [Observed] |
| Ignore modification of the identifier case | treat case-only renames as no change | [Unknown] |
| Specify the next value for sequences | include current next value when the sequence has been used at least once | off [Observed] |
| Consider index / keys except for clustered / foreign keys / check constraints / rules / triggers | include each kind; clustered keys excluded because they change physical row placement | [Unknown] |
| Use VARCHAR instead of VARCHAR2 | Oracle | [Unknown] |
| Use IF EXISTS syntax / Use DROP CASCADE syntax | drop variants | off (Confirm Drop) |
| Ignore owner / Skip grants / Show grants in user definition | owner clause; GRANT statements; grants on all accessible objects inside a user's script | Skip grants off [Observed] |
| Ignore order | ignore column order | [Unknown] |
| Reformat generated code | apply current code style to generated (not server-provided) code | on in Confirm Drop [Observed] |
| Regenerate database-provided code | regenerate property definitions the DB returns as part of source text instead of passing text through | [Unknown] |
| Visual Studio DDL compatibility | MS SQL: format like VS database projects | [Unknown] |
| Generate ALTER instead of CREATE | MS SQL routines/views | [Unknown] |
| With disabling clauses or statements | emit disable code for elements currently disabled; inline in CREATE when possible unless registry `database.script.generator.separate.disable=true` | [Unknown] |
| Use semicolon as statements delimiter | `;` instead of native delimiter (`GO`, `/`) | [Unknown] |
| Ignore extension members | PG objects owned by extensions are skipped | [Unknown] |
| Allow object recreation | recreate when ALTER is impossible | [Unknown] |
| With DEBUG / REUSE SETTINGS; Add WITH NO DATA; truncate options | category-specific (above) | — |
| Run diagnostic tool / Dump generation task | internal: diagnostics and a reproducible task dump for bug reports (dump option visibility gated by `database.script.generator.show.dump.option=false`) | off |

**File Output Options**: *Path* (directory chooser; must be a directory — "Expected directory"), *Layout* (initially `<unknown>`; five bundled layouts + user layouts; ⚙ next to it opens layout script management, *New Layout* dialog asks "Layout name:"), button *Dump*. When files are generated into a folder that belongs to a DDL data source: "New Files Are Generated — Add new files to the DDL data source? … and {0} files more". [Observed]

**One-shot actions** use the generator with stored/default options: *Generate DDL to Clipboard* (Ctrl+Alt+Shift+G), *Generate DDL to Query Console*, *Request and Copy Original DDL* (server text). Help contrasts: Go to DDL = unqualified editable source; SQL Generator = runnable script (CREATE OR REPLACE, qualified names). [Observed]

### 3.7 Code style and naming influence on generated DDL

[Observed: help `code-style-sql-ddl`, `code-style-sql-code-generation`, `settings-tools-database-other`]
- **Code Style › SQL › DDL** (used when *Reformat generated code* is on): opening parenthesis placement; element placement (Same line aligned / Wrapped unindented / Wrapped aligned / Wrapped indented); closing parenthesis (At the end / To begin / Under opening / Under elements); Align types / defaults / nullabilities; Collapse when short; Wrap/Align ALTER instructions; Wrap CONSTRAINT, KEY/CHECK, REFERENCES, cascade & deferrability; CREATE SCHEMA: Indent content, min/max blank lines between declarations; Views: Wrap AS, wrap beginning of query, indent query; postfix options (wrap first/next option, indent, align). Full catalog in doc 11.
- **Code Generation tab**: templates *Primary key*, *Foreign key*, *Index*; literal text + `{variable}` + `{flag?then:else}`; Ctrl+Q lists variables. Help examples: `{table}_pk` → `actor_pk`; `{table}_{columns}_{unique?u:}index` on non-unique index of `persons(FirstName, LastName)` → `persons_FirstName_LastName_index`. Help prose also cites `{table}_{columns}_pk` as "the" PK template. Shipped defaults per template: [Unknown → doc 13 SET].
- **Settings › Database › Other › Code Generation**: *Generate context templates:* Append to existing console / In a new console; *Statement delimiter:* (doc 11 OT-4/OT-5).

### 3.8 Editing object source: DDL editor, Database Changes, Object Migration

[Observed: help `modifying-source-code-of-database-objects`, `update-source-code-of-database-objects`, `database-changes-tool-window`, `submitting-and-reverting-changes`; inspection `SqlStorageInspection`; class names]

**Prerequisite.** Sources are fetched during introspection per data source setting *Load Sources*: None / Except System Schemas / All Schemas (context menu on data sources; registry `database.automatically.load.sources=true`). Tables have no stored source — only source-bearing objects (views, routines, triggers, packages, rules…) participate. (Introspection details: doc 04.)

**Flow.**
1. *Go to DDL* (Ctrl+B) on one or many objects opens a **DDL editor** per object with **unqualified** references; text is the object's CREATE statement.
2. Edits are saved as **local versions**; gutter change markers (click → popup with *Show Diff for Lines*, previous content).
3. **Database Changes** tool window (stripe "Database Changes" / short "DB Changes") appears only after the first modification; lists modified objects (groupable by type: routines, rules, views…). Toolbar: *Submit* (Ctrl+K), *Roll back* (Ctrl+Alt+Z; Manual tx mode), *Show Diff* (Ctrl+D; also double-click), *Group by*, *Expand All*, *Collapse All*.
4. *Submit* (from the DDL editor toolbar or the tool window; one object or a multi-selection) → **Object Migration** dialog (Preview with gear and ↗; the changed region highlighted) → OK executes. Complex changes become DROP + CREATE.
5. After success the editor resets to the **database's** version (registry `database.migration.source.revert.on.success=true`) because servers may store code differently than typed (e.g. PG re-renders view text).
6. *Clear Stored Sources* drops cached sources and local modifications; *Regenerate definition* re-derives the DDL editor text.

**Preview setting.** Settings › Database › Other › Refactoring › *Show preview of valid script when updating source text*. If off, a valid migration script **executes silently** on Submit. The default is documented inconsistently (§7).

**Staleness and conflicts.**
- Data source option *Warn when editing outdated DDL* (Options › Introspection). On opening an object whose cached source differs from the server: notification with *Synchronize* (Ctrl+F5) / *Disable check* ("Disable Outdated Object Notification"). Shown as "Unsupported" for DBMSs without the check; the inspection description lists PostgreSQL, SQL Server, Oracle, Sybase ASE.
- On Submit when the server changed meanwhile: dialog "Possible Loss of Changes — Objects being modified are changed in database" with **Force Refactoring** (overwrite) or **Abort Refactoring and Synchronize** (re-introspect). If the conflict persists: **Revert local changes**, **Keep local changes** (overwrite server), **Merge** (3-way merge dialog: accept left/right, ignore, result in the middle, *Save changes and finish merging*).
- Object name colors in the explorer/tool window: **black** = in sync, **blue** = locally modified without conflict, **red** = conflict.
- `SqlStorageInspection` reports: source changed in DB but not in the editor; you changed source but someone else changed it too; introspector upgrade requires re-fetching new properties.

**Mechanism** [Observed: names; Inferred: design]: sources live in a virtual file system (`DbSrcFileSystem`) backed by zip files (`DbSrcBackendZipWriter`, `DbSrcZipFile`; on disk `storage_v2/_src_`, doc 01); modifications are tracked by `DbSrcChangesTracker` as `DbSrcChangeList` entries with `DbContentRevision` (base text) and `DbConflictChange`, file statuses `DbSrcFileStatus` — i.e. the IntelliJ VCS *Changes* model reused with the database as the "repository". Submit runs `DbSrcMigration` which parses the edited text into a model fragment and diffs it against the cached model (same generator as Compare). → ALG-DDL-09.

### 3.9 Compare Structure and the Migration dialog

[Observed: help `schema-comparison-and-migration`; screenshots `db_schema_migration_dialog`, `db_differences_between_object_properties`; bundle `migration.*`]

**Invocation.** Select two objects **of the same kind** (data sources, databases, schemas, tables, routines…) in the explorer → *Tools › Compare Structure* (Ctrl+D). They may be on different servers, different DBMS versions, or one may be a DDL data source. Errors: "Cannot compare selected elements", "Migration Not Available". Title pattern "'{0}' to '{1}' Migration" (mapping case) or "Migration".

**Layout.**
- Toolbar: *Expand All*, *Collapse All*, *Swap* (exchange Origin/Target), *Options* (gear menu with the Ignore toggles); top-right *Show identical* checkbox.
- Header row: "Origin: <data source>" | master include checkbox | "Target: <data source>".
- Body: **table-tree** with Origin and Target rows aligned side by side and an include checkbox per changed row in the middle column (registry `database.schema.diff.table.view=true`; root node shown per `database.schema.diff.show.root=true`; pre-2021.3 layout via `database.legacy.schema.diff`). Rows show kind icon, name and a summary (type, default). Collapsed identical nodes can show a count badge (e.g. "12 / 12") [Observed; exact semantics Unknown].
- Colors (target side): **green** = exists only in Origin → will be **created** in Target; **blue** = **modified** → altered; **red** = exists only in Target → will be **dropped**.
- Hidden-changes warning "Some changes are hidden" when filters hide included changes; context actions *Include change*, *Exclude change*, *Include all*, *Exclude all*.
- Bottom tabs:
  - **Script Preview** — the editable migration script for all included changes.
  - **Object Properties Diff** — table *Property | Origin | Target* for the selected pair; differing rows highlighted (e.g. Data Type `varchar(45)` vs `varchar`, Not Null `true` vs `false`); null shown as `null`.
  - **DDL Diff** — IntelliJ diff viewer of the two CREATE texts: Previous/Next difference (Shift+F7/F7), Jump to Source (F4), Side-by-side / Unified viewer, whitespace mode (Do not ignore [default] / Trim whitespaces / Ignore whitespaces / Ignore whitespaces and empty lines), highlighting (Highlight words [default] / lines / split changes / symbols / Do not highlight), collapse unchanged, Synchronize Scrolling, settings (Show whitespaces, line numbers, indent guides, soft-wrap, highlighting level, Annotate for VCS files), Help (F1); difference counter ("1 difference"); panes are read-only (lock icons).
- Buttons: *Cancel*, *Open in New Query Console*, *Execute*; bundle also defines *Dump* (visibility [Unknown]).

**Options (ignore filters).**

| Option | Effect (help, our words) |
|---|---|
| Ignore Order | ignore item order within an object (e.g. column order) |
| Ignore Case | ignore letter case in item names |
| Ignore Implicit Objects | ignore objects the DBMS generates automatically — e.g. indexes backing FKs, SQL Server DEFAULT constraints for column defaults |
| Ignore Source Code | match routines by kind + identity (signature) and ignore their bodies |
| Ignore Owner | ignore owner/privilege properties |
| Detect column insertion | checkbox in bundle (`checkbox.detect.column.insertion`) — [Inferred] distinguishes "column inserted in the middle" from "all following columns changed" |

Options are persisted (`DbMigrationOptionsService`). [Observed: name]

**Observed matching behavior.** Comparing schema `public` with schema `testing`: objects are matched by name **relative to their schema**; a serial default referencing `testing.<seq>` vs `<seq>` is **not** reported as a difference (no checkbox on that column). [Observed: screenshot] → references are normalized relative to the compared container (ALG-DDL-12).

**Reuse.** The same dialog serves *Apply DDL Mapping* / *Apply to <ds>*; Database Changes Submit uses the single-object *Object Migration* variant. Data compare is separate (doc 08).

### 3.10 Diff and generator internals (inferred from names)

- Diff: `DbDiffRoot`, `DbDiffElement`, `DiffAlgo`, `DiffSupport`, `DbDiffCheckOption` (the ignore options), `DbDiffElementFilter`, `DiffStatus`, `ObjectWrapper`, `SchemaDiffTool/Request/Customization/Action`, `DbMigrationAction` ("Apply '{0}' state to '{1}'"), UI `DbMigrationDialog`, `DbMigrationSideTree`, `DbMigrationTreeTableModel`, `DbMigrationDiffDivider`, `DbPropsDiffTableModel`, `DbDdlDiffPanel`, `SdForallCheckbox`. [Observed]
- Generator core: `AbstractScriptGenerator`, `BaseCreateGenerator`, `BaseAlterGenerator`, `Scenario`, `ScenarioOperations`, `ScenarioDot` (graph debug dump), `Operation`/`OperationType`, `Effect`/`EffectType`/`EffectCommon`/`RequiredEffect`, `LinkedPriorityQueue`, `PreOrderComparator`, `ObjectPathTrie`, `RefQualification`, `ScriptingContext`, `ScriptingCapabilitiesImpl`, `SupportedSinceImpl`/`OrSupportedImpl` (version gates), `Reviser`, invocators `CreateCapableInvocator`/`AlterCapableInvocator`/`DropCapableInvocator`/`UnsupportedCapableInvocator`, adapters `OldCreateAdapter`/`OldAlterAdapter`/`OldDropAdapter`/`ToAlterCapableAdapter`/`ToNewProducerAdapter`, `Flag`/`Flags`/`FlagValue` (option plumbing), `ScriptGeneratorDiagnosticTool`. Per dialect: `<D>ScriptGeneratorHelper`, `<D>NamingService`. [Observed]
- Base producer families: Create/Alter/Drop × {Argument, Check, Columniation, Constraint, Data, Database, ForeignKey, Index, Key, LikeColumn, LikeTable, MatView, Routine, Schema, Sequence, SourceAware, Synonym, Table, TableColumn, Trigger, View, ViewColumn, Unknown, Other}, plus GrantsProducer, RecompileProducer, TruncateProducer/TruncateTable, SearchPathProducer, CreateCommentCapable, GroupProducer. Dialect examples: `MysqlAutoIncHack`/`MysqlAutoIncValProducer` (AUTO_INCREMENT counter), `MysqlBase*Event/Partition/Subpartition/User`, `MsAlterTableType*`, `MsAlterFileGroup`, `MsAlterLogin`, `OraAlterTablespace`, `PgBase*Role/Database`, `Sqlite*` (Alter table/column/view/routine — rebuild path), `CHouseAlterRole/User`, `Couchbase*Index/Table`, `HiveBase*`. [Observed]
- Inferred model: each producer converts an element change into operations that *provide* and *require* effects; the scenario schedules operations so requirements hold, with a pre-order comparator (schema → table → column → constraint) as tie-break; when an ALTER is unsupported for the dialect/version, the producer falls back to recreate if allowed; the `Reviser` post-processes the operation list (e.g. merging ALTER clauses, postponing FKs for cycles). [Inferred] → ALG-DDL-02/03/08.

### 3.11 DDL data sources

[Observed: help `ddl-data-sources`, `create-ddl-data-source-using-sql-files`]
- A DDL data source is a virtual database structure built from SQL files; objects appear in the explorer, resolve in code, support diagrams; intended for **versioning in VCS**.
- **Create**: (a) *Import/Export › Dump to DDL Data Source* from a real data source (choose existing DDL data source or create new; folder auto-attached in Files tool window); (b) *Data Sources and Drivers › + › DDL Data Source*, Sources pane *+* to add files/directories, *Dialect* link; (c) drag SQL files from the Files tool window to the explorer.
- **Sources pane**: add existing **scope**, an *Anonymous Scope* (not stored with other scopes), or *Edit Scopes…*; excluded files are ignored on dumps; *Configure SQL Dialect* icon; empty state "Add directories or DDL files".
- **Auto sync** checkbox: refresh model when files change.
- **Extend Data Source**: let DDL objects reference objects of a real data source (e.g. an index file on a table that only exists in the DB); referenced objects resolve and appear in the tree.
- **New objects**: *New › <object>* under the DDL data source (dialog writes files, Script = *Replace existing DDL*/new file [Inferred]) or *New › SQL File* in the directory with a CREATE statement.
- **New File Layout** tab (dump settings): *Layout* (5 options), *Root path*, *Code style* (profile), *Qualify objects with schema names* (Auto: qualify only when identical names exist in several schemas / Never / Always), *Place constraints* (Inside column / Inside table / After table), *Regenerate database-provided code*, *Reformat generated code*, *Default schemas/databases* table (DDL data sources have unnamed databases/schemas; set names used for unqualified objects).
- Messages: "No source for DDL data source", "DDL data source not found", "Where to store generated sources?" (Root Path dialog).

### 3.12 DDL mappings, linked data sources, VCS workflow

[Observed: help `ddl-mappings`, `databases-in-the-version-control-system`; bundle]
- **Mapping settings** (Data Sources and Drivers › *DDL Mappings* tab, or *DDL Mapping › Create DDL Mapping*): Name, Comment, Data Source (`<Choose Data Source>`), DDL Data Source, **Scope** tree (schemas/databases). Invalid references show `<Non-existing Data Source>` / `<Non-existing Scope>`.
- **Actions** (context menu of either side): *Update from <ds>* (DB → files; bundle label "Apply from {0}"), *Apply to <ds>* (files → DB through the Migration dialog: select changes, edit Script Preview, Execute), *Dump to <ddl ds>* (full regeneration; "The following files from the DDL data source are missing in the generated DDL dump. Delete them…?" — *Outdated Files*).
- Setting *Suggest dumping DDL for new mappings* (OT-3): after adding a mapping, ask "Do you want to dump a data source to a DDL data source for the following new mappings? … To call this action later, right-click a data source and go to '…'".
- Not supported for MongoDB.
- **Linked data sources** (*Map to*, *Clear mapping*, *Navigate*, *New…*; labels "Mapped data source", "Derived data source", "Parent data source") generalize the relation [Inferred].
- **VCS workflow** (tutorial): enable Git, clone repo, *Dump to DDL Data Source* into the repo folder, add mapping with selected schemas, commit/push. Sharing changes: change DB → *Update from* → commit/push. Receiving: pull → *Apply to* → Migration dialog → Execute.

### 3.13 Layout scripts

[Observed: `extensions/schema.layouts/*.groovy` (described, not copied), help]

Contract: a layout script configures a `LAYOUT` object with
- `baseName(ctx)` — relative path **without extension** for the object `ctx.object`;
- optional `fileName(ctx)` — final file name, with `ctx.count` (1-based ordinal in generation order) and `ctx.baseName`;
- optional `fileScope(path)` — reverse map from a relative file path to an object scope (database and/or schema) so files can be loaded back into the right namespace; returns null for files at the root;
- flag `ignoreDependencies` — when true, files need not be emitted/loaded in dependency order.

Shared rules in all five scripts: an object is written to the file of its **nearest ancestor that is stored separately** (objects with source, or direct schema children) — so columns, indexes, keys, triggers-without-source go into the table's file; names are sanitized by replacing `/` with the literal word `slash` (no other characters touched).

| Layout | Path (ours) | Dependencies | fileScope |
|---|---|---|---|
| File per object | `<object>.sql` | ignored | — |
| File per object with order | `NNN-<object>.sql` (3-digit zero-padded ordinal) | **respected** (only layout that does) | — |
| File per object by schema | `<schema>/<object>.sql` (no schema → root) | ignored | first path segment = schema |
| File per object by schema and type | `<schema>/<kind code>/<object>.sql` | ignored | first segment = schema |
| File per object by schema and database | `<database>/<schema>/<object>.sql`; no database but schema → `anonymous/<schema>/…`; database but no schema → `<database>/<object>.sql` | ignored | 2 segments + file → database+schema; `anonymous` → schema only |

Users create layouts via ⚙ › *New Layout* (name prompt) and edit them ("Edit layout script", "Add new layout script"; failure "Failed to create the layout script"). `DatabaseView.ConfigureLayout` and *Regenerate DDL files* apply a layout to an existing DDL data source.

### 3.14 Scripted extensions and context templates

- **Scripted Extensions** (explorer *Tools › Scripted Extensions*; files under the IDE's extensions folder, `schema` subfolder for object scripts): scripts receive the selection, the project and a file helper that asks for a target directory; the bundled *Generate POJOs* maps column types to Java types by pattern and writes one class per table/view. [Observed: help `generating-code`; script header described]
- **Context templates**: live templates in the "Any SQL Statement" context that use `dbObjectName()` / `dbColumns()` appear under *SQL Scripts* for a selected object; output goes to the current or a new console (OT-4). [Observed: help `context-live-templates`]

### 3.15 Virtual objects

Virtual columns (computed expression shown in the data editor), virtual foreign keys (used by completion/JOIN inference/navigation/diagrams; rules in Settings › Other › Virtual Foreign Keys) and virtual views (stored query; any statement that returns rows) are edited in the same Create/Modify dialogs but persisted in `external-data-<data_source>.xml` (path in data source *Options › Virtual objects and attributes*; *Edit External Objects File* action). The preview pane shows nothing executable for them [Inferred: nothing is sent to the DB]. [Observed: help]

### 3.16 Related settings and registry (consolidated)

| Where | Item | Default | Doc |
|---|---|---|---|
| Settings › Database › Other › Modify Object | Confirm cancellation for dialogs that modify schema | [Unknown] | 11 OT-1 |
| › Refactoring | Show preview of valid script when updating source text | contradictory docs | 11 OT-2 |
| › DDL Mappings | Suggest dumping DDL for new mappings | [Unknown] | 11 OT-3 |
| › Code Generation | Generate context templates; Statement delimiter | [Unknown] | 11 OT-4/5 |
| Code Style › SQL › DDL / Code Generation | DDL formatting; PK/FK/index name templates | — | 11 |
| Data source › Options | Load Sources (None/Except System/All); Warn when editing outdated DDL; Virtual objects and attributes path | — | 02/04 |
| Registry | `database.schema.diff.table.view` | true | — |
| | `database.schema.diff.show.root` | true | — |
| | `database.legacy.schema.diff` | false | — |
| | `database.script.generator.separate.disable` | false | — |
| | `database.script.generator.show.dump.option` | false | — |
| | `database.migration.source.revert.on.success` | true | — |
| | `database.automatically.load.sources` | true | 04 |
| | `database.query.generation.omit.schema.qualifier` | false | 08 |

### 3.17 Dialect ALTER capability matrix (reference for ALG-DDL-03)

[Inferred: DBMS documentation knowledge for current major versions; DataGrip encodes equivalent knowledge in dialect producers and `SupportedSince` gates — verify per case in doc 13 family DDL]

| Change | PostgreSQL 16 | MySQL 8.4 | SQL Server 2022 | SQLite 3.45 | Oracle 23 |
|---|---|---|---|---|---|
| Add column (nullable) | `ADD COLUMN` | `ADD COLUMN [FIRST/AFTER]` | `ADD` | `ADD COLUMN` (end only) | `ADD (...)` |
| Add NOT NULL column with default | `ADD COLUMN … NOT NULL DEFAULT` (fast, PG11+) | yes | yes (default constraint named) | only with non-NULL constant default | yes |
| Drop column | `DROP COLUMN` (+CASCADE) | yes | yes (drop default/check/index first) | 3.35+, restrictions (not PK/UNIQUE/indexed/FK) | `DROP COLUMN` / `SET UNUSED` |
| Rename column | `RENAME COLUMN` | `RENAME COLUMN` (8.0) | `sp_rename 't.c','n','COLUMN'` | 3.25+ `RENAME COLUMN` | `RENAME COLUMN` |
| Change type | `ALTER COLUMN TYPE … USING` (rewrites; blocked by dependent views) | `MODIFY COLUMN` full definition | `ALTER COLUMN` (indexes/constraints may block) | **rebuild** | `MODIFY` (restrictions when data present) |
| Change nullability | `SET/DROP NOT NULL` | `MODIFY` | `ALTER COLUMN … [NOT] NULL` | rebuild | `MODIFY … [NOT] NULL` |
| Change default | `SET/DROP DEFAULT` | `ALTER COLUMN SET/DROP DEFAULT` | drop+add default constraint | rebuild | `MODIFY … DEFAULT` |
| Reorder columns | rebuild | `MODIFY … AFTER` | rebuild | rebuild | rebuild (or invisible-column trick) |
| Add PK/UK/FK/CHECK | `ADD CONSTRAINT` (FK `NOT VALID` + `VALIDATE`) | `ADD` | `ADD CONSTRAINT` (`WITH NOCHECK`) | **rebuild** | `ADD CONSTRAINT` (`ENABLE NOVALIDATE`) |
| Drop constraint | `DROP CONSTRAINT` | `DROP PRIMARY KEY/FOREIGN KEY/INDEX/CHECK` | `DROP CONSTRAINT` | rebuild | `DROP CONSTRAINT` |
| Create/drop index online | `CONCURRENTLY` (not in tx) | `ALGORITHM=INPLACE, LOCK=NONE` | `ONLINE = ON` (Enterprise/Dev) | n/a | `ONLINE` |
| View change | `CREATE OR REPLACE VIEW` (append-only column changes) else drop/create (+dependents) | `CREATE OR REPLACE` / `ALTER VIEW` | `CREATE OR ALTER VIEW` | drop/create | `CREATE OR REPLACE` |
| Routine change | `CREATE OR REPLACE` if same signature & return type, else DROP + CREATE (observed in Object Migration) | DROP + CREATE | `CREATE OR ALTER` | n/a | `CREATE OR REPLACE` |
| Enum/domain change | `ALTER TYPE ADD VALUE`/`RENAME VALUE`; remove → recreate | column `MODIFY … ENUM(...)` | n/a (CHECK) | n/a | n/a |
| Transactional DDL | yes (except CONCURRENTLY, ADD VALUE pre-12) | no (implicit commit) | mostly yes | yes | no (implicit commit) |
| Identity add/drop | `ADD/DROP IDENTITY` | `AUTO_INCREMENT` via MODIFY | not alterable → rebuild | `INTEGER PRIMARY KEY` only at creation | `MODIFY … GENERATED … AS IDENTITY` (12c+) |
| Comment | `COMMENT ON` | inline `COMMENT '…'` (column via MODIFY; table via `ALTER TABLE … COMMENT=`) | `sp_addextendedproperty` / `sp_updateextendedproperty` | none (store elsewhere) | `COMMENT ON` |
| Owner | `ALTER … OWNER TO` | n/a (DEFINER for routines/views) | `ALTER AUTHORIZATION` | n/a | n/a (schema = owner) |

SQLite "rebuild" = the documented 12-step procedure: disable FKs, begin, create new table, copy data, drop old, rename new, recreate indexes/triggers/views, `PRAGMA foreign_key_check`, commit, re-enable FKs. DataGrip's help confirms *Modify Table* recreates SQLite tables to drop a column.

---

## 4. Algorithm specifications

### ALG-DDL-01: Model diff (structure compare)
- **Problem:** Compute the changes that make Target equal to Origin (or Current → Desired) for a scope.
- **Inputs / outputs:** two models (introspected, DDL files, or an edited copy), options `{ignore_order, ignore_case, ignore_implicit, ignore_source, ignore_owner, detect_column_insertion, rename_detection: Off|Suggest|Auto}`, root pair (may be differently named containers) → `DiffTree` of `Change{origin_path?, target_path?, kind: Added|Removed|Modified|Renamed|Identical, property_diffs: [(prop, origin, target)], children, included: bool}`.
- **Algorithm:**
  1. **Normalize** both models (ALG-DDL-12): canonical types, parsed defaults, references re-expressed relative to the compared root container (so `testing.seq` ≡ `seq` when roots are `testing`/`public`), implicit objects tagged (ALG-DDL-11).
  2. **Match** level by level (database → schema → kind families → objects → sub-objects). Key = `(kind, identity)`: name (case-folded when ignore_case, else dialect folding of unquoted identifiers); routines = name + argument-type signature; triggers = (table, name); constraints = name, except **surrogate/system-named** constraints which are matched by **definition** (type + columns + referenced table/columns + predicate).
  3. **Rename detection** among leftovers (ALG-DDL-10), default Off/Suggest — DataGrip shows drop + add [Inferred, DDL-I-05].
  4. **Compare properties** of matched pairs using a per-kind property schema with categories (`structure`, `order`, `source`, `owner`, `grants`, `comment`, `storage`); skip categories disabled by options; ignore_order compares member lists as multisets but, when off, emits an `order` difference; detect_column_insertion uses LCS over column names so a middle insertion is one Added rather than N Modified.
  5. Recurse; parent is Modified if any child differs; Identical nodes hidden unless *Show identical*.
  6. All non-identical changes `included=true`; parent checkbox tri-state; excluding a parent excludes children.
- **Data structures:** `ObjectPath` trie; `PropertyDescriptor{name, category, comparator, display}`; `IndexMap` to keep model order for stable output.
- **Complexity:** O(N log N) with hash matching; source comparison O(len) per object; LCS O(c²) per table (c = columns, small).
- **Edge cases (exhaustive list):** identifiers differing only by case (PG `"Users"` vs `users`) with ignore_case on/off; overloaded routines; routines whose signature changed (Removed + Added, never Modified); system-generated names (`SYS_C00123`, `PK__t__3213E83F…`, `t_pkey` vs `t_pk`); unnamed vs named constraint with same definition; column order differences (PG cannot reorder); type aliases (`int4`/`integer`, `varchar` vs `character varying`, MySQL `int(11)` display width); defaults referencing schema-qualified sequences in differently named schemas; serial vs identity producing identical behavior; partitioned tables and partitions; inherited columns (PG); extension-owned objects; generated/computed columns; collations differing only by version; comment-only changes; grants for roles that do not exist on Target; cross-version compare (PG 12 vs 16 features: properties absent on one side must compare as "unknown", not "different"); objects in excluded schemas referenced by included ones; Origin is a DDL data source with unnamed schema (use default-schema table); empty roots; the same object selected twice; kinds unsupported by Target DBMS (cross-DBMS compare → report "Migration Not Available").
- **Test cases:**
  | Input | Expected |
  |---|---|
  | Origin `t(a int, b text)`, Target `t(a int)` | `t` Modified; `b` Added (green); script adds column |
  | Origin `t(a)`, Target `t(a, c)` | `c` Removed (red) → DROP COLUMN |
  | Origin `public.t(id serial)`, Target `testing.t(id serial)` (compare schemas) | identical (sequence reference normalized) |
  | Origin `CHECK (x>0)` unnamed, Target same check named `t_x_check` | identical when surrogate names ignored |
  | `f(int)` vs `f(int, int)` | one Removed + one Added routine |
  | ignore_case on: `Users` vs `users` | identical; off → Removed + Added |
  | Origin columns (a,b,c), Target (a,c,b), ignore_order on | identical; off → `order` change only |
  | Origin inserts `x` between a and b, detect insertion on | one Added `x`; no Modified for b/c |
  | function body differs, ignore_source on | identical |
- **Prior art and evidence:** [Observed: help options, screenshots, `DbDiffRoot`, `DiffAlgo`, `DbDiffCheckOption`]; migra, Skeema, Atlas (Apache-2.0).
- **Used by:** Hacivat (Compare, DDL mapping apply, object source submit), Karagöz (drift detection job; optional).

### ALG-DDL-02: DDL scenario ordering with dependency graph and cycle breaking
- **Problem:** Order generated operations so each runs only when its prerequisites hold; break cycles deterministically.
- **Inputs / outputs:** `Operation{kind: Create|Alter|Drop|Rename|Grant|Comment|…, object_path, sql, provides: Set<Effect>, requires: Set<Effect>, destroys: Set<Effect>, phase}` list → ordered list (possibly with split operations) + diagnostics.
- **Algorithm:**
  1. Effects are facts: `Exists(path)`, `Absent(path)`, `HasShape(path, hash)`, `NotReferenced(path)`. Producer declarations: CreateTable provides `Exists(t)`, `Exists(t.c*)`, requires `Exists(schema)`, `Exists(types used)`, `Exists(functions used in defaults/checks/generated cols)`; CreateFK requires `Exists(t)`, `Exists(ref_t)`, `Exists(ref key)`; DropTable requires `NotReferenced(t)`; AlterColumnType requires dependent views absent (PG) and is followed by their re-creation.
  2. Graph edge A→B if `B.requires ∩ A.provides ≠ ∅` or B requires `Absent(x)` and A destroys x.
  3. Kahn's algorithm with a priority queue keyed by `(phase, trie pre-order position)`; phases: 1 drop dependents (FKs, views, triggers), 2 drop objects, 3 rename, 4 create schemas/types/sequences/functions-needed-by-tables, 5 create/alter tables (no FKs), 6 keys and indexes, 7 FKs and checks, 8 views/routines (topological among themselves), 9 triggers, 10 comments, owners, grants.
  4. **Cycle breaking** when the queue stalls: find an SCC; FK cycles → split CreateTable into CreateTable-without-FK + AlterTable ADD FK (same as *After table* placement; DataGrip notes "some foreign key declarations might be postponed"); view↔function cycles → create a stub function, then view, then real function (or report); drop cycles → drop FKs first.
  5. Recreation fallback (ALG-DDL-03) inserts Drop+Create pairs and data-preservation steps; never silent.
  6. Emit with dialect delimiter or `;` if *Use semicolon as statements delimiter*.
- **Data structures:** `petgraph::StableGraph<Operation>`, `BinaryHeap<(phase, trie_pos, seq)>`, `ObjectPathTrie`.
- **Complexity:** O((V+E) log V).
- **Edge cases (exhaustive list):** self-referencing FK (inline allowed vs postponed); mutual FKs; FK to a table in another schema in the same migration; views depending on views; materialized views (refresh needed after recreate); functions used in column defaults/checks/generated columns/index expressions; types (enums/domains/composites) used by tables and functions; enum value add cannot run inside a transaction block before PG12; enum value removal → recreate type and all dependents; sequences owned by columns (drop column drops sequence); triggers referencing functions; dropping a column used by an index/view/FK; renames before alters (and rename chains `a→b, b→a` need a temp name); grants on recreated objects; transactional vs non-transactional DDL (PG vs MySQL/Oracle) — insert checkpoints; PG `CREATE INDEX CONCURRENTLY` must run outside a transaction; MSSQL batch separation (`GO`) required before `CREATE VIEW/PROCEDURE`; Oracle `/` after PL/SQL units; identical-priority operations must keep model order for reproducibility.
- **Test cases:**
  | Input | Expected order |
  |---|---|
  | Origin tables a, b with FKs a→b and b→a; Target empty | CREATE a, CREATE b, ALTER a ADD FK, ALTER b ADD FK (DDL-I-01) |
  | v2 on v1; v1 column list changed (PG) | DROP v2, DROP v1 (or CREATE OR REPLACE if append-only), CREATE v1, CREATE v2 (DDL-I-02) |
  | int→bigint on column used by view (PG) | DROP view, ALTER COLUMN TYPE, CREATE view (DDL-I-03) |
  | new function used in new table's DEFAULT | CREATE FUNCTION before CREATE TABLE (DDL-I-16) |
  | trigger + function added | function, then trigger (DDL-I-09) |
  | rename swap `a↔b` | `a→tmp`, `b→a`, `tmp→b` |
- **Prior art and evidence:** [Observed: `Scenario`, `Effect`, `RequiredEffect`, `LinkedPriorityQueue`, `PreOrderComparator`, `ObjectPathTrie`, `ScenarioDot`, option description on postponed FKs]; pg_dump `pg_dump_sort.c` (PostgreSQL License) boundary-object cycle breaking.
- **Used by:** Hacivat; Karagöz (executes and audits each operation).

### ALG-DDL-03: ALTER vs recreate decision per dialect
- **Problem:** Choose the least destructive statement sequence for each property change.
- **Inputs / outputs:** `(dialect, server_version, object_kind, property, old, new, options{allow_recreate, online})` → `Strategy{Alter(template) | AlterWithUsing | AlterWithDependents(drop+recreate dependents) | Recreate(data_preserving) | Unsupported(reason)}`.
- **Algorithm:** rule table lookup (most specific `version_range` wins) from §3.17; if result is Recreate and `allow_recreate=false` → Unsupported with a message naming the property; for tables Recreate means rename-copy-drop with explicit data copy (SQLite 12-step) and is flagged destructive; for routines Recreate = DROP + CREATE (signature/return change); for views = DROP dependents + CREATE + recreate dependents unless CREATE OR REPLACE is legal (PG: only appended columns, same names/types for existing).
- **Data structures:** `RuleTable: HashMap<(Dialect, Kind, Property), Vec<(VersionRange, Strategy)>>`.
- **Complexity:** O(1) per property.
- **Edge cases (exhaustive list):** narrowing type (data loss → warn, require confirmation); NOT NULL on column containing NULLs (offer default/backfill); identity changes (MSSQL not alterable); collation change with dependent indexes; partitioned tables (some ALTERs must apply to partitions); online index options; MySQL implicit commit; SQLite FK-enforced tables during rebuild; Oracle columns with data restricting MODIFY; PG `USING` expression required for non-implicit casts (text→int); routine overloads (DROP must use full signature); view with `WITH CHECK OPTION`/security barrier properties; MySQL DEFINER preservation on recreate; MSSQL schemabound views blocking column changes.
- **Test cases:** PG `varchar(10)→varchar(20)` → `ALTER COLUMN TYPE`; PG `text→int` → `ALTER COLUMN TYPE int USING col::int`; PG function OUT param added → DROP FUNCTION + CREATE FUNCTION (Observed screenshot); SQLite type change → 12-step rebuild; MSSQL default change → drop named default constraint + add; MySQL column reorder → `MODIFY … AFTER x`; MSSQL identity added with allow_recreate off → Unsupported.
- **Prior art and evidence:** [Observed: `SupportedSinceImpl`, `AlterCapableInvocator`, `UnsupportedCapableInvocator`, `AllowRecreate` option, Object Migration screenshot]; Atlas/migra/Skeema.
- **Used by:** Hacivat, Karagöz.

### ALG-DDL-04: Constraint/index naming templates
- **Problem:** Generate default names from templates (Auto Generate, batch indexes, new keys).
- **Inputs / outputs:** template string, context `{table, columns[], column, ref_table, ref_columns[], schema, unique: bool, …}`, dialect limits, existing names in the namespace → name.
- **Algorithm:** tokenize into literal and `{expr}` parts (`{{` escapes a brace [Hacivat choice]); expressions: variable, or conditional `{var?then:else}` where `var` is boolean (non-empty list = true); list variables joined with `_`; apply dialect identifier length (PG 63 bytes, MySQL 64 chars, Oracle 128 (30 before 12.2), MSSQL 128) with deterministic truncation + 4-hex hash suffix; ensure uniqueness in the dialect's namespace (PG: indexes/constraints schema-wide; MySQL: per table; MSSQL: constraints schema-wide) by appending `_1`, `_2`; letter case per code style (never locale-sensitive).
- **Data structures:** parsed template AST cached per template string.
- **Complexity:** O(len).
- **Edge cases (exhaustive list):** unknown variable (render literally + warn); unbalanced braces; nested conditionals; columns with spaces/quotes/dots (sanitize to `_` vs keep quoted name — option); non-ASCII (Turkish `ş ı ğ İ`, allowed by DBs; optional ASCII transliteration); empty column list (expression index → use `expr`); multi-byte truncation boundaries (PG limit is bytes); name collision after truncation; reserved words as result (quote).
- **Test cases:** `{table}_{columns}_{unique?u:}index` on `persons(FirstName, LastName)`, non-unique → `persons_FirstName_LastName_index` [Observed: help]; unique → `persons_FirstName_LastName_uindex`; `{table}_pk` on `actor` → `actor_pk` [Observed: help]; 70-char PG name → 63-byte prefix with hash; second identical index → `_1` suffix.
- **Prior art and evidence:** [Observed: help, bundle hint, `PatternNameGenerator`, `<D>NamingService`].
- **Used by:** Hacivat.

### ALG-DDL-05: DDL files → virtual data source model
- **Problem:** Build a schema model from SQL files (DDL data source, Script resolve mode, offline design, MCP context).
- **Inputs / outputs:** file set (scope-filtered), dialect, default database/schema names, optional extended real model → model + per-object source locations + diagnostics.
- **Algorithm:** collect files by scope; order files (layout ordinal prefix `NNN-` if present, else path order with dependency-free resolution); parse with dialect (doc 06); replay statements into an in-memory model: CREATE → add (unqualified → default schema; `fileScope(path)` from the layout gives the schema/database), ALTER → patch, DROP → remove, COMMENT/GRANT/OWNER → attach; record `(file, range)` per object; parse errors → diagnostics, never abort; incremental: per-file effect sets cached by content hash, rebuild by retracting that file's effects and re-applying; resolve references against the union with the extended real data source.
- **Data structures:** `Vec<FileEffects{hash, effects}>`, model arena, `HashMap<ObjectPath, SourceLoc>`.
- **Complexity:** O(total statements); incremental O(changed file + dependents).
- **Edge cases (exhaustive list):** object defined twice (last wins + warning, both locations kept for navigation); ALTER in a file processed before its CREATE (two-pass: collect CREATEs first); unsupported statements (ignored with info); `CREATE … IF NOT EXISTS`; `SET search_path`/`USE` inside files; unnamed constraints; files excluded by scope but referenced; DML in DDL files (ignored); mixed dialect files; empty schema names in DDL data sources; file renamed/moved (fileScope changes → object moves schema); encoding/BOM; very large files.
- **Test cases:** single `CREATE TABLE family(...)` file dragged to explorer → DDL data source with table `family` [Observed: help tutorial]; `idx.sql` with `CREATE INDEX ON t(a)` + extended DB containing `t` → index under `t`; `b/v.sql` in by-schema layout → view in schema `b`.
- **Prior art and evidence:** [Observed: help DDL data sources; layout `fileScope`].
- **Used by:** Hacivat (schema-as-code; MCP server for agents without DB access).

### ALG-DDL-06: File-per-object layout (dump)
- **Problem:** Dump a model to files and map files back to objects.
- **Inputs / outputs:** model, layout `{base_name(obj), file_name(ctx)?, file_scope(path)?, ignore_dependencies}`, root path, generator options (qualify, placement, reformat, regenerate) → file set + list of outdated files.
- **Algorithm:** pick "stored separately" objects (schema children and source-bearing objects); order them (dependency order from ALG-DDL-02 if `!ignore_dependencies`, else model order); for each compute `base_name` (nearest separately-stored ancestor; sanitize), then `file_name` with `count` (1-based) and `.sql`; group nested objects (columns, keys, indexes, comments, grants) into the parent's file in a fixed sub-order; generate DDL per file; diff against existing files: write only changed (compare normalized text so user formatting survives when semantics unchanged), list files that would be deleted and ask ("Outdated Files"); offer to add new files to the DDL data source.
- **Data structures:** `BTreeMap<RelPath, FileContent>`.
- **Complexity:** O(objects + output size).
- **Edge cases (exhaustive list):** names containing `/` (DataGrip: word `slash`), `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|` (Windows), trailing dot/space (Windows), reserved names `CON`, `PRN`, `AUX`, `NUL`, `COM1`…; case-insensitive file systems (`Users` vs `users` → collision suffix); macOS NFD vs NFC (normalize to NFC); Turkish dotless/dotted i in names on case-insensitive volumes; path length limits; overloaded routines (same base name → need signature suffix; DataGrip behavior [Unknown]); objects without schema (root) or without database (`anonymous/` in by-database layout); ordinal renumbering when one object is inserted (with-order layout shifts all later files — VCS noise; Hacivat option: sparse numbering); deletions of user-added non-DDL files (never delete files the model did not generate).
- **Test cases:** schema `app`, table `a/b` → `app/aslashb.sql` (DataGrip parity) and Hacivat-safe `app/a%2Fb.sql` option; by-schema-and-type: view `v` in `app` → `app/view/v.sql`; with-order: tables `t1`, `t2` where `t2` FK→`t1` → `001-t1.sql`, `002-t2.sql`; by-database: schema `s` with no database → `anonymous/s/x.sql`.
- **Prior art and evidence:** [Observed: 5 layout scripts, help New File Layout].
- **Used by:** Hacivat.

### ALG-DDL-07: Modify-dialog edit session → minimal change script
- **Problem:** Keep a structured editor, an editable SQL preview and the underlying model consistent, emitting only the delta.
- **Inputs / outputs:** original model subtree `M0`, user edits (applier operations), dialog settings (Action, Script, Rename in, generator flags) → working model `M1`, preview SQL, final action.
- **Algorithm:**
  1. Copy `M0` → `M1`; every UI field binds to an *applier* (name, single property, list property, key/index/FK column lists, grants, source text); edits mutate `M1` and push an undo record per object tab (for *Revert*).
  2. Debounced (e.g. 200 ms) async regeneration: `changes = diff(M0, M1)` (ALG-DDL-01 restricted to the subtree, with surrogate-name and implicit-object rules) → producers → ALG-DDL-02 → text; properties equal to the DBMS default are not emitted (minimal clauses).
  3. Action setting: *Modify existing* → ALTER path; *Create modified* → CREATE for changed objects; *Create all* → CREATE for the whole subtree.
  4. **Reverse sync** ("Updating structure from preview"): if the user edits the preview, parse it (dialect parser), apply statements to a fresh copy of `M0` (ALG-DDL-05 replay) → new `M1'`; if parse fails, keep text as user-owned and stop regenerating (show a "preview edited manually" banner); on OK, user-owned text is executed verbatim.
  5. OK: Script = Execute → run via execution engine (doc 07) in the data source's transaction mode, refresh affected objects; Open in editor → console; Replace existing DDL → rewrite source ranges in DDL files (ALG-DDL-15).
  6. Cancel with changes + OT-1 on → confirm.
- **Data structures:** `EditSession{m0, m1, undo: HashMap<ObjectPath, Vec<Edit>>, preview: PreviewState{Generated(text)|UserOwned(text)}}`.
- **Complexity:** diff restricted to subtree, O(size of subtree).
- **Edge cases (exhaustive list):** rename + property change of the same column (rename first, then alter using new name); add column + add index on it in one session (column first); delete a column referenced by a key/index in the dialog (cascade the dialog model or block); Make Primary Key on nullable column (add NOT NULL); Auto Generate name after columns change (re-evaluate only if name still auto-generated); disabled fields (never emitted); grants to non-existent roles; virtual objects (persist to external-data file, no SQL); SQLite structural change → whole-table rebuild preview; preview edited then structure edited again (ask which wins); concurrent introspection updating `M0` while the dialog is open (warn, rebase).
- **Test cases:** add `country varchar(3) default 'N/A' not null` with comment on PG `actor` → `ALTER TABLE actor ADD country …;` + `COMMENT ON COLUMN actor.country IS …;` [Observed]; create PG role with login+superuser+createdb+createrole, inherit default → `CREATE USER guest SUPERUSER CREATEDB CREATEROLE;` [Observed]; no edits → empty preview, OK disabled or no-op; edit preview to add `NOT NULL` → Not Null checkbox becomes checked.
- **Prior art and evidence:** [Observed: screenshots, `TableDialogSettings.*`, progress titles, applier/builder class names].
- **Used by:** Hacivat (object dialogs), MCP (`propose_ddl` tool returns the preview script, never executes without approval).

### ALG-DDL-08: Generator option application
- **Problem:** Apply ~45 options consistently across categories, dialects and entry points (generator window, Confirm Drop, dialogs, dumps, one-shot actions).
- **Inputs / outputs:** selection, category, option values, dialect capabilities → script.
- **Algorithm:**
  1. **Applicability**: each option declares `(categories, dialects, min_version)`; hidden when not applicable (e.g. Skip DEFINER only for MySQL/MariaDB, VARCHAR2 only Oracle, MS VS-compat only MSSQL); unknown → default.
  2. **Selection expansion**: category determines the object set — CreateComplete = selection + nested (if Include nested) + Consider-* kinds; CreateBefore = tables/types/sequences without FKs/indexes/triggers; CreateAfter = the complement; Drop/Truncate/Delete = selection in reverse dependency order.
  3. **Producer flags**: map options to flags (`or_replace`, `if_not_exists`, `if_exists`, `cascade`, `qualify(Auto|Never|Always)`, `constraint_context`, `surrogate_names`, `skip_grants`, `skip_owner`, `seq_next_value`, `semicolon`, `ignore_extension_members`, `allow_recreate`, …); producers consult flags only through the capability layer (e.g. `or_replace` ignored for tables in PG, honoured for views).
  4. **Qualification** (ALG-DDL-13) and **constraint placement** (ALG-DDL-14) applied during emission.
  5. **Reformat**: run the formatter (doc 06 ALG-SQL-08) on generated fragments only, never on server-provided text; **Regenerate DB-provided code**: rebuild properties from the model instead of passing source text through.
  6. Output: clipboard, file(s) via layout (ALG-DDL-06), console.
- **Edge cases (exhaustive list):** contradictory options (IF NOT EXISTS + OR REPLACE → prefer OR REPLACE where supported, else IF NOT EXISTS); CASCADE with IF EXISTS ordering of keywords per dialect; Skip grants with *Show grants in user definition*; surrogate names off but name required by syntax (MSSQL named default constraints when altering) → keep; sequence next value when sequence never used (omit); disabled constraints with *With disabling clauses* off (create enabled! — warn); semicolon option in T-SQL scripts containing `CREATE PROCEDURE` (needs batch separation → keep `GO`); Oracle `/` after PL/SQL; extension members in PG; drop of objects with dependents outside the selection.
- **Test cases:** PG view, OR REPLACE on → `CREATE OR REPLACE VIEW`; PG table, OR REPLACE on → plain `CREATE TABLE` (not applicable); MySQL procedure, Skip DEFINER → no DEFINER clause; Drop with IF EXISTS + CASCADE on PG table → `DROP TABLE IF EXISTS t CASCADE`; Truncate PG with Restart identity + CASCADE → `TRUNCATE TABLE t RESTART IDENTITY CASCADE`; Truncate with *DELETE instead* → `DELETE FROM t`.
- **Prior art and evidence:** [Observed: `scriptGen.*` labels/descriptions, screenshots, `Flag/Flags/FlagValue`].
- **Used by:** Hacivat; MCP `generate_ddl` tool.

### ALG-DDL-09: Object source edit round-trip and conflict detection
- **Problem:** Let users edit stored source (views, routines, triggers, packages) as text, submit safely, and detect concurrent changes.
- **Inputs / outputs:** cached server source `S_base` (with introspection watermark), local text `L`, fresh server source `S_now` on submit → migration script or conflict resolution.
- **Algorithm:**
  1. Open: present `S_base` unqualified (strip current-schema qualifiers) in an editor backed by a virtual file; record `base_hash`.
  2. Edit: store `L` as a local version; compute line diff `S_base`↔`L` for gutter markers; mark object **blue**.
  3. Optional staleness check on open (if DBMS supports watermark queries): fetch server change marker (PG `xmin`/catalog age, MSSQL `modify_date`, Oracle `last_ddl_time`) → if changed, notify *Synchronize* / *Disable check*.
  4. Submit: re-fetch `S_now`; if `hash(S_now) ≠ base_hash` → conflict dialog: Force (proceed with `L`), Abort & Synchronize (refresh `S_base := S_now`, recompute status; if `L` still differs from both → **red**, offer Revert / Keep / 3-way Merge with base `S_base_old`, left `L`, right `S_now`).
  5. Parse `L` → model fragment `F`; diff `F` against cached object (ALG-DDL-01, single object) → producers (ALG-DDL-03 chooses CREATE OR REPLACE / ALTER / DROP+CREATE) → Object Migration preview (shown if invalid-or-setting-on; otherwise execute silently).
  6. Execute in a transaction where supported; on success re-introspect the object and **replace local text with server text** (`revert.on.success`), clear local version, mark **black**; on failure keep `L`, show error, stay **blue**.
- **Data structures:** `SrcEntry{path, base_text, base_hash, watermark, local_text?, status: InSync|Modified|Conflict}`; persisted per data source (zip-like store).
- **Complexity:** diff O(n·d) (Myers).
- **Edge cases (exhaustive list):** server reformats stored text (PG views — submitted text never matches stored; must not be flagged as conflict afterwards); case/whitespace-only edits; renaming the object inside the text (treat as rename or drop+create? — ask); changing signature (DROP + CREATE; dependent objects/grants lost → re-grant); dependent views on a replaced function; editing several objects then submitting a subset; invalid PL/SQL that the DB accepts with errors (Oracle "created with compilation errors" — surface warnings); triggers whose function changed; objects dropped on server meanwhile (conflict: recreate?); DBMS without watermark (no staleness check — DataGrip shows "Unsupported"); MySQL DEFINER present in text (preserve or drop per option); encrypted/wrapped sources (Oracle WRAPPED, MSSQL WITH ENCRYPTION) not editable; huge package bodies.
- **Test cases:** edit PG view body, submit → `CREATE OR REPLACE VIEW` (DDL-I-24); add OUT param to PG function → DROP FUNCTION old signature + CREATE FUNCTION [Observed]; concurrent change by another session then Submit → Possible Loss of Changes dialog; Abort & Synchronize with non-overlapping edits → Merge auto-resolves; after success, editor text equals server text.
- **Prior art and evidence:** [Observed: help, bundle `changed.in.database.*`, `SqlStorageInspection`, `srcStorage/*` names, registry].
- **Used by:** Hacivat; Karagöz (audits submitted source change as one operation with before/after hashes).

### ALG-DDL-10: Rename detection (suggested renames in diff)
- **Problem:** Avoid destructive drop+add when an object was renamed.
- **Inputs / outputs:** unmatched Removed set `R` and Added set `A` under the same parent and kind → candidate pairs with scores; user-confirmed `Renamed` changes.
- **Algorithm:** for each `(r, a)` of same kind: columns → score = 0.5·[same type] + 0.2·[same nullability] + 0.2·[same default] + 0.1·[same ordinal]; tables → Jaccard of column signatures (name, type) ≥ 0.8 plus equal key structure; indexes/constraints → identical definition ⇒ score 1; routines → equal body + signature, different name. Solve as maximum-weight bipartite matching (Hungarian, or greedy by score when small) with threshold τ (0.8); mode Suggest shows them as "rename?" (default unchecked), Auto applies.
- **Data structures:** score matrix per parent.
- **Complexity:** O(|R|·|A|) scoring; Hungarian O(n³) for n = min(|R|,|A|) (n small).
- **Edge cases (exhaustive list):** swap renames (`a↔b`); rename + type change (score below τ → drop+add); several identical candidates (ambiguous → don't suggest); cross-schema move (treat as `ALTER … SET SCHEMA` candidate); constraint renames caused by table rename (system names); renamed table whose columns also renamed; DBMS without RENAME support (SQLite < 3.25 column rename → rebuild).
- **Test cases:** `t(a int)` vs `t(b int)` → suggest rename `a→b` (score 1.0); `t(a int)` vs `t(b text)` → no suggestion; tables `users(id,name)` vs `accounts(id,name)` → suggest table rename.
- **Prior art and evidence:** DataGrip does not show renames (DDL-I-05, [Inferred]); Atlas/Skeema require explicit hints — Hacivat improvement.
- **Used by:** Hacivat.

### ALG-DDL-11: Implicit-object classification ("Ignore Implicit Objects")
- **Problem:** Identify objects created automatically by the DBMS so compare/generation can skip them.
- **Inputs / outputs:** object + catalog facts → `implicit: bool, owner: ObjectPath?`.
- **Algorithm (rules):** PG: index backing a PK/UNIQUE/EXCLUDE constraint (`pg_constraint.conindid`); sequence owned by a serial column (`pg_depend` 'a'); identity sequence ('i'); array/row types of tables; TOAST tables. MySQL: index auto-created for an FK when no usable index existed (same name as FK); PK index. MSSQL: DEFAULT constraints with system names (`is_system_named`), PK/UQ backing indexes, statistics. Oracle: `SYS_C…` constraints, LOB segments/indexes, PK/UK indexes with same name. SQLite: `sqlite_autoindex_*`. Implicit objects inherit the change kind of their owner and are emitted only through it.
- **Edge cases (exhaustive list):** user-created index that coincides with an FK (explicit, keep); serial converted to identity; PG `ALTER INDEX … RENAME` on an implicit index (now named differently — still implicit); MSSQL user-named default constraints (explicit name but still owned by column); MySQL dropping FK leaves its index (becomes explicit).
- **Test cases:** PG `id serial primary key` → table, column, implicit sequence + implicit PK index; with ignore_implicit, a missing `t_id_seq` on Target is not reported separately; MSSQL default `DF__t__c__1234` vs `DF__t__c__9876` → identical.
- **Prior art and evidence:** [Observed: help option text naming FK indexes and MSSQL default constraints].
- **Used by:** Hacivat.

### ALG-DDL-12: Normalization for comparison (types, defaults, source text, references)
- **Problem:** Avoid false differences caused by server canonicalization and container names.
- **Inputs / outputs:** object property values + context (dialect, compared root container) → normalized values.
- **Algorithm:** (1) types: alias table per dialect (`int4→integer`, `bool→boolean`, `character varying→varchar`, MySQL display widths dropped for 8.0.19+, `timestamp without time zone→timestamp`); (2) defaults/check predicates/index expressions: parse as expressions, strip redundant casts the server adds (`'a'::text` vs `'a'`), redundant parentheses, normalize literals; (3) references: rewrite qualified names whose qualifier equals the compared root container (or the current schema) to unqualified; (4) source text (views/routines): tokenize with dialect lexer, drop comments only if the DBMS doesn't store them, collapse whitespace, case-fold keywords, compare token streams; PG views compare the server-rendered text on both sides; (5) collations/charsets: resolve "default" to the effective value when known.
- **Edge cases (exhaustive list):** casts that change semantics (`'1'::int` vs `'1'`) — keep when types differ; defaults with `now()` vs `CURRENT_TIMESTAMP`; sequences in different schemas but not the compared root (keep qualifier); string literals containing schema names (don't rewrite inside literals); quoted vs unquoted identical names; Turkish-locale case folding must not be used (ASCII folding for keywords); version-dependent rendering (PG 15 vs 16 `pg_get_viewdef` differences).
- **Test cases:** `nextval('testing.seq'::regclass)` in root `testing` vs `nextval('seq'::regclass)` in root `public` → equal [Observed]; `varchar(45)` vs `varchar` → different [Observed]; view text differing only in whitespace and keyword case → equal.
- **Prior art and evidence:** [Observed: screenshots]; migra normalization.
- **Used by:** Hacivat.

### ALG-DDL-13: Schema qualification (Auto / Never / Always)
- **Problem:** Decide whether to prefix object names with schema in generated code.
- **Inputs / outputs:** set of objects in the script, current schema (login/selected), mode → per reference: qualify or not.
- **Algorithm:** Always → qualify all; Never → none; Auto → qualify none if every object in the script belongs to one schema **or** that schema is the current one; otherwise qualify all references outside the current schema (DDL data source dumps: help wording "qualify if identical names exist in several schemas" → also qualify names that are ambiguous across schemas). Server-provided text is never rewritten.
- **Edge cases (exhaustive list):** cross-schema FK references; search_path with several schemas (PG); MySQL databases as schemas; MSSQL default schema per user (`dbo`); temporary schemas; Go to DDL editor always unqualified (editing) — submit must re-qualify using the object's real schema.
- **Test cases:** selection `app.t1, app.t2`, current `public`, Auto → unqualified (single schema); selection `app.t1, sales.t2`, current `app` → `t1` unqualified, `sales.t2` qualified.
- **Prior art and evidence:** [Observed: option description, help].
- **Used by:** Hacivat.

### ALG-DDL-14: Constraint placement
- **Problem:** Place constraints inline in columns, inline in the table, or after it, honouring cycles.
- **Inputs / outputs:** table with constraints, mode `{InsideColumn, InsideTable, AfterTable}`, cycle set from ALG-DDL-02 → CREATE TABLE text + trailing ALTERs.
- **Algorithm:** for each constraint: if part of an FK cycle (or references a table created later) → AfterTable regardless of mode; else InsideColumn and single-column → column clause (`PRIMARY KEY`, `REFERENCES t(c)`, `CHECK (…)` when predicate references only that column, `UNIQUE`); else InsideTable → table element after columns; AfterTable → `ALTER TABLE ADD CONSTRAINT`. Names kept only if user-named or surrogate names on; disabled constraints get inline disable clause unless `separate.disable`.
- **Edge cases (exhaustive list):** multi-column PK in InsideColumn mode (goes to table level); check referencing two columns; self-referencing FK (inline legal in most DBs); SQLite (constraints cannot be added later → force inside table, and cycles are legal because FKs are not checked at creation); MySQL inline `REFERENCES` in column definitions is **parsed but ignored** by InnoDB (must use table-level FK); MSSQL named defaults (default constraint inline vs ALTER ADD CONSTRAINT DEFAULT FOR); deferrable FKs.
- **Test cases:** PG `t(id int, CONSTRAINT pk PRIMARY KEY(id))`, InsideColumn → `id int CONSTRAINT pk PRIMARY KEY`; MySQL FK, InsideColumn → table-level FK; cycle a↔b, InsideColumn → FKs postponed to ALTER.
- **Prior art and evidence:** [Observed: option text incl. postponed FKs, observed default Inside column].
- **Used by:** Hacivat.

### ALG-DDL-15: DDL mapping synchronization
- **Problem:** Keep a real data source and a DDL file tree in sync in both directions.
- **Inputs / outputs:** mapping `{data_source, ddl_data_source, scope: [schema paths]}`, direction `UpdateFiles | ApplyToDb | Dump` → file edits or migration script.
- **Algorithm:**
  - UpdateFiles (DB → files): introspect scope; build model from files (ALG-DDL-05); diff (ALG-DDL-01, files as Target); for each change: Added → new file per layout; Modified → regenerate that object's file region (Script = Replace existing DDL), preserving untouched files byte-for-byte; Removed → propose file deletion.
  - ApplyToDb (files → DB): diff with files as Origin, DB as Target → Migration dialog (user selection, editable script) → execute → re-introspect.
  - Dump: full regeneration through the layout; outdated-file deletion confirmation.
- **Edge cases (exhaustive list):** scope excludes a schema referenced by included objects; files outside DDL data source scope; local edits to files not yet applied (UpdateFiles must warn before overwriting); objects defined in multiple files; mapping to a deleted data source; DBMS mismatch between DDL data source dialect and real data source; MongoDB (unsupported); VCS merge conflicts in files (parse errors → block apply).
- **Test cases:** create `actor_1` in PG, *Update from PostgreSQL* → new `actor_1.sql` [Observed: help example]; add `actor_1.sql` to files, *Apply to PostgreSQL* → Migration dialog with CREATE TABLE [Observed: help example]; delete table in DB, *Dump to* → "Outdated Files" prompt.
- **Prior art and evidence:** [Observed: help `ddl-mappings`, bundle messages].
- **Used by:** Hacivat (schema-as-code); Perde could store mapping metadata per org [Hacivat idea].

---

## 5. Implications for Hacivat / Karagöz / Perde

1. **Crates**: `schema-model` (typed objects + property metamodel per dialect family; the same metamodel drives dialog field generation, as DataGrip does since 2022.1), `schema-diff` (ALG-DDL-01/10/11/12), `ddl-gen` (producers with effect declarations, rule tables ALG-DDL-03, options ALG-DDL-08, qualification/placement ALG-DDL-13/14), `ddl-schedule` (ALG-DDL-02), `ddl-files` (ALG-DDL-05/06/15), `src-edit` (ALG-DDL-09).
2. **UX**: Modify dialog = metamodel-driven form + tabs + live, editable, bidirectional SQL preview (ALG-DDL-07); Compare = aligned table-tree with include checkboxes + three tabs; Database Changes panel with blue/red status and 3-way merge; SQL Generator as a side panel with categories.
3. **Safety by default**: destructive operations (DROP, type narrowing, recreate) flagged in the preview and require explicit confirmation; *Show preview of valid script* **on** by default (DataGrip's silent submit is a footgun); EXPLAIN-like "dry-run" of migrations inside a rolled-back transaction where DDL is transactional (PG, SQLite, mostly MSSQL).
4. **Karagöz**: receives migrations as an ordered list of operations with `(kind, object_path, destructive: bool, sql)` → can authorize per object ("may ALTER tables in schema app, may not DROP") and audit per operation; Perde can require approval tokens for DDL on protected databases, verified at Karagöz (P2P path bypasses Perde).
5. **Schema as code**: DDL data sources + layouts + mappings give a Git workflow; prefer sparse ordinal numbering to avoid renumbering noise; MCP exposes the file-based model to agents offline.
6. **MCP tools**: `generate_ddl(objects, options)`, `compare_schemas(a, b, options)` (returns diff tree + script, never executes), `propose_migration` (requires human approval in Hacivat UI).

## 6. i18n considerations

- All labels (categories, options, dialog fields, colors legend, conflict dialogs) are message IDs; property names in *Object Properties Diff* come from the metamodel and need translation keys, while values stay verbatim.
- Generated SQL and comments inside it are **never localized**; error messages from the server shown verbatim.
- Counts and ChoiceFormat-style labels ("Mapping: X and N more", "Columns (4)", "1 difference", "and {0} files more") use plural-aware messages (Fluent/ICU), not concatenation — Turkish has no plural agreement after numerals but other languages do.
- Name templates with Turkish characters: optional transliteration; identifier case changes never use the Turkish locale (`I`→`ı` bug).
- File names from object names: NFC normalization and locale-independent case-insensitive collision checks (Turkish İ/ı).
- Color-only status (black/blue/red, green/blue/red) needs text/icon equivalents for accessibility in every locale.

## 7. Risks, pitfalls, open questions

- **Help contradiction on the preview setting**: `update-source-code-of-database-objects` says the migration preview for a valid script is *not* shown by default ("the valid migration script will be executed silently"), while `settings-tools-database-other` says "By default, the Object Migration dialog is displayed". Record the actual default in doc 13 (SET family); Hacivat defaults to *show*.
- Help lists "Target Name" for primary/unique keys — probably a copy-paste from the FK table [Unknown].
- Semantics of `TableDialogSettings` values (Action/Script/Rename in) are inferred from labels — verify in doc 13 (new cases proposed: DDL-I-26 Action=Create all objects; DDL-I-27 Script=Replace existing DDL in a DDL data source).
- The "12 / 12" badge and *Detect column insertion* checkbox semantics are [Unknown]; *Dump* button in the Migration dialog: visibility [Unknown].
- DataGrip's exact ordering, cycle handling and ALTER/recreate choices are not observable statically — doc 13 family DDL records generated scripts (DDL-I-01..25).
- Dialect coverage cost is high (1,282 producer classes); Hacivat: PG + SQLite first (SQLite rebuild path is unique), then MySQL, MSSQL.
- Source normalization is error-prone (servers rewrite SQL); default to semantic comparison with an "ignore source" escape hatch.
- Automatic recreate can lose data; must never be silent; MySQL/Oracle non-transactional DDL needs resumable migrations.
- Layout sanitization (`/` → `slash`) is lossy and not reversible; Hacivat should use reversible escaping and keep a manifest.
- Only the "with order" layout preserves dependency order; loading other layouts requires order-independent model building (ALG-DDL-05 two-pass).
- Open: whether DataGrip detects renames in Compare Structure (likely not; DDL-I-05); whether *Ignore Case* also affects source text; how overloaded routines are named in file layouts.
