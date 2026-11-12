---
version: 1
slug: "design-directions-harman-index-html"
primary_target: "design/directions/harman/index.html"
related_targets: []
---

# Hacivat main workspace

Mode: Operate. Primary surface of the native desktop client (macOS first, Windows/Linux in v1). Reference mockup: `design/directions/harman/index.html` (approved by the product owner on 2026-11-11 as a piece-by-piece blend of three explored directions: Grafit Konsol base, İznik Sırrı title-bar capsule and grid features, Kategori Standardı light theme).

## Direction contract

THESIS: A graphite console where trust is the only loud thing. The category default spreads accent color everywhere and stacks filled environment badges across the chrome; Hacivat refuses that. Color is reserved for meaning, and production is the one signal allowed to shout: exactly one filled badge (the title-bar capsule); every other occurrence is a small red mark or plain text.

OWN-WORLD: Graphite ground #16181D with #1E2127 panels, separated only by 1px seams (#2A2E36), no cards and no floating chrome except popovers. Source Sans 3 (small-caps section labels) paired with Fragment Mono for code, data and measurements, always tabular. Lucide icons at 1.6 stroke. Radii stay at 3, 4 and 6 px. Semantic hues: red = production, green = direct path / success, amber = pending, blue = selection and focus. The light theme is a quiet macOS white (#FFFFFF / #F7F7F8 / #E5E5E9 / #1B1B1F).

STORY: A developer opens a prod connection and knows at once where they are (orders-prod, PostgreSQL 16, production, read-only, Direct P2P via Karagöz) from a single title-bar capsule. They write SQL with schema-aware completion, read results in a dense grid with record and range inspectors, and stage edits only where edits are allowed, with each one previewed as DML.

FIRST VIEWPORT: 1440×900 window. A unified title bar holds traffic lights, a placeholder logo, then the connection capsule anchored left after the brand with a filled red PRODUCTION block (saturated red, white text, identical weight in dark and light), and a search field centered. Left: a 276px explorer tree with a Connection panel at its foot. Center top: editor tabs, then the SQL editor with gutter statement status and a completion popup with a detail pane. Center bottom: result tabs, a WHERE/ORDER BY toolbar, the grid with a right-hand Record/Selection inspector, and a "next page ready" strip. Bottom: a status bar. Signature interaction: a selected cell range updates sum/avg/count live in the inspector. Motion grammar: no ambient motion; 120–160 ms ease-out for popovers and tab switches; one-axis progress line on splash.

FORM: Blend form "harman" (piece-by-piece selection over the three-direction comparison), #1 on the ordered list after user selection. Seed key: b73d2712.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
