# Open product decisions: register

- Status: Living register. Compiled 2026-11-11 from `docs/research/` (21 algorithm catalogs plus the access, clients, datagrip, i18n, mcp, networking and verification research).
- Owner: product owner. Each decision is closed by writing an ADR in `docs/decisions/` (or a one-line "Decided" note here) and updating the catalog that raised it.
- Already decided and therefore excluded: UI engine (GPUI plus our own `hacivat-ui`, ADR 0001), visual direction, and everything a catalog marks as **Decision** in its "conflicts resolved" table (see Appendix A).

## Decision log

- **2026-11-11:** The product owner accepted the recommended defaults for D-001 through D-010 (all P0) and D-011 (run-at-caret: ask, with a direct run when there is only one candidate; a selection runs exactly as selected, split into separate statements per algorithms/20 HV-EXEC-01). Catalogs that disagree (algorithms/10 SQL-SPLIT-04, and the Appendix B items covered by these decisions) must be aligned to this register.
- **2026-11-12:** v1 scope: Hacivat standalone (no login) plus its local MCP server. Karagöz and Perde follow in v1.x, and the client keeps the managed-connection seams in place so they can plug in later. v1 database engines: PostgreSQL, MySQL/MariaDB and SQLite. SQL Server, Redis and MongoDB follow in v1.x.
- **2026-11-12:** Splash: the owner wants a bolder opening moment. It will be redesigned together with the logo work. In v1 the splash must also tell the story of a local or direct connection, because there is no Perde or Karagöz path in v1.
- **2026-11-12:** These come from the product owner's answers to the v1 design builder check:
  - Scope wins over schedule. Nothing is cut from v1.
  - The Apple Developer account is available. Windows betas ship unsigned, and Windows signing is required before 1.0.
  - Grid numeric cells follow the UI locale (TR `110.190,85`), which **overrides D-064**. Copy, export and generated SQL always use raw values, and a setting switches the grid to raw display.
  - Baseline screen-reader support gates 1.0.
  - The loopback HTTP MCP endpoint ships in v1, off by default.
  - Betas use the placeholder logo; 1.0 waits for the final logo and the new splash.
- **2026-11-12:** AI write modes for standalone v1 (design doc L9). Each connection has an AI write mode, **Ask** (the default) or **Auto**:
  - **Auto behaviour.** Non-critical writes run without a prompt. Critical statements still prompt: TRUNCATE, DELETE, DROP, destructive ALTER, UPDATE/DELETE without WHERE, GRANT/REVOKE, and anything destructive or unclassifiable.
  - **Prompt choices.** The approval prompt offers *Allow once*, *Always allow for this session* and *Deny*.
  - **Session allowances.** An allowance is scoped to one MCP session, one connection and one operation class. It is never persisted, it is always visible and revocable, and it is audited.
  - **Production.** Enabling Auto on a production-tagged connection needs a one-time typed confirmation of the connection name.
  - **Overrides.** This overrides D-007 and D-008 for standalone v1. Revisit when Perde co-approval arrives in v1.x.
- **2026-11-12:** AI intent statement on approvals (design doc L10).
  - **Required fields.** Every MCP write that may need approval carries short `what` and `why` fields, written in natural language in the user's UI language. A call missing either field is rejected.
  - **What the approval prompt shows.** Three separate items: the AI's statement (labelled untrusted and rendered as plain text), Hacivat's own deterministic summary (effect class, targets, WHERE, estimated rows), and the SQL.
  - **Mismatch warning.** A heuristic check that uses no LLM shows a warning when the statement and the summary disagree.
  - **No effect on decisions.** The statement never changes the classification, the AI write mode or the session allowance. It is audited together with the SQL.
- **2026-11-12:** The v1 design doc (`docs/superpowers/specs/2026-11-12-hacivat-v1-design.md`) is approved by the product owner.
- **2026-11-11:** UI copy: the title-bar environment label for production reads "PROD" in Turkish ("PRODUCTION" in English).

## How to use this register

1. **P0 (10 decisions)** blocks the v1 architecture, the licensing or the security posture. Decide these first, in one sitting if you can. Each one changes code that is expensive to undo, or a public promise.
2. **P1 (24 decisions)** must be decided before the feature it belongs to is built. Take them component by component.
3. **P2 (56 decisions)** have a safe default. You can **accept all P2 recommended defaults in one go** and only revisit the ones you disagree with.
4. Each entry says what is being asked in plain words, why it matters, the realistic options, a recommended default (taken from the research unless marked *register proposal*), the affected components, and where it came from.
5. Source paths are relative to `docs/research/`. `algorithms/NN` means `docs/research/algorithms/NN-*.md`. IDs in parentheses (for example `HV-SEC-02`, `U2`, `PD-14`) are the spec or question IDs inside that file.
6. When a question appeared in several files, it was merged into one decision and **every** source is listed. Disagreements between sources are listed in Appendix B.

Short glossary for terms used below:
- **Production-tagged**: a connection the product treats as "production", which switches on stricter safety defaults (see D-004).
- **Managed connection**: a database reached through Karagöz with permissions from Perde (no password typed by the user). **Unmanaged** means a normal connection the user set up themselves.
- **Grant**: a short-lived, signed permission slip from Perde that Hacivat shows to Karagöz.
- **MCP / AI host**: the Model Context Protocol, the way AI apps (Claude Desktop, Cursor, …) call tools such as Hacivat's local MCP server.
- **Fail closed / fail open**: when something breaks, either refuse (closed, safer) or carry on (open, more available).

## Summary

| ID | Tier | Title | Recommended default |
|---|---|---|---|
| D-001 | P0 | Perde core license | AGPL-3.0-only (FSL only if investors demand a non-compete) |
| D-002 | P0 | Contribution terms and permissive licenses | CLA for Perde, DCO elsewhere; Hacivat, Karagöz and shared crates Apache-2.0 |
| D-003 | P0 | Open-core feature split | access/06 matrix: security is never paid; paid = org scale and operations |
| D-004 | P0 | What counts as "production" | User tag for own profiles; org-set and not lowerable for managed databases |
| D-005 | P0 | Default TLS (encryption) mode | `Auto`: verify when offered, plaintext only on private addresses, ask otherwise |
| D-006 | P0 | `max_staleness` (agent offline window) | 15 min, strict mode on, everywhere; 5 min "high security" preset |
| D-007 | P0 | Who may approve AI-proposed writes | Owner may approve on non-production; production needs a second approver |
| D-008 | P0 | AI access baseline and leak disclosure | Ask by default, production Off (max Read), disclose the residual leak |
| D-009 | P0 | Unknown functions in read-only mode | Deny, with a per-connection allowlist |
| D-010 | P0 | Audit storage unavailable | Fail closed; "allow reads" switch available in every edition |
| D-011 | P1 | Run-at-caret default | Ask, but run directly when there is only one candidate |
| D-012 | P1 | Concurrency checks when saving edits | On for all connections; compare all loaded columns |
| D-013 | P1 | Confirmations and preview outside production | Preview mandatory on production and for >100 deletes; otherwise optional |
| D-014 | P1 | Automatic rollback on production | Notify for idle transactions; 5 min auto-rollback only for an open confirm dialog |
| D-015 | P1 | Keeping server cursors open | Keep up to 10 min idle; read ahead 10,000 rows / 16 MiB |
| D-016 | P1 | Local data at rest | Master-password vault on Linux without keyring; cache plaintext + 0600 unless sensitive/managed |
| D-017 | P1 | System catalog snapshots: embed or download | Bundle in the installer (no download at runtime) |
| D-018 | P1 | What remote users see of the schema | Shared agent catalog filtered per user; completion shows only allowed objects |
| D-019 | P1 | Read-only transactions across requests | One request = one transaction |
| D-020 | P1 | `SET` commands through Karagöz | None in v1; schema set via connection parameters |
| D-021 | P1 | Large exports and imports for agent databases | Run on Karagöz above a size threshold (threshold TBD by design) |
| D-022 | P1 | Approval bound to exact SQL or a fingerprint | Exact bytes by default; fingerprint opt-in for low-risk classes |
| D-023 | P1 | Keeping SQL text in the local audit log | Redacted literals for AI events; full text opt-in |
| D-024 | P1 | Database login inside Karagöz stream | Trust inside the authorized stream; forwarder keeps its one-time credential |
| D-025 | P1 | MySQL front-end in Karagöz v1 | v1.x; v1 MySQL only through Hacivat's own driver path |
| D-026 | P1 | Netns helper (container network access) in v1 | Leave out of v1; accept operator override |
| D-027 | P1 | AI access to Karagöz databases in v1 | v1.x |
| D-028 | P1 | AI hosts that sandbox local servers | Compatibility spike before v1; HTTP fallback in v1.x |
| D-029 | P1 | Default keyboard shortcuts | Collision-safe "hacivat" preset; DataGrip preset opt-in; first-run choice |
| D-030 | P1 | Turkish glossary and terminology | Decide with Turkish DB users before v1 string freeze |
| D-031 | P1 | gpui-component's untranslatable strings | Do not show its user-visible widgets; wrap or replace |
| D-032 | P1 | Hosted telemetry and data residency | Telemetry decision after KVKK/GDPR review; Türkiye region decision with it |
| D-033 | P1 | SAML on hosted Perde | Free self-hosted; included from the first paid hosted tier |
| D-034 | P1 | High availability (HA) as paid feature | Paid for self-host (EE), included in hosted |
| D-035 | P2 | Connection budget per profile | 4 per remote profile; extra consoles queue |
| D-036 | P2 | Smart commit | Opt-in everywhere; production stays Manual |
| D-037 | P2 | `ProxyCommand` support | Not in v1; opt-in in v1.x at most |
| D-038 | P2 | Introspection depth thresholds | 2,000 / 20,000 / 5,000 after benchmarking |
| D-039 | P2 | Mass-deletion guard UX | Confirm when user-triggered; "stale" badge for background passes |
| D-040 | P2 | Oracle support approach | Defer Oracle; when added, Karagöz allows only a read allowlist until full parser |
| D-041 | P2 | Rename detection in schema compare | Suggest (unchecked suggestions) |
| D-042 | P2 | Online DDL defaults on production | On for production, off otherwise |
| D-043 | P2 | Undo scripts for non-transactional engines | Offer, with explicit "cannot restore dropped data" message |
| D-044 | P2 | "Apply anyway" after schema drift on production | Require Perde approval token verified at Karagöz for managed; typed confirm for local |
| D-045 | P2 | Default constraint placement in generated DDL | Inside the table |
| D-046 | P2 | Schema tooling scope | Cross-DBMS report-only; no DataGrip layout import; read-only Git check |
| D-047 | P2 | Small Redis/Mongo defaults | Natural order + "_id" toggle; no MEMORY USAGE; follow mongosh literals |
| D-048 | P2 | Hand-off to external `mongosh` | Opt-in, local read-write connections only |
| D-049 | P2 | Server-side JavaScript in read-only Mongo | Confirm once per session (AI and Karagöz always deny) |
| D-050 | P2 | Redis exact count / bulk actions on production | Confirm, plus Karagöz role gate |
| D-051 | P2 | Bundled Redis command metadata | Bundle Valkey (BSD-3) data; prefer live server data |
| D-052 | P2 | Schema inference on SQL JSON columns | Yes, reuse the Mongo sampler |
| D-053 | P2 | Parameters and script variables | Bind mode; no text substitution; detect and warn |
| D-054 | P2 | Server-side parse probe and typing depth | Light local typing; opt-in probe (PG only, v1.x) |
| D-055 | P2 | Name resolution mode defaults | Playground for consoles, Script for files, shown in status bar |
| D-056 | P2 | JOIN completion style | `existing.col = new.col`; aliases `c2` |
| D-057 | P2 | Sync virtual foreign keys through Perde | Local only in v1 |
| D-058 | P2 | MySQL executable comments in read-only/AI | Deny by default |
| D-059 | P2 | Temp tables for analysts in read-only | Denied in v1 |
| D-060 | P2 | SQL formatter defaults | Presets; format-on-save off |
| D-061 | P2 | Query history retention | 10,000 per data source, 90 days; record AI statements (filterable) |
| D-062 | P2 | EXPLAIN ANALYZE on writes | Rolled back by default |
| D-063 | P2 | Result memory budgets | 256 MiB per result; min(25% RAM, 4 GiB) total |
| D-064 | P2 | Grid number/date display | Raw machine format; localized opt-in |
| D-065 | P2 | Grid display details | TINYINT(1) as number; ASCII `NULL`; local filter pauses auto-fetch |
| D-066 | P2 | Editing scope in v1 | No views, no physical row ids by default, single-table only, ClickHouse read-only |
| D-067 | P2 | Spreadsheet formula-injection guard | On for spreadsheet presets only |
| D-068 | P2 | Import commit strategy | Single transaction for new tables on transactional-DDL engines, else every 50,000 rows |
| D-069 | P2 | Regional formatting in clipboard and CSV | HTML clipboard regional; CSV stays RFC 4180, suggest regional for `tr` |
| D-070 | P2 | Big numbers in JSON export | As strings |
| D-071 | P2 | Editor convenience defaults | Preserve line endings; three large-file tiers; completion after FROM/JOIN; no soft wrap |
| D-072 | P2 | IME composition with multiple cursors | Mirror to all cursors (setting) |
| D-073 | P2 | Screen-reader exposure of the editor | Window of ±100 lines; full mode up to 2 MB |
| D-074 | P2 | Turkish-tolerant search | On for Turkish users only, toggle in search box |
| D-075 | P2 | Bidi isolation marks in GPUI text | Keep; strip in adapter only if a platform shows glyphs |
| D-076 | P2 | Right-to-left mirroring | Decide before the first RTL language |
| D-077 | P2 | Translation workflow | In-house Turkish for v1; apply for Hosted Weblate, self-host as plan B |
| D-078 | P2 | Force English server messages for AI sessions | Yes, AI sessions only |
| D-079 | P2 | Small MCP defaults | 96 KiB / 20k tokens; per-run mask key; auto-start on with notice |
| D-080 | P2 | AI-proposed GRANT/REVOKE/role changes | Deny |
| D-081 | P2 | Future MCP surfaces | Later; remote endpoint needs its own threat model |
| D-082 | P2 | Trusting `hacivat.*` labels for auto-attach | Off (admin click required) |
| D-083 | P2 | Advanced trust features | Defer; decide on customer demand |
| D-084 | P2 | Approval rules expressiveness in v1 | "N approvals from group G" only |
| D-085 | P2 | Session resume in v1 | v1.x (reconnect + "session lost" message in v1) |
| D-086 | P2 | Very restrictive corporate networks | Document as unsupported in v1 |
| D-087 | P2 | Aggressive NAT probing (birthday) | Off by default; org opt-in later |
| D-088 | P2 | Relays inside customer networks | Later |
| D-089 | P2 | Lease mirror in Perde; OpenBao backend | Mirror yes (Karagöz authoritative); OpenBao later |
| D-090 | P2 | Several agents per database (agent HA) | Single agent per database in v1 |

---

## P0 — blocks v1 architecture, licensing or security posture

### D-001 — Perde core license

- **Decided 2026-11-11:** recommended default accepted by the product owner.
- **Question:** Which license should the open part of Perde use: AGPL-3.0-only or FSL-1.1-Apache-2.0?
- **Why it matters:** It decides whether a cloud company can sell a modified Perde without sharing its changes, and whether security buyers and Linux distributions accept Perde as real open source. The stewardship promise says released versions are never relicensed, so this cannot be undone later.
- **Options:**
  - **A. AGPL-3.0-only.** OSI-approved open source (trusted, accepted by distributions). Anyone running a *modified* Perde as a service must publish the changes. It does not stop a competitor hosting *unmodified* Perde (trademarks and hosted-only operations protect us there). Some enterprises ban AGPL; the commercial license (needs D-002) is the answer for them.
  - **B. FSL-1.1-Apache-2.0.** Forbids competing commercial use for 2 years, then becomes Apache. Not OSI → "source-available" label and reputational risk for a security product.
  - **C. BSL or Apache + paid `ee/` only.** Rejected in research (BSL backlash; Apache gives the hosted business no protection).
- **Recommended default:** A, AGPL-3.0-only — OSI status matters most for a security product; FSL only if investors require an explicit non-compete.
- **Components:** Perde (relay service included; the relay wire protocol and shared `hv-*` crates stay Apache-2.0, already settled in algorithms/34 P3).
- **Sources:** algorithms/34 §2-1 and header; access/06 §5.1, §7.1-1, §7.1-5, §7.3; networking/05 §7.3-9.

### D-002 — Contribution terms and permissive licenses

- **Decided 2026-11-11:** recommended default accepted by the product owner.
- **Question:** Must Perde contributors sign a CLA (Contributor License Agreement), and do Hacivat, Karagöz and the shared crates stay Apache-2.0 with a lighter DCO sign-off?
- **Why it matters:** Selling a commercial license next to AGPL is only possible if the company holds rights to every line of Perde. This must be decided **before the first external pull request to Perde is merged**.
- **Options:**
  - **A. CLA (license grant, not copyright transfer) for Perde; DCO for Hacivat, Karagöz and `hv-*` crates, all Apache-2.0.** Keeps dual licensing possible; low friction where most community work (drivers, UI, translations) happens.
  - **B. DCO everywhere.** Friendliest, but permanently blocks a commercial Perde license once outside code is merged.
  - **C. CLA everywhere.** Maximum flexibility, more friction for no gain on permissive components.
  - Sub-choice for the permissive parts: Apache-2.0 alone vs "MIT OR Apache-2.0" (Rust norm) vs MPL-2.0.
- **Recommended default:** A with plain Apache-2.0 — it matches `hacivat-ui` (ADR 0001) and the patent grant protects users.
- **Components:** Perde, Hacivat, Karagöz.
- **Sources:** algorithms/34 §2-4 and header; access/06 §5.1 ("Contribution terms", Hacivat and Karagöz rows), §7.1-1, §7.1-2; decisions/0001 §Decision 2.

### D-003 — Open-core feature split

- **Decided 2026-11-11:** recommended default accepted by the product owner.
- **Question:** Which Perde features are free (self-hosted community edition) and which are paid (EE modules or hosted tiers)?
- **Why it matters:** It is the business model, and it is a security question: if a protective feature is paid, free users are less safe, and a lapsed license must never silently switch protection off (a real pitfall seen in Bytebase).
- **Options:**
  - **A. The access/06 rule.** "If it is security-critical or enforced at the agent → free; otherwise paid only if mainly larger organizations need it." Free: enrollment/discovery/zero-credential connect, P2P, RBAC including custom roles, read-only enforcement, masking, single-approver just-in-time access, local audit with hash chaining, audit search/export, generic SIEM export, OIDC SSO, MFA, Hacivat AI controls, SAML on self-host. Paid: SCIM, multi-stage approval workflows and rules, ChatOps approvals, managed SIEM connectors and long retention, session recording storage, device trust, org-scale AI governance, HA/multi-region (D-034), fleet management, branding, compliance packs, support. Plus a public stewardship promise (features never move from free to paid; security never paywalled; no retroactive relicensing; EE relicensed to AGPL if abandoned).
  - **B. Aggressive gating** (Teleport/Bytebase style: masking, approvals, custom roles, SSO paid). More revenue per customer; backlash and "security paywall" criticism.
  - **C. Everything open; revenue only from hosted operations.** Maximum adoption; under-monetization risk.
- **Recommended default:** A — matches the "trust is visible" positioning; revisit the paid list after measuring what prospects actually ask for.
- **Components:** Perde (gating only at Perde's API; Karagöz has no license code), Hacivat (shows entitlement errors).
- **Sources:** access/06 §5.1–5.4, §7.1-4, §7.2; algorithms/34 PD-CP-04, PD-CP-05, P1, P5; README.md (Perde "feature split TBD"); PRODUCT.md (Capabilities); networking/05 §7.3-9; networking/03 §7.1-9; algorithms/03 §7-4; algorithms/12 RQ-4.

### D-004 — What counts as "production"

- **Decided 2026-11-11:** recommended default accepted by the product owner.
- **Question:** How does a connection get the "production" tag that so many safety defaults depend on, and who may change it?
- **Why it matters:** More than ten defaults switch on only for production-tagged connections (AI access Off, mandatory edit preview, confirm-in-transaction, online DDL, Redis bulk confirmations, Manual commit). If the tag is missing or wrong, all of them silently do not apply.
- **Options:**
  - **A. User-set tag per connection only** (for example local / dev / test / prod). Simple; relies on users tagging correctly.
  - **B. User-set for their own connections; for managed databases the org sets it (from Perde/Karagöz labels) and users cannot lower it.** Org owns the truth where it matters most.
  - **C. B, plus untagged remote connections are treated as production until the user says otherwise.** Safest; more prompts on first use.
- **Recommended default:** B, and ask for the tag when a remote connection is created (*register proposal*: the sources ask for a product-wide definition but do not pick one).
- **Components:** Hacivat, Perde, Karagöz, MCP.
- **Sources:** algorithms/03 §7-2 (HV-DDL-05/11, "shared with HV-SESS-04"); algorithms/01 §7-4, HV-SESS-04; algorithms/20 PD-3; algorithms/23 PD-17; algorithms/27 MCP-06, U5; algorithms/04 U4.

### D-005 — Default TLS (encryption) mode

- **Decided 2026-11-11:** recommended default accepted by the product owner.
- **Question:** When a user creates a connection and does not pick an encryption mode, what should Hacivat do?
- **Why it matters:** TLS encrypts traffic to the database and proves you reached the right server. The common default `prefer` silently falls back to no encryption and never checks the certificate, so traffic can be read or intercepted; a strict check can block a first connection to a self-signed server.
- **Options:**
  - **A. `Auto`.** If the server offers TLS → full verification; if its certificate is untrusted → offer to pin its fingerprint or import a CA; no TLS offered → plaintext only for loopback/private addresses (with a permanent "unencrypted" badge), otherwise ask (or fail in CLI/headless mode).
  - **B. `prefer` (libpq-compatible).** No friction; silent downgrade, no verification.
  - **C. Always require full verification.** Safest; breaks many local and dev setups.
- **Recommended default:** A — safe by default, with a guided path for self-signed servers instead of a silent downgrade.
- **Components:** Hacivat (Karagöz too if it reaches databases on other hosts later).
- **Sources:** algorithms/01 §7-1, HV-SEC-02; datagrip/02 §3.11; access/04 ALG-CRED-01 edge case 10.

### D-006 — `max_staleness` (how long an offline agent trusts old permissions)

- **Decided 2026-11-11:** recommended default accepted by the product owner.
- **Question:** If a Karagöz agent loses contact with Perde, how long may it keep accepting new sessions with the permission data it has, and should it then refuse new sessions (strict mode)?
- **Why it matters:** During that window a revoked user (for example someone just offboarded) can still connect to an isolated agent. Short = safer; too short = every network blip locks people out.
- **Options:**
  - **A. 15 min, strict on** (catalog default). At the limit, refuse new sessions and end write/DDL/admin sessions; read sessions run to grant expiry.
  - **B. 5 min** (access/05 hosted default). Faster revocation; more lockouts on flaky networks.
  - **C. Different defaults for hosted and self-hosted.**
  - **D. Strict off.** Availability first; revocation can wait indefinitely.
- **Recommended default:** A in both editions, with the 5 min "high security" preset one click away and org-configurable 0–24 h — one predictable behavior, safety on by default.
- **Components:** Perde, Karagöz.
- **Sources:** algorithms/32 §2-1, A5, A6, A7, PD-AUTHZ-07; access/05 §3 ("Strict staleness"), ALG-REVOKE-01; networking/05 ALG-PERDE-05 step 7 (`offline_grace` 15 min); access/01 §7-8; algorithms/31 KG-EXEC-01 step 8; algorithms/34 PD-CP-01 (Org settings).

### D-007 — Who may approve AI-proposed writes (self-approval)

- **Decided 2026-11-11:** recommended default accepted by the product owner.
- **Question:** When an AI agent proposes a data change on a user's behalf, may that same user approve it, and in whose name is it executed and audited?
- **Why it matters:** Self-approval keeps the local experience fast, but it means one person, possibly misled by a manipulated AI, can push a change to a shared database alone. Compliance teams may require a second human.
- **Options (approval):**
  - **A. Owner approval allowed on non-production; production needs a second approver** (algorithms/32 A11).
  - **B. Owner approval allowed everywhere by default; the org can require a second approver** (algorithms/27 U2).
  - **C. Second approver always for managed connections.**
- **Options (execution identity):** (i) execute as the approving human with an `ai_originated` flag (MCP-19); (ii) a separate "AI approved write" action type.
- **Also covered:** access requests an AI files on a user's behalf (elevation, unmask, export): the user may not approve their own unless the rule says `owner_approval: allowed`. Unmanaged local connections: the local user approves (no Perde involved).
- **Recommended default:** A + (i) — keeps local-first speed where risk is low, two humans on production; confirm with compliance that audit wording "user, AI-originated" is acceptable.
- **Components:** MCP, Hacivat, Perde, Karagöz.
- **Sources:** algorithms/27 U2, U3, MCP-14, MCP-16, MCP-19; algorithms/32 §2-2, A8, A11, PD-AUTHZ-04/08/09; access/05 ALG-REQ-01; mcp/02 (local human-in-the-loop approval).

### D-008 — AI access baseline and the leak we cannot close

- **Decided 2026-11-11:** recommended default accepted by the product owner.
- **Question:** How much may an AI agent see by default, and do we accept and disclose that data an AI reads can leak through the AI's other tools?
- **Why it matters:** After an AI reads rows, a prompt injection can make it send them elsewhere via another tool in the same AI app. Hacivat cannot stop that step; it can only limit what the AI reads.
- **Options:**
  - **A. Levels Off < Ask < Metadata < Read < Read+propose.** New connections Ask; production Off and capped at Read; proposing writes on production needs an explicit toggle and typed confirmation; "Metadata" preselected in the consent dialog; every Read grant shows a disclosure sentence; AI queries always run in read-only transactions, writes only through approved proposals.
  - **B. Default every connection to Metadata only** (no rows). Safer, much less useful.
  - **C. Relax production to Ask.** Convenient, risky.
- **Recommended default:** A — reads stay useful, nothing runs unattended, and the residual risk is stated honestly.
- **Components:** MCP, Hacivat.
- **Sources:** algorithms/27 U1, U5, MCP-02 step 8, MCP-06; algorithms/01 §7-8; mcp/02 §7 (accepted risk); mcp/01 §8 (prompt injection unsolved).

### D-009 — Unknown functions in read-only mode

- **Decided 2026-11-11:** recommended default accepted by the product owner.
- **Question:** In read-only sessions (Karagöz read roles and AI reads), may a query call user-defined functions that are not known to be harmless ("VOLATILE" or unknown)?
- **Why it matters:** A function can write data or reach the network even inside a plain `SELECT`. Allowing it punches a hole in the read-only promise; denying it blocks analysts who use custom functions.
- **Options:** **A. Deny, with a per-connection allowlist** (fail closed). **B. Allow when no network-capable extension is installed** (mcp/02's original stance).
- **Recommended default:** A — the read-only guarantee must not depend on guessing; analysts allowlist once.
- **Components:** Karagöz, MCP, Hacivat (shared classifier).
- **Sources:** algorithms/13 SQ-2, SQL-SAFE-03, SQL-SAFE-04; algorithms/27 U4, MCP-08; mcp/02 ALG-MCP-11; clients/sql-tooling-05 §7 (TOCTOU risk).

### D-010 — What happens when audit storage is unavailable

- **Decided 2026-11-11:** recommended default accepted by the product owner.
- **Question:** If Karagöz cannot record audit events (queue full, disk quota reached), should it block statements or keep serving?
- **Why it matters:** Blocking guarantees "no unaudited access" (what compliance wants) but turns a full disk into an outage; carrying on keeps service up but creates blind spots.
- **Options:** **A. Fail closed** (block). **B. Allow reads only while degraded** (`allow_reads`), recording the degraded window. **C. Fail open** (Teleport's default).
- **Recommended default:** A by default, with B as an org policy switch available in **every** edition (the source asks "per edition"; gating it would contradict D-003's "security never paywalled") — the degraded window itself is audited.
- **Components:** Karagöz, Perde.
- **Sources:** algorithms/31 §3-1, KG-AUDIT-06; access/05 §7-5; access/01 §7-5.

---

## P1 — decide before the relevant feature is built

### D-011 — Run-at-caret default

- **Decided 2026-11-11:** recommended default accepted by the product owner.
- **Question:** When the user presses "Run" with the cursor inside a statement and nothing selected, what runs: the smallest statement, or a small chooser?
- **Why it matters:** Running the wrong range on a production database is a real accident. A chooser is safer but adds a popup; running the statement directly is faster.
- **Options:** **A. Ask, but run directly when there is only one candidate** (DataGrip's shipped default). **B. Smallest statement, with the range highlighted before running** (DBeaver/Beekeeper habit).
- **Recommended default:** A — the extra popup appears only when there is real ambiguity. The selection policy must be settled at the same time (see Appendix B).
- **Components:** Hacivat (also MCP `execute_at_cursor`).
- **Sources:** algorithms/20 PD-1, HV-EXEC-01, HV-EXEC-02; algorithms/10 §6-1, SQL-SPLIT-04; datagrip/06 §7; datagrip/07 §7.

### D-012 — Concurrency checks when saving grid edits
- **Question:** When the user edits rows in the grid and saves, should Hacivat check that nobody else changed those rows in the meantime, on every connection or only on production?
- **Why it matters:** Without the check, saving can silently overwrite a colleague's change (a known DBeaver problem). With it, a save can fail with "row changed" and the user must reload.
- **Options:** **A. On for all connections; deletes compare all loaded comparable columns.** **B. Only on production-tagged connections.** **C. Compare only a version column.** For MongoDB: value only / value + type / whole document.
- **Recommended default:** A, and for MongoDB value + BSON type (whole document optional, version field when configured) — silent overwrites are a trust failure.
- **Components:** Hacivat.
- **Sources:** algorithms/23 PD-14, HV-EDIT-05, HV-EDIT-06; algorithms/04 U8, HV-NOSQL-12; clients/dbeaver-04 §7 (risk 1, open question on "use all columns").

### D-013 — Confirmations and change preview outside production
- **Question:** Should destructive-statement confirmations and the "preview the SQL before saving" step be on for non-production connections too?
- **Why it matters:** Confirmations prevent accidents but cause "click-through fatigue" on local databases where mistakes are cheap.
- **Options:** **A. Preview mandatory on production and for >100 deletes; optional elsewhere; destructive confirmations on for production only.** **B. On everywhere.** **C. Off everywhere except explicit settings.**
- **Recommended default:** A — strict where it matters, calm on local databases.
- **Components:** Hacivat.
- **Sources:** algorithms/23 PD-17, HV-EDIT-09; algorithms/13 SQ-8, SQL-SAFE-07.

### D-014 — Automatic rollback on production
- **Question:** Should Hacivat ever roll back a production transaction by itself (idle transaction left open; or a confirm dialog left open)?
- **Why it matters:** An open transaction holds locks that can block the application. Rolling back automatically protects the server but can throw away a user's uncommitted work.
- **Options:** **A. Idle transactions: notify only (Commit / Roll back / Keep); confirm-in-transaction dialog: auto-rollback after 5 min.** **B. Also auto-rollback idle transactions after a threshold** (DBeaver: 600 s on production). **C. Never auto-rollback.**
- **Recommended default:** A — locks held by a pending dialog are the dangerous case; idle work is never discarded silently.
- **Components:** Hacivat.
- **Sources:** algorithms/01 §7-2, HV-CONN-04, HV-SESS-03; algorithms/20 PD-3, HV-EXEC-10; clients/oss-clients-beekeeper-and-tui §8.

### D-015 — Keeping server cursors open for "fetch more"
- **Question:** After showing the first rows of a big result, should Hacivat keep the server cursor (and its transaction) open so "fetch more" is instant, or close it?
- **Why it matters:** An open cursor holds a snapshot and possibly locks on the server; closing it means "fetch more" re-runs the query.
- **Options:** **A. Keep a suspended cursor up to 10 min idle; read ahead 10,000 rows / 16 MiB.** **B. Drain and close right after read-ahead.** **C. Spill large results to a local file on request.**
- **Recommended default:** A (with C as explicit user action) — fluid browsing with a bounded server cost.
- **Components:** Hacivat.
- **Sources:** algorithms/21 PD-6, HV-RES-05; clients/dbeaver-04 §7 (cursor vs spill question).

### D-016 — Protecting local data at rest
- **Question:** (1) On Linux without a system keyring, should Hacivat offer its own master-password vault or refuse to save secrets? (2) Should the local schema cache be encrypted by default?
- **Why it matters:** A stolen laptop can reveal saved passwords and the structure of your databases. Encryption on Linux without a keyring means extra password prompts.
- **Options:** (1) **A. Master-password vault** / B. refuse to persist secrets. (2) **A. Plaintext with owner-only file permissions (0600), encrypted when the profile is marked sensitive or is managed** / B. always encrypted with a keychain-held key.
- **Recommended default:** (1) A and (2) A — protection where it matters without keyring prompts for every local database.
- **Components:** Hacivat.
- **Sources:** algorithms/01 §7-5, HV-SEC-05; algorithms/02 §6-1, HV-MODEL-03; clients/sql-tooling-04 §6.

### D-017 — Built-in system catalog data: embed or download
- **Question:** Hacivat needs data about each database's built-in objects (system tables, functions). Ship it inside the app or download it when needed?
- **Why it matters:** Embedding every engine and version adds roughly 10–20 MB to the app; downloading breaks "works fully offline with no login".
- **Options:** **A. Bundle in each platform installer.** B. Download on demand. C. Embed only the most common versions, introspect the rest live.
- **Recommended default:** A — the offline promise is part of the product's positioning.
- **Components:** Hacivat.
- **Sources:** algorithms/02 §6-3, HV-INTRO-16.

### D-018 — What remote users see of a managed database's schema
- **Question:** For databases behind Karagöz: should the agent introspect once and filter per user, or introspect as each user? Should autocomplete show objects the user cannot access? Should the UI say "some objects are hidden"?
- **Why it matters:** One shared catalog saves load on production databases, but only works if Karagöz policy can express everything database grants can. Showing forbidden objects leaks structure; hiding them silently confuses users.
- **Options:** **A. Shared agent catalog + policy filter; completion shows only allowed objects; reveal "some objects are hidden" only if org policy allows.** B. Per-user introspection with the user's principal. C. Hybrid.
- **Recommended default:** A — less load on production, no structure leak; revealing the filter is an org setting.
- **Components:** Karagöz, Hacivat, Perde.
- **Sources:** algorithms/02 §6-5, §6-6, HV-INTRO-17; algorithms/12 RQ-1; clients/dbeaver-02 §7.2; clients/sql-tooling-04 §7.

### D-019 — Read-only transactions across several requests
- **Question:** Should a client be able to hold one read-only transaction open across several queries (to get a consistent snapshot), or is each request its own transaction?
- **Why it matters:** Multi-request snapshots help analysts compare numbers consistently, but long-held transactions burden the server and complicate Karagöz's enforcement.
- **Options:** **A. One request = one transaction** (current decision). B. Client-held read-only transactions spanning requests (access/05 allowed it).
- **Recommended default:** A — simpler, safer enforcement; revisit if analysts ask.
- **Components:** Karagöz, Hacivat, MCP.
- **Sources:** algorithms/13 SQ-1 (S-03); access/05.

### D-020 — `SET` commands through Karagöz
- **Question:** Should harmless session settings (`SET search_path`, `SET TIME ZONE`, lowering `statement_timeout`) be allowed in read-only sessions through Karagöz in v1?
- **Why it matters:** Without them, pasted scripts that start with `SET` are rejected and users must set the schema in connection settings; each allowed `SET` is a possible bypass to verify.
- **Options:** **A. None in v1; schema via connection parameters; allowlist in v1.x.** B. Small allowlist in v1.
- **Recommended default:** A — confirm the UX cost is acceptable; reasons shown clearly on denial.
- **Components:** Karagöz, Hacivat.
- **Sources:** algorithms/31 §3-3, KG-EXEC-06; algorithms/13 SQ-4; clients/sql-tooling-05 §7.

### D-021 — Large exports and imports for agent databases
- **Question:** For databases behind Karagöz, should big exports/imports run on the agent (close to the data) instead of streaming everything through Hacivat?
- **Why it matters:** Streaming gigabytes through a laptop (possibly via relay) is slow and costly; running on the agent needs a storage and transfer design and policy controls.
- **Options:** **A. Run on Karagöz above a size threshold, via a dedicated export API.** B. Always through the normal query path.
- **Recommended default:** A, threshold to be set in the design — large transfers belong next to the data.
- **Components:** Karagöz, Hacivat, Perde (policy).
- **Sources:** algorithms/24 PD-20; algorithms/13 SQ-5 (S-12); clients/dbeaver-05 §7.2.

### D-022 — Approvals tied to exact SQL or to a fingerprint
- **Question:** When someone approves a statement, is the approval valid only for those exact bytes, or for any statement with the same normalized shape?
- **Why it matters:** Exact bytes is safest and ideal for AI proposals; tools that add comments or vary whitespace break it. Fingerprints are convenient but approve a wider set of statements.
- **Options:** **A. Exact bytes by default; fingerprint mode opt-in for low-risk classes, clearly labeled as wider.** B. Fingerprint by default.
- **Recommended default:** A.
- **Components:** Perde, Karagöz, MCP.
- **Sources:** access/05 §7-11; access/03 §7.1-6; access/02 §7.3.

### D-023 — Keeping SQL text in the local audit log
- **Question:** Should the audit log store full SQL text, or text with literal values (which may contain personal data) removed?
- **Why it matters:** Full text helps investigations; literals can hold PII (names, IDs), which creates privacy and KVKK/GDPR exposure.
- **Options:** **A. Literals redacted by default for AI events; full text opt-in.** B. Full text by default (literals redacted for PII-tagged tables only, access/05).
- **Recommended default:** A for AI events (the source recommendation); human events follow access/05 unless you choose to align them.
- **Components:** Karagöz, MCP, Hacivat.
- **Sources:** algorithms/27 U6; access/05; mcp/02; access/01 §7-4 (bind parameters may contain PII).

### D-024 — Database login inside the Karagöz stream
- **Question:** When Hacivat's driver talks to the database through Karagöz's already-authorized connection, does the database login step need its own one-time secret?
- **Why it matters:** The connection is already authenticated by a device-bound Perde grant; an extra one-time password adds defense in depth but more moving parts.
- **Options:** **A. Trust inside the authorized in-process stream** (current spec); the loopback forwarder for external tools keeps its one-time local credential. B. One-time password for every mode.
- **Recommended default:** A — the extra secret would not add identity; needs Karagöz proxy design sign-off.
- **Components:** Karagöz, Hacivat.
- **Sources:** algorithms/01 §7-6, HV-CONN-06; algorithms/31 KG-EXEC-02 step 7.

### D-025 — MySQL wire front-end in Karagöz v1
- **Question:** Ship Karagöz's MySQL protocol front-end in v1, or only PostgreSQL wire + native RPC?
- **Why it matters:** Without it, MySQL users in v1 can use Hacivat but not third-party MySQL tools through the forwarder.
- **Options:** **A. v1.x.** B. v1 (more scope, more bypass testing).
- **Recommended default:** A.
- **Components:** Karagöz.
- **Sources:** algorithms/31 §3-2.

### D-026 — Netns helper (container network access) in v1
- **Question:** Should Karagöz v1 include the helper that enters a container's network, which needs Docker's most dangerous API (creating containers)?
- **Why it matters:** Without it, PostgreSQL instances whose environment password was rotated need a manual operator override; with it, the agent's privileges grow.
- **Options:** **A. Not in v1; accept operator override friction.** B. Include as opt-in.
- **Recommended default:** A.
- **Components:** Karagöz.
- **Sources:** algorithms/30 §4-1; access/04 §7-7.

### D-027 — AI access to Karagöz-managed databases in v1
- **Question:** Should the MCP server work with Karagöz-managed databases in Hacivat v1, or only with direct connections?
- **Why it matters:** The managed path needs attenuated grants and Perde co-approval; it is a significant addition to v1 scope.
- **Options:** **A. v1.x.** B. v1.
- **Recommended default:** A.
- **Components:** MCP, Karagöz, Perde.
- **Sources:** algorithms/27 U11, MCP-18, MCP-19.

### D-028 — AI hosts that sandbox local servers
- **Question:** Some AI apps sandbox the MCP process so it cannot reach Hacivat's local socket. Fall back to a loopback HTTP endpoint, or declare those hosts unsupported?
- **Why it matters:** HTTP on localhost is a bigger attack surface; unsupported hosts lose users.
- **Options:** **A. Run a compatibility spike per host before v1; HTTP fallback in v1.x.** B. Ship HTTP fallback in v1. C. Unsupported.
- **Recommended default:** A.
- **Components:** MCP.
- **Sources:** algorithms/27 U10, MCP-04; mcp/02 §7.

### D-029 — Default keyboard shortcuts
- **Question:** Copy DataGrip's shortcuts exactly, or use a collision-safe preset? What does `Ctrl+Y` do?
- **Why it matters:** Some DataGrip shortcuts collide with characters on Polish/German keyboards (`Ctrl+Alt+L` types `ł`); `Ctrl+Y` is Delete line in IntelliJ but Redo on Windows.
- **Options:** **A. Collision-safe "hacivat" preset as default, exact "datagrip" preset opt-in, first-run keymap choice (DataGrip-like / VS Code-like); `Ctrl+Y` = delete line in those presets; keep `Ctrl+U` (undo selection).** B. DataGrip-exact default.
- **Recommended default:** A.
- **Components:** Hacivat, i18n.
- **Sources:** algorithms/25 P1, P2, P10, UI-EDIT-13; i18n/02 §3.11.2.

### D-030 — Turkish glossary and terminology
- **Question:** Which Turkish words do we use for core database terms, and what stays in English?
- **Why it matters:** Wrong or inconsistent terms make the Turkish UI feel amateur; changing them after the v1 string freeze is costly.
- **Options per term:** `sütun` vs `kolon` (column); `işlem` for transaction (clashes with "operation"); translate `commit` (`onayla`) vs keep `COMMIT`; `fonksiyon` vs `işlev`; `sequence`; "relay" → `aktarıcı (relay)` vs untranslated; placeholders like `<none>` translated or not; shortcut modifier labels `Ctrl/Alt/Shift` (as printed on keys) vs Turkish names (`Üst Karakter`).
- **Recommended default:** decide with input from Turkish database users before the v1 string freeze; meanwhile keep SQL keywords and acronyms (NAT, UDP, QUIC) untranslated and modifier labels as printed on keys.
- **Components:** i18n, Hacivat, Perde.
- **Sources:** algorithms/26 §5-3, §5-11; i18n/02 §7-12; networking/01 §6 (relay terminology); datagrip/12 §7 (sentinel under-translation).

### D-031 — gpui-component's untranslatable strings
- **Question:** gpui-component localizes with a different system (rust-i18n YAML, no Turkish). Avoid its visible widgets, contribute Turkish upstream, or fork?
- **Why it matters:** Decides how much of gpui-component Hacivat can borrow in v1 without breaking "Turkish from day one".
- **Options:** **A. Never show its user-visible widgets (wrap or replace).** B. Contribute `tr` upstream and bridge the locale. C. Fork.
- **Recommended default:** A — consistent with ADR 0001's plan to replace gpui-component incrementally.
- **Components:** Hacivat, i18n.
- **Sources:** algorithms/26 §5-5; decisions/0001 §Decision 2.

### D-032 — Hosted telemetry and data residency
- **Question:** Is telemetry on by default in hosted Perde, and do we offer a Türkiye data region?
- **Why it matters:** Telemetry helps measure P2P success rates; personal data and query metadata fall under KVKK/GDPR. Turkish banks may require local residency.
- **Options:** Telemetry: A. on by default (networking/05) / B. opt-in. Residency: A. Türkiye region at launch / B. later.
- **Recommended default:** decide after the KVKK/GDPR review; until then, no telemetry without opt-in (self-hosted never phones home).
- **Components:** Perde.
- **Sources:** algorithms/33 §2-5; algorithms/34 §2-5, PD-CP-09; networking/05 §3.9; access/06 §5.3-5, §6.

### D-033 — SAML single sign-on on hosted Perde
- **Question:** Is SAML free on hosted Perde, or included from the first paid tier?
- **Why it matters:** Many Turkish enterprises, banks and universities use SAML-only identity providers (an early market); "SSO tax" draws criticism.
- **Options:** **A. Free self-hosted; included in the first paid hosted tier with published pricing.** B. Free on hosted too.
- **Recommended default:** A — OIDC is already free, so security is not paywalled.
- **Components:** Perde.
- **Sources:** algorithms/34 §2-2, PD-CP-02; access/06 §5.2 (SAML discussion).

### D-034 — High availability as a paid feature
- **Question:** Is running two or more Perde replicas (high availability) a paid self-host feature or free?
- **Why it matters:** HA is a revenue lever, but regulated self-hosters (banks) may need it to adopt at all; NetBird gating it is seen as a gap competitors can exploit.
- **Options:** **A. Paid on self-host (EE), included in hosted; single-node always free and never stopped.** B. Free HA.
- **Recommended default:** A (as provisionally specified in algorithms/34 P1) — revisit if design partners in banking push back.
- **Components:** Perde.
- **Sources:** algorithms/34 §2-3, P1, PD-CP-06; access/06 §5.2; networking/05 §3.11; networking/03 §7.1-9 and HA table.

---

## P2 — safe default now, revisit later

### D-035 — Connection budget per profile
- **Question:** How many simultaneous database connections may one remote profile open, and what happens beyond that?
- **Why it matters:** Too many connections can hit server limits (a known DBeaver issue); too few makes consoles wait.
- **Options:** **A. 4 per remote profile; extra consoles queue.** B. Higher limit. C. Fail beyond budget.
- **Recommended default:** A.
- **Components:** Hacivat.
- **Sources:** algorithms/01 §7-3, HV-SESS-01; clients/dbeaver-01 §7.1-1.

### D-036 — Smart commit
- **Question:** Should Hacivat automatically switch to manual-commit mode when a data-changing statement runs ("smart commit") on test/production connections?
- **Why it matters:** It prevents accidental auto-commits, but its keyword detection can misclassify (DBeaver issues).
- **Options:** **A. Opt-in everywhere; production defaults to Manual anyway.** B. On by default for test/production.
- **Recommended default:** A.
- **Components:** Hacivat.
- **Sources:** algorithms/01 §7-4, HV-SESS-04; clients/dbeaver-01 §7.1-8.

### D-037 — `ProxyCommand` support
- **Question:** Should SSH `ProxyCommand` (which runs an arbitrary local command) be supported?
- **Why it matters:** Some corporate SSH setups need it; it is arbitrary command execution from a config file.
- **Options:** **A. Not in v1; opt-in in v1.x at most.** B. Never.
- **Recommended default:** A.
- **Components:** Hacivat.
- **Sources:** algorithms/01 §7-7.

### D-038 — Introspection depth thresholds
- **Question:** At what object counts should Hacivat load less detail about a schema (sources → structure → names only)?
- **Why it matters:** Loading everything on huge schemas is slow; loading too little hurts completion.
- **Options:** **A. 2,000 / 20,000 / 5,000 objects (current / structure / other namespaces), confirmed by benchmark on spike machines.** B. Other values after benchmarking.
- **Recommended default:** A.
- **Components:** Hacivat.
- **Sources:** algorithms/02 §6-2, HV-INTRO-01.

### D-039 — Mass-deletion guard UX
- **Question:** If a refresh suddenly shows many objects gone, ask the user or just mark the cache stale?
- **Why it matters:** Permission errors can look like deletions; a wrong refresh wipes the explorer.
- **Options:** **A. Confirm when the user triggered the refresh; "stale" badge for background passes** (*register proposal* combining the two source options). B. Always confirm. C. Always silent badge.
- **Recommended default:** A.
- **Components:** Hacivat.
- **Sources:** algorithms/02 §6-4, HV-INTRO-11; datagrip/04 §7.

### D-040 — Oracle support approach
- **Question:** How do we add Oracle: native client library (ODPI-C/Instant Client) or pure Rust; and what may Karagöz do with Oracle before a full Oracle parser exists?
- **Why it matters:** Oracle's client library complicates packaging and licensing; without a parser, safety classification is weak.
- **Options:** Driver: A. ODPI-C/Instant Client / B. pure-Rust thin driver (none mature). Karagöz: **A. deny everything except an allowlisted read subset** / B. no Oracle in Karagöz until the parser.
- **Recommended default:** Oracle stays out of v1 (not in PRODUCT.md's engine list); when added, Karagöz uses the allowlist (current decision) and the driver choice is made then.
- **Components:** Hacivat, Karagöz.
- **Sources:** algorithms/02 §6-7, HV-INTRO-10; algorithms/11 §4-2.

### D-041 — Rename detection in schema compare
- **Question:** When comparing schemas, should Hacivat suggest "this column was renamed" instead of "dropped + added"?
- **Why it matters:** A wrong drop+add loses data; a wrong rename guess is confusing.
- **Options:** **A. Suggest (unchecked suggestions).** B. Off.
- **Recommended default:** A.
- **Components:** Hacivat.
- **Sources:** algorithms/03 §7-1, HV-DDL-03; datagrip/09 §7.

### D-042 — Online DDL defaults on production
- **Question:** On production, generate "online" DDL by default (`CREATE INDEX CONCURRENTLY`, FK `NOT VALID` + `VALIDATE`, SQL Server `ONLINE=ON`)?
- **Why it matters:** Online DDL avoids locking the app but splits work across transactions (partial states on failure).
- **Options:** **A. On for production, off otherwise.** B. Off everywhere. C. On everywhere.
- **Recommended default:** A (depends on D-004).
- **Components:** Hacivat.
- **Sources:** algorithms/03 §7-2, HV-DDL-05, HV-DDL-11.

### D-043 — Undo scripts for non-transactional engines
- **Question:** For MySQL/MariaDB/ClickHouse schema changes (which cannot be rolled back), offer a generated reverse script?
- **Why it matters:** Helpful recovery path, but it cannot restore dropped data, which must be explained.
- **Options:** **A. Offer with explicit "cannot restore dropped data" messaging.** B. Do not offer.
- **Recommended default:** A (*register proposal*; the source only frames the question).
- **Components:** Hacivat.
- **Sources:** algorithms/03 §7-3, HV-DDL-18.

### D-044 — "Apply anyway" after schema drift on production
- **Question:** If the target schema changed since the compare, may the user still apply on production?
- **Why it matters:** Applying on a drifted schema can break things; blocking can stall urgent fixes.
- **Options:** A. Allow with typed confirmation. B. Require a Perde approval token verified at Karagöz. C. Never allow.
- **Recommended default:** B for managed databases, A for local connections (*register proposal*; source ties it to the open-core split, D-003, where single-approver flows are free).
- **Components:** Hacivat, Perde, Karagöz.
- **Sources:** algorithms/03 §7-4, HV-DDL-19.

### D-045 — Default constraint placement in generated DDL
- **Question:** Put constraints inside the table definition or inline on the column (DataGrip's choice)?
- **Why it matters:** Visible in generated scripts and tests; low risk.
- **Options:** **A. Inside table.** B. Inside column (DataGrip).
- **Recommended default:** A.
- **Components:** Hacivat.
- **Sources:** algorithms/03 §7-5, HV-DDL-09.

### D-046 — Schema tooling scope
- **Question:** (1) Cross-database migration scripts (e.g. MySQL → PG)? (2) Import DataGrip's dump folder layout to help migrants? (3) How deep should Git integration go?
- **Why it matters:** Each is a sizable roadmap item.
- **Options:** (1) **report-only in v1** / type-mapped migrations; (2) **no** / yes; (3) **read-only `git status` check in v1.x** / full commit/push UI.
- **Recommended default:** the bold options.
- **Components:** Hacivat.
- **Sources:** algorithms/03 §7-6, §7-7, §7-8, HV-DDL-01, HV-DDL-15, HV-DDL-16.

### D-047 — Small Redis/Mongo defaults
- **Question:** (1) Mongo document order when no sort is given; (2) show Redis `MEMORY USAGE` in the key browser; (3) does Mongo `5.0` mean an integer (like `mongosh`) or a double?
- **Why it matters:** Stable order enables paging; memory sampling costs server time; literal types change stored data.
- **Options:** (1) **natural order + one-click "stable order (_id)" remembered per collection** / `_id` order; (2) **off; opt-in sampled estimate** / always; (3) **follow mongosh, warn and offer `Double(5)`** / treat as Double.
- **Recommended default:** the bold options.
- **Components:** Hacivat.
- **Sources:** algorithms/04 U1, U7, U9.

### D-048 — Hand-off to external `mongosh`
- **Question:** For Mongo scripts Hacivat cannot run, offer "Open in mongosh"?
- **Why it matters:** Convenience vs bypassing Hacivat's safety layer.
- **Options:** never / **opt-in on local read-write connections only (never read-only, AI or Karagöz)** / always.
- **Recommended default:** opt-in on local read-write only (v1.x).
- **Components:** Hacivat.
- **Sources:** algorithms/04 U2.

### D-049 — Server-side JavaScript in read-only Mongo sessions
- **Question:** Allow `$where` / `$function` in human read-only sessions?
- **Why it matters:** Server-side JavaScript can be expensive or abused; analysts sometimes need it.
- **Options:** allow / **confirm once per session** / deny. AI and Karagöz `deny_server_js` always deny.
- **Recommended default:** confirm once per session.
- **Components:** Hacivat, Karagöz, MCP.
- **Sources:** algorithms/04 U3, HV-NOSQL-20.

### D-050 — Redis exact count and bulk actions on production
- **Question:** On production, may users run "count exactly" (full scan) and bulk delete freely?
- **Why it matters:** Both can load or damage a production Redis.
- **Options:** always / **confirm, plus a Karagöz role gate for remote sessions** / role-gated only.
- **Recommended default:** confirm + role gate.
- **Components:** Hacivat, Karagöz.
- **Sources:** algorithms/04 U4.

### D-051 — Bundled Redis command metadata
- **Question:** Ship Redis command metadata with Hacivat (for completion and safety) or only read it from the server?
- **Why it matters:** Licensing of the data; offline completion.
- **Options:** **bundle Valkey's BSD-3 JSON (license re-check at vendoring), prefer live server data** / runtime only.
- **Recommended default:** bundle + prefer live.
- **Components:** Hacivat.
- **Sources:** algorithms/04 U5; clients/oss-clients-redisinsight-compass §8.

### D-052 — Schema inference on SQL JSON columns
- **Question:** Run the Mongo schema sampler on JSON/JSONB columns in SQL databases too?
- **Why it matters:** Useful structure view for JSON-heavy tables at little extra cost.
- **Options:** **yes, same code path** / no.
- **Recommended default:** yes.
- **Components:** Hacivat.
- **Sources:** clients/oss-clients-redisinsight-compass §8.

### D-053 — Parameters and script variables
- **Question:** (1) Support SQL*Plus `&var` / sqlcmd `$(var)` substitution and our own `@set` commands? (2) Enable DataGrip-style text templates by default for migrants? (3) Auto-enable PG `?` placeholders for connections imported from JDBC tools?
- **Why it matters:** Text substitution is an SQL-injection and quoting footgun; migrants expect it to work.
- **Options:** (1) **detect + warn in v1, prompts later** / implement now; (2) **ship disabled** / enabled; (3) **no** (`?` collides with JSONB operators) / yes.
- **Recommended default:** the bold options; bind parameters are the default mode.
- **Components:** Hacivat.
- **Sources:** algorithms/10 §6-2, §6-3, SQL-PARAM-01, SQL-PARAM-02; algorithms/20 PD-2, HV-EXEC-03; clients/sql-tooling-02 §7; datagrip/07 §7.

### D-054 — Server-side parse probe and typing depth
- **Question:** Offer an opt-in "ask the server to parse this" check (PostgreSQL) and how much type inference to build locally?
- **Why it matters:** Server probes are precise but send text to production; full local typing is a big engineering effort.
- **Options:** **light local typing + opt-in PG probe in v1.x with consent UX on production** / full local typing / no probe.
- **Recommended default:** the bold option.
- **Components:** Hacivat.
- **Sources:** algorithms/11 §4-1, SQL-SEM-03; clients/sql-tooling-04 §7; clients/sql-tooling-01 §8.

### D-055 — Name resolution mode defaults
- **Question:** Default "Playground" (each statement independent) for consoles and "Script" (statements see earlier ones) for files, and show the mode in the status bar?
- **Why it matters:** Wrong mode gives misleading "unknown table" warnings.
- **Options:** **yes, shown in status bar** / one mode everywhere.
- **Recommended default:** yes.
- **Components:** Hacivat.
- **Sources:** algorithms/12 RQ-2.

### D-056 — JOIN completion style
- **Question:** Order of generated join conditions and alias numbering.
- **Why it matters:** Cosmetic, but visible daily.
- **Options:** **`existing.col = new.col`, aliases `c2`** / DataGrip-like `c1`.
- **Recommended default:** the bold option, confirm with UX.
- **Components:** Hacivat.
- **Sources:** algorithms/12 RQ-3.

### D-057 — Sync virtual foreign keys through Perde
- **Question:** Should user-defined "virtual" relationships between tables be shared across a team through Perde?
- **Why it matters:** Team value vs open-core scope.
- **Options:** **local only in v1** / org-level sync.
- **Recommended default:** local only in v1.
- **Components:** Hacivat, Perde.
- **Sources:** algorithms/12 RQ-4.

### D-058 — MySQL executable comments in read-only/AI profiles
- **Question:** Allow `/*! ... */` comments (which MySQL executes) when their contents are pure reads?
- **Why it matters:** They are a classic bypass vector; only mysqldump-style scripts need them.
- **Options:** **deny by default** / allow when expanded effects are reads.
- **Recommended default:** deny.
- **Components:** Karagöz, MCP, Hacivat.
- **Sources:** algorithms/13 SQ-3 (S-06); clients/sql-tooling-02 §7.

### D-059 — Temp tables for analysts in read-only sessions
- **Question:** Allow creating temporary tables in read-only sessions?
- **Why it matters:** Analysts use them; they are writes and need privileges.
- **Options:** **denied in v1** / allow `WriteTemp` level.
- **Recommended default:** denied in v1.
- **Components:** Karagöz, Hacivat.
- **Sources:** algorithms/13 SQ-6.

### D-060 — SQL formatter defaults
- **Question:** Keyword case, comma style, indent, width, preset names, format-on-save.
- **Why it matters:** Taste; changing defaults later reformats users' files.
- **Options:** **presets (e.g. compact, aligned) with ~30 options; format-on-save off** / DataGrip-like ~190 options.
- **Recommended default:** presets, format-on-save off; pick the default preset with UX research.
- **Components:** Hacivat.
- **Sources:** algorithms/13 SQ-7; clients/sql-tooling-05 §7; datagrip/11 §7.

### D-061 — Query history retention
- **Question:** How much query history to keep, and record AI-originated statements?
- **Why it matters:** Usefulness vs disk use and secrets in history.
- **Options:** **10,000 entries per data source and 90 days; AI statements recorded and filterable** / unlimited.
- **Recommended default:** the bold option, with private consoles and per-data-source opt-out.
- **Components:** Hacivat, MCP.
- **Sources:** algorithms/20 PD-4, HV-EXEC-12; datagrip/10 §7.

### D-062 — EXPLAIN ANALYZE on writes
- **Question:** "Explain Analyze" actually runs the statement; wrap writes in a rollback by default?
- **Why it matters:** Otherwise analyzing an `INSERT` inserts rows.
- **Options:** **rolled back by default** / executed.
- **Recommended default:** rolled back; confirm users accept "plan shown, no rows left".
- **Components:** Hacivat.
- **Sources:** algorithms/20 PD-5; datagrip/07 §7.

### D-063 — Result memory budgets
- **Question:** Memory per result and in total before showing the "memory limit" banner.
- **Why it matters:** Stability with millions of rows vs how often users hit the limit.
- **Options:** **256 MiB per result, min(25% RAM, 4 GiB) total** / other values.
- **Recommended default:** the bold option.
- **Components:** Hacivat.
- **Sources:** algorithms/21 PD-7, HV-RES-07.

### D-064 — Grid number/date display
- **Question:** Show numbers and dates in raw machine format (`1234.5`, ISO dates) or localized (`1.234,5` for Turkish)?
- **Why it matters:** Raw is exact and copy-friendly; localized is familiar to business users.
- **Options:** **raw by default for every locale, localized opt-in** / localized by default for Turkish users.
- **Recommended default:** raw.
- **Components:** Hacivat, i18n.
- **Sources:** algorithms/22 PD-10, HV-GRID-10; algorithms/26 §5-1; i18n/02.

### D-065 — Grid display details
- **Question:** (1) MySQL `TINYINT(1)` as number or boolean; (2) NULL label in Turkish UI; (3) does a local filter pause automatic fetching?
- **Why it matters:** Fidelity and copy behavior vs familiarity.
- **Options:** (1) **number** / boolean; (2) **ASCII `NULL` everywhere** / localized (`BOŞ`); (3) **pause** / keep fetching.
- **Recommended default:** the bold options.
- **Components:** Hacivat.
- **Sources:** algorithms/21 PD-9; algorithms/01 HV-DRV-01; algorithms/22 PD-12, PD-13.

### D-066 — Editing scope in v1
- **Question:** (1) Edit through views? (2) Use physical row ids (SQLite `rowid`, PG `ctid`) when there is no key? (3) Edit join results? (4) Edit ClickHouse data?
- **Why it matters:** Each widens what users can do but adds risk of updating the wrong rows.
- **Options:** (1) **off** / on for simple views; (2) **opt-in** / never; (3) **single-table only in v1** / per-table updates in v1.x; (4) **read-only in v1** / async mutations with warning.
- **Recommended default:** the bold options.
- **Components:** Hacivat.
- **Sources:** algorithms/21 PD-8; algorithms/23 PD-15, PD-16, PD-18, HV-EDIT-01; clients/dbeaver-04 §7.

### D-067 — Spreadsheet formula-injection guard
- **Question:** Prefix cells that start with `=`, `+`, `-`, `@` so spreadsheets don't run them as formulas?
- **Why it matters:** Safety in Excel vs data fidelity (text `-5` changes).
- **Options:** **on for spreadsheet presets only** / on everywhere (DBeaver).
- **Recommended default:** spreadsheet presets only.
- **Components:** Hacivat.
- **Sources:** algorithms/24 PD-19; clients/dbeaver-04 §7.

### D-068 — Import commit strategy
- **Question:** Commit an import as one transaction or in batches?
- **Why it matters:** One transaction is simplest to reason about; big imports then hold long transactions.
- **Options:** **single transaction for new tables on engines with transactional DDL, else every 50,000 rows** / always single.
- **Recommended default:** the bold option.
- **Components:** Hacivat.
- **Sources:** algorithms/24 PD-21, HV-XFER-09.

### D-069 — Regional formatting in clipboard and CSV
- **Question:** When copying to spreadsheets or exporting CSV for Turkish users, use Turkish regional format (`;` separators, `1.234,5`) or raw?
- **Why it matters:** Turkish Excel misreads raw numbers; regional formats break other tools.
- **Options:** HTML clipboard: **regional** / raw (pending a manual Excel/LibreOffice/Sheets test). CSV: **RFC 4180 default, suggest `excel_regional` for `tr-*`** / regional by default.
- **Recommended default:** the bold options.
- **Components:** Hacivat, i18n.
- **Sources:** algorithms/24 PD-22; algorithms/26 §5-4, §5-9; i18n/02 §7-6.

### D-070 — Big numbers in JSON export
- **Question:** Write integers beyond 2^53 and long decimals as JSON strings or numbers?
- **Why it matters:** JavaScript parsers silently corrupt big numbers.
- **Options:** **strings by default** / numbers.
- **Recommended default:** strings.
- **Components:** Hacivat.
- **Sources:** algorithms/24 PD-23.

### D-071 — Editor convenience defaults
- **Question:** (1) Preserve file line endings or normalize to LF; (2) large-file thresholds; (3) completion popup after `FROM ` / `JOIN `; (4) soft wrap for SQL.
- **Why it matters:** Daily feel; line endings matter inside multi-line string literals.
- **Options:** (1) **preserve** / normalize; (2) **three tiers 4 MB / 20 MB / 2.5 MB preview, calibrate in spike** / single 20 MB limit; (3) **on (setting)** / off; (4) **off** / on.
- **Recommended default:** the bold options.
- **Components:** Hacivat.
- **Sources:** algorithms/25 P3, P5, P6, P7, UI-EDIT-02, UI-EDIT-23.

### D-072 — IME composition with multiple cursors
- **Question:** When typing with an input method (e.g. Japanese) and several cursors, mirror the composition to all cursors?
- **Why it matters:** Convenience vs platform bugs.
- **Options:** **mirror, behind a setting; fall back to primary-only if the spike finds issues** / primary only.
- **Recommended default:** the bold option.
- **Components:** Hacivat.
- **Sources:** algorithms/25 P4, UI-EDIT-12.

### D-073 — Screen-reader exposure of the editor
- **Question:** Expose a window of lines around the cursor or the whole document to screen readers?
- **Why it matters:** Whole-document exposure can be slow on big files.
- **Options:** **±100-line window by default; "screen reader optimized" mode exposes up to 2 MB** / whole document.
- **Recommended default:** the bold option.
- **Components:** Hacivat.
- **Sources:** algorithms/25 P9, UI-EDIT-24.

### D-074 — Turkish-tolerant search
- **Question:** Should search treat I/i/İ/ı as the same letter and ignore accents (`musteri` finds `müşteri`) always, or only for Turkish users?
- **Why it matters:** Helpful for Turkish text, surprising for exact matches (`sık` vs `sik`).
- **Options:** always / **only when UI language or regional format is Turkish, with a toggle in the search box (remembered)**.
- **Recommended default:** the bold option (see Appendix B on the UI-search default).
- **Components:** Hacivat, i18n.
- **Sources:** algorithms/25 P11, UI-EDIT-22; algorithms/26 §5-2, I18N-09; i18n/02 §7-7.

### D-075 — Bidi isolation marks in GPUI text
- **Question:** If invisible direction marks (U+2068/U+2069) show up as glyphs on some OS, strip them in the GPUI adapter or patch text rendering?
- **Why it matters:** Visible junk characters vs correct mixed-direction text later.
- **Options:** **keep; strip only for LTR-only chains in v1 if the spike shows glyphs** / patch the adapter.
- **Recommended default:** the bold option.
- **Components:** Hacivat, i18n.
- **Sources:** algorithms/26 §5-6.

### D-076 — Right-to-left mirroring
- **Question:** Mirror the grid and UI chrome for RTL languages (Arabic, Persian)?
- **Why it matters:** Not needed for EN + TR; required before accepting an RTL language pack.
- **Options:** **decide before the first RTL pack; RTL packs load with a warning until then** / build now.
- **Recommended default:** the bold option.
- **Components:** Hacivat, i18n.
- **Sources:** algorithms/22 PD-11; datagrip/12 §7 (RTL); i18n/01 §7-10.

### D-077 — Translation workflow
- **Question:** (1) Weblate hosting; (2) show incomplete languages in releases; (3) where community language packs live; (4) docs language in v1.
- **Why it matters:** Contributor experience and release quality.
- **Options:** (1) **in-house Turkish via PRs for v1; apply for Hosted Weblate Libre early, self-host as plan B** / self-host from day one / Pontoon later; (2) **behind "Show incomplete languages" (50 % / 90 % thresholds proposed)** / hide; (3) **Turkish in main repo, others in a `hacivat-locales` repo with the same CI** / all in main repo; (4) **English-only docs in v1** / Turkish docs now.
- **Recommended default:** the bold options.
- **Components:** i18n, Hacivat, Perde.
- **Sources:** algorithms/26 §5-7, §5-8, §5-12, I18N-19, I18N-24; i18n/01 §7-5, §7-6; datagrip/12 §7 (community packs).

### D-078 — Force English server messages for AI sessions
- **Question:** For AI sessions, set the database's message language to English (e.g. MySQL `lc_messages`) even though the user did not set it?
- **Why it matters:** Models handle English errors better; it changes a session setting silently.
- **Options:** **yes, AI-originated sessions only** / no.
- **Recommended default:** yes.
- **Components:** MCP, i18n.
- **Sources:** algorithms/26 §5-10, I18N-20.

### D-079 — Small MCP defaults
- **Question:** (1) Result size budget per AI call; (2) scope of the key used for "hash" masking; (3) auto-start Hacivat when an AI app launches the MCP shim.
- **Why it matters:** (1) AI host truncation; (2) privacy vs joining masked values across sessions; (3) convenience vs surprise.
- **Options:** (1) **96 KiB / 20k tokens, per-host override after measuring** / 256 KiB; (2) **per broker run; persistent per connection opt-in** / persistent; (3) **on, with first-time notice and setting; auto-quit when idle** / off.
- **Recommended default:** the bold options.
- **Components:** MCP.
- **Sources:** algorithms/27 U7, U8, U9; mcp/02 §7.

### D-080 — AI-proposed GRANT/REVOKE/role changes
- **Question:** May AI agents propose permission changes?
- **Why it matters:** Privilege escalation via a manipulated model.
- **Options:** **deny by default** / allow with co-approval.
- **Recommended default:** deny.
- **Components:** MCP.
- **Sources:** algorithms/27 U12, MCP-08 step 6.

### D-081 — Future MCP surfaces
- **Question:** (1) A Perde-hosted MCP endpoint for cloud AI agents; (2) publishing to the MCP Registry; (3) an MCP Apps result viewer inside AI hosts.
- **Why it matters:** (1) conflicts with "Perde carries no query traffic except relay fallback"; (2)–(3) reach vs scope.
- **Options:** (1) **later, as a relay-only broker with its own threat model** / v1; (2) **evaluate later (desktop app may not fit registry package types)**; (3) **later spike, display only, never approvals**.
- **Recommended default:** the bold options.
- **Components:** MCP, Perde.
- **Sources:** algorithms/27 U13; mcp/02 §7; mcp/01 §8.

### D-082 — Trusting `hacivat.*` labels for auto-attach
- **Question:** If a database container carries Hacivat labels, attach it automatically without an admin click?
- **Why it matters:** Convenience vs anyone who can label a container on that host influencing what gets exposed.
- **Options:** **`trustLabels` off by default (admin click in Perde)** / on.
- **Recommended default:** off; revisit after design-partner feedback.
- **Components:** Karagöz, Perde.
- **Sources:** algorithms/30 §4-3, KG-DISC-04, KG-DISC-06.

### D-083 — Advanced trust features
- **Question:** When to add (1) org-root co-signing / dual control for high-privilege grants, (2) an audit witness independent of Perde, (3) non-Biscuit tokens (PASETO/JWT) for integrations such as CI.
- **Why it matters:** They reduce what a compromised Perde can do and help integrations, at significant complexity.
- **Options:** **later, by customer demand (design keeps the slots)** / v1.
- **Recommended default:** later.
- **Components:** Perde, Karagöz.
- **Sources:** algorithms/32 §2-3; networking/05 §7.1-5, §7.3-6; access/05 §7-2, §7-9, §7-10.

### D-084 — Approval rules expressiveness in v1
- **Question:** Do v1 approval rules need a predicate language (Teleport-style filters), or is "N approvals from group G" enough?
- **Why it matters:** Expressiveness vs complexity and misconfiguration.
- **Options:** **N approvals from group G** / predicate filters.
- **Recommended default:** N approvals from group G (rich rules are paid per D-003).
- **Components:** Perde.
- **Sources:** access/01 §7 ("Open for Perde").

### D-085 — Session resume in v1
- **Question:** Should a database session survive a network drop (resume where it left off), or is "session lost; open transactions rolled back" plus fast reconnect fine for v1?
- **Why it matters:** Laptops sleep and switch networks; resume is complex and must be exactly-once.
- **Options:** **v1.x; v1 has QUIC migration + reconnect + clear message** / v1 if design partners require it.
- **Recommended default:** v1.x.
- **Components:** Perde, Karagöz, Hacivat.
- **Sources:** algorithms/33 §2-3, N7, PD-NET-08; networking/05 §7.1-6, §7.3-8.

### D-086 — Very restrictive corporate networks
- **Question:** Invest in support for proxies that strip WebSocket upgrades (custom relay transport) and NTLM/Kerberos proxy authentication?
- **Why it matters:** Some corporate networks will otherwise not be able to use Perde at all.
- **Options:** **document as unsupported in v1; revisit with demand** / build custom transport and proxy auth.
- **Recommended default:** the bold option (HTTP CONNECT proxies are supported in v1).
- **Components:** Perde, Hacivat, Karagöz.
- **Sources:** algorithms/33 §2-1, §2-2, N4, PD-NET-12; networking/04 §7-5; networking/05 §7.1-4, §7.3-4.

### D-087 — Aggressive NAT probing ("birthday" probing)
- **Question:** Use many-port probing to get direct connections through hard NATs?
- **Why it matters:** More direct connections vs firewall overload and abuse complaints to the hosted service.
- **Options:** **off by default; org opt-in later** / on in hosted.
- **Recommended default:** off.
- **Components:** Perde, Hacivat, Karagöz.
- **Sources:** algorithms/33 PD-NET-13; networking/01 §7.1-3, §7.2-6.

### D-088 — Relays inside customer networks
- **Question:** Allow customer-run peer relays, and who may be one?
- **Why it matters:** Cuts relay cost and latency; risks leaking network topology.
- **Options:** **later** / v1.
- **Recommended default:** later.
- **Components:** Perde, Karagöz.
- **Sources:** networking/05 §7.3-7; algorithms/33 PD-NET-06.

### D-089 — Lease mirror in Perde; OpenBao backend
- **Question:** (1) Should Perde keep a copy of which temporary database credentials exist (for an "active credentials" view and offline-agent alerts)? (2) Should Karagöz be able to get credentials from OpenBao for orgs that require a central secrets audit?
- **Why it matters:** (1) visibility vs a second source of truth; (2) compliance fit vs reintroducing standing root credentials.
- **Options:** (1) **yes, eventually consistent, Karagöz authoritative** / no; (2) **later** / v1.
- **Recommended default:** the bold options.
- **Components:** Perde, Karagöz.
- **Sources:** access/03 §7.2.

### D-090 — Several agents per database (agent HA)
- **Question:** One Karagöz per database, or several for high availability?
- **Why it matters:** HA requires distributing credential leases across agents.
- **Options:** **single agent per database in v1** / multiple.
- **Recommended default:** single (*register proposal*; the source only raises the question).
- **Components:** Karagöz, Perde.
- **Sources:** access/01 §7 ("Open for Karagöz").

---

## Appendix A — Checked and excluded

Not listed as decisions because they are already decided, are engineering choices, or are research/verification work rather than product choices:

- **Decided elsewhere:** UI stack and component library (ADR 0001; supersedes ui-01's React vs SolidJS question); grant format Biscuit and approval-token format (algorithms/32 A1, A8; closes mcp/02's "COSE vs JWS"); relay licensing split (algorithms/34 P3); shared crate naming `hv-*` (34 P2); HA as provisional EE (34 P1, still open as D-034); variable row heights v1.x (algorithms/22); owned-object policy on revoke (`REASSIGN OWNED`, algorithms/30 KG-CRED-06; closes access/03's question); read-replica tier mode (algorithms/30 KG-CRED-03); per-stream authorization with signed grants (algorithms/32; closes networking/02 open question 4); TURN not needed, custom/iroh relay (networking/05 §5.1); SQL Server Kerberos/AD out of v1 (access/04 §7-10).
- **Engineering, not product:** ropey version (algorithms/25 P8), Redis client crate (algorithms/04 U6), Biscuit adoption benchmark (32 §2-4), iroh version pin (33 §2-4), tree-sitter need, rowan vs slots, salsa pinning (clients/sql-tooling-01/03), Arrow vs custom columnar pages (dbeaver-05; ADR 0001 removed the IPC transport), pg_depend loading depth (dbeaver-02), QAD-only reflector servers (networking/04 §7-14).
- **Verification gates, not decisions:** engines must pass the bypass harness before being labeled "read-only enforced" (algorithms/31 §3-4; algorithms/30 §4-2); all `[Unknown]` items in datagrip/*, clients/*, networking/* that are facts to measure (NAT rates, proxy behavior, Excel paste behavior, Turkish keyboard maps).
- **Brand:** shadow-theatre heritage visibility is listed as undecided in PRODUCT.md but was treated as part of the decided visual direction for this register.

## Appendix B — Disagreements between sources

1. **Run-at-caret default (D-011).** algorithms/10 SQL-SPLIT-04 records `inside = SmallestStatement`, `selection = SmartExpand` as its decision; algorithms/20 HV-EXEC-01 specifies `inside = Ask (one-candidate shortcut)`, `selection = Exactly as separate statements`. The selection policy differs as well as the caret policy; both must be aligned when D-011 is decided.
2. **AI write self-approval (D-007).** algorithms/27 U2 recommends owner approval by default on managed connections with org tightening; algorithms/32 A11 and §2-2 require co-approval on production always. access/05 ALG-REQ-01 forbids self-approval unless permitted; mcp/02 allows local approval.
3. **`max_staleness` (D-006).** access/05: 5 min hosted; networking/05: `offline_grace` 15 min; algorithms/32 merges them at 15 min and still lists hosted vs self-hosted defaults as open.
4. **Read-only functions (D-009).** mcp/02 allows VOLATILE functions without egress extensions; algorithms/13 and algorithms/27 deny.
5. **Audit SQL text (D-023).** access/05: full text by default; mcp/02: redacted by default for AI events.
6. **Perde license (D-001).** algorithms/34's header says the AGPL recommendation is "adopted here", while its §2-1 lists the license as an open product decision.
7. **HA (D-034).** algorithms/34 P1 resolves HA as an EE entitlement, but §2-3 lists it as open; networking/05 §3.11 describes HA as a standard self-hosting option; networking/03 suggests offering it free to beat NetBird.
8. **Audit fail-closed (D-010).** algorithms/31 calls it a decision "per edition"; access/06's rule "security is never paywalled" argues against edition differences.
9. **Turkish search (D-074).** algorithms/26 assumes accent-insensitive UI search **on** by default for everyone; algorithms/25 P11 recommends Turkish-tolerant find **only** for Turkish UI/regional settings and says it aligns with algorithms/26.
10. **Cross-reference error.** algorithms/01 HV-SEC-02 points to "§9-1" for the TLS decision; the list is in §7.
