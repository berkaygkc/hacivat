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

## m4b-t09 — watcher

The event half of HV-SCRIPT-08 (`crates/core/hacivat-scripts/src/watch/{mod,debounce,poll,limits,echo}.rs`). The tab half (reload, bars) is Task 14's; the service that owns watchers is Task 10's.

### Sources (D3, R-T0-2)
- `OsEventSource` is the platform's recommended watcher behind `new_debouncer_opt::<_, RecommendedWatcher, NoCache>(150 ms, None, handler, NoCache, watcher_config())`, `watcher_config()` being `Config::default().with_follow_symlinks(false)`. Nothing is cached, so nothing walks a link. Each debounced event becomes one path hint per path; an event flagged `Flag::Rescan` (FSEvents `MustScanSubDirs`, inotify `IN_Q_OVERFLOW`) becomes a rescan hint; a `MaxFilesWatch` error becomes `Lost`; other errors become a hint on their path, or a rescan when they have none. Event kinds are never used (alg/28 C3). Its `Drop` stops the debouncer and joins its thread.
- **`PollEventSource` is Hacivat's own, not `notify`'s `PollWatcher` (a deviation from the plan's sketch).** In `notify` 8.2.2 the poll watcher keeps modification times in whole seconds and reports a file only when the time grew (`poll.rs:83`, `:366`, `:410`), so an edit made in the same second as the previous pass is never reported. Task 9's `polling_fallback_sees_a_change` failed with it [Observed]. It also walks `.git` and `node_modules` on every pass. Instead, every 2 s the source hints the root, and the reconciler answers a root hint with one scan (HV-SCRIPT-03: names and `lstat` only, never through a directory link, skipped folders not entered), compared by length and full-precision time. No watcher constructor is called for polling, so the `#[expect(clippy::disallowed_methods)]` exception the carry anticipated is not needed and `clippy.toml` is unchanged. Residual: a same-length edit within one tick of a file system with coarse times (FAT: 2 s; some SMB servers: 1 s) is not seen until the next edit; seeing it would mean reading every file.
- **Backend choice** (`limits::backend_for`, R-M4b-14): a network file system is polled. A Linux library whose folder count is more than half of the free inotify watches is polled too; the free count is `max_user_watches` minus the `inotify wd:` lines in `/proc/<pid>/fdinfo` of the processes we can read. A source that fails to start, or reports `Lost`, is replaced by the poll source. The watcher then says `BackendLost` and rescans; a loss reported by the fallback itself keeps the fallback.
- **Network detection is Linux-only ([Unknown] U-09 on macOS and Windows).** Linux reads `/proc/self/mountinfo` (the longest mount point holding the root; NFS, SMB/CIFS, AFS, 9p, Ceph, Gluster, Lustre, GPFS, davfs and the FUSE network mounts). macOS (`statfs` `f_fstypename`) and Windows (`GetDriveTypeW`) need a raw call, which this crate cannot make without `unsafe`. There it answers `FsKind::Unknown`, treated as local, so a library on a share is watched by FSEvents or `ReadDirectoryChangesW`, which see this machine's own writes only. Carry: the PO decides between a manual "poll this library" setting (Task 16) and a `rustix`/`windows-sys` dependency later.

### Reconciliation (alg/28 C3–C6, HV-SCRIPT-08 steps 4–7)
- A hint is only a path. A known file is looked at with `lstat` through `pathsafe::join_with`, which re-validates every folder on the way, so a folder replaced by a link is never looked through. A folder, a link, the root, a new script name, or a known file that stopped being a plain file is reconciled by one scan of the library, diffed with what is known. Folder moves and deletes arrive as a single hint on the folder; this covers them.
- **Dropped hints:** paths outside the root; any skipped component (dot names, `node_modules`), so editor temps and Hacivat's own `.<name>.hv-<random>.tmp` never count; other extensions; more than 32 components; non-UTF-8 names. A hint on `.git/{index,HEAD,packed-refs,refs}` is `RepositoryChanged`, once per tick.
- **Names in another normal form:** a hint whose bytes do not match a known path is matched through its NFC form (macOS keeps NFD names, and an event may carry the other form).
- **Settle:** a candidate is reported only when two looks one window (150 ms) apart agree, so a half-written file is not reported. After ten looks it is reported as `Unsettled`. A removal settles too: a save seen mid-rename (the file briefly gone) stays one `Changed`.
- **Bounds:**
  - more than 1,000 hints in one window, or more than 10,000 queued, is one `Rescan`, and a `Lost` survives an overflow;
  - a scan that hits a limit, or finds the root gone or moved, is a `Rescan`, never a list of removals;
  - undelivered events beyond 10,000 collapse into one `Rescan`;
  - the receiver is a bounded `tokio::sync::mpsc` channel, fed with `try_send`, so the watcher's thread never blocks on it and ends when it is closed.

### Echo and renames (HV-SCRIPT-08 steps 6–7)
- `EchoTable` keeps up to four stamps per path for 5 s, and at most 1,024 paths. The service records into `Watcher::echo()` from `fileio::save_with_echo`'s callback, under the path the bytes landed at: for a file link, `Joined::target`.
- A settled file is hashed only when its `(len, mtime)` matches a record, through a handle that does not follow a final link and does not block on a FIFO, reading at most the expected length plus one byte. It is dropped only when the whole stamp matches.
- Dataless files are never hashed.
- A settled removal and a settled creation pair as `Renamed` when their lengths and content hashes match. The removed file's hash is known only when a tab read or saved it (`Watcher::remember`) or Hacivat wrote it, so listing never reads a file (R-M4b-15).

### Threads and I/O
- The reconciler runs on its own named thread (`hv-library-watch`). That thread unwatches and drops the source, which joins the `notify` debouncer thread or the poll thread. `WatcherProbe::released` checks this in the tests. Corrected in fix round 1 (I6): dropping the `Watcher` no longer joins unboundedly, and notify's inotify and Windows backends do not join their own threads (below).
- **Deviation from the GC "all watcher I/O on the `CoreRuntime`":** this is a long-lived loop, like `notify`'s own threads, not a blocking-pool task. It is never the UI thread. `Watcher::start` blocks while the source registers the tree and the library is scanned once, so Task 10 calls it from the blocking pool.
- **Idle CPU** (review probe, `idle_cpu_is_recorded`, `HACIVAT_PERF=1`): ten idle watchers cost 20 ms of CPU over 5 s on macOS [Observed]. That is the debouncer's 37.5 ms tick; the reconciler waits on a condition variable.

### Interface changes against the plan's sketch
- `Watcher::start(root, opts, sink: mpsc::Sender<Vec<WatchEvent>>, source)` as planned, plus `start_with_fallback` (tests inject the fallback) and `open` (it chooses the backend).
- `WatchEvent` gains `Renamed { from, to }`, `Unsettled(rel)` and `RepositoryChanged`; `BackendLost` carries a `LostReason`.
- The `Debouncer` answers `Flushed::{Lost, Rescan, Paths}`. The plan's `burst_becomes_one_rescan` therefore expects `vec![Flushed::Rescan]`, and the `[WatchEvent::Rescan]` value is checked end to end in `burst_becomes_one_rescan_end_to_end`.
- After a `Rescan` the reconciler learns the library afresh without reporting each file; the service rescans and compares snapshots.

### Named exception: `xtask/src/check_deps.rs` (carries-w1 T9 "hardening m1")
- `WatcherFollowsLinks` also refuses stand-ins for the names it trusts, in the scripts crate's product files:
  - `FileIdCache` named at all;
  - any item, binding or rename named `NoCache`;
  - `watcher_config` bound or renamed, or defined as a `fn` anywhere but `src/watch/mod.rs`.
- Test: `watcher_cache_and_config_stand_ins_are_flagged`, nine forms with RED captured.
- At run time, `debouncer_cache_is_nocache_or_never_leaves_root` checks that the `NoCache` in the source is `notify_debouncer_full`'s own.

### [Unknown] → CI
- U-06 (inotify event shapes, the watch count for a 5,000-folder tree) is settled by `watch_real` on ubuntu and `inotify_watches_for_5000_folders_are_counted`, which prints an `HV-FACT t09 inotify_dirs=…` line.
- U-07 (`ReadDirectoryChangesW`) is settled by `watch_real` on windows. In `notify` 8.2.2 the completion routine ignores the byte count (`windows.rs:348`), and any error code other than success, abort and access-denied, `ERROR_NOTIFY_ENUM_DIR` (the overflow) included, unwatches the folder with only a log line (`:361-389`). An overflow on Windows can therefore end the watch silently, and nothing reaches the watcher. Recorded residual; carry to the controller: a periodic root hint (for example every 30 s) on Windows, which the poll source's mechanism already provides.
- U-08 (a real VS Code save) stays with the gate demo.

### Fix round 1 (review I1–I8, m1–m10; rulings R-T9-2 to R-T9-5)

- **I1, R-T9-3: racily clean files.** An equal `(len, mtime)` does not prove a file unchanged when the time is within `RACY_WINDOW` (2 s) or the time's granularity (whole seconds → 1 s, whole even seconds → 2 s) of the moment the file was last seen with that stamp, or after it (clock skew).
  - Every known file keeps `seen_at`, the wall clock before the look that last saw its stamp.
  - A hinted file in that state settles like a change and is then verified. With a known hash (a tab read or saved it, or Hacivat wrote it) the file is hashed again: another hash is `Changed`, unless it equals Hacivat's own write. Without one, the hint becomes `WatchEvent::PossiblyChanged(rel)`. The service hashes only open files, so nobody else needs to act.
  - Scans re-verify racy files with a known hash too. A racy file nobody has open and no hint names is the one residual; a later hint or edit reports it.
  - Echo suppression only ever drops a stamp with an equal content hash, so the foreign edit right after our own save, on a coarse-time file system, is reported.
  - Tests: `same_len_same_mtime_edit_after_hint` (P1), `external_edit_right_after_own_save_coarse_mtime` (P1b), `same_len_same_mtime_edit_of_a_file_nobody_has_open_is_possibly_changed`, `a_same_second_edit_on_a_coarse_time_file_system_is_not_lost` (a whole-second `Fs`).
- **I2: renames pair on a unique hash only.** A removed file with a known hash and one new file of the same length and hash, with that hash unique among the removed files and among the new ones. Otherwise the result is `Removed` plus `Created`. Tests: `identical_files_rename_pairing` (P2), `two_new_copies_of_one_moved_file_do_not_pair_either`.
  - Carry for T14: pairing ignores environment folders, so a tab that follows a rename re-applies `envscope::visible` first (R-T5-6).
- **I3: unchanged trouble is quiet.** A scan stopped at the same limit with the same files as the known ones, or a root that is still gone, is no event. A change of that state (new files under the limit view, leaving or entering the limit, the root going or coming back) is one `Rescan`, and the library is re-learned from that scan. Tests: `polled_library_at_limit_rescans_once_per_change`, `polled_library_at_limit_rescans_forever` (P4, end to end), `a_missing_root_is_one_rescan_per_change`.
- **I4: the poll source and the repository.** Each pass `lstat`s `<root>/.git/{HEAD,index,packed-refs}`. Nothing is read or followed, and a gitfile or linked `.git` is left to Task 7. A move of their `(len, mtime, dataless)` adds a `.git/index` hint, which the reconciler reports as `RepositoryChanged`. Tests: `poll_pass_reports_repository_changes`, `a_linked_repository_folder_is_never_looked_into_by_the_poll_source`.
  - The marker test also found that a quiet scan dropped a `RepositoryChanged` from the same tick; fixed.
- **I5, R-T9-4: Linux watches folder by folder.** `OsEventSource` never asks notify for a recursive watch on Linux. notify's recursive inotify walk stats with `metadata` and walks with `walkdir`'s `follow_root_links`, so a folder swapped for a link before it is watched would be followed (`inotify.rs:405`, `walkdir lib.rs:293`).
  - The root and every folder of `Reconciler::watch_dirs` get one `NonRecursive` watch each. That walk is our own, through the `Fs` seam: `lstat` only, links never entered, skipped names and dataless folders not watched, plus `<root>/.git` itself (not below it), bounded like the scan.
  - The set is synced after every scan (folders appear and vanish; the kernel drops a removed folder's watch). After the first sync, one root look catches anything made between the scan and the watches.
  - More than `WatchOpts::max_dirs` folders (`Watcher::open` sets half the free inotify watches) or a refused watch → `BackendLost(WatchBudget)` and polling.
  - Residual: a folder swapped for a link between our `lstat` and `inotify_add_watch` (which notify calls without `IN_DONT_FOLLOW`) gets one non-recursive watch on the link's target. Its hints name paths through a link, which `join_with` refuses (`LinkOnPath`), so they are dropped, and the next sync removes the watch.
  - macOS: FSEvents watches the root recursively; it does not follow a directory link (Task 0 [Observed]), and its root events (root changed) reach the reconciler as a root hint.
  - Windows: how `ReadDirectoryChangesW` treats a junction or symlink inside a recursively watched root is [Unknown] → CI `watch_real` on windows (`dir_symlink_created_after_watch_is_not_walked` is `cfg(unix)`; a junction variant is carried to Task 20's adversarial suite).
  - Tests: `watch_dirs_are_real_folders_only`, `per_folder_watches_follow_new_and_removed_folders`, `too_many_folders_for_one_watch_each_falls_back_to_polling`, `a_refused_folder_watch_falls_back_to_polling`, and real `per_directory_source_sees_new_and_removed_folders` (FSEvents non-recursive on macOS, inotify on ubuntu). On Linux, `dir_symlink_created_after_watch_is_not_walked` also asserts that the inotify watch count does not move (m7).
- **I6: a bounded drop.** `Watcher::drop` signals stop and waits at most `DROP_JOIN_BOUND` (250 ms) for the thread's completion signal (sent as the thread drops its last field, after the source). Then it joins, or abandons the thread, which ends by itself when its stuck file-system call returns. Test: `drop_blocks_during_a_blocked_fs_call` (P5): the drop returns while the gated `lstat` is still blocked, and the probe reports released after the gate opens.
  - notify's `INotifyWatcher` and `ReadDirectoryChangesWatcher` do not join their own threads on drop; they send a shutdown and return (`inotify.rs:632-637`, `windows.rs:612-617`). Those threads end shortly after. Only FSEvents, the debouncer and Hacivat's own threads are joined.
- **I7: the named tests.**
  - `git_checkout_2000_is_one_rescan` (`watch_real`; the CLI on a fixture repository it made, no system or global config). Its first run produced 789–999 per-file events: the files of the first notify batches settled one by one before the burst overflowed.
  - So the burst rule changed:
    - a burst is a run of hints each less than a window apart (an extending window), counted in distinct paths (a file rewritten in a loop is never a burst);
    - an overflowed burst's paths are dropped and its one `Rescan` is handed on once the burst is quiet, or every 2 s while it keeps going;
    - nothing settles while hints keep arriving (at most 2 s);
    - first looks still happen, so a rename's removal and creation settle together.
  - Result on macOS: `rescans=1 per_file_events=0`. Tests: `one_file_rewritten_in_a_loop_is_never_a_burst`, `an_endless_burst_still_rescans_every_two_seconds`, `a_burst_rescans_once_when_quiet_and_lost_is_never_swallowed`.
  - `short_name_event_is_reconciled`: a Windows body in `watch_real` (the 8.3 name from cmd's `%~s`; with 8.3 names off it prints `HV-FACT t09 short_names=off`), and a reconciler unit test elsewhere.
- **I8, R-T9-2: the Windows backstop.** `WatchOpts::backstop` defaults to `Some(30 s)` on Windows. `Reconciler::start_backstop` (started by the watcher for the OS backend) makes every 30 s a root look (one scan). Tests: `backstop_root_hint_finds_a_change_no_event_reported` (fake clock), `the_backstop_is_on_by_default_on_windows_only`.
- **R-T9-5: the watcher's own thread is accepted.** A blocking OS callback loop that lives as long as the library, and whose file-system calls can hang on a share (I6), must not hold a `CoreRuntime` blocking slot for its lifetime. It never runs on the UI thread, and `Watcher::start` itself is called from the blocking pool.
- **Minor findings:**
  - **m1:** check-deps refuses more than one `fn watcher_config`, or a nested one, in the home file. A constructor's config argument counts only as the bare `watcher_config()`, never `alt::watcher_config()` or a method. A second `NoCache` item anywhere was already refused. Test: `watcher_config_is_one_top_level_fn_called_bare`, RED captured.
  - **m2:** R-T9-5, above.
  - **m3:** `vboxsf`, `virtiofs`, `fuse.vmhgfs-fuse`, `prl_fs` and `fuse.gvfsd-fuse` are polled (`vm_shared_folders_are_polled`).
  - **m4:** a lost backend with no working fallback is `Backend::Unwatched` and `BackendLost(NoFallback)` (`a_lost_backend_without_a_fallback_is_unwatched`).
  - **m5:** the poll pace starts at 2 s, doubles to 30 s while passes change nothing, and resets on a change. A pass never comes sooner than five times the last pass's scan cost (`next_interval`, `EventSource::pace`). Tests: `poll_pace_backs_off_while_idle_and_resets_on_change`, `the_poll_source_paces_each_root`.
  - **m6:** `Watcher::open` keeps a caller's `Backend::Poll(_)`. A `PollReason::UserChoice` is a `service.rs` seam (T10/T16 carry). Test: `open_keeps_a_chosen_poll_backend`.
  - **m7:** see I5.
  - **m8:** `watches_in_use` counts only processes owned by the user of `/proc/self`.
  - **m9:** residual, recorded. The echo and rename hash opens the path `join_with` validated, with `O_NOFOLLOW` on the last component only. A folder swapped for a link between the two could make it read at most the expected length plus one byte of a regular file elsewhere, only to compare a hash. Reusing T5's directory-handle opens is carried to Task 20.
  - **m10:** a path goes to every watched root that holds it (nested libraries sharing a source).

### Fix round 2 (re-review N1, N2, m-fr1-1 to m-fr1-6)

- **N1: only names that matter count toward a burst.**
  - `Reconciler::push` runs the string-only half of `classify` (`name_filter`) first.
    - Dropped before the burst counter: a path outside the root, under a skipped folder (`node_modules`, `.git/objects`, `.next`, any dot folder), deeper than 32, or not UTF-8.
    - Passed on but uncounted: a name with another extension (build output such as `*.o`, `*.js`). Such a name may still be a folder. It neither starts nor extends a burst.
    - Counted: script names, extensionless names, known files, the root and the repository names.
  - Counting starts afresh after each handed-on `Rescan`, so a long run of moderate churn is not one ever-growing burst.
  - A rescan no longer learns an open file's change silently. A file with a known hash (a tab has it, or Hacivat wrote it) that changed or vanished keeps its old stamp and becomes a candidate (at most 256 per rescan). It settles and is reported as `Changed` or `Removed`, however many rescans a burst causes.
  - Files nobody has open are covered by the `Rescan` (the service rescans the tree). That keeps `git_checkout_2000_is_one_rescan` at `per_file_events=0`.
  - Tests:
    - `noise_churn_starves_a_real_edit` (P8: 10 `node_modules` paths per 100 ms for 30 s; the edit at 12 s is `Changed` at 12.2 s, and there is no `Rescan`);
    - `counted_churn_past_the_limit_still_reports_an_open_files_edit`;
    - `ignored_names_never_count_toward_a_burst`;
    - `uncounted_hints_do_not_extend_or_start_a_burst`.
- **N2: a root look after a sync adds folders.** `EventSource::sync_dirs` answers how many folders it added. When that is more than zero, the watcher pushes one root hint, as `start` does. That finds files written in a folder before its watch existed, and folders listed too late; it converges once no folder is new.
  - The same round fixed a latent wake-up gap. A hint the watcher pushes itself (this one, and `start`'s) now makes the reconciler due at once (`Debouncer::pending_at`). Before, it waited for an unrelated event.
  - Tests: `files_made_before_a_new_folders_watch_are_found` (a fake source whose sync writes into the new, empty folder), `a_hint_pushed_by_the_watcher_itself_is_due_at_once`.
- **m-fr1-1: ref-only moves.** `logs` is a repository name. On Linux, `.git/logs` is watched non-recursively when it is a real folder. The poll pass stamps `.git/logs/HEAD` too, only when `logs` is a real folder. So `reset --soft` and `update-ref` refresh the badges. Tests: `a_ref_only_move_is_a_repository_change`, `the_poll_source_sees_a_ref_only_move`.
- **m-fr1-2: a future date is capped.** A time after the look is racy only while it is within the racy window of the look. A file dated far ahead (skew, an extracted archive) is not re-verified on every pass. Test: `a_file_dated_far_ahead_is_not_racy_forever`.
- **m-fr1-3: notify-debouncer-full's root list (recorded residual).**
  - Its roots are a `Vec`: `add_root` scans it per watch, `remove_root` runs `retain(starts_with)` per unwatch, and `recursive_mode()` scans it per event.
  - A folder that vanished is unwatched by the kernel; our `unwatch` then fails before `remove_root`, so its entry stays for the source's life. That cannot be fixed from outside the crate.
  - Measured with notify's `NullWatcher` (no OS work, debug build): adding 5,000 roots costs 1,031 ms, and unwatching them one by one 485 ms (`debouncer_root_bookkeeping_for_5000_folders_is_recorded`, `HACIVAT_PERF=1`).
  - Mitigation: unwatching a per-folder source's last root stops the whole debouncer instead (no per-folder `remove_root`). This is what every drop and fallback does.
  - Left: the start cost, which is on the blocking pool, linear-per-event lookups, and the leak of vanished folders' entries (one `PathBuf` each).
- **m-fr1-4:** a hint through a folder that became a link (`join_with` answers `LinkOnPath`) is reconciled by a scan. After the scan, the per-folder watches are re-synced, which removes a stray watch on the link's target. Test: `a_hint_through_a_linked_folder_triggers_a_scan_and_so_a_sync`.
- **m-fr1-5: the backstop re-arms.** A backstop look that finds a difference no hint announced since the previous one (`TickInfo::rearm`) makes the watcher unwatch and re-watch the root of an OS source, and say `WatchEvent::BackendRearmed`. If the re-watch fails, the library falls back to polling. A per-folder source is not re-armed this way (it is Linux's, and has no backstop). Tests: `a_backstop_look_finding_an_unannounced_change_asks_for_a_rearm`, `the_watcher_rearms_a_source_that_stopped_silently`.
- **m-fr1-6: residual of R-T9-3.** A tool that restores an old modification time (`rsync -t`, `cp -p`, `touch -r`, `tar x`) on a file last seen well after that time keeps `(len, mtime)` equal and outside the racy window, so the edit is not seen until the next one. These are git's racy-clean semantics. A same-length edit restored this way is reported only if a tab re-reads the file. Seeing it would mean hashing every file on every look.

### Fix round 3 (re-review I-fr2-1, m-fr2-1, m-fr2-2)

- **I-fr2-1: a slow checkout is one rescan again.** fr2 had two causes:
  - the hold ended at `began + 2 s` while an overflowed burst's rescan was due later, so the first thousand files settled one by one;
  - fr2's count reset after a forced rescan, so the rest of a still-going burst was reported per file.

  Now:
  - While a burst has overflowed, `Debouncer::hold_until` lasts until its rescan is handed on, with no fixed end.
  - After a `max_wait`-forced rescan of a burst that is still going, the burst stays overflowed: it keeps swallowing hints and gets one rescan per 2 s. The count starts afresh only once the burst has been quiet for a window. That reverts fr2's reset; the name filter alone keeps noise from ever counting (P8 still passes).
  - The R-T9-6 exception holds inside the hold. A candidate whose file is open (a known hash) settles at its normal time, so `next_due` never holds it, and the rescan's `learn_state` keeps an open file's change back as before.

  Tests:
  - `slow_checkout_per_file_events` (P14 on a fake clock: 4,000 files rewritten one per millisecond over 4 s, one of them open). Result: only `Rescan`s plus `Changed` for the open file. The RED run gave 1,992 per-file events.
  - `an_endless_burst_still_rescans_every_two_seconds` (rescans at 2 s and 4 s, then once quiet).
  - `git_checkout_2000_is_one_rescan` now passes under parallel load (three `watch_real` and one `watch` at once).
- **m-fr2-1: the double report is pinned.** `a_second_hint_for_the_same_write_is_possibly_changed_unless_remembered` (P12's shape): a second hint for the same write within R-T9-3's window is `PossiblyChanged` when no tab has the file open, and nothing once the consumer has remembered what it re-read.
  - The real-OS test helper drops such a `PossiblyChanged` only when it arrives within `RACY_WINDOW` plus two debounce windows of the same file's `Created` or `Changed`. Before, it was any later time.
  - Carry for T10: call `Watcher::remember` after every read or reload of an open tab, and treat `PossiblyChanged` as "re-hash; act only if the hash differs". A re-hash that matches never reaches the user.
- **m-fr2-2:** re-arm (m-fr1-5) is not supported for a per-folder source. Unwatching its sole root stops its debouncer (m-fr1-3), so the re-watch fails and the library falls back to polling (`BackendLost(Failed)`). That never happens with today's defaults: the backstop is Windows-only and per-folder watching is Linux-only. Changing either default needs a re-arm that rebuilds the per-folder source.

## m4b-t10 — library service and Go to script index

### The actor (spec §5.4 rule 2; HV-SCRIPT-01..04, -08, -11 step 7)

- `LibraryService::open(&LocalPaths, &Handle)` takes the core runtime's handle (R-T1a-2) and starts **one long-lived task on its blocking pool**. That task alone owns the `LibraryRegistry` (still the only writer of `libraries.toml`), every library's listing, the event source, the settle and echo bookkeeping and the status refresh.
  - It handles one message at a time from a bounded inbox (1,024): API commands with a `oneshot` reply, hint batches from the source, and its own timers (tokio tasks that sleep and send).
  - All of its file I/O (registry, manifest, scan, `lstat`, stamps, recovery sweep) runs on that blocking thread. The repository status runs on a separate blocking task, at most one per library.
  - No lock is held across an `.await`. The only shared lock is the echo table, taken briefly by a save's blocking task and by the actor.
- **Snapshots.** After each drained burst (at most 64 messages) the actor publishes one new `WorkspaceSnapshot` with `generation + 1` through `arc-swap`, only if something changed.
  - Then it sends the events: one `Changed { id, generation }` per library that changed, then the file and status events.
  - A reader gets the old `Arc` or the new one, never a half-built snapshot.
  - Commands publish before they reply, so `attach`, `detach` and `map` are visible as soon as they return.
  - `LibrarySnapshot.generation` is the workspace generation of the library's last change; the index can use it to tell what changed.
- `WorkspaceSnapshot` gains `registry: Arc<[RegistryDiag]>` (what loading `libraries.toml` found) and `git_status(id, rel)`. The status is looked up by the scan's exact `RelPath` text: Task 7 keys it by the disk's spelling (NFD on macOS), and `git_badge_of_an_nfd_named_file_is_found_by_its_scan_path` proves an NFD-named modified file gets its badge (R-T1a-2).
- `ServiceOptions` carries the timings (settle 150 ms, status debounce 500 ms, poll and recheck 2 s, echo 5 s), the scan and status limits and the home folder for `AttachGuard`. Tests shorten the timers; nothing else changes.

### Hints, probes and rescans (alg/28 C3, C4)

- Until Task 9 merges, the service consumes Task 1a's `EventSource` seam directly: `RawHint`s, not Task 9's reconciled `WatchEvent`s. The reconciliation below is the service's own; when Task 9's `Watcher` lands, the controller can swap it in front (its `EchoTable` and settle queue overlap this task's private ones).
- A hinted path is mapped into the library first:
  - outside the canonical root (or the chosen spelling), with a `..` or a non-UTF-8 component, under a skipped name (`.`-prefixed, `node_modules`) or not a valid `RelPath`: dropped;
  - `.git/index`, `.git/HEAD`, `.git/packed-refs`, `.git/refs/…`: a status refresh;
  - `.hacivat/…`: the manifest is read again (a teammate's pull can change the binding);
  - the root itself: a rescan.
- Every other path is probed through `pathsafe::join` (every component re-validated, no link followed) and, for a plain file, `lstat`.
  - A plain script file inside a listed folder (or the root) is patched in place: updated, inserted in tree order, or removed.
  - Anything else rescans the library with `scan::scan` and diffs the listing: a folder, a link, a new folder, an error, a known folder gone, a scan that had stopped at a limit, a batch of more than 1,000 paths, or a batch the full inbox dropped (the sink sets an overflow flag).
  - So no hint can make the service list something the scan would not: every listing is either the scan's or a patch the scan would agree with.
- A source that will not start, and `RawHint::Lost`, switch the library to `Polling(BackendLost)`: a rescan and diff every 2 s. (Superseded by the watcher wiring below: Task 9's `Watcher` polls when it must, and the service has no polling of its own.)

### Settle, echo, rename (HV-SCRIPT-08 steps 5–7)

- A changed file is stamped only after its `(len, mtime)` held for one settle window (re-queued at most 10 times, then stamped anyway). A cloud placeholder is never stamped.
- Stamps go through a new `fileio::stamp_of` (named exception, fenced): the same `join`, `visible_at` and no-follow handles as `read`, the bytes hashed streaming and dropped. The service passes an environment the entry is shown under, so the scope check is never bypassed, and a file hidden everywhere is never read. Script content is read only in `fileio` (T5 mg2).
- `save` records the new stamp in the echo table before the rename (`fileio::save_with_echo`). A settled stamp equal to one recorded in the last 5 s announces nothing; otherwise `FileChanged { stamp }`. The hash is compared too, so a foreign write with the same length in the same mtime tick is still announced.
- A file gone is `FileDeleted`, unless a file of the same length and content hash appeared in the same change: then `FileRenamed`. The old hash is known from a tab's `read`, a `save` or an earlier stamp; a file never opened is never hashed for pairing.
- A deleted path that comes back is announced as `FileChanged` (a tab showing "Deleted on disk" learns it is back). Other new files raise only `Changed`, so a `git checkout` does not hash every new file.

### Status refresh (HV-SCRIPT-11 step 7)

- Debounced 500 ms after any change to the listing and after a `.git/` hint. A trigger while a status runs queues exactly one more, so a checkout storm costs at most two runs per window.
- A status equal to the last one publishes nothing and sends no `GitChanged`. The first status runs when a library comes up.
- **Limit:** the hints come from the library's own watch. When the repository's `.git` sits above the library root (a library inside a monorepo), a commit that changes only `.git/index` is seen at the next change in the library, not at once. A watch of the discovered git dir needs a path from `hacivat-git` (whose discovery is private) and a non-recursive seam; left to Task 9 or Task 13.

### Availability (HV-SCRIPT-01 step 5; the T4 carries)

- Every `RootProblem` is mapped: `Missing`, `Changed` → `RootChanged`, `Unreadable(kind)`, and `InEnvFolder(name)` → `Unavailable(InEnvFolder)` plus `LibraryDiag::RootInEnvFolder(name)`.
- An unavailable library is unwatched, shows no entries and no status, and is re-checked every 2 s. A folder that comes back is scanned and watched again. A root that now resolves elsewhere stays `RootChanged` for the session (the first canonical root is kept, fail closed).
- Diagnostics: `LibraryDiag::Scan(scan::Diag)` (including `LinkScope`), `ScanStopped(LimitHit)` and `Suppressed(n)` (R-M4b-14: the limit is visible), `Manifest(manifest::Diag)`, `NoLogicalName`. Names in them are already in display form.

### Commands

- **Attach.** The registry's checks (HV-SCRIPT-01 step 2), then a root inside an environment folder is refused up front (`envscope::root_env_folder`). With `write_manifest`, `manifest::write_new` writes `format`, `database` and `aliases = []` only if the file is absent; an existing manifest is read and never rewritten, and a write that fails (a linked `.hacivat`) refuses the whole attach with nothing registered. `db` is what the manifest says; without one written, the library binds what the folder says, and any other database is a local mapping (`map`). The registry row is saved last; a failed save reloads the registry from the file.
- **Detach** removes the row and the mappings, stops the watch, drops listing, status, settle, known hashes and echoes, and discards the recovery copies (R-T5-4). The folder is never touched (`detach_stops_watching_and_never_touches_the_folder` compares every byte).
- **Map** writes the shared alias first (when asked, through `manifest::add_alias`), so a refused alias changes nothing; the mapping goes only into `libraries.toml` (R-M4b-8). `unmap` and `drop_profile` (R-M4b-2) are new.
- **Read, save, stream** take the tab's environment (R-T5-6) and pass it to `fileio`, which refuses a path not shown under it (`production_path_is_refused_under_staging`; also `None`). They run on the blocking pool. An unknown or unavailable library answers `Io(NotFound)`.
- **Deviation:** `open_stream` is now `async` and takes the file's `TextEncoding`. Opening is I/O (so it leaves the caller's thread), and `fileio::open_stream` decodes in the file's own encoding (a Windows-1254 script would otherwise not stream). No caller existed yet.
- `create`, `rename` and `trash` stay Task 1a's fail-closed stubs: the file operations, and the `TrashContext` carry, are Task 13's.

### Launch hygiene (T5 mf4)

- `recovery::sweep_unregistered` (named exception, fenced) removes the copy folders of libraries that are not registered, at launch and only when the registry was read cleanly (not read-only, set aside, newer or unreadable: such a registry lists nothing to keep).
- Only folders named by a hyphenated library UUID are considered, never `instances` or anything else. Links are never followed. A folder with a file modified in the last 10 minutes is kept, since another instance may have registered that library after this one read the registry.

### Go to script index (HV-SCRIPT-09)

- `ScriptIndex::build(&WorkspaceSnapshot)` touches no file system (names and metadata only, R-M4b-15). It offers every listed script file except those hidden everywhere (unknown `@name`, contradiction) and disabled links. Folders are not offered.
- Display: the name, the folder and the library's folder name go through `pathsafe::display_name` (NFC, invisible and bidi characters as `<U+XXXX>`; R-M4b-23). Identity stays the scan's exact `RelPath`.
- Keys: `hacivat_l10n::search::build_key` over the display text, with the caller's `SearchOpts` (the app passes `commands::palette::search_opts(cx)`). Keys are built once per `turkish_tolerant` choice, on first use. `musteri` finds `müşteri_segment.sql` (also NFD-named) for a Turkish user and not otherwise (D-074).
- Rank: exact name or stem > name prefix > word prefix > substring > subsequence > a match only in the folder or library name (a plain substring there). Ties: shorter path, then library order, then the path's bytes.
  - alg/28's `goto_ranking_table` lists `orders, reorder, big_orders, o_r_d` "in that order (prefix, word-prefix, substring, subsequence)". The list contradicts its own labels (`reorder` is a substring match, `big_orders` a word prefix). The test follows the labels and the rank rule: `orders, big_orders, reorder, o_r_d`. Ruled R-T10-1; alg/28 is corrected (fix round 1, m9).
- Scope: `query_for(q, opts, limit, env)` keeps only files `envscope::visible` under `env`, the same rule as the tree. A `@production` file never appears for a staging or untagged connection. `query` lists every offered file, and `query_where` takes any filter (Task 15's candidates).
- Cost: only the best `limit` hits copy their strings. The 10,000-entry query budget (5 ms) is a release budget (`HACIVAT_PERF=1 cargo test --release`); the debug build measures about 31 ms, almost all of it in `hacivat-l10n`'s subsequence matcher.

### A tripwire, not an enforcement (m1)

- A test tripwire (`service_index_and_events_touch_no_file_themselves`) fails if `service.rs`, `index.rs` or `events.rs` names `std::fs`, `fs::`, `File::`, `OpenOptions`, a read call, `read_dir` or `canonicalize(`. Every touch of the disk goes through `fileio`, `pathsafe`, `scan`, `manifest`, `registry` or `recovery`.
- A `check-deps` rule was not added: `xtask/src/check_deps.rs` is Task 1a's file, and the tripwire covers this task's own modules. A token rule for the whole crate is a candidate for Task 20.

### Fix round 1 (review I1–I7, m1–m10; rulings R-T10-2, R-T10-3, R-T10-4)

**Bounded workers (I6, R-T10-4).**
- The actor now does no library I/O. A library's root validation, scans, probes, settle looks, stamps and rename pairing run as jobs on at most `ServiceOptions::workers` (8) blocking workers, one job per library at a time (`service/work.rs`). The attach steps (resolving the folder, writing the manifest) and the alias write are command jobs that are not counted against the pool.
- A job's answer is applied by the actor. Work that arrives meanwhile waits in that library's queue, coalesced into one rescan, the hinted paths and the due settle looks.
- A library whose mount hangs holds one worker and stalls only itself. `detach` removes it at once and abandons its job, whose late answer finds no library. Test: `a_hung_library_stalls_only_itself`, with a stalling `Fs`; with one worker it fails.
- **Residual:** `LibraryRegistry::add` (Task 3) still canonicalizes on the actor during an attach. The folder was resolved by a worker just before, so only a mount that hangs between the two steps can stall the actor.

**What a connection sees (m2; I1, R-T10-2).**
- `WorkspaceSnapshot`'s per-path data is now crate-private: the listings, the diagnostics and the status maps. A connection reads `snapshot.view(env)`.
  - Its `LibraryView` gives the shown entries (visible files, and the folders on their way), `file_count`, badges for shown entries only (`git_status`, `git_statuses`), and a `GitSummary` without the map.
  - Its diagnostics drop any that name a path not shown under the connection, and count them as `LibraryDiag::HiddenProblems(n)`. An `EnvContradiction` always names other environments' folders, so it is always counted, not shown. The misspelt-folder diagnostic is still named, as R-M4b-6 requires.
- `ScriptIndex` counts and queries per environment: `len_for`, `query_for`, `query_for_any`. There is no public unscoped query.
- **Events go through `subscribe(env) -> Subscription`.** `open` returns no receiver any more.
  - `Changed` and `GitChanged` always pass.
  - `FileChanged`, `FileDeleted` and `FileLeftScope` pass only for a path shown under that environment.
  - `FileRenamed` passes when both of its paths are shown. When only `from` is shown, it becomes the new `LibraryEvent::FileLeftScope { from }` ("no longer listed under this connection"), and the new path is never given. When `from` is not shown, nothing passes.
  - Test: H7, `h7_rename_is_never_paired_across_an_env_boundary`. Staging gets `FileLeftScope`, production hears nothing of the staging path, and an untagged subscriber hears no `@` path.

**The root is re-validated (I3).**
- Every bring-up and every full scan checks the root in `work::validate`. The registered path must be a real folder, not a link or junction (`RootRefusal::Link`). It must resolve to the canonical path and folder identity (Unix dev/ino) it had when first resolved this session (`Moved`, `IdentityChanged`). The attach checks run again: filesystem root, home, app folders, overlap with another library, and an environment folder.
- A failure is `Unavailable(RootChanged)` with `LibraryDiag::RootRefused(why)`; nothing is listed or watched.
- `attach` refuses a link root up front (`AttachError::RootIsLink`), because a link root would be refused at every launch.
- Tests: H6, `h6_root_replaced_by_a_link_between_sessions_is_not_adopted`; `a_root_with_a_new_identity_is_refused`.
- **Residual:** across sessions only the "not a link" rule holds; the identity is not stored in `libraries.toml`. A parent folder that became a link between sessions is still followed.

**Watch before scan (I5).** A library's watch starts as soon as its root is known (at attach, or after the first validation at launch), and only then is its first scan queued. Hints that arrive during the scan wait and are probed after it. Test: H1, `h1_file_created_between_scan_and_watch_is_listed`.

**Content only for open files (I4).**
- `read` and `save` remember the file as open in a tab. Only remembered files are settled, stamped (`fileio::stamp_of`) and announced with `FileChanged`.
- Any other changed file is a listing change (`Changed`), and its bytes are never read. Rename pairing reads only the candidates of a remembered file.
- Test: `an_unopened_changed_file_is_never_read`, which counts `stamps_read`.

**The launch sweep checks liveness (I2).** `recovery::sweep_unregistered` keeps a folder if any of its copies belongs to an instance that is alive, or whose liveness cannot be tested (R-T5-5), or if any sidecar cannot be read. The 10-minute age guard stays as a second line. Test: H2, `h2_sweep_spares_a_live_instances_copies`; once that instance is gone, the folder is swept.

**Two-pass streams (I7, R-T10-3).**
- `open_stream(.., expect)` returns a `ScriptStream`.
- Its `PassRecord` holds:
  - the opening stamp (`len`, `mtime`, and privately the full fingerprint: dev, ino, ctime);
  - a running XXH3-64 of the raw bytes at the end of every 64 KiB block: one `u64` per block, about 2,400 for a 150 MB file.
- **The pass-1/pass-2 contract.** Pass 2 opened with that record is refused at once (`ReadError::Changed`, a fenced named exception in `fileio`) unless the fingerprint is pass 1's. It stops with `ChunkError::Changed` before handing out the first block whose running hash differs, and when the file has more or fewer blocks.
- Tests: `two_passes_see_the_same_file_or_stop` (a change between the passes, and one during pass 2); the inline `second_pass_stops_at_the_first_block_that_differs`, a forged record with an unchanged fingerprint.
- `hacivat_exec::ScriptChunks` is unchanged: T12's adapter keeps the record.

**Minor findings.**
- **m3:** queries are cut to `MAX_QUERY_CHARS` (256).
- **m4:** the `OnceLock` key build is documented; Task 15 queries one index from one thread.
- **m5:** documented. The effective name is the snapshot's `db`.
- **m6:** `Remember` and `Touched` for a library that is gone are dropped.
- **m7:** a 60 s status backstop (`git_backstop`) for polled libraries and for libraries whose `.git` is above their root.
- **m8:** `Subscription` documents that a lagged receiver re-reads the snapshot.
- **m9:** alg/28's ranking text is corrected (R-T10-1).
- **m10:** the index test's comment explains why no `Fs` is needed.
- **m1:** the tripwire now covers `service/*.rs` and the renamed and brace imports. It is called a tripwire. A `check-deps` token rule is carried to T20.
- **Adopted:** the reviewer's H3 (inbox overflow) and H4 (monotonic generations under races) are adopted as named tests.

### Fix round 2 (re-review fr1: F1–F4, m-a…m-e; ruling R-T10-5)

**The actor never touches the filesystem (F1).**
- At launch and on a reload after a failed save, the service reads `libraries.toml` with the new `LibraryRegistry::load_text_only`. It compares rows by their text (id and path) and canonicalizes no row. Canonical nesting is left to each library's worker `validate` (`Overlaps`).
- An attach is registered with the new `LibraryRegistry::add_resolved`:
  - the worker job `resolve_attach` answers the folder's canonical path and identity, and makes the guard's checks (filesystem root, home and app folders, resolved in the job, and an environment folder);
  - the actor checks overlap against the canonical roots it already holds.
  - Both are T3 named exceptions in `registry.rs`. T3's own `load` and `add` are unchanged.
- The rules' home and app folders are resolved inside each job, never on the actor.
- The only files the actor still reads and writes are `libraries.toml` and the recovery folders, both in the app's own folders.
- **Test:** `a_hung_mount_never_blocks_launch_attach_or_reload`. The injected `Fs` blocks every call under one library's folder. The service loads, the healthy library comes up, the hung one shows `NotResponding`, and an attach, a map, a reload after a failed save (Unix, not as root) and a detach all answer.

**Deadlines (F2, R-T10-5).**
- A detach of a busy library frees its worker at once. Its job is tracked as abandoned, so a late answer neither frees the slot twice nor reaches a library.
- A job running past `ServiceOptions::job_deadline` (10 s) stops counting against the cap, and its library is `Unavailable(NotResponding)` until the job answers. It keeps its one job, so blocked threads stay bounded by the number of libraries.
- **Tests:** H8 (20 libraries, 8 on a dead mount: the 12 healthy ones come up, the 8 show not responding), H9 (8 detach-and-reattach retries of a hung folder, then a healthy library scans), and `a_library_that_stops_answering_comes_back`.

**H2 (F3).**
- The cause is not pinned. Rust opens the lock file with `O_CLOEXEC`, so close-on-exec is not the fix, and on macOS `Command` spawns through `posix_spawn`, so there is no fork window there either.
- What happens is that under full parallel load a relaunch now and then sees the dead instance's lock as held, or as untestable. It then keeps the folder, which is the fail-safe answer.
- The test now relaunches until the sweep happens, within the 10 s ceiling. The product is unchanged.

**The pass binds its decoding (F4).**
- `PassRecord` stores the `encoding` and whether the first block began with that encoding's BOM.
- Pass 2 in another encoding is refused at open (`ReadError::Changed`), and one whose BOM differs stops at its first block (`ChunkError::Changed`).
- The stream never decides a line ending (the text keeps the file's own), so there is no line-ending choice to bind.
- **Tests:** H12, `pass_two_without_pass_ones_bom_stops`.

**The root's identity across sessions (m-a).**
- `libraries.toml` rows gain optional `canonical` and `inode` keys. The service writes them after a root's first resolution (`set_resolved`), and `add_resolved` writes them at attach.
- A later launch whose resolution differs is `RootChanged` (`Moved`, `IdentityChanged`).
- Only the inode is compared across sessions: a device number can change with a reboot or a remount.
- An old row without the keys gets them at its next successful validation.
- **Test:** H6b, a folder above the root replaced by a link between sessions.

**Minor findings.**
- **m-b:** `Debug` of `WorkspaceSnapshot`, `LibrarySnapshot`, `EnvView`, `LibraryView` and `ScriptIndex` is written by hand and prints ids, states and counts, never a path (H11).
- **m-e:** `ScriptIndex::is_empty_for(env)` replaces `is_empty()`.
- **m-c and m-d** (forget a file when its last tab closes; a link retargeted into another scope) are carried to T14 by the controller.

### Wiring Task 9's watcher (R-C0-7)

- **One `Watcher` per library.** Each is started by a worker job, because `Watcher::start` and `open` block while the source registers and the library is scanned once. It starts before the library's first service scan (I5). Its events reach the actor through a forwarding task that tags each batch with the watcher's generation. A batch from a watcher that was dropped (detach, unavailable, re-attach) is ignored (T9 carry m-fr3-1).
- **Sources.**
  - `open` uses `Sources::Os`, which is `Watcher::open`: Task 9's backend choice per library, the OS watcher or polling for a network file system or past the inotify budget.
  - Tests pass `Sources::Custom`, through the new `FakeHandle::sources()` and `refusing_sources()`, a `testing.rs` named exception: one fake source per watcher, all driven by one handle.
- **Event mapping.**
  - `Changed`, `Unsettled`, `Created` and `Removed` become probes that patch the listing.
  - `Changed`, `Unsettled` and `PossiblyChanged` also queue a stamp, but only for a file a tab has open. A re-stamp equal to the stamp the tab knows announces nothing, so a `PossiblyChanged` that changed nothing, an echo, or our own save never reaches a subscriber. Repeated reports of one path collapse into one queued stamp (T9 carries).
  - `Renamed` moves the remembered stamp. It is announced as `FileRenamed` once the listing has dropped the old path, and that removal is not a `FileDeleted`. Renames are paired by the watcher, so the service's own pairing is gone.
  - `Rescan` and `BackendRearmed` become a rescan.
  - `RepositoryChanged` becomes a status refresh.
  - `BackendLost` and the backend's `Poll(_)`/`Unwatched` become the library's state: `Polling(reason)`, or the new `LibraryState::Unwatched`. When no watcher could start at all, the library is also listed `Unwatched`.
- **`remember` and the echo.**
  - Every `read` (a reload is a read) and every `save` sends the full `DiskStamp`, which the actor passes to `Watcher::remember`. A watcher that starts later is given every stamp already known.
  - A save records its echo in that library's watcher table (`EchoHandle`, shared through the service) under the path the bytes land at: a file link's target.
- **Removed:**
  - the interim 2 s polling, `ServiceOptions::poll`, the actor's poll timer chain and its no-source fallback;
  - the service's own settle queue, settle window and echo table (`ServiceOptions::echo_ttl`);
  - the raw-hint classification and the inbox-overflow flag (the watcher bounds its own queue and turns an overflow into `Rescan`).
  - The unavailable-library recheck stays: it is not watching.
- **Deviation:** `LibraryService::open_with` takes `Sources` instead of one `Box<dyn EventSource>`.
- **Residuals** (both closed in fix round 3, below):
  - `.hacivat/workspace.toml` edits are dot-folder names, which the watcher drops. A teammate's manifest change binds at the next rescan, not at once.
  - No test forces a stale batch from a dropped watcher (the generation check is direct code).
- **Confirming scans** (found by the H4 stress test once the watcher was wired: 64 of 200 loaded runs left deleted files listed):
  - A probe or a full scan reads the disk as it is *now*, which can be ahead of what the watcher last settled on. Example: a file the watcher reported removed comes back for a moment, is listed by a scan, and goes again before the watcher settles. The watcher's state is still "removed", so it reports nothing more.
  - The watcher's events lag the disk (by its window, more under load), so no event can tell the service. Three earlier attempts failed under load: rescanning on a disagreeing probe only, rescanning only when events came during the scan, and one confirming scan only. A trace showed the confirming scan itself landing mid-change.
  - The rule kept:
    - Each probe carries the watcher's claim (`Expect::Present` or `Absent`); a probe that disagrees asks for a confirming scan.
    - Every full scan asks for one too.
    - A confirming scan runs once the library has had no watch event for four settle windows (re-armed while events keep coming), and is repeated until one finds nothing changed against the listing before it: the disk has stopped moving.
    - Cost: one extra lstat scan per full scan when the library is at rest; probes from single saves need none.
  - **Held removals.** A confirming scan can see a rename's old path gone before the watcher reports `Renamed`. So a *scan's* removal of a file open in a tab is held until the watcher says `Removed` (`FileDeleted`) or `Renamed` (`FileRenamed`), or until a stable confirming scan shows it gone. A tab therefore never hears "deleted" before "renamed".
  - **Result:** 200 runs of the whole `service` binary, four in parallel, alternating en_US and tr_TR, at load average 20–36: 0 failures, H2 and H4 included.

### Fix round 3 (re-review fr2: G1, G2, m-f, m-g, m-i, F3; ruling R-T10-6)

- **G1: a held removal is paired before it is announced.**
  - The problem: after a `Rescan` (Task 9 sends one for a burst, an overflow or a limit, which is what a `git checkout` or `pull` looks like) the watcher learns the new name silently and never says `Renamed`. It may still say `Removed` for the open file. A stable confirming scan or that `Removed` then made a rename a delete (H14b: 20 of 20 rounds).
  - The rule now:
    - A removal of a file open in a tab is held, whether a scan or a patch found it. A scan's removal waits for the watcher's word or a stable confirming scan; a patched one, and a held one the watcher reports `Removed`, is paired at once.
    - Pairing (`work::PairJob`, on a worker) compares the held file's length and the content hash the tab knows with the files that appeared in the last 10 s (at most 64 kept) and are still listed. Only candidates of a held file's length are read, through `fileio::stamp_of`, at most 16 per pairing (I4: never a file nobody has open, except as a rename candidate of one).
    - A match is `FileRenamed`, and the tab's remembered stamp moves to the new name. When the watcher later reports the same move, it is not announced twice. No match is `FileDeleted`.
    - While the library is polled, a held removal that matched nothing and that the poller has not yet reported keeps waiting: for the poller's `Removed`, a later pairing, or at most the next 60 s backstop, which announces it deleted.
  - When a pairing finds a move, the tab's stamp moves to the new name, and the watcher is told (`remember`), so it can pair a later move of the new name itself.
  - Tests: H14 (the watcher's word after the chain settled) and H14b (a `Rescan` in the same moment; and its control without one), 20 rounds each, never "deleted" and always one rename. With pairing switched off, H14 fails in round 0 and H14b in round 1.
  - H14 renames before it sends the `Rescan`. In the probe's order (`Rescan`, then rename), a test thread descheduled between the two let the whole chain end before the rename, with no word from the watcher ever after: a property of the probe, not of the service (seen in 21 of 80 loaded runs, traced).
- **G2 (R-T10-6): the manifest is watched.** A Task 9 named exception in `watch/**`:
  - The name filter passes `.hacivat/workspace.toml` and emits the new `WatchEvent::ManifestChanged`. A hint on `.hacivat` itself does so too, and re-learns the folders. Nothing else under `.hacivat/` is reported.
  - Every backend sees it: a recursive OS watch already does; a per-folder source (Linux) watches `.hacivat/` non-recursively, a real folder only (`Reconciler::watch_dirs`); the poll source `lstat`s the file on each pass and hints it when its `(len, mtime, dataless)` moved.
  - The service answers with a manifest-only read on a worker (`work::ManifestJob`; refused when the root's identity changed, which asks for a rescan instead), not a full scan, and publishes the new database name, aliases and diagnostics.
  - Backstop: every 60 s status backstop also re-reads the manifest of every library that is up.
  - Tests: H15 through the fake source (re-binds `orders` → `billing` with no full scan) and through the poll source, the backstop with no hint at all, and four Task 9 tests (the filter, the folder hint, the per-folder set, the poll pass).
- **m-f.** The echo's landing path comes from the save's own `join`: the new `fileio::save_echoing_landing` (a Task 5 named exception; `save_with_echo` calls it) hands its callback the path it opened, a file link's target or the name itself. A link retargeted since the last scan no longer records the echo under the old target.
- **m-g: the chain is paced, not cut off.**
  - The wait before a confirming scan is the quiet span, or four times what the library's last full scan took if that is longer, so a large polled library is never rescanned back to back. Past 3 confirming scans in one chain, each wait doubles, up to 30 s.
  - Deviation from the suggested hard cap: a chain that stopped after 3 confirming scans left deleted files listed in 36 of 80 loaded H4 runs. The chain is what brings the listing to the disk when the watcher never reports back (a file that came and went before it settled), so it still runs until a confirming scan finds nothing changed, at a falling rate.
- **m-i.** `LibraryService::settled()` documents that it does not wait for the watcher's own pending settle. Tests wait on a condition first.
- **F3.** The relaunch loop prints `HV-FACT t10 h2 sweep_relaunches=N`.
- **H16** is a test now, through the test-support hook `LibraryService::inject_watch_batch`: a batch tagged with the previous watcher's generation is ignored; the same batch at the current generation is acted on.
- **m-h** is carried to Task 16 (a confirmed "Use this folder" that clears the persisted `canonical`/`inode`). Caveat: a file system whose inode numbers are not stable across mounts (FUSE such as sshfs without `use_ino`, some network mounts) shows such a library `RootChanged` at every launch until then.

## m4b-t13 — explorer projection, file operations, git badges

- **One data source.** The Scripts contributor (`crates/app/hacivat-app/src/scripts/projection.rs`) reads only `WorkspaceSnapshot::view(env)` (R-T10-2), with `env` read from the profile at answer time. Rows, counts, badges and the problems row all come from that view, so a staging connection never receives a `@production` path, not even in a count or a diagnostic. A profile's environment change moves the contributor's generation (it follows `ConnectionsModel`), so the tree is filtered again without a rescan.
- **Placement** (R-M4b-25, -31, -32). PostgreSQL: only the session's database, else the profile's. MySQL/MariaDB: every database M4 asks about. SQLite: `main` only, bound by the main file's stem. With no catalog there is no node. The "shared" badge counts the places M4 would ask about (the same catalog rule as `contributed_places`, duplicated read-only in the contributor rather than exported from the M4 file).
- **Keys** (R-M4b-26). Each key is `xxh3_128("hv-scripts\0" ‖ profile ‖ database ‖ library ‖ kind ‖ rel)`, with every part length-prefixed. The contributor keeps a key → place map per generation. Its lazy children are an `Arc<dyn Fn>` that holds only the key (it reaches the contributor through the app global), so no `Rc` is put in an `Arc`.
- **Per-generation index.** A library's shown listing (children per folder in UI collation order, file badges, the folders that hold a change, counts) is built once per (generation, environment). Paint reads only rows the explorer already holds; no UI work grows with library size per frame.
- **Explorer named exceptions (M4 files).**
  - `connections/explorer.rs` `refresh`: when a contributor's generation moves, its loaded nodes are asked again *in the same rebuild* (`reload_contributed`), parents before children. Before, they passed through "not loaded", and the tree moved the cursor from a script row up to the Scripts node on every library change (pinned by `expanded_scripts_survive_refresh` and the trash test; RED without it).
  - `load_contributed` keeps a key's first owner (T1b review m3; `a_child_key_keeps_its_first_owner`).
  - `explorer/contrib.rs`: `Contributors` is `pub(crate)` (m7).
  - `commands/palette.rs` passes `$os` to every palette title (m9). `hacivat-ui` `tree/view/row.rs`: `status_ink` — New is amber, Conflicted is ink (ruling R-T2-1 a, with a unit test and the `m4b-scripts-tree-prod` shot).
- **The service in the app.** It opens on the core runtime at boot only when `libraries.toml` exists (`install_service`, a named line in `connections/boot.rs`). Task 16's attach opens it on demand (`ensure_service`). With no library nothing starts, so every M4 tree, test and shot is unchanged. One foreground task drains the service's events (`Changed`/`GitChanged` pass every subscription) and calls `contributors_changed` once per drained batch. At quit, and in the test teardown, the service is dropped before the core runtime stops, because its actor holds a blocking-pool thread.
- **File operations** (`LibraryService::{create, rename, trash, refresh}`; T1a's stubs, now filled). They take the caller's `env` (R-T5-6).
  - Names follow the scan's rules on this host *and* on Windows. A script must end in `.sql`/`.md`.
  - The source must be shown under `env`, and so must the result. A folder that holds a listed file `env` does not show is neither renamed nor trashed (`HoldsHidden`): that would move or delete it unseen.
  - All I/O runs under T5's `Dir` chain: no-follow opens, with identity checks before and after. Create is `O_EXCL`/`mkdir`. A rename of a regular file is `link` then `unlink`, so it never replaces anything.
  - **Residual:** a folder, a file link, or a volume without hard links is renamed after an `lstat` of the new name. A file created there in between would be replaced (std has no `RENAME_NOREPLACE`).
  - Each successful operation asks for a rescan (`Msg::Rescan`, also the tree's Refresh). The watcher's report then only confirms.
- **Move to Trash** goes through `ServiceOptions::trasher`. The product's is `OsTrash`, a `trash::TrashContext` with `DeleteMethod::NsFileManager` on macOS, never the Finder `osascript` (T1a m3; a unit test asserts the method). `clippy.toml` bans `trash::delete`/`delete_all`. Tests use `FolderTrash`, so nothing reaches the user's trash.
  - **Residual:** `trash` canonicalizes the parent itself. We verify the `Dir` chain right before and after the call, but a folder swapped for a link inside that window is not caught by us.
- **Deviations.**
  - The name field is a non-modal layer drawn over the row (its bounds from `row_bounds`), not a field inside the tree widget, which has no inline editor. It selects the whole name, because `TextInput` has no range selection.
  - The diagnostics popover is the shipped `ExplainPopover` (300 px). Its "Script library settings…" footer is Task 16's.
  - "Open under another connection…" (Task 15) and "Script library settings…" (Task 16) are shown disabled. `database_menu_items` stays Task 16's (its doc says so).
  - The problems row counts the view's diagnostics plus polling and an untrusted repository.

### Fix round 1 (review C1, I1, m1–m13; rulings R-T13-1, R-T13-2)

- **No rename ever replaces an entry, on any system (C1, R-T13-1).**
  - A file (or file link) is renamed only by `hard_link(old, new)` then `remove_file(old)`. `link` fails when `new` exists. Without hard links (FAT, exFAT, some shares) the rename is refused (`NoSafeRename`); there is no fallback to `rename`.
    - Windows `std::fs::rename` is `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` and replaces a file, so it is never used on a file.
  - **A name already taken** is `Exists` on every system: the exact name, or another entry equal to it in ASCII case. A library is shared with case-insensitive volumes, so `b.sql → C.sql` next to `c.sql` is refused on Linux too. The same check runs before a create (`chain_to_parent` plus a listing).
  - **A case-only change of the entry itself** is proved by a known identity (Unix) or by its name being the only ASCII-case match. `Identity(0, 0)`, every file off Unix, is "unknown" and never proves "same file". The change goes in two steps through a random temp name with a leading dot (the scan skips it), each step after an absence check; a failed second step moves the entry back.
  - **A folder** is renamed after an absence check of its new name.
  - **Residual:** a folder created at that name in between could be replaced on Unix (`rename` onto an empty folder), and on Windows a file created between the check and a case-only step. Std has no no-replace rename and the crate takes no `unsafe`.
- **A folder moves or goes only when it is known (I1).** A folder rename or Move to Trash is refused (`NotFullyListed`, "too large to check; remove it in your file manager") when:
  - the scan did not list its whole subtree: a `ScanStopped` or `Suppressed` diagnostic anywhere (they name no place), or an `Unreadable`, `BadName` or `LinkScope` diagnostic at or under it, or a dataless folder at or under it;
  - a names-only, no-follow walk of the folder at operation time hits 20,000 names, depth 32, an unreadable or non-UTF-8 name, or a dataless folder.

  The same walk refuses `HoldsHidden` when it finds an environment folder `env` does not show, even one the scan never lists because it holds no script (`reports/@production/data.csv`). The walk reads names and metadata only, never a file.
- **R-T13-2:** a rename never changes the environment scope of the entry or anything inside it (`ScopeChange`). Examples: `@staging → common`, or `common → @staging`. The same tag in another case is allowed. Moving between folders was never offered (`OtherFolder`).
- **m1, corrected after the re-review (n1).** `clippy.toml` bans the method `trash::TrashContext::new`, and also the **type** `trash::TrashContext` (`disallowed-types`).
  - A probe showed that a method entry for `trash::TrashContext::default` resolves to nothing: clippy says it "does not refer to a reachable function", because a derived trait impl's method cannot be named by a type path, and `allow-invalid` had hidden that. It caught none of `TrashContext::default()`, `<TrashContext as Default>::default()` or a `TrashContext`-typed `Default::default()`.
  - The type ban refuses all three (probe run, then removed).
  - `os_trash_context`, the one place that names the type, carries `#[expect(clippy::disallowed_types)]`, and its `new()` call `#[expect(clippy::disallowed_methods)]`. Clippy passes with both, so both bans are proven to apply.
  - **Residual:** the crate's `DEFAULT_TRASH_CTX` constant cannot be banned by clippy (it is not a method). T20's hostile suite can add a token rule.
- **m13:** a trash whose after-check fails is `TrashedButChanged`: "moved, but a folder on the way changed meanwhile; check the trash". It is never "nothing changed".
- **App side:**
  - **m2:** an `Alias` refusal, which is another spelling of a name on disk, shows as "already has that name", and the field stays open.
  - **m3:** a name holding invisible or bidi characters is shown escaped under the field.
  - **m9:** the boot looks for `libraries.toml` on the core runtime, never with a stat on the UI thread. Without the file it sweeps orphan recovery folders there.
  - **m10:** "Open under another connection…" uses the chooser's candidate rule: the explorer's places plus each profile's configured database (R-M4b-31).
  - **m11:** file rows of a shared library say "shared" to assistive technology.
  - **m12:** the name field is re-anchored to its row on every frame.
  - The contributor's generation also moves when a catalog lands (shared-ness can change).
  - The row tooltip is one line, its facts joined by ` · `. A newline in it panicked GPUI's text layout on hover; the m7 hover shot found it.

## m4b-cifix-2 — a root's identity includes its birth time

- **Finding (CI 44990057141, Linux only).**
  - `a_root_with_a_new_identity_is_refused` timed out waiting for `RootChanged` on `ubuntu-24.04` and in `turkey-test`.
  - The test hints the root itself through the fake source, so no OS event was missing. The service re-validated the root and found the same identity.
  - ext4 gives a freed inode to the next `mkdir` in the same block group. So a root deleted and made again at the same path usually keeps its inode, and its device.
  - The in-session I3 check compared only `(dev, ino)`, and adopted the new folder. APFS never reuses an inode number, which is why macOS passed.
- **Decision.**
  - The session identity, `fileio::FolderIdentity`, is now the device, the inode and the birth time.
  - The birth time comes from `Metadata::created()`: `statx` `btime` on Linux, `st_birthtime` on macOS. It goes through `std`, so there is no `unsafe` and no new dependency.
  - A rename keeps the birth time. A root that goes missing and comes back by a rename is still the same library.
  - A folder made again is born later. It is `RootChanged`, with `RootRefused(IdentityChanged)`.
  - Where the file system reports no birth time, `born` is `None` on both sides, and the check is the old `(dev, ino)`.
- **The Linux event path is sound; no watch was added.**
  - Each folder has its own non-recursive watch (R-T9-4). notify adds `IN_DELETE_SELF` and `IN_MOVE_SELF` to every non-recursive watch.
  - A replaced root therefore yields one hint on the root itself: notify's `Remove(Folder)` or `Modify(Name(From))` on the watched path. The debouncer drops the child events queued under a removed path.
  - That hint makes the watcher scan. The service's patch, manifest and full-scan paths each re-check the identity.
  - If the hint is handled while the folder is gone, the library is `Missing`. The folder made again sends nothing, since no watch is left. The 2 s re-check of an unavailable root then refuses it.
  - No recursive watch and no parent watch were added.
- **Residuals.**
  - **Same clock tick.** Linux stamps a new inode's birth time from the coarse clock, one tick of 1 to 4 ms. A delete and a `mkdir` inside one tick, on an inode the allocator hands back, keep the same identity.
    - The inode generation (`FS_IOC_GETVERSION`) would close this gap. It needs an `ioctl`, which means `unsafe` outside `hacivat-ui-core/src/platform/**`.
    - Accepted: such a folder sits at the registered path, is a real folder and not a link, and passes the same checks as at attach. It is not a path outside the library.
  - **Across sessions.** `libraries.toml` still keeps only the inode (`m-a`). On ext4, a root deleted and made again between sessions can be adopted at the next launch. Persisting the birth time is a T3 schema change, so it is carried.
- **Tests.**
  - `a_root_with_a_new_identity_is_refused` now waits, on a condition, until the birth clock has passed the old root's.
  - `a_root_made_again_unheard_after_its_removal_is_refused`: the hint arrives while the root is gone, and the new folder sends nothing.
  - `fileio::tests::folder_identity_tells_a_folder_made_again_from_a_renamed_one`.
  - `watch::tests::the_roots_own_removal_and_move_are_hints_on_the_root`.

## m4b-cifix-2 — the debouncer never cancels a path (macOS folder deletes)

- **Finding (CI 44990057141, macOS `build-test`).**
  - `folder_delete_is_seen` saw nothing in 10 s. `per_directory_source_sees_new_and_removed_folders` saw the create and the edit, but not the removal.
  - FSEvents reports a path's recent history in each event's flags. Deleting a folder a few seconds after it was written arrives as `ItemCreated | ItemRemoved` on the folder and on each file. A local probe showed this even 1.5 s after the write.
  - notify splits those flags into `Create`, then `Remove` (`fsevent.rs` `translate_flags`).
  - notify-debouncer-full's `push_remove_event` treats a queue that starts with `Create` as "created, then removed: nothing happened" and drops it. A removed folder also drops its children's queues.
  - Nothing reached the watcher, so the delete was invisible until a full scan. macOS has no backstop (it is Windows only), so in practice that wait has no bound.
  - A desktop Mac usually adds an extended-attribute flag. Its `Modify(Metadata)` comes after the `Remove` and refills the queue, which is why the tests passed locally. The CI runner has no such flag.
  - **A product bug:** a deleted folder stayed listed on macOS.
- **Decision.**
  - The debouncer is built over `NoCancel<RecommendedWatcher>`. This wrapper hands every `Create` to the debouncer as `Modify(Any)`, keeping its paths, flags and attributes. So no queue ever starts with a `Create`, and no path is cancelled.
  - The reconciler takes only paths from a hint, never kinds (alg/28 C3), so nothing is lost. The cost: a file made and deleted within one window, which nothing ever listed, now costs one look or one scan.
  - The wrapper builds its inner watcher with `watcher_config()`, whatever config the debouncer passes (R-T0-2). The `#[expect(clippy::disallowed_methods)]` on it names that reason.
  - It applies on every platform. inotify and `ReadDirectoryChangesW` send real `Create`s, and for them the change only turns "made and gone within 150 ms" into one more hint.
- **Why the per-folder source runs on macOS.**
  - `OsEventSource::per_directory()` is public so that the per-folder logic (R-T9-4) is tested on every runner. On macOS, notify implements `NonRecursive` by filtering a recursive FSEvents stream down to the path and its direct children.
  - Each `watch` or `unwatch` there stops and restarts the stream from "now", so events in the gap are lost. The product never uses it there: `OsEventSource::new()` is per-folder only on Linux.
  - Its CI failure had the same cause as `folder_delete_is_seen`, not the per-folder mode.
- **Tests.**
  - `watch::tests::a_delete_whose_flags_still_say_created_reaches_the_root` drives the real debouncer with a scripted backend that sends the runner's sequence.
    - The control: the bare backend routes nothing.
    - Through `NoCancel`, the folder's path reaches the root.
- **Found while stress-testing; not caused by this change, and not fixed (carries).** Interleaved A/B runs of the binary before and after, under a busy machine, fail at the same rate:
  - **`git_checkout_2000_is_one_rescan`:** 6/80 before and 5/80 after. When FSEvents pauses for a window during the checkout, the first few hundred files form their own burst below the 1,000-path limit, so each is reported `Changed` before the rest overflow into the `Rescan`. The events are true, but the "one rescan" promise of the burst rule is broken. This is the burst rule's quiet-window heuristic.
  - **`folder_move_is_seen_as_renames_for_known_contents`:** when run alone or in a small group (`watch_real folder`), it fails about 9 times in 10, before and after, with an extra `PossiblyChanged(reports/d.sql)`. The whole binary passes. The move happens within R-T9-3's 2 s racy window of the fixture's writes, and a late FSEvents history hint on `d.sql` makes the ruled `PossiblyChanged`, so the test's expectation depends on how long the other tests take.

## m4b-cifix-2 — Windows: n2's library-folder alias, and the dataless hook's spelling

- **n2 on Windows (product).**
  - `non_ascii_case_only_library_rename_is_case_mismatch` answered `New` on Windows. The badge claimed that a tracked, modified file was untracked.
  - `map::same_file` was Unix-only (`(dev, ino)`), since `std` has no stable file id on Windows.
  - It now compares the final paths the system reports for open handles (`canonicalize`). Those spell every component as the disk does, so `Çal` and `çal` resolve alike under case folding. A link or junction on either side is never equal (like `lstat` on Unix).
  - It opens a handle on each candidate folder and reads no file. It runs once per distinct leading index folder that does not already match the library ASCII-case-insensitively.
  - The file-level case-only residual (n3) is unchanged.
- **The dataless hook (test).**
  - `dataless_layout_files_and_split_index_are_unavailable` answered `Ready` for the linked worktree's `commondir` on Windows. The status resolves a gitfile's target and the alternates with `canonicalize`, which gives the verbatim `\\?\C:\…` form, and the testing hook matches paths exactly.
  - The test built its absolute entries from its own `canonical`, which strips `\\?\`.
  - The product's placeholder test reads file attributes and never compares spellings, so this was a test bug.
  - The test now spells absolute entries with `std::fs::canonicalize`. The hook's doc says so.

## m4b-cifix-2 — fix round (review C1, C2, I1, I2; rulings R-CI2-1 to R-CI2-3)

- **C1 (R-CI2-1): the birth time is part of a root's identity on Linux only.**
  - On APFS and HFS+, setting a folder's modification time earlier than its birth time moves the birth time back. `rsync -a` or `tar -x` into the root (a `./` entry), `cp -Rp` and `touch -t` all do it.
  - With the birth time in the identity on macOS, an unchanged library was refused (`RootChanged`, `IdentityChanged`) after a restore. The reviewer's probe showed this end to end.
  - Linux userland cannot set `btime`, and ext4's inode reuse is what needs it. APFS never reuses an object id, and HFS+ CNIDs only grow, so macOS keeps `(dev, ino)`.
  - **Residual:** a network or FUSE mount on Linux may report a settable birth time (an NFSv4 `time_create`, an SMB server's creation time). A restore into such a root can still refuse the library. Such libraries are polled; accepted.
  - The cross-session check stays inode only (`m-a`, a T3 carry); with this fix it matters on Linux only.
  - Tests:
    - `a_root_whose_time_is_set_back_stays_the_same_library`: `touch -t 200001010000 <root>`, then a new file is listed and the library stays `Watching`. It failed with the birth time on macOS.
    - The unit test asserts that a set-back time keeps the identity.
- **C2 (R-CI2-2): index paths are never resolved unless they are plain, on every OS.**
  - The n2 alias loop joined the first components of an **index entry** (untrusted repository content) to the worktree and resolved them. On Windows, `Path::join` with a `\\host\share` component replaces the base, so `canonicalize` would contact the host and answer its NTLM challenge.
  - The same class already existed on Unix: an absolute `/net/<host>` head made `lstat` start macOS's automounter.
  - Now `map::is_plain_index_path` requires every `/`-separated component to pass `gix::validate::path::component` with the Windows, NTFS and HFS protections on (`Options::default()`), and to be one `Normal` path component. That refuses:
    - empty, `.` and `..` components;
    - `\` anywhere, and drive, UNC, verbatim and device prefixes;
    - NTFS streams and reserved device names (`aux.sql` too);
    - a trailing dot or space;
    - every spelling of `.git` (`GIT~1`, `.GIT`);
    - control bytes, and non-UTF-8.
  - `map::alias_candidate` also refuses 8.3 short-name components (`~` and a digit), since an alias is for a case-only rename. It checks that the joined path stays under the worktree, component for component.
  - Where it applies:
    - before n2's `same_file`;
    - at every index entry: an entry that is not plain leaves the index copy, so gix never stats it, and inside the library it is `Unknown(UnsafePath)`, a new `UnknownWhy` with the highest precedence;
    - in `into_library`, where such a path is keyed as the index spells it and never looked up;
    - in `Lister::on_disk`, before any call.
  - **Consequence:** on Unix a tracked name that Windows cannot hold (`a:b.sql`, `x?.sql`, `aux.sql`, a trailing dot) is now `Unknown(UnsafePath)` rather than compared. It is never a wrong badge.
  - Tests:
    - `hostile_index_paths_are_never_candidates`: UNC, drive, verbatim, device, `..` in each form, absolute `/net/x`, empty components, NTFS streams, reserved names, trailing dot and space, `.git`/`GIT~1`/`.GIT`, NUL, non-UTF-8, and 8.3 names (plain, but never an alias);
    - `an_unsafe_index_path_is_unknown_and_never_looked_up`.
- **I1 (R-CI2-3): a repository operation is one burst.**
  - The quiet-window rule split a checkout whose events paused for one window into a burst under the limit, which settled one file at a time, and the rest. That is how `git_checkout_2000_is_one_rescan` failed under load.
  - A hint on `.git/index.lock` makes the reconciler `lstat` it. It looks only at the library's own `.git`, which must be a real folder, never a link or gitfile. If the lock is held:
    - `Debouncer::begin_operation` starts an operation, bounded by `REPO_OP_MAX` = 30 s;
    - the burst does not end on a quiet window, and its distinct paths count across the whole operation;
    - nothing settles but open files (R-T9-6);
    - an overflow's rescan waits for the operation's end.
  - The lock is looked at again once per window, so a removal no hint reports is still seen within a window.
  - When the lock goes, an overflowed operation falls back to the ordinary rule from that moment: one `Rescan` once the hints are quiet for a window. So what FSEvents still delivers after the unlock is swallowed too. An operation under the limit releases its held looks one window later.
  - A lock that outlives the bound is stale. The operation ends there (one `Rescan` if it overflowed), and no new operation starts, and no per-window look continues, until the lock has been seen gone.
  - A repository above the library root gives no lock signal, and keeps the ordinary rule.
  - Tests:
    - `a_checkout_that_pauses_mid_way_is_one_rescan_while_the_index_is_locked`: 1,200 files with a 340 ms pause mid-checkout give `[Rescan]`, and only the open file is reported while the lock is held. The no-lock control reports 1,200 files one by one.
    - `a_small_operation_releases_its_files_when_the_lock_goes_unheard`.
    - `a_stale_index_lock_holds_at_most_once_for_the_bound`.
    - All three fail when the lock hint is ignored.
- **I2.** `watch_real`'s fixtures are an hour old, so no fixture file is racy when a watcher starts. `folder_move_is_seen_as_renames_for_known_contents` run alone: 0/20 failures (before: about 9 in 10).
- **m1.** `NoCancel` does not forward `Watcher::configure` or `paths_mut`. The defaults apply, so the inner watcher can never be reconfigured, link following included. Its doc says so.
- **A lock held for minutes** (`git commit -a` with the editor open holds `index.lock` for the whole message edit) delays unopened files' reports once, by up to `REPO_OP_MAX` = 30 s. After that the lock counts as stale until it has been seen gone. Accepted.

## m4b-cifix-2 — fix round 2 (re-review N1, N2; ruling R-CI2-4)

- **N1: no index-derived path is walked through an on-disk link (completes R-CI2-2).**
  - `map::Lister::on_disk` checked each component's name but still `read_dir`ed a listed folder that was a link: a library folder `link -> /net/<host>` (macOS automount) or `-> \\host\share` (Windows, NTLM), plus a crafted index entry `lib/link/x.sql`, listed the outside folder.
  - The dataless check `lstat`ed `worktree.join(index path)`, which follows every leading component.
  - Now each listing keeps every entry's `file_type()` from `read_dir`. That type follows nothing; on Windows it covers symlinks and junctions, and an unreadable type counts as a link.
  - `on_disk` never descends into a link: every component but the last must be a listed non-link, or, found only by case folding, one whose `lstat` is not a link.
  - The dataless check goes through the same walk (`Lister::lstat_under`) and `lstat`s only the last component.
  - gix's own worktree walk already refuses linked leading folders (`SymlinkCheck`).
  - Listings now index names and NFC forms, so the walk does no scan per lookup.
  - Test: `an_index_path_under_a_linked_folder_never_leaves_the_library`. Its counting seam records every path listed or `lstat`ed. The test also covers the case-only spelling `LINK/x.sql` and the badge path through `into_library`. It fails when the link check is removed.
- **N2 (R-CI2-4): the name rules are the host's, as git applies them.**
  - On every OS, a component must be:
    - one `Normal` path component of valid UTF-8;
    - not empty, `.` or `..`;
    - without `/` or NUL;
    - never a `.git` spelling.
  - The Windows and NTFS protections (`\`, drive, UNC and verbatim prefixes, streams, reserved device names, trailing dot or space, `GIT~1`) apply on Windows only, and the HFS+ `.git` spellings on macOS only.
  - The fix round's consequence for Unix is gone: `con.sql`, `aux.sql` and `a:b.sql` keep their true badge on Linux and macOS, and are `Unknown(UnsafePath)` on Windows (`windows_only_names_keep_their_badge_off_windows`).
  - A backslash is an ordinary character on Unix and cannot reach a UNC path there.
- **N3 (monorepo lock signal): carried to T20, not done here.**

## m4b-t14 — script tabs

Task 14 fills `scripts::{tabs,open,save,disk_bars,restore,unsaved_prompt}` and the named lines it needs in the console host, the exec orchestrator, the close and quit prompts, the shell and the explorer. The one rule: **a script tab runs only on the (connection, database) it was opened under, for its whole life.**

- **A script tab is a console host with a `ScriptDoc` beside it.** `console::view::build_script` builds the M3 host (toolbar, capsule, hairline, run UI, find bar, transaction UI unchanged) for `TabKind::Script`; the doc owns what the file adds: the read, dirty state, notices, disk events, recovery copies and the database check before Run. The host's `n` for a script tab is `2^30 + handle` (`tabs::client_number`), far above any console number, so its own session's `ClientId` never meets a console's.
- **Binding.** `open_script(cx, library, rel, profile, database)` resolves a missing database at open (R-T1b-1: the session's database, else the profile's; `main` on SQLite) and stores it in the handle; nothing writes another profile or database into a handle afterwards. A tab follows a rename by its path only (`tabs::follow_rename`), after the new path's scope is checked (R-T10-2, carries-w1).
  - R-M4b-30: `ConsoleHost::on_connections` skips script hosts (the R-M3-37 auto-bind), and a tab whose profile is gone has no binding at all and shows "The connection this tab was opened under was deleted." Its capsule shows no connection (`connections::capsule::current_profile`, named line): a followed script tab that runs nowhere never shows another connection.
  - `explorer::live::user_session` (named line) skips script consoles: a MySQL tab's switched session is the tab's, so neither the explorer's current database nor a script opened without a database follows it.
- **Sessions (R-M4b-25, R-M4b-27; `open::session_plan`).**
  - PostgreSQL and SQLite share the profile's sessions as a console does (slot 1 takes the default session). PostgreSQL opens only on the database its session (or, unconnected, its profile) is on; SQLite only on `main`.
  - **Deviation (stronger than the plan):** every MySQL/MariaDB script tab opens a session of its own (slot 0, never the default one), not only one under another database. Why: the pre-Run check switches a MySQL session back to the tab's database; on a session shared with Console 1 that switch would move Console 1's database too. Cost: one session per MySQL script tab. A single-session MySQL profile shares its one session only on its own database (and then a mismatch refuses instead of switching); a script on another database is refused at open, and a profile that later hands back its default session to such a tab gets "couldn't switch" and runs nothing.
  - Under a database other than the profile's, the tab sends one `Request::SwitchSchema` when it takes its session; Run stays off until the session reads the database back.
- **The check before every Run** (`tabs::run_gate`, called from `orchestrator::start_run_capped`, the single submit point). Markdown, a T2 preview, an unbound tab and a tab whose open switch failed never run. Then the session's database must equal the tab's.
  - **Finding (R-M4b-27 as written cannot hold alone):** a session never probes its schema at connect (`session/schema.rs`: "never at spawn"), so `SessionSnapshot.schema.database` is `None` until a statement that may move it. When it is unknown, the gate asks the session itself (`Request::Exec` of `schema_probe_sql`, internal and idempotent, queued on the same session) and runs the same Run again with the answer; a session that still cannot say is refused ("Hacivat can't tell which database its session is on"). Never a guess, never the profile's configured name. Cost: one round trip before a Run on a session that has not reported its database yet.
  - MySQL (own session): a mismatch is answered by one `SwitchSchema` back and the same Run again, which checks again; still off → refused with a notice naming both databases. A reconnect lands on a fresh connection that reports nothing, so the probe and the switch above run before the next statement.
  - PostgreSQL (and a shared MySQL session): a mismatch refuses ("the connection was edited"). SQLite: always `main`. Residual: an SQLite profile whose file is edited to another file while a tab is open runs the tab on the new file (its title still names the connection).
- **Save** (`scripts::save`, R-M4b-5, -16, -17, -38). Only `file.save` (⌘S/Ctrl+S in `ScriptTab`) and `file.save_all` save; they are no longer stubs. The text is checked with `scan_secrets` over the history redactor's spans (R-M4b-28); until Task 17, a finding asks in a minimal Save anyway / Cancel confirm that names no value. The save expects the stamp the tab read or saved (`None` after "Deleted on disk"); `ChangedOnDisk` asks "Overwrite?" (Cancel focused). `SavedButPathChanged` is a save plus a notice; a preview is never saved (Save disabled; `PreviewOnly` if it ever reached the service). The env goes to every file call (R-T5-6). A save is an external change for the file's tabs under other connections (the service suppresses its own echo, so `tabs::saved_elsewhere` tells them): clean ones reload, dirty ones get the bar.
- **Disk events** come from a `Subscription` per tab for its environment (re-subscribed when the profile's environment changes, with a scope re-check). Clean → `replace_from_disk(text, clean_version)` (R-T11-1), or `reload_preview` for a preview; dirty (or "Keep mine") → "Changed on disk". A composition's marked text is the tab's text, so it shows the bar, never a reload (R-T11-1).
- **Recovery** (R-M4b-5, R-T5-3..5): 5 s after the last edit of a dirty tab its text is stored under `<data>/script-recovery/` (the instance lock is taken on the blocking pool the first time). A clean tab, a save and "Don't save" remove it. At launch (`restore::recover_at_launch`, after the workspace opened, only when `libraries.toml` exists) the copies of instances that are gone open as dirty tabs bound as they were, with the notice and a launch toast; a `liveness_unknown` copy says it may belong to another window and is kept until Save or an explicit Discard.
- **Restore** (`shell::workspace`, named lines): a saved script tab's fields are read leniently, so a wrong-typed field closes only that tab (T1b m1); a second tab with the same binding is dropped (T1b m2); the profile is kept by its ID (a deleted one shows its own state, T1b m6).
- **Close and quit** (R-M4b-39): the unsaved-changes prompt runs in M3's guard chain after the transaction prompts and holds M3's one-prompt slot (`close_prompt::hold_prompt`, named lines). Quit and restart ask per dirty tab ("1 of 2 unsaved scripts").
- **Service additions** (`hacivat-scripts`, named lines): `LibraryService::read_with` (the encoding question, NOTES surface 10), `forget` (the last tab of a file closed, carries-w1 T14/T10) and `shutdown` (the app stops the service at quit, so the core runtime never waits for its actor; the shots and test teardowns call it too).
- **Autosave**: `console::autosave::allowed` keeps script tabs out of `<data>/consoles/` (R-M4b-5).
- **Not in this task (routed):** the encoding question offers four encodings as buttons, not the mock's Select, and "Open read-only" is not built (it needs a lossy `fileio` read; T17/T21); the tab title's middle-truncation (ruling d) is the tab strip's own ellipsis; "Open under another connection…" opens one non-production place directly and otherwise waits for Task 15's chooser (`open::set_open_under_another`).

### Fix round 1 (review C2 / I7; controller rulings R-T14-1..4)

- **C1, R-T14-2: Run asks the server, never a cache.** Before every Run the gate sends the engine's schema probe (`current_database()`, `DATABASE()`) on the tab's own session, then reads the session's context generation (`run_context`) right after it on the same queue, and runs the same Run again with the answer. The orchestrator sends the run only under that generation (`tabs::generation_holds`, checked after the confirmations, where the pipeline reads the generation it hands the session); a reconnect, a replay or a queued `USE` in between moves it, refuses the run ("connection changed") and the next Run checks again. The session also refuses a run whose generation moved (M3b). The session actor re-probes after a reconnect's replay (named exception in `hacivat-session/src/session/actor.rs`), and a read that fails leaves the database unknown, never the old connection's. Cost: one probe and one context read per Run of a script tab.
- **C2, R-T14-1: a SQLite tab is bound to its file.** `open_script` reads the profile's database file on the blocking pool (canonical path and, on Unix, `dev:ino`) and opens the tab once it is known; the identity is kept per handle (`tabs::SqliteFiles`) and in `workspace.json` (`sqlite_path`, `sqlite_id`, lenient, left out for other tabs). Every Run re-reads the profile's file now and refuses unless it is the same file. Windows compares the canonical path only (no stable file id without unstable APIs); residual.
- **I1: restored tabs wait for the listing.** A tab's first read waits (bounded, 10 s, on the blocking pool) until the service has read its registry and its library's first scan ended; "Opening…" shows meanwhile.
- **I2: launch recovery reaches restored tabs.** `restore::offer` hands a copy to the live doc of its binding (`ScriptDoc::apply_recovered`): set over the disk text (one undo step back), dirty, with the notice; a tab still reading takes it when its file is read.
- **I3, R-T14-3: a MySQL tab never shares a session.** A single-session profile, or one whose budget is under 2 sessions, refuses MySQL/MariaDB script tabs ("This connection allows one session; close the console or raise the limit."); a profile that hands back its default session to such a tab shows the same and runs nothing. The check-to-submit gap is closed by C1's generation.
- **I4:** "Don't save" during a quit is noted in the quit round and takes effect (the flag, the recovery copy removed on the quitting thread) only when the quit goes on; a cancelled quit leaves every tab dirty, asked again, with its copy.
- **I5:** the unsaved prompt's body starts with "environment · connection · database". The Danger-style isolation of "Don't save" at the far left is not built: `hacivat_ui::dialog` lays its footer out evenly and has no spacer (a widget change outside this task).
- **I6, R-T14-4 (overrides the mock):** after "No longer listed under this connection" the tab is fail-closed: Run and Save are off (the gate and `file.save` refuse with the reason), and the notice offers Close and, where the file is listed, "Open under another connection…". NOTES surface 6 is changed to match.
- **Minor findings fixed:** discard-and-reload reads in the file's encoding and refuses a preview or an open composition (1, 2); a followed rename removes the old path's copy (3); "Open under another connection…" is disabled with a reason when only Task 15's chooser could pick (4); absence asserts wait on the refusal they prove (5); the one-prompt slot is held only where a window will ask (7); the M4 `user_session` block is fenced (9); a second Run press during a check or a switch back waits (10); the reason line is a sentence (11); a connected production script-tab shot shows the capsule, hairline and status segment (12); the last tab's `Forget` is queued synchronously (`LibraryService::forget_now`) (13); a closed tab's handle is un-interned (14).
- **Minor findings not fixed:** (6) the editor's accessible name is cached by the editor (`a11y_label_cache`, keyed by the localization generation), so a rename does not refresh it until the language changes; refreshing needs an editor API. (8) A garbled `profile` restores the tab unbound rather than dropping it: it runs nothing (fail-safe), and T1b's `a_saved_script_tab_with_a_bad_binding_closes_and_one_without_a_profile_stays_unbound` codifies it.

### Fix round 2 (re-review fr1: I-fr1-1, m-fr1-1 to m-fr1-7)

- **I-fr1-1: the post-replay probe is bounded.** The schema probe after a reconnect is now the last step of the replay future. It runs inside the same `timeout(connect timeout)` and the same Close-serving `select!` as M3's replay.
  - A probe that hangs counts as a timed-out replay: the connection is closed, and the next attempt follows M3's backoff and give-up clock.
  - A Close during the probe ends the session at once.
  - A probe that reads nothing on a working connection leaves the database unknown (`schema` is `None`, published), so a script tab's Run refuses.
  - The last schema is kept in `kept_schema` for the next replay's plan, so M3's `USE` / `search_path` restore is unchanged. A probe that reads clears it.
  - The schema is also published as unknown while a failed replay retries.
  - Tests in `hacivat-session/tests/manager_m2b.rs` (all three fail on the fix-round-1 code):
    - `a_close_during_the_probe_after_the_replay_ends_it_at_once`;
    - `a_probe_that_hangs_after_the_replay_times_out_and_the_next_attempt_follows`;
    - `a_probe_that_fails_after_the_replay_leaves_the_database_unknown_and_keeps_the_use`.
- **m-fr1-1: SQLite checks the session's file too.** Each Run reads `PRAGMA database_list` on the tab's session and compares its `main` file (canonical path and `dev:ino`) with the tab's file and the profile's file. Any mismatch refuses, naming the tab's file and the other one. An in-memory or unreported `main` refuses, so the check fails closed.
- **m-fr1-2: some tabs take the profile's file on Run.**
  - A SQLite tab with no stored file (launch recovery without a layout entry, or a layout written before fix round 1) takes the profile's file at its first Run, as a fresh open would. The open no longer refuses it.
  - After a remount (same canonical path and inode, another device number), the tab follows its file.
  - A file replaced at the same path (a new inode) still refuses.
- **m-fr1-3:** the probe is queued before the context generation is read, and the comment now says so. A move between the two bumps the generation, which the orchestrator refuses.
- **m-fr1-4:** `RunChecks` records the session the check was made on. `generation_holds` requires both that session and the generation.
- **m-fr1-5: "Don't save" removes copies off the UI thread.** At quit, its recovery copies are removed on the core runtime's blocking pool. The scripts' quit hook (`tabs::install`) waits for those removals.
- **m-fr1-6:** `open_script` always answers `None` for SQLite, because its tab opens once the file identity is read. Task 15 must find the tab by its binding (`tabs::doc_of_script`), never through this return value. This is documented on `open_script`.
- **m-fr1-7: only restored tabs wait.** The bounded "listed" wait runs only for a handle restored at launch, from the layout or from launch recovery (`tabs::mark_restored`). A tab opened from a listed row reads at once.
- **m-fr1-8:** the shot-harness flake (`marks-awaiting`, `fold-failed`) is carried to T21.

### Fix round 3 (re-review fr2 minors: m-fr2-1 to m-fr2-3; ruling R-T14-5)

- **m-fr2-2, R-T14-5: a remount is followed only on the first Run after a launch restore.** "Remount" means the same canonical path and inode, but another device number.
  - On a tab restored at launch (from the layout or from launch recovery), the first Run may follow the remounted file. It shows "The database file is on a different volume now; running on <path>." and records the file again.
  - On any other Run, a device change refuses like any other identity change ("Run refused"), until the tab is reopened.
  - **Caveat 1, FAT and exFAT:** these file systems have no stable inode. The OS synthesizes the number from the file's directory position, so two different sticks with the same layout can match on path and inode. An APFS clone also keeps its inode number.
  - **Residual:** a swapped volume at the same mount path can therefore be followed on that one first Run, but never mid-session. The notice names the path, so the user can see it.
  - **Caveat 2, Windows:** it compares paths only, as recorded in fix round 1.
- **m-fr2-1: a recovery copy records the SQLite file.** The copy's sidecar now stores the bound SQLite file's canonical path and `dev:ino` (`db_file_path`, `db_file_id`). Both fields are lenient: absent in older copies and for other engines. To carry them, `CopyMeta` gained `db_file` and is no longer `Copy`; this is a named exception in `hacivat-scripts`.
  - When launch recovery opens a SQLite tab that has no file of its own, the tab takes the copy's file.
  - If the profile was repointed between the crash and the relaunch, Run refuses with the "Run refused" notice naming both files; it never adopts the new file silently.
  - A copy written before this change still takes the profile's file at the first Run (fix round 2's rule).
- **m-fr2-3:** a comment on `QuitDiscards` now says that only the quit guard pushes to it (when the quit goes on) and only the quit hook drains it. Its futures are `!Send`.

## m4b-cifix-3 — a polled library never misses a manifest change

- **Finding (CI 45068616161, macOS).**
  - `h15_manifest_change_rebinds_on_a_polled_library` timed out waiting for the new binding.
  - The poll source took its first stamps of `.hacivat/workspace.toml` and of the repository files at its first pass, one interval after `watch`. The service's scan reads the manifest after `watch`.
  - A rewrite that landed between that read and the first pass became the baseline, and no pass reported it. Only the 60 s backstop re-read the manifest. In the app the gap is up to one interval, 2 s. In the test it is 30 ms plus however late the poll thread first runs on a loaded runner.
  - The stamp was `(len, mtime, dataless)`. A rewrite that kept the length inside one modification-time tick was never seen. Network shares, the reason a library is polled, often keep coarse times.
- **Decision.**
  - `PollEventSource::watch` takes the repository and manifest baselines itself, on the caller's thread, before it returns. The watcher calls it before its first scan, so a change is caught either by that scan or by the next pass.
  - The manifest's stamp also carries the XXH3-64 of its bytes. `manifest::content_hash` reads them with the service's own reader: directory handles, no link followed, a plain file of at most `MANIFEST_MAX_BYTES`.
  - A dataless manifest is never read, because reading it would download it. Its stamp keeps `dataless = true` and no hash.
  - This is the only content a pass reads. Scripts are still compared by `lstat` alone ("never read what you only list"). A manifest is tiny and the service reads it on every scan anyway.
- **Tests.** In `tests/watch.rs`:
  - `a_manifest_rewrite_before_the_first_poll_pass_is_hinted`. RED: no pass named it in 10 s.
  - `a_same_length_same_mtime_manifest_rewrite_is_hinted`. RED: 358 passes, none named it.
  - `an_unchanged_manifest_is_never_hinted_by_a_poll_pass`, the control.
  - `h15_manifest_change_rebinds_on_a_polled_library` no longer sleeps 150 ms before the rewrite.

## m4b-t15 — Go to script and the connection chooser

Task 15 fills `scripts::{goto,chooser}` (HV-SCRIPT-09, R-M4b-9, R-M4b-31; NOTES surface 7) and wires the "Open under another connection…" items Tasks 13 and 14 left disabled. The one rule: **production is never chosen for the user**, on every path that opens the chooser.

- **Its own layer, the palette's anatomy.** `scripts.goto` (⇧⌘O / Ctrl+Shift+O, R-M4b-37; now enabled, `STUB_COMMANDS` holds only `scripts.attach`) pushes layer `scripts-goto` (context `Palette`, non-modal) at the command palette's place and width, with the shipped palette's query field, 23 px rows, 12 visible rows, popover surface and 24 px key-hint footer. **Deviation:** it is a view of its own, not `hacivat_ui::palette::Palette` with `set_items`: the widget draws one title per row and always has an active row, while these rows carry a folder with its environment mark, a library, the chooser's columns (mark, connection, database, "current", engine), and the chooser must be able to have **no** active row. Extending the shared widget would have been a UI-crate change under every command-palette shot; the cost is about 200 lines of layout that copy the widget's tokens.
- **Results.** Names only (R-M4b-15): `ScriptIndex::build` over the snapshot, off the UI thread, once per snapshot generation; every query runs `ScriptIndex::query_scoped` (new, named exception in `hacivat-scripts/src/index.rs`) with `commands::palette::search_opts` (I18N-09, D-074). `query_scoped` filters **each library by the environments of the connections it is shown under** (`projection::script_places`); a library no connection shows offers what an unbound tab sees (unscoped files). Why not `query_for_any` over every profile's environment: a `@production` file of a library only staging shows would be listed because some other library is shown under production. Past 2,000 scripts (`ScriptIndex::is_large`, a yes/no, never a count: T10 m2) a query runs off the UI thread; answers carry a sequence number and only the latest is shown. The 256-character cap is the index's. Names, folders and libraries are the index's escaped display text; connection names go through `display_name` too.
- **Candidates** (`chooser::candidates`): every place of `projection::script_places` (the databases the explorer asked about, C-M4-3, plus each profile's configured database, R-M4b-31; PostgreSQL limited to the database its session or profile is on, R-M4b-25), whose binding includes the library, whose `view(environment)` lists the file as an openable file, and where `open::session_plan` allows it (so a row never opens a refusal), minus the connection a tab or tree row is under, every database of it (fix round 1, R-T15-1; first written as "the place"). T13's `shown_under_another_connection` and T14's `other_places` now call it (named exceptions), so a menu item or notice button is enabled exactly when the chooser it opens has a row.
- **Order and the active row** (`chooser::order`, `preselect`): the active tab's connection (its own database first), then local, dev, test, staging, production, then untagged, then the connection's name in the UI language's order. "The focused connection" is the **active tab's** (`ActiveTab` → its script binding or its console's profile), never the explorer's selection or a tree row. The active row is the first that is not production or is the focused connection; with only production elsewhere no row is active and Enter does nothing until the user moves onto one. The footer adds "Production is never chosen for you." whenever a production row other than the focused one is listed.
- **A pick** (`chooser::decide`): one non-production candidate opens at once; production or two or more show the chooser; none offers "Open without a connection" (an unbound tab, R-M4b-30) — only when an unbound tab can show the file, otherwise the hit is stale and the results are refreshed. Opening goes through `open_script`, which focuses the tab already open for the same (file, connection, database) (R-M4b-4).
- **Live rows, no stale Enter.** The chooser's rows are recomputed when a profile changes (`observe_global::<ConnectionsModel>`) and on every library-service event, and **again on every Enter, before anything opens**, from the live snapshot and profiles (the review probe "Enter racing the candidate computation"; `enter_checks_the_live_snapshot_before_opening` moves the file into `@production` and presses Enter before the event reaches the layer). A row the user moved onto stays active only while it is the same place **in the same environment**; a retag sends it back to the preselection rule, so a staging row that becomes production is never left active.
- **"Open under another connection…".** The tree file row's item is the new command `scripts.open_under` (`ExplorerTree`, menu and palette only, title `hv-cmd-scripts-open-under` in `## m4b-t15-goto`); it opens the chooser without the row's connection (every database of it, R-T15-1). The script tab's notices call `open::open_under_another`, whose hook (`open::set_open_under_another`) now opens the chooser without the tab's connection (R-T15-1). Both leave the tab where it is: a new tab or the focused existing one, never a rebind. Escape closes a chooser opened from a menu and goes back to the results from Go to script.
- **Focus** (R-M4b-43): the query field holds the focus in the results, the chooser's own focus handle (a Tab stop) in the chooser; the view is a focus home whose primary follows the mode, so rows that vanish never strand the focus. Closing restores the focus the layer recorded; an open hands it to the new tab's editor.
- **Not built:** recently opened scripts first on the empty query (the index's order is used); the chooser header draws the file name in the header's sans face, not mono (one Fluent message).

### Fix round 1 (review C1, I1, minors 1–8; controller rulings R-T15-1, R-T15-2)

- **C1, R-T15-1: "Open under another connection…" leaves out the whole starting connection.** `chooser::candidates(library, rel, exclude: Option<ProfileId>, cx)` skips every place of the excluded profile, not only its (profile, database). So a MySQL production tab on `api` whose library is also bound to `kampanya` (an alias or a local mapping) never offers `api-prod · kampanya`, and production is never preselected on an "another connection" path (NOTES cross-cutting 6, surface 7, the mock's `c.id !== exclude`). T13's tree item (`shown_under_another_connection`), T14's notice button (`other_places`) and the tree command all go through this function; the workspace tab strip has no tab menu yet, so there is no other caller. Tests: `open_under_another_from_a_mysql_tab_never_offers_its_own_connection` (the reviewer's H2) and `tree_open_under_never_offers_another_database_of_the_rows_connection`.
- **I1, R-T15-2: activation opens only what the user saw.** Enter or a click records the row shown active (connection, database, environment), or "Open without a connection", then re-checks the rows against the live snapshot and profiles. It opens that row only if it survived unchanged, and "Open without a connection" only while there is still no candidate; otherwise it redraws and opens nothing. The preselection rule is never applied at activation time. Tests: `enter_never_opens_a_row_the_user_did_not_highlight` (the reviewer's H1: an alias removed on disk drops the highlighted staging row before Enter) and `a_click_never_opens_a_row_the_user_did_not_click` (the same with a click, dispatched before the event's redraw).
- **Minors.**
  1. `script_places` syncs the per-generation caches before it reads the snapshot.
  2. A chooser row's accessible name starts with the environment word.
  3. A Go to script hit listed only where the script may not run (a single-session MySQL profile, R-T14-3), and an Enter on a chooser with no row and no unbound open, both go through `open_script`, whose refusal says why (`chooser::refused_place`).
  4. In `index.rs` the T15 items are inside one `## m4b-t15-goto` fence: the `HashMap` path is spelled out, and `LARGE_INDEX` is an associated const.
  5. The layer closes by its view (`Workspace::remove_layer`), wherever it is in the stack.
  6. A refresh of the same query keeps the active result by its (library, path).
  7. The header draws the file name in mono: the Fluent message is split around its `$file` argument (a private-use marker), with a whole-message fallback.
  8. The T1b routing test is renamed `goto_script_without_a_library_lists_nothing`.
- **Not changed:** "recently opened" on the empty query (no store of recently opened scripts exists; alg/28 lists recency only as a tie-break), and the layer's own view instead of `Palette` + `set_items` (the deviation recorded above).

### Fix round 2 (re-review fr1: I-fr1-1, m-fr1-1 to m-fr1-3)

- **I-fr1-1 (R-T15-2 for clicks).** A chooser row's click carries the identity of the row it was painted for, `goto::Seen::Row(connection, database, environment)`, and "Open without a connection" carries `Seen::Unbound`; neither carries an index. `GotoHost::activate(seen)` re-checks the rows and opens only that identity while it is still a candidate (unbound: while there is still none). It never uses the current active row, so a row gone since the last frame, a reordered list or a candidate that just appeared opens nothing and redraws. Test: `a_click_with_a_stale_row_identity_opens_nothing` drives the handler's entry point with stale identities, so the test window's redraw after every update cannot hide the race.
- **m-fr1-1.** Enter acts on what the last frame drew as active (`painted`, recorded while drawing and after the user's own ↑/↓, never by a refresh), under the same rule.
- **m-fr1-2, m-fr1-3.** The original bullets above now say "connection" (R-T15-1), and the `chooser.rs` module doc is wrapped at 100 columns.
## m4b-t16 — attach dialog, manual mapping, Settings › Script libraries

**What is written into a library, and how it is shown first (L14, R-M4b-8, Q5, Q6).**

- **The attach dialog shows the bytes it writes.** The folder is read on the core runtime before anything is offered: `manifest::preview` (new) reads `.hacivat/workspace.toml` through the same no-link `Dir` handle as `manifest::read` (no link followed, never a FIFO, at most 64 KiB), and `envscope::root_env_folder` checks the root. With no manifest, the inset shows `ManifestV1::to_toml()` of the typed name and aliases, line by line, and Attach hands that same `ManifestV1` to the new `LibraryService::attach_writing`, which writes its `to_toml()` (`manifest::write_new`). `ManifestV1` has exactly `format`, `database` and `aliases`, so no profile id, connection name, host, port, user, keyring slot or password can reach the folder from the dialog.
- **Never over a file that appeared meanwhile.** `attach_writing` refuses `Manifest(Exists)` and registers nothing when a manifest appeared between the dialog's read and the press (the older `attach(.., write_manifest: true)` silently read it instead). The dialog then reads the folder again and shows what it found.
- **A found manifest is shown, never rewritten.** Its text (comments included) is shown read-only; Attach calls `attach(path, None, false)`. A manifest that is there but cannot be read is said so, and the folder binds by its own name until it can be read.
- **The shared alias is written only over what was shown.** `manifest::with_alias` (new) is the pure edit that `add_alias` now runs (behaviour unchanged, through `toml_edit`, comments, line endings and the BOM kept). The mapping dialog shows its result over the bytes `manifest::preview` read, the added line on the selection tint. `LibraryService::map_sharing(m, before)` (new) writes it only while the file is still exactly `before` (`manifest::add_alias_if_unchanged`), else `Alias(Changed)` and nothing changes. The default mapping writes `libraries.toml` only, so the folder's bytes are identical before and after (Q5).
- **Tested byte for byte:** `scripts_attach.rs` compares the written file with the lines the dialog drew, checks that its keys are exactly `format`, `database`, `aliases`, that no profile name, host, user, environment or profile id appears in it, and that the rest of the folder is unchanged. Mutations that write one more alias than shown, or share the alias on the one-click mapping, fail those tests.
- **Untrusted text** (the path, the found file's lines, its binding, library and connection names) is shown through `escape_untrusted` (bidi and invisible characters as `<U+XXXX>`); a found file's lines are cut at 512 characters each, and each line is a truncating data label (its tooltip has the whole line).

**Where the Scripts items appear (R-M4b-25, R-M4b-32, R-M4b-34).** `scripts::database_menu_items` answers "Attach script library…" and "Map to a script library…" only on a node where a Scripts node can show: the PostgreSQL database the profile's session is on (or, without a session, its configured database), every MySQL/MariaDB database, and the SQLite profile root. A PostgreSQL schema row, another PostgreSQL database and a SQLite attached database get no Scripts items (carries T1b m11). The placement rule is `projection::place_logical`, the same `match` as the Scripts node's. "Map…" is disabled with "No other library is attached" when every library already shows there. The attach's "Shows under" list uses the explorer's places plus each profile's configured database (R-M4b-31), as Go to script does.

**Refused up front.** A folder inside an `@<env>` folder, a folder already attached, and the 32-library limit disable Attach with an ink line. The service's own refusals (a link, the home folder, an app folder, a filesystem root, a nested library, a read-only registry) come back as the same line. Nothing is written in any refused case.

**R-T9-1: Watch: automatic / poll.** The registry row gains an optional `watch = "poll"` (`registry::WatchMode`). It is written only when not automatic, and any other value reads as automatic, so a hand edit never stops a library being watched. `LibraryService::set_watch` saves it and restarts an up library's watcher with `Backend::Poll(PollReason::UserChoice)` (Task 9's `Watcher::open` keeps a poll backend the caller sets). The snapshot carries `LibrarySnapshot::watch`. A chosen poll is not a library problem in the diagnostics popover. A library nothing watches (`Unwatched`) now is one, and the popover suggests "Watch: Every 2 seconds".

**T10 m-h: "Use this folder…".** For a library in `Unavailable(RootChanged)`, Settings shows a confirmed action (Cancel is the default). `LibraryService::adopt_root` refuses any other state. It forgets the recorded canonical path and inode (`LibraryRegistry::clear_resolved`) and this session's resolution, then validates the root from scratch: every other check still applies, so a link root, an app folder or an overlap is refused again (tested with a link). The library keeps its id, its mappings and its tabs' bindings. A detach and re-attach would lose the mappings.

**R-T10-7: an unreadable manifest keeps the last good binding.** When a manifest that bound the library (`database` or aliases read and understood) is later present but unreadable (`manifest::is_read_failure`: a link, not a file, changed while read, an I/O error, too large, not UTF-8, too deep, invalid TOML, missing or bad `format`, a bad `database`), the actor keeps the last `db`/aliases and shows the diagnostic instead of falling back to the folder name, which could bind another database. A newer `format` still binds by folder name (R-M4b-21), and a removed file binds by folder name as before. Residual: at launch there is no last good binding, so an unreadable manifest binds by folder name, as before.

**Keys and focus (R-T33-1, R-M4b-43).**
- Both dialogs are modal layers, which are focus homes (`shell::layers`). Tab is trapped in NOTES order, and every button is a Tab stop; the header × is not.
- The fields are real window Tab stops through a new opt-in, `TextInput::set_tab_stop`. This is a named exception in `hacivat-ui`: GPUI reads the flag from the tracked handle. It is off by default, so no existing form changes.
- Escape (a field's `Cancelled` too) cancels. Enter presses the focused button, and in a field it attaches.
- The Settings section is a first-stop focus home, so a removed row's focused button hands the focus to "Attach a folder…".

**Named exceptions.**
- Core files: `registry.rs`, `manifest.rs`, `service.rs`, `service/actor.rs`, `service/work.rs`, `lib.rs`.
- App files: `scripts/{mod, projection, diagnostics, file_ops}.rs` ("Script library settings…" enabled), `settings_ui/{mod, rows}.rs` (`Section::Scripts`, 9 sections).
- Tests: `tests/m4b_routing.rs` (the stub count, the filled menus, the two new rows) and `tests/shots/m4b.rs` (five states).
- UI: `hacivat-ui/src/input/mod.rs` and `hacivat-ui/src/checkbox.rs` (`Checkbox::wrap`; fix round 1, m8: the stacked layout's label is a `paragraph`, which takes only a static id, so it has no `checkbox-label-<id>`; no shot marker uses it).

**Deviations from the mock.**
- The inset is plain mono ink, not syntax-coloured.
- The polling reason is a caption line under the state, not a tooltip: the settings rows have no tooltip primitive.
- The added alias line in the mapping inset is the whole edited line (`toml_edit` rewrites the `aliases = [...]` line).

### Fix round 1 (review I1–I6, m1–m10; ruling R-T16-1)

- **I1 (R-T10-7 said truly).** When a broken manifest's last good binding is kept, the snapshot carries `LibraryDiag::ManifestKept(<database>)`. The popover shows one line in place of the read failure: ".hacivat/workspace.toml in <folder> can't be read, so the library keeps its last binding, <database>, until the file is fixed." The Settings row shows the same line, through `diagnostics::manifest_problem`; a test compares the two.
- **I2 (no inherited binding).** An adopted folder is read from scratch: `manifest_good`, `db`, aliases and the manifest diagnostics are reset. A root refused as `RootChanged` also forgets `manifest_good`. Only the user's local mappings stay; they belong to the library, not to the folder. The review's probe is now `an_adopted_folder_never_inherits_the_old_binding`.
- **I3 (transactional "Use this folder").**
  - `adopt_root` records nothing up front. It marks the library, and the next validation runs with only the identity checks waived (`RefreshJob::waived`: no recorded or session identity). The new canonical path and inode are saved only when that validation passes (`commit_adoption`). A refusal leaves the old record and `RootChanged`, so a folder that later takes the path is never adopted without a new confirmation.
  - `adopt_root` now answers once the validation is done: `Ok(true)` adopted, `Ok(false)` refused.
  - It is accepted, and the button is offered, only for `RootRefused(Moved | IdentityChanged)`, never for a link, a non-folder, an overlap, the home or an app folder (`LibrarySnapshot::can_adopt`).
  - Residual (m4): the confirmation is bound to the refusal reason and the path, not to a folder identity seen when the dialog opened (the snapshot exposes no identity). A swap between the question and the press is validated afresh and, if it is another real folder, adopted.
- **I4.** A found manifest with a read failure, or a newer `format`, heads the dialog with "Found .hacivat/workspace.toml, but it can't be read (<reason>); the library will bind by its folder name, <folder>, until it is fixed." There is one reason per diagnostic kind (`hv-scripts-manifest-reason`).
- **I5 (R-T16-1).** The mapping dialog's "Attach another folder…" opens the attach dialog with `Prefill::Folder`: the Database field shows the chosen folder's name, never the node's local name, and follows Change… while untouched. A new file can be attached, by Enter or click, only after a frame drew its fields and text. The plain database-menu attach keeps the node's name (the plan's "prefilled with the node's database name").
- **I6.** Every Settings action (Remove, "Use this folder", Watch, a mapping's Remove) reports what it could not do on its row (`hv-scripts-set-failed`: read-only registry, the library changed meanwhile, the folder refused, a failed save). The error clears on the next attempt.
- **Minor findings.**
  - m1: the inset draws at most 200 lines, then "… and N more lines".
  - m2: another library unticks the share box.
  - m3: Enter on the box presses Show here.
  - m5: Settings controls are keyed by `LibraryId`, and the mapping rows by their node.
  - m6: the tests assert the initial and Cancel focus, and wait for the service before negative checks.
  - m7: Remove's question says when open script tabs of the library have unsaved changes, whose recovery copies go with it.
  - m9: Cancel and Show here stay together on the right when the footer wraps.
  - m10: the launch-time and remove-then-break residuals of R-T10-7 go to T20; a persisted last good binding would close them.


## m4b-t17 — secret warning UI

- **Scan.** `secret_marks::scan_text` is `scan_secrets(text, &secret_spans(text))`, the spans
  mapped one to one (R-M4b-28). A script tab scans 300 ms after the last edit (and at once when
  its file is read or reloaded) on the core runtime's blocking pool, never on the UI thread. A
  newer edit replaces the waiting or running scan; an answer for an older version than the
  editor's is dropped (latest wins by document version). A save uses the tab's scan when it is
  for exactly the text being saved, else scans that text off the UI thread before it goes on.
- **Fail closed.** A scan that cannot answer (no core runtime, or it panicked) is one
  `Unscanned` finding over the whole text, as the scanner answers when its table fails to load.
  The gutter puts "Couldn't be checked for secrets" on line 1, and every save asks ("…
  couldn't be checked for secrets"). "Save anyway" is never remembered for it.
- **Gutter.** One `key-round` line mark per finding (ADR 0007 `## m4b-t17`), whose tooltip and
  accessible name name the kind ("Looks like a password", "… an access token", "… a private
  key", "… a connection URL with a password") and how to silence the line. `TestKey` and JWT read
  as an access token.
- **Dialog** (`secret_dialog`). Every save of a library script goes through `save::save`, so
  ⌘S, Save all, the unsaved prompt's Save (close, quit, restart), the recovery notice's Save and
  "Deleted on disk"'s Save all ask here first. A 480 alertdialog: title, the tab's environment ·
  connection · database, the body, a field well of **Line n · kind** rows (one per line and
  kind, 8 shown, then "… and N more lines"), the commit sentence, the allow hint and the
  "remembered" line. **Cancel** is Primary and focused; Escape, the close button and a window
  close answer Cancel and the tab stays unsaved; Enter presses the focused button. The focus
  returns where it was (the code). Nothing in it holds a value.
- **Commit sentence.** Shown unless the library's git summary is `NotARepo`: a repository,
  an untrusted or unavailable one, and a status not read yet all show it (errs toward the
  warning).
- **Remembered answers.** "Save anyway" remembers, in the tab's memory until quit (or the tab
  closes), `xxh3(kind ‖ the line's text before the literal ‖ after it ‖ the literal)` per finding.
  This joins the plan's rule (the line's text without the literal, and the kind) and `alg/28`'s
  (`kind ‖ value`): a changed value or a changed line asks again; a line moved up or down does
  not. Two identical lines are one answer.
- **Recovery (R-T5-4).** The recovery copy's `secret_suspected` is set from a scan of the
  copied text, run in the same blocking task that writes the copy (a panic counts as yes). The
  restore notice adds "Some lines of this text look like a password or a token. Saving asks
  first." when the copy's metadata says so; it never says which or shows a value.
- **Consoles** never scan, mark or ask (Q7).
- **Not in M4b.** There is no "Save as… into a library" and no "Move console to library…"; a
  later one must call `secret_dialog::confirm` (through `save::save`) the same way.
- **Copy.** The mock's hint copy follows the T6 fr2 FTL wording (a space after the dashes;
  `design/m4b-scripts` NOTES and index.html synced). New strings: `hv-scripts-secret-unscanned`,
  `-unscanned-hint`, `-unscanned-title`, `-unscanned-body`, `hv-scripts-recover-secret`.

## m4b-cifix-4 — a SQLite tab's file identity includes its birth time (Linux)

- **Finding (CI 45077623376, Linux only).**
  - `sqlite_tab_refuses_run_when_the_profile_points_to_another_file` timed out on `ubuntu-24.04` and in `turkey-test` waiting for "the replaced file is refused". macOS passed.
  - The test deletes the tab's database file and writes a new one at the same path. On journaled ext4 the new file in the same folder usually gets the freed inode back: the cifix-2 finding for folders, here for a file.
  - The tab's identity was the canonical path and `dev:ino` (R-T14-1). Both matched, so Run passed and the script would have run on the replaced file. **A product bug.**
- **Decision (R-CI2-1, applied to files).**
  - One public type, `fileio::FileIdentity { dev, ino, born }`, serves files and folders. `FolderIdentity` is now its alias, read from a folder. `born` is the birth time on Linux only (`statx` `btime` through `Metadata::created()`; userland cannot set it). On macOS it is `None` and `(dev, ino)` stays the identity: APFS never reuses an object id, and its birth time moves back with a set-back modification time.
  - `tabs::SqliteFile.id` and `recovery::DbFile.id` hold a `FileIdentity`. A replaced file is born later, so the gate's `stored == profile == session` comparison refuses it.
  - The text kept in `workspace.json` (`sqlite_id`) and in a recovery sidecar (`db_file_id`) is `dev:ino`, then `:secs.nanos` of the birth time when there is one. `FileIdentity::parse` reads it back exactly. A text written before this change has no birth time, so on Linux it no longer matches its file and Run refuses until the tab is opened again. This fails closed. M4b is unreleased.
  - A remount (R-T14-5) now also needs the same birth time: same path, same inode, same birth time, another device.
- **Residuals.**
  - **Same clock tick.** As for folders: a delete and a create inside one tick of Linux's coarse clock (1 to 4 ms), on an inode ext4 hands back, keep the same identity. Closing it needs the inode generation through an `ioctl` (`unsafe`).
  - **Windows** still compares the canonical path only (T14). NTFS's creation time can be set by any program and is "tunnelled" to a file made again under the same name within 15 s, so it would add nothing.
  - **Other file systems.** Where `statx` reports no birth time (some FUSE and network mounts), `born` is `None` on both sides and the check is `(dev, ino)`, as before.
- **The other `dev:ino` identities in M4b (audit).**
  - **Registry (T3/T10), `libraries.toml` `inode`.** Compared across sessions only. On ext4, a root deleted and made again between sessions can be adopted at the next launch. This is the cifix-2 carry (a T3 schema change). In-session identity already has the birth time.
  - **Library root (T10), `FolderIdentity`.** Already includes the birth time (cifix-2).
  - **Save stamps (T5).** `DiskStamp` is content-based: length, modification time and the XXH3-128 of the bytes. A replaced file with other bytes differs, and one with the same bytes is the same text. The read stream's private `Fingerprint` adds `dev`, `ino` and `ctime` with nanoseconds. A new file always gets a new `ctime`, which userland cannot set. Not affected.
  - **Folder handles (T3/T5), `dir::Identity`.** `Dir::verify` compares each folder from the root down, the opened folder included, against the identities read at open. The opened folder's handle is held for the whole operation, and an open handle keeps its inode allocated, so its number cannot go to another folder. A replaced ancestor therefore fails at the opened folder. Not affected.
  - **The partial-file cleanups in `manifest.rs` `create_in_place` and `fileio.rs` `publish_new`.** Each compares the name's identity with the file the same call just created and still has linked. These are not stored identities. Reuse needs that file unlinked and another created at the name in that instant, and then only that new file would be removed. Not affected.
  - **Rename target (T13), `fileops::target_of`.** Compares two `lstat`s taken in the same call. It is not a stored identity. Not affected.
  - **Git (T7), `map.rs` and `status.rs`.** Lookups within one listing. These are display only and gate nothing.
  - **Outside M4b.** `hacivat-introspect`'s SQLite `file_identity` (M4 metadata freshness) and the vault's and meta cache's link checks (both against an open handle) are not M4b's. They are not changed here.
- **Tests.**
  - `fileio::tests::file_identity_tells_a_file_written_again_from_a_renamed_one`, and `fileio::tests::file_identity_text_round_trips`.
  - `tests/recovery.rs` `recovery_round_trip_and_discard`: a birth time round-trips to the nanosecond.
  - `sqlite_tab_refuses_run_when_the_profile_points_to_another_file`: the CI failure, now GREEN on Linux.

## m4b-cifix-5 — Windows: a library root's identity is its volume serial and file id

- **Finding (CI 45091594060, the first `--no-fail-fast` run, Windows only).**
  - `service_t16` `use_this_folder_adopts_a_replaced_root_and_keeps_its_mappings` and `an_adopted_folder_never_inherits_the_old_binding`, and the app's `scripts_attach` `use_this_folder_is_confirmed_and_keeps_the_mappings`, timed out waiting for `RootChanged`. Each moves the root away and makes another folder at its path.
  - On Windows `FolderIdentity` was `None`, so `work::validate` compared the canonical path only and served the new folder as the same library, with its mappings and binding. **A product bug.** Root-replacement detection is required on Windows too: the library is the folder that was attached (I3), and an unsure answer must be `RootChanged`, never the old library.
  - `tests/fileio.rs` `open_stream_reports_a_change_on_disk`: another file of the same size renamed over a script mid-stream went unseen. The Windows `Fingerprint` is size, modification time and creation time; the rename keeps the times inside one clock tick, and NTFS tunnelling gives the new name the old creation time. **A product bug** (the cifix-4 review's m2).
- **Decision.**
  - `std` exposes a Windows file's volume serial and file index only behind the unstable `windows_by_handle`; the workspace forbids `unsafe` outside `hacivat-ui-core/src/platform/**`; no later task edits a manifest. `notify-debouncer-full`, already a dependency of `hacivat-scripts`, re-exports `file-id` (`file_id::get_file_id`: `GetFileInformationByHandleEx` `FileIdInfo`, else `GetFileInformationByHandle`), so the identity is read through it: no `unsafe` here, no new crate, no manifest edit. `file-id` is in `BACKENDS` for `hacivat-scripts` already, and no `file_id` type leaves the private module `fileio/windows_id.rs`.
  - `folder_identity` on Windows is `FileIdentity { dev: volume serial, ino: file id, born: None }`. The file id is NTFS's 64-bit file reference number kept exactly; a ReFS id that uses the high 64 bits is folded by XXH3-64. `get_file_id` follows a link, so the path must still be a real folder (`symlink_metadata`, a junction is never one) after the read.
  - On NTFS the file reference number carries a sequence number that changes when a record is reused, so a folder or file made again at the same path gets another id, and a rename keeps it. The same `validate`, `patch` and `ManifestJob` checks as on Unix now refuse a replaced root with `RootRefused(IdentityChanged)`, and "Use this folder" adopts it. `libraries.toml`'s `inode` holds the file id across sessions.
  - **No creation time in it (R-CI2-1).** Any program can set a Windows creation time (`SetFileTime`, `robocopy /DCOPY:T`), so a restore into the root would refuse an unchanged library, and tunnelling gives a folder made again under the same name within 15 s the old creation time, so it would not tell a replaced root apart either.
  - The read stream records the file's Windows identity at its path when it opens, and every chunk check compares it again (a link at the path reads as none, so it differs). A file renamed over the script is another file id.
- **Residuals.**
  - **FAT and exFAT** derive the file index from the folder entry's position. A renamed root then reads as replaced: it is refused and offered "Use this folder" (fails closed). An entry made again in the same slot keeps the index.
  - **Network shares** may report no stable id or none at all. With none at attach the library is compared by path for that session, as before; an id that becomes unreadable later refuses the root.
  - **A SQLite tab's database file** still compares the canonical path only on Windows (T14): `FileIdentity::of(&Metadata)` has nothing to read there, and the app's `sqlite_identity` is not changed in this fix. Carried: a path-based read like the folder's would close it.
  - **The read stream at open:** the identity is read by path just after the handle opens, so a swap inside that instant names the new file on both sides; the size and time checks stay as before.
- **Test fixes (no product change).**
  - `fileio::tests::file_identity_text_round_trips`: Windows holds a `SystemTime` in 100 ns steps, so `UNIX_EPOCH + 5 ns` is the epoch there. The text round-trips exactly for every time a platform holds; the fixed literal uses 500 ns, and the 5 ns digit is asserted where the platform holds it.
  - `fileio::tests::folder_identity_tells_a_folder_made_again_from_a_renamed_one`: its pure leg added 1 ns, which is no change on Windows; it adds 1 µs. Its real leg now runs on Windows too (rename keeps the identity, a folder made again does not); the birth-clock wait runs only where the birth time is part of the identity (on Windows a probe made again within 15 s keeps its first creation time, so the wait could never end).
  - `tests/fileio.rs` `names_under` built names with the native separator (`@production\purge.sql`). `RelPath` is `/`-separated on every OS, and no product path was involved: the helper now joins components with `/`.
  - `tests/scan.rs` `a_missing_or_stale_root_lists_nothing` built its stale root as `<canonical>/../a`. Joining onto a verbatim (`\\?\`) path drops the `..` in `std`, so on Windows it was the canonical path itself. The stale root now names another real folder on every OS; the `..` spelling stays as a Unix leg.
- **Tooling note.** `xtask check-deps`' `BackendInPublicApi` reads `pub const STREAM_CHUNK_BYTES: usize = 64 << 10;` as a header whose `<` never closes, so the header runs to the end of `fileio.rs` and any `file_id` path after it reads as public API. The Windows identity therefore lives in `fileio/windows_id.rs`. The checker is Task 1a's file and is not changed here (carried to the controller).
- **Review (APPROVE), m1 and m2.**
  - m1: a file id of 0, which WebDAV and some NAS or FUSE shares report for every file, is no identity (`windows_id::identity_from`): such a root is compared by its path, as where no id can be read, and two zeros never count as the same folder. Test: `fileio::tests::a_zero_file_id_is_no_identity`.
  - Residual: a share whose ids are unstable from one open to the next (a fresh id per handle) gives an unchanged root a new identity, so the library is refused with `RootChanged` and "Use this folder" is offered. This fails closed: the user adopts it again after each refusal.
  - m2 (the file id read once every N chunks instead of on every chunk): not done. A chunk read after another file took the path must never be handed out (HV-SCRIPT-12 step 4, `open_stream_reports_a_change_on_disk`), and a check every 16 chunks would hand out up to 15 of them. The extra open per 64 KiB chunk is kept; for a 150 MB script that is about 2,400 opens with zero access rights.

## m4b-t17 — fix round 1 (review I1, I2, m1–m4, m6; rulings R-T17-1, R-T17-2)

- **Gutter (R-T17-2).** A shown ▶ takes the key's slot and carries its label (ADR 0007 `### m4b-t17 fix round 1`). The key shows otherwise. No ▶, arrow stop or focus is lost to a mark.
- **Save inside the debounce (I2).** Two named tests guard it:
  - a save of text the tab has not scanned yet scans that text first, and asks;
  - a second secret on an answered line asks again.
- **One scan in flight (m2).**
  - An edit while a scan runs starts none. When that scan lands for an older version, its answer is dropped and the newer text is scanned after the debounce.
  - The text is copied out of the rope inside the blocking job.
  - The recovery copy reuses the tab's scan of the same version.
  - The save's own copy of the text stays on the UI thread (T14's). It is bounded by the size tiers: a T2 preview (above 20 MB) is never saved.
- **`Debug` (m1).** A scan's `Debug` shows its version and counts only, never a fingerprint.
- **Allow marker (m4).** `hacivat_scripts::secrets::ALLOW_MARKER` is the scanner's own spelling; the app's hints use it.
- **Tests (m6).**
  - A cancel asserts that no write started, by a count (`save::writes_started`).
  - The debounce and the one-in-flight rule are asserted on the tab's scan state.
  - The console test has a positive control.
- **Accepted (R-T17-1).** The Unscanned dialog has no rows, no allow hint and no "remembered" line.
- **Residual (m7, T14's pattern).** With no workspace window, a save's question has nowhere to open. Nothing is written, and a Save all chain or a quit round stops without a word.

## m4b-t18 — Run as script

`scripts::run_as_script` (HV-SCRIPT-12, R-M4b-20; NOTES surface 9). The runner and its exec changes are ADR 0006 `m4b-t18`.

- **Where it is offered.** A console whose editor is a T2 preview shows **Run as script** in the toolbar's Run slot instead of Run (NOTES ruling 8). There is no ⌘↵, because the preview editor is read-only and `editor.run` is disabled there. Two kinds of preview stream:
  - **A script tab's preview.** The file is read from disk. The app's `FileSource` adapts T10's `ScriptStream` and `PassRecord`: pass 2 reopens with pass 1's record, and only a pass read to its end is reopened. The tab's environment is read at each open from a watch channel that the app keeps current (R-T5-6). `chunk_error` and `read_error` are exhaustive `match`es with a test (T12 m6; T1a review). A path the library refuses, or one this connection no longer shows, maps to `Io(PermissionDenied)`, which tells the user "the file couldn't be read".
  - **A pasted preview (carries-w1 T11/T18).** "Paste as preview" (Task 11's 20–100 MB question, now on: `confirm_large_paste` for every console and script tab) opens a **new console tab on the source console's connection**. The tab shows the document's first 2.5 MB as a T2 preview, and the source console's text stays as it was.
    - *Ruling (T18): the whole pasted text lives in memory only.* It is held per console and read by `PastedSource`; both passes read the same `Arc`. It never reaches an autosave (the tab opens through `queue_text`, with autosave off), a recovery copy, history (the history row is "Pasted text") or any file, and it goes when the tab closes.
    - *Why:* a paste can hold anything, credentials included (L14). A file would need a place, a lifetime and a cleanup.
    - *Cost:* up to 100 MB of RAM per pasted preview while its tab is open, and it does not survive a restart. Fix round 1 (m5): the tab comes back as an ordinary console with the same number, and its restore reads that number's console file (`<n>.sql`), which may hold an older console's text, never the pasted text.
    - A console with no connection refuses the paste with a toast.
- **The run.** The run is `ScriptRunner::run_checked` on the core runtime, built from what the orchestrator's named hook (`script_run_parts`, beside `start_run_capped`) gathers. The hook first makes Run's own checks: no read-only reconnect pending, nothing else running, a script tab that runs somewhere, a live session (else the connect starts and nothing runs, R-M3-45), and the SQL context read (`modes_known` comes from it). Then the run uses:
  - the tab's own session, which is fixed at open (R-M4b-4);
  - the console's suppress set, history sink (origin `File`) and log;
  - the catalog refresh inbox;
  - the live environment tag, through a watch channel. `observe_global::<ConnectionsModel>` keeps it current (T12 fr1 I2). A test retags mid-run and the answered statement is not sent.
  
  At the end, the "don't ask again" keys join the console's, the SQL context is read again, and the explorer's refresh follows as after a Run (`script_run_ended`).
- **The database check (R-M4b-27, R-T14-2, R-T14-3).** Between the passes the runner asks the UI thread to run Task 14's per-Run check (`tabs::run_gate_streamed`). It is the same gate as Run's: the probe on the tab's own session, the MySQL switch back once, and the refusal notice naming both databases. The only difference is that the gate's preview refusal is skipped (`streamed`). A pass answers the context generation that the check read (`tabs::take_checked_generation`). A refusal drops the gate's re-run closure, and with it the reply, which answers `None` (fail closed). A pasted preview's console has no bound database, so it has no check, as a console's Run has none.
- **Confirmations.** They are M3b's dialogs, through `AppConfirmPort::for_script_run`. Its dialogs offer Execute all (off production) and Skip all in **open-ended words**, because the later statements are not read yet: "Execute all also runs, without asking, every later statement in the file that would ask on this connection (dev); a statement that needs its name typed still asks." Here `ConfirmContext::open_ended` replaces the count. A user's No (or Cancel) stops the run, and the units confirmed before it stay applied (R-T12-2). The strip says so: "Stopped at statement N: you didn't confirm it. The statements before it ran and stay applied; nothing after it was sent."
- **The strip (surface 9).** It is a 26 px raised row under the notices. While the run goes, it shows:
  - the amber run marker;
  - "Checking the whole file before it runs" in pass 1, or "Running as script" in pass 2;
  - the statements sent, the line, the bytes read of the file's size, and a clock repainted once a second;
  - **Stop**, a Default button and a Tab stop.

  Stop is the M3 cancel path: it stops between units and cancels the unit in flight. `exec.cancel` routes to it. Escape never stops a run.
  
  At the end the strip shows how the run ended. That is either the mock's sentence (done, a failed statement, Stop, a No) or the runner's own refusal or stop sentence, followed by **Close**. The runner writes its log lines and summary to Output. While a run goes, the toolbar's Run as script is disabled, with the tooltip "Running as script: wait for it, or stop it", and the session's per-statement run clock is not shown in the toolbar. The strip sits in a focus home whose primary is the editor (R-M4b-43), so a Stop or Close pressed from the keyboard hands the focus to the code (tested). A strip that is dropped (its tab closed) cancels its run, because a dropped sender never cancels.
- **Trust signals.** The strip is drawn under the toolbar. It never covers the capsule, the environment mark, the production hairline or the status bar's "Read-only preview" segment (Task 14's, reused). A test checks each of them during a run.
- **Residuals.**
  - ~~A confirm-in-transaction prompt raised by a streamed unit has no `follow` entry.~~ Fixed in fix round 1 (I2), below.
  - The elapsed clock follows the executor clock, like the toolbar's.

### m4b-t18 fix round 1 (review I1–I3, m1–m8; rulings R-T18-1, R-T18-2)

- **I1: plain Run never runs on a preview, or beside a stream.**
  - Every plain Run path refuses on an editor that is a T2 preview, with "A read-only preview doesn't run." (`orchestrator::refuse_plain_run`). The paths are:
    - the gutter ▶ (`run_would_be_enabled` is false, and its `ExecuteRequested` reaches `on_execute`);
    - `on_execute` (the keys);
    - the single submit point `start_run_capped` (the chooser's confirm, Re-run, T14's Retry).
  - Plain Run also refuses on a console whose session a streamed run holds ("A script is running on this connection's session…"). This covers the stream's own console and any console sharing that session (`run_as_script::blocks_run`; `run_busy` includes it).
  - A stream refuses to start while a console sharing its session runs or prepares a run. Two runs therefore never interleave on one session (review rv_b, now a test).
- **I2: a streamed unit's confirm-in-transaction prompt.**
  - The runner hands the port each unit's run through the new defaulted `ConfirmPort::follow_run`: its ticket's `choices` and `status`, the unit's masked text (the history redaction), and the session.
  - `AppConfirmPort` registers the run in its book, as M3's submit does: the prompt shows the statement and takes its lost form from the run's status.
  - It also listens for the session's loss on the core runtime with M3's `LossLink` rule, so the prompt turns to its "connection lost" form (Close only) exactly as M3's single run's does.
  - Tests: the statement is shown through a real streamed run; the lost form is shown through the port. Both are mutation-killed.
- **I3, R-T18-1: a paste from a script tab keeps that tab's binding.**
  - The pasted preview records the source script tab's doc. Its run passes T14's per-Run check through that doc's binding, on the pasted console's session (`tabs::run_gate_streamed_for`), with the MySQL switch back once and the refusal notice.
  - From a MySQL/MariaDB tab (`SessionPlan::Own`, R-T14-3), the pasted console takes a session of its own (slot 0) and refuses on the profile's default session.
  - A paste from a plain console keeps the console's own binding, as M3 does.
  - Tests: a tab bound to `orders_archive` either runs there after one `USE`, or refuses; it never runs on `orders`.
  - A source tab that has closed fails the check closed.
- **m1:** the check is made only on the session the run was started on: the console must still hold it, and `take_checked_generation` compares that session's id.
- **m2:** when the profile is deleted mid-run, the run stops instead of publishing "no environment".
- **m3:** T14's Retry on a preview retries Run as script.
- **m4, R-T18-2:** when Stop cancels the statement in flight, the strip says "Stopped during statement N; statements after it were not sent." It uses the runner's new `ScriptRunSummary::cancelled_in_flight`. The PO is informed.
- **m5:** the relaunch sentence above is corrected.
- **m6:** a pasted text held for a tab is dropped when the tab's host is built without adopting it, and whenever a console closes or another paste opens (`prune_pending`: every tab no workspace shows any more).
- **m7:** shots:
  - the running strip is now on a connected production console (capsule, swatch, hairline);
  - the confirmation is drawn over a running strip.
- **m8:** app tests cover:
  - a change between the passes (rv_a);
  - a MySQL switch back with a pasted streamed run;
  - a paste from a script tab.

## m4b-t20 — the adversarial suite (Review Focus 1–6) and the carries routed to it

- **The suite.**
  - `hacivat-scripts/tests/hostile_library.rs` (15 rows): a 64 KiB manifest of nested tables or arrays, 10,000 aliases, a NUL inside `database`, a BOM-prefixed manifest, a manifest that is a FIFO (through the service: never blocks), a script that is a FIFO or a link to a device, a 300-deep folder chain, 200,000 empty files (a names-only fake `Fs`), every bidi control in a name and a bidi-spoofed `@production`, a file replaced by a link between the read and the save, a hard link to a file outside, and the carry rows below.
  - `hacivat-git/tests/hostile_repo_more.rs` (6 rows at first, 12 after the fix rounds): a gitfile that leads outside the library and the home folder to a hostile repository, configuration includes outside the repository (every `includeIf` form), a 1 GiB index, unknown index extensions, the outer `index.lock` path, and a fan-out of alternates.
  - `hacivat-app/tests/scripts_adversarial.rs` (4 rows, `FakeDriver` per connection): Run pressed while the chooser is open runs nothing; a retag while the tab is open shows the new environment and runs where bound; a file moved into `@production/` while open under staging runs nothing and is offered only under production, never preselected; a stranded rename temp is named in the diagnostics.
  - The fuzz targets' oracles (`fuzz/scripts`): a bound manifest name passes the character rules and the limits, no key outside the schema binds; an accepted path component is one `RelPath` component, and `join_with` over an in-memory tree of folders, files and links (any text) never answers a path outside the root. 10 minutes each, 0 crashes (7.4 M and 10.5 M runs, macOS, nightly-2026-11-01).
- **Decisions in this task.**
  - **Alternates (Task 20's own probe, from the T7 review's q2).** The chain was bounded in depth only; an alternates file of 500 lines per level fanned out to 500³ folder checks (each up to 20,000 `lstat`s), so the status answered `Timeout` and its worker kept running, leaving the library `Paused` until a restart. Each alternate folder is now checked once (by canonical path), and more than `ALTERNATES_MAX` (8) distinct ones answer `Unavailable(Redirected)`. **Fix round 1 (review I1):** that left the per-line resolution unbounded (each line was `canonicalize`d, up to `MAXSYMLINKS` hops of any length, before it was counted; 1,000 short lines through a 30-link chain of 150-deep paths took about 8 s, past the budget, and every refresh paid it again). Now each file's lines are de-duplicated by their raw text and the chain's distinct lines are counted **before any is resolved**: more than `ALTERNATE_LINES_MAX` (32) answer `Redirected` at once; the worker's interrupt flag is read before each resolution and on every folder entry, so a timed-out worker stops; and an object folder's `pack` entries are capped too (`PACK_ENTRIES_MAX`, 20,000, answered `TooLarge(LooseObjects)`: `git gc` repacks). The cost of one status's alternates and placeholder checks is at most 32 resolutions (each ≤ `MAXSYMLINKS` hops) plus (1 + 8) object folders × (≤ 20,000 loose + ≤ 20,000 pack entries) `lstat`s, about 360,000 in the worst case. That is tens of milliseconds to a few hundred on a local disk, and more on a slow share, where the interrupt flag still ends a worker within one call of its caller's `Timeout`. The q2 sentence of the first round ("9 × 20,000 `lstat`s, tens of milliseconds") was wrong: it left out both terms. **Fix round 2 (re-review I1-fr1):**
    - **The cost moved into gix.** gix's store open re-reads the raw alternates file and resolves every entry, duplicates included (`gix-odb` `alternate::parse` does not de-duplicate), and the call cannot be interrupted. One short line repeated to 64 KiB, each entry a 1–4 hop link chain through 450-deep folders, took 27–113 s in the reviewer's runs, so the status answered `Timeout`, then `Paused` on every refresh.
    - **The fix:** the raw entries are capped before anything opens the store, counted as `gix-odb` parses them (every non-empty line not starting with `#`, untrimmed): more than `ALTERNATE_RAW_PER_FILE` (16) in one file, or `ALTERNATE_RAW_PER_CHAIN` (32) in the chain, gives `Redirected`. gix now resolves at most 32 entries.
    - **Defence in depth (`status::run_timed_backoff`, `GitLimits::retry_after`):** a worker abandoned at its deadline records an overrun when it finally ends. Until a backoff passes (30 s, doubling on each overrun in a row, at most an hour) the library answers `Unavailable(Timeout)` at once and starts no worker; then it is retried. A worker that answers in time clears the backoff. With the one-worker claim, a library has at most one stuck worker, never one back to back, and is never `Paused` for good.
    - **Tests:** `a_repeated_short_alternates_line_never_reaches_gix` (the reviewer's shape: a 2-hop chain through 400-deep folders, repeated to 64 KiB with whitespace variants, default budget, two passes; RED with the cap off: `Timeout`) and `an_overrun_backs_off_then_retries` (RED with the overrun record off).
  - **R-T20-2 (controller ruling, fix round 3; re-review I1-fr2): a structural alternates rule, no more mirroring of gix's parser.**
    - **Reason (H3, H4).** The fr2 cap was counted over the files our walk resolved, and gix resolves others.
      - H3: the root file holds one entry, `l0 ` (trailing space). Git and our walk trim it and resolve `l0`, a harmless alternate. gix resolves `objects/l0 `, a second link, to a folder whose alternates file is 64 KiB of `k0`.
      - H4: the root names alternate `a` (absolute), and `a`'s file says `k0`. Git and our walk join that to `a`'s objects folder, an empty one. gix joins it to the **root** objects folder, where `k0` leads to a 64 KiB file of expensive entries.
      - Each answered `Timeout` at 5 s, then `Paused`, with every entry count at 1 or 2.
    - **The rule.** Git status reads a repository with alternates only if:
      - the root `objects/info/alternates` holds at most 16 entries (`ALTERNATE_ENTRIES_MAX`), counted as gix parses them;
      - each entry is an absolute path, byte-identical to its trimmed form (no leading or trailing whitespace, no CR), and not C-quoted;
      - each names an existing object folder the user owns;
      - none of those folders has an `info/alternates` of its own, of any kind or size (no chain at all, so no nested file is read, by us or by gix).
    - Anything else answers `Unavailable(Redirected)` before gix opens anything: a relative entry, a nested alternates file, whitespace, an over-cap file, an unreadable target. It fails closed, because badges are cosmetic. `git clone --shared` and `--reference` write one absolute entry and still read.
    - The chain walk is deleted (`AltWalk`, `ALTERNATES_DEPTH_MAX`, `ALTERNATES_MAX`, `ALTERNATE_LINES_MAX`, `ALTERNATE_RAW_PER_FILE`, `ALTERNATE_RAW_PER_CHAIN`). gix now resolves at most 16 entries, none of which leads to another alternates file. The cost bound above becomes at most 16 resolutions plus (1 + 16) object folders' placeholder checks.
    - **Tests** (`hostile_repo_more.rs`):
      - `h3_an_entry_with_trailing_whitespace_is_refused` and `h4_a_nested_alternates_file_is_refused`: the reviewer's shapes, default budget, two passes, `Redirected`. RED on the fr2 code: `Timeout`.
      - `only_single_level_absolute_alternates_are_read`: one absolute alternate gives `Ready`. A nested file (empty, one line, or 1,000 lines), a relative entry and CRLF each give `Redirected`.
    - The earlier rows were kept under the rule. The fan-out's `clone --shared` case removes the alternate's own file. The repeated-line test reads 16 copies of an absolute entry.
    - m-fr2-1: `an_overrun_backs_off_then_retries` uses a 3 s backoff, so a loaded runner still lands inside it.
  - **Repository paths never leave the local drives (fix round 4, re-review I-fr3-1; Review Focus 3).**
    - **The problem.** On Windows, a `\\host\share\…` or `\\?\UNC\…` path counts as absolute. It could arrive in an alternates entry, a gitfile's `gitdir:` or a `commondir`. `canonicalize` then opened it over SMB, which sends the user's NTLM credentials, on attach and with no user action.
    - **One rule.** `map::path_prefix` classifies a path text by how it starts, read as Windows reads it. It is a pure function and reads the same on every OS. `PathPrefix::is_local` admits:
      - no prefix (a relative path, or one rooted on the current drive);
      - a drive path (`C:\`);
      - the verbatim drive path a canonical library root itself uses (`\\?\C:\`).
    - It refuses UNC (`\\host`, `//host`, mixed separators), verbatim UNC, the device and object namespaces (`\\.\`, `//./`, `\??\`, and `\\?\` followed by anything but a drive), and drive-relative `C:x`.
    - R-CI2-2's index-path guard uses the same classifier: on Windows an index path has no prefix at all. There is no second copy of the rule.
    - **The guard** (`trust::local_spelling`, which checks the raw and the trimmed text) runs before any filesystem call on every path text the repository hands us:
      - inside `trust::resolve`: the gitfile's `gitdir:`, `commondir`, alternates entries and a nested `.git`'s target;
      - a linked worktree's `gitdir` back-pointer, which gix's discovery may read;
      - every value in the repository's own `config` and `config.worktree` (`trust::config_names_a_remote_path`, run before the open). That covers `core.worktree`, `core.excludesFile`, `core.attributesFile` and any key a later gix reads. The only sections left out are `remote`, `submodule` and `url`: their values are fetch locations a status never opens, and a clone from `\\server\git\repo.git` is ordinary.
      - A refusal answers `Unavailable(Redirected)`.
    - The guard runs on every OS. It fails closed, since no real repository on Unix names `//host`.
    - A symbolic ref's name may no longer hold `:` (git's ref names never do), so no join follows a drive.
    - **Residual:** a mapped or `subst` drive letter (`Z:\`) passes. The user set that drive up and chose its host; the repository cannot choose it.
    - **Tests:**
      - `map::tests::path_prefixes_are_classified_as_windows_reads_them`, the pure classifier, runs on every OS;
      - `trust::tests::local_spelling_refuses_every_network_and_device_start`;
      - `hostile_repo_more.rs` `no_repository_path_leaves_the_local_drives`: alternates, gitfile, `commondir`, `gitdir` back-pointer, `core.worktree` and `core.excludesFile`, each crossed with six starts that name a nonexistent `.invalid` host. Each answers `Redirected`. On Windows each start must also be `is_absolute` and refused within 2 s. Restored, both repositories read, and a remote URL on a share reads too;
      - `the_guard_alone_refuses_a_working_double_slash_spelling` (Unix): `//<a real local folder>` is a working POSIX spelling, so only the guard refuses it. Each source gives `Redirected`, and `/…` gives `Ready`.
      - Both repository tests go RED with the guard answering `true`.
  - **Fix round 5 (re-review fr4).**
    - **I-fr4-1: nested repositories.** gix's untracked walk runs `gix_discover::is_git` on every nested folder that holds a `.git`.
      - For a gitfile it reads `gitdir:`, then checks `<target>/HEAD` and the target's `commondir`.
      - For a `.git` folder it reads `commondir` and `gitdir` and checks the paths they name.
      - Our pre-walk skipped a guarded gitfile target, and never read a nested folder's `commondir`.
      - Now `trust::check_nested` applies `local_spelling` to all three, before the gix status runs: the gitfile's `gitdir:` (the parsed target and the whole text), the target's `commondir`, and the folder's `commondir` and `gitdir` (`nested_pointer`). Any refusal answers `Unavailable(Redirected)` for the **whole library**, never a skip. A pointer that is not a small regular file (a FIFO gix would block on, say) is refused the same way, and a placeholder answers `Dataless`.
      - The reviewer's proof is now an assertion. A nested `sub/.git` reading `gitdir: //<real git dir>` showed `sub: Unknown(Submodule)` in the badges (still RED with the check off). It now answers `Redirected`, and the `/…` control shows `sub`.
    - **m-fr4-2: links.** `trust::resolve` walks the joined path component by component before `canonicalize`, using `symlink_metadata` and `read_link`. Every link target must be a `local_spelling`, so on Unix a `//` target is refused too.
      - More than 40 links (`LINK_HOPS_MAX`), or an unreadable link, refuses.
      - A missing component ends the walk, since `canonicalize` then fails and nothing is followed.
      - Unit test: `a_link_to_a_network_spelling_is_never_followed` checks a link at the start, in the middle, after a relative hop, after `..`, and a loop. A local chain still resolves.
    - **m-fr4-3: rooted with no drive.** `PathPrefix::Rooted` (`\x`, `/x`) is local on Unix only. On Windows it resolves against the process's current drive, which may be a share. The classifier table is updated.
    - **m-fr4-1: never-path config keys.** `user.*` and `core.comment*` are skipped, along with the `remote`, `submodule` and `url` sections.
      - **Known false-positive class (accepted, fail closed):** any other value that only *starts* like a network or device path refuses the whole library, even when gix never reads it as a path. Examples are an `alias.*` starting `//`, or a custom key holding a share path. The badges are cosmetic, and a key-by-key allow-list would miss a path key added in a later gix.
  - **Pre-merge fix (re-review fr5).**
    - **m-fr5-1.** `trust::resolve` now returns `Result<Option<PathBuf>, GitState>`:
      - `Ok(None)`: the path names nothing.
      - `Err(_)`: the path is refused, because its spelling or a link target leaves the local drives (or the link budget ran out), or the interrupt flag is set.
      - Every caller answers a refusal as the status. `check_nested` used to take a refusal for "missing" and skip it, while gix followed the link. A nested gitfile naming a local link to `//real` now answers `Redirected`, not `Ready` with `sub`.
    - **m-fr5-2.** `check_library_tree` takes the worker's interrupt flag and reads it per entry and in `check_nested`.
      - The layout checks and the library pre-walk each share one `LinkBudget` of 4,096 hops (`SCAN_HOPS_MAX`), on top of the 40-hop limit per path. Over budget answers `Redirected`.
      - Test: 2,000 nested gitfiles behind 30-hop chains answer `Redirected` in about 0.1 s, on both passes. With the budget off, the same library took 1.3 s and read `Ready`.
  - **Residual (m-fr3-1): a live local writer.** R-T20-2 checks that a vetted alternate has no `info/alternates` before gix opens the store, and gix reads the folder again during the open. A local process that creates and removes that file in the gap can make gix follow one nested file, which can cost one `Timeout`. This is bounded:
    - by the 5 s budget;
    - by one worker per library;
    - by the overrun backoff (30 s, doubling).
    - It needs a process already running as the user, which is outside the hostile-library model: hostile files, not a hostile live process. It is not fixed.
  - **N3 (cifix-2 re-review): a library inside a bigger repository.** `GitStatus::index_lock` carries the validated outer git dir's `index.lock` (none when the git dir is the library's own `.git` folder). The service hands it to the watcher (`Watcher::set_outer_lock`), and the reconciler `lstat`s it at most once a window while counted hints arrive, treating it held exactly like the local lock. Nothing outside the library is watched; the path is never derived from the index or the worktree. **Fix round 1 (review m3):** every status re-arms it, a status that is not `Ready` drops it (a path no status validates now is not looked at), and a watcher started after the status (a re-attach, an adoption, a watch-mode switch) gets the latest status's path when it starts. Test: `hostile_library.rs` `git_the_outer_index_lock_reaches_every_watcher_and_leaves_with_the_status`, end to end through the service. It goes red when a non-`Ready` status keeps the path. The restart leg cannot isolate the start hand-off, because the restart's rescan also refreshes the status. **Fix round 2 (re-review m3-fr1):**
    - Only a state that means the repository is no longer the validated one drops the lock: `NotARepo`, `Untrusted`, `Redirected`, `Unsupported`, `TrustPromoted` or `Failed`, or the library itself made unavailable (gone or replaced).
    - A passing `Timeout`, `Paused`, `TooLarge` or `Dataless` keeps it. Its layout was validated before, and dropping the lock would end a checkout's burst early in the very monorepos N3 is for.
    - The service keeps the last validated path per library and hands it to every watcher it starts.
    - Unit test `actor::t20_tests::only_a_changed_repository_drops_the_outer_lock`.
    - The service test's flake (1 in 10 runs, more under load) was the test double's: `FakeEventSource` kept one sink per path, so a dropped watcher's late `unwatch` removed its successor's sink. Each fake source now removes only its own sink, as the OS backends do. 48 of 48 runs passed six at a time.
  - **Token rules (check-deps, a named exception in Task 1a's file).** `GitTestingHookOutsideTests` also flags, in `hacivat-git/src`, any public `use` whose path goes through `testing` and any public glob of a crate-local root (T7 q1). `TrashDefaultContext` refuses `trash::DEFAULT_TRASH_CTX` in every file (T13 fr1 n1: `clippy.toml` cannot ban a constant). `ScriptContentOutsideFileio` replaces T10's text tripwire over `service.rs`, `service/**`, `index.rs` and `events.rs`: paths into `std::fs`, `std::os::<os>::fs` or `tokio::fs` through any rename, a glob from outside the crate, and the open/read/list/remove calls by name. T10's tripwire test stays as a second net. **Fix round 1:**
    - m1: the q1 rule also flags public aliases of a crate-local root (`pub use crate as hv`, `pub use super::super as root`, `pub extern crate self as hv`) and public `type`/`const`/`static` items whose tokens go through `testing`. Consumers are flagged for `testing` anywhere after `hacivat_git` once renames are resolved (`hacivat_git::hv::testing`).
    - m2: the service rule also refuses `hacivat_settings::fsio` (a second, path-based writer) and any `include!` or `#[path]` in those files.
    - Inherited false positive: a call to the injected `Fs`'s `read_dir` in a service file is flagged by name, as T10's tripwire did.
    - Fix round 2 (re-review nit): `#[cfg_attr(…, path = "…")]` is refused too. `include_str!` is not: it is compile-time only, and no file is read at run time.
  - **T9 m9.** The watcher's hash read walks no-follow folder handles from the stored canonical root (T5's `Dir`), never a path-based open after `join`: a folder swapped for a link in between is refused.
  - **T13 fr1 re-review.** n4: a failed unlink of the old name after the link is rolled back (the new name is unlinked), so "Nothing was changed" is true; when the roll-back fails too, or the folder check around the unlink fails, the answer is the new `FileOpError::RenameIncomplete` ("both names may show the same file"), never "nothing was changed". Fix round 1 (review m4): the roll-back unlinks the new name only when it is still the link this rename made (the same known device and inode as the old name). Another file that landed there is never removed, and where no identity is known (Windows) nothing is rolled back (`RenameIncomplete`). The toast says "The rename may be half done: both names may show the same file." n5: the scan reports a stranded `.hv-rename-<32 hex>` as `scan::Diag::InterruptedRename` where its folder is shown; it is never listed, never deleted, and its folder is not moved or trashed while it is there. n7: a refused name at the library root no longer refuses every folder operation. Windows: a file link is never renamed through `CreateHardLinkW` (which would link its target): `NoSafeRename`.
  - **T10 fr3 m-j.** The service's pairing reads first the candidates that kept the held file's name, then those in its folder (a stable sort); the read cap is unchanged.
  - **T16 re-review m4.** A refusal for a moved or replaced root keeps the identity of the folder found at the path (`LibrarySnapshot::refused_identity`); the confirm passes it to `adopt_root`, which adopts only that folder (`Ok(false)` for another one swapped in meanwhile).
  - **cifix-3.** The poll pass takes its stamps with the poll lock released (a hung share no longer blocks `watch`, `unwatch` or `pace`); `manifest::content_hash` re-checks dataless on its own `lstat` (m2; no test: a dataless file cannot be made without a real cloud provider, the change only refuses more); the Linux link-watch test matches by inode alone (m1).
- **Recorded residuals.**
  - **A hard link to a file outside the library** is the same inode under a name inside it; no `lstat` tells it apart. It is listed, read and saved in place like any file the user put there, and the save changes the bytes under the outside name too (`a_hard_link_to_a_file_outside_is_the_users_own_inode` pins this). Only someone who can already write the library can make one.
  - **The plan's `Untrusted(GitdirOutside)`** was never built: Task 7 ruled a gitfile that is neither a linked worktree nor a submodule `Unavailable(Redirected)` (carries-w1 T7 n2), and the row asserts that ruling.
  - **A macro-built path to `hacivat_git::testing`** (`macro_rules! m { ($m:ident) => { hacivat_git::$m::… } }`) is not seen by the token rule, as the run-time program name of `NoGitSubprocess` is not; nor is a public wrapper function in `hacivat-git` that calls a hook (fix round 1, m1). Each needs a deliberate, reviewed change, and the hooks can only add refusals or delays.
  - **Whether a configuration include was followed (fix round 1, m5; corrected in fix round 2, m5-fr1).** Under this pipeline's isolated open, the status honours none of `core.autocrlf`, `core.excludesFile` or `status.showUntrackedFiles`, even from the repository's own config (observed). The round-1 sentence "no badge can show it" was too strong. `core.bare = true` in the repository's own config gives `Unavailable(Redirected)`, so `includes_outside_the_repository_run_nothing` sets it in the outside config, asserts `Ready` with the whole badge map pinned, and, as its control, asserts `Redirected` with the same key in the repository's config.
  - **T12 n2: pass 1's context across chunks.** Pass 1 classifies a chunk after any context-changing unit in an unknown context (`PassGate.changed`), where M3's `classify_units` carries a literal search-path switch. The difference can only make pass 1 refuse or ask more, never less, and pass 2 gates every unit in its live context before it is sent. An exact carry needs `sql-safety`'s private `literal_switch` and `mark_stale` exposed (M3's crate, outside this task). Deferred to the classifier work of M5.
  - **T12 n4: MariaDB with unknown modes.** Pass 1 runs eight `Eager` splitters; T8 computed about 287 MB worst case each on adversarial shapes, so about 2.3 GB before a refusal. Not measured here: a run of that size would take most of this machine's free memory and disk. The 64 MiB held-text refusal bounds each splitter's text, not its item tables. Carried, **owner M5** (the language-intelligence pass on the splitter; fix round 1, m7): an up-front refusal when the modes are unknown and a variant holds past a small threshold.
  - **T18 n1: the transaction chip during a streamed run.** Its Commit and Roll back stay enabled while a streamed run holds the session. Both are explicit acts of the user on a transaction the script opened; neither runs any statement of the file, and the next unit sees the session's real state. A result grid's row count (a read, usually on the pool) is the other request that can reach the session between units.
  - **T18 n3.** The loss listener starts for every streamed unit (one task per unit, ended with the unit). Starting it only for units that can raise a commit prompt needs the confirm-in-transaction setting in the port, and a listener missing when a prompt does appear would leave it without its lost form. Kept.
  - **cifix-4 m1: a legacy Linux SQLite tab id.** A tab stored before cifix-4 has no birth time, so after the upgrade its Run refuses as `Edited`, naming the same file twice. A one-time effect for tabs saved before cifix-4 (M4b is unreleased); opening the tab again binds it with the full identity. Never upgraded automatically (that would reopen the ext4 reuse hole).
  - **cifix-4 m2: the Windows stream fingerprint and NTFS tunnelling.** Settled by cifix-5: the read stream's Windows check is the volume serial and file id, read at open and on every chunk; the creation time is not part of any identity. The T5 save-side residual (m11, above) stands.
- **Already done by earlier tasks (verified, no change).** H6b (a folder above the root replaced by a link between sessions): T10 fr2 m-a, `h6b_parent_replaced_by_a_link_between_sessions_is_refused`. m-k (R-T10-7): T16 fr1, `an_unreadable_manifest_keeps_the_last_good_binding` and `the_kept_binding_is_said_alike_in_the_popover_and_settings`.

## m4b-t22-pre — notices under a Run-as-script strip, tab-strip widths, shot locks

The T21 re-review residuals r1–r4 and the m12 flake, before the gate.

- **r1: a notice never vanishes under a strip.** While a strip shows, a script tab's notices sit in `disk_bars::NoticeArea`: a scroll area that gives up its height first, down to a floor that keeps the first notice's title whole. The floor is measured each frame from the title's bounds (`InlineNotice::on_title_bounds`). Before the first measure it is one line of `body_sm` plus the paddings, and a changed measure redraws only the console host. Under a strip each notice is condensed (`InlineNotice::actions_first`): its buttons sit right under its title and its other lines come after them, so the buttons show wherever the room allows. The area follows the focus (`hacivat_ui::follow::FollowFocus`, as the form dialog's body does), so a button Tab lands on scrolls into view. Without a strip, nothing changes.
- **The checks see it.** `RenderedText::folded` marks text cut or hidden at a scroll viewport's fold, which check (b) lets through as scrolling. Labels and paragraphs set it. An inline notice's title lines, given an id (`InlineNotice::title_id`, `#script-notice-title`), complete their build-time record at prepaint with what their clip shows. The shots' trust check counts a folded trust label as not whole, and the new check (s) requires a script tab's first notice title to be visible, unclipped and not folded.
- **r2.** A strip whose width changed notifies the view that drew it (`Window::current_view`), instead of `refresh_windows` redrawing every window without its view caches. Widths are dropped when their window closes (`on_window_closed`) and, for closed panes, when the workspace draws (`panes::keep_strip_widths`).
- **r3.** The tab layout and the strip's plan read the same constants: padding, gap, border, the dirty dot, the icon size, and the environment mark's `capsule::MARK_PX`. The plan's swatch had been 8 px against a 7 px mark. `panes::planned_tab_width` lets a test hold the plan against the laid-out tab.
- **r4.** A shot root's lock names its owner's pid. A lock whose owner is gone is taken over at once; the 10-minute stale rule stays for a lock with no readable owner. The guard owns the lock as soon as it is taken.
- **m12 (`introspect_live_choice_persists`).** Not reproduced. No test durability seam exists, and no wait was raised. Instead, `profile_data::timings` keeps every save's lock wait and its time under the lock, the read and the two syncs included. The test times each of its steps and prints them as `HV-FACT m4b-t22-pre …` lines, with the profile's data state and those timings: every step's time on a failure, one summary line on a pass. The next CI failure names the step and the time spent.

### Fix round 1 (T22-pre review I1–I3, m1–m3, m5; ruling R-T22-1)

- **Priority inside the console tile during a Run as script (R-T22-1), highest first:** the toolbar and the trust signals; the strip with Stop; the title and button row of the most severe notice; the code's two lines; the other notices. A lower item gives way first.
- **I1.** The toolbar's run button is Stop for the whole Run as script, the first pass and the gaps between units included (`RunUx::Running` while `run_as_script::running`; Stop is `exec.cancel`, which stops the streamed run). The notices' floor gives way to the strip: a probe on the strip's row measures how far it ran past the tile's bottom and caps the floor by that much, and the cap lifts as room comes back (`NoticeArea::strip_probe`, `tile_probe`).
- **I3.** Under a strip the notices are ordered by severity: the connection deleted, the file deleted, the file changed, the rest. The most severe sits in an area of its own (`script-notice-head`); the others share a second area with a far larger shrink factor and no floor. A button of theirs that holds the focus keeps room to show it (`FollowFocus::holds_focus`). Each title's id names its severity (`disk_bars::NOTICE_TITLE_IDS`), and check (s) requires the most severe one whole. The shot state `m4b-script-run-notices` ("Recovered…" then "Deleted on disk" under a strip) fails (s) in arrival order.
- **I2.** The head's floor is its title and its button row, plus the focus-follow margin, measured each frame (`InlineNotice::on_actions_bounds`). Tab onto its buttons never scrolls the title away. Both areas scroll back to their top when the focus leaves them (`FollowFocus::home_on_leave`).
- **m1.** The head is condensed only while it does not fit whole. Condensed, its explanation stays before its buttons in the element tree: they are drawn in a reversed column.
- **m3.** A dead or stale shot lock is taken over by renaming it to a tomb of the taker's own; the loser retries. The tomb is checked to be the lock that was judged, and a newer lock is put back.
- **m5.** `profile_data::timings` records in debug builds only; a release build reads zeros.

## m4b-gatefix-1 — an idle watcher wakes nothing; Go to script cost; stub names gone

- **Watcher idle CPU (gate E12).** The gate measured 130–240 ms of CPU per minute with three idle libraries, against 10–20 ms with none. Each library's `notify-debouncer-full` 0.7 loop slept a fixed tick (150 ms / 4) and woke about 27 times a second, forever. `OsEventSource` no longer uses that debouncer. It builds the platform's recommended watcher itself, at one reviewed site and only with `watcher_config()` (`follow_symlinks(false)`, R-T0-2), and hands every event to a debounce thread of its own (`hv-watch-debounce`).
  - That thread blocks on its channel while nothing is pending, so an idle library costs no wake-up.
  - An event is delivered 150 ms after it arrived, at most once per 37.5 ms (`FLUSH_GAP`) during a burst. A path is named once while it waits, and errors go out at the next delivery.
  - The thread delivers paths, the rescan flag and the watch-limit error, never kinds, so nothing is cancelled. cifix-2's `NoCancel` wrapper existed only to stop the old debouncer dropping a delete whose FSEvents flags still said "created". It is gone, and its test now runs through the new thread.
  - What the idle change leaves as it was: the reconciler, settle, echo, rename pairing, bursts, the poll fallback, rescans and the per-folder Linux watches. (The late item below then changes the burst rule after a repository operation.) `notify-debouncer-full` stays a dependency for `file_id` (Windows identity, `fileio/windows_id.rs`), and the manifest is unchanged.
  - Tests: `an_idle_debounce_thread_never_wakes` (0 wakes in an idle window; a ticking loop gives 14) and `watch_real::an_idle_os_source_wakes_nothing` on the real backend.
- **Go to script (gate E12).** `query_over_10k_entries_under_5ms` took 4.97–5.64 ms in release. Most of the time went to the subsequence table's allocations for keys that cannot match.
  - `hacivat-l10n`'s `fuzzy` now refuses a query that is not a subsequence of the key before it allocates, and its tables are flat. The answers are the same: a reference copy of the old body is compared on a generated corpus.
  - The index works out each entry's path scope once at build, instead of once per environment per query.
  - The ranking is unchanged, compared against the pre-change ranking on a generated corpus. The query now takes 0.77–0.80 ms.
- **Stub names (gate E9).** The stub-only variants that nothing constructed are removed: `RegistryError::NotImplemented`, `GitError::NotImplemented`, `UnavailableReason::NotImplemented`, `ReadError::NotImplemented` and `SaveError::NotImplemented`. A stopped watch source answers `WatchError::Stopped`, which is not user-visible. The Task 1a module docs now say what the stubs did. The Task 1a and Task 5 sections above describe the code as it was.
- **E8 test isolation.** The two memory perf tests of `hacivat-exec`'s `script_run` take one process-wide lock (`RSS_PERF`), because RSS is process-wide. The budget is unchanged.
- **`git_checkout_2000_is_one_rescan` (the gate CI on macOS, run 45202154953; locally 7 of 10 suite runs failed on the old code).** Traces of failing runs show FSEvents delivering a 2,000-file checkout late and in chunks: 700 files by the time git let go of `index.lock`, the rest in chunks up to 220 ms apart for 1.9 s. Three parts fix it, none of them a longer test wait:
  - The OS source hands `.git/index.lock` over without the debounce wait, so the reconciler looks at the lock while git still holds it.
  - After a repository operation ends, its burst goes on until the hints have been quiet for `OP_QUIET` (1 s; one window before), at most `REPO_OP_MAX` after the end, so late chunks count toward the operation's one `Rescan`. A small operation's files settle once that quiet passes.
  - After a burst rescan, a late hint folds into it (`RescanTail`, up to `TAIL` = 2 s after the last folded hint, at most 30 s). Its file must be unopened, have the stamp learned at or after the rescan's start, and be dated more than `TICK_MARGIN` (100 ms, or the file system's granularity) before the rescan began. A write after that stamp was learned would carry a later time, so R-T9-3's "racily clean" doubt cannot apply. Every other hint is looked at as before.
  - Result: `watch_real` passed 20 of 20 runs unloaded, 20 of 20 with 12 CPU hogs, and 20 of 20 with 16 test threads.
- **Fix round 1 (review C1, I1, I2, m2, m3, m6; ruling R-GF-1).**
  - **C1.** The post-operation phase (`OP_QUIET`) now lasts at most `OP_LATE_MAX` (4 s) after the lock went.
    - An overflowed burst's rescan is due at most `max_wait` after the overflow in every phase.
    - The rescan of an operation's burst clears the phase and the overflow.
    - A rescan is never due again before a window after the last one (`last_rescan`; `debug_assert`), so `due()` cannot answer the same past instant twice.
    - The review's probes are tests: a steady writer after an overflowed checkout gives one rescan, 2.0 s after the lock went; a small operation and then a generator give 5 rescans in 40 s.
  - **I1 (R-GF-1).** An open file's hint (`Debouncer::push_open`) is handed on at the next flush whatever the burst does. It is never swallowed by an overflow and never held by a repository operation, so E2's budget holds for open files: 150–250 ms at the reconciler in the tests. Only unopened files wait for the burst's rescan.
  - **m3.** A tab's read stamp that the watcher has not learned yet makes the file open (`Reconciler::opened`). The hash is attached once the watcher learns that stamp. The rescan tail never folds an open file.
  - **m2.** A sink that panics is caught (`catch_unwind`) and handed `RawHint::Lost`, so the watcher falls back and rescans. The debounce thread lives on.
  - **I2.** The thread publishes how many items wait (`DebounceCounts::pending`). The real-backend idle test starts its window only once nothing waits, and requires at most two wakes per event in it.
