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
- **Residual:** loose objects are not checked (that would be thousands of `lstat`s per status).

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
