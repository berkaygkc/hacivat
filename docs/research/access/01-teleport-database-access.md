# 01 — Teleport Database Access: Architecture, Identity, RBAC, Auto-Users, Access Requests, Audit

> Research document for the Hacivat product family (Hacivat desktop client, Karagöz agent, Perde control plane).
> Teleport's source is AGPL-3.0. Per the clean-room rules in `docs/research/README.md`, everything below is
> described in prose and in **our own pseudocode**. No Teleport code, SQL or proto text is reproduced verbatim
> beyond identifiers (field names, event type strings, file paths) needed for citation.

---

## 1. Scope and sources

### 1.1 What this document covers

1. Teleport Database Access architecture: Auth Service, Proxy Service, Database Service (agent), reverse tunnels, TLS routing.
2. Agent join methods (static/ephemeral tokens, AWS IAM/EC2, Kubernetes, TPM, bound keypair, CI OIDC) and the join protocol.
3. Short-lived certificate identity (X.509 extensions that carry database routing and RBAC hints) and the Database CA split.
4. Protocol-aware database proxies: PostgreSQL, MySQL, MongoDB, Redis, SQL Server (plus brief notes on others): what is parsed, what is enforced, what is audited.
5. RBAC: `db_labels`, `db_labels_expression`, `db_names`, `db_users`, `db_roles`, `db_permissions`, `db_service_labels`, trait templating, deny precedence.
6. Automatic database user provisioning (create/activate, deactivate/lock, drop) per session, incl. concurrency and crash recovery.
7. Discovery Service: cloud fetchers (AWS/Azure/GCP), Kubernetes service discovery; whether Docker discovery exists.
8. Access Requests / just-in-time approvals: data model, review thresholds, state machine, interaction with certificates and locks.
9. Audit event schema for databases (`db.session.*`), session recording, recording modes, durable audit queue, upload completion.
10. Teleport Connect desktop UX, `tsh db` flows, local proxies, VNet, the database MCP server.
11. Per-session MFA (incl. database-specific relaxations), Device Trust.
12. How Teleport gates Enterprise features in code (relevant to Perde's open-core split).

Out of scope here (covered in sibling docs): Bytebase (02), other tools/secrets engines (03), Docker zero-credential discovery and canonical ephemeral-user algorithms (04), canonical authorization/audit primitives (05), open-core comparison (06).

### 1.2 Sources

| Source | Version / commit | License | Notes |
|---|---|---|---|
| `github.com/gravitational/teleport` (sparse shallow clone at `.refs/repos/teleport`) | commit `1283425b60ec5f60d509ba4c791183d452923ff7` (committer date 2026-08-25) | Source: **AGPL-3.0** (`LICENSE`); `/api` module: **Apache-2.0** (`api/LICENSE`); Community Edition binaries: **Teleport Community Edition License** (`build.assets/LICENSE-community`) | `e/` (Enterprise) is a private submodule — present as an empty directory in the clone [Observed: `ls e` empty]. |
| Teleport RFDs in-repo (`rfd/`) | same commit | AGPL-3.0 (repo) | RFD 0011, 0068, 0090, 0113, 0116, 0127, 0133, 0151, 0152, 0168, 0171, 0181, 0202, 0234 consulted. |
| Teleport blog: "Teleport OSS will relicense to AGPLv3" | published 2023-12-01 | n/a | [Observed: https://goteleport.com/blog/teleport-oss-switches-to-agpl-v3/] |
| GitHub discussion #39158 "Teleport Community Edition will restrict commercial usage starting with version 16" | 2024 | n/a | [Observed via search: https://github.com/gravitational/teleport/discussions/39158] |
| Teleport feature matrix | fetched 2026-11-11 | n/a | [Observed: https://goteleport.com/docs/feature-matrix/] |

All file paths below are relative to the repository root of the clone. Line numbers refer to the commit above.

---

## 2. Executive summary

1. **Licensing is three-layered.** Source is AGPL-3.0 since 2023-12-01; `/api` stays Apache-2.0; official Community Edition binaries (v16+) ship under a proprietary-style "Community Edition License" usable only by orgs with **< 100 employees and < US$10M annual revenue**, with a carve-out for designated critical-security patch versions [Observed: `README.md:395-409`, `build.assets/LICENSE-community:1-20`].
2. **Three planes:** Auth Service (CA, RBAC store, audit store), Proxy Service (public ingress, protocol detection/ALPN routing, identity re-encoding), Database Service (agent next to the DB; dials *out* to the proxy over a reverse tunnel, does the real authorization and protocol parsing) [Observed: `rfd/0011-database-access.md`, `lib/srv/db/server.go:1161-1300`].
3. **The agent is the policy enforcement point.** The proxy only authenticates the client certificate and routes; `db_users`/`db_names` checks, auto-user creation, query auditing all happen in the Database Service [Observed: `lib/srv/db/proxyserver.go:501-545` vs `lib/srv/db/postgres/engine.go:245-282`]. This is exactly the shape Karagöz needs for P2P.
4. **Identity = short-lived X.509 certs** whose custom extensions (OIDs under `1.3.9999.2.*`) carry `RouteToDatabase` (service name, protocol, db user, db name, db roles) plus user roles, traits, MFA-verified flag, device extensions, active access-request IDs [Observed: `lib/tlsca/ca.go:308-328, 558-588`]. The cert carries *role names*, not permissions; the agent re-evaluates roles from its cached copy of cluster state.
5. **Agent → database authentication is credential-less for self-hosted DBs:** the agent mints a per-connection client certificate (CN = database username) signed by the Database **client** CA, which the DB is configured to trust; cloud DBs use IAM tokens (RDS, Cloud SQL, Azure AD) [Observed: `lib/srv/db/common/auth.go:1104-1150`, `lib/srv/db/postgres/connector.go:61-240`]. The DB CA was split into host vs client CAs because protocols like Redis/Cassandra don't bind CN→user [Observed: `rfd/0168-database-ca-split.md`].
6. **Join methods** span shared-secret tokens (constant-time compared), cloud-attested (IAM `sts:GetCallerIdentity` challenge, EC2, Azure, GCP, Oracle), Kubernetes SA tokens, TPM EK/AK attestation, CI OIDC (GitHub/GitLab/CircleCI/…), and **bound keypair** (pre-registered public key + signed challenges + rotation + recovery counter) [Observed: `api/types/provisioning.go:39-95`, `lib/join/server.go:256-420`, `lib/join/boundkeypair/boundkeypair.go`]. Bound keypair is the best model for Karagöz's "one-command install".
7. **Protocol engines** relay bytes after authorization but tee the server stream into a parser for audit; PostgreSQL logs simple/extended-protocol messages and `CommandComplete` row counts; MySQL rejects `COM_CHANGE_USER`; MongoDB re-authorizes **every message** against `db_names` and blocks in-band auth commands; Redis intercepts `AUTH`/`HELLO`/pub-sub [Observed: `lib/srv/db/postgres/engine.go:339-523`, `lib/srv/db/mysql/engine.go:278-335`, `lib/srv/db/mongodb/engine.go:231-280`, `lib/srv/db/redis/cmds.go:56-250`].
8. **There is no general SQL statement-level allow/deny in Teleport.** For SQL engines Teleport audits queries but does not block by statement type; fine-grained control is delegated to DB-native privileges (auto-user roles or `db_permissions`) [Observed: absence of query gating in `lib/srv/db/postgres/engine.go:339-389`; `lib/srv/db/common/role/role.go:70-110` comments]. Karagöz must go further (read-only enforcement + SQL classification).
9. **RBAC evaluation**: deny rules first and greedy (any matcher hit denies), then allow rules require *all* matchers; label selector + label expression are AND-ed on allow and OR-ed on deny; MFA and device-trust requirements are aggregated "most restrictive wins" across matching allow roles [Observed: `lib/services/role.go:3030-3270, 3316-3380`].
10. **Protocol-dependent matchers:** `db_names` is enforced only where the protocol can actually confine a session to one database (PostgreSQL, MongoDB, Spanner); for MySQL, CockroachDB, Redis, SQL Server, ClickHouse etc. only `db_users` is enforced [Observed: `lib/srv/db/common/role/role.go:70-110`].
11. **Auto-user provisioning** has modes `off | keep | best_effort_drop`; the most permissive mode across matching roles wins; DB role list = union(allow `db_roles`) minus union(deny `db_roles`); username = Teleport username (hashed to `tp-<sha1-b64>` when over MySQL's 32-char limit); users are tagged by membership in a bookkeeping role `teleport-auto-user`; a cluster-wide semaphore (`db-auto-users`, 1 lease, 1-minute TTL) serializes activation per (db,user) [Observed: `lib/services/access_checker.go:862-975`, `lib/services/role.go:4097-4106`, `lib/srv/db/common/autousers.go:55-199`, `lib/srv/db/mysql/autousers.go:433-443`].
12. **Crash recovery for auto-users** is "self-healing on next activation": activation strips any left-over role grants before re-granting; deactivation/drop are skipped while the user has live backend connections; drop falls back to deactivate on dependent-object errors [Observed: `lib/srv/db/postgres/sql/*.sql` behavior; described not copied].
13. **`db_permissions`** grant object-level privileges by matching labels on imported database objects (tables etc.), with deny subtraction and wildcard clearing; mutually exclusive with `db_roles` [Observed: `lib/srv/db/common/permissions/calculation.go`, `lib/srv/db/postgres/users.go:189-272`].
14. **Access Requests**: roles- or resource-based; thresholds with filter predicates; a role is approved when *every* threshold set mapped to it has at least one threshold that reached its approve count; any deny threshold reached → DENIED; requester (or submitter) cannot review own request; one review per author; resolved requests are immutable; certs issued from an approved request are capped at the request's access expiry [Observed: `lib/services/access_request.go:503-850`, `lib/auth/auth_with_roles.go:4026-4027`].
15. **Revocation** is via **locks** (targets: user, role, MFA device, trusted device, join token, bot instance, access request, …) streamed to every agent; each live connection is monitored; in **strict** locking mode an "unreliable lock view" event terminates sessions pre-emptively [Observed: `lib/services/lock.go:104-132`, `lib/srv/monitor.go:485-520`].
16. **Audit**: `db.session.start/end/query/query.failed/result/user.create/user.deactivate/permissions.update` plus protocol-specific events; each event carries user, session, server, connection and database metadata; queries go to both the global audit log and the session recording; results only to the recording [Observed: `lib/srv/db/common/audit.go:137-427`, `lib/events/api.go:463-585`, `lib/events/codes.go:161-184`].
17. **Durability**: agents now enqueue audit events into an on-disk **SQLite queue** (WAL, max-bytes, dead-letter, corrupt quarantine, batch delivery) before forwarding; async session recordings are written locally and an **upload completer** finishes abandoned uploads and synthesizes missing `session.end` events [Observed: `lib/events/auditqueue/auditqueue.go`, `lib/events/emitter.go:262-288`, `lib/events/recorder/recorder.go:105-170`, `lib/events/complete.go:353`]. Teleport does **not** hash-chain audit events.
18. **MFA**: per-session MFA is enforced inside role evaluation (cluster-wide or per role); for databases, local "tunnel" proxies hold MFA-backed certs in memory for the proxy lifetime (RFD 0090), and RFD 0202 allows one MFA tap to authorize several DB connections briefly.
19. **Device Trust** (Enterprise): enrolled device keys (Secure Enclave/TPM) produce device extensions in user certs; modes `off | optional | required | required-for-humans`; OSS builds report effective mode `off` but **enforce** whatever mode an Enterprise Auth configured — so OSS agents in a paid cluster still enforce [Observed: `lib/devicetrust/config/config.go`, `lib/devicetrust/authz/authz.go:136-185`].
20. **No Docker discovery exists** in Teleport's Discovery Service; discovery covers AWS/Azure/GCP databases and Kubernetes clusters/services (apps via annotations). Zero-credential local discovery is a genuine gap Karagöz can own [Observed: `lib/srv/discovery/fetchers/` contents; no `docker` matches].

---

## 3. Detailed findings

### 3.1 Licensing and editions

- **Source code:** AGPL-3.0 for everything except `/api` [Observed: `README.md:395-405`, `LICENSE`, `api/LICENSE`]. The switch from Apache-2.0 happened on 2023-12-01; docs, client libraries and pre-existing Apache branches stayed Apache-2.0 [Observed: goteleport.com blog "Teleport OSS will relicense to AGPLv3"].
- **Community Edition binaries (v16+):** "Teleport Community Edition License" — only for organizations with fewer than 100 employees **and** less than US$10,000,000 annual revenue; individuals may use it for personal/hobby purposes; no reselling/embedding; a "Critical Security Exception" allows any company to install specific patch versions designated as such on the GitHub Security Advisory page [Observed: `build.assets/LICENSE-community:1-20`; GitHub discussion #39158]. Earlier (pre-16) binaries remain Apache-2.0 [Observed via search summary of Teleport announcement].
- **Practical consequence:** a company above the threshold that wants free Teleport must **build from source** and comply with AGPL. This created strong community backlash ("bait-and-switch" reviews, HN "How did you replace Teleport?") [Observed via search results: AWS Marketplace review title, HN item 41873439]. See `06-open-core-models.md` for the full comparison; the lesson for Perde is that relicensing an agent that customers run on prod servers is uniquely trust-damaging.
- **Feature matrix (Community vs Enterprise)** [Observed: goteleport.com/docs/feature-matrix, 2026-11-11]:

| Capability | Community | Enterprise |
|---|---|---|
| Database access (cert identity, RBAC) | Yes (no Oracle) | Yes |
| Per-session MFA | Yes | Yes |
| Session recording with playback | Yes | Yes |
| Access Requests | "Only can request roles through CLI" | Full JIT incl. resource requests (Identity Governance) |
| Access Request integrations (Slack, Teams, PagerDuty, …) | Listed as available | Yes |
| SSO | GitHub only | GitHub, Google, Entra, Okta, OIDC, SAML, … |
| Device Trust | No | Yes |
| Access Lists / Access Reviews | No | Yes |
| FIPS / FedRAMP | No | Self-hosted only |
| Auto DB user provisioning, `db_permissions` | Not listed; code lives in AGPL tree (`lib/srv/db/...`) → [Inferred] available in OSS builds | Yes |

The "SSO tax" (only GitHub SSO in Community) is a deliberate gate; see doc 06.

### 3.2 Architecture

```
 Hacivat-like client           Teleport Proxy                 Teleport DB Service (agent)          Database
 (psql / GUI / tsh)     mTLS   (public ingress, ALPN/SNI      (inside private network;       mTLS/IAM   (self-hosted
   + user cert  ───────────▶   routing, identity re-encode)◀── reverse tunnel (agent dials   ─────────▶ or cloud)
                                     │                         out) ── parses protocol,
                                     │                         authorizes, audits)
                                     ▼                                   │
                              Teleport Auth Service ◀────────────────────┘ (cert signing, roles/locks
                              (CAs, RBAC, audit store,                       cache stream, audit upload,
                               access requests, locks)                       semaphores)
```

- The DB service "runs inside the customer private network and proxies database client connections received from the Teleport proxy… It is also responsible for parsing of supported database protocols and authorizing requests" [Observed: `rfd/0011-database-access.md`, Architecture section; paraphrased].
- **Three authentication legs** [Observed: `rfd/0011-database-access.md` §Authentication]:
  1. Client ↔ Proxy: mTLS with the user's short-lived cert (routing info in extensions).
  2. Proxy ↔ DB service: mTLS **over the reverse tunnel**; the proxy generates an *ephemeral* cert signed by the user CA that re-encodes the client's identity, so the DB service sees the same identity [Observed: same RFD; `lib/srv/db/common/connect/connect.go` signs with `CertSigner`].
  3. DB service ↔ DB: client cert (self-hosted; CN = db user) or cloud IAM token used as password (RDS, Redshift, Cloud SQL, AlloyDB, Azure) [Observed: `lib/srv/db/postgres/connector.go:80-240`].
- **Proxy side** (`lib/srv/db/proxyserver.go`): listeners per protocol (`ServePostgres:182`, `ServeMySQL:208`, `ServeMongo:232`, `ServeTLS:268`) or TLS routing on a single port using ALPN/SNI. `Authorize` (`:501-545`) authenticates the cert, applies the client-IP override, resolves the target cluster, and fetches **all** DB servers currently heartbeating that database. `Connect` (`:419-462`) shuffles candidate agents (`connect.ShuffleRandom`) and dials them in turn → HA by running several agents serving the same DB. `Proxy` (`:464-499`) wraps the client conn with a connection monitor and copies bytes.
- **Postgres proxy handshake** (`lib/srv/db/postgres/proxy.go:129-...`): the proxy itself speaks the first bytes of the PG protocol: accepts at most one `SSLRequest` (answers `S` and upgrades to TLS, or `N` when behind the local TLS proxy), at most one `GSSEncRequest` (declined), passes `CancelRequest` through without logging the secret, then forwards the `StartupMessage` to the DB service.
- **MySQL proxy** (`lib/srv/db/mysql/proxy.go:67-290`): runs a fake MySQL server handshake advertising a server version (default `8.0.0-Teleport`, or the real version propagated via ALPN/heartbeat), accepts any username at the proxy (auth is the cert), then hands off.
- **DB service connection handling** (`lib/srv/db/server.go`):
  - `HandleConnection` (`:1161-1195`): TLS-server-handshake over the tunneled conn, extract identity into context.
  - `handleConnection` (`:1197-1298`): `authorize` → create session tracker → create session recorder → wrap conn with **ConnectionMonitor** (idle timeout, cert expiry, locks) → `dispatch` engine → rate-limit by real client IP → compute DB roles if auto-user → `engine.HandleConnection`.
  - `authorize` (`:1347-1403`): only `LocalUser`/`RemoteUser` identities allowed; fetch the proxied database by `RouteToDatabase.ServiceName`; compute `DatabaseAutoUserMode`; build `common.Session` (random UUID session ID, identity, db user/name from cert, checker, lock targets, client IP).
  - `trackSession` (`:1436-1475`): creates a `session_tracker` resource on Auth with an expiration refreshed in a loop; closed when the context ends.
- **Database resources** can be static (agent config), dynamic (`tctl create db.yaml` / API) or discovered (Discovery Service). The agent picks up dynamic ones whose labels match its `resources` matchers (`lib/srv/db/watcher.go:80-237`), reconciles them (`startReconciler:40`), and heartbeats one `db_server` object per database so proxies know which agents serve what (`server.go:774-900`). Dynamic labels are command-generated labels refreshed periodically (`server.go:629-670`). Health checks per DB feed `TargetHealth` (`server.go:1477-1566`, RFD 0203).

### 3.3 Identity: certificates, CAs, extensions

- `tlsca.Identity.RouteToDatabase` fields: `ServiceName`, `Protocol`, `Username`, `Database`, `Roles` (requested DB roles subset) [Observed: `lib/tlsca/ca.go:308-328`].
- Custom extension OIDs: `1.3.9999.1.*` for general items (kube users/groups, app session, cluster, **MFA verified** `.1.8`, **login IP** `.1.9`, renewable `.1.13`, generation `.1.14`, private-key policy `.1.15`, user type `.1.20`, …) and `1.3.9999.2.*` for database routing (service name `.2.1`, protocol `.2.2`, username `.2.3`, database `.2.4`, allowed db names `.2.5`, allowed db users `.2.6`, impersonator `.2.7`, **active access requests** `.2.8`) [Observed: `lib/tlsca/ca.go:469-588`]. Device extensions (`DeviceID`, `AssetTag`, `CredentialID`) are encoded too (`ca.go:347-355`).
- Roles are encoded as the cert's `Groups` (role names) and traits; the agent's `Authorizer` resolves role definitions from its **local cache** of Auth state and builds an `AccessChecker` [Inferred from `server.go:1357-1366` using `s.cfg.Authorizer.Authorize(ctx)` and `authContext.Checker`; consistent with Teleport's caching access point design]. Therefore role edits take effect on *new* connections immediately (without re-issuing certs) as soon as the cache updates, while revocation of *live* sessions needs locks.
- **Cert TTL:** user certs live up to the role's `max_session_ttl`; certs issued under an access request are capped at the request's access expiry [Observed: `lib/auth/auth_with_roles.go:4026-4027`]. With per-session MFA the historical default was 1-minute DB certs (RFD 0014), relaxed by RFD 0090 for in-memory tunnel proxies.
- **Agent → DB client cert:** per connection, the agent generates a fresh key pair (algorithm per the cluster's signature suite), builds a CSR with `CN = <database user>`, asks Auth to sign it with TTL = remaining session cert lifetime, and attaches the trust chain [Observed: `lib/srv/db/common/auth.go:1104-1150`; there is a TODO to cache these certs]. For MySQL self-hosted auto-users, the created user is pinned with a `REQUIRE SUBJECT "/CN=<user>"` style clause so only a cert with that subject logs in [Observed: `lib/srv/db/mysql/autousers.go:446-457`].
- **DB CA split (RFD 0168):** previously one DatabaseCA signed both DB server certs and agent client certs; a stolen DB server key could impersonate a client on protocols that don't check CN (Redis, Cassandra/Scylla). Now: `db` (host) CA signs server certs; `db_client` CA signs agent client certs; DBs should trust only `db_client` [Observed: `rfd/0168-database-ca-split.md` What/Why].
- **Teleport-managed passwords** for cloud Redis (ElastiCache/MemoryDB "managed users"): Teleport rotates passwords of users tagged for management and stores versions in AWS Secrets Manager with CURRENT/PREVIOUS semantics and test-and-set updates [Observed: `lib/srv/db/secrets/secrets.go` interface comments; `lib/srv/db/cloud/users/users.go:47-105,191`]. This is Teleport's closest analogue to a Vault-style dynamic-secret engine.

### 3.4 Agent enrollment (join methods)

**Join methods** [Observed: `api/types/provisioning.go:39-95`]: `token`, `ec2`, `iam`, `github`, `circleci`, `kubernetes`, `azure`, `gitlab`, `gcp`, `spacelift`, `tpm`, `terraform_cloud`, `bitbucket`, `oracle`, `azure_devops`, `bound_keypair`, `generic_oidc` (+ `env0`, handled in `lib/join/server.go:371-420`).

**Join protocol** (new gRPC streaming join, `lib/join/server.go:256-370`) — prose description:
1. Client sends `ClientInit` (requested system role e.g. `Db`, requested join method, token name, hostname). Untrusted strings are sanitized before being put into diagnostics.
2. Server optionally authenticates an *existing* identity (re-join: instance certificate), validates client version.
3. Server loads the provision token; computes resulting system roles; verifies the requested join method equals the token's configured method; verifies the token allows the requested role; for re-joining instances verifies scope consistency.
4. Server sends `ServerInit` (join method, signature-algorithm suite to use for keys).
5. Method-specific challenge/response:
   - `token`: client sends `TokenInit` with secret; compared in **constant time**; scoped tokens are "consumed" with usage limits, binding to the joining public key (`lib/join/server_token.go`).
   - `iam`: server sends a random challenge; client returns a **pre-signed `sts:GetCallerIdentity` request** containing the challenge in a header; server forwards it to AWS STS and matches the returned account/ARN against token allow rules (`lib/join/server_iam.go:37-95`).
   - `ec2`: instance identity document signed by AWS.
   - `tpm`: client sends EK/AK; server sends an encrypted credential (activate-credential challenge) only the genuine TPM can decrypt (`lib/join/server_tpm.go:31-60`).
   - `kubernetes`: projected service-account JWT validated via TokenReview or JWKS.
   - OIDC CI providers: verify ID token issuer/audience/claims against allow rules.
   - `bound_keypair`: see below.
6. On success the server issues host certs (TLS + SSH) for the agent's public keys and emits a join audit event (success or failure with diagnostics) [Observed: `lib/join/server.go:884-990`].

**Bound keypair join** (most relevant model for Karagöz) [Observed: `lib/join/boundkeypair/boundkeypair.go` comments `:60-840`; `api/types/types.pb.go:7418-7428`]:
- Token has `onboarding` (either a pre-registered public key, or a one-time **registration secret** that lets the client register its own key, then the secret is cleared), `recovery` (allowed number of "hard" re-joins after identity expiry, mode `standard|relaxed|insecure`), and `rotate_after` (forces key rotation on next join).
- Each join: server issues a challenge for the bound public key, client signs; on rotation request the client generates a new key, server verifies it differs and issues a challenge against the new key.
- **Join state document**: after first recovery, the client must present the signed join state from its previous join; mismatch ⇒ server **locks the token** until an admin intervenes (detects cloned agent identity).
- All status mutations are compare-and-swap with invariants (recovery count can't go below zero, bound host ID must equal expected previous value).

**Install UX:** the Web UI "Discover" flow creates a short-lived token and serves an install script at a per-token URL; the script checks for existing installs, tests connectivity, detects OS/arch/package format, installs, writes config with the token and enables `systemd`/`launchctl`; there are distinct script handlers for node, app, **database** and discovery modes (`lib/web/join_tokens.go:447-655`; `lib/web/scripts/node-join/README.md`). Note the README admits the script does not verify package checksums.

### 3.5 Discovery Service

- Database discovery fetchers exist for AWS (RDS, RDS Proxy, Aurora, Redshift, Redshift Serverless, ElastiCache (+serverless), MemoryDB, OpenSearch, DocumentDB), Azure (MySQL/Postgres flexible & single, SQL, Managed SQL, Redis, Redis Enterprise) and GCP Cloud SQL [Observed: `lib/srv/discovery/fetchers/db/*.go`].
- Loop shape (`lib/srv/discovery/database_watcher.go:40-139`): a watcher runs all fetchers every `PollInterval` (plus immediate trigger when a `DiscoveryConfig` changes), collects resources, and calls a generic **Reconciler** that diffs "current registered" vs "newly fetched". Status counters (found/enrolled/failed) are tracked per integration and failures become "user tasks" shown in the UI.
- **Generic reconciler** (`lib/services/reconciler.go:296-470`): for each currently registered resource not in the new set → `OnDelete`; for each new resource: if not registered and the matcher accepts → `OnCreate`; if registered but **origin differs** (e.g. static config vs cloud) → leave alone unless `AllowOriginChanges`; if changed (compare ≠ equal) → `OnUpdate` if still matching, else `OnDelete`. Not-found on delete is ignored. Optional bounded concurrency. This is the template for Karagöz's Docker reconcile (canonical spec lives in doc 04).
- **Kubernetes service discovery** discovers *apps* (not DBs) from Services: skip services whose discovery-type annotation mismatches or with the ignore annotation; protocol from annotation else heuristics (`appProtocol`, port name `http/https`, ports 80/443/8080, optional HTTP probe), preferred port annotation (`lib/srv/discovery/fetchers/kube_services.go:120-330`). This is the closest Teleport gets to "label/annotation-driven local discovery".
- **No Docker discovery**: there is no Docker engine fetcher in `lib/srv/discovery` [Observed: directory listing; `grep -ril docker lib/srv/discovery` returned nothing]. Self-hosted DBs must be declared statically or via dynamic resources.

### 3.6 Protocol engines

Common engine interface: `InitializeConnection(clientConn, session)`, `HandleConnection(ctx, session)`, `SendError(err)` (`lib/srv/db/common/interfaces.go`). Engines are registered per protocol (`lib/srv/db/common/engines.go`).

#### 3.6.1 PostgreSQL (`lib/srv/db/postgres/engine.go`)
Flow (prose):
1. `InitializeConnection` (`:69-83`): read the startup message forwarded by the proxy. From `StartupMessage` parameters extract `database`, `user`, `application_name` (stored as user agent), keep the rest as startup parameters; or record a `CancelRequest` (`:212-243`).
2. `checkAccess` (`:245-282`): reject empty user (PG would silently default db name = user, which could bypass `db_names`); default db name to user name; validate auto-user naming; build matchers (user + name, protocol-aware); `Checker.CheckAccess(database, accessState, matchers)`; on failure emit `db.session.start` with failure code.
3. Cancel requests are proxied as a special case (no startup, server closes) (`:126-139, 541-550`).
4. Auto-user `Activate` (holds semaphore until connected), then `connect` with pgconn using the connector (client cert or IAM token), `SyncConn` + `Hijack` the raw connection (`:284-313`).
5. `makeClientReady` (`:316-336`): synthesize `AuthenticationOk`, forward `BackendKeyData` (so client cancel works), all `ParameterStatus`, then `ReadyForQuery('I')`. **The client never authenticates to the DB; the agent did.**
6. Emit `db.session.start` (success); start two pumps:
   - client→server (`:339-389`): decode each frontend message, audit `Query`, `Parse`, `Bind` (with formatted parameters), `Execute`, `Close`, `FunctionCall`, `Terminate`; forward bytes unchanged. **No blocking.**
   - server→client (`:464-523`): `io.Copy` server→client through a `TeeReader` into a pipe consumed by a separate parser goroutine that counts messages and emits `db.session.result` on `CommandComplete` (rows affected parsed from the command tag) or `ErrorResponse`. Back-pressure note: the pipe is synchronous, so slow parsing slows the relay.
7. On exit: close upstream first, then emit `db.session.end` (comment explains: audit emission can block, so close DB connection first so an audit outage can't keep sessions open) (`:165-209`), then auto-user `Teardown`.

#### 3.6.2 MySQL (`lib/srv/db/mysql/engine.go`, `protocol/`)
- Own packet parser for COM_* commands. Audited: `COM_QUERY`, `COM_INIT_DB` (also updates the session's current database for subsequent audit records), `CREATE_DB`, `DROP_DB`, `SHUTDOWN`, `PROCESS_KILL`, `DEBUG`, `REFRESH`, prepared-statement prepare/execute/send-long-data/close/reset/fetch/bulk-execute [Observed: `:278-335`; event types `lib/events/api.go:510-553`].
- **`COM_CHANGE_USER` is rejected** by closing the client connection (would let a client re-authenticate as a different DB user mid-session) [Observed: `:281-291`].
- `db_names` is *not* enforced for MySQL because schemas = databases and cross-db queries can't be prevented without query analysis (`lib/srv/db/common/role/role.go:73-85` comment).
- Proxy advertises a server version; agent fetches the real version via a 7-second probe handshake when unknown (`server.go:1405-1434`).

#### 3.6.3 MongoDB (`lib/srv/db/mongodb/engine.go`)
- Connection-level check without the name matcher (`authorizeConnection`, `:199-230`, `DisableDatabaseNameMatcher: true`), then **per-message** authorization: each OP_MSG/OP_QUERY carries `$db`; engine extracts the database, rejects `authenticate`, `saslStart`, `saslContinue`, `logout` outright, and runs `CheckAccess` with `db_users` + `db_names` for that database; every message is audited as `db.session.query` with success/failure (`:231-280`). Legacy `OP_KILL_CURSORS` (no db) is checked with user matcher only.
- Request/response is a strict round trip per client message, with handling for no-reply and multi-reply (exhaust) messages (`:130-166`). Max message size enforced on reads.

#### 3.6.4 Redis (`lib/srv/db/redis/engine.go`, `cmds.go`)
- RESP parsing; the engine *re-issues* commands through a go-redis client rather than raw relaying.
- Intercepts: `HELLO` (unsupported; first HELLO handled specially), unsupported `PUNSUBSCRIBE/SSUBSCRIBE/SUNSUBSCRIBE`, `SUBSCRIBE`/`PSUBSCRIBE` via a dedicated pub-sub connection, and `AUTH`: `AUTH <password>` and `AUTH <user> <password>` are allowed only if the user passes RBAC; the password is masked in audit (`AUTH ****`, `AUTH <user> ****`); for Teleport-managed users the client's AUTH is ignored with a notice (`cmds.go:153-250`).
- Credential providers: client-supplied password, Azure token, Teleport-managed user secret, or AWS IAM auth for ElastiCache/MemoryDB (`engine.go:328-475`).

#### 3.6.5 SQL Server (`lib/srv/db/sqlserver/`)
- TDS parsing: `PreLogin`, `Login7` (extracts username/database), `SQLBatch`, `RPCRequest` (audited as `db.session.sqlserver.rpc_request`), chunked packet reassembly; malformed packets → `db.session.malformed_packet` (`engine.go:75-260`).
- Upstream auth uses Kerberos/Active Directory or Azure AD rather than client certs (`sqlserver/auth.go`) [Observed: file presence; details Inferred from Teleport docs knowledge].
- No auto-user provisioning for SQL Server (`api/types/database.go:331-351` lists only PG self-hosted/RDS/Redshift, MySQL self-hosted/RDS, MongoDB self-hosted).

#### 3.6.6 Others
Cassandra (batch/prepare/execute/register events), Elasticsearch/OpenSearch/DynamoDB (HTTP request events), Snowflake, Spanner (gRPC RPC events), ClickHouse (native + HTTP), Oracle (Enterprise) [Observed: `lib/srv/db/` subdirectories; `lib/events/api.go:557-585`].

#### 3.6.7 Summary table

| Protocol | Proxy terminates? | Upstream auth by agent | `db_names` enforced | Per-command authz | Blocks | Audited |
|---|---|---|---|---|---|---|
| PostgreSQL | TLS + startup | client cert CN / IAM token | Yes (connect time) | No | — | Query, Parse, Bind(+params), Execute, Close, FunctionCall, results |
| MySQL | fake handshake | client cert subject / IAM | No | No | `COM_CHANGE_USER` | COM_* incl. prepared statements, results |
| MongoDB | TLS | X.509 (`$external`) | Yes (**every message**) | Yes (db per message) | auth/sasl/logout | every message (+failure) |
| Redis | TLS | password/ACL/IAM/managed | No | No (AUTH user checked) | unsupported cmds | every command, AUTH masked |
| SQL Server | TLS | Kerberos / Azure AD | No | No | — | SQLBatch, RPC, malformed |

### 3.7 RBAC model for databases

#### 3.7.1 Role fields (v6/v7/v8 roles)
Under `spec.allow` / `spec.deny` (`RoleConditions`, `api/proto/teleport/legacy/types/types.proto:4228-4390`):
- `db_labels` (map label → list of values; values may be `*` or regex), `db_labels_expression` (predicate language).
- `db_names`, `db_users` (lists; `*` wildcard; supports `{{internal.db_names}}`, `{{external.<claim>}}` trait interpolation applied by `ApplyTraits`, `lib/services/role.go:~750-781`).
- `db_roles` (DB-native roles granted to auto-provisioned users).
- `db_permissions` (list of `{permissions: [SELECT,…], match: {object labels}}`) (`types.proto:4380-4384, 4605-4616`).
- `db_service_labels` (+ expression): which *Database Service agents* a user may see/manage (for Discover UI flows).
Under `spec.options`: `create_db_user_mode` (`off|keep|best_effort_drop`, `types.proto:3926-3935`), `require_session_mfa`, `device_trust_mode`, `max_session_ttl`, `lock` (strict|best_effort), `mfa_verification_interval`, `client_idle_timeout`, `disconnect_expired_cert`, `record_session`.

#### 3.7.2 Label matching semantics (`lib/services/role.go:1468-1510`)
- Empty selector ⇒ matches nothing.
- `{"*": ["*"]}` ⇒ matches everything, including resources without labels.
- Otherwise every selector key must exist on the resource; if the selector values include `*` any value is fine; else the resource value must match one of the selector values as exact string or anchored regex.

#### 3.7.3 Label expressions (`lib/services/label/label_expressions.go:64-113`, RFD 0116)
Variables: `labels["k"]`, `user.metadata.name`, `user.spec.traits["t"]`; functions: `set`, `labels_matching`, `contains`, `contains_any`, `contains_all`, `regexp.match`, `regexp.replace`, `email.local`, `strings.upper`, `strings.lower`; boolean `&&`, `||`, `!`, `==`, `!=`. Example (paraphrased from RFD 0116): allow DB access when the user's `teams` trait contains the DB's `team` label. In allow conditions both `db_labels` and `db_labels_expression` must match if both set; in deny conditions either matching denies (`CheckLabelsMatch`, `role.go:3316-3380`).

#### 3.7.4 Access check algorithm (prose of `RoleSet.checkAccess`, `role.go:3030-3270`)
1. If cluster-wide MFA is "always" and not verified: deny (or add an in-band MFA *precondition* when the caller asked for preconditions).
2. **Deny pass:** for each role whose deny namespace matches: if deny labels/expression match the resource ⇒ deny; else if **any** matcher (db user, db name, …) matches the role's deny lists ⇒ deny.
3. **Allow pass:** for each role: namespace and allow labels must match; **all** matchers must match the role's allow lists. If this role matches:
   - If MFA checks can be bypassed and device is trusted ⇒ allow immediately.
   - Else if this role requires session MFA and MFA isn't satisfied ⇒ deny (or precondition).
   - Device trust: verify the role's `device_trust_mode` against device state ⇒ deny if unmet.
   - Mark allowed but keep scanning so a more restrictive matching role can still impose MFA/device requirements.
4. Allowed ⇒ return preconditions (deduplicated, sorted); else deny with a resource-kind hint ("Confirm database user and name.").

#### 3.7.5 Database-specific matchers (`lib/srv/db/common/role/role.go:42-110`; `lib/services/role.go:1440-1464, 2660-2780`)
- `db_users` matcher: case-insensitive for DBs where usernames are case-insensitive; `*` matches the requested user; AWS IAM role-as-user DBs get alternative names (short role name ↔ full ARN) — alternatives never match via wildcard.
- `db_names` matcher: exact or `*`; only added for protocols that can confine sessions (PostgreSQL, MongoDB, Spanner).
- **When auto-user is enabled and supported, the `db_users` matcher is dropped** — the user always connects as their own Teleport username, so `db_users` is irrelevant.
- Enumeration helpers (`EnumerateDatabaseUsers/Names`, `access_checker.go:997-1040`) compute "which users/names can I pick" for UI dropdowns, honoring wildcards and denies (when auto-users is on, only the Teleport username is offered).

### 3.8 Automatic database user provisioning

Sources: `rfd/0113-automatic-database-users.md`, `rfd/0152-automatic-database-users-mongodb.md`, `lib/srv/db/common/autousers.go`, `lib/srv/db/postgres/users.go`, `lib/srv/db/postgres/sql/*.sql`, `lib/srv/db/mysql/autousers.go`, `lib/srv/db/mysql/sql/*.sql`, `lib/srv/db/mongodb/autousers.go`.

**Mode resolution** (`access_checker.go:862-975`, `role.go:4097-4106`):
1. Collect roles whose `create_db_user_mode` is enabled (keep or best_effort_drop).
2. `allowedRoleSet` = those whose **allow** labels match the database; `deniedRoleSet` = those whose **deny** labels match.
3. Mode = maximum enum value across `allowedRoleSet` (`best_effort_drop` > `keep` > `off`).
4. DB roles = ∪ allow `db_roles` of allowedRoleSet − ∪ deny `db_roles` of deniedRoleSet; returned as non-nil (possibly empty) list.
5. If the user requested specific roles (cert `RouteToDatabase.Roles`), each must be in the allowed list, else access denied; otherwise grant all allowed roles (`CheckDatabaseRoles`).

**Preconditions** (`autousers.go:55-80`): database must support auto-users and must have an `admin_user` configured (the agent connects as this admin with its own cert/IAM identity). Postgres additionally rejects RDS reader endpoints (`users.go:56-66`).

**Concurrency:** a cluster-wide semaphore of kind `db-auto-users` named `hex(dbName + "-" + dbUser)` with **MaxLeases 1**, TTL 1 minute, linear retry (1s) bounded by the DB connect timeout. Activation holds the lease **until the upstream connection is established**, then releases; deactivation/delete hold it during the operation (`autousers.go:176-199`, `postgres/engine.go:140-160`).

**PostgreSQL lifecycle** (behavior of the temp-schema procedures; our wording):
- Setup (every activation, "self-healing"): ensure a NOLOGIN bookkeeping role `teleport-auto-user` exists; on PG16+ grant it to the admin `WITH ADMIN OPTION` (PG16 changed CREATEROLE semantics) (`users.go:429-475`); (re)install procedures into `pg_temp` of the admin session (so nothing persistent is left in user schemas).
- Activate(user, roles):
  - If user exists **and is a member of `teleport-auto-user`** (proves Teleport owns it):
    - If the user currently has backend sessions (`pg_stat_activity`): succeed only if the role set equals the requested set (cardinality + containment), else raise custom SQLSTATE `TP002` ("active connections and roles changed").
    - Else: run deactivate (strip all role memberships except bookkeeping) then `ALTER USER … LOGIN`.
  - Else create the user `IN ROLE teleport-auto-user` (fails if a non-Teleport user with that name exists — protects pre-existing human accounts).
  - Grant each requested role.
- Deactivate(user): if active sessions exist, do nothing (notice); else revoke all role memberships except the bookkeeping role and the user itself, then `NOLOGIN`.
- Delete(user): if active sessions exist, do nothing; else `DROP USER IF EXISTS`; on "dependent objects exist" (SQLSTATE `2BP01`) fall back to deactivate and return state `TP004` (vs `TP003` success) so the audit event can say "deactivated instead of deleted".
- Redshift has a separate procedure set; all calls wrapped in retries.

**MySQL/MariaDB specifics** (`mysql/autousers.go`):
- Max username length 32 (MySQL) / 80 (MariaDB); over-long Teleport usernames become `tp-` + base64(SHA-1(username)), and the original name is stored in user attributes (`INFORMATION_SCHEMA.USER_ATTRIBUTES` / a `teleport.user_attributes` table on MariaDB) so the procedure can verify the hashed user belongs to the same Teleport user (`TP001` on mismatch) (`:433-470`, `:546-625`).
- Ownership via role edge from `teleport-auto-user`; deactivate = revoke roles + `ACCOUNT LOCK`; activate = unlock.
- Auth options: self-hosted → require client cert subject `CN=<user>`; RDS → IAM auth plugin (`:446-457`).
- Procedure versioning (`teleport-auto-user-v5`) with "update required" check and transactions.

**MongoDB specifics** (`mongodb/autousers.go`): users created in `$external` (x509 username) with `customData.teleport-auto-user = true`; deactivate = remove roles and set `authenticationRestrictions.clientSource = ["0.0.0.0"]` (can't log in from anywhere); active-session detection via `currentOp`.

**Fine-grained `db_permissions`** (`rfd/0151`, `permissions/calculation.go`, `postgres/users.go:189-272`):
1. Import database objects (tables, …) from the session database; label them using `database_object_import_rule` resources (rules assign labels to objects by matching names/schemas).
2. For each object: union permissions from allow entries whose `match` labels match the object; subtract permissions from matching deny entries; deny `*` clears all.
3. Summarize per permission (counts by object kind) into a `db.session.permissions.update` audit event.
4. Apply via a procedure that revokes old grants and grants the new set; if permissions changed while the user has active sessions → reject (`SQLStatePermissionsChanged`).
5. `db_permissions` and `db_roles` are mutually exclusive for a session.

**Audit:** `db.session.user.create` (TDB08I) and `db.session.user.deactivate` (TDB09I, with a flag whether it was a delete) [Observed: `audit.go:272-322`, `codes.go:175-180`].

### 3.9 Session control: locks, idle timeouts, cert expiry

- Lock targets derived from a TLS identity: each role, user, MFA device used, trusted device ID, join token, bot instance ID, each active access request ID (`lib/services/lock.go:104-144`). Lock targets for the DB session also include the server.
- `ConnectionMonitor.MonitorConn` (`lib/srv/monitor.go:155-300`) starts a monitor goroutine per connection which closes the connection when: client idle timeout elapses (with optional custom message), cert expiry is reached (`disconnect_expired_cert`), a matching lock is **put**, or — in **strict** locking mode — the local lock view becomes unreliable (`OpUnreliable`) (`:485-520`). Best-effort mode keeps sessions alive on unreliable views.
- Session trackers (`server.go:1436-1475`) expose active DB sessions for listing/termination.

### 3.10 Access Requests (JIT)

**Data model** (`types.proto:3456-3470, 3491-…`): `user`, `roles`, `resource_ids`/`resource_access_ids` (resource-based requests), `state` (`NONE, PENDING, APPROVED, DENIED, PROMOTED`), `request_reason`, `resolve_reason`, `thresholds`, `rtm` (role→threshold-sets mapping), `reviews`, `suggested_reviewers`, `login_hint`, `dry_run`, `max_duration`, `session_ttl`, `assume_start_time`, `expiry`, access list promotion fields, request kind (short/long-term).

**Requestability** comes from roles' `allow.request` (`roles`, `search_as_roles` for resource requests, `thresholds`, `claims_to_roles`, `max_duration`, reason mode `required`), and reviewability from `allow.review_requests` (roles + `where` predicate) [Observed: `access_request.go:413-500` contexts; RFD 0059 search-based requests; RFD 0186 required reasons].

**Review application** (prose of `ApplyAccessReview`, `access_request.go:503-571`):
1. Author must match the reviewing user; dedupe/sort role list.
2. Compat checks: proposed state must be resolved (approve/deny/promote); request must have thresholds; **the submitter cannot review their own request**; **one review per author**; role sub-selection and annotations not supported; reviewed roles must be in the request.
3. Compute which thresholds this review counts toward by evaluating each threshold's `filter` predicate against {reviewer roles & traits, review reason/annotations, request roles/reason/system annotations}.
4. Reject if request already approved/denied/promoted (resolved requests are immutable).
5. Append review; optionally set `assume_start_time` (validated against access expiry and creation).
6. Recompute resolution; if any, set state, resolve reason, promoted access list, and set the resource expiry to the access expiry.

**Resolution** (`calculateReviewBasedResolution`, `:696-850`) is given as ALG-ACCESS-T04 below.

**Use of an approved request:** the user re-issues their certificate with the request ID; the new cert includes the requested roles and the active request ID (OID `1.3.9999.2.8`), and its TTL is capped at the request's access expiry (`auth_with_roles.go:4026`). Locking the access request ID terminates sessions using it. Audit codes: `access_request.create` T5000I, `.update` T5001I, `.review` T5002I, `.delete` T5003I, `.search` T5004I, `.expire` T5005I (`codes.go:386-396`). Notifications/approvals are driven by external **plugins** (Slack, PagerDuty, Jira, …) watching request events; access monitoring rules route notifications (RFD 0087; `lib/auth/accessmonitoringrules/`).

### 3.11 Audit events and session recording

**Common metadata** (`events.proto`):
- `Metadata`: `ei` (monotonic index within the session stream), `event` (type), `uid`, `code`, `time`, `cluster_name` (`:33-57`).
- `UserMetadata`: `user`, `login`, `impersonator`, `access_requests`, `trusted_device`, `required_private_key_policy`, `user_kind` (human/bot), `bot_name`, `bot_instance_id`, `user_origin`, `user_roles`, `user_traits` (`:104-160`).
- `SessionMetadata`: `sid`, `with_mfa`, `private_key_policy` (`audit.go:409-416`).
- `ConnectionMetadata`: `remote_addr` (client IP), `local_addr` (DB URI), `protocol: db`.
- `DatabaseMetadata`: `db_service`, `db_protocol`, `db_uri`, `db_name`, `db_user`, `db_labels`, AWS/GCP identifiers, `db_roles`, `db_type`, `db_origin` (`events.proto:3561-3610`).

**Events** [Observed: `lib/events/api.go:463-585`, `codes.go:161-184`]:

| Type | Code | Emitted when | Extra fields |
|---|---|---|---|
| `db.session.start` | TDB00I / TDB00W (failure) | after successful upstream connect, or on authz/connect failure | status (success, error, user_message), `postgres_pid`, client user agent, CA override info |
| `db.session.end` | TDB01I | connection teardown | start/end time, participants |
| `db.session.query` | TDB02I | client query/command | `db_query`, `db_query_parameters`, optional db name override |
| `db.session.query.failed` | TDB02W | query rejected (e.g. Mongo RBAC) | status error |
| `db.session.result` | TDB10I | server completion/error (recording only) | `affected_records`, status |
| `db.session.user.create` | TDB08I | auto-user activation | roles, error |
| `db.session.user.deactivate` | TDB09I | teardown | delete flag |
| `db.session.permissions.update` | — | db_permissions applied | per-permission counts by object kind |
| `db.session.malformed_packet` | TDB06I | unparseable packet | payload |
| protocol-specific | various | e.g. `db.session.postgres.statements.parse/bind/execute/close`, `db.session.postgres.function`, `db.session.mysql.*`, `db.session.sqlserver.rpc_request`, `db.session.mongodb`… | statement name, params |

**Emit vs record** (`audit.go:325-372`): `EmitEvent` = prepare (assign index/time/session ID) → write to session recording → send to global audit log. `RecordEvent` (results) = recording only. Errors are logged, not propagated — **audit failures never block the data path** (deliberate fail-open for availability).

**Session recording** (`lib/events/recorder/recorder.go:105-170`; RFD 0068): mode `off` ⇒ discard; sync modes stream to Auth; async modes write a protobuf event stream to `<data_dir>/log/upload/streaming/<ns>` (optionally encrypted, RFD 0127) and upload in multipart parts. The **upload completer** periodically finds abandoned uploads, completes them and ensures a `session.end` exists, synthesizing one from the recording if the agent crashed (`lib/events/complete.go:151-400`). RFD 0171 converts DB recordings (queries + results) into a playable transcript in `tsh play` and Web UI; note recordings include client-generated queries (GUI schema introspection), not 1:1 with user intent.

**Audit queue** (`lib/events/auditqueue/`): durable SQLite (disk or memory) queue: `Enqueue` returns nil only after durable write; single consumer delivers batches; per-batch delivery timeout; failed events retried up to `MaxAttempts` (default 3) then moved to a dead-letter table that is periodically re-swept and expires after `DeadLetterTTL`; undeserializable payloads quarantined in `corrupt_events`; `MaxBytes` hard limit (`ErrQueueFull`) and `SoftLimit` warning; orphan-queue adoption; drain on shutdown; `synchronous=NORMAL|FULL` pragma choice. The legacy path was an in-memory channel of 1024 events that **silently dropped** events when full (`emitter.go:262-288`). This evolution is a strong signal: Karagöz must have a durable local queue from day one.

**Tamper evidence:** none in the event model (no hash chain/signatures; `ei` is only an ordering index). Encryption of recordings (RFD 0127) addresses confidentiality and tampering *at rest on agent disk* but not end-to-end integrity proofs. Canonical hash-chain design for Hacivat is in doc 05.

### 3.12 MFA and Device Trust

- **Per-session MFA:** cluster `require_session_mfa` or role option; enforced in `checkAccess` (precondition or deny) [Observed: `role.go:3058-3070, 3200-3215`]. Certs carry the MFA-verified device ID (`.1.8`) which is also a lock target.
- **DB local proxy relaxation (RFD 0090):** GUI tools open many connections; instead of 1-minute certs, `tsh proxy db --tunnel` keeps MFA-issued certs **in memory** for the proxy's lifetime (bounded by `max_session_ttl` / `mfa_verification_interval`), re-prompting when expired. Connect's gateways implement a simpler expired-cert check (`lib/teleterm/gateway/db_middleware.go:40-70`).
- **Multi-session MFA (RFD 0202):** one MFA response reusable for a short window for `tsh db exec` across many databases (incl. label-selected, parallel with `--max-connections`).
- **In-band MFA (RFD 0234):** MFA as an access-check *precondition* satisfied within the connection handshake rather than by re-issuing certs (the `ReturnPreconditions` path in `checkAccess`).
- **Device Trust:** enrollment ceremony registers a device credential bound to hardware (macOS Secure Enclave, TPM on Windows/Linux; `lib/devicetrust/native/`); authentication ceremony (Init → Challenge → certs augmented with device extensions, or a confirmation token for Web) (`lib/devicetrust/authn/authn.go:37-200`); modes `off`, `optional`, `required`, `required-for-humans` (bots exempt) (`authz.go:136-185`); unknown modes fail closed.
- **Gating detail** (`lib/devicetrust/config/config.go`): effective mode for OSS builds is `off`, but *enforcement* uses the configured mode "disregarding the provenance of the binary", guaranteeing that OSS agents paired with Enterprise Auth still enforce. Config validation rejects non-off modes on OSS Auth.

### 3.13 Client UX: `tsh`, Teleport Connect, VNet, MCP

- **`tsh db login/connect/proxy/exec/ls/config`** (`tool/tsh/common/db*.go`): `tsh db login <db> --db-user --db-name [--db-roles]` reissues a cert with routing; `tsh db connect` spawns the native CLI (psql/mysql/mongosh/…) through a local proxy when needed; `tsh proxy db --tunnel` creates an authenticated local listener for GUI tools (DataGrip etc.); `tsh db exec` runs one query across many DBs.
- **Teleport Connect** = Electron UI + `tsh daemon` (Go gRPC "tshd") (`lib/teleterm/README.md`). Concept "**gateway**" = per-DB local ALPN proxy on localhost; a Gateway document tab shows host/port, a db-user/db-name chooser (populated from RBAC enumeration), the equivalent CLI command, and a "connect with CLI" option (`web/packages/teleterm/src/ui/DocumentGateway/`). Supports multiple clusters, resource search with labels, access request creation/review UI, and **Connect My Computer** (RFD 0133): one-click: creates a role and join token, downloads `teleport`, configures and runs a node agent from within the app with status/logs, start/stop/remove controls.
- **VNet** (`lib/srv/db/vnet/`, `lib/teleterm/vnet/`): a virtual network interface + DNS so that DB hostnames resolve to local addresses handled by the Teleport client, removing the need for per-DB local ports [Observed: directory presence; details Inferred].
- **Database MCP server** (`tool/tsh/common/mcp_db.go`, `lib/client/db/mcp/`): `tsh mcp db start` exposes a `list databases` tool and per-protocol `query` tools (PostgreSQL/CockroachDB, …) over a local proxy, using the user's cert. Query execution just runs the SQL and returns rows; **no read-only enforcement at the MCP layer** — safety relies on DB-side privileges and Teleport RBAC [Observed: `lib/client/db/postgres/mcp/mcp.go:37-200`]. AI-agent actions appear in audit as the human user (no separate agent attribution field, [Inferred] from `UserMetadata` fields).

### 3.14 Feature gating in code

- `lib/modules/modules.go`: a `Modules` interface supplies `BuildType` (OSS vs Enterprise), `Features()` with an `Entitlements` map (`GetEntitlement(kind)` returns enabled/limit; `UnderLimit(count)`), and legacy fallbacks; Enterprise code (`e/`) registers a different module implementation at init [Observed: `modules.go:63-205`; `e/` private]. Feature checks are scattered at use sites (`IsOSSBuild()`, entitlement lookups), e.g. Device Trust's effective mode. This means the **OSS agent binary contains the enforcement code paths** and Enterprise is mostly control-plane features + some enterprise-only engines (Oracle).

---

## 4. Algorithm specifications (Teleport-derived)

These specs describe Teleport's observed behavior in our own words, adapted where noted. Canonical Hacivat algorithms are in `04-zero-credential-discovery-and-ephemeral-users.md` (discovery, credential resolution, ephemeral users) and `05-authorization-and-audit-primitives.md` (token verification, query authorization, audit chaining, approval state machine, masking). Where a T-spec informs a canonical spec, it is noted under "Used by".

### ALG-ACCESS-T01: Role-set access decision for a database connection

- **Problem:** Decide whether identity *I* (roles R, traits T, MFA state, device state) may connect to database *D* (labels L, protocol P) as DB user *u* to DB name *n*.
- **Inputs / outputs:** Inputs: role set R (each with allow/deny conditions and options), user name, traits, D, u, n, access state {mfaRequiredCluster ∈ {never, always, perRole}, mfaVerified, deviceVerificationEnabled, deviceVerified, isBot, returnPreconditions}. Output: `Allow(preconditions[])` | `Deny(reason)`.
- **Algorithm:**
  1. `matchers ← []`. If not (D supports auto-users and auto-user mode enabled for I on D): append `UserMatcher(u, caseInsensitive = D.caseInsensitiveUsers, altNames = cloudAltNames(D,u))`. If `P ∈ {postgres, mongodb, spanner}`: append `NameMatcher(n)`.
  2. Postgres-specific pre-step: if `u == ""` ⇒ Deny("user name must not be empty"); if `n == ""` ⇒ `n ← u`.
  3. `pre ← ∅`. If `mfaRequiredCluster == always ∧ ¬mfaVerified`: if `returnPreconditions` then `pre ← pre ∪ {MFA}` else Deny(MFA required).
  4. For each role r in R (deny pass): if r.deny.namespaces doesn't match D's namespace, skip. If `labelsMatchDeny(r.deny, D, I)` ⇒ Deny. If `∃ m ∈ matchers: m.matches(r.deny)` ⇒ Deny.
  5. `bypassMFA ← mfaRequiredCluster == never ∨ mfaVerified` (simplified); `trusted ← ¬deviceVerificationEnabled ∨ deviceVerified`; `allowed ← false`.
  6. For each role r (allow pass): require namespace match; require `labelsMatchAllow(r.allow, D, I)`; require `∀ m ∈ matchers: m.matches(r.allow)`; else continue.
     - If `bypassMFA ∧ trusted` ⇒ return Allow(sortedDedup(pre)).
     - If `¬bypassMFA ∧ r.options.requireSessionMFA`: if returnPreconditions ⇒ `pre ∪= {MFA}` else Deny(MFA).
     - If `verifyDeviceMode(r.options.deviceTrustMode, trusted, isBot)` fails ⇒ Deny(trusted device required).
     - `allowed ← true` (continue to find stricter roles).
  7. Return `allowed ? Allow(sortedDedup(pre)) : Deny("Confirm database user and name.")`.
  - `labelsMatchAllow(cond)`: if neither labels nor expression set ⇒ false; result = (labels unset ∨ selectorMatch) ∧ (expr unset ∨ exprTrue).
  - `labelsMatchDeny(cond)`: if neither set ⇒ false; result = (labels set ∧ selectorMatch) ∨ (expr set ∧ exprTrue).
  - `UserMatcher.matches(list)`: any entry equals u (case-fold if configured) or entry is `*`; or any altName equals an entry exactly (wildcard does not apply to alt names).
  - `NameMatcher.matches(list)`: any entry equals n or `*`.
- **Data structures:** roles pre-expanded with traits (`{{internal.x}}`, `{{external.y}}` substituted, deduplicated); compiled label expressions cached by string.
- **Complexity:** O(|R| × (|labels| + |matchers| × |list|)) per check; regex compile is the hot spot — cache compiled patterns.
- **Edge cases:**
  - Empty selector in allow ⇒ never matches; `{"*":["*"]}` matches unlabeled DBs.
  - Deny with only `db_users: ["*"]` and no labels ⇒ deny labels unset ⇒ label stage false, but matcher stage hits ⇒ **denies everything** (greedy deny is intended).
  - Deny role whose namespace doesn't match is ignored entirely.
  - Two allow roles, one requiring MFA, one not, both matching: first non-MFA match with bypass returns allow early only when MFA is bypassable; otherwise the stricter one wins.
  - Unknown device trust mode ⇒ deny (fail closed).
  - Case-insensitive DB (e.g. some cloud DBs): `Admin` vs `admin` match.
  - MySQL: `db_names` ignored, so a role allowing only `db_names: [app]` still lets the user `USE other` — document clearly in UI.
  - Auto-user enabled: `db_users` lists are ignored.
  - Postgres empty db name defaults to user name **before** matching.
  - Trait expands to empty list ⇒ the field contributes no entries ⇒ allow fails for that role.
- **Test cases:**
  1. R={allow{db_labels env:dev, db_users:[alice], db_names:[*]}}, D{env:dev,postgres}, u=alice,n=app ⇒ Allow.
  2. Same, u=bob ⇒ Deny.
  3. Add deny{db_labels env:dev, db_names:[secret]}; n=secret ⇒ Deny; n=app ⇒ Allow.
  4. R={allow{db_labels *:*, db_users:[*]}}, D mysql, n=anything ⇒ Allow (no name matcher).
  5. R={allow{db_labels env:prod, db_users:[ro], options.require_session_mfa}}, mfaVerified=false, returnPreconditions=false ⇒ Deny(MFA); with returnPreconditions ⇒ Allow([MFA]).
  6. allow expression `contains(user.spec.traits["teams"], labels["team"])`, traits teams=[payments], D team=payments ⇒ Allow; team=risk ⇒ Deny.
  7. allow with both db_labels env:dev and expression false ⇒ Deny; deny with labels non-match but expression true ⇒ Deny.
  8. Postgres u="" ⇒ Deny(empty user).
- **Prior art and evidence:** `lib/services/role.go:1440-1510, 2660-2780, 3030-3380`; `lib/srv/db/common/role/role.go:42-110`; `lib/srv/db/postgres/engine.go:245-282`.
- **Used by:** Karagöz (local policy evaluation for P2P sessions — adapt into the canonical permission-evaluation spec in doc 05); Perde (UI "why can't I connect" explanations; enumeration of allowed users/names).

### ALG-ACCESS-T02: Label selector and expression evaluation

- **Problem:** Match resource labels against a role's selector map and optional predicate expression.
- **Inputs / outputs:** selector `map<string, list<string>>`, expression string, resource labels `map<string,string>`, user name, traits → bool.
- **Algorithm:**
  1. Selector: if empty ⇒ false. If selector has key `*` with exactly value list [`*`] ⇒ true. For each (k, vals): if k absent in labels ⇒ false; if `*` ∈ vals ⇒ continue; if no v ∈ vals with `fullMatch(v, labels[k])` (literal or anchored regex `^…$`) ⇒ false. Return true.
  2. Expression: parse once (cache AST); evaluate with environment {labels getter, user.metadata.name, user.spec.traits}; supported ops: `==`, `!=`, `&&`, `||`, `!`, functions listed in §3.7.3. Type errors ⇒ evaluation error ⇒ treated as error (role check returns error ⇒ deny).
- **Data structures:** compiled regex cache keyed by pattern; AST cache keyed by expression.
- **Complexity:** O(|selector|) + expression size; regex compile amortized.
- **Edge cases:** label key present with empty value; regex metacharacters in literal values (Teleport treats values as regex if they look like `^…$` — [Inferred]; Hacivat should require explicit `re:` prefix to avoid surprises); missing label in expression `labels["x"]` returns empty string; trait missing ⇒ empty list; case sensitivity of keys (sensitive).
- **Test cases:** `{}` vs any ⇒ false; `{"*":["*"]}` vs `{}` ⇒ true; `{"env":["prod","stage"]}` vs `{env:stage}` ⇒ true; `{"env":["^st.*$"]}` vs `{env:staging}` ⇒ true; `{"env":["*"]}` vs `{team:x}` ⇒ false (key absent); expression `labels["env"] != "prod"` vs `{}` ⇒ true.
- **Prior art and evidence:** `lib/services/role.go:1468-1510`, `lib/services/label/label_expressions.go:64-113`, `rfd/0116-label-expressions.md`.
- **Used by:** Perde (role editor, label selectors over agent-discovered DBs), Karagöz (offline evaluation).

### ALG-ACCESS-T03: Per-session automatic DB user lifecycle (Teleport model)

- **Problem:** Give each human a DB account named after their identity, with exactly the DB roles their policy grants, only for the duration of their sessions, safe under concurrency and agent crashes.
- **Inputs / outputs:** identity username U, database D (admin credentials available to the agent), role set → mode M ∈ {off, keep, best_effort_drop}, DB role list G. Output: an enabled DB login named f(U) holding exactly G during the session; after the session: disabled (keep) or dropped (best_effort_drop; fallback disabled).
- **Algorithm:**
  1. Resolve M and G (T01 sibling): M = max over allow-matching auto-create roles; G = ∪allow db_roles − ∪deny db_roles; if client requested subset S: require S ⊆ G and set G ← S.
  2. If M = off ⇒ return (normal login with requested user).
  3. Require D supports auto-users and admin configured; else fail with actionable message.
  4. `name ← f(U)` where f = identity if length ≤ DBMS limit else `prefix + base64(sha1(U))`; store U in DB-side metadata when hashed.
  5. Acquire distributed lease `(kind="db-auto-users", key=hex(D.name+"-"+name), max=1, ttl=60s)`, retrying every 1s until connect timeout.
  6. As admin: ensure bookkeeping role `B` exists (and admin can grant it); install/refresh helper procedures in a session-temporary namespace.
  7. `activate(name, G)`:
     - If `name` exists and is member of B (and, if hashed, metadata U matches; else error `TP001`):
       - If `name` has active backend sessions: if current roles (excluding B) == G ⇒ OK (reuse) else error `TP002`.
       - Else revoke all roles except B, re-enable login/unlock.
     - Else if `name` exists but not in B ⇒ error (refuse to hijack a non-managed account).
     - Else create `name` with membership in B (MySQL: pinned to cert subject / IAM plugin; Mongo: `$external` user tagged `customData`).
     - Grant each g ∈ G (or apply object permissions if `db_permissions`, mutually exclusive with G).
  8. Connect upstream as `name`; **release the lease after connect** (not at session end).
  9. On session end: acquire lease again; if M = keep ⇒ `deactivate(name)`; if M = best_effort_drop ⇒ `delete(name)`.
     - `deactivate`: if active backend sessions ⇒ no-op; else revoke all roles except B and disable login (PG `NOLOGIN`, MySQL `ACCOUNT LOCK`, Mongo `clientSource 0.0.0.0` + no roles).
     - `delete`: if active sessions ⇒ no-op; else drop; on dependency error ⇒ deactivate and report "kept".
  10. Emit audit `user.create` after step 7 and `user.deactivate{deleted?}` after step 9.
- **Data structures:** lease record {kind, name, holder, expiry}; per-DBMS procedure set with version tag for idempotent installation.
- **Complexity:** a few admin round trips per session start/end; lease contention serializes parallel connects of the same user to the same DB (1s retry granularity).
- **Edge cases:**
  - Agent crash between activate and teardown ⇒ user remains enabled with roles until next activation of the same user (which strips stale roles) — **Teleport has no periodic sweeper** [Inferred: no sweeper found in `lib/srv/db`]; Hacivat must add one (see doc 04).
  - Concurrent sessions of same user: second activation with same G reuses; with different G fails (`TP002`) — user must close old sessions.
  - Pre-existing human account with the same name ⇒ creation fails; never adopt.
  - PG16 role-grant semantics change ⇒ admin needs ADMIN OPTION on B.
  - Dependent objects (user created tables) block DROP ⇒ fallback deactivate (objects owned by an inactive user remain).
  - RDS reader endpoint ⇒ refuse (read replica can't create users).
  - Username length/charset limits ⇒ hashing; collision probability negligible but metadata check protects.
  - Lease TTL (60s) shorter than a slow activation ⇒ potential parallel activation; activation is idempotent enough to tolerate.
  - Roles in G that don't exist in DB ⇒ grant fails ⇒ session fails with clear error.
  - Deny `db_roles` removing everything ⇒ user created with no roles (DB default privileges apply — e.g. PG `PUBLIC` grants!).
- **Test cases:**
  1. New user, G=[reader] ⇒ user created in B with reader; after end (keep) ⇒ NOLOGIN, no roles.
  2. Existing B-member disabled with stale role `writer`, G=[reader] ⇒ after activate holds only reader.
  3. Second concurrent session same G ⇒ succeeds without changes; different G ⇒ TP002.
  4. Name exists outside B ⇒ activation error.
  5. best_effort_drop with user-owned table ⇒ result "deactivated", event delete=false/kept.
  6. MySQL username 40 chars ⇒ `tp-…` name, metadata holds original; another Teleport user hashing to same name ⇒ TP001.
  7. Requested roles S ⊄ G ⇒ access denied before any DB call.
- **Prior art and evidence:** `lib/services/access_checker.go:862-975`; `lib/services/role.go:4097-4106`; `lib/srv/db/common/autousers.go:55-199`; `lib/srv/db/postgres/users.go:56-475`; `lib/srv/db/postgres/sql/{activate,deactivate,delete}-user.sql` (behavior only); `lib/srv/db/mysql/autousers.go:117-625`; `lib/srv/db/mongodb/autousers.go:40-312`; RFD 0113, 0152.
- **Used by:** Karagöz (basis for canonical ephemeral-user spec in doc 04, which adds a sweeper, per-session random names option, and crash journal).

### ALG-ACCESS-T04: Access request review resolution

- **Problem:** Given thresholds, role→threshold-set mapping and the ordered list of reviews, decide whether a request becomes APPROVED, DENIED, PROMOTED or stays PENDING.
- **Inputs / outputs:** thresholds `[ {name, filter, approve:uint, deny:uint} ]`, rtm `role → [ set of threshold indexes ]`, reviews ordered by time `[ {author, proposedState, thresholdIndexes, reason} ]` → `(state, reason)` or none.
- **Algorithm:**
  1. If no thresholds or no reviews ⇒ none.
  2. `counts[i] ← (0,0)`; `approved ← ∅`; `denied ← false`; `last ← nil`.
  3. For each review rv in order: `last ← rv`.
     a. For each index i in rv.thresholdIndexes: increment approval or denial count by proposed state; if proposed = PROMOTED ⇒ stop processing reviews.
     b. If ∃ i: `thresholds[i].deny ≠ 0 ∧ counts[i].denial ≥ thresholds[i].deny` ⇒ `denied ← true`; stop.
     c. For each role in rtm not yet approved: for each set in rtm[role]: if no index j in set has `thresholds[j].approve ≠ 0 ∧ counts[j].approval ≥ thresholds[j].approve` ⇒ continue with next review (step 3). If all sets satisfied ⇒ `approved ∪= {role}`.
     d. If loop reaches here (all roles approved) ⇒ stop.
  4. If last.proposed = APPROVED: return APPROVED only if `|approved| = |rtm|`, else none. If DENIED: return DENIED only if `denied`. If PROMOTED: return PROMOTED. Reason = last.reason.
  - Threshold indexes for a review are computed at submission by evaluating each threshold's filter against {reviewer roles/traits, review reason, request roles/reason/system annotations}; empty filter matches.
  - Preconditions at submission: proposed ∈ {APPROVED, DENIED, PROMOTED}; reviewer ≠ requester (and plugin submitter ≠ requester); no prior review by same author; request not already resolved.
- **Data structures:** request record with append-only reviews; `rtm` precomputed at request creation from requester's roles' `request.thresholds`.
- **Complexity:** O(|reviews| × (|idx| + |thresholds| + |rtm|·|sets|·|idx|)); tiny in practice.
- **Edge cases:** threshold with approve=0 never satisfies approvals (deny-only); deny=0 never denies; reviewer matching zero thresholds (filter false) still recorded but counts nothing; mixed approvals across roles; promotion short-circuit; review arriving after expiry (should be rejected by expiry check elsewhere); assume_start_time later than expiry ⇒ invalid.
- **Test cases:**
  1. One threshold approve=1 deny=1; review APPROVED ⇒ APPROVED.
  2. Thresholds A(approve=2, filter reviewer has role "lead"), B(approve=1, filter reviewer has role "security"); rtm role X ⇒ sets [{A},{B}]; reviews: lead APPROVED ⇒ none; security APPROVED ⇒ none; second lead APPROVED ⇒ APPROVED.
  3. Same, one security DENIED with B.deny=1 ⇒ DENIED immediately.
  4. Requester reviews own request ⇒ rejected.
  5. Same author twice ⇒ rejected (AlreadyExists).
  6. Review on APPROVED request ⇒ rejected.
- **Prior art and evidence:** `lib/services/access_request.go:503-850`.
- **Used by:** Perde (canonical approval state machine in doc 05 extends this with expiry, cancellation, break-glass and agent push).

### ALG-ACCESS-T05: Resource reconciliation (registered vs observed)

- **Problem:** Keep a registry of resources in sync with a freshly observed set without clobbering resources owned by another origin.
- **Inputs / outputs:** `current: map<key, R>`, `observed: map<key, R>`, `matcher(R)→bool`, `compare(a,b)→Equal|Different`, `allowOriginChanges` → create/update/delete calls + aggregated errors.
- **Algorithm:**
  1. For each (k, c) in current: if k ∉ observed ⇒ `onDelete(c)` (ignore NotFound).
  2. For each (k, o) in observed:
     - If k ∉ current: if matcher(o) ⇒ `onCreate(o)`; else skip.
     - Else if ¬allowOriginChanges ∧ origin(o) ≠ origin(current[k]) ⇒ warn and skip.
     - Else if compare(o, current[k]) ≠ Equal: if matcher(o) ⇒ `onUpdate(o, current[k])` else `onDelete(current[k])`.
  3. Collect errors; continue processing other keys; report aggregate.
  4. Optional bounded parallelism with a mutex around the error list.
- **Data structures:** maps keyed by stable resource name; origin label on each resource (`config`, `dynamic`, `cloud`, …).
- **Complexity:** O(|current| + |observed|) callbacks.
- **Edge cases:** observation failure must **not** be treated as "everything disappeared" (Teleport's discovery passes only successful fetch results; on reconcile error it counts all as failed) — Hacivat must distinguish "list failed" from "list empty"; name collisions between origins; flapping resources (add hysteresis/grace in Hacivat); delete of already-deleted resource.
- **Test cases:** current {a}, observed {a', b} with a'≠a ⇒ update a, create b; current {a(config)}, observed {a(cloud)} ⇒ skip; observed {b} not matching ⇒ no create; current {a}, observed {} ⇒ delete a.
- **Prior art and evidence:** `lib/services/reconciler.go:296-470`; `lib/srv/discovery/database_watcher.go:40-139`; `lib/srv/db/watcher.go:40-237`.
- **Used by:** Karagöz (Docker reconcile loop; canonical spec doc 04), Perde (agent inventory sync).

### ALG-ACCESS-T06: Live-session control monitor (locks, idle, expiry)

- **Problem:** Terminate an established database session as soon as it becomes unauthorized, even mid-session.
- **Inputs / outputs:** connection, lock targets of the session, client idle timeout, cert expiry, locking mode (strict|best_effort), lock event stream → closes connection with reason; emits audit.
- **Algorithm:**
  1. At start: if any lock currently in force for targets ⇒ refuse. Subscribe to lock watcher filtered by targets.
  2. Wrap conn in a tracking reader recording last-read time.
  3. Loop select:
     - idle timer fires: if `now − lastActive ≥ idleTimeout` ⇒ close("idle timeout") else reschedule for remainder.
     - expiry timer fires ⇒ close("certificate expired").
     - lock event Put ⇒ close("lock in force: <message>"); Delete ⇒ ignore; Unreliable ⇒ if strict close("lock view unreliable") else continue.
     - lock stream closed: if no timers configured ⇒ exit monitor; else stop listening to locks.
     - context done ⇒ exit.
- **Data structures:** per-connection monitor; shared lock cache with targets index.
- **Complexity:** O(1) per event; lock fan-out by targets.
- **Edge cases:** lock created before monitor subscribed (covered by step 1 check); clock skew between control plane and agent for expiry (use agent clock + margin); reconnect storms after lock deletion; in P2P with Perde unreachable, "unreliable" is the norm ⇒ need explicit offline policy (doc 05).
- **Test cases:** put lock on user ⇒ session closed within event latency; idle 30s with timeout 20s ⇒ closed; strict mode + watcher disconnect ⇒ closed; best_effort ⇒ kept.
- **Prior art and evidence:** `lib/srv/monitor.go:155-520`; `lib/services/lock.go:47-144`; `lib/srv/db/server.go:1228-1233`.
- **Used by:** Karagöz (revocation push), Perde (lock API).

### ALG-ACCESS-T07: Object-level permission calculation from labels

- **Problem:** Compute DB-native grants per object from label-based allow/deny permission rules.
- **Inputs / outputs:** objects `[ {kind, schema, name, labels} ]`, allow rules `[ {perms[], match} ]`, deny rules → `map<PERM, [objects]>`.
- **Algorithm:** for each object: `P ← {}`; for each allow rule whose `match` labels (T02 selector semantics) match object labels: add `upper(trim(perm))` for each perm; for each deny rule matching (stop early if P empty): if `*` ∈ perms ⇒ P ← {} else remove each upper(perm); for each p in P: out[p].append(object).
- **Data structures:** imported object list labeled by import rules (name/schema globs → labels).
- **Complexity:** O(|objects| × |rules|).
- **Edge cases:** permission case and whitespace normalization; unsupported permission for object kind (validated before apply); objects created after session start (not covered until next activation); views/functions/sequences kinds; huge schemas (summaries in audit instead of full lists).
- **Test cases:** allow SELECT on `{sensitivity:public}`, deny `*` on `{pii:true}`; object {public, pii} ⇒ none; object {public} ⇒ SELECT.
- **Prior art and evidence:** `lib/srv/db/common/permissions/calculation.go`; `lib/srv/db/postgres/users.go:137-272`; RFD 0151.
- **Used by:** Karagöz (ephemeral-user privilege mapping; doc 04), Perde (policy editor preview).

### ALG-ACCESS-T08: Durable local audit queue with dead-lettering (Teleport model)

- **Problem:** Never lose audit events when the control plane is slow/unreachable, while bounding disk use.
- **Inputs / outputs:** events from session handlers; delivery handler that forwards a batch and returns delivered items → persisted queue, delivery, stats.
- **Algorithm:**
  1. `enqueue(e)`: if db size ≥ maxBytes ⇒ return QueueFull (caller decides fail-open/closed); insert row (payload, enqueued_at, attempts=0) in a WAL SQLite DB; return success only after commit (synchronous NORMAL or FULL).
  2. Single consumer loop: select oldest N rows; call handler with timeout; delete delivered rows; increment attempts of undelivered; move rows with attempts ≥ maxAttempts to dead-letter table with failed_at.
  3. Rows whose payload fails to deserialize ⇒ move to corrupt table.
  4. Periodic dead-letter sweep re-attempts delivery; purge dead letters older than TTL.
  5. Periodic orphan scan adopts queues left by previous process instances (crash recovery).
  6. On shutdown: drain best-effort until main+dead-letter empty or deadline.
  7. Stats: pending count, dead-letter count, corrupt count, oldest pending/dead-letter time; warn above soft limit.
- **Data structures:** tables `audit_queue(id, payload, enqueued_at, attempts)`, `audit_dead_letter(…, failed_at)`, `corrupt_events`, `teleport_info`.
- **Complexity:** amortized O(1) per event; batch delivery.
- **Edge cases:** purging dead letters = **data loss** by design (Hacivat should never purge unacknowledged events without an explicit, audited operator action); power loss with NORMAL sync may lose last commits; out-of-order delivery after retries (receiver must sort by event index/time); duplicates after crash between deliver and delete (receiver must be idempotent by event UID).
- **Test cases:** control plane down for 1h ⇒ queue grows, no loss; restart ⇒ resumes; handler accepts half of batch ⇒ the other half retried; malformed payload ⇒ corrupt table, others continue; disk full ⇒ QueueFull surfaced.
- **Prior art and evidence:** `lib/events/auditqueue/auditqueue.go:30-165`, `sqlite.go:80-110, 580-765`; `lib/events/emitter.go:262-288`.
- **Used by:** Karagöz (canonical audit batching/hash-chaining spec in doc 05 builds on this and adds a hash chain + gap detection).

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 What to copy

| Teleport pattern | Why it fits | Hacivat-family adaptation |
|---|---|---|
| Agent is the PEP; proxy only routes | Our data path is P2P; Perde never sees traffic | Karagöz performs all authz + audit; Perde only issues credentials/policy and rendezvous. |
| Short-lived, identity-bearing client credentials carrying routing + role names | Offline verification at the agent | Use short-lived capability tokens/certs signed by Perde's org CA, carrying user, roles/policy version, target DB, db user/name, MFA/device claims, request IDs (design in doc 05). Keep TTLs minutes–hours. |
| Agent → DB without user-held passwords (client-cert CN or IAM token; admin creds only on agent) | Matches "no credentials typed by user" | Karagöz resolves admin creds locally (Docker env, secrets, peer auth — doc 04) and never ships them to Hacivat. |
| Synthesized auth handshake (`AuthenticationOk` + forwarded `BackendKeyData` + parameter status) | Lets standard clients connect without passwords | Hacivat talks to Karagöz over our own authenticated channel; Karagöz terminates the PG/MySQL handshake itself and replays startup to the DB. Forward cancel keys so query cancel works. |
| Protocol-aware matchers (`db_names` only where enforceable) | Honest security model | Perde UI must show "database-level restriction not enforceable on MySQL without query analysis" or enforce it via SQL parsing in Karagöz. |
| Reject `COM_CHANGE_USER`, Mongo `saslStart/authenticate/logout`, mask Redis `AUTH` | Prevents identity switching mid-session | Same blocklist in Karagöz engines; also block PG `SET ROLE`/`SET SESSION AUTHORIZATION` when not permitted (Teleport doesn't). |
| Auto-user bookkeeping role + "refuse to adopt unmanaged accounts" + strip-before-grant | Safe, self-healing | Adopt; add a periodic sweeper and a local crash journal (doc 04). |
| Distributed lease around activation | Prevents grant races | In Karagöz a **local** mutex per (db,user) suffices (single agent per DB); if multiple agents serve one DB, coordinate via Perde lease or DB advisory lock (`pg_advisory_lock`). |
| Close upstream before emitting session end | Audit outage must not extend sessions | Copy. |
| Durable SQLite audit queue with stats | P2P ⇒ audit lives on the agent first | Copy and extend with hash chain, signed checkpoints, gap detection (doc 05); never TTL-purge undelivered events silently. |
| Locks with targets incl. access request & device | Fast revocation | Perde pushes lock/revocation lists to agents over the control channel; agents also enforce via short TTL. Offer strict mode: if control channel is down longer than X, stop new sessions. |
| Bound-keypair join with join-state verification and recovery counters | One-command install, clone detection | Karagöz install: `curl …/install.sh | sh -s -- --token <one-time>` → generates key on host → registers public key with one-time secret → subsequent joins by signed challenge; Perde detects cloned agents via join-state mismatch and locks the agent. |
| Access request thresholds with filters | Expressive multi-party approvals | Perde implements a simplified version (role-or-group-based thresholds) and records request ID inside issued credentials. |
| Separate audit for result metadata (rows affected) | Useful for forensics | Record rows affected/returned, duration (Teleport lacks duration on query events). |
| Enforcement code present in OSS agent regardless of edition | Paid control plane + OSS agent must still enforce | Karagöz (OSS) must enforce every policy primitive Perde can express, including those configured via paid features. |

### 5.2 What to avoid / improve

1. **No statement-level control:** Teleport relies on DB privileges. Hacivat's AI-agent use case needs read-only enforcement *and* SQL classification at Karagöz, with server-side defense in depth (`SET TRANSACTION READ ONLY`/`default_transaction_read_only`, read-only DB roles). See doc 05 query-authorization spec.
2. **Audit fail-open with silent drops** (legacy path) — Karagöz must fail **closed** for new statements when the durable queue is full (configurable), and surface health in Hacivat.
3. **No tamper evidence** — add hash chains + signed checkpoints uploaded to Perde.
4. **No AI-agent attribution** — Teleport's MCP server runs as the user; Hacivat should mark `actor.kind = agent`, `agent.client = <MCP client name>`, `agent.session`, and optionally require a separate, narrower policy for agent-originated queries.
5. **Auto-user crash leftovers** — add a sweeper (on agent start and periodically) that deactivates managed users with no live sessions and no active leases.
6. **Relicensing trauma** — do not put the agent under a license that can later restrict production use; keep Karagöz under a permissive or stable copyleft license from day one (doc 06).
7. **SSO tax** — Teleport Community has only GitHub SSO; Perde should include OIDC SSO in the self-hosted OSS edition to avoid the backlash documented in doc 06.
8. **Install script without checksum verification** — Karagöz installer must verify signatures/checksums (minisign/cosign) and pin the Perde endpoint's key.
9. **Per-connection cert minting** (Teleport TODO) — Karagöz should cache per-(user,db) upstream credentials for the session lifetime.
10. **GUI clients and MFA friction** — design like RFD 0090/0202 from the start: MFA unlocks a time-boxed "session grant" held in Hacivat memory, not per-connection prompts.
11. **MySQL `db_names` gap** — if Perde exposes per-database permissions for MySQL, enforce via ephemeral users' grants (`GRANT … ON db.*`) rather than connection-time checks.

### 5.3 Concrete design recommendations

- **Perde role schema (draft, inspired by Teleport but simplified):**
  - `allow/deny.db_selector` (labels map) + `db_selector_expr` (Cedar/CEL-like expression; see doc 05), `db_names`, `db_users` (only when not using ephemeral users), `db_roles` (DB-native), `db_object_permissions` (label-matched), `access_mode ∈ {read_only, read_write, admin}`, `agent_access ∈ {deny, read_only, same_as_user}` (AI agents), options `{max_session_ttl, require_mfa, require_trusted_device, idle_timeout, ephemeral_user_mode ∈ {off, keep, drop}, record_results ∈ {none, metadata, sampled, full}}`.
  - Deny-overrides; all allow matchers must match; most-restrictive options across matching allow roles.
- **Credential claims** (doc 05 for format): `sub`, `org`, `roles`, `policy_epoch`, `target{agent_id, db_id, db_user, db_name}`, `access_mode`, `mfa{verified_at, device}`, `device{id, trust}`, `request_ids[]`, `exp` ≤ min(role TTL, request expiry), `cnf` (Hacivat's ephemeral public key for proof-of-possession).
- **Agent policy cache:** Karagöz keeps a signed policy snapshot (`policy_epoch`) from Perde; tokens reference the epoch; agent refuses tokens newer than its snapshot until it syncs (or fetches), and refuses snapshots older than `max_policy_staleness` in strict mode.
- **Event names:** adopt a Teleport-like namespace for familiarity: `db.session.start`, `db.session.end`, `db.query`, `db.query.denied`, `db.result`, `db.user.provision`, `db.user.deprovision`, `access.request.*`, `agent.join`, `agent.lock` — with stable numeric codes and i18n message keys (§6).

---

## 6. i18n considerations

1. Teleport's errors are English strings concatenated with dynamic values ("access to %v denied. User does not have permissions. %v") [Observed: `role.go:3104-3110`]. Hacivat/Karagöz must return **stable error codes + structured parameters** (e.g. `ACCESS_DENIED_DB_USER{db, user}`), and localize in Hacivat/Perde, never at the agent.
2. Audit events must store machine-readable `code` and parameters, not rendered sentences; Perde renders per viewer locale (EN/TR). Teleport's event `code` (`TDB00I`) is a good precedent — keep codes stable forever.
3. Custom DB error states (Teleport uses SQLSTATE `TP001..TP004`) must map to localized messages client-side; reserve a Hacivat SQLSTATE class (e.g. `HV0xx`) and document codes in the message catalog.
4. Approval workflows: request/review reasons are free text from users — store raw, display as-is; do not machine-translate audit content.
5. Turkish-specific: case-insensitive username matching must use **locale-independent** case folding (Unicode simple case folding / ASCII lowering for DB identifiers). Teleport uses Go `strings.EqualFold` (Unicode-aware, locale-independent) — correct; a naive Turkish-locale `toLower` would map `I`→`ı` and break matches like `ADMIN` vs `admin`.
6. Label keys/values should be restricted to an ASCII identifier grammar to avoid confusables; display names can be localized separately.
7. Time values in events: UTC RFC 3339; localized formatting only in UI.

---

## 7. Risks, pitfalls, open questions

**Risks / pitfalls**
1. **Agent compromise = DB admin compromise.** Teleport's agent holds admin access to every DB it serves with auto-users. Same for Karagöz; mitigate with minimal admin privileges (CREATEROLE not SUPERUSER; dedicated admin per DB), local secret protection, and audit of admin actions.
2. **Proxy re-encoding trust:** Teleport's proxy can mint identities for the agent; in our P2P design no intermediary should be able to mint user identity — Perde issues, Hacivat holds, Karagöz verifies end-to-end. Relay fallback must be a dumb byte pipe.
3. **Query audit ≠ intent:** GUI clients emit many introspection queries; recordings are noisy (RFD 0171). Tag Hacivat-internal metadata queries vs user-typed queries at the client, but **never trust** the client tag for security decisions.
4. **Extended protocol parameters** may contain PII; Teleport logs bind parameters in clear. Need a redaction policy (doc 05).
5. **Fail-open audit** in Teleport is an explicit availability trade-off; compliance customers may require fail-closed; make it a policy option.
6. **Lease TTL vs operations** — long DDL during activation could exceed 60s lease.
7. **Postgres `PUBLIC` default privileges** make "user with no roles" still able to read some objects — ephemeral-user design must revoke/avoid `PUBLIC` grants or use dedicated schemas.
8. **Locking mode semantics offline**: Teleport's strict mode assumes connectivity to Auth; P2P agents behind NAT may lose control-plane connectivity frequently — design explicit staleness budgets.
9. **License contamination:** none of Teleport's code or SQL may be copied (AGPL). The procedures' *behavior* is described here; Karagöz's SQL must be written independently.

**Open questions**
- [Unknown] Exact current behavior of SQL Server upstream auth options in Community (Kerberos keytab vs Azure AD only) — not fully read.
- [Unknown] Whether Teleport's Oracle engine (Enterprise) implements any statement-level controls.
- [Unknown] Whether VNet for databases is generally available in Community builds and how it handles DB protocols without TLS routing.
- [Unknown] Whether access requests in Community allow resource-based requests via API (matrix says "roles through CLI").
- [Inferred, verify] Label selector values are interpreted as regex only when they are anchored patterns (`^…$`); verify `utils.SliceMatchesRegex` semantics before mirroring.
- [Open for Perde] Do we need Teleport-style threshold *filters* (predicate language) in v1, or are "N approvals from group G" sufficient?
- [Open for Karagöz] Single agent per DB vs HA multiple agents — determines whether provisioning leases must be distributed.
