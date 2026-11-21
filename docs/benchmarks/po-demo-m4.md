# M4 PO demo: "Metadata and explorer" checklist

This is the part of the M4 gate (Task 33) only the PO can do: the hand demo of plan Task 33
Step 4 (steps 1–12), on the PO's Mac, against real Docker servers, with a real window and
keyboard, plus the Step 3 measurements. Each step lists its precondition, what to do (exact SQL,
commands and keys), the expected screen (app text in `EN`), and the automated tests that already
cover it. Write each result in the tables in section 4.

- Gate record: `docs/benchmarks/m4.md` (its section 0 orders everything the PO does).
- G1 bench definitions: `docs/benchmarks/m4-bench-notes.md`.
- The gate agent did **not** pre-run this list: the draft was written without Docker or a build.
  The controller's pre-run fills the "Pre-run" column in section 4 before the PO starts.
- **Terms:** "transaction", "commit" and "rollback" stay in English in the Turkish UI (PO ruling).
  Shortcuts follow VS Code's defaults and can be changed in Settings.
- Earlier demo lists were in Turkish; this one is in English, like the gate record.

## What is new since the Task 29 draft

Task 29 started this file with one item (the encrypted G1 variant and the real Keychain read; now
step 1b). Since then:

- **Settings › General › Time zone** (PO R-W5-14 a, T32): a row after Regional format with
  "System (<zone>)", UTC and 21 IANA zones west to east; a hand-edited unknown zone shows as
  "<name> (unknown)" and the system zone is used. Step 10b.
- **Clear query history** (PO R-W5-14 b, T32): "Clear query history…" in the connection menu and
  "Clear history of this connection…" in the history dialog's ⋯ menu; Cancel is the default.
  **With damaged backups:** when rotated `history.sqlite.corrupt-*` files exist, the dialog says
  so and offers "Also delete damaged history backups" (default off). That option lands with T33b,
  which is not merged at `d5af420`: check it is on the build first. Step 10c.
- **Built-in badge** (T32): the Built-in group shows "version 17" when the shipped snapshot
  answers for it (not when a live read of a system schema does). Step 9.
- Control characters in names are drawn visibly (`␊`, `␇`), and names with invisible characters
  carry a badge (T32). Optional check in step 4.

## 0. Preparation

### 0.1 Build

The main checkout on `m4` at the gate tip (`d5af420` or later, after T33b merges). Check:

```
git log -1 --oneline
```

### 0.2 Docker stack

The local `hacivat-m2a` stack has been down since C-0, and the M4 Docker proofs ran on CI's
fresh stack (R-W2-14). For the demo the PO starts it. Run from the **main checkout** (the bind
mounts point into the checkout that runs `up`):

```
cargo xtask docker up --set full
cargo xtask docker status --set full
```

- `up` waits up to 180 s for every service to be healthy. The first time it also builds the
  `pg17wrap` image (C-6, approved with Q18).
- Good: `m2a-pg17` and `m2a-mysql84` `healthy`. The demo uses only `pg17` and `mysql84`
  (`--set pr` would start just those plus the SSH bastions).
- **Known state (C-2 never ran locally):** the MySQL/MariaDB containers keep the data volumes
  they had before T1c, so the T1c logins (`hv_catalog`, `hv_fixture`, `hvm4_*` grants) are not
  there. The demo needs only `hv_test`. `up` recreates `mysql80`/`mysql84` with the T1c option
  `--log-bin-trust-function-creators=1`, which step 8's trigger needs.
- **Never run `cargo xtask docker down`** for the demo: it removes the containers' volumes (that
  is Q20 / C-9). To pause the stack without data loss: `docker compose -p hacivat-m2a stop`.
- Do not run the demo while CI-style Docker test runs or other agents use the stack.

### 0.3 The 15,000-table fixture (and the D-038 numbers)

```
cargo xtask docker test --set pr -p hacivat-introspect --test docker_big_catalog
```

- It creates schema `hv_m4_big` (15,000 tables, 160,000 columns) in database `hv_test` on `pg17`,
  in batches of 250 tables, unless it is already there.
- It prints `HV-FACT g1 hv_m4_big pg17 … cold Structure … ms, cold Names … ms` (debug build).
  Copy that line into `m4.md` E1 (R-M4-21's D-038 evidence).
- It writes the G1 fixture (b) cache to `target/docker/g1-hv_m4_big.v1.sqlite` (section 2.1).
- Do not add tables to `hv_m4_big`: the test asserts exactly 15,000.
- It is dropped after the demo (C-9, section 3).

### 0.4 Test logins

Throwaway test values (`docker/m2a/README.md`, `docker/m2a/compose.yml`): user `hv_test`, password
`hv_test_pw`, database `hv_test`. Ports on `127.0.0.1`: `pg17` 54317, `mysql84` 33084. "Another
tool" below means the server's own client inside its container:

```
docker exec -it m2a-pg17 psql -U hv_test -d hv_test
docker exec -it m2a-mysql84 mysql -uhv_test -phv_test_pw hv_test
```

### 0.5 Demo objects

In `psql` (0.4), not in Hacivat, so the app has seen nothing yet:

```sql
CREATE SCHEMA IF NOT EXISTS demo_tr;
CREATE TABLE IF NOT EXISTS demo_tr."İstanbul_şube" (id int PRIMARY KEY, ad text);
CREATE TABLE IF NOT EXISTS demo_tr."Müşteri Siparişleri" (
  id int PRIMARY KEY,
  "müşteri_adı" text NOT NULL,
  tutar numeric(12,2)
);
CREATE SCHEMA IF NOT EXISTS demo_guard;
DO $$ BEGIN
  FOR i IN 1..24 LOOP
    EXECUTE format('CREATE TABLE IF NOT EXISTS demo_guard.g%s (id int)', i);
  END LOOP;
END $$;
CREATE SCHEMA IF NOT EXISTS demo_revoke;
CREATE TABLE IF NOT EXISTS demo_revoke.r1 (id int);
DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'hv_demo') THEN
    CREATE ROLE hv_demo LOGIN PASSWORD 'hv_demo_pw';
  END IF;
END $$;
GRANT CONNECT ON DATABASE hv_test TO hv_demo;
GRANT USAGE ON SCHEMA demo_revoke TO hv_demo;
GRANT SELECT ON ALL TABLES IN SCHEMA demo_revoke TO hv_demo;
```

In the MySQL client (0.4):

```sql
CREATE TABLE IF NOT EXISTS demo_trg (id INT PRIMARY KEY, n INT);
```

### 0.6 App and profiles

```
cargo run --release -p hacivat -- --lang en
```

Create these profiles (⌘N, "Save and connect"):

| Profile | Engine | Port | User / password | Environment | Encrypt cached metadata | Schemas (scope) | Steps |
|---|---|---|---|---|---|---|---|
| `pg17-dev` | PostgreSQL | 54317 | `hv_test` / `hv_test_pw` | `dev` | off (the default for dev) | default `@:*` | 1–7b, 9–12 |
| `pg17-prod` | PostgreSQL | 54317 | `hv_test` / `hv_test_pw` | `production` | **on** (the default for production) | default | 1b |
| `pg17-demo` | PostgreSQL | 54317 | `hv_demo` / `hv_demo_pw` | `dev` | off | Pattern `@:demo_revoke` | 7a |
| `mysql84` | MySQL | 33084 | `hv_test` / `hv_test_pw` | `dev` | off | default `*` | 8 |

All use database `hv_test`.

### 0.7 Seed the caches

1. Connect `pg17-dev`. Expand its root, then `hv_test`, `demo_tr` and its TABLES group, and
   `public`. Do not expand `hv_m4_big` yet.
2. Connect `pg17-prod`. Expand its root and `hv_test`.
3. Quit with both roots expanded (⌘Q). The caches are now on disk; `pg17-prod`'s is encrypted.

## 1. The demo (plan Task 33 Step 4)

### Step 1: offline start, cached dev profile

- **Precondition:** 0.7 done; the server is unreachable: `docker stop m2a-pg17`.
- **Do:** `cargo run --release -p hacivat -- --lang en`.
- **Expected:** `pg17-dev` comes back expanded; its tree (`hv_test`, the schemas, `demo_tr` with
  its TABLES group and both Turkish tables) paints well under a second with no connection; the
  root says "Showing cached metadata from <date, time>"; no error dialog; nothing connects.
- **Pre-verified:** `explorer_live.rs` (T24); the G1 bench (section 2.1).

### Step 1b: offline start, encrypted production profile

- **Precondition:** as step 1; `pg17-prod` was expanded at quit (0.7).
- **Do:** look at `pg17-prod` during the same start. Then measure the Keychain read of its
  `CatalogKey` outside the app (the command never prints the secret, because `-w`/`-g` are not
  given; debug builds use the service prefix `hacivat-dev`):

  ```
  time security find-generic-password -s 'hacivat/<profile uuid>/catalog-key' -a hacivat
  ```

  The profile UUID is the file name of `profiles/<uuid>.toml` in Hacivat's data folder. Measure
  the first read after a restart (it may show a Keychain prompt) and a later one separately.
- **Expected:** the Keychain is read once (one prompt at most); the window never freezes while the
  key is read; the tree follows when the key arrives. The root's tooltip: "The cached metadata is
  encrypted. Opening this connection reads its key from the Keychain." An encrypted profile that
  was **collapsed** at quit stays collapsed and makes no Keychain call until it is opened.
- **Then (E1, the encrypted time):** re-run the bench with the measured delay:

  ```
  cargo run --release -p hacivat-app --features bench --example catalog_cold -- \
    --fixture target/bench/g1-160k.v1.sqlite --runs 10 --offline --variant encrypted \
    --key-delay-ms <measured ms>
  ```

  Write the encrypted p95 next to this step (section 4.2). The bench's own encrypted variant uses
  a memory secret store and never touches the Keychain, so its 147.3 ms (T29, under load) leaves
  the real read out. **If the encrypted p95 is above 300 ms, it is a PO decision**, written here
  and not hidden in `m4.md`. Options: accept it; read the key before the window opens; or show the
  encrypted tree locked until the key arrives. Today the UI is never blocked while the key is read.
- **Pre-verified:** `encrypted_profile_not_expanded_makes_no_keyring_call_at_start`,
  `encrypted_expanded_profile_paints_after_key_without_blocking_ui` (`explorer_live.rs`).

### Step 2: connect; stale badges clear without a full re-read

- **Precondition:** step 1 window open. `docker start m2a-pg17`, then wait for
  `cargo xtask docker status --set pr` to show it `healthy`.
- **Do:** connect `pg17-dev` (its context menu, Connect).
- **Expected:** the "Showing cached metadata from …" line goes; stale marks clear; the namespaces
  are checked incrementally: no "Loading…" row over every group, the expanded rows stay in place,
  nothing collapses.
- **Pre-verified:** `on_connect_with_cache_publishes_stale_then_runs_incremental_not_full`
  (`hacivat-introspect/tests/engine.rs`); G4 (`m4.md` E4).

### Step 3: `hv_m4_big` and a Turkish filter

- **Precondition:** `pg17-dev` connected. UI in Turkish (Settings › General › Language: Türkçe),
  so the explorer filter is Turkish-tolerant (R-M4-50, D-074).
- **Do:** expand `hv_test` › `hv_m4_big` › TABLOLAR; scroll through it fast. Then focus the explorer
  filter (⌥⌘F) and type `istanbul`.
- **Expected:** the TABLOLAR header counts 15,000; scrolling stays smooth; if the schema shows a level
  pill, it reads Names (15,000 tables is Structure only for the current schema, D-038; `public`
  is current), so columns load when a table is expanded. The filter `istanbul` finds `İstanbul_şube` in `demo_tr`; names are shown exactly as
  stored.
- **Pre-verified:** T14 perf (filter median 47 ms over 165k names; apply p95 2.9 ms); shot
  `m4-tree-15000`.

### Step 4: View DDL of a table with a Turkish name

- **Precondition:** UI back in English (Settings › General › Language: English). `pg17-dev`
  connected.
- **Do:** select `demo_tr` › `Müşteri Siparişleri`, press F12 (View DDL). Toggle Qualification in
  the options strip. Press Copy in the toolbar and paste into a new console.
- **Expected:** a read-only tab "Müşteri Siparişleri · DDL"; a `CREATE TABLE` whose table name is
  in double quotes (`"Müşteri Siparişleri"`), `"müşteri_adı"` quoted, `id` and `tutar` bare, the
  primary key inside the table (the default); the caption "Generated from the catalog"; no
  "Definitions provided by server" toggle on PostgreSQL. Qualification switches between
  `demo_tr."Müşteri Siparişleri"` and the bare name. The pasted text equals the shown text.
- **Optional (T32):** in `psql`, `CREATE TABLE demo_tr.U&"x\200Dy" (id int);` makes a name with
  a zero-width joiner between `x` and `y`. After F5 on `demo_tr`, its row carries the
  invisible-characters badge, and the tooltip names `U+200D ZERO WIDTH JOINER`.
- **Pre-verified:** T19 goldens, T7 `quote_roundtrip`, T25 DDL view tests (29/0), shots `m4-*`.

### Step 5: refresh after DDL, also inside a transaction

- **Precondition:** a console on `pg17-dev`; refresh after DDL is Always (the default);
  `public` expanded.
- **Do:** run each line on its own (⌘↵):

  ```sql
  CREATE TABLE demo_m4(a int);
  BEGIN;
  CREATE TABLE demo_tx(a int);
  COMMIT;
  BEGIN;
  CREATE TABLE demo_rb(a int);
  ROLLBACK;
  ```

- **Expected:** `demo_m4` appears under `public` without pressing Refresh. After
  `CREATE TABLE demo_tx` inside the transaction it does **not** appear yet; the namespace carries a
  stamp and the console's output says the change waits for COMMIT. After `COMMIT` it appears.
  `demo_rb` never appears.
- **Pre-verified:** T21 `plan_refresh` (35/35), `docker_refresh_after_ddl`; T27.

### Step 6: rename keeps the node expanded

- **Precondition:** `public` › `demo_m4` expanded so its COLUMNS show.
- **Do:** in the console: `ALTER TABLE demo_m4 RENAME TO demo_m4_renamed;`
- **Expected:** the node changes its name in place and stays expanded, with its columns; the
  cursor stays on it.
- **Pre-verified:** T10 (an OID match is a rename, RR-1), T14 `take_renames`.

### Step 7a: a revoked privilege never deletes silently (adjusted, see `m4.md` §11 note 1)

The plan's step 7 expected the mass-deletion question after `REVOKE USAGE`. On PostgreSQL that
does not happen: its catalogs ignore schema USAGE, so the tables stay listed (R-W5-2; G4:
`revoke-usage existence_tables=24 outcome=applied_no_drops` on pg17). This step checks the safety
property; step 7b shows the question.

- **Precondition:** `pg17-demo` connected, `demo_revoke` expanded, `r1` visible.
- **Do:** in `psql` as `hv_test`: `REVOKE USAGE ON SCHEMA demo_revoke FROM hv_demo;` Then in
  Hacivat select `demo_revoke` and press F5 (Refresh).
- **Expected:** no silent deletion: `r1` stays listed. Either nothing changes, or the
  mass-deletion question appears (also acceptable), or the read fails with a permission notice and
  keeps the cache. **FAIL** only if `r1` disappears without a question.
- **Afterwards:** `GRANT USAGE ON SCHEMA demo_revoke TO hv_demo;`

### Step 7b: the mass-deletion question, Keep cached

- **Precondition:** `pg17-dev` connected, `demo_guard` expanded with its 24 tables.
- **Do:** in `psql`:

  ```sql
  DO $$ BEGIN
    FOR i IN 1..20 LOOP
      EXECUTE format('DROP TABLE demo_guard.g%s', i);
    END LOOP;
  END $$;
  ```

  Then in Hacivat select `demo_guard` and press F5.
- **Expected:** a wide dialog asks before removing 20 objects; "Keep cached" is the default button,
  "Remove from cache" an outline button. Choose **Keep cached**: the 24 rows stay and the namespace
  is marked stale (ink badge, never red). A background check later removes nothing; the next F5
  asks again. (If a background check runs between the drop and F5, it removes nothing and marks
  the namespace stale first; that is also correct.) The dialog lists no names unless T33b added
  them.
- **Pre-verified:** T10 `twenty_objects_eleven_vanishing_is_guarded`; T22 (R-W4-9); T28
  `mass-drop-guard outcome=pending_confirmation vanished=20`.

### Step 8: MySQL 8.4 trigger, server definitions

- **Precondition:** `mysql84` connected; `hv_test` › `demo_trg` expanded.
- **Do:** in the MySQL client (0.4):

  ```sql
  CREATE TRIGGER demo_trg_bi BEFORE INSERT ON demo_trg FOR EACH ROW SET NEW.n = 1;
  ```

  In Hacivat select `hv_test` and press F5. Then open View DDL (F12) on `demo_trg` and turn on
  "Definitions provided by server"; then F12 on the trigger.
- **Expected:** `demo_trg_bi` appears under `demo_trg`'s TRIGGERS. With the toggle on, the table's
  text is the server's `SHOW CREATE TABLE` output (with `ENGINE=InnoDB` and the charset) and the
  caption says "Text from the server"; off, it is the generated text. The trigger shows the
  server's `CREATE … TRIGGER` text with its definer.
- **If** `CREATE TRIGGER` fails with error 1419, `mysql84` still runs without
  `--log-bin-trust-function-creators`: re-run `cargo xtask docker up --set full` (0.2).
- **Pre-verified:** T17 `docker_mysql_intro` (28/28 on CI); R-M4-37 (`SHOW CREATE TRIGGER`
  prepares on every server, `t17-docker-facts-44649745390.txt`).

### Step 9: the Built-in group on PG 17, offline

- **Precondition:** `pg17-dev` has connected at least once (so its server version is known);
  then disconnect it, or `docker stop m2a-pg17`.
- **Do:** expand the Built-in group (last, collapsed) › `pg_catalog` › views ›
  `pg_stat_activity` › COLUMNS.
- **Expected:** the Built-in row carries the badge "version 17", tooltip "Built-in catalog
  (version 17): the server's own objects, shipped with Hacivat and read without a connection.";
  `pg_stat_activity`'s columns (`datid`, `datname`, `pid`, …) are listed with no connection.
- **Pre-verified:** `offline_explorer_shows_the_built_in_group_with_its_version_badge`,
  `offline_lookup_resolves_built_ins_without_a_connection` (`offline_explorer.rs`);
  `offline_resolution_matrix`.
- Afterwards: `docker start m2a-pg17`.

### Step 10: Settings › Metadata

- **Precondition:** a console on `pg17-dev` is active.
- **Do:** open Settings (⌘,) › Metadata. Set "Refresh the explorer after DDL" to **Ask**;
  reconnect `pg17-dev` (settings apply from the next connect, R-W5-11); run
  `CREATE TABLE demo_ask(a int);`; then set it back to Always.
- **Expected:** the page shows "Refresh the explorer after DDL" (Always / Ask / Never), "Check for
  changes in the background", the two cache sizes (512 MB per connection, 4 GB in all), and
  "Clear cached metadata…" for the active console's connection with its size in the note (the
  question opens on Cancel). With Ask, the toast offers Refresh / Not now / Always and counts one
  object.
- **Pre-verified:** `metadata_settings` (9/0 at `d5af420`); shot `m4-settings-metadata`.

### Step 10b (new): Settings › General › Time zone

- **Do:** Settings › General › Time zone. Pick **UTC**; open a new connection to `pg17-dev` and run
  `SELECT now();`. Then set it back to System.
- **Expected:** the 240 px select after Regional format offers "System (<the Mac's zone>)", UTC and
  the zone list; with UTC chosen, a new connection's timestamps show in UTC; open connections keep
  theirs (the row's note says so). A hand-edited unknown zone in the settings file shows as
  "<name> (unknown)", the note says the system zone is used, and no session is sent it.
- **Pre-verified:** `the_time_zone_row_offers_system_utc_and_the_list_and_saves_a_choice`,
  `an_unknown_time_zone_fails_closed_with_a_notice`, `an_unknown_time_zone_is_never_sent_to_a_session`;
  shot `m4-settings-time-zone`.

### Step 10c (new): Clear query history

- **Precondition:** `pg17-dev` has some history (steps 4–6). For the damaged-backups case, the
  build must contain T33b and Hacivat's data folder must hold a `history.sqlite.corrupt-*` file
  (one is left only when a damaged history file was set aside; skip that half otherwise).
- **Do:** right-click `pg17-dev` › "Clear query history…"; press Enter (cancels); open it again and
  press "Clear history". Then open the history dialog, ⋯ › "Clear history of this connection…".
- **Expected:** "Clear the query history of pg17-dev?"; the body says pinned statements go too and
  nothing on the server changes; Cancel is the default (Enter and Escape cancel); after "Clear
  history" a toast confirms it, an open history dialog searches again, and the console's recall
  no longer steps through the cleared statements. Other connections' history is untouched. With
  damaged backups present, the dialog says so and offers "Also delete damaged history backups",
  off by default.
- **Pre-verified:** `the_history_menu_clears_this_connection_after_asking_with_cancel_first`,
  `the_connection_menu_clears_the_selected_connections_history`,
  `cleared_text_is_gone_from_the_file_bytes`; shots `m4-conn-menu-clear-history`,
  `m4-clear-history-question`.

### Step 11: the scope editor, a pattern error in Turkish

- **Precondition:** UI in Turkish.
- **Do:** edit `pg17-dev` (connection dialog) › the Schemas section › Pattern; type `müş{`. Then
  replace it with `@:public|analytics:`. Then restore the original pattern. Finally tick
  "Encrypt cached metadata" and answer Cancel.
- **Expected:** for `müş{` the error line names character 4 (EN: "The pattern has an error at
  character 4: a brace isn't closed."); for the second pattern it names character 20. The field's
  border turns text-2, never red; Save waits until the pattern parses. Ticking the encryption box
  asks in place first, because it deletes the cache; Cancel leaves it off.
- **Pre-verified:** `syntax_error_points_at_the_column_in_scalars` and the Turkish error test in
  `scope_editor.rs`.

### Step 12: EN ↔ TR on every new surface

- **Do:** switch Settings › General › Language between English and Türkçe (or restart with
  `--lang tr`) while the tree is expanded, and look at: the tree's group headers (TABLES /
  TABLOLAR), the node menus, the DDL view toolbar and options, the mass-deletion dialog, the Ask
  toast, Settings › Metadata, the Time zone row, the Clear history dialog, the scope editor, the
  Loading and failure rows.
- **Expected:** every new string follows the language; catalog names stay exactly as stored; the
  expansion and the cursor survive the switch; "transaction", "commit" and "rollback" stay in
  English in Turkish text.
- **Pre-verified:** `cross_surface_m4.rs` (d); the shots in en, tr and the pseudo-locales.

## 2. Measurements on the PO's Mac (Task 33 Step 3)

Close other builds and agents; keep the bench window frontmost; keep the display awake
(`caffeinate -dis <command>`). Record the machine and the build hash (`git log -1 --oneline`).

### 2.1 G1, authoritative (E1)

From `m4-bench-notes.md` §3, on the gate's `m4` commit:

```
HACIVAT_G1_FIXTURE=$PWD/target/bench/g1-160k.v1.sqlite \
  cargo test --release -p hacivat-meta --test bench_fixture -- --ignored write_g1_fixture
HACIVAT_BENCH_HOST=po-mac cargo run --release -p hacivat-app --features bench --example catalog_cold -- \
  --fixture target/bench/g1-160k.v1.sqlite --runs 10 --offline
cargo xtask bench-report target/bench/g1-catalog-cold-*.json --milestone m4
```

- The last bench line is `RESULT: PASS|FAIL|INVALID g1-catalog-cold`; the encrypted line comes
  just before it (`RESULT: n/a g1-catalog-cold-encrypted`). INVALID means the window was not key
  at t1 or the fixture was too small; re-run.
- The bench opens and closes 20 windows (10 per variant).
- Encrypted with the real Keychain delay: step 1b.
- Fixture (b), the real `hv_m4_big` cache from 0.3:

  ```
  cargo run --release -p hacivat-app --features bench --example catalog_cold -- \
    --fixture target/docker/g1-hv_m4_big.v1.sqlite --runs 10 --offline
  ```

### 2.2 The 1-second workspace check (E12, §3.3)

- Read `process_to_t1_ms` (p50 and p95) from the 2.1 run; the budget is ≤ 1 s from process start
  to an interactive workspace with a cached catalog. The clock starts at the example's `main`, so
  exec and the loader (a few to some tens of ms) are not in it. T29's run under load gave p50
  about 0.87 s and p95 1,047 ms (not graded).
- By hand: quit Hacivat with `pg17-dev` expanded and cached, start
  `cargo run --release -p hacivat -- --lang en`, and check that the tree is shown and a console
  accepts typing within a second.

### 2.3 `--smoke` ×3 (E12)

```
cargo run --release -p hacivat -- --smoke
```

Three times, with no connection, after the heavy runs. Budgets: first frame ≤ 500 ms, idle RSS
≤ 200 MB.

## 3. After the demo (controller; C-9)

From the main checkout:

```
docker exec m2a-pg17 du -sh /var/lib/postgresql/data
HACIVAT_DOCKER=1 HACIVAT_DOCKER_SET=pr HACIVAT_DOCKER_ARTIFACTS=$PWD/target/docker HACIVAT_M4_CLEANUP=1 \
  cargo test -p hacivat-introspect --test docker_pg_intro -- --ignored --exact drop_big_fixture_when_asked
docker exec m2a-pg17 du -sh /var/lib/postgresql/data
```

- These are the variables `cargo xtask docker test` sets, plus the cleanup switch, for this one
  test only. Running the whole `docker_pg_intro` target through the wrapper would also run
  `big_catalog_cold_and_incremental_timings`, which creates `hv_m4_big` at the same time.
- Record both sizes in `m4.md` section 8 (C-9).
- `docker compose -p hacivat-m2a down -v` for the PG services **only with the PO's OK** (Q20,
  still open). The default is to keep the containers' data.
- The demo objects (`demo_tr`, `demo_guard`, `demo_revoke`, `demo_m4_renamed`, `demo_tx`,
  `demo_ask`, the role `hv_demo`, MySQL `demo_trg`) are test data on test servers; drop them if
  the PO wants a clean stack.

## 4. Results

### 4.1 Demo steps

| # | Step | Expected (short) | Pre-run | PO result |
|---|---|---|---|---|
| 1 | Offline start, cached dev profile expanded | Tree well under 1 s; "Showing cached metadata from …"; nothing connects | | |
| 1b | Offline start, encrypted production profile | One Keychain read; no freeze; tree after the key; collapsed encrypted profiles untouched | | |
| 2 | Connect | Stale marks clear; incremental check, no full re-read | | |
| 3 | `hv_m4_big` + TR filter | 15,000 tables, smooth scroll; `istanbul` finds `İstanbul_şube` | | |
| 4 | View DDL, Turkish name | Correct quoting; qualification toggles; Copy equals the text | | |
| 5 | DDL refresh, transaction | `demo_m4` at once; `demo_tx` after COMMIT; `demo_rb` never | | |
| 6 | Rename | Node stays expanded, renamed in place | | |
| 7a | REVOKE USAGE | No silent deletion | | |
| 7b | Drop 20 of 24 | The question; Keep cached keeps the rows, stale badge | | |
| 8 | MySQL trigger | Trigger after F5; server `SHOW CREATE` text with the toggle | | |
| 9 | Built-in offline | "version 17" badge; `pg_stat_activity` columns offline | | |
| 10 | Settings › Metadata | The rows; Ask toast counts one object | | |
| 10b | Time zone | UTC applies to new connections; unknown zone fails closed | | |
| 10c | Clear query history | Cancel default; cleared; damaged-backups option (T33b) | | |
| 11 | Scope editor, TR error | Error names the character; Save waits; encrypt asks first | | |
| 12 | EN ↔ TR | Every new surface follows; names verbatim; state kept | | |

### 4.2 Measurements

| Measurement | Value |
|---|---|
| Machine and build hash | |
| G1 plaintext p95 (`RESULT:` line), fixture (a) | |
| G1 encrypted p95, memory store (`--key-delay-ms 0`) | |
| Keychain `CatalogKey` read, first after restart (with prompt?) | |
| Keychain `CatalogKey` read, later | |
| G1 encrypted p95 with `--key-delay-ms <measured>` | |
| Decision if encrypted p95 > 300 ms | |
| G1 fixture (b) `hv_m4_big` p95 | |
| `HV-FACT g1` cold Structure / Names on `hv_m4_big` (0.3) | |
| `process_to_t1_ms` p50 / p95 (≤ 1 s) | |
| 1-second check by hand | |
| `--smoke` run 1: first frame / idle RSS | |
| `--smoke` run 2: first frame / idle RSS | |
| `--smoke` run 3: first frame / idle RSS | |
| `pg17` data size before / after C-9 | |
