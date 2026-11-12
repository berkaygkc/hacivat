# Direction mockup brief (shared by every direction)

Purpose: the product owner must compare visual directions **by looking at them**. Every direction renders the **same screen with the same synthetic content**, so the only difference is the visual world. Fairness matters: equal effort, equal fidelity, full commitment to each world.

Read before building:
- `/Users/berkaygokce/Documents/hobby-projects/hacivat/PRODUCT.md` (product truth, users, principles)
- `/Users/berkaygokce/.claude/skills/impeccable/reference/craft-floor.md` (quality floor and bans — obey)
- `/Users/berkaygokce/.claude/skills/impeccable/reference/operate.md` (Operate-mode guidance)
Do NOT run impeccable's concept-seed, serve-question or init flows; the direction is already chosen for you.

## Hard constraints
- Must be reproducible in GPUI (native Rust): solid fills, 1px borders, rounded corners, soft drop shadows, simple linear gradients, at most one native-vibrancy-like translucent sidebar. **No** CSS filters, stacked backdrop blurs, complex masks/clip-paths, gradient text, glow halos.
- Desktop app window, **1440×900** viewport. Draw a macOS-style window (traffic lights, unified title/toolbar) since macOS is primary. No mobile layout.
- Fonts: self-host or load from Google Fonts / Fontsource via CDN. Pick faces that suit the world (avoid the craft-floor's overused list unless justified). Monospace only for code, data and measurements. Tabular numerals in the grid.
- Icons: one consistent set (e.g. Lucide or Phosphor via CDN, or authored SVG). No emoji as icons.
- Single self-contained `index.html` (inline CSS/JS; CDN links allowed for fonts/icons only). No frameworks needed.
- Dark AND light theme (toggle in the window chrome, keyboard `T`), English AND Turkish UI strings (toggle, keyboard `L`). Turkish strings are longer; layout must not break. SQL, identifiers and data values are never translated.
- "Pro dense but breathing": DataGrip-level information density with clearer hierarchy and rhythm. Must not feel web/Electron-like and must not feel toy-like.
- Reference bar for finish quality: Linear, Raycast (and CleanMyMac for the splash/loading moment). Do not copy them.

## The screen (identical content in every direction)
Window title area: product name **Hacivat**, connection switcher showing the active connection.

1. **Left: Explorer tree** (resizable feel)
   - Group "Local": `local-sqlite` (SQLite) with `app.db`.
   - Group "Perde · Acme" (databases provided by Karagöz agents): 
     - `orders-prod` — PostgreSQL 16 — environment **production**, access **read-only**, path **direct (P2P)**, agent `karagoz@eu-west-1a`
     - `orders-staging` — PostgreSQL 16 — staging, read-write
     - `cache-prod` — Redis 7.4 — production, read-only
     - `events` — MongoDB 7 — staging
   - `orders-prod` expanded: database `orders` → schemas `public`, `billing` → `public` tables: `customers`, `orders` (selected), `order_items`, `products`, `audit_log`; folders Views, Functions, Sequences (collapsed).
2. **Center top: SQL editor** with tabs: `orders-prod.sql` (active), `revenue-by-city.sql`, `scratch.sql` (unsaved dot).
   Editor content (syntax highlighted, line numbers, gutter with per-statement status ✓ executed 42 ms on stmt 1, running spinner not needed):
   ```sql
   -- Top customers by revenue, last 30 days
   SELECT c.full_name,
          c.city,
          count(o.id)          AS orders,
          sum(oi.line_total)   AS revenue
   FROM customers c
   JOIN orders o ON o.customer_id = c.id
   JOIN order_items oi ON oi.order_id = o.id
   WHERE o.ordered_at > now() - interval '30 days'
   GROUP BY c.full_name, c.city
   ORDER BY revenue DESC
   LIMIT 100;

   SELECT * FROM orders WHERE status = 'paid' AND c
   ```
   The caret is at the end of the last line; a **completion popup** is open listing: `customer_id` (column · bigint · orders), `created_at`? no — use `city` (column · text · customers — "not in scope" dimmed), `cancelled_at` (column · timestamptz · orders), `count()` (function), `current_date` (keyword/function), `case` (keyword). Include a small detail pane for the selected item (type, nullable, FK → customers.id).
   Inlay hint on line 4–5 optional. The current statement has a subtle execution-range highlight.
3. **Center bottom: Result grid** for statement 1: columns `full_name` (text), `city` (text), `orders` (bigint), `revenue` (numeric(14,2)); 18+ visible rows of realistic synthetic data incl. Turkish names (Ada Yılmaz, Bora Işık, Cem Öztürk, Deniz Çelik, Elif Şahin, …), cities (İstanbul, Ankara, İzmir, Iğdır, Berlin, London…), one NULL city (render NULL distinctly from empty), right-aligned tabular numbers. Show: column header with type, sort indicator on revenue, selected cell range (2×3), row numbers, one **pending edit** (a changed cell, visibly marked as not yet submitted) even though prod is read-only — instead show the pending edit in a staging tab context OR show the read-only lock affordance explaining edits are disabled on prod (choose one coherent story; prefer showing the read-only state clearly and a second result tab "orders (staging)" that has a pending edit stamp). Result tabs: `Result 1` (active), `orders (staging)`.
   Grid toolbar: WHERE filter box, page indicator `1–100 of 2,481`, export/copy, "Count rows".
4. **Bottom: Status bar**: environment badge (production · read-only), path `Direct · 18 ms` (vs relay), transaction state `Auto-commit`, rows `100 rows · 42 ms`, `Ln 14, Col 49`, encoding/dialect `PostgreSQL`, language `EN/TR`.
5. **Trust cues must be unmistakable**: you are on production, read-only, direct P2P path via Karagöz. This is the product's core principle ("trust is visible").

## Second view (same file, switch with a visible control + key `S`)
A **splash / first-run loading moment** at the same window size: the app opening and connecting to `orders-prod` through Perde (steps: "Resolving agent karagoz@eu-west-1a", "Hole punching… direct path found", "Introspecting 214 objects"). This is where the CleanMyMac-level delight belongs; motion allowed (CSS animation, respects prefers-reduced-motion). Must still be GPUI-reproducible in spirit (no filters).

## Interactions to make clickable (lightweight JS)
- Theme toggle, language toggle, splash/workspace toggle.
- Clicking tree items selects them; clicking a completion item selects it; hovering grid rows/cells shows hover state; clicking result tabs switches between the two result tabs.

## Deliverables
- `design/directions/<id>/index.html`
- Screenshots via headless Chrome, 1440×900, device scale 2:
  `"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --disable-gpu --hide-scrollbars --window-size=1440,900 --force-device-scale-factor=2 --virtual-time-budget=4000 --screenshot=<out.png> "file://<abs path>/index.html?theme=dark&lang=en&view=workspace"`
  Support URL params `theme`, `lang`, `view` (workspace|splash) so screenshots are deterministic; disable entrance animations when `view=workspace`.
  Save to `/Users/berkaygokce/Documents/hobby-projects/hacivat/.impeccable/mocks/decision/<id>.png` (dark, en, workspace) plus `<id>-light.png`, `<id>-tr.png` (dark, tr), `<id>-splash.png`.
- Open each screenshot once to validate it (no blank areas, fonts loaded, nothing overflowing). One fix round, then stop.
- Final message: ≤12 lines: what the world is, fonts/icons used, any GPUI-feasibility caveats, file paths.
