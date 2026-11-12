# Hacivat Research Library

Deep research that grounds the design of the Hacivat product family. Every document here is written in English.

## Product family (context for every document)

| Component | What it is | License intent |
|---|---|---|
| **Hacivat** | Open-source desktop database client written in Rust. Multi-database (PostgreSQL, MySQL/MariaDB, SQL Server, SQLite, Redis, MongoDB, ClickHouse, …). SQL editor, data grid, schema explorer. Works fully offline with no login. Ships a native local MCP server so AI agents can use it. | Open source (permissive) |
| **Karagöz** | Open-source agent installed on a server with a single command obtained from Perde. Discovers databases (e.g. Docker containers via labels or image heuristics) and attaches them to the user's Perde account, ideally with no credentials typed by the user. Enforces permissions and records audit events locally. | Open source |
| **Perde** | Control plane: accounts, organizations, users, role-based access to databases discovered by Karagöz agents, audit log, and a **rendezvous / signaling** service that connects Hacivat and Karagöz **peer-to-peer** (NAT traversal / hole punching). A relay is only the fallback when no direct path can be established. Self-hostable; also offered as a paid hosted service. | Open core (feature split TBD) |

Hard requirements:
- Repositories, code, docs and UI source strings are English-first.
- Internationalization from day one: v1 ships English + Turkish; adding a language must be fast and require no code changes.
- Data path between Hacivat and Karagöz is direct whenever possible; Perde must not carry query traffic except as a relay fallback.
- Since traffic can be P2P, authorization and audit must be enforceable at Karagöz, not only at Perde.

## Layout

| Folder | Scope |
|---|---|
| `datagrip/` | Legitimate, non-decompiling teardown of JetBrains DataGrip 2024.3.5 (behavior, architecture, data models, shipped resources, official docs, IntelliJ Community platform sources). |
| `clients/` | Open-source DB clients and language tooling: DBeaver, postgres-language-server, sqlparser-rs, sqruff, pgcli, RedisInsight, MongoDB Compass, Beekeeper Studio, UI toolkits. |
| `networking/` | NAT traversal, hole punching, relays: Tailscale/DERP, Headscale, NetBird, iroh, STUN/TURN/ICE, QUIC. |
| `access/` | Agent-based database access, RBAC, audit, approvals, dynamic credentials, masking: Teleport, Bytebase, CloudBeaver, Vault/Infisical, Docker discovery patterns, database MCP servers. |
| `i18n/` | Localization architecture for Rust desktop + web + server, Turkish-specific pitfalls. |
| `mcp/` | Model Context Protocol: spec, existing database MCP servers, security model for Hacivat's local MCP server. |
| `algorithms/` | Consolidated, implementation-ready algorithm specifications distilled from all research. |

## Conventions

### Evidence tags
- **[Observed: <source>]** — seen directly: file path (with line numbers when possible), URL, doc page, or a reproducible runtime observation.
- **[Inferred]** — reasoning from names, structure or general knowledge. Always say what it is based on.
- **[Unknown]** — explicitly open question.

### Clean-room and licensing rules
- DataGrip is proprietary: no decompilation, no bytecode inspection, no copying of shipped resource text (SQL, scripts) into our code. Describe behavior and structure in our own words.
- GPL/AGPL/BSL/proprietary sources: describe algorithms in prose and our own pseudocode only; never paste code.
- Permissive sources (MIT/Apache/BSD): still prefer pseudocode; note the license and exact file so a future implementer can decide on reuse with attribution.
- Every document lists the license of each studied project.

### Document template
1. Scope and sources (with versions/commit hashes and licenses)
2. Executive summary (≤ 20 bullets)
3. Detailed findings (sectioned, evidence-tagged)
4. Algorithm specifications (see template below)
5. Implications for Hacivat / Karagöz / Perde (concrete design recommendations)
6. i18n considerations
7. Risks, pitfalls, open questions

### Algorithm specification template
```
### ALG-<AREA>-<NN>: <Name>
- Problem:
- Inputs / outputs:
- Algorithm: numbered steps or pseudocode
- Data structures:
- Complexity / performance notes:
- Edge cases (exhaustive list):
- Test cases (input → expected):
- Prior art and evidence:
- Used by: Hacivat | Karagöz | Perde
```
