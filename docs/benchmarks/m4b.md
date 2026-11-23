# M4b gate record

Milestone M4b, "Script workspace (local)", covers:

- script libraries: `libraries.toml`, `.hacivat/workspace.toml` and binding (HV-SCRIPT-01..12);
- the Scripts projection under each connection's database, with environment folders;
- read-only git badges (`gix`, isolated, reduced trust);
- the file watcher and external edits;
- script tabs bound to (profile, database), with save, conflict, recovery and restore;
- Go to script and the connection chooser;
- the secret warning;
- the large-file carries: T2 preview, the 20–100 MB paste question, and two-pass Run as script;
- the adversarial suite.

Status words:

- **PASS** / **FAIL**: decided by the evidence in the row.
- **PENDING (CI)**: needs a job of the gate's CI runs. The controller fills those cells.
- **PENDING (PO)**: needs the PO at the Mac (the demo, a quiet-machine measurement, a sign-off).

Facts:

- **Gate tip `c8812c7d`** (`m4b`: T0–T21, cifix-1..6, T22-pre and R-C0-14/15). This record is
  committed on top of it; it changes no code.
- **SDD** = `.superpowers/sdd/2026-11-17-m4b-script-workspace/`. That folder holds the plan, the
  ledger `progress.md`, the task reports and reviews, `carries-w1.md` and
  `po-questions-m4b.md`. It also holds `shot-superseded-m4b.txt` and the 656 new PNGs in
  `render-m4b-new/`.
- **Machine:** a MacBook Pro (Mac16,7) with an Apple M4 Pro (14 cores: 10 performance, 4
  efficiency), 48 GB of memory, on macOS 26.6.2 (25G83). The git CLI is 2.50.1.
  - This is the PO's Mac. It was **not quiet**: other desktop processes and a virtual machine ran
    throughout, and the 1-minute load average was 5–18 (see each line).
  - No hostnames, accounts or personal names are recorded here.
- **Measured vs. quoted.** Section 3a and the E12 numbers were measured by the gate agent on this
  commit. Everything else is quoted from a task report, a review, the ledger, or a CI job.
- **Docker** never ran locally (R-W2-14, R-M4b-36). Every Docker fact must be quoted from an
  `HV-FACT` line of the gate's CI runs, which are:
  - **45202154953** and **45202181119**: `workflow_dispatch` on `c8812c7d`;
  - **45202171406**: the `pull_request` run of PR #6 (`m4b` → `main`, draft, "gate evidence
    only").
- **CI state at the time of writing.** No job of the three runs had failed. These were green
  and quoted below: docker-full ×2, docker-pr, fuzz-scripts ×2, build-test ubuntu (dispatch 1)
  and build-test windows (PR). Others were still running; those cells read "PENDING (CI)" with
  the run id, and the controller fills them in.

## 0. Open before the gate closes, in order

1. **E9 is FAIL** (section 1). The literal check finds `NotImplemented` outside tests: five
   declared variants that nothing constructs, stale Task 1a module docs, and two live
   `WatchError::NotImplemented` returns that mean "this source was stopped". No stub was found
   unfilled. A small cleanup task is needed, then a re-run of the check:
   - remove the dead variants;
   - rename the watcher's variant, for example to `Stopped`;
   - fix the docs.
2. **E12 Go to script is FAIL on this machine.** `query_over_10k_entries_under_5ms` gave a best
   of five of 4,971–5,587 µs, and failed 8 of 9 runs at load 6–16; lower load did not help. This is the first release run
   of that line (T10 measured only debug). The paths:
   - a speed-up of `ScriptIndex::query`. T10 found the time is mostly in `hacivat-l10n`'s
     Turkish-tolerant subsequence matcher;
   - a re-measure on a quiet machine (`po-demo-m4b.md` §2.4);
   - a ruling. The query runs off the UI thread past 2,000 entries (T15), so typing never waits on
     it.
3. **E12 watcher idle CPU is FAIL.** With three libraries, the real app uses 130–240 ms of CPU
   per minute at idle, against 10–20 ms with none. The cause is `notify-debouncer-full`'s fixed
   37.5 ms tick, one thread per library (E12 table). The options:
   - one shared debouncer for every library;
   - our own debounce timer, armed only while events are pending (the source already re-checks
     with `lstat`, R-T9-3);
   - a larger `tick_rate` passed to `new_debouncer_opt`. Its signature takes one, and today it is
     `None` (`watch/mod.rs:274`), which gives 150 / 4.

   Alternatively, a PO ruling that ~0.25 % of a core is "≈ 0".
4. **The CI cells.** The controller fills them from runs 45202154953, 45202181119 and 45202171406
   (E2, E10, E11). Every job must be green.
5. **PO at the Mac:**
   - `po-demo-m4b.md` §1, steps 1–12, with Docker up (`pg17` and `pg16`), real VS Code, Finder
     and Terminal;
   - §2, the E12 rows marked PO;
   - the PNG and DESIGN.md sign-off (E10);
   - the rulings in sections 5–6;
   - the PO queue (`po-bekleyen.md` item 20), including the Roll back wording.
6. **Merge to `main`:** only on the PO's word. PR #6 stays a draft until then.

## 1. Exit criteria E1 to E12

| # | Criterion | Status | Evidence |
|---|---|---|---|
| E1 | The same library appears under orders-prod and orders-staging; a file opened from each runs on that connection with the right capsule | **PASS** (automated, local, and Docker on the gate runs); demo step 2 **PENDING (PO)** | **Local at `c8812c7d`, all green (§3a):** `scripts_projection` `same_library_under_prod_and_staging` and `rows_under_two_connections_have_different_keys` (T13); `scripts_tabs` `tab_runs_on_the_connection_it_was_opened_under`, `two_tabs_same_file_two_connections` and `restore_keeps_binding_and_reads_current_env` (T14, which also probes the server database before every Run, R-T14-2); `mysql_tab_reconnect_reapplies_its_database_before_run`, `unbound_script_tab_is_never_bound_by_a_connect`. **Docker (CI only):** `hacivat-scripts --test docker_m4b` `same_library_runs_on_each_connection` (pg17/pg16, mysql84/mysql80, sqlite), through the real ScriptRunner, the gate and T14's check (R-T19-1), plus `mysql_schema_is_the_database_level`, `mysql_tab_session_reopened_after_a_kill_is_switched_back` and `mysql_default_session_reconnect_replays_its_schema`. **Gate run 45202154953, docker-full (job 137788196716, green, 202 result blocks, 0 failed)** logs six `HV-FACT t19 e1` lines, among them `HV-FACT t19 e1 profile=orders-prod target=pg17 database=orders server_version_num=170012 env=production asked=2 no_asked=1 units_run=2 marker_prod=1 marker_staging=0 production_file_listed=true` and `HV-FACT t19 e1 profile=orders-staging target=pg16 database=orders server_version_num=160016 env=staging asked=0 no_asked=- units_run=2 marker_prod=1 marker_staging=1 production_file_listed=false`. The same pairs follow for `mysql84` (8.4.13, production) and `mysql80` (8.0.46, staging), and for sqlite (3.54.1). Each run lands only on its own server: the staging run leaves the prod marker at 1. Production asks, and staging asks nothing. Also logged: `HV-FACT t19 mysql_switch target=mysql84 default=orders script_before=orders script=orders_archive`, `HV-FACT t19 mysql_reconnect … after_check=orders_archive new_connection=true` and `HV-FACT t19 mysql_reopen … killed_state=Broken(NetworkReset) … after_check=orders_archive`. Dispatch 2 (45202181119, job 137788282835) logs the same lines. |
| E2 | External edits made in VS Code appear without a reload | **PASS** (automated and measured, local); real VS Code **PENDING (PO)**: demo step 4 | **Local:** `watch_real` (real FSEvents) green; `scripts_tabs` `external_edit_appears_without_reload`, `dirty_tab_is_never_reloaded` and `save_over_external_change_asks`; `reload` (T11) `reload_is_held_during_a_composition_and_applied_after` and `the_landing_never_edits_or_scans_the_change`. **Measured (E12):** an in-place write → `LibraryEvent::FileChanged` on the open file in 321–371 ms (p50 339) over 20 writes, through the real `LibraryService` with OS watch sources on the 10,000-file library. A real VS Code save was never automated (T0 U-08), so the PO's step 4 settles it. On the other OSes, `watch_real` is green on Linux (build-test ubuntu, 45202154953) and Windows (build-test windows, PR 45202171406). build-test macOS is **PENDING (CI)**. |
| E3 | An `@production/` file is hidden on staging | **PASS** (automated); demo step 3 **PENDING (PO)** | `envscope` `envscope_table` (18 rows) and `untagged_profile_sees_only_unscoped`; `scan` `symlinked_env_folder_is_not_followed` and the invisible-prefix rows (T4 fr1, 26 more forms probed in review); `scripts_projection` `production_file_hidden_on_staging`; `scripts_goto` `production_scoped_file_offers_only_production` and `chooser_never_preselects_production`. R-T13-2 refuses a rename that changes a file's scope. All green at `c8812c7d`. |
| E4 | A secret-like literal triggers the warning | **PASS** (automated); demo step 6 **PENDING (PO)** | `secrets` `secret_table` and `allow_comment_silences_one_line` (T6, with the byte-identity proptests against M3's redactor); `scripts_secrets` `secret_literal_triggers_the_warning_on_save` and `warning_never_echoes_the_value` (T17, 15 tests). Release perf (§3a): a 2 MiB scan in 3 ms (budget 20). All green. |
| E5 | Git status badges match `git status` on a fixture repository | **PASS** (automated on all three OSes); demo step 7 **PENDING (PO)** | **Local, this commit:** `status_differential` `status_matches_git_cli` green, logging `HV-FACT t07 git_version=2.50.1 fixtures=23 equal=23`. `hostile_repo_runs_no_program` logs `HV-FACT t07 hostile os=macos detector=sandbox_exec spawned=Some(0) markers=0`, with the control at `spawned=Some(1)`. `scripts_projection` `git_badges_match_git_status_on_fixture`. Two independent reviews of the no-spawn core (T7 rounds 1–3, T20 rounds 1–5). `foreign_owner_repo_is_untrusted` is skipped without root (`HV-FACT t07 foreign_owner_repo_is_untrusted skipped uid=501`); its unit tests cover the check. **Gate runs:** build-test ubuntu of 45202154953 (job 137788196190, 0 failed) logs `HV-FACT t07 git_version=2.56.1 fixtures=23 equal=23`, `HV-FACT t07 hostile os=linux detector=strace spawned=Some(0) markers=0`, and the control `detector=strace spawned=Some(1)`. build-test windows of the PR run 45202171406 (job 137788236883, 0 failed) logs `HV-FACT t07 git_version=2.56.1.windows.1 fixtures=23 equal=23`, `HV-FACT t07 hostile os=windows detector=job_object spawned=Some(0) markers=0`, and the control `spawned=Some(1)`. |
| E6 | No credentials are ever written into a library folder | **PASS** (automated, local, and Docker on the gate runs); demo step 11 **PENDING (PO)** | **Local:** `manifest` `manifest_type_has_only_three_fields` (T3); `recovery` `recovery_lives_outside_the_library_and_is_private` (T5); the app test `scripts_no_credentials` `no_credentials_are_ever_written_into_a_library_folder` (T19); `docker_m4b` `no_credentials_reach_the_library_with_a_fake` and `mapping_lifecycle_writes_no_profile_id_near_the_library` (these need no server). The E6 surfaces are attach, map, share, save, recover and run. "Save as… into a library" and "Move console to library…" were struck by R-C0-12. **Docker:** `no_credentials_reach_the_library_on_real_servers` (pg17, mysql84). Gate runs: docker-full of 45202154953 and 45202181119, and docker-pr of 45202171406 (job 137788238050, green). Each logs `HV-FACT t19 e6 connections=2 paths=9 trash=1 needles=25 found=0 git_unchanged=true manifest_keys=aliases,database,format recovered=1` twice, with `HV-FACT t19 e6 target=pg17 audited=true` and `target=mysql84 audited=true`. |
| E7 | Security: path traversal, symlinks, untrusted `workspace.toml`, watcher limits, git never runs a program; fuzz targets clean for one nightly | **PASS** (automated, and one nightly fuzz-scripts on each gate dispatch) | **Tests, green at `c8812c7d`:** `pathsafe` `pathsafe_component_table`; `scan` `symlink_outside_is_listed_disabled_and_never_opened` and `scan_limits_stop_with_a_notice`; `fileio` `save_refuses_a_path_that_became_a_link`; `manifest` and `registry` (nesting guard, no replace); `watch_real` `dir_symlink_created_after_watch_is_not_walked`; `hostile_library` (15), `hostile_repo` and `hostile_repo_more` (6), `scripts_adversarial` (4). T20 found and fixed the alternates timeout (R-T20-2) and UNC/NTLM routes (fr4/fr5); 6 mutations go red. Three check-deps token rules are new, and `check-deps` is ok. **Fuzz:** T20 ran `workspace_toml` and `pathsafe` locally for 10 min each, with 0 crashes (7.4M and 10.5M runs; `pathsafe` again at 12.5M after fr1). **Gate fuzz-scripts, green on both dispatches, 600 s per target:** 45202154953 (job 137788196720) logs `workspace_toml` "Done 5858894 runs in 601 second(s)" and `pathsafe` "Done 10255046 runs"; 45202181119 (job 137788283448) logs 5,890,889 and 10,445,287 runs. The recorded residuals are in §9. |
| E8 | Carried M3 items: files and encodings (UI-EDIT-02), T2 preview, 20–100 MB paste confirmation, Run as script in constant memory, `StreamSplitter` MySQL hold fixed | **PASS** (automated, local, and Docker on the gate runs); the local 150 MB perf test has a test-isolation defect (carried) | **Local:** `fileio` `fileio_round_trip_table`, `save_keeps_mode_and_link_target` and `failed_save_leaves_original` (T5); the editor `t2_preview` and paste tests, with R-T11-2 (no editable paste above 20 MB); `sql-split` with `a_misread_block_is_a_stop_or_the_whole_split` (R-M4b-47) and `eager_is_exact_off_mysql` (R-T8-4); exec `script_run` with `run_as_script_stops_when_file_changes_mid_run` and `rename_replace_mid_run_keeps_old_inode` (T12, two passes on every dialect, R-T12-1/2); `scripts_run_as_script` (T18, 17 tests). **Release perf, this commit:** `mysql_dump_of_150_mb_runs_in_constant_memory` run on its own (`--exact`, one test thread), twice: `HV-FACT t12 target=fake-mysql bytes=159744000 units=153600 pass1_rss_growth_mib=13 peak_held=132456 secs=23` (budget < 64 MiB growth), and 13 MiB / 25 s the second time. Run together with the other ignored test of `script_run` (the plan's `-- --ignored` form), it **failed** with `pass1_rss_growth_mib=254`. The cause is a test-isolation defect, not the runner: `one_line_pg_script_over_64_mib_is_refused` runs on a parallel thread of the same process, and the assertion reads process-wide RSS. Carried (§9) as a test fix: serialize the two RSS perf tests. **Docker:** `hacivat-exec --test docker_script_run`: `mysql_dump_of_150_mb_runs_in_constant_memory_on_a_real_server`, `stray_end_file_executes_nothing_on_a_real_server` and `a_denied_statement_late_in_a_file_executes_nothing_on_a_real_server`. **Gate run 45202154953, docker-full:** `HV-FACT t19 e8 target=mysql84 bytes=157298639 units=10613 units_run=10613 rows=1698080 rss_before_mib=31 rss_peak_mib=51 rss_growth_mib=20 pass1_peak_growth_mib=13 peak_held=110176 secs=195` (bound < 64 MiB). `stray_end … refused=Uncertain(UnknownBlock) units_run=0` on mysql80, mysql84, mariadb106, mariadb1011 and mariadb114, each with `statements=…Questions:1` equal to `one_read`. `late_denial … at_unit=Some(8001) units_run=0 rows=0 asked=0` on pg14–18 (`pg_state_change_unmoved=true`) and the five MySQL-family lines. Dispatch 2 (45202181119): `rss_growth_mib=7 pass1_peak_growth_mib=0`. PR run docker-pr: `rss_growth_mib=8`. |
| E9 | No silent weakening: every M4b stub filled; `rg 'NotImplemented' crates/core/hacivat-scripts crates/core/hacivat-git crates/app/hacivat-app/src/scripts` empty outside tests | **FAIL** (the literal check). No unfilled stub was found. | The check prints 21 lines outside `tests/`. **Live code** (two lines): `watch/mod.rs:299` (the OS source after its debouncer is gone) and `watch/poll.rs:305` (the poll source after `stop`) return `Err(WatchError::NotImplemented)`. That is a stopped source, not a stub; both sources are filled (T9). **Declared variants that no product code constructs** (five): `RegistryError::NotImplemented` (`registry.rs:116`), `GitError::NotImplemented` (`hacivat-git/src/lib.rs:60`), `UnavailableReason::NotImplemented` (`service.rs:267`), `ReadError::NotImplemented` and `SaveError::NotImplemented` (`fileio.rs:196`, `:230`), `WatchError::NotImplemented` (`watch/mod.rs:135`). **Matches of those variants:** `projection.rs:852`, and `run_as_script.rs:220` (the exhaustive `ReadError` map) and `:1441` (in its `#[cfg(test)]` module). **Stale Task 1a docs:** `hacivat-scripts/src/lib.rs:18-23`, `service.rs:9-10`, `fileio.rs:5-6`, `hacivat-git/src/lib.rs:11`. No `todo!`/`unimplemented!` in the three paths. **Fix:** a small cleanup task, then a re-run of the check (section 0 item 1). |
| E10 | Shots: 0 violations; every M4 shot unchanged against `render-m4-gate-e7323aa/`; `pseudo-screens` green | **PASS** (harness, T22-pre on `c8812c7d`'s code); `pseudo-screens` **PENDING (CI)**; PNG sign-off **PENDING (PO)** | T22-pre fr1, on the code merged as `c8812c7d`, did not re-render (no UI code changed since): a full render of **5,772** PNGs in 4 shards with **0 violations**. Against `render-m4-gate-e7323aa`: 5,116 compared, **36 breached**, which is exactly `shot-superseded-m4b.txt` (the T16 "Script libraries" Settings nav entry: 9 states × dark/light × en-XA/tr-XT at 960); **656 new** M4b PNGs in `render-m4b-new/`; 0 other changes. The T22-pre review approved them, checking real frames. DESIGN.md "Script Workspace (M4b)" (T21, re-derived from the code in fr1). **Needed:** `pseudo-screens` (ubuntu 1/2, 2/2, macos 1/1) green on 45202154953 and 45202181119; the PO's sign-off of the 36 + 656 PNGs and DESIGN.md. |
| E11 | Gates green: clippy, the full test set and the perf lines, check-deps, l10n, tokens, `cargo deny`, `aws-lc-rs` absent; CI on the gate tip, every job green | Local lines **PASS** at `c8812c7d` (§3a), except the one perf line under E12; CI **PENDING (CI)**: 45202154953, 45202181119, 45202171406 | **Local (§3a):** clippy clean on every line; the full test set batched by crate is **8,038 passed, 0 failed**, 417 ignored, plus every feature line green; `check-deps` ok, including the three arms and `NoGitSubprocess`; `l10n check` 0 errors (96 L025 warnings); `tokens --check`, the generator `--check`s, `minicat --check` and `docker check-pins` ok; `cargo deny` bans, licenses and sources ok; `aws-lc-rs` matches no package. Perf lines: scan, status, secrets, reload and idle are green; `query_over_10k_entries_under_5ms` FAILS (E12). **CI at the time of writing: no job had failed.**
- **45202154953 green (23):** l10n-check, release-tests, turkey-test, tokens, switch ubuntu, build-test ubuntu, check-deps, fuzz-meta, launch-smoke ubuntu/windows, fuzz-sql, vault-linux, nightly-package ×3, minicat-verify, bench, cargo-deny, keyring-windows/linux, docker-full, fuzz-scripts, sql-differential.
- **45202181119 green (17):** docker-full, fuzz-scripts, sql-differential, launch-smoke ×3, keyring ×2, release-tests, l10n-check, minicat-verify, nightly-package windows/ubuntu, tokens, vault-linux, check-deps, cargo-deny.
- **PR 45202171406 green (11):** build-test windows, turkey-test, docker-pr, check-deps, cargo-deny, l10n-check, tokens, launch-smoke ubuntu/windows, switch ubuntu, minicat-verify. launch-smoke macos and switch macos show **cancelled**; the controller should check why, and rerun.
- **Still running:** build-test macOS ×3, build-test windows ×2, build-test ubuntu (dispatch 2 and PR), turkey-test (dispatch 2), pseudo-screens ×6, nightly-package macOS (dispatch 2), fuzz-meta, fuzz-sql and bench (dispatch 2), switch macOS ×2. **Needed:** every job green, namely build-test ×3, turkey-test, docker-pr (PR run), docker-full, fuzz-scripts, fuzz-meta, fuzz-sql, pseudo-screens and nightly-package ×3. A Windows build-test that times out in the post-job cache save after green tests is rerun with `--failed` (M4 lesson). |
| E12 | Budgets on the PO's Mac | **FAIL** as measured. 6 of 8 sub-budgets PASS (not a quiet machine); Go to script **FAIL** (marginal); watcher idle CPU **FAIL** (a steady debouncer tick). Quiet re-runs **PENDING (PO)** | See the table below. |

**Summary at `c8812c7d`.**

- **PASS on their automated evidence:** E1, E3, E4, E5, E6, E7 and E8, Docker facts from the gate
  runs included; and E2. Each still has its PO demo step.
- **FAIL:**
  - E9: the literal `NotImplemented` check. No unfilled stub was found.
  - E12: Go to script is 5–12 % over its 5 ms budget, and the watcher's idle CPU is a steady
    debouncer tick per library.
- **PENDING (CI):** E10's pseudo-screens and E11's remaining jobs.

**The gate does not close until section 0 items 1–4 are settled.**

### E12 budgets (measured by the gate agent on this Mac, build `c8812c7d`, release)

The fixture is a generated library of 10,000 `.sql` files, 100 folders of 100. It is a git
repository with one commit, packed with `git gc`, plus 100 modified files. The fixtures were
built under the slot's `tmp/` and deleted afterwards.

| Budget | Method | Result | Status |
|---|---|---|---|
| Scan of 10,000 files ≤ 500 ms, off the UI thread | `HACIVAT_PERF=1 cargo test --release -p hacivat-scripts --test scan -- --ignored` (`scan_of_10k_files_under_500ms`; real APFS tree) | `HV-FACT t04 scan_files=10000 took_ms=27`; 20,000 in 43 ms (budget 300). Through the real `LibraryService` (attach on the service's workers, never the caller's thread): attach → 10,000 entries listed and `Watching` in **136 ms** (the attach call itself returned in 53 ms). Load average 11. | **PASS** |
| External edit visible ≤ 1 s after the write | A scratch test (not committed) through `LibraryService::open` with the OS watch sources: open a file (`read`), wait 2.5 s (past R-T9-3's racy-clean window), write it in place, and time until `LibraryEvent::FileChanged` for that file. 20 files. | min 321, **p50 339**, p95 369, max **371 ms**. The tab's reload then adds a background diff and a UI landing ≤ 1 ms (next row). | **PASS** (service level; the visible tab with real VS Code is demo step 4) |
| Git status of 10,000 files ≤ 1 s | `HACIVAT_PERF=1 cargo test --release -p hacivat-git --test status_differential -- --ignored` (`status_of_10k_files_under_1s`) | `HV-FACT t07 perf status_10k_ms=75 changed=100`; 20,000 dataless excludes in 89 ms. Through the service: attach → git summary `Ready` in 236 ms (from the attach call, the first status included). This settles T0 U-05 (no gix `parallel` feature needed). | **PASS** |
| Go to script query ≤ 5 ms on 10,000 entries | `HACIVAT_PERF=1 cargo test --release -p hacivat-scripts --test index -- --ignored` (`query_over_10k_entries_under_5ms`: Turkish-tolerant `rep42` over 10,000 paths, best of 5 after a warm-up) | 9 runs: 5,011, 4,971, 5,013, 5,126, 5,063 and 5,069 µs at load 11–16, then 5,233, 5,369 and 5,587 µs at load 6. 1 of 9 under the budget. Lower load did not help, so the miss is not explained by load. This is the first release run (T10 reported debug only, 31 ms). | **FAIL** (5–12 % over; the PO re-measures or rules) |
| Reload UI share ≤ 1 ms on 2 MB | `HACIVAT_PERF=1 cargo test --release -p hacivat-ui-editor --lib -- --ignored reload_ui_share` | `HV-FACT m4b-t11 reload_ui_share_2mb case=middle worst_us=917`; `case=both_ends worst_us=386` (worst of 20 rounds; load 14). | **PASS** (close to the limit under load) |
| Idle CPU of the watcher ≈ 0, one sample per minute for 10 min | (a) The scratch service test: three libraries watched (10,000 + 100 + 100 files), process CPU from `ps` once a minute for 10 min. (b) The real release app (`hacivat`, isolated `HOME` with the same three libraries in `libraries.toml`), window open and idle, CPU from `ps` once a minute for 10 min. (c) `watch_real` `idle_cpu_is_recorded` (10 watchers, 5 s). | (a) Service only, 3 libraries: 210, 100, 150, 110, 130, 140, 160, 130, 170 and 160 ms of CPU per minute. (b) The real app with 3 libraries: 130, 240, 130, 170, 150, 160, 170, 170, 149 and 161 ms per minute, RSS 104–135 MiB. The **same app with no library**: 10, 140, 1,230 (one burst), 20 and 10 ms per minute over 5 minutes. (c) `HV-FACT t09 idle_watchers=10 wall_ms=5063 cpu_ms=20`. A 3-minute repeat of (a) without a git repository gave 220/200/190 ms, so git status is not the cause. **Cause, found by `sample`:** each library's `notify-debouncer-full` 0.7.0 thread ("notify-rs debouncer loop") wakes every `tick` = 150 ms / 4 = 37.5 ms forever, even with no event (`std::thread::sleep(tick)`, `notify-debouncer-full/src/lib.rs:674`). That is about 27 wakes per second per library, a steady ~0.25 % of one core for three libraries. It grows with the number of libraries (32 allowed) and keeps the process from idling. | **FAIL** (not ≈ 0: steady, about 10× the no-library baseline). Fix options in §0. |
| First frame ≤ 500 ms and idle RSS ≤ 200 MB, no library (`--smoke` ×3) | `target/release/hacivat --smoke` ×3 with an empty, isolated `HOME` (so the PO's own settings were not touched) | first frame **271 / 180 / 159 ms**; idle RSS **121 / 121 / 122 MiB** | **PASS** |
| First frame ≤ 500 ms and idle RSS ≤ 200 MB with 3 libraries attached (spec §3.3) | `--smoke` ×3 with an isolated `HOME` whose `libraries.toml` lists the three libraries (10,000 + 100 + 100 files, the large one a git repository). The library service opens at launch when the registry exists; it resolved and wrote back each library's canonical path and inode, so all three were opened. | first frame **146 / 162 / 162 ms**; idle RSS (5 s after the first frame) **138 / 139 / 138 MiB** | **PASS** (the PO repeats it on a quiet machine with real libraries, `po-demo-m4b.md` §2.2) |

## 2. Gate steps (Task 22)

| Step | What | State |
|---|---|---|
| 1 | W5 merged; clean every workspace member in the gate's slot | **Done.** W5 (T19–T21) and T22-pre are merged (`c8812c7d`). Slot `slot-ui-1`: `cargo clean -p <each of the 43 members>` removed 6,685 files, 4.1 GiB (debug), and 14 files from release. |
| 2 | clippy, the full test set, shot-diff, check-deps, l10n, tokens, deny, `aws-lc-rs`, perf lines; CI | **Local lines done** (§3a): all green except the Go to script perf line and the E9 check. The shot-diff is T22-pre's (E10). CI is the controller's, and was in progress. |
| 3 | E12 on the PO's Mac; `--smoke` ×3; machine, build hash | **Done headlessly** (E12 table; build `c8812c7d`). The quiet-machine and real-library re-runs are **PENDING (PO)**: `po-demo-m4b.md` §2. |
| 4 | `po-demo-m4b.md` | Written. Every step was pre-run through its automated tests. Real VS Code, Finder and Docker are the PO's. |
| 5 | `m4b.md` | This file |
| 6 | Commit `docs(m4b): gate record and PO demo checklist` | This commit (not pushed; the controller pushes) |

## 3a. Step 2 local gate lines at `c8812c7d` (gate agent)

- **When and where:** run 2026-11-23 in the merge worktree (`m4b` = `c8812c7d`, no code change),
  in the slot target `target-core/slot-ui-1`.
- **Environment:** `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`,
  `-j 6`, `--locked`; `--no-fail-fast` on every test line.
- **Slot:** test executables were deleted after each line. The slot stayed at or under 7.3 GiB
  for debug, and 11 GiB with the release builds.
- **Machine:** no Docker.

| # | Command | Result |
|---|---|---|
| 1 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| 2 | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | clean |
| 3 | `cargo clippy --locked -p hacivat-app -p hacivat-ui --all-targets --features hacivat-app/scene-check,hacivat-app/pseudo,hacivat-ui/scene-check -- -D warnings` | clean |
| 4 | the same for `-p hacivat-app` with `scene-check,pseudo,shots` | clean |
| 5 | `cargo clippy --locked --tests --no-default-features -p hacivat-introspect -p hacivat-meta -p hacivat-ddl -- -D warnings` | clean |
| 6 | `cargo test` over the 30 core and sql crates (`hacivat-scripts` and `hacivat-git` included) | **5,155 passed, 0 failed**, 385 ignored (Docker and perf), 578 s, load 6–18 |
| 7 | `cargo test -p hacivat-text -p hacivat-ui -p hacivat-ui-core -p hacivat-ui-editor -p hacivat-ui-grid -p hacivat-sql-editor` | **1,074/0**, 21 ignored |
| 8 | `cargo test -p hacivat-app` | **1,426/0**, 10 ignored |
| 9 | `cargo test -p hacivat -p xtask -p hacivat-bench` and the four spikes | 383/0, 1 ignored |
| 6–9 | the whole workspace, batched | **8,038 passed, 0 failed, 417 ignored** |
| 10 | `cargo test -p hacivat-ui-core --features test-support,scene-check` | 84/0 |
| 11 | `cargo test -p hacivat-l10n --features pseudo` | 152/0 |
| 12 | `cargo test -p hacivat-app --features scene-check,pseudo` | **1,442/0**, 10 ignored |
| 13 | `cargo test -p hacivat-testkit --features ssh` | 29/0, 1 ignored |
| 14 | `cargo test -p hacivat-ui-editor --features scene-check` | 307/0, 4 ignored |
| 15 | `cargo test -p hacivat-ui-grid --features test-support,scene-check` | 251/0, 4 ignored |
| 16 | `cargo test -p hacivat-meta --all-features` | 360/0, 9 ignored |
| 17 | `cargo test -p hacivat-driver --features fake` | 178/0 |
| 18 | `cargo test -p hacivat-ui --features scene-check` | 330/0, 2 ignored |
| 19 | `cargo test -p xtask l10n -- --ignored repo_catalogs_are_clean` | 1/0 |
| 20 | `HACIVAT_PERF=1 cargo test --release -p hacivat-scripts --test scan --test index --test secrets -- --ignored` | scan 2/0 (27 ms / 43 ms); secrets 1/0 (3 ms); **index 0/1** (5,011 µs; repeats in E12) |
| 21 | `HACIVAT_PERF=1 cargo test --release -p hacivat-git --test status_differential -- --ignored` | 2/0 (75 ms; 89 ms) |
| 22 | `HACIVAT_PERF=1 cargo test --release -p hacivat-scripts --test watch_real -- --ignored idle_cpu_is_recorded` | 1/0, `HV-FACT t09 idle_watchers=10 wall_ms=5063 cpu_ms=20` |
| 23 | `HACIVAT_PERF=1 cargo test --release -p hacivat-ui-editor --lib -- --ignored reload_ui_share` | 1/0 (917 µs / 386 µs) |
| 24 | `HACIVAT_PERF=1 cargo test --release -p hacivat-exec --test script_run -- --ignored` | `one_line_pg_script_over_64_mib_is_refused` ok; `mysql_dump_of_150_mb_runs_in_constant_memory` **FAILED together** (254 MiB, process-wide RSS shared with the parallel test) and **ok alone** ×2 (13 MiB; E8) |
| 25 | `rg 'NotImplemented' crates/core/hacivat-scripts crates/core/hacivat-git crates/app/hacivat-app/src/scripts` (E9) | **21 lines outside tests** (E9 row) |
| 26 | `cargo run -p xtask -- check-deps` | `check-deps: ok` |
| 27 | `cargo run -p xtask -- l10n check` | **0 errors**; 96 warnings, all L025 (an English id no code references yet) |
| 28 | `cargo run -p xtask -- tokens --check` | ok (exit 0) |
| 29 | `gen-sql-tables --check`, `gen-pg-visitor --check`, `gen-fn-catalog --check`, `minicat --check`, `docker check-pins` | all fresh or canonical, exit 0 |
| 30 | `cargo deny --all-features check licenses bans sources` (cargo-deny 0.20.2) | bans ok, licenses ok, sources ok |
| 31 | `cargo tree -i aws-lc-rs --workspace --all-features` (and `aws-lc-sys`); `Cargo.lock` | matches no package; 0 `aws-lc` entries in the lock |
| 32 | `cargo build --release -p hacivat`, then `--smoke` ×3 twice | E12 |

**Not re-run:** the shot render. No UI code changed after T22-pre's render on this code
(T22-pre's verification, E10).

The `linker_messages` warning (`ld: __eh_frame section too large`) shows when the big app test
binaries link. It is environmental and pre-existing (T13, T15 and T22-pre reviews), and it is
carried (§9).

## 4. CI runs cited

| Run | Ref | What it shows |
|---|---|---|
| 44949866840 | `m4b` `1d7f6ae` | First m4b run: fuzz-scripts builds with gix; the shot flake R-C0-6 (root-caused by T21) |
| 44981552034, 44990057141, 45068616161, 45077623376, 45091594060, 45102908608 | `m4b` W2–W4 tips | cifix-1..6 inputs (product bugs found: ext4 inode reuse, FSEvents create→remove cancel, Windows `same_file`, root identity on Windows, NTFS tunnelling) |
| 45103981137, 45109431480 | `m4b-t19-docker` | T19's E1/E6/E8 facts (31 `HV-FACT t19` lines), exact statement accounting; context only for this gate |
| 45146579446 | `m4b-cifix-6` | build-test macOS/ubuntu, turkey and docker-full green; the Windows timeout → R-C0-15 |
| 45155213467 | `m4b-t20-adversarial` | Windows git suites green (`hostile_repo_more` 8/0, `status_differential` 25/0) |
| 45169394359, 45169529481 | `m4b` `d5a4fd95` | The previous gate tip: PR run 45169529481 all green (docker-pr included); superseded by T22-pre |
| **45202154953** | `m4b` `c8812c7d` | **Gate dispatch 1**: 23 jobs green (docker-full, fuzz-scripts, build-test ubuntu, turkey, nightly-package ×3 …), none failed; build-test macOS/windows, pseudo-screens ×3 and switch/launch-smoke macOS still running: PENDING (CI) |
| **45202181119** | `m4b` `c8812c7d` | **Gate dispatch 2**: 17 green (docker-full, fuzz-scripts …), none failed; build-test ×3, turkey, pseudo-screens ×3 and others running: PENDING (CI) |
| **45202171406** | PR #6, `c8812c7d` | **Gate PR run** (`pull_request`): docker-pr, build-test windows and turkey-test green; build-test ubuntu/macOS running; launch-smoke macOS and switch macOS **cancelled**: PENDING (CI) |

## 5. Rulings R-M4b-1 to R-M4b-47 for PO confirmation

The full text is in `plan-m4b.md` "Assumptions and rulings" (the pre-flight rewrote superseded
sentences in place) and in ADR 0009 (`docs/decisions/0009-script-workspace.md`).

| # | One line | PO-Q | As built | PO confirm |
|---|---|---|---|---|
| R-M4b-1 | Two new core crates, `hacivat-scripts` and `hacivat-git` | | As ruled; check-deps arms | |
| R-M4b-2 | Library state in `<config>/libraries.toml`, one writer | Q18 | T3 `LibraryRegistry`, 0600 atomic; canonical path and inode persisted (T10 fr2) | |
| R-M4b-3 | Database level: PG database, MySQL schema, SQLite file stem | Q13 | As ruled (amended by R-M4b-32) | |
| R-M4b-4 | One tab per (file, connection); no in-tab switch | Q2 | T14; "Open under another connection…" opens a second tab | |
| R-M4b-5 | Explicit save; recovery copies outside the library | Q1 | T5/T14; copies 5 s after the last edit, keyed per (library, path, profile, database) | |
| R-M4b-6 | Environment folder rule: five tags, ASCII case-insensitive; anything else hides | Q3, Q8 | T4, with invisible-prefix hardening (fr1) | |
| R-M4b-7 | Binding: local mapping, else database name or alias, NFC | | T3/T13 | |
| R-M4b-8 | `workspace.toml` written once, after the user sees it | Q6 | T16: exactly the bytes shown, never over an existing file | |
| R-M4b-9 | The chooser: only visible connections, least severe first, production never preselected | Q14 | T15 (+ R-T15-1/2) | |
| R-M4b-10 | Git via `gix`, isolated, reduced trust, no drivers | Q4 | T7/T20; `Unknown` for filtered and dataless paths | |
| R-M4b-11 | Badges M / N / C / ?, folder dot, never red | | T13 (R-T2-1a colours) | |
| R-M4b-12 | `.sql` and `.md` only; Markdown does not run | Q12 | As ruled | |
| R-M4b-13 | Skip `.git`, `.hacivat`, dot folders, `node_modules`; `.gitignore` does not hide | | As ruled (C11) | |
| R-M4b-14 | Limits: 20,000 files, depth 32, 32 libraries; stop with a notice | Q16 | T4 (200k names cap, 256 diagnostics); a chain deeper than 32 stops the library (PO queue item 20) | |
| R-M4b-15 | Dataless files listed, opened on demand, git `Unknown` | | T4/T7 (R-T7-3/4: never download) | |
| R-M4b-16 | External changes: debounce 150 ms, echo table, clean tabs reload in place | | T9/T10/T11 (racy-clean R-T9-3) | |
| R-M4b-17 | Secret warning: library files only, at save, values never shown | Q7 | T6/T17 | |
| R-M4b-18 | Tree file ops: new, rename, Move to Trash, reveal, copy path | Q11 | T13; no-replace rename (R-T13-1); scope-changing moves refused (R-T13-2) | |
| R-M4b-19 | Scripts node only when a library is bound | Q9 | As ruled | |
| R-M4b-20 | Large files: T2 preview, Run as script streams and gates each unit | Q10 | T11/T12/T18 (two passes, R-M4b-47, R-T12-1) | |
| R-M4b-21 | `workspace.toml` schema v1 | | T3 | |
| R-M4b-22 | Session restore as `TabKind::Script` | | T1b/T14 | |
| R-M4b-23 | Names shown NFC, invisible characters as `<U+XXXX>`, identity by bytes | | T3/T13 | |
| R-M4b-24 | Streamed runs write one history row | | T12 | |
| R-M4b-25 | Scripts nodes per engine: PG only under the session's database | Q19 | As ruled | |
| R-M4b-26 | Contributed tree keys scoped by profile and database | | T13 | |
| R-M4b-27 | A script tab verifies its database before every Run; MySQL own session | Q21 | T14 (R-T14-2/3), Docker T19 | |
| R-M4b-28 | `hacivat-scripts` does not depend on `hacivat-exec` | | As ruled (+ `hacivat-git`, `serde`: R-T1a-2) | |
| R-M4b-29 | Script tabs are a tab kind; `workspace.json` v2 | | T1b | |
| R-M4b-30 | An unbound script tab is never bound by a connect | | T14 test | |
| R-M4b-31 | No Scripts node before a catalog exists; Go to script still offers it | Q23 | As ruled | |
| R-M4b-32 | SQLite libraries bind at the profile root only | | As ruled | |
| R-M4b-33 | ADR 0009 | | Done | |
| R-M4b-34 | T1b closes three M4 seam gaps as named exceptions | | T1b (+ the owner-only child lookup, accepted) | |
| R-M4b-35 | HV-EXEC-13 (explain plan) not in M4b, to the M5 pre-flight | Q20 | Carried (§9) | |
| R-M4b-36 | Docker proofs on CI only; `hacivat-scripts` in the Docker crate list | | As ruled | |
| R-M4b-37 | Chords: Go to script ⇧⌘O / Ctrl+Shift+O; Save ⌘S / Ctrl+S in script tabs; no Save all chord off macOS | Q22 | T1b | |
| R-M4b-38 | Reload held during a composition; amended by R-T11-1 (refuses on an edited version) | | T11 | |
| R-M4b-39 | The unsaved-changes prompt (Cancel focused) | | T14 | |
| R-M4b-40 | M4b fuzz targets in their own package `fuzz/scripts/` | | T1a/T20 | |
| R-M4b-41 | check-deps placement, `NoGitSubprocess` token rule | | T1a (+ fr1 evasion forms) | |
| R-M4b-42 | `workspace.toml` written with ordinary permissions | | T3 | |
| R-M4b-43 | Focus recovery and Tab stops on every new surface | | T13–T18, T21 | |
| R-M4b-44 | Shots leave no temp directory | | T21 | |
| R-M4b-45 | Nested Scripts folders start collapsed after a restart | | T13 | |
| R-M4b-46 | Eager splitter policy for the streamed run | | **Superseded by R-M4b-47** | |
| R-M4b-47 | MySQL/MariaDB Run as script makes two passes; any stop in pass 1 refuses the whole file (T12 applies it to every dialect, R-T12-1) | | T8/T12/T18, Docker T19 | |

## 6. Wave and controller rulings (for confirmation)

Short form; the full text and reason of each is in `progress.md`.

| Ruling | One line | PO |
|---|---|---|
| R-C0-1 | The plan binds after the pre-flight; the PO-question defaults apply | |
| R-C0-2 | Spike builds in a pool slot, not under `/tmp` | |
| R-C0-3 | T0, T2 and the pre-flight in parallel | |
| R-C0-4 | No vendor-shaped token literal in a committed file (split at run time) | |
| R-C0-5 | W1 core tasks start before T1b | |
| R-C0-6 | The `#editor-placeholder` shot flake does not block W1 (root-caused by T21) | |
| R-C0-7 | Merge order T9, then T10, with the watcher wiring | |
| R-C0-8 | No `cargo clean` without `-p` | |
| R-C0-9 | One CI fix-up task per round; a cross-target `cargo check --tests` is standing | |
| R-C0-10 | At most two gpui builds at once; ≥ 25 GiB free before one | |
| R-C0-11 | CI test steps run with `--no-fail-fast` | |
| R-C0-12 | "Save as… into a library" and "Move console to library…" struck from M4b (M5 or later) | PO queue |
| R-C0-13 | T19 merges before cifix-6 | |
| R-C0-14 | `opt-level = 2` for `sql-lexer`, `sql-split`, `sha2` in dev builds | |
| R-C0-15 | build-test timeout 90 → 150 min | |
| R-T0-1 | No-op `SubmoduleStatus`: submodules show Unknown | |
| R-T0-2 | Watcher with `NoCache` and `follow_symlinks(false)` | |
| R-T1a-1..4 | The "git" literal rule scoped; T1a's five deviations; no proptest glob; random temp suffixes | |
| R-T1b-1 | A script tab stores the database it was opened under | |
| R-T2-1..5 | Git colours (New amber, Conflicted ink); lowercase "shared"; F2 renames, no Trash chord; the connection name never truncates; the Danger buttons | Trash chord: PO queue |
| R-T3-1 | macOS check-then-act window around `.hacivat`/save accepted (no `openat` without unsafe) | |
| R-T4-1..3 | Link targets resolved as text inside the root; `join` returns what it resolved; Windows root spelling | |
| R-T5-1..6 | Restore on failed in-place write; no canonicalize; recovery keyed per (library, path, profile, database) + instance; no redaction in copies; copies without a lock still written; env required on every file call | |
| R-T6-1 | The M3 vertical-tab redactor hang fixed in T6 | PO queue (hotfix to main?) |
| R-T7-1..6 | Case-only mismatch Unknown; git control files regular and capped; dataless → Unavailable; never download; no replace refs; loose objects lstat-capped | |
| R-T8-1..5 | Cap decision text-only; byte cap 64 MiB; refusal of cap-exceeding inputs; Eager = Exact off MySQL, no lone-CR anchors; the `test-support` manifest exception | |
| R-T9-1..6 | "Watch: automatic / poll" per library (no network-fs detection off Linux); Windows 30 s root look; racy-clean rule; Linux per-folder watches; the watcher's own thread; after an overflow only open files reported | R-T9-1: PO queue |
| R-T10-1..7 | Go-to ranking table text; per-subscriber env filter; `open_stream` stamps and hashes; bounded worker pool; 10 s job deadline; `ManifestChanged`; keep the last good binding while the manifest is unreadable | |
| R-T11-1/2 | `replace_from_disk` refuses on an edited version; 20–100 MB pastes offer Cancel / Paste as preview only | T11 m2 decimals: PO queue |
| R-T12-1/2 | Two passes on every dialect; pass 1 runs the static gate over every unit, a "No" stops the run with the confirmed prefix applied | R-T12-2: PO queue |
| R-T13-1/2 | No-replace rename everywhere; scope-changing rename or move refused | R-T13-2: PO queue |
| R-T14-1..5 | SQLite tab stores the file identity; fresh server database probe before every Run; MySQL tab never shares a session; Run and Save off after "No longer listed"; remount-follow only on the first Run after a restore | |
| R-T15-1/2 | The starting connection is left out entirely; activation opens only the unchanged highlighted row | |
| R-T16-1/2 | Never prefill a node's local name from the mapping origin; a plain attach from the database menu keeps the node-name prefill | |
| R-T17-1/2 | T17's six deviations accepted; the ▶ takes the gutter slot | |
| R-T18-1/2 | A pasted preview inherits its tab's binding; Stop wording "Stopped during statement N; statements after it were not sent." | R-T18-2: PO queue |
| R-T19-1 | `hacivat-scripts` dev-dependencies `hacivat-results`, `sql-dialect`, so E1 runs through the real runner | |
| R-T20-1/2 | `Untrusted(GitdirOutside)` superseded by `Unavailable(Redirected)`; alternates allowed only as a root file of ≤ 16 absolute byte-exact entries, else Redirected | |
| R-T21-1 | The script tab title shortens by width; the full connection name shows while the active tab fits | |
| R-T22-1/2 | In a console tile during a run: toolbar and trust, then strip and Stop, then the most severe notice, then two code lines; a focused control is never obscured | |
| R-T22-3 | ρ1/ρ2 recorded as M4b residuals; the fix carried to the M5 pre-flight (§9) | |
| R-CI2-1..4 | btime is an identity part on Linux only; index heads validated before any fs call; `index.lock` bounds a checkout burst; host-only name rules | |

**PO queue items** (`po-bekleyen.md` item 20), all open:

- R-M4b-46 → R-M4b-47;
- HV-EXEC-13 to M5;
- Q20–Q23;
- the Trash chord;
- the depth-32 library rule;
- three M3 redaction gaps (EXECUTE IMMEDIATE; NBSP, lone CR and `Bearer`+tab; `dbpassword`/`sslpassword` values);
- the R-T6-1 hotfix question;
- R-T9-1 (network shares);
- the T11 size decimals;
- R-T12-2, R-T13-2, R-T18-2 and R-C0-12;
- the Roll back wording (T18): today "Stopped at statement N: the server returned an error." The
  proposal: "Stopped at statement N: you rolled it back. The statements before it ran and stay
  applied; nothing after it was sent."

## 7. PO questions (`po-questions-m4b.md`)

Every default is in force as a ruling (R-C0-1); none was overturned.

| Q | Topic | Default applied |
|---|---|---|
| Q1 | Autosave or ⌘S | ⌘S only, with recovery copies (R-M4b-5) |
| Q2 | Same script under two connections | Two tabs, the connection fixed per tab (R-M4b-4) |
| Q3 | `@prod/` and other non-tag folders | Hidden everywhere, with a "did you mean" diagnostic (R-M4b-6) |
| Q4 | Git engine | `gix`, isolated, reduced trust (R-M4b-10); T0 parity 0 differences |
| Q5 | Where manual mappings live | `libraries.toml` only; one tick shares an alias (R-M4b-8) |
| Q6 | Create `workspace.toml` on attach | Yes, once, after showing it (R-M4b-8) |
| Q7 | What "warn before saving" means | One dialog at save, lines without values (R-M4b-17) |
| Q8 | Untagged profiles see `@env` folders | No (R-M4b-6) |
| Q9 | Scripts node everywhere | Only when bound (R-M4b-19) |
| Q10 | Run as script for huge files in M4b | Kept in M4b (T8, T11, T12, T18) |
| Q11 | Delete from Hacivat | Move to Trash only (`trash`, NsFileManager) |
| Q12 | Which files | `.sql` and `.md` (R-M4b-12) |
| Q13 | "The database" for MySQL and SQLite | Schema; file stem (R-M4b-3, R-M4b-32) |
| Q14 | Go to script with one candidate | Opens directly unless production; production always asks (R-M4b-9) |
| Q15 | Symbolic links | Inside file links work; outside listed disabled; folder links not followed |
| Q16 | Library size | 20,000 files, depth 32, 32 libraries; polling fallback (R-M4b-14) |
| Q17 | Side-by-side diff on change | Not in M4b (Reload / Keep mine / Overwrite?) |
| Q18 | Where the library list lives | `libraries.toml` (R-M4b-2) |
| Q19 | Scripts under PG's other databases | No; only the session's database (R-M4b-25) |
| Q20 | Explain plan in M4b | No; M5 pre-flight (R-M4b-35) |
| Q21 | MySQL tab on another database may open a connection | Yes, its own session (R-M4b-27) |
| Q22 | Save all chord on Windows/Linux | None (R-M4b-37) |
| Q23 | Scripts node before databases are known | Only under listed databases; Go to script offers every connection (R-M4b-31) |

## 8. The C-M4-1..9 picks (pre-flight, `preflight-m4b.md` P-M4b-3..10, -24)

| Row | Seam | Pick | Later change |
|---|---|---|---|
| C-M4-1 | Tree widget | Used as shipped, no M4 edit | T13 named exception in `row.rs` colours (R-T2-1a) |
| C-M4-2 | `ExplorerContributor` | Shape, with three seam gaps closed by T1b (activate, menu, redraw; R-M4b-34) | The owner-only child lookup (T1b m8), accepted |
| C-M4-3 | Database listing | Shape with fail-closed limits (R-M4b-25, R-M4b-31) | none |
| C-M4-4 | Database identity | Fallback `(ProfileId, DatabaseName)`; SQLite at the root only (R-M4b-32) | none |
| C-M4-5 | Console on another database | Fallback for PG/SQLite (the session's database); MySQL gets its own session per tab (R-M4b-27) | R-T14-2/3: a fresh probe before every Run; never a shared session |
| C-M4-6 | Database level per engine | Shape, used by the app only | none |
| C-M4-7 | Refresh/generation events | Shape plus C-M4-2's redraw fix | none |
| C-M4-8 | Expanded state across restarts | Shape for the Scripts node; nested folders start collapsed (R-M4b-45) | none |
| C-M4-9 | No M4 code writes in a library folder | Holds (E6 audit) | none |

## 9. Carried items (M5 and later), M4b residuals included

| Item | To | Origin |
|---|---|---|
| HV-EXEC-13 explain-plan model and views | **M5 pre-flight** | R-M4b-35, Q20 |
| R-T22-3 ρ1/ρ2: in a 120 px console tile, a focused lower notice pushes the run strip out, or the most severe notice's focused button cannot show; the toolbar Stop stays visible. Fix: raise the console/results split floor (about 180 px) while a Run-as-script strip shows | **M5 pre-flight** (M4b residual) | T22-pre re-review, R-T22-3 |
| "Save as… into a library" and "Move console to library…" (R-M4b-17's share surfaces) | M5 or later | R-C0-12 |
| Scripts under SQLite attached databases | M5 (M4's R-W4-2 seam) | R-M4b-32 |
| One parser-based redaction pass for the M3 history gaps: EXECUTE IMMEDIATE text; NBSP, lone CR and `Bearer`+tab; `dbpassword`/`sslpassword` values | M5 (PO decides) | T6 reviews; PO queue |
| T12 n2: pass-1 classification context across chunks (only over-refuses or over-asks; pass 2 gates every unit) | M5 | T20 carry 7, ADR 0009 |
| T12 n4: MariaDB 8-splitter worst-case RSS (≈ 2.3 GB computed, not measured); an up-front refusal | M5 (language-intelligence pass on the splitter) | T20 carry 8 / m7 |
| T20 residual: a mapped or `subst` Windows drive letter passes the local-path classifier (the user chose its host) | recorded (ADR 0009 `m4b-t20`) | T20 fr3–fr5 |
| T20 residual: the live-writer TOCTOU on an alternate folder the user owns (bounded by the 5 s budget, one worker per library, and the backoff; outside the hostile-library model) | recorded (ADR 0009) | T20 re-review fr3, m-fr3-1 |
| T20 residual: the config every-value scan's false-positive class (a value that only starts like a network path is refused) | recorded (ADR 0009) | T20 m-fr4-1 |
| T20 residuals: a symbolic link defeats the lexical guard (m-fr4-2) and a rooted path depends on the current drive (m-fr4-3, now refused on Windows) | recorded | T20 fr4/fr5 reviews |
| T18 n1/n3: the transaction chip's Commit/Roll back during a streamed run; the loss listener always started | recorded (ADR 0009) | T20 carries |
| cifix-4 m1: a restored Linux tab with a legacy (no birth time) id shows the same file twice (one-time, pre-release) | recorded | T20 carry 23 |
| T10 residual: an open file deleted within 10 s of an identical copy is announced as moved to the copy | recorded | T10 fr3 |
| T5 residuals: xattrs and ACLs lost on rename-replace; in-place writes for hard-linked files not atomic (restore on error, R-T5-1); the R-T3-1 macOS check-then-act window | recorded (ADR 0009) | T3, T5 |
| `introspect_live_choice_persists` (M4 flake, `metadata_settings`): instrumented with an `HV-FACT m4b-t22-pre … failed_in=…` line on failure; not reproduced in 824 local runs. If CI fails it, the fact names the step | watch on CI; root-cause on first failure | T21 m12, T22-pre |
| The `linker_messages` warning (`ld: __eh_frame section too large (max 16MB)`) on the big app test binaries | M5 or later (split the big test binaries, or accept) | T13/T15/T22-pre reviews |
| Test isolation: `script_run`'s two ignored RSS perf tests run in parallel in one process, so the 150 MB test's process-wide RSS assertion fails when both run (`-- --ignored`). Serialize them (a static lock, as cifix-6 did) or move one to its own target | before the next perf run (small test fix) | this gate (E8) |
| E9 cleanup: dead `NotImplemented` variants, `WatchError::NotImplemented` meaning "stopped", stale Task 1a docs | **before the gate closes** (§0) | this gate |
| Watcher idle CPU: `notify-debouncer-full`'s 37.5 ms tick per library (E12), with the fix options in §0 item 3 | **before the gate closes**, or a PO ruling | this gate |
| Go to script release budget (5 ms at 10,000 entries) at the limit on this Mac | **before the gate closes** (§0): quiet re-run or ruling | this gate |
| notify 9.x (needs a ruling on BSL-1.0 `xxhash-rust`) | after M4b | T0 U-11 |
| Windows `build-test` post-job cache-save timeout (M4 carry) | watch; R-C0-15 raised the cap | M4, R-C0-15 |

## 10. M5 pointer

M5 "Language intelligence" is planned in
`.superpowers/sdd/2026-11-17-m5-language-intelligence/plan-m5.md`. Its state:

- v2, docs only, reviewed in `plan-m5-review.md`;
- `po-questions-m5.md` is next to it;
- spec §6 M5 is the target: completion p95 < 20 ms on a 1 MB file, the corpus families, formatter
  idempotency, and ≥ 98 % differential agreement with libpg_query.

Its pre-flight starts from this gate's tip and takes, besides its own items:

- HV-EXEC-13 (explain plan; R-M4b-35);
- the R-T22-3 split-floor fix;
- the redaction pass for the M3 history gaps;
- T12 n2/n4 (the splitter context and the MariaDB bound);
- SQLite attached-database scripts;
- the struck share surfaces (R-C0-12).

M4b ships no parser change that M5's plan must re-read. One exception: `sql-split`'s `Eager`
policy, which differs from `Exact` only on MySQL/MariaDB (R-T8-4), and its doc-hidden
`held_items()`.

## 11. Notes found while writing this record

1. **The Go to script perf line had never run in release before the gate.** T10 measured debug
   only and left the release run to the gate; the first release run is at the budget (E12).
2. **E9's literal check was never run by a task.** Every filling task removed its own stub's
   behaviour, but not the variant or the module doc that named it. The live
   `WatchError::NotImplemented` returns are T9's choice for a stopped source.
3. **Demo step 7 as planned would show nothing.** `git add` changes no badge under R-M4b-11, so
   `po-demo-m4b.md` uses a commit instead.
4. **The E12 "external edit" figure is a service-level number.** It is measured from the write to
   the `FileChanged` event the tab reloads from. The visible tab adds a background diff and a UI
   landing ≤ 1 ms. The end-to-end check with real VS Code is demo step 4.
5. **The watcher's idle cost was never measured past 5 s.** T9's `idle_cpu_is_recorded` (10 watchers, 5 s, 20 ms) records the figure but asserts nothing. At this tick rate, the per-minute figure is what shows the steady cost.
