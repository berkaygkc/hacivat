# 28 — Script workspace (L14, local scope): Implementation-Ready Algorithm Catalog

> Date: 2026-11-21. Status: binding spec for milestone M4b once merged (plan `docs/superpowers/plans/2026-11-21-m4b-script-workspace.md`, Task 0). Spec IDs are `HV-SCRIPT-NN`. The provisional text was the plan's rulings R-M4b-1..25. This file corrects them with two throwaway spikes run on macOS (spike A: `gix` status; spike B: `notify` events) and with the cited sources. Where a spike or a source contradicts a ruling, the correction is marked **Correction** and repeated in §0.6.

## 0. Front matter

### 0.1 Scope

In scope (spec §6 M4b, decision L14 local scope): the library registry and `.hacivat/workspace.toml`; logical-database binding; the safe scanner; `@<environment>/` scoping; projection into the explorer; context-from-location opening; file I/O with encodings and recovery; the watcher; "Go to script"; secret-literal warnings; read-only git status; large files (T2 preview, paste confirmation, reduced Run as script).

Out of scope: Perde team libraries, in-app commit/pull/push, git network operations, side-by-side diff, multi-target runs (deferred list in the plan; D-091).

### 0.2 Sources

Evidence tags (plan Task 0 Step 4, extending `docs/research/README.md`): **[Observed: …]** is a spike result; **[Documented: …]** cites a source from the table below (or a file in this repository); **[Inferred: …]** says what it rests on; **[Unknown]** is open and names the test that settles it (§0.7). "Spike A" and "spike B" are the Task 0 spikes in §0.4 and §0.5. Their code was throwaway and lived outside the repository; their scripts and numbers are recorded here.

| # | Source | Version / date | Licence | Used for |
|---|---|---|---|---|
| S1 | `git-status(1)`, `git-config(1)`, `gitattributes(5)`, `githooks(5)`, `git(1)` manual pages, read locally | git 2.50.1 | GPL-2.0 (docs read, nothing copied) | which config makes `git status` run a program; porcelain v2 format; `safe.directory`; `--no-optional-locks` |
| S2 | git-scm.com `git-config` page, sections `core.fsmonitor`, SCOPES | fetched 2026-11-21 | GPL-2.0 (docs) | "pathname of the fsmonitor hook command"; protected configuration |
| S3 | `gix` 0.90.0 source (`src/open/{options,permissions,repository}.rs`, `src/filter.rs`, `src/config/cache/access.rs`, `src/status/**`), `gix-path` 0.14.1 `src/env/mod.rs`, `gix-status` 0.37.0, `gix-diff` 0.70.0, `gix-filter` 0.36.1, from the cargo registry | crates.io, published 2026-10-22 | MIT OR Apache-2.0 | the trust model, driver filtering, spawn sites |
| S4 | `notify` 8.2.2 and 9.0.0, `notify-debouncer-full` 0.7.0 source and CHANGELOG, from the cargo registry | crates.io | CC0-1.0; MIT OR Apache-2.0 | FSEvents flags and latency, `follow_symlinks` default, poll defaults, debouncer behaviour |
| S5 | crates.io API `/api/v1/crates/<name>/versions` for gix, notify, notify-debouncer-full, trash, encoding_rs, xxhash-rust | fetched 2026-11-21 | — | release dates, licences |
| S6 | `inotify(7)`, man7.org | man-pages 6.19 | GPL (docs) | limits, overflow, non-recursive watches, network file systems, rename cookies |
| S7 | Apple, *File System Events Programming Guide* (archive), "Using the File System Events API" | updated 2012-12-13 | Apple docs | `MustScanSubDirs`, dropped events, `lstat` advice, advisory nature |
| S8 | Microsoft Learn, `ReadDirectoryChangesW` | updated 2025-07-01 | Microsoft docs | buffer overflow, `ERROR_NOTIFY_ENUM_DIR`, 64 KB network limit, short names |
| S9 | AWS IAM User Guide, "IAM identifiers", section "Understanding unique ID prefixes" | fetched 2026-11-21 | AWS docs | `AKIA`, `ASIA` |
| S10 | GitHub Docs, "About authentication to GitHub" (token formats); GitHub blog, "Behind GitHub's new authentication token formats" (2021) | fetched 2026-11-21 | GitHub docs | `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_`, `github_pat_`; the `ghs_` length change |
| S11 | Slack developer docs, "Tokens" | fetched 2026-11-21 | Slack docs | `xoxb-`, `xoxp-`, `xapp-`, `xwfp-`, `xoxe.xapp-` |
| S12 | Stripe docs, "API keys" | fetched 2026-11-21 | Stripe docs | `sk_live_`, `rk_live_`, `pk_live_`, test prefixes |
| S13 | Google Cloud docs, "Manage API keys" | fetched 2026-11-21 | Google docs | that the key format is **not** documented there |
| S14 | RFC 7519 (JWT) §3, §6, §7.2; RFC 7515 §2, §3.1 | IETF | IETF trust | three base64url parts, empty signature allowed |
| S15 | RFC 7468 (PEM) §2, §3, §10, §11 | IETF | IETF trust | `-----BEGIN <label>-----`, `PRIVATE KEY`, `ENCRYPTED PRIVATE KEY` |
| S16 | RFC 3986 §3.2.1 | IETF | IETF trust | `user:password` userinfo is deprecated |
| S17 | VS Code docs, "Workspace Trust" | fetched 2026-11-21 | MIT (docs) | prior art: a new folder opens restricted, "prevent automatic code execution" |
| S18 | IntelliJ IDEA help, "System settings" (2026.3) | fetched 2026-11-21 | JetBrains docs | "Back up files before saving … known as safe write" |
| S19 | Vim help, `'backupcopy'` (Vim 9.2) | fetched 2026-11-21 | Vim licence (docs) | `yes` copies and overwrites, `no` renames and writes new, `auto` picks |
| S20 | This repository: `alg/25` UI-EDIT-02, -04, -12, -14, -23; `alg/20` HV-EXEC-12, -15; `alg/26` I18N-09; `datagrip/10` §3.4–3.5, §3.11; `datagrip/11` ALG-SET-01; `crates/core/hacivat-exec/src/history/redact.rs`; `crates/core/hacivat-proto/src/lib.rs` (`EnvTag`) | branch `m4b` at `56f5cec` | ours | the machinery M4b reuses |

Clean room: no Zed GPL crate, nothing under `.refs/`, no DataGrip decompilation. Secret patterns in HV-SCRIPT-10 are written from the vendor facts above in our own words; no rule file of any scanner was read or copied.

### 0.3 Spec index

| ID | Title | Owner tasks | Priority |
|---|---|---|---|
| HV-SCRIPT-01 | Library registry and `.hacivat/workspace.toml` | T3 | **v1** |
| HV-SCRIPT-02 | Logical-database binding | T3 | **v1** |
| HV-SCRIPT-03 | Library scan and path safety | T4 | **v1** |
| HV-SCRIPT-04 | Environment scoping `@<environment>/` | T4 | **v1** |
| HV-SCRIPT-05 | Projection into the explorer | T13 | **v1** |
| HV-SCRIPT-06 | Context-from-location opening, script tabs, restore | T14 | **v1** |
| HV-SCRIPT-07 | File I/O: decode, encodings, atomic save, recovery copies | T5 | **v1** |
| HV-SCRIPT-08 | Watcher and external changes | T9, T11, T14 | **v1** |
| HV-SCRIPT-09 | "Go to script" and the connection chooser | T10, T15 | **v1** |
| HV-SCRIPT-10 | Secret-literal detection and the save warning | T6, T17 | **v1** |
| HV-SCRIPT-11 | Read-only git status | T7, T13 | **v1** |
| HV-SCRIPT-12 | Large files: T2 preview, paste confirmation, reduced Run as script | T8, T11, T12, T18 | **v1** |

### 0.4 Spike A — `gix` status (macOS, git 2.50.1)

**Setup.** A throwaway binary opened the repository found by walking up from the library root to a `.git` entry, with `gix::open::Options::isolated().with(gix::sec::Trust::Reduced)`, then ran `repo.status(Discard)?.untracked_files(UntrackedFiles::Files).index_worktree_submodules(Given { ignore: All, .. }).into_iter([":(top,literal)<library prefix>"])`. It mapped items to `{Modified, New, Conflicted}` (precedence Conflicted > New > Modified), dropped paths absent from the worktree, and never called `Outcome::write_changes`. A Python mapper turned `git --no-optional-locks status --porcelain=v2 -z --untracked-files=all [-- <prefix>]` into the same three states (`1`/`2` records: `X ∈ {A,R,C}` → New, else `X` or `Y` ∈ `{M,T}` → Modified; `u` → Conflicted; `?` → New). Both sides ran with `GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1`; `gix` first, then the CLI (which may refresh the index).

**Parity table** [Observed: spike A, two full runs, identical results]:

| Fixture | Paths with a badge | gix = CLI | Index bytes unchanged by gix |
|---|---|---|---|
| clean | 0 | yes | yes |
| worktree-modified (`a.sql` rewritten after 1.1 s) | 1 (Modified) | yes | yes |
| index-modified (`git add` of a change) | 1 (Modified) | yes | yes |
| untracked (`new.sql`, `fresh/n.sql`) | 2 (New) | yes | yes |
| index-added | 1 (New) | yes | yes |
| deleted (worktree `rm`, and `git rm --cached` + `rm`) | 0 (absent on both sides) | yes | yes |
| staged-rename (`git mv a.sql renamed.sql`) | 1 (New: the destination) | yes | yes |
| conflict (merge with a content conflict) | 1 (Conflicted) | yes | yes |
| autocrlf-crlf (`core.autocrlf=true`, CRLF files, one changed) | 1 (Modified); unchanged CRLF files clean | yes | yes |
| library-is-subfolder (changes inside and outside `lib/`) | 2, only inside `lib/` | yes | yes |
| nested-untracked-dir (`deep/er/est/z.sql`) | 2 (New, per file) | yes | yes |
| ignored (`.gitignore` hides a file and a folder) | 1 (the visible file) | yes | yes |
| mode-change (`chmod +x`) | 1 (Modified) | yes | yes |
| added-then-modified (`AM`) | 1 (New) | yes | yes |
| same-size-same-second (racy rewrite, same length, no wait) | 1 (Modified) | yes | yes |
| touched-unchanged (`touch` after 1.1 s) | 0 | yes | yes |
| unicode-names (NFD name on disk; `core.precomposeUnicode=true`) | 2; both report the NFC bytes | yes | yes |
| symlink-typechange (file replaced by a symlink) | 1 (Modified) | yes | yes |

18 fixtures (the 10 of the plan's Step 2 plus 8 extras), 0 disagreements.

**Hostile repository** [Observed: spike A]. Local config set `core.fsmonitor`, `filter.x.{clean,smudge,process,required}`, `diff.external`, `diff.x.{textconv,command}`, `core.hooksPath` (with `post-index-change`, `reference-transaction`, `pre-commit`), the same hooks in `.git/hooks`, `core.sshCommand`, `credential.helper`, `core.pager`, `core.editor`, `gpg.program`, `core.askPass`, `sequence.editor`, `core.gitProxy`, `uploadpack.packObjectsHook`, `merge.x.driver`, `safe.directory=*`, `include.path` and `includeIf.gitdir:/.path` (both setting `core.fsmonitor`), and `.gitattributes` `*.sql filter=x diff=x merge=x`; every program touched its own marker. The environment was hostile too: `PATH` started with a fake `git` that touches a marker, `HOME`/`XDG_CONFIG_HOME` held a `.gitconfig` and `git/config` setting `core.fsmonitor` and `safe.directory=*` and `git/attributes` `* filter=x`, `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`, `GIT_CONFIG_COUNT/KEY_0/VALUE_0`, `GIT_EXTERNAL_DIFF`, `GIT_SSH_COMMAND` were set, and `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE` pointed at a different (decoy) repository. The binary ran under `sandbox-exec` with `(deny process-fork (with send-signal SIGKILL))`, so **any** attempt to spawn a process kills it (exit 137); a control binary that spawns `/usr/bin/true` exits 137 under the same profile and 0 without it.

| Result | Value |
|---|---|
| Markers created | **none** |
| Exit status | 0 (no spawn was even attempted) |
| Index bytes | unchanged |
| Output | `a.sql` Modified, `new.sql` New, `.gitattributes` New; the attribute probe reports `filter` set on `a.sql` and `new.sql` |
| Decoy repository | not used (its untracked files did not appear) |
| Global attributes (`* filter=x` in the fake home) | not read (`.gitattributes` itself carried no `filter`) |
| Repository config sections | all seen with `trust=Reduced`; `gix::config::section::is_trusted` false for every one |

**Per-vector controls** [Observed: spike A]: one repository per vector, the git CLI as a positive control, `gix` under the same sandbox.

| Vector | CLI command | CLI ran the program | gix ran it / attempted a spawn |
|---|---|---|---|
| `core.fsmonitor` | `git status` | yes | no / no |
| `core.fsmonitor` via `include.path` | `git status` | yes | no / no |
| `core.fsmonitor` via `includeIf` | `git status` | yes | no / no |
| `core.fsmonitor` with `--no-optional-locks` | `git --no-optional-locks status` | **yes** | no / no |
| `filter.x.clean` + `.git/info/attributes` | `git status` | yes | no / no |
| `filter.x.process` + `.git/info/attributes` | `git status` | yes | no / no |
| `filter.x.clean` + in-tree `.gitattributes` | `git status` | yes | no / no |
| `diff.external` | `git diff` | yes | no / no |
| `diff.x.textconv` | `git diff` | yes | no / no |
| `post-index-change` in `.git/hooks` | `git status` | yes in run 1, no in run 2 (git writes the index only when its refresh finds stale stat data) | no / no |
| `post-index-change` via `core.hooksPath` | `git status` | yes | no / no |

**Negative controls: which `gix` options are load-bearing** [Observed: spike A].

| Open options | Global config | `git_dir_trust` after open | Filter driver ran |
|---|---|---|---|
| `isolated().with(Reduced)` | `safe.directory = *` | Reduced | **no** |
| `isolated()` (trust from ownership) | — | Full | **yes** (sandboxed: spawn attempted, exit 137) |
| `default().with(Reduced)` (not isolated) | `safe.directory = *` | **Full** (promoted) | **yes** |

So both halves are required: without `with(Reduced)`, an owned repository is fully trusted and its `filter.<x>.clean`/`process` runs during status; without `isolated()`, a user's global `safe.directory = *` promotes the repository back to full trust (`gix` `open/repository.rs`, the `check_safe_directories` branch) [Observed: spike A; Documented: S3]. `core.fsmonitor` and hooks never ran under any option: `gix` status implements neither [Observed: spike A; S3 grep finds fsmonitor only as an index-flag name].

**Spawn sites inside the `gix` build** [Documented: S3]. `gix-command` (`prepare.rs`), `gix-filter` driver spawn (`driver/init.rs:75`), `gix-diff` textconv and external diff (`blob/pipeline.rs:535`, `blob/platform.rs:580`), and `gix-path` (`git --exec-path` in `env/mod.rs:173`; the installation-config lookup). With isolated permissions (`git_binary: false`, no system/global/env config) and reduced trust, none is reachable from status: drivers come only from trusted sections (`filter.rs` `extract_drivers`, `config/cache/access.rs` `diff_drivers`), and installation config is never loaded. `gix-transport`, `gix-protocol` and `gix-packetline` compile, but with no client feature (`gix-transport` default features are empty) [Observed: `cargo tree -e features`].

### 0.5 Spike B — `notify` events (macOS FSEvents)

**Setup.** One process watched a fresh library recursively with the raw `notify::recommended_watcher` and three `notify_debouncer_full::new_debouncer` instances at 50, 150 and 500 ms (tick rate default), all printing epoch-ms timestamps. Each scenario got a fresh library (`a.sql`, `b.sql`, `reports/c.sql`, `reports/r.md`), a watcher that printed READY after 700 ms, the operation, 2.5 s of quiet, then a stop file. `notify` 8.2.2 / debouncer 0.7.0, default features (FSEvents). Editor patterns are **simulated** with scripted file operations unless marked real.

| Scenario | How | Raw events | Raw spread (first→last) | Debounced at 150 ms: events / ms after the operation ended | Paths in the 150 ms output |
|---|---|---|---|---|---|
| VS Code in-place write | simulated: open with truncate, write, close | 4 | 0 ms | 1 / 164 | `a.sql` (kind `Create(File)`) |
| vim `backupcopy=auto` | simulated: create+delete `4913`, rename `a.sql`→`a.sql~`, write new `a.sql`, delete `a.sql~` | 12 | 0 ms | 5 / 162 | `4913`, `a.sql`, `a.sql~` |
| vim | **real** `vim -Es -u NONE -N` with `backupcopy=auto writebackup nobackup`, append a line, `:wq` | 23 | 13 ms | 6 / 171 | `.a.sql.swx`, `.a.sql.swp`, `a.sql` |
| JetBrains safe write (brief's form) | simulated: write `a.sql.tmp`, rename over `a.sql` | 9 | 0 ms | 6 / 157 | `a.sql`, `a.sql.tmp` |
| JetBrains safe write (`___jb_tmp___`/`___jb_old___` form) | simulated [Inferred: commonly reported naming; S18 confirms only "backup before saving … safe write"] | 11 | 0 ms | 6 / 124 | `a.sql`, `a.sql___jb_tmp___`, `a.sql___jb_old___` |
| `sed -i ''` | **real** BSD sed (writes `.!<pid>!a.sql`, renames it over) | 10 | 0 ms | 6 / 146 | `.!<pid>!a.sql`, `a.sql` |
| Hacivat's own save (temp, `fsync`, rename) | simulated | 9 | 0 ms | 6 / 142 | `a.sql`, `a.sql.hv-tmp` |
| two saves 100 ms apart | simulated in-place writes | 8 | 103 ms | 1 / 35 (50 ms: 2 events) | `a.sql` |
| `git checkout` of 2,000 files | **real** git, branch switch rewriting `many/f0000..f1999.sql` | 8,519 | 354 ms | 2,108 / 592 | 546 distinct paths incl. `.git/**` |
| create 1,500 files | simulated burst | 4,558 | 172 ms | 1,502 / 139 | 1,500 |
| `mv reports archive` | **real** | 4 | 0 ms | 1 / 174 | `archive` only (`Create(Folder)`) |
| `rm -rf reports` | **real** | 11 | 0 ms | 1 / 162 | `reports` only (`Modify(Metadata(Extended))`) |
| `ln -s a.sql link.sql`, `ln -s /private/tmp outside` | **real** | 4 | 0 ms | 2 / 318 | `link.sql`, `outside` (`Create(Other)`) |
| real VS Code save | not run: no GUI automation | — | — | — | [Unknown] U-08 |

Further observations [Observed: spike B]:

1. **Event kinds are not reliable on macOS.** An in-place write to a file created 6 s before the watch started is reported as `Create(File)` + `Modify(Metadata)` + `Modify(Data)`, raw and debounced; FSEvents item flags accumulate. A folder delete reaches the 150 ms output only as `Modify(Metadata(Extended))` on the folder; a folder move only as `Create(Folder)` on the new name. The debouncer did **not** stitch any save into a single `Rename` event.
2. **Batching.** `notify` 8.2.2 creates the stream with latency 0.0 and `kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer` [Documented: S4 `fsevent.rs:290-291`], so one save arrives as one batch (raw spread 0–13 ms for every editor pattern).
3. **No `MustScanSubDirs`** (notify's `Flag::Rescan`, S4 `fsevent.rs:116`) appeared, not even for 8,519 events.
4. **Watch cost.** The watcher process showed the same `lsof` count (8 lines) for a 10-folder and a 5,000-folder tree; RSS rose from 7.3 to 14.2 MiB with four watchers (the debouncers' file-id caches) [Observed]. FSEvents needs no per-folder handle [Inferred from the constant count].
5. **Symlinks.** A recursive watch on a library with a symlinked directory pointing outside it reported nothing for writes inside the target, through the link or directly [Observed]. But `notify::Config::default()` has `follow_symlinks: true`, used by the inotify and poll backends [Documented: S4 `config.rs:122`, `inotify.rs:609`, `poll.rs:145`].
6. **Poll defaults.** `poll_interval` 30 s, `compare_contents: false` [Documented: S4 `config.rs:118-123`].

### 0.6 Corrections to the plan (rulings that change)

| # | Ruling | Correction | Evidence |
|---|---|---|---|
| C1 | R-M4b-10 says "opened with isolated options … and `Trust::Reduced`" | Confirmed, and **both** are load-bearing; Task 7 must also assert `repo.git_dir_trust() == Trust::Reduced` after open and answer `Unavailable` otherwise, because a global `safe.directory=*` promotes trust when the options are not isolated | §0.4 negative controls |
| C2 | R-M4b-10 "any path whose attributes name a `filter` … is `Unknown`" | Confirmed as **Hacivat's** job: under reduced trust `gix` ignores the driver and hashes the raw bytes, so such a path comes back `Modified` (or clean), never "unknown"; Task 7 queries the `filter` attribute itself (`attributes_only` + `selected_attribute_matches(["filter"])`, shown working in the spike) | §0.4 hostile output |
| C3 | R-M4b-16 / Task 9 interface: events are classified `Changed/Created/Removed` | On macOS the debounced kind is not trustworthy (§0.5 obs. 1). A debounced event is a **dirty-path hint**: the watcher stats the path (`lstat`), compares the `DiskStamp`, and rescans a directory hint's subtree. Kinds are kept only for logging | §0.5 |
| C4 | R-M4b-16 rename rule | The debouncer emits no `Rename` for atomic saves, so "same path, new content" is the normal save and needs no rename logic. A true rename (`mv a.sql b.sql`) arrives as hints for both paths; the content-hash pairing of R-M4b-16 applies to those hints after reconciliation | §0.5 |
| C5 | Task 9 `Watcher` config | Set `notify::Config::default().with_follow_symlinks(false)` (inotify and poll follow by default) and `with_poll_interval(2 s)` for `PollEventSource` | §0.5 obs. 5–6 |
| C6 | Debounce 150 ms | **Kept** (§HV-SCRIPT-08 Decision), plus a settle rule: content is read only after the stamp is unchanged for one more window, so an in-place write is never read half-written | §0.5 |
| C7 | Plan Tech Stack pins `notify` "latest" | Pin **`notify` 8.2.2 + `notify-debouncer-full` 0.7.0**, not 9.0.0/0.8.0: 9.0.0 was published on 2026-11-21 (the day of this spike) with many breaking changes, and it adds `xxhash-rust` 0.8.21 (**BSL-1.0**, not on `deny.toml`'s allow-list) as a non-optional dependency of its poll backend | S5; `notify-9.0.0/Cargo.toml:125` |
| C8 | `trash` (Q11) latest | Pin **5.3.0** (2026-11-05), not 5.4.0 (2026-11-17, four days old); both MIT | S5 |
| C9 | R-M4b-17 "the history redaction scanner's spans" | The redactor masks **every** quoted string after a secret keyword in its statement and arms on `_`-separated parts (`password_hash`), by design for history. Used as the warning's detector it would warn on `WHERE password_hash = 'x'`, which Review Focus 6 says must not warn. The warning keeps only the **value-position** spans (HV-SCRIPT-10 step 2). With R-M4b-28 the app passes `secret_spans(text)` into `scan_secrets`, so `SecretSpanKind` must tell a value position (`Value`) from another string of an armed statement (`Armed`); the redactor's mask kinds (`Word`/`String`/`Mixed`/`Meta`) do not, so Task 6's named exception in `redact.rs` adds it | `redact.rs:16-33` doc comment |
| C10 | ADR "0008 Script workspace" (draft) | `docs/decisions/0008-metadata-and-introspection.md` already exists (M4). This milestone's ADR is **0009**; the pre-flight ruled the same (R-M4b-33) | repository |
| C11 | R-M4b-12/13 listing | sed's temp file `.!<pid>!a.sql` ends in `.sql`. The scanner also skips **files** whose name starts with `.` (not only folders), so editor temps never list even when a scan lands mid-save | §0.5 `sed -i` |

### 0.7 [Unknown] → test backlog

| ID | Unknown | Settled by (task, test) |
|---|---|---|
| U-01 | `gix` parity on Linux and Windows (case-insensitive NTFS, `core.autocrlf` default on Windows, symlinks off) | T7 `status_matches_git_cli` on the three `build-test` runners |
| U-02 | `gix` never spawns on Linux/Windows | T7 `hostile_repo_runs_no_program` (markers) on three OSes; Linux run under `strace -f -e trace=execve` (second review); macOS under `sandbox-exec` with `(deny process-fork (with send-signal SIGKILL))` as in spike A |
| U-03 | Foreign-owned repository (another uid): `gix` with forced reduced trust does not report it, so Hacivat must check ownership itself | T7 `foreign_owner_repo_is_untrusted` (chown as root in CI, else the `FakeOwner` hook) |
| U-04 | Submodule roots and nested repositories inside a library | T7 `submodule_root_is_unknown` (new fixture) |
| U-05 | `gix` status time on 10,000 files without the `parallel` feature | T7 `status_of_10k_files_under_1s` (`#[ignore = "perf"]`); if it fails, Task 1a adds `parallel` by ruling |
| U-06 | Linux inotify: event shapes for the same save table; watches used by a 5,000-folder tree; `IN_Q_OVERFLOW` mapping to `Flag::Rescan` | T9 `watch_real` on the ubuntu runner, plus a CI probe printing the inotify watch count for a generated 5,000-folder tree |
| U-07 | Windows `ReadDirectoryChangesW`: event shapes, overflow (`ERROR_NOTIFY_ENUM_DIR`), short (8.3) names in events, junctions | T9 `watch_real` on the windows runner; `short_name_event_is_reconciled` |
| U-08 | A real VS Code save (default settings) — VS Code documents no setting for how a save writes the file (S17 search found only `files.autoSave`) [Documented: absence in S17] | T9 `external_in_place_write_is_seen` and `external_temp_then_rename_write_is_seen` cover both shapes; the M4b gate's PO demo (E2) saves from VS Code by hand |
| U-09 | SMB/NFS/AFP behaviour and their detection by `statfs` / drive type | T9 `network_fs_chooses_polling` (unit, injected `FsKind`); a manual check on an SMB share at the gate if one exists |
| U-10 | Dataless files (`SF_DATALESS`, `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`): detection and that listing never downloads | T4 `dataless_flag_is_read_from_metadata_only` (fake metadata); never run against a real iCloud folder |
| U-11 | `notify` 9.x readiness (re-evaluate once two weeks old; needs the `xxhash-rust` BSL-1.0 ruling) | controller ruling after M4b; no test |
| U-12 | Whether `gix` reports a path whose attribute names a filter that **is** configured globally (we never load global config) | T7 `filtered_path_is_unknown` covers repository-level attributes; global attributes are never read (spike A) |

## 1. Specifications

### HV-SCRIPT-01: Library registry and `.hacivat/workspace.toml`

- **Problem:** Remember which folders are script libraries and what logical database each binds, without writing anything sensitive into the folder, and read a folder-level manifest that teammates share through git or a shared drive, which is untrusted input.
- **Inputs / outputs:** In: a folder the user picked; `<config>/libraries.toml`; `<library>/.hacivat/workspace.toml` if present. Out: `LibraryRoot { id, path, canonical }`, `Manifest { format, database, aliases }`, diagnostics.
- **Algorithm:**
  1. **Registry file** `<config>/libraries.toml` (R-M4b-2): `version = 1`; `[[library]] id = "<uuid v4>", path = "<absolute, as chosen>", added_at = "<RFC 3339 UTC>"`; `[[mapping]] profile = "<uuid>", database = "<exact name>", library = "<uuid>"`. One writer (`LibraryRegistry`), written through `fsio::atomic_write` (temp + `fsync` + rename, file 0600, folder 0700; spec §5.6).
  2. **Attach:** canonicalize the chosen path (`realpath`); refuse a path inside another library or containing one (nesting would project files twice), the home directory itself, a filesystem root, and the app's own config/data directories. At most 32 libraries (R-M4b-14).
  3. **Manifest read** (untrusted): `lstat` `.hacivat` and `.hacivat/workspace.toml`; a symlink at either → ignored with a diagnostic. Size > 64 KiB → ignored. Not UTF-8 → ignored. Parse with `toml` (no `include`-like features exist in TOML). Keys: `format` (integer), `database` (string, 1–128 bytes, no C0/C1 control characters, no bidi controls), `aliases` (array of ≤ 32 such strings). Unknown keys → ignored with a diagnostic naming the key (escaped). `format` > 1 → bind by folder name, diagnostic "written by a newer Hacivat". Missing `database` → folder name.
  4. **Manifest write** (R-M4b-8): only on attach after the user saw the exact text and path, only if the file does not exist (`O_CREAT|O_EXCL` on a temp, then `link`/`rename` without replacing). Content is exactly `format = 1\ndatabase = "<name>"\naliases = [...]\n`, TOML-escaped. Ordinary permissions (umask, typically 0644; `.hacivat/` created with defaults), **not** `fsio::atomic_write`: the file is meant to be committed and read by teammates (R-M4b-42). An alias added later goes through `toml_edit`, preserving comments and formatting. No other field can be written: the writer's type has exactly `format`, `database`, `aliases` (L14 credential rule).
  5. **Missing folder** (unmounted drive, deleted): the library stays registered with `LibraryState::Unavailable(Missing)`; nothing is projected; Settings shows it.
- **Data structures:** `LibraryId(Uuid)`, `LibraryRoot`, `Manifest`, `LibraryDiag { kind, rel: Option<RelPath> }` (display escapes bidi/invisible characters as `<U+XXXX>`).
- **Complexity:** registry ≤ 32 rows; manifest ≤ 64 KiB, parsed once per change.
- **Edge cases:** manifest is a FIFO or a device (`lstat` not a regular file → ignored); manifest with a BOM (strip a UTF-8 BOM before parsing) [Inferred]; `database = ""`; `database` with `/`, `..` or NUL (a name, never a path: accepted as text if it passes the character rules, never joined to a path); duplicate aliases (deduplicated after NFC); alias equal to `database`; two libraries with the same path spelled differently (canonical equality refuses the second); a case-only different path on a case-insensitive volume (canonical path compares equal) [Inferred: `realpath` returns the stored case on APFS]; a library on a removable drive that later mounts elsewhere (missing until re-attached).
- **Test cases:** `manifest_table` — `format=1, database="orders"` → binds `orders`; `format=2` → folder name + diagnostic; 64 KiB + 1 → ignored; invalid TOML → ignored; `database` with U+202E → refused with diagnostic; unknown key `run = "x"` → ignored + diagnostic. `manifest_symlink_is_ignored`. `attach_writes_manifest_only_when_absent` (existing file bytes unchanged). `alias_append_keeps_comments`. `registry_is_0600_and_atomic`. `nested_library_is_refused`. `writer_type_has_exactly_three_fields` (a compile-time/serde round-trip test listing the fields).
- **Prior art:** DataGrip "attached directories" are IDE-level, not per folder [Documented: `datagrip/10` §3.4–3.5]. VS Code keeps trust decisions outside the folder and opens new folders restricted [Documented: S17].
- **Decision:** as R-M4b-2, -8, -21. Manifest is data only; it cannot name a path, program, connection or safety setting in any minor format.

### HV-SCRIPT-02: Logical-database binding

- **Problem:** Decide which libraries appear under a database node `(profile, database)`.
- **Inputs / outputs:** In: `DbNode { profile, engine, database }`, library snapshots, local mappings. Out: `Vec<LibraryId>` in stable order.
- **Algorithm:**
  1. **Database level per engine** (R-M4b-3, Q13): PostgreSQL → the database; MySQL/MariaDB → the schema; SQLite → the database name is always `"main"` and the logical name is the file stem of the main database file; attached databases get no Scripts node in M4b (R-M4b-32).
  2. If a local mapping `(P, D) → L` exists for this node → `[L]` plus nothing automatic for that node (R-M4b-7: a mapping beats automatic binding for that node only).
  3. Else every library whose `database` or an alias **matches** `D`.
  4. **Match:** NFC-normalize both, then ASCII case-insensitive comparison; non-ASCII letters must be equal after NFC (never locale casing: `İ` ≠ `I`, `ı` ≠ `i`) [Documented: `alg/26` I18N invariants; `datagrip/11` ALG-SET-01 edge cases].
  5. Order: library display name (NFC, I18N-09 key), then `LibraryId`.
- **Edge cases:** PostgreSQL database names are case-sensitive on the server (`Orders` ≠ `orders`) but match the same library here by design [Inferred: the binding is a convenience, the tab runs on the exact node]; a mapping to a library that was removed (row dropped on load with a diagnostic); a profile deleted (its mapping rows dropped, R-M4b-2); two libraries binding the same name (both shown, one folder row each); Turkish `İNVENTORY` vs `inventory` (no match: `İ` is not ASCII).
- **Test cases:** `binding_table` — `orders`/`ORDERS` match; `orders`/`orders_p` no match; `İnventory`/`inventory` no match; NFD `café` vs NFC `café` match; mapping beats automatic; mapping only for its node. `deleted_profile_drops_mappings`.
- **Decision:** R-M4b-7 unchanged.

### HV-SCRIPT-03: Library scan and path safety

- **Problem:** List a folder tree that the user does not control, without ever touching a path outside the library, following a link out, hanging on a loop, or downloading cloud placeholders.
- **Inputs / outputs:** In: `LibraryRoot`, limits. Out: sorted `ScriptEntry`s (folders first), diagnostics.
- **Algorithm:**
  1. Open the root by its canonical path; walk with `lstat` only (`symlink_metadata`), never `stat`, never `read`.
  2. **Skip** (R-M4b-13, corrected C11): any folder **or file** whose name starts with `.`; `node_modules`. `.gitignore` is not consulted.
  3. **List** files whose extension is `.sql` or `.md` (ASCII case-insensitive), and folders that contain one at any depth (R-M4b-12).
  4. **Names:** reject components that are empty, `.`, `..`, contain `/`, `\`, NUL, or (on Windows) `:`, or are Windows device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, with any extension), or end with `.` or space; > 255 bytes → skipped with a diagnostic. Display is NFC with bidi/invisible characters escaped (R-M4b-23); identity is the OS bytes.
  5. **Symlinks** (Q15): a symlink to a file is resolved once with `realpath`; inside the canonical root → `InsideLink` (listed, opens the target); outside → `OutsideLink` (listed disabled, "points outside the library", never opened); dangling → `OutsideLink`. A symlink to a directory is **never followed and not listed**: whether it holds scripts cannot be known without following it [Inferred: following directory links is how loops and escapes happen; FSEvents does not follow them either, §0.5 obs. 5].
  6. **Dataless** (R-M4b-15): `st_flags & SF_DATALESS` on macOS, `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` on Windows → `dataless = true` from metadata only [Unknown U-10].
  7. **Limits** (R-M4b-14): 20,000 listed files, depth 32; hitting one stops that library's scan, keeps what was found, raises a diagnostic.
  8. **Every open later** (read, save, rename, trash) re-validates: join the `RelPath` to the canonical root, open each component with `O_NOFOLLOW` (Unix) or check reparse points (Windows), and confirm the final `realpath` is inside the root (TOCTOU: a directory replaced by a link between scan and save is refused).
- **Data structures:** `RelPath(Arc<str>)` ('/'-separated, validated components); `ScriptEntry`; `LinkState { Plain, InsideLink, OutsideLink, Loop }`.
- **Complexity:** O(entries) `lstat` calls; 20,000 files ≤ 300 ms on a local SSD [Unknown → T4 `#[ignore = "perf"] scan_20k_files_under_300ms`].
- **Edge cases:** symlink loop `a → b → a` (`Loop`, disabled); hard link to a file outside (indistinguishable by name; reading it is reading a file the user placed in the folder; accepted) [Inferred]; NFD names from macOS; a file named `x.SQL`; a folder named `x.sql`; Windows junction (treated as a directory link: not followed) [Unknown U-07]; permissions denied on a subfolder (skipped with a diagnostic); a library root that becomes a symlink (canonical path re-checked on every rescan; mismatch → `Unavailable(RootChanged)`).
- **Test cases:** `pathsafe_component_table` (each rejected name above, plus accepted `ünïcode.sql`, `rapor İ.sql`); `symlink_outside_is_listed_disabled_and_never_opened`; `symlink_loop_is_disabled`; `dir_symlink_is_not_followed`; `dot_files_are_not_listed` (`.!123!a.sql`, `.a.sql.swp`); `scan_limits_stop_with_a_notice`; `save_refuses_a_path_that_became_a_link`.
- **Decision:** R-M4b-12/13/14/15 with correction C11.

### HV-SCRIPT-04: Environment scoping `@<environment>/`

- **Problem:** Show a file only under connections whose environment tag the file's `@` folders allow; fail closed.
- **Inputs / outputs:** In: the file's `RelPath`, the connection's `Option<EnvTag>`. Out: `EnvScope { Any, Only(EnvSet), Hidden(ScopeProblem) }`, `visible: bool`.
- **Algorithm (R-M4b-6):**
  1. For each folder component starting with `@`: if the rest equals `local`, `dev`, `test`, `staging` or `production` under ASCII case-insensitive comparison (`EnvTag::id`, `hacivat-proto/src/lib.rs:227`) → that tag; otherwise → `Hidden(Unknown(name))` with the "did you mean" diagnostic (edit distance ≤ 3 or a known prefix such as `prod`).
  2. Allowed set = intersection over components; empty intersection → `Hidden(Contradiction)`.
  3. `visible(entry, env)`: `Any` → true; `Only(set)` → `env ∈ set`; `Hidden` → false; `env = None` sees only `Any`.
  4. Folder components only; a file named `@production.sql` is not a scope [Inferred: the rule names folders].
- **Edge cases:** `@Production`, `@PRODUCTION` (scope); `@prod`, `@productİon` (U+0130; not ASCII-equal → hidden), `@production ` (trailing space → hidden), `@production/@staging/x.sql` (hidden), `@` alone (hidden), NFD-normalized names (ASCII tag names are unaffected by NFC/NFD), a symlinked `@staging` folder pointing at `@production` (directory links are never followed, HV-SCRIPT-03 step 5).
- **Test cases:** `envscope_table` (18 rows as in the plan's Review Focus 2); `untagged_profile_sees_only_unscoped`; `symlinked_env_folder_is_not_followed`; `file_named_at_production_is_not_a_scope`.
- **Decision:** R-M4b-6 unchanged.

### HV-SCRIPT-05: Projection into the explorer

- **Problem:** Show one library under every database node it binds, as a "Scripts" node above the schemas, with a shared badge and a file count, with stable identity.
- **Inputs / outputs:** In: `WorkspaceSnapshot`, the explorer's database nodes (C-M4-3/4), git status. Out: contributed nodes (C-M4-2).
- **Algorithm:**
  1. For each database node, `binding::resolve` (HV-SCRIPT-02); none → no Scripts node (R-M4b-19).
  2. One library → its folders and files directly; several → one folder row per library.
  3. Filter entries by `envscope::visible` for the node's profile environment **now** (re-evaluated on profile change).
  4. Count = visible files (tabular digits); folders with no visible file are not shown.
  5. SHARED badge when the library is projected under ≥ 2 connections; tooltip lists the others.
  6. Keys (R-M4b-26): `TreeKey(xxh3_128("hv-scripts\0" ‖ profile uuid ‖ database ‖ library uuid ‖ entry kind ‖ rel path))`, every part length-prefixed like M4's object keys, so the same file under `orders-prod` and `orders-staging` has two keys (shared expansion or selection would blur which connection a row belongs to).
  7. Git badges per file (HV-SCRIPT-11), folder dot when any descendant has a badge; `Unknown` shows its own glyph; never red.
- **Edge cases:** Scripts node under a greyed PostgreSQL database (not shown, R-M4b-25); library unavailable (node shows the state row, no files); 20,000 files (lazy children per folder).
- **Test cases:** `same_library_under_two_connections` (gate E1); `production_file_hidden_on_staging` (gate E3); `scripts_node_absent_without_binding`; `keys_never_collide_with_object_keys`, `same_file_has_one_key_per_connection`.
- **Decision:** as the plan (R-M4b-11, -19, -25).

### HV-SCRIPT-06: Context-from-location opening, script tabs, restore

- **Problem:** A script runs on the connection under which it was opened, for the tab's whole life.
- **Inputs / outputs:** In: an open request `(library, rel, profile, database)`. Out: a script tab bound to `(ProfileId, DatabaseName)`.
- **Algorithm:**
  1. The binding is fixed at open (R-M4b-4); there is no rebind; "Open under another connection…" opens a new tab through the chooser (HV-SCRIPT-09).
  2. The capsule, hairline and status segment read the profile's **current** environment on every frame, exactly as consoles do.
  3. MySQL/MariaDB node other than the profile default → the tab opens **its own** console session (never the profile's default session) and switches it with `Request::SwitchSchema` (R-M4b-25). Before **every** Run the tab compares its session's current database with its binding: MySQL/MariaDB re-switch once and continue only if the re-probe reads the bound database; PostgreSQL and SQLite refuse Run with a notice naming both databases (R-M4b-27: a reconnect silently drops `USE`).
  3a. An unbound script tab is never bound by a later connect (R-M4b-30); it binds only through "Open under another connection…".
  4. If the file stops being visible under the binding (moved into `@production/` while open under staging) → the tab keeps its binding and shows "No longer listed under this connection"; Run still runs only on the binding.
  5. Restore (R-M4b-22, -29): the tab is saved as `TabKind::Script { library, rel_path, profile, database }` in `workspace.json`'s pane tree (version 2); missing library/file → notice + recovery copy if any; deleted profile → unbound tab that runs nothing.
- **Edge cases:** as Review Focus 1 of the plan.
- **Test cases:** `tab_runs_on_the_connection_it_was_opened_under`, `two_tabs_same_file_two_connections`, `restore_keeps_binding_and_reads_current_env`, `moved_into_production_folder_keeps_binding_and_notices`, `deleted_profile_restores_unbound`, `reconnect_to_default_database_is_switched_back_before_run`, `unbound_script_tab_is_not_bound_by_connect`.
- **Decision:** R-M4b-4, -22, -25, -27, -29, -30.

### HV-SCRIPT-07: File I/O — decode, encodings, atomic save, recovery copies

- **Problem:** Open any script without corrupting it, save it back byte-faithfully, and never lose unsaved text, without writing anything but the script into the library.
- **Inputs / outputs:** In: bytes on disk. Out: `Decoded { text, meta: FileMeta { encoding, bom, line_ending }, stamp, tier }`; on save, `DiskStamp`.
- **Algorithm:**
  1. **Decode** per `alg/25` UI-EDIT-02: BOM sniff (UTF-8, UTF-16LE/BE) → that encoding; else strict UTF-8; else a remembered per-file choice; else the "unknown encoding" prompt (choose, or open read-only). Legacy encodings through `encoding_rs` (Windows-1254 for Turkish, Windows-1252, ISO-8859-9, Shift_JIS, GB18030, …). Dominant line ending recorded (CRLF/LF/CR, ties → LF) [Documented: `alg/25:252-299`].
  2. **Stamp** `DiskStamp { len, mtime, hash: xxh3_128(bytes) }` (existing `twox-hash`).
  3. **Save:** encode with the same encoding, BOM and dominant line ending; if `expect` is given and the file's current stamp differs → `SaveError::ChangedOnDisk` (the UI asks, HV-SCRIPT-08). Write a temp file **in the same directory** named `.<name>.hv-<random>.tmp` (starts with `.`, never listed, C11; not `.sql`), `fsync`, copy the original's permission bits, `rename` over the target, `fsync` the directory. If the target is an `InsideLink`, save to the link's resolved target so the link survives. Hard links: renaming breaks them; detect `st_nlink > 1` and write in place instead (truncate + write + `fsync`), after the same stamp check [Inferred: rename-based atomic save cannot preserve a hard link].
  4. Record the written stamp in the watcher's echo table **before** the rename, and the temp path as an expected transient (§0.5 "Hacivat's own save" shows events for both).
  5. **Recovery copies** (R-M4b-5): after 5 s idle, unsaved text → `<data>/script-recovery/<library-uuid>/<xxh3(rel)>.sql` (0600, directory 0700), plus a sidecar with the library id, rel path, encoding and base stamp; deleted on save or discard; offered on next launch. Never inside the library.
- **Edge cases:** read-only file (save → error, tab stays dirty); full disk (temp write fails → original intact, tab dirty); file replaced by a directory; target became a link between open and save (HV-SCRIPT-03 step 8); 0755 mode kept; Windows: `rename` over a file open in another program may fail with sharing violation → retry 3× with backoff, then error [Inferred]; NFD name preserved on save (the path bytes from the scan are reused).
- **Test cases:** `fileio_round_trip_table` (UTF-8, UTF-8 BOM, UTF-16LE BOM, UTF-16BE BOM, Windows-1254 with `ğüşıöçİ`, mixed CRLF/LF → dominant kept, CR-only); `save_keeps_mode_and_link_target`; `save_keeps_hard_link`; `failed_save_leaves_original`; `save_detects_change_since_open`; `recovery_copy_is_outside_library_and_0600`; `temp_file_is_never_listed`.
- **Decision:** as R-M4b-5 plus the hard-link rule and the temp-name rule above.

### HV-SCRIPT-08: Watcher and external changes

- **Problem:** Show edits made by other programs promptly, without reading half-written files, without reloading over unsaved edits, without reacting to Hacivat's own saves, and without unbounded work on bursts.
- **Inputs / outputs:** In: OS events (`notify`), or polling. Out: `WatchEvent { Changed(rel), Created(rel), Removed(rel), Rescan, BackendLost(reason) }` after reconciliation and echo filtering; then `LibraryEvent`s.
- **Algorithm:**
  1. **Backend choice** (R-M4b-14): `notify::recommended_watcher` with `Config::default().with_follow_symlinks(false)` (C5). Polling (`PollWatcher`, 2 s, `compare_contents: false`) when: the library is on a network file system (`statfs` type / drive type) [Documented: S6, inotify "does not catch remote events that occur on network filesystems … must fall back to polling"]; on Linux the directory count exceeds 50 % of the free inotify watch budget (`/proc/sys/fs/inotify/max_user_watches`, per real user ID, S6; inotify is not recursive, so one watch per directory, S6); or the OS watcher fails to start or reports a fatal error (`BackendLost`).
  2. **Debounce** with `notify-debouncer-full` at **150 ms** (decision below). Each output event is a **dirty-path hint** (C3): its kind is not trusted.
  3. **Overflow → rescan:** a debounced batch of more than 1,000 hints, a `Flag::Rescan` event (FSEvents `MustScanSubDirs`, S7: "you must recursively rescan the path"; dropped events also set it, S7), inotify `IN_Q_OVERFLOW` (S6), or Windows `ERROR_NOTIFY_ENUM_DIR` / an overflowed buffer (S8: "the entire contents of the buffer are discarded") → `Rescan` of the library (or of the flagged subtree).
  4. **Reconcile each hint:** drop paths outside the library or skipped by HV-SCRIPT-03 (dot files, other extensions) — but a hint on a **directory** (or on a path that is now a directory, or absent and previously a directory) rescans that subtree, because folder moves and deletes surface only as one hint on the folder (§0.5). For a file: `lstat`; absent and known → `Removed`; present and unknown → `Created`; present and known → compare `(len, mtime)` with the last stamp; different → `Changed` candidate.
  5. **Settle (C6):** a `Changed`/`Created` candidate is read only when its `(len, mtime)` is the same at two checks one debounce window apart; until then it is re-queued (at most 10 times, then read anyway and flagged). This keeps an in-place writer's half-written state out of a clean tab [Inferred: in-place saves truncate then write; §0.5 shows the debouncer may split saves that span > 100 ms].
  6. **Echo:** the save path records `(rel, DiskStamp)` before its rename (HV-SCRIPT-07 step 4); a reconciled file whose full stamp (len, mtime, xxh3-128) equals a recorded one within 5 s is Hacivat's own write and is dropped. The hash is computed only when `(len, mtime)` match, so foreign edits pay one `lstat` and no read.
  7. **Renames** (C4): after reconciliation, a `Removed(a)` and a `Created(b)` in the same batch whose content hash equals `a`'s last known hash → `FileRenamed { a → b }`; otherwise they stay a delete and a create (R-M4b-16).
  8. **Tabs** (R-M4b-16): clean tab → in-place reload (one replace from the common prefix/suffix, caret and selection mapped, undo cleared, waits for IME composition to end); dirty tab → "Changed on disk" bar; deleted → "Deleted on disk" bar.
  9. Bounded queues: the hint channel holds at most 10,000 entries; overflow → one `Rescan` [Inferred: bounded memory under a burst].
- **Data structures:** `Debouncer` (wraps the crate), `EchoTable { by_rel: HashMap<RelPath, (DiskStamp, Instant)> }`, `SettleQueue`, `Backend { Os, Poll(PollReason) }`.
- **Complexity / performance:** idle: no CPU (FSEvents/inotify block) [Inferred → T9 records idle CPU]; per save: one batch of 4–23 raw events on macOS (§0.5); latency from save to reload ≈ 150 ms debounce + one settle window ≈ 300–350 ms.
- **Edge cases:** git checkout of thousands of files (one rescan, §0.5: 8,519 raw events in 354 ms); vim swap files and `4913` probes (skipped names); `.git/**` events when the library is a repository (`.git` is a dot folder: skipped, but a hint under `.git/` triggers a git-status refresh, HV-SCRIPT-11); a library root deleted or renamed (FSEvents root-changed, S7; → `Unavailable(Missing)`); the library on a network share (polling); clock skew making `mtime` go backwards (stamp compares `len` and `mtime` for inequality, not order); a symlinked directory pointing outside (not watched through, §0.5 obs. 5 and C5).
- **Test cases (from §0.5, copied as named tests):**
  - `atomic_rename_save_is_one_change` — hints `a.sql.tmp`, `a.sql` within the window → after reconcile `[Changed("a.sql")]` (the temp is skipped by name or absent).
  - `vim_backup_rename_save_is_one_change` — hints `4913`, `a.sql~`, `a.sql` → `[Changed("a.sql")]`.
  - `sed_temp_is_never_listed` — hint `.!123!a.sql` → nothing.
  - `inplace_write_reported_as_create_is_a_change` — a hint whose kind is `Create(File)` for a known file → `Changed`.
  - `folder_delete_hint_on_parent_rescans` — single hint `reports` (kind `Modify(Metadata)`) after `rm -rf reports` → `Removed` for `reports/c.sql` and `reports/r.md`.
  - `folder_move_hint_rescans_both` — single hint `archive` after `mv reports archive` → entries move (`FileRenamed` pairs by hash).
  - `burst_becomes_one_rescan` — 1,001 hints in one window → `[Rescan]`.
  - `own_write_is_an_echo`, `echo_expires_after_5s`.
  - `half_written_file_waits_to_settle` — stamp changes between two checks → no read until stable.
  - `follow_symlinks_is_off` — the watcher's config has `follow_symlinks == false`.
  - `small_inotify_budget_chooses_polling` (Linux), `network_fs_chooses_polling`, `backend_lost_switches_to_polling`.
  - `watch_real`: `external_in_place_write_is_seen`, `external_temp_then_rename_write_is_seen`, `folder_delete_is_seen`, `git_checkout_2000_is_one_rescan` (condition waits ≤ 10 s).
- **Prior art:** FSEvents is advisory and asks for periodic sweeps (S7); inotify rename-cookie matching "is thus inherently racy" (S6); `ReadDirectoryChangesW` overflow discards the buffer (S8).
- **Decision:**
  - **Debounce 150 ms** [Observed: §0.5]. Every editor save on macOS arrives as one raw batch spread over 0–13 ms, so 150 ms has more than 10× margin. Measured end-to-end delivery at 150 ms is 124–174 ms after the save ends. 50 ms splits two saves 100 ms apart into two events (harmless but twice the work). 500 ms adds about 0.5 s of visible lag for nothing. Linux and Windows spreads are [Unknown U-06/U-07]; if T9's `watch_real` on those runners shows a single save spread over more than 100 ms, the window is raised there by ruling.
  - Kinds are hints, reconciliation decides (C3). Settle before read (C6).
  - `notify` 8.2.2 + `notify-debouncer-full` 0.7.0 (C7).

### HV-SCRIPT-09: "Go to script" and the connection chooser

- **Problem:** Find any script by name across libraries and open it under a connection the user picks, never defaulting into production.
- **Inputs / outputs:** In: query text; the snapshot; connections. Out: ranked results; then an open request (HV-SCRIPT-06).
- **Algorithm:**
  1. **Index** names only (never contents; R-M4b-15): file name, library-relative folder, library name, each with the I18N-09 search key (`alg/26:384`).
  2. **Rank:** exact name > prefix > word-prefix > substring > subsequence on the search key; ties: shorter path, then recently opened, then path.
  3. **Candidates** for a picked file: the places the explorer lists plus every profile's configured `(profile, database)` (R-M4b-31), filtered to those that bind the file's library (HV-SCRIPT-02) and for which the file is visible (HV-SCRIPT-04).
  4. **Chooser order and default** (R-M4b-9): focused tab's connection first if a candidate; then local, dev, test, staging, production (reverse of `EnvTag::severity_rank`); untagged; then name. Production is never preselected unless it is the focused connection. One non-production candidate → open directly; production or ≥ 2 → chooser; none → "Open without a connection".
- **Edge cases:** 20,000 × 32 files (index ≤ 640k names; incremental updates from `LibraryEvent`s); a file visible on no connection; Turkish query `İSTANBUL` finds `istanbul.sql` (I18N-09 folding); the focused connection is production (it may be preselected, being the user's current context).
- **Test cases:** `goto_ranking_table`, `chooser_never_preselects_production`, `single_non_production_candidate_opens_directly`, `index_reads_no_contents` (a FakeFs that fails on any read).
- **Decision:** R-M4b-9, -31.

### HV-SCRIPT-10: Secret-literal detection and the save warning

- **Problem:** Warn before a script that contains a password- or token-like literal is saved into (or moved into) a library, without crying wolf and without echoing the value.
- **Inputs / outputs:** In: the text. Out: `Vec<SecretFinding { range, line, kind }>` (never the value).
- **Algorithm:**
  1. Skip any line whose comment contains `hacivat:allow-secret`.
  2. **SQL value positions** (spans from `hacivat_exec::history::secret_spans`, passed in by the app, R-M4b-28; kept only when their kind is `Value`, C9): report only the value right after `PASSWORD [=|(]`, after `IDENTIFIED [WITH <plugin>] BY|AS [PASSWORD]`, after `SET PASSWORD [FOR <user>] =`, `WITH PASSWORD =` (SQL Server `CREATE/ALTER LOGIN`), the string values of `pgp_sym_encrypt`-like key arguments, `PRAGMA key/rekey/hexkey/textkey = '…'`, `ATTACH … KEY '…'`, and connection-string values `password=`/`pwd=` and URI userinfo passwords (`scheme://user:pass@host`; RFC 3986 deprecates `user:password`, S16). **Not** reported: every other string after a secret word (the redactor's history rule), column names (`password_hash`), parameters (`:pw`, `$1`, `?`, `@pw` without a value), placeholders (`'***'`, `'<password>'`, `'${SECRET}'`, `'{{secret}}'`), empty strings, `NULL`. A literal default such as `'changeme'` is still a password and **is** reported.
  3. **Token formats** (vendor facts, our own patterns):
     - AWS access key id: `AKIA` (long-term) or `ASIA` (temporary STS) + 16 upper-case letters/digits, on a word boundary [Documented: prefixes S9; the 20-character length is not stated on that page → Inferred from common practice, tested by fixed fixtures].
     - GitHub: `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_` + ≥ 30 base62 characters; `github_pat_` + ≥ 22 characters of `[A-Za-z0-9_]` [Documented: prefixes S10]. Do not assume a fixed length: `ghs_` installation tokens change format in 2026 (S10).
     - Slack: `xoxb-`, `xoxp-`, `xapp-`, `xwfp-`, `xoxe.xapp-` followed by `-`-separated alphanumeric sections [Documented: S11].
     - Stripe: `sk_live_`, `rk_live_` + ≥ 16 alphanumerics (secret and restricted live keys) [Documented: S12]. `sk_test_`/`rk_test_` are reported as a lower-severity kind; `pk_` keys are publishable and **not** reported ("Only publishable keys are safe to expose", S12).
     - Google API key: `AIza` + 35 of `[A-Za-z0-9_-]` [Inferred: the format is not documented on S13; widely observed shape; a fixed-fixture test only].
     - PEM private key: a line `-----BEGIN <label>-----` where the label ends with `PRIVATE KEY` (covers `PRIVATE KEY`, `ENCRYPTED PRIVATE KEY`, and legacy `RSA/EC/DSA/OPENSSH PRIVATE KEY`) [Documented: S15 §2, §10, §11; legacy labels Inferred].
     - JWT: three base64url segments separated by `.`, the first decoding to JSON starting with `{"` and containing `"alg"`; the third may be empty (unsecured JWT, S14 §6) [Documented: S14 §3, S14 RFC 7515 §2].
  4. No generic entropy rule in v1 (R-M4b-17).
  5. Findings carry the kind and the line; the dialog lists lines and kinds, never values; "Save anyway" remembers the fingerprints `xxh3(kind ‖ value)` in memory for the tab until quit.
- **Edge cases:** `İDENTİFİED BY` (the redactor's fixed casing table accepts `İ`/`ı` as `I`, `redact.rs:24-25`); a secret inside a block comment (reported: comments are shared too); a secret split across lines by string concatenation (not detected) [Inferred: acceptable miss]; a 25 MB file (scan off the UI thread, in chunks with overlap of 512 bytes) [Inferred]; dollar-quoted PostgreSQL strings.
- **Test cases:** `secret_table` (30 rows; the Review Focus 6 cases): reported — `CREATE USER app PASSWORD 's3cret'`, `ALTER ROLE x PASSWORD 'x1'`, `IDENTIFIED BY tiger`, `IDENTIFIED WITH mysql_native_password BY 'pw'`, `SET PASSWORD FOR u = 'pw'`, `postgresql://u:pw@h/db`, `AKIA` + 16 fixture chars, `ghp_` + 36 fixture chars, `xoxb-` fixture, `sk_live_` fixture, PEM `-----BEGIN PRIVATE KEY-----`, a fixture JWT; not reported — `ALTER ROLE x PASSWORD NULL`, `PASSWORD :pw`, `PASSWORD '***'`, `'${secret}'`, `PASSWORD ''`, `SELECT password_hash FROM users WHERE password_hash = 'x'`, `pk_live_` fixture, `-- hacivat:allow-secret` line; `warning_never_echoes_the_value`; `allow_comment_silences_one_line`. Fixture tokens are synthetic and obviously fake (repeated `EXAMPLE`/`0` characters), never real keys.
- **Prior art:** vendor prefixes exist precisely so scanners can find leaked tokens (S10 blog: prefixes "to indicate the token's type").
- **Decision:** R-M4b-17, -28, with correction C9 (value positions only).

### HV-SCRIPT-11: Read-only git status

- **Problem:** Show modified/new/conflicted badges for a library inside a git repository, matching `git status`, with **no** program from the repository, the user's git config or the environment ever running.
- **Inputs / outputs:** In: library root, `GitLimits { timeout: 5 s, max_entries }`. Out: `GitState { NotARepo, Untrusted(reason), Unavailable(err), Ready(GitStatus) }`.
- **Threat facts** [Documented: S1, S2, spike A]: the git CLI's own `status` runs `core.fsmonitor` (a non-boolean value "contains the pathname of the fsmonitor hook command", S2), `filter.<driver>.clean` and `.process` named by `gitattributes` (S1 `gitattributes(5)` "filter"), and the `post-index-change` hook when it writes the index (S1 `githooks(5)`: "invoked when the index is written"; `git-status(1)` BACKGROUND REFRESH: status writes the refreshed index by default); `git diff` runs `diff.external` and textconv. `--no-optional-locks` (`GIT_OPTIONAL_LOCKS=0`) does **not** stop `core.fsmonitor` (spike A). Git trusts repository-local config unless the repository is owned by someone else; `safe.directory` "is only respected in protected configuration", i.e. system, global and command scopes (S1). This is why Q4 chose `gix` over a hardened CLI: the CLI path would rely on knowing every such key.
- **Algorithm:**
  1. **Discover:** walk up from the canonical library root to the first entry named `.git` (directory or gitfile); the user's home directory is the ceiling (R-M4b-10: a `$HOME/.git` dotfiles repository never claims every library under home [Inferred]), and the walk stops where `st_dev` changes (filesystem boundary). Spike A used a plain upward walk with an explicit ceiling; `gix`'s own `discover` is not needed. None → `NotARepo`.
  2. **Ownership** (Unix): `.git` (and the worktree root) owned by another uid → `Untrusted(ForeignOwner)`, no status [Unknown U-03]. `gix` would open it anyway under forced reduced trust, so this check is Hacivat's.
  3. **Open:** `gix::open_opts(worktree, gix::open::Options::isolated().with(gix::sec::Trust::Reduced))`. Then assert `repo.git_dir_trust() == Trust::Reduced`, else `Unavailable(TrustPromoted)` (C1). Never use `discover*` with environment overrides; `GIT_DIR` and friends are ignored under isolated permissions (spike A decoy).
  4. **Status:** `repo.status(progress::Discard)?.untracked_files(UntrackedFiles::Files).index_worktree_submodules(Submodule::Given { ignore: Ignore::All, check_dirty: false })`, `should_interrupt_owned(flag)` set by the 5 s timer, `into_iter([":(top,literal)<prefix>"])` where `<prefix>` is the library root relative to the worktree ('/'-separated; empty → no pattern). Never call `Outcome::write_changes` (the index stays byte-identical, spike A).
  5. **Map** (precedence Conflicted > New > Modified):
     - `IndexWorktree` summary `Conflict` → Conflicted; `Added` (untracked) or `IntentToAdd` → New; `Modified`/`TypeChange` → Modified; `Removed` → absent (not listed); `Renamed`/`Copied` → New for the destination.
     - `TreeIndex` `Addition` → New; `Modification` → Modified; `Rewrite` → New (destination); `Deletion` → absent.
     - Keys: repository-relative path minus `<prefix>/` → `RelPath`; paths outside the prefix are dropped.
  6. **Unknown overrides:** for each mapped path and for every listed file, look up the `filter` attribute (`repo.attributes_only(&index, Source::WorktreeThenIdMapping)`, `selected_attribute_matches(["filter"])`): set or valued → `Unknown(Filtered)` (C2). Dataless files (HV-SCRIPT-03) → `Unknown(Dataless)`, and they must not be hashed: if `gix` would read them, exclude their paths from the pathspec instead [Inferred: excluding keeps the OS from downloading them; tested with fake metadata, U-10]. Submodule roots → `Unknown(Submodule)` [Unknown U-04].
  7. **Refresh:** on a watcher hint under `.git/` (`index`, `HEAD`, `refs/**`) or in the library, debounce 500 ms, recompute in the background; publish with the snapshot generation.
  8. Every `gix` entry point used is listed in a code comment with why it cannot spawn (Task 7 Step 3).
- **Data structures:** `GitStatus { by_path: BTreeMap<RelPath, FileStatus> }`, `FileStatus { Modified, New, Conflicted, Unknown(UnknownWhy) }`.
- **Complexity:** spike A fixtures run in milliseconds; 10,000 files [Unknown U-05].
- **Edge cases:** library is a subfolder of a large monorepo (pathspec limits the walk to the prefix; spike A `library-is-subfolder`); bare repository (no worktree → `NotARepo` for our purpose); linked worktree (`.git` gitfile) [Inferred: `gix` resolves gitfiles; test]; `core.autocrlf=true` (honoured: built-in EOL conversion is not a driver, spike A); NFD names (both sides report NFC when `core.precomposeUnicode=true`, spike A); LFS pointers (just files; `filter=lfs` attribute → `Unknown(Filtered)`); `.gitattributes` itself untracked (it is a dot file: not listed); racy timestamps (spike A `same-size-same-second`, `touched-unchanged` agree with the CLI).
- **Test cases:** `status_matches_git_cli` over the 18 fixtures of §0.4 (gate E5); `hostile_repo_runs_no_program` with the §0.4 config, environment and markers (plus the macOS sandbox run); `trust_is_reduced_even_with_global_safe_directory_star`; `global_and_system_config_are_not_read`; `git_dir_env_is_ignored` (decoy repository); `filtered_path_is_unknown`; `foreign_owner_repo_is_untrusted`; `discovery_stops_at_home_and_fs_boundary`; `not_a_repo_is_not_an_error`; `index_is_never_written` (byte equality, not only mtime); `timeout_is_unavailable_not_partial`; `status_of_10k_files_under_1s` (perf).
- **Prior art:** VS Code restricts new folders because "Restricted Mode tries to prevent automatic code execution" (S17) — the same threat, solved here by never executing.
- **Decision:** `gix` =0.90.0, `default-features = false`, `features = ["status", "sha1"]`, isolated + reduced trust (C1, C2). The `NoGitSubprocess` source check covers our crates; spawn sites inside `gix` are unreachable under these options (§0.4) and the hostile test guards them.

### HV-SCRIPT-12: Large files — T2 preview, paste confirmation, reduced Run as script

- **Problem:** Open and run script files larger than the editor tiers allow.
- **Inputs / outputs:** as `alg/25` UI-EDIT-23 and `alg/20` HV-EXEC-15 (reduced form).
- **Algorithm:**
  1. Tier at open (UI-EDIT-23): T2 above 20 MB → read-only preview of the first 2.5 MB cut at a line break, banner with Run as script.
  2. Paste that would take a buffer from ≤ 20 MB to 20–100 MB → confirmation (UI-EDIT-14 step 5): Cancel is primary and focused; "Paste as preview". Above 100 MB → refused.
  3. Run as script: stream the file from disk in 64 KiB chunks through `StreamSplitter::with_policy(ctx, flavor, StreamPolicy::Eager)` (Task 8, R-M4b-46: memory bounded by the longest statement plus one chunk; a later block mismatch that would refuse units already sent yields one `Ambiguous { offset, reason: UnknownBlock }` and the run **stops** there, "the statements before it ran"), classify and gate **each unit** exactly as Run, stop on the first error by default, single target (the tab's binding). The default `StreamSplitter::new` and its whole-script equality stay for every other caller.
  4. A watcher change during a run → the run continues on the bytes it already read; the tab shows "Changed on disk" after the run [Inferred: a running script must not switch input mid-stream].
- **Edge cases:** gzip dumps (not opened, deferred); a 15 MB single line (T1 windowing); UTF-16 dump (decode on the fly; `encoding_rs` streaming decoder).
- **Test cases:** `peak_memory_is_bounded_on_a_150mb_dump` (Task 8, `Eager`), `eager_stream_stops_at_a_stray_end` (Task 8), `t2_preview_is_read_only`, `paste_25mb_asks_and_cancel_is_default`, `run_as_script_gates_every_unit`, `run_as_script_stops_on_first_error`.
- **Decision:** R-M4b-20, -46.

## 2. Dependency pins for Task 1a

All licences checked against `deny.toml` `[licenses] allow` (Apache-2.0, Apache-2.0 WITH LLVM-exception, MIT, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, OFL-1.1, MPL-2.0, CC0-1.0) with `OR` = either side and `AND` = both [Observed: spike lockfile resolved with Rust 1.98.1, `cargo tree --target all -e normal,build`].

| Crate | Pin | Features | Licence | Published | Owner crate |
|---|---|---|---|---|---|
| `gix` | `=0.90.0` | `default-features = false`, `["status", "sha1"]` | MIT OR Apache-2.0 | 2026-10-22 | `hacivat-git` |
| `notify` | `=8.2.2` | default (`macos_fsevent`) | CC0-1.0 | 2026-11-08 | `hacivat-scripts` |
| `notify-debouncer-full` | `=0.7.0` | default | MIT OR Apache-2.0 | 2026-01-23 | `hacivat-scripts` |
| `encoding_rs` | `=0.8.44` (already in `Cargo.lock`) | default | (Apache-2.0 OR MIT) AND BSD-3-Clause | 2026-10-30 | `hacivat-scripts` |
| `trash` | `=5.3.0` (if Q11) | default | MIT | 2026-11-05 | `hacivat-scripts` |

`status` pulls `dirwalk`, `index`, `blob-diff`, `attributes` and therefore `command` (the `gix-command` crate compiles, unused under the trust rules of HV-SCRIPT-11). No network, credential or prompt crate enters: `gix-credentials`, `gix-prompt`, `gix-negotiate`, `curl`, `reqwest` are absent; `gix-transport`, `gix-protocol` and `gix-packetline` compile with no client feature [Observed: `cargo tree -e features`].

The 83 crates the five pins add that are not in our `Cargo.lock` at the same version are listed in ADR 0009 with their licences; all are on the allow-list (MIT OR Apache-2.0 × 60, MIT × 10, Unlicense OR MIT × 4, Unlicense/MIT × 1, MIT/Apache-2.0 × 1, Apache-2.0 OR MIT × 1, Apache-2.0 × 2, ISC × 2, Zlib OR Apache-2.0 OR MIT × 1, CC0-1.0 × 1). 15 of the 83 are a second version of a crate already in the lock (`bstr`, `bumpalo`, `cc`, `errno`, `filetime`, `heapless`, `jiff`, `jiff-core`, `jiff-static`, `jiff-tzdb`, `mio`, `pin-project-lite`, `regex-automata`, `smallvec`, `walkdir`); the spike resolved from an empty lock, so inside the workspace most of them unify with the existing versions [Inferred: semver-compatible requirements; Task 1a's `Cargo.lock` diff is the record]. Rejected alternative: `notify` 9.0.0 + `notify-debouncer-full` 0.8.0, which adds `xxhash-rust` 0.8.21 under **BSL-1.0** (not allowed) and was one day old (C7).

## 3. Unresolved items needing a product or controller decision

1. **ADR number** (C10): settled by R-M4b-33 (pre-flight): **ADR 0009**.
2. **`notify` 9** (U-11): stay on 8.2.2 for M4b; moving to 9.x needs a `[[licenses.exceptions]]` for `xxhash-rust` (BSL-1.0, a permissive OSI licence) or an upstream change.
3. **Hidden files** (C11): skipping files whose names start with `.` also hides a user's deliberately hidden `.scratch.sql`. Recommended: accept (consistent with skipped dot folders).
4. **Hard links** (HV-SCRIPT-07 step 3): in-place save for `st_nlink > 1` trades atomicity for keeping the link; recommended as written.
