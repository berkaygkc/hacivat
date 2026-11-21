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
