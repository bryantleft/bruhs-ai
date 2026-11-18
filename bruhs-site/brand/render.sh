#!/bin/sh
# Regenerates the raster brand assets in public/ from brand/*.html with
# headless Google Chrome: og.png (social card), apple-touch-icon.png and the
# Mac disk image background, ../packaging/macos/dmg-background.tiff (1x + 2x).
# Run brand/logo.mjs first if the mark changed. Binds 127.0.0.1:8492.
set -e
cd "$(dirname "$0")/.."
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
python3 -m http.server 8492 --bind 127.0.0.1 >/dev/null 2>&1 & SERVER=$!
trap 'kill $SERVER' EXIT
sleep 1
"$CHROME" --headless=new --hide-scrollbars --force-device-scale-factor=1 --window-size=1200,630 \
  --virtual-time-budget=4000 --screenshot=public/og.png http://127.0.0.1:8492/brand/og.html 2>/dev/null
"$CHROME" --headless=new --hide-scrollbars --force-device-scale-factor=1 --window-size=180,180 \
  --virtual-time-budget=2000 --screenshot=public/apple-touch-icon.png http://127.0.0.1:8492/brand/touch-icon.html 2>/dev/null
WORK=$(mktemp -d)
trap 'kill $SERVER; rm -rf "$WORK"' EXIT
for SCALE in 1 2; do
  "$CHROME" --headless=new --hide-scrollbars --force-device-scale-factor=$SCALE --window-size=640,420 \
    --virtual-time-budget=4000 --screenshot="$WORK/dmg-$SCALE.png" http://127.0.0.1:8492/brand/dmg.html 2>/dev/null
done
# One TIFF holding both sizes, so Retina screens get the 2x art; the 2x one
# needs 144 dpi or Finder shows it at double size.
sips -s dpiWidth 144 -s dpiHeight 144 "$WORK/dmg-2.png" >/dev/null
tiffutil -cathidpicheck "$WORK/dmg-1.png" "$WORK/dmg-2.png" -out ../packaging/macos/dmg-background.tiff
echo "wrote public/og.png, public/apple-touch-icon.png and ../packaging/macos/dmg-background.tiff"
