#!/bin/bash
# Full VF pass: build, capture every reference + candidate, compare, write side-by-side images.
# Candidates exist at 2x only on this host (macOS Retina); 1x / 1.5x need a matching display.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
cargo build -q -p spike-harman -p xtask || exit 2
fail=0
for theme in dark light; do
  for lang in en tr; do
    tag="$theme-$lang-2x"
    design/vf/capture.sh ref "$theme" "$lang" 2 >/dev/null 2>&1
    design/vf/capture.sh cand "$theme" "$lang" 2 >/dev/null 2>&1
    echo "=== $tag"
    cargo run -q -p xtask -- vf --reference "target/vf/ref-$tag.png" --candidate "target/vf/cand-$tag.png" \
      --scale 2 --theme "$theme" --side-by-side "target/vf/side-by-side-$tag.png" > "target/vf/vf-$tag.txt" 2>&1 \
      || fail=1
    grep -E "^(FAIL|vf:)" "target/vf/vf-$tag.txt"
  done
done
for scale in 1 1.5; do
  design/vf/capture.sh ref dark en "$scale" >/dev/null 2>&1
  echo "=== dark-en-${scale}x: reference captured; candidate needs a ${scale}x display"
done
exit $fail
