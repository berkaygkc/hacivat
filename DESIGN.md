---
name: Hacivat
description: A graphite database console where trust is the only loud thing.
colors:
  # Dark theme (default)
  dark-ground: "#16181D"
  dark-panel: "#1E2127"
  dark-side: "#1E2127"
  dark-head: "#1E2127"
  dark-gutter: "#1A1D22"
  dark-raised: "#252931"
  dark-hover: "#23272E"
  dark-seam: "#2A2E36"
  dark-seam-strong: "#363B45"
  dark-seam-soft: "#23262D"
  dark-text: "#C9D1D9"
  dark-text-2: "#8B949E"
  dark-text-3: "#828A96"
  # ## m3c-t41 (M3c-38): disabled ink, never remapped by the Raised Ink Rule (Task 43 adds its row).
  dark-text-disabled: "#636A76"
  dark-on-accent: "#10141C"
  dark-on-red: "#10141C"
  dark-cp-fill: "#CE2C31"
  dark-on-cp-fill: "#FFFFFF"
  dark-blue: "#7AA2F7"
  dark-blue-tint: "rgba(122,162,247,0.12)"
  dark-blue-tint-strong: "rgba(122,162,247,0.22)"
  dark-blue-hover: "#8DB0F8"
  dark-red: "#E06C75"
  dark-red-tint: "rgba(224,108,117,0.13)"
  dark-red-line: "rgba(224,108,117,0.55)"
  dark-green: "#98C379"
  dark-green-fill: "rgba(152,195,121,0.55)"
  dark-amber: "#E5C07B"
  dark-amber-tint: "rgba(229,192,123,0.13)"
  dark-syn-keyword: "#B4A7F2"
  dark-syn-function: "#7DCFDB"
  dark-syn-string: "#98C379"
  dark-syn-number: "#E5C07B"
  dark-syn-comment: "#828A96"
  dark-syn-operator: "#8B949E"
  dark-syn-identifier: "#C9D1D9"
  dark-syn-alias: "#DCE2E9"
  # Light theme
  light-ground: "#F7F7F8"
  light-panel: "#FFFFFF"
  light-side: "#F7F7F8"
  light-head: "#F7F7F8"
  light-gutter: "#FAFAFB"
  light-raised: "#FFFFFF"
  light-hover: "#F1F1F3"
  light-seam: "#E5E5E9"
  light-seam-strong: "#D8D8DE"
  light-seam-soft: "#EFEFF2"
  light-text: "#1B1B1F"
  light-text-2: "#55555E"
  light-text-3: "#6C6C76"
  # ## m3c-t41 (M3c-38)
  light-text-disabled: "#A0A0A8"
  light-on-accent: "#FFFFFF"
  light-on-red: "#FFFFFF"
  light-cp-fill: "#CE2C31"
  light-on-cp-fill: "#FFFFFF"
  light-blue: "#3B5FD9"
  light-blue-tint: "rgba(59,95,217,0.08)"
  light-blue-tint-strong: "rgba(59,95,217,0.15)"
  light-blue-hover: "#3352C4"
  light-red: "#CE2C31"
  light-red-tint: "rgba(206,44,49,0.08)"
  light-red-line: "rgba(206,44,49,0.45)"
  light-green: "#1F7A50"
  light-green-fill: "rgba(31,122,80,0.42)"
  light-amber: "#A35C00"
  light-amber-tint: "rgba(163,92,0,0.10)"
  light-syn-keyword: "#4B57C2"
  light-syn-function: "#0B7189"
  light-syn-string: "#2B7A30"
  light-syn-number: "#B4530A"
  light-syn-comment: "#7C7C86"
  light-syn-operator: "#6E6E78"
  light-syn-identifier: "#1B1B1F"
  light-syn-alias: "#1B1B1F"
  # macOS window controls (theme-independent)
  traffic-close: "#FF5F57"
  traffic-minimize: "#FEBC2E"
  traffic-zoom: "#28C840"
typography:
  body:
    fontFamily: "Source Sans 3, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.35
    fontFeature: "tnum"
  body-strong:
    fontFamily: "Source Sans 3, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 600
    lineHeight: 1.35
    fontFeature: "tnum"
  panel-title:
    fontFamily: "Source Sans 3, system-ui, sans-serif"
    fontSize: "13.5px"
    fontWeight: 600
    letterSpacing: "0.075em"
    fontFeature: "tnum"
    fontVariantCaps: synthesized
  label-sc:
    fontFamily: "Source Sans 3, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 600
    letterSpacing: "0.075em"
    fontFeature: "tnum"
    fontVariantCaps: synthesized
  capsule-env:
    fontFamily: "Source Sans 3, system-ui, sans-serif"
    fontSize: "13.5px"
    fontWeight: 700
    letterSpacing: "0.09em"
    fontVariantCaps: synthesized
  control:
    fontFamily: "Source Sans 3, system-ui, sans-serif"
    fontSize: "12.5px"
    fontWeight: 500
    fontFeature: "tnum"
  body-sm:
    fontFamily: "Source Sans 3, system-ui, sans-serif"
    fontSize: "12.5px"
    fontWeight: 400
    fontFeature: "tnum"
  caption:
    fontFamily: "Source Sans 3, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.35
    fontFeature: "tnum"
  micro:
    fontFamily: "Source Sans 3, system-ui, sans-serif"
    fontSize: "11.5px"
    fontWeight: 400
    fontFeature: "tnum"
  stat-value:
    fontFamily: "Source Sans 3, system-ui, sans-serif"
    fontSize: "15px"
    fontWeight: 600
    letterSpacing: "-0.005em"
    fontFeature: "tnum"
  inlay:
    fontFamily: "Source Sans 3, system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 500
    lineHeight: "14px"
  code:
    fontFamily: "Fragment Mono, ui-monospace, monospace"
    fontSize: "12.5px"
    fontWeight: 400
    lineHeight: "18px"
    fontFeature: "tnum"
  mono-data:
    fontFamily: "Fragment Mono, ui-monospace, monospace"
    fontSize: "11.5px"
    fontWeight: 400
    fontFeature: "tnum"
  mono-micro:
    fontFamily: "Fragment Mono, ui-monospace, monospace"
    fontSize: "11px"
    fontWeight: 400
    lineHeight: "14px"
    fontFeature: "tnum"
  mono-kbd:
    fontFamily: "Fragment Mono, ui-monospace, monospace"
    fontSize: "11px"
    fontWeight: 400
    lineHeight: 1
    fontFeature: "tnum"
  mono-counter:
    fontFamily: "Fragment Mono, ui-monospace, monospace"
    fontSize: "15px"
    fontWeight: 400
    lineHeight: "18px"
    fontFeature: "tnum"
rounded:
  none: "0px"
  xs: "3px"
  sm: "4px"
  md: "6px"
  full: "9999px"
spacing:
  2xs: "2px"
  xs: "4px"
  sm: "6px"
  md: "8px"
  lg: "10px"
  xl: "12px"
  2xl: "14px"
  3xl: "16px"
  danger-isolation: "28px"
  tree-indent: "14px"
  titlebar-height: "40px"
  statusbar-height: "24px"
  panel-header-height: "30px"
  editor-toolbar-height: "32px"
  result-toolbar-height: "34px"
  strip-height: "26px"
  field-height: "26px"
  control-height: "24px"
  tree-row-height: "23px"
  kv-row-height: "22px"
  grid-row-height: "21px"
  grid-header-height: "36px"
  code-line-height: "18px"
  explorer-width: "276px"
  inspector-width: "252px"
  editor-gutter-width: "58px"
  completion-list-width: "300px"
  completion-detail-width: "236px"
components:
  titlebar:
    backgroundColor: "{colors.dark-ground}"
    height: "{spacing.titlebar-height}"
    padding: "0 10px 0 14px"
  capsule:
    backgroundColor: "{colors.dark-panel}"
    textColor: "{colors.dark-text}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    height: "26px"
    padding: "0 9px"
  capsule-hover:
    backgroundColor: "{colors.dark-hover}"
  capsule-env:
    backgroundColor: "{colors.dark-cp-fill}"
    textColor: "{colors.dark-on-cp-fill}"
    typography: "{typography.capsule-env}"
    padding: "0 10px 0 9px"
  palette-field:
    textColor: "{colors.dark-text-3}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    height: "26px"
    padding: "0 6px 0 9px"
  segmented:
    textColor: "{colors.dark-text-2}"
    rounded: "{rounded.sm}"
    height: "26px"
    padding: "0 9px"
  segmented-active:
    backgroundColor: "{colors.dark-raised}"
    textColor: "{colors.dark-text}"
  panel-header:
    backgroundColor: "{colors.dark-head}"
    textColor: "{colors.dark-text-2}"
    typography: "{typography.panel-title}"
    height: "{spacing.panel-header-height}"
    padding: "0 10px 0 12px"
  tree-filter:
    textColor: "{colors.dark-text-3}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.sm}"
    height: "28px"
    padding: "0 6px 0 8px"
  tree-group:
    textColor: "{colors.dark-text-3}"
    typography: "{typography.label-sc}"
    height: "26px"
    padding: "6px 10px 0 12px"
  tree-row:
    textColor: "{colors.dark-text}"
    typography: "{typography.body}"
    height: "{spacing.tree-row-height}"
    padding: "0 10px 0 8px"
  tree-row-hover:
    backgroundColor: "{colors.dark-hover}"
  tree-row-selected:
    backgroundColor: "{colors.dark-blue-tint}"
  connection-kv:
    textColor: "{colors.dark-text}"
    typography: "{typography.body-sm}"
    height: "{spacing.kv-row-height}"
    padding: "0 12px"
  editor-tab:
    textColor: "{colors.dark-text-2}"
    typography: "{typography.body}"
    height: "{spacing.panel-header-height}"
    padding: "0 12px"
  editor-tab-active:
    backgroundColor: "{colors.dark-raised}"
    textColor: "{colors.dark-text}"
  editor-toolbar:
    backgroundColor: "{colors.dark-raised}"
    height: "{spacing.editor-toolbar-height}"
    padding: "0 10px 0 12px"
  editor-gutter:
    textColor: "{colors.dark-text-3}"
    typography: "{typography.code}"
    width: "{spacing.editor-gutter-width}"
  inlay-hint:
    backgroundColor: "{colors.dark-seam-soft}"
    textColor: "{colors.dark-text-3}"
    typography: "{typography.inlay}"
    rounded: "{rounded.xs}"
    padding: "0 4px"
  completion-popup:
    backgroundColor: "{colors.dark-raised}"
    textColor: "{colors.dark-text}"
    rounded: "{rounded.md}"
    width: "536px"
  completion-item:
    textColor: "{colors.dark-text}"
    typography: "{typography.code}"
    rounded: "{rounded.xs}"
    height: "23px"
    padding: "0 7px 0 5px"
  completion-item-active:
    backgroundColor: "{colors.dark-blue-tint-strong}"
  button:
    textColor: "{colors.dark-text}"
    typography: "{typography.control}"
    rounded: "{rounded.sm}"
    height: "{spacing.control-height}"
    padding: "0 9px"
  button-hover:
    backgroundColor: "{colors.dark-hover}"
  button-primary:
    backgroundColor: "{colors.dark-blue}"
    textColor: "{colors.dark-on-accent}"
    rounded: "{rounded.sm}"
    height: "{spacing.control-height}"
    padding: "0 9px"
  button-primary-hover:
    backgroundColor: "{colors.dark-blue-hover}"
  button-ghost:
    textColor: "{colors.dark-text-2}"
    rounded: "{rounded.sm}"
    height: "{spacing.control-height}"
    padding: "0 9px"
  button-icon:
    textColor: "{colors.dark-text-2}"
    rounded: "{rounded.sm}"
    height: "{spacing.control-height}"
    padding: "0 6px"
  button-danger:
    textColor: "{colors.dark-red}"
    rounded: "{rounded.sm}"
    height: "{spacing.control-height}"
    padding: "0 9px"
  button-danger-hover:
    backgroundColor: "{colors.dark-red}"
    textColor: "{colors.dark-on-red}"
  button-disabled:
    textColor: "{colors.dark-text-3}"
  chip:
    textColor: "{colors.dark-text-2}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.sm}"
    height: "22px"
    padding: "0 8px"
  kbd:
    textColor: "{colors.dark-text-3}"
    typography: "{typography.mono-kbd}"
    rounded: "{rounded.xs}"
    height: "16px"
    padding: "2px 4px"
  result-tab:
    textColor: "{colors.dark-text-2}"
    typography: "{typography.body}"
    height: "{spacing.panel-header-height}"
    padding: "0 12px"
  result-tab-active:
    backgroundColor: "{colors.dark-raised}"
    textColor: "{colors.dark-text}"
  result-toolbar:
    backgroundColor: "{colors.dark-raised}"
    height: "{spacing.result-toolbar-height}"
    padding: "0 10px 0 12px"
  query-box:
    backgroundColor: "{colors.dark-panel}"
    textColor: "{colors.dark-text}"
    typography: "{typography.code}"
    rounded: "{rounded.sm}"
    height: "{spacing.control-height}"
    padding: "0 8px"
  grid-header:
    backgroundColor: "{colors.dark-head}"
    textColor: "{colors.dark-text}"
    typography: "{typography.body-strong}"
    height: "{spacing.grid-header-height}"
    padding: "0 10px"
  grid-cell:
    textColor: "{colors.dark-text}"
    typography: "{typography.body}"
    height: "{spacing.grid-row-height}"
    padding: "0 10px"
  grid-cell-in-range:
    backgroundColor: "{colors.dark-blue-tint}"
  grid-cell-active:
    backgroundColor: "{colors.dark-blue-tint-strong}"
  grid-cell-pending:
    backgroundColor: "{colors.dark-amber-tint}"
    textColor: "{colors.dark-text}"
  grid-row-number:
    backgroundColor: "{colors.dark-gutter}"
    textColor: "{colors.dark-text-3}"
    typography: "{typography.micro}"
    width: "46px"
  inspector:
    backgroundColor: "{colors.dark-gutter}"
    width: "{spacing.inspector-width}"
  inspector-field:
    textColor: "{colors.dark-text}"
    typography: "{typography.body}"
    padding: "6px 12px 7px"
  inspector-field-pending:
    backgroundColor: "{colors.dark-amber-tint}"
  inspector-stat:
    textColor: "{colors.dark-text}"
    typography: "{typography.stat-value}"
  dml-preview:
    backgroundColor: "{colors.dark-panel}"
    textColor: "{colors.dark-syn-identifier}"
    typography: "{typography.mono-data}"
    rounded: "{rounded.sm}"
    padding: "8px 10px"
  next-page-strip:
    backgroundColor: "{colors.dark-raised}"
    textColor: "{colors.dark-text-2}"
    typography: "{typography.caption}"
    height: "{spacing.strip-height}"
    padding: "0 10px 0 12px"
  status-bar:
    backgroundColor: "{colors.dark-ground}"
    textColor: "{colors.dark-text-2}"
    typography: "{typography.caption}"
    height: "{spacing.statusbar-height}"
  status-segment:
    padding: "0 10px"
  status-segment-production:
    backgroundColor: "{colors.dark-red-tint}"
    textColor: "{colors.dark-red}"
    padding: "0 10px 0 12px"
  splash-node:
    backgroundColor: "{colors.dark-panel}"
    textColor: "{colors.dark-text}"
    typography: "{typography.body-strong}"
    rounded: "{rounded.sm}"
    width: "168px"
    height: "46px"
    padding: "0 12px"
  splash-intro-strip:
    backgroundColor: "{colors.dark-panel}"
    textColor: "{colors.dark-text}"
    typography: "{typography.mono-counter}"
    rounded: "{rounded.sm}"
    width: "640px"
    height: "52px"
    padding: "0 12px"
  splash-boot-panel:
    backgroundColor: "{colors.dark-panel}"
    rounded: "{rounded.md}"
    width: "640px"
  splash-log-line:
    textColor: "{colors.dark-text}"
    typography: "{typography.body}"
    height: "22px"
    padding: "0 14px 0 12px"
---

# Design System: Hacivat

## Overview

**Creative North Star: "The Graphite Console"**

Hacivat is a dense, quiet working surface for people who spend hours inside databases and sometimes touch production. The ground is graphite, panels are one step lighter, and everything is separated by single-pixel seams rather than cards, gaps or floating chrome. Colour is not decoration: every hue on screen answers a question (where am I, can I write, is the path direct, is something pending, what is selected). Production is the one signal allowed to shout, and it shouts exactly once, in the title-bar capsule.

Density is deliberate. Rows are 21 to 23px, toolbars 24 to 34px, the type ramp lives between 11 and 15px, and all numbers sit on tabular figures so columns of data align without effort. Source Sans 3 carries the interface; Fragment Mono carries code, identifiers, data types and measurements. Section labels use true small caps, never uppercase-with-tracking on body text.

The light theme is a calm macOS white rather than an inversion of the dark one: white content panes, #F7F7F8 chrome, near-black ink, and deeper, more saturated semantic hues that hold contrast on white. The production capsule fill is identical in both themes so the signal never changes weight when the theme does. Shadow-theatre heritage appears only in the product name and the logo; the logo in the build is an explicit placeholder (dashed frame with an H monogram) and is not part of the system.

**Key Characteristics:**
- Graphite tiles separated by 1px seams; no cards, no floating chrome except popovers.
- Hue reserved for meaning: red production, green direct path and success, amber pending, blue selection and focus.
- Exactly one filled production badge per window.
- Source Sans 3 + Fragment Mono, tabular numerals everywhere, small-caps section labels.
- Radii of 3, 4 and 6px on controls and surfaces; tiny marks are square; elevation reserved for popovers.
- Lucide icons at 1.6 stroke, 12px in chrome, 14px default.
- No ambient motion in the workspace; one choreographed connect sequence on the splash.

## Colors

A neutral graphite (or quiet white) field where four semantic hues carry all the meaning and nothing else is tinted. Every role exists in both themes under the same name with a `dark-` or `light-` prefix; components below cite the dark value and resolve to the light sibling in light theme.

### Primary
- **Signal Blue** (dark-blue / light-blue): selection, focus and the one primary action per toolbar (Run, Submit). blue-hover is the primary button's hover fill (lighter in dark, deeper in light). Focus outlines, the selection box around a cell range, the active gutter range, the caret, tree selection edge and sort indicators. Its tints (blue-tint, blue-tint-strong) fill selected rows, in-range cells, the active cell and the active completion item.

### Secondary
- **Production Red** (dark-red / light-red): production in its small forms: 7px swatches, lock icons on read-only production, the 1px hairline across the top of a production editor, the destructive button outline (red-line) and its focused fill, the label of a destructive menu item (Delete…), and the tinted production segment of the status bar (red-tint).
- **Capsule Fill** (cp-fill with on-cp-fill, both themes): the saturated red block and white small-caps text inside the title-bar capsule. Reserved for that one element.

### Tertiary
- **Direct-Path Green** (dark-green / light-green): the direct P2P path icon, executed-statement checks, "next page ready", completed splash steps. green-fill marks prefetched pages in the page strip and the scroll track.
- **Pending Amber** (dark-amber / light-amber): uncommitted edits. Pending cells (amber-tint fill with a 1px amber inner stroke), the row-number edge, the pending stamp in tabs and toolbars, new values in the inspector.

### Neutral
- **Graphite Ground / Light Chrome** (ground): window ground, title bar, status bar.
- **Panel** (panel, side, head): panes, explorer, panel headers. Equal in dark; in light the side and head are #F7F7F8 chrome around white content.
- **Gutter** (gutter): editor gutter, grid row-number column, inspector, scroll track.
- **Raised** (raised): active tabs, editor and result toolbars, next-page strip, popover body.
- **Hover** (hover): pointer hover on rows, tabs, ghost buttons, status segments.
- **Seams** (seam, seam-strong, seam-soft): seam separates every tile and region; seam-strong outlines controls, the capsule and popovers; seam-soft draws grid cell lines and inlay backgrounds.
- **Ink** (text, text-2, text-3): primary text, secondary labels and inactive tabs, tertiary metadata, placeholders and key hints. Every alpha tint is built on its own solid token, in both themes.

### Syntax
Keyword (lilac / indigo), function (cyan / teal), string (green), number (amber / burnt orange), comment (dim slate, italic), operator (text-2 tone), identifier (text), alias (slightly brighter than text in dark; weight 500 in light).

### Named Rules
**The One Filled Badge Rule.** Exactly one filled production badge exists per window: the cp-fill block in the title-bar capsule. Every other production occurrence is a 7px swatch, a lock icon, a 1px hairline, a tinted status segment, or plain red text.

**The Hue Is Meaning Rule.** Red, green, amber and blue are never used decoratively. If an element is coloured, it must answer "which environment", "which path", "what is pending", "what is selected" or "which action destroys something" (red, for destructive actions only).

**The Raised Ink Rule.** Tertiary ink (text-3) never sits on the raised surface: dark text-3 reaches only 4.18:1 on raised #252931. Metadata on raised toolbars, active tabs, the next-page strip and popovers uses text-2. Disabled items are exempt: a disabled menu item or control stays text-3 on raised surfaces (disabled controls carry no contrast requirement); the disabled state must also be conveyed by non-interaction.

**The Same Weight Rule.** The capsule fill and its white text are identical in dark and light themes.

## Typography

**Display Font:** none; the interface has no display tier.
**Body Font:** Source Sans 3 (with system-ui, sans-serif)
**Label/Mono Font:** Fragment Mono (with ui-monospace, monospace)

**Character:** A humanist sans that stays readable at 11 to 13px, paired with a narrow, slightly warm mono for everything a user might copy: SQL, identifiers, types, row counts, timings.

### Hierarchy
- **Panel title** (600, 13.5px, small caps, 0.075em): panel and inspector headers (Explorer, Record, Selected range, DML preview).
- **Label small caps** (600, 13px, 0.075em): explorer group headers and the NULL marker (0.08em).
- **Capsule env** (700, 13.5px, small caps, 0.09em): the production word inside the capsule only.
- **Stat value** (600, 15px, -0.005em): live sum/avg/count values in the selection inspector, the largest text in the workspace.
- **Body** (400 / 600, 13px, 1.35): tree rows, tabs, grid cells, splash log messages.
- **Control / body-sm** (500 / 400, 12.5px): button labels; connection sheet, breadcrumbs, chips.
- **Caption / micro** (400, 12px / 11.5px): status bar, panel meta, next-page strip, counts, completion type labels.
- **Code** (Fragment Mono 400, 12.5px / 18px): editor lines and gutter, completion names. Comments italic.
- **Mono data / micro / kbd** (Fragment Mono 11.5px, 11px, 11px): DML preview and identifiers in prose; status and tree meta, inspector field keys; key hints and grid column-type subtitles.
- **Inlay** (Source Sans 3 500, 11px / 14px): inline type hints in the editor.
- **Mono counter** (Fragment Mono 400, 15px / 18px): the splash introspection counts.

### Named Rules
**The Tabular Everywhere Rule.** Every text run uses tabular figures. Numbers in quantities and measures are formatted by the UI locale (en-US: 1,240.00; tr-TR: 1.240,00), right-aligned in grid and inspector. Identifiers and keys (ids, foreign keys) render raw. Copy and export always use raw, unformatted values.

**The Mono Means Copyable Rule.** Fragment Mono marks text that is literal: SQL, identifiers, types, connection names, timings and counts. Prose and UI labels stay in Source Sans 3.

**The 11px Floor Rule.** No text renders below 11px, including key hints, inlays and column-type subtitles.

**The Crisp Text Rule.** Text is drawn without platform stroke dilation, matching the approved mock (antialiased, thin). On macOS the app sets its own `AppleFontSmoothing` preference to 0 at startup. A user setting can switch to the heavier platform-standard rendering (PO decision, M0, 2026-11-12).

**The Synthesized Small Caps Rule.** Section labels are small caps at weight 600 with 0.075em tracking, drawn as pre-cased capitals at 0.7× size. They are never uppercased body text at full size.

**PO decision (M0, 2026-11-12): small caps are synthesized, like the approved mock.** Tokens marked `fontVariantCaps: synthesized` render pre-cased capitals at 0.7× size with the token's tracking (`hacivat_ui_core::fonts::small_caps`), not the font's `smcp`/`c2sc`; casing (including Turkish İ) happens upstream, never in the renderer.

## Layout

A 1440 x 900 reference window on a three-row grid: 40px title bar, tiled workspace, 24px status bar. Tiles are separated by a 1px gap that shows the seam colour beneath, so regions read as cut from one sheet. Left column: 276px explorer over a 205px connection panel. Centre column: 322px editor over the results pane; results hold the grid and a 252px right inspector. Every panel opens with a 30px header; editor and result toolbars are 32px and 34px on the raised tone; the next-page strip is 26px.

Spacing steps are 2, 4, 6, 8, 10, 12, 14 and 16px; horizontal padding at panel edges is 12px on the left and 10px on the right. Tree depth indents 14px per level from an 8px base. Destructive actions are isolated by 28px of empty space plus a seam before them.

**The Status Follows The Tab Rule.** The status bar's environment, access and rows/timing segments, the result toolbar, the inspector and the next-page strip all follow the active result tab. Switching from a production result to a staging result swaps them together; the title-bar capsule continues to describe the editor's connection.

**The Overflow Rule.** Toolbars collapse trailing actions into an overflow menu when width runs short, before any label truncates or wraps. Turkish strings fit the 1440px reference with about 3px to spare, so any narrower window or longer locale depends on this.

## Elevation & Depth

Flat by default. Depth comes from tone steps (ground, panel, raised) and 1px seams. The only shadow in the workspace is the popover shadow, used on the completion popup. Selection and pending states use tinted fills and 1px inset outlines, not lift. Traffic lights and hollow swatches are drawn with borders, never shadows; the primary button's hover is a fill change.

### Shadow Vocabulary
- **Popover (dark)** (`0 10px 28px rgba(0,0,0,0.34), 0 1px 2px rgba(0,0,0,0.30)`): completion popup and any future popover, menu or tooltip.
- **Popover (light)** (`0 12px 32px -6px rgba(20,20,40,0.20), 0 0 0 1px rgba(20,20,40,0.04), 0 1px 2px rgba(20,20,40,0.08)`): same role in light theme.

### Named Rules
**The Popover-Only Shadow Rule.** Only floating surfaces that appear above the workspace cast a shadow. Panels, toolbars, tabs and buttons never do.

## Shapes

Square-cornered tiles with small, consistent radii on controls: 3px on key hints, inlays and completion items; 4px on every control (buttons, fields, capsule, segmented controls, chips, DML preview); 6px on floating surfaces (completion popup, splash boot panel). Tiny marks are square (0px): environment swatches, pending stamps, the splash run marker and the prefetched band in the scroll track, all at 7px where they mark state. The scroll thumb and page strip take 3px. The local environment swatch is a hollow circle. Borders are always 1px. Selected rows and tabs draw a 1px edge (blue for tree selection; tab bottom continues into the raised toolbar).

Environment marks are shape-coded as well as colour-coded: production is a filled red square, staging a hollow square (1px text-2 border), local a hollow circle (1px text-3 border).

## Components

All icons are Lucide at 1.6 stroke width: 12px inside chrome and controls, 14px as the default size. Icons sit at text-2 unless they carry meaning (lock red, path green, selection blue).

### Buttons
- **Shape:** gently squared (4px), 24px tall, 9px horizontal padding, 6px icon gap, 12.5px weight 500 label; key hints trail in 11px mono at text-3.
- **Default:** transparent with a seam-strong outline and text ink; hover fills with hover.
- **Primary:** blue fill, on-accent text at weight 600; one per toolbar. Hover swaps fill and border to blue-hover.
- **Ghost:** no outline, text-2 ink, text on hover. **Icon:** 6px horizontal padding.
- **Danger:** red-line outline and red text at rest, isolated by 28px plus a seam; fills red with on-red text only on hover or keyboard focus.
<!-- ## m3c-t43b (fence added by m3c-t43b-fix, review m8): the Disabled row below was edited in place, "text-3 ink" → "text-disabled ink", what has shipped since Task 41. -->
- **Disabled:** text-disabled ink (the M3c disabled token, below) with the seam-strong outline kept, no fill, no hover response; danger and ghost disabled lose their colour and outline respectively.
- **Focus:** 1px blue outline at 1px offset on every focusable element.

### Title-Bar Capsule (signature)
A single 26px segmented capsule anchored left after the brand: cp-fill production block in capsule-env type, then connection name (600) with engine at text-2, access (red lock + "read-only"), path (green arrows + "Direct (P2P)"), and a chevron. v1 shows the path as "Direct"; "(P2P)" appears only with Perde/Karagöz paths. Segments are divided by seam-strong; hover tints every segment except the production block. The production label is "PRODUCTION" in English and "PROD" in Turkish, so the block keeps its width across locales. The splash assembles the same capsule and docks it into this slot.

### Explorer Tree
- **Filter field:** 28px, 4px radius, search icon, kbd hint.
- **Group header:** small-caps label at text-3 followed by a seam-soft rule and a mono count.
- **Rows:** 23px, chevron + type-specific icon + label, environment swatch and lock/path flags after the label, mono count or engine meta right-aligned. Hover fills hover; selection fills blue-tint with a 1px blue left edge and a blue icon.

### Connection Panel
Mono connection name in the header, then 22px key/value rows on a 92px key column: environment, access with role, path with latency, agent, engine, grantor, audit.

### Editor
- **Tabs:** 30px, mono file name, environment swatch on production tabs, unsaved dot, close icon. Active tab takes the raised tone and joins the toolbar below.
- **Production hairline:** a 1px red line across the top of an editor bound to production.
- **Toolbar:** raised 32px bar with Run (primary), Explain, Format (ghost), a seam-led breadcrumb of connection, database and schema in mono, and auto-commit and limit chips.
- **Gutter:** 58px, line numbers at text-3 (current line at text), green check on executed statements, a 1px green range bar per executed statement and a blue bar on the current statement. End-of-line status shows "executed", timing and row count.
- **Completion popup:** 300px list plus 236px detail pane on raised, 6px radius, seam-strong border, popover shadow. Items are 23px with kind icon, mono name with the typed prefix underlined in blue, and type at text-2; out-of-scope items dim and italicise their type. Active item uses blue-tint-strong. A 24px footer lists key hints and the match count. It expands along Y only.

### Results
- **Result tabs:** same anatomy as editor tabs; staging tabs carry the hollow swatch, "(staging)" and an amber pending stamp with count.
- **Toolbar:** WHERE and ORDER BY query boxes (mono, keyword in syntax keyword colour, blue border on focus), pager with locale-formatted range, Count rows, Copy, Export, then the isolated danger action. On a writable target it shows a target chip, pending marker, DML preview icon, Discard and a primary Submit.
- **Grid:** 36px headers (name 600 plus mono type subtitle at text-3, blue sort index), 21px rows with seam-soft lines, 46px gutter-toned row numbers, numbers right-aligned. NULL renders as small caps at text-3; empty strings as mono `''`. Cell range: blue-tint fill, active cell blue-tint-strong, 1px blue selection box with a 5px square handle. Cell hover: 1px seam-strong inset outline. Pending cell: amber-tint, 1px amber inset outline, old value struck through at text-3; small-caps amber "edited" stamp in the trailing column.
- **Inspector:** 252px on gutter tone. Record fields show mono key and type above the value; pending fields take amber-tint with old value struck and new value amber 600. Selected range shows a two-column grid of live stat values. On writable results a DML preview (mono, syntax-coloured, 4px radius, panel tone) and a note naming the target replace the stats.
- **Next-page strip:** 26px raised strip with a page map (6px cells in a 3px-radius frame, current blue, prefetched green-fill), green "Next page ready", prefetched range, and selection dimensions with a locale-formatted sum.

### Status Bar
24px on ground, 12px text-2, segments divided by seams. Left: environment segment (production: red-tint fill, red 600 text with swatch and lock; staging: plain text with hollow swatch and mono target), path with latency and agent, auto-commit, rows and timing. Right: cursor position, encoding, engine, language toggle. Segments that are buttons take hover.

### Segmented Controls
26px groups with seam-strong outline and dividers; pressed segment on raised (panel in light) with text ink. Language segments use 11.5px mono.

### Splash (signature)
A 3.42s connect sequence that explains the trust model, then hands off into the workspace. Everything sits on the ground with no shadow: the placeholder mark and version at top; a path diagram of three 168 x 46px nodes (Hacivat "this Mac", Perde rendezvous, Karagöz agent) on panel with 4px radius, joined by 1px wires; a 640 x 52px introspection strip of six seam-divided counters (label caption at text-3, value in 15px mono that steps text-3, text-2, text); the capsule; and a 640px step log (6px radius, 31px header with the 7px production swatch, target, "opening" and a mono clock). Log rows are 22px: mono offset, status (7px square amber run marker, then green check), message with mono detail revealed on completion, duration. A 1px progress seam fills along the log's bottom edge in text-3 and turns green when ready. "Press any key to skip" sits below; skipping jumps to the assembled capsule and docks it. No filled badge is visible in the title bar until the docked capsule replaces the splash copy, so one filled badge holds at every frame.

The sequence, in order: (1) signaling: the route through Perde draws in 560ms with a 5px pulse travelling it, and Perde and the agent brighten as it arrives. (2) Hole punching: both peers draw half-lines toward each other (400ms); on meeting, the direct line commits to 1.5px green, the "direct (P2P) 18 ms" label rises 4px into place, and the signaling route recedes to 40%. (3) Authorize: the role and "read-only" label settles under the agent with a red lock. (4) Introspection: three 4px green data pulses travel the direct line while the kinds count up, 80ms apart. (5) The capsule assembles piece by piece (production, name, access, path; 80ms stagger, 6px slide). (6) Hand-off: the capsule docks into the title-bar slot on a short arc (x on ease-out, y on ease-in-out, 560ms), the stage fades, the workspace seams draw from their ends, and the panes rise 8px into place 120ms apart.

### Motion
The workspace has no ambient motion; tab, popover and selection changes are instant. Any transition added there uses the state duration (240ms) or shorter with the ease-out curve. Motion vocabulary: commit 140ms (checks, colour commits, exits), state 240ms (fades, labels, capsule pieces, panes), travel 400ms (hole punch, data pulses, counters, seams), focal 560ms (signaling route, capsule hand-off); stagger 80ms within one element and 120ms between panes; ease-out (0.16, 1, 0.3, 1) for arrivals and fades, ease-in-out (0.65, 0, 0.35, 1) for line draws and travel. Reduced motion lands every step as its end state through a 240ms crossfade: no draws, pulses, slides or docking travel.

**The Pure Function Of Time Rule.** The splash is a cue sheet: every property is computed from elapsed time as clamp((t - cue) / duration) through a named easing, so play, skip and freeze all render the same frame. Port one animation per element with that delta.

**GPUI porting notes.** GPUI has no general transforms. Animate absolute offsets for slides, the capsule dock and pane arrivals; animate quad width or height for the progress seam and the workspace seams that draw from their ends; animate opacity for fades. Line draws (signaling route, hole-punch halves) are computed as truncated polylines: walk the segment list to the current length and emit the partial path each frame, rather than relying on dash offsets.

### Connection Flow (M2a)
The connection dialog, its errors and the live capsule states (design/m2-connect/NOTES.md, PO answers 2026-11-12). Every primitive here uses the existing tokens only: solid fills, 1px borders, radii 0/3/4/6, the popover shadow only, no transforms, no blur.

- **Dialog sizes:** the existing Dialog anatomy (raised, 1px seam-strong, 6px radius, popover shadow, 16px padding) in three widths: 420 (alerts, confirmations), 480 "wide" for technical content (connect errors, the production prompt, the password prompt, pg_service import), 640 "form" for the connection dialog. The form variant has a header (padding 12/16, seam below), a scrolling body (padding 12/16/14) and a footer (padding 12/16, seam above). The test strip sits between body and footer so it shows at any scroll position. At 1024 x 700 the form dialog keeps its width and its body scrolls (Overflow Rule).
- **Scrim below the title bar:** ground at alpha 0.6, from y = 40px (titlebar-height) to the window bottom, so the capsule (environment, connection, state) stays readable behind every dialog. Title-bar controls are inert while a modal is open.
- **Raised Ink Rule in dialogs:** text-3 maps to text-2 inside a dialog (labels, hints, key hints, the close and dismiss icons). Field wells (TextInput, Select, Segmented, inset boxes, inline notices) restore text-3 on their own panel (dark) / ground (light) fill. Disabled items keep text-3.
- **Segmented in a field well:** the well is panel in dark and ground in light; the pressed segment is raised in dark and panel in light, with its label at weight 600.
- **Environment marks:** 7px, shape-coded, one hue. Production a filled red square, staging a hollow square (1px text-2), local a hollow circle (1px text-3; text-2 on raised), dev a hollow circle (1px text-2) with a 3px text-2 centre dot, test a filled text-2 circle. The same mark appears wherever an environment does: Segmented, explorer group and row, capsule, status bar, sheet, prompt. A mark is never a badge: the One Filled Badge Rule counts only the capsule's cp-fill block.
- **Checkbox:** 14 x 14, 3px radius, 1px seam-strong, fill panel (dark) / ground (light). Checked: fill and border blue, Lucide check at 12px stroke 2.2 in on-accent. Label 13px text, optional sub-label text-2. Focus: 1px blue ring at 1px offset.
- **Select:** a field-shaped button: 26px field-height, 4px radius, 1px seam-strong, padding 0 6 0 9, fill panel / ground, value 13px text, chevrons-up-down at 12px text-2. It opens the existing popup menu (raised, 6px radius, 24px rows, 200px minimum) with a blue check column and an optional text-2 hint. Space, ↓ or ↵ open it; ↑/↓ move, ↵ picks, Esc closes. A disabled option keeps its reason as the hint.
- **Disclosure:** a 32px row with a 1px seam above; chevron right or down (swapped, never rotated) at 12px text-2; title 13px weight 600 text; the live summary right-aligned at 12.5px text-2 with an ellipsis. Content is indented 22px.
- **Inline notice:** padding 8/10, 4px radius, 1px seam-strong, fill panel / ground, 12.5px text with secondary lines in text-2 and parameter names in Fragment Mono 11.5. The icon is 14px text-2 (info), or a green circle-check when it reports something done. Actions are existing Buttons. Prose wraps to the notice's width.
- **Step trail:** an inline list, 20px tall, 14px column gap, wrapping without separators. Done: check 12px green, label text, mono 11px duration text-2. Running: the 7px square amber run marker. Failed: ✕ 12px text with the label at weight 600. Pending: a 7px hollow seam-strong square with the label text-2. Each step announces its state in words (Fluent), never by colour alone.
- **Choice list (radio rows):** a group with a 1px seam-strong border and 4px radius, rows divided by seam. Rows are at least 34px, padding 7/10: a 14px radio (1px seam-strong; selected: blue border and a 6px blue dot), the 7px environment mark, the label at 13px weight 600 and a description at 12.5px text-2. Selected row blue-tint, hover hover. Arrow keys move and select.
- **Sticky toast:** the existing Toast (320px; 380px when wide) with no TTL and Role::Alert: a 600 title, body lines in text-2, a list with 3px square text-2 bullets, and a row of Buttons. It takes the keyboard focus only on request (F6); Escape dismisses it and returns the focus.
- **Explanation popover:** the popover surface (raised, seam-strong, 6px radius, popover shadow), 300px wide, padding 10/12; a 13px weight 600 title after a 12px icon in text; body paragraphs at 12.5px text-2 that wrap to the card. Opened from the "unencrypted" capsule segment or its command; Escape returns the focus to the trigger.
- **Capsule states:** extra segments in the existing 26px capsule anatomy, divided by seam-strong. The production block always comes first, in every state. Connecting: the 7px amber run marker, "Connecting" in text, then the step and a running clock in mono 11.5 text-2 (44px minimum so the capsule does not jump). Connected: access (a lock, red only on production; "read-write" in text-2 otherwise), path (green arrows + "Direct" + latency in mono), an "unencrypted" segment (lock-open 12px text-2, label text) where plaintext was allowed, and an "unsupported" mark (circle-alert text-2) for a server below the supported floor. Failed: triangle-alert 12px text + "Couldn't connect", then Retry (rotate-cw text-2, text on hover); the body opens the error dialog. A non-production tag segment shows the mark and the tag in text-2.
- **Errors without a new hue:** a failure is an ink-coloured alert glyph, the trail's ✕ and plain words; red stays for production and destructive actions (Delete… in a menu is red text; the delete confirmation's Danger button is outline-only until hover or focus, with the initial focus on Cancel). Server text sits verbatim in its own mono field under "Technical details", never in the heading.
- **Production prompt:** calm, no warning icon. With Production chosen, the consequence list names the capsule word in red small caps beside the swatch (never a second filled block); any other choice shows one calm line. The primary names the result ("Save as production"), disabled as "Pick an environment" until a choice is made.
- **Long locales in the form:** nothing runs past the 640px card. A Segmented in a field well shrinks to its row, but each segment keeps at least 44px; when the row cannot show every label whole, the Environment control becomes a 200px Select ("Choose…", then local … production) rather than five ellipsized segments (the Overflow Rule). A checkbox sub-label ellipsizes; when the footer buttons do not fit one line, Cancel / Save / Save and connect wrap under Test, still right-aligned. Prose (dialog bodies, notices, toast titles and lines, popover titles and paragraphs, empty states) wraps; one-line values in key/value rows (the sheet, the test diagnostics) ellipsize.
- **Key hints on a Primary button** take the button's on-accent ink on every surface (text-3 on the blue fill is unreadable); other buttons keep text-3, or text-2 on raised.
- **Explorer rows:** when a row is crowded, its meta (live mark, latency, engine) ellipsizes first; the name gives way last and keeps at least about six characters and the ellipsis.
- **Minimum window:** at 960 x 600 the explorer, the sheet and the title bar fit without clipping: the explorer list and the sheet scroll (the sheet shows a 4px seam-strong scroll thumb, 3px radius, while its rows overflow), and the palette field shrinks and ellipsizes before any segment collapses.

### Secure Network and Sessions (M2b)
TLS review, SSH, the vault and live sessions (design/m2b-secure/NOTES.md), as shipped. **No new hue and no new filled block:** every component below uses the existing tokens for their existing meanings. Green is only a check that something succeeded, red only production and destroying data (the vault reset), amber only a run in progress; a mismatch, a rejected answer or a failed hop is an ink glyph. The capsule's cp-fill stays the window's one filled badge. Primitives keep the M2a rules: solid fills, 1px borders, radii 3/4/6, the popover shadow only.

- **Dialog anatomy (prompts):** the 480 "wide" dialog of M2a (the reset confirmation is the 420 one): a header with the title at 13px weight 600 text and the close icon; a subtitle at 12.5px text-2 that names the asker (the 7px environment mark, the connection in Fragment Mono 11.5, the hop as "host, hop n of m"); form rows on a 120px label column (labels 13px text-2, Raised Ink Rule); a footer with the trust-widening action alone on the far left and Cancel / the primary on the right. Masked fields are 240px TextInputs that never copy, cut or expose their value.
- **Safe default, roles:** hard stops (host key changed, pin changed, vault reset, vault damaged) are alertdialogs whose primary is Cancel or Close with the initial focus, so Enter and Escape both stop; the trust-widening action ("Remove the old key and trust the new one" with the 12px key-round icon, "Replace pin" with the fingerprint icon) is a Default (outline) button, never Danger and never the Enter default. The plaintext question is an alertdialog too (a security question), whose default is the safe answer, "Use an SSH tunnel…". The other prompts (new host key, keyboard-interactive, SSH secret, vault create and unlock) are dialogs; the new-host-key prompt also opens on Cancel (it arrives mid-connect). Every dialog completes from the keyboard: Tab is trapped in the card, Enter acts on key-down only (never held, never with a modifier), Escape cancels.
- **Fingerprint changed (key comparison):** two fingerprint insets 8px apart, labelled by fact ("Stored key" / "Offered now", "Pinned" / "Now") and never by hue: panel (dark) / ground (light) fill, 1px seam-strong, 4px radius, padding 8/10; a header with the label at 12.5px text-2, the algorithm in Fragment Mono 11.5 text and a ghost Copy; the `SHA256:` or hex fingerprint in Fragment Mono 11.5/17 text, two lines of 16 bytes for certificates. The stored inset ends with a seam and "Stored in" (12px text-2 + mono). An OpenSSH-stored key gets no replace button: a command inset (the same inset, the `ssh-keygen -R` command in mono, Copy command) and Try again (rotate-cw) instead.
- **Hop route:** the route row of the SSH section and the SSH secret prompt: this computer (12px text-2), then each hop and the database in Fragment Mono 11.5, every stop after a 12px arrow-right icon in text-2 that travels with it (a wrapped route never ends a line on an arrow). The asking hop is boxed: 1px seam-strong, 3px radius, padding 0/4, text ink; other stops text-2.
- **SSH section:** the M2a Disclosure; folded with the tunnel on, its summary leads with the 12px route glyph in text-2. Open: the checkbox, Import from `~/.ssh/config…` (Default button, 12px file-up icon) whose menu opens under the button with its right edge on the button's (above it when the dialog body has no room below), so it stays inside the card; it has a title row ("Hosts in ~/.ssh/config", 12px text-2, padding 4/10) and the aliases in Fragment Mono 11.5 with their resolution as a text-2 hint (an unusable entry is disabled with its reason); the sign-in method in a Segmented; the agent and keyboard-interactive notes (key-round / info glyph at 12px text-2, text that wraps).
- **Keyboard-interactive (server-text inset):** the fingerprint inset at most 160px tall: the caption "Sent by the server, shown as received" at 12px text-2 stays on top; the server's name (mono 11.5 text) and instruction (mono 11.5 text-2) scroll on their own up to 119px with a 4px seam-strong thumb (3px radius, no track). Backslashes, controls and bidi marks are shown escaped. Each prompt is its verbatim mono label above a full-width field (masked unless the server allows echo). A meta line at 12px text-2 says the attempt and that answers are not kept; a rejected round leads with the ink alert glyph (12px, text) and its sentence in text.
- **SSH secret prompt:** the M2a password prompt per hop: Route (the hop route, asking hop boxed), SSH user or Key file as a static mono value, the masked field, then Remember (a 240px Select) for SSH passwords and key passphrases. A miss puts the ink alert line and "Attempt n of 3" under the field.
- **Vault dialogs:** Create (480): the reason ("This computer has no system keyring.") at 13px text, the vault file as a static mono value, Master password and Repeat (240px masked), two validation lines (12px rows 3px apart: a 7px hollow seam-strong square while unmet, a 12px green check once met, a 12px ink ✕ with text ink on a mismatch), an info inline notice for "can't be recovered", and Not now (ghost) / Create vault (primary, disabled until both rules hold). Unlock (480): one masked field, Reset vault… (ghost, far left, shown only when the file may be reset), Cancel / Unlock. Reset (420 alertdialog, stacked over Unlock): the typed word in a mono field, Reset vault as the Danger button (trash icon), disabled until the field matches exactly and never pressed by Enter. Damaged (480 alertdialog): Reset vault… (outline, far left) and Close (primary, focused).
- **Reconnecting capsule:** the capsule's run state: the 7px amber run marker, "Reconnecting · attempt n" in text, and between attempts the countdown ("in 4 s") in Fragment Mono 11.5 text-2, which repaints once a second. The production block still comes first; outside production the tag segment keeps its mark in text-2. The status bar says the same attempt. At the 960px minimum every capsule state collapses in a fixed order to its glyph with the full text as its tooltip and accessible name, and the capsule is clamped to its room: the palette field (never under 170px) and the trailing controls never move.
- **Idle:** the capsule keeps the environment and name and adds the 12px unplug glyph and "Disconnected (idle)" in text-2; the explorer row and the status bar say so too; a 320px toast with the unplug icon names the threshold in hours or minutes.
- **Lost toast:** the M2a sticky toast, wide (380px), no TTL: the rotate-cw icon when Hacivat reconnected, unplug when it did not; the title at weight 600 ("Reconnected to orders-dev"), one body line in text-2 that says whether a transaction was rolled back (only when one was open), the lost session state as one locale list (temporary tables, variables, prepared statements, locks; 3px square text-2 bullets), and Show details (selects the connection) / Dismiss. One per profile; a newer one replaces it.
- **Gap alert:** the same sticky toast, urgent (never queued behind session toasts), with the ink triangle-alert glyph, never a hue; its line depends on the reason.
- **Read-only access:** the capsule's access segment is a Tab stop: a 12px lock (red only on production, text-2 elsewhere) and "read-only (enforced by server role)" or "read-only (guarded)", focus ring 1px blue inset 2px; the sheet's Access value is a borderless button with the short form ("read-only · enforced / guarded"), underlined on hover. Both say whether their popover is open (`aria-expanded`), as the Disclosure and the Import button do.
- **Read-only popover:** the explanation popover (raised, seam-strong, 6px radius, popover shadow), 300px wide, padding 10/12; title at 13px weight 600 after the lock (red on production, text elsewhere); body at 12.5px text-2; the findings list (a 12.5px text-2 lead, then 12px rows 2px apart with 3px square text-2 bullets, literals in mono 11.5); the SQL inset (fingerprint-inset anatomy, one Fragment Mono 11.5/17 block per line 4px apart with a 2ch hanging indent; keywords syntax keyword, strings syntax string, placeholders italic text-2, `-- optional:` comments italic syntax comment) and Copy SQL right-aligned under it. The popover never grows past the room between the title bar and the status bar (6px clear of each): the body scrolls with the 4px seam-strong thumb while the title and Copy SQL stay. Escape or a click outside closes it and gives the focus back to its trigger.
- **Errors:** the new classes (`tls.pin_changed`, `auth.insecure_method`, `ssh.*`) use the M2a error dialog unchanged; under SSH the step trail starts with the SSH step, and a hop failure names the hop's host. Server text stays in Technical details.
- **Following the focus:** the connection dialog's body scrolls to the control the focus moves to, by the least amount and 8px clear of its edge, so "Connect through SSH…", "Use an SSH tunnel…" and Tab land on a visible SSH host field at the 960 x 600 minimum. A control the user scrolled away from stays put until the focus moves again.
- **Pinned, vault:** a pinned certificate reads "TLS 1.3 · pinned" in the status bar, the sheet and the test report, never "verified"; on a machine without a keyring the sheet's Password row reads "vault · forever". `tls.not_supported_by_server` offers "Connect through SSH…", which opens the connection's SSH section with the tunnel on.
- **Long locales:** prose in every prompt wraps inside its card (a value column shrinks with the card, so a hint under a Select wraps rather than running past the edge); one-line values ellipsize, a field's placeholder included (it ends in "…" rather than running under the field's edge). The shots check, in English, Turkish and both pseudo-locales at 1280 x 800 and 960 x 600, that each surface's own labels are visible, not scrolled out of the dialog body, and that no text recorded while a dialog card lays out (labels, paragraphs, field text) runs past a side of that card. That containment check does not cover text recorded when the view is built (inline notices, the error dialog's prose, the explorer's paragraphs) nor popovers, menus and toasts, which have no card; those rely on wrapping and on the clip check.

<!-- ## m3c-t43-shots: part A (the merged M3c surfaces, Tasks 31–38 and 40); ## m3c-t43b: part B adds the grid menu (Task 39), the narrow forms, the disabled token, the keyboard-active menu row and the chrome metrics (Task 41, F2 M9) and the settings rows (Task 42). -->
<!-- ## m3c-t43b (fence added by m3c-t43b-fix, review m8): part B rewrote part A's comment above in place (its "Part B adds …" plan became what part B added) and widened the next paragraph's surfaces from 14–22 to 14–25. -->
### Query, Results and Grid
The console's editor, its results and the grid as shipped through M3c (design/m3b-safe/NOTES.md surfaces 14–25; the shots in `tests/shots/m3c.rs` are the reference renders). **No new hue, no new filled block:** blue is the selection, the caret and the current match; amber only a run or a job in progress (the static 7px marker, never a spinner); green only a check that something succeeded; an error, an invalid pattern or a refusal is an ink glyph and words. Red stays production and destruction. Everything reuses the M2/M3a tokens: solid fills, 1px borders, radii 3/4/6, the popover shadow only.

- **Trust signals over the console:** nothing covers the title bar, the console toolbar (crumb, mode chip, transaction badge, read-only chips), the cursor strip or the status bar. The find bars are rows under their toolbar, never boxes over the code; the go-to popover and the value viewer sit below the result toolbar and above the strip; menus stop above the status bar. The shots check every production, transaction, read-only and cursor label is drawn whole in every M3c state that carries one.
- **Carets (14):** every caret is the M3a 2px blue caret, all blinking in phase, no visual "main" caret; each selection is blue-tint-strong (seam-strong unfocused). The status bar's editor group says "3 carets" before the selected count and gives way with it. A column selection is one range per line in the same display columns (a short line gets an empty range at its end). An input method's marked text is mirrored at every caret with the M3a underline when every caret is in plain text; otherwise the other carets collapse first and the status bar says once "Input method: typing at the main caret only…" (polite). Run with several carets runs the main caret's statement only, and the band shows only that one.
- **Folds (15):** the fold column is the gutter's last 14px, right of the line numbers (the gutter keeps its width: 58px for three-digit line numbers, one monospace advance more per extra digit): a 12px chevron in text-2 (text under the pointer), down when open, right when folded; open chevrons show only while the pointer is over the gutter or a gutter button has the focus, a folded header's always. The folded header keeps its text and gets the inlay "⋯ n lines" 8px after it (11/14 Source Sans 3 500, text-2 on seam-soft, light seam, 3px radius, 14px tall, 5px side padding; text under the pointer; click unfolds, its tooltip names the lines) and a 1px seam-soft line under the row; line numbers skip the hidden lines. A mark inside a fold paints on its header, the one that most needs the user winning (awaiting > escalated > failed > running > cancelling > queued > cancelled > rolled back > skipped > done > stale), and a failure says "failed inside the fold · line n". Run at a caret in a fold runs the whole statement; the band covers the header row. A focused chevron draws the 1px blue ring. Every fold is at least three lines; folds have no default chord.
- **Editor find bar (16):** a 32px raised row under the editor toolbar (padding 0 10 0 12, gap 6, Raised Ink Rule). The find field is 24px on the field well (panel / ground, 1px seam-strong, 4px radius) with the 12px search glyph in text-3, the query in Fragment Mono 12, and four 20 x 18 toggles inside the well (Match case, Whole words, Regular expression, Turkish-tolerant; pressed: blue-tint-strong + 1px blue + text). The count is Fragment Mono 11.5 text-2, at least 58px ("3 of 41", "No matches"; an invalid pattern: ink circle-alert + "Invalid pattern", the parser's message in the tooltip, the field border text-2, nothing searched). Then previous / next, Select all occurrences (capped at 10,000: the status bar says "Selected the first 10,000 of n matches") and close. Other matches are a 1px text-3 outline, the current one is the selection. Scrollbar ticks are 2px text-2, the current one blue, shown only while the document is taller than the view; **the track stays 8px** (the mock's 9px would move every M3a marker shot; PO item) and the editor draws no vertical thumb. Turkish-tolerant is on by default when the UI language or region is Turkish. Tests and shots never read or write the macOS find pasteboard (an injected one).
- **Gutter ▶ and line commands (17):** the ▶ is a 16 x 18 button in the mark column of a statement's first row, the play glyph at 12px (`IconSize::Chrome`; the mock's 10px has no icon size) in text-2, text and the hover fill under the pointer, the 1px blue ring when focused. It shows on the hovered row (on every statement row while a gutter button has the focus) only while Run would be enabled, takes a done or failed mark's place while shown, and never covers an awaiting, escalated, running, cancelling or queued mark. Its tooltip (the gutter tooltip, 300px) titles "Run this statement" with the ⌘↵ key, then "Last runs" and up to three rows (outcome glyph, the time in Fragment Mono 11 text-2, the mark's sentence), and "Runs through the same checks as Run."; a statement never run says "Not run yet." The marks gain the awaiting stamp (the 7px amber square), the escalated hourglass (12px amber) and the rolled-back undo glyph (12px text-2, "rolled back"). "Mixed line endings" is a text button in the status bar's editor group; its menu opens above it with the counts header and "Normalize to LF / CRLF", the dominant one first with "most lines". A history recall says "From history: statement 2 of 48 · ⌃⌥↑ older · ⌃⌥↓ newer" in the status bar.
- **Grid model (18):** a sorted header draws a 12px blue arrow after its name (up ascending, down descending), and with two or more keys the level digit in Fragment Mono 11 blue, 6px apart; its tooltip names the level and the collation, or says text compares by code point. Local sorts say so in a note row under the result toolbar (the M3a info row: raised, padding 5/12, 12px text-2, the arrow-up-down glyph or the amber marker, a 20px Default button): "Sorted locally…" + Clear sort, "Sorting n rows…" + Cancel after 200ms, "n new rows at the end (not sorted)" + Re-sort, and both facts in one row for a partial result with a tail. The unsorted tail starts under a 1px dashed text-3 line (3/3 dashes). Frozen columns paint in their own layer with a 1px seam-strong right edge on the last one (no glyph; the header tooltip says "Frozen…"); the rest scrolls under them. Several ranges draw one blue box each (the M3a box and handle), the strip says "Selection 2 ranges · 10 cells". The resize handle is a 1px text-2 line on the hovered header edge; dragging draws a dashed text-2 guide and the width tag ("136 px · Drag to resize · double-click to fit", the popover anatomy with the value in mono, no middle dot before the width; PO item).
- **Grid find and go to row (19, 20):** the grid's find bar replaces the result toolbar's left group inside its 34px row with the editor's field anatomy, a fixed 240px field (the toolbar measures at max content; NOTES' 180–400 range is a PO item) and a fifth toggle, Filter rows (filter glyph); the right group stays. A match outlines the matching text in its cell (1px text-3), the current one blue-tint-strong with a blue outline; a cut value adds "· may be incomplete" with a dotted text-2 underline. Filter rows numbers view rows 1…n and adds the note row "Showing the n rows that match, of the m fetched…" + Show all rows; the cursor strip stays. Go to row is a 300px explanation popover under the toolbar's left edge: the title with the arrow-right glyph (NOTES' list-ordered is not shipped), a 26px mono field, one 12px line (hint, the live target "Row 1,000, column city", or an ink error with the field border text-2; a hidden column adds "Show it and go"), key hints in 11.5px text-2. Rows read in the format locale (`1.000` is row 1000 in Turkish). Past a million unfetched rows it asks in place (Cancel Default, **Fetch and go** Primary and focused, last); fetching shows the amber marker, "Fetching to row n: k of m rows", "Escape hides this; fetching goes on in the strip." and Stop fetching.
- **Views and the value viewer (21, 22):** the view switch is a 22px Segmented (Table / Transposed / Text, 12px glyphs) at the head of the result toolbar's right group; Transposed is disabled above 100,000 rows (focusable, the reason in its tooltip); a narrow toolbar keeps the glyphs. The record toggle is a ghost panel-right icon (pressed: blue-tint-strong + blue border); a one-row result shows it with its word "Record". The record inspector is 252px on the gutter tone with a seam on its left: a 30px head header (small-caps "RECORD", "Row 4 of 184" in mono, ▲▼), fields of key (mono 11), duplicate suffix "(2)" in text-2, type (mono 11 text-3) and the value (13px, mono 11.5 for mono types) clamped to three lines with "Show more". Transposed is its own grid: the names frozen in the first column, one column per row, no sort indicator, find and go-to off. The text view is a Pretty ASCII table in Fragment Mono 11.5/17 (widths capped at 255 characters; whole lines select and copy); above 100,000 rows it asks in place (Back to the table Default, Show as text Primary, last). The value viewer is a 480px non-modal popover on the dialog anatomy anchored to its cell: the column name in mono (data, ellipsizes), "{type} · row n", Raw / Pretty (JSON, XML; invalid JSON keeps Raw with Pretty disabled and says why), Hex for bytes (8 bytes a row under 560px), the value well (panel / ground, 1px seam-strong, 4px radius, Fragment Mono 11.5/17, 120–240px; it scrolls, and no scroll thumb is drawn yet: NOTES asks for a 4px one, a T38 gap), the truncation notice with "Re-run with a 64 MB value limit" (Default; only a statement classified as read is re-run, through Run's checks), the size in mono 11.5 text-2 and Copy raw value (Default; always the server's bytes). Sizes read in MB through the shared size messages.
- **Grid accessibility (Task 40):** while a screen reader is attached the grid exposes its visible rows (the header and ±5 rows of overscan) as rows and cells named by their display text (NULL, cut and loading values in words), the focus cell as the active descendant, sort direction on the header cells, and one polite announcement when the row changes ("Row n of N", "or more" for an open result) or a large selection is made; nothing is built without one. Tab leaves the grid from any cell.
<!-- ## m3c-t43b: part B (Tasks 39, 41 and 42). -->
- **Disabled ink (`text-disabled`, M3c-38):** dark #636A76, light #A0A0A8, one token for every disabled control: buttons and their key hints and glyphs, checkboxes, Segmented segments (the view switch's Transposed), the read-only mode chip, find toggles and menu rows. The Raised Ink Rule never remaps it (a disabled step button in a find bar or a result toolbar stays text-disabled, not text-2). A disabled item that says why keeps its reason readable in text-2 (a menu row's hint, a tooltip).
- **Menu rows and the keyboard-active row (every PopupMenu, M3c-36):** 24px rows, the label at text, its hint after it at text-2 (data hints in mono), keys trailing at text-2, separators as a 1px seam; a checked row draws the blue check in the menu's check column, and a menu with any checked row draws the column on every row. The active row (moved to by the keys or the pointer) is blue-tint-strong with a 1px blue left edge and text ink for its label, hint and keys (a destructive row keeps its red); disabled rows and notes never become active. A menu opened from the keyboard (⇧F10, the context-menu key, a screen reader's Show menu) starts on its first enabled row; one opened by a pointer starts with none. Anchored menus are placed before they paint and never cross the window's edge or the status bar (a long one shows whole rows and scrolls).
- **Grid context menu (23, Task 39):** a right press on a cell, a column header or a row number opens the menu at the pointer (what the press hits outside the selection is selected first; inside it, the menu acts on the selection); ⇧F10 opens it under the focus cell. Cell: Copy ⌘C · Copy with headers | Open value viewer ⇧↵ · Raw display "this column" | Sort ascending · Sort descending · Clear sort | Go to row… ⌘L. Header: Sort ascending (✓ when current) · Sort descending · Clear sort · "Add as sort level n" while another column is sorted | Freeze up to here, or Unfreeze columns once frozen · Hide column · Show all columns · Fit width | Raw display · Copy column name. Row number: Copy rows · Copy with headers | Show as record · Go to row…. Disabled rows keep their place in text-disabled with the reason as their hint: "Ranges don't line up" (a copy the grid would refuse), "Select one cell", "Not sorted", "None hidden", "Last shown column". The Transposed view's cell menu ends after Raw display; the Text view has none. Shipped without the mock's 12px icon column (PopupMenu has none yet; a PO item), so Raw display shows its check in the check column.
- **Narrow panes (24, Task 41, R-M3-75):** a console split down to the 120px floor keeps every trust signal whole in every locale. Below the per-locale minimum (en 126, tr 141, pseudo 219px) the console toolbar takes its narrow stage after `wrap_row`: padding 4/8 (spacing xs / md), 6px between items and 4px between rows. Run and Stop become icon-only buttons with their words and keys in the tooltip and the accessible name (N1); the connection name may ellipsize (data) but its environment mark never does (N2); the transaction badge keeps its stamp, wraps "Transaction · n statements" inside itself and keeps Commit and Roll back as 20px icon buttons (check, undo-2; N3); the mode chip keeps its word ("Auto", "Manual", "Smart"; a read-only pane keeps its disabled "Auto-commit") and drops its commit glyph (N4); the waiting item reads on two lines with Cancel's word wherever it fits, else its x glyph (N1′); Running is the run marker and the clock, "Running" in the tooltip (N5); "Cancelling…" keeps its word. Chips wrap inside themselves on 15px lines with 2/7px padding, at least 22px tall (the read-only lock chip, red on production, with its whole label; the pending "Read-only after this transaction"); no chrome ends in "…" <!-- ## m3c-narrow (fix round 1, M3) -->but a pane tab's title (F12, below). A narrow result toolbar compacts the view switch and the record toggle first, moves Count rows into the overflow, and gives Copy up last (M3c-18). The status bar follows the focused pane. <!-- ## m3c-narrow (F10, F11, F12) -->The pane's own surfaces give way too, never cut: a tab strip narrower than its tabs shrinks the inactive tabs first, their titles ellipsizing, down to nothing (Ctrl+Tab still reaches them), then the active tab's title ellipsizes (data or Fluent, its tooltip and accessible name whole); an empty console whose text area leaves the placeholder less than 120px wraps it under the first line across the editor's width (<!-- ## m3c-narrow (fix round 1, M2) -->8px (spacing md) in from each side); an editor narrower than 180px draws the reduced-mode banner's narrow form: 4px beside the well, padding 6, no info glyph, <!-- ## m3c-narrow (fix round 1, I1) -->the title whole (a trust signal: the well never gets shorter than the title and its padding, and it clips, so a title the pane cannot hold is reported as cut) and the body below it in a scroll viewport that gives up its height first; the text then keeps two lines where the pane has room for them, and gives them up before the title.
- **Chrome metrics (`hacivat_ui::metrics`, F2 M9):** a wrapped chrome sentence's lines are 16px apart with 4px above and below (the strip's reason, the waiting sentence); the compact button is 20px tall with 7px side padding, a 5px icon gap and a 12px label; the strip keeps 6px between its sentence and its buttons; a menu footer keeps 5px between a chord and its word; the narrow chips' 15px lines and 2/7px padding above. Widgets read these names <!-- ## m3c-narrow (T43b review m9): the three exceptions this sentence named (the cursor strip's own 16px pitch and literal 6px gaps, the narrow mode chip's 6px sides and literal 22px minimum) were carried to Task 41's owner and now read the names; the mode chip's sides went from 6px to 7px. -->with no exception: the cursor strip (`results/strip.rs`) reads the wrap pitch and its 6px gap by name, and the narrow mode chip (`console/tx_chip.rs`) takes the 2/7px padding and the 22px minimum as the lock and pending chips do.
- **Settings rows (25, Task 42):** the shipped Settings page (a 176px navigation on side, the selected section blue-tint with a 1px blue left edge; a search field up to 320px; a 30px small-caps section title in text-2) gains four sections, Editor, Execution, Results and History (M3c-37). A row is padded 6/12: its title in a 200px column at 13px text-2, then its control (a 26px Segmented, or a field), then its note at 12px text-2 under the control. The largest value kept per cell is an 88px mono number field (left-aligned as shipped; the mock's right alignment needs an input option, a PO item) beside a 76px KB / MB Select, noted "From 64 KB to 64 MB."; a value outside the range is set to the bound and says so under the field with the info glyph ("128 MB is above the largest allowed, so it was set to 64 MB."; below 64 KB likewise). While searching (English and Turkish, Turkish-tolerant) the section title hides and each row names its section after its control in text-2.
<!-- ## m3c-narrow: the "Known gaps the shots list" bullet is gone: part B's F10, F11 and F12 are fixed (the narrow-pane rules above), and `KNOWN_FINDINGS` and `KNOWN_FINDINGS_B` are empty for the T45 gate. -->

## Do's and Don'ts

### Do:
- **Do** keep exactly one filled production badge (the capsule); express production elsewhere with swatches, lock icons, the 1px editor hairline, the tinted status segment or plain red text.
- **Do** name the write target on every staging action ("Submit to orders-staging", "Delete selected rows from orders-staging", "Runs on orders-staging in one transaction").
- **Do** format quantities by UI locale (en-US / tr-TR) and copy or export raw values.
- **Do** make the status bar, result toolbar, inspector and next-page strip follow the active result tab.
- **Do** collapse toolbar actions into an overflow menu when narrow, and test every surface in Turkish.
- **Do** separate regions with 1px seams and tone steps; keep radii to 3, 4 and 6px.
- **Do** use Lucide at 1.6 stroke, 12px in chrome and 14px default.
- **Do** use text-2, not text-3, for metadata on the raised surface.
- **Do** drive splash motion only from the motion tokens, and keep a crossfade fallback for reduced motion.
- **Do** isolate destructive actions with space and a seam, outline-only until hover or focus.

### Don't:
- **Don't** add a second filled environment badge anywhere, including tabs, tree rows or the status bar.
- **Don't** use semantic hues decoratively or introduce a fifth accent outside syntax highlighting.
- **Don't** put shadows on panels, toolbars, tabs or buttons; shadows belong to popovers.
- **Don't** wrap content in shadowed cards or float chrome other than popovers; bordered insets (DML preview, boot panel) stay flat.
- **Don't** treat the placeholder logo as final, or extend shadow-theatre heritage beyond the name and the logo.
- **Don't** add ambient motion to the workspace.
- **Don't** render text below 11px or put a radius on marks smaller than 8px.
- **Don't** use proportional figures or uppercase-with-tracking in place of small caps.
