# VF tooling

- `run-all.sh` builds, captures every reference (Chromium) and candidate (spike-harman, macOS), runs `cargo run -p xtask -- vf` per capture and writes `target/vf/side-by-side-*.png`.
- `capture.sh ref|cand <theme> <lang> <scale>` captures one frame; candidates are converted to untagged sRGB (xtask refuses a candidate with an embedded ICC profile).
- `samples.json` is generated: run `python3 design/vf/gen_samples.py` from the repo root to regenerate it (edit the generator, not the JSON).
- `pair.py <tag> x0 y0 x1 y1 <out.png>` stacks the same device-pixel region of reference and candidate for eyeballing.
