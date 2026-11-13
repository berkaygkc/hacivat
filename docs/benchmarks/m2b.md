# M2b gate record (draft)

Milestone M2b, "Secure network and sessions": TLS pins and certificate review, the public-plaintext
question, the insecure-auth guard and channel binding, MySQL VerifyCa through a vendored
`mysql_async`, SSH tunnels (host keys, agent, encrypted keys, keyboard-interactive, jump chains,
shared sessions, `~/.ssh/config` import), the Linux vault, the session core (actor, transaction state,
cancel, liveness, reconnect, pools), read-only connections guarded in layers with a role probe, and the
UI for all of it. This record also closes the M2 exit criteria (spec section 6, M2), checked at the end
of M2b.

This is the **draft** gate record, written by the gate agent (Task 25). Everything an agent can do is
done and recorded here. What only the PO can do is marked **PO-pending** and is prepared in
`docs/benchmarks/po-demo-m2b.md` (in Turkish). Baselines are **not** copied: the PO signs them off
first (Step 6).

- Gate branch: `m2b-gate`, based on `m2b` at `ce992cc` (Tasks 1 to 24 merged, pushed).
- Draft PR #3 `m2b` → `m2a` (opened by this gate) gives
  `docker-pr`; a manual `workflow_dispatch` of `ci.yml` on `m2b` gives the nightly jobs (section 3).
- Machine: the PO's Mac (macOS, Apple silicon), with the 19 containers of `hacivat-m2a` (17 targets
  and 2 helpers) up for the whole run. No hostnames, accounts or personal names are recorded here.

**Verdict: not yet a PASS.** Every local gate is green, and so are `docker-pr`, `docker-full` and
the keyring and vault CI jobs. But CI `build-test` fails on all three operating systems, and
`turkey-test` fails too, because of two test-code defects that the local macOS run cannot show
(section 3.2). Both are diagnosed, with fixes proposed; they are outside the gate's scope (CI config
and docs), so they are not fixed here. E11, the PO halves of E6, E7 and E9, and the connected-RSS
reading of E13 also wait for the PO.

## 1. Exit criteria E1 to E14

| # | Criterion | Status | Evidence |
|---|---|---|---|
| E1 | Connect works against the Docker matrix, direct and through SSH (single hop and jump chain), for PostgreSQL 14–18, MySQL 8.0/8.4, MariaDB 10.6/10.11/11.4 and SQLite | **PASS** (chain on the `pr` targets) | `cargo xtask docker test --set full` 99/99 locally and on CI `docker-full` (section 2, 3). `connect_matrix_direct_and_via_ssh`: every TCP target (pg14–18, pg17-tls, mysql80/84, mariadb106/1011/114) directly and through `sshd-a`; pg17 and mysql84 also through the `sshd-a` → `sshd-b` chain (encrypted-key hop, password hop). The chain runs on the `pr` targets only, a narrowing the controller accepted (ADR 0004, `m2b-t23`, "Paths"). M2a's suite re-ran in the same run: `connects_and_detects_every_matrix_target` (pg13 = `server.unsupported_version`), `sqlite_file_connects`, `test_report_is_complete_for_every_target` |
| E2 | Reconnect works: outage, terminated backend and SSH death recover with replay; the lost state is reported; a user statement is never replayed | **PASS** | Docker: `server_outage_reconnects_within_backoff` (20 s outage, reconnect within the B19 schedule), `terminated_backend_with_open_tx_reports_lost_tx` (`SessionLost { open_tx, temp_objects }` and `Lost`, then Retry connects; by the T17 C1 ruling a lost open transaction is never reconnected on its own), `insert_killed_mid_flight_reports_outcome_unknown_without_duplicate` (`outcome_unknown: true`, at most one row), `a_real_server_ending_the_session_is_reconnected_with_session_lost` (pg17, mysql84: idle session reconnects, the toast claims no rollback). In-process: `one_ssh_death_reconnects_each_dependent_once` (B20). See finding G5b (section 5): a direct idle connection without keep-alive notices the loss only at its next request |
| E3 | Cancel works on every engine within 1 s, direct and through SSH; stale cancels never hit the next statement | **PASS** | Docker `cancel_matrix`: pg17, mysql84 and mariadb114, directly and through `sshd-a`, each under 1 s, the next statement runs; SQLite's endless CTE interrupted under 1 s. `stale_cancel_never_hits_the_next_request` (hacivat-session) |
| E4 | SQL-SAFE-06 server-blocked rows are blocked with the classifier disabled (L3–L6 stand alone) on PostgreSQL 14–18, MySQL 8.0/8.4 and SQLite; MariaDB results recorded | **PASS** (with the recorded `EXECUTE` residual) | `docker_m2b_bypass.rs` 4/4 (section 6). PostgreSQL 13–18 and MySQL 8.0/8.4: every server-blocked row stopped by 1a, L3, L4 or the protocol alone, also as the superuser or owner; the side channel shows no trace. MariaDB recorded, label stays guarded (C19). SQLite on every PR. Residual: an owner's `CALL` of a definer procedure escapes the runtime layers on MySQL (`hv_t23_escape_ro`) and is a committed L6 gap on MySQL and MariaDB; the label stays guarded, least privilege is the remedy (ADR 0004 `m2b-t23`, "T18 carries"). These are not server-blocked rows |
| E5 | Read-only guards for all three engines, with the role probe and honest labels (MariaDB guarded, C19) | **PASS (automated); PO look pending** | Docker `docker_readonly` 10/10 (hv_ro enforced on pg13–18; fixture roles with superuser membership, column UPDATE, CREATE on `public` read guarded). App: `read_only_labels_and_remediation_popover`, `copy_sql_copies_the_remediation_exactly`, `postgres_13_shows_the_schema_grant_form`, `enforced_with_temp_notes_temp_and_mariadb_has_no_sql`. Label shots in section 7. Demo step 8 (PO). On pg17 `hv_ro` has no EXECUTE on either `pg_logical_emit_message` overload (checked), so it reads enforced |
| E6 | TLS `Auto` flows: pin, CA import, the unencrypted badge, the public-plaintext question, pin-changed hard stop; "Encrypt without verifying" removed | **PASS (automated); demo steps 1–4 PO-pending** | hacivat-net TLS tests (T3: pins, rotation = `tls.pin_changed` with both fingerprints, expiry not overridden, encrypted PKCS #8, RFC 5929); T14 pipeline (`pipeline_tls`, 28); T19 app (`connections_tls`, incl. `encrypt_without_verifying_is_gone`, `pin_changed_dialog_has_cancel_as_default`, `plaintext_question_dialog`); Docker `tls_auto_pin_and_verify_ca` (pg17-tls and mysql84, both paths), `unencrypted_badge_cases_on_loopback`. The public-plaintext question cannot be reached on the loopback matrix (Q39): shots and `use_an_ssh_tunnel_answer_end_to_end` |
| E7 | SSH: host-key policies, known hosts, agent, encrypted key, keyboard-interactive (in-process), `~/.ssh/config` import, shared sessions | **PASS (automated); demo steps 5–7 PO-pending** | T4 (`ssh_config` 39, `known_hosts` 37), T9 (`ssh_tunnel`: agent through `TestAgent`, encrypted key, keyboard-interactive and password against the in-process `TestSshServer`, kill switch per hop), T20 (`connections_ssh_form`), T21 (`connections_prompts`: host-key new/changed, keyboard-interactive, passphrase per hop), Docker `two_profiles_share_one_ssh_session` and the chain leg of `connect_matrix_direct_and_via_ssh`. Windows agent pipe untested (open item) |
| E8 | Insecure-auth guard (PG `R` 3/5, MySQL key retrieval), channel binding, MySQL VerifyCa | **PASS** | Docker `insecure_auth_refused_through_ssh_on_md5` (pg17-auth `hv_md5`/`hv_clear` through SSH under Auto → `auth.insecure_method`; Disable and loopback connect; SCRAM connects); T10 wire observer tests (codes 3 and 5 refused before the library answers), Docker `docker_pg` channel binding on pg17-tls; T8/T13 patch §2 `password_guard` and Docker `docker_exec`/`docker_mysql`; mysql84 VerifyCa → `CaOnly` (Docker) and the `mysql84-x509` client-certificate sign-in |
| E9 | Secrets: the vault on Linux CI, keyring CI on Linux and Windows, live keyring on the PO's Mac; no secret at rest outside the keyring or vault | **PASS (CI and Mac); post-demo grep PO-pending** | CI `vault-linux`, `keyring-linux`, `keyring-windows` green, each running its real test (section 3). `live_keyring_round_trip` x2 on this Mac: `ok`, `ok`, no dialog (section 4). Automated: `connections_toml_never_contains_a_secret`; the cross-surface flows assert that no password reaches `connections.toml`, Hacivat's `known_hosts` or a frame. The grep after the demo is in `po-demo-m2b.md` |
| E10 | The Docker matrix in CI: `docker-pr` green on the PR; `docker-full` green nightly; every image pinned by digest | **PASS** (`docker-full` by manual dispatch) | `docker-pr` green on PR #3 (263 tests: 164 FakeDriver + 99 Docker); `docker-full` green on dispatch run 43476270529 (99/99, 56 binaries). `cargo xtask docker check-pins` ok locally and in both jobs. Scheduled workflows run only on the default branch, so "nightly" applies once M2 reaches `main`; until then a dispatch stands in (G8). Digests in section 6 |
| E11 | Shots are clean, and the PO signs off the M2b surfaces and the Turkish strings | **PO-pending (harness half PASS)** | Section 7: 1012 shots rendered at `ce992cc` with 0 violations (checks a–f), 5/5 harness self-checks; CI's macOS `pseudo-screens` render also 1012/0. Against a render at the `m2a` tip (`a645c5f`, 308 shots): 200 identical (0.0000 %), 108 changed (all intended M2b changes, section 7), 704 new. Sign-off and the Turkish review are the PO's |
| E12 | Gates are green: clippy `-D warnings`, the full test set, `check-deps` (with `GuardNotRuntimeFree`), `l10n check`, `tokens --check`, `cargo deny`, and `cargo tree -i aws-lc-rs` empty | **PASS locally; FAIL on CI** | Section 2: every line green on macOS. CI `build-test` fails on Windows (clippy: an import used only by a `#[cfg(unix)]` test) and on Linux and macOS (`mysql_async_is_the_vendored_copy` needs every platform's packages offline); `turkey-test` fails on the second. Findings G1 and G2 (section 3.2) |
| E13 | Budgets: first frame ≤ 500 ms and idle RSS ≤ 200 MB with no connection; idle RSS with one direct and one SSH connection recorded | **PASS (no connection); connected reading PO-pending** | `--smoke` x3: 245, 194, 208 ms and 108, 107, 108 MiB (section 4). `--smoke` cannot connect and the app does not reconnect at boot, so the one-direct-plus-one-SSH reading is the demo's post-check 2 |
| E14 | Driver stream injection verified (§5.7): PostgreSQL over SSH channel streams, MySQL through the forwarder over SSH | **PASS** | `connect_matrix_direct_and_via_ssh` asserts `forwarded() == (engine != PostgreSQL)` on every SSH leg of every target; `mysql_over_ssh_uses_forwarder` (hacivat-session, in-process SSH server) |

Summary: E1, E2, E3, E4, E8, E10 and E14 PASS. E5, E6, E7 and E9 pass their automated halves and wait
for the demo. E11 and the connected half of E13 wait for the PO. **E12 fails on CI** until findings G1
and G2 are fixed (two small test changes, section 3.2).

## 2. Step 2 gates (local, on `m2b-gate` at `ce992cc`)

All commands ran with `CARGO_TARGET_DIR=<main>/target` and `CARGO_INCREMENTAL=0`, after
`cargo clean -p hacivat -p hacivat-app -p hacivat-session -p hacivat-net` (and the same with
`--release`). Docker was not started or stopped; the Docker tests ran through the lock wrapper.
Each line ran on its own, so one failure could not hide another.

| Gate | Result |
|---|---|
| `cargo clippy --workspace --all-targets -- -D warnings` | clean (every workspace crate checked) |
| (1) `cargo test --workspace` | 153 binaries, 2410 passed, 0 failed, 102 ignored (Docker-, live- and CI-gated) |
| (2) `cargo test -p hacivat-ui-core --features test-support,scene-check` | 5 binaries, 81 passed, 0 failed |
| (3) `cargo test -p hacivat-l10n --features pseudo` | 8 binaries, 151 passed, 0 failed |
| (4) `cargo test -p hacivat-app --features scene-check,pseudo` | 19 binaries, 464 passed, 0 failed |
| (5) `cargo test -p hacivat-driver --features fake` | 9 binaries, 164 passed, 0 failed |
| (6) `cargo test -p hacivat-testkit --features ssh` | 6 binaries, 25 passed, 0 failed, 1 ignored |
| (7) `cargo test --release -p hacivat-l10n --features pseudo` | 8 binaries, 154 passed, 0 failed |
| (8) `cargo xtask docker check-pins` | ok |
| `cargo xtask check-deps` (includes the `GuardNotRuntimeFree` rule) | ok |
| `cargo xtask l10n check` | 0 errors, 0 warnings |
| `cargo xtask tokens --check` | ok |
| `cargo deny --all-features check licenses bans sources` | bans ok, licenses ok, sources ok |
| `test -z "$(cargo tree -i aws-lc-rs --workspace 2>/dev/null)"` | empty (no `aws-lc-rs` in the graph) |
| (9) `cargo xtask docker test --set full` | 56 binaries, 99 passed, 0 failed, 0 ignored, 111 s; every target healthy |
| (9) again, after the demo pre-runs rotated pg17-tls's certificate and sshd-a's host key | 99 passed, 0 failed |
| (10) `HACIVAT_KEYRING_LIVE=1 cargo test -p hacivat-secrets -- --ignored live_keyring_round_trip` | x2, section 4 |

Test counts are the sums of the `test result` lines of each command.

**RUSTSEC-2023-0071** (`rsa`, reached through russh's RSA support, the Marvin timing issue, no fixed
version) is a **documented acceptance, not a CI pass** (Q45). The `deny` job checks licences, bans and
sources only, as in M2a, so no job checks advisories. ADR 0003 records why it is accepted: the
advisory concerns RSA decryption and signing timing an attacker can measure, Hacivat only signs SSH
authentication challenges locally with the user's own key, and the copy prefers Ed25519 keys.

## 3. CI

- Draft PR #3, `m2b` → `m2a`, opened by this gate: `pull_request` run 43476256011 (GitHub Actions,
  workflow `ci`).
- `workflow_dispatch` of `ci.yml` on `m2b`: run 43476270529 (the scheduled and dispatch-only jobs).
- Both runs are on `ce992cc`. No secret is used by any job; CI never signs.

### 3.1 Results

| Job | PR run 43476256011 | Dispatch run 43476270529 |
|---|---|---|
| `docker-pr` | **success** (263 passed) | skipped (PR/push only) |
| `docker-full` | skipped (schedule/dispatch only) | **success** (99 passed, 56 binaries) |
| `keyring-linux` | skipped | **success** (`live_keyring_round_trip ... ok`, gnome-keyring over D-Bus) |
| `keyring-windows` | skipped | **success** (`live_keyring_round_trip ... ok`, Credential Manager) |
| `vault-linux` | skipped | **success** (`linux_without_secret_service_uses_vault ... ok`) |
| `release-tests` | skipped | success |
| `check-deps`, `l10n-check`, `tokens`, `cargo-deny` | success | success |
| `launch-smoke` (macOS, Ubuntu, Windows) | success | success |
| `switch (ubuntu-24.04)` | success | success |
| `switch (macos-15)` | cancelled: "not acquired by Runner of type hosted" (GitHub's macOS arm64 capacity); the re-run (attempt 2) was cancelled the same way | cancelled, same cause. The job runs the `switch`/`language` subset of line (4), `cargo test -p hacivat-app --features scene-check,pseudo`, which passed locally on macOS (464) |
| `build-test (windows-2022)` | **failure** (G1) | **failure** (G1) |
| `build-test (ubuntu-24.04)` | **failure** (G2) | **failure** (G2) |
| `build-test (macos-15)` | **failure** (G2) | **failure** (G2) |
| `turkey-test` | **failure** (G2) | **failure** (G2) |
| `nightly-package` | skipped | Windows success; Ubuntu **failure** (G3, also on `main`); macOS cancelled (no hosted macOS arm64 runner acquired) |
| `pseudo-screens` | skipped | macOS: render **1012 shots, 0 violations**; the step after it, `shot-diff` against `design/baselines/m1/macos`, breaches the 24 M1 shots M2a changed on purpose (re-baseline PO-pending since M2a). Ubuntu: **failure**, 360 violations (G4, same class as `main`) |
| `bench` | skipped | success (regression signal only) |

### 3.2 CI findings (diagnosed, not fixed: outside the gate's scope)

- **G1. Windows clippy: an unused import.** `crates/core/hacivat-session/tests/pipeline_ssh.rs:29`
  imports `IssueCode`, and its only use is at line 502, inside `a_key_file_other_users_can_read_is_refused`,
  which is `#[cfg(unix)]` (line 484). On Windows clippy `-D warnings` fails: `unused import: IssueCode`.
  From Task 14 (`fc5293d`); never seen before, because CI had not run on `m2b`. Fix: import it inside
  the unix test (or `#[cfg(unix)] use hacivat_session::pipeline::IssueCode;`). Clippy stops at the
  first failing crate, so a re-run may show further Windows-only warnings in later crates.
- **G2. `mysql_async_is_the_vendored_copy` fails on every CI runner.** The helper `cargo_metadata()`
  in `crates/core/hacivat-session/tests/m2b_interfaces.rs` runs `cargo metadata --format-version 1
  --offline` and asserts success (it drops stderr). `cargo metadata` resolves every platform's
  packages, and a runner downloads only its own (a Linux runner never fetches `pageant` or
  `windows-*`), so `--offline` fails. Reproduced on this Mac with a temporary `CARGO_HOME` made of
  links to the real registry minus one Windows-only crate: `error: failed to download pageant v0.2.5
  … --offline was specified`; the same command with `--filter-platform aarch64-apple-darwin`
  succeeds. It passes locally only because this Mac's registry holds every platform's crates. From
  Task 1. Fix options: pass `--filter-platform <host triple>` (from `rustc -vV`), or drop `--offline`,
  or add `cargo fetch --locked` before the tests in `build-test` and `turkey-test` (but those are M1
  blocks of the append-only `ci.yml`). The test-side fix is the smaller one.
- **G3. `nightly-package (ubuntu)`: "Could not find a square icon to use as AppImage icon".**
  `packaging/package-linux.sh` through cargo-packager. Not an M2b regression: the scheduled `main`
  run 43407970313 fails the same way. Owner: follow-up (packaging).
- **G4. `pseudo-screens (ubuntu)`: 360 violations on Linux, the same kind as `main`'s 260.** On
  `main` (run 43407970313) the M1 surfaces fail on Linux: `menu-app` 220, `dialog-about` 16, the
  shells and toast 24, mostly check (a) "Fluent label not bracketed" in popup menus and the about
  dialog, plus 32 (b) clips of the explorer filter placeholder. On `m2b` the same 260 recur, and the
  M2a and M2b surfaces of the same kind add 100: `explorer-menu` 60, `conn-delete-confirm` 24 (M2a),
  `ssh-import-menu` 16 (M2b). The macOS render of the same job is clean (1012, 0 violations), as is
  the local one. So this is the inherited Linux-harness problem with popup menus and alert dialogs,
  growing with each new popup surface, not an M2b regression. Owner: follow-up (Linux harness).

## 4. Smoke (E13) and the live keyring test (E9)

`cargo run -p hacivat --release -- --smoke`, three runs on the PO's Mac, no connection, nothing else
running:

| Run | first-frame-ms | idle-rss-mib |
|---|---|---|
| 1 | 245 | 108 |
| 2 | 194 | 107 |
| 3 | 208 | 108 |

Median: first frame 208 ms, idle RSS 108 MiB (M2a: 211 ms, 105 MiB). Budgets: first frame at most
500 ms, idle RSS at most 200 MB. **PASS** for the no-connection case (worst run 245 ms, 108 MiB).

Idle RSS with one direct and one SSH connection: `--smoke` never connects, the app restores no
connection at boot, and no test measures the app process with live connections. It is the demo's
post-check 2 (PO).

Live keyring test, `HACIVAT_KEYRING_LIVE=1 cargo test -p hacivat-secrets -- --ignored
live_keyring_round_trip`, through the dev-signing runner:

| Run | Result | Wall time |
|---|---|---|
| 1 | `live_keyring_round_trip ... ok` | 5 s (test 0.10 s) |
| 2 | `live_keyring_round_trip ... ok` | 3 s (test 0.08 s) |

Neither run showed a Keychain dialog or blocked (a 90 s cap was set).

## 5. PO demo pre-runs and demo findings

`po-demo-m2b.md` has 13 steps. Everything that can run headless was run (results are in each step's
"Ön doğrulama" line). The GUI halves are the PO's.

| Step | Pre-run by the gate agent | Result |
|---|---|---|
| 1 pg17-tls pin | the fingerprint command against the certificate the server presents (`openssl s_client -starttls postgres` from the compose network) | equal |
| 2 pin changed | in-place certificate rotation (G5a) | served fingerprint `DD:1C:C4:…:FF:7D` → `63:A4:A2:…:EE:75`, container healthy, full Docker set 99/99 afterwards |
| 3 mysql84 VerifyCa | CA artifact present (`CN=MySQL_Server_8.4.13_Auto_Generated_CA_Certificate`); Docker `tls_auto_pin_and_verify_ca` | green |
| 4 public plaintext | unreachable on loopback (Q39) | shots and app tests |
| 5 SSH import, shared session | `ss` session count (0 before), published host key = live key (`ssh-keyscan`) | as expected |
| 6 jump chain | Docker chain leg of `connect_matrix_direct_and_via_ssh` | green |
| 7 host key changed | `docker exec m2a-sshd-a /regen-hostkey.sh` | `SHA256:95lOZ…` → `SHA256:NuSJA…`, artifact = live key, healthy, Docker 99/99 afterwards |
| 8 read-only labels | `hv_ro` signs in with the matrix password, is no superuser, has no EXECUTE on `pg_logical_emit_message` | enforced expected |
| 9 reconnect | Docker `a_real_server_ending_the_session_is_reconnected_with_session_lost` | green (see G5b) |
| 10–13 | app tests and Docker `cancel_matrix` | green |

Demo findings (the plan's demo text, corrected in `po-demo-m2b.md`):

- **G5a. Restarting `pg17-tls` does not change its certificate.** `docker/m2a/pg-tls/init.sh` makes
  the certificate once, at `initdb`, and a restart keeps the data volume. The demo rotates it in place
  (a new certificate in `PGDATA`, then `pg_reload_conf()`), which the gate pre-ran.
- **G5b. A direct idle connection does not notice a terminated backend.** With `ManagerConfig::m2b()`
  (B18) a profile without `keepalive_s` gets no keep-alive on a direct transport (60 s only over
  SSH), and M2b has no query console. So after `pg_terminate_backend` the capsule keeps saying
  Connected until the next request. M2a pinged every connection every 30 s. The demo sets "Keep-alive
  every 30 s" or uses the debug statement. **PO question (B18):** should direct connections keep a
  default keep-alive so the capsule tells the truth?
- **G5c. Host-key checks need a fresh SSH session.** While any profile holds the shared `sshd-a`
  session, a new connection reuses it and the changed host key is not asked about; the tunnel also
  stays 30 s after its last user (B11). The demo disconnects everything on `sshd-a` and waits 35 s.
  As built and as ruled; noted so that it does not read as a failure.

## 6. Docker matrix: digests, versions, bypass suite

### 6.1 Image digests (`cargo xtask docker check-pins`: ok)

| Image (compose `image:` or Dockerfile `FROM`) | Digest |
|---|---|
| `postgres:13` (pg13) | `sha256:4689940c683801b4ab839ab3b0a0a3555a5fe425371422310944e89eca7d8068` |
| `postgres:14` (pg14) | `sha256:980b7cb415aa0f812f65bdec8f6f599be408d8ed9c83ffcaa12650eac397b264` |
| `postgres:15` (pg15) | `sha256:5a407822929c3789ff5154d3d35bd7e2090318149344c8a3d4ca942b035822ed` |
| `postgres:16` (pg16) | `sha256:63e83b5a3dc36a917dccb76fe1bd2ac51f1ec9c1deb47ff20d4f04233bf6eae9` |
| `postgres:17` (pg17, pg17-tls, pg17-auth, and `FROM` of pg17-tr) | `sha256:846a7ddaa67a4b354ae1d21087b683901aa6113a4ed626d3787cfbf6c85e3e07` |
| `postgres:18` (pg18) | `sha256:b9f6b8ff03b62f3cfd9d9ba3fe56226a9991c79d3fb46fd46536263bf02be553` |
| `mysql:8.0` (mysql80) | `sha256:7dcddc01f13bab2f15cde676d44d01f61fc9f99fe7785e86196dfc07d358ae2b` |
| `mysql:8.4` (mysql84, mysql84-x509, mysql-x509-certs) | `sha256:a81d043930e51745522bd5c430961cf8286f0a23f0a9a2a6e0108f7ea71b0902` |
| `mariadb:10.6` | `sha256:c7d726a1c0d29e486a1ab4ef0c76c75431a11428e76c50e99ba32106cea04114` |
| `mariadb:10.11` | `sha256:7d829180b5b2a9ba74f6242876ab401a0d7dd338a807e522ef92bc9698760f20` |
| `mariadb:11.4` | `sha256:5e73baf8179b29f115101bb102763a47a017361f3e9792daccba6646c3ab1532` |
| `alpine:3.20` (`FROM` of `hacivat-m2a-sshd:2`: sshd-a, sshd-b, ssh-client-keys) | `sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc` |

The two locally built images (`hacivat-m2a-pg17-tr:17`, `hacivat-m2a-sshd:2`) are pinned through their
`FROM` lines.

### 6.2 Resolved versions (`<main>/target/docker/resolved-versions.txt`, set `full`, aarch64)

| Target | Version |
|---|---|
| pg13 (unsupported-version fixture) | PostgreSQL 13.23 |
| pg14 / pg15 / pg16 | 14.25 / 15.20 / 16.16 |
| pg17, pg17-tls, pg17-auth | 17.12 |
| pg17-tr | 17.11 (G7) |
| pg18 | 18.7 |
| mysql80 / mysql84 / mysql84-x509 | 8.0.46 / 8.4.13 / 8.4.13 |
| mariadb106 / mariadb1011 / mariadb114 | 10.6.28 / 10.11.19 / 11.4.13 |
| SQLite | bundled (rusqlite) |

- **G7. The local `pg17-tr` image is older than its pinned `FROM`.** It reports 17.11, while the
  pinned `postgres:17` digest is 17.12 (pg17, pg17-tls and pg17-auth). It was built before Task 16
  pinned the digests, and `docker up` reuses an existing local image. CI builds it fresh. Owner:
  controller (`docker compose -p hacivat-m2a build pg17-tr` at the next planned `up`).

### 6.3 Bypass suite (E4)

The full tables, per engine and version, are in ADR 0004, section `m2b-t23 — bypass-suite results`
(`docs/decisions/0004-sessions-and-read-only-guard.md`), generated from the `t23-bypass-<target>.tsv`
artifacts of the final `--set full` run. The local and CI `docker-full` runs of this gate passed the
same four tests. In short:

| Engine | Judged | Result |
|---|---|---|
| PostgreSQL 13–18 | yes | every server-blocked row stopped by 1a, 1b, L3 or the protocol alone (also as superuser); large objects on 13–15 stopped by L6 (ROLLBACK + canary), from 16 by L3; `txid_current()` a fail-closed gap; F1 fixed (layer 1b refuses 27 non-transactional functions, `pg_logical_emit_message` among them, before any I/O) |
| MySQL 8.0 / 8.4 | yes | every server-blocked row stopped by L4 or the protocol alone; F2 fixed (prepared `SET GLOBAL` no longer ends the connection); owner `CALL` residual: `hv_t23_escape` committed L6 gap, `hv_t23_escape_ro` **escaped** (label guarded, least privilege is the remedy) |
| MariaDB 10.6 / 10.11 / 11.4 | recorded | like MySQL on the read-only setting; `LOCK TABLES` a gap; both owner `CALL`s committed L6 gaps; label stays guarded (C19) |
| SQLite | yes | read-only session over a fresh file refuses writes; runs on every PR |
| Through `sshd-a` | pg17, mysql84 | every server-blocked row gave the direct result |

## 7. Shots (E11; Step 6 waits for the PO)

**Render at the gate head.** `SHOTS_DIR=<main>/target/shots-review-m2b/_render-m2b cargo test -p
hacivat-app --features scene-check,pseudo,shots --test shots -- --ignored` on `ce992cc`: **1012
rendered, 0 violations** (78 surfaces x dark/light x en/tr/en-XA/tr-XT, some at 960x600 or 1024x700
too; checks (a) to (f): bracketed Fluent labels in the pseudo locales, nothing clipped, 11 px floor,
one filled badge, title bar intact, text inside its dialog card). Without `--ignored` the harness
self-checks pass 5/5 (`a_missing_surface_fails_its_frame`, `a_pushed_title_bar_fails_its_frame`,
`the_matrix_covers_every_surface_locale_and_theme`,
`pseudo_labels_must_be_bracketed_and_unclipped_and_at_least_11_px`,
`the_command_line_reads_like_libtest`). CI's macOS `pseudo-screens` job rendered the same 1012 with
0 violations on a clean runner.

**Against the `m2a` tip.** The `m2a` tip (`a645c5f`) was rendered from an exported tree with the
same harness: 308 shots, 0 violations. `cargo xtask shot-diff --baseline <m2a render> --candidate
<m2b render>`: **308 compared, 108 changed, 704 new**; the same 108 at zero tolerance
(`--max-share 0`), and the other 200 at exactly 0.0000 %. The changed shots, all intended:

| Surface (variants) | Changed | Cause |
|---|---|---|
| `conn-dialog-new` (1280 and 1024), `conn-dialog-engine-switch`, `conn-dialog-pasted` | 6.4 to 18.5 % | the folded "SSH tunnel · Off" section (T20) makes the card taller, so it re-centres; TLS section changes (T19) |
| `conn-error-tls` | 3.9 to 5.6 % | B6: "Review certificate…", "Import CA…" and "Edit connection" replace "Encrypt without verifying"; body "…Review it before you trust it." |
| `capsule-production-ro` | 0.6 to 1.1 % | honest access label: "read-only (guarded)" in the capsule and the status bar; the sheet's Access row moves under Environment with its lock (T22) |
| `capsule-connected-unencrypted`, `-connecting`, `-failed`, `-unsupported`, `unencrypted-popover`, `explorer-list`, `explorer-menu`, `shell-minimum` | 0.16 to 0.81 % | most likely the capsule, sheet and explorer-row changes of T22 and T24 (path segment, access row, fit at 960 px, route icon); these small changes were not inspected pair by pair |

The three largest groups were looked at side by side (a pair each) and match the cause given. The
PO's review of `1-m2a-changed/` is the check for the rest.

**Review folders** (in `<main>/target/shots-review-m2b/`, not committed):

- `1-m2a-changed/`: the 108 changed shots as `<name>__ONCE.png` (m2a tip) and `<name>__SIMDI.png`
  (m2b), 216 files;
- `2-m2b-new/`: the 704 new M2b shots;
- `_render-m2b/`: all 1012 shots of the gate head, the source for `design/baselines/m2b/macos/`
  after the PO signs off.

`design/baselines/m2b/` was **not** written (Step 6 waits for the PO).

**How the `m2a` render was made, and a caution.** The `m2a` tree was exported with `git archive`
into a scratch folder and built with the project's single target directory, as the rules require.
Cargo hashes path crates relative to the workspace root, so two trees share artifact names in one
target directory. On the first try, `hacivat-l10n`'s build-script output (`ids.rs`) from `m2b` was
reused, because the exported files kept their older commit times, and the `m2a` build failed. The
gate touched every exported file, rebuilt, rendered, and then ran `cargo clean -p` for every
workspace crate (8.6 GiB), so no `m2a` artifact can be mistaken for an `m2b` one. The harness
self-checks above were rebuilt from `m2b` after that clean. Anyone repeating this should do the same
clean afterwards.

## 8. Rulings B1 to B31 for PO confirmation (outcome as built)

The PO is asked to confirm each, and especially **B1, B5, B8, B10, B13, B14 and B17**.

| B | Ruling | Outcome |
|---|---|---|
| **B1** | Core of every SESS spec in M2b; UI only where mocked (reconnecting capsule, session-lost toast). Transaction badge, Commit/Rollback, mode and isolation selectors, cancel-escalation dialog, idle-transaction and close-with-open-tx prompts, "waiting for a connection", the single-session toggle, Smart commit, startup scripts, L1/L2: **M3** | Built as ruled. The core emits the events (`SessionEvent`, `ConnectionEvent`). Visible cancel exists only as debug-build palette commands (Q39). **PO confirm** |
| B2 | `RawConn` bounded text execution (`execute`, `batch`, `tx_status`, `reset`, `backend_id`) | Built; internal value reads use `ExecOpts::values` (finding F3) |
| B3 | Transaction state from the protocol | Built: PG `Z` byte through `wire::Observer`; MySQL status flags, re-read with `COM_PING` after an `ERR` (T13 I1, keeps the diagnostics area), autocommit from `SERVER_STATUS_AUTOCOMMIT` (T17 R5); SQLite `is_autocommit` |
| B4 | The M2a main connection is the default console session; eager reconnect | Built |
| **B5** | Pins are the SHA-256 of the leaf certificate DER, not SPKI (`alg/01`); a pin forbids the plaintext fallback (Q13); a changed certificate is a hard stop | Built; ADR 0003 records the SPKI deviation. Disable with a pin shows a notice (`disable_ignores_pin`). **PO confirm** |
| B6 | Interactive TLS: Pin, Import CA, Cancel; "Encrypt without verifying" removed | Built; the review opens Cancel-focused (PO question, morning list); expired or not-yet-valid certificates are never offered a pin |
| B7 | Public host without TLS asks; remember adds `allow_public_plaintext` | Built; unreachable on the loopback matrix (Q39); app tests and shots |
| **B8** | Insecure-auth guard, outcome-based for both engines | Built. Costs: an md5-only private server without TLS under Auto now needs Disable; M2a profiles set to Require by "Encrypt without verifying" hit `auth.insecure_method` (open item). **PO confirm** |
| B9 | `tls-server-end-point` channel binding | Built (SHA-224 also handled, a T3 extra); Docker channel-binding test on pg17-tls |
| **B10** | `mysql_async` 0.37.1 vendored with five patches, not forked; VerifyCa enabled for MySQL/MariaDB | Built (`third_party/mysql_async/PATCHES.md`); upstream PRs are a PO action. **PO confirm** |
| B11 | SSH with russh (ring); own known-hosts store, never writes `~/.ssh`; remote name resolution; channel hygiene; sharing; 30 s grace; SSH retention | Built. Keyboard-interactive answers are never stored or replayed (T9 I3); russh's own `String` copies of passwords and answers are not zeroed (accepted gap, ADR 0003); host-key algorithms limited to the known key types (T4 carry) |
| B12 | Own `~/.ssh/config` parser | Built; the strictest `StrictHostKeyChecking` across hops applies (T4 I1 ruling); a password in `ProxyJump` is dropped with a notice |
| **B13** | Vault: Argon2id m = 64 MiB, t = 3, p = 1; XChaCha20-Poly1305; bounded header; atomic writes; Linux without Secret Service only | Built; plus an exclusive file lock and random temp names (T6 I1/I2 ruling), bounded retries, never `VaultCorrupt` on a race. **PO confirm** |
| **B14** | Secure event input stays in M9 | Not in M2b, as ruled. **PO confirm** |
| B15 | Keyring CI on Linux and Windows, vault on Linux | Built; all three green (section 3) |
| B16 | PgBouncer M8, WAL readers M4, C11 M8; MariaDB stays guarded | As ruled; MariaDB label decision is the PO's (open item) |
| **B17** | `docker/m2a` stays the matrix; sshd-a/b, pg17-auth, read-only roles; no toxiproxy (`ChaosProxy`), no PgBouncer; digests pinned | Built; pg17-auth is on 54347 (54327 is pg17-tls, T16 carry); `mysql84-x509` added for the client-certificate sign-in (F2 fix round). **PO confirm** |
| B18 | Liveness: 1 s tick, keep-alive per profile (60 s over SSH, otherwise off), idle disconnect by environment, clock-jump detection | Built. See G5b: a direct connection without keep-alive is no longer pinged (M2a pinged every 30 s). **PO question** |
| B19 | Reconnect with backoff, replay, never replay user statements, the session-lost toast | Built, with the T17 rulings: an open (or unknown) transaction at loss ends `Lost` with `SessionLost` and never reconnects on its own; requests arriving during recovery are answered `ConnectionLost { outcome_unknown: false }`; a user `SET autocommit = 0` means no automatic reconnect (over-cautious, safe) |
| B20 | One SSH death, one reconnect per dependent | Built (`one_ssh_death_reconnects_each_dependent_once`) |
| B21 | Read-only layers L0, L3–L6; transaction keywords denied; labels | Built; plus layer 1b (F1), the read-only PG options allowlist (T18 I-3c), a reset before the identity probe (T18 I-3b) |
| B22 | Transaction defaults; read-only sessions always Auto | Built |
| B23 | Cancel over the session's transport; stale-cancel guard; MySQL pre-warm at 500 ms | Built. Deviation: the 30 s side-connection cache was dropped (T15 ruling 2): a fresh side connection per cancel; revisit in M3 |
| B24 | Three new error classes (48 → 51) | Built |
| B25 | Encrypted PKCS #8 client keys; legacy encrypted keys refused | Built |
| B26 | Canonical path checks for untrusted origins | Built |
| B27 | Empty pgpass password is `None` | Built |
| B28 | M2b mocks controller-approved | **PO review pending** (`design/m2b-secure/index.html`, NOTES.md "Open questions") |
| B29 | `CapsuleModel::Reconnecting` | Built (T22) |
| B30 | Profile additions, format stays `version = 1` | Built; M2a-written files round-trip byte for byte |
| B31 | Out of M2b, with owners | As ruled (open items below) |

## 9. Open items, with owners

Owners: **PO**, **controller** (merge, push, Docker stack), **M3**, **M4**, **M8**, **M9**,
**follow-up** (a small task, any implementer).

### 9.1 Blocking the gate

| Item | Owner | How |
|---|---|---|
| G1: Windows clippy, unused `IssueCode` import in `pipeline_ssh.rs` | follow-up (before merge) | section 3.2; then re-run `build-test (windows-2022)` |
| G2: `mysql_async_is_the_vendored_copy` needs `--filter-platform` (or no `--offline`) | follow-up (before merge) | section 3.2; then re-run `build-test` x3 and `turkey-test` |
| Run the 13-step demo and fill its table, including the E9 grep and the E13 connected RSS | PO | `docs/benchmarks/po-demo-m2b.md` |
| Sign off the shots and the Turkish strings; then copy the macOS PNGs to `design/baselines/m2b/macos/` | PO, then any implementer | section 7 |
| `m2b` lacks `m2a`'s last two commits: `a645c5f` (P47: global commands and menu items inert under a modal layer) and `977111d` (M2a demo doc). They merge cleanly (`git merge-tree`: no conflict). Merge `m2a` into `m2b` before the demo, so the new M2b dialogs get P47's menu greying | controller | `git merge origin/m2a` on `m2b` |

### 9.2 PO decisions and questions

| Item | Owner |
|---|---|
| Upstream PRs for the five `mysql_async` patches (`third_party/mysql_async/PATCHES.md` has the drafts) | PO |
| M2a profiles set to Require by "Encrypt without verifying" now hit `auth.insecure_method` (PostgreSQL md5/password: always; MySQL: on a `caching_sha2` cache miss); the fix is Review certificate → Pin (Q20) | PO |
| Widen MySQL's Auto plaintext fallback from loopback to Private now that patch §2 removes M2a's reason (Q41). Until then a MySQL-family server without TLS through SSH needs Disable | PO |
| MariaDB "enforced" label: it stays guarded (C19); flip or keep | PO |
| B18 / G5b: default keep-alive for direct connections | PO |
| Confirm the certificate review and the new-host-key prompt open Cancel-focused (morning list) | PO |
| `tls.not_supported_by_server`: should "Connect through SSH…" be the primary, focused fix (T24 R7)? | PO |
| Confirm the pinned label "TLS 1.3 · pinned" / "TLS 1.3 · sabitlendi" (T24 I3) | PO |
| SSH import notice always lists "host, user, key and jump host"; vary it with what was imported? (T20 M3) | PO |
| Lists inside UI sentences follow the UI language, the regional format governs numbers and dates only (I18N-05 reading, T22) | PO confirm |
| MySQL: an owner's `CALL` of a definer procedure escapes the runtime read-only layers (label stays guarded). Refuse `CALL` on read-only sessions in M3? | PO |
| PostgreSQL: `pg_logical_emit_message` is executable by `PUBLIC` by default, so default clusters read guarded until the remediation's optional `REVOKE EXECUTE` (F1) | PO (information) |
| Insecure-method review copy (T19 M4) | PO |
| Controller-approved M2b mocks (B28): TR "zorunlu" for enforced; one "Reset vault…" button on the unlock dialog | PO |
| Pre-flight rulings Q1–Q46 (especially Q4: awaiting `tokio::sync` channels on the foreground is allowed; determinism through `allow_parking` in test helpers) and B1, B8 (morning list) | PO confirm |

### 9.3 PO copy review (Turkish strings)

Every M2b Turkish string is a draft for the PO: `locales/tr/hacivat-ui.ftl` has 216 messages in its
`## m2b-*` blocks (215 carry `# Review: PO`), and `locales/tr/hacivat-core.ftl` 18 (all marked). The
shots in section 7 show them in place. The M9 string freeze is the final review.

### 9.4 Deferred by plan

| Item | Owner |
|---|---|
| The Docker directory rename (`docker/m2a`, project `hacivat-m2a`) | M9 |
| OS sleep hooks and App Nap (B18 detects clock jumps only) | M9 |
| Secure event input for masked fields (B14) | M9 |
| PgBouncer, and C11's separate read-only credential | M8 |
| SQLite WAL readers (budget stays 1) | M4 |
| Session UIs, Smart commit, startup scripts, L1/L2 (B1) | M3 |
| CONN-02 | M4 |
| Session resume across networks, SOCKS/HTTP proxies, `my.cnf`, PuTTY keys and Pageant, per-profile `UserKnownHostsFile`, `IdentityAgent` | v1.x |

### 9.5 From the ledger and the task reports

| Item | Owner | Status |
|---|---|---|
| RUSTSEC-2023-0071 (`rsa` via russh): documented acceptance, not a CI pass (Q45; ADR 0003) | PO (aware) | accepted |
| G3: `nightly-package (ubuntu)` AppImage icon; also fails on `main` | follow-up (packaging) | open |
| G4: `pseudo-screens (ubuntu)` violations; also fails on `main` | follow-up (M1 shots on Linux) | open |
| G7: rebuild the local `pg17-tr` image from its pinned `FROM` | controller | open |
| G8: `docker-full` runs nightly only once M2 is on `main` (scheduled workflows use the default branch); dispatch until then | controller | open |
| Windows SSH agent (named pipe) path untested (T9 e) | follow-up | open |
| russh `String` copies of SSH passwords and keyboard-interactive answers are not zeroed (T9 b) | accepted (ADR 0003) | accepted |
| A stuck bastion: timed-out opens wait until the chain closes (T9 N1) | follow-up | parked |
| `pg_advisory_unlock_all()` in the PG reset batch is not `pg_catalog`-qualified (T18 carry to the T10 owner; `crates/core/hacivat-driver-pg/src/exec.rs:39`) | follow-up | open |
| `RESET PERSIST` L5 untested by the fixture (it is `3615` before any privilege check) | follow-up | open (ADR 0004) |
| MySQL `SELECT … INTO @var` and `CALL` close a read-only session (fail-closed cost, T5 m-6) | M3 | accepted |
| A read-only session on a WAL SQLite file leaves `-wal`/`-shm` (T11 I3) | M4 | accepted (ADR 0002) |
| Side-connection cache for cancels dropped (T15 ruling 2) | M3 (perf) | open |
| Session queue cap (T7 M12) | M3 | parked |
| `Broken(Cancelled)` becomes Disconnected; profile-wide close; idle-disconnect inference is fragile (T17) | M3 | accepted for M2b |
| Manual-mode replay resets a user's `SET autocommit = 1` (T17 P3) | M3 | parked |
| Bind stored SSH secrets to the hop identity; repeated unlock dialog after a declined hop move; retention-move sweep (T21 S1, S2, n1) | M3 | parked |
| Gpui fork lock-order inversion between `App::shutdown` and the core relay (shots harness fixed by stopping the runtime per shot; real quit tests could meet it) | M3 / follow-up | open |
| T22 minors: dropped capsule items do not come back; in pseudo locales the connecting name shrinks before the step and clock; icon-only clipping below 960 px unchecked; one-frame transition clipping | follow-up | open |
| T24 polish carries: eye toggle on masked fields; "Public key" row in the secret prompt; per-hop marks in `ssh.*` errors; SSH server text in Technical details; the pre-probe "Keychain" label on a vault machine (R4); the vault label ellipsizes in pseudo locales | follow-up | open |
| M2a gate items still PO-pending (M2a demo, Keychain delete check, M2a shot sign-off, 24 M1 re-baselines, M2a one-connection RSS, Windows/Linux hosts) | PO | see `docs/benchmarks/m2a.md` |

## 10. Windows and Linux (Step 5)

- CI: the keyring and vault jobs passed on Linux and Windows (section 3), `docker-pr` and
  `docker-full` passed on Linux, `launch-smoke` on all three. `build-test` fails on all three (G1, G2).
- No Windows or Linux hosts were provided. Demo steps 1, 5 and 6 on Windows and Linux are **open**
  (owner: PO, supplies hosts).

## 11. Files in this gate commit

- `docs/benchmarks/m2b.md` (this file).
- `docs/benchmarks/po-demo-m2b.md` (the Turkish PO checklist).
- No baselines copied: `design/baselines/m2b/` waits for the PO's sign-off. No code changed. The only
  push is the draft PR (`m2b` was already on `origin`); `m2b-gate` is not pushed.
