# Hacivat patches to `mysql_async` 0.37.1

This directory is a vendored copy of the crates.io package `mysql_async` 0.37.1, used through
`[patch.crates-io]` in the workspace `Cargo.toml` (M2b, Q2, Q29). The workspace excludes
`third_party/`, so the copy is not a workspace member: its own lints, layers and tests are not
Hacivat's.

## §0 Vendored copy, test, bench and CI files removed (Task 1)

The copy is byte-identical to the registry source (`src/**`, `README.md`, `README.tpl`, the
licences, `.gitignore`, `.cargo_vcs_info.json`), except for:

- `Cargo.toml`: the `[[test]]` targets, every `[dev-dependencies.*]` table and `[profile.bench]`
  are removed (cargo warns about profiles in a non-root package on every build, and the tests
  are not shipped). Nothing else changed.
- Not copied: `tests/`, `Cargo.lock`, `Cargo.toml.orig`, `azure-pipelines.yml` and cargo's
  extraction marker `.cargo-ok`.

`diff -r ~/.cargo/registry/src/*/mysql_async-0.37.1/src third_party/mysql_async/src` is empty
after Task 1.

Every patch is fenced in the source by a `// HACIVAT PATCH §N` comment. Tests:
`crates/core/hacivat-driver-mysql/tests/patches.rs`. Line numbers are in the patched files.

## §1 `skip_domain_validation` matches the name error by variant (Task 8)

- `src/io/tls/rustls_io.rs` (`DangerousVerifier::verify_server_cert`, l.178-182): the arm
  `Err(ref e) if e.to_string().contains("NotValidForName") && skip` becomes
  `Err(rustls::Error::InvalidCertificate(CertificateError::NotValidForName | NotValidForNameContext { .. })) if skip`.
- Why: rustls renders that error as "certificate not valid for name ..." (Display), so the upstream
  substring `NotValidForName` never matched and `with_danger_skip_domain_validation(true)` skipped
  nothing. Kept as an upstream bug fix; Hacivat's VerifyCa runs through its own `ChainOnly`
  verifier via §5.
- Test: `skip_domain_validation_matches_the_name_error_by_variant` (fails on the substring match).
- Upstream PR text: "rustls: match `CertificateError::NotValidForName`/`NotValidForNameContext` by
  variant. The `to_string().contains(\"NotValidForName\")` check never matches the Display text, so
  `skip_domain_validation` had no effect."

## §2 `password_guard`, an outcome predicate at full auth (Task 8)

- `src/opts/mod.rs`: `MysqlOpts.password_guard: Option<PtrEq<dyn Fn() -> bool + Send + Sync>>`
  (default `None`), `Opts::password_guard()`, `OptsBuilder::password_guard(Option<Arc<..>>)`; a
  small `PtrEq<T>` wrapper (Arc compared and hashed by pointer, Debug prints `<opaque>`) keeps
  `Opts: Eq + Hash + Debug`.
- `src/error/mod.rs`: `DriverError::InsecurePasswordRefused`.
- `src/conn/mod.rs` (`continue_caching_sha2_password_auth`, the `0x04` "perform full
  authentication" arm, ~l.807-815): before either branch (the clear password when `is_secure() ||
  is_socket()`, or requesting the server RSA key with `0x02`), `if !guard() { return
  Err(InsecurePasswordRefused) }`. No password byte and no `0x02` request leave the client.
- Reachability notes (patched-file line numbers):
  - `sha256_password` is unreachable via the handshake (every plugin except `caching_sha2_password`
    maps to `mysql_native_password`, `conn/mod.rs` l.550-555). Via an AuthSwitch it is an `Other`
    plugin (see below).
  - `mysql_clear_password` is refused before any write while `enable_cleartext_plugin` is false
    (gate at l.663-668).
  - `mysql_old_password` is refused while `secure_auth` is true (the default).
  - MariaDB ed25519 and parsec send a signature, never the password.
  - `Other` plugins (`sha256_password` by switch, `dialog`, gssapi): `gen_data` returns `None`, the
    client writes an empty packet, then fails with `UnknownAuthPlugin`.
  - The native-password response is a salted SHA-1 scramble, never the clear password.
  - The guard's safety therefore depends on Hacivat keeping `enable_cleartext_plugin(false)` and
    `secure_auth(true)` (Task 13 sets both).
- Write trace (patched-file lines): l.583 SSL request; l.626 handshake response (scramble);
  l.677/679 the generic auth-switch response for every switched plugin (clear-password gate at
  l.663-668); l.782 the parsec response (signature); l.819/822/833 the `caching_sha2` full-auth
  writes (clear password, `0x02` key request, encrypted password), all behind the guard.
- Tests: `password_guard_false_refuses_before_any_password_byte` (server sees zero bytes after
  the refusal), `password_guard_true_or_absent_keeps_upstream_flow`.
- Upstream PR text: "Add `OptsBuilder::password_guard`: a callback consulted before the password
  (or an RSA key request) is sent at `caching_sha2_password` full authentication, so callers can
  refuse to authenticate over a link they do not consider secure."

## §3 `multi_statements` knob (Task 8)

- `src/opts/mod.rs`: `MysqlOpts.multi_statements` (default `true`), `Opts::multi_statements()`,
  `OptsBuilder::multi_statements(bool)`; `get_capabilities` (~l.1222-1235) sets
  `CLIENT_MULTI_STATEMENTS` only when true (removed from the unconditional list).
- The driver never sends `COM_SET_OPTION`, so nothing re-enables it after the handshake.
- Test: `capability_knobs_shape_the_handshake_response`.
- Upstream PR text: "Make `CLIENT_MULTI_STATEMENTS` configurable (default unchanged)."

## §4 `local_infile_capability` knob (Task 8)

- `src/opts/mod.rs`: `MysqlOpts.local_infile_capability` (default `true`),
  `Opts::local_infile_capability()`, `OptsBuilder::local_infile_capability(bool)`;
  `get_capabilities` sets `CLIENT_LOCAL_FILES` only when true.
- This only changes the advertised capability. A hostile server can still send a `0xFB` LOCAL INFILE
  request; the client refuses it (`LocalInfileError::NoHandler`) only because no infile handler is
  installed. Hacivat must never install one (`local_infile_handler`).
- Test: `capability_knobs_shape_the_handshake_response`.
- Upstream PR text: "Make `CLIENT_LOCAL_FILES` configurable (default unchanged), so a client
  can refuse LOCAL INFILE requests at the protocol level."

## §5 `SslOpts::with_rustls_client_config` (Task 8)

- `src/opts/mod.rs`: `SslOpts.rustls_client_config: Option<PtrEq<rustls::ClientConfig>>`
  (`#[cfg(feature = "rustls-tls")]`, default `None`), `with_rustls_client_config(Arc<ClientConfig>)`,
  getter `rustls_client_config()`. `SslOpts` keeps `Eq + Hash` through `PtrEq` (pointer identity).
- `src/io/tls/rustls_io.rs` (`build_tls_connector`, ~l.36-41): when set, return
  `TlsConnector::from(config.clone())` before anything else. Root certificates, the client
  identity and the danger flags are ignored and no `ClientConfig` of the driver's own is built
  or fallen back to; `with_danger_tls_hostname_override` still supplies the server name
  (`conn/mod.rs` ~l.588, untouched).
- Used by Hacivat to run the MySQL handshake through `TlsParams.config` (pins, verifier,
  decrypted client key; Task 13 wires it, one `TlsParams` per handshake attempt).
- Test: `custom_client_config_is_used` (counting verifier called once, SNI `db`).
- Upstream PR text: "Allow supplying a prebuilt `rustls::ClientConfig` via
  `SslOpts::with_rustls_client_config` for callers that need custom verifiers or key providers."
