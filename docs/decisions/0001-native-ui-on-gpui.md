# ADR 0001 — Hacivat desktop UI: native Rust on GPUI with our own component library

- Status: Accepted (pending a one-week validation spike, see "Validation")
- Date: 2026-11-11
- Supersedes: the Tauri v2 + TypeScript recommendation in `docs/research/clients/ui-01-stack-decision.md` §6

## Context

`ui-01-stack-decision.md` scored Tauri v2 + TypeScript highest (90/100) with GPUI second (70/100; 75 under "native purist" weights). That matrix weighted code reuse with the Perde web panel (C11) and the maturity of web editors/grids heavily, and gave no weight to product positioning.

Product owner input:
- Hacivat's audience is developers. "Native Rust" is a strong trust and excitement signal for that audience; a webview shell is a negative signal ("burun kıvırıyorlar").
- Priorities are performance, a lightweight feel, and user trust.
- We want to build and publish our own UI component library rather than depend on a third-party widget set.
- v1 languages are English and Turkish (both LTR), which removes most of GPUI's i18n penalty (incomplete RTL) for v1.

License facts (verified in `docs/research/verification/02-external-fact-check.md`): `gpui` and its platform crates are Apache-2.0; Zed's `editor`, `ui`, `workspace`, `rope`, `text` crates are GPL-3.0-or-later. Using GPL crates would force Hacivat (and any library we publish that links them) to be GPL; attribution alone does not satisfy GPL. Zed's editor is also not a standalone, published library.

## Decision

1. **Engine:** GPUI (Apache-2.0), pinned to a specific git commit and upgraded deliberately. No GPL Zed crates are used or read for implementation ideas.
2. **Components:** our own library, `hacivat-ui` (Apache-2.0, published separately): data grid (2-axis virtualization, 10M+ rows), SQL editor (completion, inlays, gutter, multi-cursor), docking/tabs/splits, theming, Fluent-based i18n. gpui-component (Apache-2.0) may be used as reference or temporary dependency and replaced incrementally.
3. **Boundary:** the application talks to GPUI only through a thin adapter inside `hacivat-ui`, so a fork of GPUI or a different engine remains possible.
4. **Core stays UI-agnostic:** drivers, SQL engine, MCP, networking live in `hacivat-core` crates with no UI dependency (ALG-UI-02 in ui-01 still applies as a design principle). With an in-process native UI, the windowed Arrow IPC transport (ALG-UI-01) is not needed; result windows are shared in-process.
5. **Fallback engine:** Linebender stack (winit + wgpu + parley + AccessKit + taffy) if validation fails.
6. **Messaging:** "Built with GPUI, the engine behind Zed" is accurate and allowed; do not imply Zed endorsement or use Zed trademarks beyond factual attribution.

## Consequences

- Positive: native performance and memory footprint; single language/toolchain; no webview divergence across OSes; strong developer positioning; a publishable permissive UI library.
- Negative: we build the SQL editor and grid ourselves (largest cost); GPUI is pre-1.0 and Zed-driven (API churn); packaging, code signing and auto-update are DIY; RTL needs our own work; accessibility is less work than first assumed because GPUI exposes AccessKit hooks, but every custom widget (grid, editor) still needs its own a11y tree; GPUI's clipboard has no HTML flavor, so rich copy (grid → HTML table for Excel/Docs) needs our own per-platform clipboard module; no UI code sharing with the Perde web panel (Perde panel is a separate web codebase).

## Validation (one-week spike on GPUI, all three OSes)

| # | Benchmark | Pass |
|---|---|---|
| B1 | Scroll 10M rows × 50 columns with fling | ≥ 55 fps macOS/Windows, ≥ 45 fps Linux reference; no blank frame > 100 ms |
| B3 | 50k-line SQL file | typing latency ≤ 16 ms p95; completion popup ≤ 50 ms |
| IME | Turkish Q and F layouts, dead keys, Japanese/Chinese IME smoke test | correct composition, caret and candidate window placement |
| PKG | Signed builds + update flow on macOS/Windows/Linux | works with chosen tooling |

Failing B1/B3/IME on any OS triggers re-evaluation with the fallback engine.
