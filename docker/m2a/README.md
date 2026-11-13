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
| `pg17-tr` | `postgres:17` + tr_TR locale (`pg-tr/Dockerfile`), `lc_messages=tr_TR.UTF-8` | 54337 | none | full |
| `mysql80` | `mysql:8.0` | 33080 | self_signed | full |
| `mysql84` | `mysql:8.4` | 33084 | self_signed | pr, full |
| `mariadb106` | `mariadb:10.6` | 33106 | none | full |
| `mariadb1011` | `mariadb:10.11` | 33111 | none | full |
| `mariadb114` | `mariadb:11.4` | 33114 | self_signed | full |

`targets.toml` holds the same list for `hacivat-testkit`. `tls_default` is what each server
offers out of the box, observed on the first `up` and committed.

## Fixtures

The test credentials are throwaway values. Never copy them into a non-test file.

- On every server: user `hv_test` (its password is in `compose.yml`), database `hv_test`,
  plus the databases `orders` and `orders_archive` (`init/pg.sql`, `init/mysql.sql`).
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
```

- `up` waits up to 180 s for every selected service to be healthy, then copies the CA
  certificate.
- `test` never starts or restarts anything. If a selected service is not healthy, it names
  the service and fails. Otherwise it runs `cargo test … -- --ignored` with these variables:
  - `HACIVAT_DOCKER=1`;
  - `HACIVAT_DOCKER_SET=<set>`;
  - `HACIVAT_DOCKER_ARTIFACTS=<main checkout>/target/docker`.

  By default the crates are `hacivat-driver-pg`, `hacivat-driver-mysql`,
  `hacivat-driver-sqlite` and `hacivat-session`.
- `down` removes the containers and their anonymous volumes. The next `up` starts again from
  the init scripts.
- The bind mounts point into the checkout that ran `up`. During M2a the controller runs the
  project from the main checkout, never from a worktree.
