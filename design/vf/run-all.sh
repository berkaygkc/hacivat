#!/bin/bash
# Full VF pass: build, capture every reference + candidate, compare, write side-by-side images.
# Candidates exist at 2x only on this host (macOS Retina); 1x / 1.5x need a matching display
# (see README.md).
#
# Each capture starts from a clean slate (stale PNGs are removed first), so a failed capture can
# never be compared as if it were fresh. A failed capture marks the run failed and skips its
# compare; capture stderr is shown, not discarded.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
cargo build -q -p spike-harman -p xtask || exit 2
mkdir -p target/vf
fail=0
for theme in dark light; do
  for lang in en tr; do
    tag="$theme-$lang-2x"
    ref="target/vf/ref-$tag.png"
    cand="target/vf/cand-$tag.png"
    report="target/vf/vf-$tag.txt"
    echo "=== $tag"
    rm -f "$ref" "$cand" "$report"
    if ! design/vf/capture.sh ref "$theme" "$lang" 2 >/dev/null; then
      echo "FAIL $tag: reference capture failed; compare skipped" >&2
      fail=1
      continue
    fi
    if ! design/vf/capture.sh cand "$theme" "$lang" 2 >/dev/null; then
      echo "FAIL $tag: candidate capture failed; compare skipped" >&2
      fail=1
      continue
    fi
    status=0
    cargo run -q -p xtask -- vf --reference "$ref" --candidate "$cand" \
      --scale 2 --theme "$theme" --side-by-side "target/vf/side-by-side-$tag.png" >"$report" 2>&1 \
      || status=$?
    if [ "$status" -ne 0 ]; then
      fail=1
    fi
    if [ "$status" -ge 2 ]; then
      # Error (bad expectation, reference/token mismatch, bad image): show the whole report.
      cat "$report"
    else
      grep -E "^(FAIL|vf:)" "$report" || true
    fi
  done
done
for scale in 1 1.5; do
  rm -f "target/vf/ref-dark-en-${scale}x.png"
  if design/vf/capture.sh ref dark en "$scale" >/dev/null; then
    echo "=== dark-en-${scale}x: reference captured; candidate needs a ${scale}x display"
  else
    echo "FAIL dark-en-${scale}x: reference capture failed" >&2
    fail=1
  fi
done
exit "$fail"
