# pgpass parity vectors

`vectors.toml` holds the ten `.pgpass` vectors of HV-SEC-04: a file and a lookup tuple each,
with the outcome both sides must give. They cover the escaped colon, the `*` wildcard, a case
mismatch, an empty password, CRLF line ends, `localhost` versus the socket directory, a
database named `replication`, the default port, a kept trailing space and the first match
winning.

`crates/core/hacivat-session/tests/docker_pgpass.rs` checks every vector two ways and asserts
that the outcomes agree with each other and with `expect`:

- **Ours:** `hacivat_secrets::pgpass_lookup` over the same bytes, written to a `0600` file.
- **libpq:** `psql -w` inside the `m2a-pg17` container (`docker exec`), with `PGPASSFILE`
  pointing at a temporary `0600` copy of the file. psql connects with
  `host=<host> hostaddr=<container ip>`: the container's own address is not loopback, so the
  server asks for a scram-sha-256 password (loopback and the socket are `trust` in the official
  image), and libpq looks the file up under `host`. The temporary file is removed after each
  run.

The psql outcome is read from its exit status and libpq's message: success, or the server's
"database ... does not exist" after a successful sign-in, is `accepted`; "password
authentication failed" is `rejected`; "no password supplied" is `no_password`.

`@PW@` in a line stands for the servers' throwaway test password (from `hacivat-testkit`), so
the password itself is not written here.

Run with the compose project up:

```
cargo xtask docker test --set pr -p hacivat-session
```

(`pg17` is in both the `pr` and `full` sets.)
