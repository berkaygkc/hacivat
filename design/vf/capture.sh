#!/bin/bash
# Captures VF frames into target/vf/.
#   design/vf/capture.sh ref  <theme> <lang> <scale>   Chromium reference of the harman mock
#   design/vf/capture.sh cand <theme> <lang> <scale>   GPUI candidate (spike-harman window, macOS)
# Needs CARGO_TARGET_DIR (shared target dir) for `cand`.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/target/vf"
mkdir -p "$OUT"
kind="$1"; theme="${2:-dark}"; lang="${3:-en}"; scale="${4:-2}"
tag="${theme}-${lang}-${scale}x"
case "$kind" in
  ref)
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --disable-gpu \
      --hide-scrollbars --window-size=1440,900 --force-device-scale-factor="$scale" \
      --virtual-time-budget=4000 --screenshot="$OUT/ref-$tag.png" \
      "file://$ROOT/design/directions/harman/index.html?theme=$theme&lang=$lang&view=workspace&result=1"
    ;;
  cand)
    BIN="${CARGO_TARGET_DIR:-$ROOT/target}/debug/spike-harman"
    LOG="$OUT/cand-$tag.log"
    "$BIN" --theme "$theme" --lang "$lang" --scale "$scale" --print-window-id >"$LOG" 2>&1 &
    pid=$!
    for _ in $(seq 1 50); do
      grep -q '^WINDOW_ID=' "$LOG" 2>/dev/null && break
      kill -0 "$pid" 2>/dev/null || { cat "$LOG"; exit 1; }
      sleep 0.2
    done
    wid="$(sed -n 's/^WINDOW_ID=//p' "$LOG" | head -1)"
    sleep 2
    screencapture -x -o -l "$wid" "$OUT/cand-$tag.png"
    # screencapture tags the file with the display profile (Display P3); the reference is sRGB.
    sips -m "/System/Library/ColorSync/Profiles/sRGB Profile.icc" "$OUT/cand-$tag.png" --out "$OUT/cand-$tag.png" >/dev/null
    kill "$pid" 2>/dev/null || true
    ;;
  *) echo "usage: capture.sh ref|cand theme lang scale" >&2; exit 2 ;;
esac
echo "wrote $OUT/$kind-$tag.png"
