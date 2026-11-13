# M2a Docker server matrix

The engine tests (R17) run against these servers. The compose project is `hacivat-m2a`, and
every container is named `m2a-<id>`. Every port binds to `127.0.0.1` only.

| id | image | host port | `tls_default` | sets |
|---|---|---|---|---|
| `pg13` | `postgres:13` (below the floor: the unsupported-version fixture) | 54313 | none | full |
| `pg14` | `postgres:14` | 54314 | none | full |
| `pg15` | `postgres:15` | 54315 | none | full |
| `pg16` | `postgres:16` | 54316 | none | full |
| `pg17` | `postgres:17` | 54317 | none | pr, full |
| `pg18` | `postgres:18` | 54318 | none | full |
| `pg17-tls` | `postgres:17`, `ssl=on`, self-signed (`pg-tls/init.sh`) | 54327 | self_signed | full |
| `pg17-auth` | `postgres:17`, md5 and cleartext logins (`pg-auth/`) | 54347 | none | full |
| `pg17-tr` | `postgres:17` + tr_TR locale (`pg-tr/Dockerfile`), `lc_messages=tr_TR.UTF-8` | 54337 | none | full |
| `mysql80` | `mysql:8.0` | 33080 | self_signed | full |
| `mysql84` | `mysql:8.4` | 33084 | self_signed | pr, full |
| `mariadb106` | `mariadb:10.6` | 33106 | none | full |
| `mariadb1011` | `mariadb:10.11` | 33111 | none | full |
| `mariadb114` | `mariadb:11.4` | 33114 | self_signed | full |
| `sshd-a` | Alpine OpenSSH bastion (`sshd/`), networks `default` + `ssh-inner` | 2222 | n/a | pr, full |
| `sshd-b` | the same image, **no published port**: reached only through `sshd-a` | none | n/a | pr, full |

`ssh-client-keys` is a helper service (no target): it writes the client key pairs once and
stays up so the bastions can wait for it.

Each `[[target]]` also carries `service` (the compose name) and `internal_port`:
`DockerTarget.internal` is the in-network endpoint (`pg17:5432`) that tests going through
`sshd-a` or `sshd-b` dial. The bastions are `[[ssh_target]]`s (`id`, `host`, `port`, `user`,
`via`, `sets`); `hacivat_testkit::ssh_targets()` lists them.

`targets.toml` holds the same list for `hacivat-testkit`. `tls_default` is what each server
offers out of the box, observed on the first `up` and committed.

## Fixtures

The test credentials are throwaway values. Never copy them into a non-test file.

- On every server: user `hv_test` (its password is in `compose.yml`), database `hv_test`,
  plus the databases `orders` and `orders_archive` (`init/pg.sql`, `init/mysql.sql`).
- Fixture tables `hv_probe(a int)` and `hv_probe_log(id, v int)` on every database server.
- `hv_ro` on every database server: read-only (PostgreSQL: `pg_read_all_data`, inherited, with
  CREATE on `public` and TEMP revoked from PUBLIC; MySQL/MariaDB: `SELECT, SHOW VIEW`).
- `pg17-auth` only: `hv_md5` (md5) and `hv_clear` (cleartext `password` method), both with the
  throwaway password.
- SSH: user `hv_ssh` on both bastions, with the throwaway password or the generated keys.
  The `ssh-client-keys` service writes `ssh-client-ed25519` and `ssh-client-ed25519-enc`
  (passphrase `hv_test_pw`) plus their `.pub` files, and each bastion writes
  `ssh-host-<id>.pub`, all into `<main checkout>/target/docker` (testkit:
  `ssh_client_key(encrypted)`, `ssh_host_pub(id)`). Nothing private is committed; the client
  keys are 0644 so the Linux CI runner user can read them.
- On MySQL and MariaDB only:
  - `hv_locked`, an account created with `ACCOUNT LOCK`;
  - `hv_ro`, which has read-only grants on the three databases.
- `mysql84`'s auto-generated CA certificate is copied to
  `<main checkout>/target/docker/mysql84-ca.pem`. Tests reach it only through
  `hacivat_testkit::artifact("mysql84-ca.pem")`.

## Commands

Run these from the repository root (or a worktree).

```
cargo xtask docker up --set pr|full        # controller and CI only
cargo xtask docker status [--set pr|full]
cargo xtask docker test --set pr|full [-p <crate>]...
cargo xtask docker down                    # controller and CI only
cargo xtask docker pin                     # resolve every image tag to @sha256 (network)
cargo xtask docker check-pins              # fails on any image or FROM without a digest
```

Every `image:` and `FROM` is pinned to its multi-platform index digest. After changing a tag,
run `pin` once and commit the result.

- `up` waits up to 180 s for every selected service to be healthy, then copies the CA
  certificate.
- `test` never starts or restarts anything. If a selected service is not healthy, it names
  the service and fails. Otherwise it runs `cargo test … -- --ignored` with these variables:
  - `HACIVAT_DOCKER=1`;
  - `HACIVAT_DOCKER_SET=<set>`;
  - `HACIVAT_DOCKER_ARTIFACTS=<main checkout>/target/docker`.

  By default the crates are `hacivat-driver-pg`, `hacivat-driver-mysql`,
  `hacivat-driver-sqlite`, `hacivat-session` and `hacivat-testkit`.
- `down` removes the containers and their anonymous volumes. The next `up` starts again from
  the init scripts.
- The bind mounts point into the checkout that ran `up`. During M2a the controller runs the
  project from the main checkout, never from a worktree.
