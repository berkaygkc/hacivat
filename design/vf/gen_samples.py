"""Regenerates design/vf/samples.json (run from the repo root: python3 design/vf/gen_samples.py).

Coordinates are logical px on the 1440x900 frame, derived from the harman layout and checked by
running `xtask vf` with the Chromium reference as its own candidate. Tokens that have no flat
>= 3x3 px area in the static frame are deliberately not sampled: hover (no pointer), text and
syn_* (glyph strokes only), on_* inks.
"""
import json

C = [  # name, x, y, token
    ("titlebar-bg", 1050, 20, "ground"),
    ("cp-fill-capsule", 185, 20, "cp_fill"),
    ("capsule-bg", 643, 10, "panel"),
    ("palette-bg", 900, 10, "ground"),
    ("seg-pressed-bg", 1170, 10, "raised"),
    ("titlebar-seam", 1050, 39, "seam"),
    ("explorer-bg", 150, 600, "side"),
    ("explorer-header-bg", 200, 55, "head"),
    ("explorer-selected-row", 250, 336, "blue_tint"),
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
    ("editor-band-line14", 1300, 350, "blue_tint"),
    ("popup-selected-item", 997, 182, "blue_tint_strong"),
    ("popup-detail-bg", 1220, 300, "raised"),
    ("popup-foot-bg", 1100, 330, "head"),
    ("results-header-bg", 800, 378, "head"),
    ("result-tab-active-bg", 294, 385, "raised"),
    ("gtool-bg", 1255, 404, "raised"),
    ("where-box-bg", 580, 404, "panel"),
    ("grid-header-bg", 800, 429, "head"),
    ("grid-rownum-bg", 290, 470, "gutter"),
    ("grid-cell-bg", 700, 470, "panel"),
    ("grid-selected-cell", 880, 515, "blue_tint"),
    ("grid-active-cell", 990, 515, "blue_tint_strong"),
    ("grid-selbox-border", 866, 520, "blue"),
    ("grid-fill-col", 1160, 470, "panel"),
    ("scroll-track", 1184, 700, "gutter"),
    ("strip-bg", 800, 864, "raised"),
    ("strip-page-current", 293, 863, "blue"),
    ("inspector-bg", 1300, 860, "gutter"),
    ("inspector-header-bg", 1300, 442, "gutter"),
    ("seam-inspector", 1188, 600, "seam"),
    ("status-bg", 700, 888, "ground"),
    ("status-env-tint", 5, 888, "red_tint"),
    ("status-seam", 700, 876, "seam"),
    ("swatch-prod-explorer", 120, 221, "red"),
    ("swatch-amber-result-tab", 580, 377, "amber"),
    ("tab-dot-text2", 788, 55, "text_2"),
    ("scroll-thumb", 1184, 480, "text_3@.6"),
    ("scroll-prefetch", 1184, 506, "green_fill@.6"),
    ("strip-page-prefetched", 300, 863, "green_fill"),
    ("grid-header-seam-strong", 800, 462, "seam_strong"),
    ("grid-row-seam-soft", 700, 483, "seam_soft"),
]
colors = [dict(name=n, x=x, y=y, token=t, theme=th) for th in ("dark", "light") for n, x, y, t in C]

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
json.dump(dict(colors=colors, scanlines=S), open("design/vf/samples.json", "w"), indent=1, ensure_ascii=False)
print(len(colors), len(S))
