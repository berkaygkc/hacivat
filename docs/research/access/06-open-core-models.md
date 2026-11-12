# 06 — Open-core models, licenses and feature splits: lessons for Perde

> Research document for the Hacivat product family (Hacivat desktop client, Karagöz server agent, Perde control plane).
> Written 2026-11-11. This is engineering and product research, **not legal advice**; any license decision must be reviewed by counsel before the first public release.

---

## 1. Scope and sources

### 1.1 Question being answered

Perde is described as "open core (feature split TBD)": self-hostable OSS plus a paid hosted service. Karagöz (the agent on production servers) and Hacivat (the desktop client) are open source. This document answers:

1. Which licenses do comparable projects use for their core and for their paid parts, and how is the split implemented technically (separate repo, `ee/` folder, private submodule, build tags, license key)?
2. What is free versus paid in each comparable product, especially for **security features** (SSO, MFA enforcement, audit logs, approvals, masking, device trust)?
3. How did communities react to license changes and to paywalled security ("SSO tax")?
4. What should Perde do: license choice for each component, which features to gate, how to gate them in code, and which features must never be gated?

### 1.2 Projects compared and evidence sources

| Project | Evidence | License as observed (2026-11-11) |
|---|---|---|
| Teleport (gravitational/teleport) | Sparse clone `.refs/repos/teleport` at commit `1283425b60ec5f60d509ba4c791183d452923ff7` (2026-08-25): `LICENSE`, `docs/pages/feature-matrix.mdx`, `docs/pages/faq.mdx`, `docs/pages/includes/ent-vs-community-faq.mdx`, `lib/modules/modules.go`, submodule entry `e` (`git ls-tree HEAD e` → commit `42c822cecac2ca29701d159b17f70d89553e123f`); `entitlements/entitlements.go` via GitHub API; web search on the v16 Community license | Source: **AGPL-3.0** (`LICENSE`). Enterprise code: private `e/` submodule. Community Edition *binaries*: "Teleport Community License" (commercial restrictions; < 100 employees and < US$10M revenue) since v16. |
| Bytebase (bytebase/bytebase) | Clone `.refs/repos/bytebase` at `d651388204137f01ffb5ffc0c559dc885ca5f8b7`; see document 02 | **MIT** except dirs named `*enterprise*` and "any code controlling feature, permission, role and plan enablement" → **Bytebase Enterprise License** (source-available). |
| NetBird (netbirdio/netbird) | Clone `.refs/repos/netbird` at `eb2aa2847097c4a241da77f86577a87122a14792`: `LICENSE`, `LICENSES/REUSE.toml`, `CONTRIBUTOR_LICENSE_AGREEMENT.md`; web: NetBird AGPL announcement, docs.netbird.io/selfhosted/enterprise | **BSD-3-Clause** for clients and most code; **AGPL-3.0-only** for `management/`, `signal/`, `relay/` (and per the `LICENSE` header text also `combined/`) since v0.53.0 (announced 2025-08-04; release scheduled 2025-08-05, GitHub release timestamp 2025-08-06T02:11Z; corrected after verification). Commercial license for self-hosted enterprise features. CLA required. |
| Tailscale (tailscale/tailscale) | Clone `.refs/repos/tailscale` at `7b2c32b9138b0ab2321910c7eaa8e81b24919dce`: `LICENSE`, `licenses/README.md`; web: tailscale.com/pricing | Client/daemon **BSD-3-Clause**; coordination server **proprietary** (not published); GUI clients for Windows/macOS/iOS closed (per Headscale README). |
| Headscale (juanfont/headscale) | Clone `.refs/repos/headscale` at `443f3a0ecd76e9697eb01bd09a616340963e0b79`: `LICENSE`, `README.md` | **BSD-3-Clause**; independent reimplementation of the Tailscale control server, single-tailnet scope; one maintainer employed by Tailscale. |
| Infisical (Infisical/infisical) | GitHub API `repos/Infisical/infisical/contents/LICENSE`; web: infisical.com/pricing | **MIT** except `ee/` directories → `ee/LICENSE` (proprietary). |
| GitLab (gitlabhq/gitlabhq) | GitHub API `LICENSE`; web: about.gitlab.com/pricing/feature-comparison (tier markers did not render) | **MIT** for CE; `ee/` → `ee/LICENSE` (proprietary); `jh/` (JiHu China) separate; docs CC BY-SA 4.0; all client-side JS MIT. |
| Grafana (grafana/grafana) | GitHub API license (`AGPL-3.0`); web: Grafana Enterprise docs | **AGPL-3.0** (OSS); Grafana Enterprise is a separate proprietary build. |
| Sentry (getsentry/sentry) | GitHub API `LICENSE.md` | **FSL-1.1-Apache-2.0** (Functional Source License, converts to Apache-2.0 two years after each release). |
| PostHog (PostHog/posthog) | GitHub API `LICENSE` | **MIT** except `ee/` → `ee/LICENSE` (proprietary). |
| Supabase (supabase/supabase) | GitHub API license | **Apache-2.0** (monorepo); hosting is the business. Web: supabase.com/pricing. |
| Context: HashiCorp Vault/Terraform, OpenBao, OpenTofu, Elasticsearch, Redis, Valkey | GitHub API license endpoints and `LICENSE` headers | Vault/Terraform **BSL 1.1** (licensor now IBM); OpenBao/OpenTofu **MPL-2.0**; Elasticsearch **AGPL-3.0 / SSPL-1.0 / ELv2** triple license (x-pack ELv2-only); Redis 8+ **RSALv2 / SSPL / AGPL-3.0** tri-license (≤7.2 BSD-3); Valkey **BSD-3-Clause**. |
| sso.tax | Fetched 2026-11-11 | — |

Clean-room note: this document reads license texts and public feature tables only; no proprietary code is reproduced.

---

## 2. Executive summary

1. **Four technical patterns dominate:** (a) *private enterprise submodule/repo* (Teleport `e/`, Grafana Enterprise); (b) *`ee/` folder in the public repo under a source-available license* (GitLab, PostHog, Infisical, Bytebase's `enterprise/`); (c) *whole product under a delayed-open "fair source" license* (Sentry FSL, HashiCorp BSL); (d) *fully open, monetise hosting* (Supabase, Headscale-as-community).
2. **Copyleft for the server is now the mainstream defensive choice.** Grafana (2021), Teleport (v15), NetBird (2025, server dirs only), Elastic (added AGPL 2024) and Redis (added AGPL in v8, 2025) all converged on **AGPLv3 for the server side** while keeping clients/agents permissive. [Observed: license files; Inferred: dates from general knowledge where not fetched]
3. **Agents and clients stay permissive** in every successful case: NetBird client BSD-3, Tailscale client BSD-3, Teleport's `tsh`/Connect are in the AGPL repo but distributed freely, Grafana Alloy/agents Apache. Security teams install agents on production hosts; copyleft or source-available agents slow adoption. [Observed: NetBird `LICENSE` header; Tailscale `LICENSE`]
4. **Source-available relicensing triggers forks when there is a large external contributor base and a cloud competitor:** HashiCorp BSL → OpenTofu (Linux Foundation) and OpenBao (started under LF Edge; moved to the OpenSSF in June 2025 — corrected after verification [Observed: https://openbao.org/blog/openbao-joins-the-openssf/]); Elastic SSPL → OpenSearch (AWS); Redis RSAL/SSPL → Valkey (Linux Foundation). Two of the three originators later added an OSI license back (Elastic AGPL, Redis AGPL). [Observed: OpenBao/OpenTofu MPL-2.0, Valkey BSD-3, Elastic/Redis LICENSE headers]
5. **Teleport v16 is the cautionary tale most relevant to Perde:** source stays AGPL, but official CE binaries carry a commercial license restricting companies with ≥100 employees or ≥$10M revenue; CE also gets only GitHub SSO, CLI-only role requests, no device trust, no session moderation, no dual authorization. Community reaction included "bait-and-switch" reviews and "Ask HN: How did you replace Teleport?". [Observed: `feature-matrix.mdx`; web search results]
6. **The "SSO tax" is a recognised reputational liability** (sso.tax catalogues vendors with 50–200%+ uplifts for SSO). Bytebase is listed; Teleport CE is the canonical example of SSO gating (GitHub only). [Observed: sso.tax; Teleport feature matrix]
7. **Counter-examples that free core security features and still monetise:** Teleport CE includes RBAC, per-session MFA, session recording, structured audit **and SIEM export**; Tailscale includes configuration audit logs and ACLs on the free plan; Supabase self-host gates nothing. [Observed: `feature-matrix.mdx`; tailscale.com/pricing; supabase.com/pricing]
8. **What vendors commonly gate (and customers accept):** SCIM/directory sync, HA/active-active, advanced approval workflows and access reviews, long audit retention and *streaming*, compliance (FedRAMP/FIPS), hardware-key and HSM integrations, custom branding, premium integrations, support SLAs. These are "scale, governance, compliance, convenience". [Observed: across feature tables]
9. **What draws backlash when gated:** SSO/SAML/OIDC login, MFA enforcement, basic audit log visibility, data masking that *disappears* on downgrade, and retroactive license tightening. [Observed: sso.tax; Teleport reviews] [Inferred: synthesis]
10. **Bytebase's gate set is the most aggressive** of the access-governance tools studied: masking, approval workflow, custom roles, 2FA enforcement, OIDC/SAML/LDAP and full audit are Enterprise-only; FREE cannot read audit logs at all. (Document 02.)
11. **NetBird's 2025 split is the closest analogue to Perde** (P2P data plane + management/signal/relay control plane): clients BSD-3, control plane AGPL, commercial license for HA, SCIM, EDR/MDM, traffic-flow logging, peer approval, audit streaming. The relicense was possible because NetBird required a CLA from day one. [Observed: NetBird `LICENSE`, `REUSE.toml`, CLA; docs.netbird.io]
12. **Headscale shows that a popular closed control plane invites a community reimplementation**; Tailscale chose to embrace it (employs a maintainer) rather than fight it. Perde being open from the start avoids that dynamic. [Observed: Headscale README]
13. **Implementation lessons:** single source of truth for the plan matrix shared by server and UI (Bytebase `plan.yaml`); offline-verifiable signed license (Bytebase RS256 JWT bound to workspace id; NetBird `NB_LICENSE_KEY` validated against a license service); entitlements enumerated as named capabilities (Teleport `entitlements.go`, ~31 kinds). [Observed]
14. **Recommendation — licenses:** Hacivat **Apache-2.0** (or MIT/Apache-2.0 dual, Rust convention); Karagöz **Apache-2.0**; Perde core **AGPL-3.0-only** with a separate `ee/` tree under a source-available commercial license; CLA (or copyright assignment-lite) for Perde only; protocol/SDK crates Apache-2.0; trademarks registered for "Hacivat", "Karagöz", "Perde".
15. **Recommendation — never gate:** anything Karagöz enforces (authorization, read-only, masking, approvals verification, local audit), SSO login via OIDC *and* SAML for self-hosted, MFA enforcement, audit search/export with operator-chosen retention, tamper-evidence verification, security fixes for supported versions.
16. **Recommendation — gate (paid):** SCIM/directory sync, HA/multi-region Perde, advanced multi-stage approval workflows + ChatOps integrations + access reviews/certifications, managed long-term audit archive and SIEM streaming connectors beyond the generic one, PII auto-classification, compliance reporting packs, custom branding, fleet-scale agent management (staged rollouts), managed relays, SLA support.
17. **Recommendation — failure semantics:** an expired or missing license must **never weaken enforcement**; it only disables *configuration* of paid management features, after a grace period, without deleting data (Bytebase's "filter, don't delete" audit retention is the right instinct; its "masking stops on FREE" is the wrong one).
18. **Hosted pricing:** per active user with a generous free tier (e.g. ≤5 users, ≤3 agents) and no per-agent tax for small teams; publish prices (Bytebase's public $20/user Pro tier and Tailscale's published tiers are well received; "contact us" for SSO is what sso.tax shames). [Inferred: synthesis]

---

## 3. Detailed findings

### 3.1 License taxonomy (what each license actually does)

| License | OSI-approved | Core obligation / restriction | Effect against "cloud vendor hosts your product" | Effect on enterprise adoption | Examples |
|---|---|---|---|---|---|
| MIT / BSD-3 / Apache-2.0 | Yes | Attribution (Apache adds patent grant + NOTICE) | None | Best | Tailscale client, Headscale, Supabase, NetBird client, Valkey |
| MPL-2.0 | Yes | File-level copyleft | Weak | Good | OpenTofu, OpenBao (and HashiCorp pre-2023) |
| AGPL-3.0 | Yes | Network-use copyleft (§13): modified versions offered over a network must offer source | Strong in practice: hosted competitors must publish their changes; many large companies forbid AGPL internally (driving commercial licenses) | Medium: some corporate policies ban AGPL, but *unmodified internal use* is generally fine | Grafana, Teleport source, NetBird server, Elasticsearch (option), Redis 8 (option) |
| SSPL-1.0 | No (OSI rejected) | Offering as a service requires releasing the *entire* service stack | Very strong | Poor; distros drop it | MongoDB, Elasticsearch (option), Redis (option) |
| BSL 1.1 (Business Source) | No | Production use allowed per "Additional Use Grant"; converts to an OSI license after Change Date (≤4 years) | Strong (explicit non-compete) | Medium-poor; legal review needed for each grant wording | HashiCorp (Vault/Terraform, IBM licensor), Sentry 2019–2023 |
| FSL-1.1 (Functional Source) | No ("fair source") | Any use except "Competing Use"; converts to Apache-2.0 or MIT after **2 years** | Strong for 2 years | Medium | Sentry (since 2023) |
| ELv2 (Elastic License 2.0) | No | Cannot provide as managed service; cannot circumvent license keys | Strong | Medium | Elastic x-pack |
| RSALv2 | No | Cannot offer as managed service / commercialise | Strong | Medium-poor | Redis 7.4+ (option) |
| Custom "Community License" on binaries | No | Teleport CE binaries: commercial use limited to orgs <100 employees and <$10M revenue; no resale/embedding | Strong | Poor for mid-size companies, which is the intent | Teleport v16+ CE binaries |
| Proprietary source-available (`ee/LICENSE` family) | No | Use only with paid subscription; dev/test allowed; modifications owned by vendor | n/a (applies to paid part only) | Acceptable when core is OSI | GitLab EE, PostHog ee, Infisical ee, Bytebase enterprise |

[Observed: license texts fetched for Sentry FSL ("Competing Use" definition, Permitted Purposes), Vault BSL (IBM licensor, additional use grant), Elastic triple license, Redis tri-license, Bytebase/Infisical/PostHog/GitLab `ee` headers] [Inferred: adoption effects]

Key subtleties:

- **AGPL's trigger is modification + network interaction.** Unmodified self-hosting of Perde by a company triggers nothing beyond keeping the license notice; a hosted competitor that patches Perde must publish its patches. That is precisely the protection Perde's hosted business needs, without restricting users. [Inferred: from AGPL §13 wording]
- **Dual licensing (AGPL + commercial) requires the vendor to own or be licensed all copyright**, hence CLAs: NetBird's CLA "gives us the right to re-license the collective work going forward" — exactly what enabled its 2025 relicense. [Observed: `netbird/CONTRIBUTOR_LICENSE_AGREEMENT.md`; NetBird announcement]
- **Mixed-license repos need machine-readable boundaries.** NetBird uses REUSE (`LICENSES/REUSE.toml` with `default_license = "BSD-3-Clause"` and `[[files]] paths = ["management/", "signal/", "relay/"] license = "AGPL-3.0-only"`). Note a small inconsistency: the prose header of `LICENSE` also lists `combined/`, which `REUSE.toml` omits — exactly the kind of drift a CI check should catch. [Observed]
- **Bytebase's non-directory clause** ("any code controlling feature, permission, role and plan enablement") makes the boundary fuzzy and discourages forks touching IAM code (document 02 §3.1). Perde should use directory/crate boundaries only.

### 3.2 Per-project analysis

#### 3.2.1 Teleport

- **Licensing history:** Apache-2.0 for OSS until v14 → **AGPLv3 from v15** (source) → **v16 (2024): Community Edition binaries under the "Teleport Community License"**, limiting commercial use to organisations with fewer than 100 employees *and* less than $10M annual revenue, and forbidding resale/embedding; Enterprise binaries under commercial terms; source code remains AGPLv3 (users may still build from source). [Observed: `LICENSE` = AGPL-3.0; web: Teleport v16 release notes / GitHub discussion #39158, openSUSE packaging commit noting AGPL from 15.0.0] [Inferred: "build from source remains AGPL" from source license — Teleport's own FAQ should be confirmed]
- **Code split:** enterprise code lives in a private git submodule `e/` (pinned commit `42c822c…`); the public tree compiles without it. Runtime capability switches are a `Features` struct (`lib/modules/modules.go`: `Cloud`, `IsUsageBasedBilling`, `AdvancedAccessWorkflows`, `RecoveryCodes`, `Plugins`, `AutomaticUpgrades`, `AccessGraph`, `AccessControls`, `ProductType`, plus an `Entitlements` map) and a list of **entitlement kinds** (`entitlements/entitlements.go`): AccessGraph, AccessGraphDemoMode, AccessLists, AccessMonitoring, AccessRequests, ActivityCenter, App, Beams, ClientIPRestrictions, CloudAuditLogRetention, DB, Desktop, DeviceTrust, ExternalAuditStorage, FeatureHiding, HSM, Identity, JoinActiveSessions, LicenseAutoUpdate, MobileDeviceManagement, OIDC, OktaSCIM, OktaUserSync, Policy, SAML, SessionLocks, SessionSummaries, UnrestrictedManagedUpdates, UpsellAlert, UsageReporting, WorkloadClusters. [Observed]
- **Free vs paid (feature matrix, Community column):**

| Capability | Community | Enterprise |
|---|---|---|
| SSO | **GitHub only** | GitHub, Google Workspace, Entra ID, Okta, SailPoint, OIDC, SAML |
| SCIM provisioning | ✖ | Identity Governance add-on |
| Hardware private keys (YubiKey) | ✖ | ✔ |
| Device Trust | ✖ | ✔ |
| Per-session MFA | ✔ | ✔ |
| RBAC | ✔ | ✔ |
| JIT access requests & reviews | **"Only can request roles through CLI"** | Identity Governance |
| Session recording, enhanced (BPF) recording, protocol-level events | ✔ | ✔ |
| Dual authorization, session sharing & moderation | ✖ | ✔ |
| IP-based restrictions | ✖ | ✔ |
| Structured audit logs **and export to SIEM** | ✔ | ✔ |
| FedRAMP / FIPS binaries | ✖ | Self-hosted Enterprise |
| HSM/TPM for machine identity, external PKI, Sigstore | ✖ | ✔ |
| MCP server proxying, per-tool MCP audit | ✔ | ✔ |
| Per-agent cryptographic identity (AI agents) | ✖ | ✔ |
| Oracle database access | ✖ | ✔ |

[Observed: `docs/pages/feature-matrix.mdx` at the cloned commit]

- **Reaction:** v16 provoked public criticism (GitHub discussion #39158; an AWS Marketplace review titled "Complete bait-and-switch vendor lock-in"; HN thread "Ask HN: How did you replace Teleport?"). [Observed: search result titles; content not reproduced] The criticism centred on (a) restricting existing users mid-stream, (b) the SSO-only-GitHub limitation, (c) uncertainty about which binary/agent version is safe to run.
- **Lesson for Perde:** Teleport demonstrates that a strong AGPL source license plus *distribution-level* restrictions on binaries is legally possible but reputationally costly. It also shows the opposite good practice: audit export to SIEM and per-session MFA are free.

#### 3.2.2 Bytebase

Summarised from document 02: MIT core + Enterprise License carve-out; three plans in `backend/enterprise/plan.yaml` (FREE 10 instances/20 users; TEAM = "Pro" $20/user/month, 10 instances; ENTERPRISE custom); signed RS256 JWT license bound to workspace id; ~72 server-side gates; UI reads the same YAML. Paid: masking, classification, approval workflow, risk assessment, custom roles, JIT, request-role, OIDC/SAML/LDAP, 2FA enforcement, password policy, SCIM, token duration, external secret manager, watermark, environment tiers, full audit, HA. TEAM: Google/GitHub SSO, groups, read-only routing, query policy, 7-day audit window. FREE cannot search audit logs although rows are written. [Observed: document 02 §3.10]

Lessons: excellent *implementation* (single plan matrix, minimal-plan error messages, instance-scoped features, HA gate via replica heartbeat), poor *placement* for a security product (masking and approvals vanish on downgrade; audit invisible on FREE).

#### 3.2.3 NetBird

- **Split:** `LICENSE` header: BSD-3 for the repository except `management/`, `signal/`, `relay/`, `combined/` → AGPLv3 (each dir carries its own `LICENSE`). `REUSE.toml` formalises the three main dirs. Clients, the agent and libraries stay BSD-3. [Observed]
- **Timing:** effective from v0.53.0, announced 2025-08-04; not retroactive (prior versions remain BSD-3); enabled by the CLA. Dashboard repo also AGPL. [Observed: https://netbird.io/knowledge-hub/netbird-agpl-announcement (dated 2025-08-04; v0.53.0 scheduled/published 2025-08-05)]
- **Commercial license for self-hosted** adds: active-active HA for Management and Signal, SCIM, EDR/MDM integrations (CrowdStrike, SentinelOne, Intune…), traffic-flow event logging and streaming, peer approval, audit event streaming and notifications, standard support; license key `NB_LICENSE_KEY` validated against NetBird's license service; licenses per legal entity; 30-day assisted PoC. "Team" and "Business" are cloud plans. [Observed: docs.netbird.io/selfhosted/enterprise] A third-party blog reports a "Commercial Starter" self-hosted license at €2,000 (2026) — [Unknown: not verified against NetBird's own pricing page].
- **Reaction:** muted compared to HashiCorp; HN noted anyone can fork the last BSD version. The architecture-aligned split (permissive edge, copyleft control plane) is widely seen as fair. [Observed: search summary] [Inferred: tone]
- **Lesson for Perde:** this is the best template — same topology (P2P agents + signal/relay/management), copyleft only where hosting competition happens, permissive where adoption happens, CLA in place before any relicensing need.

#### 3.2.4 Tailscale and Headscale

- **Tailscale:** client/daemon BSD-3 (cross-platform `tailscaled`), GUI apps on proprietary OSes closed, coordination server closed SaaS. Pricing (2026): Personal $0 (up to 6 users, unlimited devices, 3 ACL groups), Standard $8/user/month (SCIM, 10 ACL groups, MDM/EDR posture integrations), Premium $18/user/month (network flow logs, log streaming, advanced SSH, 300 ACL groups), Enterprise custom; configuration audit logs and ACLs on all plans; SSH session recording via a PAM extension at Enterprise. [Observed: `tailscale/LICENSE`; Headscale README; tailscale.com/pricing] SSO: Tailscale has always required an identity provider for login (Google, Microsoft, GitHub, Apple, Okta, OIDC) — login *is* SSO even on free. [Inferred: product behaviour from general knowledge; the fetched pricing page did not state per-tier SSO]
- **Headscale:** BSD-3 self-hosted control server, deliberately narrow scope (single tailnet), "not associated with Tailscale Inc." but one maintainer is employed by Tailscale and may contribute on work time. [Observed: Headscale README]
- **Lesson:** a closed control plane did not hurt Tailscale's adoption because (a) the free tier is generous, (b) login uses existing IdPs, and (c) Tailscale embraced the community clone. Perde, needing self-hosting as a requirement, should simply be the "Headscale" itself.

#### 3.2.5 Infisical

- MIT + `ee/` (proprietary). [Observed: LICENSE header]
- Pricing (2026): Secrets Management Free (5 identities, 3 environments), Pro $20/identity/month annual (SAML SSO, RBAC/ABAC, 30-day audit retention, public-DB rotation), Advanced $40 (dynamic secrets, gateways, custom roles, SSO/MFA enforcement, 90-day audit), Enterprise (OIDC, LDAP, SCIM, approval workflows, KMIP, HSM, audit streaming, custom retention); PAM Pro $20/user (access requests, approvals, credential rotation, session recording 30 days). Self-hosted core free (MIT); licensed self-hosting for Enterprise. Free tier lists no SSO. [Observed: infisical.com/pricing]
- **Lesson:** Infisical gates even SAML at the first paid tier and OIDC at Enterprise — a pricing structure sso.tax would flag; but its *dynamic secrets* feature placement (Advanced) is a reasonable "advanced capability" gate.

#### 3.2.6 GitLab

- MIT CE + proprietary `ee/` in the same repository (single codebase, license-key unlocks EE features in the same binary — "buyer-based open core": features for individual contributors free, for managers Premium, for executives/security/compliance Ultimate). [Observed: LICENSE header] [Inferred: buyer-based model from GitLab's public stewardship/handbook statements, general knowledge]
- Typical split (verify against current docs before citing externally): self-managed Free includes LDAP and instance-wide OmniAuth (including SAML) login; GitLab.com group SAML SSO and SCIM are Premium; multiple required MR approvers and protected-environment deployment approvals are Premium; audit event streaming, compliance frameworks enforcement and most security scanners are Ultimate. [Inferred: general knowledge; the fetched comparison page did not render tier markers]
- **Lesson:** GitLab's long-standing "stewardship promise" (never move a feature from free to paid) is the trust anchor that made `ee/` acceptable to the community. Perde should publish an equivalent promise.

#### 3.2.7 Grafana

- Relicensed Grafana, Loki and Tempo from Apache-2.0 to AGPLv3 in 2021; agents/SDKs stayed Apache. [Observed: current license AGPL-3.0] [Inferred: date/scope from general knowledge]
- Enterprise-only (proprietary build): SAML, team sync, enhanced LDAP (active sync), protected roles, RBAC and data-source permissions, auditing, usage insights, recorded queries, reporting (PDF), white-labeling, enterprise data-source plugins (Datadog, Splunk, Snowflake…), Vault integration/KMS encryption, request security, query caching, runtime settings updates. OSS keeps basic OAuth (Google/GitHub/generic OAuth) and LDAP login. [Observed: Grafana Enterprise docs page]
- **Lesson:** AGPL relicense of a hugely popular project did not cause a fork, because nothing was taken away and AGPL is OSI-approved. Grafana *does* gate auditing and SAML — a classic SSO-tax pattern softened by free OAuth/OIDC.

#### 3.2.8 Sentry

- BSD-3 until 2019 → BSL 1.1 (2019) → **FSL-1.1-Apache-2.0** (2023): any use except "Competing Use" (making the software available to others in a commercial product/service that substitutes for it); each release becomes Apache-2.0 after two years. Permitted purposes explicitly include internal use, non-commercial education/research, and professional services to licensees. [Observed: `LICENSE.md`] Sentry co-founded the "Fair Source" label to describe such licenses. [Inferred: general knowledge]
- Self-hosted Sentry is feature-complete in principle (paid differences are SaaS conveniences/quotas); sso.tax historically listed Sentry's hosted SSO uplift at 208% (entry dated 2018). [Observed: sso.tax] [Inferred: self-hosted completeness]
- **Lesson:** FSL is a credible alternative to AGPL if the goal is "no hosted competitors for 2 years, then fully open"; it is not OSI-approved, so some distributions and companies will treat it as proprietary.

#### 3.2.9 PostHog

- MIT + `ee/` (proprietary); historically offered a `posthog-foss` mirror without `ee/`. Self-hosting is a "hobby" deployment; PostHog stopped supporting large-scale self-hosting and focuses on cloud with usage-based pricing and generous free volumes; enterprise features (SSO enforcement/SAML, advanced permissions, audit logs) sit in paid add-ons. [Observed: LICENSE header] [Inferred: product details from general knowledge; pricing page did not render]
- **Lesson:** when the product's value is the hosted service, open source becomes a distribution/marketing channel; acceptable for analytics, not for a security product whose users *must* self-host for compliance.

#### 3.2.10 Supabase

- Apache-2.0 monorepo; self-hosting via Docker/CLI with no feature gating stated. Hosted plans: Free, Pro from $25/month, Team from $599/month (dashboard SSO, platform audit logs, SOC2, 28-day logs), Enterprise; auth audit logs 1h/7d/28d by tier; PITR as paid add-on. [Observed: supabase.com/pricing]
- **Lesson:** "open everything, sell operations" works when the operational burden (Postgres HA, backups, scaling) is real. For Perde, rendezvous/relay infrastructure and HA control plane are the analogous operational burden.

### 3.3 Context: the relicensing wave and its forks

| Event | What changed | Community response | Outcome |
|---|---|---|---|
| MongoDB 2018 | AGPL → SSPL | Distros (Debian, Fedora, RHEL) dropped MongoDB; OSI rejected SSPL | Commercially successful; open-source community moved to Postgres/FerretDB |
| Elastic 2021 | Apache-2.0 → SSPL/ELv2 | AWS forked **OpenSearch** (Apache-2.0) | 2024: Elastic added **AGPLv3** as a third option ("open source again"); current `LICENSE.txt` triple-licensed, x-pack ELv2-only |
| HashiCorp Aug 2023 | MPL-2.0 → **BSL 1.1** for Terraform, Vault, Consul, Nomad, etc. | OpenTF manifesto → **OpenTofu** (Linux Foundation, MPL-2.0); **OpenBao** fork of Vault (MPL-2.0; started under LF Edge, moved to the OpenSSF in June 2025 — corrected after verification) | HashiCorp acquired by IBM (Vault LICENSE now names IBM as licensor); forks active |
| Redis Mar 2024 | BSD-3 → RSALv2/SSPL | **Valkey** (Linux Foundation, BSD-3) backed by AWS, Google, Oracle; most distros switch | Redis 8 (2025) added **AGPLv3** option; Valkey remains |
| Teleport 2024 (v16) | CE binaries → commercial "Community License" with size limits | Criticism, migration threads; no major fork observed | Teleport remains AGPL source + closed `e/` |
| NetBird Aug 2025 (v0.53) | Server dirs BSD-3 → AGPLv3 | Mild; fork possibility noted | Clients unaffected; commercial license for HA etc. |

[Observed: current license files of Vault, OpenBao, OpenTofu, Elasticsearch, Redis, Valkey, Teleport, NetBird] [Inferred: dates and events not individually fetched are from general knowledge and should be verified before external citation]

Pattern: **moving away from OSI licenses causes forks when (1) there are many external contributors, (2) hyperscalers depend on the software, and (3) an OSI-licensed last version exists.** Moving *to AGPL* (an OSI license) rarely causes forks.

### 3.4 The "SSO tax" and security paywalls

- sso.tax argues SSO is a core security requirement; gating it behind enterprise tiers, bundling, or seat minimums pushes companies toward weaker security; acceptable is SSO on all plans or a reasonable surcharge. Examples on the page: Sentry +208% (2018 entry), Docker +167%, Atlassian Jira Cloud +51%, Bytebase "call us" (2024). [Observed: sso.tax]
- Mapping of security-basic features to editions:

| Feature | Teleport CE | Bytebase FREE | NetBird CE (self-host) | Tailscale Free | Infisical Free | Grafana OSS | Supabase self-host |
|---|---|---|---|---|---|---|---|
| SSO with generic OIDC | ✖ (GitHub only) | ✖ (Team: Google/GitHub only) | ✔ (IdP-based login is core) [Inferred] | ✔ (IdP login) [Inferred] | ✖ | ✔ (generic OAuth) | ✔ (app auth) |
| SAML | ✖ | ✖ | [Unknown] | [Unknown] | ✖ | ✖ | ✔ (app auth; paid MAU on hosted) |
| SCIM | ✖ | ✖ | ✖ (commercial) | ✖ | ✖ | ✖ | n/a |
| MFA enforcement | per-session MFA ✔ | ✖ (2FA policy paid) | via IdP | via IdP | ✖ | via IdP | ✔ |
| Audit log visible | ✔ | ✖ | ✔ (activity) [Inferred] | ✔ config logs | ✖ | ✖ (auditing is Enterprise) | ✔ |
| Audit export/streaming | ✔ SIEM export | ✖ | ✖ (streaming commercial) | ✖ (Premium) | ✖ | ✖ | logs only |
| Approvals / JIT | CLI role requests only | ✖ | peer approval ✖ | ✖ | ✖ | n/a | n/a |
| Data masking | n/a | ✖ | n/a | n/a | n/a | n/a | n/a |
| HA control plane | ✔ (CE can run HA) [Inferred] | ✖ | ✖ (commercial) | n/a (SaaS) | ✔ (self-host) [Inferred] | ✔ | ✔ |

[Observed where not tagged; sources: feature tables above]

- **Arguments for gating SSO/SAML (vendor view):** SSO strongly correlates with "company with an IT department and budget", making it the most efficient price discriminator; SAML/SCIM integrations carry real support cost (IdP quirks, certificate rotation, per-customer debugging); enterprises rarely churn over price once SSO is integrated.
- **Arguments against (security view):** for a *privileged database access* product, identity is the security boundary — without SSO, offboarding relies on manually deleting Perde accounts, which is precisely the failure mode Perde exists to prevent; paywalling it means the free/self-hosted product is insecure by construction, damaging trust and the OSS brand; the most security-conscious early adopters (who evaluate on self-host) are the ones who hit the wall; reputational cost (sso.tax listing, HN threads) is real for a security tool.
- **Middle ground observed in the market:** free OIDC + paid SAML (Grafana, partly); free IdP-based login + paid SCIM (Tailscale, NetBird); free SSO + paid governance (Teleport would be this if it freed OIDC). The defensible line is: **authentication free, provisioning automation (SCIM/directory sync) paid**.

---

## 4. Algorithm specifications

Canonical access-control algorithms are in documents 04 and 05; the ALGs here cover the *open-core mechanics* Perde needs. Bytebase-derived license verification is described in document 02 (ALG-BYTEBASE-05); the ones below are the Perde designs.

### ALG-OPENCORE-01: Entitlement evaluation with fail-secure semantics

- **Problem:** Decide whether a Perde feature is available to a tenant, such that losing a license can never weaken security enforcement, and UI and API always agree.
- **Inputs / outputs:** tenant id; feature id; current time; effective entitlement set (from license or hosted subscription); feature registry (id → class, minimum plan, grace policy). Output: `Allowed | ReadOnly | Denied(reason, minimum_plan)`.
- **Algorithm:**
  1. Look up `feature` in the registry. If unknown → `Denied(unknown_feature)` and log an internal error (catches typos; registry is generated from one YAML used by server and UI).
  2. If `feature.class == ENFORCEMENT` (anything Karagöz enforces or Perde needs to *produce* enforcement artifacts: policy compilation, capability signing, revocation, audit ingest) → `Allowed` unconditionally. (Invariant I1: enforcement features are never in the paid set; CI test asserts it.)
  3. `ent ← effectiveEntitlements(tenant, now)` (ALG-OPENCORE-02).
  4. If `feature ∈ ent.features` → `Allowed`.
  5. If `feature ∈ ent.previously_entitled` and `now < ent.grace_until` → `Allowed` with a warning banner.
  6. If `feature ∈ ent.previously_entitled` (grace over) → `ReadOnly`: existing configuration keeps working (e.g. an existing SCIM connection keeps syncing deletions — deprovisioning must never stop; additions may be blocked), editing is blocked, data remains visible/exportable.
  7. Else → `Denied(minimum_plan(feature))`.
- **Data structures:** `feature_registry.yaml` {id, class ∈ {ENFORCEMENT, CORE, PAID}, min_plan, readonly_behavior}; per-tenant cached entitlement snapshot with version and `grace_until`.
- **Complexity:** O(1) after cache; cache TTL ≤ 60 s with explicit invalidation on license upload.
- **Edge cases (exhaustive list):** feature renamed between versions (keep aliases); license uploaded with features unknown to this Perde version (ignore unknown, log); downgrade while a paid workflow instance is mid-flight (let in-flight approval requests finish under old rules; block new ones); clock rollback on self-hosted server (use monotonic "last seen time" stored in DB to detect rollback; never extend grace on rollback); multi-replica caches disagreeing for ≤ TTL; air-gapped installs with no license server; hosted tenant payment failure (same path as expiry); SCIM deprovisioning during ReadOnly (must continue); feature both in base plan and add-on; trial ending; tenant migrated self-hosted → hosted (entitlements re-derived); offline Karagöz (never consults entitlements at all).
- **Test cases (input → expected):** (1) `policy.compile` with no license → Allowed. (2) `scim.configure` on Community → Denied(min=Business). (3) Business license expired yesterday, grace 14 d, `approvals.multistage.create` → Allowed + warning. (4) same after 15 d → ReadOnly; existing multi-stage rules still evaluated? → **Yes for evaluation if the rule tightens security** (see I2 below); creation blocked. (5) Unknown feature id → Denied + internal error metric. (6) license lists `future_feature_x` → ignored, no crash.
  - Invariant I2: a lapsed license may **remove convenience but never relax a restriction**: if a paid policy feature (e.g. multi-stage approval) was configured, its evaluation continues (more restrictive), or the system falls back to the *more* restrictive free equivalent (e.g. single-approver required) — never to "no approval".
- **Prior art and evidence:** Bytebase `IsFeatureEnabled` + plan.yaml shared with UI (doc 02); Teleport `Features`/entitlements; Bytebase anti-pattern "approval feature off ⇒ no approval" (`review/finding.go`).
- **Used by:** Perde.

### ALG-OPENCORE-02: Offline-verifiable license token for self-hosted Perde

- **Problem:** Self-hosted (possibly air-gapped) Perde must verify entitlements without contacting the vendor, resist trivial tampering, and support key rotation.
- **Inputs / outputs:** license string (compact token), embedded trust anchors (key set with ids and validity windows), installation id, now. Output: `EffectiveEntitlements{plan, features[], limits{users, agents, ha_replicas}, not_before, expires_at, grace_until, trial, licensee}` or `Invalid(reason)` (→ Community).
- **Algorithm:**
  1. Parse token header; require algorithm `EdDSA` (Ed25519) — reject all others (no algorithm negotiation).
  2. Find key by `kid` in the embedded key set; reject unknown `kid`; reject if now outside the key's validity window (supports rotation and compromise response by shipping new releases).
  3. Verify signature over the canonical payload bytes.
  4. Validate claims: `iss == "perde-licensing"`, `aud == "perde-selfhosted"`, `nbf ≤ now + skew`, version field supported.
  5. If `installation_id` claim present, require equality with this install's id (generated at first boot, stored in DB; exportable for offline license requests). Trials must always be bound.
  6. Compute `grace_until = expires_at + grace_days` (claim, default 14).
  7. If `now > grace_until` → Community entitlements but remember `previously_entitled = features` (for ReadOnly semantics in ALG-01).
  8. Cache result; persist "last verified now" to detect clock rollback (if `now < last_seen - 24h` → treat as `now = last_seen`).
- **Data structures:** key set `[{kid, public_key, valid_from, valid_to}]` embedded in the binary; license stored in DB settings; installation id.
- **Complexity:** one Ed25519 verify per cache refresh.
- **Edge cases:** token truncated/whitespace (trim, base64url strict); duplicate `kid`; key retired but license still valid (allow if signed while key valid: include `iat` and require `iat` within key window); license for different install; license with seats below current active users (do not lock anyone out; block new invites, show banner); clock skew ±5 min; leap seconds irrelevant; very large feature lists (cap 4 KB); unicode in licensee name (NFC, display only); air-gapped renewal by file upload; same license on two installs (detect via telemetry only if the operator opted in; otherwise rely on contract).
- **Test cases:** (1) valid token → entitlements. (2) `alg=RS256` with valid RSA sig → Invalid(alg). (3) payload bit flip → Invalid(signature). (4) wrong install id → Invalid(binding). (5) expired 3 days ago, grace 14 → features still Allowed with warning. (6) clock set back one year → uses last_seen; no resurrection of expired license.
- **Prior art and evidence:** Bytebase RS256 JWT with `kid`, `iss`, `aud`, workspace binding, expiry→FREE (doc 02 §3.10.2); NetBird `NB_LICENSE_KEY` validated online (docs.netbird.io) — Perde must support offline.
- **Used by:** Perde.

### ALG-OPENCORE-03: Downgrade and data-retention handling

- **Problem:** When a tenant loses a paid feature, decide what happens to stored data and configuration without data loss, security regression, or surprise.
- **Inputs / outputs:** feature transitions `(feature, from_state, to_state)`; stored objects per feature. Output: per-object action and user-visible notices.
- **Algorithm:**
  1. On entitlement change, diff old vs new feature sets.
  2. For each lost feature, apply its registry `readonly_behavior`:
     - *Data features* (long audit archive, session summaries): keep data; keep read and export; stop new writes to the paid store; new events continue into the core store (the core audit store is never gated).
     - *Policy features* (multi-stage approvals, advanced masking policies authored via paid UI): keep evaluating (ALG-01 I2); block edits except deletion; show "managed by paid feature — read-only".
     - *Integration features* (SCIM, SIEM connectors, ChatOps): keep deprovisioning/delete-propagation paths running; pause create/update; keep last sync state.
     - *Scale features* (HA replicas): refuse to start additional replicas; never stop the primary.
  3. Emit an audit event `entitlement.changed` with old/new sets.
  4. Notify org owners (email/in-app) with localized message ids.
- **Data structures:** registry behaviors; entitlement change log.
- **Complexity:** O(features × objects) once per change, batched.
- **Edge cases:** downgrade during an in-flight approval (complete under captured rules); re-upgrade (restore edit capability, nothing lost); tenant deletes paid-feature objects while ReadOnly (allowed — deletion reduces risk... except audit, which follows retention policy only); legal hold on audit data (overrides deletion); hosted tenant non-payment beyond N days (data export window, then contractually defined deletion — never silent).
- **Test cases:** (1) Business→Community with 3 multi-stage approval rules → rules still enforced, edit disabled, delete allowed. (2) SCIM lost → user deactivation from IdP still applied; new users not created. (3) Long-term audit archive lost → historical audit searchable/exportable; no new archive writes. (4) HA lost with 3 replicas running → replicas 2–3 refuse leadership/serving after restart; primary unaffected.
- **Prior art and evidence:** Bytebase audit retention implemented as search filter (data kept) vs. masking/approval silently disabled (doc 02 §7.1); GitLab stewardship promise [Inferred].
- **Used by:** Perde.

### ALG-OPENCORE-04: Code-boundary enforcement for a mixed-license repository

- **Problem:** Ensure AGPL core never depends on proprietary `ee/` code, permissive crates never depend on AGPL code, and every file declares its license — automatically, in CI.
- **Inputs / outputs:** repository tree, REUSE metadata, crate/module dependency graph. Output: pass/fail with violations.
- **Algorithm:**
  1. Load `REUSE.toml` (or SPDX headers) → map each path to license L ∈ {Apache-2.0, AGPL-3.0-only, LicenseRef-Perde-EE}.
  2. Verify every source file resolves to exactly one license; verify each license text exists in `LICENSES/`.
  3. Build the module dependency graph (Cargo workspace metadata for Rust crates; Go module/package imports or TS import graph for web).
  4. For each edge `a → b`: allowed iff `rank(L(a)) ≥ rank(L(b))` where rank(Apache)=0 < rank(AGPL)=1 < rank(EE)=2, i.e. permissive code may not import AGPL or EE; AGPL core may not import EE (EE plugs in through registered interfaces only).
  5. Verify the OSS build (`--no-default-features` / without `ee` feature) compiles and passes tests (job builds the Community artifact from a tree with `ee/` deleted).
  6. Verify the release artifacts' embedded SBOM lists the correct license per component.
- **Data structures:** path→license map; dependency graph; allowed-edge matrix.
- **Complexity:** O(files + edges).
- **Edge cases:** generated code (protobuf/OpenAPI) inherits the license of its source schema; vendored third-party code keeps its own license (exempt list with SPDX ids); test fixtures; documentation (CC BY 4.0 recommended); i18n resource files (same license as the component they ship with; translations contributed under the same CLA); build scripts; a shared protocol crate used by Hacivat, Karagöz and Perde must be Apache-2.0 (rank 0) so all three can depend on it; the drift case seen in NetBird (prose `LICENSE` vs `REUSE.toml` disagree) → CI compares both.
- **Test cases:** (1) Karagöz crate imports `perde-core` → fail. (2) `perde-core` imports `ee/scim` → fail. (3) `ee/scim` imports `perde-core` → pass. (4) new file without header in path not covered by REUSE → fail. (5) Community build with `ee/` removed → must compile.
- **Prior art and evidence:** NetBird `LICENSES/REUSE.toml`; GitLab `ee/` (EE extends CE via `prepend_mod` hooks) [Inferred]; Teleport `e/` submodule compiled in only for Enterprise builds [Observed: submodule].
- **Used by:** Hacivat, Karagöz, Perde (CI).

### ALG-OPENCORE-05: Fair active-user counting for hosted billing

- **Problem:** Bill hosted Perde per *active* user without penalising occasional break-glass users or AI agents, and without needing data-path visibility (P2P).
- **Inputs / outputs:** per-tenant events: logins, capability issuances (from Perde), agent-reported session starts (from audit uploads); billing period. Output: billable active-user count; free-tier eligibility.
- **Algorithm:**
  1. A *human user* is active in a period if Perde issued at least one database-access capability to them, or they performed an admin action, in that period. (Logins alone do not count.)
  2. Service accounts / AI-agent identities are counted separately: first N free per tenant, then a lower unit price (they cannot be "active" in the human sense).
  3. Break-glass: users with ≤1 capability issued in the period and only via an approved emergency request are not billed (encourages correct break-glass use instead of shared accounts).
  4. Deduplicate by user id across agents/regions.
  5. Free tier: tenant with ≤5 billable users and ≤3 agents is free (no SSO restriction).
- **Data structures:** per-period HyperLogLog or exact set of user ids per tenant (exact is fine at expected scale).
- **Complexity:** O(events).
- **Edge cases:** user renamed/merged via IdP (count once by stable subject id); user deleted mid-period (counted if active before); capability issued but never used (counted — Perde cannot see usage without audit; optionally refine with agent audit "session.start"); agents offline uploading audit late (period close waits for a grace window, then late events go to next period); time zones (UTC periods).
- **Test cases:** (1) 10 users logged in, 4 got capabilities → 4 billable. (2) one emergency-only user → 3 billable. (3) 2 AI agents → counted in agent identities bucket. (4) user active in both regions → 1.
- **Prior art and evidence:** Teleport "Pay only for what you use: active users and protected resources" (pricing page fetch); Tailscale per-user; Infisical per-identity; Bytebase per-seat (doc 02).
- **Used by:** Perde (hosted).

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 License per component (recommendation)

| Component | Recommended license | Rationale | Alternatives considered |
|---|---|---|---|
| **Hacivat** (desktop client, Rust) | **Apache-2.0** (or MIT OR Apache-2.0 dual, the Rust ecosystem norm) | Maximum adoption; offline use with no login is a core promise; patent grant protects users; desktop clients are not a hosting threat | MPL-2.0 (protects file-level changes; less familiar to Rust users) |
| **Karagöz** (agent) | **Apache-2.0** | Installed on production servers via a single command: security/legal teams must approve quickly; agents with AGPL or source-available terms delay adoption; Karagöz enforces security so it must be auditable and forkable by users; NetBird/Tailscale keep agents permissive | AGPL (would complicate embedding in customers' images; little benefit since agents are not hosted services) |
| **Shared protocol/SDK crates** (wire protocol, capability token format, audit record format) | **Apache-2.0** | Must be usable by all three components and by third parties (SIEM parsers, custom clients) | — |
| **Perde core** (control plane, rendezvous/signal, relay) | **AGPL-3.0-only** | Protects the hosted business from a cloud vendor offering modified Perde without contributing back; OSI-approved (no fork-trigger, no distro rejection); unmodified self-hosting carries no burden; matches Grafana/Teleport-source/NetBird server choice | FSL-1.1-Apache-2.0 (stronger anti-competition for 2 years, then permissive; not OSI → reputational risk for a security product); BSL (worse: longer change date, HashiCorp backlash association); Apache + `ee/` only (no protection for the hosted business; relies entirely on EE features) |
| **Perde EE** (`ee/` paid modules) | **Source-available commercial license** (`LicenseRef-Perde-EE`, modelled on the GitLab/PostHog `ee/LICENSE` family: free for dev/test, production with subscription) | Keeps everything in one repo (simpler development, auditable by customers), clear directory boundary | Private repo/submodule like Teleport `e/` (less transparency; harder customer security review) |
| **Docs** | CC BY 4.0 | Reuse in blogs/tutorials | CC BY-SA (GitLab) |
| **Contribution terms** | CLA for Perde (core and EE); DCO for Hacivat/Karagöz/SDK | Dual licensing of Perde (AGPL + commercial for AGPL-averse enterprises) needs the CLA; permissive components gain nothing from a CLA and DCO is friendlier | CLA everywhere (more friction); DCO everywhere (blocks future Perde commercial licensing) |
| **Trademarks** | Register "Hacivat", "Karagöz", "Perde" and logos; publish a trademark policy | Prevents confusing forks/hosted clones using the names while code stays open | — |

**Stewardship promise (publish on day one):** (1) features never move from free to paid; (2) security enforcement and security fixes are never paywalled; (3) the license of released versions never changes retroactively; (4) if the company stops maintaining Perde, the EE directory is relicensed under AGPL (a "dead-man's switch" clause, a commitment some vendors use to reassure customers). [Inferred: synthesis from GitLab stewardship + FSL conversion idea]

### 5.2 Feature split decision matrix

Scoring each candidate on: **S** = security criticality (if paywalled, does the free product become unsafe?), **E** = enforcement locality (must it run in Karagöz/Hacivat?), **C** = buyer correlation (only larger orgs need it), **O** = operational cost to vendor, **B** = backlash risk if gated. Rule: if S=High or E=Yes → free. Otherwise paid if C=High and B≤Medium.

| Feature | S | E | C | O | B | Decision |
|---|---|---|---|---|---|---|
| Agent enrollment, discovery, zero-credential connect | High | Yes | Low | Low | High | **Free** |
| Rendezvous / NAT traversal / P2P | High | — | Low | Medium | High | **Free** (self-host); hosted relay bandwidth metered |
| RBAC with predefined roles + label/attribute conditions | High | Yes | Low | Low | High | **Free** |
| Custom roles | Medium | Yes (evaluated at agent) | Medium | Low | High (Bytebase gates it; criticised) | **Free** |
| Read-only enforcement, statement classification, ephemeral DB users | High | Yes | Low | Low | High | **Free** |
| Column masking (policy engine + per-column rules + exemptions) | High | Yes | Medium | Low | High | **Free** |
| PII auto-classification (scanning data to suggest semantic types), classification dashboards | Medium | Partly | High | Medium | Low | **Paid** |
| Single-approver JIT access requests + statement-pinned grants | High | Verify at agent | Low | Low | High | **Free** |
| Multi-stage approval workflows, approval rules engine with risk/CEL, auto-approval policies, scheduled access, access reviews/certifications | Medium | Verify at agent | High | Medium | Medium | **Paid** (with I2: lapse never relaxes) |
| ChatOps approvals (Slack/Teams/PagerDuty/Jira) | Low | No | High | High | Low | **Paid** |
| Local audit at agent + hash chaining + upload + gap detection | High | Yes | Low | Low | High | **Free** |
| Audit search/export in Perde with operator-chosen retention (self-hosted) | High | No | Low | Low | High | **Free** |
| Generic SIEM export (JSON over webhook/syslog/file) | High | No | Medium | Low | High (Teleport CE has it) | **Free** |
| Managed SIEM connectors (Splunk HEC, Datadog, Sentinel, Chronicle, S3 Object-Lock archive), long-term managed retention (hosted) | Medium | No | High | High | Low | **Paid** |
| Session/query result recording (row-level capture) | Medium | Yes | High | High (storage) | Medium | **Paid** (capture engine OSS; managed storage/replay UI paid) |
| SSO: OIDC | High | No | Medium | Low | High | **Free** |
| SSO: SAML | High | No | High | Medium | High | **Free** (self-host); hosted: included from first paid tier, never "contact us" |
| MFA enforcement (WebAuthn/TOTP), per-session MFA for sensitive DBs | High | Partly | Low | Low | High | **Free** |
| SCIM / directory sync | Medium | No | High | Medium | Medium | **Paid** — *but* deprovisioning via OIDC `back-channel logout`/session revocation and short token TTL is free |
| Device trust (client attestation) | Medium | Partly | High | High | Medium | **Paid** |
| AI-agent controls in Hacivat MCP (read-only clamp, write confirmation, masked write-back refusal) | High | Yes | Low | Low | High | **Free** |
| AI-agent governance at org scale (per-agent identities, agent inventory, anomaly detection) | Medium | No | High | High | Low | **Paid** |
| HA / multi-region Perde, horizontal relays | Low | No | High | High | Low | **Paid** (self-host); hosted includes it |
| Fleet agent management (staged auto-updates, version policies) | Low | No | High | Medium | Low | **Paid** (basic self-update free) |
| Custom branding, custom domains (hosted) | Low | No | High | Low | Low | **Paid** |
| Compliance report packs (SOC 2/ISO 27001/KVKK/GDPR evidence exports) | Low | No | High | Medium | Low | **Paid** |
| SLA support, onboarding | — | — | High | High | None | **Paid** |

**Arguing both sides on SAML specifically:** Gating SAML would capture most enterprise revenue (SAML ≈ "has Okta/Entra with procurement"), and SAML support cost is real. But (a) OIDC is already free so the security argument is satisfied for most IdPs, and (b) SAML-only IdPs (ADFS, some Shibboleth deployments) are common in Turkish enterprises, banks and universities — exactly an early market. Recommendation: free in self-hosted Perde (the vendor's support cost is not incurred), included in the first paid hosted tier with published pricing. Revenue capture should come from SCIM, multi-stage approvals, managed audit, HA, and compliance packs — features that scale with organisational complexity.

### 5.3 Implementation guidance (code)

1. **One registry file** `perde/features.yaml` (id, class, min_plan, readonly_behavior, i18n keys for name/description) consumed by server (build-time codegen) and web UI (copied at build, as Bytebase does). CI test asserts no `ENFORCEMENT`-class feature has `min_plan != community`.
2. **Gate at the API boundary**, never inside Karagöz. Karagöz has **no license code at all** (it verifies policy/capability signatures from Perde, nothing else). This makes the "security never paywalled" promise structurally true and auditable.
3. **EE as plug-ins behind interfaces**: core defines traits/interfaces (`ApprovalWorkflowEngine`, `DirectoryProvisioner`, `AuditSink`, `Classifier`); core ships the free implementations (single-approver engine, OIDC JIT user creation, generic webhook sink, manual classification); `ee/` registers advanced implementations when entitled.
4. **Errors are typed and localizable**: `FeatureNotEntitled{feature_id, minimum_plan}` → UI renders localized upgrade copy; never a raw English string from the server.
5. **No phone-home requirement** for self-hosted (air-gapped banks); optional anonymous telemetry opt-in with HMAC-anonymised identifiers (Teleport documents HMAC-SHA-256 anonymisation with a cluster-local key — a good pattern). [Observed: Teleport `faq.mdx` lines 186-201]
6. **License file for hosted tenants** is the same token format, issued by billing, so hosted and self-hosted share one entitlement path (Bytebase does this: SaaS signs licenses with the private key).
7. **Release engineering**: two artifacts per release — `perde` (AGPL core only, `ee/` stripped at build) and `perde-ee` (core + ee). Community users can run either; the EE artifact without a license behaves exactly like Community (Bytebase and GitLab model), so customers can upgrade without redeploying.

### 5.4 Hosted pricing sketch (for discussion)

| Tier | Price idea | Includes |
|---|---|---|
| Free (hosted) | $0 | ≤5 active users, ≤3 agents, OIDC/SAML SSO, MFA, RBAC, masking, single-approver JIT, 30-day audit in Perde, generic SIEM webhook, community support |
| Team | per active user/month, published | Unlimited agents, 1-year audit, SCIM, multi-stage approvals, ChatOps, managed relays, email support |
| Business | per active user/month, published | Session/result recording storage, managed SIEM connectors, device trust, compliance packs, staged agent rollouts, custom roles templates library, 99.9% SLA |
| Enterprise / Self-hosted commercial | annual | `perde-ee` license for self-hosting, HA, air-gapped licensing, KVKK/GDPR DPA, dedicated support, AGPL-alternative commercial license if required by legal |

[Inferred: synthesis; numbers intentionally omitted until market research]

---

## 6. i18n considerations

- **License texts are not translated** in-repo; English is legally authoritative. A Turkish *informational* summary of the licenses and the stewardship promise may be offered in docs, clearly marked non-binding.
- Upgrade prompts, entitlement errors, plan names and feature names come from the feature registry as **message ids** (`feature.scim.name`, `entitlement.denied`, with arguments `{feature, minimum_plan}`) so adding a language requires no code change (Hacivat/Perde hard requirement). Plan names ("Free", "Team", "Business") should be translatable display strings mapped from stable ids.
- **Currency and invoicing:** hosted Perde selling to Turkish customers must handle TRY pricing or USD with Turkish e-invoice (e-Fatura/e-Arşiv) requirements; display prices with locale-aware formatting (`₺1.234,56` vs `$1,234.56`). Keep billing amounts in integer minor units (Bytebase stores cents in `pricing/plan.go`).
- **Data protection:** KVKK (Turkey's Personal Data Protection Law) and GDPR shape the "compliance pack" feature and hosted data residency; audit logs contain personal data (user identities, query text) — retention defaults and export tooling must be documented in Turkish and English.
- Turkish case-folding pitfalls apply to feature ids and license claim parsing: compare ids with ASCII-only lowercasing (a Turkish-locale `toLowerCase("SCIM")` produces `scım` with dotless ı and would fail lookup).
- Date formats in license UI (expiry, grace end) should be locale-formatted from UTC instants; never parse dates from localized strings.

---

## 7. Risks, pitfalls, open questions

### 7.1 Risks

1. **AGPL aversion**: some enterprises ban AGPL even for unmodified internal use. Mitigation: the commercial self-hosted license doubles as an AGPL alternative (requires CLA — decide before accepting first external Perde contribution).
2. **CLA friction** reduces contributions to Perde. Mitigation: lightweight CLA (license grant, not assignment), clear explanation of why (dual licensing), DCO for the permissive components where most community contributions (DB drivers, UI, i18n) are expected.
3. **Fork risk if the split is perceived as unfair** (HashiCorp/Redis lesson). Mitigation: OSI license for the core, stewardship promise, never relicense released versions, keep enforcement in Apache-licensed Karagöz.
4. **Under-monetisation**: if SSO, masking and approvals are free, revenue depends on governance/scale/operations. Teleport and Bytebase gate more aggressively; NetBird and Tailscale show the gentler model can work when hosted operations are valuable. Revisit after measuring which paid features prospects ask for.
5. **Hosted competitor using AGPL Perde unmodified**: AGPL does not prevent unmodified hosting by a third party. Mitigation: trademarks (they cannot call it Perde), velocity, hosted-only operational features (managed relays, global rendezvous), and EE modules.
6. **License-key bypass** in self-hosted EE builds: inevitable for source-available code; rely on contracts and trademark, not obfuscation (Bytebase/GitLab accept this). Never add DRM to the security path.
7. **Feature registry drift** between server and UI → users see enabled UI that the API refuses. Mitigation: single YAML, build-time copy/codegen, contract tests.

### 7.2 Pitfalls observed elsewhere (do not repeat)

- Security features that silently switch off on downgrade (Bytebase masking/approvals).
- SSO limited to one social provider in the free edition (Teleport CE: GitHub only).
- Retroactive tightening of the license applying to an existing user base (Teleport v16 binaries).
- Fuzzy license boundaries not tied to directories (Bytebase "any code controlling … permission, role …").
- Prose LICENSE and machine-readable REUSE metadata disagreeing (NetBird `combined/`).
- "Contact us" for security basics (sso.tax).

### 7.3 Open questions

- [Unknown] Whether NetBird's self-hosted "Commercial Starter" (€2,000, reported by a third party for 2026) exists as described; affects price anchoring for self-hosted Perde EE.
- [Unknown] GitLab's current tier for group SAML/SCIM and audit streaming (feature-comparison page did not render tier markers; verify on docs.gitlab.com before citing).
- [Unknown] PostHog's current add-on structure for SSO/audit (pricing page did not render).
- [Unknown] Whether Teleport CE built from AGPL source (rather than downloaded binaries) is free of the Community License restrictions in Teleport's own interpretation — affects how the market perceives "AGPL + binary license" strategies.
- Decision needed: AGPL-3.0-only vs FSL-1.1-Apache-2.0 for Perde core. This document recommends AGPL for OSI status and trust in a security product; FSL is the fallback if investors require explicit non-compete terms.
- Decision needed: whether the Rendezvous/relay service is licensed with Perde core (AGPL) or separately (Apache) so third parties can run compatible relays; recommendation: relay protocol spec Apache-2.0, reference relay AGPL with core.

---

*Sources (web):* [Bytebase pricing](https://www.bytebase.com/pricing) · [sso.tax](https://sso.tax/) · [Tailscale pricing](https://tailscale.com/pricing) · [Infisical pricing](https://infisical.com/pricing) · [Supabase pricing](https://supabase.com/pricing) · [Grafana Enterprise docs](https://grafana.com/docs/grafana/latest/introduction/grafana-enterprise/) · [GitLab feature comparison](https://about.gitlab.com/pricing/feature-comparison/) · [NetBird self-hosted enterprise](https://docs.netbird.io/selfhosted/enterprise) · [NetBird AGPL announcement](https://netbird.io/knowledge-hub/netbird-agpl-announcement) · [NetBird v0.53.0 release](https://newreleases.io/project/github/netbirdio/netbird/release/v0.53.0) · [Teleport v16 Community Edition discussion #39158](https://github.com/gravitational/teleport/discussions/39158) · [Teleport OSS relicense to AGPLv3](https://goteleport.com/blog/teleport-oss-switches-to-agpl-v3/) · [openSUSE Teleport packaging commit (AGPL since 15.0.0)](https://src.opensuse.org/pool/teleport/commit/4b5a74dfe04091f7f48a35c813dc79abecd278323d0341acb1d41857974800fb) · [Teleport CE license text (ScanCode)](https://scancode-licensedb.aboutcode.org/teleport-ce-2024.html) · [AWS Marketplace review "Complete bait-and-switch vendor lock-in"](https://aws.amazon.com/marketplace/reviews/reviews-list/prodview-fnmgbsyr3kwko/review/a99c0d7b-a8c3-3a80-bf8c-12cb1f17bb40) · [Teleport pricing](https://goteleport.com/pricing/) · [wz-it.com NetBird self-hosting license 2026](https://wz-it.com/en/blog/netbird-self-hosting-license-commercial-starter/). License files via GitHub API: getsentry/sentry `LICENSE.md`, hashicorp/vault `LICENSE`, elastic/elasticsearch `LICENSE.txt`, redis/redis `LICENSE.txt`, Infisical/infisical, PostHog/posthog, gitlabhq/gitlabhq `LICENSE`; SPDX ids for openbao/openbao, opentofu/opentofu, valkey-io/valkey, supabase/supabase, grafana/grafana, hoophq/hoop, dbeaver/cloudbeaver.
