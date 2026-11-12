# VF tooling

- `run-all.sh` builds, captures every reference (Chromium) and candidate (spike-harman, macOS), runs `cargo run -p xtask -- vf` per capture and writes `target/vf/side-by-side-*.png`. Each capture starts from deleted PNGs; a failed capture fails the run and skips its compare.
- `capture.sh ref|cand <theme> <lang> <scale>` captures one frame on macOS or Linux; `capture.ps1` does the same on Windows. Candidates are written as untagged sRGB (xtask refuses a candidate with an embedded ICC profile).
- `samples.json` is generated: run `python3 design/vf/gen_samples.py` from the repo root to regenerate it (edit the generator, not the JSON).
- `pair.py <tag> x0 y0 x1 y1 <out.png>` stacks the same device-pixel region of reference and candidate for eyeballing.

`xtask vf` first validates its expectations against the reference: both images must be 1440×900 × scale, every reference sample pixel must match its DESIGN.md token (ΔE2000 ≤ 1; translucent tokens over their `over` backdrop), and every scanline must find exactly its expected edges in the reference. Any of those failing exits 2 ("reference/token mismatch" or "bad expectation"); only then is the candidate graded (exit 1 on a candidate FAIL).

## M1 shell (Task 18)

The M1 shell is graded on `samples-m1-shell.json` (`python3 design/vf/gen_samples.py --set m1-shell`): only the title bar, the status bar, the explorer and connection panel headers and the seams, because M1 has no tree, editor content, grid, inspector or view switcher.

The candidate comes from the screenshot harness (R8: GPUI's `HeadlessAppContext` with the real text system and the Metal/wgpu headless renderer), which also renders every M1 surface in `en`, `tr`, `en-XA` and `tr-XT`, dark and light, into `target/shots/`:

```sh
cargo test -p hacivat-app --features shots --test shots -- --ignored
for theme in dark light; do for lang in en tr; do
  design/vf/capture.sh ref "$theme" "$lang" 2
  cargo run -p xtask -- vf --reference "target/vf/ref-$theme-$lang-2x.png" \
    --candidate "target/shots/shell-demo-production-$theme-$lang-1440x900@2x.png" \
    --samples design/vf/samples-m1-shell.json --scale 2 --theme "$theme"
done; done
```

The real-window fallback (`capture.sh app`) runs `hacivat --vf-demo --print-window-id --theme --lang` with a fresh `HOME`, so no saved layout moves a seam. The binary cannot pick a display, so capture at the scale of the display its window opens on (1 on the PO's external main display); `capture.sh` refuses a capture whose width does not match `<scale>`:

```sh
cargo build -p hacivat -p xtask
design/vf/capture.sh ref dark en 1 && design/vf/capture.sh app dark en 1
cargo run -p xtask -- vf --reference target/vf/ref-dark-en-1x.png \
  --candidate target/vf/cand-app-dark-en-1x.png --samples design/vf/samples-m1-shell.json --scale 1 --theme dark
```

`cargo xtask shot-diff --baseline <dir> --candidate target/shots [--max-de 2.0 --max-share 0.001]` compares same-named PNGs pixel by pixel (ΔE2000) and exits 1 when a shot changed in more than the allowed share of pixels, is missing, or changed size. The nightly `pseudo-screens` job runs it against `design/baselines/m1/macos` once those baselines exist (Task 19).

## macOS

```sh
cargo build -p spike-harman -p xtask
design/vf/run-all.sh                     # all four 2x captures + compares
```

### 1x on the Mac's external scale-1.0 display

The built-in panel is Retina (2x). A 1x candidate needs the window on a display that renders at scale 1.0 (the PO's external 5120×1440 display):

```sh
design/vf/capture.sh ref  dark en 1
design/vf/capture.sh cand dark en 1      # spike-harman exits 4 on the Retina panel; read the
                                         # "spike-harman: displays [...]" line it printed
HARMAN_DISPLAY=<index of the external display> design/vf/capture.sh cand dark en 1
cargo run -p xtask -- vf --reference target/vf/ref-dark-en-1x.png \
  --candidate target/vf/cand-dark-en-1x.png --scale 1 --theme dark
```

spike-harman checks the window's real scale factor and exits 4 if it differs from `--scale`, so a 1x run on the wrong display cannot produce a mislabelled capture.

## Linux

Needs Chrome or Chromium (`CHROME=/path/to/chrome` if it is not `google-chrome`, `google-chrome-stable`, `chromium` or `chromium-browser` on `PATH`) and one capture tool, picked in this order:

- Wayland: `grim`, with `VF_GRIM_GEOMETRY="x,y WxH"` set to the window's logical rectangle (Wayland has no portable way to look up another client's window).
- X11: ImageMagick `import -window <title>`, else `xwd -name <title>` converted by ImageMagick (`magick` or `convert`).

The window is found by the `WINDOW_TITLE=` line spike-harman prints with `--print-window-id`. ImageMagick strips any embedded profile, so the PNG is untagged sRGB.

```sh
cargo build -p spike-harman -p xtask
CHROME=chromium design/vf/capture.sh ref  dark en 1
design/vf/capture.sh cand dark en 1
cargo run -p xtask -- vf --reference target/vf/ref-dark-en-1x.png \
  --candidate target/vf/cand-dark-en-1x.png --scale 1 --theme dark
```

## Windows

`capture.ps1` captures the reference with headless Chrome and the candidate's client area with PrintWindow (`PW_RENDERFULLCONTENT`, so DirectX content is included) or, with `-Method CopyFromScreen`, from the screen (window must be frontmost and unoccluded). The script is per-monitor DPI aware, so it captures physical pixels; set the display to the scale you capture (150 % for `-Scale 1.5`). System.Drawing writes PNGs without an ICC profile.

```powershell
cargo build -p spike-harman -p xtask
pwsh design/vf/capture.ps1 -Kind ref  -Theme dark -Lang en -Scale 1.5 -Chrome "C:\Program Files\Google\Chrome\Application\chrome.exe"
pwsh design/vf/capture.ps1 -Kind cand -Theme dark -Lang en -Scale 1.5
cargo run -p xtask -- vf --reference target/vf/ref-dark-en-1.5x.png `
  --candidate target/vf/cand-dark-en-1.5x.png --scale 1.5 --theme dark
```

`capture.ps1` and the Linux paths of `capture.sh` have not been run yet (no Windows or Linux host in M0 so far); CI lints `capture.ps1` with PSScriptAnalyzer and the shell scripts pass shellcheck.
