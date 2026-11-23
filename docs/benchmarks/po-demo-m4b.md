# M4b PO demo: "Script workspace (local)" checklist

This is the part of the M4b gate (plan Task 22) that only the PO can do. It has two parts:

- the hand demo of plan Task 22 Step 4, steps 1–12, on the PO's Mac, with a real window and
  keyboard, real VS Code, Finder and Terminal, and the Docker stack up;
- the E12 measurements of Step 3 that need the real app or a quiet machine.

Each step gives:

- its precondition;
- what to do (exact text, commands and keys);
- the expected screen (app text in `EN`);
- the automated tests that already cover it;
- the gate agent's pre-run.

Write each result in the tables of section 4.

- Gate record: `docs/benchmarks/m4b.md`. Its section 0 orders everything the PO does.
- **Pre-run by the gate agent (Task 22, at `c8812c7d`).** The agent had no Docker, no VS Code
  session and no hands on the window. "Pre-run: tests" means the step's behaviour was re-run
  through the named automated tests on this commit, all green (`m4b.md` section 3a), except where
  a step says otherwise. "PO" means
  only the PO can check it.
- **Terms:** "transaction", "commit" and "rollback" stay in English in the Turkish UI (PO ruling).
  Shortcuts follow VS Code's defaults and can be changed in Settings.
- **Adjusted from the plan's list:**
  - Struck by R-C0-12: "Save as… into a library" and "Move console to library…". No M4b task built
    them, and they moved to M5 or later. Step 11 audits the surfaces M4b has: attach, map, share,
    save, recover and run.
  - Step 7: `git add` changes no badge. By R-M4b-11, a file added to the index is still **N**
    (new) or **M** (modified), so the step uses `git commit` and a new edit instead.
  - Step 9 adds the later rulings:
    - R-T18-1: a pasted preview inherits the tab's binding;
    - R-T18-2: the Stop wording;
    - R-T22-1 and R-T22-2: Stop and the notices stay reachable in a small tile;
    - the Roll back wording, which is a PO item.
  - Step 2 adds R-T21-1: the connection name in a script tab's title is never cut.

## 0. Preparation

### 0.1 Build

Use the main checkout on `m4b` at the gate tip (`c8812c7d`, or the tip that holds this file).

```
git log -1 --oneline
cargo build --release -p hacivat
```

### 0.2 Docker stack

The local `hacivat-m2a` stack has been down since M4's C-0, and the M4b Docker proofs ran on CI
(R-W2-14, R-M4b-36). For the demo the PO starts the two PostgreSQL servers it needs. Run this from
the **main checkout**:

```
docker compose -p hacivat-m2a start pg17 pg16
docker compose -p hacivat-m2a ps pg17 pg16
```

- Good: `m2a-pg17` and `m2a-pg16` are `healthy`.
- If the containers do not exist, use `cargo xtask docker up --set full` instead. It waits up to
  180 s.
- **Never run `cargo xtask docker down`** for the demo, because it removes the volumes. To pause:
  `docker compose -p hacivat-m2a stop`.
- Test logins (throwaway, `docker/m2a/README.md`): user `hv_test`, password `hv_test_pw`.
- Ports on `127.0.0.1`: `pg17` 54317, `pg16` 54316.
- Both servers create the database `orders` in `docker/m2a/init/pg.sql`. If a server's volume is
  older than that script, create it:

```
docker exec m2a-pg17 psql -U hv_test -d hv_test -c 'CREATE DATABASE orders'
docker exec m2a-pg16 psql -U hv_test -d hv_test -c 'CREATE DATABASE orders'
```

- An "already exists" error is fine.
- For step 9, create a demo table on staging:

```
docker exec m2a-pg16 psql -U hv_test -d orders -c 'CREATE TABLE IF NOT EXISTS demo_dump (id int, note text)'
docker exec m2a-pg17 psql -U hv_test -d orders -c 'CREATE TABLE IF NOT EXISTS demo_rb (id int)'
```

### 0.3 The library folder `orders-scripts/` (a git repository)

Make it anywhere outside iCloud Drive, for example `~/Documents/orders-scripts`:

```
mkdir -p ~/Documents/orders-scripts/@production ~/Documents/orders-scripts/dumps
cd ~/Documents/orders-scripts
printf 'SELECT version();\n' > report.sql
printf 'DELETE FROM demo_rb;\n' > @production/cleanup.sql
printf 'SELECT 1 AS müşteri;\n' > müşteri_raporu.sql
printf 'dumps/\n' > .gitignore
git init -q && git add -A && git commit -q -m 'demo library'
python3 - <<'EOF'
# A 150 MB dump for step 9 (INSERTs into demo_dump on staging). It is git-ignored, so it is
# still listed (R-M4b-13) and gets no badge.
row = "INSERT INTO demo_dump VALUES ({}, '" + "x" * 120 + "');\n"
with open("dumps/big.sql", "w") as f:
    i = 0
    while f.tell() < 150 * 1024 * 1024:
        f.write(row.format(i)); i += 1
EOF
ls -la dumps/big.sql
```

- There is no `.hacivat/` folder yet. Step 1 writes it.
- Delete the folder after the demo (section 3).

### 0.4 App and profiles

```
cargo run --release -p hacivat -- --lang en
```

Create these profiles (⌘N, "Save and connect"):

| Profile | Engine | Port | User / password | Database | Environment |
|---|---|---|---|---|---|
| `orders-prod` | PostgreSQL | 54317 (pg17) | `hv_test` / `hv_test_pw` | `orders` | `production` |
| `orders-staging` | PostgreSQL | 54316 (pg16) | `hv_test` / `hv_test_pw` | `orders` | `staging` |

## 1. The demo (plan Task 22 Step 4)

### Step 1: attach the folder

- **Do:**
  1. Open Settings › Script libraries and click "Attach a folder…".
  2. Choose `orders-scripts`.
  3. Check that the Database field reads `orders` (the folder name was `orders-scripts`, so type
     `orders` if needed).
  4. Click Attach.
- **Expected:**
  - The dialog says "Hacivat creates this file in the folder:" and shows the exact text of
    `.hacivat/workspace.toml`: `format = 1`, `database = "orders"`, `aliases = []`.
  - Under it: "It holds only the database name and its aliases: never a connection, a host or a
    password."
  - After Attach, the library is listed with its file count and "git repository".
  - Nothing is written over an existing file. If you attach a second time, the dialog says
    "Found .hacivat/workspace.toml — binds to orders" and changes nothing.
- **Pre-run: tests.**
  - `attach_shows_the_exact_file_it_will_write`, `attach_over_existing_manifest_does_not_rewrite_it` and the other `scripts_attach` tests (T16);
  - `manifest_type_has_only_three_fields` (T3).

### Step 2: the same library under prod and staging, each runs on its own server

- **Do:**
  1. Connect `orders-prod` and `orders-staging`, and expand each root once, so their catalogs list
     `orders` (R-M4b-31).
  2. Explorer: open `orders-prod` › orders › **Scripts**, then `orders-staging` › orders ›
     **Scripts**.
  3. Open `report.sql` under each, and Run (⌘↩).
- **Expected:**
  - The Scripts node shows the outlined "shared" badge and a file count. Its tooltip names the
    other connection.
  - Two tabs, `report.sql · orders-prod` and `report.sql · orders-staging`.
  - Each Run shows its own server: PostgreSQL 17.x under prod and 16.x under staging.
  - The capsule reads PRODUCTION on the prod tab and STAGING on the staging tab. Switching tabs
    switches it.
  - The tab's connection cannot be changed. Its menu offers "Open under another connection…",
    which opens a second tab.
  - R-T21-1: narrow the window until the tabs shrink. The file part of the title shortens first,
    and the connection name is never cut while the active tab fits.
- **Pre-run: tests.**
  - `same_library_under_prod_and_staging`, `rows_under_two_connections_have_different_keys`
    (T13, `scripts_projection`);
  - `tab_runs_on_the_connection_it_was_opened_under` and `two_tabs_same_file_two_connections`
    (T14, `scripts_tabs`).
  - Docker, CI only: `same_library_runs_on_each_connection` (T19, pg17/pg16, mysql84/mysql80,
    sqlite). The facts are in `m4b.md` E1.

### Step 3: `@production/cleanup.sql` is hidden on staging

- **Do:**
  1. Expand both Scripts nodes.
  2. Press ⇧⌘O (Go to script) and type `cleanup`.
- **Expected:**
  - `@production/cleanup.sql` is listed under `orders-prod` only.
  - Go to script finds it and offers only `orders-prod`. Because that is production, it asks
    before opening, and "Production is never chosen for you": nothing is preselected.
- **Pre-run: tests.**
  - `envscope_table` (T4, 18 rows), `production_file_hidden_on_staging` (T13);
  - `production_scoped_file_offers_only_production` and `chooser_never_preselects_production`
    (T15).

### Step 4: an edit in VS Code appears without a reload

- **Precondition:** `report.sql · orders-staging` is open and clean.
- **Do:**
  1. `code ~/Documents/orders-scripts/report.sql`. Change the text and save (⌘S in VS Code).
  2. In Hacivat, type in the same tab without saving. Change the file again in VS Code and save.
  3. Click "Keep mine", then ⌘S in Hacivat.
- **Expected:**
  1. Within a second the Hacivat tab shows VS Code's text, with no prompt. The caret stays on its
     line.
  2. The bar "report.sql changed on disk." appears with Reload / Keep mine. Your typed text stays.
  3. ⌘S asks "The file changed on disk since you opened it. Overwrite?" with Overwrite (danger) and
     Cancel.
  - The `orders-prod` tab of the same file also reloads (if clean) or shows the bar (if dirty).
- **Pre-run.**
  - Tests: `watch_real` (T9, real FSEvents), `external_edit_appears_without_reload`,
    `dirty_tab_is_never_reloaded`, `save_over_external_change_asks` (T14);
    `reload_is_held_during_a_composition_and_applied_after` (T11).
  - Measured: an in-place write → `FileChanged` on the open file in 321–371 ms (p50 339) over 20 writes on
    this Mac (`m4b.md` E12).
  - **PO:** real VS Code.

### Step 5: Finder rename and delete

- **Do:**
  1. In Finder, rename `müşteri_raporu.sql` (open in a tab) to `musteri_raporu_2.sql`.
  2. Then move it to the Trash.
- **Expected:**
  1. "müşteri_raporu.sql was renamed to musteri_raporu_2.sql; this tab follows it". The tab title
     follows.
  2. "musteri_raporu_2.sql was deleted on disk." with "The text stays in this tab. Save writes the
     file again."
- **Pre-run.**
  - Tests: `watch_real` renames and removals, T10's rename pairing in `service`, and the T14
    disk-bar tests.
  - **PO:** real Finder.
- Rename it back afterwards: step 8 uses this file.

### Step 6: the secret warning

- **Do:**
  1. In `report.sql · orders-staging`, type `CREATE USER app PASSWORD 'hunter2';` on line 1.
  2. Press ⌘S, then Cancel.
  3. Add ` -- hacivat:allow-secret` at the end of the line (a space after the dashes) and press
     ⌘S.
- **Expected:**
  - A key glyph shows in the gutter of line 1. Its tooltip names the kind, never the value.
  - ⌘S opens "1 line in report.sql looks like a secret", which lists "Line 1" and never
    `hunter2`, with Save anyway / Cancel (Cancel is the default).
  - Cancel saves nothing.
  - With the allow comment the save goes through without a question.
- **Pre-run: tests.**
  - `secret_table` (T6), `allow_comment_silences_one_line` (T6);
  - `secret_literal_triggers_the_warning_on_save`, `warning_never_echoes_the_value` (T17).
- Undo the line afterwards. Do not commit it.

### Step 7: git badges equal `git status`

- **Do:**
  1. `cd ~/Documents/orders-scripts && git status --short`. Compare with the explorer badges.
  2. `printf 'x\n' > new.sql`, then edit `report.sql` in Hacivat and save.
  3. `git add -A && git commit -q -m wip`.
- **Expected:**
  1. Identical: **M** for a modified file and **N** for an untracked one. Clean and ignored files
     have no badge (`dumps/big.sql` has none). Folders with changes show a dot.
  2. Within about a second, `new.sql` shows **N** and `report.sql` shows **M**.
  3. Both badges clear. `git add` alone changes no badge, because R-M4b-11 counts both "added to
     the index" and "untracked" as N.
- **Pre-run: tests.**
  - `status_matches_git_cli` (T7, 25 fixtures equal `git status`), `hostile_repo_runs_no_program`;
  - `git_badges_match_git_status_on_fixture` (T13).

### Step 8: Go to script in Turkish

- **Do:**
  1. Quit, then start `cargo run --release -p hacivat -- --lang tr`.
  2. Press ⇧⌘O, type `musteri`, then Enter.
- **Expected:**
  - `müşteri_raporu.sql` is found.
  - The chooser lists `orders-staging` and `orders-prod`. **Staging is preselected**, and
    production never is.
- **Pre-run: tests.**
  - `index_is_turkish_tolerant_for_tr` (T10);
  - `chooser_never_preselects_production` and the `scripts_goto` set (T15, 28 tests).

### Step 9: a 150 MB dump, Run as script, Stop

- **Do:**
  1. Open `dumps/big.sql` under `orders-staging`.
  2. Choose Run as script.
  3. After a few seconds, press Stop.
- **Expected:**
  - The banner "Read-only preview: this file is 150 MB." shows, followed by "The editor shows its
    first 2.5 MB. Run as script sends the whole file."
  - Run as script first reads the whole file (pass 1, nothing runs), then streams it. The strip
    shows "Running as script", the statements sent, the line, and "N of 150 MB".
  - During the run the toolbar shows Stop.
  - Stop ends the run with either text:
    - "Stopped during statement N; statements after it were not sent." (R-T18-2: the statement in
      flight was already sent);
    - "Stopped after N statements. Nothing after them was sent."
  - Memory stays flat. Activity Monitor shows no growth near 150 MB.
- **Also check:**
  - R-T22-1 and R-T22-2: during the run, drag the results split up to its smallest size. Stop
    stays visible in the toolbar. Tab onto a notice button: it scrolls into view.
  - R-T18-1: copy a 25 MB slice (`head -c 26000000 dumps/big.sql | pbcopy`) and paste it into
    `report.sql · orders-staging`. The question offers only Cancel / Paste as preview. The
    preview tab is bound to `orders-staging` and runs only there.
- **Roll back wording (PO item):**
  1. In `orders-prod`, create a library file `rb.sql` with
     `INSERT INTO demo_rb VALUES (1); INSERT INTO demo_rb VALUES (2);`.
  2. Run as script. Confirm the first statement, then choose Roll back in its "rows affected"
     prompt (the streamed unit's commit prompt, T18). The gate agent could not drive this prompt,
     so note what it actually offers.
  - Today the strip says "Stopped at statement 1: the server returned an error. Nothing after it
    was sent."
  - The proposed text is in `po-bekleyen.md` item 20: "Stopped at statement N: you rolled it back.
    The statements before it ran and stay applied; nothing after it was sent."
  - Decide which one to keep.
- **Pre-run: tests.**
  - `run_as_script_stops_when_file_changes_mid_run` (T12, exec);
  - `mysql_dump_of_150_mb_runs_in_constant_memory`: 13 MiB growth when run alone. It fails beside
    its sibling perf test (a test-isolation defect, `m4b.md` E8);
  - the `scripts_run_as_script` set (T18, 17 tests), `cross_surface_m4b`, `the_landing_never_edits_or_scans_the_change`.
  - Docker, CI only: the 150 MiB dump on mysql84 (`m4b.md` E8).

### Step 10: quit with an unsaved script; force-quit and recover

- **Do:**
  1. Type in `report.sql · orders-staging` without saving, then ⌘Q.
  2. Cancel. Type again, wait **6 seconds** (recovery copies are written 5 s after the last edit),
     and force-quit: `pkill -9 -x hacivat`, or Activity Monitor › Force Quit.
  3. Start the app again.
- **Expected:**
  1. "Save changes to report.sql?" with Save / Don't save / Cancel. Cancel is focused, and
     Escape answers Cancel.
  2. After the relaunch, "Recovered unsaved changes to 1 script". The tab is open with your text
     and "Recovered unsaved changes from <date>." with Save / Discard….
  - The recovery copy is under Hacivat's data folder, never in `orders-scripts`.
- **Pre-run: tests.**
  - `recovery_lives_outside_the_library_and_is_private` (T5);
  - the T14 quit-guard and launch-recovery tests in `scripts_tabs`.

### Step 11: nothing about the connection in the library

- **Do:**

```
ls -la ~/Documents/orders-scripts/.hacivat
cat ~/Documents/orders-scripts/.hacivat/workspace.toml
grep -rIn -e hv_test -e 127.0.0.1 -e localhost -e 54316 -e 54317 -e orders-prod -e orders-staging \
  ~/Documents/orders-scripts --exclude-dir=.git
```

- **Expected:**
  - `.hacivat/` holds only `workspace.toml`, with the keys `format`, `database` and `aliases`.
  - `grep` prints nothing. The files you typed in step 6 do not count: they hold no connection
    data.
- **Pre-run: tests.**
  - `no_credentials_are_ever_written_into_a_library_folder` (T19, app);
  - `mapping_lifecycle_writes_no_profile_id_near_the_library`;
  - Docker, CI only: `no_credentials_reach_the_library_on_real_servers`, with `needles=25 found=0`
    (`m4b.md` E6).

### Step 12: EN ↔ TR on every new surface

- **Do:** switch Settings › General › Language between English and Türkçe. Visit each surface:
  - Settings › Script libraries;
  - the attach and map dialogs;
  - the Scripts tree and its menu;
  - a script tab, its title and tooltip;
  - the disk bars and Overwrite?;
  - Go to script and the chooser;
  - the secret warning;
  - the preview banner and Run as script strip;
  - the unsaved prompt;
  - the recovery notice.
- **Expected:**
  - Every new string follows the language.
  - File names stay exactly as on disk.
  - The tabs and their bindings survive the switch.
- **Pre-run: tests.**
  - `l10n check` 0 errors;
  - the M4b shots in en, tr and the pseudo-locales (656 new PNGs, `m4b.md` E10);
  - `cross_surface_m4b`.

## 2. Measurements on the PO's Mac (Task 22 Step 3, the "PO manual" rows of E12)

Close other builds and agents, keep the display awake (`caffeinate -dis …`), and record the build
hash (`git log -1 --oneline`). The gate agent measured the headless budgets on this Mac under
load (`m4b.md` E12). These steps are the ones that need the real app or a quiet machine.

### 2.1 `--smoke` ×3, no library (E12; spec §3.3)

```
cargo build --release -p hacivat
for i in 1 2 3; do ./target/release/hacivat --smoke; done
```

- Budgets: `first-frame-ms` ≤ 500 and `idle-rss-mib` ≤ 200.

### 2.2 First frame and idle RSS with 3 libraries attached

1. Attach three libraries: `orders-scripts`, plus two more folders of `.sql` files. One can be a
   generated 10,000-file library (delete it afterwards):

   ```
   python3 -c "import os
   for i in range(10000):
       d=os.path.expanduser('~/Documents/lib-10k/d%03d' % (i//100)); os.makedirs(d, exist_ok=True)
       open(d+'/f%05d.sql' % i, 'w').write('SELECT %d;\\n' % i)"
   ```

2. Quit.
3. Run `--smoke` three times as in 2.1.

- The library service starts at launch when `libraries.toml` exists, so `--smoke` includes it.
- Budgets: first frame ≤ 500 ms; idle RSS ≤ 200 MB.

### 2.3 Idle CPU of the watcher, one sample per minute for 10 minutes

- With the three libraries attached and the app idle (no tab typing, window in the background),
  run in Terminal:

```
pid=$(pgrep -x hacivat)
cpu() { LC_ALL=C ps -o time= -p $pid | awk -F: '{ s=0; for (i=1;i<=NF;i++) s=s*60+$i; printf "%d", s*1000 }'; }
prev=$(cpu)
for m in $(seq 1 10); do sleep 60; now=$(cpu); echo "minute $m cpu_ms=$((now - prev))"; prev=$now; done
```

- Budget: ≈ 0. Compare with the same loop and no library attached.
- The gate agent measured 130–240 ms per minute with three libraries, against 10–20 ms with none
  (`m4b.md` E12: FAIL, a debouncer tick per library). Re-run this after the fix, or rule on it.

### 2.4 The headless budgets, on a quiet machine (optional re-run)

The gate agent's numbers came from a loaded machine. To re-run them quietly, from the checkout:

```
HACIVAT_PERF=1 cargo test --release -p hacivat-scripts --test scan --test index --test secrets -- --ignored --nocapture
HACIVAT_PERF=1 cargo test --release -p hacivat-git --test status_differential -- --ignored --nocapture
HACIVAT_PERF=1 cargo test --release -p hacivat-ui-editor --lib reload_ui_share -- --ignored --nocapture
```

- Read the `HV-FACT` lines.
- Budgets: scan of 10,000 files ≤ 500 ms; git status of 10,000 files ≤ 1 s; Go to script
  ≤ 5 ms; reload UI share ≤ 1 ms on 2 MB.

## 3. After the demo

- `rm -rf ~/Documents/orders-scripts`, after Settings › Script libraries › Remove….
- `docker compose -p hacivat-m2a stop pg17 pg16`, if you want the stack down again.
- The demo tables `demo_dump` and `demo_rb` are test data on test servers. Drop them if you want a
  clean stack.

## 4. Results

### 4.1 Demo steps

| # | Step | Expected (short) | Pre-run | PO result |
|---|---|---|---|---|
| 1 | Attach | Exact bytes shown; three keys; never rewritten | tests | |
| 2 | Prod and staging | Shared badge; pg17 vs pg16 version; PRODUCTION / STAGING capsule; full connection name in the title | tests; Docker on CI | |
| 3 | `@production/` | Prod only; Go to script asks, nothing preselected | tests | |
| 4 | VS Code | Reload within 1 s; bar when dirty; Overwrite? | tests; latency measured | PO (VS Code) |
| 5 | Finder | Tab follows the rename; "deleted on disk" | tests | PO (Finder) |
| 6 | Secret | Gutter glyph; "Line 1", no value; Cancel default; allow comment saves | tests | |
| 7 | Git | Badges = `git status`; commit clears them | tests | |
| 8 | TR Go to script | `musteri` finds `müşteri_raporu.sql`; staging preselected | tests | |
| 9 | 150 MB | Banner; two passes; Stop wording; flat memory; Stop reachable in a small tile; Roll back wording decided | tests; Docker on CI | PO (Docker up) |
| 10 | Quit / recover | Save prompt; recovered after force-quit | tests | |
| 11 | Library audit | Only `workspace.toml`; grep empty | tests; Docker on CI | |
| 12 | EN ↔ TR | Every new surface follows | l10n, shots | |

### 4.2 Measurements

| Measurement | Value |
|---|---|
| Machine and build hash | |
| `--smoke` ×3, no library: first frame / idle RSS | |
| `--smoke` ×3, 3 libraries: first frame / idle RSS | |
| Idle CPU, 10 samples (ms per minute) | |
| Headless re-run on a quiet machine (optional): scan / status / Go to script / reload | |
