# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

> Hacivat is a native desktop application (macOS, Windows, Linux) rendered with GPUI, using its own custom-drawn design language rather than OS-native widgets. "web" is recorded because the visual system is self-drawn, not platform-native. Desktop only: no mobile or tablet layouts. macOS is the primary design and test target; Windows and Linux ship in v1 with their window-chrome and shortcut conventions respected. Perde's control panel is a web app sharing the same design tokens.

## Stack

- Product UI: native Rust on GPUI (Apache-2.0, pinned commit) with our own published component library `hacivat-ui` — decided in `docs/decisions/0001-native-ui-on-gpui.md`. No GPL Zed crates.
- Design exploration mockups: static HTML/CSS under `design/`, restricted to effects GPUI can reproduce (solid fills, borders, rounded corners, drop shadows, simple linear gradients, at most native window vibrancy; no CSS filters, stacked backdrop blur, or complex masks).
- Design tokens are exported to `DESIGN.md` + `tokens.json`, consumed by both `hacivat-ui` and the Perde web panel.

## Users

Primary users, in priority order (all three are first-class):
1. **Backend / full-stack developers** — daily development and production debugging: write queries, inspect data, make quick fixes, across several database engines.
2. **DBAs / platform engineers** — schema work, performance (explain plans), permissions and access management.
3. **Data / analytics practitioners** — long analytical queries, very large result sets, export.

Secondary (Perde, not Hacivat-primary): team leads/administrators who grant access to agent-discovered databases and review audit logs.

## Product Purpose

An open-source, multi-database desktop client that matches DataGrip-class depth (SQL editor, data grid, schema tools) while being faster, lighter and more trustworthy. Together with Karagöz (server agent) and Perde (control plane), it lets a developer open production databases without typing credentials: Karagöz discovers databases on servers, Perde grants role-based access, and Hacivat connects peer-to-peer (relay only as fallback). Success: developers choose Hacivat over DataGrip/DBeaver/TablePlus for daily work and trust it with production.

## Positioning

- Native Rust, open source, no login required for local use; built with GPUI, the engine behind Zed.
- Zero-credential, peer-to-peer access to production databases through Karagöz + Perde, with authorization and tamper-evident audit enforced at the agent.
- Native local MCP server so AI agents can use the user's connections under explicit, narrower permissions and human approval for writes.
- First-class internationalization from day one (English + Turkish in v1).

## Operating Context

- Long working sessions (hours), many tabs, multiple connections to different engines (PostgreSQL, MySQL/MariaDB, SQL Server, SQLite, ClickHouse, Redis, MongoDB) and environments (local, staging, production).
- Production access is high-stakes: users must always know which environment, which permissions (read-only vs write) and whether a transaction is open.
- Keyboard-heavy workflows; command palette and shortcuts expected (DataGrip-like defaults acknowledged).
- Large data: results with millions of rows, wide tables, JSON/binary values.

## Capabilities and Constraints

- Core surfaces (v1): connection/explorer tree, SQL editor (completion, inlays, per-statement execution status), result data grid (editing with pending changes and DML preview), Redis key browser, MongoDB document views, query history, settings, Perde sign-in and agent-provided databases, MCP permissions/approvals.
- English-first source strings; i18n via Fluent; English + Turkish in v1; Turkish strings are typically longer and must not break layouts; adding a language must need no code changes.
- Feature split between self-hosted and hosted Perde is undecided.
- Research and algorithms: `docs/research/` (index in `docs/research/README.md`).

## Brand Commitments

- Names: **Hacivat** (desktop client), **Karagöz** (server agent), **Perde** (control plane / "the screen"), from Turkish shadow theatre (Karagöz–Hacivat). Perde is the stage that connects the two characters.
- How visibly the shadow-theatre heritage appears is **undecided**: explore both (a) a visible identity element (logo, iconography, empty-state illustration, voice) and (b) a subtle trace (logo, names, small details such as a splash) as alternatives.
- Repositories, code, docs and UI source strings are English.

## Evidence on Hand

- No logo, screenshots, testimonials, customers or benchmarks exist yet. Do not fabricate any.
- Extensive research library and decisions: `docs/research/`, `docs/decisions/`.

## Product Principles

1. **Trust is visible.** Environment, permissions, transaction state and data path (direct vs relay) are always legible; destructive actions are never ambiguous.
2. **Speed you can feel.** Every interaction is instant or shows progress; nothing blocks the UI; huge results stay fluid.
3. **Depth without clutter.** DataGrip-level power, revealed progressively; the default view stays calm.
4. **Keyboard first, mouse friendly.** Every action reachable by keyboard and palette; pointer interactions are precise and forgiving.
5. **Open by default.** Works offline without an account; cloud features are additive, never required.

## Accessibility & Inclusion

- Screen-reader support via AccessKit for the grid and editor is a goal; full keyboard operation is required.
- Turkish keyboard layouts (Q and F) and IME must work correctly; locale-invariant handling of SQL identifiers.
