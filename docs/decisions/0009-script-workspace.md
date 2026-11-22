# ADR 0009 — Script workspace

- Status: Accepted (skeleton; later M4b tasks append `## m4b-tNN — <topic>` sections)
- Date: 2026-11-21
- Scope: M4b (HV-SCRIPT-01..12 in `docs/research/algorithms/28-script-workspace.md`; R-M4b-1 to -46)
- Number: ADR 0008 is M4's "Metadata and introspection"; M4b's is 0009 (R-M4b-33).

## Context

M4b ships decision L14 in its local scope: a folder of plain `.sql`/`.md` files becomes a script library bound to a logical database, is projected as a "Scripts" node under every connection that shows that database, and runs a script on the connection it was opened under. The folder is shared with other tools and people: other editors write it, git manages it, teammates sync it. Everything in it is therefore untrusted input, and Hacivat must never turn it into a way to run code or to leak credentials. Two new dependencies carry most of the risk, a git library and a file watcher; Task 0 spiked both (`alg/28` §0.4, §0.5) before any manifest changes.

## Decisions

### D1. Two new Core crates (R-M4b-1)

- `crates/core/hacivat-scripts`: registry, manifest, binding, scan, path safety, environment scoping, file I/O, recovery copies, watcher, secret scanner, "Go to script" index, `LibraryService`.
- `crates/core/hacivat-git`: read-only status only.
- Both are Core: no `gpui`, no `crates/ui/*`. Only `hacivat-app` sees them and UI types together.
- `check-deps` `BackendOutsideOwner` arms: `notify` and `notify-debouncer-full` only in `hacivat-scripts`; `gix` and every `gix-*` crate as a direct dependency only in `hacivat-git`; `encoding_rs` only in `hacivat-scripts`. No type of these crates appears in the owners' public API.

### D2. Git status through `gix`, isolated, reduced trust; never a git subprocess (R-M4b-10, Q4)

- `gix =0.90.0`, `default-features = false`, `features = ["status", "sha1"]`.
- Open with `gix::open::Options::isolated().with(gix::sec::Trust::Reduced)`. After open, assert two invariants, else `Unavailable(TrustPromoted)`: `repo.git_dir_trust() == Trust::Reduced`, and no config section of source kind `Repository` has `trust == Full` (gix can promote single sections when a `safe.directory` matches the config file's path, `open/repository.rs:425-465`). Both open halves are load-bearing (`alg/28` §0.4): without `with(Reduced)` an owned repository is fully trusted and its filter driver runs during status; without `isolated()` a global `safe.directory = *` promotes trust back to full.
- **Status call path (fix round 1, controller ruling):** never `Repository::status(..)`. Its built-in submodule handler re-opens the repository with `Options::default()` for every gitlink entry in a build without `parallel` (`gix` `status/index_worktree.rs:231-233`): system, global and environment config with includes, trust from ownership, and on Windows a `git config` spawn from `PATH` (`gix-path` `env/git/mod.rs:176-177`). Task 7 calls `Repository::index_worktree_status(…)` with Hacivat's own no-op `SubmoduleStatus` (gitlinks → `Unknown(Submodule)`) and `Repository::tree_index_status(…)` for HEAD vs index. `parallel` stays off. The fix-round spike showed the nested open (a global-config FIFO detector fired) with the built-in handler and none with the no-op handler (`alg/28` §0.8).
- Under these options the spike ran the status of a repository configured with `core.fsmonitor`, filter `clean`/`process`, `diff.external`, textconv, `core.hooksPath` and hooks, plus a hostile `HOME`, `PATH` and `GIT_*` environment, under a sandbox that kills the process on any spawn attempt: no program ran and no spawn was attempted; the git CLI ran every one of those programs.
- A path whose attributes name a `filter` is `Unknown(Filtered)`: `gix` does not run the driver and would compare raw bytes, so Hacivat checks the attribute itself.
- Ownership by another user (Unix), checked for the worktree, `.git`, and a gitfile's resolved git dir and common dir → `Untrusted`, decided by Hacivat (forced reduced trust makes `gix` open it regardless). Gitfile, `commondir` or alternates redirection out of the repository → `Unavailable(Redirected)`; `core.worktree` from repository config is ignored under reduced trust. Unsupported formats (`sha256`, `reftable`) → `Unavailable(Unsupported)`.
- The index is never written (`Outcome::write_changes` is never called).
- The status runs on its own blocking worker with a 5 s budget; at the deadline the caller answers `Unavailable(Timeout)` without waiting (gix's blocking opens of a FIFO `.gitignore` cannot be interrupted, `alg/28` §0.8), and at most one stuck worker exists per library. Dataless paths are passed as `:(exclude)` pathspecs, so gix never opens them.
- **`NoGitSubprocess`**: no product crate builds `std::process::Command` for `git`/`git.exe` (source-level `check-deps` arm; tests may run the git CLI only on fixtures they created). Spawn-capable code inside `gix` (`gix-command`, `gix-filter` drivers, `gix-diff` textconv, `gix-path`'s `git --exec-path` and its Windows `git config` lookup for system config) is unreachable through D2's options **and** call path; the hostile-repository and submodule tests guard that on every OS (Windows inside a Job Object limited to one active process).
- Rejected: the git CLI with protective flags (relies on knowing every program-running key; `--no-optional-locks` does not stop `core.fsmonitor`, spike A); `git2`/libgit2 (GPL-2.0 with linking exception is not on the allow-list).

### D3. File watching: `notify` + `notify-debouncer-full`, polling fallback (R-M4b-14, -16)

- `notify =8.2.2`, `notify-debouncer-full =0.7.0`, default features (FSEvents on macOS, inotify on Linux, `ReadDirectoryChangesW` on Windows).
- Build the debouncer with `new_debouncer_opt::<_, RecommendedWatcher, NoCache>(150 ms, None, handler, NoCache, Config::default().with_follow_symlinks(false))`. Never `new_debouncer`: it takes no `Config`, and its default `FileIdMap` cache walks every added path with `follow_links(true)` (on watch start, rescans and every `Create`), so a directory link to `/` would be walked whatever `follow_symlinks` says; the fix-round spike saw it walk 20,000 files through a link created after the watch started (`alg/28` §0.8). inotify and poll follow symlinks by default, hence `with_follow_symlinks(false)`. Renames pair by content hash, so file ids are not needed.
- Debounce 150 ms (`alg/28` HV-SCRIPT-08 Decision). Debounced events are dirty-path hints; the watcher reconciles each against the disk (`lstat`, `DiskStamp`, subtree rescan for a directory hint), because on macOS the reported kinds are unreliable (an in-place write arrives as `Create`, a folder delete as a metadata change on the folder). Content is read only after the stamp settles for one more window.
- Polling every 2 s for network file systems, an insufficient inotify budget (Linux), or a lost backend; more than 1,000 hints in one window, or an OS overflow/rescan signal, is one rescan.
- Rejected for now: `notify 9.0.0` + debouncer `0.8.0` (published the day of the spike; adds `xxhash-rust` under BSL-1.0, not on the allow-list).

### D4. Encodings: `encoding_rs` (UI-EDIT-02)

- `encoding_rs =0.8.44` (already in `Cargo.lock`), `(Apache-2.0 OR MIT) AND BSD-3-Clause`, only in `hacivat-scripts`. Save keeps encoding, BOM and the dominant line ending.

### D5. Move to Trash: `trash` (Q11)

- `trash =5.3.0` (MIT), only in `hacivat-scripts`, if the PO keeps Q11's default. 5.4.0 was four days old at the spike.

### D6. Untrusted input

Everything read from a library is data: file and folder names, contents, `.hacivat/workspace.toml`, symlink targets and everything under `.git/`. It never selects code to run, never changes a connection's environment, read-only state, capsule or safety settings, and never names a path outside the library. The manifest has exactly `format`, `database`, `aliases`; a later key that could name a path, program, connection or setting must bump `format`. Displayed names escape bidi and invisible characters as `<U+XXXX>`. Directory symlinks are never followed, by the scanner or by any watcher cache; file symlinks pointing outside the library are listed disabled and never opened; every read and write re-validates the path against the canonical root.

### D7. Credentials never reach a library (L14)

Hacivat writes into a library folder only (a) the bytes of a script the user saved and (b) `.hacivat/workspace.toml` through the typed writer, once, on attach, after the user saw it. Both writes are relative to the validated directory fd (`openat`/`mkdirat`/`renameat`/`linkat` with `O_NOFOLLOW`); a `.hacivat` that is a symlink or not a directory is refused, and the manifest is published with `linkat`, never replacing an existing file. No cache, state, recovery copy, lock file, log, profile id, connection name, host, port, user, keyring slot or password. Save temp files live in the same directory only for the instant of the atomic rename, are named `.<name>.hv-<random>.tmp` and are never listed. Recovery copies and all script-workspace state live under the app's own directories (0700/0600).

## Dependencies added (Task 0 spike, for Task 1a)

Crates the five pins add that are not in `Cargo.lock` at the same version, normal and build edges, all four `deny.toml` targets (`cargo tree --target all -e normal,build`, Rust 1.98.1, resolved from an empty lock). Every licence is on the allow-list. "second version" means the name is already in the lock at another version; Task 1a's resolution inside the workspace decides whether a second copy remains.

| Crate | Version | Licence | In `Cargo.lock` |
|---|---|---|---|
| `bstr` | 1.14.0 | MIT OR Apache-2.0 | second version |
| `bumpalo` | 3.20.4 | MIT OR Apache-2.0 | second version |
| `cc` | 1.11.3 | MIT OR Apache-2.0 | second version |
| `clru` | 0.6.3 | MIT | new |
| `dashmap` | 6.2.1 | MIT | new |
| `errno` | 0.3.15 | MIT OR Apache-2.0 | second version |
| `faster-hex` | 0.10.1 | MIT | new |
| `file-id` | 0.2.3 | MIT OR Apache-2.0 | new |
| `filetime` | 0.2.30 | MIT/Apache-2.0 | second version |
| `gix` | 0.90.0 | MIT OR Apache-2.0 | new |
| `gix-actor` | 0.44.1 | MIT OR Apache-2.0 | new |
| `gix-attributes` | 0.37.1 | MIT OR Apache-2.0 | new |
| `gix-bitmap` | 0.6.1 | MIT OR Apache-2.0 | new |
| `gix-chunk` | 0.10.1 | MIT OR Apache-2.0 | new |
| `gix-command` | 0.12.1 | MIT OR Apache-2.0 | new |
| `gix-commitgraph` | 0.42.0 | MIT OR Apache-2.0 | new |
| `gix-config` | 0.62.2 | MIT OR Apache-2.0 | new |
| `gix-config-value` | 0.21.1 | MIT OR Apache-2.0 | new |
| `gix-date` | 0.18.1 | MIT OR Apache-2.0 | new |
| `gix-diff` | 0.70.0 | MIT OR Apache-2.0 | new |
| `gix-dir` | 0.32.0 | MIT OR Apache-2.0 | new |
| `gix-discover` | 0.57.1 | MIT OR Apache-2.0 | new |
| `gix-error` | 0.5.2 | MIT OR Apache-2.0 | new |
| `gix-filter` | 0.36.1 | MIT OR Apache-2.0 | new |
| `gix-fs` | 0.24.1 | MIT OR Apache-2.0 | new |
| `gix-glob` | 0.29.1 | MIT OR Apache-2.0 | new |
| `gix-hash` | 0.28.1 | MIT OR Apache-2.0 | new |
| `gix-hashtable` | 0.18.1 | MIT OR Apache-2.0 | new |
| `gix-ignore` | 0.24.1 | MIT OR Apache-2.0 | new |
| `gix-imara-diff` | 0.3.3 | Apache-2.0 | new |
| `gix-index` | 0.58.0 | MIT OR Apache-2.0 | new |
| `gix-lock` | 26.0.1 | MIT OR Apache-2.0 | new |
| `gix-macros` | 0.2.1 | MIT OR Apache-2.0 | new |
| `gix-object` | 0.66.1 | MIT OR Apache-2.0 | new |
| `gix-odb` | 0.87.0 | MIT OR Apache-2.0 | new |
| `gix-pack` | 0.77.0 | MIT OR Apache-2.0 | new |
| `gix-packetline` | 0.24.1 | MIT OR Apache-2.0 | new |
| `gix-parallel` | 0.50.1 | MIT OR Apache-2.0 | new |
| `gix-path` | 0.14.1 | MIT OR Apache-2.0 | new |
| `gix-pathspec` | 0.22.1 | MIT OR Apache-2.0 | new |
| `gix-protocol` | 0.68.0 | MIT OR Apache-2.0 | new |
| `gix-quote` | 0.10.1 | MIT OR Apache-2.0 | new |
| `gix-ref` | 0.69.1 | MIT OR Apache-2.0 | new |
| `gix-refspec` | 0.47.1 | MIT OR Apache-2.0 | new |
| `gix-revision` | 0.52.0 | MIT OR Apache-2.0 | new |
| `gix-revwalk` | 0.38.0 | MIT OR Apache-2.0 | new |
| `gix-sec` | 0.16.1 | MIT OR Apache-2.0 | new |
| `gix-shallow` | 0.15.1 | MIT OR Apache-2.0 | new |
| `gix-status` | 0.37.0 | MIT OR Apache-2.0 | new |
| `gix-submodule` | 0.36.1 | MIT OR Apache-2.0 | new |
| `gix-tempfile` | 26.0.1 | MIT OR Apache-2.0 | new |
| `gix-trace` | 0.3.1 | MIT OR Apache-2.0 | new |
| `gix-transport` | 0.62.0 | MIT OR Apache-2.0 | new |
| `gix-traverse` | 0.64.0 | MIT OR Apache-2.0 | new |
| `gix-url` | 0.40.1 | MIT OR Apache-2.0 | new |
| `gix-utils` | 0.5.2 | MIT OR Apache-2.0 | new |
| `gix-validate` | 0.13.1 | MIT OR Apache-2.0 | new |
| `gix-worktree` | 0.59.0 | MIT OR Apache-2.0 | new |
| `gix-zlib` | 0.4.0 | MIT OR Apache-2.0 | new |
| `heapless` | 0.8.0 | MIT OR Apache-2.0 | second version |
| `inotify` | 0.11.5 | ISC | new |
| `inotify-sys` | 0.1.8 | ISC | new |
| `jiff` | 0.2.46 | Unlicense OR MIT | second version |
| `jiff-core` | 0.1.8 | Unlicense OR MIT | second version |
| `jiff-static` | 0.2.46 | Unlicense OR MIT | second version |
| `jiff-tzdb` | 0.1.10 | Unlicense OR MIT | second version |
| `kqueue` | 1.2.1 | MIT | new |
| `kqueue-sys` | 1.1.2 | MIT | new |
| `mio` | 1.2.5 | MIT | second version |
| `nonempty` | 0.12.0 | MIT | new |
| `notify` | 8.2.2 | CC0-1.0 | new |
| `notify-debouncer-full` | 0.7.0 | MIT OR Apache-2.0 | new |
| `notify-types` | 2.3.1 | MIT OR Apache-2.0 | new |
| `objc2-core-services` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | new |
| `pin-project-lite` | 0.2.18 | Apache-2.0 OR MIT | second version |
| `prodash` | 31.0.1 | MIT | new |
| `regex-automata` | 0.4.19 | MIT OR Apache-2.0 | second version |
| `sha1dc` | 0.1.5 | MIT OR Apache-2.0 | new |
| `smallvec` | 1.16.6 | MIT OR Apache-2.0 | second version |
| `trash` | 5.3.0 | MIT | new |
| `unicode-bom` | 2.0.3 | Apache-2.0 | new |
| `urlencoding` | 2.1.3 | MIT | new |
| `walkdir` | 2.5.1 | Unlicense/MIT | second version |

## Open items

- Linux and Windows watcher behaviour, inotify watch use, SMB/NFS, dataless files, foreign-owned repositories, submodules and `gix` performance on 10,000 files are `[Unknown]` (`alg/28` §0.7, U-01..U-12); Tasks 4, 7 and 9 settle them.
- `notify` 9.x: revisit after M4b; needs a ruling on `xxhash-rust` (BSL-1.0).
- v1.x git network operations "use the user's own `git`" (L14). They will need a named, reviewed exception to `NoGitSubprocess`; nothing in M4b runs git.

## Rule

Sections below are appended by M4b tasks as `## m4b-tNN — <topic>`. A security choice made in a fix round goes into the same section before re-review.

## m4b-t01a — manifests and seams

### Manifests (the M4b table; no later task edits a manifest)

- Workspace pins exactly as D2–D5: `gix =0.90.0` (no default features; `status`, `sha1`; no `parallel`, R-T0-1), `notify =8.2.2`, `notify-debouncer-full =0.7.0`, `encoding_rs =0.8.44`, `trash =5.3.0` (Q11's default). `Cargo.lock` gains 72 third-party packages and loses or bumps none; `cargo deny --all-features check licenses bans sources` passes with no `deny.toml` change. Three of the 72 are not in D's spike table (`defmt 0.3.101`, `gix-note 0.3.1`, `gix-worktree-stream 0.39.0`, all MIT OR Apache-2.0): the workspace resolve differs from the spike's empty-lock resolve.
- `hacivat-scripts` normal dependencies: `hacivat-proto`, `hacivat-settings`, `hacivat-l10n`, **`hacivat-git`**, `arc-swap`, `twox-hash`, `toml`, `toml_edit`, `serde`, `uuid`, `regex`, `icu_normalizer`, `tokio` (`sync`, `rt`, `time`), the four backends, and `libc` on Unix. `hacivat-git` is added because Task 10's `WorkspaceSnapshot.git: HashMap<LibraryId, GitState>` and the debounced status refresh live in this crate; the plan's table did not list it. Dev-dependencies: itself with `test-support`, `hacivat-exec` (R-M4b-28: dev only), `hacivat-session` (`test-support`), `hacivat-driver` (`fake`), the three driver crates, `hacivat-testkit` (`ssh`), `hacivat-net`, `hacivat-secrets`, `tokio` (`macros`, `rt-multi-thread`, `test-util`), `tempfile`, `proptest`.
- `hacivat-git`: `hacivat-proto`, `gix`; dev `tempfile`.
- `hacivat-app`: both crates; `hacivat-scripts/test-support` as a dev-dependency and in the `shots` feature (Tasks 13–18 drive libraries through the fake watcher source).
- `fuzz/scripts`: its own cargo-fuzz package and lock over `hacivat-scripts` only (R-M4b-40), targets `workspace_toml` (over `manifest::read_bytes`) and `pathsafe` (over `pathsafe::component` and `RelPath::parse`); the CI job `fuzz-scripts` (`ci.yml` block `m4b-t01a-ci`, dispatch and schedule only, pinned nightly, actions by SHA). No libclang step: no `pg_query` in the graph.

### Layering rules (`cargo xtask check-deps`)

- `BackendOutsideOwner` gains `notify`, `notify-debouncer-full`, `encoding_rs`, `trash` (owner `hacivat-scripts`) and `gix` plus the prefix arm `gix-*` (owner `hacivat-git`), over resolved and declared edges of any kind.
- `ScriptsBelowExec` (new): `hacivat-scripts` reaches no `hacivat-exec`, `hacivat-meta`, `hacivat-session`, `hacivat-introspect`, driver or `russh` through normal edges (declared, optional included, or resolved at any depth), and `hacivat-exec` depends on `hacivat-scripts` by no edge (R-M4b-28, C-M4-6). The plan asked for a one-off metadata test; a rule keeps it true for every later task.
- `NoGitSubprocess` (new, token-based in `source_violations`, R-M4b-41): no non-test file of a product crate (every `crates/<layer>/<crate>/` except the bench tool `hacivat-bench`; everything outside the crate's `tests/`, with `#[path]` and `include!` targets counted) holds a string literal (plain, raw, byte, C, or one `concat!` of literals) with a word whose last path component is `git`/`git.exe`, ASCII case-insensitive; and `hacivat-git` and `hacivat-scripts` name no `process::` path at all. Price: product strings may not use the bare word (write "repository"); a name assembled at run time from characters or escapes is not seen outside those two crates. Unit tests inside `src/` are product files for this rule: fixtures that run the CLI live under `tests/`.
- `WatcherFollowsLinks` (new, R-T0-2): no non-test file under `crates/` names `new_debouncer`, `FileIdMap` or `RecommendedCache`, calls `with_follow_symlinks(true)`, names `new_debouncer_opt` without both `NoCache` and `with_follow_symlinks(false)`, or names `PollWatcher` without `with_follow_symlinks(false)`. `clippy.toml` also bans `notify_debouncer_full::new_debouncer` (`disallowed-methods`) and the types `FileIdMap`, `RecommendedCache` (`disallowed-types`), with `allow-invalid` for crates that do not depend on the debouncer. A Hacivat cache that never leaves the root (C5's alternative) needs a reviewed change to this rule.
- `BackendInPublicApi` (new): no `src/` file of `hacivat-scripts` or `hacivat-git` shows a backend type in a `pub` item's header (an enum's or trait's whole body), an `impl` header, a `pub use`, or through a name imported from a backend; a glob import of a backend is refused. `pub` items of private modules count (internals use `pub(crate)`).
- `PRODUCT_CRATES` gains both crates; `xtask docker` `DEFAULT_TEST_CRATES` gains `hacivat-scripts` (R-M4b-36).

### Seams (fail closed, F31; none wired into a user path)

- `hacivat-scripts` declares every module the later tasks own (`registry`, `manifest`, `binding`, `names`; `scan`, `pathsafe`, `envscope`, `tree`, `dataless`; `fileio`, `encoding`, `recovery`; `secrets`; `watch`; `service`, `index`, `events`), so no later task edits `lib.rs`. Stubs: `LibraryService::open` publishes an empty snapshot in `Unavailable(NotImplemented)` and starts nothing, every other service call answers `NotImplemented`; `binding::resolve` binds nothing; `envscope::{scope_of, visible}` hide everything; `scan_secrets` answers one `Unscanned` finding over the whole text; `fileio::{read, save}`, `manifest::{read, read_bytes}`, `pathsafe::{component, join}` and `LogicalDb::parse` refuse.
- `RelPath::parse` (and its `Deserialize`) is real: relative, `/`-separated, no empty, `.` or `..` component, no `\`, no control character. The full name policy (device names, trailing dot or space, 255 bytes, `O_NOFOLLOW`, the canonical root) stays in `pathsafe::{component, join}` (Task 4), and nothing reaches the file system except through `join`. `LibraryId`, `RelPath` and `DatabaseName` serialize as plain strings for Task 1b's `TabKind::Script`.
- The watcher seam is built only as `new_debouncer_opt::<_, RecommendedWatcher, NoCache>(150 ms, None, …, NoCache, Config::default().with_follow_symlinks(false))` (`watch::OsEventSource::new`); its `watch` is a stub that watches nothing. `testing::FakeEventSource` (feature `test-support`) is the injected source for deterministic tests.
- Script text never shows in `Debug` (`Decoded`, `SaveText`), and those types do not derive `PartialEq`.
- `hacivat-git`: the types and `status` answering `Unavailable(NotImplemented)`. Paths are keyed by `GitPath` (`Borrow<str>`), because `hacivat-git` cannot name `hacivat_scripts::RelPath` (the dependency goes the other way); the service looks entries up by `rel.as_str()`.
- `hacivat-exec`: `history::{secret_spans, SecretSpanKind}` with kinds `Value`, `ConnectionValue`, `Armed` (C9; the stub answers one `Value` span over the whole text), mirrored by `hacivat_scripts::SpanKind { Password, ConnectionString, Other }`; `script_run::{ScriptChunks, ChunkError, ScriptRunner, ScriptRunRequest, ScriptRunOutcome, run_file}`, re-exported at the crate root (`RefuseReason`/`StopReason` as `ScriptRefuseReason`/`ScriptStopReason`, since `units::RefuseReason` exists); `run`/`run_file` answer `Refused(NotImplemented)`, pull no chunk and send nothing.
- `hacivat-settings`: `LocalPaths::{libraries_toml, script_recovery_dir}`.

### Deviations from the plan's interface sketch

1. `LibraryService::open(paths, rt: &tokio::runtime::Handle)`, not `&CoreRuntime`: `CoreRuntime` lives in `hacivat-session`, which `hacivat-scripts` never depends on. The app passes `CoreRuntime::handle()`.
2. `ScriptChunks`/`ChunkError` exist in both `hacivat-scripts` (what `open_stream` returns) and `hacivat-exec` (what `run_file` takes), as `SpanKind` mirrors `SecretSpanKind`; the app adapts one to the other. Neither crate may depend on the other, and `hacivat-proto` is not a Task 1a file.
3. `WorkspaceSnapshot` has a `state` field (the service's own state, which the stub sets to `Unavailable(NotImplemented)`), and `LibrarySnapshot` has a `root: LibraryRoot`.
4. `hacivat-git` keys statuses by `GitPath`, not `RelPath` (above).

The controller accepted all deviations (R-T1a-2); their carries go to Tasks 7, 10 and 12.

### Fix round 1 (review I1–I3, M1–M6; rulings R-T1a-1, R-T1a-2)

- **The bare word stays refused (R-T1a-1).** Product `.rs` literals may not hold `git`/`git.exe` as a path leaf; inline `#[cfg(test)]` modules under `src/` count, CLI fixtures live under `tests/`. A later need gets an exact-literal allow-list reviewed with that task, never a pattern.
- **No process API in `hacivat-git` and `hacivat-scripts` (I1).** `NoGitSubprocess` refuses there:
  - the identifiers `process` (wherever it stands, so `use std::{fs, process as sp}` and `std::os::unix::process` count), `CommandExt`, `gix_command`, the `gix-path` `env` functions that build or run a program (`shell_command`, `exe_invocation`, `core_dir_program`, `installation_program`, `installation_config*`, `system_prefix`), and the libc `exec*`, `fexecve`, `posix_spawn*`, `popen`, `vfork`, `pidfd_spawn`;
  - the paths `gix::command`, `gix::path::env`, `gix_path::env`, `libc::{fork, system, syscall, daemon}`, `std::process`, `tokio::process`, after `use … as` and `extern crate … as` renames are resolved (`use gix as g; g::command::prepare` counts), in imports and in expressions;
  - any glob import from outside the crate (`use gix::*`, `use libc::*`), which would hide the rest.
  - `tokio::spawn`, `spawn_blocking`, `std::thread::spawn`, libc constants and `gix::open`/`gix::sec`/`gix::path::from_bstr` stay allowed. Clippy could not scope a ban to two crates, so this is a check-deps rule only; renames cannot evade it because they are resolved.
- **Compile-time spellings (I2).** The tokenizer keeps char literals (cooked) as tokens; the literal check reads every string raw and with its escapes resolved (`\xHH`, `\u{…}`, simple escapes, `\`-newline continuations), every `concat!` joined (char pieces, nested `concat!`/`stringify!`), and every `stringify!` printed. `.git`, `.git/index`, `gitdir:`, hyphenated Fluent ids and prose with ".git" still pass. Run-time assembly (`["gi", "t"].concat()`, `format!`, `env::var`) stays a documented limit outside the two crates, where the process-API ban covers it.
- **Watcher, per call site (I3).** `WatcherFollowsLinks` now applies to `hacivat-scripts` only (the one crate that may depend on `notify`) and checks each construction: `new_debouncer_opt` needs a turbofish whose cache is `NoCache` and a config argument that is the shared `watcher_config()` or sets `with_follow_symlinks(false)` inline (every other mention, a function pointer included, is refused; imports are not mentions); a watcher type's `new`/`with_initial_scan`, direct or qualified (`<X as Watcher>::new`), needs the same config argument; renaming or aliasing a watcher type or `new_debouncer_opt` is refused; `with_follow_symlinks(…)` with anything but `false` is refused; every `Config::default()` must set `with_follow_symlinks(false)` in its own method chain; `fn watcher_config` must set it in its body; `recommended_watcher`, `new_debouncer`, `FileIdMap` and `RecommendedCache` are refused outright. `clippy.toml` adds `disallowed-methods` for `notify::recommended_watcher`, `notify::Watcher::new` (which also catches generic `W::new` and qualified calls), `notify::PollWatcher::new` and `notify::PollWatcher::with_initial_scan`; a probe showed all five forms refused. **Carry for Task 9:** the poll fallback is the one site that constructs a watcher directly; it carries `#[expect(clippy::disallowed_methods, reason = "…")]` and passes `watcher_config()`.
- **Public API (M1).** Type aliases of backend types (transitively) and renames of a backend crate (`use notify as n; pub use n::Config`) count as backend names; a backend trait implemented for a type this file declares private (`impl DebounceEventHandler for Sink` with `struct Sink;`) is allowed.
- **Sets (M2).** `ScriptsBelowExec` also refuses `hacivat-net`, `hacivat-secrets` and `rustls`, and walks build edges as well as normal ones. `BACKENDS` gains `notify-types`, `file-id`, `inotify`, `inotify-sys`, `kqueue`, `kqueue-sys` and `fsevent-sys` (owner `hacivat-scripts`); the public-API list gains them too.
- **Types (M3–M5).** `EnvSet`'s bit is an exhaustive `match`. `LibraryId` has no `Default`. `LogicalDb` derives no `PartialEq`/`Eq`/`Hash`/`Ord` (nor do `DbNode` and `ManifestRead`, which hold one); binding uses `LogicalDb::same_db`/`names::same_db` (HV-SCRIPT-02 step 4), a stub answering `false` until Task 3. `LocalPaths::script_recovery_dir` returns `Option<PathBuf>`, `None` unless `lib` is one plain name.

## m4b-t03 — registry, manifest, binding

### Registry (`registry::LibraryRegistry`, HV-SCRIPT-01 steps 1, 2, 5)

- The single writer of `<config>/libraries.toml`: `version = 1`, `[[library]] id, path, added_at` (RFC 3339 UTC), `[[mapping]] profile, database, library`, and no other key (by type: the file cannot hold a host, user, keyring slot or password). Written through `fsio::atomic_write` (0600, a created folder 0700).
- Loading follows the profile store's rules (`hacivat-session` `profile/store.rs`): a missing file is an empty registry; a file that is not UTF-8 or TOML, has a malformed `version`, a `library`/`mapping` that is not an array of tables, is over 1 MiB, or nests deeper than 16 levels is set aside with `fsio::quarantine(path, "corrupt", now)`; a newer `version`, an unreadable file, or one that cannot be set aside makes the registry read-only (every change answers `ReadOnly`; nothing is written, never a silent empty list over the user's file). A bad row (`BadId`, `RelativePath`, `Duplicate`, `MissingField`), a mapping to an unregistered library (`OrphanMapping`) and rows past 32 libraries are dropped with a `RegistryDiag`.
- `add` checks on canonical paths (both sides, a not-yet-existing app folder through its deepest existing ancestor): absolute input, an existing folder, a UTF-8 path, not a filesystem root, not the home folder itself, not the app's config/data/cache/runtime folder or inside or containing one, not a registered library under another spelling or through a link (`AlreadyRegistered`), not inside or containing one (`Nested`, whole components), at most 32. The chosen spelling is stored; `RegisteredLibrary::root()` canonicalizes again on every call, so a missing folder stays registered and is the service's `Unavailable(Missing)`.
- Deleted profiles: `drop_profile` and `retain_profiles` (at start, against the profiles that exist) drop their mappings (R-M4b-2). `remove` drops a library's mappings and never touches the folder. Mutations do not save; the caller saves once.

### Manifest (`manifest`, HV-SCRIPT-01 steps 3–4)

- **Read** never panics and binds only the schema. Before parsing: `lstat` of `.hacivat` and of the file (a link or junction → `LinkIgnored`, a non-folder → `NotAFolder`, a FIFO/device/folder → `NotAFile`, never opened), the size (> 64 KiB → `TooLarge`, checked on `lstat` and again while reading through `take`), the open with `O_NOFOLLOW` (macOS `O_NOFOLLOW_ANY`) and `O_NONBLOCK`, and an `fstat` that must match the `lstat`. Then: UTF-8 BOM stripped, `NotUtf8`, **`TooDeep`** (defence in depth for small stacks: a 64 KiB file of `[` overflowed a 256 KiB thread stack inside the `toml` parser in this task's hostile test. `toml` 1.2.1's own recursion limit (80) protects the core runtime's 2 MiB threads, but not a small stack. A lexer that copies the token boundaries of the pinned `toml_parser` 1.1.5 lexer (fix round 1: strings, comments and atoms exactly as it ends them) measures `[`/`{` nesting first and refuses more than 16 levels; the schema needs 1. Every bracket token the parser will see is counted, so deeper input gets `TooDeep`), `InvalidToml`, `MissingFormat`, `BadFormat` (not a positive integer), `NewerFormat(n)` (bind by folder name, nothing else read). Unknown keys are named (escaped as `<U+XXXX>`, cut at 64 characters, at most 32 named and the rest counted); values are never kept. `database` and each alias go through `LogicalDb::parse`; aliases are deduplicated against each other and against the bound name (the database, or the folder name when it is absent), and cut at 32 (`TooManyAliases`). `ManifestRead::logical(folder)` is the binding name: `database`, else the folder name, else nothing.
- **Write** (`write_new`) writes exactly a `ManifestV1 { format, database, aliases }` (`to_toml()` is the text the attach dialog shows) and never replaces a file: a same-folder temp (`create_new`, `.workspace.toml.hv-<uuid v4>.tmp`, never the process id, R-T1a-4; synced) is published with `fs::hard_link`, which is `linkat` and fails when the name exists (→ `Exists`; the caller reads it instead); the temp is removed either way. When linking fails for another reason (exFAT, FAT and SMB have no hard links), the fallback creates the final name itself with `create_new` (`O_CREAT|O_EXCL`), which never replaces a file either; a partial file it created is removed. `write_new_without_hard_links` (feature `test-support`) forces that branch for the test. Modes come from the umask (R-M4b-42), never `fsio::atomic_write`.
- **Links (I4/n5).** A `.hacivat` that is a link (a dangling one included), a junction or not a folder is refused with `NotAFolder`, and nothing is created where it points (`mkdir` of `.hacivat` does not follow a link at the last component; a link there makes it fail).
- **Alias edit** (`add_alias`) goes through `toml_edit` (comments and layout stay), refuses a missing manifest (`NoManifest`: writing one needs the user to see it first, R-M4b-8), a linked, unreadable, invalid, newer or format-less one, and a full list (32), and is a no-op when the name already binds (NFC, ASCII case-insensitive). It replaces the file through a same-folder temp and a rename, so a hard link to a file outside the library is replaced, never written through; the old file's mode is kept.

### Directory handles without `openat` (a deviation from D7's wording)

D7 and HV-SCRIPT-01 step 4 say the writes are relative to the validated directory fd (`openat`/`mkdirat`/`linkat`). `std` has no `openat` on stable, `libc` is in the manifest for constants only, the workspace denies `unsafe` outside `hacivat-ui-core/src/platform/**`, and no `rustix`/`cap-std` dependency exists (no later task edits a manifest). So Task 3 opens a `Dir` handle per folder (`O_DIRECTORY` plus the no-follow flags; on Windows `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT` and an `fstat` that refuses a link or junction, i.e. a name-surrogate reparse point; other reparse tags such as cloud placeholders do not redirect and pass), keeps it for the whole operation, and runs every operation between two checks that each folder's path, from the root down, still names the opened folder (`lstat`: not a link, a folder, the same device and inode on Unix). Per platform:
- **Linux:** the operations go through `/proc/self/fd/<n>/<name>`, which is relative to the handle itself (the effect of `openat`/`linkat`). When procfs does not answer for the handle (or names another folder), every write refuses with `WriteError::Io(Unsupported)` and creates nothing; there is no fallback to plain paths (fix round 1, review m2). Reads still work, since each read re-checks the file's identity.
- **macOS:** the opens use `O_NOFOLLOW_ANY` (no link in any component; the kernel refuses it together with `O_NOFOLLOW`), on the canonical root, so no create ever goes through a link. `mkdir`, `link`, `rename` and `unlink` are path-based between the identity checks. Residual race (corrected in fix round 1, review m3): a local process that can write the library and swaps `.hacivat` for a link (or moves the real folder out and links to it) between a check and the next path call can make a path-based step act in a folder it chose. If it leaves the swap in place the after-check answers `Changed`; **a swap, the step and a swap-back between two checks goes unseen.** What such a step can do: a hard link or rename of our own temp file places the manifest's own text (format, database, aliases — never a credential) under `workspace.toml` in that folder, and never replaces a file there. The fallback's clean-up unlink of a partial `workspace.toml` (after a failed write, ENOSPC/EIO) was a third case. Fix round 1 narrows it: the unlink runs only while the name still is the file this call created (same device and inode), so it needs a second swap inside the window between that check and the unlink. Temp-file unlinks touch only random names that can only be ours. Closing all of this needs `openat`/`linkat`/`unlinkat`, i.e. a manifest change (`rustix`) or an `unsafe` exception: **carry for the controller**, together with T5's save, which has the same shape.
- **Windows:** the folder handles are opened without `FILE_SHARE_DELETE`, so neither the root nor `.hacivat` can be renamed, deleted or replaced by a junction while they are held. The junction case is tested (`manifest_write_refuses_junctioned_hacivat_dir`, `manifest_behind_a_junction_is_ignored`, `mklink /J`).

### Binding (`binding`, `names`, HV-SCRIPT-02)

- `LogicalDb::parse`: no C0/C1 control or DEL (`NameError::Control`), no bidi control U+061C, U+200E/F, U+202A–E, U+2066–9 (`Bidi`), then NFC, then 1–128 bytes of the NFC text (`Empty`, `TooLong`). A name with `/`, `..` or `:` is just a name and is never joined to a path. `LogicalDb` (de)serializes as a plain string through `parse`.
- `same_db`: NFC of both, then `eq_ignore_ascii_case`; never locale or Unicode casing (`İ` ≠ `I`, `ı` ≠ `i`, `ß` ≠ `SS`, `Ä` ≠ `ä`). The binding tests also run under `LC_ALL=tr_TR.UTF-8`.
- `resolve`: a mapping for the node's exact `(profile, database)` binds its library only, or nothing when that library is gone (no automatic fallback for a mapped node); otherwise every library whose database or an alias matches the node's logical name. Order: the folder name, NFC and ASCII-lowercased, then the `LibraryId`. **Deviation:** the plan names the I18N-09 key for the order; `hacivat_l10n::search::SearchKey` exposes no comparable folded text, so the order key is NFC + ASCII lowercase (stable and locale-free). `resolve` does not filter by `LibraryState`; the projection does (HV-SCRIPT-05).
- `logical_for_sqlite(main_file) -> Option<LogicalDb>`: the file stem (R-M4b-3, -32). **Deviation:** the plan's sketch returns a `LogicalDb`; a stem that is missing or not a valid name has none, so such a node binds only through a mapping.

### Over-breadth carries (carries-w1 m2) and other notes

- `check-deps` refuses the identifier `process` anywhere in this crate's non-test files and any glob import from outside the crate (R-T1a-1 fr1). Task 3's code has neither; the only process use is in `tests/manifest.rs` (`mkfifo` for the FIFO case, `cmd /C mklink /J` for the junction case), which the rule exempts as test files. The tests import every item by name (no proptest, no glob; R-T1a-3). Cost: none so far; a later task wanting a local variable named `process` must rename it.
- `tests/seams.rs` (Task 1a's file) is edited as a named exception: the stub tests of the four filled seams (`resolve_stub_binds_nothing`, `same_db_stub_matches_nothing`, the manifest half of `manifest_and_pathsafe_stubs_read_and_accept_nothing`) are replaced by `resolve_binds_nothing_without_libraries`, since the real behaviour is in `tests/{binding,manifest}.rs`. `NameError::NotImplemented` and `manifest::Diag::NotImplemented` are gone; `RegistryError::NotImplemented` stays for the service stubs (Task 10).
- No logging anywhere in these modules; diagnostics hold key names and error kinds, never values.

### Fix round 1 (review I1, m1–m12)

- **Line endings (I1).** `add_alias` writes the text back in the file's dominant line ending (CRLF lines against LF-only lines; ties LF), since `toml_edit` writes LF after every line, and keeps a UTF-8 BOM. A CRLF manifest from a Windows teammate stays CRLF in git.
- **Nesting guard (m1).** Rewritten as a copy of the pinned `toml_parser` 1.1.5 lexer's token boundaries (see Manifest above; `manifest::nesting_depth`). In particular, a backslash before an LF no longer hides the next line, a comment ends at a CR, and a quote inside an atom (`a = x"…`) starts no string. Every hostile form is tested on a 256 KiB thread, where a regression aborts the test process. A `toml_parser` bump must re-check the rules.
- **No procfs (m2).** Linux writes fail closed (`Unsupported`); see Linux above. The probe is injectable for tests (`manifest::with_procfs_absent`, feature `test-support`, per thread), so the refusal is tested on every OS.
- **Temp files (m4).** A temp file is removed on every error path, including a failed after-check. Its random name can only be ours, so the removal skips the identity checks. Each write also sweeps exactly `.workspace.toml.hv-<32 lowercase hex>.tmp` regular files older than 60 s (a crash's leftovers; a younger one may be another writer's), at most 4,096 entries. This is the only deletion Hacivat makes in a library, and it touches only names it makes itself.
- **Registry load (m6, m7).**
  - Rows whose canonical path (a missing folder through its deepest existing ancestor) equals an earlier row's are dropped as `Duplicate`. Rows inside or containing an earlier row's folder are dropped as `Nested`.
  - `libraries.toml` is opened only if it (or a link's target) is a regular file, with `O_NONBLOCK` and an `fstat` re-check. Otherwise: `RegistryDiag::NotAFile`, read-only, and the file is left alone, so a FIFO can never hang the start.
- **Display (m8).** `names::escape_untrusted` is public, and `LogicalDb::display_escaped()` shows invisible and format characters (ZWSP, U+2028/9, U+FEFF, soft hyphen, controls, bidi controls) as `<U+XXXX>`. Carry for T10, T13, T14 and T16: every library-derived name is shown through it, including the attach dialog's `ManifestV1::to_toml()` text.
- **SQLite stem (m11).** `logical_for_sqlite` answers `None` for `:memory:` and for a dot file (`.db`, `.hidden.db`).
- **Tests (m5, m10):** `case_only_and_nfd_spellings_are_the_same_library`, `registry_is_0600_and_atomic` and `manifest_edge_rows` (duplicate plain and quoted keys, NUL, CRLF), and `deleted_profile_drops_mappings` now checks the saved-and-reloaded registry.
- **Not changed:** m12. `lib.rs`'s Task 1a doc block still lists the old stubs, and `lib.rs` is in no W1 task's Files line; carry to the next owner of `lib.rs`, or to the controller's merge.

## m4b-t01b — UI and app seams

- **Stubs, none on a user path.** `crate::scripts` holds every M4b app module as a fail-closed stub (owners in its module docs). `ScriptsContributor` contributes no node and answers generation 0, and it is **not registered** (Task 13 registers it), so M4's "no contributor" check and every M4 shot hold. `scripts::open_script` opens nothing. `scripts::database_menu_items` adds no item. `scripts::goto::open` opens an empty Go-to-script layer over `Palette`. The six M4b commands (`scripts.goto`, `file.save`, `file.save_all`, `scripts.reveal`, `scripts.attach`, `scripts.rename`) are registered **disabled**, so none is a palette row and their chords pass through. `EditorView::replace_from_disk` refuses with `ReloadRefused::NotImplemented` and keeps the text, and `set_preview` does nothing.
- **Script tabs are a tab kind (R-M4b-29).** `TabKind::Script { script: ScriptTabId }`. `TabKind` is `Copy` and is copied at about ninety sites, so the binding (library, file, profile, database) sits behind an interned handle, as M4's `DdlObjectId` does (R-W0-1). The same binding always gets the same handle, so "one tab per (file, connection)" (R-M4b-4) compares handles (`Workspace::open_script_tab`). `workspace.json` stores the binding itself (`kind: "script"`, `library`, `rel_path`, `profile`, `database`), never the handle, and goes to version 2.
  - A version-1 file loads as before. A script-kind tab in it, or one whose library or path does not parse, closes like a tab of an unregistered kind, and the file is not set aside.
  - An unreadable profile restores the tab unbound, as for a console. It is never bound to another connection.
- **Where a console's text comes from.** `ConsoleSource { Console, Script { library, rel, database } }` lives beside each console's state in `Consoles` (`Consoles::source`/`set_source`, default `Console`). It is not a field of `ConsoleState`: a field would break the fourteen `ConsoleState` literals in M3's code and tests. Every console M3 registers or restores therefore stays a console of its own.
- **The three M4 seam gaps (R-M4b-34).**
  - (1) Enter or a double click on a contributed node that does not expand calls its owner's `activate`, deferred out of the explorer's update. An expandable node still toggles.
  - (2) A contributed node's menu is its owner's `context_menu`, or none when that is empty. The PG database and MySQL/MariaDB schema menus end with `database_menu_items` after a separator. The SQLite profile root gets them after its items and a separator, with a separator before `Delete…`, which stays last (T2 ruling 17).
  - (3) `contrib::contributors_changed` touches the now-public `Contributors` global, which `ExplorerView` observes. A contributor's change then rebuilds and redraws the tree with no other event.
- **Ownership.** The owner of a contributed key is the contributor that listed it under a database (the first one, if two list the same key), or the owner of the node it was listed under. Lazy children are asked of that owner only (M4 asked every contributor). No other contributor is ever asked to activate, describe or give a menu for a key it does not own.
- **Keys (R-M4b-37, controller ruling R-T2-1 (c)).**

  | Command | Context | macOS | Windows and Linux |
  |---|---|---|---|
  | `scripts.goto` | `Workspace` | `cmd-shift-o` | `ctrl-shift-o` |
  | `file.save` | `ScriptTab` | `cmd-s` | `ctrl-s` |
  | `file.save_all` | `Workspace` | `cmd-alt-s` | none (palette and menu only) |
  | `scripts.rename` | `ExplorerTree` | `f2` | `f2` |
  | `scripts.reveal`, `scripts.attach` | — | none | none |

  Move to Trash has no chord. `ScriptTab` is the active script tab's context.
- **Copy.** The mock's generated FTL goes into `## m4b-t01b-seams` of `locales/{en,tr}/hacivat-ui.ftl` as is, with two changes:
  - The two save titles take the ids the R5 rule gives their commands (`hv-cmd-file-save`, `hv-cmd-file-save-all`; the mock wrote `hv-cmd-scripts-save*`).
  - One title is added for F2 (`hv-cmd-scripts-rename`, TR marked `# Review: PO`).

  The `hui-editor-t2-*` and `hui-editor-paste-confirm-*` ids stay with Task 11.
- **Icons.** `folder` and `file-text` are new SVGs. `IconName` also gains `Download` and `FileDiff`, whose SVGs already ship.
- **Placeholders for Task 1a.** `scripts::{DatabaseName, LibraryId, PathError, RelPath}` stand in for Task 1a's `hacivat_scripts` types, which were not on its manifests commit. They copy the API Task 1a merged (`Display` and string serde for `LibraryId`, `RelPath::parse -> Result`, `components`, `DatabaseName::new`), so the controller swaps them for one `pub use` line. `scripts::parse_library_id` and `scripts::file_name` are the app's own helpers over that API. `scripts::tabs::display_name` stands in for Task 4's display escaping.

## m4b-t04 — scan and scope

Task 4 fills `pathsafe`, `scan`, `envscope`, `tree` and `dataless` (HV-SCRIPT-03, -04; R-M4b-6, -12..15, -23; Q3 option (a), Q15; alg/28 C11).

### Names (`pathsafe::component`)

- Refused on every host: empty, `.`, `..`, `/` or `\`, NUL, any control character (C0, DEL, C1), more than 255 bytes, not UTF-8. Bidi and invisible characters are allowed as names. Display escapes them.
- Refused on Windows only: `<>:"|?*` (the plan named `:`; the other six cannot be created through Win32 either), a trailing dot or space, and the device names with any extension. The device names are the plan's list plus `COM0`, `LPT0`, the superscript `COM¹²³`/`LPT¹²³`, `CONIN$` and `CONOUT$`. The stem before the first `.` is compared with its trailing spaces dropped, so `con .sql` counts.
  - On Unix these names stay ordinary. A macOS teammate's `con.sql` lists on macOS.
  - `component_on(name, Host)` runs the Windows rules on every CI runner.
- Signature: the seam's `Result<&str, PathError>` is kept. The plan's `NameProblem` is `PathError::Name(NameProblem)`, and its `PathProblem` is the other `PathError` variants. Task 1a's `PathError::NotImplemented` is gone, so a stale caller fails to compile instead of silently matching nothing.

### Joining (`pathsafe::join`)

- Every component is re-validated. The skipped ones (a leading `.`, `node_modules`) are refused (`Name(Skipped)`), so nothing opens `.git/` or `.hacivat/` through a library path. Task 3's manifest writer has its own fd-relative code.
- The root must still resolve, both as chosen (`path`) and as stored (`canonical`), to `canonical`. Otherwise the answer is `RootChanged`.
- Folders on the way are checked with `symlink_metadata`. A link or junction is `LinkOnPath`, even one that stays inside, and a file on the way is `NotAFolder`. Then the parent's `realpath` must lie under the root.
- The last component:
  - absent: the path is answered (new script);
  - a FIFO, socket or device: `NotAFile`;
  - a file or folder: its `realpath` must lie under the root;
  - a file link: answers its target only when the scan would list that target itself (a regular `.sql`/`.md` file inside the root, in no skipped folder). Otherwise `OutsideRoot`, which covers out, dangling, a loop, a folder, `.git/config`, `notes.txt` and the root itself. The link is resolved as text inside the root (fix round 1 below).
- Containment is a component-wise byte prefix of canonical paths, never a case fold. On a case-insensitive volume, `realpath` returns the stored case on both sides.
- `join` is a check, as fresh as the call. Task 5 still opens relative to a directory handle with `O_NOFOLLOW` (HV-SCRIPT-03 step 8, carries n8). Since fix round 1 it answers a `Joined` (what it resolved), and the caller re-checks scope.

### The scan (`scan::scan`)

- Through the `Fs` seam, which has `read_dir` (names, capped), `symlink_metadata`, `canonicalize` and `read_link`. It has no read method, so a scan cannot read contents by construction.
  - Fix round 1 replaced the first round's `resolve` (OS `realpath`, which walked outside paths) with `read_link`, and loops are counted as hops.
  - Tests inject an in-memory `Fs`.
- The walk is iterative, with an explicit stack. The names of each folder are visited in byte order. A stop on the file or depth limit is deterministic. A stop on the visited limit is not: since fix round 1 (m3) a folder is read only up to the cap, so which names it yields first depends on the OS's order. The output is in tree order (`tree::order`): an ancestor before its contents, folders before files among siblings, then the name's bytes.
- Not listed:
  - dot names (folders and files, C11) and `node_modules` (ASCII case-insensitive), skipped silently;
  - names that fail `component`, each with a `Diag::BadName` that holds the display form;
  - FIFOs, sockets and devices;
  - folders without a listed file below them.
- File links are resolved once:
  - `InsideLink` takes the target's metadata. Its scope is the intersection of the link's scope and the target's (`envscope::combine`). Without that, a plain `a.sql` pointing at `@staging/drop_all.sql` would show a staging-only script on production.
  - No system call names a path outside the root (fix round 1, I3; the first round's `realpath` did).
  - A link to a folder is never followed. One named like a script is listed as `OutsideLink`; any other is not listed.
- Limits are R-M4b-14 plus two the plan did not name:
  - 20,000 files;
  - depth 32: a listed entry has at most 32 components;
  - 200,000 visited names: a library full of `.txt` files also stops;
  - 256 diagnostics, with the environment diagnostics first and the rest counted in `suppressed`.
  - Any of the first three stops the whole scan where it is, with `stopped` set and what was found kept.
- `ScanResult.root` reports a root that could not be scanned: `Missing`, `Changed` (repointed, or a stored path that is not canonical) or `Unreadable`. Then nothing is listed.
- Identity is the OS bytes. `RelPath` holds exactly what `read_dir` returned (NFD on macOS included); display is `RelPath::display_text`, which is NFC plus the `<U+XXXX>` escapes.
  - **Carry for Tasks 7 and 10:** `GitPath` keys must equal these bytes.
- Dataless files: macOS `SF_DATALESS` is defined here as `1 << 30`, because `libc` 0.2.191 lacks it. On Windows, `RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN` and `OFFLINE` all count.
  - Fail closed: a file wrongly called dataless only loses its badge, while a file wrongly called local would be hashed and so downloaded.
  - The flag comes from the scan's own `lstat`. Other systems answer `false`.
- **Carry for Task 10:** `LibraryDiag` (service, Task 10's file) has no scan arm yet. Task 10 adds `LibraryDiag::Scan(scan::Diag)` and maps `ScanResult.root` to `Unavailable(Missing | RootChanged)`.

### Environment scoping (`envscope`)

- Only the five tags scope, ASCII case-insensitive (`@PRODUCTION`). No locale casing anywhere, so `@productİon` and `@productıon` are unknown under `tr_TR` too. The tests also run under `LC_ALL=tr_TR.UTF-8`.
- Unknown, and so hidden everywhere with a diagnostic:
  - any other `@name`;
  - a folder that reads as an environment folder but is not a plain `@` followed by visible text: a full-width `＠` or small `﹫` commercial at, or an invisible formatting character anywhere in it (`\u{200B}@production`, `@production\u{200B}`, a bidi override).
  - Treating those as ordinary folders would fail open.
- "Did you mean" (`EnvFolder::Unknown { suggestion }`) is computed over the visible text, ASCII-folded, in this order:
  1. an exact tag;
  2. the closest tag by optimal string alignment distance, at most 3 and at most half the tag's length, ties going to the more severe tag;
  3. a tag the text starts, given at least 3 characters (`@prod`);
  4. a tag that starts the text (`@develop`).
  - So `@x` and `@` suggest nothing, and `@stg` suggests nothing.
- Folder components only (`@production.sql` scopes nothing).
  - A folder entry is scoped by its own name too (`folder_scope`), so the `@production` row itself is hidden on staging.
  - `visible` recomputes the scope from the entry's path and also requires the stored scope, so an entry built with `EnvScope::Any` cannot widen it, and a link keeps its target's narrower scope.
- Diagnostics: one `UnknownEnvFolder { name, suggestion }` per distinct unknown name among the listed folders, and one `EnvContradiction { folder }` where a chain first becomes empty. The text comes from the UI's FTL, never from here.

### Measurements and verification

- Scan time on macOS (APFS, debug build), three runs: 10,000 files in 46 ms, 20,000 files in 91–95 ms. These are the `#[ignore = "perf"]` tests, gated on `HACIVAT_PERF`.
- The `pathsafe` fuzz target ran for 60 s on nightly-2026-11-01: 10.2 M runs, no crash.

### Named exception

`tests/seams.rs` (Task 1a's file): the stub assertions for `visible`, `pathsafe::component` and `pathsafe::join` are removed, with fenced `// ## m4b-t04-scanner` notes in their place. Their real rules are tested in `tests/{envscope,pathsafe,scan}.rs`.

### Fix round 1 (review I1–I3, m1–m9; rulings R-T4-1, R-T4-2)

- **I1, leading characters before `@`.** `classify` first skips a leading run that a person does not see: Unicode `White_Space` (NBSP, U+3000, the en and em spaces), `Default_Ignorable_Code_Point`, combining marks (`\p{M}`), controls, format, private-use and unassigned code points, and the blank glyphs (U+2800, the Hangul fillers, U+1D159).
  - The classes come from `regex`'s Unicode tables, already a dependency, so there is no manifest change.
  - A name whose first visible character is the ASCII `@` scopes when the rest is exactly a tag: ` @production` and `\u{200B}@production` are `Tag(Production)`, per the controller's ruling.
  - Anything else is unknown and hidden with "did you mean": a lookalike `@` after the blanks, a misspelt tag, an invisible character after the `@`. A visible character before the `@` (`x@`, `(@`, `é@`) keeps an ordinary folder.
  - First-round change: `\u{200B}@production` was `Unknown`; it now scopes. Both answers keep it off staging.
- **I2, `join` answers what it resolved (R-T4-2).** `join` and `join_with` answer `Joined { path, target, kind }`, where `target` is the link's target inside the library and `kind` is `Absent`, `File` or `Folder`.
  - **The caller contract** is in the `Joined` doc. Before opening, the Task 5 read and save, the Task 14 tab and the Task 15 chooser re-apply `envscope::visible_at(rel, kind, joined.target, env)`, which combines the scope of `rel` with the scope of the resolved target.
  - `join_entry(root, &ScriptEntry)` refuses with `PathError::Changed` when a scanned plain file is now a link, a link points elsewhere (`ScriptEntry::target`, new, records the scanned target), a link became a file, or a file became a folder. It refuses `OutsideLink` and `Loop` entries outright.
- **I3, link resolution is text-only inside the root (R-T4-1, which replaces the plan's "resolved once with `realpath`").** `pathsafe::resolve_link`, used by both the scan and `join`:
  1. reads the link text (`read_link`, on the link itself);
  2. walks a relative text from the link's real folder; an absolute text must begin with the canonical root, component for component, or the link is `Outside` with no further call;
  3. pops on `..`, refusing to pop above the root;
  4. `lstat`s each name inside the root, and splices in any link it meets, with at most 40 hops before `Loop`;
  5. canonicalises the link-free final path, which stays inside the root, and requires a name the scan would list.
  - On Windows a UNC, device, `\\?\`, other-drive, drive-relative or rooted text is refused from its components alone, so no SMB connection is ever made.
  - An absolute link that spells the root another way (for example `/var/…` for `/private/var/…`, or another case) is `Outside`: the rule is a text comparison with the root's spellings (re-review n2 below).
  - A link hidden by its target's scope gets `Diag::LinkScope { rel }` (m4).
- **m1, identity.** `join` canonicalises the folders on the way, and the final file or folder, and requires `inside_rel` to equal the requested `RelPath` byte for byte. Otherwise it answers `PathError::Alias`: an 8.3 short name (`GIT~1/config`, `NODE_M~1/x.sql`) or a case or normalisation variant. Scan-produced paths are the OS's stored bytes, so they always match.
- **m2, dataless folders.** A folder with the dataless flag is listed as a disabled row (`ScriptEntry.dataless = true`, `kind = Folder`), with its ancestors, and is never read. The test double fails on any `read_dir` of such a folder. Running the scan thread under `setiopolicy_np(IOPOL_MATERIALIZE_DATALESS_FILES_OFF)` would need FFI and `unsafe`, which this crate may not use. Not done.
- **m3.** `Fs::read_dir(dir, cap)` stops reading at `max_visited − visited + 1` names.
- **m5.** `ScanResult.stopped` and `.suppressed` must be shown by Task 10 (R-M4b-14). The controller routes this carry.
- **m6.** Tests named `symlink_loop_is_disabled` and `dir_symlink_is_not_followed` were added, and ADS names (`a.sql:stream`, `a.sql::$DATA`) are rows of the Windows table.
- **m7, chosen: refuse.** A library root whose canonical path has an environment folder anywhere, its own name included, is refused: `RootProblem::InEnvFolder(name)` in the scan, `PathError::RootInEnvFolder` in `join`. `envscope::root_env_folder` is public, so attach (Tasks 3, 16) can refuse before adding the library.
  - Why not scope by the ancestor: every consumer would have to carry the root's scope (the index, the chooser, `visible_at`), and one that forgot would fail open. A refusal cannot be forgotten.
  - Cost: a user who keeps libraries under `…/@production/` moves them, or attaches the parent.
- **m8.** Left as it is (plan-mandated); it is on the PO queue.
- **m9.** `lib.rs` gains a fenced `## m4b-t04-scanner` note (named exception): `visible`, `component`, `join` and `scan` are no longer stubs. T1a's bullets are untouched, so the merge with Task 3 stays clean.

### Re-review notes (n1–n4; ruling R-T4-3)

- **n1, the test guard.** The in-memory `Fs` now refuses any path whose components after the root are not plain names. A lexical `starts_with` had accepted `/lib/../elsewhere`.
  - Proof: the reviewer's mutation (`resolve_link` pushes `..` instead of popping it, so it looks up through `..`) now turns `outside_link_texts_are_refused_without_touching_outside` RED. Before this change it stayed green.
  - The real-file-system twin `a_link_out_through_dot_dot_is_outside` checks only the answer, not the lookups.
- **n2, absolute link targets on Windows (R-T4-3).** An absolute text may start with any of the root's spellings (`pathsafe::root_spellings`), compared as text, component for component:
  - the canonical root, on every host;
  - on Windows also the root as the user chose it (`LibraryRoot::path`) and the plain form of the canonical root (`C:\…` for `\\?\C:\…`, `\\server\…` for `\\?\UNC\server\…`), because `mklink` stores absolute targets as typed.
  - After a match the walk continues from the canonical root, so no lookup goes through another spelling.
  - **Remaining cost:** any other absolute text is a disabled "points outside the library" row even when it leads back in. That covers a different drive-letter case, a `subst` or mapped drive, a path through a junction or symlink outside, 8.3 spellings, and on macOS the `/var/…` spelling of `/private/var/…`. A relative link is unaffected. This is a PO-visible cost; it fails closed.
- **n3.** The "stop is deterministic" line above is corrected for the visited-limit cap.
- **n4, residual beside R-T3-1.** Two final `canonicalize` calls go through the OS: the one in `resolve_link` (on the link-free final path) and the one in `join`'s identity check.
  - Both are made only after an `lstat` walk has shown every component inside the root and free of links. If an inside folder were swapped for an outside link in the tiny window between the walk and the call, `realpath` would follow it. The swap needs a local writer racing the scan, the same class of window as R-T3-1, and the fd-relative `O_NOFOLLOW` open of Task 5 still protects the read and the save.
  - On Windows, `canonicalize` is a `CreateFileW` on an inside file, which may recall a `RECALL_ON_OPEN` placeholder. That is a download, never an outside access.
  - Not changed; recorded.

## m4b-t05 — file I/O and recovery copies

### Read (HV-SCRIPT-07 step 1; UI-EDIT-02 step 1)

- `fileio::read` sniffs a UTF-8, UTF-16LE or UTF-16BE BOM, else takes strict UTF-8, else answers `NeedsEncoding { guess: None }` (no guesser in M4b; the app asks and calls `read_with(root, rel, label)` with a WHATWG label, so `ISO-8859-9` is `windows-1254`). A BOM-less file with more than 1 % NUL in its first 64 KiB is `Binary`. Decoding never substitutes (`*_without_replacement`); line endings are never normalised, and `FileMeta::line_ending` is the dominant one (ties LF).
- `read_with` re-encodes what it decoded and answers `Lossy` when that would not give back the same bytes (Shift_JIS's duplicate NEC/IBM rows decode to one character that encodes to the other row), so a save of unchanged text never rewrites a file. `replacement` is never offered (`UnknownEncoding`); bytes that do not decode are `Undecodable`.
- Tiers (UI-EDIT-23): `tier_for(len, lines)` is Full up to 4 MiB and 100,000 lines, Reduced up to 20 MiB, Preview above; a line over 64 KiB makes a Full file Reduced. A Preview reads the first 2.5 MiB, decodes it incrementally (a character cut by the read is dropped) and ends after the last `\n` (or a `\r` that is not the last character); its stamp still covers the whole file, hashed streaming in 1 MiB reads. `ReadLimits::whole` reads up to 64 MiB as Reduced ("Open anyway").
- A file that changes while it is read (`fstat` before and after, or a length mismatch) answers `Io(Interrupted)`; the watcher reads it again once its stamp settles (C6).

### Save (HV-SCRIPT-07 steps 3–4; carries-w1 T5, n8)

- The text is encoded first; a character the encoding cannot hold is `Unencodable { first_offset }` (the byte offset in the text) and nothing on disk is touched. UTF-16 is encoded by hand, since `encoding_rs` encodes UTF-16 as UTF-8.
- With `expect`, the current stamp (read through a no-follow handle and hashed) must equal it, else `ChangedOnDisk(current)`; a file gone since is `DeletedOnDisk` (Save to recreate passes no `expect`). The file must open for writing as it is, so a read-only file is refused and never replaced by a rename.
- The bytes go to `.<name>.hv-<uuid v4>.tmp` in the same folder (a dot name the scanner never lists, C11; `<name>` cut on a character boundary so the whole name fits 255 bytes, m10; never the process id, R-T1a-4), created with `create_new` and the no-follow flags, 0600 when it replaces a file until it takes the original's permission bits (`mode & 0o777`) and owner (`fchown`), written and synced. The name is then re-checked (still the file that was checked, or still nothing), `save_with_echo` reports the new stamp (step 4: the service records the echo before the file changes), and the temp is renamed over the name relative to the folder handle, and the folder synced. Every failure removes the temp; the original stays as it was (tested with injected faults at write, sync and rename, and a read-only folder). `fileio::is_temp_name` lets the watcher recognise the transient.
- **Hard links** (alg/28 §3 item 4): a file with `st_nlink > 1` is written in place through its own handle (write, truncate to the new length, sync) after the same stamp check, so every name keeps the new bytes. The same in-place write is used when the temp cannot take the original's owner (a file owned by another user in a shared folder). That write is not atomic: an I/O error in the middle leaves a mix, while the tab stays dirty and its recovery copy stays. Windows has no link count in stable `std`, so it always renames.
- Windows: the rename retries three times (50, 100, 200 ms) on a sharing or lock violation (alg/28 edge case).

### Paths (HV-SCRIPT-03 step 8; R-T3-1)

- Every read, save and stream walks the `RelPath` from `LibraryRoot::canonical` one folder at a time with Task 3's directory-handle approach, kept as a local copy in `fileio.rs` until the controller reconciles the two (Task 3 was in review): each folder opened with `O_DIRECTORY` and the no-follow flags (macOS `O_NOFOLLOW_ANY`; Windows `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT`, held without `FILE_SHARE_DELETE`), its identity re-checked from the root down around every operation, and on Linux every operation goes through `/proc/self/fd/<n>/`. The root is opened by its canonical path and never canonicalized again, so a root that became a link is refused.
- A link or junction in a folder position, a folder, FIFO or device at the end, and a dangling, looping or outside link are refused (`PathRefused` on read, `PathChanged` on save); files are opened with `O_NONBLOCK`, so a FIFO swapped in never blocks.
- A link at the end (an `InsideLink`) is resolved once (`canonicalize`) and followed only to a regular file inside the canonical root; the target path is walked again without following anything, and the save writes the target, so the link stays.
- **Hidden components are refused** (`PathRefused`), in the path and in a link's target: a name starting with `.` is never listed (C11), and this keeps `.git/` and `.hacivat/` out of reach of a script tab (a library link `x.sql -> .git/hooks/pre-commit` or `-> .hacivat/workspace.toml` is never written through). On Windows, device names, `:` streams, reserved characters and a trailing dot or space are refused too. This overlaps Task 4's `pathsafe::component`, which is a stub on this task's base; the controller can point `check_rel` at it after both merge.
- Residual (R-T3-1, unchanged): on macOS, `rename`, `unlink` and `canonicalize` are path calls between identity checks. A local process that can already write the library could swap a folder for a link exactly between a check and the call and make the script's own text land where it chose; the after-check reports the change. Closing it needs `openat`/`renameat` (`rustix`) or an `unsafe` exception. Extended attributes, ACLs and Finder tags of a replaced file are not copied (a rename-based save drops them; the in-place cases keep them).

### Stream (`fileio::open_stream`; HV-SCRIPT-12 step 4)

- One handle for the whole run; 64 KiB reads decoded incrementally in the chosen encoding (its own BOM dropped, a character split by a read kept for the next one). After every read, the handle's and the path's `(len, mtime, dev, ino, ctime)` must equal the ones at open (Windows: `len, mtime, created`), else `ChunkError::Changed`, and nothing read after the change is handed out; bytes that do not decode are `ChunkError::Decode`. `LibraryService::open_stream` (Task 10) wires it; Task 12 builds on the stop.

### Recovery copies (HV-SCRIPT-07 step 5; R-M4b-5, Q1)

- `recovery::{store, take_all, discard}` under `LocalPaths::script_recovery_dir(<library uuid>)`: `<xxh3-128(rel path) hex>.sql` (the text, UTF-8) and a `.meta` sidecar, both through `fsio::atomic_write` (0600), in folders set to 0700 (also when they already existed). Nothing is written into a library (L14). When `script_recovery_dir` answers `None`, nothing is written or read and there is no fallback folder (`RecoveryError::NoFolder`; carries-w1 m5).
- **Deviation:** the sidecar is TOML, not JSON (`format`, `rel_path`, `saved_at_secs`/`_nanos`, and an optional `[base]` with `len`, `mtime_secs`/`_nanos` and the hash as hex), because this crate's manifest has `toml` and no `serde_json`, and no later task edits a manifest. A pre-1970 base stamp is dropped (the app then treats the file as changed). The plan's sidecar also named the encoding; it is not kept, since the app re-reads the file's meta on restore.
- `take_all` validates and does not delete (a second crash before the user answers still loses nothing): a sidecar whose `rel_path` is not library-relative or does not hash to its file name, a newer `format`, a sidecar over 16 KiB, a missing copy or one that is not UTF-8 is skipped. `Recovered`'s `Debug` shows the text's length only.

### Over-breadth carries (carries-w1 m2) and other notes

- The `process` identifier and glob imports from outside the crate are refused in this crate's non-test files: Task 5's code has neither; `tests/fileio.rs` runs `mkfifo` (Unix) and `cmd /C mklink /J` (Windows) under `tests/`, which the rule exempts. Tests import every item by name (no proptest glob, R-T1a-3).
- `fileio::faults` (feature `test-support` only) injects one failure at the save's write, sync or rename step on the calling thread; the product build has a no-op twin.
- `tests/seams.rs` (Task 1a's file) is edited as a named exception: `fileio_stubs_read_and_write_nothing` is removed (the real behaviour is in `tests/fileio.rs`), and its `fileio` import with it. `ReadError::NotImplemented` and `SaveError::NotImplemented` stay for the service stubs (Task 10).
- **Carry for Task 14:** a `Decoded` in the `Preview` tier holds only the first 2.5 MiB; its tab is read-only and must never be saved (a save with its stamp would succeed and cut the file).

### Fix round 1 (review I1–I4, m1–m12; rulings R-T5-1 to R-T5-4)

This subsection supersedes, where they differ, the bullets above on link resolution (`canonicalize`), the local `Dir` copy, the hidden-name check, the in-place write, the recovery key and the sidecar.

- **Shared folder handles (m1, m4, m12).** Task 3's `Dir` moved unchanged in behaviour into `fileio::dir` (crate-private), and `manifest.rs` uses it (named exception: its private copy and procfs probe are gone; `manifest::with_procfs_absent` is re-exported). Additions: `open_canonical` (the root by its stored canonical path, never canonicalized again), `open_write`, a `private` flag on `create_temp`, the temp predicate as an argument of `sweep_stale_temps`, and the Windows rename retry. Linux without procfs now refuses every save (`Io(Unsupported)`), as the manifest writer does.
- **Paths through Task 4 (I2, R-T5-2).** Every read, save and stream first calls `pathsafe::join`: the name rules (hidden and skipped names, Windows device names including `CONIN$`, `COM0` and the superscripts, streams, 8.3 and case aliases: m6), the root check, no link on the way, and a final link resolved as text inside the root (`resolve_link`). `fileio` itself never canonicalizes and never resolves a link; the reviewer's bounce link `l.sql -> ../out/bounce.sql -> <lib>/real.sql` is refused and `out/` is untouched (`link_that_leaves_the_root_and_returns_is_refused`). The path `join` answered (the target for an `InsideLink`) is then walked again with no-follow handles, and anything that is a link now is refused (a swap after `join`). The `_scoped` forms (`read_scoped`, `read_with_scoped`, `save_scoped`, `open_stream_scoped`) also re-apply `envscope::visible_at` to what `join` resolved and answer `OutOfScope`; **Task 14's tabs use them**. The plain forms keep the plan's signatures, for callers without a connection.
- **In place, restored (I1, R-T5-1).** The handle is re-checked against the stamp right before writing (fingerprint and a hash of what it holds now: m3), and the original bytes are kept (files over 64 MiB are not written in place: `Io(FileTooLarge)`). On any error they are written back, the mtime is set back (`File::set_times`), and the content is verified; the answer is then the I/O error, with the file as it was and the editor's stamp still valid. Only when the write-back fails too is the answer `SaveError::Damaged`; the caller then stores the new text with `recovery::CopyMeta { damaged: true }`. Tested with a `WriteAfter(n)` fault (the ENOSPC/EFBIG stand-in) on the in-place, rename and new-file paths, and a failing restore.
- **No-replace publish of a new file (m4).** A new file's temp is published with a hard link (fails when the name exists → `ChangedOnDisk` with that file's stamp) and, without hard links, with `create_new`; never with a rename.
- **Streaming stamps (m5).** The current-file check hashes in 1 MiB reads; nothing is read whole except the original of an in-place write.
- **Stale temps (m2).** Each save sweeps its folder of our own temps, exactly `.<name>.hv-<32 lowercase hex>.tmp` and older than 60 s.
- **Preview (I4).** `From<&Decoded> for SaveText` is gone; `Decoded::save_text()` is `None` for a Preview. A preview read in a chosen encoding gets the `Lossy` check (m8).
- **Recovery (I3, m7; R-T5-3, R-T5-4).** A copy is keyed by `RecoveryKey { library, rel, profile, database }` (the tab's binding) and by the writing `Instance` (a random id plus an exclusive `File::try_lock` advisory lock on `script-recovery/instances/<id>.lock`, held for the instance's life; no process id). The stem is the XXH3-128 of those parts, each length-prefixed. `take_all` offers every copy whose instance holds no lock any more (two tabs of one file come back as two entries); a running instance's copies are never offered or deleted by another; `discard` removes only this instance's copy of one key, `discard_entry` an offered one, `discard_library` a detached library's folder. Copies older than 14 days are deleted, not offered. The sidecar (format 2) records the binding, encoding, BOM, line ending, base stamp, `secret_suspected` (set by the caller's secret scan, for Task 17's notice; no redaction, since Q1 says a crash loses nothing) and `damaged`. Dead instances' lock files older than a minute are swept at start.
- **Windows residuals (m11).** `std` has no stable link count or file index (`MetadataExt::number_of_links`/`file_index` are unstable), and this crate has no `windows-sys`: a hard-linked file is replaced by a rename (the link breaks), and the stream's and the re-check's fingerprint is `(len, mtime, created)`, which NTFS file tunnelling can keep across a rename-replace with the same length and mtime. Add the file index when a manifest change is allowed.
- `lib.rs`'s Task 1a doc block gets a `## m4b-t05-fileio` note (named exception, m10).

### Fix round 2 (re-review I-fr1-1, I-fr1-2, mf1–mf5; rulings R-T5-5, R-T5-6)

This subsection supersedes the fix-round-1 bullet on the `_scoped` forms.

- **One public function per operation, the environment required (I-fr1-1, R-T5-6).** The functions are:
  - `read(root, rel, env, limits)`;
  - `read_with(root, rel, env, label)`;
  - `save(root, rel, env, text, expect)`;
  - `save_with_echo(root, rel, env, text, expect, echo)`;
  - `open_stream(root, rel, env, enc)`.

  `env: Option<EnvTag>`, and `envscope::visible_at` is applied to what `pathsafe::join` resolved. The `_scoped` variants and the unchecked scope are gone; no unscoped form exists. `None` is an untagged connection or an unbound tab, and it sees no `@`-scoped file (fail closed).

  `LibraryService::{read, save, open_stream}` take the tab's environment too (`service.rs`, a named exception; still stubs), so Task 10 inherits the shape.

  Test: `every_public_function_refuses_what_the_environment_does_not_show` (a `@production` path under staging, dev and `None`, through all five functions, plus a new file there).
- **Partial meta (I-fr1-2).** `FileMeta.partial` is set only by the preview read. `save` refuses partial meta with `SaveError::PreviewOnly` before anything else, whatever text it carries and with or without `expect`. `recovery::store` refuses it too (`RecoveryError::PreviewOnly`), since a preview tab is read-only and never has unsaved text.

  Test: `a_preview_tabs_meta_cannot_be_saved` (the reviewer's route: the buffer plus the tab meta of a 22 MB preview; the file is unchanged).
- **mf1.** A failed final `sync_all` of the write-back answers `Damaged`: a write-back that may not be durable is not a restore. Test: `failed_restore_sync_reports_damage`.
- **mf2.** Once the new bytes are on disk (renamed, hard-linked or written in place), a folder check that fails after it answers `SaveError::SavedButPathChanged(stamp)`. This is a completed save with a warning: the tab is clean at that stamp and the user is told the folder changed. It is never a plain failure.
  - The shared `Dir` gains `rename_noting_after` and `publish_link_noting_after`, which run the operation after the before-check and report the after-check instead of failing.
  - The link publish unlinks its temp in the same step, so a folder moved right after it leaves no temp (found by this round's test).

  Test: `a_folder_moved_after_the_write_is_a_completed_save` (rename, in-place and new-file paths).
- **mf3 (R-T5-5).** Where advisory locks are unsupported (some NFS or SMB homes), `Instance::start` still succeeds, unlocked (`Instance::is_locked()` is `false`), and copies are still written.
  - Their sidecars carry `unlocked = true`, so an unlocked writer's unlocked lock file is never read as "dead".
  - A copy whose writer cannot be shown dead (that mark, or a lock that cannot be tested here) is offered with `RecoveryEntry::liveness_unknown = true`.
  - **Carry to M8:** two instances that start together can both offer one dead instance's copies; the M8 single-instance lock closes that.

  Test: `without_advisory_locks_copies_are_kept_and_offered_marked` (with `recovery::with_locks_unsupported`, test-support).
- **mf4 (carry to Task 10, routed by the controller).** At launch, sweep the `script-recovery/<uuid>` folders of libraries not in the registry, past `MAX_AGE`.
- **mf5.** The Windows rename retry is a `retrying(op, retry_on, sleep)` helper. Its unit test (`fileio::dir::tests::rename_retry_waits_and_gives_up`) injects a sharing violation and records the waits: 50 + 100 + 200 ms, four tries, then the error; other errors are not retried.
  - The retry runs inside the shared `Dir::rename`, so it **also applies to Task 3's manifest rename** (`manifest::add_alias`): on Windows, an alias edit over a manifest another program holds open is retried up to about 350 ms before it fails. Nothing else in Task 3's behaviour changed.
  - It retries `ERROR_ACCESS_DENIED` too, so a permanent denial costs about 350 ms; accepted.

## m4b-t07 — git status

HV-SCRIPT-11, `crates/core/hacivat-git` (`discover`, `trust`, `status`, `map`). Showing a badge never runs a program: the hostile and FIFO tests under `crates/core/hacivat-git/tests/` run the real call path under a spawn detector on every OS.

### The pipeline (all on one timed worker)

- **One worker per call.** Discovery, the layout reads, the open, both status halves, the attribute probe and the mapping all run on a dedicated thread (n4). The caller waits at most `GitLimits::timeout` (5 s). At the deadline it answers `Unavailable(Timeout)`, sets the interrupt flag and returns without joining the worker: an untracked FIFO named `.gitignore` blocks gix's open forever (`alg/28` §0.8, m3). While that worker lives, a new call for the same library root answers `Unavailable(Timeout)` at once and starts nothing. So at most one stuck worker exists per library, keyed by the root as the caller passes it (Task 10 passes a stable path).
- **Discovery** walks up from the canonical library root to the first `.git` entry, metadata only. It never considers the home directory or anything above it, and stops where `st_dev` changes. On Windows, where `std` has no device id, the path prefix stands in. A FIFO, socket or device named `.git` is not a marker. `gix::discover` is not used, because it honours environment overrides.
- **Before the open** (`trust::check_layout`):
  - Ownership (Unix, via `gix::sec::identity`): the worktree, `.git`, the resolved git dir and common dir, and every alternate must be the user's, else `Untrusted(ForeignOwner)`.
  - A symlinked `.git` is refused, and so is a plain `.git` directory with a `commondir`: `Unavailable(Redirected)`.
  - A gitfile is followed only to:
    - a linked worktree's git dir `<common>/worktrees/<name>`, whose `commondir` must lead back to `<common>`;
    - a submodule git dir under an enclosing `.git/modules/`;
    - a directory inside the worktree.
  - Alternates are followed when each line names an existing directory the user owns, chained at most four deep; the quoted form is not followed.
  - So linked worktrees and `clone --shared` repositories are read, under the same isolated, reduced-trust open (n2), and anything else is `Redirected`.
  - `extensions.objectFormat` other than `sha1`, and `extensions.refStorage` other than `files`, are read raw from the common dir's config and answer `Unavailable(Unsupported)` (m2).
- **The open:** `gix::open_opts(worktree, Options::isolated().with(Trust::Reduced))`. Afterwards (C1, I1):
  - `git_dir_trust() == Reduced`, and no section of source kind `Repository` has full trust, else `Unavailable(TrustPromoted)`.
  - The git dir, common dir and workdir gix resolved must equal what `check_layout` accepted, else `Unavailable(Redirected)`. This also makes a fully trusted open that honoured `core.worktree` fail closed (mutation M1 of the report).
- **The status (C12, n3):**
  - `index_worktree_status` with `FastEq`, our `NoopSubmodules` (records the gitlink, answers "no change", opens nothing) and `EmissionMode::Matching`.
  - Then `tree_index_status` with `TrackRenames::Disabled`, so no similarity work runs and no budget comes from repository config. A staged rename's destination is an Addition, so it is New, as before.
  - Never `Repository::status()`. Swapping gix's `BuiltinSubmoduleStatus` back in made the nested default-options open read a system-config FIFO, and `submodule_entry_reads_no_global_config_and_spawns_nothing` failed (mutation M4).
  - The pathspec is `:(top,literal)<prefix>`.
  - `NeedsUpdate` and deletions give no badge. `Outcome::write_changes` is never called, so the index bytes are unchanged (`index_is_never_written`).
  - More than `max_entries` paths answers `Unavailable(Failed(…))`, never a partial map: a missing badge would read as clean.
- **Unknown overrides** win over every other badge. `Unknown` is ranked Submodule > Dataless > Filtered, then Conflicted > New > Modified.
  - **Filtered (C2):** our own probe, `attributes_only` plus `selected_attribute_matches(["filter"])`, runs over every tracked path and every changed one. `filter` set or valued gives `Unknown(Filtered)`, even when the path is clean, since an LFS pointer's bytes are not what the index compares. `-filter` does not count. A failed lookup counts as filtered.
  - **Submodule:** every gitlink under the prefix, what the no-op handler saw, and an untracked nested repository.
  - **Dataless:** `SF_DATALESS` (macOS), `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | …_RECALL_ON_OPEN` (Windows), from `lstat`.
- **Keys** are library-relative and spelled as the disk spells them, byte for byte, so they equal the scanner's `RelPath` (R-T1a-2). Each component is matched against its directory's listing, exactly first and then by NFC equality (`gix::utils::str::precompose`): macOS keeps an NFD name while the index holds NFC (`nfd_named_modified_file_has_badge`). A path not on the disk has no badge, nor has a name that is not UTF-8.
- Error texts say "repository", never the bare word (R-T1a-1). Nothing is logged.

### Deviation: dataless paths leave the index copy, not the pathspec

`alg/28` HV-SCRIPT-11 step 6 named `:(exclude,top,literal)` pathspecs. With 20,000 of them the walk became quadratic and ran past the 5 s budget (`status_with_20k_excludes_under_1s`, U-05).

Task 7 instead removes those entries from an in-memory copy of the index (`State::remove_entries`, plus the cache-tree extension), which is never written. gix then neither compares nor hashes them. The walk sees an untracked name (an `lstat`, no open), and the precedence turns that back into `Unknown(Dataless)`.

- Measured on macOS, debug build: 297 ms for 20,000 dataless paths, 131 ms for a 10,000-file status with 100 changes.
- [Observed]: such a path is never read (`excluded_path_is_never_opened_by_status`: a file that must be hashed is unreadable; not flagged, the call fails with `Unavailable(Failed)`; flagged, it is `Ready` with `Unknown(Dataless)`; a mode-000 file on Unix, a share-nothing handle on Windows, n7).
- [Inferred]: that this keeps the OS from downloading the file (n6).
- `GitError` has no `Io` variant. A whole-call I/O failure is `Failed(<message>)`, and the message never holds file contents.

### Test seam

`hacivat_git::testing::status_treating_as_dataless` (doc-hidden) adds library-relative paths to the dataless set. It can only add `Unknown(Dataless)` answers and index removals, so it cannot weaken a status. It exists because no test can make a real cloud placeholder.

### Rejected

- `:(exclude)` pathspecs for dataless paths: quadratic, as above.
- Running `git status` with protective flags: Q4, ADR D2.
- `parallel`: R-T0-1 holds; 10,000 files take 131 ms without it.

### Fix round 1 (review I1–I5, m1–m10; rulings R-T7-1..3)

**One normal form for comparing (I1).**
- The library prefix and the index paths are compared component by component in NFC.
- macOS keeps an NFD folder name; under `core.precomposeUnicode` the index holds NFC. Before this fix, a Turkish `çalışma/` library answered an empty, confident "clean".
- The pathspec carries the prefix in the disk's spelling, in NFC and in NFD (their union), so it matches the index's spelling whichever it is.
- Keys are still the disk's bytes.

**Case-only differences (I2, R-T7-1).**
- On a file system that folds case (`core.ignoreCase`, as gix reads it), an index path that differs from the disk only by case is `Unknown(CaseMismatch)`, keyed by the disk's spelling. It is never dropped.
- The disk entry is found through the file system's own lookup. On Unix the name is identified by `(dev, ino)` (no casing code). On Windows, which has no stable file id in `std`, it is the unique ASCII case-insensitive NFC match; failing that (a non-ASCII case-only rename), the badge is keyed by the index's spelling, which no scanned row has, so in the UI it is in effect dropped (corrected in fix round 2, n3; a recorded Windows residual).
- A library folder that matches its index spelling only by case marks every path under it `Unknown(CaseMismatch)`.
- Only changed paths are mapped, so a clean, case-renamed file has no badge (the CLI shows none either).
- The differential oracle no longer shares the product's listing rule:
  - it keeps the CLI's own spelling;
  - it drops a path only when the OS's `lstat` cannot find it;
  - it compares through NFC plus an explicit case-only rule;
  - a separate assertion checks that every key is the disk's spelling.

**Bounded reads, so an abandoned worker stays cheap (I3, R-T7-2).** gix's `ref_contents` follows a symlink and reads with no limit. Before the open, these files must be regular files (not a symlink, FIFO or device) under their caps; otherwise `Unavailable(Redirected)`, or `Failed(TooLarge)` when over the cap:

| File | Cap |
|---|---|
| `HEAD` | 4 KiB |
| every loose ref on its symbolic chain, under the git dir and the common dir (folders on the way must be real directories; names must stay under `refs/` with no `..`; at most 5 hops) | 4 KiB |
| `packed-refs`, `info/exclude`, `info/attributes` | 1 MiB |
| the index | 256 MiB (it can legitimately pass 1 MiB; the cap only bounds a hostile one) |

- **Nested repositories** inside the library: gix's classification reads their `HEAD` with no limit. Hacivat's own pre-walk of the library (names and `lstat` only) therefore requires each nested `.git` to be one of:
  - a real directory with a regular `HEAD` under 4 KiB;
  - a regular gitfile under 4 KiB whose target's `HEAD` is the same.
  A symlinked `.git` is refused.
- The pre-walk enters no symlinked folder, no `.git` and no nested repository. More than 200,000 entries, or a folder deeper than 64, answers `Failed(TooLarge)`.
- These checks run after the format check, because a reftable repository keeps a file at `refs/heads`.
- **Residual:** a swap between the check and gix's read (a concurrent writer inside the user's own repository) is not closed.
- Measured on macOS, debug build: 135 ms for 10,000 files and 325 ms for 20,000 dataless paths, including the pre-walk.

**Placeholders among the files gix reads (I4, R-T7-3).**
- If a `.gitignore`, `.gitattributes` or `.gitmodules` in the library, in a folder between the worktree and the library, or in `info/exclude`/`info/attributes` is a cloud placeholder, it is not opened, since opening downloads it.
- The library answers `Unavailable(Dataless)`, the diagnostic for that case. Tracked and untracked files are covered alike, because the pre-walk sees both.

**No-spawn proof on Linux (I5).** The `build-test` job installs `strace` on Linux (`# ## m4b-t07-git`, named exception). With `CI` set, the hostile and positive-control tests fail when no spawn detector ran. The positive control asserts whenever a detector runs.

**Minor items:**
- **m1.** The one-stuck-worker cap is keyed by the canonical library root. The caller's spelling (a symlink alias, `..`) no longer matters.
- **m2.** A call while a stuck worker lives answers `Unavailable(Paused)`: "status is paused: a file in the repository does not respond". The call that hit the budget answers `Timeout`. The pause lasts until the blocked read returns or the app restarts, because the worker is never joined. Task 13 shows the notice text from FTL.
- **m3.** `Failed` carries a fixed `FailKind`: `LibraryUnreadable`, `PathNotUtf8`, `OwnerUnreadable`, `Open`, `Index`, `Worktree`, `Head`, `Setup`, `TooManyEntries`, `TooLarge` or `Worker`. No gix text is kept, since a config parse error can quote config bytes.
- **m4.** A repository config that cannot be read (missing, not regular, over 1 MiB, unparseable, or the same for `config.worktree`) is `Unsupported`, fail closed.
- **m5.** An untracked FIFO, socket or device is `Unknown(NotAFile)`. gix emits none of them, so the pre-walk records them.
- **m7.** The FIFO-based global-config tests are Unix-only, because Windows has no FIFOs. On Windows the submodule and global-config vectors are covered by `hostile_repo_runs_no_program`: it holds a submodule and a hostile `HOME` and runs inside the Job Object, which counts any started process.
- **m8.** The hostile fixture holds a live filter vector: a same-length `b.sql` with a touched mtime, so gix must hash it through the pipeline. It answers `Unknown(Filtered)`, and no marker appears.
- **m9.** The doc-hidden `pub mod testing` is product API by necessity. A `testing` feature would need a manifest edit (Task 1a's). It can only add `Unknown(Dataless)` answers and `Unavailable(Dataless)` refusals.
- **m10.** Ownership is checked again, after the open, on the worktree, git dir and common dir that gix opened.
- **For the Task 1a owner (m6).** `BackendInPublicApi` counts the `_` of `use … as _` as a backend name, so a private impl header with `'_` reads as public API. This crate names its lifetimes instead; the rule could skip `_` imports.

### Fix round 2 (re-review F1–F3, n1–n6; ruling R-T7-4)

**Control files are capped too (F1, completes R-T7-2).**
- gix-glob reads a `.gitignore` or `.gitattributes` whole, with no cap, and gix's allocation limit does not cover it. A 300 MiB sparse `.gitignore` used to cost 621 MB of memory.
- So every `.gitignore`, `.gitattributes` and `.gitmodules` that gix would read, in the library and in each folder from the worktree down to it, must be a regular file of at most 1 MiB:
  - an oversize file answers `Failed(TooLarge)`;
  - a FIFO, socket, device or symlink answers `Unavailable(Redirected)` at once. A FIFO `.gitignore` no longer hangs a worker at all.
- These are `lstat` checks; nothing is read.

**The caller does no file-system work (F2).** Path resolution can block without limit on a stale network mount, so it never runs on the caller.
- The caller claims the library under the spelling it was given and waits at most the budget.
- The worker resolves the path, then claims the canonical library too:
  - if another worker holds it, the answer is `Paused` (m1: an alias reaches the cap once its own worker has resolved it);
  - both claims are released when the worker ends.
- The tests stall the worker through the doc-hidden `testing::status_with_stall`, before or after the resolution. That hook can only delay a status.

**Never download, inside `.git` too (F3, R-T7-4).**
- Before the open, Hacivat `lstat`s the files below. If any is a cloud placeholder, the library answers `Unavailable(Dataless)` and nothing is opened:
  - `HEAD`, `config`, `config.worktree`, the index, `packed-refs`, `info/exclude` and `info/attributes`;
  - the `objects/pack` folder and every entry in it;
  - every loose ref on HEAD's symbolic chain;
  - a main or nested gitfile;
  - a nested repository's `HEAD`.
- ~~**Residual:** loose objects are not checked (that would be thousands of `lstat`s per status).~~ Corrected in fix round 3 (G2): the premise was wrong. gix reads loose objects on every status (the HEAD commit and every tree reachable from it), so they are now checked; see `### Fix round 3`.

**The index ceiling is gix's (n1).**
- Under reduced trust, gix refuses an index whose decoded entries (`entries × size_of::<gix::index::Entry>()`) exceed `gitoxide.objects.allocLimitIfReducedTrust`.
  - That limit defaults to 16 MiB. The repository cannot raise it: the key is read from trusted sections only, and `GIT_ALLOC_LIMIT` is denied.
  - Hacivat does not raise it either. A larger budget would let an abandoned worker hold more.
- `INDEX_MAX` is now the same 16 MiB. A pre-check reads only the index's 12-byte header and refuses above that limit, so a library inside a bigger repository answers the `Unavailable(TooLarge)` diagnostic, never a silent `Failed(Index)`.
- **Measured** (`index_entry_ceiling_is_measured_at_the_boundary`: a real index, built with `update-index --index-info`, macOS, gix 0.90.0):
  - `size_of::<Entry>()` is 80 bytes;
  - **209,715 tracked entries** answer `Ready`;
  - 209,716 answer `Unavailable(TooLarge)`.
- That is the ceiling on tracked files in the whole repository, not only in the library. A library's own 20,000-file limit (R-M4b-14) is far below it.

**Case-only rename of the library folder, beyond ASCII (n2).**
- A leading index folder that names the same directory as the library by `(dev, ino)` (`Çal` in the index, `çal` on disk) is an alias of the library prefix:
  - its pathspec is added;
  - its paths are `Unknown(CaseMismatch)`, never `New`.
- This is Unix only. On Windows it stays the residual described in fix round 1.

**Bounded costs (n4).**
- The pre-walk counts entries while it lists them, so one huge folder is never buffered whole.
- The case-only lookup reads each folder's `(dev, ino)` table once, instead of one `lstat` per name per path.

**Dataless folders are listed by nobody (n5, as T4's scanner).** The pre-walk does not enter them. The status:
- excludes them from gix's walk with `:(exclude,top,literal)`;
- drops their tracked entries from the in-memory index copy;
- gives them no badge.

More than 256 dataless folders answer `Unavailable(Dataless)`, which keeps the exclusion list short and its matching linear.

**CI (n6).** The `strace` install retries: three `apt-get update` attempts, `Acquire::Retries=3`, and `--no-install-recommends`.

**Cost.** Measured on macOS, debug build: 147 ms for a 10,000-file status and 339 ms for 20,000 dataless paths, pre-walk and checks included.

### Fix round 3 (re-review G1, G2, p1–p3; rulings R-T7-5, R-T7-6)

**No replace-ref scan (G1, R-T7-5).**
- Under reduced trust, gix's open still scans `refs/replace/**`: `core.useReplaceRefs` defaults to on and can come only from trusted sections.
  - It reads every entry whole, with no cap.
  - It walks a symlinked `refs/replace` root out of the repository. A link to `~` would read, and download, the whole home folder.
- The status now opens with `config_overrides(["core.useReplaceRefs=false"])`. That is an API source of kind `Override`, which gix trusts, and which is not a repository section, so both trust invariants still hold. The scan never runs.
- A `refs/replace` that exists but is not a real folder (a symlink, a file) answers `Unavailable(Redirected)` before the open.
- **Cost:** no CLI parity for `git replace`. A replaced commit is seen as the original, as the CLI shows it under `GIT_NO_REPLACE_OBJECTS=1`.
- Test `replace_refs_are_never_scanned` (P11) proves it functionally: `git replace HEAD side` would make a status that honours replace refs see `a.sql` as staged-modified, and ours stays clean. A 200 MiB file sits beside the ref, and a symlinked `refs/replace` is refused.

**Loose objects are checked (G2, R-T7-6).**
- gix reads loose objects on every status: the HEAD commit, every tree reachable from it, and index blobs for a deleted `.gitattributes` or `.gitignore`.
- Before the open, Hacivat `lstat`s these in the repository's object folder and in each alternate's:
  - the `pack` folder and its entries;
  - every two-hex-digit fan-out folder and its entries.
- A placeholder answers `Unavailable(Dataless)`.
- More than **20,000** loose entries per object folder answers `Unavailable(TooLarge(LooseObjects))`, with the diagnostic "too many loose objects; run git gc".
  - `git gc --auto`, run by `git commit`, keeps loose objects near 6,700, so the cap is rarely met.
  - A repository that has auto gc turned off and many unpacked commits is refused rather than half-checked.
- `GitError::TooLarge` now carries `TooLargeWhy::{Index, LooseObjects}`.
- Tests: `dataless_loose_object_is_unavailable` (P10: the HEAD commit, the root tree, a fan-out folder) and `too_many_loose_objects_is_unavailable`.

**Layout files before they are read (p1).**
- The main gitfile, a plain `.git`'s `commondir`, a linked worktree's `commondir` and `gitdir`, `objects/info/alternates`, and each alternate's objects (packs and loose, same cap) are checked for placeholders in `check_layout`, before it reads them.
- A split index's `sharedindex.*` must be a regular file under the index cap: a FIFO answers `Redirected` and never hangs a worker. It is also checked for placeholders.
- Test `dataless_layout_files_and_split_index_are_unavailable`.

**Index file cap (p2).**
- `INDEX_MAX`, the file size, equals gix's 16 MiB allocation limit.
- The measured 209,715-entry ceiling holds for short paths (`d<n>/f<n>`). An index whose paths are long reaches 16 MiB on disk with fewer entries, so it is refused below 209,715. That is conservative: gix's own path-storage estimate would refuse it too.

**Test hooks fenced (p3).**
- A `check-deps` rule `GitTestingHookOutsideTests` refuses `hacivat_git::testing` in any file outside `crates/core/hacivat-git/tests/`, through renames, groups and globs. It is a named exception in `xtask/src/check_deps.rs`, inside `// ## m4b-t07-git` blocks.
- Test `git_testing_hooks_outside_its_tests_are_flagged` covers six forms, plus a clean file and the crate's own `tests/`.
- So the doc-hidden hooks stay test-only, whoever depends on the crate.

**Cost.** Measured on macOS, debug build: 170 ms for a 10,000-file status (its 10,100 loose objects are checked) and 345 ms for 20,000 dataless paths.

## m4b-t06 — secret-literal scanner

HV-SCRIPT-10; R-M4b-17, -28; alg/28 C9.

- **Two inputs, one crate boundary (R-M4b-28).** `scan_secrets(text, history_spans)` takes the history redactor's spans as an argument; the app passes `hacivat_exec::history::secret_spans(text)` mapped one to one onto `SpanKind`. `hacivat-scripts` still never depends on `hacivat-exec` (dev-dependency only, for the tests).
- **Value positions only (C9).** `secret_spans` is built from `scan_into`, the scan `redact` itself runs, so every range the history masks lies inside a span (`secret_spans_cover_what_redact_masks`, and a property test in `redact.rs`).
  - Each mask now records a role: value position, comment value, connection string, meta-command line, or armed. `redact` and `scrub` ignore the role, and their output is unchanged.
  - A value counts only when the word before it names the value itself: its last `_` part is the secret part (`PASSWORD`, `MASTER_PASSWORD`, `client_secret`, `api_key`, `@pw`). A word that only mentions a secret does not count, so `WHERE password_hash = 'x'` and `passwd_changed = now()` stay quiet. They are still masked in history.
  - `IDENTIFIED … BY`, crypto-function arguments, `PRAGMA key`, `ATTACH … KEY` and `INTO @pw` always count.
  - Inside a connection string or a psql meta-command line, the `password=` values, the secret pairs, the `Bearer` credentials and the URI password come back as `ConnectionValue` spans, never the whole string. So `'the password hash'` is quiet.
  - Code quoted in code (a dollar body, or a string with doubled quotes) is read again, up to the redactor's depth of 3, and only its quoted values count.
- **Not reported.** Empty and all-`*` literals; bind parameters (`:pw`, `$1`, `?`, `@pw`); templates (`${…}`, `{{…}}`, `<…>`); a bare `NULL`; a bare word that calls a function. A literal default such as `'changeme'` is reported.
- **Token formats** live in `crates/core/hacivat-scripts/data/secret_patterns.toml`.
  - Each row has `id`, `kind`, `regex`, `source` (an alg/28 S9–S16 document) and an optional `check`.
  - The rows cover AWS, the GitHub prefixes and `github_pat_`, Slack, Stripe live keys, Stripe test keys as `TestKey`, the Google API key shape, PEM private-key labels, JWTs (the header must decode to a JSON object naming `alg`), and URI userinfo passwords.
  - Publishable Stripe keys are never reported. There is no entropy rule (R-M4b-17).
  - The patterns are written in our own words from those documents; no scanner's rule file was read.
- **Linear time, no locale casing.** The regexes are compiled once and run on the `regex` crate, which never backtracks. Lines are found by a binary search over the newline offsets, and overlapping findings of one kind are merged. Casing is ASCII only.
  - Measured on 2 MiB in a release build: `scan_secrets` takes 3 ms; `secret_spans` takes 66 ms, all of it the redactor's two readings.
  - The app runs both off the UI thread, so the UI-thread share is 0.
- **Allow marker.** A line whose comment holds `hacivat:allow-secret` drops its own findings; fix round 1 (m1) made "comment" exact (below). Known limit: a block comment opened on an earlier line does not carry the marker forward.
- **Fail closed.**
  - A pattern table that fails to parse, has a bad row, a duplicate id, an uncited source or an unknown key answers one `Unscanned` finding over the whole text.
  - A span outside the text, or one that cuts a character, is reported with its range clamped, never dropped.
- **Never the value.** `SecretFinding` holds a range, a zero-based line (`\r\n` counts once) and a kind. Nothing in the scanner logs; tests check `Debug`.
- **Named exceptions.**
  - `redact.rs`: the role field, set at every mask call site, and the `named` scan flag set where `Expect::Value` is set, all behaviour-neutral for `redact`/`scrub`.
  - The two stub tests that pinned the fail-closed stubs now pin the filled shape: `hacivat-scripts/tests/seams.rs` `secret_scanner_seam_reports_values_only` and `hacivat-exec/tests/m4b_stubs.rs` `exec_secret_spans_tell_value_from_armed` (renamed in fix round 1, m9).
  - `hacivat-exec/tests/history.rs`: new cases only.

### Fix round 1 (review I1, I2, m1–m11; controller rulings)

- **Byte identity, kept as a test.** `redact.rs` carries M3's redactor verbatim as the test-only module `redact/m3_base.rs`. `redact_is_byte_identical_to_m3` (SQL-shaped vocabulary) and `…_on_any_text` (proptest, 3,000 cases each) plus a fixed corpus compare `redact` and both `scrub`s with it.
- **One behaviour change, found by that test: a vertical tab.** `u8::is_ascii_whitespace` leaves out 0x0b, so a vertical tab outside quotes made an empty token, and M3's scan never returned. The history writer and the secret scan spun forever on such text. The whitespace run now takes 0x0b too.
  - M3 never returned once a vertical tab started a token, so on every text M3 returns on, this line is never reached and the output is byte-identical.
  - `a_vertical_tab_never_hangs_the_scan` and `a_vertical_tab_is_white_space` guard it.
  - The controller rules whether this M3 fix stays here (see the report).
- **I1, comment prose.** `Mask` records the depth of the scan that made it. A bare-word value found in a comment body or in quoted code is `Armed`; this includes a word one reading masked inside a comment while the other read it as code (`# password reset flow`). Quoted values, `password=` values, URI passwords and tokens in comments still count. `-- IDENTIFIED BY tiger` is now quiet (alg/28 [Known gap]).
- **I2, pair names.** A `password=`/JSON/`Bearer` value inside a string, comment or meta-command line is a `ConnectionValue` only when its pair's name names the value (`last_part_names_a_value` over the name's `_`/`-`/`.`/camelCase parts: `password`, `Pwd`, `apiKey`, `aws_secret_access_key`, SQLite `?key=`). `password_changed_at`, `password_policy`, `password_min_length` and `password_hash` stay quiet. URI passwords and `Bearer` credentials always count.
- **m2, crypto keys.** A call records its key arguments, so only the key or password position (at the call's own depth) is a `Value`: `crypt` argument 1, `pgp_pub_decrypt` arguments 2 and 3, argument 2 of the others. Data, options and nested calls (`gen_salt('bf')`) are `Armed`, and history still masks them all.
- **m1, allow marker.** A linear lexer in `secrets.rs` reads the text twice: MySQL (backslash escapes, backquotes, `#`, `--` before space, non-nesting blocks, `/*!` as code) and standard (nesting blocks, `$tag$` bodies). The marker counts only inside a comment that starts on its line in both readings, so a `#` comment never silences (MySQL only), and neither do markers in strings or after `#>`.
- **m3.** Overlapping findings merge across kinds, keeping the most specific (private key > token > JWT > test key > connection string > password).
- **m4, m5.** The AWS tail is base32 with a digit (`aws_shape`; about 4 % of real ids, all letters, are missed). Slack tokens start their first run with a digit and are at least 24 characters (`min_len`, [Inferred]).
- **m6.** More placeholders are quiet: `%s`, `%(name)s`, `$NAME`, `{name}`, `${{ … }}`, `{{ … }}` with spaces or dots, `[REDACTED]`. A quoted `'null'` is a literal and is reported (m11).
- **m7, m8.** New rows: `pgp-private-key-block` (alg/28 S21, RFC 4880 §6.2) and `sql-server-sp-addlogin` (S22; kind `password`). The loader now pins sources to alg/28 S9–S16, S21 and S22. Split tokens, positional `INSERT … VALUES` and comment prose are alg/28 [Known gap]s. The EXECUTE IMMEDIATE history gap is M3's and goes to the PO list, not touched here.
- **m9, m10.** Stale test names and docs are fixed (`lib.rs` doc line, `m4b_stubs.rs` module doc). JWT fixtures are joined from parts.

### Fix round 2 (re-review I-fr1-1, n1–n6)

- **I-fr1-1.** A name whose last part *ends with* `PASSWORD`, `PASSWD`, `PWD` or `SECRET` names the value (`PGPASSWORD=`, `sslpassword=`, `dbpassword=`). Parts are compared uppercased, so ASCII case does not matter. `passwordhash`, `password_hash`, `password_policy` and `password_file` stay quiet.
- **n1, a `#` glued to a name.** MySQL does read `app#1 …` as a comment: "From a `#` character to the end of the line", and `#` is not an unquoted-identifier character (MySQL 8.4 Reference Manual §11.7, §11.2). The warning still reports the standard reading's value there (`CREATE USER app#1 IDENTIFIED BY tiger`): the cross-reading downgrade of I1 skips a line whose first `#` is glued to a name, because the safe answer is to warn. One pass builds a per-line table, so the check stays linear. The check looks only at the **first** `#` on the line (p2). A line such as `SELECT '#'; ALTER USER u IDENTIFIED BY tiger # note` can therefore misjudge which `#` opened MySQL's comment, and in each such case it errs towards a warning, never towards silence.
- **n2.** `{…}`, `{{…}}`, `${…}` and `${{…}}` are placeholders only when they hold name bytes (`[A-Za-z0-9_.-]`, spaces around allowed). `'{s3cr3t-P@ss!}'` reports, while `'{hunter2}'` reads as a name and stays quiet. `<…>` stays broad, as alg/28's `'<password>'` asks, so `'<P@ss w0rd>'` is quiet.
- **n3, identity tests.**
  - Proptests: the vocabulary and `\PC` identity proptests now run 20,000 cases each, and the vocabulary adds NUL, 0x01, 0x1f, FF, CR, DEL, NBSP, U+0085, U+3000, U+FEFF and more masked-path words.
  - Fixed generator: `redact_is_byte_identical_to_m3_with_control_bytes` runs 20,000 cases over statement pieces and every control byte but VT. It asserts that at least a quarter are masked; measured: 15,204.
  - Cost: about 110 s of debug test time at 20,000 cases.
  - Gating (p3): the two identity proptests default to 4,000 cases and run 20,000 when `HACIVAT_IDENTITY_FULL=1`. The test reads that variable itself, because `with_cases` overrides `PROPTEST_CASES`. `ci.yml` `build-test` sets the variable only on the nightly schedule (`docker-full` runs only `--ignored` tests, so it never reaches these). The fixed 20,000-case control-byte test always runs.
- **n4.** The allow marker needs a space after `--` (MySQL's rule). alg/28 step 1 and the two hint strings (`hv-scripts-secret-gutter-hint`, `hv-scripts-secret-allow`, en and tr; a named exception in T1b's FTL block) say so. The design mock's copy tables keep the old wording until the controller syncs them.
- **n6, accepted.** `EXEC sp_password 'old', 's3cret', 'l'` reports only `'old'`. The redactor arms on `SP_PASSWORD`, and its first value takes the value position. The line is still flagged, which is all the save dialog needs.
- **n5** (non-ASCII spaces, a lone CR in a meta line, `Bearer` before a TAB) is pre-existing M3 behaviour and is on the PO list; it is not touched.
