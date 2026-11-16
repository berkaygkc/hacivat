# M2b "Secure network and sessions" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish M2 on top of M2a "Connect". That means:
- TLS `Auto` with its interactive half: certificate review, pinning, CA import, the public-plaintext question, and a hard stop when a pinned certificate changes;
- SSH tunnels: jump chains, shared sessions, host-key policies with Hacivat's own known-hosts list, ssh-agent, key files with passphrases, keyboard-interactive, and `~/.ssh/config` import;
- the session actor with protocol-derived transaction state, cancel escalation, reset and schema tracking;
- `hacivat-guard`, the read-only layers L0, L3, L4, L5 (role probe) and L6, with honest labels;
- liveness and reconnect with state replay and the D-085 "session lost" message;
- the Linux master-password vault;
- the full Docker matrix in CI, with every image pinned by digest.

M2's exit criteria (spec §6 M2) are checked at the end of this plan (Task 25).

**Architecture:**
- One new core crate, `hacivat-guard`. It is sans-I/O: it produces read-only plans, probe SQL, post-checks, SQLite hardening and authorizer decisions as data. The session layer runs them. Karagöz will link it later (§5.1 rule 3).
- `hacivat-net` gains `ssh/` (config import, known hosts, `TunnelManager` over `russh`), and TLS pinning with certificate capture.
- `hacivat-secrets` gains the `FileVault` and a `FallbackBackend` that switches to it on Linux without Secret Service.
- `hacivat-driver`'s `RawConn` gains a bounded text execution surface: `execute`, `batch`, `tx_status`, `reset` and `backend_id`. M3 adds typed streaming on top.
- `hacivat-session` gains `session/`: actor, transactions, cancel, environment plan, schema context, reset, router, pools, liveness, recovery and guarded read-only execution.
  - The M2a "main connection" becomes the profile's **default console session** (B4).
  - `ConnectionManager` keeps its public role (one `LiveState` per profile for the capsule). Internally it owns a `SessionManager` per profile.
- `mysql_async` 0.37.1 is vendored under `third_party/mysql_async/` with five small patches, wired in through `[patch.crates-io]` (B10). Patch §5 lets MySQL use Hacivat's own rustls `ClientConfig`, so pins, review, outcome and decrypted client keys work for MySQL exactly as for PostgreSQL (Q2).
- The app gets the M2b surfaces through stub modules that Task 1 creates. Each later task fills only its own modules. Task 1 makes `interaction.rs` route every new `Question` variant, and creates `events.rs`, which dispatches every `ConnectionEvent`.

**Tech Stack:**
- Everything from M2a. Its Tech Stack still applies, and so do its manifests and rulings.
- SSH: `russh` (current 0.5x; `default-features = false, features = ["ring", "rsa", "flate2"]`, ring backend only, never `aws-lc-rs`; Q28) with its `keys` module, used for the client, the agent client and, in tests, the server and the agent server.
- TLS display and keys: `x509-parser` (certificate review), `sha2`, `pkcs8` with `encryption`, `pkcs5`, `pem`, `std` and `sha1-insecure` (encrypted PKCS #8 client keys: PBES2, plus the SHA-1 PRF of OpenSSL 1.0 keys; Q5, Q36).
- Vault: `chacha20poly1305` (XChaCha20-Poly1305), `argon2` (Argon2id), `getrandom` (`getrandom::fill` for salt and nonce; already in the lock), and `secrecy` with its `serde` feature (vault JSON parsed into `SecretString`). All are RustCrypto or already in the graph, MIT OR Apache-2.0 (Q30).
- Known hosts: `hmac` and `sha1` for hashed `known_hosts` entries. Both are already in the graph through russh.
- Vendored: `mysql_async` 0.37.1 (MIT OR Apache-2.0) plus the patches in `third_party/mysql_async/PATCHES.md` (§1–§5).
- Exact versions are whatever is current and compatible when Task 1 adds them. `Cargo.lock` is the record. Task 1's manifest table is exhaustive. Any other crate needs a ruling.

**Execution (PO, 2026-11-12, nonstop mandate):**
- Subagent-driven, in parallel waves of 3–5 worktree agents.
- Branch `m2b` is cut from `m2a` at its tip after the `m2a-finalfix` worktree's work has merged into `m2a` (the M2a final review `489b039` and the CI fix `144b38c` are already in; Q45). Task branches are `m2b-<slug>`, based on `m2b`. No merge to `main` overnight (ledger ruling).
- The PO is asleep. Every open item is settled below as a ruling (B-numbers) and listed for the PO's morning review.

**Spec:** `docs/superpowers/specs/2026-11-12-hacivat-v1-design.md`. Read:
- the decision log, especially **L15** (M2b's scope), L1, L9 (later) and L13;
- the §4 rows "Connections and profiles", "Network security: TLS and SSH", "Secrets storage", "Sessions and transactions", "Safety and read-only", "Production UX", and the subsection **"Safety model (normative summary)"** with its engine-facts table;
- §5.1 (layout and dependency rules 1–4), §5.3, §5.4, §5.6, §5.7 and §5.8;
- §6 M2 (exit criteria), §7.1 (Docker matrix, bypass suite) and §7.2;
- §10 C1, C10, C11, C19 and C20.

Also read:
- `docs/decisions/open-product-decisions.md`: D-004, **D-005**, **D-014**, **D-016**, **D-035**, **D-036**, **D-037**, **D-085**.
- `docs/research/algorithms/01-connectivity-and-sessions.md`: HV-SEC-01 (SSH), HV-SEC-02 (TLS), HV-SEC-05 (vault), HV-CONN-01 (transport step, side channels), HV-CONN-04, HV-CONN-05, HV-SESS-01..08, and §5 (test infrastructure). **Numbering (R2, unchanged): `alg/01` numbers SSH as SEC-01 and TLS as SEC-02. L15 says "TLS … (HV-SEC-01)", "SSH (HV-SEC-02)". This plan uses `alg/01`'s numbers and L15's content.**
- `docs/research/algorithms/13-sql-safety-lint-format.md`: §0.1 (the layers L0–L7), §0.2 S-02, S-07, S-09, S-10 and S-11, SQL-SAFE-05 (per-engine guard) and SQL-SAFE-06 (bypass catalogue: PostgreSQL, MySQL/MariaDB and SQLite rows).
- `docs/research/algorithms/27-mcp-server.md` MCP-09, **step 1 only** (the role probe) and its PostgreSQL, MySQL and SQLite rows.
- `docs/research/algorithms/20-query-execution.md`: HV-EXEC-05 (actor and queue; request kinds only), HV-EXEC-09 (cancel) and HV-EXEC-10 (transaction state). M3 owns the rest.
- `design/m2-connect/NOTES.md` surfaces 5, 6 and 7 and `index.html`. They are PO-approved and **binding**. Also `design/m2b-secure/` (Task 2), which is controller-approved and binding once Task 2 merges.
- `DESIGN.md` (with its "Connection Flow (M2a)" components) and `.impeccable/design.json`.
- `docs/superpowers/plans/2026-11-12-m2a-connect.md`: its Global Constraints and rulings R1–R31, which are **inherited**, and its appendix of pre-flight rulings.
- `docs/decisions/0002-driver-crates.md`, especially the "Known limits" and the "M2b follow-ups" in each section.
- The M2a ledger `<main>/.superpowers/sdd/2026-11-12-m2a-connect/progress.md` and the M2b ledger `<main>/.superpowers/sdd/2026-11-13-m2b-secure-network-sessions/progress.md`. Read both by absolute main-checkout path: `.superpowers/` is git-ignored.

## Global Constraints

Every task's requirements include this section. `<main>` is the absolute path of the main checkout; the controller states it in every brief and in `global-constraints.md`.

- **Inherited.** The M2a Global Constraints apply unchanged, except where a line below replaces or extends one. They cover:
  - the M1 rules: GPUI pin, no Zed GPL crates, toolchain 1.98.1, no `unsafe`, ASCII-only casing (INV-3), the text rules, the Fluent grammar, semantic tokens, the named design rules, the stubs rule F31 and the test-bodies rule F9;
  - layering; secrets; threading (§5.4, restated below as the Q4 ruling); machine text; Fluent conventions; engines and versions;
  - Docker, the full test set, execution, modal layers (extended below, Q34), Escape and paste in fields, key bindings, writes, and rate limits.
- **Branches.** The feature branch is `m2b`, cut from `m2a` after the `m2a-finalfix` work has merged into `m2a` (Q45). Each task works in its own worktree on a branch named `m2b-<slug>`. Its first commands are `git checkout -B m2b-<task-slug> m2b`, then `git merge-base --is-ancestor m2b HEAD`. Stop if either fails. Never use `m2b/<slug>`: git cannot create it while `m2b` exists.
- **SDD files.** The ledger, `global-constraints.md` (an exact copy of this section) and the implementer and reviewer rules live in `<main>/.superpowers/sdd/2026-11-13-m2b-secure-network-sessions/`. Briefs cite them by absolute path.
- **Disk and target directories (Q1; M2a W4 lesson: the shared gpui `target` caused stale-artifact collisions).**
  - **Admission.** Before each dispatch the controller checks `df -h <main>`. A gpui agent (any task that builds `gpui`: every `crates/ui` and `crates/app` task) starts only with **≥ 25 GiB free**; a core-only agent only with **≥ 8 GiB free**. Otherwise the task queues.
  - **At most two gpui agents run at once unless `df` shows ≥ 60 GiB free.**
  - Every agent exports **`CARGO_INCREMENTAL=0`**.
  - **Task 1 builds in `<main>/target`** (warm; it is the only W0 builder).
  - Every other task uses its own **`CARGO_TARGET_DIR=<main>/target-core/m2b-tNN`** (gpui tasks included), and the controller **deletes that directory after the task merges**.
  - No two agents ever share a target directory.
  - The controller's final verification of a wave runs in `<main>/target`, after `cargo clean -p <every crate the wave touched>`.
  - Before its own final run, each task runs `cargo clean -p <crates it touched>` in its own directory.
  - Nobody but the controller runs `cargo test --workspace`.
- **Dependencies.**
  - Task 1 writes every manifest change M2b needs (its Step 3 table), `deny.toml`, the `[patch.crates-io]` entry and the vendored `third_party/mysql_async/`. No later task edits a manifest, except: Task 8, which edits files inside `third_party/mysql_async/src/**` only; and Task 13, which removes `rustls-native-certs` from `crates/core/hacivat-driver-mysql/Cargo.toml` (a removal only, once §5 makes it unused; Q2).
  - **Never run a plain `cargo update`.** Add a dependency by editing the manifest, then `cargo check`. If a bump is unavoidable, use `cargo update -p <crate> --precise <v>` and list it in the commit body.
  - "Only additions" in `Cargo.lock`: an upward bump of an existing entry is allowed only when listed in the commit body. The `mysql_async` entry losing its `source` and `checksum` (it becomes a path package) is an allowed change; Task 1 lists it (Q29).
- **Licences and crypto.**
  - The cargo-deny allow-list is unchanged.
  - `aws-lc-rs` and `aws-lc-sys` stay banned. russh is built with `default-features = false, features = ["ring", "rsa", "flate2"]` (Q28); Task 1 proves this with `cargo tree -i aws-lc-rs` (expected: nothing) and records `cargo tree -e features -i russh`, `cargo tree -i ring`, `cargo tree -i rsa` and `cargo tree -i pageant --target x86_64-pc-windows-msvc` in ADR 0003.
  - russh's RSA support pulls the `rsa` crate (RUSTSEC-2023-0071, the Marvin timing issue, no fixed version). It is accepted, and ADR 0003 records why:
    - the advisory concerns RSA decryption and signing timing that an attacker measures, and Hacivat only signs SSH authentication challenges locally with the user's own key;
    - Ed25519 keys are preferred in the copy.
  - The `deny` CI job checks licences, bans and sources (no advisories), unchanged from M2a. RUSTSEC-2023-0071 is therefore a **documented acceptance, not a CI pass**; the ledger records it (Q45).
- **Secrets (extended).** The M2a rules now also cover:
  - **SSH passwords, SSH key passphrases, keyboard-interactive answers** (one-time codes included), **the vault master password**, **TLS key passphrases**, and **vault contents**.
  - None of these ever appears in `connections.toml`, `known_hosts`, `vault.bin` in plain text, logs, `Debug`, panic messages, test output or snapshots. A type that holds one derives `Debug` only (redacted), never `PartialEq`.
  - **Secret-scan invariant (M2a T7, `carries_secret`).** Task 1 makes `hacivat_driver::url::carries_secret` public (in `url/native/mod.rs` and its re-export in `url/mod.rs`). Every value that M2b reads from a file the user did not type into Hacivat passes it before it reaches a profile:
    - `~/.ssh/config` values;
    - imported key paths;
    - `known_hosts` comments.

    A hit refuses the value: the entry gets a `ConfigIssue::SecretInValue` notice, and the value is never echoed.
  - The vault key and the decrypted map live only in `Zeroizing` buffers or `SecretString` (the vault JSON is parsed straight into them; Q30). The vault file is written atomically (temp file, `fsync`, rename) with mode `0600`.
  - **Session overlay (Q17).** A secret that worked for a live connection (including retention `Never`, one-time codes and a typed SSH passphrase) is kept in memory, in `SecretString`/`Zeroizing` buffers, by that profile's `SessionManager` for reconnects and side connections only. It is dropped at disconnect, `forget` and quit, and never persisted. ADR 0003 records this.
- **Security review.** Tasks 1, 3, 4, 5, 6, 8, 9, 10, 11, 13, 14, 15 (it holds the session overlay's secrets, Q17), 18, 19, 21 and 23 are security-sensitive.
  - Their reviewers run on **opus**, with the **adversarial probes** listed in each task's "Review" line.
  - The reviewer writes and runs at least one hostile test that the implementer did not write, and reports it.
- **ADRs.** Every security choice is recorded:
  - `docs/decisions/0003-secure-network.md`: TLS pins and review, the plaintext question, the insecure-auth guard, channel binding, SSH, known hosts, the agent, the vault, the session overlay, the keyring CI and the deferrals;
  - `docs/decisions/0004-sessions-and-read-only-guard.md`: the actor model, transaction state, cancel ladder, reconnect policy, guard layers per engine, labels, the role probe and the bypass-suite results;
  - `docs/decisions/0002-driver-crates.md`: one M2b section per driver task and one for the vendored `mysql_async`.

  Task 1 creates 0003 and 0004 with their Context and Rule headings. Each task appends its own section. A security choice made during a fix round goes into the same task's section before re-review.
- **Append-only shared files.** Each task appends a block headed:
  - FTL and TOML: `## m2b-tNN-<slug>`;
  - Rust: `// ## m2b-tNN-<slug>`;
  - ADRs: `## m2b-tNN — <topic>`;
  - `ci.yml`: `# ## m2b-tNN-<slug>`.

  The files:
  - `locales/{en,tr}/{hacivat-ui,hacivat-core}.ftl`;
  - `locales/_schema/literal-allowlist.toml`;
  - ADRs 0002, 0003 and 0004;
  - `.github/workflows/ci.yml`;
  - `crates/app/hacivat-app/tests/support/mod.rs`;
  - `crates/core/hacivat-session/tests/common/mod.rs`;
  - `docker/m2a/init/*.sql`.

  The controller resolves append-only conflicts by keeping both blocks.

  **Named exceptions** (each edits another task's block, once):
  - Task 14 rewrites `error-conn-tls-untrusted-body` (in M2a T10's block of `hacivat-core.ftl`) back to the mock's `err-tls-b` text, now that review exists;
  - Task 19 deletes `hv-conn-fix-require` (in M2a T20's block, EN and TR);
  - Task 20 rewrites the `hv-conn-issue-ssh-*` placeholders, and Task 19 rewrites `hv-conn-issue-tls-key-legacy-encrypted`, in Task 1's block of `hacivat-ui.ftl` (EN and TR; Q3).
- **Docker.**
  - The compose project `hacivat-m2a` (`docker/m2a/`, containers `m2a-<id>`) stays the matrix (B17), and it is controller-owned. Only the controller runs `cargo xtask docker up` or `down`. Task 16 Step 5 is the one task step that may, and it runs only after the controller confirms Task 13's Docker run is finished (Q7).
  - Tests that need a server are `#[ignore = "docker"]` and run only through `cargo xtask docker test --set pr|full [-p <crate>]`.
  - **Set-aware targets (Q6).** A Docker test gets its targets only from `hacivat_testkit::require_target(id)` (returns `None`, with `eprintln!("skip {id}: not in set {set}")`, when the id is not in the active `HACIVAT_DOCKER_SET`; panics when the id is in the set but unhealthy) or `targets(Some(engine))`. No literal target id appears outside a `require_target` call. A target in the active set never skips silently (M2a ruling).
  - Through SSH, a test reaches the database at the target's in-network endpoint `DockerTarget.internal` (compose service name and internal port; Task 16), never `127.0.0.1:<published port>`.
  - Fixture roles and tables that a test needs (`hv_ro`, `hv_probe`, `hv_probe_log`) are created idempotently through the admin connection by the test's helpers, so a test never depends on a volume having been recreated (Q7).
  - In-process servers (the testkit SSH server, agent and `ChaosProxy`) need no Docker. Their tests run in plain `cargo test`.
- **Full test set.** "All tests pass" means every line below. Each task runs the lines for the crates it touches. PR CI runs lines 1–6 and `docker-pr`. Nightly runs line 7, `docker-full` and the keyring jobs. The gate runs all of them.
  ```
  cargo test --workspace
  cargo test -p hacivat-ui-core --features test-support,scene-check
  cargo test -p hacivat-l10n --features pseudo
  cargo test -p hacivat-app --features scene-check,pseudo
  cargo test -p hacivat-driver --features fake
  cargo test -p hacivat-testkit --features ssh
  cargo test --release -p hacivat-l10n --features pseudo
  cargo xtask docker check-pins
  cargo xtask docker test --set full            # Docker (local; CI: docker-pr on PRs, docker-full nightly)
  HACIVAT_KEYRING_LIVE=1 cargo test -p hacivat-secrets -- --ignored live_keyring_round_trip   # gate, PO's Mac; CI on Linux and Windows
  ```
  **Every new test file compiles under plain `cargo test --workspace` (Q5):**
  - scene-check-only asserts are `#[cfg(feature = "scene-check")]` (the M2a `connections_live.rs` pattern);
  - unix-only tests and asserts are `#[cfg(unix)]` (`TestAgent`, `0o600` mode asserts, `std::os::unix::fs::symlink`); Windows CI runs `cargo test --workspace`;
  - each task also runs `cargo test -p <crate> --no-run` without features;
  - a test that needs the platform keyring state injects its backend and waits for the probe to land (`144b38c`), never the real platform keyring.
- **Test timing (Q4, Q25).**
  - Tests wait on a condition, never on a fixed sleep: `run_until_parked`, or a settle loop that advances the clock and re-checks a predicate (the app's `settle_until`, appended by Task 1), bounded by a timeout. Real-time sleeps are allowed only where a real server or socket sets the pace (Docker, chaos), and are then bounded.
  - `#[tokio::test(start_paused = true)]` is used only with FakeDriver or pure state, **never with in-process servers on real sockets** (the paused clock auto-advances while the runtime waits on socket I/O and fires timeouts spuriously).
- **Threading (Q4 ruling; restates the inherited M2a line).** The UI thread never awaits I/O, and nothing on the UI thread prompts the keychain (§5.4).
  - **Awaiting `tokio::sync` channels (`oneshot`, `watch`, `mpsc`, `broadcast`) from GPUI foreground tasks is ALLOWED.** It is what M2a ships and it passed CI. Every new prompt follows the `AppUiPort` pattern: the question arrives on an `mpsc` channel, and the answer leaves on a `oneshot`. A prompt closed by any means answers `Answer::Cancelled`. Core-runtime calls made from the UI (vault create, unlock and reset; `KnownHosts::replace`; `read_ssh_config`) return through a `oneshot` the foreground task awaits.
  - **The determinism requirement is met in tests:** every shared test helper that builds a shell with connections calls `cx.executor().allow_parking()` (as M2a's `shell_with_connections_prepared` does since `144b38c`), and tests wait on a condition (above), never on fixed sleeps.
  - **Do not convert `live.rs`, `events.rs`, `interaction.rs` or `password_prompt.rs` to polling.** Timer polling is used only where there is no channel to await.
- **Modal layers (M2b additions, Q34).**
  - Custom prompt layers follow the M2a Dialog key-phase fix: Enter acts on key-down (bubble), never when held or with a modifier; the key-up `Enter` is suppressed in capture; Escape, Enter and Tab bubble out of a nested Select.
  - Hard stops (host key changed, pin changed) have no Enter default other than Cancel.
  - Row-level keys (Task 20's Alt+↑/↓) are element-level `on_key_down`, never `cx.bind_keys` (`menus::rebuild` owns bindings).
  - Raised Ink: fingerprint insets and popovers on raised surfaces use text-2.
- **No silent weakening.** A stub (F31) for a security layer fails closed. Examples:
  - the read-only executor stub returns `ExecErrorKind::Internal`;
  - the pin-check stub treats every pin as a mismatch;
  - the host-key stub refuses.

  It never succeeds without the check.
- **Rate limits.** Watch `~/.claude/rate-limits.json`. Pause new dispatches at `five_hour ≥ 80 %` or `seven_day ≥ 90 %`.

## Review Focus

These are the five input classes the spec implies, no test elsewhere exercises them, and they are most likely to hurt users. Most likely first.

1. **The network goes away mid-work.**
   - Cases: a laptop sleeps for two hours; Wi-Fi drops during `INSERT`; the server is restarted; the SSH bastion's session dies; the clock jumps.
   - Expected:
     - a user statement is never replayed, and an interrupted one reports "outcome unknown";
     - recovery sends one ping, not 120;
     - each dependent profile reconnects once (group invalidation), with backoff;
     - after a reconnect, the "Reconnected to …" toast lists exactly what was lost (an open transaction, temp tables, session variables).
   - Tests:
     - `user_statement_is_never_replayed_after_connection_loss` and `clock_jump_pings_once_not_per_missed_tick` (Task 17);
     - `one_ssh_death_reconnects_each_dependent_once` (Task 17, with the in-process SSH server);
     - `terminated_backend_with_open_tx_reports_lost_tx` and `insert_killed_mid_flight_reports_outcome_unknown_without_duplicate` (Task 23, Docker).
2. **Hostile or odd SSH environments.**
   - Cases: `~/.ssh/config` with `ProxyCommand`, an `Include` loop, `Match` blocks, unsupported `%` tokens, or a password-looking value; hashed `known_hosts` lines; `@revoked` and `@cert-authority` markers; an agent holding many keys against `MaxAuthTries`; a bastion with `AllowTcpForwarding no`; IPv6 bastions; a host key that changed.
   - Expected:
     - nothing runs a command;
     - nothing auto-accepts a changed key;
     - Hacivat never writes `~/.ssh/known_hosts`;
     - every refusal names the hop.
   - Tests:
     - `proxycommand_entries_are_listed_but_unusable`, `include_loop_stops_at_depth_8`, `match_blocks_are_skipped_with_a_notice`, `secret_looking_values_are_refused`, `hashed_entries_match` and `revoked_marker_is_a_hard_stop` (Task 4);
     - `identities_only_tries_the_key_file_first`, `many_agent_keys_without_identities_only_reports_auth_with_hop`, `forwarding_refused_names_the_hop`, `server_initiated_channels_are_refused` and `changed_key_is_never_accepted_in_any_policy` (Task 9).
3. **Certificates in the wild.**
   - Cases: a self-signed `CA:TRUE` leaf; an IP-only SAN; an expired pinned certificate; a legitimately rotated certificate on a pinned endpoint; Apple's 825-day rule; a subject with Turkish characters; TLS through SSH (SNI is the database host, never `127.0.0.1`).
   - Expected:
     - pinning works for the first five;
     - a pin never extends expiry;
     - rotation is a hard stop with old and new fingerprints;
     - subjects are shown verbatim.
   - Tests:
     - `ca_true_self_signed_can_be_pinned`, `ip_only_san_can_be_pinned`, `long_validity_cert_can_be_pinned`, `pin_does_not_override_expiry`, `rotated_certificate_is_pin_mismatch_with_both_fingerprints`, `pinned_endpoint_never_allows_plaintext` and `turkish_subject_is_verbatim` (Task 3);
     - `pin_flow_on_mysql84` (Task 13, MySQL through Hacivat's own `ClientConfig`, Q2);
     - `tls_through_ssh_sends_the_database_host_as_sni` (Task 14).
4. **Read-only bypass attempts with no classifier.**
   - Cases: `COMMIT; DROP …`, `SET TRANSACTION READ WRITE`, MySQL DDL, `/*!50000 DROP TABLE t */`, `SELECT pg_advisory_lock(1)`, SQLite `ATTACH 'new.db'` and `VACUUM INTO`, `PRAGMA query_only=0`.
   - Expected: the server-side layers L3–L6 alone block each one, or neutralise it after the request. No file is created. No lock or session state survives the request.
   - Tests: `pg_bypass_rows_blocked_without_classifier`, `mysql_bypass_rows_blocked_without_classifier` and `sqlite_bypass_rows_blocked_without_classifier` (Task 23); `attach_and_vacuum_into_leave_no_file` (Task 11); `smuggled_commit_is_an_enforcement_gap_and_discards_the_connection` (Task 18).
5. **Secrets in the new places.**
   - Cases: an SSH password or passphrase, a keyboard-interactive one-time code, the vault master password, a TLS key passphrase. Also: a vault file that was tampered with, truncated or written by a newer version; a wrong master password three times; a master password forgotten.
   - Expected:
     - none of the secrets reaches a log, a profile or `Debug`;
     - a tampered vault is never overwritten;
     - a forgotten password can only reset, with a typed confirmation.
   - Tests:
     - `ssh_secrets_never_logged` (Task 9);
     - `tampered_vault_is_untouched`, `newer_vault_version_is_read_only`, `wrong_master_password` and `crafted_header_params_are_corrupt` (Task 6);
     - `vault_locked_asks_unlock_then_reads`, `vault_unlock_cancel_falls_back_to_password_prompt` and `three_wrong_master_passwords_fall_back_to_password_prompt` (Task 14);
     - `prompted_ssh_secret_stored_only_after_success` and `connect_logs_never_contain_any_m2b_secret` (Task 14);
     - `vault_reset_needs_typed_confirmation` (Task 21).
   - `capture_tracing` (every "never logged" test) captures at TRACE and installs the `log` bridge (`tracing_log::LogTracer`), so russh and `mysql_async` `log` records are scanned too (Q33).

## Assumptions and rulings

These are controller calls made while the PO is away. The PO can overrule each one. B-numbers continue M2a's R-numbers without reusing them. Each says why, and what it costs if wrong.

- **B1. Scope of "SESS-01..08" in M2b (L15).**
  - **M2b ships the core of every SESS spec with tests:** actor, queue, transaction state, cancel ladder, environment plan, schema context, reset, router, pools and budget.
  - **It ships the UI only where the mocks show it:** the reconnecting capsule and the session-lost toast.
  - These move to **M3**, where consoles exist (L15(c): no query console in M2):
    - the transaction badge, Commit and Roll back, mode and isolation selectors;
    - the cancel-escalation dialog;
    - the idle-transaction and close-with-open-transaction prompts;
    - "waiting for a connection";
    - the single-session toggle.

    M2b's core emits the events those surfaces need (`SessionEvent`, `ConnectionEvent`).
  - **Also M3:**
    - **Smart commit** (it needs the `sql-safety` classifier). `TxMode::Smart` exists, and `set_mode(Smart)` returns `TxError::NeedsClassifier`.
    - **Startup scripts** (they need the execution-mode splitter). `EnvStep::StartupScript` is reserved.
    - **Layers L1 and L2** (single-statement gate and classifier).
  - *Cost if wrong:* a PO who wants these in M2 adds about two UI tasks to M2b.
- **B2. `RawConn` gains a bounded text execution surface.** The methods: `execute(sql, ExecOpts) -> ExecSummary`, `batch(sql)`, `tx_status()`, `reset()` and `backend_id()`. `ExecSummary` carries the command tag, rows affected, and up to `max_rows` rows as text. Rows are filled by the simple protocol only; extended-protocol rows are counted, not decoded. M3 adds typed streaming fetch next to it. This supersedes M2a R29 ("M3 adds execute and fetch"): sessions, transactions, the guard and the role probe cannot be built without it.
- **B3. Transaction state comes from the protocol (HV-SESS-03).**
  - **PostgreSQL:** `tokio-postgres` does not expose the `ReadyForQuery` status. The driver's own stream wrapper, `wire::Observer`, frames the backend messages and records the last `Z` status byte (`I`/`T`/`E`). The same observer enforces B8.
  - **MySQL and MariaDB:** `Conn::last_ok_packet().status_flags()` (`SERVER_STATUS_IN_TRANS`). After an `ERR` packet the previous state is kept, because MySQL rolls back only the statement.
  - **SQLite:** `Connection::is_autocommit()`.
- **B4. The default console session.**
  - The M2a main connection becomes `SessionKind::Console(ClientId::DEFAULT)`. It is the profile's default session, and it takes the `Priority::Console` budget permit M2a already takes.
  - M3's first console attaches to it.
  - It reconnects **eagerly**, with backoff, because the capsule shows it (mock "Reconnecting · attempt 3"). Other consoles (M3) reconnect lazily, on their next request (HV-CONN-05).
- **B5. TLS pins.**
  - A pin is the SHA-256 of the **leaf certificate DER**. That is the fingerprint the dialog shows, the one users compare (`openssl x509 -fingerprint -sha256`), and what the copy promises ("trusts exactly this certificate"). `alg/01` says SPKI; this deviation is recorded in ADR 0003.
  - Pins are stored in the profile as `tls.pins = [{ endpoint = "host:port", sha256 = "<hex>" }]`. `host` is the TLS host as typed, and IPv6 goes in brackets. A fingerprint is not a secret.
  - A matching pin overrides the chain-trust and name checks. It never overrides `NotBefore` or `NotAfter` (D-005: "a pin doesn't extend the expiry date").
  - A pin is checked in every mode except Disable. Require with a pin verifies the pin.
  - **A pin also forbids the plaintext fallback (Q13).** `build_tls_params` sets `plaintext_allowed = false` whenever `settings.pins` holds an entry for `pin_endpoint(tls_host, port)`, even with `allow_public_plaintext`. A pin means "I expect TLS with this certificate"; an `N` answer to the `SSLRequest` on a pinned endpoint fails `tls.not_supported_by_server` with fixes `[EditConnection]` and is never asked. MySQL is unaffected (its Auto fallback is loopback-only, and loopback skips pins).
  - Pins apply to MySQL and MariaDB exactly as to PostgreSQL, through patch §5 (B10, Q2).
  - A changed certificate on a pinned endpoint is a hard stop: new class `tls.pin_changed`, with fix `ReviewCertificate`. The review dialog shows the old and the new fingerprint. Replacing the pin is an explicit action, never the default button.
- **B6. Interactive TLS (D-005 option A).**
  - Interactive `tls.untrusted` (`UnknownIssuer`, including `CaUsedAsEndEntity` and the Apple policy codes M2a maps to it) in `Auto`, `VerifyFull` or `VerifyCa` asks `Question::Certificate`. The answers:
    - `Pin`: profile change `AddPin`, then reconnect;
    - `ImportCa(path)`: profile change `SetCaFile`, and the path is added to `ApprovedRoots`, then reconnect;
    - `Cancelled`: `tls.untrusted`, with fixes `[ReviewCertificate, ImportCa, EditConnection]` and the certificate in `ConnectError.certificate`.
  - Headless: `tls.untrusted`, with no prompt.
  - `tls.hostname` offers no pin: a name mismatch on a chain the platform trusts is not "untrusted".
  - **The interim "Encrypt without verifying" fix (M2a R3) is removed.**
    - `FixId::EncryptWithoutVerifying` stays in `hacivat-proto`, because ids are stable, but nothing offers it any more.
    - `hv-conn-fix-require` is deleted.
    - `error-conn-tls-untrusted-body` returns to the mock text ("…Review it before you trust it.").
    - `hv-conn-enc-unverified` stays, for profiles the user set to Require.
- **B7. Public host without TLS (D-005).**
  - Interactive `tls.not_supported_by_server` under `Auto` asks `Question::PlaintextPublic`. The first attempt has already stopped at the TLS step with no credential sent (P9).
  - The answers:
    - `ConnectPlaintext { remember }`: one retry with plaintext allowed. `remember` adds profile change `AllowPublicPlaintext` (`tls.allow_public_plaintext = true`).
    - `UseSsh`: the UI opens the connection dialog with the SSH section open.
    - `Cancelled`.
  - Headless fails as M2a did.
  - The permanent "unencrypted" badge applies to these connections too.
- **B8. Insecure-auth guard: refuse to expose a password unless it is safe.**
  - `DriverConnect.insecure_auth` is `Allow` when any of these holds:
    - the address is loopback or a Unix socket;
    - the mode is `Disable`;
    - the user consented to public plaintext (B7).
  - Otherwise it is `RefuseUnlessVerified`. The driver then refuses any step that would reveal the password, unless the TLS outcome is `Full`, `CaOnly` or `Pinned`.
  - **PostgreSQL:** the `wire::Observer` refuses `R` codes 3 (cleartext) and 5 (md5) before `tokio-postgres` answers.
  - **MySQL and MariaDB:** the vendored patch §2 adds `OptsBuilder::password_guard(Option<Arc<dyn Fn() -> bool + Send + Sync>>)`. It is evaluated at the `caching_sha2_password` full-authentication point (`0x04`), before the password is sent in clear or the server's RSA key is requested. The driver supplies "outcome ∈ {Full, CaOnly, Pinned} (read from `TlsParams.outcome`) or `InsecureAuth::Allow`". `sha256_password` is not reachable in `mysql_async` (the handshake maps it to native), and `mysql_clear_password` stays off (`enable_cleartext_plugin(false)`, M2a).
  - **The guard is outcome-based for both engines (Q20)**, never mode-based: Require with a pin is `Pinned` and is allowed.
  - The result is the new class **`auth.insecure_method`**, with fixes `[ReviewCertificate, EditConnection]`. When TLS was up, `ConnectError.certificate` carries the presented chain's review, so "Review certificate" can pin; after the pin the outcome is `Pinned` and the guard allows the password (Q20).
  - *Cost:* an md5-only private server without TLS under `Auto` now needs an explicit `Disable`. M2a profiles set to Require by "Encrypt without verifying" now hit `auth.insecure_method` (PostgreSQL md5/password: always; MySQL: on a `caching_sha2` cache miss); the fix is Review certificate → Pin. Both are behaviour changes from M2a and are on the PO list.
- **B9. Channel binding.**
  - `PgTlsStream::channel_binding()` returns `tls-server-end-point` (RFC 5929): SHA-256 of the leaf for MD5-, SHA-1- and SHA-256-signed certificates, and the signature's own hash (SHA-384 or SHA-512, including `ecdsa-with-SHA512`) otherwise; RSASSA-PSS uses the hash from its parameters (Q36). Ed25519 and any unknown signature algorithm give no binding.
  - `tokio-postgres` uses `ChannelBinding::Prefer` (its default), so `SCRAM-SHA-256-PLUS` detects a man in the middle that terminates TLS, even under Require.
- **B10. `mysql_async` is vendored, not forked.**
  - `third_party/mysql_async/` holds 0.37.1 plus five patches, each a numbered section in `PATCHES.md` with its diff summary and an upstream PR draft:
    1. `skip_domain_validation` matches the `NotValidForName` and `NotValidForNameContext` **variants**, not their text. An upstream bug fix (M2a P11); Hacivat's own VerifyCa no longer depends on it (§5).
    2. `OptsBuilder::password_guard(Option<Arc<dyn Fn() -> bool + Send + Sync>>)`: an outcome predicate evaluated at the `caching_sha2_password` full-auth point (`0x04`); `false` refuses before the cleartext password or the RSA key request (B8, Q20).
    3. `OptsBuilder::multi_statements(bool)` drops `CLIENT_MULTI_STATEMENTS` (L3 for read-only).
    4. `OptsBuilder::local_infile_capability(bool)` drops `CLIENT_LOCAL_FILES`.
    5. `SslOpts::with_rustls_client_config(Arc<rustls::ClientConfig>)`: when set, `build_tls_connector` returns `TlsConnector::from(cfg)` and ignores the roots, the client identity and the danger flags; the hostname override still supplies the server name. Hacivat's `TlsParams.config` is used as is: the pin-aware recording verifier (pins, presented-chain capture for review, `TlsParams.outcome`), the platform verifier, the CA file, and the client identity Hacivat decrypted (encrypted PKCS #8, B25). This is the same configuration PostgreSQL uses (Q2).
  - *Why vendor:* no git fork on the PO's GitHub account overnight, and no `allow-git` entry.
  - Upstream PRs are a PO action on the morning list.
  - **VerifyCa is enabled for MySQL and MariaDB**, running through Hacivat's own `ChainOnly` verifier via §5: Validate drops the `TlsModeUnsupported` rule, and the TLS Select enables the mode.
  - The `rustls-native-certs` root path is removed from the MySQL connect path (the platform verifier is used, as for PostgreSQL). ADR 0002 records the removal of M2a's "OS roots vs platform verifier" difference.
  - MySQL `Auto` plaintext fallback stays loopback-only (M2a ruling). M2a limited it because `mysql_async` sent the password on the RSA-key path; patch §2 removes that reason, so **widening it to Private is a PO question** on the morning list (Q41). Until the PO answers, the M2a rule stands.
- **B11. SSH (HV-SEC-01, D-037).**
  - The library is `russh` with the ring backend. Auth methods: agent, key file (OpenSSH and PKCS #8, passphrase from `SecretSlot::SshPassphrase { hop }`), password and keyboard-interactive.
  - Not in v1: `ProxyCommand` (D-037: entries are shown disabled), PuTTY `.ppk` and Pageant (v1.x), and a forced local port (so `sec-ssh-forced-port-multi` is not reachable).
  - **Hops.** `SshSpec.hops` lists the jump hosts first and the bastion last. The bastion uses the auth method chosen in the dialog.
  - **Jump hosts** try, in order: the agent, the bastion's key file if one is set, an `IdentityFile` imported for that hop, then a password or keyboard-interactive prompt if the server offers it.
  - **Order within one hop** (`alg/01`): agent → key → password → keyboard-interactive. With `IdentitiesOnly`, the key file comes before the agent.
  - **Host keys.** Hacivat keeps its own store at `<config>/known_hosts`: OpenSSH format, `0600`, ordered writer. It reads `~/.ssh/known_hosts` and `known_hosts2` read-only, hashed entries included, and **never writes them**.
    - `KnownHosts::load` does no I/O: the files are read lazily on the core runtime at the first `check` or `trust` (no startup file I/O on the main thread; Q42).
    - `check` re-reads an OpenSSH file whenever its `(mtime, len)` changed since the last read (one `stat` per check, under the writer's mutex), so the copied `ssh-keygen -R` advice takes effect on the next retry without a restart (Q24).
    - Policies: `Ask` (default), `AcceptNew` and `Strict`. There is never an "accept all".
    - `AcceptNew` is an explicit user policy, so it also applies headless. It never applies to a changed key.
    - A changed key is a hard stop (`ssh.host_key_changed`). The review dialog offers to replace the old key only when that key is in Hacivat's own store. Otherwise it shows the `ssh-keygen -R` command to copy.
    - `@revoked` gives the new class **`ssh.host_key_revoked`**.
    - `@cert-authority`: host certificates are verified when russh exposes them. Otherwise the key is treated as unknown. Task 4 pins whichever behaviour holds.
    - Per-profile `UserKnownHostsFile` is not imported (v1.x).
  - **Remote name resolution only.** The database host goes into the `direct-tcpip` request. With SSH the trail drops `Resolve` and shows `Ssh` before `Connect`, a deliberate change to the mock's `stepsFor`. `ResolveStep::applies` is `false` for `TransportSpec::SshTunnel`: no local DNS lookup of the database host, which would also leak internal names (Q44).
  - **Channel hygiene (Q46).** Hacivat never requests agent forwarding, X11, a pty, a shell or exec, and never accepts a server-initiated channel. `direct-tcpip` originator fields carry `127.0.0.1:0`, not a local address.
  - **The address class under SSH** comes from the typed database host: an IP literal is its own class, `localhost` is Loopback, and any other name is Private (it resolves inside the bastion's network). B8's guard still applies.
  - **Sharing** is keyed by the chain prefix: host, port, user, auth-identity fingerprint and policy per hop. A Test connection never shares (CONN-07).
  - **Timing.** An idle tunnel closes after a 30 s grace. Keep-alive runs every 30 s per hop, up to 3 failures. Hops are released in reverse order, on partial failure too.
  - **SSH secrets** have their own retention, `Profile.ssh_retention` (default Forever), matching the mock's second Remember Select. `SecretSlot::SshPassphrase { hop }` holds one passphrase per hop: the first key on that hop that needs one. ADR 0003 records the limit (Q43).
  - **MySQL through SSH** uses M2a's `LoopbackForwarder` over the tunnel factory (R18). PostgreSQL takes the channel stream directly.
- **B12. `~/.ssh/config` import uses its own parser** (no new crate).
  - Supported:
    - `Host` patterns with `*`, `?` and `!`;
    - first-value-wins;
    - `Include` with `~` and a `*` glob in the last segment only, at most 8 levels deep;
    - the keys `HostName`, `User`, `Port`, `IdentityFile`, `IdentitiesOnly`, `ProxyJump` (chains and comma lists), `ServerAliveInterval`, `StrictHostKeyChecking` (`yes` → Strict, `accept-new` → AcceptNew, `ask`/`no`/`off` → Ask, with a notice for `no`/`off`), `UserKnownHostsFile` (noted, not imported) and `ProxyCommand` (the entry is marked unusable).
  - `Match` blocks are skipped with a notice.
  - `IdentityFile` tokens `%d %h %p %r %u %%` are expanded, and `HostName` with `%h` is expanded the same way. Any other token makes the entry unusable.
  - `IdentityAgent` (the 1Password and Secretive agents) is noted, not imported (v1.x): `ConfigNotice::IdentityAgentIgnored { alias }` tells the user why the agent is not used (Q43).
- **B13. Vault (D-016 option A, HV-SEC-05, M2a R5).**
  - **Format.** `FileVault` at `<data>/vault.bin`:
    - header: magic `HVVAULT1`, `format_major = 1`, Argon2id `m = 65536 KiB`, `t = 3`, `p = 1`, a 16-byte salt and a 24-byte nonce;
    - body: one XChaCha20-Poly1305 blob over a JSON map from service name to secret. The header is the associated data.
  - **The header is unauthenticated until the key is derived, so it is bounded first (Q30):** `m_kib` in 8,192–1,048,576, `t` in 1–10, `p` in 1–8, otherwise `VaultCorrupt` (a crafted `m_kib` would otherwise abort the process on allocation). Salt and nonce come from `getrandom::fill`. The decrypted JSON is parsed straight into `SecretString`/`Zeroizing<String>`, never a plain `String`.
  - **Writes** rewrite the whole file atomically.
  - **Activation.** `FallbackBackend` uses it only when the keyring probe says unavailable, which in practice means Linux without Secret Service. The probe runs once, on the core runtime (P36). The code is cross-platform for tests. Once the vault is chosen, `FallbackBackend::probe_available()` returns `true`, so Forever stays enabled with the vault label (Q23).
  - **Opt-in.** The first Forever save on such a machine opens "Create a vault": from the connection dialog's save (Task 21) **and** from the connect path when a prompted Forever secret is stored after success (the pipeline asks `Question::CreateVault`, Interactive only; this is the common Linux path; Q38). Cancelling keeps the secret Until restart, with the M2a toast.
  - **Forever-put errors never become hard errors (Q38).** `VaultLocked`, `VaultReadOnly` and `VaultCorrupt` on a Forever `put` fall back to `UntilRestart` with a reason (the M2a toast). `vault.create` and `vault.unlock` run on the blocking pool (Argon2).
  - **Master password.** At least 8 characters, no other rule. Unlock lasts for the app run and is locked at quit.
  - **Attempts.** 3 per unlock gesture, with no lockout.
  - **Recovery.** A forgotten password means reset: typing the word the dialog shows deletes the vault.
  - **Failure handling.**
    - A tampered or corrupt file: AEAD error, the file is never written, and reset is offered.
    - A newer `format_major`: read-only, never written.
  - **macOS and Windows** never use the vault.
- **B14. Secure event input for password fields stays in M9.** The M2a ledger ruled this: unsafe FFI and a process-wide counter, with an ADR in the M9 security pass. Every M2b secret field uses the M2a masked `TextInput`, so M9's work covers them all. **Not M2b; owner M9.**
- **B15. Keyring verification on Windows and Linux.**
  - Task 16 adds CI jobs:
    - `keyring-linux`: `dbus-run-session` plus `gnome-keyring-daemon --unlock`;
    - `keyring-windows`: Credential Manager;
    - `vault-linux`: no D-Bus, which exercises the vault path.
  - Each runs `live_keyring_round_trip`, or the vault round trip for `vault-linux`.
  - macOS stays manual at the gate, with the PO present (M2a ledger).
- **B16. Engine and Docker scope.**
  - PgBouncer (spec §7.1, "for MCP-09 pooler detection") arrives with **M8**, its consumer.
  - SQLite's WAL budget (1 writer + 2 readers, D-035) arrives with **M4**, the first reader. M2b keeps SQLite at budget 1.
  - C11's separate read-only credential belongs to the AI pool, so **M8**.
  - MariaDB runs the bypass harness in M2b and the results are recorded. It stays "read-only (guarded)" (C19). Flipping the label is a PO decision at the gate.
- **B17. Docker matrix.**
  - `docker/m2a/` and the project `hacivat-m2a` stay; a rename is M9 cleanup. Task 16 adds:
    - **`sshd-a`** (published on `127.0.0.1:2222`; on the default network and on `ssh-inner`) and **`sshd-b`** (not published; on `ssh-inner` and the default network). Both come from one `docker/m2a/sshd/Dockerfile` on a digest-pinned Alpine. Host keys and the test client key pair are generated at container start into the artifacts mount, so no private key is committed and GitHub push protection is never triggered. The test user is `hv_ssh` with password `hv_ssh_pw`.
    - **`pg17-auth`**: `pg_hba` maps user `hv_md5` to `md5` and `hv_clear` to `password`, for B8.
    - read-only roles in `docker/m2a/init`: PostgreSQL `hv_ro` (`pg_read_all_data`, `REVOKE TEMP`; it inherits, so it can read, Q21) and MySQL/MariaDB `hv_ro` (`SELECT`, `SHOW VIEW`), for the role probe. Tests also create them idempotently through the admin connection (Q7).
    - `targets.toml` `[[target]]` gains `service` (compose name) and `internal_port`, and `DockerTarget` gains `internal: (String, u16)`, the in-network endpoint used through SSH (Q6).
  - There is no toxiproxy. `hacivat-testkit::ChaosProxy`, an in-process relay that can cut, refuse and blackhole, simulates outages without touching the controller's compose project.
  - **Every `image:`** becomes `tag@sha256:<multi-arch index digest>`, and so does every `FROM`. `cargo xtask docker pin` writes the digests; `cargo xtask docker check-pins` fails on any unpinned line.
  - **Sets:** `pr` = pg17, mysql84, sshd-a, sshd-b. `full` = everything.
  - **CI:** `docker-pr` on every PR; `docker-full` nightly, including MariaDB.
- **B18. Liveness (HV-CONN-04).**
  - The supervisor ticks every 1 s per live session.
  - **Keep-alive** comes from `Profile.keepalive_s`. Without it: 60 s when the transport is SSH, otherwise off.
  - **Idle disconnect** comes from `Profile.idle_disconnect_s`. Without it, by environment: dev 4 h, test 2 h, staging 2 h, production 1 h, local or none: off.
  - Keep-alive never counts as activity.
  - With an open transaction at the threshold there is no disconnect. Instead `SessionEvent::IdleTransaction` fires once (notify only, D-014).
  - **Sleep and wake:** M2b detects clock jumps only: `Suspect` when `wall_delta − mono_delta > 5 s` **or** `mono_delta > tick + 5 s` (on Windows, `Instant` may include suspended time, Q31) ⇒ one immediate ping. On `Suspect` with the transaction Idle, the idle threshold is compared against **wall** time since the last activity: Disconnect if exceeded, else Ping (a production profile idle for 59 minutes plus a night asleep disconnects). OS sleep notifications and the App Nap opt-out need platform FFI, so they go to **M9**.
- **B19. Reconnect (HV-CONN-05, D-085).**
  - **Backoff:** 0, 1, 2, 4 and 8 s, then every 30 s up to 5 min, with ±20 % jitter from an injectable RNG. After that the state is `Lost`, with Retry.
  - An auth failure while reconnecting stops at once and shows Failed (no loop).
  - **Replay:** the environment plan, the last schema, the transaction mode and isolation, and the read-only guard.
  - **Never replayed:** user statements. A statement that died in flight returns `ConnectionLost { outcome_unknown: true }`. App-internal idempotent reads are re-run once.
  - **The toast.** Every reconnect that replaced a server session fires `ConnectionEvent::SessionLost { lost }`. The toast always shows its title and body. It adds the rolled-back line only when an open transaction was lost, and the "also gone" line only when other state was lost.
  - **What counts as lost.** M2b tracks temp objects, session variables and locks from the first keyword of each user statement (`CREATE TEMP…`, `SET`, `LOCK`, `SELECT pg_advisory_lock`/`GET_LOCK`). M3 replaces the scan with the classifier.
  - **Reconnects and side connections use the session overlay (Q17).** Each `SessionManager` keeps an in-memory overlay: the profile with every `ProfileChange` of its successful connects applied, the one-gesture plaintext consent (`ConnectPlaintext` without remember), and the `SessionSecrets` that worked. The app pushes edits with `ConnectionManager::update_profile(id, Arc<Profile>)` after every save and `apply_changes`; that replaces the overlay's profile and keeps the consent and secrets. The overlay is dropped at disconnect, `forget` and quit, and never persisted. A reconnect that still needs interaction (a one-time code, an unknown host key, a locked vault) stops with `LiveState::Lost { class }` and `needs_interaction`; the capsule's Retry starts an interactive connect. Recovery never prompts.
- **B20. One SSH death, one reconnect per dependent.**
  - `TunnelManager` publishes `TunnelEvent::Dead { key, generation }`.
  - Recovery takes a per-transport-generation lock, so each dependent profile reconnects once, even when many sessions share the tunnel.
- **B21. Read-only in M2b.**
  - **The layers:**
    - L0: input at most 1 MiB, no NUL;
    - L3: PostgreSQL extended protocol; MySQL without `CLIENT_MULTI_STATEMENTS` and `CLIENT_LOCAL_FILES`, with prepared statements only (`ER_UNSUPPORTED_PS` is denied); SQLite prepares the full text and requires `stmt_readonly`;
    - L4: the per-engine wrapper from `hacivat-guard`;
    - L5: the role probe, which drives the label;
    - L6: PostgreSQL canary, ROLLBACK check, session reset (the `DISCARD ALL` sequence without its `DEALLOCATE ALL` step, plus deallocating SQL-level prepared statements by name, Q10) and replay of the session's environment plan (Q15); MySQL ROLLBACK, `COM_RESET_CONNECTION` and the same replay; SQLite has no per-request reset.
  - **Transaction-control statements on a read-only session (Q35).** A user statement whose first keyword is `COMMIT`, `ROLLBACK`, `BEGIN`, `START`, `END`, `ABORT`, `SAVEPOINT` or `RELEASE` is denied before execution as `ReadOnlyDenied(Transaction)`. It is a first-keyword check, not a classifier; the canary and the post-check still catch smuggled cases.
  - **"read-only (enforced by server role)"** is shown only for PostgreSQL and MySQL when the probe finds a least-privilege role. MariaDB (C19) and SQLite (no server role) always show "guarded". Every other read-only connection shows "read-only (guarded)" with a remediation popover.
  - **An enforcement gap** taints the session, discards the connection and emits `ConnectionEvent::EnforcementGap`. Gap causes: a non-NULL canary with the classifier off (S-10), "no transaction in progress" on ROLLBACK, or a write command tag.
- **B22. Transaction defaults.**
  - Production defaults to Manual (D-036). Everything else defaults to Auto. `Profile.tx_mode` overrides.
  - **Read-only sessions are always Auto (Q14).** `Profile.tx_mode` and the D-036 default apply only to read-write sessions. On a read-only session, `SetMode(Manual | Smart)` and `TxCommand::Begin` are `TxError::Unsupported` (the guard owns the transaction). Otherwise SQLite's authorizer would deny Manual's `BEGIN` on every statement, and on PostgreSQL and MySQL the guard's own `BEGIN … ROLLBACK` would nest inside, or implicitly commit, the Manual transaction.
  - Manual on PostgreSQL and SQLite sends `BEGIN` before the next statement. MySQL uses `SET autocommit = 0`.
  - Switching Manual → Auto in a transaction is refused (`TxError::InProgress`).
  - Isolation changes apply at the next begin.
  - There is no auto-rollback anywhere in M2b. The 5-minute auto-rollback of a confirm-in-transaction dialog (D-014) arrives with that dialog in M3.
- **B23. Cancel (HV-SESS-05, HV-EXEC-09).**
  - Each engine cancels over the same transport as the session:
    - PostgreSQL: `CancelToken::cancel_query_raw` over a stream from the same `StreamFactory`, with the same TLS `Connector`;
    - MySQL and MariaDB: `KILL QUERY <id>` on a side connection opened through the pipeline with `Department::Cancel` (outside the budget, cached for 30 s);
    - SQLite: `InterruptHandle`.
  - **Stale-cancel guard (Q18):** `SessionHandle::cancel(seq)` and `escalate(seq, e)` name the request. A cancel is sent only while the session runs that sequence number; otherwise the answer is `AlreadyFinished`.
  - **MySQL pre-warm (Q37):** the side connection is pre-opened once a statement has run for 500 ms, so the first cancel does not pay for a TLS and auth handshake through the tunnel. It uses the session overlay's secrets (Q17). ADR 0004 records that `KILL QUERY <id>` behind ProxySQL or MaxScale may hit another backend, and that thread ids come from the handshake's 32-bit field.
  - **Timing:** up to 2 s for a terminal state, then `CancelOutcome::StillRunning`. At 10 s, `SessionEvent::SlowCancel`.
  - `Escalation::Terminate` uses `pg_terminate_backend` or `KILL CONNECTION` on a side connection. `Escalation::Deactivate` closes the socket and marks the session Broken.
  - The UI for all of this is M3 (B1).
- **B24. New error classes:** `tls.pin_changed`, `auth.insecure_method` and `ssh.host_key_revoked`. `ErrClass::ALL` goes from 48 to 51. The `FixId` set is unchanged.
- **B25. Encrypted TLS client keys (HV-SEC-02).**
  - Encrypted PKCS #8 (PBES2) keys are supported, with the passphrase in `SecretSlot::TlsKeyPassphrase`. When none is stored, an Interactive connect asks for it.
  - Legacy OpenSSL `Proc-Type: 4,ENCRYPTED` keys are refused with the new `IssueCode::TlsKeyLegacyEncrypted`.
  - The dialog shows the passphrase field again, reversing M2a T17's ruling.
- **B26. Paths (Q22).** For untrusted origins, Validate checks every TLS, SQLite and SSH key path against `ApprovedRoots` on canonical forms. (Ledger: T15 note to M2b.)
  - Both the path and **every root** are canonicalised (`std::fs::canonicalize`), so macOS `/var/folders/…` → `/private/var/…` matches on both sides.
  - A `VerbatimDisk` prefix is stripped back to `Disk` (`\\?\C:\` → `C:\`) by a small local helper (no new crate).
  - M2a's text and prefix rules (including the SQLite `\\` rule) run on the **typed** path; the containment check runs on the canonical forms.
  - A missing file (a new SQLite file) canonicalises its parent directory and appends the file name.
  - A path that canonicalises to a UNC, device or verbatim-UNC prefix is `PathRefused`, and so is a symlink that escapes the roots.
  - Paths are still shown as the user typed them.
- **B27. pgpass.** An empty password field is `None`, not `Some("")` (libpq parity; ledger, T19 follow-up). The default socket directories stay as T8 mapped them.
- **B28. Mocks.** New surfaces are mocked in `design/m2b-secure/` (Task 2) in the harman world, with the M2a mock bar. Under the overnight ruling the controller approves them, and the PO reviews the links in the morning. The PO-approved M2a surfaces 5 and 6 are not re-mocked.
- **B29. The capsule's reconnecting state does not exist in code yet.** `hacivat-ui/src/capsule.rs:119` has no such variant. Task 1 adds `CapsuleModel::Reconnecting`, and Task 22 renders it. The other M2a seams exist as named and are used:
  - `ConnectDeps.transport_wrap` and `extra_steps`;
  - `IssueCode::TransportUnavailable` (still used for Karagöz);
  - `ConnectRequest.department`, `LiveState::Lost` and `ForwarderMode`.
- **B30. Profile additions** (TOML keys in brackets):
  - `single_session: bool` [`single_session`, default false];
  - `tx_mode: Option<TxMode>` [`tx_mode`];
  - `isolation: Option<IsolationLevel>` [`isolation`];
  - `idle_disconnect_s: Option<u32>` [`idle_disconnect_s`];
  - `ssh_retention: Retention` [`ssh_retention`, default Forever];
  - `TransportSpec::SshTunnel(SshSpec)` [`transport = "ssh_tunnel"` plus an `[ssh]` table];
  - `TlsSettings.pins` [`tls.pins`] and `TlsSettings.allow_public_plaintext` [`tls.allow_public_plaintext`].

  The format stays `version = 1`, because every new key is optional. M2a builds keep these keys untouched (toml_edit preserves unknown keys).
- **B31. Things that stay out of M2b, with owners:**
  - CONN-02: M4.
  - Session resume across networks (D-085): v1.x.
  - SOCKS and HTTP proxies (HV-SEC-06): v1.x.
  - `my.cnf`: v1.x.
  - OS sleep hooks and App Nap: M9.
  - Secure event input: M9.
  - PgBouncer: M8.
  - The C11 credential: M8.
  - SQLite WAL readers: M4.
  - Smart commit, startup scripts, L1/L2 and the session UIs: M3.
  - The Docker directory rename: M9.

## Ledger items deferred to M2b: where each one lands

| Deferred item (M2a ledger) | Lands in | Notes |
|---|---|---|
| Linux master-password vault (R5) | T6 core, T21 dialogs, T16 `vault-linux` CI | B13 |
| TLS interactive flows (pin, CA import, certificate review) | T3 net, T14 pipeline, T19 UI | B5, B6 |
| Remove the interim "Encrypt without verifying" | T14 (catalog fixes, copy), T19 (fix action, string) | B6 |
| Reconnect (CONN-05) | T17 core, T22 capsule and toast | B19, B20 |
| `require_auth`-style guard on PG `R` 3/5 | T10 driver (observer), T14 policy | B8 |
| `tls-server-end-point` channel binding | T10 | B9 |
| `mysql_async` knob (refuse RSA key retrieval when insecure) and VerifyCa | T8 patches, T13 driver, T12 Validate, T19 TLS Select | B10 |
| SQLite ATTACH, VACUUM INTO, authorizer, `LIMIT_ATTACHED` | T5 decisions, T11 apply, T23 bypass suite | B21 |
| Canonicalize before the approved-roots check | T12 | B26 |
| Empty pgpass password → `None` | T6 | B27 |
| Multi-arch digest pinning of Docker images | T16 | B17 |
| Full CI Docker matrix | T16 | B17 |
| Secure event input for password fields | **not M2b**, owner M9 | B14 |
| Windows and Linux keyring verification | T16 CI jobs; T25 records | B15 |
| Encrypted TLS client key passphrase (T17 hid the field) | T3, T14, T19 | B25 |

## New UI surfaces (need mocks; Task 2)

M2a surfaces 5 (TLS certificate decision, public-host-without-TLS, unencrypted badge) and 6 (SSH settings, import menu, new-host-key prompt) are approved and are built as mocked. These **new** surfaces have no mock yet:

1. **Host key changed: a hard stop.** A 480 px dialog showing the old and new SHA256 fingerprints, where the old key is stored, and the action available: "Remove the old key and trust the new one" (Hacivat store only) or a copyable `ssh-keygen -R` command.
2. **Keyboard-interactive prompt.** The server's name and instructions verbatim, N prompts (echoed or masked) and an attempt count.
3. **SSH password and key-passphrase prompts,** per hop. A variant of the M2a password prompt, with Remember.
4. **Vault:** create (master password twice, "can't be recovered" note), unlock (attempt count), and reset (typed confirmation). The Remember Select row reads "Forever (encrypted file)" on Linux without a keyring.
5. **Certificate changed on a pinned endpoint:** a hard stop, a variant of surface 5 with both fingerprints and the explicit "Replace pin" button.
6. **Read-only label states:** "read-only (enforced by server role)" vs "read-only (guarded)", with a remediation popover (SQL snippet and Copy) in the capsule, plus the sheet row.
7. **Idle-disconnected notice** (one toast) and the **`auth.insecure_method`** and **`ssh.*`** error copy in the existing error-dialog anatomy, including "failed at hop 2 (bastion.acme.io)".

## Waves

The work is interface-first. Task 1 commits every cross-task type and signature with fail-closed stubs (F31). It also provides working minimal versions of the helpers that same-wave tasks consume:
- `FakeDriver` execution scripting (consume-once entries, Q23);
- `hacivat_guard::plan_for`, as a constant table with Task 5's exact values (pure data, fail-closed by construction; Q8), so Task 7's environment-plan test is green in W1;
- `hacivat_testkit::require_target` (Q6), so Docker tests written in W2 are set-aware before Task 16 lands;
- the app's `interaction.rs` routing and `events.rs` dispatch (awaiting `tokio::sync` channels, the Q4 ruling);
- the app's `dialog::open_edit(id, DialogFocus, ..)` seam and the `fixes.rs` wiring of `ConnectThroughSsh` and `ReviewHostKey` (Q9), so Tasks 19 and 20 never edit each other's files;
- `ConnectionsModel` storing typed SSH secrets on save, and its `known_hosts()`/`tunnels()` accessors (Q23);
- the `settle_until` test helper (Q4);
- the vendored `mysql_async` (unpatched);
- every manifest and `deny.toml`;
- the ADR 0003/0004 skeletons;
- the `check-deps` rule for `hacivat-guard`.

| Wave | Runs | Tasks (parallel agents; model) | Unblocked by |
|---|---|---|---|
| W0 | 2 | **T1** interface commit (opus; `CARGO_TARGET_DIR=<main>/target`) · **T2** mocks for new surfaces (opus) | — |
| W1 | 5 | **T3** TLS pins, review, encrypted keys (opus) · **T4** SSH config import and known hosts (opus) · **T5** `hacivat-guard` (opus) · **T6** vault, fallback backend, pgpass (opus) · **T7** session actor core: queue, tx, cancel, env, schema, reset (opus) | T1 merged |
| W2 | 5 | **T8** `mysql_async` patches (sonnet) · **T9** SSH `TunnelManager` and testkit SSH server and agent (opus; T4) · **T10** PostgreSQL driver M2b (opus) · **T11** SQLite driver M2b (opus; T5) · **T12** profiles, store, Validate (opus) | W1 merged |
| W3 | 4 | **T13** MySQL/MariaDB driver M2b (opus; T8) · **T14** pipeline: interactive TLS, plaintext question, SSH transport, secrets, insecure-auth policy, new classes (opus; T3, T6, T9, T12) · **T15** `SessionManager`, router, pools (opus; T7) · **T16** Docker matrix, digests, CI matrix and keyring jobs, `ChaosProxy` (sonnet). **T16 Step 5 runs last in W3**, after T13's Docker run (Q7) | W2 merged |
| W4 | 5 | **T17** liveness, recovery, `ConnectionManager` integration (opus; T15, T9) · **T18** guarded read-only execution and role probe (opus; T5, T10, T11, T13) · **T19** TLS UI (opus, gpui; T14) · **T20** SSH settings UI (opus, gpui; T12, T4) · **T21** SSH and vault prompts (opus, gpui; T14, T6) | W3 merged |
| W5 | 2 | **T22** live-state UI: reconnecting, session lost, idle, read-only labels, SSH path (opus, gpui; T17, T18) · **T23** conformance suite over Docker (opus; all core) | W4 merged |
| W6 | 1 | **T24** shots, cross-surface tests, DESIGN.md (opus, gpui) | W5 merged |
| W7 | 1 | **T25** M2b gate (sonnet, with controller) | everything |

W4 runs at most two gpui agents at once unless `df` shows ≥ 60 GiB free (Q1); T21 starts when T19 or T20 finishes. Every dispatch passes the disk admission rule (Global Constraints).

File ownership. "Owns" means creates or modifies; anything not listed belongs to Task 1.
- **T1:** see its Files, which list every site its new variants, fields, signatures and seams touch (Q3, Q9, Q40).
- **T2:** `design/m2b-secure/{index.html,NOTES.md}`.
- **T3:** `crates/core/hacivat-net/src/tls.rs` and `crates/core/hacivat-net/tests/{tls_pins,tls_review}.rs`.
- **T4:** `crates/core/hacivat-net/src/ssh/{config,known_hosts}.rs` and `crates/core/hacivat-net/tests/{ssh_config,known_hosts}.rs`.
- **T5:** `crates/core/hacivat-guard/src/**` (except `lib.rs` module lines) and `crates/core/hacivat-guard/tests/**`.
- **T6:**
  - `crates/core/hacivat-secrets/src/backend/{vault,fallback,mod}.rs` and `crates/core/hacivat-secrets/src/store.rs` (`backend_choice`, `vault()`, the Forever-put fallback);
  - `crates/core/hacivat-secrets/src/files/pgpass.rs` (empty password);
  - `crates/core/hacivat-secrets/tests/{vault,fallback}.rs` and `tests/pgpass.rs` (append).
- **T7:** `crates/core/hacivat-session/src/session/{actor,tx,cancel,env,schema,reset}.rs` and `crates/core/hacivat-session/tests/{session_actor,tx,cancel,env_schema}.rs`.
- **T8:** `third_party/mysql_async/src/**`, `third_party/mysql_async/PATCHES.md`, `crates/core/hacivat-driver-mysql/tests/patches.rs` and `crates/core/hacivat-driver-mysql/tests/docker_mysql.rs` (the superseded test's `#[ignore]` only).
- **T9:**
  - `crates/core/hacivat-net/src/ssh/tunnel.rs` and `crates/core/hacivat-net/tests/ssh_tunnel.rs`;
  - `crates/core/hacivat-testkit/src/{ssh_server,agent}.rs`.
- **T10:** `crates/core/hacivat-driver-pg/src/**` (new `wire.rs`, `exec.rs`, `cancel.rs`), its tests, and its ADR 0002 section.
- **T11:** `crates/core/hacivat-driver-sqlite/src/**` (new `harden.rs`, `exec.rs`), its tests, and its ADR 0002 section.
- **T12:**
  - `crates/core/hacivat-session/src/profile/{mod,store}.rs` (field bodies, the TOML store);
  - `crates/core/hacivat-session/src/pipeline/validate.rs`;
  - `crates/core/hacivat-session/tests/{profiles,validate_m2b}.rs`.
- **T13:** `crates/core/hacivat-driver-mysql/src/**` (new `exec.rs`), its tests, and its ADR 0002 section.
- **T14:**
  - `crates/core/hacivat-session/src/pipeline/{mod,steps}.rs` and new `pipeline/{ssh_step,prompts}.rs`;
  - `crates/core/hacivat-driver/src/errors.rs` and `crates/core/hacivat-driver/catalog/errors.toml`;
  - the `hacivat-core.ftl` blocks (EN, TR) and the named exception;
  - `crates/core/hacivat-session/src/test_conn.rs`;
  - `crates/core/hacivat-session/tests/{pipeline_tls,pipeline_ssh,pipeline_secrets}.rs`.
- **T15:** `crates/core/hacivat-session/src/session/{router,pool}.rs` (with the session overlay) and `crates/core/hacivat-session/tests/{router,pool}.rs`.
- **T16:**
  - `docker/m2a/**` (new `sshd/`, `pg-auth/`, init blocks, digests);
  - `crates/core/hacivat-testkit/src/{lib,chaos}.rs` (except T1's `require_target`) and `crates/core/hacivat-testkit/tests/{targets,chaos,ssh_targets}.rs`;
  - `xtask/src/docker.rs` (`pin`, `check-pins`, sets) and `xtask/src/main.rs` (one arm);
  - `.github/workflows/ci.yml` (its blocks).
- **T17:**
  - `crates/core/hacivat-session/src/session/{liveness,recovery}.rs` and `crates/core/hacivat-session/src/manager.rs`;
  - `crates/core/hacivat-session/tests/{liveness,recovery,manager_m2b}.rs`.
- **T18:** `crates/core/hacivat-session/src/session/readonly.rs` and `crates/core/hacivat-session/tests/readonly.rs`, plus `tests/docker_readonly.rs`.
- **T19:**
  - `crates/app/hacivat-app/src/connections/{cert_dialog,plaintext_prompt,fixes,error_view}.rs` and `dialog/{mod,form}.rs` (except T1's `open_edit` seam and its two `fixes.rs` lines);
  - `model.rs` (`apply_changes` only);
  - `crates/app/hacivat-app/tests/connections_tls.rs`.
- **T20:** `crates/app/hacivat-app/src/connections/dialog/ssh.rs` and `crates/app/hacivat-app/tests/connections_ssh_form.rs`.
- **T21:** `crates/app/hacivat-app/src/connections/{hostkey_prompt,kbd_prompt,secret_prompt,vault_dialog}.rs` and `crates/app/hacivat-app/tests/connections_prompts.rs`.
- **T22:**
  - `crates/app/hacivat-app/src/connections/{session_toasts,live,capsule,sheet,status,explorer}.rs` (`live.rs` keeps awaiting its `watch` channel, Q4 ruling);
  - `crates/app/hacivat-app/src/connections/boot.rs` (the named lines only: `ManagerConfig::m2b()` and the Linux `FallbackBackend`);
  - `crates/app/hacivat-app/src/commands/debug.rs` (the debug-build cancel demo commands, Q39);
  - `crates/ui/hacivat-ui/src/capsule.rs` (`Reconnecting` rendering);
  - `crates/app/hacivat-app/tests/connections_live_m2b.rs`.
- **T23:** `crates/core/hacivat-session/tests/docker_m2b_*.rs`.
- **T24:** `crates/app/hacivat-app/tests/{shots.rs (append surfaces), cross_surface_m2b.rs}` and `DESIGN.md` (a new "Secure Network and Sessions (M2b)" section).
- **T25:** `docs/benchmarks/{m2b.md,po-demo-m2b.md}` and `design/baselines/m2b/**` (after the PO signs off).

Each UI task (T19–T22) appends its own FTL block, and owns the shots fixtures of its surfaces, as in M2a P19.

---

## File Structure

```
hacivat/
  Cargo.toml                         # + russh, x509-parser, sha2, pkcs8, chacha20poly1305, argon2, getrandom, hmac, sha1;
                                     #   hacivat-guard; [patch.crates-io] mysql_async = { path = "third_party/mysql_async" };
                                     #   [workspace] exclude = ["third_party"]
  deny.toml                          # unchanged bans; comment on rsa (Global Constraints, licences and crypto)
  third_party/mysql_async/           # 0.37.1 + PATCHES.md §0–§5 (T1 copies pristine, T8 patches)
  docker/m2a/{compose.yml, targets.toml, sshd/{Dockerfile,entrypoint.sh,sshd_config}, pg-auth/{pg_hba.conf,init.sql}, init/*.sql}
  docs/decisions/{0003-secure-network.md, 0004-sessions-and-read-only-guard.md}
  design/m2b-secure/{index.html, NOTES.md}
  crates/core/
    hacivat-proto/src/lib.rs                         # + 3 ErrClass
    hacivat-guard/src/{lib, plan, post, input, sqlite, probe, label}.rs           # NEW crate (sans-I/O)
    hacivat-net/src/tls.rs                           # pins, presented chain, review, encrypted keys
    hacivat-net/src/ssh/{mod, config, known_hosts, tunnel}.rs                     # NEW
    hacivat-secrets/src/backend/{vault, fallback}.rs; files/pgpass.rs
    hacivat-driver/src/{driver, errors, fake}.rs; url/{mod, native/mod}.rs (pub carries_secret); catalog/errors.toml
    hacivat-driver-pg/src/{wire, exec, cancel}.rs    # NEW files; connect/tls/probe extended
    hacivat-driver-mysql/src/exec.rs                 # NEW; connect/tls extended
    hacivat-driver-sqlite/src/{harden, exec}.rs      # NEW; open extended
    hacivat-session/src/session/{mod, actor, tx, cancel, env, schema, reset, router, pool, liveness, recovery, readonly}.rs   # NEW
    hacivat-session/src/pipeline/{ssh_step, prompts}.rs                           # NEW; mod/steps/validate/interaction/trail extended
    hacivat-session/src/{manager, profile/{mod,store}}.rs
    hacivat-testkit/src/{ssh_server, agent, chaos}.rs                             # NEW
  crates/ui/hacivat-ui/src/capsule.rs               # + Reconnecting
  crates/app/hacivat-app/src/connections/{cert_dialog, plaintext_prompt, hostkey_prompt, kbd_prompt, secret_prompt,
                                          vault_dialog, session_toasts, events}.rs, dialog/ssh.rs           # NEW
  crates/app/hacivat-app/src/commands/debug.rs      # + debug-build cancel demo (T22, Q39)
  locales/{en,tr}/{hacivat-ui,hacivat-core}.ftl     # appended blocks
  xtask/src/{docker.rs, check_deps.rs}
  docs/benchmarks/{m2b.md, po-demo-m2b.md}
```

---
### Task 1: Interface commit (types, stubs, manifests, vendored driver, app routing)

**Build:** `CARGO_TARGET_DIR=<main>/target` (warm; the only W0 builder), `CARGO_INCREMENTAL=0` (Q1).

**Files:**
- Create:
  - `crates/core/hacivat-guard/{Cargo.toml, src/{lib,plan,post,input,sqlite,probe,label}.rs}`
  - `crates/core/hacivat-net/src/ssh/{mod,config,known_hosts,tunnel}.rs`
  - `crates/core/hacivat-secrets/src/backend/fallback.rs`
  - `crates/core/hacivat-driver-pg/src/{wire,exec,cancel}.rs`, `crates/core/hacivat-driver-mysql/src/exec.rs`, `crates/core/hacivat-driver-sqlite/src/{harden,exec}.rs`
  - `crates/core/hacivat-session/src/session/{mod,actor,tx,cancel,env,schema,reset,router,pool,liveness,recovery,readonly}.rs`, `crates/core/hacivat-session/src/pipeline/{ssh_step,prompts}.rs`
  - `crates/core/hacivat-testkit/src/{ssh_server,agent,chaos}.rs`
  - `crates/app/hacivat-app/src/connections/{cert_dialog,plaintext_prompt,hostkey_prompt,kbd_prompt,secret_prompt,vault_dialog,session_toasts,events}.rs`, `crates/app/hacivat-app/src/connections/dialog/ssh.rs`
  - `third_party/mysql_async/**`: a pristine copy of `~/.cargo/registry/src/*/mysql_async-0.37.1` (Q29).
    - Its `Cargo.toml` drops `[dev-dependencies]`, `[[test]]`, `[[bench]]`, `[[example]]` and `[profile.bench]` (cargo warns on every build about profiles in a non-root package).
    - The copy omits `Cargo.toml.orig`, `Cargo.lock`, `azure-pipelines.yml` and `tests/`.
    - `PATCHES.md` starts with §0, "vendored copy, test, bench and CI files removed", and lists §1–§5 as "Task 8".
    - **Q2 note:** T1 copies the source unmodified and makes no TLS change in `hacivat-driver-mysql`. Patch §5 (`SslOpts::with_rustls_client_config`, Task 8) is the hook through which MySQL later uses Hacivat's own `TlsParams.config`; Task 13 wires it.
  - `docs/decisions/0003-secure-network.md` and `docs/decisions/0004-sessions-and-read-only-guard.md`: Status, Context, and a Rule heading naming the per-task section format.
  - Test files: `crates/core/hacivat-session/tests/m2b_interfaces.rs`, `crates/core/hacivat-testkit/tests/require_target.rs` and `crates/app/hacivat-app/tests/m2b_routing.rs`.
- Modify, workspace and manifests:
  - `Cargo.toml` (workspace deps, `exclude = ["third_party"]`, `[patch.crates-io]`) and `deny.toml` (comments only, unless `cargo deny` needs more; then a ruling). `exclude` is essential: otherwise the path crate becomes a workspace member, and clippy `-D warnings` and the `check-deps` rules `UnknownLayer`/`MissingWorkspaceLints` hit `mysql_async` (Q29).
  - `Cargo.lock`: the `mysql_async` entry loses `source` and `checksum`; list it in the commit body (Q29).
  - the manifests of `hacivat-{net,secrets,driver,driver-sqlite,session,testkit,app}` (Step 3 table).
- Modify, core types and stubs:
  - `crates/core/hacivat-proto/src/lib.rs`;
  - `crates/core/hacivat-net/src/{lib,tls,address}.rs` (types, the new `build_tls_params` signature, `ssh_address_class`; behaviour unchanged);
  - `crates/core/hacivat-driver/src/{driver,errors,fake,lib}.rs`, `url/mod.rs` and **`url/native/mod.rs`** (`carries_secret` from `pub(super)` to `pub`, Q3);
  - `crates/core/hacivat-driver/catalog/errors.toml` (three `reachable = false` entries);
  - the three driver crates (trait stubs);
  - `crates/core/hacivat-session/src/{lib,manager}.rs` (`PathKind` already exists in `manager.rs` with `Direct`; T1 adds `Ssh` there and `LiveConnection.path` reuses it), `pipeline/{mod,interaction,trail,validate,steps}.rs` (types; PostConnect calls `session::readonly::access_for`; the `Answer` match at `steps.rs:341-342` gains a `_ =>` arm that cancels; `validate.rs::tls_issue` gains the new `TlsSetupError` arms) and `profile/mod.rs` (`ProfileDraft.ssh_secrets`);
  - `crates/core/hacivat-session/src/profile/store.rs` (minimal: parse and write `transport = "ssh_tunnel"` with an empty `SshSpec`, at `store.rs:669` and `:866`; Task 12 owns the real `[ssh]` table);
  - `crates/core/hacivat-secrets/src/{lib,store,backend/mod,backend/vault}.rs` (`SecretBackend::choice`, `SecretStore::vault`);
  - `crates/core/hacivat-testkit/src/lib.rs` (`require_target`, working; Q6);
  - `crates/ui/hacivat-ui/src/capsule.rs` (the variant, with a minimal render);
  - `xtask/src/check_deps.rs` (the `hacivat-guard` rule).
- Modify, app (minimal arms, renames, seams and fields only; Q3, Q9, Q23):
  - `crates/app/hacivat-app/src/connections/{mod,interaction,events}.rs`;
  - `connections/report.rs` (`TlsVerified::Pinned` arm at `:267-269`) and `connections/capsule.rs` (`TlsVerified::Pinned` at `:118-123`; `LiveState::Reconnecting` at `:173,240,250`);
  - `connections/{commands,status,explorer}.rs` and `connections/error_view.rs` (`LiveState::Reconnecting` arms at `commands.rs:203`, `error_view.rs:445,478`, `status.rs:62`, `explorer.rs:645`; the new `ConnectError`/`ErrArgs` fields in the `src` tests at `error_view.rs:1175,1277`);
  - `connections/password_prompt.rs` (`PasswordRequest` → `PromptRequest` at `:28,265,274`; the irrefutable `let Question::Password` at `:66` becomes a `match` that answers `Cancelled` for other variants);
  - `connections/boot.rs` (`ConnectDeps.{tunnels, known_hosts}`: `KnownHosts::load(..)`, which does no I/O, and `TunnelManager::new`, at `:142-153`; Q23, Q42);
  - `connections/fixes.rs` (the arms `ReviewCertificate`, `ImportCa`, `ConnectThroughSsh` and `ReviewHostKey`; `ConnectThroughSsh` → `dialog::ssh::open_with_ssh` and `ReviewHostKey` → `hostkey_prompt::review_changed`, one line each; Q9);
  - `connections/dialog/mod.rs` (the seam `open_edit(id, DialogFocus, window, cx)`, with a working `Default`; Q9) and `dialog/form.rs` (one hook line for the SSH section; `issue_text` arms at `:596-617` for the five new `IssueCode`s, mapped to Task 1's placeholder messages);
  - `connections/model.rs` (`ConnectionsModel::{known_hosts, tunnels}`; `save` stores `ProfileDraft.ssh_secrets` under `SecretSlot::{SshPassword, SshPassphrase} { hop }` with `ssh_retention`, about 15 lines mirroring the DB password; `save` calls `vault_dialog::ensure_vault_before_forever` (one line) and `ConnectionManager::update_profile` (one line, Q17); Q23);
  - `crates/app/hacivat-app/tests/connections_errors.rs` (`:234`, the new `ConnectError` fields);
  - `crates/app/hacivat-app/tests/support/mod.rs` (append block `// ## m2b-t01-interfaces`: `settle_until(vcx, timeout, pred)`, which loops `allow_parking`, a short real sleep, `advance_clock(16 ms)` and `run_until_parked` until `pred` holds, and panics with the predicate's name on timeout; Q4).
- Modify, existing tests that the new variants, fields and signature break (Q3):
  - `TransportSpec::SshTunnel` unit-variant uses: `crates/core/hacivat-session/tests/profiles.rs:183` and `tests/common/mod.rs:363`;
  - `Question` irrefutable `let`: `crates/core/hacivat-session/tests/common/mod.rs:547`;
  - `ConnectDeps { .. }` literals: `crates/core/hacivat-session/tests/{common,docker_support}/mod.rs` (and `pipeline/mod.rs`);
  - `DriverConnect { .. }` literals (33 in 9 files; add `insecure_auth`): `hacivat-driver/src/{driver,fake}.rs`, `hacivat-driver-sqlite/src/lib.rs`, `hacivat-driver-sqlite/tests/sqlite.rs`, `hacivat-driver-pg/tests/{unit,docker_pg}.rs`, `hacivat-driver-mysql/tests/{unit,docker_mysql}.rs`, `hacivat-session/src/pipeline/steps.rs`;
  - `build_tls_params(` with five arguments (about 40 calls): `hacivat-net/tests/tls.rs`, `hacivat-driver-pg/tests/{unit,docker_pg}.rs`, `hacivat-driver-mysql/tests/{unit,docker_mysql}.rs`, `hacivat-session/src/pipeline/{steps,validate}.rs` (`steps.rs:615`, `validate.rs:203`);
  - every `TlsSettings { … }` and `Profile { … }` literal in the workspace (add `..Default::default()`).
- Modify, locales: `locales/{en,tr}/hacivat-ui.ftl`, block `## m2b-t01-interfaces`: placeholder messages `hv-conn-issue-ssh-host-missing`, `-ssh-user-missing`, `-ssh-too-many-hops`, `-ssh-key-file-missing` and `hv-conn-issue-tls-key-legacy-encrypted`, each with `# Review: PO` (typed message ids exist only for messages in `locales/en`). Task 20 and Task 19 rewrite their text (named exception).

**Interfaces (Produces; every later task consumes these names).** Derives follow M2a Task 1:
- every data type derives `Debug, Clone, PartialEq`, plus `Eq, Hash` where the fields allow, `Copy` for fieldless enums, and `Default` where a default is named;
- **secret-holding types derive `Debug` only (redacted), never `PartialEq`**;
- stubs fail closed (Global Constraints, "No silent weakening");
- `BoxFuture` is `futures::future::BoxFuture`.

```rust
// hacivat-proto
ErrClass::{TlsPinChanged = "tls.pin_changed" in Tls, SshHostKeyRevoked = "ssh.host_key_revoked" in Ssh,
           AuthInsecureMethod = "auth.insecure_method" in Auth}            // ErrClass::ALL.len() == 51

// hacivat-net::tls (T3 implements; T1 adds the types and threads `port` and `key_passphrase` through)
pub struct CertPin { pub endpoint: String, pub sha256: [u8; 32] }          // endpoint = pin_endpoint(host, port)
pub struct TlsSettings { pub mode: TlsMode, pub ca_file: Option<PathBuf>, pub client_cert: Option<ClientCertPaths>,
                         pub pins: Vec<CertPin>, pub allow_public_plaintext: bool }
pub enum TlsVerified { Full, CaOnly, Pinned, No }
pub enum TlsFailure { /* M2a */ .., PinMismatch }
pub struct PresentedChain(pub Vec<CertificateDer<'static>>);                // leaf first
pub struct TlsParams { /* M2a fields */ .., pub presented: Arc<Mutex<Option<PresentedChain>>>,
                       pub outcome: Arc<Mutex<Option<TlsVerified>>> }      // outcome written on a successful verify
pub enum TlsSetupError { /* M2a */ .., KeyEncryptedLegacy(PathBuf), KeyPassphraseWrong(PathBuf) }
pub fn build_tls_params(settings: &TlsSettings, tls_host: &str, port: u16, class: AddressClass,
                        key_passphrase: Option<&SecretString>) -> Result<Option<TlsParams>, TlsSetupError>;
                                                                           // T3: a pin for pin_endpoint(tls_host, port) forces
                                                                           // plaintext_allowed = false (Q13). TlsParams.config is
                                                                           // also what MySQL uses, via vendored patch §5 (Q2)
pub fn pin_endpoint(tls_host: &str, port: u16) -> String;                  // "db:5432", "[::1]:5432"
pub fn cert_sha256(der: &[u8]) -> [u8; 32];
pub fn fingerprint_lines(sha256: &[u8; 32]) -> [String; 2];                // "AB:CD:…" 16 bytes per line
pub enum ChainRole { Leaf, Intermediate, Root }
pub struct ChainEntry { pub subject: String, pub role: ChainRole }
pub struct CertReview { pub endpoint: String, pub subject: String, pub issuer: String, pub not_before_unix: i64,
                        pub not_after_unix: i64, pub sha256: [u8; 32], pub chain: Vec<ChainEntry>, pub pinned: Option<[u8; 32]> }
pub fn cert_review(chain: &PresentedChain, endpoint: &str, pinned: Option<[u8; 32]>) -> Option<CertReview>;
pub fn tls_server_end_point(leaf_der: &[u8]) -> Option<Vec<u8>>;           // RFC 5929 (B9)

// hacivat-net::address (working in T1)
pub fn ssh_address_class(host: &str) -> AddressClass;   // B11: IP literal → its own class, "localhost" → Loopback, other names → Private

// hacivat-net::ssh (T4: config, known_hosts; T9: tunnel)
pub enum SshAuth { Agent, KeyFile { path: PathBuf }, Password, KeyboardInteractive }
pub struct HopSpec { pub host: String, pub port: u16, pub user: String, pub auth: SshAuth, pub identities_only: bool,
                     pub identity_files: Vec<PathBuf> }                    // imported IdentityFile values for this hop
pub enum HostKeyPolicy { #[default] Ask, AcceptNew, Strict }               // ids "ask" | "accept_new" | "strict"
pub struct SshSpec { pub hops: Vec<HopSpec>, pub host_keys: HostKeyPolicy, pub keepalive_s: Option<u32> }   // last hop = bastion
pub struct HostKey { pub algorithm: String, pub sha256: [u8; 32], pub openssh: String }
impl HostKey { pub fn fingerprint(&self) -> String }                        // "SHA256:<base64, no padding>"
pub enum KnownSource { Hacivat, OpenSsh { path: PathBuf, line: usize } }
pub enum HostKeyCheck { Known(KnownSource), Unknown, Changed { old: HostKey, source: KnownSource }, Revoked { source: KnownSource } }
pub struct KnownHosts { .. }
impl KnownHosts {
    pub fn load(openssh_files: Vec<PathBuf>, hacivat_file: PathBuf) -> Arc<KnownHosts>;   // no I/O (Q42): files are read lazily
                                                       // at the first check/trust, and an OpenSSH file is re-read when its
                                                       // (mtime, len) changed (Q24)
    pub fn check(&self, host: &str, port: u16, key: &HostKey) -> HostKeyCheck;
    pub fn trust(&self, host: &str, port: u16, key: &HostKey) -> io::Result<()>;           // Hacivat store only
    pub fn replace(&self, host: &str, port: u16, old: &HostKey, new: &HostKey) -> io::Result<()>;  // Hacivat store only
    pub fn hacivat_path(&self) -> &Path;
}
pub enum ConfigIssue { ProxyCommand, UnsupportedToken(String), SecretInValue, IncludeTooDeep, NoHostName }
pub enum ConfigNotice { MatchSkipped { line: usize }, UserKnownHostsIgnored { alias: String }, StrictNoMappedToAsk { alias: String },
                       IdentityAgentIgnored { alias: String },                            // Q43
                       IncludeTooDeep { line: usize } }                                   // Q32: the Include loop is visible
pub struct SshConfigHost { pub alias: String, pub hostname: Option<String>, pub port: Option<u16>, pub user: Option<String>,
                           pub identity_files: Vec<PathBuf>, pub identities_only: bool, pub proxy_jump: Vec<String>,
                           pub server_alive_interval: Option<u32>, pub strict_host_key_checking: Option<HostKeyPolicy>,
                           pub unusable: Option<ConfigIssue> }
pub struct SshConfig { pub hosts: Vec<SshConfigHost>, pub notices: Vec<ConfigNotice> }
pub fn read_ssh_config(path: &Path, home: &Path, local_user: &str, is_secret: &dyn Fn(&str) -> bool) -> io::Result<SshConfig>;
                                                                           // callers pass hacivat_driver::url::carries_secret
pub fn resolve_alias(cfg: &SshConfig, alias: &str) -> Result<SshSpec, ConfigIssue>;      // follows ProxyJump into hops
pub struct ChainKey(pub [u8; 32]);
pub struct TransportGeneration { pub key: ChainKey, pub generation: u64 }
pub enum TunnelEvent { Dead { key: ChainKey, generation: u64 } }
pub enum SshSecretKind { Password { host: String, user: String }, Passphrase { key: PathBuf } }
pub struct KbdPrompt { pub text: String, pub echo: bool }
pub struct KbdRequest { pub host: String, pub name: String, pub instructions: String, pub prompts: Vec<KbdPrompt> }
pub trait SshPrompter: Send + Sync + 'static {
    fn host_key(&self, hop: u8, host: &str, port: u16, key: &HostKey) -> BoxFuture<'static, bool>;
    fn secret(&self, hop: u8, kind: SshSecretKind, attempt: u8) -> BoxFuture<'static, Option<SecretString>>;
    fn keyboard(&self, hop: u8, req: KbdRequest, attempt: u8) -> BoxFuture<'static, Option<Vec<SecretString>>>;
}
pub struct HopSecrets { pub password: Option<SecretString>, pub passphrase: Option<SecretString> }   // Debug only
pub struct TunnelRequest { pub spec: SshSpec, pub secrets: Vec<HopSecrets>, pub share: bool, pub connect_timeout: Duration }
pub enum SshError { HostKeyUnknown { hop: u8, host: String, port: u16, key: HostKey },
    HostKeyChanged { hop: u8, host: String, port: u16, old: HostKey, new: HostKey, source: KnownSource },
    HostKeyRevoked { hop: u8, host: String, port: u16 }, Auth { hop: u8, user: String, host: String },
    AgentUnavailable { hop: u8 }, ChannelOpen { hop: u8, reason: String }, Unreachable { hop: u8, net: NetError },
    Timeout { hop: u8 }, Cancelled, Protocol { hop: u8 } }
pub struct TunnelConfig { pub idle_grace: Duration /* 30 s */, pub keepalive: Duration /* 30 s */, pub keepalive_max: u32 /* 3 */ }
pub struct TunnelManager { .. }
impl TunnelManager {
    pub fn new(known: Arc<KnownHosts>, config: TunnelConfig) -> Arc<TunnelManager>;
    pub fn acquire(self: &Arc<Self>, req: TunnelRequest, prompter: Option<Arc<dyn SshPrompter>>, cancel: CancellationToken)
        -> BoxFuture<'static, Result<TunnelLease, SshError>>;
    pub fn sessions_open(&self) -> usize;
    pub fn events(&self) -> broadcast::Receiver<TunnelEvent>;
}
pub struct TunnelLease { .. }   // Drop releases (reverse hop order, 30 s grace at zero users)
impl TunnelLease { pub fn factory(&self, host: &str, port: u16) -> StreamFactory; pub fn generation(&self) -> TransportGeneration;
                   pub fn bastion(&self) -> &str; pub fn hops(&self) -> u8 }

// hacivat-secrets
SecretError::{ /* M2a */ .., VaultLocked, VaultCorrupt, WrongMasterPassword, VaultReadOnly }
pub trait SecretBackend { /* M2a */ ..; fn as_vault(&self) -> Option<&dyn VaultBackend> { None }
    fn choice(&self) -> Option<BackendChoice> { None } }   // Q23: KeyringBackend → Some(Keyring) once probed; MemoryBackend →
                                                           // Some(Memory); FallbackBackend delegates to its probed choice
pub trait VaultBackend: SecretBackend { fn exists(&self) -> bool; fn is_unlocked(&self) -> bool;
    fn create(&self, master: SecretString) -> Result<(), SecretError>; fn unlock(&self, master: SecretString) -> Result<(), SecretError>;
    fn lock(&self); fn reset(&self) -> Result<(), SecretError>; }
pub struct VaultParams { pub m_kib: u32, pub t: u32, pub p: u32 }            // Default 65536, 3, 1
pub struct FileVault { .. }  impl FileVault { pub fn open(path: PathBuf) -> FileVault; pub fn with_params(path: PathBuf, p: VaultParams) -> FileVault }
pub enum BackendChoice { Keyring, Vault, Memory }
pub struct FallbackBackend { .. }
impl FallbackBackend { pub fn new(keyring: Arc<dyn SecretBackend>, vault: Arc<FileVault>, probe: Box<dyn Fn() -> bool + Send + Sync>) -> Self }
                       // SecretBackend::probe_available() is true when the vault is chosen, so Forever stays enabled with
                       // the vault label (Q23); choice() and as_vault() come through the trait
impl SecretStore { pub fn backend_choice(&self) -> Option<BackendChoice>;    // None until probed (reads SecretBackend::choice)
                   pub fn vault(&self) -> Option<&dyn VaultBackend> }        // Q23; T21 and T14 reach the vault through it

// hacivat-driver
pub enum TxStatus { #[default] Idle, InTx, Failed }
pub enum Protocol { #[default] Extended, Simple }
pub struct ExecOpts { pub max_rows: usize /* 1000 */, pub protocol: Protocol }
pub struct ExecSummary { pub command_tag: Option<String>, pub rows_affected: Option<u64>, pub columns: Vec<String>,
                         pub rows: Vec<Vec<Option<String>>>, pub row_count: u64, pub truncated: bool,
                         pub notice_codes: Vec<String>, pub tx: TxStatus }
pub enum InsecureAuth { #[default] RefuseUnlessVerified, Allow }
DriverConnect { /* M2a */ .., pub insecure_auth: InsecureAuth }
RawErrorKind::{ /* M2a */ .., InsecureAuthRefused }
pub trait RawConn { /* M2a */ ..;
    fn execute<'a>(&'a mut self, sql: &'a str, opts: ExecOpts) -> BoxFuture<'a, Result<ExecSummary, RawError>>;
    fn batch<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), RawError>>;     // trusted control text, simple protocol
    fn tx_status(&self) -> TxStatus;
    fn reset(&mut self) -> BoxFuture<'_, Result<(), RawError>>;   // PG: the DISCARD ALL sequence minus DEALLOCATE ALL (Q10);
                                                                  // MySQL: COM_RESET_CONNECTION; SQLite: no-op
    fn backend_id(&self) -> Option<u64>; }                        // PG pid (from BackendKeyData, Q11), MySQL thread id
pub fn classify_ssh(err: &SshError) -> Classified;                                       // errors.rs, T14
pub use url::carries_secret;                                                              // now pub (secret-scan invariant)
// FakeDriver (working in T1). Matching (Q23): the first *unconsumed* entry whose prefix matches is used once; an
// unmatched statement gives Ok(ExecSummary::default()) with the status unchanged; executed() records execute and batch
// text in order; cancel resolves the running statement with RawErrorKind::Cancelled unless ignores_cancel (needs tokio `sync`).
pub struct FakeExec { pub prefix: &'static str, pub result: Result<ExecSummary, RawError>, pub delay: Duration,
                      pub tx_after: Option<TxStatus>, pub panics: bool }
impl FakeDriver { pub fn exec(self, script: Vec<FakeExec>) -> Self; pub fn backend(self, id: u64) -> Self;
    pub fn presented(self, der: Vec<Vec<u8>>) -> Self;            // written into TlsParams.presented on a TLS failure
    pub fn tls_outcome(self, v: TlsVerified) -> Self; pub fn executed(&self) -> Vec<String>;
    pub fn ignores_cancel(self) -> Self; pub fn reset_fails(self) -> Self;
    pub fn consumes_factory(self) -> Self;                         // opens the Endpoint factory's stream, like the PG driver
    pub fn script_then(self, first: Vec<Result<(), RawError>>, then: Result<(), RawError>) -> Self;
    pub fn requests(&self) -> Vec<FakeRequest>;                    // FakeRequest gains `insecure_auth` and `read_only`
    pub fn cancel_calls(&self) -> usize; pub fn reset_calls(&self) -> usize; pub fn kill_connections(&self) }  // live conns fail with Io(ConnectionReset)

// hacivat-guard (T5 implements; depends on hacivat-proto only)
pub struct GuardPlan { pub session_setup: Vec<&'static str>, pub before: Vec<&'static str>, pub canary: Option<&'static str>,
                       pub after: Vec<&'static str>, pub reset: ResetKind }
pub enum ResetKind { DiscardAll, ResetConnection, None }
pub fn plan_for(kind: DbmsKind) -> Option<GuardPlan>;   // working in T1 (Q8): a constant table with exactly Task 5's values; T5 tests it
pub struct PostFacts { pub tx_after_statement: Option<TxStatusLite>, pub canary: Option<Option<String>>,
                       pub rollback_notice_codes: Vec<String>, pub command_tag: Option<String>, pub rows_affected: Option<u64> }
pub enum TxStatusLite { Idle, InTx, Failed }                                // guard cannot depend on hacivat-driver
pub enum GapReason { CanaryAssigned, TransactionEndedByStatement, WriteSucceeded }
pub enum PostCheck { Clean, Gap(GapReason) }
pub fn check_post(kind: DbmsKind, facts: &PostFacts) -> PostCheck;
pub const MAX_STATEMENT_BYTES: usize = 1 << 20;
pub enum InputViolation { TooLong { limit: usize }, Nul }
pub fn check_input(sql: &str) -> Result<(), InputViolation>;
pub struct SqliteHardening { pub query_only: bool, pub defensive: bool, pub attach_create: bool, pub attach_write: bool,
                             pub limit_attached: i32, pub limit_length: i32, pub limit_sql_length: i32, pub limit_variable_number: i32 }
pub fn sqlite_hardening() -> SqliteHardening;
pub enum SqliteAction<'a> { Read { table: &'a str, column: &'a str }, Select, Function { name: &'a str },
    Pragma { name: &'a str, arg: Option<&'a str> }, Attach { file: &'a str }, Detach, Insert { table: &'a str },
    Update { table: &'a str }, Delete { table: &'a str }, Create, Drop, Alter, Transaction, Savepoint, Analyze, Recursive, Other(i32) }
pub enum Authorization { Allow, Deny, Ignore }
pub fn sqlite_authorize(a: &SqliteAction<'_>) -> Authorization;
pub fn role_probe_sql(kind: DbmsKind) -> &'static [&'static str];
pub enum RoleWarning { Superuser, BypassRls, Replication, CreateRole, ServerFileRoles, ExecuteServerProgram, SignalBackend,
                       WritePrivileges, TempPrivilege, EgressExtension(String), RoleGrantNotExpanded, Unparsed }
pub struct RoleFindings { pub least_privilege: bool, pub warnings: Vec<RoleWarning> }
pub fn interpret_probe(kind: DbmsKind, results: &[Vec<Vec<Option<String>>>]) -> RoleFindings;   // one row set per probe statement
pub fn access_label(kind: DbmsKind, read_only: bool, findings: Option<&RoleFindings>) -> AccessLabel;
pub fn remediation(kind: DbmsKind) -> Option<&'static str>;                // SQL with <role>/<database> placeholders

// hacivat-session::profile (T12 store)
Profile { /* M2a */ .., pub single_session: bool, pub tx_mode: Option<TxMode>, pub isolation: Option<IsolationLevel>,
          pub idle_disconnect_s: Option<u32>, pub ssh_retention: Retention, pub transport: TransportSpec }
pub enum TransportSpec { Direct, SshTunnel(SshSpec), Karagoz { org: String, agent_id: String, resource_id: String } }
ProfileDraft { /* M2a */ .., pub ssh_secrets: Vec<HopSecrets> }              // Q23; Debug only; ConnectionsModel::save stores them

// hacivat-session::pipeline
pub enum SecretPrompt { SshPassword { hop: u8, host: String, user: String }, SshPassphrase { hop: u8, key: PathBuf },
                        TlsKeyPassphrase { key: PathBuf } }
pub enum Question { Password { .. } /* M2a */, Certificate { profile: ProfileId, name: String, review: CertReview },
    PlaintextPublic { profile: ProfileId, name: String, host: String, ip: Option<IpAddr> },
    HostKey { profile: ProfileId, name: String, hop: u8, host: String, port: u16, key: HostKey },
    Secret { profile: ProfileId, name: String, prompt: SecretPrompt, attempt: u8 },
    Keyboard { profile: ProfileId, name: String, hop: u8, request: KbdRequest, attempt: u8 },
    UnlockVault { attempt: u8 }, CreateVault }
pub enum Answer { Password(SecretString), Cancelled /* M2a */, Pin, ImportCa(PathBuf), ConnectPlaintext { remember: bool },
    UseSsh, TrustHostKey, Secret(SecretString), Keyboard(Vec<SecretString>), Master(SecretString) }   // Debug only
pub enum ProfileChange { AddPin(CertPin), ReplacePin { old: [u8; 32], new: CertPin }, SetCaFile(PathBuf), AllowPublicPlaintext }
                                                       // ReplacePin is built only by cert_dialog::review (Q26); the pipeline never
pub struct SessionSecrets { pub db: Option<SecretString>, pub hops: Vec<HopSecrets>, pub tls_key_passphrase: Option<SecretString> }
                                                       // Debug only (Q17): the secrets that worked, kept in the session overlay
pub struct HostKeyMismatch { pub hop: u8, pub host: String, pub port: u16, pub old: HostKey, pub new: HostKey, pub source: KnownSource }
ConnectError { /* M2a */ .., pub certificate: Option<CertReview>, pub host_key: Option<HostKeyMismatch>,
               pub profile_changes: Vec<ProfileChange> }
ErrArgs { /* M2a */ .., pub hop: Option<u8>, pub hop_host: Option<String> }
ConnectRequest { /* M2a */ .., pub read_only: bool,                         // profile.read_only || session read-only
                 pub draft_ssh: Vec<HopSecrets>,                             // typed in the dialog, for Test (Q23)
                 pub overlay_secrets: Option<SessionSecrets>,                // reconnects and side connections (Q17); used before the store
                 pub plaintext_consent: bool }                               // the overlay's one-gesture consent (Q17)
ConnectDeps { /* M2a */ .., pub tunnels: Arc<TunnelManager>, pub known_hosts: Arc<KnownHosts> }
LiveConnection { /* M2a */ .., pub profile_changes: Vec<ProfileChange>, pub transport: Option<TransportGeneration>,
                 pub path: PathKind, pub via: Option<String>, pub access_warnings: Vec<RoleWarning>,
                 pub secrets_used: SessionSecrets, pub plaintext_consent: bool }   // Q17
pub fn insecure_auth_for(profile: &Profile, class: AddressClass, mode: TlsMode) -> InsecureAuth;   // pub (Q23); T14 implements
IssueCode::{ /* M2a */ .., SshHostMissing, SshUserMissing, SshTooManyHops, SshKeyFileMissing, TlsKeyLegacyEncrypted }
TrailStep::Ssh                                                               // id "ssh"

// hacivat-session::session
pub struct SessionId(pub u64);  pub struct ClientId(pub u64);  impl ClientId { pub const DEFAULT: ClientId = ClientId(0); }
pub enum JobKind { Export, Import, DdlApply, DriftCheck }
pub enum SessionKind { Console(ClientId), Editor(String), Introspection { database: Option<String> }, Service, Job(u64, JobKind) }
pub struct SessionSpec { pub kind: SessionKind, pub read_only: bool, pub department: Department }
pub enum SessionState { #[default] Disconnected, Connecting, Idle, Busy { req: u64, since: Instant },
                        Cancelling { req: u64, since: Instant }, Broken(ErrClass) }
pub enum TxMode { #[default] Auto, Manual, Smart }               // ids "auto" | "manual" | "smart"
pub enum IsolationLevel { #[default] Default, ReadUncommitted, ReadCommitted, RepeatableRead, Serializable }
pub struct TxState { pub mode: TxMode, pub isolation: IsolationLevel, pub status: TxStatus, pub since: Option<Instant>, pub statements: u32 }   // Default
pub enum TxCommand { Begin, Commit, Rollback, SetMode(TxMode), SetIsolation(IsolationLevel) }
pub enum TxError { InProgress, NeedsClassifier, Unsupported, FailedNeedsRollback }
pub struct SchemaCtx { pub database: Option<String>, pub schema: Option<String>, pub search_path: Vec<String>, pub user: Option<String> }   // Default
pub enum CancelState { #[default] Idle, Cancelling { since: Instant }, SlowCancel { since: Instant }, Terminating, Deactivating }
pub enum CancelOutcome { Cancelled, AlreadyFinished, StillRunning, Disconnected, Failed(ErrClass) }
pub enum Escalation { Terminate, Deactivate }
pub struct SessionSnapshot { pub state: SessionState, pub tx: TxState, pub schema: Option<SchemaCtx>, pub cancel: CancelState,
                             pub read_only: bool, pub access: AccessLabel, pub generation: u64 }
pub enum ExecOrigin { User, Internal { idempotent: bool } }
pub enum Request { Exec { sql: String, opts: ExecOpts, origin: ExecOrigin }, Tx(TxCommand), SwitchSchema(String), Ping, Reset }
pub enum DenyLayer { Input, Protocol, Transaction, Engine }
pub enum ExecErrorKind { Db, ConnectionLost { outcome_unknown: bool }, Cancelled, ReadOnlyDenied(DenyLayer),
                         Tx(TxError), EnforcementGap(GapReason), NotConnected, Internal }
pub struct ExecError { pub kind: ExecErrorKind, pub server: Option<ServerMessage> }
pub type Reply = Result<ExecSummary, ExecError>;
pub struct RequestTicket { pub seq: u64, pub reply: oneshot::Receiver<Reply> }
impl RequestTicket { pub fn cancel(&self, h: &SessionHandle) -> oneshot::Receiver<CancelOutcome> }   // h.cancel(self.seq) (Q18)
pub enum ClosePolicy { RefuseIfOpenTx, RollBack }
pub enum CloseRefused { OpenTransaction }
pub struct LostState { pub open_tx: bool, pub temp_objects: bool, pub session_vars: bool, pub prepared: u32, pub locks: bool }   // Default
impl LostState { pub fn is_empty(&self) -> bool }
pub enum SessionEvent { SlowCancel { req: u64 }, IdleTransaction { minutes: u32 }, EnforcementGap(GapReason), Lost(LostState),
                        Reconnected, SchemaChanged(SchemaCtx), TxChanged(TxState) }
pub struct SessionHandle { .. }   // Clone
impl SessionHandle { pub fn id(&self) -> SessionId; pub fn kind(&self) -> &SessionKind;
    pub fn submit(&self, req: Request) -> RequestTicket;
    pub fn cancel(&self, seq: u64) -> oneshot::Receiver<CancelOutcome>;                  // Q18: AlreadyFinished unless `seq` is running
    pub fn escalate(&self, seq: u64, e: Escalation) -> oneshot::Receiver<CancelOutcome>;
    pub fn state(&self) -> watch::Receiver<SessionSnapshot>; pub fn events(&self) -> broadcast::Receiver<SessionEvent>;
    pub fn close(&self, p: ClosePolicy) -> BoxFuture<'static, Result<(), CloseRefused>>; }
// session::actor (T7)
pub fn spawn_session(rt: &Handle, spec: SessionSpec, profile: Arc<Profile>, live: LiveConnection, hooks: SessionHooks) -> SessionHandle;
pub struct SessionHooks { pub side: Arc<dyn SideConnector>, pub reconnect: Option<Arc<dyn Reconnector>> }   // T17 supplies `reconnect`
pub trait SideConnector: Send + Sync { fn open_side(&self) -> BoxFuture<'static, Result<LiveConnection, ConnectError>>; }  // Department::Cancel;
                                                                           // T15's implementation connects with the session overlay (Q17)
pub trait Reconnector: Send + Sync { fn reconnect(&self, attempt: u32) -> BoxFuture<'static, Result<LiveConnection, ConnectError>>; }
// session::tx, env, schema, reset (T7)
pub fn default_mode(env: Option<EnvTag>) -> TxMode;                           // Production → Manual
pub fn begin_sql(kind: DbmsKind, iso: IsolationLevel, read_only: bool) -> Vec<String>;
pub struct EnvPlan { pub steps: Vec<EnvStep> }
pub enum EnvStep { ClientIdentity(String), ReadOnly(Vec<String>), TimeZone(String), RestoreSchema(String), StartupScript /* M3 */ }
impl EnvPlan { pub fn statements(&self) -> Vec<String> }
pub fn build_env_plan(kind: DbmsKind, profile: &Profile, read_only: bool, last_schema: Option<&SchemaCtx>) -> EnvPlan;
pub fn schema_probe_sql(kind: DbmsKind) -> &'static str;
pub fn parse_schema_probe(kind: DbmsKind, s: &ExecSummary) -> SchemaCtx;
pub fn switch_schema_sql(kind: DbmsKind, name: &str) -> String;
pub fn needs_schema_reprobe(sql: &str) -> bool;
// session::router, pool (T15)
pub enum RoutingMode { Multiple, Single }
pub struct RouteRequest { pub client: Option<ClientId>, pub department: Department, pub needs_tx_context: bool, pub isolated: bool }
pub enum RouteTarget { Default, Client(ClientId), Pooled, NewJob, Mcp(ClientId), NewConsole }
pub fn route(mode: RoutingMode, req: &RouteRequest, target_busy: bool) -> RouteTarget;
pub struct SessionManager { .. }
impl SessionManager { pub fn new(rt: Handle, profile: Arc<Profile>, deps: Arc<ConnectDeps>, budget: Arc<Budget>) -> Arc<SessionManager>;
    pub fn open(self: &Arc<Self>, spec: SessionSpec, ix: Interaction) -> BoxFuture<'static, Result<SessionHandle, ConnectError>>;
    pub fn pooled(self: &Arc<Self>, kind: SessionKind) -> BoxFuture<'static, Result<PooledSession, ConnectError>>;
    pub fn default_session(&self) -> Option<SessionHandle>; pub fn open_count(&self) -> usize;
    pub fn close_all(&self) -> BoxFuture<'static, ()>;
    pub fn update_profile(&self, profile: Arc<Profile>);   // Q17: replaces the overlay's profile, keeps consent and secrets
    pub fn overlay(&self) -> Option<SessionOverlay>; }
pub struct SessionOverlay { pub profile: Arc<Profile>, pub plaintext_consent: bool, pub secrets: SessionSecrets }   // Debug only (Q17);
                                                       // never persisted; dropped at disconnect, forget and quit
pub struct PooledSession { .. }   // Deref<Target = SessionHandle>; Drop → reset → idle pool (60 s idle close);
                                  // a borrow of the default session (Single mode, SQLite) never resets it (Q19)
// session::liveness, recovery (T17)
pub struct LivenessPolicy { pub keepalive: Option<Duration>, pub idle_disconnect: Option<Duration>, pub tick: Duration }
impl LivenessPolicy { pub fn for_profile(p: &Profile) -> LivenessPolicy }
pub enum LivenessAction { Ping, Disconnect, IdleTransaction, Suspect }
pub struct Supervisor { .. }
impl Supervisor { pub fn new(p: LivenessPolicy, now: Instant, wall: SystemTime) -> Self;
    pub fn tick(&mut self, now: Instant, wall: SystemTime, busy: bool, tx: TxStatus) -> Option<LivenessAction>;
    pub fn note_activity(&mut self, now: Instant); pub fn note_ping(&mut self, now: Instant); }
pub struct RecoveryPolicy { pub schedule: Vec<Duration>, pub steady: Duration, pub give_up_after: Duration, pub jitter: f32 }
impl RecoveryPolicy { pub fn delay(&self, attempt: u32, elapsed: Duration, rng: &mut dyn FnMut() -> f32) -> Option<Duration> }
pub fn is_recovery_case(class: ErrClass) -> bool;
pub struct Tracked { pub tx: TxStatus, pub temp_created: bool, pub set_seen: bool, pub prepared: u32, pub locks_seen: bool }   // Default
pub fn track_statement(t: &mut Tracked, sql: &str);                       // first-keyword scan (B19)
pub fn lost_state(t: &Tracked) -> LostState;
// session::readonly (T18)
pub fn run_guarded<'a>(conn: &'a mut dyn RawConn, kind: DbmsKind, sql: &'a str, opts: ExecOpts, replay: &'a [String])
    -> BoxFuture<'a, Result<ExecSummary, ExecError>>;   // Q15: `replay` = the session's EnvPlan::statements() (minus PG client
                                                        // identity), run after the reset; the actor passes its current plan
pub fn access_for<'a>(conn: &'a mut dyn RawConn, kind: DbmsKind, read_only: bool) -> BoxFuture<'a, (AccessLabel, Vec<RoleWarning>)>;
//   T1 stubs: run_guarded → Err(Internal); access_for → (ReadOnlyGuarded | ReadWrite, []) as M2a R10

// hacivat-session::manager (T17)
pub enum PathKind { Direct, Ssh }                     // exists in manager.rs with Direct; T1 adds Ssh (Q3)
LiveInfo { /* M2a */ .., pub via: Option<String>, pub hops: u8, pub access_warnings: Vec<RoleWarning> }
LiveState::{ /* M2a */ .., Reconnecting { attempt: u32, next_in: Option<Duration> } }
pub enum ConnectionEvent { SessionLost { profile: ProfileId, lost: LostState }, IdleDisconnected { profile: ProfileId },
    IdleTransaction { profile: ProfileId, minutes: u32 }, EnforcementGap { profile: ProfileId, reason: GapReason },   // reason: Q35 copy
    ProfileChanges { profile: ProfileId, changes: Vec<ProfileChange> } }
impl ConnectionManager { pub fn events(&self) -> broadcast::Receiver<ConnectionEvent>;
                         pub fn sessions(&self, id: ProfileId) -> Option<Arc<SessionManager>>;
                         pub fn update_profile(&self, id: ProfileId, profile: Arc<Profile>); }   // Q17; the app calls it after
                                                                           // every save and apply_changes
pub use hacivat_guard::{RoleWarning, GapReason, remediation};               // hacivat-session lib.rs re-export (Q5): no app → guard edge
impl ManagerConfig { pub fn m2b() -> ManagerConfig }   // liveness from LivenessPolicy::for_profile (B18); Default stays M2a's 30 s ping

// hacivat-testkit (T9: ssh, T16: chaos)
pub fn require_target(id: &str) -> Option<DockerTarget>;   // working in T1 (Q6): None + eprintln!("skip {id}: not in set {set}")
                                                           // outside the active set; panics when in the set but unhealthy
pub fn require_target_in_set(set: Option<&str>, id: &str) -> Option<DockerTarget>;   // the pure core, like targets_in_set
DockerTarget { /* M2a */ .., pub internal: (String, u16) }  // T16 (Q6): compose service and internal port, for paths through SSH
#[cfg(feature = "ssh")] pub mod ssh_server;  // Q23, T9 implements:
//   TestSshServer::start(TestSshConfig) -> TestSshServer { pub addr: SocketAddr, pub host_key: HostKey }
//   impl TestSshServer { host() -> String; auth_attempts() -> usize; sessions_open() -> usize; sessions_accepted() -> usize;
//                        channels_opened() -> usize; kill_sessions(); stop();
//                        open_server_channel() -> BoxFuture<Result<(), _>>; last_originator() -> Option<(String, u32)> }   // Q46 probes
//   TestSshConfig builders: user_pw(name, pw), user_key(name, PublicKey), user_kbd(name, &[(prompt, echo, answer)]),
//                           forward(ForwardPolicy::{Allow, Deny}), forward_to(ForwardTarget::{Any, Redirect(SocketAddr)}),
//                           max_auth_tries(u32), slow_banner(Duration), ipv6()
//   ForwardTarget::Redirect sends every direct-tcpip to one address (for unresolvable hosts such as `db.internal`)
#[cfg(feature = "ssh")] pub mod agent;       // TestAgent::start(keys) -> TestAgent { socket: PathBuf } (#[cfg(unix)])
pub mod chaos;                               // ChaosProxy::start(upstream) -> ChaosProxy { addr }; cut(d), blackhole(bool), restore()

// hacivat-ui
CapsuleModel::Reconnecting { env: EnvKind, env_label: SharedString, name: SharedString, engine: SharedString,
                             label: SharedString, next: Option<SharedString> }

// hacivat-app::connections (stubs answer Cancelled / do nothing; owners fill them)
pub(crate) fn cert_dialog::open(q: Question, reply: oneshot::Sender<Answer>, window: &mut Window, cx: &mut App);   // T19
pub(crate) fn cert_dialog::review(err: Arc<ConnectError>, window: &mut Window, cx: &mut App);                    // T19 (fix); builds
                                       // ProfileChange::ReplacePin itself on "Replace pin", applies it, then reconnects (Q26)
pub(crate) fn plaintext_prompt::open(q, reply, window, cx);                                                       // T19
pub(crate) fn hostkey_prompt::open(q, reply, window, cx); pub(crate) fn hostkey_prompt::review_changed(m: HostKeyMismatch, profile: ProfileId, window, cx);  // T21
pub(crate) fn kbd_prompt::open(q, reply, window, cx); pub(crate) fn secret_prompt::open(q, reply, window, cx);  // T21
pub(crate) fn vault_dialog::open(q, reply, window, cx); pub(crate) fn vault_dialog::forever_label(cx: &App) -> MsgRef<NoArgs>;  // T21
pub(crate) enum VaultReady { NotNeeded, Ready, Declined }
pub(crate) fn vault_dialog::ensure_vault_before_forever(cx: &mut App) -> Task<VaultReady>;   // T21; T1 stub → NotNeeded;
                                                                           // ConnectionsModel::save calls it before a Forever put (one T1 line)
pub(crate) fn session_toasts::{lost(profile, lost, cx), idle(profile, cx), idle_tx(profile, minutes, cx), gap(profile, reason, cx)};  // T22
pub(crate) fn dialog::ssh::section(draft: &ProfileDraft, window, cx) -> AnyElement; pub(crate) fn dialog::ssh::open_with_ssh(id: ProfileId, window, cx);  // T20
pub(crate) enum DialogFocus { Default, Ssh }                                                                     // Q9
pub(crate) fn dialog::open_edit(id: ProfileId, focus: DialogFocus, window: &mut Window, cx: &mut App);          // T1, working for
                                       // Default; Ssh opens the SSH disclosure with the checkbox on and focuses the SSH host (T20)
// fixes.rs (T1, one line each): FixAction::ConnectThroughSsh → dialog::ssh::open_with_ssh; ReviewHostKey → hostkey_prompt::review_changed
impl ConnectionsModel { pub fn apply_changes(cx: &mut App, id: ProfileId, changes: Vec<ProfileChange>) -> Task<Result<(), SaveError>>;  // T19
                        pub fn known_hosts(&self) -> Arc<KnownHosts>; pub fn tunnels(&self) -> Arc<TunnelManager>; }   // T1 (Q23)
// interaction.rs (working): PromptRequest { question, reply } replaces PasswordRequest; every Question variant → its module's `open`.
// events.rs (working): a core-side relay of ConnectionManager::events() into a tokio::sync::mpsc, awaited by a foreground task
//   (the AppUiPort pattern; allowed by the Q4 ruling); every ConnectionEvent → its handler; RecvError::Lagged re-reads state (Q42).
```

- [ ] **Step 1: Write the failing tests.**
```rust
// crates/core/hacivat-session/tests/m2b_interfaces.rs
#[test] fn three_new_error_classes() { assert_eq!(ErrClass::ALL.len(), 51);
    for (c, id) in [(ErrClass::TlsPinChanged, "tls.pin_changed"), (ErrClass::AuthInsecureMethod, "auth.insecure_method"),
                    (ErrClass::SshHostKeyRevoked, "ssh.host_key_revoked")] { assert_eq!(c.id(), id); assert_eq!(ErrClass::from_id(id), Some(c)); } }
#[test] fn debug_redaction_m2b() {   // Answer, HopSecrets, TunnelRequest, ProfileDraft.ssh_secrets, SessionSecrets, FileVault (Q32)
    let hop = || HopSecrets { password: Some(SecretString::from("s3cret")), passphrase: Some(SecretString::from("s3cret")) };
    for s in [format!("{:?}", Answer::Secret(SecretString::from("s3cret"))), format!("{:?}", Answer::Master(SecretString::from("s3cret"))),
              format!("{:?}", Answer::Keyboard(vec![SecretString::from("s3cret")])), format!("{:?}", hop()),
              format!("{:?}", TunnelRequest { spec: SshSpec::default(), secrets: vec![hop()], share: true, connect_timeout: Duration::ZERO }),
              format!("{:?}", ProfileDraft { ssh_secrets: vec![hop()], ..Default::default() }),
              format!("{:?}", SessionSecrets { db: Some(SecretString::from("s3cret")), hops: vec![hop()], tls_key_passphrase: None }),
              format!("{:?}", FileVault::open(PathBuf::from("/nonexistent/vault.bin")))] {
        assert!(!s.contains("s3cret"), "{s}"); } }
#[test] fn profile_m2b_defaults() { let p = Profile::default(); assert!(!p.single_session); assert_eq!(p.tx_mode, None);
    assert_eq!(p.ssh_retention, Retention::Forever); assert!(p.tls.pins.is_empty() && !p.tls.allow_public_plaintext);
    assert_eq!(HostKeyPolicy::default(), HostKeyPolicy::Ask); assert_eq!(InsecureAuth::default(), InsecureAuth::RefuseUnlessVerified); }
#[tokio::test] async fn fake_driver_scripts_exec_and_tx() { let f = FakeDriver::new(DbmsKind::Postgres).exec(vec![
        FakeExec { prefix: "BEGIN", result: Ok(ExecSummary::default()), delay: Duration::ZERO, tx_after: Some(TxStatus::InTx), panics: false }]);
    let mut c = connect_fake(&f).await; c.execute("BEGIN", ExecOpts::default()).await.unwrap(); assert_eq!(c.tx_status(), TxStatus::InTx);
    c.execute("select 1", ExecOpts::default()).await.unwrap(); assert_eq!(f.executed(), ["BEGIN", "select 1"]); }
#[tokio::test] async fn fake_exec_entries_are_consumed_once() { let f = FakeDriver::new(DbmsKind::Postgres).exec(vec![
        FakeExec { prefix: "SELECT x", result: Err(raw(RawErrorKind::Io)), delay: Duration::ZERO, tx_after: None, panics: false }]);
    let mut c = connect_fake(&f).await; assert!(c.execute("SELECT x", ExecOpts::default()).await.is_err());
    assert!(c.execute("SELECT x", ExecOpts::default()).await.is_ok()); assert_eq!(c.tx_status(), TxStatus::Idle); }   // unmatched → Ok, status unchanged
#[test] fn ssh_address_class_rule() { assert_eq!(ssh_address_class("db.internal"), AddressClass::Private);
    assert_eq!(ssh_address_class("localhost"), AddressClass::Loopback); assert_eq!(ssh_address_class("8.8.8.8"), AddressClass::Public); }
#[tokio::test] async fn read_only_stub_fails_closed() { let f = FakeDriver::new(DbmsKind::Postgres); let mut c = connect_fake(&f).await;
    assert_eq!(run_guarded(&mut *c, DbmsKind::Postgres, "select 1", ExecOpts::default(), &[]).await.unwrap_err().kind, ExecErrorKind::Internal); }
#[test] fn mysql_async_is_the_vendored_copy() { let md = cargo_metadata(); let p = md.packages.iter().find(|p| p.name == "mysql_async").unwrap();
    assert!(p.manifest_path.as_str().contains("third_party/mysql_async"), "{}", p.manifest_path); }
// xtask/src/check_deps.rs (append to its tests; uses the existing fx_at / rule_of helpers, Q23)
#[test] fn guard_with_tokio_is_flagged() { let md = fx_at(&[("hacivat-guard", "crates/core", &["tokio"])]);
    assert_eq!(rule_of(&md, "hacivat-guard"), Rule::GuardNotRuntimeFree); }
//   plus: the real-metadata test `new_core_crates_are_core_layer_and_clean` gains `hacivat-guard` (Core layer, no violation)
// crates/core/hacivat-testkit/tests/require_target.rs (no Docker)
#[test] fn require_target_is_none_outside_the_active_set() { assert!(require_target_in_set(Some("pr"), "mariadb114").is_none());
    assert!(require_target_in_set(None, "no-such-id").is_none()); }   // pure variant, like targets_in_set (no env mutation: no unsafe)
// crates/app/hacivat-app/tests/m2b_routing.rs — the shell helper calls allow_parking(); waits are settle_until (Q4 ruling)
#[gpui::test] fn every_new_question_routes_and_stub_answers_cancelled(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx);
    for q in all_m2b_questions() { let a = ask_through_port(&vcx, q); settle_until(&vcx, secs(2), || a.get().is_some());
        assert!(matches!(a.get(), Some(Answer::Cancelled))); } }
#[gpui::test] fn connection_events_reach_their_handlers(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx);
    emit_all_connection_events(&vcx); settle_until(&vcx, secs(2), || handler_hits(&vcx) == 5); assert_eq!(handler_hits(&vcx), 5); }
#[gpui::test] fn connect_through_ssh_fix_opens_the_edit_dialog(cx: &mut TestAppContext) { let vcx = with_failed(cx, dns_error());   // Q9 seam
    click_fix(&vcx, FixAction::ConnectThroughSsh); settle_until(&vcx, secs(2), || layer_open(&vcx, "connection-dialog")); }
#[gpui::test] fn save_stores_typed_ssh_secrets_by_hop(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx);   // Q23
    save_draft(&vcx, ssh_draft_with_secrets("pw0", "pp0")); settle_until(&vcx, secs(2), || stored(&vcx, SecretSlot::SshPassword { hop: 0 }).is_some());
    assert_eq!(stored(&vcx, SecretSlot::SshPassphrase { hop: 0 }).as_deref(), Some("pp0")); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --test m2b_interfaces`, then `cargo test -p xtask check_deps`, then `cargo test -p hacivat-testkit --test require_target`, then `cargo test -p hacivat-app --test m2b_routing`. Expected: FAIL, because the types are missing.
- [ ] **Step 3: Implement the interfaces, stubs and manifests.** This is the manifest table, and it is exhaustive:

  | Manifest | Adds |
  |---|---|
  | workspace `Cargo.toml` | `russh = { version = "<current>", default-features = false, features = ["ring", "rsa", "flate2"] }` (Q28) · `x509-parser` · `sha2` · `pkcs8` (`encryption`, `pkcs5`, `pem`, `std`, `sha1-insecure`; Q5, Q36) · `chacha20poly1305` · `argon2` · `getrandom` (the version already in the lock that offers `getrandom::fill`; Q30) · `secrecy` adds the `serde` feature (Q30) · `hmac` · `sha1` · `tracing-log` (already in the lock; `capture_tracing`'s `log` bridge, Q33) · `hacivat-guard` path · `[workspace] exclude = ["third_party"]` · `[patch.crates-io] mysql_async = { path = "third_party/mysql_async" }` |
  | `hacivat-guard` | `hacivat-proto` only. `[lints] workspace = true` |
  | `hacivat-net` | `russh`, `sha2`, `x509-parser`, `pkcs8`, `hmac`, `sha1`, `base64`, `secrecy`, `tokio-util`, `tokio` (`sync`, `time`, `net`, `io-util`, `rt`). Dev: `hacivat-testkit = { workspace = true, features = ["ssh"] }`, `tokio` (`test-util`, `macros`, `rt-multi-thread`; Q5), `rcgen`, `tempfile`, `tracing-log` |
  | `hacivat-secrets` | `chacha20poly1305`, `argon2`, `getrandom`, `serde_json`, `secrecy` (`serde`). Dev: `tempfile` |
  | `hacivat-driver` | tokio adds `sync` (FakeDriver cancel uses `tokio::sync::Notify`; Q5) |
  | `hacivat-driver-sqlite` | `hacivat-guard`; `rusqlite` adds `hooks`, `limits`, `modern_sqlite` (crate-local feature list) |
  | `hacivat-driver-mysql` | Dev: `tokio-rustls`, `rcgen` (Task 8's in-process greeting server; Q2) |
  | `hacivat-session` | `hacivat-guard`, `getrandom` (recovery jitter). Dev: `hacivat-testkit` with `features = ["ssh"]` (Q5), `tracing-log`. `lib.rs`: `pub use hacivat_guard::{RoleWarning, GapReason, remediation};` (no new app → guard edge) |
  | `hacivat-testkit` | `[features] ssh = ["dep:russh", "dep:hacivat-net"]`; `russh` (optional); `hacivat-net` (optional; `TestSshServer.host_key` is a `hacivat-net` type); `tokio` (`rt`, `net`, `io-util`, `sync`, `time`, `macros`) (Q5) |
  | `hacivat-app` | Dev: `hacivat-testkit` with `features = ["ssh"]` (Q5) |

  Without the `ssh` feature on each test crate's testkit dev-dependency, `cargo test -p hacivat-session` (each task's own command) would not compile files that `cargo test --workspace` compiles through feature unification (the M2a "passes in one, fails in the other" lesson).

  Then:
  - Run `cargo tree -i aws-lc-rs --workspace`. Expected: "nothing to print". If russh cannot build without it, stop and report BLOCKED with the feature graph.
  - Record in ADR 0003's T1 section: `cargo tree -e features -i russh`, `cargo tree -i ring`, `cargo tree -i rsa` and `cargo tree -i pageant --target x86_64-pc-windows-msvc` (Q28).
  - The `check-deps` rule `GuardNotRuntimeFree`: `hacivat-guard`'s normal-dependency closure may contain no `tokio`, `futures`, `salsa`, `hacivat-l10n`, `sql-sema` or `gpui` (§5.1 rule 3).
  - `plan_for` ships **working** (Q8): the constant table with Task 5's exact values (the PostgreSQL, MySQL/MariaDB and SQLite plans under Task 5 Produces). Every other guard function is a fail-closed stub.
  - `require_target` and `require_target_in_set` ship working in the testkit (Q6).
  - `interaction.rs` replaces `PasswordRequest` with `PromptRequest { question, reply }` (also in `password_prompt.rs`) and routes by variant. `events.rs` subscribes to `ConnectionManager::events()` through a core-side relay into a `tokio::sync::mpsc` and **awaits it in a foreground task**, the same pattern as `AppUiPort` (Q4 ruling: allowed; no polling), and calls each handler. `RecvError::Lagged` re-reads state (Q42).
  - The app's `dialog::open_edit(id, DialogFocus, ..)` seam works for `Default`; `fixes.rs` routes `ConnectThroughSsh` and `ReviewHostKey` to their owners' stubs (Q9).
  - `ConnectionsModel::save` stores `ProfileDraft.ssh_secrets` (Q23); `boot.rs` builds `KnownHosts::load(..)` (no I/O) and `TunnelManager::new` into `ConnectDeps` (Q23, Q42).
  - `CapsuleModel::Reconnecting` renders like `Connecting`, with no clock. Task 22 finishes it.
  - The five new `IssueCode`s map to Task 1's placeholder messages in `issue_text` (Q3).
- [ ] **Step 4:** Run Step 2's commands, then `cargo test -p hacivat-driver --features fake`, `cargo check --workspace --all-targets`, `cargo test -p hacivat-session --no-run`, `cargo test -p hacivat-net --no-run` and `cargo test -p hacivat-app --no-run` (no features; Q5), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo xtask check-deps` and `cargo deny check licenses bans sources`. Expected: PASS, and all M2a tests still green (`cargo test -p hacivat-session -p hacivat-net -p hacivat-secrets -p hacivat-driver -p hacivat-driver-pg -p hacivat-driver-mysql -p hacivat-driver-sqlite` and `cargo test -p hacivat-app --features scene-check,pseudo`).
- [ ] **Step 5: Commit** with the message `feat(m2b): interface commit — guard crate, ssh/tls/vault types, RawConn execution surface, session types, vendored mysql_async, app prompt routing`. The commit body lists the `mysql_async` `Cargo.lock` entry change (path package: no `source`, no `checksum`) and any `--precise` bump.

**Review:** opus, adversarial. Probes:
- Does any stub succeed without its check (No silent weakening)?
- Is `aws-lc` absent from `cargo tree`?
- Does any new type that holds a secret derive `PartialEq` or print in `Debug`?
- Is the vendored copy byte-identical to the registry source, apart from the documented manifest edits? (`diff -r` against `~/.cargo/registry/src/*/mysql_async-0.37.1/src`.) Are `tests/`, `Cargo.lock`, `Cargo.toml.orig`, `azure-pipelines.yml` and `[profile.bench]` gone, and is `mysql_async` absent from `cargo metadata`'s workspace members?
- Is `plan_for` exactly Task 5's table, string for string?
- Does every Q3 site compile with a minimal arm (no `todo!()`, no `unreachable!()` on a reachable variant)?
- Does any new app test wait on a fixed sleep instead of `settle_until`?

### Task 2: Mocks for the new M2b surfaces (W0, parallel with Task 1)

**Files:** Create `design/m2b-secure/index.html` and `design/m2b-secure/NOTES.md`.

**Interfaces:**
- Consumes: `design/m2-connect/index.html`. Reuse its CSS, the Lucide sprite, the mock bar, the EN/TR toggle (`L`), the theme toggle (`T`), Notes (`N`) and deep links. The visual world is harman, unchanged (DESIGN.md).
- Produces: the binding mock for surfaces 1–7 of "New UI surfaces", and a copy table (`id | EN | TR | Status`) that Tasks 19–22 map to Fluent ids:
  - `hk-changed-*` → `hv-conn-hk-changed-*`;
  - `kbd-*` → `hv-conn-kbd-*`;
  - `sshsec-*` → `hv-conn-ssh-secret-*`;
  - `vault-*` → `hv-conn-vault-*`;
  - `pin-changed-*` → `hv-conn-tls-pin-changed-*`;
  - `ro-*` → `hv-conn-ro-*`;
  - `idle-*` → `hv-conn-idle-*`;
  - the error copy → `error-conn-<class>-*`.

  Every Turkish string carries "PO review".

- [ ] **Step 1:** Load the `impeccable` skill (or `frontend-design:frontend-design`), then read NOTES.md "NEW primitives" and surfaces 5–7. Each new surface reuses existing primitives: the 480 px wide dialog, the fingerprint inset, the chain list, the choice list, the sticky toast, the explanation popover and the masked input.
- [ ] **Step 2: Build the surfaces, one surface-switcher entry each.**
  1. **Host key changed.** No warning icon. The title states the fact: "The host key of bastion.acme.io changed". Old and new `SHA256:` fingerprints sit in two insets. "Stored in: Hacivat's known hosts" or "~/.ssh/known_hosts line 14". The actions: Cancel (primary, initial focus) and the outline button "Remove the old key and trust the new one" (Hacivat store only). For the `~/.ssh` source, a copyable `ssh-keygen -R "[bastion.acme.io]:22"` instead.
  2. **Keyboard-interactive.** "bastion.acme.io asks:", the server's name and instructions verbatim in their own field, 1–3 prompts with masked or echoed fields, and "Attempt 1 of 3".
  3. **SSH password and passphrase,** per hop. The hop is named ("jump-eu.acme.io, hop 1 of 2"), with the Remember Select.
  4. **Vault:**
     - Create: why a vault is needed ("This computer has no system keyring"), the master password twice, the 8-character minimum, and "Hacivat can't recover it".
     - Unlock: "Attempt 2 of 3".
     - Reset: type `reset` to confirm. The primary button is disabled until it matches.
     - The Remember Select row "Forever (encrypted file)".
  5. **Pinned certificate changed.** Surface 5's anatomy with "Pinned" and "Now" fingerprints. Primary: Cancel. Secondary: "Replace pin".
  6. **Read-only labels.**
     - Capsule: "read-only (enforced by server role)" (red lock only on production, DESIGN.md).
     - "read-only (guarded)" opens a 300 px popover: why it is only guarded, a SQL inset with the remediation snippet (`<role>` placeholders) and Copy.
     - The sheet's Access row.
  7. **Idle and errors.**
     - Toast: "Disconnected orders-dev after 4 h idle. It reconnects when you use it."
     - Error-dialog copy for `auth.insecure_method` ("The server asked for the password in a form that isn't safe on this connection"), `ssh.auth` with its hop, `ssh.channel_open` ("bastion.acme.io doesn't allow tunnels (AllowTcpForwarding)"), `ssh.agent_unavailable` and `tls.pin_changed`.
- [ ] **Step 3: Write NOTES.md.** Give each surface Intent, Anatomy and Keyboard sections, then the copy table, then "Open questions for the PO". Answer each open question with the ruling taken (B-numbers).
- [ ] **Step 4: Verify.** Open `index.html` in a browser (built-in browser skill or `open`). Every surface renders in EN/TR × dark/light with no console errors. `grep -c "PO review" NOTES.md` equals the number of TR strings.
- [ ] **Step 5: Commit** with the message `design(m2b): mocks for host-key changed, keyboard-interactive, SSH secrets, vault, pin changed, read-only labels, idle and SSH errors`. The controller then reviews the mocks against DESIGN.md (One Filled Badge, Hue Is Meaning, Raised Ink Rule) and logs `Ruling (PO-away): M2b mocks controller-approved` in the ledger, with the link for the PO.

**Review:** opus (design). Probes:
- Is any second filled badge or new hue introduced?
- Is any destructive or trust-widening action the default button?
- Is server text ever placed inside a translated sentence?

### Task 3: TLS pins, certificate review, encrypted client keys, channel-binding hash (HV-SEC-02; B5, B6, B9, B25)

**Files:**
- Modify: `crates/core/hacivat-net/src/tls.rs`.
- Create: `crates/core/hacivat-net/tests/{tls_pins,tls_review}.rs`.
- Append: an ADR 0003 section, `## m2b-t03 — TLS pins and review`.

**Interfaces:**
- Consumes: Task 1's tls types.
- Produces:
  - **Verifier wrapper.** In every mode except Disable, the `Recording` wrapper becomes a pin-aware verifier:
    1. Capture the presented chain into `presented`, on success and on failure.
    2. If a pin exists for `pin_endpoint(tls_host, port)`: the leaf's `cert_sha256` must equal it, and the leaf's validity window must contain now. Otherwise `PinMismatch` or `Expired`/`NotYetValid`. A match writes `outcome = Pinned` and skips the chain and name checks.
    3. With no pin, the M2a mode behaviour applies. On success, write `outcome`: `Full` for Auto and VerifyFull, `CaOnly` for VerifyCa, `No` for Require.
    4. Handshake signatures are always verified.
  - **A pin forbids the plaintext fallback (Q13):** `build_tls_params` sets `plaintext_allowed = false` whenever `settings.pins` holds an entry for `pin_endpoint(tls_host, port)`, even with `allow_public_plaintext`.
  - The same `TlsParams.config` (verifier, roots, client identity) is what MySQL uses through vendored patch §5 (Q2), so nothing in this task is PostgreSQL-specific.
  - `build_tls_params(.., port, .., key_passphrase)` decrypts encrypted PKCS #8 keys (PBES2 with PBKDF2-SHA2 or scrypt, and the SHA-1 PRF of OpenSSL 1.0 keys through `sha1-insecure`; Q36):
    - an encrypted key with no passphrase → `KeyEncrypted`;
    - a wrong passphrase → `KeyPassphraseWrong`;
    - `Proc-Type: 4,ENCRYPTED` → `KeyEncryptedLegacy`.
  - `cert_review` parses with `x509-parser`. Subject and issuer are RFC 4514 strings, verbatim. The chain roles come by position: leaf, intermediates, and the last entry as Root when it is self-issued. Malformed DER gives `None`, never a panic.
  - `tls_server_end_point` follows B9: `ecdsa-with-SHA512` → SHA-512, RSASSA-PSS → the hash from its parameters (Q36), Ed25519 → `None`.
  - `fingerprint_lines` produces upper-case hex with colons, 16 bytes per line.

- [ ] **Step 1: Write the failing tests.** They use in-process `tokio-rustls` servers with `rcgen` certificates; `handshake(settings, port) -> Result<TlsVerified, TlsFailure>` is a test helper.
```rust
#[tokio::test] async fn ca_true_self_signed_can_be_pinned() { let srv = server(self_signed_ca_true("db")); let pin = pin_of(&srv);
    assert_eq!(handshake(auto_with(&[pin]), srv.port()).await, Ok(TlsVerified::Pinned)); }
#[tokio::test] async fn ip_only_san_can_be_pinned() { let srv = server(self_signed_ip_san("10.0.0.5"));   // Q33
    assert_eq!(handshake_as(auto_with(&[pin_of_as(&srv, "10.0.0.5")]), "10.0.0.5", srv.port()).await, Ok(TlsVerified::Pinned)); }
#[tokio::test] async fn long_validity_cert_can_be_pinned() { let srv = server(self_signed_valid_days("db", 3650));   // the 825-day case (Q33)
    assert_eq!(handshake(auto_with(&[pin_of(&srv)]), srv.port()).await, Ok(TlsVerified::Pinned)); }
#[test] fn pinned_endpoint_never_allows_plaintext() {   // Q13
    let s = TlsSettings { allow_public_plaintext: true, ..auto_with(&[pin("db:5432")]) };
    assert!(!build_tls_params(&s, "db", 5432, AddressClass::Private, None).unwrap().unwrap().plaintext_allowed);
    assert!(build_tls_params(&auto_with(&[pin("db:5433")]), "db", 5432, AddressClass::Private, None).unwrap().unwrap().plaintext_allowed); }
#[tokio::test] async fn unknown_issuer_captures_the_presented_chain() { let srv = server(self_signed("db"));
    let (r, params) = handshake_keep(auto_with(&[]), srv.port()).await; assert_eq!(r, Err(TlsFailure::UnknownIssuer));
    let rev = cert_review(params.presented.lock().unwrap().as_ref().unwrap(), "db:5432", None).unwrap();
    assert_eq!(rev.sha256, cert_sha256(srv.leaf_der())); assert_eq!(rev.chain[0].role, ChainRole::Leaf); }
#[tokio::test] async fn rotated_certificate_is_pin_mismatch_with_both_fingerprints() { let old = pin_of(&server(self_signed("db")));
    let srv = server(self_signed("db")); let (r, p) = handshake_keep(auto_with(&[old.clone()]), srv.port()).await;
    assert_eq!(r, Err(TlsFailure::PinMismatch)); let rev = cert_review(p.presented.lock().unwrap().as_ref().unwrap(), &old.endpoint, Some(old.sha256)).unwrap();
    assert_eq!(rev.pinned, Some(old.sha256)); assert_ne!(rev.sha256, old.sha256); }
#[tokio::test] async fn pin_mismatch_even_when_the_chain_is_trusted() { let ca = test_ca(); let srv = server(ca.leaf("db"));
    let s = TlsSettings { ca_file: Some(ca.pem_file()), ..auto_with(&[other_pin("db:5432")]) };
    assert_eq!(handshake(s, srv.port()).await, Err(TlsFailure::PinMismatch)); }
#[tokio::test] async fn pin_does_not_override_expiry() { let srv = server(expired_self_signed("db"));
    assert_eq!(handshake(auto_with(&[pin_of(&srv)]), srv.port()).await, Err(TlsFailure::Expired)); }
#[tokio::test] async fn pin_overrides_name_mismatch() { let srv = server(self_signed("localhost"));   // connect as "db"
    assert_eq!(handshake(auto_with(&[pin_of_as(&srv, "db")]), srv.port()).await, Ok(TlsVerified::Pinned)); }
#[tokio::test] async fn require_records_chain_and_reports_not_verified() { let srv = server(self_signed("db"));
    let (r, p) = handshake_keep(mode(TlsMode::Require), srv.port()).await; assert_eq!(r, Ok(TlsVerified::No)); assert!(p.presented.lock().unwrap().is_some()); }
#[test] fn pins_never_apply_in_disable() { assert!(build_tls_params(&TlsSettings { mode: TlsMode::Disable, ..auto_with(&[pin("db:5432")]) }, "db", 5432, AddressClass::Public, None).unwrap().is_none()); }
#[test] fn turkish_subject_is_verbatim() { let rev = review_of(self_signed_cn("Müşteri İşlemleri")); assert!(rev.subject.contains("Müşteri İşlemleri")); }
#[test] fn malformed_der_gives_none() { assert!(cert_review(&PresentedChain(vec![CertificateDer::from(vec![0x30, 0x03, 0x01])]), "h:1", None).is_none()); }
#[test] fn pin_endpoint_and_fingerprint_lines() { assert_eq!(pin_endpoint("::1", 5432), "[::1]:5432"); assert_eq!(pin_endpoint("db.", 5432), "db:5432");
    let l = fingerprint_lines(&[0xAB; 32]); assert_eq!(l[0].split(':').count(), 16); assert_eq!(l[0], l[1]); assert!(l[0].starts_with("AB:AB")); }
#[test] fn encrypted_pkcs8_keys() { let (cert, key_enc) = client_cert_encrypted("pw1");
    assert!(build_tls_params(&with_client(&cert, &key_enc), "db", 5432, AddressClass::Private, Some(&SecretString::from("pw1"))).is_ok());
    assert_eq!(err_of(&with_client(&cert, &key_enc), Some("nope")), TlsSetupError::KeyPassphraseWrong(key_enc.clone()));
    assert_eq!(err_of(&with_client(&cert, &key_enc), None), TlsSetupError::KeyEncrypted(key_enc.clone()));
    assert!(matches!(err_of(&with_client(&cert, &legacy_encrypted_pem()), Some("pw1")), TlsSetupError::KeyEncryptedLegacy(_)));
    assert!(build_tls_params(&with_client(&cert, &client_key_pbes2_sha1_prf("pw1")), "db", 5432, AddressClass::Private, Some(&SecretString::from("pw1"))).is_ok()); }   // Q36
#[test] fn end_point_hash_follows_rfc_5929() { assert_eq!(tls_server_end_point(&sha256_signed_der()).unwrap(), sha256(&sha256_signed_der()).to_vec());
    assert_eq!(tls_server_end_point(&sha384_signed_der()).unwrap().len(), 48); assert!(tls_server_end_point(&ed25519_signed_der()).is_none());
    assert_eq!(tls_server_end_point(&ecdsa_sha512_signed_der()).unwrap().len(), 64);
    assert_eq!(tls_server_end_point(&rsa_pss_sha384_signed_der()).unwrap().len(), 48); }   // Q36
```
- [ ] **Step 2:** Run `cargo test -p hacivat-net --test tls_pins --test tls_review`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies described in Produces. `TlsParams` is built fresh per attempt (M2a ruling), so `presented` and `outcome` never mix two handshakes.
- [ ] **Step 4:** Run Step 2's command and `cargo test -p hacivat-net`. Expected: PASS, with the M2a TLS tests green.
- [ ] **Step 5: Commit** with the message `feat(net): TLS pins on the leaf certificate, presented-chain capture and review, encrypted PKCS#8 client keys, tls-server-end-point hash`.

**Review:** opus, adversarial. Probes:
- Can a pin for `db:5432` be used for `db:5433`, or for `DB` against `db`? (The endpoint is case-sensitive as typed; record the rule.)
- Does an expired pinned certificate ever pass?
- Is the presented chain written before the verdict, so a failed handshake still has it?
- Do two concurrent handshakes on one `TlsParams` mix their outcomes?
- Does x509 parsing panic on fuzzed DER? (Run 10,000 random mutations.)

### Task 4: `~/.ssh/config` import and known hosts (HV-SEC-01; B11, B12)

**Files:**
- Modify: `crates/core/hacivat-net/src/ssh/{config,known_hosts}.rs`.
- Create: `crates/core/hacivat-net/tests/{ssh_config,known_hosts}.rs`, with fixtures under `crates/core/hacivat-net/tests/fixtures/ssh/`.
- Append: an ADR 0003 section, `## m2b-t04 — SSH config import and known hosts`.

**Interfaces:**
- Consumes: Task 1's ssh types and `hacivat_driver::url::carries_secret`. That is a dev-only consumption through the session layer: `hacivat-net` cannot depend on `hacivat-driver`, so `read_ssh_config` takes the predicate as a parameter. Its signature becomes `read_ssh_config(path, home, local_user, is_secret: &dyn Fn(&str) -> bool)`, and Task 1's stub must already use that signature.
- Produces:
  - **Config parsing per B12.** `resolve_alias` builds hops from `ProxyJump`: `ProxyJump j1,j2` on alias `a` gives `[j1, j2, a]`, each resolved through its own `Host` block. A cycle is `ConfigIssue::IncludeTooDeep`, after 8 levels. `HostName` with `%h` expands like `IdentityFile`; `IdentityAgent` gives `ConfigNotice::IdentityAgentIgnored { alias }` (Q43).
  - **`KnownHosts`:**
    - Parsing: plain lines, `[host]:port`, `|1|salt|hash` (HMAC-SHA1 over `host` or `[host]:port`), comma lists, `@revoked` and `@cert-authority` markers, `#` comments, CRLF, and lines up to 16 KiB (longer lines are skipped and logged).
    - `load` does no I/O (Q42): every file is read lazily at the first `check` or `trust` (the pipeline calls them on the core runtime). The Hacivat file is created on the first `trust`, `0600`, in a `0700` directory.
    - `check` re-reads an OpenSSH file when its `(mtime, len)` changed since the last read: one `stat` per file per check, under the same mutex as the writer (Q24). The copied `ssh-keygen -R` advice therefore works on the next retry without a restart.
    - `trust` and `replace` go through one ordered writer (`Mutex<()>` around read-modify-rename).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn host_patterns_first_value_wins() { let c = cfg("Host *.acme.io !secret.acme.io\n User deploy\nHost db.acme.io\n User other\n");
    assert_eq!(host(&c, "db.acme.io").user.as_deref(), Some("deploy")); assert!(find(&c, "secret.acme.io").is_none()); }
#[test] fn proxy_jump_chain_resolves_to_hops() { let c = cfg("Host bastion\n HostName b.acme.io\n ProxyJump jump-eu,jump-2\nHost jump-eu\n HostName j.acme.io\n User u1\nHost jump-2\n HostName k.acme.io\n");
    let s = resolve_alias(&c, "bastion").unwrap(); assert_eq!(s.hops.iter().map(|h| h.host.as_str()).collect::<Vec<_>>(), ["j.acme.io", "k.acme.io", "b.acme.io"]); }
#[test] fn identity_file_tokens_expand() { let c = cfg_home("Host b\n HostName b.io\n User u\n IdentityFile ~/.ssh/%r@%h_%p\n", "/home/me");
    assert_eq!(host(&c, "b").identity_files, [PathBuf::from("/home/me/.ssh/u@b.io_22")]); }
#[test] fn unsupported_token_marks_entry_unusable() { assert_eq!(host(&cfg("Host b\n IdentityFile ~/.ssh/%C\n"), "b").unusable, Some(ConfigIssue::UnsupportedToken("%C".into()))); }
#[test] fn proxycommand_entries_are_listed_but_unusable() { assert_eq!(host(&cfg("Host p\n ProxyCommand nc %h %p\n"), "p").unusable, Some(ConfigIssue::ProxyCommand)); }
#[test] fn include_loop_stops_at_depth_8() { let c = cfg_files(&[("config", "Include a\nHost x\n HostName x.io\n"), ("a", "Include config\n")]);   // no hang, no stack overflow
    assert!(c.notices.iter().any(|n| matches!(n, ConfigNotice::IncludeTooDeep { .. }))); assert!(find(&c, "x").is_some()); }   // Q32
#[test] fn hostname_percent_h_expands() { assert_eq!(host(&cfg("Host db\n HostName %h.acme.io\n"), "db").hostname.as_deref(), Some("db.acme.io")); }   // Q43
#[test] fn identity_agent_is_noted_not_imported() { assert!(cfg("Host b\n IdentityAgent ~/.1password/agent.sock\n").notices.iter()
    .any(|n| matches!(n, ConfigNotice::IdentityAgentIgnored { alias } if alias == "b"))); }   // Q43
#[test] fn match_blocks_are_skipped_with_a_notice() { assert!(matches!(cfg("Match host x\n User y\n").notices[0], ConfigNotice::MatchSkipped { line: 1 })); }
#[test] fn secret_looking_values_are_refused() { assert_eq!(host(&cfg("Host b\n HostName h\n IdentityFile /k?password=x\n"), "b").unusable, Some(ConfigIssue::SecretInValue)); }
#[test] fn strict_host_key_checking_never_maps_to_accept_all() { for (v, p) in [("yes", HostKeyPolicy::Strict), ("accept-new", HostKeyPolicy::AcceptNew), ("no", HostKeyPolicy::Ask), ("off", HostKeyPolicy::Ask), ("ask", HostKeyPolicy::Ask)] {
    assert_eq!(host(&cfg(&format!("Host b\n StrictHostKeyChecking {v}\n")), "b").strict_host_key_checking, Some(p)); } }
#[test] fn hashed_entries_match() { let kh = known(&[&hashed_line("db.acme.io", 22, &K1)]); assert!(matches!(kh.check("db.acme.io", 22, &K1), HostKeyCheck::Known(KnownSource::OpenSsh { .. }))); }
#[test] fn non_default_port_uses_bracket_form() { let kh = known(&[&format!("[db.acme.io]:2222 {}", K1.openssh)]);
    assert!(matches!(kh.check("db.acme.io", 2222, &K1), HostKeyCheck::Known(_))); assert_eq!(kh.check("db.acme.io", 22, &K1), HostKeyCheck::Unknown); }
#[test] fn changed_key_reports_old_key_and_source() { let kh = known(&[&format!("db {}", K1.openssh)]);
    assert!(matches!(kh.check("db", 22, &K2), HostKeyCheck::Changed { old, source: KnownSource::OpenSsh { line: 1, .. } } if old == K1)); }
#[test] fn revoked_marker_is_a_hard_stop() { assert!(matches!(known(&[&format!("@revoked * {}", K1.openssh)]).check("db", 22, &K1), HostKeyCheck::Revoked { .. })); }
#[test] fn trust_writes_hacivat_store_only() { let (kh, ssh_file, store) = known_tmp(); let before = std::fs::read(&ssh_file).unwrap();
    kh.trust("db", 22, &K1).unwrap(); assert_eq!(std::fs::read(&ssh_file).unwrap(), before); #[cfg(unix)] assert_eq!(mode_of(&store), 0o600);   // Q5
    assert!(matches!(kh.check("db", 22, &K1), HostKeyCheck::Known(KnownSource::Hacivat))); }
#[test] fn replace_only_touches_the_hacivat_store() { let (kh, _, store) = known_tmp(); kh.trust("db", 22, &K1).unwrap(); kh.replace("db", 22, &K1, &K2).unwrap();
    assert!(!read(&store).contains(&K1.openssh)); assert!(matches!(kh.check("db", 22, &K2), HostKeyCheck::Known(KnownSource::Hacivat))); }
#[test] fn concurrent_trusts_are_ordered() { let (kh, _, store) = known_tmp(); std::thread::scope(|s| { for i in 0..8 { let kh = &kh; s.spawn(move || kh.trust(&format!("h{i}"), 22, &K1).unwrap()); } });
    assert_eq!(read(&store).lines().count(), 8); }
#[test] fn cert_authority_line_is_recognised() { let r = known(&[&format!("@cert-authority *.acme.io {}", CA.openssh)]).check("db.acme.io", 22, &K1);
    assert_eq!(r, HostKeyCheck::Unknown); }   // Q32: assert the exact variant implemented (Unknown unless russh exposes host certificates); ADR records it
#[test] fn openssh_file_edit_is_seen_without_restart() { let (kh, ssh_file) = known_openssh_tmp(&format!("db {}", K1.openssh));   // Q24
    assert!(matches!(kh.check("db", 22, &K2), HostKeyCheck::Changed { .. })); rewrite_with_new_mtime(&ssh_file, "");   // as `ssh-keygen -R db` would
    assert_eq!(kh.check("db", 22, &K2), HostKeyCheck::Unknown); }
#[test] fn load_does_no_io() { let kh = KnownHosts::load(vec![PathBuf::from("/nonexistent/known_hosts")], tmp_store()); assert_eq!(kh.check("db", 22, &K1), HostKeyCheck::Unknown); }   // Q42
#[test] fn fingerprint_is_sha256_base64_unpadded() { assert!(K1.fingerprint().starts_with("SHA256:")); assert!(!K1.fingerprint().ends_with('=')); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-net --test ssh_config --test known_hosts`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. Parse bytes as UTF-8 lossily and only for display. Matching compares the raw bytes and ASCII-folds the host (OpenSSH lower-cases host names in its own ASCII way, INV-3).
- [ ] **Step 4:** Run Step 2's command. Expected: PASS. Pin the `@cert-authority` behaviour in ADR 0003.
- [ ] **Step 5: Commit** with the message `feat(net): ~/.ssh/config import (ProxyJump chains, tokens, Include) and known hosts with hashed entries, markers and Hacivat's own 0600 store`.

**Review:** opus, adversarial. Probes:
- Can a crafted `known_hosts` line make an unknown key "known"? Try a wildcard abuse like `* key`, a negated pattern, or a hash collision attempt.
- Can an `Include` follow a symlink outside home? (Allowed, but never executed. Assert that only reading happens.)
- Can a `Host` value inject a newline into the Hacivat store through `trust`?
- Can a non-UTF-8 byte cause a panic?
- Can a 10 MB line cause one?

### Task 5: `hacivat-guard`, the sans-I/O read-only layers (SQL-SAFE-05, HV-SESS-04, MCP-09 step 1; B21)

**Files:**
- Modify: `crates/core/hacivat-guard/src/{plan,post,input,sqlite,probe,label}.rs`.
- Create: `crates/core/hacivat-guard/tests/{plans,post,sqlite_authorizer,probe,labels}.rs`.
- Append: an ADR 0004 section, `## m2b-t05 — guard layers per engine`.

**Interfaces:**
- Consumes: `hacivat-proto` (`DbmsKind`, `AccessLabel`).
- Produces: the bodies of Task 1's guard functions, with these exact values, **except `plan_for`, which Task 1 ships working with the plan values below; Task 5 tests it** (`plans.rs`; Q8). If Task 5 finds a plan value wrong, it fixes `plan_for` and reports it to the controller, because Task 7 tests against the same values.
  - **PostgreSQL plan:**
    - `session_setup` = `["SET default_transaction_read_only = on", "SET standard_conforming_strings = on", "SET client_encoding = 'UTF8'", "SET idle_in_transaction_session_timeout = '30s'"]`;
    - `before` = `["BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY", "SELECT 1"]`;
    - `canary` = `Some("SELECT txid_current_if_assigned()")`;
    - `after` = `["ROLLBACK"]`;
    - `reset` = `DiscardAll` (the name is kept; the driver runs the `DISCARD ALL` sequence without `DEALLOCATE ALL`, Task 10, Q10; ADR 0004 records the equivalence with S-02/#pg-discard).
  - **MySQL and MariaDB plan:**
    - `session_setup` = `["SET SESSION TRANSACTION READ ONLY", "SET NAMES utf8mb4"]`;
    - `before` = `["START TRANSACTION READ ONLY"]`;
    - `canary` = `None`;
    - `after` = `["ROLLBACK"]`;
    - `reset` = `ResetConnection`.
  - **SQLite plan:** every list empty, `reset` = `None`. SQLite's layers are connection-level, through `sqlite_hardening`.
  - **`check_post` gaps:**
    - the statement left the transaction `Idle` (PostgreSQL or MySQL) → `TransactionEndedByStatement`;
    - the canary is `Some(Some(_))` → `CanaryAssigned`;
    - `rollback_notice_codes` contains `"25P01"` (PostgreSQL "no transaction in progress") → `TransactionEndedByStatement`;
    - `rows_affected > 0` with a write command tag (`INSERT`, `UPDATE`, `DELETE`, `MERGE`, `COPY` (from), `CREATE`, `DROP`, `ALTER`, `TRUNCATE`) → `WriteSucceeded`.
  - **`sqlite_hardening`:**
    - `query_only: true`, `defensive: true`, `attach_create: false`, `attach_write: false`;
    - `limit_attached: 0`, `limit_length: 100_000_000`, `limit_sql_length: 1 << 20`, `limit_variable_number: 32_766`.
  - **`sqlite_authorize`** denies by default. It allows:
    - `Read`, `Select` and `Recursive`;
    - `Function` except `load_extension`, `readfile`, `writefile`, `edit` and `fts3_tokenizer`;
    - `Pragma` named in `SQLITE_READ_PRAGMAS` **with no argument**. The list: `table_info`, `table_xinfo`, `index_list`, `index_info`, `index_xinfo`, `foreign_key_list`, `database_list`, `collation_list`, `function_list`, `pragma_list`, `schema_version`, `user_version`, `page_count`, `page_size`, `encoding`, `compile_options`, `query_only`, `journal_mode`, `trusted_schema`, `foreign_keys` (the last four read by Task 11's test and M2a's probe; with an argument they stay denied; Q16).

    It denies everything else: `Attach`, `Detach`, writes (temp included), `Create`, `Drop`, `Alter`, `Transaction`, `Savepoint`, `Analyze`, `Other`, and any pragma with an argument.
  - **`role_probe_sql`:**
    - PostgreSQL, 5 statements:
      1. `SELECT rolsuper, rolbypassrls, rolreplication, rolcreaterole FROM pg_roles WHERE rolname = current_user`
      2. `SELECT r.rolname FROM pg_roles r WHERE pg_has_role(current_user, r.oid, 'MEMBER') AND r.rolname IN ('pg_read_server_files','pg_write_server_files','pg_execute_server_program','pg_signal_backend','pg_write_all_data')`
      3. `SELECT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r','p','v','m','f') AND n.nspname NOT IN ('pg_catalog','information_schema') AND (pg_has_role(current_user, c.relowner, 'USAGE') OR has_table_privilege(c.oid, 'INSERT, UPDATE, DELETE, TRUNCATE')))`
      4. `SELECT has_database_privilege(current_database(), 'TEMP')`
      5. `SELECT extname FROM pg_extension WHERE extname IN ('dblink','postgres_fdw','pg_net','http')`
    - MySQL and MariaDB, 1 statement: `SHOW GRANTS`.
    - SQLite: none.
  - **`interpret_probe`:**
    - PostgreSQL: least privilege means no `t` in row 1, no rows in 2, `f` in 3, and no rows in 5. A `t` in 4 adds `RoleWarning::TempPrivilege` only and does **not** clear `least_privilege` (MCP-09 parity: temp creation is already blocked inside the read-only transaction, #pg-ro-temp; Q21). Each other hit adds its `RoleWarning`.
    - MySQL: every `GRANT` line's privilege list is within {`USAGE`, `SELECT`, `SHOW VIEW`}, with no `WITH GRANT OPTION` and no `PROXY`. A role-grant line (`GRANT \`r\`@\`%\` TO …`) is `RoleGrantNotExpanded`, so not least privilege (fail closed).
    - Anything unparsable is `Unparsed`, so not least privilege.
  - **`access_label`:** not read-only → `ReadWrite`. Read-only, PostgreSQL or MySQL, with least privilege → `ReadOnlyEnforced`. Everything else → `ReadOnlyGuarded`, which includes MariaDB (C19) and SQLite.
  - **`remediation`:**
    - PostgreSQL: `CREATE ROLE <role> LOGIN PASSWORD '<password>';\nGRANT pg_read_all_data TO <role>;\n-- optional: REVOKE TEMP ON DATABASE <database> FROM PUBLIC;\n-- optional: REVOKE CREATE ON SCHEMA public FROM PUBLIC;` (CREATE line: T5 review, clusters created on PG ≤ 14) (no `NOINHERIT`: a non-inheriting member would not get `pg_read_all_data` and could read nothing; Q21)
    - MySQL: `CREATE USER '<role>'@'%' IDENTIFIED BY '<password>';\nGRANT SELECT, SHOW VIEW ON <database>.* TO '<role>'@'%';`
    - MariaDB and SQLite: `None`.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn pg_plan_matches_safe_05() { let p = plan_for(DbmsKind::Postgres).unwrap(); assert_eq!(p.before, ["BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY", "SELECT 1"]);
    assert_eq!(p.canary, Some("SELECT txid_current_if_assigned()")); assert_eq!(p.after, ["ROLLBACK"]); assert_eq!(p.reset, ResetKind::DiscardAll);
    assert!(p.session_setup.contains(&"SET default_transaction_read_only = on")); }
#[test] fn mysql_and_mariadb_plans() { for k in [DbmsKind::MySql, DbmsKind::MariaDb] { let p = plan_for(k).unwrap();
    assert_eq!(p.session_setup[0], "SET SESSION TRANSACTION READ ONLY"); assert_eq!(p.before, ["START TRANSACTION READ ONLY"]); assert_eq!(p.reset, ResetKind::ResetConnection); } }
#[test] fn post_check_gaps() { assert_eq!(check_post(DbmsKind::Postgres, &facts().canary(Some("731"))), PostCheck::Gap(GapReason::CanaryAssigned));
    assert_eq!(check_post(DbmsKind::Postgres, &facts().tx_after(TxStatusLite::Idle)), PostCheck::Gap(GapReason::TransactionEndedByStatement));
    assert_eq!(check_post(DbmsKind::Postgres, &facts().rollback_notice("25P01")), PostCheck::Gap(GapReason::TransactionEndedByStatement));
    assert_eq!(check_post(DbmsKind::MySql, &facts().tag("INSERT").rows(1)), PostCheck::Gap(GapReason::WriteSucceeded));
    assert_eq!(check_post(DbmsKind::Postgres, &facts().tx_after(TxStatusLite::InTx).canary(None)), PostCheck::Clean); }
#[test] fn input_limits() { assert_eq!(check_input(&"x".repeat(MAX_STATEMENT_BYTES + 1)), Err(InputViolation::TooLong { limit: MAX_STATEMENT_BYTES }));
    assert_eq!(check_input("select '\0'"), Err(InputViolation::Nul)); assert!(check_input("select 1").is_ok()); }
#[test] fn sqlite_hardening_values() { let h = sqlite_hardening(); assert!(h.query_only && h.defensive && !h.attach_create && !h.attach_write); assert_eq!(h.limit_attached, 0); }
#[test] fn sqlite_authorizer_default_deny() { use SqliteAction::*; for (a, want) in [
        (Read { table: "t", column: "a" }, Authorization::Allow), (Select, Authorization::Allow), (Function { name: "count" }, Authorization::Allow),
        (Function { name: "load_extension" }, Authorization::Deny), (Function { name: "writefile" }, Authorization::Deny),
        (Pragma { name: "table_info", arg: None }, Authorization::Allow), (Pragma { name: "query_only", arg: None }, Authorization::Allow),
        (Pragma { name: "journal_mode", arg: None }, Authorization::Allow), (Pragma { name: "query_only", arg: Some("0") }, Authorization::Deny),
        (Pragma { name: "journal_mode", arg: Some("delete") }, Authorization::Deny), (Pragma { name: "writable_schema", arg: None }, Authorization::Deny),
        (Attach { file: "new.db" }, Authorization::Deny), (Detach, Authorization::Deny), (Insert { table: "temp_t" }, Authorization::Deny),
        (Transaction, Authorization::Deny), (Other(99), Authorization::Deny)] { assert_eq!(sqlite_authorize(&a), want, "{a:?}"); } }
#[test] fn pg_probe_least_privilege_and_warnings() { assert!(interpret_probe(DbmsKind::Postgres, &pg_rows(false, &[], false, false, &[])).least_privilege);
    let f = interpret_probe(DbmsKind::Postgres, &pg_rows(true, &["pg_signal_backend"], true, true, &["dblink"]));
    assert!(!f.least_privilege); for w in [RoleWarning::Superuser, RoleWarning::SignalBackend, RoleWarning::WritePrivileges, RoleWarning::TempPrivilege, RoleWarning::EgressExtension("dblink".into())] { assert!(f.warnings.contains(&w), "{w:?}"); }
    let t = interpret_probe(DbmsKind::Postgres, &pg_rows(false, &[], false, true, &[]));   // TEMP alone (Q21)
    assert!(t.least_privilege); assert_eq!(t.warnings, [RoleWarning::TempPrivilege]); }
#[test] fn mysql_grants() { assert!(interpret_probe(DbmsKind::MySql, &grants(&["GRANT USAGE ON *.* TO `hv_ro`@`%`", "GRANT SELECT, SHOW VIEW ON `hv_test`.* TO `hv_ro`@`%`"])).least_privilege);
    assert!(!interpret_probe(DbmsKind::MySql, &grants(&["GRANT SELECT, INSERT ON `d`.* TO `u`@`%`"])).least_privilege);
    assert!(!interpret_probe(DbmsKind::MySql, &grants(&["GRANT FILE ON *.* TO `u`@`%`"])).least_privilege);
    assert!(interpret_probe(DbmsKind::MySql, &grants(&["GRANT `r`@`%` TO `u`@`%`"])).warnings.contains(&RoleWarning::RoleGrantNotExpanded)); }
#[test] fn unparsable_probe_is_guarded() { assert!(!interpret_probe(DbmsKind::Postgres, &[vec![vec![Some("weird".into())]]]).least_privilege); }
#[test] fn labels() { let lp = RoleFindings { least_privilege: true, warnings: vec![] };
    assert_eq!(access_label(DbmsKind::Postgres, true, Some(&lp)), AccessLabel::ReadOnlyEnforced); assert_eq!(access_label(DbmsKind::MariaDb, true, Some(&lp)), AccessLabel::ReadOnlyGuarded);
    assert_eq!(access_label(DbmsKind::Sqlite, true, None), AccessLabel::ReadOnlyGuarded); assert_eq!(access_label(DbmsKind::MySql, false, Some(&lp)), AccessLabel::ReadWrite); }
#[test] fn remediation_has_only_placeholders() { let s = remediation(DbmsKind::Postgres).unwrap(); assert!(s.contains("<role>") && s.contains("<database>"));
    assert!(!s.contains("NOINHERIT")); assert!(remediation(DbmsKind::MariaDb).is_none()); }   // Q21
```
- [ ] **Step 2:** Run `cargo test -p hacivat-guard`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies. Keywords and pragma names are compared with `eq_ignore_ascii_case` (INV-3). The crate has no I/O and no async.
- [ ] **Step 4:** Run `cargo test -p hacivat-guard` and `cargo xtask check-deps`. Expected: PASS.
- [ ] **Step 5: Commit** with the message `feat(guard): read-only plans per engine, post-checks, input limits, SQLite hardening and default-deny authorizer, role probe and honest labels`.

**Review:** opus, adversarial. Probes:
- Does any SQLite action pass that writes or attaches? Enumerate every `SQLITE_*` action code (0–33), map each through Task 11's adapter, and assert Deny for every one not in the allow list.
- Does a pragma name differ in case or carry a schema prefix (`main.query_only`)?
- Is a `GRANT` line with backtick-quoted privileges or extra whitespace misparsed as least privilege?
- Does any probe interpretation fail open?

### Task 6: Vault, fallback backend and pgpass (HV-SEC-05, D-016; B13, B27)

**Files:**
- Modify: `crates/core/hacivat-secrets/src/backend/{vault,fallback,mod}.rs`, `crates/core/hacivat-secrets/src/store.rs` (`backend_choice`, `vault()`, the Forever-put fallback) and `crates/core/hacivat-secrets/src/files/pgpass.rs`.
- Create: `crates/core/hacivat-secrets/tests/{vault,fallback}.rs`. Append to `tests/pgpass.rs`.
- Append: an ADR 0003 section, `## m2b-t06 — vault`.

**Interfaces:**
- Consumes: Task 1's vault types.
- Produces:
  - **`FileVault`, per B13.**
    - The header layout:

      | Bytes | Field |
      |---|---|
      | 0–7 | `HVVAULT1` |
      | 8–9 | `format_major`, u16 LE = 1 |
      | 10–11 | `format_minor` = 0 |
      | 12–15 | `m_kib` u32 LE |
      | 16–19 | `t` |
      | 20–23 | `p` |
      | 24–39 | salt |
      | 40–63 | nonce |

    - The body is the ciphertext. The AAD is the 64-byte header.
    - **Header bounds before deriving (Q30):** `m_kib` in 8,192–1,048,576, `t` in 1–10, `p` in 1–8; anything else is `VaultCorrupt` and no key is derived. A `FileVault` built with `with_params(p)` (the tests' fast params) also accepts a header equal to its own `p`; `FileVault::open` accepts only the bounds.
    - The plaintext is a JSON object `{ "<service>": "<secret>" }`, built inside `Zeroizing<Vec<u8>>` and parsed straight into `SecretString` values (secrecy's `serde` feature), never a plain `String` (Q30).
    - Salt and nonce come from `getrandom::fill` (Q30). Every write draws a fresh nonce and keeps the salt.
    - `create` refuses a master password under 8 characters (`SecretError::Backend("master-too-short")` is never shown; Task 21 validates first) and refuses when the file exists.
    - `unlock` derives the key and decrypts. A wrong password is `WrongMasterPassword`; an AEAD failure with a header that parses is the same. A header that does not parse, or a truncated file, is `VaultCorrupt`.
    - A newer `format_major` is `VaultReadOnly`: `get` works, `put` and `delete` refuse.
    - `get` while locked is `VaultLocked`.
    - `lock` zeroizes the key. `reset` deletes the file and locks.
  - **`FallbackBackend`.** On the first call it runs `probe` once (`OnceLock`). Probe true routes everything to the keyring; probe false routes to the vault. `SecretBackend::choice()` reports which (`KeyringBackend` reports `Some(Keyring)` once probed, `MemoryBackend` `Some(Memory)`). `as_vault()` returns the vault once it has been chosen. `probe_available()` returns `true` when the vault is chosen, so Forever stays enabled with the vault label (Q23).
  - **`SecretStore`:** `backend_choice()` reads `SecretBackend::choice`; `vault()` returns `as_vault()` (Q23).
  - **Forever-put fallback (Q38).** `SecretStore::put(.., Forever)` adds `VaultLocked`, `VaultReadOnly` and `VaultCorrupt` to the errors that become `Stored::FellBackTo { retention: UntilRestart, reason }` (the M2a toast path, `store.rs:208-213`), never a hard error. Callers run `create` and `unlock` on the blocking pool (Argon2).
  - **pgpass:** an empty password field gives `None`.

- [ ] **Step 1: Write the failing tests.** The tests use fast Argon2 params through `with_params(.., VaultParams { m_kib: 64, t: 1, p: 1 })`.
```rust
#[test] fn vault_round_trip() { let v = vault_tmp(); v.create(sec("correct horse")).unwrap(); v.put(&key(), &sec("s3cret")).unwrap(); v.lock();
    assert_eq!(v.get(&key()), Err(SecretError::VaultLocked)); v.unlock(sec("correct horse")).unwrap(); assert_eq!(expose(v.get(&key()).unwrap()), "s3cret"); }
#[test] fn vault_file_never_contains_plaintext() { let (v, path) = vault_tmp_path(); v.create(sec("correct horse")).unwrap(); v.put(&key(), &sec("s3cret-value")).unwrap();
    let bytes = std::fs::read(&path).unwrap(); assert!(!contains(&bytes, b"s3cret-value") && !contains(&bytes, key().service().as_bytes())); #[cfg(unix)] assert_eq!(mode_of(&path), 0o600); }   // Q5
#[test] fn wrong_master_password() { let v = created("correct horse"); v.lock(); assert_eq!(v.unlock(sec("wrong horse")), Err(SecretError::WrongMasterPassword)); assert!(!v.is_unlocked()); }
#[test] fn tampered_vault_is_untouched() { let (v, path) = created_with_secret(); v.lock(); flip_byte(&path, 70); let before = std::fs::read(&path).unwrap();   // Q30: the AEAD cannot tell tamper from a wrong password
    assert!(matches!(v.unlock(sec("correct horse")), Err(SecretError::WrongMasterPassword | SecretError::VaultCorrupt))); assert!(v.put(&key(), &sec("x")).is_err()); assert_eq!(std::fs::read(&path).unwrap(), before); }
#[test] fn truncated_vault_is_corrupt() { let (v, path) = created_with_secret(); v.lock(); truncate(&path, 20); assert_eq!(v.unlock(sec("correct horse")), Err(SecretError::VaultCorrupt)); }
#[test] fn newer_vault_version_is_read_only() { let (v, path) = created_with_secret(); v.lock(); set_major(&path, 2); let v = FileVault::with_params(path, fast());
    assert!(v.unlock(sec("correct horse")).is_ok()); assert_eq!(expose(v.get(&key()).unwrap()), "s3cret");   // Q30: one unlock, Ok
    assert_eq!(v.put(&key(), &sec("x")), Err(SecretError::VaultReadOnly)); }
#[test] fn crafted_header_params_are_corrupt() { for (m, t, p) in [(u32::MAX, 3, 1), (4, 3, 1), (65536, 0, 1), (65536, 11, 1), (65536, 3, 9)] {   // Q30
    let (v, path) = created_with_secret(); v.lock(); set_params(&path, m, t, p); let v = FileVault::open(path);
    assert_eq!(v.unlock(sec("correct horse")), Err(SecretError::VaultCorrupt), "{m} {t} {p}"); } }
#[test] fn master_shorter_than_8_is_refused() { assert!(vault_tmp().create(sec("short")).is_err()); }
#[test] fn every_write_uses_a_fresh_nonce() { let (v, path) = created_with_secret(); let n1 = nonce_of(&path); v.put(&key(), &sec("y")).unwrap(); assert_ne!(nonce_of(&path), n1); assert_eq!(salt_of_before_after(&path), true); }
#[test] fn reset_deletes_file_and_locks() { let (v, path) = created_with_secret(); v.reset().unwrap(); assert!(!path.exists() && !v.is_unlocked() && !v.exists()); }
#[test] fn debug_never_shows_master_or_values() { let v = created_with_secret().0; assert!(!format!("{v:?}").contains("s3cret") && !format!("{v:?}").contains("correct horse")); }
// tests/fallback.rs
#[test] fn fallback_uses_keyring_when_probe_true() { let (fb, kr, _) = fallback(true); fb.put(&key(), &sec("x")).unwrap(); assert_eq!(kr.calls(), 1); assert_eq!(fb.choice(), Some(BackendChoice::Keyring)); }
#[test] fn fallback_uses_vault_when_probe_false_and_probes_once() { let probes = counter(); let (fb, _, vault) = fallback_counting(false, &probes);
    vault.create(sec("correct horse")).unwrap(); fb.put(&key(), &sec("x")).unwrap(); fb.get(&key()).unwrap(); assert_eq!(probes.get(), 1); assert_eq!(fb.choice(), Some(BackendChoice::Vault)); assert!(fb.as_vault().is_some()); }
#[test] #[ignore = "ci-linux"] fn linux_without_secret_service_uses_vault() {   // run by CI `vault-linux` (Task 16): no D-Bus session
    let fb = FallbackBackend::new(Arc::new(KeyringBackend::new(None)), Arc::new(vault_fast_tmp()), Box::new(|| KeyringBackend::new(None).probe_available()));
    let _ = fb.get(&key()); /* first call runs the probe */ fb.as_vault().unwrap().create(sec("correct horse")).unwrap();
    fb.put(&key(), &sec("x")).unwrap(); assert_eq!(fb.choice(), Some(BackendChoice::Vault)); }
#[test] fn store_reports_backend_choice() { let s = SecretStore::new(Some(Arc::new(fallback(false).0))); assert_eq!(s.backend_choice(), None); let _ = s.keyring_available(); assert_eq!(s.backend_choice(), Some(BackendChoice::Vault));
    assert!(s.vault().is_some()); assert!(s.keyring_available()); }   // Q23: the vault keeps Forever enabled
#[test] fn locked_vault_forever_put_falls_back_to_until_restart() { let s = store_with_locked_vault();   // Q38
    assert!(matches!(s.put(&key(), sec("x"), Retention::Forever), Ok(Stored::FellBackTo { retention: Retention::UntilRestart, reason: SecretError::VaultLocked }))); }
// tests/pgpass.rs (append)
#[test] fn empty_password_field_is_none() { assert!(lookup(&file(0o600, "h:5432:d:u:\n"), "h", 5432, "d", "u").unwrap().password.is_none()); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-secrets --test vault --test fallback --test pgpass`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies. The write sequence is: write `vault.bin.tmp` with mode `0600`, `fsync`, rename, then `fsync` the directory on Unix. Key derivation runs on the caller's thread; callers are already on the blocking pool.
- [ ] **Step 4:** Run Step 2's command and `cargo test -p hacivat-secrets`. Expected: PASS, with the M2a store tests green.
- [ ] **Step 5: Commit** with the message `feat(secrets): XChaCha20-Poly1305 + Argon2id file vault, keyring-or-vault fallback backend, empty pgpass password is None`.

**Review:** opus, adversarial. Probes:
- Is a nonce ever reused, including after a failed write?
- Can a downgrade of the header params decrypt? (The header is AAD, so it must fail.)
- Is anything secret left in memory after `lock`? (Check that `Zeroizing` covers the JSON buffer.)
- Does a temp file survive a crash with plaintext? (It never holds plaintext; assert it.)
- Is a symlink at `vault.bin` followed on write? (Refuse it.)
- Is the unlock timing data-dependent beyond the AEAD itself?

### Task 7: Session actor core: queue, transactions, cancel, environment, schema, reset (HV-SESS-01 actor, -03, -05, -06, -07, -08; HV-EXEC-05/-09/-10 core; B1, B3, B22, B23)

**Files:**
- Modify: `crates/core/hacivat-session/src/session/{mod,actor,tx,cancel,env,schema,reset}.rs`.
- Create: `crates/core/hacivat-session/tests/{session_actor,tx,cancel,env_schema}.rs`. Append helpers to `tests/common/mod.rs`.
- Append: an ADR 0004 section, `## m2b-t07 — session actor`.

**Interfaces:**
- Consumes: Task 1's session types, `FakeDriver` execution scripting and `LiveConnection`.
- Produces:
  - **`spawn_session`:** a Tokio task that owns the `LiveConnection`.
    - It runs a FIFO `VecDeque` of requests, one at a time, and publishes a `watch` of `SessionSnapshot` (the generation bumps on every change) and a `broadcast` of `SessionEvent`.
    - A panic or `JoinError` in the driver call is caught (`AssertUnwindSafe` plus `catch_unwind` on the future) and marks the session `Broken(Internal)`. The connection is discarded, other sessions are unaffected, and the panic message is logged without SQL.
    - Read-only sessions send `Exec` through `readonly::run_guarded(.., replay)`, which is Task 18's. `replay` is the session's current `EnvPlan::statements()` (including `RestoreSchema`), minus the PostgreSQL client identity, which the startup parameter restores (Q15).
    - `spawn_session` runs neither the environment plan nor a schema probe: connect already set the identity and read-only (Q27).
  - **Transactions, per B22 and HV-SESS-03:**
    - status comes from `RawConn::tx_status()` after every round trip;
    - Manual on PostgreSQL and SQLite sends `begin_sql(..)` before the next statement when the status is `Idle`. MySQL sends `SET autocommit = 0` at `SetMode(Manual)` and `SET autocommit = 1` at `SetMode(Auto)`;
    - Manual → Auto while `InTx` or `Failed` is `TxError::InProgress`. `SetMode(Smart)` is `NeedsClassifier`. In `Failed`, any `Exec` other than ROLLBACK is `TxError::FailedNeedsRollback`. That check is on the first keyword only and is advisory; the server rejects anyway;
    - **Read-only sessions are always Auto (Q14):** `Profile.tx_mode` and `default_mode` apply only to read-write sessions; on a read-only session `SetMode(Manual | Smart)` and `TxCommand::Begin` are `TxError::Unsupported`;
    - `begin_sql(Postgres, Serializable, false)` = `["BEGIN ISOLATION LEVEL SERIALIZABLE"]`. For MySQL it is `["SET TRANSACTION ISOLATION LEVEL SERIALIZABLE", "START TRANSACTION"]`. For SQLite it is `["BEGIN"]`; SQLite always uses `BEGIN IMMEDIATE` when the first statement is not `SELECT`/`WITH`, by first keyword.
  - **Cancel, per B23:** `cancel(seq)` answers `AlreadyFinished` unless `seq` is the running request (the stale-cancel guard, Q18). Otherwise it marks `Cancelling(seq)`, completes every queued request with `Cancelled`, and calls the connection's `CancelHandle`. For MySQL (no handle), it opens a side connection through `SessionHooks.side` and executes `KILL QUERY <backend_id>`; the side connection is **pre-opened once a statement has run for 500 ms** (Q37). The outcome comes within `CANCEL_WAIT` (2 s), otherwise `StillRunning`, and `SlowCancel` fires at 10 s. `escalate(seq, Terminate)` runs `pg_terminate_backend(<pid>)` or `KILL CONNECTION <id>` on a side connection. `escalate(seq, Deactivate)` closes and marks `Broken(Cancelled)`. ADR 0004 records the `KILL` limits behind proxies (Q37).
  - **`build_env_plan`:**
    1. client identity: PostgreSQL `SET application_name = '<escaped>'`; MySQL needs nothing, because the connect attribute already carries it;
    2. read-only setup: `plan_for(kind).session_setup` when `read_only`;
    3. time zone: none until the settings model (M3);
    4. schema restore: `switch_schema_sql` only when `last_schema.schema.is_some()` (Q27).

    With Task 1's working `plan_for`, `env_plan_order` is green in W1 without Task 5 (Q8).

    `apply` runs each statement through `batch`.
  - **Schema:**
    - `schema_probe_sql`: PostgreSQL `SELECT current_database(), current_schema(), array_to_string(current_schemas(true), ','), current_user`; MySQL `SELECT DATABASE(), CURRENT_USER()`; SQLite `PRAGMA database_list`.
    - `switch_schema_sql`: PostgreSQL `SET search_path TO "<name with "" doubled>"`; MySQL `` USE `<name with `` doubled>` ``. The exact name is sent, never case-folded.
    - `needs_schema_reprobe(sql)`: the first keyword, ASCII-folded and with leading whitespace and comments skipped, is one of `SET`, `RESET`, `USE`, `BEGIN`, `START`, `COMMIT`, `END`, `ROLLBACK`, `ABORT` or `DISCARD`.
    - **The actor re-probes (Q27) only** after an `ExecOrigin::User` statement matching `needs_schema_reprobe`, after a user `TxCommand::{Commit, Rollback}`, and after `SwitchSchema`, and emits `SchemaChanged` on a difference. **Never** after internal control statements (Manual's `BEGIN`, `SET autocommit`, the reset's `ROLLBACK`) or after `Reset`: Reset's environment plan restores the last schema.
  - **`Reset`, per HV-SESS-08:**
    1. ROLLBACK if not `Idle`;
    2. `RawConn::reset()`;
    3. a verify probe: PostgreSQL `SELECT current_user = session_user`;
    4. re-apply the environment plan.

    Any failure marks `Broken` and closes the connection.
  - **`close(RefuseIfOpenTx)`** while `InTx` is `Err(CloseRefused::OpenTransaction)`. `close(RollBack)` rolls back, then closes.

- [ ] **Step 1: Write the failing tests** (FakeDriver).
```rust
#[tokio::test] async fn requests_run_in_fifo_order() { let (h, f) = session(fake_pg().exec(vec![slow("select 1", ms(50))]));
    let a = h.submit(exec("select 1")); let b = h.submit(exec("select 2")); a.reply.await.unwrap().unwrap(); b.reply.await.unwrap().unwrap();
    assert_eq!(f.executed(), ["select 1", "select 2"]); assert!(b.seq > a.seq); }
#[tokio::test] async fn tx_state_follows_protocol_status() { let (h, _) = session(fake_pg().exec(vec![tx("BEGIN", TxStatus::InTx), tx_err("SELECT 1/0", "22012", TxStatus::Failed)]));
    run(&h, "BEGIN").await; assert_eq!(snap(&h).tx.status, TxStatus::InTx); let _ = run(&h, "SELECT 1/0").await; assert_eq!(snap(&h).tx.status, TxStatus::Failed); }
#[tokio::test] async fn manual_mode_begins_before_next_statement() { let (h, f) = session(fake_pg()); tx_cmd(&h, TxCommand::SetMode(TxMode::Manual)).await.unwrap();
    run(&h, "update t set a = 1").await; assert_eq!(f.executed(), ["BEGIN", "update t set a = 1"]); }
#[tokio::test] async fn mysql_manual_uses_autocommit() { let (h, f) = session(fake_mysql()); tx_cmd(&h, TxCommand::SetMode(TxMode::Manual)).await.unwrap(); assert_eq!(f.executed(), ["SET autocommit = 0"]); }
#[tokio::test] async fn manual_to_auto_in_transaction_is_refused() { let (h, _) = in_tx_session().await;
    assert_eq!(tx_cmd(&h, TxCommand::SetMode(TxMode::Auto)).await.unwrap_err().kind, ExecErrorKind::Tx(TxError::InProgress)); }
#[tokio::test] async fn smart_mode_needs_classifier() { let (h, _) = session(fake_pg()); assert_eq!(tx_cmd(&h, TxCommand::SetMode(TxMode::Smart)).await.unwrap_err().kind, ExecErrorKind::Tx(TxError::NeedsClassifier)); }
#[test] fn defaults_and_begin_sql() { assert_eq!(default_mode(Some(EnvTag::Production)), TxMode::Manual); assert_eq!(default_mode(Some(EnvTag::Dev)), TxMode::Auto);
    assert_eq!(begin_sql(DbmsKind::Postgres, IsolationLevel::Serializable, false), ["BEGIN ISOLATION LEVEL SERIALIZABLE"]);
    assert_eq!(begin_sql(DbmsKind::MySql, IsolationLevel::ReadCommitted, false), ["SET TRANSACTION ISOLATION LEVEL READ COMMITTED", "START TRANSACTION"]); }
// tests/cancel.rs
#[tokio::test] async fn cancel_resolves_running_and_drops_queue() { let (h, f) = session(fake_pg().exec(vec![slow("select pg_sleep(60)", secs(60))]));
    let a = h.submit(exec("select pg_sleep(60)")); let b = h.submit(exec("select 2")); settle().await;
    assert_eq!(h.cancel(a.seq).await.unwrap(), CancelOutcome::Cancelled); assert_eq!(a.reply.await.unwrap().unwrap_err().kind, ExecErrorKind::Cancelled);
    assert_eq!(b.reply.await.unwrap().unwrap_err().kind, ExecErrorKind::Cancelled); assert_eq!(f.cancel_calls(), 1); assert_eq!(snap(&h).state, SessionState::Idle); }
#[tokio::test] async fn stale_cancel_never_hits_the_next_request() { let (h, f) = session(fake_pg()); let a = h.submit(exec("select 1")); a.reply.await.unwrap().unwrap();
    assert_eq!(h.cancel(a.seq).await.unwrap(), CancelOutcome::AlreadyFinished); assert_eq!(f.cancel_calls(), 0); }
#[tokio::test] async fn stale_cancel_with_old_seq_does_not_cancel_the_next_request() {   // Q18
    let (h, f) = session(fake_pg().exec(vec![slow("select 2", ms(200))])); let a = h.submit(exec("select 1")); let b = h.submit(exec("select 2"));
    a.reply.await.unwrap().unwrap(); assert_eq!(h.cancel(a.seq).await.unwrap(), CancelOutcome::AlreadyFinished);
    assert!(b.reply.await.unwrap().is_ok()); assert_eq!(f.cancel_calls(), 0); }
#[tokio::test(start_paused = true)] async fn cancel_waits_two_seconds_then_still_running_and_slow_at_ten() { let (h, _) = session(fake_pg().ignores_cancel().exec(vec![slow("select x", secs(60))]));
    let a = h.submit(exec("select x")); let mut ev = h.events(); settle().await; let c = h.cancel(a.seq); tokio::time::advance(secs(2)).await;
    assert_eq!(c.await.unwrap(), CancelOutcome::StillRunning); tokio::time::advance(secs(8)).await; assert!(matches!(next(&mut ev).await, SessionEvent::SlowCancel { .. })); }
#[tokio::test] async fn mysql_cancel_uses_kill_query_on_a_side_connection() { let side = FakeDriver::new(DbmsKind::MySql);
    let (h, _) = session_with_side(fake_mysql().backend(42).exec(vec![slow("select sleep(60)", secs(60))]), &side);
    let a = h.submit(exec("select sleep(60)")); settle().await; let _ = h.cancel(a.seq).await; assert_eq!(side.executed(), ["KILL QUERY 42"]); }
#[tokio::test(start_paused = true)] async fn mysql_side_connection_is_prewarmed_at_500ms() { let side = FakeDriver::new(DbmsKind::MySql);   // Q37
    let (h, _) = session_with_side(fake_mysql().backend(42).exec(vec![slow("select sleep(60)", secs(60))]), &side); let _a = h.submit(exec("select sleep(60)"));
    tokio::time::advance(ms(400)).await; settle().await; assert_eq!(side.connect_calls(), 0); tokio::time::advance(ms(200)).await; settle().await; assert_eq!(side.connect_calls(), 1); }
#[tokio::test] async fn terminate_escalation_uses_a_side_connection() { let side = FakeDriver::new(DbmsKind::Postgres);
    let (h, _) = session_with_side(fake_pg().backend(7).ignores_cancel().exec(vec![slow("select x", secs(60))]), &side); let a = h.submit(exec("select x")); settle().await;
    let _ = h.escalate(a.seq, Escalation::Terminate).await; assert_eq!(side.executed(), ["SELECT pg_terminate_backend(7)"]); }
#[tokio::test] async fn driver_panic_marks_session_broken_internal() { let (h, _) = session(fake_pg().exec(vec![panics("select boom")]));
    let r = h.submit(exec("select boom")).reply.await.unwrap(); assert_eq!(r.unwrap_err().kind, ExecErrorKind::Internal); assert_eq!(snap(&h).state, SessionState::Broken(ErrClass::Internal)); }
// tests/env_schema.rs
#[test] fn env_plan_order() { let p = build_env_plan(DbmsKind::Postgres, &ro_profile(), true, Some(&schema("app")));
    assert_eq!(p.statements(), ["SET application_name = 'Hacivat'", "SET default_transaction_read_only = on", "SET standard_conforming_strings = on",
        "SET client_encoding = 'UTF8'", "SET idle_in_transaction_session_timeout = '30s'", "SET search_path TO \"app\""]); }
#[test] fn switch_schema_quotes_exactly() { assert_eq!(switch_schema_sql(DbmsKind::Postgres, "Müşteri"), "SET search_path TO \"Müşteri\"");
    assert_eq!(switch_schema_sql(DbmsKind::Postgres, "a\"b"), "SET search_path TO \"a\"\"b\""); assert_eq!(switch_schema_sql(DbmsKind::MySql, "an`x"), "USE `an``x`"); }
#[test] fn reprobe_rule() { for s in ["SET search_path=x", "  /* c */ use db", "rollback", "Begin"] { assert!(needs_schema_reprobe(s), "{s}"); }
    for s in ["select 1", "settings", "user"] { assert!(!needs_schema_reprobe(s), "{s}"); } }
#[tokio::test] async fn schema_reprobed_after_rollback() { let (h, f) = session(fake_pg().exec(vec![rows("SELECT current_database()", &[&["d", "app", "app,public", "u"]])]));
    let mut ev = h.events(); run(&h, "ROLLBACK").await;   // a user statement
    assert!(f.executed().iter().any(|s| s.starts_with("SELECT current_database()"))); assert!(matches!(next_schema_changed(&mut ev).await, SessionEvent::SchemaChanged(_))); }   // Q32
#[tokio::test] async fn internal_control_statements_never_reprobe() { let (h, f) = session(fake_mysql()); tx_cmd(&h, TxCommand::SetMode(TxMode::Manual)).await.unwrap();   // Q27
    assert!(!f.executed().iter().any(|s| s.starts_with("SELECT DATABASE()"))); }
#[tokio::test] async fn read_only_session_is_always_auto() { let (h, _) = ro_session(fake_pg(), production_profile());   // Q14
    assert_eq!(snap(&h).tx.mode, TxMode::Auto); assert_eq!(tx_cmd(&h, TxCommand::SetMode(TxMode::Manual)).await.unwrap_err().kind, ExecErrorKind::Tx(TxError::Unsupported));
    assert_eq!(tx_cmd(&h, TxCommand::Begin).await.unwrap_err().kind, ExecErrorKind::Tx(TxError::Unsupported)); }
#[tokio::test] async fn reset_rolls_back_then_resets_then_reapplies_env() { let (h, f) = in_tx_session().await; h.submit(Request::Reset).reply.await.unwrap().unwrap();
    assert_eq!(&f.executed()[f.executed().len() - 3..], ["ROLLBACK", "SELECT current_user = session_user", "SET application_name = 'Hacivat'"]); assert_eq!(f.reset_calls(), 1); }
#[tokio::test] async fn failed_reset_breaks_and_closes() { let (h, f) = session(fake_pg().reset_fails()); let _ = h.submit(Request::Reset).reply.await;
    assert!(matches!(snap(&h).state, SessionState::Broken(_))); assert_eq!(f.open_connections(), 0); }
#[tokio::test] async fn close_with_open_tx() { let (h, _) = in_tx_session().await; assert_eq!(h.close(ClosePolicy::RefuseIfOpenTx).await, Err(CloseRefused::OpenTransaction));
    assert_eq!(h.close(ClosePolicy::RollBack).await, Ok(())); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --test session_actor --test tx --test cancel --test env_schema`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. Every literal interpolated into control SQL is escaped by its own function (`quote_pg_ident`, `quote_my_ident`, `quote_pg_literal`) and never built with string concatenation at call sites. The keep-alive ping and the read-only path belong to other tasks (17 and 18).
- [ ] **Step 4:** Run Step 2's command and `cargo test -p hacivat-session`. Expected: PASS, with the M2a tests green.
- [ ] **Step 5: Commit** with the message `feat(session): session actor with FIFO queue, protocol-derived transaction state, cancel ladder with stale-cancel guard, environment plan, schema tracking, reset`.

**Review:** opus. Probes:
- Is there a race between cancel and completion (cancel issued while the reply is being sent)?
- Is any panic not caught?
- Is quoting complete for names with NUL, backticks or quotes?
- Is a session ever reused after `Broken`?

### Task 8: `mysql_async` patches (B10; Q2)

**Files:**
- Modify: `third_party/mysql_async/src/**` (the five patches) and `third_party/mysql_async/PATCHES.md` (§1–§5, each with the diff summary, rationale and an upstream PR text).
- Modify: `crates/core/hacivat-driver-mysql/tests/docker_mysql.rs` (the superseded VerifyCa defect test's `#[ignore]` only; Q29, Q40).
- Create: `crates/core/hacivat-driver-mysql/tests/patches.rs`.

**Interfaces:**
- Consumes: the pristine vendored copy from Task 1.
- Produces, in `mysql_async` (vendored):
  - **§1** `skip_domain_validation` matches `rustls::CertificateError::{NotValidForName, NotValidForNameContext { .. }}` by **variant**. It stays as an upstream bug fix; Hacivat's VerifyCa runs through its own `ChainOnly` verifier via §5.
  - **§2** `OptsBuilder::password_guard(Option<Arc<dyn Fn() -> bool + Send + Sync>>)`, getter `Opts::password_guard()`. Default `None` (upstream behaviour). It is evaluated at the `caching_sha2_password` "perform full auth" point (`0x04`, `conn/mod.rs:807-827`), before either branch: sending the password in clear (`is_secure() || is_socket()`) or requesting the server's RSA key (`0x02`). `false` returns the new `DriverError::InsecurePasswordRefused`, which Task 13 maps to `RawErrorKind::InsecureAuthRefused`. `sha256_password` is unreachable (the handshake maps every plugin except `caching_sha2` to native, `:550-555`), and `mysql_clear_password` stays gated by `enable_cleartext_plugin(false)`; `PATCHES.md` records both facts.
  - **§3** `OptsBuilder::multi_statements(bool)` and **§4** `OptsBuilder::local_infile_capability(bool)`, with getters; defaults `true`, so upstream behaviour is unchanged; `get_capabilities` (`opts/mod.rs:1152-1158`) honours them.
  - **§5** `SslOpts::with_rustls_client_config(Arc<rustls::ClientConfig>)`, getter `SslOpts::rustls_client_config()`. When set, `build_tls_connector` (`src/io/tls/rustls_io.rs`) returns `TlsConnector::from(cfg)` and ignores the root certificates, the client identity and the danger flags; `with_danger_tls_hostname_override` still supplies the server name. Default `None` (upstream behaviour).

- [ ] **Step 1: Write the failing tests** (`crates/core/hacivat-driver-mysql/tests/patches.rs`).
```rust
#[test] fn knobs_default_to_upstream_behaviour() { let o = Opts::from(OptsBuilder::default()); assert!(o.password_guard().is_none()); assert!(o.multi_statements() && o.local_infile_capability());
    assert!(SslOpts::default().rustls_client_config().is_none()); }
#[test] fn knobs_set() { let o = Opts::from(OptsBuilder::default().password_guard(Some(Arc::new(|| false))).multi_statements(false).local_infile_capability(false));
    assert!(!(o.password_guard().unwrap())() && !o.multi_statements() && !o.local_infile_capability()); }
#[test] fn insecure_password_refused_error_exists() { let e = mysql_async::DriverError::InsecurePasswordRefused; assert!(!format!("{e}").is_empty()); }
#[tokio::test] async fn custom_client_config_is_used() {   // §5: an in-process tokio-rustls server that speaks only the MySQL greeting + SSL switch
    let srv = mysql_greeting_tls_server(self_signed("db")).await; let (cfg, calls) = counting_verifier_config();
    let opts = OptsBuilder::default().ip_or_hostname("127.0.0.1").tcp_port(srv.port())
        .ssl_opts(SslOpts::default().with_rustls_client_config(cfg).with_danger_tls_hostname_override(Some("db".into())));
    let _ = Conn::new(opts).await; assert_eq!(calls.load(Ordering::SeqCst), 1); assert_eq!(srv.sni(), Some("db".into())); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-driver-mysql --test patches`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** the five patches. Each is the smallest diff that does the job, and each is fenced by a `// HACIVAT PATCH §N` comment. `PATCHES.md` lists every touched file and line range. Do not reformat upstream code.
- [ ] **Step 4:** Run Step 2's command, then `cargo test -p hacivat-driver-mysql` (the M2a unit tests), then `cargo xtask docker test --set pr -p hacivat-driver-mysql` (the M2a Docker tests stay green; the VerifyCa defect test now **fails as expected**, so mark it `#[ignore = "superseded by Task 13"]` with a note). Expected: PASS.
- [ ] **Step 5: Commit** with the message `fix(vendor): mysql_async 0.37.1 patches — VerifyCa variant match, password guard at full auth, multi-statements and local-infile capability knobs, custom rustls ClientConfig`.

**Review:** opus, adversarial.
- Probes: is there any auth path that still sends the password, or asks for the key, when the guard returns `false`? Trace every call of `write_bytes` in `src/conn/mod.rs` auth code.
- With §5 set, does any code path still build a `ClientConfig` of its own (roots, `DangerousVerifier`, identity) or fall back to it on error?
- Does the capability knob also stop `COM_SET_OPTION` from turning multi-statements back on? (The driver never sends it; assert that it is not reachable from `Queryable`.)

### Task 9: SSH `TunnelManager`, testkit SSH server and agent (HV-SEC-01; B11, B20)

**Files:**
- Modify: `crates/core/hacivat-net/src/ssh/{tunnel,mod}.rs`, `crates/core/hacivat-testkit/src/{ssh_server,agent}.rs`.
- Create: `crates/core/hacivat-net/tests/ssh_tunnel.rs`.
- Append: an ADR 0003 section, `## m2b-t09 — SSH tunnels`.

**Interfaces:**
- Consumes:
  - Task 4's `KnownHosts`;
  - Task 1's `TunnelManager`, `SshPrompter`, `TunnelRequest` and `TunnelLease`;
  - `hacivat-net`'s `direct`, `resolve` and `StreamFactory`.
- Produces:
  - **`acquire`, following HV-SEC-01 steps 1–7.**
    - **Connecting.** Hop 0 uses a TCP connect via `direct`. Hop *i* > 0 opens a `direct-tcpip` channel on session *i−1* and runs SSH over it.
    - **Host keys.** Each hop's key is checked against `KnownHosts`, applying the policy:
      - `Unknown` + `Ask` + a prompter → `prompter.host_key`; `true` → `known.trust`, `false` → `HostKeyUnknown`;
      - `Unknown` + `Ask` without a prompter → `HostKeyUnknown`;
      - `Unknown` + `AcceptNew` → trust, logged;
      - `Unknown` + `Strict` → `HostKeyUnknown`;
      - `Changed` → `HostKeyChanged` in every policy;
      - `Revoked` → `HostKeyRevoked`.

      The host key is verified **before any authentication byte** is sent.
    - **Authentication order (B11).** Agent: `SSH_AUTH_SOCK` on Unix, `\\.\pipe\openssh-ssh-agent` on Windows. Key files: decrypted with `HopSecrets.passphrase`, else `prompter.secret(Passphrase)`, up to 3 attempts per key. Then password, then keyboard-interactive through `prompter.keyboard`, up to 3 attempts.
    - **Sharing.** Leases are shared by `ChainKey` (a SHA-256 over every hop's `(host, port, user, auth identity, policy)`) unless `share == false`.
    - **Keep-alive** runs per hop (`TunnelConfig`). After `keepalive_max` failures the generation bumps and `TunnelEvent::Dead` is sent once.
    - **Release** happens in reverse order, after `idle_grace` with zero users. On partial failure, the hops already up are released at once.
  - **`TunnelLease::factory(host, port)`** opens a `direct-tcpip` channel per call. The database host is resolved remotely.
  - **testkit `TestSshServer`** (russh server, `feature = "ssh"`), exactly Task 1's declared API (Q23):
    - `TestSshServer::start(cfg) -> TestSshServer { pub addr: SocketAddr, pub host_key: HostKey }`, with `host()`, `auth_attempts()`, `sessions_open()`, `sessions_accepted()`, `channels_opened()`, `kill_sessions()` and `stop()`;
    - `TestSshConfig` builders `user_pw`, `user_key`, `user_kbd`, `forward(ForwardPolicy::{Allow, Deny})`, `forward_to(ForwardTarget::{Any, Redirect(SocketAddr)})`, `max_auth_tries`, `slow_banner` and `ipv6`. `Redirect` sends every `direct-tcpip` to one address, so tests can use unresolvable database hosts such as `db.internal`.
    - `open_server_channel()` lets a test try a server-initiated channel, and `last_originator()` reports the last `direct-tcpip` originator (Q46).

    It binds `127.0.0.1:0` (or `[::1]:0` with `ipv6()`).
  - **testkit `TestAgent::start(keys) -> TestAgent { pub socket: PathBuf }`**, `#[cfg(unix)]` only, over russh's agent server. Every test that uses it is `#[cfg(unix)]` (Windows CI runs `cargo test --workspace`; Q5).
  - **Channel hygiene (B11, Q46):** the client never requests agent forwarding, X11, a pty, a shell or exec, refuses every server-initiated channel, and sends `127.0.0.1:0` as the `direct-tcpip` originator.

- [ ] **Step 1: Write the failing tests** (all in-process; `echo_target()` is a tokio TCP echo server).
```rust
#[tokio::test] async fn password_auth_and_direct_tcpip_relays() { let t = echo_target().await; let s = ssh(cfg().user_pw("u", "pw")).await;
    let lease = mgr(&s).acquire(req(&s, SshAuth::Password).secret_pw("pw"), None, tok()).await.unwrap(); assert_eq!(echo_through(&lease, t).await, b"ping"); }
#[tokio::test] async fn host_key_is_checked_before_any_auth() { let s = ssh(cfg().user_pw("u", "pw")).await;
    let e = mgr_strict(&s).acquire(req(&s, SshAuth::Password).secret_pw("pw"), None, tok()).await.unwrap_err();
    assert!(matches!(e, SshError::HostKeyUnknown { hop: 0, .. })); assert_eq!(s.auth_attempts(), 0); }
#[tokio::test] async fn unknown_host_key_asks_in_ask_policy_and_trust_stores() { let s = ssh(cfg().user_pw("u", "pw")).await; let p = prompter().trusts();
    mgr(&s).acquire(req(&s, SshAuth::Password).secret_pw("pw"), Some(p.clone()), tok()).await.unwrap(); assert_eq!(p.host_key_asks(), 1);
    assert!(matches!(known_of(&s).check(&host(&s), port(&s), &s.host_key), HostKeyCheck::Known(KnownSource::Hacivat))); }
#[tokio::test] async fn accept_new_stores_without_prompt() { let s = ssh(cfg().user_pw("u", "pw")).await; let p = prompter();
    mgr_with(known_empty(), HostKeyPolicy::AcceptNew).acquire(req_policy(&s, HostKeyPolicy::AcceptNew), Some(p.clone()), tok()).await.unwrap(); assert_eq!(p.host_key_asks(), 0); }
#[tokio::test] async fn changed_key_is_never_accepted_in_any_policy() {
    for policy in [HostKeyPolicy::Ask, HostKeyPolicy::AcceptNew, HostKeyPolicy::Strict] { let s = ssh(cfg().user_pw("u", "pw")).await; let known = known_with(&s, &OTHER_KEY);
        let e = mgr_with(known, policy).acquire(req_policy(&s, policy), Some(prompter().trusts()), tok()).await.unwrap_err();
        assert!(matches!(e, SshError::HostKeyChanged { hop: 0, .. }), "{policy:?}"); } }
#[tokio::test] async fn encrypted_key_without_passphrase_prompts_per_hop() { let (a, b) = jump_pair_with_encrypted_keys("pa", "pb").await; let p = prompter().passphrases(["pa", "pb"]).trusts();
    mgr(&a).acquire(chain_req(&a, &b), Some(p.clone()), tok()).await.unwrap(); assert_eq!(p.secret_asks(), [(0, "passphrase"), (1, "passphrase")]); }
#[cfg(unix)] #[tokio::test] async fn agent_auth_with_in_process_agent() { let k = ed25519(); let agent = TestAgent::start(vec![k.clone()]).await;
    let s = ssh(cfg().user_key("u", k.public())).await; assert!(mgr(&s).acquire(req(&s, SshAuth::Agent).agent(&agent.socket), None, tok()).await.is_ok()); }
#[cfg(unix)] #[tokio::test] async fn identities_only_tries_the_key_file_first() { let s = ssh(cfg().user_key("u", KEY_A.public()).max_auth_tries(1)).await;
    let agent = agent_with_wrong_keys(5).await; let r = req_key(&s, &KEY_A_FILE).identities_only(true).agent(&agent.socket);
    assert!(mgr(&s).acquire(r, None, tok()).await.is_ok()); }
#[cfg(unix)] #[tokio::test] async fn many_agent_keys_without_identities_only_reports_auth_with_hop() {   // MaxAuthTries (Q33)
    let s = ssh(cfg().user_key("u", KEY_A.public()).max_auth_tries(2)).await; let agent = agent_with_wrong_keys(5).await;
    let e = mgr(&s).acquire(req(&s, SshAuth::Agent).agent(&agent.socket), None, tok()).await.unwrap_err(); assert!(matches!(e, SshError::Auth { hop: 0, .. })); }
#[tokio::test] async fn keyboard_interactive_prompts_are_forwarded_verbatim() { let s = ssh(cfg().user_kbd("u", &[("Doğrulama kodu: ", false, "123456")])).await;
    let p = prompter().keyboard(["123456"]).trusts(); mgr(&s).acquire(req(&s, SshAuth::KeyboardInteractive), Some(p.clone()), tok()).await.unwrap();
    assert_eq!(p.kbd_requests()[0].prompts[0].text, "Doğrulama kodu: "); assert!(!p.kbd_requests()[0].prompts[0].echo); }
#[tokio::test] async fn two_leases_same_chain_share_one_session_and_different_user_does_not() { let s = ssh(cfg().user_pw("u", "pw").user_pw("v", "pw")).await; let m = mgr(&s);
    let _a = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap(); let _b = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap(); assert_eq!(s.sessions_open(), 1);
    let _c = m.acquire(req_user(&s, "v"), None, tok()).await.unwrap(); assert_eq!(s.sessions_open(), 2); }
#[tokio::test] async fn test_connection_never_shares() { let s = ssh(cfg().user_pw("u", "pw")).await; let m = mgr(&s);
    let _a = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap(); let _t = m.acquire(req_user(&s, "u").no_share(), None, tok()).await.unwrap(); assert_eq!(s.sessions_open(), 2); }
#[tokio::test] async fn idle_session_closes_after_grace() {   // real time (Q25): never start_paused with real sockets
    let s = ssh(cfg().user_pw("u", "pw")).await; let m = mgr_fast(&s);   // TunnelConfig { idle_grace: ms(200), keepalive: ms(100), .. }
    drop(m.acquire(req_user(&s, "u"), None, tok()).await.unwrap()); tokio::time::sleep(ms(100)).await; assert_eq!(m.sessions_open(), 1);
    timeout(secs(1), wait_until(|| m.sessions_open() == 0)).await.unwrap(); }
#[tokio::test] async fn partial_chain_failure_releases_in_reverse() { let (a, b) = jump_pair().await; let e = mgr(&a).acquire(chain_req(&a, &b).bad_password_at(1), None, tok()).await.unwrap_err();
    assert!(matches!(e, SshError::Auth { hop: 1, .. })); settle().await; assert_eq!(a.sessions_open(), 0); }
#[tokio::test] async fn forwarding_refused_names_the_hop() { let s = ssh(cfg().user_pw("u", "pw").forward(ForwardPolicy::Deny)).await;
    let lease = mgr(&s).acquire(req_user(&s, "u"), None, tok()).await.unwrap(); let e = lease.factory("db", 5432).open().await.unwrap_err();
    assert!(matches!(classify_channel_err(&e), Some(SshError::ChannelOpen { hop: 0, .. }))); }   // the classified error only (Q32)
#[tokio::test] async fn server_initiated_channels_are_refused() { let s = ssh(cfg().user_pw("u", "pw")).await;   // Q46
    let _lease = mgr(&s).acquire(req_user(&s, "u"), None, tok()).await.unwrap(); assert!(s.open_server_channel().await.is_err()); }
#[tokio::test] async fn direct_tcpip_originator_is_loopback_zero() { let t = echo_target().await; let s = ssh(cfg().user_pw("u", "pw")).await;   // Q46
    let lease = mgr(&s).acquire(req_user(&s, "u"), None, tok()).await.unwrap(); let _ = echo_through(&lease, t).await;
    assert_eq!(s.last_originator(), Some(("127.0.0.1".into(), 0))); }
#[tokio::test] async fn keepalive_failure_emits_dead_once_with_generation() { let s = ssh(cfg().user_pw("u", "pw")).await; let m = mgr_fast_keepalive(&s); let mut ev = m.events();
    let lease = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap(); let g = lease.generation(); s.kill_sessions();
    assert_eq!(timeout(secs(5), ev.recv()).await.unwrap().unwrap(), TunnelEvent::Dead { key: g.key.clone(), generation: g.generation });
    assert!(timeout(ms(300), ev.recv()).await.is_err()); }
#[tokio::test] async fn ipv6_bastion() { let s = ssh(cfg().user_pw("u", "pw").ipv6()).await; assert!(mgr(&s).acquire(req_user(&s, "u"), None, tok()).await.is_ok()); }
#[tokio::test] async fn cancel_during_handshake_releases_everything() { let s = ssh(cfg().user_pw("u", "pw").slow_banner(ms(500))).await; let t = tok();
    let h = tokio::spawn(mgr(&s).acquire(req_user(&s, "u"), None, t.clone())); tokio::time::sleep(ms(50)).await; t.cancel();
    assert!(matches!(h.await.unwrap(), Err(SshError::Cancelled))); settle().await; assert_eq!(s.sessions_open(), 0); }
#[tokio::test] async fn ssh_secrets_never_logged() { let s = ssh(cfg().user_pw("u", "s3cret-ssh")).await;
    let logs = capture_tracing(|| async { mgr(&s).acquire(req(&s, SshAuth::Password).secret_pw("s3cret-ssh"), None, tok()).await.unwrap(); }).await; assert!(!logs.contains("s3cret-ssh")); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-net --test ssh_tunnel` and `cargo test -p hacivat-testkit --features ssh`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. The manager's maps are guarded by `std::sync::Mutex` and are never held across `.await`. Every hop's russh handler has its `check_server_key` delegate to the policy above; there is no code path that returns `Ok(true)` without it. `SshError` has its own `Display` with the hop index and **no** secret.
- [ ] **Step 4:** Run Step 2's commands. Expected: PASS.
- [ ] **Step 5: Commit** with the message `feat(net): SSH TunnelManager — jump chains, shared sessions, host-key policies, agent/key/password/keyboard-interactive, keep-alive and group events; testkit SSH server and agent`.

**Review:** opus, adversarial. Probes:
- Is any `check_server_key` path accepting without the policy, including jump hops and reconnects?
- Is a password sent to an unverified host?
- Does the refcount underflow on double drop?
- Is a channel leaked when `open()` is cancelled?
- Does a prompter that never answers block other profiles' tunnels? (It must be per-acquire.)
- Is the agent socket path taken from an untrusted profile? (No: from the environment only.)
- Does the client ever request agent forwarding, X11, a pty, a shell or exec, or accept a server-initiated channel? Does a `direct-tcpip` originator leak a local address (Q46)?
- Is any test `start_paused` while talking to the in-process server over real sockets (Q25)?

### Task 10: PostgreSQL driver M2b: execution, wire observer, insecure-auth guard, channel binding, cancel, reset (B2, B3, B8, B9, B23)

**Files:**
- Modify: `crates/core/hacivat-driver-pg/src/{lib,connect,tls,probe,wire,exec,cancel}.rs`.
- Create: `crates/core/hacivat-driver-pg/tests/{wire,docker_exec}.rs`.
- Append: an ADR 0002 section, `## m2b-t10 — hacivat-driver-pg`.

**Interfaces:**
- Consumes:
  - Task 1's `RawConn` surface and `InsecureAuth`;
  - Task 3's `TlsParams.outcome` and `tls_server_end_point` (merged in W1);
  - Task 16's `pg17-auth` target. Its Docker tests run in W3 after Task 16 merges, so they are written now, marked `#[ignore = "docker"]`, and get their targets only through `require_target(..)`/`targets(..)` (Task 1; Q6): no literal target id outside a `require_target` call.
- Produces:
  - **`wire::Observer<S>`** wraps the stream that `tokio-postgres` reads.
    - It has states `AwaitSslResponse` (when SSL is requested), `Framed` and `Passthrough`.
    - It frames backend messages: one type byte, then an i32 big-endian length.
    - Until `AuthenticationOk` it inspects `R` messages. Code 3 or 5 with `InsecureAuth::RefuseUnlessVerified`, and a TLS outcome not in {Full, CaOnly, Pinned}, fails the read with an `io::Error` whose inner value is `InsecureAuthRefusedMarker`. That maps to `RawErrorKind::InsecureAuthRefused`, and no response is ever written.
    - It records the PID from `BackendKeyData` (`K`, after `AuthenticationOk`, before the first `Z`) into an `AtomicI32`; **`backend_id` reads it** (`tokio-postgres` exposes no public PID: `CancelToken.process_id` is `pub(crate)`; Q11).
    - After `AuthenticationOk` it records every `Z` status byte into an `Arc<AtomicU8>` that `tx_status()` reads, and **counts `Z` messages** (`AtomicU64`, with a `tokio::sync::Notify`). `execute`, `batch` and `reset` record the count before sending; after an `Err` they wait (bounded, 1 s) until the count advances, because `tokio-postgres` returns at `ErrorResponse` before `ReadyForQuery` is read. `tx_status()` is then exact (Q12).
    - **Layering (Q12):** `PgTlsStream(Observer<Framed>(tokio_rustls::client::TlsStream<Observer<AwaitSslResponse>(Injected)>))`. The inner observer sees only the 1-byte `S`/`N` SSL response, then becomes `Passthrough` after `S` or `Framed` after `N`; with `SslMode::Disable` the raw observer starts in `Framed`. Both share one state (`Arc`). Every `R` and `Z` after `S` is read on the **decrypted** side. `channel_binding()` is computed from the inner rustls connection's `peer_certificates()[0]`.
  - **`PgTlsStream::channel_binding()`** returns `ChannelBinding::tls_server_end_point(hash)` when `tls_server_end_point(leaf)` is `Some`.
  - **`execute`:**
    - `Protocol::Simple` → `simple_query`, with rows as text up to `max_rows`;
    - `Protocol::Extended` → `client.query_raw(sql, [])`, counting rows and dropping the values; the command tag comes from `RowStream::rows_affected`.
    - A notice SQLSTATE goes into `notice_codes`. The connection is driven with `futures::stream::poll_fn(|cx| connection.poll_message(cx))` (replacing M2a's spawned `connection.await`, which logged and dropped notices), and `AsyncMessage::Notice` goes into a bounded per-connection buffer (100 entries) that `execute` drains. No notice text is logged (Q12).
  - **`batch`** is `batch_execute`. **`backend_id`** is the PID the observer recorded (Q11).
  - **`reset` (Q10)** is the documented `DISCARD ALL` sequence **without `DEALLOCATE ALL`**, then a by-name deallocation of SQL-level prepared statements:
    - `CLOSE ALL; SET SESSION AUTHORIZATION DEFAULT; RESET ALL; UNLISTEN *; SELECT pg_advisory_unlock_all(); DISCARD PLANS; DISCARD TEMP; DISCARD SEQUENCES;`
    - then `SELECT name FROM pg_prepared_statements WHERE from_sql`, and `DEALLOCATE "<quoted>"` for each.

    `DEALLOCATE ALL` would drop `tokio-postgres`'s cached `typeinfo` statements (`client.rs:74-130`; `clear_type_cache()` does not drop them), and the next query on an enum, domain, composite, `citext`, `hstore` or PostGIS column would fail with `26000`. ADR 0004 records the equivalence with S-02/#pg-discard.
  - **`cancel_handle()`** returns `PgCancel { token, factory, connector }`. Its `cancel()` opens a stream from the connection's `StreamFactory` and calls `token.cancel_query_raw(stream, connector)` with the same TLS settings.

- [ ] **Step 1: Write the failing tests.**
```rust
// tests/wire.rs (no Docker)
#[test] fn observer_frames_one_byte_reads() { let mut o = observer(InsecureAuth::Allow, None); feed_bytewise(&mut o, &[auth_ok(), ready(b'T')].concat()); assert_eq!(o.tx(), TxStatus::InTx); }
#[test] fn observer_refuses_cleartext_and_md5_unless_verified() { for code in [3, 5] { for outcome in [None, Some(TlsVerified::No)] {
    let mut o = observer(InsecureAuth::RefuseUnlessVerified, outcome); let e = feed_err(&mut o, &auth_request(code)); assert!(is_insecure_marker(&e), "{code} {outcome:?}"); } } }
#[test] fn observer_allows_when_verified_or_allowed() { for outcome in [TlsVerified::Full, TlsVerified::CaOnly, TlsVerified::Pinned] { assert!(feed_ok(&mut observer(InsecureAuth::RefuseUnlessVerified, Some(outcome)), &auth_request(5))); }
    assert!(feed_ok(&mut observer(InsecureAuth::Allow, None), &auth_request(3))); }
#[test] fn observer_lets_scram_through_and_sees_ssl_response() { assert!(feed_ok(&mut observer(InsecureAuth::RefuseUnlessVerified, None), &auth_request(10)));
    let mut o = observer_ssl(InsecureAuth::RefuseUnlessVerified); feed_ok(&mut o, b"N"); assert!(is_insecure_marker(&feed_err(&mut o, &auth_request(3)))); }
#[test] fn failed_status_is_e() { let mut o = observer(InsecureAuth::Allow, None); feed_bytewise(&mut o, &[auth_ok(), ready(b'E')].concat()); assert_eq!(o.tx(), TxStatus::Failed); }
#[test] fn observer_records_backend_key_data_pid() { let mut o = observer(InsecureAuth::Allow, None); feed_bytewise(&mut o, &[auth_ok(), backend_key_data(4242, 7), ready(b'I')].concat());
    assert_eq!(o.backend_pid(), Some(4242)); assert_eq!(o.ready_count(), 1); }   // Q11, Q12
#[tokio::test] async fn observer_on_tls_path_sees_decrypted_messages() {   // TLS over a duplex: an R(5) after the handshake is refused (Q12)
    let (client, server) = tls_duplex_after_ssl_response(b'S').await; let e = read_through_pg_tls_stream(client, InsecureAuth::RefuseUnlessVerified, None, server.send(&auth_request(5))).await.unwrap_err();
    assert!(is_insecure_marker(&e)); }
// tests/docker_exec.rs (#[ignore = "docker"])
// every Docker test takes its target from require_target(..) (Q6): `let Some(t) = require_target("pg17") else { return };`
#[tokio::test] #[ignore = "docker"] async fn execute_simple_and_extended() { let mut c = pg("pg17").await;
    let s = c.execute("SELECT 1 AS a, 'ş' AS b", simple()).await.unwrap(); assert_eq!(s.rows, [vec![Some("1".into()), Some("ş".into())]]);
    let e = c.execute("SELECT generate_series(1, 5)", ExecOpts::default()).await.unwrap(); assert_eq!(e.row_count, 5); assert!(e.rows.is_empty()); }
#[tokio::test] #[ignore = "docker"] async fn extended_rejects_multiple_commands() { let mut c = pg("pg17").await;
    assert_eq!(c.execute("SELECT 1; SELECT 2", ExecOpts::default()).await.unwrap_err().server.unwrap().code.as_deref(), Some("42601")); }
#[tokio::test] #[ignore = "docker"] async fn tx_status_follows_protocol() { let mut c = pg("pg17").await; c.batch("BEGIN").await.unwrap(); assert_eq!(c.tx_status(), TxStatus::InTx);
    let _ = c.execute("SELECT 1/0", ExecOpts::default()).await; assert_eq!(c.tx_status(), TxStatus::Failed); c.batch("ROLLBACK").await.unwrap(); assert_eq!(c.tx_status(), TxStatus::Idle); }
#[tokio::test] #[ignore = "docker"] async fn tx_status_is_failed_right_after_error() { let mut c = pg("pg17").await; c.batch("BEGIN").await.unwrap();   // no sleep (Q12)
    assert!(c.execute("SELECT 1/0", ExecOpts::default()).await.is_err()); assert_eq!(c.tx_status(), TxStatus::Failed); }
#[tokio::test] #[ignore = "docker"] async fn rollback_outside_tx_reports_25p01_notice() { let mut c = pg("pg17").await; assert!(c.execute("ROLLBACK", simple()).await.unwrap().notice_codes.contains(&"25P01".to_string())); }
#[tokio::test] #[ignore = "docker"] async fn cancel_pg_sleep_within_one_second() { for t in ["pg17", "pg17-tls"].into_iter().filter_map(require_target) { let mut c = pg_on(&t).await; let h = c.cancel_handle().unwrap();
    let start = Instant::now(); let (r, _) = tokio::join!(c.execute("SELECT pg_sleep(60)", ExecOpts::default()), async { tokio::time::sleep(ms(200)).await; h.cancel().await });
    assert_eq!(r.unwrap_err().server.unwrap().code.as_deref(), Some("57014")); assert!(start.elapsed() < secs(2)); assert!(c.execute("SELECT 1", simple()).await.is_ok()); } }
#[tokio::test] #[ignore = "docker"] async fn reset_discards_all() { let mut c = pg("pg17").await;
    c.batch("PREPARE p AS SELECT 1; LISTEN ch; CREATE TEMP TABLE t(a int); SELECT pg_advisory_lock(1)").await.unwrap(); c.reset().await.unwrap();
    assert_eq!(scalar(&mut c, "SELECT count(*) FROM pg_prepared_statements WHERE from_sql").await, "0");   // Q10 assert_eq!(scalar(&mut c, "SELECT count(*) FROM pg_listening_channels()").await, "0");
    assert_eq!(scalar(&mut c, "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND pid = pg_backend_pid()").await, "0"); }
#[tokio::test] #[ignore = "docker"] async fn md5_and_cleartext_users_refused_without_verified_tls() { for user in ["hv_md5", "hv_clear"] {
    let e = pg_as("pg17-auth", user, InsecureAuth::RefuseUnlessVerified).await.unwrap_err(); assert_eq!(e.kind, RawErrorKind::InsecureAuthRefused, "{user}");
    assert!(pg_as("pg17-auth", user, InsecureAuth::Allow).await.is_ok(), "{user}"); } }
#[tokio::test] #[ignore = "docker"] async fn scram_channel_binding_require_succeeds_on_pg17_tls() { assert!(pg_with_channel_binding_required("pg17-tls").await.is_ok()); }
#[tokio::test] #[ignore = "docker"] async fn extended_query_on_enum_column_after_reset() { let mut c = pg("pg17").await;   // Q10
    c.batch("DROP TABLE IF EXISTS hv_mood_t; DROP TYPE IF EXISTS hv_mood; CREATE TYPE hv_mood AS ENUM ('ok', 'sad'); CREATE TABLE hv_mood_t(m hv_mood); INSERT INTO hv_mood_t VALUES ('ok')").await.unwrap();
    assert!(c.execute("SELECT m FROM hv_mood_t", ExecOpts::default()).await.is_ok()); c.reset().await.unwrap();
    assert!(c.execute("SELECT m FROM hv_mood_t", ExecOpts::default()).await.is_ok()); }
#[tokio::test] #[ignore = "docker"] async fn backend_id_is_pid() { let mut c = pg("pg17").await; assert_eq!(c.backend_id().unwrap().to_string(), scalar(&mut c, "SELECT pg_backend_pid()").await); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-driver-pg --test wire`. Expected: FAIL. The Docker file runs in Step 4 once `pg17-auth` exists; until then, its `pg17-auth` tests are written and listed as pending in the report.
- [ ] **Step 3: Implement** the bodies in Produces. The observer allocates nothing per message beyond a small header buffer. A frame length below 4 or above 1 GiB is `io::ErrorKind::InvalidData`.
- [ ] **Step 4:** Run `cargo test -p hacivat-driver-pg`, then `cargo xtask docker test --set pr -p hacivat-driver-pg`. Expected: PASS. The `pg17-auth` tests run after Task 16 merges: the controller re-runs this command in W3 and records the result in the ledger.
- [ ] **Step 5: Commit** with the message `feat(driver-pg): bounded execution, wire observer for tx status, backend pid and the insecure-auth guard, tls-server-end-point channel binding, cancel over the same transport, type-cache-safe session reset`.

**Review:** opus, adversarial. Probes:
- Can a server split an `R` header across reads to slip past the observer? (Byte-wise test.)
- Can it send `R` code 3 **after** a fake `AuthenticationOk` inside a single TCP segment?
- Does the observer desync on `CopyData` or large `DataRow` messages?
- Does the cancel request go over plaintext when the session is TLS?
- Is the notice buffer bounded (100 entries)?
- Can `reset` reach a `DEALLOCATE ALL` by any path (Q10)? Does an inner observer ever see ciphertext as frames (Q12)?

### Task 11: SQLite driver M2b: execution, interrupt, read-only hardening (B2, B21; ADR 0002 M2b notes)

**Files:**
- Modify: `crates/core/hacivat-driver-sqlite/src/{lib,open,probe,harden,exec}.rs`.
- Create: `crates/core/hacivat-driver-sqlite/tests/{harden,exec}.rs`.
- Append: an ADR 0002 section, `## m2b-t11 — hacivat-driver-sqlite`.

**Interfaces:**
- Consumes: Task 5's `sqlite_hardening` and `sqlite_authorize`; Task 1's `RawConn` surface.
- Produces:
  - **The read-only open** (`DriverConnect.read_only`) applies, in this order:
    1. `SQLITE_OPEN_READ_ONLY`;
    2. `DbConfig::SQLITE_DBCONFIG_DEFENSIVE`;
    3. `SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE` = false and `SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE` = false;
    4. `set_limit(SQLITE_LIMIT_ATTACHED, 0)`, plus the length, SQL-length and variable limits;
    5. `PRAGMA query_only = 1`;
    6. finally, the authorizer (`Connection::authorizer`). It maps rusqlite's `AuthAction` to `SqliteAction` and returns `sqlite_authorize`'s verdict.
  - **`execute`** prepares with rusqlite. A non-empty tail is refused (rusqlite's `MultipleStatement`). On a read-only connection `stmt.readonly()` must be true, otherwise `RawErrorKind::Db` with the `SQLITE_AUTH`-like reason `readonly-required`. Rows come back as text.
  - **`tx_status`** comes from `is_autocommit()`. **`reset`** is `Ok(())`. **`backend_id`** is `None`.
  - **`ExecSummary.command_tag`** is the first keyword in ASCII upper case, so the guard's `WriteSucceeded` post-check works (Q35).
  - **`cancel_handle()`** returns the `InterruptHandle` wrapper.
  - The authorizer is installed only on read-only connections. A read-write profile is unchanged.

- [ ] **Step 1: Write the failing tests.**
```rust
#[tokio::test] async fn read_only_open_applies_hardening() { let mut c = ro(db_with_table()).await;   // allowed by Q16's pragma list
    assert_eq!(text(&mut c, "PRAGMA query_only").await, "1"); assert!(!text(&mut c, "PRAGMA journal_mode").await.is_empty()); }   // M2a's probe still reads it
#[tokio::test] async fn attach_and_vacuum_into_leave_no_file() { let dir = tempdir(); let mut c = ro(db_in(&dir)).await;
    for sql in [format!("ATTACH '{}' AS n", dir.path().join("new.db").display()), format!("VACUUM INTO '{}'", dir.path().join("copy.db").display())] {
        assert!(c.execute(&sql, simple()).await.is_err(), "{sql}"); }
    assert!(!dir.path().join("new.db").exists() && !dir.path().join("copy.db").exists()); }
#[tokio::test] async fn ro_denials() { let mut c = ro(db_with_table()).await; for sql in ["PRAGMA query_only = 0", "INSERT INTO t VALUES (1)", "CREATE TEMP TABLE x(a)",
    "SELECT load_extension('x')", "PRAGMA writable_schema = 1", "BEGIN"] { assert!(c.execute(sql, simple()).await.is_err(), "{sql}"); }
    assert_eq!(text(&mut c, "SELECT count(*) FROM t").await, "1"); }
#[tokio::test] async fn full_tail_required() { let mut c = rw(db_with_table()).await; assert!(c.execute("SELECT 1; DELETE FROM t", simple()).await.is_err());
    assert_eq!(text(&mut c, "SELECT count(*) FROM t").await, "1"); }
#[tokio::test] async fn read_write_profile_has_no_authorizer() { let mut c = rw(db_with_table()).await; assert!(c.execute("INSERT INTO t VALUES (2)", simple()).await.is_ok()); }
#[tokio::test] async fn tx_status_from_autocommit() { let mut c = rw(db_with_table()).await; c.batch("BEGIN").await.unwrap(); assert_eq!(c.tx_status(), TxStatus::InTx);
    c.batch("COMMIT").await.unwrap(); assert_eq!(c.tx_status(), TxStatus::Idle); }
#[tokio::test] async fn interrupt_cancels_a_long_recursive_cte() { let mut c = rw(db_with_table()).await; let h = c.cancel_handle().unwrap(); let start = Instant::now();
    let (r, _) = tokio::join!(c.execute("WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r) SELECT count(*) FROM r", ExecOpts::default()),
        async { tokio::time::sleep(ms(200)).await; h.cancel().await });
    assert!(r.is_err()); assert!(start.elapsed() < secs(2)); assert!(c.execute("SELECT 1", simple()).await.is_ok()); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-driver-sqlite --test harden --test exec`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. Every action code rusqlite reports maps to a `SqliteAction`. Unmapped codes become `Other(code)`, and therefore Deny.
- [ ] **Step 4:** Run `cargo test -p hacivat-driver-sqlite`. Expected: PASS, with the M2a tests green.
- [ ] **Step 5: Commit** with the message `feat(driver-sqlite): bounded execution with full-tail and stmt_readonly checks, interrupt cancel, read-only hardening (defensive, attach off, LIMIT_ATTACHED 0, query_only, default-deny authorizer)`.

**Review:** opus, adversarial. Probes:
- Can `PRAGMA main.query_only = 0` get through?
- Does `ATTACH` via a `file:` URI with `mode=rwc` get through?
- Does a `VACUUM INTO` through an expression or a parameter get through?
- Can a function called `readfile` in another case (`ReadFile`) get through?
- Does `sqlite3_stmt_readonly` stay true for `ANALYZE`? (Analyze is denied anyway.)
- Do triggers on temp views get through?

### Task 12: Profiles, store and Validate for M2b (B10, B26, B30)

**Files:**
- Modify: `crates/core/hacivat-session/src/profile/{mod,store}.rs` and `crates/core/hacivat-session/src/pipeline/validate.rs`.
- Create: `crates/core/hacivat-session/tests/{profiles_m2b,validate_m2b}.rs`.

**Interfaces:**
- Consumes: Task 1's profile fields, `SshSpec`, `HopSpec`, `CertPin` and `IssueCode`, and Task 1's minimal `ssh_tunnel` parse/write in `store.rs` (an empty `SshSpec`), which this task replaces with the real `[ssh]` table (Q3).
- Produces:
  - **TOML** (B30). `[connections.ssh]` holds `host_keys`, `keepalive_s` and `[[connections.ssh.hops]]` (`host`, `port`, `user`, `auth` = `agent|key_file|password|keyboard_interactive`, `key_file`, `identities_only`, `identity_files`).
    - `tls.pins = [{ endpoint, sha256 = "<64 hex>" }]` and `tls.allow_public_plaintext`.
    - `single_session`, `tx_mode`, `isolation`, `idle_disconnect_s`, `ssh_retention`.
    - A key at its default is **not written** (verbatim round trip of M2a files).
    - A `password`, `passphrase` or `secret` key typed into `[ssh]` is ignored and reported, as in M2a T12.
  - **Validate:**
    - `SshHostMissing`, `SshUserMissing` and `SshKeyFileMissing` (on `KeyFile` without a path, or a path that does not exist);
    - `SshTooManyHops` when there are more than 9 hops;
    - Karagöz still gives `TransportUnavailable`;
    - VerifyCa is accepted for MySQL and MariaDB (`TlsModeUnsupported` stays as a variant but is no longer produced for them);
    - `TlsSetupError::KeyEncryptedLegacy` maps to `IssueCode::TlsKeyLegacyEncrypted`;
    - **every TLS, SQLite and SSH key path of an untrusted origin is checked against `ApprovedRoots` on canonical forms, per B26 (Q22):** path and roots both canonicalised; `\\?\C:\` stripped to `C:\` by a local helper; M2a's text and prefix rules on the typed path; a missing file through its parent directory; UNC, device and verbatim-UNC canonical forms refused.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn ssh_profile_round_trips() { let p = ssh_profile(vec![hop("j.acme.io", "u1", SshAuth::Agent), hop("b.acme.io", "deploy", key_file("/home/me/.ssh/id_ed25519"))]);
    let (s, _) = store_tmp(); s.save(&[p.clone()]).unwrap(); assert_eq!(reload(&s)[0], p); }
#[test] fn pins_round_trip_as_hex() { let p = with_pins(&[pin("db:5432", [0xAB; 32])]); let toml = save_to_string(&p); assert!(toml.contains(&"ab".repeat(32))); assert_eq!(load_from_string(&toml)[0].tls.pins, p.tls.pins); }
#[test] fn m2a_file_round_trips_verbatim() { let text = include_str!("fixtures/m2a_connections.toml"); assert_eq!(resave(text), text); }
#[test] fn unknown_ssh_keys_preserved_and_typed_password_ignored() { let r = load_report("[connections.ssh]\nfuture = 1\npassword = \"x\"\n");
    assert!(r.diags.iter().any(|d| matches!(d, ProfileDiag::PasswordIgnored { .. }))); assert!(resave_report(&r).contains("future = 1")); assert!(!resave_report(&r).contains("password")); }
#[test] fn ssh_transport_without_table_loads_and_validate_reports() { let p = load_from_string("[[connections]]\n...transport = \"ssh_tunnel\"\n")[0].clone();
    assert!(validate(&p, EngineCatalog::embedded()).iter().any(|i| i.code == IssueCode::SshHostMissing)); }
// tests/validate_m2b.rs
#[test] fn ssh_issues() { let mut p = ssh_profile(vec![hop("", "", SshAuth::Password)]); let codes = codes(&p); assert!(codes.contains(&IssueCode::SshHostMissing) && codes.contains(&IssueCode::SshUserMissing));
    p.transport = TransportSpec::SshTunnel(SshSpec { hops: vec![hop("h", "u", SshAuth::Agent); 10], ..Default::default() }); assert!(codes(&p).contains(&IssueCode::SshTooManyHops)); }
#[test] fn karagoz_still_transport_unavailable() { assert!(codes(&karagoz_profile()).contains(&IssueCode::TransportUnavailable)); }
#[test] fn mysql_verify_ca_is_accepted_now() { assert!(validate(&mysql_profile(TlsMode::VerifyCa), EngineCatalog::embedded()).is_empty()); }
#[cfg(unix)] #[test] fn canonicalize_before_roots_rejects_symlink_escape() { let home = tempdir(); let link = home.path().join("ca.pem"); std::os::unix::fs::symlink("/etc/hosts", &link).unwrap();   // Q5
    let p = imported(pg_with_ca(&link)); assert!(codes_with_roots(&p, &[home.path()]).contains(&IssueCode::PathRefused)); }
#[test] fn imported_ssh_key_outside_roots_refused() { let outside = tempdir(); let key = outside.path().join("id_ed25519"); std::fs::write(&key, "k").unwrap();   // portable (Q5)
    let home = tempdir(); let p = imported(ssh_profile(vec![hop("b", "u", key_file(key.to_str().unwrap()))])); assert!(codes_with_roots(&p, &[home.path()]).contains(&IssueCode::PathRefused)); }
#[cfg(target_os = "macos")] #[test] fn canonical_root_on_macos_temp_is_accepted() { let root = tempdir();   // /var/folders → /private/var (Q22)
    let f = root.path().join("ca.pem"); std::fs::write(&f, "x").unwrap(); assert!(!codes_with_roots(&imported(pg_with_ca(&f)), &[root.path()]).contains(&IssueCode::PathRefused)); }
#[cfg(windows)] #[test] fn windows_verbatim_disk_is_normalised() { let root = tempdir(); let f = root.path().join("ca.pem"); std::fs::write(&f, "x").unwrap();   // Q22
    assert!(!codes_with_roots(&imported(pg_with_ca(&f)), &[root.path()]).contains(&IssueCode::PathRefused)); }
#[test] fn missing_sqlite_file_is_checked_through_its_parent() { let root = tempdir(); let f = root.path().join("new.sqlite");   // Q22
    assert!(!codes_with_roots(&imported(sqlite_at(&f)), &[root.path()]).contains(&IssueCode::PathRefused)); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --test profiles_m2b --test validate_m2b`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces, with toml_edit as in M2a T12. The hex for `sha256` is lower-case on write and accepted in either case on read. That is ASCII only, so INV-3 is respected.
- [ ] **Step 4:** Run Step 2's command and `cargo test -p hacivat-session --test profiles`. Expected: PASS.
- [ ] **Step 5: Commit** with the message `feat(session): SSH, pins, plaintext consent and session defaults in connections.toml; Validate for SSH, VerifyCa on MySQL, canonical paths before approved roots`.

**Review:** opus. Probes:
- Does a hand-edited `[ssh]` with a secret survive a save?
- Does a pin hex of the wrong length load?
- Is there a TOCTOU between canonicalize and use? (Record it: the driver reopens by path; the risk is accepted and noted in ADR 0003.)

### Task 13: MySQL/MariaDB driver M2b: execution, status flags, read-only capabilities, Hacivat's TLS config, outcome-based password guard (B2, B3, B5, B8, B10, B21; Q2, Q20)

**Files:**
- Modify: `crates/core/hacivat-driver-mysql/src/{lib,connect,tls,probe,exec}.rs` and `crates/core/hacivat-driver-mysql/Cargo.toml` (remove `rustls-native-certs` only; the named exception in Global Constraints).
- Create: `crates/core/hacivat-driver-mysql/tests/docker_exec.rs`.
- Append: an ADR 0002 section, `## m2b-t13 — hacivat-driver-mysql`, which supersedes the P11 defect note and records the removal of the OS-roots path (MySQL now uses the platform verifier, as PostgreSQL does).

**Interfaces:**
- Consumes: Task 8's patches §1–§5; Task 3's `TlsParams` (pin-aware verifier, `presented`, `outcome`, decrypted client identity); Task 1's `RawConn` surface; Task 16's targets, through `require_target`/`targets(Some(engine))` only (Q6).
- Produces:
  - **`opts_for(req)`:**
    - **TLS (Q2):** `ssl_opts` = `SslOpts::default().with_rustls_client_config(tls.config.clone()).with_danger_tls_hostname_override(Some(tls_host))`. Every mode (Require, VerifyCa, VerifyFull, Auto) goes through Hacivat's `TlsParams.config`, so pins, the presented-chain capture for review, `TlsParams.outcome`, the platform verifier, the CA file and the client key Hacivat decrypted (encrypted PKCS #8, B25) all apply to MySQL and MariaDB. The M2a `rustls-native-certs` roots path and the VerifyCa `skip_domain_validation` path are removed; VerifyCa runs through Task 3's `ChainOnly` verifier.
    - **Password guard (Q2 §2, Q20):** `password_guard(Some(Arc::new(move || req.insecure_auth == InsecureAuth::Allow || matches!(*outcome.lock().unwrap(), Some(TlsVerified::Full | TlsVerified::CaOnly | TlsVerified::Pinned)))))`: outcome-based, never mode-based, so Require with a pin is allowed.
    - `multi_statements(!req.read_only)`;
    - `local_infile_capability(false)` always;
    - `enable_cleartext_plugin(false)` (M2a, unchanged).
  - **`execute`:**
    - `Simple` → `query_iter`, with rows as text;
    - `Extended` → `exec_iter(sql, ())`, a prepared statement. `ER_UNSUPPORTED_PS` (1295) on a read-only connection is `RawErrorKind::Unsupported` and never falls back to `COM_QUERY`. On a read-write connection it falls back to `query_iter`.
    - `tx` comes from `last_ok_packet().status_flags()`, and an `ERR` keeps the previous value.
    - **`ExecSummary.command_tag`** is the statement's first keyword in ASCII upper case (MySQL has no command tags), so the guard's `WriteSucceeded` post-check works (Q35). Task 11 does the same for SQLite.
  - **`reset`** is `conn.reset()` (`COM_RESET_CONNECTION`), then `SET SESSION TRANSACTION READ ONLY` again when read-only. **`backend_id`** is `conn.id()`.
  - **`DriverError::InsecurePasswordRefused`** maps to `RawErrorKind::InsecureAuthRefused`. Errors 1317 (query interrupted) and 3024 (`max_execution_time`) map to `RawErrorKind::Cancelled`.
  - **Docker fixtures (Q7):** the test helpers create their fixtures idempotently through the admin connection: `ensure_probe_table` (`hv_probe`) and `ensure_ro_role` (`CREATE USER IF NOT EXISTS 'hv_ro'@'%' … ; GRANT SELECT, SHOW VIEW ON hv_test.* TO 'hv_ro'@'%'`), so the tests do not depend on Task 16's init scripts having run on a fresh volume.

- [ ] **Step 1: Write the failing tests** (Docker; family loops use `targets(Some(DbmsKind::MySql))` and `targets(Some(DbmsKind::MariaDb))`, single targets use `require_target(..)`; Q6).
```rust
#[tokio::test] #[ignore = "docker"] async fn execute_and_status_flags() { for_each_mysql_family(|mut c| async move { c.batch("START TRANSACTION").await.unwrap(); assert_eq!(c.tx_status(), TxStatus::InTx);
    let _ = c.execute("SELECT * FROM no_such_table", simple()).await; assert_eq!(c.tx_status(), TxStatus::InTx); c.batch("ROLLBACK").await.unwrap(); assert_eq!(c.tx_status(), TxStatus::Idle); }).await; }
#[tokio::test] #[ignore = "docker"] async fn command_tag_is_the_first_keyword() { let mut c = mysql("mysql84").await; ensure_probe_table(&admin("mysql84").await).await;   // Q35
    assert_eq!(c.execute("insert into hv_probe values (1)", simple()).await.unwrap().command_tag.as_deref(), Some("INSERT")); }
#[tokio::test] #[ignore = "docker"] async fn read_only_connection_has_no_multi_statements() { ensure_ro_role(&admin("mysql84").await).await; let mut c = mysql_ro("mysql84").await;
    assert!(c.execute("SELECT 1; DROP TABLE hv_probe", simple()).await.is_err()); assert!(table_exists(&admin("mysql84").await, "hv_probe").await); }
#[tokio::test] #[ignore = "docker"] async fn read_only_extended_never_falls_back() { ensure_ro_role(&admin("mysql84").await).await; let mut c = mysql_ro("mysql84").await;
    assert_eq!(c.execute("PREPARE s FROM 'SELECT 1'", ExecOpts::default()).await.unwrap_err().kind, RawErrorKind::Unsupported); }
#[tokio::test] #[ignore = "docker"] async fn kill_query_side_connection_cancels_sleep() { let mut a = mysql("mysql84").await; let id = a.backend_id().unwrap(); let mut b = mysql("mysql84").await; let start = Instant::now();
    let (r, _) = tokio::join!(a.execute("SELECT SLEEP(60)", simple()), async { tokio::time::sleep(ms(200)).await; b.batch(&format!("KILL QUERY {id}")).await });
    assert!(matches!(r, Ok(_) | Err(RawError { kind: RawErrorKind::Cancelled, .. }))); assert!(start.elapsed() < secs(2)); assert!(a.execute("SELECT 1", simple()).await.is_ok()); }   // Q32
#[tokio::test] #[ignore = "docker"] async fn reset_connection_clears_user_vars_and_temp_tables() { let mut c = mysql("mysql84").await;
    c.batch("USE hv_test").await.unwrap(); c.batch("SET @x = 1").await.unwrap(); c.batch("CREATE TEMPORARY TABLE tt(a int)").await.unwrap(); c.reset().await.unwrap();
    assert_eq!(text(&mut c, "SELECT @x IS NULL").await, "1"); assert!(c.execute("SELECT * FROM tt", simple()).await.is_err());
    assert_eq!(text(&mut c, "SELECT DATABASE()").await, "hv_test"); }   // Q15: the default database survives COM_RESET_CONNECTION
#[tokio::test] #[ignore = "docker"] async fn get_lock_after_reset_is_released() { let mut c = mysql("mysql84").await; c.batch("DO GET_LOCK('hv', 0)").await.unwrap(); c.reset().await.unwrap();
    let free = text(&mut admin("mysql84").await, "SELECT IS_FREE_LOCK('hv')").await; assert_eq!(free, "1"); }   // Q32: the documented behaviour; on failure ADR 0004 records it
#[tokio::test] #[ignore = "docker"] async fn verify_ca_with_container_ca_skips_the_name() { assert!(mysql_tls("mysql84", TlsMode::VerifyCa, Some(artifact("mysql84-ca.pem"))).await.is_ok()); }
#[tokio::test] #[ignore = "docker"] async fn pin_flow_on_mysql84() {   // Q2: the presented chain is captured on UnknownIssuer; a retry with the pin gives Pinned
    let (e, params) = mysql_tls_keep("mysql84", auto_with(&[])).await; assert_eq!(e.unwrap_err().kind, RawErrorKind::Tls(TlsFailure::UnknownIssuer));
    let pin = pin_from(&params, "127.0.0.1", port_of("mysql84")); let (r, p2) = mysql_tls_keep("mysql84", auto_with(&[pin])).await;
    assert!(r.is_ok()); assert_eq!(*p2.outcome.lock().unwrap(), Some(TlsVerified::Pinned)); }
#[tokio::test] #[ignore = "docker"] async fn encrypted_pkcs8_client_key() {   // Q2: the ClientConfig carries the client cert Hacivat decrypted
    let params = tls_params_with_encrypted_client_key("hv_test_pw"); assert!(params.config.client_auth_cert_resolver.has_certs());
    assert!(mysql_with_params("mysql84", params).await.is_ok()); }   // a `REQUIRE X509` user is optional; the resolver assertion is mandatory
#[tokio::test] #[ignore = "docker"] async fn insecure_password_refused_over_plaintext_full_auth() { flush_privileges(&mut admin("mysql84").await).await;
    let e = mysql_as("mysql84", TlsMode::Disable, InsecureAuth::RefuseUnlessVerified).await.unwrap_err(); assert_eq!(e.kind, RawErrorKind::InsecureAuthRefused);
    assert!(mysql_as("mysql84", TlsMode::Disable, InsecureAuth::Allow).await.is_ok()); }
#[tokio::test] #[ignore = "docker"] async fn require_with_pin_full_auth_is_allowed() { flush_privileges(&mut admin("mysql84").await).await;   // Q20: outcome-based
    let pin = pin_of_target("mysql84").await; assert!(mysql_as_with(&"mysql84", TlsSettings { mode: TlsMode::Require, ..auto_with(&[pin]) }, InsecureAuth::RefuseUnlessVerified).await.is_ok()); }
#[test] fn password_guard_matrix() { for (mode, outcome, ia, want) in insecure_matrix() { assert_eq!(guard_allows(mode, outcome, ia), want, "{mode:?} {outcome:?} {ia:?}"); } }   // every mode × outcome × InsecureAuth
```
- [ ] **Step 2:** Run `cargo xtask docker test --set full -p hacivat-driver-mysql` and `cargo test -p hacivat-driver-mysql`. Expected: FAIL. **Coordination (Q7):** this Docker run happens before Task 16 Step 5 recreates the stack; tell the controller when it is finished. The controller re-runs this task's Docker set after Task 16 merges.
- [ ] **Step 3: Implement** the bodies in Produces. Remove the M2a `TlsModeUnsupported` refusal in the driver and the `rustls-native-certs` path, and drop the `#[ignore]` from Task 8's superseded defect test (it now asserts success).
- [ ] **Step 4:** Run Step 2's commands. Expected: PASS.
- [ ] **Step 5: Commit** with the message `feat(driver-mysql): bounded execution, status-flag tx state, read-only without multi-statements, COM_RESET_CONNECTION, Hacivat's rustls config (pins, review, VerifyCa, encrypted keys), outcome-based password guard`.

**Review:** opus, adversarial. Probes:
- Is a prepared `CALL` executed in read-only mode? (Session RO and privileges must stop its writes. Test with `hv_ro`.)
- Does a `max_execution_time` interruption leave the connection usable?
- Is the password guard ever `true` when the outcome is `No` or `None` and `InsecureAuth` is `RefuseUnlessVerified`? (`password_guard_matrix`.)
- Does any connect path still build `mysql_async`'s own `ClientConfig` (roots, `DangerousVerifier`)?

### Task 14: Pipeline M2b: interactive TLS, the plaintext question, the SSH transport step, secrets and prompts, the insecure-auth policy, new error classes (HV-CONN-01, -03; B5–B8, B11, B19, B24, B25)

**Files:**
- Modify:
  - `crates/core/hacivat-session/src/pipeline/{mod,steps,ssh_step,prompts}.rs`;
  - `crates/core/hacivat-driver/src/errors.rs` and `crates/core/hacivat-driver/catalog/errors.toml`;
  - `locales/{en,tr}/hacivat-core.ftl` (a block `## m2b-t14-pipeline`, plus the named exception);
  - `crates/core/hacivat-session/src/test_conn.rs` (Test never shares tunnels).
- Create: `crates/core/hacivat-session/tests/{pipeline_tls,pipeline_ssh,pipeline_secrets}.rs`.
- Append: an ADR 0003 section, `## m2b-t14 — pipeline policies`.

**Interfaces:**
- Consumes:
  - Task 3 (pins, review, encrypted keys); Task 6 (`as_vault`); Task 9 (`TunnelManager`, `SshPrompter`); Task 12 (profile and Validate);
  - Task 1's `Question`/`Answer`, `ProfileChange`, `ConnectError` fields and `InsecureAuth`;
  - M2a's `UiPort`.
- Produces:
  - **Transport step (order 40).**
    - With `TransportSpec::SshTunnel(spec)`:
      1. Hop secrets: `ConnectRequest.overlay_secrets` first (reconnects and side connections, Q17), then `ConnectRequest.draft_ssh` (Test from the dialog, Q23), then the store (`SshPassword { hop }`, `SshPassphrase { hop }`, with `ssh_retention`) on `spawn_blocking`, outside the deadline.
      2. `TunnelManager::acquire(.., share = purpose != Test, prompter = PipelinePrompter)`. The `PipelinePrompter` maps SSH prompts to `Question::{HostKey, Secret, Keyboard}` through the `Interaction`; Headless has no prompter. `acquire` is **not** wrapped in `clock.run`: hop network timeouts come from `TunnelRequest.connect_timeout`, and prompt time never counts toward the deadline (P23, Q44).
      3. The factory is `lease.factory(db_host, db_port)`, and `Endpoint::Network.direct = None`, so MySQL goes through the forwarder.
      4. The trail gets `Ssh`. `ResolveStep::applies` is `false` for `TransportSpec::SshTunnel`: no local DNS lookup of the database host (Q44).
      5. The lease becomes a `CleanupGuard`. `LiveConnection.transport`, `path = Ssh` and `via = lease.bastion()` are set.
    - Prompted SSH secrets are stored only after the whole connect succeeds (R8).
    - Every secret that worked (DB password, hop secrets, TLS key passphrase) is returned in `LiveConnection.secrets_used`, and the one-gesture plaintext consent in `LiveConnection.plaintext_consent`, for Task 15's session overlay (Q17).
  - **TLS step (order 50).**
    - `tls_host` is the database host. The class is `classify_host` for Direct and the B11 rule for SSH.
    - With a client key, `TlsKeyPassphrase` is read (blocking, outside the deadline). It is asked through `Question::Secret(TlsKeyPassphrase)` when missing and the key is encrypted.
    - Pins come from the profile. A pinned endpoint never gets `plaintext_allowed` (Task 3, Q13).
    - The same `TlsParams` serves MySQL through patch §5 (Q2): this step is engine-neutral.
  - **Insecure-auth policy** (B8): `insecure_auth_for(profile, class, mode) -> InsecureAuth`.
  - **DriverConnect step (order 70).**
    - On `tls.untrusted`, Interactive: build the review from `TlsParams.presented`, ask `Question::Certificate`, and then:
      - `Pin` → push `AddPin`, rebuild `TlsParams` with the pin, and retry once;
      - `ImportCa(p)` → push `SetCaFile(p)`, `ApprovedRoots::add(p)`, rebuild and retry once;
      - `Cancelled` → the error with `certificate = Some(review)`.
    - On `PinMismatch` → `tls.pin_changed`, with `certificate` holding both fingerprints. Never asked. **The pipeline never produces `ProfileChange::ReplacePin`**; only `cert_dialog::review` builds it, from the error (Task 19, Q26).
    - On `tls.not_supported_by_server` for a **pinned endpoint** → the error with fixes `[EditConnection]`, never asked (Q13).
    - On `tls.not_supported_by_server` under Auto, Interactive → `Question::PlaintextPublic` (skipped when `ConnectRequest.plaintext_consent` is set: the retry uses it directly, Q17):
      - `ConnectPlaintext { remember }` → retry once with `plaintext_allowed = true` and `InsecureAuth::Allow`, plus `AllowPublicPlaintext` when `remember`;
      - `UseSsh` → the error with fixes `[ConnectThroughSsh, EditConnection]`.
    - At most one retry per question kind per connect.
    - On `auth.insecure_method` when TLS was up, `ConnectError.certificate` carries the presented chain's review, so "Review certificate" can pin; after the pin the outcome is `Pinned` and the guard allows the password (Q20).
  - **Secrets resolution** (`resolve_auth` stays). When the backend is a locked vault, Interactive asks `Question::UnlockVault { attempt }` up to 3 times. Then:
    - `Master` → `vault.unlock`;
    - `WrongMasterPassword` → the next attempt;
    - `Cancelled`, or 3 failures → continue as if nothing is stored, which leads to the password prompt.

    **Creating the vault (Q38).** When a prompted Forever secret is stored after success (`remember_password`) and `SecretStore::vault()` reports a vault that does not exist, Interactive asks `Question::CreateVault`; `Master` → `vault.create` on the blocking pool, then the put; `Cancelled` → the secret is kept Until restart (M2a toast). This is the common Linux path. The dialog's save path is Task 21's (`ensure_vault_before_forever`). Headless → never asks; the put falls back. `VaultLocked`, `VaultReadOnly` and `VaultCorrupt` on any Forever put fall back to `UntilRestart` with a reason (Task 6), never a hard error.
  - **`ProfileChange`s** are returned in `LiveConnection.profile_changes`, and also in `ConnectError.profile_changes` when the connect fails later.
  - **`classify_ssh` table:**

    | `SshError` | Class |
    |---|---|
    | `HostKeyUnknown` | `ssh.host_key_unknown` |
    | `HostKeyChanged` | `ssh.host_key_changed` (with `ConnectError.host_key`) |
    | `HostKeyRevoked` | `ssh.host_key_revoked` |
    | `Auth` | `ssh.auth` |
    | `AgentUnavailable` | `ssh.agent_unavailable` |
    | `ChannelOpen` | `ssh.channel_open` |
    | `Unreachable { net }` | the class `net` already maps to |
    | `Timeout` | `network.timeout` |
    | `Cancelled` | `cancelled` |
    | `Protocol` | `network.reset` |

    The hop arguments are `ErrArgs.hop` and `hop_host`.
  - **`errors.toml`:**
    - mark reachable: every `ssh.*`, `tls.pin_changed` and `auth.insecure_method`;
    - fixes:
      - `tls.untrusted` = `["review_certificate", "import_ca", "edit_connection"]`;
      - `tls.pin_changed` = `["review_certificate"]`;
      - `auth.insecure_method` = `["review_certificate", "edit_connection"]`;
      - `ssh.host_key_changed` = `["review_host_key"]`;
      - `ssh.host_key_unknown` = `["try_again", "edit_connection"]`;
      - `tls.not_supported_by_server` = `["connect_through_ssh", "edit_connection"]`;
      - every other `ssh.*` = `["try_again", "edit_connection"]`.
  - **`hacivat-core.ftl`:** `error-conn-<class>-title` and `-body` for every newly reachable class, in EN and TR, with `# Review: PO`. The copy comes from Task 2's NOTES. The bodies use `$hop_host` and `$hop`, never server text. `error-conn-tls-untrusted-body` returns to the mock's `err-tls-b` (the named exception).

- [ ] **Step 1: Write the failing tests** (FakeDriver, the in-process SSH server, and a scripted `UiPort` with `ScriptedUi::answers_m2b([...])`).
```rust
// tests/pipeline_tls.rs
#[tokio::test] async fn untrusted_auto_interactive_asks_certificate_and_pins() { let f = FakeDriver::new(DbmsKind::Postgres).presented(vec![leaf_der()])
        .script(vec![Err(tls(TlsFailure::UnknownIssuer)), Ok(())]); let ui = ScriptedUi::answers_m2b([Answer::Pin]);
    let live = connect_ix(&f, pg_auto("db", 5432), &ui).await.unwrap(); assert!(matches!(ui.asked()[0], Question::Certificate { .. }));
    assert_eq!(live.profile_changes, [ProfileChange::AddPin(CertPin { endpoint: "db:5432".into(), sha256: cert_sha256(&leaf_der()) })]);
    assert_eq!(f.last_request().unwrap().tls.unwrap().settings.pins.len(), 1); assert_eq!(f.connect_calls(), 2); }
#[tokio::test] async fn import_ca_answer_sets_ca_file_and_approves_root() { let (f, ui) = untrusted_then_ok(Answer::ImportCa(home_file("corp-ca.pem")));
    let live = connect_ix(&f, pg_auto("db", 5432), &ui).await.unwrap(); assert_eq!(live.profile_changes, [ProfileChange::SetCaFile(home_file("corp-ca.pem"))]); }
#[tokio::test] async fn cancelled_certificate_question_returns_review_in_error() { let (f, ui) = untrusted_then_ok(Answer::Cancelled);
    let e = connect_ix(&f, pg_auto("db", 5432), &ui).await.unwrap_err(); assert_eq!(e.class, ErrClass::TlsUntrusted); assert!(e.certificate.is_some());
    assert_eq!(e.fixes, [FixId::ReviewCertificate, FixId::ImportCa, FixId::EditConnection]); }
#[tokio::test] async fn headless_untrusted_never_asks() { let (f, _) = untrusted_then_ok(Answer::Pin); assert_eq!(connect_headless(&f, pg_auto("db", 5432)).await.unwrap_err().class, ErrClass::TlsUntrusted); assert_eq!(f.connect_calls(), 1); }
#[tokio::test] async fn pin_changed_is_hard_stop_with_both_fingerprints() { let f = FakeDriver::new(DbmsKind::Postgres).presented(vec![leaf_der()]).connect_result(Err(tls(TlsFailure::PinMismatch)));
    let e = connect_ix(&f, pinned(pg_auto("db", 5432), [1; 32]), &ScriptedUi::answers_m2b([])).await.unwrap_err();
    assert_eq!(e.class, ErrClass::TlsPinChanged); assert_eq!(e.certificate.unwrap().pinned, Some([1; 32])); }
#[tokio::test] async fn pinned_endpoint_tls_not_offered_is_not_asked() { let f = FakeDriver::new(DbmsKind::Postgres).connect_result(Err(tls_not_offered()));   // Q13
    let ui = ScriptedUi::answers_m2b([]); let e = connect_ix(&f, pinned(pg_auto("203.0.113.10", 5432), [1; 32]), &ui).await.unwrap_err();
    assert_eq!(e.class, ErrClass::TlsNotSupportedByServer); assert_eq!(e.fixes, [FixId::EditConnection]); assert!(ui.asked().is_empty()); }
#[tokio::test] async fn public_plaintext_asks_and_retries_once() { let f = FakeDriver::new(DbmsKind::Postgres).script(vec![Err(tls_not_offered()), Ok(())]);   // renamed (Q44): T10's Require path proves "not sent"
    let ui = ScriptedUi::answers_m2b([Answer::ConnectPlaintext { remember: true }]); let live = connect_ix(&f, pg_auto("203.0.113.10", 5432), &ui).await.unwrap();
    let reqs = f.requests(); assert!(!reqs[0].tls.as_ref().unwrap().plaintext_allowed); assert!(reqs[1].tls.as_ref().map_or(true, |t| t.plaintext_allowed));
    assert_eq!(reqs[1].insecure_auth, InsecureAuth::Allow); assert_eq!(live.profile_changes, [ProfileChange::AllowPublicPlaintext]); }
#[tokio::test] async fn use_ssh_answer_returns_connect_through_ssh_fix() { let f = FakeDriver::new(DbmsKind::Postgres).connect_result(Err(tls_not_offered()));
    let e = connect_ix(&f, pg_auto("203.0.113.10", 5432), &ScriptedUi::answers_m2b([Answer::UseSsh])).await.unwrap_err(); assert_eq!(e.fixes, [FixId::ConnectThroughSsh, FixId::EditConnection]); }
#[test] fn insecure_auth_policy_matrix() { use InsecureAuth::*; for (host, mode, consent, want) in [("127.0.0.1", TlsMode::Auto, false, Allow), ("10.0.0.5", TlsMode::Disable, false, Allow),
    ("203.0.113.10", TlsMode::Auto, true, Allow), ("10.0.0.5", TlsMode::Auto, false, RefuseUnlessVerified), ("203.0.113.10", TlsMode::VerifyFull, false, RefuseUnlessVerified)] {
    assert_eq!(insecure_auth_for(&pg_with(host, mode, consent), classify_host(host, &[]), mode), want, "{host} {mode:?}"); } }
#[tokio::test] async fn insecure_method_class_and_fixes() { let e = err_with(&FakeDriver::new(DbmsKind::Postgres).connect_result(Err(raw(RawErrorKind::InsecureAuthRefused))), pg_auto("10.0.0.5", 5432)).await;
    assert_eq!(e.class, ErrClass::AuthInsecureMethod); assert_eq!(e.fixes, [FixId::ReviewCertificate, FixId::EditConnection]); }
#[tokio::test] async fn insecure_method_keeps_certificate_when_tls_was_up() {   // Q20
    let f = FakeDriver::new(DbmsKind::Postgres).presented(vec![leaf_der()]).tls_outcome(TlsVerified::No).connect_result(Err(raw(RawErrorKind::InsecureAuthRefused)));
    let e = err_with(&f, pg_require("10.0.0.5", 5432)).await; assert_eq!(e.class, ErrClass::AuthInsecureMethod); assert!(e.certificate.is_some()); }
#[tokio::test] async fn pipeline_never_produces_replace_pin() { let f = FakeDriver::new(DbmsKind::Postgres).presented(vec![leaf_der()]).connect_result(Err(tls(TlsFailure::PinMismatch)));   // Q26
    let e = connect_ix(&f, pinned(pg_auto("db", 5432), [1; 32]), &ScriptedUi::answers_m2b([Answer::Pin])).await.unwrap_err();
    assert!(!e.profile_changes.iter().any(|c| matches!(c, ProfileChange::ReplacePin { .. }))); }
#[test] fn tls_untrusted_never_offers_encrypt_without_verifying() { assert!(!fixes_for(ErrClass::TlsUntrusted).contains(&FixId::EncryptWithoutVerifying)); }
// tests/pipeline_ssh.rs
#[tokio::test] async fn ssh_profile_trail_is_ssh_connect_tls_signin_check() { let s = ssh(cfg().user_pw("u", "pw")).await; let f = FakeDriver::new(DbmsKind::Postgres).consumes_factory();
    let live = connect_ssh(&f, &s, pg_auto("db.internal", 5432), Answer::TrustHostKey, "pw").await.unwrap();
    assert_eq!(steps(&live), [TrailStep::Ssh, TrailStep::Connect, TrailStep::Tls, TrailStep::SignIn, TrailStep::CheckServer]); assert_eq!(live.path, PathKind::Ssh); }
#[tokio::test] async fn tls_through_ssh_sends_the_database_host_as_sni() { let s = ssh(cfg().user_pw("u", "pw")).await; let f = FakeDriver::new(DbmsKind::Postgres).consumes_factory();
    connect_ssh(&f, &s, pg_auto("db.internal", 5432), Answer::TrustHostKey, "pw").await.unwrap();
    assert_eq!(f.last_request().unwrap().tls.unwrap().server_name, "db.internal"); }   // a String in FakeTls (Q44)
#[tokio::test] async fn ssh_profile_never_resolves_the_database_host_locally() { let s = ssh(cfg().user_pw("u", "pw").forward_to(ForwardTarget::Redirect(echo_pg_like().await))).await;   // Q44
    let live = connect_ssh(&FakeDriver::new(DbmsKind::Postgres).consumes_factory(), &s, pg_auto("db.internal", 5432), Answer::TrustHostKey, "pw").await.unwrap();
    assert!(!steps(&live).contains(&TrailStep::Resolve)); assert_eq!(local_dns_lookups(), 0); }
#[tokio::test] async fn host_key_prompt_time_does_not_count_toward_the_deadline() { let s = ssh(cfg().user_pw("u", "pw")).await;   // P23, Q44
    let ui = ScriptedUi::answers_m2b([Answer::TrustHostKey, Answer::Password(sec("pw"))]).delay(secs(3));
    assert!(connect_ssh_with_deadline(&FakeDriver::new(DbmsKind::Postgres).consumes_factory(), &s, pg_auto("db.internal", 5432), &ui, secs(2)).await.is_ok()); }
#[test] fn ssh_address_class_names_are_private() { assert_eq!(ssh_address_class("db.internal"), AddressClass::Private); assert_eq!(ssh_address_class("localhost"), AddressClass::Loopback);
    assert_eq!(ssh_address_class("8.8.8.8"), AddressClass::Public); }
#[tokio::test] async fn ssh_failure_names_the_hop() { let (a, b) = jump_pair().await; let e = connect_chain_bad_pw_at(&a, &b, 1).await.unwrap_err();
    assert_eq!(e.class, ErrClass::SshAuth); assert_eq!(e.args.hop, Some(1)); assert_eq!(e.args.hop_host.as_deref(), Some(b.host().as_str())); }
#[tokio::test] async fn host_key_changed_error_carries_mismatch() { let s = ssh(cfg().user_pw("u", "pw")).await; let e = connect_ssh_known_other(&s).await.unwrap_err();
    assert_eq!(e.class, ErrClass::SshHostKeyChanged); assert_eq!(e.host_key.unwrap().new, s.host_key); }
#[tokio::test] async fn mysql_over_ssh_uses_forwarder() { let s = ssh(cfg().user_pw("u", "pw")).await; let live = connect_ssh(&FakeDriver::new(DbmsKind::MySql), &s, mysql_auto("db", 3306), Answer::TrustHostKey, "pw").await.unwrap();
    assert!(live.forwarded()); }
#[tokio::test] async fn test_connection_with_ssh_does_not_share_tunnel() { let (m, s) = manager_ssh().await; connect_and_wait(&m).await; let _ = test_with_ssh(&m).await; assert_eq!(s.sessions_open(), 2); }
// tests/pipeline_secrets.rs
#[tokio::test] async fn prompted_ssh_secret_stored_only_after_success() { let s = ssh(cfg().user_pw("u", "pw")).await;
    let deps = deps_ssh(&s, FakeDriver::new(DbmsKind::Postgres).consumes_factory().script(vec![Err(db("3D000")), Ok(())]));   // first connect fails after SSH succeeded
    let _ = connect(&deps, req(ssh_forever(&s)), ui_secret("pw"), token(), progress()).await; assert!(stored(&deps, SecretSlot::SshPassword { hop: 0 }).is_none());
    connect(&deps, req(ssh_forever(&s)), ui_secret("pw"), token(), progress()).await.unwrap(); assert_eq!(stored(&deps, SecretSlot::SshPassword { hop: 0 }).as_deref(), Some("pw")); }
#[tokio::test] async fn ssh_secret_prompts_per_hop() { let (a, b) = jump_pair_with_encrypted_keys("pa", "pb").await; let ui = ScriptedUi::answers_m2b([Answer::TrustHostKey, Answer::Secret(sec("pa")), Answer::TrustHostKey, Answer::Secret(sec("pb"))]);
    connect_chain(&a, &b, &ui).await.unwrap(); assert_eq!(secret_prompt_hops(&ui), [0, 1]); }
#[tokio::test] async fn vault_locked_asks_unlock_then_reads() { let (deps, vault) = deps_with_locked_vault("correct horse", "pw"); let ui = ScriptedUi::answers_m2b([Answer::Master(sec("wrong horse")), Answer::Master(sec("correct horse"))]);
    connect(&deps, req(forever_profile()), ui.ix(), token(), progress()).await.unwrap(); assert_eq!(unlock_attempts(&ui), [1, 2]); assert!(vault.is_unlocked()); }
#[tokio::test] async fn three_wrong_master_passwords_fall_back_to_password_prompt() { let (deps, vault) = deps_with_locked_vault("correct horse", "pw");   // Q33
    let ui = ScriptedUi::answers_m2b([Answer::Master(sec("w1")), Answer::Master(sec("w2")), Answer::Master(sec("w3")), Answer::Password(sec("pw"))]);
    connect(&deps, req(forever_profile()), ui.ix(), token(), progress()).await.unwrap(); assert_eq!(unlock_attempts(&ui), [1, 2, 3]);
    assert!(matches!(ui.asked()[3], Question::Password { .. })); assert!(!vault.is_unlocked()); }
#[tokio::test] async fn first_forever_secret_on_vault_machine_asks_create_vault() { let deps = deps_with_absent_vault();   // Q38
    let ui = ScriptedUi::answers_m2b([Answer::Password(sec("pw")), Answer::Master(sec("correct horse"))]);
    connect(&deps, req(forever_profile()), ui.ix(), token(), progress()).await.unwrap(); assert!(matches!(ui.asked()[1], Question::CreateVault)); assert!(vault_of(&deps).exists()); }
#[tokio::test] async fn overlay_secrets_are_used_before_the_store_and_returned() { let deps = deps_never_retention();   // Q17
    let live = connect(&deps, req(never_profile()).overlay_secrets(db_secret("pw")), Interaction::Headless, token(), progress()).await.unwrap();
    assert_eq!(expose(live.secrets_used.db.as_ref().unwrap()), "pw"); }
#[tokio::test] async fn vault_unlock_cancel_falls_back_to_password_prompt() { let (deps, _) = deps_with_locked_vault("correct horse", "pw"); let ui = ScriptedUi::answers_m2b([Answer::Cancelled, Answer::Password(sec("pw"))]);
    connect(&deps, req(forever_profile()), ui.ix(), token(), progress()).await.unwrap(); assert!(matches!(ui.asked()[1], Question::Password { .. })); }
#[tokio::test] async fn encrypted_tls_key_passphrase_asked_when_missing() { let ui = ScriptedUi::answers_m2b([Answer::Secret(sec("pw1"))]); let live = connect_with_encrypted_client_key(&ui).await.unwrap();
    assert!(matches!(&ui.asked()[0], Question::Secret { prompt: SecretPrompt::TlsKeyPassphrase { .. }, .. })); drop(live); }
#[tokio::test] async fn connect_logs_never_contain_any_m2b_secret() { let logs = capture_tracing(|| full_m2b_connect_with_secrets("s3cret-db", "s3cret-ssh", "s3cret-kbd", "s3cret-master", "s3cret-key")).await;
    for s in ["s3cret-db", "s3cret-ssh", "s3cret-kbd", "s3cret-master", "s3cret-key"] { assert!(!logs.contains(s), "{s}"); } }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --test pipeline_tls --test pipeline_ssh --test pipeline_secrets`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. The question-and-retry loop is one function, `prompts::ask_and_retry`, with a per-kind retry counter. The questions never loop.
- [ ] **Step 4:** Run Step 2's command, then `cargo test -p hacivat-session`, `cargo test -p hacivat-driver --features fake` and `cargo xtask l10n check` (L026: every reachable class has EN messages; TR present). Expected: PASS, with the M2a pipeline tests green, adapted only where a fix list changed.
- [ ] **Step 5: Commit** with the message `feat(session): interactive TLS (pin, import CA), public-plaintext question, SSH transport step with prompts per hop, vault unlock, insecure-auth policy, new error classes and messages`.

**Review:** opus, adversarial. Probes:
- Does any answer path send credentials before consent?
- Can a scripted UI make the pipeline loop (`Pin` → still untrusted → ask again)? It must not.
- Does a `ProfileChange` reach a profile of a different id?
- Is a pin added during a **Test** persisted? (Only through the dialog draft, Task 19.)
- Can a pinned endpoint end up in plaintext through any answer or retry (Q13)?
- Is `TunnelManager::acquire` inside `clock.run` anywhere (Q44)? Does the SSH path call the local resolver?
- Does Headless prompt anywhere?
- Are prompted SSH secrets stored on failure?
- Can `ImportCa` with a path outside home bypass `ApprovedRoots`? (`ImportCa` paths come from the file picker and are approved by design. Assert that it is the picker path, not a typed one.)

### Task 15: `SessionManager`, router and pools (HV-SESS-01, -02, -08; D-035)

**Files:**
- Modify: `crates/core/hacivat-session/src/session/{router,pool}.rs` (`SessionManager` and its overlay live in `pool.rs`).
- Create: `crates/core/hacivat-session/tests/{router,pool}.rs`.
- Append: an ADR 0003 section, `## m2b-t15 — session overlay` (secrets with retention `Never` live in memory for the live session's duration; Q17).

**Interfaces:**
- Consumes: Task 7's `spawn_session`, `SessionHooks` and `Request::Reset`; M2a's `Budget`, `Priority` and `connect`.
- Produces:
  - **`route`, following HV-SESS-02 steps 1–7:**
    1. Single mode → `Default`.
    2. A client is attached → `Client`.
    3. `Introspection`, `QuickDoc`, `CodeGeneration`, `TextSearch` or `Refactoring` → `Pooled`.
    4. `DataExport` or `DataImport` → `NewJob`.
    5. `Mcp` → `Mcp(client or ClientId(u64::MAX))`.
    6. Anything else → `NewConsole`.
    7. With `isolated && target_busy && !needs_tx_context` → `Pooled`.
  - **`SessionManager`:**
    - `open(spec)` takes a budget permit by kind (Console → `Priority::Console`, Editor → `Editor`, Job → `Job`, Introspection and Service → `Introspection`), connects with `ConnectRequest { read_only: profile.read_only || spec.read_only, department: spec.department, .. }`, and spawns the session. The permit lives as long as the session.
    - `pooled(kind)` reuses an idle pooled session or opens one. Dropping the `PooledSession` sends `Request::Reset` and returns it to idle. A failed reset closes it and frees the permit. An idle pooled session closes after 60 s.
    - Single-session mode (profile flag), or SQLite (budget 1), makes `open` and `pooled` return the default session. **A `PooledSession` resets only sessions the pool owns (Q19):** a borrow of the default session is a borrow guard, never resets on drop (no `ROLLBACK` of the user's transaction), and its requests queue behind the console's (HV-SESS-02).
    - **The session overlay (Q17):** after each successful connect the manager applies the connect's `profile_changes` to its in-memory profile copy, records `plaintext_consent`, and keeps `secrets_used` (`SessionSecrets`, `SecretString`/`Zeroizing` buffers). Reconnects (Task 17) and side connections connect with `ConnectRequest { profile: overlay.profile, overlay_secrets: Some(overlay.secrets), plaintext_consent: overlay.plaintext_consent, .. }`. `update_profile(p)` replaces the overlay's profile and keeps consent and secrets. The overlay is dropped at `close_all` (disconnect, `forget`, quit) and is never persisted.
    - Idle pools are keyed by `(kind, read_only)`, so a read-only request is never served by a read-write pooled session.
    - The side connector (`Department::Cancel`) connects outside the budget, and caches the connection for 30 s.
    - MCP sessions never share with user consoles.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn route_table() { use RouteTarget::*; let r = |m, c, d, tx, iso, busy| route(m, &RouteRequest { client: c, department: d, needs_tx_context: tx, isolated: iso }, busy);
    assert_eq!(r(RoutingMode::Single, None, Department::Introspection, false, false, false), Default);
    assert_eq!(r(RoutingMode::Multiple, Some(ClientId(3)), Department::Console, false, false, false), Client(ClientId(3)));
    assert_eq!(r(RoutingMode::Multiple, None, Department::Introspection, false, false, false), Pooled);
    assert_eq!(r(RoutingMode::Multiple, None, Department::DataExport, false, false, false), NewJob);
    assert_eq!(r(RoutingMode::Multiple, Some(ClientId(9)), Department::Mcp, false, false, false), Mcp(ClientId(9)));
    assert_eq!(r(RoutingMode::Multiple, Some(ClientId(3)), Department::Console, false, true, true), Pooled);
    assert_eq!(r(RoutingMode::Multiple, Some(ClientId(3)), Department::Console, true, true, true), Client(ClientId(3))); }
// tests/pool.rs
#[tokio::test] async fn budget_of_four_queues_the_fifth_console_first() { let (sm, _) = manager4(fake_pg()); let held = open_n(&sm, 4).await;
    let intro = tokio::spawn(sm.pooled(SessionKind::Service)); let console = tokio::spawn(sm.open(console_spec(5), ix_none())); drop(held.into_iter().next());
    assert!(timeout(ms(200), console).await.is_ok()); assert!(timeout(ms(50), intro).await.is_err()); }
#[tokio::test] async fn pooled_release_resets_and_reuses() { let (sm, f) = manager4(fake_pg()); { let s = sm.pooled(SessionKind::Service).await.unwrap(); run(&s, "SET search_path = x").await; }
    settle().await; let _s2 = sm.pooled(SessionKind::Service).await.unwrap(); assert_eq!(f.connect_calls(), 1); assert_eq!(f.reset_calls(), 1); }
#[tokio::test(start_paused = true)] async fn pooled_idle_closes_after_60s() { let (sm, f) = manager4(fake_pg()); drop(sm.pooled(SessionKind::Service).await.unwrap());
    tokio::time::advance(secs(61)).await; settle().await; assert_eq!(f.open_connections(), 0); }
#[tokio::test] async fn failed_reset_discards_and_frees_the_permit() { let (sm, f) = manager_n(fake_pg().reset_fails(), 1); drop(sm.pooled(SessionKind::Service).await.unwrap()); settle().await;
    assert_eq!(f.open_connections(), 0); assert!(timeout(ms(200), sm.open(console_spec(1), ix_none())).await.is_ok()); }
#[tokio::test] async fn single_session_mode_shares_one_connection() { let (sm, f) = manager_single(fake_pg()); let a = sm.open(console_spec(1), ix_none()).await.unwrap();
    let b = sm.pooled(SessionKind::Service).await.unwrap(); assert_eq!(a.id(), b.id()); assert_eq!(f.connect_calls(), 1); drop(b); settle().await; assert_eq!(f.reset_calls(), 0); }
#[tokio::test] async fn single_session_pooled_drop_never_resets() { let (sm, f) = manager_single(fake_pg().exec(vec![tx("BEGIN", TxStatus::InTx)]));   // Q19
    let console = sm.open(console_spec(1), ix_none()).await.unwrap(); run(&console, "BEGIN").await; drop(sm.pooled(SessionKind::Service).await.unwrap()); settle().await;
    assert_eq!(f.reset_calls(), 0); assert_eq!(snap(&console).tx.status, TxStatus::InTx); }
#[tokio::test] async fn overlay_keeps_profile_changes_and_secrets_for_side_connections() { let (sm, f) = manager4(fake_pg_with_pin_answer_and_never_retention());   // Q17
    let _s = sm.open(console_spec(1), ix_interactive_pin_and_pw("pw")).await.unwrap(); let o = sm.overlay().unwrap();
    assert_eq!(o.profile.tls.pins.len(), 1); assert!(o.secrets.db.is_some()); sm.update_profile(Arc::new(renamed(&o.profile)));
    assert_eq!(sm.overlay().unwrap().profile.tls.pins.len(), 1); assert!(sm.overlay().unwrap().secrets.db.is_some()); let _ = f; }
#[tokio::test] async fn sqlite_is_single_session() { let (sm, f) = manager_sqlite(); let _a = sm.open(console_spec(1), ix_none()).await.unwrap(); let _b = sm.open(console_spec(2), ix_none()).await.unwrap(); assert_eq!(f.connect_calls(), 1); }
#[tokio::test] async fn read_only_spec_reaches_connect() { let (sm, f) = manager4(fake_pg()); sm.open(SessionSpec { read_only: true, ..console_spec(1) }, ix_none()).await.unwrap(); assert!(f.last_request().unwrap().read_only); }
#[tokio::test] async fn cancel_side_connection_is_outside_budget() { let (sm, _) = manager_n(fake_mysql().backend(5), 1); let s = sm.open(console_spec(1), ix_none()).await.unwrap();
    let q = s.submit(exec_slow("select sleep(60)")); settle().await; assert_eq!(timeout(secs(3), s.cancel(q.seq)).await.unwrap().unwrap(), CancelOutcome::Cancelled); }
#[tokio::test] async fn mcp_sessions_never_share_with_user_console() { let (sm, _) = manager4(fake_pg()); let user = sm.open(console_spec(1), ix_none()).await.unwrap();
    let mcp = sm.open(SessionSpec { kind: SessionKind::Console(ClientId(77)), department: Department::Mcp, read_only: true }, ix_none()).await.unwrap(); assert_ne!(user.id(), mcp.id()); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --test router --test pool`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. The `Priority` order is M2a's. The fair semaphore is M2a's `Budget`; it is not replaced.
- [ ] **Step 4:** Run Step 2's command, then `cargo test -p hacivat-session --test budget`. Expected: PASS.
- [ ] **Step 5: Commit** with the message `feat(session): router by department, SessionManager with budgeted consoles, reset-on-release pools with idle close, single-session borrows, side connections, session overlay`.

**Review:** opus. Probes:
- Is a permit leaked when `open` is cancelled midway?
- Does a pooled session cross read-only boundaries (a read-only spec served by a read-write pooled session)? It must not; the pools are keyed by `read_only`.
- Can two concurrent releases double-reset?
- Can a borrow of the default session ever reset it (Q19)? Does an overlay secret outlive `close_all` or reach `Debug` (Q17)?

### Task 16: Docker matrix, digest pinning, CI matrix and keyring jobs, `ChaosProxy` (L15, §7.1; B15, B17)

**Files:**
- Modify:
  - `docker/m2a/compose.yml` and `docker/m2a/targets.toml`;
  - Create `docker/m2a/sshd/{Dockerfile,entrypoint.sh,sshd_config}` and `docker/m2a/pg-auth/{pg_hba.conf,init.sh}`;
  - append to `docker/m2a/init/{pg.sql,mysql.sql}` (the `hv_ro` roles) and `docker/m2a/pg-tr/Dockerfile` (`FROM` digest);
  - `docker/m2a/README.md`.
- Modify: `crates/core/hacivat-testkit/src/{lib,chaos}.rs` (Task 1's `require_target` stays as is), `crates/core/hacivat-testkit/tests/targets.rs` (the new `pr` set: `targets.rs:123` asserts `["mysql84","pg17"]` today; Q6), `xtask/src/docker.rs` and `xtask/src/main.rs` (one arm), and `.github/workflows/ci.yml` (blocks `# ## m2b-t16-…`).
- Create: `crates/core/hacivat-testkit/tests/{chaos,ssh_targets}.rs`.

**Interfaces:**
- Consumes: the M2a T11 harness.
- Produces:
  - **New services**, per B17:
    - **`sshd-a`**: `127.0.0.1:2222`; networks `default` and `ssh-inner`.
    - **`sshd-b`**: no published port; networks `ssh-inner` and `default`. The topology test asserts that it is reachable from the host only through `sshd-a`.
    - **`pg17-auth`**: `127.0.0.1:54327`; users `hv_md5` (md5) and `hv_clear` (password); `password_encryption = md5` for those two users at init.
  - **Fixture tables and roles** in `init/{pg.sql,mysql.sql}`: `hv_probe(a int)`, `hv_probe_log(id serial or auto_increment, v int)` and the `hv_ro` roles (PostgreSQL: `CREATE ROLE hv_ro LOGIN PASSWORD …; GRANT pg_read_all_data TO hv_ro; REVOKE TEMP ON DATABASE hv_test FROM PUBLIC; REVOKE CREATE ON SCHEMA public FROM PUBLIC;` (the CREATE revoke keeps `hv_ro` enforced on pg13/pg14; T5 review), **no `NOINHERIT`**, Q21; MySQL/MariaDB: `SELECT, SHOW VIEW`), used by Tasks 13, 18 and 23. Task 13 runs in the same wave, so its tests create `hv_probe` and `hv_ro` themselves when missing (`ensure_probe_table`, `ensure_ro_role`, idempotent, through the admin connection; Q7).
  - **`targets.toml` `[[target]]` gains `service` (compose name) and `internal_port`**, and `DockerTarget` gains `internal: (String, u16)`: the in-network endpoint that tests through `sshd-a`/`sshd-b` connect to (`pg17:5432`, not `127.0.0.1:54317`; Q6). The `pr` set update goes into `tests/targets.rs` too.
  - **Generated at startup.** The sshd entrypoint generates host keys and writes `ssh-host-<id>.pub` to `/artifacts`. A one-shot `ssh-client-keys` service writes `ssh-client-ed25519` and `ssh-client-ed25519-enc` (passphrase `hv_test_pw`) plus their `.pub` files; the sshd containers mount the `.pub` into `authorized_keys`. Nothing private is committed.
  - **`targets.toml`** gains `[[ssh_target]] id, port, user = "hv_ssh", via = Option<id>`. Testkit gains `ssh_targets()`, `ssh_target(id)` and `ssh_client_key(encrypted: bool) -> PathBuf` (through `artifact`).
  - **`ChaosProxy`:** `start(upstream) -> ChaosProxy { addr }`, plus `cut(for_: Duration)` (close every relay and refuse new connections for the duration), `blackhole(on)` (stop forwarding without closing) and `restore()`. It counts `accepted()`.
  - **`xtask docker pin`** resolves each `image:` tag with `docker buildx imagetools inspect <tag> --format '{{json .Manifest.Digest}}'` (the index digest) and rewrites `tag` as `tag@sha256:…`, for every `FROM` too. **`xtask docker check-pins`** fails on any `image:` or `FROM` without `@sha256:`.
  - **Sets:** `pr` = pg17, mysql84, sshd-a, sshd-b. `full` = everything.
  - **CI:**
    - `docker-pr` runs `check-pins`, then `up --set pr`, then `test --set pr`;
    - `docker-full` is new, on schedule and `workflow_dispatch`, and runs `up --set full` then `test --set full`;
    - `keyring-linux` (ubuntu): `sudo apt-get install -y gnome-keyring dbus-x11`, then `dbus-run-session -- bash -c 'echo hv | gnome-keyring-daemon --unlock --components=secrets && HACIVAT_KEYRING_LIVE=1 cargo test -p hacivat-secrets -- --ignored live_keyring_round_trip'`; the test asserts `KeyringBackend::probe_available()` is `true` before the round trip, so a broken D-Bus setup fails loudly instead of silently testing the vault;
    - `keyring-windows` (windows-latest): the same test;
    - `vault-linux` (ubuntu, no D-Bus): `cargo test -p hacivat-secrets --test fallback -- --include-ignored linux_without_secret_service_uses_vault`. **Task 6 owns that test, and this job runs it**: Task 6 writes it as `#[ignore = "ci-linux"]`.
- **Docker test ownership:** Tasks 10, 13, 18 and 23 write the Docker tests that use these targets.

- [ ] **Step 1: Write the failing tests.**
```rust
// xtask (append to docker.rs tests)
#[test] fn check_pins_flags_unpinned_lines() { assert_eq!(unpinned("services:\n a:\n  image: postgres:17\n b:\n  image: mysql:8.4@sha256:0123\n"), ["postgres:17"]);
    assert_eq!(unpinned_dockerfile("FROM alpine:3.20\n"), ["alpine:3.20"]); }
#[test] fn pin_rewrites_tag_with_digest() { assert_eq!(pin_line("    image: postgres:17", "sha256:abcd"), "    image: postgres:17@sha256:abcd"); assert_eq!(pin_line("FROM alpine:3.20", "sha256:ef"), "FROM alpine:3.20@sha256:ef"); }
#[test] fn pr_set_includes_sshd() { let ids = services_in_set(&read("docker/m2a/targets.toml"), "pr").unwrap(); for id in ["pg17", "mysql84", "sshd-a", "sshd-b"] { assert!(ids.contains(&id.to_string()), "{id}"); } }
#[test] fn repo_compose_and_dockerfiles_are_pinned() { assert!(unpinned(&read("docker/m2a/compose.yml")).is_empty()); }
#[test] fn sshd_b_is_not_published() { assert!(service(&read("docker/m2a/compose.yml"), "sshd-b").ports.is_empty()); }
#[test] fn every_target_has_an_internal_endpoint() { for t in targets_in_set(None, None) { assert!(!t.internal.0.is_empty() && t.internal.1 > 0, "{}", t.id); } }   // testkit (Q6)
#[test] fn pg_init_never_uses_noinherit() { assert!(!read("docker/m2a/init/pg.sql").to_uppercase().contains("NOINHERIT")); }   // Q21
// testkit tests/chaos.rs
#[tokio::test] async fn chaos_proxy_relays_cuts_and_refuses() { let up = echo().await; let p = ChaosProxy::start(up).await; assert_eq!(echo_via(p.addr).await, b"ping");
    let held = connect(p.addr).await; p.cut(ms(300)); assert!(read_eof(held).await); assert!(connect(p.addr).await.is_err() || refused_soon(p.addr).await);
    tokio::time::sleep(ms(350)).await; assert_eq!(echo_via(p.addr).await, b"ping"); }
#[tokio::test] async fn blackhole_stalls_without_closing() { let p = ChaosProxy::start(echo().await).await; let mut c = connect(p.addr).await; p.blackhole(true);
    assert!(timeout(ms(200), roundtrip(&mut c)).await.is_err()); p.blackhole(false); }
#[tokio::test] #[ignore = "docker"] async fn ssh_targets_accept_the_generated_key() { let t = ssh_target("sshd-a").unwrap(); assert!(tcp_banner(t.port).await.starts_with("SSH-2.0")); assert!(ssh_client_key(false).exists()); }
```
- [ ] **Step 2:** Run `cargo test -p xtask docker` and `cargo test -p hacivat-testkit`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. `sshd_config` contains: `AllowTcpForwarding yes`, `PermitOpen any`, `PasswordAuthentication yes`, `KbdInteractiveAuthentication no` (keyboard-interactive is covered by the in-process server), `PubkeyAuthentication yes`, `ClientAliveInterval 0`, `MaxAuthTries 6`, `LogLevel VERBOSE`. Run `xtask docker pin` once, with network access (controller), and commit the digests.
- [ ] **Step 4:** Run Step 2's commands, then `cargo xtask docker check-pins`. Expected: PASS.
- [ ] **Step 5: Docker (this task only). After the controller confirms Task 13's Docker run is finished (Q7)**, run `cargo xtask docker down`, then `cargo xtask docker up --set full`. This step runs last in W3; the controller then re-runs Task 13's and Task 10's Docker sets on the new stack and records them in the ledger. Every service must be healthy within 90 s. Then run `cargo xtask docker test --set full -p hacivat-testkit`. Leave the stack up for the controller, and record the health table and the resolved image digests in the report.
- [ ] **Step 6: Commit** with the message `test(docker): sshd bastions with generated keys, md5/password PG, read-only roles, digest-pinned images, ChaosProxy; CI docker-full, keyring-linux/windows, vault-linux`.

**Review:** sonnet, plus a controller check. Probes:
- Is a private key, or any password other than the documented throwaway ones, committed?
- Do digests resolve multi-arch? (`docker buildx imagetools inspect` must list both `linux/amd64` and `linux/arm64` for each.)
- Can CI jobs read repository secrets? (They need none.)

### Task 17: Liveness, recovery, `ConnectionManager` integration (HV-CONN-04, -05; D-085; B4, B18, B19, B20)

**Files:**
- Modify: `crates/core/hacivat-session/src/session/{liveness,recovery}.rs` and `crates/core/hacivat-session/src/manager.rs`.
- Create: `crates/core/hacivat-session/tests/{liveness,recovery,manager_m2b}.rs`.
- Append: an ADR 0004 section, `## m2b-t17 — liveness and recovery`.

**Interfaces:**
- Consumes:
  - Task 7's actor, events and `SessionHooks.reconnect`;
  - Task 15's `SessionManager`;
  - Task 9's `TunnelEvent` (through `ConnectDeps.tunnels`);
  - Task 14's `LiveConnection.transport`;
  - M2a's `ConnectionManager` API, which is kept: every M2a manager test stays green.
- Produces:
  - **`Supervisor`, per HV-CONN-04:**
    1. When `busy` → `None`.
    2. Clock jump (Q31): `wall_delta − mono_delta > 5 s` **or** `mono_delta > tick + 5 s` (Windows `Instant` may include suspended time) → `Suspect` once. At the next tick, with the transaction Idle, the idle threshold is compared against **wall** time since the last activity: `Disconnect` if exceeded, else `Ping`. Missed ticks are never replayed.
    3. Idle threshold reached: with `tx` in `{InTx, Failed}` → `IdleTransaction` once per idle period; otherwise → `Disconnect`.
    4. Keep-alive due (counted from the later of the last activity and the last ping) → `Ping`.

    Keep-alive pings never count as activity. `LivenessPolicy::for_profile` applies B18's presets.
  - **The actor wiring.** Task 7's actor runs the supervisor on a 1 s `interval` with `MissedTickBehavior::Skip`. A ping is `RawConn::ping()` with a timeout of `min(every / 2, 10 s)`. A failure (or `Disconnect`) leads to recovery or to a graceful disconnect.
  - **`RecoveryPolicy::default()`:** schedule `[0, 1, 2, 4, 8] s`, steady 30 s, give up after 300 s, jitter 0.2. `delay` returns `None` past `give_up_after`.
  - **`is_recovery_case`:** true for `network.*`, `server.shutdown`, `server.starting`, `karagoz.upstream_unreachable` and `karagoz.resume_expired`.
  - **`track_statement`:** the first-keyword scan from B19. `lost_state` maps a `Tracked` into a `LostState`.
  - **`ConnectionManager` integration:**
    - Each live profile holds an `Arc<SessionManager>` and the default console `SessionHandle` (B4).
    - `LiveState::Connected` is published once the default session is `Idle`.
    - On a recovery case for the default session:
      1. `LiveState::Reconnecting { attempt, next_in }`.
      2. Reconnect Headless through `SessionHooks.reconnect` (a pipeline `connect` with `Purpose::Main`), using the **session overlay** (Task 15, Q17): the profile with this session's `ProfileChange`s applied and any `update_profile` edits, the one-gesture plaintext consent, and the `SessionSecrets` that worked. So a pin, CA or consent answered at connect time holds on reconnect, and a secret with retention `Never` (or a one-time-code-free SSH passphrase typed once) is reused.
      3. Replay the environment plan (schema, transaction mode and isolation, read-only).
      4. `LiveState::Connected`, then `ConnectionEvent::SessionLost { lost }` (always, B19).
    - An `auth.*` failure during reconnect → `LiveState::Failed` and stop.
    - **A reconnect that still needs interaction** (`ConnectError.needs_interaction`: a one-time code, an unknown host key, a locked vault) → `LiveState::Lost { class }` at once, no backoff loop. The capsule's Retry starts an interactive connect. Recovery never prompts (Q17).
    - `ConnectionManager::update_profile(id, profile)` forwards to that profile's `SessionManager::update_profile` (Q17); the app calls it after every save and `apply_changes`.
    - `ConnectionEvent::EnforcementGap` carries the session's `GapReason` (Q35).
    - After the give-up time → `LiveState::Lost { class }` (Retry, as in M2a).
    - A user statement in flight when the connection dies fails with `ConnectionLost { outcome_unknown: true }` and is never re-run. An `Internal { idempotent: true }` request is re-queued once at the head.
    - **Group invalidation:** on `TunnelEvent::Dead { key, generation }`, every profile whose `LiveConnection.transport` matches goes into recovery **once**, under a per-`(key, generation)` lock (B20).
    - `Disconnect` from idle → `LiveState::Disconnected` plus `ConnectionEvent::IdleDisconnected`. `IdleTransaction` → `ConnectionEvent::IdleTransaction`. A session `EnforcementGap` → `ConnectionEvent::EnforcementGap`. A connect's `profile_changes` → `ConnectionEvent::ProfileChanges`, on success and on failure.
    - `LiveInfo` gains `via`, `hops` and `access_warnings`.
    - Recovery during `shutdown` aborts.

- [ ] **Step 1: Write the failing tests.**
```rust
// tests/liveness.rs (pure Supervisor; Instant/SystemTime driven by the test)
#[test] fn keepalive_every_60_idle_300() { let mut s = sup(Some(60), Some(300)); let acts = run_ticks(&mut s, 301, TxStatus::Idle);
    assert_eq!(pings_at(&acts), [60, 120, 180, 240]); assert_eq!(disconnect_at(&acts), Some(300)); }
#[test] fn open_tx_at_threshold_emits_idle_transaction_not_disconnect() { let mut s = sup(None, Some(300)); let acts = run_ticks(&mut s, 900, TxStatus::InTx);
    assert_eq!(acts.iter().filter(|a| **a == LivenessAction::IdleTransaction).count(), 1); assert!(!acts.contains(&LivenessAction::Disconnect)); }
#[test] fn busy_skips_and_keepalive_is_not_activity() { let mut s = sup(Some(60), Some(120)); assert!(tick_busy(&mut s, 200).is_none()); let acts = run_ticks(&mut s, 121, TxStatus::Idle);
    assert_eq!(disconnect_at(&acts), Some(120)); }
#[test] fn clock_jump_pings_once_not_per_missed_tick() { let mut s = sup(Some(60), None); let acts = jump_wall(&mut s, Duration::from_secs(7200));
    assert_eq!(acts, [LivenessAction::Suspect, LivenessAction::Ping]); }
#[test] fn monotonic_jump_alone_is_suspect() { let mut s = sup(Some(60), None); assert_eq!(jump_mono_and_wall(&mut s, secs(7200)), [LivenessAction::Suspect, LivenessAction::Ping]); }   // Windows QPC (Q31)
#[test] fn sleep_past_idle_threshold_disconnects() { let mut s = sup(None, Some(3600)); idle_for(&mut s, secs(59 * 60));   // Q31
    assert_eq!(jump_wall(&mut s, secs(8 * 3600)), [LivenessAction::Suspect, LivenessAction::Disconnect]); }
#[test] fn presets() { assert_eq!(LivenessPolicy::for_profile(&env(EnvTag::Production)).idle_disconnect, Some(secs(3600)));
    assert_eq!(LivenessPolicy::for_profile(&env(EnvTag::Dev)).idle_disconnect, Some(secs(4 * 3600))); assert_eq!(LivenessPolicy::for_profile(&env(EnvTag::Local)).idle_disconnect, None);
    assert_eq!(LivenessPolicy::for_profile(&ssh_profile()).keepalive, Some(secs(60))); assert_eq!(LivenessPolicy::for_profile(&direct_profile()).keepalive, None); }
// tests/recovery.rs
#[test] fn backoff_schedule_with_jitter_bounds() { let p = RecoveryPolicy::default(); let mut lo = || 0.0f32; let mut hi = || 1.0f32;
    for (i, base) in [0u64, 1, 2, 4, 8, 30, 30].iter().enumerate() { let d = |r: &mut dyn FnMut() -> f32| p.delay(i as u32, secs(0), r).unwrap().as_millis() as f64;
        assert!((d(&mut lo) - *base as f64 * 800.0).abs() < 1.0); assert!((d(&mut hi) - *base as f64 * 1200.0).abs() < 1.0); }
    assert!(p.delay(20, secs(301), &mut lo).is_none()); }
#[test] fn recovery_cases() { assert!(is_recovery_case(ErrClass::NetworkReset)); assert!(is_recovery_case(ErrClass::ServerShutdown)); assert!(!is_recovery_case(ErrClass::AuthInvalid)); }
#[test] fn lost_state_from_tracked() { let mut t = Tracked::default(); for s in ["BEGIN", "CREATE TEMP TABLE x(a int)", "SET work_mem = '1MB'", "SELECT pg_advisory_lock(1)"] { track_statement(&mut t, s); }
    t.tx = TxStatus::InTx; assert_eq!(lost_state(&t), LostState { open_tx: true, temp_objects: true, session_vars: true, prepared: 0, locks: true }); }
// tests/manager_m2b.rs
#[tokio::test(start_paused = true)] async fn liveness_failure_reconnects_and_emits_session_lost() { let (m, f) = manager(ok_pg().pings(vec![Err(io_reset())])); let mut ev = m.events();
    connect_and_wait(&m).await; tokio::time::advance(secs(31)).await; settle().await; assert!(matches!(m.state(p().id), LiveState::Reconnecting { attempt: 1, .. } | LiveState::Connected(_)));
    wait_connected(&m, p().id).await; assert!(matches!(ev.recv().await.unwrap(), ConnectionEvent::SessionLost { .. })); assert_eq!(f.connect_calls(), 2); }
#[tokio::test(start_paused = true)] async fn auth_failure_during_reconnect_stops_with_failed() { let (m, f) = manager(FakeDriver::new(DbmsKind::Postgres).script(vec![Ok(()), Err(db("28P01"))]).pings(vec![Err(io_reset())]));
    connect_and_wait(&m).await; tokio::time::advance(secs(60)).await; settle().await; assert!(matches!(m.state(p().id), LiveState::Failed(_))); assert_eq!(f.connect_calls(), 2); }
#[tokio::test] async fn user_statement_is_never_replayed_after_connection_loss() { let (m, f) = manager(ok_pg().exec(vec![slow("INSERT INTO t VALUES (1)", ms(200))])); connect_and_wait(&m).await;
    let s = m.sessions(p().id).unwrap().default_session().unwrap(); let t = s.submit(user_exec("INSERT INTO t VALUES (1)")); settle().await; f.kill_connections();
    assert_eq!(t.reply.await.unwrap().unwrap_err().kind, ExecErrorKind::ConnectionLost { outcome_unknown: true }); wait_connected(&m, p().id).await;
    assert_eq!(f.executed().iter().filter(|s| s.starts_with("INSERT")).count(), 1); }
#[tokio::test] async fn internal_idempotent_read_is_retried_once() { let (m, f) = manager(ok_pg().exec(vec![fails_once_with_reset("SELECT 42")])); connect_and_wait(&m).await;   // not the schema probe's text (Q27)
    let s = default_session(&m); assert!(s.submit(internal_exec("SELECT 42")).reply.await.unwrap().is_ok());
    assert_eq!(f.executed().iter().filter(|x| x.starts_with("SELECT 42")).count(), 2); }
#[tokio::test] async fn reconnect_after_pin_answer_uses_the_pin() { let (m, f) = manager_with_pin_answer(); connect_and_wait(&m).await; f.kill_connections();   // Q17
    wait_connected(&m, p().id).await; assert_eq!(f.last_request().unwrap().tls.unwrap().settings.pins.len(), 1); }
#[tokio::test] async fn reconnect_with_never_retention_reuses_session_secret() { let (m, f) = manager_never_retention_prompted("pw"); connect_and_wait(&m).await;   // Q17
    f.kill_connections(); wait_connected(&m, p().id).await; assert_eq!(f.connect_calls(), 2); assert!(nothing_stored(&m)); }
#[tokio::test] async fn reconnect_needing_interaction_stops_with_lost() { let (m, f) = manager_kbd_otp(); connect_and_wait(&m).await; f.kill_connections();   // Q17
    wait_state(&m, |s| matches!(s, LiveState::Lost { .. })).await; assert_eq!(f.connect_calls(), 1); }
#[tokio::test] async fn replay_restores_schema_tx_mode_and_read_only() { let (m, f) = manager_ro(ok_pg()); connect_and_wait(&m).await; set_schema(&m, "app").await; set_mode(&m, TxMode::Manual).await;
    f.kill_connections(); wait_connected(&m, p().id).await; let after = after_last_connect(&f.executed());
    assert!(after.contains(&"SET default_transaction_read_only = on".to_string()) && after.contains(&"SET search_path TO \"app\"".to_string()));
    assert_eq!(snap(&default_session(&m)).tx.mode, TxMode::Manual); }
#[tokio::test] async fn one_ssh_death_reconnects_each_dependent_once() { let (m, s, f) = manager_two_profiles_one_bastion().await; connect_both(&m).await; s.kill_sessions();
    wait_connected_both(&m).await; assert_eq!(f.connect_calls(), 4); assert_eq!(s.sessions_accepted(), 2); }
#[tokio::test(start_paused = true)] async fn idle_disconnect_emits_event_and_state_disconnected() { let (m, _) = manager_profile(ok_pg(), idle(300)); let mut ev = m.events(); connect_and_wait(&m).await;
    tokio::time::advance(secs(301)).await; settle().await; assert_eq!(m.state(p().id), LiveState::Disconnected); assert!(matches!(ev.recv().await.unwrap(), ConnectionEvent::IdleDisconnected { .. })); }
#[tokio::test(start_paused = true)] async fn gives_up_after_five_minutes_with_lost() { let (m, _) = manager(FakeDriver::new(DbmsKind::Postgres).script_then(vec![Ok(())], Err(refused())).pings(vec![Err(io_reset())]));
    connect_and_wait(&m).await; tokio::time::advance(secs(400)).await; settle().await; assert!(matches!(m.state(p().id), LiveState::Lost { .. })); }
#[tokio::test] async fn profile_changes_are_published() { let (m, _) = manager_with_pin_answer(); let mut ev = m.events(); connect_and_wait(&m).await;
    assert!(matches!(ev.recv().await.unwrap(), ConnectionEvent::ProfileChanges { changes, .. } if matches!(changes[0], ProfileChange::AddPin(_)))); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --test liveness --test recovery --test manager_m2b`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. Keep `ManagerConfig.liveness` as the default keep-alive for profiles with none set (M2a compatibility). Its default of 30 s still applies to Direct profiles under the M2a manager tests: B18's off-by-default applies only to `LivenessPolicy::for_profile` when `ManagerConfig::m2b()` is used; the app boot (Task 22) switches to it. The jitter RNG draws from `getrandom::fill` (Q30: no `rand_core`), with an injectable `fn() -> f32` in tests. The pure `Supervisor` tests drive `Instant`/`SystemTime` by hand; `start_paused` is used only with FakeDriver (Q25).
- [ ] **Step 4:** Run Step 2's command, then `cargo test -p hacivat-session` (the M2a `manager.rs` tests included). Expected: PASS.
- [ ] **Step 5: Commit** with the message `feat(session): liveness supervisor with clock-jump detection, reconnect with backoff and state replay, session-lost events, SSH group invalidation, ConnectionManager over SessionManager`.

**Review:** opus. Probes:
- Can a reconnect storm happen? Use 50 sessions on one tunnel and assert one recovery per profile.
- Does any code path replay a user statement?
- Does `Reconnecting` ever show for a profile that was never connected?
- Does shutdown wait on a backoff sleep? (It must abort within the M2a 2 s budget.)

### Task 18: Guarded read-only execution and the role probe (HV-SESS-04, SQL-SAFE-05, MCP-09 step 1; B21)

**Files:**
- Modify: `crates/core/hacivat-session/src/session/readonly.rs`.
- Create: `crates/core/hacivat-session/tests/readonly.rs` and `crates/core/hacivat-session/tests/docker_readonly.rs`.
- Append: an ADR 0004 section, `## m2b-t18 — read-only execution and labels`.

**Interfaces:**
- Consumes: Task 5's plans, checks and probe; Tasks 10, 11 and 13 (driver surfaces); Task 7's actor (which calls `run_guarded`); the pipeline's PostConnect (which calls `access_for`).
- Produces:
  - **`run_guarded(conn, kind, sql, opts, replay)`:**
    1. `check_input(sql)`, else `ReadOnlyDenied(Input)`.
    1a. A first keyword of `COMMIT`, `ROLLBACK`, `BEGIN`, `START`, `END`, `ABORT`, `SAVEPOINT` or `RELEASE` (ASCII-folded, leading whitespace and comments skipped) is `ReadOnlyDenied(Transaction)` before any I/O (Q35). This is the producer of `DenyLayer::Transaction`, not a classifier; steps 5–7 still catch smuggled cases.
    2. `plan.before` through `batch`.
    3. `conn.execute(sql, ExecOpts { protocol: Extended, .. })`. A server rejection with a read-only class code becomes `ExecErrorKind::Db` with the server message (codes PG `25006`, `42501`; MySQL `1792`, `1142`, `1290`; SQLite `SQLITE_READONLY`/`SQLITE_AUTH`).
    4. Note `tx_status()` → `PostFacts.tx_after_statement`.
    5. The canary (PostgreSQL) through `execute(Simple)`.
    6. `plan.after` (ROLLBACK) through `execute(Simple)`, collecting its `notice_codes`.
    7. `check_post`. On `Gap` → return `EnforcementGap(reason)`; the actor marks the session `Broken`, closes the connection and emits `SessionEvent::EnforcementGap`.
    8. `conn.reset()` when `plan.reset != None`, then `replay` (Q15): the session's full environment plan (read-only setup, `RestoreSchema`), so the schema survives every guarded request and `SchemaChanged` does not flip.

    If step 2 fails, the request fails and nothing else runs. If step 3 errors, steps 6 and 8 still run.

    SQLite: only steps 1 and 1a, then `execute`. Hardening is connection-level, through Task 11.
    Read-only sessions are always Auto (B22, Q14), so `run_guarded` never runs inside a user transaction.
  - **`access_for(conn, kind, read_only)`:** when `read_only` and the engine is PostgreSQL or MySQL, run `role_probe_sql` through `execute(Simple)` and return `(access_label(..), findings.warnings)`. A probe that fails is treated as `Unparsed`, so `Guarded`. MariaDB and SQLite return `(ReadOnlyGuarded, [])` with no probe. Read-write returns `(ReadWrite, [])`.

- [ ] **Step 1: Write the failing tests.**
```rust
// tests/readonly.rs (FakeDriver)
#[tokio::test] async fn pg_guarded_exec_sequence() { let f = fake_pg_ro_clean(); let mut c = conn(&f).await; run_guarded(&mut *c, DbmsKind::Postgres, "select * from t", ExecOpts::default(), &pg_session_setup()).await.unwrap();   // replay = the four session_setup statements (Q15)
    assert_eq!(f.executed(), ["BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY", "SELECT 1", "select * from t", "SELECT txid_current_if_assigned()", "ROLLBACK",
        "SET default_transaction_read_only = on", "SET standard_conforming_strings = on", "SET client_encoding = 'UTF8'", "SET idle_in_transaction_session_timeout = '30s'"]);
    assert_eq!(f.reset_calls(), 1); }
#[tokio::test] async fn smuggled_commit_is_an_enforcement_gap_and_discards_the_connection() { let (h, f) = ro_session(fake_pg_ro_clean().exec(vec![tx("SELECT 1; COMMIT", TxStatus::Idle)]));
    let r = h.submit(user_exec("SELECT 1; COMMIT")).reply.await.unwrap(); assert_eq!(r.unwrap_err().kind, ExecErrorKind::EnforcementGap(GapReason::TransactionEndedByStatement));   // smuggled past 1a
    assert!(matches!(snap(&h).state, SessionState::Broken(_))); assert_eq!(f.open_connections(), 0); }
#[tokio::test] async fn canary_non_null_is_a_gap() { let f = fake_pg_ro_clean().exec(vec![rows("SELECT txid_current_if_assigned()", &[&["731"]])]); let mut c = conn(&f).await;
    assert_eq!(run_guarded(&mut *c, DbmsKind::Postgres, "select 1", ExecOpts::default(), &[]).await.unwrap_err().kind, ExecErrorKind::EnforcementGap(GapReason::CanaryAssigned)); }
#[tokio::test] async fn transaction_keywords_are_denied_before_io() { for sql in ["COMMIT", "  /* x */ begin", "SAVEPOINT a", "release a", "END"] {   // Q35
    let f = fake_pg_ro_clean(); let mut c = conn(&f).await; assert_eq!(run_guarded(&mut *c, DbmsKind::Postgres, sql, ExecOpts::default(), &[]).await.unwrap_err().kind,
        ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction), "{sql}"); assert!(f.executed().is_empty()); } }
#[tokio::test] async fn schema_survives_a_guarded_request() { let (h, f) = ro_session(fake_pg_ro_clean());   // Q15
    h.submit(Request::SwitchSchema("app".into())).reply.await.unwrap().unwrap(); run(&h, "select 1").await.unwrap();
    assert_eq!(f.executed().last().unwrap(), "SET search_path TO \"app\""); }
#[tokio::test] async fn input_over_1mib_rejected_before_io() { let f = fake_pg_ro_clean(); let mut c = conn(&f).await; let big = "x".repeat(MAX_STATEMENT_BYTES + 1);
    assert_eq!(run_guarded(&mut *c, DbmsKind::Postgres, &big, ExecOpts::default(), &[]).await.unwrap_err().kind, ExecErrorKind::ReadOnlyDenied(DenyLayer::Input)); assert!(f.executed().is_empty()); }
#[tokio::test] async fn statement_error_still_rolls_back_and_resets() { let f = fake_pg_ro_clean().exec(vec![err("insert into t values (1)", "25006")]); let mut c = conn(&f).await;
    assert_eq!(run_guarded(&mut *c, DbmsKind::Postgres, "insert into t values (1)", ExecOpts::default(), &[]).await.unwrap_err().kind, ExecErrorKind::Db);
    assert!(f.executed().contains(&"ROLLBACK".to_string())); assert_eq!(f.reset_calls(), 1); }
#[tokio::test] async fn mysql_guarded_sequence() { let f = fake_mysql_ro_clean(); let mut c = conn(&f).await; run_guarded(&mut *c, DbmsKind::MySql, "select 1", ExecOpts::default(), &mysql_session_setup()).await.unwrap();
    assert_eq!(f.executed(), ["START TRANSACTION READ ONLY", "select 1", "ROLLBACK", "SET SESSION TRANSACTION READ ONLY", "SET NAMES utf8mb4"]); assert_eq!(f.reset_calls(), 1); }
#[tokio::test] async fn access_for_probes_only_when_read_only() { let f = fake_pg_probe_least(); let mut c = conn(&f).await;
    assert_eq!(access_for(&mut *c, DbmsKind::Postgres, false).await.0, AccessLabel::ReadWrite); assert!(f.executed().is_empty());
    assert_eq!(access_for(&mut *c, DbmsKind::Postgres, true).await.0, AccessLabel::ReadOnlyEnforced);
    assert_eq!(access_for(&mut *conn(&fake_mariadb()).await, DbmsKind::MariaDb, true).await.0, AccessLabel::ReadOnlyGuarded); }
// tests/docker_readonly.rs — targets only through require_target/targets(..) (Q6); hv_ro created idempotently by the helpers (Q7)
#[tokio::test] #[ignore = "docker"] async fn pg_hv_ro_can_read_a_table() { let s = ro_session_as_on("pg17", "hv_ro").await; assert!(run(&s, "SELECT count(*) FROM hv_probe").await.is_ok()); }   // Q21
#[tokio::test] #[ignore = "docker"] async fn pg_hv_ro_is_enforced_and_superuser_is_guarded() { assert_eq!(connect_ro_as("pg17", "hv_ro").await.access, AccessLabel::ReadOnlyEnforced);
    let su = connect_ro_as("pg17", "hv_test").await; assert_eq!(su.access, AccessLabel::ReadOnlyGuarded); assert!(su.access_warnings.contains(&RoleWarning::Superuser)); }
#[tokio::test] #[ignore = "docker"] async fn mysql_hv_ro_enforced_mariadb_guarded() { assert_eq!(connect_ro_as("mysql84", "hv_ro").await.access, AccessLabel::ReadOnlyEnforced);
    for t in targets(Some(DbmsKind::MariaDb)) { assert_eq!(connect_ro_as(&t.id, "hv_ro").await.access, AccessLabel::ReadOnlyGuarded, "{}", t.id); } }
#[tokio::test] #[ignore = "docker"] async fn pg_advisory_lock_and_listen_do_not_survive_a_request() { let s = ro_session_on("pg17").await; run(&s, "SELECT pg_advisory_lock(42)").await.unwrap();
    let _ = run(&s, "LISTEN ch").await; let admin = admin("pg17").await; assert_eq!(scalar(&admin, &format!("SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND pid={}", backend(&s))).await, "0");
    assert_eq!(scalar_on_session(&s, "SELECT count(*) FROM pg_listening_channels()").await, "0"); }
#[tokio::test] #[ignore = "docker"] async fn sqlite_is_guarded() { assert_eq!(connect_ro_sqlite().await.access, AccessLabel::ReadOnlyGuarded); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --test readonly`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. Never send user text through `batch`.
- [ ] **Step 4:** Run Step 2's command, then `cargo xtask docker test --set full -p hacivat-session` (the `docker_readonly` file). Expected: PASS.
- [ ] **Step 5: Commit** with the message `feat(session): guarded read-only execution (L0/L3/L4/L6) with enforcement-gap discard, role probe and honest labels`.

**Review:** opus, adversarial. Probes:
- Can a statement that times out during step 3 leave the session in a read-write transaction? (Steps 6–8 must still run, or the connection must be discarded.)
- Is the canary skipped on error paths in a way that hides a gap?
- Is `access_for` ever reached with user-controlled SQL?
- Does a probe on a 100k-table database finish in under 2 s? (`EXISTS`; record the timing in ADR 0004.)

### Task 19: TLS UI: certificate review, pin changed, CA import, the public-plaintext question, pinned row, key passphrase, removal of "Encrypt without verifying" (NOTES surface 5; new surface 5; B5–B7, B25)

**Files:**
- Modify:
  - `crates/app/hacivat-app/src/connections/{cert_dialog,plaintext_prompt,fixes,error_view}.rs` and `dialog/{mod,form}.rs` (the TLS section, the Remember Select label through `vault_dialog::forever_label`, and the passphrase field);
  - `model.rs` (the `apply_changes` body only).
- Create: `crates/app/hacivat-app/tests/connections_tls.rs`.
- Append: the FTL block `## m2b-t19-tls` in EN and TR, and rewrite Task 1's placeholder `hv-conn-issue-tls-key-legacy-encrypted` (named exception; Q3). `CARGO_TARGET_DIR=<main>/target-core/m2b-t19`, `CARGO_INCREMENTAL=0` (Q1).

**Interfaces:**
- Consumes: Task 14's `Question::{Certificate, PlaintextPublic, Secret(TlsKeyPassphrase)}`, `ConnectError.certificate` and `ProfileChange`; Task 3's `fingerprint_lines`; M2a's dialog, the 480 px wide size, the fingerprint inset, the chain list and the inline notice.
- Produces:
  - **`cert_dialog::open`** (surface 5).
    - A 480 px dialog: subject, issuer, the valid-until date (the region format, M1 I2), the two fingerprint lines in mono with Copy, the chain rows with their roles, and the pin note.
    - Buttons: Cancel (left), Import CA… (a file picker; the chosen path answers `ImportCa`) and **Pin this certificate** (primary, Enter).
    - With `review.pinned.is_some()` it is the **pin-changed** variant (new surface 5): "Pinned" and "Now" fingerprints, primary Cancel (initial focus, the only Enter default; Q34), and secondary "Replace pin". It is reachable only from the error (the pipeline never asks on `PinMismatch`).
  - **`cert_dialog::review(err)`** opens the same dialog from an error (the `ReviewCertificate` fix). **It builds the `ProfileChange` itself (Q26):** "Replace pin" → `ProfileChange::ReplacePin { old: review.pinned.unwrap(), new: CertPin { endpoint: review.endpoint, sha256: review.sha256 } }`; "Pin" → `AddPin`; Import CA → `SetCaFile`. It applies the change through `apply_changes` (or the dialog draft, for Test), then starts a new connect gesture. The pipeline never produces `ReplacePin`, so every pin replacement is an explicit, traceable user action.
  - **`plaintext_prompt::open`:** 420 px, with the "Don't ask again for this connection" checkbox. Buttons: Cancel, Connect without encryption, and **Use an SSH tunnel…** (primary). `UseSsh` also calls `dialog::ssh::open_with_ssh(profile)`.
  - **Fixes.** `FixAction::{ReviewCertificate, ImportCa}` are wired here. `ConnectThroughSsh` and `ReviewHostKey` are already wired by Task 1 (one line each in `fixes.rs`, Q9); do not touch them. `EncryptWithoutVerifying` is removed (the enum arm and the button), and so is `hv-conn-fix-require` (the named exception).
  - **TLS section:**
    - a "Pinned certificate" row (`hv-conn-tls-pin`, a short fingerprint or `hv-conn-tls-pin-none`) with "Remove pin";
    - a "Key passphrase" field, masked with Remember, shown when a client certificate is set;
    - VerifyCa enabled for MySQL and MariaDB.
  - **`ConnectionsModel::apply_changes`** applies each `ProfileChange` to the stored profile and saves it through the ordered writer, then calls `ConnectionManager::update_profile(id, profile)` so the live session overlay sees it (Q17). A Test run in the dialog applies changes to the **draft** only.
  - **UI lessons (Q34):** the dialogs follow the Global Constraints "Modal layers" rules (Enter on key-down only; key-up Enter suppressed; Escape, Enter and Tab bubble out of a nested Select; hard stops default to Cancel). Waits in tests use `settle_until` on a condition, never a fixed sleep.

- [ ] **Step 1: Write the failing tests** (gpui; `settle` as in M2a).
```rust
#[gpui::test] fn certificate_question_opens_review_dialog(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx); let q = cert_question(review_fixture("Müşteri DB", [0xAB; 32]));
    ask(&vcx, q); assert_eq!(layer_width(&vcx, "cert-dialog"), px(480.0)); assert_eq!(mono_lines(&vcx, "cert-fingerprint").len(), 2);
    assert!(text_of(&vcx, "cert-subject").contains("Müşteri DB")); assert_eq!(primary_label(&vcx, "cert-dialog"), t(&vcx, "hv-conn-tls-pin-btn")); }
#[gpui::test] fn enter_pins_escape_cancels(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx); let a = ask(&vcx, cert_question(review_fixture("db", [1; 32]))); press(&vcx, "enter"); assert!(matches!(a.get(), Some(Answer::Pin)));
    let b = ask(&vcx, cert_question(review_fixture("db", [1; 32]))); press(&vcx, "escape"); assert!(matches!(b.get(), Some(Answer::Cancelled))); }
#[gpui::test] fn import_ca_answers_the_picked_path(cx: &mut TestAppContext) { let vcx = shell_with_picker(cx, "/home/me/corp-ca.pem"); let a = ask(&vcx, cert_question(review_fixture("db", [1; 32])));
    click(&vcx, "cert-import"); settle_until(&vcx, secs(2), || a.get().is_some()); assert!(matches!(a.get(), Some(Answer::ImportCa(p)) if p == PathBuf::from("/home/me/corp-ca.pem"))); }
#[gpui::test] fn pin_changed_dialog_has_cancel_as_default(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx); ask(&vcx, cert_question(pinned_review([1; 32], [2; 32])));
    assert_eq!(focused_id(&vcx), "cert-cancel"); assert!(!is_primary(&vcx, "cert-replace-pin")); assert_eq!(mono_lines(&vcx, "cert-fingerprint-old").len(), 2);
    press(&vcx, "enter"); assert!(!layer_open(&vcx, "cert-dialog")); }   // Enter = Cancel on a hard stop (Q34)
#[gpui::test] fn replace_pin_from_error_writes_replace_pin(cx: &mut TestAppContext) { let vcx = with_failed(cx, pin_changed_error([1; 32], [2; 32]));   // Q26
    click_fix(&vcx, FixAction::ReviewCertificate); click(&vcx, "cert-replace-pin"); settle_until(&vcx, secs(2), || read_connections_toml(&vcx).contains(&"02".repeat(32)));
    assert!(!read_connections_toml(&vcx).contains(&"01".repeat(32))); assert_eq!(last_applied_change(&vcx), Some(ProfileChange::ReplacePin { old: [1; 32], new: pin("db:5432", [2; 32]) })); }
#[gpui::test] fn plaintext_question_dialog(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx); let a = ask(&vcx, plaintext_question("db.acme.io", "203.0.113.10"));
    assert_eq!(layer_width(&vcx, "plaintext-prompt"), px(420.0)); click(&vcx, "plain-remember"); click(&vcx, "plain-connect");
    assert!(matches!(a.get(), Some(Answer::ConnectPlaintext { remember: true }))); }
#[gpui::test] fn review_certificate_fix_reopens_dialog_from_error(cx: &mut TestAppContext) { let vcx = with_failed(cx, untrusted_error_with_review()); click_fix(&vcx, FixAction::ReviewCertificate); assert!(layer_open(&vcx, "cert-dialog")); }
#[gpui::test] fn encrypt_without_verifying_is_gone(cx: &mut TestAppContext) { let vcx = with_failed(cx, untrusted_error_with_review()); assert!(!any_button_with(&vcx, "hv-conn-fix-require"));
    assert!(!ftl_has_id("en", "hv-conn-fix-require") && !ftl_has_id("tr", "hv-conn-fix-require")); }
#[gpui::test] fn tls_section_pinned_row_and_remove_pin(cx: &mut TestAppContext) { let vcx = dialog_editing(cx, pinned_profile()); assert!(text_of(&vcx, "tls-pin-row").contains("AB:AB"));
    click(&vcx, "tls-remove-pin"); assert!(draft(&vcx).profile.tls.pins.is_empty()); }
#[gpui::test] fn key_passphrase_field_visible_with_client_cert(cx: &mut TestAppContext) { let vcx = dialog_editing(cx, client_cert_profile()); assert!(visible(&vcx, "tls-key-passphrase")); assert!(is_masked(&vcx, "tls-key-passphrase")); }
#[gpui::test] fn mysql_verify_ca_enabled(cx: &mut TestAppContext) { let vcx = dialog_new_mysql(cx); assert!(select_option_enabled(&vcx, "tls-mode", "verify_ca")); }
#[gpui::test] fn apply_changes_persists_pin(cx: &mut TestAppContext) { let vcx = shell_with_profile(cx, pg_profile()); emit(&vcx, ConnectionEvent::ProfileChanges { profile: pg_id(), changes: vec![ProfileChange::AddPin(pin("db:5432", [0xAB; 32]))] });
    settle_until(&vcx, secs(2), || read_connections_toml(&vcx).contains(&"ab".repeat(32))); assert_eq!(overlay_pins(&vcx, pg_id()), 1); }   // update_profile reached the overlay (Q17)
#[gpui::test] fn test_dialog_pin_updates_draft_only(cx: &mut TestAppContext) { let vcx = dialog_test_with_pin_answer(cx); assert_eq!(draft(&vcx).profile.tls.pins.len(), 1); assert!(!read_connections_toml(&vcx).contains("pins")); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test connections_tls`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. Fluent ids:
  - the M2a mock ids mapped as in M2a (`tls-*` → `hv-conn-tls-*`, `plain-*` → `hv-conn-plain-*`);
  - the new `hv-conn-tls-pin-changed-{title,body,old,new,replace}`;
  - the TR text from NOTES (M2a and Task 2), with `# Review: PO`.

  The dialog subscribes to `InputEvent::Cancelled` on its inputs, and its layer holds a strong clone until it answers (M2a modal rule).
- [ ] **Step 4:** Run Step 2's command, `cargo test -p hacivat-app --features scene-check,pseudo` (M2a suites green, with the `EncryptWithoutVerifying` tests replaced) and `cargo xtask l10n check`. Expected: PASS.
- [ ] **Step 5: Commit** with the message `feat(app): certificate review with pin and CA import, pin-changed hard stop, public-plaintext question, pinned row and key passphrase; remove "Encrypt without verifying"`.

**Review:** opus, adversarial (UI security). Probes:
- Can Enter on any focused element trust a changed certificate?
- Is the fingerprint ever truncated in a way that changes its meaning (the full 64 hex shown)?
- Does the Copy button copy exactly the fingerprint?
- Does `apply_changes` write to a different profile when two dialogs are open?
- Can any path other than an explicit "Replace pin" click produce `ReplacePin` (Q26)?
- Raised Ink: do fingerprint insets on the raised dialog use text-2 (Q34)?

### Task 20: SSH settings in the connection dialog and `~/.ssh/config` import (NOTES surface 6)

**Files:**
- Modify: `crates/app/hacivat-app/src/connections/dialog/ssh.rs`.
- Create: `crates/app/hacivat-app/tests/connections_ssh_form.rs`.
- Append: the FTL block `## m2b-t20-ssh`, and rewrite Task 1's `hv-conn-issue-ssh-*` placeholders (named exception; Q3). `CARGO_TARGET_DIR=<main>/target-core/m2b-t20`, `CARGO_INCREMENTAL=0` (Q1).

**Interfaces:**
- Consumes: Task 4's `read_ssh_config` (run on the core runtime) and `resolve_alias`; Task 12's profile fields and SSH issues; Task 1's `dialog::open_edit(id, DialogFocus, ..)` seam, its `fixes.rs` wiring of `ConnectThroughSsh` and the `ProfileDraft.ssh_secrets` save path (Q9, Q23); M2a's Disclosure, Checkbox, Segmented, Select, TextInput, PopupMenu and InlineNotice. This task edits only `dialog/ssh.rs`: never `dialog/mod.rs`, `form.rs` or `fixes.rs` (Task 19's files).
- Produces:
  - **`section(draft, ..)`.** The SSH disclosure with its summary: `hv-conn-ssh-sum-off`, `-sum-on` (`$host`) or `-sum-hops` (`$host`, `$n`). Inside:
    - the checkbox "Connect through an SSH tunnel" and Import from `~/.ssh/config…`;
    - SSH host and port, and SSH user;
    - auth as a Segmented (Agent / Key file / Password / Keyboard-interactive), with the fields for each method: the agent note with its key count (read off-thread), the key file with Choose…, the passphrase with Remember, the password with Remember (`ssh_retention`), and the keyboard-interactive note. Typed passwords and passphrases go into `ProfileDraft.ssh_secrets[hop]` (`SecretString`), which Task 1's `ConnectionsModel::save` stores under `SecretSlot::{SshPassword, SshPassphrase} { hop }` and Test passes as `ConnectRequest.draft_ssh` (Q23);
    - the jump hosts as an ordered list (number, host, user, remove; Add jump host; Alt+↑/↓ moves the focused row, through element-level `on_key_down` on the row, never `cx.bind_keys`: `menus::rebuild` owns bindings; Q34);
    - the route row ("this Mac → … → db:5432");
    - the Host keys Select (three options, never accept-all).
  - **The import menu** lists the aliases with their resolution. `ProxyCommand` entries are disabled with `hv-conn-ssh-proxycmd`; any other `unusable` entry is disabled with its reason. Picking an alias fills the fields, tints them (M2a paste tint) and shows `hv-conn-ssh-imported` (`$alias`). Config notices are listed in the notice.
  - **`open_with_ssh(id)`** calls `dialog::open_edit(id, DialogFocus::Ssh, ..)` (Task 1's seam): the edit dialog opens with the disclosure open and the checkbox on, and focuses the SSH host (Q9).
  - **SSH issues** render inside `section()`, matched by `IssueCode` (`SshHostMissing` beside the host, `SshUserMissing` beside the user, `SshKeyFileMissing` beside the key file, `SshTooManyHops` under the jump list), with the rewritten `hv-conn-issue-ssh-*` text. `ValidationIssue.field` has no SSH `FieldId`, so the M2a field renderer is not used (Q9).
  - **UI lessons (Q34):** the Global Constraints "Modal layers" rules apply (Escape, Enter and Tab bubble out of the nested Selects); tests wait with `settle_until`.
  - **`hv-conn-step-ssh`** is the trail label.

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn ssh_disclosure_summary(cx: &mut TestAppContext) { let vcx = dialog_new(cx); assert_eq!(summary(&vcx, "ssh"), t(&vcx, "hv-conn-ssh-sum-off"));
    set_ssh(&vcx, "bastion.acme.io", &["jump-eu.acme.io"]); assert_eq!(summary(&vcx, "ssh"), tf(&vcx, "hv-conn-ssh-sum-hops", &[("host", "bastion.acme.io"), ("n", "1")])); }
#[gpui::test] fn auth_segmented_switches_fields(cx: &mut TestAppContext) { let vcx = dialog_ssh_on(cx); pick_segment(&vcx, "ssh-auth", "key"); assert!(visible(&vcx, "ssh-key-file") && visible(&vcx, "ssh-passphrase"));
    pick_segment(&vcx, "ssh-auth", "kbd"); assert!(visible(&vcx, "ssh-kbd-note") && !visible(&vcx, "ssh-passphrase")); }
#[gpui::test] fn jump_host_list_keyboard(cx: &mut TestAppContext) { let vcx = dialog_ssh_on(cx); add_jump(&vcx, "j1", "u"); add_jump(&vcx, "j2", "u"); focus_row(&vcx, 1); press(&vcx, "alt-up");
    assert_eq!(hop_hosts(&draft(&vcx)), ["j2", "j1", ""]); click(&vcx, "ssh-jump-remove-0"); assert_eq!(hop_hosts(&draft(&vcx)), ["j1", ""]); }
#[gpui::test] fn route_row_draws_real_path(cx: &mut TestAppContext) { let vcx = dialog_ssh_full(cx); assert_eq!(text_of(&vcx, "ssh-route"), format!("{} → jump-eu.acme.io → bastion.acme.io → db-prod-eu.acme.io:5432", t(&vcx, "hv-conn-ssh-this-mac"))); }
#[gpui::test] fn host_key_policy_has_no_accept_all(cx: &mut TestAppContext) { let vcx = dialog_ssh_on(cx); assert_eq!(select_options(&vcx, "ssh-hostkeys"), ["ask", "accept_new", "strict"]); }
#[gpui::test] fn import_menu_lists_aliases_and_disables_proxycommand(cx: &mut TestAppContext) { let vcx = dialog_with_ssh_config(cx, "Host bastion\n HostName b.acme.io\n User deploy\nHost corp\n ProxyCommand nc %h %p\n");
    open_import(&vcx); assert!(menu_item_enabled(&vcx, "bastion")); assert!(!menu_item_enabled(&vcx, "corp")); assert_eq!(menu_hint(&vcx, "corp"), t(&vcx, "hv-conn-ssh-proxycmd")); }
#[gpui::test] fn import_fills_fields_and_shows_notice(cx: &mut TestAppContext) { let vcx = dialog_with_ssh_config(cx, BASTION_WITH_JUMP); open_import(&vcx); pick(&vcx, "bastion");
    assert_eq!(field(&vcx, "ssh-host"), "b.acme.io"); assert_eq!(hop_hosts(&draft(&vcx)).len(), 2); assert!(notice_text(&vcx).contains("bastion")); }
#[gpui::test] fn connect_through_ssh_fix_opens_dialog_with_ssh_open(cx: &mut TestAppContext) { let vcx = with_failed(cx, dns_error()); click_fix(&vcx, FixAction::ConnectThroughSsh);
    assert!(disclosure_open(&vcx, "ssh") && checkbox(&vcx, "ssh-use")); assert_eq!(focused_id(&vcx), "ssh-host"); }
#[gpui::test] fn ssh_issues_render_inline(cx: &mut TestAppContext) { let vcx = dialog_ssh_on(cx); press(&vcx, "cmd-s"); assert!(issue_visible(&vcx, "ssh-host", IssueCode::SshHostMissing)); }
#[gpui::test] fn typed_ssh_password_goes_into_the_draft_by_hop(cx: &mut TestAppContext) { let vcx = dialog_ssh_on(cx); pick_segment(&vcx, "ssh-auth", "password");   // Q23
    type_into(&vcx, "ssh-password", "pw0"); assert!(draft(&vcx).ssh_secrets[0].password.is_some()); assert!(!format!("{:?}", draft(&vcx)).contains("pw0")); }
#[gpui::test] fn alt_arrows_do_not_register_a_global_binding(cx: &mut TestAppContext) { let vcx = dialog_ssh_on(cx); assert!(!global_bindings(&vcx).iter().any(|b| b.contains("alt-up"))); }   // Q34
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test connections_ssh_form`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. The config file is read on the core runtime and the result is sent back over a `oneshot` (§5.4). Values that fail `carries_secret` are never shown.
- [ ] **Step 4:** Run Step 2's command, then `cargo test -p hacivat-app --features scene-check,pseudo --test connection_dialog`. Expected: PASS.
- [ ] **Step 5: Commit** with the message `feat(app): SSH tunnel section — auth per method, jump hosts, route row, host-key policy, ~/.ssh/config import`.

**Review:** opus. Probes:
- Can the import show a value that `carries_secret` refused?
- Can the keyboard reach every control (Tab, arrows)?
- Does the route row truncate before the overflow rule?

### Task 21: SSH and vault prompts (NOTES surface 6 new-host-key prompt; new surfaces 1–4; B11, B13)

**Files:**
- Modify: `crates/app/hacivat-app/src/connections/{hostkey_prompt,kbd_prompt,secret_prompt,vault_dialog}.rs`.
- Create: `crates/app/hacivat-app/tests/connections_prompts.rs`.
- Append: the FTL block `## m2b-t21-prompts`. `CARGO_TARGET_DIR=<main>/target-core/m2b-t21`, `CARGO_INCREMENTAL=0`. **Dispatch:** T21 starts when T19 or T20 finishes, unless `df` shows ≥ 60 GiB free (Q1).

**Interfaces:**
- Consumes: Task 14's `Question::{HostKey, Secret(Ssh*), Keyboard, UnlockVault, CreateVault}`; Task 4's `KnownHosts::replace` (run on the core runtime, through `ConnectionsModel::known_hosts()`, Q23); Task 6's vault (`create`, `reset`, `exists`, through `SecretStore::vault()`, Q23; `create` on the core runtime's blocking pool, Argon2, Q38); `SecretBackend::choice` through `SecretStore::backend_choice()`; `ConnectError.host_key`.
- Produces:
  - **`hostkey_prompt::open`:** 420 px. "First connection to `$host`", the algorithm and the `SHA256:` fingerprint inset, the store note, and the buttons Cancel and **Trust and connect** (primary).
  - **`hostkey_prompt::review_changed(mismatch)`** (new surface 1):
    - 480 px, primary Cancel (initial focus);
    - Hacivat source: "Remove the old key and trust the new one" calls `KnownHosts::replace` on the core runtime, then reconnects;
    - OpenSSH source: a copyable `ssh-keygen -R "[host]:port"` (or `ssh-keygen -R host` on port 22) and no replace button. After the user runs it, Retry works without a restart: `KnownHosts::check` re-reads the file on an `(mtime, len)` change (Task 4, Q24).
    - Hard stop: Cancel is the only Enter default (Q34).
  - **`kbd_prompt::open`** (new surface 2): the server's name and instructions verbatim, in their own fields; one TextInput per prompt (masked unless `echo`); "Attempt `$n` of 3"; answers in order.
  - **`secret_prompt::open`** (new surface 3): the M2a password-prompt anatomy, naming the hop (`$hop_host`, "hop `$n` of `$total`") or the key file; masked; Remember shows the profile's `ssh_retention`.
  - **`vault_dialog`** (new surface 4):
    - `CreateVault`: two masked fields, at least 8 characters and equal, and the "can't be recovered" note; it answers `Master`;
    - `UnlockVault { attempt }`: a masked field and "Attempt `$n` of 3", with the link "Forgot it?" to reset;
    - reset: type the word shown (`hv-conn-vault-reset-word`, in EN and TR), then `vault.reset()` on the core runtime;
    - `forever_label(cx)` returns `hv-conn-ret-forever-vault` when `secrets().backend_choice() == Some(Vault)`, and the M2a Keychain label otherwise.

    `ensure_vault_before_forever` (called by `ConnectionsModel::save` through Task 1's line) opens `CreateVault` before the first Forever save when `backend_choice() == Some(Vault)` and the vault does not exist. It returns `Ready` once the vault is created, and `Declined` when the user cancels. On `Declined`, the save keeps the secret Until restart, with the M2a toast.
  - Every input in these dialogs is masked, and `text_for_range` is masked too (the M2a I2 rule).
  - **UI lessons (Q34):** the Global Constraints "Modal layers" rules apply; fingerprint insets on raised surfaces use text-2 (Raised Ink). Core-runtime calls return through a `oneshot` the foreground task awaits (the Q4 ruling: allowed).
  - **Probe-dependent tests (Q34):** `forever_label_…` and `first_forever_save_…` inject `FallbackBackend { probe: || false }` over `MemoryBackend` (never the platform keyring) and wait with `settle_until` for `backend_choice()` to land (the `144b38c` pattern).

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn new_host_key_prompt(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx); let a = ask(&vcx, host_key_question("bastion.acme.io", &K1));
    assert!(text_of(&vcx, "hk-fingerprint").starts_with("SHA256:")); assert!(text_of(&vcx, "hk-store").contains("~/.ssh/known_hosts")); press(&vcx, "enter"); assert!(matches!(a.get(), Some(Answer::TrustHostKey))); }
#[gpui::test] fn host_key_changed_offers_replace_only_for_hacivat_store(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx);
    review_changed(&vcx, mismatch(KnownSource::Hacivat)); assert!(visible(&vcx, "hk-replace")); assert_eq!(focused_id(&vcx), "hk-cancel"); close_layers(&vcx);
    review_changed(&vcx, mismatch(KnownSource::OpenSsh { path: "/home/me/.ssh/known_hosts".into(), line: 14 })); assert!(!visible(&vcx, "hk-replace"));
    assert_eq!(text_of(&vcx, "hk-keygen"), "ssh-keygen -R \"[bastion.acme.io]:2222\""); }
#[gpui::test] fn kbd_prompt_renders_server_text_verbatim_and_masks_non_echo(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx);
    let a = ask(&vcx, kbd_question("Duo", "Kod girin", &[("Doğrulama kodu: ", false), ("Kullanıcı: ", true)]));
    assert_eq!(text_of(&vcx, "kbd-instructions"), "Kod girin"); assert!(is_masked(&vcx, "kbd-0") && !is_masked(&vcx, "kbd-1"));
    type_into(&vcx, "kbd-0", "123456"); type_into(&vcx, "kbd-1", "deploy"); press(&vcx, "enter"); assert!(matches!(a.get(), Some(Answer::Keyboard(v)) if v.len() == 2)); }
#[gpui::test] fn ssh_secret_prompt_names_the_hop(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx); ask(&vcx, ssh_password_question(1, "jump-eu.acme.io", 2));
    assert!(text_of(&vcx, "secret-title").contains("jump-eu.acme.io")); assert!(is_masked(&vcx, "secret-input")); }
#[gpui::test] fn vault_create_requires_matching_8_char_passwords(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx); let a = ask(&vcx, Question::CreateVault);
    type_into(&vcx, "vault-new", "short"); type_into(&vcx, "vault-repeat", "short"); assert!(!enabled(&vcx, "vault-create"));
    set_field(&vcx, "vault-new", "correct horse"); set_field(&vcx, "vault-repeat", "correct horse"); click(&vcx, "vault-create"); assert!(matches!(a.get(), Some(Answer::Master(_)))); }
#[gpui::test] fn vault_unlock_counts_attempts(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx); ask(&vcx, Question::UnlockVault { attempt: 2 });
    assert_eq!(text_of(&vcx, "vault-attempt"), tf(&vcx, "hv-conn-vault-attempt", &[("n", "2")])); }
#[gpui::test] fn vault_reset_needs_typed_confirmation(cx: &mut TestAppContext) { let vcx = shell_with_vault(cx); open_vault_reset(&vcx); assert!(!enabled(&vcx, "vault-reset-confirm"));
    type_into(&vcx, "vault-reset-word", &t(&vcx, "hv-conn-vault-reset-word")); click(&vcx, "vault-reset-confirm"); settle_until(&vcx, secs(2), || !vault_exists(&vcx)); }
#[gpui::test] fn forever_label_reads_encrypted_file_when_vault(cx: &mut TestAppContext) { let vcx = shell_with_fallback_probe_false(cx);   // injected FallbackBackend over MemoryBackend (Q34)
    settle_until(&vcx, secs(2), || backend_choice(&vcx) == Some(BackendChoice::Vault));
    assert_eq!(select_option_label(&vcx, "remember", "forever"), t(&vcx, "hv-conn-ret-forever-vault")); }
#[gpui::test] fn first_forever_save_asks_to_create_vault(cx: &mut TestAppContext) { let vcx = shell_with_fallback_probe_false_no_vault(cx);
    settle_until(&vcx, secs(2), || backend_choice(&vcx).is_some()); save_forever_profile(&vcx); settle_until(&vcx, secs(2), || layer_open(&vcx, "vault-dialog")); }
#[gpui::test] fn host_key_changed_enter_cancels(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx); review_changed(&vcx, mismatch(KnownSource::Hacivat));   // Q34
    press(&vcx, "enter"); assert!(!layer_open(&vcx, "hk-changed")); assert_eq!(replace_calls(&vcx), 0); }
#[gpui::test] fn prompt_inputs_mask_text_for_range(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx); ask(&vcx, Question::UnlockVault { attempt: 1 }); type_into(&vcx, "vault-unlock", "s3cret");
    assert!(!ime_text(&vcx, "vault-unlock").contains("s3cret")); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test connections_prompts`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. Ids: `hv-conn-hk-*` (M2a mock), plus `hv-conn-hk-changed-*`, `hv-conn-kbd-*`, `hv-conn-ssh-secret-*`, `hv-conn-vault-*` and `hv-conn-ret-forever-vault` (Task 2's NOTES). Server text is never interpolated into a translated sentence.
- [ ] **Step 4:** Run Step 2's command, `cargo test -p hacivat-app --features scene-check,pseudo` and `cargo xtask l10n check`. Expected: PASS.
- [ ] **Step 5: Commit** with the message `feat(app): host-key prompts (new and changed), keyboard-interactive, SSH secret prompts per hop, vault create/unlock/reset`.

**Review:** opus, adversarial. Probes:
- Can the replace button appear for an `~/.ssh` source?
- Is the `ssh-keygen` command shell-safe for hosts with spaces or quotes? (Refuse them; host names were validated.)
- Does a Cancel after a wrong master password leak attempts across gestures?
- Does any prompt keep a secret in a `SharedString`? (It must be in `SecretString`, and the field cleared on close.)
- Does any test touch the platform keyring, or wait a fixed time for the probe (Q34)?

### Task 22: Live-state UI: reconnecting, session lost, idle, read-only labels, SSH path (NOTES surface 7; new surfaces 6–7; B4, B19, B21)

**Files:**
- Modify: `crates/app/hacivat-app/src/connections/{session_toasts,live,capsule,sheet,status,explorer,boot}.rs` (in `boot.rs`, only the `ManagerConfig::m2b()` switch and the Linux `FallbackBackend`; Task 1 already builds `KnownHosts` and `TunnelManager`, Q23), `crates/app/hacivat-app/src/commands/debug.rs` (the debug-build cancel demo, Q39) and `crates/ui/hacivat-ui/src/capsule.rs` (`Reconnecting` rendering).
- **`live.rs` is not converted to polling** (Q4 ruling): its foreground mirror keeps awaiting `changes.changed()` on its `watch` channel, as M2a ships. Only the new states are added.
- Create: `crates/app/hacivat-app/tests/connections_live_m2b.rs`.
- Append: the FTL block `## m2b-t22-live`. `CARGO_TARGET_DIR=<main>/target-core/m2b-t22`, `CARGO_INCREMENTAL=0` (Q1).

**Interfaces:**
- Consumes: Task 17's `LiveState::Reconnecting` and `ConnectionEvent`; Task 18's `AccessLabel`, `access_warnings` and `remediation`; M2a's capsule, sticky toast, explanation popover and status segments.
- Produces:
  - **Boot.** Task 1's boot already puts `KnownHosts::load(~/.ssh/known_hosts{,2}, <config>/known_hosts)` (no I/O at boot: the files are read lazily on the core runtime at the first SSH connect, Q42) and `TunnelManager::new` into `ConnectDeps`. This task adds `FallbackBackend` (Linux only, `cfg(target_os = "linux")`), which wraps the keyring with `FileVault::open(<data>/vault.bin)`, and sets `ManagerConfig::m2b()`.
  - **Debug cancel demo (Q39).** In debug builds only, `commands/debug.rs` gains two palette commands: "Run `SELECT pg_sleep(30)` on the default session" and "Cancel" (`RequestTicket::cancel` on that request). B1 leaves no other user-visible cancel in M2b; the PO demo uses these.
  - **`CapsuleModel::Reconnecting`:** the run marker, `hv-capsule-reconnecting` (`$n`) and `hv-capsule-next` (`$seconds`, with the unit inside the message, INV-1). For production the filled block stays first, and there is exactly one filled badge.
  - **`session_toasts::lost`.** A sticky toast:
    - title `hv-conn-lost-title` (`$name`) and body `hv-conn-lost-body`;
    - `hv-conn-lost-tx` only when `open_tx`;
    - `hv-conn-lost-other` with a locale list of the other lost items (ICU list format, Turkish `ve`);
    - Show details (opens the M2a sheet section) and Dismiss.
  - **`session_toasts::idle`** shows one toast, `hv-conn-idle-disconnected` (`$name`, `$hours`); `$hours` is recomputed from `LivenessPolicy::for_profile` (the event carries only the profile). **`idle_tx`** shows a notice (M3 adds the prompt). **`gap(profile, reason)`** shows a sticky alert whose copy varies by `GapReason` (Q35): `TransactionEndedByStatement` → "Hacivat stopped a statement that ended the read-only transaction on a read-only connection and closed that session."; `CanaryAssigned` and `WriteSucceeded` → "…that changed data…"
  - **Read-only labels.** Capsule and sheet: `hv-conn-access-ro-enforced` vs `hv-conn-access-ro-guarded` (M2a). Guarded opens the 300 px popover with the warnings (`hv-conn-ro-warn-*`) and the remediation snippet in a mono inset with Copy.
  - **SSH path.** The capsule path segment is `hv-conn-path-ssh` (`$host`) plus latency. The explorer row gets the route flag. The status bar mirrors the capsule (state, path, encryption).
  - **UI lessons (Q34):** the remediation popover on the raised capsule uses text-2 for its inset (Raised Ink); tests wait with `settle_until`.

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn reconnecting_capsule_shows_attempt_and_countdown(cx: &mut TestAppContext) { let vcx = with_state(cx, LiveState::Reconnecting { attempt: 3, next_in: Some(secs(4)) });
    assert_eq!(capsule_text(&vcx, "run"), tf(&vcx, "hv-capsule-reconnecting", &[("n", "3")])); assert_eq!(capsule_text(&vcx, "next"), tf(&vcx, "hv-capsule-next", &[("seconds", "4")])); }
#[gpui::test] fn one_filled_badge_while_reconnecting_production(cx: &mut TestAppContext) { let vcx = with_prod_state(cx, LiveState::Reconnecting { attempt: 1, next_in: None }); assert_one_filled_badge(&vcx); }
#[gpui::test] fn session_lost_toast_lists_lost_items(cx: &mut TestAppContext) { let vcx = shell_with_profile(cx, pg_profile());
    emit(&vcx, ConnectionEvent::SessionLost { profile: pg_id(), lost: LostState { open_tx: true, temp_objects: true, session_vars: true, prepared: 0, locks: false } });
    let toast = sticky_toast(&vcx); assert!(toast.lines.iter().any(|l| l == &t(&vcx, "hv-conn-lost-tx"))); assert!(toast.actions.len() == 2); }
#[gpui::test] fn session_lost_toast_without_losses_shows_body_only(cx: &mut TestAppContext) { let vcx = shell_with_profile(cx, pg_profile());
    emit(&vcx, ConnectionEvent::SessionLost { profile: pg_id(), lost: LostState::default() }); assert_eq!(sticky_toast(&vcx).lines.len(), 1); }
#[gpui::test] fn turkish_lost_list_uses_ve(cx: &mut TestAppContext) { let vcx = shell_tr(cx); emit_lost_temp_and_vars(&vcx); assert!(sticky_toast(&vcx).lines.iter().any(|l| l.contains(" ve "))); }
#[gpui::test] fn idle_disconnected_toast_once(cx: &mut TestAppContext) { let vcx = shell_with_profile(cx, pg_profile()); emit(&vcx, ConnectionEvent::IdleDisconnected { profile: pg_id() }); emit(&vcx, ConnectionEvent::IdleDisconnected { profile: pg_id() });
    assert_eq!(toasts_with(&vcx, "hv-conn-idle-disconnected"), 1); }
#[gpui::test] fn read_only_labels_and_remediation_popover(cx: &mut TestAppContext) { let vcx = connected_with(cx, AccessLabel::ReadOnlyGuarded, vec![RoleWarning::Superuser]);
    click_capsule_segment(&vcx, "access"); assert!(popover_text(&vcx).contains("<role>")); assert!(button_visible(&vcx, "ro-copy-snippet"));
    let vcx2 = connected_with(cx, AccessLabel::ReadOnlyEnforced, vec![]); assert_eq!(capsule_text(&vcx2, "access"), t(&vcx2, "hv-conn-access-ro-enforced")); }
#[gpui::test] fn ssh_path_segment_and_explorer_flag(cx: &mut TestAppContext) { let vcx = connected_ssh(cx, "bastion.acme.io"); assert!(capsule_text(&vcx, "path").contains("bastion.acme.io"));
    assert!(explorer_row_has_flag(&vcx, "route")); assert_eq!(status_text(&vcx, "path"), capsule_text(&vcx, "path")); }
#[gpui::test] fn enforcement_gap_sticky_alert(cx: &mut TestAppContext) { let vcx = shell_with_profile(cx, pg_profile());
    emit(&vcx, ConnectionEvent::EnforcementGap { profile: pg_id(), reason: GapReason::TransactionEndedByStatement }); settle_until(&vcx, secs(2), || has_sticky_toast(&vcx));
    assert_eq!(sticky_toast(&vcx).role, Role::Alert); assert_eq!(sticky_toast(&vcx).lines[0], t(&vcx, "hv-conn-gap-tx-ended")); }   // one whole message per GapReason: hv-conn-gap-tx-ended / hv-conn-gap-changed-data (Q35)
#[gpui::test] fn debug_cancel_command_cancels_the_demo_sleep(cx: &mut TestAppContext) { let vcx = connected_with_fake_pg_sleep(cx);   // Q39
    run_command(&vcx, "debug-run-pg-sleep"); run_command(&vcx, "debug-cancel"); settle_until(&vcx, secs(2), || fake_cancel_calls(&vcx) == 1); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test connections_live_m2b`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies in Produces. Every count or duration is one message with its unit inside it (INV-1, M2a P28).
- [ ] **Step 4:** Run Step 2's command and `cargo test -p hacivat-app --features scene-check,pseudo`. Expected: PASS, with the M2a capsule and live tests green.
- [ ] **Step 5: Commit** with the message `feat(app): reconnecting capsule, session-lost and idle toasts, enforcement-gap alert, read-only labels with remediation, SSH path; boot wires known hosts, tunnels and the Linux vault`.

**Review:** opus. Probes:
- Is there a second filled badge in any state?
- Does the toast double on two quick reconnects? (One per event: the second replaces the first for the same profile.)
- Does boot touch the keychain, D-Bus or `known_hosts` files on the UI thread? (It must not, P36, Q42.)
- Are the debug cancel commands compiled out of release builds (Q39)?

### Task 23: Conformance suite over the Docker matrix (M2 exit criteria; Review Focus 1, 3, 4)

**Files:**
- Create: `crates/core/hacivat-session/tests/{docker_m2b_connect,docker_m2b_cancel_reconnect,docker_m2b_bypass,docker_m2b_tls_ssh}.rs`.
- Append: an ADR 0004 section, `## m2b-t23 — bypass-suite results`, with a table for each engine and version.

**Interfaces:**
- Consumes: everything core from W1–W4; the Task 16 targets; `ChaosProxy`.
- Produces: the evidence for gate criteria E1–E8. **Every TCP test runs twice: direct and through `sshd-a`** (`alg/01` §5). The PostgreSQL and MySQL rows run a third time through the `sshd-a` → `sshd-b` jump chain.
- **Targets (Q6):** every target comes from `require_target(..)` or `targets(Some(engine))`; no literal id appears outside a `require_target` call. A target outside the active set skips with its `eprintln!`; a target in the set never skips. Through SSH, connect to `DockerTarget.internal` (compose service and internal port), never the published `127.0.0.1` port.
- MySQL rows now verify pins, review and `TlsVerified::Pinned` through patch §5 (Q2); `tls_auto_pin_and_verify_ca` on `mysql84` is expected to pass.

- [ ] **Step 1: Write the tests.** They are RED until the stack and every earlier task are merged.
```rust
#[tokio::test] #[ignore = "docker"] async fn connect_matrix_direct_and_via_ssh() { for t in tcp_targets() { for path in [Path::Direct, Path::Ssh("sshd-a"), Path::Chain("sshd-a", "sshd-b")] {
    if matches!(path, Path::Chain(..)) && !t.in_pr_set() { continue; } let live = connect_main(&t, path).await.unwrap_or_else(|e| panic!("{} {path:?}: {:?}", t.id, e.class));
    assert_eq!(live.path, path.kind(), "{}", t.id); } } }
#[tokio::test] #[ignore = "docker"] async fn two_profiles_share_one_ssh_session() { let (m, tunnels) = manager_ssh_matrix().await; connect(&m, pg17_via("sshd-a")).await; connect(&m, mysql84_via("sshd-a")).await;
    assert_eq!(tunnels.sessions_open(), 1); }
#[tokio::test] #[ignore = "docker"] async fn cancel_matrix() { for (t, sql) in [("pg17", "SELECT pg_sleep(60)"), ("mysql84", "SELECT SLEEP(60)"), ("mariadb114", "SELECT SLEEP(60)")] {
    let Some(t) = require_target(t) else { continue }; for path in [Path::Direct, Path::Ssh("sshd-a")] {
    let s = session_on(&t, path).await; let q = s.submit(user_exec(sql)); tokio::time::sleep(ms(600)).await; let start = Instant::now();   // past the 500 ms MySQL pre-warm (Q37)
    assert_eq!(q.cancel(&s).await.unwrap(), CancelOutcome::Cancelled, "{} {path:?}", t.id); assert!(start.elapsed() < secs(1)); assert_eq!(q.reply.await.unwrap().unwrap_err().kind, ExecErrorKind::Cancelled);
    assert!(run(&s, "SELECT 1").await.is_ok()); } } assert_sqlite_long_cte_interrupted().await; }
#[tokio::test] #[ignore = "docker"] async fn terminated_backend_with_open_tx_reports_lost_tx() { for t in ["pg17", "mysql84", "mariadb114"].into_iter().filter(|id| require_target(id).is_some()) { let (m, mut ev) = manager_connected(t).await; let s = default_session(&m);
    run(&s, "BEGIN").await.unwrap(); run(&s, "CREATE TEMPORARY TABLE hv_tmp(a int)").await.unwrap(); kill_backend(t, s.backend_id()).await;
    let lost = wait_session_lost(&mut ev).await; assert!(lost.open_tx && lost.temp_objects, "{t}"); assert!(matches!(m.state(pid(t)), LiveState::Connected(_))); } }
#[tokio::test] #[ignore = "docker"] async fn insert_killed_mid_flight_reports_outcome_unknown_without_duplicate() { let (m, _) = manager_connected_via_chaos("mysql84").await; let s = default_session(&m);
    let q = s.submit(user_exec("INSERT INTO hv_probe_log(v) SELECT SLEEP(1) + 1")); tokio::time::sleep(ms(300)).await; chaos("mysql84").cut(secs(2));
    assert_eq!(q.reply.await.unwrap().unwrap_err().kind, ExecErrorKind::ConnectionLost { outcome_unknown: true }); wait_connected(&m, pid("mysql84")).await;
    assert!(count(&admin("mysql84").await, "SELECT count(*) FROM hv_probe_log").await <= 1); }
#[tokio::test] #[ignore = "docker"] async fn server_outage_reconnects_within_backoff() { let (m, _) = manager_connected_via_chaos("pg17").await; chaos("pg17").cut(secs(20)); wait_state(&m, |s| matches!(s, LiveState::Reconnecting { .. })).await;
    timeout(secs(45), wait_connected(&m, pid("pg17"))).await.unwrap(); }
#[tokio::test] #[ignore = "docker"] async fn pg_bypass_rows_blocked_without_classifier() { for t in targets(Some(DbmsKind::Postgres)).iter().map(|t| t.id.as_str()).filter(|id| !id.contains("-")) { let s = ro_session_as(t, "hv_ro").await;   // pg14..pg18 in the active set (Q6) let admin = admin(t).await;
    for (id, sql) in PG_SERVER_BLOCKED { let r = run(&s, sql).await; assert!(r.is_err(), "{t} {id}: {sql}"); assert_no_side_effect(&admin, id).await; }   // P1 P2 P3 P4 P5 P7 P8 P14(COPY FROM) P26 P37 P38
    for (id, sql) in PG_NEUTRALISED { let _ = run(&s, sql).await; assert_no_lingering_state(&admin, &s, id).await; }                      // P6 P15(advisory) P25(NOTIFY) P26(LISTEN, WITH HOLD)
    for (id, sql) in PG_PRIVILEGE_ONLY { assert!(run(&s, sql).await.is_err(), "{t} {id}"); } } }                                           // P11 P13 P14(read) P16 P31 P33 P34 with hv_ro
#[tokio::test] #[ignore = "docker"] async fn mysql_bypass_rows_blocked_without_classifier() { for t in targets(Some(DbmsKind::MySql)).iter().map(|t| t.id.as_str()) { let s = ro_session_as(t, "hv_ro").await;   // Q6 let admin = admin(t).await;
    for (id, sql) in MYSQL_SERVER_BLOCKED { assert!(run(&s, sql).await.is_err(), "{t} {id}: {sql}"); assert_no_side_effect(&admin, id).await; }   // M1 M2 M5(FOR UPDATE) M6 M8 M17
    run(&s, "SET SESSION TRANSACTION READ WRITE").await.ok(); assert!(run(&s, "CREATE TABLE hv_m9(a int)").await.is_err(), "{t} M9");
    let _ = run(&s, "SELECT GET_LOCK('hv_m5', 0)").await; assert_eq!(scalar(&admin, "SELECT IS_FREE_LOCK('hv_m5')").await, "1", "{t} M5 GET_LOCK"); } }
#[tokio::test] #[ignore = "docker"] async fn mariadb_bypass_rows_recorded() { for t in targets(Some(DbmsKind::MariaDb)) { let rows = record_bypass_results(&t, MYSQL_SERVER_BLOCKED).await;
    assert!(rows.iter().all(|r| r.result != BypassResult::Unknown), "{}", t.id); } }   // every row has a recorded result (Q32); results → ADR 0004; label stays guarded (C19)
#[tokio::test] async fn sqlite_bypass_rows_blocked_without_classifier() {   // no server: runs on every PR (Q32) let (s, dir) = ro_sqlite_session().await;
    for sql in ["INSERT INTO t VALUES (1)", "ATTACH 'n.db' AS n", "VACUUM INTO 'c.db'", "PRAGMA query_only = 0", "CREATE TEMP TABLE x(a)", "SELECT writefile('w', 'x')"] { assert!(run(&s, sql).await.is_err(), "{sql}"); }
    assert_eq!(files_in(&dir), ["db.sqlite"]); }
#[tokio::test] #[ignore = "docker"] async fn tls_auto_pin_and_verify_ca() { for t in ["pg17-tls", "mysql84"].into_iter().filter_map(require_target) { let ui = ScriptedUi::answers_m2b([Answer::Pin]);   // MySQL pins via patch §5 (Q2)
    let live = connect_main_ix(&t, TlsMode::Auto, &ui).await.unwrap(); assert_eq!(live.tls.verified, TlsVerified::Pinned, "{t}"); assert_eq!(live.profile_changes.len(), 1); }
    assert!(connect_mysql_verify_ca("mysql84").await.is_ok()); }
#[tokio::test] #[ignore = "docker"] async fn insecure_auth_refused_through_ssh_on_md5() { let e = connect_via_ssh_as("pg17-auth", "hv_md5", "sshd-a").await.unwrap_err();
    assert_eq!(e.class, ErrClass::AuthInsecureMethod); assert!(connect_via_ssh_as_disable("pg17-auth", "hv_md5", "sshd-a").await.is_ok()); }
#[tokio::test] #[ignore = "docker"] async fn unencrypted_badge_cases_on_loopback() { let live = connect_main(&target("pg17"), Path::Direct).await.unwrap(); assert!(!live.tls.encrypted); assert_eq!(live.address, AddressClass::Loopback); }
```
- [ ] **Step 2:** Run `cargo xtask docker test --set full -p hacivat-session`. Expected: FAIL before W4 is merged; PASS after. Any failure is triaged as an earlier task's defect and routed to its owner (`fix round`), never patched here.
- [ ] **Step 3: Implement** the helpers in `tests/common/mod.rs` (appended), and the row constants with ids that match SQL-SAFE-06 (`PG_SERVER_BLOCKED: &[(&str, &str)]`, and the others).
- [ ] **Step 4:** Run Step 2's command three times. Expected: three green runs (flake check). Write the bypass table into ADR 0004.
- [ ] **Step 5: Commit** with the message `test(session): M2b conformance over the Docker matrix — connect direct/SSH/chain, cancel, reconnect, bypass suite without classifier, TLS pin, insecure auth`.

**Review:** opus, adversarial. Probes:
- Is any bypass row asserted only by "error happened", without a side-channel check?
- Do the MariaDB results claim "enforced"?
- Does a test pass by skipping when a target is missing? (M2a ruling: a full-set test must not silently skip; Q6: `require_target` panics for an unhealthy in-set target.)
- Is any literal target id used outside `require_target` (Q6)?

### Task 24: Shots, cross-surface tests, DESIGN.md (W6)

**Files:**
- Modify: `crates/app/hacivat-app/tests/shots.rs` (append the M2b surfaces; it stays `harness = false`) and `DESIGN.md` (append the section "Secure Network and Sessions (M2b)").
- Create: `crates/app/hacivat-app/tests/cross_surface_m2b.rs`. `CARGO_TARGET_DIR=<main>/target-core/m2b-t24`, `CARGO_INCREMENTAL=0` (Q1). Waits use `settle_until` (Q4).

**Interfaces:**
- Consumes: every M2b `*_fixture` that Tasks 19–22 wrote, behind the `shots` feature (M2a R28).
- Produces:
  - **Shots** of every M2b surface in en, tr, en-XA and tr-XT, dark and light, at 1280×800 and 960×600, with 0 clip violations:
    - the certificate review, pin changed and plaintext question;
    - the SSH section (off, single hop, two jumps, import menu);
    - the host key (new and changed) and keyboard-interactive prompts;
    - the SSH secret prompt;
    - vault create, unlock and reset;
    - the reconnecting capsule (production and not);
    - the session-lost toast (with and without losses);
    - the idle toast and the gap alert;
    - read-only enforced and guarded, with the popover;
    - the SSH path in the capsule and the status bar;
    - the new error copy.
  - **Cross-surface flows** (FakeDriver and the in-process SSH server):
    - `untrusted_then_pin_then_capsule_shows_pinned`;
    - `ssh_connect_with_new_host_key_then_capsule_via_bastion`;
    - `reconnect_flow_shows_reconnecting_then_lost_toast`;
    - `first_forever_save_on_vault_machine_creates_vault`;
    - `connect_through_ssh_fix_end_to_end`.
  - **DESIGN.md:** the new components (fingerprint changed, keyboard-interactive, vault dialogs, reconnecting capsule, lost toast, read-only popover), with their tokens. No new hue.

- [ ] **Step 1:** Write the cross-surface tests, then run `cargo test -p hacivat-app --features scene-check --test cross_surface_m2b`. Expected: FAIL until the fixtures exist, then PASS.
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features shots,scene-check,pseudo --test shots`. Expected: 0 clip violations, and the M2a shots unchanged (diff gate 0 % for every M2a shot not touched by M2b).
- [ ] **Step 3:** Update DESIGN.md.
- [ ] **Step 4:** Run the full app set (Global Constraints line 4). Expected: PASS.
- [ ] **Step 5: Commit** with the message `test(app): M2b shots and cross-surface flows; DESIGN.md secure-network components`.

**Review:** opus (design and a11y). Probes:
- Can every new dialog be completed from the keyboard alone?
- Do screen-reader roles match (alertdialog for hard stops)?
- Does any TR string overflow at 960 px?

### Task 25: M2b gate (W7)

**Files:**
- Create: `docs/benchmarks/m2b.md` and `docs/benchmarks/po-demo-m2b.md`, and `design/baselines/m2b/macos/*.png` after the PO signs off.

**M2 exit criteria (spec §6 M2), checked at the end of M2b.** Each is PASS or FAIL in `m2b.md`, with evidence:

| # | Criterion | Evidence |
|---|---|---|
| E1 | Connect works against the Docker matrix, direct and through SSH (single hop and jump chain), for PostgreSQL 14–18, MySQL 8.0/8.4, MariaDB 10.6/10.11/11.4 and SQLite | T23 `connect_matrix_direct_and_via_ssh`; M2a T19 suite re-run |
| E2 | **Reconnect** works: outage, terminated backend and SSH death recover with replay; the lost state is reported; a user statement is never replayed | T23 `server_outage_reconnects_within_backoff`, `terminated_backend_with_open_tx_reports_lost_tx`, `insert_killed_mid_flight_…`; T17 `one_ssh_death_…` |
| E3 | **Cancel** works on every engine within 1 s, direct and through SSH; stale cancels never hit the next statement | T23 `cancel_matrix`; T7 `stale_cancel_never_hits_the_next_request` |
| E4 | **SQL-SAFE-06 server-blocked rows are blocked with the classifier disabled** (L3–L6 stand alone) on PostgreSQL 14–18, MySQL 8.0/8.4 and SQLite; MariaDB results recorded | T23 bypass tests; the ADR 0004 table |
| E5 | Read-only guards for all three engines, with the role probe and honest labels (MariaDB guarded, C19) | T18 docker tests; T22 label shots |
| E6 | TLS `Auto` flows: pin, CA import, the unencrypted badge, the public-plaintext question, pin-changed hard stop; "Encrypt without verifying" removed | T3, T14, T19, T23 `tls_auto_pin_and_verify_ca` |
| E7 | SSH: host-key policies, known hosts, agent, encrypted key, keyboard-interactive (in-process), `~/.ssh/config` import, shared sessions | T4, T9, T20, T21, T23 `two_profiles_share_one_ssh_session` |
| E8 | Insecure-auth guard (PG `R` 3/5, MySQL key retrieval), channel binding, MySQL VerifyCa | T10, T13, T23 `insecure_auth_refused_through_ssh_on_md5` |
| E9 | Secrets: the vault on Linux CI, keyring CI on Linux and Windows, live keyring on the PO's Mac; no secret at rest outside the keyring or vault (`connections.toml`, `known_hosts`, logs) | CI `vault-linux`, `keyring-linux`, `keyring-windows`; `live_keyring_round_trip` ×2; a grep after the demo |
| E10 | The Docker matrix in CI: `docker-pr` green on the PR; `docker-full` green nightly; every image pinned by digest | CI run links; `xtask docker check-pins` |
| E11 | Shots are clean, and the PO signs off the M2b surfaces and the Turkish strings | T24; PO |
| E12 | Gates are green: clippy `-D warnings`, the full test set, `check-deps` (with `GuardNotRuntimeFree`), `l10n check`, `tokens --check`, `cargo deny`, and `cargo tree -i aws-lc-rs` empty | the Step 2 output |
| E13 | Budgets: first frame ≤ 500 ms and idle RSS ≤ 200 MB with no connection; idle RSS with one direct and one SSH connection recorded | `--smoke` ×3 |
| E14 | Driver stream injection verified (§5.7): PostgreSQL over SSH channel streams, MySQL through the forwarder over SSH | T14 `mysql_over_ssh_uses_forwarder`; T23 |

- [ ] **Step 1:** Make sure W6 is merged on `m2b`. Run `cargo clean -p hacivat -p hacivat-app -p hacivat-session -p hacivat-net` (and the same with `--release`) in `<main>/target`.
- [ ] **Step 2:** Run:
  ```
  cargo clippy --workspace --all-targets -- -D warnings \
    && <full test set, lines 1–8> \
    && cargo xtask check-deps && cargo xtask l10n check && cargo xtask tokens --check \
    && cargo deny --all-features check licenses bans sources \
    && test -z "$(cargo tree -i aws-lc-rs --workspace 2>/dev/null)" \
    && cargo xtask docker test --set full
  ```
  Expected: all green. Then push `m2b` and record the CI runs (`docker-pr`, and a manual `docker-full`, `keyring-linux`, `keyring-windows`, `vault-linux`).
- [ ] **Step 3:** On the PO's Mac, run `cargo run -p hacivat --release -- --smoke` 3 times, and the live keyring test twice (E9). Record idle RSS with one SSH connection (demo step 6).
- [ ] **Step 4: Write `po-demo-m2b.md`.** The PO runs it with `cargo xtask docker up --set full`; the gate agent pre-runs it and records the results:
  1. `pg17-tls` in Auto: the certificate dialog. Check the fingerprint against `docker/m2a/README.md`. Pin. The capsule shows "TLS 1.3 · pinned".
  2. Restart `pg17-tls` (its certificate regenerates): connect again, and the pin-changed hard stop shows the old and new fingerprints. Cancel is the default; Replace pin; connected.
  3. `mysql84` in VerifyCa with the container CA: connected (E8).
  4. The public-plaintext question cannot be reached against the local Docker matrix (every target is loopback). Show T19's `plaintext-prompt` shots and the cross-surface test instead (Q39: no `demo-fakes` feature is built).
  5. A new profile `pg17` through SSH: Import from `~/.ssh/config` (a sample is in the README) → `sshd-a`. Connect: the new-host-key prompt; Trust. The capsule shows "SSH via 127.0.0.1" and latency. Connect `mysql84` through the same bastion. Then run `docker exec m2a-sshd-a ss -tn | grep -c ESTAB` and check the shared session count.
  6. A jump chain `sshd-a` → `sshd-b` → `pg17` with the encrypted client key: two passphrase prompts naming each hop; Remember Forever; disconnect and reconnect, with no prompt.
  7. Change `sshd-a`'s host key (`docker exec m2a-sshd-a /regen-hostkey.sh`): the host-key-changed dialog offers "Remove the old key and trust the new one" (Hacivat store).
  8. A read-only profile as `hv_ro`: "read-only (enforced by server role)". As `hv_test`: guarded, with the popover and the snippet.
  9. Reconnect: `SELECT pg_terminate_backend(<pid>)` from an admin `psql` on the default session's backend. The capsule shows Reconnecting, then Connected; the "Reconnected to …" toast appears.
  10. Idle: set `idle_disconnect_s = 60` by hand in `connections.toml`, wait 61 s, and the idle toast appears.
  11. Switch EN↔TR on each new dialog: no English leaks.
  12. Quit while connected through SSH: the app exits within 2 s.
  13. Cancel (debug build, Q39): palette "Run `SELECT pg_sleep(30)` on the default session", then "Cancel". The statement ends with "cancelled" within 1 s and the session stays usable.
- [ ] **Step 5: Linux and Windows.** Record the CI keyring and vault jobs, and the `docker-full` results. If the PO provides hosts, run demo steps 1, 5 and 6 there. Otherwise list them as open items.
- [ ] **Step 6:** The PO reviews the shots and the Turkish strings. After sign-off, copy the macOS PNGs to `design/baselines/m2b/macos/`.
- [ ] **Step 7: Write `m2b.md`:**
  - the E1–E14 table;
  - the rulings B1–B31 for PO confirmation, especially B1 (UI deferrals to M3), B5 (cert pins rather than SPKI), B8 (md5 private servers now need Disable), B10 (vendored `mysql_async`), B13 (vault params), B14 (secure input in M9) and B17 (no toxiproxy, no PgBouncer);
  - the resolved image digests;
  - the bypass-suite table (a link to ADR 0004);
  - the open items with owners:
    - upstream PRs for the five `mysql_async` patches (PO);
    - M2a profiles set to Require by "Encrypt without verifying" now hit `auth.insecure_method` (PostgreSQL md5/password: always; MySQL: on a `caching_sha2` cache miss); the fix is Review certificate → Pin (PO, Q20);
    - widen MySQL's Auto plaintext fallback from loopback to Private now that patch §2 removes M2a's reason (PO question, Q41);
    - RUSTSEC-2023-0071 (`rsa` via russh) is a documented acceptance, not a CI pass (`deny` checks no advisories; Q45);
    - the MariaDB "enforced" label decision (PO, C19);
    - the PO review of every M2b TR string;
    - the controller-approved M2b mocks (B28);
    - the Docker directory rename (M9);
    - OS sleep hooks and App Nap (M9);
    - secure event input (M9);
    - PgBouncer and C11 (M8);
    - SQLite WAL readers (M4).
- [ ] **Step 8: Commit** with the message `docs(m2b): gate record, PO demo checklist, approved secure-network baselines`.

---

## Self-review notes

- **Spec coverage (L15 M2b).** Every L15 item has an owning task:

  | L15 item | Tasks |
  |---|---|
  | TLS `Auto` flows: pin, CA import, unencrypted badge | T3, T14, T19, T22 |
  | SSH | T4, T9, T14, T20, T21 |
  | Session actor, transactions, cancel | T7, T15 |
  | `hacivat-guard` read-only layers with the role probe | T5, T11, T13, T18 |
  | Reconnect with state replay | T17, T22 |
  | Docker matrix in CI | T16, T23 |
  | M2 exit criteria | T25 E1–E14 |

  HV-SESS-01..08 are covered in core. Smart commit, startup scripts and the session UIs are deferred under B1, which is listed for the PO.
- **Ledger deferrals.** Each has a row in "Ledger items deferred to M2b". Secure event input is explicitly not M2b (M9).
- **Type consistency.** These names are used identically in every task that touches them, as defined in Task 1:
  - `Question`/`Answer` variants, `ProfileChange`, `InsecureAuth::RefuseUnlessVerified`;
  - `TlsVerified::Pinned`, `TlsFailure::PinMismatch`, `CertPin { endpoint, sha256 }`, `cert_review`;
  - `HostKeyCheck`, `KnownSource`, `TunnelManager::acquire`, `TransportGeneration`;
  - `SessionHandle::{submit, cancel(seq), escalate(seq, e)}`, `RequestTicket::cancel`, `ExecErrorKind::ConnectionLost { outcome_unknown }`, `LiveState::Reconnecting { attempt, next_in }`, `ConnectionEvent::*` (with `EnforcementGap { profile, reason }`);
  - `run_guarded(.., replay)`, `access_for`, `plan_for`, `check_post`;
  - `SessionSecrets`, `SessionOverlay`, `ConnectionManager::update_profile`, `SecretBackend::choice`, `SecretStore::vault`, `require_target`, `dialog::open_edit`/`DialogFocus`.

  The `read_ssh_config` signature carries the `is_secret` predicate (Task 4 Consumes). Task 1's stub must use that exact signature.
- **Conflict control.** Files that two tasks would both want are split by ownership:
  - `steps.rs` (T14) vs `readonly::access_for` (T18), through T1's call site;
  - `interaction.rs` and `events.rs` (T1, working routing) vs the dialog modules;
  - `dialog/form.rs`, `dialog/mod.rs` and `fixes.rs` (T19) vs `dialog/ssh.rs` (T20), through T1's hook line, `open_edit` seam and `fixes.rs` wiring (Q9);
  - `model.rs` (`apply_changes`, T19) vs the `vault_dialog::ensure_vault_before_forever` call (one T1 line in `save`, body filled by T21);
  - `boot.rs` (T22 only).

  The FTL blocks are per task. The two named exceptions are listed in Global Constraints.
- **Proportion.** The code blocks are test names, assertions and signatures. Bodies are given only for exact SQL and the values the spec fixes (guard plans, probe SQL, header layout, backoff schedule).
- **Known risks** (also in the report to the PO):
  - russh's feature names for a ring-only build must be verified in T1 (BLOCKED path defined);
  - the vendored `mysql_async` must track upstream security fixes by hand until the PRs land;
  - disk is the binding constraint: the admission rule (≥ 25 GiB per gpui agent, ≥ 8 GiB per core agent), `CARGO_INCREMENTAL=0`, per-task directories deleted at merge, and at most two gpui agents unless ≥ 60 GiB are free (Q1);
  - the M2a `ConnectionManager` rewrite (T17) touches the code the capsule depends on, so the M2a manager and live tests are the regression net;
  - B8 changes behaviour for md5-only private servers.

---

## Appendix: pre-flight rulings applied (M2b)

The pre-flight review (`<main>/.superpowers/sdd/2026-11-13-m2b-secure-network-sessions/preflight.md`, Q1–Q46) was adopted in one amend pass before T1's dispatch. **Q4 was overridden by the controller**: awaiting `tokio::sync` channels from GPUI foreground tasks stays allowed; determinism comes from `allow_parking()` in shared test helpers and condition-based waits; nothing is converted to polling. Q39 took the "point to the shots" option (no `demo-fakes` feature).

| # | What changed | Where |
|---|---|---|
| Q1 | Disk admission (≥ 25 GiB gpui, ≥ 8 GiB core), `CARGO_INCREMENTAL=0`, T1 in `<main>/target`, others in `target-core/m2b-tNN` deleted at merge, ≤ 2 gpui agents unless ≥ 60 GiB | Global Constraints; Waves; T1, T19–T22, T24 |
| Q2 | Patch §5 `with_rustls_client_config`; §2 becomes the outcome predicate `password_guard`; MySQL uses `TlsParams.config` | Architecture; Tech Stack; B5, B8, B10; T1 (vendored-copy note); T3; T8; T13; T23 |
| Q3 | T1 Files list every broken match, literal, rename and the five placeholder issue messages; named exception for T19/T20 | T1 Files; Global Constraints |
| Q4 | **Overridden:** awaiting `tokio::sync` channels is allowed; `allow_parking()` in shared helpers; `settle_until` condition waits; no polling; T22 does not convert `live.rs` | Global Constraints (Threading, Test timing); T1; T22 |
| Q5 | Manifest rows (testkit `ssh` on test crates, tokio `test-util`, testkit → net, driver tokio `sync`, pkcs8 `pem`, guard re-exports); compile without features; `#[cfg(unix)]` gating | T1 Step 3; Global Constraints; T4, T6, T9, T12 |
| Q6 | `require_target` (T1), `service`/`internal_port`/`internal` and `tests/targets.rs` (T16); no literal ids | Global Constraints; T1; T10, T13, T16, T18, T23 |
| Q7 | T13's Docker run precedes T16 Step 5; idempotent `hv_ro`/`hv_probe`; controller re-runs | Waves; Global Constraints; T13; T16 |
| Q8 | `plan_for` ships working in T1; T5 tests it | Waves; T1; T5; T7 |
| Q9 | `dialog::open_edit(id, DialogFocus)` seam and two `fixes.rs` lines in T1; T20 renders SSH issues by `IssueCode` | T1; T19; T20 |
| Q10 | PostgreSQL reset = `DISCARD ALL` minus `DEALLOCATE ALL`, plus by-name `from_sql` deallocation; enum test | B21; T1; T5; T10 |
| Q11 | Observer records the `BackendKeyData` PID | T1; T10 |
| Q12 | Observer layering, `Z` counter after errors, notices via `poll_message` | T10 |
| Q13 | A pin forbids the plaintext fallback | B5; T1; T3; T14 |
| Q14 | Read-only sessions always Auto; Manual/Smart/Begin `Unsupported` | B22; T7; T18 |
| Q15 | `run_guarded(.., replay)` replays the full environment plan | B21; T1; T7; T13; T18 |
| Q16 | `query_only`, `journal_mode`, `trusted_schema`, `foreign_keys` readable without argument | T5; T11 |
| Q17 | Session overlay, `SessionSecrets`, `ConnectionManager::update_profile`, interaction-needing reconnect → `Lost` | Global Constraints; B19, B23; T1; T14; T15; T17; T19 |
| Q18 | `cancel(seq)`, `escalate(seq, e)`, `RequestTicket::cancel` | B23; T1; T7; T15; T23 |
| Q19 | A single-session borrow never resets | T1; T15 |
| Q20 | Outcome-based guard for both engines; `auth.insecure_method` carries the certificate; PO note | B8; T13; T14; T25 |
| Q21 | No `NOINHERIT`; TEMP is a warning only | B17; T5; T16; T18 |
| Q22 | Canonical roots, verbatim-disk strip, typed-path text rules, missing files via parent | B26; T12 |
| Q23 | `SecretBackend::choice`, `SecretStore::vault`, Fallback probe semantics, SSH secrets in draft/request/save, model accessors, `TestSshServer` API with `Redirect`, `FakeExec` consume-once, `Default` derives, `pub` helpers, `fx_at`/`rule_of` test | T1; T6; T9; T14; T20; T21 |
| Q24 | `KnownHosts` re-reads OpenSSH files on `(mtime, len)` change | B11; T1; T4; T21 |
| Q25 | No `start_paused` with real sockets; T9 uses a 200 ms real grace | Global Constraints; T9; T17 |
| Q26 | `cert_dialog::review` builds `ReplacePin`; the pipeline never does | T1; T14; T19 |
| Q27 | Re-probe only after user statements, user tx ends and `SwitchSchema`; nothing at spawn; T17 uses `SELECT 42` | T7; T17 |
| Q28 | russh `default-features = false, features = ["ring", "rsa", "flate2"]`; `cargo tree` records | Tech Stack; Global Constraints; T1 |
| Q29 | Vendored-copy hygiene; `Cargo.lock` entry change listed; T8 owns the `docker_mysql.rs` ignore | Global Constraints; T1; T8 |
| Q30 | Vault header bounds, `SecretString` JSON, `getrandom`; test renames | Tech Stack; B13; T1; T6; T17 |
| Q31 | Clock jump also on monotonic delta; idle compared on wall time after sleep | B18; T17 |
| Q32 | Real asserts in weak tests (F9) | T1, T4, T7, T9, T13, T23 |
| Q33 | Review Focus tests; `capture_tracing` at TRACE with `LogTracer` | Review Focus; T1 table; T3; T9; T14 |
| Q34 | Key phases, hard-stop Enter = Cancel, element-level row keys, Raised Ink, probe-waiting tests | Global Constraints (Modal layers); T19–T22 |
| Q35 | MySQL/SQLite `command_tag`; gap copy by `GapReason`; transaction keywords denied on read-only | B21; T1; T11; T13; T17; T18; T22 |
| Q36 | pkcs8 `sha1-insecure`; ECDSA-SHA512 and RSA-PSS channel-binding hashes | Tech Stack; B9; T1; T3 |
| Q37 | MySQL side connection pre-warmed at 500 ms; `KILL` limits in ADR 0004 | B23; T7; T23 |
| Q38 | `CreateVault` on the connect path; vault errors on Forever put fall back; create on the blocking pool | B13; T6; T14; T21 |
| Q39 | Debug-build cancel commands (T22); demo step 4 points to shots; step 13 shows cancel | Ownership; T22; T25 |
| Q40 | Ownership list synced with task Files | Waves (file ownership) |
| Q41 | MySQL Auto fallback widening is a PO question | B10; T25 |
| Q42 | `KnownHosts::load` lazy; `Lagged` re-reads state | B11; T1; T4; T22 |
| Q43 | `IdentityAgent` notice, `HostName %h`, one passphrase per hop | B11, B12; T1; T4 |
| Q44 | T14 SNI compare, plaintext test rename, Resolve skipped under SSH, `acquire` outside `clock.run` | B11; T14 |
| Q45 | `m2b` cut after `m2a-finalfix` merges; ledger records `144b38c` and the advisory acceptance | Execution; Global Constraints; T25 |
| Q46 | SSH channel hygiene probes and tests | B11; T1; T9 |

The pre-flight reviewer's checklist (kept for the record):
1. russh's current version and features build ring-only, with the client, agent client, server (testkit) and agent server. *(Not verifiable offline; Q28; T1 keeps the BLOCKED path.)*
2. `tokio-postgres` 0.7.x exposes `ChannelBinding::tls_server_end_point`, `CancelToken::cancel_query_raw` with a custom `TlsConnect`, `RowStream::rows_affected`, and notice delivery through the `Connection` stream. *(Yes; no public PID, Q11; type-info cache vs `DEALLOCATE ALL`, Q10.)*
3. rusqlite 0.42 exposes `SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE`/`_WRITE` (feature `modern_sqlite`), `set_limit(SQLITE_LIMIT_ATTACHED)` and the authorizer `AuthAction` set that Task 11 maps. *(Yes; Q16.)*
4. The vendored `mysql_async` builds as a `[patch]` path crate with the dev sections removed, and `cargo deny` accepts it. *(Yes with `exclude`; Q29; TLS needs §5, Q2.)*
5. The `pkcs8` encryption features are licence-clean, and their PBES2 support covers OpenSSL's default `-v2 aes-256-cbc` output. *(Yes; add `pem`, `sha1-insecure`; Q5, Q36.)*
6. The `gnome-keyring` unlock recipe works on `ubuntu-24.04` runners. *(Not verifiable offline; the job asserts `probe_available()` first.)*
7. Disk headroom. *(Q1.)*
8. Every interface name in Task 1 matches its use in Tasks 3–24. *(Q3, Q9, Q11, Q15, Q17, Q18, Q23, Q26, Q44.)*
