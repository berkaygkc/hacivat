# Hacivat: notes for coding agents

## Build target slots (disk hygiene)
- Agent builds never create a new `CARGO_TARGET_DIR` per task. They use a fixed pool under `<checkout>/target-core/`:
  - `slot-core-1` … `slot-core-3` for core crates;
  - `slot-ui-1` … `slot-ui-2` for gpui (`hacivat-app`) builds;
  - `merge` for the integrator only.
- **Claiming a slot:** `mkdir <slot>.lock` (atomic). Write the owner and time into `<slot>.lock/owner`, and `rm -rf <slot>.lock` when done. Never delete the slot itself, so the dependency cache stays warm.
- **Changing worktree path:** if a slot was last built from a different path, first run `cargo clean -p <member>` for every workspace member (`cargo metadata --no-deps`). Stale `CARGO_MANIFEST_DIR` artifacts otherwise double the slot's size.
- **Environment:** `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`.

## Tests and temporary directories
- A test's `tempfile::TempDir` guard must live until the test ends, and until any child process it hands the path to (sshd, psql, the app) has exited.
- Never call `.keep()`, `.into_path()` or `mem::forget` on a test tempdir.
- In gpui tests, keep the guard in the test (or in a harness the test owns) and pass only the path into app state. The test context does not drop app state, so a guard stored there leaks.
