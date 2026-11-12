# 03 — Other Access Tools and Secrets Engines

CloudBeaver, Hoop.dev, StrongDM, JumpServer, HashiCorp Vault / OpenBao database secrets engine, Infisical dynamic secrets, PgBouncer and ProxySQL. These are the reference points for how Karagöz should hand out database credentials, proxy wire protocols, gate statements and clean up.

> Companion documents: `01-teleport-database-access.md` (Teleport), `02-bytebase.md` (Bytebase), `04-zero-credential-discovery-and-ephemeral-users.md` (**canonical** ephemeral-user provisioning and credential-resolution ALGs), `05-authorization-and-audit-primitives.md` (**canonical** token verification, policy evaluation, query authorization, audit chaining, approval state machine, masking ALGs), `06-open-core-models.md`. The ALGs in this document are **derived** specs. They capture what these tools teach us, and where they overlap with the canonical ALGs in 04/05 they say so and defer to them.

---

## 1. Scope and sources

### 1.1 Repositories studied (shallow clones under `.refs/repos/`, read-only, nothing built or executed)

| Project | Path | Commit (HEAD at clone) | Commit date | License (verified) | Reuse rule |
|---|---|---|---|---|---|
| CloudBeaver (DBeaver web) | `.refs/repos/cloudbeaver` | `5b232f1d7a1b60606bdcc2764c7faadd0ffff132` | 2026-11-10 | Apache-2.0 (`LICENSE`) | Permissive; prefer pseudocode, attribute if reused |
| Hoop.dev | `.refs/repos/hoop` | `d96e43a21efdd3616ea35b271aaa929da3eb99d5` | 2026-11-11 | MIT (`LICENSE`, "Copyright (c) 2020-2024 - Decimals, Inc."). **But** the wire-protocol codecs live in the private module `github.com/hoophq/libhoop` (README, `sidecar/README.md`) | Permissive for what is public; the protocol engine is closed |
| OpenBao (Vault fork, Linux Foundation) | `.refs/repos/openbao` (sparse: `internal/builtin/logical/database`, `internal/builtin/database/*`, `sdk/database`, `sdk/helper/template`, `website/content/docs/{secrets/databases,concepts}`) | `0d706a0c474ebf26265d0d1e4d9060320cd8ae13` | 2026-11-10 | MPL-2.0 (`LICENSE`; file headers `SPDX-License-Identifier: MPL-2.0`) | File-level copyleft: copying a file means keeping that file MPL. We use pseudocode only |
| HashiCorp Vault | not cloned (license checked via GitHub API, `main` = `50b430473c6d5c2c9c4b6fc4702b6333f4e4575d`) | — | — | **BSL 1.1**. Licensor: IBM. Licensed Work: "Vault Version 1.15.0 or later"; the Additional Use Grant forbids competitive hosted/embedded offerings | Prose only |
| Infisical | `.refs/repos/infisical` (sparse: `backend/src/ee/services/dynamic-secret*`, `backend/src/ee/LICENSE.md`) | `51a2bc0711195f3b61ed96baaa4f1fbbb6633a45` | 2026-11-11 | MIT outside `ee/`. Everything under any `ee/` directory is under the proprietary "Infisical Enterprise license" (`backend/src/ee/LICENSE.md`): production use needs a subscription | **Dynamic secrets live in `ee/` and are proprietary → prose only** |
| JumpServer | `.refs/repos/jumpserver` (sparse: `apps/{acls,perms,terminal,assets/models}`, `README.md`) | `5cb4034eed13ff3633985634b293ca7cdedbec57` | 2026-11-11 | GPL-3.0 (`LICENSE`) | Prose only |
| PgBouncer | `.refs/repos/pgbouncer` | `ac3bb6d748cefc9470c2c66e061549a59a072ce3` | 2026-11-06 | ISC (`COPYRIGHT`) | Permissive |
| ProxySQL | `.refs/repos/proxysql` (sparse: `lib/`, `include/`) | `e30a016be613a9e9758f03df78f462b99efd2974` | 2026-11-10 | GPL-3.0 (`LICENSE`) | Prose only |
| StrongDM | proprietary, no source | — | — | Proprietary SaaS | Public docs only; describe behavior only |

### 1.2 Web sources (StrongDM)
- `https://docs.strongdm.com/concepts/what-is-strongdm`: architecture (client, gateways, relays, proxy cluster workers, admin UI).
- `https://docs.strongdm.com/admin/access/policies.md`: Cedar-based Policy-Based Action Control (PBAC), evaluation order, effects.
- `https://help.strongdm.com/hc/en-us/articles/16818171795213` and `…/16818290207117`: deployment guide (proxy nodes, reference architectures). Seen only as search-result summaries.

### 1.3 What was deliberately not done
- No binaries were run. Hoop's `libhoop` codecs are private, so protocol-parsing details for Hoop are **[Unknown]** beyond what the call sites show.
- JumpServer's DB proxy **Magnus** is an Enterprise component (closed). The CE web DB connector **Chen** lives in its own repository and was not cloned. Our JumpServer DB-proxy description comes from the core repo's models and README.
- Vault's MongoDB / MSSQL / Redis plugins were **not** read: OpenBao at this commit ships only `cassandra`, `influxdb`, `mysql`, `postgresql`, `valkey` (`git ls-tree` over `internal/builtin/database/`), and Vault is BSL. Statements about those Vault plugins are tagged **[Inferred: recall of public Vault docs, not re-verified at a commit]**.

---

## 2. Executive summary

1. **The dynamic-credential lifecycle is a solved pattern.** In Vault/OpenBao it is *render templated creation statements → execute in one transaction → attach a lease (TTL, max TTL) → renew by extending `VALID UNTIL` → revoke via templated or default statements on expiry*. Karagöz should copy the **shape** of this lifecycle. It should not adopt Vault itself (BSL, heavy).
2. OpenBao **adds a 5-second buffer** to the DB-side expiration (`VALID UNTIL`) beyond the lease TTL. The database-native expiry is a *backstop* if revocation never runs, and the lease is the primary mechanism [Observed: `openbao/internal/builtin/logical/database/path_creds_create.go:131-134`, `secret_creds.go:72-75`].
3. **Revocation must work after the role definition is deleted.** OpenBao stores `db_name` and `revocation_statements` inside the lease's internal data and falls back to them [Observed: `secret_creds.go:123-163`]. Karagöz leases must likewise be self-contained.
4. **OpenBao's default PostgreSQL revoke is best-effort.** It revokes table/sequence/schema/connect privileges and then `DROP ROLE IF EXISTS`. It does **not** terminate live sessions and does **not** handle owned objects (`REASSIGN OWNED`/`DROP OWNED`) [Observed: `internal/builtin/database/postgresql/postgresql.go:317-432`]. Karagöz must do better: terminate backends, drop owned, then drop the role. See ALG-SECENG-05.
5. **The password must never cross the wire in plaintext DDL.** OpenBao can SCRAM-hash the password locally before substituting it into `CREATE ROLE … PASSWORD '<scram-verifier>'` [Observed: `postgresql.go:270-276`, `postgresql/scram/scram.go`], so `log_statement=ddl` never records a usable secret.
6. **Usernames are generated from templates** with `truncate`, `random`, `unix_time`, `truncate_sha256` and similar. DBMS length limits are baked into the defaults (PG 63, MySQL 32, legacy MySQL 16) [Observed: `postgresql.go:37`, `mysql/mysql.go:32-33`, `sdk/helper/template/funcs.go`].
7. **Static-role rotation uses a priority queue plus a WAL.** A 5-second ticker pops due items. Failure re-queues with a 10-second back-off. A write-ahead log entry makes half-finished rotations recoverable after a crash [Observed: `rotation.go:24-275`]. Root-credential rotation has its own WAL rollback [Observed: `rollback.go`].
8. **Infisical's dynamic secrets are an enterprise (`ee/`) feature.** Revocation is a delayed BullMQ job at `expireAt` (3 attempts). A failed revocation is retried every 4 h ±20 % jitter (3 more times), then the admins are e-mailed and an "orphaned lease" metric is emitted [Observed: `dynamic-secret-lease-queue.ts:49-376`]. This is the right operational shape: retry, escalate, *never silently forget*.
9. **Infisical splits statements naively on `;`** inside a transaction [Observed: `providers/sql-database.ts:322`]. That breaks `DO $$ … $$` blocks and string literals containing `;`. OpenBao uses a heuristic `END` detector for this case [Observed: `postgresql.go:469-501`]. Karagöz must use a real lexer.
10. **Hoop.dev is "MIT", but the protocol brain is a private module.** It also contains a signed-license feature gate (`guardrails`, `data-masking`, `access-requests`, `runbooks`, `access-control`, `resource-discovery`, …) [Observed: `hoop/common/license/license.go:40-74`]. This is the open-core pattern Perde should *avoid copying in spirit* (see doc 06).
11. **Hoop's gateway PG proxy uses the PostgreSQL `user` startup parameter as a bearer secret.** It hashes it with SHA-256, looks up a time-bounded "connection credential", sets a context deadline at expiry, polls the IdP to detect user revocation, and supports revoke-by-credential-ID [Observed: `hoop/gateway/proxyproto/postgresproxy/postgresproxy.go:171-320`]. This works with *any* stock client and is a strong fallback for Karagöz when clients cannot speak our handshake.
12. **Hoop binds approvals to the SHA-256 of the exact statement bytes**, never a normalized form, "so an approval releases those bytes and no others" [Observed: `hoop/gateway/models/reviews.go:83-88, 290-299`]. Hacivat/Karagöz should adopt this for one-time query approvals.
13. **Hoop's access-request rules** carry `approval_required_groups`, `all_groups_must_approve`, `min_approvals`, `force_approval_groups`, `skip_review_groups`, `access_max_duration`, `pending_ttl_sec` and `approval_ttl_sec`. Review states are `PENDING/APPROVED/REJECTED/REVOKED/PROCESSING/EXECUTED/EXPIRED` [Observed: `gateway/models/access_request_rules.go:31-55`, `reviews.go:21-38`]. This is a good feature checklist for doc 05's approval state machine.
14. **CloudBeaver's model** is subjects (users and teams) with global permissions plus object permissions on connections and projects. Effective permissions are the union over user and teams. A built-in `user` team includes everyone, anonymous users included [Observed: `cloudbeaver/config/core/initial-data.conf`, `CBEmbeddedSecurityController.java:952-4060`]. The CE embedded secret store writes secrets with encoding `PLAINTEXT` [Observed: `CBSecretControllerEmbedded.java:280-290`].
15. **StrongDM's model is the closest commercial analogue to Perde+Karagöz.** Credentials are decrypted only by the last proxy node, so users never see them. Roles grant resources. Access Workflows provide JIT. Cedar PBAC (Postgres and SQL Server only) gives per-action forbid/permit/MFA/justification. Forbid overrides permit. Existing sessions are disconnected within 5 s when a policy forbids them [Observed: docs.strongdm.com policies page].
16. **JumpServer** models access as `AssetPermission` (users/groups × assets/nodes × accounts × protocols × action bitmask × validity window, optionally `from_ticket`). Ordered ACLs add login/command/connect-method/data-masking rules with actions `reject/accept/review/warning/notice/…`. Its native-client **database protocol proxy (Magnus) is Enterprise-only** [Observed: `jumpserver/README.md` components table, `apps/perms/models/asset_permission.py`, `apps/acls/*`].
17. **PgBouncer teaches the startup handshake** in detail: SSLRequest accept/NAK, GSSENC always NAK, v2 protocol rejected, re-sent startup rejected, unknown protocol extensions answered with `NegotiateProtocolVersion`. It also teaches *SCRAM pass-through*: the client's SCRAM ClientKey/ServerKey are cached and reused to authenticate to the backend. `mock_auth` hides user existence [Observed: `pgbouncer/src/client.c:1223-1385`].
18. **PgBouncer's `auth_query` default** respects `rolvaliduntil` (it returns NULL once expired) [Observed: `pgbouncer/doc/config.md:562-581`]. This matches the `VALID UNTIL` backstop of item 2.
19. **ProxySQL's ordered rule chain** (`match_digest`/`match_pattern`, `flagIN`/`flagOUT`, `apply`, `error_msg`/`OK_msg`, `replace_pattern`, `destination_hostgroup`, `mirror_*`, `cache_ttl`, `log`) and its **digest-based firewall whitelist** (`OFF/DETECTING/PROTECTING`) are the reference for statement-level routing and allow-listing [Observed: `proxysql/include/ProxySQL_Admin_Tables_Definitions.h:46-335`].
20. **Recommendation:** Karagöz should own a small, embedded "secrets-engine core" (lease store, template renderer, per-DBMS provisioners, revocation loop with escalation, orphan sweeper, rotation WAL). It should treat Vault/OpenBao/Infisical only as *optional external credential sources*, and never require them.

---

## 3. Detailed findings

### 3.1 CloudBeaver: access management and connection sharing

#### 3.1.1 Subject model
- **Subjects** are rows in `CB_AUTH_SUBJECT(SUBJECT_ID, SUBJECT_TYPE, IS_SECRET_STORAGE)`. Users (`CB_USER`) and teams (`CB_TEAM`) both reference a subject row. Membership is in `CB_USER_TEAM(USER_ID, TEAM_ID, …)`, which in recent versions records a *team role*, a *grant type* and *granted by* [Observed: `server/bundles/io.cloudbeaver.service.security/db/cb_schema_create.sql:34-147`; `service.admin.graphqls:25-48` with `@since(version: "26.3.0")` on `grantType`/`grantedBy`].
- **Global permissions** live in `CB_AUTH_PERMISSIONS(SUBJECT_ID, PERMISSION_ID)`. Known IDs: `admin`, configuration manager, `private-project-access`, `secret-manager`, `sql-result-update`, `sql-execute-query`, `fs-resource-edit`, `edit-meta`, `edit-data`, plus "global permissions" `permission.sql.script.execution`, `permission.data-editor.import`, `permission.data-editor.editing` [Observed: `io.cloudbeaver.model/src/io/cloudbeaver/DBWConstants.java:27-44`].
- **Object permissions** live in `CB_OBJECT_PERMISSIONS(OBJECT_ID, OBJECT_TYPE, SUBJECT_ID, PERMISSION)`, primary key over all four. Object types cover data sources (connections) and projects [Observed: schema lines 67-80].
- **Default teams** come from `initial-data.conf`: `admin` ("Has all permissions.", permissions `["admin"]`) and `user` ("All users, including anonymous.", no permissions) [Observed: `config/core/initial-data.conf`].
- **Effective permission resolution** goes through `getAllLinkedSubjects(subjectId)`, which returns `{subjectId} ∪ teams(subjectId)`. The permission query is `SELECT … FROM CB_OBJECT_PERMISSIONS WHERE SUBJECT_ID IN (<linked>) AND OBJECT_TYPE=? [AND OBJECT_ID=?]`. The result is a **pure union**: there is no deny, no precedence and no conditions [Observed: `CBEmbeddedSecurityController.java:952-970, 3990-4060`].
- **Connection sharing.** Admin GraphQL exposes `addConnectionsAccess(projectId, connectionIds, subjects)`, `deleteConnectionsAccess`, `getConnectionSubjectAccess` and `getSubjectConnectionAccess` [Observed: `service.admin.graphqls:308-321`]. Sharing a connection with a team grants it to all members.
- **Projects.** There are global/shared projects and a per-user private project. `LocalResourceController.getProjectPermissions` gives `PROJECT_ADMIN` to admins and `RESOURCE_VIEW, DATA_SOURCES_VIEW` to others on shared projects, and `RESOURCE_EDIT, DATA_SOURCES_EDIT` on the user's own project. The private project requires `private-project-access` and `supportsCustomConnections` [Observed: `io.cloudbeaver.model/src/io/cloudbeaver/model/rm/local/LocalResourceController.java:167-215`].
- **Anonymous access** is configurable (`anonymousAccessEnabled`, `anonymousUserRole: user`) and so are custom connections (`supportsCustomConnections`) [Observed: `server/test/.../conf/cloudbeaver.conf:40-45`, test fixture; production defaults are **[Unknown]** but the env-var defaults in that file are `true`].

#### 3.1.2 Credential storage: shared vs per-user
- A connection can carry **shared credentials**: `WebConnectionInfo.getSharedSecrets()` lists `dataSourceContainer.listSharedCredentials()` [Observed: `WebConnectionInfo.java:614-619`]. Admin config flags `publicCredentialsSaveEnabled` and `adminCredentialsSaveEnabled` control whether users or admins may persist credentials [Observed: `service.admin.graphqls:223-226`].
- **Per-subject secrets** go to `CB_SUBJECT_SECRETS(SUBJECT_ID, SECRET_ID, PROJECT_ID, OBJECT_ID, OBJECT_TYPE, SECRET_VALUE, ENCODING_TYPE)`. Teams can be secret holders (`IS_SECRET_STORAGE`). The CE embedded controller **encodes all secrets as `PLAINTEXT`** and refuses any other encoding on read [Observed: `CBSecretControllerEmbedded.java:40-50, 280-290`]. Protection therefore depends on the security database's own at-rest encryption **[Inferred]**. External secret managers in EE probably plug in through `DBSSecretController`.
- Credentials can also be **"saved only in session"** (`setCredentialsSavedInSession`) [Observed: `WebConnectionInfo.java` near line 640].
- **User credentials for auth providers** are stored in `CB_USER_CREDENTIALS(USER_ID, PROVIDER_ID, CRED_ID, CRED_VALUE)` [Observed: schema line 149-162].

#### 3.1.3 Auth providers and hardening
- In CE: `LocalAuthProvider` (username/password), `RPAuthProvider` (reverse-proxy header auth), and an **LDAP** provider bundle (`io.cloudbeaver.service.auth.ldap`, `plugin.xml` declares `authProvider id="ldap"`) [Observed]. Provider configurations are admin-managed (`saveAuthProviderConfiguration`) [Observed: `service.admin.graphqls`].
- **Brute-force protection**: `bruteforce/BruteForceUtils.java`, `UserLoginRecord.java`, plus `CB_AUTH_ATTEMPT` / `CB_AUTH_ATTEMPT_INFO` tables [Observed].
- Session binding to IP (`bindSessionToIp`) and forced HTTPS [Observed: `CBConstants.java:32-34`].
- **EE-only (per DBeaver product pages; not verifiable in the CE repo)**: SAML/OIDC SSO, Kerberos, cloud IAM auth (AWS/Azure/GCP), external secret managers, query manager/audit, and team-scoped resource controls **[Inferred: DBeaver/CloudBeaver Enterprise marketing; CE code carries `ERROR_CODE_LICENSE_DENIED = "licenseRequired"` in `DBWebException.java:42`, so the shared codebase has license-gated paths]**.

#### 3.1.4 Lessons
- A union-only RBAC with teams is simple and auditable, but cannot express "everyone in team X **except** prod" without a deny. Perde should add explicit deny or forbid-overrides semantics (doc 05).
- Plaintext secret storage behind an app DB is a weak default. Karagöz never stores standing DB secrets on Perde (zero-credential design, doc 04). Where Karagöz caches anything, it uses OS keyring or an age/XChaCha20-sealed file.
- "Anonymous users are in the `user` team" is a footgun. Perde must have no anonymous principal.

### 3.2 Hoop.dev

#### 3.2.1 Licensing and gating
- The repository is MIT [Observed: `LICENSE`]. The README states: *"The wire protocol codecs live in a private module (`github.com/hoophq/libhoop`). Building the `sidecar/` module needs `GOPRIVATE=…` and credentials for it."* [Observed: `README.md`]. The agent's PG path calls `libhoop.NewDBCore(ctx, streamClient, opts).Postgres()` [Observed: `agent/controller/postgres.go:86`]. So the **parsing, masking and guardrail enforcement engine is not open source**. This is the "open shell, closed core" variant of open core.
- The signed license (RSA, `KeyID`, `Signature`, `Payload{Type: oss|enterprise, IssuedAt, ExpireAt, AllowedHosts, Features}`) gates these feature keys: `guardrails`, `data-masking`, `ai-session-analyzer`, `access-requests`, `runbooks`, `jira-integration`, `event-routing`, `ai-agents`, `rulepacks`, `machine-identities`, `access-control`, `provisioning-hub`, `resource-discovery`. "Empty or absent means all features are enabled." Explicit errors exist for "data masking … not supported with the open source license" and the same for webhooks [Observed: `common/license/license.go:33-99`].

#### 3.2.2 Architecture (gateway era, still shipped)
- **Gateway**: REST on `:8009`, gRPC on `:8010` [Observed: README table]. Postgres-backed models cover connections, reviews, access-request rules, guardrails, data masking, runbooks, sessions, audit and machine identities (`gateway/models/*`).
- **Agent**: runs inside customer infrastructure and **dials out** to the gateway over gRPC (`HOOP_KEY` = DSN-encoded token with scheme `grpc[s]`/`http[s]`/`ws[s]`, address, name and agent mode). SPIFFE SVID mode is an alternative [Observed: `agent/config/config.go:21-105`]. Agent modes are `standard`, `embedded` and `multi-connection` [Observed: `common/proto/const.go:163-165`].
- **Credentials** for a connection are stored on the gateway as env vars prefixed `envvar:` [Observed: `gateway/models/envvars.go:20-44`]. Plaintext secrets are encrypted with a 32-byte AES key that is generated once and kept in `private.serverconfig.credential_encryption_key` [Observed: `gateway/models/credential_crypto.go:16-55`]. Per session, the gateway sends `connectionParams.EnvVars` to the agent. The agent parses host/port/user/pass/sslmode and opens the upstream connection [Observed: `agent/controller/postgres.go:42-87`]. **The gateway therefore holds standing DB credentials.** This is the opposite of Karagöz's zero-credential goal.
- **Per-session protocol options** passed to the engine: DLP provider (`mspresidio` analyzer/anonymizer URLs, or GCP DLP credentials), `dlp_info_types`, masking character `#`, `data_masking_entity_data`, `dlp_experimental_redact_rows`, `guard_rail_rules` [Observed: `postgres.go:51-83`]. Protocol controllers exist for postgres, mysql, mongodb, mssql, oracle, ssh, ssm, tcp, http/mcp proxies [Observed: `agent/controller/*.go`].
- **Native client access through the gateway (PG)** [Observed: `gateway/proxyproto/postgresproxy/postgresproxy.go:171-320`]:
  1. Decode the first packet. If it is an SSLRequest and TLS is configured, reply `'S'`, do the TLS handshake, then decode the real startup packet.
  2. CancelRequest → *not implemented* (returns error).
  3. Parse the startup parameters. **The `user` parameter value is treated as the secret access key.**
  4. `Hash256Key(secret)`, then look up a valid `ConnectionCredentials` row (`secret_key_hash`, `expire_at`, `revoked_at`, `user_subject`, `connection_name`) [Observed: `gateway/models/connection_credentials.go:11-24`].
  5. Reject if expired. For human credentials, verify the user's IdP token now and **poll it** for the life of the connection (`usertoken.PollingUserToken`), cancelling on failure.
  6. Create a context with a deadline at `expire_at` and cancel cause "connection access expired".
  7. Open an internal gRPC stream as that user (impersonation header) toward the agent and pump bytes.
  8. `RevokeByCredentialID` cancels every live connection using the credential.
  9. A background job, `credentialsweeper`, later finalizes audit sessions of expired credentials. It used to run inline on read endpoints and was moved to a job for correctness and multi-tenant fairness [Observed: `gateway/jobs/credentialsweeper/sweeper.go` header comment].
- **DB role provisioning (AWS)**: the agent can create `ro` / `rw` / `ddl` roles per database. The PG template uses `CREATE ROLE … WITH LOGIN ENCRYPTED PASSWORD … NOINHERIT NOCREATEDB NOCREATEROLE NOSUPERUSER` (or `ALTER ROLE` if it exists) and loops over schemas to `GRANT USAGE` and `GRANT <privs> ON ALL TABLES IN SCHEMA`. Privileges are `SELECT` for ro and `SELECT, INSERT, UPDATE, DELETE` for rw [Observed: `agent/controller/system/dbprovisioner/{const.go,postgres.go}`]. These are **standing** role tiers, not per-session ephemeral users.
- **Runbooks** are templates fetched from a Git repo (HTTP basic or SSH key auth) and executed against connections [Observed: `common/runbooks/config.go`].

#### 3.2.3 Reviews and access requests
- Review **types**: `jit` (time-boxed access) and `onetime` (single execution). **Statuses**: `PENDING, APPROVED, REJECTED, REVOKED, PROCESSING, EXECUTED, UNKNOWN, EXPIRED`. `EXPIRED` is a terminal state for sidecar reviews past their deadline [Observed: `gateway/models/reviews.go:21-58`].
- A review records `AccessDurationSec`, `ReviewGroups[{GroupName, Status, OwnerEmail…}]`, `ForceApprovalGroups`, `MinApprovals`, `TimeWindow{Type,…}`, the input (`BlobInputID`, `InputEnvVars`, `InputClientArgs`) and, for sidecar reviews, `SidecarID` + `ListenerName` + `StatementHash` [Observed: `reviews.go:67-110`].
- **Statement hash semantics**: SHA-256 of the *exact* bytes. "anything that folded case, whitespace or literals here would widen what a single approval releases". A partial unique index prevents duplicate reviews for the same bytes while one is live [Observed: `reviews.go:83-88, 145-155, 285-299`].
- **Access-request rules** per org: `connection_names[]`, `access_type`, `approval_required_groups[]`, `all_groups_must_approve`, `reviewers_groups[]`, `force_approval_groups[]` (any member can approve alone), `skip_review_groups[]`, `access_max_duration`, `min_approvals`, `pending_ttl_sec`, `approval_ttl_sec`, plus attribute bindings [Observed: `gateway/models/access_request_rules.go:31-55`].

#### 3.2.4 Sidecar direction (2026)
- The new product is "runtime control for agents": a sidecar on the wire in front of a DB/HTTP upstream. It applies four controls: Data Masking (rewrites response values in memory), Guardrails (ordered deny list per statement; the client gets a *real protocol error* such as a pgwire `FATAL` with the configured message), Session Analyzer (an LLM scores intent; local rules first), and Reviews [Observed: `README.md`]. Supported protocols: PostgreSQL, SQL Server, HTTP. Design rule: "Relay codecs are pure functions over bytes … The daemon owns protocol transports" [Observed: `sidecar/README.md`].
- **Lesson for Karagöz:** return refusals as native protocol errors (PG `ErrorResponse` with SQLSTATE `42501 insufficient_privilege`, MySQL `ER_SPECIFIC_ACCESS_DENIED_ERROR` 1227, TDS `ERROR` token with number 229/262). Never drop the connection, because agents and humans both handle a typed error far better.

### 3.3 StrongDM (proprietary; public docs only)

- **Components** [Observed: docs.strongdm.com/concepts/what-is-strongdm]: an Admin UI that pushes configuration to clients in real time; a **Client** (desktop app and/or `sdm` CLI) that tunnels to gateways over a single TLS 1.2 TCP connection; **Gateways** (ingress) and **Relays** (egress-only reverse tunnels to gateways); **Proxy cluster workers** as an alternative node type; and the **node intermediary**, i.e. the last node, which "decrypts credentials on behalf of end users". SDKs and a Terraform provider also exist.
- **Credentials** live with StrongDM or in a customer secret store, and users never see them. Roles let many users share resource credentials without unique DB users per person. "Temporary credential provisioning for on-demand grants" also exists [Observed: same page].
- **Grants** are role-based. Access Workflows automate JIT requests with approve/deny [Observed]. Static vs dynamic (tag-based) access rules **[Inferred: recall of StrongDM docs; not in the fetched page]**.
- **Cedar PBAC** [Observed: docs.strongdm.com/admin/access/policies]:
  - Entities: principals (users, service accounts, matched e.g. `principal in StrongDM::Role::"…"`), actions, and resources typed as `Postgres::Database`, `SQLServer::Database`, etc.
  - Database-action control applies to **Postgres and SQL Server only**, with ">180 Postgres actions" (CRUD and more). A `connect` permit grants nothing further.
  - Context: device trust, IP, location, date/time, action, resource.
  - Effects: `forbid` (overrides all permits), `permit`, MFA challenge (retry within 5 minutes; the retry is re-evaluated without another prompt), justification (logged), approval, logout/notify on denial.
  - Evaluation order: (1) grant check (role or temporary grant), (2) forbids, then permits, (3) requirements (MFA, justification), (4) default deny unless a global `permit(principal, action, resource);` exists.
  - **Policy changes that forbid a connection disconnect existing sessions within 5 seconds** (CLI ≥ 51.53.0).
- **Logging**: protocol-level query logs across DBMSs, replays for supported protocols, and admin activity logs. Gateways/relays can store logs locally and ship them to a customer archive or SIEM [Observed: search summaries of strongdm.com pages; vendor material].
- **Lessons**: (a) "last node decrypts credentials" is exactly Karagöz's role, but Karagöz goes further and *discovers* the credentials locally (doc 04). (b) Cedar with forbid-overrides plus requirement effects (MFA/justification/approval) is an excellent policy shape for Perde. (c) The 5-second re-evaluation of live sessions after a policy change is a concrete SLO we should match. Karagöz pushes policy-epoch bumps and re-evaluates open sessions (doc 05).

### 3.4 JumpServer (GPL-3.0)

- **Components** [Observed: `README.md` components table]: Core (Django API), Lina (web UI), Luna (web terminal), **KoKo** ("general-purpose protocol connector and proxy", CE; SSH/K8s and DB CLIs in a terminal), **Chen** ("web database connector", CE), Kael (AI), Tinker (Windows apps, free for CE), and Enterprise-only Panda, Razor (RDP), **Magnus ("Enterprise Edition database protocol proxy")** and Nec (VNC). The `TerminalType` enum lists the same plus `omnidb`, `guacamole`, `xrdp`, `video_worker`, `facelive` [Observed: `apps/terminal/const.py:59-76`]. Lifecycle: "V3 Community Edition (CE) reaches end of life on December 31, 2026" [Observed: README line 29].
- **Authorization unit**: `AssetPermission` with `users`, `user_groups`, `assets`, `nodes` (an asset tree), `accounts` (JSON list of account names or special aliases), `protocols`, `actions` (bitmask: connect, upload, download, copy, paste, delete, share), `date_start`, `date_expired`, expiry-soon notifications, `is_active`, `from_ticket` [Observed: `apps/perms/models/asset_permission.py:60-95`; `apps/perms/const.py:11-31`]. JIT access is implemented as tickets that create time-bounded `AssetPermission`s (`from_ticket=True`).
- **ACLs** (ordered by `priority`, then active, then name; each with `action` and `reviewers`) [Observed: `apps/acls/models/base.py:34-50`]:
  - `LoginACL` (user login: IP group, time period), `LoginAssetACL` (user × asset × account), `CommandFilterACL` with `CommandGroup` (type `command` or `regex`, one command per line, `ignore_case`), `ConnectMethodACL`, and `DataMaskingRule` (`fields_pattern` default `password`, `masking_method` ∈ {`fixed_char`, `hide_middle`, `keep_prefix`, `keep_suffix`}, `mask_pattern`) [Observed: `apps/acls/models/*.py`].
  - Actions: `reject, accept, review, warning, notice, notify_and_warn, face_verify, face_online, change_secret` [Observed: `apps/acls/const.py:5-14`].
  - Account matching accepts the account username, `*` or `@ALL` [Observed: `base.py:115-119`].
  - Rule conditions include `ip_group` and `time_period` [Observed: `base.py:69-72`].
- **Sessions**: `apps/terminal/models/session/{session,command,replay,sharing}.py` give session records, per-command records (with risk level), replays and session sharing [Observed: file list].
- **Lessons**: (a) Column masking keyed on a *field-name pattern* is cheap and works without a schema catalog; it complements Bytebase-style classification (doc 02). (b) The `review` ACL action, which turns a matching command into a ticket, is the same idea as Hoop's statement reviews. (c) Gating the *native-client* DB proxy as enterprise is a split Perde should **not** copy, because the core OSS value of Karagöz *is* native-client access.

### 3.5 Vault / OpenBao database secrets engine (in depth)

#### 3.5.1 Licensing context
- HashiCorp Vault ≥ 1.15.0 is **BSL 1.1**, now with IBM as licensor [Observed: GitHub API on `hashicorp/vault` `LICENSE`]. OpenBao forked the last MPL-2.0 code and is MPL-2.0 [Observed]. We describe behavior and use our own pseudocode.

#### 3.5.2 Plugin architecture
- The backend (`internal/builtin/logical/database`) talks to DB plugins through the **dbplugin v5** interface: `Initialize(config, verify)`, `NewUser`, `UpdateUser` (password change and/or expiration change), `DeleteUser`, `Type`, `Close`. Plugins can run in-process or out-of-process over gRPC (`sdk/database/dbplugin/v5/grpc_*.go`) [Observed: file list]. A v4 wrapper (`version_wrapper.go`) adapts legacy plugins that *generate* passwords themselves [Observed: `path_creds_create.go:217-230` comment].
- **Shipped plugins in OpenBao at this commit**: cassandra, influxdb, mysql (+ legacy mysql), postgresql (with a `scram/` subpackage), valkey [Observed]. Vault additionally ships MSSQL, MongoDB, MongoDB Atlas, Oracle (separate), Redis, Redshift, Snowflake, Elasticsearch, HANA, Couchbase and others **[Inferred: Vault docs]**.

#### 3.5.3 Connection configuration (`database/config/:name`)
Fields [Observed: `path_config_connection.go:74-170`]:
- `plugin_name`, `plugin_version`, `verify_connection` (default true: connect on write);
- `allowed_roles` (list; `"*"` or globs; a role not listed → `"%q is not an allowed role"` at creds time, `path_creds_create.go:98-102`);
- `password_policy` (name of a password policy; the role-level setting overrides it, `path_creds_create.go:160-163`);
- `root_rotation_statements` (statements for `rotate-root`);
- plugin-specific connection details: `connection_url` with `{{username}}`/`{{password}}` placeholders, `username`, `password`, `max_open_connections`, `max_idle_connections`, `max_connection_lifetime`, `username_template`, PG `password_authentication` (`password` | `scram-sha-256`) [Observed: `postgresql.go:82-125`, `postgresql/passwordauthentication.go`].

#### 3.5.4 Dynamic roles (`database/roles/:name`)
Fields [Observed: `path_roles.go:150-246`]: `db_name`, `credential_type` (`password` | `rsa_private_key` | `client_certificate`), `credential_config`, `default_ttl`, `max_ttl`, `creation_statements`, `revocation_statements`, `rollback_statements`, `renew_statements`.

Template variables available in statements [Observed: `postgresql.go:260-266`, `customDeleteUser`]: `{{name}}` and `{{username}}` (same value), `{{password}}`, `{{expiration}}` (formatted timestamp). Revocation gets only name/username.

#### 3.5.5 Credential issuance flow (`database/creds/:role`)
[Observed: `path_creds_create.go:74-248`]
1. Begin a transactional storage scope (OpenBao adds storage transactions).
2. Load the role and the connection config. Enforce `allowed_roles`. Check the plugin supports the credential type.
3. `CalculateTTL(system, increment=0, role.default_ttl, period=0, role.max_ttl, explicitMax=0, start=zero)`. This clamps to mount/system maxima.
4. Refuse on performance-standby replicas (cannot persist the lease).
5. Take the connection (cached plugin instance) under a read lock.
6. `expiration = now + ttl + 5s` (buffer so the DB never expires before the lease).
7. Generate the credential: a password from the password policy (role, then connection, then default), an RSA keypair (public key sent to the DB, private key returned), or a client certificate (subject from the username metadata).
8. `NewUser{UsernameConfig{DisplayName, RoleName}, Statements: creation, RollbackStatements: rollback, Expiration, Password|PublicKey|Subject}`.
9. The response holds the username, password/private key, and **internal data** `{username, role, db_name, revocation_statements}` that the secret engine sees at renew/revoke. `Secret.TTL = role.default_ttl`, `Secret.MaxTTL = role.max_ttl`. The core creates the **lease** and registers it with the expiration manager.

#### 3.5.6 Plugin-side creation (PostgreSQL)
[Observed: `postgresql.go:237-285, 440-501`]
- Error if `creation_statements` is empty (`ErrEmptyCreationStatement`).
- Generate the username from `username_template`. The default is `v-<displayName trunc 8>-<roleName trunc 8>-<random 20>-<unix_time>`, truncated to **63** (PG `NAMEDATALEN-1`).
- Under the plugin mutex, `BEGIN` a transaction. If `password_authentication = scram-sha-256`, **replace `{{password}}` with a locally computed SCRAM-SHA-256 verifier**. Execute each statement, then `COMMIT`.
- **Statement splitting**: if a statement "contains a multiline statement", decided by the case-sensitive `END` keyword outside quoted literals, it is executed as-is. Otherwise it is split on `;`. The source documents this as best effort: an `END` inside a comment fools it [Observed: `postgresql.go:466-484`].

#### 3.5.7 Renewal
[Observed: `secret_creds.go:30-102`]
- Recompute TTL with `req.Secret.Increment` against `default_ttl`/`max_ttl` from **IssueTime** (max TTL is absolute from issuance).
- If `ttl > 0`: `UpdateUser{Expiration{NewExpiration: now+ttl+5s, Statements: renew_statements}}`. The PG default renew statement is `ALTER ROLE "{{name}}" VALID UNTIL '{{expiration}}';` [Observed: `postgresql.go:28-30`].
- Lease semantics [Observed: `website/content/docs/concepts/lease.mdx`]: the increment counts **from now**, not from the old expiry. It is advisory, so callers must read back the granted TTL.

#### 3.5.8 Revocation
[Observed: `secret_creds.go:104-192`]
- Read `username` and `role` from internal data. If the role still exists, use its `db_name` and statements. If it was deleted, fall back to the embedded `db_name` + `revocation_statements`. Empty statements → the plugin default.
- **PG default revoke** [Observed: `postgresql.go:317-432`]:
  1. `SELECT exists(… pg_roles WHERE rolname=$1)`. If absent → success (idempotent).
  2. Collect schemas from `information_schema.role_column_grants WHERE grantee=$1`. For each: `REVOKE ALL PRIVILEGES ON ALL TABLES IN SCHEMA s`, `REVOKE USAGE ON SCHEMA s`.
  3. Always also revoke on `public`: tables, sequences, schema usage.
  4. `REVOKE CONNECT ON DATABASE current_database()`.
  5. Run them **outside a transaction, continuing on error** ("remove as much access as possible"), remembering the last error.
  6. If any step failed → return an error (the lease stays and is retried). Otherwise `DROP ROLE IF EXISTS`.
  - **Gaps**: (a) no `pg_terminate_backend` for live sessions, so an open session keeps working until it disconnects, even though `DROP ROLE` fails while sessions exist **[Inferred: PG semantics: DROP ROLE errors if the role owns objects or has privileges in other databases; open sessions don't block DROP ROLE itself but keep authenticated sessions alive]**; (b) no `REASSIGN OWNED`/`DROP OWNED`, so a role that created a table cannot be dropped; (c) only the *current* database is cleaned.
- **MySQL default revoke**: `REVOKE ALL PRIVILEGES, GRANT OPTION FROM '{{name}}'@'%'; DROP USER '{{name}}'@'%'` [Observed: `mysql/mysql.go:21-24`]. The default rotate statement is `ALTER USER '{{username}}'@'%' IDENTIFIED BY '{{password}}';` [Observed: line 26-28]. Username templates: default `v-<display trunc 10>-<role trunc 10>-<random 20>-<unix_time>` truncated to **32**; legacy `v-<role trunc 4>-<random 20>` truncated to **16** (pre-5.7.8 limit) [Observed: lines 32-33].
- **Valkey/Redis**: create = `ACL SETUSER <name> ON ><password> <rules…>` where the rules come from creation statements parsed as a JSON array (raw string array as fallback); the default rule is `["~*", "+@read"]` (all keys, read commands). Revoke = `ACL DELUSER <name>`. Password rotate = `ACL SETUSER <name> RESETPASS ><password>` [Observed: `valkey/valkey.go:23, 125-197`]. **Note** that Redis ACL users have no native expiry, so revocation is the only end-of-life.
- **Vault MSSQL (prose) [Inferred: Vault docs]**: creation typically `CREATE LOGIN [{{name}}] WITH PASSWORD = '{{password}}'; CREATE USER [{{name}}] FOR LOGIN [{{name}}]; GRANT …`. The default revocation kills the login's sessions (by enumerating `sys.dm_exec_sessions`), drops the user in each database where it exists, then `DROP LOGIN`. A `contained_db` option is available for contained databases. SQL Server has **no login expiry**, so revocation is mandatory.
- **Vault MongoDB (prose) [Inferred: Vault docs]**: creation statement is a JSON document `{ "db": "admin", "roles": [{ "role": "readWrite", "db": "app" }] }` executed as `createUser`. Revocation is `dropUser`. Supports `write_concern`. MongoDB has no user expiry either.

#### 3.5.9 Rollback statements
- `rollback_statements` are passed with `NewUser` for plugins that can undo a partial creation [Observed: `path_creds_create.go:144-146`]. In SQL plugins the creation transaction itself is the main rollback mechanism. Rollback statements matter for non-transactional DBMSs (e.g. Cassandra) **[Inferred]**.

#### 3.5.10 Leases and the expiration manager
- Every dynamic secret has a lease (`lease_id` prefixed by the request path, e.g. `database/creds/readonly/<id>`). Revocation can be manual, by **prefix** (`bao lease revoke -prefix database/creds/`), automatic on expiry, or cascaded on **token revocation** (revoking a token revokes all leases created with it) [Observed: `concepts/lease.mdx`].
- The expiration manager keeps leases in storage, schedules timers, and on startup restores pending leases. Revocation failures retry with exponential back-off. In Vault ≥ 1.8, after a fixed number of attempts a lease becomes **irrevocable** and is listed for operators instead of being retried forever **[Inferred: Vault docs/changelog recall; `vault/expiration.go` not in sparse checkout]**.

#### 3.5.11 Static roles and rotation
[Observed: `rotation.go`, `path_roles.go:231-246`, `path_rotate_credentials.go`]
- A static role binds an *existing* DB user (`username`) to managed rotation: `rotation_period`, `rotation_statements`, `credential_type`. `static-creds/:name` returns the current password plus `ttl` (time to next rotation), `rotation_period` and `last_vault_rotation`.
- **Queue**: on mount init, after loading WALs, `populateQueue` lists static roles and pushes `queue.Item{Key: role, Priority: NextRotationTime().Unix()}`. A ticker (default **5 s**, configurable via `rotation_queue_tick_interval`) runs `rotateCredentials`, which pops items until it finds one not yet due and pushes that one back.
- **Per item**: take the role's exclusive lock. If the role is gone, re-queue at +10 s / drop. If not due, push back and stop. Otherwise call `setStaticAccount` (create or load a WAL with the new password, call `UpdateUser`, persist, delete the WAL). On failure, re-queue at **now+10 s** and keep the WAL ID in `item.Value`, so the retry reuses the *same* new password (idempotent).
- **WAL reconciliation at startup**: a WAL whose `LastVaultRotation` is zero belongs to a role that was never created → delete it. A WAL older than the role's `LastVaultRotation` → delete it. Otherwise → enqueue immediately with the WAL ID, so the interrupted rotation is completed with the same password.
- **Root rotation rollback** (`rollback.go`): a WAL `{ConnectionName, UserName, NewPassword, OldPassword}` is written before rotating root. On rollback: if storage already holds `NewPassword` → nothing to do. Otherwise, try to connect with the stored (old) password. If that works → done. If not → connect with `NewPassword` and set the password back to `OldPassword`. This reconciles every interleaving of "DB changed / storage changed".

#### 3.5.12 Password policies and username templating
- **Password policy** = length + rules. Each `charset` rule has a `min-chars`. A candidate is generated from the de-duplicated union of rule charsets and **checked** against each rule, with retry. The docs explicitly discuss **modulo bias** when mapping random bytes to a charset whose size doesn't divide 256 and require bias-free selection [Observed: `concepts/password-policies.mdx:27-82`].
- **Username template functions** [Observed: `sdk/helper/template/funcs.go:19-98`]: `unix_time`, `unix_time_millis`, `timestamp(fmt)`, `truncate(n)`, `truncate_sha256(n)` (keep a prefix and append a hash when too long, so distinct long inputs stay distinct; requires n > 8), `sha256`, `base64`/`hex` encode/decode, `uppercase`, `lowercase`, `replace`, `uuid`, `random(n)`. The older `credsutil.GenerateUsername` joins `v`, displayName, roleName, 20 random alphanumerics and the unix time with `_`, then truncates and applies case [Observed: `sdk/database/helper/credsutil/usernames.go`].

### 3.6 Infisical dynamic secrets (proprietary `ee/`; prose only)

- **Licensing**: MIT for the core. Every `ee/` directory is under the Infisical Enterprise license, so production use requires a subscription [Observed: root `LICENSE`, `backend/src/ee/LICENSE.md`]. Dynamic secrets live in `backend/src/ee/services/dynamic-secret*`, which makes **dynamic DB credentials a paid feature** in Infisical.
- **Providers** [Observed: `providers/` file list]: SQL (Postgres, MySQL `mysql2`, Oracle, MSSQL, SAP ASE, Vertica; `SqlProviders` enum in `models.ts:26-33`), Azure SQL, Cassandra, ClickHouse, Couchbase, Elasticsearch, Kafka, LDAP, Milvus, MongoDB, Mongo Atlas, RabbitMQ, Redis, AWS ElastiCache/MemoryDB, AWS IAM, Azure Entra ID, GCP IAM, GitHub, Kubernetes, SAP HANA, Snowflake, SSH, Tailscale, TOTP, OAuth2, IBM API Connect.
- **SQL provider behavior** [Observed: `providers/sql-database.ts`]:
  - Input validation checks host reachability (with SSRF-style host validation, `verifyHostInputValidity`) and **whitelists Handlebars expressions** per template: creation may use `username, password, expiration, database`; renew may use `username, expiration, database`; revoke may use `username, database`.
  - Username: from a user-supplied template rendered with helpers `random(n≤100)`, `truncate`, `replace` (RE2 regex), `uppercase`, `lowercase`, and a normalizer that folds identity names to `[A-Za-z0-9_-]` and caps at 63 chars [Observed: `templateUtils.ts`].
  - Password: required counts per class (lowercase/uppercase/digits/symbols) and allowed symbols, with separate Oracle defaults. It fills required classes, then the remainder, then shuffles with a CSPRNG (`crypto.randomInt`) [Observed: lines 32-120].
  - Create/revoke/renew: render with `noEscape: true`, **split on `;`**, and execute each part in one knex transaction. On error, **redact** username/password/database from the error message before surfacing it [Observed: lines 313-387].
  - Renew is a no-op if there is no renew statement.
  - Optional **gateway** (a relay agent inside the customer network): the provider callback runs over a gateway-proxied local port.
- **Mongo**: `createUser` with configured roles, then `dropUser` [Observed: `mongo-db.ts:92-120`]. **Redis**: templated ACL statements, split on `;` [Observed: `redis.ts:56-181`].
- **Lease TTL policy** [Observed: `dynamic-secret-lease-service.ts:153-157, 325-333`]: create uses `ttl || defaultTTL` and errors if beyond `maxTTL` from now. Renew sets the new expiry to `old expireAt + ttl` and rejects if it would pass `createdAt + maxTTL` ("The requested renewal would exceed the maximum allowed lease duration"). Note the semantics differ from Vault: Infisical extends from the **old expiry**, Vault from **now**.
- **Revocation queue** [Observed: `dynamic-secret-lease-queue.ts`]:
  - At lease creation, enqueue a delayed job `delay = expireAt − now`, `attempts: 3`, back-off 1 min.
  - On manual revoke, stop the scheduled job by lease ID.
  - On final failure, `queueFailedRevocation`: a retry job ID `<leaseId>-retry` with delay ≈ **4 h ±20 % jitter**, up to `MAX_REVOCATION_RETRY_COUNT = 3`.
  - After all retries fail, mark the lease as failed-to-revoke and **e-mail** the admins (15-minute delay). Record per-provider pruning outcomes and an **orphaned-lease metric**.
  - Project restore re-queues revocation of expired leases under a distinct job ID.

### 3.7 PgBouncer (ISC): protocol-proxy reference

- **Pool modes** [Observed: `doc/config.md:83-96`]: `session` (default; server released on client disconnect), `transaction` (released at transaction end), `statement` (released after each query; multi-statement transactions disallowed). Transaction mode breaks session state (SET, advisory locks, LISTEN, temp tables, session-level prepared statements). `max_prepared_statements` tracks protocol-level prepared statements across server connections in transaction mode [Observed: config.md:370].
- **Startup handling** [Observed: `src/client.c:1223-1335`]:
  - Partial packets: wait, or switch to dynamic buffering if larger than `pkt_buf`.
  - `SSLRequest` (code 80877103): an SSLRequest received *inside* TLS → disconnect. If TLS is enabled and the transport is not a unix socket → answer `'S'` and do the TLS accept. Otherwise answer `'N'`.
  - `GSSENCRequest` (80877104): always `'N'`.
  - Startup v2 → disconnect "old V2 protocol not supported".
  - Startup v3 or v3-with-unsupported-minor/extensions: enforce `client_tls_sslmode >= require` except on unix sockets. A **re-sent startup → disconnect**. `decide_startup_pool` parses `user`, `database`, `options`, `application_name`, ignores parameters listed in `ignore_startup_parameters`, tracks `track_extra_parameters`, and answers **`NegotiateProtocolVersion`** for unsupported minor versions or `_pq_.` extensions [Observed: client.c:992-1110].
  - A password packet before startup → disconnect.
  - `CancelRequest` (80877102) is matched to the client's issued cancel key and forwarded to the right server **[Inferred: `PKT_CANCEL` constant in `include/bouncer.h:256`; handling is in `objects.c`/`pooler.c`, not read]**.
- **Auth** [Observed: config.md:479-584]: `auth_type` ∈ `cert` (user from the client cert CN), `md5`, `scram-sha-256`, `plain` (deprecated), `trust`, `any`, `hba` (per-path methods from `auth_hba_file`, e.g. **`peer` on unix socket**), `ldap`, `pam`. `auth_user` + `auth_query` look up unknown users from the DB itself. The default is `SELECT rolname, CASE WHEN rolvaliduntil < now() THEN NULL ELSE rolpassword END FROM pg_authid WHERE rolname=$1 AND rolcanlogin`. The docs recommend a SECURITY DEFINER function over a superuser `auth_user`.
- **SCRAM pass-through** [Observed: client.c:1336-1385]: after verifying the client's SCRAM exchange against a stored verifier, PgBouncer copies the derived **ClientKey and ServerKey** into the pool's credential and sets `scram_passthrough_valid`. It can then authenticate to the backend with SCRAM *without knowing the plaintext password*.
- **`mock_auth`** [Observed: client.c:80, 233, 281, 435]: when the user does not exist, PgBouncer runs a fake authentication exchange so timing and messages don't reveal user existence.
- **`server_reset_query`** (default `DISCARD ALL`) cleans session state before a server connection is reused in session mode [Observed: config.md:673].

### 3.8 ProxySQL (GPL-3.0): rule-chain reference

- **Users** (`mysql_users`): `username`, `password` (clear or hash), `active`, `use_ssl`, `default_hostgroup`, `default_schema`, `schema_locked`, `transaction_persistent`, `fast_forward` (bypass the query processor), `backend`, `frontend`, `max_connections`, `attributes`, `comment` [Observed: `ProxySQL_Admin_Tables_Definitions.h:33`]. ProxySQL authenticates the client itself and uses the same credentials to the backend. Frontend and backend identities are the same user row.
- **Query rules** (latest `pgsql_query_rules`/`mysql_query_rules`) [Observed: lines 46-80, 329-335]: `rule_id` (evaluation order), `active`, match on `username`, `schemaname`/`database`, `flagIN`, `client_addr`, `proxy_addr`, `proxy_port`, `digest` (exact fingerprint hash), `match_digest` (regex on the normalized fingerprint), `match_pattern` (regex on raw text), `negate_match_pattern`, `re_modifiers`. Actions: `flagOUT` (continue the chain with a new flag), `replace_pattern` (rewrite), `destination_hostgroup` (route), `cache_ttl` / `cache_empty_result` / `cache_timeout`, `reconnect`, `timeout`, `retries`, `delay`, `next_query_flagIN`, `mirror_flagOUT` / `mirror_hostgroup`, `error_msg` (deny with message), `OK_msg` (fake success), `sticky_conn`, `multiplex`, `log`, `apply` (stop evaluating).
- **Firewall whitelist** (same file): `*_firewall_whitelist_users(active, username, client_address, mode ∈ {OFF, DETECTING, PROTECTING})`, `*_firewall_whitelist_rules(active, username, client_address, schemaname/database, flagIN, digest)` and `*_firewall_whitelist_sqli_fingerprints`. In PROTECTING mode only allow-listed digests may run. In DETECTING mode violations are logged only.
- **PostgreSQL support**: `pgsql_*` tables exist at this commit (users, query rules, hostgroups) [Observed].
- **Lessons**: (a) Separate "match on normalized fingerprint" from "match on raw text". (b) A *detect-then-protect* rollout mode is very useful for introducing allow-lists without breaking production. (c) `OK_msg` (fake success) is dangerous for audit integrity, so Karagöz should never fake success.

### 3.9 Comparative matrix

| Dimension | CloudBeaver CE | Hoop (gateway) | StrongDM | JumpServer | Vault/OpenBao DB | Infisical DS | PgBouncer | ProxySQL |
|---|---|---|---|---|---|---|---|---|
| Who holds DB secrets | App DB (`CB_SUBJECT_SECRETS`, PLAINTEXT) | Gateway (AES-encrypted env vars) | StrongDM / customer store; last node decrypts | Core (accounts vault) | Engine (root creds in storage) | Infisical (encrypted) | `auth_file` / auth_query | `mysql_users` |
| Per-user ephemeral DB users | No | No (ro/rw/ddl tiers) | "temporary credential provisioning" | No (account rotation) | **Yes** | **Yes** | No | No |
| Protocol-aware | Driver-level (JDBC) | Yes (private libhoop) | Yes | Magnus (EE) | n/a | n/a | PG wire | MySQL/PG wire |
| Statement policy | Permissions only | Guardrails (EE-gated) | Cedar PBAC (PG, MSSQL) | Command ACL regex | n/a | n/a | n/a | Rule chain + firewall |
| Approvals / JIT | No | Reviews jit/onetime, multi-group | Access Workflows, approval effect | Tickets → timed AssetPermission | n/a | Approval in SecretOps (separate) | n/a | n/a |
| Masking | No (CE) | DLP entities (EE-gated) | **[Unknown]** | Field-pattern masking | n/a | n/a | n/a | `replace_pattern` (rewrite) |
| Native client support | n/a (web) | Yes (token in `user`) | Yes (local ports) | Magnus (EE) | n/a | n/a | Yes | Yes |
| License | Apache-2.0 | MIT + private core | Proprietary | GPL-3.0 (+EE) | BSL / MPL-2.0 | MIT + proprietary ee | ISC | GPL-3.0 |

---

## 4. Algorithm specifications (derived)

> All ALGs here are **derived** from the studied tools. Canonical owners:
> - ephemeral DB user provisioning, crash recovery, credential resolution → **doc 04** (`04-zero-credential-discovery-and-ephemeral-users.md`);
> - token verification, permission evaluation, query authorization, approval state machine, audit chaining, masking → **doc 05** (`05-authorization-and-audit-primitives.md`).
> Where a derived ALG overlaps a canonical one, the canonical one wins; these specs add the details learned here (back-offs, buffers, edge cases).

### ALG-SECENG-01: Lease registry, expiry and revocation loop with escalation
- **Problem:** Every ephemeral credential Karagöz creates must be revoked on time, survive Karagöz restarts, never be forgotten silently, and allow renewal up to an absolute cap.
- **Inputs / outputs:** Inputs: `IssueRequest{db_id, principal, role_template_id, requested_ttl}`, `RenewRequest{lease_id, increment}`, `RevokeRequest{lease_id | prefix | principal}`, the clock, and provisioner callbacks (`create`, `extend`, `delete`) from doc 04. Outputs: the `Lease` record, revocation outcomes, alerts/audit events.
- **Algorithm:**
  1. **Issue.** `ttl = clamp(requested_ttl or role.default_ttl, 1s, role.max_ttl, agent.max_ttl)`. `hard_deadline = now + role.max_ttl`.
  2. Write a lease row `state=PROVISIONING` with `{lease_id=ULID, db_id, username (pre-generated, ALG-SECENG-02), revocation_plan (fully rendered statements + dbms + target databases), created_at, expire_at=now+ttl, hard_deadline}`, **fsync**, *then* call `create` (intent-before-action, as in Vault's WAL; doc 04 owns the full crash-recovery path).
  3. `create` sets the DB-native expiry to `expire_at + buffer` where the DBMS supports it (PG `VALID UNTIL`, MySQL `PASSWORD EXPIRE INTERVAL` is not equivalent, see edge cases). `buffer = 5 s` (OpenBao), configurable.
  4. On success: `state=ACTIVE`; schedule a timer at `expire_at`. On failure: the transactional create rolled back, so `state=FAILED_CREATE`, then run `delete` anyway (idempotent) and mark `REVOKED` when it reports absent.
  5. **Renew.** Reject if `state≠ACTIVE`. `new_expire = min(now + increment, hard_deadline)` (Vault semantics: from *now*, not from the old expiry). If `new_expire ≤ now` → reject. Call `extend(new_expire + buffer)`, then persist and reschedule. Return the granted TTL (callers must read it back).
  6. **Expiry tick.** A single scheduler goroutine/task holds a min-heap keyed by `next_action_at`. On wake: pop all due leases. For each, `state=REVOKING`, `attempts += 1`, call `delete(revocation_plan)`.
  7. On success: `state=REVOKED`, `revoked_at=now`, emit audit `credential.revoked`, then terminate the live proxied sessions for that lease (doc 05) *before* the delete when possible.
  8. On failure: `next_action_at = now + backoff(attempts)`, where `backoff = min(base·2^(attempts−1), cap) · jitter(0.8..1.2)`, base 10 s, cap 4 h (Infisical's 4 h ±20 % long-tail, OpenBao's 10 s short re-queue). After `max_attempts` (default 12, ≈ 1–2 days) → `state=IRREVOCABLE`: emit a **high-severity** audit event plus an alert to Perde, and keep retrying daily (never drop the row).
  9. **Revocation precedence**: a manual revoke, a principal revocation (user removed from org) or a prefix revoke (`db_id`, `principal`, `role_template_id`) moves the matching ACTIVE leases to REVOKING immediately.
  10. **Startup**: load all non-terminal leases. PROVISIONING → hand to doc 04's crash-recovery (probe the DB for the username; if present, treat as ACTIVE with the original expiry; if absent, mark REVOKED). ACTIVE with `expire_at ≤ now` → REVOKING immediately. Rebuild the heap.
  11. **Orphan sweep** (hourly): list DB users matching Karagöz's naming prefix/marker (doc 04). Any user without an ACTIVE/REVOKING lease row → schedule a revoke with a synthetic lease (`origin=sweeper`).
- **Data structures:** SQLite table `leases(lease_id PK, db_id, principal_id, role_template_id, username, state, created_at, expire_at, hard_deadline, attempts, next_action_at, revocation_plan BLOB, last_error, origin)`, indexes on `(state, next_action_at)` and `(db_id)`. In-memory min-heap `(next_action_at, lease_id)`.
- **Complexity / performance:** O(log n) per schedule/pop. Revocations run per DBMS connection with bounded concurrency (default 4 per DB) so a revocation storm (e.g. principal removal with 500 leases) does not exhaust `max_connections`.
- **Edge cases (exhaustive):**
  - Clock jumps backwards or forwards (NTP): use a monotonic clock for timers and wall time only for persisted values. On a forward jump > 1 min, re-scan.
  - DB unreachable at expiry: retries with back-off; the DB-native expiry (PG) still blocks new logins.
  - DBMS without native expiry (MySQL/MariaDB: `PASSWORD EXPIRE` makes the user change the password but does not block login in sandbox mode; SQL Server; MongoDB; Redis): the revocation loop is the *only* control. Escalate faster (cap 15 min) and alert sooner.
  - Role template deleted before expiry: `revocation_plan` is embedded in the lease (OpenBao's internal-data fallback).
  - Database dropped or container removed: `delete` reports "target gone", so mark `REVOKED(reason=target_gone)`.
  - Container recreated with the same name but a new volume: the user doesn't exist, so delete is idempotent and returns success.
  - User already dropped by a DBA: success (idempotent existence check first, as in OpenBao).
  - Live sessions: terminate the proxied sessions first, then DB backends (ALG-SECENG-05), then drop.
  - Objects owned by the ephemeral user: ALG-SECENG-05 reassigns or drops them per policy.
  - Renew after `hard_deadline`: reject with `LEASE_MAX_TTL_EXCEEDED`.
  - Renew racing expiry: serialize per lease (row lock or a per-lease mutex). A renew that lands while REVOKING is rejected.
  - Duplicate revoke requests: idempotent.
  - Agent uninstall: the uninstall hook runs "revoke all", then waits up to N seconds and reports leftovers.
  - Lease store corrupted or lost: the orphan sweep (marker-based) is the safety net.
  - Many leases expiring at once (same TTL from a batch): jitter the schedule by ±2 s.
- **Test cases (input → expected):**
  1. Issue ttl=1h, max=8h at t0 → ACTIVE, expire=t0+1h, DB VALID UNTIL=t0+1h+5s.
  2. Renew at t0+30m, increment=1h → expire=t0+1h30m.
  3. Renew at t0+7h50m, increment=1h → expire=t0+8h (capped), granted TTL 10m.
  4. Renew at t0+8h → rejected `LEASE_MAX_TTL_EXCEEDED`.
  5. Expiry while the DB is down for 3 h → attempts at +10 s, +20 s, +40 s … The revoke succeeds after recovery and the state is REVOKED. No IRREVOCABLE.
  6. Delete keeps failing (permission error) for 2 days → IRREVOCABLE after attempt 12, one alert, daily retries continue.
  7. Kill Karagöz between the lease insert and `create` → on restart the lease is PROVISIONING. The probe finds no user, so it is marked REVOKED.
  8. Kill Karagöz after `create` but before the ACTIVE update → the probe finds the user, so the lease is ACTIVE with the original expiry.
  9. DBA drops the user manually → the revoke at expiry succeeds (absent) and the audit reason is `already_absent`.
  10. Orphan `hv_alice_ro_x7k…` exists with no lease → the sweeper revokes it within 1 h and emits `credential.orphan_revoked`.
  11. Revoke prefix `db_id=pg-main` with 3 active leases → all 3 go REVOKING immediately.
- **Prior art and evidence:** OpenBao `path_creds_create.go` (+5 s buffer, internal data), `secret_creds.go` (role-deleted fallback), `concepts/lease.mdx` (increment from now, prefix revoke); Infisical `dynamic-secret-lease-queue.ts` (delayed job at expiry, retry 4 h ±20 %, e-mail, orphan metric) and `dynamic-secret-lease-service.ts` (maxTTL from createdAt); Vault irrevocable leases [Inferred].
- **Used by:** Karagöz (owner), Perde (receives alerts and lease mirror for UI), Hacivat (shows remaining TTL and a renew button).

### ALG-SECENG-02: Ephemeral username generation
- **Problem:** Generate a DB username that is unique, attributable to a person and a role, within DBMS limits, safe to embed in DDL, and recognizable by the orphan sweeper.
- **Inputs / outputs:** Input: `dbms`, `principal_handle` (e.g. email local part or Perde user slug), `role_key`, `agent_prefix` (default `hv`), and an RNG. Output: `username` (ASCII).
- **Algorithm:**
  1. `limit` by DBMS: PostgreSQL 63 bytes; MySQL ≥ 5.7.8 32 chars; MariaDB 80 chars **[Inferred: MariaDB docs]**; SQL Server login `sysname` 128; MongoDB: no practical limit (use 64); Redis: no limit (use 64); ClickHouse: no hard limit (use 64).
  2. `slug(s)`: Unicode NFKD, strip combining marks, map Turkish letters explicitly (`ı→i, İ→i, ş→s, ğ→g, ç→c, ö→o, ü→u`), **ASCII-lowercase** (locale-independent; never `toLowerCase` under a `tr` locale), replace `[^a-z0-9]` with `_`, collapse repeated `_`, trim `_`.
  3. `user = truncate(slug(principal_handle), 10)`, `role = truncate(slug(role_key), 8)`.
  4. `rand = random_base32_lower(8)` (40 bits) from a CSPRNG.
  5. `ts = base36(unix_seconds)` (6–7 chars).
  6. `name = join("_", agent_prefix, user, role, ts, rand)`.
  7. If `len(name) > limit`: shorten `user` first, then `role` (minimum 2 chars each). If still too long → `agent_prefix_<sha256(user|role)[0:8]>_<ts>_<rand>`.
  8. Ensure the first char is a letter (the prefix guarantees it).
  9. Return the name. The caller still quotes it as an identifier (PG `"…"`, MySQL `'…'@'%'`/backticks, MSSQL `[…]`).
- **Data structures:** none persistent. The name is stored in the lease.
- **Complexity:** O(len).
- **Edge cases:** empty or emoji-only handle → `user="u"`; handle that slugs to digits → still prefixed; a collision (the probability with 40 bits per second per role is negligible, but the create step must detect "already exists" → regenerate up to 3×); case-folding DBMS (MySQL usernames are case-sensitive, PG folds unquoted identifiers, so always quote); a reserved word as a segment (irrelevant because of the prefix); a Turkish dotless/dotted I in a mixed-case name (`IŞIK` → `isik`); RTL characters (stripped); MySQL host part not included in the 32 chars.
- **Test cases:**
  1. (`postgres`, `berkay.gökçe`, `readonly`) → matches `^hv_berkay_go_readonly_[0-9a-z]{6,7}_[a-z2-7]{8}$`.
  2. (`mysql`, `ılgaz.öztürk`, `analytics-rw`) → length ≤ 32, starts `hv_ilgaz_oz`.
  3. (`mysql`, 40-char handle, 20-char role) → length ≤ 32, uses the shortened or hash form.
  4. (`postgres`, `İSTANBUL`) → user segment `istanbul` (not `i̇stanbul`).
  5. 10⁶ generations in the same second → no duplicates.
- **Prior art and evidence:** OpenBao default templates (`postgresql.go:37`, `mysql.go:32-33`), template funcs incl. `truncate_sha256` (`sdk/helper/template/funcs.go`), `credsutil/usernames.go`; Infisical identity normalizer (63-char cap, `templateUtils.ts`).
- **Used by:** Karagöz.

### ALG-SECENG-03: Password generation (bias-free, template-safe)
- **Problem:** Generate a high-entropy password that satisfies DBMS complexity rules, contains no characters that break DDL literals or connection strings, and has no modulo bias.
- **Inputs / outputs:** Input: `length` (default 32), `rules[] {charset, min}`. Output: password.
- **Algorithm:**
  1. Default charset classes: lower `a-z`, upper `A-Z`, digits `0-9`, symbols `-_.~+` (exclude `' " \ ; @ : / ? # % & = space` and backtick). For SQL Server, include ≥1 from each of 3 classes (Windows policy); for Oracle-like policies, start with a letter.
  2. Draw `min` chars from each rule's charset using **rejection sampling**: draw a byte b; accept iff `b < 256 − (256 mod |cs|)`; index `b mod |cs|`.
  3. Fill the remaining `length − Σmin` from the union charset, also by rejection sampling.
  4. Fisher–Yates shuffle with unbiased `randint` (rejection sampling on u32).
  5. Verify all rules. If the check fails (impossible by construction), regenerate.
- **Edge cases:** `Σmin > length` → config error at load time; a charset with duplicates → de-duplicate first (OpenBao: "de-duplicated union"); DBMS max password length (MySQL 32 for old `mysql_native_password`? **[Unknown]**; use ≤ 64); `validate_password` plugin on MySQL with a dictionary check (random strings pass).
- **Test cases:** length 32 with rules {lower:1, upper:1, digit:1} → 32 chars, each class present. Chi-square over 10⁶ samples with charset size 67 → uniform (p > 0.01). The output never contains `'` or `;`.
- **Prior art:** OpenBao password policies doc (bias discussion), Infisical `generatePassword` (required counts, shuffle with `crypto.randomInt`).
- **Used by:** Karagöz.

### ALG-SECENG-04: Statement template rendering and safe execution
- **Problem:** Render per-role DDL templates (`{{username}}`, `{{password}}`, `{{expiration}}`, `{{database}}`, `{{schema}}`) without injection, then split and execute them correctly per DBMS.
- **Inputs / outputs:** Input: template text, variables, dbms. Output: an ordered list of statements, executed transactionally where supported.
- **Algorithm:**
  1. **Typed placeholders**: the template language distinguishes `{{ident username}}`, `{{literal password}}`, `{{literal expiration}}`. Each is escaped per DBMS:
     - PG: ident `"…"` with `"`→`""`; literal `'…'` with `'`→`''`, and standard_conforming_strings assumed on (verify via `SHOW standard_conforming_strings`).
     - MySQL: ident with backticks; user-account `'name'@'host'` with `'`→`''` and `\`→`\\` (NO_BACKSLASH_ESCAPES-aware).
     - MSSQL: `[…]` with `]`→`]]`; literal `N'…'`.
  2. Whitelist the allowed variables per template kind (create: user/pass/exp/db; renew: user/exp/db; revoke: user/db), as in Infisical. Reject unknown expressions at template load, not at issue time.
  3. For PG with SCRAM: replace `{{literal password}}` with the locally computed SCRAM-SHA-256 verifier (`SCRAM-SHA-256$4096:<salt>$<StoredKey>:<ServerKey>`), so DDL logs never contain the plaintext (OpenBao `password_authentication=scram-sha-256`). For MySQL 8, prefer `IDENTIFIED WITH caching_sha2_password AS '<hash>'` only if we can compute the hash format reliably **[Unknown: format portability]**; otherwise set `log_statement`-style protections (doc 04 recommends a session-level `SET sql_log_off=1` when privileged).
  4. **Split** with a real lexer per dialect (sqlparser-rs / pg_query tokenizer from `clients/`): split on `;` only at depth 0, outside quotes, dollar-quotes, comments and `BEGIN…END` blocks. Never split on raw `;` (Infisical's pitfall) or rely on an `END` heuristic (OpenBao's).
  5. Execute inside one transaction where DDL is transactional (PG, MSSQL mostly). For MySQL (implicit commit per DDL), execute sequentially with a **compensating plan** (the rendered revoke) on any failure.
  6. Redact secrets from any error message before logging or surfacing it (Infisical `sanitizeString`).
- **Edge cases:** password containing template syntax (impossible by ALG-03's charset, but still escape); a template containing `{{password}}` in an identifier position (rejected by typing); multi-database grants (PG `GRANT CONNECT` per database needs separate connections, so the template declares a `per_database:` section); `DO $$ … $$` blocks; `CREATE FUNCTION` bodies; MySQL `DELIMITER` (client-only directive, rejected); MSSQL `GO` (client-only batch separator, so split on it as a batch boundary at line level); statement timeout on locked catalogs (`lock_timeout=5s`).
- **Test cases:** PG template `CREATE ROLE {{ident username}} LOGIN PASSWORD {{literal password}} VALID UNTIL {{literal expiration}}; GRANT pg_read_all_data TO {{ident username}};` → 2 statements, correct quoting, and the password is a SCRAM verifier. A `DO $$ BEGIN …; …; END $$;` block → 1 statement. MySQL username `o'brien` → `'o''brien'@'%'`. A template with `{{exec}}` → load-time error.
- **Prior art:** OpenBao `executeStatements` + `containsMultilineStatement`, SCRAM pre-hash; Infisical whitelisted Handlebars + naive split + sanitization.
- **Used by:** Karagöz.

### ALG-SECENG-05: DBMS-specific revocation (improved defaults)
- **Problem:** Remove an ephemeral user completely, including live sessions and owned objects, idempotently, across DBMSs.
- **Inputs / outputs:** Input: `username`, `dbms`, `target_databases`, `owned_object_policy ∈ {drop, reassign_to:<role>, fail}`. Output: `REVOKED | ABSENT | ERROR(retryable|fatal)`.
- **Algorithm (per DBMS):**
  - **PostgreSQL** (connect as the Karagöz admin role to each database where the user has CONNECT; the list comes from `pg_database` + `has_database_privilege`):
    1. Existence check in `pg_roles`. If absent → `ABSENT`.
    2. `ALTER ROLE u NOLOGIN` (blocks new sessions immediately; also `VALID UNTIL 'epoch'` as belt-and-braces).
    3. `SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = u` (repeat until 0 or a 10 s timeout).
    4. In **each** database: `REASSIGN OWNED BY u TO <owner_role>` (if the policy says reassign) then `DROP OWNED BY u` (this also revokes privileges and default privileges in that DB).
    5. `DROP ROLE IF EXISTS u` (cluster-wide). On error 2BP01 (dependent objects in a DB we didn't visit) → retryable, and add that database.
  - **MySQL/MariaDB**: `ALTER USER u ACCOUNT LOCK` → `KILL` each `processlist` row of that user (`SELECT id FROM information_schema.processlist WHERE user=u`) → `DROP USER IF EXISTS 'u'@'<host>'` for every host row in `mysql.user` with that name (MySQL `DROP USER` revokes privileges implicitly; OpenBao's explicit REVOKE first is harmless). Definer-owned views/routines/events with `DEFINER=u` keep existing but fail at execution, so report them as a warning.
  - **SQL Server**: `ALTER LOGIN [u] DISABLE` → `KILL` sessions from `sys.dm_exec_sessions WHERE login_name=u` → in each database: `DROP USER IF EXISTS [u]` (fails if the user owns a schema, so `ALTER AUTHORIZATION ON SCHEMA::s TO dbo` first) → `DROP LOGIN [u]`.
  - **MongoDB**: `dropUser` in the auth DB, then `killAllSessionsByPattern` with `users: [{user, db}]`.
  - **Redis/Valkey**: `ACL DELUSER u` (also disconnects that user's clients in Redis ≥ 6.2 **[Inferred]**), then `ACL SAVE` if an aclfile is in use. If ACLs come from a config file that Karagöz doesn't own → `fatal` and alert.
  - **ClickHouse**: `DROP USER IF EXISTS u` (+ `KILL QUERY WHERE user=u`).
- **Edge cases:** user owns objects in a database Karagöz cannot connect to; the admin role lacks `pg_signal_backend`; superuser-owned sessions (cannot terminate) → retryable; replicas (DDL goes to the primary only, and the user disappears on replicas via replication; on logical replicas it must be dropped separately); Galera/Group Replication (DDL replicates); the user is the owner of the database itself (`DROP OWNED` cannot drop a database, so reassign is needed); MSSQL contained databases (no login); the Redis `default` user must never be touched (guard: refuse to revoke names not matching the Karagöz prefix).
- **Test cases:** PG user with an open session and an owned temp table → session terminated, role dropped, `REVOKED`. PG user owning `public.t1` with policy `fail` → `ERROR(fatal, owns_objects)` and an alert. MySQL user with 2 host rows → both dropped. Revoke a non-prefixed username → refused. Revoke twice → second result `ABSENT`.
- **Prior art:** OpenBao PG/MySQL defaults (and their gaps), Vault MSSQL session kill [Inferred], Valkey `ACL DELUSER`.
- **Used by:** Karagöz (canonical provisioning in doc 04 calls this).

### ALG-SECENG-06: Credential rotation with write-ahead intent (static/admin credentials)
- **Problem:** When Karagöz manages a *long-lived* credential (e.g. its own admin role on a bare-metal DB, created once during enrollment per doc 04), rotate it periodically without ever losing the working password across crashes.
- **Inputs / outputs:** Input: `{db_id, username, current_secret (from the sealed store)}`, `rotation_period`. Output: the new secret persisted and the DB updated, or a consistent rollback.
- **Algorithm:**
  1. Priority queue keyed by `next_rotation_at`. Tick every 5 s. Pop while due; push back the first not-due item.
  2. For a due item: if a WAL exists for it, reuse `wal.new_secret`. Otherwise generate (ALG-03) and write a WAL `{db_id, username, old_secret, new_secret, created_at}` sealed and fsynced.
  3. Connect with `old_secret` and run `ALTER … PASSWORD new` (PG with a SCRAM verifier, ALG-04).
  4. Persist `new_secret` to the sealed store, then delete the WAL.
  5. On failure at step 3: re-queue at now+10 s with the WAL kept (the same new secret is retried, so the operation is idempotent).
  6. **Startup reconciliation** for each WAL: if the store has `new_secret` → delete the WAL. Else try to connect with `old_secret`; if that works → the DB is unchanged, so retry the rotation later. Else try `new_secret`; if that works → persist it and delete the WAL. If neither works → alert (`credential.lost`) and fall back to doc 04's credential re-resolution (e.g. re-derive via local peer auth / container env).
- **Edge cases:** the DB changed the password but the store write failed (handled in step 6); two Karagöz instances on the same DB (leader lock in the DB via `pg_try_advisory_lock` or Perde-assigned ownership); a password policy rejecting the new secret (regenerate, bounded); replica lag for MySQL users on read replicas.
- **Test cases:** crash after step 3 → restart, the store has the old value, connecting with old fails and with new succeeds → new is persisted. Crash before step 3 → the old one works → rotation retried with the same new secret.
- **Prior art:** OpenBao `rotation.go` (queue, 5 s tick, 10 s back-off, WAL reuse) and `rollback.go` (root-rotation reconciliation).
- **Used by:** Karagöz.

### ALG-SECENG-07: PostgreSQL front-end termination at Karagöz (startup, auth substitution, cancel mapping)
- **Problem:** Hacivat (or any stock PG client tunneled through Hacivat's local forwarder) connects to Karagöz. Karagöz must authenticate the *principal* by Perde-issued capability, not DB password; open the upstream with the ephemeral/local credential; and relay the protocol with inspection hooks.
- **Inputs / outputs:** Input: a client byte stream (already inside the P2P-encrypted tunnel, or plain TCP on a local listener), the session's verified capability (doc 05) or a bearer access key in `user` (Hoop-style fallback). Output: a proxied session, audit events, typed errors.
- **Algorithm:**
  1. Read an int32 length plus an int32 code. Bound the length (≤ 10 000 bytes for startup, as in PG; otherwise `FATAL 08P01`).
  2. `80877103 SSLRequest`: if the transport is already the authenticated P2P tunnel → answer `'N'` (no double TLS). If it is a raw TCP listener exposed beyond loopback → answer `'S'` and TLS-accept with a Karagöz-local cert. A second SSLRequest inside TLS → disconnect (PgBouncer).
  3. `80877104 GSSENCRequest` → `'N'`.
  4. `80877102 CancelRequest(pid, key)`: look up `proxy_cancel_map[(pid,key)] → (upstream_addr, upstream_pid, upstream_key)`. If found, open a fresh upstream connection and send the CancelRequest. Always close the client socket without a response (PG semantics). Unknown keys are silently ignored (do not leak existence).
  5. `196608 (3.0)` or `3.x` with `_pq_.*` options: parse key/value pairs. Reject a duplicate startup. For unsupported minor versions or `_pq_` options, send `NegotiateProtocolVersion` listing the unsupported options (PgBouncer).
  6. **Principal resolution**:
     a. Tunnel mode: the principal comes from the tunnel's verified capability. The `user` parameter is either ignored or must equal a *DB-role alias* the capability allows (e.g. `readonly`); `database` must be allowed.
     b. Bearer mode (local listener for third-party clients): `user` holds a short-lived access key. Compute `SHA-256(key)` and look up a key in Karagöz's local key table (pushed from Perde or minted by Hacivat with a capability). Constant-time compare. If expired/revoked → run a **mock auth** exchange (request a password, then fail) so existence isn't revealed (PgBouncer `mock_auth`). Never log the raw `user` value in this mode.
  7. Send `AuthenticationOk` to the client (the client never supplies a DB password; in bearer mode, optionally request a cleartext password = second factor over TLS only).
  8. **Upstream**: resolve the credential (doc 04: ephemeral lease via ALG-SECENG-01, or local peer/socket auth). Connect over unix socket when available. Perform the upstream auth (SCRAM/md5/trust/peer) as a normal client. Rewrite `user` to the resolved DB user; pass through `database`, `application_name` (suffixed with `hacivat:<session_id>` for DBA visibility), `options` (**strip** `-c` settings that could weaken controls, e.g. `default_transaction_read_only=off`, `role`, `session_authorization`; doc 05 read-only enforcement sets `default_transaction_read_only=on` here when the grant is read-only).
  9. Relay upstream `ParameterStatus` messages. Replace upstream `BackendKeyData(pid,key)` with a proxy-issued `(ppid, pkey)` (random 32-bit key, or longer keys with protocol 3.2) and store the mapping for step 4.
  10. Enter the relay loop: client→server messages pass through the query-authorization hook (doc 05: `Q`, `P`, `B`, `E`, `D`, `S`, `F` function calls (deny by default), `X`). Server→client messages pass through the masking hook (doc 05: `T` RowDescription + `D` DataRow) and audit counters (`C` CommandComplete tags → rows affected).
  11. Denials are sent as `ErrorResponse(S=ERROR, C=42501, M=<localized-safe message>)` followed by `ReadyForQuery` with the right transaction status, so the session stays usable (Hoop's "real pgwire error").
  12. Session end (client `X`, EOF, lease expiry, policy revocation): close upstream, remove the cancel mapping, emit audit `db.session.end`.
- **Data structures:** `proxy_cancel_map` (bounded LRU, entries removed at session end); `bearer_keys(hash PK, principal, db_id, role_alias, expire_at, revoked_at)`; per-session state `{principal, lease_id, tx_status, prepared_statements{name→sql}, portals}`.
- **Complexity:** O(message) relay. Parsing cost only for client-originated query messages.
- **Edge cases:** clients that send SSLRequest then GSSENC (libpq `gssencmode=prefer` order: GSS first, then SSL); `sslmode=require` clients when we answer `'N'` in tunnel mode → *client fails*, so Hacivat's forwarder must configure the driver with `sslmode=disable` toward the loopback forwarder, and third-party clients use the bearer listener with TLS; `replication=database` startup parameter (deny unless explicitly granted); `options=-c search_path=…` (allow); protocol 3.2 (PG 18) longer cancel keys (support variable-length keys); the startup packet split across TCP segments (buffer until complete); pipelining before `ReadyForQuery`; `COPY FROM STDIN` (`d`/`c`/`f` messages → doc 05 classification as write); an upstream `NoticeResponse` storm; upstream `ParameterStatus` changes mid-session (relay); a lease expiring mid-transaction (terminate with `FATAL 57P01`-like message + audit).
- **Test cases:** psql via tunnel, `user=readonly dbname=app` → connected as the ephemeral `hv_…`; `SHOW application_name` → `psql hacivat:<sid>`. `CancelRequest` with the proxy key → the upstream query is cancelled. `CancelRequest` with a random key → silently closed, no upstream traffic. Bearer key expired → mock password request, then `28P01`. `options='-c default_transaction_read_only=off'` on a read-only grant → stripped and audited. `DELETE` on a read-only grant → `ERROR 42501`, the session continues, and `SELECT 1` then works.
- **Prior art:** PgBouncer `client.c` (startup state machine, NegotiateProtocolVersion, mock auth, SCRAM pass-through), Hoop `postgresproxy.go` (token in `user`, hashed lookup, deadline at expiry, IdP polling, revoke by credential ID, cancel not implemented, which is a gap to avoid). Teleport (doc 01) for the cert-based variant.
- **Used by:** Karagöz (server side), Hacivat (local forwarder side and driver config).

### ALG-SECENG-08: Approval binding to exact statement bytes (one-time query approval)
- **Problem:** A user needs approval for one specific risky statement. The approval must release exactly that statement, once, within a window, for that principal and database. It must never release a variant.
- **Inputs / outputs:** Input: `principal`, `db_id`, `db_role`, `statement_bytes` (as sent on the wire: simple-query text, or the extended-protocol Parse text plus the canonical encoding of bound parameters), `approval_rule`. Output: approval request ID; later `ALLOW_ONCE` or `DENY`.
- **Algorithm:**
  1. `h = SHA-256(len‖statement_bytes ‖ len‖param_encoding ‖ db_id ‖ db_role)` (hex). Bind the parameters too, because otherwise an approved `DELETE … WHERE id=$1` could be replayed with any id.
  2. If a live request with the same `(principal, h)` exists in `PENDING|APPROVED` → return it (dedupe on retry, Hoop's partial unique index).
  3. Otherwise create a request with `{h, preview (truncated, masked), pending_ttl, approval_ttl, required groups, min approvals}` and send it to Perde (doc 05 owns the state machine).
  4. Karagöz holds the statement (it does not forward it) and returns `ERROR 42501` with "approval pending: <id>" to the client. Optionally keep the session open for retry.
  5. When the client retries the same bytes: recompute `h`. If an `APPROVED` request exists with `approved_at + approval_ttl > now` and `uses < 1` → atomically mark it `EXECUTED` (compare-and-set), then forward. Otherwise deny.
  6. Audit both the approval and the execution with the same `h`.
- **Edge cases:** a client that reformats SQL on retry (whitespace or comment changes, e.g. ORMs adding comments) → a different hash, so a new request is needed (intended; Hacivat should resend the identical bytes); statements with a non-deterministic `NOW()` (fine, the bytes are equal); multi-statement simple queries (`Q` with `;`) → hash the whole message and approve as a unit; extended protocol where the Parse happened earlier → hash at Execute using the stored Parse text + Bind params; an approval for principal A used by B → denied (the principal is in the key); the approver is the requester → disallowed unless the rule permits (doc 05).
- **Test cases:** approve `DELETE FROM t WHERE id=$1` with param 7 → executing with 7 is allowed once, a second execution is denied, executing with 8 is denied. The same SQL with an extra space → a new request. Approval expired after 15 min → denied.
- **Prior art:** Hoop `reviews.go` (`StatementHash`, exact bytes, "an approval releases those bytes and no others"), JumpServer `review` ACL action, StrongDM approval effect.
- **Used by:** Karagöz (enforcement), Perde (workflow), Hacivat (UX: "request approval" on a denied query; resend identical bytes).

### ALG-SECENG-09: Ordered statement rule chain with detect/protect rollout
- **Problem:** Let admins express statement-level routing/deny/log rules (ProxySQL-like), with a safe rollout mode, on top of the policy decision from doc 05.
- **Inputs / outputs:** Input: a classified statement `{kind, tables, fingerprint, fingerprint_hash, raw}` (from doc 05's SQL classifier), the session context, and an ordered rules list. Output: `{verdict: allow|deny|review, message, log: bool, matched_rule_ids}`.
- **Algorithm:**
  1. `flag = 0`. Iterate rules by ascending `rule_id` where `active` and `rule.flag_in == flag`.
  2. A rule matches if all present predicates hold: `principal`/`group`, `database`, `client_kind` (human/AI agent), `kind ∈ set`, `fingerprint_hash ==`, `fingerprint ~ regex` (RE2, linear time), `raw ~ regex` (discouraged), and `negate` applied to the regex predicate.
  3. On a match: record the rule ID. If `rule.action` is set (deny/review/log) → set the verdict candidate. If `rule.flag_out` is set → `flag = flag_out` and continue. If `rule.apply` → stop.
  4. The final verdict combines with doc 05's policy decision using **forbid-overrides**: any deny from either side → deny; else any review → review; else allow.
  5. **Mode** per rule set: `DETECTING` (deny/review verdicts are logged as `would_deny` but allowed) or `PROTECTING` (enforced). An allow-list mode (`only listed fingerprints may run`) is available per principal (ProxySQL firewall whitelist).
- **Edge cases:** rule loops via flags (prevent by requiring `flag_out > flag_in` or a max-hop count of 32); regex catastrophic backtracking (use RE2 only); an unparseable statement (classifier returns `unknown` → match only `kind=unknown` rules; default deny for AI-agent principals per doc 05); a never-matching rule list → the policy decision stands; ordering changes (version the rule set; audit records carry the version).
- **Test cases:** rules [1: kind∈{drop,truncate} → deny, apply]. `DROP TABLE x` → deny(rule 1). In DETECTING mode → allowed + `would_deny` audit. Allow-list for principal `ai-agent-1` with 3 fingerprints → a 4th fingerprint is denied.
- **Prior art:** ProxySQL `mysql_query_rules`/`pgsql_query_rules` fields and firewall whitelist modes; Hoop guardrails (ordered deny list with custom message); JumpServer command ACL (regex groups, priority).
- **Used by:** Karagöz (evaluation), Perde (authoring and versioning), Hacivat (shows the matched rule message).

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 Karagöz
1. **Build the secrets-engine core in-process** (Rust): a lease store (SQLite, WAL mode), scheduler (ALG-SECENG-01), username/password generators (02/03), template renderer + lexer-based splitter (04), per-DBMS revokers (05) and a rotation WAL (06). Do not depend on Vault, OpenBao or Infisical. Offer an **optional** "external credential source" adapter (read a static secret or request a dynamic one from OpenBao/Vault/Infisical) for organizations that already standardize on them; doc 04's credential-resolution order slots it in.
2. **Always set DB-native expiry where it exists** (PG `VALID UNTIL` = lease expiry + 5 s) as a backstop. For MySQL/MSSQL/Mongo/Redis, compensate with a shorter retry cap and an hourly orphan sweep.
3. **Pre-hash PG passwords** with SCRAM before DDL. Never put plaintext passwords in statements that may be logged. Redact any error that might echo them.
4. **Revocation is complete, not best-effort**: NOLOGIN/LOCK → terminate sessions → handle owned objects → drop. Treat "owns objects" as a policy decision (default: `reassign_to` the DB owner for AI-agent/human sessions with write grants; `fail` for read-only grants because they should own nothing).
5. **Return typed protocol errors** for every denial (PG 42501, MySQL 1227/1142, TDS 229), with a localized message key (see §6), and keep the session alive.
6. **Implement cancel-key mapping from day one** (Hoop's gap). Users of a DB client expect "cancel query" to work.
7. Provide a **bearer-key loopback listener** (ALG-SECENG-07 6b) so third-party tools (psql, DBeaver, BI tools) can use Karagöz-brokered access through Hacivat's forwarder without our SDK. Keys are short-lived, hashed at rest, and revocable by ID, as in Hoop.
8. Implement **detect-then-protect** for any new rule set (ALG-SECENG-09) and for AI-agent allow-lists.
9. Escalate un-revocable credentials loudly: the IRREVOCABLE state, an alert to Perde, and a red badge in Hacivat.

### 5.2 Perde
1. **Access-request rules**: adopt Hoop's field set (`approval_required_groups`, `all_groups_must_approve`, `min_approvals`, `force_approval_groups`, `skip_review_groups`, `access_max_duration`, `pending_ttl`, `approval_ttl`) plus StrongDM's requirement effects (MFA, justification). Model JIT grants as *time-bounded grants* (JumpServer's `from_ticket` permission rows), never as edits to standing roles.
2. **Policy semantics**: grants (RBAC) first, then policy with **forbid-overrides**, then requirements (StrongDM evaluation order). Push policy epochs so live sessions are re-evaluated within **≤ 5 s** (StrongDM's documented SLO).
3. **No anonymous principal** and no "all users" implicit team that grants access (CloudBeaver footgun). The org-wide group exists only for UI defaults.
4. **Never store standing DB credentials centrally** (Hoop gateway and CloudBeaver CE both do). Perde stores only *policy and metadata*. Karagöz resolves credentials locally (doc 04).
5. **Open-core line** (feeds doc 06): do **not** gate the protocol engine, native-client access, basic approvals, masking primitives or audit integrity behind a license (Hoop's EE gates `guardrails`/`data-masking`/`access-requests`; JumpServer gates Magnus; Infisical gates dynamic secrets). Security-critical controls in the OSS core avoid the "security tax" backlash.

### 5.3 Hacivat
1. Show the lease TTL and a renew button. Warn 5 min before `hard_deadline`.
2. On a 42501 with an `approval pending: <id>` payload, show "Request approval" / "Waiting for approval" and **resend the identical bytes** when approved (ALG-SECENG-08).
3. Configure the PG/MySQL driver toward the local forwarder with `sslmode=disable` (the tunnel provides encryption). Expose a "copy connection string for external tool" action that mints a bearer key (ALG-SECENG-07).

---

## 6. i18n considerations

- **Error messages crossing the wire**: protocol error messages (PG `M` field, MySQL error text, TDS message) are shown verbatim by third-party clients. Karagöz should send **English text plus a stable message key** (e.g. in PG the `D`etail/`H`int fields carry `hacivat.err.approval_pending id=…`). Hacivat parses the key and renders the localized string (TR/EN). Third-party clients see English. Never localize SQLSTATE codes.
- **Usernames from human names**: the Turkish dotted/dotless I (`I/ı/İ/i`) breaks naive lowercasing under a `tr` locale. ALG-SECENG-02 mandates explicit transliteration and ASCII-only lowercasing. Test with `IŞIK`, `İzmir`, `ıspanak`.
- **Approval previews** in Perde/Slack/e-mail must keep the SQL verbatim (no translation). Labels and state names (PENDING, APPROVED, …) are localized from keys. Durations are formatted with ICU (`1 saat 30 dakika` / `1 hour 30 minutes`).
- **Masking method names** (JumpServer-style `hide_middle`, `keep_prefix`) are enum keys and get translated labels.
- **Timestamps** in `VALID UNTIL` and audit records are always UTC ISO-8601. The UI renders local time.
- **Notification e-mails** for irrevocable credentials (Infisical-style) use the recipient's locale, with a fallback to the org default, then English.

---

## 7. Risks, pitfalls, open questions

### 7.1 Risks and pitfalls
1. **Plaintext in DDL logs**: `log_statement=ddl|all`, `pgaudit`, MySQL general log or `audit_log` capture `CREATE USER … IDENTIFIED BY '<pw>'`. Mitigations: PG SCRAM pre-hash; for MySQL `SET SESSION sql_log_off=1` (needs SUPER/SYSTEM_VARIABLES_ADMIN) or pre-hashed `IDENTIFIED WITH … AS`. Document the residual risk.
2. **Naive `;` splitting** (Infisical) and heuristic `END` detection (OpenBao) both mis-split real-world templates. Use a lexer.
3. **Incomplete revocation** (OpenBao PG default): live sessions survive and owned objects block DROP ROLE. Our ALG-05 fixes this, but it needs elevated rights (`pg_signal_backend`, ownership of objects). Doc 04 must size the Karagöz admin role accordingly.
4. **No native expiry** on MySQL/MSSQL/Mongo/Redis, so a dead Karagöz means credentials live forever. Mitigations: orphan sweep at startup and hourly, uninstall hook, Perde alert when an agent is offline with active leases (lease mirror in Perde).
5. **Bearer key in the PG `user` field** may appear in client-side logs, `~/.pgpass`, shell history, or BI tool configs. Keep the TTL short (≤ 1 h), scope it to one DB, and allow revocation by ID. Never log it at Karagöz (hash only).
6. **Approval hashing on exact bytes** frustrates ORMs that inject comments or vary whitespace. That is accepted by design. Consider an *opt-in* "fingerprint approval" (normalized) for low-risk classes only, and flag it clearly as wider.
7. **Rule regexes on raw SQL** are bypassable (comments, casing, unicode escapes). Prefer matching on parser output. Raw regex is for logging only.
8. **Mock auth** must be timing-consistent, or user/key existence leaks.
9. **Cancel-key mapping** must not allow cross-session cancellation: random keys, bounded map, and removal at session end.
10. **Revocation storms** (offboarding a user with many leases across many DBs) can exhaust connections. Use bounded concurrency per DB.
11. **License contamination**: do not copy code from Vault (BSL), JumpServer/ProxySQL (GPL) or Infisical `ee/` (proprietary). OpenBao (MPL-2.0) files may be reused only with file-level MPL obligations. We use pseudocode only.

### 7.2 Open questions
- **[Unknown]** Exact MySQL 8 / MariaDB pre-hashed `IDENTIFIED … AS` formats Karagöz can compute portably (caching_sha2 includes a salt and rounds; MariaDB `ed25519` plugin). Needs a spike.
- **[Unknown]** Whether Redis `ACL DELUSER` kills existing authenticated connections for that user on all supported versions (6.0 vs 6.2+ vs Valkey 7/8). Needs a spike.
- **[Unknown]** Hoop `libhoop` internals (how it parses extended-protocol messages and where masking hooks sit). Closed source, so we design our own (doc 05).
- **[Unknown]** StrongDM's Cedar action taxonomy for Postgres ("> 180 actions"). The Policy Taxonomy page was not fetched. That would help in choosing Perde's action granularity.
- **[Unknown]** CloudBeaver EE's exact secret-encryption implementation and which auth providers are EE-only at this version. The CE repo only shows the license-denied error code.
- **Open**: should Perde mirror lease state (for an "active credentials" UI and offline-agent alerts) even though Karagöz is the authority? Recommendation: yes, as an eventually consistent mirror, with Karagöz authoritative (doc 05 audit upload path).
- **Open**: the default owned-object policy for write grants: `reassign_to` the DB owner vs `fail`. Needs product input. `reassign` silently transfers user-created tables to the owner, while `fail` leaves an IRREVOCABLE lease.
- **Open**: should Karagöz offer OpenBao as a *backend* (Karagöz asks OpenBao for creds) for orgs that require a central secrets audit trail? That is low effort via the external-source adapter, but it reintroduces standing root creds in OpenBao.
