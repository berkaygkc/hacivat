# Algorithm catalog — index

Implementation-grade specs for every algorithm in Hacivat, Karagöz and Perde. Each spec follows the ALG template in [`../README.md`](../README.md): it has a stable ID, inputs/outputs, steps, edge cases, evidence tags and a verification status. Every catalog file ends with a list of unresolved product decisions. Those decisions are consolidated in [`../../decisions/open-product-decisions.md`](../../decisions/open-product-decisions.md).

Total: **341 specs** in 21 files.

## Hacivat: database access (01–04)

| File | Spec ID prefixes | Specs |
|---|---|---:|
| [01 — Connectivity and sessions](01-connectivity-and-sessions.md) | HV-DRV, HV-CONN, HV-SEC, HV-SESS | 26 |
| [02 — Introspection and metadata model](02-introspection-and-metadata.md) | HV-INTRO, HV-MODEL | 20 |
| [03 — DDL generation and schema compare](03-ddl-and-schema-compare.md) | HV-DDL | 19 |
| [04 — NoSQL: Redis and MongoDB](04-nosql-redis-mongo.md) | HV-NOSQL | 20 |

## Hacivat: SQL language engine (10–13)

File 10 §0.2 holds the shared spec index, priorities, dependency map and crate layout for files 10–13.

| File | Spec ID prefixes | Specs |
|---|---|---:|
| [10 — Lexing, splitting, parameters](10-sql-lexing-splitting-params.md) | SQL-LEX, SQL-SPLIT, SQL-PARAM | 14 |
| [11 — Parsing and semantics](11-sql-parsing-and-semantics.md) | SQL-PARSE, SQL-SEM | 8 |
| [12 — Completion and name resolution](12-sql-completion-and-resolution.md) | SQL-RES, SQL-COMPL | 11 |
| [13 — Safety, lint, format](13-sql-safety-lint-format.md) | SQL-SAFE, SQL-LINT, SQL-FMT | 12 |

## Hacivat: execution, results, grid, editor (20–25)

| File | Spec ID prefixes | Specs |
|---|---|---:|
| [20 — Query execution](20-query-execution.md) | HV-EXEC | 16 |
| [21 — Result fetching and result store](21-result-fetching-and-store.md) | HV-RES | 14 |
| [22 — Data grid component](22-data-grid-component.md) | HV-GRID | 16 |
| [23 — Data editing](23-data-editing.md) | HV-EDIT | 11 |
| [24 — Clipboard, import, export](24-clipboard-import-export.md) | HV-XFER | 13 |
| [25 — SQL editor component](25-sql-editor-component.md) | UI-EDIT | 24 |

## Cross-cutting (26–27)

| File | Spec ID prefixes | Specs |
|---|---|---:|
| [26 — Internationalization](26-i18n.md) | I18N | 24 |
| [27 — Local MCP server](27-mcp-server.md) | MCP | 23 |

## Karagöz agent (30–31)

| File | Spec ID prefixes | Specs |
|---|---|---:|
| [30 — Enrollment, discovery, credentials](30-karagoz-enrollment-discovery-credentials.md) | KG-ENROLL, KG-DISC, KG-CRED | 22 |
| [31 — Execution and audit](31-karagoz-execution-and-audit.md) | KG-EXEC, KG-AUDIT | 17 |

## Perde control plane (32–34)

| File | Spec ID prefixes | Specs |
|---|---|---:|
| [32 — Authorization](32-perde-authorization.md) | PD-AUTHZ | 9 |
| [33 — Networking](33-perde-networking.md) | PD-NET | 13 |
| [34 — Control plane](34-perde-control-plane.md) | PD-CP | 9 |

## Related

- Architecture decisions: [`../../decisions/`](../../decisions/) (ADR 0001: native UI on GPUI)
- Verification of DB behavior and external facts: [`../verification/`](../verification/)
- DataGrip teardown: [`../datagrip/`](../datagrip/)
