# M2a gate record (draft)

Milestone M2a, "Connect": the connection layer, the profile store, the secrets store, the three
drivers (PostgreSQL, MySQL/MariaDB, SQLite), and every connection surface of the app (dialog, explorer,
capsule, test strip, error dialog, prompts).

This is the **draft** gate record, written by the gate agent while the PO was away. Everything an
agent can do is done and recorded. What only the PO can do is marked **PO-pending** and is prepared
in `docs/benchmarks/po-demo-m2a.md` (in Turkish). Baselines are **not** copied: the PO signs them off
(Step 6).

- Gate branch: `m2a-t22`, based on `m2a` at `c3518b0` (Tasks 1 to 21 merged).
- The controller pushes `m2a` and records CI. The CI status here is **pending controller push**.
- Machine: the PO's Mac (macOS, Apple silicon, 13 Docker services up for the whole run). No
  hostnames, accounts or personal names are recorded in this file.

## 1. Exit criteria E1 to E10

| # | Criterion | Status | Evidence |
|---|---|---|---|
| E1 | Connect and test through the pipeline against PostgreSQL 14 to 18, MySQL 8.0/8.4, MariaDB 10.6/10.11/11.4 and a SQLite file; PG 13 gives `server.unsupported_version` | **PASS** | `cargo xtask docker test --set full` green (section 2): `connects_and_detects_every_matrix_target`, `test_report_is_complete_for_every_target`. Resolved versions in section 4 |
| E2 | Stream injection verified per crate and recorded | **PASS** | ADR 0002 (sections m2a-t13, t14, t15; PG injected stream, MySQL through the TCP loopback forwarder when a transport wrapper is set, own TCP on Direct, P13); Docker `stream_injection_is_used_for_the_main_connection` |
| E3 | URL round trip: render after parse is the identity on every catalog template; Copy never contains the password; pasted URLs connect | **PASS** | `every_catalog_template_compiles_and_round_trips` (hacivat-driver, `url_template`), `display_and_copy_never_contain_the_password` (hacivat-driver), Docker `url_round_trip_connects`; app: `copy_url_never_includes_password` |
| E4 | Keyring round trip on the PO's Mac, dev-signed, no prompt on the second run; Delete removes the Keychain item | **PO-pending (partly run)** | `live_keyring_round_trip` x2 by the gate agent: run 1 and run 2 both `ok`, no Keychain dialog, no blocking, unattended (section 3). The "Delete removes the Keychain item" half is demo step 9 (PO) |
| E5 | Error-classification goldens: code tables, Fluent EN/TR goldens, real-server classes including a Turkish server | **PASS** | `classify` tests and `error_fluent_golden` (hacivat-driver); Docker `error_classes_against_real_servers`, `turkish_server_wrong_password_is_auth_invalid`, `pg_turkish_server_text_is_kept_verbatim_and_marked` |
| E6 | pgpass parity with `psql` on 10 vectors | **PASS** | Docker `pgpass_parity_with_psql` |
| E7 | Shots are clean across en/tr/en-XA/tr-XT x dark/light; the PO signs off the design | **PO-pending (harness half PASS)** | Task 21 harness: 308 rendered, 0 violations (re-run at the gate head: 308 rendered, 0 violations; `shot-diff` against `design/baselines/m1/macos`: 68 compared, 24 breached, 240 new). Sign-off, including the 24 M1 re-baselines (section 7), is the PO's |
| E8 | Gates green: clippy `-D warnings`, the full test set, `check-deps`, `l10n check` (L026), `tokens --check`, `cargo deny`, and CI including `docker-pr` | **PASS locally; CI pending controller push** | Section 2. CI run: pending controller push (record the run link here) |
| E9 | No secret at rest outside the keyring | **PASS (automated); PO demo grep pending** | `connections_toml_never_contains_a_secret` (hacivat-app, `connections_model`). Grep of the app's config/data/cache directories for the Docker test password after the gate agent's app runs: `grep -r` of `~/Library/Application Support/dev.hacivat.Hacivat` and `~/Library/Caches/dev.hacivat.Hacivat` returned nothing (exit 1). **Weak evidence:** the gate agent ran only `--smoke` (no profile, no password was ever entered in the app), so this grep proves little. The meaningful grep is the one after the PO demo. The post-demo grep is in `po-demo-m2a.md` |
| E10 | Budgets: first frame at most 500 ms, idle RSS at most 200 MB with no connection; idle RSS with one connection recorded | **PASS (no connection); one-connection RSS PO-pending** | `--smoke` x3, section 3. The one-connection idle RSS needs a live connection in the window (demo); the smoke flag only measures the no-connection state |

Summary: E1, E2, E3, E5, E6 PASS. E8 and E9 pass locally. E4, E7, E10 (one-connection RSS) and the
E9 post-demo grep need the PO. CI is pending the controller's push.

## 2. Step 2 gates (local, on `m2a-t22`)

All commands ran with `CARGO_TARGET_DIR=<main>/target`, after `cargo clean -p hacivat -p hacivat-app`
(and the same with `--release`). Docker was not started or stopped.

| Gate | Result |
|---|---|
| `cargo clippy --workspace --all-targets -- -D warnings` (no features) | clean |
| clippy `-D warnings`, feature configs: `-p hacivat-app --all-targets --features scene-check,pseudo,shots`; `-p hacivat-ui-core --features test-support,scene-check`; `-p hacivat-driver --features fake`; `-p hacivat-l10n --features pseudo` | clean (all four) |
| `cargo test --workspace` | 101 test binaries, 1374 passed, 0 failed, 34 ignored (Docker- and live-gated); run with `--no-fail-fast`, after the fix below |
| `cargo test -p hacivat-ui-core --features test-support,scene-check` | 5 binaries, 81 passed, 0 failed |
| `cargo test -p hacivat-l10n --features pseudo` | 8 binaries, 151 passed, 0 failed |
| `cargo test -p hacivat-app --features scene-check,pseudo` | 13 binaries, 254 passed, 0 failed (includes the two tests gated on `pseudo`) |
| `cargo test -p hacivat-driver --features fake` | 9 binaries, 153 passed, 0 failed |
| `cargo test --release -p hacivat-l10n --features pseudo` | 8 binaries, 154 passed, 0 failed |
| `cargo test -p xtask l10n -- --ignored repo_catalogs_are_clean` | 1 passed |
| `cargo xtask check-deps` | ok |
| `cargo xtask l10n check` (with L026) | ok, 0 errors, 0 warnings |
| `cargo xtask tokens --check` | ok |
| `cargo deny --all-features check licenses bans sources` | ok |
| `cargo xtask docker test --set full` | 22 binaries, 32 passed, 0 failed, 0 ignored; all 13 targets healthy for the whole run |
| `cargo fmt --check` | not a gate (CI has no fmt job). `cargo fmt --check` reports diffs only in `spikes/spike-harman`, `spikes/spike-probes` and `xtask/src/vf.rs`, none in code this milestone touched (Task 21 recorded fmt clean for app, ui, ui-core and driver) |

### Defects the gate found and fixed

1. **`cargo test --workspace` failed (2 tests), which `cargo test -p hacivat-app --features scene-check,pseudo`
   hides.** `the_environment_select_sets_the_tag_like_the_segments` and
   `the_environment_select_follows_the_automatic_local_tag` (Task 21) open the dialog in `tr-XT`, which
   exists only with the `pseudo` feature. CI's `build-test` job runs `cargo test --workspace --locked`
   without it, so it would have failed. Fixed by gating both tests with `#[cfg(feature = "pseudo")]`
   (they still run in the `pseudo` line, 254 app tests pass there).
2. **`M2A_PENDING_COMMANDS` (Task 1) emptied**, as its comment demanded of the gate: every bound
   command is registered now, and `keymap_commands_are_registered` asserts it without an exception.
3. **ADR 0002 gained the loopback-forwarder slot-race ruling** (controller ruling, ledger), section
   `m2a-t22`.

Test counts are the sum of `test result` lines of each command. Doc tests with 0 tests are included
in the binary counts but add nothing.

## 3. Smoke (E10) and the live keyring test (E4)

`cargo run -p hacivat --release -- --smoke`, three runs, on the PO's Mac with no connection:

| Run | first-frame-ms | idle-rss-mib |
|---|---|---|
| 1 | 262 | 104 |
| 2 | 206 | 105 |
| 3 | 211 | 105 |

Median: first frame 211 ms, idle RSS 105 MiB.

Budgets: first frame at most 500 ms, idle RSS at most 200 MB. **PASS** for the no-connection case (worst run 262 ms and 105 MiB). Idle RSS with one connection is not measured by `--smoke` and is PO-pending (demo).

Live keyring test (`HACIVAT_KEYRING_LIVE=1 cargo test -p hacivat-secrets -- --ignored live_keyring_round_trip`,
run through the dev-signing runner, identity "Hacivat Dev"):

| Run | Result | Wall time |
|---|---|---|
| 1 | `live_keyring_round_trip ... ok` | 6 s (test 0.44 s) |
| 2 | `live_keyring_round_trip ... ok` | 5 s (test 0.46 s) |

Neither run showed a Keychain dialog, and neither blocked (limit: 60 s). The automated half of E4 passes twice,
unattended. E4 stays **PO-pending** for the demo half (Delete removes the item, demo step 9) and for the PO's
confirmation that the first run after a rebuild does not prompt.

## 4. Resolved Docker image versions

From `<main>/target/docker/resolved-versions.txt` (Task 19, set `full`, server aarch64):

| Target | Resolved version |
|---|---|
| pg13 (unsupported-version fixture) | PostgreSQL 13.23 |
| pg14 | PostgreSQL 14.24 |
| pg15 | PostgreSQL 15.19 |
| pg16 | PostgreSQL 16.15 |
| pg17 | PostgreSQL 17.11 |
| pg18 | PostgreSQL 18.6 |
| pg17-tls (self-signed leaf certificate) | PostgreSQL 17.11 |
| pg17-tr (tr_TR locale, `lc_messages`) | PostgreSQL 17.11 |
| mysql80 | MySQL 8.0.46 |
| mysql84 | MySQL 8.4.13 |
| mariadb106 | MariaDB 10.6.28 |
| mariadb1011 | MariaDB 10.11.19 |
| mariadb114 | MariaDB 11.4.13 |
| SQLite | bundled 3.54.1 (rusqlite 0.42) |

Images use floating major tags in M2a (ledger ruling, Task 11). M2b revisits with multi-arch
index digests.

## 5. Windows and Linux (Step 5)

- CI `build-test`, `docker-pr`, `launch-smoke`: **pending controller push**. Record the run link and
  the three job results here when the controller has pushed `m2a`.
- No Windows or Linux hosts were provided. Demo steps 1 to 3 on Windows and Linux, and the Windows
  Credential Manager round trip, are **open** (owner: PO, supplies hosts).
- The Linux keyring build is verified by CI only (`rustup target add` plus `cargo check`, P35). A
  Secret Service round trip on Linux is manual and **open** (owner: PO).

## 6. Rulings

### 6.1 Plan rulings R1 to R31 (outcome as built)

| R | Ruling | Outcome |
|---|---|---|
| R1 | Crate set and placement (proto, net, secrets, driver, driver-{pg,mysql,sqlite}, session, testkit) | Built as ruled; `check-deps` ok. No `hacivat-core` facade yet |
| R2 | ID numbering follows `alg/01` (SSH = SEC-01, TLS = SEC-02) | Followed |
| R3 | TLS in M2a is the non-interactive half of D-005; "Encrypt without verifying" interim fix | Built (`FixId::EncryptWithoutVerifying`, `hv-conn-fix-require`, capsule "not verified"). **PO approved** (ledger); copy to review |
| R4 | No UI shells for the TLS certificate dialog or SSH | Built as ruled; Validate rejects `TransportSpec::SshTunnel` with `TransportUnavailable` |
| R5 | Linux master-password vault moves to M2b | `VaultBackend` trait only; Linux without Secret Service falls back to UntilRestart with one toast. **PO approved** |
| R6 | Linux keyring via `async-secret-service`, `crypto-rust`, `tokio` | Built; CI checks the build, no live Linux round trip (open) |
| R7 | Keyring entry names `hacivat[-dev]/<uuid>/<slot>`, account `hacivat`; dev signing script and runner | Built. A test fails on keyring's mock backend (ledger ruling). Runner never fails (P35) |
| R8 | Retention default Forever; Test never stores; a re-entered password is stored only after success | Built. **PO approved**. One 3-rejection budget per gesture (P8) |
| R9 | One `connections.toml` file, toml_edit, quarantine on parse failure, read-only on newer major | Built |
| R10 | Read-only is a label plus a minimal session setting | Built (PG option, MySQL `SET SESSION`, SQLite read-only open); `ReadOnlyEnforced` never produced |
| R11 | Unsupported servers: `server.unsupported_version`, Connect anyway, "unsupported" mark | Built; "Supported versions" URL is a placeholder (open item) |
| R12 | HV-CONN scope: CONN-01/03/07 full, CONN-04 ping plus 30 s liveness, no reconnect | Built. **PO approved**. Reconnect and the keep-alive supervisor move to M2b; CONN-02 to M4 |
| R13 | Budget seam: 4 remote, 1 SQLite | Built (`Budget`), stated as a fact in Advanced |
| R14 | Interaction: `UiPort::ask(Question)`, password only | Built; Headless fails with `needs_interaction` |
| R15 | `(Decimal)` Fluent type, `PreparedQuery` in Task 10, L026 over `errors.toml` | Built (Task 10) |
| R16 | Duration copy: whole ms totals, 2-decimal running clocks, one message per timed string | Built (P28) |
| R17 | Docker harness `hacivat-m2a`, major tags, `docker-pr` with pg17 and mysql84 | Built (Task 11); full matrix passes locally; CI `docker-pr` pending |
| R18 | Driver ADR; `mysql_async` cannot take an injected stream, so the TCP loopback forwarder; OS roots as DER with a scoped `webpki-roots` licence exception | Built (Tasks 5, 13, 14); ADR 0002 sections t13, t14, t15, t22. For PO review (fallback would be `native-tls`) |
| R19 | The capsule follows `ConnectionsModel::current()` | Built |
| R20 | Keys and menus: `Explorer` context, Esc as a hook, only `connection.new` in the menu bar, modal layers block workspace bindings | Built; `M2A_PENDING_COMMANDS` emptied at this gate |
| R21 | Paste interception through `set_paste_filter`; the Password field never intercepts | Built |
| R22 | Naming and environment rules: whole-token production hint with `HintSource`; local auto-tag; RFC 1918 hosts ask | Built (P7) |
| R23 | Duplicate = `<name>-2`; secrets copied | Built |
| R24 | pg_service import dialog, environment required for remote rows | Built; password from a world-readable file is dropped with a warning |
| R25 | Failure position in the trail derived from the error class family | Built |
| R26 | "Copy report" and "Copy details (English)" rendered from the `en` bundle | Built |
| R27 | Title bar stays visible during a modal; scrim from y = 40 px | Built (changes 8 M1 shots, section 7) |
| R28 | Shots data through public `*_fixture` constructors behind `shots` | Built (Task 21) |
| R29 | Driver traits use `BoxFuture`; `RawConn` is probes only | Built |
| R30 | Explicitly not in M2a (session actor, guard, reconnect, SSH, vault, `my.cnf`, CONN-02, import from other tools) | Not built, as ruled |
| R31 | Explorer group "No environment" goes last | Built (`hv-explorer-group-unset`) |

### 6.2 PO-away rulings and controller rulings (ledger)

| Ruling | Outcome |
|---|---|
| Production hint matches `prod`, `prd`, `live`, `production` as whole tokens split on `[-._/:0-9]` | Built (P7). "delivery-db" no longer matches |
| `webpki-roots` (CDLA-Permissive-2.0) allowed as a scoped `deny.toml` exception | Written in Task 1 (P1); `cargo deny` ok |
| MySQL trust = `with_disable_built_in_roots` plus OS roots as DER | Built (Task 14). OS-roots semantics differ slightly from the platform verifier (no revocation, no intermediate fetch) |
| R3 interim "Encrypt without verifying" accepted until M2b | Built; PO approved |
| The keyring backend must be the native store, with a test that fails on the mock | Built (Task 4) |
| Adopt all pre-flight rulings P1 to P49 in one plan-amend pass | Done (plan amended at `033c69c`) |
| R3, R5, R8, R12 narrow L15 M2a scope; L15(b) mocks unsigned | **PO approved all ten PO-away rulings; mocks visually approved** (ledger) |
| Branches stack `m2a` to `m2b` to `m3`; no merges to `main` overnight; PO-only gate steps accumulate in a morning list | This draft and `po-demo-m2a.md` are that list |
| New UI mocks for later phases are controller-approved; PO reviews links | Open (PO) |
| Live and real-Keychain tests stay `#[ignore]` and run only with the PO present | Followed; this gate ran the keyring test only under the 60 s no-dialog rule (section 3) |
| Branch naming `m2a-<slug>`; File menu hidden until Task 18 registered `connection.new`; `result_large_err` allowed in driver/session | Followed |
| Task 4: `scripts/dev-sign.sh` is canonical; keychain cancel maps to Unavailable | Built |
| Task 5: VerifyCa without a CA file is a Validate error; `TlsFailure::from_rustls` public | Built |
| Loopback TCP forwarder slot race by a local process is inherent (same as `ssh -L`) | Documented in ADR 0002 section m2a-t22 (this gate) |
| macOS secure event input deferred to M9 (ADR, unsafe FFI) | Open (M9) |
| Task 6: workspace Tab/Shift-Tab focus traversal added by Task 18 | Built |
| Task 8: pgpass password stops at the first unescaped `:`; socket host maps to localhost only for default socket dirs; `read_pg_service` reports a dropped-password warning | Built |
| Task 10: `tls.untrusted` body matches the M2a fix; `config.invalid` gets an "Edit connection" button; "This Mac" stays for the macOS gate | Built; copy to review |
| Task 11: images stay on floating tags in M2a | Followed (M2b revisits) |
| Task 13: P9 proven at wire level; cleartext/md5 auth over unverified TLS goes into ADR "Known limits" | Documented. M2b implements a `require_auth`-style guard and channel binding |
| Task 14: MySQL/MariaDB Auto plaintext fallback limited to Loopback (private-network hosts require TLS under Auto); explicit Disable stays, with a warning | Built. M2b: upstream or `[patch]` knob against RSA key retrieval when not secure |
| Task 15: `hacivat-net` dependency in the SQLite driver accepted; non-absolute and `file:` paths refused; UNC paths refused | Built |
| Task 7: claimed prefixes are final; systemic secret scan over the whole `ParseOutcome`; allowlist key rule; `sslpassword` carried as `tls_key_passphrase` | Built (fix rounds 1 to 3, plus secret-keys follow-up) |
| Task 9: `Failed` capsule carries the environment; read-only lock red only on production (DESIGN.md) | Built; the lock colour is on the PO review list |
| Task 12: duplicate of a managed profile becomes User plus UserTyped | Built |
| Task 16: approved roots for imported TLS files = user home plus picked paths; `RawErrorKind::PasswordRequired`; `HostInvalid` and `PathRefused` issue codes; MySQL Auto infers plaintext from `tls_summary().encrypted == false` | Built |
| Task 17: SQLite "open read-only" is `mode=ro`, the top Read-only is `profile.read_only` (two controls); key-passphrase field hidden until encrypted keys are supported | Built; PO note |
| Task 18: explorer actions never act on a hidden row; Delete is red | Built |
| Task 20: error dialog initial focus is never "Encrypt without verifying"; "Try again" starts a new gesture | Built |
| Task 21: M1 `empty_state` label stays; wrapping variant only for the M2a sheet | Built; 24 M1 shots still drift for earlier M2a reasons (section 7) |

### 6.3 Pre-flight rulings (P1 to P49) that changed behaviour

| P | Behaviour change |
|---|---|
| P1 | Scoped `webpki-roots` licence exception written in Task 1 |
| P5 | `aws-lc-rs`/`aws-lc-sys` banned; ring provider installed by `CoreRuntime::new` |
| P7 | Whole-token production hint with a source (host, then name) |
| P8 | One budget of 3 server rejections per connect gesture; a stored or draft password that fails is not re-prompted by the pipeline |
| P9 | PostgreSQL `Auto` on a host where plaintext is not allowed uses `SslMode::Require`: a public host without TLS fails before any credential is sent (`tls.not_supported_by_server`) |
| P10 | MySQL forwarder is always TCP `127.0.0.1:0`, `prefer_socket(false)` always, OS roots as DER |
| P11 | MySQL/MariaDB `VerifyCa` unsupported in M2a (disabled option, Validate rule) |
| P13 | A set `transport_wrap` sends MySQL through the forwarder (`LiveConnection::forwarded()`) |
| P15 | `shutdown_blocking(budget)` called synchronously on quit; the app exits within the budget |
| P16 | `InputEvent::Cancelled` treated as Escape on every layer field; masked inputs never call the paste filter |
| P17 | `connections::boot` installs the model and the live hooks before `open_workspace`; M1 shells unchanged |
| P23 | Guard seam for extra connect steps; slow-secret tests split; MySQL Direct opens no pre-stream |
| P26 | PO-away rulings R3, R5, R8, R12 and L15(b) logged for confirmation (PO approved) |
| P30, P31 | Empty Name takes the suggested name; Copy URL toast says the password is not included |
| P43 | The TLS step is omitted from the trail when TLS is disabled |
| P44 | `ConnectRequest.department`; keyring comment = profile name; MariaDB 4151 maps to `auth.locked` |
| P47 | Under a modal layer only that layer's bindings plus `app.quit` resolve |

## 7. Open items, with owners

Owners: **PO** (needs a human), **controller** (push, CI, merge), **M2b**, **M3**, **M9**, **follow-up**
(small task, any implementer).

### 7.1 PO steps that block closing the gate

| Item | Owner | How |
|---|---|---|
| Run the 15-step demo and fill its table | PO | `docs/benchmarks/po-demo-m2a.md` |
| E4: Delete removes the Keychain item, and no prompt on the second keyring run | PO | demo step 9; section 3 |
| E9 post-demo grep returns nothing | PO | end of `po-demo-m2a.md` |
| E10: idle RSS with one connection recorded | PO | read the process RSS in Activity Monitor with one live connection (Docker pg17), or add it to `--smoke` later |
| E7: sign off the shots and the design | PO | shots are rendered by `cargo test -p hacivat-app --features scene-check,pseudo,shots --test shots -- --ignored --nocapture` into `<main>/target/shots` |
| Copy the approved macOS PNGs to `design/baselines/m2a/macos/` (Step 6) | PO, then any implementer | not done by the gate agent |
| Re-baseline the 24 drifting M1 shots (below) | PO | `cargo xtask shot-diff --baseline design/baselines/m1/macos --candidate target/shots` (run from the checkout where the shots were rendered; at gate time the 308 PNGs are in the gate worktree's `target/shots`) |

**The 24 M1 baselines that breach the shot-diff** (`design/baselines/m1/macos`, all 8 variants of
each: dark and light x en, tr, en-XA, tr-XT at 1280x800@2x). All three drifts come from intended M2a
changes, not from Task 21 (proved by rendering with the original Task 9 files):

| Surface (8 shots each) | Drift | Cause |
|---|---|---|
| `dialog-about-*` | about 2.4 to 3.1 % | Task 9: the modal scrim starts below the title bar (R27), so the title bar is no longer dimmed |
| `menu-app-*` | about 1.7 to 4.1 % | Task 18 (P18): the File menu has a new "New connection" row and rows below moved down |
| `toast-info-*` | about 0.2 to 0.4 % | Task 9: the dismiss mark is an icon `Button` (keyboard reachable), a few px further left |

The other 44 M1 shots are at 0.0000 %.

### 7.2 PO copy review (Fluent strings)

Turkish strings carry `# Review: PO` in the catalogs (`locales/tr/hacivat-ui.ftl` 371,
`locales/tr/hacivat-core.ftl` 52, `locales/tr/hui.ftl` 5 comment lines; English
`locales/en/hacivat-core.ftl` 1). The strings the plan and the ledger name for a closer look:

- `hv-conn-fix-require` ("Encrypt without verifying") and `hv-conn-fix-require-note`: the note must say that
  the server is not authenticated (ledger).
- `hv-conn-enc-unverified` ("TLS 1.3 . not verified").
- `hv-capsule-unsupported` and its visual mark (circle-alert, text-2).
- `hv-explorer-group-unset` ("No environment" group).
- `hv-conn-url-multihost` (first host used).
- `hv-conn-keyring-unavailable`.
- `hv-conn-access-ro-guarded` ("read-only (guarded)").
- The password prompt: `hv-conn-pw-title`, `hv-conn-pw-body` ("It's kept only if the connection works."
  is wrong when retention is Never; ledger Task 20 #10).
- `hv-conn-toast-delete-secret` (new in Task 21, TR `# Review: PO`).
- The `tls.untrusted` body (no "Review it before you trust it" while only the interim fix exists) and the
  `config.invalid` "Edit connection" button; "This Mac" wording stays for the macOS gate, platform variants
  at the M9 copy review.
- Warnings in the connection dialog (MySQL/MariaDB plaintext, Disable warning).
- Visual: read-only lock red only on production (DESIGN.md) versus the mock's red for any read-only (kept DESIGN.md).
- The key-passphrase field is hidden until `hacivat-net` supports encrypted keys (a stored value is kept).

### 7.3 Deferred and follow-up lines from the ledger

| Item | Owner | Status |
|---|---|---|
| "Supported versions" URL `https://hacivat.dev/docs/engines` is a placeholder (R11) | PO | open |
| `deny.toml` exception for `webpki-roots` (CDLA-Permissive-2.0), written in Task 1 (P1) | PO confirm | written; fallback is `native-tls` |
| MySQL/MariaDB `VerifyCa` unsupported (P11): upstream fix or `[patch]` fork | M2b | open |
| L15 deviations R3, R5, R8, R12 and the L15(b) mock sign-off (P26) | PO | approved in the ledger; recorded here |
| Windows and Linux keyring verification, demo steps 1 to 3 on both | PO (hosts) | open |
| Windows and Linux CI results (`build-test`, `docker-pr`, `launch-smoke`) | controller | pending push |
| Cleartext/md5 auth over unverified TLS or LAN-stripped Auto: refuse auth codes 3/5 unless verified TLS or Disable; `tls-server-end-point` channel binding (Task 13) | M2b | open (ADR "Known limits") |
| MySQL plaintext path leaks the password to an active MITM (RSA key retrieval, no knob): upstream knob or `[patch]` (Task 14) | M2b | open (ADR) |
| `hacivat-guard`: SQLite `ATTACH`/`VACUUM INTO` closure, canonicalize before approved-roots check, authorizer (Task 15) | M2b | open (ADR "Known limits") |
| Interactive TLS (pinning, CA import, "public host without TLS" dialog), SSH, vault, reconnect, keep-alive supervisor, CONN-05 | M2b | by plan |
| CONN-02 | M4 | by plan |
| Auto on a private host with an untrusted chain fails by design until M2b pinning (D-005) | M2b | by design |
| `empty pgpass password` should be treated as none, not `Some("")` (libpq parity) (Task 19) | follow-up (T8/T16 area) | open |
| Default socket dirs: only `/var/run/postgresql` on Debian; we map three (Task 19) | follow-up | open |
| macOS secure event input (`EnableSecureEventInput`) for masked inputs | M9 | deferred with ADR |
| Task 6 minor: no `set_well` test | follow-up | open |
| Task 3 minors: `pass%77ord` percent-encoded secret-key regression test, doc that the list cap is per list node (assigned to Task 7) | follow-up | verify whether covered by Task 7 |
| Task 4 minor: bare-userinfo scrub over-scrubs Windows paths; wider no-panic alphabet | done in the catalog follow-up (drive-letter rule, wider fuzz) | closed |
| Task 8 minor: `O_NONBLOCK` gated by arch | done in the catalog follow-up | closed |
| Task 7 residual low risk: zero-width, fullwidth, combining, bracket secret keys (not accepted by any DB) | M2b | accepted |
| Task 21 not fixed: SQLite file section labelled SERVER; long URL row shows the tail; popover fixture anchor sits left of "unencrypted"; context menu opens at (0,0) before the first paint; explorer names ellipsize early at 276 px when a row carries latency and a versioned engine | follow-up / PO look | open |
| Capsule segment bounds for the keyboard explain anchor (Task 18 post-merge list) | follow-up | open |
| New UI mocks for later phases (controller-approved): PO reviews links | PO | open |
| Docker images on floating tags; multi-arch index digests | M2b | by ruling |

## 8. Files in this gate commit

- `docs/benchmarks/m2a.md` (this file), `docs/benchmarks/po-demo-m2a.md` (Turkish PO checklist).
- `docs/decisions/0002-driver-crates.md`: section `m2a-t22`.
- `crates/app/hacivat-app/tests/connection_dialog.rs`: two tests gated on `pseudo`.
- `crates/app/hacivat-app/tests/commands.rs`: `M2A_PENDING_COMMANDS` removed.
- No baselines copied (`design/baselines/m2a/` does not exist yet). No push.
