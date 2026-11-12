#!/bin/bash
# Captures VF frames into target/vf/ (macOS and Linux; Windows: capture.ps1).
#   design/vf/capture.sh ref  <theme> <lang> <scale>   Chromium reference of the harman mock
#   design/vf/capture.sh cand <theme> <lang> <scale>   GPUI candidate (spike-harman window)
#   design/vf/capture.sh app  <theme> <lang> <scale>   the app itself: `hacivat --vf-demo
#                                                      --print-window-id --theme --lang`, written
#                                                      as cand-app-<tag>.png (R8 fallback when a
#                                                      headless shot fails the VF self-check)
#
# Environment:
#   CHROME           Chrome/Chromium binary (default: the macOS app bundle, or the first of
#                    google-chrome, google-chrome-stable, chromium, chromium-browser on PATH).
#   CARGO_TARGET_DIR Target dir holding debug/spike-harman and debug/hacivat (default:
#                    <repo>/target).
#   HARMAN_DISPLAY   Index into the display list spike-harman prints; picks the display whose
#                    scale matches <scale> (e.g. the Mac's external 1x display, see README.md).
#   VF_GRIM_GEOMETRY Wayland only: the window's "x,y WxH" in logical px for grim.
#
# Candidate capture: macOS screencapture by window id; Linux X11 `import -window` (ImageMagick)
# or `xwd` (converted with ImageMagick), Linux Wayland `grim`, chosen by availability. Every
# candidate is written as untagged sRGB (xtask vf refuses an embedded ICC profile).
#
# `app` runs the binary with HOME set to a fresh temporary directory, so no saved layout
# (workspace.json) or settings of the user can move a seam. The binary has no --scale check:
# put its window on a display of the requested scale (the Retina panel for 2).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/target/vf"
mkdir -p "$OUT"
kind="${1:-}"; theme="${2:-dark}"; lang="${3:-en}"; scale="${4:-2}"
tag="${theme}-${lang}-${scale}x"
os="$(uname -s)"

have() { command -v "$1" >/dev/null 2>&1; }

find_chrome() {
  if [ -n "${CHROME:-}" ]; then
    echo "$CHROME"
    return
  fi
  if [ "$os" = Darwin ]; then
    echo "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    return
  fi
  local c
  for c in google-chrome google-chrome-stable chromium chromium-browser; do
    if have "$c"; then
      command -v "$c"
      return
    fi
  done
  echo "capture.sh: no Chrome/Chromium found; set CHROME" >&2
  exit 1
}

# ImageMagick 7 is `magick`; 6 is `convert`/`mogrify`.
magick_cmd() {
  if have magick; then echo magick; elif have convert; then echo convert; else echo ""; fi
}

# Writes the window titled $1 to $2 (Linux).
capture_linux() {
  local title="$1" png="$2" im
  im="$(magick_cmd)"
  if [ -n "${WAYLAND_DISPLAY:-}" ] && have grim; then
    if [ -z "${VF_GRIM_GEOMETRY:-}" ]; then
      echo "capture.sh: Wayland needs VF_GRIM_GEOMETRY=\"x,y WxH\" of the window" >&2
      return 1
    fi
    grim -s "$scale" -g "$VF_GRIM_GEOMETRY" "$png"
  elif have import; then
    import -window "$title" "$png"
  elif have xwd && [ -n "$im" ]; then
    xwd -silent -name "$title" | "$im" xwd:- "$png"
  else
    echo "capture.sh: need grim (Wayland), ImageMagick import, or xwd + ImageMagick (X11)" >&2
    return 1
  fi
  # Drop any embedded profile/gamma chunks: the comparison assumes untagged sRGB.
  if [ -n "$im" ]; then
    "$im" "$png" -strip "$png"
  fi
}

case "$kind" in
  ref)
    chrome="$(find_chrome)"
    "$chrome" --headless=new --disable-gpu \
      --hide-scrollbars --window-size=1440,900 --force-device-scale-factor="$scale" \
      --virtual-time-budget=4000 --screenshot="$OUT/ref-$tag.png" \
      "file://$ROOT/design/directions/harman/index.html?theme=$theme&lang=$lang&view=workspace&result=1"
    ;;
  cand|app)
    if [ "$kind" = cand ]; then
      BIN="${CARGO_TARGET_DIR:-$ROOT/target}/debug/spike-harman"
      name="cand-$tag"
      args=(--theme "$theme" --lang "$lang" --scale "$scale" --print-window-id)
      if [ -n "${HARMAN_DISPLAY:-}" ]; then
        args+=(--display "$HARMAN_DISPLAY")
      fi
      run_env=()
    else
      BIN="${CARGO_TARGET_DIR:-$ROOT/target}/debug/hacivat"
      name="cand-app-$tag"
      args=(--vf-demo --print-window-id --theme "$theme" --lang "$lang")
      run_env=(HOME="$(mktemp -d)")
    fi
    LOG="$OUT/$name.log"
    env ${run_env[@]+"${run_env[@]}"} "$BIN" "${args[@]}" >"$LOG" 2>&1 &
    pid=$!
    trap 'kill "$pid" 2>/dev/null || true' EXIT
    for _ in $(seq 1 50); do
      grep -q '^WINDOW_ID=' "$LOG" 2>/dev/null && break
      kill -0 "$pid" 2>/dev/null || { cat "$LOG" >&2; exit 1; }
      sleep 0.2
    done
    if ! grep -q '^WINDOW_ID=' "$LOG"; then
      echo "capture.sh: $(basename "$BIN") did not report its window" >&2
      cat "$LOG" >&2
      exit 1
    fi
    sleep 2
    png="$OUT/$name.png"
    if [ "$os" = Darwin ]; then
      wid="$(sed -n 's/^WINDOW_ID=//p' "$LOG" | head -1)"
      screencapture -x -o -l "$wid" "$png"
      # screencapture tags the file with the display profile (Display P3); the reference is sRGB.
      sips -m "/System/Library/ColorSync/Profiles/sRGB Profile.icc" "$png" --out "$png" >/dev/null
      # xtask vf refuses candidates that still carry an embedded profile: leave it untagged (sRGB).
      sips --deleteColorManagementProperties "$png" >/dev/null
      # The app cannot check its display's scale (spike-harman exits 4 instead): refuse a
      # capture whose size says it came from a display of another scale.
      want="$(awk -v s="$scale" 'BEGIN { printf "%d", 1440 * s + 0.5 }')"
      got="$(sips -g pixelWidth "$png" | sed -n 's/.*pixelWidth: //p')"
      if [ "$kind" = app ] && [ "$got" != "$want" ]; then
        rm -f "$png"
        echo "capture.sh: the window is ${got} px wide, not ${want} (scale $scale); move it to a" \
          "display of that scale (on a Mac with a 1x main display, capture scale 1)" >&2
        exit 1
      fi
    else
      title="$(sed -n 's/^WINDOW_TITLE=//p' "$LOG" | head -1)"
      capture_linux "$title" "$png"
    fi
    ;;
  *) echo "usage: capture.sh ref|cand|app theme lang scale" >&2; exit 2 ;;
esac
if [ "$kind" = ref ]; then
  echo "wrote $OUT/ref-$tag.png"
else
  echo "wrote $png"
fi
