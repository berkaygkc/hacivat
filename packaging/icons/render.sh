#!/usr/bin/env bash
# Re-render the Linux app icons (placeholder until a final app icon exists): the hv-mark of
# crates/ui/hacivat-ui-core/assets/icons/hacivat/hv-mark.svg (dashed frame, H) on the DESIGN.md
# dark ground. The dashes are cut by hand because ImageMagick's stroke-dasharray is unreliable.
# usage: packaging/icons/render.sh   (needs ImageMagick 7 `magick`; output is committed)
set -euo pipefail
cd "$(dirname "$0")"
GROUND='#16181D'   # dark-ground
FRAME='#8B949E'    # dark-text-2
MARK='#7AA2F7'     # dark-blue
GAPS=""
for c in 147 256 365; do
  GAPS="$GAPS rectangle $((c - 18)),60 $((c + 18)),100 rectangle $((c - 18)),412 $((c + 18)),452"
  GAPS="$GAPS rectangle 60,$((c - 18)) 100,$((c + 18)) rectangle 412,$((c - 18)) 452,$((c + 18))"
done
magick -size 512x512 xc:none \
  -fill "$GROUND" -draw 'roundrectangle 0,0 511,511 102,102' \
  -fill none -stroke "$FRAME" -strokewidth 24 -draw 'roundrectangle 80,80 432,432 67,67' \
  -stroke none -fill "$GROUND" -draw "$GAPS" \
  -fill none -stroke "$MARK" -strokewidth 37 \
  -draw 'stroke-linecap round line 192,170 192,342 line 320,170 320,342 line 192,256 320,256' \
  -strip PNG32:hacivat-512.png
for s in 256 128; do
  magick hacivat-512.png -filter Lanczos -resize "${s}x${s}" -strip "PNG32:hacivat-$s.png"
done
