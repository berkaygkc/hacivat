"""Regenerates design/vf/samples.json (run from the repo root: python3 design/vf/gen_samples.py).
`--set m1-shell` writes design/vf/samples-m1-shell.json instead: the M1 shell subset (Task 18).

Coordinates are logical px on the 1440x900 frame, derived from the harman layout and checked by
running `xtask vf` with the Chromium reference as its own candidate. Tokens that have no flat
>= 3x3 px area in the static frame are deliberately not sampled: hover (no pointer), text and
syn_* (glyph strokes only), on_* inks.

`token` may be a {theme: token} dict where the mock styles the two themes differently. A `@op`
suffix multiplies the token's alpha (CSS `opacity`). Translucent tokens name the opaque token they
are composited over in `over`; `xtask vf` checks every REFERENCE pixel against
token-over-backdrop from DESIGN.md (dE2000 <= 1) before it compares the candidate.
"""
import json
import sys

C = [  # name, x, y, token[, over]
    ("titlebar-bg", 1050, 20, "ground"),
    ("cp-fill-capsule", 185, 20, "cp_fill"),
    ("capsule-bg", 643, 10, "panel"),
    ("palette-bg", 900, 10, "ground"),
    ("seg-pressed-bg", 1170, 10, "raised"),
    ("titlebar-seam", 1050, 39, "seam"),
    ("explorer-bg", 150, 600, "side"),
    ("explorer-header-bg", 200, 55, "head"),
    ("explorer-selected-row", 250, 336, "blue_tint", "side"),
    ("explorer-selected-bar", 0, 336, "blue"),
    ("sheet-header-bg", 170, 686, "head"),
    ("sheet-bg", 265, 760, "side"),
    ("seam-explorer-editor", 276, 300, "seam"),
    ("seam-explorer-editor-low", 276, 800, "seam"),
    ("seam-left-split", 100, 670, "seam"),
    ("seam-editor-results", 700, 362, "seam"),
    ("editor-red-line", 900, 40, "red"),
    ("editor-tab-active-bg", 466, 44, "raised"),
    ("editor-header-bg", 1000, 55, "head"),
    ("qbar-bg", 900, 86, "raised"),
    ("run-button-bg", 300, 76, "blue"),
    ("editor-bg", 1300, 250, "panel"),
    # .lines .l.band::before: --blue-a at opacity .55 (dark) / .8 (light).
    ("editor-band-line14", 1300, 350, {"dark": "blue_tint@.55", "light": "blue_tint@.8"}, "panel"),
    ("popup-selected-item", 997, 182, "blue_tint_strong", "raised"),
    ("popup-detail-bg", 1220, 300, "raised"),
    ("popup-foot-bg", 1100, 330, "head"),
    ("results-header-bg", 800, 378, "head"),
    ("result-tab-active-bg", 294, 385, "raised"),
    ("gtool-bg", 1255, 404, "raised"),
    # .qbox: --panel, but --ground in light.
    ("where-box-bg", 580, 404, {"dark": "panel", "light": "ground"}),
    ("grid-header-bg", 800, 429, "head"),
    ("grid-rownum-bg", 290, 470, "gutter"),
    ("grid-cell-bg", 700, 470, "panel"),
    ("grid-selected-cell", 880, 515, "blue_tint", "panel"),
    ("grid-active-cell", 990, 515, "blue_tint_strong", "panel"),
    ("grid-selbox-border", 866, 520, "blue"),
    ("grid-fill-col", 1160, 470, "panel"),
    ("scroll-track", 1184, 700, "gutter"),
    ("strip-bg", 800, 864, "raised"),
    ("strip-page-current", 293, 863, "blue"),
    ("inspector-bg", 1300, 860, "gutter"),
    ("inspector-header-bg", 1300, 442, "gutter"),
    ("seam-inspector", 1188, 600, "seam"),
    ("status-bg", 700, 888, "ground"),
    ("status-env-tint", 5, 888, "red_tint", "ground"),
    ("status-seam", 700, 876, "seam"),
    ("swatch-prod-explorer", 120, 221, "red"),
    ("swatch-amber-result-tab", 580, 377, "amber"),
    ("tab-dot-text2", 788, 55, "text_2"),
    ("scroll-thumb", 1184, 480, "text_3@.6", "gutter"),
    ("scroll-prefetch", 1184, 506, "green_fill@.6", "gutter"),
    ("strip-page-prefetched", 300, 863, "green_fill", "raised"),
    ("grid-header-seam-strong", 800, 462, "seam_strong"),
    ("grid-row-seam-soft", 700, 483, "seam_soft"),
]
def sample(theme, name, x, y, token, over=None):
    d = dict(name=name, x=x, y=y, token=token[theme] if isinstance(token, dict) else token, theme=theme)
    if over:
        d["over"] = over
    return d


colors = [sample(th, *c) for th in ("dark", "light") for c in C]

S = [
    dict(name="titlebar-bottom-seam", axis="y", at=1050, **{"from": 20, "to": 60}, expect_edges_px=[39, 40, 41]),
    dict(name="explorer-header-seam", axis="y", at=272, **{"from": 60, "to": 80}, expect_edges_px=[69, 70]),
    dict(name="explorer-editor-seam", axis="x", at=667, **{"from": 250, "to": 300}, expect_edges_px=[276, 277]),
    dict(name="left-column-split", axis="y", at=272, **{"from": 660, "to": 690}, expect_edges_px=[670, 671]),
    dict(name="editor-header-qbar-32", axis="y", at=1000, **{"from": 60, "to": 110}, expect_edges_px=[69, 70, 101, 102]),
    dict(name="editor-results-seam-header-30", axis="y", at=800, **{"from": 361, "to": 396},
         expect_edges_px=[362, 363, 392, 393]),
    dict(name="toolbar-34-and-grid-header-36", axis="y", at=1255, **{"from": 395, "to": 431},
         expect_edges_px=[426, 427]),
    dict(name="grid-rows-1-5-pitch-21", axis="y", at=700, **{"from": 458, "to": 572}, tol_de=0.8,
         note="seam_soft vs panel is only dE 1.4 in dark, below the default 2.0 edge threshold",
         expect_edges_px=[462, 463, 483, 484, 504, 505, 525, 526, 546, 547, 567, 568]),
    dict(name="grid-track-inspector-edge", axis="x", at=600, **{"from": 1170, "to": 1196},
         expect_edges_px=[1179, 1180, 1188, 1189]),
    dict(name="strip-and-status-top", axis="y", at=850, **{"from": 845, "to": 890},
         expect_edges_px=[850, 851, 876, 877]),
]

# The M1 shell set (Task 18): only regions that the harman reference and the M1 demo shell
# (`hacivat --vf-demo`, `shell-demo-production`) draw alike. M1 has no tree, editor content,
# results grid, inspector, production hairline, next-page strip or view switcher, so samples sit
# in the title bar (capsule, palette, language segments), the status bar, the explorer and
# connection panel headers (x < 276) and on the seams (x = 276; y = 362, 671). Points avoid
# glyphs in both frames; xtask's m1_shell_samples_validate_against_reference checks the zones.
M1 = [  # name, x, y, token[, over]
    ("titlebar-bg", 1105, 20, "ground"),
    ("cp-fill-capsule", 183, 12, "cp_fill"),
    ("capsule-name-bg", 300, 9, "panel"),
    ("palette-bg", 980, 10, "ground"),
    # The theme control: moon pressed (raised) in dark, unpressed (ground) in light.
    ("theme-seg-moon-bg", 1390, 10, {"dark": "raised", "light": "ground"}),
    ("titlebar-seam", 1105, 39, "seam"),
    ("explorer-header-bg", 200, 55, "head"),
    ("connection-header-bg", 170, 686, "head"),
    ("seam-explorer-editor", 276, 300, "seam"),
    ("seam-explorer-editor-low", 276, 800, "seam"),
    ("seam-left-split", 100, 670, "seam"),
    ("seam-editor-results", 700, 362, "seam"),
    ("status-bg", 700, 888, "ground"),
    ("status-seam", 700, 876, "seam"),
]
M1_SCANS = [
    dict(name="titlebar-bottom-seam", axis="y", at=160, **{"from": 30, "to": 60}, expect_edges_px=[39, 40]),
    dict(name="explorer-header-seam", axis="y", at=272, **{"from": 60, "to": 76}, expect_edges_px=[69, 70]),
    dict(name="explorer-editor-seam", axis="x", at=667, **{"from": 250, "to": 300}, expect_edges_px=[276, 277]),
    dict(name="left-column-split", axis="y", at=272, **{"from": 660, "to": 690}, expect_edges_px=[670, 671]),
    dict(name="editor-results-seam", axis="y", at=700, **{"from": 361, "to": 372}, expect_edges_px=[362, 363]),
    dict(name="status-top", axis="y", at=700, **{"from": 872, "to": 890}, expect_edges_px=[876, 877]),
]

SETS = ("default", "m1-shell")
chosen = "default"
if "--set" in sys.argv:
    i = sys.argv.index("--set")
    if i + 1 >= len(sys.argv):
        sys.exit(f"gen_samples.py: --set needs a value ({', '.join(SETS)})")
    chosen = sys.argv[i + 1]
    if chosen not in SETS:
        sys.exit(f"gen_samples.py: unknown set {chosen!r} ({', '.join(SETS)})")

if chosen == "m1-shell":
    m1 = [sample(th, *c) for th in ("dark", "light") for c in M1]
    json.dump(dict(colors=m1, scanlines=M1_SCANS), open("design/vf/samples-m1-shell.json", "w"), indent=1,
              ensure_ascii=False)
    print(len(m1), len(M1_SCANS))
else:
    json.dump(dict(colors=colors, scanlines=S), open("design/vf/samples.json", "w"), indent=1, ensure_ascii=False)
    print(len(colors), len(S))
