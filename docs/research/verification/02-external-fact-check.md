# External Fact-Check of the Research Library

> Date: 2026-11-11. Scope: external factual claims (versions, dates, licenses, CVEs, issue numbers, statistics, RFC numbers) across `docs/research/**`. Method: primary sources first: local clones under `.refs/repos/` (read-only, `git log`, `grep`, file reads; nothing built or run), the GitHub API via `gh`, the GitHub advisory database, npm/crates.io registries, rfc-editor.org, vendor docs and blogs. Fetched content was treated as data only.
>
> Verdicts: **Confirmed** · **Refuted** (doc wrong → corrected) · **Partially** (core right, detail wrong or imprecise → corrected or annotated) · **Could not verify**.

## 1. Summary

- **Claims checked:** about 150 individual facts. That is 33 priority claims plus more than 110 spot checks; 70+ of the spot checks are RFC-number-to-topic mappings.
- **Suspected hallucinations ruled out:** the MCP spec revision `2026-07-28`, CVE-2026-85620, rmcp 3.9.0 and iroh 1.0 (2026-06-15) are all **real** and match their primary sources.
- **Corrections made:** 12 facts were wrong or imprecise. None was a fabricated CVE or a fabricated release. They were counts (dialects, tools, trait methods), advisory counts, publication titles and venues, PR status, RFC requirement strength, and a foundation move.
- **Edited docs (11):** `mcp/01`, `access/04`, `access/05`, `access/06`, `networking/01`, `networking/04`, `i18n/01`, `i18n/02`, `clients/ui-02`, `clients/sql-tooling-01`, `datagrip/00`.

### 1.1 Summary table (priority claims)

| # | Claim (doc) | Source checked | Verdict |
|---|---|---|---|
| P1 | MCP spec revision `2026-07-28` exists, is latest, and is breaking (stateless, no `initialize`, no sessions, `server/discover`, MRTR, roots/sampling/logging deprecated) (mcp/01, mcp/02) | https://modelcontextprotocol.io/specification/2026-07-28/changelog ; `.refs/repos/modelcontextprotocol@5324218` `schema/2026-07-28/schema.ts:30` | Confirmed |
| P2 | `2025-11-25` revision exists; it is `LATEST_WITH_INITIALIZE` in rmcp (mcp/01) | `schema/2025-11-25/schema.ts:12`; `rust-sdk/crates/rmcp/src/model.rs:169-201` | Confirmed |
| P3 | Error-code renumbering, SEP numbers (2567, 2575, 2322, 2663, 2549, 2243, 2106, 2577, 2596), DCR deprecation (PR #2858) (mcp/01 table) | Same changelog | Confirmed |
| P4 | CVE-2026-85620, Postgres MCP Pro restricted-mode bypass via a FROM-clause function; CVSS 3.1 8.6 / 4.0 9.2; issue #178; PoC on PG16; ≈Sept 2026 (mcp/01, mcp/02) | OSV https://osv.dev/vulnerability/CVE-2026-85620 (pub. 2026-09-04, CNA VulnCheck, ≤0.3.0, no fix); GitHub advisory `GHSA-cm87-gp55-7cgj` (8.6/9.2); `gh issue view 178 -R crystaldba/postgres-mcp` (opened 2026-06-06, open, PG16 PoC); VulnCheck advisory URL | Confirmed (added GHSA ID and dates to the doc) |
| P5 | Postgres MCP Pro walker descends one level into list/tuple attributes (`safe_sql.py:930-944`) | `.refs/repos/postgres-mcp/src/postgres_mcp/sql/safe_sql.py:925-946` | Confirmed (the root-cause mapping stays tagged Inferred in the doc, which is correct) |
| P6 | Anthropic's archived Postgres server: `BEGIN TRANSACTION READ ONLY` followed by a raw `client.query(sql)` (index.ts:117-118); `COMMIT; DROP SCHEMA public CASCADE;` bypass; Datadog 2025-08-21; vulnerable since v0.1.0 (2024-11-19); v0.6.2 unpatched; ~20k weekly npm downloads; Zed fork 0.1.4 fixed it; `SET` state persists in the pool | `.refs/repos/servers-archived/src/postgres/index.ts:115-118`; https://securitylabs.datadoghq.com/articles/mcp-vulnerability-case-study-SQL-injection-in-the-postgresql-mcp-server/ (Santiago Mola, 2025-08-21) | Confirmed (Datadog itself says 20k in one place and 21k in another) |
| P7 | rmcp crate version 3.9.0, MSRV 1.88, edition 2024, Tier 1, conformance 30/30 and 39/39 | `.refs/repos/rust-sdk/Cargo.toml:12-14`, `ROADMAP.md:5-28`; crates.io `max_stable_version` = 3.9.0 (2026-11-05); GitHub release `rmcp-v3.9.0` 2026-11-05 | Confirmed |
| P8 | rmcp v3.0.0 shipped 2026-07-28 alongside the spec | GitHub release `rmcp-v3.0.0` 2026-07-28T22:52Z; crates.io 3.0.0 created 2026-07-28 | Confirmed |
| P9 | rmcp HTTP server: `allowed_hosts` defaults to loopback; `allowed_origins` empty means Origin validation disabled unless `enforce_origin_validation` is called | `rust-sdk/crates/rmcp/src/transport/streamable_http_server/tower.rs:242-271` ("Defaults to an empty list, which disables Origin validation for backward compatibility") | Confirmed |
| P10 | Survey licenses: servers-archived MIT (© 2024 Anthropic), postgres-mcp MIT (© 2025 Crystal Corp.), dbhub MIT (© 2025 Bytebase), genai-toolbox Apache-2.0, supabase-mcp Apache-2.0, motherduck MIT (© 2024), mcp-redis MIT (© 2025 Redis), mongodb-mcp-server Apache-2.0, mcp-clickhouse Apache-2.0 | `LICENSE` in each `.refs/repos/*` clone; `gh api repos/crystaldba/postgres-mcp/license` → MIT | Confirmed (all 9) |
| P11 | Survey commits/versions: postgres-mcp 0.3.0 @15c8e33 (2026-08-15), dbhub 1.6.1, genai-toolbox 1.16.0, motherduck 1.1.0, mcp-redis 0.5.1, mongodb-mcp-server 3.3.0, mcp-clickhouse 0.7.0; HEAD dates | `git log -1`, `package.json`, `pyproject.toml`, `cmd/version.txt` | Confirmed (all) |
| P12 | Tool lists: archived Postgres server (`query`), archived SQLite server (6 tools), Postgres MCP Pro (9 tools at the cited lines), DBHub, MotherDuck (5), ClickHouse, MongoDB (per-category list), mcp-redis "~53" | `server.py`, `src/tools/*`, `packages/tools-mongodb/src/tools/**`; `grep -c @mcp.tool` = 53 for mcp-redis | Confirmed |
| P13 | genai-toolbox prebuilt Postgres config exposes "~25" tools | `internal/prebuiltconfigs/tools/postgres.yaml`: 29 `kind: tool` + 5 toolsets; file is 311 lines | **Partially → corrected** to 29 |
| P14 | DBHub "three GHSAs" on DNS rebinding | `gh api repos/bytebase/dbhub/security-advisories`: only `GHSA-fm8p-53ww-hf6w` is published (2026-06-24). `GHSA-fp99-xwp4-hv8q` and `GHSA-qvg2-3c48-77mx` return 404 and appear only in code comments (`src/server.ts:236`, `cross-origin.ts:19`) | **Partially → corrected**. Also added the two published DBHub read-only bypass advisories (`GHSA-j656-3hf2-fvjc`, `GHSA-mwwr-p57h-56pf`) |
| P15 | iroh 1.0 released 2026-06-15; wire protocol and API declared stable; >200 M endpoints in 30 days; "95% of data direct" | https://iroh.computer/blog/v1 (2026-06-15); `.refs/repos/iroh/CHANGELOG.md:225`; GitHub release v1.0.0 2026-06-15T12:55Z | Confirmed |
| P16 | iroh release history (rc.0 2026-05-07 … 1.4.1 2026-10-21; main at 1.5.0); features `unstable-net-report`/`unstable-custom-transports`; constants HEARTBEAT 5 s / PATH idle 15 s / relay idle 30 s / 8 paths; license MIT OR Apache-2.0 | `CHANGELOG.md` headings; `iroh/Cargo.toml:3,163-166`; `iroh/src/socket.rs:105-137`; `LICENSE-MIT`, `LICENSE-APACHE`; crates.io iroh 1.4.1 | Confirmed |
| P17 | NetBird server parts relicensed to AGPL in v0.53.0, dated 2025-08-04 (access/06) | https://netbird.io/knowledge-hub/netbird-agpl-announcement (post dated 2025-08-04; v0.53.0 "scheduled for 5 August 2025"); GitHub release v0.53.0 published 2025-08-06T02:11Z; `.refs/repos/netbird/LICENSE:1-2`, `LICENSES/REUSE.toml:5` | **Partially → corrected** (2025-08-04 is the announcement date, not the release date) |
| P18 | NetBird license layout: BSD-3 except `management/`, `signal/`, `relay/`, `combined/` (AGPL); REUSE.toml omits `combined/`; AGPL also in `proxy/` and `tools/idp-migrate/`; default setup key is reusable, unlimited, 30 days | `LICENSE`, `REUSE.toml`, per-directory LICENSE files, `management/server/types/setupkey.go:189-191` | Confirmed |
| P19 | AWS NAT Gateway always symmetric; Azure symmetric with no user setting; GCP symmetric unless EIM, which requires static port allocation (networking/01 §3.10) | https://tailscale.com/blog/nat-traversal-improvements-pt-2-cloud-environments (2025-10-21); GCP primary doc https://docs.cloud.google.com/nat/docs/ports-and-addresses ("to configure EIM, you must use static port allocation"; EIM excludes dynamic port allocation); Azure Learn NAT gateway resource page (UDP idle timeout 4 min, not configurable; SNAT port reuse rules) | Confirmed. AWS and Azure mapping behaviour rests on the Tailscale post; neither AWS nor Azure documents "EIM" explicitly |
| P20 | Tailscale: "well north of 90%" direct (2025); "over 90% of the time" estimate (2020); two hard NATs "almost always" relay; FreeBSD PF EIM patch; Sophos port preservation; PAN-OS 11.1 Persistent NAT | https://tailscale.com/blog/nat-traversal-improvements-pt-1 (2025-10-15); https://tailscale.com/blog/how-nat-traversal-works | Confirmed |
| P21 | RFC numbers cited across networking/access/mcp/i18n (≈75 distinct numbers) | Topic-to-number mapping checked for every cited RFC; RFC 10069 (Multipath QUIC, Proposed Standard) checked on rfc-editor.org | Confirmed (no mis-numbered RFC found) |
| P22 | Teleport: source AGPL-3.0 since 2023-12-01; `/api` Apache-2.0; CE binaries v16+ under a Community license for < 100 employees and < US$10M revenue, with a Critical Security Exception | `.refs/repos/teleport@1283425` `LICENSE`, `api/LICENSE`, `build.assets/LICENSE-community:4-19,62-63`, `README.md:395-409`; https://goteleport.com/blog/teleport-oss-switches-to-agpl-v3/ (2023-12-01); discussion #39158 (2024-03-08; "less than $10MM in annual recurring revenue"); v15.0.0 release 2024-02-01, v16.0.0 2024-06-13 | Confirmed |
| P23 | Bytebase license: MIT Expat, except dirs containing "enterprise" and "any code controlling feature, permission, role and plan enablement" (LICENSE.enterprise); Free plan 10 instances / 20 users; Pro $20/user/month | `.refs/repos/bytebase/LICENSE:1-6`, `backend/enterprise/plan.yaml:3-4`, `pricing/plan.go:49`; https://www.bytebase.com/pricing | Confirmed |
| P24 | Infisical: MIT outside `ee/`, `ee/LICENSE` inside; OpenBao MPL-2.0 | `.refs/repos/infisical/LICENSE:1-8`, `backend/src/ee/LICENSE.md`; `.refs/repos/openbao/LICENSE` | Confirmed |
| P25 | OpenBao "(LF Edge)" (access/06) | https://openbao.org/blog/openbao-joins-the-openssf/ (moved to OpenSSF; announced 2025-06-17) | **Partially → corrected** |
| P26 | Docker Engine API v1.57 at the studied moby commit | `.refs/repos/moby@36d6bbb` `api/swagger.yaml:24-27` (`basePath: "/v1.57"`, `version: "1.57"`) | Confirmed |
| P27 | Bitnami moved images to `bitnamilegacy/*` plus a commercial catalogue in 2025 (was [Inferred; verify]) | https://community.broadcom.com/blogs/beltran-rueda-borrego/2025/08/18/how-to-prepare-for-the-bitnami-changes-coming-soon (archiving to Legacy from 2025-08-28; public catalogue deletion postponed to 2025-09-29; Bitnami Secure Images; dev-only free tier) | Confirmed → tag upgraded to Observed, with dates added |
| P28 | Crowdin free OSS license excludes applicants with "commercial products related to the open-source project" | https://crowdin.com/product/for-open-source (criterion 4, verbatim) | Confirmed |
| P29 | Weblate Libre hosting is free for public libre projects with 160k-string-tier limits; Fluent supported since 4.8 (monolingual, plurals "No"); server GPL-3.0 | https://weblate.org/en/hosting/ ; https://docs.weblate.org/en/latest/formats/fluent.html ; `gh api repos/weblateorg/weblate/license` | Confirmed |
| P30 | DBeaver #40901: opened 2026-04-24, open; Turkish JVM cache misses and double LIMIT; CE 26.0.4; PRs #40885 and #42781 | `gh issue view 40901 -R dbeaver/dbeaver` (body matches, CE 26.0.4); `gh pr view`: #40885 **closed unmerged** (2026-04-23); #42781 open (2026-11-08) | **Partially → corrected** (the PR status was ambiguous and implied #40885 was a fix) |
| P31 | fluent-rs `NUMBER()` only merges options; `FluentNumber::as_string` uses `to_string` plus min-fraction padding (locale-insensitive) | `.refs/repos/fluent-rs/fluent-bundle/src/builtins.rs:1-13`; `src/types/number.rs:148-160` | Confirmed |
| P32 | Unicode MessageFormat 2.0 stable in CLDR/LDML 47 (March 2025); `Intl.MessageFormat` is TC39 Stage 1; ICU4X has no MF2 formatter; ICU4X 2.4.0 on CLDR 49; relative time / display names / units are in `icu_experimental` | https://blog.unicode.org/2025/03/unicode-cldr-47-release-messageformat-2.html (CLDR 47 released 2025-03-13); https://github.com/tc39/proposal-intl-messageformat ("Stage: 1"); `.refs/repos/icu4x` `Cargo.toml:111`, `CHANGELOG.md:8`, `components/experimental/src/*` | Confirmed |
| P33 | Client licenses: RedisInsight SSPL v1 (3.8.0), Compass SSPL v1, mongodb-schema Apache-2.0 (12.7.0), Beekeeper GPL-3.0 + `src-commercial` commercial + ui-kit AGPL/commercial, Glide Data Grid MIT, AG Grid Community MIT, Handsontable "SEE LICENSE IN LICENSE.txt", Monaco MIT; Zed `rope`/`text`/`editor` GPL-3.0-or-later, `gpui`/`sum_tree` Apache-2.0; gpui-component Apache-2.0 | Repo `LICENSE` files; `zed/crates/*/Cargo.toml`; npm registry `latest` metadata | Confirmed |
| P34 | AG Grid Community limits: range selection and clipboard are Enterprise (was [Inferred]) | https://www.ag-grid.com/javascript-data-grid/community-vs-enterprise/ | Confirmed → tag upgraded (wording: "enhanced" copy/paste is Enterprise) |
| P35 | pgls typecheck calls `conn.prepare(sql)`, not EXPLAIN, even though ARCHITECTURE.md says EXPLAIN | `.refs/repos/postgres-language-server/crates/pgls_typecheck/src/lib.rs:62-85`; `ARCHITECTURE.md:98`; `docs/features/type_checking.md:65-67` | Confirmed. Nuance: sqlx `prepare` is a protocol-level Parse, not SQL `PREPARE`; the repo docs now call the EXPLAIN path "legacy" |
| P36 | libpg_query at PG 18.6; pg_query.rs 18.1.0; libpg_query BSD-3 | `.refs/repos/libpg_query/Makefile:10-12` (`PG_VERSION = 18.6`); `pg_query.rs/Cargo.toml` | Confirmed |
| P37 | sqlparser-rs has "17 dialects" and "~246" Dialect hook methods | `src/dialect/*`: 16 `impl Dialect for` (15 named + Generic); `dialect_from_str` maps 16; the trait body (`mod.rs:236-2351`) has 220 `fn` items | **Refuted → corrected** (16; ~220) |
| P38 | sqruff "26 dialects" followed by a list | `crates/lib-dialects/src`: 27 dialect modules; the doc's own list has 27 entries | **Partially → corrected** (27) |
| P39 | JetBrains EULA prohibits reverse engineering | https://www.jetbrains.com/legal/docs/toolbox/user/ (User Agreement v2.0, effective 2025-04-16, §3.5(b): "reverse-engineer, disassemble, or decompile the Product… try to derive the source code", "unless this right is granted by applicable law") | Confirmed → clause and URL added to datagrip/00 |
| P40 | YouTrack issue numbers in datagrip/* | `grep` over `datagrip/`: no YouTrack IDs are cited | N/A (nothing to verify) |
| P41 | DataGrip 2024.3.5, build 243.24978.79 | `/Applications/DataGrip.app/Contents/Resources/product-info.json` | Confirmed |

### 1.2 Summary table (additional spot checks)

| # | Claim | Source | Verdict |
|---|---|---|---|
| S1 | CVE-2025-49596 MCP Inspector, CVSS 9.4 | GitHub advisory `GHSA-7f8r-222p-6f5g` (pub. 2025-06-13, CVSS 4.0 9.4) | Confirmed |
| S2 | CVE-2025-6514 `mcp-remote`, CVSS 9.6, 2025-07 | `GHSA-6xpm-ggf7-wc3p` (pub. 2025-07-09, CVSS 3.1 9.6) | Confirmed |
| S3 | Trend Micro SQLite MCP article (2025-06), >5,000 forks, Anthropic declined to patch | trendmicro.com article (2025-06-24) | Confirmed (exact date added) |
| S4 | Simon Willison, "Supabase MCP lethal trifecta", 2025-07-06 | simonwillison.net/2025/Jul/6/supabase-mcp-lethal-trifecta/ — actual title "Supabase MCP can leak your entire SQL database" | **Partially → corrected** (title) |
| S5 | Supabase "Defense in depth for MCP servers", 2025-09-16 | supabase.com/blog/defense-in-depth-mcp | Confirmed |
| S6 | Invariant Labs GitHub MCP exploit, 2025-05-26 | invariantlabs.ai/blog/mcp-github-vulnerability | Confirmed |
| S7 | Trautwein et al. DCUtR: 4.4 M attempts, 85 k networks, 167 countries, 70% ± 7.1%, 97.6% first-try; "arXiv:2510.27500 (IMC '26)" | https://arxiv.org/abs/2510.27500 (2025-10-31). The IMC '26 paper is a retitled version, arXiv:2604.12484 (probelab.io publication page) | **Partially → corrected** (venue attribution) |
| S8 | ProbeLab "Can libp2p punch through NATs?" (2026-08-03): 43,684 peers, PRC 82.9% (3,669), symmetric 39.7% (536), 3.9% symmetric, 8.3% reachable, ~36% no reservation | probelab.io blog | Confirmed |
| S9 | Ford/Srisuresh/Kegel 2005: 82% UDP / 64% TCP | arXiv cs/0603074 abstract | Confirmed |
| S10 | RFC 4787 REQ-1 EIM MUST, REQ-2 paired, REQ-5 ≥2 min, REQ-9 hairpinning, REQ-11 determinism | rfc-editor.org RFC 4787 text | Confirmed |
| S11 | RFC 6888 REQ-2/4/5/7/8 (120 s)/11/15 | RFC 6888 text | Confirmed |
| S12 | RFC 6888 "PCP MUST (REQ-9)" | RFC 6888 REQ-9: MUST implement *a* mapping-control protocol, which SHOULD be PCP | **Partially → corrected** |
| S13 | RFC 9308 §2: 3% (Trammell) to 5% (Swett) of networks block UDP | RFC 9308 text, §2 "The Necessity of Fallback" | Confirmed |
| S14 | RFC 7675 consent: 5 s ± 20%, 30 s expiry | RFC 7675 text | Confirmed |
| S15 | RFC 6962 §2.1 leaf prefix 0x00 / node prefix 0x01 (was [Inferred]) | RFC 6962 text, lines 233-239 | Confirmed → tag upgraded |
| S16 | RFC 10069 = Multipath QUIC, Proposed Standard, 2026 | rfc-editor.org/info/rfc10069 ("Managing Multiple Paths for a QUIC Connection") | Confirmed (month not shown on the info page) |
| S17 | Birthday-paradox table (174→50%, 256→64%, 1024→98%, 2048→99.9%; both-hard 54k/170k) | Tailscale "How NAT traversal works"; recomputed: 1−(1−256/65535)^k | Confirmed |
| S18 | Tailscale "about 33%" IPv6 (2020) | same blog (says "about 33%", later "34%") | Confirmed (source is internally inconsistent) |
| S19 | Tailscale KB "Peer relays" /kb/1591, last validated 2026-02-04 | tailscale.com/kb/1591/peer-relays | Confirmed |
| S20 | Tailscale has no birthday-paradox code at @7b2c32b | `grep -rli birthday tailscale/wgengine tailscale/net` → none | Confirmed |
| S21 | Headscale BSD-3, 0.30.0 unreleased, minimum client v1.84.0, `hskey-auth-` prefix | `.refs/repos/headscale/LICENSE`, `CHANGELOG.md:1-5`, `hscontrol/db/preauth_keys.go:86` | Confirmed |
| S22 | Tailscale client license BSD-3 | `.refs/repos/tailscale/LICENSE` | Confirmed |
| S23 | iroh 0.96 blog 2026-01-27 vs tag 2026-01-28 | iroh blog; CHANGELOG | Confirmed (the two dates are consistent) |
| S24 | Teleport "Ask HN: How did you replace Teleport?" = HN 41873439 | HN API (2024-10-17) | Confirmed |
| S25 | Teleport discussion #39158 title/date | github.com/gravitational/teleport/discussions/39158 | Confirmed |
| S26 | OCSF Datastore Activity class 6005, activity IDs in 1.3 | schema.ocsf.io API 1.3.0 | Confirmed |
| S27 | DBeaver issues #3568, #34854 (~25k rows), #11931, #17099, #42570 (800 MB), #42803, #42205, #9699, #35261, #39861 (20,000+ views), #34432, #19589, #35197, #3536, #37855, #10047, #42474, #42465, #42468: titles, states, years | `gh issue view` for each | Confirmed (19/19) |
| S28 | DBeaver PR #42805: merged 2026-11-10, +88,069/−3,395, 2,306 files | `gh pr view 42805` | Confirmed |
| S29 | DBeaver commit f78ac45 (2026-11-11), Apache-2.0 | local clone | Confirmed |
| S30 | Client versions: RedisInsight 3.8.0; compass-crud 14.9.0, compass-editor 0.73.1, compass-components 1.71.1; harlequin 2.17.0; rainfrog 0.4.6; gobang 0.1.0-alpha.5 (last commit 2023-01-25); Beekeeper 6.2.2 with `tabulator-tables ^6.7.1` at line 158 | local `package.json`/`pyproject.toml`/`Cargo.toml` | Confirmed |
| S31 | CodeMirror view 6.41.0 (npm 6.43.15), state 6.6.0, autocomplete 6.20.1; TanStack virtual-core 3.19.11; Glide 6.0.4-alpha25 (npm 6.0.3); AG Grid 36.3.0; Handsontable 18.1.2; Monaco 0.57.0 | local clones + npm registry | Confirmed |
| S32 | egui 0.37.0, MIT OR Apache-2.0; floem MIT; lapce Apache-2.0 | local clones | Confirmed |
| S33 | sqlparser-rs 0.64.0; salsa 0.28.5; 193 `Span::empty()` in spans.rs | local clones; crates.io | Confirmed |
| S34 | CLDR 49 Turkish: decimal `,`, group `.`, list `;`, percent `%#,##0`, ∞/NaN | ICU4X test data `cldr-numbers-full/main/tr/numbers.json` | Confirmed |
| S35 | ICU4X `fold_turkic` at `casemapper.rs:240,438`; Unicode License V3 | local clone | Confirmed |
| S36 | `messageformat` v4 is Apache-2.0 (was [Inferred]); `@messageformat/fluent` Apache-2.0 | npm registry (4.1.0; 0.15.0) | Confirmed → tag upgraded |
| S37 | Pontoon BSD-3-Clause (was [Inferred; verify]) | `gh api repos/mozilla/pontoon/license` | Confirmed → tag upgraded |
| S38 | Bytebase only `enterprise` dir is `backend/enterprise/` | local clone | Confirmed |
| S39 | HashiCorp BSL (Aug 2023), Elastic and Redis later added AGPL, Valkey BSD-3 (access/06) | general record; consistent with the cited LICENSE headers | Confirmed (not re-fetched individually) |
| S40 | GCP Cloud NAT EIM incompatible with dynamic port allocation | GCP docs | Confirmed |
| S41 | Azure NAT UDP idle timeout 4 min, not configurable | Azure Learn "NAT Gateway Resource" | Confirmed (not claimed explicitly in the docs; useful context) |
| S42 | MCP spec repo LICENSE: MIT → Apache-2.0 transition, docs CC-BY-4.0 | `.refs/repos/modelcontextprotocol/LICENSE` (same text in rust-sdk) | Confirmed |

## 2. Details of corrections (all edits carry "(corrected after verification)")

1. **mcp/01 §genai-toolbox.** "~25 Postgres tools" became "29 Postgres tools (plus 5 toolsets)". The line citation was widened to `postgres.yaml:15-311`.
2. **mcp/01, DBHub GHSAs (3 places: summary bullet 16, §DBHub HTTP hardening, incident table I9).** Only `GHSA-fm8p-53ww-hf6w` is publicly published (repo advisory 2026-06-24; global DB 2026-09-24, critical). The other two IDs appear only in code comments and return 404 from the advisory API. The two DBHub read-only bypass advisories published the same day were added; they are directly relevant to doc 02's threat model.
3. **mcp/01 sources.** The CVE-2026-85620 entry now carries the GHSA ID, publication date, CNA, affected range and URLs. CVE-2025-49596 and CVE-2025-6514 now carry GHSA IDs, dates and CVSS versions. The Simon Willison post title was corrected and the Trend Micro date set to 2025-06-24.
4. **access/06, NetBird.** "since v0.53.0 (2025-08-04)" now reads: announced 2025-08-04, release scheduled 2025-08-05, GitHub release timestamp 2025-08-06T02:11Z. The "[web search result]" evidence became `[Observed: netbird.io announcement URL]`.
5. **access/06, OpenBao.** "(LF Edge)" now reads "started under LF Edge, moved to the OpenSSF in June 2025" (2 places).
6. **access/04, Bitnami.** [Inferred; verify] became Observed (Broadcom community post), with the dates 2025-08-28 and 2025-09-29 and the Secure Images tiering.
7. **access/05, RFC 6962.** [Inferred: not re-fetched] became Observed, with the formula line.
8. **networking/01 and networking/04, Trautwein et al.** The IMC '26 attribution was moved to the retitled arXiv:2604.12484 version; 2510.27500 is now marked as the preprint (3 places).
9. **networking/01 §3.9, RFC 6888 REQ-9.** "PCP MUST" now reads "a mapping-control protocol MUST, which SHOULD be PCP".
10. **i18n/02, DBeaver #40901.** PR #40885 is marked closed and not merged; PR #42781 is marked open as of 2026-11-11 (2 places).
11. **i18n/01.** `messageformat` v4 Apache-2.0 and Pontoon BSD-3-Clause were upgraded to Observed.
12. **clients/ui-02, AG Grid.** [Inferred: AG Grid feature matrix] became Observed, with the URL (2 places).
13. **clients/sql-tooling-01.** sqlparser-rs went from 17 to 16 dialects, and the trait went from ~246 to ~220 hook methods (2 places each, summary and §detail). The sqruff list went from "26 dialects" to 27, matching its own enumerated list.
14. **datagrip/00.** The JetBrains User Agreement v2.0 §3.5(b) wording and URL were added to the prohibited-actions table.

## 3. Items confirmed but worth a note (no edit)

- **AWS and Azure NAT mapping.** The only explicit "symmetric" statement is Tailscale's (pt. 2). AWS's NAT Gateway docs say nothing about EIM. Azure's docs describe SNAT port reuse rules (the same port can be reused toward different destinations, and different ports are used for the same destination) but do not use EIM/EDM terms. Treat this as a measured third-party observation, not a vendor guarantee. GCP's EIM requirement is vendor-documented.
- **pgls typecheck.** The upstream docs now describe EXPLAIN-based checking as "legacy", running alongside new static `typecheck` rules. The doc's statement that the code calls `conn.prepare` is still accurate.
- **Datadog download count.** The article itself gives both 20k and 21k weekly downloads.
- **Tailscale 2020 IPv6 figure.** The article says both 33% and 34%.
- **CVE-2026-85620 fix status.** OSV lists no fixed version and the issue is still open. The doc's "fix status disputed across trackers" is compatible with this.

## 4. Could not verify

- None of the priority claims. The NVD pages were not machine-readable (they return only the "NVD - Home" shell), so CVE details were taken from OSV and the GitHub advisory database, which mirror the CVE records.
- The publication month of RFC 10069 does not appear on its rfc-editor info page. The doc's "November 2026" comes from datatracker and was left as is.
