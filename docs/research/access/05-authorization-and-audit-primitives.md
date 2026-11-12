# 05 — Authorization and Audit Primitives (tokens, policy, query authorization, tamper-evident audit, approvals, masking)

> Research document for the Hacivat product family. This is the **canonical home** of the agent-side authorization and audit algorithms that docs 01–04 cross-reference: agent enrollment & identity, capability-token verification, policy evaluation at Karagöz, query authorization (classifier + server-side enforcement), audit hashing/chaining/batching/gap detection, the access-request state machine, revocation push, and result-stream masking.
>
> Companions: `01-teleport-database-access.md` (ALG-ACCESS-T01…T08: Teleport's role matching, access requests, local audit queue), `02-bytebase.md` (ALG-BYTEBASE-01…06: CEL-conditioned IAM, approval flows, lineage masking, license gating, statement-pinned JIT), `03-other-access-tools-and-secrets-engines.md` (ALG-SECENG-01…09: leases, revocation, PG front-end termination, statement-bound approvals, rule chains), `04-zero-credential-discovery-and-ephemeral-users.md` (ALG-DISC/CRED/EPH: discovery, credential resolution, ephemeral principals).

---

## 1. Scope and sources

### 1.1 Questions answered

1. Which **credential/capability format** should Perde issue so Karagöz can verify it **offline** (P2P data path, Perde possibly unreachable), with attenuation (AI agents), proof-of-possession and fast revocation?
2. Which **authorization model** (RBAC / ABAC / ReBAC) and which **policy language** (Cedar, OPA/Rego, Oso, CEL, OpenFGA/SpiceDB) fit Perde (definition, administration) and Karagöz (evaluation at the edge)?
3. How does Karagöz **authorize individual queries**, combining a SQL safety classifier with authoritative server-side enforcement?
4. How do we make the agent-local **audit log tamper-evident**, ship it to Perde in batches, and detect gaps/truncation/forks?
5. **What to log** (query text vs fingerprint, rows, duration, identities, AI attribution), privacy/PII handling, retention, SIEM export.
6. The **access request / approval** lifecycle as a state machine enforceable at the agent.
7. **Result-stream masking** at Karagöz.

### 1.2 Sources

| Source | Version / date | License | Evidence used |
|---|---|---|---|
| Biscuit specification (`eclipse-biscuit/biscuit/SPECIFICATIONS.md`) | main, fetched 2026-11-11 | Apache-2.0 | Block chain structure, sealed vs attenuable, third-party blocks, revocation ids, Datalog v3.0–v3.3, scopes, Ed25519/P-256 [Observed: https://raw.githubusercontent.com/eclipse-biscuit/biscuit/main/SPECIFICATIONS.md] |
| C2SP `tlog-checkpoint` (+ `signed-note`, `tlog-cosignature`) | main, fetched 2026-11-11 | CC-BY-4.0 | Checkpoint lines, cosignatures, consistency rule [Observed: https://raw.githubusercontent.com/C2SP/C2SP/main/tlog-checkpoint.md] |
| RFC 6962 (Certificate Transparency) §2.1 | 2013 | IETF | Merkle tree hash definition (leaf prefix 0x00, node prefix 0x01), inclusion/consistency proofs [Observed: https://www.rfc-editor.org/rfc/rfc6962.txt §2.1 (`MTH({d(0)}) = SHA-256(0x00 || d(0))`, interior nodes `0x01` prefix), verified 2026-11-11] |
| RFC 9162 (CT v2) | 2021 | IETF | Updated proof algorithms [Inferred] |
| Macaroons (Birgisson et al., NDSS 2014) | 2014 | paper | HMAC-chained caveats, third-party caveats, discharge macaroons [Inferred: well-known] |
| PASETO spec (`paseto-standard/paseto-spec`) v4 | — | — | `v4.public` Ed25519, `v4.local` XChaCha20+BLAKE2b, footer, implicit assertions [Inferred] |
| RFC 7519 (JWT), RFC 8725 (JWT BCP), RFC 7800 (PoP `cnf`), RFC 9449 (DPoP) | — | IETF | JWT pitfalls and PoP claims [Inferred] |
| Cedar (`cedar-policy/cedar`) | — | Apache-2.0 | permit/forbid, default deny, forbid-overrides, schema validation, templates, entity hierarchy [Inferred from Cedar docs/paper "Cedar: A New Language for Expressive, Fast, Safe, and Analyzable Authorization" (OOPSLA 2024)] |
| OPA / Rego; `microsoft/regorus` (Rust Rego) | — | Apache-2.0 / MIT | Policy-as-code, bundles, decision logs [Inferred] |
| Oso (Polar) | library deprecated 2023 in favour of Oso Cloud | Apache-2.0 (legacy lib) | [Inferred] |
| Google Zanzibar (USENIX ATC 2019); OpenFGA (CNCF); SpiceDB (AuthZed) | — | paper / Apache-2.0 / Apache-2.0 | Relation tuples, zookies, consistency [Inferred] |
| CEL (`google/cel-spec`) — as used by Bytebase | — | Apache-2.0 | See doc 02 [Observed in doc 02] |
| OCSF "Datastore Activity" class 6005 | OCSF 1.1–1.3 | Apache-2.0 | Activity IDs Read/Update/Connect/Query/Write/Create/Delete/List/Encrypt/Decrypt (1.3) [Observed: https://docs.tenzir.com/reference/ocsf/1-3-0/classes/datastore_activity] |
| MongoDB audit messages in OCSF format | MongoDB "upcoming" docs | — | Vendor precedent for OCSF DB audit [Observed: https://www.mongodb.com/docs/upcoming/reference/audit-message/ocsf/ (search result)] |
| Teleport audit events, access requests, locks | commit `1283425b…` | AGPL-3.0 | Via doc 01 (prose only) |
| Bytebase IAM/CEL/masking/approvals | commit `d6513882…` | see doc 02 | Via doc 02 |

---

## 2. Executive summary

1. **Two-layer identity:** (a) long-lived *agent identity* (Ed25519 key generated on the host, certified by the org CA in Perde, renewed with proof-of-possession and a monotonic join counter for clone detection); (b) short-lived *session grants* issued per (user, device, database, mode) that the agent verifies **offline**.
2. **Recommended grant format: Biscuit v3** (Ed25519, Datalog v3.3) for session grants, because it gives offline verification, **offline attenuation** (Hacivat can narrow a user grant into a read-only, table-scoped, 10-minute grant for an AI agent without calling Perde), **third-party blocks** (an approval service or an offline "break-glass" key can co-sign), and per-block **revocation identifiers**. [Observed: Biscuit spec] PASETO v4.public is the fallback if Biscuit's complexity is rejected; plain JWT only with a hardened profile.
3. **Proof-of-possession is mandatory:** each grant carries `cnf` = Hacivat's device public key; Karagöz accepts a grant only over a channel whose peer authenticated with that key. Stolen tokens are useless without the device key.
4. **Revocation = short TTL + push:** grants live 5–15 min (auto-refreshed by Hacivat while Perde is reachable), sessions live ≤ policy max; Perde pushes revocations (by grant revocation id, user, device, request id, role, agent) with a monotonically increasing `revocation_epoch`; agent strict mode refuses new sessions when its view is older than `max_staleness`.
5. **Model: RBAC with label selectors + ABAC conditions**, not ReBAC. Resources are agent-discovered databases with labels; roles bind label selectors to modes. ReBAC (Zanzibar/OpenFGA/SpiceDB) solves sharing graphs we do not have and requires online checks the P2P agent cannot make. Perde may still use a ReBAC store *internally* for org/team/project membership.
6. **Policy language: Cedar** (Rust-native, Apache-2.0, default-deny, forbid-overrides, schema-validated, analyzable, **policy templates** map 1:1 onto approved access requests). Perde compiles each agent's relevant policy slice into a signed snapshot; Karagöz evaluates it locally. OPA/Rego is heavier (Go/WASM; Regorus exists in Rust) and less analyzable; CEL (Bytebase) is an expression language, not a policy system.
7. **Agent-side floors:** host operator can set local `forbid` policies (e.g. `hacivat.readonly=true` label, `/etc/karagoz/policy.d/*.cedar`) that Perde cannot override — limits the blast radius of a Perde compromise.
8. **High-risk grants require a second signer:** write/admin/unmasked grants can be configured to require a Biscuit third-party block from an org **approval key** that Perde's issuing key cannot forge (separate HSM/offline key or a second admin's device key).
9. **Query authorization = policy mode ∧ classifier verdict ∧ server enforcement.** The classifier (parser-based, per dialect) is a UX/early-rejection and attribution layer; the *authoritative* boundary is the DB principal's privileges (doc 04) plus read-only transactions. Unknown/unparseable statements are denied in read-only mode.
10. **Classifier must treat as writes:** data-modifying CTEs, `SELECT … FOR UPDATE/SHARE`, `SELECT INTO`, `EXPLAIN ANALYZE <dml>`, `COPY … FROM`, `CALL`/`DO`/`EXECUTE`, side-effecting functions (`nextval`, `setval`, `pg_terminate_backend`, `lo_import`, `dblink_exec`, `set_config`), MySQL `INTO OUTFILE`, `LOAD DATA`, `HANDLER`, Mongo `$out`/`$merge`, Redis write/dangerous categories.
11. **Audit chain:** every record `h_i = SHA-256(domain ‖ h_{i-1} ‖ seq_i ‖ canonical(record_i))`, per agent stream; every N records or T seconds a **signed checkpoint** (C2SP `tlog-checkpoint` style: origin, size, RFC 6962 Merkle root) by the agent key; Perde verifies continuity and **cosigns** (witness) — the agent stores Perde's cosignature, so neither side can later rewrite history alone.
12. **Gap detection** is by `(stream_id, seq)` continuity, prev-hash linkage and checkpoint consistency proofs; Perde distinguishes *pending* (agent offline), *gap* (missing seq), *fork* (two different records for one seq) and *truncation* (checkpoint size decreased / stream restarted without `stream.rotate` record).
13. **Privacy-preserving chaining:** commit to sensitive fields by salted hash inside the chained record and store the cleartext (query text, literals, sample rows) in a separate, deletable "payload" — erasure (GDPR/KVKK) deletes payload + salt without breaking chain verification.
14. **What to log by default:** session start/end, every statement's normalized fingerprint + classifier verdict + decision + rows affected/returned + duration + error code + principal + DB-native user + AI attribution; full query text **on by default but literal-redacted** for tables labelled `pii`; result data never by default.
15. **AI attribution** is first-class: `actor.kind ∈ {human, ai_agent, automation}`, MCP client name/version, model id if known, delegation chain (Biscuit attenuation blocks), `prompt_ref` (opaque id, never the prompt by default), and separate policy (`agent_access`).
16. **SIEM export:** OCSF Datastore Activity (class 6005) JSON as the canonical export shape, plus syslog RFC 5424/CEF and HTTPS/S3 sinks; exports carry chain metadata so SIEM-side verification is possible.
17. **Access requests** are a state machine (`pending → approved|denied|expired|cancelled`, `approved → active → completed|revoked|expired`) whose *output* is a Cedar template-linked policy + request id embedded in grants; Karagöz enforces by checking request id validity against the pushed revocation set.
18. **Masking at Karagöz on the result stream** using column lineage from the parser; fail closed (mask or reject) when lineage cannot be determined for a column that may derive from a sensitive source; masking functions include HMAC-based deterministic pseudonyms (joinable within an org, not reversible).
19. **Fail-closed rules:** audit queue full → new statements denied (configurable to "degraded" for read-only); policy snapshot too stale (strict mode) → no new sessions; clock untrusted → grants with tight skew only.

---

## 3. Detailed findings

### 3.1 Requirements derived from the architecture

| Requirement | Consequence |
|---|---|
| Data path is P2P (Hacivat ↔ Karagöz); Perde may be unreachable mid-session | Authorization must be decidable at Karagöz with local state only: verifiable tokens + cached signed policy. |
| Karagöz is OSS and runs on customer hosts | No secret shared with Perde can be relied on for verification → **asymmetric** signatures (rules out HMAC-only Macaroons as the primary format). |
| AI agents act through Hacivat's local MCP server | Need **delegation with attenuation** done locally by Hacivat. |
| Perde self-hostable + hosted | Same token format and verification in both; org CA per Perde org; hosted Perde must not hold DB credentials. |
| Audit recorded at Karagöz first | Durable local store; tamper evidence that survives a malicious later Karagöz/host compromise *for records already checkpointed and cosigned*. |
| Users may be offline from Perde while connected P2P | Grant refresh needs Perde; sessions should survive brief Perde outages (bounded by grant expiry + grace). |

### 3.2 Capability token formats compared

| Property | JWT (JWS, EdDSA) | PASETO v4.public | Macaroons | Biscuit v3 | X.509 short-lived cert | SSH cert |
|---|---|---|---|---|---|---|
| Offline verification with public key | Yes | Yes | **No** (HMAC root key needed by verifier) | Yes | Yes | Yes |
| Holder-side attenuation offline | No (re-issue) | No | **Yes** (append caveat, HMAC chain) | **Yes** (append block signed by ephemeral key chain) | No (can't sub-issue without CA bit) | No |
| Third-party / co-signed conditions | No (nested JWS ad hoc) | No | Yes (third-party caveats + discharge macaroons) | **Yes** (third-party blocks with external signature) | No | No |
| Expressive authorization logic in token | Claims only | Claims only | Caveat predicates (app-defined strings) | **Datalog** facts/rules/checks | Extensions (ad hoc) | Principals, critical options, extensions |
| Revocation handle | `jti` | `jti` in claims | caveat id | **per-block revocation id** (signature bytes) | serial | key id / serial |
| Algorithm agility pitfalls | `alg` header confusion, `none`, `kid` injection (RFC 8725) | **Versioned** — no alg header choice | Fixed HMAC-SHA256 | Ed25519 or P-256 per block | Mature | Mature |
| Size | ~300–800 B | ~300–600 B | ~200–500 B | ~400 B–2 KB (grows with blocks) | ~600 B–1.5 KB | ~500 B–1 KB |
| Rust ecosystem | `jsonwebtoken`, `josekit` | `rusty_paseto`, `pasetors` | `macaroon` (thin) | `biscuit-auth` (official, Apache-2.0) | `rcgen`, `rustls` | `ssh-key` |
| Fit for TLS/QUIC channel auth | via app layer | via app layer | via app layer | via app layer | **native mTLS** | SSH only |

Evidence: Biscuit's attenuable vs sealed tokens, third-party blocks requiring datalog ≥ v3.2, revocation ids derived from block signatures, scope annotations (`trusting authority|previous|<pubkey>`), check kinds (`check if`, `check all` v3.1, `reject if` v3.3) and authorizer `allow if`/`deny if` policies evaluated in order with default fail. [Observed: Biscuit SPECIFICATIONS.md] Macaroons' verifier must know the root HMAC key — fatal for an OSS agent that should not hold a Perde secret capable of minting tokens. [Inferred from NDSS 2014 construction] JWT's well-known failure modes are `alg` confusion and `none`, `kid`/`jku` header injection, missing `aud` checks. [Inferred: RFC 8725]

**Decision recommended:**

- **Agent identity:** X.509 (Ed25519) issued by the org CA → used directly in mTLS/QUIC handshakes with Perde and to sign audit checkpoints. Lifetime 24 h, renewed at 50 %. (Teleport uses short-lived certs for all identities — doc 01.)
- **Hacivat device identity:** Ed25519 key generated in the OS keystore (macOS Keychain / Secure Enclave via P-256 if hardware-bound is required → Biscuit supports P-256 blocks [Observed]); enrolled once with Perde; used as `cnf` key.
- **Session grant:** Biscuit v3 token, authority block signed by the **org grant-issuing key** (distinct from the CA key; rotated quarterly; public keys distributed to agents in the policy snapshot with `kid`).
- **Fallback if Biscuit is rejected:** PASETO `v4.public` with a strict claim set, and implement AI delegation as a *second* PASETO signed by Hacivat's device key chained via `parent_jti` + `cnf` — doable but reimplements half of Biscuit.

### 3.3 Grant contents (Biscuit authority block — fact vocabulary)

Facts are our own vocabulary; Karagöz's authorizer only trusts facts from the authority block (default scope) and from explicitly trusted third-party keys. [Observed: Biscuit default scope = authority + current block + authorizer]

```
// authority block (signed by org grant key, kid in rootKeyId)
org("org_7f3a");
user("usr_91c2");                 // Perde user id
device("dev_44aa", "ed25519/…");  // cnf: device public key
agent("agt_0b12");                // audience: exactly one agent
database("agt_0b12", "db_orders");// db stable id (from doc 04 inventory)
mode("read");                     // read | write | ddl | admin
db_scope("app", "*");             // (database/schema, table) patterns; optional
mask_profile("default");          // masking profile id; "none" requires unmask right
policy_epoch(4812);               // snapshot version used to issue
request("req_55e0");              // optional: access request id
mfa("2026-11-11T14:02:00Z", "webauthn");
device_trust("managed");
actor_kind("human");
check if time($t), $t <= 2026-11-11T14:17:00Z;        // exp
check if time($t), $t >= 2026-11-11T14:01:30Z;        // nbf (with skew)
check if channel_peer_key($k), $k == "ed25519/…";     // proof of possession
check if agent_id($a), $a == "agt_0b12";              // audience
```

AI attenuation block appended by Hacivat (no Perde round trip):

```
// block 1 (signed by holder's ephemeral key chain; cannot add rights, only checks)
actor_kind_claim("ai_agent");          // informational, NOT trusted for rights
check if operation($op), ["read"].contains($op);
check if table($db, $t), $db == "app", ["orders", "customers"].contains($t);
check if time($t), $t <= 2026-11-11T14:10:00Z;
check if mcp_client($c), $c == "claude-desktop";   // authorizer provides fact from MCP handshake
```

Because blocks can only add **checks** that must all pass (facts from non-authority blocks are not trusted by default), attenuation can only *reduce* authority. [Observed: Biscuit spec — checks in any block must succeed; default trust scope] Karagöz must take `actor.kind` for audit **from the presence of attenuation blocks declaring it plus the channel metadata**, never trust a block to *raise* rights.

Third-party approval block (for write/admin/unmask when the org enables "dual control"):

```
// block 2: external signature by approval key ed25519/APPROVER_PUB
approved("req_55e0", "usr_12ab", 2026-11-11T14:00:10Z);
```

Authorizer policy (in Karagöz snapshot):

```
check if mode("read") or approved($r, $who, $at) trusting ed25519/APPROVER_PUB, request($r);
```

### 3.4 Proof-of-possession and channel binding

- Hacivat and Karagöz establish the P2P channel (QUIC or TLS 1.3 over the hole-punched path; see `networking/`) with **mutual public-key authentication**: Karagöz presents its agent cert (chain to org CA, pinned by Hacivat from Perde's org metadata); Hacivat presents its device key (raw public key or self-signed cert).
- Karagöz injects `channel_peer_key(<hacivat device pubkey>)` into the authorizer; the grant's `check if channel_peer_key(...)` must succeed. Additionally bind the grant to the channel with a TLS exporter value (RFC 5705/8446 `exporter("hacivat-grant", 32)`) signed by the device key in the grant-presentation message → defeats relaying a grant over a different channel even if both device keys were the same machine. [Inferred design; DPoP RFC 9449 analogy]
- Relay fallback (Perde relay) carries ciphertext only; channel auth is end-to-end, so relay compromise yields nothing.

### 3.5 Revocation

| Mechanism | Latency | Works offline? | Notes |
|---|---|---|---|
| Short grant TTL (5–15 min) | ≤ TTL | Yes | Hacivat refreshes at 2/3 TTL; refresh is where Perde re-evaluates policy, MFA freshness, device trust. |
| Push revocation list over agent control channel | seconds | No (needs channel) | Entries: `{type: grant_revocation_id|user|device|request|role|agent|policy_epoch_min, value, until}`; `revocation_epoch` monotonically increasing; agent acks epoch. |
| Session kill on revocation | seconds | — | Agent terminates active sessions matching the entry (Teleport "locks" analogue — doc 01). |
| Strict staleness | — | — | If agent hasn't synced revocations for > `max_staleness` (default 5 min hosted, configurable), refuse **new** sessions; optionally terminate existing write sessions. |
| Policy epoch floor | minutes | Partial | Perde can set `min_policy_epoch`; grants issued under older epochs rejected (mass invalidation after a policy fix). |

Revocation entries expire at the max lifetime of anything they could match (e.g. a grant revocation entry lives until that grant's `exp`), so the list stays small. Biscuit revocation ids: one per block (its signature) — revoking the authority block's id kills the grant and all attenuated children; revoking an AI attenuation block's id kills only the AI's delegated token. [Observed: revocation id = block signature]

### 3.6 Authorization models: RBAC vs ABAC vs ReBAC for Perde

| Model | What it expresses well | Fit for Perde/Karagöz |
|---|---|---|
| RBAC (roles → permissions on resource sets) | Org admin mental model; audit-friendly; Teleport roles, Bytebase roles | **Core.** Roles reference resources by **label selectors** (Teleport `db_labels`), not lists. |
| ABAC (attributes of subject/resource/context in conditions) | Time windows, MFA freshness, device trust, IP, request ids, data sensitivity labels, AI actor kind | **Needed** as conditions on role bindings (Bytebase uses CEL conditions on bindings — doc 02). |
| ReBAC (relationship tuples, Zanzibar) | Sharing graphs ("viewer of folder ⇒ viewer of doc"), nested groups at scale | Overkill for DB access; requires an online check service (Zanzibar zookies for consistency) — conflicts with offline agent. Possible *inside* Perde for org→team→project membership, compiled down to role bindings. |

Conclusion: **RBAC + label selectors + ABAC conditions**, compiled into a per-agent signed snapshot. Teleport's model (allow/deny, deny-overrides, label matching, traits) and Bytebase's (role binding + CEL condition with expiry) are both instances. [Observed: docs 01, 02]

### 3.7 Policy languages compared

| | Cedar | OPA / Rego | Oso / Polar | CEL | OpenFGA / SpiceDB |
|---|---|---|---|---|---|
| Kind | Policy language + engine | General policy-as-code (Datalog-ish) | Policy language (logic) | Expression language | ReBAC service + modelling DSL |
| Runtime | Rust crate (`cedar-policy`) | Go; WASM; Rust port `regorus` | Rust core, library deprecated (Oso Cloud) | Go/Java/C++/Rust (`cel-interpreter`) | Go servers |
| Default semantics | default deny; any `forbid` overrides `permit` | whatever you write | whatever you write | n/a | relation check |
| Static validation | Schema-based type checking; analyzable (SMT) | Optional type checking (newer) | Limited | Type checking with declarations | Model validation |
| Templates | **Policy templates** with `?principal`/`?resource` slots | No (data-driven instead) | No | No | Tuples |
| Edge evaluation | Embedded, µs per request | Embedded via WASM/regorus | n/a | Embedded | Requires service or local tuple copy |
| Explainability | Returns determining policy ids | Decision logs | — | — | Expand API |
| Risk | Smaller ecosystem | Expressive → hard to audit; perf pitfalls | Deprecated | Not a policy system | Online dependency |

**Recommendation: Cedar in both Perde and Karagöz.** Map:

- `principal`: `User::"usr_…"` with parents `Group::"…"`, `Role::"…"`; `Device`; `AiAgent::"…"` (parent = the user it acts for).
- `action`: `Action::"connect"`, `"query.read"`, `"query.write"`, `"query.ddl"`, `"query.admin"`, `"export"`, `"unmask"`, `"approve"`.
- `resource`: `Database::"agt_0b12/db_orders"` with attributes `labels` (record), `engine`, `env`; child `Table::"…"` when object-level policies exist.
- `context`: `mfa_age_s`, `device_trust`, `actor_kind`, `request_id`, `time`, `classifier` verdict, `client_ip_class` (`p2p|relay`).

Example policies (our own):

```cedar
// Role "analyst": read prod non-PII databases, MFA within 12 h
permit (principal in Role::"analyst", action == Action::"query.read", resource)
when { resource.labels.env == "prod" && !(resource.labels has pii && resource.labels.pii == "true")
       && context.mfa_age_s < 43200 };

// AI agents never write, regardless of user role
forbid (principal, action in [Action::"query.write", Action::"query.ddl", Action::"query.admin"], resource)
when { context.actor_kind == "ai_agent" };

// Access-request template (linked on approval)
permit (principal == ?principal, action == Action::"query.write", resource == ?resource)
when { context.request_id == "req_55e0" && context.time < 1762870000 };
```

Agent-local floor (operator file `/etc/karagoz/policy.d/floor.cedar`, merged as `forbid` only):

```cedar
forbid (principal, action in [Action::"query.write", Action::"query.ddl", Action::"query.admin"], resource)
when { resource.labels has readonly && resource.labels.readonly == "true" };
```

Biscuit vs Cedar division of labour: the **grant** (Biscuit) carries *who/what/where/until* and attenuations; the **snapshot policy** (Cedar) decides per statement with richer context. Karagöz requires both to allow. This duplication is deliberate: Biscuit checks bound the grant even if the snapshot is stale; Cedar applies per-statement and org-wide forbids (e.g., new forbid pushed after grant issuance).

### 3.8 Query authorization: classifier + server enforcement

Three layers (doc 04 §3.14 for the server side):

1. **DB principal privileges** (ephemeral user of the tier) — authoritative.
2. **Session/transaction mode** — PG `default_transaction_read_only` + `BEGIN READ ONLY` wrapper per statement batch in read mode; MySQL `START TRANSACTION READ ONLY`; ClickHouse `readonly=1`.
3. **Karagöz classifier** — parser-based classification per dialect (sqlparser-rs / pg_query for PG; see `clients/` research), producing: statement kind, read/write/ddl/dcl/admin/txn/session category, referenced objects (for object-level policy + masking lineage), functions called, risk flags.

Classification table (non-exhaustive, our own):

| Construct | Category | Why |
|---|---|---|
| `SELECT` (plain), `SHOW`, `DESCRIBE`, `EXPLAIN` (without ANALYZE), `VALUES`, `TABLE t` | read | — |
| `WITH … AS (INSERT/UPDATE/DELETE …) SELECT …` (PG) | write | data-modifying CTE |
| `SELECT … FOR UPDATE/NO KEY UPDATE/SHARE/KEY SHARE` | write-lock | takes row locks; can block prod |
| `SELECT … INTO new_table` (PG/MSSQL) | ddl+write | creates table |
| `EXPLAIN ANALYZE <stmt>` | category of `<stmt>` | executes it |
| `SELECT nextval()/setval()`, `pg_advisory_lock`, `pg_terminate_backend`, `pg_cancel_backend`, `set_config(…, false)`, `lo_import/lo_export`, `dblink_exec`, `pg_read_file` | write/admin | side-effecting functions; allowlist pure functions instead of blocklisting |
| user-defined functions in SELECT | unknown→ resolve via catalogue: `provolatile = 'v'` + not in allowlist → write-risk | volatile UDFs can write |
| `COPY … TO STDOUT` | read (export action) | — |
| `COPY … FROM` | write | — |
| `CALL`, `DO`, `EXECUTE`, `PREPARE` (of write) | write/unknown | procedural |
| `SET`/`RESET`/`SET ROLE`/`SET SESSION AUTHORIZATION`/`RESET ROLE` | session; **role switching denied** | identity switching (doc 01 blocklist) |
| `BEGIN/COMMIT/ROLLBACK/SAVEPOINT` | txn | allowed; `BEGIN READ WRITE` in read mode denied |
| `LISTEN/NOTIFY` | notify=write-ish | side effect |
| MySQL `SELECT … INTO OUTFILE/DUMPFILE`, `LOAD DATA`, `HANDLER … READ` (read), `DO`, `LOCK TABLES` | write/admin/read/lock | — |
| MySQL `COM_CHANGE_USER`, PG `startup` re-auth | denied | identity switching |
| MSSQL `EXEC`, `sp_executesql`, `OPENROWSET`, `xp_cmdshell`, `EXECUTE AS`, `REVERT` | admin/unknown/denied | — |
| Mongo `find`, `aggregate` without `$out/$merge`, `count`, `distinct`, `listCollections` | read | — |
| Mongo `aggregate` with `$out`/`$merge`, `insert/update/delete/findAndModify`, `createIndexes`, `drop` | write/ddl | — |
| Redis per ACL categories (`@read`, `@write`, `@admin`, `@dangerous`, `@scripting`) | map 1:1 | Redis already classifies commands [Inferred: `COMMAND INFO` exposes ACL categories] |
| Multi-statement batch | max category over all statements | `SELECT 1; DROP TABLE x` |
| Parse failure / unsupported syntax | **unknown** | deny in read mode; allow in admin mode with audit flag |
| Comments, hints (`/*+ … */`), Unicode homoglyphs, `\` escapes | parse normally; never regex | regex classifiers are bypassable |

Decision (per statement): `allow` iff Cedar(principal, action = category→action, resource/table, context) = permit **and** grant checks pass **and** (mode=read ⇒ category ∈ {read, txn-ro, session-safe}) ; the statement is then executed by a principal that *cannot* do more than the mode anyway. Disagreement between classifier and DB (classifier says read, DB says `42501 permission denied` or `25006 read-only transaction`) is logged as `classifier_miss` — a valuable signal.

### 3.9 Audit: what to log

Record types (namespace aligned with doc 01 §5.3):

| Type | Key fields |
|---|---|
| `agent.start` / `agent.stop` / `stream.rotate` | agent version, config hash, policy epoch, prev stream id |
| `db.session.start` | session id, user id, device id, actor (kind, mcp client, model id?, delegation chain revocation ids), grant revocation id, request id, db id, engine, DB principal (ephemeral name), mode, channel (p2p/relay, peer addr class), MFA age |
| `db.session.end` | reason (client_close, ttl, revoked, idle, agent_shutdown, target_disappeared), counts (stmts, denied, rows), bytes |
| `db.query` | session id, seq-in-session, statement fingerprint (normalized, literals → `?`), fingerprint hash, **text commitment** (salted hash) + payload ref, classifier category + flags, decision (allow/deny + determining policy ids), objects referenced, rows returned/affected, duration (queue, exec, stream), error SQLSTATE/code, masked columns, truncated flag, transaction id |
| `db.query.denied` | same as above without execution fields + deny reason code |
| `db.result.export` | format, rows, bytes, destination class (file, clipboard, MCP), masking profile |
| `db.user.provisioned / deactivated / deleted / orphan_cleaned` | DB principal, tier, grants hash (doc 04 ALG-EPH) |
| `access.request.*` | Perde-side, mirrored to agent when it affects grants |
| `policy.snapshot.applied`, `revocation.applied` | epoch, hash |
| `audit.checkpoint` | size, root, signature (also a record itself) |
| `audit.degraded` / `audit.recovered` | queue state |

AI attribution fields: `actor.kind`, `actor.on_behalf_of` (user), `actor.client` (MCP client name/version from MCP `initialize`), `actor.model` (if Hacivat knows it; self-reported → mark `asserted`), `actor.tool` (MCP tool name, e.g. `run_query`), `actor.prompt_ref` (opaque id that lets the user find the conversation locally; prompt text never leaves the device by default), `actor.delegation` (list of Biscuit block revocation ids).

### 3.10 Privacy, PII and retention

- **Literal redaction:** fingerprints replace literals with `?`; full text stored only in the payload store, optionally literal-redacted for resources labelled `pii=true` or when classifier detects PII-like literals (emails, IBAN incl. Turkish `TR` IBANs, TCKN 11-digit national ID with checksum, phone numbers, credit cards with Luhn). TCKN validation (checksum on digits 10 and 11) reduces false positives. [Inferred: TCKN algorithm public]
- **Result data** is never logged by default; optional sampled result logging applies masking first.
- **Erasure:** chained record stores `text_commit = SHA-256(salt ‖ text)`; payload store holds `(salt, text)` keyed by record hash; deleting the payload satisfies erasure while the chain still verifies (the commitment reveals nothing without the salt). Same for `actor.prompt_ref`.
- **Retention:** policy per org: hot (Perde DB, searchable) default 90 days, cold (object storage, compressed, chain-verified) default 1 year (PCI DSS 10.5.1 asks 12 months with 3 months immediately available [Inferred]); payloads can have shorter retention than metadata. Agent-local retention: until acked by Perde + `local_keep` (default 7 days) for forensic overlap.
- **Jurisdiction:** KVKK (Türkiye) and GDPR both treat audit logs containing personal data as personal data processing; logs are justified under legitimate interest/security obligations but need minimisation and retention limits → defaults above. [Inferred; not legal advice]
- **Access to audit logs** is itself audited (`audit.viewed`, `audit.exported`).

### 3.11 Tamper evidence: options

| Technique | Detects | Cost | Notes |
|---|---|---|---|
| Hash chain (each record includes prev hash) | modification/deletion/reordering *after* a trusted anchor | O(1) per record | Truncation of the tail is undetectable without an external anchor. |
| Signed checkpoints by agent key | tail truncation after checkpoint; provenance | 1 signature per checkpoint | A compromised agent can still re-sign a rewritten history unless the old checkpoint was already seen elsewhere. |
| External witness (Perde cosigns) | rewriting after witness; forks | 1 round trip per checkpoint | C2SP cosignature model; consistency rule: never sign inconsistent checkpoints. [Observed: tlog-checkpoint] |
| Merkle tree (RFC 6962) over records | efficient inclusion proofs for single records; consistency proofs between checkpoints | O(log n) proofs | Enables "prove this query was logged" without revealing neighbours; allows Perde to keep only roots + selected records. |
| Transparency-log style public log | equivocation by Perde itself | high | Optional for hosted Perde: publish org checkpoints to a public witness network — likely overkill v1. |
| WORM storage (S3 Object Lock) | deletion at rest | storage cost | Complementary for exports. |

Recommendation: **hash chain + RFC 6962 Merkle roots in signed checkpoints + Perde cosignature**; Perde stores full records; agent keeps cosigned checkpoints. Verification tool (`karagoz audit verify`, `perde audit verify`) is OSS.

### 3.12 SIEM export

- Canonical export: **OCSF Datastore Activity (class_uid 6005)**; activity ids in 1.3: Read, Update, Connect, Query, Write, Create, Delete, List, Encrypt, Decrypt; `type_uid = class_uid*100 + activity_id`. [Observed: Tenzir OCSF 1.3.0 docs] Map: `db.session.start` → Connect; `db.query` read → Query/Read; write → Write/Update/Delete by statement kind; ddl → Create/Delete; extensions under `unmapped`/profile for chain fields (`hacivat.seq`, `hacivat.hash`, `hacivat.checkpoint`).
- Transports: HTTPS webhook (HMAC-signed body), Splunk HEC, syslog (RFC 5424 over TLS, RFC 5425), S3/GCS/Azure Blob JSONL (gzip) with Object Lock, OpenTelemetry Logs (OTLP). Exactly-once is impossible; at-least-once with record `uid` = `stream_id:seq` for SIEM dedupe.
- Export happens from Perde (aggregated). Self-hosters without Perde-side SIEM can also configure **agent-direct export** (for air-gapped setups) — same format.

### 3.13 Access requests / JIT

Observed designs: Teleport access requests (roles or resources, thresholds, reviewers, `max_duration`, request ids embedded in certificates — doc 01 ALG-ACCESS-T04); Bytebase statement-pinned JIT grants and approval flows by risk level (doc 02 ALG-BYTEBASE-03/06); Hoop.dev/StrongDM per-query reviews (doc 03 ALG-SECENG-08). Perde's design (ALG-REQ-01) supports three request kinds:

1. **Role/mode elevation** for a database set for a duration (Teleport-like).
2. **One-shot statement approval** bound to exact statement bytes (hash) + target + expiry (Bytebase/Hoop-like) — ideal for AI agents proposing a write.
3. **Unmask/export** approvals.

### 3.14 Masking at the result stream

Bytebase computes masking by column lineage from its parser and applies masking algorithms server-side before returning results (doc 02 ALG-BYTEBASE-04). For Karagöz the natural place is the result encoder after the DB driver and before the P2P channel; exports and MCP tool outputs go through the same path. Masking is defense-in-depth for *display*; it does **not** prevent inference via predicates (`WHERE salary > 100000`) or aggregates — document as a known limitation, optionally forbid predicates/aggregates on masked columns in strict profiles.

---

## 4. Algorithm specifications

Shared notation: `H = SHA-256`; `‖` = concatenation of length-prefixed byte strings (u32 big-endian length ‖ bytes) unless stated; `Sign_k`/`Verify_K` = Ed25519; all timestamps RFC 3339 UTC with millisecond precision in records, Unix seconds in tokens; `skew` = 60 s default.

### ALG-AGENT-01: Agent enrollment, identity issuance and renewal

- **Problem:** Install Karagöz with one command, give it a cryptographic identity bound to the host, let it renew without secrets on disk that can be cloned unnoticed, and let Perde detect clones and revoke.
- **Inputs / outputs:** In: one-time join token `J` (from Perde UI: `{token_id, secret 256-bit, org_id, allowed_labels, expires_at (≤ 1 h), uses=1}`), Perde endpoint URL + **pinned org CA fingerprint** (both embedded in the install command), host facts. Out: agent keypair `(k_a, K_a)`, agent cert `C_a` (24 h), `agent_id`, join state `{generation g, last_renew_nonce}`, Perde trust bundle.
- **Algorithm:**
  1. Installer downloads the binary, verifies signature (minisign/cosign) against a key compiled into the installer script and the version manifest signed by the release key. Abort on mismatch.
  2. Agent generates `(k_a, K_a)` (Ed25519) locally — in TPM 2.0 / `systemd-creds` sealed storage when available, else file `0600` owned by `karagoz` user in `/var/lib/karagoz/identity/`.
  3. Agent opens TLS to Perde, **verifying Perde's server cert chains to a CA whose fingerprint equals the pinned one** (TOFU avoided).
  4. Agent sends `Enroll{token_id, proof = HMAC(secret, "enroll" ‖ K_a ‖ nonce_perde), K_a, host facts (hostname, machine-id hash, OS, agent version, container runtime summary)}` after fetching `nonce_perde` (anti-replay).
  5. Perde verifies token unused & unexpired & org match, marks it consumed **atomically** (single-use), creates `agent_id`, sets `generation = 1`, issues `C_a` (Subject `agent_id`, SAN URI `spiffe://<org>/agent/<agent_id>` style, EKU clientAuth+serverAuth, validity 24 h) and returns trust bundle (org CA certs, grant-issuing public keys with `kid`, approval keys, current policy snapshot pointer, revocation epoch).
  6. Agent persists `C_a`, `generation`, bundle; starts control channel (mTLS with `C_a`).
  7. **Renewal** at 50 % lifetime: agent sends `Renew{agent_id, generation g, nonce_perde, Sign_{k_a}("renew" ‖ agent_id ‖ g ‖ nonce_perde)}`; Perde checks `g == stored_generation` → issues new cert, sets `stored_generation = g+1`, agent persists `g+1` **before** using the new cert (write-ahead; if crash after Perde increments but before persist, next renew presents `g` and Perde accepts `g == stored-1` once only if `nonce` chain proves continuity — see edge 3).
  8. **Clone detection:** if two processes renew with the same key, generation diverges: one presents a stale `g` → Perde **locks** the agent (`agent.lock reason=generation_mismatch`), pushes revocation, alerts admins; recovery requires re-enrollment with a new join token.
  9. **Recovery after long downtime** (cert expired): renewal still permitted with the bound key + generation within `max_offline` (default 30 days); beyond that, re-enroll.
  10. **CA rotation:** Perde publishes new CA in bundle (both trusted) ≥ 2 renewal cycles before switching issuance; agents and Hacivat accept either during overlap.
- **Data structures:** `agents(agent_id, org_id, public_key, generation, status ∈ {active, locked, decommissioned}, last_seen, labels, version)`; join tokens table with `consumed_at`.
- **Complexity:** negligible.
- **Edge cases (exhaustive list):** (1) token reused → reject, alert (possible leak); (2) token expired between download and enroll → clear error, ask for new command; (3) crash between Perde increment and local persist → Perde accepts `g = stored-1` once if accompanied by signature over the last issued cert's serial (proves the agent received it) — otherwise lock; (4) VM snapshot restore → older generation → lock (true positive: restored image can be a clone); provide "I restored from snapshot" re-enroll flow; (5) host renamed → facts update, no identity change; (6) clock wrong on host (cert not yet valid) → agent reports `clock_skew` using Perde's `Date` header; (7) Perde pinned CA changed legitimately (self-hosted reinstall) → agent refuses; operator re-runs install command; (8) proxy/MITM corporate TLS inspection → pin fails by design; support `--perde-ca-file`; (9) container deployment where `/var/lib/karagoz` is ephemeral → must mount a volume, else every restart re-enrolls (installer warns); (10) two agents on the same host for different orgs → separate state dirs and users; (11) agent decommissioned in Perde → renew returns `decommissioned`, agent wipes key and exits; (12) TPM unavailable → file key, report `key_protection=file` to Perde (shown in UI).
- **Test cases:** fresh enroll → active, gen 1; second enroll with same token → rejected; renew ×3 → gen 4; copy state dir to another VM and run both → second renew locks agent; restore snapshot of gen 2 after gen 4 → lock; expired cert after 10 days offline → renew OK; after 40 days → re-enroll required.
- **Prior art and evidence:** Teleport bound-keypair join with join-state counters and recovery (doc 01 §5.1 table, "Bound-keypair join…"); SPIFFE/SPIRE node attestation [Inferred]; Tailscale node keys + machine keys [Inferred; see `networking/`].
- **Used by:** Karagöz, Perde.

### ALG-AUTHZ-01: Session grant verification at the agent (offline)

- **Problem:** Decide whether a presented grant authorises opening a session on a given database, without contacting Perde.
- **Inputs / outputs:** In: grant bytes `G`, channel facts `{peer_key, tls_exporter, channel_kind p2p|relay}`, presentation signature `σ = Sign_{device}(exporter ‖ H(G))`, requested `{db_id, mode}`, local state `{trusted root keys by kid, approval keys, revocation set R with epoch, policy snapshot P with epoch, agent_id, now}`. Out: `Accept(SessionContext)` | `Reject(code)`.
- **Algorithm:**
  1. Size check (`|G| ≤ 16 KiB`, ≤ 8 blocks). Parse Biscuit; read `rootKeyId` → root key; unknown kid → `unknown_issuer` (trigger bundle refresh asynchronously).
  2. Verify the block signature chain and proof (Biscuit verification). Reject sealed/unsealed according to policy (attenuable allowed; AI tokens may be sealed by Hacivat to stop further delegation).
  3. Verify `σ` with the device key from the authority fact `device(_, key)` and check `key == peer_key` (channel PoP).
  4. Compute revocation ids of all blocks; if any ∈ R → `revoked`. Also check fact-level revocations: user, device, request, agent, role (via authority facts) against R.
  5. If `policy_epoch(e)` in grant `< P.min_policy_epoch` → `stale_grant`. If `e > P.epoch` → fetch snapshot (if reachable) else in strict mode `agent_policy_stale` / in lenient mode continue (grant checks still bound it).
  6. Build authorizer: facts `time(now)`, `agent_id(self)`, `channel_peer_key(peer_key)`, `channel_kind(kind)`, `requested_db(db_id)`, `requested_mode(mode)`, `mcp_client(c)` (if session opened by Hacivat's MCP bridge, from Hacivat-signed session metadata), plus agent-wide checks: `check if database($a, $d), agent_id($a), requested_db($d)`; `check if mode($m), mode_allows($m, requested_mode)`; policies `allow if true` after checks (Biscuit authorizer policies), with Datalog run limits (max facts 1 000, max iterations 100, max time 1 ms). [Observed: Biscuit authorizer checks/policies semantics]
  7. Run authorizer; failing check → `grant_check_failed{index}` (record which check, not the full token, in audit).
  8. Extract `SessionContext{user, device, org, db, mode (min of authority mode and attenuations' allowed ops), scope, mask_profile, request ids, mfa, actor_kind (ai_agent if any block declares it or MCP bridge flag), delegation revocation ids, exp = min(all time checks)}`.
  9. Run ALG-AUTHZ-02 for `Action::"connect"`; if permit → `Accept`.
  10. Schedule session termination at `exp` unless renewed by a fresh grant presented in-band (Hacivat sends refreshed grant; same user/device/db; steps 1–9 again; mode may only decrease mid-session unless reconnect).
- **Data structures:** `R` as hash sets per type + epoch; LRU cache of verified grants keyed by `H(G)` (TTL = remaining validity) to avoid re-verifying per statement.
- **Complexity:** Ed25519 verify ~50 µs per block; Datalog small; total < 1 ms.
- **Edge cases:** clock skew (apply `skew` to nbf only, not exp, configurable); agent clock badly wrong (> 5 min vs Perde's last `Date`) → strict mode rejects all grants with `agent_clock_untrusted`; grant for another agent → `wrong_audience`; grant replay on a different channel → PoP fails; relay channel when policy requires p2p → Cedar context forbids; Biscuit with unknown Datalog version → reject (spec mandates); grant with attenuation block that *adds facts* like `mode("admin")` → ignored because non-authority facts aren't trusted (test it explicitly); revocation set unknown (fresh agent, never synced) → reject all until first sync; extremely long-lived grants (misconfigured Perde) → agent cap `max_grant_lifetime` (default 24 h) regardless of token; P-256 device keys (Secure Enclave) → supported; multiple concurrent sessions per grant → allowed up to `max_sessions_per_grant` (default 8).
- **Test cases:** valid grant → accept; flip one byte → signature failure; valid grant presented by another device key → PoP fail; AI block restricting to `orders` → session accepts, statement on `customers` later denied in ALG-AUTHZ-03; revoked authority block → reject; revoked AI block only → AI token rejected, original user token still valid; grant with `exp` in past by 30 s → reject (no skew on exp).
- **Prior art and evidence:** Biscuit spec [Observed]; Teleport cert-embedded identity incl. request ids, MFA verified, device id (doc 01); DPoP/RFC 7800 `cnf` [Inferred].
- **Used by:** Karagöz (PEP); Hacivat (attenuation, presentation).

### ALG-POLICY-01: Policy snapshot compilation and distribution (Perde → Karagöz)

- **Problem:** Give each agent exactly the policies it needs, versioned, signed, small, and verifiable offline.
- **Inputs / outputs:** In: org roles, bindings, conditions, access-request template links, org-wide forbids, agent inventory labels. Out: `Snapshot{org, agent_id, epoch, min_policy_epoch, issued_at, cedar_policies (text), cedar_schema hash, entities (users/groups/roles **referenced**, AI agent entities, databases of this agent with labels), revocation_epoch pointer, approval keys, grant keys}`, signed by Perde's policy key.
- **Algorithm:** (1) On any change, bump org `epoch`; (2) for each agent, select policies whose resource scope can match any of the agent's databases (static analysis on `resource ==`/`in`/label conditions; when undecidable include it); (3) include principal entities only for users bound to included policies (privacy: agents don't learn the full directory); (4) validate with Cedar schema; (5) sign; (6) push delta notification; agents fetch full snapshot (simple, robust) with `If-None-Match: epoch`; (7) agent verifies signature, schema hash, monotonic epoch (reject rollback), applies atomically, writes `policy.snapshot.applied` audit record.
- **Edge cases:** user directory too large to embed → only users referenced by principal-specific policies; group membership changes affect grants on refresh anyway; snapshot apply failure → keep previous, alert; rollback attempt (lower epoch) → reject; local floor policies merged after verification (only `forbid` allowed in floor files; `permit` in floor is a config error).
- **Test cases:** add role binding → epoch+1, agent applies within seconds; tamper snapshot → rejected; floor `permit` → agent refuses to start with clear message.
- **Used by:** Perde, Karagöz.

### ALG-AUTHZ-02: Permission evaluation at the agent (Cedar)

- **Problem:** Evaluate an action request against the snapshot + floors with full context; return decision and explanation.
- **Inputs / outputs:** In: `SessionContext`, action (`connect`, `query.read|write|ddl|admin`, `export`, `unmask`), resource (Database or Table entity), context record. Out: `Decision{allow|deny, determining_policy_ids[], errors[]}`.
- **Algorithm:**
  1. Build principal entity: `User::"u"`; if `actor_kind = ai_agent` → principal `AiAgent::"<session>"` with parent `User::"u"` (so `principal in User::"u"`-style permits for the human do **not** automatically apply unless policy uses `principal in`; org default template includes `forbid` for AI writes).
  2. Context: `{mode_from_grant, mfa_age_s, device_trust, actor_kind, mcp_client, request_ids (set), channel_kind, time, classifier: {category, flags, objects}}`.
  3. Evaluate floors (forbid-only) ∪ snapshot policies with Cedar `is_authorized`; any forbid → deny; else any permit → allow; else deny.
  4. Intersect with grant: action must be ≤ grant mode (`read < write < ddl < admin` lattice, `export` and `unmask` separate flags). Both must allow.
  5. Policy evaluation errors (e.g. missing attribute) → treated by Cedar as the policy not applying; if a **forbid** errored → deny (fail closed) — implement by running a validation pass at snapshot load so this cannot happen silently.
  6. Return decision + determining policy ids for the audit record.
- **Complexity:** Cedar evaluation ~10–100 µs for hundreds of policies; cache per `(session, action, resource)` for statements of the same shape within the session, invalidated on snapshot/revocation change.
- **Edge cases:** resource labels change after session start (inventory update) → re-evaluate on next statement (cache invalidation by inventory generation); entity missing from snapshot (new user) → deny with `unknown_principal` (grant was issued after snapshot; trigger fetch); AI agent acting for a user whose role is revoked mid-session → revocation push kills session anyway.
- **Test cases:** analyst on prod non-PII read → allow (policy id `analyst-read`); same on PII DB → deny; AI agent write with user role allowing write → deny by `forbid-ai-write`; floor readonly label → write denied even for admin.
- **Prior art:** Teleport deny-overrides role-set matching (doc 01 ALG-ACCESS-T01); Bytebase CEL-conditioned bindings (doc 02 ALG-BYTEBASE-01).
- **Used by:** Karagöz; Perde uses the same engine for "what-if"/explain UI.

### ALG-AUTHZ-03: Query authorization (classifier + policy + server-side enforcement)

- **Problem:** For each statement (or batch) decide allow/deny, execute in a way the DB itself confines, and record classification outcomes.
- **Inputs / outputs:** In: statement text/bytes (or structured command for Mongo/Redis), dialect + server version, `SessionContext`, DB principal tier (doc 04), catalogue cache (function volatility, object ids), masking profile. Out: execution or denial; audit record; masking plan for ALG-MASK-01.
- **Algorithm:**
  1. **Normalise input**: reject NUL bytes and invalid UTF-8 for SQL engines (unless the DB encoding requires otherwise — treat as unknown); size limit (default 1 MiB).
  2. **Parse** with the dialect parser; on failure → `category = unknown`.
  3. **Split** into statements; for each derive `category` per §3.8 table, `objects` (fully qualified, resolved with `search_path`/default DB), `functions` (resolve volatility from catalogue cache; unknown function → `unknown_fn` flag), flags (`locks`, `exports`, `role_switch`, `multi_stmt`, `txn_control`, `explain_analyze`, `copy`, `procedural`).
  4. **Batch category** = max over statements by lattice `read < write < ddl < admin`, with `unknown` treated as `admin` for decision purposes and `role_switch` always denied unless mode=admin and policy allows `session.role_switch`.
  5. **Policy**: ALG-AUTHZ-02 with action from category and resource per object (if object-level policies exist: evaluate each table; any deny → deny batch).
  6. **Mode wrapper**: if session mode is read: wrap the batch in a read-only transaction when not already inside a client-opened transaction (PG: `BEGIN READ ONLY; …; COMMIT`; MySQL: `START TRANSACTION READ ONLY`); if the client opened a transaction itself, Karagöz must have opened it as read-only at `BEGIN` time (rewrite `BEGIN` → `BEGIN READ ONLY`); `SET TRANSACTION READ WRITE`/`SET default_transaction_read_only = off` are classified `session.unsafe` and denied in read mode.
  7. **Guardrails** (policy options): statement timeout (server-side `statement_timeout`/`MAX_EXECUTION_TIME`/`max_execution_time`), max rows returned (stream cut + `truncated` flag), `UPDATE/DELETE` without `WHERE` → require confirmation (UX) or deny for AI actors, `DROP`/`TRUNCATE` → require approval (ALG-REQ-01 kind 2).
  8. **Execute** as the session's DB principal.
  9. **Reconcile**: map DB errors — PG `42501`/`25006`, MySQL `1142`/`1792`, MSSQL `229`/`3906`, Mongo `Unauthorized (13)`, Redis `NOPERM`, ClickHouse `READONLY (164)`/`ACCESS_DENIED (497)` — to `denied_by_db`; if classifier said allowed-read but DB denied a write-like action → audit `classifier_miss=true` (and if classifier said read and DB *succeeded* in a write — detectable via rows affected > 0 or command tag `INSERT/UPDATE/DELETE` — raise **high severity** `enforcement_gap`, terminate session).
  10. **Audit** (ALG-AUDIT-01) with timings and counts.
- **Data structures:** `ClassifiedStatement{kind, category, objects[], functions[], flags, fingerprint, fingerprint_hash}`; per-session catalogue cache (function volatility map, invalidated on DDL in session or every 5 min).
- **Complexity:** parse O(n) in statement length; typical < 1 ms; catalogue lookups cached.
- **Edge cases (exhaustive list):** (1) `EXPLAIN ANALYZE DELETE …` → write; (2) data-modifying CTE; (3) `SELECT … FOR UPDATE` → write-lock (deny in read mode); (4) `SELECT nextval('s')` → write; (5) volatile UDF in SELECT → write-risk unless allowlisted; (6) `PREPARE p AS DELETE …; EXECUTE p` → classify `PREPARE` by inner statement and track prepared names per session; (7) extended-protocol PG Parse/Bind/Execute (if wire-proxy mode, doc 03 ALG-SECENG-07) → classify at Parse, remember per statement name; (8) multi-statement with string literal containing `;` → parser handles; (9) MySQL conditional comments `/*!50000 DROP TABLE x */` executed by server → parser must treat versioned comments as code (sqlparser may ignore them → treat any `/*!` as `unknown`); (10) PG dollar-quoted strings; (11) `DO $$ … $$` → procedural → write; (12) `COPY (SELECT …) TO STDOUT` → export action; (13) `\copy`/psql meta-commands are client-side → never reach Karagöz; (14) `SET search_path` to attacker schema with shadowing functions → session-safe? No: classify as `session.search_path` and re-resolve objects after it; (15) `CALL proc()` → write unless proc marked read-only in catalogue (PG procedures can commit) → write; (16) MSSQL `EXEC('…')` dynamic SQL → unknown; (17) Mongo `$where`/`$function` JavaScript → unknown in read mode? They can't write but are expensive → allow with timeout (policy); (18) Redis `EVAL` → `@scripting`; executed commands are ACL-checked by Redis per call [Inferred] → allow per ACL; (19) Redis `MULTI/EXEC` → classify all queued commands; (20) `LISTEN` → allowed read-ish; `NOTIFY` → write; (21) `VACUUM`, `ANALYZE` → admin; (22) `CREATE TEMP TABLE` in read mode → write (PG allows temp tables only if TEMP privilege; read-only transaction forbids it anyway); (23) statement exceeding size limit → deny `statement_too_large`; (24) classifier version skew vs server version syntax (new keywords) → unknown → deny in read mode with message suggesting update; (25) comments carrying instructions to AI — irrelevant to classification but logged.
- **Test cases (input → expected):** `SELECT * FROM orders` (read mode) → allow, wrapped RO; `WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d` → deny `write_in_read_mode`; `EXPLAIN SELECT 1` → allow; `EXPLAIN ANALYZE UPDATE t SET x=1` → deny; `SELECT 1; DROP TABLE t` → deny batch; `/*!50000 DROP TABLE t*/ SELECT 1` (MySQL) → unknown → deny; `SET ROLE postgres` → deny `role_switch`; `SELECT pg_terminate_backend(123)` → deny admin; read principal + classifier bug letting `INSERT` through → DB error `42501` → `classifier_miss`; Redis `SET k v` in read → deny; Mongo `aggregate([{$merge: …}])` → deny.
- **Prior art and evidence:** Teleport relies on DB privileges only (doc 01 §5.2-1); Bytebase SQL review + read-only enforcement per engine (doc 02 ALG-BYTEBASE-02); Hoop/StrongDM rule chains (doc 03 ALG-SECENG-09); server-side read-only matrix (doc 04 §3.14).
- **Used by:** Karagöz; Hacivat (same classifier runs locally for UX hints — never trusted).

### ALG-REVOKE-01: Revocation push, staleness and session kill

- **Problem:** Propagate revocations quickly; bound damage when propagation fails.
- **Inputs / outputs:** In: Perde revocation events; agent control channel; local `R`, `revocation_epoch`, `last_sync`. Out: updated `R`; terminated sessions; audit records.
- **Algorithm:** (1) Perde appends revocation `{epoch++, type, value, until, reason}` to org log; (2) pushes to connected agents; agents apply in epoch order, detect gaps (missing epoch) and fetch the range; (3) for each new entry, terminate matching active sessions (reason `revoked`), audit `revocation.applied`; (4) heartbeat every 15 s carries current `revocation_epoch`; agent behind → fetch; (5) `last_sync` older than `max_staleness` → strict: reject new sessions (`agent_revocation_stale`), lenient: allow but flag sessions `stale_authz=true` in audit; (6) prune entries past `until`.
- **Edge cases:** Perde partition → staleness logic; huge burst (org-wide lock) → single entry type `org_lock`; out-of-order delivery → epoch ordering; agent restarts → persisted `R` reloaded, then sync before accepting sessions (unless offline mode allowed).
- **Test cases:** revoke user → active session killed within heartbeat; network cut 10 min with strict 5 min → new sessions refused, existing read sessions continue until grant exp.
- **Prior art:** Teleport locks (doc 01 ALG-ACCESS-T06).
- **Used by:** Perde, Karagöz.

### ALG-AUDIT-01: Audit record canonicalization, hashing and chaining

- **Problem:** Produce an append-only, verifiable sequence of audit records on the agent, robust to crashes, with privacy-separable payloads.
- **Inputs / outputs:** In: event structs from sessions/provisioning/policy; agent key; current stream state. Out: durable records `{stream_id, seq, ts, type, body, prev_hash, hash}` + payload rows.
- **Algorithm:**
  1. **Stream identity:** `stream_id = H("hv-stream" ‖ agent_id ‖ start_ts ‖ random128)`; a new stream begins on first start or after unrecoverable local store loss; the first record of a stream is `stream.start{prev_stream_id?, prev_stream_last_hash?}` (links streams).
  2. **Split payload:** for each sensitive field `f` (query text, literals, prompt ref, sampled rows) generate `salt_f` (16 random bytes), compute `commit_f = H("hv-commit" ‖ salt_f ‖ f)`, store `(record_key, field, salt_f, f)` in payload table; body keeps `commit_f`.
  3. **Canonicalize body** with deterministic CBOR (RFC 8949 §4.2.1 core deterministic encoding: shortest ints, sorted map keys by bytewise encoding, no indefinite lengths) — or RFC 8785 JCS if JSON is preferred; choose one and version it (`canon_v = 1`). Floats forbidden (durations in integer µs).
  4. **Hash:** `leaf_i = H(0x00 ‖ "hv-audit-v1" ‖ stream_id ‖ u64be(seq_i) ‖ u64be(ts_i_ms) ‖ canon(body_i))`; `chain_i = H("hv-chain" ‖ chain_{i-1} ‖ leaf_i)` with `chain_0 = H("hv-chain-genesis" ‖ stream_id)`. Store both (leaf for Merkle, chain for linear verification).
  5. **Durability:** write in a single SQLite transaction (WAL, `synchronous=FULL`) `records` + `payloads` + `stream_head(seq, chain)`; group commit: batch writes every ≤ 5 ms or 64 records to amortize fsync.
  6. **Ordering:** a single writer task per stream assigns `seq`; producers send via bounded channel. Channel full → back-pressure: statements wait up to 100 ms; still full → **fail closed** for new statements (`audit_unavailable`) unless policy `audit_degraded_mode=allow_reads` (then reads proceed and `audit.degraded` is recorded once the writer recovers, with counts of unlogged events — honest gap).
  7. **Disk quota:** if records awaiting upload exceed quota (default 1 GiB), behave as step 6 (never delete unacked records).
- **Data structures:** `records(stream_id, seq PK, ts, type, body BLOB, leaf, chain, uploaded_at?, cosigned_checkpoint?)`; `payloads(leaf, field, salt, value, expires_at)`; `stream_head`.
- **Complexity:** ~2 hash ops + CBOR encode per record (µs); fsync amortized.
- **Edge cases:** clock going backwards → `ts` may decrease; ordering is by `seq`, not ts; record `clock_anomaly` when `ts_i < ts_{i-1} - 1s`; crash mid-transaction → SQLite atomicity, seq continuity preserved; local DB corruption → start new stream with `stream.start{prev_stream_id, reason=store_lost}` → Perde sees a gap in the old stream (expected and flagged); erasure → delete payload rows only (body/leaf untouched); huge query text (MBs) → payload store with size cap + `truncated` flag in body (commit over full text computed streaming).
- **Test cases:** recompute chain from records → matches `stream_head`; delete record 5 of 10 → verification fails at 6; alter a body byte → fail; delete payload of record 3 → verification still passes, `text` unavailable; kill -9 during write burst → no seq gaps after restart.
- **Prior art and evidence:** RFC 6962 leaf/node prefixing to prevent second-preimage confusion [Inferred]; Teleport durable SQLite audit queue (doc 01 ALG-ACCESS-T08) lacks chaining.
- **Used by:** Karagöz.

### ALG-AUDIT-02: Checkpoints, batched upload, cosigning and gap detection

- **Problem:** Ship records to Perde efficiently, give both sides non-repudiable anchors, and detect missing, forked or truncated history.
- **Inputs / outputs:** In: local records; Perde endpoint. Out: uploaded batches; signed checkpoints; Perde cosignatures; Perde-side stream status `{healthy, pending, gap, fork, truncated}`.
- **Algorithm (agent):**
  1. Every `N=1000` records or `T=10 s` (whichever first; also on session end and shutdown), build checkpoint over the stream's leaves `[0, size)`: `root = MTH(leaves)` per RFC 6962 (leaf hash already prefixed with 0x00; internal nodes `H(0x01 ‖ left ‖ right)`; split at largest power of two < n). Maintain a compact range (log n hashes) incrementally — O(log n) per append.
  2. Checkpoint note text (C2SP style): line 1 origin `hacivat.dev/audit/<org_id>/<agent_id>/<stream_id>`; line 2 `size`; line 3 base64 `root`; no extension lines. Sign as a signed note with agent key → record `audit.checkpoint` (itself appended to the stream *after* the checkpoint it describes; it is covered by the next checkpoint). [Observed: C2SP tlog-checkpoint format; extension lines NOT RECOMMENDED]
  3. **Upload** batches `{stream_id, from_seq, records[from..to], checkpoint(size=to+1), consistency_proof(prev_cosigned_size → size)}` compressed (zstd), ≤ 4 MiB; retry with backoff; idempotent by `(stream_id, seq)`.
  4. On `200`, Perde returns its **cosignature** over the checkpoint; agent stores it and marks records `uploaded`. Records remain locally for `local_keep`.
- **Algorithm (Perde):**
  5. Verify agent cert & checkpoint signature; verify each record's leaf/chain recomputation from `from_seq` using stored `chain_{from_seq-1}`; verify Merkle root equals recomputed root over all leaves (or consistency proof from last cosigned checkpoint + new leaves).
  6. **Classify:**
     - `from_seq > last_seq + 1` → **gap** (`missing [last+1, from-1]`): accept batch into a quarantine buffer, request missing range; if agent replies `not_available` (store lost) → mark permanent gap, alert.
     - `from_seq ≤ last_seq` and overlapping records differ (leaf mismatch) → **fork**: reject, lock agent (possible tampering or clone), alert.
     - checkpoint `size < last_cosigned_size` or inconsistent with previously cosigned checkpoint → **truncation/rewrite**: reject, alert; never cosign (consistency rule). [Observed: C2SP consistency rule]
     - New `stream_id` without `stream.start.prev_stream_id` linking to the latest known stream's head → **stream reset**: accept, flag `unlinked_stream` (expected only after documented store loss).
     - Agent silent > `pending_threshold` (default 15 min) while heartbeats arrive → **pending/withheld**: alert ("agent alive but not uploading audit").
  7. Cosign valid checkpoints (Ed25519 cosignature with timestamp as in C2SP cosignature) and persist.
  8. **Cross-check with session metadata:** Perde knows grants it issued and sessions reported in heartbeats (`active_session_ids`); a session that started (heartbeat) but whose `db.session.start` never arrives within T → `missing_session_audit` alert.
- **Data structures (Perde):** `streams(stream_id, agent_id, last_seq, last_chain, last_cosigned_size, last_root, status)`, `records` (partitioned by org/day), `checkpoints`, `gaps`.
- **Complexity:** Perde verification O(batch) hashes; Merkle compact range O(log n) memory per stream.
- **Edge cases:** batch reordering on the network → idempotent apply in seq order; Perde DB restore from backup (Perde forgets newer cosignatures) → agent presents its stored cosigned checkpoint → Perde accepts as proof and re-ingests; agent offline for days → large backlog uploaded in chunks; org deletes data per retention → Perde keeps checkpoints (tiny) so chain continuity metadata survives record deletion; erasure of payloads on Perde mirrors agent; agent clone (two agents same key) → forks at same seq → lock (ties to ALG-AGENT-01).
- **Test cases:** normal flow 10 k records → 10 checkpoints all cosigned; drop batch 3 in transit → Perde detects gap then fills after retry; agent rewrites record 50 after it was cosigned → fork/truncation detected, agent locked; agent loses SQLite store → new stream with `prev_stream_id`, Perde flags old stream gap tail between last cosigned and last heartbeat seq.
- **Prior art and evidence:** C2SP tlog-checkpoint/cosignature [Observed]; RFC 6962 [Inferred]; Teleport audit uploader without tamper evidence (doc 01 ALG-ACCESS-T08).
- **Used by:** Karagöz, Perde; OSS verifier CLI.

### ALG-REQ-01: Access request and approval state machine

- **Problem:** Represent JIT access (role elevation, one-shot statements, unmask/export) with multi-party approval, expiry and revocation, enforceable at the offline agent.
- **Inputs / outputs:** In: request `{kind ∈ {elevation, statement, unmask, export}, requester (user or ai_agent on behalf of user), targets (db ids / label selector), mode, duration ≤ max, justification, statement bytes (kind=statement), ticket ref}`, approval rules (by target labels & mode: `{required: [{group, count}], forbid_self_approval: true, require_mfa_for_reviewers: true, max_duration}`). Out: state transitions, Cedar template link (elevation/unmask/export) or **statement approval token** (Biscuit third-party block / signed object) (statement), audit events.
- **States:** `pending`, `approved`, `denied`, `cancelled`, `expired` (pending timeout), `active` (first use), `completed` (duration ended or one-shot consumed), `revoked`.
- **Transitions:**
  1. `create` → `pending` (validate targets exist; compute matching rule; reviewers notified).
  2. `pending` + `review(approve)` by eligible reviewer (not requester; for AI-originated requests, the human on whose behalf it acts is **also** not allowed to self-approve unless rule permits "owner approval") → record; when every required `{group, count}` satisfied → `approved`. Same reviewer counts once; reviewer in multiple groups counts toward one group (choose assignment maximizing satisfaction — bipartite matching; small sizes → brute force).
  3. `pending` + `review(deny)` → `denied` (any single deny if rule `deny_on_first`; else threshold of denies).
  4. `pending` + requester `cancel` → `cancelled`; `pending` + timeout (default 24 h) → `expired`.
  5. `approved` → Perde: elevation/unmask/export → create Cedar template-linked policy with `context.request_id == R && time < exp`, bump epoch, push snapshot; statement → issue approval object `{request_id, agent_id, db_id, stmt_hash = H(normalized_bytes), exact_bytes_hash = H(bytes), mode, nbf, exp, approver ids}` signed by approval key (Biscuit third-party block appended to the requester's next grant, or standalone signed object presented with the statement).
  6. `approved` + first grant/statement using `request_id` seen in agent audit → `active`.
  7. `active|approved` + `exp` → `completed`; statement kind + execution reported → `completed` (one-shot; agent records `request_consumed` and adds `request_id` to local consumed set so replays are rejected even offline).
  8. `approved|active` + admin `revoke` → `revoked` → revocation push type `request` → agent kills sessions using it.
- **Agent enforcement:** statement approvals are checked in ALG-AUTHZ-03 step 5: if policy requires approval for the statement's category, require a valid approval object whose `exact_bytes_hash` matches the incoming statement bytes (default) or `stmt_hash` matches the fingerprint (only if the approval says `binding=fingerprint`), not consumed, within validity, signed by a trusted approval key, audience matches agent/db. [Prior art: doc 02 ALG-BYTEBASE-06, doc 03 ALG-SECENG-08]
- **Edge cases:** reviewer loses eligibility (group change) before threshold met → their approval no longer counts (re-evaluate at each review); request targets a DB that disappears → `expired` with reason; duration requested > max → clamp & note; approval during Perde→agent partition → agent can't learn the template; statement approvals still work if the approval object travels with Hacivat (offline-capable by design); clock skew → exp checks with skew only on nbf; AI agents spamming requests → per-user rate limit and dedupe identical pending requests; break-glass: pre-authorized emergency role with immediate `approved` + mandatory post-hoc review and high-severity alert.
- **Test cases:** rule {admins:2} → two different admins approve → approved; same admin twice → still pending; requester is admin → own approval ignored; statement approval for `DELETE FROM t WHERE id=5`, executed with `id=6` → denied (hash mismatch); executed twice → second denied (consumed); revoke during active session → session terminated.
- **Prior art and evidence:** Teleport access request thresholds/reviews (doc 01 ALG-ACCESS-T04); Bytebase approval flows (doc 02 ALG-BYTEBASE-03).
- **Used by:** Perde (state machine), Karagöz (enforcement), Hacivat (request UI, MCP "request approval" tool).

### ALG-MASK-01: Data masking at the result-stream level

- **Problem:** Apply column-level masking to result sets (and exports, MCP outputs) according to policy, correctly even for derived columns, with fail-closed behaviour.
- **Inputs / outputs:** In: classified statement (AST), catalogue (column semantic labels: Perde-assigned `pii.email`, `pii.tckn`, `pii.phone`, `secret`, … from labels or `hacivat.masking.*` hints or auto-classifier suggestions approved by admin), session `mask_profile`, unmask grants, result column metadata from the driver. Out: `MaskPlan{per output column: rule | pass | reject}`, transformed result stream, audit `masked_columns`.
- **Algorithm:**
  1. **Lineage:** from the AST, compute for each output column the set of source columns it depends on (through aliases, expressions, CTEs, subqueries, views (expand view definitions from catalogue), `UNION` (union of branch lineages per position), `JOIN`s, window/aggregate functions). Track **transform class**: `identity` (column as is), `preserving` (lower/upper/trim/cast to text/concat with constants — may still reveal data), `aggregate_safe` (`count`, `count distinct` → reveals cardinality only), `aggregate_revealing` (`min`, `max`, `string_agg`, `array_agg`, `json_agg`), `opaque` (UDF, unknown function).
  2. If lineage cannot be computed (parse failure, `SELECT *` from unknown function, dynamic SQL) and the referenced objects include **any** sensitive column → mark all output columns `reject` or `mask_all` per profile (fail closed). If objects are not determinable at all → if the session has any active masking rules on this DB → `reject` statement with `masking_lineage_unknown`.
  3. For each output column: collect sensitive sources; pick the **strictest** rule among them (order: `reject > null > full > hash > partial > pass`); `aggregate_safe` → pass; `aggregate_revealing`/`preserving`/`identity` → apply rule; `opaque` with sensitive source → strictest of (rule, `full`).
  4. If the user has an `unmask` grant for those columns (via ALG-REQ-01 or policy) → pass, but audit `unmasked=true`.
  5. **Apply** in the result encoder per row, streaming (no buffering): functions —
     - `full`: replace with fixed token `••••` (type-aware: numbers → NULL or 0 per profile).
     - `partial(email)`: keep first char of local part + domain (`a•••@example.com`); `partial(phone)`: last 2–4 digits; `partial(tckn)`: first 0 / last 2; `partial(card)`: last 4.
     - `hash`: `base32(HMAC-SHA256(org_mask_key ‖ column_semantic, value))[0..16]` — deterministic within the org (joins/group-by on pseudonyms still work), not reversible without key; key held by Karagöz (distributed encrypted to agents) — rotate per org policy.
     - `null`: NULL.
     - `date_trunc(year|month)`: generalization for dates.
     - `range(bucket)`: numeric generalization.
  6. **Type safety:** masked values must fit the declared result column type or the column type must be changed in metadata sent to Hacivat (Karagöz controls its own result schema in native RPC mode). In wire-proxy mode (doc 03 ALG-SECENG-07), keep the DB's declared type: text types → masked text; numeric → NULL; binary → NULL.
  7. Mark result metadata `masked: rule_id` per column → Hacivat shows a mask icon; export paths re-use plan; copy-to-clipboard of masked cells yields masked values.
  8. Mongo/Redis: rules on field paths (`customer.email`, `$..ssn`) / key patterns + hash fields; apply on documents during BSON→wire encoding; projections with `$`-operators → treat as opaque.
- **Data structures:** `SensitiveCatalog{(db, schema, table, column) → semantic label}`; `MaskProfile{semantic label → rule}`; per-statement `MaskPlan`.
- **Complexity:** lineage O(AST); per-cell transform O(len); HMAC ~1 µs per value.
- **Edge cases (exhaustive list):** (1) `SELECT *` → expand from catalogue at plan time; if table altered between plan and execution, compare driver column names/positions and fail closed on mismatch; (2) column aliases hiding sources (`email AS x`) → lineage handles; (3) `concat(first, ' ', email)` → preserving → mask whole; (4) `substring(email, 1, 3)` → preserving → mask; (5) `count(DISTINCT email)` → pass; (6) `max(salary)` → mask; (7) `CASE WHEN ssn LIKE '1%' THEN 'a' ELSE 'b' END` → derived from ssn → mask (inference via predicate inside projection); (8) predicates in `WHERE` on masked columns → not masked (known inference channel) → strict profile denies predicates on `secret`-class columns; (9) `JSON` extraction from a sensitive JSON column (`data->>'email'`) → lineage to JSON column → rule of the JSON column (or per-path rules if defined); (10) views → expand definitions (requires catalogue read); security-barrier views/materialized views → treat matview columns as their own catalogued columns; (11) UDF returning sensitive data from a non-sensitive input (e.g. `get_email(id)`) → `opaque`; if the function body references sensitive tables (catalogue analysis) → mask; else pass with `opaque_fn` audit flag; (12) `COPY (query) TO STDOUT` → same plan; (13) `RETURNING` clause of DML → same plan; (14) result column type mismatch after masking → per step 6; (15) NULL values → stay NULL (don't reveal NULL vs non-NULL difference? reveal; documented); (16) binary/blob → NULL; (17) set operations with type coercion → per position; (18) recursive CTEs → fixed-point lineage; (19) huge rows → streaming; (20) error messages that echo values (e.g. unique violation `Key (email)=(a@b.com) already exists`) → mask error detail text for sessions with masking active (regex on known `DETAIL` patterns) or drop `DETAIL`; (21) `EXPLAIN` output may include literals/constants → not data; pass; (22) DB-side errors in `RAISE` with data → same as 20.
- **Test cases:** `SELECT email FROM users` with `partial` → `a•••@x.com`; `SELECT lower(email) e FROM users` → masked; `SELECT count(*) , count(distinct email) FROM users` → pass; `SELECT u.email, o.total FROM users u JOIN orders o …` → email masked only; `SELECT * FROM users` after `ALTER TABLE users ADD COLUMN tckn` (catalogued `pii.tckn`) → tckn masked; unparseable statement on DB with masking rules → reject; unique violation error detail → masked.
- **Prior art and evidence:** Bytebase lineage-based masking (doc 02 ALG-BYTEBASE-04); PostgreSQL Anonymizer, SQL Server Dynamic Data Masking (`MASKED WITH (FUNCTION = 'partial(...)')`) [Inferred] — server-side DDM is bypassable by inference too, and needs DB-level config; Karagöz-level masking works across engines.
- **Used by:** Karagöz; Hacivat (display indicators); Perde (catalog/profile admin).

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 Karagöz

1. Embed `biscuit-auth` (Apache-2.0) and `cedar-policy` (Apache-2.0) crates; both Rust — matches the Rust stack.
2. Single PEP pipeline for every request: `grant verify (AUTHZ-01) → policy (AUTHZ-02) → classify+authorize (AUTHZ-03) → execute as tier principal (doc 04) → mask (MASK-01) → audit (AUDIT-01)`.
3. Fail-closed defaults: no revocation sync yet → no sessions; audit unavailable → no new statements; masking lineage unknown on masked DB → reject.
4. Local operator floor policies (`/etc/karagoz/policy.d/*.cedar`, forbid-only) and `hacivat.readonly` label support.
5. OSS `karagoz audit verify` that recomputes chain, Merkle roots and checks Perde cosignatures.

### 5.2 Perde

1. Org CA + separate keys: CA (agent/device certs), grant-issuing key, policy-snapshot key, approval key(s) (optionally offline/HSM/second-admin device for dual control), audit cosigning key. Key separation limits the impact of a single key compromise; publish key ids & fingerprints in the org security page.
2. Policy authoring UI generates Cedar; "explain" endpoint runs the same evaluation as agents (determining policy ids).
3. Access requests (ALG-REQ-01) with Slack/Teams/email notifications (i18n templates), and MCP-originated requests surfaced distinctly ("requested by Claude Desktop on behalf of Ayşe").
4. Audit ingestion verifies chains, cosigns, stores, and offers OCSF export + SIEM sinks; gap/fork dashboard.
5. Open-core: tamper evidence, basic RBAC, audit export (at least JSONL/syslog) and SSO must not be paywalled (security basics; see doc 06). Candidates for paid: advanced approval workflows, SIEM-native connectors, long retention hosting, dual-control HSM integrations, compliance reports.

### 5.3 Hacivat

1. Holds device key in OS keystore; refreshes grants; performs attenuation for MCP sessions (read-only by default; table scoping from the user's MCP config; short TTL), seals AI tokens.
2. Runs the same classifier locally for UX (warn before sending a write in a read session) but never relies on it.
3. Shows masked columns, mode, expiry, request status; "Request access" flows; for AI tool calls proposing writes → create statement approval request rather than failing.

---

## 6. i18n considerations

- Denial reasons, request states, audit event titles are **codes** (`authz.deny.write_in_read_mode`, `request.state.approved`) with Fluent messages; parameters (table names, times) formatted by locale (Turkish date/time formats, 24 h).
- Audit records store codes and structured fields — never localized strings — so exports and verification are language-independent; OCSF `message` field filled in English (SIEM convention), localized rendering only in UIs.
- Cedar policy text and identifiers are ASCII; display names of roles/groups are user-provided Unicode (NFC normalized; Turkish casing handled with locale-aware display but **ASCII-only** comparisons for identifiers — no `toLowerCase()` with Turkish locale on keys).
- PII detectors include Turkish formats (TCKN with checksum, `TR` IBAN, Turkish phone numbers `+90 5xx`), plus locale-neutral ones.
- Notification templates for approvals (email/Slack) localized per recipient preference.

## 7. Risks, pitfalls, open questions

1. **Biscuit adoption risk:** smaller ecosystem than JWT; Datalog limits and versioning must be pinned (agents must reject unknown versions — spec mandates). Mitigation: wrap in our own `GrantV1` API so the format can be swapped. [Open: benchmark token sizes with 3–4 blocks over P2P handshake]
2. **Perde compromise** still yields new grants for anything not behind dual control or local floors. Make dual control easy for prod write/admin.
3. **Classifier ≠ security boundary**: must be communicated in docs and UI; DB privileges are the boundary; shared-superuser mode (doc 04 §3.15) weakens everything → clearly labelled.
4. **Masking inference channels** (predicates, timing, error messages) are inherent; masking is for casual exposure and AI context minimisation, not a hard guarantee.
5. **Audit fail-closed vs availability:** some orgs will prefer availability; expose the switch with an explicit audit record of degraded windows.
6. **Clock dependence** for TTLs on hosts without NTP; agent should report skew and refuse in strict mode.
7. **Payload erasure** semantics across Perde backups and SIEM exports — erasure cannot reach already-exported SIEM copies; document data flows.
8. **Cedar policy explosion** with many template-linked access requests → prune expired links on epoch bump; cap active links per org.
9. **[Unknown]** Whether to also support PASETO/JWT grants for third-party integrations (e.g. CI pipelines without Biscuit libraries) — likely via a Perde token-exchange endpoint that mints Biscuits.
10. **[Unknown]** Witness beyond Perde for hosted customers who distrust Perde itself (public witness network / customer-run witness). Design leaves room: checkpoints are standard signed notes.
11. **[Open]** Per-statement approval binding default (exact bytes vs fingerprint) — exact bytes recommended (doc 02 §5 agrees).
