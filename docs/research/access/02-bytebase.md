# 02 — Bytebase: SQL review, change approval, masking, SQL-editor access control, IAM and feature gating

> Research document for the Hacivat product family (Hacivat desktop client, Karagöz server agent, Perde control plane).
> Written 2026-11-11. All file paths are relative to the Bytebase repository root unless stated otherwise.

---

## 1. Scope and sources

### 1.1 What this document covers

Bytebase is the closest open-source analogue to what Perde + Karagöz want to offer on the *governance* side of database access: who may run which statement against which database, what must be approved by whom, which columns are masked for whom, and how all of it is audited. Unlike Teleport (document 01), Bytebase is **not** a wire-protocol proxy: it is a web application that holds database credentials ("data sources") and executes statements on the user's behalf through its own SQL editor and its own change-rollout engine. That difference is important for Hacivat: Bytebase can apply masking and statement classification because *its own server* sees every statement and every result row. Karagöz will be in the same position (it terminates the session on the database host), so most Bytebase mechanisms transfer, but Perde cannot use them centrally because Perde does not see the data path (P2P requirement).

Scope items studied:

1. SQL review rule catalog (every rule type, category, engine coverage, payload shape, severity, binding to environments/projects).
2. Change and approval workflows (plan → plan checks → issue → approval → rollout → task runs), approval-rule matching with CEL, risk levels, approval templates.
3. Data masking (semantic types, algorithms, classification levels, global masking rules, column-level overrides, exemptions, where in the result path masking is applied, side-channel defences).
4. SQL editor access control (read-only enforcement per engine, statement classification, per-statement ACL, query data policy, admin mode, export, just-in-time "access grants", request-role workflow).
5. Audit log (what is recorded, redaction, retention, export, stdout streaming, MCP/AI provenance).
6. IAM model (principals, predefined roles, permission catalog, workspace vs project policy, CEL conditions).
7. Feature gating in code (plan matrix, signed license, instance activation, HA replica gate, trial) and public pricing tiers.

Out of scope: schema-migration internals (gh-ost, declarative SDL), GitOps/VCS integration, Terraform provider, AI SQL assistant internals — except where they touch access control.

### 1.2 Sources

| Source | Version / commit | License | Notes |
|---|---|---|---|
| `github.com/bytebase/bytebase` (shallow clone at `.refs/repos/bytebase`) | commit `d651388204137f01ffb5ffc0c559dc885ca5f8b7`, committed 2026-11-11 | **Mixed**: MIT Expat for most code; *Bytebase Enterprise License* (source-available, proprietary) for (a) any source file whose directory name contains `enterprise` and (b) "any code controlling feature, permission, role and plan enablement" | See §3.1. Read-only study; nothing built or executed. |
| `LICENSE`, `LICENSE.enterprise` | same commit | — | Read in full. |
| bytebase.com/pricing (fetched 2026-11-11) | live page | — | Plan names and marketing feature table. |
| sso.tax (fetched 2026-11-11) | live page | — | Bytebase listed with "Call us" SSO pricing (entry dated 2024-02-12). |

Key files read (all paths relative to repo root):

- Licensing / gating: `backend/enterprise/plan.yaml`, `backend/enterprise/license.go`, `backend/enterprise/pricing/plan.go`, `backend/enterprise/pricing/pricing.go`, `backend/enterprise/keys/README.md`, `backend/api/v1/trial.go`, `frontend/src/types/plan.ts`, `frontend/scripts/copy-config-files.sh`.
- IAM: `backend/component/iam/manager.go`, `backend/component/iam/condition.go`, `backend/common/permission/permission.yaml`, `backend/store/predefined_roles.go`, `backend/common/cel.go`, `backend/common/cel_attributes.go`, `proto/store/store/policy.proto`.
- SQL review: `backend/plugin/advisor/advisor.go`, `backend/plugin/advisor/sql_review.go`, `backend/plugin/advisor/builtin_rules.go`, `backend/plugin/advisor/{mysql,pg,mssql,oracle,tidb,mariadb,oceanbase,snowflake,bigquery,redshift}/`, `proto/store/store/review_config.proto`, `backend/store/policy.go` (`GetReviewConfigForDatabase`), `frontend/src/types/sql-review-schema.yaml`, `frontend/src/locales/sql-review/en-US.json`.
- Workflow / approval: `backend/component/review/{evaluator.go,approval.go,workflow.go,finding.go,risk.go,rollout.go}`, `proto/store/store/{approval.proto,issue.proto,plan.proto,task_run.proto,project.proto,setting.proto}`.
- SQL editor: `backend/api/v1/sql_service.go`, `backend/api/v1/query_statement_classification.go`, `backend/api/v1/sql_service_export.go`, `backend/api/v1/access_grant_service.go`, `proto/store/store/access_grant.proto`, `backend/plugin/db/driver.go`, `backend/plugin/db/pg/pg.go`, `backend/plugin/db/redshift/redshift.go`, `backend/plugin/parser/pg/query_antlr.go`, `backend/plugin/parser/redis/query.go`, `backend/common/engine.go`.
- Masking: `backend/api/v1/masking_evaluator.go`, `backend/api/v1/query_result_masker.go`, `backend/api/v1/predicate_columns_check.go`, `backend/api/v1/mcp_masking.go`, `backend/component/masker/{masker.go,semantic_type.go}`, `proto/store/store/setting.proto` (`Algorithm`, `SemanticTypeSetting`, `DataClassificationSetting`).
- Audit: `backend/api/v1/audit.go`, `backend/api/v1/audit_redact.go`, `backend/api/v1/audit_log_service.go`, `proto/v1/v1/audit_log_service.proto`, `proto/v1/v1/annotation.proto`, `backend/server/server.go`.
- AI/MCP controls: `backend/api/v1/mcp_gate.go`, `backend/api/v1/mcp_sql_clamp.go`, `backend/api/v1/mcp_masking.go`.

**Clean-room note.** Because the license explicitly puts "any code controlling feature, permission, role and plan enablement" under the proprietary Enterprise License — a clause that is not bounded by directory and arguably reaches into `backend/store/predefined_roles.go`, `backend/common/permission/`, `backend/component/iam/` and every `IsFeatureEnabled` call site — this document describes **all** Bytebase mechanisms in prose and original pseudocode only. No code is reproduced. Short identifiers (enum names, permission strings, file names) are cited as facts for reference.

---

## 2. Executive summary

1. **License is "MIT + enterprise carve-out", not a pure open core.** `LICENSE` grants MIT for everything *except* directories whose name contains `enterprise` and "any code controlling feature, permission, role and plan enablement", which falls under `LICENSE.enterprise` (production use requires a paid subscription; modifications are owned by Bytebase). The only `enterprise` directory is `backend/enterprise/` (license verification, plan matrix, pricing). [Observed: `LICENSE`, `LICENSE.enterprise`, `find -name '*enterprise*'`]
2. **Feature gating is a single YAML matrix + a signed JWT.** `backend/enterprise/plan.yaml` lists three plans (FREE, TEAM, ENTERPRISE) with seat/instance limits and feature lists; the server embeds it and verifies an RS256 JWT license (`iss=bytebase`, `aud=bb.license`, `kid=v1`) against an embedded public key. Missing/invalid/expired license ⇒ FREE. The same YAML is copied into the frontend at build time so UI and API agree. [Observed: `backend/enterprise/license.go`, `frontend/scripts/copy-config-files.sh:14`]
3. **~72 server-side gate call sites** (`IsFeatureEnabled` / `IsFeatureEnabledForInstance`) guard individual RPCs or setting writes; gating is "refuse the write/read with PermissionDenied + upgrade message", never a separate binary. [Observed: grep over `backend/`]
4. **What is paid (Enterprise):** dynamic data masking, data classification, approval workflow, risk assessment, custom roles, JIT access, request-role workflow, enterprise SSO (OIDC/SAML/LDAP), 2FA enforcement, password policy, SCIM/directory sync, token-duration control, external secret managers, watermark, restrict-copy, environment tiers, unlimited audit retention, HA (multi-replica). **TEAM ("Pro", $20/user/month)** adds Google/GitHub SSO, user groups, database groups, read-only data-source routing, query policy, batch query, and a 7-day audit window. FREE: 10 instances, 20 users. [Observed: `plan.yaml`, `pricing/plan.go`, bytebase.com/pricing]
5. **Audit rows are written on every plan; only reading them is gated** (FREE: none, TEAM: last 7 days, ENTERPRISE: unlimited). Retention is a *query filter*, not deletion. [Observed: `audit.go` has no license dependency; `audit_log_service.go:36-50`; `license.go GetAuditLogRetentionDays`]
6. **IAM = Google-Cloud-style policies.** Bindings `{role, members[], condition(CEL)}` at workspace and project level; ~154 permissions named `bb.<collection>.<verb>`; 11 predefined roles; custom roles are Enterprise. Members: `users/{email}`, `groups/{email}`, `serviceAccounts/…`, `workloadIdentities/…`, `allUsers`. [Observed: `component/iam/manager.go`, `permission.yaml`, `predefined_roles.go`]
7. **CEL is used everywhere** with *different variable environments* per use: IAM conditions (environment, database, schema, table, request.time), masking rules (adds instance, project, column, classification_level), masking exemptions (adds request.time), approval rules (statement type, affected rows, table rows, SQL text, risk level, issue labels, requested role/unmask/export, expiration days). [Observed: `backend/common/cel.go`]
8. **SQL review catalog:** 105 rule types (enum `SQLReviewRule.Type`, 7 retired numbers reserved), in 11 categories, implemented as ~364 (rule × engine) advisors across MySQL, MariaDB, TiDB, OceanBase, PostgreSQL, Oracle, SQL Server, Snowflake, BigQuery, Redshift. Each rule has a level (ERROR/WARNING) and an optional typed payload (naming regex+max length, number, string list, comment convention, string, case). [Observed: `proto/store/store/review_config.proto`, `plugin/advisor/*`]
9. **Review configs bind by tag policy:** a database's effective SQL-review config is looked up first on its *environment*, then on its *project*; first enforced match wins (no merging). [Observed: `backend/store/policy.go:580-650`]
10. **Approval rules are an ordered list of `(source, CEL condition, template)`**; templates are an ordered list of *roles*; the first matching rule wins, then a fallback pass with only `resource.project_id` available. Approvers act strictly in order; one rejection blocks; the creator can re-request. Self-approval and last-plan-editor approval are project settings. [Observed: `component/review/evaluator.go:233-312`, `workflow.go:340-420`]
11. **Risk level is now computed in code, not configured:** HIGH = DROP DATABASE/TABLE/SCHEMA or TRUNCATE; MODERATE = DELETE/UPDATE/MERGE/EXECUTE/ALTER TABLE/DROP INDEX; otherwise LOW. The level is injected into the CEL environment as `risk.level`. [Observed: `component/review/risk.go`]
12. **Approvals are invalidated by content changes** via an `approval_input_version` on the plan plus optimistic concurrency (advisory lock, re-read, compare-and-swap on the issue payload). [Observed: `component/review/approval.go:50-200`]
13. **Read-only enforcement is mostly parser-based.** Only PostgreSQL, CockroachDB and Redshift (non-datashare) additionally open the session with `default_transaction_read_only=true`; every other driver ignores the read-only flag and relies on (a) a per-engine statement classifier and (b) a separately configured *read-only data source* (a DB user with read-only grants). [Observed: `plugin/db/driver.go:103-107`, `plugin/db/pg/pg.go:99`]
14. **Per-statement ACL on "new-ACL" engines** (MySQL, PostgreSQL, Oracle, SQL Server, TiDB, Snowflake, Spanner, BigQuery, MongoDB, Elasticsearch): every statement's *query span* (referenced databases/tables) is checked against `bb.sql.select|dml|ddl|explain|info|export` with the CEL attributes of each referenced database. Cross-database references must clear IAM on the foreign database even when a JIT grant matches. [Observed: `sql_service.go:1542-1700`]
15. **JIT access grants are pinned to an exact statement:** a grant stores the target databases, the query text, its SHA-256 (`query_hash`, whitespace-trimmed), unmask/export flags, schema, and TTL; it is created as an `ACCESS_GRANT` issue that goes through the approval engine; at query time an active grant matching creator + target + exact statement hash is auto-selected. [Observed: `proto/store/store/access_grant.proto`, `access_grant_service.go:128-280`, `sql_service.go:201-299`]
16. **Masking is applied in the server, after execution, per result column, using column lineage ("query span")**: each output column's *source columns* are resolved; 0 sources ⇒ no mask; exactly 1 plain source ⇒ that column's masker; 1 source through an expression or ≥2 sources ⇒ default full mask (`******`). Queries whose lineage cannot be resolved on a masking engine are refused. [Observed: `query_result_masker.go`]
17. **Side channels are taken seriously:** WHERE/JOIN predicates on a masked column are refused (PG, MySQL, MariaDB, SQL Server), DB error messages are redacted if the statement touched masked columns, and result/lineage column-count mismatches abort rather than mis-mask. [Observed: `predicate_columns_check.go`, `query_result_masker.go:40-90`]
18. **AI-agent (MCP) controls are first-class:** a per-workspace MCP capability ceiling, a read-only clamp on statements, explicit confirmation for writes, MCP-provenance fields on every audit row (`scope`, `client_id`, `correlation_id`), and a refusal of SQL that contains the full-mask sentinel `******` to stop agents writing masked placeholders back into real rows. [Observed: `mcp_gate.go`, `mcp_sql_clamp.go`, `mcp_masking.go`, `audit_log_service.proto:203-260`]
19. **Audit redaction is schema-driven:** a protobuf field option `audit_behavior` (`SENSITIVE`, `OMIT`) marks fields that must never be logged; a CI inventory test fails the build when a new field is not classified. [Observed: `audit_redact.go`, `proto/v1/v1/annotation.proto:40-60`]
20. **Main takeaways for Perde/Karagöz:** copy the *shapes* (CEL-conditioned bindings, statement-pinned JIT grants, lineage-based masking with fail-closed rules, field-level audit redaction annotations, approval input versioning), not the business model: Bytebase paywalls masking, approvals, SSO and audit visibility, which conflicts with a security product whose agent must enforce policy locally (see document 06).

---

## 3. Detailed findings

### 3.1 Licensing and repository layout

- `LICENSE` opens with "Portions of this software are licensed as follows": (1) source files that contain "enterprise" in their dirname → `LICENSE.enterprise`; (2) "Any code controlling feature, permission, role and plan enablement" → (by context) also `LICENSE.enterprise`; (3) everything else → MIT Expat. [Observed: `LICENSE` lines 1-8]
- `LICENSE.enterprise` (Bytebase Enterprise License): production use only with an agreed Bytebase Terms of Service / subscription "for the correct number of instance and paid plan"; modification and publishing patches allowed but all rights in modifications belong to Bytebase and may only be used with a subscription; free copy/modify for development and testing; otherwise forbidden to copy, merge, publish, distribute, sublicense or sell. This is structurally the same text family as GitLab's/PostHog's/Infisical's `ee/LICENSE` (source-available, not OSI). [Observed: `LICENSE.enterprise`] [Inferred: family resemblance from comparing with the `LICENSE` headers of GitLab, PostHog and Infisical fetched via GitHub API — see document 06]
- `find . -type d -name '*enterprise*'` returns exactly one directory: `backend/enterprise/` containing `plan.yaml`, `license.go`, `pricing/plan.go`, `pricing/pricing.go`, and `keys/{prod,dev}.pub.pem`. [Observed]
- **Ambiguity:** clause (2) is not directory-scoped. Predefined roles (`backend/store/predefined_roles.go`), the permission catalog (`backend/common/permission/permission.yaml`), the IAM checker (`backend/component/iam/`) and the ~72 `IsFeatureEnabled` call sites are all arguably "code controlling … permission, role and plan enablement". A fork that removes gates would be making modifications to such code. [Inferred: plain reading of the clause; not legal advice]
- Repository also contains `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `CONTEXT.md` (agent instructions for Bytebase's own contributors). They were treated as data and not followed. [Observed]
- Layout: Go backend (`backend/`), React/TypeScript frontend (`frontend/`, uses `@rollup/plugin-yaml`), protobuf APIs (`proto/v1` public API via ConnectRPC; `proto/store` storage payloads), Postgres metadata DB with migrations in `backend/migrator/migration/<version>/`. [Observed]

### 3.2 Domain model (needed to read everything else)

| Concept | Meaning | Evidence |
|---|---|---|
| Workspace | Tenant root. Self-hosted = one workspace; SaaS = many. Holds workspace IAM policy, settings (approval rules, semantic types, classification, masking rules), license. | `license.go` keyed by `workspaceID`; `policy.proto` `Policy_Resource WORKSPACE` |
| Environment | Ordered deployment stage (e.g. test, prod). Carries tag policy (SQL review config), rollout policy; "environment tier" (protected/unprotected) is Enterprise. | `policy.proto` `ENVIRONMENT`; `plan.yaml FEATURE_ENVIRONMENT_TIERS` |
| Project | Unit of ownership; databases belong to exactly one project; holds project IAM policy, masking exemptions, query-data policy, project settings (self-approval, JIT, require approval, data classification config…). | `proto/store/store/project.proto`; `policy.proto PROJECT` |
| Instance | A database server Bytebase connects to. Holds one or more **data sources**: one `ADMIN` and zero or more `READ_ONLY` (each a credential set; may come from an external secret manager). Instances can be "activated" against the license. | `sql_service.go routeDataSource`; `license.go IsInstanceEffectivelyActivated` |
| Database | A database (or schema, depending on engine) synced from an instance; has labels, effective environment, project, and catalog (per-column semantic type and classification). | `store/DatabaseMessage`; `ColumnCatalog` used in `masking_evaluator.go` |
| Database group | CEL-defined dynamic set of databases (Team+). | `plan.yaml FEATURE_DATABASE_GROUPS`; `DatabaseGroupCELAttributes` |
| Plan / Spec | A change request body: list of specs (create database, change database with sheet/release), with `approval_input_version` and `has_rollout`. | `proto/store/store/plan.proto` |
| Issue | Review object. Types: `DATABASE_CHANGE`, `ROLE_GRANT`, `ACCESS_GRANT`. Status: `OPEN`, `DONE`, `CANCELED`. Holds approval state and risk level. | `proto/store/store/issue.proto` |
| Rollout / Task / TaskRun | Execution of an approved plan, per database per environment stage; task run status `PENDING, RUNNING, DONE, FAILED, CANCELED, NOT_STARTED, SKIPPED…`. | `proto/store/store/task_run.proto` |
| Access grant | Time-boxed, statement-pinned read permission (JIT). Status `PENDING, ACTIVE, REVOKED`. | `proto/store/store/access_grant.proto` |
| Policy | Typed policy rows attached to workspace/environment/project: `ROLLOUT`, `MASKING_EXEMPTION`, `QUERY_DATA`, `MASKING_RULE`, `IAM`, `TAG`, `REVIEW_RULE`, `REVIEW_AI`. | `proto/store/store/policy.proto:11-27` |

### 3.3 IAM model

#### 3.3.1 Principals and members

- Principal kinds and their member strings: human users `users/{email}`, service accounts `serviceAccounts/{email}`, workload identities `workloadIdentities/{email}` (OIDC federation — e.g. a GitHub Actions token exchanged for a Bytebase token), groups `groups/{email}`, and the special `allUsers`. [Observed: `component/iam/manager.go formatUserNameByType`, `proto/store/store/policy.proto Binding` comment]
- `allUsers` semantics differ by level: at **workspace** level in SaaS mode it is *ignored* (members must be explicit); at **project** level it means "all workspace members". [Observed: `component/iam/manager.go doCheckPermission` comments]
- Group membership is resolved from a cached snapshot (`GetGroupMembersSnapshot`); groups can be synced from an IdP via SCIM/directory sync (Enterprise). [Observed: `manager.go`; `plan.yaml FEATURE_SCIM, FEATURE_DIRECTORY_SYNC`]

#### 3.3.2 Permission catalog

`backend/common/permission/permission.yaml` lists **154 permissions** of the form `bb.<collection>.<verb>`. Distribution by collection (count): policies 13, savedQueries 8, projects 8, users 7, sql 7, settings 7, releases 7, instances 7, databases 7, workloadIdentities 6, serviceAccounts 6, workspaces 5, roles 5, reviewConfigs 5, identityProviders 5, groups 5, databaseGroups 5, accessGrants 5, revisions 4, plans 4, issues 4, rollouts 3, issueComments 3, taskRuns 2, sheets 2, reviewRuns 2, planCheckRuns 2, instanceRoles 2, databaseCatalogs 2, changelogs 2, auditLogs 2, subscription 1, queryHistories 1. [Observed: `grep -oE 'bb\.[a-zA-Z]+' permission.yaml | sort | uniq -c`]

The data-plane permissions that matter most for Hacivat:

| Permission | Meaning (as used in `sql_service.go`) |
|---|---|
| `bb.sql.select` | Run read statements (SELECT, read-only commands) in the SQL editor. |
| `bb.sql.info` | Run information-schema / metadata reads (`SelectInfoSchema` span type). |
| `bb.sql.explain` | Run EXPLAIN; always comes from IAM, never from a JIT grant. |
| `bb.sql.dml` | Run INSERT/UPDATE/DELETE/MERGE directly in the editor (new-ACL engines). |
| `bb.sql.ddl` | Run DDL directly in the editor (new-ACL engines). |
| `bb.sql.export` | Export result sets (download / SharePoint). |
| `bb.sql.admin` | "Admin mode" — `AdminExecute` bidirectional stream using the admin data source. |
| `bb.accessGrants.{create,get,list,activate,revoke}` | JIT grant lifecycle. |
| `bb.policies.{get,update,create,delete}MaskingRulePolicy` / `…MaskingExemptionPolicy` | Masking administration split from generic policy permissions. |
| `bb.auditLogs.{search,export}` | Audit visibility. |
| `bb.projects.{getIamPolicy,setIamPolicy}`, `bb.workspaces.{getIamPolicy,setIamPolicy}` | Policy administration. |

[Observed: `permission.yaml`, `predefined_roles.go`, `proto/v1/v1/sql_service.proto` method options `(bytebase.v1.permission)`]

Each RPC declares its required permission and whether it is audited as **protobuf method options** (`(bytebase.v1.permission) = "bb.sql.admin"`, `(bytebase.v1.audit) = true`, `allow_without_credential`, `auth_method`). 117 RPCs are marked audited. [Observed: `proto/v1/v1/annotation.proto:12-16`, `grep -c '(bytebase.v1.audit) = true'`]

#### 3.3.3 Predefined roles

| Role (resource ID) | Display name | Level | Essence (summarised from permission sets) |
|---|---|---|---|
| `workspaceAdmin` | Workspace admin | workspace | Everything, incl. IAM, settings, subscription. |
| `workspaceDBA` | Workspace DBA | workspace | Instances, databases, policies, review configs, most project admin; no subscription/workspace IAM. |
| `workspaceMember` | Workspace member | workspace | Baseline: can create access grants, list roles/users/groups, read review configs and policies, create groups. |
| `projectOwner` | Project owner | project | Full project control incl. project IAM. |
| `projectDeveloper` | Project developer | project | Create/update issues & plans, run plan checks, view databases/schema/changelogs, saved queries, **search project audit logs**; no rollout create. |
| `projectReleaser` | Project releaser | project | Create rollouts and task runs; read plans/issues; comment. |
| `sqlEditorUser` | SQL Editor User | project | `bb.sql.select/info/explain/dml/ddl` + DB metadata reads. |
| `sqlEditorReadUser` | SQL Editor Read User | project | `bb.sql.select/info/explain` only. |
| `sqlEditorExporter` | SQL Editor Exporter | project | Read user + `bb.sql.export`. |
| `gitopsServiceAgent` | GitOps Service Agent | project | Plans, releases, rollouts, task runs (CI bot). |
| `projectViewer` | Project viewer | project | Read project, databases, schema; create issues/comments (so viewers can *request* changes). |

[Observed: `backend/store/predefined_roles.go:29-660`; migration history shows renames, e.g. `roles/projectQuerier → roles/sqlEditorUser` (`migration/3.1/0002##rename_project_querier.sql`) and removal of `projectExporter` merged into a condition on `sqlEditorUser` (`migration/3.10/0007##remove_exporter_role.sql`)]

Custom roles (any subset of permissions) require `FEATURE_CUSTOM_ROLES` (Enterprise). [Observed: `role_service.go:77,125`]

#### 3.3.4 Policies, bindings and conditions

- An IAM policy is a list of bindings `{role: "roles/<id>", members: [...], condition: google.type.Expr}`. Policies exist at workspace and project level; effective permission = OR over both. [Observed: `policy.proto Binding/IamPolicy`; `manager.go doCheckPermission`]
- The *generic* permission check (`CheckPermission`) **ignores CEL except for expiry**: it validates the binding (dropping bindings whose time window closed) and then treats the binding as granting. Resource-scoped conditions (database/schema/table/environment) are evaluated only by the data-plane checks in `sql_service.go` (`hasDatabaseAccessRights`). [Observed: comment "CEL on the binding is not considered" in `manager.go`; `sql_service.go:2399-2438`]
- `CheckProjectWidePermission` is a hardening variant: a binding whose condition references *any* CEL variable other than `request.time` is skipped entirely, so a "table-slice" grant cannot widen into project-wide operations. The detection uses the CEL *type-checker's reference map* (not AST walking), so macros/comprehensions are seen through and any newly declared attribute is automatically treated as scoping. [Observed: `component/iam/condition.go conditionScopesResources`]
- Multi-project checks require the permission in **every** listed project. [Observed: `manager.go` "When multiple projects are specified, the user should have permission on every projects"]
- IAM-condition CEL environment: `resource.environment_id`, `resource.database` (full name `instances/{i}/databases/{d}`), `resource.schema_name`, `resource.table_name`, `request.time`; string extension functions; max expression size 1 MiB. [Observed: `common/cel.go:101-109`]
- Quirk: for permissions other than `bb.sql.ddl`/`bb.sql.dml`, a `resource.environment_id in [...]` clause is **stripped by regex** from the expression before evaluation ("environment only applies to DDL/DML"). This is textual surgery on CEL and is fragile (e.g. `!(resource.environment_id in [...])` or `||` combinations). [Observed: `sql_service.go stripEnvironmentCondition`, `envConditionRe`] [Inferred: fragility]

### 3.4 SQL review

#### 3.4.1 Architecture

- An **advisor** is a per-(engine, rule-type) checker registered at init into a two-level map `engine → ruleType → Advisor`; duplicate registration panics. `Check` dispatches and recovers panics into an error advice. [Observed: `plugin/advisor/advisor.go:118-178`]
- The advisor `Context` carries: the parsed statements (ASTs from Bytebase's own "omni" parser stack), the database schema *before* (`OriginalMetadata`) and *after* (`FinalMetadata`, produced by a "walk-through" that applies the DDL to an in-memory catalog), an optional live `*sql.DB` driver (for dry-run / affected-row estimates), session user, current database, engine, object case sensitivity, flags for prior-backup and gh-ost, a per-review memo cache. [Observed: `advisor.go Context`]
- **Builtin rules** are appended unless the user config overrides the same (type, engine): `BUILTIN_STATEMENT_MAXIMUM_SQL_SIZE` (all engines, WARNING, default `common.MaxSheetCheckSize`), `BUILTIN_PRIOR_BACKUP_CHECK` (MySQL, MariaDB, PostgreSQL, TiDB, SQL Server, Oracle), `BUILTIN_WALK_THROUGH_CHECK` (MySQL, MariaDB, TiDB, PostgreSQL, OceanBase). [Observed: `builtin_rules.go`]
- Evaluation order (`SQLReviewCheck`): (1) append builtins; (2) **pre-parse** size rule — if the SQL exceeds the size limit, return only that advice (parser never runs on huge input); (3) split/parse statements; (4) if a final schema is available, run the walk-through — a walk-through error (e.g. referencing a non-existent table) short-circuits with a single advice at the walk-through rule's level; (5) run each configured rule whose engine matches; collect advices into ERROR and WARNING buckets each capped at `common.MaximumAdvicePerStatus`; stop early when both are full; (6) return errors first, then warnings. Advisor internal errors become an ERROR advice titled with the rule type (fail-visible, not fail-silent). [Observed: `sql_review.go:94-215`]
- Naming rules support template tokens `{{table}}`, `{{column_list}}`, `{{referencing_table}}`, `{{referencing_column}}`, `{{referenced_table}}`, `{{referenced_column}}`, with a default max identifier length 63 (PostgreSQL's limit) used for MySQL too. [Observed: `sql_review.go:24-75`]
- Rule payload types (proto oneof): `NamingRulePayload{max_length, format}`, `NumberRulePayload{number}`, `StringArrayRulePayload{list}`, `CommentConventionRulePayload{required, max_length}`, `StringRulePayload{value}`, `NamingCaseRulePayload{upper}`. Levels: `ERROR`, `WARNING`. Each rule instance also carries its `engine`, so one review config holds per-engine rule rows. [Observed: `proto/store/store/review_config.proto`]

#### 3.4.2 Binding review configs to databases

- A review config (`ReviewConfigPayload{sql_review_rules[]}` + `enforce` flag) is attached to an environment or project through a **TAG policy** whose map contains the reserved key `bb.tag.review_config` → `reviewConfigs/{id}`. [Observed: `policy.proto TagPolicy` comment; `store/policy.go getReviewConfigByResource`]
- Resolution for a database: try the database's **effective environment** first, then its **project**; the first resource that has an enforced tag policy pointing at an enforced config wins. **No merge** of environment and project rules. [Observed: `store/policy.go:580-608`]
- Project setting `enforce_sql_review` and `require_plan_check_no_error` decide whether ERROR advices block rollout. [Observed: `proto/store/store/project.proto` fields 14, 16]
- Separate **AI review** policy (`REVIEW_AI`) holds natural-language guidance; both workspace and project policies reach the AI reviewer, project wins on conflict. [Observed: `policy.proto ReviewAIPolicy`; `store/policy.go:550-573`]
- The UI ships three templates (`sql-review.sample.yaml`, `sql-review.dev.yaml`, `sql-review.prod.yaml`) and a full schema `sql-review-schema.yaml` with 384 (rule, engine) entries — slightly more than the backend registers, since the UI schema lists builtins and a few rules per engine that the backend implements generically. [Observed: `frontend/src/types/`]

#### 3.4.3 Full rule catalog

Legend for engines: MY MySQL, MA MariaDB, TI TiDB, OB OceanBase, PG PostgreSQL, OR Oracle, MS SQL Server, SF Snowflake, BQ BigQuery, RS Redshift. Category comes from `frontend/src/types/sql-review-schema.yaml`; title from `frontend/src/locales/sql-review/en-US.json` (MIT part of the repo; titles are short labels, reproduced as facts). Engines are the backend registrations found by scanning `advisor.Register(storepb.Engine_X, storepb.SQLReviewRule_Y, …)` in `backend/plugin/advisor/**`. Enum numbers 32, 35, 36, 39, 41, 42, 44 are reserved for retired rules (full-table-scan, filesort, temporary, minimum plan level, max LIMIT value, max join tables, join strict column attrs) — i.e. Bytebase removed every *EXPLAIN-plan-based* rule from the catalog. [Observed: `review_config.proto` reserved comments]

| Category | Rule type | Title | Engines |
|---|---|---|---|
| COLUMN (not in UI schema) | `COLUMN_ADD_NOT_NULL_REQUIRE_DEFAULT` | (no UI title; Oracle advisor only) | OR |
| ADVICE | `ADVICE_ONLINE_MIGRATION` | Online migration | MA MY |
| BUILTIN | `BUILTIN_PRIOR_BACKUP_CHECK` | Prior backup feasibility check | MA MS MY OR PG TI |
| BUILTIN | `BUILTIN_STATEMENT_MAXIMUM_SQL_SIZE` | Restrict the SQL file size | all (pre-parse check in `sql_review.go`, no per-engine advisor) |
| BUILTIN | `BUILTIN_WALK_THROUGH_CHECK` | Schema walk-through validation | MA MY OB PG TI |
| COLUMN | `COLUMN_AUTO_INCREMENT_INITIAL_VALUE` | Restrict the initial value of auto-increment columns | MA MY OB TI |
| COLUMN | `COLUMN_AUTO_INCREMENT_MUST_INTEGER` | Enforce the use of "INTEGER" data type for auto-increment columns | MA MY OB TI |
| COLUMN | `COLUMN_AUTO_INCREMENT_MUST_UNSIGNED` | Enforce the use of "UNSIGNED" data type for auto-increment columns | MA MY OB TI |
| COLUMN | `COLUMN_COMMENT` | Column comment convention | MA MY OB OR PG TI |
| COLUMN | `COLUMN_CURRENT_TIME_COUNT_LIMIT` | Restrict the number of columns in the table that acquire system time | MA MY OB TI |
| COLUMN | `COLUMN_DEFAULT_DISALLOW_VOLATILE` | Disallow setting volatile default value on columns | PG |
| COLUMN | `COLUMN_DISALLOW_CHANGE` | Prohibit using "CHANGE COLUMN" statement | MA MY OB TI |
| COLUMN | `COLUMN_DISALLOW_CHANGE_TYPE` | Prohibit modifying column types | MA MY OB PG TI |
| COLUMN | `COLUMN_DISALLOW_CHANGING_ORDER` | Prohibit changing the order of columns in a table | MA MY OB TI |
| COLUMN | `COLUMN_DISALLOW_DROP` | Prohibit dropping columns | OB |
| COLUMN | `COLUMN_DISALLOW_DROP_IN_INDEX` | Prohibit dropping indexed columns | MA MY OB TI |
| COLUMN | `COLUMN_DISALLOW_SET_CHARSET` | Prohibit defining character set in column properties | MA MY OB TI |
| COLUMN | `COLUMN_MAXIMUM_CHARACTER_LENGTH` | Restrict the length of "CHAR" data type | MA MY OB OR PG TI |
| COLUMN | `COLUMN_MAXIMUM_VARCHAR_LENGTH` | Restrict the length of "VARCHAR" data type | MA MS MY OB OR SF |
| COLUMN | `COLUMN_NO_NULL` | Enforce "NOT NULL" constraints on columns | MA MS MY OB OR PG SF TI |
| COLUMN | `COLUMN_REQUIRED` | Enforce the inclusion of specific columns in a table | MA MS MY OB OR PG SF TI |
| COLUMN | `COLUMN_REQUIRE_CHARSET` | Require charset for text columns | MY |
| COLUMN | `COLUMN_REQUIRE_COLLATION` | Require collation for text columns | MY |
| COLUMN | `COLUMN_REQUIRE_DEFAULT` | Enforce setting default value on columns | MA MY OB OR PG TI |
| COLUMN | `COLUMN_SET_DEFAULT_FOR_NOT_NULL` | Enforce default value on "NOT NULL" columns | MA MY OB TI |
| COLUMN | `COLUMN_TYPE_DISALLOW_LIST` | Prohibit the use of certain column data types | MA MS MY OB OR PG TI |
| DATABASE | `DATABASE_DROP_EMPTY_DATABASE` | Prohibit deleting non-empty database | MA MY OB TI |
| ENGINE | `ENGINE_MYSQL_USE_INNODB` | Enforce InnoDB storage engine | MA MY |
| INDEX | `INDEX_CREATE_CONCURRENTLY` | Enforce concurrent index creation | PG |
| INDEX | `INDEX_KEY_NUMBER_LIMIT` | Restrict the number of columns in a single index | MA MY OB OR PG TI |
| INDEX | `INDEX_NOT_REDUNDANT` | Disallow redundant indexes | MS |
| INDEX | `INDEX_NO_DUPLICATE_COLUMN` | Prohibit indexes containing duplicate columns | MA MY OB PG TI |
| INDEX | `INDEX_PK_TYPE_LIMIT` | Primary key type limit | MA MY OB TI |
| INDEX | `INDEX_PRIMARY_KEY_TYPE_ALLOWLIST` | Allowable list of primary key types | MY OB PG TI |
| INDEX | `INDEX_TOTAL_NUMBER_LIMIT` | Restrict the number of indexes on a single table | MA MY OB PG TI |
| INDEX | `INDEX_TYPE_ALLOW_LIST` | Allowable list of index types | MY |
| INDEX | `INDEX_TYPE_NO_BLOB` | Prohibit creating indexes on "BLOB" and "TEXT" data type columns | MA MY OB TI |
| NAMING | `NAMING_COLUMN` | Enforce column naming format | MA MY OB PG TI |
| NAMING | `NAMING_COLUMN_AUTO_INCREMENT` |  Enforce auto-increment column naming format | MA MY OB TI |
| NAMING | `NAMING_FULLY_QUALIFIED` | Fully qualified object name | PG |
| NAMING | `NAMING_IDENTIFIER_CASE` | Enforce identifier case | OR SF |
| NAMING | `NAMING_IDENTIFIER_NO_KEYWORD` | Prohibit using keywords as identifiers | MS MY OR SF |
| NAMING | `NAMING_INDEX_FK` | Enforce foreign key naming format | MA MY OB PG TI |
| NAMING | `NAMING_INDEX_IDX` | Enforce index naming format | MA MY OB PG TI |
| NAMING | `NAMING_INDEX_PK` | Enforce primary key naming format | PG |
| NAMING | `NAMING_INDEX_UK` | Enforce unique key naming format | MA MY OB PG TI |
| NAMING | `NAMING_TABLE` | Enforce table naming format | MA MS MY OB OR PG SF TI |
| NAMING | `NAMING_TABLE_NO_KEYWORD` | Prohibit using keywords as table names | MS OR SF |
| SCHEMA | `SCHEMA_BACKWARD_COMPATIBILITY` | Check application backward compatibility | MA MS MY OB PG SF TI |
| STATEMENT | `STATEMENT_ADD_CHECK_NOT_VALID` | Enforce including "NOT VALID" option when adding "CHECK" constraints | PG |
| STATEMENT | `STATEMENT_ADD_COLUMN_WITHOUT_POSITION` | Check no position in ADD COLUMN clause | OB |
| STATEMENT | `STATEMENT_ADD_FOREIGN_KEY_NOT_VALID` | Enforce including "NOT VALID" option when adding foreign keys | PG |
| STATEMENT | `STATEMENT_AFFECTED_ROW_LIMIT` | Restrict the maximum number of updated or deleted rows (estimated). | MA MY OB PG |
| STATEMENT | `STATEMENT_CHECK_SET_ROLE_VARIABLE` | Check if Set Role statement at the beginning | PG |
| STATEMENT | `STATEMENT_CREATE_SPECIFY_SCHEMA` | Prohibit creating objects without specifying the schema | PG |
| STATEMENT | `STATEMENT_DISALLOW_ADD_COLUMN_WITH_DEFAULT` | Restrict adding columns with default values to a table | PG |
| STATEMENT | `STATEMENT_DISALLOW_ADD_NOT_NULL` | Restrict adding "NOT NULL" constraint to existing columns | PG |
| STATEMENT | `STATEMENT_DISALLOW_COMMIT` | Prohibit explicit "COMMIT" statement | MA MY OB PG TI |
| STATEMENT | `STATEMENT_DISALLOW_CROSS_DB_QUERIES` | Disallow cross database queries | MS |
| STATEMENT | `STATEMENT_DISALLOW_LIMIT` | Prohibit using "LIMIT" clause in DML statements | MA MY OB TI |
| STATEMENT | `STATEMENT_DISALLOW_OFFLINE_DDL` | Disallow Offline DDL | OB |
| STATEMENT | `STATEMENT_DISALLOW_ON_DEL_CASCADE` | Prohibit using CASCADE option for ON DELETE clauses | PG |
| STATEMENT | `STATEMENT_DISALLOW_ORDER_BY` | Prohibit using "ORDER BY" clause in "UPDATE" and "DELETE" statements | MA MY OB TI |
| STATEMENT | `STATEMENT_DISALLOW_RM_TBL_CASCADE` | Prohibit using CASCADE when removing a table | PG |
| STATEMENT | `STATEMENT_DISALLOW_TRUNCATE` | Prohibit "TRUNCATE" statement | MA MS MY OB OR PG |
| STATEMENT | `STATEMENT_DML_DRY_RUN` | Validate the executability of DML statements | MA MY OB OR PG TI |
| STATEMENT | `STATEMENT_INSERT_DISALLOW_ORDER_BY_RAND` | Prohibit using "ORDER BY rand()" in "INSERT" statement | MA MY OB PG TI |
| STATEMENT | `STATEMENT_INSERT_MUST_SPECIFY_COLUMN` | Enforce specifying column names in "INSERT" statements | MA MY OB OR PG TI |
| STATEMENT | `STATEMENT_INSERT_ROW_LIMIT` | Restrict the maximum number of inserted rows | MA MY OB PG |
| STATEMENT | `STATEMENT_MAXIMUM_STATEMENTS_IN_TRANSACTION` | Restrict the number of statements in a transaction | MY |
| STATEMENT | `STATEMENT_MAX_EXECUTION_TIME` | Require an explicit maximum execution time | MA MY |
| STATEMENT | `STATEMENT_MERGE_ALTER_TABLE` | Prohibit issuing multiple independent changes on the same table | MA MY OB PG TI |
| STATEMENT | `STATEMENT_NON_TRANSACTIONAL` | Detect and report non-transactional statements | PG |
| STATEMENT | `STATEMENT_OBJECT_OWNER_CHECK` | Object owner check | PG |
| STATEMENT | `STATEMENT_REQUIRE_ALGORITHM_OPTION` | Require specifying the ALGORITHM option in ALTER TABLE statements | MA MY |
| STATEMENT | `STATEMENT_REQUIRE_LOCK_OPTION` | Require specifying the LOCK option in ALTER TABLE statements | MA MY |
| STATEMENT | `STATEMENT_SELECT_NO_SELECT_ALL` | Prohibit using "SELECT *" | MA MS MY OB OR PG SF TI |
| STATEMENT | `STATEMENT_WHERE_DISALLOW_FUNCTIONS_AND_CALCULATIONS` | Do not apply functions or perform calculations on indexed fields in the WHERE clause | MS MY OR PG |
| STATEMENT | `STATEMENT_WHERE_MAXIMUM_LOGICAL_OPERATOR_COUNT` | Restrict the number of values in the IN or OR clause of the WHERE clause | MY |
| STATEMENT | `STATEMENT_WHERE_NO_EQUAL_NULL` | Prohibit using NULL equality comparison in WHERE clause | MY |
| STATEMENT | `STATEMENT_WHERE_NO_LEADING_WILDCARD_LIKE` | Prohibit using leading wildcard in filter conditions | MA MY OB OR PG TI |
| STATEMENT | `STATEMENT_WHERE_REQUIRE_SELECT` | Enforce the presence of "WHERE" condition in SELECT statements | MA MS MY OB OR PG SF TI |
| STATEMENT | `STATEMENT_WHERE_REQUIRE_UPDATE_DELETE` | Enforce the presence of "WHERE" condition in UPDATE/DELETE statements | BQ MA MS MY OB OR PG SF TI |
| SYSTEM | `SYSTEM_CHARSET_ALLOWLIST` | Allowable list of Charset | MA MY OB PG TI |
| SYSTEM | `SYSTEM_COLLATION_ALLOWLIST` | Allowable list of Collation | MA MY OB PG TI |
| SYSTEM | `SYSTEM_COMMENT_LENGTH` | Restrict the length of comments | PG |
| SYSTEM | `SYSTEM_EVENT_DISALLOW_CREATE` | Prohibit creating events | MY |
| SYSTEM | `SYSTEM_FUNCTION_DISALLOWED_LIST` | Prohibit the use of certain functions | MY |
| SYSTEM | `SYSTEM_FUNCTION_DISALLOW_CREATE` | Prohibit creating functions | MS MY |
| SYSTEM | `SYSTEM_PROCEDURE_DISALLOW_CREATE` | Prohibit creating procedures | MS MY |
| SYSTEM | `SYSTEM_VIEW_DISALLOW_CREATE` | Prohibit creating views | MY |
| TABLE | `TABLE_COMMENT` | Table comment convention | MA MY OB OR PG TI |
| TABLE | `TABLE_DISALLOW_DDL` | Disallow DDL | MS MY |
| TABLE | `TABLE_DISALLOW_DML` | Disallow DML | MS MY |
| TABLE | `TABLE_DISALLOW_PARTITION` | Prohibit using partition table | MA MY OB PG TI |
| TABLE | `TABLE_DISALLOW_SET_CHARSET` | Prohibit defining character set in table properties | MY |
| TABLE | `TABLE_DISALLOW_TRIGGER` | Prevent the use of triggers on tables | MY |
| TABLE | `TABLE_DROP_NAMING_CONVENTION` | Restrict the naming format of tables to be deleted | MA MS MY OB PG RS SF TI |
| TABLE | `TABLE_LIMIT_SIZE` | Limit DDL operations on tables with a large number of rows | MY |
| TABLE | `TABLE_NO_DUPLICATE_INDEX` | Prohibit duplicate indexes on table | MY |
| TABLE | `TABLE_NO_FOREIGN_KEY` | Prohibit using foreign key constraints | MA MS MY OB OR PG SF TI |
| TABLE | `TABLE_REQUIRE_CHARSET` | Require charset | MY |
| TABLE | `TABLE_REQUIRE_COLLATION` | Require collation | MY |
| TABLE | `TABLE_REQUIRE_PK` | Enforce inclusion of primary key in a table | MA MS MY OB OR PG SF TI |
| TABLE | `TABLE_TEXT_FIELDS_TOTAL_LENGTH` | Restrict the total length of text fields in a table | MY |

Counts: 105 rule types; per category COLUMN 22 (incl. the Oracle-only `COLUMN_ADD_NOT_NULL_REQUIRE_DEFAULT` that is not in the UI schema), STATEMENT 34, TABLE 14, NAMING 11, INDEX 9, SYSTEM 8, BUILTIN 3, and one each for ENGINE, DATABASE, SCHEMA, ADVICE. Per engine (registered advisors): MySQL 82, MariaDB 59, OceanBase 57, PostgreSQL 54, TiDB 51, Oracle 25, SQL Server 23, Snowflake 14, BigQuery 1, Redshift 1. [Observed: register-call counts per directory]

Observations on the catalog that matter for Hacivat:

- **Most rules are migration hygiene, not access control.** Only a handful are relevant to *interactive* safety: `STATEMENT_WHERE_REQUIRE_UPDATE_DELETE`, `STATEMENT_AFFECTED_ROW_LIMIT`, `STATEMENT_DISALLOW_TRUNCATE`, `STATEMENT_DML_DRY_RUN`, `STATEMENT_INSERT_ROW_LIMIT`, `STATEMENT_DISALLOW_RM_TBL_CASCADE`, `DATABASE_DROP_EMPTY_DATABASE`, `TABLE_DISALLOW_DDL/DML` (per-table write freeze), `STATEMENT_MAX_EXECUTION_TIME`, `STATEMENT_NON_TRANSACTIONAL` (PG statements that cannot run in a transaction, e.g. `CREATE INDEX CONCURRENTLY`), `STATEMENT_WHERE_REQUIRE_SELECT`, `STATEMENT_SELECT_NO_SELECT_ALL`, `STATEMENT_DISALLOW_CROSS_DB_QUERIES`.
- **Some rules need a live connection** (`STATEMENT_DML_DRY_RUN`, `STATEMENT_AFFECTED_ROW_LIMIT` via EXPLAIN estimates, `TABLE_LIMIT_SIZE`, `ADVICE_ONLINE_MIGRATION` via row counts). In Hacivat's architecture only Karagöz can run those. [Inferred: from `Context.Driver` field and rule names]
- **PostgreSQL-specific online-DDL safety rules** (`INDEX_CREATE_CONCURRENTLY`, `STATEMENT_ADD_CHECK_NOT_VALID`, `STATEMENT_ADD_FOREIGN_KEY_NOT_VALID`, `STATEMENT_DISALLOW_ADD_NOT_NULL`, `COLUMN_DEFAULT_DISALLOW_VOLATILE`, `STATEMENT_DISALLOW_ADD_COLUMN_WITH_DEFAULT`) mirror the well-known lock-avoidance guidance; MySQL equivalents are `STATEMENT_REQUIRE_ALGORITHM_OPTION` / `STATEMENT_REQUIRE_LOCK_OPTION` and `ADVICE_ONLINE_MIGRATION` (gh-ost).
- `SCHEMA_BACKWARD_COMPATIBILITY` (drop/rename column/table, type narrowing) is the rule most relevant to "AI agent proposes a migration" flows.

### 3.5 Change workflow and approvals

#### 3.5.1 Lifecycle

```
Plan (specs: create/change database, sheet or release)  ──plan checks (SQL review, statement summary: types, affected rows, changed tables)──►
Issue (DATABASE_CHANGE, OPEN, draft→submitted)  ──approval finding (async runner)──►  approval template attached + risk level
   ──approvers act in role order──►  approved
Rollout (tasks per database, grouped by environment stage)  ──rollout policy: automatic or roles allowed to run──►  TaskRuns (PENDING→RUNNING→DONE/FAILED/CANCELED/SKIPPED)
Issue → DONE
```

[Observed: `component/review/{evaluator.go,approval.go,rollout.go,workflow.go}`, `proto/store/store/{plan.proto,issue.proto,task_run.proto,policy.proto RolloutPolicy{automatic, roles}}`]

- Approval finding runs in a background `Runner` fed by a bus of issue refs; it may return `done=false` ("plan checks not finished, retry later"). [Observed: `evaluator.go:39-160`, `buildCELVariablesForIssue` comment]
- Project settings governing the workflow: `require_issue_approval` (rollout creation blocked until approved), `allow_self_approval`, `allow_last_plan_editor_approval`, `require_plan_check_no_error`, `enforce_sql_review`, `allow_request_role`, `allow_just_in_time_access`, `parallel_tasks_per_rollout`, `execution_retry_policy.maximum_retries`, `force_issue_labels`, `enforce_issue_title`. [Observed: `proto/store/store/project.proto`]
- Rollout creation (`Workflow.CreateRollout`) re-checks: draft issues refused; if approval required, the issue must be approved *for this plan version*; then within a DB transaction it takes a Postgres advisory lock keyed on (project, plan), locks the issue row, re-verifies the observed approval and writes tasks atomically. [Observed: `component/review/rollout.go`]

#### 3.5.2 Approval rules ("custom approval")

- Workspace setting `WorkspaceApprovalSetting.rules[]`, each `{template, condition (CEL), source}` with `source ∈ {CHANGE_DATABASE, CREATE_DATABASE, EXPORT_DATA (deprecated), REQUEST_ROLE, REQUEST_ACCESS, SOURCE_UNSPECIFIED (fallback)}`. [Observed: `proto/store/store/setting.proto:223-238`]
- A template is `{title, description, flow: {roles: [ordered role names]}}` — each step is a **role**, and any holder of that role (via workspace or project IAM) may act for that step. [Observed: `approval.proto ApprovalTemplate/ApprovalFlow`; `workflow.go canReview`]
- Matching (`getApprovalTemplate`): Phase 1 iterates rules **in order**, considering only rules with the issue's source; an empty condition is *skipped* (not a catch-all); the literal `"true"` matches immediately; otherwise compile with the full `ApprovalFactors` environment and evaluate with **partial evaluation** against *each* CEL variable map (one map per task/statement-type/table combination — see below); the first rule for which *any* map yields `true` wins. Evaluation errors on one map are ignored (continue). Phase 2 repeats with `SOURCE_UNSPECIFIED` rules using only `resource.project_id`. No match ⇒ no approval required. [Observed: `evaluator.go:233-312`]
- CEL variables for approval (`ApprovalFactors`): resource (`environment_id, project_id, instance_id, db_engine, database_name, schema_name, table_name`), statement (`affected_rows, table_rows, sql_type, text`), request (`expiration_days, role, unmask, data_export`), `risk.level` (string LOW/MODERATE/HIGH), `issue.labels` (list, only for CHANGE_DATABASE rules — validation rejects it elsewhere). [Observed: `common/cel.go:18-48`, `ValidateApprovalExprForSource`]
- CEL vars are **expanded per statement type × changed table** (`expandCELVars`), so a rule like `statement.sql_type == "DROP_TABLE" && resource.environment_id == "prod"` matches if *any* statement in *any* task satisfies it. [Observed: `evaluator.go:1189-1230` function names and comments]
- **Risk level** is computed in code from statement types (§2 item 11) and injected; "risk layer removal (3.13)" means the older configurable risk rules were removed in favour of this fixed classification plus CEL on `risk.level`. [Observed: `risk.go`; comment in `common/cel.go:16-17`]
- Approval feature gate: when `FEATURE_APPROVAL_WORKFLOW` is unavailable the finder *intentionally* returns "no template" — i.e. on FREE/TEAM nothing requires approval. [Observed: `finding.go evaluateApprovalTemplateForIssue`; `review/finding.go:80`]

#### 3.5.3 Review actions and state

- Approval state lives in `IssuePayloadApproval{approval_template, approvers[{status PENDING/APPROVED/REJECTED, principal}], approval_finding_done, approval_input_version}`. [Observed: `approval.proto`]
- `Approve`/`Reject`: refused if any step is already rejected; the *next pending role* is the step's role; the actor must hold it; self-approval refused unless the project allows it; on approve, refused if actor is the **last plan editor** unless allowed; append `{status, principal}`; emit comment + "approval requested" (to notify next step) or "sent back". [Observed: `workflow.go:340-405`]
- `Request` (re-request after rejection): only the creator; removes REJECTED entries and re-emits "approval requested". [Observed: `workflow.go:390-410`]
- **Invalidation:** the plan carries `approval_input_version`, bumped when approval-relevant content changes; the issue's approval is re-found when the stored `approval_input_version` differs. Applying a new finding is an optimistic, lock-protected compare-and-swap: abort with "conflict" if the issue's approval, labels (when labels matter), draft flag, type, plan UID, plan approval version or plan-check generation changed between read and write; refuse if a rollout already started under `require_issue_approval`. [Observed: `approval.go:50-200`]
- Notifications to approvers go through webhooks/IM (Slack, Feishu, WeCom, Lark, DingTalk…). [Observed: `setting.proto AppIMSetting`; `NotifyApprovalRequested`]

### 3.6 SQL editor access control

#### 3.6.1 Query path (`SQLService.Query`)

Ordered steps (prose summary of `sql_service.go:300-512`):

1. Resolve user, instance, database from the request name; refuse deactivated users.
2. **Pre-match a JIT access grant** (§3.6.5) if the project allows JIT.
3. For engines **without** per-statement ACL: run the engine's read-only validator; refuse non-read-only statements with `NON_READ_ONLY` command error.
4. For EXPLAIN requests: refuse EXPLAIN-ANALYZE-style explains whose wrapped statement is not read-only (because they execute); validate explain format per engine.
5. **MCP clamp** (AI agents, §3.9): if the session is clamped to read-only, refuse non-read statements; if the session may write but the statement is a write, require explicit confirmation and route to a write-capable data source.
6. Compute the **effective query-data policy** (project + workspace): max result rows, max result size, timeout, allow-admin-data-source, disable-copy.
7. **Choose a data source**: if the client named one, use it; else if admin data source is allowed *and* the statement needs admin (validator says not read-only, or — for Mongo/Elasticsearch — query span is DML/DDL) → admin; else exactly one read-only data source → it; >1 read-only → error "specify data_source_id"; none → admin; no admin → error.
8. Check the chosen data source is queryable: non-admin data sources require `FEATURE_INSTANCE_READ_ONLY_CONNECTION` on an activated instance; admin data source is refused when the query policy disallows admin *and* a read-only data source exists. (Without `FEATURE_QUERY_POLICY` the admin data source is simply allowed.)
9. Open a driver with `ReadOnly = clamped || dataSource.type == READ_ONLY`.
10. Execute with retry/stop-on-error, passing the **per-statement access check** (§3.6.3) and masking (`SkipMasking` only when a matched JIT grant has `unmask=true`).
11. Record query history (statement, duration, error) and return results; permission-denied details list the resources and required permission.

[Observed: `sql_service.go:300-512, 2459-2586`]

#### 3.6.2 Read-only enforcement per engine — the important finding

| Layer | Engines | Mechanism |
|---|---|---|
| Session-level read-only | PostgreSQL, CockroachDB, Redshift (not datashares) | Connection runtime parameter `default_transaction_read_only=true` when `ConnectionContext.ReadOnly`. |
| Statement validator (whitelist) | All engines with a registered validator: BigQuery, Cassandra, ClickHouse, CockroachDB, CosmosDB, Doris, DynamoDB, Hive, MariaDB, SQL Server, MySQL, OceanBase, Oracle, PostgreSQL, Redis, Redshift, Snowflake, Spanner, StarRocks, TiDB, Trino | Parse, accept only read statement kinds. PG: `SELECT` (unless `SELECT … INTO` or a data-modifying CTE — whitelist: a CTE body is read-only only if it is itself a SELECT node), `EXPLAIN` (EXPLAIN ANALYZE only of read-only SELECT), `SHOW`, and `SET` (flagged); everything else refused. Redis: an explicit allow-map of command names that Redis itself flags read-only, kept in sync by a test against a `COMMAND INFO` snapshot. |
| Query-span classification | MongoDB, Elasticsearch | Validator plus span type must be Select/SelectInfoSchema/Explain. |
| Credential-level | any | A `READ_ONLY` data source configured by the admin with a DB user that only has read grants. Gated `FEATURE_INSTANCE_READ_ONLY_CONNECTION` (TEAM+). |

[Observed: `plugin/db/driver.go:103-107` comment "Three honor it … Every other driver ignores it"; `plugin/db/pg/pg.go:99-101`; `plugin/db/redshift/redshift.go:121-123`; `plugin/parser/pg/query_antlr.go:19-100`; `plugin/parser/redis/query.go` header comment; `query_statement_classification.go shouldClassifyStatementByQuerySpan`]

Weaknesses worth recording for our own design:

- PostgreSQL's `default_transaction_read_only` is a *default*: a session may `SET default_transaction_read_only = off` or `SET TRANSACTION READ WRITE`. The PG validator *accepts* `SET` statements (it only marks them). Combined with a SELECT that calls a writing function (`SELECT my_func_that_inserts()`, `SELECT nextval(...)`, `SELECT pg_terminate_backend(...)`, `SELECT dblink_exec(...)`), the parser would see "SELECT" and the session-level guard could have been switched off by an earlier `SET` in the same request. [Inferred: from validator accepting `VariableSetStmt` and PG semantics; **not** reproduced against a running Bytebase — treat as an open question to verify, not a confirmed vulnerability]
- MySQL/MariaDB/SQL Server/Oracle/Mongo/Redis get *no* session-level guard; MySQL could use `START TRANSACTION READ ONLY` or `SET SESSION TRANSACTION READ ONLY`, SQL Server `ApplicationIntent=ReadOnly` (only meaningful on AG secondaries) — Bytebase does not. Real safety on these engines depends on the admin creating a genuinely read-only DB user. [Observed: driver comment] [Inferred: alternatives]
- **Export on MySQL skips the read-only validator** ("New query ACL experience. if engine != MYSQL …"); the code comment itself says "The validation below admits a write on MySQL and on an engine with no validator, and the driver runs it", relying on the per-statement ACL to require `bb.sql.dml`/`ddl`. [Observed: `sql_service.go Export` ~line 1245-1260]
- `disable_download` (hide download destination) is enforced **only in the UI** — "The Export API does not check this setting." [Observed: `policy.proto QueryDataPolicy.disable_download` comment; `sql_service_export.go decideExportDestination` comment]

#### 3.6.3 Per-statement ACL ("new query ACL")

- For new-ACL engines (MySQL, PostgreSQL, Oracle, SQL Server, TiDB, Snowflake, Spanner, BigQuery, MongoDB, Elasticsearch) each non-empty statement gets a **query span** (statement type + set of referenced databases/schemas/tables + result-column lineage + predicate columns). [Observed: `common/engine.go:87-110`; `sql_service.go:1542-1700`]
- Span type → required permission: `DDL → bb.sql.ddl`, `DML → bb.sql.dml`, `Explain → bb.sql.explain`, `SelectInfoSchema → bb.sql.info`, `Select → bb.sql.select`, unknown → fallback `bb.sql.select`; in export mode `select`/`info` become `bb.sql.export`. [Observed]
- For **each referenced database** the user's bindings (workspace + project) are evaluated with CEL attributes `{request.time, resource.database, resource.environment_id (DDL/DML only), schema/table when known}`. A JIT grant can satisfy the permission *only for its own target databases* and never satisfies `bb.sql.explain`. [Observed: `accessCheckWithGrantedTargets` comments "grantSatisfies…"]
- Cross-database and linked-server references (SQL Server linked servers, Oracle DB links, Postgres dblink-like) are resolved; references to targets outside the request's project, or private links the user does not own, are refused. [Observed: function names `refuseLinkedReadsShadowedByPrivateLinks`, `refuseLinkedTargetsOutsideProject`, `remoteColumnRefusal`]
- Engines with no span extractor fall back to a database-level check of `bb.sql.select` (or `explain`). [Observed: comment "No spans: an engine without a span extractor"]

#### 3.6.4 Admin mode and query-data policy

- **Admin mode** = `AdminExecute`, a bidirectional streaming RPC requiring `bb.sql.admin`, executing on the admin data source without the read-only validator; gated `FEATURE_SQL_EDITOR_ADMIN_MODE` (present in all plans). It is audited like other calls; since it is a stream, the audit interceptor builds its own entry. [Observed: `sql_service.go:76`; `audit_redact.go` comment about AdminExecute]
- **QueryDataPolicy** (project and workspace): `maximum_result_rows`, `disable_copy_data` (workspace; Enterprise `FEATURE_RESTRICT_COPYING_DATA`), `allow_admin_data_source` (workspace), `disable_download` (UI-only), deprecated `disallow_ddl`/`disallow_dml`; plus effective timeout and maximum result size. Setting the policy needs `FEATURE_QUERY_POLICY` (TEAM+). [Observed: `policy.proto QueryDataPolicy`; `org_policy_service.go:559,691,695`]
- **Watermark** (Enterprise) overlays user identity on the SQL editor to deter screenshots. [Observed: `plan.yaml FEATURE_WATERMARK`; `setting_service.go:1165`]

#### 3.6.5 Just-in-time access grants (Enterprise `FEATURE_JIT`)

- Create (`CreateAccessGrant`): requires project setting `allow_just_in_time_access`; validates targets (`instances/{i}/databases/{d}`, all in one instance and project); requires non-empty query and reason; the query must classify as **read-only** for the engine; creator is the authenticated caller (client-supplied creator ignored); expiration is either an absolute `expire_time` or a `ttl` (stored as `requested_duration`, converted to an absolute expiry **at activation**); both capped by the workspace `maximum_request_expiration`; payload `{targets, query, unmask, export, reason, requested_duration, schema, container}`; approval is evaluated **once at creation** with source `REQUEST_ACCESS` (so later rule edits do not change it); an `ACCESS_GRANT` issue is created and the workflow started; if no approval needed the issue completes immediately and the grant becomes ACTIVE. MCP sessions cannot create grants that would wait for a human ("unattended"). [Observed: `access_grant_service.go:128-300`]
- Store derives `query_hash` = lowercase hex SHA-256 of the query after trimming boundary whitespace (space, tab, LF, CR, VT, FF). [Observed: `access_grant.proto` field 10 comment]
- Use (`preCheckAccess`): on Query/Export, list ACTIVE grants for (project, creator=current user, target database, statement, schema, container, requireExport, not expired); pick the best (prefer `unmask=true`); re-verify the statement is read-only. A matched grant (a) satisfies the permission on its targets, (b) optionally disables masking, (c) is echoed back as `applied_access_grant`. [Observed: `sql_service.go:201-299`]
- Revoke/activate RPCs exist; `activate` can refresh expiry. [Observed: `access_grant_service.go:362-445`]

This "approve one exact statement for a limited time" primitive is the most transferable idea for Hacivat's AI-agent and break-glass flows (see §5).

#### 3.6.6 Request-role workflow (Enterprise `FEATURE_REQUEST_ROLE_WORKFLOW`)

- A user requests a project role (e.g. `sqlEditorUser`, optionally with a CEL condition limiting databases and an expiry); this becomes a `ROLE_GRANT` issue with approval source `REQUEST_ROLE`; CEL vars include `request.role`, `request.expiration_days`, and resource attributes of the requested databases; on approval, a binding is added to the project IAM policy with the condition (including `request.time < timestamp(...)`). [Observed: `issue_service.go:636`; `evaluator.go buildCELVariablesForRoleGrant`; IAM expiry handling in `utils.ValidateIAMBinding`] [Inferred: binding shape from proto + validate function name]

### 3.7 Data masking

#### 3.7.1 Configuration objects

| Object | Where | Shape |
|---|---|---|
| Semantic type | Workspace setting `SemanticTypeSetting.types[]` | `{id (uuid), title, description, icon, algorithm}`; two **builtins always appended**: `Default` (full mask `******`) and `Default Partial` (range mask with no slices ⇒ "middle" algorithm). Stored copies of builtin IDs are dropped and replaced. |
| Masking algorithm | `Algorithm` oneof | `FullMask{substitution ≤16 bytes}`; `RangeMask{slices[{start,end (character indices, end-exclusive), substitution}]}`; `MD5Mask{salt}`; `InnerOuterMask{prefix_len, suffix_len, type INNER/OUTER, substitution}`. |
| Data classification | Workspace setting `DataClassificationSetting.configs[]` | `{id, title, levels[{title, level int (higher = more sensitive)}], classification map id("1-2-3") → {title, level?}}`; a project selects one config (`data_classification_config_id`). |
| Column catalog | Per database | Each column may carry `semantic_type` (id) and `classification` (id). |
| Masking rule policy | Workspace policy `MASKING_RULE` | Ordered `rules[{id, condition (CEL), semantic_type}]`. CEL vars: `resource.environment_id, project_id, instance_id, database_name, schema_name, table_name, column_name, classification_level (int)`. |
| Masking exemption policy | Project policy `MASKING_EXEMPTION` | `exemptions[{members[users/…, groups/…], condition}]`; condition restricted to `==` on `resource.instance_id/database_name/schema_name/table_name/column_name` joined by `&&`, plus `request.time < timestamp(...)`; empty condition = all databases forever. |

[Observed: `proto/store/store/setting.proto:240-342`, `policy.proto MaskingExemptionPolicy/MaskingRulePolicy`, `component/masker/semantic_type.go`, `common/cel.go:111-136`]

Feature gates: `FEATURE_DATA_MASKING` (Enterprise, *instance-level*: needs an activated instance) for masking, `FEATURE_DATA_CLASSIFICATION` for classification. [Observed: `plan.yaml instanceFeatures`; `org_policy_service.go:717,727`; `setting_service.go:284`; `sql_service.go:873`]

#### 3.7.2 Algorithm semantics (observed behaviour)

- **Full mask**: replaces every value — including NULL — with the substitution (default `******`). It is the only masker that hides *every* value. [Observed: `masker.go FullMasker`, `MasksEveryValue`]
- **Default partial ("middle")**: for strings, keeps a middle slice of runes and pads with `**` on both sides: length 0–1 → empty middle; 2–3 → the single middle char; otherwise truncate to a multiple of 4 and keep the second and third quarters. Numbers are formatted to decimal text and middled; timestamps formatted (`2006-01-02 15:04:05.000000` or RFC3339Nano) and middled; booleans and NULL → `******`; bytes keep middle bytes with `**` padding; nested JSON values masked recursively. **This leaks half of the value** — intended for display, not for secrecy. [Observed: `masker.go DefaultRangeMasker`, `middle`]
- **Range mask**: replace character ranges `[start,end)` with substitution; indices are character (rune) indices, out-of-range values clamped. [Observed: `setting.proto RangeMask` comments; `masker.go formatRanges/getValidRangeValue`]
- **MD5 mask**: hex MD5 of `text(value) + salt`. With an empty salt this is reversible for low-entropy domains by dictionary (phone numbers, national IDs). **Quirk:** for `timestamptz` values the MD5 masker returns the *middle* of the formatted timestamp rather than a hash. [Observed: `masker.go:510-583`] [Inferred: dictionary risk]
- **Inner/Outer**: keep or mask `prefix_len`/`suffix_len` margins (INNER masks the middle, OUTER masks the margins); NULL and booleans replaced regardless. [Observed: `masker.go InnerOuterMasker`]

#### 3.7.3 Decision procedure per column (summary of `masking_evaluator.go`)

1. Compute the column's classification level from its classification id within the project's classification config (unknown ⇒ 0/unset).
2. Iterate global masking rules **in order**; the first whose CEL condition is true decides the semantic type (record rule id/title as the "reason"). If the semantic type id is unknown, keep the id with no algorithm (later ⇒ no masker).
3. If no global rule matched, use the column's own catalog `semantic_type`, if any.
4. If a semantic type was found, evaluate the user's **exemptions** (only exemptions whose members include the user directly or via group) with CEL over the column coordinates, classification level and current time; any true ⇒ **not masked**.
5. Return the evaluation (semantic type, algorithm name, human-readable context such as "Global masking rule: …" or "Column-level semantic type: inst.db.schema.table.col", classification level).

Precedence therefore is: **global rule > column-level semantic type**, and **exemption beats both**. Note the global rule is first-match, so rule order is security-relevant. [Observed: `masking_evaluator.go:75-200`]

#### 3.7.4 Where masking is applied — result path

After execution, *inside the Bytebase server*, `QueryResultMasker.MaskResults` runs for each (statement span, result) pair:

1. Skip EXPLAIN results (plans contain no row data).
2. If the span reports an unsupported function or a not-found object and the result has no error ⇒ refuse ("masking error").
3. If the engine supports masking and the span has **unresolved columns** ⇒ refuse the whole query ("masking cannot be applied … the query was not returned"); this also triggers a metadata re-sync so stale catalogs self-heal.
4. If the result is an error with no rows and the span touches any masked column (result lineage **or predicate columns**) ⇒ replace the DB error text with a generic message and drop structured error details (DB errors can embed values, e.g. invalid input syntax).
5. If the result's column count differs from the span's resolved column count and masked columns are involved ⇒ refuse (position-based masking would misalign).
6. Build one masker per output column from lineage: 0 source columns (constants) ⇒ none; exactly 1 effective masker and the column is a *plain field* ⇒ that masker; exactly 1 but through an *expression* (on engines that track plain-field-ness) ⇒ default full mask with reason "Expression-based column"; ≥2 effective maskers ⇒ default full mask with reason "Multiple source columns".
7. Mask every non-nil value in place; attach per-column `MaskingReason` (semantic type, rule id, algorithm, context, classification level) so the UI can show *why* a column is masked.

[Observed: `query_result_masker.go:30-120, 300-420, 470-533`]

Additional defences:

- **Predicate-column refusal**: on SQL Server, PostgreSQL, MySQL, MariaDB, a statement whose WHERE/JOIN conditions read a column that is masked *for this user* is refused ("Using sensitive columns in query conditions is not allowed: …") because row counts, timing and result sets would otherwise leak the hidden value (binary-search attack). Exemptions are applied identically on this path so a user exempted from masking is not refused. [Observed: `predicate_columns_check.go:15-40, 149-156`; `exemptionsForPrincipal` comment]
- **Document stores** (MongoDB, CosmosDB, Elasticsearch) use a separate field-path masking implementation (`document_masking.go`, 1385 lines). [Observed]
- Engines supporting masking: MySQL, PostgreSQL, Oracle, SQL Server, MariaDB, OceanBase, TiDB, BigQuery, Spanner, Redshift, Cassandra, Trino, Snowflake; **not** Redis, ClickHouse, StarRocks, Hive, CockroachDB (and Mongo via the document path). [Observed: `common/engine.go:124-150`]
- Masking is skipped only by (a) an exemption or (b) a JIT access grant with `unmask=true`. Admin mode (`AdminExecute`) — [Unknown: whether admin-mode results are masked; not traced].

#### 3.7.5 Masking × AI agents

`maskedWriteRefusal` refuses any MCP-originated SQL containing the literal `******`: an agent that reads a masked value and writes it back (e.g. "copy this row") would silently destroy real data. Only the full-mask default sentinel is detectable; MD5/range/inner-outer outputs are indistinguishable from real data, so Bytebase deliberately accepts false positives (any six asterisks anywhere in the text, including comments). [Observed: `mcp_masking.go:18-70` comment]

### 3.8 Audit log

- **Capture:** a ConnectRPC interceptor (`AuditInterceptor`, unary and streaming) writes one row per call to an RPC whose method option `(bytebase.v1.audit) = true` (117 RPCs), *after* the handler, under the workspace and, where applicable, the project (rows can be filed under multiple parents). Calls refused by the permission check before reaching the handler are streamed to stdout (when enabled) but **not stored**. [Observed: `audit.go:32-60, 205-440`]
- **Row schema** (`AuditLog`): `name` (`projects/{p}/auditLogs/{uid}` or `workspaces/{w}/auditLogs/{uid}`), `create_time`, `actor` (`users/…`, `serviceAccounts/…`, `workloadIdentities/…`), `method` (e.g. `/bytebase.v1.SQLService/Query`), `severity` (syslog-style DEBUG…EMERGENCY; permission-denied refusals marked WARNING), `resource`, `request` (JSON), `response` (JSON, may be trimmed), `status` (`google.rpc.Status`), `latency`, `service_data` (Any; e.g. IAM `PolicyDelta`), `request_metadata{caller_ip, caller_supplied_user_agent}`, `mcp_delegation{scope, resource, client_id, correlation_id}`, `federated_identity{issuer, subject}`. [Observed: `proto/v1/v1/audit_log_service.proto:160-260`]
- For SQL editor queries the *request* contains the statement text; result rows are not in the audit row (responses are redacted/omitted by annotation). Query history is a separate table (`createQueryHistory`: statement, user, duration, error). [Observed: `sql_service.go:1456`] [Inferred: result rows excluded based on OMIT annotations; exact fields not enumerated]
- **Redaction:** field option `audit_behavior`: `UNSPECIFIED` (logged), `SENSITIVE` (credential: never logged; API may return it only on the minting response), `OMIT` (unbounded bodies, base64 blobs, bearer capabilities, personal data). Applied recursively on request and response; `Any` fields are only kept if their packed type is registered for that field, otherwise dropped (a protojson failure would otherwise lose the whole row). A CI "inventory" test forces every new field to be classified. It is a *denylist* with a build-time completeness check. [Observed: `audit_redact.go:16-60`, `annotation.proto:40-60`]
- **Visibility gating:** `SearchAuditLogs`/`ExportAuditLogs` require `FEATURE_AUDIT_LOG` (TEAM+). Retention: FREE 0 (no access), TEAM 7 days, ENTERPRISE unlimited — implemented as a *filter on search*, so older rows still exist in the DB. Page size max 5000; the first page pins the DB clock as an upper bound so paging terminates even though the search itself writes an audit row. [Observed: `audit_log_service.go:36-100`, `license.go GetAuditLogRetentionDays/Cutoff`]
- **Streaming to SIEM:** only via **stdout** (`EnableAuditLogStdout` workspace profile flag AND valid audit-log entitlement; not in SaaS). No native syslog/Kafka/S3 sink. [Observed: `server.go:160-180`, `setting_service.go:980-1000`]
- **No tamper evidence:** rows live in Bytebase's Postgres metadata DB; no hash chain or signatures were found. [Inferred: absence in `audit.go`/store; grep found no hash/chain logic]

### 3.9 AI agent / MCP controls (relevant to Hacivat's local MCP server)

- Bytebase ships an MCP server that calls the same v1 API with a *delegated credential* (OAuth2 grant with scope such as `mcp:read-only`). Every call carries `mcp_delegation` provenance in the audit row and a session-scoped `correlation_id`. [Observed: `audit_log_service.proto MCPDelegation`]
- **Capability ceiling** per workspace (`mcpServingClasses`): which method classes (read/write/…) each stored MCP capability serves; DISABLED serves nothing; a *missing* key is treated as "nobody decided" and is an error, not a silent denial. [Observed: `mcp_gate.go` comments]
- **Read-only clamp:** because `SQLService/Query` is classified READ but can execute DML/DDL on new-ACL engines, the clamp inspects the statement and refuses non-reads, *and* opens the DB session read-only regardless of which data source was chosen. Bytebase documents this as "classifier-enforced, not proven". [Observed: `mcp_sql_clamp.go` comments; `sql_service.go:338-394`]
- **Write confirmation:** MCP writes require an explicit confirmation step; MCP cannot EXPLAIN or EXPORT statements it cannot prove are reads; MCP cannot create JIT grants that need a human approver. [Observed: `sql_service.go:358-382`, `access_grant_service.go refuseUnattendedMCPAccessGrant`]

### 3.10 Feature gating implementation

#### 3.10.1 Plan matrix

`backend/enterprise/plan.yaml` (embedded with `go:embed`, parsed YAML → JSON → protobuf `PlanConfig` at init; panics on malformed config):

| Plan | maximumInstanceCount | maximumSeatCount | Features added over the previous plan |
|---|---|---|---|
| FREE | 10 | 20 | Change management (all 13 incl. online schema change, SQL review, backup/rollback, batch changes, progressive rollout, changelog, rollout policy), SQL editor (admin mode, NL→SQL, AI explain/suggest, autocomplete, schema diagram/editor, data export, query history, saved scripts), IAM, SSL, SSH tunnel, cloud IAM auth, custom sync time, custom connection limit, environments, IM notifications, Terraform provider, community support |
| TEAM | 10 | −1 (unlimited) | Batch query, read-only connection routing, query policy, Google+GitHub SSO, user groups, disallow self-service signup, audit log (limited), database groups, email support |
| ENTERPRISE | −1 | −1 | Restrict copying data, SharePoint export, risk assessment, approval workflow, full audit log, enterprise SSO, 2FA, password restrictions, custom roles, request-role workflow, JIT, data masking, data classification, SCIM, directory sync, token-duration control, disallow password sign-in, external secret manager, user email-domain restriction, environment tiers, dashboard announcement, API integration guidance, custom logo, watermark, roadmap prioritization, custom MSA, dedicated support with SLA |

`instanceFeatures`: `FEATURE_DATA_MASKING`, `FEATURE_INSTANCE_READ_ONLY_CONNECTION`, `FEATURE_EXTERNAL_SECRET_MANAGER` — these additionally require the *instance* to be activated under the license. [Observed: `backend/enterprise/plan.yaml`]

#### 3.10.2 License verification (prose)

- Keys: `keys/prod.pub.pem` and `keys/dev.pub.pem` embedded; release mode selects one. The private key exists only in Bytebase's SaaS (used to sign customer licenses and trials), optionally provided base64-encoded through configuration. [Observed: `license.go NewConfig/NewLicenseService`]
- Token: JWT signed **RS256** with header `kid` = `v1`; claims: `instanceCount` (activated instances), `instance` (instance limit), `seat`, `ha`, `trialing`, `plan` (FREE/TEAM/ENTERPRISE), `orgName`, `workspaceId`, plus registered claims (`iss`=`bytebase`, `aud` contains `bb.license`, `sub` = workspace id, `iat`, optional `exp`, optional `nbf`). [Observed: `license.go Claims`, `CreateLicense`]
- Parse: reject non-RSA algorithms (prevents `alg=none`/HMAC confusion); reject wrong `kid`; verify signature; check `nbf`; check `iss`, `aud`; plan string must be a known enum; **enterprise non-trial licenses are bound to the workspace id** (mismatch ⇒ invalid; trial and TEAM licenses are not bound). Expiry is checked *separately*: an expired license is stored/parsed fine but the effective subscription becomes FREE while preserving `ExpiresTime` (so the UI can say "expired on …"). Uploading an already-expired license is refused. [Observed: `license.go parseLicenseUncheckedExpiry, validateLicense, LoadEffectiveSubscription`]
- Storage: the license string lives in the workspace's system setting in the metadata DB; an LRU (128 entries, 1-minute TTL) plus singleflight caches the parsed subscription per workspace; any DB error or parse error ⇒ FREE ("fail to free", i.e. fail-closed for paid features, fail-open for core). [Observed]
- Limits: seat limit = license `seat` if >0 else plan default (FREE 20; paid unlimited); instance limit = license `instance` if >0 else plan default (−1 ⇒ unlimited). If instance limit ≤ activated-instance limit the license is "unified" and every instance counts as activated. [Observed: `userLimitFromSubscription`, `GetInstanceLimit`, `isUnifiedInstanceLimit`]
- **HA gate:** replicas heartbeat into the metadata DB every ~10 s; the server counts replicas active within 30 s (cached); more than one replica without `ha=true` in the license ⇒ startup/periodic error instructing to scale to 1 or buy HA. Skipped in SaaS. [Observed: `CountActiveReplicas`, `CheckReplicaLimit`]
- **Trial:** self-service 14-day ENTERPRISE trial, 20 seats, 10 instances, bound to the workspace, `trialing=true`, minted by the SaaS-side signer. [Observed: `backend/api/v1/trial.go:13-27`]
- **Gate call shape:** `IsFeatureEnabled(ctx, workspaceID, feature)` returns `nil` or an error "feature X is a TEAM/ENTERPRISE feature, please upgrade to access it" (minimal plan derived from the matrix); handlers wrap it as `PermissionDenied`. `IsFeatureEnabledForInstance` additionally requires an activated instance on paid plans; on FREE it degenerates to the plain check. [Observed: `license.go IsFeatureEnabled*`]
- **Frontend parity:** `frontend/scripts/copy-config-files.sh` copies `backend/enterprise/plan.yaml` to `frontend/src/types/plan.yaml` at build; `frontend/src/types/plan.ts` builds the same matrix, computes the minimum required plan for upgrade prompts, and exposes `hasFeature`/`hasInstanceFeature`; components such as `FeatureBadge`, `FeatureAttention` render upgrade nudges. [Observed]
- **Some gates degrade rather than refuse**: approval workflow unavailable ⇒ no approval required (§3.5.2); query policy unavailable ⇒ admin data source allowed; audit stdout requires both the setting and the entitlement. [Observed]

#### 3.10.3 Pricing tiers (public)

| Public name | Code plan | Price | Limits | Notes |
|---|---|---|---|---|
| Community | FREE | Free | 10 instances, 20 users | Self-host or cloud. |
| Pro | TEAM | $20 per user per month (monthly or yearly), self-service via Stripe | 10 instances, unlimited users | `pricing/plan.go`: `PricePerSeatPerMonth: 2000` cents, `InstanceCount: 10`, `SelfServicePurchase: true`. |
| Enterprise | ENTERPRISE | Contact sales, yearly | Custom | 14-day trial; HA, onboarding, custom MSA, HIPAA BAA as add-ons; wire/AWS/GCP marketplace. |

[Observed: bytebase.com/pricing fetched 2026-11-11; `backend/enterprise/pricing/plan.go`] The marketing name "Pro" corresponds to code plan `TEAM`. The sso.tax entry (dated 2024-02-12) listed Bytebase at "$995/year" base with SSO "Call us" — out of date relative to today's Pro tier which includes Google/GitHub (but not OIDC/SAML/LDAP) SSO. [Observed: sso.tax]

---

## 4. Algorithm specifications (Bytebase-derived, for reference)

These ALGs describe **observed Bytebase behaviour** in our own words, as reference designs. The canonical Hacivat/Karagöz/Perde algorithms live in:
- document 04 (`04-zero-credential-discovery-and-ephemeral-users.md`) — credential resolution, ephemeral users;
- document 05 (`05-authorization-and-audit-primitives.md`) — permission evaluation at the agent, query authorization, audit chaining, access-request state machine, result-stream masking.

Where an ALG below is a Perde/Karagöz adaptation rather than a pure description, it says so.

### ALG-BYTEBASE-01: Conditioned role-binding permission check

- **Problem:** Decide whether principal P holds permission `perm` on a data resource R (database/schema/table), where roles are granted by bindings carrying CEL conditions, at two policy levels.
- **Inputs / outputs:** P (identity + group memberships), `perm`, R attributes `{environment_id, database_fullname, schema, table}`, current time; workspace policy WP, project policy PP, role→permission map. Output: allow/deny + the matching binding (for audit).
- **Algorithm:**
  1. `bindings ← bindingsFor(P, WP) ∪ bindingsFor(P, PP)` where `bindingsFor` keeps bindings whose members contain P's member string, a group containing P, or `allUsers` (except workspace-level `allUsers` in multi-tenant mode).
  2. Drop bindings whose condition is a pure expiry that has passed (`request.time` bound).
  3. For each remaining binding b in stable order:
     a. If `perm ∉ permissions(b.role)` → continue.
     b. `expr ← b.condition.expression`; if `perm` is a read permission and the deployment uses Bytebase's semantic, environment clauses are ignored (Perde should **not** copy the regex stripping; instead model environment as an attribute that read permissions may legitimately reference).
     c. Evaluate `expr` with attributes `{request.time: now, resource.database: R.db, resource.environment_id: R.env (writes only), resource.schema_name, resource.table_name}`. Compile errors ⇒ log and continue (treat as non-granting).
     d. If true → return allow(b).
  4. Return deny.
  - Project-wide variant: in step 3, skip any binding whose condition references any variable other than `request.time` (detected through the type-checked reference map).
- **Data structures:** policy snapshot cache keyed by (workspace, project) with version; role snapshot cache; group-member snapshot cache; compiled-CEL cache keyed by expression hash.
- **Complexity:** O(#bindings × CEL eval); CEL eval is linear in expression size (size limit 1 MiB in Bytebase — far too large for an agent; Perde should cap at a few KB).
- **Edge cases:** empty condition (= unconditional); malformed CEL (deny-by-skip, never error-to-allow); user deactivated (deny before evaluation); group deleted while binding remains; binding referencing a deleted role (skip); `allUsers` at workspace vs project; multi-project operations (must hold in all); expiry exactly equal to now (`<` ⇒ expired); clock skew between Perde and agent (agent must use its own clock with a skew allowance); resource attributes unknown (schema not synced) — Bytebase passes empty strings, which can make `resource.table_name != "x"` true unexpectedly.
- **Test cases:** (1) sqlEditorReadUser bound with `resource.database == "instances/a/databases/sales"`, query on `sales` → allow; on `hr` → deny. (2) same binding with `request.time < T`, at T+1s → deny. (3) condition `resource.table_name == "orders"`, project-wide permission check → binding skipped → deny. (4) malformed CEL → deny and log. (5) user in group bound to role → allow; group removed → deny after cache refresh.
- **Prior art and evidence:** `backend/component/iam/manager.go`, `condition.go`, `backend/api/v1/sql_service.go hasDatabaseAccessRights`; Google Cloud IAM Conditions (same CEL + binding model).
- **Used by:** Perde (policy authoring/compilation), Karagöz (evaluation — see doc 05 canonical ALG).

### ALG-BYTEBASE-02: SQL review evaluation pipeline

- **Problem:** Produce a bounded, ordered list of advices (ERROR/WARNING) for a SQL script against a configured rule set for one database.
- **Inputs / outputs:** script text, engine, effective rule list (from environment-then-project binding), optional schema metadata and live connection. Output: advices `[ {status, code, title, content, position} ]`, errors first.
- **Algorithm:**
  1. Resolve rule list: environment config if enforced, else project config, else none.
  2. Append builtin rules (size limit; prior-backup check; walk-through) unless the user configured the same (type, engine).
  3. If any size-limit rule for this engine fires (script bytes > limit) → return only that advice.
  4. Split and parse statements; on syntax error return the parser's advice.
  5. If target schema metadata is available: apply DDL to an in-memory copy ("walk-through"); on failure, return a single advice at the walk-through rule's configured level.
  6. For each rule with matching engine: run its checker; on checker failure add an ERROR advice "rule check failed"; bucket advices by status with per-status cap N; stop when both buckets are full.
  7. Return errors ++ warnings.
- **Data structures:** registry `map[engine]map[ruleType]checker`; per-review memo map; rule payload union.
- **Complexity:** O(rules × statements) AST walks; live-connection rules add round trips (dry run, EXPLAIN) — must be time-bounded.
- **Edge cases:** script over size limit (do not parse); mixed engines in one config (filter by engine); rule configured for an engine with no checker (Bytebase errors "unknown advisor" → surfaces as ERROR advice — Hacivat should instead report "unsupported on this engine" as INFO); walk-through on unknown objects; case-sensitive identifiers; checker panic; statements in procedural blocks (PL/pgSQL, T-SQL batches with `GO`); multi-statement transactions; comments/hints; parameter placeholders.
- **Test cases:** (1) `DELETE FROM t;` with `STATEMENT_WHERE_REQUIRE_UPDATE_DELETE` ERROR → 1 error. (2) 50 MB script with size rule → single size advice, parser not invoked. (3) `ALTER TABLE missing ADD c int` with walk-through → single walk-through advice. (4) both buckets capped → evaluation stops early. (5) user overrides builtin size rule to ERROR → only the override applies.
- **Prior art and evidence:** `backend/plugin/advisor/sql_review.go`, `advisor.go`, `builtin_rules.go`; sqruff/sqlfluff lint architecture (see `clients/` docs).
- **Used by:** Hacivat (local lint before execution, offline), Karagöz (authoritative pre-execution check incl. live-connection rules), Perde (rule-set authoring and distribution).

### ALG-BYTEBASE-03: Approval-rule matching and ordered role approval

- **Problem:** For a change/access request, select the approval template and drive approvals to completion with invalidation on content change.
- **Inputs / outputs:** request source; list of CEL var maps (one per task × statement type × table); ordered rules `(source, condition, template)`; actor actions. Output: template (or none), approval state, events.
- **Algorithm (matching):**
  1. If the approval feature is disabled (Bytebase) → no approval. (Perde: never disable for security reasons; see doc 06.)
  2. Compute risk level from statement types (HIGH if any of DROP DATABASE/TABLE/SCHEMA, TRUNCATE; else MODERATE if any of DELETE/UPDATE/MERGE/EXECUTE/ALTER TABLE/DROP INDEX; else LOW) and inject `risk.level` and labels.
  3. For rules with this source in order: skip empty conditions; literal `true` ⇒ match; else if ∃ var map with condition true ⇒ match. First match wins.
  4. Else repeat for fallback rules with only `project_id`.
  5. Store `{template, approval_input_version, finding_done=true}` via compare-and-swap under a (project, plan) advisory lock; conflict ⇒ retry later.
- **Algorithm (actions):**
  - Approve/Reject: refuse if any rejection exists; `step ← first role without an APPROVED entry`; actor must hold `step`; refuse self-approval / last-editor approval per settings; append entry; notify next step or creator.
  - Re-request: creator only; remove rejected entries; notify.
  - Content change: bump `approval_input_version`; next finding replaces approval state (prior approvals void).
  - Completed when all roles have APPROVED entries.
- **Data structures:** ordered rule list; approval payload; per-plan version counter; advisory lock keyed by hash(project, plan).
- **Complexity:** O(rules × varMaps) CEL evaluations; var maps can explode (tasks × types × tables) — cap.
- **Edge cases:** no rule matches (auto-approve); template with zero roles; role with no holders (Bytebase has `FindUnapprovableStep` to surface this); approver loses role mid-flow; same person holds consecutive roles (Bytebase requires separate actions per step); rule list edited after finding (finding is sticky until input version changes); plan edited after approval (invalidate); rollout already started (refuse new finding); var-map evaluation error; issue labels changed when labels affect approval.
- **Test cases:** (1) DROP TABLE in prod with rule `risk.level == "HIGH" && resource.environment_id == "prod"` → template [projectOwner, workspaceDBA]. (2) Approve by projectOwner then reject by DBA → blocked; creator re-requests → DBA pending again. (3) Edit plan after first approval → approvals cleared. (4) Creator attempts approval with self-approval disallowed → refused. (5) No matching rules → no approval required.
- **Prior art and evidence:** `component/review/evaluator.go`, `approval.go`, `workflow.go`, `risk.go`; canonical Perde state machine in doc 05.
- **Used by:** Perde (authoritative), Karagöz (verifies approval token before executing a gated statement), Hacivat (UI).

### ALG-BYTEBASE-04: Lineage-based result masking (observed pipeline)

- **Problem:** Mask result values derived from sensitive columns, for a specific user, without leaking via expressions, predicates, errors or misalignment.
- **Inputs / outputs:** per statement: query span (output columns → set of source columns; predicate columns; unresolved-columns flag; type), result (column names, rows, error); policies (masking rules, column catalog, semantic types, classification config, exemptions for the user). Output: masked result + per-column reasons, or refusal.
- **Algorithm:**
  1. Pre-execution: if any predicate column is masked for the user (engines with predicate tracking) → refuse statement.
  2. Post-execution, per statement: skip EXPLAIN; if lineage unresolved on a masking engine → refuse (and schedule metadata resync); if error-only result and statement touches masked columns → replace error text generically; if result column count ≠ span column count and masked columns involved → refuse.
  3. For each output column: collect effective maskers of its source columns (ALG of §3.7.3 per source column); 0 → identity; 1 and plain field → that masker; 1 via expression, or ≥2 → full mask.
  4. Apply maskers to all non-nil values; attach reasons.
- **Data structures:** prefetched maps database→project, database→schema metadata, project→exemption policy (avoid N+1); semantic type→masker map.
- **Complexity:** O(rows × columns) masking; O(columns × rules) evaluation (cacheable per (user, column) per policy version).
- **Edge cases:** `SELECT *` expansion; views (lineage through view definition); CTEs and subqueries; UNION branches with different sources; `CAST(masked AS int)` in predicates; aggregates (`COUNT(masked)` → source is masked ⇒ full mask even though count reveals nothing — over-masking); window functions; JSON operators; `NULL` values (Bytebase leaves nil untouched but maskers map NULL to `******` for non-nil NULL kinds); binary columns; stale schema; case sensitivity of identifiers; quoted identifiers; temporary tables; stored procedures returning result sets (no lineage ⇒ refuse on masking engines); engines without lineage (Redis, ClickHouse — Bytebase does not mask).
- **Test cases:** (1) `SELECT email FROM users` with email semantic type full → `******`. (2) `SELECT upper(email)` → full mask with reason "Expression-based column". (3) `SELECT name || email` → full mask "Multiple source columns". (4) `SELECT id FROM users WHERE email = 'x@y'` → refused (predicate). (5) `SELECT CAST(ssn AS int) FROM t` failing with value in error → generic error. (6) user exempted by `resource.table_name == "users"` until tomorrow → unmasked; after expiry → masked.
- **Prior art and evidence:** `api/v1/query_result_masker.go`, `masking_evaluator.go`, `predicate_columns_check.go`, `component/masker/*`. Canonical streaming variant for Karagöz: doc 05 ("data masking at result-stream level").
- **Used by:** Karagöz (must run at the agent because results never transit Perde), Hacivat (displays reasons; never trusted for enforcement).

### ALG-BYTEBASE-05: Signed-license feature gating (observed) and Perde adaptation

- **Problem:** Decide at runtime whether a feature is entitled, with offline verification and safe failure.
- **Inputs / outputs:** stored license string, embedded public key(s), plan matrix, feature id, (optionally) resource (instance). Output: allow or error naming the minimum plan.
- **Algorithm (observed):**
  1. Load license from settings (cached ≤1 min, singleflight).
  2. Parse JWT: algorithm family must be RSA; `kid` must equal the embedded key version; verify signature; check `nbf`, `iss`, `aud`; plan must be known; enterprise non-trial must match workspace id.
  3. Any failure → FREE. Expired → FREE (retain expiry for UI).
  4. `allowed ← matrix[plan][feature]`; if not allowed → error with minimal plan (TEAM if TEAM has it, else ENTERPRISE).
  5. For instance-scoped features on paid plans: also require instance activated or a unified license.
- **Perde adaptation (proposal):** use Ed25519 (smaller keys, no RSA/PSS pitfalls) with a key-set (multiple `kid`s) to allow rotation; include `features` explicitly in the token (not only plan) so hosted plans can grant à la carte without new binaries; never gate *security enforcement in Karagöz* — gate only Perde management features (doc 06 decision matrix); grace period (e.g. 14 days) after expiry before downgrading management features; never delete data on downgrade (Bytebase's filter-not-delete on audit is the right instinct).
- **Data structures:** plan matrix (YAML shared by server and UI), key set, cached effective subscription per tenant.
- **Complexity:** O(1) after cache.
- **Edge cases:** clock skew vs `nbf`/`exp`; multiple replicas with different cache ages (1-minute inconsistency); license uploaded for wrong tenant; downgraded while feature is configured (existing masking rules then stop applying in Bytebase — a *security regression on non-payment*); offline/air-gapped renewal; key rotation; HA replica counting false positives during rolling deploy.
- **Test cases:** (1) `alg=HS256` token signed with the public key as HMAC secret → rejected. (2) valid enterprise license for workspace A loaded in B → FREE. (3) expired yesterday → FREE, UI shows expiry. (4) TEAM license, request `FEATURE_DATA_MASKING` → error "ENTERPRISE feature". (5) 2 replicas, no HA claim → error.
- **Prior art and evidence:** `backend/enterprise/license.go`, `plan.yaml`, `frontend/src/types/plan.ts`.
- **Used by:** Perde.

### ALG-BYTEBASE-06: Statement-pinned just-in-time access grant

- **Problem:** Let a user (or AI agent) obtain time-boxed permission to run *one specific* read statement against specific databases, optionally unmasked or exportable, after approval.
- **Inputs / outputs:** targets, statement text, flags {unmask, export}, reason, TTL or absolute expiry, schema; approval rules. Output: grant (PENDING→ACTIVE→expired/REVOKED); at query time, the grant applied or not.
- **Algorithm:**
  1. Validate: feature/project setting enabled; targets exist in one project/instance; statement parses and is read-only for the engine; reason non-empty; expiry ≤ workspace max.
  2. `hash ← sha256_hex(trim_ascii_ws(statement))`.
  3. Evaluate approval rules once with source REQUEST_ACCESS and vars `{targets' resource attrs, request.unmask, request.data_export, request.expiration_days}`; persist the result (sticky).
  4. Create grant(PENDING) + linked issue; when approved, activate: `expire_time ← now + ttl` if TTL-based.
  5. At query time: candidates ← ACTIVE grants with creator = caller, target ∋ database, hash(statement) = hash, schema/container match, export flag if exporting, expire_time > now; choose the one with unmask preferred; re-verify read-only; apply to its targets only; all other referenced databases still need regular IAM.
- **Data structures:** grant table indexed by (project, creator, status, query_hash); issue link.
- **Complexity:** O(log n) lookup by hash.
- **Edge cases:** whitespace/comment differences (only boundary whitespace trimmed — a trailing comment breaks the match, by design); parameterised statements (no parameters — literal text only); statement referencing another database (needs IAM there); grant approved after its absolute expiry; revocation during a running query; clock skew; unicode normalisation of the statement (not done — NFC vs NFD would differ); case of SQL keywords (not normalised).
- **Test cases:** (1) grant for `SELECT * FROM orders WHERE id=7` on db A; executing identical text → allowed; with trailing `;` → depends on trim rules (`;` not trimmed → no match). (2) executing on db B → no grant. (3) `unmask=true` → masking skipped. (4) TTL 1h, approved after 3h → expires 1h after activation. (5) revoked → next execution denied.
- **Prior art and evidence:** `proto/store/store/access_grant.proto`, `access_grant_service.go`, `sql_service.go preCheckAccess/selectBestAccessGrant`.
- **Used by:** Perde (issuance/approval), Karagöz (verification + enforcement; the grant becomes a signed capability in our design — doc 05), Hacivat (request UI; MCP tool "request approval for this exact query").

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 Architecture placement

| Bytebase mechanism | Where it runs in Bytebase | Where it must run for us | Why |
|---|---|---|---|
| Statement classification (read-only, per-statement permission) | Bytebase server | **Karagöz** (authoritative) + Hacivat (advisory, early feedback) | Data path is P2P; Perde never sees statements. |
| SQL review rules | Bytebase server (plan checks) | Hacivat for lint-as-you-type (offline, no login); Karagöz for enforcement and live-connection rules; Perde distributes rule sets | Hacivat must work offline; Karagöz is the trust boundary. |
| Masking | Bytebase server after execution | **Karagöz**, streaming per row | Results never pass through Perde; Hacivat is untrusted for enforcement. |
| Approvals / JIT | Bytebase server | Perde decides; Karagöz verifies a signed artifact offline | Karagöz must be able to enforce without calling Perde on each query. |
| Audit | Bytebase metadata DB | Karagöz local append-only log, batched to Perde with hash chaining (doc 05) | Bytebase has no tamper evidence; ours must, since the agent may be on a compromised host. |
| Feature gating | Bytebase server | Perde only | Never gate enforcement in the OSS agent (doc 06). |

### 5.2 Concrete recommendations

1. **Adopt the binding model** `{role, members, condition}` with CEL-like conditions, but define a *small, typed* attribute vocabulary that Karagöz can evaluate offline: `resource.{agent_id, database_id, engine, environment, labels, schema, table, column}`, `request.{time, client (hacivat|mcp|cli), ai_agent (bool), purpose}`, `statement.{class (read|dml|ddl|admin), risk, tables, estimated_rows}`. Compile policies in Perde to a deterministic, size-bounded form (doc 05 discusses Cedar/CEL/OPA choices); never do textual surgery on expressions (Bytebase's environment-stripping regex is an anti-pattern).
2. **Copy the permission split for SQL**: `sql.select`, `sql.info` (metadata), `sql.explain`, `sql.dml`, `sql.ddl`, `sql.export`, `sql.admin` (unrestricted session), plus `sql.unmask`. Predefined roles: *Read*, *Read+Export*, *Write (DML)*, *Schema (DDL)*, *Admin*, *Viewer (metadata only)*. These map cleanly onto per-DBMS ephemeral-role privilege templates (doc 04).
3. **Defence in depth for read-only**, better than Bytebase: (a) parser classifier in Karagöz (fail closed on unknown statements), (b) **ephemeral DB user with read-only grants** created per session (doc 04 — this is the real guarantee and Bytebase's "read-only data source" equivalent, but automatic), (c) session-level read-only where the DBMS supports it (`default_transaction_read_only` + `ALTER ROLE … SET default_transaction_read_only=on` so `SET` cannot override without privileges? — note: a role-level default can still be overridden by the session; the grants are the real guard), MySQL `SET SESSION TRANSACTION READ ONLY`, Mongo read-only built-in role, Redis ACL `+@read -@write -@dangerous`. Treat (a) as UX, (b) as enforcement.
4. **Refuse `SET` of transaction-mode/role variables** in read-only sessions in the Karagöz classifier (Bytebase allows `SET`). Explicitly deny `SET ROLE`, `SET SESSION AUTHORIZATION`, `SET default_transaction_read_only`, `SET TRANSACTION READ WRITE`, `RESET ALL`, `DISCARD ALL` in read-only mode.
5. **Statement-pinned grants as signed capabilities**: implement ALG-BYTEBASE-06 but as a Perde-signed token (Biscuit/PASETO, doc 05) carrying `{agent, database, stmt_hash, flags, nbf, exp, approver ids}`; Karagöz verifies offline. Normalise the statement more carefully than Bytebase: canonical fingerprint (doc 05) + exact-text hash, and decide explicitly which one binds (exact text by default; fingerprint only for "this query shape with any literals" grants, flagged as higher risk).
6. **Approval input versioning**: any edit to a pending request's statements/targets voids prior approvals; persist the version in the capability so a stale approval cannot authorize a changed statement.
7. **Masking**: implement lineage-based masking in Karagöz with Bytebase's fail-closed rules (unresolved lineage ⇒ refuse; expression ⇒ full mask; ≥2 sources ⇒ full mask; predicate on masked column ⇒ refuse; error text redaction). Do **not** ship MD5 without a per-tenant secret (use HMAC-SHA-256 with a tenant key for deterministic pseudonymisation) and do not call "middle" masking a security control (label it "display truncation").
8. **AI agents (Hacivat MCP)**: copy Bytebase's three ideas: a per-workspace capability ceiling for MCP clients; a read-only clamp independent of the human's own rights; and a *masked-sentinel write-back refusal*. Improve on it by making Karagöz emit a **distinct, configurable sentinel per tenant** (e.g. `⟦masked⟧`) for full masks so detection is reliable, and by marking masked cells in the wire result metadata so Hacivat can refuse to build write-back statements from masked cells at the UI level.
9. **Audit**: copy (a) schema-level redaction annotations with a CI inventory test, (b) MCP/agent provenance fields (`client_kind`, `oauth_client_id`, `correlation_id`), (c) "first page pins the clock" pagination. Add what Bytebase lacks: hash chaining, agent-side signing, native SIEM export (doc 05).
10. **Do not copy the business gating of security features.** Bytebase paywalls masking, approvals, 2FA enforcement, OIDC/SAML and audit visibility; for Perde the recommended split is in doc 06 (security enforcement and basic SSO free; scale/compliance/convenience paid).

### 5.3 Data model sketch for Perde (derived)

```
Org ─┬─ Environment (ordered, tier: protected?)
     ├─ Project ─┬─ Database (discovered by Karagöz: agent_id, engine, labels, env)
     │           ├─ ProjectPolicy { bindings[], masking_exemptions[], query_policy }
     ├─ OrgPolicy { bindings[], masking_rules[] (ordered), approval_rules[] (ordered), review_rule_sets[] }
     ├─ SemanticType[] { id, title(i18n key), algorithm }
     ├─ ClassificationConfig[] { levels[], classes{} }
     ├─ Request (kind: role|access|change, state machine doc 05, input_version)
     └─ AuditStream per agent (hash-chained batches)
```

---

## 6. i18n considerations

- Bytebase UI ships `en-US`, `zh-CN`, `ja-JP`, `es-ES`, `vi-VN` with **separate locale bundles per domain** (`locales/<lang>.json`, `locales/sql-review/<lang>.json`, `locales/subscription/<lang>.json`, `locales/dynamic/<lang>.json`). Rule titles/descriptions live in the frontend keyed by rule type; backend advice `title`/`content` strings are English and are shown verbatim. [Observed: `frontend/src/locales/`]
  - **For Hacivat/Perde:** advice and refusal messages produced by Karagöz must be **message IDs + arguments** (e.g. Fluent IDs), not English sentences, so Hacivat renders them in Turkish/English without a server round trip. Keep SQL text, identifiers and values outside the translated template.
- Rule configuration UI labels (`component.number.title` etc.) are per-rule keys — a good pattern to mirror for our rule catalog.
- **Turkish-specific pitfalls** in this domain:
  - Naming-convention regexes and identifier case rules (`NAMING_IDENTIFIER_CASE`, keyword checks) must use **locale-independent** case folding (ASCII or Unicode simple folding), never the Turkish locale where `I`→`ı` and `i`→`İ`; a Turkish-locale `toUpperCase("id")` yields `İD` and would break keyword detection and masking-rule matching on `resource.column_name == "ID"`.
  - Masking `RangeMask` uses **character (rune) indices** — correct for Turkish letters (ç, ğ, ı, ö, ş, ü are single code points in NFC); but text in NFD (decomposed) would shift indices. Normalise to NFC before masking in Karagöz.
  - Collation allowlists (`SYSTEM_COLLATION_ALLOWLIST`) should include `tr_TR`/`utf8mb4_turkish_ci` awareness in default templates for Turkish users.
  - CEL string comparisons are code-point exact: Perde's policy editor should warn when a condition literal contains dotted/dotless i variants that do not match the catalog's actual identifier.
- Classification level titles and semantic type titles are user content; store them as user-provided strings with optional per-locale variants.
- Audit export: timestamps in UTC RFC 3339; never localise audit data at rest.

---

## 7. Risks, pitfalls, open questions

### 7.1 Pitfalls observed in Bytebase (to avoid)

1. **Masking disappears when the license lapses** (FREE has no `FEATURE_DATA_MASKING`; the masker short-circuits when the feature is not enabled) — a non-payment event becomes a data-exposure event. [Inferred: from gate at `sql_service.go:873` and `instanceFeatures`; exact behaviour on downgrade not executed]
2. **Approval workflow silently degrades to "no approval"** on FREE/TEAM. Fine for a change-management product; unacceptable for a privileged-access product.
3. **Read-only relies on parsing** for most engines; `SET` statements accepted on PG; Export on MySQL skips the validator; `disable_download` is UI-only. [Observed, see §3.6.2]
4. **Environment-condition regex stripping** may mis-handle negations or OR-combinations. [Inferred]
5. **Audit tables are mutable and untamper-evident**; refusals before the handler are not stored (only stdout).
6. **MD5 masking** without salt is dictionary-reversible; MD5 on `timestamptz` returns a partial value instead of a hash. [Observed + Inferred]
7. **First-match ordering** in masking rules and approval rules is easy to misconfigure; Bytebase offers no "shadowed rule" warnings in the code paths read. [Inferred]
8. The license clause covering "any code controlling … permission, role …" makes reuse of Bytebase's IAM/permission code legally risky even though the files sit outside `enterprise/` — we do not reuse any Bytebase code. [Inferred]

### 7.2 Open questions

- [Unknown] Whether results of `AdminExecute` (admin mode) are masked.
- [Unknown] Exact behaviour when a TEAM workspace that previously configured masking rules (during a trial) downgrades — are rules hidden but persisted, and does masking stop immediately?
- [Unknown] Whether the PG `SET default_transaction_read_only = off; SELECT writing_function()` sequence is actually reachable through the editor on a read-only data source (would also be stopped if the read-only data source's DB user lacks write grants).
- [Unknown] How `query_hash` matching treats a trailing semicolon or a leading comment (only ASCII boundary whitespace is documented as trimmed).
- [Unknown] Whether project-level IAM conditions on `resource.table_name` are honoured for MongoDB collections and Elasticsearch indices in the per-statement ACL.
- [Unknown] Bytebase's licensing history (whether earlier versions were Apache-2.0 or MIT-only) — the shallow clone has no history; not needed for our decisions.

### 7.3 Risks for our design

- Re-implementing lineage for masking across 7+ engines is the single largest engineering cost; Bytebase uses a large in-house parser stack ("omni") for it. Hacivat's research on `sqlparser-rs`, `pg_query.rs` (see `clients/`) must confirm lineage feasibility per engine; ship masking first for PostgreSQL and MySQL, refuse (fail closed) elsewhere.
- Statement-pinned grants are brittle for humans (any edit invalidates) but ideal for AI agents (exact text, reviewable). Offer both exact and fingerprint modes with different approval policies.
- A CEL-like condition language at the agent increases attack surface; bound expression size, evaluation cost, and available functions.

---

*Sources (web): [Bytebase pricing](https://www.bytebase.com/pricing), [sso.tax](https://sso.tax/).*
