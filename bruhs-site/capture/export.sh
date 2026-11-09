#!/bin/sh
# Turns raw 2880x1800 captures into the WebP files public/shots/ serves.
# The hero keeps the whole window; surface shots drop the sidebar (544px at
# 2x) and crop to 16:10 so the tab row and the picture fit one screen.
set -e
RAW=${1:-/tmp/bruhs-shots}
OUT=$(cd "$(dirname "$0")/../public/shots" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
magick "$RAW/hero-agents.png" -resize 1440x900 "$TMP/hero-1440.png"
cwebp -quiet -q 82 -m 6 "$RAW/hero-agents.png" -o "$OUT/hero-2880.webp"
cwebp -quiet -q 85 -m 6 "$TMP/hero-1440.png" -o "$OUT/hero-1440.webp"
for pair in agents:hero-agents tasks:today calendar:calendar meetings:meetings people:people notes:notes life:life; do
  name=${pair%%:*} src=${pair##*:}
  [ -f "$RAW/$src.png" ] || continue
  magick "$RAW/$src.png" -crop 2336x1460+544+0 +repage "$TMP/$name.png"
  magick "$TMP/$name.png" -resize 1168x730 "$TMP/$name-1168.png"
  cwebp -quiet -q 82 -m 6 "$TMP/$name.png" -o "$OUT/$name-2336.webp"
  cwebp -quiet -q 85 -m 6 "$TMP/$name-1168.png" -o "$OUT/$name-1168.webp"
done
ls -l "$OUT"
