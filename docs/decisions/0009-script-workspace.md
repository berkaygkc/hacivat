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
