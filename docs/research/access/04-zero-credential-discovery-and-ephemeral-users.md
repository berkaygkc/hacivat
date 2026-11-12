# 04 — Zero-Credential Discovery and Ephemeral Database Users (Karagöz core)

> Research document for the Hacivat product family. Audience: implementers of **Karagöz** (server agent) and the parts of **Perde** that present and govern discovered databases. Companion documents: `01-teleport-database-access.md` (how Teleport provisions users and proxies protocols), `03-other-access-tools-and-secrets-engines.md` (Vault/OpenBao/Infisical dynamic credentials, PgBouncer/ProxySQL), `05-authorization-and-audit-primitives.md` (tokens, policy, audit chain, approvals, masking — the ALGs that consume what this document produces).

---

## 1. Scope and sources

### 1.1 Questions this document answers

1. How can an agent running on a production host **find** databases (Docker, Kubernetes, systemd/bare metal) without the user typing anything?
2. How can it **connect** to them with no credentials typed by the user — which credential sources exist, in what order should they be tried, and which network path should be used?
3. What does it cost in security terms (Docker socket = root), and what least-privilege alternatives exist?
4. How should the agent **create per-user, short-lived database principals** with an exact privilege mapping per DBMS, and how does it clean them up — including after a crash?
5. What is enforceable **server-side** (by the DBMS itself) versus only by the agent?

### 1.2 Sources studied

All repositories cloned shallowly (depth 1) into `.refs/repos/<name>`; read-only; no builds or scripts executed.

| Source | Path in `.refs/repos` | Commit | Date | License | Use in this doc |
|---|---|---|---|---|---|
| docker-library/postgres (official image) | `docker-library-postgres` | `dfc76bb299b057785026eb2eea39556b6fe52983` | 2026-10-15 | MIT | Entrypoint env handling, `_FILE` secrets, `pg_hba.conf` defaults |
| docker-library/mysql | `docker-library-mysql` | `9b6c81572125a4ff828e3037c71940f3f8311978` | 2026-10-20 | GPL-2.0 (packaging scripts) — prose only | Root/user creation, random root password |
| MariaDB/mariadb-docker | `mariadb-docker` | `9bbc511fe5ad9269ee3485858ae92da972b219ea` | 2026-11-11 | GPL-2.0 — prose only | `MARIADB_*` env, healthcheck user/cnf, `unix_socket` user |
| docker-library/mongo | `docker-library-mongo` | `7c860f9b3a6e21754c9753c341a71328dc1724a6` | 2026-11-10 | Apache-2.0 | `MONGO_INITDB_ROOT_*`, `--auth` behaviour |
| docker-library/redis | `docker-library-redis` | `30e7d6b51c8e9e99cd28b6665a3fa363fdc5a45a` | 2026-10-07 | BSD-3-Clause | Protected-mode patch, no auth by default |
| docker-library/docs | `docker-library-docs` (sparse) | `252f63485fc5a2acb0d882cc790882a2d671596a` | 2026-11-10 | MIT | Image usage docs (postgres, mysql, mariadb, mongo, redis, clickhouse) |
| ClickHouse (docker/server only) | `clickhouse` (sparse) | `0e0a2cc91286ff390bfa268677f5c3ebdb19dce0` | 2026-11-11 | Apache-2.0 | `CLICKHOUSE_*` env, default-user network restriction |
| moby/moby (`api/` only) | `moby` (sparse) | `36d6bbbaa133d770e65b5adc043136acbadc6831` | 2026-11-11 | Apache-2.0 | Engine API v1.57 swagger: list filters, events, exec, archive |
| traefik/traefik (docker provider + label parser) | `traefik` (sparse) | `b45eeb07b7b15e8b4bb85a7b0ed8a32eaa75c8c9` | 2026-11-10 | MIT | Label conventions, enable/constraints, network/port selection, watch loop |
| Tecnativa/docker-socket-proxy | `docker-socket-proxy` | `2e118a0c0c66e1831f68b7afd6d49ad3e2f96b5d` | 2026-07-27 | Apache-2.0 | Socket-proxy ACL model and its limits |
| gravitational/teleport (`lib/srv/db/postgres/sql`) | `teleport` (sparse) | `1283425b60ec5f60d509ba4c791183d452923ff7` | 2026-08-25 | AGPL-3.0 — prose only | Prior art for auto user provisioning/cleanup |
| Microsoft Learn: SQL Server on Linux env vars | https://learn.microsoft.com/en-us/sql/linux/configure/environment-variables (updated 2026-10-14, git `0a26bce5`) | — | — | Docs | `MSSQL_SA_PASSWORD`, `MSSQL_DB/USER/PASSWORD` |
| CloudNativePG docs `applications.md` | https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/main/docs/src/applications.md | main @ 2026-11-11 | — | Apache-2.0 | `<cluster>-app` / `-superuser` secrets and keys |
| Zalando postgres-operator user guide | https://postgres-operator.readthedocs.io/en/stable/user/ | — | — | MIT | Secret naming `{user}.{cluster}.credentials.postgresql.acid.zalan.do` |

General DBMS behaviour (PostgreSQL, MySQL, MariaDB, SQL Server, MongoDB, Redis, ClickHouse privilege systems) is described from the vendors' public documentation and long-standing behaviour; where a claim was not re-verified against a primary source in this session it is tagged **[Inferred]** with its basis.

---

## 2. Executive summary

1. **Discovery is cheap and well-trodden.** Traefik's Docker provider is the canonical model: list all containers, inspect each, decode namespaced labels (`traefik.enable`, `traefik.docker.network`, …), filter by an `exposedByDefault` flag plus a constraint expression, and on every relevant container event (`start`, `die`, `health_status*`) re-list everything, with exponential back-off on stream failure. [Observed: `traefik/pkg/provider/docker/pdocker.go:75-160`, `shared_labels.go:43-63`, `config.go:197-235`]
2. **Recommendation: an opt-in `hacivat.*` label namespace** modelled on Traefik (`hacivat.enable`, `hacivat.engine`, `hacivat.port`, `hacivat.network`, `hacivat.credentials.*`, `hacivat.name`, `hacivat.labels.<k>`, `hacivat.readonly`, …), plus image heuristics as a fallback that only **proposes** databases; Perde must require an explicit "attach" before any heuristic-found database is reachable.
3. **Official images expose bootstrap credentials in the container config.** `POSTGRES_PASSWORD`, `MYSQL_ROOT_PASSWORD`, `MARIADB_ROOT_PASSWORD`, `MONGO_INITDB_ROOT_PASSWORD`, `MSSQL_SA_PASSWORD`, `CLICKHOUSE_PASSWORD` are visible in `GET /containers/{id}/json` → `Config.Env` even though the entrypoints `unset` them inside the process. [Observed: `docker-library-postgres/docker-entrypoint.sh:381`, `docker-library-mongo/docker-entrypoint.sh:415`] (corrected after verification) Only the postgres and mongo entrypoints scrub PID 1's environment; `mysqld` and `clickhouse-server` still carry `MYSQL_ROOT_PASSWORD` / `CLICKHOUSE_PASSWORD` in `/proc/1/environ`, and every `docker exec` process inherits `Config.Env` (e.g. `clickhouse-client` silently authenticates with it). [Observed: docker test, postgres:16/18, mysql:8.4, mongo:7, clickhouse/clickhouse-server:24.8, see verification/01-db-behavior-tests.md#docker-env]
4. **These env values are bootstrap-only.** Every studied entrypoint applies them only when the data directory is empty; if an operator later rotated the password, the env is stale. Credential resolution must therefore **validate** a candidate before trusting it. [Observed: `docker-library-postgres/docker-entrypoint.sh:246-247,345-372`]
5. **`*_FILE` variants are universal** (postgres, mysql, mariadb, mongo implement a shared `file_env` helper; ClickHouse supports only `CLICKHOUSE_PASSWORD_FILE`; SQL Server documents none). The file usually lives under `/run/secrets/…` and can be read with `GET /containers/{id}/archive?path=…` (read-only API) or from the bind-mount source on the host. [Observed: `docker-library-postgres/docker-entrypoint.sh:9-25`, `clickhouse/docker/server/entrypoint.sh:52-57`]
6. **Several images allow password-less local access inside the container**: official PostgreSQL leaves initdb's default local `trust` lines (only remote `host all all all <method>` is added); ClickHouse's `default` user without a password is restricted to `127.0.0.1/::1`; Redis has no auth at all and the image disables protected mode. Karagöz can reach "loopback-only" principals by entering the container's network namespace (`--network container:<id>` helper or `nsenter`), not by `docker exec` text scraping. [Observed: `docker-library-postgres/docker-entrypoint.sh:266-285`; `clickhouse/docker/server/entrypoint.sh:150-168`; `docker-library-redis/7.2/debian/Dockerfile:85-90`] [Observed: docker test, postgres:16/17/18, clickhouse/clickhouse-server:24.8, redis:7 (7.4.12), see verification/01-db-behavior-tests.md#local-auth, #ch-default, #redis-noauth — ClickHouse also accepts the container's own non-loopback IP for the restricted `default` user]
7. **MariaDB writes a ready-made credential file**: `<datadir>/.my-healthcheck.cnf` with a random `healthcheck` user (USAGE only by default) — useful for liveness, never for data access. MySQL/MariaDB "random root password" is printed to container logs ("GENERATED ROOT PASSWORD"), which Karagöz must treat as sensitive and should *not* harvest by default. [Observed: `mariadb-docker/docker-entrypoint.sh:345-364,379-382`; `docker-library-mysql/docker-entrypoint.sh:271-273`]
8. **The Docker socket is root-equivalent**, and *read-only* filtering does not help secrets: `GET /containers/{id}/json` returns all env secrets and `GET /containers/{id}/archive` returns arbitrary files. Tecnativa's proxy only filters by URL prefix and method; it cannot restrict per container or per field. [Observed: `docker-socket-proxy/haproxy.cfg:48-79`]
9. **Recommendation: a built-in "docker-guard"** inside Karagöz (or a tiny sidecar) that allowlists exactly `GET /_ping`, `GET /version`, `GET /containers/json`, `GET /containers/{id}/json`, `GET /events`, `GET /containers/{id}/archive` (only for paths named in that container's `*_FILE` env), and optionally `POST /containers/create` for a pinned netns-helper image — and redacts env values for containers not opted in.
10. **Connection path preference:** container network IP (Linux) → published port bound to loopback → published port on other host IP → netns-entry helper for loopback-only principals → unix socket via bind mount. Docker Desktop (macOS/Windows) cannot route to container IPs; published ports or a helper are required. [Observed: Traefik IP/port selection `config.go:327-425`; Inferred for Desktop routing]
11. **Kubernetes analogue:** annotations `hacivat.io/*` on Services/Pods, credentials from Secrets referenced by `env[].valueFrom.secretKeyRef` or from well-known operator secrets (CloudNativePG `<cluster>-app` with `username/password/host/port/dbname/uri/jdbc-uri/pgpass`; Zalando `{user}.{cluster}.credentials.postgresql.acid.zalan.do`). Requires namespaced `get` on specific secrets — design a minimal Role. [Observed: CNPG `applications.md`; Zalando user guide]
12. **Bare metal:** PostgreSQL peer auth via `pg_ident` map for a dedicated `karagoz` OS user, MariaDB `unix_socket`, MySQL `auth_socket`, Debian's `/etc/mysql/debian.cnf`, Redis `requirepass`/`aclfile` in `redis.conf`. Peer/socket auth is the cleanest zero-credential bootstrap because no secret ever exists.
13. **Ephemeral per-user principals are worth it** even though Karagöz executes queries itself: they give server-side privilege enforcement independent of Karagöz's SQL classifier, DB-native attribution (`pg_stat_activity.usename`, MySQL processlist, SQL Server `login_name`), per-user connection limits, and RLS hooks.
14. **Never use `SET ROLE` / `SET SESSION AUTHORIZATION` / `EXECUTE AS` from an admin login to "drop privileges" for user-supplied SQL**: the user can `RESET ROLE` / `REVERT`. The connection's *login* principal must itself be the least-privileged one.
15. **Teleport's PostgreSQL auto-user procedures are the best prior art for cleanup**: temporary (`pg_temp`) procedures, a bookkeeping group role marking managed users, "strip all roles then re-grant" on activation (crash recovery), refuse to change roles while sessions are active, and fall back from DROP to NOLOGIN when the user owns objects (SQLSTATE `2BP01`). [Observed: `teleport/lib/srv/db/postgres/sql/{activate,deactivate,delete}-user.sql`, AGPL — described, not copied]
16. **Crash safety requires an on-disk journal plus a reaper**: write-ahead "intent" records before CREATE, mark "active" after GRANT, reconcile on startup against the DBMS catalogue filtered by a managed-marker (group role / comment / `customData` / naming prefix), and sweep expired principals periodically.
17. **Redis is a special case**: ACL users created with `ACL SETUSER` live only in memory unless `ACL SAVE`/`CONFIG REWRITE` is called — ephemeral users vanish on restart, which is the desired behaviour; never persist them.
18. **Read-only must be layered**: (a) DB privileges (authoritative), (b) read-only session/transaction (`default_transaction_read_only`, `START TRANSACTION READ ONLY`, ClickHouse `readonly=1`), (c) Karagöz classifier (UX + early rejection). Only (a) is non-bypassable; (b) is bypassable by the session itself on most engines except ClickHouse `readonly=1`.
19. **Consent model:** discovery results are sent to Perde as *metadata only* (engine, version, name, labels, ports, health); credentials never leave the host; an org admin must attach a database (or a `hacivat.enable=true` label pre-consents) before any user can connect.

---

## 3. Detailed findings

### 3.1 Trust boundaries and threat model for Karagöz on a host

| Actor / asset | Trust | Notes |
|---|---|---|
| Host root / Docker daemon | Fully trusted (it can do anything to Karagöz) | Karagöz cannot defend against the host it runs on. |
| Karagöz process | Trusted to enforce policy; must minimise privileges | Holds DB admin credentials in memory; holds its agent identity key. |
| Containers / other local processes | Untrusted | May set misleading labels (`hacivat.enable=true` on an attacker container) to phish users into sending queries/credentials to it. |
| Perde | Trusted for identity & policy distribution; **not** trusted with DB credentials | Perde compromise must not yield DB passwords (they never leave the host). Perde compromise *does* allow issuing tokens → see doc 05 for local approval pinning / agent-side policy floors. |
| Hacivat user | Authenticated via Perde-issued capability; authorised by policy | Can send arbitrary SQL; must be contained by DB privileges. |
| AI agents via Hacivat MCP | Same as user, plus attribution flag | See doc 05 §3.9. |

Threats specific to discovery:

- **T1 Label spoofing.** Any container can carry `hacivat.*` labels. Mitigation: label-discovered databases are still "pending attach" in Perde unless the host operator configured Karagöz with an allowlist (e.g. compose project `prod`), and the UI shows image, container name, compose project and image digest at attach time. Changes of image digest for an attached database raise an alert and suspend access until re-confirmed (ALG-DISC-01 step 9).
- **T2 Credential exfiltration through Karagöz.** Karagöz reads env secrets; a bug that logs `Config.Env` would leak them. Mitigation: a single "secret-reading" module with typed `Secret<T>` wrappers (redacted `Debug`, zeroize on drop), structured logging that cannot format secrets.
- **T3 Docker socket abuse.** If Karagöz is compromised, a mounted Docker socket lets an attacker start a privileged container with `/` bind-mounted → host root. Mitigation in §3.12.
- **T4 Stale/rotated credentials causing lockouts.** Repeated failed logins with a stale env password could trigger `fail2ban`, MySQL `FAILED_LOGIN_ATTEMPTS`/`PASSWORD_LOCK_TIME`, or SQL Server lockout policy. Mitigation: at most one validation attempt per candidate per backoff window (ALG-CRED-01 E14).
- **T5 Privilege creep of ephemeral users.** Leftover users after crashes keep privileges. Mitigation: journal + reaper (ALG-EPH-02), NOLOGIN-by-default between sessions.

### 3.2 Docker Engine API essentials for discovery

API version at the studied commit: **v1.57** (`basePath: "/v1.57"`). [Observed: `moby/api/swagger.yaml:24-27`] Clients should negotiate: call `GET /_ping` (returns `API-Version` header) or `GET /version`, then use `min(client_max, server_version)`; the Docker Go client's `WithAPIVersionNegotiation` does exactly this. [Inferred from moby client behaviour; `_ping` defined at `swagger.yaml:10571`]

Endpoints Karagöz needs (and nothing else in the default configuration):

| Endpoint | Purpose | Key fields / params | Evidence |
|---|---|---|---|
| `GET /_ping`, `GET /version` | Liveness, version negotiation, OS/arch (for Desktop detection) | `ApiVersion`, `Os`, `KernelVersion`, `Platform.Name` | `swagger.yaml:10571` |
| `GET /containers/json?all=true&filters=…` | Enumerate | Filters: `label=key`, `label=key=value`, `ancestor=<image>`, `status=`, `network=`, `publish=`, `expose=`, `health=` | `swagger.yaml:8132-8190` |
| `GET /containers/{id}/json` | Inspect | `Config.Image`, `Image` (ID/digest), `Config.Env`, `Config.Labels`, `Config.ExposedPorts`, `Config.Cmd`/`Entrypoint` (flags like `--port`, `--requirepass`), `State.{Status,Health,StartedAt,Pid}`, `NetworkSettings.Networks[*].{IPAddress,NetworkID,Aliases}`, `NetworkSettings.Ports`, `HostConfig.NetworkMode`, `Mounts[*].{Type,Source,Destination}` | `swagger.yaml:8407` |
| `GET /events?filters={"type":["container","network"]}&since=<ts>` | Change stream | Container actions include `create, start, die, destroy, health_status, rename, update, restart, pause, unpause, kill, oom, exec_*`; network `connect/disconnect` | `swagger.yaml:10731-10790` |
| `GET /containers/{id}/archive?path=…` | Read a single file (tar stream) without exec | Used for `*_FILE` secrets and config files (`redis.conf`, `users.d/*.xml`) | `swagger.yaml:9390` |
| `POST /containers/{id}/exec`, `POST /exec/{id}/start` | Optional: run a client binary inside the container | `User`, `Cmd`, `Env`, `AttachStdout` | `swagger.yaml:10999,11107` |
| `POST /containers/create` + `start` | Optional: netns helper with `HostConfig.NetworkMode = "container:<id>"` | — | Inferred from Docker semantics; ClickHouse docs use `--network=container:some-clickhouse-server` [Observed: `docker-library-docs/clickhouse/content.md:40,50`] |

**Socket locations to probe, in order** [Inferred from Docker/Podman conventions]:

1. `$DOCKER_HOST` if set (supports `unix://`, `tcp://` with TLS, `ssh://` — Karagöz should support only `unix://` and `npipe://` by default).
2. `/var/run/docker.sock` (rootful Docker; often a symlink to `/run/docker.sock`).
3. `$XDG_RUNTIME_DIR/docker.sock` (rootless Docker, e.g. `/run/user/1000/docker.sock`).
4. `/run/podman/podman.sock` (rootful Podman Docker-compatible API) and `$XDG_RUNTIME_DIR/podman/podman.sock` (rootless). Podman's compat API serves the same `/containers/json` shapes; label/event semantics are close enough to reuse the same code paths. [Inferred]
5. Windows: `npipe:////./pipe/docker_engine`.
6. containerd/nerdctl/CRI-O without a Docker-compatible socket: out of scope for v1 (would need CRI `ListContainers` with labels; Kubernetes path covers most such hosts).

**Event stream semantics that matter** [Inferred from Engine behaviour, consistent with the swagger]:

- Events are delivered at-most-once over a long-lived HTTP response; a dropped connection loses events. `since=<last_seen_time>` replays the daemon's in-memory event buffer (bounded, ~1000 recent events in moby) — so a resync by full list is mandatory after reconnect. Traefik simply re-lists everything on each relevant event and on reconnect. [Observed: `traefik/pkg/provider/docker/pdocker.go:94-145`]
- Labels are immutable after creation; `docker update` changes resources, not labels. A label change therefore always manifests as `destroy` + `create` + `start` with a **new container ID** (compose recreate). Identity of a discovered database must not be the container ID alone (see ALG-DISC-01 "stable key").
- `health_status: healthy|unhealthy` arrives as action prefix `health_status` with suffix; Traefik matches by prefix. [Observed: `pdocker.go:123-125`]
- `rename` changes `Name` only; IDs stay.
- Network `connect`/`disconnect` change reachable IPs without any container event → subscribe to `type=network` too, or periodically resync.

### 3.3 Traefik's Docker provider as the model

Studied: `traefik/pkg/provider/docker/{pdocker.go,shared.go,shared_labels.go,config.go}` and `pkg/config/label/label.go` (MIT).

1. **Namespaced flat labels decoded into typed config.** `label.Decode(labels, &conf, "traefik.docker.", "traefik.enable")` turns flat dotted keys into a struct, with filters restricting which prefixes are considered. [Observed: `shared_labels.go:43-46`, `label.go:28-32`] → For Hacivat: decode `hacivat.*` into a typed `LabelSpec` with strict validation and unknown-key warnings.
2. **Opt-in vs opt-out.** `Enable` defaults to the provider's `ExposedByDefault`; `traefik.enable=false` excludes. [Observed: `shared_labels.go:44`] → Karagöz: `discovery.docker.exposedByDefault=false` default; heuristics propose, labels enable.
3. **Constraint expression** at the provider level, e.g. `Label("env","prod")`, evaluated against container labels before any work. [Observed: `config.go:205-214`] → Karagöz config: `constraints: "LabelRegex(`com.docker.compose.project`, `^prod-`)"`-style expression (we would implement a small, safe matcher: `Label(k,v)`, `LabelRegex(k,re)`, `Image(glob)`, `&&`, `||`, `!`).
4. **Running/health filter.** Non-running containers are dropped unless `allowNonRunning`; unhealthy/starting ones are dropped unless `allowEmptyServices`. [Observed: `config.go:216-233`] → Karagöz keeps non-running containers in inventory (state `stopped`) so Perde can show them, but marks them unreachable.
5. **Network selection.** If `traefik.docker.network` is set, use that network's address; if `NetworkMode` is `host`, use node IP / `host.docker.internal` / `127.0.0.1`; if `container:<id>`, recurse into the connected container; otherwise first network found, with a warning when a named network is missing (common compose prefix mistake). [Observed: `config.go:359-425`] → Karagöz reuses this logic and adds deterministic ordering (sort network names) since map iteration is random — Traefik's "first available" is nondeterministic in Go.
6. **Port selection.** Explicit label wins; else the **lowest** exposed port. [Observed: `shared.go:189-206`] → For databases, prefer the engine's canonical port when it is exposed (5432, 3306, 27017, 6379, 1433, 9000/8123), then lowest.
7. **Bound-port option.** `UseBindPortIP` uses the host binding instead of the container IP, falling back when the binding IP is unspecified (`0.0.0.0`). [Observed: `config.go:327-357`]
8. **Compose naming.** Service name is `<service>_<project>` when compose labels exist. [Observed: `shared.go:208-215`, labels `com.docker.compose.project`/`service` at `shared_labels.go:11-14`] → Karagöz derives a stable key from `(compose project, compose service, compose container-number)`.
9. **Watch loop.** Initial list → publish config → subscribe to container events → on `start`/`die`/`health_status*` re-list all and rebuild → on stream error return error and let exponential back-off reconnect. [Observed: `pdocker.go:75-150`] Traefik does **not** debounce; under compose up of 30 containers this causes 30 full relists. Karagöz should debounce (ALG-DISC-01).

### 3.4 Proposed `hacivat.*` label scheme (Docker) and `hacivat.io/*` annotations (Kubernetes)

Design principles: (a) mirror Traefik's ergonomics; (b) every label optional; (c) secrets are *referenced*, never placed in labels (labels are world-readable through the API and `docker inspect`); (d) versioned via `hacivat.v=1` only if a breaking change ever happens.

| Label | Type / values | Default | Meaning |
|---|---|---|---|
| `hacivat.enable` | bool | agent `exposedByDefault` (false) | Pre-consent from the host operator: this container is a database to attach. `false` hides it even from heuristic proposals. |
| `hacivat.engine` | `postgres` \| `mysql` \| `mariadb` \| `mongodb` \| `redis` \| `mssql` \| `clickhouse` \| `valkey` … | heuristic | Overrides image heuristics. |
| `hacivat.name` | string ≤ 63, `[a-z0-9-]` | `<compose service>` or container name | Display / resource name in Perde (unique per agent; collision → suffix). |
| `hacivat.description` | string | — | UI description. |
| `hacivat.port` | int | engine canonical port if exposed | In-container port to connect to. |
| `hacivat.network` | Docker network name | first network (sorted) | Which network to use for container-IP connections (like `traefik.docker.network`). |
| `hacivat.connect` | `auto` \| `network` \| `published` \| `netns` \| `socket` | `auto` | Force a connection path (ALG-DISC-04). |
| `hacivat.socket` | in-container path | — | Unix socket path for `socket` mode (must be on a bind mount). |
| `hacivat.tls` | `disable` \| `prefer` \| `require` \| `verify-full` | `prefer` | Client TLS mode. |
| `hacivat.credentials.source` | `auto` \| `env` \| `file` \| `none` \| `peer` | `auto` | Restrict credential discovery (ALG-CRED-01). `none` = no auth required (e.g. dev Redis). |
| `hacivat.credentials.user-env` | env var name | engine default | Which env var holds the admin username. |
| `hacivat.credentials.password-env` | env var name | engine default | Which env var holds the admin password (or `<NAME>_FILE`). |
| `hacivat.credentials.file` | in-container path | — | File containing the password (read via archive API). |
| `hacivat.databases` | comma list | all non-system | Restrict which logical databases are exposed. |
| `hacivat.readonly` | bool | false | Host-operator floor: never allow writes through Hacivat, whatever Perde says (agent-side policy floor, doc 05 ALG-AUTHZ-02). |
| `hacivat.provisioning` | `ephemeral` \| `tier` \| `shared` | `ephemeral` if admin creds allow, else `shared` | How user sessions get DB principals (§3.15). |
| `hacivat.labels.<key>` | string | — | Arbitrary labels promoted to Perde resource labels for RBAC matching (`env`, `team`, `pii`). Keys `[a-z0-9_.-]{1,63}`. |
| `hacivat.masking.<column-selector>` | masking rule id | — | Optional host-declared masking hints, e.g. `hacivat.masking.public.users.email=email-partial` (doc 05 ALG-MASK-01). |

Example (compose):

```yaml
services:
  db:
    image: postgres:17
    environment:
      POSTGRES_PASSWORD_FILE: /run/secrets/pg_pw
    secrets: [pg_pw]
    labels:
      hacivat.enable: "true"
      hacivat.name: "orders-db"
      hacivat.labels.env: "prod"
      hacivat.labels.team: "payments"
      hacivat.labels.pii: "true"
secrets:
  pg_pw:
    file: ./secrets/pg_pw.txt
```

Kubernetes annotations use the same keys under `hacivat.io/` (annotation keys are DNS-prefixed): `hacivat.io/enable`, `hacivat.io/engine`, `hacivat.io/port`, `hacivat.io/credentials-secret: <name>`, `hacivat.io/credentials-user-key: username`, `hacivat.io/credentials-password-key: password`, `hacivat.io/label.env: prod`. Annotations (not labels) because values may contain characters invalid in label values. [Inferred design]

### 3.5 Image heuristics (classification)

Heuristics only **propose**; confidence score drives UI wording ("Looks like PostgreSQL 17"). Order of evidence (strongest first): explicit label → image repository match → process/command match (`Config.Cmd`/`Entrypoint` contains `postgres`, `mysqld`, `mariadbd`, `mongod`, `redis-server`, `valkey-server`, `sqlservr`, `clickhouse-server`) → exposed canonical port → environment variable signature → active protocol probe (ALG-DISC-02).

| Engine | Image repository patterns (after stripping registry and tag) | Canonical ports | Env signature | Version source |
|---|---|---|---|---|
| PostgreSQL | `postgres`, `library/postgres`, `postgis/postgis`, `timescale/timescaledb*`, `pgvector/pgvector`, `bitnami/postgresql*`, `bitnamilegacy/postgresql`, `supabase/postgres`, `crunchydata/crunchy-postgres`, `ghcr.io/cloudnative-pg/postgresql`, `percona/percona-postgresql*`, `cimg/postgres`, `circleci/postgres` | 5432 | `POSTGRES_*`, `PGDATA`, `PG_MAJOR`, `PG_VERSION`, `POSTGRESQL_*` (Bitnami) | `PG_VERSION` env [Observed: `docker-library-postgres/18/trixie/Dockerfile:89-92`], startup `ParameterStatus server_version` |
| MySQL | `mysql`, `mysql/mysql-server`, `container-registry.oracle.com/mysql/*`, `bitnami/mysql`, `percona/percona-server`, `percona`, `circleci/mysql` | 3306, 33060 (X) | `MYSQL_ROOT_PASSWORD`, `MYSQL_*`, `MYSQL_MAJOR`, `MYSQL_VERSION` | handshake `server_version` |
| MariaDB | `mariadb`, `library/mariadb`, `bitnami/mariadb`, `linuxserver/mariadb`, `yobasystems/alpine-mariadb` | 3306 | `MARIADB_*` (and `MYSQL_*` aliases) | handshake contains `MariaDB` |
| MongoDB | `mongo`, `mongodb/mongodb-community-server`, `mongodb/mongodb-enterprise-server`, `bitnami/mongodb`, `percona/percona-server-mongodb` | 27017 | `MONGO_INITDB_*`, `MONGO_VERSION`, `MONGODB_*` (Bitnami) | `hello`/`buildInfo` |
| Redis-compatible | `redis`, `redis/redis-stack*`, `bitnami/redis`, `valkey/valkey`, `bitnami/valkey`, `eqalpha/keydb`, `docker.dragonflydb.io/dragonflydb/dragonfly`, `redis/redis-stack-server` | 6379 | `REDIS_PASSWORD` (Bitnami), `REDIS_VERSION`, `VALKEY_*` | `HELLO 3`/`INFO server` |
| SQL Server | `mcr.microsoft.com/mssql/server`, `mcr.microsoft.com/azure-sql-edge`, `mcr.microsoft.com/mssql/rhel/server` | 1433 | `ACCEPT_EULA`, `MSSQL_SA_PASSWORD`, `SA_PASSWORD` (deprecated), `MSSQL_PID` | `SELECT @@VERSION`, PRELOGIN version |
| ClickHouse | `clickhouse/clickhouse-server`, `clickhouse` (official library image), `yandex/clickhouse-server` (legacy), `bitnami/clickhouse` | 9000 (native), 8123 (HTTP), 9440/8443 (TLS) | `CLICKHOUSE_USER`, `CLICKHOUSE_PASSWORD(_FILE)`, `CLICKHOUSE_DB`, `CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT` | `SELECT version()` |

Caveats: Bitnami moved its versioned images to the unpatched `bitnamilegacy/*` archive starting 2025-08-28 (public `docker.io/bitnami` catalogue deletion postponed to 2025-09-29), with the paid "Bitnami Secure Images" catalogue replacing it and only a dev/test free tier remaining under `bitnami/`, so self-hosters will run a mix of names [Observed: https://community.broadcom.com/blogs/beltran-rueda-borrego/2025/08/18/how-to-prepare-for-the-bitnami-changes-coming-soon; verified 2026-11-11]; images may be mirrored (`registry.example.com/library/postgres`) → match on the **last path components**, not full reference; `Config.Image` may be a bare image ID when the image was retagged → fall back to `Image` inspect's `RepoTags`/`RepoDigests` (needs `GET /images/{id}/json`, which is a separate allowlist entry in docker-guard).

### 3.6 Credential sources per official image (verified)

#### 3.6.1 PostgreSQL (`postgres`)

- Variables: `POSTGRES_PASSWORD` (required unless `POSTGRES_HOST_AUTH_METHOD=trust`), `POSTGRES_USER` (default `postgres`), `POSTGRES_DB` (default = user), `POSTGRES_INITDB_ARGS`, `POSTGRES_HOST_AUTH_METHOD`. All except the last go through `file_env`, so `POSTGRES_PASSWORD_FILE`/`POSTGRES_USER_FILE`/`POSTGRES_DB_FILE` work; setting both `X` and `X_FILE` is a startup error. [Observed: `docker-entrypoint.sh:9-25,235-240`]
- **Bootstrap-only:** initialisation happens only if `$PGDATA/PG_VERSION` does not exist. [Observed: `docker-entrypoint.sh:246-247,345-372`]
- **Process env is scrubbed** (`unset "${!POSTGRES_@}"`) before `exec postgres`, so reading `/proc/<pid>/environ` of the postgres process yields nothing — but `Config.Env` in the container inspect still contains them. [Observed: `docker-entrypoint.sh:381`]
- **pg_hba:** the entrypoint appends exactly one line `host all all all <method>` where method defaults to the server's `password_encryption` (scram-sha-256 on 14+). It does not override initdb's earlier lines, and initdb (without `--auth-local`) emits `trust` for `local` and loopback `host` lines. Hence **inside the container, socket and 127.0.0.1 connections as any role are trusted** unless `POSTGRES_INITDB_ARGS` contained `--auth-local`/`--auth-host`. [Observed: `docker-entrypoint.sh:266-285`] [Observed: docker test, postgres:16/17/18 — `pg_hba.conf` has `local`/`127.0.0.1`/`::1` `trust` lines plus `host all all all scram-sha-256`; `docker exec … psql -U postgres` needs no password; `VALID UNTIL` is ignored under trust, only `NOLOGIN` blocks; see verification/01-db-behavior-tests.md#local-auth]
- PG 18+ image: `PGDATA=/var/lib/postgresql/18/docker`, `VOLUME /var/lib/postgresql`. [Observed: `18/trixie/Dockerfile:189-192`] Relevant if Karagöz ever reads `pg_hba.conf` via archive API (`$PGDATA/pg_hba.conf`).
- Bitnami (`bitnami/postgresql`): `POSTGRESQL_PASSWORD`, `POSTGRESQL_POSTGRES_PASSWORD` (admin when a custom user is set), `POSTGRESQL_USERNAME`, `POSTGRESQL_DATABASE`, `*_FILE` variants. [Inferred from Bitnami README conventions; verify per version]

#### 3.6.2 MySQL (`mysql`)

- `MYSQL_ROOT_PASSWORD` | `MYSQL_ALLOW_EMPTY_PASSWORD` | `MYSQL_RANDOM_ROOT_PASSWORD` (one required), `MYSQL_ROOT_HOST` (default `%`), `MYSQL_DATABASE`, `MYSQL_USER`+`MYSQL_PASSWORD` (non-root; `MYSQL_USER=root` is rejected), `MYSQL_ONETIME_PASSWORD` (expires root after first use). All via `file_env`. [Observed: `docker-library-mysql/docker-entrypoint.sh:139-164,230-234,341-343`]
- Random root password is **printed to stdout** as `GENERATED ROOT PASSWORD: …`, i.e. it lands in `docker logs`. [Observed: `:271-273`] Policy: Karagöz does **not** parse logs for credentials by default; an explicit agent flag `--allow-log-credential-harvest` may enable it for a one-time bootstrap, after which ALG-EPH should rotate to a Karagöz-owned admin.
- `root@localhost` and `root@%` both get the root password; no socket-auth user is created. [Observed: `:275-303`] [Observed: docker test, mysql:8.0/8.4 — `docker exec … mysql -uroot` → `ERROR 1045 … (using password: NO)`; both rows use `caching_sha2_password`; see verification/01-db-behavior-tests.md#local-auth]
- The non-root user gets `GRANT ALL ON \`db\`.*` — useful as a fallback admin for a single schema but cannot create users (no `CREATE USER` privilege). [Observed: `:315-321`]

#### 3.6.3 MariaDB (`mariadb`)

- `MARIADB_ROOT_PASSWORD` | `MARIADB_ROOT_PASSWORD_HASH` | `MARIADB_ALLOW_EMPTY_ROOT_PASSWORD` | `MARIADB_RANDOM_ROOT_PASSWORD` (required at init), `MARIADB_ROOT_HOST`, `MARIADB_DATABASE`, `MARIADB_USER`, `MARIADB_PASSWORD(_HASH)`, replication vars; all `MYSQL_*` names are accepted as aliases. [Observed: `mariadb-docker/docker-entrypoint.sh:168-172,269-290`]
- `*_PASSWORD_HASH` variants mean the env may contain **only a hash** → no usable credential; resolution must skip and fall to another source. [Observed: `:172,275-276,397-400`]
- `mariadb-install-db … --auth-root-authentication-method=normal` → root@localhost uses a password, not `unix_socket`. [Observed: `:234`]
- Optional `MARIADB_MYSQL_LOCALHOST_USER` creates `mysql@localhost IDENTIFIED VIA unix_socket` with `MARIADB_MYSQL_LOCALHOST_GRANTS` (default USAGE). [Observed: `:414-426`]
- Healthcheck users `healthcheck@{127.0.0.1,::1,localhost}` with a random 32-char password, USAGE (or `MARIADB_HEALTHCHECK_GRANTS`), written to `<datadir>/.my-healthcheck.cnf` (mode 0600). [Observed: `:345-364`] → Usable for **liveness/version probing** by Karagöz via netns helper, not for data.
- Random root printed to logs as with MySQL. [Observed: `:379-382`]

#### 3.6.4 MongoDB (`mongo`)

- `MONGO_INITDB_ROOT_USERNAME` + `MONGO_INITDB_ROOT_PASSWORD` (both or neither; `_FILE` supported). When present, the entrypoint adds `--auth` and creates the user in `admin` with role `root`. [Observed: `docker-library-mongo/docker-entrypoint.sh:248-266,371-380`]
- Without them, **no authentication** is enabled — any client reaching the port is root-equivalent.
- `MONGO_INITDB_DATABASE` (default `test`) only for init scripts. [Observed: `:383`]
- Env scrubbed from process (`unset "${!MONGO_INITDB_@}"`) but visible in inspect. [Observed: `:415`]
- Replica sets with `--keyFile` imply auth; the keyfile itself is an internal-auth secret and must **not** be used by Karagöz. [Observed: `:313-315`]

#### 3.6.5 Redis (`redis`)

- No password variable in the official image; auth requires a config file or command flag (`redis-server --requirepass …` → visible in `Config.Cmd`; or `--aclfile`). The image patches the source to disable protected mode so that other containers can connect. [Observed: `docker-library-redis/7.2/debian/Dockerfile:85-90`; `docker-library-docs/redis/content.md:11`] [Observed: docker test, redis:7 (7.4.12) — `protected-mode no`, `user default on nopass … +@all`, PING from another container → PONG; see verification/01-db-behavior-tests.md#redis-noauth]
- Bitnami/Valkey: `REDIS_PASSWORD(_FILE)`, `ALLOW_EMPTY_PASSWORD`, `VALKEY_PASSWORD`. [Inferred from Bitnami conventions]
- Credential sources in order: `Config.Cmd` args `--requirepass X` / `--user …` → `REDIS_PASSWORD`/`_FILE` → config file named in cmd (read via archive: `requirepass`, `user … on >pw`, `aclfile`) → no auth.

#### 3.6.6 SQL Server (`mcr.microsoft.com/mssql/server`)

- `ACCEPT_EULA` (required), `MSSQL_SA_PASSWORD` (`SA_PASSWORD` deprecated), optional `MSSQL_DB` + `MSSQL_USER` + `MSSQL_PASSWORD` (newer images create a DB and a non-sa user), `MSSQL_PID`, `MSSQL_TCP_PORT` (default 1433). No `_FILE` support documented. [Observed: Microsoft Learn env var page, updated 2026-10-14]
- No socket/peer auth on Linux; `sa` is the only zero-config admin. Password policy: ≥8 chars, 3 of 4 classes, ≤128 chars → ephemeral password generator must satisfy it.

#### 3.6.7 ClickHouse (`clickhouse/clickhouse-server`)

- `CLICKHOUSE_USER` (default `default`), `CLICKHOUSE_PASSWORD` or `CLICKHOUSE_PASSWORD_FILE` (file read only if it exists), `CLICKHOUSE_DB`, `CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT` (0/1), `CLICKHOUSE_SKIP_USER_SETUP`. [Observed: `clickhouse/docker/server/entrypoint.sh:52-60`]
- If a user or password is set, the `default` user is **removed** and the named user is created with `networks ::/0` and `access_management` = the env flag. If nothing is set and the default user is unmodified, `default` is restricted to `::1`/`127.0.0.1` (password-less). [Observed: `:125-168`] [Observed: docker test, clickhouse/clickhouse-server:24.8 (24.8.14.39) — another container gets `Code: 516 … Authentication failed`; loopback and, notably, the container's own eth0 IP are accepted; with `CLICKHOUSE_PASSWORD` set, `users.d/default-user.xml` removes and recreates `default` with `<ip>::/0</ip>` and `access_management 0`; see verification/01-db-behavior-tests.md#ch-default]
- Consequence: for zero-config ClickHouse, Karagöz must use a netns helper to connect from loopback; to create ephemeral users it needs a user with `access_management=1` — which the default config lacks. Ephemeral provisioning on ClickHouse therefore often requires the operator to set `CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1` (or `hacivat.provisioning=shared`).

### 3.7 Reading `*_FILE` secrets safely

Paths in order of preference:

1. **Archive API (`GET /containers/{id}/archive?path=<p>`)** — returns a tar with one entry; works for bind mounts and the container's filesystem; read-only HTTP method; no process spawned inside the container. Size-limit the response (secrets are tiny: cap 64 KiB) and reject symlinks pointing outside (tar header type). [Observed endpoint: `swagger.yaml:9390`; constraints Inferred]
2. **Host path of the bind mount** — `Mounts[]` entry whose `Destination` is the file or a parent dir; compose `secrets:` with `file:` are bind mounts of the host file at `/run/secrets/<name>`. Requires Karagöz to have host read permission on that path (often root-only). [Inferred from compose secrets implementation]
3. **`docker exec cat`** — last resort; spawns a process in the container (visible in the container's process list and in `exec_create/exec_start` events, which is arguably good for transparency). Requires `EXEC` permission — avoid by default.

Caveats: Swarm secrets are tmpfs mounts; whether the archive API can read tmpfs-backed paths is **[Unknown]** (historically `docker cp` did not support tmpfs mounts) → exec fallback. The secret file may have a trailing newline; `file_env` uses `$(< file)` which strips trailing newlines → Karagöz must strip trailing `\n`/`\r\n` identically, otherwise passwords mismatch. [Observed: `docker-library-postgres/docker-entrypoint.sh:21`; bash command substitution semantics]

### 3.8 Connection paths

| Path | When it works | Pros | Cons |
|---|---|---|---|
| **A. Container IP on a Docker network** | Linux host, Karagöz on host or attached to the same network | No published port needed; matches how apps connect; TLS/hostname not required | Docker Desktop (macOS/Windows) VMs do not route container IPs to the host; containers with only `internal: true` networks still routable from host on Linux [Inferred]; IP changes on restart (re-resolve every connect) |
| **B. Published port bound to loopback** (`127.0.0.1:5432->5432`) | Port published | Stable; works on Desktop | Not always published (best practice for DBs is *not* to publish) |
| **C. Published port on 0.0.0.0/other IP** | Port published publicly | Stable | Indicates exposure; Karagöz should raise a "database exposed on all interfaces" finding to Perde |
| **D. Netns helper** (`docker run --network container:<id> <karagoz-helper>` or `nsenter -t <State.Pid> -n`) | Need loopback-only principals (PG trust, ClickHouse default, MariaDB healthcheck); or no route | Reaches `127.0.0.1` inside container → password-less where images allow it | Needs `POST /containers/create` (helper) or `CAP_SYS_ADMIN`+`CAP_SYS_PTRACE` (nsenter); helper image must be pinned by digest; adds a moving part |
| **E. Unix socket via bind mount** | Operator mounts the socket dir (e.g. `/var/run/postgresql`) to the host | Enables peer auth; no TCP | Peer UID mapping across container boundary is subtle: the server resolves the peer UID with the container's `/etc/passwd`; Karagöz would have to connect with UID 999 (postgres in Debian images) to be "postgres" [Inferred] |
| **F. `docker exec` running the native CLI** (`psql`, `mysql`, `mongosh`, `redis-cli`, `clickhouse-client`) | CLI present in image | Zero networking assumptions | Text scraping of CLI output is fragile and lossy (types, NULLs, binary), process per query; only acceptable for one-off bootstrap commands (e.g. create the Karagöz admin role), never for user queries |

Recommendation: implement A, B/C, D, E; use F only in a "bootstrap" ALG step where the trust-local default lets Karagöz create its own dedicated admin principal, after which A–C are used with that principal.

### 3.9 Password-less local bootstrap per engine (inside the container)

| Engine (official image defaults) | Password-less principal reachable from inside container? | How |
|---|---|---|
| PostgreSQL | **Yes** — any role via socket or 127.0.0.1 (`trust`) unless initdb args changed it | netns helper → `127.0.0.1:5432` user `POSTGRES_USER` (or `postgres`), or exec `psql -U postgres` |
| MySQL | No (root@localhost has password) | — |
| MariaDB | Only `healthcheck` (USAGE) and optional `mysql@localhost` via `unix_socket` | socket auth requires running as OS user `mysql` inside the container → exec as `User: mysql` |
| MongoDB | Yes if no `MONGO_INITDB_ROOT_*` (no auth at all). "Localhost exception" only while no users exist (observed: with `--auth` and zero users a 127.0.0.1 client may create exactly one user; a client from another container may not; afterwards the exception is closed) [Observed: docker test, mongo:7 (7.0.45), see verification/01-db-behavior-tests.md#mongo-localhost] | any path |
| Redis | Yes unless `requirepass`/ACL configured | any path |
| SQL Server | No | — |
| ClickHouse | Yes for `default` from loopback when no `CLICKHOUSE_USER/PASSWORD` set | netns helper → `127.0.0.1:9000/8123` |

### 3.10 Kubernetes equivalents

Discovery sources [Inferred from Kubernetes API conventions unless noted]:

1. **Annotated Services/Pods**: `hacivat.io/enable: "true"` on a Service; endpoints via EndpointSlices; port by name (`postgresql`, `mysql`, `mongodb`, `redis`, `tds`, `clickhouse-native`) or `hacivat.io/port`.
2. **Operator CRDs** (watch, read-only): CloudNativePG `Cluster` (services `<cluster>-rw`, `-ro`, `-r`; secret `<cluster>-app` of type `kubernetes.io/basic-auth` with keys `username, password, hostname, port, dbname, .pgpass, uri, jdbc-uri, fqdn-uri, fqdn-jdbc-uri`; `<cluster>-superuser` only if `.spec.enableSuperuserAccess: true`). [Observed: CNPG `applications.md`] Zalando `postgresql` CR → secret `{username}.{cluster}.credentials.postgresql.acid.zalan.do` with `username`/`password`. [Observed: Zalando user guide] Others (Crunchy PGO `<cluster>-pguser-<user>`, MongoDB Community `<name>-<db>-<user>`, Percona operators, Bitnami Helm charts `<release>-postgresql` with key `postgres-password`) — **[Inferred]**, verify per version before shipping heuristics.
3. **Workload env inspection**: a StatefulSet running `postgres:*` with `env[].valueFrom.secretKeyRef{name,key}` → resolve the Secret. Plain-text `env[].value` → same as Docker env.

Minimum RBAC for an in-cluster Karagöz (namespaced, one Role per opted-in namespace):

```yaml
# Pseudocode manifest (our own); restrict secrets by resourceNames once known.
rules:
- apiGroups: [""]           ; resources: ["services","pods","endpoints"] ; verbs: ["get","list","watch"]
- apiGroups: ["discovery.k8s.io"] ; resources: ["endpointslices"]        ; verbs: ["get","list","watch"]
- apiGroups: ["apps"]       ; resources: ["statefulsets","deployments"]   ; verbs: ["get","list","watch"]
- apiGroups: [""]           ; resources: ["secrets"] ; verbs: ["get"] ; resourceNames: ["orders-app", "orders-superuser"]
- apiGroups: ["postgresql.cnpg.io"] ; resources: ["clusters"] ; verbs: ["get","list","watch"]
```

`list`/`watch` on secrets is equivalent to reading all secrets in the namespace — avoid; use `get` with `resourceNames`, which forces a two-step flow: discovery proposes "needs access to secret X", the cluster operator applies the generated Role patch (Perde shows the exact YAML). This is the Kubernetes analogue of label pre-consent.

Connection: in-cluster Karagöz connects to `<svc>.<ns>.svc:port` directly. An out-of-cluster agent would need `pods/portforward` (`create`) — discouraged.

### 3.11 systemd / bare-metal databases

Discovery [Inferred from standard Linux layouts]:

- Listening sockets: parse `/proc/net/tcp{,6}` + `/proc/<pid>/fd` (or `ss -lntp`) → map port → PID → `/proc/<pid>/exe`/`cmdline` (`postgres -D …`, `mysqld`, `mariadbd`, `mongod --config`, `redis-server`, `sqlservr`, `clickhouse-server`). Requires root or `CAP_SYS_PTRACE`/`CAP_DAC_READ_SEARCH` to resolve other users' fds.
- Unix sockets: `/var/run/postgresql/.s.PGSQL.<port>`, `/run/mysqld/mysqld.sock`, `/var/run/redis/redis-server.sock`, `/tmp/mongodb-27017.sock`.
- systemd units: `postgresql@<ver>-<cluster>.service`, `mysql.service`, `mariadb.service`, `mongod.service`, `redis-server.service`, `mssql-server.service`, `clickhouse-server.service`. Debian's `pg_lsclusters` output gives version/cluster/port/data dir.

Zero-credential auth on bare metal (best option overall):

| Engine | Mechanism | One-time operator step (Perde shows it) |
|---|---|---|
| PostgreSQL | `peer` auth over unix socket, mapped with `pg_ident.conf` | `pg_ident.conf`: `karagoz  karagoz  karagoz_admin`; `pg_hba.conf` (before other `local` lines): `local all karagoz_admin peer map=karagoz`; `CREATE ROLE karagoz_admin LOGIN CREATEROLE;` (+ grants per §3.13). Debian default `local all postgres peer` means a root agent could also `setuid(postgres)` — not recommended. |
| MariaDB | `unix_socket` plugin (default enabled on Debian/Ubuntu; root@localhost uses it) | `CREATE USER karagoz@localhost IDENTIFIED VIA unix_socket; GRANT … ;` and run Karagöz as OS user `karagoz` |
| MySQL | `auth_socket` plugin (bundled, must be loaded) | `INSTALL PLUGIN auth_socket SONAME 'auth_socket.so'; CREATE USER 'karagoz'@'localhost' IDENTIFIED WITH auth_socket;` |
| MySQL/MariaDB (Debian legacy) | `/etc/mysql/debian.cnf` (`debian-sys-maint` user with full privileges) | Readable only by root; treat as a fallback admin credential source |
| Redis | `requirepass`/`user` lines/`aclfile` in `/etc/redis/redis.conf`; unix socket perms | Read config (root) |
| MongoDB | No OS-level auth; `security.authorization` in `/etc/mongod.conf`; X.509 possible | Operator creates a Karagöz user |
| SQL Server (Linux) | No socket auth; AD/Kerberos possible | Operator creates a Karagöz login |
| ClickHouse | `users.d/*.xml` / `users.xml` `default` user often password-less from localhost | Read config (root) |

### 3.12 Security analysis: Docker socket and least privilege

1. **Docker socket = root.** Anyone who can `POST /containers/create` can bind-mount `/` and run privileged → host root. Even `GET` endpoints leak secrets: all env vars (`/containers/{id}/json`), arbitrary files (`/containers/{id}/archive`), image layers (`/images/{id}/get`). [Inferred from Engine API semantics; endpoint list from `swagger.yaml`]
2. **docker-socket-proxy (Tecnativa)**: HAProxy in front of the socket; `POST=0` blocks non-GET/HEAD; sections toggled by env (`CONTAINERS`, `EVENTS`, `EXEC`, `IMAGES`, `NETWORKS`, `INFO`, `VERSION`, `PING`, …) matched by URL path regex; default allows only `EVENTS`, `PING`, `VERSION`. [Observed: `docker-socket-proxy/README.md:85-146`, `haproxy.cfg:48-79`] Limits: path-prefix granularity (`CONTAINERS=1` grants both `/containers/{id}/json` and `/containers/{id}/archive`), no per-container scoping, no response redaction. It reduces blast radius from "root" to "read all container secrets" — acceptable only if combined with network isolation of the proxy.
3. **Recommended: Karagöz docker-guard** (in-process allowlist on the client side is useless if the process is compromised; it must be a **separate process** with the socket, exposing a narrower socket to the main agent):
   - Allow: `GET /_ping`, `GET /version`, `GET /containers/json` (forced `all=true`, filters passed through), `GET /containers/{id}/json` (response **redacted**: env values replaced by `<redacted>` except for containers that are attached in the guard's local allowlist and only for the env names in the engine's credential spec), `GET /events` (filters forced to `type=container|network`), `GET /containers/{id}/archive` only for paths equal to the value of a `*_FILE` env var or `hacivat.credentials.file`, size ≤ 64 KiB.
   - Optional (disabled by default): `POST /containers/create` only with a body whose `Image` equals the pinned helper digest, `HostConfig.NetworkMode` = `container:<attached id>`, no mounts, no privileges, no capabilities, `ReadonlyRootfs`, `AutoRemove`; `POST /containers/{id}/start|wait|delete` for that helper only.
   - Deny everything else.
   - Implementation: ~1 kLoC; runs as root (or in the `docker` group), main agent runs as unprivileged `karagoz` user and talks to the guard over a unix socket with `SO_PEERCRED` check.
4. **Rootless Docker / Podman rootless**: the daemon runs as an unprivileged user; socket compromise yields that user, not root. Container IPs live in a separate network namespace (slirp4netns/pasta) and are **not routable from the host** → Karagöz must use published ports or a netns helper. [Inferred from rootless networking design]
5. **userns-remap**: containers' root maps to an unprivileged host UID; does not change the socket's root-equivalence.
6. **Agent-in-a-container mode**: Karagöz shipped as a container with the guard's socket mounted, attached to the DB networks it needs (`docker network connect`), `read_only: true`, `cap_drop: [ALL]`, `no-new-privileges`. The network attach itself needs `POST /networks/{id}/connect` → done once by the installer, not at runtime.
7. **Secrets in memory**: store resolved admin credentials only in memory in a zeroizing type; never write to disk except optionally encrypted with a host-bound key (TPM / `systemd-creds`) to survive restarts without re-reading env. Never transmit to Perde; never include in audit records; redact in crash dumps (`prctl(PR_SET_DUMPABLE, 0)`).
8. **Consent**: Perde shows exactly which credential source Karagöz used ("POSTGRES_PASSWORD from container env", "peer auth as karagoz_admin") on the database page; source changes are audit events (`db.credential_source.changed`).

### 3.13 Ephemeral per-user principals: exact privilege mapping per DBMS

Abstract privilege levels defined in Perde policy (doc 05): `connect` (metadata only), `read`, `write` (DML), `ddl` (schema changes), `admin` (everything the Karagöz admin can do except principal management), plus optional object scoping (`databases`, `schemas`, `tables`). Below are **our own** SQL templates per DBMS (identifiers must be quoted with the engine's identifier quoting; all literals bound or escaped). Names: see ALG-EPH-03.

Bookkeeping marker per engine (how Karagöz recognises its own principals during crash recovery):

| Engine | Marker |
|---|---|
| PostgreSQL | membership in group role `hacivat_managed` (NOLOGIN) + `COMMENT ON ROLE` JSON `{"hv":1,"sid":…,"exp":…}` |
| MySQL 8.0.21+ | `ATTRIBUTE '{"hv":1,"sid":…,"exp":…}'` on the user (stored in `mysql.user.User_attributes`) |
| MariaDB | Name prefix `hv_` + a Karagöz-owned table `hacivat_meta.principals` (MariaDB has no user attribute/comment) [Inferred] |
| SQL Server | Name prefix `hv_` + table `hacivat_meta.dbo.principals` in a Karagöz-owned database (or `master` extended property is not available for logins) [Inferred] |
| MongoDB | `customData: {hv:1, sid, exp}` on the user |
| Redis | name prefix `hv_` (in-memory only) |
| ClickHouse | name prefix `hv_` + `SETTINGS` profile/`COMMENT` (supported on users in recent versions [Inferred]) |

#### 3.13.1 PostgreSQL

One-time setup by Karagöz admin (`karagoz_admin`, which needs `CREATEROLE` and, for grants on objects it does not own, membership/ADMIN of the owner roles or `pg_read_all_data`/`pg_write_all_data` membership with ADMIN OPTION):

```sql
CREATE ROLE hacivat_managed NOLOGIN;                       -- marker group
CREATE ROLE hacivat_read  NOLOGIN;  GRANT pg_read_all_data  TO hacivat_read;    -- PG14+
CREATE ROLE hacivat_write NOLOGIN;  GRANT pg_read_all_data, pg_write_all_data TO hacivat_write;
-- Pre-14 or object-scoped: per schema
GRANT USAGE ON SCHEMA app TO hacivat_read;
GRANT SELECT ON ALL TABLES IN SCHEMA app TO hacivat_read;
GRANT SELECT ON ALL SEQUENCES IN SCHEMA app TO hacivat_read;
ALTER DEFAULT PRIVILEGES FOR ROLE app_owner IN SCHEMA app GRANT SELECT ON TABLES TO hacivat_read;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA app TO hacivat_write;
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA app TO hacivat_write;
ALTER DEFAULT PRIVILEGES FOR ROLE app_owner IN SCHEMA app GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO hacivat_write;
CREATE ROLE hacivat_ddl NOLOGIN; GRANT app_owner TO hacivat_ddl;   -- DDL = act as owner
```

Per session:

```sql
CREATE ROLE "hv_ab12cd34_r" LOGIN PASSWORD '<random 32B base64url>'
  VALID UNTIL '<now+ttl>' CONNECTION LIMIT 4 INHERIT   -- INHERIT required, see notes
  IN ROLE hacivat_managed;
COMMENT ON ROLE "hv_ab12cd34_r" IS '{"hv":1,"sid":"…","exp":"…","user":"<perde user id>"}';
GRANT hacivat_read TO "hv_ab12cd34_r";
ALTER ROLE "hv_ab12cd34_r" SET default_transaction_read_only = on;   -- read tier only
ALTER ROLE "hv_ab12cd34_r" SET statement_timeout = '60s';
ALTER ROLE "hv_ab12cd34_r" SET search_path = …;                    -- optional
-- Database restriction: REVOKE CONNECT ON DATABASE x FROM PUBLIC is a global change; instead
-- grant CONNECT only where needed if PUBLIC was already revoked by the operator.
```

Notes:
- `INHERIT` must be on (default) so that privileges of `hacivat_read` apply without `SET ROLE`. On PG16+, `GRANT … WITH INHERIT TRUE, SET FALSE` lets the user inherit privileges without being able to `SET ROLE hacivat_write`-like escalation; for `hacivat_ddl` use `SET FALSE` too so objects are not created *as* `app_owner` unless intended. [Observed: docker test, postgres:16/17/18 — `SET ROLE hacivat_read` → "permission denied to set role" while SELECT via inheritance works; see verification/01-db-behavior-tests.md#pg-ephemeral]
- PG16 changed `CREATEROLE`: a CREATEROLE role can only manage roles it created (it gets ADMIN OPTION on them automatically) → `karagoz_admin` with CREATEROLE is no longer near-superuser. Good for least privilege. [Observed: docker test, postgres:16/17/18 — `ALTER ROLE app_owner …` by `karagoz_admin` → "permission denied to alter role"; the whole template (create/comment/grant/ALTER ROLE SET/NOLOGIN/REVOKE/terminate/DROP ROLE) ran as `karagoz_admin`; see verification/01-db-behavior-tests.md#pg-ephemeral]
- (added after verification) Object-level grants (`GRANT USAGE ON SCHEMA app`, `GRANT SELECT ON ALL TABLES IN SCHEMA app`) issued by an admin that neither owns the objects nor holds grant options only emit `WARNING: no privileges were granted` — they do not fail. Provisioning must check for this warning (or verify with `has_table_privilege`) instead of relying on the statement succeeding. [Observed: docker test, postgres:16/17/18, see verification/01-db-behavior-tests.md#pg-ephemeral]
- `VALID UNTIL` only blocks *new password logins* after expiry; it does not terminate sessions → the reaper must also `pg_terminate_backend` on expiry. [Observed: docker test, postgres:16/18 — open session kept querying after `VALID UNTIL '2000-01-01'`; new scram login failed; note `trust` logins (inside the official container) ignore `VALID UNTIL` entirely, so deactivation must use `NOLOGIN`; see verification/01-db-behavior-tests.md#pg-ephemeral and #local-auth]
- `default_transaction_read_only` is a convenience (user can `SET` it off) — real enforcement is the absence of write privileges. `nextval()` is rejected in read-only transactions; temp tables are allowed only if `TEMP` privilege on the database (PUBLIC has it by default) — revoke `TEMP` from PUBLIC is an operator decision. [Observed: docker test, postgres:16/17/18 — after `SET default_transaction_read_only = off` the INSERT fails with "permission denied for table orders"; `CREATE TEMP TABLE` succeeds for the read tier; see verification/01-db-behavior-tests.md#pg-ephemeral]
- Functions: PUBLIC has `EXECUTE` on new functions by default; a `SECURITY DEFINER` function owned by a privileged role can write on behalf of a read-only user. Karagöz should report such functions in a pre-attach "privilege lint". [Inferred]
- Deactivate: `ALTER ROLE … NOLOGIN; REVOKE hacivat_read FROM …; SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = …`.
- Delete: `DROP ROLE` fails with `2BP01` if the role owns objects or has privileges anywhere; then `REASSIGN OWNED BY … TO hacivat_orphans; DROP OWNED BY …` **in every database of the cluster** (these are per-database), then `DROP ROLE`. Teleport instead falls back to NOLOGIN. [Observed: Teleport `delete-user.sql` (prose)] Recommendation: for `read` tier the DROP will almost always succeed; for `ddl` tier objects created during the session are reassigned to the schema owner role (`app_owner`) — not to an orphan role — so applications keep working.

#### 3.13.2 MySQL 8.x / 9.x

```sql
CREATE ROLE IF NOT EXISTS 'hacivat_read', 'hacivat_write', 'hacivat_ddl';  -- hacivat_ddl added (corrected after verification: without it the GRANT below fails with ERROR 1410)
GRANT SELECT, SHOW VIEW ON `app`.* TO 'hacivat_read';
GRANT SELECT, SHOW VIEW, INSERT, UPDATE, DELETE ON `app`.* TO 'hacivat_write';
GRANT SELECT, SHOW VIEW, INSERT, UPDATE, DELETE, CREATE, ALTER, DROP, INDEX, CREATE VIEW,
      REFERENCES, TRIGGER ON `app`.* TO 'hacivat_ddl';

CREATE USER 'hv_ab12cd34_r'@'<karagoz source host or %>'
  IDENTIFIED BY '<random>'
  WITH MAX_USER_CONNECTIONS 4
  PASSWORD EXPIRE NEVER          -- expiry handled by reaper; MySQL expiry forces password change, not lockout
  ATTRIBUTE '{"hv":1,"sid":"…","exp":"…"}';
GRANT 'hacivat_read' TO 'hv_ab12cd34_r'@'…';
SET DEFAULT ROLE 'hacivat_read' TO 'hv_ab12cd34_r'@'…';
```

- Host part: restrict to the exact address Karagöz connects from (container network gateway IP when on the host, or the agent container's IP) — `%` only if unavoidable. MySQL resolves `'u'@'h'` by most-specific host match; a different user with same name and broader host could shadow → names are unique anyway.
- Read-only session: `SET SESSION TRANSACTION READ ONLY` (bypassable by user) — enforcement is grants. [Observed: docker test, mysql:8.0 (8.0.46)/8.4 (8.4.13) — the rest of this template (ATTRIBUTE, `SET DEFAULT ROLE`, SELECT allowed, INSERT/CREATE TEMPORARY TABLE denied, `SET ROLE NONE` drops all access, `ACCOUNT LOCK` → ERROR 3118 on new logins, `DROP USER`) works as written; see verification/01-db-behavior-tests.md#mysql-ephemeral]
- Deactivate: `ALTER USER … ACCOUNT LOCK;` + `KILL <id>` for rows in `performance_schema.processlist`/`information_schema.PROCESSLIST` where `USER = …`.
- Delete: `DROP USER` — succeeds even if the user is a `DEFINER` of views/routines/events/triggers (MySQL 8 warns; objects become orphaned and fail at execution; MySQL 8.0.22+ errors for orphan-definer creation unless `SET_USER_ID`/`SET_ANY_DEFINER`). Recommendation: for `ddl` tier, before DROP, check `information_schema.{VIEWS,ROUTINES,TRIGGERS,EVENTS}` for `DEFINER = 'hv_…@…'` and either alter definer to the app owner or keep the account locked. [Inferred]
- Username limit: 32 characters. [Inferred: MySQL docs, since 5.7.8]
- MariaDB: roles exist (10.0.5+) but **no `SET DEFAULT ROLE` for others in older versions**? (MariaDB supports `SET DEFAULT ROLE r FOR user` [Inferred]); no `ATTRIBUTE` → use meta table; `ACCOUNT LOCK` supported (10.4.2+); `unix_socket` default for root on many distros.

#### 3.13.3 SQL Server

```sql
-- server level
CREATE LOGIN [hv_ab12cd34_r] WITH PASSWORD = N'<random meeting policy>', CHECK_POLICY = ON, CHECK_EXPIRATION = OFF,
       DEFAULT_DATABASE = [app];
-- per database in scope
USE [app];
CREATE USER [hv_ab12cd34_r] FOR LOGIN [hv_ab12cd34_r];
ALTER ROLE db_datareader ADD MEMBER [hv_ab12cd34_r];          -- read
-- write: ALTER ROLE db_datawriter ADD MEMBER …;  ddl: ALTER ROLE db_ddladmin ADD MEMBER …
-- belt-and-braces for read tier
DENY INSERT, UPDATE, DELETE, ALTER, EXECUTE TO [hv_ab12cd34_r];   -- EXECUTE deny optional
```

- Contained databases allow `CREATE USER … WITH PASSWORD` without a login (single-DB scope) — useful when Karagöz's admin is only `db_owner` of one DB. [Inferred]
- Deactivate: `ALTER LOGIN … DISABLE;` + `KILL <session_id>` from `sys.dm_exec_sessions WHERE login_name = …`.
- Delete: in each DB `DROP USER` (fails if user owns a schema → `ALTER AUTHORIZATION ON SCHEMA::x TO dbo` first), then `DROP LOGIN` (fails while sessions exist).
- Karagöz admin minimum: `ALTER ANY LOGIN` (server) + `ALTER ANY USER` + `ALTER ANY ROLE` in target DBs, or `securityadmin` + `db_securityadmin` (note: `securityadmin` is effectively sysadmin-equivalent because it can grant server permissions — document this). [Inferred from SQL Server security docs]
- Name limit: `sysname` = 128.

#### 3.13.4 MongoDB

```javascript
// in admin db (users authenticate against admin; roles can reference other dbs)
db.getSiblingDB("admin").createUser({
  user: "hv_ab12cd34_r",
  pwd: "<random>",
  roles: [ { role: "read", db: "app" } ],              // write: readWrite; ddl: dbAdmin + readWrite
  customData: { hv: 1, sid: "…", exp: ISODate("…") },
  mechanisms: ["SCRAM-SHA-256"]
});
```

- Deactivate: no "lock" primitive → `revokeRolesFromUser` all roles + `killAllSessionsByPattern: [{ users: [{user, db:"admin"}] }]`; or change password to random. Delete: `dropUser` (affects new auth; existing sessions keep running until killed).
- Karagöz admin minimum: `userAdmin` on `admin` (or `userAdminAnyDatabase`) — which is privilege-escalation-capable (can grant itself root). Document as unavoidable.
- Custom roles (`createRole`) allow collection-scoped privileges (`find` on `app.orders`).

#### 3.13.5 Redis (6+ ACL)

```
ACL SETUSER hv_ab12cd34_r on >'<random>' resetkeys ~* resetchannels -@all +@read +@connection -@dangerous
# write tier:  ~* +@read +@write -@dangerous -@admin
# key-scoped: %R~app:* (read-only key pattern, Redis 7+), ~app:* (read+write)
```

- `-@dangerous` removes `KEYS`, `FLUSHALL`, `CONFIG`, `DEBUG`, `MONITOR`, … — `KEYS` is in `@dangerous`, so browsing must use `SCAN` (Hacivat's Redis browser must use SCAN anyway). [Observed: docker test, redis:7 (7.4.12) — the read template allows GET/SCAN and denies SET, KEYS, SORT_RO, EVAL/FCALL, FUNCTION/SCRIPT FLUSH, CLIENT KILL/LIST/PAUSE, CONFIG SET, PUBLISH; plain `+@read -@write` still allows `KEYS`/`SORT_RO`; see verification/01-db-behavior-tests.md#redis-acl]
- (added after verification) Rule order matters: `+@scripting` listed after `-@write` re-grants `FUNCTION FLUSH/LOAD/DELETE/RESTORE`, and `SCRIPT FLUSH` is `@scripting` but not `@write` — never add `+@scripting` to a read tier; if scripts are needed, grant `+eval_ro +evalsha_ro +fcall_ro` individually. [Observed: docker test, redis:7, see verification/01-db-behavior-tests.md#redis-acl]
- Deactivate: `ACL SETUSER hv_… off` + `CLIENT KILL USER hv_…`. Delete: `ACL DELUSER hv_…` (also kills connections).
- **Do not call `ACL SAVE` / `CONFIG REWRITE`** → user disappears on restart (desired). If the server uses an `aclfile` and someone else calls `ACL SAVE`, Karagöz users get persisted → reaper must also run on startup (ALG-EPH-02 handles via prefix).
- Karagöz admin minimum: a user with `+acl` (`@admin` subset) — effectively full control.
- Replication/cluster: ACL is **per node**; in Redis Cluster/replicas, `ACL SETUSER` must be applied to every node Karagöz will connect to. [Inferred]

#### 3.13.6 ClickHouse

```sql
CREATE USER hv_ab12cd34_r IDENTIFIED WITH sha256_password BY '<random>'
  HOST IP '<karagoz ip>'
  VALID UNTIL '<now+ttl>'                    -- supported in recent versions [Inferred]
  SETTINGS readonly = 1, max_execution_time = 60;
GRANT SELECT ON app.* TO hv_ab12cd34_r;
```

- `readonly = 1` is enforced server-side and the user cannot change settings → the strongest built-in read-only mode among the engines studied. `readonly = 2` permits changing settings but not writes. [Inferred from ClickHouse settings docs]
- Requires SQL-driven access control (`access_management = 1` for the admin) and a writable access storage (`local_directory`). Users defined in `users.xml` cannot be modified by SQL.
- Delete: `DROP USER` ; kill: `KILL QUERY WHERE user = …` (ClickHouse has no persistent sessions over HTTP; native protocol sessions end with connection).

### 3.14 Server-side read-only enforcement matrix

| Engine | Authoritative (privileges) | Session-level (bypassable?) | Notes |
|---|---|---|---|
| PostgreSQL | SELECT-only grants / `pg_read_all_data` | `default_transaction_read_only` (user can SET off); `BEGIN READ ONLY` (user can COMMIT and start new) | SECURITY DEFINER functions, `dblink`/FDW, `COPY TO PROGRAM` (superuser only), large objects (`lo_*` need privileges) |
| MySQL/MariaDB | SELECT/SHOW VIEW grants | `SET SESSION TRANSACTION READ ONLY` (bypassable) ; global `super_read_only` (server-wide, not per user) | (corrected after verification) DML on *existing* temporary tables is allowed in RO transactions, but `CREATE TEMPORARY TABLE` inside `START TRANSACTION READ ONLY` fails (ERROR 1792); under `super_read_only` temp tables and `GET_LOCK` work; DDL inside `START TRANSACTION READ ONLY` executes via implicit commit [Observed: docker test, mysql:8.0/8.4, see verification/01-db-behavior-tests.md#mysql-ro and #mysql-super-ro]; `SELECT … INTO OUTFILE` needs `FILE` privilege (don't grant) |
| SQL Server | `db_datareader` + `DENY` | `ApplicationIntent=ReadOnly` only routes to readable secondaries — not enforcement | `EXECUTE` on procs owned by dbo bypasses via ownership chaining → DENY EXECUTE for read tier |
| MongoDB | `read` role | — | `$out`/`$merge` aggregation stages require write privileges → blocked by role. (corrected after verification) A user that also has `readWrite` on *another* database can `$out`/`$merge` the read-only data into it; `$where`/`$function` server-side JS run under `read` [Observed: docker test, mongo:7 (7.0.45), see verification/01-db-behavior-tests.md#mongo-read] |
| Redis | `+@read -@write` | — | Lua `EVAL` is `@scripting`; scripts run commands under caller's ACL (Redis 7) [Observed: docker test, redis:7 (7.4.12) — `EVAL`/`FCALL` calling SET → "ACL failure in script"; `+@read -@write` still permits `KEYS`; see verification/01-db-behavior-tests.md#redis-acl] |
| ClickHouse | `GRANT SELECT` | `readonly=1` (not bypassable by user) | — |

### 3.15 Provisioning modes and the `SET ROLE` pitfall

| Mode | Principal per session | When |
|---|---|---|
| `ephemeral` (default when possible) | new user per (user, database, tier) per session or lease; deleted/locked at end | Admin credential with user-management rights available |
| `tier` | a fixed small set of LOGIN principals (`hv_tier_read`, `hv_tier_write`, …) with Karagöz-managed random passwords, rotated daily | Admin cannot create users per session (e.g. ClickHouse users.xml) but operator created tier users once; or very high session churn |
| `shared` | the discovered credential itself (e.g. `postgres` superuser) | Last resort; Perde shows a red "shared superuser" badge; only the agent classifier stands between the user and the DB → allowed only for `admin` tier or with explicit org setting |

The pitfall: connecting as an admin and issuing `SET ROLE hacivat_read` (PG), `SET SESSION AUTHORIZATION`, `EXECUTE AS USER` (SQL Server) or `SET ROLE` (MySQL 8) before running user SQL **does not contain** the user — they can `RESET ROLE`, `REVERT`, or `SET ROLE ALL`. Only if Karagöz *never* passes raw user SQL could this be safe, which contradicts a SQL editor. PostgreSQL has no "irrevocable SET ROLE" for a session (PG has `SET ROLE` only). [Observed: docker test, postgres:16/17/18 — `SET ROLE ro` → `RESET ROLE` returns to `postgres`, `SET SESSION AUTHORIZATION ro` → `RESET SESSION AUTHORIZATION` likewise, and `SET ROLE` is accepted inside a `READ ONLY` transaction; see verification/01-db-behavior-tests.md#set-role. MySQL/SQL Server variants: Inferred from engine semantics] Therefore `tier` mode logs in **as** the tier user.

---

## 4. Algorithm specifications

Conventions: `now()` is the agent's monotonic-plus-wall clock pair (monotonic for timers, wall for persisted timestamps); "Perde" means the agent's control channel; all persisted agent state lives in `state.db` (SQLite, WAL mode, `synchronous=FULL` for journal tables).

### ALG-DISC-01: Docker discovery and reconciliation loop (events + periodic resync)

- **Problem:** Maintain an accurate, low-latency inventory of database containers on a host and synchronise it to Perde, tolerating dropped event streams, daemon restarts, container recreation and label spoofing.
- **Inputs / outputs:**
  - In: Docker-compatible socket (via docker-guard), agent config (`exposedByDefault`, `constraints`, `resyncInterval` default 60 s, `debounce` default 750 ms), persisted inventory from `state.db`.
  - Out: inventory map `StableKey → DiscoveredDb`, diff events to Perde (`db.discovered`, `db.updated`, `db.gone`, `db.identity_changed`), local triggers to credential resolution (ALG-CRED-01) and to session teardown when a DB disappears.
- **Algorithm:**
  1. **Connect.** Resolve socket (§3.2 order). `GET /_ping`; negotiate API version; `GET /version` → record `engine_id` (daemon ID from `/info` if allowlisted, else hash of socket path + `Os/Arch/KernelVersion`). If engine identity differs from persisted one, mark all persisted entries `unverified` (host was reinstalled or socket points to another daemon).
  2. **Subscribe first, list second.** Open `GET /events?filters={"type":["container","network"]}` *before* the full list, buffering events into a channel; record `t0` = daemon time from the first event or `/version` `Time` header [Inferred]. This prevents the race where a container starts between list and subscribe.
  3. **Full list (resync).** `GET /containers/json?all=true`. For each summary, if `Labels` contains `hacivat.enable=false` → skip. If constraint expression false → skip. Else `GET /containers/{id}/json` (parallelism ≤ 8, timeout 5 s each).
  4. **Classify** each inspected container with ALG-DISC-02 → `(engine, confidence, evidence)`; drop if `engine = none`.
  5. **Parse labels** with ALG-DISC-03 → `LabelSpec` (errors become warnings attached to the entry, never crash).
  6. **Compute stable key**: if compose labels present → `compose:<project>/<service>/<container-number>`; else if `hacivat.name` → `name:<hacivat.name>`; else `container-name:<Name without leading slash>`. Keep `container_id` and `image_digest` as mutable attributes. (Container IDs change on every recreate; names are stable across compose recreates.)
  7. **Compute endpoint candidates** with ALG-DISC-04 (not connecting yet).
  8. **Diff** new snapshot against inventory:
     - new key → `discovered` (status `pending_attach` unless `hacivat.enable=true` *and* the agent config `trustLabels=true`, then `auto_attached`).
     - existing key, changed attributes → `updated`; if `image_digest` changed **or** engine changed **or** `compose project` changed for an *attached* DB → `identity_changed`: suspend new sessions on it, require re-confirmation in Perde (T1 mitigation), unless image repository is unchanged and only the tag/digest moved within the same major version and org policy `autoAcceptMinorUpgrades=true`.
     - key missing → mark `gone_pending` with timestamp; only emit `gone` after `goneGrace` (default 120 s) to absorb compose recreate (destroy→create gap). Active sessions to a gone DB are terminated with reason `target_disappeared`.
  9. **Emit** batched diff to Perde (idempotent: each entry carries `(stable_key, generation)` where generation increments on each change; Perde ignores lower generations).
  10. **Event loop.** For each buffered or live event:
      - container `start`, `die`, `destroy`, `rename`, `health_status*`, `update`, `pause`, `unpause`, `restart`, `create` → mark `dirty(container_id)`; network `connect`/`disconnect` → mark `dirty(container_id from Actor.Attributes.container)`.
      - Ignore `exec_*`, `attach`, `top`, `resize`, `copy`, `archive-path` (our own archive reads must not trigger reconciliation storms).
      - Debounce: after first dirty mark, wait `debounce` ms (max wait 5 s) then reconcile **only dirty IDs** (inspect each; 404 → treat as destroy), recomputing steps 4–9 for them. If > 32 dirty IDs in one window → do a full resync instead.
  11. **Periodic resync** every `resyncInterval` ± 10 % jitter: full steps 3–9 (catches missed events, label drift on daemons that reuse names, network changes without events).
  12. **Stream failure.** On EOF/error: exponential back-off (initial 500 ms, factor 2, max 30 s, full jitter); on reconnect go to step 1 (subscribe-then-list). While disconnected, inventory is served with `stale=true` to Perde after 60 s; existing sessions continue (they do not depend on the Docker API).
  13. **Shutdown.** Persist inventory with generation numbers to `state.db`.
- **Data structures:**
  - `DiscoveredDb { stable_key, container_id, name, image_ref, image_digest, engine, engine_version?, confidence, labels: LabelSpec, compose: {project, service, number}?, state: running|stopped|paused|restarting|dead, health: none|starting|healthy|unhealthy, endpoints: [EndpointCandidate], cred_status: unknown|resolved(source)|failed(reason), attach_status: pending|attached|suspended|ignored, generation: u64, first_seen, last_seen, gone_since? }`
  - `dirty: HashSet<ContainerId>`, `debounce_timer`, `inventory: HashMap<StableKey, DiscoveredDb>`, `by_container: HashMap<ContainerId, StableKey>`.
- **Complexity / performance notes:** Full resync is O(N) inspects; with N=200 containers at 8 parallel × ~5 ms each ≈ 125 ms. Events cost O(dirty). Memory O(N). Avoid inspecting non-candidate containers: pre-filter summaries by image heuristic (summary has `Image`, `Labels`, `Ports`) and only inspect plausible ones plus all `hacivat.*`-labelled ones; unknown-image containers are inspected at most once per `(id, image)` and cached as `not_db`.
- **Edge cases (exhaustive list):**
  1. Container created but never started → state `created`; keep as `stopped`, no endpoints.
  2. Compose recreate (destroy + create) within grace → same stable key, new container_id; no `gone`.
  3. Two containers yield same stable key (e.g. manual `docker run --name` collision after rename, or two compose projects with same name on different daemons through one socket — impossible) → disambiguate by appending `#<short id>` and warn.
  4. `docker rename` → stable key for non-compose containers changes → treat as old key gone (after grace) + new key discovered; Perde links them via `container_id` continuity (emit `renamed` hint).
  5. Scale-out (`--scale db=3`) → three keys `…/db/1..3`; replicas of the same DB → Perde groups by `compose service` with `replica` index.
  6. Container restarting loop → flapping `start`/`die`; debounce plus "flap detector" (≥ 5 state changes in 60 s → state `restarting`, suppress per-event Perde updates).
  7. Event stream returns events out of order or duplicated → reconciliation is state-based (inspect), so order does not matter.
  8. Daemon restart → stream EOF → reconnect → full resync; containers with `restart: unless-stopped` come back with same IDs.
  9. Docker Desktop VM restart → same as 8; plus container IP routing absent (ALG-DISC-04 handles).
  10. Swarm services/tasks → containers carry `com.docker.swarm.service.name`; treat task containers like compose replicas; endpoints via task IP on overlay (only reachable if agent attached to overlay) [Inferred].
  11. Host network mode → container has no IP; endpoint is `127.0.0.1:<port>`; port conflicts between two host-mode DBs are the operator's problem; dedupe by port.
  12. `network_mode: container:<other>` → resolve IP through the other container (Traefik recursion), with cycle guard (depth ≤ 4).
  13. Label value invalid (e.g. `hacivat.port=abc`) → warning, ignore that label only.
  14. Container with `hacivat.enable=true` but no DB detected and no `hacivat.engine` → `unknown_engine` entry shown to operator with the evidence; never connectable.
  15. Inspect returns 404 (destroyed between list and inspect) → treat as destroy.
  16. Inspect slow/hangs (daemon under load) → timeout, keep previous state, retry next cycle.
  17. Thousands of containers (CI hosts) → pre-filter keeps work bounded; cap inventory at 1 000 DB entries with warning.
  18. Clock skew between agent and daemon → `since` uses daemon timestamps only.
  19. API version too old (< 1.41) → refuse with clear error (no health in list, different filter semantics).
  20. Podman compat quirks (e.g. `health_status` event naming) → normalise action strings: lowercase, take prefix before `:`.
  21. Agent itself runs in a container → exclude own container ID (from `/proc/self/cgroup` or `HOSTNAME` match) to avoid self-discovery.
  22. Image digest unavailable (locally built image) → identity uses image ID instead.
  23. Persisted inventory references DBs on an engine with different `engine_id` → all `unverified` until seen again.
- **Test cases (input → expected):**
  1. Empty host → inventory empty, no Perde messages except heartbeat.
  2. Start `postgres:17` with no labels, `exposedByDefault=false` → `discovered`, `pending_attach`, confidence ≥ 0.9.
  3. Same with `hacivat.enable=false` → nothing discovered.
  4. `docker compose up` with 3 DB services → exactly one batched diff with 3 entries after debounce.
  5. `docker compose up --force-recreate db` → no `gone`; one `updated` with new container_id; generation +1.
  6. Change image `postgres:16`→`postgres:17` on attached DB → `identity_changed`, sessions blocked until confirmed (unless policy allows).
  7. Kill event stream (simulate EOF) while starting a container → after reconnect, the container appears (resync).
  8. 50 restarts in 60 s → state `restarting`, ≤ 3 Perde updates.
  9. `docker network disconnect` the DB from its only network → endpoints update to none reachable; cred status unchanged.
  10. Container destroyed and not recreated → `gone` emitted after 120 s; active sessions terminated with `target_disappeared`.
  11. Agent container with `postgres` client image (false positive by image heuristic) → excluded as self.
  12. Container `redis:7` started with `--requirepass x` → discovered; credential source cmd (ALG-CRED-01).
- **Prior art and evidence:** Traefik docker provider (MIT) — subscribe/list/rebuild loop with back-off [Observed: `traefik/pkg/provider/docker/pdocker.go:75-150`], filtering [Observed: `config.go:197-235`], network/IP selection [Observed: `config.go:359-425`]; Docker events API [Observed: `moby/api/swagger.yaml:10731-10790`]. Kubernetes informers (list-then-watch with resourceVersion) are the same pattern [Inferred].
- **Used by:** Karagöz (core); Perde (inventory UI, attach flow).

### ALG-DISC-02: Engine classification (labels, image, process, ports, env, probe)

- **Problem:** Decide whether a container is a database and which engine/version, with a confidence score, without connecting with credentials.
- **Inputs / outputs:** In: inspect JSON, `LabelSpec`. Out: `{engine ∈ Engines ∪ {none}, confidence ∈ [0,1], evidence: [String], version?}`.
- **Algorithm:**
  1. If `LabelSpec.engine` set and valid → return `(engine, 1.0, ["label"])` (still run step 6 probe later to fill version; probe mismatch → warning `label_engine_mismatch`).
  2. Normalise image reference: strip registry host (first component containing `.` or `:` or equal to `localhost`), strip `@digest`, split tag; lowercase; candidate names = last component, last two components (`bitnami/postgresql`).
  3. Score = 0. Image match table (§3.5) → +0.6 for exact repo match, +0.4 for substring match on last component (e.g. `my-postgres-custom`).
  4. Process: `Config.Entrypoint ++ Config.Cmd` tokens contain engine binary name → +0.25.
  5. Env signature: presence of any engine-specific env prefix (`POSTGRES_`, `PG_MAJOR`, `MYSQL_`, `MARIADB_`, `MONGO_`, `REDIS_`, `MSSQL_`/`ACCEPT_EULA`, `CLICKHOUSE_`) → +0.2; take `PG_VERSION`/`MYSQL_VERSION`/`MARIADB_VERSION`/`MONGO_VERSION`/`REDIS_VERSION` as version hint.
  6. Ports: canonical port in `Config.ExposedPorts` → +0.1.
  7. Choose engine with max score; if top score < 0.35 → `none`; if two engines within 0.1 (MySQL vs MariaDB) → mark `ambiguous`, prefer env-signature winner; the active probe resolves it.
  8. **Active probe** (only for running containers with a reachable endpoint, and only unauthenticated handshakes; rate-limited 1/min/container):
     - PostgreSQL: send `SSLRequest`; reply `S`/`N` → PG-compatible (also matches CockroachDB/Yugabyte/Supabase poolers — record `pg_compatible`).
     - MySQL/MariaDB: read initial handshake packet (server sends first) → protocol version 10, server version string (`…-MariaDB` → MariaDB).
     - MongoDB: `OP_MSG hello` is allowed unauthenticated → `maxWireVersion`, `setName`.
     - Redis: `PING` → `+PONG` or `-NOAUTH` (both confirm Redis); `HELLO 3` may reveal `server: redis|valkey`, `version`.
     - SQL Server: TDS PRELOGIN → version token.
     - ClickHouse: HTTP `GET /ping` on 8123 → `Ok.`; native port: send Hello requires credentials → skip.
     Probe success sets confidence = max(confidence, 0.95) and fills version.
- **Data structures:** static `EngineSpec { id, image_patterns, binaries, env_prefixes, canonical_ports, probe_fn, cred_spec }` table, versioned in agent; updatable from Perde as signed data (no code).
- **Complexity:** O(1) per container; probes bounded by rate limit.
- **Edge cases:** image built `FROM postgres` with custom name (score from env/process); sidecars like `prometheuscommunity/postgres-exporter` (contain `POSTGRES`-like env `DATA_SOURCE_NAME`, image contains "postgres") → negative patterns list (`*exporter*`, `*pgadmin*`, `*adminer*`, `*pgbouncer*`, `*backup*`, `*wal-g*`) subtract 0.5; PgBouncer/ProxySQL containers → classified as `pooler` (separate kind; can be targets but not provisioning-capable); Redis Sentinel (`redis-sentinel` cmd, port 26379) → `sentinel` kind, not a DB; Mongo `mongos` router → `mongodb` with `role=router`; ClickHouse Keeper → not a DB; distroless images with no env → rely on probe; probe hitting a TLS-only port → handle TLS alert as "unknown TLS service".
- **Test cases:** `postgres:17-alpine` → postgres, ≥0.9, version `17.x` from env; `bitnami/postgresql:16` → postgres; `mariadb:11` → mariadb not mysql; `mysql:8.4` → mysql; `prometheuscommunity/postgres-exporter` → none; `redis/redis-stack-server` → redis; `valkey/valkey:8` → redis-compatible (engine `valkey`); `mcr.microsoft.com/mssql/server:2022-latest` → mssql; custom image `acme/db:1` with `Cmd=["postgres","-c","…"]` and `PGDATA` env → postgres ~0.45 → probe → 0.95; `edoburu/pgbouncer` → pooler.
- **Prior art and evidence:** Image/env conventions verified in official entrypoints (§3.6). Netdata/Datadog autodiscovery use image-name templates similarly [Inferred].
- **Used by:** Karagöz.

### ALG-DISC-03: `hacivat.*` label parsing and validation

- **Problem:** Turn flat labels into a typed, validated spec; tolerate typos; reject dangerous values.
- **Inputs / outputs:** In: `map<string,string>` labels (Docker) or annotations (`hacivat.io/` prefix, K8s). Out: `LabelSpec`, `[Warning]`.
- **Algorithm:**
  1. Select keys with prefix `hacivat.` (Docker) or `hacivat.io/` (K8s; map `hacivat.io/credentials-password-env` → `credentials.password-env`, `hacivat.io/label.env` → `labels.env`).
  2. For each key, split remainder on `.` into path; look up in schema tree (§3.4). Unknown path → warning `unknown_label` with Levenshtein suggestion (distance ≤ 2) — e.g. `hacivat.enabled` → "did you mean hacivat.enable".
  3. Parse value by type: bool accepts `true|false|1|0|yes|no` (case-insensitive); int port 1–65535; enum exact lowercase; name regex `^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$`; env var names `^[A-Za-z_][A-Za-z0-9_]*$`; file paths must be absolute, no `..` segments, no NUL, length ≤ 4096.
  4. Secret-looking values in any label (e.g. a value matching high-entropy ≥ 20 chars, or key containing `password` but not ending `-env`/`file`) → warning `secret_in_label`, value discarded and never logged/sent.
  5. `hacivat.labels.<k>`: k regex `^[a-z0-9]([a-z0-9_.-]{0,61}[a-z0-9])?$`, value ≤ 256 chars, max 32 such labels; reserved keys (`hacivat.*` names, `agent`, `engine`) rejected.
  6. Conflicts: `credentials.source=none` with `credentials.password-env` → warning; `connect=socket` without `socket` → error for that field (fall back to `auto`).
- **Edge cases:** duplicate keys differing by case (Docker labels are case-sensitive; treat `Hacivat.Enable` as unknown with suggestion); empty string values → treated as unset; very long label sets (>1 MB) → truncate processing at 256 hacivat keys; Unicode in values → allowed only for `name`/`description` (NFC-normalised, ≤ 200 chars).
- **Test cases:** `{hacivat.enable: "TRUE"}` → enable=true; `{hacivat.port: "70000"}` → warning, port unset; `{hacivat.password: "hunter2"}` → warning `secret_in_label`, nothing stored; `{hacivat.labels.env: "prod", hacivat.labels.Team: "x"}` → env=prod, warning for `Team`; K8s `{hacivat.io/enable: "true", hacivat.io/label.env: "prod"}` → same spec as Docker.
- **Prior art and evidence:** Traefik's `label.Decode` with prefix filters [Observed: `traefik/pkg/config/label/label.go:28-32`, `pkg/provider/docker/shared_labels.go:43-46`].
- **Used by:** Karagöz.

### ALG-DISC-04: Endpoint and connection-path selection

- **Problem:** For a discovered DB, produce an ordered list of ways Karagöz can open a TCP/unix connection, honouring host type (Linux vs Desktop vs rootless) and principal requirements (loopback-only).
- **Inputs / outputs:** In: inspect JSON, `LabelSpec`, agent environment `{os, rootless, agent_in_container, agent_networks, desktop_vm}`, `needs_loopback: bool` (from ALG-CRED-01). Out: `[EndpointCandidate{kind, address, port, network?, tls_server_name?}]`.
- **Algorithm:**
  1. `port_in` = label port, else canonical engine port if in `ExposedPorts`, else lowest exposed port, else engine canonical port (images often omit EXPOSE).
  2. If `LabelSpec.connect ≠ auto` → produce only that kind (still validated).
  3. If `needs_loopback` → `[netns]` only (plus `socket` if declared).
  4. Else build list:
     a. `network`: if Linux and not rootless and not desktop: for each network in `NetworkSettings.Networks` sorted by (label-named first, then user-defined before `bridge`, then name) with non-empty `IPAddress` → candidate `(ip, port_in)`. If agent runs in a container: only networks the agent is also attached to.
     b. `published`: for bindings of `port_in/tcp` in `NetworkSettings.Ports`: loopback-bound first (`127.0.0.1`, `::1`), then specific IPs, then `0.0.0.0`/`::` mapped to `127.0.0.1` (and record `exposed_publicly=true` finding).
     c. host network mode → `127.0.0.1:port_in`.
     d. `netns` helper as last resort if allowed by config.
  5. Connection attempts use candidates in order with 2 s connect timeout each; first success is cached per DB until the next inventory generation.
- **Edge cases:** IPv6-only networks (use `GlobalIPv6Address`); container in `none` network (only netns/exec); multiple published bindings to different host ports; `MSSQL_TCP_PORT`/`PGPORT`-style overrides (env `PGPORT` → port_in; ClickHouse port from config file — probe 9000 then 8123); macvlan/ipvlan networks are **not** reachable from the host itself by design (Linux limitation) [Inferred] → skip and prefer published/netns; Docker Desktop with `host.docker.internal`; Windows containers (npipe) out of scope.
- **Test cases:** Linux, bridge network only, unpublished → `[network 172.x:5432]`; Desktop, published `127.0.0.1:15432->5432` → `[published 127.0.0.1:15432]`; rootless, unpublished → `[netns]` (if allowed) else none + finding "unreachable: publish port or enable helper"; ClickHouse with default user and no password → needs_loopback → `[netns]`.
- **Prior art and evidence:** Traefik `getIPPort`/`getIPAddress`/`getPortBinding` [Observed: `traefik/pkg/provider/docker/config.go:327-440`].
- **Used by:** Karagöz.

### ALG-CRED-01: Credential resolution order

- **Problem:** Obtain a working *administrative* credential (or credential-less method) for a discovered DB without user input, preferring the least privileged, most stable, least secret-exposing method, and without causing lockouts.
- **Inputs / outputs:** In: `DiscoveredDb`, `EngineSpec.cred_spec`, agent config (`allowLogHarvest=false`, `allowExec=false`, `allowNetnsHelper=true`), operator-provided overrides from local config file (`/etc/karagoz/databases.d/*.toml`, root-owned 0600). Out: `ResolvedCredential { method: peer|socket|trust_loopback|password|none, user, secret: Secret<String>?, source: String (human-readable, no secret), needs_loopback: bool, capabilities: {can_create_users, can_grant, is_superuser} }` or `Unresolved(reasons[])`.
- **Algorithm (ordered; stop at first candidate that validates in step 3):**
  1. Build candidate list:
     1. **Operator-local override** in `/etc/karagoz/databases.d/` matching stable key (explicit user + password file / peer user). Highest priority: explicit host-operator intent.
     2. **Karagöz-owned admin principal** previously bootstrapped (credential stored encrypted in `state.db` bound to host key; §3.12-7). Preferred over env because it survives password rotation and is least-privilege.
     3. **Socket/peer auth** (bare metal; or `socket` label): `karagoz_admin` via `peer map`.
     4. **Labels-declared** `hacivat.credentials.{user-env,password-env,file}`.
     5. **Engine env conventions** (§3.6), in each engine's order:
        - postgres: user = `POSTGRES_USER` (or `_FILE`) else `postgres`; password = `POSTGRES_PASSWORD` else `POSTGRES_PASSWORD_FILE` (archive read); if `POSTGRES_HOST_AUTH_METHOD=trust` → method `none` over TCP.
        - mysql: `root` + `MYSQL_ROOT_PASSWORD`/`_FILE`; if `MYSQL_ALLOW_EMPTY_PASSWORD` → root with empty password; then `MYSQL_USER`/`MYSQL_PASSWORD` (schema-scoped, `can_create_users=false`).
        - mariadb: `root` + `MARIADB_ROOT_PASSWORD`/`MYSQL_ROOT_PASSWORD`(+`_FILE`); skip `*_HASH`; then `MARIADB_USER`/`MARIADB_PASSWORD`.
        - mongodb: `MONGO_INITDB_ROOT_USERNAME`/`_PASSWORD` (+`_FILE`), authSource `admin`; if neither set and no `--auth`/`--keyFile` in cmd → method `none`.
        - redis: `--requirepass`/`--user` args in `Config.Cmd`; `REDIS_PASSWORD`/`_FILE`, `VALKEY_PASSWORD`; config file from cmd (`redis-server /path/redis.conf`) via archive read → `requirepass` or `user default on >pw`; else `none`.
        - mssql: `sa` + `MSSQL_SA_PASSWORD` else `SA_PASSWORD`; then `MSSQL_USER`/`MSSQL_PASSWORD` (db-scoped).
        - clickhouse: if `CLICKHOUSE_USER`/`CLICKHOUSE_PASSWORD(_FILE)` → that user over network; else → `default` with no password, `needs_loopback=true`.
     6. **Loopback trust** (postgres official image): any role, no password, `needs_loopback=true` (netns helper) — preferred *only* to bootstrap a Karagöz admin role (ALG-EPH-01 step 0), never for sessions.
     7. **Log harvest** (`GENERATED ROOT PASSWORD:` in container logs) only if `allowLogHarvest` → immediately bootstrap Karagöz admin and recommend the operator rotate root.
     8. **Exec-based bootstrap** (`docker exec -u postgres psql …` / `mariadb` as `mysql` via unix_socket) only if `allowExec`.
  2. Deduplicate candidates by `(method, user, secret-hash)`.
  3. **Validate** each candidate in order: open connection via ALG-DISC-04 endpoints (TLS `prefer`), authenticate, run a capability query:
     - PG: `SELECT rolsuper, rolcreaterole, rolcanlogin FROM pg_roles WHERE rolname = current_user` + `SELECT version()`.
     - MySQL/MariaDB: `SHOW GRANTS FOR CURRENT_USER()` (parse `CREATE USER`, `GRANT OPTION`, `ALL PRIVILEGES ON *.*`).
     - SQL Server: `SELECT IS_SRVROLEMEMBER('sysadmin'), HAS_PERMS_BY_NAME(NULL,NULL,'ALTER ANY LOGIN')`.
     - Mongo: `connectionStatus` with `showPrivileges`.
     - Redis: `ACL WHOAMI` + `ACL GETUSER <me>` (if permitted) or `ACL DRYRUN`.
     - ClickHouse: `SHOW GRANTS`, `SELECT access_management FROM system.users WHERE name=currentUser()` (if visible).
  4. On auth failure: record `failed(candidate, reason)`; **do not retry the same candidate** for `credBackoff` (5 min, doubling to 6 h) — protects against lockouts (T4). On network failure: not counted as auth failure.
  5. First success → `ResolvedCredential`; report `source` string and `capabilities` to Perde (no secret).
  6. If resolved credential is a *bootstrap* one (env superuser, trust loopback, harvested) and `can_create_users` → run ALG-EPH-01 step 0 to create a dedicated Karagöz admin and switch to candidate 1.2 for all future work. Report to Perde `credential_source=karagoz_admin`.
  7. Re-validate on: inventory `identity_changed`, auth failure during session provisioning, every 24 h.
- **Data structures:** `CandidateCred { priority, method, user, secret_ref (lazy fetch), source_desc }`; `CredFailureCache: Map<(stable_key, candidate_fingerprint), {count, next_try}>` persisted.
- **Complexity / performance:** ≤ ~8 candidates, each one connection; secrets fetched lazily (archive read only when the candidate is tried).
- **Edge cases (exhaustive list):**
  1. Env and `_FILE` both set → image refuses to start; Karagöz prefers `_FILE` but the container is crash-looping anyway.
  2. Password rotated after init → env candidate fails → falls through; Perde shows "credential in env is stale; add an override or label".
  3. `_FILE` path not readable (tmpfs swarm secret) → exec fallback if allowed, else skip.
  4. `_FILE` content with trailing newline / CRLF → strip as bash `$(<file)` does (only trailing newlines).
  5. Password containing non-UTF-8 bytes → treat secret as bytes; MySQL/PG accept bytes; Mongo SCRAM requires SASLprep → fail with reason.
  6. Hash-only env (`MARIADB_ROOT_PASSWORD_HASH`) → skip with reason `hash_only`.
  7. `MYSQL_ONETIME_PASSWORD` → root password is expired after first use; login succeeds but every statement errors until `ALTER USER` → Karagöz must not change root's password silently; report `expired_password`.
  8. `MYSQL_RANDOM_ROOT_PASSWORD` without log harvest → unresolved; Perde shows instructions.
  9. Account lockout policies → backoff rules; never parallel-try candidates against the same server.
  10. TLS required by server (`hostssl` only, `require_secure_transport`) → retry with TLS; self-signed → `prefer` mode accepts without verification for *local* endpoints only (container IP/loopback), log fingerprint (TOFU) and pin it.
  11. Server requires client certs → unresolved unless operator override.
  12. Mongo replica set where the container is a secondary → auth works; provisioning must target the primary (`hello.primary`) → if primary is another container, map via inventory.
  13. Redis with `rename-command ACL ""` → cannot provision; fall to `shared` mode with warning.
  14. PostgreSQL with `POSTGRES_USER` not superuser (impossible in official image — initdb user is superuser) but Bitnami custom user is not superuser → use `POSTGRESQL_POSTGRES_PASSWORD` for `postgres`.
  15. ClickHouse `default` removed and named user without `access_management` → `can_create_users=false` → `tier`/`shared` mode.
  16. Multiple databases on the same server instance discovered twice (two compose services pointing at one external DB via env) → only containers that *are* the DB are discovered; application containers are not DBs (negative heuristic: app images with `DATABASE_URL` env are not DBs).
  17. Credentials found in an *application* container's `DATABASE_URL` for an external managed DB (RDS) → out of scope v1; could be a "discovered external database" feature later (privacy-sensitive; off by default).
- **Test cases:**
  1. `postgres:17` with `POSTGRES_PASSWORD=x` → method password, user postgres, superuser → bootstrap Karagöz admin → future source `karagoz_admin`.
  2. Same but password changed via `ALTER USER` → env fails → loopback trust via netns → bootstrap succeeds.
  3. `POSTGRES_INITDB_ARGS=--auth-local=scram-sha-256 --auth-host=scram-sha-256` + stale env → unresolved; Perde shows override instructions.
  4. `mariadb:11` with `MARIADB_ROOT_PASSWORD_FILE=/run/secrets/x` → archive read → root.
  5. `mysql:8.4` with `MYSQL_RANDOM_ROOT_PASSWORD=yes`, harvest disabled → unresolved(`random_root_password`).
  6. `mongo:8` with no env → method none; Perde warning "MongoDB without authentication".
  7. `redis:7` cmd `redis-server --requirepass s3cr3t` → password from cmd.
  8. `clickhouse/clickhouse-server` with no env → default user, needs_loopback → netns → `can_create_users=false` → `shared` read-only? (no: `default` has full rights) → shared mode with warning.
  9. Wrong password twice → second attempt not made within 5 min.
- **Prior art and evidence:** Entrypoint conventions [Observed: §3.6 references]; Teleport requires operator-provided admin user for auto-provisioning (`admin_user` in database spec) rather than discovering it — see doc 01 [Observed by fork in doc 01]; Vault's database engine requires a root credential configured by an operator and recommends rotating it (`rotate-root`) — see doc 03.
- **Used by:** Karagöz; Perde (displays source and remediation).

### ALG-EPH-01: Ephemeral database principal provisioning

- **Problem:** At session start, create (or re-activate) a DB principal whose privileges exactly match the user's authorised tier and object scope, with a TTL, recoverably.
- **Inputs / outputs:** In: verified session grant from doc 05 ALG-AUTHZ-02 `{session_id, perde_user_id, db_stable_key, tier ∈ {read, write, ddl, admin}, scope: {databases[], schemas[]?, tables[]?}, ttl, max_conns}`, `ResolvedCredential` (admin). Out: `SessionPrincipal {name, secret, engine-specific connect params, expires_at}`; journal entries.
- **Algorithm:**
  0. **Bootstrap (once per DB):** ensure Karagöz admin exists with minimal rights (§3.13 per engine) and the marker/tier roles exist (`hacivat_managed`, `hacivat_read`, `hacivat_write`, `hacivat_ddl` / MySQL roles / Mongo custom roles). Idempotent DDL (`IF NOT EXISTS` or catalogue checks). Record `bootstrap_version` in `state.db`; re-run when the template version increases.
  1. **Name** := ALG-EPH-03(session, tier).
  2. **Journal intent:** insert `principals(name, db, session_id, tier, scope_hash, state='creating', created_at, expires_at)` and fsync.
  3. **Reuse check:** query catalogue for `name` (PG `pg_roles`, MySQL `mysql.user`, …).
     - Exists and marked managed (§3.13 marker) and belongs to same `(perde_user, db, tier, scope_hash)` → **re-activate**: strip all grants/memberships except marker (crash leftovers), then continue at step 5. If it has live sessions and grants differ → error `principal_busy_with_different_grants` (cannot change privileges under running sessions — Teleport behaviour).
     - Exists but **not** managed → abort `name_collision` (never touch unmanaged principals).
  4. **Create** with random secret (32 bytes CSPRNG, base64url; SQL Server: ensure 3-of-4 classes by appending `aA1!`-style class chars derived deterministically from random bytes), TTL attributes where supported (`VALID UNTIL`), connection limit, marker.
  5. **Grant** tier role(s) + scope grants; apply session settings (`default_transaction_read_only`, timeouts, `readonly=1`).
  6. **Verify** by logging in as the new principal through the same endpoint and running `SELECT 1`/`PING`/`hello`, plus a negative check for read tier where cheap (e.g. PG: `SELECT has_table_privilege(...,'INSERT')` false on one sampled table; MySQL `SHOW GRANTS`).
  7. **Journal commit:** `state='active'`.
  8. Return principal; the secret is kept only in the session's memory (Karagöz is the client; Hacivat never sees it).
  9. **Lease renewal:** while session is alive, extend `expires_at` (and `VALID UNTIL`) by `ttl` when < 1/3 remains, bounded by the grant's absolute expiry from the capability token (doc 05).
- **Data structures:** `principals` journal table: `name PK, db_key, engine, session_id, perde_user_id, tier, scope_hash, state ∈ {creating, active, deactivating, deactivated, deleting, deleted, failed}, created_at, expires_at, last_error, attempts`.
- **Complexity / performance:** 3–10 statements, typically 10–50 ms on local DBs; dominated by PG `GRANT` on many tables when not using predefined roles → prefer `pg_read_all_data` or schema-level default privileges set at bootstrap rather than per-session per-table grants.
- **Edge cases (exhaustive list):**
  1. Admin credential lacks rights for a scope (PG: cannot grant on tables owned by role it is not member of) → fail with explicit privilege diagnosis; Perde shows the GRANT the operator must run.
  2. Name collision with unmanaged principal → abort (never adopt).
  3. Two concurrent sessions of the same user/tier → names differ (session-specific suffix) *or* shared lease mode (`per_user_lease`) where one principal serves multiple sessions with refcount; choose per-engine: Redis/Mongo per session; PG/MySQL per user-lease to reduce churn (configurable).
  4. DB is a read replica (PG hot standby, MySQL `super_read_only`) → cannot create users; must provision on primary and wait for replication (poll `pg_roles` on replica until visible, max 10 s) or fall back to tier mode.
  5. Galera / group replication → DDL on users replicates; ok. Redis cluster → apply to all nodes.
  6. MySQL `CREATE USER` with `ATTRIBUTE` on 8.0.20 or older / MariaDB → syntax error → use meta-table fallback (feature-detect at bootstrap).
  7. Password policy plugins (`validate_password`, SQL Server CHECK_POLICY) rejecting generated secret → regenerate with policy-satisfying generator; detect policy from error code; max 3 attempts.
  8. Clock skew DB vs agent for `VALID UNTIL` → use DB clock (`now()` from DB) when computing TTL literals.
  9. Scope references a database that does not exist → fail early.
  10. Admin connection dropped mid-way → journal state `creating`; recovery in ALG-EPH-02.
  11. PG `CONNECTION LIMIT` hit (user opens many tabs) → error surfaced to Hacivat as `connection_limit_reached` with the limit.
  12. Mongo: user created on primary, session connects to secondary before replication → retry auth with backoff ≤ 3 s.
  13. Pooled connections (PgBouncer between Karagöz and PG) → new users need PgBouncer `auth_query`; detect `pooler` kind and prefer direct endpoint.
  14. DDL tier creating objects → owner is the ephemeral user unless `SET ROLE app_owner` is done by Karagöz at session start (here SET ROLE is to *raise* ownership target, not to drop privileges — acceptable because the ephemeral user is itself bounded by membership in `hacivat_ddl`); PG16 `SET` option must be TRUE for this grant only.
- **Test cases:**
  1. PG read tier → user can `SELECT` from `app.orders`; `INSERT` fails with `42501`; `SET default_transaction_read_only=off; INSERT` still fails.
  2. PG crash after CREATE before GRANT → restart → ALG-EPH-02 deletes or re-activates; no user with LOGIN and no marker exists.
  3. MySQL 8.4 read tier → `SHOW GRANTS` shows only role; `DELETE` fails `1142`.
  4. SQL Server password policy enabled → generated password accepted on first try.
  5. Redis read tier → `GET` ok, `SET` → `NOPERM`; `KEYS *` → `NOPERM`; restart Redis → user gone.
  6. Mongo read tier on `app` → `find` ok, `insertOne` unauthorized; `customData.hv == 1`.
  7. ClickHouse read tier → `INSERT` fails; `SET readonly=0` fails.
  8. Existing unmanaged role named `hv_…` → abort `name_collision`.
- **Prior art and evidence:** Teleport auto-user procedures: marker group role, strip-then-regrant on activation, active-connection guard [Observed: `teleport/lib/srv/db/postgres/sql/activate-user.sql` (AGPL; described only)]; Vault/OpenBao `creation_statements` + `revocation_statements` with templated `{{name}}`/`{{password}}`/`{{expiration}}` (see doc 03).
- **Used by:** Karagöz.

### ALG-EPH-02: Principal cleanup, expiry and crash recovery (reaper)

- **Problem:** Guarantee that no managed principal retains LOGIN or privileges beyond its session/lease, even across agent crashes, DB restarts, network partitions and host reboots.
- **Inputs / outputs:** In: journal (`principals`), live session table, DB catalogues (filtered by marker), clock. Out: catalogue mutations, journal transitions, audit events `db.user.deactivated|deleted|orphan_cleaned`.
- **Algorithm:**
  1. **Triggers:** (a) session end (normal), (b) lease expiry timer, (c) agent startup, (d) periodic sweep every 5 min ± jitter, (e) Perde revocation push (doc 05 ALG-REVOKE-01), (f) DB rediscovered after downtime.
  2. **Session end (a):** decrement refcount; if 0 → `deactivating`: engine-specific deactivate (PG `NOLOGIN` + revoke tiers + terminate backends; MySQL `ACCOUNT LOCK` + kill; MSSQL `DISABLE` + kill; Mongo revoke roles + kill sessions; Redis `off` + `CLIENT KILL USER`; ClickHouse `DROP USER` directly) → `deactivated`; then attempt delete (step 4) unless `retainForAudit` is configured.
  3. **Expiry (b):** same as (a) but also terminates live sessions (`reason=lease_expired`), and notifies Hacivat.
  4. **Delete:** engine-specific delete; on dependency failure (PG `2BP01`, MSSQL schema ownership, MySQL definer objects) → reassign to configured owner if policy says so, else keep `deactivated` (NOLOGIN, no grants) and mark `retained_owns_objects`; record in Perde so an admin can resolve.
  5. **Startup / sweep (c,d,f):** for each attached DB with resolved admin:
     1. Fetch catalogue list of managed principals (marker filter + `hv_` prefix).
     2. Join with journal by name:
        - Catalogue has it, journal `active` and session alive → keep.
        - Catalogue has it, journal `active` but no live session (crash) → deactivate + delete.
        - Catalogue has it, journal `creating` → deactivate + delete (never resume half-provisioned).
        - Catalogue has it, journal missing (state.db lost/restored from backup) → if marker present → deactivate + delete; if only prefix matches without marker → report `suspicious_unmarked_principal`, do nothing.
        - Journal has it, catalogue does not (DB restored from backup / Redis restart) → mark `deleted`.
        - Expired `expires_at` (DB clock) → deactivate + delete.
     3. Retry failed operations with backoff; after 24 h of failure raise Perde alert.
  6. **Unreachable DB** during cleanup → keep journal state `pending_cleanup`; principals still have `VALID UNTIL`/expiry where supported, so exposure is bounded for PG/ClickHouse; for MySQL/Mongo/Redis/MSSQL exposure is bounded only by the fact that the password exists solely in Karagöz memory (lost on crash) — document this residual risk.
  7. **Agent uninstall:** `karagoz uninstall --purge-principals` runs sweep with "delete all managed" and drops marker/tier roles.
- **Data structures:** journal (above); `cleanup_queue` (priority by expires_at); per-DB mutex to serialise provisioning and cleanup.
- **Complexity:** sweep O(managed principals) per DB; catalogues queries are cheap.
- **Edge cases:** DB restored from an old backup re-introducing deleted principals with known-to-nobody passwords (sweep removes them because marker present); agent's state.db restored from backup (stale journal) — catalogue is the source of truth for existence, journal for intent; two agents managing the same DB (HA pair or accidental double install) → each agent stores `agent_id` in the marker comment/attribute and only cleans its own principals, except principals of agents Perde reports as decommissioned; DB clock jumps; principal renamed by an admin manually (no longer matches prefix → treated as unmanaged, reported); MySQL `ACCOUNT LOCK` unsupported (MariaDB < 10.4.2) → set random password instead; Redis `CLIENT KILL USER` unsupported on very old 6.0.x? (supported since 6.0 [Inferred]).
- **Test cases:** kill -9 agent during an active PG session → restart → within 1 sweep the principal is NOLOGIN/dropped and its backend terminated; DB down during session end → principal cleaned when DB returns; journal deleted → sweep still removes marked principals; principal owning a table → retained as NOLOGIN, Perde alert.
- **Prior art and evidence:** Teleport deactivate/delete procedures with active-connection guard and NOLOGIN fallback [Observed: `teleport/lib/srv/db/postgres/sql/{deactivate,delete}-user.sql`]; Vault lease revocation queue and `rollback_statements` (doc 03).
- **Used by:** Karagöz; Perde (alerts).

### ALG-EPH-03: Principal name generation

- **Problem:** Generate names that are unique, recognisable as managed, safe as identifiers in all engines, within length limits, and that do not leak PII into DB catalogues/logs more than necessary.
- **Inputs / outputs:** In: `perde_user_id`, `session_id` or lease id, tier, engine. Out: name.
- **Algorithm:**
  1. `h = base32_lower_nopad(SHA-256(org_id ‖ perde_user_id ‖ db_key ‖ tier ‖ lease_id))[0..10]`.
  2. `name = "hv_" + h + "_" + tierChar` (`r|w|d|a`) → 15 chars (fits MySQL 32, PG 63).
  3. Optional readable mode (org setting `readableDbUsernames=true`): `hv_<slug(username)[0..10]>_<h[0..6]>_<t>` where slug = lowercase ASCII, Turkish-aware transliteration (`ı→i, İ→i, ş→s, ğ→g, ç→c, ö→o, ü→u`), non-alnum → `_`; truncate to engine limit.
  4. Mapping name → user/session is stored in the journal and in the audit log (so DB-native logs can be correlated), not necessarily in the name.
- **Edge cases:** Turkish dotted/dotless I (use explicit mapping, not locale lowercasing); collision of truncated hashes (50 bits → negligible; still checked in ALG-EPH-01 step 3); engines with case folding (PG folds unquoted identifiers — always quote; MSSQL collation case-insensitive → names already lowercase); reserved words (prefix avoids).
- **Test cases:** user "Işıl Öztürk" readable mode → `hv_isil_oztur_ab12cd_r`; default mode → `hv_[a-z2-7]{10}_r`; length ≤ 32.
- **Prior art:** Vault/OpenBao `username_template` (doc 03); Teleport uses the Teleport username as DB username directly (doc 01).
- **Used by:** Karagöz.

### ALG-K8S-01: Kubernetes discovery (summary)

- **Problem:** Same as ALG-DISC-01 for clusters.
- **Algorithm:** informers (list+watch with resourceVersion) on Services, EndpointSlices, StatefulSets/Deployments (for env), and known operator CRDs in allowed namespaces; classify with ALG-DISC-02 on Pod spec (image, command, env); credentials via ALG-CRED-01 with K8s sources: annotation-declared secret → operator convention secret → `secretKeyRef` in DB pod env → plaintext env; `Forbidden` on secret `get` → produce `needs_rbac` with a generated Role patch for the operator; watch resync handled by informers (relist on `410 Gone`).
- **Edge cases:** multiple Services selecting the same pods (choose `-rw`/primary for provisioning); headless services; secrets rotated (watch secret only if `get`-able by name → periodic re-get every 10 min); cluster-scoped vs namespaced install.
- **Test cases:** CNPG cluster `orders` with `enableSuperuserAccess:false` → app user only → `can_create_users=false` → `tier`/`shared`; with superuser → ephemeral mode.
- **Used by:** Karagöz (K8s mode).

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 Karagöz

1. **Process split:** `karagoz-guard` (root or docker group; holds the Docker socket; exposes narrowed API) + `karagoz` (unprivileged; discovery, credentials, sessions, audit). Ship both in one binary with subcommands; systemd units with `ProtectSystem=strict`, `NoNewPrivileges`, `CapabilityBoundingSet=` (empty for main agent).
2. **Default posture:** `exposedByDefault=false`, `trustLabels=false` (labels mark candidates but attach still requires Perde admin click), `allowExec=false`, `allowNetnsHelper=true`, `allowLogHarvest=false`.
3. **Bootstrap-to-dedicated-admin** (ALG-CRED-01 step 6) is the key UX trick: a superuser from env is used once to create `karagoz_admin` with only user-management rights + tier roles; afterwards env rotation does not break Karagöz and the superuser credential can be forgotten (zeroized).
4. **Provisioning default:** `ephemeral` with per-user leases (not per query), TTL = min(session token TTL, 8 h), deactivate on last session close, delete on sweep.
5. **Privilege lint** at attach time: report PUBLIC grants, SECURITY DEFINER functions, Redis without auth, Mongo without auth, DB published on 0.0.0.0, PG `trust` for remote hosts (read `pg_hba_file_rules` view if admin can), `MYSQL_ROOT_HOST=%`. These become Perde "findings" — cheap, valuable, and a differentiator.
6. **Engine spec as data:** image patterns, env names, probe definitions, grant templates versioned and signed, updatable without agent release.

### 5.2 Perde

1. Inventory UI shows: engine/version, evidence ("image postgres:17, env POSTGRES_*"), credential source (string), provisioning mode badge (`ephemeral` / `tier` / `shared superuser` in red), findings, attach status.
2. Attach flow: admin reviews container identity (image digest, compose project, host) → assigns resource labels (pre-filled from `hacivat.labels.*`) → sets default tiers. Identity change → re-confirm.
3. Never request, store or display DB secrets. Operator overrides live on the host (`/etc/karagoz/databases.d/`), not in Perde. (A future "remote override" feature would require end-to-end encryption to the agent's key; keep out of v1.)
4. Generated remediation snippets (pg_hba lines, GRANTs, K8s Role patches, compose labels) — i18n'd prose, code untranslated.

### 5.3 Hacivat

1. Hacivat never receives DB credentials for Karagöz-managed databases; it receives a session handle. "Copy connection string" is disabled for these connections (or shows a local tunnel endpoint if a future wire-proxy mode exists).
2. Show the effective DB principal and tier in the connection status bar ("connected as hv_ab12cd34_r — read only, expires 17:40").

---

## 6. i18n considerations

- All discovery/credential/provisioning outcomes are **codes** with parameters (`cred.unresolved.random_root_password`, `disc.finding.published_publicly {port}`), rendered by Perde/Hacivat via Fluent (doc `i18n/`). Agent logs stay English.
- Remediation snippets: SQL/YAML untranslated; surrounding explanation translated. Never interpolate translated text into SQL.
- Turkish casing: never use locale-sensitive lowercasing for identifiers, label keys or env var names (`I`→`ı` bug would break `hacivat.I…` lookups or produce `POSTGRES_PASSWORD`→`posTgres…` mismatches). Use ASCII-only case folding (`to_ascii_lowercase`). Name slugs use the explicit transliteration table in ALG-EPH-03.
- Engine names, product names and env var names are not translated.

## 7. Risks, pitfalls, open questions

1. **Root equivalence remains** wherever the guard has the Docker socket. Rootless Docker/Podman reduce this but break container-IP routing. [Design trade-off]
2. **Env secrets are inherently exposed** to anyone with read API access; Karagöz is not making this worse but is a new consumer. Document clearly for security reviewers.
3. **Stale env credentials** are common in long-lived prod; the bootstrap-to-dedicated-admin path mitigates only if Karagöz was installed while env was valid, or via trust loopback (PG) — MySQL/MSSQL with rotated passwords need operator overrides.
4. **Admin principals are escalation-capable** on Mongo (`userAdmin`), Redis (`+acl`), SQL Server (`securityadmin`) — Karagöz compromise ⇒ DB compromise on those engines. Ephemeral users do not change this.
5. **Residual exposure after crash** for engines without native expiry (MySQL, MariaDB, Mongo, Redis, MSSQL) until the next sweep; mitigated by secrets existing only in memory. [Open: use MySQL `PASSWORD EXPIRE INTERVAL 1 DAY`? It forces change rather than lock — not suitable.]
6. **Read replicas** cannot host user creation; tier mode needed for replica-only access. [Open]
7. **Docker Desktop & rootless networking** make "zero-config" harder; netns helper requires `POST /containers/create`, which is the most dangerous API → keep opt-in. [Open: nsenter path requires root agent]
8. **Unverified items to confirm before shipping heuristics:** Bitnami env names post-2025 catalogue change; Crunchy/MongoDB/Percona operator secret names; ClickHouse `VALID UNTIL` minimum version; MariaDB user name length limit and `SET DEFAULT ROLE … FOR`; archive API behaviour on tmpfs-backed swarm secrets; Docker event buffer size for `since` replay. [Unknown]
9. **Legal/consent:** harvesting credentials from containers on a host where the installer is not the DB owner (shared hosts) — Perde ToS and an explicit "I am authorised to manage databases on this host" confirmation at install.
10. **SQL Server and Kerberos/AD-only** environments: no zero-credential path; out of scope v1.
